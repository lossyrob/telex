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
  scheduling, timeout, concurrency, retry, backoff, host-specific process
  cleanup: detached-descendant containment on Windows and Linux, and
  assigned-group containment for conforming non-detaching detectors on macOS,
  output, and logs; fixed-target Telex event emission; receipt-gated state
  commit; runtime-generated event sequence identity; pause/resume/update/delete
  and attempt/state inspection surfaces; small optional examples; restart
  recovery; local SQLite and networked Postgres operation; production packaging
  and troubleshooting.
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

On 2026-10-25 the Watcher code path completed with #179 (PR #180 merged as 0d5347a6). The critical path now runs through the operator's five-minute custom-watch gate; #172 (macOS runtime, HOLD) still blocks product-level usable and closure. Earlier text below is historical.

On 2026-10-25 Campaign and owning Local expressly accepted bounded #181
delivery at #185 merge
`65f5846d50bc4c07b3eaf9ffed0800f4cb19a17b`,
main CI `40217242404` 24/24, Docs `40217242455` 2/2 and the full
[field report](
https://github.com/lossyrob/telex/issues/181#issuecomment-6401070057).
Local's 13,796-byte qualification, SHA-256
`efa69b137efb54b6de605cd72165eb9d407374bdf9df3e17f99dc675813f0f2d`,
independently verifies merged-head provider results and four retained
Windows/macOS captures; they are not aliases of PR CI or hard real-time
guarantees. The existing `watcher-cli-ux` Local181 external edge is
satisfied and retained, not deleted. Owning Watcher separately directs
#179 from blocked/watch to in-progress/focus; Local's later 2,185-byte
supplement qualifies only its earlier preserve-watch instruction.
Owner `c90bafbb-6e49-4ced-a684-e15cfadfc69e`, tracker, PR #180 and completed
runtime-core lineage stay intact. PR #180 retains fresh current-head
source/design/CI/field and Campaign gates; no readiness, merge or accepted
Watcher delivery follows. The older pending/blocked/watch state below is
historical. Local real-time 200ms coverage and unknown-cause debt,
recurrence-as-new-item rule, sentinel/Clippy, marker and publication
dispositions remain separate. No new runtime/CI/observer, product action,
release, #153, hardening/closure or workstation-freeze authority follows.
The owner is not waiting for this publication.

On 2026-10-23 Campaign made the separately owned Local repair
`cooperative-partial-page-repair`
([#181](https://github.com/lossyrob/telex/issues/181)) a pending external
upstream dependency of `watcher-cli-ux`. Delivery of #179 / PR #180 is
blocked until accepted repair delivery with exact-head hosted validation
and Campaign acceptance. PR #180 and owner
`c90bafbb-6e49-4ced-a684-e15cfadfc69e` stay in place; no root repair is
absorbed into Watcher. The completed runtime-core dependency and Watcher's
own current-head review, design, CI and field-proof requirements remain;
none is waived. This dependency changes no Watcher product semantics.

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

[Issue #12](https://github.com/lossyrob/telex/issues/12) publication revision 3,
body SHA-256
`c5c694681a2f3dc2060146fe932c26b3644d1d0518f1406e30c798853c168956`,
preserves `application-client-ready` as the historical design-only checkpoint
and records the supported Rust core merged through
[PR #132](https://github.com/lossyrob/telex/pull/132) as
`4ecbe84e99e00ab0cea3bcf3619d539c222746af`. Issues #129 and #124 are complete.
The publication does not provide a first binding, complete conformance, consumer
integration, or production readiness.

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

The builder accepted `dumb-watcher-contract-gate` as contract-usability
acceptance, completing the `minimal-contract-accepted` checkpoint. The evidence
was the ordinary authoring flow in merged `watcher.md`: command, cadence,
timeout, backend, sender, and target are the only required registration fields;
other generic fields have explicit defaults; and manifests, pinning, kind
allowlists, provider preflight, downtime declarations, and template conformance
are absent from v2 registration/runtime semantics. PR #135 had PAW `+1` review
4825216100 and green CI; the focused design-steward follow-up in PR #141 had
`+1` review 5044005481 and green CI.

This acceptance is not production-runtime or five-minute operational proof.
Issue [#144](https://github.com/lossyrob/telex/issues/144) and
[`tasks/minimal-example-pack.md`](tasks/minimal-example-pack.md) now provide the
fresh tracker and bounded task specification for `minimal-example-pack`. The
node is complete. On 2026-10-22 the operator approved its launch as the first
live merge-gate pilot, and Campaign launched owner session
76a56ac2-dd56-403f-99e7-9eadb3458ffe. Campaign cleared PR #163 at exact head
`fdb4f52f`, and the owner merged it as `5ad7eb04`; issue #144 closed.
`watcher-runtime-core` completed on 2026-10-23 after its closing repair merged
and the owner posted the #162 field report. Its Application Client holds were
discharged: `client-conformance` merged through PR #161 as `4d25fdff` on
2026-10-22, and Watcher's post-merge consumer integration gate receipt passed on
that merged main. Issue [#162](https://github.com/lossyrob/telex/issues/162) and
[`tasks/watcher-runtime-core.md`](tasks/watcher-runtime-core.md) are its
reviewed and reconciled tracker and task specification. On 2026-10-22 the
operator approved launch through the Streamliner workflow v2 adapter, and
Campaign launched owner session 42c1104a-7eb2-477e-a574-2bffb1cd06fe. Its merge
gate launched in shadow mode; Campaign now runs the merge gate and merges at the
exact cleared SHA. Neither launch grants release, tag, publish, fleet, or I161-1
authority. By operator priority decision on 2026-10-22, Telex Watcher is to
replace the waiter session in the operator workflow, and `watcher-runtime-core`
is the intended first owner under the operator workflow v2. The merged core now
provides authoritative exact-store/exact-operation `NotRecorded`,
exact-same-operation retry after that result, and `RetentionBoundaryCrossed`
when absence evidence is no longer available. Watcher's post-merge gate receipt
found the merged bundle aligned with those requirements, including SQLite and
credentialed PostgreSQL parity. The completed internal gate remains a historical
dependency. There is no private-client or mandatory-template fallback, and
`five-minute-custom-watch-gate` remains planned.

## Decisions

- **The detector is generic; the reaction is fixed:** a detector may encode any
  local observation policy, but the Watcher can only emit a normalized Telex
  message.
- **The Watcher is a separate in-repo application:** it may share Telex crates and
  packaging conventions, but it is not part of the core local exchange,
  `telex-console`, or Operator Station process.
- **Scripts are trusted same-user code:** registration is local-only in the first
  product; the Watcher does not claim to sandbox an agent-authored executable.
- **macOS detector support is non-detaching:** Windows and Linux retain
  detached-descendant containment. On macOS, every attempt-scoped local process
  must remain in the Watcher-assigned group; escaped work is outside the cleanup
  guarantee until a separately authorized architecture says otherwise.
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
- **The builder accepted contract usability, not runtime operation:** the
  shortest ordinary authoring path needs only the minimal command, cadence,
  timeout, backend, sender, and target inputs plus explicit defaults, without
  mandatory hardening ceremony.
  `minimal-example-pack` may proceed, while `five-minute-custom-watch-gate`
  still owns proof that a watch can be created, registered, left running, and
  diagnosed in five minutes.
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
- **Authoritative non-acceptance is merged but not yet consumer-proven:** the
  Application Client core provides exact-store/exact-operation `NotRecorded`,
  exact-same-operation retry, and retention-boundary failure. Watcher still
  requires first-binding and `client-conformance` evidence across both backends;
  uncertain sends remain reconciliation-first and query-only until then.
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

## 2026-10-26: Same-head CI reruns supersede #203's launch prohibition

Campaign `toolu_01BAMphx5UxVR6KEHAWxh5Ud` records the operator's
09:25 ET directive: "Reruns are fine." The previous #203 and #167
no-retry/flake-exception clauses are superseded for same-head CI reruns.
Record run, job and test on the existing tracker or discovered-work record
before rerunning failed CI jobs at the unchanged head. Green exact-head
reruns satisfy CI; red merges remain forbidden. Repeated failure of the
same test is a finding, not noise. No check waiver, skip or weakening,
cosmetic-head retry, local runtime or product retry weakening follows.

Existing test sentinel `fc772c8f-2a38-40de-a868-69a5397dbbc5` owns the
cross-family Windows flake/environment investigation and approach decision,
amending R15 for this purpose. #167 B/C and #175/LockFileEx-33 staffing
are deferred to its recommendation; #167 remains held/no merge and those
candidates stay unstaffed. This is not a new owner launch, implementation
assignment, investigation result or recommendation.

Campaign's private planning process-document commit
`74721a829f1dda93d1d2acc869a60fa2266c45f1` updates the owner guide and gate
charter; it does not change telex main. Private document content is not
copied here. The directive and rule summary are the canonical projection.

All existing #203 observations, owner, launch and historical scope remain
exact, with the old no-rerun clause explicitly historical in this respect.
#203 stays outside the graph; no node or dependency is added. No #203/#207
acceptance, CI result, causal finding, #166 debt clearance, new wake,
release, hardening or freeze credit follows. Existing owners, reruns and
Sentinel do not wait for this recording.

## 2026-10-26: #203 completed with retained debt

Campaign `toolu_01HDSNCtTXA8Xp833gAu323W` explicitly accepts the existing
external #203 repair as completed with retained debt after PR #207 merged
at `ee398fcf34b6031e8daaa10b2efdaf6f05db7d72`. The merge verdict is
[gate 6417405039](https://github.com/lossyrob/telex/pull/207#issuecomment-6417405039).
Watcher `toolu_01NcjaCpMWJPym6znkDaWbnJ` requests the existing ledger
record's in-progress-to-completed update. #203 remains out of the graph:
no node or dependency is created, and both graphs stay exact.
Existing owner `b8e0a6cc-6ae3-4c1b-b6f8-db46d47a279f` is unchanged.

All five numbered gate debts remain open with that same owner: the
reverted-fix failing check was local-only; the original 10-second overrun
is unexplained; `cli_v2.rs` has the same drop hazard; transport tests have
unsized three-second timeouts; and a failed test leaves its fixture root.
The full gate text and each numbered block are retained verbatim in the
qualification. The local-reproduction/rerun process debt is also retained:
those runs violated the hosted-only scope and receive no gate credit.
This recording authorizes no additional local reproduction or rerun.

The gate's hosted dump, regression, source-review, timing and exact-head
CI findings remain Campaign-attributed, not independently replayed by
the reconciler. The actual product merge is a separate external boundary
already present in the retained artifact history, not an artifact commit.
No causal equivalence to #166, #167 acceptance, #166 debt clearance,
new staffing or wake follows.

Campaign explicitly says not to wait for the owner's field report before
recording completion. The pending state in the original Watcher request
remains history. Before this proposal was frozen, Campaign
`toolu_01X24xWTf2B7HV1v6ZRfgs8r` and Watcher
`toolu_01X39eUUxFMnrCFuZ8g4XmtP` directed two additive follow-through
entries: the full [owner field report](https://github.com/lossyrob/telex/issues/203#issuecomment-6417840481)
and main push CI `40332382383` at the actual merge `ee398fcf`.
The report is retained in full as owner-attributed source/native/timing
and discovery context, not replayed diagnostics. Its recurrence condition
is to reopen #203 if the Watcher lib harness exits `0xc0000409` again.
The reconciler independently verified the main run completed successfully
on attempt 1 and all 21 jobs are actual successful attempt-1 entries at
that exact merge. No logs, archives, dumps or test runtime were replayed.
This bounded main-CI follow-through does not clear any of the five debts,
the process debt, #166 debts or other repair/acceptance obligations.

All original observations, launch/scope history and the later same-head
CI rerun qualification remain exact. Completion grants no blanket
hardening, release or workstation-freeze credit and does not delay work.

### Later discovery routing: #208 is outside PR #194

Campaign direction toolu_01XptNJYT2dvk1dQoWUgG4VP adds the Sentinel-filed [#208](https://github.com/lossyrob/telex/issues/208) scoped-scan Live-fixture seed failure as separate tracking. [PR #194](https://github.com/lossyrob/telex/pull/194) explicitly states that it does not repair #208. The public issue and PR scope statement were checked; source, failure-path and incidence claims remain attributed. No common cause, new owner or runtime authority, automatic PR expansion, #194 approval or #208 closure follows. The failed attempt remains evidence despite its green rerun. PR #202 is a separate verified product boundary; its #198 completion and debts will be recorded separately.
