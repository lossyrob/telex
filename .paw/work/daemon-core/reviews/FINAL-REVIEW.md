# FINAL-REVIEW: Daemon Core Verbs (SQLite axis)

## Mode and Reviewer Summary

- Workflow: PAW Lite, Final Review Mode: society-of-thought (parallel,
  configured as a single ad-hoc specialist).
- Specialist: `general-reviewer` (claude-opus-4.7-high), per
  WorkflowContext. Built-in specialist rosters intentionally not invoked.
- Perspectives applied (cap: 2):
  - `premortem` — what fails in the field that the suite does not catch?
  - `retrospective` — does the landed implementation actually satisfy the
    node outcome anchor (daemon.md §17 SQLite axis) and the plan's frozen
    invariants?
- Bounded panel + synthesis; dissent preserved in the Dissent Log.
- Scope reviewed: `git diff origin/main...HEAD` (commit `059e7fc`).
  Primary files: `src/daemon.rs`, `src/daemon_ipc.rs`,
  `src/backend/sqlite.rs`, `src/commands/{attach,wait,send,reply,detach,
  disposition,daemon}.rs`, `src/session_watch.rs`,
  `tests/daemon_core_sqlite.rs`, `docs/design/daemon-delivery-budget.toml`,
  `SKILL.md`, `README.md`. No code modified by this review.

## Verdict (initial review)

`pass-with-nonblocking-notes`

## Verdict (after re-review)

`pass` — both Should-Fix items resolved. See **Re-review** section at
the end of this file.

The implementation lands the SQLite daemon-core slice required by
daemon.md §17. All 19 §17 rows have at least one executable test
(`tests/daemon_core_sqlite.rs::section17_01_*..section17_19_*`); the
SQLite store-lock, one-shot verbs, daemon-scoped IPC, lease-epoch fence,
non-destructive reaping, idle-TTL, operator reset, drain + next-call
respawn floor, capability redaction, and the §15 verb / SKILL cutover are
present and exercised. Verdict is **not blocking** — no high-confidence
correctness, security, or delivery-loss/duplication bug was found that
breaks the frozen contract in normal operation. Two important findings
that warrant fixes before broad rollout are listed under
**Should-Fix** (one is a clearly broken SQL migration path; the other
is a partial gap against §17 test 14's PID-reuse-race assertion). One
minor nonblocking item and several coverage-fidelity notes are recorded.

Counts:

- Must-Fix (blocking): 0
- Should-Fix (important): 2
- Nonblocking (minor / deferred): 4

If no blocking issues are found, that fact is stated explicitly above.

---

## Must-Fix Findings

**None.** No bug found that breaks the frozen §17 contract under
normal operation. Both perspectives concur.

---

## Should-Fix Findings

### S1. `ALTER TABLE ... ADD COLUMN IF NOT EXISTS` is a SQLite syntax error

- Severity: important
- File:location: `src/backend/sqlite.rs:422` (inside `do_schema`, in the
  branch guarded by `else if !has_column(c, "deliveries",
  "consumed_at_ms")?` at line 419).
- Issue: The migration emits
  `ALTER TABLE deliveries ADD COLUMN IF NOT EXISTS consumed_at_ms INTEGER;`.
  SQLite (verified locally against `sqlite3` 3.49.1, the version
  bundled by the project's `rusqlite = "0.32"`) does **not** accept
  `IF NOT EXISTS` on `ADD COLUMN` — it raises
  `near "EXISTS": syntax error`. Any v0 store that already has a
  `deliveries` table without a `consumed_at_ms` column will fail
  schema migration on first open and the store is unusable. The
  surrounding comments in the file (lines 5–10, 419–426) explicitly
  promise this v0→v2 migration path is supported.
- Concrete fix: Drop the `IF NOT EXISTS` clause — the branch is
  already guarded by `!has_column`, so plain
  `ALTER TABLE deliveries ADD COLUMN consumed_at_ms INTEGER;` is
  correct and sufficient. Add a regression test that seeds a v0
  `deliveries` table (no `consumed_at_ms`) and asserts
  `SqliteBackend::open_locked` + `init_schema` succeeds and back-fills
  the new column.
- Evidence: Verified syntax error with
  `python -c "import sqlite3; sqlite3.connect(':memory:').execute('ALTER TABLE t ADD COLUMN IF NOT EXISTS x INTEGER')"`
  on SQLite 3.49.1. The only legacy migration test currently in tree
  (`legacy_null_epoch_store_key` in `src/daemon.rs:2210`) seeds only a
  `leases` table, so the broken branch is never exercised in CI; the
  field-report should call this out so any user migrating a v0 store
  with a `deliveries` table is not silently bricked.

### S2. Client-side server-auth reads start time but never compares it (PID-reuse race)

- Severity: important
- File:location: `src/daemon.rs:3675` (Linux
  `platform::verify_server_peer` — `let _ = linux_start_time_ticks(pid)?;`)
  and `src/daemon.rs:3998–4027` (Windows `process_identity` —
  `start_time_100ns` field is captured then marked
  `#[allow(dead_code)]` and never compared).
- Issue: daemon.md §17 test 14 requires that the client-side
  server-auth performed **before** `Hello`/`store_key` disclosure
  prevents a PID-reuse race from authenticating the wrong process.
  The current implementation collects the server PID via
  `SO_PEERCRED` / `GetNamedPipeServerProcessId`, checks UID/SID
  match, checks exe canonical path, and **reads** the start time but
  never compares it against an expected value. The only remaining
  PID-reuse defense is the exe-path check, which only excludes
  non-`telex` binaries; another `telex` invocation by the same user
  that ends up with the daemon's old PID would pass authentication.
  (The local `tests/daemon_core_sqlite.rs::section17_14_os_trust_negatives`
  test acknowledges in a comment that the cross-user squat is not
  exercised, but it also does not exercise the PID-reuse axis at all.)
- Concrete fix: Persist the daemon's PID **and** start time in the
  capability file (`CapFile` already exists in
  `src/daemon.rs:223–239`), have `connect_existing` read them before
  the transport-level peer-credential lookup, and compare the
  observed start time against the capability-file value. Reject on
  mismatch. Add a regression test that swaps the start time and
  asserts `Unauthorized`.
- Evidence: `let _ = linux_start_time_ticks(pid)?;` discards the
  Linux result; `start_time_100ns` on `ProcessIdentity` is annotated
  `#[allow(dead_code)]` on Windows. No code path compares either
  value to an expected baseline. The §17 test 14 row explicitly
  requires "a PID-reuse race does not authenticate the wrong
  process".

---

## Nonblocking Notes / Deferred Observations

### N1. `spawn_daemon` discards stderr

- Severity: minor
- File:location: `src/daemon.rs:934–948`
- Issue: `stderr(Stdio::null())` makes daemon-startup failures
  invisible to the client during `connect_or_spawn`. Diagnostic
  ergonomics only — does not affect correctness.
- Fix sketch: capture stderr into a sibling log file under
  `run_dir` (owner-only) for the most recent spawn attempt.

### N2. §17 test 1 ("concurrent first-use") is single-process

- Severity: minor (coverage fidelity)
- File:location: `tests/daemon_core_sqlite.rs:118–154`
- Issue: The test races 8 tokio tasks against
  `platform::Listener::bind`, exercising the OS exclusivity
  primitive within one process. The plan's hook for §17 row 1
  calls for a "multi-process connect-or-spawn test". The
  single-process variant validates the primitive but not the
  cross-process auto-spawn loser path.
- Fix sketch: add an integration test that spawns N child telex
  processes that all call `connect_or_spawn` and asserts exactly
  one daemon survives (e.g. by reading the cap file's `instance_id`
  observed by each child).

### N3. `heartbeat_members_once` heartbeats idle members

- Severity: minor (intentional, worth recording)
- File:location: `src/daemon.rs:1071–1138`
- Issue: After `reset` / `sessionEnd` / `IdleTtlReap`, members are
  marked `idle=true` but the heartbeat loop continues to update
  `heartbeat_at_ms` on the lease row. This is consistent with the
  §10 "non-destructive reaping" contract (the lease and station
  remain), but it means an idle station never auto-releases its
  lease toward another writer. Acceptable for v1; worth recording
  for any future GC / handoff design.

### N4. `detach_member` swallows `release_epoch_lease` errors

- Severity: minor
- File:location: `src/daemon.rs:1681–1685`
- Issue: `let _ = backend.release_epoch_lease(...).await;` removes
  the in-memory member but ignores backend failures. If the
  release fails (transient SQLite error), the in-memory state and
  the durable lease drift: the daemon believes the address is
  free but the lease still names the previous owner. The next
  intra-daemon `register` would observe `AlreadyOwned`. Edge
  case, not in the §17 happy path; worth logging into
  `recent_errors` like the drain path does.

---

## Coverage Assessment Against §17 Rows

Mapping of §17 rows to the executable tests in
`tests/daemon_core_sqlite.rs`. All 19 rows have a SQLite-axis test;
"OK" means the test exists and asserts the row's key contract.
"OK (caveat)" means the row is covered but a specific assertion is
weaker than the design contract.

| §17 row | Test | Status | Notes |
|---|---|---|---|
| 1  Concurrent first-use            | `section17_01_concurrent_first_use`            | OK (caveat) | Single-process variant — see N2 |
| 2  Singleton + per-store lock      | `section17_02_singleton_and_store_lock`        | OK | Endpoint exclusivity + canonical-store lock + alias-path; same-SID single subcase only |
| 3  Crash-during-wait → NeedsAttach | `section17_03_crash_wait_needs_attach_reregister` | OK | `wait` reattach path exercised; CLI-side reconnect in `commands/wait.rs::wait_loop` covered by its own unit tests |
| 4  Restart: no loss/no resurrect   | `section17_04_restart_no_loss_no_resurrection` | OK | Durable consumed row retained across restart asserted |
| 5  Explicit ack + fan-out + dedup  | `section17_05_explicit_ack_fanout_dedup`       | OK | AckNoOp / AlreadyConsumed / per-recipient marking / cross-recipient isolation all asserted |
| 6  Multi-writer Postgres (PG only) | `section17_06_sqlite_single_writer_postgres_na`| OK | SQLite N/A asserted via canonical-store lock refusal |
| 7  Ownership-rotation race         | `section17_07_ownership_rotation_race`         | OK | `NotOwner` precedence over `AlreadyConsumed` asserted via successor-marks-then-predecessor-acks |
| 8  sessionEnd reaping              | `section17_08_session_end_reaping`             | OK | Non-destructive; waiter released; station survives |
| 9  Loader-pid death                | `section17_09_loader_pid_death`                | OK | pid + start-time guard; reuse-skew tested via `skew_first_watch_pid_start_time` |
| 10 Idle-TTL                        | `section17_10_idle_ttl_presence_ended`         | OK | PresenceEnded + station persists + re-arm on next message |
| 11 Operator reset                  | `section17_11_reset_non_destructive_audit`     | OK | Lease epoch unchanged; member idle; recent-error audit asserted |
| 12 Epoch monotonicity              | `section17_12_epoch_monotonicity`              | OK | After `ReleaseOwnership` at E, next claim is E+1 |
| 13 Ordered-handoff floor           | `section17_13_ordered_handoff_sqlite_floor`    | OK | SQLite release + next-call respawn floor; live two-daemon overlap refused |
| 14 OS trust negatives              | `section17_14_os_trust_negatives`              | OK (caveat) | verifier-before-Hello + redaction + bind exclusivity asserted; PID-reuse axis not asserted (see S2); cross-user squat skipped by design |
| 15 IPC version/capability compat   | `section17_15_ipc_compatibility`               | OK | Required cap mismatch + protocol major mismatch + Ack frame shape + NeedsAttach frame asserted |
| 16 Protocol-major parallel         | `section17_16_protocol_major_parallel`         | OK | Distinct singleton hashes + distinct cap paths asserted |
| 17 Durable BackendClock + TTL      | `section17_17_durable_backend_clock`           | OK | High-water clock asserted to never regress across restart |
| 18 Cross-store + from ambiguity    | `section17_18_cross_store_from_ambiguity`      | OK | Same `session_id` in two stores isolated; ambiguous `from` returns `Ambiguous`; unknown returns `NeedsAttach` |
| 19 Latency + retention budget      | `section17_19_delivery_budget`                 | OK | Reads `docs/design/daemon-delivery-budget.toml`; falsifiable p95/p99 + retention thresholds |

Net assessment: the §17 SQLite axis is green at the implementation
level. The two coverage caveats above (N2 / S2 sit under rows 1 and
14) are real but bounded; neither falsifies the row's primary
assertion as written in the matrix.

---

## Dissent Log and Synthesis Trace

The two perspectives produced largely overlapping findings; only one
substantive disagreement arose, recorded below.

### Premortem perspective

- Highest-confidence field-failure prediction: S1 (the
  `IF NOT EXISTS` syntax error). The bug is dormant in CI but
  trivially reachable by any user with a partially-migrated v0
  store; the failure mode is loud (cannot open store) but
  user-fatal.
- Second prediction: S2 (PID-reuse race). Low real-world
  probability under same-user assumption, but the design contract
  is explicit and the captured-but-discarded start-time strongly
  suggests an oversight, not an intentional waiver.
- Minor risks: stderr-eating spawn (N1) makes any startup failure
  feel like a hung daemon; idle members keep their leases live
  (N3) is consistent with the spec but means there is no upper
  bound on stored stations growing in `members` map / leases
  table beyond operator reset.
- Premortem flagged but did not promote to a finding: tight 100 ms
  poll in `wait_for_message_with_idle_ttl` (`src/daemon.rs:1847`)
  is a SQLite-write-on-every-iteration via `prove_current_owner`'s
  `heartbeat_epoch`. Under many concurrent waiters this multiplies
  WAL writes. Not blocking against the §13 budget; worth watching
  in scale tests.

### Retrospective perspective

- Confirmed the node outcome anchor is hit: SQLite daemon core +
  one-shot verbs + §17 axis are implemented and tested with the
  frozen invariants (epoch monotonicity, non-deleting release,
  retained consumed rows, durable clock, single-writer store
  authority, explicit ack with NotOwner precedence, owner-only
  cap/endpoint, verifier-before-Hello, NeedsAttach + reattach,
  Drain + next-call respawn floor, frozen exit codes).
- Confirmed docs / SKILL / CLI cutover (`SKILL.md`, `README.md`,
  `DISPATCH.md`, `TELEX.md`, `src/commands/*`, hidden
  `telex daemon` subcommand).
- Status redaction is observed in code paths (`push_recent_error`
  scrubs `admin_cap`; `singleton_key` uses
  `redacted_material()` that omits the SID). Tests assert no
  `admin_cap` in JSON.
- Retrospective flagged but did not promote: the daemon enforces
  one membership per `(store_key, address)` (`src/daemon.rs:1544`).
  The §17 row 5 phrase "two-waiters-both-print-then-one-ack" is
  realised in tree via the fan-out path (one session attending
  `addr:a` + `addr:b`, message `to=addr:a, cc=addr:b`), not via
  two distinct sessions attending the same address. This matches
  the explicit-only membership model in §14, so retrospective
  treats this as design-consistent, not a gap.

### Dissent

- **D1**: Premortem proposed promoting N3 ("idle members keep
  their leases live forever, no GC") to Should-Fix on the grounds
  that this is a long-term operational footgun (lease table can
  grow without bound). Retrospective dissented: daemon.md §13.1
  and the §17 row 19 retention threshold explicitly own bulk GC
  as the deferred `#24` work, and the field-report section of the
  plan already commits to reporting "deferred or out-of-node
  work". The non-destructive reaping property is also a frozen
  contract in §10.1. Synthesis: kept as N3 nonblocking, but
  explicitly noted as a known long-term scaling consideration the
  field report should surface for `seamless-upgrade` /
  `validation-harness` planning.

### Synthesis trace

1. Both perspectives independently identified S1 as the highest-
   confidence concrete bug. Promoted to Should-Fix; not blocking
   because CI never exercises the broken branch and the
   workstream is a core replacement (no production v0 stores
   asserted to exist), but the migration claim is documented in
   code comments and should not be left broken.
2. Both perspectives identified S2 as a partial contract gap
   against §17 row 14. Promoted to Should-Fix because the design
   text is explicit; not blocking because the same-user
   exe-canonical-path defense remains in effect and no realistic
   exploitation path is reachable without an additional bug.
3. N1 / N2 / N4 — both perspectives agreed these are minor and do
   not affect contract conformance. Recorded as nonblocking.
4. D1 reconciled in favor of retrospective's reading of the spec;
   recorded as N3 with explicit cross-reference to the deferred
   GC work.
5. No issue rose to "blocking" under either perspective.

End of initial review.

---

## Re-review (S1 + S2 only)

Re-review scope: only the fixes for S1 and S2 in the working tree
diff against `059e7fc`. Scope-limited per the user's request; no
rescan of the rest of the change set.

### Updated verdict

`pass` — both Should-Fix items resolved at the contract level with
correct, surgical fixes and regression coverage.

### Updated counts

- Must-Fix (blocking) — unresolved: 0
- Should-Fix (important) — unresolved: 0 (down from 2)
- Nonblocking (minor) — unchanged: 4 (N1–N4)
- New re-review observations promoted to findings: 0

### S1: `ALTER TABLE ... ADD COLUMN IF NOT EXISTS` — Resolved

- File:location: `src/backend/sqlite.rs:422` —
  `ALTER TABLE deliveries ADD COLUMN consumed_at_ms INTEGER;`
  (the `IF NOT EXISTS` clause removed; the surrounding
  `!has_column(c, "deliveries", "consumed_at_ms")` guard at line
  419 makes the conditional `IF NOT EXISTS` redundant anyway).
- Regression: `tests/conformance.rs:1243
  legacy_deliveries_table_migrates_consumed_column` seeds a v0
  `deliveries` table with no `consumed_at_ms`, then asserts
  `init_schema` succeeds and that the column is back-filled with
  the existing `delivered_at_ms` value (the documented
  do-not-redeliver semantics). Verified locally: `cargo test
  --quiet --test conformance legacy_deliveries_table_migrates_consumed_column`
  → `1 passed; 0 failed`.
- Synthesis: the broken v0→v2 migration path is now correct
  syntax and exercised in CI, closing both the bug and the
  coverage gap noted in the initial review.

### S2: client-side server-auth PID + start-time compare — Resolved

- Contract change:
  - `CapFile` (`src/daemon.rs:257–266`) carries two new
    `Option`-typed fields: `server_pid: Option<u32>` and
    `server_start_time: Option<u64>`, both with
    `#[serde(default, skip_serializing_if = "Option::is_none")]`.
  - `new_state` (`src/daemon.rs:1049–1055`) populates them from
    `std::process::id()` and
    `crate::session_watch::capture_process_start_time(...)` when
    the daemon writes its capability file at startup.
  - `connect_existing` (`src/daemon.rs:891–895`) reads the cap
    file **before** any transport-level interaction and passes
    `cap.server_pid` and `cap.server_start_time` into
    `platform::verify_server_peer`, which now compares observed
    peer PID + start time against the expected values from the
    cap file and rejects mismatches with `DaemonError::Unauthorized`.
  - Unix path (`src/daemon.rs:3691–3725`) reads
    `linux_start_time_ticks(pid)` and forwards it (no longer
    discarded via `let _ = ...`). Windows path
    (`src/daemon.rs:3950–3973`) now reads
    `info.start_time_100ns` and forwards it (the
    `#[allow(dead_code)]` annotation removed from
    `ProcessIdentity.start_time_100ns` at line 4062).
  - The non-Unix / non-Windows shim
    (`src/daemon.rs:4336–4347`) updated to the new signature
    and continues to fail closed with `Unsupported`.
  - New shared helper `verify_expected_peer_identity`
    (`src/daemon.rs:60–87`) centralises the comparison and
    fails closed when an expected value is present but
    observed is `None`.
- Regression: `src/daemon.rs:4437–4453
  peer_identity_rejects_pid_or_start_time_mismatch` asserts:
  (a) matching pid + start time accepts; (b) pid mismatch
  rejects with `Unauthorized`; (c) start-time mismatch rejects
  with `Unauthorized`; (d) missing observed start-time when
  an expected value is set fails closed. Verified locally:
  `cargo test --quiet --lib peer_identity_rejects_pid_or_start_time_mismatch`
  → `1 passed; 0 failed`. The two updated literals in
  `tests/daemon_core_sqlite.rs:761,773
  section17_16_protocol_major_parallel` add `server_pid` and
  `server_start_time` to the synthesised `CapFile` rows; the
  test still type-checks and runs (it does not exercise
  verify-time matching, only cap-file layout).
- Cross-platform start-time encoding verified:
  - **Linux:** writer
    (`src/session_watch.rs:160-166 process_start_time`) and
    reader (`src/daemon.rs:3754–3782 linux_start_time_ticks`)
    both parse `/proc/{pid}/stat`, drop everything through
    the last `") "`, and pick `fields[19]` as `u64`. Same
    jiffies value in both directions.
  - **Windows:** writer
    (`src/session_watch.rs:230–241 process_start_time`) and
    reader (`src/daemon.rs:4070–4087 process_start_time`) both
    composite the `creation: FILETIME` as
    `((dwHighDateTime as u64) << 32) | dwLowDateTime as u64`.
    Same 100-ns value in both directions.
- Backward-compat note (intentional, accepted, not promoted to
  a finding): `Option` typing means a cap file written by an
  older daemon (no pid/start_time) will deserialize to `None`
  and skip the compare, preserving forward-compat. This is a
  silent downgrade only if an attacker can write the cap file —
  which requires same-OS-user write access to the owner-private
  cap path, i.e. the v1 same-user trust model. Consistent with
  daemon.md §7.0 / §7.3 (intra-user isolation is explicitly
  out of scope in v1).

### Re-review dissent log

- **Premortem perspective:** floated whether the backward-compat
  `Option` typing should be tightened (require both fields
  present, reject otherwise) to prevent a silent downgrade. No
  exploitation path exists under the v1 same-user threat model
  (an attacker who can write the cap file already controls the
  user account and could write any cap file they want), and the
  daemon always writes both fields in `new_state`. Not
  promoted.
- **Retrospective perspective:** confirmed both fixes match the
  remediation prescribed in the initial review and that the §17
  row 14 contract assertion ("a PID-reuse race does not
  authenticate the wrong process") is now enforced end-to-end.
  No remaining contract gap on either Should-Fix axis.

### Re-review synthesis

Both perspectives concur. S1 and S2 are closed. Nonblocking
items N1–N4 from the initial review remain as-is; no new
findings were promoted. Updated verdict: `pass`.

End of re-review.
