# Application Client Contract Final Field Report

## Report identity

- Work item: issue #118, Application Client contract convergence
- Generated: 2026-07-25T22:04:59.063Z
- Branch: `feature/app-client-contract-118`
- Evidence branch head before this report:
  `d055bd36e6ebc69d7b6abb9bd0519a78586f1686`
- Current base reference: `origin/main` at
  `a5180eb5884a73ce1df1c7c893af9b8889d646bd`
- Scope: design-only contract, crosswalk, bundle, publication, and approval
  evidence through Gate 6 final review

This report is the pre-PR durable evidence snapshot required before
`commit-and-clean`. Gate 7 PR review, sentry, human merge-floor, merge
authorization, and post-merge reconciliation are intentionally pending and are
not represented as complete.

## Outcome

The Application Client contract converges every exported Watcher and Operator
Station shared semantic requirement into one API-neutral contract. The branch
contains no production client implementation and no Streamliner changes.

- Watcher requirements: 15 accepted, 0 deferred, 0 rejected
- Operator requirements: 15 accepted, 0 deferred, 0 rejected
- Combined crosswalk: 30 accepted, 0 deferred, 0 rejected
- Final PAW review: PASS
- Final-review findings: 0 must-fix, 0 should-fix, 5 non-blocking consider
- Issue #12 publication: published and exactly verified
- Private fallback seams: prohibited

## Immutable identities

| Artifact | Identity |
|---|---|
| Approved Plan.md revision 20 | source head `855563a30f053f4cc563494793d98b2beb355ce0`; blob `c514abe05a46cd55eef84f967c343d08378a0926`; 53872 bytes; SHA-256 `fc0cfee6809644b71f26f2a0a8bf7c4b419c678a267739bed8a768d00bb9297f` |
| Final contract bundle | source head `8854e5e36ac5c18320b208d1a90bffaffbff33a5`; manifest blob `04c294f0790a5ba764354def07d1108575918d2e`; 2411 bytes; SHA-256 `abe8abb8f5be5544651d18e1c535b16a94ff99690b835b7c08bb3541292b0e87` |
| Exact publication revision 1 | source head `ee999dab46b0a8a67f09f3a4bdf6b8203ee21c2d`; blob `89f22602d61e77464a98a224ad2505708a2e0f9c`; 7059 bytes; SHA-256 `a7857aebd125e94c487b2ddac6e807f5dc9df7a4d934c2dd9277268c9093e14e` |
| Historical issue #12 body | `docs/design/history/application-client-issue-12-original.md`; blob `3ad98e9f9b455b03fe4582943aabfea47b702505`; 16175 bytes; SHA-256 `c0a5fed4a1fa894ccc6accedee3e0e66af318a10df4ad27e6f2f065f6881c3dc` |
| Live issue #12 | https://github.com/lossyrob/telex/issues/12; fetched 7059 canonical bytes; SHA-256 `a7857aebd125e94c487b2ddac6e807f5dc9df7a4d934c2dd9277268c9093e14e` |

## Source provenance

| Domain evidence | Immutable identity |
|---|---|
| Watcher requirement export `5042702401` | SHA-256 `9a037f94af84516592a56dc9c0c701ce0277e305c83dad368227fc25a5b18d9a` |
| Watcher merged-source addendum `5043498697` | SHA-256 `fa02b844c62eef17f4c08b9bc1d7d94539e525034e3f0d474b2bfe2d45caed94` |
| Watcher canonical design | head `e007a8067b3b91b5c57a2a756ce878e310595a05`; blob `e861119d8f26e7efaad9628558436dca789b948d` |
| Operator requirement export `5042612298` | SHA-256 `adf2f8e439e5c224059ca51142701f604a82203c39c9829d1323a88c58889f7e` |
| Operator merged-source addendum `5044388908` | SHA-256 `702ebdb1ea81329294c35a452670b2313625142bc0281c9100d8ae892890c9ea` |
| Operator canonical design | head `2d99e552292a4401d3403540b6d2eaa90272282d`; blob `82dc6de625a795140edbfec605e8b526d742118e` |

The Gate 3 preflight reproduced all source-comment digests and both canonical
design blobs. Latest-main changes did not alter either canonical domain source.

## Repository deliverables

The canonical manifest lists:

| Path | Byte length | SHA-256 |
|---|---:|---|
| `docs/design/DECISIONS.md` | 147413 | `61f9314ce4eb5ee567a2627ba06398bd550c51b1cbcdacbf30eaa4b79c403ba7` |
| `docs/design/application-client-crosswalk.md` | 27060 | `c613c7618aaac2aeda8a08baf0c4eac22b6f205d6e7ab9049693a4f9161043f5` |
| `docs/design/application-client.md` | 20065 | `53cfa825d24d9bad7d10f23ef134da8770a8cee20161d1b7660bc66a9bc1d053` |
| `docs/design/history/application-client-issue-12-original.md` | 16175 | `c0a5fed4a1fa894ccc6accedee3e0e66af318a10df4ad27e6f2f065f6881c3dc` |
| `docs/design/index.md` | 6523 | `48d1ff854ef3558df8a2d56276229a0808cd745c611bed358962f37712068c70` |

Bundle closure from the approved planning base is exactly these five files plus
`docs/design/application-client.bundle.json`. The manifest excludes itself,
temporary `.paw` evidence, the replacement issue body, and transport artifacts.

## ADR 0049 authority

- Number: 0049
- Title: `One API-neutral Application Client contract governs explicit station capabilities and forbids private fallbacks`
- Stable slug: `application-client-semantic-boundary`
- Allocation request: `1812`
- Handled disposition: `909`
- Allocation response: `1817`
- Allocation base: `7a568c43413fc7aeab6a484b07dce0f0db11d68f`
- Pre-allocation high-water: 0048

The Gate 3 preflight confirmed 0049 was absent from the latest decision log,
remained reserved to issue #118, and had no collision or competing reservation.
No alternate number or no-ADR determination was requested.

`application-client.bundle.json` retains `reserved-not-landed` as the frozen
campaign allocation-ledger status. ADR 0049 uses `Accepted (pending
validation)` as the decision-document lifecycle status. These are separate
axes; campaign/workstream reconciliation owns the post-merge allocation-ledger
transition.

## Approval inventory

### Gate 1: internal planning review

- Mode: society-of-thought, parallel, non-interactive
- Specialist: `general-reviewer`
- Model: `claude-opus-4.8-high`
- Perspectives: premortem and retrospective
- Final findings: 0 must-fix, 0 should-fix, 2 non-blocking consider

### Gate 2: exact Plan.md approval

| Approver | Request | Approval | Additional evidence |
|---|---:|---:|---|
| Application Client orchestration | 746 | 749 | approval disposition 310 |
| Campaign orchestration | 747 | 748 | exact head/blob/byte/digest attestation |

### Gate 3: consumer contract approval

The first candidate `ba2bcbe42274030ed20079f3bab9955f54a8667e` /
`ca7a96e0...` was invalidated by Watcher W-15 finding `800`. The repair added
stable opaque logical-store identity to every address/membership status
projection and updated W-15 mapping/rationale. Both source-fidelity rereviews
then passed with 0 must-fix and 0 should-fix.

| Consumer | Fresh request | Approval | Supplements / authorization |
|---|---:|---:|---|
| Operator Station | 809 | 813 | exact-head/blob supplement 817; packet supplement 820; domain authorization 792 |
| Telex Watcher | 810 | 814 | domain authorization 793 |

Campaign completion message: `818`.

### Gate 4: shared bundle approval

| Approver | Request | Approval |
|---|---:|---:|
| Application Client orchestration | 821 | 823 |
| Campaign orchestration | 822 | 824 |

Both approvals reproduced the source head, manifest blob/digest, every listed
file identity, complete 218869-byte packet SHA-256
`7b46a9c1318eeab3443082ef691d2a490da40407573c185433bb9c7bbd63d6d9`,
consumer approval inventory, design-only scope, and no-private-fallback
boundary.

### Gate 5: exact issue #12 publication

| Approver | Request | Approval | Notes |
|---|---:|---:|---|
| Application Client orchestration | 826 | 829 | exact source/blob/body and live-provenance attestation |
| Campaign orchestration | 827 | 832 | reachability blocker 828; unchanged re-request 831 after branch push |

Publication authorization: `833`.

Publication command:

`gh issue edit 12 --repo lossyrob/telex --body-file .paw/work/application-client-contract-118/publication/issue-12-body.md`

The command used the command-scoped `lossyrob` GitHub token. No assignee was
added or changed. Immediate refetch reproduced 7059 bytes and SHA-256
`a7857aebd125e94c487b2ddac6e807f5dc9df7a4d934c2dd9277268c9093e14e`
with no U+FFFD replacement character. Completion evidence messages: `835` and
`836`. Publication verification acceptance: `837`.

### Gate 6: final PAW review

- Diff: `origin/main...feature/app-client-contract-118`
- Mode: society-of-thought, parallel, non-interactive
- Specialist: `general-reviewer`
- Model: `claude-opus-4.8-high`
- Perspectives: premortem and retrospective
- Premortem: APPROVE, 0 must-fix, 0 should-fix, 3 consider
- Retrospective: APPROVE, 0 must-fix, 0 should-fix, 3 consider
- Synthesis: PASS, 0 must-fix, 0 should-fix, 5 deduplicated consider
- Review artifacts:
  - `.paw/work/application-client-contract-118/reviews/REVIEW-general-reviewer-premortem.md`
  - `.paw/work/application-client-contract-118/reviews/REVIEW-general-reviewer-retrospective.md`
  - `.paw/work/application-client-contract-118/reviews/REVIEW-SYNTHESIS.md`

## Final-review consider items

The consider-only observations are acknowledged and intentionally not applied:

1. Manifest allocation status and ADR lifecycle status are separate axes.
2. The W-15 delivered mapping is a compliant superset of the frozen Plan matrix
   floor.
3. Issue #12 uses immutable `8854e5e...` permalinks; post-merge reachability
   should be rechecked if the PR is squash-merged.
4. Historical issue #12 digest is already durably bound by the manifest, so
   restating it is redundant.
5. `AC-##` source requirement IDs and `AC-C##` contract IDs are visually close
   but internally consistent.

None is a quick win: every durable textual change would alter a bundle-listed
or publication byte and invalidate completed exact-byte approvals.

## Scope and boundary confirmation

- No `src/` or production client implementation was changed.
- No `.streamliner/` artifact was changed.
- Watcher detector/runtime policy remains outside the shared client.
- Operator Station UX, mediation, routing, notification, and human policy remain
  outside the shared client.
- CLI parsing, raw private daemon IPC, spike helpers, sender-occupancy delivery
  claims, and product-private client forks remain prohibited fallbacks.
- `application-client-ready` is design-only and does not claim implementation,
  binding, conformance, consumer integration, or production readiness.

## Pending Gate 7 and merge-floor evidence

The following are intentionally pending:

- final PR URL and exact PR head;
- green CI and clean mergeability;
- zero unresolved review threads;
- current-head paired `PAW Review: +1`;
- Watcher or Loop sentry mode;
- worker-authored `merge-floor-ready` and `node-merge-floor-ready` packets;
- workstream-authored `attention.merge-floor` request to literal
  `attention:rob`;
- explicit human approval disposition;
- campaign merge authorization citing the workstream request and human
  disposition;
- post-human `merge-ready` and `node-merge-ready` packets;
- merge and post-merge reconciliation.

This worker will not author the human merge-floor request and will not merge the
PR.

## Process feedback

- Exact-byte gates successfully caught a real W-15 status-provenance gap before
  shared approval.
- Requiring immutable GitHub link reachability before publication prevented a
  published checkpoint with temporarily broken artifact links.
- Consumer startup holds require explicit campaign authorization in the review
  request to avoid procedural rejection.
- The manifest generator and closed canonicalization transform made every
  bundle and publication identity independently reproducible.
