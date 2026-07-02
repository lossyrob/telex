# Plan

## Approach Summary

Issue #6's node outcome anchor is the full seamless-upgrade platform: versioned install plus stable launcher, upgrade/rollback/gc/version UX, daemon drain/respawn integration, compatibility gates, mixed-version observability, installer updates, and Copilot bridge cleanup/versioning.

This implementation will deliver the local versioned-install and compatibility floor in one PR: the normal Rust `telex` binary doubles as the stable launcher when installed at `<root>/bin/telex(.exe)`, installed versions live under `<root>/versions/<tag>/`, `<root>/current` is the atomic selection pointer, upgrade/rollback drain the current daemon before switching, GC is conservative about current/previous/running versions, and Copilot bridge GC stays in the Copilot command boundary. The PR will use `Refs #6`, not `Closes #6`, unless implementation also proves the remaining issue-wide closure criteria; the original node completion condition remains preserved for reconciliation.

The plan intentionally does not add Rust-side GitHub release discovery in this PR: the current crate has no HTTP client dependency and the existing release download flow lives in install scripts. That omission is the reason this plan remains a partial/local floor rather than full issue closure.

Council synthesis used for this plan: `C:\Users\robemanuele\.copilot\session-state\a08f3885-d94e-4af1-8806-bcf305d8162f\files\council-seamless-upgrade-plan\synthesis.md`.

## Work Items

- [x] **Versioned install layout and launcher dispatch**: add install/layout helpers, same-binary launcher mode in `src/main.rs`, current-pointer resolution, version manifests, atomic file writes, process/stdio-safe child execution, and migration from today's flat install layout into the new version store.
- [x] **Upgrade, rollback, gc, and version UX**: add top-level `version`, `upgrade`, `rollback`, and `gc` commands with JSON/text output, daemon drain-before-switch integration, local/archive install inputs, compatibility prechecks, conservative GC, and mixed-version observability.
- [x] **Copilot bridge version and GC UX**: add `telex copilot gc`, expose bridge protocol/min-plugin/install metadata in status/version output, and keep all bridge cleanup in `commands/copilot.rs` without daemon-core Copilot coupling.
- [x] **Postgres/schema compatibility hardening**: add executable Postgres schema-version metadata and fail-closed checks comparable to SQLite for daemon-aware binaries, plus status/version reporting for protocol/schema compatibility so too-new stores and unsafe rollback targets fail closed. If true direct pre-epoch binary hard-fail cannot be implemented without a destructive Postgres lease-table rewrite, keep the PR partial and document the remaining direct-old-binary gap instead of claiming closure.
- [x] **Installer and docs update**: update `install.ps1`, `install.sh`, README/Copilot skill docs as needed to describe the versioned layout, launcher behavior, rollback/gc commands, and partial closure boundary.
- [x] **Validation and PR hygiene**: add targeted unit/process tests for CLI parsing, launcher dispatch, layout switching/rollback/gc, Copilot GC, and schema gates; run focused validation; commit with selective staging and create the PR with `Refs #6` if the full node anchor remains incomplete.

## Key Decisions

- Reuse the normal Rust binary as the stable launcher instead of adding a second launcher crate/artifact. The launcher path is entered only when the executable is installed at `<root>/bin/telex(.exe)` and a recursion guard is absent.
- Default upgrade/rollback ordering is drain current daemon, atomically switch `<root>/current`, then verify the selected binary. This is safer than allowing the old daemon to continue writing after a pointer flip. Drain waits are bounded; a failed drain aborts the switch unless the user explicitly asks for a non-default force mode that still preserves compatibility/current-version protections.
- Rollback is compatibility-gated and forward-safe: refuse targets without enough manifest/protocol/schema compatibility data rather than blindly switching to an unsafe old binary. Version manifests should include at least tag, package version, binary path, installed timestamp, install source, supported schema range, protocol major/minor, required capabilities, Copilot bridge protocol, minimum compatible plugin version, and previous-current tag.
- GC is conservative: preserve current, previous, current/running executable versions, and anything whose liveness cannot be determined. `--force` may remove non-current stale files but must not override current/previous/running protections.
- Copilot bridge cleanup is command-layer scope (`telex copilot gc` and attach/detach/session-end cleanup), not daemon-core scope.
- Unless the implementation proves all issue-wide acceptance including release-discovery and remaining backend downgrade/transfer semantics, the PR should use `Refs #6` and mark the node partial.

## Success Criteria

- A stable PATH executable in `<root>/bin` dispatches to the selected version under `<root>/versions/<tag>` and propagates child exit status with inherited stdin/stdout/stderr.
- Existing flat installs can be moved into or superseded by the versioned layout without overwriting a running locked binary.
- `telex upgrade` installs/switches from a local binary or extracted release payload, drains the daemon before switching, writes an auditable manifest/history, and leaves existing old-version processes undisturbed.
- `telex rollback` switches only to an installed target that passes manifest/protocol/schema compatibility checks; unsafe rollback is refused with an operator-readable reason.
- `telex gc` reports and removes only safe stale versions; it never deletes current, previous, or in-use binaries and defaults to conservative behavior when liveness is uncertain.
- `telex version` and `telex status` expose enough install, binary path, daemon protocol/version, schema, and bridge compatibility metadata to diagnose mixed-version state.
- `telex copilot gc` reports/removes stale bridge registry/binding/extension artifacts conservatively without daemon-core Copilot coupling.
- SQLite's existing direct-old-binary store barrier remains covered; Postgres gains daemon-aware schema-version fail-closed checks, and any remaining direct pre-epoch binary gap is documented as a reason for `Refs #6`.
- Focused tests cover launcher/layout behavior, upgrade/rollback/gc safety, Copilot GC, command parsing/output shape, and schema compatibility checks.

## Open Questions

None requiring user or orchestrator decision before implementation. The council's open variables are carried as implementation constraints above, and the PR linkage is predetermined as `Refs #6` if the final diff remains a local upgrade floor rather than the full issue #6 closure.
