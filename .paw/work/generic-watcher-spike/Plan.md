# Generic Watcher Spike Plan

Revision: 5

## Outcome

Build and exercise an experimental, separately runnable `telex-watcher`
application that persists trusted local detector registrations, executes them
outside agent sessions, and emits only normalized Telex messages. The node will
demonstrate generic GitHub, repository-customized GitHub, and live Azure DevOps
detectors, restart recovery, a real Copilot bridge wakeup, and durable
unoccupied-address queueing without a session-owned waiter or Loop task.

Fixture-backed Azure DevOps execution is sufficient for deterministic
implementation tests, but it does not satisfy the live distinct-provider
completion criterion. If reachable ADO coordinates and credentials cannot be
obtained, the implementer will send `decision-needed` and hold rather than
claiming the node complete.

## Scope Guardrails

- The runtime is provider-neutral. GitHub and Azure DevOps semantics remain in
  editable detector scripts.
- The runtime's only trigger reaction is a normalized Telex send.
- Registration is local-only and detector commands are trusted same-user code.
- Target, sender, cadence, timeout, attention, disposition requirement, working
  directory, and environment allowlist are Watcher policy.
- Detector output cannot reroute, impersonate, schedule, launch an agent, or
  request a post-trigger action.
- The Telex integration is explicitly experimental and does not freeze the
  issue #12 Application Client contract.

## Approach

### 1. Add a narrow experimental application

- Add `telex-watcher/` as a Rust workspace member with its own binary. This
  keeps the runnable dogfood application under workspace build/test/clippy
  coverage while leaving Telex core and `telex-console` unchanged.
- Use a dedicated SQLite registry below the user's Telex data directory.
- Hold an advisory `fs2` file lock while `run` is active so one runtime
  schedules a registry at a time while management commands remain available.
- Create one explicit registry schema version for the spike and fail clearly if
  an unknown version is opened. General migration, backup, retention, and
  compaction administration are deferred and recorded in the report.

Persist each watch's:

- stable watch ID, command argv, canonical script path, and working directory;
- `pinned` or `follow-path` script mode and pinned digest when applicable;
- immutable sender and target;
- cadence, timeout, attention, disposition requirement, and environment
  allowlist;
- opaque parameters and current opaque state;
- lifecycle status, next due time, failure count, and last diagnostic.

Persist bounded attempts and accepted-event provenance sufficient to explain:

- which script digest ran;
- which prior state was supplied;
- which result was returned;
- which event ID and normalized envelope were accepted;
- which sender/target and Telex receipt were used;
- whether state committed after acceptance.

Removed watches retain their state and provenance in the spike registry.
Long-term pruning and archival policy are deferred.

### 2. Define detector protocol version 1

The Watcher sends one JSON request on stdin:

```json
{
  "schemaVersion": 1,
  "attempt": {
    "id": "attempt-id",
    "now": "2026-07-19T00:00:00Z"
  },
  "watch": {
    "id": "stable-watch-id",
    "parameters": {}
  },
  "script": {
    "mode": "pinned",
    "sha256": "..."
  },
  "state": {}
}
```

The detector must exit successfully and emit exactly one bounded JSON result:

- `idle`: successful observation with optional `nextState`;
- `event`: one required event plus optional `nextState`;
- `terminal`: optional final event plus optional `nextState`;
- `degraded`: no state advancement; retry under Watcher backoff policy.

An event contains only:

- stable event ID;
- namespaced kind;
- subject and body;
- opaque provider metadata.

Protocol version 1 limits detector stdout to 256 KiB, stderr to 64 KiB,
serialized opaque state to 256 KiB, subject to 512 UTF-8 bytes, body to
128 KiB, and metadata to 64 KiB. Cap failures are visible attempt failures;
truncated events are never sent.

Process exit status represents command execution success/failure. Structured
JSON represents detector state.

### 3. Enforce fail-safe state and duplicate ordering

- `idle`: atomically commit validated `nextState`, reset failure backoff, and
  schedule the next run.
- New `event`: send to the fixed sender/target first. Only after a durable
  `delivered` or `queued-unoccupied` receipt atomically commit `nextState`, the
  sent-event ledger row, and the next schedule.
- `terminal` with an event: use the same send-before-state transaction and mark
  the watch terminal in the local commit.
- `terminal` without an event: commit state and terminal status directly.
- `degraded`, timeout, non-zero exit, malformed/oversize output, digest
  mismatch, or Telex failure: retain prior detector state, record the attempt,
  and apply bounded backoff.

Each committed event row binds:

- `watch_id` and `event_id`;
- prior-state hash and committed next-state hash;
- normalized event-envelope hash;
- script digest;
- sender, target, Telex message ID, and receipt.

An existing `(watch_id, event_id)` never authorizes a new state transition:

- if the normalized event-envelope hash matches the committed ledger row, treat
  the repeated ID as a visible `stale-duplicate` no-op even when the watch state
  has advanced through later events; do not send and do not commit the
  detector's newly proposed `nextState`;
- if the same ID now has different event evidence, fail the attempt as
  `duplicate-event-conflict`; do not send and do not advance state. Record both
  hashes for diagnosis, but do not increment provider-failure backoff.

Because ledger and state commit atomically, a committed ledger row proves the
original transition, not any later proposal. If Telex accepts a message and the
Watcher crashes before the local transaction, no ledger row exists and the
stable event ID may be sent again after restart. The report will preserve this
explicit at-least-once window.

### 4. Bound scheduling and trusted process execution

- Schedule active watches from persisted due times and execute an overdue watch
  once on restart before resuming normal cadence.
- Enforce one in-flight attempt per watch and a configurable global concurrency
  limit.
- Spawn each detector in a `command-group` process group/Windows Job Object.
  Timeout and graceful Watcher shutdown terminate the full descendant tree.
- Read stdout and stderr concurrently with hard byte limits.
- Apply bounded exponential retry backoff for execution failures and degraded
  results.
- Hash the script immediately before execution and again before accepting the
  result. A follow-path change during execution invalidates the attempt.
- Clear the child environment, restore the minimal platform launch baseline,
  and copy only registration-approved inherited variables. Values are read at
  execution time and never persisted. Exact values of explicitly sensitive
  allowlisted variables (`GH_TOKEN`, `AZURE_DEVOPS_EXT_PAT`, and names ending
  in `TOKEN`, `PAT`, `KEY`, or `SECRET`) are redacted from bounded stderr; the
  runtime never logs the constructed child environment.
- Handle Ctrl+C/SIGINT/SIGTERM by stopping new scheduling, allowing a short
  drain, and terminating remaining process groups.

Abrupt Watcher death can leave Unix descendants beyond the dead process's
control. The spike records interrupted attempts and delays their next run by at
least the configured timeout plus failure backoff, but production-grade orphan
adoption/termination is deferred to operational hardening and called out as a
known limitation.

After restart, overdue watches receive bounded 0-10% cadence jitter so a group
of recovered watches does not all invoke providers on the same scheduler tick.

### 5. Use a narrow experimental Telex adapter

- Define an internal `TelexAdapter` plus a serialized lifecycle coordinator for
  sender attachment, strict send, reconciliation, and detach.
- Implement the spike adapter by invoking a configured `telex` executable.
- Add a narrow experimental `telex send --require-attached` mode. It preserves
  normal CLI behavior by default, but for Watcher traffic it returns
  `NeedsAttachReason::RestartLost` or
  `NeedsAttachReason::DeliberatelyDetached` without running the current unbound
  `register_for_retry` path.
  This prevents daemon restart from replacing PID-bound membership with a
  registration that has no watched process.
- Generate one cryptographically random, never-persisted runtime session UUID
  on every Watcher OS-process start. One runtime session spans every configured
  sender address. Reuse that UUID only while the same Watcher process remains
  alive, including across Telex daemon restart; every new Watcher process gets a
  new UUID.
- Capture the actual Watcher PID. Explicitly attach every distinct sender before
  any watch using it becomes eligible:

  ```text
  telex --address <sender> attach
    --session <runtime-uuid>
    --no-session-bind
    --watch-pid <watcher-pid>:required
    --occupant watcher:<watcher-pid>
  ```

  `required` plus Telex's captured process start time makes Watcher death a
  negative liveness signal for every sender in the runtime session.
- Startup normalizes/deduplicates the sender set and remains non-ready for sends
  until all required senders attach. Partial attachment is visible and
  retried at 1, 2, 4, 8, and 15 seconds (30-second ceiling); exhaustion leaves
  the runtime `degraded-not-ready` and forbids detector sends. The Watcher never
  silently operates with a partially attached identity set.
- Sender normalization trims surrounding whitespace, rejects empty values, and
  otherwise preserves the exact Telex address string without case-folding.
- After every attach/reconcile, query station status for the runtime UUID and
  fail readiness if any desired sender is absent, attached under another
  session, has empty `watch_pids`, or does not contain the actual Watcher PID
  with role `required`.
- Reconcile configured senders under one lifecycle lock:
  - adding a watch with a new sender attaches that sender before activation;
  - every scheduler cycle reads the registry revision and recomputes the sender
    set from all non-removed watches, including paused watches;
  - pausing/resuming therefore does not detach a still-configured sender;
  - removing the last non-removed watch for a sender detaches that address;
  - another active or paused watch sharing the sender keeps membership attached;
  - re-adding a detached sender explicitly attaches it and clears only the
    current runtime session/address tombstone.
- Invoke strict send with explicit runtime session and fixed sender. On
  `NeedsAttach` caused by daemon membership loss, reconcile all currently
  configured senders with the same UUID/PID, then retry the send exactly once.
  A deliberate-detach reason does not hidden-recover during shutdown.
- Run a bounded periodic sender reconciliation so daemon restart is repaired
  even before the next event. This is a Watcher-owned local lifecycle check, not
  a Telex waiter and not an agent-session task. The timer runs every five
  seconds when healthy and backs off to at most 30 seconds after failures.
  Timer and send-triggered reconciliation share one lock/generation so they
  cannot stampede or race shutdown.
- Normalize metadata with protocol version, watch ID, event ID, attempt ID,
  script digest/mode, and nested detector metadata.
- Accept only `delivered` and `queued-unoccupied`; unknown receipt vocabulary
  fails closed.
- Distinguish daemon unavailable, residual `NeedsAttach`, IPC/auth rejection,
  command failure, malformed JSON, unknown receipt, and durable acceptance in
  attempts and inspection output.
- Graceful shutdown first blocks new scheduling/reattachment, drains bounded
  sends, and detaches every sender under the runtime UUID. Record every
  per-address outcome and retry transient detach failures up to three times.
  Unresolved detach failures make shutdown degraded but do not reuse the UUID.
  Detach tombstones remain attributable to that discarded UUID; the next
  process's fresh UUID is not blocked.
- Abrupt Watcher death assumes no cleanup. Within a bounded daemon heartbeat
  allowance, the dead/reused required PID ends the whole runtime session,
  releases every sender lease, and marks each sender idle/unoccupied. A new
  Watcher UUID retries boundedly until it can claim the stable sender addresses;
  no force takeover/reset is used. If takeover still fails after 30 seconds,
  startup remains non-ready and surfaces a blocker instead of operating
  partially.
- Inbound messages or replies to a Watcher sender are not consumed in #101.
  While the Watcher is down they remain durable and report
  `queued-unoccupied`. While its sender-only station is attached, a receipt may
  say `delivered` because the address is occupied, but no receive/ack loop exists
  and the Watcher must not claim application consumption.

The strict-send flag, CLI subprocess lifecycle, sender-only occupancy semantics,
and all session/attachment/receipt assumptions are temporary #12 evidence.
Tests use a fake adapter where appropriate. The report will not present this
shape as a public Application Client contract. If `--require-attached` cannot
remain a narrow backward-compatible CLI change, reopen the mechanism choice and
use the council's fallback: a spike-private `Register`/`Send`/`Detach` IPC
adapter with the same lifecycle semantics.

### 6. Provide local management and diagnostics

Implement:

- `add --file <watch.json>`;
- `list`;
- `show <watch-id>`;
- `pause <watch-id>`;
- `resume <watch-id>`;
- `update <watch-id> --file <watch.json>`;
- `remove <watch-id>`;
- `attempts <watch-id> [--limit <n>]`;
- `events <watch-id> [--limit <n>]`;
- `run`, with bounded `--once` and watch-selection options for tests/exercises.

Add/update validates local paths, argv, JSON, timing bounds, environment variable
names, script mode/digest, and Telex addresses. Sender and target cannot be
changed by update; rerouting requires a new watch ID.

`list`, `show`, `attempts`, and `events` expose enough structured JSON-capable
diagnostics to correlate watch, attempt, event, script digest, state hashes,
receipt, and failure/backoff without reading SQLite directly.

The `run` process writes scheduler/lock/adapter/shutdown lifecycle records as
JSONL on stderr so multi-day dogfood can redirect and tail a simple run log.
Lifecycle records include `runtime_session_id`, `watcher_pid`, sender,
desired/attached sender sets, reconcile generation, lease epoch when available,
typed result/error, retry count, and detach outcome. Production rotation remains
deferred.

### 7. Add editable detector examples

- Generic GitHub PR detector using `gh`, adapting useful review/check/merge
  decisions from the Loop reference without its worker/waiter lifecycle.
- Customized GitHub detector demonstrating repository-specific author/comment
  filtering without runtime changes.
- Azure DevOps PR detector using the same protocol against REST 7.1 data.
- Fixture-capable provider inputs for deterministic tests.
- A non-PR HTTP/JSON or file-condition detector plus sample registration
  prepared explicitly for later `viability-gate` builder dogfood.

The non-PR scenario is a handoff from #101; this node prepares and validates the
template contract but does not count it as one of its required live provider
proofs. Readiness means the template executes through the real runtime against a
local fixture, produces a protocol-valid `idle` or `event` result, and passes the
same state/provenance assertions as provider templates; live Telex delivery is
left to `viability-gate`.

Azure DevOps proof policy:

- A checked-in ADO response fixture validates parser and protocol behavior.
- Live proof requires reachable organization/project/repository/PR coordinates
  and an explicitly allowlisted credential path such as
  `AZURE_DEVOPS_EXT_PAT`.
- If live access is unavailable, send `decision-needed` to the workstream and
  campaign orchestrators and stop before claiming completion.
- Approval of this plan does not waive live ADO.

### 8. Validate behavior and run the live spike

Automated validation will cover:

- protocol parsing, caps, and policy-field exclusion;
- idle state advancement;
- state unchanged after failed Telex send;
- state plus ledger commit after `delivered` and `queued-unoccupied`;
- duplicate event ID no-op/conflict behavior with no state advancement;
- terminal event ordering and crash/retry window;
- restart recovery from committed opaque state;
- timeout, full process-group termination, non-overlap, malformed output,
  oversize output, non-zero exit, degraded backoff, pinned mismatch, and
  follow-path drift;
- daemon restart and `NeedsAttach` recovery;
- two Watcher OS-process starts producing different runtime UUIDs, while one
  live process retains the same UUID across Telex daemon restart;
- two distinct senders attached under one fresh runtime UUID with the actual
  Watcher PID recorded as `required` for both;
- station-status assertions rejecting any Watcher-owned desired sender with
  empty `watch_pids`, a non-`required` predicate, another session, or the wrong
  PID/start time;
- strict send never invoking unbound auto-registration, including a forced
  daemon-loss race between attach and send;
- paired compatibility coverage showing ordinary unflagged `telex send`
  retains its existing auto-recovery behavior;
- every Watcher attach passing `--no-session-bind`, including a test with
  ambient `TELEX_SESSION_PID` set that proves no extra anchor is registered;
- every send passing explicit `--session` and `--from`, plus an ambiguity test
  that omitting `from` with multiple senders is refused;
- add/pause/resume/remove reconciliation for watches sharing sender addresses;
- partial multi-sender attach remaining non-ready and converging or failing
  boundedly;
- graceful detach followed by a new-runtime UUID reattaching the stable senders;
- abrupt Watcher termination releasing all sender leases within two daemon
  heartbeat intervals, followed by fresh-runtime takeover without force reset;
- inbound message and reply queueing to a sender while the Watcher is down,
  without claiming consumption;
- inbound traffic while a sender-only station is attached, proving that a
  `delivered` occupancy receipt still leaves the message unconsumed without a
  receive/ack loop;
- shutdown racing `NeedsAttach` without reattaching after detach begins;
- management and diagnostic commands;
- generic GitHub, customized GitHub, ADO fixture, and non-PR template behavior.

Live sequence:

1. Build the local Telex and Watcher binaries.
2. Preflight GitHub and live ADO coordinates/credentials. Send
   `decision-needed` and hold if live ADO is unavailable. Preflight must also
   identify a controllable PR comment, reviewer-state, or policy/check action
   that can be caused during the exercise; record the exact trigger action for
   reproducibility.
3. Register and run the first generic GitHub and live Azure DevOps watches
   through a fully detached Watcher process with no session waiter.
4. After the first end-to-end GitHub and ADO events, send their evidence to the
   Telex Watcher workstream orchestrator as a disposition-required
   `provider-proof-checkpoint` and stop before broader dogfooding.
5. Resolve any feedback and wait for the workstream orchestrator to approve
   continuing. Automated tests and non-provider implementation may continue
   while evidence is under review, but no broader live dogfooding proceeds. If
   either provider event or the checkpoint disposition remains unavailable
   after four hours of active runtime/review, send `decision-needed` with
   partial evidence and hold.
6. Exercise the repository-customized GitHub policy without runtime changes.
7. Prove both an actual Copilot bridge wakeup and a
   `queued-unoccupied` receipt to a pre-created unoccupied address.
8. Stop/restart the Watcher and verify committed state and event IDs are not
   replayed; separately demonstrate the controlled send-accepted/local-commit
   duplicate window.
9. Exercise at least two configured sender addresses:
   1. capture the initial runtime UUID, PID-bound status, and lease epochs;
   2. restart the Telex daemon and prove all senders reconcile under the same
      runtime UUID/PID;
   3. gracefully detach, restart, and prove a different UUID claims both;
   4. abruptly terminate that Watcher and verify both senders become
      idle/unoccupied within two observed daemon heartbeat intervals;
   5. start another fresh UUID and claim both without force takeover.
10. Send a new message and a reply to a Watcher sender during downtime, then
    verify durable queueing without a false consumption claim.
11. Send inbound traffic while the sender-only Watcher station is attached and
    prove `delivered` means occupied rather than consumed.
12. Exercise timeout, overlap prevention, malformed/oversize output, script
   drift, execution failure, degraded backoff, and daemon restart.
13. Stop only the exact proof process after evidence is captured.

### 9. Produce the spike report

Create `docs/generic-watcher-spike-report.md` covering:

- provider scenarios, first-event checkpoint, and evidence;
- detector protocol and duplicate/state/send ordering;
- GitHub/customized-GitHub/live-ADO results;
- prepared non-PR viability-gate handoff;
- wakeup and durable-queue evidence;
- restart, timeout, overlap, malformed output, and failure behavior;
- stable watch/event/script/state/receipt provenance;
- detector-authoring experience;
- trusted-local execution, environment, credential, and diagnostic observations;
- every temporary integration shortcut;
- experimental service-station identity, multi-sender attach/reconcile/detach,
  required-PID liveness, graceful/abrupt restart, sender-only inbound queueing,
  and any measured race or CLI limitation;
- known defects and incomplete validation;
- discrete deferred production items;
- issue #12 requirements for lifecycle/recovery, push/poll, service identity,
  send/receive/reply/disposition, cursor/restart, provenance/metadata, and
  IPC/binding ergonomics, including:
  - stable service address versus ephemeral never-reused process session;
  - typed PID/start-time liveness;
  - atomic or visibly partial multi-address attach/reconcile/detach;
  - typed `NeedsAttach` reasons and caller-selected recovery policy;
  - explicit sender selection for multi-address applications;
  - bounded reconcile-and-send without raw CLI error parsing;
  - collision/takeover policy with no hidden force reset;
  - receipt occupancy versus durable acceptance versus actual consumption;
  - sender-only versus bidirectional application-station semantics;
  - message/event-ID deduplication guidance across accepted-send/local-commit
    crash windows;
  - daemon-restart notification or ergonomic explicit reattachment without a
    resident waiter or timer-only discovery;
  - status/audit for session, PID predicate, lease epoch, idle state, and last
    reconcile.

Explicit deferred items include:

- general registry migrations and backup/restore tooling;
- retention, export, purge, and compaction administration;
- production log rotation and service supervision;
- suspend/clock-jump and rate-limit hardening;
- abrupt-crash orphan adoption/termination;
- automatic degradation notifications;
- production packaging, sandboxing, and remote administration.

## Work Items

1. **Runtime and protocol:** implement the narrow crate, registry, detector
   contract, bounded scheduler/execution, receipt-gated transaction, Telex
   adapter, and required management CLI.
2. **Behavioral tests:** prove state ordering, duplicate safety, recovery,
   provenance, CLI behavior, and bounded failure cases.
3. **Detector examples:** add generic GitHub, customized GitHub, Azure DevOps,
   and non-PR handoff templates plus deterministic fixtures.
4. **Live proof and report:** run the detached provider/checkpoint/restart/
   failure exercise and write the report.

These items are sequenced because tests/templates depend on the protocol and the
report depends on executed evidence.

## Completion Criteria

- The application and workspace pass formatting, build, tests, and clippy.
- Required management operations work against a persistent registry.
- Generic GitHub, customized GitHub, and live Azure DevOps detectors execute
  through the same provider-neutral runtime.
- The workstream orchestrator approves the first GitHub/ADO event checkpoint
  before broader dogfooding continues.
- Duplicate event IDs never advance state solely because a ledger row exists.
- Event-producing state is unchanged on send failure and commits only after a
  durable Telex receipt.
- Every configured sender is explicitly attached before use under one fresh
  per-process runtime UUID with the actual Watcher PID as a required liveness
  predicate; strict send never creates unbound membership.
- Daemon restart restores all sender registrations under the same live runtime
  identity, graceful stop detaches all senders, and abrupt Watcher death makes
  them idle/unoccupied before a fresh runtime UUID takes over without force.
- Messages/replies addressed to sender-only stations during downtime are shown
  durably queued and are not reported as consumed by Watcher.
- Restart, timeout, non-overlap, malformed/oversize output, execution failure,
  degraded output, and script provenance are demonstrated.
- A detached Watcher produces both a real Copilot bridge wakeup and durable
  unoccupied queueing with no originating-session waiter.
- The non-PR scenario is ready for `viability-gate`.
- The report honestly identifies the at-least-once window, temporary Telex
  integration, issue #12 requirements, risks, and deferred production work.
