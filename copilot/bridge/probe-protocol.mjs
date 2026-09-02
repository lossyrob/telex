// telex copilot bridge — probe protocol
//
// A pure, SDK-free module (the `busy-state.mjs` precedent) holding the exact wire contract of the
// `probe` verb, so the contract is unit-testable without a live Copilot session and so the literal
// op/field/error strings can be asserted against their Rust counterparts.
//
// Why a probe verb exists at all: the telex daemon restores a push registration only after it has
// *proved* the producer is alive. OS-level peer verification (same user, matching executable,
// matching pid + start time) happens before a single byte is sent; the probe then proves the
// process on the other end is this session's bridge and speaks a protocol the daemon understands.
//
// What a probe response deliberately does NOT contain: no file paths, no busy/idle diagnostics, no
// secret, no message content. It echoes the nonce, names the session, states the protocol, and
// reports a bridge generation. Anything more would be an information-disclosure surface on an
// endpoint whose whole job is to answer "are you alive and are you who I recorded".

import { timingSafeEqual } from "node:crypto";

/** Bridge wire protocol version. Bumped from 1 to 2 by the `probe` verb. */
export const COPILOT_BRIDGE_PROTOCOL = 2;

/** Minimum protocol a producer must advertise for the daemon to treat it as probe-capable. */
export const BRIDGE_PROBE_MIN_PROTOCOL = 2;

export const PROBE_OP = "probe";

export const PROBE_ERRORS = {
  BAD_JSON: "bad_json",
  UNAUTHORIZED: "unauthorized",
  UNSUPPORTED_OP: "unsupported_op",
  UNSUPPORTED_PROTOCOL: "unsupported_protocol",
  NONCE_REQUIRED: "nonce_required",
  REQUEST_TOO_LARGE: "request_too_large",
  RATE_LIMITED: "rate_limited",
};

/** Bounds on a probe nonce: long enough to be unguessable, short enough to bound work. */
export const PROBE_NONCE_MIN_LENGTH = 16;
export const PROBE_NONCE_MAX_LENGTH = 128;

/** Probe rate limit: a generous ceiling that still bounds a hostile same-user prober. */
export const PROBE_RATE_LIMIT_MAX = 30;
export const PROBE_RATE_LIMIT_WINDOW_MS = 10_000;

/**
 * Constant-time secret comparison.
 *
 * `timingSafeEqual` throws on length mismatch, so lengths are compared first — that leak is
 * unavoidable and harmless (the secret length is a fixed constant of the implementation), while a
 * byte-by-byte early return would leak the secret itself.
 */
export function secretMatches(provided, expected) {
  if (typeof provided !== "string" || typeof expected !== "string") return false;
  const a = Buffer.from(provided, "utf8");
  const b = Buffer.from(expected, "utf8");
  if (a.length !== b.length) return false;
  try {
    return timingSafeEqual(a, b);
  } catch {
    return false;
  }
}

/**
 * Classify a parsed request line.
 *
 * Returns `{ kind: "probe" | "push" }`, so the caller dispatches on one value rather than
 * re-deriving the shape at each branch.
 */
export function classifyRequest(input) {
  if (input && typeof input === "object" && input.op === PROBE_OP) {
    return { kind: "probe" };
  }
  return { kind: "push" };
}

/**
 * Validate a probe request against the shared secret.
 *
 * Fail-closed and order-sensitive: authorization is checked before anything about the request body
 * is echoed back, so an unauthorized caller learns nothing beyond "unauthorized".
 */
export function validateProbeRequest(input, expectedSecret) {
  if (!secretMatches(input?.secret, expectedSecret)) {
    return { ok: false, error: PROBE_ERRORS.UNAUTHORIZED };
  }
  const nonce = input?.nonce;
  if (
    typeof nonce !== "string" ||
    nonce.length < PROBE_NONCE_MIN_LENGTH ||
    nonce.length > PROBE_NONCE_MAX_LENGTH
  ) {
    return { ok: false, error: PROBE_ERRORS.NONCE_REQUIRED };
  }
  const requested = input?.protocol;
  if (typeof requested === "number" && requested > COPILOT_BRIDGE_PROTOCOL) {
    return { ok: false, error: PROBE_ERRORS.UNSUPPORTED_PROTOCOL };
  }
  return { ok: true, nonce };
}

/**
 * Build the probe response.
 *
 * The nonce is echoed byte-for-byte: that, combined with the daemon having verified the peer's
 * process identity before sending, is what makes a replayed or forged answer useless.
 */
export function buildProbeResponse({ nonce, sessionId, bridgeGeneration }) {
  return {
    ok: true,
    nonce,
    sessionId,
    protocol: COPILOT_BRIDGE_PROTOCOL,
    bridgeGeneration,
  };
}

export function buildProbeError(error) {
  return { ok: false, error };
}

/**
 * A simple sliding-window rate limiter for probe requests.
 *
 * Kept in this module (rather than inline in the extension) so its behavior is unit-tested: an
 * unbounded probe verb on a long-lived endpoint is a cheap way to burn a session's CPU.
 */
export function createProbeRateLimiter({
  max = PROBE_RATE_LIMIT_MAX,
  windowMs = PROBE_RATE_LIMIT_WINDOW_MS,
  now = () => Date.now(),
} = {}) {
  let hits = [];
  return {
    allow() {
      const cutoff = now() - windowMs;
      hits = hits.filter((t) => t > cutoff);
      if (hits.length >= max) return false;
      hits.push(now());
      return true;
    },
    get size() {
      return hits.length;
    },
  };
}
