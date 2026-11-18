// Pure view-model logic: everything the three panes derive from a snapshot.

import type { Agent, Message, Project, Snapshot, Thread, Tier, View } from "./types";

export const TIERS: Tier[] = ["blocked", "needs-reply", "routine", "fyi"];

export const TIER_LABEL: Record<Tier, string> = {
  blocked: "Blocked",
  "needs-reply": "Needs reply",
  routine: "Routine",
  fyi: "FYI",
};

/** What each tier means, in the agent's terms. Shown in tooltips and settings. */
export const TIER_MEANING: Record<Tier, string> = {
  blocked: "The agent has stopped and cannot continue without you.",
  "needs-reply": "The agent wants an answer at its next checkpoint but is still working.",
  routine: "Worth seeing when you get to it. Nothing is waiting on you.",
  fyi: "Informational. Never asks for anything.",
};

export const TIER_GLYPH: Record<Tier, string> = { blocked: "!", "needs-reply": "?", routine: "•", fyi: "i" };

const TIER_RANK: Record<Tier, number> = { blocked: 3, "needs-reply": 2, routine: 1, fyi: 0 };

export function maxTier(a: Tier | null, b: Tier | null): Tier | null {
  if (!a) return b;
  if (!b) return a;
  return TIER_RANK[a] >= TIER_RANK[b] ? a : b;
}

export function descendants(projects: Project[], id: string): Set<string> {
  const out = new Set<string>([id]);
  let grew = true;
  while (grew) {
    grew = false;
    for (const p of projects) {
      if (p.parent_id && out.has(p.parent_id) && !out.has(p.id)) {
        out.add(p.id);
        grew = true;
      }
    }
  }
  return out;
}

export function projectPath(projects: Project[], id: string | null): string[] {
  const path: string[] = [];
  let cursor = id;
  for (let guard = 0; cursor && guard < 32; guard++) {
    const p = projects.find((x) => x.id === cursor);
    if (!p) break;
    path.unshift(p.name);
    cursor = p.parent_id;
  }
  return path;
}

export function matchesFilter(thread: Thread, agent: Agent | undefined, filter: string): boolean {
  const q = filter.trim().toLowerCase();
  if (!q) return true;
  const hay = [
    thread.subject,
    thread.snippet,
    thread.agent,
    agent?.display_name,
    agent?.card.host,
    agent?.card.repo,
    agent?.card.branch,
    agent?.card.harness,
    agent?.presence.host,
  ]
    .filter(Boolean)
    .join(" ")
    .toLowerCase();
  return q.split(/\s+/).every((term) => hay.includes(term));
}

export function threadsForView(snap: Snapshot, view: View, filter = ""): Thread[] {
  const agents = new Map(snap.agents.map((a) => [a.address, a]));
  let list: Thread[];
  switch (view.kind) {
    case "needs-me":
      list = snap.threads.filter((t) => t.status === "needs-me" && (!view.tier || tierBucket(t.tier) === view.tier));
      break;
    case "fyi":
      list = snap.threads.filter((t) => t.status === "fyi");
      break;
    case "waiting":
      list = snap.threads.filter((t) => t.status === "waiting");
      break;
    case "snoozed":
      list = snap.threads.filter((t) => t.status === "snoozed");
      break;
    case "done":
      list = snap.threads.filter((t) => t.status === "done");
      break;
    case "project": {
      const ids = descendants(snap.projects, view.id);
      list = snap.threads.filter((t) => t.project_id !== null && ids.has(t.project_id));
      break;
    }
    case "unsorted":
      list = snap.threads.filter((t) => t.project_id === null);
      break;
    case "all":
      list = [...snap.threads];
      break;
    default:
      list = [];
  }
  return list.filter((t) => matchesFilter(t, agents.get(t.agent), filter)).sort(compareThreads);
}

/** FYI never sits in the inbox, so in "needs me" lists it folds into Routine. */
export function tierBucket(tier: Tier): Tier {
  return tier === "fyi" ? "routine" : tier;
}

const STATUS_RANK: Record<string, number> = { "needs-me": 0, waiting: 1, snoozed: 2, fyi: 3, done: 4 };

export function compareThreads(a: Thread, b: Thread): number {
  const s = (STATUS_RANK[a.status] ?? 9) - (STATUS_RANK[b.status] ?? 9);
  if (s !== 0) return s;
  if (a.status === "needs-me") {
    const t = TIER_RANK[b.tier] - TIER_RANK[a.tier];
    if (t !== 0) return t;
    // Within a tier, the longest-waiting agent comes first.
    return a.last_inbound_ms - b.last_inbound_ms;
  }
  return b.last_activity_ms - a.last_activity_ms;
}

export interface Section {
  key: string;
  label: string;
  tier: Tier | null;
  threads: Thread[];
}

/** Group a sorted list into the sections the middle pane renders. */
export function sections(threads: Thread[]): Section[] {
  const out: Section[] = [];
  const push = (key: string, label: string, tier: Tier | null, items: Thread[]) => {
    if (items.length) out.push({ key, label, tier, threads: items });
  };
  const needs = threads.filter((t) => t.status === "needs-me");
  for (const tier of ["blocked", "needs-reply", "routine"] as Tier[]) {
    push(tier, TIER_LABEL[tier], tier, needs.filter((t) => tierBucket(t.tier) === tier));
  }
  push("waiting", "Sent · awaiting pickup", null, threads.filter((t) => t.status === "waiting"));
  push("snoozed", "Snoozed", null, threads.filter((t) => t.status === "snoozed"));
  push("fyi", "FYI", "fyi", threads.filter((t) => t.status === "fyi"));
  push("done", "Done", null, threads.filter((t) => t.status === "done"));
  return out;
}

export interface TreeNode {
  project: Project;
  depth: number;
  children: TreeNode[];
  needsMe: number;
  topTier: Tier | null;
  agents: number;
}

/** Nested project tree; counts roll up from subfolders. */
export function projectTree(snap: Snapshot): TreeNode[] {
  const build = (parent: string | null, depth: number): TreeNode[] =>
    snap.projects
      .filter((p) => p.parent_id === parent)
      .sort((a, b) => a.order - b.order || a.name.localeCompare(b.name))
      .map((project) => {
        const children = build(project.id, depth + 1);
        const own = snap.threads.filter((t) => t.project_id === project.id && t.status === "needs-me");
        let needsMe = own.length;
        let topTier: Tier | null = own.reduce<Tier | null>((acc, t) => maxTier(acc, t.tier), null);
        let agents = snap.agents.filter((a) => a.project_id === project.id).length;
        for (const c of children) {
          needsMe += c.needsMe;
          topTier = maxTier(topTier, c.topTier);
          agents += c.agents;
        }
        return { project, depth, children, needsMe, topTier, agents };
      });
  return build(null, 0);
}

export function flattenTree(nodes: TreeNode[], collapsed: Set<string>): TreeNode[] {
  const out: TreeNode[] = [];
  const walk = (list: TreeNode[]) => {
    for (const n of list) {
      out.push(n);
      if (!collapsed.has(n.project.id)) walk(n.children);
    }
  };
  walk(nodes);
  return out;
}

export function shortAddress(address: string): string {
  const parts = address.split(/[/:]/).filter(Boolean);
  return parts[parts.length - 1] ?? address;
}

/** Long working directories keep their meaningful tail. */
export function shortPath(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  if (parts.length <= 3) return path;
  return `…/${parts.slice(-2).join("/")}`;
}

export const HARNESS_LABEL: Record<string, string> = {
  "claude-code": "Claude",
  claude: "Claude",
  codex: "Codex",
  "codex-cli": "Codex",
  copilot: "Copilot",
  "copilot-cli": "Copilot",
};

export function harnessLabel(harness: string | null | undefined): string | null {
  if (!harness) return null;
  return HARNESS_LABEL[harness] ?? harness;
}

/** The one-line "who and where" answer shown on every thread. */
export function whereParts(agent: Agent | undefined): { label: string; value: string; verified: boolean }[] {
  if (!agent) return [];
  const c = agent.card;
  const parts: { label: string; value: string; verified: boolean }[] = [];
  const harness = harnessLabel(c.harness);
  if (harness) parts.push({ label: "Harness", value: harness, verified: false });
  // Telex's own record of the occupant host outranks the self-report.
  if (agent.presence.host) parts.push({ label: "Machine", value: agent.presence.host, verified: true });
  else if (c.host) parts.push({ label: "Machine", value: c.host, verified: false });
  if (c.repo) parts.push({ label: "Repo", value: c.branch ? `${c.repo} @ ${c.branch}` : c.repo, verified: false });
  else if (c.cwd) parts.push({ label: "Folder", value: shortPath(c.cwd), verified: false });
  if (c.terminal) parts.push({ label: "Terminal", value: c.terminal, verified: false });
  else if (c.session_name) parts.push({ label: "Session", value: c.session_name, verified: false });
  return parts;
}

export const PRESENCE_LABEL: Record<string, string> = {
  listening: "Listening now",
  attached: "Attached, not listening",
  offline: "Session gone",
  unknown: "Presence unknown",
};

export const PRESENCE_HELP: Record<string, string> = {
  listening: "A reply reaches this session immediately.",
  attached: "The session is registered but not waiting. It sees a reply at its next check.",
  offline: "No session attends this address. A reply is kept durably until one re-attaches.",
  unknown: "Telex has no record of this address yet.",
};

/** Honest delivery state for a message the human sent. */
export function receiptLabel(m: Message, agent: Agent | undefined): { text: string; tone: "ok" | "wait" | "bad" | "muted" } {
  if (m.state === "sending") return { text: "Sending…", tone: "muted" };
  if (m.state === "failed") return { text: "Not sent", tone: "bad" };
  if (m.state === "indeterminate") return { text: "Unconfirmed: Telex did not answer", tone: "bad" };
  const r = m.receipt;
  if (!r) return { text: "Accepted by Telex", tone: "muted" };
  if (r.disposition === "handled") return { text: "Handled by the agent", tone: "ok" };
  if (r.disposition === "rejected") return { text: "Rejected by the agent", tone: "bad" };
  if (r.disposition) return { text: `Agent marked it ${r.disposition}`, tone: "ok" };
  if (r.consumed) return { text: "Picked up by the session", tone: "ok" };
  if (agent?.presence.state === "offline" || r.occupied_at_send === false) {
    return { text: "Queued: no session is attending. It will arrive when one attaches.", tone: "wait" };
  }
  return { text: "Delivered to the session queue, not picked up yet", tone: "wait" };
}

export function relTime(ms: number, now: number): string {
  if (!ms) return "";
  const s = Math.max(0, Math.round((now - ms) / 1000));
  if (s < 45) return "now";
  if (s < 3600) return `${Math.round(s / 60)}m`;
  if (s < 86400) return `${Math.round(s / 3600)}h`;
  if (s < 7 * 86400) return `${Math.round(s / 86400)}d`;
  return new Date(ms).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

export function untilTime(ms: number, now: number): string {
  const s = Math.max(0, Math.round((ms - now) / 1000));
  if (s < 3600) return `${Math.max(1, Math.round(s / 60))}m`;
  if (s < 86400) return `${Math.round(s / 3600)}h`;
  return new Date(ms).toLocaleDateString(undefined, { weekday: "short", hour: "numeric" });
}

export function clock(ms: number): string {
  return new Date(ms).toLocaleString(undefined, { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" });
}

export type Segment = { kind: "text"; text: string } | { kind: "link"; text: string; href: string };

/** Message bodies are untrusted: only http(s) URLs become (confirm-first) links. */
export function linkify(text: string): Segment[] {
  const out: Segment[] = [];
  const re = /https?:\/\/[^\s<>"')\]]+/g;
  let last = 0;
  for (let m = re.exec(text); m; m = re.exec(text)) {
    let href = m[0];
    const trailing = href.match(/[.,;:!?]+$/);
    if (trailing) href = href.slice(0, -trailing[0].length);
    if (m.index > last) out.push({ kind: "text", text: text.slice(last, m.index) });
    out.push({ kind: "link", text: href, href });
    last = m.index + href.length;
    re.lastIndex = last;
  }
  if (last < text.length) out.push({ kind: "text", text: text.slice(last) });
  return out;
}

export function snoozeOptions(now: number): { label: string; until: number }[] {
  const d = new Date(now);
  const tomorrow = new Date(d.getFullYear(), d.getMonth(), d.getDate() + 1, 9, 0, 0).getTime();
  const daysToMonday = ((8 - d.getDay()) % 7) || 7;
  const monday = new Date(d.getFullYear(), d.getMonth(), d.getDate() + daysToMonday, 9, 0, 0).getTime();
  return [
    { label: "30 minutes", until: now + 30 * 60_000 },
    { label: "2 hours", until: now + 2 * 3_600_000 },
    { label: "Tomorrow 9:00", until: tomorrow },
    { label: "Monday 9:00", until: monday },
  ];
}

export function viewTitle(snap: Snapshot, view: View): string {
  switch (view.kind) {
    case "needs-me":
      return view.tier ? TIER_LABEL[view.tier] : "Needs me";
    case "fyi":
      return "FYI";
    case "waiting":
      return "Sent · awaiting pickup";
    case "snoozed":
      return "Snoozed";
    case "done":
      return "Done";
    case "all":
      return "All threads";
    case "unsorted":
      return "Unsorted";
    case "agents":
      return "Agents";
    case "project":
      return projectPath(snap.projects, view.id).join(" › ") || "Project";
  }
}

export function focusLabel(snap: Snapshot): string | null {
  const f = snap.settings.focus;
  if (f.mode === "off") return null;
  if (f.until_ms !== null && f.until_ms <= snap.now_ms) return null;
  const what = f.mode === "blocked-only" ? "Only Blocked interrupts" : "Silent";
  return f.until_ms ? `${what} · ${untilTime(f.until_ms, snap.now_ms)} left` : what;
}
