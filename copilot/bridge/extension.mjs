// telex copilot bridge
//
// An in-session Copilot CLI extension that lets an external same-user process
// (the telex daemon's on-deliver handler, `telex copilot push`) inject a
// message into THIS live session as a turn -- without the agent running or
// re-arming a `telex wait` waiter.
//
// telex embeds these bytes (include_str!) and writes them into the session
// extension dir on `telex copilot attach --copilot-bridge`. A new live session
// runs `extensions_reload` once; a resumed session discovers the retained file
// during startup. `telex copilot detach` removes the file (and the agent reloads
// to unload it immediately when needed).
//
// Transport (Option A): a per-session OS named pipe (Windows) / unix domain
// socket (POSIX). The endpoint path is derived from the Copilot session id so it
// is stable across `/clear` reloads. Access is gated two ways: the OS (POSIX 0700 dir +
// 0600 socket) AND a per-session secret, written into the owner-only registry, that every
// push request must present. The secret is required because the default Windows named-pipe
// DACL grants Everyone READ, so the OS ACL alone does not restrict the pipe to the owner.
//
// Wire protocol: one JSON request per connection, newline-terminated:
//   {"prompt": "...", "displayPrompt": "[telex] FROM: <addr> SUBJECT: <subject>", "mode": "enqueue"}
// Response, newline-terminated:
//   {"ok": true, "sessionId": "...", "mode": "enqueue", "accepted": "queued"|"pending"}
// or, when a non-`interrupt` push arrives while a root turn is running (issue #65):
//   {"ok": false, "sessionId": "...", "mode": "enqueue", "error": "deferred_until_idle"}
// The bridge forwards `mode` verbatim; the attention->mode decision
// (interrupt -> immediate, else -> enqueue) is made by `telex copilot push`.

import { mkdir, writeFile, rm, chmod, readFile } from "node:fs/promises";
import { createServer } from "node:net";
import { homedir, platform } from "node:os";
import { join } from "node:path";
import { joinSession } from "@github/copilot-sdk/extension";
import { randomBytes } from "node:crypto";
import { createBusyTracker, DEFERRED_UNTIL_IDLE } from "./busy-state.mjs";

const isPosix = platform() !== "win32";
const MAX_REQUEST_BYTES = 8 * 1024 * 1024; // 8 MiB: fits a max daemon message plus JSON-escaped prompt wrapping, so large messages push as turns instead of dead-lettering
const SEND_ACK_TIMEOUT_MS = 2_000;
const pendingSends = new Set();

// Busy/idle tracking (issue #65) lives in the side-effect-free `busy-state.mjs` module so its
// contract is unit-tested (busy-state.test.mjs). The bridge defers non-`interrupt` pushes while a
// root turn is running so a queued turn cannot land behind (and duplicate) work the agent handles
// manually mid-turn. This is liveness/scheduling state only -- durable Telex state stays authoritative.
const busyTracker = createBusyTracker();
const currentlyBusy = () => busyTracker.currentlyBusy();

const copilotHome =
  process.env.TELEX_COPILOT_HOME || join(homedir(), ".copilot");
const registryDir = join(copilotHome, "telex-bridge");
await mkdir(registryDir, { recursive: true, mode: 0o700 });
if (isPosix) {
  await chmod(registryDir, 0o700).catch(() => {});
}

// joinSession first: we need session.sessionId to derive the endpoint + registry.
const session = await joinSession({
  tools: [
    {
      name: "telex_bridge_info",
      description:
        "Return this session's telex bridge endpoint, registry path, and pid.",
      parameters: { type: "object", properties: {} },
      // No skipPermission: the bridge requests no elevated permission, so a
      // (re)load is silent (no permission prompt). The push path is the pipe,
      // not this tool; the tool is only a debug affordance.
      handler: async () =>
        JSON.stringify(
          { sessionId, endpoint, registryPath, pid: process.pid },
          null,
          2,
        ),
    },
  ],
});

const sessionId = session.sessionId;
const bindingsPath = join(registryDir, `${sessionId}.bindings.json`);

// Feed every session event to the busy tracker; it filters to root-agent turn boundaries and
// maintains the self-heal timers (see busy-state.mjs for the contract).
session.on((event) => busyTracker.onEvent(event));

// Per-session shared secret: an application-layer capability so only a client that can read
// the owner-only registry (i.e. `telex copilot push`) may inject a turn. Defense-in-depth
// over the OS ACL, needed because the default Windows named-pipe DACL grants Everyone READ.
const secret = randomBytes(32).toString("hex");
const registryPath = join(registryDir, `${sessionId}.json`);
const bridgeProtocol = Number("__TELEX_BRIDGE_PROTOCOL__");
const telexBuildId = "__TELEX_BUILD_ID__";

// Derive the same-user endpoint from the session id (stable across reloads).
const endpoint =
  platform() === "win32"
    ? { kind: "pipe", path: `\\\\.\\pipe\\telex-bridge-${sessionId}` }
    : { kind: "unix", path: join(registryDir, `${sessionId}.sock`) };

function writeResponse(socket, value) {
  try {
    socket.write(JSON.stringify(value) + "\n");
  } catch {}
}

function sendAckTimeout() {
  return new Promise((resolve) => {
    setTimeout(() => resolve({ timedOut: true }), SEND_ACK_TIMEOUT_MS).unref();
  });
}

function observeSendPromise(sendPromise) {
  const observed = Promise.resolve(sendPromise)
    .then((messageId) => ({ ok: true, messageId }))
    .catch((e) => {
      console.error(
        "telex-bridge: session.send rejected after push acknowledgement:",
        (e && e.message) || e,
      );
      return { ok: false, error: String((e && e.message) || e) };
    })
    .finally(() => {
      pendingSends.delete(observed);
    });
  pendingSends.add(observed);
  return observed;
}

async function handleConnection(socket) {
  // Newline-delimited framing: process the first complete line (one JSON
  // request), respond with one JSON line, then close. Does not depend on the
  // client half-closing first.
  let raw = "";
  let handled = false;
  socket.setEncoding("utf8");
  socket.on("data", async (chunk) => {
    if (handled) return;
    raw += chunk;
    if (raw.length > MAX_REQUEST_BYTES) {
      handled = true;
      writeResponse(socket, { ok: false, error: "request_too_large" });
      socket.end();
      return;
    }
    const nl = raw.indexOf("\n");
    if (nl === -1) return; // wait for a full line
    handled = true;
    const line = raw.slice(0, nl);
    let input;
    try {
      input = JSON.parse(line.trim() || "{}");
    } catch (e) {
      writeResponse(socket, { ok: false, error: "bad_json" });
      socket.end();
      return;
    }
    if (typeof input.secret !== "string" || input.secret !== secret) {
      writeResponse(socket, { ok: false, error: "unauthorized" });
      socket.end();
      return;
    }
    if (typeof input.prompt !== "string" || input.prompt.trim() === "") {
      writeResponse(socket, { ok: false, error: "prompt_required" });
      socket.end();
      return;
    }
    const mode = input.mode === "immediate" ? "immediate" : "enqueue";
    // Defer-until-idle (issue #65): a non-`interrupt` push while a root turn is running must NOT be
    // sent yet, or it queues behind the current turn and can duplicate work the agent handles
    // manually mid-turn. Yield once first so a just-arrived root `turn_end` event settles before we
    // decide -- this collapses the common `agentStop`-drain-vs-`turn_end` race into a single
    // non-deferred attempt instead of a re-defer. `immediate` (interrupt) is never deferred.
    if (mode === "enqueue" && currentlyBusy()) {
      await new Promise((r) => setImmediate(r));
      if (currentlyBusy()) {
        writeResponse(socket, {
          ok: false,
          error: DEFERRED_UNTIL_IDLE,
          sessionId,
          mode,
        });
        socket.end();
        return;
      }
    }
    const options = { prompt: input.prompt, mode };
    if (typeof input.displayPrompt === "string" && input.displayPrompt) {
      options.displayPrompt = input.displayPrompt;
    }
    try {
      const observedSend = observeSendPromise(session.send(options));
      // Wait briefly for the SDK's `session.send` RPC ack. In the normal/idle path this preserves
      // the old positive confirmation and message id. When the agent is busy, that promise can sit
      // behind the current turn; if `telex copilot push` times out first, the daemon records a failed
      // push and retries quickly, producing duplicate turns even though the original enqueue often
      // lands later. After this short window, report "pending" success and keep observing the promise
      // for logging; durable re-provision/backstop redelivery covers the rare async failure.
      const result = await Promise.race([observedSend, sendAckTimeout()]);
      if (result && result.timedOut) {
        writeResponse(socket, { ok: true, sessionId, mode, accepted: "pending" });
      } else if (!result.ok) {
        writeResponse(socket, { ok: false, error: result.error });
      } else {
        writeResponse(socket, {
          ok: true,
          sessionId,
          messageId: result.messageId,
          mode,
          accepted: "queued",
        });
      }
    } catch (e) {
      writeResponse(socket, {
        ok: false,
        error: String((e && e.message) || e),
      });
    }
    socket.end();
  });
  socket.on("error", () => {});
}

const server = createServer(handleConnection);

// On POSIX a stale socket file blocks listen(); remove it first.
if (endpoint.kind === "unix") {
  await rm(endpoint.path, { force: true }).catch(() => {});
}

await new Promise((resolve, reject) => {
  server.once("error", reject);
  server.listen(endpoint.path, resolve);
});
if (isPosix && endpoint.kind === "unix") {
  // Fail closed: if we cannot restrict the socket to the owner, do not serve an insecure
  // endpoint. The daemon's push handler then fails and retries, and the failure is visible.
  try {
    await chmod(endpoint.path, 0o600);
  } catch (e) {
    try {
      server.close();
    } catch {}
    await rm(endpoint.path, { force: true }).catch(() => {});
    throw new Error(
      "telex-bridge: refusing to serve, could not secure socket permissions: " +
        (e && e.message ? e.message : e),
    );
  }
}

const createdAt = new Date().toISOString();
// Registry write, re-run on a heartbeat so `telex copilot push` and the turn guard can tell a
// live bridge (fresh heartbeat + live pid) from a stale registry a crashed process left behind.
// It also advertises the max request size so push can preflight against the real (negotiated) cap.
async function writeRegistry() {
  await writeFile(
    registryPath,
    JSON.stringify(
      {
        sessionId,
        endpoint,
        pid: process.pid,
        secret,
        maxRequestBytes: MAX_REQUEST_BYTES,
        bridgeProtocol,
        telexBuildId,
        createdAt,
        heartbeatAt: new Date().toISOString(),
        // Diagnostic only (issue #65): the bridge's connect-time answer is the ONLY authoritative
        // busy signal. Nested under `diagnostics` and never read by `telex copilot push` as a
        // decision input -- a 15s-stale flag used for scheduling would reopen the mistimed-injection
        // bug. `busyStaleHealCount` tallies stale-busy self-heals so a missed-turn_end regression is
        // visible in the field rather than silent.
        diagnostics: {
          busyAtLastHeartbeat: busyTracker.currentlyBusy(),
          busySince: busyTracker.snapshot().busySince,
          busyStaleHealCount: busyTracker.snapshot().staleHealCount,
        },
      },
      null,
      2,
    ),
    "utf8",
  );
  if (isPosix) {
    await chmod(registryPath, 0o600).catch(() => {});
  }
}

async function bridgeBindingExists() {
  try {
    const bindings = JSON.parse(await readFile(bindingsPath, "utf8"));
    return Array.isArray(bindings) && bindings.length > 0;
  } catch (e) {
    // Missing bindings means an explicit destructive transition completed. Other read/parse
    // failures fail safe so a corrupt ref-count cannot stop a bridge another address may share.
    return e && e.code === "ENOENT" ? false : true;
  }
}

if (!(await bridgeBindingExists())) {
  try {
    server.close();
  } catch {}
  if (endpoint.kind === "unix") {
    await rm(endpoint.path, { force: true }).catch(() => {});
  }
  process.exit(0);
}

await writeRegistry();
const heartbeatTimer = setInterval(async () => {
  if (!(await bridgeBindingExists())) {
    await cleanup({ removeRegistry: true });
    process.exit(0);
  }
  await writeRegistry().catch(() => {});
}, 15000);
// Never let the heartbeat keep the process alive on its own.
heartbeatTimer.unref?.();

await session.log(
  `telex-bridge ready for ${sessionId} on ${endpoint.kind} ${endpoint.path}`,
);

const cleanup = async ({ removeRegistry = false } = {}) => {
  try {
    clearInterval(heartbeatTimer);
  } catch {}
  try {
    server.close();
  } catch {}
  // Only modify the registry/socket if they still point at THIS process, so a
  // /clear reload (old process SIGTERM'd after the new one rewrote the registry and rebound the
  // same session-derived endpoint) does not delete the newer bridge's registry/socket and make
  // push report "no live bridge".
  try {
    const raw = await readFile(registryPath, "utf8");
    if (JSON.parse(raw).pid === process.pid) {
      if (removeRegistry) {
        await rm(registryPath, { force: true });
      }
      if (endpoint.kind === "unix") {
        await rm(endpoint.path, { force: true }).catch(() => {});
      }
    }
  } catch {}
};

const cleanupOnSignal = async () => {
  await cleanup({ removeRegistry: !(await bridgeBindingExists()) });
};
process.once("SIGTERM", cleanupOnSignal);
process.once("SIGINT", cleanupOnSignal);
