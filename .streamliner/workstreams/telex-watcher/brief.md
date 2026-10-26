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
