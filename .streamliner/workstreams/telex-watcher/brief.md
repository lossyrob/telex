# Telex Watcher (external deterministic watch loops)

## Purpose

Create a separately installable, headless Telex application that runs durable
deterministic detector scripts outside agent sessions and sends a Telex message
to a configured address when a condition is met. This removes long-lived polling
and attached waiter tasks from the agent runtime while preserving the flexibility
for agents to author and refine highly specific GitHub, Azure DevOps, and
arbitrary local detectors.

## Approach

The workstream began with a deliberately generic vertical spike. The runtime
understands only a narrow detector protocol: execute a trusted local command on
a schedule, provide its prior opaque state, validate the structured result, send
any reported event to the watch's fixed Telex address, and commit the detector's
next state only after Telex returns a durable send receipt. The runtime does not
understand GitHub, Azure DevOps, PR policy, or arbitrary trigger actions.

The spike landed in [PR #105](https://github.com/lossyrob/telex/pull/105) and is
documented in [`docs/generic-watcher-spike-report.md`](../../../docs/generic-watcher-spike-report.md).
It demonstrates the contract with GitHub and Azure DevOps detectors, including
repository-specific author/comment filtering. The builder passed the viability
gate after scoped post-merge dogfood confirmed useful, low-noise PR supervision
without a session-owned Loop task.

The `watcher-contract` node landed the accepted production design in
[PR #115](https://github.com/lossyrob/telex/pull/115). Production implementation
consumes the shared Telex Application Client seam tracked in
[#12](https://github.com/lossyrob/telex/issues/12), alongside the Operator Station
workstream. The Watcher runtime and detector-template library can then advance
under that accepted contract before operational hardening and closure.

The richer rationale and detector protocol sketch are preserved in
[`docs/initial-shaping.md`](docs/initial-shaping.md).

## Design References

- `telex:docs/design/index.md` - entry point for Telex's intended-system design.
- `telex:docs/design/watcher.md` - normative production Watcher contract.
- `telex:docs/design/DECISIONS.md` - ADR 0046 records the load-bearing
  provider-neutral, trusted-local, receipt-gated architecture.
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
and is tracked by parent issue
[#100](https://github.com/lossyrob/telex/issues/100). The
`generic-watcher-spike` completed through
[#101](https://github.com/lossyrob/telex/issues/101) and
[PR #105](https://github.com/lossyrob/telex/pull/105). The experimental runtime
proved the provider-neutral detector contract, receipt-gated state transaction,
PID-bound sender lifecycle, occupied Copilot wakeup, durable unoccupied queueing,
and editable generic GitHub, customized GitHub, Azure DevOps, and non-PR
templates.

The builder passed `viability-gate` after a scoped Watcher-backed PR lifecycle
dogfood on Operator Station PR #104. The shared runtime detected the merged PR in
about 26 seconds, emitted one baseline snapshot and one merge event, produced no
duplicates or noise, agreed with the canonical checker, required no fallback,
removed the watch cleanly, and remained live for reuse.

The `watcher-contract` completed through
[#110](https://github.com/lossyrob/telex/issues/110) and
[PR #115](https://github.com/lossyrob/telex/pull/115). The merged design adds
`docs/design/watcher.md`, four canonical v1 schemas, and ADR 0046. The exact
Watcher shared-client requirements were dual-approved and published to
[issue #12](https://github.com/lossyrob/telex/issues/12#issuecomment-5042702401).

No Watcher implementation node is launch-ready yet. `watcher-runtime` and
`detector-template-library` are planned with resolvable dependencies on
`telex/application-client/application-client-ready-gate`. Application Client
workstream [#117](https://github.com/lossyrob/telex/issues/117) is active and
contract-convergence node [#118](https://github.com/lossyrob/telex/issues/118)
is the current shared work. Campaign/#12 must disposition the requirements and
complete that gate before either Watcher node can become ready. There is no
spike-private fallback.

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
- **Destructive Telex tests use an isolated plane:** daemon restart, failure, and
  upgrade evidence must use unique absolute `TELEX_HOME`, `TELEX_DB`, and
  `TELEX_INSTALL_ROOT` values plus the absolute worktree binary. The default
  local daemon and installed launcher are campaign coordination infrastructure.
- **Live provider mutation requires explicit authority:** credentials and
  coordinates are not permission to mutate a provider resource. A meaningful
  transition must use an owned or explicitly authorized disposable resource.
- **External detector hosting is viable:** builder dogfood confirmed that a
  shared Watcher runtime can replace a session-owned PR sentry loop for scoped
  supervision with timely, low-noise Telex delivery and clean watch lifecycle.
  Production semantics still require the contract and shared-client gates.
- **The production Watcher domain contract is accepted:** `docs/design/watcher.md`,
  its four canonical schemas, and ADR 0046 govern downstream runtime/template
  work. Intentional changes require normal design/decision updates.
- **There is no private Application Client fallback:** production Watcher nodes
  wait for #12/campaign convergence. CLI subprocess parsing, raw daemon IPC,
  `TELEX_WATCHER_INTERNAL_SEND_ONCE_V1`, and sender occupancy are not accepted
  production client seams.

## Open Questions

- Which of the 15 Watcher shared-client requirements will #12 accept, defer, or
  reject, and which dispositions block runtime versus template promotion?
- When `application-client-ready` exists, should `watcher-runtime` and
  `detector-template-library` be promoted in parallel or staged around a shared
  conformance harness?

## Imports and Exports

### Imports

- Telex local-exchange startup, sender membership, durable send receipts,
  address routing, attention levels, metadata, and Postgres support.
- Existing Loop skill detector logic and tests as domain examples, excluding its
  owner-bound worker and attached waiter runtime.
- The `telex/application-client/application-client-ready-gate` export from
  Application Client workstream #117 after contract convergence #118 and #12
  dispositions.

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
- The accepted production Watcher contract in `docs/design/watcher.md`, ADR 0046,
  and canonical request/result/event-metadata/health schemas.

## Closeout Observations

- Test-support helper packaging is promoted into the accepted Watcher contract:
  `watcher-runtime` must prove the default production package excludes
  `fake_detector` and `fake_telex` while keeping the product crate top-level.
- PAW PR-sentry bootstrap must run a terminal PR-state check immediately before
  adding state/activity watches. A PR can merge during detector-bundle and
  credential preflight; merged/closed state should skip registration rather than
  leave stale watches or start a Loop fallback.

Continue parking bounded detector-template, diagnostics, CLI, and polling-policy
improvements here during dogfooding. Any expansion into general automation,
remote executable registration, hosted event ingestion, or cross-principal
script trust belongs in its own issue, candidate, or follow-on workstream.
