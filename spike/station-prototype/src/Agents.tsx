import { useState } from "react";
import { Empty, HarnessTag, PresenceDot, TierMark, useStation } from "./bits";
import { harnessLabel, projectPath, relTime } from "./model";
import { DRAG_AGENT } from "./Sidebar";
import { AgentCard } from "./ThreadPane";
import type { Agent } from "./types";

type GroupBy = "project" | "machine" | "harness";

function groupKey(agent: Agent, by: GroupBy, path: (id: string | null) => string): string {
  if (by === "machine") return agent.presence.host ?? agent.card.host ?? "Unknown machine";
  if (by === "harness") return harnessLabel(agent.card.harness) ?? "Unknown harness";
  return path(agent.project_id) || "Unsorted";
}

/** The fleet roster: every agent, grouped the way the operator is thinking. */
export function AgentList({ selected, onSelect }: { selected: string | null; onSelect: (address: string) => void }) {
  const { snap } = useStation();
  const [by, setBy] = useState<GroupBy>("machine");
  const path = (id: string | null) => projectPath(snap.projects, id).join(" › ");
  const groups = new Map<string, Agent[]>();
  for (const a of snap.agents) {
    const key = groupKey(a, by, path);
    groups.set(key, [...(groups.get(key) ?? []), a]);
  }
  const keys = [...groups.keys()].sort((a, b) => a.localeCompare(b));
  return (
    <section className="list-pane">
      <header className="list-head">
        <h1>Agents</h1>
        <span className="list-count">{snap.agents.length}</span>
      </header>
      <div className="list-filter">
        <div className="segmented">
          {(["machine", "project", "harness"] as GroupBy[]).map((g) => (
            <button type="button" key={g} className={by === g ? "on" : ""} onClick={() => setBy(g)}>
              By {g}
            </button>
          ))}
        </div>
      </div>
      <div className="list-scroll">
        {snap.agents.length === 0 && <Empty title="No agents yet">Agents appear here the first time they message you.</Empty>}
        {keys.map((key) => (
          <div className="list-group" key={key}>
            <div className="group-head">
              <span>{key}</span>
              <span className="group-count">{groups.get(key)!.length}</span>
            </div>
            {groups.get(key)!.map((a) => (
              <button
                type="button"
                key={a.address}
                className={`thread-row agent-row ${selected === a.address ? "selected" : ""}`}
                onClick={() => onSelect(a.address)}
                draggable
                onDragStart={(e) => {
                  e.dataTransfer.setData(DRAG_AGENT, a.address);
                  e.dataTransfer.effectAllowed = "move";
                }}
              >
                <div className="thread-top">
                  <PresenceDot agent={a} />
                  <span className="thread-agent">{a.display_name}</span>
                  <HarnessTag agent={a} />
                  <span className="spacer" />
                  {a.needs_me > 0 && a.top_tier && (
                    <span className={`side-count tier-${a.top_tier}`}>
                      <TierMark tier={a.top_tier} /> {a.needs_me}
                    </span>
                  )}
                </div>
                <div className="thread-snippet">
                  {[by !== "machine" && (a.presence.host ?? a.card.host), a.card.repo, a.card.branch].filter(Boolean).join(" · ") || a.address}
                </div>
                <div className="thread-meta">
                  {by !== "project" && <span className={`chip ${a.project_id ? "chip-project" : "chip-dim"}`}>{path(a.project_id) || "unsorted"}</span>}
                  <span className="chip chip-dim">last {relTime(a.last_seen_ms, snap.now_ms)}</span>
                  {a.notify !== "inherit" && <span className="chip">{a.notify}</span>}
                </div>
              </button>
            ))}
          </div>
        ))}
      </div>
    </section>
  );
}

export function AgentPane({ agent, onCompose }: { agent: Agent; onCompose: (to: string) => void }) {
  const { snap, openThread } = useStation();
  const threads = snap.threads.filter((t) => t.agent === agent.address).sort((a, b) => b.last_activity_ms - a.last_activity_ms);
  return (
    <section className="thread-pane">
      <header className="pane-head">
        <div className="pane-title">
          <h1>{agent.display_name}</h1>
          <div className="pane-actions">
            <button type="button" className="btn btn-primary" onClick={() => onCompose(agent.address)}>
              ✎ Message
            </button>
          </div>
        </div>
        <div className="who-line">
          <HarnessTag agent={agent} />
          <PresenceDot agent={agent} label />
          <span className="mono muted">{agent.address}</span>
        </div>
      </header>
      <div className="timeline">
        <AgentCard agent={agent} />
        <h3 className="sub-head">Threads with this agent</h3>
        {threads.length === 0 && <div className="muted pad">No threads yet.</div>}
        {threads.map((t) => (
          <button type="button" key={t.thread_id} className={`thread-row status-${t.status} tier-edge-${t.tier}`} onClick={() => openThread(t.thread_id)}>
            <div className="thread-top">
              {t.status === "needs-me" ? <TierMark tier={t.tier} /> : <span className="tier tier-none" />}
              <span className="thread-subject">{t.subject}</span>
              <span className="spacer" />
              <span className="chip chip-dim">{t.status}</span>
              <span className="thread-time">{relTime(t.last_activity_ms, snap.now_ms)}</span>
            </div>
            <div className="thread-snippet">{t.snippet}</div>
          </button>
        ))}
      </div>
    </section>
  );
}
