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

Both bounds are **qualified**. They apply to an intent that is:

- in a scope holding no more than **64** live intents (one pass budget), so it is attempted in the
  first pass after the trigger; and
- not currently in failure backoff, pull-waiter backoff, or quarantine.

They explicitly exclude: no successor daemon exists at all (start one with `telex attach`), a
backend outage, a competing fresh owner, and a producer too old to answer the liveness probe.

### Larger scopes

A scope holding more than one pass budget gets a computable *queue delay* rather than a recovery
bound. An intent waits at most

```
ceil(live_intents / 4) * 5 s
```

before it is attempted, where 4 is the guaranteed per-pass progress in the pathological case where
every intent consumes its full timeout (a healthy pass drains up to 64). Its own recovery then
completes within the applicable bound above. At the 512-intent per-scope cap that is ≤ 640 s in the
pathological case and ≈ 40 s in the healthy case.

### Waiting is not failing

`telex --address <station> status` reports a station-intent row with a state. The one that most
often surprises people is `deferred_lease`: after a crash, the predecessor's epoch lease is not
stale yet, so the successor is **waiting for it to expire**, not failing. It retries at a fixed 5 s
cadence, never backs off exponentially, and never counts toward quarantine. `next_attempt_ms` says
when it will try again.

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

`telex upgrade` and `telex rollback` then drive one reconciliation pass on the successor by invoking
the binary the switch just selected (`telex daemon reconcile` on that binary). That indirection is
required, not cosmetic: the daemon only accepts IPC from a client whose executable matches its own,
so a pass requested from the pre-switch process would either spawn the wrong binary or be refused.
The successor summary distinguishes a pass that *ran and restored nothing* from one that never ran.

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
