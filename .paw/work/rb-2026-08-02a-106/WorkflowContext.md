# WorkflowContext

Work Title: Station Intent Reconciliation
Work ID: rb-2026-08-02a-106
Workflow Identity: paw-lite
Base Branch: main
Target Branch: feature/station-intent-reconciliation-106
Execution Mode: worktree
Repository Identity: github.com-lossyrob/lossyrob/telex@c8f0041e7a632c46552f18ff0f8de0535bb9123a
Execution Binding: worktree:rb-2026-08-02a-106:feature/station-intent-reconciliation-106
Workflow Mode: custom
Review Strategy: local
Review Policy: final-pr-only
Session Policy: continuous
Final Agent Review: enabled
Final Review Mode: society-of-thought
Final Review Interactive: false
Final Review Models: none
Final Review Specialists: adaptive full-depth plus rubber-duck
Final Review Interaction Mode: parallel
Final Review Specialist Models: claude-opus-5
Final Review Perspectives: auto
Final Review Perspective Cap: 2
Implementation Model: none
Plan Generation Mode: single-model
Plan Generation Models: none
Planning Docs Review: enabled
Planning Review Mode: society-of-thought
Planning Review Interactive: false
Planning Review Models: none
Planning Review Specialists: adaptive full-depth
Planning Review Interaction Mode: parallel
Planning Review Specialist Models: claude-opus-5
Planning Review Perspectives: auto
Planning Review Perspective Cap: 2
Custom Workflow Instructions: Use high reasoning for all Society-of-Thought specialists. Work shaping is enabled for architectural ambiguity. Use the council skill for gated high-stakes decisions and never use spar. Care knob is fail-toward-surfacing; posture is craft.
Initial Prompt: Implement GitHub issue #106 completely, including daemon lifecycle, durable versioned station-intent reconciliation, Copilot bridge behavior, recovery semantics, diagnostics, compatibility and security constraints, documentation, and SQLite/Postgres/process tests.
Issue URL: https://github.com/lossyrob/telex/issues/106
Remote: origin
Artifact Lifecycle: commit-and-clean
Artifact Paths: auto-derived
Additional Inputs: Workstream rb-2026-08-02a; operating tier L; final PR title prefix [rb-2026-08-02a-106].

## Control State

TODO Mirror: active-required-items
Reconciliation: current

### Required Workflow Items
- `init` | `resolved` | `activity`
- `planning` | `resolved` | `activity`
- `planning-docs-review` | `pending` | `activity`
- `implementation` | `pending` | `activity`
- `final-review` | `pending` | `activity`
- `final-pr` | `pending` | `activity`

### Configured Procedure Items
- `procedure:planning-review` | `pending` | `procedure`
- `procedure:final-review` | `pending` | `procedure`
