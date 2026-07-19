import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useState } from "react";

import { formatTimestamp, mergeMessages } from "./model";
import type {
  SourceReferenceView,
  StationMessage,
  StationState,
  ThreadView,
} from "./types";

const EMPTY_STATE: StationState = {
  config: {
    stationAddress: "operator:rob",
    ingressAddress: "attention:rob",
    storeFingerprint: "loading",
    telexVersion: "loading",
  },
  messages: [],
  status: {
    phase: "starting",
    detail: null,
    courierState: "starting",
    station: null,
    ingress: null,
    diagnostics: [],
  },
};

export default function App() {
  const [state, setState] = useState<StationState>(EMPTY_STATE);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [thread, setThread] = useState<ThreadView | null>(null);
  const [replyBody, setReplyBody] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    const unlisteners: Array<() => void> = [];

    void invoke<StationState>("initial_state")
      .then((next) => {
        if (!active) return;
        setState(next);
        setSelectedId((current) => current ?? next.messages[0]?.id ?? null);
      })
      .catch((cause: unknown) => {
        if (active) setError(String(cause));
      });

    void listen<StationState>("station-state", (event) => {
      if (!active) return;
      setState(event.payload);
      setSelectedId((current) => current ?? event.payload.messages[0]?.id ?? null);
    }).then((unlisten) => unlisteners.push(unlisten));

    void listen<StationMessage>("station-delivery", (event) => {
      if (!active) return;
      setState((current) => ({
        ...current,
        messages: mergeMessages(current.messages, [event.payload]),
      }));
      setSelectedId((current) => current ?? event.payload.id);
    }).then((unlisten) => unlisteners.push(unlisten));

    return () => {
      active = false;
      for (const unlisten of unlisteners) unlisten();
    };
  }, []);

  const loadThread = useCallback(async (messageId: number) => {
    setError(null);
    try {
      const next = await invoke<ThreadView>("read_thread", { messageId });
      setThread(next);
    } catch (cause) {
      setThread(null);
      setError(String(cause));
    }
  }, []);

  useEffect(() => {
    if (selectedId !== null) void loadThread(selectedId);
  }, [loadThread, selectedId]);

  const selected = useMemo(
    () => state.messages.find((message) => message.id === selectedId) ?? null,
    [selectedId, state.messages],
  );

  const runAction = useCallback(
    async (action: () => Promise<unknown>) => {
      setBusy(true);
      setError(null);
      try {
        await action();
        const next = await invoke<StationState>("initial_state");
        setState(next);
        if (selectedId !== null) await loadThread(selectedId);
      } catch (cause) {
        setError(String(cause));
      } finally {
        setBusy(false);
      }
    },
    [loadThread, selectedId],
  );

  const sendReply = () => {
    if (!selected || !replyBody.trim()) return;
    void runAction(async () => {
      await invoke("reply_to", {
        messageId: selected.id,
        body: replyBody.trim(),
      });
      setReplyBody("");
    });
  };

  const disposition = (dispositionState: "deferred" | "handled" | "closed") => {
    if (!selected) return;
    void runAction(() =>
      invoke("set_disposition", {
        messageId: selected.id,
        dispositionState,
        note: `Station marked ${dispositionState}`,
      }),
    );
  };

  return (
    <main className="app-shell">
      <header className="app-header">
        <div>
          <p className="eyebrow">Experimental Windows station</p>
          <h1>Operator Station</h1>
          <p className="subtitle">
            {state.config.stationAddress} · {state.config.storeFingerprint}
          </p>
        </div>
        <div className="header-status">
          <StatusPill
            label={`Courier: ${state.status.courierState}`}
            healthy={state.status.courierState === "armed"}
          />
          <StatusPill
            label={`Station: ${occupancyLabel(state.status.station?.occupied)}`}
            healthy={state.status.station?.occupied === true}
          />
          <StatusPill
            label={`Operator agent: ${occupancyLabel(state.status.ingress?.occupied)}`}
            healthy={state.status.ingress?.occupied === true}
          />
        </div>
      </header>

      {state.status.detail ? (
        <div className="runtime-banner">{state.status.detail}</div>
      ) : null}
      {error ? <div className="error-banner">{error}</div> : null}
      {state.status.diagnostics.length > 0 ? (
        <details className="diagnostics">
          <summary>Runtime diagnostics ({state.status.diagnostics.length})</summary>
          <ul>
            {state.status.diagnostics.map((diagnostic, index) => (
              <li key={`${index}:${diagnostic}`}>{diagnostic}</li>
            ))}
          </ul>
        </details>
      ) : null}

      <section className="workspace">
        <aside className="feed-pane">
          <div className="pane-heading">
            <div>
              <h2>Feed</h2>
              <span>{state.messages.length} loaded</span>
            </div>
            <button
              className="secondary"
              onClick={() => void runAction(() => invoke("retry_courier"))}
              type="button"
            >
              Retry courier
            </button>
          </div>
          <div className="feed-list">
            {state.messages.length === 0 ? (
              <p className="empty">No Station messages yet.</p>
            ) : (
              state.messages.map((message) => (
                <button
                  className={`feed-card ${message.id === selectedId ? "selected" : ""}`}
                  key={message.id}
                  onClick={() => setSelectedId(message.id)}
                  type="button"
                >
                  <div className="card-meta">
                    <span className={`attention ${message.attention}`}>
                      {message.attention}
                    </span>
                    <span>{formatTimestamp(message.sentAtMs)}</span>
                  </div>
                  <strong>{message.subject || message.kind}</strong>
                  <span className="from">{message.from || "unknown sender"}</span>
                  <p>{message.body}</p>
                  <div className="card-footer">
                    {message.requiresDispositionForCurrentRecipient ? (
                      <span className="actionable">Disposition required</span>
                    ) : (
                      <span>Informational</span>
                    )}
                    {message.ackPending ? <span>Ack pending</span> : null}
                    {message.latestDisposition ? (
                      <span>{message.latestDisposition}</span>
                    ) : null}
                  </div>
                </button>
              ))
            )}
          </div>
        </aside>

        <section className="thread-pane">
          {selected && thread ? (
            <>
              <div className="pane-heading">
                <div>
                  <p className="eyebrow">Mediated thread #{selected.threadId}</p>
                  <h2>{selected.subject || selected.kind}</h2>
                </div>
                <span className="kind">{selected.kind}</span>
              </div>

              <SourceReferences sources={thread.sources} />

              <div className="thread-list">
                {thread.thread.map(({ message, dispositions }) => (
                  <article className="thread-message" key={message.id}>
                    <div className="thread-header">
                      <strong>{message.from || "unknown"}</strong>
                      <span>#{message.id}</span>
                      <span>{formatTimestamp(message.sentAtMs)}</span>
                    </div>
                    <p>{message.body}</p>
                    {dispositions.length > 0 ? (
                      <div className="dispositions">
                        {dispositions.map((item) => (
                          <span key={item.id}>
                            {item.state}
                            {item.note ? `: ${item.note}` : ""}
                          </span>
                        ))}
                      </div>
                    ) : null}
                  </article>
                ))}
              </div>

              <div className="composer">
                <label htmlFor="reply">Reply to operator agent</label>
                <textarea
                  id="reply"
                  onChange={(event) => setReplyBody(event.target.value)}
                  placeholder="Type the decision or instruction..."
                  rows={4}
                  value={replyBody}
                />
                <div className="composer-actions">
                  <button
                    disabled={busy || !replyBody.trim()}
                    onClick={sendReply}
                    type="button"
                  >
                    Send reply
                  </button>
                  <button
                    className="secondary"
                    disabled={busy}
                    onClick={() => disposition("deferred")}
                    type="button"
                  >
                    Defer
                  </button>
                  <button
                    className="secondary"
                    disabled={busy}
                    onClick={() => disposition("handled")}
                    type="button"
                  >
                    Handle
                  </button>
                  <button
                    className="secondary"
                    disabled={busy}
                    onClick={() => disposition("closed")}
                    type="button"
                  >
                    Close
                  </button>
                </div>
              </div>

              {thread.rawMetadata ? (
                <details className="raw-metadata">
                  <summary>Raw metadata</summary>
                  <pre>{thread.rawMetadata}</pre>
                </details>
              ) : null}
            </>
          ) : (
            <p className="empty">Select a message to inspect its thread.</p>
          )}
        </section>
      </section>

      <footer>
        <span>{state.config.telexVersion}</span>
        <span>{state.status.phase}</span>
      </footer>
    </main>
  );
}

function SourceReferences({ sources }: { sources: SourceReferenceView[] }) {
  if (sources.length === 0) return null;
  return (
    <section className="sources">
      <h3>Raw source provenance</h3>
      {sources.map((source) => (
        <article key={`${source.storeFingerprint}:${source.id}`}>
          <div>
            <strong>{source.subject || `Source message #${source.id}`}</strong>
            <p>
              {source.from || "unknown"} → {source.to} ·{" "}
              {formatTimestamp(source.sentAtMs)}
            </p>
          </div>
          <span
            className={
              source.resolution === "resolved" ? "resolved" : "unavailable"
            }
          >
            {source.resolution === "resolved"
              ? "Resolved"
              : "Unavailable in current store"}
          </span>
          <code>{source.storeFingerprint || "no store fingerprint"}</code>
        </article>
      ))}
    </section>
  );
}

function StatusPill({
  label,
  healthy,
}: {
  label: string;
  healthy: boolean;
}) {
  return <span className={`status-pill ${healthy ? "healthy" : "warning"}`}>{label}</span>;
}

function occupancyLabel(occupied: boolean | undefined): string {
  if (occupied === true) return "online";
  if (occupied === false) return "unattended";
  return "unknown";
}
