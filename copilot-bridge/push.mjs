// telex copilot push (prototype reference)
//
// Simulates the telex daemon's on-deliver handler: resolve a session's bridge
// endpoint from the registry, connect, hand off one message, print the result.
// The real handler is `telex copilot push --session <id>` (Rust), exec'd by the
// daemon's generic on-deliver primitive. This JS reference proves the wire
// contract and is the oracle the Rust handler must match.
//
// Usage:
//   node push.mjs --session <copilot-session-id> --prompt "text" [--display "[telex] from <addr>"] [--mode enqueue|immediate]
//   node push.mjs --latest --prompt "text"            # target the most recent bridge
//   node push.mjs --registry <path-to-json> --prompt "text"

import { readFile, readdir, stat } from "node:fs/promises";
import { connect } from "node:net";
import { homedir } from "node:os";
import { join } from "node:path";

const registryDir = join(homedir(), ".copilot", "telex-bridge");

function parseArgs(argv) {
  const out = { mode: "enqueue" };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--session") out.session = argv[++i];
    else if (a === "--prompt") out.prompt = argv[++i];
    else if (a === "--display") out.display = argv[++i];
    else if (a === "--mode") out.mode = argv[++i];
    else if (a === "--registry") out.registry = argv[++i];
    else if (a === "--latest") out.latest = true;
  }
  return out;
}

async function resolveRegistryPath(args) {
  if (args.registry) return args.registry;
  if (args.session) return join(registryDir, `${args.session}.json`);
  if (args.latest) {
    const files = (await readdir(registryDir)).filter((f) =>
      f.endsWith(".json"),
    );
    if (files.length === 0) throw new Error("no bridge registry entries found");
    let best = null;
    for (const f of files) {
      const p = join(registryDir, f);
      const s = await stat(p);
      if (!best || s.mtimeMs > best.mtimeMs) best = { p, mtimeMs: s.mtimeMs };
    }
    return best.p;
  }
  throw new Error("specify --session <id>, --registry <path>, or --latest");
}

function sendOverEndpoint(endpoint, payload) {
  return new Promise((resolve, reject) => {
    const socket = connect(endpoint.path);
    let raw = "";
    let done = false;
    const finish = (fn, arg) => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      try {
        socket.end();
      } catch {}
      fn(arg);
    };
    const timer = setTimeout(() => {
      try {
        socket.destroy();
      } catch {}
      finish(reject, new Error("timeout waiting for bridge response"));
    }, 10000);
    socket.setEncoding("utf8");
    socket.on("connect", () => {
      socket.write(JSON.stringify(payload) + "\n");
    });
    socket.on("data", (chunk) => {
      raw += chunk;
      const nl = raw.indexOf("\n");
      if (nl === -1) return;
      try {
        finish(resolve, JSON.parse(raw.slice(0, nl).trim() || "{}"));
      } catch (e) {
        finish(reject, new Error("bad response from bridge: " + raw));
      }
    });
    socket.on("end", () => {
      if (done) return;
      try {
        finish(resolve, JSON.parse(raw.trim() || "{}"));
      } catch (e) {
        finish(reject, new Error("bad response from bridge: " + raw));
      }
    });
    socket.on("error", (e) => finish(reject, e));
  });
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  if (!args.prompt) throw new Error("--prompt is required");
  const registryPath = await resolveRegistryPath(args);
  const reg = JSON.parse(await readFile(registryPath, "utf8"));
  if (!reg.endpoint || !reg.endpoint.path) {
    throw new Error("registry entry has no endpoint: " + registryPath);
  }
  const payload = {
    prompt: args.prompt,
    mode: args.mode,
  };
  if (args.display) payload.displayPrompt = args.display;
  const result = await sendOverEndpoint(reg.endpoint, payload);
  console.log(JSON.stringify(result));
  process.exit(result.ok ? 0 : 1);
}

main().catch((e) => {
  console.error("push failed: " + (e && e.message ? e.message : e));
  process.exit(2);
});
