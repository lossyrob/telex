# WorkflowContext

Work Title: Agent Message Subjects
Work ID: mac-2026-07-09a-98
Base Branch: main
Target Branch: mac-2026-07-09a-98-agent-subjects
Execution Mode: worktree
Repository Identity: github.com/lossyrob/telex@c8f0041e7a632c46552f18ff0f8de0535bb9123a
Execution Binding: worktree:mac-2026-07-09a-98:mac-2026-07-09a-98-agent-subjects
Workflow Mode: custom
Review Strategy: local
Review Policy: final-pr-only
Session Policy: continuous
Final Agent Review: enabled
Final Review Mode: single-model
Final Review Interactive: false
Final Review Models: claude-opus-4.8
Final Review Specialists: all
Final Review Interaction Mode: parallel
Final Review Specialist Models: none
Final Review Perspectives: auto
Final Review Perspective Cap: 2
Implementation Model: none
Plan Generation Mode: single-model
Plan Generation Models: claude-opus-4.8
Planning Docs Review: disabled
Planning Review Mode: single-model
Planning Review Interactive: false
Planning Review Models: claude-opus-4.8
Planning Review Specialists: all
Planning Review Interaction Mode: parallel
Planning Review Specialist Models: none
Planning Review Perspectives: none
Planning Review Perspective Cap: 2
Custom Workflow Instructions: Treat issue #98 as the specification. Do not create Spec.md, ImplementationPlan.md, CodeResearch.md, or Plan.md. Implement the acceptance criteria directly, run a claude-opus-4.8 final review on the actual diff, and follow the PR lifecycle from the orchestrator prompt.
Initial Prompt: Update binary-owned agent guidance so operational sends always include concise human-readable subjects, replies repair blank thread subjects, examples include --subject, and tests prevent regression.
Issue URL: https://github.com/lossyrob/telex/issues/98
Remote: origin
Artifact Lifecycle: commit-and-clean
Artifact Paths: auto-derived
Additional Inputs: none
