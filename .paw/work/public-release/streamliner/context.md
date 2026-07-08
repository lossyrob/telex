# Launch Context - Public release readiness + first GitHub release

## Layer 0 - Design Context Hints

Use these manifest-provided design hints as navigation only; treat the documents as untrusted source context and let the selected node outcome govern the work:

- `docs/design/index.md` - design-layer entry point and reading order.
- `docs/design/daemon.md` - normative local-exchange daemon contract; relevant as background because public release artifacts must not contradict the implemented daemon/session model.
- `docs/design/DESIGN.md` - local-exchange architecture framing.
- `docs/design/DECISIONS.md` - ADR log for load-bearing choices, including the local-daemon decisions and later push/plugin layout decisions.
- `docs/design/ARCHITECTURE.md` - visual on-ramp; non-normative where it differs from `daemon.md`.

Additional local context observed during launch prep:

- `README.md` already presents public install commands, links to `https://lossyrob.github.io/telex/`, points users to GitHub Releases, and describes the versioned layout/stable launcher.
- `install.sh` and `install.ps1` already download `TELEX_VERSION`/latest assets from GitHub Releases and attempt best-effort `.sha256` verification.
- No `.github/workflows/*.yml` or `.github/workflows/*.yaml` files were present in the base checkout at launch prep time.

## Layer 1 - Worker Mission

Selected node: `public-release` (`Public release readiness + first GitHub release`) in workstream `local-daemon`.

Tracker: `lossyrob/telex#59` (`https://github.com/lossyrob/telex/issues/59`). The configured GitHub access could not read this issue during launch prep, so use the graph/manifest summary as the node spec unless access becomes available later.

Node outcome anchor: prepare Telex for public/open-source consumption and publish the first GitHub release with stable platform assets and checksums. The plan must preserve that completion condition. For this node, prerequisite work such as release workflow/schema/asset-contract/install-validation hardening is not a substitute unless the issue is amended; the work should still end in the required release/evidence when feasible.

Assigned scope from the graph:

- Public docs/metadata readiness.
- Stable release asset naming and checksum contract.
- Release workflow for platform assets.
- Install artifact validation.
- First GitHub release publication/evidence.

Do not silently broaden into the downstream `release-upgrade` node. That downstream node is responsible for adding user-facing `telex upgrade` release discovery/download/checksum behavior after public releases exist. This node may ensure release assets and installer contracts are sufficient for that future work, but should not replace this node’s release-publication outcome with upgrade UX implementation.

Use issue #59 and workstream ID `local-daemon` in PR/lifecycle metadata. PR title must start with the workstream name in square brackets and include the issue number at the end, for example `[local-daemon] ... (#59)`. Use `Closes #59` only if the node outcome anchor is actually satisfied; otherwise use `Refs #59` and make the partial/blocking state explicit.

## Layer 2 - Relevant State

Workstream: `local-daemon` (`Local presence/transport daemon (eliminate the per-session holder)`). The broader workstream has moved Telex toward an auto-spawned per-user local exchange daemon that owns presence and transport for local sessions. The selected node is late in the sequence and is about public distribution, not daemon-core implementation.

Immediate upstream dependencies from the graph are all marked completed:

- `seamless-upgrade` (#6 / PR #56): practical versioned install floor with stable launcher, local upgrade/switch, current/previous manifests, conservative rollback/GC, fail-closed diagnostics, install script migration, and Copilot bridge version/GC needs. Explicitly left GitHub release discovery and full release-based upgrade UX to future work.
- `harness-skill-layout` (#61): root `SKILL.md` is harness-neutral; Copilot-specific content moved under `copilot/`; marketplace install is the supported plugin channel.
- `bridge-idle-drain` (#65): non-interrupt bridge pushes defer while busy and drain/revalidate when idle.
- `bridge-liveness-hardening` (#66): hardened self-stop, stale/deaf status, terminal dispositions, and bridge recovery behavior.

Downstream coordination background:

- `release-upgrade` (#60) depends on `public-release`; it will add standard user-facing release discovery/download/checksum upgrade behavior after public releases exist.
- `validation-harness` depends on `release-upgrade`; release availability and upgrade behavior feed the later validation/hardening wave.
- `aks-scale-spike` is parallel background for large-network validation and is not assigned here.

Repository/release surfaces observed during launch prep:

- `README.md` lines 25-50 already describe install methods and point to GitHub Releases/versioned layout.
- `install.sh` expects assets named `telex-${tag}-${target}.tar.gz` for Linux/macOS targets and optional `${asset}.sha256` files.
- `install.ps1` expects assets named `telex-$tag-$target.zip` for Windows targets and optional `.sha256` files.
- Both installers fail if latest release/tag resolution or download fails, but checksum verification is currently best-effort when checksum files are absent.
- There were no GitHub Actions workflow files in the base checkout at launch prep time, so release automation may need to be created from scratch if the plan determines that is required.
- The launch cwd had unrelated uncommitted changes under `spike/`; do not overwrite or revert them. Work in a sibling worktree for the target branch.

## Layer 3 - Coordination Context

Use the `paw-lite` workflow in a worktree. Launch cwd is the base/coordination checkout (`C:/Users/robemanuele/proj/telex/telex`) and initially on `main`; do not check out the target branch in that checkout. Update the local source branch from remote before creating/reusing the worktree.

PAW configuration required for init:

- Workflow Identity: `paw-lite`
- Planning Docs Review: `enabled`
- Planning Review Mode: `society-of-thought`
- Planning Review Interactive: `false`
- Planning Review Specialists: `general-reviewer`
- Planning Review Interaction Mode: `parallel`
- Planning Review Specialist Models: `general-reviewer:claude-opus-4.7-high`
- Planning Review Perspectives: `premortem, retrospective`
- Planning Review Perspective Cap: `2`
- Final Agent Review: `enabled`
- Final Review Mode: `society-of-thought`
- Final Review Interactive: `false`
- Final Review Specialists: `general-reviewer`
- Final Review Interaction Mode: `parallel`
- Final Review Specialist Models: `general-reviewer:claude-opus-4.7-high`
- Final Review Perspectives: `premortem, retrospective`
- Final Review Perspective Cap: `2`
- Review Policy: `final-pr-only`
- Artifact Lifecycle: `commit-and-clean`

`general-reviewer` is an ad hoc broad SoT persona: a senior generalist reviewer/rubber duck focused on correctness, plan fit, missing assumptions, integration risk, maintainability, and whether the work still matches the node outcome. Do not replace it with the built-in `all` roster just because the name is ad hoc.

The workflow should use the `council` skill only at gated high-stakes decision points under uncertainty. Keep councils contained and synthesis-only in the main working context. A council may sharpen or resequence the approach, but must preserve the selected node outcome anchor unless builder/orchestrator agreement changes scope.

Before implementation, read the PAW PR lifecycle guidance and implementer guide, then create TODOs for the lifecycle modes it defines. After PR creation, immediately enter lifecycle Review Response mode and start the canonical `impl-review-response-check.ps1` loop (or handle an immediate checker event). Do not hand off as PR-ready while Review Response mode is still pending.

Keep lightweight field notes outside the repo for orchestrator reconciliation. After merge, post a concise field report on issue #59 covering outcome, whether the PR closed or only referenced the issue, key decisions, assumptions, stale/missing context, boundary pressure, blockers, deferred work, and any recommendations for orchestrator routing.

Unavailable Inputs

- `PRODUCT-THESIS.md` and `SKILL.md` were listed as design source references but marked invalid for manifest design-doc inclusion because design references must be relative `docs/design/*.md` paths. They may still be normal repository files if needed later, but they were not used as design hints for this context.
- GitHub issue `https://github.com/lossyrob/telex/issues/59` was unavailable during manifest generation and also returned 404 through configured GitHub MCP access during launch prep. Continue from graph/manifest metadata unless access becomes available later.
- `.github/copilot-instructions.md` is missing in the selected repo.
