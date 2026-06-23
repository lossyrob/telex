# Council synthesis packet — plan review for local-daemon / design-foundation (telex #34)

## decision_vector

> Is this plan sound, complete, and faithful enough that executing it produces a design-gate-ready telex design layer — resolving all 9 deliverables and all 8 open questions with implementable specifics — WITHOUT re-deciding the ratified architecture? Where are the CORE gaps, faithfulness breaks, or under-specified open-question resolutions, and what is the smallest direction-preserving fix for each?

## recommendation

**GO-WITH-CHANGES.**

The plan is directionally correct: faithful to ratified architecture, preserves the preserved dissent, lands all 9 deliverables in concrete work items, and proposes resolutions for all 8 OQs. The 9 changes catalogued in `decisive_arguments` are tractable in-place edits to `Plan.md` and `daemon.md` (no architectural rework, no new deliverables). They convert resolutions from "slogan-level" to "implementable-contract level," close three deliverable-coverage holes the OQ-focused surfaces would have missed (`seen`-dedup shape, daemon-era `from`-default, single-source SKILL mechanism), and tighten the four lifecycle hazards (legacy cutover overlap, hook semantics, watch-pid death path, takeover local-eviction) into executable state transitions.

## confidence

**HIGH** — round-2 produced epistemic-act SYNTHESIZE/SUPPORT/CONCEDE across the live cruxes with no remaining CORE disagreements; the most consequential gaps are grounded in cited code seams (`src/backend/{sqlite,postgres}.rs`, `src/commands/attach.rs:477/485`, `src/identity.rs`, `src/commands/skill.rs`, `src/session_registry.rs`) rather than speculation.

## convergence

- **Achieved on every CORE finding's direction.** Each of the 9 CORE findings has same-direction smallest-changes from the members who weighed in — with ONE explicit mechanism split on DA-1 (OQ5): gr-premortem + gr-baseline converged on drain-then-claim + occupant rotation; gr-retrospective held in R2 that occupant rotation alone suffices. Synthesis adopts the two-phase position because gr-baseline R2 Crux i produced a wire-level `Frame::Message` proof gr-retrospective's mechanism does not address. Recorded as `minority_report` (mechanism) and `open_questions` OQ-α (framing).
- **One soft ADJACENT swap** on Q-A: in round 2 gr-retrospective conceded to splitting ADR 0019 while gr-baseline conceded to keeping 8; both agreed on the substance (the two concerns inside 0019 must be enumerated clearly). Synthesis lean: keep 8 with a "Scope" header on 0019.
- **No member identified an unfixable architectural problem** (no `stop:`-with-proof). The plan does not require revisiting any ratified decision.
- **No fake convergence.** Every CORE finding ties to a code-seam or doc citation; no shared hedge was substituted for a real specific.

## confidence_basis

- Code-seam grounding: every CORE finding cites at least one verified file/line from the brief or independent confirmation by a member (e.g., baseline verified `DECISIONS.md` numbering 0001..0013 is intact; premortem traced legacy heartbeat in `src/backend/{sqlite,postgres}.rs` and confirmed it returns `Result<()>` with no rowcount).
- Cross-member validation: the three "novel" CORE findings raised by only gr-baseline in round 1 (`seen`-dedup, `from`-default, single-source SKILL) were ratified as CORE by both gr-premortem and gr-retrospective in round 2 with independent grounds (premortem cited `src/identity.rs` and `src/commands/skill.rs`; retrospective endorsed from implementer perspective).
- Provider diversity: gpt-5.5, gemini-3.1-pro-preview, claude-opus-4.7 self-reported as OpenAI, Google, Anthropic respectively (UNVERIFIED, see `provenance_manifest`); no obvious cross-provider fallback signal.
- One independence concern: the synthesis author is the runner, not a member; the faithfulness check is by gr-premortem (gpt family) to avoid Anthropic-author-checking-Anthropic-member.

## decisive_arguments

Each item is a CORE finding the plan should adopt before producing the design layer. `source_agents` is who first raised it (round 1) and who ratified it (round 2).

### DA-1 — OQ5 cutover: two-phase rule (drain THEN claim with occupant rotation)

- **claim:** Occupant-rotation alone cannot fence a live legacy holder; the daemon-side server fence alone cannot prevent in-flight `Frame::Message` double-delivery during overlap. The fix is a two-phase cutover.
- **source_agents:** gr-premortem F1 (R1) + gr-baseline F5 (R1); converged in R2 on **drain-then-claim with occupant rotation** by gr-premortem Crux 1 + gr-baseline Crux i. gr-retrospective Crux A (R2) reached a DIFFERENT mechanism — argued occupant rotation alone IS the cutover (no separate drain) — which gr-baseline's in-flight `Frame::Message` proof in Crux i refutes. Synthesis adopts the gr-premortem + gr-baseline two-phase position because it covers the wire-level double-delivery case gr-retrospective's rotation-alone mechanism does not. See `open_questions` OQ-α.
- **evidence:** `src/backend/sqlite.rs:325-333` and `src/backend/postgres.rs:313-320` — legacy `heartbeat` updates by `address` only and returns `Result<()>` with no rowcount, so a legacy binary cannot observe self-demotion. `src/commands/attach.rs:464-490` — legacy holder ships `Frame::Message` BEFORE `mark_delivered` commits; if the daemon has rebound the address's waiter endpoint, two endpoints emit independently regardless of post-emit row fence.
- **smallest_change to plan:** Rewrite OQ5 with explicit two-phase rule:
  - **Phase 1 (drain):** Daemon-aware claimant detects non-epoch rows; before binding its own waiter, it must confirm no legacy holder is actively bound — either via address-keyed IPC probe with quit/handover signal, OR by waiting for a bounded stale-window. No live overlap of two waiter-binds per address.
  - **Phase 2 (claim):** Only after drain, claim `epoch=1` and atomically rotate `occupant` to `owner_instance_id`. Remove the occupant-null release branch (per OQ1). Use owner/epoch-guarded heartbeat/release that returns rowcount for daemon-aware holders.
  - Add an explicit cutover gating assertion: "no `Frame::Message` from a non-epoch holder reaches a recipient after the daemon's waiter binds."

### DA-2 — OQ6 v1 path: instance-admin capability over user-private daemon IPC

- **claim:** Per-session capability "held in the plugin/session env" is not obtainable on Copilot CLI today because `Register` runs in a child process and the sessionEnd hook is a separately-spawned process; v1 must use the same-trust admin cap.
- **source_agents:** gr-premortem F2 (R1) + gr-baseline F3 (R1); converged in R2 (premortem Crux 2, baseline Crux ii). gr-retrospective did not pressure-test OQ6 (her R2 reactions were OQ5/OQ8/the three novel findings/Q-A/OQ7/sessionEnd/delivery-fence), so this CORE finding rests on two members rather than three — a weaker but still convergent ratification base.
- **evidence:** sessionEnd hook plumbing on `feature/copilot-session-end-plugin` runs `telex session-end` as a separate command (`integrations/copilot-cli/hooks.json:4-9`); `src/commands/session_end.rs:34-43` reads only a session id from stdin/flag; existing branch uses filesystem `session_registry` (`src/session_registry.rs`) which the plan explicitly drops as authority. A child `attach` cannot mutate the parent harness's environment for a later hook process.
- **smallest_change to plan:** Lead OQ6 with: "Hook calls `DeregisterSession(session_id, admin_cap)`; daemon verifies `admin_cap` matches instance secret and `session_id` exists in its in-memory map; if so, drops associated addresses. Protocol reserves `per_session_cap: Option<Cap>` as a forward-defense field against lateral hook compromise (defer with rationale: same-trust user-private threat model in v1 makes per-session cap zero-marginal-value over admin cap). No filesystem session registry. No per-session cap minted at Register in v1." This preserves the ratified "one token v1, scope/rotation reserved" without requiring an impossible env carrier.

### DA-3 — OQ8 attendance recovery: `suspect` / `verified` / `lapsed` state machine + `wait` auto-Re-register

- **claim:** A respawned daemon must not freshen recovered rows as live attendance without proof; the long-lived `wait` is the only client that can re-prove liveness for an already-running session, and it must auto-Re-register on UnknownSession from inherited env.
- **source_agents:** gr-premortem F3 (R1) + gr-retrospective F2 (R1); converged + idempotency residual added in R2 (premortem Crux 3, retrospective Crux C, baseline Crux iii).
- **evidence:** OQ8 wording (`Plan.md:182-189`) — durable rows survive, in-memory `session_id->addresses`/watch-pids/IPC waiters do not. Loader's `attach` is one-shot; only `wait` is long-lived. Standard socket semantics: ERROR_BROKEN_PIPE on Windows named pipes / EPIPE on Unix domain sockets wakes a blocked `wait` read.
- **smallest_change to plan:** OQ8 specifies an attendance state machine in `daemon.md`:
  - **suspect** — on respawn, all rows recovered from durable lease/delivery storage are marked `suspect`. Daemon MUST NOT heartbeat or deliver for `suspect` rows.
  - **verified** — promoted by a successful `Register`, `Re-register`, or authenticated `wait`-connect with valid `TELEX_SESSION_ID` and `admin_cap`. Promotion refreshes `last_confirmed` and rebuilds the watch-pid set.
  - **lapsed** — recovered after daemon-down TTL or stale-attendance/takeover with no proof.
  - **`wait` client contract:** broken-pipe + UnknownSession-on-reconnect is a normal control-flow path. `wait` MUST re-register from `TELEX_SESSION_ID` + `admin_cap` in inherited env before failing.
  - **Idempotency:** Re-register MUST be idempotent — concurrent waits for the same `session_id` converge to a single map entry (daemon.md picks last-writer-wins on address set OR union).
  - Add this to the crash-during-`wait` gating test; the suspect-row invariant strengthens the competing-daemons gating test too.

### DA-4 — OQ3/OQ4 dismissal-path matrix: 4 disjoint cases (watch-pid death = immediate teardown)

- **claim:** Positive death evidence (watch-pid anchor pid dead OR start-time mismatch) is categorically different from "idle"; routing it through `occupied_stale` makes the empirical OQ4=no resolution non-recoverable until manual takeover, defeating the "no idle teardown, but DO clean up dead" intent.
- **source_agents:** gr-retrospective F1 (R1); converged in R2 (premortem Crux 5, baseline Crux iv).
- **evidence:** OQ4 names loader anchor + start-time as the SOLE env-sourced backstop on Copilot CLI; OQ2 says "never tears down" — the wording overlap is the bug.
- **smallest_change to plan:** OQ3 wording adds: "When the watch-pid predicate transitions to dead (anchor pid no longer alive OR start-time mismatch), the daemon's local watcher MUST issue an internal `DeregisterSession` for the associated `session_id`, bypassing `occupied_stale`. `occupied_stale` is reserved for the unobserved-death case (no hook, no watch-pid signal)." Dismissal-path matrix in `daemon.md` lists exactly 4 disjoint cases: (1) sessionEnd hook = clean exit; (2) watch-pid failure = crash/kill; (3) operator takeover = override; (4) daemon-down TTL = backstop only. Restate "no idle-TTL teardown" as: "no time-based dismissal of *live* sessions; positive death evidence triggers immediate teardown."

### DA-5 — OQ2 takeover: atomic at the daemon (mint epoch + evict map + close IPC waiters + bind anew)

- **claim:** Backend epoch fencing alone leaves stale IPC waiters and `session_id->addresses` mappings inside the same daemon process; intra-daemon takeover (the common case) needs atomic local eviction.
- **source_agents:** gr-retrospective F3 (R1); converged in R2 (premortem Crux 5, baseline Crux v).
- **evidence:** OQ2 plan wording (`Plan.md:133-141`) describes only the epoch mint; daemon singleton identity (ratified) means intra-daemon is the common case.
- **smallest_change to plan:** OQ2 adds: "Takeover is atomic at the daemon: mint new backend epoch, evict prior `session_id->addresses` entry for the rotated address, close IPC waiters bound under the prior occupant, bind under the new occupant — single critical section, no observable intermediate state." Add a 5th gating test (or extend an existing one): **intra-daemon takeover local-eviction** — assert a `wait` connected under the old occupant receives a defined error/disconnect (not silent hang), and a new `wait` under the new occupant receives subsequent messages.

### DA-6 — OQ2 hook semantics: split positive-presence (refresh) vs sessionEnd (remove, never refresh)

- **claim:** OQ2's "register, each `wait` connect, and each hook touch" refreshing `last_confirmed` is too broad; refreshing on the sessionEnd path that also deregisters creates a half-processed-end reanimation window (refreshed but failed before deregister artificially reanimates a dying session).
- **source_agents:** gr-premortem F4 (R1); converged in R2 (retrospective Crux F, baseline Crux vii).
- **evidence:** sessionEnd is the only currently-shipped hook (`integrations/copilot-cli/hooks.json` on the feature branch); it is a removal signal, not positive attendance.
- **smallest_change to plan:** Split "hook touch" in OQ2 wording: positive-presence hooks (Register, sessionResume / connect-style hooks if added later) refresh `last_confirmed`; sessionEnd does NOT refresh — it removes session membership / releases addresses; failed sessionEnd records a recent error and leaves prior `last_confirmed` unchanged (no refresh, no removal).

### DA-7 — OQ1 delivery fence: executable backend API contract + ownership-loss-around-delivery test

- **claim:** "`mark_delivered_if_current_owner` — no message frame unless the daemon owns the epoch" is description-level; without a typed result enum, daemon-core can rebuild the existing pre-mark race with two queries.
- **source_agents:** gr-premortem F5 (R1); converged in R2 (retrospective Crux G, baseline Crux vii).
- **evidence:** `src/commands/attach.rs:477/485` — the verified hazard is exactly that a `Frame::Message` ships before `mark_delivered` commits. The brief and ratified architecture name the server-side fence as the closure mechanism.
- **smallest_change to plan:** OQ1 specifies the backend API contract:
  - `mark_delivered_if_current_owner(address, owner_instance_id, lease_epoch, message_id) -> Result<{Delivered, NotOwner, AlreadyDelivered}>`
  - daemon's response: `NotOwner` → self-demote (mirror of 0-row heartbeat); `AlreadyDelivered` → no-op (idempotency); `Delivered` → permit `Frame::Message` emission.
  - **The ordering invariant:** the daemon MUST receive a non-`NotOwner` result BEFORE emitting `Frame::Message`. No frame after `NotOwner` or `AlreadyDelivered`.
  - Add a gating-test scenario for **ownership-loss-around-delivery**: daemon binds, message arrives, ownership rotates between the call and the response; assert `NotOwner` returned, daemon self-demotes, no duplicate downstream frame.
  - Extend the existing handoff-duplicates gating test to exercise this race.

### DA-8 — `seen`-dedup redesign: durable `deliveries` as authority + bounded in-memory fast-path keyed by lease_epoch

- **claim:** "Redesign `seen` for a long-lived daemon" is a Council-B ratified prerequisite, named in ADR 0016 and WI-2 but never given a shape (bounded vs durable, key, seeding, eviction). It is not an OQ, which is why the existing OQ-pressure-test missed it — a coverage hole.
- **source_agents:** gr-baseline F1 (R1); ratified CORE in R2 (premortem Crux 4a, retrospective Crux B1).
- **evidence:** Current `seen` is unbounded `Mutex<HashSet<i64>>` (`src/commands/attach.rs:32-41,67-83`), never pruned because holders restart; daemon voids that. Durable `deliveries(message_id, recipient)` is unique-keyed (`src/backend/sqlite.rs:66-72`, `src/backend/mod.rs:55-73`) and is already cross-restart authority.
- **smallest_change to plan:** Add a labeled `seen`-redesign resolution to `Plan.md` (alongside the 8 OQs) and reflect in ADR 0016:
  - Durable `deliveries(message_id, recipient)` is the cross-epoch dedup authority (no behavioral change to 0011/0013).
  - In-memory dedup is a bounded fast-path keyed by `(recipient, message_id, lease_epoch)` or equivalent in-flight identity.
  - Reset/drop on epoch loss (self-demote, takeover).
  - Seed from `fetch_undelivered` on claim.
  - Evict on: durable mark via `mark_delivered_if_current_owner` returning Delivered; terminal disposition; epoch transition.

### DA-9 — `from`-default daemon-era resolution rule (ADR 0019 must specify the supersession's replacement)

- **claim:** ADR 0019 supersedes 0010's local-holder-registry as the `from`-default source, but the plan never specifies the daemon-era replacement. With one daemon serving N sessions, 0010's "uniquely live local station for the current backend" degenerates to refused-ambiguous-from in the common multi-session case — a silent UX regression. Implementer will be forced to invent a rule at the first `send` against a daemon.
- **source_agents:** gr-baseline F2 (R1); ratified CORE in R2 (premortem Crux 4b, retrospective Crux B2).
- **evidence:** Current `from` precedence falls back to the uniquely live local station via the holder registry (`src/identity.rs:1-17,72-116`, `src/registry.rs:131-155`). Daemon-native session ownership (deliverable 6) gives the daemon a `session_id->addresses` map, but no design ties it to `from`-resolution.
- **smallest_change to plan:** Add daemon-era `from`-resolution rule in `daemon.md` (and a parenthetical in ADR 0019):
  - Precedence: explicit `--from` > `$TELEX_ADDRESS` / `--address` > daemon `ResolveFrom(TELEX_SESSION_ID)` against that session's registered addresses.
  - Resolution: exactly one address inferred → succeed; multiple → refuse with `ambiguous-from`; none → existing unrepliable/refusal rules.
  - **Never** infer from all daemon-owned addresses (across sessions).
  - Harness/plugin MUST propagate `TELEX_SESSION_ID` to the `send`/`reply` process environment.
  - (Acceptable alternative: explicit defer entry in the relocations/supersedes/defers map citing a named follow-up node, with ADR 0019 stating "0010 mechanism superseded; policy in force pending <follow-up>." — but a one-paragraph in-place spec is preferred and cheap.)

### DA-10 — Single-source SKILL.md / plugin-skill mechanism (Deliverable 7 sub-requirement)

- **claim:** Issue #34 deliverable 7 explicitly names "specify the single-source SKILL.md / plugin-skill mechanism (one source serves both the CLI command and the plugin skill)." WI-5 ADR 0021 names verb + docs/SKILL cutover but not the mechanism — a deliverable-coverage hole.
- **source_agents:** gr-baseline F4 (R1); ratified CORE in R2 (premortem Crux 4c, retrospective Crux B3).
- **evidence:** Task spec `design-foundation.md:99-104`; current CLI embeds root `SKILL.md` via `include_str!` (`src/commands/skill.rs:1-10`); plan correctly keeps `SKILL.md` at root (moving needs code) but is silent on plugin-skill consumption.
- **smallest_change to plan:** ADR 0021 (and one paragraph in `daemon.md` or DESIGN.md) name:
  - Canonical file: root `SKILL.md` (unchanged).
  - CLI consumer: `include_str!` in `src/commands/skill.rs` (unchanged).
  - Plugin-skill consumer: manifest pointer if the harness supports it, otherwise a thin wrapper that `exec`s `telex skill --raw`.
  - Invariant: no generated divergent copy; both consumers point at the same file.

## minority_report

**One CORE mechanism minority preserved (DA-1 OQ5).** gr-retrospective in R2 Crux A argued that occupant rotation alone IS the cutover mechanism (no separate drain phase): "Legacy `telex` updates attendance via `WHERE address=? AND occupant=?`. If the daemon claims epoch 1 by writing its `owner_instance_id` into the `occupant` field, the legacy heartbeat will naturally 0-row on its next tick, triggering legacy's existing 'lost ownership' shutdown logic." Synthesis adopts the gr-premortem + gr-baseline two-phase position (drain THEN claim) because gr-baseline's R2 Crux i traced the actual code seam: the legacy holder ships `Frame::Message` at `src/commands/attach.rs:~477` BEFORE its post-emit `mark_delivered` at `~485` checks ownership; a daemon that has rebound the same address's IPC waiter endpoint will independently emit; two waiter-binds existing transiently produces wire-level double-delivery regardless of whether the legacy holder's subsequent DB write is rejected. gr-retrospective's mechanism does not address the wire-level case; the two-phase mechanism does. The minority position would be adoptable only if (a) the legacy holder's local heartbeat actually returned rowcount and self-terminated on 0-row (current `heartbeat` returns `Result<()>` per `src/backend/{sqlite,postgres}.rs:325-333/313-320`), or (b) the legacy holder somehow could not bind its IPC waiter once the daemon had rotated occupant — neither is the case today.

**One ADJACENT framing swap (Q-A).** In round 2 gr-retrospective conceded to splitting ADR 0019 (9 ADRs total) while gr-baseline conceded to keeping 8 with a Scope header. Substance is identical (the two concerns inside 0019 must be enumerated clearly); count is the only divergence. Synthesis lean: keep 8 (see `open_questions` OQ-β).

**No other CORE disagreement survived round 2.**

## parked

- Full non-binary occupant-status policy (named PARKED in brief; ratified as out-of-scope).
- fd-over-IPC pid-reuse-immune backstop (PARKED in brief).
- Daemon-owned directory/occupancy reads (PARKED).
- Per-session cryptographic capability enforcement in v1 (parked by DA-2; protocol field reserved).
- Per-session-PID OQ4 reopen (closed empirically; do not reopen without contradicting evidence).
- Issue #34 body amendment for the docs/design deviation (Q-D resolved: PR + field report only; orchestrator action).

## coverage_manifest

Each of the 9 coverage-checklist surfaces mapped to a verdict + the member(s) who examined it.

1. **Faithfulness to node outcome anchor** — **core-finding-present** (DA-8 `seen` shape + DA-9 `from`-default + DA-10 single-source SKILL each close a sub-deliverable gap that would otherwise downgrade to enabling work). Examined: gr-baseline (R1 F1/F2/F4); ratified by gr-premortem (R2 Crux 4) + gr-retrospective (R2 Crux B).
2. **OQ1 / OQ5 / OQ8 (epoch + crash + cutover triad)** — **core-findings-present** (DA-1 OQ5 two-phase cutover; DA-3 OQ8 suspect/verified/lapsed + wait re-register; DA-7 OQ1 delivery-fence contract). Examined: gr-premortem (R1 F1/F3/F5).
   - DA-1 OQ5 ratification base: gr-premortem R1 F1 + R2 Crux 1, gr-baseline R1 F5 + R2 Crux i. gr-retrospective R2 Crux A is NOT a ratifying voice for two-phase cutover — she argued the rotation-alone alternative; recorded as `minority_report` mechanism dissent.
   - DA-3 OQ8 ratification base: gr-premortem R2 Crux 3 + gr-retrospective R2 Crux C + gr-baseline R2 Crux iii (idempotency residual).
   - DA-7 OQ1 ratification base: gr-premortem R1 F5 + gr-retrospective R2 Crux G + gr-baseline R2 Crux vii.
3. **OQ2 stale-attendance / takeover** — **core-findings-present** (DA-5 atomic local eviction; DA-6 sessionEnd hook split). Examined: gr-premortem (R1 F4) + gr-retrospective (R1 F3); ratified by gr-baseline (R2 Crux v/vii).
4. **OQ6 DeregisterSession proof** — **core-finding-present** (DA-2 admin-cap v1 path). Examined: gr-premortem (R1 F2) + gr-baseline (R1 F3); ratified in R2 by gr-premortem Crux 2 + gr-baseline Crux ii. gr-retrospective did not pressure-test OQ6 (round-1 examined-and-passed entry was Q-B verb mapping, not OQ6 authorization); ratification base is two members.
5. **OQ3 / OQ4 watch-pid + empirical OQ4=no** — **core-finding-present** (DA-4 dismissal-path matrix). Examined: gr-retrospective (R1 F1); ratified by gr-baseline (R2 Crux iv) + gr-premortem (R2 Crux 5).
6. **OQ7 Status freeze line** — **examined-passed-with-writing-discipline-gate** (boundary is clean; ADJACENT requirement that each gating test gets per-test observable assertions). Examined: gr-retrospective (R1 passed) + gr-baseline (R1 F7 raised observables); ratified ADJACENT in R2.
7. **Document architecture** — **examined-passed-with-minor-note** (root vs `docs/design/` split + `daemon.md`-spine + SKILL.md binary-embed correctly excluded all pass; the only structural issue is the ADR 0019 Scope-header preference in `open_questions` OQ-β). Examined: gr-retrospective (R1 passed) + gr-baseline (R1 F6).
8. **Plan completeness / sequencing** — **examined-passed** (single-threaded ordering coherent; definition-of-done adequate; all 9 deliverables map to a real WI per gr-baseline R1). The three coverage holes called out are sub-deliverable specs, not WI omissions; closed by DA-8/DA-9/DA-10.
9. **Faithfulness vs ratified + preserved dissent** — **examined-passed** (no silent contradictions found: verbs preserved per ADR 0007 + preserved-dissent; no held-stream SessionConnect; capability "one token v1, scope/rotation reserved" matches DA-2; lease-epoch fence wraps `mark_delivered_if_current_owner` server-side; `seen` redesign per Council B [shape now specified by DA-8]; takeover called "load-bearing" per Council E; minimal upgrade floor split per Council C; daemon-native `DeregisterSession` per Council G). Examined: gr-baseline (R1); ADR numbering verified clean 0001..0013 → 0014.

## open_questions

- **OQ-α — OQ5 framing nuance (CORE substance settled, framing residual).** gr-retrospective in R2 framed occupant rotation as the cutover mechanism on its own; gr-premortem and gr-baseline (the latter with the in-flight `Frame::Message` proof) argued drain-then-claim is required. Synthesis adopts drain+rotate (the stronger position) because it covers the wire-level double-delivery case. Resolution: writer of `daemon.md` should write the two-phase rule per DA-1 and not the rotation-alone variant.
- **OQ-β — Q-A final form (ADJACENT).** Keep 8 ADRs with a Scope header on 0019 noting "considered splitting; declined for log brevity," OR split into 9 with renumbered 0020/0021 → 0021/0022. Either is acceptable; the substance (enumerate IPC scoping + session ownership clearly) is the same. Lean: 8.
- **OQ-γ — sessionResume hook semantics (ADJACENT).** DA-6 splits hook semantics for sessionEnd; the design should also state up front whether any future positive-presence resume hook is in scope or explicitly out of scope at this layer (so daemon-core is not stranded if such a hook lands later). Lean: state "if a positive-presence hook is added later, it joins the refresh path; design-foundation does not require it in v1."

## reopen_conditions

The council recommendation should be reopened (new round, possibly new evidence) if any of the following surfaces:

- The drain phase of DA-1 cannot be realized via existing address-keyed IPC + bounded stale-wait (i.e., a new IPC verb is required) — would convert CORE direction-preserving fix into an architectural change.
- A Copilot CLI plugin API surface is identified (current or imminent) that lets the plugin pre-populate the sessionEnd hook's env from a value captured at `attach` — would let per-session cap be the v1 path and DA-2 should be re-tightened, not loosened (still reserved, but with concrete carrier).
- The wait-Re-register path in DA-3 cannot be implemented because socket-EOF semantics are masked by the chosen IPC transport (would force introducing a positive-presence heartbeat from `wait`).
- The single-source SKILL mechanism in DA-10 hits a harness constraint (plugin manifest cannot point at a file outside the plugin dir, AND `exec`ing the binary is rejected) — would force a code-touching deviation that violates "no production code" in this node.

## audit_triggers

- The `faithfulness_check` returns DISPUTE (see below).
- Any CORE finding lands in `daemon.md` with weaker wording than the `smallest_change` in this packet (e.g., DA-7 ships without typed result enum; DA-3 ships without the explicit `suspect`/`verified`/`lapsed` state-machine).
- The "considered splitting; declined for log brevity" note on ADR 0019 is omitted (forces orchestrator to re-litigate Q-A).
- The relocations/supersedes/defers map (WI-6) ships without entries for `from`-default policy (DA-9) and single-source SKILL mechanism (DA-10) — these are the easiest places for the deliverable-coverage holes to silently re-open.
- Any model-identity fallback signal in the live runtime (e.g., gr-premortem turn arrives self-reporting Anthropic) — would invalidate the provider-diversity claim and demote `convergence` to "two members + one duplicate."

## faithfulness_check

**SUPPORT (after three iterations).** By **gr-premortem** (requested model `gpt-5.5`, non-author, non-Anthropic for genuine independence from the rapporteur).

Iteration history (recorded for audit honesty):
- **Pass 1: DISPUTE** — `DA-2/coverage falsely cite gr-retrospective R2 Crux B3 for OQ6; Crux B3 was SKILL.` Fixed: removed gr-retrospective from DA-2 source_agents and from coverage surface 4 ratifier list; ratification base now correctly stated as gr-premortem + gr-baseline (two members, not three).
- **Pass 2: DISPUTE** — `DA-1 overstates gr-retrospective Crux A; rotation-alone did not ratify drain-then-claim.` Fixed: DA-1 source_agents now lists gr-retrospective Crux A as a DIFFERENT mechanism (rotation-alone) that the synthesis adopted away from with proof (gr-baseline's wire-level `Frame::Message` argument); convergence section updated; minority_report rewritten to preserve the mechanism dissent with full grounds.
- **Pass 3: DISPUTE** — `coverage_manifest still treats gr-retrospective Crux A as ratifying DA-1 two-phase cutover.` Fixed: coverage surface 2 line now per-OQ-itemizes DA-1/DA-3/DA-7 ratification bases and explicitly excludes gr-retrospective Crux A from the OQ5 two-phase ratifiers, pointing to minority_report.
- **Pass 4 (final): SUPPORT** — DA-1..DA-10 each map to real transcript turns with correct attribution; minority_report honestly records the DA-1 OQ5 mechanism dissent (rotation-alone vs drain-then-claim) with grounds for adopting the two-phase position; coverage_manifest per-finding ratification bases match transcript; provenance_manifest accurate; OQ-α framing nuance and OQ-β Q-A swap preserved.

Audit-honest note: the iteration count itself is a weak signal that the original rapporteur draft was insufficiently rigorous on attribution detail. The final state is faithful; the path to it was not single-shot. This is recorded rather than erased.

## provenance_manifest

| agent_id | requested_model | persona | provider_family (self-reported) | model_identity | fallback_suspected |
|---|---|---|---|---|---|
| gr-premortem | gpt-5.5 | general-reviewer + premortem overlay | OpenAI (GPT family, tier unsure) | UNVERIFIED | no |
| gr-retrospective | gemini-3.1-pro-preview | general-reviewer + retrospective overlay | Google / Gemini 3 / pro tier | UNVERIFIED | no |
| gr-baseline | claude-opus-4.7 | general-reviewer + baseline lens | Anthropic / Claude (Opus-class assumed per request) | UNVERIFIED | no |

Notes on provenance honesty:
- Each member self-reported a provider_family consistent with the requested model in R1 and R2. No member reporting Anthropic for a non-Claude request, nor OpenAI for a non-GPT request, was observed — so no fallback signal.
- Model identity is UNVERIFIED — this is the standard runtime caveat; the runtime may silently fall back, and self-report alone is not proof.
- Nesting outcome: **nested subagents** (3 members spawned in parallel as `task` agents). Not degraded-single-context.

## artifact paths

- **brief_path:** `C:\Users\robemanuele\proj\telex\telex-design-foundation\.paw\work\design-foundation\reviews\planning\council-plan-01\brief.md`
- **transcript_path:** `C:\Users\robemanuele\proj\telex\telex-design-foundation\.paw\work\design-foundation\reviews\planning\council-plan-01\transcript.md`
- **synthesis_path:** `C:\Users\robemanuele\proj\telex\telex-design-foundation\.paw\work\design-foundation\reviews\planning\council-plan-01\synthesis.md`
