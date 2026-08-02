# Docs — Station Intent Reconciliation (issue #106)

As-built technical reference for the change. Written after implementation, so where the plan and the
code diverge this file records the code and the reason.

## What the change is

Push delivery arms a station by putting an `on_deliver` handler into the daemon's in-memory
`MemberRecord`. That record does not survive a daemon replacement, so `telex upgrade`,
`telex daemon stop --drain`, and a crash all silently convert a push-attended Copilot session into
an unattended one. Messages stayed durable; nothing arrived as a turn until a human noticed and ran
`telex copilot resume`. An idle session is exactly the case where nobody notices.

This change adds a durable, host-local, owner-private, versioned **station intent** — the exact
desired push registration for one `(store_key, session_id, address)` binding — plus a daemon-owned
reconciler that restores it *after proving the producer is alive*.

The distinction that governs the whole design: an intent is **desired state**, never attendance.
In-memory `MemberRecord` plus the backend epoch lease remain the only authority for who attends an
address and who may deliver to it. Restoration requires a live, verified producer, which is what
keeps this from being the "rebuild membership from history" that ADR 0023 forbids.

## Module map

| File | Responsibility |
|---|---|
| `src/platform_fs.rs` | Owner-private filesystem primitives promoted out of `daemon::platform`, plus a fail-closed per-file read, canonical containment, and shared process/host/boot identity. `daemon::platform` re-exports them, so daemon behavior is unchanged. |
| `src/station_intent.rs` | The V1 manifest, hashed intent ids, the owner-private store, bounded scanning with a persisted round-robin cursor, and GC. |
| `src/handler_kinds.rs` | Generic handler-kind and producer-root registries; the single shared `build_push_argv`. |
| `src/daemon_reconcile.rs` | The reconciler: constants and published bounds, outcome classes, wave scheduling, member creation, the cached index, the event log, and the trigger/report seam. Physically a sibling file, mounted as `daemon::reconcile` so it can use daemon-private state without widening that surface; re-exported at `crate::daemon_reconcile`. |
| `src/intent_test_support.rs` | Fake producer endpoint + intent fixtures. |
| `copilot/bridge/probe-protocol.mjs` | The pure, SDK-free probe wire contract (the `busy-state.mjs` precedent). |

## Lifecycle

### Attach

1. `provision_bridge` writes the extension and records the binding.
2. **Before `Register`**, a `pending` intent is written. Order matters: a crash between here and a
   successful register leaves a record the daemon never acts on and GC removes. The reverse order
   would leave a window where push is armed with no durable record of the desired state.
3. `Register` arms push.
4. After `daemon_armed_push` confirms it, telex runs *the same probe the daemon will*, re-captures
   producer identity, records the member's actual `cc_watermark_ms`, and finalizes the intent to
   `live` with `generation + 1`.
5. Any failure rolls back: the intent is removed **first**, then the bridge binding, because a
   half-armed bridge with a leftover intent is the one shape that could later claim a station the
   user never successfully attached.

### Reconcile

Ordered, all fail-closed, and each step maps to exactly one projected state:

| Step | Failure → |
|---|---|
| Persisted state is `live` | anything else → surfaced, never reconciled |
| Host and boot identity match | `unverifiable` / `foreign_host_or_boot` |
| Handler kind registered | `incompatible` / `handler_kind_unregistered` |
| Producer protocol ≥ 2 | `legacy_producer` |
| No live armed pull waiter | `deferred_pull_waiter` |
| Store selector resolves | `unverifiable` / `store_selector_unresolved` |
| Credential root registered, path contained | `unverifiable` / `insecure` |
| Credential inside `max_age_ms` | `unverifiable` / `credential_stale` (no secret read, no connection) |
| `verify_server_peer` before sending | `unverifiable` / `producer_identity_mismatch` |
| Probe: nonce echoed, session matches | `failed` / `probe_*` |
| Tombstone pre-check, claim, tombstone re-check | `revoked`, `deferred_lease`, or `failed` |

### Teardown

Detach and session end both revoke exactly the affected intent(s). Ordering is **durable tombstone
first, local intent revocation second**, so a crash between them leaves tombstone-wins — which the
reconciler already honors.

## Safety properties and how each is enforced

| Property | Enforcement |
|---|---|
| An explicitly detached station never returns | Unconditional tombstone check before *and* after the epoch claim; `daemon_reconcile.rs` contains no `clear_detach_tombstone` call, asserted structurally by a unit test against the source |
| No secret is ever persisted | The intent carries a *pointer*; the credential is read fresh, age-checked before the read, and dropped immediately after the probe |
| No arbitrary argv | The manifest carries a handler *kind* and a validated `session_id`; argv is rebuilt by one shared builder from the daemon's own executable and store resolution |
| The daemon stays harness-agnostic | Composition-time registries; the daemon core sees opaque ids, never a Copilot path or symbol |
| Cross-host isolation | Intents are local files, additionally bound to `host_id` and `boot_id` |
| Single writer | A reconciled restore claims the epoch lease exactly as an ordinary register does; an incumbent is never force-stolen |
| CC messages committed during the gap stay visible | `cc_watermark_ms` is captured at finalize and *passed through*, never recomputed as "now" |
| One pass cannot overrun its tick | `RECONCILE_PASS_DEADLINE` (4 s) < `RECONCILE_INTERVAL` (5 s), enforced by a `const` assertion |
| A wedged intent cannot starve others | Per-intent timeout, per-pass budget, exponential backoff on genuine failures only, quarantine after 10 |
| Push is never silently downgraded to pull | Anti-downgrade guard inside `register_member`, so it also covers old clients and plain `telex attach` |
| A recovery-state condition never wedges a session | The turn guard *warns and allows* |

## Published bounds

Derived from the constants, not asserted:

- **Graceful drain / upgrade:** `RECONCILE_INTERVAL (5 s) + BRIDGE_PROBE_TIMEOUT (1 s) + validation
  and claim allowance (2 s)` = ≤ 8 s, documented conservatively as **≤ 10 s**.
- **Hard crash:** `liveness_window_secs() + graceful bound` — the crashed predecessor never released
  its lease, so every attempt inside the liveness window is `DeferredLease` (a *waiting* state on a
  fixed 5 s cadence, never backed off, never counted toward quarantine). Documented as
  `liveness_window_secs() + 10 s`, never as a literal.

Both are qualified to an intent in a scope holding ≤ `RECONCILE_PASS_BUDGET` (64) live intents and
not currently backed off or quarantined. Larger scopes get a *queue delay* ceiling instead:
`ceil(N / RECONCILE_MAX_CONCURRENCY) * RECONCILE_INTERVAL`.

`graceful_recovery_bound_ms()`, `crash_recovery_bound_ms()`, and `max_queue_delay_ms(n)` are public
functions, so docs and tests read the bound rather than restating it.

## Wire and release surface

| Axis | Before | After |
|---|---|---|
| `STATION_INTENT_SCHEMA_VERSION` | — | `1` (supported range `1..=1`) |
| `COPILOT_BRIDGE_PROTOCOL` | `1` | `2` (adds `probe`) |
| `PROTOCOL_MINOR` | `4` | `5` (adds `ReconcileIntents`, intent status rows) |
| `MIN_COMPATIBLE_PLUGIN_VERSION` | `0.1.0` | **unchanged** — the plugin hook surface is untouched |

New wire types: `IntentRecoveryState`, `IntentStatus`, `ReconcileReport`, `DrainIntentReport`,
`Request::ReconcileIntents`, `Response::Reconciled`, `NeedsAttachReason::{PushIntentPending,
PushIntentUnrecoverable, Unknown}`, and an optional `drain_intents` field on `Response::Ack`.
Everything additive; older clients ignore fields they do not know.

## Deviations from the plan, and why

Four, all direction-preserving. Each is recorded here and in `Plan.md`.

### 1. Producer roots are validated, not rewritten (Windows)

**Plan:** harden the Copilot bridge root with `platform_fs::ensure_owner_private_dir`.

**Problem, found empirically:** on Windows that applies a *protected* DACL, which re-propagates
inheritance to children. Files whose access came purely from inherited ACEs — every file the bridge
wrote before telex touched the directory — are left with an empty DACL and become unreadable **to
everyone, including their own author**. A process-test reproduction showed `read` on a pre-existing
`.bindings.json` failing with `Access is denied` immediately after hardening.

**Resolution:** `ensure_owner_private_producer_root` is create-strict, validate-existing. A root
telex creates gets the owner-only descriptor and has no children to strip. A root that already
exists is *validated*: owner must be the current user and every allowed ACE must name the current
user, `SYSTEM`, local `Administrators`, the logon-session SID (`S-1-5-5-*`, which Windows puts in a
token's default DACL), or an AppContainer SID. `Everyone`, `Authenticated Users`, `Users`, and any
foreign SID fail closed. The posture is unchanged; the producer's own files keep working. Per-file
credential checks apply independently, so containment in this directory is never the only thing
being trusted. Regression test:
`platform_fs::tests::producer_root_hardening_never_strips_an_existing_producer_file`.

### 2. Intent finalization also happens at the turn boundary

**Plan:** attach finalizes `pending` → `live` after `daemon_armed_push`.

**Problem:** on a *first* attach the bridge extension has been written but not loaded — the agent
still has to run `extensions_reload` — so there is no live producer to probe or describe. Attach
alone therefore cannot finalize the very first binding, which is the common case.

**Resolution:** `ProducerDescriptorV1::validate` requires concrete producer identity only when the
state is not `Pending` (safe, because a `Pending` intent is never reconciled), and the `agentStop`
drain hook — which already runs every turn and is already the explicit reconcile trigger of decision
14(d) — finalizes any `pending` intent for the session once the bridge is answering. Recovery is
therefore armed within one turn of the reload, with no new command and no new lifecycle.

### 3. `--daemon-instance` requires connecting before provisioning

**Plan:** attach sources `instance_id` from the cap file "on the connection it already makes".

**Problem:** `provision_bridge` builds the handler argv *before* `attach::run` connects, so on a
cold start there is no cap file yet and every bridge attach failed.

**Resolution:** `daemon_instance_id` calls `connect_or_spawn` first, then reads the cap file. This
adds no new lifecycle — attach connects on the very next step regardless — it just moves the connect
one step earlier so the argv the daemon stores and the argv a later reconcile rebuilds are
byte-identical.

### 4. Self-owned lease adoption

**Plan:** `AlreadyOwned` is either `DeferredLease` (not stale yet) or `Failed` (fresh competing
owner).

**Problem:** if the daemon holding the lease is *this* daemon and it has no in-memory member, both
branches are wrong — it would defer forever against itself, wedging the binding.

**Resolution:** when `owner_instance_id == state.instance_id`, adopt the lease already held. No new
claim, no steal, and the post-claim tombstone re-check still runs.

### Minor

- The plan's `--drain-timeout-ms` flag does not exist in this codebase. The bounded in-flight
  handler wait uses a named constant (`DRAIN_INFLIGHT_WAIT`, 5 s) instead of inventing a flag.
- `node --test copilot/bridge` (directory form) does not resolve on Windows runners; CI uses
  `node --test "copilot/bridge/*.test.mjs"`, which Node expands itself on both platforms and which
  satisfies the same requirement (every `*.test.mjs` under `copilot/bridge` runs).
- SHA-256 is implemented in `platform_fs` rather than pulled from `sha2`, which is only in the
  dependency graph behind the optional `self-update` feature. Intent identity must be identical in
  every feature combination, including `--no-default-features --features sqlite`.

## Operating notes

- Intent scopes are namespaced by user identity, **canonicalized config root**, and protocol major.
  Point `TELEX_CONFIG` / `TELEX_HOME` at a scratch directory to test destructively without touching
  real bindings.
- The reconcile event log is NDJSON at `<run_dir>/intents/<singleton_hash>/reconcile-events.ndjson`,
  with a single rotation at 1 MiB. No secrets, no credential paths, no raw argv.
- `telex --address <addr> status` surfaces intent rows including **intent-only** rows with no
  member, plus the three issue-named conditions `live_intent_missing_member`,
  `member_missing_live_producer`, and `intent_protocol_incompatible`.
- `telex daemon stop --drain`, `telex upgrade`, and `telex rollback` print a pre-drain intent report
  computed from the cached index only — no directory scan, no probe — so it cannot slow a drain.

## Verification

- `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`
- `cargo test --workspace` (includes `tests/station_intent.rs`, 20 daemon-core rows)
- `cargo test --no-default-features --features sqlite --test daemon_process_sqlite station_intent_`
- `cargo test --all-features --test conformance --test daemon_core_postgres` (Postgres rows skip
  cleanly when `TELEX_PG_URL` is unset)
- `node --test "copilot/bridge/*.test.mjs"`
- Feature-matrix builds: `--no-default-features --features sqlite`, `--features postgres`,
  `--features entra`, `--features "sqlite,self-update"`
