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

## Reading order

1. `PRODUCT-THESIS.md` (root) — why telex exists and what it promises.
2. `DESIGN.md` — how the system is shaped.
3. `daemon.md` — the precise contracts `daemon-core` and downstream nodes implement.
4. `DECISIONS.md` — why each load-bearing choice was made.

## Open-question resolutions (design-foundation)

The eight open questions carried into the local-daemon `design-foundation` node — epoch
lifecycle, session presence + reaping, typed watch-pid, per-session PID, legacy cutover,
DeregisterSession proof, the Status freeze line, and crash durability — are resolved in
[daemon.md](daemon.md) (see its "Open-question resolutions" section), with the
decisions recorded as ADRs 0014–0023 in [DECISIONS.md](DECISIONS.md). The
session-ownership and liveness model was subsequently revised to a minimal form by **ADR
0023** (unique `session_id` + explicit-only membership + non-destructive presence +
agent-acked delivery).

## Scope note

The design layer lives under `docs/design/` and is edited by node-worker sessions.
`SKILL.md` and `README.md` remain at the repository root (`SKILL.md` is embedded into
the `telex` binary and is the agent-facing usage guide, not part of this spec layer).
