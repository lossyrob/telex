# Council brief — plan review for local-daemon / design-foundation (telex #34)

## Why this council is open

A **design-foundation** node must produce the design layer for replacing telex's
per-session "holder" with an **auto-spawned per-user local daemon**. The architecture
is already **ratified** (decision ledger + a prior spar round + two council reviews —
treat as settled context, NOT up for re-decision). A **plan** has been written for the
write-up node. This council pressure-tests **the plan** and **the open-question
resolutions it proposes** before execution, replacing a lighter spar step. The plan
gates an entire downstream workstream (it is the "design-gate"), so getting it right is
consequential.

## Decision vector (every round carries this)

> **Is this plan sound, complete, and faithful enough that executing it produces a
> design-gate-ready telex design layer — resolving all 9 deliverables and all 8 open
> questions with implementable specifics — WITHOUT re-deciding the ratified
> architecture?** Where are the CORE gaps, faithfulness breaks, or under-specified
> open-question resolutions, and what is the smallest direction-preserving fix for
> each?

**Success condition (this is a `deliberate`, not a `debate`):** convergence toward a
sharper, more complete plan is the goal. Reward sharper specifics on the
open-question resolutions and concrete gap-fills. Do **not** stage disagreement, and
do **not** relitigate ratified architecture. Vagueness/dissolved specifics is the
failure mode to guard against, not premature agreement.

## Node outcome anchor (must be preserved)

All **9 deliverables** + **8 open-question resolutions** produced as a rigorous,
internally consistent design layer, ending in a PR to `main`. This is **design/writing
only — no production code.** The plan must still END in the required design evidence,
not merely enabling work.

## What is RATIFIED (context, not authority — do NOT reopen)

These are settled. Challenge them only by proving the *plan* cannot succeed without
revising them (that would be a `stop` with proof, and should be rare):

- Per-user auto-spawned daemon owns presence + transport for all locally-attended
  addresses; **zero persistent session processes** (one-shot register/wait/release).
- **Server-side lease-epoch fence**: a monotonic `lease_epoch` + `owner_instance_id`;
  epoch-guarded heartbeat/release AND delivery emission
  (`mark_delivered_if_current_owner` — no message frame unless the daemon owns the
  epoch); self-demote on 0-row heartbeat; ordered handoff; remove occupant-null release.
- **`seen`-dedup redesign** for a long-lived daemon (today `seen` is an in-memory
  `HashSet<i64>` that is never pruned *because holders restart* — a long-lived daemon
  voids that).
- **Liveness:** sessionEnd hook = healthy disconnect; typed `--watch-pid` = ungraceful
  backstop (v1 floor = loader anchor + start-time); **no idle-TTL teardown**; minimal
  **stale-attendance/takeover** (`last-confirmed`, `occupied_stale`, operator takeover)
  as a load-bearing recovery path.
- **Daemon singleton identity** = user SID + config root + protocol-major.
- **Daemon-scoped capability + version-handshake IPC** (Hello/HelloAck; scoped
  capability auth; one token v1, scope/rotation reserved).
- **Daemon-native session ownership**: in-memory `session_id -> addresses` is the
  authority; Register/Re-register/DeregisterSession; reshapes the filesystem
  `session_registry` (drop it as authority, reuse hook plumbing).
- **Fencing-first sequencing**; both SQLite + Postgres in the deliverable; a minimal
  upgrade floor lands early.
- Durable buffer reuse of decisions 0011/0013 (the `deliveries` table).
- **Preserved dissent (do NOT drop):** no held-stream `SessionConnect` liveness; **no
  verb renames**; record capability scope/rotation fields now but defer tiers.

## The plan under review

Read in full: **`.paw/work/design-foundation/Plan.md`** (relative to repo root
`C:/Users/robemanuele/proj/telex/telex-design-foundation`). It proposes:

- A two-layer doc split: **`docs/design/`** (system spec: `index.md`, migrated
  `DESIGN.md` + `DECISIONS.md`, new `daemon.md` normative contract) vs **repo root**
  (loose vision: `PRODUCT-THESIS.md`, `TELEX.md`, `DISPATCH.md`, `README.md`,
  `SKILL.md`). **NOTE this deviates from issue #34's explicit "keep the design layer at
  the repo root"** — the builder approved the deviation live; the plan flags it for
  orchestrator reconciliation and does not edit the issue.
- "**Local exchange**" as the telex-metaphor anchor for the daemon; "station" recast
  from "resident holder + waiter" to "a registration in the local exchange."
- A **full rewrite** of `DESIGN.md` to the daemon end-state (builder-approved;
  rationale: private repo ships before opening).
- **8 ADRs (0014–0021)** extending the numbered series, each pointing into `daemon.md`.
- Single-threaded authoring (no fleet), `daemon.md` as the spine.
- Detailed **resolutions for all 8 open questions** (reproduced below).

## The 9 deliverables (from issue #34)

1. `DECISIONS.md` ADR(s): daemon presence/transport split; zero persistent session
   processes; server-side lease-epoch fence; `seen`-dedup redesign; minimal
   stale-attendance/takeover; typed watch-pid; daemon singleton identity;
   capability/version-handshake IPC; daemon-native `DeregisterSession`; verb/docs
   cutover; how this supersedes/relocates 0004, #5/#17, #3; record deferred items.
2. `DESIGN.md` update (station model -> daemon + one-shot verbs).
3. `PRODUCT-THESIS.md` update (no-server -> auto-spawned local daemon).
4. IPC/attendance protocol + authorization spec.
5. Daemon lifecycle contract + Status surface (+ 4 gating tests: concurrent first-use,
   crash-during-`wait`, competing daemons, handoff duplicates).
6. Daemon-native session ownership (Register/Re-register/DeregisterSession; #31 reshape).
7. Verb + docs/SKILL cutover decision (keep verbs; docs update WITH daemon-core; hide
   daemon entrypoint; single-source skill mechanism).
8. Minimal upgrade floor + legacy/non-epoch-lease cutover rule.
9. Resolutions for the 8 open questions.

## The 8 open-question resolutions to PRESSURE-TEST (the crux of this review)

These are reproduced from the plan. **This is where the council should spend most of
its focus** — are they implementable, internally consistent, and complete? (Full text
in `Plan.md` "Open-question resolutions".)

- **OQ1 Epoch lifecycle:** new `lease_epoch` + `owner_instance_id` columns; increment
  on claim/takeover (claim `epoch=current+1` conditioned on observed row); epoch-guarded
  heartbeat/release; 0-row heartbeat -> self-demote; server-side
  `mark_delivered_if_current_owner`; ordered handoff quiesce->flush->unbind->claim;
  Postgres reclaim expressed in epochs (higher-epoch claim under row condition), not
  timing; remove occupant-null release.
- **OQ2 Stale-attendance + takeover (no teardown):** `attendance_last_confirmed_at`
  updates on register/wait-connect/hook; `occupied_stale` derived
  (now-last_confirmed > configurable `stale_after`); never tears down; operator takeover
  = privileged RPC minting a new epoch, allowed once `occupied_stale`, reported with
  prior occupant + last-confirmed.
- **OQ3 Typed `--watch-pid`:** anchor (any sufficient) vs required (all necessary) + a
  pid+start-time reuse guard (today `process_alive` is pid-only); v1 floor = loader
  anchor + start-time; expose required/anchor flags only with a real consumer.
- **OQ4 Distinct per-session PID? RESOLVED NO (empirically grounded):** live probe
  (Copilot CLI 1.0.64-1, Windows) shows `copilot.exe` is a supervisor that re-execs an
  identical-argv inner worker; the inner PID is NOT env-exposed AND spawns lazily (fresh
  idle sessions are loader-only), so it is not reliably capturable at register time and
  finding it needs the ppid-walk the design rejects. -> loader anchor + start-time is
  the sole env-sourced backstop; hook is the healthy-dismiss path; stale-attendance/
  takeover is the load-bearing unhooked-dismiss recovery.
- **OQ5 Legacy/non-epoch cutover:** a lease row with no `lease_epoch` is a legacy
  holder; first daemon-aware claimant treats NULL/absent epoch as epoch 0 and claims
  epoch 1 under the row condition, fencing legacy out; legacy holders self-demote on
  first 0-row heartbeat. Hard cutover acceptable.
- **OQ6 DeregisterSession proof (no external registry):** daemon owns session->addresses
  in memory; hook presents a scoped capability bound to daemon-instance + session,
  minted at Register, held in the plugin/session env (not filesystem); fallback
  instance-admin capability for the user-private same-trust case.
- **OQ7 Status freeze line:** design-foundation freezes the Status *contract shape*
  (field set + meaning: epoch, instance, attendees w/ last-confirmed/stale, backoff,
  recent errors, protocol version) + the 4 gating tests' observable assertions;
  daemon-core owns exact rendering/format/diagnostic depth.
- **OQ8 Attendance durability across daemon crash:** durable = lease rows (incl. epoch/
  owner_instance/last_confirmed) + durable delivery buffer; rebuilt-by-re-register =
  in-memory session->addresses + live watch-pid set + IPC waiters; respawn re-reads
  leases under a new epoch and clients re-register; a session ending while the daemon is
  down is recovered via TTL daemon-down backstop and/or higher-epoch fence + stale-
  attendance/takeover (no permanent zombie).

## Plan-level open questions the council should weigh in on

- **Q-A — ADR granularity:** is 8 ADRs (0014–0021) right, or should some merge given
  the log's "keep entries short" convention?
- **Q-B — Verb vocabulary:** issue/brief say register/deregister/wait one-shot verbs;
  shipped CLI verbs are `attach`/`detach`/`wait` and "no rename" is preserved dissent.
  Plan resolves: keep `attach`/`detach`/`wait` as CLI verbs (now one-shot against the
  daemon); Register/Re-register/DeregisterSession are the IPC-protocol operations.
  Sound, or a contradiction?
- **Q-C — `TELEX.md` touch:** extend the historical-telex narrative with "local
  exchange," or leave root vision docs untouched?
- **Q-D — Issue-amendment mechanism:** plan documents the docs/design deviation in the
  PR + field report (not editing issue #34's body, per authority limits). Correct?

## Code-seam grounding (verified facts the design must respect)

- Holder loop (`src/commands/attach.rs`): sends the `Frame::Message` to the waiter at
  ~line 477 **before** `mark_delivered` commits at ~line 485 = the double-delivery
  hazard the server-side epoch fence must close. `seen` is `Mutex<HashSet<i64>>`,
  deliberately never pruned (ADR 0013).
- Lease is keyed by **address only**; **no epoch / owner-generation column exists
  today** (`src/registry.rs` `HolderRecord` has none; backend `release_lease` deletes
  `WHERE address=? AND (occupant=? OR occupant IS NULL)` — the occupant-null release to
  remove).
- IPC (`src/ipc.rs`) endpoint is **address-keyed**; `Wait`/`Shutdown` are
  **unauthenticated**; JSON-lines over serde; the daemon model needs a daemon-scoped
  (not address-keyed) endpoint with a version handshake + capability auth.
- `wait` exit codes: 0 delivered, 2 idle-timeout, 3 holder-gone, 4 holder-hung
  (`src/commands/wait.rs`).
- `session_watch::process_alive` is **pid-only** (no start-time) today.
- Greenfield: **no `migrations/` dir** (schema via `CREATE TABLE IF NOT EXISTS`);
  pre-first-non-beta, single-user.
- The sessionEnd hook plumbing exists on branch `feature/copilot-session-end-plugin`
  (`src/session_registry.rs` filesystem registry + `src/commands/session_end.rs` +
  `integrations/copilot-cli/{plugin.json,hooks.json}`); the plan reshapes the
  filesystem registry out as authority.

## Source documents (members may read for depth)

All paths relative to `C:/Users/robemanuele/proj/telex/telex-design-foundation`:

- `.paw/work/design-foundation/Plan.md` — **the plan under review (read fully).**
- `.streamliner/workstreams/local-daemon/tasks/design-foundation.md` — node spec.
- `.streamliner/workstreams/local-daemon/brief.md` — workstream brief.
- `.streamliner/workstreams/local-daemon/docs/initial-shaping.md` — the full ratified
  decision ledger + Spar R1 + Council outcomes (authoritative ratified input).
- `DESIGN.md`, `DECISIONS.md` (esp. 0004, 0005, 0010, 0011, 0012, 0013),
  `PRODUCT-THESIS.md`, `SKILL.md` — the existing design layer being extended.

## Constraints / non-goals / parked

- **Design/writing only — no production code.** Do not propose code as a deliverable.
- **Keep the design layer reusable + at the chosen location** (root vs docs/design is
  decided = docs/design for the system spec; do not relitigate beyond Q-D's mechanism).
- telex core stays **harness-agnostic**; Copilot specifics live only in the plugin.
- **PARKED (record, do not steer the recommendation):** the full non-binary occupant
  status policy; fd-over-IPC pid-reuse-immune backstop; daemon-owned directory/occupancy
  reads. The ratified architecture itself.

## Coverage checklist (each surface must be examined by >=1 member)

1. **Faithfulness to the node outcome anchor** — does the plan still END in all 9
   deliverables + 8 OQ resolutions, or does any get silently downgraded to enabling
   work?
2. **OQ1/OQ5/OQ8 (the epoch + crash + cutover triad)** — are the epoch lifecycle,
   legacy cutover, and crash-durability resolutions mutually consistent and
   implementable against "lease keyed by address only, no epoch column today, both
   backends, Postgres MVCC commit-order"? Any race or flip-flop left open?
3. **OQ2 stale-attendance/takeover** — is the threshold + takeover-mints-new-epoch flow
   safe and free of teardown? Interaction with OQ1 fencing.
4. **OQ6 DeregisterSession proof** — is a capability "held in the plugin/session env"
   actually obtainable without an external registry, given the hook runs as a separate
   short-lived process? Is the fallback sound?
5. **OQ3/OQ4 watch-pid + the empirical OQ4=no resolution** — is the loader-anchor floor
   + start-time guard sufficient given lazy inner-process spawn? Any over-claim?
6. **OQ7 Status freeze line** — is "freeze the field set, defer the rendering" a clean,
   testable boundary, or does it leave daemon-core under-specified / over-constrained?
7. **Document architecture** — is the root vs docs/design split + `daemon.md`-as-spine
   + 8-ADR structure coherent and maintainable? Migration/link-fix risk? `SKILL.md`
   binary-embed correctly excluded?
8. **Plan completeness/sequencing** — missing deliverable coverage, missing
   relocations/supersedes/defers, single-threaded vs fleet, definition-of-done
   adequacy, internal-consistency risk across the interlocking docs.
9. **Faithfulness vs ratified + preserved dissent** — does anything in the plan
   silently contradict a ratified decision or the preserved dissent (no SessionConnect,
   no verb rename)?

## Roster, depth, mode

- **Mode:** `deliberate` (panel -> one focused interaction round if material
  disagreement remains; convergence-toward-built-recommendation success condition).
- **Depth:** `medium` (3 members, isolated panel + up to 2 interaction rounds; stop at
  sharper convergence or a named irreducible).
- **Members** (all wear the **general-reviewer** persona — a broad senior generalist
  reviewer / rubber duck focused on correctness, plan fit, missing assumptions,
  integration risk, maintainability, and whether the work still matches the node's
  intended outcome — differentiated by model + perspective overlay; do NOT substitute a
  built-in `all` specialist roster):
  - **gr-premortem** — general-reviewer + **premortem** overlay (assume the executed
    plan failed the design-gate; what most likely caused it?). Model: `gpt-5.5`.
  - **gr-retrospective** — general-reviewer + **retrospective** overlay (a downstream
    `daemon-core` implementer is now building against this design; where did it leave
    them stuck, ambiguous, or forced to re-decide?). Model: `gemini-3.1-pro-preview`.
  - **gr-baseline** — general-reviewer, baseline lens (holistic correctness + faithful-
    ness + completeness). Model: `claude-opus-4.7` (reasoning effort: high).
- **Independent rapporteur:** the runner authors the synthesis as an independent
  rapporteur; a non-author member adds the one-line `faithfulness_check`.

## Required output

Write `synthesis.md` as the council synthesis packet (schema below), flushing each
member turn to `transcript.md` as it happens. Return to the driver ONLY the synthesis
path + a short meta report (rounds run, members + requested models + self-reported
provider_family, nesting outcome, any errors). Do not return round contents.

Synthesis packet fields (required): `decision_vector`, `recommendation`
(GO / GO-WITH-CHANGES / REVISE), `confidence`, `convergence`, `confidence_basis`,
`decisive_arguments` (claim + source_agents + evidence), `minority_report`, `parked`,
`coverage_manifest` (each of the 9 checklist surfaces -> core-finding / examined-passed
/ not-examined + by whom), `open_questions`, `reopen_conditions`, `audit_triggers`,
`faithfulness_check` (by a non-author member: SUPPORT/DISPUTE + one line),
`provenance_manifest` (per member: requested_model, persona, self-reported
provider_family, model_identity: UNVERIFIED, fallback_suspected), and the artifact
paths. Classify every finding CORE / ADJACENT / PARKED against the decision vector;
only a CORE finding with proof may be recommendation-blocking. Each member emits the
structured turn (epistemic_act, key_claim, confidence, grounds, warrant,
rebuttal_conditions, relevance, smallest_change, what_gets_smaller).
