# Windows token buffer alignment

- **Workstream:** `local-daemon`
- **Node:** `windows-token-buffer-alignment`
- **Type:** implementation
- **Status:** planned; launch requires Tier B landing and exact verification
- **Attention:** focus
- **Depends on:** none
- **Blocks:** `hardening-gate`, `schema3-release-preparation`
- **Owner:** Local Daemon workstream orchestrator until an authorized implementer is assigned
- **Tracker:** [lossyrob/telex#154](https://github.com/lossyrob/telex/issues/154)
- **Parent workstream:** [lossyrob/telex#32](https://github.com/lossyrob/telex/issues/32)
- **Campaign:** [Addressable Attention #102](https://github.com/lossyrob/telex/issues/102)

## Outcome

Eliminate undefined behavior in both Windows token-user lookups on exact main
`ed417c6b938f92fe3bfb84f3b0cc0bea719fbbd0`. Both
`src/backend/sqlite.rs` and `src/daemon.rs` pass `Vec<u8>` storage to
`GetTokenInformation(TokenUser)` and then access it as `TOKEN_USER`. Use aligned
storage or a shared safe token-information helper, and audit every remaining
`GetTokenInformation` call site for the same pattern.

Preserve SID selection, store identity, authentication, and fallback behavior. This
repair is required before Local Daemon hardening-gate acceptance, but it is
independent of and non-blocking to PR #138.

## Inputs

- The corrected issue #154 tracker, updated `2026-09-24T21:14:24Z` with body
  SHA-256
  `16e87cf86a3fd600214a94523ee7376d6fc7d9f33a5285946499f9917adf6f69`,
  and the exact-main inspection of `src/backend/sqlite.rs` and `src/daemon.rs`.
- The daemon-only alignment repair on unmerged PR #138 as reference material, not
  main authority or a dependency.
- Current Windows identity and peer-authentication contracts.

The corrected tracker matches this authority: main retains both unsafe paths, the
daemon-only repair is unmerged, and the available evidence does not prove that
alignment caused the reported heap corruption.

## Boundaries

### In scope

- Aligned storage or a shared safe helper for `TokenUser` information.
- A Windows test proving each returned buffer satisfies
  `align_of::<TOKEN_USER>()` before dereference.
- Audit and repair of any remaining unsafe `GetTokenInformation` byte-buffer call
  sites.
- Windows platform and supported feature-combination coverage for both identity
  paths, plus required exact-head repository CI.

### Out of scope

- Weakening SID verification, peer authentication, owner-private storage, or
  fallback behavior.
- Station-intent transactional authority from issue #153.
- Application Client installed-current work from issue #152.
- Changes to, dependency on, or inheritance from PR #138.
- Claims that alignment caused heap corruption without evidence.
- Unrelated product behavior.

## Success criteria

- No Windows token structure is dereferenced through byte-aligned storage.
- Windows tests prove the alignment required by `TOKEN_USER` before dereference in
  both exact-main identity paths.
- The remaining `GetTokenInformation` call sites have been audited and do not retain
  the unsafe pattern.
- Windows platform/feature coverage, exact-head review, and required CI confirm
  that SID identity, authentication, owner-private storage, and fallback contracts
  remain unchanged.

## Engagement

- Product launch is conditional on campaign-approved Tier B landing and exact
  verification. The Local Daemon orchestrator then prepares one external repair
  session through Streamliner, registers the exact prepared checkout path in branch
  mode, verifies `session-online`, grants standalone write authority, and obtains
  acknowledgement before product writes.
- This is one of exactly three new delivery sessions in the packet: two repair
  sessions and one later release worker. Do not create dormant placeholders.
- Every new session or delegated agent must explicitly set
  `model=gpt-6-astra`, `reasoning_effort=high`, and
  `context_tier=long_context`; silent downgrade is not authorized. This artifact
  role creates or delegates none.
- Review checkouts must be physically distinct and read-only. Never use
  `open_pr_session` for a reviewer.
- Register external waits only through WATCHER
  `2a4bc4c8-1211-49d4-ba68-9d05d5d7530d`.
- Merge only the exact reviewed, green, inspected head after campaign
  merge authorization.
