# Plan: Mediated Human-Attention Loop

Plan revision: 2

## Outcome anchor

Deliver a runnable Windows-first experimental loop for issue #93:

1. A worker sends an operational message to `attention:rob`.
2. An operator agent attending `attention:rob` inspects the raw worker thread and
   sends a distilled escalation to `operator:rob`.
3. The desktop Station attending `operator:rob` backfills and displays the
   escalation, emits a Windows notification under a narrow policy, shows the
   mediated thread and source provenance, and accepts a human reply and minimum
   disposition actions.
4. The operator agent receives the human reply in the mediated thread and routes
   the result back through the separate raw worker thread.
5. Restarting the Station restores recent and unresolved messages from Telex.

The spike will prepare this loop for the builder's viability gate without
self-passing that gate or defining the production Application Client contract
owned through issue #12.

## Approach summary

Add a separately runnable Tauri v2 application under
`operator-station-spike/`. It will not be a member of the root Rust workspace and
will not add desktop or UI dependencies to the `telex` binary.

The Station will use the installed or explicitly configured `telex` executable
as a temporary subprocess integration seam:

- attach `operator:rob` with a stable application session identity and the
  desktop process as a watched PID;
- build startup state from the deduplicated union of actionable
  `telex inbox --json` and recent `telex inbox --all --json` snapshots;
- poll bounded snapshots for new messages and refresh both address-status
  projections;
- call `telex read --full --json` for thread detail and disposition history;
- call `telex reply --json` for human replies;
- call the existing disposition verbs for defer/handle/close;
- preserve backend selection through optional backend/database configuration.

This deliberately favors a real, narrow loop over a premature reusable client
API. The report will inventory subprocess polling, command JSON parsing,
application identity, lifecycle, and cursor limitations as requirements for
issue #12.

The live demonstration will use one explicit isolated SQLite path supplied to
the worker, operator agent, Station, and smoke harness. It will not share the
implementer's coordination database or rely on whichever backend happens to be
the developer's default.

## Operational defaults

The spike will make its bounded behavior visible and configurable:

| Setting | Default |
|---|---:|
| Poll interval | 2 seconds |
| Telex subprocess timeout | 10 seconds |
| Recent backfill limit | 200 messages |
| Actionable backfill limit | 1,000 messages |
| Status refresh interval | Same 2-second scheduler tick as message refresh |

The Station will record the tested `telex --json version` output at startup and
include it in diagnostics and the spike report. Response parsing will use strict
typed fixtures for that tested version so missing, renamed, or unexpected
top-level JSON fields fail visibly rather than silently dropping provenance or
disposition state.

The Station will not arm `telex wait` or register a push callback. This is an
explicit spike shortcut: the attached address is read through polling durable
records, so production answerback, push-delivery health, richer cursors, and
deaf-station semantics remain requirements for issue #12 rather than conclusions
of this experiment.

## Experimental message convention

The reusable operator-agent assignment will use:

- ingress address: `attention:rob`;
- human Station address: `operator:rob`;
- escalation kind: `operator-station-spike.escalation`;
- human-reply kind: `operator-station-spike.human-reply`;
- an opaque metadata envelope aligned with the extension proposal:

```json
{
  "extensions": {
    "operator-station-spike": "urn:telex:experimental:operator-station-spike:v1"
  },
  "dataschema": "urn:telex:experimental:operator-station-spike:v1#escalation",
  "ext": {
    "operator-station-spike": {
      "sourceMessages": [
        {
          "id": 123,
          "threadId": 123,
          "from": "worker:example",
          "to": "attention:rob",
          "subject": "Decision needed",
          "sentAtMs": 1750000000000
        }
      ],
      "ingressAddress": "attention:rob",
      "operatorAgent": {
        "assignmentVersion": "1",
        "modelId": "recorded-by-agent"
      }
    }
  }
}
```

Telex core will continue to carry this metadata opaquely. The Station will only
render recognized experimental source references; unknown metadata remains
visible as raw JSON. The operator agent sends from its own attended address and
never impersonates the worker.

The parser will recognize both the experimental
`operator-station-spike` key/URN and a reserved future `operator-station`
key/URN concurrently. This accept-both behavior is a migration escape hatch, not
a production convention. The report will inventory every baked-in spike string
and present promotion or retirement as an issue #12 decision.

## Notification policy

Initial backfill never emits a toast. For newly observed primary inbound
messages, the Station uses this table:

| Condition | Toast |
|---|---|
| `attention=interrupt` | yes |
| `attention=next-checkpoint` and disposition required | yes |
| recognized escalation kind, disposition required, and attention is not `fyi` | yes |
| background/FYI status, CC traffic, or other messages | no; feed only |

Feed content always comes from Telex. A small persisted
`last-observed-max-message-id` marker is permitted solely to prevent restart
re-toasts; it is not a local message cache. Cold start seeds that marker from the
maximum ID in the first successful backfill, suppressing all startup toasts.

## Work items

### 1. Build the experimental Station runtime

- Scaffold the standalone Tauri/React application and document Windows
  prerequisites and configuration.
- Implement a typed Telex CLI adapter with bounded command timeouts, explicit
  UTF-8 JSON handling, actionable error reporting, and unit-tested argument and
  response parsing.
- Attach the configured Station address on startup with a stable session ID and
  `anchor:<pid>` watch on the Tauri Rust process. Persist the generated Station
  session UUID under the application data directory and reuse it across
  restarts.
- Require the canonical `TELEX_OPERATOR_SPIKE_DB` variable for the live demo.
  The Station maps it into its adapter configuration, and every
  worker/operator-agent/harness command passes
  `--db $env:TELEX_OPERATOR_SPIKE_DB`.
- Before coding against the metadata shape, preserve a captured fixture from a
  real `send` -> `inbox --json` -> `read --full --json` round trip and assert the
  source-reference envelope survives byte-for-byte.
- Implement bounded actionable-plus-recent polling/backfill, deduplication by
  message ID, address occupancy/status refresh, and visible reconnect/error
  status without creating a Telex waiter.
- Emit Windows toasts only for newly observed primary inbound messages that are
  eligible under the declared policy. Initial backfill and repeated snapshots
  must not re-toast.
- Keep diagnostics in a bounded in-memory ring and avoid persistent message-body
  logging. The report captures one diagnostic snapshot; long-run telemetry and
  rolling-log policy remain explicit issue #12 requirements.

### 2. Build the feed, thread, provenance, reply, and disposition UI

- Show recent inbound Station messages with sender, kind, attention,
  disposition requirement, subject/body, and latest disposition.
- Load the complete mediated thread and disposition history for a selected
  message.
- Render experimental source-message references separately from the mediated
  thread, while retaining raw metadata inspection. Resolve source IDs when
  available; show `resolved`, `not-found`, or `different-backend` state and retain
  captured sender, subject, and timestamp when a raw source cannot be opened.
- Send a human reply in the selected mediated thread from the Station address.
- Support the minimum useful disposition actions: defer, handle, and close.
- Show occupancy for both `attention:rob` and `operator:rob`, including a visible
  warning when the operator agent or Station address is unattended.
- Preserve restart continuity by rebuilding feed and thread state from Telex
  backfill rather than browser-only message storage. Combine up to 1,000
  actionable messages with the 200 most recent messages so an older unresolved
  obligation remains visible after newer traffic.

### 3. Package the operator-agent assignment and dogfood walkthrough

- Add a reusable operator-agent assignment that explains attach/push setup,
  dedupe, raw-message handling, escalation criteria, metadata construction,
  disposition transitions, human-reply handling, and routing back to the
  worker.
- Version-stamp the assignment. Require the operator agent to record its
  assignment version and model ID in escalation metadata.
- Require these raw-thread disposition transitions: `handled` when resolved
  locally; `deferred` while awaiting worker evidence; `escalated` immediately
  after creating a human escalation; and `closed` after routing the mediated
  outcome back to the worker.
- Add a Windows walkthrough and bounded smoke harness using an isolated SQLite
  store path shared explicitly by every participant so the builder can launch
  the complete loop without further implementation. The harness must fail before
  sending if the canonical database variable is absent and must print the
  resolved database path for each participant.
- Add a scripted operator-agent stand-in for regression coverage. It supplements
  rather than replaces the prompt-driven operator-agent run and must assert the
  raw message reaches `escalated`, the human reply stays in the mediated thread,
  and the routed response reaches the worker in the original raw thread.
- Exercise operator-agent absence by stopping its `attention:rob` station and
  asserting the Station's occupancy warning becomes visible before reattaching.
- Keep all policy in the assignment or Station; do not add filtering, routing,
  or human UI semantics to Telex core.

### 4. Demonstrate and report the complete loop

- Exercise the real Telex message path on Windows with separate worker,
  operator-agent, and Station identities.
- Record raw worker message, human escalation, human reply, routed worker
  response, thread IDs, source metadata, disposition states, Station backfill,
  notification attempt/result, message-to-toast latency, address occupancy, the
  exact Telex version/backend/database identity, and Station restart behavior.
- Write `docs/operator-loop-spike-report.md` with demonstrated value, failures,
  assumptions, temporary integration shortcuts, known defects, and concrete
  Application Client requirements for issue #12.
- Include a success-criterion evidence matrix with one row per issue #93
  criterion and links to test names, captured JSON, screenshots/log evidence,
  or walkthrough steps.
- Include an append-only "Viability gate observations" section and recording
  protocol so subsequent builder dogfood has a nominated evidence home without
  changing the experimental contract.
- Clearly distinguish the completed implementation/demo from the separate
  builder viability judgment.

## Key decisions and boundaries

- **Separate application:** desktop dependencies remain outside Telex core and
  the root workspace.
- **CLI subprocess seam:** accepted only for this spike; it is not a public
  Application Client decision.
- **Bounded polling instead of a waiter:** the desktop process owns a two-second
  polling loop over durable records. It will not run `telex wait`; restart
  recovery comes from actionable-plus-recent backfill. The missing push,
  station-health, and cursor contract is reported to issue #12.
- **Two auditable threads:** raw worker/operator traffic and mediated
  operator/human traffic remain distinct and are linked through opaque source
  references.
- **No generalized routing or command execution:** the Station presents and
  replies; the operator assignment contains filtering and routing judgment.
- **Windows-first:** toast registration and manual desktop validation target
  Windows. Portable unit tests may use no-op notification behavior.
- **No production packaging promise:** development launch and builder dogfood
  are in scope; installer, auto-start, upgrade, and multi-platform support are
  deferred.
- **Self-labelled experiment:** the standalone Rust package keeps a `-spike`
  suffix and its README opens with "spike-only; do not depend on this crate."
  Adapter methods are observations for issue #12, not exported architecture.

## Verification and evidence

- Frontend unit tests for message merge/dedupe, notification eligibility,
  source-reference parsing (including legacy/future alias and missing-source
  degradation), and reply/disposition interaction.
- Rust unit tests for configuration, command construction, JSON parsing,
  strict tested-version fixtures, notification policy, persisted session/cursor
  reuse, restart-quiet toast behavior, and subprocess error handling.
- `npm test`, `npm run build`, `cargo fmt --check`, and `cargo test` in the
  standalone Station.
- Root `cargo test --workspace` only as a boundary regression guard proving the
  separate experiment did not disturb or enter Telex core; it does not validate
  the standalone Station.
- Windows desktop launch with an isolated SQLite store and the current Telex
  binary.
- Live end-to-end evidence for worker -> operator agent -> Station -> human
  reply -> operator agent -> worker, plus a Station restart/backfill check.
- Smoke-harness regression for the full route-back leg and raw `escalated`
  disposition.
- Restart stress that sends at least 220 newer background messages after an
  unresolved escalation, restarts the Station, and verifies the older
  actionable escalation has fallen outside the 200-message recent tail but
  remains visible through actionable backfill without a duplicate toast.
- Preflight the configured actionable limit against the tested binary with a
  real `telex inbox --limit 1000 --json` call and fail visibly if that command or
  response shape is unsupported.
- Captured real metadata round-trip fixture and verified graceful behavior when
  its referenced source message cannot be resolved.

## Risks and mitigations

- **Subprocess/JSON contract leakage:** keep the adapter private to the spike,
  document every command relied upon, and report the required semantic client
  operations to issue #12.
- **Polling latency or missed toast:** use monotonic message IDs, bounded
  snapshots, explicit dedupe, a persisted observation cursor, and no-toast
  initial backfill; durable Telex data remains authoritative.
- **Duplicate or stale delivery:** dedupe by message ID and display disposition
  state; the operator assignment must also dedupe pushed raw messages.
- **Toast registration differences on developer machines:** make toast failure
  visible without blocking feed updates, and record the actual Windows result.
- **Old unresolved obligations:** union a high bounded actionable set with the
  recent feed and test past the default inbox window. More than 1,000 concurrent
  unresolved items is outside this spike and becomes a cursor/query requirement
  for issue #12.
- **UI scope growth:** prefer a plain two-pane feed/thread surface. Do not add
  routing modes, aliases, rich decisions, process control, or broad filtering.
- **Operator-agent absence:** queued messages and source threads remain durable;
  the Station surfaces address occupancy and the walkthrough explicitly
  reattaches the role before continuing.
- **No production telemetry:** the spike records per-demo notification outcome,
  latency, and adapter errors but does not add a durable metrics subsystem. That
  is carried as an issue #12/operational-hardening requirement.

## Completion rule

Use `Closes #93` only if the runnable Station, reusable assignment, spike
report, source-provenance convention, complete live loop, notification behavior,
and restart/backfill evidence are all present. Otherwise create a partial PR
with `Refs #93` and identify the exact missing outcome.
