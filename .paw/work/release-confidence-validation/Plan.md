# Plan - Release Confidence Validation

## Approach Summary

This is a validation/reporting node, not a feature change. The deliverable is a concise
validation report (posted to issue #78, feeding the `hardening-gate`) backed by evidence
from the real shipped paths on Windows. Scope is flagged lightweight: exercise the
practical, high-value release-confidence paths using the existing gating tests (which are
the design's release-confidence gates) plus practical CLI smoke, attempt the real
Copilot bridge push and the configured Postgres/Entra backend, and file/accept any gaps.

Environment already confirmed:
- Build: `cargo build` (default features sqlite+postgres+self-update) succeeds; `telex
  version --json` contract surface intact (bridge_protocol 1, daemon protocol 1.3).
- Postgres/Entra: backend `pg-rde-telex` configured in `~/.telex/config.toml` (Entra CLI
  auth); `az account show` authenticated to the R&D Test subscription -> real PG smoke
  feasible (requires `entra` feature build).
- This is a Copilot CLI session, so a real bridge push smoke is feasible.

Execution is single-threaded (all steps run locally, mostly test/CLI invocations), so
implement directly without fleet dispatch.

## Work Items

- [x] `build-version` - Confirm build + capture `telex version --json` install/version
  surface as evidence; build the `entra` variant needed for the Postgres smoke.
- [x] `release-upgrade` - Release install/upgrade smoke: run `release_contract` and
  `release_upgrade` gating tests; practical `upgrade --from` against a local built asset,
  confirm versioned install layout, `version --json` tag metadata, and conservative
  `rollback`/`gc` behavior.
- [x] `daemon-durability` - Daemon core + durability + idle-drain + liveness gates: run
  `daemon_core_sqlite`, `daemon_process_sqlite`, `copilot_plugin`, `conformance` suites and
  the bridge `busy-state.test.mjs`. Map results to acceptance: #65/#66 no-repro, no-loss on
  restart, no duplicate-storm, false-deaf/self-stop.
- [x] `copilot-bridge` - Real Copilot bridge push smoke (bounded): exercise attach +
  extensions_reload + push + ack/disposition + detach/stop-delivery in a real session if
  feasible; otherwise document evidence from `copilot_plugin` + bridge unit tests. Confirm
  detach/mute sticks.
- [x] `postgres-smoke` - Postgres/Entra real-use smoke: run `daemon_core_postgres` against
  `pg-rde-telex` (entra build + Entra auth); compare lease/reclaim/push/disposition to the
  SQLite smoke. If backend unreachable, document as not-run with rationale (residual risk).
- [x] `report` - Write durable validation report at
  `docs/notes/release-confidence-validation.md` (scenarios, commands, results, failures,
  fixes/issues filed, residual risk); post a summary comment to issue #78; file gap issues
  for anything not fixed in-node.

## Key Decisions

- Report home: committed `docs/notes/release-confidence-validation.md` (referenceable by
  the hardening gate) + a posted summary comment on issue #78. Both, per acceptance.
- Treat the existing gating test suites as the primary release-confidence evidence; they
  are the design's normative gates for daemon lifecycle, delivery, idle-drain, liveness,
  and the Copilot plugin. Supplement with practical CLI/real-session smoke, not replace.
- Do not build new validation harnesses or the AKS scale rig (explicit boundary).
- File issues (not in-node fixes) for gaps unless a fix is trivial and clearly in-scope.

## Open Questions

None.
