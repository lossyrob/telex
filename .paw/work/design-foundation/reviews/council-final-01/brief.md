# Council brief — FINAL review of the produced design layer (telex #34)

## Why this council is open

This is the **final pre-PR review** of the design layer produced by the
`local-daemon / design-foundation` node. The architecture is **ratified** and the
**plan was already reviewed** by a prior council (GO-WITH-CHANGES, all findings folded
in). This review checks the **produced artifacts as written** before they become the
design-gate PR to `main`. It is **not** a re-litigation of the architecture or the plan.

## Decision vector (every round carries this)

> **Does the produced design layer faithfully and consistently realize the reviewed
> plan — all 9 deliverables and all 8 open-question resolutions at contract strength —
> with no internal inconsistencies, factual/logical errors in the written contracts, or
> faithfulness breaks, such that it is ready for the builder's design-gate?** Where it
> falls short, what is the smallest direction-preserving fix?

**Success condition (`deliberate`):** converge on a sharper, correct, internally
consistent artifact set. Reward concrete catches (a wrong claim, a contradiction, a
missing/weakened contract, a dead reference) with the smallest fix. Do **not** re-open
the ratified architecture or the already-resolved plan decisions; do **not** propose new
scope. This is design/writing only — **no production code** is a deliverable.

## What is ratified / settled (context, not authority — do NOT reopen)

- Per-user auto-spawned daemon ("local exchange"); zero persistent session processes;
  server-side lease-epoch fence; seen-dedup redesign; hook + typed watch-pid liveness
  with minimal stale-attendance/takeover and no idle-TTL teardown; daemon singleton
  identity; capability/version IPC; daemon-native session ownership; both backends;
  minimal upgrade floor early.
- **Preserved dissent (must remain honored):** no held-stream `SessionConnect`; **no
  verb renames** (`attach`/`detach`/`wait` kept); capability scope/rotation reserved.
- **Builder-directed decisions:** docs/design relocation (deviates from issue #34's
  "keep at root" — flagged, not silently done); "local exchange" metaphor; full DESIGN
  rewrite to the daemon end-state; `SKILL.md` stays root (binary-embedded) and its
  narrative cutover is deferred to `daemon-core` (so README/SKILL still describing the
  holder model is INTENTIONAL, not an inconsistency).

## The artifacts under review (read these)

Repo root: `C:/Users/robemanuele/proj/telex/telex-design-foundation`. Review the
committed design layer:

- `docs/design/daemon.md` — **the normative daemon contract (read fully; this is the
  core deliverable).**
- `docs/design/DESIGN.md` — rewritten architecture (local exchange).
- `docs/design/DECISIONS.md` — ADRs; check **0014–0021** (new) and the superseded/amended
  status lines on **0004, 0005, 0009, 0010, 0012, 0013**.
- `docs/design/index.md` — design-layer entry point.
- `PRODUCT-THESIS.md` (root) — exchange framing.
- For faithfulness grounding: the plan and the prior council synthesis —
  `.paw/work/design-foundation/Plan.md` and
  `.paw/work/design-foundation/reviews/planning/council-plan-01/synthesis.md`.

## The 9 deliverables (must all be present + substantive)

1. `DECISIONS.md` ADR(s) for the daemon split + the enumerated mechanisms + supersedes.
2. `DESIGN.md` update (station model -> daemon + one-shot verbs).
3. `PRODUCT-THESIS.md` update (no-server -> auto-spawned local daemon).
4. IPC/attendance protocol + authorization.
5. Daemon lifecycle contract + Status surface (+ gating tests).
6. Daemon-native session ownership (Register/Re-register/DeregisterSession).
7. Verb + docs/SKILL cutover decision (keep verbs; hide daemon entrypoint; single-source
   skill).
8. Minimal upgrade floor + legacy/non-epoch cutover rule.
9. Resolutions for the 8 open questions.

## The 10 plan-review CORE contracts that MUST be present at contract strength

The prior council required these; verify each is actually written into `daemon.md`/ADRs
at **>= the specified strength** (a contract shipping weaker than this is a CORE finding):

- **DA-1 (OQ5):** two-phase legacy cutover — **drain** (confirm no legacy waiter bound)
  **then claim** epoch=1 + occupant rotation; rotation-alone is insufficient; frozen
  assertion "no `Frame::Message` from a non-epoch holder reaches a recipient after the
  daemon's waiter binds."
- **DA-2 (OQ6):** v1 = **instance `admin_cap`** over user-private daemon IPC; per-session
  cap reserved + deferred (hook is a separate process; per-session cap not obtainable in
  v1).
- **DA-3 (OQ8):** **suspect/verified/lapsed** recovery state machine; daemon must NOT
  heartbeat/deliver for `suspect` rows; `wait` auto-Re-register on `UnknownSession`;
  idempotent Re-register.
- **DA-4 (OQ3/OQ4):** dismissal-path matrix (4 disjoint cases); **watch-pid death =
  immediate teardown** (internal DeregisterSession), bypassing `occupied_stale`;
  `occupied_stale` reserved for unobserved death; "no idle teardown" restated as "no
  time-based dismissal of a *live* session; positive death evidence = immediate teardown."
- **DA-5 (OQ2):** takeover **atomic at the daemon** (mint epoch + evict map + close IPC
  waiters + bind, single critical section); the intra-daemon takeover local-eviction
  gating test.
- **DA-6 (OQ2):** hook-semantics split — positive-presence refreshes `last_confirmed`;
  **sessionEnd does NOT refresh** (removal signal); failed sessionEnd records error, no
  refresh.
- **DA-7 (OQ1):** typed `mark_delivered_if_current_owner(...) -> {Delivered | NotOwner |
  AlreadyDelivered}` with the ordering invariant (non-`NotOwner` BEFORE any
  `Frame::Message`); ownership-loss-around-delivery gating scenario.
- **DA-8 (seen redesign):** durable `deliveries` = cross-epoch authority; in-memory
  bounded fast-path keyed `(recipient, message_id, lease_epoch)`; seed on claim; evict on
  durable-mark/terminal-disposition/epoch-transition; drop on epoch loss.
- **DA-9 (from-default):** daemon-era `ResolveFrom(TELEX_SESSION_ID)` against that
  session's addresses; never infer across sessions; supersedes 0010's mechanism.
- **DA-10 (single-source SKILL):** canonical root `SKILL.md`; CLI `include_str!`; plugin
  = manifest pointer or `exec telex skill --raw`; no divergent copy.

Plus: **Q-A** = 8 ADRs with a **Scope header on 0019**; **five** gating tests with
per-test observable assertions (OQ7 freeze); OQ4 resolved as "no usable per-session PID"
(empirically grounded); the relocations/supersedes/defers map present with deferred items
explicit.

## Coverage checklist (each surface examined by >=1 member)

1. **Faithfulness to the plan:** are DA-1..DA-10 + Q-A + the 5 gating tests present in the
   artifacts at contract strength? Any silently weakened?
2. **Internal consistency:** do `DESIGN.md`, `daemon.md`, and the ADRs agree (epoch
   fence, verbs, station recast, TTL narrowed)? Any doc describing a superseded mechanism
   as *current* that is NOT an intentional historical ADR body or the deferred-cutover
   README/SKILL?
3. **Correctness of the written contracts:** are the technical claims sound — the epoch
   CAS/self-demote logic, the `mark_delivered_if_current_owner` ordering, the two-phase
   cutover proof, suspect/verified/lapsed, the admin_cap flow, Postgres-reclaim-in-epochs?
   Any logical gap, contradiction, or hand-wave that would block `daemon-core`?
4. **Completeness:** all 9 deliverables substantive; all 8 OQs resolved with *implementable*
   specifics; deferred items explicit; relocations map complete.
5. **ADR quality:** supersedes/amends correct and bidirectionally coherent
   (0004->0014, 0005/0010/0012/0013 status, 0009 recast); numbering clean 0001..0021;
   append-only respected (historical bodies not rewritten); Scope header on 0019.
6. **Doc architecture + links:** the `docs/design/` migration is coherent; `index.md`
   accurate; cross-references resolve; `SKILL.md` correctly left at root/binary-embedded;
   no dead links.
7. **Faithfulness to ratified + preserved dissent:** verbs not renamed; no SessionConnect;
   scope/rotation reserved; nothing silently contradicts a ratified decision.
8. **New errors introduced in the writing:** any wrong, ambiguous, or misleading statement
   a careful reader/implementer would trip on (this is the retrospective lens).
9. **Node outcome anchor + PR framing:** is the anchor (all 9 + 8) actually satisfied so
   the PR can use `Closes #34`? Is the docs/design deviation correctly flagged (not
   silently done)?

## Constraints / non-goals / parked

- **Design/writing only — no production code.** Do not propose code as a fix.
- Do not relitigate the ratified architecture or the resolved plan decisions.
- PARKED (record, do not steer): full non-binary status policy; fd-over-IPC; daemon-owned
  directory reads; per-session cap tiers; the README/SKILL narrative cutover (deferred to
  daemon-core by design).

## Roster, depth, mode

- **Mode:** `deliberate` (panel -> one focused interaction round if material disagreement;
  convergence-toward-correct-artifacts).
- **Depth:** `medium` (3 members, isolated panel + up to 2 interaction rounds).
- **Members** (all wear the **general-reviewer** persona — senior generalist reviewer /
  rubber duck on correctness, faithfulness, consistency, completeness, integration risk
  for the downstream `daemon-core` implementer; do NOT substitute a built-in `all`
  roster):
  - **gr-premortem** — general-reviewer + **premortem** overlay (assume the design-gate PR
    failed / `daemon-core` got stuck; what in the *written artifacts* caused it?). Model:
    `gpt-5.5`.
  - **gr-retrospective** — general-reviewer + **retrospective** overlay (a `daemon-core`
    implementer built against these docs; where were they misled, stuck, or forced to
    re-decide?). Model: `gemini-3.1-pro-preview`.
  - **gr-baseline** — general-reviewer, baseline lens (holistic correctness + faithfulness
    + completeness + consistency). Model: `claude-opus-4.7` (reasoning_effort high).
- **Independent rapporteur:** the runner; a non-author (preferably non-Claude) member adds
  the `faithfulness_check`.

## Required output

Write `synthesis.md` (same schema as the planning council). `recommendation` is
**GO / GO-WITH-CHANGES / REVISE** for the **PR**. Flush each member turn to
`transcript.md`. Return ONLY the synthesis path + a short meta report (rounds, members +
requested models + self-reported provider_family, nesting outcome, errors). Classify every
finding CORE/ADJACENT/PARKED; only a CORE finding with proof may be PR-blocking. Each
member emits the structured turn (epistemic_act, key_claim, confidence, grounds, warrant,
rebuttal_conditions, relevance, smallest_change, what_gets_smaller). For any CORE finding,
cite the exact file + line/section so the fix is mechanical.
