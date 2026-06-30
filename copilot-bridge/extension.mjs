// telex copilot bridge (prototype)
//
// An in-session Copilot CLI extension that lets an external same-user process
// (the telex daemon's on-deliver handler, `telex copilot push`) inject a
// message into THIS live session as a queued turn -- without the agent running
// or re-arming a `telex wait` waiter.
//
// Transport: a per-session OS named pipe (Windows) / unix domain socket
// (POSIX), authorized by same-user OS ACL. No bearer token at rest. The pipe
// path is derived from the Copilot session id, so it is stable across `/clear`
// reloads.
//
// Wire protocol: one JSON request object per connection, newline-terminated:
//   {"prompt": "...", "displayPrompt": "[telex] from <addr>", "mode": "enqueue"}
// Response, newline-terminated:
//   {"ok": true, "sessionId": "...", "messageId": "...", "mode": "enqueue"}
//
// This is a PROTOTYPE of the bytes telex would embed (include_str!) and write
// into the session extension dir on `telex attach --copilot-bridge`.

import { mkdir, writeFile, rm } from "node:fs/promises";
import { createServer } from "node:net";
import { homedir, platform } from "node:os";
import { join } from "node:path";
import { joinSession } from "@github/copilot-sdk/extension";

const MAX_REQUEST_BYTES = 1024 * 1024; // 1 MiB guard
const registryDir = join(homedir(), ".copilot", "telex-bridge");
await mkdir(registryDir, { recursive: true });

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

await writeFile(
  registryPath,
  JSON.stringify(
    {
      sessionId,
      endpoint,
      pid: process.pid,
      createdAt: new Date().toISOString(),
    },
    null,
    2,
  ),
  "utf8",
);

await session.log(
  `telex-bridge ready for ${sessionId} on ${endpoint.kind} ${endpoint.path}`,
);

const cleanup = async () => {
  try {
    server.close();
  } catch {}
  try {
    await rm(registryPath, { force: true });
  } catch {}
  if (endpoint.kind === "unix") {
    await rm(endpoint.path, { force: true }).catch(() => {});
  }
};

process.once("SIGTERM", cleanup);
process.once("SIGINT", cleanup);
