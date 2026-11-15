import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { call, host, snapshot, subscribe } from "./api";
import { AgentList, AgentPane } from "./Agents";
import { Empty, Modal, Station, TierMark, type StationCtx } from "./bits";
import { threadsForView } from "./model";
import { Organize } from "./Organize";
import { SettingsPanel, type SettingsTab } from "./SettingsPanel";
import { Sidebar } from "./Sidebar";
import { ThreadList } from "./ThreadList";
import { ThreadPane } from "./ThreadPane";
import type { NotificationRecord, Snapshot, View } from "./types";

interface Toast {
  id: string;
  text: string;
  detail?: string;
  tone: "info" | "error" | "notify";
  record?: NotificationRecord;
}

function Compose({ to, onClose }: { to: string; onClose: () => void }) {
  const [snap, setSnap] = useState<Snapshot | null>(null);
  const [recipient, setRecipient] = useState(to);
  const [subject, setSubject] = useState("");
  const [body, setBody] = useState("");
  const [attention, setAttention] = useState("next-checkpoint");
  const [requires, setRequires] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    void snapshot().then(setSnap);
  }, []);
  const send = async () => {
    try {
      await call("compose", { to: recipient, subject, body, attention, requires_disposition: requires });
      onClose();
    } catch (e) {
      setError(String((e as Error).message ?? e));
    }
  };
  return (
    <Modal title="New message to an agent" onClose={onClose}>
      <div className="form-col">
        <label>
          <span>To (Telex address)</span>
          <input list="agent-addresses" value={recipient} onChange={(e) => setRecipient(e.target.value)} autoFocus={!to} placeholder="agent address" />
          <datalist id="agent-addresses">
            {snap?.agents.map((a) => (
              <option key={a.address} value={a.address}>{a.display_name}</option>
            ))}
          </datalist>
        </label>
        <label>
          <span>Subject</span>
          <input value={subject} onChange={(e) => setSubject(e.target.value)} />
        </label>
        <label>
          <span>Message</span>
          <textarea rows={6} value={body} onChange={(e) => setBody(e.target.value)} autoFocus={!!to} />
        </label>
        <div className="form-row">
          <label className="urgency">
            <span>Deliver</span>
            <select value={attention} onChange={(e) => setAttention(e.target.value)}>
              <option value="interrupt">Interrupt now</option>
              <option value="next-checkpoint">At next checkpoint</option>
              <option value="background">Whenever</option>
            </select>
          </label>
          <label className="check">
            <input type="checkbox" checked={requires} onChange={(e) => setRequires(e.target.checked)} />
            <span>Ask the agent to record an outcome</span>
          </label>
          <span className="spacer" />
          <button type="button" className="btn btn-primary" disabled={!recipient.trim() || !body.trim()} onClick={() => void send()}>
            Send
          </button>
        </div>
        {error && <div className="receipt-error">{error}</div>}
      </div>
    </Modal>
  );
}

const HELP: [string, string][] = [
  ["j / k", "Next / previous thread"],
  ["r", "Reply"],
  ["⌘↩", "Send"],
  ["e", "Done"],
  ["s", "Snooze 2 hours"],
  ["f", "Flag for follow-up"],
  ["u", "Mark unread"],
  ["m", "Move to project"],
  ["/", "Filter"],
  ["n", "New message"],
  ["g then i / b / a / o", "Needs me / Blocked / Agents / Organize"],
  ["?", "This help"],
];

export function App() {
  const [snap, setSnap] = useState<Snapshot | null>(null);
  const [up, setUp] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [view, setViewState] = useState<View>({ kind: "needs-me" });
  const [threadId, setThreadId] = useState<number | null>(null);
  const [agentAddr, setAgentAddr] = useState<string | null>(null);
  const [filter, setFilter] = useState("");
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [organize, setOrganize] = useState(false);
  const [settings, setSettings] = useState<SettingsTab | null>(null);
  const [compose, setCompose] = useState<string | null>(null);
  const [help, setHelp] = useState(false);
  const filterRef = useRef<HTMLInputElement>(null);
  const composerRef = useRef<HTMLTextAreaElement>(null);
  const pendingG = useRef(false);
  const refreshing = useRef(false);
  const dirty = useRef(false);

  const toast = useCallback((t: Omit<Toast, "id">, ttl = 6000) => {
    const id = Math.random().toString(36).slice(2);
    setToasts((list) => [...list.slice(-4), { ...t, id }]);
    window.setTimeout(() => setToasts((list) => list.filter((x) => x.id !== id)), ttl);
  }, []);

  const refresh = useCallback(async () => {
    if (refreshing.current) {
      dirty.current = true;
      return;
    }
    refreshing.current = true;
    try {
      do {
        dirty.current = false;
        setSnap(await snapshot());
        setLoadError(null);
      } while (dirty.current);
    } catch (e) {
      setLoadError(String((e as Error).message ?? e));
    } finally {
      refreshing.current = false;
    }
  }, []);

  useEffect(() => {
    void refresh();
    const off = subscribe(
      (event) => {
        if (event.type === "changed") void refresh();
        if (event.type === "notify") {
          // In the desktop app the shell raises the native notification; the
          // in-window toast is the same evidence for the browser host.
          toast({ text: event.record.title, detail: event.record.body, tone: "notify", record: event.record }, event.record.demand ? 15000 : 7000);
        }
      },
      (ok) => {
        setUp(ok);
        if (ok) void refresh();
      },
    );
    const timer = window.setInterval(() => void refresh(), 20_000);
    return () => {
      off();
      window.clearInterval(timer);
    };
  }, [refresh, toast]);

  const act = useCallback(
    async <T,>(cmd: string, args: Record<string, unknown> = {}): Promise<T | undefined> => {
      try {
        const result = await call<T>(cmd, args);
        void refresh();
        const partial = (result as { handle_errors?: string[]; errors?: string[] } | null) ?? null;
        const errors = partial?.handle_errors ?? partial?.errors ?? [];
        if (errors.length) toast({ text: "Partly done", detail: errors.join("; "), tone: "error" }, 12000);
        return (result ?? (null as T)) as T;
      } catch (e) {
        void refresh();
        toast({ text: "That did not work", detail: String((e as Error).message ?? e), tone: "error" }, 12000);
        return undefined;
      }
    },
    [refresh, toast],
  );

  const setView = useCallback((next: View) => {
    setViewState(next);
    setFilter("");
  }, []);

  const openThread = useCallback(
    (id: number) => {
      setThreadId(id);
      setAgentAddr(null);
      setSettings(null);
      setViewState((current) => {
        if (current.kind === "agents") return { kind: "all" };
        return current;
      });
    },
    [],
  );

  const openAgent = useCallback((address: string) => {
    setViewState({ kind: "agents" });
    setAgentAddr(address);
  }, []);

  const threads = useMemo(() => (snap ? threadsForView(snap, view, filter) : []), [snap, view, filter]);
  const thread = snap?.threads.find((t) => t.thread_id === threadId) ?? null;

  // Keep a thread selected: prefer the current one, else the top of the list.
  useEffect(() => {
    if (!snap || view.kind === "agents") return;
    if (threadId !== null && snap.threads.some((t) => t.thread_id === threadId)) return;
    setThreadId(threads[0]?.thread_id ?? null);
  }, [snap, threads, threadId, view.kind]);

  // The desktop shell asks the window to show a thread (notification / tray click).
  useEffect(() => {
    if (host !== "tauri") return;
    let dispose: (() => void) | undefined;
    void import("@tauri-apps/api/event").then(({ listen }) =>
      listen<{ thread_id?: number; view?: string }>("station-open", (e) => {
        if (e.payload.thread_id) {
          setViewState({ kind: "all" });
          setThreadId(e.payload.thread_id);
        } else if (e.payload.view === "needs-me") setViewState({ kind: "needs-me" });
      }).then((un) => (dispose = un)),
    );
    return () => dispose?.();
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement;
      const typing = target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.tagName === "SELECT" || target.isContentEditable;
      if (typing || e.metaKey || e.ctrlKey || e.altKey) return;
      if (organize || settings || compose !== null) return;
      if (pendingG.current) {
        pendingG.current = false;
        if (e.key === "i") setView({ kind: "needs-me" });
        if (e.key === "b") setView({ kind: "needs-me", tier: "blocked" });
        if (e.key === "a") setView({ kind: "agents" });
        if (e.key === "o") setOrganize(true);
        return;
      }
      const index = threads.findIndex((t) => t.thread_id === threadId);
      switch (e.key) {
        case "g":
          pendingG.current = true;
          window.setTimeout(() => (pendingG.current = false), 1200);
          break;
        case "j":
        case "ArrowDown":
          if (threads[index + 1]) setThreadId(threads[index + 1].thread_id);
          e.preventDefault();
          break;
        case "k":
        case "ArrowUp":
          if (index > 0) setThreadId(threads[index - 1].thread_id);
          e.preventDefault();
          break;
        case "r":
          composerRef.current?.focus();
          e.preventDefault();
          break;
        case "e":
          if (thread) void act("mark_done", { thread_id: thread.thread_id });
          break;
        case "s":
          if (thread) void act("snooze", { thread_id: thread.thread_id, until_ms: Date.now() + 2 * 3_600_000 });
          break;
        case "f":
          if (thread) void act("flag", { thread_id: thread.thread_id, on: !thread.pinned });
          break;
        case "u":
          if (thread) void act("mark_unread", { thread_id: thread.thread_id });
          break;
        case "m":
          document.querySelector<HTMLButtonElement>("[data-move-button]")?.click();
          break;
        case "/":
          filterRef.current?.focus();
          e.preventDefault();
          break;
        case "n":
          setCompose("");
          e.preventDefault();
          break;
        case "?":
          setHelp(true);
          break;
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [threads, threadId, thread, act, setView, organize, settings, compose]);

  if (!snap) {
    return (
      <div className="boot">
        <Empty title={loadError ? "Station is not reachable" : "Starting Station…"}>
          {loadError ?? "Connecting to Telex."}
          {loadError && host === "browser" && <div className="muted small">In a browser this UI needs `station-headless` running.</div>}
        </Empty>
      </div>
    );
  }

  const ctx: StationCtx = {
    snap,
    act,
    agent: (address) => snap.agents.find((a) => a.address === address),
    view,
    setView,
    openThread,
    openAgent,
    notice: (text) => toast({ text, tone: "info" }),
  };

  const blocked = snap.threads.filter((t) => t.status === "needs-me" && t.tier === "blocked" && !t.muted);
  const selectedAgent = snap.agents.find((a) => a.address === agentAddr) ?? null;
  const st = snap.station;
  const trouble = !up
    ? "Lost contact with the Station engine. Reconnecting…"
    : !st.connected
      ? `Not connected to Telex${st.connect_error ? `: ${st.connect_error}` : ""}`
      : snap.health.find((h) => h.last_error)?.last_error
        ? `An attended address has a problem: ${snap.health.find((h) => h.last_error)!.last_error}`
        : null;

  return (
    <Station.Provider value={ctx}>
      <div className="app">
        {trouble && (
          <div className="strip strip-trouble" role="status">
            <span>{trouble}</span>
            <span className="spacer" />
            <button type="button" className="btn btn-mini" onClick={() => setSettings("health")}>
              Health
            </button>
          </div>
        )}
        {blocked.length > 0 && !(blocked.length === 1 && blocked[0].thread_id === threadId && view.kind !== "agents") && (
          <div className="strip strip-blocked" role="alert">
            <TierMark tier="blocked" />
            <span>
              {blocked.length === 1
                ? `${ctx.agent(blocked[0].agent)?.display_name ?? blocked[0].agent} is blocked on you: ${blocked[0].subject}`
                : `${blocked.length} agents are blocked on you`}
            </span>
            <span className="spacer" />
            <button
              type="button"
              className="btn btn-mini"
              onClick={() => {
                setView({ kind: "needs-me", tier: "blocked" });
                setThreadId(blocked[0].thread_id);
              }}
            >
              Show
            </button>
          </div>
        )}
        <div className="panes">
          <Sidebar onOrganize={() => setOrganize(true)} onSettings={() => setSettings("notifications")} onCompose={() => setCompose("")} />
          {view.kind === "agents" ? (
            <>
              <AgentList selected={agentAddr} onSelect={setAgentAddr} />
              {selectedAgent ? (
                <AgentPane agent={selectedAgent} onCompose={(to) => setCompose(to)} />
              ) : (
                <section className="thread-pane">
                  <Empty title="Pick an agent">See who it is, where its session runs, and every thread you have with it.</Empty>
                </section>
              )}
            </>
          ) : (
            <>
              <ThreadList threads={threads} selected={threadId} filter={filter} setFilter={setFilter} filterRef={filterRef} />
              {thread ? (
                <ThreadPane thread={thread} composerRef={composerRef} />
              ) : (
                <section className="thread-pane">
                  <Empty title="No thread selected" />
                </section>
              )}
            </>
          )}
        </div>
        <div className="toasts" aria-live="polite">
          {toasts.map((t) => (
            <button
              type="button"
              key={t.id}
              className={`toast toast-${t.tone} ${t.record ? `tier-edge-${t.record.effective_tier}` : ""}`}
              onClick={() => {
                if (t.record?.thread_id) openThread(t.record.thread_id);
                setToasts((list) => list.filter((x) => x.id !== t.id));
              }}
            >
              <div className="toast-title">{t.text}</div>
              {t.detail && <div className="toast-detail">{t.detail}</div>}
            </button>
          ))}
        </div>
        {organize && <Organize onClose={() => setOrganize(false)} />}
        {settings && <SettingsPanel initial={settings} onClose={() => setSettings(null)} />}
        {compose !== null && <Compose to={compose} onClose={() => { setCompose(null); void refresh(); }} />}
        {help && (
          <Modal title="Keyboard" onClose={() => setHelp(false)}>
            <table className="log">
              <tbody>
                {HELP.map(([k, v]) => (
                  <tr key={k}>
                    <td><kbd>{k}</kbd></td>
                    <td>{v}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </Modal>
        )}
      </div>
    </Station.Provider>
  );
}
