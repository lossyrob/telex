// Contract tests for the bridge probe protocol.
//
// The literal op/field/error strings are asserted here *and* in the Rust reconciler, following the
// `busy-state.test.mjs` precedent: the two implementations of one wire contract must not be able to
// drift silently.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

import {
  BRIDGE_PROBE_MIN_PROTOCOL,
  COPILOT_BRIDGE_PROTOCOL,
  PROBE_ERRORS,
  PROBE_NONCE_MAX_LENGTH,
  PROBE_NONCE_MIN_LENGTH,
  PROBE_OP,
  buildProbeError,
  buildProbeResponse,
  classifyRequest,
  createProbeRateLimiter,
  secretMatches,
  validateProbeRequest,
} from "./probe-protocol.mjs";

const SECRET = "a".repeat(64);
const NONCE = "b".repeat(32);

test("cross-language contract: the literals the Rust reconciler asserts", () => {
  // Mirrors busy-state.test.mjs: these values are duplicated in src/daemon_reconcile.rs and
  // src/station_intent.rs, so a rename on either side fails here.
  assert.equal(PROBE_OP, "probe");
  assert.equal(COPILOT_BRIDGE_PROTOCOL, 2);
  assert.equal(BRIDGE_PROBE_MIN_PROTOCOL, 2);
  assert.equal(PROBE_ERRORS.UNAUTHORIZED, "unauthorized");
  assert.equal(PROBE_ERRORS.UNSUPPORTED_OP, "unsupported_op");
  assert.equal(PROBE_ERRORS.UNSUPPORTED_PROTOCOL, "unsupported_protocol");

  const rust = readFileSync(new URL("../../src/daemon_reconcile.rs", import.meta.url), "utf8");
  assert.match(rust, /BRIDGE_PROBE_MIN_PROTOCOL: u32 = 2;/);
  assert.match(rust, /"op": "probe"/);
  assert.match(rust, /"unsupported_op"/);
  assert.match(rust, /"unsupported_protocol"/);
  assert.match(rust, /"bridgeGeneration"/);
  assert.match(rust, /"sessionId"/);
});

test("classifyRequest separates a probe from a push", () => {
  assert.equal(classifyRequest({ op: "probe" }).kind, "probe");
  assert.equal(classifyRequest({ prompt: "hi" }).kind, "push");
  assert.equal(classifyRequest(null).kind, "push");
  assert.equal(classifyRequest({ op: "something-else" }).kind, "push");
});

test("a valid probe echoes the nonce and names the session", () => {
  const validated = validateProbeRequest(
    { op: PROBE_OP, secret: SECRET, nonce: NONCE, protocol: 2 },
    SECRET,
  );
  assert.equal(validated.ok, true);
  const response = buildProbeResponse({
    nonce: validated.nonce,
    sessionId: "sess-1",
    bridgeGeneration: 7,
  });
  assert.deepEqual(response, {
    ok: true,
    nonce: NONCE,
    sessionId: "sess-1",
    protocol: 2,
    bridgeGeneration: 7,
  });
});

test("a probe response never leaks paths, secrets, or busy diagnostics", () => {
  const response = buildProbeResponse({
    nonce: NONCE,
    sessionId: "sess-1",
    bridgeGeneration: 1,
  });
  const keys = Object.keys(response).sort();
  assert.deepEqual(keys, ["bridgeGeneration", "nonce", "ok", "protocol", "sessionId"]);
  const encoded = JSON.stringify(response);
  assert.ok(!encoded.includes(SECRET));
  assert.ok(!encoded.includes("endpoint"));
  assert.ok(!encoded.includes("busy"));
});

test("a wrong or missing secret is rejected without echoing anything", () => {
  for (const secret of [undefined, null, "", "c".repeat(64), SECRET.slice(0, 63)]) {
    const result = validateProbeRequest({ op: PROBE_OP, secret, nonce: NONCE }, SECRET);
    assert.equal(result.ok, false);
    assert.equal(result.error, PROBE_ERRORS.UNAUTHORIZED);
    assert.equal(result.nonce, undefined);
  }
});

test("secretMatches is total and length-safe", () => {
  assert.equal(secretMatches(SECRET, SECRET), true);
  assert.equal(secretMatches("short", SECRET), false);
  assert.equal(secretMatches(undefined, SECRET), false);
  assert.equal(secretMatches(SECRET, undefined), false);
  assert.equal(secretMatches(123, SECRET), false);
});

test("a missing or malformed nonce is rejected", () => {
  for (const nonce of [undefined, "", "tooshort", "d".repeat(PROBE_NONCE_MAX_LENGTH + 1), 42]) {
    const result = validateProbeRequest({ op: PROBE_OP, secret: SECRET, nonce }, SECRET);
    assert.equal(result.ok, false);
    assert.equal(result.error, PROBE_ERRORS.NONCE_REQUIRED);
  }
  const shortest = validateProbeRequest(
    { op: PROBE_OP, secret: SECRET, nonce: "e".repeat(PROBE_NONCE_MIN_LENGTH) },
    SECRET,
  );
  assert.equal(shortest.ok, true);
});

test("a protocol newer than this bridge is refused rather than guessed at", () => {
  const result = validateProbeRequest(
    { op: PROBE_OP, secret: SECRET, nonce: NONCE, protocol: COPILOT_BRIDGE_PROTOCOL + 1 },
    SECRET,
  );
  assert.equal(result.ok, false);
  assert.equal(result.error, PROBE_ERRORS.UNSUPPORTED_PROTOCOL);
});

test("probe rate limiting bounds a hostile same-user prober", () => {
  let clock = 0;
  const limiter = createProbeRateLimiter({ max: 3, windowMs: 100, now: () => clock });
  assert.equal(limiter.allow(), true);
  assert.equal(limiter.allow(), true);
  assert.equal(limiter.allow(), true);
  assert.equal(limiter.allow(), false, "the fourth probe inside the window is refused");
  clock += 101;
  assert.equal(limiter.allow(), true, "the window slides");
});

test("buildProbeError carries only an error code", () => {
  assert.deepEqual(buildProbeError(PROBE_ERRORS.RATE_LIMITED), {
    ok: false,
    error: "rate_limited",
  });
});

test("the extension advertises the protocol and wires the probe verb", () => {
  const extension = readFileSync(new URL("./extension.mjs", import.meta.url), "utf8");
  assert.match(extension, /from "\.\/probe-protocol\.mjs"/);
  assert.match(extension, /classifyRequest\(input\)\.kind === "probe"/);
  assert.match(extension, /protocol: COPILOT_BRIDGE_PROTOCOL/);
  assert.match(extension, /bridgeGeneration/);
  // The push path must use the constant-time comparison too, not `!==`.
  assert.match(extension, /!secretMatches\(input\.secret, secret\)/);
});
