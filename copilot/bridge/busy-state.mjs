// Busy/idle state machine for the telex copilot bridge (issue #65).
//
// Extracted into a side-effect-free module (no SDK import, no top-level work) so the busy contract
// -- the core new behavior of the idle-drain feature -- can be unit-tested with a fake clock and
// synthetic events (`busy-state.test.mjs`), while `extension.mjs` stays a single import-loaded
// script the Copilot extension host runs for its side effects. `telex copilot attach` materializes
// this file alongside `extension.mjs` in the session extension dir so the relative import resolves.
//
// Contract:
//  - Busy is gated on the ROOT-agent turn boundary only. Sub-agent turn events carry an `agentId`
//    and are ignored; a sub-agent's inner `turn_end` must not clear the gate while the parent turn
//    is still in flight, or the stale-injection this feature removes would return.
//  - Default is busy (deferring is the safe action; injecting a stale turn is the risk). This
//    conservative default is "boot-unknown": we have not yet observed an authoritative root turn
//    boundary.
//  - Self-heal clears a stuck busy so a missed/never-fired `turn_end` (crash, SDK drift, or a load
//    outside any turn) cannot defer forever:
//      * IDLE heal (short window of no activity) applies ONLY in the boot-unknown state -- before a
//        root `turn_start` has been observed. After a confirmed `turn_start`, a quiet window is a
//        legitimately-running-but-silent turn, not idleness, so the idle heal must not fire there
//        (that false-clear would re-queue a push behind the running turn -- the exact bug #65 fixes).
//      * CEILING heal (busy older than a full turn budget) always applies as a hard backstop against
//        a missed `turn_end` even while unrelated activity continues.

// Cross-language contract string (issue #65): the bridge returns this exact `error` value for a
// busy-deferred push, and `telex copilot push` (Rust) matches the same literal to map it to
// PUSH_EXIT_DEFERRED. A drift on either side silently downgrades deferral to a transient retry, so
// both sides pin the literal via a named constant + a test. Keep in sync with the Rust
// `BRIDGE_DEFERRED_ERROR` in `src/commands/copilot.rs`.
export const DEFERRED_UNTIL_IDLE = "deferred_until_idle";

export function createBusyTracker(options = {}) {
  const idleStaleMs = options.idleStaleMs ?? 60 * 1000;
  const maxTurnMs = options.maxTurnMs ?? 30 * 60 * 1000;
  const now = options.now ?? (() => Date.now());

  let busy = true;
  let busySince = now();
  let lastActivityAt = now();
  // Whether we have observed an authoritative root turn boundary (turn_start/turn_end). Until then
  // `busy` is only a conservative boot guess and the short idle heal is allowed.
  let observedRootTurn = false;
  let staleHealCount = 0;

  function onEvent(event) {
    lastActivityAt = now();
    if (!event || event.agentId) return; // sub-agent or malformed: not the root turn boundary
    if (event.type === "assistant.turn_start") {
      busy = true;
      busySince = now();
      observedRootTurn = true;
    } else if (event.type === "assistant.turn_end") {
      busy = false;
      busySince = null;
      observedRootTurn = true;
    }
  }

  function currentlyBusy() {
    if (busy && busySince != null) {
      const t = now();
      const ceilingHit = t - busySince > maxTurnMs;
      const idleHit = !observedRootTurn && t - lastActivityAt > idleStaleMs;
      if (ceilingHit || idleHit) {
        busy = false;
        busySince = null;
        staleHealCount += 1;
      }
    }
    return busy;
  }

  function snapshot() {
    return { busy, busySince, observedRootTurn, staleHealCount };
  }

  return { onEvent, currentlyBusy, snapshot };
}
