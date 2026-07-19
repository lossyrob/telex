export interface DispositionRecord {
  id: number;
  messageId: number;
  recipient: string;
  state: string;
  note: string | null;
  byPrincipal: string | null;
  atMs: number;
}

export interface StationMessage {
  id: number;
  threadId: number;
  parentId: number | null;
  from: string | null;
  to: string;
  deliveredTo: string | null;
  primaryTo: string | null;
  cc: string[];
  deliveryRole: string | null;
  kind: string;
  attention: string;
  requiresDisposition: boolean;
  requiresDispositionForCurrentRecipient: boolean;
  subject: string | null;
  body: string;
  metadata: string | null;
  sentAtMs: number;
  createdAtMs?: number | null;
  latestDisposition: string | null;
  actionable: boolean;
  ackPending: boolean;
  sourceReferences?: SourceReferenceView[];
  metadataError?: string | null;
}

export interface AddressStatus {
  address: string;
  occupied: boolean;
  health: string;
  detail: string | null;
}

export interface StationRuntimeStatus {
  phase: string;
  detail: string | null;
  courierState: string;
  station: AddressStatus | null;
  ingress: AddressStatus | null;
  diagnostics: string[];
}

export interface StationConfig {
  stationAddress: string;
  ingressAddress: string;
  storeFingerprint: string;
  telexVersion: string;
}

export interface StationState {
  config: StationConfig;
  messages: StationMessage[];
  status: StationRuntimeStatus;
}

export interface ThreadItem {
  message: StationMessage;
  dispositions: DispositionRecord[];
}

export interface SourceReferenceView {
  id: number;
  threadId: number;
  from: string | null;
  to: string;
  subject: string | null;
  sentAtMs: number;
  storeFingerprint: string | null;
  resolution: "resolved" | "unavailable-in-current-store";
  message: StationMessage | null;
}

export interface ThreadView {
  selected: StationMessage;
  thread: ThreadItem[];
  sources: SourceReferenceView[];
  rawMetadata: string | null;
}
