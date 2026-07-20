# Generic Watcher Spike Report

## Result

The spike demonstrates a provider-neutral external detector runner that operates
outside agent sessions and performs one reaction only: send a normalized Telex
message to registration-owned routing.

The implementation includes:

- an experimental `telex-watcher` binary and SQLite registry;
- local add, list, show, pause, resume, update, remove, attempts, events, and run
  commands;
- versioned JSON detector request/result semantics;
- opaque detector state;
- receipt-gated event state transitions;
- stable watch/event/script/message provenance;
- bounded detector execution and concurrency;
- PID-bound multi-sender Telex station lifecycle;
- editable GitHub, customized GitHub, Azure DevOps, and non-PR detector
  examples.

The required GitHub, repository-customized GitHub, and live Azure DevOps
scenarios were exercised through the same runtime. Occupied Copilot wakeup and
unoccupied durable queueing were both observed without an originating-session
waiter.

This evidence is sufficient for the #101 implementation node. It does not pass
the later builder-owned multi-watch viability gate.

## What was built

### Experimental application

`telex-watcher/` is a workspace member with its own binary. It is explicitly
experimental and not part of Telex core or `telex-console`.

The runtime stores:

- watch policy and lifecycle state;
- opaque detector state;
- bounded attempt diagnostics;
- sent-event transition evidence;
- runtime incarnation diagnostics.

Every Watcher process creates a fresh runtime UUID. One runtime session serves
all configured sender addresses. Each sender is explicitly attached with the
actual Watcher PID as a Telex `required` watch predicate before its watches can
send.

### Management CLI

The spike implements:

```text
telex-watcher add --file <watch.json>
telex-watcher list
telex-watcher show <watch-id>
telex-watcher pause <watch-id>
telex-watcher resume <watch-id>
telex-watcher update <watch-id> --file <watch.json>
telex-watcher remove <watch-id>
telex-watcher attempts <watch-id>
telex-watcher events <watch-id>
telex-watcher run [--once] [--watch <id>] [--concurrency <n>]
```

Sender and target are immutable for a watch ID. Registration validates local
paths, argv, timing bounds, script mode/digest, environment variable names,
addresses, parameters, and state size before persistence.

## Detector protocol

### Request

Watcher writes one JSON request to detector stdin:

```json
{
  "schemaVersion": 1,
  "attempt": {
    "id": "attempt-id",
    "now": "2026-07-19T00:00:00Z"
  },
  "watch": {
    "id": "watch-id",
    "parameters": {}
  },
  "script": {
    "mode": "pinned",
    "sha256": "..."
  },
  "state": {}
}
```

### Result

The detector exits zero after writing exactly one structured result:

| Outcome | Meaning | State behavior |
|---|---|---|
| `idle` | Successful observation with no event | Valid `nextState` commits immediately |
| `event` | Send one normalized Telex event and continue | `nextState` commits only after durable Telex receipt |
| `terminal` | Optional final event, then stop | Event-producing state follows receipt; eventless state commits directly |
| `degraded` | Source could not be evaluated | State does not advance |

Process failure remains separate from detector outcome.

The event contains only:

- stable event ID;
- namespaced kind;
- subject;
- body;
- opaque metadata.

It cannot override sender, target, attention, disposition policy, cadence,
timeout, working directory, environment policy, or request an action.

### Limits

Protocol version 1 enforces:

| Value | Limit |
|---|---:|
| Detector stdout | 256 KiB |
| Detector stderr | 64 KiB |
| Opaque state | 256 KiB serialized |
| Event subject | 512 UTF-8 bytes |
| Event body | 128 KiB |
| Detector metadata | 64 KiB serialized |
| Normalized Watcher metadata | 80 KiB serialized |

Truncated or oversize event output is never sent as success.

## State and send ordering

The event transaction is:

```text
read prior state
-> execute and validate detector
-> normalize event
-> send to fixed Telex sender/target
-> receive delivered or queued-unoccupied receipt
-> atomically commit next state + sent-event row + attempt result
```

Failed sends, malformed receipts, unknown receipt values, timeouts, malformed
output, digest mismatch, script drift, and degraded results leave prior state
unchanged.

### Duplicate event IDs

Committed event evidence binds:

- watch and event ID;
- prior-state and committed next-state hashes;
- normalized envelope hash;
- script digest;
- sender and target;
- Telex message ID and receipt.

An already-committed event ID never authorizes detector-proposed state:

- matching envelope evidence becomes a visible `stale-duplicate` no-op;
- conflicting evidence becomes `duplicate-event-conflict`;
- neither branch sends, advances state, or marks terminal.

### At-least-once window observed live

The live provider proof initially parsed real Telex receipts with the wrong
field-name convention. Telex had durably accepted messages, but Watcher rejected
the local receipt representation and did not commit state or ledger rows.

The detector repeated the same stable event IDs. Several accepted messages later
arrived as duplicates. After receipt compatibility was fixed, the next accepted
message committed state and provenance.

This is the intended safe failure direction:

- no consume-before-send loss;
- visible duplicates after acceptance/commit uncertainty;
- stable event IDs make deduplication possible.

## Provider evidence

### Evidence matrix

| Scenario | Provider input | Event | Receipt/consumer evidence | Result |
|---|---|---|---|---|
| Generic GitHub | Public merged `lossyrob/telex` PR | `github.pull-request.completed` | Telex `delivered`; recipient ack and handled disposition | Passed |
| Customized GitHub | Public PR with author activity plus one external human comment | `github.pull-request.external-activity` | Telex `queued-unoccupied`; message remains in durable inbox | Passed |
| Azure DevOps | Campaign-authorized disposable PR with one README-only commit | `azure-devops.pull-request.created` | Telex `delivered`; recipient ack and handled disposition | Passed |
| Non-PR handoff | Local JSON condition fixture | `local.file-json.ready` | Runtime protocol/replay tests; live delivery deferred to viability gate | Ready |

Private Azure DevOps organization/project/repository coordinates are
intentionally omitted from this public report. The disposable PR remains
active and unmerged as directed.

### Generic GitHub

The generic detector adapted useful PR state decisions from the Loop reference:

- terminal merged/closed state;
- changes requested;
- failing checks;
- blocked/dirty/behind/unstable merge state;
- approved clean readiness;
- optional deterministic initial snapshot.

The live proof produced message `1410`. Watcher recorded:

- stable event ID;
- script digest;
- prior and next state hashes;
- normalized envelope hash;
- durable `delivered` receipt;
- atomic event/state commit.

The Copilot bridge surfaced the event as a real turn. The recipient explicitly
acked and handled it, proving bridge wake and consumption rather than relying on
the `delivered` receipt alone.

### Repository-customized GitHub

The customized detector:

- ignores the PR author;
- ignores configured self/bot logins;
- retains substantive external reviews/comments;
- emits only when external activity remains after filtering.

The live case ignored repository-owner activity and selected one external human
comment. Watcher sent message `1431` to a pre-created unoccupied address.

Evidence:

- receipt: `queued-unoccupied`;
- target occupancy: false;
- no daemon member;
- durable inbox contained the actionable message with no disposition.

This demonstrates customization without runtime changes and store-and-forward
delivery without a session waiter.

### Azure DevOps

The detector uses REST API 7.1 and supports two explicit credential policies:

- allowlisted bearer token;
- allowlisted PAT.

The modes are mutually exclusive. Tokens are neither persisted nor emitted.

The first live read-only snapshot proved authentication, provider parsing,
Watcher execution, Telex acceptance, and state commit. The workstream checkpoint
correctly required a meaningful provider transition rather than only a
snapshot.

The campaign then authorized a disposable test PR whose only repository change
added the approved README description. No comments, reviewers, approvals,
policy overrides, merge, or unrelated state changes were performed.

Watcher emitted:

```text
azure-devops.pull-request.created
```

The event was correlated to:

- the provider PR creation timestamp;
- a stable event ID;
- attempt ID;
- script digest;
- prior/next state hashes;
- normalized envelope hash;
- Telex message `1427`;
- durable `delivered` receipt;
- atomic state/event commit.

The next poll returned `idle` with the same cursor and did not replay the event.

During this exercise, live reviewer objects omitted optional `isRequired`.
Strict property access initially produced `degraded`; state remained empty and
no ledger row was created. The detector now handles omitted optional fields and
Watcher preserves degraded stderr in attempt diagnostics.

## Runtime and process lifecycle

### Scheduling and bounds

- global detector concurrency is configurable and bounded;
- one watch cannot overlap itself;
- detector stdout/stderr are drained concurrently;
- timeout and shutdown terminate process groups/Windows Job Objects;
- failures use bounded exponential backoff;
- overdue watches run once after restart rather than replaying every missed
  cadence and receive deterministic 0-10% interval jitter to avoid a
  simultaneous provider burst.

### Script provenance

Every attempt records the exact executed SHA-256.

- `pinned` mode rejects changed content;
- `follow-path` hashes immediately before execution and again before accepting
  output;
- a mid-run change becomes visible `script-drift` and does not send or commit.

### Environment policy

Detector processes start from a cleared environment plus:

- a minimal platform launch baseline;
- explicitly allowlisted inherited variable names.

Credential values are read at execution time, never stored in registration, and
redacted from bounded stderr when their allowlisted names are token/PAT/key/
secret-like.

### Sender-station lifecycle

One fresh runtime UUID spans all configured sender addresses. Each sender is
attached before use with the Watcher PID as a Telex `required` predicate.

The runtime:

- reconciles senders at startup, on registry revision, periodically, and after
  typed membership loss;
- runs periodic reconciliation independently from detector execution, so a
  long detector does not stall membership repair;
- verifies sender session/PID/role/status after attachment;
- distinguishes touched/attached senders (shutdown compensation) from verified
  senders (ready for use) in lifecycle diagnostics;
- remains non-ready on partial attachment;
- sends with explicit session and sender;
- retries one send after explicit reconciliation;
- detaches all known senders during graceful shutdown;
- never force-takes an address.

Abrupt PID death was observed to release all sender leases and make addresses
unoccupied before a fresh runtime UUID claimed them.

### Restart-record reconciliation

The first abrupt proof runs exposed stale local diagnostics:

- dead runtime rows still marked `running`;
- an unfinished attempt left open.

Startup reconciliation now runs after acquiring the Watcher file lock and before
recording the new runtime. In one SQLite transaction it:

- marks prior running runtimes `interrupted`;
- closes unfinished attempts as `runtime-interrupted` with no state/receipt
  commit;
- increments affected watch failure state;
- delays retry by detector timeout plus failure backoff;
- preserves detector state and sent-event ledger.

The live proof registry confirmed stale runtime/attempt reconciliation and a
fresh PID-bound runtime.

### Graceful detach and daemon restart

Final destructive daemon tests ran on an isolated plane with unique
`TELEX_HOME`, `TELEX_DB`, and `TELEX_INSTALL_ROOT`.

Graceful `run --once` evidence:

- attached a sender with a required Watcher PID;
- completed zero paused-watch runs;
- detached the sender;
- left no daemon member and an unoccupied released lease.

Isolated live-daemon restart evidence:

- Watcher runtime UUID and PID remained unchanged;
- daemon owner instance changed;
- Watcher explicitly reattached the sender;
- required PID predicate remained live;
- sender remained occupied;
- no waiter was introduced.

The isolated Watcher/daemon were stopped and their resolved test root was
deleted after evidence capture.

## Failure behavior

Automated real-process coverage includes:

- malformed JSON;
- result schema mismatch;
- policy fields in detector output;
- stdout/body/metadata/state caps;
- nonzero process exit;
- timeout and descendant termination;
- degraded backoff;
- pinned digest mismatch;
- follow-path drift;
- unknown Telex receipt;
- typed `NeedsAttach` and one reconcile/retry;
- partial multi-sender attachment cleanup;
- shutdown preventing reattachment;
- stale duplicate and event-ID collision;
- terminal event and eventless terminal behavior;
- registry reopen/restart recovery;
- runtime-interrupted attempt reconciliation;
- configuration revision versus attempt-result distinction;
- actual global concurrency cap and per-watch non-overlap.

Validation includes 48 Watcher unit tests, 18 end-to-end behavior tests, 14
protocol/example tests, two root private-send process tests, targeted clippy,
workspace build, and PowerShell detector smoke tests.

The repository-wide all-targets clippy command also reports pre-existing test
lint failures outside this change. Changed Watcher targets and the root Telex
binary pass `-D warnings`.

## Detector-authoring experience

Useful properties:

- stdin/stdout JSON is easy to fixture and replay;
- provider scripts remain independently editable;
- opaque cursor state keeps provider details out of runtime;
- deterministic initial snapshot/created events make read-only proof possible;
- fixed runtime policy prevents accidental rerouting;
- script digest and attempt inspection make iteration auditable.

Friction:

- sample registration requires absolute command/script/working-directory paths;
- PowerShell strict mode requires explicit handling for omitted provider fields;
- credentials require deliberate allowlist configuration;
- provider query shape changes can produce degradation even when authentication
  is healthy;
- authoring a meaningful live transition requires an owned or explicitly
  authorized provider resource.

## Security and trust observations

- Detector scripts are arbitrary trusted local code running with user authority.
- Registration is local CLI/database mutation only; Telex messages cannot
  register executable code.
- Environment clearing reduces accidental credential leakage but is not a
  sandbox.
- Logs and provider error bodies can still reveal sensitive context; public
  reports must redact private coordinates/content.
- Sender stations are send-only in this spike. Inbound messages may be reported
  `delivered` because the address is occupied even though Watcher has no receive/
  ack loop. Receipt occupancy must not be confused with application consumption.
- Stable event IDs expose duplicate risk but do not eliminate the
  accepted-send/local-commit crash window.

## Temporary integration shortcuts

### Spike-private send-once mode

Watcher uses the current Telex executable for attach, status, detach, and send.
The normal `telex send` command auto-registers on membership loss, which would
drop Watcher PID predicates.

The spike adds an explicitly unstable internal environment contract:

```text
TELEX_WATCHER_INTERNAL_SEND_ONCE_V1=<runtime-session-id>
```

It activates only when the value exactly matches explicit `--session`. It is:

- not a Clap flag;
- absent from help/docs;
- not a stable Rust API;
- not a supported SDK/binding;
- tested to leave ordinary send behavior unchanged.

The private path returns the existing typed daemon response without hidden
registration. Watcher owns explicit reconcile and retry.

### Local binary dogfood incident

The initial live proof switched the user's global versioned Telex binary and
restarted the shared local daemon. That disrupted campaign coordination and was
the wrong test boundary.

The stable global `v0.1.0` binary/daemon and implementer bridge were restored.
Subsequent daemon restart/destructive lifecycle evidence used an isolated test
plane. Future node tests must always isolate:

- `TELEX_HOME`;
- `TELEX_DB`;
- `TELEX_INSTALL_ROOT`;
- branch binary path.

## Issue #12 Application Client requirements

The spike exports requirements, not an API design:

1. **Stable service address, ephemeral runtime session.** One application
   process needs a never-reused incarnation identity while stable sender
   responsibilities survive process replacement.
2. **Process-bound liveness.** Application attachment needs typed PID/start-time
   predicates so crash presence becomes idle/unoccupied.
3. **Multi-address lifecycle.** Attach/reconcile/detach should be atomic or
   return explicit partial results and compensation handles.
4. **Strict send recovery policy.** Applications must choose whether
   `NeedsAttach` is returned or automatically repaired; recovery must never
   silently discard liveness predicates.
5. **Typed membership-loss reasons.** Restart loss and deliberate detach must
   remain distinguishable.
6. **Bounded reconcile-and-send.** Applications need one semantic operation
   without parsing CLI stderr or exposing raw daemon IPC.
7. **Explicit sender selection.** Multi-address applications cannot rely on
   ambiguous default-from resolution.
8. **Observable collision/takeover.** No hidden force takeover; expose current
   owner/epoch and bounded retry/reset guidance.
9. **Receipt semantics.** Separate durable acceptance, current occupancy, push
   attempt, recipient consumption, and disposition.
10. **Inbound application semantics.** Sender-only and bidirectional stations
    need explicit contracts. A real receiver needs cursor, receive, ack,
    disposition, and reply APIs.
11. **Lifecycle status.** Expose runtime session, PID predicates, lease epoch,
    idle state, last reconciliation, and detach outcomes.
12. **Daemon restart signal.** Applications need ergonomic explicit
    reattachment or restart notification without a resident waiter.
13. **Dedup guidance.** Document message/event IDs and the
    accepted-send/local-commit duplicate window.

## Decisions and pivots

- Allowed successful `idle` results to advance opaque state; `degraded` does
  not.
- Kept attention and disposition requirement in registration policy.
- Chose fresh per-process session UUID rather than stable session reuse.
- Chose one runtime session for multiple sender addresses.
- Chose `required` Watcher PID liveness rather than anchor-only presence.
- Rejected a public `telex send --require-attached` flag; kept strict send
  spike-private.
- Required live ADO evidence; fixture-only proof remained insufficient.
- Added a dedicated ADO PR-created event after the workstream rejected a generic
  snapshot as the meaningful provider transition.
- Added startup reconciliation after live abrupt-death evidence exposed stale
  diagnostic rows.
- Moved daemon restart testing to an isolated plane after the shared-plane
  coordination incident.

## Known limitations and deferred work

- No production installer/service supervisor.
- No remote registration or administration.
- No script sandbox or signed catalog.
- No general registry migration/backup/compaction administration.
- No production log rotation or automatic degradation notification.
- No multi-host ownership/failover.
- No webhook/GitHub App/Azure Service Hook ingestion.
- No comprehensive provider template compatibility guarantee.
- Abrupt Watcher death can leave detector descendants; startup delay reduces
  overlap risk but production orphan adoption remains deferred.
- Send-only Watcher does not consume inbound service traffic.
- Follow-path mode can reject output during an edit; atomic file replacement is
  recommended.
- Azure DevOps policy/check coverage is intentionally conservative.
- GitHub review-thread comments not returned by the selected `gh pr view` fields
  are outside the example policy.

## Viability-gate handoff

The later builder gate receives:

- runnable experimental Watcher and management CLI;
- generic GitHub/customized GitHub/Azure DevOps templates;
- non-PR local JSON template and registration;
- provider and failure fixtures;
- this report's trust, reliability, and #12 findings.

The non-PR template executes through the real runtime against a local fixture,
produces a protocol-valid event, and replay-suppresses through opaque state.
Its live Telex delivery remains a separate builder dogfood exercise, as planned.
