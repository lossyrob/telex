// Behavioral tests for the bridge busy/idle contract (issue #65). Run with `node --test`.
// The pure busy-state module has no SDK dependency, so a fake clock + synthetic events fully
// exercise the core new behavior of the idle-drain feature.

import { test } from "node:test";
import assert from "node:assert/strict";
import { createBusyTracker, DEFERRED_UNTIL_IDLE } from "./busy-state.mjs";

test("deferred contract string is the exact literal the Rust push handler matches", () => {
  // Cross-language contract with src/commands/copilot.rs::BRIDGE_DEFERRED_ERROR. A drift here
  // silently downgrades busy-deferral to a transient fast-retry.
  assert.equal(DEFERRED_UNTIL_IDLE, "deferred_until_idle");
});

// A controllable clock so tests are deterministic (no real sleeping).
function fakeClock(startMs = 1_000_000) {
  let t = startMs;
  return { now: () => t, advance: (ms) => (t += ms) };
}

const root = (type) => ({ type }); // no agentId => root-agent event
const sub = (type) => ({ type, agentId: "sub-1" });

test("defaults to busy before any root turn boundary is observed", () => {
  const tracker = createBusyTracker({ now: () => 1000 });
  assert.equal(tracker.currentlyBusy(), true);
});

test("root turn_end clears busy; root turn_start sets it", () => {
  const clock = fakeClock();
  const tracker = createBusyTracker({ now: clock.now });
  tracker.onEvent(root("assistant.turn_end"));
  assert.equal(tracker.currentlyBusy(), false, "root turn_end => idle");
  tracker.onEvent(root("assistant.turn_start"));
  assert.equal(tracker.currentlyBusy(), true, "root turn_start => busy");
});

test("sub-agent turn events do NOT clear busy (agentId filter)", () => {
  const clock = fakeClock();
  const tracker = createBusyTracker({ now: clock.now });
  tracker.onEvent(root("assistant.turn_start"));
  tracker.onEvent(sub("assistant.turn_end")); // inner sub-agent turn ends...
  assert.equal(
    tracker.currentlyBusy(),
    true,
    "a sub-agent turn_end must not clear the root busy gate",
  );
  tracker.onEvent(sub("assistant.turn_start"));
  assert.equal(tracker.currentlyBusy(), true);
});

test("after a confirmed root turn_start, a long quiet window does NOT idle-heal", () => {
  // The regression the PAW review flagged: a genuinely long but event-quiet root turn must stay
  // busy so a non-interrupt push keeps deferring rather than queueing behind the running turn.
  const clock = fakeClock();
  const tracker = createBusyTracker({
    now: clock.now,
    idleStaleMs: 60 * 1000,
    maxTurnMs: 30 * 60 * 1000,
  });
  tracker.onEvent(root("assistant.turn_start"));
  clock.advance(5 * 60 * 1000); // 5 minutes of silence, well past the idle window
  assert.equal(
    tracker.currentlyBusy(),
    true,
    "a confirmed running turn stays busy through a quiet window (no idle-heal)",
  );
});

test("boot-unknown busy idle-heals after the idle window (no turn ever observed)", () => {
  // Reattach into an idle session: busy is only a boot guess, so a quiet window means no turn.
  const clock = fakeClock();
  const tracker = createBusyTracker({ now: clock.now, idleStaleMs: 60 * 1000 });
  assert.equal(tracker.currentlyBusy(), true);
  clock.advance(61 * 1000);
  assert.equal(
    tracker.currentlyBusy(),
    false,
    "boot-unknown busy heals to idle after the idle window",
  );
  assert.equal(tracker.snapshot().staleHealCount, 1);
});

test("boot-unknown busy stays busy while activity keeps arriving inside the idle window", () => {
  const clock = fakeClock();
  const tracker = createBusyTracker({ now: clock.now, idleStaleMs: 60 * 1000 });
  for (let i = 0; i < 5; i++) {
    clock.advance(30 * 1000);
    tracker.onEvent(sub("assistant.streaming_delta")); // activity, but not a root boundary
    assert.equal(tracker.currentlyBusy(), true, "activity keeps the boot-unknown busy fresh");
  }
});

test("hard ceiling heals a stuck busy even while activity continues (missed turn_end)", () => {
  // SDK drift / crash mid-turn where turn_end never fires but other events keep coming: the idle
  // heal is defeated by activity, so the ceiling is the backstop that prevents a permanent latch.
  const clock = fakeClock();
  const tracker = createBusyTracker({
    now: clock.now,
    idleStaleMs: 60 * 1000,
    maxTurnMs: 30 * 60 * 1000,
  });
  tracker.onEvent(root("assistant.turn_start"));
  for (let i = 0; i < 40; i++) {
    clock.advance(60 * 1000); // 40 minutes of periodic activity, no turn_end
    tracker.onEvent(sub("assistant.streaming_delta"));
  }
  assert.equal(
    tracker.currentlyBusy(),
    false,
    "the max-turn ceiling clears a busy that a missed turn_end would otherwise latch",
  );
});
