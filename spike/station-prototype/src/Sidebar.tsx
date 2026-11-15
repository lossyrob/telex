import { useState } from "react";
import { Menu, notifyItems, useStation, type MenuItem } from "./bits";
import { flattenTree, focusLabel, projectTree, TIER_GLYPH, type TreeNode } from "./model";
import type { Tier, View } from "./types";

const DRAG_THREAD = "application/x-station-thread";
const DRAG_AGENT = "application/x-station-agent";

function sameView(a: View, b: View): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

function Row({
  view,
  label,
  count,
  glyph,
  tier,
  indent = 0,
  strong = false,
}: {
  view: View;
  label: string;
  count?: number;
  glyph?: string;
  tier?: Tier;
  indent?: number;
  strong?: boolean;
}) {
  const { view: current, setView } = useStation();
  const active = sameView(current, view);
  return (
    <button
      type="button"
      className={`side-row ${active ? "active" : ""} ${strong ? "strong" : ""}`}
      style={{ paddingLeft: 12 + indent * 14 }}
      onClick={() => setView(view)}
    >
      <span className={`side-glyph ${tier ? `tier-${tier}` : ""}`}>{glyph ?? ""}</span>
      <span className="side-label">{label}</span>
      {count ? <span className={`side-count ${tier ? `tier-${tier}` : ""}`}>{count}</span> : null}
    </button>
  );
}

function ProjectRow({
  node,
  collapsed,
  toggle,
}: {
  node: TreeNode;
  collapsed: boolean;
  toggle: () => void;
}) {
  const { view, setView, act, snap, notice } = useStation();
  const [over, setOver] = useState(false);
  const [renaming, setRenaming] = useState(false);
  const [adding, setAdding] = useState(false);
  const [draft, setDraft] = useState("");
  const p = node.project;
  const active = view.kind === "project" && view.id === p.id;

  const drop = (e: React.DragEvent) => {
    e.preventDefault();
    setOver(false);
    const agent = e.dataTransfer.getData(DRAG_AGENT);
    const thread = e.dataTransfer.getData(DRAG_THREAD);
    // Dropping a thread files its agent, so future threads follow. Hold Option
    // to file only that one thread.
    if (thread && (e.altKey || !agent)) {
      void act("thread_update", { thread_id: Number(thread), project_id: p.id });
      notice(`Filed this thread in ${p.name}`);
    } else if (agent) {
      void act("agent_update", { address: agent, project_id: p.id });
      const name = snap.agents.find((a) => a.address === agent)?.display_name ?? agent;
      notice(`Filed ${name} in ${p.name}. Its threads follow.`);
    }
  };

  const items: MenuItem[] = [
    { label: "New subfolder…", onSelect: () => { setDraft(""); setAdding(true); } },
    { label: "Rename…", onSelect: () => { setDraft(p.name); setRenaming(true); } },
    { separator: true, label: "" },
    { heading: true, label: "Notifications from this project" },
    ...notifyItems(p.notify, (n) => void act("project_update", { id: p.id, notify: n })),
    { separator: true, label: "" },
    {
      label: "Delete project",
      danger: true,
      onSelect: () => {
        if (window.confirm(`Delete “${p.name}”? Its agents and subfolders move up one level.`)) {
          void act("project_delete", { id: p.id });
        }
      },
    },
  ];

  return (
    <>
      <div
        className={`side-row project-row ${active ? "active" : ""} ${over ? "drop-over" : ""}`}
        style={{ paddingLeft: 6 + node.depth * 14 }}
        onDragOver={(e) => {
          if (e.dataTransfer.types.includes(DRAG_AGENT) || e.dataTransfer.types.includes(DRAG_THREAD)) {
            e.preventDefault();
            setOver(true);
          }
        }}
        onDragLeave={() => setOver(false)}
        onDrop={drop}
      >
        <button
          type="button"
          className="twisty"
          onClick={toggle}
          aria-label={collapsed ? "Expand" : "Collapse"}
          style={{ visibility: node.children.length ? "visible" : "hidden" }}
        >
          {collapsed ? "▸" : "▾"}
        </button>
        {renaming ? (
          <input
            className="side-input"
            autoFocus
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onBlur={() => setRenaming(false)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                void act("project_update", { id: p.id, name: draft });
                setRenaming(false);
              }
              if (e.key === "Escape") setRenaming(false);
            }}
          />
        ) : (
          <button type="button" className="side-main" onClick={() => setView({ kind: "project", id: p.id })}>
            <span className="side-label">{p.name}</span>
            {p.notify !== "inherit" && (
              <span className="side-flag" title={`Notification override: ${p.notify}`}>
                {p.notify === "mute" ? "🔕" : p.notify === "elevate" ? "▲" : "!"}
              </span>
            )}
            {node.needsMe > 0 && node.topTier && (
              <span className={`side-count tier-${node.topTier}`} title={`${node.needsMe} need you`}>
                {TIER_GLYPH[node.topTier]} {node.needsMe}
              </span>
            )}
          </button>
        )}
        <Menu button="⋯" items={items} className="btn-ghost side-more" align="right" title="Project options" />
      </div>
      {adding && (
        <div className="side-row" style={{ paddingLeft: 26 + (node.depth + 1) * 14 }}>
          <input
            className="side-input"
            autoFocus
            placeholder="Subfolder name"
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onBlur={() => setAdding(false)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && draft.trim()) {
                void act("project_create", { name: draft, parent_id: p.id });
                setAdding(false);
              }
              if (e.key === "Escape") setAdding(false);
            }}
          />
        </div>
      )}
    </>
  );
}

export function Sidebar({
  onOrganize,
  onSettings,
  onCompose,
}: {
  onOrganize: () => void;
  onSettings: () => void;
  onCompose: () => void;
}) {
  const { snap, act, view, setView } = useStation();
  const [collapsed, setCollapsed] = useState<Set<string>>(() => {
    try {
      return new Set(JSON.parse(localStorage.getItem("station.collapsed") ?? "[]") as string[]);
    } catch {
      return new Set();
    }
  });
  const [adding, setAdding] = useState(false);
  const [draft, setDraft] = useState("");
  const [overUnsorted, setOverUnsorted] = useState(false);
  const c = snap.counts;
  const nodes = flattenTree(projectTree(snap), collapsed);
  const focus = focusLabel(snap);
  const unhealthy =
    !snap.station.connected || snap.health.some((h) => !h.registered || !h.receive_ready || h.last_error || h.degraded);

  const toggle = (id: string) => {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      localStorage.setItem("station.collapsed", JSON.stringify([...next]));
      return next;
    });
  };

  const focusItems: MenuItem[] = [
    { heading: true, label: "What may interrupt me" },
    { label: "Everything my settings allow", checked: !focus, onSelect: () => void act("set_focus", { mode: "off" }) },
    { label: "Only Blocked · for 1 hour", onSelect: () => void act("set_focus", { mode: "blocked-only", minutes: 60 }) },
    { label: "Only Blocked · until I turn it off", onSelect: () => void act("set_focus", { mode: "blocked-only" }) },
    { label: "Nothing · for 30 minutes", onSelect: () => void act("set_focus", { mode: "silent", minutes: 30 }) },
    { label: "Nothing · for 2 hours", onSelect: () => void act("set_focus", { mode: "silent", minutes: 120 }) },
    { separator: true, label: "" },
    { label: "Notification settings…", onSelect: onSettings },
  ];

  return (
    <nav className="sidebar">
      <div className="side-head">
        <div className="brand">
          <span className={`health-dot ${unhealthy ? "bad" : "ok"}`} title={unhealthy ? "Station needs attention. Open Settings › Health." : "Attending and healthy"} />
          <span className="brand-name">Station</span>
          <span className="brand-sub">{snap.settings.addresses.map((a) => a.address).join(", ") || "no address"}</span>
        </div>
        <button type="button" className="btn btn-ghost" title="New message to an agent (n)" onClick={onCompose}>
          ✎
        </button>
      </div>

      <Menu
        className={`focus-btn ${focus ? "focus-on" : ""}`}
        button={
          <>
            <span>{focus ? "◐" : "◯"}</span>
            <span className="focus-text">{focus ?? "Interruptions: normal"}</span>
          </>
        }
        items={focusItems}
      />

      <div className="side-scroll">
        <Row view={{ kind: "needs-me" }} label="Needs me" count={c.needs_me} strong />
        <Row view={{ kind: "needs-me", tier: "blocked" }} label="Blocked" glyph="!" tier="blocked" count={c.blocked} indent={1} />
        <Row view={{ kind: "needs-me", tier: "needs-reply" }} label="Needs reply" glyph="?" tier="needs-reply" count={c.needs_reply} indent={1} />
        <Row view={{ kind: "needs-me", tier: "routine" }} label="Routine" glyph="•" tier="routine" count={c.routine} indent={1} />
        <Row view={{ kind: "fyi" }} label="FYI" glyph="i" tier="fyi" count={c.fyi_unread} />
        <Row view={{ kind: "waiting" }} label="Sent · awaiting pickup" glyph="↗" count={c.waiting} />
        <Row view={{ kind: "snoozed" }} label="Snoozed" glyph="☾" count={c.snoozed} />
        <Row view={{ kind: "done" }} label="Done" glyph="✓" />
        <Row view={{ kind: "all" }} label="All threads" glyph="≡" />

        <div className="side-section">
          <span>Projects</span>
          <button type="button" className="btn btn-ghost" title="New project" onClick={() => { setDraft(""); setAdding(true); }}>
            ＋
          </button>
        </div>
        {nodes.map((node) => (
          <ProjectRow
            key={node.project.id}
            node={node}
            collapsed={collapsed.has(node.project.id)}
            toggle={() => toggle(node.project.id)}
          />
        ))}
        {adding && (
          <div className="side-row" style={{ paddingLeft: 26 }}>
            <input
              className="side-input"
              autoFocus
              placeholder="Project name"
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              onBlur={() => setAdding(false)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && draft.trim()) {
                  void act("project_create", { name: draft });
                  setAdding(false);
                }
                if (e.key === "Escape") setAdding(false);
              }}
            />
          </div>
        )}
        {snap.projects.length === 0 && !adding && (
          <div className="side-hint">No projects yet. Use Organize to create them from your agents in one click.</div>
        )}
        <div
          className={`side-row project-row ${view.kind === "unsorted" ? "active" : ""} ${overUnsorted ? "drop-over" : ""}`}
          style={{ paddingLeft: 6 }}
          onDragOver={(e) => {
            if (e.dataTransfer.types.includes(DRAG_AGENT)) {
              e.preventDefault();
              setOverUnsorted(true);
            }
          }}
          onDragLeave={() => setOverUnsorted(false)}
          onDrop={(e) => {
            e.preventDefault();
            setOverUnsorted(false);
            const agent = e.dataTransfer.getData(DRAG_AGENT);
            if (agent) void act("agent_update", { address: agent, project_id: null });
          }}
        >
          <span className="twisty" style={{ visibility: "hidden" }}>▸</span>
          <button type="button" className="side-main" onClick={() => setView({ kind: "unsorted" })}>
            <span className="side-label muted">Unsorted</span>
            {c.unsorted_agents > 0 && <span className="side-count">{c.unsorted_agents}</span>}
          </button>
        </div>
        <button type="button" className={`organize-btn ${snap.suggestions.length ? "has-suggestions" : ""}`} onClick={onOrganize}>
          <span>Organize…</span>
          {snap.suggestions.length > 0 && <span className="pill">{snap.suggestions.length} suggestions</span>}
        </button>

        <div className="side-section">
          <span>Fleet</span>
        </div>
        <Row view={{ kind: "agents" }} label="Agents" glyph="◎" count={snap.agents.length} />
      </div>

      <div className="side-foot">
        <button type="button" className="btn btn-ghost" onClick={onSettings}>
          ⚙ Settings
        </button>
        {snap.station.sandbox && <span className="pill" title="Running against an isolated Telex sandbox">sandbox</span>}
      </div>
    </nav>
  );
}

export { DRAG_AGENT, DRAG_THREAD };
