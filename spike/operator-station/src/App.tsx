import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { formatTimestamp, mergeMessages } from "./model";
import type {
  DispositionRecord,
  SentReceipt,
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
  const [readIds, setReadIds] = useState<Set<number>>(() => new Set());
  const threadCache = useRef(new Map<number, ThreadView>());
  const threadRequests = useRef(new Map<number, Promise<ThreadView>>());
  const readStorageKey =
    state.config.storeFingerprint === "loading"
      ? null
      : `operator-station:read:${state.config.storeFingerprint}:${state.config.stationAddress}`;

  const getThread = useCallback((messageId: number): Promise<ThreadView> => {
    const cached = threadCache.current.get(messageId);
    if (cached) return Promise.resolve(cached);

    const pending = threadRequests.current.get(messageId);
    if (pending) return pending;

    const request = invoke<ThreadView>("read_thread", { messageId })
      .then((next) => {
        threadCache.current.set(messageId, next);
        return next;
      })
      .finally(() => {
        threadRequests.current.delete(messageId);
      });
    threadRequests.current.set(messageId, request);
    return request;
  }, []);

  const prefetchThreads = useCallback(
    (messages: StationMessage[]) => {
      void (async () => {
        for (const message of messages.slice(0, 20)) {
          try {
            await getThread(message.id);
          } catch {
            // Foreground selection reports errors; speculative prefetch stays silent.
          }
        }
      })();
    },
    [getThread],
  );

  useEffect(() => {
    if (!readStorageKey) return;
    try {
      const stored = JSON.parse(localStorage.getItem(readStorageKey) ?? "[]");
      setReadIds(
        new Set(
          Array.isArray(stored)
            ? stored.filter((value): value is number => Number.isSafeInteger(value))
            : [],
        ),
      );
    } catch {
      setReadIds(new Set());
    }
  }, [readStorageKey]);

  const updateReadIds = useCallback(
    (update: (current: Set<number>) => Set<number>) => {
      setReadIds((current) => {
        const next = update(current);
        if (next === current || !readStorageKey) return next;
        localStorage.setItem(
          readStorageKey,
          JSON.stringify([...next].sort((left, right) => left - right)),
        );
        return next;
      });
    },
    [readStorageKey],
  );

  const markRead = useCallback(
    (messageId: number) => {
      updateReadIds((current) => {
        if (current.has(messageId)) return current;
        const next = new Set(current);
        next.add(messageId);
        return next;
      });
    },
    [updateReadIds],
  );

  const markUnread = useCallback(
    (messageId: number) => {
      updateReadIds((current) => {
        if (!current.has(messageId)) return current;
        const next = new Set(current);
        next.delete(messageId);
        return next;
      });
    },
    [updateReadIds],
  );

  useEffect(() => {
    let active = true;
    const unlisteners: Array<() => void> = [];

    void invoke<StationState>("initial_state")
      .then((next) => {
        if (!active) return;
        setState(next);
        prefetchThreads(next.messages);
      })
      .catch((cause: unknown) => {
        if (active) setError(String(cause));
      });

    void listen<StationState>("station-state", (event) => {
      if (!active) return;
      setState(event.payload);
      prefetchThreads(event.payload.messages);
    }).then((unlisten) => unlisteners.push(unlisten));

    void listen<StationMessage>("station-delivery", (event) => {
      if (!active) return;
      setState((current) => ({
        ...current,
        messages: mergeMessages(current.messages, [event.payload]),
      }));
      threadCache.current.delete(event.payload.id);
    }).then((unlisten) => unlisteners.push(unlisten));

    return () => {
      active = false;
      for (const unlisten of unlisteners) unlisten();
    };
  }, [prefetchThreads]);

  const loadThread = useCallback(async (messageId: number) => {
    setError(null);
    try {
      const next = await getThread(messageId);
      setThread(next);
    } catch (cause) {
      setThread(null);
      setError(String(cause));
    }
  }, [getThread]);

  useEffect(() => {
    if (selectedId !== null) void loadThread(selectedId);
  }, [loadThread, selectedId]);

  useEffect(() => {
    if (selectedId !== null && thread?.selected.id === selectedId) {
      markRead(selectedId);
    }
  }, [markRead, selectedId, thread?.selected.id]);

  const selected = useMemo(
    () => state.messages.find((message) => message.id === selectedId) ?? null,
    [selectedId, state.messages],
  );
  const unreadCount = useMemo(
    () => state.messages.filter((message) => !readIds.has(message.id)).length,
    [readIds, state.messages],
  );

  const runAction = useCallback(
    async (action: () => Promise<unknown>) => {
      setBusy(true);
      setError(null);
      try {
        await action();
        if (selectedId !== null) threadCache.current.delete(selectedId);
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
    const body = replyBody.trim();
    setBusy(true);
    setError(null);
    void invoke<SentReceipt>("reply_to", {
      messageId: selected.id,
      body,
    })
      .then((receipt) => {
        const now = Date.now();
        const optimisticReply: StationMessage = {
          id: receipt.id,
          threadId: receipt.threadId,
          parentId: receipt.parentId,
          from: receipt.from ?? state.config.stationAddress,
          to: receipt.to,
          deliveredTo: null,
          primaryTo: receipt.to,
          cc: [],
          deliveryRole: null,
          kind: "operator-station-spike.human-reply",
          attention: receipt.attention ?? "background",
          requiresDisposition: receipt.requiresDisposition ?? true,
          requiresDispositionForCurrentRecipient: false,
          subject: selected.subject ? `Re: ${selected.subject}` : "Human reply",
          body,
          metadata: null,
          sentAtMs: now,
          createdAtMs: now,
          latestDisposition: null,
          actionable: false,
          ackPending: false,
        };
        setThread((current) => {
          if (!current || current.selected.id !== selected.id) return current;
          const next = {
            ...current,
            thread: [
              ...current.thread,
              { message: optimisticReply, dispositions: [] },
            ],
          };
          threadCache.current.set(selected.id, next);
          return next;
        });
        setReplyBody("");
      })
      .catch((cause: unknown) => setError(String(cause)))
      .finally(() => setBusy(false));
  };

  const disposition = (dispositionState: "deferred" | "handled" | "closed") => {
    if (!selected) return;
    setBusy(true);
    setError(null);
    void invoke<DispositionRecord>("set_disposition", {
      messageId: selected.id,
      dispositionState,
      note: `Station marked ${dispositionState}`,
    })
      .then((record) => {
        const terminal = ["handled", "closed", "rejected"].includes(record.state);
        setState((current) => ({
          ...current,
          messages: current.messages.map((message) =>
            message.id === record.messageId
              ? {
                  ...message,
                  latestDisposition: record.state,
                  actionable: !terminal,
                }
              : message,
          ),
        }));
        setThread((current) => {
          if (!current) return current;
          const updateMessage = (message: StationMessage) =>
            message.id === record.messageId
              ? {
                  ...message,
                  latestDisposition: record.state,
                  actionable: !terminal,
                }
              : message;
          const next = {
            ...current,
            selected: updateMessage(current.selected),
            thread: current.thread.map((item) =>
              item.message.id === record.messageId
                ? {
                    message: updateMessage(item.message),
                    dispositions: [...item.dispositions, record],
                  }
                : item,
            ),
          };
          threadCache.current.set(current.selected.id, next);
          return next;
        });
      })
      .catch((cause: unknown) => setError(String(cause)))
      .finally(() => setBusy(false));
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
              <span>
                {state.messages.length} loaded · {unreadCount} unread
              </span>
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
              state.messages.map((message) => {
                const unread = !readIds.has(message.id);
                return (
                  <button
                    className={`feed-card ${message.id === selectedId ? "selected" : ""} ${unread ? "unread" : "read"}`}
                    key={message.id}
                    onClick={() => {
                      setSelectedId(message.id);
                      markRead(message.id);
                    }}
                    type="button"
                  >
                  <div className="card-meta">
                    <span className="card-meta-left">
                      {unread ? <span aria-label="Unread" className="unread-dot" /> : null}
                      <span className={`attention ${message.attention}`}>
                        {message.attention}
                      </span>
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
                    <span className={`read-label ${unread ? "unread" : ""}`}>
                      {unread ? "Unread" : "Read"}
                    </span>
                  </div>
                  </button>
                );
              })
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
                <div className="thread-heading-actions">
                  <span className="kind">{selected.kind}</span>
                  <button
                    className="secondary"
                    onClick={() => markUnread(selected.id)}
                    type="button"
                  >
                    Mark unread
                  </button>
                </div>
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
      <h3>Operator-agent asserted source</h3>
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
              source.resolution === "matched" ? "resolved" : "unavailable"
            }
          >
            {sourceResolutionLabel(source.resolution)}
          </span>
          <code>{source.storeFingerprint || "no store fingerprint"}</code>
        </article>
      ))}
    </section>
  );
}

function sourceResolutionLabel(
  resolution: SourceReferenceView["resolution"],
): string {
  switch (resolution) {
    case "matched":
      return "ID and complete envelope matched";
    case "eligible-for-resolution":
      return "Matching store; resolution pending";
    case "envelope-mismatch":
      return "Source envelope is incomplete or mismatched";
    case "unavailable-in-current-store":
      return "Unavailable in current store";
  }
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
