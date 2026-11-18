import { Empty, HarnessTag, PresenceDot, projectName, TierMark, useStation } from "./bits";
import { relTime, sections, TIER_MEANING, untilTime, viewTitle } from "./model";
import { DRAG_AGENT, DRAG_THREAD } from "./Sidebar";
import type { Thread } from "./types";

function ThreadRow({ thread, selected }: { thread: Thread; selected: boolean }) {
  const { snap, agent, openThread, view } = useStation();
  const a = agent(thread.agent);
  const machine = a?.presence.host ?? a?.card.host;
  const showProject = view.kind !== "project" && thread.project_id;
  return (
    <button
      type="button"
      className={`thread-row status-${thread.status} tier-edge-${thread.tier} ${selected ? "selected" : ""} ${thread.unread ? "unread" : ""}`}
      onClick={() => openThread(thread.thread_id)}
      draggable
      onDragStart={(e) => {
        e.dataTransfer.setData(DRAG_THREAD, String(thread.thread_id));
        e.dataTransfer.setData(DRAG_AGENT, thread.agent);
        e.dataTransfer.effectAllowed = "move";
      }}
      data-thread={thread.thread_id}
    >
      <div className="thread-top">
        {thread.status === "needs-me" ? <TierMark tier={thread.tier} /> : <span className="tier tier-none" />}
        <span className="thread-agent">{a?.display_name ?? thread.agent}</span>
        <HarnessTag agent={a} />
        <PresenceDot agent={a} />
        <span className="spacer" />
        {thread.pinned && <span className="chip chip-flag" title="Flagged for follow-up">⚑</span>}
        {thread.muted && <span title="Muted thread">🔕</span>}
        <span className="thread-time">
          {thread.status === "snoozed" && thread.snooze_until_ms
            ? `☾ ${untilTime(thread.snooze_until_ms, snap.now_ms)}`
            : relTime(thread.last_activity_ms, snap.now_ms)}
        </span>
      </div>
      <div className="thread-subject">
        {thread.unread > 0 && <span className="unread-dot" />}
        {thread.subject}
      </div>
      <div className="thread-snippet">{thread.snippet}</div>
      <div className="thread-meta">
        {machine && <span className="chip">{machine}</span>}
        {showProject && <span className="chip chip-project">{projectName(snap.projects, thread.project_id)}</span>}
        {!thread.project_id && view.kind !== "unsorted" && <span className="chip chip-dim">unsorted</span>}
        {thread.open_obligations > 0 && (
          <span className="chip chip-obligation" title="The agent asked for an explicit outcome">
            needs disposition
          </span>
        )}
        {thread.cc_only && <span className="chip chip-dim">cc</span>}
      </div>
    </button>
  );
}

export function ThreadList({
  threads,
  selected,
  filter,
  setFilter,
  filterRef,
}: {
  threads: Thread[];
  selected: number | null;
  filter: string;
  setFilter: (value: string) => void;
  filterRef: React.RefObject<HTMLInputElement | null>;
}) {
  const { snap, view } = useStation();
  const groups = sections(threads);
  return (
    <section className="list-pane">
      <header className="list-head">
        <h1>{viewTitle(snap, view)}</h1>
        <span className="list-count">{threads.length}</span>
      </header>
      <div className="list-filter">
        <input
          ref={filterRef}
          placeholder="Filter by agent, machine, repo, text…  ( / )"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              setFilter("");
              (e.target as HTMLInputElement).blur();
            }
          }}
        />
      </div>
      <div className="list-scroll">
        {groups.length === 0 &&
          (filter ? (
            <Empty title="No matches">Nothing here matches “{filter}”.</Empty>
          ) : view.kind === "needs-me" ? (
            <Empty title="Nothing needs you">
              Agents can still reach you. Blocked messages will interrupt; everything else waits here quietly.
            </Empty>
          ) : (
            <Empty title="Nothing here yet" />
          ))}
        {groups.map((group) => (
          <div className="list-group" key={group.key}>
            <div
              className={`group-head ${group.tier ? `tier-${group.tier}` : ""}`}
              title={group.tier ? TIER_MEANING[group.tier] : undefined}
            >
              {group.tier && group.key !== "fyi" && group.key !== "done" && <TierMark tier={group.tier} />}
              <span>{group.label}</span>
              <span className="group-count">{group.threads.length}</span>
            </div>
            {group.threads.map((t) => (
              <ThreadRow key={t.thread_id} thread={t} selected={t.thread_id === selected} />
            ))}
          </div>
        ))}
      </div>
    </section>
  );
}
