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
3. `Register` arms push. For a binding that has a station-intent record, the daemon commits its
   durable **armed proof** onto that record *before* it installs the member — after every fallible
   step, so a successful stamp is followed only by an infallible in-memory commit. A register that
   owes a proof and cannot persist it is refused (`Incompatible` / `PushIntentUnrecoverable`), with
   the lease it claimed released and no member created.
4. After `daemon_armed_push` confirms it, telex runs *the same probe the daemon will*, re-captures
   producer identity, records the member's actual `cc_watermark_ms`, and finalizes the intent to
   `live` with `generation + 1`.
5. Any failure rolls back: the intent is removed **first**, then the bridge binding, because a
   half-armed bridge with a leftover intent is the one shape that could later claim a station the
   user never successfully attached. The removal is conditional on the generation this invocation
   wrote *and* on the record still being an unarmed `pending` one, so it can never destroy a record
   the daemon's stamp, a concurrent attach, or a turn-boundary finalize has moved on from.

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
| A successful push registration is always durably recoverable | For a binding with a station-intent record, the armed proof is committed inside `register_member` before the member is installed, and a proof that cannot be persisted refuses the whole registration; `write_pending`, the attach rollback's delete, and the stamp all take the same per-intent write lock, so no interleaving leaves an armed member with nothing on disk. A binding with *no* record owes no proof: the stamp opens the scope through the non-creating path, so a scope that does not exist is "nothing to prove" rather than a failure, and a client that writes no intent is never refused for a scope it never used |
| A rollback removes only what its own invocation created | Every delete is conditional on the generation the caller wrote *and* on the caller's predicate re-evaluated under the lock; the generation advances monotonically even across a lifecycle transition, so a re-attach over a tombstone can still clean up after itself; on the daemon side a refused proof releases only the lease that call claimed, and never touches a pre-existing adopted member |
| A GC TTL cannot be extended by retrying | Every clock is anchored to the event it is about — the orphan clocks read *proof*, the unarmed pending clock reads `created_at_ms`, the armed pending clock reads the idempotent proof's own timestamp — never `evidence.last_attempt_ms` or `updated_at_ms`, both of which a failing retry rewrites. `write_pending` inherits those two clocks only from a record that is *itself* `pending`, so a retry of one attach cannot reset them and a genuinely new attach over a tombstone is not born expired |
| Arming authority never outlives the lifecycle that earned it | The armed proof is inherited only across a retry of the same pending attach; a write over a revoked or otherwise inert record starts a new lifecycle with no proof, so `finalize_admission` cannot promote a new attach on a previous daemon's arming |
| A stale projection cannot outlive the record it described | The pre-drain report compares the cached entry's generation against the durable manifest's, and prefers the newer manifest; every durable transition moves the generation while evidence-only rewrites do not |
| "There is no record" is always a proven answer, never a failed look | Every authority-bearing existence check goes through `platform_fs::path_present`, whose `Ok(false)` is only a positive `NotFound`; an undecidable answer is an `Err` each caller classifies (`RecordUnusable` for the arming stamp and the obligation observation, `Unavailable` for the anti-downgrade guard, "not provably gone" for the GC credential rule, a failed CAS rather than a create). `Path::exists()` collapsed all of those onto the permissive branch |
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

## Corrections made during the independent re-review

The final-review remediation was applied by the pass that found it, so an independent re-review
followed, focused on the GC/`Pending` lifecycle, the retry split, the guard precedence order, and
the new successor path. It found eight further defects — one blocking, two high, five medium. All
are fixed. Each changes a documented property, so each is recorded here.

### 1. BLOCKING — the reload-plus-replacement deadlock, and the crash between `Register` and finalize

Two routine events combined into a binding that could never recover, and the fix for the first half
had a circular dependency that left the second half open.

**The deadlock.** A bridge reload (`extensions_reload`, `/clear`, an extension-host restart) gives
the producer a new `(pid, start_time)` while the `live` intent still names the old pair. The daemon
proves that pair before it sends a byte, so every pass fails `producer_identity_mismatch`. The
turn-boundary hook exists to re-record the identity — but it only acted on bindings for which the
*currently connected daemon* reported `push_registered`. Trace it with a daemon replacement in the
middle: the successor has no member, and it cannot create one, because creating one is what the
stale identity prevents. The repair was gated on the thing the repair was supposed to restore, and
the binding stayed unrecoverable for as long as the record survived.

**The crash window.** Between `Register` returning (push armed) and the producer-side finalize
(record promoted to `live`), the durable record is a bare `pending` one. A crash there — of the CLI,
the agent, or the machine — left push delivering with a record the five-minute pending TTL then
deleted. Recovery was silently disarmed for a station that worked, and the user found out only after
the next daemon replacement.

**Fix — an explicit durable armed proof, and an asymmetric transition table.**

`StationIntentV1` gains `armed: Option<ArmedProofV1> { armed_at_ms, daemon_instance_id }`, written by
the **daemon** inside `register_member` at the moment it commits an armed push member — never by the
producer side, never inferred from a credential file existing. Additive and optional, so records
written before it read back as "never armed", and an older build round-trips it through the
unknown-field passthrough.

`station_intent::finalize_admission(state, armed_durably, armed_now)` is now the single place the
producer-side transition rules live, and it is deliberately asymmetric:

- `live` → **refresh**, with no daemon involvement at all. Being `live` *is* durable proof the
  binding was armed, so a bridge that proves it is alive right now may re-record its own identity.
  This is what breaks the deadlock.
- `pending` → **promote**, but only on one of two authorities: a daemon reporting `push_registered`
  for the binding right now, or the record's own armed proof. A merely-existing bridge still cannot
  arm an attach that was never registered — the security property is unchanged, and is now stated as
  a table rather than implied by a call-site filter.
- `revoked`/`tombstoned` → **refused**, always. A detach, session end, or operator reset that lands
  mid-finalize wins.

The decision is re-made *inside* the per-intent write lock against the record as it actually is, so
the check and the write are one critical section. Restoration is untouched: it still requires the
credential rules, `verify_server_peer`, the probe, and the daemon epoch fence. GC governs an armed
`pending` record by `STATION_INTENT_ARMED_PENDING_TTL` (24 h) instead of the five-minute rule,
because it describes delivery that really was armed.

One further defect fell out of the trace: after the identity was repaired, the binding was still
parked on the ladder its *stale* descriptor had earned — up to the quarantine hour. The ladder
belongs to a descriptor, not a binding, so a durable state transition (any generation move) now
clears `consecutive_failures` and `next_attempt_ms`. Evidence writes deliberately do not move the
generation and therefore forgive nothing.

Tests: `station_intent::tests::finalize_admission_is_the_whole_producer_side_transition_table`
(the full table, including every refusal),
`station_intent::tests::the_armed_proof_is_durable_daemon_evidence_and_live_implies_it`,
`station_intent_a_reloaded_producer_recovers_with_no_member_to_start_from` (the trace end to end,
asserting the memberless refresh is admitted *and* that an unarmed `pending` record in the same
position is refused), `station_intent_the_daemon_stamps_the_armed_proof_when_it_arms_push` (a pull
register earns none; an armed one does; an armed `pending` record is still not claimable),
`station_intent::tests::gc_governs_an_armed_pending_record_by_its_own_longer_ttl`,
`station_intent_a_durable_transition_clears_a_ladder_the_old_descriptor_earned`.

### 2. HIGH — destructive deletion races

`IntentStore::remove` was an unconditional unlink taking no lock. Every caller decides from a
snapshot, so between the decision and the unlink the record can change: GC classified a record it
loaded earlier in the pass, and the attach rollback removed "the record this invocation created"
without checking that it still was. A failing older attach could therefore delete a `live` record a
concurrent finalize had just promoted, and a GC pass could delete a newer generation than the one it
judged.

**Fix.** `remove` is gone. Deletion goes through `remove_if_unchanged(id, expected_generation,
predicate)` — which re-acquires the per-intent write lock, reloads, and requires both the generation
and the caller's own condition to still hold — or `remove_unreadable_if_unchanged`, which re-confirms
under the lock that the manifest is still unreadable and still past its TTL (so a manifest that was
merely mid-rewrite is not deleted because one unlucky pass could not parse it). `gc_reason` was
extracted as a pure function so the same decision can be applied twice: once to classify, once under
the lock. A lock the pass could not take is a keep, never a delete. The attach rollback passes the
generation `write_pending` returned and the predicate "still an unarmed `pending` record".

`write_pending_intent`'s check-then-write moved into `IntentStore::write_pending`, under the same
per-intent lock: unserialized, two concurrent attaches both read "no live record", both compute
`existing.generation + 1`, and the second clobbers the first at the *same* generation — which
defeats every downstream generation CAS, since a CAS holder cannot tell the record under it was
replaced. It also carries an existing armed proof forward, so a re-attach that then crashes does not
re-open the window above.

Tests: `station_intent::tests::deletion_is_refused_when_the_record_moved_under_the_decision`,
`station_intent::tests::write_pending_is_generation_safe_and_never_demotes_a_live_record`,
`commands::copilot::tests::attach_rollback_only_deletes_the_record_this_attach_left_behind`.

### 3. HIGH — an inert record could shadow a live one for up to a TTL

`reconcile_once` claimed the per-`(store_key, address)` winner slot **before** the `is_reconcilable`
check. Scan order is generation-descending, so a `revoked` generation 3 for one session consumed the
address and the `live` generation 2 for another lost. That is not a scheduling delay: the shadowed
record was `continue`d before it was indexed, so for as long as the tombstone survived (up to its
seven-day TTL) the live binding was invisible to `telex status`, absent from the pre-drain report,
unseen by the turn guard, and attempted by no pass at all.

**Fix.** Every record the pass loads is indexed first; only *reconcilable* records then compete for
the address slot. Test:
`station_intent_a_revoked_record_never_shadows_a_live_one_for_the_same_address` — a revoked
generation 3 against a live generation 2, asserting exactly one armed member, that it is the live
intent's session, that **both** rows reach the index, and that the drain report sees the binding.

### 4. MEDIUM — the orphan TTL clocks were unreachable

`gc` measured "how long unproven" from `last_success_ms.or(last_attempt_ms)`. The reconciler persists
scheduling state on every genuine failure, so for a record that had never once succeeded the attempt
clock was refreshed every few seconds forever — and neither the dead-producer orphan rule nor the
credential-missing rule could ever fire for exactly the abandoned records they exist to collect.

**Fix.** `StationIntentV1::last_proven_ms()` reads *proof* only: `last_success_ms`,
`producer_verified_ms`, and the durable state transition a finalize performs (which is itself gated
on a live probe). An attempt is not proof and no longer appears in the clock at all. Test:
`station_intent::tests::gc_orphan_clocks_are_never_refreshed_by_a_retry_attempt` drives the failing
case directly — a dead producer past the TTL with `last_attempt_ms` pinned to *now* and 4,000
consecutive failures — for both rules, plus the negative control that a producer proven a second ago
is not an orphan.

### 5. MEDIUM — a finalize was invisible to an immediately following drain

The pre-drain report was projected from the daemon's cached index, which only a reconcile pass
refreshes — while the record is written by a producer-side finalize in a *different process*. So
`copilot attach` immediately followed by `telex upgrade` drained with `recoverable: 0` for a binding
that had just been fully armed and finalized, and the successor-verification step skipped itself on
"no recoverable station intents". The one path that exists to carry push delivery across a daemon
replacement quietly became a no-op.

**Fix, on both sides.** `drain_intent_report` gained a bounded durable backfill (`list_ids` + `load`,
capped at the per-scope cap; still no probe, no connection, no backend or network I/O), composed so
neither source masks the other: a cached projection naming a *problem* wins, otherwise the manifest
the successor will actually read wins, and a cached projection is never retracted by the durable
read. And the attach path now asks the daemon to reconcile immediately after a successful finalize,
so the index is warm rather than merely eventually correct. Test:
`station_intent_a_finalize_is_visible_to_the_very_next_drain_decision` performs the durable
transition with no pass at all and asserts the next drain decision counts it.

### 6. MEDIUM — the Windows boot identity degraded to a per-process value

If the per-boot record could not be persisted to `HKCU\Software\telex`, the resolver returned the
value it had just minted "for this process". That is not a degradation. The identity is compared for
**exact equality** across processes, so a per-process value makes every intent
`foreign_host_or_boot` the instant anyone else reads it — terminal, after which GC removes the record
as a foreign identity with a dead producer and the anti-downgrade guard refuses unrelated attaches,
with nothing anywhere naming the cause.

**Fix.** A failed persist, or a persist that cannot be read back, is now an explicit error;
`boot_id()` memoizes it, so the answer is at least consistent for the process's life. The decision is
factored into `resolve_minted_boot_id` so the denied-persistence branch is reachable from a test
without mutating the machine the suite runs on.

The old stability test called the *memoized* `boot_id()` two hundred times, so it compared a cached
`String` to itself and would have passed unchanged against the jittering implementation it existed to
rule out. `platform_fs::boot_id_uncached()` (a `#[doc(hidden)]` test seam) re-enters the resolver
every call, and `tests/boot_identity.rs` spawns a genuinely **independent process** — the test binary
re-invoked on an emitter test — and requires exact agreement, including for a third process started
later. Tests: `platform_fs::imp::boot_id_tests::a_boot_id_that_cannot_be_persisted_fails_explicitly`,
`platform_fs::tests::boot_identity_is_stable_across_repeated_independent_resolutions`,
`boot_identity_agrees_across_an_independent_process`,
`boot_identity_survives_repeated_independent_mint_attempts`.

### 7. MEDIUM — the first backoff rung was twice what the docs promised

`backoff_delay` used `consecutive_failures` as the exponent, but that counter includes the failure
being scheduled, so the first transient failure waited 10 s against a documented
`RECONCILE_BACKOFF_INITIAL` of 5 s. Wrong at exactly the rung that matters most — a bridge
mid-reload, which is over within a tick or two. **Fix:** the exponent is `failures - 1`, giving the
documented `5 s → 10 s → 20 s → … → 5 min`. The test now pins four rungs and the zero-count case
(the `DeferredPullWaiter` cadence, which never advances the counter) rather than one band.

### 8. MEDIUM — the two-session exclusivity test asserted almost nothing

`armed.len() <= 1` is satisfied by the two failures that matter most: zero armed members (nothing
recovered at all) and the *wrong* session winning. Both rivals also shared one producer, so the
loser failed on `probe_session_mismatch` and the dedupe under test never had to work. **Fix:** each
rival now has a healthy producer and its own credential, generations make the expected winner
deterministic, and the assertion is `armed == ["sess-a"]` — exactly one, and the expected one — plus
a check that the loser is still indexed rather than silently dropped.

## Corrections made during the final gate

A final gate pass over the re-review result found three further defects — 1 high, 2 medium — two of
which land on residual risks the re-review had recorded rather than closed.

### 1. HIGH — a successful push registration could lose its only durable proof

The armed stamp was applied at the outer `register_member` seam, *after* `register_member_inner` had
returned, and its result was discarded: `mark_armed` returning `Ok(false)` (no manifest for the
binding) or an outright error still produced `Registered`. That made the proof a diagnostic rather
than part of the registration, and two orderings exploited it.

The race, exactly: attach A writes its `pending` record and registers. Concurrent attach B, for the
same binding, replaces the record at a new generation via `write_pending`, then fails downstream and
rolls its own write back — deleting the file. If that delete lands after A committed its member and
before A stamped, A's stamp finds nothing, the miss is swallowed as "the ordinary pull-attach case",
and A returns a successful push registration whose only durable trace has been destroyed. The
station delivers until the next daemon replacement and then silently stops. A crash of the daemon in
the same window does the same thing, and an unwritable scope did it without any race at all.

**Fix — make the proof part of the registration transaction.**

- **Observe the obligation up front.** An arming register (`on_deliver.is_some()`) asks
  `DaemonState::durable_intent_present` — under the per-station admission guard, before any work —
  whether the binding has a durable record. If it does, this register *owes* a proof. That question
  cannot be answered at stamp time: "no record" there is both "a pull or plain `--on-deliver` attach
  that never wrote an intent" (nothing owed) and "the record was deleted under me" (everything
  owed). An unreadable scope fails closed with the same typed `Incompatible` /
  `PushIntentUnrecoverable` shape the anti-downgrade guard in the same function already uses.
- **Make the stamp's outcome legible.** `IntentStore::mark_armed` became `stamp_armed_proof`,
  returning `ArmedProofStamp::{NoRecord, Stamped{generation}, AlreadyArmed{generation}}` instead of a
  bool that collapsed "nothing to prove" into "already proven".
- **Commit the proof before the member.** The stamp moved *inside* `register_member`, immediately
  before `state.insert_member(...)` on both the new-member and the refresh branch — after every
  fallible step, so a successful stamp is followed only by an infallible in-memory commit. There is
  no longer any state in which a member is armed and nothing durable says so.
- **Refuse rather than report a success it cannot back.** `commit_armed_proof` returns the typed
  refusal when a proof is owed and the record is gone, corrupt, or unwritable.
- **Roll back exactly what this call created.** The new-member branch releases only the epoch lease
  it just claimed and installs no member. The refresh branch touches nothing at all, so a
  pre-existing (possibly adopted) member and its lease survive a failed write — tearing down a
  working station because a diagnostic write failed would be strictly worse than the failure.
  `clear_definite_session_end` moved after the commit so a refused register has no side effect.
- **Idempotency preserved.** `AlreadyArmed` is a success and does not move the generation, so a
  re-attach neither churns concurrent CAS holders nor moves the clock the armed pending TTL reads.

The ordering is what closes the race rather than merely narrowing it: `write_pending`, the
conditional rollback delete, and the stamp all take the same per-intent write lock, so a concurrent
rollback either lands *before* the stamp (the register is refused and no armed member exists) or
*after* it (the record carries the proof, so both of the rollback's gates refuse). No interleaving
yields "member armed, nothing durable".

**Tests.** `a_concurrent_pending_rollback_before_the_proof_refuses_the_register` schedules the exact
B-replaces-then-rolls-back sequence through the admission commit gate and asserts a typed refusal,
no member, no resurrected record, and a released lease.
`a_register_whose_manifest_is_missing_at_the_proof_is_refused` covers the plain missing-manifest
shape; `a_register_whose_proof_cannot_be_written_is_refused_and_leaves_no_member` covers a corrupt
manifest; `a_failed_proof_on_a_refresh_leaves_the_pre_existing_member_untouched` pins the adopted
member, its lease epoch, and its owner instance id across a refused refresh;
`a_concurrent_rollback_cannot_delete_the_record_of_a_committed_register` pins the post-commit
invariant and idempotency; `a_push_register_for_a_binding_with_no_intent_record_still_succeeds`
guards the non-bridge push clients against the new refusal; and
`an_unreadable_record_refuses_the_arming_stamp_rather_than_reporting_success` pins the store-level
contract.

### 2. MEDIUM — a stale cached failure outlived the generation it was recorded against

Residual risk #4 of the re-review, realized. The pre-drain durable backfill kept a cached *problem*
projection whenever one existed, because neither the cached entry nor the durable read carried a
generation to compare — so the rule "a cached failure is evidence the successor will hit the same
wall" was applied to a record the successor would never read.

The sequence is the routine one this whole recovery path exists for: a bridge reloads, the pass
fails `producer_identity_mismatch` against the stale `(pid, start_time)` and caches `unverifiable`
for generation N, and the turn-boundary hook then re-records the live identity at generation N+1.
Nothing refreshes the cached projection in between — only a reconcile pass does, and `upgrade`
drains before the next tick. So a binding that had *just been repaired* drained as `degraded`, and
successor verification skipped the hand-off it exists to perform.

**Fix.** `durable_intent_states` now returns `(state, generation)`, and `drain_intent_report` keeps a
cached problem only while `cached_generation >= durable_generation`; otherwise the newer manifest
wins. Generation is the right discriminator because every durable transition (a finalize, an identity
refresh, an arming stamp, a re-attach) moves it, while the reconciler's evidence-only rewrites
deliberately do not. Rejected manifests are unaffected: they cannot be loaded, so they never reach
`durable_intent_states` and their cached verdict is untouched.

**Test.** `station_intent_a_cached_failure_never_outlives_the_generation_it_was_recorded_against`
asserts `degraded == 1` while the cached verdict still describes generation 2, performs the identity
repair with no pass in between, asserts the cached projection is *still* at generation 2 (so the
test is about the composition rule, not about a refresh), then asserts `recoverable == 1` and
`degraded == 0`.

### 3. MEDIUM — a failing re-attach could push the pending TTL out forever

`write_pending` refreshes `updated_at_ms`, and the `Pending` GC arms aged from that field. A
re-attach *is* a `write_pending`, so a producer whose finalize kept failing — a bridge stuck
mid-reload, a probe that never answers — extended its own leftover's expiry every few seconds. GC
could never collect the exact class of record it exists for, and the scope grew a permanent resident
per wedged binding.

**Fix.** `StationIntentV1::pending_clock_ms()` is an explicit lifecycle clock, anchored to the event
each TTL is actually about, and neither event can be replayed by retrying:

- An **unarmed** `pending` record ages from `created_at_ms`. `write_pending` already carries that
  field forward from the record it replaces, so a re-attach cannot reset it.
- An **armed** one ages from `armed.armed_at_ms`, floored at `created_at_ms` so a clock-skewed or
  hand-edited proof cannot age a record from before it existed. The stamp is idempotent, so a
  re-register cannot move it either.

`updated_at_ms` remains an honest last-write field; it is simply no longer any pending TTL's clock.
Deliberate state transitions still get the clock they are supposed to have: arming moves the record
onto the 24 h armed clock, and a revocation ages from the revocation.

**Tests.** `a_repeated_pending_write_cannot_push_the_pending_ttl_out_forever` runs ten re-attaches
spanning more than twice the TTL, asserts the last-write clock really was refreshed past it, and
asserts the record is still collected — then walks both legitimate transitions (arming earns the
longer clock; a revocation is visible for its own TTL and then bounded).
`gc_governs_an_armed_pending_record_by_its_own_longer_ttl` was tightened so `updated_at_ms` is set
*later* than both clocks, meaning a rule that still read it would keep both records and fail.

### Test discrimination

Each fix was mutation-checked rather than assumed: the guard was temporarily reverted to the pre-fix
behavior and the suite re-run. `commit_armed_proof` forced to `Ok(())` fails the four G1 register
tests with `expected a typed refusal, got Registered { .. }`; neutralizing the generation comparison
fails the G2 test on `recoverable` `0 != 1`; reverting `pending_clock_ms` to `updated_at_ms` fails
the G3 clock assertion.

## Corrections made during the final approval gate

A gate over the final-gate result found two further defects — 1 high, 1 medium. Both are cases where
a fix from the previous pass was right about the *mechanism* and too broad about the *scope* it
applied to: one inherited a lifecycle's clock and proof across a boundary where the lifecycle had
ended, and the other treated every failure to reach the intent scope as a durability failure even
for registrations with no durability to lose.

### 1. HIGH — a re-attach after a teardown was born already expired

`write_pending` carried `created_at_ms` and the armed proof forward from *whatever* record it
replaced. That is exactly right for a retry of an attach that has not finalized — it is what makes
the pending TTL unreachable by retrying (finding G3 above) — and exactly wrong for a genuinely new
attach.

A `revoked` tombstone is kept for the seven-day terminal TTL, so it is still on disk for a week after
a `copilot detach`, a fallback downgrade, a `station reset`, or a session end. Re-attaching that
binding wrote a new `pending` record that inherited the *original* attach's creation time, so its
pending clock was already days past the five-minute TTL. The next GC pass deleted it — before
`extensions_reload` had loaded the bridge and before the turn-boundary finalize could promote it. The
attach reported success, its record vanished seconds later, and it did so again on every retry for a
week. The armed variant was worse in a second way: the tombstone's `armed` proof came along too, so
`finalize_admission` would have promoted the new attach on the strength of a *previous* daemon's
arming (`armed_durably`), which is precisely the "a merely-existing bridge arms an attach that was
never registered" hole the admission rules exist to close. It also made the attach unable to roll
itself back, since `rollback_removable` refuses an armed record — a failing attach could not delete
the record it had just written.

**Fix — the two fields belong to a lifecycle, not to a file.** `write_pending` now distinguishes the
two things that reach it:

- The record it replaces is itself `Pending`: this write is another attempt at the *same*
  unfinalized attach. It inherits that lifecycle's `created_at_ms` and its armed proof, so no
  amount of retrying buys more life than the one attach earned, and a crash between `Register` and
  the finalize still has the proof it needs to be repaired. The durable proof also *wins* over
  anything the caller supplied, because only `stamp_armed_proof` may mint one.
- The record it replaces is `Revoked` or otherwise inert: the lifecycle is over, and this is a new
  attach that happens to reuse the binding. It keeps its own `created_at_ms` — a full pending TTL to
  reach its finalize — and carries **no** proof. It earns the longer armed clock the only way a
  lifecycle can: a new daemon stamps it.

The generation is the one field still inherited-and-advanced across the transition: it is a per-file
compare-and-set token, not a lifecycle property, so the attach rollback's generation gate and every
concurrent CAS holder are unaffected.

**Tests.** `a_new_attach_over_a_finished_lifecycle_starts_its_own_pending_clock` ages a revoked
record six days, re-attaches, and asserts the record survives a GC pass just inside its *new* TTL and
is collected just past it — so the fresh clock is a full TTL and not an exemption.
`a_new_pending_lifecycle_never_inherits_the_previous_ones_armed_proof` does the same over a
tombstone that *was* armed, asserting the new record is unarmed, that
`finalize_admission` refuses to promote it with no live member, that it is governed by the unarmed
TTL, and then that a new daemon stamp gives it a proof naming the new instance and moves it onto the
24 h clock. `a_fresh_pending_lifecycle_earns_one_clock_and_no_retry_can_earn_another` is the
anti-regression guard in the other direction: ten retries spanning more than twice the TTL after the
transition, asserting the clock was set once and the record is still collected.
`attach_rollback_only_deletes_the_record_this_attach_left_behind` gains a fourth case for the
re-attach-after-teardown shape, asserting the new record is unarmed and that the attach can remove
exactly what it wrote.

### 2. MEDIUM — an ordinary push register was refused for a scope it never used

`commit_armed_proof` refused the registration on *any* error from the stamp, including when
`owes_proof` was `false`. And `stamp_intent_armed` opened the intent scope through the **creating**
path, so on a host where the scope could not be made — a read-only or full run directory, or debris
where the `intents` directory belongs — every arming register failed with `Incompatible` /
`PushIntentUnrecoverable`, including for the clients that write no intent at all. Those clients have
no durable state to protect, so the refusal protected nothing and denied a registration that would
have worked. It also created a scope as a side effect of a registration with nothing to put in it —
the same "a read path creates what it documents as not creating" smell the re-review closed
elsewhere.

**Fix — make the rule a table, and read the scope the way the observation does.**

- `station_intent::armed_proof_admission(stamp, owes_proof)` is the decision, decidable without a
  daemon or a filesystem fault, alongside `finalize_admission`. A stamped or already-armed record
  commits. A missing record, or a scope that could not be opened, refuses **only** a register that
  owes a proof. A record that is present and unreadable refuses **either way**.
- `stamp_intent_armed` returns a classified `ArmedProofRefusal` (`ScopeUnavailable` /
  `RecordUnusable`) instead of a bare string, so the decision is made from a closed set rather than
  by matching on a message.
- The stamp now opens the scope through `intent_store_readonly`, the same non-creating open
  `durable_intent_present` uses. A scope with no directory holds no records, so that case is
  `ArmedProofStamp::NoRecord` — provably nothing to prove — and it is then gated by `owes_proof`
  exactly as a missing manifest is.

Fail-closed is unchanged everywhere it was load-bearing: a register that observed a durable record
and then could not stamp it is still refused, and so is a register that finds a record present and
unreadable even when the up-front observation saw nothing — which is the concurrent-attach window,
where a record can appear between the observation and the stamp.

**Tests.** `armed_proof_admission_is_the_whole_daemon_side_proof_table` asserts all five outcomes
against both values of the obligation.
`a_push_register_owing_no_proof_survives_a_scope_that_cannot_be_created` drives a real register
against a run directory with a file where the `intents` directory belongs, asserting `Registered`,
an armed member, and that no scope was created.
`the_proof_commit_gate_refuses_an_unowed_register_only_for_a_broken_record` drives the gate directly
across all three shapes: no record (unowed commits, owed refuses), a healthy record (stamped even
when unowed — the benign half of the observation race), and a corrupt record (refused in both
directions).

### Test discrimination for this gate

Both fixes were mutation-checked in **both** directions, since each is a scoping rule and the failure
modes are symmetric:

- `write_pending` reverted to inheriting unconditionally: the three new lifecycle tests and the
  rollback test fail (`a new lifecycle is not a retry of the one it replaced` — `left: 1000`,
  `right: 518701000`).
- `write_pending` mutated to never inherit: `a_repeated_pending_write_cannot_push_the_pending_ttl_out_forever`,
  `write_pending_is_generation_safe_and_never_demotes_a_live_record`, and
  `a_fresh_pending_lifecycle_earns_one_clock_and_no_retry_can_earn_another` fail — so the fix cannot
  be "corrected" into re-opening G3.
- `armed_proof_admission` reverted to refusing every failure (with the creating open restored):
  the table test fails on `Err(ScopeUnavailable)` and the register test fails on its precondition.
- `armed_proof_admission` mutated to gate *every* failure on the obligation: the table test fails on
  `Err(RecordUnusable)` and the gate test fails on `a broken record refuses even an unowed register`.

## Corrections made during the existence-probe gate

A gate pass over the admission and lifecycle rules the previous two gates installed asked one
question of each: *how is "there is no record here" actually decided?* The answer everywhere was
`Path::exists()`, which is the one std API that cannot express "I could not tell" — it maps every
metadata failure onto `false`. In each of these rules `false` is the **permissive** branch, so the
whole set had a shared fail-open: a durable record that exists but cannot be stat'd reads as a
binding that never existed.

One high finding, plus the audit of every sibling check it implicated. All fixed in one pass,
because they share a cause and a fix.

### 1. HIGH — an inaccessible station-intent record read as "no record", and an ordinary admission committed

`DaemonState::durable_intent_present` and `IntentStore::stamp_armed_proof` both decided existence
with `Path::exists()`. For a record that is on disk but whose metadata the platform refuses to hand
over — a denied ACL, an untraversable parent directory, a volume that went away, an antivirus lock —
both answered "not there":

1. The up-front observation set `owes_armed_proof = false`, putting the register in the **permissive
   column** of the admission table.
2. The stamp returned `ArmedProofStamp::NoRecord` for the same reason.
3. `armed_proof_admission(Ok(NoRecord), false)` is `Commit`.

So the register committed an armed push member over a durable record it had never read, never
stamped, and never even confirmed was there — including over a `live` record mid-reconcile. The
`RecordUnusable` row, which exists precisely to refuse this, was unreachable through the stat path:
it could only be reached by a record that stat'd *successfully* and then failed to load. The
response said `Registered`; the durable state said something else; nothing reconciled the two.

The same collapse also made `stamp_armed_proof`'s deliberate "the record vanished under me" remap
unsound in the other direction: it converted a genuine `Io` failure into `NoRecord` whenever the
re-check could not decide either.

**Fix — one probe, and absence has to be proven.**

- `platform_fs::path_present(path) -> Result<bool>` is the single existence primitive. `Ok(false)`
  is only ever a positive `NotFound` from the platform; every other outcome is an `Err` the caller
  must classify. It lives in `platform_fs` because that module's stated rule 1 is "anything that
  cannot be positively verified is an error, never a silent 'assume fine'", and `exists()` was the
  one hole in it.
- `durable_intent_present` returns `Err` for an undecidable record, and the register fails closed
  with the same typed `PushIntentUnrecoverable` refusal it already used for an unreadable scope.
- `stamp_armed_proof` returns `Err` rather than `NoRecord`, which `stamp_intent_armed` classifies
  `RecordUnusable` — the row that refuses in **both** columns of the table, which is what makes the
  fix hold for the `owes_proof == false` case the finding is about.
- The vanished-record remap now requires `Ok(false)` from the re-check. An undecidable re-check
  leaves the original `Io` failure standing.

### 2. HIGH — the same collapse in the anti-downgrade guard and the scope root

The audit the finding asked for turned up two more instances on the same admission path, both
reachable by the same condition:

- `IntentStore::open_existing` decided "this host never attached" with `root.exists()`. An intent
  scope full of records whose root could not be stat'd therefore reported `Ok(None)` — an empty
  scope — to *every* read path at once: `durable_intent_present`, the arming stamp, the
  anti-downgrade guard, and the drain report.
- `DaemonState::lookup_live_intent` decided `LiveIntentLookup::Absent` with `path_for(&id).exists()`.
  `Absent` is the answer that lets a pull-only registration proceed over a push binding, and the
  guard's whole reason for re-reading the manifest is the daemon-replacement window in which the
  cached index is empty — so a record it could not stat produced exactly the silent downgrade the
  guard exists to prevent. The three-way `Unavailable` arm was already there and already refused;
  it was simply unreachable through the stat path.

**Fix.** Both now use `path_present`. An undecidable root is an `Err` from `open_existing` (which
`intent_store_readonly` already documents callers must fail closed on), and an undecidable record is
`LiveIntentLookup::Unavailable`, not `Absent`.

### 3. MEDIUM — GC could delete a record because it could not look at the credential

`gc_reason`'s credential rule read `!intent.producer.credential.path.exists()`. The credential is
the bridge registry, which lives in a directory telex deliberately *shares* with an external
producer, so a permissions change, an antivirus lock, or a mount that hiccupped is a routine
metadata failure there — and each of them read as "the credential file is gone". Past the
15-minute TTL, GC then deleted the durable record of a binding that may have been delivering the
whole time. Deletion is the one GC action recovery cannot undo.

**Fix.** `credential_provably_absent` fires only on `Ok(false)`. A credential telex could not look
at keeps the record; the rule still fires normally for a credential that is provably gone.

### 4. MEDIUM — three more lifecycle checks with the same shape

- `IntentStore::revoke` returned `Ok(false)` — "there was nothing here to revoke", which every
  caller treats as success — for a record it could not stat. The daemon's session-end and detach
  paths would then consider a live intent retired while the record on disk still said the station
  was armed. Now an `Err`.
- `IntentStore::write_cas_locked` treats "no record" plus `expected_generation == 0` as *create*, so
  an undecidable record turned a lost compare-and-set into an unconditional overwrite of a record
  the caller had never read. Now an `Err`. (`write_atomic`'s cap check is the same probe and was
  fixed with it; the mutation check below reverts both, because either one alone still refuses.)
- `backend_open_existing_only` classified a SQLite store file it could not stat as `store_missing`,
  which is **terminal** — the intent parks on the hour-long quarantine cadence on the reasoning that
  a store which does not exist will not start existing. A locked or briefly unreadable store file is
  the opposite kind of condition, so it now returns `store_unreadable` and takes the ordinary retry
  ladder.

### 5. LOW — the drain hook could disable itself

`copilot drain`'s fast path skipped the daemon round-trip entirely when the bridge registry did not
`exists()`. A registry telex could not stat therefore produced `no_bridge` on *every turn stop* for
as long as the condition lasted — the drain hook silently opting out for exactly the sessions it
serves. `no_bridge_fast_path` now takes the fast path only on a proven absence; an undecidable
answer costs one daemon round-trip.

Two checks were audited and deliberately left best-effort, with the reasoning recorded at the call
site: `read_cursor` (the scan cursor is a scheduling hint whose read path already defaults on
failure) and `ensure_owner_private_dir` / `ensure_owner_private_producer_root` (which create and
then validate, so an undecidable path fails at the create or the shape check rather than silently
passing).

### The test seam, and why it is one

The behavior under test is "what does telex do when the filesystem answers with an error", not "how
does this platform produce that error". Every real way to produce one is platform-specific and flaky
in CI: `chmod 000` on a parent is a no-op under a root test runner and has no Windows equivalent, a
Windows deny-ACE has no Unix equivalent, and the paths each platform rejects outright differ. So the
error is injected at the single function that asks — `platform_fs::stat_faults::Unstatable`, a
`#[cfg(test)]` RAII guard keyed by exact path, following the same pattern as the daemon's existing
`delivery_admission_control` seam. It carries an optional "answer the first N probes truthfully"
count, which is what makes the *re-check* in `stamp_armed_proof` pinnable separately from the entry
check.

**Tests** (11 new):

| Test | Pins |
|---|---|
| `path_present_reports_absence_only_when_it_can_prove_it` | the probe's contract, and that the fault is scoped |
| `an_unstatable_record_refuses_an_arming_register_that_owed_no_proof` | **the finding**, end to end: nothing exists at the observation (`owes == false`), the record appears and becomes unstatable at the commit gate, the register is refused, no member, no false proof — and the control on the same state, with the fault gone, registers and stamps |
| `an_unstatable_record_makes_an_arming_register_fail_closed_up_front` | the observation itself: `durable_intent_present` errors, and the register commits nothing |
| `an_unstatable_scope_root_is_not_an_empty_scope` (daemon) and `an_unstatable_scope_root_is_an_error_not_an_absent_scope` (store) | the scope-root collapse, plus that a provably absent scope is still `None` |
| `an_unstatable_record_refuses_a_pull_only_downgrade_rather_than_allowing_it` | the anti-downgrade guard: `Unavailable` not `Absent`, the pull register refused — and that a readable non-live record still admits it |
| `an_unstatable_record_is_never_reported_as_no_record` | the stamp's entry check, with a genuinely absent binding as the control |
| `the_vanished_record_remap_requires_a_proven_absence` | both halves of the remap, driven through the real race (the per-intent lock is held, so the locked load fails with I/O after the entry check passed): a record deleted mid-stamp is `NoRecord`, an undecidable re-check is not |
| `gc_keeps_a_record_whose_credential_could_not_be_stat_ed` | the GC rule in both directions |
| `an_unstatable_record_cannot_be_reported_as_nothing_to_revoke` | revoke, with both a readable record and a provably absent one as controls |
| `a_cas_against_an_unstatable_record_fails_rather_than_creating_one` | the CAS-becomes-create path |
| `an_unreadable_sqlite_store_file_is_transient_not_terminal` | the missing-versus-unreadable store classification, with the terminal verdict for a provably absent store as the control |
| `an_unstatable_bridge_registry_does_not_report_no_bridge` | the drain fast path, with the pull-only no-op it exists for as the control |

### Test discrimination for this gate

Every guard was reverted to the defect — the probe's error collapsed back into `false` at that call
site, which is exactly what `Path::exists()` did — and the suite re-run. All ten mutants are caught:

| Mutation | Result |
|---|---|
| `stamp_armed_proof` entry probe | 2 tests fail |
| `durable_intent_present` | 1 fails |
| `open_existing` root probe | 2 fail |
| `lookup_live_intent` | 1 fails |
| GC credential rule | 1 fails |
| `revoke` | 1 fails |
| `write_cas_locked` + `write_atomic` (its second guard; either alone still refuses) | 1 fails |
| vanished-record remap re-check | 1 fails |
| `no_bridge_fast_path` | 1 fails |
| `backend_open_existing_only` | 1 fails |

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
  built from the cached index plus a bounded durable backfill for bindings the index has no fresh
  answer for — no probe, no connection, no backend or network I/O, so it still cannot slow a drain.
  The backfill is what makes a station attached seconds before the drain visible: the record is
  written by a producer-side finalize in another process, and only a reconcile pass refreshes the
  index. The two sources are composed **by generation**: a cached problem projection wins only while
  it still describes the record on disk, so a binding whose producer identity was just repaired is
  reported as recoverable rather than carrying forward the failure the repair fixed.
- A push registration for a station that has a station-intent record only succeeds once the daemon
  has durably recorded that it armed delivery. The proof is written before the member is installed,
  and a register that cannot write it is refused with `Incompatible` / `PushIntentUnrecoverable` —
  no member created, any epoch lease it claimed released, and a station that was already attended
  left exactly as it was. A binding with no intent record (a pull attach, or a plain
  `telex attach --on-deliver`) owes no proof and is unaffected, including on a host where the intent
  scope cannot be created at all — the proof commit reads the scope without creating it, so "no
  scope" is "nothing to prove" rather than a failure.
- Re-attaching a station you detached (or that a session end revoked) starts a **new** attach
  lifecycle rather than resuming the old one: it gets the full five-minute window to finalize, and
  the daemon has to arm it again before anything may promote it. The tombstone's own arming proof is
  never inherited.
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
- `cargo build --workspace`
- `cargo test --workspace` — 20 test binaries, 0 failures (lib 377, `tests/station_intent.rs` 39,
  and `tests/daemon_process_sqlite.rs` 43)
- `cargo test --no-default-features --features sqlite --test daemon_process_sqlite --test station_intent station_intent_`
- `cargo test --no-default-features --features sqlite --test daemon_process_sqlite copilot_fallback`
- `cargo test --all-features --test conformance --test daemon_core_postgres` (Postgres rows skip
  cleanly when `TELEX_PG_URL` is unset)
- `node --check copilot/bridge/extension.mjs`, `node --test "copilot/bridge/*.test.mjs"` — 20 pass
- Feature-matrix builds with `RUSTFLAGS=-D warnings`: `--no-default-features --features sqlite`,
  `--features postgres`, `--features entra`, default, `--features sqlite,self-update`
- Compiled-out `telex upgrade` fail-closed check — exits 1 with the documented message
- `mdbook build docs/guide`

## Exact-scope repair after the full PAW review

The full review of `c13bad6441e74771280770793e76a302aafc6388...f2bd68e7064eb9571bc29e38b08035405100503b`
reported **5 Must and 7 Should** findings in pending review `5045956337`. This repair round addresses
all 12 findings and merges `origin/main@c67946ec494cdedb7defa638d953b831d76d6ec5` without rebasing
or rewriting history. That merge incorporates the accepted Copilot App lifecycle semantics from
#139 and the canonical Local Daemon design from #140. The repair code head before this evidence
update is `4f98341`; the final pushed head must include this document and is recorded in the PR body.

The campaign reserved ADR 0051 for Operator Station and allocated **ADR 0052** to Local Daemon
station intent. All 32 station-intent references across the 18 inventoried PR files now use 0052,
including ADR 0023's amendment pointer and the ADR heading. Operator Station references under
`.streamliner/**` remain 0051. No schema value, protocol value, runtime constant, or stored record
changed during the renumbering.

### Finding disposition

| Finding | Resolution |
|---|---|
| M1 | Unix owner-private tests extract errors without requiring `ProducerIdentity: Debug`. |
| M2 | Withdrawal is fallible and linearized with reconciliation. Pending records use exact-generation conditional deletion; live records transition to durable `Revoked`; reset, detach, fallback, and session end cannot race a restored member into existence. |
| M3 | One request-originated deadline bounds discovery, GC, waves, outcome withdrawal, cursor/evidence writes, event logging, and the admin response to four seconds. Scope filtering and cursors are store-correct. |
| M4 | A real producer-process identity change clears descriptor-specific durable failures while preserving lifetime counters; successor index reconstruction proves the reset survives restart. |
| M5 | The connected bridge peer is authenticated before any credential bytes are written, and responses are capped at 16 KiB with the exact boundary accepted. |
| S1 | Successor results retain structured nonzero output, name `successor_binary` on every branch, and kill/reap timed-out direct CLI children. |
| S2 | Capacity guidance states that revoked records retain their slot until the seven-day terminal TTL and daemon GC; detach does not free capacity immediately. |
| S3 | ADR 0052 provenance update completed for exactly 32 references in 18 files. |
| S4 | Windows producer-root documentation now matches the implemented principal allowlist and makes no AppContainer support claim. |
| S5 | Destructive process evidence covers hard-killed producer continuity and accepted-unacknowledged delivery, busy handoff, partial multi-store drain, and live-Postgres epoch fencing. |
| S6 | This evidence record and the PR description are refreshed after code and main integration. |
| S7 | Windows first boot-ID mint uses a global owner-only mutex; a barrier-driven 12-process cold start proves one persisted identity. |

### Repair validation

- `cargo build --workspace` passed.
- `cargo fmt --all -- --check` passed.
- `cargo clippy --all-features --workspace -- -D warnings` passed.
- `node --test copilot/bridge/*.test.mjs` passed: 20 tests.
- `cargo test --all-features --test station_intent` passed: 63 tests after M4/M5 and 62 at
  the M2/M3 checkpoint.
- The station-intent SQLite process selectors passed, including hard-kill continuity, busy
  successor handoff, partial multi-store drain, fallback withdrawal, and four restart/upgrade
  reconciliation cases.
- `cargo test --all-features --test boot_identity` passed: 6 tests; the platform filesystem unit
  selector passed 19 tests.
- The live Postgres fencing test passed repeatedly against a disposable PostgreSQL 16 instance in
  the implementation validation environment. The final local command skipped cleanly because
  `TELEX_PG_URL` was not set.
- `cargo test --workspace` reached **450 passed, 1 failed**. The failure is
  `application_client::tests::attach_rejects_duplicates_and_defines_empty_as_noop`; the identical
  diagnostic reproduces on untouched `main@c67946e`, so it is a current-main baseline failure rather
  than a station-intent repair regression.

The prior exact head had five green GitHub jobs and one Ubuntu compile failure. M1 fixes that compile
error, but no exact-head CI result exists until this repair is pushed. Native Unix trust checks,
macOS transport coverage, and authoritative Linux/Postgres CI therefore remain evidence gaps at
handoff. A focused or delta review must cover `f2bd68e..final-head`, including the main merge,
Application Client conflict resolution, M1-M5/S1-S7 repairs, and ADR 0052.

### Decision boundary

The requested next action is focused or delta re-review of the final clean pushed head, followed by
exact-head CI observation. A review +1 would be technical evidence only. It would not authorize a
merge, operate a builder gate, or start downstream work. Any head movement, reopened actionable
finding, required-check failure, or loss of clean mergeability invalidates that evidence.
