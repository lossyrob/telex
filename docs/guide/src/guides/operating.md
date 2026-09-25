# Operating telex

## The daemon lifecycle

The exchange auto-spawns on the first daemon-backed command; there is no manual
start. Inspect the resolved backend and address projection with:

```sh
telex status --address <addr>
```

The daemon runs per user. Inspect and control it with the `telex daemon` family
(run `telex daemon --help` for the full set):

```sh
telex daemon status            # daemon internals
telex daemon version           # running daemon version
telex daemon stop --drain      # stop after draining in-flight work
```

Runtime state (the IPC socket and lease state) lives in a per-user runtime
directory: on Windows under `%LOCALAPPDATA%\telex\run`, on Unix a per-user socket
directory. The local message store is the SQLite file at `~/.telex/telex.db`.

## Stopping a station

`station stop` is the symmetric inverse of the attach and wait loop. It marks the
station non-attending, releases membership durably, and waits for tracked waiters
to exit:

```sh
telex station stop --address <addr>
```

After it returns, a later message to the address stays queued until a future
attach or wait; it is not consumed by an orphaned waiter.

## Teardown: which command to use

| Command | Effect |
|---|---|
| `telex detach --address <addr>` | Drop this session's membership of the address, non-destructively. The station and durable buffer remain. |
| `telex station stop --address <addr>` | Mark the station non-attending, release membership durably, and wait for tracked waiters to exit. |
| `telex address retire --address <addr>` | Retire the address so it drops from directory listings. |
| `telex daemon stop --drain` | Stop the local exchange after draining in-flight work. |
| `telex copilot detach` | Copilot push sessions: detach the address and remove the bridge files. |

None of these delete durable messages; a later attach or wait resumes against the
retained buffer.

## Upgrading the binary

Release installs use a versioned layout instead of overwriting the binary on
`PATH` in place. A stable launcher lives under the install root's `bin/`, immutable
binaries live under `versions/<tag>/`, and `current` selects the version new
invocations use. Old in-flight processes keep running on their version while new
shells use the selected one.

```text
<install-root>/
  bin/telex(.exe)
  versions/<tag>/telex(.exe)
  current
  previous
```

Upgrade, roll back, and inspect versions:

```sh
telex version --json

# Fetch, verify, and install the latest compatible public release:
telex upgrade

# Install a specific public release by tag:
telex upgrade --version vX.Y.Z

# Install a local/manual build (no download):
telex upgrade --from <path-to-telex-binary> --version vX.Y.Z

telex rollback
telex gc --dry-run
```

Without `--from`, `telex upgrade` discovers a GitHub release (the latest full release by
default, or `--version <tag>`), selects this platform's asset, downloads the archive and its
`.sha256` sidecar, and **verifies the checksum before installing** — then installs through the
same versioned layout as the local path. It is **fail-closed**: a missing or mismatched
checksum, a missing platform asset, an unsupported platform, an incompatible version, or a
network/rate-limit error aborts without changing `current`. Set `GITHUB_TOKEN` to raise the API
rate limit. Prebuilt binaries are published for Windows (x86_64, ARM64), Linux (x86_64), and
macOS (Apple Silicon, Intel); on other platforms install from source with
`cargo install --git https://github.com/lossyrob/telex --features entra`. If telex is already on
the resolved release it reports "already current" and does nothing (override with `--force`).

`telex upgrade` reads the downloaded (checksum-verified) binary's own metadata by running it
once (`telex --json version`) before installing; in locked-down environments an OS quarantine
prompt (macOS Gatekeeper, Windows SmartScreen) on that step is the likely cause if an upgrade
stalls. The checksum verifies **integrity**, not authenticity — it protects against a corrupted
or truncated download, and the trust root is the GitHub repository the asset comes from.

`telex upgrade` and `telex rollback` drain the current local daemon before
switching `current`, unless `--skip-drain` is passed. Rollback refuses installed
versions whose manifest is incompatible with this build's protocol/schema floor.

For a manual in-place replacement, drain and replace in this order:

```sh
telex station stop --address <addr>
telex daemon stop --drain
# replace the telex binary
telex attach --address <addr> --description "<s>"
telex wait --address <addr> --out-dir <dir>
```

If a session resumes without an armed waiter, recovery is durable: inspect
`telex inbox --address <addr>` and `telex read --id <id>`, then arm a fresh wait.

## Auditing

Export messages and disposition history as JSON lines for provenance:

```sh
telex export --address <addr>
telex export --thread <id>
telex export --since <id>
```

## Push recovery after a daemon replacement

A push-attended station (a Copilot bridge bound with `telex copilot attach --copilot-bridge`)
records a durable **station intent**: the exact desired push registration for that
`(store, session, address)` binding. When a daemon is replaced — `telex upgrade`,
`telex daemon stop --drain`, or a crash — the successor restores the registration *by itself*, with
no manual `telex copilot resume`.

Restoration is not "replay a record". The successor restores a binding only after it has proved the
producer is alive: same user, matching executable, matching pid and process start time, matching
machine and boot, and an authenticated liveness probe whose nonce the producer must echo. A producer
that cannot be proved alive is never restored — messages stay durable and readable with
`telex inbox`, and the condition is reported rather than silent.

### Published recovery bounds

Both bounds are measured from "a compatible successor daemon is running **and** the producer is
live and verifiable", and both are derived from the reconciler's constants rather than asserted.

| Situation | Bound |
|---|---|
| Graceful drain or upgrade | **≤ 10 s** (one 5 s reconcile tick + a 1 s probe + a 2 s validation/claim allowance) |
| Hard crash | **`liveness_window_secs()` + 10 s** (the crashed daemon never released its lease, so the successor waits for it to go stale) |

Both bounds are **qualified**. They apply only when the automatic enumeration and required reads
complete, and to an intent that is:

- in a scope holding no more than **64** live intents (one pass budget), so it is attempted in the
  first pass after the trigger; and
- not currently in failure backoff, pull-waiter backoff, or quarantine.

They explicitly exclude: no successor daemon exists at all (start one with `telex attach`), a
backend outage, a competing fresh owner, and a producer too old to answer the liveness probe.

### Larger scopes

A scope holding more than one pass budget has a queue delay, not a fixed recovery bound. Each pass
still returns within four seconds and advances a scope-specific cursor, but bounded discovery,
backoff, quarantine, and slow intents mean there is no truthful minimum progress count per pass.
Use successive reconcile reports and `telex status` to observe progress rather than deriving a
completion time from the scope size.

If a report is truncated or degraded, its count and over-cap indication are lower bounds, not an
inventory or capacity-recovery promise. Automatic scans and GC cover records only when their
enumeration and read opportunities complete; they do not promise fair stable-tail progress, a
complete automatic GC, or exact capacity recovery.

For an exact inventory or eligible reclamation, use the explicitly offline path:

```bash
telex daemon stop --drain
# Stop other telex/Copilot processes that can write station intents.
telex --json daemon recover-intents       # exact inventory
telex --json daemon recover-intents --gc  # exact inventory, eligible GC, exact remaining count
```

The command refuses rather than guessing if a daemon is still reachable, its stopped state cannot be
established, the intent scope is absent, or the scope is not positively recognized as supported local
storage. Do not infer completeness from bounded reports.

At the 512-record write cap, existing records can still be updated or explicitly withdrawn. A live
record withdrawn to `revoked` continues to occupy its slot for the seven-day terminal TTL; daemon
GC frees it only after that TTL. Detach therefore does not free capacity immediately.

### Waiting is not failing

`telex --address <station> status` reports a station-intent row with a state. The one that most
often surprises people is `deferred_lease`: after a crash, the predecessor's epoch lease is not
stale yet, so the successor is **waiting for it to expire**, not failing. It retries at a fixed 5 s
cadence, never backs off exponentially, and never counts toward quarantine. `next_attempt_ms` says
when it will try again.

A genuine failure takes a jittered `5 s → 10 s → 20 s → … → 5 min` ladder, and after ten consecutive
failures drops to an hourly cadence. Repairing the *cause* does not leave you waiting out the
ladder: any durable change to the record — the turn-boundary producer refresh after a bridge
reload, a `copilot resume`, a re-attach — clears it, and the next tick attempts the binding again.

Three conditions are named explicitly in status:

- `live_intent_missing_member` — push is desired here but not currently armed.
- `member_missing_live_producer` — push is registered but the producer has gone quiet.
- `intent_protocol_incompatible` — this daemon cannot reconcile the recorded intent (schema skew, or
  a producer that predates the liveness probe).

### What recovery does *not* do

- **It never silently downgrades push to pull.** If a plain `telex attach` would create a pull-only
  member over a live push intent, the daemon either reconciles it to push or refuses with
  `Incompatible` / `PushIntentUnrecoverable`. It never creates the pull-only member.
  This guarantee is scoped to the case with **no live armed pull waiter**: an armed waiter still
  wins, and the intent waits rather than forcing the conflict.
- **It never returns an explicitly detached station.** `telex copilot detach` and
  `telex station stop` write a durable tombstone, and the reconciler honors it unconditionally.
  Explicit attach is the only way back.
- **It never crosses hosts.** Intents are local files bound to this machine and this boot, so a
  shared Postgres store — or a synced home directory — cannot let one host restore another's bridge.
- **It never re-arms a station you reset.** `telex station reset` (and `telex daemon reset`)
  withdraws the desired state as well as the membership: the affected intents are revoked, so the
  reconciler leaves the station idle. `telex --address <station> copilot resume` is the way back.
- **It never arms an attach that was never registered.** A running bridge is not by itself
  permission to turn a half-written attach into a live push binding. Finalizing one requires either
  a daemon that reports push armed for it right now, or the daemon's own durable record that it
  armed the binding earlier.
- **It never reports a push registration it cannot prove.** For a station that has a station-intent
  record, the daemon writes its durable "I armed this" proof *before* it installs the member, and
  refuses the whole registration if that write fails. So there is no state in which push is armed
  and nothing on disk says so — you either get a working, recoverable station or a typed refusal
  (`PushIntentUnrecoverable`) with no member created and any claimed lease released. A station that
  was already attended is left exactly as it was. A station with **no** intent record — a pull
  attach, or a plain `telex attach --on-deliver` — has nothing to prove and is never refused by
  this; that includes on a host where the intent scope could not be created at all.
- **It never re-uses a torn-down station's arming.** Re-attaching an address you detached (or that
  a session end revoked) starts a fresh attach: it gets the full attach window to finish, and it has
  to be armed again by the daemon before anything can promote it. The old record's proof is not
  carried over.
- **It never treats "I could not read that" as "there is nothing there".** Every recovery decision
  that turns on whether a station has a durable record — may this register commit, may this pull
  attach downgrade push, has this station's credential really been deleted — is decided from a
  *positive* answer. If the intent scope, a station's record, or a bridge credential exists but the
  operating system will not report on it (a permissions change, a lock, a profile on a volume that
  went away), telex refuses or waits rather than proceeding as if the station were new. You may see
  a `PushIntentUnrecoverable` refusal you would not have seen before; the alternative was a
  registration that reported success and left nothing recoverable behind.

### Recovering from a bridge reload

`extensions_reload`, `/clear`, and an extension-host restart all give the Copilot bridge a new
process. The daemon proves a producer by executable, pid, and process start time before it sends
anything, so a reloaded bridge no longer matches what the intent recorded, and status shows
`producer_identity_mismatch`.

This heals on its own: the next Copilot turn boundary re-records the running bridge's identity —
after proving it answers a liveness probe — and the following reconcile tick restores the binding.
It does **not** need a daemon that already has the station armed, which is the case that used to
deadlock: a reload followed by a daemon replacement leaves a successor that cannot create the member
because the identity is stale, and cannot refresh the identity if refreshing required the member.

If a session ends before a turn boundary happens, `telex --address <station> copilot resume` does
the same thing immediately.

### Before you drain

`telex daemon stop --drain`, `telex upgrade`, and `telex rollback` print a pre-drain intent report:

```
station intents  recoverable 2 pending 0 degraded 0 incompatible 0 unknown 0
```

`recoverable` is what a successor is expected to restore automatically. `pending` is a push attach
that has not finalized yet — it is **not** restored automatically; it finalizes at the next Copilot
turn boundary (after `extensions_reload`), and only then becomes recoverable. `degraded` and
`incompatible` need action: run `telex --address <station> copilot resume` after the switch.
Rolling back to a binary that predates this feature returns those stations to manual resume, and the
rollback output warns about it.

The report reads the durable records as well as the daemon's cached view, so a station you attached
seconds ago is counted correctly, and a station whose recorded producer identity was *just* repaired
(the usual case after `extensions_reload`) is counted as recoverable rather than carrying forward the
failure the repair fixed.

`telex upgrade` and `telex rollback` then drive one reconciliation pass on the successor by invoking
the binary the switch just selected (`telex daemon reconcile` on that binary). That indirection is
required, not cosmetic: the daemon only accepts IPC from a client whose executable matches its own,
so a pass requested from the pre-switch process would either spawn the wrong binary or be refused.
The successor summary distinguishes a pass that *ran and restored nothing* from one that never ran.

The `station_intent_reconcile` object in `--json` output always names the binary it is about
(`successor_binary`, `null` only for `--no-switch`), including when the step was skipped or the
successor failed. A successor that ran but could not complete a pass exits non-zero *and* reports
`{"reconciled": false, "error": ...}`; that structured reason is carried through verbatim (bounded)
rather than replaced with a generic rejection. A successor CLI child that overruns the bound is
killed and reaped before `upgrade`/`rollback` returns; a daemon it successfully spawned is a
separate detached service governed by the normal daemon lifecycle. The result records `timed_out`,
`terminated`, `reaped`, and the direct child's pid.

### Destructive testing

Station intents are namespaced by a hash of your user identity, your **canonicalized config root**,
and the protocol major. To experiment destructively without touching your real bindings, point
`TELEX_CONFIG` (and `TELEX_HOME`) at a scratch directory: the scratch daemon gets its own intent
scope, and nothing you do there can restore or revoke a station in your normal scope.

## Recovering from a lost daemon

A `wait` that finds no daemon exits with a distinct code (see
[Exit codes](../reference/exit-codes.md)). Run `telex attach` (the spawning and
recovery verb) and re-arm the wait. If a replacement daemon already exists, a
wait can reconnect during its bounded reconnect grace.

## Turn-end and resume reconciliation

For turn-end guards or resume reconciliation, use
`telex station status --session <id>` to get a compact JSON projection of the
session's attended addresses, waiter counts, station health, and pending
unconsumed counts.

## Uninstall and cleanup

1. Stop the daemon: `telex daemon stop --drain`.
2. Remove local state: delete `~/.telex/` (the SQLite store and config).
3. Remove the Copilot plugin, if installed: `copilot plugin uninstall telex@telex`.
4. For a Postgres backend, drop the telex schema in the database if it is no
   longer needed.
