# WorkflowContext

Work Title: Telex TUI Inspector
Work ID: telex-tui-inspector
Base Branch: main
Target Branch: feature/telex-tui-inspector
Execution Mode: worktree
Repository Identity: github.com/lossyrob/telex@c8f0041e7a632c46552f18ff0f8de0535bb9123a
Execution Binding: worktree:telex-tui-inspector:feature/telex-tui-inspector
Workflow Mode: custom
Review Strategy: local
Review Policy: final-pr-only
Session Policy: continuous
Final Agent Review: enabled
Final Review Mode: multi-model
Final Review Interactive: smart
Final Review Models: gpt-5.5, claude-opus-4.8
Final Review Specialists: all
Final Review Interaction Mode: parallel
Final Review Specialist Models: none
Final Review Perspectives: auto
Final Review Perspective Cap: 2
Implementation Model: none
Plan Generation Mode: single-model
Plan Generation Models: claude-opus-4.8
Planning Docs Review: enabled
Planning Review Mode: single-model
Planning Review Interactive: smart
Planning Review Models: gpt-5.5
Planning Review Specialists: all
Planning Review Interaction Mode: parallel
Planning Review Specialist Models: none
Planning Review Perspectives: auto
Planning Review Perspective Cap: 2
Custom Workflow Instructions: PAW Lite flow for a read-only, live-tail Telex message inspector TUI. Stages: work-shaping (interactive) -> single-model plan (session model, Opus 4.8) -> planning docs review (single-model, gpt-5.5) -> fleet implement -> final review (multi-model, gpt-5.5 + claude-opus-4.8) -> final PR. Build the TUI as a new in-repo Cargo workspace member crate (e.g. telex-tui / telex-top) that is separately installable and reuses the core `telex` library crate in-process via a path dependency; the core `telex` binary and its dependency graph must stay lean (no ratatui/crossterm in core). Implementation language is Rust (ratatui + crossterm). Read-only: no disposition/send/mutation from the TUI. Live tail via the existing fetch_after cursor poll (no blocking telex wait). Support both SQLite and Postgres backends through the Backend trait. Likely a small core-lib refactor to extract a backend-open/read-query helper so the TUI does not reimplement profile->backend wiring. Views per agreed mockups: global live Feed (home), Addresses (Miller-column drill-down), and Thread transcript, with a shared detail pane.
Initial Prompt: Build a UX to inspect telex messages: a read-only, live-tail terminal UI (TUI) for watching telex messages, addresses, and threads. Chosen approach: in-repo Rust ratatui TUI as a separate, separately-installable workspace member crate that reuses the core telex lib in-process, keeping the core agent binary lean.
Issue URL: none
Remote: origin
Artifact Lifecycle: commit-and-clean
Artifact Paths: auto-derived
Additional Inputs: none
