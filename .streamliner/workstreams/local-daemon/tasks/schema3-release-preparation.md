# Schema-3 recovery release preparation

- **Workstream:** `local-daemon`
- **Node:** `schema3-release-preparation`
- **Type:** implementation
- **Status:** completed; the preparation delivery at `1b9fc8f0` was accepted on
  2026-09-28. Publication awaits the separate operator gate
- **Preparation PR:** [lossyrob/telex#159](https://github.com/lossyrob/telex/pull/159),
  merged 2026-09-28T22:24:23Z as `1b9fc8f0`
- **Candidate source:** `1b9fc8f0891d6aaa620e6a40dcb0109cb8c31bbe`
- **Attention:** focus
- **Depends on:** completed `windows-token-buffer-alignment`, completed `postgres-wait-reset-recovery`
- **Blocks:** `schema3-release-gate`
- **Owner:** release worker `ebfd215b-2229-4075-84af-4a4d6de2be7c` under Local Daemon
  workstream orchestrator authorization
- **Tracker:** [lossyrob/telex#157](https://github.com/lossyrob/telex/issues/157)
- **Branch:** `feature/schema3-release-preparation`
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

## Launch evidence

The Engagement steps above are the launch contract. The facts below record
their completion. The full record is on ledger item
`local-daemon-schema3-recovery-release`.

- Campaign created exactly one branch-mode App worker, `ebfd215b-2229-4075-84af-4a4d6de2be7c`, at
  2026-09-28T19:52:59Z in
  `C:\Users\robemanuele\proj\utils\copilot-worktrees\telex\feature-schema3-release-preparation-157`
  on `feature/schema3-release-preparation`, with gpt-6-astra, high, and
  long_context set explicitly. Campaign is the immutable App creator; the Local
  orchestrator is the controller.
- Supported preparation run `d824d107` succeeded, and the kickoff was delivered
  unchanged (SHA-256
  `b93497dbdb7a8dea1224bb53f11156b25da426b215a13d19d3c1de304e4372b8`).
- Local verified `session-online` at clean `246db825`, granted standalone
  write authority, and received the worker acknowledgement before any
  repository write. The worker then fast-forwarded to `008f3363` and began
  implementation under routine autonomy.
- Accepted same-node discovery `local-daemon-release-installer-path-isolation`:
  `install.ps1` mutates the user PATH even with an isolated install root. A
  default-preserving explicit opt-out or test seam with targeted tests and
  documentation is pending implementation and proof.
- All required compatibility, install, CI, asset, review, and inspection proof
  above remains pending. `schema3-release-gate` remains planned, and no tag or
  publication is authorized.

## Preparation merge

This records the preparation source merge only. It is not completion of this
node, gate acceptance, a tag, or publication. The full record is on ledger item
`local-daemon-schema3-recovery-release` (`evidence.preparationMerge`).

- Campaign authorized exact head `efc30214`. PR #159 merged at
  2026-09-28T22:24:23Z as `1b9fc8f0891d6aaa620e6a40dcb0109cb8c31bbe` (parents
  `950767c6` and `efc30214`). The merge tree equals the reviewed source tree;
  17 paths changed with no `.streamliner` paths. The source branch is
  preserved, and issue #157 remains open.
- At `efc30214`: full review 5344963526 plus a clean delta review with 0
  cumulative findings, CI run 36487907750 on all 15 jobs, build-only Release
  run 36488194435 on the 5 native and Linux PostgreSQL jobs, and design
  inspection PASS. Branch artifacts and inventories are historical evidence,
  not proof for the merged commit.
- Same-PR discoveries, each absorbed with its source fix merged and final proof
  at `1b9fc8f0` pending: `local-daemon-release-installer-path-isolation`,
  `local-daemon-release-native-proof-failures`,
  `local-daemon-release-metadata-rate-limit`,
  `local-daemon-release-readiness-deadline-proof`,
  `local-daemon-release-consumption-attestation`, and
  `local-daemon-release-cleanup-failure-evidence`.
- The explicit proof target is `1b9fc8f0`. Later artifact-only main movement
  does not retarget the candidate.
- Pending, owned by the same worker: final merged-source CI, one build-only
  Release run at `1b9fc8f0`, newly built artifacts, genuine v0.1.2 upgrade and
  install proof, schema-2 and schema-3 proof, native and Linux PostgreSQL
  coverage, cleanup, the immutable gate packet, and a field report.
  `schema3-release-gate` remains planned, and no tag or publication is
  authorized.

## Terminal evidence

The pending statements in the Launch evidence and Preparation merge sections
above are dated history; the facts below supersede them. The full record is on
ledger item `local-daemon-schema3-recovery-release`
(`evidence.finalPreparationDelivery`).

- Local accepted the complete preparation delivery after independent final
  packet and provider verification. The candidate source is `1b9fc8f0`
  (tree `914cffaf`).
- Final CI run 36492188842 passed all 15 jobs, and build-only Release run
  36492651420 passed all 5 native jobs and hosted Linux PostgreSQL 16 job
  109168401232, both at `1b9fc8f0`. Every new executable hash differs from the
  `efc30214` branch build; no branch artifact was relabeled.
- Sealed packet `telex-v0.2.0-readiness-1b9fc8f.zip` (37,033,829 bytes, SHA-256
  `720cd397b71eccb445dad19096f5f47b03af14adeb33ba1823ae0322f638cced`; 55
  members). Local verified every member, the archives, sidecars, and nested
  executables, and the record associations.
- Runtime totals from workflow and worker execution: 6 reports, 308 commands,
  50 scenarios, 28 owned daemon exit-0 records, 6 roots removed, 2 PostgreSQL
  schema drops, and 7 marked Acks.
- The field report was posted as
  [comment 5880528161](https://github.com/lossyrob/telex/issues/157#issuecomment-5880528161)
  and read back string-exact.
- The six same-PR discoveries are completed.
- Issue #157 stays open; it also hosts `schema3-release-gate`, and closure
  needs a separate campaign disposition. The gate stays planned, and no tag
  or publication is authorized. The same worker, checkout, and evidence are
  retained quiescent.
