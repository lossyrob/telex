# Council transcript — plan review for local-daemon / design-foundation (telex #34)

- Mode: deliberate
- Depth: medium
- Brief: `.paw/work/design-foundation/reviews/planning/council-plan-01/brief.md`
- Plan under review: `.paw/work/design-foundation/Plan.md`
- Runner: contained council-runner (sole transcript writer; flushes each member turn as it completes)
- Nesting outcome: nested subagents

---

# Round 1 — isolated panel (members did not see each other's turns)

## Member turn — gr-premortem (round 1)

- agent_id: gr-premortem
- requested_model: gpt-5.5
- provider_family (self-reported): OpenAI / GPT family / unsure tier
- persona: general-reviewer + premortem overlay
- round: 1
- timestamp: 2026-06-23T01:50:02Z

### Examined-and-passed surfaces (brief one-liners; what you checked and why it passed)
- OQ1 daemon-aware epoch basics: new-daemon heartbeat/release self-demote is directionally sound if specified as conditional writes returning row count; concern is legacy overlap and delivery-fence precision, not the epoch concept itself.
- Q-B verb vocabulary: CLI `attach`/`detach`/`wait` mapped to IPC Register/DeregisterSession/Wait preserves the ratified no-rename constraint (Plan.md:227-233; brief.md:153-156).
- Document-location deviation mechanism: plan flags the `docs/design/` deviation for PR/field-report reconciliation rather than silently pretending issue #34 already said it (Plan.md:197-204; brief.md:71-76).

### Findings

#### Finding F1 — Live legacy holder cutover assumes old code can self-demote
- epistemic_act: CHALLENGE
- key_claim: OQ5 is not implementable as written because a legacy holder will not see a 0-row epoch heartbeat or self-demote after a daemon epoch-1 claim.
- confidence: HIGH
- grounds: Plan.md:159-166 says the first daemon-aware claimant treats NULL/absent epoch as epoch 0, claims epoch 1, and legacy holders self-demote on their first 0-row heartbeat. Current backend `heartbeat` returns `Result<()>`, not affected rows (src/backend/mod.rs:49-51); SQLite and Postgres heartbeat update only by `address` and ignore row count (src/backend/sqlite.rs:325-335; src/backend/postgres.rs:313-320); `attach` treats any Ok heartbeat as success (src/commands/attach.rs:183-185).
- warrant: An old binary cannot apply the new epoch predicate, cannot observe a 0-row epoch-guarded heartbeat, and can keep refreshing `heartbeat_at_ms` for a row whose occupant/epoch was taken by the daemon; that leaves duplicate-delivery and zombie-liveness risk during cutover.
- rebuttal_conditions: Falsified if the design mandates a compatibility shim that intercepts/rewrites legacy heartbeats, or proves existing legacy holders are forcibly stopped before any epoch claim.
- dissent_or_alignment: n/a (round 1, isolated)
- relevance: CORE
- smallest_change: Rewrite OQ5 to prohibit live-overlap epoch takeover of a reachable legacy holder: first detect non-epoch rows, stop/drain the legacy holder via existing address-keyed IPC or wait until its heartbeat is stale, then claim epoch 1. Remove the "legacy self-demotes on 0-row heartbeat" claim; add a cutover gating assertion for "old heartbeat after epoch claim cannot keep the new lease fresh or deliver."
- what_gets_smaller: Shrinks the highest-risk upgrade ambiguity from "old holders somehow fence themselves" to an explicit hard-cutover protocol daemon-core can implement.

#### Finding F2 — Session-scoped capability is not carried to the hook
- epistemic_act: CHALLENGE
- key_claim: OQ6's "capability held in the plugin/session env" is not proven obtainable because `Register` runs in a child process and the sessionEnd hook is a separate short-lived process.
- confidence: HIGH
- grounds: Plan.md:167-174 requires a per-session capability minted at Register and held in plugin/session env. The current hook plumbing runs `telex session-end` as a separate command (feature branch integrations/copilot-cli/hooks.json:4-9), and `session_end` reads only a session id from stdin/flag (src/commands/session_end.rs on origin/feature/copilot-session-end-plugin:34-43). The existing branch uses filesystem `session_registry` to bridge attach-time state to hook-time cleanup (src/session_registry.rs on that branch:17-19,103-114), which the plan explicitly drops as authority.
- warrant: A child `telex attach`/Register process cannot mutate the parent Copilot session environment for a later hook. Without a defined carrier, daemon-core must invent either a registry, a weaker proof, or plugin-specific machinery.
- rebuttal_conditions: Falsified if Copilot plugin hooks can persistently set per-session environment variables after Register, or if Register is always invoked by a plugin wrapper that owns both minting and later hook env injection.
- dissent_or_alignment: n/a (round 1, isolated)
- relevance: CORE
- smallest_change: Make the v1 OQ6 proof-carrier explicit: either (a) sessionEnd uses the daemon instance-admin capability plus hook-provided session id under same-user local trust, with per-session caps deferred/reserved; or (b) introduce a narrow user-private capability store/keyring that is not the session-address authority. Do not leave "held in session env" as the normative proof unless the carrier is documented.
- what_gets_smaller: Turns DeregisterSession from a hand-waved auth story into an implementable hook contract without reviving the filesystem registry as ownership authority.

#### Finding F3 — Crash restart must not freshen unproven attendance
- epistemic_act: REFINE
- key_claim: OQ8 can create permanent zombies if a respawned daemon heartbeats/reclaims old leases before clients re-register and re-prove live attendance.
- confidence: MEDIUM
- grounds: OQ8 says the in-memory `session_id -> addresses`, watch-pid set, and IPC waiters are lost on crash, while lease rows survive (Plan.md:182-187). It then says respawn re-reads leases, resumes heartbeat under a new epoch, and clients re-register (Plan.md:187-189). A session that ended while the daemon was down may have only a no-op hook and no future re-register (Plan.md:189-193).
- warrant: If the daemon refreshes heartbeat/new epoch from durable rows alone, TTL daemon-down recovery is defeated and stale attendance becomes "fresh" without any live session proof.
- rebuttal_conditions: Falsified if "re-validates live ones" is specified to mean an actual client Register/Re-register/wait-connect or validated persisted watch-pid, and no heartbeat/delivery occurs before that proof.
- dissent_or_alignment: n/a (round 1, isolated)
- relevance: CORE
- smallest_change: In OQ8/daemon.md, define restart rows from a prior owner as `suspect`/`daemon_down_recovered`: do not heartbeat or deliver for them until Register/Re-register/wait-connect proves live attendance. Let unproven rows lapse via daemon-down TTL or surface as stale/takeover candidates. Add this to crash-during-wait and competing-daemon gating assertions.
- what_gets_smaller: Separates durable lease recovery from durable attendance proof, avoiding a crash path that silently reintroduces permanent occupied zombies.

#### Finding F4 — `hook touch` should not refresh attendance for sessionEnd
- epistemic_act: CHALLENGE
- key_claim: OQ2's last-confirmed update on "each hook touch" conflicts with sessionEnd as healthy disconnect and can delay takeover for dead sessions.
- confidence: MEDIUM
- grounds: Plan.md:133-141 updates `attendance_last_confirmed_at` on register, wait-connect, and hook touch. The ratified liveness model says sessionEnd hook is healthy disconnect/deregister (brief.md:50-53), and the current plugin hook is specifically `sessionEnd` (integrations/copilot-cli/hooks.json on origin/feature/copilot-session-end-plugin:4-9).
- warrant: A terminal hook is negative attendance, not positive confirmation. If it refreshes `last_confirmed` before/while DeregisterSession fails, the design masks staleness and delays the load-bearing takeover path.
- rebuttal_conditions: Falsified if "hook touch" is narrowed to non-terminal resume/start hooks and sessionEnd is explicitly excluded from last-confirmed refresh.
- dissent_or_alignment: n/a (round 1, isolated)
- relevance: CORE
- smallest_change: Split hook semantics: Register/Re-register/wait-connect and any future positive resume hook may update `last_confirmed`; sessionEnd must remove session membership/release addresses, and on failure only record a recent error without refreshing attendance.
- what_gets_smaller: Prevents a small wording ambiguity from becoming a recovery-race bug in stale-attendance/takeover.

#### Finding F5 — Delivery fence needs executable ordering semantics
- epistemic_act: REFINE
- key_claim: OQ1 names `mark_delivered_if_current_owner` but does not yet specify enough ordering/result semantics to prevent daemon-core from rebuilding the current send-before-fence race.
- confidence: MEDIUM
- grounds: The plan requires "no Message frame unless the daemon still owns the epoch" (Plan.md:125-127). The verified current hazard is that `Frame::Message` is sent before `mark_delivered` commits (src/commands/attach.rs:477-488; brief.md:165-167). The brief says the server-side fence must close exactly this hazard (brief.md:43-46,109-114).
- warrant: Without an explicit backend API contract and failure matrix, an implementer could put the owner check only in post-send marking; that still emits a frame from a superseded daemon.
- rebuttal_conditions: Falsified if daemon.md will normatively define the exact critical section, including NotOwner/AlreadyDelivered results, local send failure handling, and the handoff-duplicates gating test.
- dissent_or_alignment: n/a (round 1, isolated)
- relevance: CORE
- smallest_change: Add to WI-2/OQ1: define the delivery-fence algorithm as a named contract with address + owner_instance_id + lease_epoch + message_id inputs; no IPC frame may be emitted after a NotOwner/AlreadyDelivered result; local write failure and ownership-loss races must have specified durable outcomes; the handoff-duplicates test must exercise losing ownership around delivery authorization.
- what_gets_smaller: Converts a slogan-level fence into an implementable proof obligation tied to the known double-delivery seam.

## Member turn — gr-retrospective (round 1)

- agent_id: gr-retrospective
- requested_model: gemini-3.1-pro-preview
- provider_family (self-reported): Google / Gemini 3 family / pro tier
- persona: general-reviewer + retrospective overlay
- round: 1
- timestamp: 2026-06-22T21:57:08Z

### Examined-and-passed surfaces (one-liners; what you checked and why it passed)
- Coverage surface 6 (OQ7 Status freeze line): Examined the "freeze field set, defer rendering" boundary -> Passed; it gives `daemon-core` the exact data requirements needed for the 4 gating tests without over-constraining the diagnostic output format or JSON structure.
- Coverage surface 7 (Document architecture): Examined the `docs/design/` split and `SKILL.md` deferral -> Passed; `daemon.md` as the normative spine correctly isolates the system spec, and deferring `SKILL.md` is strictly necessary to prevent breaking the `telex skill` binary embed in the current CLI before `daemon-core` ships.
- Coverage surface 7 (Q-B Verb vocabulary): Examined the CLI vs IPC verb mapping -> Passed; mapping CLI `attach` to IPC `RegisterSession` honors the ratified "no verb rename" dissent while giving the internal IPC protocol precise lifecycle semantics.

### Findings

#### Finding F1 — Ambiguity in watch-pid failure state transition (OQ3/OQ4)
- epistemic_act: REFINE
- key_claim: The plan fails to specify the exact lifecycle action the daemon takes when the `--watch-pid` backstop fails, creating a contradiction with the "never tears down" rule.
- confidence: HIGH
- grounds: OQ2 explicitly states the daemon "never tears down - idle-but-alive stays occupied". However, OQ4 defines the `watch-pid` (loader anchor) as the "sole env-sourced backstop" and a "load-bearing unhooked-dismiss recovery". As an implementer, it is undefined whether a failed `watch-pid` simply stops `last_confirmed` updates (letting the session passively age into `occupied_stale` over time) or if it actively triggers an immediate teardown/DeregisterSession.
- warrant: If a dead pid just halts updates, the backstop takes `stale_after` time to take effect, delaying recovery. If it triggers immediate teardown, this must be explicitly carved out as an exception to OQ2's absolute "never tears down" phrasing, otherwise the implementer is forced to guess which rule wins.
- rebuttal_conditions: If the `watch-pid` is intended only to flag a session as `stale` rather than evict it, the design must state that explicitly (though this weakens the backstop).
- dissent_or_alignment: n/a
- relevance: CORE
- smallest_change: In WI-2 and OQ4, explicitly state: "A watch-pid failure actively triggers an immediate local DeregisterSession and backend lease release (bypassing the `occupied_stale` wait period). OQ2's 'no teardown' rule applies strictly to network idle time, not dead local processes."
- what_gets_smaller: Eliminates state-machine ambiguity for the implementer regarding process-death vs network-idle handling.

#### Finding F2 — OQ8 crash recovery lacks a re-registration trigger mechanism
- epistemic_act: CHALLENGE
- key_claim: The OQ8 recovery path relies on clients to "re-register" after a daemon crash, but lacks a defined trigger mechanism for them to do so since the initial registration is a one-shot loader command.
- confidence: HIGH
- grounds: OQ8 states "clients re-register to repopulate session->addresses + watch-pids". However, the Copilot loader runs `attach` (which calls `Register`) as a one-shot step and exits or waits. The inner worker only loops on the `wait` CLI verb. If the daemon crashes and respawns, the inner worker's next `wait` will hit a new daemon instance that lacks the `session_id` memory map.
- warrant: Because the loader will not re-run `attach`, the inner worker's `wait` loop is the only active client component. If `telex wait` simply errors out on `UnknownSession`, the session is permanently orphaned in `occupied_stale`. The implementer needs to know *who* invokes the `Re-register` IPC.
- rebuttal_conditions: If the daemon implicitly rebuilds the session purely from a `Wait` IPC, this must be specified (but would lack the watch-pid).
- dissent_or_alignment: n/a
- relevance: CORE
- smallest_change: In OQ8 and WI-2, specify: "Because the loader's `attach` is one-shot, the `telex wait` CLI command MUST intercept `UnknownSession` errors from the daemon and automatically issue a `Re-register` IPC, repopulating the watch-pid from its inherited environment."
- what_gets_smaller: Closes a critical implementation gap in the crash-recovery loop, ensuring `daemon-core` builds the necessary retry logic into the `wait` command.

#### Finding F3 — OQ2 Takeover RPC fencing conflates backend epochs with local session eviction
- epistemic_act: REFINE
- key_claim: The operator takeover RPC ambiguously relies on "minting a new epoch" to fence out the old owner, omitting the required local eviction of the stale IPC session.
- confidence: HIGH
- grounds: OQ2 says takeover "mints a new epoch for the new owner (fencing the old one out)". However, OQ1 establishes that epochs fence *competing daemons* at the server level, not local clients connected to the same daemon.
- warrant: If the daemon simply increments the backend epoch during a takeover, the old local client's IPC connection (`wait`) might still be active in the daemon's memory, allowing it to potentially receive messages. To truly fence the old client, the daemon must actively drop the old session locally.
- rebuttal_conditions: If the takeover RPC inherently targets a *new* daemon instance (which wouldn't have the old client), but OQ18 specifies a singleton identity, meaning there is only one daemon.
- dissent_or_alignment: n/a
- relevance: CORE
- smallest_change: In OQ2, amend the takeover definition to: "Operator takeover is an explicit privileged RPC that **locally evicts the old session's IPC waiters and `session_id` mapping**, AND mints a new backend epoch to force a clean server handoff."
- what_gets_smaller: Ensures the implementer correctly drops the local memory state and IPC connections during a takeover, rather than just bumping a backend integer.

#### Finding F4 — Q-A ADR Granularity recommendation
- epistemic_act: SUPPORT
- key_claim: The 8-ADR split (0014-0021) is the correct granularity and should be affirmed.
- confidence: HIGH
- grounds: `DECISIONS.md` log conventions favor short entries. Mapping each major architectural shift (epoch fence, seen-dedup, liveness, identity, IPC, upgrade floor) to its own numbered ADR prevents a single monolithic record from becoming unreadable.
- warrant: Since the detail is centralized in `daemon.md`, the ADRs serve purely as bounded decision rationales. Merging them would obscure the distinct context behind decisions like the epoch fence vs the upgrade floor.
- rebuttal_conditions: n/a
- dissent_or_alignment: n/a
- relevance: ADJACENT
- smallest_change: Council answers Q-A with: "Proceed with the 8 ADRs as proposed; they correctly separate concerns."
- what_gets_smaller: Resolves Q-A and clears the path for the document architecture as proposed.

## Member turn — gr-baseline (round 1)

- agent_id: gr-baseline
- requested_model: claude-opus-4.7
- provider_family (self-reported): Anthropic / Claude (Opus-class assumed per request, unverified at runtime) / frontier
- persona: general-reviewer + baseline lens
- round: 1
- timestamp: 2026-06-23T02:00:00Z

### Examined-and-passed surfaces (one-liners; what you checked and why it passed)

- **Surface 1 (faithfulness to node outcome anchor):** I mapped each of the 9 deliverables (issue #34 / task spec) to a concrete WI in `Plan.md` -> all 9 land in a real work item (D1 -> WI-5 ADRs 0014/0019/0021; D2 -> WI-3; D3 -> WI-4; D4 -> WI-2 daemon.md IPC/auth sections; D5 -> WI-2 lifecycle + gating-tests bullet; D6 -> WI-2 session-ownership + WI-5 ADR 0019; D7 -> WI-5 ADR 0021; D8 -> WI-5 ADR 0020 + WI-2 upgrade-floor bullet; D9 -> WI-6 + plan OQ section). None silently downgraded to enabling work. (One coverage hole found inside D7 — see F4.)
- **Surface 8 (sequencing/completeness):** Single-threaded ordering (WI-1 prereq -> WI-2 spine -> WI-3/4/5/6 -> WI-7 audit) is internally coherent and matches "doc coherence beats parallelism" given the interlocking docs. Definition-of-done lists files-exist + ADRs + OQ resolutions + relocations-map + no-code + council-incorporated; adequate as a doc-only gate.
- **Surface 9 (faithfulness vs ratified + preserved dissent):** Cross-checked `initial-shaping.md` Spar R1 + Council A–H + preserved-dissent block against `Plan.md`. No silent contradictions: (a) verbs preserved — `attach`/`detach`/`wait` kept as CLI; Register/Re-register/DeregisterSession are IPC ops only (matches "no rename" preserved dissent + ADR 0007); (b) no held-stream `SessionConnect` anywhere in plan; (c) capability "one token v1, scope/rotation reserved" matches "record scope/rotation fields now, defer tiers"; (d) lease-epoch fence wrapped server-side around `mark_delivered_if_current_owner` per Council A; (e) `seen` redesign elevated per Council B (under-specified — see F1); (f) takeover called "load-bearing" per Council E; (g) minimal upgrade floor split per Council C; (h) daemon-native `DeregisterSession` per Council G.
- **ADR numbering / supersede targets exist:** Verified DECISIONS.md runs 0001..0013 (`grep ^## \d{4}`); 0014–0021 is a clean continuation. All supersedes targets exist: 0004 (holder/waiter split), 0009 (station — recast), 0010 (local holder registry), 0012 (pid-watch), 0013 (`seen` unpruned), 0005 (TTL narrowed to daemon-down). No phantom citations.
- **Q-C (TELEX.md untouched):** Defensible — TELEX.md is the historical-telex narrative, "local exchange" is genuinely apt there, but D1 + "root docs evolve outside this node" make it consistent to defer; mention lineage only in DESIGN.md / PRODUCT-THESIS.md. Passes.
- **Q-D (issue-amendment mechanism):** Plan correctly documents the D1 deviation in PR + field report rather than editing #34's body — matches authority limits (orchestrator action). Passes.

### Findings

#### Finding F1 — `seen`-dedup redesign deliverable is under-specified at plan time

- epistemic_act: CHALLENGE
- key_claim: ADR 0016 + WI-2 commit to redesigning `seen` "for a long-lived daemon," but the plan never names the shape (bounded vs durable, key, eviction policy, persistence boundary) the design will land on — this is a load-bearing deliverable-1 requirement *and* a ratified-prerequisite (Council B, elevating #26 from carry to design-foundation prerequisite), so leaving it as a placeholder bullet risks the design-gate accepting a missing normative spec.
- confidence: HIGH
- grounds:
  - Plan WI-2 lists "delivery + `seen`-dedup redesign" as a single sub-bullet of `daemon.md`, no shape specified.
  - ADR 0016 description: "Supersedes 0013's never-prune `seen`; elevates #26." — supersession noted, replacement not.
  - Plan's "Open-question resolutions" section spells implementable specifics for OQ1/2/3/4/5/6/7/8 but contains nothing for the `seen` redesign, because it is *not* an OQ — it slipped between the OQ pressure-test surface and the "details deferred to `daemon.md`" surface.
  - Council B (in `initial-shaping.md` §"Council review outcomes"): "Specify a bounded/durable tombstone *before* `daemon-core`." That "before" is what the design-gate is gating on.
  - Code-seam grounding (brief §"Code-seam grounding"): `seen` is `Mutex<HashSet<i64>>` — today never pruned *because holders restart*; daemon voids that. The double-delivery hazard (Frame::Message at line 477 before `mark_delivered` at line 485) interacts with `seen` design across handoff (per-process `seen` resets across handoff).
- warrant: A deliverable explicitly named as a ratified prerequisite cannot be left as "we'll write a section about it" without naming the design knob (bounded N most-recent ids? durable tombstone table keyed by (recipient, message_id) with TTL? per-(owner_instance, address) reset semantics on epoch change?). The OQ-resolution section is the right precedent for the level of plan-time specificity required, and `seen` deserves the same treatment.
- rebuttal_conditions: Plan adds a paragraph in the OQ-resolutions section (or a sibling block, e.g. "Prerequisite resolutions: `seen`-dedup redesign") committing to a specific shape — e.g., "bounded LRU of size N per (owner_instance, address), seeded from durable `deliveries` table on claim; durable `deliveries.mark_delivered_if_current_owner` is the authoritative gate, `seen` is a fast-path optimization; on epoch change the new daemon seeds `seen` from `deliveries` for that address" — and ADR 0016 echoes the chosen direction in one sentence.
- dissent_or_alignment: n/a (round 1, isolated). I expect gr-premortem to surface a "we shipped without `seen` shape" failure-mode independently.
- relevance: CORE
- smallest_change: Add a labeled `seen`-redesign resolution alongside the 8 OQ resolutions in `Plan.md` naming (a) bounded vs durable, (b) the key tuple, (c) the seeding/reset rule on epoch change, (d) whether it remains a fast-path or replaces `deliveries` as the dedup authority. Reflect the chosen shape in ADR 0016 as one sentence.
- what_gets_smaller: Closes the gap between "Council B elevated this to prerequisite" and "the plan's resolutions section pressure-tests OQs but not this," ensuring the design-gate has a testable artifact for the prerequisite.

#### Finding F2 — `from`-default resolution path is silently broken by superseding 0010 without a daemon-era replacement

- epistemic_act: CHALLENGE
- key_claim: ADR 0019 supersedes "0010's local holder registry as the `from`-default source," but the plan does not specify the daemon-era `from`-resolution rule — and 0010's "uniquely live local station for this backend" rule does *not* trivially port to a single per-user daemon serving multiple sessions, where multiple addresses are routinely "locally held" by the same daemon.
- confidence: HIGH
- grounds:
  - ADR 0010 (`DECISIONS.md:391–438`) makes `from` default = "the uniquely live local station for the current backend" — explicitly *refused* with `refused-ambiguous-from` when more than one local station exists. Plan §WI-5 ADR 0019 supersedes "0010's local holder registry as the `from`-default source."
  - With one per-user daemon and N concurrent sessions each attached to its own address(es), the daemon's `session_id -> addresses` map (deliverable 6) is multi-station by design. "Uniquely live local station" therefore degenerates to `refused-ambiguous-from` in the common case — a regression of 0010's UX win unless the resolution becomes *per-session*.
  - The plan covers daemon-native session ownership (deliverable 6, OQ6) but never connects it to `send`/`reply` `from`-resolution. No work item promises the `from`-resolution spec.
  - Per-session resolution requires the calling `send`/`reply` process to know its session id — which only the harness can supply (via env, e.g. `TELEX_SESSION_ID`). The plumbing exists conceptually (plan WI-2 names env-passed capability) but the design connection is absent.
- warrant: Superseding an existing accepted ADR without specifying the replacement leaves daemon-core to re-decide a user-visible behavior (when does `send` auto-resolve `from`?). That violates the node anchor ("design layer the rest of the workstream executes against … without re-deciding architecture") and risks a silent UX regression at the first multi-session daemon run.
- rebuttal_conditions: Plan adds either (a) an explicit `from`-resolution section in `daemon.md` ("send/reply with no `from` consults the daemon: if `TELEX_SESSION_ID` is set, default to the unique address held by that session for the current backend; else preserve 0010's same-backend-unique-station rule across the daemon's full session->addresses set"), or (b) an explicit defer-with-rationale entry in the relocations/supersedes/defers map ("daemon-era `from`-default behavior tracked under <Issue/Node>; ADR 0019 supersedes the *mechanism* of 0010 — local registry — but leaves the *policy* of 0010 in force pending that work").
- dissent_or_alignment: n/a (round 1, isolated). I expect gr-retrospective to land here independently (a daemon-core implementer hits this first time they wire `send`).
- relevance: CORE
- smallest_change: One paragraph in `daemon.md` (and a parenthetical in ADR 0019) spelling out the daemon-era `from`-resolution rule + which env var the harness/plugin must propagate, OR an explicit defer entry with named follow-up.
- what_gets_smaller: Eliminates a silent ratified-ADR-supersession with no replacement — the failure mode most likely to surface only when downstream code is written.

#### Finding F3 — OQ6 DeregisterSession proof: env-propagation mechanism is hand-waved; same-trust fallback is the de facto v1 path and should be the headline

- epistemic_act: REFINE
- key_claim: OQ6's resolution ("scoped capability bound to daemon-instance + session, minted at `Register` and held in the plugin/session environment") is sound in spirit but glosses over the mechanism by which a separately-spawned sessionEnd hook process inherits an env var minted by an unrelated short-lived `attach` invocation — and on Copilot CLI today, the *real* v1 path is the "fallback instance-admin capability for the user-private same-trust case," which the plan treats as a parenthetical rather than the headline.
- confidence: MEDIUM
- grounds:
  - `attach` is now a one-shot process (plan §"Topology"). When `attach` returns, the daemon hands back a per-session capability; for the hook (a separate process spawned by the harness at sessionEnd) to present it, it must end up in the hook process's env. `attach` cannot inject env vars upward into the harness; only the harness can put them in the hook's env.
  - On Copilot CLI specifically, the plugin owns the integration boundary (`integrations/copilot-cli/{plugin.json,hooks.json}` per brief §"Code-seam grounding"). Whether the plugin can stash an arbitrary per-session string and surface it as env to its own hook is a harness-implementation question the design assumes "yes" without spelling out the contract.
  - Plan correctly notes the "instance-admin capability for the user-private same-trust case" — for a per-user daemon on a single-user dev box, this is the entire threat model that exists v1. The forward-compatible per-session cap is design hygiene, not a v1 requirement.
  - OQ6's framing makes the per-session cap sound load-bearing for v1; it isn't — and that obscures the real v1 contract.
- warrant: A design contract that requires "the plugin stashes a per-session secret in the session env" without specifying the stash mechanism leaves daemon-core *and* the copilot-plugin node both blocked on a harness-API question. Reframing makes the v1 path (same-trust fallback) the clear contract and the per-session cap a reserved-field forward path — matching the preserved-dissent posture ("record scope/rotation fields now, defer tiers").
- rebuttal_conditions: There exists a Copilot CLI plugin API surface (current or imminent) that lets the plugin pre-populate the sessionEnd hook's env from a value it captured during `attach`; in that case the per-session cap can be the v1 path and the rewording is unnecessary.
- dissent_or_alignment: n/a (round 1, isolated).
- relevance: CORE
- smallest_change: In OQ6 resolution, lead with: "v1 path on Copilot CLI = instance-admin capability over the per-user named pipe (same-trust). Per-session scoped capability is the protocol-reserved forward field for non-same-trust harnesses; its env-propagation mechanism is harness-specific and out of scope here." Keep the rest as-is.
- what_gets_smaller: Removes ambiguity about which capability path daemon-core implements first, and aligns OQ6 with the preserved dissent on capability tiering.

#### Finding F4 — Deliverable 7 sub-requirement "single-source SKILL.md / plugin-skill mechanism" is missing from the plan

- epistemic_act: CHALLENGE
- key_claim: Issue #34 deliverable 7 explicitly lists "specify the single-source `SKILL.md` / plugin-skill mechanism (one source serves both the CLI command and the plugin skill)" as a design output. Plan WI-5 ADR 0021 names "Verb + docs/SKILL cutover; design-layer relocation to `docs/design/`" but does not include the single-source mechanism, and no other WI does either — the SKILL holder/waiter narrative cutover itself is correctly deferred to daemon-core, but the *mechanism spec* is a `design-foundation` output, not a `daemon-core` one.
- confidence: HIGH
- grounds:
  - Task spec `design-foundation.md:99–104`: "specify the single-source `SKILL.md` / plugin-skill mechanism (one source serves both the CLI command and the plugin skill)."
  - Plan §"Document architecture": "`SKILL.md` is binary-embedded (`telex skill` -> `include_str!` in `src/commands/skill.rs`), so it must not move (moving needs code = out of scope) and its holder/waiter narrative cutover is explicitly deferred to `daemon-core` (deliverable 7)."
  - Plan §WI-5 ADR 0021 mentions cutover but not the single-source mechanism. Plan §"Definition of done" requires "`SKILL.md` untouched" — consistent with deferring the narrative but silent on the mechanism.
  - The mechanism is a real open design point: is the source the embedded `SKILL.md` (one file, two consumers via include + plugin manifest pointer)? Is it a generated file with a build step? Is the plugin-skill a thin wrapper that `exec`s `telex skill`? Without specifying, daemon-core picks an architecture.
- warrant: Deliverable 7 is a named issue-#34 output. Silently dropping a sub-requirement is exactly the "deliverable downgraded to enabling work" failure mode the brief's surface 1 (faithfulness) names.
- rebuttal_conditions: A clear ratified decision already lives in `initial-shaping.md` that the plan inherits without restating ("ratified Plugin scope: ... telex skill as a real plugin skill, one source serving both command and plugin skill"). I see that line at `initial-shaping.md:151–152`. So the *decision* is ratified; what's missing is the *mechanism specification* (which file, which build/load path, how the plugin manifest references it). If the plan owner believes the ratified phrase alone is the design output, this finding downgrades to "make sure the design-foundation PR restates and cites the ratified line as a normative reference in `daemon.md` or DESIGN.md".
- dissent_or_alignment: n/a (round 1, isolated).
- relevance: CORE
- smallest_change: In WI-2's `daemon.md` section list (or WI-5's ADR 0021), add: "Single-source skill mechanism: name the file (current `SKILL.md`), the CLI consumer (`include_str!`), and the plugin-skill consumer (manifest pointer / wrapper / `exec`), and assert no divergent copies." One paragraph + a sentence in ADR 0021.
- what_gets_smaller: Closes a faithfulness gap that the OQ-focused review surfaces would miss.

#### Finding F5 — OQ5 fence-out-legacy claim is sound only if the daemon's claim rotates the legacy-recognizable identity field; plan should make this explicit

- epistemic_act: REFINE
- key_claim: OQ5 asserts "legacy holders self-demote on their first 0-row heartbeat (they don't understand epochs, so their non-epoch-guarded heartbeat is superseded by the daemon's epoch-guarded writes)" — this is true only if the daemon's claim also rotates a field the legacy heartbeat keys on (notably `occupant`), because today's legacy heartbeat `WHERE address=? AND occupant=?` will otherwise still match the row after the daemon's claim and continue to extend TTL without 0-row-ing.
- confidence: MEDIUM
- grounds:
  - Code seam (brief §"Code-seam grounding"): backend `release_lease` is `WHERE address=? AND (occupant=? OR occupant IS NULL)`; existing lease is "keyed by address only" with no epoch column; the legacy `heartbeat`/`release` flow lacks an epoch predicate.
  - OQ1 specifies daemon writes use `WHERE address=? AND epoch=? AND owner=?`. The plan does not say the *claim* operation rotates `occupant` (or any legacy-keyed field) — it says it claims "epoch = 1 under the row condition."
  - If the daemon claim sets `(epoch=1, owner_instance=…)` but leaves `occupant=<legacy>`, the legacy heartbeat `UPDATE … SET ttl=… WHERE address=? AND occupant=<legacy>` returns >0 rows and the legacy holder never sees the 0-row signal.
- warrant: The cutover guarantee in OQ5 ("hard cutover acceptable; legacy self-demote on first 0-row heartbeat") rests on a concrete write-collision behavior. Naming exactly which field rotation produces the 0-row makes it implementable; leaving it implicit invites a daemon-core implementation that has the lease row but never quiesces the legacy holder.
- rebuttal_conditions: The plan's `daemon.md` already plans to spell out the claim SQL precisely (rotating `occupant` to the daemon's owner_instance_id), in which case this is a remind-rather-than-fix. The plan's OQ5 paragraph just needs a half-sentence to lock the contract at plan level.
- dissent_or_alignment: n/a (round 1, isolated).
- relevance: CORE (small-surface; high-leverage)
- smallest_change: In OQ5 resolution, add: "Daemon claim rotates `occupant` to the daemon's `owner_instance_id` (so a legacy heartbeat keyed on the old `occupant` 0-rows on its next write); release path drops the occupant-null branch (per OQ1)."
- what_gets_smaller: Makes the cutover claim concretely testable and removes a plausible flip-flop pathway between legacy holder and daemon.

#### Finding F6 — ADR 0019 bundles two separable concerns (Q-A); split improves audit trail without raising count meaningfully

- epistemic_act: PROPOSE
- key_claim: On Q-A (ADR granularity), the proposed 0014–0021 set is largely well-sized, but **0019 ("Daemon-scoped capability/version-handshake IPC + daemon-native session ownership")** bundles two architecturally distinct decisions — *protocol scoping/auth* (Council F) and *session-ownership authority* (Council G) — that have independent supersedes targets (0010's local holder registry is the session-ownership angle; the address-keyed unauthenticated IPC in `src/ipc.rs` is the protocol angle). Splitting into 0019a/0019b is cheaper than the future cost of citing one ADR for two unrelated decisions.
- confidence: MEDIUM
- grounds:
  - DECISIONS.md convention (line 7–19): record load-bearing decisions that would be "costly or confusing to relitigate." Bundling makes future "what did we decide about IPC auth?" lookups noisier.
  - Council F (IPC scoping/auth) and Council G (DeregisterSession authority) are listed as separate review outcomes in `initial-shaping.md:245–253`.
  - ADR 0017 likewise bundles three OQs (OQ2/OQ3/OQ4) into one ADR; that bundling is more defensible because all three are "liveness model" and have the same supersedes target (0012). 0019 doesn't have that single-thread.
- warrant: ADR splits/merges are cheap at plan time; auditability is the point of the log.
- rebuttal_conditions: The DECISIONS.md style guide's "keep entries short" + the plan's "detail in `daemon.md`" posture argue for fewer, terser ADRs. If the author prefers tight, 8 → 9 is a wash and this can be marked "considered, declined for log brevity" without harm.
- dissent_or_alignment: n/a (round 1, isolated).
- relevance: ADJACENT
- smallest_change: Either split ADR 0019 into 0019 (scoped/versioned/auth IPC; Council F; reshapes `src/ipc.rs`) and 0020 (daemon-native session ownership; Council G; supersedes 0010 mechanism, reshapes #23/#31) — renumbering 0020/0021 → 0021/0022 — or document "considered split, kept merged for log brevity" in Plan Q-A.
- what_gets_smaller: Makes the supersession trail cleaner without altering the design substance.

#### Finding F7 — The "4 gating tests' observable assertions" (Deliverable 5 + OQ7) need a plan-time commitment to per-test assertion sketches, not just a name list

- epistemic_act: REFINE
- key_claim: Deliverable 5 and OQ7 both bind the design-gate to "the 4 gating tests' observable assertions" (concurrent first-use, crash-during-`wait`, competing daemons, handoff duplicates), but the plan only names the tests; without at least a one-line observable per test, OQ7's "Status field set + 4 gating tests' observable assertions" cannot be the testable boundary it claims to be.
- confidence: MEDIUM
- grounds:
  - Plan §WI-2 bullet: "the 4 gating tests" — name-only.
  - Plan §OQ7 resolution: "Frozen = the field set + the 4 gating tests' observable assertions; not frozen = wire format, verbosity, extra diagnostics." — the *frozen* clause is supposed to be the design-gate contract.
  - Council D (in `initial-shaping.md:235–238`) called these tests as "`daemon-core` acceptance" — i.e., daemon-core's pass/fail criterion. The acceptance criterion has to be specified by design-foundation.
- warrant: An acceptance test list with no per-test observable is a name, not a spec. If "handoff duplicates" passes when there are zero duplicates *measured how*? (via `mark_delivered_if_current_owner` returning 0-rows on the demoted daemon? via inbox uniqueness? via Status surface counters?) Naming the observable is the design-gate's job.
- rebuttal_conditions: The plan's WI-2 includes "the 4 gating tests" specifically intending one observable-assertion subsection per test inside `daemon.md`. If so, the plan should say so at WI-2 to lock the level of specificity.
- dissent_or_alignment: n/a (round 1, isolated).
- relevance: ADJACENT (close to CORE — if missed, daemon-core acceptance is re-decided)
- smallest_change: In WI-2's section list, add: "Per-test observable assertion (one paragraph per test, naming the Status fields / backend rows / IPC frames consulted)." In OQ7, name those observables in one sentence (e.g., "handoff duplicates: no two `Frame::Message` instances with the same `(recipient, message_id)` across both daemons; verified by `deliveries.mark_delivered_if_current_owner` 0-row on demoted daemon").
- what_gets_smaller: Makes OQ7's "frozen = field set + observables" clause actually frozen and testable.




---

# Round 2 - focused interaction (members saw the other two round-1 turns)

## Member turn — gr-premortem (round 2)

- agent_id: gr-premortem
- requested_model: gpt-5.5
- provider_family (self-reported): OpenAI
- persona: general-reviewer + premortem overlay
- round: 2
- timestamp: 2026-06-23T02:07:26Z

### Reactions to live cruxes

#### Crux 1 — OQ5 cutover mechanism
- epistemic_act: SYNTHESIZE
- key_claim: Hard cutover is required; rotating `occupant` is useful only as a defensive claim detail, not as the cutover mechanism.
- confidence: HIGH
- grounds: Today's `heartbeat` updates by `address` only and returns `Result<()>`, so a legacy holder cannot observe self-demotion and rotating `occupant` does not fence its heartbeats (src/backend/sqlite.rs:325-333, src/backend/postgres.rs:313-320). `release_lease` is occupant-checked but still has the occupant-null branch to remove (src/backend/sqlite.rs:338-346, src/backend/postgres.rs:323-331). Legacy delivery can still emit before durable mark (src/commands/attach.rs:464-490), so live overlap preserves the hazard.
- dissent_or_alignment: Disagree with gr-baseline F5 as sufficient; align that daemon claims should rotate `occupant` to `owner_instance_id` as defense after no live legacy holder remains.
- relevance: CORE
- smallest_change: Replace OQ5 with: non-epoch live rows block daemon claim; operator must stop/drain legacy holders or wait for a proven lapse. Only then may daemon claim epoch 1, rotate `occupant=owner_instance_id`, remove occupant-null release, and use owner/epoch-guarded heartbeat/release returning rowcount for daemon-aware holders. Do not rely on legacy self-demotion.
- what_gets_smaller: Eliminates the unsupported assumption that old binaries can be fenced by fields they neither check nor report.

#### Crux 2 — OQ6 v1-path
- epistemic_act: SYNTHESIZE
- key_claim: gr-baseline's same-trust instance-admin cap is adequate for v1 if named as the normative user-private path and per-session caps are explicitly reserved.
- confidence: HIGH
- grounds: Ratified auth requires daemon-scoped capability IPC with one token v1 and scope/rotation reserved; it does not require v1 per-session token enforcement. In the user-private same-trust case, a per-user daemon endpoint plus instance capability is enough to authorize `DeregisterSession(session_id, proof)` because all callers are inside the same OS-user trust boundary.
- dissent_or_alignment: Align with gr-baseline F3; refine the wording to avoid pretending Copilot CLI can currently carry a per-session secret across hook processes.
- relevance: CORE
- smallest_change: OQ6 should lead with `instance_admin_cap` over a user-private daemon IPC endpoint as v1. The hook sends `session_id + instance_admin_cap`; daemon validates the cap, checks the session exists, and deregisters idempotently. `session_cap` remains a protocol field reserved for harnesses with a real env carrier.
- what_gets_smaller: Removes the impossible env-carry dependency while preserving scoped-capability shape and forward compatibility.

#### Crux 3 — OQ8 composition
- epistemic_act: SYNTHESIZE
- key_claim: The suspect-row rule plus wait-triggered re-register forms the right crash recovery loop.
- confidence: HIGH
- grounds: OQ8 already distinguishes durable lease/delivery rows from lost in-memory `session_id -> addresses`, watch-pids, and waiters. On daemon respawn, re-read lease rows must not be treated as live attendance until a client proves liveness; gr-retrospective's `wait` re-register on `UnknownSession` supplies that proof path.
- dissent_or_alignment: Align with gr-retrospective F2 and my prior suspect-row concern.
- relevance: CORE
- smallest_change: Specify an attendance state machine: `suspect` on respawn for recovered rows -> `verified` on `Register/Re-register` or authenticated wait-connect, refreshing `last_confirmed` and rebuilding watch-pids -> `lapsed` after daemon-down TTL/stale backstop with no proof. `wait` must handle `UnknownSession` by issuing `Re-register` from inherited env before failing.
- what_gets_smaller: Converts crash recovery from vague "clients re-register" into an executable transition model.

#### Crux 4a — `seen`-dedup (react to gr-baseline F1)
- epistemic_act: SUPPORT
- key_claim: This is a CORE gap; "use deliveries table" is likely the intended direction but not specified enough to execute safely.
- confidence: HIGH
- grounds: Current `seen` is an unbounded in-memory `HashSet<i64>` that protects concurrent drains before durable delivery exists (src/commands/attach.rs:32-41, src/commands/attach.rs:67-83). The durable `deliveries` table is unique on `(message_id, recipient)` and is already cross-restart authority (src/backend/sqlite.rs:66-72, src/backend/mod.rs:55-73), but the plan does not state how daemon-era in-flight dedup, epoch reset, or eviction work.
- dissent_or_alignment: Align with gr-baseline F1.
- relevance: CORE
- smallest_change: ADR 0016 / `daemon.md` must state: durable `deliveries(message_id, recipient)` is cross-epoch authority; in-memory dedup is only a bounded fast-path keyed by `(recipient, message_id, lease_epoch)` or equivalent in-flight identity; reset/drop on epoch loss; seed from `fetch_undelivered`; evict on durable mark, terminal disposition, or epoch transition.
- what_gets_smaller: Prevents reintroducing either the unbounded-daemon memory leak or the pre-mark double-enqueue race.

#### Crux 4b — `from`-default (react to gr-baseline F2)
- epistemic_act: SUPPORT
- key_claim: This is CORE because ADR 0019 explicitly supersedes the local-holder registry as `from` source but gives no daemon-era replacement.
- confidence: HIGH
- grounds: Current `from` precedence falls back to the uniquely live local station (src/identity.rs:1-17, src/identity.rs:72-116) discovered through the holder registry (src/registry.rs:1-14, src/registry.rs:131-155). A single daemon serving multiple sessions would make "all live local addresses" ambiguous in common use unless resolution is scoped by `TELEX_SESSION_ID`.
- dissent_or_alignment: Align with gr-baseline F2 as CORE, not successor-node adjacent.
- relevance: CORE
- smallest_change: Add daemon-era rule: explicit `--from` > `$TELEX_ADDRESS`/`--address` > daemon `ResolveFrom(TELEX_SESSION_ID)` using that session's registered addresses; exactly one address infers, multiple refuses ambiguous, none follows existing unrepliable/refusal rules. Never infer from all daemon-owned addresses.
- what_gets_smaller: Avoids a first-run UX regression where every multi-session daemon makes default replies ambiguous or wrong.

#### Crux 4c — single-source SKILL (react to gr-baseline F4)
- epistemic_act: SUPPORT
- key_claim: CORE for design-gate completeness, though not a daemon-core protocol blocker.
- confidence: MEDIUM
- grounds: Deliverable 7 requires the single-source SKILL/plugin mechanism. Current CLI embeds root `SKILL.md` with `include_str!` (src/commands/skill.rs:1-10), and the plan preserves root `SKILL.md` because of that embed (Plan.md:42-48), but it does not specify how the plugin skill consumes the same source.
- dissent_or_alignment: Align with gr-baseline F4.
- relevance: CORE
- smallest_change: ADR 0021 must name root `SKILL.md` as canonical; CLI consumes it via `include_str!`; plugin skill consumes the same file via manifest pointer if supported, otherwise a thin wrapper/exec of `telex skill --raw`; no generated divergent copy.
- what_gets_smaller: Closes a deliverable-level ambiguity without changing the daemon architecture.

#### Crux 5 — gr-retrospective F1/F3 reaction
- epistemic_act: CONCEDE
- key_claim: Agree with both: watch-pid death is a local death proof requiring immediate deregistration, and takeover must evict local daemon state, not just bump backend epoch.
- confidence: HIGH
- grounds: OQ2's "never tears down" is about network idle/stale attendance, not a proven-dead local process. Conversely, backend epoch fencing alone leaves stale IPC waiters and `session_id -> addresses` mappings alive inside the daemon, which can misroute waits or deliveries after takeover.
- dissent_or_alignment: Align with gr-retrospective F1/F3; refine that hook/sessionEnd healthy semantics and watch-pid failure semantics must remain distinct.
- relevance: CORE
- smallest_change: Add lifecycle rules: watch-pid failure triggers immediate local `DeregisterSession`, waiter close, address unbind/lease release, bypassing `occupied_stale`; operator takeover atomically evicts old local session mappings/caps/waiters and mints the new backend epoch before serving under the new owner.
- what_gets_smaller: Prevents "zombie local truth" from surviving either a proven process death or an operator takeover.

## Member turn — gr-retrospective (round 2)

- agent_id: gr-retrospective
- requested_model: gemini-3.1-pro-preview
- provider_family (self-reported): Google (Gemini)
- persona: general-reviewer + retrospective overlay
- round: 2
- timestamp: 2026-06-22T22:11:33Z

### Reactions to live cruxes

#### Crux A — OQ5 hard-cutover vs occupant rotation
- epistemic_act: SYNTHESIZE
- key_claim: Occupant rotation *is* the hard cutover mechanism; it structurally forces the legacy client to self-terminate without requiring novel IPC control planes.
- confidence: HIGH
- grounds: Legacy `telex` updates attendance via `WHERE address=? AND occupant=?`. If the daemon claims epoch 1 by writing its `owner_instance_id` into the `occupant` field, the legacy heartbeat will naturally 0-row on its next tick, triggering legacy's existing "lost ownership" shutdown logic.
- dissent_or_alignment: Agree with gr-baseline on the mechanism (rotation), agree with gr-premortem on the outcome (no live overlap).
- relevance: CORE
- smallest_change: Specify that daemon epoch 1 claim overwrites `occupant` with `owner_instance_id`, causing legacy heartbeats to 0-row and self-terminate.
- what_gets_smaller: Eliminates the need to design or build a fragile external "stop legacy" IPC mechanism.

#### Crux B1 — `seen`-dedup shape (gr-baseline F1)
- epistemic_act: SUPPORT
- key_claim: "Redesign seen-dedup" is a goal, not an implementable design; without a defined shape, I cannot write the code for it.
- confidence: HIGH
- grounds: The implementer must know if this is an in-memory ring buffer per daemon, a shared SQLite table, and what the eviction TTL is.
- dissent_or_alignment: Agree with gr-baseline.
- relevance: CORE
- smallest_change: Append a 1-sentence shape to WI-2 (e.g., "in-memory LRU cache per session, bounded to N messages, cleared on epoch takeover").
- what_gets_smaller: Unblocks immediate implementation of the deduplication layer.

#### Crux B2 — `from`-default supersession (gr-baseline F2)
- epistemic_act: SUPPORT
- key_claim: A multi-session daemon completely breaks the assumption of a "uniquely live local station", requiring explicit `TELEX_SESSION_ID` resolution.
- confidence: HIGH
- grounds: If `telex send` is invoked on a machine where the daemon is managing 3 sessions, the CLI has no way to infer which `from` address to use without explicit environment injection or flags.
- dissent_or_alignment: Agree with gr-baseline.
- relevance: CORE
- smallest_change: Specify that `from` resolution mandates the presence of `TELEX_SESSION_ID` (inherited from the orchestrator) or explicit deferral.
- what_gets_smaller: Prevents a critical routing ambiguity in the CLI `send` command.

#### Crux B3 — single-source SKILL (gr-baseline F4)
- epistemic_act: SUPPORT
- key_claim: Missing the SKILL.md mechanism means Deliverable 7 is objectively unmet in the plan.
- confidence: HIGH
- grounds: As an implementer, I would have either skipped this (claiming it's out of daemon-core scope) or hacked a duplicate file. The specification must dictate the canonical path and how the plugin reads it vs the CLI.
- dissent_or_alignment: Agree with gr-baseline.
- relevance: CORE
- smallest_change: Name the canonical `SKILL.md` file path and specify the read mechanism (e.g., `include_str!` or manifest pointer) in ADR 0021.
- what_gets_smaller: Closes a direct deliverable gap.

#### Crux C — OQ8 recovery state machine + blocked-wait hole
- epistemic_act: REFINE
- key_claim: A blocked `wait` client serves as the recovery validation hook because the IPC pipe breaking upon daemon death forces the client to reconnect.
- confidence: HIGH
- grounds: When the daemon dies, the named pipe or HTTP long-poll breaks. The `telex wait` process wakes up, catches the error, and reconnects to the new daemon instance, passing its `TELEX_SESSION_ID`. This reconnection event is what transitions the recovered session from `suspect` to `verified`. No periodic ping is needed.
- dissent_or_alignment: Agree with gr-premortem's state machine, refining how it composes with my F2 finding.
- relevance: CORE
- smallest_change: Specify that an incoming IPC reconnection from `telex wait` (with session ID) promotes a `suspect` session to `verified`.
- what_gets_smaller: Resolves the edge case of totally idle but still-alive sessions without adding periodic polling overhead.

#### Crux D — Q-A ADR granularity final position
- epistemic_act: CONCEDE
- key_claim: Bundling IPC auth and backend lease ownership into one ADR creates confusion for the implementer who has to build two entirely different subsystems.
- confidence: MEDIUM
- grounds: IPC scoping (CLI -> Daemon) and Session Ownership (Daemon -> Backend) touch different codebases and have different failure modes.
- dissent_or_alignment: Concede to gr-baseline (challenge my previous support for 8).
- relevance: ADJACENT
- smallest_change: Split ADR 0019 into two ADRs (making 9 total).
- what_gets_smaller: Reduces cognitive load and clarifies supersession targets during implementation.

#### Crux E — OQ7 gating-tests observables
- epistemic_act: CONCEDE
- key_claim: Naming tests without defining their failure/success observables leaves the gate functionally open to weak implementations.
- confidence: HIGH
- grounds: If I write a test called `test_epoch_takeover` but don't know I specifically need to assert that the old client receives `NotOwner` on its next delivery attempt, I might just assert the epoch incremented and call it done.
- dissent_or_alignment: Concede to gr-baseline.
- relevance: CORE
- smallest_change: Require a 1-2 line list of specific state observables for each of the 4 gating tests in WI-2.
- what_gets_smaller: Guarantees the gating tests actually enforce the architectural invariants.

#### Crux F — sessionEnd hook last_confirmed (gr-premortem F4)
- epistemic_act: SUPPORT
- key_claim: Refreshing `last_confirmed` on a `sessionEnd` hook creates a zombie session that the backend thinks is actively healthy.
- confidence: HIGH
- grounds: A hook firing naturally feels like "activity" to an implementer. I would have likely updated `last_confirmed` out of habit, breaking the semantic meaning of "confirmed dead".
- dissent_or_alignment: Agree with gr-premortem.
- relevance: CORE
- smallest_change: Explicitly document in the hook contract that `sessionEnd` must NOT update `last_confirmed`.
- what_gets_smaller: Prevents a subtle but fatal lifecycle bug.

#### Crux G — delivery fence executable semantics (gr-premortem F5)
- epistemic_act: SUPPORT
- key_claim: Without specifying the atomic tuple for the backend API, implementers will default to separate SELECT (check lease) and UPDATE (deliver) queries, creating a race condition.
- confidence: HIGH
- grounds: The fence must be enforced atomically at the database layer. The API contract must explicitly require `(address, owner_instance_id, lease_epoch, message_id)` to be evaluated together.
- dissent_or_alignment: Agree with gr-premortem.
- relevance: CORE
- smallest_change: Add the required atomic tuple constraint for delivery to the OQ1 resolution.
- what_gets_smaller: Eliminates a race condition in message delivery during epoch transitions.

## Member turn — gr-baseline (round 2)

- agent_id: gr-baseline
- requested_model: claude-opus-4.7
- provider_family (self-reported): Anthropic (Claude)
- persona: general-reviewer + baseline lens
- round: 2
- timestamp: 2026-06-23T02:13:00Z

### Reactions to live cruxes

#### Crux i — OQ5 mechanism convergence
- epistemic_act: SYNTHESIZE (concede premortem F1 is necessary; my F5 is necessary; complementary, not exclusive)
- key_claim: Daemon-side server-fence alone is INSUFFICIENT against a live legacy holder, because today's legacy holder IS the IPC waiter-server. The hazard is not just a duplicate durable mark; it's that the legacy holder's `Frame::Message` (attach.rs ~477) ships to whatever waiter is bound to its IPC endpoint BEFORE its post-emit `mark_delivered` (~485) trips against the (rotated) row. A daemon that has rebound the same address's waiter endpoint will independently emit. Two endpoints exist transiently => double-delivery on the wire, regardless of `mark_delivered_if_current_owner` rejecting the legacy holder's DB write afterward.
- confidence: 0.85
- grounds: attach.rs holder loop emits frame then marks; daemon-era waiter-bind is a distinct IPC endpoint, not a shared queue; recipient is whichever bind they connected to. Server-side row fence is post-emit.
- dissent_or_alignment: ALIGN with gr-premortem F1 on hard cutover; my F5 (rotate occupant + drop occupant-null release) is still independently required to prevent stale legacy from re-grabbing during/after drain.
- relevance: CORE
- smallest_change: OQ5 specifies a two-phase rule: (1) **drain phase** — daemon-aware claimant uses address-keyed IPC probe OR a bounded stale-wait to confirm no legacy holder is actively bound before binding its own waiter; if a holder is bound, send a quit/handover signal (or wait for natural expiry); (2) **claim phase** — only after drain, claim `epoch=1` with occupant rotation and remove the occupant-null release path. Document explicitly: "no live overlap of two waiter-binds per address."
- what_gets_smaller: closes the in-flight double-delivery window that the epoch fence alone cannot cover, while preserving my fence against legacy claim/release races.

#### Crux ii — OQ6 final v1-path framing
- epistemic_act: SYNTHESIZE (converge with gr-premortem F2)
- key_claim: v1 contract is `DeregisterSession(session_id, instance_admin_cap)` + hook-provided `session_id` from inherited env — period. No additional cryptographic per-session proof. The daemon's in-memory `session_id -> addresses` map IS the authorization scope: admin cap proves "you may speak to this daemon instance," and `session_id` selects which entry to drop. The per-session capability field is declared in the protocol struct (Optional/Reserved) but unused in v1.
- confidence: 0.9
- grounds: threat model is user-private same-trust (same OS user, same machine, same daemon process). Any process with admin cap already has filesystem access to do worse. Adding per-session crypto in v1 buys nothing against the actual adversary.
- dissent_or_alignment: ALIGN with gr-premortem F2; refine her wording with the "reserved field" clause.
- relevance: CORE
- smallest_change: OQ6 final wording: "Hook calls `DeregisterSession(session_id, admin_cap)`; daemon verifies admin_cap matches instance secret and `session_id` exists in its map; if so, drops associated addresses. Protocol reserves a `per_session_cap: Option<Cap>` field for future blast-radius narrowing (defense-in-depth against a compromised hook in one session deregistering another); v1 ignores it. No filesystem session registry. No per-session cap minted at Register in v1."
- what_gets_smaller: removes the cap-minting-and-propagation plumbing from v1 scope while preserving the protocol upgrade path. The failure mode forward-defended is lateral hook compromise: a misbehaving hook in session A could deregister session B's addresses if it learns the admin cap; per-session caps would limit it to A. Acceptable to defer because same-trust + binary attack surface.

#### Crux iii — OQ8 composition + idle-wait hole
- epistemic_act: SYNTHESIZE + REFINE (compose premortem F3 and retrospective F2; surface one residual)
- key_claim: The composition (respawn rebuilds rows as `suspect`; first `wait`-driven UnknownSession triggers auto-Re-register that promotes to `verified`) IS end-to-end correct for the blocked-wait case. Broken-pipe wakes the blocked read on both Windows named pipes (ERROR_BROKEN_PIPE/ERROR_PIPE_NOT_CONNECTED) and Unix domain sockets (EPIPE/ECONNRESET); `wait` reconnects via auto-spawn, sends its next protocol frame (heartbeat or recv-poll), receives UnknownSession, Re-registers from inherited env. Hole I want to flag: idempotency + race.
- confidence: 0.8
- grounds: standard socket EOF semantics; the design relies on auto-spawn already; env is inherited at session-process tree start.
- dissent_or_alignment: ALIGN with composition; add one residual.
- relevance: CORE (residual is a small specification gap, not a design break)
- smallest_change: OQ8 adds two clauses: (a) Re-register MUST be idempotent — multiple concurrent waits for the same session_id racing to Re-register converge to a single map entry (last-writer-wins on address set, or union; daemon.md picks one); (b) `wait` MUST treat broken-pipe + UnknownSession-on-reconnect as a normal control flow path, not an error — re-register from `TELEX_SESSION_ID` + admin cap in env, then resume waiting on the same address. Add this as a fifth gating-test invariant or fold into the daemon-crash test.
- what_gets_smaller: closes the blocked-wait recovery loop end-to-end without re-architecting; eliminates the "what if multiple waits all reconnect at once" ambiguity.

#### Crux iv — Watch-pid failure semantics
- epistemic_act: SUPPORT (with sharpening)
- key_claim: gr-retrospective F1 is correct and the empirical OQ4=no resolution makes it MORE load-bearing than implied. "Process proved dead" is categorically different from "process idle" — the "no idle-TTL teardown" stance is about the absence of time-based dismissal of live sessions, not about ignoring positive death evidence. With OQ4=no, the dismissal-path matrix is exactly: (1) sessionEnd hook = clean exit, (2) watch-pid failure = crash/kill, (3) takeover = operator override, (4) daemon-down TTL = backstop only. Path (2) is the ONLY automatic recovery for unhooked crashes. If it routes through `occupied_stale` instead of immediate DeregisterSession, the address is non-recoverable until manual takeover, defeating the "no idle teardown, but DO clean up dead" intent.
- confidence: 0.9
- grounds: OQ3/OQ4 in plan; ratified "minimal stale-attendance" exists for unobservable-death cases, not for observable-death cases.
- dissent_or_alignment: ALIGN with retrospective F1 fully.
- relevance: CORE
- smallest_change: OQ3 wording adds: "When the watch-pid predicate transitions to dead (anchor pid no longer alive OR start-time mismatch), the daemon's local watcher MUST issue an internal `DeregisterSession` for the associated session_id, bypassing `occupied_stale`. `occupied_stale` is reserved for the unobserved-death case (no hook, no live watch-pid signal). State explicitly: positive death evidence => immediate teardown; absence of positive liveness evidence => derived-stale flag only."
- what_gets_smaller: removes ambiguity about whether watch-pid death goes through stale-attendance; tightens the dismissal-path matrix into 4 disjoint cases.

#### Crux v — Takeover local-eviction
- epistemic_act: REFINE (this is an OQ2-substantive gap, not just clarification)
- key_claim: gr-retrospective F3 surfaces a substantive missing decision, not a clarification. OQ2 as written only specifies the backend fence (new epoch); it is silent on local daemon-process state. Two cases: (a) cross-daemon takeover — old daemon is gone or self-demoted; new daemon naturally has no local state to evict; (b) intra-daemon takeover — same daemon process accepts an admin RPC reassigning the address to a new occupant; here the daemon MUST evict the prior `session_id -> address` entry, close IPC waiters bound under the old occupant identity, and bind anew. Backend-only fencing leaves the local IPC plane serving the old occupant.
- confidence: 0.85
- grounds: OQ2 plan wording mentions only the epoch mint; the takeover RPC is described as operator-facing, which means intra-daemon is the common case.
- dissent_or_alignment: ALIGN with retrospective F3 on substance; my refinement is that it's BOTH an OQ2 wording fold AND a separate gating test (local state is not what the existing 4 cover).
- relevance: CORE
- smallest_change: OQ2 final wording adds: "Takeover is atomic at the daemon: mint new epoch, evict old `session_id -> addresses` mapping for the rotated address, close IPC waiters bound under the prior occupant, bind under the new occupant — single critical section, no observable intermediate state." And: add a 5th gating test (or extend an existing one) — intra-daemon takeover local-eviction: assert that after takeover, a `wait` connected under the old occupant receives a defined error/disconnect (not silent hang), and a new `wait` under the new occupant receives subsequent messages.
- what_gets_smaller: closes the local-IPC-plane gap; aligns OQ2 with how the daemon actually runs.

#### Crux vi-a — Q-A (ADR 0019 split)
- epistemic_act: CONCEDE
- key_claim: Keep 8 ADRs as drafted. With gr-retrospective F4 supporting 8 and the consolidated form workable, splitting 0019 is not worth a CORE finding.
- confidence: 0.9
- grounds: ADJACENT by my own classification; not a faithfulness/completeness break.
- dissent_or_alignment: CONCEDE to retrospective F4 (round 1).
- relevance: ADJACENT
- smallest_change: 0019 body briefly enumerates the two concerns (IPC scoping + session ownership) under a "Scope" header and notes "considered splitting; declined for log brevity."
- what_gets_smaller: removes my F6 from the must-do list.

#### Crux vi-b — F7 observables (CORE vs ADJACENT)
- epistemic_act: REFINE (keep ADJACENT, but elevate via OQ7's existing clause)
- key_claim: OQ7's "frozen = field set + the 4 gating tests' observable assertions" already commits the doc to enumerating per-test observables. So the requirement EXISTS; F7's risk is the writer might gloss it. GO-WITH-CHANGES is fine if the change is: each gating-test subsection in `daemon.md` enumerates (a) preconditions, (b) action, (c) observable assertions referencing Status fields by name. This is a writing-discipline gate, not a design gate.
- confidence: 0.85
- grounds: OQ7 wording already covers it; my F7 was about ensuring it actually gets written.
- dissent_or_alignment: NEUTRAL — retrospective passed OQ7 without probing this; my probe is compatible.
- relevance: ADJACENT
- smallest_change: WI-2 / WI-7 acceptance criterion: "each of the 4 gating tests has an explicit `Observables:` bullet list naming the Status fields and value transitions asserted."
- what_gets_smaller: lifts F7 from "risk" to "checklisted writer obligation"; no plan restructure.

#### Crux vii — sessionEnd + delivery-fence (gr-premortem F4/F5)
- epistemic_act: SUPPORT both (concede I missed them)
- key_claim (F4): AGREE — sessionEnd hook MUST NOT refresh `attendance_last_confirmed_at`. sessionEnd is a removal signal; refreshing on the same path that deregisters is incoherent and creates a small window where a half-processed hook (refreshed but failed before deregister) artificially reanimates a dying session. Plan as written says "register, each `wait` connect, and each hook touch" refreshes — "each hook touch" is too broad. Refine to: positive-presence hooks (Register, sessionResume/connect-style) refresh; sessionEnd does NOT refresh, only removes; failed sessionEnd emits an error and leaves the prior `last_confirmed` unchanged (no refresh, no removal).
- key_claim (F5): AGREE — OQ1 needs an executable backend API contract. Specify: `mark_delivered_if_current_owner(address, owner_instance_id, lease_epoch, message_id) -> Result<{Delivered, NotOwner, AlreadyDelivered}>`; daemon treats `NotOwner` as a self-demotion trigger (mirror of 0-row heartbeat); `AlreadyDelivered` as a no-op (idempotency). Add a gating-test assertion: ownership-loss-around-delivery — daemon binds, message arrives at recipient, ownership rotates between Frame::Message emit and mark_delivered call; assert `NotOwner` returned, daemon self-demotes, no duplicate downstream side-effects (e.g., `seen`-dedup catches it on the recipient side).
- confidence: 0.9
- grounds: F4 — OQ2 wording is too broad on "hook touch"; symmetry argument. F5 — without the typed result enum, "epoch-fenced server-side" is a description not a contract.
- dissent_or_alignment: ALIGN with both premortem F4 and F5.
- relevance: CORE for both
- smallest_change: (F4) split "hook touch" in OQ2 into "positive-presence hook touch" (refreshes) vs "sessionEnd" (removes; never refreshes; error-on-failure); (F5) OQ1 specifies the API signature, the three result variants, the daemon's response to each, and the ownership-loss-around-delivery scenario in the gating test for the epoch fence.
- what_gets_smaller: F4 closes the half-processed-end reanimation hazard; F5 makes the epoch fence implementable rather than aspirational.


---

# Faithfulness check (post-synthesis)

Non-author cross-check by **gr-premortem** (requested model gpt-5.5, provider OpenAI self-reported) on the rapporteur synthesis. Four iterations:

1. **DISPUTE** — `DA-2/coverage falsely cite gr-retrospective R2 Crux B3 for OQ6; Crux B3 was SKILL.`
2. **DISPUTE** — `DA-1 overstates gr-retrospective Crux A; rotation-alone did not ratify drain-then-claim.`
3. **DISPUTE** — `coverage_manifest still treats gr-retrospective Crux A as ratifying DA-1 two-phase cutover.`
4. **SUPPORT** — `final section records SUPPORT; attributions, manifests, minority dissent, provenance align with transcript.`

All three earlier breaks fixed in-place in `synthesis.md`. Iteration history recorded in the synthesis `faithfulness_check` section so future audits see the path, not just the verdict.

