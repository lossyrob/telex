# WorkflowContext

Work Title: Detect Plugin Binary Skew
Work ID: rb79-20260718-79
Workflow Identity: paw-lite
Base Branch: main
Target Branch: feature/rb79-20260718-79
Execution Mode: worktree
Repository Identity: github.com-lossyrob/lossyrob/telex@c8f0041e7a632c46552f18ff0f8de0535bb9123a
Execution Binding: worktree:rb79-20260718-79:feature/rb79-20260718-79
Workflow Mode: custom
Review Strategy: local
Review Policy: final-pr-only
Session Policy: continuous
Final Agent Review: enabled
Final Review Mode: society-of-thought
Final Review Interactive: false
Final Review Models: gpt-5.6-sol
Final Review Specialists: all
Final Review Interaction Mode: parallel
Final Review Specialist Models: gpt-5.6-sol
Final Review Perspectives: auto
Final Review Perspective Cap: 2
Implementation Model: gpt-5.6-sol
Plan Generation Mode: single-model
Plan Generation Models: gpt-5.6-sol
Planning Docs Review: disabled
Planning Review Mode: single-model
Planning Review Interactive: false
Planning Review Models: gpt-5.6-sol
Planning Review Specialists: all
Planning Review Interaction Mode: parallel
Planning Review Specialist Models: gpt-5.6-sol
Planning Review Perspectives: auto
Planning Review Perspective Cap: 2
Custom Workflow Instructions: Use GPT-5.6 Sol with long context and xhigh reasoning throughout. Preserve binary-owned plugin compatibility and the versioned install/upgrade model. Correctness and compatibility take priority.
Initial Prompt: Solve issue #79 by making plugin/binary skew detectable and stale-binary hook failure visible and actionable without introducing a silent failure mode.
Issue URL: https://github.com/lossyrob/telex/issues/79
Remote: origin
Artifact Lifecycle: commit-and-clean
Artifact Paths: auto-derived
Additional Inputs: Workstream rb79-20260718; Telex backend local

## Control State

TODO Mirror: active-required-items
Reconciliation: current

### Required Workflow Items
- `init` | `resolved` | `activity`
- `planning` | `resolved` | `activity`
- `planning-docs-review` | `not_applicable` | `activity`
- `implementation` | `resolved` | `activity`
- `final-review` | `resolved` | `activity`
- `final-pr` | `pending` | `activity`

### Configured Procedure Items
- `procedure:planning-review` | `not_applicable` | `procedure`
- `procedure:final-review` | `resolved` | `procedure`
