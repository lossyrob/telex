# Station Intent Reconciliation Plan

Issue: <https://github.com/lossyrob/telex/issues/106>
Revision: post cycle-2 planning-docs review. Every cycle-1 finding (MF-1..MF-17, SF-1..SF-18,
CO-1..CO-5) and every cycle-2 finding (MF2-1, MF2-2, SF2-1..SF2-6, CO2-1..CO2-4) in
`reviews/planning/REVIEW-SYNTHESIS.md` is resolved below, and all six recorded trade-offs
(TO-1..TO-6) carry an explicit decision. Decision priority applied throughout: **Correctness >
Security > Reliability > Performance > Maintainability > Developer Experience.**

## Approach Summary

Add a host-local, owner-private, versioned **station-intent** layer that records the exact desired
push registration for a binding and never counts as attendance. Intents live under the daemon's
already-hardened runtime directory, namespaced by the daemon singleton hash, so they inherit the
fail-closed owner-private checks on both Windows and Unix while remaining isolated per config root
and per protocol major.

Copilot attach/resume writes a `pending` intent before registering and finalizes it to `live` only
after it has itself proved the producer endpoint answers an authenticated probe. The daemon reaches
producers exclusively through a **generic producer descriptor** (transport, endpoint path, process
identity, and a credential *pointer* - constrained to a registered owner-private producer root and
re-validated per file on both platforms - resolved at reconcile time), so the daemon core never
learns that the producer is Copilot and never caches a rotating secret. Handler restoration goes
through a **generic handler-kind registry** whose Copilot entry is registered by the harness layer,
and whose argv is rebuilt by one pure shared builder from the daemon's own executable and store
resolution rather than from persisted argv.

Reconciliation is a daemon-owned operation with its own code path (not a bare `Register`
side effect): single-flight, drain-suppressed, tombstone-checked before and after the epoch claim
with the tombstone-clearing branch structurally unreachable, generation-CAS guarded, CC-watermark
preserving, budgeted by both a per-pass count and a per-pass wall-clock deadline shorter than the
tick, per-intent timed out, deferred at a fixed cadence when the predecessor's lease is merely not
stale yet, exponentially backed off only on genuine failure, and partially failure-isolated. It runs
at daemon startup and on the existing 5 s heartbeat tick, is reachable as an explicit admin-proofed
request, and exposes one trigger plus per-pass reports as the only scheduling seam.

Membership and delivery authority are unchanged: in-memory `MemberRecord` plus the backend epoch
lease remain the only attendance and ownership truth. Intents are host-local files, so cross-host
Postgres cannot restore another host's bridge by construction, and each intent additionally carries
host and boot identity so a shared or synced home directory cannot defeat that.

## Constants and Published Bounds

All timing values below are named constants; no work item may hard-code a literal, and no test may
assert a literal where a constant exists (SF-2, MF-5).

| Constant | Home | Value | Notes |
|---|---|---|---|
| `STATION_INTENT_SCHEMA_VERSION` / `_MIN_SUPPORTED` / `_MAX_SUPPORTED` | `src/station_intent.rs` | `1` / `1` / `1` | constant + supported range, matching the repo's protocol convention (SF-7) |
| `RECONCILE_INTERVAL` | `src/daemon_reconcile.rs` | `HEARTBEAT_INTERVAL` (5 s, `src/daemon.rs:49`) | reconcile runs on the existing heartbeat tick, not a second loop (SF-2, DS-2) |
| `RECONCILE_TRIGGER` (in-process notify handle) | `src/daemon_reconcile.rs` | n/a - explicit trigger, no env cadence override | the seam startup, upgrade/rollback, `ReconcileIntents`, and tests pulse; `HEARTBEAT_INTERVAL` is never made env-overridable (SF2-1) |
| `RECONCILE_PASS_DEADLINE` | `src/daemon_reconcile.rs` | 4 s (must be `< RECONCILE_INTERVAL`) | wall-clock ceiling for one pass, so a pass can never overrun the tick that started it (MF2-1b) |
| `BRIDGE_PROBE_TIMEOUT` | `src/station_intent.rs` | 1 s | per-probe I/O ceiling; leaves 2 s for local validation and lease claim inside the per-intent budget |
| `RECONCILE_PER_INTENT_TIMEOUT` | `src/daemon_reconcile.rs` | 3 s | whole per-intent budget incl. probe + claim; must be `<= RECONCILE_PASS_DEADLINE` |
| `RECONCILE_PASS_BUDGET` | `src/daemon_reconcile.rs` | 64 intents/pass, round-robin cursor | bounded work (MF-10); an upper bound on a pass, not a guarantee it completes - the deadline may cut a pass short (MF2-1b) |
| `RECONCILE_MAX_CONCURRENCY` | `src/daemon_reconcile.rs` | 4 | herd cap (CO-1); also the guaranteed minimum progress per pass (MF2-1b) |
| `RECONCILE_BACKOFF_INITIAL` / `_MAX` / jitter | `src/daemon_reconcile.rs` | 5 s / 5 min / +-20 % | per-intent **failure** backoff (MF-6, CO-1); never applied to `DeferredLease` |
| `RECONCILE_DEFERRED_LEASE_RETRY` | `src/daemon_reconcile.rs` | `RECONCILE_INTERVAL` (5 s), fixed, no exponential growth, no jitter | retry cadence for the `DeferredLease` outcome - an incumbent lease that is simply not stale yet (MF2-1a) |
| `RECONCILE_QUARANTINE_AFTER` | `src/daemon_reconcile.rs` | 10 consecutive failures -> hourly retry | wedge prevention (MF-10); `DeferredLease` and `DeferredPullWaiter` do not increment the counter |
| `STATION_INTENT_MAX_COUNT` / `_MAX_BYTES` | `src/station_intent.rs` | 512 per scope / 16 KiB per file | input domain bound (MF-10, CO-5); over-cap scan behavior is defined in decision 15 (CO2-1) |
| `STATION_INTENT_PENDING_TTL` | `src/station_intent.rs` | 5 min | GC for crash-during-attach (MF-9) |
| `STATION_INTENT_UNVERIFIABLE_TTL` | `src/station_intent.rs` | 7 days | GC for orphans (MF-10) |
| `CREDENTIAL_MAX_AGE_MS_DEFAULT` | `src/station_intent.rs` | 24 h; a descriptor may only lower it | ceiling for the `max_age_ms` field of the credential descriptor (MF2-2) |
| `COPILOT_BRIDGE_PROTOCOL` | `src/commands/copilot.rs:72` | `1` -> `2` | `probe` verb added |
| `BRIDGE_PROBE_MIN_PROTOCOL` | `src/commands/copilot.rs` | `2` | below this the producer is *legacy*, not *failed* (TO-1) |
| `PROTOCOL_MINOR` | `src/daemon_ipc.rs:12` | `4` -> `5` | reconcile/status capability |
| `RECONCILE_MIN_DAEMON_MINOR` | `src/commands/copilot.rs` | `5` | client-side capability gate (MF-12a) |
| `MIN_COMPATIBLE_PLUGIN_VERSION` | `src/commands/copilot.rs:74` | `0.1.0` - **unchanged by this work** | fourth release axis; the plugin hook surface (`copilot/plugin/hooks.json`) is untouched, so it is asserted-unchanged rather than bumped (CO2-3) |
| `liveness_window_secs()` | `src/daemon.rs:5336` | default 15 s, env-overridable | already exists; read, never duplicated |

**Pass scheduling (MF2-1b).** A pass is a sequence of waves of at most `RECONCILE_MAX_CONCURRENCY`
intents, each intent bounded by `RECONCILE_PER_INTENT_TIMEOUT`. A pass stops when the scope is fully
swept, when `RECONCILE_PASS_BUDGET` intents have been attempted, or when the remaining
`RECONCILE_PASS_DEADLINE` is less than `RECONCILE_PER_INTENT_TIMEOUT` (no wave is ever started that
could outlive the deadline). Consequences, all of which the constants must keep true and a unit test
must assert: a pass never exceeds `RECONCILE_PASS_DEADLINE < RECONCILE_INTERVAL`, so it cannot
overrun the tick that started it and the single-flight guard never has to skip a tick in the normal
case; and every pass makes at least `RECONCILE_MAX_CONCURRENCY` intents of progress even when every
intent times out. The round-robin cursor persists across passes, so a cut-short pass resumes where
it stopped rather than restarting.

**Two published recovery bounds** (TO-5, MF-5a, MF2-1), both measured from "a compatible successor
daemon is running *and* the producer is live and verifiable", and both derived from the constants
above rather than asserted. **Both bounds are explicitly qualified** to an intent that is (i) in a
scope whose live-intent count is `<= RECONCILE_PASS_BUDGET`, so it is attempted in the first pass
after the trigger, and (ii) not currently in failure backoff, `DeferredPullWaiter` backoff, or
`Quarantined`. Scopes larger than one pass budget are covered by the separate queue-delay formula
below, not by these bounds.

- **Graceful drain / upgrade:** `RECONCILE_INTERVAL (5 s) + BRIDGE_PROBE_TIMEOUT (1 s) + local
  validation and backend claim allowance (2 s)` = **<= 8 s**, documented conservatively as
  **<= 10 s**. `drain_members` (`src/daemon.rs:3492-3529`) releases every
  epoch lease, so no stale-lease wait applies.
- **Hard crash:** `liveness_window_secs() + graceful bound` = **<= 25 s at the default 15 s
  window**. Derivation: a crashed predecessor never releases its lease, so `claim_epoch_lease`
  (`src/backend/sqlite.rs:1334-1335`) returns `AlreadyOwned` until `stale_cutoff_ms` passes. That
  outcome is classified as **`DeferredLease`**, not as a failure (see decision 6), so it retries at
  the fixed `RECONCILE_DEFERRED_LEASE_RETRY` = `RECONCILE_INTERVAL` cadence and never enters the
  exponential ladder. The first attempt at or after the cutoff therefore lands within one
  `RECONCILE_INTERVAL` of it - worst case `liveness_window_secs() + 5 s` - and adds
  `BRIDGE_PROBE_TIMEOUT + validation/claim allowance` = 3 s, giving no more than
  `liveness_window_secs() + 8 s`, documented conservatively as `liveness_window_secs() + 10 s`. Docs and
  tests express this as `liveness_window_secs() + 10 s`, never as the literal 25.

**Deterministic maximum queue delay for scopes larger than one pass (MF2-1b).** For a scope holding
`N` live intents with `N > RECONCILE_PASS_BUDGET`, the round-robin cursor gives eventual coverage
with a computable ceiling rather than a recovery bound: an intent waits at most
`ceil(N / P_min)` passes, where `P_min = RECONCILE_MAX_CONCURRENCY` (4) is the guaranteed per-pass
progress in the pathological case where every intent consumes its full timeout, and up to
`RECONCILE_PASS_BUDGET` (64) intents drain per pass in the healthy case. So
`max_queue_delay = ceil(N / P_min) * RECONCILE_INTERVAL`, and the intent's own recovery completes
within `max_queue_delay + <the applicable bound above>`. At the `STATION_INTENT_MAX_COUNT` = 512
cap this is `<= 128 passes * 5 s = 640 s` pathological and `8 passes * 5 s = 40 s` healthy. This
formula - not a fixed number - is what `documentation` publishes for large scopes and what the
`daemon-reconcile` 600-intent test asserts.

Both bounds explicitly exclude: no successor daemon exists (issue non-goal), backend outage, a
competing fresh owner, a legacy producer that cannot answer `probe`, quarantined intents, intents
in failure or pull-waiter backoff, and scopes larger than one pass budget (which use the queue-delay
formula above).

## Key Decisions

1. **Intent root (resolves MF-1, TO-4).** Intents live at
   `<run_dir>/intents/<singleton_hash>/<intent_id>.intent.json`, where `run_dir` is
   `daemon::resolved_runtime_dir()` (`src/daemon.rs:10362`) and `singleton_hash` is
   `SingletonKey::short_hash()` (`src/daemon.rs:178-224`) obtained by both writer and reader via
   `DaemonPaths::current()` (`src/daemon.rs:255-276`). This is the single root; every artifact
   states it identically. Rationale under the stated priority: `run_dir` is the directory ADR 0025
   designates as authority-bearing and the only one with a real fail-closed owner-private check on
   both platforms (`platform::ensure_owner_private_dir`, `src/daemon.rs:10528` unix /
   `src/daemon.rs:10896` Windows with `validate_owner_private_dir_shape` +
   `validate_owner_private_dir_security`), while the `<singleton_hash>` component - which hashes
   user identity, canonicalized config root, and `PROTOCOL_MAJOR` - preserves config-root isolation
   for destructive testing *and* namespaces the set per protocol major (SF-7). The Copilot bridge
   root keeps only bridge artifacts; the config root remains identity-only, so ADR 0025 is extended,
   not violated.
2. **Cross-platform secure create/read (resolves MF-1.2/1.3, MF2-2).** `daemon::platform`'s
   owner-private primitives are promoted verbatim into a shared `src/platform_fs.rs`
   (`ensure_owner_private_dir`, `write_owner_only_file`, plus a new `read_owner_only_file`), and
   `daemon::platform` re-exports them so no daemon behavior changes. Writes are
   `write_owner_only_file` to `<intent_id>.<rand>.tmp` (CREATE_NEW semantics) followed by
   `fs::rename` for atomic replace. Reads are fail-closed **per file, on both platforms**, so the
   rules do not depend on the file living inside the intent scope:
   - Both platforms: the file is opened first and every check is made on the open handle (no
     path re-resolution between check and read); it must be a regular file, must not be a
     symlink or reparse point, and must be `<= STATION_INTENT_MAX_BYTES`.
   - Unix: `uid == geteuid()` and `mode & 0o077 == 0` on the handle's `fstat`.
   - Windows: a new **per-file** validator `validate_owner_private_file_security(handle)` is added
     to `platform_fs`, modelled on the existing directory validators
     `validate_owner_private_dir_shape` (`src/daemon.rs:11088`, whose reparse-point rejection at
     `:11112` is the pattern for the file-level reparse check) and
     `validate_owner_private_dir_security` (`:11121`). It asserts: the file's owner SID equals the
     current user's SID; the DACL is present and non-null; the current user has the required read
     access; ACEs for the current user, `SYSTEM`, and local `Administrators` are allowed; and no ACE
     grants access to broad or unrelated principals such as `Everyone`, `Authenticated Users`, or
     `Users`. Inherited ACEs are accepted only when their trustee is in that same allowlist.
     Reparse points are rejected by opening with
     `FILE_FLAG_OPEN_REPARSE_POINT` and rejecting `FILE_ATTRIBUTE_REPARSE_POINT`. This closes the
     cycle-2 gap in which containment-in-the-scope-directory was the only Windows file check; the
     containing directory is still validated with `ensure_owner_private_dir` on every pass, but it
     is no longer the *only* check and is no longer required to be the intent scope.
   Any failure yields intent state `Insecure` (never `Live`), is logged to the reconcile event log,
   is surfaced in status, and is GC-eligible. The Unix-only helpers `ensure_private_dir` /
   `write_private_file` (`src/commands/copilot.rs:2212`, `:2327`) are not used for intents or for
   the credential read.
3. **Generic producer descriptor; the daemon never reads Copilot files (resolves MF-2, MF2-2, SF-6,
   TO-3 producer half, DS-6).** The intent carries `producer: ProducerDescriptorV1` of kind
   `local_endpoint_challenge_v1` with: `transport` (`named_pipe` | `unix_socket`), `endpoint_path`,
   `exe_path`, `pid`, `start_time`, `host_id`, `boot_id`, `protocol: {min,max}`, and
   `credential: { kind: "owner_private_json_field_v1", root_id, path, pointer, max_age_ms }`. At
   reconcile time the daemon resolves the credential by reading the owner-private JSON file at
   `path` with `read_owner_only_file` (decision 2's per-file, both-platform rules) and extracting
   the JSON pointer. It therefore always uses the *current* per-process secret and never persists
   one - the bridge's `randomBytes(32)`-per-process rotation
   (`copilot/bridge/extension.mjs:86`) becomes a non-issue rather than a permanent fail-closed. No
   secret is ever written into an intent, into status, or into the event log.

   **`path` is constrained, not free-form (MF2-2).** A credential path is only dereferenced when all
   of the following hold, and any failure yields `Unverifiable` with no connection attempt and no
   secret use:
   - `root_id` names a **producer root registered at composition time** in the same generic registry
     that owns handler kinds (decision 7) - the harness layer registers the Copilot bridge root;
     the daemon core still learns no Copilot path or filename, only "root `X` is registered".
   - The registered root is created and repaired by `platform_fs::ensure_owner_private_dir`, so its
     Windows DACL actually exists (Node's `mode: 0o700` / `chmod` in `extension.mjs` is a no-op on
     Windows); `copilot-intent` performs that ensure at attach time before any credential path is
     recorded. Files written by the bridge may retain the process token's normal safe
     current-user/SYSTEM/Administrators DACL; the per-file validator accepts that safe shape rather
     than requiring the intent directory's non-inheritable owner-only DACL to appear on an
     independently written file.
   - `path` canonicalizes to a location strictly under that root, with no `..` component, no
     symlink or reparse point anywhere on the resolved chain, and a canonical-prefix check made
     against the canonicalized root (not a string prefix).
   - The file itself passes decision 2's per-file checks, including the new Windows
     `validate_owner_private_file_security`. Failure of a security check yields `Insecure`, not
     `Unverifiable`.
   This keeps the credential read consistent with the rest of the manifest's posture (hashed
   filenames, CO-5; no persisted exe path, MF-17) rather than being the one place an
   attacker-influenceable absolute path reaches a filesystem operation.

   **`max_age_ms` has a defined outcome (MF2-2).** `max_age_ms` bounds the age of the credential
   file, measured as `durable_now - mtime` on the same open handle the read used, and is clamped to
   `CREDENTIAL_MAX_AGE_MS_DEFAULT` (a descriptor may only lower it, never raise it). When the file
   is older than `max_age_ms` the intent becomes **`Unverifiable`**: the secret is not read into
   memory, no connection is attempted, no probe is sent, the state is surfaced in status and the
   event log with `failure_code = credential_stale`, the attempt is backoff-eligible, and the intent
   becomes GC-eligible after `STATION_INTENT_UNVERIFIABLE_TTL` - the same terminal path decision 15
   already gives an *absent* credential file. A fresh attach/resume rewrites the credential file and
   the intent, which is the documented way out.

   The daemon core has no Copilot symbol, path, or filename knowledge:
   `push_delivery_health`'s "harness-neutral: never reads the bridge registry" doc-comment
   (`src/daemon.rs:2675`) is amended to "resolves only generic producer credential descriptors under
   registered producer roots" and the descriptor kind is the boundary. ADR 0039's harness-agnostic
   boundary is preserved, so **no ADR exception is required** for the challenge.
4. **Peer verification before any secret leaves the daemon; identity binding (resolves MF-3, MF-4).**
   The reconciler connects to `endpoint_path`, then calls
   `platform::verify_server_peer(&conn, exe_path, Some(pid), Some(start_time))`
   (`src/daemon.rs:10597` unix / `:10956` Windows) *before* sending anything. That single call
   verifies same-user ownership, executable match, and pid+start-time identity, which is exactly
   what binds the challenge responder to the recorded producer process. Only then is the probe sent:
   `{op:"probe", nonce, protocol}`; the response must echo the nonce byte-for-byte and carry the
   expected `session_id` and a `bridge_generation`. Strictness rules: (a) the reconciler never calls
   `process_alive_with_start_time` (`src/session_watch.rs:107`), whose `(_, _) => true` arm fails
   **open** when a start time is uncapturable; it uses a new `producer_identity_matches` that
   requires `Some(start_time)` on both sides and returns false otherwise; (b) `host_id` and
   `boot_id` mismatch make the intent stale, closing the Linux boot-relative
   `(pid, start_time)` reproducibility hole and the network-home hole (SF-16); (c) a platform that
   cannot resolve the peer executable, host id, or boot id fails closed (intent state
   `Unverifiable`), never verified. All three identity inputs come from the shared primitives named
   in decision 5, never from a parallel implementation.
5. **First-attach lifecycle and finalization (resolves MF-4 capture gap, MF-9 pending state,
   SF2-6).** Attach/resume writes the intent in state `pending` **before** `Register`. Bridge-mode
   attach still ignores `COPILOT_LOADER_PID` (`src/commands/copilot.rs:1004-1010`) - the loader pid
   is not the producer. After `daemon_armed_push` (`src/commands/copilot.rs:1847`) confirms the
   daemon armed push, the Copilot CLI performs the *same* probe the daemon will, reading pid/secret
   from the live bridge registry it already owns, captures the producer identity, and atomically
   finalizes the intent to `live` with `generation + 1`.

   **Named shared identity primitives (SF2-6).** The four identity fields are captured through
   exactly four functions, all shared, none duplicated in `src/commands/copilot.rs`:
   - `start_time` - the existing `capture_process_start_time(pid)` (`src/session_watch.rs:98`).
   - `exe_path` - a new `platform_fs::process_exe_path(pid)`, created by promoting the existing
     per-platform resolvers that are today private inside `daemon::platform`: `proc_pidpath`
     (`src/daemon.rs:10703`, macOS), `QueryFullProcessImageNameW` (`:11413`, Windows), and the Unix
     `server_executable` path used by `verify_server_peer` (`:10614`). `daemon::platform` re-exports
     it so `verify_server_peer` keeps calling the same code.
   - `host_id` - a new `platform_fs::host_id()` (stable machine identity: `/etc/machine-id` or
     `/var/lib/dbus/machine-id` on Linux, `IOPlatformUUID` on macOS,
     `HKLM\SOFTWARE\Microsoft\Cryptography\MachineGuid` on Windows), hashed before storage.
   - `boot_id` - a new `platform_fs::boot_id()` (`/proc/sys/kernel/random/boot_id` on Linux, boot
     time on macOS/Windows), hashed before storage.
   Every one of them returns `Result` and **fails closed** exactly as decision 4(c) promises: if any
   is unresolvable the attach does not finalize the intent, and on the reconcile side an intent
   whose value cannot be recomputed is `Unverifiable`, never verified.

   If the probe fails or any identity primitive fails, the existing rollback path
   (`src/commands/copilot.rs:1061-1069`) runs and the intent is removed. `pending` intents are never
   reconciled and are GC'd after `STATION_INTENT_PENDING_TTL`, so a crash mid-attach can never
   produce a claimable record. The reconciler takes the same per-`MemberKey` admission guard
   attach's `Register` takes (through the acquiring entry point named in decision 6), so it cannot
   claim a station out from under an in-flight attach.
6. **Reconciliation is its own daemon operation, not a `Register` side effect (resolves MF-6, MF-7,
   MF-8, MF-9, SF-15, MF2-1a, SF2-1, SF2-2, SF2-5, CO2-2).** New `src/daemon_reconcile.rs`.

   **Two-level API, so the inline caller cannot deadlock (SF2-5).** The module exposes exactly two
   entry points, and every caller states which one it uses:
   - `reconcile_once(state, scope) -> ReconcileReport` - the **acquiring** entry point. It owns the
     per-scope single-flight guard, the pass budget/deadline scheduling, and it acquires the
     per-`MemberKey` `delivery_admission` guard (`src/daemon.rs:815`, `:3805-3813`) for each intent
     before calling the locked routine. Used by the startup scan, the heartbeat tick, and
     `Request::ReconcileIntents`.
   - `reconcile_intent_locked(state, intent) -> IntentOutcome` - the **guard-free inner** routine.
     It *assumes the caller already holds* the per-`MemberKey` admission guard for that intent's key
     and never takes it. It is the only entry point decision 10's inline reconcile inside
     `register_member` may call, because `register_member` already holds that guard for the whole
     call (acquired at `src/daemon.rs:3800-3804`) and the in-code contract is explicit: "Admission
     is per station and outermost: never acquire it while holding another daemon lock"
     (`:3795-3796`). Calling `reconcile_once` from `register_member` would self-deadlock the
     hottest register path; the plan forbids it, and a debug assertion plus a unit test that calls
     the inline path under a held guard enforce it.

   The wire request is `Request::ReconcileIntents { proof, scope }` / `Response::Reconciled
   { report }`, with `proof` carrying the admin capability exactly like `Request::Drain { proof }`
   (`src/daemon_ipc.rs:287`) so an arming, process-spawning operation is not reachable from
   unproofed request paths.

   Member creation goes through a distinct `register_member_reconciled(...)` path (sharing helpers
   with `register_member`, `src/daemon.rs:3785-3934`) that owns its own ordering rather than
   inheriting a flag-coupled one:
   - **Tombstone ordering: own it, and never take the clearing branch (SF2-2).** Correct statement
     of the hazard: `register_member` *already* checks `backend.detach_tombstone(...)` before
     `claim_epoch_lease` (`src/daemon.rs:3962-3963` before `:3994`) and re-checks after the claim
     (`:4022-4023`) releasing the lease on a hit (`:4026`) - but **both checks are gated on
     `recovery`**, and the `recovery = false` branch *clears* the durable tombstone at
     `src/daemon.rs:4046`. The defect to guard against is therefore not a missing pre-claim check;
     it is that a reconciler inheriting the flag could take the tombstone-*clearing* branch and
     resurrect an explicitly detached station - the exact acceptance criterion "detached/tombstoned
     never auto-return". `register_member_reconciled` accordingly performs the pre-claim check, the
     claim, and the post-claim re-check *unconditionally* (no `recovery` flag at all), releases the
     lease and marks the intent `Revoked` on a post-claim hit, and **structurally cannot reach the
     clearing call**: tombstone clearing lives only on the explicit attach/resume path.
     Detach ordering is fixed: durable tombstone first, local intent revocation second, so a crash
     between them leaves tombstone-wins, which the reconciler already honors.
   - **Retry state preserved.** The first pass (no member) passes the descriptor-derived
     `on_deliver`. Every subsequent pass for a member that already exists passes
     `on_deliver: None, replace_on_deliver: false`, taking the preserving branch
     (`src/daemon.rs:3847-3865`) and never re-entering `on_deliver_forget_member` +
     `spawn_on_deliver_backlog` (`src/daemon.rs:3906-3918`). A no-op refresh performs no backend
     write at all.
   - **CC watermark preserved.** The intent persists `cc_watermark_ms`, captured from
     `MemberRecord.on_deliver_cc_after_ms` at finalize time. Reconciliation passes that value
     through instead of recomputing `on_deliver_cc_lower_bound` (`src/daemon.rs:4132`) = *now*, and
     the value is monotonic non-increasing across reconciles. Without this, every CC message
     committed during the restart gap becomes permanently invisible.

   **Per-intent outcome classes, and which ones back off (MF2-1a).** `IntentOutcome` is typed and
   drives the retry policy; only the third class is backoff-eligible:
   - `Restored` / `RefreshedNoOp` - success; failure counters reset.
   - **`DeferredLease`** - `claim_epoch_lease` returned `AlreadyOwned` and the incumbent lease is
     simply **not yet stale** (`durable_now < lease_heartbeat + liveness_window_secs()*1000`; the
     cutoff is `stale_cutoff_ms`, `src/backend/sqlite.rs:1334`, `:1404`). On the crash path this is
     the *expected* outcome of every attempt in the first `liveness_window_secs()`, so it is **not a
     failure**: it retries at the fixed `RECONCILE_DEFERRED_LEASE_RETRY` = `RECONCILE_INTERVAL`
     cadence with no exponential growth and no jitter, does not advance the
     `RECONCILE_QUARANTINE_AFTER` counter, and is projected as a normal waiting state rather than an
     error. This is what makes the published crash bound derivable (see Constants and Published
     Bounds). `DeferredPullWaiter` (decision 13) is likewise deferred, but with its own backoff
     because the wait is unbounded in principle.
   - `Failed { code }` - probe failure, credential failure, backend error, timeout, or a lost claim
     to a *fresh* competing owner. This is the only class that enters
     `RECONCILE_BACKOFF_INITIAL` -> `_MAX` with jitter and counts toward quarantine.
   - Terminal/inert classes (`Revoked`, `Tombstoned`, `Insecure`, `Incompatible`, `LegacyProducer`,
     `Unverifiable`) are not retried on the fast cadence; they are surfaced and GC-governed.

   **Cadence and triggers (SF2-1).** There is exactly one loop: the reconcile pass is invoked from
   the existing `heartbeat_loop` (`src/daemon.rs:3253`), and `HEARTBEAT_INTERVAL` is **not** made
   env-overridable (it carries a documented, test-enforced invariant against
   `ON_DELIVER_DEFERRED_BACKSTOP`, `src/daemon.rs:2370-2375`, asserted at `:7039`). The previously
   proposed `TELEX_RECONCILE_INTERVAL_MS` env override is **removed** - riding the heartbeat tick
   means it could only ever lengthen the interval, which is useless for the bound tests it was
   introduced for. It is replaced by an explicit in-process **trigger + completion notification
   seam** (`RECONCILE_TRIGGER`): the heartbeat tick, the startup scan, upgrade/rollback, and
   `Request::ReconcileIntents` all pulse the same trigger, and each completed pass publishes a
   `ReconcileReport` on a broadcast/watch channel with a monotonically increasing `pass_seq`.
   Production pulses respect each intent's `next_attempt_ms`; they schedule work but never bypass
   backoff, quarantine, `DeferredPullWaiter`, or `DeferredLease`. Tests advance an injected
   monotonic clock until the target intent is due, pulse the trigger, and await the next `pass_seq`
   report instead of sleeping on wall clock; bound
   assertions are then computed from the constants plus observed per-pass timings rather than from
   a generous poll. The same seam is what makes `drain-upgrade`'s bounded wait for a post-switch
   reconcile report implementable without polling.

   **Bounded, non-overrunning passes (MF2-1b).** Passes follow the Pass scheduling rules in the
   Constants section: waves of at most `RECONCILE_MAX_CONCURRENCY`, per-intent
   `RECONCILE_PER_INTENT_TIMEOUT`, stop at `RECONCILE_PASS_BUDGET` or `RECONCILE_PASS_DEADLINE`,
   round-robin cursor persisted across passes. `state.is_draining()` suppresses reconciliation
   exactly as `heartbeat_members_once` does (`src/daemon.rs:3262-3264`); the per-scope single-flight
   guard plus the per-`MemberKey` `delivery_admission` taken by `reconcile_once` prevents
   overlapping passes and duplicate epoch claims; store resolution is **open-existing-only** (a
   missing SQLite file or unconfigured profile marks the intent `Unverifiable` and never creates a
   store); failures are isolated per intent and reported as counts, so a partial pass is never a
   half-owned member set.

   **Cached in-memory intent index (CO2-2).** The daemon holds an `IntentIndex` in `DaemonState`:
   a map from `(store_key, session_id, address)` to `{ state, generation, delivery_mode,
   last_attempt_ms, last_success_ms, next_attempt_ms, failure_code, mtime }`. It is built by the
   startup scan, is rewritten by every reconcile pass for the entries that pass touches (including
   entries the scan observed but the budget skipped, which are indexed from their manifest header
   only), and is updated in place by every daemon-side intent mutation (revocation on detach or
   session end, GC removal, quarantine). Client-side writes (attach/resume) are picked up on the
   next pass, so index staleness is bounded by `RECONCILE_INTERVAL` in the steady state and by
   `max_queue_delay` for over-budget scopes. The index is the **only** source for the drain report
   (`drain-upgrade`), which is therefore I/O-free by construction; the report carries
   `index_as_of_ms` so an operator can see the staleness rather than assume freshness.
7. **Generic handler-kind registry, one pure argv builder, explicit executable and store resolution
   (resolves MF-17, SF2-3, SF2-4, TO-3 handler half).** New `src/handler_kinds.rs` holds a generic
   `HandlerKindRegistry`; `src/commands/copilot.rs` registers `telex_copilot_push_v1` at composition
   time. The daemon core only knows "this kind is registered and trusted". The persisted descriptor
   carries **no executable path and no `--backend`/`--db` strings**; its only parameter is
   `session_id`, validated against the intent's own `session_id` and a strict charset/length rule.
   Unknown or unregistered kinds are never launched.

   **The argv refactor is explicit (SF2-3).** `bridge_handler_argv(ctx, session_id)`
   (`src/commands/copilot.rs:288-306`) cannot be the daemon's builder as it stands: it takes a
   client `Ctx` the daemon does not have, takes no executable parameter, and derives `--backend` /
   `--db` from `ctx.cfg` (`:293-302`). It is therefore **split**:
   - `build_push_argv(exe: &Path, selector: &StoreSelector, session_id: &str, instance_id: &str)
     -> Vec<String>` - a pure, dependency-free function in `src/handler_kinds.rs`. It is the single
     owner of argv shape and is the only place `--daemon-instance` is appended (SF2-4).
   - `bridge_handler_argv(ctx, session_id)` becomes a thin client-side wrapper: it resolves the
     executable and a `StoreSelector` from `ctx.cfg` exactly as today, reads `instance_id` from the
     cap file, and calls `build_push_argv`. Behavior for existing attach callers is unchanged.
   - The daemon-side handler-kind builder calls `build_push_argv` with the daemon's own inputs.
   A unit test asserts both callers produce byte-identical argv for the same
   (exe, selector, session, instance).

   **Executable resolution, named accurately (SF2-3.2).** The daemon-side builder passes
   `canonical_current_exe()` (`src/daemon.rs:10340`), which is `std::env::current_exe()` +
   `fs::canonicalize`: **the real path of the currently running, installed versioned binary** -
   under the versioned layout `versions/<tag>/telex` (`src/install.rs:1`, `maybe_dispatch_launcher`
   at `:189-240`, `LAUNCHER_GUARD_ENV` at `:12`). Canonicalization resolves *away from* any stable
   launcher shim, so this plan does **not** describe it as "the running stable launcher" (the
   previous revision did, incorrectly). The consequence is stated rather than glossed: a restored
   handler targets the exact binary the reconciling daemon is running, which is matched-version by
   construction and is identical to today's `bridge_handler_argv` behavior (it already bakes
   `std::env::current_exe()`), so this is not a regression. It also means the cycle-1 concern
   "launcher target must stay resolvable across upgrade and rollback" is answered by **re-derivation,
   not by path stability**: argv is never persisted, and the successor daemon installed by
   `upgrade` / `rollback` rebuilds it from its own `canonical_current_exe()` on the post-switch
   reconcile pass (decision 14c).

   **Store selector resolution is one named, fallible mapping (SF2-3.3).** The daemon holds an
   opaque `store_key`, so a `store_key -> StoreSelector` mapping is unavoidable; the plan names it
   rather than claiming it does not exist. New `daemon::store_selector_for_key(&state, store_key)
   -> Result<StoreSelector>`: for Postgres it reuses the existing profile scan
   `resolve_postgres_profile_for_store_key` (`src/daemon.rs:2184-2196`, which compares
   `crate::profiles::store_key(profile, None)`); for SQLite it emits the `--db` path of the store
   the daemon already opened. **Failure mode**: no match, an ambiguous match, or a profile that no
   longer exists yields intent state `Unverifiable` with `failure_code = store_selector_unresolved`
   - no member is created and no store is opened, consistent with the open-existing-only rule of
   decision 6. Correct claim: there is exactly **one** argv shape (`build_push_argv`) and exactly
   **one** selector-resolution function per side, with a test asserting the daemon-side mapping
   agrees with the client-side `ctx.cfg` resolution for the same store.
8. **Process fencing and in-flight helpers (resolves SF-10 second half, SF2-4, AC11, matrix T21).**
   Epoch advancement fences the daemon, not helper processes already spawned by a dying one. Two
   concrete guards: (a) graceful drain waits, bounded by the existing `--drain-timeout-ms`, for
   in-flight `on_deliver` tasks before releasing leases, so the graceful path has zero overlap;
   (b) **every** push helper argv carries `--daemon-instance <instance_id>` - which is guaranteed
   because the flag is appended by the shared `build_push_argv` of decision 7, so the attach path
   and the daemon-side handler-kind builder cannot diverge (SF2-4). **The attach path sources the
   instance id from the cap file it already reads on connect**: `CapFile.instance_id`
   (`src/daemon.rs:286`) via `read_cap_file` (`:1731`, already called by the client connect path at
   `:1815`), which a successor rewrites in `new_state` (`:2072-2090`). The helper re-reads the cap
   file (`DaemonPaths::current().cap_path`) immediately before injecting a turn, aborting if
   `instance_id` changed - which is precisely the crash-path window. Residual, deliberately
   accepted: a helper spawned by a crashed daemon *before* any successor exists may still inject;
   that is the at-least-once behavior the issue permits, and the old owner still cannot mark
   consumption because `mark_consumed_if_current_owner` (`src/backend/mod.rs:159-170`) is
   epoch-guarded.
9. **Suppression state after restart (resolves SF-10 first half, matrix T23).** Push suppression,
   backoff, and dead-letter state are in-memory and there is no durable API for them; adding one is
   out of scope. Decision: a reconciled member starts with clean push-attempt state, so a
   previously hard-capped push may be attempted once more after a daemon replacement. This is
   documented as at-least-once behavior, is bounded by the same backoff/backstop/hard-cap policy
   (`docs/design/daemon.md` sec.13.2), and cannot produce a retry storm because reconciliation
   refreshes never reset that state (decision 6).
10. **Anti-downgrade enforcement lives in the daemon (resolves MF-11, SF2-5, AC7, matrix
    T18/T19/T25).** The guard is inside `register_member`, alongside the existing
    `DeliveryModeConflict` tripwire (`src/daemon.rs:3815-3830`), so it also covers older clients and
    `telex attach`. When a `Register` would create a **new** member (no existing record) for a key
    that has a `live` intent declaring push, the daemon: calls
    `daemon_reconcile::reconcile_intent_locked(...)` - the **guard-free inner** entry point of
    decision 6, because `register_member` already holds the per-`MemberKey` admission guard for this
    key (`src/daemon.rs:3800-3804`) and that guard is documented as outermost and non-reentrant
    (`:3795-3796`); calling the acquiring `reconcile_once` here would deadlock the register path and
    is explicitly forbidden. On success the incoming registration is treated as a refresh (which
    preserves `on_deliver`); on failure the daemon returns typed `ERROR_INCOMPATIBLE` with a
    `PushIntentUnrecoverable` reason and the specific cause. It never creates a pull-only member
    over a live push intent. Client-side, `wait` and `send`/`reply` recovery
    (`src/commands/wait.rs:351-368`, `src/commands/send.rs:118-141`) render that typed error
    actionably; that is messaging only, not the enforcement layer.
11. **Version skew and legacy posture (resolves MF-12, TO-1, AC12).**
    - **Daemon minor skew (new client, old daemon).** A client-side capability gate modeled on
      `ensure_fallback_protocol` (`src/commands/copilot.rs:2503`) refuses to write or finalize an
      intent when the connected daemon's minor is `< RECONCILE_MIN_DAEMON_MINOR`, prints the
      existing "restart/update the daemon" guidance, and - critically - suppresses the pull-only
      auto-registration for an address that has a local live push intent, so a 1.4 daemon cannot be
      used to silently downgrade. This closes the gap left by `SingletonKey` hashing only
      `protocol_major` (`src/daemon.rs:178-224`).
    - **Legacy producers are legacy, not failures.** A bridge whose registry advertises
      `protocol < BRIDGE_PROBE_MIN_PROTOCOL`, or that lacks the field entirely (the resident-JS case
      `write_bridge_extension` cannot reload, `src/commands/copilot.rs:181-187`), yields intent state
      `legacy_producer` - not `incompatible`. Legacy intents are never auto-reconciled (liveness is
      unprovable) but they also never wedge anything: status reports them, the turn guard warns, and
      the documented manual `telex copilot resume` path (`docs/guide/src/guides/troubleshooting.md:37-45`)
      keeps working exactly as today. Fail-closed is thereby scoped to *advertised-but-unverifiable*
      producers, which is the TO-1 resolution both dissenting reviews proposed. "Fail closed" means
      **never silently restore a reduced mode**; it does not mean blocking agent turns.
    - **Rollback.** `telex rollback` gains the same pre-flight report as upgrade and warns when live
      intents exist that the target binary cannot reconcile. Intents are never deleted by rollback;
      an older daemon simply ignores an unknown directory, and the `<singleton_hash>` namespacing
      plus schema range keeps a pre-feature binary inert with respect to them. The documented
      consequence of rolling back is a return to manual `copilot resume`, surfaced in the rollback
      output.
12. **Turn guard warns; it never blocks (resolves SF-9, and the MF-10 wedge).** `evaluate_guard`
    (`src/commands/copilot.rs:1956-2089`) gains a reason code `push_intent_unrestored` that returns
    a **neutral/allow decision with a warning**, replacing today's silent `no_attended_stations`
    allow for the specific case "this session has a live local intent but no push member". The
    daemon-unavailable fail-open invariant is restated verbatim and regression-tested. Rationale
    under the priority order: blocking every agent turn on a recovery-state condition converts one
    orphaned intent into a wedged session (MF-10) and buys no delivery correctness, because the
    guard cannot deliver messages.
13. **Pull-waiter precedence is preserved (resolves SF-8, TO-2, AC13, matrix T6).** The existing
    rule stands: a live armed pull waiter wins, and `register_member` continues to reject a push
    registration in that state. Reconciliation does not force the conflict - it records intent state
    `deferred_pull_waiter` and retries with backoff, so the loser is *not* permanent, and status
    shows `CoverageConflict`. The anti-downgrade guarantee of decision 10 is therefore scoped to the
    case with no live armed waiter, which is the only reading compatible with the acceptance
    criterion "existing pull-wait reconnect behavior remains intact".
14. **Successor and wake triggers (resolves MF-5b, TO-6, AC1, AC2).** No bridge-side active loop and
    no bridge-spawned processes: the bridge remains observation-only, which keeps the race the risk
    assessment named and the new-exec surface out of the design. The named triggers are:
    (a) **daemon startup scan**, in `serve()` after `new_state` (`src/daemon.rs:2037-2090`);
    (b) **heartbeat-tick reconcile** every `RECONCILE_INTERVAL`;
    (c) **upgrade/rollback successor** - `perform_upgrade` (`src/commands/upgrade.rs:248-266`) and
    `rollback` (`:356-382`) gain a post-switch step that spawns the successor via the existing
    `connect_or_spawn` path (`src/daemon.rs:1859`, `spawn_daemon`) and waits, bounded, for the
    reconcile report before printing results. This is a deliberate, bounded extension of ADR 0028
    ("only `attach` auto-spawns"), recorded in the new ADR, and it is what makes the issue's
    motivating scenario - `telex upgrade` with an idle Copilot session - recover automatically;
    (d) **explicit request** `ReconcileIntents`, issued by the Copilot `agentStop` drain hook when a
    daemon is already reachable (never spawning one).
    Idle hard crash with no successor is out of scope by the issue's own non-goal ("keeping delivery
    active while an explicitly stopped daemon has no successor"); the first successor spawned by any
    later client operation performs (a) and recovers every intent in the scope. Status reports the
    degraded state in the meantime.
15. **Intent lifecycle, TTL, caps, over-cap behavior and GC (resolves MF-10, CO-5, CO2-1).** States
    are typed (decision 16). Bounds: `STATION_INTENT_MAX_COUNT` per scope (writes beyond it fail
    with a typed error rather than silently growing), `STATION_INTENT_MAX_BYTES` per file, hashed
    filenames `sha256(store_key \| 0x1f \| session_id \| 0x1f \| address)` truncated to 32 hex chars
    plus `.intent.json`, so address/store strings never reach the filesystem path (path-traversal
    and collision surface closed).

    **Over-cap scan behavior (CO2-1).** The cap is a write-time rule, so a scope can legitimately
    hold more than `STATION_INTENT_MAX_COUNT` entries (an older build, a manual copy, or a GC that
    has not yet run). The scanner therefore: enumerates the whole directory but never loads more
    than `RECONCILE_PASS_BUDGET` manifests per pass; **never deletes an entry merely for being over
    cap** (deletion is GC's job and only under the TTL rules below); processes entries in the
    deterministic `(store_key, address, generation desc)` order of decision 6 with a persisted
    round-robin cursor keyed by the last processed sort position, so coverage is eventual and
    complete even above the cap - the cursor advances past processed entries and wraps at the end,
    and entries added or removed between passes cannot starve another entry because the cursor
    compares sort position, not index; refuses new writes with the typed cap error while over cap;
    and reports `over_cap: true` plus the observed count in status, the drain report, and the
    reconcile event log so the condition is visible rather than silent. GC pressure is applied in
    the usual order (expired `Pending` first, then `Unverifiable` / `Insecure` past TTL, then
    intents whose credential root or file is gone), which is the only mechanism that brings a scope
    back under the cap.

    GC runs in the same bounded maintenance tick and from `telex copilot gc`: `pending` older than
    `STATION_INTENT_PENDING_TTL`, `unverifiable`/`insecure` older than
    `STATION_INTENT_UNVERIFIABLE_TTL`, intents whose credential file is absent (or whose registered
    producer root is no longer registered), and intents for a different `host_id`/`boot_id` whose
    producer is provably gone. `copilot gc`'s keep heuristic is re-pointed at intents first, with
    `.bindings.json` as a secondary hint.
16. **Typed states and precedence (resolves SF-4, SF-7).** New `IntentRecoveryState` enum in
    `src/daemon_ipc.rs`, serde-tagged with an `Unknown` catch-all like `StationHealth`
    (`src/daemon_ipc.rs:551-570`): `Pending`, `Live`, `Restored`, `DeferredLease`,
    `DeferredPullWaiter`, `LegacyProducer`, `Incompatible`, `Unverifiable`, `Insecure`,
    `Quarantined`, `Revoked`, `Tombstoned`, `OwnershipConflict`, `Unknown`. Precedence when several
    apply, highest first: `Tombstoned` > `Revoked` > `Insecure` > `Incompatible` >
    `OwnershipConflict` > `Quarantined` > `Unverifiable` > `LegacyProducer` > `DeferredPullWaiter` >
    `DeferredLease` > `Pending` > `Restored` > `Live`. `DeferredLease` (decision 6) is projected as
    a *waiting*, non-error state with `next_attempt_ms` set from
    `RECONCILE_DEFERRED_LEASE_RETRY`, so an operator watching the crash path sees "waiting for the
    predecessor's lease to go stale", not "failing".
    Vocabulary rule: when a `MemberRecord` exists, `StationHealth` and `PushDeliveryHealth` remain
    authoritative and the intent state is supplementary; when no member exists, the intent state is
    the only projection. `NeedsAttachReason` (`src/daemon_ipc.rs:307-311`) gains `PushIntentPending`
    and the missing `Unknown` catch-all. Manifests preserve unknown fields on rewrite so a V1 daemon
    cannot silently drop a future field.
17. **Legacy `.bindings.json` truth ordering and removal (resolves SF-5).** No intent is ever
    synthesized from legacy bindings - the "legacy migration helpers" of the previous revision are
    removed as contradictory and unsafe (a synthesized intent would be indistinguishable from an
    authentic one and cannot carry store/mode/CC truth). Truth ordering: the intent manifest is
    authoritative for recovery and for `copilot gc` keep decisions; `.bindings.json` remains
    authoritative **only** for the extension teardown ref-count, is written after the intent in a
    fixed order, and is scheduled for removal in the release after the one that ships this feature -
    recorded in the ADR and in `docs/developing/releasing.md`. Drift between the two is reported by
    `copilot gc` and by station status rather than silently repaired.
18. **Diagnostics carry evidence, not just state (resolves SF-3).** Every intent projection carries
    `last_attempt_ms`, `last_success_ms`, `attempts`, `failure_code`, `producer_verified_ms`,
    `next_attempt_ms`, and `recovery_latency_ms`, matching the existing `*_since_ms` / `*_for_ms` /
    `*_count` idiom on `MemberStatus` (`src/daemon_ipc.rs:456-537`). Status intent rows are
    projected from the cached `IntentIndex` (decision 6) and carry `index_as_of_ms`, so a reader can
    distinguish "no live intent" from "index not refreshed since the last pass". A rotating NDJSON
    reconcile event log (`<run_dir>/intents/<singleton_hash>/reconcile-events.ndjson`, single
    rotation to `.ndjson.1`) reuses the `HOOK_LOG_FILE` idiom (`src/commands/copilot.rs:29`,
    `:2748-2765`) with a size cap, no secrets, and no raw argv. Intent rows are part of the
    authenticated status projection, never the uncapped `status_minimal` projection.
19. **Session end revokes intent (resolves SF-17).** The daemon's own `end_session_members` /
    sessionEnd / watch-pid-death paths mark matching intents `revoked`; because intents are generic
    and daemon-owned, this needs no Copilot coupling. An ended session can therefore never be
    re-attended by a stale intent.
20. **Authority split unchanged (affirmed by the review's Observations).** Intent is desired local
    state; in-memory `MemberRecord` plus the backend epoch lease remain the only attendance and
    delivery authority. No backend schema migration, no new lease/tombstone/delivery mechanism, and
    membership is still never rebuilt from history - restoration requires a *live, verified*
    producer, which is what distinguishes it from resurrection.
21. **ADR hygiene (resolves SF-18).** One new ADR **0050 - "Durable station intent and daemon-owned
    reconciliation"** (next free number; the highest existing is 0049 at
    `docs/design/DECISIONS.md:2038`). It records `Revises: 0023 (amended - intent is desired state,
    not membership; the never-rebuild-from-history invariant is preserved), 0025 (extended - run_dir
    now also carries intent manifests), 0028 (bounded exception - upgrade/rollback may spawn the
    successor they just installed), 0039 (clarified - generic descriptor kinds keep the daemon
    harness-agnostic)`. ADR 0023's status line is updated to note the amendment. The pre-existing
    duplicate `0039` heading (`DECISIONS.md:1497` and `:1581`) is left untouched and is explicitly
    out of scope; the new entry does not add to the collision.
22. **Cross-host and cross-store guarantees.** Local files are not remotely discoverable, and
    `host_id` + `boot_id` binding makes a network-mounted or synced home a non-defeat (SF-16). The
    same address on multiple stores is distinguished because store key is part of both the intent
    identity and the hashed filename.

## Work Items

Dependency order (SF-1). `paw-lite` parallel dispatch must respect these edges; items on the same
level may proceed in parallel.

| Level | Items (parallel within a level) | Depends on |
|---|---|---|
| 0 | `platform-fs`, `bridge-protocol` | none |
| 1 | `intent-model`, `handler-registry` | `platform-fs` |
| 2 | `test-harness` | `bridge-protocol`, `intent-model` |
| 2 | `copilot-intent` | `intent-model`, `handler-registry`, `bridge-protocol` |
| 2 | `daemon-reconcile` | `intent-model`, `handler-registry` |
| 3 | `anti-downgrade` | `daemon-reconcile`, `copilot-intent` |
| 4 | `diagnostics` | `daemon-reconcile`, `anti-downgrade` |
| 5 | `drain-upgrade` | `diagnostics` |
| 6 | `tests` | `test-harness`, `anti-downgrade`, `diagnostics`, `drain-upgrade` |
| 7 | `documentation` | `drain-upgrade`, `tests` |
| 8 | `verification` | all |

- [x] **Promote owner-private filesystem and process-identity primitives to a shared module**
  (`platform-fs`)
  - Depends on: none.
  - Move `ensure_owner_private_dir`, `write_owner_only_file` and the Windows
    `create_owner_only_dir` / `validate_owner_private_dir_*` / `owner_only_security_attributes`
    helpers from `daemon::platform` into `src/platform_fs.rs`; re-export from `daemon::platform` so
    the daemon, cap-file, and socket paths are byte-for-byte unchanged.
  - Add `read_owner_only_file(path, max_bytes)` implementing decision 2's fail-closed **per-file**
    read rules on handle (regular file only, no symlink/reparse point, size cap, Unix uid +
    `mode & 0o077`), plus `write_owner_only_file_atomic(path, bytes)` (temp + rename).
  - Add the new Windows per-file validator `validate_owner_private_file_security(handle)` (owner SID
    == current user, non-null DACL, current-user read access, allowlisted current-user/SYSTEM/local-
    Administrators ACEs only; inherited ACEs permitted only for that allowlist) and the
    handle-based reparse rejection, modelled on `validate_owner_private_dir_shape`
    (`src/daemon.rs:11088`, reparse check `:11112`) and `validate_owner_private_dir_security`
    (`:11121`). This is what makes a credential read outside the intent scope checkable on Windows
    (MF2-2).
  - Add `contained_under(root, path)` implementing decision 3's canonical-prefix containment check
    (canonicalize both, reject `..`, reject any symlink/reparse point on the resolved chain).
  - Add the shared process-identity primitives of decision 5 (SF2-6): `process_exe_path(pid)`
    (promoting `proc_pidpath` `src/daemon.rs:10703`, `QueryFullProcessImageNameW` `:11413`, and the
    Unix `server_executable` used by `verify_server_peer` `:10614`), `host_id()`, and `boot_id()`.
    All return `Result` and fail closed; `daemon::platform` re-exports them so `verify_server_peer`
    keeps calling the same code and `src/commands/copilot.rs` grows no parallel implementation.
  - Success (automated): `cargo test --workspace` passes unchanged daemon cap/socket tests; new unit
    tests cover reject-symlink, reject-oversize, reject-group-readable (Unix), reject-broad/foreign
    ACE and accept-current-user-plus-SYSTEM/Administrators ACEs (Windows, on a file *outside* any
    intent scope), reject-reparse-point
    (Windows), containment rejection for a path that escapes the root via link or `..`, and
    accept-happy-path on both platforms; `process_exe_path` / `host_id` / `boot_id` have
    round-trip and fail-closed tests; Windows tests are not `#[cfg(unix)]`-gated.
  - Success (manual): confirm on Windows that a hand-relaxed DACL on (a) the scope directory and
    (b) a credential file outside the scope both make the read path fail closed.

- [x] **Define the station-intent model and secure store** (`intent-model`)
  - Depends on: `platform-fs`.
  - New `src/station_intent.rs`: `StationIntentV1` (schema version, generation, timestamps, state,
    store key, session id, address, occupant/description/scope/tags, `delivery_mode`, `wake_on_cc`,
    `cc_watermark_ms`, `handler: HandlerDescriptorV1`, `producer: ProducerDescriptorV1` incl.
    `credential { kind, root_id, path, pointer, max_age_ms }`, `daemon_compat`, `singleton_hash`,
    evidence fields, unknown-field passthrough), `IntentRecoveryState` with the precedence of
    decision 16 (including `DeferredLease`), and an `IntentStore` bound to
    `<run_dir>/intents/<singleton_hash>` with `list`, `load`, `write_atomic`, `revoke`, `remove`,
    `gc`, and a persisted round-robin scan cursor keyed by sort position.
  - Enforce `STATION_INTENT_MAX_COUNT`, `STATION_INTENT_MAX_BYTES`, hashed `IntentId`, schema
    version range checks, unknown-field preservation, the decision-15 over-cap scan rules (never
    delete for being over cap; report `over_cap`; cursor still gives eventual coverage), and the
    `max_age_ms` clamp to `CREDENTIAL_MAX_AGE_MS_DEFAULT`.
  - No legacy synthesis: reading `.bindings.json` is not part of this module (decision 17).
  - Success (automated): unit tests for schema round-trip incl. unknown fields; version below/above
    range rejected; hashed id stability and collision-free derivation for adversarial
    address/store strings; atomic write leaves no partial file when interrupted before rename; cap
    and size limits produce typed errors; a scope seeded with `STATION_INTENT_MAX_COUNT + 88`
    entries still reaches every entry within `ceil(N / RECONCILE_MAX_CONCURRENCY)` cursor
    advances and deletes nothing; a credential descriptor whose `max_age_ms` exceeds the default is
    clamped; precedence function is exhaustively tested.
  - Success (manual): inspect a real intent file and confirm no secret material is present.

- [x] **Add the generic handler-kind registry and the shared argv builder** (`handler-registry`)
  - Depends on: `platform-fs`.
  - New `src/handler_kinds.rs` with `HandlerKindRegistry`, `HandlerKindId`, the registry of
    **producer roots** decision 3 requires (`root_id -> absolute root path`, registered at
    composition time by the harness layer), and the pure
    `build_push_argv(exe, selector, session_id, instance_id)`; unknown kinds and unregistered
    `root_id`s are rejected.
  - Refactor `bridge_handler_argv` (`src/commands/copilot.rs:288-306`) into a thin client-side
    wrapper over `build_push_argv` that resolves exe + `StoreSelector` from `ctx.cfg` and reads
    `instance_id` from the cap file (SF2-3, SF2-4). `--daemon-instance` is appended **only** inside
    `build_push_argv`, so attach and the daemon-side builder cannot diverge.
  - Add `daemon::store_selector_for_key` (decision 7) reusing
    `resolve_postgres_profile_for_store_key` (`src/daemon.rs:2184-2196`) for Postgres and the opened
    SQLite path otherwise, returning a typed error that maps to `Unverifiable` /
    `store_selector_unresolved`.
  - `src/commands/copilot.rs` registers `telex_copilot_push_v1` plus the Copilot bridge producer
    root; the daemon-side builder passes `canonical_current_exe()` (`src/daemon.rs:10340`) - the
    canonicalized path of the **currently running installed versioned binary**, not a stable
    launcher shim (decision 7).
  - Add strict `session_id` validation (charset, length) at descriptor load and at build.
  - Success (automated): unit tests prove an unregistered kind never builds argv; a descriptor with
    an injected `--backend`/`--db`/path parameter is rejected at load; a credential `root_id` that
    is not registered is rejected; `build_push_argv` output is byte-identical between the attach
    wrapper and the daemon-side builder for the same (exe, selector, session, instance), and always
    contains `--daemon-instance`; `store_selector_for_key` agrees with the client-side `ctx.cfg`
    resolution for the same store and returns the typed error for an unknown/ambiguous store key.
  - Success (manual): none required.

- [x] **Add the bridge probe protocol as a testable module** (`bridge-protocol`)
  - Depends on: none.
  - Extract a pure, SDK-free, exporting `copilot/bridge/probe-protocol.mjs` (request/response
    framing, nonce echo, constant-time secret comparison via `crypto.timingSafeEqual`, protocol
    fields, error codes), following the `busy-state.mjs` precedent; `extension.mjs` imports it and
    gains the `probe` handler plus `protocol: COPILOT_BRIDGE_PROTOCOL`, `bridge_generation`, and
    `start_time` fields in the registry it writes.
  - Probe responses expose only: echoed nonce, session id, protocol, bridge generation. No file
    paths, no busy diagnostics, no secret. Add a simple per-connection probe rate limit (CO-3).
  - Change `.github/workflows/ci.yml:28` from the single hardcoded path to
    `node --test copilot/bridge` so new JS tests actually run (MF-13).
  - Success (automated): `node --test copilot/bridge` runs both the existing
    busy-state tests and new probe-protocol tests (nonce echo, wrong secret, missing secret,
    unsupported protocol, oversized frame); a cross-language contract test asserts the literal
    op/field/error strings against their Rust counterparts, mirroring
    `copilot/bridge/busy-state.test.mjs:9-13`.
  - Success (manual): `node --check copilot/bridge/extension.mjs` still passes; a live Copilot
    session answers a probe after `extensions_reload`.

- [x] **Wire Copilot lifecycle to intents** (`copilot-intent`)
  - Depends on: `intent-model`, `handler-registry`, `bridge-protocol`.
  - Attach/resume: ensure the Copilot bridge producer root exists with correct owner-only
    permissions via `platform_fs::ensure_owner_private_dir` (so the Windows DACL exists at all -
    Node's `mode: 0o700` is a no-op there) **before** recording any credential path; write `pending`
    intent before `Register`; after `daemon_armed_push`, run the local probe, capture producer
    identity through the shared primitives `process_exe_path` / `capture_process_start_time` /
    `host_id` / `boot_id` (decision 5, SF2-6) and `cc_watermark_ms`, finalize to `live` with an
    incremented generation; roll back and remove the intent on any failure, including any identity
    primitive failing.
  - Attach registers its push handler with argv produced by the shared `build_push_argv`, sourcing
    `--daemon-instance` from `CapFile.instance_id` (`src/daemon.rs:286`) via `read_cap_file`
    (`:1731`) on the connection it already makes (`:1815`) - so attach-registered and
    reconcile-registered handlers carry the same fence flag (SF2-4).
  - Detach / failed provisioning / fallback downgrade / GC / last-binding teardown: durable tombstone
    first, then exact per-binding intent revocation - never whole-session, never touching other
    stores or addresses.
  - Client capability gate `ensure_reconcile_capability` (decision 11) applied to attach/resume and
    to Copilot-side recovery paths.
  - `.bindings.json` retained ref-count-only, written after the intent (decision 17).
  - Success (automated): `cargo test --workspace` unit tests over a temp run dir prove: exact
    per-binding revocation with two addresses and two stores; pending intent left by a simulated
    mid-attach crash is never `live`; capability gate refuses on a simulated minor-4 status;
    rollback removes the intent; the argv attach registers contains `--daemon-instance` and equals
    the daemon-side builder's argv; a credential path outside the registered producer root is
    refused at write time.
  - Success (manual): attach a real Copilot session and confirm exactly one `live` intent file with
    correct store/address/session/CC fields.

- [x] **Implement daemon-owned reconciliation** (`daemon-reconcile`)
  - Depends on: `intent-model`, `handler-registry`.
  - New `src/daemon_reconcile.rs` implementing decision 6 end to end: the two-level API
    (`reconcile_once` acquires the single-flight guard and the per-`MemberKey` `delivery_admission`;
    `reconcile_intent_locked` assumes the guard is held and never takes it - SF2-5), startup scan
    invoked from `serve()`, heartbeat-tick invocation via the `RECONCILE_TRIGGER` seam plus per-pass
    `ReconcileReport` publication (SF2-1), `Request::ReconcileIntents { proof, scope }` with admin
    proof, `register_member_reconciled` with unconditional tombstone-before-claim + post-claim
    re-check and **no reachable tombstone-clearing branch** (SF2-2), generation CAS, preserved retry
    state, preserved `cc_watermark_ms`, drain suppression, wave scheduling under
    `RECONCILE_PASS_BUDGET` / `RECONCILE_MAX_CONCURRENCY` / `RECONCILE_PER_INTENT_TIMEOUT` /
    `RECONCILE_PASS_DEADLINE`, the `IntentOutcome` classes with `DeferredLease` on a fixed
    `RECONCILE_DEFERRED_LEASE_RETRY` and backoff/jitter/quarantine only for `Failed` (MF2-1a),
    open-existing-only store resolution via `store_selector_for_key`, per-intent failure isolation,
    the cached `IntentIndex` (CO2-2), and a typed `ReconcileReport` carrying `over_cap` and
    `index_as_of_ms`.
  - Producer verification per decision 4, credential resolution per decision 3 (registered producer
    root, containment, per-file security check, `max_age_ms` -> `Unverifiable` with no secret read),
    deterministic ordering by `(store_key, address, generation desc)` with first-live-wins and no
    force-steal of any incumbent - reconciled or fresh (CO-2).
  - Emit reconcile events to the rotating NDJSON log and update intent evidence fields.
  - Success (automated): `cargo test --workspace` and
    `cargo test --all-features --test conformance --test daemon_core_postgres` cover: restore after
    restart; idempotent repeated passes with unchanged backoff state and no duplicate sweep;
    **discriminating tombstone tests** - (i) the reconcile path never calls the tombstone-clearing
    routine (assert the durable tombstone still exists, byte-identical, after a full pass over a
    tombstoned intent, and a compile/dispatch-level assertion that the clearing call is unreachable
    from `register_member_reconciled`), and (ii) a tombstone written between pre-check and claim is
    caught by the post-claim re-check and the lease is released; CC message committed during the gap
    is delivered; probe failure, pid-reuse, host/boot mismatch, insecure file, stale credential
    (`max_age_ms`), unresolvable store selector and legacy producer each map to the right state;
    drain suppression; `AlreadyOwned` before the stale cutoff produces `DeferredLease` with a fixed
    5 s next attempt, an unchanged failure counter, and recovery within
    `liveness_window_secs() + 10 s`; the inline call from `register_member` (decision 10) completes
    without deadlock under the held admission guard; and, under 600 synthetic intents against the
    512 cap, budget/deadline honored (no pass exceeds `RECONCILE_PASS_DEADLINE`), nothing deleted
    for being over cap, `over_cap` reported, and every intent attempted within the published
    `ceil(N / RECONCILE_MAX_CONCURRENCY)`-pass queue-delay formula.
  - Success (manual): kill a daemon with a live bridge, spawn a successor, and observe recovery
    within the published crash bound.

- [x] **Enforce anti-downgrade in the daemon** (`anti-downgrade`)
  - Depends on: `daemon-reconcile`, `copilot-intent`.
  - Implement decision 10 inside `register_member` - calling `reconcile_intent_locked`, never
    `reconcile_once` (SF2-5) - plus decision 13's pull-waiter scoping, plus the `NeedsAttachReason`
    additions, plus actionable client rendering in `wait`/`send` recovery.
  - Success (automated): tests prove a pull-only `Register` from an old-style client cannot create a
    non-push member over a live intent; the matched-version case of matrix row T25 (a pull-only
    `Register` from any client, on a daemon of the same version, over a live push intent) either
    reconciles to push or returns `ERROR_INCOMPATIBLE` / `PushIntentUnrecoverable`, never a pull-only
    member; the inline path does not deadlock; a live armed pull waiter still wins and the intent is
    deferred, not failed permanently; existing pull-wait reconnect tests remain green as negative
    controls.
  - Success (manual): none required.

- [x] **Add diagnostics, status projection and evidence** (`diagnostics`)
  - Depends on: `daemon-reconcile`, `anti-downgrade`.
  - Bump `PROTOCOL_MINOR` to 5; add intent rows (state + evidence per decision 18) to daemon and
    station status, including intent-only rows with no member; add the three issue-named conditions
    `live_intent_missing_member`, `member_missing_live_producer`, `intent_protocol_incompatible`;
    add turn-guard `push_intent_unrestored` warn-and-allow (decision 12) with the fail-open
    invariant restated; add the reconcile event log.
  - Success (automated): status tests assert intent-only projection, precedence, redaction (no
    secret, no raw argv), and that the guard warns rather than blocks; a guard regression test
    covers daemon-unavailable fail-open.
  - Success (manual): `telex station status --session <id>` shows a degraded intent-only row after a
    daemon kill.

- [x] **Drain, upgrade and rollback signaling** (`drain-upgrade`)
  - Depends on: `diagnostics`.
  - Compute the drain report from in-memory members plus the cached `IntentIndex` of decision 6
    only - no directory scan, no probe, no network I/O, evaluated before the lease-release loop in
    `drain_members` (`src/daemon.rs:3492`) - so it cannot push graceful drain past
    `--drain-timeout-ms` (SF-11, CO2-2); report `recoverable`, `degraded`, `incompatible`,
    `unknown`, `over_cap` counts plus `index_as_of_ms` through `daemon stop --drain`, `upgrade`, and
    `rollback`, with an explicit "unavailable" rendering when the daemon cannot be reached.
  - Add the bounded in-flight-handler wait to graceful drain (decision 8a).
  - Add the post-switch successor spawn and bounded reconcile verification to `perform_upgrade` and
    `rollback` (decision 14c), awaiting the next `ReconcileReport` on the trigger/report seam rather
    than polling (SF2-1), and the rollback warning of decision 11.
  - Success (automated): `tests/release_upgrade.rs` covers upgrade-with-live-intent recovery and
    rollback warning; a drain test asserts the report is produced within the timeout with 512
    intents present and that drain performs no intent-directory read (index-only assertion).
  - Success (manual): run `telex upgrade` on a host with one idle attached session and confirm push
    is restored without manual resume.

- [x] **Build the missing test harness** (`test-harness`)
  - Depends on: `bridge-protocol`, `intent-model`.
  - Add a Rust **fake producer endpoint** (named pipe on Windows, UDS on Unix) that speaks the probe
    protocol, with knobs for wrong secret, wrong nonce, wrong session, legacy protocol, hang, and
    abrupt death - the harness `tests/daemon_process_sqlite.rs` lacks today (MF-14).
  - Give `TestDaemon` a constructor that exercises the real startup path (config root + run dir +
    intent scan) so startup reconciliation is testable in-process (MF-15).
  - Expose the `RECONCILE_TRIGGER` pulse plus the per-pass `ReconcileReport` subscription as the
    test seam (SF2-1): tests fire a pass and await the next `pass_seq` instead of sleeping. There is
    **no** `TELEX_RECONCILE_INTERVAL_MS` env override, and `HEARTBEAT_INTERVAL` is not made
    env-overridable (it carries a test-enforced invariant, `src/daemon.rs:2370-2375`, `:7039`);
    bound assertions are computed from the constants plus observed per-pass timings and read
    `liveness_window_secs()` rather than literals.
  - Add owner-private credential-file fixtures (inside and outside a registered producer root, with
    correct and relaxed permissions, fresh and past-`max_age_ms` mtimes) so the decision 2/3 read
    rules are testable on both platforms.
  - Add frozen schema fixtures under `tests/fixtures/station_intent/v1/` for version-skew tests.
  - Extend the process-suite CI filter (`.github/workflows/ci.yml:72`) so the new tests, named with a
    `station_intent_` prefix, also run on macOS alongside `copilot_fallback` (SF-13).
  - Success (automated): the harness itself has a smoke test on all three platforms; a test proves
    the trigger seam drives a pass deterministically with no wall-clock sleep; the fake
    endpoint is verified to be rejected by `verify_server_peer` when it runs under a different
    executable identity than the intent records.
  - Success (manual): none required.

- [x] **Cover lifecycle, storage, bridge and delivery semantics with tests** (`tests`)
  - Depends on: `test-harness`, `anti-downgrade`, `diagnostics`, `drain-upgrade`.
  - Implement the full matrix in the Traceability section below, including the **negative controls**
    the review found missing: a test that fails if the reconciler never runs, and a test that fails
    if reconciliation resurrects a tombstoned station.
  - Include the cross-version pair (N intent/bridge with N+1 daemon, and the reverse), the
    matched-version anti-downgrade row (T25), the protocol-major separation test, the Postgres
    single-writer assertion, the cross-host negative (a foreign `host_id` intent is never restored),
    and the no-retry-storm assertion (backoff state unchanged across 3 reconcile passes, including
    across `DeferredLease` outcomes).
  - Success (automated): `cargo test --workspace`; `cargo test --no-default-features --features
    sqlite --test daemon_process_sqlite station_intent_`; `cargo test --all-features --test
    conformance --test daemon_core_postgres`; `node --test copilot/bridge` - all green.
  - Success (manual): none required.

- [x] **Document the revised lifecycle and operator contract** (`documentation`)
  - Depends on: `drain-upgrade`, `tests`.
  - `.paw/work/rb-2026-08-02a-106/Docs.md` as the as-built technical reference.
  - ADR 0050 per decision 21, and the ADR 0023 status amendment.
  - Update `docs/design/daemon.md` (new intent subsection under sec.5/sec.13.2/sec.14.3),
    `docs/design/ARCHITECTURE.md` sec.4/sec.9, `docs/design/copilot-bridge-push.md` (probe verb,
    protocol 2), `docs/guide/src/guides/operating.md` (both published recovery bounds **with their
    qualifications** - scope within one pass budget, intent not backed off or quarantined - plus the
    separate `ceil(N / RECONCILE_MAX_CONCURRENCY) * RECONCILE_INTERVAL` queue-delay formula for
    larger scopes, the `DeferredLease` waiting state, the TO-1/TO-2 scoping of the
    no-silent-downgrade guarantee, and the isolated-config-root guidance for destructive testing),
    and `docs/guide/src/guides/troubleshooting.md` (legacy-bridge path stays valid).
  - Update `docs/developing/releasing.md` and `tests/release_contract.rs` with the version axes:
    `STATION_INTENT_SCHEMA_VERSION`, `COPILOT_BRIDGE_PROTOCOL`, `PROTOCOL_MINOR`,
    `MIN_COMPATIBLE_PLUGIN_VERSION` (`src/commands/copilot.rs:74`, already surfaced in operator
    output at `:1641-1642`) - which this work leaves **unchanged**, because the plugin hook surface
    (`copilot/plugin/hooks.json`, `copilot/plugin/plugin.json`) is untouched and the new `probe`
    verb is a bridge-extension change gated by `COPILOT_BRIDGE_PROTOCOL`; the runbook records it as
    an asserted-unchanged axis so a future bridge/plugin contract change must revisit it (CO-4,
    CO2-3) - and the `.bindings.json` removal milestone. The unchanged-value assertion compares
    against a frozen previous-release contract fixture under `tests/fixtures/release/`, not against
    a second copy of the current constant.
  - Success (automated): `mdbook build docs/guide` succeeds; `tests/release_contract.rs` asserts the
    documented version axes match the constants, including that `MIN_COMPATIBLE_PLUGIN_VERSION` is
    unchanged from the previous release.
  - Success (manual): a reviewer can follow the operating guide to reproduce both published bounds
    within their stated qualifications.

- [x] **Run full verification and prepare reviewable commits** (`verification`)
  - Depends on: everything.
  - `cargo fmt --check`; `cargo clippy --workspace -- -D warnings`; `cargo test --workspace`;
    feature-matrix builds (`--no-default-features --features sqlite`, `--features postgres`,
    `--features entra`, `"sqlite,self-update"`); process suite; Postgres conformance when
    `TELEX_PG_URL` is available; `node --test copilot/bridge`.
  - Confirm the traceability tables below are fully satisfied, artifacts are consistent, and staging
    is selective.
  - Success (automated): all commands above exit zero (Postgres skips cleanly when unset).
  - Success (manual): final self-review of the diff against this plan.

## Traceability

### Acceptance criteria (issue #106) -> work items

| Acceptance criterion | Work items | Primary evidence |
|---|---|---|
| Auto-regains membership + `on_deliver` after same-major replacement, no manual resume | `daemon-reconcile`, `copilot-intent`, `drain-upgrade` | T1, T7 |
| Recovery within a documented bound after a successor exists | `daemon-reconcile`, `documentation` | T1, T7 bound assertions |
| Graceful drain gives an actionable pre/post-drain signal | `drain-upgrade`, `diagnostics` | T1, T22 |
| Multi-address / multi-store exact restore (store/mode/CC) | `intent-model`, `daemon-reconcile` | T13, T14 |
| Detached/tombstoned never auto-return | `copilot-intent`, `daemon-reconcile` | T11, T12 |
| Dead bridges / stale registries not restored | `daemon-reconcile` | T8, T9, T10 |
| Generic recovery cannot silently downgrade push to pull | `anti-downgrade` | T25, T18, T19, T6 |
| Lease epoch advances; old instance fenced | `daemon-reconcile` | T7, T21 |
| Queued messages durable and delivered at least once | `daemon-reconcile` | T3, T4 |
| Consumed / terminal never resurrected | `daemon-reconcile` | T5 |
| No two instances concurrently push as current owner | `daemon-reconcile`, `drain-upgrade` | T21, T24 |
| Incompatibility surfaced clearly and fails closed | `diagnostics`, `anti-downgrade` | T18, T19, T20 |
| Existing pull-wait reconnect behavior intact | `anti-downgrade` | T6 (negative control) |
| Isolated config-root guidance for destructive testing | `documentation` | doc review |

### Test matrix (issue #106) -> owning suite

| # | Scenario | Suite | Item |
|---|---|---|---|
| T1 | Graceful drain, live idle bridge, no backlog | process SQLite + fake endpoint | `tests` |
| T2 | Graceful drain while bridge busy/deferred | process SQLite (busy knob) | `tests` |
| T3 | Message committed during restart gap (incl. CC subset) | daemon core SQLite + Postgres | `tests` |
| T4 | Push accepted before drain but unacked | daemon core SQLite | `tests` |
| T5 | Ack/terminal disposition before restart | daemon core SQLite | `tests` |
| T6 | Blocked pull waiter (negative control) | daemon core SQLite | `tests` |
| T7 | Hard crash, bridge alive | process SQLite | `tests` |
| T8 | Hard crash, bridge dead | process SQLite | `tests` |
| T9 | Fresh registry, challenge fails | fake endpoint unit + process | `tests` |
| T10 | PID reused with different start time | daemon core SQLite | `tests` |
| T11 | Explicit detach / tombstone | daemon core SQLite | `tests` |
| T12 | Explicit resume after tombstone | daemon core SQLite | `tests` |
| T13 | Same session, multiple stations | daemon core SQLite | `tests` |
| T14 | Same session/address across stores | daemon core SQLite | `tests` |
| T15 | Competing sessions for one address | daemon core SQLite | `tests` |
| T16 | SQLite restart, no concurrent holders | process SQLite | `tests` |
| T17 | Postgres restart/handoff single-writer | `daemon_core_postgres` | `tests` |
| T18 | N bridge/intent with N+1 daemon | fixtures + capability gate | `tests` |
| T19 | N+1 intent with N daemon | fixtures + capability gate | `tests` |
| T20 | Protocol-major change | intent scope namespacing unit | `tests` |
| T21 | In-flight handler during drain | process SQLite + `--daemon-instance` guard | `tests` |
| T22 | Partial multi-store drain failure | daemon core SQLite | `tests` |
| T23 | No-disposition informational message, no retry storm | daemon core SQLite | `tests` |
| T24 | Repeated reconciliation idempotent | daemon core SQLite | `tests` |
| T25 | Matched-version anti-downgrade: pull-only `Register` over a live push intent on a same-version daemon reconciles to push or returns `PushIntentUnrecoverable`, never a pull-only member (CO2-4) | daemon core SQLite + inline `reconcile_intent_locked` | `tests` |
| T26 | Crash path `DeferredLease`: `AlreadyOwned` before the stale cutoff is deferred at a fixed 5 s retry, never backed off or quarantined, and recovery lands within `liveness_window_secs() + 10 s` (MF2-1a) | process SQLite | `tests` |
| T27 | Over-budget scope: 600 intents against the 512 cap - no pass exceeds `RECONCILE_PASS_DEADLINE`, nothing is deleted for being over cap, `over_cap` is reported, and every intent is attempted within the published queue-delay formula (MF2-1b, CO2-1) | daemon core SQLite | `tests` |
| T28 | Credential read security: file outside the registered producer root, relaxed Windows DACL, reparse point, and past-`max_age_ms` mtime each yield `Insecure`/`Unverifiable` with no secret read and no connection (MF2-2) | `platform-fs` unit + daemon core SQLite | `tests` |

**Coverage as built (corrected after the final review).** The table above is the *ownership* map;
this is what is actually asserted. Recorded honestly rather than left implying full coverage.

| Row | State |
|---|---|
| T1, T6, T9, T10, T11, T13, T14, T17, T18, T19, T20, T23, T24, T25, T27, T28 | covered |
| T15 | covered as of the final review (`station_intent_two_sessions_never_both_attend_one_address`) |
| T7, T8, T16, T26 | **partial** — the restart path is covered by graceful stop; no test kills the daemon, so the `AlreadyOwned` / `DeferredLease` crash path is exercised only at daemon-core level (`postgres_station_intent_restore_is_single_writer`, tightened in the final review to assert `deferred_lease == 1 && failed == 0`, no failure-counter advance, and the fixed cadence) |
| T2, T3, T4, T5, T12, T21, T22 | **not covered** — no test sends a message across a restart gap, drives a busy bridge, resumes after a tombstone, or fails one store of a multi-store drain. `T21` has the `--daemon-instance` fence half only. Carried as follow-up work; none of these is a *new* gap introduced by the fixes, and each is a test gap rather than a known defect |

### Review findings -> resolution

**Cycle 1** (MF-1..MF-17, SF-1..SF-18, CO-1..CO-5):

| Finding | Resolved by |
|---|---|
| MF-1 | Decisions 1, 2; `platform-fs`, `intent-model` |
| MF-2 | Decision 3; `daemon-reconcile` |
| MF-3 | Decision 4; `daemon-reconcile` |
| MF-4 | Decisions 4, 5; `copilot-intent` |
| MF-5 | Constants/bounds section; decisions 14, 6 |
| MF-6 | Decision 6 (retry preservation); T23, T24 |
| MF-7 | Decision 6 (tombstone ordering + re-check); T11 |
| MF-8 | Decision 6 (`cc_watermark_ms`); T3 |
| MF-9 | Decision 6 (single-flight, drain, budget, partial failure) + decision 5 (pending) |
| MF-10 | Decision 15 + decision 12 |
| MF-11 | Decision 10 |
| MF-12 | Decision 11; `drain-upgrade`, `bridge-protocol` |
| MF-13 | `bridge-protocol` (module extraction + CI glob) |
| MF-14 | `test-harness` (fake endpoint + contract test) |
| MF-15 | `test-harness` (TestDaemon startup path, trigger/report seam per SF2-1, fixtures) |
| MF-16 | This revision: all questions decided; see Resolved Questions |
| MF-17 | Decision 7 |
| SF-1 | Dependency graph, per-item success criteria, traceability tables |
| SF-2 | Constants table; module homes (`station_intent.rs`, `daemon_reconcile.rs`); heartbeat-tick reuse |
| SF-3 | Decision 18 |
| SF-4 | Decision 16 |
| SF-5 | Decision 17 |
| SF-6 | Decisions 3, 7 |
| SF-7 | Decisions 1, 16 |
| SF-8 | Decision 13 |
| SF-9 | Decision 12 |
| SF-10 | Decisions 8, 9 |
| SF-11 | `drain-upgrade` (in-memory `IntentIndex` evaluation before lease release; index defined in decision 6) |
| SF-12 | T2 plus decision 6 (deferred pushes are re-deferred, not pushed into a busy turn) |
| SF-13 | `test-harness` (macOS filter, fixtures, negative controls), `tests` |
| SF-14 | Decisions 11 (legacy is not failure), 12 (warn not block), 15 (TTL/GC); tombstone authority unchanged under same-user trust |
| SF-15 | Decision 6 (admin proof on `ReconcileIntents`) |
| SF-16 | Decisions 4, 22 (host/boot binding + cross-host negative test) |
| SF-17 | Decision 19 |
| SF-18 | Decision 21 |
| CO-1 | `RECONCILE_MAX_CONCURRENCY` + backoff jitter |
| CO-2 | Decision 6 deterministic ordering, no force-steal of any incumbent |
| CO-3 | `bridge-protocol` constant-time comparison, minimal probe response, rate limit |
| CO-4 | `documentation` (release runbook + `release_contract.rs` version axes, incl. `MIN_COMPATIBLE_PLUGIN_VERSION`) |
| CO-5 | Decision 15 hashed filenames and input bounds |

**Cycle 2** (MF2-1, MF2-2, SF2-1..SF2-6, CO2-1..CO2-4):

| Finding | Resolved by |
|---|---|
| MF2-1a backoff vs. crash bound | Decision 6 `IntentOutcome::DeferredLease` with fixed `RECONCILE_DEFERRED_LEASE_RETRY` (<= `RECONCILE_INTERVAL`, no exponential growth, no quarantine advance); crash-bound derivation rewritten in Constants and Published Bounds; T26 |
| MF2-1b budget/concurrency vs. bounds | `RECONCILE_PASS_DEADLINE` + wave scheduling (a pass can never overrun its tick); both bounds qualified to scopes `<= RECONCILE_PASS_BUDGET` and to intents not backed off or quarantined; separate deterministic `ceil(N / RECONCILE_MAX_CONCURRENCY) * RECONCILE_INTERVAL` queue-delay formula for larger scopes; T27 |
| MF2-2 credential read boundary | Decision 2 per-file both-platform rules incl. new Windows `validate_owner_private_file_security` + handle-based reparse rejection; decision 3 `root_id` registered producer root, canonical containment, and `max_age_ms` -> `Unverifiable` with no secret read; `platform-fs`, `copilot-intent`; T28 |
| SF2-1 cadence seam | Decision 6 "Cadence and triggers": `TELEX_RECONCILE_INTERVAL_MS` removed, replaced by the `RECONCILE_TRIGGER` pulse + per-pass `ReconcileReport` subscription; `HEARTBEAT_INTERVAL` untouched; `test-harness`, `drain-upgrade` |
| SF2-2 tombstone rationale | Decision 6 restated against the real code (`recovery`-gated pre/post checks at `src/daemon.rs:3962`, `:4022`, release at `:4026`; the hazard is the `recovery = false` clearing branch at `:4046`); `register_member_reconciled` checks unconditionally and cannot reach the clearing call; discriminating tests in `daemon-reconcile` |
| SF2-3 argv builder / exe / store selector | Decision 7: pure `build_push_argv(exe, selector, session_id, instance_id)`; `bridge_handler_argv` becomes a thin wrapper; `canonical_current_exe()` named as the currently running installed versioned binary (not a stable launcher); `daemon::store_selector_for_key` with `Unverifiable` / `store_selector_unresolved` failure; `handler-registry` |
| SF2-4 `--daemon-instance` ownership | Decisions 7, 8: the flag is appended only inside `build_push_argv`, so attach and reconcile cannot diverge; attach sources `instance_id` from `CapFile.instance_id` via `read_cap_file` on the connection it already makes; `handler-registry`, `copilot-intent` |
| SF2-5 reentrant admission guard | Decision 6 two-level API (`reconcile_once` acquires, `reconcile_intent_locked` assumes held); decision 10 calls the locked variant only; deadlock regression test in `anti-downgrade` / `daemon-reconcile` |
| SF2-6 identity primitives | Decision 5 names `platform_fs::process_exe_path` / `host_id` / `boot_id` (promoted from `daemon::platform`) alongside `capture_process_start_time`; `platform-fs`, `copilot-intent` |
| CO2-1 over-cap scan | Decision 15 "Over-cap scan behavior" (never delete for over-cap, bounded per-pass loads, sort-position cursor for eventual coverage, `over_cap` reporting); T27 |
| CO2-2 intent index | Decision 6 "Cached in-memory intent index" (`IntentIndex` in `DaemonState`, maintenance and invalidation rules, `index_as_of_ms`); `drain-upgrade` index-only assertion |
| CO2-3 `MIN_COMPATIBLE_PLUGIN_VERSION` | Constants table + `documentation`: added as the fourth release axis and recorded as unchanged, with the reason |
| CO2-4 anti-downgrade matrix row | T25 (matched-version case), referenced from the acceptance-criteria table |

### Trade-offs -> decisions

| Trade-off | Decision taken | Where |
|---|---|---|
| TO-1 fail-closed vs. today's degraded-but-working path | Scope fail-closed to *advertised-but-unverifiable* producers; pre-probe bridges are `legacy_producer`, keep the documented manual path, and never wedge a turn | Key Decisions 11, 12 |
| TO-2 push-intent authority vs. pull-waiter precedence | Preserve existing pull-waiter precedence; scope anti-downgrade to the no-live-waiter case; deferred (not permanent) loser | Key Decision 13 |
| TO-3 ADR 0039 boundary vs. typed descriptor | Generic descriptor-kind registries for both producer credential and handler; no ADR exception needed, boundary preserved and clarified in ADR 0050 | Key Decisions 3, 7, 21 |
| TO-4 manifest root | `<run_dir>/intents/<singleton_hash>/` - the hardened, authority-bearing root, namespaced to keep config-root isolation and protocol-major separation | Key Decision 1 |
| TO-5 one bound vs. two | Two published bounds, both derived from named constants and both explicitly qualified (scope within one pass budget; intent not backed off or quarantined), with a separate queue-delay formula for larger scopes; crash bound reads `liveness_window_secs()` | Constants and Published Bounds |
| TO-6 bridge wake signal | Declined; replaced by startup scan, heartbeat tick, upgrade/rollback successor spawn, and admin-proofed explicit reconcile from the existing hook | Key Decision 14 |

## Resolved Questions

The three questions `WorkShaping.md` previously left open are decided here and mirrored there; no
question remains open.

1. **Challenge framing, timeout, secret binding, negotiation** - decisions 3 and 4 plus the
   `BRIDGE_PROBE_TIMEOUT` / `BRIDGE_PROBE_MIN_PROTOCOL` constants.
2. **Synchronous startup scan vs. readiness-gated async reconciler** - the startup scan is
   asynchronous and non-blocking: `serve()` accepts connections immediately and spawns the first
   pass, which is budgeted and per-intent timed out, so a large or corrupt intent set cannot delay
   daemon readiness. Ordinary client recovery is not gated on it because decision 10 puts the
   anti-downgrade guard inside `register_member`, where it applies whether or not the scan has run.
3. **Bridge wake signal** - declined, with the alternative triggers named in decision 14. The
   council's preserved dissent is recorded there and in `WorkShaping.md`.

## Open Questions

None.

## Implementation Deviations (as built)

Four direction-preserving deviations, each recorded with the reason. Full detail in `Docs.md`.

1. **Producer roots are validated, not rewritten (Windows).** The plan specified
   `ensure_owner_private_dir` for the Copilot bridge root. On Windows that applies a *protected*
   DACL, which re-propagates inheritance and leaves pre-existing children with an empty DACL —
   unreadable even to the process that wrote them, which would break the bridge itself
   (reproduced: `read` on a pre-existing `.bindings.json` failed with `Access is denied` immediately
   after hardening). Replaced by `ensure_owner_private_producer_root`: create-strict for a directory
   telex creates, validate-never-rewrite for one that already exists, with an ACE allowlist of
   current user / `SYSTEM` / local `Administrators` / logon-session SID. Posture unchanged (a
   broadly-ACLed root still fails closed); per-file credential checks still apply independently.
   Regression test:
   `platform_fs::tests::producer_root_hardening_never_strips_an_existing_producer_file`.
   **Pivot (final review):** the allowlist as first written also accepted the broad AppContainer
   groups `S-1-15-2-*` and capability SIDs `S-1-15-3-*`. Those prefixes existed only in a
   `#[cfg(test)]` SDDL helper before this work; promoting them into the *enforced* validator
   widened the two validate-only paths this feature added (the producer credential read and an
   existing bridge root) to accept `ALL APPLICATION PACKAGES`, a group comparable in reach to
   `Users`. They are removed.
2. **Intent finalization also happens at the turn boundary.** On a *first* attach the bridge
   extension is written but not yet loaded, so there is no producer to probe or describe and attach
   alone cannot finalize the very first binding. `ProducerDescriptorV1::validate` therefore requires
   concrete producer identity only when the state is not `Pending` (safe: a `Pending` intent is never
   reconciled), and the `agentStop` drain hook — already the explicit reconcile trigger of decision
   14(d) — finalizes pending intents once the bridge answers. Recovery is armed within one turn of
   `extensions_reload`, with no new command and no new lifecycle.
3. **`--daemon-instance` requires connecting before provisioning.** `provision_bridge` builds argv
   before `attach::run` connects, so on a cold start there was no cap file yet. `daemon_instance_id`
   now calls `connect_or_spawn` first. No new lifecycle — attach connects on the very next step
   regardless — it just moves the connect one step earlier so attach-registered and
   reconcile-registered argv stay byte-identical.
4. **Self-owned lease adoption.** `AlreadyOwned` where the owner *is this daemon instance* is
   neither `DeferredLease` nor `Failed`: both would wedge a binding whose member was lost from
   memory while the lease is still held. The reconciler adopts the lease it already holds. No new
   claim, no steal, and the post-claim tombstone re-check still runs.
   **Pivot (final review):** adoption is now conditional on no *other session in this daemon*
   holding a member for the address (idle or not). "We already own it" is a statement about the
   daemon, not the session, and unconditional adoption let two sessions' intents for one address
   both end up with an armed member.

### Pivots from the final adversarial review

Behavioral changes to what the plan specified, recorded here because each changes a stated
property. The full list of 40 fixes with mechanisms and tests is in `Docs.md`, "Corrections made
during the final adversarial review".

5. **GC is state-scoped and TTL-governed on every reason.** Decision 15 described GC reasons
   without saying which states each applies to, and as built the credential-existence and
   identity rules applied to `Pending` too. On a first attach that deletes the record deviation 2
   exists to promote, roughly 60 s after the attach. `Pending` is now governed by
   `STATION_INTENT_PENDING_TTL` and by nothing else, a missing credential needs a new
   `STATION_INTENT_CREDENTIAL_MISSING_TTL` (15 min, measured from the durable `evidence` clock,
   not manifest age), and an unsupported schema version is never deleted at all — which is what
   makes the documented "a rollback never deletes intents" guarantee true rather than
   time-limited.
6. **Retry policy is decided separately from projected state.** Decision 3 said a `max_age_ms`
   expiry is "backoff-eligible", but as built every credential condition returned `Terminal`,
   which carries the one-hour quarantine cadence. All six transient credential conditions now
   return `Failed` while still projecting `Unverifiable`; `RegistryError` gained
   `ContainmentUnreadable` so an absent file during a bridge reload is no longer a *security*
   verdict.
7. **Durable evidence is authoritative for retry state.** The plan treated `evidence` as
   diagnostics. It is now read back: the index seeds backoff, quarantine, and attempt counts from
   the manifest on first sight, so a crash loop cannot reset them by replacing the daemon. In
   exchange, evidence is written only when it changes or once per `EVIDENCE_REFRESH_INTERVAL`
   (60 s), and no longer bumps `updated_at_ms` — which also gives GC an age clock that an ordinary
   retry cannot reset.
8. **The CC watermark is refreshed, not only passed through.** Decision 7 required pass-through so
   gap-committed CC messages stay visible. Pass-through alone meant the durable value never moved
   after finalize, so every replacement replayed the whole session's CC history. The durable value
   is now refreshed from the live member on a successful outcome, and every restore takes
   `max(member, manifest)` so a live member is never rewound.
9. **`station reset` withdraws the intent.** The plan did not consider reset. It is the one
   deliberate operator action with no durable marker, so the reconciler re-armed it within a tick.
   `reset_station` now revokes the affected intents, and the reconciler treats an
   `idle && !idle_rearmable` member as a revocation.
10. **Trigger (c) invokes the successor binary rather than `connect_or_spawn`.** Decision 14(c)
    said "upgrade/rollback spawning the successor they installed". `connect_or_spawn` spawns
    `current_exe()`, which during an upgrade is the *pre-switch* binary — and because the daemon's
    peer check requires a matching executable, that left the old binary serving and locked every
    new-binary client out. A new `telex daemon reconcile` subcommand is invoked on the
    newly-selected binary instead; it retries a draining predecessor and a pass that did not run.
    `ReconcileReport` gained `ran` / `skipped_reason` so those are distinguishable.
11. **`Pending` is not `recoverable`.** The drain report folded it in, contradicting
    `IntentRecoveryState::is_recoverable` and `is_reconcilable`. It has its own counter, and
    rejected-but-identifiable manifests are indexed so status and the drain report are not blind to
    them (`DrainIntentReport::unidentifiable` counts the rest).
12. **Windows `boot_id` is minted and persisted, not derived.** The plan assumed a stable boot
    identifier existed on every platform. On Windows the derivation
    (`SystemTime::now() - GetTickCount64()`) is not stable within one boot, and an exact-equality
    comparison across two processes made every intent fail closed. The identifier is now minted
    once per boot, persisted in `HKCU\Software\telex` (environment-independent, unlike any file
    path), validated against monotonic uptime and a tolerant boot instant, and cached per process.
13. **The release-contract fixture declares expected movement.** The runbook step "roll the fixture
    forward" could not be executed without turning CI red, because the test hardcoded both sides of
    every comparison. The fixture now carries `expected_movement` and the test asserts that
    relationship.

Minor, non-behavioral:

- The plan's `--drain-timeout-ms` flag does not exist in this codebase; the bounded in-flight
  handler wait uses a named constant (`DRAIN_INFLIGHT_WAIT`, 5 s) rather than inventing a flag.
- CI runs `node --test "copilot/bridge/*.test.mjs"` instead of the directory form, which does not
  resolve on Windows runners. Same requirement satisfied: every `*.test.mjs` under `copilot/bridge`
  runs on both platforms.
- SHA-256 is implemented in `platform_fs` rather than taken from `sha2`, which is only in the
  dependency graph behind the optional `self-update` feature. Intent identity must be byte-identical
  in every feature combination, including `--no-default-features --features sqlite`.