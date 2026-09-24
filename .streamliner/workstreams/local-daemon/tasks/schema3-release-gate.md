# Schema-3 recovery release gate

- **Workstream:** `local-daemon`
- **Node:** `schema3-release-gate`
- **Type:** gate
- **Status:** planned; explicit operator approval required
- **Attention:** focus
- **Depends on:** completed `schema3-release-preparation`
- **Owner:** campaign and operator
- **Evidence tracker:** [lossyrob/telex#157](https://github.com/lossyrob/telex/issues/157)
- **Parent workstream:** [lossyrob/telex#32](https://github.com/lossyrob/telex/issues/32)
- **Campaign:** [Addressable Attention #102](https://github.com/lossyrob/telex/issues/102)

## Decision

Approve or decline tagging and publication of the immutable reviewed v0.2.0
schema-3 recovery candidate. Preparation completion and green checks do not imply
approval. The operator must approve publication explicitly.

This gate governs only the bounded recovery release. It does not accept Local
Daemon hardening, workstream closure, PR #138, issues #152/#153, or
Watcher/Station runtime delivery.

## Required evidence

- Exact candidate source head, tree, commits, and review/inspection decisions.
- Merged issue #154 and #155 repairs with exact-head campaign merge authority.
- Consistent v0.2.0 release metadata.
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
