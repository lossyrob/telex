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

## Corrections made during the final adversarial review

A society-of-thought final review (architecture, correctness, edge cases, security, testing,
release, plus an adversarial rubber-duck trace) found a further set of defects. All blocking, high,
and medium findings are fixed; each behavior change is recorded here because each one changes a
documented property.

### Lifecycle and GC

1. **GC deleted a first-attach `Pending` intent within ~60 s.** The credential-existence rule was an
   unguarded `else if` after the pending-TTL arm, and on a first attach the credential path is the
   bridge registry the extension has not written yet (the agent still has to run
   `extensions_reload`). The record the turn-boundary finalizer promotes was therefore gone before
   it could run, silently leaving push armed with **no durable intent** — the exact state the
   feature exists to remove. **Fix:** `Pending` is now governed by `STATION_INTENT_PENDING_TTL` and
   by nothing else. Tests:
   `station_intent::tests::gc_never_deletes_a_pending_intent_whose_producer_does_not_exist_yet`,
   `commands::copilot::tests::a_first_attach_pending_intent_is_valid_and_survives_gc`.
2. **GC deleted a `Live` intent whenever the registry was momentarily absent.** The bridge deletes
   and rewrites its own registry on `/clear`, `extensions_reload`, and extension-host restart, so
   any GC pass landing in that window destroyed a healthy recovery record — the only place a `Live`
   intent was destroyed with no tombstone and no TTL. **Fix:** the credential-absent rule now
   requires `STATION_INTENT_CREDENTIAL_MISSING_TTL` (15 min) measured from the last time the daemon
   *proved or attempted* the producer (`evidence`), not from manifest age.
3. **An orphaned `Live` intent could never expire.** Runtime failure states are deliberately never
   persisted, so the `STATION_INTENT_UNVERIFIABLE_TTL` orphan branch was unreachable for anything
   but `Revoked`; and because `apply_outcome` bumped `updated_at_ms` on every attempt, the TTL clock
   reset forever. **Fix:** evidence writes no longer bump `updated_at_ms`, and a finalized intent
   whose producer is provably dead expires on the orphan TTL.
4. **A rollback could delete newer-schema intents.** `UnsupportedSchema` fell into the
   unreadable-past-TTL sweep, contradicting the documented "intents are never deleted by a
   rollback". **Fix:** a manifest this build cannot read *because of its schema version* is kept
   forever. Test: `station_intent::tests::gc_never_deletes_a_manifest_from_a_newer_schema`.
5. **Interrupted-write debris was invisible to GC.** Orphaned `*.tmp` files (and now `*.lock`) are
   swept on the GC cadence.

### Retry policy

6. **Transient credential conditions were `Terminal`, parking a live binding for an hour.**
   `credential_stale`, `credential_age_unknown`, `credential_unreadable`, `credential_malformed`,
   `credential_field_missing`, and `credential_unresolved` are all ordinary conditions of a *live*
   producer whose registry is rewritten on a 15 s heartbeat. **Fix:** retry policy and projected
   state are now separate axes — these return `IntentOutcome::Failed` (still projecting
   `Unverifiable`) and take the 5 s → 5 min ladder, while the security and genuinely-inert classes
   (`credential_outside_root`, `credential_insecure`, `credential_root_unregistered`,
   `foreign_host_or_boot`, `handler_kind_unregistered`, `legacy_producer`, `store_missing`,
   `store_selector_unresolved`) stay `Terminal`. `RegistryError` also gained
   `ContainmentUnreadable`, so an absent file during a bridge reload is no longer collapsed into a
   *security* verdict. Tests:
   `station_intent_transient_credential_conditions_take_the_backoff_ladder`,
   `daemon::reconcile::tests::a_transient_credential_condition_is_not_a_terminal_outcome`.
7. **The bridge registry was rewritten non-atomically.** `writeFile` truncates first, so a read
   landing in that window saw a partial document. **Fix:** `writeRegistry()` writes a temp file and
   renames. Test: `probe-protocol.test.mjs` "the registry is written atomically…".
8. **Retry state did not survive a daemon replacement.** The durable `evidence` block was written
   every pass and never read back, so backoff and quarantine reset on the event most likely to
   follow a crash loop, and `recovery_latency_ms` restarted the clock on the exact event it
   measures. **Fix:** the index seeds `attempts`, `consecutive_failures`, `next_attempt_ms`,
   `last_attempt_ms`, `last_success_ms`, `producer_verified_ms`, and `failure_code` from the
   manifest on first sight. Test: `station_intent_retry_state_survives_a_daemon_replacement`.
9. **The healthy steady state rewrote every manifest every 5 s.** **Fix:** evidence is persisted
   when a scheduling-relevant field changes, and otherwise at most once per
   `EVIDENCE_REFRESH_INTERVAL` (60 s) so `last_success_ms` stays a usable clock for GC. Test:
   `daemon::reconcile::tests::unchanged_evidence_is_not_rewritten_on_every_tick`.

### Delivery correctness

10. **The CC watermark was frozen at attach time.** The durable value was written once at finalize
    while the member's `on_deliver_cc_after_ms` advanced in memory and died with the daemon, so
    every replacement re-injected the whole session's CC history as agent turns. **Fix:**
    `apply_outcome` refreshes the durable watermark from the live member on a successful outcome,
    and every restore takes `max(existing_member, manifest)` so a live member is never rewound.
    Test: `station_intent_cc_watermark_is_refreshed_and_never_rewound`.
11. **`telex station reset` was undone by the reconciler within a tick.** Reset was the one
    deliberate operator action with no durable marker. **Fix:** `reset_station` revokes the affected
    intents (exact per binding, reversible with `copilot resume`), and the reconciler treats an
    `idle && !idle_rearmable` member as `Terminal(Revoked, "operator_reset")` as a second line of
    defence. Test: `station_intent_operator_reset_is_not_undone_by_the_reconciler`.
12. **Two sessions could both attend one address.** The per-address dedupe was keyed on the full
    `(store, session, address)` tuple — which no two manifests can share, so it rejected nothing —
    and the `AlreadyOwned` self-adoption branch treated "this daemon owns the address" as licence to
    adopt it for a second session. **Fix:** dedupe on `(store_key, address)`, and adopt a
    self-owned lease only when no other session in this daemon holds the address. Test:
    `station_intent_two_sessions_never_both_attend_one_address`.
13. **A stale `--daemon-instance` fence dead-lettered every message permanently.** The fence value
    is read before `Register`; if the daemon is replaced in that window the handler names a dead
    instance and `copilot push` returns `PUSH_EXIT_PERMANENT`, which the daemon treats as
    non-retryable — and the reconcile no-op short-circuit meant the argv was never rebuilt.
    **Fix:** attach compares `MemberStatus::owner_instance_id` against the baked fence and fails
    closed on a mismatch, and reconciliation repairs a member whose stored argv differs from the
    argv this daemon builds.
14. **The turn guard's new branch suppressed every other coverage check.** **Fix:** the
    unrestored-intent branch moved to just before the `covered` early return, guarded on all
    coverage sets being empty; the mixed case reports both. Tests:
    `guard_reports_an_unrestored_intent_without_suppressing_a_real_coverage_gap`,
    `guard_reports_an_unrestored_intent_when_nothing_else_is_uncovered`.
15. **A scoped `ReconcileIntents` advanced the shared cursor past out-of-scope intents.** **Fix:**
    `last_considered_position` is assigned below the scope filter.
16. **The heartbeat loop could be starved by trigger pulses.** The tick was recreated each
    iteration, so a pulse stream faster than `HEARTBEAT_INTERVAL` starved `heartbeat_members_once`
    and let epoch leases go stale. **Fix:** the heartbeat deadline is tracked explicitly and
    anchored, so a trigger wake never skips a due heartbeat.

### Safety and fail-closed posture

17. **The anti-downgrade guard failed *open* when the intent scope could not be opened.** Both
    inputs were false in exactly the window the guard exists for. **Fix:** `lookup_live_intent`
    returns `Live` / `Absent` / `Unavailable`, and `Unavailable` fails closed with
    `PushIntentUnrecoverable`.
18. **Read paths created the intent scope as a side effect.** **Fix:** `intent_store_readonly` uses
    `open_existing`, and `intent_statuses` / `pending_push_intent` / the anti-downgrade lookup use
    it. The comment now matches the code.
19. **The credential age check and the read used two separate opens.** **Fix:** one
    `read_owner_only_file_with_meta` handle, per Plan decision 2. The trade-off is recorded in the
    code: the bytes reach memory, but the age gate runs before the secret is extracted, connected
    to, or sent.
20. **`write_cas` was a read-then-write.** **Fix:** every mutating entry point takes a bounded,
    stale-tolerant per-intent write lock, and `finalize_intent` goes through `update_locked`
    instead of a bare `write_atomic` with no CAS at all.
21. **Windows: the ACE allowlist had been silently widened** to accept `ALL APPLICATION PACKAGES`
    (`S-1-15-2-*`) and capability SIDs on the two *validate-only* paths this feature added — the
    producer credential read and the existing bridge root. **Fix:** the enforced allowlist is back
    to current user / `SYSTEM` / `Administrators` / logon-session SID.
22. **Windows: `boot_id` was not stable within one boot.** It was derived as
    `SystemTime::now() - GetTickCount64()`, which jitters across a second boundary and shifts on any
    clock step; a one-second divergence between the CLI and the daemon made every intent
    `foreign_host_or_boot` and, through the anti-downgrade guard, blocked unrelated `telex attach`
    calls. **Fix:** the identifier is minted once per boot and persisted owner-private under
    `%LOCALAPPDATA%\telex\boot-id.json`, validated against monotonic uptime *and* a tolerant boot
    instant, claimed with create-new semantics so concurrent minters converge, and cached per
    process. Test: `platform_fs::tests::boot_identity_is_stable_across_repeated_independent_resolutions`.
23. **Windows: an atomic intent write failed when any reader held the file open.**
    `open_owner_only_file` used `FILE_SHARE_READ` only, so `rename` returned `ERROR_ACCESS_DENIED`
    (mapped to `PermissionDenied`, which the fallback branch did not handle). **Fix:**
    `FILE_SHARE_DELETE | FILE_SHARE_WRITE` on the read handle, plus a bounded
    `PermissionDenied` retry in `write_owner_only_file_atomic` for foreign readers.
24. **The probe response was read uncapped and its error code retained verbatim.** A 1 MiB `error`
    string would push every `StatusReport` past `MAX_JSONL_FRAME_BYTES` and break `telex status`
    for the whole daemon until restart. **Fix:** `PROBE_MAX_RESPONSE_BYTES` frames the read and
    `sanitize_failure_code` clamps the code to 64 `[a-z0-9_]` characters.
25. **`copilot detach` with the daemon down revoked nothing.** The `?` propagated the connect error
    and skipped every local teardown step, so the `Live` intent survived with no tombstone and the
    next daemon start auto-returned a station the user had detached. **Fix:** the local revoke and
    bridge-binding teardown run on the error path too, with an explicit "the durable tombstone was
    NOT written" message and a non-zero exit.
26. **The attach rollback removed a bridge binding it did not create.** That silently decremented
    the ref-count, so a later detach of an unrelated address deleted the shared bridge and registry
    out from under a live push station. **Fix:** `BridgeBindingWrite` mirrors `PendingIntentWrite`;
    rollback removes only what the invocation added.
27. **`copilot gc` failed open on a per-file intent read error.** **Fix:** a per-file failure is as
    disqualifying as an unreadable scope; `station_intents_readable` reflects it, a new
    `station_intents_unreadable` count is reported, and binding/intent drift is now reported in both
    directions.

### Operator-facing reporting

28. **`Pending` was counted as `recoverable` in the drain report.** It is never reconciled, so the
    pre-drain signal overstated what a successor restores and `upgrade` burned its successor
    timeout on it. **Fix:** `DrainIntentReport::pending` is its own counter, rendered separately by
    `daemon stop --drain`, `upgrade`, and `rollback`.
29. **Rejected manifests were invisible.** The comment claimed they were indexed; no `index_upsert`
    happened, so `telex status` and the drain report showed nothing for exactly the intents they
    exist to flag. **Fix:** `ScanPage::rejected` carries the binding identity when the document
    parsed far enough to know it, those entries are indexed (highest-precedence-wins), and
    `DrainIntentReport::unidentifiable` counts the rest and folds them into `degraded`. Test:
    `station_intent_rejected_manifests_are_visible_in_status_and_the_drain_report`.
30. **A skipped pass was indistinguishable from an empty one.** **Fix:** `ReconcileReport::ran` and
    `skipped_reason`. Test: `station_intent_a_suppressed_pass_is_reported_as_not_run`.
31. **`upgrade`/`rollback` spawned the *pre-switch* binary as the "successor".**
    `connect_or_spawn` spawns `current_exe()`, which during an upgrade is
    `versions/<old-tag>/telex`; the daemon left running was the old binary, and because
    `connect_existing` requires the server executable to match the client's, every subsequent
    client (all the new binary) got `Unauthorized` from a daemon with no idle shutdown — including
    `telex daemon stop`. On rollback it resurrected the binary being rolled back from. **Fix:** a
    new `telex daemon reconcile` subcommand, invoked as a child process of the **newly selected**
    binary, which retries a draining predecessor and a pass that did not run.
32. **The successor probe reported success on a zeroed report.** Covered by 30 and 31 together:
    `daemon reconcile` retries until a pass reports `ran`.
33. **`CAP_STATION_INTENT` was advertised with no consumer.** **Fix:** `DaemonStatus` carries the
    daemon's capability list and `ensure_reconcile_capability` gates on it as well as on the minor.
34. **Following the release runbook broke the release-contract test.** It hardcoded both sides of
    every comparison, so rolling the fixture forward as the checklist demands turned CI red with no
    instruction to also edit the test. **Fix:** the fixture declares `expected_movement`
    (`unchanged` / `changed` / `introduced`) and the test asserts that relationship; the runbook
    step now says to reset it.
35. **`over_cap` was off by one** against the write cap (`>` vs `>=`).
36. **`Revoked` / `Tombstoned` precedence and doc comments were swapped** relative to Plan decision
    16. Corrected; `Tombstoned` is documented as reserved, since this build persists and projects
    `Revoked` for both causes.

### Test-integrity fixes

37. `station_intent_trigger_seam_drives_a_pass_without_a_wall_clock_sleep` always timed out and
    fell back to calling `reconcile_once` directly, so the seam could have been entirely unwired
    with the test green (and it was the slowest test in the suite). `TestDaemon` gained
    `spawn_trigger_consumer`, and the test now asserts the pulse drives a real pass.
38. The Postgres `DeferredLease` assertion was `deferred_lease + failed == 1`, which passes for the
    exact regression that matters (classifying a fresh incumbent as `epoch_claim_lost` breaks the
    published crash bound). Tightened to `deferred_lease == 1 && failed == 0`, plus assertions that
    the failure counter did not advance and the retry is the fixed cadence.
39. The over-budget test summed *attempts* rather than distinct intents, so a stalled cursor
    re-attempting the same 64 intents satisfied it. It now counts distinct attempted bindings.
40. macOS ran only the three process-level station-intent tests; the credential rules, probe
    transports, and scan cursor all live in the core suite. CI now runs `--test station_intent`
    there too, and Node is pinned (the bridge-contract step needs Node >= 21 for its own glob
    expansion).

## Corrections made during implementation review

Six defects an adversarial review of the diff surfaced, all fixed. Recorded because each is a
subtle failure mode that a passing test suite did not catch, and the reasoning is worth keeping.

1. **The scan cursor advanced past intents the pass never attempted.** `IntentStore::scan` persisted
   the cursor at the last entry it *loaded*, but a pass attempts only what fits inside
   `RECONCILE_PASS_DEADLINE`. A truncated pass therefore skipped everything between — permanently.
   Worse, for a scope smaller than the budget the cursor always landed on the maximum, `start` reset
   to 0 every pass, and the round-robin property the queue-delay bound rests on did not exist at
   all: four permanently-`DeferredLease` intents would occupy every wave forever while the tail
   never ran. **Fix:** `scan` reads the cursor and exposes `loaded_positions`; the reconciler calls
   `advance_cursor` with the last intent it actually *attempted* (or, if it attempted none, the last
   it considered, so a pass of entirely inert entries still makes progress). Regression test:
   `station_intent_cursor_advances_only_past_attempted_intents`.
2. **Admission-guard acquisition sat outside the per-intent timeout.** `register_member` holds the
   same per-`MemberKey` guard across backend work, so one slow station could block a wave task
   indefinitely; `reconcile_once` joins every handle, and `heartbeat_loop` awaits the pass inline,
   so a single blocked station could stop *every* member's epoch heartbeat and let leases go stale.
   **Fix:** guard acquisition and the reconcile call are now inside one
   `tokio::time::timeout(RECONCILE_PER_INTENT_TIMEOUT, …)`.
3. **The anti-downgrade guard was inert until the first pass.** It gated on the in-memory index,
   which only a completed reconcile pass populates — and `serve()` accepts connections before that
   pass runs, which is exactly the daemon-replacement window the guard exists to protect. **Fix:**
   the guard consults the durable manifest (`load_live_intent`) and treats an index hit with an
   unreadable manifest as fail-closed. Regression test:
   `station_intent_anti_downgrade_guard_works_before_the_first_reconcile_pass`.
4. **Intent `generation` was reset to 1 on every pending write.** With finalize bumping to 2, the
   generation cycled `1 → 2 → 1 → 2` and was not monotonic, so a reconcile pass holding generation 2
   could pass its compare-and-set against a *newer* manifest that had cycled back to 2 and clobber a
   fresh producer descriptor with a stale one — after which every probe failed
   `producer_identity_mismatch` and the station was parked for an hour. **Fix:**
   `write_pending_intent` builds on any existing generation. Regression test:
   `station_intent_generation_never_resets_so_a_stale_pass_cannot_clobber_a_resume`.
5. **A re-attach demoted a working `live` intent to `pending`.** If the subsequent finalize failed
   (bridge mid-reload, probe rate limit) the attach still succeeded, leaving push working *now* with
   a `pending` record GC deletes five minutes later — precisely the "works now, no recovery later"
   state this feature exists to remove. **Fix:** `write_pending_intent` leaves an existing `live`
   intent alone and lets finalize update it in place, and the attach rollback removes only an intent
   that invocation actually created.
6. **A transient backend error was classified terminal.** Any `backend_for` failure — a Postgres
   connect blip, a failover — mapped to `Terminal(Unverifiable)`, which parks an intent for the
   one-hour quarantine retry with no self-healing path, inconsistent with every other backend error
   in the same function. **Fix:** only a genuinely absent store (`store_missing`) is terminal; a
   connection error takes the ordinary 5 s → 5 min failure ladder.

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
- `telex copilot gc` reads station intents **first**: a session named by a non-revoked intent is
  kept even when its bridge heartbeat is stale, because deleting the bridge under a live intent is
  the one action GC could take that recovery cannot undo. `.bindings.json` is a secondary hint, and
  drift between the two (`binding_intent_drift`) is *reported*, never silently repaired. An
  unreadable intent scope is reported as `station_intents_readable: false` rather than treated as
  "no intents" — an unreadable scope must not become a licence to delete a live session's bridge.
- A `pending` intent (a push attach that has not finalized) makes an unattended `wait`/`send` return
  `NeedsAttach` with reason `PushIntentPending` instead of silently re-registering. `wait` and
  `send` render it as "run `extensions_reload`, or re-run `copilot resume`" and stop, because a
  generic retry there would race the in-flight attach.

## Verification

- `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`
- `cargo test --workspace` (includes `tests/station_intent.rs`, 20 daemon-core rows)
- `cargo test --no-default-features --features sqlite --test daemon_process_sqlite station_intent_`
- `cargo test --all-features --test conformance --test daemon_core_postgres` (Postgres rows skip
  cleanly when `TELEX_PG_URL` is unset)
- `node --test "copilot/bridge/*.test.mjs"`
- Feature-matrix builds: `--no-default-features --features sqlite`, `--features postgres`,
  `--features entra`, `--features "sqlite,self-update"`
