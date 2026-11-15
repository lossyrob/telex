#!/usr/bin/env node
// Populate a running Station with a believable fleet: agents on several
// machines and harnesses sending real Telex messages through the agent kit.
//
//   node scenarios/demo-seed.mjs            # against the default ~/.telex-station
//   STATION_HOME=/path node scenarios/demo-seed.mjs

import os from "node:os";
import path from "node:path";
import { Agent, Station, sleep } from "./lib.mjs";

const home = process.env.STATION_HOME || path.join(os.homedir(), ".telex-station");
const station = new Station({ home, port: Number(process.env.STATION_PORT_HTTP || 4317), address: process.env.STATION_ADDRESS || `operator:${os.userInfo().username}` });

const fleet = {
  impl: new Agent(station, {
    address: "telex://lossyrob/telex/T-A:station-impl-134", name: "station-impl-134", harness: "claude-code", host: "devbox-2", os: "linux",
    repo: "lossyrob/telex", branch: "feature/station-app", cwd: "/home/rob/proj/telex", terminal: "tmux main:3.1",
    open: "ssh devbox-2 -t tmux attach -t main", project: "telex/station",
  }),
  review: new Agent(station, {
    address: "telex://lossyrob/telex/T-A:station-review-134", name: "station-review-134", harness: "codex", host: "devbox-1", os: "linux",
    repo: "lossyrob/telex", branch: "feature/station-app", cwd: "/home/rob/review/telex", terminal: "tmux review:1.0",
    open: "ssh devbox-1 -t tmux attach -t review", project: "telex/station",
  }),
  watcher: new Agent(station, {
    address: "telex://lossyrob/telex/T-A:watcher-mac-172", name: "watcher-mac-172", harness: "claude-code", host: "macbook", os: "darwin",
    repo: "lossyrob/telex", branch: "claude/telex-issue-172", cwd: "/Users/rob/proj/telex/telex", terminal: "Claude desktop",
  }),
  docs: new Agent(station, {
    address: "telex://lossyrob/telex/T-A:docs-sweep", name: "docs-sweep", harness: "copilot", host: "devbox-3", os: "win32",
    repo: "lossyrob/telex", branch: "docs/no-version-gating", cwd: "C:\\src\\telex", terminal: "Windows Terminal · tab 2",
  }),
  paw: new Agent(station, {
    address: "telex://lossyrob/paw/planner", name: "paw-planner", harness: "claude-code", host: "devbox-2", os: "linux",
    repo: "lossyrob/paw", branch: "main", cwd: "/home/rob/proj/paw", terminal: "tmux paw:0.0", open: "ssh devbox-2 -t tmux attach -t paw",
  }),
  pawReview: new Agent(station, {
    address: "telex://lossyrob/paw/reviewer", name: "paw-reviewer", harness: "codex", host: "devbox-1", os: "linux",
    repo: "lossyrob/paw", branch: "main", cwd: "/home/rob/proj/paw", terminal: "tmux paw:1.0",
  }),
  infra: new Agent(station, {
    address: "infra:terraform-eastus", name: "terraform-eastus", harness: "copilot", host: "cloud-vm-eastus", os: "linux",
    repo: "lossyrob/infra", branch: "main", cwd: "/srv/infra", terminal: "ssh session", open: "ssh ops@cloud-vm-eastus",
  }),
  nightly: new Agent(station, {
    address: "ci:nightly", name: "nightly-build", harness: "ci", host: "devbox-3", os: "win32", repo: "lossyrob/streamliner", branch: "main",
  }),
};

await station.waitFor("station up", (s) => s.station.connected, 20_000);

fleet.nightly.notify({ priority: "fyi", subject: "Nightly build is green", body: "All 6 jobs passed in 14m 02s. No flaky retries." });
fleet.docs.notify({
  priority: "routine", subject: "Docs sweep: pass 2 finished",
  body: "Rewrote the macOS admission wording in 7 files to drop version-number gating.\n\nPR: https://github.com/lossyrob/telex/pull/254\nNothing needs you unless you want to skim the diff.",
  url: "https://github.com/lossyrob/telex/pull/254",
});
fleet.watcher.notify({
  priority: "routine", subject: "Rebased onto main",
  body: "Rebased claude/telex-issue-172 onto main. Two conflicts in src/daemon.rs, both resolved in favour of main. Tests pass locally.",
});
fleet.pawReview.notify({ priority: "fyi", subject: "Review queue empty", body: "No open review requests. Idling." });
await sleep(300);
fleet.paw.notify({
  priority: "needs-reply", subject: "Which reviewer model for PAW v3?",
  body: "The v3 plan needs a default reviewer. Options I see:\n\n1. Keep general-reviewer on the current Opus\n2. Split security review to a second model\n\nI'll keep drafting the rest of the plan and pick this up at my next checkpoint.",
  options: ["Keep one reviewer", "Split security review", "Draft both, I'll compare"],
});
fleet.review.notify({
  priority: "needs-reply", subject: "Approve the Station receive-loop plan?", wantsOutcome: true,
  body: "Plan review for #134 is ready. One open question: should ack happen before or after the notification decision?\n\nMy read of the contract says after durable ingest and independent of notification, which is what the plan does.",
  options: ["Approve", "Approve with changes", "Hold"],
});
await sleep(300);
fleet.infra.notify({
  priority: "blocked", subject: "terraform apply wants to destroy 3 resources",
  body: "`terraform plan` for eastus shows 3 to destroy:\n\n- azurerm_postgresql_flexible_server.telex_pg\n- azurerm_private_endpoint.pg\n- azurerm_dns_a_record.pg\n\nThis looks like a rename, but I will not apply a destroy on the Telex Postgres server without you. Waiting.",
  options: ["Do not apply", "Apply: it's the rename", "Show me the full plan"],
});
fleet.impl.notify({
  priority: "blocked", subject: "CI is red: Postgres credentials expired",
  body: "The conformance job fails at connect: `password authentication failed for user telex_ci`.\n\nI can't rotate the secret myself. I'm stopped until the TELEX_PG_URL secret is refreshed or you tell me to skip the Postgres leg.",
  options: ["Rotated, re-run", "Skip the Postgres leg", "Use the vault entry pg-ci"],
});

// Presence variety: one live waiter, one session gone.
fleet.impl.listen();
fleet.paw.listen();
fleet.docs.leave();
fleet.nightly.leave();

const snap = await station.waitFor("all seeded", (s) => (s.threads.length >= 8 ? s : null), 20_000);
console.log(`seeded ${snap.threads.length} threads from ${snap.agents.length} agents; needs me: ${snap.counts.needs_me} (blocked ${snap.counts.blocked})`);
