// @vitest-environment jsdom

import "@testing-library/jest-dom/vitest";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import App from "./App";
import type { StationMessage, StationState, ThreadView } from "./types";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

const fingerprint = `sha256:${"a".repeat(64)}`;
const escalation: StationMessage = {
  id: 2,
  threadId: 2,
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
  subject: "Choose a release path",
  body: "The operator agent recommends the safe path.",
  metadata: null,
  sentAtMs: 100,
  latestDisposition: null,
  actionable: true,
  ackPending: false,
};
const state: StationState = {
  config: {
    stationAddress: "operator:rob",
    ingressAddress: "attention:rob",
    storeFingerprint: fingerprint,
    telexVersion: "telex 0.1.0",
  },
  messages: [escalation],
  status: {
    phase: "ready",
    detail: null,
    courierState: "armed",
    station: {
      address: "operator:rob",
      occupied: true,
      health: "armed",
      detail: null,
    },
    ingress: {
      address: "attention:rob",
      occupied: true,
      health: "attended-push",
      detail: null,
    },
    diagnostics: [],
  },
};
const thread: ThreadView = {
  selected: escalation,
  thread: [{ message: escalation, dispositions: [] }],
  sources: [],
  rawMetadata: null,
};

describe("App", () => {
  beforeEach(() => {
    vi.mocked(listen).mockResolvedValue(() => {});
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === "initial_state") return state;
      if (command === "read_thread") return thread;
      if (command === "reply_to") return { id: 3 };
      if (command === "set_disposition") return { state: "handled" };
      if (command === "retry_courier") return null;
      throw new Error(`unexpected command: ${command}`);
    });
  });

  it("loads the feed and sends a reply through the Station command", async () => {
    render(<App />);

    expect(
      await screen.findByText("Choose a release path"),
    ).toBeInTheDocument();
    expect(await screen.findByText("Reply to operator agent")).toBeInTheDocument();

    fireEvent.change(
      screen.getByPlaceholderText("Type the decision or instruction..."),
      { target: { value: "Use the safe path." } },
    );
    fireEvent.click(screen.getByRole("button", { name: "Send reply" }));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("reply_to", {
        messageId: 2,
        body: "Use the safe path.",
      }),
    );
  });
});
