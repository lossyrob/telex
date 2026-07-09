# Copilot Detached Waiter Fallback Implementation Plan

## Overview

Implement issue #88 by retaining Copilot extension push as the preferred delivery
path and adding a first-class, cross-platform pull fallback built on the existing
single-shot `telex wait --out-dir` contract. Telex will prepare one durable waiter
run at a time, while the Copilot task runner remains responsible for detached
execution and completion wakeups.

The daemon will enforce the harness-neutral invariant that one station cannot use
an on-deliver push handler and a live waiter simultaneously. Copilot-specific
mode selection, launcher generation, and recovery instructions remain in the
Copilot adapter and embedded Copilot skill.

## Current State Analysis

- `telex copilot attach --copilot-bridge` provisions push, but a generic waiter
  can currently arm against the same member (`src/commands/copilot.rs`,
  `src/daemon.rs`).
- A push registration can currently refresh a member that already has a live
  waiter, so push and pull are not mutually exclusive (`src/daemon.rs`).
- `telex wait --out-dir` already provides the required single-shot artifact
  protocol and the daemon already rejects a second live waiter
  (`src/commands/wait.rs`, `src/daemon.rs`).
- Status reports push registration and waiter counts independently, but does not
  expose one derived delivery mode or a conflict state
  (`src/daemon_ipc.rs`, `src/commands/status.rs`,
  `src/commands/station.rs`).
- The embedded Copilot skill documents only a hand-written Windows PowerShell
  wrapper. There is no Telex-owned run preparation, no macOS/Linux-native path,
  and no automated macOS fallback exercise (`copilot/COPILOT.md`,
  `.github/workflows/ci.yml`).

## Desired End State

- A Copilot agent without extension support can prepare a fallback run, launch
  exactly one detached waiter, receive durable artifacts, acknowledge by message
  id, and prepare the next run on macOS or Windows.
- Run preparation is idempotent per `(store, session, address)`: repeated
  preparation returns the same unfinished run rather than minting competing
  artifact directories.
- The daemon rejects both illegal mixed-mode entry points:
  - a waiter while an on-deliver handler is registered;
  - a push registration while a live waiter exists.
- Push-to-pull fallback is performed by the detached fallback runner: it
  atomically clears the member's on-deliver handler, verifies the clear, removes
  the address's bridge binding, and immediately enters one wait.
- Pull-to-push remains an explicit safe sequence: stop the waiter/station, then
  attach push. Push attach rejects a still-live waiter without mutating it.
- Status exposes neutral `delivery_mode` values (`push`, `pull`, `conflict`) and
  separates that configured path from instantaneous station health. The Copilot
  skill describes neutral `pull` mode as the Copilot pull fallback.
- The agent-stop guard surfaces mixed-mode conflict and gives mode-specific
  recovery guidance.
- A targeted process test exercises the generated fallback path on Windows and
  macOS CI.

## Key Decisions

1. **Daemon enforcement, Copilot orchestration.** Exclusivity is expressed only
   in generic on-deliver/waiter terms; the daemon does not learn Copilot bridge
   concepts.
2. **Idempotent prepare plus hidden run.** `telex copilot fallback prepare`
   creates or returns one current run manifest. A hidden
   `telex copilot fallback run` performs the transition and one wait only after
   the detached task actually starts, so a failed task launch leaves push intact.
3. **Additive protocol evolution.** Add an optional
   `Register.replace_on_deliver` field, bump protocol minor version to 4, and
   fail closed before fallback transition when the running daemon is older.
   Old daemons ignore the additive field; the runner also verifies that push was
   actually cleared before waiting.
4. **Asymmetric transitions.** Push-to-pull earns an atomic in-place member
   refresh because fallback launch may fail. Pull-to-push uses explicit
   `station stop` then push attach because a temporary uncovered state is safe
   and messages remain durable.
5. **Windows-only generated wrapper.** Windows receives a Telex-owned `.ps1`
   launcher for the known detached bare-executable issue. Unix returns direct
   executable argv; no unnecessary shell script is generated.
6. **Two status axes.** `delivery_mode` is derived from registered mechanisms
   and remains `pull` between waiter runs. `station_health` continues to report
   whether that mode is currently covered, recently delivered, unattended, or
   conflicting.

## What We're NOT Doing

- No generic `mode push|pull` state machine or persisted preferred-mode bit.
- No daemon awareness of Copilot bridge files, heartbeats, extensions, or task
  runner semantics.
- No daemon-owned background waiter, infinite polling loop, or automatic waiter
  re-arm.
- No waiter supersession or process killing; duplicates remain fail-closed.
- No generated Unix shell wrapper.
- No changes to the harness-neutral root `SKILL.md` or its assignment preamble.
- No automatic pull-to-push drain hidden inside generic registration.
- No requirement that GitHub's PR review decision be non-empty for a
  same-owner review.

## Phase Status

- [x] **Phase 1: Enforce delivery exclusivity and expose mode** - Add the protocol, daemon gates, status model, and conflict guard.
- [x] **Phase 2: Add the Copilot fallback run lifecycle** - Implement idempotent preparation, platform launch metadata, atomic push-to-pull transition, and one-shot execution.
- [x] **Phase 3: Exercise fallback cross-platform** - Add end-to-end process coverage and macOS/Windows CI execution.
- [ ] **Phase 4: Documentation** - Update the binary-owned Copilot workflow, operator guide, ADR, and as-built Docs.md.

## Phase Candidates

- [x] Daemon-side push/waiter exclusivity
- [x] Idempotent Copilot fallback run manifests
- [x] Windows-only generated PowerShell launcher
- [x] Neutral delivery-mode status
- [x] Targeted macOS and Windows CI exercise

---

## Phase 1: Enforce Delivery Exclusivity and Expose Mode

### Changes Required

- **`src/daemon_ipc.rs`**:
  - bump protocol minor version to 4;
  - add optional `Register.replace_on_deliver`;
  - add forward-compatible `DeliveryMode` values for `push`, `pull`, and
    `conflict`;
  - add `delivery_mode` to `MemberStatus`;
  - add a `coverage_conflict` station-health value.
- **`src/commands/attach.rs`**:
  - preserve existing public attach behavior;
  - expose an internal registration path that can explicitly replace/clear an
    on-deliver handler for the Copilot fallback runner.
- **`src/daemon.rs`**:
  - reject `Wait` while the member has an on-deliver handler;
  - reject registration of an on-deliver handler while a live waiter exists,
    without mutating the current member;
  - honor explicit handler replacement while preserving ordinary
    re-registration's current push-preservation behavior;
  - derive delivery mode and conflict health in status.
- **`src/commands/status.rs` and `src/commands/station.rs`**:
  expose `delivery_mode` in JSON and text output.
- **`src/commands/copilot.rs`**:
  preflight push attach so a live waiter produces actionable
  `station stop` guidance before bridge provisioning; classify conflict in the
  turn guard without hiding it behind push health.
- **Tests**:
  extend daemon, IPC, CLI parsing, status, and guard unit tests for both
  exclusivity gates, replacement compatibility, mode derivation, and conflict
  reporting.

### Success Criteria

#### Automated Verification

- [x] A waiter against a push member returns a terminal non-delivery outcome and
  leaves push registered.
- [x] Push registration against a live waiter fails without stopping that
  waiter.
- [x] Explicit replacement clears push while an ordinary refresh preserves it.
- [x] Old-wire JSON without the new field deserializes with existing behavior.
- [x] Status serializes `push`, `pull`, and the defensive `conflict` tripwire.
- [x] Library tests pass: `cargo test --lib`.

#### Manual Verification

- [x] Text and JSON status distinguish delivery mode from station health.
- [x] Push attach with a live waiter tells the operator to stop the station and
  retry, without leaving bridge files behind.

---

## Phase 2: Add the Copilot Fallback Run Lifecycle

### Changes Required

- **`src/cli.rs`**:
  add `copilot fallback prepare` and hidden `copilot fallback run` arguments.
  Prepare accepts Copilot session mapping, optional station metadata, wait
  timeout/attention/CC options, and an explicit force switch for leaving a live
  push bridge.
- **`src/commands/copilot.rs`**:
  - maintain owner-private fallback state under the Telex home, keyed by safe
    session/address tokens;
  - serialize a versioned run manifest and canonical current-run pointer under a
    short-lived state lock;
  - return an unfinished current run on repeated preparation and create a fresh
    unique run only after the prior run has a terminal `exit.code`;
  - claim one execution of a run before touching delivery state so duplicate
    launcher starts cannot race writes into the same artifact directory;
  - return direct executable argv on Unix and generate a parameter-free
    PowerShell launcher on Windows using only the exact executable and generated
    run path;
  - have hidden run validate the current manifest and protocol minor version,
    refuse a live bridge unless force was explicit, clear and verify push
    registration, remove the address's bridge binding, then execute one existing
    wait with the manifest's run directory;
  - preserve station occupant/description/scope/tags when transitioning an
    existing member and explicitly attach a missing member in pull mode.
- **`src/commands/wait.rs`**:
  expose a narrow internal helper that writes standard terminal error artifacts
  when fallback setup fails before the waiter starts.
- **Tests**:
  cover manifest round trips, idempotent prepare, unique completed runs,
  stale/duplicate run claims, Unix argv, Windows launcher quoting, protocol
  skew, live-push force policy, metadata preservation, and pre-wait error
  artifacts.

### Success Criteria

#### Automated Verification

- [x] Two prepares before completion return one run directory and launcher.
- [x] A duplicate launcher cannot overwrite a live run's artifacts.
- [x] A launcher that never starts leaves push unchanged.
- [x] A running fallback clears push before arming its waiter and refuses to
  wait if the clear cannot be proved.
- [x] Startup failures still produce `status.json` and terminal `exit.code`.
- [x] Library tests pass: `cargo test --lib`.

#### Manual Verification

- [x] Prepare JSON contains an exact run directory, launcher program/argv, and
  expected artifact paths without requiring an agent-authored script.
- [x] The live-push refusal and `--force` path are explicit and observable.

---

## Phase 3: Exercise Fallback Cross-Platform

### Changes Required

- **`tests/daemon_process_sqlite.rs`**:
  add a focused process-level scenario that:
  - establishes a Copilot station;
  - prepares and launches the platform-specific fallback command;
  - observes a live waiter and `delivery_mode=pull`;
  - sends a message, reads the exact run's delivery artifacts, and acknowledges
    it;
  - prepares a fresh run and proves duplicate preparation is idempotent;
  - verifies direct wait/push mixed-mode rejection and the explicit
    pull-to-push stop/retry sequence.
- **`.github/workflows/ci.yml`**:
  add a targeted fallback job on `macos-latest` and `windows-latest`, leaving the
  existing broad Linux/Windows matrix unchanged.

### Success Criteria

#### Automated Verification

- [x] `cargo test --test daemon_process_sqlite copilot_fallback` passes locally
  on macOS.
- [ ] The targeted GitHub Actions job passes on macOS and Windows.
- [x] Existing workspace tests remain green: `cargo test --workspace`.

#### Manual Verification

- [x] The macOS launch path uses direct executable argv.
- [ ] The Windows launch path uses the generated PowerShell file and no
  prompt-embedded wrapper body.

---

## Phase 4: Documentation

### Changes Required

- **`.paw/work/mac-2026-07-09a-88/Docs.md`**:
  create the as-built technical reference using `paw-docs-guidance`.
- **`copilot/COPILOT.md`**:
  replace the manual Windows-only fallback with the prepare/launch/artifact
  workflow, explicit push/pull transitions, dedupe rules, protocol-skew
  remediation, and teardown guidance.
- **`docs/guide/src/guides/copilot-push.md`**:
  document the operator-level fallback and link the generic pull artifact
  contract.
- **`docs/design/DECISIONS.md`**:
  record the harness-neutral exclusivity invariant, additive protocol field,
  idempotent Copilot run preparation, and asymmetric transition rationale.
- **Generated CLI reference**:
  rely on `docs/guide/generate-reference.sh` during the docs build rather than
  hand-editing generated output.

### Success Criteria

#### Automated Verification

- [ ] Embedded-skill boundary tests pass:
  `cargo test --test copilot_plugin`.
- [ ] Documentation builds with `mdbook build docs/guide` when mdBook is
  available.

#### Manual Verification

- [ ] `telex copilot skill` gives one platform-appropriate fallback sequence
  with no infinite loop and no agent-authored platform script.
- [ ] Root `telex skill` remains harness-neutral.
- [ ] Docs.md accurately matches the final CLI and artifact contract.

---

## Final Verification

- [ ] `cargo fmt --check`
- [ ] `cargo test --workspace`
- [ ] `cargo build --workspace`
- [ ] `cargo clippy --workspace -- -D warnings`
- [ ] `node --check copilot/bridge/extension.mjs`
- [ ] `node --test copilot/bridge/busy-state.test.mjs`
- [ ] Configured single-model final review by `claude-opus-4.8` has no blocking
  findings.
- [ ] Issue #88 acceptance is satisfied end to end; PR uses `Closes #88`.

## References

- Issue: https://github.com/lossyrob/telex/issues/88
- Specification: issue #88 (no separate Spec.md by workflow configuration)
- Research: `.paw/work/mac-2026-07-09a-88/CodeResearch.md`
- Workflow: `.paw/work/mac-2026-07-09a-88/WorkflowContext.md`
