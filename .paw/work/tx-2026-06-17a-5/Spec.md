# Spec — Issue #5: Enforce session-binding in the holder

## Problem
The resident holder (`telex attach`) keeps an address's lease alive by heartbeating across agent
turns (Decision 0004/0005). If the holder is mis-launched as a **detached/persistent** process
(e.g. Copilot CLI `detach: true`, or any "daemonize" path), it **outlives its session**, keeps
heartbeating, and the address falsely reports `occupied`/live forever. Today this is prevented
only by convention in SKILL.md. The failure is not self-correcting: the orphaned holder is
exactly the thing that would otherwise let the TTL lapse.

## Goal
Add **defense-in-depth inside the binary**: the holder watches a signal representing "my session
is alive," and when that signal disappears it **releases the lease and exits through the normal
shutdown path** — so even a mis-launched detached holder cannot outlive its session.

## Outcome anchor (issue acceptance criteria)
1. `telex attach --session-pid <pid>`: killing `<pid>` causes the holder to release the lease and
   exit **within the liveness window**; the address shows free afterward.
2. A holder launched detached (e.g. `detach: true`) but given `--session-pid` still
   self-terminates when the session ends.
3. `--no-session-bind` runs a persistent holder that survives the launcher (documented escape
   hatch).
4. Release on session-death is **identical** to `detach`/ctrl-c (lease released, IPC endpoint
   cleaned up).
5. Cross-platform: pid-watch works on Windows and Unix; the fd/pipe path (if implemented) works
   on both or is clearly gated.
6. `SKILL.md` updated to mention `--session-pid` as the belt-and-suspenders companion to
   launching background + session-bound.

## User stories
- **As an agent runtime that knows its durable session pid**, I pass `--session-pid <pid>` (or
  export `TELEX_SESSION_PID`) so that if my session dies — even abruptly, even if the holder was
  accidentally detached — the lease releases promptly and the address frees.
- **As an operator running a deliberately persistent, server-side holder** (an address served as
  a long-running daemon, not tied to any agent session), I pass `--no-session-bind` so the holder
  is never bound to a launcher pid, even if `TELEX_SESSION_PID` happens to be set in the
  environment.

## Functional requirements
- **FR1** New holder flag `--session-pid <PID>` reading env `TELEX_SESSION_PID`. When set
  (and not overridden), the holder polls that pid and, on its death, runs the normal
  shutdown→`release_lease` path and exits.
- **FR2** New holder flag `--no-session-bind`: disables session binding even if
  `TELEX_SESSION_PID` is present in the environment. Conflicts with `--session-pid`.
- **FR3** New holder flag `--session-poll-secs <N>` (default 2): cadence of the liveness check.
  Detection latency must be comfortably inside the default 15s liveness window.
- **FR4** Liveness check is cross-platform: `kill(pid,0)` on Unix; `OpenProcess` +
  `WaitForSingleObject` on Windows. "Process exists but access-denied" counts as **alive**;
  "no such process" counts as **dead**.
- **FR5** On detected session death, the holder logs a clear line
  (`[holder] session <pid> gone; releasing lease and exiting`) and releases via the exact
  same code path as `detach`/ctrl-c.
- **FR6** When binding is active, the holder logs at startup which pid it is bound to.
- **FR7** Default behavior (no flag, no env) is **unchanged** (no binding) — fully backward
  compatible.

## Non-functional requirements
- No behavior/lease/messaging change when the new flags are unused.
- New platform dependencies must already exist in the lock tree (no tree growth): `libc` (unix),
  `windows-sys` (windows).
- Feature-gated builds (`--no-default-features --features sqlite` / `--features postgres`) must
  still compile (the new code is backend-agnostic).

## Out of scope (declined / deferred — see ImplementationPlan + DECISIONS 0010)
- **ppid-default fallback** (proposal item 3): DECLINED as a *default* — spawn-and-return
  launchers would make the holder wrongly self-exit. Offered neither as default nor opt-in in
  this PR; the explicit `--session-pid`/env is the sanctioned binding. (High-spread; flagged
  for human review.)
- **`--session-fd` inherited-pipe path** (proposal item 2): DEFERRED — explicitly optional in
  the issue; pid-watch satisfies every acceptance criterion. Recorded as carry-forward.
- OS-level launcher guarantees (Job Objects, process groups/cgroups): out of scope — they
  require launcher cooperation and cannot be self-enforced by telex (the issue says so).

## Success criteria / validation
- Unit tests for the liveness check (self alive; never-allocated pid dead; spawned-then-reaped
  child dead).
- Manual Windows end-to-end: a dummy process + `telex attach --session-pid <dummy>` on a temp
  backend; killing the dummy releases the lease and exits the holder; `telex status` shows
  `occupied=false`.
- `cargo build`, `cargo test`, `cargo clippy --all-targets`, plus both feature-gated builds, all
  green.
