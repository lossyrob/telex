# Telex Watcher (external deterministic watch loops)

## Purpose

Create a separately installable, headless Telex application that runs durable
deterministic detector scripts outside agent sessions and sends a Telex message
to a configured address when a condition is met. This removes long-lived polling
and attached waiter tasks from the agent runtime while preserving the flexibility
for agents to author and refine highly specific GitHub, Azure DevOps, and
arbitrary local detectors.

## Approach

The workstream starts with a deliberately generic vertical spike. The daemon
understands only a narrow detector protocol: execute a trusted local command on
a schedule, provide its prior opaque state, validate the structured result, send
any reported event to the watch's fixed Telex address, and commit the detector's
next state only after Telex returns a durable send receipt. The daemon does not
understand GitHub, Azure DevOps, PR policy, or arbitrary trigger actions.

The spike must demonstrate this contract with both GitHub and Azure DevOps
detectors, including at least one detector customized for repository-specific
author or comment filtering. A builder viability gate then evaluates whether
agents can create and improve detectors quickly and whether external events
reliably wake the responsible Telex address without leaving any background task
inside the session.

If the gate passes, a contract node promotes the successful experimental
semantics into production design. Production implementation consumes the shared
Telex Application Client seam tracked in
[#12](https://github.com/lossyrob/telex/issues/12), alongside the Operator Station
workstream. The Watcher runtime and detector-template library can then advance
under that accepted contract before operational hardening and closure.

The richer rationale and detector protocol sketch are preserved in
[`docs/initial-shaping.md`](docs/initial-shaping.md).

## Design References

- `telex:docs/design/index.md` - entry point for Telex's intended-system design.
- `telex:PRODUCT-THESIS.md` - durable responsibilities, store-and-forward
  delivery, and Telex's boundary against workflow execution.
- `telex:docs/design/daemon.md` - local-exchange lifecycle and durable send
  semantics the Watcher uses rather than reimplementing.
- `telex:docs/design/DESIGN.md` - the explicit boundary that Telex, not generic
  loop skills, owns long-duration message delivery and agent wakeup.
- `telex:docs/design/proposals/EXTENSIONS.md` - namespaced message kinds and
  opaque metadata conventions for normalized watch events.
- `telex:.streamliner/workstreams/operator-station/brief.md` - sibling campaign
  workstream and consumer of the shared application-client seam.

## Boundaries

- **In scope:** a per-user headless Watcher process; trusted local detector
  commands; persistent watch registration and opaque detector state; bounded
  scheduling, timeout, concurrency, retry, backoff, and logs; fixed-target Telex
  event emission; pause/resume/update/delete inspection surfaces; pinned and
  development-friendly script lifecycle; GitHub and Azure DevOps detector
  examples; restart recovery; local SQLite and networked Postgres operation;
  production packaging and troubleshooting.
- **Out of scope:** general cron or workflow automation; arbitrary post-trigger
  actions; interpreting provider semantics in the Watcher runtime; running
  scripts inside the Telex local exchange or Operator Station; remote
  message-driven registration of executable code; session/process supervision;
  hosted webhook infrastructure; replacing Telex delivery, disposition, or
  attention semantics.
- **Deferred:** signed or remotely distributed detector catalogs; OS sandboxing
  beyond same-user trusted-local execution; webhook/GitHub App ingestion;
  multi-host ownership and failover of one watch; a rich Watcher UI; remote
  administration; cross-principal authorization beyond the selected Telex
  backend's trust model.

## Current State

The workstream is part of the
**[Addressable Attention campaign #102](https://github.com/lossyrob/telex/issues/102)**
documented in `.streamliner/shaping/roadmap.md`. It is formed under parent issue
[#100](https://github.com/lossyrob/telex/issues/100). The first executable node is
the `generic-watcher-spike`, tracked by
[#101](https://github.com/lossyrob/telex/issues/101), which proves the detector
contract, persistent external runtime, Telex event transaction, and
agent-authored GitHub/Azure DevOps examples before production architecture is
accepted.

The existing Lossyrob Loop scripts are reference implementations for domain
checks and event-state reasoning, but their session-owned worker plus attached
waiter lifecycle is explicitly not reused. The shared production application
client remains a campaign-level seam through issue #12 and must not be
independently designed inside this workstream.

## Decisions

- **The detector is generic; the reaction is fixed:** a detector may encode any
  local observation policy, but the Watcher can only emit a normalized Telex
  message.
- **The Watcher is a separate in-repo application:** it may share Telex crates and
  packaging conventions, but it is not part of the core local exchange,
  `telex-console`, or Operator Station process.
- **Scripts are trusted same-user code:** registration is local-only in the first
  product; the Watcher does not claim to sandbox an agent-authored executable.
- **Detector output is structured, not exit-code folklore:** process exit status
  reports execution success/failure; a versioned JSON result reports
  `idle`, `event`, `terminal`, or `degraded`, plus opaque next state and optional
  normalized event content.
- **Target and sender are registration policy:** detector output cannot silently
  reroute messages or impersonate another Telex address.
- **State follows the Telex receipt:** event-producing next state is committed
  only after the message is durably accepted by Telex. A failed send leaves the
  prior state available for retry.
- **At-least-once is the safe failure direction:** every event carries a stable
  detector event ID and watch ID for deduplication and audit; a narrow duplicate
  is preferable to silent loss.
- **Templates demonstrate the protocol rather than define providers:** GitHub and
  Azure DevOps examples are editable starting points that agents can specialize.
- **Experimental integration does not set the application-client contract:** the
  spike may call the CLI or current Rust library; production work consumes the
  campaign's shared #12 checkpoint.

## Open Questions

- What exact JSON input/output envelope gives detectors enough context while
  keeping Watcher-owned routing, credentials, and execution policy authoritative?
- Should eventless `nextState` commits be allowed on every successful poll, and
  how should a detector distinguish ignored observations from unseen work?
- What are the precise semantics of pinned scripts versus a development
  `follow-path` mode, and how is the executed content digest recorded?
- How are credentials exposed safely to a detector: inherited CLI authentication,
  explicit environment allowlists, or named command wrappers?
- Which watch lifecycles are required initially: single-event, until-terminal,
  explicit cancellation, and/or address-bound expiration?
- When a detector repeatedly degrades, should Watcher notify the target address,
  a separate operator address, or only expose local diagnostics?
- What application-client shape from #12 best serves both a headless service and
  the Operator Station without coupling either product to internal daemon IPC?

## Imports and Exports

### Imports

- Telex local-exchange startup, sender membership, durable send receipts,
  address routing, attention levels, metadata, and Postgres support.
- Existing Loop skill detector logic and tests as domain examples, excluding its
  owner-bound worker and attached waiter runtime.
- The shared Telex Application Client contract from issue #12 after the viability
  gate.

### Exports

- A demonstrated generic detector protocol and reliable external watch loop.
- GitHub and Azure DevOps detector templates suitable for rapid agent
  customization.
- A separately installable Telex Watcher application that can target any durable
  Telex address.
- Normalized, provenance-rich watch event conventions consumable by agent
  sessions, operator agents, and Operator Station.
- Operational evidence about application-client needs shared with issue #12 and
  the Operator Station campaign workstream.

## Closeout Observations

Parking lot for bounded detector templates, diagnostics, CLI ergonomics, and
polling-policy improvements discovered during dogfooding. Any expansion into
general automation, remote executable registration, hosted event ingestion, or
cross-principal script trust belongs in its own issue, candidate, or follow-on
workstream.
