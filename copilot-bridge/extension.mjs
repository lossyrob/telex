// telex copilot bridge
//
// An in-session Copilot CLI extension that lets an external same-user process
// (the telex daemon's on-deliver handler, `telex copilot push`) inject a
// message into THIS live session as a turn -- without the agent running or
// re-arming a `telex wait` waiter.
//
// telex embeds these bytes (include_str!) and writes them into the session
// extension dir on `telex copilot attach --copilot-bridge`; the agent then runs
// the `extensions_reload` tool once to load it. `telex copilot detach` removes
// the file (and the agent reloads to unload).
//
// Transport (Option A): a per-session OS named pipe (Windows) / unix domain
// socket (POSIX). The endpoint path is derived from the Copilot session id so it
// is stable across `/clear` reloads. Access is gated two ways: the OS (POSIX 0700 dir +
// 0600 socket) AND a per-session secret, written into the owner-only registry, that every
// push request must present. The secret is required because the default Windows named-pipe
// DACL grants Everyone READ, so the OS ACL alone does not restrict the pipe to the owner.
//
// Wire protocol: one JSON request per connection, newline-terminated:
//   {"prompt": "...", "displayPrompt": "[telex] from <addr>", "mode": "enqueue"}
// Response, newline-terminated:
//   {"ok": true, "sessionId": "...", "messageId": "...", "mode": "enqueue"}
// The bridge forwards `mode` verbatim; the attention->mode decision
// (interrupt -> immediate, else -> enqueue) is made by `telex copilot push`.

import { mkdir, writeFile, rm, chmod, readFile } from "node:fs/promises";
import { createServer } from "node:net";
import { homedir, platform } from "node:os";
import { join } from "node:path";
import { joinSession } from "@github/copilot-sdk/extension";
import { randomBytes } from "node:crypto";

const isPosix = platform() !== "win32";
const MAX_REQUEST_BYTES = 1024 * 1024; // 1 MiB guard
const registryDir = join(homedir(), ".copilot", "telex-bridge");
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
// Per-session shared secret: an application-layer capability so only a client that can read
// the owner-only registry (i.e. `telex copilot push`) may inject a turn. Defense-in-depth
// over the OS ACL, needed because the default Windows named-pipe DACL grants Everyone READ.
const secret = randomBytes(32).toString("hex");
const registryPath = join(registryDir, `${sessionId}.json`);

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
    const options = { prompt: input.prompt, mode };
    if (typeof input.displayPrompt === "string" && input.displayPrompt) {
      options.displayPrompt = input.displayPrompt;
    }
    try {
      const messageId = await session.send(options);
      writeResponse(socket, { ok: true, sessionId, messageId, mode });
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

await writeFile(
  registryPath,
  JSON.stringify(
    {
      sessionId,
      endpoint,
      pid: process.pid,
      secret,
      createdAt: new Date().toISOString(),
    },
    null,
    2,
  ),
  "utf8",
);
if (isPosix) {
  await chmod(registryPath, 0o600).catch(() => {});
}

await session.log(
  `telex-bridge ready for ${sessionId} on ${endpoint.kind} ${endpoint.path}`,
);

const cleanup = async () => {
  try {
    server.close();
  } catch {}
  // Only remove the registry AND the unix socket if they still point at THIS process, so a
  // /clear reload (old process SIGTERM'd after the new one rewrote the registry and rebound the
  // same session-derived endpoint) does not delete the newer bridge's registry/socket and make
  // push report "no live bridge".
  try {
    const raw = await readFile(registryPath, "utf8");
    if (JSON.parse(raw).pid === process.pid) {
      await rm(registryPath, { force: true });
      if (endpoint.kind === "unix") {
        await rm(endpoint.path, { force: true }).catch(() => {});
      }
    }
  } catch {}
};

process.once("SIGTERM", cleanup);
process.once("SIGINT", cleanup);
