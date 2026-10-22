# Telex Campaign Roadmap

> Current campaign-level plan for Telex. The campaign concept is defined by
> Streamliner's `CAMPAIGNS.md`; this document is the project-local instance and is
> revised as workstreams pass gates or seams change.

## Post-release execution direction (2026-10-08)

On 2026-10-08 the operator approved the campaign's post-release recommendation,
recorded in campaign directive
`campaign-postrelease-execution-directive-20261008.json` (SHA-256
`a4d98af7d891efe7164839c79b4431d07acfc1ceef606c54e46b82ab7eb5613d`). Two lanes
run with their existing writers on their original branches. No new worker,
replacement, PAW reinitialization, or node split is introduced.

- **Main effort: Application Client `client-conformance`
  ([#152](https://github.com/lossyrob/telex/issues/152)).** Finish the accepted
  trusted `InstalledCurrent` bootstrap and all ten public Rust conformance
  families across isolated SQLite and credentialed PostgreSQL, with public-only
  Watcher send-only and Station bidirectional fixtures, shared-semantic repairs,
  and migration guidance, in one delivery PR. The direction adds no
  executable-digest requirement or unsafe fallback.
- **Parallel: Local Daemon `station-intent-reconciliation`
  ([#106](https://github.com/lossyrob/telex/issues/106) / PR #138).** Integrate
  current main into the adopted PR and prove the accepted outcome: persistent
  owner-private OS advisory locking with no age takeover or lock-path deletion
  or replacement (M3), and honest degraded enumeration, truncation, and
  lower-bound counts with conditional eventual coverage (M5). Merged Copilot App
  lifecycle, Windows token-buffer alignment, PostgreSQL recovery, credential
  lifecycle, and schema-3 behavior are conserved. The 2026-09-02 technical
  floor is not current merge authority; merge needs fresh exact-head review,
  CI, design inspection, and campaign authorization.

Both writers resumed on 2026-10-08. The issue #152 writer is on
`feature/client-conformance` at preserved clean head
`5fdd95c2c32cde08a6c936171b208bbf14f6677c`, with no PR then. The PR #138 writer
is on `feature/station-intent-reconciliation-106` at clean head
`6315c24a5b36989f3f9dac916458f8ea9e752e60`; the PR is open and non-draft but
conflicts with main. Each writer is merging current main
`2c084873719080b812e0d0c4bf92125afa751499` into its branch by ordinary merge.
No new source commit, proof, review, or merge is recorded here.

Later on 2026-10-08, after exact-head review, required CI, design inspection,
and campaign authorization, PR #138 merged as `6ab6143a` and issue #106 was
closed as completed. The merge is source only: it publishes nothing and does
not accept the hardening gate.

The operator later authorized a source-only 0.3.0 transition inside the same
issue #152 PR. Published v0.2.1 exposes an exhaustive `ApplicationClientError`,
so the accepted typed bootstrap failure is a breaking Rust source change. The
root package moves to unpublished 0.3.0 with coupled metadata and migration
guidance. This grants no tag or publication, changes no protocol or schema
version, and adds no node.

Writers may inspect, integrate, implement, test, commit, and push within
accepted scope. Workstream orchestrators coordinate existing reviewers,
stewards, external waits, and merge handoffs. Only material new contract,
trust, migration, ownership, accepted-gap, or destructive-action choices are
escalated. Artifact reconciliation does not gate product work, and product
writers do not edit `.streamliner/**`.

Conditional follow-through, each after its own prerequisites:

1. Completed on 2026-10-22: `client-conformance` merged through PR #161 as
   `4d25fdff`, and Watcher and Operator Station then passed
   `consumer-integration-gate` on that merged main.
2. After both complete, the accepted Watcher runtime and CLI and the direct
   Windows Station implementation are promoted and prepared, without a
   premature product launch.
3. After PR #138 merges, `station-intent-transactional-authority`
   ([#153](https://github.com/lossyrob/telex/issues/153)) is promoted from the
   actual landed authority with a complete task specification. It remains
   mandatory for Local Daemon closure and does not retroactively block PR #138
   or the hardening gate.

Builder-owned usability, hardening, and closure gates remain separate
decisions. The optional example pack
([#144](https://github.com/lossyrob/telex/issues/144)) is not launched by this
direction. All daemon, upgrade, credential, and database proof stays isolated;
there is no operator installation, shared store, fleet, or persistent PATH
action. The direction grants no release, tag, or publication authority, does
not change v0.2.0 or v0.2.1, and does not close the campaign. New sessions and
delegates explicitly request `gpt-6-astra`, `reasoning_effort=high`, and
`context_tier=long_context` with no silent downgrade. The operator requested
1.2M context; the last verified catalog showed 1,050,000, so actual support is
reverified before any new preparation.

## Completed execution override: schema-3 recovery release (2026-09-24)

This override is complete and kept as dated history. Issues #154 and #155
merged through PRs #158 and #156. The v0.2.0 tag at `1b9fc8f0` failed its
Windows proof and remains unpublished; its original Errno 13 cause is unknown.
After a new operator decision, corrective v0.2.1 from
`212b76a4c586101bdc2a53264e2a4c3e2326671a` was published on 2026-09-30 as the
Latest release. Tracker 157 was set to closed as completed by an explicit
provider action, and `schema3-release-gate` is complete. The
recovery-release-specific exclusions and serial launch mechanics below ended or
were superseded with the release; durable isolation, model, review, merge,
product-launch, and campaign-closure boundaries remain as restated in the
2026-10-08 direction above.

The operator authorized a bounded v0.2.0 recovery release so released clients can
use schema-3 stores. This directive takes precedence over the historical status
and next-action text below for this effort; it does not replace the broader
campaign intent, stages, or workstream closure criteria.

The delivery path is owned by Local Daemon:

1. `windows-token-buffer-alignment`
   ([#154](https://github.com/lossyrob/telex/issues/154)) repairs both current-main
   SQLite and daemon peer-authentication token buffers and audits the remaining
   call sites. The unmerged PR #138 fix is not baseline authority.
2. `postgres-wait-reset-recovery`
   ([#155](https://github.com/lossyrob/telex/issues/155)) adopts and completes
   existing [PR #156](https://github.com/lossyrob/telex/pull/156).
3. `schema3-release-preparation`
   ([#157](https://github.com/lossyrob/telex/issues/157)) begins only after both
   repair merges and campaign verification. One isolated release worker owns
   consistent version metadata, release notes, genuine v0.1.2 upgrade and fresh
   installation proof, and exact-candidate CI and build-only release validation.
4. `schema3-release-gate` is campaign/operator-owned. Tagging and publication
   require separate explicit operator approval for the immutable candidate;
   the same release worker then owns publication and installation verification.

PR #138 remains merge-unapproved and excluded. Issue #152 conformance, #153
transactional authority, Watcher/Station runtime expansion, and campaign closure
are also excluded; their preserved workers must not be revived for this effort.
The release gate does not accept Local Daemon hardening or campaign closure,
and historical next actions below grant no launch authority for this release.

All new sessions and delegated agents must explicitly request `gpt-6-astra`,
`reasoning_effort=high`, and `context_tier=long_context` (the operator requested
1.2M context), with no silent downgrade. Product launch requires reviewed Tier B
authority on main, exact-path Streamliner preparation, verified `session-online`,
standalone `write-authorized`, and acknowledgement. Every product merge requires
exact-head campaign authorization after review, CI, and design inspection.

Schema guards remain intact. All upgrade, database, and daemon proof uses
disposable isolated roots, stores, and binaries, never the operator's installed
daemon or shared database. Genuine old-binary compatibility must be demonstrated,
not inferred from the manifest range.

## Current main effort

**Campaign — [Addressable Attention #102](https://github.com/lossyrob/telex/issues/102).**
Make Telex useful as a complete attention path: deterministic external
conditions and agent-generated obligations can reach the responsible agent or
human without session-bound polling, background waiters, or manual terminal
inspection.

## Campaign — Addressable Attention ([#102](https://github.com/lossyrob/telex/issues/102)) *(main effort)*

**Declared intent.** A Telex user can delegate long-duration observation and
human-attention routing to durable external applications. Agent sessions remain
free to reason and respond while Telex Watcher observes conditions outside the
session, Telex transports and wakes, and Operator Station gives the human a
direct actionable inbox and reply surface. Users may layer their own mediation
agents over ordinary Telex messages, but that convention is not shipped or
required by the campaign products.

**Review question.** Can external events and agent obligations reliably reach the
right agent or human, and receive a response, without manual tab polling or a
long-lived task occupying the session?

**Theater.** The Telex application layer: non-agent stations, deterministic event
producers, human recipients, and the shared programmatic client they consume.

**State.** Both builder viability gates and both initial production
domain-contract nodes completed. Application Client contract convergence is
merged, the design-only `application-client-ready` checkpoint is published, and
Application Client core implementation is merged through PR #132. The first
supported Rust binding completed through issue
[#149](https://github.com/lossyrob/telex/issues/149) and PR #151 at exact reviewed
head `c03db454781164f47a20e997665fe1251e07bd15`, merged as
`ddedfab57cc305a1e91a81d7e49e712bb36d32fd`. Issue #12 publication revision 4
records that binding publication. Application Client `client-conformance`
completed through [#152](https://github.com/lossyrob/telex/issues/152) and
PR #161 at exact reviewed head `52abb30e49dff54f9a9ce7b31c64584a871d11c3`,
merged as `4d25fdff375dcfd55b4a7b4613a5b5ecb8241bdb` on 2026-10-22; issue #152
remains open. One bundle covers the ten accepted semantic families through the
public Rust surface across SQLite and credentialed Postgres, with public-only
send-only and bidirectional consumer fixtures. Watcher and Operator Station then
passed `consumer-integration-gate` on that merged main, confirming that their
consumers need no private seam, and the `supported-client` checkpoint is
complete. Later runtime/usability gates remain planned. Operator
Station's direct-Station contract reset
completed through issue #134 and PR #136, merged as
`e071e3170c19ab1b8a753b502c67be2ee80688ec`. The builder accepted the
direct human-attended product boundary, ADR 0051 supersession, external-only
mediation, shared-client dependency, and downstream geometry at
`direct-station-direction-gate`. This closes the design checkpoint without
launching `station-app`, which remains planned and unlaunched; its Application
Client `client-conformance` and `consumer-integration-gate` dependencies are
satisfied, and a separately prepared and authorized launch is still required.
Telex
Watcher's minimal v2
authoring/registration reset completed through issue #133 and PR #135, merged as
`b91e8301899351c0411d6e2e9ac5290af8a3cb4c`; its builder-owned
`dumb-watcher-contract-gate` and `minimal-contract-accepted` checkpoint are
complete. Issue [#144](https://github.com/lossyrob/telex/issues/144) and its
bounded task specification now provide the optional example pack's launch
prerequisite. On 2026-10-22 the operator approved its launch as the first live
merge-gate pilot; Campaign cleared PR #163 at `fdb4f52f`, and the owner merged
it as `5ad7eb04`, completing the node. Runtime core is in
progress: Watcher's post-merge gate receipt discharged its Application Client
holds on `client-conformance` and the pre-integration
`consumer-integration-gate`, and issue #162 is its reviewed and reconciled
tracker. On 2026-10-22 the operator approved launch through the Streamliner
workflow v2 adapter, and Campaign launched its owner session with a shadow merge
gate.
Operator Station issue #146 separately preserves
[PR #143 Postgres and UI/UX lessons](../workstreams/operator-station/docs/postgres-dogfood-evidence.md)
as completed, non-gating evidence. The report does not promote spike mechanisms,
change direct-Station authority, or advance any production node or dependency.
Local Daemon release-confidence validation completed, but issue #106 exposed a
daemon-replacement push-intent gap. Existing PR #138 was adopted as the repair
ahead of the still-unaccepted hardening gate. Its writer resumed on 2026-10-08
in parallel with issue #152, and the PR merged that day after exact-head
review, CI, design inspection, and campaign authorization, so its
station-intent contract is now merged authority.
The schema-3 recovery release is complete:
corrective v0.2.1 was published on 2026-09-30.

## Covering workstreams

| Workstream | Tracker | Outcome | Current first move |
|---|---|---|---|
| Operator Station | [#92](https://github.com/lossyrob/telex/issues/92) | Direct human-attended Telex desktop endpoint for inbox, notification, reply, disposition, health, and recovery. | The builder accepted the direct contract and downstream geometry at `direct-station-direction-gate`, closing the design checkpoint. `station-app` remains planned and unlaunched; its `client-conformance` and `consumer-integration-gate` dependencies are satisfied, and launch still requires separate preparation and authorization. By operator sequencing decision on 2026-10-22, which is a priority choice and not a technical dependency, `station-app` also does not launch until Telex Watcher passes `five-minute-custom-watch-gate` and reaches its `usable-watcher` checkpoint. |
| Telex Watcher | [#100](https://github.com/lossyrob/telex/issues/100) | Headless, provider-neutral execution of trusted agent-authored observations with fixed Telex delivery and no session-owned background tasks. | Issue #144's optional example pack completed on 2026-10-22 in the first live merge-gate pilot: Campaign cleared PR #163 at `fdb4f52f`, and the owner merged it as `5ad7eb04`. Runtime core is in progress under issue #162: Watcher's post-merge gate receipt discharged its Application Client `client-conformance` and `consumer-integration-gate` holds, and the operator approved its workflow v2 launch on 2026-10-22 with a shadow merge gate. |
| Telex Application Client | [#117](https://github.com/lossyrob/telex/issues/117) | One supported semantic client contract and implementation for long-lived applications, without product-private forks. | Issue #152 `client-conformance` completed through its single delivery PR #161, merged at reviewed head `52abb30e` as `4d25fdff`; App, as owner, closed the issue as completed at 2026-10-22T17:59:33Z. The predecessor head `7505568c` had Local and retained source review approval after repairs, including source changes intended to address failed CI at `8bd3c330`, `2b8ade1d`, and `ec8b88f7`. It passed required CI, and on 2026-10-22 the Watcher and Operator Station pre-merge consumer attestations approved it. The Application Client design inspection also approved it, but with an inspector profile that differed from the task's; the campaign later declined to credit it. The Local Daemon design inspection required changes for four static design findings (D161-1, D161-2a, D161-2b, D161-3), so that head received no design credit. Successor `f02f0ee9` is intended to address those findings; Local's source review and the retained gpt-6-astra delta review approved it with no findings, its required CI failed in the required Live Postgres parity job, and it has no CI, consumer, or design credit. Residual successor `689883e2` was intended to address those CI failures, but Local's source review required changes for finding L161-6. Successor `0490f5e5` was intended to address L161-6; the retained gpt-6-astra delta review approved it, but Local's source review found a narrow remaining part of L161-6. Head `745eaf28` was intended to close that remaining part; Local's source review and the retained gpt-6-astra delta review approved it with no findings, but its required CI failed in the Live Postgres parity and Windows InstalledCurrent Postgres process-proof jobs. Head `0486beeb` was intended to address both failures; Local's source review and the retained gpt-6-astra delta review approved it with no findings, and its required CI passed. Watcher's fresh pre-merge consumer attestation approved it, but Operator Station's returned changes required because its PostgreSQL consumer fixture dropped the schema before stopping its daemon. Head `4cd34938` was intended to address that finding; Local's source review and the retained gpt-6-astra delta review approved it with no findings, and its required CI passed at that exact head. Fresh Watcher and Operator Station pre-merge consumer attestations both approved it, and Operator Station closed OS-PR161-0486BEE-1. The Local Daemon design inspection approved `4cd34938`, resolving D161-1, D161-2a, D161-2b, D161-3, and OS-PR161-0486BEE-1; the campaign resolved the inspector profile by enforcing the operator's standing instruction that new sessions use gpt-6-astra with high reasoning and long context, and a fresh Application Client design inspection with that profile approved `4cd34938` with no blocking design finding. That evidence did not carry over to head `5d21a63d`, which added an ephemeral Windows CI proof of the official v0.2.1 upgrade for E161-1; the implementer reported fresh source and design approvals, but its required CI failed in the new released-v0.2.1 proof job and the Windows build-and-test job, Local's source and design reviews returned changes required for proof-script findings L161-7, L161-8, P161-1, and P161-2, and Watcher and Operator Station approved its source consumability but withheld runtime credit, so it received no gate credit. Head `631aff22` replaced that PowerShell proof with an ignored Windows test in the existing Rust `InstalledCurrent` harness, without production source changes; Local's design inspection approved it, but Local's source review returned changes required for L161-9, a second panic in teardown after an early proof failure. Head `dacf3d18` fixed L161-9; Local's source review and design inspection approved it, and the implementer reported a fresh Application Client design approval, but its required CI failed in a Windows test job, where a cold-start fallback child exited with 0xC0000374, although the official v0.2.1 proof job passed, and Watcher and Operator Station approved its source consumability but withheld runtime credit, so it received no gate credit. Final head `52abb30e` makes the first production source change since `4cd34938`: in `src/station_intent.rs`, slot acquisition and worker completion for a cancellable filesystem phase now share one deadline-plus-grace window, which the implementer reports fixes a recurring cooperative-grace race. It also fails any Windows child exit status above 255 as abnormal and adds a Windows heap diagnostics CI job with full PageHeap and crash dumps. The implementer reports no defensible source cause for the earlier 0xC0000374 exit. Local's source review and design inspection, the implementer's fresh source and design reviews, and Watcher's and Operator Station's source attestations approved it, its required CI passed, both consumers qualified it at runtime, and the PR merged as `4d25fdff` under App's normal guarded merge authorization; Watcher and Operator Station then passed the post-merge consumer integration gate, completing the `supported-client` checkpoint. The implementer's field report states that the permanent hosted Windows proof using the official v0.2.1 asset resolves E161-1. Proof-isolation incident I161-1 remains open: the operator directed that current host permissions and evidence be preserved while the existing owners continue a bounded read-only impact review, and impact assessment and isolation acceptance remain open. |
| Local Daemon | [#32](https://github.com/lossyrob/telex/issues/32) | Reliable local presence and transport across SQLite/Postgres, Copilot push delivery, daemon replacement, upgrade, and restart. | The schema-3 recovery release is complete (v0.2.1). Issue #106 / PR #138 `station-intent-reconciliation` merged on 2026-10-08 and is complete. Isolated both-backend evidence goes to the separate hardening gate, and issue #153 must complete before Local Daemon closure; its prepared worker is held until Streamliner preparation passes the long-context tier to its initializer. |

## Shared seam

**Telex Application Client — [#12](https://github.com/lossyrob/telex/issues/12).**
Both production applications are long-lived non-agent stations. They need one
supported semantic client surface for process identity, attach/detach/recovery,
send, receive, reply, disposition, backend selection, and provenance.

The product spikes must not wait for this seam: they may use current CLI or Rust
library integration and must report every shortcut. After viability evidence is
available, #12 is revised and promoted as the single owner of the shared contract.
The seam is now formed as the third enabling
[Application Client workstream #117](https://github.com/lossyrob/telex/issues/117).
Issue #12 remains the sole semantic owner. Node #118 first publishes the
API-neutral `application-client-ready` checkpoint; later workstream nodes
implement and validate the supported core and binding.

Neither Operator Station nor Telex Watcher may independently freeze a competing
public client API.

## Staging

### Stage 1 — Parallel operational-loop viability

The parallel Wave 1 implementation stage produced:

- Operator Station `operator-loop-spike`: merged and reconciled historical proof
  of a human-attended Station, notifications, durable reply, honest wait/ack
  attendance, provenance, restart continuity, and unresolved-history recovery.
  Its worker → operator agent → human topology remains evidence, not current
  product authority.
- Telex Watcher `generic-watcher-spike`: merged and reconciled proof of external
  detector → Watcher → Telex → target agent with no originating session waiter.
  Evidence includes generic/custom GitHub, an authorized live Azure DevOps PR
  transition, occupied Copilot wakeup, durable unoccupied queueing, receipt-gated
  state, and isolated daemon-restart testing.

The spikes answer different questions and should not block each other:

- Is a human-attended Telex inbox, notification, and reply surface valuable and
  natural?
- Is generic external detector hosting reliable and broadly adaptable?

### Stage 2 — Independent viability gates

Each workstream has passed its independent builder gate:

- Watcher passed after scoped PR-lifecycle dogfood (~26-second merge detection,
  one snapshot plus one merge event, no duplicate/noisy events,
  canonical-checker agreement, clean watch removal, and reusable shared
  runtime).
- Operator Station passed after guided dogfood proved the human inbox,
  provenance, Windows notification, durable reply, restart continuity, and
  disposition experience. Later direction review retained those product
  findings while externalizing mediation policy.

Both gates produce evidence for #12:

- lifecycle and recovery needs;
- push/callback/poll requirements;
- service/application identity;
- cursor and restart behavior;
- provenance and metadata;
- supported IPC/binding ergonomics.

### Stage 3 — Contract convergence and shared application-client checkpoint

Watcher contract node #110 and initial Operator contract node #114 completed in
parallel, each exporting merged-source requirements without freezing a
competing shared API. Application Client node #118 consolidated both accepted
contracts and spike/gate evidence into #12, recorded explicit dispositions, and
accepted one semantic contract. The resulting `application-client-ready`
checkpoint is complete.

Application Client convergence, client-core implementation, and the Rust-first
binding are complete. Issue #149 and PR #151 landed the binding at
`telex::application_client`; issue #12 publication revision 4 records the
transition. Issue #152 and its reviewed bundle-first task governed
`client-conformance`, which completed through PR #161, merged as `4d25fdff` on
2026-10-22, with all ten conformance families, public-only Watcher and Station
fixtures, and temporary-seam replacement guidance in one PR. Watcher and
Operator Station then passed `consumer-integration-gate` on that merged main, so
product implementation may be prepared under separate authorization. Operator
issue #134 and PR #136 completed the direct product contract reset without
changing the generic Application Client ownership boundary.

### Stage 4 — Production applications under accepted contracts

After the shared semantic checkpoint:

- Operator Station completed its design reset around direct human attendance.
  The builder accepted the direction gate and closed the design checkpoint.
  The desktop app remains planned and unlaunched; its Application Client
  `client-conformance` and `consumer-integration-gate` dependencies are
  satisfied, and launch still requires separate preparation and authorization.
  By operator sequencing decision on 2026-10-22, which is a priority choice and
  not a technical dependency, `station-app` also does not launch until Telex
  Watcher passes `five-minute-custom-watch-gate` and reaches its
  `usable-watcher` checkpoint.
- Telex Watcher's minimal command-plus-policy contract and builder usability
  gate are accepted. Issue #144's optional examples completed on 2026-10-22
  through PR #163, merged as `5ad7eb04` in the first live merge-gate pilot.
  Application Client `client-conformance` completed and
  `consumer-integration-gate` accepted the merged public revision without a
  private seam. Runtime core is in progress under reviewed and reconciled
  tracker issue #162, launched on 2026-10-22 with a shadow merge gate; the CLI
  remains planned.

Each retains its own usability and operational-hardening gates.

### Stage 5 — Campaign integration exercise

Before campaign close, exercise the full seam:

```text
external condition
      → Telex Watcher
      → responsible agent or directly attended Station address
      → Operator Station when the destination is human-attended
      → human reply
      → responsible agent
```

Campaign closure checks both completed workstreams and the meaning at their seam:
source provenance remains intact, routing is predictable, notifications do not
collapse into noise, and no session-bound polling task is required.

## Coverage map

| Declared-intent slice | Covered by |
|---|---|
| External long-duration observation outside sessions | Telex Watcher |
| Agent-authored custom detector policy | Telex Watcher minimal detector contract and optional examples |
| Durable event delivery and agent wakeup | Existing Telex local exchange and bridges |
| Optional filtering/aggregation conventions | External user-developed agents over ordinary Telex |
| Direct human inbox, notifications, replies, and disposition | Operator Station |
| Supported long-lived application integration | Shared issue #12 / completed Application Client core and Rust binding; conformance pending |
| End-to-end external-event-to-human-to-agent loop | Campaign integration exercise |

## Seams and ownership

| Seam | Owner | Consumers |
|---|---|---|
| `application-client-ready` | #12 or its promoted enabling workstream | Operator Station, Telex Watcher |
| Normalized watch event envelope | Telex Watcher | Agents, Operator Station, optional external mediation |
| Human-attended message/reply provenance | Operator Station | Human operator, originating agents |
| Durable address/message/disposition semantics | Telex core | All campaign workstreams |

## Boundary rules

- Telex core carries messages and liveness; it does not poll providers or run
  detector policy.
- Telex Watcher executes trusted observations and sends Telex; it does not run
  arbitrary trigger actions or own human UX.
- Operator Station presents and replies; it does not host detector scripts or
  become the availability boundary for watches or ship semantic agent policy.
- Optional user-authored mediation may reason and filter outside the product;
  Telex core, Watcher, and Station do not require or interpret that convention.
- Shared application-client semantics have one owner through #12.
- Destructive daemon, upgrade, handoff, and branch-binary tests use an isolated
  `TELEX_HOME`, `TELEX_DB`, `TELEX_INSTALL_ROOT`, absolute worktree binary, and
  disposable proof stations. The default local daemon and installed launcher are
  campaign coordination infrastructure and are never test targets.

## Side issue

- [#12](https://github.com/lossyrob/telex/issues/12) — revise the existing
  embeddable SDK design around the post-daemon reality and broaden it to desktop,
  headless service, and agent SDK application stations after the viability
  reports exist. Both viability decisions and contract-node promotions are now
  published; #12 remains the sole owner of shared client convergence.

## Current next actions

1. Issue #152 `client-conformance` is complete. PR #161 merged at reviewed head
   `52abb30e` as `4d25fdff` on 2026-10-22 after source review, design
   inspection, required CI, and both consumers' runtime qualification of that
   exact head, and Watcher and Operator Station then passed the post-merge
   consumer integration gate, completing the `supported-client` checkpoint. The
   Application Client brief and task keep the full head history. Issue #152
   remains open; this record does not close it. The implementer's field report
   states that the permanent hosted Windows proof using the official v0.2.1
   asset resolves E161-1. Proof-isolation incident I161-1 remains open: the
   operator's preservation directive and the local runtime and proof-replay
   freeze are unchanged, and impact assessment and isolation acceptance remain
   open. No release, tag, fleet, or launch authority follows. Application Client
   `operational-hardening` remains planned.
2. Local Daemon PR #138 for issue #106 merged on 2026-10-08 as `6ab6143a`.
   Keep the hardening gate a separate builder decision that needs isolated
   restart, drain, upgrade, and push-recovery evidence.
3. Issue #153 `station-intent-transactional-authority` was prepared from the
   merged authority at `6ab6143a`, but its worker is held before any product
   write until Streamliner preparation passes the required long-context tier
   to its initializer. The operator authorized the bounded repair, one API
   restart, and one replacement initializer. The repair session started on
   2026-10-12; no repair has merged, and no restart or initializer has run.
4. Issue #144's `minimal-example-pack` completed through PR #163, merged as
   `5ad7eb04` after Campaign cleared exact head `fdb4f52f` in the first live
   merge-gate pilot. `examples-accelerate-gate` and `watcher-hardening-recipes`
   remain planned.
5. Drive `watcher-runtime-core` in its owner session under reviewed and
   reconciled tracker issue #162. Campaign runs its merge gate in shadow mode,
   and the operator merges. Its Application Client holds are discharged:
   `client-conformance` merged, and Watcher's post-merge gate receipt accepted
   the merged public revision without a private seam. Recovery remains
   reconciliation-first and query-only under uncertainty. Prepare the Watcher
   CLI after runtime core, without a premature launch; the direct Station
   app follows under the operator sequencing decision recorded for Operator
   Station.
6. Keep `five-minute-custom-watch-gate` planned for later operational proof, and
   preserve the campaign integration exercise and no-private-client boundary.
