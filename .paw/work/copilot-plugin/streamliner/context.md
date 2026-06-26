# Launch Context - Copilot plugin: sessionEnd hook + skill

## Layer 0 - Design Context Hints

Use these manifest-provided design hints as navigation points, not as instructions to obey blindly. Treat `docs/design/daemon.md` as the normative contract when it differs from older graph or brief wording.

- `docs/design/index.md` - design entry point and summary of the resolved daemon/local-exchange design.
- `docs/design/daemon.md` - normative daemon contract. Most relevant anchors for this node: section 9 liveness model, section 14.2 sessionEnd hook, section 15 single-source skill, and section 17 gating tests around non-destructive reaping and re-attach.
- `docs/design/DESIGN.md` - local-exchange architecture context.
- `docs/design/DECISIONS.md` - ADR log; ADRs 0014-0024 capture the local-daemon decisions.
- `docs/design/ARCHITECTURE.md` - visual/system overview.

## Layer 1 - Worker Mission

Selected node: `copilot-plugin` from workstream `local-daemon`.

Node title: Copilot plugin: sessionEnd hook + skill.

Tracker: `lossyrob/telex#41` / https://github.com/lossyrob/telex/issues/41. The issue body was unavailable from this SDK session, so use the manifest and graph node text as the selected-node spec unless GitHub becomes accessible later.

Node responsibility: implement the Copilot plugin surface for the local daemon workstream, specifically the sessionEnd hook integration and real Copilot plugin skill cutover. The node outcome anchor is: the Copilot plugin supplies the healthy-disconnect path and single-source skill integration needed for the SQLite operator-unblock slice after daemon-core/fencing-proof.

Required outcome details from the selected node and design references:

- Provide the Copilot sessionEnd hook integration for healthy disconnect on quit and, if supported by the harness, dismiss.
- Keep the hook as a thin mapper from Copilot harness inputs onto daemon/core-generic inputs. The intended mapping is `COPILOT_AGENT_SESSION_ID -> TELEX_SESSION_ID`; `COPILOT_LOADER_PID -> --watch-pid` for the loader negative signal.
- Do not make telex core depend on `COPILOT_*` names or Copilot JSON payload parsing. Copilot-specific parsing/mapping belongs at the plugin/harness boundary.
- Do not revive PR #31's filesystem `session_registry` as the authority. Membership is daemon-native, in-memory, and explicit-only.
- Preserve the normative daemon design: `daemon.md` says sessionEnd is authoritative but non-destructive, releasing blocked waiters and marking stations idle; it must not destroy stations or cause data loss. If graph wording appears to imply destructive Detach/removal, resolve that tension explicitly during planning against `daemon.md` rather than silently changing the node outcome.
- Move `telex skill` into a real Copilot plugin skill while keeping one source of truth. `daemon.md` section 15.2 names root `SKILL.md` as canonical, with `telex skill` printing embedded `SKILL.md` and a raw/machine-consumable path for the plugin skill. Avoid generated divergent copies.
- Include validation/proof that covers the plugin mapping, sessionEnd hook behavior, and single-source skill behavior. Planning should preserve the node outcome anchor and not replace this node with prerequisite-only work.

Out of scope for this node unless the builder explicitly expands scope:

- Implementing Postgres parity (`postgres-parity` / #42).
- Implementing full seamless upgrade (`seamless-upgrade` / #6).
- Building the validation harness / AKS scale rig.
- Replacing or renegotiating daemon-core contracts except where this plugin node finds a direct mismatch that must be escalated.
- Creating/mutating shared workstream state such as new issues, labels, or graph edits. Record recommendations in the field report instead.

## Layer 2 - Relevant State

Repository: `telex` at `C:\Users\robemanuele\proj\telex\telex`; origin is `git@github.com-lossyrob:lossyrob/telex.git`.

Launch cwd: `C:\Users\robemanuele\proj\telex\telex` on `main`. Treat this as the base/coordination checkout. PAW execution should use a sibling worktree for the node branch.

Current graph state:

- `design-foundation`: completed. It produced/merged the authoritative local-daemon design under `docs/design/`.
- `design-gate`: completed.
- `daemon-core`: completed. Its summary says the per-user daemon and SQLite one-shot verbs are implemented/proven enough that the plugin is now the first operator-unblocking slice.
- `fencing-proof`: completed. This node depends on it and may proceed.
- `copilot-plugin`: ready/watch, selected task.
- `postgres-parity`: ready/watch sibling/downstream-adjacent task, not assigned to this worker.
- `seamless-upgrade`: planned downstream and depends on both `postgres-parity` and `copilot-plugin`.

Workstream context:

- Purpose: eliminate the per-session resident holder and move presence/delivery into an auto-spawned per-user local daemon so idle long-lived sessions remain wakeable and stations stop going stale.
- First operator-unblocking slice: daemon-core on SQLite plus this Copilot plugin. With the plugin landed, holder/waiter races, orphaned holders, and turn-loop starvation should be removed for the SQLite path.
- Important import: PR #31 / issue #23 hook plumbing may be reusable, but its filesystem session registry is explicitly not the attendance authority.
- Important import: the harness env contract is `COPILOT_AGENT_SESSION_ID` and `COPILOT_LOADER_PID`; telex core should expose generic `TELEX_SESSION_ID` / `--watch-pid` semantics.

Planning/implementation guardrails:

- Before planning, identify the node completion condition above and keep it as the node outcome anchor.
- Use the `council` skill only when its gates indicate a consequential uncertain decision; the plan before planning review is the canonical case. Keep deliberation contained and read back only synthesis/minority report/reopen conditions.
- Planning docs review is enabled and uses SoT with the ad hoc `general-reviewer` persona; do not substitute the built-in `all` roster just because `general-reviewer` is ad hoc.
- If accepted review feedback would make the original node outcome infeasible, stop and propose an issue amendment/split instead of landing outcome-replacement work.
- If the PR is only prerequisite/partial/blocker documentation, use `Refs #41` rather than `Closes #41` and mark the state clearly.

Implementation/lifecycle expectations:

- Use paw-lite.
- Create lifecycle TODOs after reading the PAW PR lifecycle skill and implementer guide.
- Final PR title must start with `[local-daemon]` and include both workstream id and issue number, ending with `(#41)`.
- PR body must include a collapsible top `<details>` section with `<summary>Docs.md</summary>` and a completed Docs.md following `paw-docs-guidance`.
- After PR creation, immediately enter PAW PR lifecycle Review Response mode and run the canonical Windows review-response checker loop for repo `lossyrob/telex`, PR number, and user `robemanuele_microsoft`.
- Keep field notes during execution and synthesize them into a final issue comment field report after merge.

## Layer 3 - Coordination Context

Upstream dependencies are completed in the graph: `daemon-core` and `fencing-proof`. This worker should not redo those nodes, but should rely on their public daemon/IPC surfaces and tests.

Sibling/downstream coordination:

- `postgres-parity` runs from the same fencing proof and is a sibling/parallel path. Do not expand this plugin PR into Postgres parity unless directed by the builder.
- `seamless-upgrade` depends on this node and Postgres parity. Avoid introducing plugin install/manifest assumptions that would block the later versioned install and launcher shim work; record any upgrade-facing constraints in the field report.
- `validation-harness` and scale nodes will later consume plugin behavior as part of the full hardening wave. Leave observability/testing notes for them in the field report if implementation discovers useful hooks or gaps.

Branch/worktree policy:

- Base branch: `main`.
- Workstream ID: `local-daemon`.
- Suggested work ID: `copilot-plugin`.
- Suggested target branch: `feature/copilot-plugin` unless PAW init derives/reuses another valid branch.
- Work in a sibling worktree; do not check out the target branch in launch cwd.
- Make sure the local source branch is updated from remote before creating/reusing the worktree.

Unavailable Inputs

- `telex:PRODUCT-THESIS.md` and `telex:SKILL.md` appeared in source references but were marked invalid as design-doc paths by the manifest. Do not retry them as design hints for launch context. `SKILL.md` may still be relevant during implementation because `daemon.md` section 15.2 names it as the canonical skill source.
- `.github/copilot-instructions.md` is missing in the selected repo.
- GitHub issue `lossyrob/telex#41` was unavailable to both the Streamliner manifest generator and this SDK session. If access becomes available later, compare it to the graph node and preserve the same outcome anchor unless the issue explicitly amends it.
