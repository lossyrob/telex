#!/usr/bin/env node
// station-notify: how an agent session reaches its human through Telex Station.
//
// Replaces "call a script that posts to Teams". It sends one ordinary Telex
// message to the operator's Station address, attaches a self-reported card
// (harness, machine, repo, branch, folder, terminal, session) so the human can
// see who is asking and where the session lives, and can optionally block for
// the reply.
//
//   station-notify --priority blocked --subject "CI creds expired" \
//       --body "Need the PG password to continue" --options "Use vault|Skip" --wait 900
//
// Runs anywhere Node runs (macOS, Linux, Windows), which is everywhere Claude
// Code, Codex and Copilot CLI run. Requires `telex` on PATH or $TELEX_BIN.

import { spawnSync } from "node:child_process";
import os from "node:os";

const PRIORITY = {
  blocked: "interrupt", // I have stopped and cannot continue without you
  "needs-reply": "next-checkpoint", // answer when you can; I am still working
  routine: "background", // worth knowing; nothing waits on you
  fyi: "fyi", // informational only
};

function usage(code = 0) {
  console.error(`usage: station-notify [options]
  --priority <blocked|needs-reply|routine|fyi>   default: needs-reply
  --subject <text>            short headline
  --body <text>               message (or pipe it on stdin)
  --options "A|B|C"           suggested one-click replies
  --wants-outcome             ask the human to record handled/declined
  --wait [seconds]            block for the reply in this thread (default 900)
  --reply-to <message-id>     continue an existing thread
  --to <address>              Station address (default $STATION_ADDRESS or operator:$USER)
  --from <address>            this agent's address (default $TELEX_ADDRESS)
  --name <text>               display name for this agent
  --project <path>            filing hint, e.g. "telex/station"
  --url <url>                 a link worth opening (PR, run, doc)
  --tag <text>                repeatable
  --card key=value            override a self-reported field (repeatable)
  --json                      print machine-readable results`);
  process.exit(code);
}

const args = process.argv.slice(2);
const opt = { priority: "needs-reply", tags: [], card: {}, wait: null };
for (let i = 0; i < args.length; i++) {
  const a = args[i];
  const next = () => {
    if (i + 1 >= args.length) usage(64);
    return args[++i];
  };
  if (a === "--priority") opt.priority = next();
  else if (a === "--subject") opt.subject = next();
  else if (a === "--body") opt.body = next();
  else if (a === "--options") opt.options = next().split("|").map((s) => s.trim()).filter(Boolean);
  else if (a === "--wants-outcome") opt.wantsOutcome = true;
  else if (a === "--wait") opt.wait = args[i + 1] && /^\d+$/.test(args[i + 1]) ? Number(next()) : 900;
  else if (a === "--reply-to") opt.replyTo = next();
  else if (a === "--to") opt.to = next();
  else if (a === "--from") opt.from = next();
  else if (a === "--name") opt.name = next();
  else if (a === "--project") opt.project = next();
  else if (a === "--url") opt.url = next();
  else if (a === "--tag") opt.tags.push(next());
  else if (a === "--card") {
    const [k, ...v] = next().split("=");
    opt.card[k] = v.join("=");
  } else if (a === "--json") opt.json = true;
  else if (a === "-h" || a === "--help") usage(0);
  else usage(64);
}
if (!(opt.priority in PRIORITY)) usage(64);

const TELEX = process.env.TELEX_BIN || "telex";
const env = process.env;
const from = opt.from || env.TELEX_ADDRESS;
if (!from) {
  console.error("station-notify: set $TELEX_ADDRESS (this agent's Telex address) or pass --from");
  process.exit(64);
}
const to = opt.to || env.STATION_ADDRESS || `operator:${env.USER || env.USERNAME || "operator"}`;
const session = env.TELEX_SESSION_ID || `station-notify:${from}`;

function sh(cmd, argv) {
  const r = spawnSync(cmd, argv, { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] });
  return r.status === 0 ? r.stdout.trim() : "";
}

function detectHarness() {
  if (env.STATION_HARNESS) return env.STATION_HARNESS;
  if (env.CLAUDECODE || env.CLAUDE_CODE_ENTRYPOINT) return "claude-code";
  if (env.CODEX_HOME || env.CODEX_SANDBOX || env.CODEX_MANAGED_BY_NPM) return "codex";
  if (Object.keys(env).some((k) => k.startsWith("COPILOT_") || k.startsWith("GITHUB_COPILOT"))) return "copilot";
  return undefined;
}

function detectRepo() {
  const url = sh("git", ["config", "--get", "remote.origin.url"]);
  const m = url.match(/[:/]([^/:]+\/[^/]+?)(?:\.git)?$/);
  return m ? m[1] : undefined;
}

function card() {
  const host = os.hostname().split(".")[0];
  const tmux = env.TMUX ? sh("tmux", ["display-message", "-p", "#S:#I.#P"]) : "";
  const terminal = tmux ? `tmux ${tmux}` : env.TERM_PROGRAM || undefined;
  const remote = env.SSH_CONNECTION ? `ssh ${host}` : "";
  const attach = tmux ? `tmux attach -t ${tmux.split(":")[0]}` : "";
  const open = [remote && attach ? `${remote} -t ${attach}` : remote || attach].filter(Boolean)[0];
  const c = {
    name: opt.name || env.STATION_AGENT_NAME,
    harness: detectHarness(),
    model: env.ANTHROPIC_MODEL || env.OPENAI_MODEL || undefined,
    host,
    os: process.platform,
    cwd: process.cwd(),
    repo: detectRepo(),
    branch: sh("git", ["rev-parse", "--abbrev-ref", "HEAD"]) || undefined,
    session_id: env.CLAUDE_SESSION_ID || env.COPILOT_SESSION_ID || env.TELEX_SESSION_ID || undefined,
    terminal,
    open_command: open,
    project_hint: opt.project || env.STATION_PROJECT,
    url: opt.url,
    tags: opt.tags.length ? opt.tags : undefined,
    ...opt.card,
  };
  for (const k of Object.keys(c)) if (c[k] === undefined || c[k] === "") delete c[k];
  return c;
}

function telex(argv, input) {
  const r = spawnSync(TELEX, argv, { encoding: "utf8", input, stdio: ["pipe", "pipe", "pipe"] });
  if (r.error) {
    console.error(`station-notify: cannot run ${TELEX}: ${r.error.message}`);
    process.exit(69);
  }
  return r;
}

function readStdin() {
  if (process.stdin.isTTY) return "";
  try {
    return spawnSync(process.platform === "win32" ? "more" : "cat", [], { stdio: [0, "pipe", "ignore"], encoding: "utf8" }).stdout;
  } catch {
    return "";
  }
}

const body = (opt.body ?? readStdin()).trim();
if (!body) {
  console.error("station-notify: empty message (use --body or pipe text on stdin)");
  process.exit(64);
}

const self = card();
// Attaching is what makes replies routable back to this session, and it records
// a description the Telex directory itself holds.
const description = [self.name || from, self.harness, self.repo && `${self.repo}${self.branch ? `@${self.branch}` : ""}`]
  .filter(Boolean)
  .join(" · ");
const attach = telex(["attach", "--address", from, "--session", session, "--description", description, "--json"]);
if (attach.status !== 0) {
  console.error(`station-notify: attach failed: ${attach.stderr.trim()}`);
  process.exit(70);
}

const metadata = JSON.stringify({ station: { v: 0, card: self, ...(opt.options ? { replies: opt.options } : {}) } });
const send = opt.replyTo
  ? ["reply", "--to-message", String(opt.replyTo), "--address", from, "--from", from, "--session", session]
  : ["send", "--to", to, "--from", from, "--session", session, "--metadata", metadata];
send.push("--attention", PRIORITY[opt.priority], "--body-stdin", "--json");
if (opt.subject) send.push("--subject", opt.subject);
if (opt.wantsOutcome || opt.priority === "blocked") send.push("--requires-disposition");
const sent = telex(send, body);
if (sent.status !== 0) {
  console.error(`station-notify: send failed: ${sent.stderr.trim()}`);
  process.exit(70);
}
const receipt = JSON.parse(sent.stdout);
if (opt.wait === null) {
  if (opt.json) console.log(JSON.stringify({ sent: receipt }));
  else console.log(`sent #${receipt.id} to ${to} (${receipt.occupied ? "Station is attending" : "queued: no Station attending right now"})`);
  process.exit(0);
}

// Block for the human's reply in this thread. Other traffic is left queued for
// the session's normal Telex loop.
const deadline = Date.now() + opt.wait * 1000;
for (;;) {
  const remaining = deadline - Date.now();
  if (remaining <= 0) {
    console.error(`station-notify: no reply within ${opt.wait}s (message #${receipt.id} stays in the operator's queue)`);
    process.exit(2);
  }
  const w = telex(["wait", "--address", from, "--session", session, "--timeout-ms", String(Math.min(remaining, 60_000)), "--json"]);
  if (w.status === 2) continue;
  if (w.status === 3 || w.status === 5) {
    telex(["attach", "--address", from, "--session", session, "--description", description, "--json"]);
    continue;
  }
  if (w.status !== 0) {
    console.error(`station-notify: wait failed (${w.status}): ${w.stderr.trim()}`);
    process.exit(70);
  }
  const message = JSON.parse(w.stdout);
  telex(["ack", "--address", from, "--session", session, "--id", String(message.id), "--json"]);
  if (message.thread_id !== receipt.thread_id) {
    console.error(`station-notify: received unrelated message #${message.id}; still waiting`);
    continue;
  }
  telex(["handle", "--address", from, "--session", session, "--id", String(message.id), "--note", "read by agent", "--json"]);
  if (opt.json) console.log(JSON.stringify({ sent: receipt, reply: { id: message.id, from: message.from, body: message.body } }));
  else console.log(message.body);
  process.exit(0);
}
