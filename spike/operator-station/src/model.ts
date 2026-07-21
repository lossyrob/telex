import type { SourceReferenceView, StationMessage } from "./types";

const EXTENSION_KEY = "operator-station-spike";
const EXTENSION_ID = "urn:telex:experimental:operator-station-spike:v1";

interface SourceMessageJson {
  id?: unknown;
  threadId?: unknown;
  from?: unknown;
  to?: unknown;
  subject?: unknown;
  sentAtMs?: unknown;
  storeFingerprint?: unknown;
}

interface ValidSourceMessage extends SourceMessageJson {
  id: number;
  threadId: number;
  to: string;
  sentAtMs: number;
}

export function mergeMessages(
  current: StationMessage[],
  incoming: StationMessage[],
): StationMessage[] {
  const byId = new Map<number, StationMessage>();
  for (const message of current) byId.set(message.id, message);
  for (const message of incoming) byId.set(message.id, message);
  return [...byId.values()].sort(
    (left, right) => right.sentAtMs - left.sentAtMs || right.id - left.id,
  );
}

export function experimentalSources(
  metadata: string | null,
  activeStoreFingerprint: string,
): SourceReferenceView[] {
  if (!metadata) return [];

  let envelope: unknown;
  try {
    envelope = JSON.parse(metadata);
  } catch {
    return [];
  }
  if (!isRecord(envelope)) return [];

  const extensions = envelope.extensions;
  if (
    !isRecord(extensions) ||
    extensions[EXTENSION_KEY] !== EXTENSION_ID
  ) {
    return [];
  }

  const ext = envelope.ext;
  if (!isRecord(ext) || !isRecord(ext[EXTENSION_KEY])) return [];
  const sources = ext[EXTENSION_KEY].sourceMessages;
  if (!Array.isArray(sources)) return [];

  return sources.flatMap((source) => {
    if (!isSource(source)) return [];
    const storeFingerprint =
      typeof source.storeFingerprint === "string"
        ? source.storeFingerprint
        : null;
    return [
      {
        id: source.id,
        threadId: source.threadId,
        from: typeof source.from === "string" ? source.from : null,
        to: source.to,
        subject: typeof source.subject === "string" ? source.subject : null,
        sentAtMs: source.sentAtMs,
        storeFingerprint,
        resolution:
          storeFingerprint === activeStoreFingerprint
            ? "eligible-for-resolution"
            : "unavailable-in-current-store",
        message: null,
      } satisfies SourceReferenceView,
    ];
  });
}

export function formatTimestamp(epochMs: number): string {
  if (!Number.isFinite(epochMs)) return "Unknown time";
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(epochMs));
}

function isSource(value: unknown): value is ValidSourceMessage {
  if (!isRecord(value)) return false;
  return (
    typeof value.id === "number" &&
    Number.isInteger(value.id) &&
    value.id > 0 &&
    typeof value.threadId === "number" &&
    Number.isInteger(value.threadId) &&
    value.threadId > 0 &&
    typeof value.to === "string" &&
    typeof value.sentAtMs === "number"
  );
}

function isRecord(value: unknown): value is Record<string, any> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}
