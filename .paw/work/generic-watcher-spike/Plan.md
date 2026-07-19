# Generic Watcher Spike Plan

Revision: 3

## Outcome

Build and exercise an experimental, separately runnable `telex-watcher`
application that persists trusted local detector registrations, executes them
outside agent sessions, and emits only normalized Telex messages. The spike will
demonstrate generic GitHub, repository-customized GitHub, and Azure DevOps
detectors, including restart recovery, a real Telex bridge wakeup, and durable
unoccupied-address queueing, without leaving a session-owned waiter or loop.

## Approach

### 1. Add a separate experimental application

- Add `telex-watcher/` as a Rust workspace member with its own
  `telex-watcher` binary. Unlike the existing throwaway `spike/` crate, this
  node exports a runnable multi-day dogfood application, so it remains under
  workspace build/test/clippy coverage. If the viability gate rejects the
  product, the whole member can be removed without changing Telex core.
- Keep it outside Telex core and `telex-console`; label the surface experimental
  in CLI help and documentation.
- Use a dedicated SQLite registry, defaulting below the user's Telex data
  directory, for watch configuration, opaque detector state, attempts, and sent
  event provenance.
- Store the registry schema version in `PRAGMA user_version`. New registries
  apply ordered migrations `0 -> 1 -> 2`; open runs every lower-version
  migration in one transaction and refuses an unknown higher version without
  modifying the file. Version 2 intentionally adds send-time routing
  provenance so the migration harness is exercised by this spike.
- Hold an advisory OS file lock with `fs2` while `run` is active so only one
  runtime schedules a registry at a time while management commands remain
  available. Lock ownership, not lock-file existence, is authoritative and
  releases automatically on process exit. Network-share registries are
  unsupported in the spike.

### 2. Define detector protocol version 1

The Watcher sends one JSON request on stdin containing:

- `schemaVersion: 1`;
- stable watch ID and opaque registration parameters;
- the detector's prior opaque JSON state;
- current UTC time and attempt ID;
- script mode and the exact SHA-256 digest being executed.

The detector must exit successfully and emit exactly one bounded JSON result on
stdout:

- `idle`: successful observation with optional `nextState`;
- `event`: one required normalized event plus optional `nextState`;
- `terminal`: optional final event plus optional `nextState`, then stop after a
  successful transaction;
- `degraded`: no state advancement; retry under Watcher backoff policy.

An event contains only stable event ID, namespaced kind, subject, body, and
opaque metadata. It cannot provide target, sender, cadence, timeout,
environment, a command, or any post-trigger action. Attention and disposition
requirements are registration policy rather than detector output.

Process exit status represents execution success or failure. Protocol version 1
uses these explicit limits:

| Field | Limit |
|---|---:|
| Detector stdout | 256 KiB |
| Detector stderr | 64 KiB |
| Opaque state | 256 KiB serialized |
| Event subject | 512 UTF-8 bytes |
| Event body | 128 KiB |
| Detector metadata | 64 KiB serialized |

Cap failures set structured `truncated` and `limit` diagnostics; truncated event
content is never sent as success.

### 3. Persist policy, state, and provenance

Persist each watch's:

- stable ID, command argv, working directory, and explicit script path;
- `follow-path` or `pinned` script mode and pinned digest when applicable;
- fixed Telex sender and target;
- cadence, timeout, attention, disposition requirement, and environment
  allowlist;
- opaque parameters and current opaque state;
- lifecycle status, next due time, failure count, and last diagnostic.

Persist every attempt with timing, outcome, script digest, bounded/redacted
stderr, and error details. Persist accepted events with watch ID, event ID,
send-time sender and target, Telex message ID/receipt, attempt ID, script
digest, and acceptance timestamp.
Removal is a soft lifecycle state and preserves audit rows. Sent-event
provenance is not automatically pruned. Attempts retain the newest 1,000 rows
per watch by default and may also be pruned explicitly with `gc`; the report
will measure registry growth and record this experimental policy.

Use a unique `(watch_id, event_id)` ledger index and an index on
`(watch_id, accepted_at_ms)` for inspection. Explicit event-ledger pruning is
allowed only for terminal or removed watches after export, never for active
watches. `status` warns when the ledger exceeds 100,000 rows or the registry
exceeds 100 MiB, and the spike report records the measured growth rate.

Persist runtime identity/heartbeat and in-flight attempt PID, process-group ID,
process start token, and command fingerprint. On restart, a stale in-flight
attempt becomes `orphaned` and the watch is not rescheduled until the old group
is proven gone or an operator explicitly invokes recovery. This prevents
overlap even when abrupt Unix process death leaves descendants.

### 4. Enforce the state/send transaction

- `idle`: atomically commit validated `nextState`, reset failure backoff, and
  schedule the next run.
- `event`: first check the sent-event ledger for `(watch_id, event_id)`. An
  already-committed event is not sent again; its proposed state may be committed
  because the ledger proves prior durable acceptance. Otherwise send to the
  registration's fixed Telex sender/target first. Only after a durable
  `delivered` or `queued-unoccupied` receipt atomically commit `nextState`, the
  sent-event ledger row, and the next schedule.
- `terminal` with an event: use the same send-before-state ordering, then mark
  the watch terminal in the same local commit transaction. The sent-event
  ledger uses the same `(watch_id, event_id)` key for terminal events.
- `terminal` without an event: commit state and terminal status directly.
- `degraded`, timeout, non-zero exit, malformed output, oversize output, pinned
  digest mismatch, or Telex failure: retain prior detector state, record the
  attempt, increase bounded exponential backoff, and expose the failure through
  inspection commands.

If Telex accepts a message and the Watcher crashes before its local commit, the
detector may repeat the stable event ID on restart because no ledger row exists.
That applies equally to `event` and event-producing `terminal` results. The
scheduler re-runs the still-active watch; each interrupted acceptance attempt
can create at most one additional copy before its local commit. The spike will
document this explicit at-least-once window rather than hiding it.

### 5. Bound scheduling and execution

- Schedule active watches from persisted due times and recover them on restart.
- Enforce one in-flight attempt per watch plus a configurable global
  concurrency limit.
- Spawn every detector in a cross-platform process group using
  `command-group`: a Windows Job Object or a Unix process group. Timeout,
  Watcher shutdown, and cancellation terminate the full descendant tree, not
  only the shell parent.
- Read stdout and stderr concurrently with hard byte caps.
- Apply bounded exponential retry backoff for execution failures and degraded
  results.
- Use monotonic deadlines while the process is running and wall-clock UTC only
  for persisted due times and detector `now`. Detect divergence between
  monotonic and wall-clock elapsed time. After resume or a large forward jump,
  run each overdue watch at most once, spread due work with bounded jitter, and
  schedule the next cadence from completion. A backward wall-clock jump
  rebases deadlines rather than stalling until the old timestamp returns.
  Record observed clock skew on the attempt.
- Read and hash the script immediately before spawn, then hash it again before
  accepting detector output. A digest change during execution invalidates the
  result, so a follow-path mid-save cannot emit or advance state. Detector
  authoring guidance will recommend atomic file replacement.
- Clear the child environment. Restore the platform launch baseline
  (`PATH`, temporary-directory variables, locale, shell/system-root variables,
  and user profile/config roots) and copy only registration-approved inherited
  variable names. Example allowlists include `GH_CONFIG_DIR`, `GH_TOKEN`,
  `AZURE_DEVOPS_EXT_PAT`, and `AZURE_CONFIG_DIR`. Values are read at execution
  time, never persisted. Known inherited secret values are replaced in bounded
  stderr with `[redacted:<VARIABLE>]`.
- Handle Ctrl+C, SIGINT, and SIGTERM as a graceful drain: stop scheduling,
  wait up to 10 seconds, then terminate remaining process groups. On Windows,
  require and test Job Object `KILL_ON_JOB_CLOSE` behavior. On Unix, SIGKILL
  cannot run cleanup; persisted orphan state blocks rescheduling and
  `recover <watch-id> --terminate` performs a start-token/fingerprint-guarded
  best-effort group kill. If identity cannot be proved, report the orphan for
  operator action rather than risking an unrelated PID.
- Write Watcher-runtime JSONL logs separately from detector attempt stderr.
  Default to `<registry>.logs/`, rotate daily, retain seven files and at most
  50 MiB combined, and support `run --log-file` plus `--log-level`.

### 6. Use a narrow experimental Telex adapter

- Define an internal `TelexAdapter` interface returning a classified send
  receipt. Implement it with a configured `telex` subprocess for the spike;
  tests use a fake adapter, and a future issue #12 client can replace the CLI
  implementation without changing scheduling or state logic.
- Give the application a stable service session identity derived from its
  registry and pass it on every send. This is an application identity, not
  `COPILOT_AGENT_SESSION_ID`; agent-session coordination commands retain their
  existing Copilot session requirement.
- Pass the registration's fixed sender/target and policy-controlled attention
  and disposition requirement.
- Add Watcher protocol version, watch ID, event ID, attempt ID, script digest,
  and script mode to normalized metadata while retaining detector metadata as
  an opaque nested value.
- The CLI adapter passes an explicit sender and session on every invocation. It
  relies on the current `telex send` recovery path to register/re-register the
  sender after `NeedsAttach`, including after daemon restart. Residual
  `NeedsAttach`, daemon unavailable, IPC/auth rejection, process failure,
  malformed JSON, and unknown receipt values are classified separately and do
  not advance detector state.
- Parse the JSON send receipt and treat only durable `delivered` or
  `queued-unoccupied` receipts as acceptance. Any new receipt vocabulary fails
  closed until classified.
- Apply adapter backoff from 1 second to a 60-second ceiling with 20% jitter.
  Persist the last successful contact and current adapter failure/backoff
  summary so an all-watches Telex outage is distinguishable from provider
  failures. Test both transient daemon restart and a daemon that remains down
  through the ceiling.
- Report this subprocess/session integration as temporary evidence for issue
  #12 rather than a production application-client contract.

### 7. Provide local management commands

Implement commands sufficient for dogfooding:

- `add --file <watch.json>`;
- `list`;
- `show <watch-id>`;
- `pause <watch-id>`;
- `resume <watch-id>`;
- `update <watch-id> --file <watch.json>`;
- `remove <watch-id>`;
- `attempts <watch-id> [--limit <n>]`;
- `events <watch-id> [--limit <n>]`;
- `status`;
- `stop [--grace-seconds <n>]`;
- `recover <watch-id> --terminate`;
- `backup <path>`;
- `export <path>`;
- `gc [--attempt-retention <n>] [--events-before <time> --status
  terminal|removed]`;
- `run`, plus bounded single-pass/id-selective options for tests and manual
  exercises.

Add/update validates command argv, canonical local script and working-directory
paths, protocol policy, JSON fields, script digest, timing bounds, and Telex
addresses before persisting changes. It also validates environment variable
names without persisting their values. `list` and `show` expose a per-watch
health summary, consecutive failure count, next due time, last attempt ID,
last script digest, and last accepted event/receipt. `attempts` and `events`
provide structured JSON-capable local diagnostics correlated by watch,
attempt, event, script digest, and Telex message ID. `update` cannot change
sender or target after creation; rerouting requires removing the old watch and
adding a new watch ID. `status` reports registry schema version, runtime
PID/uptime, last scheduler tick, adapter last contact/current backoff, runtime
log path, and watch counts by lifecycle. `backup` uses SQLite's online backup
API; recovery stops the runtime, restores the file, and reopens through normal
schema checks.

### 8. Add editable detector examples

- Generic GitHub PR detector using `gh`, adapting useful PR state/review/check
  decisions from the Loop reference without its worker/waiter lifecycle.
- Customized GitHub detector demonstrating repository-specific author/comment
  filtering without runtime changes.
- Azure DevOps PR detector using the same stdin/stdout protocol against Azure
  DevOps REST data.
- Include sample registrations and fixture-capable inputs so detector behavior
  is reproducible. The Azure DevOps template accepts `organization`, `project`,
  `repository`, and `pullRequest` parameters and uses
  `AZURE_DEVOPS_EXT_PAT` only when explicitly allowlisted. The deterministic
  proof fixture will live at
  `telex-watcher/examples/fixtures/azure-devops-pr.json` and preserve an Azure
  DevOps REST 7.1 PR response shape.
- Before the live exercise, preflight optional live ADO coordinates supplied as
  `TELEX_WATCHER_ADO_ORG`, `TELEX_WATCHER_ADO_PROJECT`,
  `TELEX_WATCHER_ADO_REPO`, and `TELEX_WATCHER_ADO_PR`. If all are present and
  reachable, exercise the real endpoint. Otherwise execute the ADO detector
  through the detached live Watcher against the checked-in fixture and mark
  provider-network validation incomplete in the report rather than silently
  dropping the required detector scenario.
- The external Plan approval is also the explicit decision point for this
  fallback: approval of revision 3 accepts fixture-mode as sufficient for the
  detector-protocol proof when no live ADO coordinates exist. Feedback that
  requires live ADO becomes a blocking prerequisite before implementation.

### 9. Validate behavior and run the live spike

Automated validation will cover:

- protocol parsing and policy-field rejection;
- idle state advancement;
- event state remaining unchanged before/after a failed Telex send;
- event state and sent ledger committing after both `delivered` and
  `queued-unoccupied` durable receipts, with unknown receipt values rejected;
- stable event/watch/script provenance;
- restart recovery from persisted state, with zero resend after a fully
  committed event and at most one duplicate per deliberately interrupted
  send-accepted/local-commit attempt;
- timeout, malformed output, output caps, non-zero exit, degraded backoff,
  pinned digest mismatch, follow-path mid-execution drift, process-tree
  termination, and non-overlap;
- event-producing terminal crash/retry behavior;
- daemon restart and `NeedsAttach` recovery through the CLI adapter;
- degraded-watch surfacing and structured attempt/event diagnostics;
- schema `1 -> 2` migration preserving committed sent-event rows and refusal of
  unknown higher versions;
- graceful drain, Unix orphan-state recovery, and Windows Job Object closure;
- simulated suspend/forward/backward clock jumps with one-run catch-up and
  jitter;
- runtime log rotation and adapter-wide stall visibility;
- CLI add/inspect/pause/resume/update/remove/status/stop/recover/backup/export/
  gc behavior.

The live exercise will:

1. Build the Watcher and local Telex binary.
2. Preflight GitHub and optional live ADO access, then register generic GitHub,
   customized GitHub, and Azure DevOps watches. Use the checked-in ADO fixture
   if live ADO coordinates are absent or unreachable and record that limitation.
3. Launch `telex-watcher run` as a fully independent detached process, with no
   Telex waiter or Loop task in the originating agent session.
4. Observe an actual Copilot bridge wakeup at the occupied implementer address
   and a `queued-unoccupied` receipt for a pre-created unoccupied proof address.
5. Restart the Telex daemon under the running Watcher and verify sender
   re-registration without permanent stall.
6. Stop and restart the Watcher, verify opaque state recovery, assert zero
   resend for committed event IDs, and demonstrate the documented single-crash
   duplicate window with a controlled acceptance/commit interruption.
7. Exercise timeout with a descendant process, overlap, malformed/oversize
   output, digest drift, execution failure, and degraded backoff, then inspect
   the bounded structured diagnostics.
8. Exercise graceful stop, abrupt restart with a controlled stale attempt,
   scheduler catch-up after a simulated clock gap, registry backup/restore, and
   schema migration while preserving committed event provenance.
9. Stop only the exact proof process after evidence is captured; the application
   itself remains runnable for later multi-day dogfooding.

### 10. Produce the spike report

Create `docs/generic-watcher-spike-report.md` covering:

- exercised provider scenarios and evidence;
- detector protocol and state/send ordering;
- restart and failure behavior;
- script and event provenance;
- detector-authoring experience;
- trusted-local execution, credential, and logging observations;
- registry growth, retention threshold, backup/restore, schema migration,
  scheduler clock behavior, and abrupt-shutdown limitations;
- every temporary integration shortcut;
- requirements exported to issue #12, organized by lifecycle/recovery,
  push/poll behavior, service/application identity, send/receive/reply/
  disposition needs, cursor/restart behavior, provenance/metadata, and
  supported IPC/binding ergonomics;
- known defects, risks, incomplete validation, and discrete deferred work.

## Key Decisions

- Eventless `idle` results may advance opaque state because detectors need to
  record deliberately ignored observations and provider cursors.
- `degraded` never advances state.
- Attention and `requiresDisposition` are registration policy, not detector
  output.
- `pinned` is the default dogfood mode; `follow-path` is an explicit
  development opt-in that records every executed digest and rejects results if
  the file changes during execution.
- Removed watches retain sent-event provenance and recent attempt history;
  explicit `gc` may prune attempts and may prune exported ledger rows only for
  terminal or removed watches.
- Repeated failures remain locally visible in the spike; automatic failure
  notification policy is deferred to the production contract.
- The internal `TelexAdapter` is the migration seam. The CLI subprocess is its
  experimental implementation; no new public SDK is frozen in this node.
- Both live Copilot wakeup and durable unoccupied queueing are required in this
  node.
- Sender and target are immutable for a watch ID.
- Registry schema migration, backup, process-level status, and rotated runtime
  logs are included because the deliverable is a multi-day dogfood application,
  not a one-shot schema prototype.

## Work Items

1. **Runtime and protocol:** implement the crate, registry, detector contract,
   scheduler, process bounds, Telex adapter, and management CLI.
2. **Behavioral tests:** prove state ordering, recovery, provenance, lifecycle
   commands, and bounded failure cases with real child processes and a fake
   receipt-producing Telex executable.
3. **Detector examples:** add and exercise generic GitHub, customized GitHub,
   and Azure DevOps templates plus sample registrations.
4. **Live proof and report:** run the detached multi-detector/restart/failure
   exercise, capture evidence, and write the spike report.

Work items are intentionally sequenced because tests and templates depend on
the protocol/runtime contract, and the report depends on the executed system.

## Completion Criteria

- The application and workspace pass formatting, build, tests, and clippy.
- All management operations work against a persistent registry.
- The required three detector scenarios run through the same provider-neutral
  runtime.
- Event-producing state is demonstrably unchanged on send failure and committed
  only after durable Telex acceptance.
- Restart, timeout, overlap, malformed output, execution failure, degraded
  output, and script provenance are demonstrated.
- A detached Watcher process produces both a real Telex bridge wakeup and a
  durable unoccupied-address queue with no originating-session waiter.
- The report honestly identifies the at-least-once window, temporary CLI
  integration, issue #12 requirements, risks, and deferred production work.
