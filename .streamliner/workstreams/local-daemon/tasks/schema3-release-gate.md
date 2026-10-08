# Schema-3 recovery release gate

- **Workstream:** `local-daemon`
- **Node:** `schema3-release-gate`
- **Type:** gate
- **Status:** completed; the operator approved v0.2.1 publication from
  `212b76a4` on 2026-09-30, and it was published and verified. The earlier
  planned wording below is dated history
- **Attention:** focus
- **Depends on:** completed `schema3-release-preparation`
- **Owner:** campaign and operator
- **Evidence tracker:** [lossyrob/telex#157](https://github.com/lossyrob/telex/issues/157)
- **Parent workstream:** [lossyrob/telex#32](https://github.com/lossyrob/telex/issues/32)
- **Campaign:** [Addressable Attention #102](https://github.com/lossyrob/telex/issues/102)

## Decision

Approve or decline tagging and publication of the immutable, reviewed corrective
v0.2.1 schema-3 recovery candidate after its source is reviewed, merged, and
proven at the final merged commit. Preparation completion and green checks do not
imply approval. The operator must approve publication explicitly. The
2026-09-30 v0.2.0 approval covered only source `1b9fc8f0` and does not carry
over to v0.2.1.

This gate governs only the bounded recovery release. It does not accept Local
Daemon hardening, workstream closure, PR #138, issues #152/#153, or
Watcher/Station runtime delivery.

## Required evidence

- Exact candidate source head, tree, commits, and review/inspection decisions.
- Merged issue #154 and #155 repairs with exact-head campaign merge authority.
- Consistent v0.2.1 release metadata.
- Bounded causal evidence for the v0.2.0 tag-run Windows proof failure and its
  reviewed correction.
- Genuine isolated v0.1.2 upgrade through controlled release, manifest, asset, and
  checksum paths.
- Fresh `install.ps1` and `install.sh` evidence from controlled candidate assets.
- Protocol 1.4-to-1.5 daemon transition evidence from disposable roots.
- Representative schema-2 migration and preexisting schema-3 connection evidence
  from disposable databases.
- Required exact-head CI and build-only Release workflow matrix results.
- Every platform asset, checksum, executable build identity, and source
  association.
- Truthful release notes, exclusions, supported upgrade behavior, downgrade
  limits, tested compatibility, remaining inference, and uncertainty.

## Gate result

- **Approve:** record explicit operator approval, then return tagging,
  publication, and installation verification to the same release worker.
- **Decline or defer:** preserve the immutable candidate and evidence. Do not tag
  or publish.

No additional permanent release session is authorized.

## Publication attempt history

- 2026-09-30: the operator approved v0.2.0 publication from `1b9fc8f0` only.
  The same worker pushed tag `v0.2.0`, and tag Release run 36734022444
  (attempt 1) failed on Windows x64 job 109950859494; the Linux PostgreSQL and
  Publish jobs were skipped. Nothing was published, and v0.1.2 remains the
  latest release. The tag stays unpublished; no retry, tag move, deletion, or
  withdrawal is authorized.
- The operator then chose to hold v0.2.0 and prepare a reviewed corrective
  v0.2.1 candidate. This gate stays planned and unaccepted, and issue #157
  stays open.

## Engagement

- Every new session or delegated agent must explicitly set
  `model=gpt-6-astra`, `reasoning_effort=high`, and
  `context_tier=long_context`; silent downgrade is not authorized. This artifact
  role creates or delegates none.
- Gate review checkouts must be physically distinct and read-only. Never use
  `open_pr_session` for a reviewer.
- Register external waits only through WATCHER
  `2a4bc4c8-1211-49d4-ba68-9d05d5d7530d`.
- If the operator approves publication, the same isolated release worker resumes
  only after exact-path verification and explicit acknowledgement. No fourth
  delivery session is created.

## Gate outcome

The full record is on ledger item `local-daemon-schema3-recovery-release`
(`evidence.v021Publication` and `completionEvidence`).

- The operator chose "Authorize exact-source v0.2.1 tag and publication
  (Recommended)" only for source `212b76a4` and its sealed readiness packet.
  This was new authority; the v0.2.0 approval did not carry over.
- The same worker pushed lightweight tag `v0.2.1` at `212b76a4`. Tag Release
  run 36754203801 (attempt 1) passed all 8 jobs, including the tag check, 5
  native builds with both Windows native controls, Linux PostgreSQL, and
  Publish. No rerun, tag move, or withdrawal.
- [Release v0.2.1](https://github.com/lossyrob/telex/releases/tag/v0.2.1) (ID 400301560) was published at
  2026-09-30T17:57:24Z as stable and Latest, with 10 live assets.
- Local independently hashed the live assets. Isolated post-publication runs on
  Windows x64 and Ubuntu WSL x64 used the tagged installers against live
  endpoints and upgraded a real published v0.1.2 install.
- Publication packet SHA-256 `2f6d35836e9564cad3cc0a356e39c6081bd6825a780912476e93d2a0ad81f562`
  (49 members). Field report:
  [comment 5916978763](https://github.com/lossyrob/telex/issues/157#issuecomment-5916978763).
- Campaign accepted only this bounded gate completion, and Local then set tracker 157 to closed as completed at 2026-09-30T18:16:08Z (event 32189988997) by an explicit provider action, not a commit directive.
- The `v0.2.0` tag stays at `1b9fc8f0`, unpublished history, and the original
  Errno 13 cause remains unknown. This gate does not accept hardening or
  closure.
