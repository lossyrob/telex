// Shared helpers for the demo seed and the end-to-end suite. Everything here
// talks to a real `telex` binary and a real `station-headless` process inside an
// isolated sandbox; nothing touches ~/.telex.

import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
export const ROOT = path.resolve(here, "..");
export const TELEX = process.env.STATION_TELEX_EXE || path.resolve(ROOT, "../../target/debug/telex");
export const HEADLESS = path.resolve(ROOT, "target/debug/station-headless");
export const NOTIFY = path.resolve(ROOT, "agent-kit/station-notify.mjs");

export function sandboxEnv(home) {
  const r = path.join(home, "sandbox");
  const env = {
    ...process.env,
    TELEX_HOME: path.join(r, "home"),
    TELEX_RUN_DIR: path.join(r, "run"),
    TELEX_DB: path.join(r, "telex.db"),
    TELEX_CONFIG: path.join(r, "config.toml"),
    TELEX_INSTALL_ROOT: path.join(r, "install"),
    XDG_STATE_HOME: path.join(r, "state"),
    TELEX_BIN: TELEX,
  };
  for (const k of ["TELEX_BACKEND", "TELEX_ADDRESS", "TELEX_SESSION_ID", "TELEX_SESSION_PID"]) delete env[k];
  return env;
}

export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

export class Station {
  constructor({ home, port = 4317, operator = "rob", address = "operator:rob" }) {
    this.home = home;
    this.port = port;
    this.operator = operator;
    this.address = address;
    this.env = sandboxEnv(home);
    this.proc = null;
  }

  async start() {
    const log = fs.openSync(path.join(os.tmpdir(), `station-headless-${this.port}.log`), "a");
    this.proc = spawn(HEADLESS, ["--port", String(this.port)], {
      env: { ...process.env, STATION_HOME: this.home, STATION_OPERATOR: this.operator, STATION_ADDRESS: this.address, STATION_TELEX_EXE: TELEX },
      stdio: ["ignore", log, log],
      detached: true,
    });
    this.proc.unref();
    await this.waitFor("station attending", (s) => s.station.connected && s.health.some((h) => h.registered && h.receive_ready && !h.last_error), 30_000);
  }

  /** Simulate a crash: no clean shutdown, no detach. */
  kill() {
    if (this.proc) {
      try {
        process.kill(this.proc.pid, "SIGKILL");
      } catch {
        /* already gone */
      }
      this.proc = null;
    }
  }

  async call(cmd, args = {}) {
    const res = await fetch(`http://127.0.0.1:${this.port}/api/call`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ cmd, args }),
    });
    const payload = await res.json();
    if (!payload.ok) throw new Error(`${cmd}: ${payload.error}`);
    return payload.result;
  }

  snapshot() {
    return this.call("snapshot");
  }

  /** Poll the snapshot until `pred` returns a truthy value; returns it. */
  async waitFor(what, pred, timeout = 15_000) {
    const deadline = Date.now() + timeout;
    let last;
    let health = "";
    for (;;) {
      try {
        const snap = await this.snapshot();
        const value = pred(snap);
        if (value) return value;
        last = undefined;
        health = JSON.stringify(snap.health.map((h) => [h.address, h.registered, h.receive_ready, h.last_error, h.evidence]));
      } catch (e) {
        last = e;
      }
      if (Date.now() > deadline) throw new Error(`timed out waiting for: ${what}${last ? ` (${last.message})` : ""} health=${health}`);
      await sleep(200);
    }
  }

  telex(args, { input, session } = {}) {
    const env = { ...this.env };
    if (session) env.TELEX_SESSION_ID = session;
    const r = spawnSync(TELEX, args, { env, encoding: "utf8", input });
    return { code: r.status, out: r.stdout, err: r.stderr, json: () => JSON.parse(r.stdout) };
  }

  /** Stop the sandbox daemon so a later run starts clean. */
  stopDaemon() {
    this.telex(["daemon", "stop", "--drain"]);
  }
}

/** A simulated agent session that uses the real agent kit and the real CLI. */
export class Agent {
  constructor(station, { address, name, harness, host, repo, branch, cwd, terminal, project, os: osName, open }) {
    this.station = station;
    this.address = address;
    this.name = name;
    this.session = `sim-${name}`;
    this.cardArgs = [];
    const fields = { harness, host, repo, branch, cwd, terminal, os: osName, open_command: open };
    for (const [k, v] of Object.entries(fields)) if (v) this.cardArgs.push("--card", `${k}=${v}`);
    this.project = project;
  }

  env() {
    return { ...this.station.env, TELEX_ADDRESS: this.address, TELEX_SESSION_ID: this.session, STATION_ADDRESS: this.station.address };
  }

  notifyArgs({ priority = "needs-reply", subject, body, options, wantsOutcome, replyTo, wait, to, url, tags = [] }) {
    const a = [NOTIFY, "--priority", priority, "--name", this.name, "--json", ...this.cardArgs];
    if (subject) a.push("--subject", subject);
    if (body) a.push("--body", body);
    if (options) a.push("--options", options.join("|"));
    if (wantsOutcome) a.push("--wants-outcome");
    if (replyTo) a.push("--reply-to", String(replyTo));
    if (wait) a.push("--wait", String(wait));
    if (to) a.push("--to", to);
    if (url) a.push("--url", url);
    if (this.project) a.push("--project", this.project);
    for (const t of tags) a.push("--tag", t);
    return a;
  }

  /** Send and return the Telex receipt ({id, thread_id, occupied, ...}). */
  notify(opts) {
    const r = spawnSync("node", this.notifyArgs(opts), { env: this.env(), encoding: "utf8" });
    if (r.status !== 0) throw new Error(`notify failed (${r.status}): ${r.stderr}`);
    return JSON.parse(r.stdout).sent;
  }

  /** Send and block for the human's reply, like a real blocked agent. */
  ask(opts) {
    const child = spawn("node", this.notifyArgs({ wait: 60, ...opts }), { env: this.env(), stdio: ["ignore", "pipe", "pipe"] });
    let out = "";
    let err = "";
    child.stdout.on("data", (d) => (out += d));
    child.stderr.on("data", (d) => (err += d));
    const done = new Promise((resolve, reject) => {
      child.on("exit", (code) => (code === 0 ? resolve(JSON.parse(out)) : reject(new Error(`ask exited ${code}: ${err}`))));
    });
    return { done, child };
  }

  telex(args, opts = {}) {
    return this.station.telex(args, { session: this.session, ...opts });
  }

  /** Go away entirely: no session attends this address any more. */
  leave() {
    return this.telex(["station", "stop", "--address", this.address, "--json"]);
  }

  attach() {
    return this.telex(["attach", "--address", this.address, "--description", this.name, "--json"]);
  }

  /** Take one delivery the way a pull-mode session would. */
  receive(timeoutMs = 5000) {
    const w = this.telex(["wait", "--address", this.address, "--timeout-ms", String(timeoutMs), "--json"]);
    if (w.code !== 0) return null;
    const message = w.json();
    this.telex(["ack", "--address", this.address, "--id", String(message.id), "--json"]);
    return message;
  }

  /** Hold a live waiter open so the directory shows this session listening. */
  listen(timeoutMs = 600_000) {
    const child = spawn(TELEX, ["wait", "--address", this.address, "--session", this.session, "--timeout-ms", String(timeoutMs), "--json"], {
      env: this.env(),
      stdio: "ignore",
      detached: true,
    });
    child.unref();
    return child;
  }
}
