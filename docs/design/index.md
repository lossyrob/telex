# Telex Design Layer

The **system spec** for telex: the architecture, the normative contracts, and the
decision record that implementation is built against. This is the rigorous,
node-worker-edited design layer (distinct from the root-level vision/direction docs —
[`PRODUCT-THESIS.md`](../../PRODUCT-THESIS.md), [`TELEX.md`](../../TELEX.md),
[`DISPATCH.md`](../../DISPATCH.md)).

## Documents

- **[DESIGN.md](DESIGN.md)** — the architecture: the message fabric, the **local
  exchange** (per-user daemon) presence/transport model, addresses, leases, delivery,
  attention, and disposition.
- **[DECISIONS.md](DECISIONS.md)** — the append-only numbered decision log (ADRs). The
  load-bearing choices and their supersessions.
- **[daemon.md](daemon.md)** — the **normative daemon contract**: the local-exchange
  IPC/attendance protocol, authorization, the lease-epoch fence, the lifecycle contract
  + Status surface, daemon-native session ownership, the liveness model, the minimal
  upgrade floor, the gating tests, and the consolidated resolutions of the
  design-foundation open questions.
- **[ARCHITECTURE.md](ARCHITECTURE.md)** — the **visual on-ramp** to the daemon design:
  mermaid diagrams (component map, pull **and push** message delivery, restart/re-attach,
  station liveness, the single-writer epoch fence, authorization) that teach the
  local-exchange architecture above the contract. Non-normative; `daemon.md` governs.
- **[copilot-bridge-push.md](copilot-bridge-push.md)** — the **push-delivery** design: the
  generic daemon on-deliver exec primitive and the Copilot CLI session bridge that turns a
  delivered message into an agent turn, the alternative to the agent-armed `wait` pull for
  push-capable harnesses. Governed by
  [daemon.md sec.13.2](daemon.md#132-on-deliver-push-opt-in-harness-neutral),
  [DECISIONS.md ADR 0039](DECISIONS.md#0039--push-delivery-via-a-generic-on-deliver-exec--copilot-session-bridge),
  and the Copilot skill-ownership decision
  [DECISIONS.md ADR 0040](DECISIONS.md#0040--copilot-skill-is-binary-owned-the-plugin-skill-is-a-bootstrap).
  The harness-neutral content/layout boundary (root `SKILL.md` neutral; Copilot content
  nested under `copilot/`) is
  [DECISIONS.md ADR 0044](DECISIONS.md#0044--harness-neutral-root-skill-per-harness-content-nested-under-harness);
  see also [copilot-plugin-validation.md](copilot-plugin-validation.md).

## Reading order

1. `PRODUCT-THESIS.md` (root) — why telex exists and what it promises.
2. `DESIGN.md` — how the system is shaped.
3. `ARCHITECTURE.md` — the visual on-ramp (mermaid diagrams) to the local-exchange design.
4. `daemon.md` — the precise contracts `daemon-core` and downstream nodes implement.
5. `copilot-bridge-push.md` — how push delivery layers a harness bridge on the daemon's
   on-deliver exec (read after `daemon.md` sec.13.2).
6. `DECISIONS.md` — why each load-bearing choice was made.

## Open-question resolutions (design-foundation)

The eight open questions carried into the local-daemon `design-foundation` node — epoch
lifecycle, session presence + reaping, typed watch-pid, per-session PID, legacy cutover,
**sessionEnd removal proof** (no external registry; explicit membership via `Detach`/`Ack`), the Status freeze line, and crash durability — are resolved across
[daemon.md](daemon.md)'s normative sections (the one-time legacy cutover in
[DECISIONS.md](DECISIONS.md) ADR 0024), with the decisions recorded as ADRs 0014–0024. The
session-ownership and liveness model was subsequently revised to a minimal form by **ADR
0023** (unique `session_id` + explicit-only membership + non-destructive presence +
agent-acked delivery).

## Scope note

The design layer lives under `docs/design/` and is edited by node-worker sessions.
`SKILL.md` and `README.md` remain at the repository root (`SKILL.md` is embedded into
the `telex` binary and is the agent-facing usage guide, not part of this spec layer).
