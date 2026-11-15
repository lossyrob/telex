import { useState } from "react";
import { HarnessTag, Modal, PresenceDot, ProjectPicker, useStation } from "./bits";
import { flattenTree, projectPath, projectTree } from "./model";
import type { Agent, RuleField, Suggestion } from "./types";

const FIELD_LABEL: Record<RuleField, string> = {
  address: "Address",
  repo: "Repo",
  host: "Machine",
  harness: "Harness",
  branch: "Branch",
  tag: "Tag",
  hint: "Agent’s project hint",
  name: "Agent name",
};

const BASIS_LABEL: Record<string, string> = {
  repo: "Same repo",
  namespace: "Same address prefix",
  host: "Same machine",
  hint: "Asked for by the agents",
};

function SuggestionCard({ s }: { s: Suggestion }) {
  const { act, snap, notice } = useStation();
  const [path, setPath] = useState(s.project_path.join(" / "));
  const names = s.agents.map((addr) => snap.agents.find((a) => a.address === addr)?.display_name ?? addr);
  const apply = async () => {
    const project_path = path.split("/").map((p) => p.trim()).filter(Boolean);
    const done = await act("suggestion_apply", { id: s.id, project_path });
    if (done !== undefined) notice(`Filed ${s.agents.length} agent${s.agents.length === 1 ? "" : "s"} in ${project_path.join(" › ")}. New agents that match will follow.`);
  };
  return (
    <div className="suggestion">
      <div className="suggestion-main">
        <div className="suggestion-title">
          <span className="pill">{BASIS_LABEL[s.basis] ?? s.basis}</span> {s.title}
        </div>
        <div className="suggestion-agents">{names.join(", ")}</div>
        <div className="suggestion-rule">
          Rule: {FIELD_LABEL[s.field]} matches <code>{s.pattern}</code>
        </div>
      </div>
      <div className="suggestion-act">
        <label>
          <span>File in</span>
          <input value={path} onChange={(e) => setPath(e.target.value)} title="Use “/” for subfolders, e.g. telex / station" />
        </label>
        <button type="button" className="btn btn-primary" onClick={() => void apply()}>
          Create &amp; file {s.agents.length}
        </button>
        <button type="button" className="btn btn-ghost" onClick={() => void act("suggestion_dismiss", { id: s.id })}>
          Dismiss
        </button>
      </div>
    </div>
  );
}

function UnsortedAgent({ agent }: { agent: Agent }) {
  const { act } = useStation();
  const [picking, setPicking] = useState(false);
  return (
    <div className="unsorted-agent">
      <PresenceDot agent={agent} />
      <span className="thread-agent">{agent.display_name}</span>
      <HarnessTag agent={agent} />
      <span className="muted small">{[agent.presence.host ?? agent.card.host, agent.card.repo].filter(Boolean).join(" · ") || agent.address}</span>
      <span className="spacer" />
      <span className="menu-wrap">
        <button type="button" className="btn" onClick={() => setPicking((x) => !x)}>
          File in…
        </button>
        {picking && (
          <ProjectPicker
            current={null}
            onClose={() => setPicking(false)}
            onPick={(id) => {
              void act("agent_update", { address: agent.address, project_id: id });
              setPicking(false);
            }}
          />
        )}
      </span>
    </div>
  );
}

function RuleEditor() {
  const { snap, act } = useStation();
  const [field, setField] = useState<RuleField>("address");
  const [pattern, setPattern] = useState("");
  const [project, setProject] = useState("");
  const nodes = flattenTree(projectTree(snap), new Set());
  const matchCount = (ruleField: RuleField, rulePattern: string) => {
    // Mirrors the engine's case-insensitive glob closely enough for a preview.
    const re = new RegExp("^" + rulePattern.toLowerCase().split("*").map((p) => p.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")).join(".*") + "$");
    return snap.agents.filter((a) => {
      const values: (string | null)[] =
        ruleField === "address" ? [a.address]
        : ruleField === "repo" ? [a.card.repo]
        : ruleField === "host" ? [a.card.host]
        : ruleField === "harness" ? [a.card.harness]
        : ruleField === "branch" ? [a.card.branch]
        : ruleField === "hint" ? [a.card.project_hint]
        : ruleField === "name" ? [a.card.name]
        : a.card.tags;
      return values.some((v) => v && re.test(v.toLowerCase()));
    }).length;
  };
  return (
    <div className="rules">
      {snap.rules.length === 0 && <div className="muted pad">No rules yet. Accept a suggestion above, or add one here.</div>}
      {snap.rules.map((r, i) => (
        <div className={`rule ${r.enabled ? "" : "rule-off"}`} key={r.id}>
          <span className="rule-n">{i + 1}</span>
          <span>
            When <b>{FIELD_LABEL[r.field]}</b> matches <code>{r.pattern}</code> → <b>{projectPath(snap.projects, r.project_id).join(" › ") || "(deleted project)"}</b>
          </span>
          <span className="muted small">{matchCount(r.field, r.pattern)} matching now</span>
          <span className="spacer" />
          <button type="button" className="btn btn-mini" title="Earlier rules win" disabled={i === 0} onClick={() => void act("rule_move", { id: r.id, delta: -1 })}>↑</button>
          <button type="button" className="btn btn-mini" disabled={i === snap.rules.length - 1} onClick={() => void act("rule_move", { id: r.id, delta: 1 })}>↓</button>
          <button type="button" className="btn btn-mini" onClick={() => void act("rule_update", { id: r.id, enabled: !r.enabled })}>{r.enabled ? "Disable" : "Enable"}</button>
          <button type="button" className="btn btn-mini danger" onClick={() => void act("rule_delete", { id: r.id })}>Delete</button>
        </div>
      ))}
      <div className="rule rule-new">
        <span>When</span>
        <select value={field} onChange={(e) => setField(e.target.value as RuleField)}>
          {(Object.keys(FIELD_LABEL) as RuleField[]).map((f) => (
            <option key={f} value={f}>{FIELD_LABEL[f]}</option>
          ))}
        </select>
        <span>matches</span>
        <input placeholder="pattern, * is a wildcard" value={pattern} onChange={(e) => setPattern(e.target.value)} />
        <span>→</span>
        <select value={project} onChange={(e) => setProject(e.target.value)}>
          <option value="">choose project</option>
          {nodes.map((n) => (
            <option key={n.project.id} value={n.project.id}>{" ".repeat(n.depth * 2)}{n.project.name}</option>
          ))}
        </select>
        {pattern && <span className="muted small">{matchCount(field, pattern)} match</span>}
        <button
          type="button"
          className="btn btn-primary"
          disabled={!pattern.trim() || !project}
          onClick={() => {
            void act("rule_create", { field, pattern: pattern.trim(), project_id: project });
            setPattern("");
          }}
        >
          Add rule
        </button>
      </div>
    </div>
  );
}

/** One place to get organized: accept suggestions, file stragglers, tune rules. */
export function Organize({ onClose }: { onClose: () => void }) {
  const { snap } = useStation();
  const unsorted = snap.agents.filter((a) => !a.project_id);
  return (
    <Modal title="Organize agents into projects" onClose={onClose} wide>
      <p className="lede">
        Agents are filed into projects, and their threads follow. Filing is local to this Station: it changes what you see and how you are
        notified, never what Telex delivers.
      </p>
      <h3 className="sub-head">Suggested projects</h3>
      {snap.suggestions.length === 0 && (
        <div className="muted pad">{unsorted.length ? "No groupings stand out among the unsorted agents." : "Everything is filed. New agents that match your rules file themselves."}</div>
      )}
      {snap.suggestions.map((s) => (
        <SuggestionCard key={s.id} s={s} />
      ))}

      <h3 className="sub-head">Unsorted agents ({unsorted.length})</h3>
      {unsorted.length === 0 && <div className="muted pad">None.</div>}
      {unsorted.map((a) => (
        <UnsortedAgent key={a.address} agent={a} />
      ))}
      {unsorted.length > 0 && <div className="muted small pad">Tip: you can also drag any thread or agent onto a project in the sidebar.</div>}

      <h3 className="sub-head">Filing rules</h3>
      <p className="muted small">
        Checked in order for every agent you have not filed by hand. Order of precedence: your own filing, then the first matching rule, then the
        agent’s own project hint.
      </p>
      <RuleEditor />
    </Modal>
  );
}
