import { describe, expect, it } from "vitest";

import { experimentalSources, mergeMessages } from "./model";
import type { StationMessage } from "./types";

const message = (id: number, sentAtMs = id): StationMessage => ({
  id,
  threadId: id,
  parentId: null,
  from: "attention:rob",
  to: "operator:rob",
  deliveredTo: "operator:rob",
  primaryTo: "operator:rob",
  cc: [],
  deliveryRole: "to",
  kind: "operator-station-spike.escalation",
  attention: "next-checkpoint",
  requiresDisposition: true,
  requiresDispositionForCurrentRecipient: true,
  subject: `Message ${id}`,
  body: "body",
  metadata: null,
  sentAtMs,
  latestDisposition: null,
  actionable: true,
  ackPending: false,
});

describe("mergeMessages", () => {
  it("deduplicates by id and keeps newest first", () => {
    expect(mergeMessages([message(1), message(2)], [message(2, 20), message(3)]))
      .toEqual([message(2, 20), message(3), message(1)]);
  });
});

describe("experimentalSources", () => {
  const fingerprint = `sha256:${"a".repeat(64)}`;
  const metadata = JSON.stringify({
    extensions: {
      "operator-station-spike":
        "urn:telex:experimental:operator-station-spike:v1",
    },
    ext: {
      "operator-station-spike": {
        sourceMessages: [
          {
            id: 12,
            threadId: 9,
            from: "worker:demo",
            to: "attention:rob",
            subject: "Decision needed",
            sentAtMs: 100,
            storeFingerprint: fingerprint,
          },
        ],
      },
    },
  });

  it("accepts only the experimental namespace", () => {
    expect(experimentalSources(metadata, fingerprint)).toHaveLength(1);
    expect(
      experimentalSources(
        JSON.stringify({
          extensions: {
            "operator-station":
              "urn:telex:experimental:operator-station-spike:v1",
          },
          ext: { "operator-station": { sourceMessages: [] } },
        }),
        fingerprint,
      ),
    ).toEqual([]);
  });

  it("marks a mismatched store unavailable", () => {
    const [source] = experimentalSources(
      metadata,
      `sha256:${"b".repeat(64)}`,
    );
    expect(source?.resolution).toBe("unavailable-in-current-store");
  });

  it("keeps a matching-store source explicitly pending resolution", () => {
    const [source] = experimentalSources(metadata, fingerprint);
    expect(source?.resolution).toBe("eligible-for-resolution");
  });

  it("ignores malformed metadata", () => {
    expect(experimentalSources("{", fingerprint)).toEqual([]);
  });
});
