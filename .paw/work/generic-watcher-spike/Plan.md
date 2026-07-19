# Generic Watcher Spike Plan

Revision: 4

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

- Define an internal `TelexAdapter` interface returning a classified receipt.
- Implement the spike adapter by invoking a configured `telex` executable.
- Give the application a stable service session identity derived from the
  registry and pass an explicit session and sender on every send.
- Rely on the current `telex send` recovery path to re-register after
  `NeedsAttach`, including after daemon restart.
- Normalize metadata with protocol version, watch ID, event ID, attempt ID,
  script digest/mode, and nested detector metadata.
- Accept only `delivered` and `queued-unoccupied`; unknown receipt vocabulary
  fails closed.
- Distinguish daemon unavailable, residual `NeedsAttach`, IPC/auth rejection,
  command failure, malformed JSON, unknown receipt, and durable acceptance in
  attempts and inspection output.

The CLI subprocess/session behavior is a temporary integration shortcut.
Tests use a fake adapter. The report will translate observed requirements into
issue #12 rather than presenting this trait or subprocess shape as a public
contract.

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
Production rotation remains deferred.

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
9. Exercise timeout, overlap prevention, malformed/oversize output, script
   drift, execution failure, degraded backoff, and daemon restart.
10. Stop only the exact proof process after evidence is captured.

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
- known defects and incomplete validation;
- discrete deferred production items;
- issue #12 requirements for lifecycle/recovery, push/poll, service identity,
  send/receive/reply/disposition, cursor/restart, provenance/metadata, and
  IPC/binding ergonomics.

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
- Restart, timeout, non-overlap, malformed/oversize output, execution failure,
  degraded output, and script provenance are demonstrated.
- A detached Watcher produces both a real Copilot bridge wakeup and durable
  unoccupied queueing with no originating-session waiter.
- The non-PR scenario is ready for `viability-gate`.
- The report honestly identifies the at-least-once window, temporary Telex
  integration, issue #12 requirements, risks, and deferred production work.
