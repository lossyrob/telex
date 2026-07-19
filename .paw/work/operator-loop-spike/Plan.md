# Plan: Mediated Human-Attention Loop

Plan revision: 4

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
- build startup/history state from `telex export --address operator:rob`,
  `telex inbox --all --limit 200 --json`, and on-demand
  `telex read --full --json`;
- own one supervised `telex wait` courier subprocess at a time inside the
  desktop runtime for live delivery;
- on courier delivery, parse the wait payload, enrich it through `telex read`,
  ingest/dedupe it into Station state, then issue transport `telex ack` and
  re-arm only after the ack succeeds;
- recover and surface waiter timeout, daemon-gone, daemon-hung,
  presence-ended, re-attach, and malformed-payload outcomes;
- refresh both address-status projections on a bounded timer;
- call `telex read --full --json` for thread detail and disposition history;
- call `telex reply --json` for human replies;
- call the existing disposition verbs for defer/handle/close;
- require an explicit SQLite database path for the executable spike.

This deliberately favors a real, narrow loop over a premature reusable client
API. The report will inventory courier supervision, subprocess JSON parsing,
application identity, lifecycle, full-history startup projection, and cursor
limitations as requirements for issue #12.

The live demonstration will use one explicit isolated SQLite path supplied to
the worker, operator agent, Station, and smoke harness. It will not share the
implementer's coordination database or rely on whichever backend happens to be
the developer's default.

## Operational defaults

The spike will make its bounded behavior visible and configurable:

| Setting | Default |
|---|---:|
| Live courier | one supervised `telex wait` process |
| Courier idle timeout | 30 seconds, then re-arm |
| Non-wait subprocess timeout | 10 seconds |
| Recent backfill | `telex inbox --all --limit 200 --json` |
| Unresolved backfill | complete selected-address history via `telex export` |
| Status refresh interval | 5 seconds |
| Recovery backoff | 1 second, capped at 5 seconds |
| Ack retry budget | 3 attempts within 15 seconds |

The Station will record the tested `telex --json version` output at startup and
include it in diagnostics and the spike report. Response parsing will use strict
typed required fields for that tested version so missing, renamed, invalid, or
incompatible fields fail visibly rather than silently dropping provenance or
disposition state. Additive unknown fields remain compatible and are ignored.

The live receive loop is an explicitly experimental application-owned courier
supervisor, not an agent-session background waiter. It uses the daemon's current
one-shot `wait`/`ack` contract but does not claim that subprocess supervision is
the production Application Client shape. Richer streaming/callback delivery,
cursor APIs, application identity, and lifecycle management remain requirements
for issue #12.

Postgres execution is intentionally out of scope for this Windows/local spike.
Defining a credential-free stable Postgres store fingerprint and supported
application lifecycle belongs to issue #12 and later operational hardening.

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
          "sentAtMs": 1750000000000,
          "storeFingerprint": "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
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

The parser will recognize the experimental `operator-station-spike` key/URN
values only. Every other namespace remains
opaque and is rendered as raw metadata. The spike will not reserve, accept, or
preview a production-looking `operator-station` namespace before the viability
gate, `station-contract`, and issue #12 define the supported contract. The report
will inventory every baked-in experimental string and present promotion or
retirement as a later decision.

Every numeric source reference carries the same non-sensitive store fingerprint
algorithm used for local Station state. The Station resolves a source ID only
when the reference fingerprint exactly matches the active store fingerprint.
A missing or mismatched fingerprint renders `unavailable in current store` with
captured sender, subject, and timestamp; it must never open a same-number message
from another store.

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
maximum ID only after the complete export-backed unresolved projection and the
recent inbox snapshot both succeed and their union is complete, suppressing all
startup toasts. Live courier deliveries beyond that marker are toast candidates.

## Planned repository paths

- Station package overview: `operator-station-spike/README.md`
- Station package and frontend: `operator-station-spike/package.json`,
  `operator-station-spike/src/`
- Tauri runtime and CLI adapter:
  `operator-station-spike/src-tauri/src/`
- Operator-agent assignment:
  `operator-station-spike/OPERATOR-AGENT.md`
- Builder walkthrough: `operator-station-spike/WALKTHROUGH.md`
- Smoke harness:
  `operator-station-spike/harness/Invoke-OperatorLoopSmoke.ps1`
- Shared fingerprint helper:
  `operator-station-spike/harness/Get-OperatorSpikeStoreFingerprint.ps1`
- Captured additive-compatible CLI fixtures:
  `operator-station-spike/fixtures/telex-cli/*.json`
- Sanitized live demonstration evidence:
  `operator-station-spike/evidence/demo-transcript.json` and
  `operator-station-spike/evidence/demo-summary.md`
- Spike findings and issue #12 requirements:
  `docs/operator-loop-spike-report.md`

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
- Spawn every Telex child with the same persisted `TELEX_SESSION_ID`,
  configured `TELEX_ADDRESS`, and `TELEX_OPERATOR_SPIKE_DB`; also pass explicit
  session/address flags where the verb supports them. Every re-issued `attach`
  restores `--watch-pid anchor:<tauri-pid>`.
- Require the canonical `TELEX_OPERATOR_SPIKE_DB` variable for the live demo.
  The Station maps it into its adapter configuration, and every
  worker/operator-agent/harness command passes
  `--db $env:TELEX_OPERATOR_SPIKE_DB`.
- Require the versioned operator-agent assignment itself to repeat that database
  rule in its setup and every example command; the role must refuse to start the
  demo when the variable is absent.
- Scope every persisted Station session ID and toast/high-water marker to the
  configured Station address plus a normalized store fingerprint. The Station
  and PowerShell helper both require the database to exist, resolve the final
  Windows path, strip a `\\?\` prefix, normalize separators, lowercase the
  Windows path, and hash the resulting UTF-8 bytes with full SHA-256. Persist and
  display only the fingerprint, never the absolute database path or an
  unredacted store key. A Station-address or store change creates a distinct
  local state scope.
- Before coding against the metadata shape, preserve a captured fixture from a
  real `send` -> `wait` -> `read --full --json` round trip and assert the
  source-reference envelope survives enrichment byte-for-byte. Telex stores
  `message.metadata` as an opaque JSON string, so the adapter must explicitly
  parse that string a second time before interpreting the experimental envelope.
- Capture additive-compatible fixtures for every response shape the adapter
  parses: `wait`, `read --full`, `inbox --all`, `export`, `ack`, one disposition
  verb, and `reply`.
- Implement startup/history projection by streaming
  `telex export --address operator:rob --since 0` JSON lines and retaining every
  unresolved primary inbound disposition-required message plus the 200 most
  recent messages. Merge `telex inbox --all --limit 200 --json` for current
  delivery role/status fields and use `read --full` on selection.
- Complete and ingest the startup export+inbox union, then seed its high-water
  marker, before arming the first live courier. Messages arriving during startup
  remain durably queued and are delivered after arming; this removes the
  live-vs-backfill toast race.
- Implement one managed courier child at a time using
  `telex wait --timeout-ms 30000`. On exit `0`, validate the payload, fetch the
  authoritative message/metadata through `read`, ingest and dedupe it in backend
  Station state, emit it to the frontend, then `ack`; do not re-arm before ack
  success. On duplicate redelivery, update the existing record and ack
  idempotently.
- Retry `ack` up to three times within 15 seconds. If it still fails, retain the
  ingested message as `ack-pending`, surface degraded status, and re-arm after
  recovery backoff; at-least-once redelivery is deduped and retries the ack
  instead of leaving the Station silently unarmed.
- Treat exit `1` as a persistent command/protocol/configuration error: surface it
  and pause automatic re-arm until configuration changes or the user requests
  retry. Treat exit `2` as an idle re-arm. Trust `wait`'s internal re-register
  during its reconnect grace; after terminal exit `3` or `5`, explicitly
  re-issue anchored `attach` before the next courier. After two consecutive exit
  `4` daemon-hung outcomes, enter persistent degraded state and pause re-arm
  until status reports recovery or the user retries. Any malformed exit-0
  payload is a visible ingest error and is not acked. Never run two courier
  children for the same Station concurrently.
- On graceful application shutdown, cancel/kill the managed child and stop the
  Station membership. After an unexpected Tauri crash, the anchor-watch causes
  daemon-side presence end and an orphaned courier exits without ack, preserving
  at-least-once redelivery on restart.
- Implement address occupancy/status refresh on a five-second timer, separate
  from the live courier path.
- Emit Windows toasts only for newly observed primary inbound messages that are
  successfully ingested from the live courier and eligible under the declared
  policy. Initial history projection and duplicate redelivery must not re-toast.
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
  the reference fingerprint matches the active store and the message is
  available; otherwise show `unavailable in current store` and retain captured
  sender, subject, and timestamp. Never infer identity from numeric ID alone.
- Send a human reply in the selected mediated thread from the Station address.
- Support the minimum useful disposition actions: defer, handle, and close.
- Show occupancy for both `attention:rob` and `operator:rob`, including a visible
  warning when the operator agent or Station address is unattended.
- Preserve restart continuity by rebuilding feed and thread state from Telex
  history rather than browser-only message storage. Retain all unresolved
  primary obligations found in the selected-address export plus the 200 most
  recent messages.

### 3. Package the operator-agent assignment and dogfood walkthrough

- Add a reusable operator-agent assignment that explains attach/push setup,
  dedupe, raw-message handling, escalation criteria, metadata construction,
  disposition transitions, human-reply handling, and routing back to the
  worker.
- Version-stamp the assignment. Require the operator agent to record its
  assignment version and model ID in escalation metadata and to compute/include
  the shared experimental store fingerprint on every numeric source reference.
- Require these raw-thread disposition transitions: `handled` when resolved
  locally; `deferred` while awaiting worker evidence; `escalated` immediately
  after creating a human escalation; and `closed` after routing the mediated
  outcome back to the worker.
- Add a Windows walkthrough and bounded smoke harness using an isolated SQLite
  store path shared explicitly by every participant so the builder can launch
  the complete loop without further implementation. The harness must fail before
  sending if the canonical database variable is absent and must print the same
  safe store fingerprint for each participant without logging the absolute path.
  The harness initializes the database first and sets the variable from the
  resolved canonical Windows path so path aliases do not split local state
  fingerprints.
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
  exact Telex version, configured selector type, safe store fingerprint, and
  Station restart behavior.
- After the first complete real worker -> operator agent -> Station -> human
  reply -> operator agent -> worker demonstration, send a disposition-required
  `demo-review-requested` Telex message to the Operator Station workstream
  orchestrator with the evidence package and wait for its review before final
  implementation review or PR creation. Apply any `demo-feedback` and repeat the
  checkpoint as needed; a `demo-approved` response confirms only that the node's
  first complete demonstration was reviewed, not that the later builder
  viability gate passed. There is no timeout-based self-approval: an absent
  response leaves the gate closed and is escalated through the existing blocker
  protocol while this session remains attached.
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
- **Application-owned wait courier:** the desktop runtime supervises one
  short-lived `telex wait` child at a time, ingests before acking, and re-arms
  after terminal courier outcomes. This proves healthy attendance with today's
  daemon but remains a subprocess shortcut, not the issue #12 client contract.
- **Export-backed restart projection:** startup scans selected-address history
  to retain all unresolved primary obligations plus a bounded recent tail. The
  current CLI's full-history materialization cost is explicitly a spike
  shortcut and an issue #12 cursor/query requirement.
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
- **Raw demo path remains process-local:** `TELEX_OPERATOR_SPIKE_DB` necessarily
  carries the isolated SQLite path in participant process environments. The
  Station must not persist or display it, and the report records this as a
  temporary secret-handling/configuration shortcut for issue #12.
- **Self-labelled experiment:** the standalone Rust package keeps a `-spike`
  suffix and its README opens with "spike-only; do not depend on this crate."
  Adapter methods are observations for issue #12, not exported architecture.

## Verification and evidence

- Frontend unit tests for message merge/dedupe, notification eligibility,
  source-reference parsing (experimental namespace only, unknown namespace raw
  rendering, matching/missing/mismatched store fingerprints, and
  unavailable-source degradation), and reply/disposition interaction.
- Rust unit tests for configuration, command construction, JSON parsing,
  required-field validation with additive-field tolerance, courier exit-state
  recovery (including exit `1`, consecutive exit `4`, terminal `3`/`5`
  re-attach, and repeated ack failure), shared session/address/database child
  environment, ingest-before-ack ordering, idempotent duplicate delivery,
  notification policy, delayed courier arming until startup completion,
  persisted session/cursor scoping by address/store fingerprint, restart-quiet
  toast behavior after a complete initial union, and subprocess cleanup/error
  handling.
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
- Restart stress that sends at least 1,050 newer FYI or terminally dispositioned
  messages after an unresolved escalation, restarts the Station, and verifies
  the older obligation has fallen outside the explicit
  `inbox --all --limit 200` recent tail. The load also exceeds 1,000 newer IDs to
  disprove the superseded inbox-bound assumption; export-backed unresolved
  projection must still surface it without a duplicate toast.
- Captured real metadata round-trip fixture and verified graceful behavior when
  its referenced source message cannot be resolved or carries a mismatched store
  fingerprint. Add a two-store collision fixture where the same numeric ID
  exists in both stores and the Station refuses to open the wrong one.

## Risks and mitigations

- **Subprocess/JSON contract leakage:** keep the adapter private to the spike,
  document every command relied upon, and report the required semantic client
  operations to issue #12.
- **Courier failure or missed toast:** supervise exactly one waiter, make every
  terminal outcome visible, ingest before ack, normally re-arm only after ack,
  use visible `ack-pending` degraded redelivery after the bounded retry budget,
  dedupe by message ID, and suppress toasts during initial history projection.
- **Duplicate or stale delivery:** dedupe by message ID and display disposition
  state; the operator assignment must also dedupe pushed raw messages.
- **Toast registration differences on developer machines:** make toast failure
  visible without blocking feed updates, and record the actual Windows result.
- **Large history at startup:** the spike uses full selected-address `export` to
  recover every unresolved primary obligation plus a bounded recent tail. This
  is correct for the dogfood store but may materialize substantial history; a
  production unresolved-query/cursor surface is an issue #12 requirement.
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
