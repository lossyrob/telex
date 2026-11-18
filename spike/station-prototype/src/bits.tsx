import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import type { Agent, NotifyOverride, Project, Snapshot, Tier, View } from "./types";
import {
  flattenTree,
  harnessLabel,
  PRESENCE_HELP,
  PRESENCE_LABEL,
  projectTree,
  TIER_GLYPH,
  TIER_LABEL,
  TIER_MEANING,
} from "./model";

export interface StationCtx {
  snap: Snapshot;
  /** Run an engine command; failures surface as a toast and resolve to undefined. */
  act: <T = unknown>(cmd: string, args?: Record<string, unknown>) => Promise<T | undefined>;
  agent: (address: string) => Agent | undefined;
  view: View;
  setView: (view: View) => void;
  openThread: (threadId: number) => void;
  openAgent: (address: string) => void;
  notice: (text: string) => void;
}

export const Station = createContext<StationCtx | null>(null);

export function useStation(): StationCtx {
  const ctx = useContext(Station);
  if (!ctx) throw new Error("Station context missing");
  return ctx;
}

export function TierMark({ tier, label = false }: { tier: Tier; label?: boolean }) {
  return (
    <span className={`tier tier-${tier}`} title={`${TIER_LABEL[tier]}: ${TIER_MEANING[tier]}`}>
      <span className="tier-glyph">{TIER_GLYPH[tier]}</span>
      {label && <span className="tier-label">{TIER_LABEL[tier]}</span>}
    </span>
  );
}

export function PresenceDot({ agent, label = false }: { agent: Agent | undefined; label?: boolean }) {
  const state = agent?.presence.state ?? "unknown";
  return (
    <span className={`presence presence-${state}`} title={`${PRESENCE_LABEL[state]}. ${PRESENCE_HELP[state]}`}>
      <span className="presence-dot" />
      {label && <span>{PRESENCE_LABEL[state]}</span>}
    </span>
  );
}

export function HarnessTag({ agent }: { agent: Agent | undefined }) {
  const label = harnessLabel(agent?.card.harness);
  if (!label) return null;
  return <span className={`harness harness-${label.toLowerCase()}`}>{label}</span>;
}

export interface MenuItem {
  label: string;
  hint?: string;
  onSelect?: () => void;
  danger?: boolean;
  checked?: boolean;
  disabled?: boolean;
  separator?: boolean;
  heading?: boolean;
}

/** Small popover menu. Closes on outside click or Escape. */
export function Menu({
  button,
  items,
  className = "",
  align = "left",
  title,
}: {
  button: ReactNode;
  items: MenuItem[];
  className?: string;
  align?: "left" | "right";
  title?: string;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLSpanElement>(null);
  useDismiss(ref, open, () => setOpen(false));
  return (
    <span className="menu-wrap" ref={ref}>
      <button
        type="button"
        className={`btn ${className}`}
        title={title}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={(e) => {
          e.stopPropagation();
          setOpen((o) => !o);
        }}
      >
        {button}
      </button>
      {open && (
        <div className={`menu menu-${align}`} role="menu">
          {items.map((item, i) =>
            item.separator ? (
              <div className="menu-sep" key={i} />
            ) : item.heading ? (
              <div className="menu-heading" key={i}>
                {item.label}
              </div>
            ) : (
              <button
                type="button"
                role="menuitem"
                key={i}
                disabled={item.disabled}
                className={`menu-item ${item.danger ? "danger" : ""}`}
                onClick={(e) => {
                  e.stopPropagation();
                  setOpen(false);
                  item.onSelect?.();
                }}
              >
                <span className="menu-check">{item.checked ? "✓" : ""}</span>
                <span className="menu-label">{item.label}</span>
                {item.hint && <span className="menu-hint">{item.hint}</span>}
              </button>
            ),
          )}
        </div>
      )}
    </span>
  );
}

export function useDismiss(ref: React.RefObject<HTMLElement | null>, open: boolean, close: () => void) {
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) close();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") close();
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open, close, ref]);
}

export function Modal({
  title,
  onClose,
  children,
  wide = false,
}: {
  title: string;
  onClose: () => void;
  children: ReactNode;
  wide?: boolean;
}) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);
  return (
    <div className="modal-scrim" onMouseDown={onClose}>
      <div className={`modal ${wide ? "modal-wide" : ""}`} role="dialog" aria-label={title} onMouseDown={(e) => e.stopPropagation()}>
        <header className="modal-head">
          <h2>{title}</h2>
          <button type="button" className="btn btn-ghost" onClick={onClose} aria-label="Close">
            ✕
          </button>
        </header>
        <div className="modal-body">{children}</div>
      </div>
    </div>
  );
}

export const NOTIFY_LABEL: Record<NotifyOverride, string> = {
  inherit: "Follow my defaults",
  elevate: "Elevate: treat Routine as Needs reply",
  "blocked-only": "Only interrupt for Blocked",
  mute: "Mute: never interrupt",
};

export function notifyItems(current: NotifyOverride, set: (n: NotifyOverride) => void): MenuItem[] {
  return (Object.keys(NOTIFY_LABEL) as NotifyOverride[]).map((n) => ({
    label: NOTIFY_LABEL[n],
    checked: current === n,
    onSelect: () => set(n),
  }));
}

/**
 * Pick where something is filed. Shows the folder tree, "Unsorted", and an
 * inline "new project" field so organizing never needs a separate screen.
 */
export function ProjectPicker({
  current,
  onPick,
  onClose,
}: {
  current: string | null;
  onPick: (projectId: string | null) => void;
  onClose: () => void;
}) {
  const { snap, act } = useStation();
  const [name, setName] = useState("");
  const [parent, setParent] = useState<string | null>(null);
  const ref = useRef<HTMLDivElement>(null);
  useDismiss(ref, true, onClose);
  const nodes = flattenTree(projectTree(snap), new Set());
  const create = async () => {
    const trimmed = name.trim();
    if (!trimmed) return;
    const made = await act<{ id: string }>("project_create", { name: trimmed, parent_id: parent });
    if (made?.id) onPick(made.id);
  };
  return (
    <div className="picker" ref={ref} onMouseDown={(e) => e.stopPropagation()}>
      <div className="picker-list">
        <button type="button" className={`menu-item ${current === null ? "current" : ""}`} onClick={() => onPick(null)}>
          <span className="menu-check">{current === null ? "✓" : ""}</span>
          <span className="menu-label muted">Unsorted</span>
        </button>
        {nodes.map((n) => (
          <button
            type="button"
            key={n.project.id}
            className={`menu-item ${current === n.project.id ? "current" : ""}`}
            style={{ paddingLeft: 10 + n.depth * 14 }}
            onClick={() => onPick(n.project.id)}
          >
            <span className="menu-check">{current === n.project.id ? "✓" : ""}</span>
            <span className="menu-label">{n.project.name}</span>
          </button>
        ))}
      </div>
      <div className="picker-new">
        <input
          placeholder="New project…"
          value={name}
          onChange={(e) => setName(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void create();
          }}
        />
        {nodes.length > 0 && (
          <select value={parent ?? ""} onChange={(e) => setParent(e.target.value || null)} title="Create inside">
            <option value="">top level</option>
            {nodes.map((n) => (
              <option key={n.project.id} value={n.project.id}>
                {"  ".repeat(n.depth)}in {n.project.name}
              </option>
            ))}
          </select>
        )}
        <button type="button" className="btn btn-primary" disabled={!name.trim()} onClick={() => void create()}>
          Create
        </button>
      </div>
    </div>
  );
}

export function projectName(projects: Project[], id: string | null): string {
  if (!id) return "Unsorted";
  return projects.find((p) => p.id === id)?.name ?? "Unsorted";
}

export function Empty({ title, children }: { title: string; children?: ReactNode }) {
  return (
    <div className="empty">
      <div className="empty-title">{title}</div>
      {children && <div className="empty-body">{children}</div>}
    </div>
  );
}
