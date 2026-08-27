# Addressable Attention Devbox Handoff

> Durable campaign-transfer packet for the Addressable Attention campaign.
> This packet prepares a transfer; campaign authority has **not** transferred
> until the handshake in this document completes.

## Transfer status

- **Campaign:** [Addressable Attention #102](https://github.com/lossyrob/telex/issues/102)
- **Repository:** `lossyrob/telex`
- **Prepared from main:** `636ecce360de80bbcbc0d18a61d6d6cdbbcf23f3`
- **Telex release:** `v0.1.2`, build `636ecce360de80bbcbc0d18a61d6d6cdbbcf23f3`
- **Current campaign authority:** laptop session
  `362234a1-1051-48ba-a6f0-0c1fb98eeac0`
- **Laptop local control:** `telex://lossyrob/telex/T-A:campaign-orch`
  on backend `local`
- **Laptop PostgreSQL relay:**
  `telex://lossyrob/telex/T-A:campaign-handoff-laptop`
  on backend `pg-rde-telex`
- **Future devbox campaign address:** pending builder input
- **Authority state:** `prepared-not-transferred`

The devbox orchestrator becomes primary only after it verifies this packet and
both orchestrators complete the explicit authority handshake below. Until then,
the laptop campaign orchestrator remains authoritative.

## Campaign purpose and current stage

The campaign makes long-duration external observation and human-attention
routing durable application responsibilities rather than session-owned polling
or terminal-tab inspection.

Completed:

- Operator Station vertical spike, viability gate, production contract, ADR
  0047/0048, and merged requirements export.
- Telex Watcher vertical spike, viability gate, production contract, ADR 0046,
  four canonical schemas, and merged requirements export.
- Formation of the shared Application Client workstream.

Current stage:

- Stage 3, shared contract convergence.
- Application Client node #118 is the only in-progress campaign node.
- Product implementation nodes remain planned and blocked on
  `application-client-ready`.

## Workstream inventory

### Operator Station

- **Workstream:** `operator-station`
- **Tracker:** [#92](https://github.com/lossyrob/telex/issues/92)
- **Artifacts:**
  - `.streamliner/workstreams/operator-station/brief.md`
  - `.streamliner/workstreams/operator-station/graph.json`
  - `.streamliner/workstreams/operator-station/reconciliation-note.md`
- **Completed node:** #114 / PR #116
- **Merge:** `0722051760bab569d3f947fd7b29f2dabe13ef77`
- **Planned/blocked:** `station-app`, `operator-broker`
- **Dependency:** `application-client/application-client-ready-gate`
- **Laptop orchestrator address:**
  `telex://lossyrob/telex/T-A:operator-station-orch` on backend `local`
- **Next responsibility:** review the exact #118 candidate bundle as a consumer.

### Telex Watcher

- **Workstream:** `telex-watcher`
- **Tracker:** [#100](https://github.com/lossyrob/telex/issues/100)
- **Artifacts:**
  - `.streamliner/workstreams/telex-watcher/brief.md`
  - `.streamliner/workstreams/telex-watcher/graph.json`
  - `.streamliner/workstreams/telex-watcher/reconciliation-note.md`
- **Completed node:** #110 / PR #115
- **Merge:** `09aa6f45f213b45207adc4cf80676dcce91250da`
- **Planned/blocked:** `watcher-runtime`, `detector-template-library`
- **Dependency:** `application-client/application-client-ready-gate`
- **Laptop orchestrator address:**
  `telex://lossyrob/telex/T-A:watcher-orch` on backend `local`
- **Next responsibility:** review the exact #118 candidate bundle as a consumer.

### Telex Application Client

- **Workstream:** `application-client`
- **Tracker:** [#117](https://github.com/lossyrob/telex/issues/117)
- **Artifacts:**
  - `.streamliner/workstreams/application-client/brief.md`
  - `.streamliner/workstreams/application-client/graph.json`
  - `.streamliner/workstreams/application-client/docs/initial-shaping.md`
- **In-progress node:** [#118](https://github.com/lossyrob/telex/issues/118),
  `contract-convergence`
- **Checkpoint:** `application-client-ready-gate`, planned
- **Laptop orchestrator address:**
  `telex://lossyrob/telex/T-A:application-client-orch` on backend `local`
- **ADR:** 0049 reserved; not landed
- **Issue #12:** unchanged; shared contract/checkpoint not published

Current #118 working state:

- Worktree:
  `C:\Users\robemanuele\proj\telex\telex-app-client-118`
- Branch: `feature/app-client-contract-118`
- HEAD: `3d46fc51ed89b98d0032e59b90bb635c1f0f0539`
- Plan: revision 19
- Current exact Plan.md:
  - bytes: `52830`
  - SHA-256:
    `812339c56111ec5a1210fd371f3fe62947ae1b06cd79a2b42c764318271b84aa`
- Dirty files:
  - `.paw/work/application-client-contract-118/Plan.md`
  - `.paw/work/application-client-contract-118/approvals/ledger.json`
  - `.paw/work/application-client-contract-118/inputs/source-freeze.json`
- Branch checkpoint is not yet on origin. Campaign transfer-prep message `1885`
  requests a durable pushed checkpoint before authority transfer.
- Revision 17 approval is stale. The current plan must use existing ADR 0049
  allocation rather than request a duplicate determination.

### Local daemon substrate

The repository also contains the `local-daemon` workstream:

- **Tracker:** #32
- **Artifacts:**
  - `.streamliner/workstreams/local-daemon/brief.md`
  - `.streamliner/workstreams/local-daemon/graph.json`
- All implementation/release nodes are completed.
- `hardening-gate` is ready; `closure-gate` is planned.
- This workstream is adjacent substrate, not part of Addressable Attention
  campaign authority unless the builder explicitly adds it.
- Issue #119 is closed and merged into Telex v0.1.2; its old worktree is not
  active campaign work.

## ADR ledger

| ADR | Workstream/node | State | Allocation source |
|---:|---|---|---|
| 0046 | Watcher `watcher-contract` | merged | message `1625` |
| 0047 | Operator `station-contract` | merged | message `1635` |
| 0048 | Operator `station-contract` | merged | message `1635` |
| 0049 | Application Client `contract-convergence` | allocated, not landed | message `1817`, base `7a568c43413fc7aeab6a484b07dce0f0db11d68f` |

Do not renumber or request a second ADR determination for #118. Revalidate ADR
0049 against latest main and the allocation ledger before editing
`docs/design/DECISIONS.md`.

## Tracker and publication authority

- Campaign tracker: #102
- Application Client contract owner: #12
- Workstream trackers: #92, #100, #117
- Current node: #118
- Merged contract PRs: #115 and #116
- No #118 PR exists yet.
- The builder personally reviews and merges every PR.
- Campaign/workstream technical verification is not merge permission.
- Human merge permission must be routed through the exact human-attention
  process and is single-use for the cited PR evidence.

## Attention and communication policy

Forward to the human:

- PR opened/review-ready/approved/merge-floor/merged-or-closed milestones;
- node/workstream completion and reconciliation;
- current blockers and `decision-needed` items;
- material blocker changes.

Filter:

- routine CI transitions;
- detector attempts and polling results;
- duplicate status;
- unchanged blockers;
- low-value progress ticks.

The laptop remains the local human-attention and workstream relay unless the
builder moves those responsibilities:

- local human mediator: `attention:rob`
- Operator Console: `operator:rob`
- local workstream orchestrator addresses listed above
- PostgreSQL campaign relay:
  `telex://lossyrob/telex/T-A:campaign-handoff-laptop`

## Authority-transfer protocol

1. Builder provides the exact devbox PostgreSQL-backed campaign address.
2. Devbox orchestrator sends a disposition-required `campaign-transfer-hello`
   to `telex://lossyrob/telex/T-A:campaign-handoff-laptop` on backend
   `pg-rde-telex`. It includes:
   - devbox session ID;
   - exact address;
   - Telex version/build;
   - station health;
   - fetched repository main commit.
3. Laptop replies with `campaign-transfer-package`, referencing:
   - this Git commit and both handoff artifact paths;
   - exact artifact SHA-256 values;
   - current #118 transfer-checkpoint branch/head/digest;
   - all outstanding obligations and authority rules.
4. Devbox verifies:
   - main and workstream graphs;
   - campaign/workstream issues;
   - ADR ledger;
   - #118 branch checkpoint and dirty/clean state;
   - PostgreSQL answerback to both campaign stations.
5. Devbox sends disposition-required `campaign-transfer-accept` with its
   verification evidence and the exact handoff package identity.
6. Builder confirms the authority switch.
7. Laptop sends `campaign-authority-transferred` to all local workstream
   orchestrators and records the devbox address. From that point:
   - devbox owns campaign decisions and cross-workstream sequencing;
   - laptop acts only as local Telex/Operator Console relay unless directed;
   - no plan, checkpoint, or merge approval is duplicated across orchestrators.
8. Both sides terminally disposition the transfer thread. GitHub #102 receives
   the authority-transfer record.

## Required first actions on devbox

1. Clone/fetch `lossyrob/telex` and check out current `origin/main`.
2. Load `.streamliner/shaping/roadmap.md` and this packet.
3. Load all four workstream graphs; treat `local-daemon` as adjacent, not owned.
4. Verify Streamliner discovers #117 and node #118 as in progress.
5. Verify #118 checkpoint commit/branch after the laptop reports message `1885`
   complete.
6. Establish the devbox PostgreSQL campaign station and complete the authority
   protocol before directing any worker.
7. Continue #118 from the latest exact plan revision; do not replay stale plan
   approvals or reallocate ADR 0049.

## Known risks during transfer

- #118 currently has uncommitted planning state; transfer is not seamless until
  the requested checkpoint is pushed.
- Laptop workstream orchestrators currently attend the local SQLite store. The
  PostgreSQL laptop campaign address is the relay unless they later attach
  dedicated PostgreSQL stations.
- Operator Console is local to the laptop.
- Do not run destructive Telex tests on either coordination plane.
- Do not merge PRs from either campaign orchestrator.

