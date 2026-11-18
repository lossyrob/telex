// Mirrors `core/src/model.rs` view types.

export type Tier = "blocked" | "needs-reply" | "routine" | "fyi";
export type Direction = "inbound" | "outbound";
export type NotifyOverride = "inherit" | "mute" | "elevate" | "blocked-only";
export type ThreadStatus = "needs-me" | "waiting" | "snoozed" | "done" | "fyi";
export type FocusMode = "off" | "blocked-only" | "silent";
export type RuleField = "address" | "repo" | "host" | "harness" | "branch" | "tag" | "hint" | "name";

export interface Card {
  name: string | null;
  harness: string | null;
  model: string | null;
  host: string | null;
  os: string | null;
  cwd: string | null;
  repo: string | null;
  branch: string | null;
  session_id: string | null;
  session_name: string | null;
  terminal: string | null;
  open_command: string | null;
  project_hint: string | null;
  url: string | null;
  tags: string[];
}

export interface Receipt {
  occupied_at_send: boolean | null;
  consumed: boolean;
  disposition: string | null;
  refreshed_at_ms: number;
}

export interface Message {
  id: number;
  local_id: string | null;
  direction: Direction;
  from: string | null;
  to: string;
  cc: string[];
  kind: string;
  attention: string;
  tier: Tier;
  requires_disposition: boolean;
  subject: string | null;
  body: string;
  sent_at_ms: number;
  role: string;
  disposition: string | null;
  open_obligation: boolean;
  unread: boolean;
  card: Card | null;
  replies: string[];
  other_metadata: string | null;
  receipt: Receipt | null;
  state: string | null;
  error: string | null;
  source: string;
}

export interface Thread {
  thread_id: number;
  subject: string;
  agent: string;
  station_address: string;
  status: ThreadStatus;
  tier: Tier;
  unread: number;
  open_obligations: number;
  last_activity_ms: number;
  last_inbound_ms: number;
  snippet: string;
  project_id: string | null;
  project_source: string;
  snooze_until_ms: number | null;
  pinned: boolean;
  muted: boolean;
  cc_only: boolean;
  messages: Message[];
}

export interface Presence {
  state: "listening" | "attached" | "offline" | "unknown";
  detail: string | null;
  host: string | null;
  description: string | null;
  tags: string | null;
  checked_at_ms: number;
}

export interface Agent {
  address: string;
  display_name: string;
  card: Card;
  nickname: string | null;
  notes: string | null;
  locator_note: string | null;
  project_id: string | null;
  project_source: string;
  notify: NotifyOverride;
  presence: Presence;
  first_seen_ms: number;
  last_seen_ms: number;
  thread_ids: number[];
  needs_me: number;
  top_tier: Tier | null;
}

export interface Project {
  id: string;
  name: string;
  parent_id: string | null;
  color: string | null;
  notify: NotifyOverride;
  order: number;
}

export interface Rule {
  id: string;
  field: RuleField;
  pattern: string;
  project_id: string;
  enabled: boolean;
}

export interface ChannelPolicy {
  banner: boolean;
  sound: boolean;
  demand: boolean;
  badge: boolean;
  remind_after_min: number;
}

export interface Settings {
  operator_name: string;
  addresses: { address: string; label: string | null; enabled: boolean }[];
  blocked: ChannelPolicy;
  needs_reply: ChannelPolicy;
  routine: ChannelPolicy;
  fyi: ChannelPolicy;
  focus: { mode: FocusMode; until_ms: number | null };
  quiet_hours: { enabled: boolean; start_min: number; end_min: number; allow_blocked: boolean };
  read_receipts: boolean;
  coalesce_window_s: number;
  use_project_hints: boolean;
}

export interface NotificationRecord {
  id: string;
  at_ms: number;
  kind: string;
  thread_id: number | null;
  message_id: number | null;
  tier: Tier;
  effective_tier: Tier;
  title: string;
  body: string;
  banner: boolean;
  sound: boolean;
  demand: boolean;
  badge: boolean;
  outcome: string;
  reason: string;
}

export interface Suggestion {
  id: string;
  basis: string;
  title: string;
  project_path: string[];
  field: RuleField;
  pattern: string;
  agents: string[];
}

export interface Health {
  address: string;
  registered: boolean;
  receive_ready: boolean;
  sender_ready: boolean;
  degraded: boolean;
  recovering: boolean;
  pending_unconsumed: number;
  inbound_actionable: number;
  principal: string | null;
  principal_verification: string;
  last_error: string | null;
  last_receive_ms: number | null;
  evidence: string[];
}

export interface Counts {
  needs_me: number;
  blocked: number;
  needs_reply: number;
  routine: number;
  fyi_unread: number;
  waiting: number;
  snoozed: number;
  unsorted_agents: number;
  badge: number;
  top_tier: Tier | null;
}

export interface Snapshot {
  rev: number;
  now_ms: number;
  station: {
    operator_name: string;
    store_id: string;
    backend: string;
    connected: boolean;
    connect_error: string | null;
    telex_build: string | null;
    sandbox: boolean;
    port: string;
    port_note: string | null;
  };
  threads: Thread[];
  agents: Agent[];
  projects: Project[];
  rules: Rule[];
  settings: Settings;
  health: Health[];
  notifications: NotificationRecord[];
  suggestions: Suggestion[];
  counts: Counts;
}

export type StationEvent =
  | { type: "changed"; rev: number }
  | { type: "notify"; record: NotificationRecord }
  | { type: "badge"; counts: Counts };

/** What the middle pane is showing. */
export type View =
  | { kind: "needs-me"; tier?: Tier }
  | { kind: "fyi" }
  | { kind: "waiting" }
  | { kind: "snoozed" }
  | { kind: "done" }
  | { kind: "all" }
  | { kind: "project"; id: string }
  | { kind: "unsorted" }
  | { kind: "agents" };
