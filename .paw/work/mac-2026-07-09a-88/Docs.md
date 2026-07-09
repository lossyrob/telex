# Copilot Detached Waiter Fallback

## Overview

Telex now supports a first-class Copilot CLI pull fallback when extension push
cannot be loaded. Push remains the preferred path. The fallback reuses the
existing single-shot `telex wait --out-dir` delivery contract, but Telex owns run
identity, private artifact paths, platform launch details, and safe delivery-mode
transitions.

The Copilot task runner still owns detached execution and completion wakeups.
Telex never creates an internal background worker or infinite wait loop. Each
prepared launcher executes one wait, writes a terminal `exit.code`, and exits.
The agent reads that exact run's artifacts, deduplicates and acknowledges the
message, then explicitly prepares the next run.

## Architecture and Design

### High-Level Architecture

The implementation has four layers:

1. **Harness-neutral daemon enforcement**
   - A station with an opaque on-deliver handler cannot arm a waiter.
   - A station with a live waiter cannot register an on-deliver handler.
   - Protocol 1.4 adds an explicit `replace_on_deliver` registration operation
     so a harness can clear push without detaching the durable member.
2. **Copilot run preparation**
   - `telex copilot fallback prepare` creates one current run for a
     `(store, session, address)`.
   - The run has a unique owner-private directory, a versioned manifest, and
     structured launcher data.
   - Repeated preparation returns the same unfinished run.
3. **Detached run execution**
   - A hidden command claims the prepared run, verifies protocol/mode, clears
     and verifies push, removes the address's bridge binding, then invokes one
     ordinary wait into the prepared directory.
   - Unix launches the current Telex executable directly.
   - Windows launches a Telex-generated PowerShell file to avoid the Copilot
     detached bare-executable failure mode.
4. **Durable completion handoff**
   - The existing wait artifact contract remains authoritative:
     `status.json` always, `delivery.json`/`message.json` on message delivery,
     and `exit.code` written last.
   - `wait.pid` identifies the active waiter for teardown and diagnostics.

### Design Decisions

**Mode enforcement belongs in the daemon.** The daemon understands only generic
on-deliver handlers and waiters; it does not learn about Copilot extensions or
heartbeats. Enforcing both illegal entry points makes mixed consumption
fail-closed for every harness.

**Preparation is side-effect-free for delivery.** A prepared command may never
start because the host task runner is unavailable or misconfigured. Push is
therefore left untouched until the detached run has actually claimed its
manifest.

**Transitions are asymmetric.** Push-to-pull uses in-place
`replace_on_deliver` immediately before waiting, minimizing the uncovered
window. Pull-to-push is intentionally explicit: `station stop` ends the waiter,
then push attach registers the bridge. Messages remain durable during that
temporary uncovered state.

**Mode and health are separate.** Status reports neutral `delivery_mode`
(`push`, `pull`, or defensive `conflict`) independently from
`station_health`. A pull station remains in pull mode between one-shot waits,
while health shows whether it is armed, recently delivered, or unattended.

**The launcher carries no address/session shell input.** User-controlled
identity and wait options remain in the private JSON manifest. Launcher commands
contain only the exact executable and generated run path, reducing
cross-platform quoting and injection risk.

### Integration Points

- `src/daemon_ipc.rs` defines protocol 1.4, delivery mode, conflict health, and
  additive registration replacement.
- `src/daemon.rs` enforces push/waiter exclusivity and derives status.
- `src/commands/copilot.rs` owns preparation, run claims, launchers, bridge
  teardown, and turn-guard conflict handling.
- `src/commands/wait.rs` remains the artifact authority and exposes a narrow
  internal path for setup failures to produce normal terminal artifacts.
- `copilot/COPILOT.md` is embedded into the binary and is the version-matched
  agent workflow.
- The root `SKILL.md` remains generic and harness-neutral.

## User Guide

### Prerequisites

- A current Telex binary and running daemon supporting protocol 1.4.
- `COPILOT_AGENT_SESSION_ID` supplied by Copilot CLI.
- A host task runner that can launch one fully detached command and notify the
  session when it exits.
- On Windows, `pwsh` for the generated PowerShell launcher.

### Basic Usage

When bridge provisioning or `extensions_reload` is unavailable:

```sh
telex --address <addr> copilot fallback prepare --description "<work>"
```

Read the returned JSON and run `launcher.command` as one detached task. Keep the
returned `run_dir`; it identifies the only artifacts that belong to that task.

After the completion wake:

1. Read `run_dir/exit.code`.
2. For exit `0`, read `delivery.json` and deduplicate by `message.id`.
3. Acknowledge the primary delivery:

   ```sh
   telex ack --address <addr> --session "$COPILOT_AGENT_SESSION_ID" --id <message-id>
   ```

4. Record any required workflow disposition.
5. Call `fallback prepare` again. Because the previous run is terminal, Telex
   creates a new unique run.

Preparation is idempotent while a run is unfinished. If a completion
notification arrives without `exit.code`, check status. A live waiter means the
original run is still active. Otherwise prepare again to retrieve the same run
and retry its generated launcher.

### Advanced Usage

Wait behavior is configured when preparing:

```sh
telex --address <addr> copilot fallback prepare \
  --timeout-ms 1800000 \
  --min-attention next-checkpoint \
  --wake-on-cc
```

`--force` records an intentional downgrade when a bridge heartbeat is still
live. It is not a default recovery step.

To return to push:

```sh
telex --address <addr> station stop --session "$COPILOT_AGENT_SESSION_ID"
telex --address <addr> copilot attach --copilot-bridge --description "<work>"
```

Then reload extensions. Push attach refuses to mutate a station while its waiter
is still live.

To end the fallback station, run `station stop` and do not prepare another run.

## API Reference

### Key Components

| Surface | Contract |
|---|---|
| `copilot fallback prepare` | Creates or returns the one current run and emits platform launcher metadata. |
| Hidden fallback runner | Claims the run, performs the push-to-pull transition, and runs one wait. |
| `Register.replace_on_deliver` | Additive protocol 1.4 flag that explicitly clears/replaces push during member refresh. |
| `delivery_mode` | Neutral status value: `push`, `pull`, `conflict`, or `unknown` for older peers. |
| `coverage_conflict` | Station-health tripwire for simultaneous push and waiter state. |
| Turn guard | Blocks current-protocol conflict and preserves existing mode-specific re-arm guidance. |

Prepare JSON includes:

- `mode` (`pull-fallback` at the Copilot boundary);
- `reused`;
- `run_id` and `run_dir`;
- `launcher.program`, `launcher.args`, and `launcher.command`;
- exact completion and payload artifact paths.

### Configuration Options

| Option | Effect |
|---|---|
| `--timeout-ms` | Idle timeout for this one wait; defaults to 30 minutes. |
| `--min-attention` | Minimum message attention that wakes the run. |
| `--wake-on-cc` | Wakes for live CC observer traffic without making it disposition-required. |
| `--description`, `--scope`, `--tags`, `--occupant` | Metadata used if fallback must attach a missing station. |
| `--force` | Allows an explicitly intentional transition away from a live bridge. |

## Testing

### How to Test

The process-level fallback test:

```sh
cargo test --no-default-features --features sqlite \
  --test daemon_process_sqlite \
  copilot_fallback
```

The cold-start case creates pull fallback without a prior bridge, then receives
and acknowledges two messages across two runs. The transition case provisions
push, executes the generated launcher, verifies pull mode and metadata
inheritance, sends and acknowledges through the exact run artifacts, re-arms,
rejects a duplicate launcher, rejects push while pull is live, then performs the
explicit stop-and-return-to-push sequence.

GitHub Actions runs this focused test on `macos-latest` and `windows-latest`.
The normal workspace suite exercises the Unix path on Linux.

### Edge Cases

- **Old daemon:** preparation/run fails closed with restart/update guidance
  before mixed delivery can start.
- **Healthy push:** fallback refuses unless `--force` was explicit.
- **Duplicate prepare:** returns the current unfinished run.
- **Duplicate launcher:** the run claim rejects it without replacing live or
  terminal artifacts.
- **Setup failure before wait:** writes `status.json` and terminal
  `exit.code=1`.
- **Push registration while pull is live:** rejected without stopping the
  waiter.
- **Wait while push is registered:** returns a terminal non-delivery outcome.
- **CC wake:** remains observer traffic; recipient-specific delivery metadata
  determines whether acknowledgement/disposition is required.

## Limitations and Future Work

- Pull fallback remains one-shot and agent-rearmed by design; it does not remove
  the Copilot turn-boundary responsibility that push avoids.
- Completed run directories remain under the local Telex home as operational
  artifacts. Automated retention/garbage collection is not included.
- Windows fallback depends on PowerShell because that is the proven Copilot
  detached-task compatibility path.
- A temporary uncovered interval is expected when explicitly stopping pull and
  returning to push; durable buffering prevents message loss.
