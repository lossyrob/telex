// One transport for two hosts: in Tauri the engine runs in-process; in a
// browser the same commands go to `station-headless` over HTTP + SSE.

import type { Snapshot, StationEvent } from "./types";

const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export const host: "tauri" | "browser" = inTauri ? "tauri" : "browser";

export async function call<T = unknown>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  if (inTauri) {
    const { invoke } = await import("@tauri-apps/api/core");
    return (await invoke("station_call", { cmd, args })) as T;
  }
  const response = await fetch("/api/call", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ cmd, args }),
  });
  if (!response.ok) throw new Error(`Station is unreachable (${response.status})`);
  const payload = (await response.json()) as { ok: boolean; result?: T; error?: string };
  if (!payload.ok) throw new Error(payload.error ?? "command failed");
  return payload.result as T;
}

export const snapshot = () => call<Snapshot>("snapshot");

/** Subscribe to engine events. Returns an unsubscribe function. */
export function subscribe(onEvent: (event: StationEvent) => void, onStatus: (up: boolean) => void): () => void {
  if (inTauri) {
    let dispose: (() => void) | undefined;
    let cancelled = false;
    void import("@tauri-apps/api/event").then(({ listen }) =>
      listen<StationEvent>("station-event", (e) => onEvent(e.payload)).then((un) => {
        if (cancelled) un();
        else dispose = un;
      }),
    );
    onStatus(true);
    return () => {
      cancelled = true;
      dispose?.();
    };
  }
  const source = new EventSource("/api/events");
  source.onopen = () => onStatus(true);
  source.onerror = () => onStatus(false);
  source.onmessage = (e) => {
    try {
      onEvent(JSON.parse(e.data) as StationEvent);
    } catch {
      /* ignore a malformed frame; the next snapshot resyncs */
    }
  };
  return () => source.close();
}

/** Ask the host shell to do something only a desktop shell can. */
export async function shell(action: string, args: Record<string, unknown> = {}): Promise<void> {
  if (!inTauri) return;
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("shell_action", { action, args });
}
