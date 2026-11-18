import { describe, expect, it } from "vitest";
import {
  compareThreads,
  descendants,
  linkify,
  projectTree,
  receiptLabel,
  sections,
  shortPath,
  threadsForView,
  whereParts,
} from "./model";
import type { Agent, Message, Project, Snapshot, Thread } from "./types";

const card = { name: null, harness: null, model: null, host: null, os: null, cwd: null, repo: null, branch: null, session_id: null, session_name: null, terminal: null, open_command: null, project_hint: null, url: null, tags: [] };

function thread(id: number, over: Partial<Thread> = {}): Thread {
  return {
    thread_id: id, subject: `s${id}`, agent: "agent:a", station_address: "operator:rob", status: "needs-me", tier: "routine",
    unread: 0, open_obligations: 0, last_activity_ms: id, last_inbound_ms: id, snippet: "", project_id: null, project_source: "none",
    snooze_until_ms: null, pinned: false, muted: false, cc_only: false, messages: [], ...over,
  };
}

function agent(over: Partial<Agent> = {}): Agent {
  return {
    address: "agent:a", display_name: "a", card: { ...card }, nickname: null, notes: null, locator_note: null, project_id: null,
    project_source: "none", notify: "inherit", presence: { state: "unknown", detail: null, host: null, description: null, tags: null, checked_at_ms: 0 },
    first_seen_ms: 0, last_seen_ms: 0, thread_ids: [], needs_me: 0, top_tier: null, ...over,
  };
}

function snap(threads: Thread[], projects: Project[] = [], agents: Agent[] = [agent()]): Snapshot {
  return { threads, projects, agents } as unknown as Snapshot;
}

const project = (id: string, parent: string | null = null): Project => ({ id, name: id, parent_id: parent, color: null, notify: "inherit", order: 0 });

describe("ordering", () => {
  it("puts blocked first and, within a tier, the longest-waiting agent first", () => {
    const list = [
      thread(1, { tier: "routine", last_inbound_ms: 10 }),
      thread(2, { tier: "blocked", last_inbound_ms: 50 }),
      thread(3, { tier: "blocked", last_inbound_ms: 20 }),
      thread(4, { tier: "needs-reply", last_inbound_ms: 5 }),
      thread(5, { status: "done", last_activity_ms: 999 }),
    ].sort(compareThreads);
    expect(list.map((t) => t.thread_id)).toEqual([3, 2, 4, 1, 5]);
  });

  it("groups into the sections the list renders", () => {
    const groups = sections([
      thread(1, { tier: "blocked" }),
      thread(2, { tier: "fyi" }), // an FYI that is flagged still sits under Routine
      thread(3, { status: "waiting" }),
      thread(4, { status: "fyi" }),
    ]);
    expect(groups.map((g) => [g.key, g.threads.length])).toEqual([["blocked", 1], ["routine", 1], ["waiting", 1], ["fyi", 1]]);
  });
});

describe("views", () => {
  const projects = [project("telex"), project("station", "telex"), project("paw")];
  const s = snap(
    [thread(1, { project_id: "telex" }), thread(2, { project_id: "station", tier: "blocked" }), thread(3, { project_id: "paw", status: "done" }), thread(4)],
    projects,
    [agent({ card: { ...card, host: "devbox-2", repo: "lossyrob/telex" } })],
  );

  it("a project view includes its subfolders", () => {
    expect([...descendants(projects, "telex")].sort()).toEqual(["station", "telex"]);
    expect(threadsForView(s, { kind: "project", id: "telex" }).map((t) => t.thread_id)).toEqual([2, 1]);
  });

  it("unsorted, tier and text filters", () => {
    expect(threadsForView(s, { kind: "unsorted" }).map((t) => t.thread_id)).toEqual([4]);
    expect(threadsForView(s, { kind: "needs-me", tier: "blocked" }).map((t) => t.thread_id)).toEqual([2]);
    expect(threadsForView(s, { kind: "all" }, "devbox-2 s3").map((t) => t.thread_id)).toEqual([3]);
    expect(threadsForView(s, { kind: "all" }, "nomatch")).toEqual([]);
  });

  it("counts roll up the tree with the most urgent tier", () => {
    const tree = projectTree(s);
    const telex = tree.find((n) => n.project.id === "telex")!;
    expect(telex.needsMe).toBe(2);
    expect(telex.topTier).toBe("blocked");
    expect(telex.children[0].needsMe).toBe(1);
    expect(tree.find((n) => n.project.id === "paw")!.needsMe).toBe(0);
  });
});

describe("who and where", () => {
  it("prefers the host telex recorded over the self-report and marks provenance", () => {
    const a = agent({
      card: { ...card, harness: "claude-code", host: "claims-devbox", repo: "o/r", branch: "b", terminal: "tmux 1" },
      presence: { state: "listening", detail: null, host: "real-host", description: null, tags: null, checked_at_ms: 0 },
    });
    expect(whereParts(a)).toEqual([
      { label: "Harness", value: "Claude", verified: false },
      { label: "Machine", value: "real-host", verified: true },
      { label: "Repo", value: "o/r @ b", verified: false },
      { label: "Terminal", value: "tmux 1", verified: false },
    ]);
  });

  it("keeps the tail of a long folder", () => {
    expect(shortPath("/a/b/c/d/e")).toBe("…/d/e");
    expect(shortPath("C:\\src\\telex")).toBe("C:\\src\\telex");
  });
});

describe("receipts say what is actually known", () => {
  const base = { direction: "outbound", state: "accepted" } as Message;
  const receipt = (over: Partial<NonNullable<Message["receipt"]>>) => ({ ...base, receipt: { occupied_at_send: true, consumed: false, disposition: null, refreshed_at_ms: 0, ...over } }) as Message;

  it("distinguishes queued, delivered, picked up and handled", () => {
    const offline = agent({ presence: { state: "offline", detail: null, host: null, description: null, tags: null, checked_at_ms: 0 } });
    expect(receiptLabel(receipt({}), offline).tone).toBe("wait");
    expect(receiptLabel(receipt({}), offline).text).toMatch(/^Queued/);
    expect(receiptLabel(receipt({}), agent()).text).toMatch(/not picked up yet/);
    expect(receiptLabel(receipt({ consumed: true }), agent()).text).toBe("Picked up by the session");
    expect(receiptLabel(receipt({ consumed: true, disposition: "handled" }), agent()).tone).toBe("ok");
    expect(receiptLabel({ ...base, state: "indeterminate" } as Message, agent()).tone).toBe("bad");
  });
});

describe("untrusted text", () => {
  it("only links http(s) and never emits markup", () => {
    const segs = linkify('see https://example.com/a?b=1. <script>x</script> javascript:alert(1) file:///etc/passwd');
    expect(segs.filter((s) => s.kind === "link").map((s) => s.text)).toEqual(["https://example.com/a?b=1"]);
    expect(segs.map((s) => s.text).join("")).toBe('see https://example.com/a?b=1. <script>x</script> javascript:alert(1) file:///etc/passwd');
  });
});
