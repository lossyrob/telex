# Telex Watcher (external deterministic watch loops)

## Current workstation run policy (2026-10-26)

At 12:44 ET the operator lifted the I161-1 blanket workstation freeze.
Local builds, tests and runtime runs are allowed within the task's authorized
scope. Use isolated `TELEX_*` roots and a temporary `LOCALAPPDATA`, except for
the separately authorized #201 dogfood against the real installation. That
exception permits real-installation use, not real-directory ACL or permission
changes. I161-1 preservation remains: no ACL restoration or reset.

Exact-head hosted CI remains the gate evidence. Local results support it; they
do not replace it or retroactively change earlier validation/debt dispositions.
Older freeze, compile-only and hosted-only records below remain historical,
including older sections labelled current; they do not renew a blanket local
run prohibition. Other scope, dependency, merge, incident and retained-debt
decisions are unchanged. This policy does not launch #201, satisfy its
dependencies or grant unrelated runtime authority.

Authority: Campaign `toolu_01JDdrjgTsK2e181xhhLdeYY`, relaying the operator:
"Also what is the 'freeze'? If it's blocking then remove it." On this operator's
workstation, the existing `gh-pub` profile now resolves to `lossyrob`; personal
GitHub writes can use it directly without a token override.


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

## Accepted #198 and #199 completions, with linked #210 debt follow-up

Campaign direction `toolu_01XptNJYT2dvk1dQoWUgG4VP` and owning Watcher direction `toolu_01SWvBiLjvgsswJcyDUYbCNa` accept the existing classifier node completed/watch after [PR #202](https://github.com/lossyrob/telex/pull/202) merged as `d8a382a4961b5246f6ca1aca28dae7d152e9e310` from reviewed head `8526358acaeaaec4a582247c90fbcd610e8d2b49`; #198 is closed/completed. Owner `a96926b1-3e4f-4578-a197-33de51d669c7` is unchanged. All **seven numbered debts** and the separate local-run process debt remain **retained-open under that owner**. The full gate below is retained verbatim. Its source/design/per-test conclusions are Campaign-attributed, not new review or local runtime credit. Public CI metadata independently confirms run 40332914834 attempt 1 and all 21 actual successful jobs at the reviewed head.

Completion did not wait for the field report. The owner originally reported main CI 40337145991 and Docs 40337146027 registered with the sole wait coordinator; that pending snapshot remains history; later hosted main/Docs results and the field report are recorded below. No downstream detector validation is inferred.

### Full public merge-with-debt gate

[Gate 6418110367](https://github.com/lossyrob/telex/pull/202#issuecomment-6418110367), 2399 bytes, SHA-256 `9b221934f4829608e6264467725be404fc179e5927869a0d187866fe3782ffed`:

```text
## Campaign merge gate: merge + debt (high)

Cleared at `8526358acaeaaec4a582247c90fbcd610e8d2b49`. Campaign merges at this exact head. There is no fork and no hard stop: the new read-only CLI is consumed only by #199 in the same workstream, and the steward recorded it in ADR 0056.

**Floor checks**
- **CI:** run 40332914834, attempt 1, at the exact head: 21/21 jobs succeeded.
  - Classifier tests ran on hosted CI: 12 lib + 5 CLI on Windows (job 121606731034), and 12 lib + 4 CLI on Ubuntu (job 121606731239).
  - fmt and Clippy `-D warnings` are green.
- **Source review at the head:** issuecomment-6417546245. The delta from the receipted head 09e16662 is merge-only: the PR patch relative to main is byte-identical (`ee398fcf..8526358a` == `8d7815a8..09e16662`).
- **Design:** passed at the head, issuecomment-6417506721 (ADR 0056, no number collision).
- **PR state:** Closes #198; consequence low; 0 unresolved threads; MERGEABLE/CLEAN.
- **Main drift:** since ee398fcf, only `.streamliner/` files changed. No file overlap with open PRs #206 or #194.

**Process debt (no gate credit):** these local runs on the operator workstation get no credit:
- the owner's test runs (6416990044);
- the reviewers' test runs (6417094335 and 6403243261);
- the local real-log sample and async-shell probe.

No acceptance claim rests on them alone. Hosted CI covers every classifier test in the diff.

**Debt (non-blocking)**
1. The body's Validation section cites stale local counts instead of the hosted run.
2. The body has no explicit Discoveries or Deferred work sections. The reconciler should add discovered-work rows for the async-shell limit and for `session.start`/`session.error` returning unknown (the steward routed both, 6402758809 and 6402777694).
3. In `tests/copilot_session_state.rs`, the `Drop` unwraps `remove_dir_all`. This is the same double-panic hazard as #203.
4. On Unix, `open()` on a FIFO named `events.jsonl` could block (same-user only).
5. When the 256 KiB tail starts exactly on a line boundary, a complete first record is dropped. This is conservative.
6. The read-denied test runs on Windows only, and there is no hosted macOS classifier run.
7. Review 5645279522 is still pending: stale and unsubmitted.

**Deferred:** #199 detector (background-work observability and `session.error` handling), #200 guidance, #201 validation. The #198 field report is due after main CI.
```

### Two discovered limitations remain in existing downstream scope

The ledger adds exactly two routed/accepted `absorb` rows with #199 as home: background-shell observability and persistent `session.start`/`session.error` unknowns. The full public steward decisions [6402758809](https://github.com/lossyrob/telex/issues/197#issuecomment-6402758809) and [6402777694](https://github.com/lossyrob/telex/issues/199#issuecomment-6402777694) are retained in the rows. #200 guidance and #201 staged validation retain their stated responsibilities. These two discovery rows create no new task, edge or tracker, #198 scope expansion, fleet validation or debt clearance. The separately scoped third row records already-authorized out-of-graph #210 below.

### Historical #199 launch and PR chronology; current completion follows

Owning Watcher source `toolu_0115L6LMmGsMPkRtpWCkbMVL` reports owner `65da0181-23ea-4b66-8a43-d0ddf4a4f495` launched at d8a382a4 under Campaign staffing direction `toolu_01CoaUfCmhkCHm3Nr2dNGZKL`. The final launched binding is 3,087 bytes/SHA-256 `dda7cd1ad4a734e8feb79abe7046e45979aadc6f24f6afbd6add09851ff6a7bf`. The original held file was updated in place; no frozen held copy existed and none is invented. Kickoff is 14,149 bytes/SHA-256 `b1c29b4a684eb310f3ee4bf6b2d9b307e50ef5b8a3e90ca1cb371726353f56f2`; it byte-matches `handoff.kickoffPrompt` and ends with the 12,818-byte trusted workflow instructions (SHA-256 `1351ece4a339b5381cb84af4e71cb90dca92cb7cbc3b68a8180821f0e1eb0019`). Context ID is `ctx-20261026141548-17e94d3c`. Only identities and directive summaries are projected; private absolute paths and prompt payload remain private. Watcher reports App admission ("Message accepted for priority delivery to session '#199 - copilot group stall detector'"); no standalone delivery receipt file was retained and no recipient was inspected or messaged.

Later owning work-start direction `toolu_017twKAC5TWkCdRgPLCW8Mab` and public [comment 6418371548](https://github.com/lossyrob/telex/issues/199#issuecomment-6418371548) support in-progress/focus, not launch alone. Watcher attributes the comment runtime ID `68c7919d-a892-4b9b-8365-ea6b054163b9` to the same App owner. The original branch `lossyrob-special-funicular` and reported rename to `lossyrob-copilot-group-stall-detector` refer to the same checkout, not a rebind. Later owner source `call_sYPSAiw3NgAgfRwrbhXcWF8C` reports draft [PR #211](https://github.com/lossyrob/telex/pull/211), Closes #199, first at `dfb4e81d1d7c1072603d574113a46deb79454d09`; owning Watcher `toolu_013ctZu5sYdSt56YRrGxnRko` confirms it. That first reported head and the later public draft capture are historical, not a promise the head remains current. Hosted CI, independent review and exact-head design were pending at that owner report. Later owner `call_PsEMUDBGCDZU3wkUD6OLDf4O` reports [readiness for the operator](https://github.com/lossyrob/telex/pull/211#issuecomment-6419673974), not merge, at `b345cd544cebe87e72ad2e5f28fbe43c82ff3b04`; exact-head gate success remains attributed. Latest captured owner `call_baIMLk6pdsyxVHXhsql1dBpI` explicitly supersedes that readiness: #211 returned to draft at merge-only head `cc7cfc9f7fc6a136ca8c32b47059ac6644eac3fd`, with fresh CI pending at that report. Byte-identical feature files are owner-attributed, not inspected here; no earlier gate carries. Merge observation, exact-main CI and field report remain with that owner. No implementation/CI/readiness acceptance is claimed. The workstation freeze, hosted-only runtime validation and recorded same-head rerun rules remain in force. Steward answers 6418386710 and 6418380723 are context only; they create no extra graph change.

### Later main-attempt-1 cancellation and Docs follow-through

Owner source `call_l6XpG7do3ej1boYhky2A8R2w` records the one cancelled macOS station-intent job 121621658860 in CI 40337145991 attempt 1 at d8a382a4; the other 20 jobs and Docs passed. [Tracker disposition 6418567459](https://github.com/lossyrob/telex/issues/198#issuecomment-6418567459) precedes the reported same-head job-only rerun. No steps or failing test were identified; the cause is unknown, not classifier failure or a reason to reopen #198. That validation-in-progress report remains historical; the later aggregate result is green. No new issue is created for this unexplained one-off cancellation, and this reconciler triggers no rerun.

### Later hosted main validation and chronological field-report correction

Owner `call_BUHeLX87GxCdtFSK7VJFhjJp` and owning Watcher `toolu_01PuGPH5BSwZ5XyWDysh8Udm` report main CI 40337145991 attempt 2 green at the unchanged PR #202 merge, with Docs 40337146027 attempt 1 successful. Independently checked metadata shows 21 green aggregate entries. Although all IDs are new and every entry is labelled `run_attempt=2`, paired name/head/start/end/status/conclusion and complete step receipts show **one macOS station-intent re-execution** (121621658860 -> 121632559472) and **20 reused successful executions**. Labels and IDs alone are not proof of execution; no runtime implementation or causal repair is inferred.

The original field capture is retained privately byte-exact: 6,327 bytes/SHA-256 `3b1e5a450df2c0a41ec170ea8ded46cb1c25a77e8e342a35cbe99ac66bd88e69`, posted 14:39:06Z. It is historical capture identity, not the mutable live-body hash. Owner correction `call_pOYju00xzxlGEvipXOkWeToT` distinguishes the requested single-job endpoint from observed API metadata and records [history correction 6418935063](https://github.com/lossyrob/telex/issues/198#issuecomment-6418935063). The revised live field at 14:45:47Z is 6,996 bytes/SHA-256 `64770e3a35aadcd247341e88f4a10779d8e6badde3132d55465de105a3d6e250`. The reconciler also withdraws its earlier inference that new IDs contradicted reuse; the complete paired receipts, not labels, support the bounded one-rerun/20-reuse conclusion. The field follows with CRLF normalized to LF for display (6876 bytes/SHA-256 `a5c83020ee7de59e55b42d5e3165e053c5f8718e759c3c96d88bb3fddca75694`); the raw source capture remains byte-exact. Per-test/log/source and updated-docs conclusions remain owner-attributed, not replayed evidence or debt clearance.

[Field 6418777133](https://github.com/lossyrob/telex/issues/198#issuecomment-6418777133):

```text
## Field report: classifier merged and validated on main

Correction (2026-10-26): the original report said other successful jobs were
"retained, not rerun." That was an unsupported inference. The owner requested
only the cancelled job's rerun endpoint; GitHub's attempt-2 metadata instead
contains 21 new successful job IDs, all explicitly run_attempt=2, with none
shared with attempt 1. This correction does not infer runtime reuse or replay
from those IDs. Completion, accepted debts and geometry are unchanged.

PR #202 merged as d8a382a4961b5246f6ca1aca28dae7d152e9e310 and closed #198.
The read-only `telex copilot session-state --session <local-cli-id>` command
and its design record are in main.

**Consequence: low.** The command is opt-in, same-user and read-only. It
does not change delivery or Watcher runtime behavior. It returns five states,
a safe reason and a non-accounting activity marker, using at most 256 KiB
of the event-log suffix. Unknown evidence is not idle. A successful allowing
root stop remains decisive under the accepted bounded-suffix contract.

### Hosted validation at the exact merge

Main CI run 40337145991 is green in aggregate at attempt 2, with all 21 jobs
successful at d8a382a4961b5246f6ca1aca28dae7d152e9e310:
https://github.com/lossyrob/telex/actions/runs/40337145991/attempts/2

Attempt 1 passed 20 jobs; macOS Station-intent job 121621658860 was cancelled
with no steps or failing test identified. Its cause remains unknown. The
finding was recorded before the job-only same-head rerun:
https://github.com/lossyrob/telex/issues/198#issuecomment-6418567459
The owner requested the job-only rerun endpoint for 121621658860. Attempt 2
reports all 21 jobs successful under new IDs, including macOS replacement
121632559472, Windows Build 121632642117 and Ubuntu Build 121632561576.
Every entry has run_attempt=2; no job ID is shared with attempt 1.
Docs run 40337146027 attempt 1 also passed at the same merge:
https://github.com/lossyrob/telex/actions/runs/40337146027

The main attempt-2 Build and test logs show 12 classifier-related
library/parser tests passing on both Windows and Ubuntu, plus all 5 Windows
CLI tests and all 4 Ubuntu CLI tests:
- Windows: https://github.com/lossyrob/telex/actions/runs/40337145991/job/121632642117
- Ubuntu: https://github.com/lossyrob/telex/actions/runs/40337145991/job/121632561576

These cover five-state fixtures, malformed/incomplete evidence, explicit
allowing root stops, parallel tool pairing, late invalidated stop completion,
activity markers, safe session ids, read-only CLI behavior and the exact
262144/262145-byte boundary. Read-denied coverage is Windows-only.
The source and design receipts cleared the PR head 8526358a:
https://github.com/lossyrob/telex/pull/202#issuecomment-6417546245
https://github.com/lossyrob/telex/pull/202#issuecomment-6417506721
Those premerge receipts are distinct from the hosted main evidence above.

**Hosted-only note:** all runtime acceptance credit comes from hosted CI.
Earlier owner/reviewer local tests, the local log sample and async-shell
probe are retained as process debt and receive zero gate credit. No local
tests, builds, runtime probes or cleanup commands were run for this main
validation. The field report does not establish real producer scheduling
or complete background-work observability.

### Seven accepted deferred debts

Campaign's merge-with-debt record is the disposition authority:
https://github.com/lossyrob/telex/pull/202#issuecomment-6418110367

1. The PR body's Validation section has stale local counts, not hosted-run
   evidence. The hosted evidence above supersedes those counts for acceptance;
   the historical body has not been rewritten.
2. The body lacks Discoveries and Deferred work sections. This field report
   supplies the record; the reconciler was asked to add discovered-work rows
   for background-work observability and session.start/session.error unknowns.
3. The classifier CLI test fixture Drop unwraps remove_dir_all, retaining the
   double-panic hazard seen in #203. No fix is claimed here.
4. On Unix, opening a same-user FIFO named events.jsonl could block. The
   byte bound is not a guarantee against this open-time case.
5. If the bounded suffix begins exactly at a line boundary, its first complete
   record is still discarded. This conservative loss of evidence remains.
6. Read-denied testing is Windows-only, and there is no hosted macOS
   classifier run. Other macOS CI jobs do not fill that coverage gap.
7. Review 5645279522 remains stale, PENDING and unsubmitted. Original draft
   comments are preserved; their disposition replies were added and both
   threads resolved without submitting the review.

These remain deferred work accepted by Campaign, not completed fixes.
The owning Watcher coordinator and reconciler received the full debt record.

### Discoveries and successor dispositions

**Async-shell/background work:** log tool completion is not a complete
inventory of unfinished background work. The accepted contract does not
parse tool-result text, guess from timing or assume App confirmation fills
the gap. Producer/background-agent semantics remain unverified.
The steward's disposition is:
https://github.com/lossyrob/telex/issues/197#issuecomment-6402758809

- #199 exposes observed evidence or explicitly marks background work
  unobserved in each member snapshot.
- #200 tells members legitimately waiting on background work to park with
  a reason, return to active when woken, and explain legitimate waits when rung.
- #201 validates the staged idle-member/background-shell case without
  disturbing the work. That broader workstream gate is not claimed passed here.

**session.start/session.error:** both remain unknown in this classifier.
The historical uncredited sample reported six session.start and one
session.error unknowns; those names are discovery context, not acceptance
measurements. #199 must ensure a persistently quiet member in either case
cannot silently suppress alerting, by supported recognition or by notifying
the root:
https://github.com/lossyrob/telex/issues/199#issuecomment-6402777694
Guidance and end-to-end handling continue through #200 and #201.
The reconciler was asked to retain both discovery dispositions durably;
this report does not assert its ledger edits have already landed.

**Design impact: updated-docs.** The #197 design is recorded in
docs/design/copilot-session-stalls.md, ADR 0056 and the design index.
The docs explicitly retain the unverified assumption that an allowing root
stop ends the agent loop; overlapping hooks and producer scheduling were
not observed. No new design decision or geometry change is requested.

#198 remains closed: hosted main validation demonstrates no failure of its
stated outcome. Detector behavior, guidance and broader field validation
continue in #199, #200 and #201.

```

### Historical #210 launch and readiness; current completion follows

Watcher `toolu_01MF24iympMS8NJ2tTaEaBHu` reports Campaign direction `toolu_01F5TT6rPiJgecHikhdSiurr` and [issue #210](https://github.com/lossyrob/telex/issues/210), covering only the cli_v2 Fixture/Host Drops and classifier LocalHome Drop. The external row initially described in-progress work and links #203 deferred item 3 and #198 gate debt 3. Both linked debts remain retained-open; the later completion below does not grant debt clearance. Owner App session is `1ad2264e-cecd-4c2f-9d76-bbf7a1d73660`, originally branch `lossyrob-super-meme` at d8a382a4. The launched binding is 2,048 bytes/SHA-256 `10280e5cab94dfb71ad4147d899272395f6d7d00ab1129a36b3bc07cf1b73c1d`; the kickoff is 16,813 bytes/SHA-256 `3fa60f2ab9c45b5a78b4c736c5974b860993047de912f820b4554f5883928104`. Watcher custody `toolu_017SzuuB8VVgX1ZuUTyaNEFR` explicitly says the 1,962-byte creation receipt (SHA-256 `e8c497b6e43f0e67635e4c779faa68c55926cf9afb9192ea907c8232cbf5bccc`) was reconstructed **after the fact**. No receipt was retained at launch and there was no separate delivery message: the reconstructed create-session prompt points by reference to the bound kickoff. Tool-result and creation inspection metadata remain coordinator-attributed; neither creation nor recipient execution was independently replayed. Private paths and prompt payload remain private.

The public [work-start comment 6418720851](https://github.com/lossyrob/telex/issues/210#issuecomment-6418720851) at 14:36:40Z supports in-progress, after the initial planned snapshot. Watcher maps runtime `98c6c90c-1214-42e9-8cb8-2ee17e50607a` to the same App owner and reports branch rename to `lossyrob-unwind-safe-test-drops`, not a rebind. Owner `call_7g0ScJTgZkvoFEtRaMYbam4l` later reports draft [PR #213](https://github.com/lossyrob/telex/pull/213), first at `13ac7077b971a6332e8354bc21aee50787ccb7fa`, two scoped test files, hosted CI and independent exact-head review pending, and design impact none. These are historical attributed claims, not current immutable review heads or acceptance. Owner `call_dQRhdj67sIYJyep2wxNIl6KK` later reported readiness/non-draft at that first head and a Campaign doorbell, but `call_agdECZhlUnvq4spEiM2LkRzo` explicitly **supersedes that ready receipt**: merging main b5bad893 into the PR branch produced `247473328c9d5ab586781f9b5ce395b12f8783b0` without rebasing. CI 40347009547 and exact-head independent delta review were pending at the later report. The unchanged scoped files/patch are owner-attributed, not inspected here. No readiness or debt clearance carries. No new graph node, edge, task or launch is created by this recording.


### Current #210 completion with retained debt

Campaign direction `toolu_014qniyg2xjam96f8wcbA4Qz` accepts [#210](https://github.com/lossyrob/telex/issues/210) completed after [PR #213](https://github.com/lossyrob/telex/pull/213) merged as `f3cce4e9835d6492ecf78a685dc7f2b73616e1eb` at 15:33:50Z; the issue closed/completed at 15:33:52Z. The external ledger row is completed, with the same owner `1ad2264e-cecd-4c2f-9d76-bbf7a1d73660` and no graph node. The historical draft, withdrawal and pending-gate reports above are not current operational state. Before merge, owner `call_HkIcW0OeFW2cBRKfgl5gR6qs` had reported renewed readiness at `247473328c9d5ab586781f9b5ce395b12f8783b0` under [receipt 6420057374](https://github.com/lossyrob/telex/pull/213#issuecomment-6420057374). Campaign merits and per-test/source/CI claims remain attributed; this reconciler checks public identities and the complete gate, not product source bodies or logs.

Gate debts 1 and 2 remain with #210/#213: untested Host Drop error paths during unwinding and deliberate temporary-root leaks when cleanup is skipped. Debt 3 belongs on the existing #199/#211 gate, not closed #210. Watcher addendum `toolu_015pMbxyo3fqt18d4foaez5x` reports that gate comment was not yet available; no comment or additional task is invented. #199 remains in-progress/focus. The prior #203 deferred item 3 and #198 gate debt 3 remain open because this completion grants no retirement authority. Watcher reports the same #210 owner registered exact-merge main CI 40350552596 at `f3cce4e9`, queued at that report, and retains the field report. No terminal main-CI result is inferred and neither follow-through holds accepted completion. Recording does not delay either owner or authorize a new wake, runtime action or CI rerun.

[Full gate 6420074604](https://github.com/lossyrob/telex/pull/213#issuecomment-6420074604), 1784 bytes, SHA-256 `5b6974bcc775896bd015da9446857e9a2cedaec441a50f778c6e7e92717405c8`, retained verbatim:

```text
## Campaign merge gate: MERGE (#213 @ 247473328c9d5ab586781f9b5ce395b12f8783b0)

**Verdict:** merge + debt (high confidence). No hard stop, no operator fork. Care/posture: balanced/craft. Consequence: low (test-only).

**Floor (verified by Campaign at the exact head)**
- Head 24747332 = merge of reviewed 13ac7077 with main b5bad893 (no rebase). PR net delta is unchanged: telex-watcher/tests/cli_v2.rs and tests/copilot_session_state.rs, +60/-5, byte-identical to the reviewed head.
- CI run 40347009547: attempt 1, 21/21 success at this head. No rerun, no waiver.
- Receipts: source review 6418835089 (no findings, at 13ac7077) and delta review 6419606902 (no findings, merge equivalence verified at this head).
- closingIssuesReferences == [210]. MERGEABLE/CLEAN. 0 unresolved threads. Main drift: none (base == main b5bad893).
- Body: Closes #210, consequence, hosted-only validation, discoveries/deferred present.

**Merits (Campaign direct review of the 60-line diff)**
The Drop impls for Fixture and Host (cli_v2.rs) and LocalHome (copilot_session_state.rs) still attempt cleanup. Strict failure is kept outside unwinding and suppressed only while `std::thread::panicking()`. Four directory regressions run hosted on Ubuntu and Windows.

**Debt (retained, non-blocking)**
1. The Host Drop status/kill error paths under unwinding have no fault-injection test (disclosed in the body).
2. Temp roots still leak when cleanup is skipped during unwinding (by design, disclosed).
3. Same-family Drop unwrap/expect sites remain in PR #211's new test files (stall_detect_tests.rs, tests/copilot_stall_detect.rs). These are recorded on #211's gate, not here.

**Follow-through:** owner owns main CI at the merge commit and the #210 field report. No field-report hold on node completion.
```


### Current #199 completion with debts 2-7 retained

Campaign direction `toolu_01VH8YEigrpPdMyA71SZDKEM` accepts [#199](https://github.com/lossyrob/telex/issues/199) completed after [PR #211](https://github.com/lossyrob/telex/pull/211) merged as `3c9ac7ca960c5abae9d3f98a56f3c1ba91a6b63c` at 16:01:44Z from head `ab076172b5fc3a38075f302e874c6537c25c4a16`; the issue closed/completed at 16:01:45Z. The existing node is completed/watch under the same owner `65da0181-23ea-4b66-8a43-d0ddf4a4f495`. The complete gate follows. Debt 1 is resolved only under that exact-head authority: the cli_v2 Fixture user-home expect is now unwind-guarded. Debts **2-7 remain retained-open**. The two new routed discovery rows record their existing target accepted with these limits, not classifier-debt clearance, complete background-work observability or end-to-end workstream validation.

The now-existing gate binds #210 gate debt 3 to #199 debt 2: Drop unwrap sites in the two detector test files remain open. Resolved debt 1 covers cli_v2 only and does not resolve those sites or retire #203/#198 debt. Watcher `toolu_0133DUM4QTfowyuENAcwCecz` reports a future separate hygiene issue/owner for debts 2+3; those identities are not invented here. The guidance aspect of debt 4 routes to #200; the product-hint aspect remains with the #199 owner pending steward. Debt 6 remains #201 follow-through. This tuple launches neither #200 nor #201 and records no new node, owner or wake. Actual later staffing/bindings remain separate intake.

Independent public metadata verifies PR CI 40351001712 attempt 1: all 21 jobs succeeded at `ab076172`, with complete frozen/live identity/timing/step receipts matching. Source/design/per-test merits and consequence normal remain Campaign/owner-attributed; no product source or logs were replayed. Owner `call_wwW2aRCXY6WjYnH3X2clDz6v` reports main CI 40355122718 active and Docs 40355122774 observed at the merge; these are registration/pending snapshots only, not terminal credit. Required exact-main validation and the field report stay with that owner, with **no completion hold**. Earlier launch/draft/readiness paragraphs are history, not current operational state. The #198 completion alone did not accept the detector; this separate Campaign gate does, within its retained limits.

[Full gate 6420726231](https://github.com/lossyrob/telex/pull/211#issuecomment-6420726231), 3490 bytes / SHA-256 `cd5ba3c51f8c5b5953df921b13dc10e187b51fd658502e662bd91c0fbb0062a3`, retained verbatim:

```text
## Campaign merge gate: MERGE (#211 @ ab076172b5fc3a38075f302e874c6537c25c4a16)

**Verdict:** merge + debt (high confidence). No hard stop, no operator fork. Care/posture: balanced/craft. Consequence: normal (opt-in read-only detector, two root-facing event kinds, no change to existing runtime behaviour).

**Floor (verified by Campaign at the exact head)**
- **Merge chain:**
  - ab076172 merges cc7cfc9f with main f3cce4e9 (no rebase);
  - cc7cfc9f merges the receipted b345cd54 with b5bad893.
  - 12 of the 13 feature files are byte-identical to b345cd54.
- **The one changed file is `telex-watcher/tests/cli_v2.rs`:** the overlap with #213 was resolved by keeping #213's `panicking()` guards. #211's new user-home removal now also follows the guard, and a both-path regression test was added.
- CI run 40351001712: attempt 1, 21/21 success at this head. No rerun, no waiver, no Windows ceiling hit.
- Receipts at this head:
  - independent bounded delta review 6420184129 (no blocking findings);
  - design steward confirmation 6420166447 (PASS).

  Full source review 6419202683 and design inspection 6419177235 at b345cd54 still apply to the unchanged feature delta.
- closingIssuesReferences == [199]. MERGEABLE/CLEAN. 0 unresolved threads.
- Main drift: fad3cb2e (#214) touches only .github/workflows/ci.yml and CONTRIBUTING.md. That is disjoint and not product code, so it is accepted under the gate charter's disjoint-drift exception. Post-merge main CI is required.

**Merits (Opus merits review at b345cd54, re-checked against the merge delta)**
- **The protocol_v2 move is mechanical.** The only change is `parse_json` from `pub(crate)` to `#[doc(hidden)] pub`, and the old path is preserved by re-export. The wire format is unchanged.
- **The detector is read-only.** SQLite opens read-only with SELECT queries only. Watcher is only queried with `list --active`. Logs and membership files are only read. The integration test asserts no bytes change and no directories are created.
- **The semantics match the #199/#197 decisions and the steward's corrections:**
  - grace must be strictly exceeded, and can restart only once per episode;
  - the detector excludes exactly its own watch ID;
  - the advisory needs old evidence on both idle and unknown members;
  - state advances only on receipt, with shared dedup.
- **All I/O is bounded.**

**Debt (retained, non-blocking)**
1. RESOLVED in this head: the `cli_v2.rs` Fixture user-home `expect` in Drop is now unwind-guarded, with a regression test.
2. `stall_detect_tests.rs:29` and `tests/copilot_stall_detect.rs:77`: unwrap in Drop, the #203/#210 double-panic family.
3. `cli_v2.rs` test creates and removes a directory in the real user home, and leaves it behind if the process aborts.
4. `stall_detect.rs:302`: a member address with more than 1024 inbox messages and no recent wake source stays degraded, so the detector goes quiet in practice. This is documented but has no operator-facing hint.
5. Pre-existing `session_state.rs:91`: `File::open` runs before the regular-file check, so a FIFO `events.jsonl` blocks until Watcher's timeout.
6. No host-executed attempt calls back into its own management endpoint; deferred to #201.
7. The body has no Deferred work or Discoveries sections; #200 and #201 are referenced only inline.

**Follow-through:** the owner owns main CI at the merge commit (required, see drift above) and the #199 field report. Guidance continues in #200, and wider workstream validation in #201.
```

All prior graph summaries remain exact historical suffixes; only the existing #198 and #199 status/attention/summary fields change. All dependencies, nodes, edges, checkpoints and design links are exact. All eleven prior Watcher records, including #203 and its five open debts/process debt, stay exact. Full Local, #166 eight debts and both sent wakes stay exact. No runtime, downstream detector validation, debt clearance, release, hardening or workstation-freeze credit. Bookkeeping does not delay owners or authorized CI follow-through.

## Later required inbox-history repair upstream of #201

The design steward moved PR #211 gate debt 4 out of the #199 owner obligation into required task #216, `stall-detector-inbox-history`. This is planned work, not accepted retained debt, a reopened #199, a repair already shipped or part of validation gate #201. Debts 2, 3, 5, 6 and 7 are unchanged by this decision; separate #215 staffing is not projected here.

The task depends on completed #199 and belongs only to `stall-detection-validated`. Gate #201 now depends on both #200 and #216. No dependency is added to `usable-watcher` or `closure-ready`, and #216 does not hold the first dogfood release. The stall and attention-needed rules, #200 scope and #201 pass criteria are unchanged. Persistently degraded guidance stays with #200; a per-cause Watcher protocol diagnostic remains out of scope.

The preceding completion text and ledger qualifications preserve the earlier accepted gate and owner-report snapshots. Their no-new-node/no-geometry wording describes those earlier scoped dispositions, not this later explicitly authorized shape. The original #199 debt-4 home is superseded only for that debt; neither the historical summary nor the initial retention list renews a current #199-owner hold. Historical pending CI/field wording likewise does not renew a current hold; later #199/#210 terminal reports remain separate intake. No other debt route is changed and no debt is cleared.

This artifact performs no owner launch, wake, runtime acceptance or #201 validation. The canonical #216 node is the adapter precondition for the owning Watcher to launch it separately. Authority is https://github.com/lossyrob/telex/issues/197#issuecomment-6420988993 and https://github.com/lossyrob/telex/pull/211#issuecomment-6420990411; task https://github.com/lossyrob/telex/issues/216. Product conclusions below are attributed to the design steward, not an independent product-source/runtime investigation. Raw source bytes are preserved; the displays explicitly convert CRLF to LF.

### Design steward shape decision

Design steward: shape decision for #211 gate debt 4 (inbox history limit)

Decision: a NEW item, #216, upstream of #201. It is not accepted debt, and it is not part of #201.

Debt 4 (https://github.com/lossyrob/telex/pull/211#issuecomment-6420726231): when a member's inbox history is longer than 1024 items and the newest items show no deferred or unacknowledged message, the detector returns `degraded` for the whole group. The v2 degraded result carries no reason.

Why a new item:
- It is not polish. Every long-lived member address eventually passes that size, and from then on stall and attention-needed detection go quiet for the whole group. That defeats the outcome of this design rather than adding a cosmetic gap.
- An operator hint alone does not fix it. The v2 degraded result has no reason field, so a per-cause hint would need a Watcher protocol change. Removing the cause is the smaller and correct product change.
- #201 is a validation gate with "Product changes; failures found here become new issues" as its exclusion. A known silencing gap belongs upstream of the gate, so the gate validates the fixed detector.
- #199 is closed by #211. Reopening it would mix a new outcome into a closed item.

Shape change:
- New node `stall-detector-inbox-history` (#216): type task, attention watch. It depends on `copilot-group-stall-detector` (#199) and belongs to checkpoint `stall-detection-validated`. Like the rest of the stream, it is not a dependency of `usable-watcher` or `closure-ready` and does not hold the first dogfood release.
- `stall-detection-workstream-gate` (#201) gains a dependency on `stall-detector-inbox-history` (#216), in addition to #200.

Unchanged: the stall and attention-needed rules in this design, #200's scope, and #201's pass criteria. Agent guidance for a persistently degraded detector stays with #200. A per-cause diagnostic in the degraded result is out of scope here; it can be proposed as discovered work against the Watcher protocol.

### Design steward debt disposition

Design steward: debt 4 disposition

Debt 4 (a member with an inbox history longer than 1024 items leaves the detector degraded and quiet) moves out of retained debt into a new item, #216, upstream of #201. The decision record is on #197. The #199 owner no longer holds this debt. Debts 2, 3, 5, 6 and 7 are unchanged by this decision.


## #216 owner and amended cost boundary (2026-10-26)

Owning Watcher reports one delivered owner, session `4b60ddef-c734-453a-b95a-d8f3307daa9c`, for existing node `stall-detector-inbox-history`. Launch branch `lossyrob-cuddly-carnival`, start commit `a4b3b21fffb947e220c65267289fd758bd962cde`; the captured controller binding later names `lossyrob-stall-inbox-queries` as its current branch. This is the same owner, not a second launch. Canonical prerequisite was published as `170f83b3fe5231dd1de6298095ab271ea1b6481b`. Coordinator-reported delivery was `2026-10-26T17:03:09.499028Z`. The 15958-byte kickoff has SHA-256 `e1532d4702747474579dc6e9cd572277aba41f3f032c87538c6520b516184fc1`; its entire captured handoff prompt is byte-identical. Profile is `gpt-6.1-sol` / high / default / interactive. Preparation, profile checks, delivery and the current-branch observation are controller reports, not recipient inspection or replay.

The owner posted its plan at https://github.com/lossyrob/telex/issues/216#issuecomment-6422307467. Owning Watcher later expressly reports #216 in progress (`toolu_01JGrox1HW2qtH7JyVf1zQ1v`); status becomes in-progress and attention stays watch. All dependencies and checkpoint membership remain unchanged. #199 gate debt 4 is assigned work, not resolved. No implementation, CI, readiness or runtime acceptance follows. The #200/#215 completion/debt records and #201 execution remain separate successors.

The same owner later reported first draft PR https://github.com/lossyrob/telex/pull/220 at `a4424c77ef0c35d10ad9a7e42e3ace081ec44ab8` (`call_MaBD3DRT1MMun9Rg7NBPxybj`). At that report it was draft with exact-head independent review, design inspection and hosted CI pending. Implementation/local proof complete and a merged-#217 base are owner claims, not reconciler replay or acceptance; this records the initial snapshot, not later head/readiness.

The steward selected option A after launch, amending the outcome without graph geometry changes. The bound original kickoff remains exact history; this later decision does not rewrite it. Bounded means result size and detector memory, not constant database work. Full public decision follows as an explicitly CRLF-to-LF-normalized display; private source bytes remain exact. https://github.com/lossyrob/telex/issues/216#issuecomment-6422351363

Design steward: #216 cost boundary decision

Decision: option A, with the conditions below. Option B is not authorized for this item. This amends the outcome text I wrote. "Cost does not grow with history length" asked for more than the existing stores can give without a schema change, and the owner was right to stop and return it.

What #216 is for: the detector must give a correct answer for any history length, instead of going degraded and silent past 1024 items. Correctness without a size limit is the requirement. A strict bound on database work was a secondary aim. Reaching it would need a maintained per-recipient wake-state projection, with write-path changes in both backends and compatibility with older writers such as v0.2.1. That is a store change, not part of this item, and it is not easily reversed.

Amended cost requirement:

1. Returned data and detector memory are bounded independent of history length. Each active member gets a scalar or fixed-size answer (for example EXISTS or a single count), never a list of messages.
2. Database work may grow linearly with that member's own retained history in the worst case. It must not grow with other addresses' history. Each query is constrained by recipient address equality so that the existing messages(to_addr, id) and dispositions(message_id, id) indexes are usable. A scan of every address's messages does not meet this.
3. Each member gets a fixed number of queries per run, not one query per message.
4. SELECT-only through the read-only backend, as before. No cap, no newest-N inference, no sampling, and no claim that EXISTS makes the work bounded.
5. A slow query must surface as the existing visible run failure or timeout. It must not be converted into an absent wake source or a silent pass.
6. docs/design/copilot-session-stalls.md states this cost characteristic plainly: bounded result and memory, worst-case work linear in the member's retained history, which grows with use because acking does not shrink it. It also names a maintained projection as the remedy if real histories make polling too slow. The PR records the SQLite EXPLAIN QUERY PLAN for each new query, showing that the address index is used. For Postgres the query only needs to be index-eligible; no plan assertion is required.

Unacknowledged counts: unchanged. Keep the existing undelivered_counts semantics and every other caller as they are; its cost is in the same class this decision accepts. The detector may use it, or a scalar query with identical semantics, for the unacknowledged wake source over full history. Do not change the shared function for this item.

The rest of the outcome stands: identical semantics on both backends, the #197 and #199 rules unchanged, the three test cases, and the guidance cleanup in https://github.com/lossyrob/telex/issues/216#issuecomment-6421106582.

Not decided here: a wake-state projection or index. If evidence later shows polling cost matters, propose it as a separate store item through the steward. It is not discovered work for #216.

I have updated the issue body to match.


## #215 completion, exact gate-debt resolution and accepted constraints (2026-10-26)

Campaign `toolu_01Vh1JWYa1NJHL4KHh22PfnL` accepts existing out-of-graph #215 complete after PR #218 merge `a4b3b21fffb947e220c65267289fd758bd962cde` at `ec8875272b6220770486dd8c60e10744a0c29a6a`. Owning Watcher expressly resolves #211 gate debts 2/3. Same owner `bd6ddeda-7950-4756-8b92-494c1030581e`; initial branch `lossyrob-scaling-guide`, public PR branch `lossyrob-stall-fixture-hygiene`, kickoff 17867 bytes / SHA-256 `654e536652e1644f37e0547679b64cff17b47eebe4b7483ddf29a4549ee9386a`, actual controller creation receipt at `2026-10-26T16:10:27.560Z`. This records existing staffing, not a new launch or runtime-ID mapping. Original hosted-only/old-freeze kickoff remains exact history under the later current policy.

The earlier readiness and merge snapshots left main CI/field pending. Later owner `call_CIHZM9WWjJ7E103DlVu9glQh` and Campaign accept terminal follow-through: main run 40364215376 at the exact merge passed 21/21 on attempt 1; independently captured full run/job/ordered-step metadata match. Observer and named-test/runtime merits remain attributed, with no raw logs or local replay. All three delivery debts have final accepted dispositions; no per-debt item/node, #201 widening or #172 authorization. #210 external debt 3 maps only to the resolved #199 gate debt 2; original #198/#203 and other owned debts are not retired.

Full public merge gate, field and final debt decision follow as CRLF-to-LF displays; private raw bytes remain exact.

https://github.com/lossyrob/telex/pull/218#issuecomment-6422110810

## Campaign merge gate - PR #218 at ec887527 (Closes #215)

Verdict: **merge**, with debt.

Gated head: ec8875272b6220770486dd8c60e10744a0c29a6a. Merge base 3c9ac7ca.

### Floor
- Exact-head CI: run 40361125894 (pull_request), attempt 1, 21/21 success. It was created at 16:38:56Z, after #206 merged (a0a9c064, 16:29Z), so the merge ref included #206.
- Receipts at this head:
  - Design steward: issuecomment-6421588334, which follows 6421190414, the teardown finding addressed at 5ab1c8aa, and the 6421240060 re-inspection.
  - Independent delta review: issuecomment-6421608765. R1 from the full source review 6421493834 is resolved, with no new findings.
- closingIssuesReferences: [215]. Unresolved threads: 0. MERGEABLE/CLEAN.
- Main drift: a0a9c064 (#206, tests/credential_command.rs only) was already in the CI merge ref, and 170f83b3 is .streamliner only. Both are disjoint, so no merge is required.

### Merits (Campaign review)
- Drop on the stall_detect_tests Directory and the copilot_stall_detect Fixture now attempts removal on unwind and fails only when not panicking. Three tests each cover unwind and normal cleanup, preservation of the original panic, and failure on normal return. This resolves #211 debt 2.
- The binary_owned_stall_detector test no longer writes under the real HOME, which resolves #211 debt 3. On Unix it requires TELEX_TEST_HOME_ROOT, validated without repair: absolute, not a symlink (including trailing separator or dot spellings), user-owned, mode 0700, ancestors root- or user-owned and not group- or world-writable, outside the canonical real home, and a complete socket path that fits. On Windows it uses a home inside the fixture root.
- Linux CI creates a short private root with sudo used only for setup, and an always() step fails if any test home leaks. The variable persists through GITHUB_ENV to the later sqlite test steps in the same job. macOS jobs do not run these tests.
- The teardown-bypass subprocess test proves that an abrupt exit leaves a fixture only outside the real home.

### Debt (carried, non-blocking)
1. On Linux and macOS, `cargo test --workspace` now fails without TELEX_TEST_HOME_ROOT. This is documented in CONTRIBUTING, but it is new friction for Unix contributors. The operator workstation is Windows and is unaffected.
2. macOS never runs the isolated-home fixture or the binary-owned detector integration test. This is a pre-existing coverage gap.
3. The Linux CI setup depends on passwordless sudo on hosted runners.

### Follow-through (owner bd6ddeda)
Report main CI for the merge commit and post the #215 field report.

https://github.com/lossyrob/telex/issues/215#issuecomment-6422826516

## Field report

#215 is delivered by [PR #218](https://github.com/lossyrob/telex/pull/218), merged as `a4b3b21fffb947e220c65267289fd758bd962cde`. Exact-merge main CI passed on its first attempt; the issue remains closed.

**What held.** Both named test destructors attempt cleanup while unwinding without replacing the original panic with a second cleanup panic. Cleanup failure on a normal return still fails the test. The binary-owned stall-detector fixture no longer creates a standalone directory in the real user home. Product socket validation, pathname limits, and runtime behavior are unchanged.

**What changed.** Unix fixtures use unique children of a validated, private, short `TELEX_TEST_HOME_ROOT` outside the real home. An unset or invalid root fails the test with setup guidance rather than falling back or repairing the directory. Windows keeps its fixture-local root. Linux CI provisions the Unix root with setup-only sudo for `mktemp` and `chown`; tests, `chmod`, and the post-test emptiness assertion run unprivileged. The empty setup root is left for disposable-runner teardown. CONTRIBUTING documents the prerequisite and a possible no-sudo local setup.

**Evidence.**

| Gate | Exact commit | Result |
| --- | --- | --- |
| Final PR-head hosted CI | `ec8875272b6220770486dd8c60e10744a0c29a6a` | [Run 40361125894](https://github.com/lossyrob/telex/actions/runs/40361125894): 21/21 jobs successful, attempt 1 |
| Post-merge main hosted CI | `a4b3b21fffb947e220c65267289fd758bd962cde` | [Run 40364215376](https://github.com/lossyrob/telex/actions/runs/40364215376): 21/21 jobs successful, attempt 1 |

The [main-CI receipt](https://github.com/lossyrob/telex/pull/218#issuecomment-6422816713) links Linux and Windows logs showing the named cleanup and live-management regressions passing, plus Linux root-validation, teardown-bypass, and emptiness checks. One managed observer followed the merge SHA. No reruns were requested or performed by this owner, and no local runtime validation was used.

The [independent source review](https://github.com/lossyrob/telex/pull/218#issuecomment-6421493834) and [final-head delta review](https://github.com/lossyrob/telex/pull/218#issuecomment-6421608765) cover the final implementation; R1 is resolved with no remaining findings. The [design inspection passed at the same head](https://github.com/lossyrob/telex/pull/218#issuecomment-6421588334). [Campaign's merge gate](https://github.com/lossyrob/telex/pull/218#issuecomment-6422110810) records the merge and carried debts.

**Carried debts and dispositions.**

1. Unix workspace tests now require `TELEX_TEST_HOME_ROOT`. The contributor setup friction is accepted as a documented setup constraint, with no new item.
2. macOS does not run the isolated-home or detector integration tests. This pre-existing coverage gap is accepted debt, with no new item and no change to #201. Watcher detector execution currently fails closed on macOS pending containment. When containment is authorized and lands under #172 or its successor, macOS hosted detector coverage becomes an item through the design steward. CLI-direct and unit coverage could run today but is also accepted without an item. No macOS fixture runtime proof is claimed. Future setup must meet the #215 conditions or stop, without relaxing a check.
3. Linux CI setup depends on hosted passwordless sudo for the two approved provisioning commands. This is accepted as a documented setup constraint, with no new item.

The [steward's final debt disposition](https://github.com/lossyrob/telex/pull/218#issuecomment-6422370702) records the macOS trigger and the coordinator's accepted setup constraints. Linux's root-level provisioning pattern is not assumed portable to macOS.

**Design impact: none.** This is test-harness and contributor/CI setup work; it introduces no product or workstream geometry change.

https://github.com/lossyrob/telex/pull/218#issuecomment-6422370702

Design steward: disposition of #218 merge-gate debt 2 (macOS stall-test coverage)

Decision: accepted debt with a recorded trigger. No new item, and #201 is not widened. This answers the routing in https://github.com/lossyrob/telex/pull/218#issuecomment-6422110810.

The gap is really two parts with different causes:

1. The Watcher-hosted test, binary_owned_stall_detector_reads_live_management_and_readonly_store in telex-watcher/tests/cli_v2.rs, with its isolated-home fixture. It starts `telex-watcher run`, which executes the detector. On macOS, Watcher v2 detector execution fails closed by design: process.rs on main returns UnsupportedPlatform, "verified containment requires Windows jobs or a Linux subreaper" (the platform module under cfg(all(not(target_os = "linux"), not(windows)))). The test cannot pass on macOS until macOS detector containment exists, which is #172's subject and on HOLD. An item created now could not be closed by a PR. Option (a) would also tie #201 to #172.
2. The CLI-direct tests in tests/copilot_stall_detect.rs and the unit tests in src/commands/copilot/stall_detect_tests.rs. They set their own HOME, use an absent watcher executable and need no TELEX_TEST_HOME_ROOT. They could run on hosted macOS today, but they exercise platform-neutral logic for a configuration the Watcher cannot host on macOS. That coverage is not worth an item now.

Trigger: when macOS detector containment is authorized and lands under #172 or its successor, macOS hosted coverage of the Watcher v2 detector tests, including both parts above, becomes an item through the design steward. Hazard for that future item: the #218 Linux root pattern is not expected to transfer. On macOS, / is a read-only sealed system volume. sun_path is 104 bytes, which leaves about 14 bytes for the root after the 89-byte suffix. $TMPDIR under /var/folders is too long, and /var is a symlink. The item must meet the #215 conditions (https://github.com/lossyrob/telex/issues/215#issuecomment-6421010376) on macOS, or stop and record why. It must not relax a check.

Debts 1 and 3 are the coordinator's accepted setup constraints; I have no design objection.


## #200 guidance completion and retained follow-through boundaries (2026-10-26)

Campaign `toolu_01GveqQN5GHTUpT12jn1UqAb` accepts #200 complete after PR #217 merged `2dc8134ac87ac1af5570d82c1d35484e89c0d934` from `b7fe01cb3607bf7138351bdc2d16ad5e065cad39`. The same App owner `fb884e5b-cd43-41b7-aed9-b22f58477c62` reports obligations complete (`call_sXBnua8K3vopxT8CEkSlGYNt`). This records existing staffing and completion, not a new owner or launch.

Controller-provided launch custody: initial branch `lossyrob-legendary-pancake`, public PR branch `lossyrob-stall-detection-guidance`, kickoff 15374 bytes / SHA-256 `7cb445790589810a0b0c1f1a5ecd75d8c9b222b29e6811fb482dc98e0d2c68a9`, start `3c9ac7ca960c5abae9d3f98a56f3c1ba91a6b63c`, delivered `2026-10-26T16:08:54.9770455Z`, and workflow 14034 bytes / SHA-256 `55131fd3bdfbc3978c408acb4b6d3b1b70320be390ceb30b6ce708b05004e2fe`. Kickoff equals the entire captured handoff prompt. Preparation, profile and delivery remain coordinator-attributed. The coordinator reports no separate #200 creation receipt: none is invented, and no recipient workspace, activity or transcript was inspected. Original freeze, canonical-pending and temporary-guidance launch instructions remain historical under the current operator policy and later #216 routing.

The readiness and Campaign merge snapshots left main CI and field follow-through with this owner. Later exact-merge main CI run 40365748597, attempt 2, is green in aggregate: one new Windows fallback job 121738788335 follows failed job 121722844042; 20 other successes retain identical full ordered steps, execution timestamps, heads, conclusions and runner identity. All 21 aggregate job IDs were regenerated and runner-group metadata changed; neither implies new execution. Both original and later metadata envelopes are retained. Docs run 40365748539 has two successful jobs. This is metadata evidence only; documentation merits, named tests, observer exit and runtime claims remain owner/Campaign-attributed.

Preserve the original attempt-1 failure and the single recorded failed-jobs rerun. #182/Local retains one distinct waiter-list readiness candidate, unassigned, not a demonstrated product defect or repair. Missing status samples, outcomes and waiter liveness/output prevent a causal diagnosis. #208 Live-seed admission remains separate. The later green execution explains neither failure. No run, observer, log/source replay or local runtime was added here.

Retained #217 gate debts: #216 owns the temporary bounded-inbox limitation cleanup in both guidance surfaces; #201 owns live workstream validation. Source-text guards do not execute registration examples or prove App wakeup. Installation, one-registry and observation limits remain. No #216 completion or #201 readiness/validation is inferred, no geometry or checkpoint changes, and no Local-family state is projected by this packet.

Full public final-readiness, Campaign merge gate and terminal field follow as CRLF-to-LF displays; original raw bytes remain separately bound.

https://github.com/lossyrob/telex/pull/217#issuecomment-6422354531

PAW Implementer: Review Addressed

Ready for operator at b7fe01cb3607bf7138351bdc2d16ad5e065cad39.

All 21 [hosted CI jobs](https://github.com/lossyrob/telex/actions/runs/40361887570), [independent review](https://github.com/lossyrob/telex/pull/217#pullrequestreview-5655965370), and [design inspection](https://github.com/lossyrob/telex/pull/217#issuecomment-6421710161) pass at that exact head. No unresolved review threads or main conflict; the PR closes only #200. Consequence: low, documentation and its regression guard only.

Read and triaged every non-blocking observation in review 5655965370. Acknowledge and retain these boundaries:

- The existing history cap remains with #216; this guidance preserves routing identity/history and uses the App fallback.
- Local classification and one-registry inventory do not establish every App or background-work state; broader observation is outside #200.
- Registration depends on existing-installation values, as required by the confirmed scope; no guessed values or wrapper are added.
- Source-text checks establish template parity and required guidance, not live registration or App wakeup. Live dogfood remains with #201.

No code change or new review cycle is needed for these observations. Prior-head Windows failures retain the #182/#208 disposition; this exact-head green run is the gate evidence. The PR is ready for the operator's merge decision. This session will continue monitoring and will not merge.

https://github.com/lossyrob/telex/pull/217#issuecomment-6422398993

## Campaign merge gate - PR #217 at b7fe01cb (Closes #200)

Verdict: **merge**, with low debt.

Gated head: b7fe01cb3607bf7138351bdc2d16ad5e065cad39. Merge base 3c9ac7ca.

### Floor
- Exact-head CI: run 40361887570 (pull_request), attempt 1, 21/21 success. It was created at 16:43:46Z, so the merge ref included #206 (a0a9c064).
- Receipts at this head:
  - Independent review: PASS, review 5655965370, commit_id b7fe01cb.
  - Design steward: PASS, issuecomment-6421710161, which carries forward the f9d34af6 PASS (6421211275). The only change since is the formatting-only test delta.
  - Readiness: issuecomment-6422354531.
- closingIssuesReferences: [200]. Unresolved threads: 0. MERGEABLE/CLEAN. Not a draft.
- Main drift since the base:
  - #206 is already in the CI merge ref.
  - #218 (a4b3b21f) touched ci.yml, CONTRIBUTING, the stall-detector test fixtures, the cli_v2 tests and credential_command.
  - V6 changed .streamliner only.
  - None of these overlap this PR's five files (copilot/COPILOT.md, the guide pages, SUMMARY and tests/copilot_plugin.rs), and the new test reads only those files. The drift is disjoint, so no merge is required.

### Merits (Campaign review)
- The binary-owned COPILOT.md now documents opt-in group stall watching:
  - membership records and the active, parked, operator-held and done states;
  - a PowerShell 7 Watcher v2 registration template that takes values from the existing installation, with no guessed paths and an ask_user fallback;
  - a confirm-then-ring response that calls get_sessions_status before acting;
  - degraded-is-not-silence guidance.
- The new guide page and the push-guide link are consistent with COPILOT.md. The contract test asserts that the registration template is identical across COPILOT.md and the guide, that the required fields are present and that the shared response vocabulary is present in both.
- It does not add a detector script or a polling loop, and it tells members not to park merely to suppress degradation.

### Debt
1. The bounded-inbox limit paragraph ("more than 1024 retained inbox entries ... until a release includes #216") is temporary guidance. The #216 owner's kickoff carries the steward's COPILOT.md cleanup for it.
2. Live validation of this guidance on a real workstation belongs to #201.

### Follow-through (owner fb884e5b)
Report main CI for the merge commit and post the #200 field report.

https://github.com/lossyrob/telex/issues/200#issuecomment-6423179743

## Field report: #200

Delivered in [PR #217](https://github.com/lossyrob/telex/pull/217), merged as `2dc8134ac87ac1af5570d82c1d35484e89c0d934`. #200 remains closed.

Design impact: updated-docs

### What held

Binary-owned Copilot instructions and the linked guide now cover root-owned group membership, member state transitions, the shipped Watcher v2 registration template, distinct stall/advisory handling, and fresh App confirmation before ringing members. Backend, registry and cadence each have one installation source. Unknown installation values go to the operator; no wrapper, product flags, detector implementation or plugin bootstrap changes were added.

### What changed during implementation

Design inspection narrowed degraded-state handling to retained routing identity/history, incomplete-coverage reporting and App inspection. The documentation says the bounded-inbox limitation applies until a release includes #216. A hosted rustfmt finding in the source-text guard was corrected without changing its behavior.

### Evidence

The final pre-merge head `b7fe01cb3607bf7138351bdc2d16ad5e065cad39` passed all 21 [hosted CI jobs](https://github.com/lossyrob/telex/actions/runs/40361887570), [independent review](https://github.com/lossyrob/telex/pull/217#pullrequestreview-5655965370), and [design inspection](https://github.com/lossyrob/telex/pull/217#issuecomment-6421710161). All non-blocking observations were [explicitly dispositioned](https://github.com/lossyrob/telex/pull/217#issuecomment-6422354531).

At the exact merge commit, [main CI attempt 2](https://github.com/lossyrob/telex/actions/runs/40365748597/attempts/2) passed all 21 jobs. The [Docs run](https://github.com/lossyrob/telex/actions/runs/40365748539) passed both guide build and Pages deployment.

Main CI attempt 1 failed the Windows E2E test `station_intent_a_busy_station_defers_the_successor_handoff_until_the_waiter_is_gone`, timing out while listing a waiter. The [failure record](https://github.com/lossyrob/telex/issues/182#issuecomment-6423039499) preceded the [single authorized failed-jobs rerun](https://github.com/lossyrob/telex/issues/182#issuecomment-6423062122). No source/check changes or second rerun were made. A green rerun is evidence for that attempt, not a repair or common-cause finding. [Sentinel classified](https://github.com/lossyrob/telex/issues/182#issuecomment-6423155985) this as a distinct waiter-list readiness observation: initial setup succeeded, but the later daemon projection barrier did not yield the waiter PID; busy-handoff assertions were unreached. Earlier pre-merge Live-seeding failures retain their separate [#208 classification](https://github.com/lossyrob/telex/issues/208#issuecomment-6421783331).

### Carried boundaries

- #216 owns the history-cap product repair and cleanup of its temporary limitation paragraphs in both guidance surfaces.
- #201 owns live workstream validation. Source-text guards do not execute the registration examples or prove App wakeup; no local runtime was performed by this owner.
- Local and one-registry observation limits and existing-installation prerequisites remain explicit. The Windows E2E observation remains under #182 for requested local adjudication; no product defect or repair owner is inferred.

The post-merge observer has completed and exited. No merged #200 guidance-outcome failure was found.


## #222 Watcher bootstrap shape and #223 outbound routing (2026-10-26)

Watcher design `toolu_015RDXAXUBTBpbRkmXBYyDxu` / public6425261263 adds exactly planned/watch task node `watcher-installation-bootstrap` tracked by [#222](https://github.com/lossyrob/telex/issues/222). Its only dependencies are completed watcher-cli-ux and stall-detection-agent-guidance. runtime-hardening gains only this dependency; closure already reaches it transitively. All 22 old node objects remain exact except that one appended hardening edge. Existing statuses, summaries, geometry, root fields, checkpoints, design links and ordering remain exact. No #201/five-minute/macOS edge is added.

The coordinator `toolu_01596kTy4zSBeUtdxZib8VwJ` requires this canonical node and GitHub tracker before launching its ready single owner. This records no owner, requested/effective profile, launch, implementation, installed state or readiness/validation result. The coordinator receives a doorbell only after verified publication.

Owning Watcher `toolu_01HCzYW79vQL9tMEhErXwssY` reports Campaign `toolu_019JXfDVGr4mW1LwPYYa7NBw` priority hold of five-minute-custom-watch-gate until #222 product delivery. This is NOT a graph dependency and planned-node publication does not lift it. The gate stays planned/unowned; no status change is made here.

### Complete #222 shape/outcome/boundaries

https://github.com/lossyrob/telex/issues/222#issuecomment-6425261263

Raw 2101 bytes / SHA-256 `15990c6d76bf591556868af4e5630334ea4e804e540ee4e715af77edd66e42bf`; LF display 2100 bytes / SHA-256 `7d98e6b9d6dbda31ac6ce60f9d7682b6d5abd261aa607245c360356958d3841e`.

```text
Design steward decision: this joins the Watcher geometry as a new item.

Node: `watcher-installation-bootstrap`, a task tracked by this issue, with attention watch.
- Depends on `watcher-cli-ux` and `stall-detection-agent-guidance`, both completed.
- `runtime-hardening` gains a dependency on it, so `closure-gate` depends on it transitively.
- It is not a dependency of #201, `five-minute-custom-watch-gate` or `watcher-macos-containment-runtime`.

Why it is Watcher work: the Watcher guidance (stall guide, `telex copilot skill`) starts from a running Watcher installation whose trusted executable, registry, profile and sender values the root can look up. No supported way exists to create or record one. Runtime-hardening exercises packaging and upgrade but does not provide this, and the #201 dogfood setup was operator-authorized for that run only.

Outcome:
1. A supported, discoverable Watcher installation and launch record holds the trusted `telex` and `telex-watcher` paths, the registry, the backend profile and the send-only sender, for an agent to read instead of guess.
2. A documented bootstrap path creates that record and starts the runtime with its required environment. It has explicit shutdown, rollback, and handling for an existing older installation. It never silently modifies an existing installation.
3. The stall guide (and `telex copilot skill` where it carries the same text) points to that path. It names the supported root delivery modes: Copilot bridge push, or the documented pull fallback (`telex copilot fallback prepare`) where the host cannot load the bridge. It keeps the two modes distinct, as #201 recorded in 6425143128.

Boundaries:
- It uses existing backend profiles and authentication. If a new backend authentication capability is needed, that is a separate item.
- Telex binary installation and upgrade stay with the existing release-upgrade surface.
- The App bridge loading gap is #223 and is not part of this item.
- Platforms are Windows and Linux. macOS follows the containment item and remains debt here.

The reconciler writes the graph change.

```

### #223 routes to Local, no Watcher geometry/dependency

https://github.com/lossyrob/telex/issues/223#issuecomment-6425261961

Raw 881 bytes / SHA-256 `e001e67d741596207da1c10abd3d926f7eb3cf66a9785fc443949c3e923e29ad`; LF display 880 bytes / SHA-256 `5f1fdfedef777735ef422fd4de32f9a8a1a75f2cc4b6cf7343673a4d56bc81c2`.

```text
Design steward decision: this does not join the Watcher geometry. It routes to the Local Daemon workstream.

Why: first-time Copilot bridge loading (`telex copilot attach --copilot-bridge`, then `extensions_reload`) and the push/pull delivery contract belong to Local Daemon's `push-delivery` design (docs/design/copilot-bridge-push.md, guides/copilot-push.md). Whether App-hosted sessions get a supported first-time bridge path, or the guide states pull fallback as the App path, is that workstream's decision.

Watcher dependencies: none. #201 uses the supported pull fallback (6425143128). The Watcher stall guide naming push as the only root delivery mode is Watcher's own wording; the fix moves into #222's outcome, item 3, and does not wait on this issue.

The Watcher coordinator hands this off to Local Daemon routing. The outcome text in this issue body stands as filed.

```

### Earlier supported-pull semantics and conditions, not runtime proof

https://github.com/lossyrob/telex/issues/201#issuecomment-6425143128

Raw 3150 bytes / SHA-256 `b09087f984619ca0593731a907ab1d68bc421ab60e4a3b3b10480658dea6ed14`; LF display 3149 bytes / SHA-256 `74778ede2fc6291ba2232145258ade22023255305573f13e1357bf2f4df0ba7d`.

```text
Design steward answer for the #201 owner: root wake path when the App session cannot reload extensions.

Decision: Telex's supported pull fallback satisfies the real-root criterion. This is not an upstream blocker, and #201 keeps its outcome and scope. The operator-chosen root, Campaign, stays the root.

Why: the #197 criterion, as copilot-session-stalls.md records it, is that Telex delivers the detector event to the root as a turn, the root confirms the stall with the App's get_sessions_status, and then rings the affected members. It does not name a delivery channel. copilot-push.md documents the pull fallback (`telex copilot fallback prepare` plus exactly one pull-mode wait) as the supported Copilot delivery path when `extensions_reload` is unavailable. A turn produced by that path is Telex delivery. The stall guide's sentence "already receive turns through Copilot push delivery" is a guidance gap: it names one channel where the design needs any supported one. That gap does not change what the gate proves.

Conditions:
1. Use the documented fallback as written: Telex-owned `copilot fallback prepare` and a single pull-mode wait whose completion wakes the App session. Do not use a custom polling loop, a scripted inbox read, or the operator relaying messages.
2. The event the root acts on is the real detector event, delivered by the real Watcher registration on the real backend, to the root's real address. "Exactly one event per episode" is counted where the event is emitted and stored in Telex, not by what the wait surfaced. The report states how it was counted.
3. Re-arming: a pull wait completes once. If the run spans more than one episode, or also receives the staged attention-needed case, the root re-prepares or re-arms per the guide before the next episode. Report any event that arrived while no wait was armed, and when it was read. Do not count it as a missed or duplicate emission.
4. The report says plainly that root delivery used the pull fallback and that bridge push delivery to an App root was not exercised. It names the delivery mode station_health reported (`pull`). Do not claim push and do not imply the guide was followed as written.
5. Confirm and ring are unchanged: the root confirms with get_sessions_status before acting and rings members through its normal channel. The report names the channel the members' replies came back on.
6. File the App bridge / stall-guide prerequisite gap as a separate discovered-work issue, per the owner guide. It covers both the missing extensions_reload in App-hosted sessions and the stall guide naming push only. It is not a dependency of #201, and #201 does not amend the shipped guidance (no guidance or product change in this item).
7. If the fallback itself fails, for example no wake on completion or a lost or duplicated event, that is a product failure: file it. If it prevents confirm/ring, bring it back to me; the answer is an upstream item, as in 6424158582 item 5.

The alternative, an external CLI root, would change the operator's chosen arrangement and is not needed. No graph or outcome change. Earlier conditions in 6424158582 still apply.

```

Two new Watcher records retain #222 disposition and #223 outbound routing, with all 15 old rows/root/order exact. No Local ledger/node/owner is changed. #222 owns Watcher bootstrap and stall-group installation/root-mode wording, not host loading, new backend authentication, Telex binary install/upgrade or macOS containment. Owning Local `call_kUHJSXbVmnRG8PJlpwsuplKL` separately assigns #223 host-capability/first-load/fallback/recovery text; shared copilot/COPILOT.md changes require open-PR plus named-section coordination, not a dependency. #224 and the actual blocked #201/external edge remain their separate packets and receive no runtime, cause or acceptance credit here.

No new task markdown file or launch packet is invented. Only graph, ledger and this append change: one new task node, exactly the declared runtime-hardening edge and two rows; Watcher17/23. All other workstreams/product/canonical design, Local debts/observations, #166 eight debts/two sent wakes and existing Watcher debt/notification state remain exact. No new observer, root re-arm, runtime/install/ACL action, wake, repair, gate waiver or debt clearance follows.


## #225 planned Watcher runtime-failed diagnostics, external #195 consumer (2026-10-26)

Watcher steward `toolu_01R872e2ciDWRcqikEuCpnZz` / [public decision 6425865009](https://github.com/lossyrob/telex/pull/221#issuecomment-6425865009) accepts existing [#225](https://github.com/lossyrob/telex/issues/225) as planned/watch task `watcher-runtime-failure-diagnostics`. Readiness is a coordinator report (`toolu_013hXzYq7X45TS3HAq1m3CcX`), not a new graph status or assigned owner. Only completed watcher-runtime-core and watcher-cli-ux are dependencies; no Watcher node gains a dependency on it. Sentinel #195 / PR #221 is external discovery/consumer provenance and stays outside Watcher geometry. Its workstream owns the external edge.

For runtime-failed startup/run errors, write a bounded 4 KiB redacted stage-labeled cause chain to stderr, with truncation marked. Preserve the stdout envelope, runtime-failed code/exit 1 and all other specific codes. Update docs/design/watcher.md and the exit-code reference; test forced startup failure/stage labeling, redaction, byte cap and unchanged envelope on every platform where the binary builds. Do not diagnose/fix the #195 macOS cause, add a logging framework or persisted startup records.

The cap is 4,096 bytes including the diagnostic/truncation marker. Preserve envelope shape/code/exit; the source issue allows message text to point to actual stderr. Redact environment values/DSNs/tokens/secret-like values; existing design permits local registry/endpoint paths. These are future acceptance requirements, not measurements or current implementation proof.

### Full steward decision and bounded native-consumer hold

Raw 2785 bytes / SHA-256 `624299d760a30baeae82ae318f5448d122416c0fcac90a648ec74cab05c98aad`; LF display 2784 bytes / SHA-256 `993044a5f588671549e2fd2e1039a17b7ded38cfe40945b3ef625357a62ccb63`.

```text
Design steward decision for #195 / PR #221 on the diagnostic receipt https://github.com/lossyrob/telex/pull/221#issuecomment-6425811617 (head 85d78160, run 40389528160/1, job 121806570895).

Classification: the macOS startup cause is still unknown, but the reason it cannot be seen is a product defect. That defect is now upstream item #225. No more evidence can come from #195 until #225 lands.

Source facts (read only):
- The envelope comes from `run_host` in telex-watcher/src/main.rs (lines 119-123). It maps every error that is not `registry-already-owned` and not a top-level `TransportError` to fixed text and drops the cause chain. The runtime writes nothing to stderr, which is why the receipt has 0 stderr bytes. A test-side change cannot recover it.
- This rules out the leads I listed in 6425476487. `endpoint()` failures in transport/unix.rs, including the home-directory requirement, the private-directory and ancestor checks, and the sun_path length check, come back as a top-level `TransportError`, so they would report a transport code, not `runtime-failed`.
- The remaining sources are all product paths. `Runtime::open` and `Registry::open`. The listener steps after `endpoint()`, where an existing-socket check, unlink, `UnixListener::bind` or the 0600 chmod could fail. `Runtime::run` startup (unfinished containment, start_runtime, heartbeat). A management-server task failure. Which one, and whether the cause is in the fixture or the product, is not established.

Decision:
1. New Watcher item #225 (`watcher-runtime-failure-diagnostics`). For `runtime-failed`, write a bounded, redacted, stage-labeled cause chain to stderr. The stdout envelope, the code and the exit code stay as they are. Update the design and exit-code docs, and add tests on every platform the binary builds on. It depends only on completed Watcher items, so it is ready now. It does not fix the macOS cause. It is also an operator defect: the guide tells operators to inspect stderr after exit 1, and today that stderr is empty.
2. #195 depends on #225. PR #221 stays draft. No more test-only diagnostics, reruns or new pushes for this question until #225 merges. Another fixture-state probe would at best narrow the failure to a stage, and it would use a run without fixing the gap.
3. After #225 merges, #195 may rebase. The ordinary CI run of that rebased head is the next single diagnostic run, and the existing cli_v2 early-exit diagnostic will then show the stderr. Ownership still follows 6425476487:
   - A fixture or test-environment cause stays in #195 as a test-only fix.
   - A product cause, or a deliberate macOS startup limit, comes back to me.
4. Sentinel takes no part of #225. A Watcher owner implements it.

This is a boundary and shape decision, not merge approval.

```

The generic missing diagnostic is steward-classified product work; which stage failed on macOS and whether #195 needs fixture, product or platform work remain unresolved. Supplied source lines, run/job/head and empty-stderr receipt are attributed reports; no source, logs or runtime was replayed. #225 does not need native macOS evidence for closure. Only after #225 merges may #195 rebase for the steward-prescribed single ordinary CI diagnostic; this recording grants no retry, push, rebase, cause disposition or merge.

Exactly one new row/node and this append change, with all prior rows/nodes/root/geometry/order/dependencies/checkpoints/design links/debts/wakes and other workstreams exact. No task file, observer, launch/profile, runtime/install/ACL change, #201 validation or other-family acceptance/clearance is added. Watcher receives a doorbell after verified publication and retains its own single owner launch.


## Five-minute custom-watch gate: readiness, #232 tracker and prospective protocol (2026-10-26)

Owning Watcher `toolu_01A7jSrD2dsSLansvrrrRY92` reports Campaign lifted the decision-(b) #222 delivery priority hold and delegated sequencing to Watcher. Both existing runtime-core/CLI-UX dependencies are complete. Replace the old pending-with-operator/no-Watcher-launch summary wording with this current source-qualified readiness; keep the exact old summary below as history. Design steward `toolu_01HoPqPAzZhqDZsgd1qqWve7` opened [tracker #232](https://github.com/lossyrob/telex/issues/232) and supplied the exact appended protocol summary. Status remains planned, attention focus; all dependencies, checkpoints, order/geometry and other fields/nodes remain exact. No row, task Markdown or owner is added.

The dedicated Windows/PowerShell7 gate uses its own installation, registry, sender and gate address, never root-201/#201. A future Watcher-coordinator gate runner prepares installation through its own operator ask_user and launches two fresh no-context subjects. Each fixed plain request permits at most three user turns and five active minutes. Subject1 creates/registers a customized local-file watch and is archived before the trigger; exactly one delivered message/no duplicate is checked within the specified intervals. Subject2 diagnoses an injected detector-command failure from Watcher surfaces without changing anything. One run counts; failures are findings, not retry-until-green. The builder rules on #232 and closes it. These are prospective requirements, not this reconciler execution, subject/runner launch, current installed state or measured success. Later owning Watcher `toolu_01Ks72xgpLrctV5EUeHSXkdw` reports gate-runner session a44b6ed2 prepared and held pending tracker publication. Keep that short ID source-attributed, without resolving a native/full identity or asserting effective profile, installation preparation outcomes, subjects started or a gate result. The earlier no-owner/no-tracker paragraph remains its source-time history, not a current denial. No product/PR/macOS/#201 authority, runtime/API/backend expansion, installed modification, wait/root rearm or debt credit follows.

### Exact original summary retained as history

```text
Builder verifies a frontier agent can create a small customized watch in a few turns, register it without mandatory framework ceremony, leave it running after the session ends, and diagnose a failure from Watcher status and attempt/state surfaces. On 2026-10-25 both dependencies completed; the gate is ready and pending with the operator, with no Watcher owner launch.
```

### Exact owning gate-only source excerpt

Complete source bound privately; first #222-completion section is separate and not altered here.

```text
2. five-minute-custom-watch-gate: Campaign lifted the decision-(b) priority hold on #222 product delivery (Campaign message toolu_018ua1rPaucF16szM5sJDd3g, 22:04Z). Its dependencies are complete, so it is ready. Campaign delegated sequencing to me (Watcher coordinator). It has no tracker or owner yet. I've asked the steward for a tracker and a verification protocol before I start it. Please replace the summary's "pending with the operator, with no Watcher owner launch" wording when you next project it. Status stays planned until the gate actually starts.
```

### Complete design source

```text
Steward graph change for telex-watcher node five-minute-custom-watch-gate. Evidence: https://github.com/lossyrob/telex/issues/232 (tracker and verification spec, posted by the design steward).
1. Set tracker to github lossyrob/telex #232.
2. Append to its summary: "On 2026-10-26 the design steward opened tracker #232 and pinned verification: a Windows/PowerShell 7 run against a dedicated gate installation, registry, sender and gate address (not root-201 or #201). A gate-runner session launched by the Watcher coordinator prepares the installation under operator ask_user and collects evidence. Two fresh no-context subject sessions each get a fixed plain request with at most three user turns and five active minutes: subject 1 creates and registers a customized local-file watch; after subject 1 is archived, one trigger must deliver exactly one message; subject 2 diagnoses an injected detector-command failure from Watcher surfaces without changing anything. One run counts. The builder rules on #232, and the ruling closes it. No product, PR, macOS or #201 authority."
No change to dependencies, status, attention or checkpoints. Status changes stay with the coordinator.
```

### Exact later owning held-runner excerpt

The same bound source has separate #225/#233 chronology that is not projected here.

```text
The #232 gate runner (session a44b6ed2) is prepared and held. It waits on your publication of tracker #232 on five-minute-custom-watch-gate.
```

### Complete tracker/specification body (raw6535/SHA3d8e3f6a61eeba4cceb5186affe250d5888469517a0feccc053a74eae2d85edd; CRLF-to-LF display only)

```text
Tracker for graph node `five-minute-custom-watch-gate` (workstream telex-watcher, checkpoint `usable-watcher`). Dependencies `watcher-runtime-core` (#162) and `watcher-cli-ux` (#179) are complete. #222 merged as 7036e710.

## Outcome

The builder verifies that a frontier agent with no prior Watcher context can:

1. Create a small customized watch in a few turns.
2. Register it without mandatory framework ceremony.
3. Leave it running after the session ends.
4. Diagnose a failure from Watcher status, health, show and attempts output.

Host: Windows with PowerShell 7 (the Windows-path gate). macOS stays out while #172 holds; Linux is not required.

## Roles

- **Gate runner**: one session the Watcher coordinator launches. It prepares the installation, launches the subjects, triggers the condition, injects the failure, collects evidence, and asks the operator for the ruling through its own ask_user. It is never a subject.
- **Subjects**: two fresh Copilot App sessions with the App's default model, created with `coordinate_with_creator: false`. Each works in a new empty folder, not a telex checkout, so it cannot read `.streamliner`, task files or design history. Record each model ID. The operator's user-level instructions are host environment; record that they were present.
- **Judge**: the builder (operator) rules pass or fail on this issue. The gate runner and design steward do not.

## Precondition (outside the timed run)

Before subject 1 starts, the gate runner prepares a dedicated gate installation following the merged installation guide:

- its own registry;
- its own send-only sender address;
- an installation record created by `installation.ps1`;
- a running owner that passes verify and preflight and reports ready.

Every real installation change here goes through the operator's ask_user in the gate runner's session, not in a subject session.

Do not use root-201, the #201 installation, or any live workstream address or registry.

Delivery target: a fresh dedicated gate address that no session attaches to. The gate runner reads its inbox only after the trigger.

## Phase A: create and register (subject 1)

The kickoff is exactly this request, with the placeholders filled. Add nothing else.

> On this Windows machine Telex and Telex Watcher are installed and a Watcher runtime is already running. Its installation record is at `<record path>`. Set up a watch that sends one Telex message to `<gate address>` when any .txt file in `<empty gate directory>` contains the word READY; don't repeat it while that stays true. Check about once a minute. It must keep working after this session ends. Docs: https://lossyrob.github.io/telex/ and https://github.com/lossyrob/telex/blob/7036e7103aae4416cf0a0e2030175dc901ece893/docs/watcher-runtime.md . Tell me when it is registered and ready.

Omit the record sentence if the record is at the default discovery path. No example script matches this condition as written, so it requires customization. The subject may copy an example or write its own; record which it did. That observation is not a ruling on `examples-accelerate-gate`.

Pass criteria:

- **Turn budget.** At most three user turns: the request plus two replies. A reply may only answer a question the subject asked or say "continue". No hints about commands, docs, files or fixes.
- **Time.** Readiness reached within five minutes of active agent time. Measure from the request to the subject's ready report, minus time spent waiting for the user.
- **Ready.** `show` and `health` report the watch registered and ready, with the requested route and sender. The runtime reports ready for the current registry revision.
- **No ceremony.** Only a command, bounds and route were needed. No mandatory manifest, pinning, kind allowlist, provider preflight or test framework.
- **No session-owned polling.** No background loop, waiter or second Watcher owner started by the subject.

## Phase B: running after the session ends

After subject 1 reports ready, the gate runner archives it, which stops its CLI process. Then:

1. Wait at least two poll intervals.
2. Write one .txt file containing READY into the gate directory.

Pass criteria:

- Exactly one message reaches the gate address within three poll intervals of the trigger. No duplicate arrives in a further three intervals.
- `events` shows its receipt, and `attempts` continues after the archive timestamp.

## Phase C: diagnose a failure (subject 2)

The gate runner breaks the detector without editing the registry: rename the script file the registered command references so the command fails. If the command references no file, ask the steward on this issue for an equivalent injection before proceeding. Confirm that `attempts` or `health` records the failure. Then launch subject 2 with exactly:

> A Telex Watcher watch on this Windows machine should notify `<gate address>` when a .txt file in `<gate directory>` contains READY, but it doesn't seem to be working. Find out why using Watcher, and tell me the cause and the fix. Don't change anything.

Same turn and time budget as Phase A.

Pass criteria:

- The subject names the missing or moved detector command as the cause.
- It reached that cause from Watcher status, health, show or attempts output, as its transcript shows.
- It proposes a correct fix and changes nothing.

## Run discipline

- One run counts. A failed criterion is a finding for the builder, not a reason to rerun until it passes.
- A rerun needs either a fix that has landed or an explicit builder decision recorded here.

## Evidence (posted on this issue by the gate runner)

- Subject model IDs.
- Kickoff text as sent.
- Per-subject user-turn count and active time, with the start, end and wait timestamps used.
- The detector script and registration as registered.
- `show`, `health`, `status` and `attempts` JSON at ready, after the trigger and after the injection.
- The delivered message and its receipt.
- The archive timestamp.
- The injection performed.
- Subject 2's diagnosis.
- Cleanup: watch removed, gate owner stopped, and the record retained or removed per the installation guide's rollback.

No credential values may appear in the evidence.

## Out of scope

- No product change, PR, merge or release authority.
- No macOS claim.
- No #201 installation change.

The builder's ruling closes this issue. Findings that need product work are filed as discovered work and routed.

```


## #222 same owner, implementation published/in review, not operator-ready (2026-10-26)

Watcher `toolu_01UyJYEkRbKr4nquJn5iH55q` reports sole existing owner `e292322d-c271-432d-8703-5fc14a4c63c3`, starting from published bootstrap `331c4da1ed9970771e3d7213d13dc2ed068d31e7`, initial branch `lossyrob-ubiquitous-meme`. Reported kickoff delivery is 20:38:10Z, 19,302 bytes / `6cd974fb4d3d6db79b25621d2ba0155dbea4ae5b5e527715b443831c3b6f802d`, with Step-4 checks passed. Those publication/duplicate/checkout/context/kickoff/delivery checks are coordinator-attributed, not replayed here. Initially the node stayed planned until a public claim. The subsequent owner draft PR report and owning in-progress confirmation meet that condition for this same node/row; record in-progress, attention watch unchanged. No second owner, node, family, row or occurrence is created.

The complete binding version is 2,796 bytes / `200b77a992b76bb88a630a97ab1c7bc7dfa3234c84680019a39e6d182c47f676`, captured 20:40:38.889Z, with updatedAt 20:40:02Z after the initial launch intake. Retain its later branchRenamedTo `lossyrob-watcher-installation-bootstrap` without dating the rename or proving current branch/checkout. Only seven named absolute private-path leaves become checkout/context/document names; restoring them recovers the complete original object. No referenced context, handoff, kickoff, launch adapter, domain note, workflow or recipient was inspected. Declared gpt-6.1-sol/high/default/interactive and coordinateWithCreator=false remain recorded requested snapshots, not effective profile, capacity, activity, current HEAD/cleanliness or runtime delivery.

Owner `call_KW1VlAILudESTB7dEkRbQNbE` reports draft [PR #228](https://github.com/lossyrob/telex/pull/228), first head `b75c417f8d1c2896f8f45647aea5050fe3d177b9`, Closes #222, implementation published/in review as a normal consequence. Independent App PR review, exact-head steward inspection and hosted CI were requested/pending at that source. This is not a source diff/review/CI inspection, accepted product, Campaign gate, merge or current/exact-head readiness; later reports remain separate qualifications. Shared-file changes are reported limited to stall installation/root-mode wording; #223/PR #226 App-delivery slices stay separate. No live installation changed was reported. Do not mark operator-ready.

The complete original bootstrap contract, dependencies and other fields remain exact: existing profiles/auth, installation/runtime/shutdown/rollback/older-installation handling, distinct bridge-push versus supported pull-fallback modes. No Task Markdown or runtime action is added. No silent install modification, bridge loading/wake, supported-wait/root test, five-minute release, #201 validation/rearm, #224 repair, cleanup, debt credit, implementation completion or new worker follows. The original planned/unassigned labels remain prelaunch history, not a present work hold.

### Complete actual owner implementation source

```text
Reconcile #222 as implementation published, in review: draft https://github.com/lossyrob/telex/pull/228 at b75c417f8d1c2896f8f45647aea5050fe3d177b9, Closes #222, normal consequence. Independent App PR review, exact-head steward inspection and hosted CI are requested/pending. Shared-file edit is only stall installation/root-mode wording; #223/PR #226 slices remain separate. No live installation changed. Do not mark operator-ready yet.
```

### Exact later owning #222-only excerpt

Source `toolu_01WzN28S1CERA3BRZ8U9xfdC`; complete private intake bound separately, unrelated #225/private-path text neither published nor followed.

```text
Separately, #222 is in progress: owner e292322d draft PR https://github.com/lossyrob/telex/pull/228 (Closes #222), already in your intake.
```


### Later readiness, accepted merge and owning bounded completion

The later same owner reports readiness at `a6581875c1a36aafcb9c083d8d1cf071aee266ad` after exact-head design, independent review and all21 hosted jobs. Preserve the independent review first-run Windows HOME-probe timeout and its unchanged-head9.05s rerun; later owner/Campaign receipts supply the previously pending Linux execution report. No source/job/log/runtime replay is claimed here.

Campaign `toolu_01VX657ifrdVjxL5eN8d466o` reports PR #228 merged `7036e7103aae4416cf0a0e2030175dc901ece893` at22:04:03Z; public issue222 closed/completed22:04:04Z and parsed closing refs are exactly[222]. Owning Watcher `toolu_01A7jSrD2dsSLansvrrrRY92` explicitly marks this node completed. Record same-node/row completed/watch; owner remains historical, no duplicate worker or geometry. All earlier planned/pending/in-review text is preserved chronological source history, not a current denial. Runtime-hardening #222 edge is satisfied by this existing-node completion; no edge/node/status is otherwise altered. Its five-minute/examples gates remain separate.

Campaign gate retains owner exact-merge mainCI/Docs/field follow-through; terminal receipts are not supplied at this owning completion source and are not invented as passed. No installed migration, live runtime, #201/#224 credit, five-minute outcome/start, examples, macOS or debt clearance follows. Later Campaign priority-hold lift/root discovery and tracker232 are separate qualifications; this packet does not deny them, launch their runner/subjects or execute an installation/test.

Complete actual owner readiness source:

```text
#222 / PR #228 is Ready for operator at exact a6581875c1a36aafcb9c083d8d1cf071aee266ad: https://github.com/lossyrob/telex/pull/228#issuecomment-6428394245. Exact-head DESIGN PASS6428153664 and independent APPROVE6428243977; all21 CI jobs in run40407195526 passed, including Linux guard/restored-pair detector tests. Approval validation notes acknowledged without weakening tests. Ready/CLEAN/MERGEABLE, closing only #222, normal consequence. Current main cb747a8f adds only Streamliner metadata, so no further rebase/push. Operator owns merge; read-only lifecycle monitoring and postmerge field report remain mine. App restart recovery reconciled live GitHub and dead observer manifests; no duplicate push/comment.
```

Exact owning #222 completion section:

```text
Two Watcher status changes:
1. watcher-installation-bootstrap (#222) -> completed. PR https://github.com/lossyrob/telex/pull/228 merged 22:04:03Z at merge commit 7036e7103aae4416cf0a0e2030175dc901ece893 (head a6581875). Campaign merge-gate record: https://github.com/lossyrob/telex/pull/228#issuecomment-6428413609. CI 21/21 at the exact head (run 40407195526). Issue #222 closed 22:04:04Z. Owner e292322d is historical. runtime-hardening's #222 edge is now satisfied; it still waits on five-minute-custom-watch-gate and examples-accelerate-gate.
```

### Complete independentReview receipt 6428243977 (raw 3246 bytes / 8b617705abb2ca37d8551998c8ae6a03a1f8689d5c30326380abb126561a78f4; CRLF-to-LF display only)

```text
🐾 PAW Review: +1

Independent verdict: APPROVE for PR #228 at exact published head `a6581875c1a36aafcb9c083d8d1cf071aee266ad`, against #222. This supersedes my historical f006 verdict for the current review gate. No blocking product findings and no additional non-blocking product findings.

I reviewed the full main-to-head change from `f4c58eed5fe9336408750d69d59ae446707c9f88`, including the Linux repair. Local range-diff confirms both earlier implementation patches are unchanged by the rebase. D1/D2 remain resolved.

The repair meets the requested bootstrap behavior: the helper path is derived only as `watcher_process_guard` beside the recorded Watcher; Linux rejects missing/non-file/non-owner-executable helpers and verifies the recorded hash; Windows requires `processGuardSha256` to be null ([installation.ps1:171-180](https://github.com/lossyrob/telex/blob/a6581875c1a36aafcb9c083d8d1cf071aee266ad/telex-watcher/installation.ps1#L171-L180), [241-250](https://github.com/lossyrob/telex/blob/a6581875c1a36aafcb9c083d8d1cf071aee266ad/telex-watcher/installation.ps1#L241-L250)). The guide requires same-build pair retention and upgrade/rollback, and includes the standalone-copy detector lesson ([guide:40-51](https://github.com/lossyrob/telex/blob/a6581875c1a36aafcb9c083d8d1cf071aee266ad/docs/guide/src/guides/watcher-installation.md#L40-L51), [292-301](https://github.com/lossyrob/telex/blob/a6581875c1a36aafcb9c083d8d1cf071aee266ad/docs/guide/src/guides/watcher-installation.md#L292-L301)). The test-only error wrapper prints the terminating exception message and exits 1; the exact Entra feature-refusal assertion remains intact ([installation.rs:108-115](https://github.com/lossyrob/telex/blob/a6581875c1a36aafcb9c083d8d1cf071aee266ad/telex-watcher/tests/installation.rs#L108-L115), [517-527](https://github.com/lossyrob/telex/blob/a6581875c1a36aafcb9c083d8d1cf071aee266ad/telex-watcher/tests/installation.rs#L517-L527)).

### Independent validation and non-blocking limitations

- Rebuilt both Telex and Watcher at this head. PowerShell parsing and patch whitespace checks passed; all 14 `copilot_plugin` tests passed.
- The Windows installation run had 7 passes, 1 failure, and 1 ignored shared subprocess role. The failure was the preliminary 15-second `pwsh` `$HOME` probe timing out before installation code ran ([installation.rs:163-168](https://github.com/lossyrob/telex/blob/a6581875c1a36aafcb9c083d8d1cf071aee266ad/telex-watcher/tests/installation.rs#L163-L168)). Its unchanged-head targeted rerun passed in 9.05 seconds. This repeats the earlier local probe timeout and is a non-blocking test-reliability observation, not an all-green first-run claim or a demonstrated bootstrap defect.
- Linux-specific guard execution was not run locally. [Exact-head hosted CI](https://github.com/lossyrob/telex/actions/runs/40407195526) still had both Windows and Linux workspace test steps in progress when checked. The Linux-only refusal/restoration test and retained-pair detector proof therefore still require hosted execution evidence.
- This scope-limited review does not establish production installation or real root-turn delivery, and is not a merge-readiness claim. No GitHub self-approval review or merge was performed.
```

### Complete designReview receipt 6428153664 (raw 1723 bytes / 527b1fefb4dd1a40f2742f422d629a5a3a83cabe21814ee28f0e0d5579d5f95c; CRLF-to-LF display only)

```text
Design steward recheck at exact head a6581875c1a36aafcb9c083d8d1cf071aee266ad: DESIGN PASS. The Linux process-guard addition meets the five conditions I gave the owner, and D1/D2 from https://github.com/lossyrob/telex/pull/228#issuecomment-6427578746 still hold.

Checked read only. The head is three commits on main f4c58eed. The two earlier commits are the rebased f006 implementation; the guard commit is a6581875.
1. Derived path only: `Get-ProcessGuardHash` builds `watcher_process_guard` from the recorded Watcher executable's directory. That is the same place process.rs looks (current_exe directory). The record has no guard-path field. The exact 14-key set rejects extra keys.
2. On Linux, a missing guard, a non-file guard, no owner-execute bit, a malformed hash or a hash mismatch throws. `Assert-Record` serves create, verify and preflight, so the check runs before launch.
3. Elsewhere, `processGuardSha256` must be null. Non-null is rejected. The tool still refuses platforms other than Windows and Linux (line 22).
4. The guide states pair handling. Retention puts the same-build guard in the same directory. Upgrade and rollback retain, copy, verify and swap the Watcher/guard pair together and preserve the previous pair. A new Watcher must never sit beside an old guard.
5. The guide states the standalone-copy lesson in one line.

This adds no runtime contract. The guard and its lookup are unchanged from main.

The Entra test fix only reports the raw exception message. The exact assertion is unchanged, which is the right handling.

Linux execution is proven only when hosted run 40407195526 passes. CI and independent review remain the mechanical gates. This is a design result, not merge approval.

```

### Complete ownerReadiness receipt 6428394245 (raw 1372 bytes / b63a38524f9829100ea6252a88fa03f0b4c1cea0b5f6e409f7dee6c115c17e4c; CRLF-to-LF display only)

```text
Ready for operator at a6581875c1a36aafcb9c083d8d1cf071aee266ad.

Exact-head gates are complete: DESIGN PASS in comment 6428153664, independent APPROVE in comment 6428243977, and all 21 jobs in CI run 40407195526 passed. Linux logs confirm the guard refusal/restoration test, retained-pair detector proof and unchanged Entra refusal assertion executed successfully; the Windows installation cases also passed.

Approval notes were read in full and triaged. Retaining the reviewer's initial Windows result of 7 passes, 1 preliminary HOME-probe timeout and 1 ignored shared role alongside its successful unchanged-head rerun (9.05 seconds). This recurring pre-product startup timeout remains a non-blocking test-reliability observation; no timeout or assertion was weakened in this PR. Hosted results now supply the Linux execution evidence that was pending during review. Production installation and real root-turn delivery remain outside this proof.

The current head is unchanged, GitHub reports CLEAN/MERGEABLE, there are no review threads or outstanding product findings, and the only closing reference is #222. Main's subsequent cb747a8f commit changes only Streamliner metadata; no rebase or repeated push was needed. Consequence: normal. Operator owns merge; this session will continue read-only lifecycle monitoring and post-merge validation without merging.

```

### Complete campaignGate receipt 6428413609 (raw 1462 bytes / c4a280532d37f933e5ab595500c42dd302ba40ce19d44359a320aab596c5d7ec; CRLF-to-LF display only)

```text
Campaign gate: MERGE at exact head a6581875c1a36aafcb9c083d8d1cf071aee266ad (Closes #222).

Floor:
- PR open, not draft, MERGEABLE/CLEAN. Closing refs exactly [222]. 0 unresolved review threads.
- CI run 40407195526 attempt 1: pull_request, completed/success, 21/21 jobs success, headSha a6581875.
- Receipts at exact head: design steward DESIGN PASS 6428153664; independent PAW review APPROVE 6428243977; owner readiness 6428394245.
- Final-main drift: merge-base f4c58eed..current main cb747a8f changes only .streamliner/workstreams/local-daemon/* (4 files). That is disjoint from the PR's 10 product/docs/test files, so the disjoint-drift exception applies and no re-run is required.

Merits (Campaign read): installation.ps1 creates, verifies or preflights a non-secret installation record only. It never installs binaries or starts a runtime. It requires absolute paths and refuses linked/reparse or foreign-owned/foreign-writable ancestors on Windows and Linux. cli.rs only adds a long_version stating the build's backend and Entra features. Guidance and tests bind the stall guide to the default record lookup (TELEX_HOME, otherwise ~/.telex) and keep pull fallback and explicit re-arm text. No runtime, auth, ACL or production state is changed by this PR.

Not credited: #201 acceptance, #224 repair, the five-minute custom-watch gate outcome, or any installed-Watcher migration. After merge, the owner owns exact-merge main CI/Docs and the field report.
```


### Later exact-merge field and Campaign acceptance

Actual Campaign `toolu_01P2hV91frZ3muRGj7wzKTvn` now accepts owner field6428776300 and exact-merge7036 CI40409517934/Docs40409517942 follow-through. This later qualification fulfills the previously pending obligation; all earlier source-time pending statements above remain exact history. The owner is historical and the same #222 node/row is completed/watch, without new staffing. Only complete field and bounded run metadata were independently matched. Reported CI21/21/Docs2/2, installation8pass1ignored per Windows/Linux, plugin14, isolated detector/guard/refusal/registry and design/source methods remain attributed. The field retains the preliminary HOME-probe timeout/9.05s rerun and packaging/error-format lesson without weakening assertions. No installed migration, root/App wake, #201/#224, five-minute gate or debt outcome follows.

Complete later field (4614 bytes / be3a9e066c07aa7e3bae30961b19319d7a85a2d5c720a412f0ca093e08c3152c; CRLF-to-LF display only):

```text
## Field report: Watcher installation bootstrap

Delivered in [PR #228](https://github.com/lossyrob/telex/pull/228), merged at `7036e7103aae4416cf0a0e2030175dc901ece893`. Post-merge validation is complete at that exact SHA:

- [Main CI 40409517934](https://github.com/lossyrob/telex/actions/runs/40409517934): completed successfully, all 21 jobs passed.
- [Main Docs 40409517942](https://github.com/lossyrob/telex/actions/runs/40409517942): guide build and GitHub Pages deployment both passed.
- The merge introduced only the previously accepted Streamliner metadata drift relative to reviewed head `a6581875`; product files are unchanged. [Merge gate](https://github.com/lossyrob/telex/pull/228#issuecomment-6428413609).

### What changed

The [PowerShell 7.4+ bootstrap](https://github.com/lossyrob/telex/blob/7036e7103aae4416cf0a0e2030175dc901ece893/telex-watcher/installation.ps1) exclusively creates a private schema-v1 record and its hashed companion verifier. Lookup uses an explicit operator record path, otherwise exactly `<TELEX_HOME or ~/.telex>/watcher/installation.json`. Verification is credential-value-free; launch preflight separately checks the required environment. The record pins selected Telex identity, retained Watcher and, on Linux, the derived adjacent process guard. Windows requires a null guard hash.

The [installation guide](https://github.com/lossyrob/telex/blob/7036e7103aae4416cf0a0e2030175dc901ece893/docs/guide/src/guides/watcher-installation.md) and mirrored stall templates now provide record discovery, feature prerequisites, launch, readiness, shutdown and explicit older-installation/rollback handling. Bridge push and single-shot pull fallback remain distinct delivery modes.

### What held

The exact-merge [Linux job](https://github.com/lossyrob/telex/actions/runs/40409517934/job/121876137054) and [Windows job](https://github.com/lossyrob/telex/actions/runs/40409517934/job/121876137030) each ran the installation target successfully: 8 passed, 1 shared subprocess-role fixture ignored. Logs confirm default/explicit discovery, exclusive creation and refusal cases, credential-free verification versus preflight, live isolated registration, successful detector execution, shutdown and retained registry. Linux additionally passed guard absence/non-file/non-executable/hash-change refusal and exact-pair restoration. The unchanged authentication-feature refusal assertion passed on both platforms. All 14 Copilot plugin/source-contract tests passed on the main Windows job.

The initial Linux failure at the earlier head exposed a packaging prerequisite: Watcher alone can start and attach but cannot execute Linux detectors. Retaining the same-build adjacent guard repaired the fixture and procedure. The second failure was PowerShell's wrapped stderr splitting the expected feature diagnostic; tests now emit raw exception messages without weakening the assertion. Exact-head [design inspection](https://github.com/lossyrob/telex/pull/228#issuecomment-6428153664) and [independent review](https://github.com/lossyrob/telex/pull/228#issuecomment-6428243977) passed before merge.

### Successor guidance and evidence limits

Runtime hardening should consume this documented 14-field schema rather than infer installation values. Retain, verify and swap Watcher and its same-build Linux guard as one pair in a separate directory. Each record has its own private directory; creation refuses an existing record, companion tool or registry. A copied verifier left after a late record-write failure is retained, never overwritten. Run preflight in the actual launch environment and use the guide's explicit cutover procedure for older installations.

Retain the independent review's local reliability evidence: 7 installation passes, 1 preliminary 15-second PowerShell HOME-probe timeout before product code, and 1 ignored role; its unchanged-head targeted rerun passed in 9.05 seconds. This recurring harness-startup observation was reported to the coordinator for test-sentinel/#182 routing. No bootstrap outcome failure was established, and no timeout or assertion was relaxed.

This proves the isolated Windows/Linux bootstrap outcome and published documentation, not production installation, root-turn delivery, #201 acceptance or a live installed-Watcher migration. Issue #222 remains closed.

**Design impact: updated-docs.** The approved installation/discovery contract and Linux pair requirement are documented; the runtime contract, backend authentication and release-upgrade mechanisms are unchanged. No design decision remains open.

```

Complete actual owner terminal source:

```text
#222 / watcher-installation-bootstrap is delivered and post-merge validated; field report: https://github.com/lossyrob/telex/issues/222#issuecomment-6428776300. Exact merge7036e710 CI40409517934 passed21/21 jobs and Docs40409517942 built/deployed successfully; design impact updated-docs, issue stays closed, Campaign acceptance requested.
```

Complete actual campaign terminal source:

```text
#222 follow-through accepted. Owner e292322d posted field report 6428776300. Main CI 40409517934 passed 21/21 and Docs 40409517942 passed 2/2 at merge 7036e710. Node `watcher-installation-bootstrap` is completed. No #201 or #224 credit.
```

Complete actual owningWatcher terminal source:

```text
Campaign accepted #222 completion (Campaign message toolu_013nrrGWN5yqLnx16jXbs1YH), adding to my earlier completion report. Owner e292322d's follow-through is accepted at merge 7036e710: main CI passed 21/21 and Docs 2/2. Field report: https://github.com/lossyrob/telex/issues/222#issuecomment-6428776300.
```


## #225 brief source-display convention (2026-10-26)

The quoted owning reopening excerpt below omits exactly one trailing ASCII space after its final sentence for repository whitespace hygiene. Its complete original excerpt and actual incoming body remain unchanged in the ledger and bound source. The ledger records the exact one-character display projection, body line and full append byte/SHA pins; adding that U+0020 back recovers the complete original append. This is a declared reversible display-only change, not a rewritten source, authority or outcome. All other source text and raw public-body custody remain unchanged.


## #225 same owner, public activation and initial implementation/CI history (2026-10-26)

Watcher `toolu_01WzN28S1CERA3BRZ8U9xfdC` reports sole App owner `23511e04-e331-4bf3-b44d-c5b97f011a52`, start `fc962a6ab7d12f40b004b4eed6b5d18700292a34`, initial branch `lossyrob-probable-robot` and kickoff delivery at 21:09Z: `ctx-20261026210729-4eec17f3/kickoff.md`, 16,491 bytes / `83d39d35670935370da00988b2445fcea3d6ed9069b5c60fccb30a1c2e3b4faf`. The launch source initially required planned status until a public claim/PR. The later public claim [6427435102](https://github.com/lossyrob/telex/issues/225#issuecomment-6427435102) at 21:12:02Z meets that condition; this same node/row becomes in-progress, attention watch unchanged. Watcher attributes CLI `d501f9e9-f5a6-46ea-843d-7b05cc720178` to that same App owner; no recipient/native lookup or second worker is claimed here. Later reported branch is `lossyrob-watcher-failure-diagnostics`; no rename time/mechanism or current checkout proof is inferred.

The complete current binding source is 2,403 bytes / `96ab97a3b3581c193b6c94ee9d54cea76bda9dd9c57cba62780dd36de48967b9`, observed at 21:35:13.859Z. Its six named absolute private-path leaves alone become checkout/context/document names in the ledger, with a complete object inverse. No referenced context, handoff, kickoff, launch adapter, domain note or workflow file was followed. The retained literal updatedAt 21:09:38Z precedes the included public claim at 21:12:02Z; it does not date later-added publicClaim/branchRenamedTo/cliSessionId fields, nor does this later copy prove launch-time bytes. The declared gpt-6.1-sol/high/default/interactive profile and coordinateWithCreator=false remain coordinator-recorded requested snapshots, not an effective profile, capacity, delivery or current activity observation.

Initial public-claim source says no PR yet. The later same-owner source reports draft [PR #229](https://github.com/lossyrob/telex/pull/229), Closes #225, first head `de96d59ec0d77e4143a9cd999f0c58505f889985`, opened 21:21:07Z. Implementation/local-regression completion is owner-attributed; independent exact-head review, design inspection and hosted CI were pending at that source. These snapshots are not current/exact-head approval and do not deny later reports. Watcher reports adoption of PR #221 three cfg-only `telex-watcher/src/process.rs` warning fixes so diagnostics tests build on macOS under `-D warnings`, with no containment behavior change; no product diff or build was inspected here.

A later Watcher report retains initial run `40403620740`, job `121855619687`, macOS failure of `registry_ownership_and_input_codes_do_not_gain_runtime_failure_diagnostics`: `Runtime::open` hit `registry open -> database is locked`. Owner is reworking the fixture to hold the Unix ownership lock directly rather than rely on SQLite startup. This is attributed evidence for the #195 startup question, not its actual native stage/cause, a fixture/product disposition, cause fix, common failure family or readiness. No CI run/job/log, product source, fixture or runtime was read/replayed by this reconciler.

The existing 4,096-byte redacted stage-labeled stderr/marked-truncation contract, unchanged stdout envelope/runtime-failed/exit 1/other specific codes, cross-platform automated coverage, permitted local path allowance and secret exclusions remain exact. No task Markdown, new row/node/failure count, owner/observer launch, native inspection, implementation acceptance, runtime/containment outcome or debt credit is added. #195/PR #221 PRODUCT225 hold remains unchanged; no rerun, rebase, push, merge or ordinary-CI diagnostic is authorized here. No #201 validation, root re-arm, five-minute release, installed upgrade or environmental clearance follows.

### Complete publicClaim source `toolu_01U6brZ9MwvKJLggkk3KhY1V`

```text
#225 status change: watcher-runtime-failure-diagnostics is now in-progress. Public owner claim and plan: https://github.com/lossyrob/telex/issues/225#issuecomment-6427435102 (21:12:02Z). It names CLI session d501f9e9-f5a6-46ea-843d-7b05cc720178, which resolves to the app session 23511e04-e331-4bf3-b44d-c5b97f011a52 I launched, so this is the same single owner. The branch is now lossyrob-watcher-failure-diagnostics. There is no PR yet. Scope note: #225 will carry PR #221's three cfg-only telex-watcher/src/process.rs warning fixes so its macOS diagnostics tests build under -D warnings. There is no containment behavior change.
```

### Complete firstPr source `call_YLt2ZB9UWwN5tbQqyAKhA45j`

```text
#225 has draft PR https://github.com/lossyrob/telex/pull/229 at de96d59ec0d77e4143a9cd999f0c58505f889985. Implementation and local regressions are complete; independent exact-head review, design inspection and hosted CI are pending.
```

### Complete initialMacosFailure source `toolu_013acG63pFo9ga8En1E2HwGU`

```text
#225 evidence: draft PR https://github.com/lossyrob/telex/pull/229 (Closes #225), head de96d59ec0d77e4143a9cd999f0c58505f889985, branch lossyrob-watcher-failure-diagnostics, opened 21:21:07Z by the sole owner 23511e04. Status stays in-progress. Initial CI run 40403620740, job 121855619687, failed the new test registry_ownership_and_input_codes_do_not_gain_runtime_failure_diagnostics on macOS: Runtime::open hit "registry open -> database is locked". The owner is reworking that test to hold the Unix ownership lock directly rather than depend on SQLite startup. This is retained evidence for the #195 startup question, not a cause fix and not readiness.
```


### Complete public owner claim and separate current PR metadata

Claim body 885 bytes / `aa8b427ff2aa30ed7f2cef64553d859fa2e6b3419f34c6905c27a9e5f93da72f`; the CLI-to-App relationship remains Watcher-attributed, not a native lookup here. Bounded current public PR snapshot names head `12b164a51765420a60b0eeadfd7be8f5635edebe`, state `closed`, draft=false, parsed closing references [225]. That is distinct from first-reported de96d59e and not a current gate or source/CI/runtime review. All first-PR/initial-failure statements above remain chronological reports, not current denials.

```text
Owner: session d501f9e9-f5a6-46ea-843d-7b05cc720178

I will attach stage context at the registry, endpoint, runtime startup/execution, heartbeat and management-server boundaries, then emit one bounded, redacted stderr cause chain for runtime-failed. The JSON envelope shape, specific error codes and exit status remain unchanged; the generic message will point to stderr.

Coverage will force startup failure and check redaction, the 4096-byte cap with a truncation marker, and the CLI contract. Linux/Windows workspace CI will run the new tests; a focused macOS job will run them with warnings denied. I will reuse PR #221's three cfg-only warning fixes without changing containment behavior.

The runtime guide, design and exit-code reference will describe the diagnostic. The macOS startup cause stays with #195 after this item lands; installation bootstrap stays with #222.

```


## #225 later macOS prerequisite block and bounded test-boundary reopening

Watcher `toolu_01XJnKUAoyYGrNoQAHwE3Vdq` reports this same owner/node blocked after draft718b705f macOS40411290147/job121882332938 stopped at runtime open -> registry open -> SQLite5 before reaching the verified injected CORRUPT11 cause. The preceding write-time trigger40410492025/job121879665853 stopped at the same reported stage. Preserve both failed fixture snapshots, design/independent pass at718 and the halted iteration, without a #195 native cause, fixture/product repair, common-cause or readiness claim.

Actual steward6428753387 later chose a narrow test boundary: only sqlite_startup_failure_omits_environment_values_dsns_and_database_error_text receives the macOS-only ignore with its exact #233 reason. The body, Error code11 and redaction assertions remain unchanged, and every other test still runs on all platforms. Watcher `toolu_01Ks72xgpLrctV5EUeHSXkdw` reports blocked -> in-progress and relayed the exact attribute-only change to existing owner23511e04. Earlier no-skip/blocked/every-platform statements above remain the complete earlier phase, not denials of this later authorized boundary. No implementation delta or new head readiness is independently inspected or granted here.

Separate #233 must fix the macOS registry-open cause, remove this one ignore and pass exact-head hosted macOS, without weakening ownership/lock/store identity or #172 HOLD. Its node/checkpoint/staffing publication is separate. #195 remains governed by its single post-229merge rebased diagnostic; add the pre-authorized #233 edge only if that actual run shows registry-openSQLite5. No such result/edge/PRODUCT225 release is supplied by this packet. No new owner, source/log/runtime replay, installation, root test or retry/rearm is authorized.

Complete earlier owning blocked source:

```text
Status change: watcher-runtime-failure-diagnostics (#225) in-progress -> blocked. Evidence: https://github.com/lossyrob/telex/pull/229#issuecomment-6428718574 (22:21:35Z).
- PR #229 is a draft at head 718b705f3058052e5066e6b2c982d198e6803760.
- Independent review and design checks pass at that head, but macOS CI fails: run 40411290147, job 121882332938.
- The failing test is sqlite_startup_failure_omits_environment_values_dsns_and_database_error_text. On macOS the Watcher executable stops with "registry open -> SQLite code 5 database is locked" before the injected fixture error is reached. The earlier write-time fixture hit the same error: run 40410492025, job 121879665853, comment 6428609612.
- The owner (23511e04) stopped fixture iterations and will not skip, relax checks or fix startup.
- The blocker is a macOS registry-open prerequisite or a test-boundary decision. I've routed it to the steward. No #195 cause attribution.
```

Exact later owning reopening prefix (#232 held-runner section is separate):

```text
Status change: watcher-runtime-failure-diagnostics (#225) blocked -> in-progress. Steward boundary decision: https://github.com/lossyrob/telex/issues/225#issuecomment-6428753387 (22:23:28Z). I relayed the exact test-only cfg_attr change to owner 23511e04. On your #233 node (from the steward's change): staffing is mine. I'll launch its owner after #225 completes and the node is published. I relayed the #195 note (https://github.com/lossyrob/telex/issues/195#issuecomment-6428756204) to the Sentinel and asked it to send the rebased-run evidence to both of us.
```

### Complete macosWriteFixture public source 6428609612 (1323 bytes / 1b0c43b182e49dfaedf3254ed5d5159efe4d28d3e31e72c7c3df38b5e418fd8b; CRLF-to-LF display only)

```text
Integrated-head CI observation, not a #195 cause finding:

- Head: d331499f72bd859e20ee1d4878d54d39882b12fb
- Run: https://github.com/lossyrob/telex/actions/runs/40410492025
- Job: https://github.com/lossyrob/telex/actions/runs/40410492025/job/121879665853
- Test: sqlite_startup_failure_omits_environment_values_dsns_and_database_error_text
- Failure: the executable reports SQLite code 5 before reaching the fixture's code-1811 trigger.

Exact relevant excerpt:

```text
thread 'sqlite_startup_failure_omits_environment_values_dsns_and_database_error_text' (12400) panicked at telex-watcher/tests/runtime_diagnostics.rs:144:5:
runtime-failed
  caused by: runtime open
  caused by: registry open
  caused by: SQLite: Error code 5: The database file is locked
  caused by: Error code 5: The database file is locked
```

All 11 binary/unit tests and the other two executable diagnostics tests passed in this job. The strengthened exact-code assertion correctly exposed that the secret-bearing trigger was not reached on macOS; this is not a reason to weaken the assertion, skip the macOS executable test, or fix product startup behavior within #225.

This is further evidence for the separately owned #195 diagnostic lead, not attribution of its early exit. No unchanged-head rerun has been requested.

```

### Complete macosReadFixture public source 6428718574 (2478 bytes / e1cc8f79afb744d53321a1824345ffa36fe1370724fd97edee1521daa4a3d2ae; CRLF-to-LF display only)

```text
PR #229 is blocked on the macOS executable SQLite diagnostic path at head 718b705f3058052e5066e6b2c982d198e6803760.

- Run: https://github.com/lossyrob/telex/actions/runs/40411290147
- Job: https://github.com/lossyrob/telex/actions/runs/40411290147/job/121882332938
- Test: sqlite_startup_failure_omits_environment_values_dsns_and_database_error_text
- The raw schema-read assertions completed: the fixture produces SQLITE_CORRUPT code 11 and contains private text.
- The branch executable instead returned SQLite code 5 before the expected code-11 assertion. The test failed rather than accepting a substitute error.

Exact excerpt:

```text
thread 'sqlite_startup_failure_omits_environment_values_dsns_and_database_error_text' (13262) panicked at telex-watcher/tests/runtime_diagnostics.rs:164:5:
runtime-failed
  caused by: runtime open
  caused by: registry open
  caused by: SQLite: Error code 5: The database file is locked
  caused by: Error code 5: The database file is locked
```

The 11 binary/unit tests, executable I/O diagnostic and ownership/input precedence tests passed. The earlier write-time trigger fixture also returned code 5 in the executable (run 40410492025, job 121879665853; preserved in comment 6428609612). Changing the injection to a verified read-time error did not unblock the required executable assertion.

No further fixture workaround, platform skip, broadened accepted code, unchanged-head rerun, or product startup fix is being attempted here. This is additional evidence for the separately owned macOS startup lead, not a claim about #195's early-exit cause. The PR remains draft and not ready; the coordinator has been asked to route the prerequisite within the agreed boundary.

Independent review passed at this same head: https://github.com/lossyrob/telex/pull/229#issuecomment-6428684988. Both non-blocking notes have been read and dispositioned:

- Pre-existing heartbeat/cleanup error precedence is unchanged. Preserving multiple simultaneous failures is outside #225's requirement; no change is requested or included here.
- Separate Rust panic-hook output is not part of this formatter's diagnostic. The review demonstrated no production input path to the synthetic panic; adding a global panic/logging framework is not justified within this item.

These are recorded scope dispositions, not outstanding requested changes. Neither review pass nor design pass substitutes for the failed hosted CI gate.

```

### Complete stewardTestBoundary public source 6428753387 (2726 bytes / 976a48cbf1bb8f721862ba816b5a269eb1879cdb97a01616de0cce4649e5b847; CRLF-to-LF display only)

```text
Steward decision: test boundary for #225 (PR #229), and a new upstream item #233 for the macOS registry-open lock. Option (c) from the coordinator.

**Why this is a real boundary.** On hosted macOS, the Watcher executable fails at `registry open` with SQLite code 5 before it can reach any injected SQLite cause. Two independent fixtures both failed this way: the write-time trigger and the verified read-time SQLITE_CORRUPT (https://github.com/lossyrob/telex/pull/229#issuecomment-6428718574). Fixing startup is outside #225 by its node definition ("Do not diagnose/fix the #195 macOS cause"), and widening #225 to do it would mix a product repair into a diagnostics item. So #225's requirement to test "on every platform where the binary builds" is narrowed for exactly one case: the end-to-end SQLite-cause executable test on macOS. That case moves to #233.

**Required change to PR #229 (test only):**

1. On `sqlite_startup_failure_omits_environment_values_dsns_and_database_error_text` only, add `#[cfg_attr(target_os = "macos", ignore = "blocked by #233: macOS registry open returns SQLITE_BUSY")]`. The test is reported as ignored, with its reason, on macOS.
2. Do not change the test body, the exact `Error code 11:` assertion, or the redaction assertions.
3. Do not skip, relax or replace any other test on any platform.
4. The macOS I/O, ownership/input-precedence and unit diagnostics tests keep running there.
5. State the boundary and #233 in the PR description.

Everything else in PR #229 is unchanged; the design passes at d331499f and 718b705f carry forward.

**Readiness:**

- **#225 / PR #229.** The design returns to PASS at the head that adds only that attribute, after I confirm the delta. That head still needs green hosted CI, including macOS, plus Campaign's gate. The independent review at 718b705f covers the test body, but whether the attribute-only delta needs another review is the reviewer's and Campaign's call. Merging closes #225.
- **#233** (new node `watcher-macos-sqlite-registry-open`) depends on #225. Its closing PR fixes the macOS cause, removes the attribute, and passes hosted macOS at its exact head. Staffing and priority are the coordinator's.
- **#195.** Its plan does not change: after #229 merges, it runs one rebased diagnostic. If that run shows `runtime open -> registry open -> SQLite code 5`, #195 becomes blocked on #233. The reconciler adds that edge on that evidence without coming back to me. If the run shows a different cause, #195 proceeds on its own evidence and no edge is added. Either way, PR #221's post-merge run is attribution evidence only; it is not a fix and not a reason for #195 to absorb #233.

No change to #172's HOLD or to Windows/Linux behavior.

```


## #225 accepted product merge and owning bounded completion

Actual Campaign `toolu_01MRuxaXXyYpkLBr1vF7nLqm` reports merged [PR #229](https://github.com/lossyrob/telex/pull/229), exact candidate `12b164a51765420a60b0eeadfd7be8f5635edebe`, merge `00c0edfab22e58583d8ef6f2bd47425872cc4bfa` at 22:47:41Z, closing #225 at 22:47:42Z. Owning Watcher `toolu_01Tq17HtJhxEyhq2QsBL17Ap` changes the same diagnostics node from in-progress to completed. Record completed/watch with the same historical owner, no new node/row/worker or edge. Complete earlier launch/claim/first-draft/CI-failure/block/boundary/reopening statements remain chronological source snapshots, not current denials or retroactive readiness.

Read the complete Campaign gate: CI 22/22, exact-head design, reviewer carry over the approved four-line macOS ignore and disjoint-drift merits remain Campaign/Watcher methods. This reconciler matches only complete public bodies and bounded merge/closing metadata, without source/job/log/runtime replay. The one deferred macOS case and #233 cause/ignore-removal/hosted proof remain separate; ownership/lock/store identity and #172 HOLD are unchanged. No macOS repair, #195 cause/edge or #201 acceptance follows.

At the owning completion source, exact-merge main CI 40415150085 and Docs 40415150116 are still running and the field is pending. This records neither a current poll nor passed follow-through. The same owner stays for those obligations; later actual results need their own qualification. The owning source reports the single rebased #195 diagnostic may now run under its existing steward decision. This packet does not run or reauthorize it; the conditional #233 edge still waits on actual registry-open -> SQLite5 evidence. Watcher reports #233 dependency-ready but will staff only after #233 publication; this packet creates no #233 shape/owner or launch. No installation/upgrade/environmental cleanup, root/wait/retry, five-minute/examples outcome, debt credit or successor authority is granted.

Complete actual Campaign source:

```text
#229 merged at 00c0edfa (gate 6429148380; ledger cedd23d) and closes #225. The reviewer's PASS was carried to the exact head over the 4-line macOS ignore deferred to #233; design 6428803538; CI 22/22. The drift exception for 7036e710..c4f6ac8c is file- and API-disjoint. Owner 23511e04 owns the follow-through. No credit for #233, #195 or #201.
```

Complete actual owning completion source:

```text
Status change: watcher-runtime-failure-diagnostics (#225) in-progress -> completed.
- PR https://github.com/lossyrob/telex/pull/229 merged at 22:47:41Z as merge commit 00c0edfab22e58583d8ef6f2bd47425872cc4bfa (head 12b164a51765420a60b0eeadfd7be8f5635edebe). The head contains the steward's single macOS test ignore, which defers the SQLite registry-open failure to #233.
- Campaign gate record: https://github.com/lossyrob/telex/pull/229#issuecomment-6429148380. Issue #225 closed at 22:47:42Z.
- The owner reports exact-merge main CI 40415150085 and Docs 40415150116 still running, with its field report to follow. I'll send any CI failure.
- Owner 23511e04 stays for follow-through only.
- #233 is now dependency-ready, but I'll launch its owner only after the #233 node is published.
- The Sentinel's single rebased #195 diagnostic may now run; its conditional #195 -> #233 edge still waits on that evidence.
```

Complete public gate6429148380 (1731 bytes / c3623db105a93c2c76ca6c9db02e65bbcd6f5ea75e4a9bfa0691642a473453c5; CRLF-to-LF display only):

```text
Campaign gate: MERGE at exact head 12b164a51765420a60b0eeadfd7be8f5635edebe.

Floor:
- Parsed closing references: [225] only. Review threads: 0. MERGEABLE/CLEAN.
- Hosted CI run 40412375343 (pull_request, head 12b164a5, attempt 1): 22/22 jobs succeeded. The macOS diagnostics job 121886040344 ignores only the SQLite case approved in the #225 boundary decision (6428753387), with the reason "blocked by #233".
- Independent review: the existing reviewer's PASS from 718b705f explicitly carries to this exact head (6428684988). The 718b705f..12b164a5 delta is only the 4-line macOS cfg_attr(ignore), which Campaign verified.
- Design: steward exact-head DESIGN PASS (6428803538).
- Readiness/preflight: 6429132328.

Main drift: PR base 7036e710; current main c4f6ac8c (#227). Drift 7036e710..c4f6ac8c touches .streamliner, CONTRIBUTING.md, docs/design/daemon.md and src/backend, src/daemon.rs, tests/daemon_core_postgres.rs, tests/fixtures/pg_latency_proxy.rs. Campaign grants the disjoint-drift exception because the drift is file- and API-disjoint:
- None of #229's 9 paths (telex-watcher/*, docs/design/watcher.md, docs/watcher-runtime.md, the exit-codes guide page, ci.yml) is touched.
- #229 adds no new telex library API use.
- #227 only added a Backend trait method with a default implementation and changed no pub signatures used by telex-watcher (SqliteBackend, Backend, postgres::make_tls).
The PR does not get credit for latest-main execution; exact-merge main CI is the execution check.

Scope: this merge gives no credit for macOS SQLITE_BUSY (#233 owns the fix, removing the ignore and hosted macOS proof), for #195 or for #201. Normal consequence. The owner owns exact-merge main CI/Docs and the field report on #225.
```


## #225 later accepted exact-merge follow-through

After the earlier owning completion reported running CI/Docs and a pending field, owner `call_6RDdkxkGPY0KG4PxxeIpZaf4` supplied the complete field [6429402955](https://github.com/lossyrob/telex/issues/225#issuecomment-6429402955). Direct Campaign `toolu_01VgaKbu27AMgwTi2sWvmZtC` accepted exact-merge CI 40415150085 (reported 22/22), Docs 40415150116 (reported 2/2) and the field at `00c0edfab22e58583d8ef6f2bd47425872cc4bfa`. Owner `call_b0vjtNWNvfPo4PCdGFwKVG0i` then confirmed acceptance and no remaining owner duty. Keep that owner historical and the bounded node completed/watch. The earlier pending reports and the field's acceptance-pending last line are unchanged source-time statements, fulfilled by this later actual authority, not current denials or retroactive invented success.

Independently match only the complete public field and exact-merge push/attempt 1/completed-success run metadata. Job totals, macOS 11 unit/2 executable/1 ignored, Windows 11 unit/3 executable/zero ignored, raw CORRUPT11 fixture proof, ordinary lifecycle, isolated validation/cleanup and design updated-docs remain owner/Campaign attribution, not product/source/job/log/runtime/installation or registry replay. #233 still owns the macOS registry cause, ignore removal and unchanged hosted assertion proof; no #195 cause or #201 acceptance follows. No new owner, node, row, edge, experiment, install/upgrade/environmental outcome, #172 containment claim or debt/hold credit is granted.

Complete actual owner source:

```text
Reconcile #225's merged/post-merge evidence from the verified field report: https://github.com/lossyrob/telex/issues/225#issuecomment-6429402955. PR #229 merged as 00c0edfab22e58583d8ef6f2bd47425872cc4bfa, #225 is closed, exact-merge CI 40415150085 and Docs 40415150116 succeeded, isolated validation passed, and design impact is updated-docs; Campaign acceptance is requested, while #233 still owns the macOS SQLite repair and ignore removal (no completion credit for #233, #195 or #201).
```

Complete actual campaign source:

```text
#225 follow-through accepted. Field report 6429402955, CI 40415150085 22/22 and Docs 40415150116 2/2 all passed at exact merge 00c0edfa. The node is complete. #233 keeps the macOS obligation; no credit for #233, #195 or #201.
```

Complete actual ownerConfirmation source:

```text
Campaign has now explicitly accepted #225's post-merge field report https://github.com/lossyrob/telex/issues/225#issuecomment-6429402955 after independently verifying exact-merge CI 22/22 and Docs 2/2, and declared the node complete with no remaining owner duty; update the prior acceptance-pending state accordingly.
```

Complete public field (3673 bytes / b32f6b4bb1b13d8ec8b56320a6acae28b63b8a5f281dbf45d42dfc76b398eb64; CRLF-to-LF display only):

```text
## Field report: #225

[PR #229](https://github.com/lossyrob/telex/pull/229) merged as `00c0edfab22e58583d8ef6f2bd47425872cc4bfa` at 2026-10-26T22:47:41Z; #225 is closed. Main CI, Docs, and isolated local validation passed at that exact merge, including the intervening main changes covered by the [Campaign gate](https://github.com/lossyrob/telex/pull/229#issuecomment-6429148380).

### What held

Generic `runtime-failed` errors now produce a stage-labeled cause chain on stderr, capped at 4096 UTF-8 bytes including the newline and any truncation marker. The stdout envelope schema, error code, and exit 1 remain intact. Ownership/input error codes retain precedence and add no runtime-failure diagnostic; typed transport precedence also remains covered.

Secret-named environment values are redacted at any nonempty length; other environment values are selected from 8 UTF-8 bytes. Static stage labels and OS error numbers survive short ordinary values such as `1`, `C`, and `true`. SQLite messages, embedded credentials/DSNs, and task payloads do not pass through as arbitrary diagnostic text. Returned shutdown/task failures use the same bounded path.

### Exact-merge evidence

| Evidence | Result |
|---|---|
| [Main CI 40415150085](https://github.com/lossyrob/telex/actions/runs/40415150085) | Completed successfully; all 22 jobs passed at `00c0edfa`. |
| [Docs 40415150116](https://github.com/lossyrob/telex/actions/runs/40415150116) | Guide build and GitHub Pages deployment both succeeded at `00c0edfa`. |
| [macOS diagnostics job 121895475772](https://github.com/lossyrob/telex/actions/runs/40415150085/job/121895475772) | 11 unit tests and 2 executable tests passed; the single approved SQLite executable test was ignored with the #233 reason. |
| Local Windows diagnostics | 11 unit tests and all 3 executable tests passed, with zero ignored. The SQLite fixture first proved raw code 11 with private text, then verified the executable retained code 11 without that text. |
| Local ordinary runtime lifecycle | `executable_runtime_supports_complete_ordinary_lifecycle_and_safe_defaults` passed. |

Local validation used absolute binaries built from the exact merge, unique absolute `TELEX_HOME`, `TELEX_DB`, and `TELEX_INSTALL_ROOT`, and temporary user/`LOCALAPPDATA` roots. The isolated runtime was shut down and fixture directories removed. The operator's installed runtime, real user-directory permissions, and main checkout were untouched; the owner worktree is clean.

### What successors should know

The [approved boundary](https://github.com/lossyrob/telex/issues/225#issuecomment-6428753387) leaves only `sqlite_startup_failure_omits_environment_values_dsns_and_database_error_text` ignored on macOS, with its body and exact-code/redaction assertions unchanged. [#233](https://github.com/lossyrob/telex/issues/233) owns the macOS registry startup repair, removing that ignore, and passing the unchanged executable test on hosted macOS. This is not a diagnosis of #195's early exit. #195's rebased ordinary diagnostic and #201's work remain independent; this report gives none of those items completion credit and makes no macOS containment claim.

Design impact: updated-docs. The Watcher design, runtime guide, and exit-code reference describe the diagnostic, redaction threshold, preserved stages, and stderr inspection. No startup store or logging framework was added.

Consequence: normal - error observability and failure propagation change, while the successful lifecycle and machine-readable envelope contract are preserved.

Post-merge follow-through is complete; Campaign acceptance of this report is pending.

```


## #233 planned macOS registry-open repair, qualified external #195 consumer (2026-10-26)

Add the steward-requested planned/watch task-type graph node, its existing #225 prerequisite, the exact #225 test-ignore-removal summary suffix and usable-watcher membership. Add no task Markdown or ledger row. Every old node/root/ordering/dependency/attention/status/debt/wake stays exact apart from that one requested #225 summary suffix and checkpoint membership. Watcher alone owns later staffing after #225 completion AND verified node publication; this reconciler launches nobody.

The single post-PR229 diagnostic reported by the Sentinel at candidate/checkout `fa99d3d1940a0102a33831d50fd5e3721c177f2c`, base `00c0edfab22e58583d8ef6f2bd47425872cc4bfa`, run40416658503 attempt1/nativejob121900589733 reportedly matches runtime open -> registry open -> SQLite code5. This fulfills the condition in preauthorization6428756204 on evidence6429504931. Its earlier literal add-edge wording and the Sentinel/owning requests are preserved below, then qualified by the later owning external representation and explicit steward clarification6429646368 rather than projected as a dangling graph edge.

Per prior canonical steward6425865009 and explicit public clarification [6429646368](https://github.com/lossyrob/telex/issues/195#issuecomment-6429646368), Sentinel #195 / PR #221 remains external discovery/consumer provenance outside Watcher geometry; its workstream owns the external dependency. The steward calls the old "reconciler adds the edge" wording loose and explicitly directs no #195 node or dependsOn edge. Owning `toolu_0195Y2zYKV3kCW8zfBhSQrCP` likewise directs brief plus #233 node/task provenance, NO195node/row and NOdependsOn edge. Record draft PR221/fa99d3d1 as an external blocked consumer of separately owned #233, unchanged owner and operational block under Watcher coordination. The same qualification is appended to the new #233 summary, leaving the original requested summary as an exact prefix. All four canonical graphs have no195/221/233 tracker endpoint before this addition. #195 does not absorb the #233 fix; #233 retains product repair and SQLite test-ignore removal.

The reported186-byte complete stderr, childexit1/stdout169/Cargo101, eight native plus offline passes, unreached endpoint assertion, exact provider/checkout and raw74403/zeroNUL/SHA6c9166ba34bf089bc135214ec08c6aa8840c2c52bd8719b66102842a4d53f84b are Sentinel/public-source attribution only. No linked private custody, native job/log, product source or runtime was followed. No lock holder, contention cause or old9a/85cause, successful runtime acceptance, test-ignore-removal result, #195 repair, further push/diagnostic/rerun, new owner/observer or operational permission follows. #233 retains the product repair and approved ignore-removal work; no result is credited now.

### Complete original steward #233 shape; original node summary and #225 suffix are retained

```text
Steward graph change for telex-watcher. Evidence: https://github.com/lossyrob/telex/issues/225#issuecomment-6428753387 and tracker https://github.com/lossyrob/telex/issues/233.
1. Add node `watcher-macos-sqlite-registry-open`: type task, title "Open the SQLite registry on macOS", tracker lossyrob/telex #233, status planned, attention watch, repo telex, dependsOn [watcher-runtime-failure-diagnostics]. Summary: "Hosted macOS CI on PR #229 showed the Watcher executable failing at runtime open -> registry open -> SQLite code 5 (database file is locked) before reaching any injected cause (runs 40410492025 and 40411290147). One PR closes #233. It fixes the macOS cause, removes PR #229's macOS-only ignore on sqlite_startup_failure_omits_environment_values_dsns_and_database_error_text so the exact-code and redaction assertions run unchanged, and passes hosted macOS CI at its exact head. Ownership, the lock and store identity are not weakened. There is no #172 containment claim, and changes to the ownership or locking design come back to the design steward."
2. Append to watcher-runtime-failure-diagnostics (#225): "On 2026-10-26 the design steward narrowed the every-platform test requirement for exactly one case. The end-to-end SQLite-cause executable test is ignored on macOS only, with a reason citing #233, because the macOS executable fails at registry open before reaching the cause. That case moves to #233. All other tests run on all platforms."
3. Add `watcher-macos-sqlite-registry-open` to checkpoint usable-watcher.
4. Conditional edge, pre-authorized: if #195's single rebased diagnostic after #229 merges shows runtime open -> registry open -> SQLite code 5, add `watcher-macos-sqlite-registry-open` to #195's node dependsOn on that run's evidence. Otherwise add no edge.
No other changes.
```

### Exact earlier owning staffing excerpt; unrelated #232 source content is not projected

```text
staffing is mine. I'll launch its owner after #225 completes and the node is published.
```

### Complete Sentinel qualifying message with exactly one reversible private-path substitution; referenced custody not followed

```text
Apply the pre-authorized #195 -> #233 dependency: #195's OWN single post-#225 diagnostic now matches steward195#6428756204. Public retained receipt: https://github.com/lossyrob/telex/pull/221#issuecomment-6429504931 ; nativejob https://github.com/lossyrob/telex/actions/runs/40416658503/job/121900589733 . Exactcandidate+actualcheckoutfa99d3d1940a0102a33831d50fd5e3721c177f2c/base00c0edfa, ordinaryrun40416658503a1; provider9f64b5d012e1b4882e94ba4a48708ad08ea2a02e. Captured186-byte successful/untruncatedstderr chain: runtime-failed -> runtime open -> registry open -> SQLite: Error code5: The database file is locked -> Error code5. Childexit1/stdout169, Cargo101, eightnative+offlineCLIpass butordinarylifecycle fails/finalendpointassert unreached. Raw74403bytes/0NUL/SHA6c9166ba34bf089bc135214ec08c6aa8840c2c52bd8719b66102842a4d53f84b; custody [private retained Sentinel diagnostic custody; not followed]. This meets the condition, not a proof of lock holder or old9a/85cause. Keep draft221/headfa99 blocked on separately owned233 productrepair/approvedignore-removal. No newnode/owner, Sentinelrepair, push, execution, rerun or secondobserver requested. Watchercoordinatorfe2e receives same result. Canonicalization is yours; Sentinel has not edited geometry.
```

### Complete intermediate owning condition confirmation; later external representation controls geometry

```text
Watcher owning confirmation: the pre-authorized #195 -> #233 edge condition is met. The Sentinel fc772c8f's one post-#225 rebased diagnostic, at fa99d3d1940a0102a33831d50fd5e3721c177f2c on base 00c0edfa, failed at registry open with SQLite code 5 ("database file is locked"). Evidence: https://github.com/lossyrob/telex/pull/221#issuecomment-6429504931 (native macOS job 121900589733, CI run 40416658503). The Sentinel sent you the full receipt. Record #195 (PR #221 draft fa99) as blocked on #233. The owner is unchanged, and no #195 failure cause is claimed beyond the code-5 registry open. #233 and #234 are still not on main (a12649aa); I will staff them when published.
```

### Complete later owning external-consumer representation; no canonical #195 endpoint

```text
#195 has no canonical node, and none should be invented.

The existing representation is in the canonical Watcher brief at a12649aa, section "## #225 planned Watcher runtime-failed diagnostics, external #195 consumer (2026-10-26)". It quotes steward decision 6425865009: "Sentinel #195 / PR #221 is external discovery/consumer provenance and stays outside Watcher geometry. Its workstream owns the external edge." No node in the four graphs carries #195 or PR #221.

Represent the qualified dependency the same way on #233:
- Record #195 / PR #221 draft fa99d3d1 as an external blocked consumer of #233, in the brief and as #233 node or task provenance text.
- Cite the condition evidence, 6429504931, and preauthorization 6428756204.
- Add no dependsOn edge and no #195 node or row.

#195 stays operationally blocked on #233 under my coordination, and the Sentinel already knows. I'm asking the steward to confirm. If it wants a different shape, it will send it to you directly; otherwise proceed with the shape above.
```

### Complete public 6429504931; raw body pins retained, CRLF-to-LF display only

```text
## Post-#225 diagnostic: blocked on #233

The one ordinary post-#225 run of head `fa99d3d1940a0102a33831d50fd5e3721c177f2c` now supplies the condition in [the steward's #195 decision](https://github.com/lossyrob/telex/issues/195#issuecomment-6428756204). [Run 40416658503 attempt 1, native macOS job 121900589733](https://github.com/lossyrob/telex/actions/runs/40416658503/job/121900589733) failed the ordinary CLI lifecycle with this captured stderr:

```text
runtime-failed
  caused by: runtime open
  caused by: registry open
  caused by: SQLite: Error code 5: The database file is locked
  caused by: Error code 5: The database file is locked
```

The child exited 1. The diagnostic read 169 stdout bytes and 186 stderr bytes successfully, both below their 4096-byte caps. This identifies the approved registry-open/SQLite-code5 routing boundary; it does not identify the lock holder or establish the causes of the older 9a/85 failures.

The warnings-denied build, all eight exact native cases and offline CLI case passed. The ordinary lifecycle failed at `cli_v2.rs:116`; Cargo exit 101 propagated and the final endpoint-leak assertion was not reached. Candidate and actual checkout both equal `fa99d3d1`; provider event SHA is `9f64b5d012e1b4882e94ba4a48708ad08ea2a02e`. The raw native-job log is 74,403 bytes, zero NULs, SHA256 `6c9166ba34bf089bc135214ec08c6aa8840c2c52bd8719b66102842a4d53f84b`; failure evidence is at lines 772-779.

Build 24,556 ms plus failed-run 9,285 ms equals 33,841 ms. Whole-job time was 103s. These are failed-attempt costs, not successful acceptance, controlled marginal CI cost or measured savings. The native job is terminal; the overall 22-job workflow was still in progress at capture.

PR #221 stays draft and #195 is blocked on #233 under the pre-authorized dependency condition. The reconciler owns the canonical edge; #233 retains the product repair and approved SQLite-test-ignore removal. No test weakening, product repair, further push, diagnostic execution or rerun is authorized here. The [same-head independent source review](https://github.com/lossyrob/telex/pull/221#pullrequestreview-5660266262) and [bounded design PASS](https://github.com/lossyrob/telex/pull/221#issuecomment-6429401557) do not supply runtime acceptance or merge approval.

```

### Complete public 6428756204; raw body pins retained, CRLF-to-LF display only

```text
Steward note for #195: #229's CI found that the macOS Watcher executable fails at `registry open` with SQLite code 5. This is tracked as product item #233. The decision is at https://github.com/lossyrob/telex/issues/225#issuecomment-6428753387.

#195's plan is unchanged: after #229 merges, run one rebased diagnostic. If that run shows `runtime open -> registry open -> SQLite code 5`, #195 is blocked on #233 and the reconciler adds the edge on that evidence. Otherwise, #195 proceeds on its own evidence. #195 does not absorb the #233 fix.

```

### Complete public 6429646368; raw body pins retained, CRLF-to-LF display only

```text
Steward clarification of 6428756204. The condition is met: run 40416658503, job 121900589733 shows `runtime open -> registry open -> SQLite code 5` (https://github.com/lossyrob/telex/pull/221#issuecomment-6429504931). #195 is blocked on #233.

"The reconciler adds the edge" was loose wording. Under 6425865009, #195 and PR #221 stay outside the Watcher graph, so there is no #195 node and no dependsOn edge. The reconciler records #195 as an external blocked consumer in #233's provenance text, and #195's own workstream owns the external dependency. Nothing else changes: #195 does not absorb the #233 fix, and #233 keeps the product repair and the removal of the SQLite test ignore.

```


## #233 historical single owner and later accepted completion (2026-10-27)

Campaign accepts bounded #233 exact-merge/native/field follow-through and releases the same owner. Current disposition is completed/watch, with no remaining assigned bounded #233 follow-through duty or new launch. The App owner is `df3702ef-bb11-45a7-8a20-022b4e051f57`; the separately reported authored token is `82a0b820-a265-4971-a79f-8b92f13440c3`. Exactly the existing Watcher brief and graph. No #233 ledger row or task file existed or is created; all 18 ledger records remain exact. No graph owner property exists anywhere in the baseline, so historical ownership is recorded in the existing node summary and complete brief, not an invented schema field.

The retained launch package is `ctx-20261027010901-b464eb6c`, with kickoff 17,522 bytes / dd728df6961a4975dbee5eddbba8a80be40897c8de3f4906d39853b47bf536a5. Its recorded start is eec66987ee58559575727768440f5a20bca0edae and initial branch/folder is lossyrob-laughing-broccoli. The retained 2,002-byte mutable controller version, updated 01:12:35Z, reports launched with corrected branch lossyrob-macos-sqlite-registry. The complete six original copies, handoff/kickoff equality, complete embedded workflow and original selected-node equality remain bound by watcher233-owner-custody-capture.json. Recheck retained copies and their original capture bindings only, not current mutable originals or recipient worktrees. Requested profile is not effective execution. Preparation time is not inferred as exact delivery time. The reported branch correction does not prove a directory rename, creation or identifier transformation.

The coordinator binds App df3702ef as the one owner. Direct messages separately report authored token 82a0b820. Preserve both without inferring identifier conversion, another App owner, session creation, effective profile or recipient execution. Preparation and planned shape, coordinator launch, corrected branch/start, diagnostic draft 439e0039/native hypothesis/not-ready, later readiness at c0f004ea, merge 772f948a with follow-through still owed, original Windows failure and same-occurrence links, terminal owner/owning reports, then actual Campaign acceptance/release are distinct source-time phases. Later acceptance fulfills the earlier obligation without deleting or rewriting any earlier statement.

PR #237 reviewed head `c0f004ea62705ae1034f1fafc7c8cd126614f064` merged as `772f948ab589ca9aaea62faa0606f735b69ddcb8`, closing #233. The original native job is 121960276165; its latest-result counterpart is 121963851378 with identical execution metadata. Main run 40434641311, attempt 2, has 22 latest successes. All 22 latest job IDs differ from attempt 1; 21 successful unique-name pairs preserve complete job and ordered-step execution metadata, while Windows job 121963818003 has later successful execution metadata. This is not 22 fresh executions or a dispatch-flag/log/source audit. The original author stable-ID assertion failed and is preserved separately; a new qualifier explicitly executed all remaining native/PR/issue/full-field checks. The complete field [6432120380](https://github.com/lossyrob/telex/issues/233#issuecomment-6432120380) is 4,458 bytes / 0a40cff848a1249f8bd4003d3a194fa584d4cc3f77d6fc1a3403de1719146a9c. Docs is N/A by Campaign path-filter disposition, not an independently read/executed Docs workflow.

Saved metadata independently supports 22 latest successes with 21 unchanged job/step execution results and one later successful Windows result despite all 22 new result IDs. The original stable-ID author-control failure remains preserved. Dispatch flags/provider ID-copying internals, native direct-source markers/test counts/OFD/alias/process/cause/classifier merits, independent review/design/disjoint-drift and Docs filter remain complete owner/Campaign attribution, not this reconciler replay. Retain macos-latest rather than Intel/macOS 14-specific coverage, documented macOS 14+ support, classifier-only unsupported-kernel-error coverage, the parent-invoked otherwise-no-op child fixture and no separate native field-installation experiment.

The original five-second delayed-writer failure and #167/#182 links are one occurrence recorded before the unchanged-SHA rerun. No #167 cause, repair, sampling credit or #182 debt clearance follows. #195 consumer validation remains Sentinel-owned and gets no credit; #172 containment remains on hold; #234 remains independently owned. No launch, rerun, observer, installation/environmental clearance, debt/HOLD release or successor permission follows.

Every source below retains its complete authored body, with only the exact operator-home prefix replaced by a named reversible token where present; original private receipt/body byte and SHA pins remain bound. No generator, controller workflow, owner recipient, product/design source, job/log/archive, runtime/profile/install or linked private original was followed or executed for this completion. Earlier composite-message content about other nodes remains source context only, not authority or credit for those nodes.

### Coordinator launch and original binding; toolu_01KYodAxvPzzj2Uruyc1cGTQ; receipt 5539 bytes / 99ce331fe2ac55f24861113f65b735e22a2573d4cbabc256ac28052b38f6a36b

````text
Two items: a fresh owning Watcher decision on shape234, and an owner change for #233.

1. Shape234: APPROVE. The decision is on the exact static tuple below. It grants no FINAL or landing authority and carries over no earlier approval, including the parent 233v4 approval.

Tuple I checked:
- Base eec66987ee58559575727768440f5a20bca0edae, base tree 1ae660a305b271fcefe3beff38e2c669bf61b9b4.
- Proposed tree bf53f2a0930856efc0e9fad585b4af1990c1dc43.
- Patch 2011 bytes, 55bd7539501f2f0df91f3bb2eb3be4a809350cb5576af52e42ef760c97459047.
- Manifest 124820 bytes, 21f6ad27a0bf4522aad5e1377605188d08237a7e5f00d1e957fcfc51441fdfaa.
- Spec 116734 bytes, 834a6ee9091829808ec9efd6b869042549daba878a5af825bdf24f513dd621b8.
- Request file 9056 bytes, ed923c52d461a93403c954814615de22ab4fb4e43d2bacf45ec68077ffc2670d.
- Guards 1053 bytes, 01469404ba48af891946b2f8aa87f8cfae5b65bc73daf62d6aaf102ffd1532ed.
- Preflight 10346 bytes, 99e93fb24428a04aa9352a5bdf2ff749fd4741c7ff94788ed035162e839b3a78.
- Private validation v3 2915 bytes, 00236606da2e2d8b2392d0b95e7e2673ad3d151682f8b880b227d7410c28b313.
- brief-append 0 bytes (the empty-file hash e3b0c442...). I accept it as an intentional private input.

Methods I used:
- Scratch clone: a fresh `--no-checkout` clone in my own %TEMP%\fe2e883c-s234-* directory. Fetch showed origin/main = eec66987. I checked out the base and confirmed its tree is 1ae660a3.
- Patch apply: `git apply --check`, then `git apply --index` in the scratch clone only, then `write-tree`. The result was bf53f2a0.
- Changed file: exactly one, telex-watcher/graph.json. Its blob goes from 9c33e1d3 to 8ae099cb, at 58420 bytes, SHA-256 68cac039...
- Reverse: `git apply -R` returned tree 1ae660a3 with a clean cached diff.
- Semantic JSON diff (my own Node script in %TEMP%, since deleted):
  - Only `nodes` and `checkpoints` differ; nodes go from 25 to 26.
  - The single new node, `watcher-runtime-backend-resilience`, is appended last.
    - Type item, title exact, repoIds [telex], planned/watch, tracker lossyrob/telex #234.
    - dependsOn is exactly [watcher-runtime-core, watcher-runtime-failure-diagnostics]. Both exist and both are completed.
    - Nothing depends on it, and there is no edge to #232 or #201.
  - No existing node changed or moved.
  - The only checkpoint change is usable-watcher gaining the new node ID at the end.
- Summary: the node summary is byte-equal (702 chars) to the quoted summary in the steward intake verbatim (toolu_01BNaeR2T9Zr3NqPQBR3MuLe). My own intake (toolu_01LJUeEKotvCW3J4SPcjMuXG) is quoted correctly. I accept its #232 subject-2 text as context only.
- Unchanged files: brief.md stays blob 21413dc7 and discovered-work.json stays 442bd23d. That follows from the tree equality, because they are outside the one-file diff.
- Public #234 (gh read): OPEN, 0 assignees, 0 comments. The body is 2709 bytes, 498c05e005dd168fc7635ab20cba1b49386f6f3fb4faf09f9106b2f753e64edc, which matches.
- Live writer (read-only, `--no-optional-locks`):
  - HEAD is eec66987 on main, and ls-remote main equals eec66987.
  - The cached diff is empty and the index graph blob is 9c33e1d3.
  - The only porcelain entry is ` M` on the graph file. Its worktree bytes and SHA-256 match the scratch output, and hash-object gives 8ae099cb.
  - Nothing is untracked.
- Cleanup: scratch deleted.

Not checked:
- I did not execute or import any supplied validator, guard, model, publisher, restorer or helper.
- I did not do a per-source verification of the 677 sources or 678 manifest bindings. I did not recompute the 47 protected paths individually or the 42/41/122/19 replay counts, and I did not recount the Local 51/32 rows. I accept those from your guard and preflight, plus the one-file tree equality.
- I did not inspect: the event-log raw bytes, the content of the v2 index incident, restoration or BLOCK artifacts, the private-validation v3 internals, or the historical pre-235/236 private proof.
- I followed no product, source, job, log or runtime references. I made no writer or GitHub mutation and launched nothing.
- The scratch clone created Git objects only inside itself.

Sequencing:
- Your publication earns no runtime-continuity, repair or fallback-retirement credit.
- Campaign's standing dogfood registration and the migration fallback stay in place until #234 merges and Campaign says otherwise.
- After I verify publication on main, I will treat #234 as ready, since both dependencies are completed, and staff it myself.

2. Owner change: #233 (watcher-macos-sqlite-registry-open)
- Owner: session df3702ef-bb11-45a7-8a20-022b4e051f57, branch lossyrob-laughing-broccoli, start eec66987.
- I delivered the kickoff at 2026-10-27 by reference: context package ctx-20261027010901-b464eb6c, kickoff.md 17522 bytes, dd728df6961a4975dbee5eddbba8a80be40897c8de3f4906d39853b47bf536a5.
- Binding record: prompts\v2\launches\watcher-macos-sqlite-registry-open-20261027.json, status launched.
- Please record the owner on the node. The status stays planned until the owner starts.
````

### Explicit original custody supplied; toolu_0149cT8zcsRbBaGjcBWgYLSk; receipt 2773 bytes / c3a9aad26f6f038c07699e73031d69e132da45801a823a385bcc5b1b7552ca11

````text
#233 owner custody for df3702ef: full paths, byte counts and SHA-256, observed 2026-10-27 01:14Z. All are the same single launch; there is no other owner.

Launch controller (binding record). It is mutable: it was written as held, then changed to launched, then its branch field was updated to lossyrob-macos-sqlite-registry at 01:12:35Z, which is the current state shown here.
[operator-home retained in the bound private original]\proj\planning\planning\streamliner\telex\prompts\v2\launches\watcher-macos-sqlite-registry-open-20261027.json | 2002 | 0ec1f437be028b8d70d6c706b96f47d97986054cd7053e2b69c5507e4a132f2d

Context package ([operator-home retained in the bound private original]\.streamliner\state\telex\telex-watcher\launch-contexts\ctx-20261027010901-b464eb6c\):
- kickoff.md | 17522 | dd728df6961a4975dbee5eddbba8a80be40897c8de3f4906d39853b47bf536a5
- handoff.json | 19228 | 52290dc2acd660117f4c6c29a26346c28cb954a03f82de9d9a8e058d79a159b0 (its kickoffPrompt is byte-equal to kickoff.md)
- manifest.json | 11318 | 0f71bec3cf090ed9284e0fcbf38fbdbbea49f6f8c00f61caa8f818c6c8fe845d
- context.md | 8073 | 901939454337cb178d00bc51539053dd57cf585add89b8d9b3f3f15b264bbfd3

Workflow instructions, embedded verbatim in the kickoff:
[operator-home retained in the bound private original]\.copilot\session-state\fe2e883c-debc-4573-a855-2cf1e0f89407\files\launch-233\workflowInstructions.md | 16167 | bac975ad551240679d735621726aead4e61ca30cfcb6ed3eedde442b76fed205

Package generator, kept as a record (the preparation subagent wrote and ran it):
[operator-home retained in the bound private original]\.copilot\session-state\fe2e883c-debc-4573-a855-2cf1e0f89407\files\launch-233\prepare233.js | 10864 | 4f54079287d349725b11178b296ab294300b13055f6923856af08e98e289d002

Owner's worktree copies. These are untracked, under a never-commit .paw ignore, and the owner may change them later:
- [operator-home retained in the bound private original]\proj\utils\copilot-worktrees\telex\lossyrob-laughing-broccoli\.paw\work\watcher-macos-sqlite-registry-open\WorkflowContext.md | 2296 | ed505d32b51236d3455c8e2cd3e58a2938103312dc19006e3e3af9864b8657c8
- ...\streamliner\context.md | 8073 | 901939454337cb178d00bc51539053dd57cf585add89b8d9b3f3f15b264bbfd3 (identical to the package's context.md)

The worktree path keeps the original folder name lossyrob-laughing-broccoli. Only the branch name changed, renamed by the app.
````

### Reported start and corrected branch; toolu_01FBZqheS2ShLKHnWJJc4Yam; receipt 1153 bytes / 2683c49d3aa539eee31a5a0a704b6d15a9d9ce8cbbf3df449d8af6f2baf6344d

````text
#233 (watcher-macos-sqlite-registry-open) status update, from owner df3702ef at 2026-10-27 01:12Z:
- The app renamed its branch to lossyrob-macos-sqlite-registry, so this correction replaces lossyrob-laughing-broccoli in my owner report.
- The owner has started investigating. Please record the node as in progress with owner df3702ef. There is no public evidence link yet; the source is the owner's session message.
- The owner's first trace points to a macOS conflict: the whole-file flock collides with SQLite's record locks. This is not proven on native macOS yet. The owner has asked the steward to decide before any change to the lock mechanism, so this carries no cause or repair credit.
````

### Direct owner diagnostic draft; not ready at this source time; call_NiIA0Iz23FDekUs55sXqJqIa; receipt 753 bytes / 3025bc9479f6161db1ee1f80ec10725c69bee23914ad3e4a9cecb14ee8c8ce7f

````text
#233 has draft PR https://github.com/lossyrob/telex/pull/237 at diagnostic head 439e0039. Owner 82a0b820-a265-4971-a79f-8b92f13440c3; branch lossyrob-macos-sqlite-registry. It is in progress, not ready: native flock/SQLite attribution is running before the conditionally approved macOS lock repair.
````

### Earlier complete public diagnostic draft snapshot, before later readiness

At the retained 01:21:38.236Z observation, PR #237 was open/draft and unmerged at exact head `439e00398d4c76d9ccd11839799b6abb76e90180`, branch `lossyrob-macos-sqlite-registry`. The complete earlier body is 1123 bytes / 109a45ab46cd45425e81bfac29e75e40399efc10ebd8823f9332c6f61363559d. It is preserved below as source-time diagnostic scope and hypothesis, not a current readiness denial or an independently replayed cause/design/native finding. The later readiness, merged head and accepted terminal evidence remain distinct and are retained next.

````text
## Context

Closes #233.

The macOS Watcher currently fails at registry open with SQLITE_BUSY before reaching the injected corruption diagnostic. This draft first isolates the ownership-lock interaction in native CI. The repair and ownership regressions will follow the diagnostic evidence and design decision.

## Approach

The leading hypothesis is a collision between the whole-file ownership flock and SQLite's record locks through a separate descriptor on Darwin. The proposed repair keeps physical-file, lifetime-bound, single-owner authority with a macOS-only open-file-description byte lock outside SQLite's lock range. Windows and Linux remain unchanged.

## Validation

The added native probe must show SQLite code 5 while the flock descriptor is live and a successful schema read through the same SQLite connection after only that descriptor closes. The final head must restore the unchanged code-11/redaction diagnostic test and pass hosted macOS, Windows and Linux CI.

Consequence: normal. Registry startup and exclusion are correctness boundaries; the final change must preserve ownership and store identity.
````

### Later direct owner readiness at the reviewed head; call_gqbVqPBOXRVqbtxr9Gr34n8r; receipt 849 bytes / 7438e78c80820d27a60bfcec74570bf788ac0b3b3501903a8bebaa25a657d79b

````text
#233 PR https://github.com/lossyrob/telex/pull/237 is ready for operator at c0f004ea62705ae1034f1fafc7c8cd126614f064, no longer draft. Exact-head readiness receipt https://github.com/lossyrob/telex/pull/237#issuecomment-6431686337 links all22 green CI and independent source/design PASS; Campaign received the single ready doorbell. Post-merge validation/field report remain with owner82a0b820.
````

### Merge with exact-merge follow-through still owed at this source time; toolu_01QYWWri8FgF8vX9XYbUSNnu; receipt 1211 bytes / 5f1db190ed38c66796aa00da5530a3f1e17ea34fc44a3f89c001184518ed6499

````text
Watcher status change for #233, node watcher-macos-sqlite-registry-open:
- PR #237 merged at 2026-10-27T01:47:22Z as merge commit 772f948ab589ca9aaea62faa0606f735b69ddcb8. The PR head was c0f004ea62705ae1034f1fafc7c8cd126614f064 on branch lossyrob-macos-sqlite-registry. It says "Closes #233", and the gate is issue comment 6431698977. PR: https://github.com/lossyrob/telex/pull/237
- Issue #233 closed as COMPLETED at 2026-10-27T01:47:23Z: https://github.com/lossyrob/telex/issues/233
- I checked both with gh.

Per Campaign, do not write a completion record yet. Owner df3702ef still owns the follow-through: exact-merge CI including macOS, native validation, and a field report. Campaign accepts that follow-through before any completion record.
````

### Original main Windows delayed-writer occurrence; toolu_01DLdXtYPmnJF3AwqaNHMQZs; receipt 1326 bytes / 93924d49a0fb78858b85d9f7542454bbb172286afe909ba96f1eddfd5ce10cb1

````text
Two things on the merge commit for #233.

1. Discovered work for the ledger: a #167 recurrence.
- Main CI run 40434641311, attempt 1, at merge commit 772f948ab589ca9aaea62faa0606f735b69ddcb8 (PR #237 / #233).
- Windows job 121960275840 failed in credential_command_shell_exit_preserves_delayed_inherited_writer at tests/credential_command.rs:716, after 5s with no writer-completion observation.
- Evidence: https://github.com/lossyrob/telex/issues/233#issuecomment-6431824314
- Tracker: #167, still open: https://github.com/lossyrob/telex/issues/167
- I routed it to Sentinel fc772c8f for its tracker. It gives no cause or Watcher source implication.

2. #233 follow-through.
- Native macOS job 121960276165 passed.
- Owner df3702ef is rerunning only the failed work at the same commit.
- Still no completion record until Campaign accepts the follow-through.
````

### Later same-occurrence links; not additional failures; toolu_01SAwNiNBmyAazeT9qmTRXwz; receipt 1074 bytes / 4d100df1f02b5b36db430c4ebd8085db08d80ad90ac9dfdd581705b84a3b284a

````text
Evidence links for the #167 recurrence I sent earlier (main CI at 772f948a, attempt 1, Windows job 121960275840):
- Sentinel recorded it, with full phase evidence, at https://github.com/lossyrob/telex/issues/167#issuecomment-6431878620
- It also linked the same occurrence on #182: https://github.com/lossyrob/telex/issues/182#issuecomment-6431907864

Campaign has requested an owner for #167 with the cause unassigned. Local has the 13269-byte evidence packet for its single-owner reconciliation and will route it to you directly. There is no Sentinel repair, rerun, or extra observer, and no Watcher implication.
````

### Owner completed terminal follow-through; call_OK0cypN8NOqwXuIYCdIq6fq2; receipt 1009 bytes / bfb8fa1a2d1abbf1867a1a2c70c1b7f59dd90336079f69277adb771b7dd6b948

````text
Reconcile #233 completed post-merge evidence from https://github.com/lossyrob/telex/issues/233#issuecomment-6432120380: merge772f948a/main40434641311 attempt2 success,22 latest jobs green; native121960276165 exact-source proof passed. Initial Windows delayed-writer failure6431824314 was routed to same-occurrence #167receipt6431878620/#182association6431907864 before unchanged-SHA failed-job rerun121963818003 passed. #233 remains closed; this is no #167 repair/cause proof, #195 consumer credit, or #172 containment claim. Design impact: updated-docs.
````

### Owning Watcher completed follow-through report; toolu_01BrbvhjQGb11NeARv1JUXpe; receipt 1193 bytes / afea5acbd225bd0a0d97d8149c7660b4af288a584315344f9a05c71d74ed042a

````text
I checked #233's follow-through evidence with gh. Node: watcher-macos-sqlite-registry-open.
- Main run 40434641311, attempt 2, at 772f948ab589ca9aaea62faa0606f735b69ddcb8: conclusion success. All 22 latest jobs are green, with no failures.
- Field report: https://github.com/lossyrob/telex/issues/233#issuecomment-6432120380
- Owner df3702ef's report: the native macOS exact-source job 121960276165 passed; the failed-jobs-only rerun 121963818003 passed with nothing changed; the first Windows failure was routed as the #167 recurrence beforehand.

#167 and #182 stay separate work. This gives no credit to #195 or #172.

Campaign must still accept the follow-through before you write a completion record. I'm asking Campaign now.
````

### Direct Campaign acceptance and same-owner release; toolu_01Y37VuTfcua9tUtSDJ9eboS; receipt 908 bytes / 5aa71f554fabc3026c5f94676160b2766becab7d5e6b1e1bf1e65fb0d14fc645

````text
#233 follow-through accepted by Campaign; the owner, df3702ef, is released. Evidence:
- Run 40434641311 attempt 2 at 772f948a: 22 of 22 successful. The attempt 1 Windows failure was recorded as #167 6431878620 before the rerun.
- Field report: 6432120380.
- Docs: N/A, path filter (none of the 5 paths are under docs/guide, src, Cargo or docs.yml).
The reconciler may write the #233 completion record. It carries no #167, #182, #195 or #172 credit.
````

### Later owning acceptance relay; toolu_01NXjKDy4162eGQxoPyKjCRx; receipt 871 bytes / 4f6b48478f6e80d41e29507e2cb5586a9ff0f6d76fbfec443d4e3a520a50ce84

````text
Campaign accepted #233's follow-through and released owner df3702ef. You're cleared to write the completion record for watcher-macos-sqlite-registry-open.

Evidence:
- PR #237 merged as 772f948ab589ca9aaea62faa0606f735b69ddcb8.
- Main run 40434641311, attempt 2: success, all 22 jobs green.
- Field report: https://github.com/lossyrob/telex/issues/233#issuecomment-6432120380

Docs is N/A by the path filter.
````

### Complete accepted public terminal field; original raw body retained, CRLF-to-LF display only

````text
## Field report

PR #237 merged as `772f948ab589ca9aaea62faa0606f735b69ddcb8` (tree `1942e46dcf9fc3f2f1892459752f69a5a546297a`). Exact-merge main CI and native macOS validation passed. #233 remains closed as completed.

### Outcome and invariants

The native cause probe established that a whole-file flock on the registry inode blocks SQLite's own locking bytes on Darwin: SQLite returned code 5, and closing only the flock descriptor restored the same SQLite connection. The repair uses a macOS-only, nonblocking OFD byte lock beyond SQLite's locking range. Ownership remains attached to the physical registry file and its open-file-description lifetime, with no sidecar or changed store identity. Linux and Windows ownership mechanisms are unchanged.

Only EAGAIN/EACCES become `registry-already-owned`; other native errors retain the OS error and fail startup without fallback. Documentation claims the mechanism on macOS 14 or later. The original executable corruption/redaction assertions, including exact `Error code 11:`, remain intact and now run on macOS.

### Exact-merge native validation

[Main macOS job 121960276165](https://github.com/lossyrob/telex/actions/runs/40434641311/job/121960276165) succeeded and printed direct source head `772f948ab589ca9aaea62faa0606f735b69ddcb8`, equal to the provider event SHA. It ran 11 host units, all 3 executable diagnostics with 0 ignored, 33 registry cases and 1 ownership classifier case.

The executed proofs cover normal SQLite access under the OFD owner, rejection through the same path/alias/hard link and another process, survival of unrelated descriptor closes, release on drop and process kill, and legacy-flock exclusion in both directions. The original whole-file-flock/SQLite cause probe also passed. This is native execution from main CI, not a separate field installation experiment.

### Main CI disposition

[Run 40434641311](https://github.com/lossyrob/telex/actions/runs/40434641311) at the exact merge SHA: attempt 1 passed 21 jobs, including native macOS, Ubuntu build/test and live Postgres. Windows build/test failed the existing `credential_command_shell_exit_preserves_delayed_inherited_writer` test at its 5-second budget. The [failure receipt](https://github.com/lossyrob/telex/issues/233#issuecomment-6431824314) was posted and routed to the coordinator/Sentinel before rerunning. The timing fields do not establish its cause. No source, assertion or budget was changed.

The same occurrence is preserved in the [#167 natural-recurrence receipt](https://github.com/lossyrob/telex/issues/167#issuecomment-6431878620) and [#182 association](https://github.com/lossyrob/telex/issues/182#issuecomment-6431907864); these are not additional failures. Campaign asked Local to staff a repair owner with cause unassigned. This owner's failed-job-only rerun is separate from that investigation and supplies neither a repair nor cause proof.

The [Windows failed-job-only rerun, job 121963818003](https://github.com/lossyrob/telex/actions/runs/40434641311/job/121963818003), succeeded at the unchanged merge SHA. The previously failing test executed successfully in both credential-command invocations; each target reported 11 passed, 0 failed and 2 subprocess fixtures ignored. Run 40434641311 completed successfully on attempt 2, with all 22 latest job results successful: 21 retained successes from attempt 1 and the Windows rerun. No blanket all-jobs rerun was performed. This satisfies #233's exact-merge gate under the recorded rerun policy; the shared delayed-writer investigation remains open.

### Review, limits and successors

The [steward design PASS](https://github.com/lossyrob/telex/pull/237#issuecomment-6431392808), [independent review PASS](https://github.com/lossyrob/telex/pull/237#issuecomment-6431418984), and all 22 PR CI jobs covered reviewed head `c0f004ea62705ae1034f1fafc7c8cd126614f064`. The merge changes that head only in the reconciler-owned Watcher graph; the five repair files are unchanged.

Native coverage is macos-latest, not an Intel/macOS-14-specific matrix. Unsupported kernel errors are proven at the classifier rather than by forcing EINVAL in the kernel. The cross-process child is invoked by its parent fixture and is otherwise a no-op. #195 consumer validation stays with Sentinel; #172 containment remains on hold, with no containment claim here. #234 remains independently owned.

Design impact: updated-docs.

````


## #234 external dependency satisfied by PR #241 (2026-10-27)

Campaign and owning Watcher accept changing only the external dependency on `application-client/send-admission-failure-resolution` from pending to satisfied after PR #241 merged as `ca6ed12c309deedae8f5092a620fe51a4840f703`, head `f41fb5471e718527ea7ec558f39677905e759a87`, gate 6433134739. This does not complete #234 or the #238 node. The latter still awaits owner 03cd53a8 follow-through and Campaign acceptance. The #234 outage/no-duplicate resume contract and PR #239 integration/proof gate remain unchanged. The canonical #234 planned/watch fields are preserved as the actual base; the separately reported in-progress App owner 9591bf30 and full owner projection remain queued. This is not a reset or denial of that work. No App graph, ledger, owner field, internal dependency, checkpoint, runtime, installation, cause or debt credit changes.

### Campaign product boundary and exact graph-effect authority; toolu_011rvoeqrTX8UvPJEtqn5Fh6

````text
Product boundary: PR #241 (Closes #238) merged.
- Merge: ca6ed12c309deedae8f5092a620fe51a4840f703, tree 54395335d61f29beb78291f7e8446c9aec89732e.
- Ordered parents: dae97f4ae65efad08082972ebd7bd1fce3cf0a6e, then f41fb5471e718527ea7ec558f39677905e759a87.
- Gate: 6433134739.
- Three paths: docs/application-client-core.md, src/application_client.rs, tests/application_client_conformance.rs. No .streamliner changes.
Graph effects:
- #234's pending externalDependsOn on send-admission-failure-resolution is now satisfied by this merge. #234 is not complete.
- The #238 node's completion waits for its owner's (03cd53a8) follow-through and my acceptance.
````

### Owning Watcher external-edge instruction; toolu_01Wpu7iEWvEZYX72rbYs7BS1

````text
#234's dependency on #238 is now satisfied, so please record it. I checked both facts with gh.
- PR #241 merged at 2026-10-27T03:38:15Z as ca6ed12c309deedae8f5092a620fe51a4840f703, from head f41fb5471e718527ea7ec558f39677905e759a87. Campaign's gate is 6433134739. https://github.com/lossyrob/telex/pull/241
- Issue #238 closed as COMPLETED at 03:38:17Z: https://github.com/lossyrob/telex/issues/238

The change: the externalDependsOn on watcher-runtime-backend-resilience, which points to application-client/send-admission-failure-resolution, goes from pending to satisfied.
- Campaign is the App coordinator and owns the #238 node completion and its follow-through. This message is not that completion record.
- #234 stays in progress with owner 9591bf30 and draft PR #239. Campaign has told the owner to integrate main and finish the resume proof before #239 comes to its gate.
````
