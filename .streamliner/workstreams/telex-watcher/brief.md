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

The `watcher-contract` node landed the first production design in
[PR #115](https://github.com/lossyrob/telex/pull/115). Its provider-neutral,
trusted-local, fixed-route, receipt-gated architecture remains a durable input,
but operator dogfood later rejected its opinionated authoring and registration
ceremony. The next confidence transition is therefore a minimal-authoring
contract that supersedes the pinning, manifest, allowed-kind, preflight, and
downtime requirements while preserving the core Watcher boundary.

After that contract is accepted, runtime work consumes the supported Telex
Application Client seam tracked in [#12](https://github.com/lossyrob/telex/issues/12).
Minimal examples may proceed from the new contract without waiting for runtime
implementation.

The richer rationale and detector protocol sketch are preserved in
[`docs/initial-shaping.md`](docs/initial-shaping.md).

## Design References

- `telex:.streamliner/workstreams/telex-watcher/design/current-design.md` -
  canonical integrated workstream design, boundaries, dependencies, and open
  questions.
- `telex:docs/design/index.md` - entry point for Telex's intended-system design.
- `telex:docs/design/watcher.md` - normative production Watcher contract.
- `telex:docs/design/DECISIONS.md` - ADR 0046 records the retained
  provider-neutral, trusted-local, receipt-gated architecture; ADR 0050 records
  the minimal v2 authoring and runtime-owned identity direction.
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
  commands; minimal persistent registration and opaque detector state; bounded
  scheduling, timeout, concurrency, retry, backoff, process cleanup, output, and
  logs; fixed-target Telex event emission; receipt-gated state commit;
  runtime-generated event sequence identity; pause/resume/update/delete and
  attempt/state inspection surfaces; small optional examples; restart recovery;
  local SQLite and networked Postgres operation; production packaging and
  troubleshooting.
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
  backend's trust model; optional pinning/digest guards, manifests, kind
  allowlists, provider preflight, downtime budgets, and deep hardening recipes.

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

Application Client contract convergence completed through
[#118](https://github.com/lossyrob/telex/issues/118) and clean replacement
[PR #126](https://github.com/lossyrob/telex/pull/126), merged at
`62c2b23cc3d54877226f46df44d6036b7dffa380`. Polluted
[PR #123](https://github.com/lossyrob/telex/pull/123) is closed unmerged at
`8b388a3c65a5bf804d6b8d5b43334047fa92ceb2` and remains preserved only as
protocol-forensics evidence; its old approval and merge-floor records are not
authority.

[Issue #12](https://github.com/lossyrob/telex/issues/12) now publishes
`application-client-ready` as a design-only semantic checkpoint. It accepts all
15 Watcher requirements and unlocks detailed downstream planning, but it does
not mean a supported client core, first binding, conformance suite, or consumer
integration exists. Its supporting traceability link still points at the
pre-repair `docs/design/application-client-crosswalk.md`; Application Client
orchestration owns updating that link to
`docs/notes/application-client/requirements-crosswalk.md`. Non-blocking wording
alignment remains tracked in
[#124](https://github.com/lossyrob/telex/issues/124).

The first detector-template implementation was promoted through
[#127](https://github.com/lossyrob/telex/issues/127) and
[PR #131](https://github.com/lossyrob/telex/pull/131). The implementation was
technically reviewed and merge-ready, but operator product feedback rejected
its mandatory template-framework boundary. Both tracker and PR were closed
without merge on 2026-07-30 as superseded, not as an implementation-defect
finding. The branch, reviews, repairs, fixtures, and conformance evidence remain
preserved as examples and implementation learning.

The approved direction is now a deliberately dumb Watcher: registration names
an agent-authored command plus minimal execution and fixed-routing policy; the
script owns provider semantics, event kind/content, provider cursor/replay, and
optional hardening. Watcher owns generic lifecycle, bounds, diagnostics, opaque
state, durable Telex delivery, receipt-gated commit, and runtime-generated event
sequence identity.

`minimal-watcher-authoring-contract` completed through
[#133](https://github.com/lossyrob/telex/issues/133) and
[PR #135](https://github.com/lossyrob/telex/pull/135), merged as
`b91e8301899351c0411d6e2e9ac5290af8a3cb4c`. The merge promoted the minimal v2
registration/request/result schemas, runtime-owned event identity, and ADR 0050
as project design authority. The canonical integrated workstream design now
lives in [`design/current-design.md`](design/current-design.md).

The builder-owned `dumb-watcher-contract-gate` remains planned and is the next
Watcher decision. Its acceptance is not implied by the contract merge.
`minimal-example-pack` remains blocked on that gate. `watcher-runtime-core`
remains blocked on both that gate and Application Client `client-conformance`,
including promotion of authoritative exact-store/exact-operation
`not-recorded`. There is no private-client or mandatory-template fallback.

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
- **The minimal v2 production Watcher contract is accepted:**
  `docs/design/watcher.md`, the three canonical v2 schemas, retained ADR 0046,
  and superseding ADR 0050 govern downstream runtime/example work. Intentional
  changes require normal design/decision updates.
- **The original contract is now a historical input, not current authoring
  direction:** provider-neutral trusted-local execution, fixed routing,
  structured results, receipt-gated state, diagnostics, and no workflow actions
  are retained. ADR 0050 supersedes mandatory script pinning,
  digests, manifests, event-kind allowlists, provider preflight, downtime
  declarations, and template conformance as ordinary registration requirements.
- **Watcher generates event identity:** runtime persists a per-watch committed
  event sequence so retries retain one ID and later recurrences receive new IDs;
  agent scripts do not implement distributed retry/occurrence identity.
- **Examples are optional teaching material:** agents may copy an example or
  write a script from scratch. Fixtures, tests, manifests, pinning, and provider
  hardening remain user/project choices rather than product ceremony.
- **There is no private Application Client fallback:** production Watcher nodes
  wait for #12/campaign convergence. CLI subprocess parsing, raw daemon IPC,
  `TELEX_WATCHER_INTERNAL_SEND_ONCE_V1`, and sender occupancy are not accepted
  production client seams.
- **Authoritative non-acceptance remains shared-client work:** the Application
  Client must promote exact-store/exact-operation `not-recorded` and prove
  identity-checkable same-operation retry through `client-conformance`; uncertain
  Watcher sends remain query-only and blocked until then.
- **`application-client-ready` is design-only:** it permits detailed node
  promotion under the accepted semantics but does not satisfy
  `watcher-runtime`'s dependency on an implemented, conformant supported client.
- **Node launches use the Streamliner launch broker:** future implementer and
  reviewer sessions use the graph-configured Watcher v2 prompt profiles through
  `POST /api/launch-preparations/runs`; no hand-written node terminal prompts or
  direct terminal helper launches are permitted.
- **Workers execute product missions, not workstream gates:** gate, approval,
  reconciliation, and evidence state remains orchestration-owned and off product
  PR branches unless a node explicitly names an artifact as a product
  deliverable.

## Open Questions

- Is an explicit v1 compatibility adapter worth carrying for the experimental
  spike, or is documented re-registration sufficient?
- Which bounded parts of PR #131 should be extracted later into the minimal
  example pack or optional hardening recipes?

## Imports and Exports

### Imports

- Telex local-exchange startup, sender membership, durable send receipts,
  address routing, attention levels, metadata, and Postgres support.
- Existing Loop skill detector logic and tests as domain examples, excluding its
  owner-bound worker and attached waiter runtime.
- The `telex/application-client/client-conformance` export from Application
  Client workstream #117 before production runtime integration.

### Exports

- A demonstrated generic detector protocol and reliable external watch loop.
- Small optional GitHub, Azure DevOps, HTTP/JSON, file, and command examples
  suitable for rapid agent customization without a mandatory framework.
- A separately installable Telex Watcher application that can target any durable
  Telex address.
- Normalized, provenance-rich watch event conventions consumable by agent
  sessions, operator agents, and Operator Station.
- Operational evidence about application-client needs shared with issue #12 and
  the Operator Station campaign workstream.
- The accepted production Watcher contract in `docs/design/watcher.md`, ADR 0046,
  its superseding minimal-authoring ADR, and canonical v2
  registration/request/result schemas.

## Closeout Observations

- Test-support helper packaging is promoted into the accepted Watcher contract:
  `watcher-runtime` must prove the default production package excludes
  `fake_detector` and `fake_telex` while keeping the product crate top-level.
- PR #131's provider ordering, bounded metadata, recurrence identity, and
  cross-platform fixes remain candidates for optional examples or
  library-maintainer tests, not ordinary registration requirements.

Continue parking bounded detector-template, diagnostics, CLI, and polling-policy
improvements here during dogfooding. Any expansion into general automation,
remote executable registration, hosted event ingestion, or cross-principal
script trust belongs in its own issue, candidate, or follow-on workstream.
