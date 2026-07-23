# WorkflowContext

Work Title: Preserve Copilot Bridge State
Work ID: preserve-copilot-bridge-state-119
Base Branch: main
Target Branch: feature/preserve-copilot-bridge-state-119
Execution Mode: worktree
Repository Identity: github.com/lossyrob/telex@c8f0041e7a632c46552f18ff0f8de0535bb9123a
Execution Binding: worktree:preserve-copilot-bridge-state-119:feature/preserve-copilot-bridge-state-119
Workflow Mode: custom
Review Strategy: local
Review Policy: final-pr-only
Session Policy: continuous
Final Agent Review: enabled
Final Review Mode: multi-model
Final Review Interactive: false
Final Review Models: gpt-5.6-sol, claude-opus-4.8
Final Review Specialists: all
Final Review Interaction Mode: parallel
Final Review Specialist Models: none
Final Review Perspectives: auto
Final Review Perspective Cap: 2
Implementation Model: none
Plan Generation Mode: single-model
Plan Generation Models: gpt-5.6-sol
Planning Docs Review: enabled
Planning Review Mode: multi-model
Planning Review Interactive: false
Planning Review Models: gpt-5.6-sol, claude-opus-4.8
Planning Review Specialists: all
Planning Review Interaction Mode: parallel
Planning Review Specialist Models: none
Planning Review Perspectives: auto
Planning Review Perspective Cap: 2
Custom Workflow Instructions: Use PAW Lite. Planning review must run gpt-5.6-sol at xhigh effort and claude-opus-4.8 at xhigh effort. Final review must run gpt-5.6-sol and claude-opus-4.8 at max effort. Continue autonomously through PR creation unless upfront work shaping or a design decision is required. Commit all implementation work and remove commit-and-clean PAW artifacts before the final PR.
Initial Prompt: Work on https://github.com/lossyrob/telex/issues/119. Use paw-lite in a worktree. planning review multimodel gpt 5.6 sol xhigh and opus 4.8 xhigh. final review multimodel gpt 5.6 sol and opus 4.8 max. commit and clean. don't stop until PR, unless there's any work shaping or design decisions up front.
Issue URL: https://github.com/lossyrob/telex/issues/119
Remote: origin
Artifact Lifecycle: commit-and-clean
Artifact Paths: auto-derived
Additional Inputs: none
