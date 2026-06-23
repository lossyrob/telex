# Plan — local-daemon / design-foundation (issue #34)

> paw-lite plan artifact. Source of truth for work-item progress. Design/writing
> only — no production code. Ends in a PR to `main` that **is** the design-gate.

## Node outcome anchor

Produce the **9 deliverables** and resolve the **8 open questions** for the
local-daemon architecture as a rigorous, internally consistent design layer, ending
in a PR to `main` (the design-gate). The architecture is ratified
(`docs/initial-shaping.md` ledger + Spar R1 + Council outcomes) — this node **writes
it up** and **resolves the open questions with implementable specifics**; it does not
re-decide the architecture. `Closes #34` only if all 9 + 8 land coherently; otherwise
`Refs #34` with explicit partial state.

## Approach Summary

The work is **document production grounded in verified code seams**. Architecture is
decided; the deliverables interlock heavily (the epoch fence, the IPC protocol, the
lifecycle contract, attendance, and session ownership all reference each other), so
the docs are authored **single-threaded for one coherent voice and clean
cross-referencing** rather than fleet-dispatched. Code seams are already gathered and
recorded as field notes (grounding citations below).

### Document architecture (where each deliverable lands)

Builder-directed two-layer split (shaping decision, deviates from the issue's
"keep at repo root" — see Key Decisions D1):

- **`docs/design/` — the actual system spec** (the Streamliner-described doc type,
  edited by node workers, manifest-aligned):
  - `docs/design/index.md` — **NEW.** Design-layer entry point (the
    `docs/design/index.md` the launch manifest expects); maps the layer and the
    open-question resolutions.
  - `docs/design/DESIGN.md` — **MIGRATED + REWRITTEN** to the daemon end-state
    (deliverable 2; Key Decision D3 = full rewrite).
  - `docs/design/DECISIONS.md` — **MIGRATED + EXTENDED** with ADRs 0014+
    (deliverable 1).
  - `docs/design/daemon.md` — **NEW.** The **normative daemon contract** that
    `daemon-core` implements against (deliverables 4, 5, 6, 8 + the gating tests +
    the relocations/supersedes/defers map + the consolidated OQ resolutions).
- **Repo root — loose, ad-hoc vision/direction anchors** (evolve outside this node):
  - `PRODUCT-THESIS.md` — **UPDATED** to the local-exchange framing (deliverable 3).
  - `TELEX.md`, `DISPATCH.md`, `README.md`, `SKILL.md` — **stay at root.**
    `README.md` gets link fixes only. `SKILL.md` is **binary-embedded**
    (`telex skill` -> `include_str!` in `src/commands/skill.rs`), so it must not move
    (moving needs code = out of scope) and its holder/waiter narrative cutover is
    explicitly deferred to `daemon-core` (deliverable 7).

### Telex-metaphor anchor (Key Decision D2)

The per-user daemon is framed as the **local exchange** — the historical telex
switching center that connected stations/numbers. "Daemon" stays the precise
technical term; `telex daemon` is the hidden entrypoint; **"station" (ADR 0009) is
recast** from "resident holder + waiter" to "a registration in the local exchange"
so the vocabulary survives the holder's removal.

## Work Items

- [x] **WI-1 — Establish `docs/design/` + migrate.** `git mv DESIGN.md DECISIONS.md
  docs/design/`; create `docs/design/index.md` skeleton; fix inbound links
  (`README.md`, `PRODUCT-THESIS.md`, inter-doc refs, `brief.md`/`tasks` only if they
  point at moved files — leave `.streamliner/` workstream docs as-is unless broken).
  Verify the `SKILL.md` embed path is untouched. (D1; structural prerequisite.)
- [x] **WI-2 — Author `docs/design/daemon.md` normative spec.** The core contract.
  Sections: local-exchange overview; singleton identity + auto-spawn; lifecycle state
  machine + Status surface (OQ7 frozen field set); daemon-scoped IPC protocol + version
  handshake; scoped-capability authorization (**admin-cap v1 per DA-2**; `per_session_cap`
  reserved); attendance model + record shape + **suspect/verified/lapsed recovery state
  machine (DA-3)** + **dismissal-path matrix (DA-4)** + **atomic takeover (DA-5)** +
  **hook-semantics split (DA-6)**; lease-epoch fence + epoch lifecycle + **typed
  `mark_delivered_if_current_owner` result contract + ordering invariant (DA-7)** +
  ordered handoff + Postgres reclaim; delivery + **`seen`-dedup redesign shape (DA-8)**;
  daemon-native session ownership + **daemon-era `from`-resolution rule (DA-9)** + `wait`
  auto-Re-register; liveness + typed watch-pid; **single-source SKILL mechanism (DA-10)**;
  minimal upgrade floor + **two-phase legacy cutover (DA-1)**; the **five gating tests**
  with per-test observable assertions; **OQ-γ sessionResume-scope note**; relocations/
  supersedes/defers map; consolidated OQ resolutions. (Deliverables 4, 5, 6, 8, parts of
  1/9.)
- [x] **WI-3 — Rewrite `docs/design/DESIGN.md`** to the daemon end-state: local
  exchange as design center; recast Station; replace the resident-holder/waiter-loop
  and local-holder-registry sections; point detail into `daemon.md`; keep
  telex/station vocabulary coherent. (Deliverable 2; D3.)
- [x] **WI-4 — Update `PRODUCT-THESIS.md`** — "one small binary, no server" ->
  "one small binary + an auto-spawned local exchange (daemon)"; zero-config/implicit
  framing (like `rust-analyzer`/`gopls`); keep it vision-level. (Deliverable 3.)
- [x] **WI-5 — Author ADRs 0014–0021 in `docs/design/DECISIONS.md`.** Concise
  decision records + supersessions, each pointing into `daemon.md`. **8 ADRs confirmed
  (Q-A resolved)**:
  - 0014 — Per-user local daemon (local exchange); zero persistent session processes.
    *Supersedes 0004; recasts 0009 station; moots #3.*
  - 0015 — Server-side lease-epoch fence + ordered handoff + epoch lifecycle +
    **typed `mark_delivered_if_current_owner` contract (DA-7)**. *(OQ1)*
  - 0016 — `seen`-dedup redesign for a long-lived daemon **(shape per DA-8)**.
    *Supersedes 0013's never-prune `seen`; elevates #26.*
  - 0017 — Liveness: sessionEnd hook + typed `--watch-pid` + **dismissal-path matrix
    (DA-4)**; minimal stale-attendance/takeover; no idle-TTL teardown. *(OQ2, OQ3, OQ4);
    supersedes 0012; narrows 0005 TTL to the daemon-down backstop.*
  - 0018 — Daemon singleton identity + lifecycle contract + Status surface. *(OQ7)*
  - 0019 — **Scope header (Q-A):** covers (a) daemon-scoped capability/version-handshake
    IPC (**admin-cap v1, DA-2**) **and** (b) daemon-native session ownership +
    **`from`-default rule (DA-9)** + **suspect/verified/lapsed recovery (DA-3)**; note
    splitting considered + declined for brevity. *(OQ6, OQ8); reshapes #23/#31;
    supersedes 0010's local holder registry as the `from`-default source.*
  - 0020 — Minimal upgrade floor + **two-phase legacy/non-epoch-lease cutover (DA-1)**.
    *(OQ5)*
  - 0021 — Verb + docs/SKILL cutover (**single-source SKILL mechanism, DA-10**);
    design-layer relocation to `docs/design/`. *Records D1.*
- [x] **WI-6 — Consolidated OQ resolutions + relocations map.** A clearly labeled
  section (in `daemon.md`, surfaced from `index.md`) resolving all 8 OQs with
  implementable specifics and pointing to where each is specified; plus the explicit
  relocate/supersede/defer table across #32/#23/#31/#5/#17/#3/#26/#27/#24/#6/#28/#33.
  (Deliverable 9 + the issue's "record deferred items explicitly.")
- [x] **WI-7 — Consistency + cross-reference audit + index finalize.** Whole-layer
  read for internal consistency (no doc describes a superseded mechanism as current
  except where explicitly framed as the v0-being-replaced); finalize `index.md`;
  verify every cross-link resolves; confirm no production code touched; draft the
  issue-amendment note (D1 deviation) for the PR + field report.

> Execution mode: **single-threaded** (WI-1 first as a structural prerequisite, then
> WI-2 as the spine, then WI-3/4/5/6 against it, then WI-7). No fleet dispatch — design
> coherence beats parallelism here. No `cargo build/test` (no code). Markdown only.

## Open-question resolutions (the 8, with the intended specifics)

These are the substance the design must land. **Sharpened by the plan-review council
(GO-WITH-CHANGES, HIGH; `reviews/planning/council-plan-01/synthesis.md`)** from
slogan-level to implementable-contract level — the council's 10 CORE findings
(DA-1..DA-10) are folded in below with their tags. All sit within the ratified
architecture; the node outcome anchor is unchanged.

- **OQ1 — Epoch lifecycle.** Monotonic `lease_epoch` + `owner_instance_id` on the
  lease row (new columns; today the lease is keyed by `address` only with no owner
  generation). Epoch **increments on claim/takeover** (a new daemon claims
  `epoch = current + 1` atomically, conditioned on the observed row). Heartbeat and
  release are **epoch-guarded** (`WHERE address=? AND epoch=? AND owner=?`) and **must
  return rowcount**; a 0-row heartbeat means a higher epoch exists -> the daemon
  **self-demotes** (closes its waiters, stops emitting). **Delivery emission is
  epoch-fenced server-side** via a **typed backend contract (DA-7):**
  `mark_delivered_if_current_owner(address, owner_instance_id, lease_epoch, message_id)
  -> Result<{Delivered | NotOwner | AlreadyDelivered}>`. The daemon **must receive a
  non-`NotOwner` result BEFORE emitting any `Frame::Message`**: `NotOwner` -> self-demote
  (mirror of 0-row heartbeat), `AlreadyDelivered` -> no-op (idempotency), `Delivered` ->
  permit emission. This closes the verified `attach.rs:477/485` hazard (frame shipped
  before `mark_delivered` commits). **Ordered handoff**: quiesce -> flush pending
  `mark_delivered` -> unbind -> claim new epoch (no TTL gap). **Postgres cross-machine
  reclaim is expressed in epochs, not timing**: a reclaiming daemon wins by claiming a
  higher epoch under the same row condition; the loser self-demotes on its next 0-row
  heartbeat. SQLite-local is the simple single-writer case; `postgres-parity` proves it
  under competing daemons. Remove the occupant-null release path. **Gating tests** add an
  **ownership-loss-around-delivery** scenario (ownership rotates between the
  `mark_delivered_if_current_owner` call and its response -> assert `NotOwner`, daemon
  self-demotes, no duplicate downstream frame) and extend handoff-duplicates to exercise
  it.
- **OQ2 — Stale-attendance threshold + takeover (no teardown).**
  `attendance_last_confirmed_at` is refreshed by **positive-presence signals only
  (DA-6):** `Register`, each authenticated `wait` connect, and any future positive
  resume/connect hook. **`sessionEnd` does NOT refresh** — it is a removal signal
  (release addresses / drop session membership); a **failed** `sessionEnd` records a
  recent error and leaves `last_confirmed` unchanged (no refresh, no reanimation
  window). `occupied_stale` is a **derived flag** (now − last_confirmed >
  configurable `stale_after`, default a small multiple of the heartbeat/lease window —
  exact default in `daemon.md`), reserved for the **unobserved-death case** (no hook,
  no watch-pid signal — see OQ3 dismissal matrix), surfaced in Status and
  `address list`. It **never tears down** — idle-but-alive stays `occupied` and
  wakeable. Operator **takeover** is **atomic at the daemon (DA-5):** in one critical
  section it mints a new backend epoch, **evicts the prior `session_id -> addresses`
  entry** for the rotated address, **closes IPC waiters** bound under the prior
  occupant, and binds under the new occupant — no observable intermediate state.
  Allowed once `occupied_stale`, reported with prior occupant + last-confirmed. A
  gating test (**intra-daemon takeover local-eviction**) asserts the old-occupant
  `wait` gets a defined error/disconnect (not a silent hang) and a new-occupant `wait`
  receives subsequent messages.
- **OQ3 — Typed `--watch-pid` + dismissal-path matrix.** Generalize singular
  `--session-pid` (#5/#17) into typed predicates: **anchor** (alive if any sufficient
  pid survives) vs **required** (alive only if all necessary survive), plus a **pid +
  start-time reuse guard** (today `process_alive` is pid-only). **v1 floor = loader
  anchor + start-time**; expose `required`/`anchor` flags only where a real consumer/
  test exists. Names stay harness-agnostic; the Copilot mapping lives in the plugin.
  **Dismissal-path matrix (DA-4) — 4 disjoint cases** (in `daemon.md`): (1) **sessionEnd
  hook** = clean exit (deregister); (2) **watch-pid failure** (anchor pid dead OR
  start-time mismatch) = crash/kill -> the daemon's local watcher issues an **internal
  `DeregisterSession` immediately, bypassing `occupied_stale`**; (3) **operator
  takeover** = override; (4) **daemon-down TTL** = backstop only. Restate "no idle-TTL
  teardown" precisely as **"no time-based dismissal of *live* sessions; positive death
  evidence triggers immediate teardown."** This keeps the empirical OQ4=no resolution
  recoverable without manual takeover.
- **OQ4 — Distinct per-session PID? RESOLVED: no (empirically grounded).** Live probe
  (Copilot CLI 1.0.64-1, Windows): `COPILOT_AGENT_SESSION_ID` + `COPILOT_LOADER_PID`
  are env-exposed; `copilot.exe` is a supervisor that **re-execs an identical-argv
  inner worker**, but the inner PID is **not** env-exposed **and spawns lazily** (fresh
  idle sessions are loader-only). So the inner pid is **not reliably capturable at
  register time**, and discovering it needs the **ppid-walk the design rejects**
  (ADR 0012). Therefore the **loader anchor + start-time** is the sole env-sourced
  backstop in v1; the sessionEnd hook is the necessary healthy-dismiss path; minimal
  stale-attendance/takeover is the load-bearing unhooked-dismiss recovery. This
  **reinforces** Council E; the typed framework's "additional per-session pid" slot is
  documented as *not reliably sourceable on Copilot CLI today*.
- **OQ5 — Legacy-holder / non-epoch-lease cutover rule (two-phase, DA-1).** A lease row
  with **no `lease_epoch`** (or `owner_instance_id`) is a **legacy holder**. Occupant
  rotation alone is **insufficient** — a legacy holder ships `Frame::Message`
  (`attach.rs:~477`) *before* its post-emit `mark_delivered` (`~485`) checks ownership,
  and its `heartbeat` returns `Result<()>` with **no rowcount** (`sqlite.rs:325-333`,
  `postgres.rs:313-320`), so it cannot observe self-demotion; if the daemon has rebound
  the address's waiter endpoint, two endpoints emit independently regardless of any
  post-emit row fence. So the cutover is **two-phase**:
  - **Phase 1 (drain):** the daemon-aware claimant detects the non-epoch row and, before
    binding its own waiter, confirms **no legacy holder is actively bound** — via an
    address-keyed IPC probe carrying a quit/handover signal, **or** by waiting a bounded
    stale-window. No live overlap of two waiter-binds per address.
  - **Phase 2 (claim):** only after drain, claim `epoch = 1` (treating NULL/absent epoch
    as epoch 0) under the row condition and **atomically rotate `occupant` ->
    `owner_instance_id`**, using owner/epoch-guarded heartbeat/release that returns
    rowcount. Remove the occupant-null release branch.
  - **Cutover gating assertion:** *no `Frame::Message` from a non-epoch holder reaches a
    recipient after the daemon's waiter binds.* Hard cutover for existing sessions is
    acceptable (ratified). *(Minority dissent preserved: gr-retrospective held rotation-
    alone suffices; adopted the two-phase position on the wire-level proof above —
    `synthesis.md` minority_report / OQ-α.)*
- **OQ6 — DeregisterSession proof — v1 = instance-admin capability (DA-2).** The daemon
  owns `session_id -> addresses` **in memory**. A per-session capability "held in the
  session env" is **not obtainable on Copilot CLI today**: `Register` runs in a child
  `attach`/loader process and the `sessionEnd` hook is a **separately spawned** process
  (`integrations/copilot-cli/hooks.json`; `session_end.rs` reads only a session id), so
  a child cannot mutate the parent harness env for a later hook. **v1 path:** the hook
  calls `DeregisterSession(session_id, admin_cap)`; the daemon verifies `admin_cap`
  against its instance secret and that `session_id` exists in its map, then drops the
  associated addresses. The protocol **reserves `per_session_cap: Option<Cap>`** as
  forward defense against lateral hook compromise, **deferred with rationale** (the v1
  threat model is same-trust user-private, making a per-session cap zero-marginal-value
  over the admin cap; "one token v1, scope/rotation reserved" is the ratified posture).
  **No filesystem session registry; no per-session cap minted at Register in v1.**
  Reshapes #23/#31: reuse the hook plumbing, drop `session_registry` as authority, hook
  is a thin mapper (`COPILOT_AGENT_SESSION_ID -> TELEX_SESSION_ID`).
- **OQ7 — Status freeze line.** `design-foundation` **freezes the Status contract
  shape** (the fields and their meaning: epoch, instance id, attendees with
  address/session/occupant/last-confirmed/stale, backoff/crashloop state, recent
  errors, protocol version) as the normative surface; `daemon-core` **acceptance** owns
  the exact rendering/format and the diagnostic depth. Frozen = the field set + the
  gating tests' **per-test observable assertions**; not frozen = wire format, verbosity,
  extra diagnostics.
- **OQ8 — Attendance durability across a daemon crash — suspect/verified/lapsed (DA-3).**
  **Durable** (survives crash, recovered from the backend): the **lease rows** (address,
  occupant, epoch, owner_instance, last_confirmed) and the **durable delivery buffer**
  (0011/0013). **Rebuilt by client re-register** (in-memory, lost on crash): the
  `session_id -> addresses` map, the live **watch-pid set** (pids + start-times), and
  IPC waiter registrations. A respawned daemon **must not freshen recovered rows as live
  attendance without proof** — an explicit attendance state machine in `daemon.md`:
  - **suspect** — every row recovered on respawn; the daemon **must NOT heartbeat or
    deliver** for `suspect` rows.
  - **verified** — promoted by a successful `Register`, `Re-register`, or authenticated
    `wait`-connect (valid `TELEX_SESSION_ID` + `admin_cap`); promotion refreshes
    `last_confirmed` under a **new epoch** and rebuilds the watch-pid set.
  - **lapsed** — recovered then aged out via daemon-down TTL or stale-attendance/takeover
    with no proof.
  - **`wait` client contract:** broken-pipe / `UnknownSession`-on-reconnect is a **normal
    control-flow path** — `wait` MUST **auto-Re-register** from `TELEX_SESSION_ID` +
    `admin_cap` in inherited env before failing (it is the only long-lived client able to
    re-prove a running session; loader's `attach` is one-shot). `Re-register` is
    **idempotent** (concurrent waits for one `session_id` converge to a single map entry;
    `daemon.md` picks last-writer-wins or union on the address set). A session that
    **ends while the daemon is down** lapses via the TTL daemon-down backstop (the one
    surviving TTL role) and/or is fenced by the respawned daemon's higher epoch — no
    permanent zombie. Add the suspect-row invariant to the crash-during-`wait` and
    competing-daemons gating tests.

### Deliverable-coverage resolutions (non-OQ, surfaced by the council)

Three sub-deliverables the OQ-focused surfaces would have missed — each closes a real
coverage hole, not an OQ:

- **`seen`-dedup redesign (DA-8; deliverable 1 / ADR 0016).** Durable
  `deliveries(message_id, recipient)` (UNIQUE-keyed, already cross-restart authority per
  0011/0013) is the **cross-epoch dedup authority** (no behavioral change to 0011/0013).
  The in-memory dedup becomes a **bounded fast-path keyed by `(recipient, message_id,
  lease_epoch)`** (in-flight identity), **seeded from `fetch_undelivered` on claim**,
  and **reset/dropped on epoch loss** (self-demote, takeover); evict an entry on durable
  mark (`mark_delivered_if_current_owner` -> `Delivered`), terminal disposition, or
  epoch transition. This replaces the never-pruned unbounded `Mutex<HashSet<i64>>` that
  relied on holder restart (ADR 0013) — voided by a long-lived daemon.
- **Daemon-era `from`-default rule (DA-9; deliverable 6 / ADR 0019).** ADR 0019
  supersedes 0010's local-holder-registry as the `from`-default source, so it must name
  the replacement or `send` degenerates to `refused-ambiguous-from` in the common
  multi-session case. Rule (in `daemon.md` + a parenthetical in ADR 0019): precedence
  **explicit `--from` > `$TELEX_ADDRESS`/`--address` > daemon
  `ResolveFrom(TELEX_SESSION_ID)`** against *that session's* registered addresses; one
  inferred -> succeed, multiple -> `ambiguous-from`, none -> existing unrepliable rules.
  **Never infer across all daemon-owned addresses (across sessions).** The harness/plugin
  MUST propagate `TELEX_SESSION_ID` to the `send`/`reply` process env.
- **Single-source SKILL / plugin-skill mechanism (DA-10; deliverable 7 / ADR 0021).**
  Canonical file: root `SKILL.md` (unchanged). CLI consumer: `include_str!` in
  `src/commands/skill.rs` (unchanged). Plugin-skill consumer: a manifest pointer if the
  harness supports it, otherwise a thin wrapper that `exec`s `telex skill --raw`.
  Invariant: **no generated divergent copy; both consumers point at the same file.**

## Key Decisions

- **D1 — Build `docs/design/` and migrate `DESIGN.md` + `DECISIONS.md` there.**
  Builder-directed (shaping). Two-layer doc model: root = loose ad-hoc vision; 
  `docs/design/` = the rigorous system spec edited by node workers. Manifest-aligned
  (the launch manifest expects `docs/design/index.md` + relative `docs/design/*.md`).
  **Deviates from issue #34's "keep the design layer at the repo root"** — builder
  approved live; flag as a deviation in the PR + field report; updating the
  brief/issue text is an **orchestrator** action (not done by this node). `SKILL.md`
  stays root (binary-embedded). Use `git mv` to preserve history.
- **D2 — "Local exchange" telex-metaphor anchor.** Daemon = the local exchange;
  "daemon" stays technical; "station" recast as a registration in the exchange.
- **D3 — Full rewrite of `DESIGN.md`** to the daemon end-state (not transitional
  supersede). Builder rationale: private repo, ships before opening, so describing
  not-yet-shipped behavior as current is acceptable. DECISIONS.md still
  append/supersede per its log convention.
- **D4 — Single-threaded authoring**, `daemon.md` as the normative spine, ADRs concise
  and pointing into it. No fleet dispatch; design coherence beats parallelism.
- **D5 — Plan review uses the council skill** (builder-directed, replaces spar):
  contained council-runner, `general-reviewer` persona + premortem/retrospective
  overlays + model diversity, artifacts under `reviews/planning/`.
- **D6 — Honor preserved dissent (ratified, do not reopen):** no held-stream
  `SessionConnect`; **no verb renames** — keep `attach`/`detach`/`wait` as the CLI
  verbs (now one-shot against the daemon); Register/Re-register/DeregisterSession are
  IPC-protocol operations, not CLI renames (Q-B resolved); record capability
  scope/rotation fields now, defer tiers.
- **D7 — Council plan-review folded in (GO-WITH-CHANGES, HIGH).** All 10 CORE findings
  (DA-1..DA-10) adopted as in-place sharpenings; minority report (DA-1 mechanism: chose
  drain-then-claim over rotation-alone on a wire-level proof) preserved. Carry the
  council `reopen_conditions` into the field report. No architectural rework; anchor
  preserved.

## Open Questions

**All four plan-level questions are resolved (council-confirmed; none blocking).**

- **Q-A — ADR granularity. RESOLVED: keep 8 ADRs (0014–0021).** ADR 0019 gets an
  explicit **"Scope" header** noting it covers two concerns (daemon-scoped
  capability/version IPC **and** daemon-native session ownership) and that splitting was
  considered and declined for log brevity. *(Council OQ-β; omitting the note is an audit
  trigger — it would force re-litigating the split.)*
- **Q-B — Verb vocabulary. RESOLVED: no contradiction.** Keep `attach`/`detach`/`wait`
  as the user-facing CLI verbs (now one-shot against the daemon); Register/Re-register/
  DeregisterSession are the **protocol/IPC** operations, not CLI renames. Council
  examined-passed against the ratified "keep verbs" / preserved dissent.
- **Q-C — `TELEX.md` touch. RESOLVED: leave `TELEX.md` untouched.** Mention the
  exchange lineage only in `DESIGN.md`/`PRODUCT-THESIS.md` (root vision docs evolve
  ad-hoc outside this node). Council document-architecture surface passed.
- **Q-D — Issue-amendment mechanism. RESOLVED: PR + field report only.** Do **not** edit
  issue #34's body (authority limit); document the D1 docs/design deviation in the PR +
  field report for orchestrator routing. Council parked it as an orchestrator action.
- **OQ-γ (new, ADJACENT) — sessionResume hook scope.** State up front in `daemon.md`:
  *if a positive-presence resume/connect hook is added later it joins the
  `last_confirmed` refresh path; `design-foundation` does not require it in v1.* Keeps
  `daemon-core` from being stranded if such a hook lands.

## Definition of done

- `docs/design/{index.md, DESIGN.md, DECISIONS.md, daemon.md}` exist, internally
  consistent; `PRODUCT-THESIS.md` updated; root links fixed; `SKILL.md` untouched.
- ADRs 0014–0021 recorded with correct supersedes/relocations/deferrals (0019 carries
  the Scope header).
- All 8 OQs resolved with implementable specifics; the three council deliverable-
  coverage resolutions (`seen`-redesign, `from`-default, single-source SKILL) landed;
  relocations/supersedes/defers map present (incl. `from`-default + single-source SKILL
  entries); deferred items explicit.
- The **five** gating tests specified as `daemon-core` acceptance: concurrent first-use;
  crash-during-`wait` (with suspect-row + `wait` Re-register); competing daemons;
  handoff duplicates (extended with ownership-loss-around-delivery); intra-daemon
  takeover local-eviction. Each with per-test observable assertions (OQ7).
- No production code changed; design layer ready for the design-gate PR to `main`.
- Council plan-review synthesis incorporated (DA-1..DA-10) or explicitly dispositioned;
  minority report (DA-1 mechanism) preserved.
