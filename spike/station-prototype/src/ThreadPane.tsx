import { useEffect, useRef, useState } from "react";
import { shell } from "./api";
import {
  HarnessTag,
  Menu,
  NOTIFY_LABEL,
  notifyItems,
  PresenceDot,
  ProjectPicker,
  TierMark,
  useStation,
  type MenuItem,
} from "./bits";
import {
  clock,
  linkify,
  PRESENCE_HELP,
  PRESENCE_LABEL,
  projectPath,
  receiptLabel,
  relTime,
  snoozeOptions,
  untilTime,
  whereParts,
} from "./model";
import type { Agent, Message, Thread } from "./types";

/** Untrusted text: rendered inertly; links need an explicit, informed click. */
export function Body({ text }: { text: string }) {
  const open = (href: string) => {
    let hostname = href;
    try {
      hostname = new URL(href).hostname;
    } catch {
      return;
    }
    if (!window.confirm(`Open this link from an agent message?\n\n${href}\n\nDestination: ${hostname}`)) return;
    void shell("open_url", { url: href });
    if (!("__TAURI_INTERNALS__" in window)) window.open(href, "_blank", "noopener,noreferrer");
  };
  return (
    <div className="msg-body">
      {linkify(text).map((seg, i) =>
        seg.kind === "text" ? (
          <span key={i}>{seg.text}</span>
        ) : (
          <a
            key={i}
            href={seg.href}
            title={`Opens ${seg.href} after you confirm`}
            onClick={(e) => {
              e.preventDefault();
              open(seg.href);
            }}
          >
            {seg.text}
          </a>
        ),
      )}
    </div>
  );
}

function Fact({ label, value, copy = false }: { label: string; value: string | null | undefined; copy?: boolean }) {
  const { notice } = useStation();
  if (!value) return null;
  return (
    <div className="fact">
      <dt>{label}</dt>
      <dd>
        <span className={copy ? "mono" : ""}>{value}</span>
        {copy && (
          <button
            type="button"
            className="btn btn-mini"
            onClick={() => {
              void navigator.clipboard?.writeText(value);
              notice(`Copied ${label.toLowerCase()}`);
            }}
          >
            Copy
          </button>
        )}
      </dd>
    </div>
  );
}

/** Who this agent is and where its session lives, with provenance kept visible. */
export function AgentCard({ agent, compact = false }: { agent: Agent; compact?: boolean }) {
  const { act, snap } = useStation();
  const [picking, setPicking] = useState(false);
  const c = agent.card;
  const p = agent.presence;
  const path = projectPath(snap.projects, agent.project_id);
  const source =
    agent.project_source === "agent"
      ? "filed by you"
      : agent.project_source.startsWith("rule:")
        ? "filed by a rule"
        : agent.project_source === "hint"
          ? "filed by the agent’s own hint"
          : "";
  const save = (field: string) => (e: React.FocusEvent<HTMLInputElement | HTMLTextAreaElement>) => {
    const value = e.target.value.trim();
    const current = (agent as unknown as Record<string, string | null>)[field] ?? "";
    if (value !== current) void act("agent_update", { address: agent.address, [field]: value || null });
  };
  return (
    <div className={`agent-card ${compact ? "compact" : ""}`}>
      <div className="card-cols">
        <dl className="card-col">
          <div className="card-heading">
            Where to find it <span className="prov prov-self">self-reported</span>
          </div>
          <Fact label="Harness" value={[c.harness, c.model].filter(Boolean).join(" · ") || null} />
          <Fact label="Machine" value={c.host ? [c.host, c.os].filter(Boolean).join(" · ") : null} />
          <Fact label="Repo" value={c.repo ? (c.branch ? `${c.repo} @ ${c.branch}` : c.repo) : null} />
          <Fact label="Folder" value={c.cwd} copy />
          <Fact label="Terminal" value={c.terminal} />
          <Fact label="Session" value={[c.session_name, c.session_id].filter(Boolean).join(" · ") || null} copy={!!c.session_id && !c.session_name} />
          <Fact label="Open with" value={c.open_command} copy />
          {c.url && (
            <div className="fact">
              <dt>Link</dt>
              <dd>
                <Body text={c.url} />
              </dd>
            </div>
          )}
          {c.tags.length > 0 && <Fact label="Tags" value={c.tags.join(", ")} />}
          {!c.harness && !c.host && !c.repo && !c.cwd && !c.open_command && (
            <div className="card-none">
              This agent has not said where it runs. Agents using the Station notify helper report this automatically.
            </div>
          )}
        </dl>
        <dl className="card-col">
          <div className="card-heading">
            Recorded by Telex <span className="prov prov-core">from the exchange</span>
          </div>
          <Fact label="Address" value={agent.address} copy />
          <div className="fact">
            <dt>Presence</dt>
            <dd>
              <PresenceDot agent={agent} label />
              <div className="fact-help">{PRESENCE_HELP[p.state]}</div>
            </dd>
          </div>
          <Fact label="Host" value={p.host} />
          <Fact label="Describes itself as" value={p.description} />
          <Fact label="Directory tags" value={p.tags} />
          <Fact label="First seen" value={agent.first_seen_ms ? clock(agent.first_seen_ms) : null} />
          <Fact label="Last message" value={agent.last_seen_ms ? clock(agent.last_seen_ms) : null} />
        </dl>
      </div>
      <div className="card-local">
        <div className="card-heading">
          Mine <span className="prov prov-local">only on this Station</span>
        </div>
        <div className="local-grid">
          <label>
            <span>Name I use</span>
            <input defaultValue={agent.nickname ?? ""} placeholder={agent.display_name} onBlur={save("nickname")} key={`n-${agent.address}-${agent.nickname}`} />
          </label>
          <label>
            <span>How I get to it</span>
            <input
              defaultValue={agent.locator_note ?? ""}
              placeholder="e.g. devbox-2, tmux window 3 (overrides the self-report)"
              onBlur={save("locator_note")}
              key={`l-${agent.address}-${agent.locator_note}`}
            />
          </label>
          <label className="span-2">
            <span>Notes</span>
            <input defaultValue={agent.notes ?? ""} placeholder="Anything worth remembering about this agent" onBlur={save("notes")} key={`o-${agent.address}-${agent.notes}`} />
          </label>
        </div>
        <div className="local-actions">
          <span className="menu-wrap">
            <button type="button" className="btn" onClick={() => setPicking((x) => !x)}>
              📁 {path.length ? path.join(" › ") : "Unsorted"}
            </button>
            {source && <span className="fact-help inline">{source}</span>}
            {picking && (
              <ProjectPicker
                current={agent.project_source === "agent" ? agent.project_id : null}
                onClose={() => setPicking(false)}
                onPick={(id) => {
                  void act("agent_update", { address: agent.address, project_id: id });
                  setPicking(false);
                }}
              />
            )}
          </span>
          <Menu
            button={<>🔔 {NOTIFY_LABEL[agent.notify]}</>}
            items={notifyItems(agent.notify, (n) => void act("agent_update", { address: agent.address, notify: n }))}
          />
        </div>
      </div>
    </div>
  );
}

function MessageItem({ m, thread, agent, onReplyTo }: { m: Message; thread: Thread; agent: Agent | undefined; onReplyTo: (m: Message) => void }) {
  const { act, snap } = useStation();
  const mine = m.direction === "outbound";
  const receipt = mine ? receiptLabel(m, agent) : null;
  const dispose = (state: string, ask?: string) => {
    const note = ask ? window.prompt(ask) : null;
    if (ask && note === null) return;
    void act("disposition", { message_id: m.id, state, note: note || undefined });
  };
  const items: MenuItem[] = mine
    ? []
    : [
        { label: "Reply to this message", onSelect: () => onReplyTo(m) },
        { separator: true, label: "" },
        { heading: true, label: "Record an outcome for the agent" },
        { label: "Handled", onSelect: () => dispose("handled") },
        { label: "Declined…", onSelect: () => dispose("rejected", "Why are you declining? (the agent sees this)") },
        { label: "Escalated…", onSelect: () => dispose("escalated", "Escalated to whom / why?") },
        { label: "Closed", onSelect: () => dispose("closed") },
      ];
  return (
    <article className={`msg ${mine ? "msg-mine" : "msg-agent"} ${m.unread ? "msg-unread" : ""} ${!mine ? `tier-edge-${m.tier}` : ""}`}>
      <header className="msg-head">
        {mine ? (
          <span className="msg-author">You</span>
        ) : (
          <>
            <TierMark tier={m.tier} label />
            <span className="msg-author">{agent?.display_name ?? m.from ?? "unknown sender"}</span>
            {m.role !== "primary" && <span className="chip chip-dim">cc: no action expected</span>}
          </>
        )}
        <span className="msg-time" title={clock(m.sent_at_ms)}>
          {relTime(m.sent_at_ms, snap.now_ms)}
        </span>
        <span className="spacer" />
        {!mine && m.requires_disposition && (
          <span className={`chip ${m.open_obligation ? "chip-obligation" : "chip-ok"}`}>
            {m.open_obligation ? (m.disposition ? `${m.disposition}: outcome still owed` : "wants an outcome") : `you: ${m.disposition}`}
          </span>
        )}
        {!mine && !m.requires_disposition && m.disposition && <span className="chip chip-ok">you: {m.disposition}</span>}
        {items.length > 0 && <Menu button="⋯" items={items} className="btn-ghost" align="right" title="Message actions" />}
      </header>
      {m.subject && m.subject !== thread.subject && !m.subject.startsWith("Re: ") && <div className="msg-subject">{m.subject}</div>}
      <Body text={m.body} />
      {m.other_metadata && (
        <details className="msg-meta">
          <summary>Other metadata (not interpreted)</summary>
          <pre>{m.other_metadata}</pre>
        </details>
      )}
      {receipt && (
        <footer className={`receipt receipt-${receipt.tone}`}>
          <span>{receipt.text}</span>
          {m.error && <span className="receipt-error">{m.error}</span>}
          {m.local_id && m.state !== "sending" && (
            <span className="receipt-actions">
              {m.state === "indeterminate" && (
                <button type="button" className="btn btn-mini" onClick={() => void act("outbox_check", { local_id: m.local_id })}>
                  Check with Telex
                </button>
              )}
              <button type="button" className="btn btn-mini" onClick={() => void act("outbox_retry", { local_id: m.local_id })}>
                Retry
              </button>
              {m.state === "failed" && (
                <button type="button" className="btn btn-mini" onClick={() => void act("outbox_discard", { local_id: m.local_id })}>
                  Discard
                </button>
              )}
            </span>
          )}
        </footer>
      )}
    </article>
  );
}

const URGENCY: { value: string; label: string; help: string }[] = [
  { value: "interrupt", label: "Interrupt now", help: "Wakes the agent even mid-task." },
  { value: "next-checkpoint", label: "At next checkpoint", help: "The agent reads it when it reaches a natural stopping point." },
  { value: "background", label: "Whenever", help: "No urgency; read with other routine traffic." },
];

function Composer({ thread, agent, replyTo, clearReplyTo, inputRef }: {
  thread: Thread;
  agent: Agent | undefined;
  replyTo: Message | null;
  clearReplyTo: () => void;
  inputRef: React.RefObject<HTMLTextAreaElement | null>;
}) {
  const { act } = useStation();
  const [body, setBody] = useState("");
  const [busy, setBusy] = useState(false);
  const defaultUrgency = thread.tier === "blocked" && thread.status === "needs-me" ? "interrupt" : "next-checkpoint";
  const [urgency, setUrgency] = useState(defaultUrgency);
  const [done, setDone] = useState(true);
  const lastInbound = [...thread.messages].reverse().find((m) => m.direction === "inbound");
  const suggestions = lastInbound?.replies ?? [];

  // A different thread is a different conversation: reset the draft controls.
  useEffect(() => {
    setBody("");
    setUrgency(defaultUrgency);
    setDone(true);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [thread.thread_id]);

  const send = async () => {
    const text = body.trim();
    if (!text || busy) return;
    setBusy(true);
    const result = lastInbound
      ? await act("reply", { thread_id: thread.thread_id, body: text, attention: urgency, handle: done, reply_to: replyTo?.id })
      : await act("compose", { to: thread.agent, subject: thread.subject, body: text, attention: urgency, from: thread.station_address });
    setBusy(false);
    if (result !== undefined) {
      setBody("");
      clearReplyTo();
    }
  };

  const presence = agent?.presence.state ?? "unknown";
  return (
    <div className="composer">
      {suggestions.length > 0 && (
        <div className="suggested" title="Suggested by the agent. Picking one only fills the reply; you still send it.">
          <span className="suggested-label">Agent suggests</span>
          {suggestions.map((s) => (
            <button
              type="button"
              key={s}
              className="chip chip-reply"
              onClick={() => {
                setBody(s);
                inputRef.current?.focus();
              }}
            >
              {s}
            </button>
          ))}
        </div>
      )}
      {replyTo && (
        <div className="replying-to">
          Replying to: “{replyTo.body.slice(0, 80)}”
          <button type="button" className="btn btn-mini" onClick={clearReplyTo}>
            ✕
          </button>
        </div>
      )}
      <textarea
        ref={inputRef}
        value={body}
        placeholder={`Reply to ${agent?.display_name ?? thread.agent}…   (r to focus, ⌘↩ to send)`}
        onChange={(e) => setBody(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            void send();
          }
          if (e.key === "Escape") (e.target as HTMLTextAreaElement).blur();
        }}
        rows={3}
      />
      <div className="composer-bar">
        <label className="urgency" title={URGENCY.find((u) => u.value === urgency)?.help}>
          <span>Deliver</span>
          <select value={urgency} onChange={(e) => setUrgency(e.target.value)}>
            {URGENCY.map((u) => (
              <option key={u.value} value={u.value}>
                {u.label}
              </option>
            ))}
          </select>
        </label>
        {lastInbound && (
          <label className="check" title="Also record “handled” on what the agent asked, and clear this thread from Needs me.">
            <input type="checkbox" checked={done} onChange={(e) => setDone(e.target.checked)} />
            <span>Mark done when sent</span>
          </label>
        )}
        <span className="spacer" />
        <span className={`deliver-hint presence-${presence}`}>
          {presence === "listening" && "Will reach the session right away"}
          {presence === "attached" && "Session is attached; it reads this at its next check"}
          {presence === "offline" && "No session attending: kept until one re-attaches"}
        </span>
        <button type="button" className="btn btn-primary" disabled={!body.trim() || busy} onClick={() => void send()}>
          {busy ? "Sending…" : done && lastInbound ? "Send & done" : "Send"}
        </button>
      </div>
    </div>
  );
}

export function ThreadPane({ thread, composerRef }: { thread: Thread; composerRef: React.RefObject<HTMLTextAreaElement | null> }) {
  const { snap, agent, act, openAgent, notice } = useStation();
  const a = agent(thread.agent);
  const [expanded, setExpanded] = useState(false);
  const [picking, setPicking] = useState(false);
  const [scope, setScope] = useState<"agent" | "thread">("agent");
  const [replyTo, setReplyTo] = useState<Message | null>(null);
  const scroller = useRef<HTMLDivElement>(null);
  const where = whereParts(a);
  const path = projectPath(snap.projects, thread.project_id);

  useEffect(() => {
    setReplyTo(null);
    setPicking(false);
  }, [thread.thread_id]);

  // Opening a thread is what "read" means here.
  useEffect(() => {
    if (thread.unread > 0 && thread.thread_id > 0) void act("mark_read", { thread_id: thread.thread_id });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [thread.thread_id, thread.unread]);

  useEffect(() => {
    scroller.current?.scrollTo({ top: scroller.current.scrollHeight });
  }, [thread.thread_id, thread.messages.length]);

  const more: MenuItem[] = [
    {
      label: thread.pinned ? "Remove follow-up flag" : "Flag for follow-up",
      hint: "f",
      onSelect: () => void act("flag", { thread_id: thread.thread_id, on: !thread.pinned }),
    },
    { label: "Mark unread", hint: "u", onSelect: () => void act("mark_unread", { thread_id: thread.thread_id }) },
    {
      label: thread.muted ? "Unmute this thread" : "Mute this thread",
      onSelect: () => void act("thread_update", { thread_id: thread.thread_id, muted: !thread.muted }),
    },
    { separator: true, label: "" },
    { label: "Open agent details", onSelect: () => openAgent(thread.agent) },
  ];
  const snooze: MenuItem[] = [
    ...(thread.snooze_until_ms
      ? [{ label: "Wake now", onSelect: () => void act("snooze", { thread_id: thread.thread_id, until_ms: null }) }, { separator: true, label: "" }]
      : []),
    ...snoozeOptions(snap.now_ms).map((o) => ({
      label: o.label,
      onSelect: () => {
        void act("snooze", { thread_id: thread.thread_id, until_ms: o.until });
        notice(`Snoozed. The agent sees “deferred”; new messages wake it.`);
      },
    })),
  ];

  return (
    <section className="thread-pane">
      <header className="pane-head">
        <div className="pane-title">
          <h1>{thread.subject}</h1>
          <div className="pane-actions">
            {thread.status !== "done" && thread.status !== "fyi" && (
              <button type="button" className="btn" title="Record handled and clear from Needs me (e)" onClick={() => void act("mark_done", { thread_id: thread.thread_id })}>
                ✓ Done
              </button>
            )}
            <Menu button={thread.snooze_until_ms ? `☾ ${untilTime(thread.snooze_until_ms, snap.now_ms)}` : "☾ Snooze"} items={snooze} align="right" title="Hide until later (s)" />
            <Menu button="⋯" items={more} align="right" title="More" />
          </div>
        </div>
        <div className="who-line">
          {thread.status === "needs-me" && <TierMark tier={thread.tier} label />}
          {thread.status === "waiting" && <span className="chip">sent · awaiting pickup</span>}
          {thread.status === "snoozed" && <span className="chip">snoozed</span>}
          {thread.status === "done" && <span className="chip chip-ok">done</span>}
          <button type="button" className="who-name" onClick={() => setExpanded((x) => !x)} title="Who is this and where does it run?">
            {a?.display_name ?? thread.agent}
          </button>
          <HarnessTag agent={a} />
          <PresenceDot agent={a} label />
          <span className="spacer" />
          <span className="menu-wrap">
            <button type="button" className="chip chip-project chip-button" onClick={() => setPicking((x) => !x)} title="Change where this is filed (m)" data-move-button>
              📁 {path.length ? path.join(" › ") : "Unsorted"} ▾
            </button>
            {picking && (
              <div className="picker-wrap">
                <div className="segmented">
                  <button type="button" className={scope === "agent" ? "on" : ""} onClick={() => setScope("agent")}>
                    File the agent
                  </button>
                  <button type="button" className={scope === "thread" ? "on" : ""} onClick={() => setScope("thread")}>
                    Only this thread
                  </button>
                </div>
                <ProjectPicker
                  current={thread.project_id}
                  onClose={() => setPicking(false)}
                  onPick={(id) => {
                    if (scope === "agent") void act("agent_update", { address: thread.agent, project_id: id });
                    else void act("thread_update", { thread_id: thread.thread_id, project_id: id });
                    setPicking(false);
                  }}
                />
              </div>
            )}
          </span>
        </div>
        <button type="button" className="where-strip" onClick={() => setExpanded((x) => !x)} aria-expanded={expanded}>
          <span className="where-label">Where</span>
          {a?.locator_note ? (
            <span className="where-part where-mine" title="Your own note">
              {a.locator_note}
            </span>
          ) : where.length ? (
            where.map((p) => (
              <span key={p.label} className={`where-part ${p.verified ? "where-verified" : ""}`} title={`${p.label}${p.verified ? " (recorded by Telex)" : " (self-reported)"}`}>
                {p.value}
              </span>
            ))
          ) : (
            <span className="where-part muted">{thread.agent} has not reported where it runs</span>
          )}
          <span className="spacer" />
          <span className="where-toggle">{expanded ? "Hide details ▴" : "Details ▾"}</span>
        </button>
        {expanded && a && <AgentCard agent={a} compact />}
      </header>

      <div className="timeline" ref={scroller}>
        {thread.messages.map((m) => (
          <MessageItem key={m.local_id ?? m.id} m={m} thread={thread} agent={a} onReplyTo={(msg) => { setReplyTo(msg); composerRef.current?.focus(); }} />
        ))}
        <div className="timeline-foot">
          {a && a.presence.state !== "unknown" && (
            <span>
              {a.display_name}: {PRESENCE_LABEL[a.presence.state].toLowerCase()}
            </span>
          )}
        </div>
      </div>

      <Composer thread={thread} agent={a} replyTo={replyTo} clearReplyTo={() => setReplyTo(null)} inputRef={composerRef} />
    </section>
  );
}
