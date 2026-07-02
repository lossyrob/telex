# WorkflowContext

Work Title: Seamless Upgrade
Work ID: seamless-upgrade
Workflow Identity: paw-lite
Base Branch: main
Target Branch: feature/seamless-upgrade
Execution Mode: worktree
Repository Identity: github.com-lossyrob/lossyrob/telex@c8f0041e7a632c46552f18ff0f8de0535bb9123a
Execution Binding: worktree:seamless-upgrade:feature/seamless-upgrade
Workflow Mode: custom
Review Strategy: local
Review Policy: final-pr-only
Session Policy: continuous
Final Agent Review: enabled
Final Review Mode: society-of-thought
Final Review Interactive: false
Final Review Models: gpt-5.5, gemini-3-pro-preview, claude-opus-4.7
Final Review Specialists: general-reviewer
Final Review Interaction Mode: parallel
Final Review Specialist Models: general-reviewer:claude-opus-4.7-high
Final Review Perspectives: premortem, retrospective
Final Review Perspective Cap: 2
Implementation Model: none
Plan Generation Mode: single-model
Plan Generation Models: gpt-5.5, gemini-3-pro-preview, claude-opus-4.7
Planning Docs Review: enabled
Planning Review Mode: society-of-thought
Planning Review Interactive: false
Planning Review Models: gpt-5.5, gemini-3-pro-preview, claude-opus-4.7
Planning Review Specialists: general-reviewer
Planning Review Interaction Mode: parallel
Planning Review Specialist Models: general-reviewer:claude-opus-4.7-high
Planning Review Perspectives: premortem, retrospective
Planning Review Perspective Cap: 2
Custom Workflow Instructions: none
Initial Prompt: none
Issue URL: https://github.com/lossyrob/telex/issues/6
Remote: origin
Artifact Lifecycle: commit-and-clean
Artifact Paths: auto-derived
Additional Inputs: streamliner-context=C:/Users/robemanuele/proj/telex/telex-seamless-upgrade/.paw/work/seamless-upgrade/streamliner/context.md

## Control State

TODO Mirror: active-required-items
Reconciliation: current

### Required Workflow Items
- `init` | `resolved` | `activity`
- `planning` | `resolved` | `activity`
- `planning-docs-review` | `resolved` | `activity`
- `implementation` | `resolved` | `activity`
- `final-review` | `pending` | `activity`
- `final-pr` | `pending` | `activity`

### Configured Procedure Items
- `procedure:planning-review` | `resolved` | `procedure`
- `procedure:final-review` | `pending` | `procedure`
