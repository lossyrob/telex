# Schema-3 recovery release preparation

- **Workstream:** `local-daemon`
- **Node:** `schema3-release-preparation`
- **Type:** implementation
- **Status:** planned; launch only after both repair merges and campaign-verified dependency closure
- **Attention:** focus
- **Depends on:** completed `windows-token-buffer-alignment`, completed `postgres-wait-reset-recovery`
- **Blocks:** `schema3-release-gate`
- **Owner:** Local Daemon workstream orchestrator until the isolated release worker is authorized
- **Tracker:** [lossyrob/telex#157](https://github.com/lossyrob/telex/issues/157)
- **Parent workstream:** [lossyrob/telex#32](https://github.com/lossyrob/telex/issues/32)
- **Campaign:** [Addressable Attention #102](https://github.com/lossyrob/telex/issues/102)

## Outcome

Prepare one immutable, reviewed v0.2.0 schema-3 recovery release candidate after
issues #154 and #155 merge and the campaign verifies dependency closure.
Preparation does not authorize a tag or publication.

The candidate must include both repairs, schema-3 compatibility, truthful release
notes and exclusions, genuine isolated upgrade and fresh-install proof, exact-head
CI, the build-only Release workflow matrix, and concrete asset-to-source evidence.
Submit the immutable candidate and evidence to `schema3-release-gate`.

## Preserved incident evidence

Use the existing read-only incident assessment:

`C:\Users\robemanuele\.copilot\session-state\b737a932-c687-4b95-b273-8335a59f6e89\files\schema3-release-incident-evidence.md`

Do not repeat the shared-database investigation or read the shared database.

## Required work

### Version and metadata

- Set v0.2.0 in `Cargo.toml` and refresh `Cargo.lock`.
- Update both version values in `.github/plugin/marketplace.json`.
- Update `copilot/plugin/plugin.json`.
- Update the `--plugin-version` value in
  `copilot/plugin/skills/telex/SKILL.md`.
- Audit and align every other release-coupled metadata surface.
- Audit the schema-2 fixture in `src/commands/upgrade.rs` for historical versus
  current meaning. Preserve historical schema-2 semantics where intended; do not
  replace `2` mechanically or weaken the old schema guard.

### Compatibility and install proof

- Install a genuine v0.1.2 binary in disposable, isolated install, config, and
  database roots. Never use the operator's actual binary, installed-user daemon,
  shared configuration, or shared database.
- Exercise the v0.1.2 binary's real `telex upgrade` path against controlled
  candidate release, manifest, asset, and checksum endpoints.
- Prove the protocol 1.4-to-1.5 daemon transition without touching an
  installed-user daemon.
- Exercise a representative schema-2 migration and a preexisting schema-3
  connection using disposable databases.
- Exercise fresh `install.ps1` and `install.sh` installs from controlled candidate
  assets.
- Treat manifest schema range 2..3 as inference until the old-binary upgrade
  exercise succeeds.

### Candidate and evidence

- Write release notes that identify schema-3 support, both repairs, supported
  upgrade behavior, downgrade limits, and explicit exclusions.
- Exclude PR #138 station-intent restoration, issue #152 Application Client
  consumer bootstrap, issue #153 transactional authority, Watcher/Station
  runtimes, and campaign closure.
- Run required exact-head CI and the build-only Release workflow matrix.
- Verify every platform asset, checksum, executable build identity, and
  candidate/source association.
- Obtain exact-head implementation review plus compatibility and design
  inspection.
- Bind exact commands, outcomes, artifact identities, coverage, limitations, and
  uncertainty to one immutable candidate.

## Boundaries

- Keep completed release nodes historical; do not reopen or reset them.
- Do not relax the old schema guard, redesign automatic schema policy, or add
  unrelated features.
- Do not use or mutate production/shared databases, shared configuration, the
  operator installation, or the installed-user daemon.
- Do not merge or depend on PR #138. Do not revive writers for PR #138, issue
  #152, issue #153, Watcher/Station runtimes, or campaign closure.
- Do not tag, publish, or install for the operator under preparation authority.

## Success criteria

- Both prerequisite repair merges and campaign dependency verification are
  recorded before release work starts.
- All release-coupled metadata agrees on v0.2.0.
- A genuine isolated v0.1.2 binary completes the real controlled upgrade path,
  including manifest and checksum handling and protocol 1.4-to-1.5 transition.
- Fresh PowerShell and shell installs succeed from controlled candidate assets.
- Disposable tests prove representative schema-2 migration and preexisting
  schema-3 connection behavior.
- Exact-head CI and the build-only Release matrix pass.
- Every platform asset and checksum resolves to the reviewed source head and
  reports the expected executable build identity.
- The immutable candidate packet states tested compatibility, inference,
  exclusions, downgrade limits, and remaining uncertainty without overclaiming.

## Engagement

- Launch one new isolated release worker only after both repair merges and
  campaign-verified dependency closure. This is the third and final new delivery
  session in the packet; do not create a dormant placeholder.
- The Local Daemon orchestrator prepares the worker through Streamliner, registers
  the exact prepared checkout path in branch mode, verifies `session-online`,
  grants standalone write authority, and obtains acknowledgement before product
  writes.
- Every new session or delegated agent must explicitly set
  `model=gpt-6-astra`, `reasoning_effort=high`, and
  `context_tier=long_context`; silent downgrade is not authorized. This artifact
  role creates or delegates none.
- Review checkouts must be physically distinct and read-only. Never use
  `open_pr_session` for a reviewer.
- Register external waits only through WATCHER
  `2a4bc4c8-1211-49d4-ba68-9d05d5d7530d`.
- Product merge requires exact-head campaign merge authorization.
- After separate operator approval at `schema3-release-gate`, the same release
  worker owns tagging, publication, and installation verification. Do not create
  another permanent release session.
