# Enforce session-binding in the holder

## Overview

The resident holder (`telex attach`) keeps an address's lease alive by heartbeating across agent
turns. If the holder is accidentally launched as a **detached/persistent** process (e.g. Copilot
CLI `detach: true`, or any "daemonize" path), it **outlives its session**, keeps heartbeating, and
the address falsely reports `occupied`/live forever — defeating the TTL backstop, because the
orphaned holder is exactly what keeps the heartbeat alive.

This work adds **defense-in-depth inside the binary**: the holder can be bound to the pid of its
launching session and, the moment that process dies, it **releases the lease and exits through the
normal shutdown path**. Even a mis-launched detached holder can no longer outlive its session. This
turns Decision 0004 (holder lifetime must track the session) from advisory into enforceable.

## Architecture and Design

### High-Level Architecture
- A new cross-platform liveness primitive, `telex::session_watch::process_alive(pid) -> bool`,
  answers "does this process still exist?" conservatively.
- When the holder is bound to a session pid, it spawns a small **watch task** alongside its existing
  heartbeat and poll tasks. The task polls `process_alive` on an interval; on a confirmed death it
  signals the holder's existing `state.shutdown`, which breaks the main `select!` loop and runs
  `release_lease` + IPC-endpoint cleanup — **the exact same tail as `detach`/ctrl-c**. There is no
  second release path.

### Design Decisions
- **Explicit binding only (`--session-pid` / `$TELEX_SESSION_PID`).** A parent-pid (ppid) default
  was deliberately **declined**: launchers that spawn-and-return (common for async/background
  launches) would leave the holder watching a dead or reparented parent and self-exit immediately,
  breaking the primary use case. The issue itself names ppid "the central risk." (DECISIONS 0010.)
- **Reuse `state.shutdown`** rather than add a release path, so session-death release is identical
  to `detach`/ctrl-c by construction.
- **Conservative liveness.** Only a definite "no such process" releases; an existing-but-unqueryable
  process (e.g. access-denied) or any ambiguous probe error counts as alive, so the holder never
  releases a lease on a transient error.
- **Runtime precedence, not clap `conflicts_with`.** clap 4 treats env-sourced values as "present"
  for conflict checks, so `conflicts_with` would reject `TELEX_SESSION_PID=… --no-session-bind`
  before the holder runs. Precedence is resolved in a pure, unit-tested helper instead.
- **Inherited-fd path deferred.** The pid-reuse-immune `--session-fd` upgrade is a documented future
  option; pid-watch satisfies every acceptance criterion today.

### Integration Points
- `src/commands/attach.rs` (holder): resolves the watched pid and spawns the watch task.
- `src/session_watch.rs` (new): the liveness primitive + pid-resolution helper.
- `src/cli.rs` (`AttachArgs`): the three new flags.
- No change to lease, heartbeat, delivery, or messaging semantics when the flags are unused.

## User Guide

### Prerequisites
You need the durable pid of the session/agent process that owns the holder (the process whose death
should free the address).

### Basic Usage
Bind the holder to your session so it self-releases when the session dies:

```sh
telex attach --address <addr> --description "<s>" --session-pid <your-session-pid>
# or, once per session:
export TELEX_SESSION_PID=<your-session-pid>
telex attach --address <addr> --description "<s>"
```

This is the belt-and-suspenders companion to launching the holder **background + session-bound** —
even if the holder is accidentally detached, killing `<your-session-pid>` releases the lease and the
address shows free within the liveness window.

### Advanced Usage
- **Tune detection latency:** `--session-poll-secs N` (default 2). The effective interval is clamped
  to the lease liveness window so the address always frees within it.
- **Run a deliberately persistent, server-side holder:** `--no-session-bind` runs an unbound holder
  that outlives its launcher; it overrides `--session-pid` / `$TELEX_SESSION_PID` and never errors,
  even when the env var is set.

## API Reference

### Key Components
- `telex::session_watch::process_alive(pid: u32) -> bool` — conservative cross-platform process
  existence check (`kill(pid,0)` on Unix; `OpenProcess(SYNCHRONIZE)` + `WaitForSingleObject` on
  Windows). Reusable by any code needing a "is this pid alive?" check.
- `telex::session_watch::resolve_session_pid(no_session_bind: bool, session_pid: Option<u32>) ->
  Option<u32>` — pure binding-precedence resolution (opt-out wins; `0` is an unbound sentinel).

### Configuration Options
| Flag / env | Default | Effect |
|---|---|---|
| `--session-pid <PID>` / `TELEX_SESSION_PID` | unset (no binding) | Holder watches this pid and self-releases on its death. |
| `--session-poll-secs <N>` | 2 | Liveness-check interval; clamped to the lease liveness window. |
| `--no-session-bind` | off | Run a persistent holder; overrides `--session-pid`/env. |

## Testing

### How to Test
1. Start a long-lived dummy process and note its pid (the stand-in "session").
2. `telex attach --address <addr> --session-pid <dummy-pid>` (against a scratch backend).
3. Confirm `telex status --address <addr>` shows `occupied=true`.
4. Kill the dummy process.
5. Within the liveness window, `telex status` shows `occupied=false` and the holder has exited; its
   log shows `[holder] session <pid> gone; releasing lease and exiting`.

### Edge Cases
- **Already-dead pid at startup:** the holder releases promptly (first poll tick fires immediately).
- **Access-denied / transient probe error:** treated as alive — no premature release.
- **`--session-pid 0` / `TELEX_SESSION_PID=0`:** treated as "no binding" (unbound sentinel).
- **`--no-session-bind` with `TELEX_SESSION_PID` set:** runs persistent, never errors.

## Limitations and Future Work
- **Pid reuse:** raw-pid watching can, in theory, bind to a recycled pid within the poll window. The
  deferred inherited-fd (`--session-fd`) path is the reuse-immune upgrade.
- **No zero-config binding:** binding is explicit by design (ppid-default declined). A future
  fd-based path could offer safe zero-config binding.
- **Holder-release path is validated manually + by unit tests** (liveness primitive, resolution
  precedence); a full spawned-holder integration test is recommended carry-forward.
