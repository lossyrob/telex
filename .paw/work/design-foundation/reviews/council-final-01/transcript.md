# Council transcript — FINAL review of design layer (telex #34) — council-final-01

Mode: deliberate, depth: medium (3 members, isolated panel + up to 2 interaction rounds).
Decision vector: Does the produced design layer faithfully and consistently realize the
reviewed plan (9 deliverables + 8 OQ resolutions at contract strength) with no internal
inconsistencies, factual/logical errors in the written contracts, or faithfulness breaks,
such that it is ready for the builder's design-gate? Where it falls short, what is the
smallest direction-preserving fix?

Roster:
- gr-baseline       — general-reviewer, baseline lens (requested model: claude-opus-4.7, effort high). Surfaces: faithfulness DA-1..DA-10 + consistency + completeness (#1, #2, #4).
- gr-premortem      — general-reviewer + premortem overlay (requested model: gpt-5.5, effort high). Surfaces: correctness of written contracts + ADR quality + faithfulness-to-ratified (#3, #5, #7).
- gr-retrospective  — general-reviewer + retrospective overlay (requested model: gemini-3.1-pro-preview, effort high). Surfaces: downstream-implementer ambiguity + doc architecture/links + new errors / PR framing (#6, #8, #9).

Any member may raise any surface. CORE = concrete defect in the written docs (wrong/contradictory claim, contract weaker than DA-x strength, missing deliverable/OQ, dead link, broken supersession). ADJACENT/PARKED never blocking.

---


---

# Round 1 (isolated panel)

## gr-baseline — round 1
- agent_id: gr-baseline
- requested_model: claude-opus-4.7
- provider_family (self-reported): anthropic
- persona+overlay: general-reviewer + baseline
- epistemic_act: SUPPORT
- timestamp_utc: 2026-06-23T03:02:33Z

### Coverage notes (per assigned surface)

**Surface #1 — Faithfulness to the plan (DA-1..DA-10 + Q-A + 5 gating tests at contract strength).**
Examined the produced artifacts against `.paw/work/design-foundation/Plan.md` "Open-question resolutions" and `reviews/planning/council-plan-01/synthesis.md` DA-1..DA-10. Verdict: **examined-passed.** Each DA-x is written at the synthesized strength, in the locations the plan named, with the proof-of-strength tokens preserved (the typed result enum for DA-7, the frozen wire-level assertion for DA-1, the `bypassing occupied_stale` rule for DA-4, the epoch-keyed fast-path with `drop on epoch loss` for DA-8, etc.).
- DA-1 (drain-then-claim + frozen "no Frame::Message from a non-epoch holder reaches a recipient after the daemon's waiter binds") — `daemon.md` §12 lines 539–554 + ADR 0020 lines 803–807. ✓
- DA-2 (admin_cap v1, per-session cap reserved + deferred with separate-process rationale) — `daemon.md` §7.1 lines 312–326 + §14.2 lines 605–615 + ADR 0019 lines 770–779. ✓
- DA-3 (suspect/verified/lapsed; MUST NOT heartbeat or deliver for suspect; wait auto-Re-register on `UnknownSession`; idempotent Re-register) — `daemon.md` §14.3 lines 619–636 + §14.4 lines 638–648. ✓
- DA-4 (4-case matrix; watch-pid death → internal `DeregisterSession` **bypassing `occupied_stale`**; "no time-based dismissal of a *live* session; positive death evidence = immediate teardown") — `daemon.md` §9.3 lines 392–405 + §9 line 350–353. ✓
- DA-5 (takeover atomic at the daemon: mint epoch + evict map + close IPC waiters + bind, single critical section; intra-daemon takeover local-eviction gating test) — `daemon.md` §10.2 lines 425–443 + §17 test 5 lines 727–730. ✓
- DA-6 (sessionEnd does NOT refresh `last_confirmed`; failed sessionEnd records error and leaves `last_confirmed` unchanged) — `daemon.md` §10.1 lines 410–417. ✓
- DA-7 (typed `mark_delivered_if_current_owner(...) -> {Delivered|NotOwner|AlreadyDelivered}` with the ordering invariant: non-`NotOwner` BEFORE any `Frame::Message`; ownership-loss-around-delivery scenario) — `daemon.md` §11.3 lines 481–504 + §17 test 4 lines 723–726 + ADR 0015 lines 651–658. ✓
- DA-8 (durable `deliveries` = cross-epoch authority; bounded fast-path keyed `(recipient, message_id, lease_epoch)`; seed on claim; evict on durable-mark/terminal-disposition/epoch-transition; drop on epoch loss) — `daemon.md` §13 lines 563–588 + ADR 0016. ✓
- DA-9 (daemon-era `ResolveFrom(TELEX_SESSION_ID)` against that session's addresses; never infer across sessions; supersedes 0010's mechanism) — full precedence rule including ambiguous-from in `DESIGN.md` "Default `from` via daemon session ownership" lines 401–420; `daemon.md` carries the supersession entry in the relocations table line 765; ADR 0019 line 776–778 carries the daemon-side parenthetical; ADR 0010 status line 394 records the supersession. ✓
- DA-10 (canonical root `SKILL.md`; CLI `include_str!`; plugin = manifest pointer or `exec telex skill --raw`; no divergent copy) — `daemon.md` §15.2 lines 679–689 + ADR 0021 lines 829–832. ✓
- Q-A (8 ADRs with explicit Scope header on 0019) — DECISIONS.md ADR 0019 lines 754–758 has the Scope header noting "splitting was considered and declined for log brevity." ✓
- Five gating tests with per-test observable assertions (OQ7 freeze) — `daemon.md` §17 lines 707–730 enumerates exactly five (concurrent first-use, crash-during-`wait`, competing daemons, handoff duplicates + ownership-loss-around-delivery, intra-daemon takeover local-eviction), each with explicit assertions; ADR 0018 lines 742–744 names the same five. ✓
- OQ-γ (sessionResume hook scope) — `daemon.md` "Open-question resolutions" lines 748–752. ✓
- OQ4 = no usable per-session PID empirically grounded — `daemon.md` §9.2 lines 374–388. ✓

**Surface #2 — Internal consistency (DESIGN.md, daemon.md, ADRs agree).**
Examined every prior-ADR status line against the daemon.md narrative and ADR 0014–0021 bodies; cross-checked the `from`-default and `seen`-redesign supersession chains end-to-end; checked the relocations/supersedes table in daemon.md §"Relocations, supersessions, deferrals" vs DECISIONS.md status lines. Verdict: **examined-passed with one ADJACENT pre-existing typo (F-baseline-1).**
- ADR 0004 status: "Superseded by 0014…" ✓ matches `daemon.md` §1 holder removal and ADR 0014 supersedes line.
- ADR 0005 status: "Narrowed by 0017 to the daemon-down backstop role" ✓ matches `daemon.md` §14.5 "the one surviving role of the old TTL heartbeat" and §9 "Lease TTL survives in exactly one role: the daemon-down backstop."
- ADR 0009 status: "Amended by 0014 (station recast)" ✓ matches DESIGN.md §"Station: a registration in the local exchange" lines 54–84 (no longer a resident pair).
- ADR 0010 status: "Accepted (policy stands; mechanism superseded by 0019)" ✓ matches DESIGN.md §"Default `from` via daemon session ownership" lines 401–420 (precedence preserved; mechanism is `ResolveFrom`).
- ADR 0012 status: "Relocated by 0017: pid-watch moves from the holder into the exchange; ppid rejection stands, reaffirmed by OQ4 probe" ✓ matches `daemon.md` §9.1–§9.2.
- ADR 0013 status: "Drain retained; never-prune `seen` rationale superseded by 0016's bounded epoch-keyed fast-path" ✓ matches `daemon.md` §13.
- DESIGN.md describes the architecture in the local-exchange end-state with explicit pointers to `daemon.md` for mechanism (lines 10–13: "this document owns the architecture and framing and points into daemon.md for the precise contracts"). No DESIGN.md prose describes a superseded mechanism (holder + waiter / local-holder registry) as current.

**Surface #4 — Completeness (9 deliverables substantive; 8 OQs resolved with implementable specifics; deferred items explicit; relocations map complete).**
Verdict: **examined-passed.**
- D1 — ADR 0014 + 0015 + 0016 + 0017 + 0018 + 0019 + 0020 + 0021 present, each with the named supersessions/amendments and pointing into daemon.md sections. ✓
- D2 — DESIGN.md rewritten to the local-exchange end-state; station recast; "no required hosted server" intact. ✓
- D3 — PRODUCT-THESIS.md lines 90–93 + 128–129 carry the auto-spawned local-exchange framing. ✓
- D4 — IPC/attendance protocol + authorization: `daemon.md` §6 + §7. ✓
- D5 — Lifecycle + Status surface (frozen) + 5 gating tests: `daemon.md` §3 + §4 + §17. ✓
- D6 — daemon-native session ownership: `daemon.md` §14 (in-memory authority; Register/ReRegister/DeregisterSession; ResolveFrom). ✓
- D7 — verbs preserved; daemon entrypoint hidden; single-source skill: `daemon.md` §15. ✓
- D8 — minimal upgrade floor + two-phase cutover rule: `daemon.md` §12 + §16. ✓
- D9 — consolidated OQ resolutions: `daemon.md` "Open-question resolutions" with each OQ rowed and pointed at its specifying section. ✓
- Deferred items explicit: `daemon.md` "Deferred (explicit — not dropped)" lines 781–793 lists full non-binary status policy, fd-over-IPC, daemon-owned directory reads, `per_session_cap`/multi-tier, #27, #24, #12. ✓
- Reopen conditions carried: lines 795–809 list all four conditions from the plan-review synthesis. ✓
- Relocations/supersedes table: lines 759–779 cover ADR supersessions and issue/PR dispositions (#32, #23/#31, #5/#17, #3, #26, #6) — includes the from-default and single-source SKILL placements that the synthesis flagged as the easiest holes to silently re-open (audit_triggers). ✓

**Other surfaces I spot-checked (not assigned but examined):**
- Doc-architecture + links (surface #6): index.md lines 11–35 lists DESIGN.md/DECISIONS.md/daemon.md with the right one-line each; reading order coherent; OQ-resolution pointer correct. All `daemon.md` intra-document anchors I followed (`§11.4`, `§14.3`, `§3.3`, `§14.4`, `§11.3`, `§11.1`, `§10.2`, §16, §17) resolve to real section headings. ✓
- ADR numbering (surface #5): clean 0001..0021; append-only respected; historical bodies unchanged (status lines only updated). ✓
- Faithfulness to ratified + preserved dissent (surface #7): verbs unchanged (`daemon.md` §15.1 lines 660–675); no `SessionConnect` (line 672 explicit); capability `scope`/`rotation` + `per_session_cap` reserved (§7.1 lines 321–326). ✓

### Findings

#### F-baseline-1
- finding_id: F-baseline-1
- relevance: ADJACENT
- key_claim: ADR 0007's "Amended:" status line mis-cites the superseding ADR as **0010** when the parenthetical and the substance ("durable delivery tracking, issue #10") describe **0011**; the typo predates this node but lives in a status line a careful daemon-core reader following the supersession chain will trip on.
- confidence: HIGH
- grounds: `docs/design/DECISIONS.md:279` reads "**Amended:** the "no per-recipient delivery table" clause is superseded by 0010 (durable delivery tracking, issue #10); the rest of this entry stands." ADR 0010 (line 391 onward) is titled "Default message `from` to the locally-held lease via a local holder registry," not a delivery-tracking ADR. ADR 0011 (line 440 onward) is titled "Durable per-recipient delivery tracking for restart-safe backlog delivery" and its own line 444 reads "Supersedes: the 'no per-recipient delivery table' clause of 0007" — bidirectionally proving 0011 is the correct number.
- warrant: The plan's "Definition of done" demands "ADRs 0014–0021 recorded with correct supersedes/relocations/deferrals." This finding is in 0007 (not 0014–0021), so it is technically outside the node's primary scope, but the brief sanctions status-line edits across the log ("only status lines should be updated" for the historical-body ADRs), and a wrong cross-reference inside the decision log is exactly the integration-risk class this final review is supposed to catch — a daemon-core reader who follows 0007 → 0010 lands on the `from`-default ADR, not on delivery tracking, and has to reconstruct the trail.
- rebuttal_conditions: If the typo was intentionally preserved as a known pre-existing wart that the workstream charter excludes from this node, or if there is a separate ADR 0010 amendment I have missed that retroactively explains the citation.
- smallest_change: In `docs/design/DECISIONS.md:279`, change `0010` to `0011`. One character.
- what_gets_smaller: The cross-reference-integrity risk in the decision log — a downstream reader chasing the supersession chain cannot land on the wrong ADR.

#### F-baseline-2
- finding_id: F-baseline-2
- relevance: ADJACENT
- key_claim: The dismissal-path matrix entry for "watch-pid failure" in `daemon.md` §9.3 says "(anchor pid dead OR start-time mismatch)" without re-stating the `any-anchor-suffices` vs `all-required-needed` quantifiers from §9.1; for the v1 floor (single loader anchor) it is unambiguous, but a future required-predicate consumer reading only the matrix could mis-interpret the trigger.
- confidence: MEDIUM
- grounds: `daemon.md:399` row 2 wording is "**watch-pid failure** (anchor pid dead OR start-time mismatch)"; `daemon.md:360–366` §9.1 defines the predicate types as "**`anchor`** — the station is alive if **any** anchor pid survives" and "**`required`** — the station is alive only if **all** required pids survive." The plan (Plan.md:180–187) frames the trigger as "watch-pid failure (anchor pid dead OR start-time mismatch)" — the wording lift is faithful to the plan; the imprecision is inherited.
- warrant: Faithful-to-plan, but in a final-review lens a reader-trip hazard. Not a contract weakening (the §9.1 semantics govern; the matrix is presentational). daemon-core's v1 floor is one anchor + start-time, so this never bites in v1; it is housekeeping for the moment a `required` consumer is added.
- rebuttal_conditions: Reviewer judges the §9.1 cross-anchor is sufficient and matrix concision wins.
- smallest_change: In `daemon.md:399`, append the quantifier — e.g., "**watch-pid failure** (no anchor pid survives, OR any required pid dies, OR any monitored pid's start-time mismatches)." Single-line edit.
- what_gets_smaller: Future-consumer mis-interpretation risk if/when a `required` predicate ships.

#### F-baseline-3
- finding_id: F-baseline-3
- relevance: PARKED
- key_claim: Casing inconsistency between the `WatchPid` struct (`role: Anchor | Required`) and the prose-level predicate names (`anchor` / `required`) in `daemon.md`.
- confidence: HIGH
- grounds: `daemon.md:222` `WatchPid { pid: u32, start_time: u64, role: Anchor | Required }` (TitleCase); `daemon.md:362,364` "**`anchor`**" / "**`required`**" (lowercase, backticks).
- warrant: Purely cosmetic. The document explicitly says shapes are "normative *shapes* (names, fields, types, ordering invariants), not final source" (lines 21–22), so daemon-core picks the actual case. Not a blocker.
- rebuttal_conditions: None worth pursuing — flagging only for completeness as the rapporteur's faithfulness_check may scan for it.
- smallest_change: Optional: align casing or note "case-normalized in source." Skip if blocker-budget is tight.
- what_gets_smaller: Trivial reader friction.

### Examined-passed (surfaces you verified are fine — short list)
- DA-7 ordering invariant present verbatim: "non-`NotOwner` result **before it emits any `Frame::Message`**" (`daemon.md:495–501`) + idempotency `AlreadyDelivered` = no-op no-frame, `NotOwner` = self-demote no-frame.
- DA-1 frozen assertion present verbatim and bolded: "no `Frame::Message` from a non-epoch holder reaches a recipient after the daemon's waiter binds" (`daemon.md:552–554`).
- DA-1 preserved minority (gr-retrospective rotation-alone position) honored explicitly with grounds-for-adopting-two-phase: `daemon.md:556–561`.
- DA-3 wait reconnect-on-EOF grace + auto-Re-register on `UnknownSession`: `daemon.md:167–175` + `:638–648`. ReRegister explicitly idempotent (union of address sets by default) per `:644–647`.
- DA-5 atomic critical section enumerated 1-2-3-4 (mint epoch → evict map → close waiters → bind) with explicit "No observable intermediate state": `daemon.md:430–443`.
- DA-8 epoch-loss drop preserved: "**Reset/drop** the entire fast-path on epoch loss (self-demote, takeover) — its identity is epoch-scoped, so it must not survive a fence" (`daemon.md:582–584`).
- DA-10 invariant preserved: "**no generated divergent copy**" (`daemon.md:687`).
- Q-A: ADR 0019 "Scope." line present (DECISIONS.md:754–758).
- OQ4 empirical grounding: live probe Copilot CLI 1.0.64-1; inner pid not env-exposed + lazily spawned; ppid-walk rejected (daemon.md:374–381).
- Status freeze line correctly limited to "field set + meaning + gating-test observable assertions; not frozen = wire format, ordering, additional diagnostics" (`daemon.md:180–184`).
- Reopen conditions all four preserved (drain-needs-new-IPC, plugin-API-for-per-session-cap, transport-masks-EOF, harness-rejects-skill-pointer): `daemon.md:795–809`.
- Deferred items explicit (full non-binary status policy, fd-over-IPC, daemon-owned directory reads, per_session_cap tiers, #27, #24, #12): `daemon.md:781–793`.
- D1 deviation acknowledged in ADR 0021: "Relocate the design layer… deviates from issue #34's 'keep the design layer at the repo root' — builder-directed during shaping and is flagged for orchestrator reconciliation" (DECISIONS.md:838–841).
- README.md/SKILL.md intentional holder-model narrative deferral honored per ADR 0021 line 832 ("SKILL.md + plugin-doc narrative updates land with `daemon-core`, never mid-workstream") — examined as intentional non-issue.
- All intra-document anchors in `daemon.md` I followed resolve; index.md and DESIGN.md cross-doc links resolve.

### Dissent_or_alignment
Align with a clean **GO** (or **GO-WITH-CHANGES** if any panel member finds a true CORE I missed): the produced design layer faithfully and consistently realizes the plan at the contract strength the prior council required; DA-1..DA-10 + Q-A + the five gating tests + OQ4=no + OQ-γ + the relocations/defers map all land at or above the synthesized smallest_change. My three findings are non-blocking (one pre-existing typo, one wording sharpening, one cosmetic); only F-baseline-1 is worth the one-character fix before the PR. I anticipate gr-premortem and gr-retrospective may surface implementer-trip hazards I have not pressure-tested with their overlays, but baseline-faithfulness/consistency/completeness is satisfied.

---

## gr-premortem — round 1
- agent_id: gr-premortem
- requested_model: gpt-5.5
- provider_family (self-reported): openai
- persona+overlay: general-reviewer + premortem
- epistemic_act: CHALLENGE
- timestamp_utc: 2026-06-23T03:05:41Z

### Premortem scenarios -> defects
1. **Builder double-delivers during ownership loss because the delivery fence is only aspirational.** Passes — `docs/design/daemon.md` §11.3 gives the typed backend method and ordering rule: `mark_delivered_if_current_owner(...) -> Delivered | NotOwner | AlreadyDelivered` (lines 489-493), and the daemon must receive a non-`NotOwner` result before emitting a `Frame::Message` (lines 495-503).
2. **Builder ships a daemon that overlaps a legacy holder during first daemon-aware rollout.** Passes — `daemon.md` §12 makes drain-before-bind/claim normative (lines 541-550) and freezes the assertion that no non-epoch holder frame reaches a recipient after daemon waiter bind (lines 552-553); ADR 0020 repeats the same rule (DECISIONS.md lines 803-808).
3. **Daemon-core stalls choosing how concurrent `ReRegister` updates merge a multi-address session.** Adjacent defect, not CORE — `daemon.md` §14.4 says concurrent waits converge, but then leaves "last-writer-wins on the address set, or union — `daemon-core` picks and freezes one; default union" (lines 644-647). Default union is directionally clear and matches the plan allowance, but the normative doc can remove a choice point.
4. **Crash-respawn treats recovered durable rows as live and delivers from `suspect`.** Passes — `daemon.md` §14.3 says recovered rows start `suspect` and the daemon "MUST NOT heartbeat or deliver" for them (lines 622-624), promotes to `verified` only on Register/ReRegister/Wait proof (lines 625-628), and covers `lapsed` (lines 629-631).
5. **ADR chain breaks and future readers cannot tell what superseded what.** Passes for assigned ADR surfaces — 0004 status points to 0014 (DECISIONS.md line 152) and 0014 has `Supersedes: 0004` / `Amends: 0009` (lines 607-609); 0005, 0010, 0012, 0013 status lines carry their narrowed/superseded mechanisms (lines 201, 394, 482, 524); numbering is clean 0001..0021 (headings lines 49-816); 0019 has the required Scope note (lines 754-758).
6. **Builder follows DESIGN.md and reintroduces live-session TTL teardown despite daemon.md.** Adjacent defect, not CORE — DESIGN.md still describes the v0 backend as "TTL-heartbeat liveness" (lines 191-193, 225-228) without immediately naming ADR 0017's narrowing. `daemon.md` governs mechanisms (lines 5-11) and correctly says no idle teardown / TTL only daemon-down backstop (lines 350-353, 419-423), so this is a clarity tightening rather than a blocker.
7. **Preserved dissent silently disappears via renamed verbs or `SessionConnect`.** Passes — `daemon.md` §15.1 states CLI verbs are unchanged (lines 660-669), protocol operations are not CLI renames, and held-stream `SessionConnect` is not adopted (lines 671-673); ADR 0021 repeats the same (DECISIONS.md lines 826-828).

### Coverage notes (per assigned surface)
- **#3 Correctness of written contracts:** examined `daemon.md` §§7, 9-14, 16-17 plus ADRs 0015/0017/0019/0020. Verdict: **examined-passed with ADJACENT tightening** on `ReRegister` merge wording. Epoch CAS/self-demote, delivery fence ordering, two-phase cutover, suspect/verified/lapsed, admin_cap flow, and Postgres reclaim are present at implementable contract strength (daemon.md lines 453-529, 541-553, 605-647).
- **#5 ADR quality:** examined DECISIONS.md 0001..0021 headings and assigned status lines. Verdict: **examined-passed**. Supersedes/amends are bidirectional where required (0004/0014, 0009/0014), 0005/0010/0012/0013 status lines carry the mechanism changes, new ADRs 0014-0021 are sequential, and 0019 has a Scope note (DECISIONS.md lines 152, 201, 362, 394, 482, 524, 604-843).
- **#7 Faithfulness to ratified + preserved dissent:** examined Plan.md OQ resolutions/definition of done, planning synthesis DA-1..DA-10, daemon.md, DESIGN.md, PRODUCT-THESIS.md, index.md, and ADRs 0014-0021. Verdict: **examined-passed with ADJACENT clarity note** on DESIGN.md TTL phrasing. Verbs are not renamed, no held-stream `SessionConnect`, capability scope/rotation/per-session caps are reserved, and design relocation is flagged (daemon.md lines 321-326, 660-689; DECISIONS.md lines 826-842; index.md lines 37-41).
- **Other surfaces examined:** docs/design entrypoint and product framing. Verdict: **examined-passed** — index.md points to PRODUCT-THESIS, DESIGN, daemon, DECISIONS in the right order (lines 22-27), and PRODUCT-THESIS frames the auto-spawned local exchange without a hosted server (lines 88-93, 123-135).

### Findings
- finding_id: F-premortem-1
- relevance: ADJACENT
- key_claim: `ReRegister`'s idempotency contract is almost sufficient, but the normative text leaves an avoidable builder choice between last-writer-wins and union for the session address set.
- confidence: MEDIUM
- grounds: `docs/design/daemon.md` §14.4 lines 644-647: "`ReRegister` is **idempotent**: concurrent waits for one `session_id` converge to a single map entry (`daemon.md`'s rule: last-writer-wins on the address set, or union — `daemon-core` picks and freezes one; default **union**, so a multi-address session is never narrowed by one re-register)."
- warrant: A design-gate contract should not ask the builder to choose a semantic in the same sentence that claims to be the rule. The stated default union is direction-preserving and prevents narrowing multi-address sessions, so this is not a CORE gap, but tightening it removes a race-semantics decision from daemon-core.
- rebuttal_conditions: Not a finding if the council accepts "default union" as already normative enough, or if the builder's acceptance criteria intentionally own this exact choice.
- smallest_change: Replace the parenthetical with a single rule, e.g. "concurrent waits for one `session_id` converge by unioning address membership; a ReRegister for one address refreshes proof/watch-pids for that address and never narrows other addresses."
- what_gets_smaller: Builder discretion, race ambiguity, and future disagreement about multi-address session narrowing.

- finding_id: F-premortem-2
- relevance: ADJACENT
- key_claim: DESIGN.md's backend-strategy paragraphs still present "TTL-heartbeat liveness" as the v0 baseline without local cross-reference to ADR 0017's narrowing, which can momentarily read as live-session TTL liveness rather than daemon-down/exchange-heartbeat proof.
- confidence: MEDIUM
- grounds: `docs/design/DESIGN.md` lines 191-193: "The portable v0 baseline uses **poll** delivery and **TTL-heartbeat** liveness for *both* SQLite and Postgres"; lines 225-228 repeat that Postgres runs "TTL-heartbeat liveness as SQLite." The governing daemon contract says no idle-TTL teardown and TTL survives only as the daemon-down backstop (`docs/design/daemon.md` lines 350-353, 419-423).
- warrant: The contradiction is not blocking because `daemon.md` explicitly governs mechanisms (daemon.md lines 5-11) and later DESIGN sections describe stale-attendance/takeover. Still, a builder reading DESIGN first could briefly infer that TTL remains the live-session liveness policy.
- rebuttal_conditions: Not a finding if "TTL-heartbeat liveness" is considered unambiguously daemon/exchange liveness after the DESIGN rewrite.
- smallest_change: Add one sentence near DESIGN.md lines 191-193 and/or 225-228: "In the daemon era this TTL heartbeat is the exchange/daemon-down backstop; live-session dismissal is governed by daemon.md's hook + watch-pid + stale-attendance/takeover model, not idle TTL teardown."
- what_gets_smaller: Cross-document ambiguity around 0005 vs 0017 and the chance of reintroducing a superseded live-session TTL interpretation.

### Examined-passed
- Epoch lifecycle is concrete: claim/takeover increments by CAS on observed row, epochs are monotonic, heartbeat/release are owner/epoch guarded and rowcount-returning, and 0-row heartbeat self-demotes (daemon.md lines 453-478).
- Server-side delivery fence is concrete: typed outcome, non-`NotOwner` before frame, and no frame after `NotOwner` or `AlreadyDelivered` (daemon.md lines 489-503).
- Ordered handoff and Postgres reclaim are concrete: quiesce/flush/unbind/claim flow and epoch-based cross-machine race resolution under READ COMMITTED (daemon.md lines 506-529).
- Two-phase legacy cutover is concrete and preserves dissent: drain before daemon waiter bind, then claim epoch=1 + occupant rotation; frozen no-non-epoch-frame-after-bind assertion; minority rotation-alone position preserved with reopen conditions (daemon.md lines 531-561).
- Admin capability flow matches DA-2: instance secret in user-private `daemon.cap`, privileged RPCs carry proof, scope/rotation/per-session cap reserved/deferred, and `DeregisterSession(session_id, admin_cap)` is the v1 hook path (daemon.md lines 309-331, 605-615; DECISIONS.md lines 769-779).
- Liveness/dismissal matrix is concrete: sessionEnd, watch-pid failure, operator takeover, daemon-down TTL are disjoint; watch-pid death triggers internal DeregisterSession and bypasses `occupied_stale` (daemon.md lines 390-405).
- Gating tests cover the required five observables, including crash-during-wait suspect rows, competing daemons, ownership-loss-around-delivery, and intra-daemon takeover local eviction (daemon.md lines 707-730).
- ADR 0019 has the Scope note required by Q-A (DECISIONS.md lines 754-758); ADR 0021 preserves verbs, hides daemon from normal help, single-sources SKILL, and flags docs/design relocation deviation (DECISIONS.md lines 826-842).

### Dissent_or_alignment
Alignment: no CORE blocker found on my assigned surfaces; I recommend GO from this lens with the two adjacent wording tightenings above.

---

## gr-retrospective — round 1
- agent_id: gr-retrospective
- requested_model: gemini-3.1-pro-preview
- provider_family (self-reported): google
- persona+overlay: general-reviewer + retrospective
- epistemic_act: PROPOSE
- timestamp_utc: 2026-06-23T03:05:58Z

### Implementer-on-the-job traces
1. **Trace 1 (Readiness ACK ambiguity):** Implementer reaches the connect-or-spawn loop in daemon.md §2.2 step 3, reads "spawn the daemon, await its readiness ACK, and connect." They look for an out-of-band readiness signal (stdout/pipe) because they are told to await it *before* connecting, but §2.3 implies readiness *is* the ability to connect and complete Hello. They are misled into building a redundant signaling mechanism.
2. **Trace 2 (ReRegister/Wait auth contradiction):** Implementer builds the auth checks based on §7.1 ("Register, ReRegister, Wait need no proof"), then reaches §14.3 and §14.4 which state wait must inherit "admin_cap if needed" and that promotion occurs via "Wait-connect... (+ admin_cap where the operation is privileged)". They are forced to re-decide whether these operations are actually unprivileged, stalling the auth implementation.
3. **Trace 3 (Wait frame session_id omission):** Implementer writes the Wait struct from §6.2, omitting session_id as specified. Later, in §14.3, they must implement suspect row promotion via an "authenticated Wait-connect carrying a valid TELEX_SESSION_ID". Because the frame lacks the ID, they cannot map the waiter to the attendance record. They must unilaterally mutate the contract to add session_id to Wait.
4. **Trace 4 (Takeover eviction overreach):** Implementer writes the Takeover critical section from §10.2 step 3 ("closes the IPC waiters bound under the prior occupant"). They blindly close *all* waiters for that session_id across the exchange, inadvertently dropping the session's other validly attended addresses.

### Coverage notes (per assigned surface)
- **#6 Doc architecture + links:** Examined index.md against the filesystem, and all anchor links in docs/design/*.md against their targets. Passed.
- **#8 New errors introduced in the writing:** Examined daemon.md chronologically as an implementer. Found core contradictions around IPC structures and auth requirements. Core-findings.
- **#9 Node outcome anchor + PR framing:** Examined DECISIONS.md and the 9 deliverables in daemon.md/DESIGN.md. Checked PR framing deviation in ADR 0021. Passed.

### Findings

- finding_id: F-retro-1
- relevance: CORE
- key_claim: The spawn-lock chronological instructions demand an impossible out-of-band readiness ACK before connection.
- confidence: HIGH
- grounds: daemon.md §2.2 step 3: "acquire the spawn-lock, then spawn the daemon, await its readiness ACK, and connect."
- warrant: A client cannot "await its readiness ACK, and connect" if readiness is signaled by completing the Hello handshake on the connection (as implied by §2.3). An implementer will be misled into building a separate stdout/pipe readiness signal.
- rebuttal_conditions: If the daemon actually writes a readiness line to stdout before the client connects, but the spec does not mention this.
- smallest_change: Change step 3 to: "acquire the spawn-lock, then spawn the daemon, connect, and await its readiness ACK (the HelloAck)."
- what_gets_smaller: Removes the implication of an out-of-band readiness signal.

- finding_id: F-retro-2
- relevance: CORE
- key_claim: The crash recovery sections falsely imply ReRegister and Wait might require the dmin_cap, directly contradicting the explicit authorization model.
- confidence: HIGH
- grounds: daemon.md §14.4 requires wait to auto-ReRegister with "admin_cap if needed". §14.3 says promotion occurs via "Register, ReRegister, or an authenticated Wait-connect ... (+ admin_cap where the operation is privileged)". But §7.1 strictly states: "Unprivileged requests (Hello, Register, ReRegister, Wait) need no proof".
- warrant: The implementer must write the structs and auth checks. These "where privileged" hedges on strictly unprivileged operations will cause them to doubt §7.1 and add unnecessary capability checks.
- rebuttal_conditions: None. This is a direct contradiction.
- smallest_change: Remove "(+ admin_cap where the operation is privileged)" from §14.3 and "and admin_cap if needed" from §14.4.
- what_gets_smaller: Eliminates the contradiction and keeps presence verbs cleanly unprivileged.

- finding_id: F-retro-3
- relevance: CORE
- key_claim: The Wait frame definition omits the session_id field required by the crash recovery state machine to promote suspect rows.
- confidence: HIGH
- grounds: daemon.md §14.3 says a suspect row is "promoted by ... an authenticated Wait-connect carrying a valid TELEX_SESSION_ID". But §6.2 defines Wait as { store_key, address, attention?, timeout_ms } (omitting session_id).
- warrant: The daemon cannot map an incoming Wait to a specific session's attendance record without a session_id in the frame. The implementer will get stuck or unilaterally alter the Wait frame.
- rebuttal_conditions: If the daemon infers the session out-of-band, but no such mechanism exists.
- smallest_change: Add session_id to the Wait frame definition in §6.2: Wait { store_key, address, session_id, attention?, timeout_ms }.
- what_gets_smaller: Aligns the frame struct with the required promotion logic.

- finding_id: F-retro-4
- relevance: ADJACENT
- key_claim: The takeover instructions for closing IPC waiters are insufficiently scoped and risk tearing down innocent listeners.
- confidence: MEDIUM
- grounds: daemon.md §10.2 step 3: "closes the IPC waiters bound under the prior occupant"
- warrant: A literal implementer might drop all waiters for the session across all its attended addresses, breaking unrelated listeners.
- rebuttal_conditions: The context implies it's scoped to the rotated address.
- smallest_change: Change to "closes the IPC waiters for the rotated address bound under the prior occupant".
- what_gets_smaller: Removes ambiguity and prevents collateral teardown of unaffected addresses.

### Examined-passed
- **Internal link resolution:** Checked all .md links in docs/design/. Every anchor (e.g., #101-last_confirmed-occupied_stale-and-the-hook-semantics-split-oq2-da-6) resolves perfectly to its slugified heading. SKILL.md correctly points to the root directory.
- **Node outcome anchor:** All 9 deliverables are present and substantive. The 8 open questions are resolved with actionable specifics. ADR 0021 correctly flags the docs/design directory relocation as a deviation from the brief, so it was not silently done.
- **PR framing:** The design layer fully satisfies the plan, and Closes #34 is safe to use once the writing defects are fixed.

### Dissent_or_alignment
I am aligned with the architecture and plan, but I dissent from passing daemon.md as-is because the chronological ambiguity around readiness, the ReRegister auth contradiction, and the missing session_id in the Wait frame will immediately block the daemon-core implementer.

---

# Round 2 (focused interaction — cross-pressure on gr-retro's 3 CORE findings)

## gr-baseline — round 2
- agent_id: gr-baseline
- requested_model: claude-opus-4.7
- provider_family (self-reported): anthropic
- persona+overlay: general-reviewer + baseline
- epistemic_act: REFINE
- timestamp_utc: 2026-06-23T03:11:30Z

### Crux judgments

#### F-retro-1 (readiness ordering, §2.2 step 3)
- crux_id: F-retro-1
- gr-retro relevance: CORE
- my judgment: **ADJACENT** (wording sharpening; spec is self-consistent when §2.2 and §2.3 are read together)
- rationale grounded in cited text: §2.2 step 3 reads "await its **readiness ACK**, and connect" (`daemon.md:101`); §2.3 names the only available observation mechanism — "A connecting client treats 'endpoint exists but Hello does not complete within the readiness window' as not-ready and retries within backoff" (`daemon.md:116–117`). Read in conjunction with §6.1 ("The **first** frame on every connection is a handshake," `daemon.md:257`), the "readiness ACK" *is* successful Hello/HelloAck completion on a (re)connect attempt — there is no separate out-of-band ACK channel in the protocol. The "connect" in step 3 is therefore not a post-ACK action; it is the mechanism *by which* the ACK is observed. §2.3 explicitly governs (it defines what "readiness" means and how a client recognizes it), so a downstream implementer cannot get materially stuck: the only thing they *can* do is poll-connect-and-Hello, exactly as §2.3 prescribes.
- refined smallest_change (if any): Optional one-line clarification at §2.2 step 3: "…spawn the daemon, then poll-connect-Hello until it succeeds within the readiness window (this **is** the readiness ACK; see §2.3)." This is a wording sharpening, not a contract change. I would not block on it.

#### F-retro-2 (auth phrasing in §14.3/§14.4 vs §7.1)
- crux_id: F-retro-2
- gr-retro relevance: CORE
- my judgment: **ADJACENT** (parentheticals are honest hedges; §7.1 governs as the privileged-op authority and is not contradicted)
- rationale grounded in cited text: §7.1 is unambiguous and authoritative: "**Unprivileged** requests (`Hello`, `Register`, `ReRegister`, `Wait`) need no proof" (`daemon.md:315–317`) and "**Privileged** requests (`DeregisterSession`, `Detach`, `Takeover`, `Drain`, `Status detail`) carry `proof = admin_cap`" (`daemon.md:318–320`). §14.3's "authenticated `Wait`-connect carrying a valid `TELEX_SESSION_ID` (+ `admin_cap` **where the operation is privileged**)" (`daemon.md:625–626`) — the parenthetical's scope is the privileged side of the OR-list (none of Register/ReRegister/Wait are privileged per §7.1), so it is correctly vacuous on those three and only "fires" if a future flow mixes a privileged op into the recovery sequence. "Authenticated" here means *identity-bearing* (the TELEX_SESSION_ID is the session-identity token that the in-memory map can check against §14.1), not *capability-bearing*. §14.4's "and `admin_cap` if needed" (`daemon.md:642–643`) is the same honest hedge: for `Wait`/`ReRegister`, "if needed" reduces to "never per §7.1" — but writing it leaves room for future privileged paths in the same flow without rewording. No contract weakening; §7.1's per-op privilege table is the single source of truth.
- refined smallest_change (if any): None required at contract strength. If anything, an editorial pass could replace "authenticated" in §14.3 with "identity-bearing" to dissolve the apparent friction with §7.1's "need no proof," but baseline-faithfulness is satisfied as written.

#### F-retro-3 (Wait frame omits session_id)
- crux_id: F-retro-3
- gr-retro relevance: CORE
- my judgment: **ADJACENT** (mechanism is supplied by §14.4; §14.3 wording is loose but downstream-resolvable) — this is the strongest of the three retro findings and the one closest to my CORE/ADJACENT boundary.
- rationale grounded in cited text: §6.2 defines `Wait { store_key, address, attention?, timeout_ms }` (`daemon.md:280`), no `session_id` field. §14.3 lists three promotion paths: "promoted by a successful `Register`, `ReRegister`, or an authenticated `Wait`-connect carrying a valid `TELEX_SESSION_ID`" (`daemon.md:625–626`). Taken literally and in isolation, the third path is unsatisfiable — a `Wait` frame as defined cannot "carry" `TELEX_SESSION_ID`. **But** §14.4 supplies the actual mechanism: "A `Wait` that returns `UnknownSession` triggers the same `ReRegister` then retries" (`daemon.md:643–644`), with `ReRegister { store_key, address, session_id, watch_pids[] }` (`daemon.md:277`) explicitly carrying `session_id`. Read holistically, §14.3's "Wait-connect carrying TELEX_SESSION_ID" denotes *the Wait flow*, in which the auto-`ReRegister` (path #2) is the actual frame that carries `session_id` and effects the promotion. An implementer who reads §14.3 + §6.2 + §14.4 together cannot get stuck: they will implement (a) `Wait` per §6.2 (no `session_id` field), (b) auto-`ReRegister` on `UnknownSession` per §14.4 (which carries `session_id`), and (c) treat that `ReRegister` as the promotion event per §14.3 path #2. The §14.4 mechanism makes the spec self-consistent.
- refined smallest_change (if any): I agree with gr-retrospective that this is the one place where prose tightening pays for itself; my refinement of their fix:
  - **Refined wording for `daemon.md:625–626`:** "promoted by a successful `Register` or `ReRegister`. A `Wait` that hits `UnknownSession` does **not** promote on its own; it triggers the auto-`ReRegister` per [§14.4](#144-wait-auto-re-register), which is the promoting frame."
  - This removes the literal-but-unsatisfiable third path, explicitly routes Wait-induced promotion through `ReRegister`, and aligns §14.3's enumeration with §6.2's frame set without changing any contract. I would **not** add a `session_id` field to the `Wait` frame — the §14.4 design (lazy auto-ReRegister on `UnknownSession`) is deliberate, idempotent, and avoids paying handshake cost on every Wait.

### Any new finding (only if a fresh CORE surfaces)

None. No fresh CORE surfaced in re-reading §2.2, §2.3, §6.1, §6.2, §7.1, §14.3, §14.4 under pressure. My round-1 F-baseline-1..3 findings remain ADJACENT/PARKED.

### Convergence

After pressure-testing, I align with the panel that the design layer is **GO** at contract strength: §7.1 governs authorization, §2.3 governs readiness, and §14.4 supplies the auto-`ReRegister` mechanism that resolves §14.3's loose enumeration — gr-retrospective's three flags are real reader-trip hazards but all ADJACENT (the strongest, F-retro-3, has the cleanest one-line prose fix in §14.3 that I would recommend folding into the merge but would not block on).

---

## gr-premortem — round 2
- agent_id: gr-premortem
- requested_model: gpt-5.5
- provider_family (self-reported): openai
- persona+overlay: general-reviewer + premortem
- epistemic_act: CHALLENGE
- timestamp_utc: 2026-06-23T03:11:30Z

### Crux judgments (with worst-plausible mis-implementation per finding)
- crux_id: F-retro-1
- gr-retro relevance vs your judgment: gr-retrospective says CORE; gr-premortem judges ADJACENT.
- one-line worst-plausible mis-implementation if shipped as-is: a literal implementer waits for a separate readiness signal before opening the endpoint, so spawn succeeds but connect-or-spawn deadlocks because the only real ACK is reachable through Hello.
- verdict + refined smallest_change: The wording in `daemon.md` §2.2 step 3 is genuinely trap-shaped, because "await its readiness ACK, and connect" orders ACK before connection. But `daemon.md` §2.3 is adjacent and binds readiness to endpoint availability plus Hello completion within the readiness window, while §6.2 lists `Hello`/`HelloAck` and no `ReadinessAck` frame. I do not think a careful daemon-core implementer gets stuck if they read §2.3 and §6.2, but a hurried one could add an unnecessary parent/child ready pipe or wait on a nonexistent pre-connect ACK. Smallest change: rewrite §2.2 step 3 as "acquire the spawn-lock, spawn the daemon, then retry connect + Hello until HelloAck completes within the readiness window" and add in §2.3 that the readiness ACK is `HelloAck`, not an out-of-band signal.

- crux_id: F-retro-2
- gr-retro relevance vs your judgment: gr-retrospective says CORE; gr-premortem judges ADJACENT.
- one-line worst-plausible mis-implementation if shipped as-is: after respawn, daemon-core rejects `Wait`/auto-`ReRegister` promotion unless the waiter carries `admin_cap`, causing live sessions without that credential to age from `suspect` to `lapsed`.
- verdict + refined smallest_change: `daemon.md` §7.1 is explicit that `Hello`, `Register`, `ReRegister`, and `Wait` are unprivileged and need no proof, and §6.2 marks both `ReRegister` and `Wait` as not privileged. Therefore the §14.3 phrase "(+ `admin_cap` where the operation is privileged)" and §14.4 "`admin_cap` if needed" are honest hedges under the wider contract: Wait/ReRegister never need it. The premortem artifact is still plausible because "authenticated Wait-connect" can make a reader over-associate suspect promotion with admin proof. Smallest change: in §14.3 say "valid `TELEX_SESSION_ID`; no `admin_cap` is required for `Wait`/`ReRegister` per §7.1" and in §14.4 replace "and `admin_cap` if needed" with "no `admin_cap` for this unprivileged `ReRegister`; privileged verbs remain governed by §7.1."

- crux_id: F-retro-3
- gr-retro relevance vs your judgment: gr-retrospective says CORE; gr-premortem judges ADJACENT.
- one-line worst-plausible mis-implementation if shipped as-is: daemon-core adds `session_id` to the `Wait` frame and promotes `suspect` rows directly on Wait, creating protocol drift and possibly skipping the watch-pid/session-map rebuild that `ReRegister` is meant to perform.
- verdict + refined smallest_change: The current mechanism is coherent if read across sections: `daemon.md` §6.2 intentionally keeps `Wait { store_key, address, attention?, timeout_ms }` sessionless; §14.4 says the long-lived wait client uses inherited `TELEX_SESSION_ID` to auto-`ReRegister` on reconnect or `UnknownSession`; §14.3's "Wait-connect carrying a valid `TELEX_SESSION_ID`" should therefore be understood as the Wait path routing through that auto-`ReRegister`, not as a missing `Wait` field. If shipped as-is, a careful implementer should choose (b), route §14.3 through auto-`ReRegister`; a hurried implementer might choose (a), add a field. I do not see (c), genuine stuckness, unless they read §14.3 while ignoring §14.4. Smallest change: rewrite §14.3 to "promoted by Register/ReRegister; a Wait reconnect promotes only by first performing the auto-`ReRegister` described in §14.4" and add "the `Wait` IPC frame remains sessionless" to §14.4.

### Any new finding (only if a fresh CORE surfaces under premortem pressure)
No fresh CORE finding surfaced. The three pressure points are real wording hazards, but each is bounded by nearby normative context in `daemon.md` §§2.3, 6.2, 7.1, and 14.4.

### Convergence
I converge toward "ship after small wording fixes" rather than "daemon-core blocked": the worst plausible failures are documentation-induced detours, not unresolved design mechanics.

---

## gr-retrospective — round 2
- agent_id: gr-retrospective
- requested_model: gemini-3.1-pro-preview
- provider_family (self-reported): Google
- persona+overlay: general-reviewer + retrospective
- epistemic_act: CONCEDE
- timestamp_utc: 2026-06-23T03:11:14Z

### Defense / concede per finding

**F-retro-1 (§2.2 step 3 ordering / out-of-band ACK):**
- crux_id: `Spawn_ACK_Mechanism`
- defended_relevance: ADJACENT
- reasoning: While §2.2's "await its readiness ACK, and connect" implies a distinct out-of-band signal prior to connection, §2.3 explicitly defines this readiness ACK as the successful completion of the `Hello` handshake on the bound endpoint, giving the implementer a concrete and non-blocking mechanism to follow.
- refined smallest_change: Update §2.2 to "spawn the daemon, connect, and await its readiness ACK (successful Hello)."

**F-retro-2 (§14.3/§14.4 admin_cap hedges):**
- crux_id: `Admin_Cap_Hedges`
- defended_relevance: ADJACENT
- reasoning: The parenthetical hedges "where the operation is privileged" (§14.3) and "if needed" (§14.4) for `ReRegister`/`Wait` are confusing, but §7.1 provides an unambiguous, tabular truth-table explicitly stating these operations are unprivileged, which an implementer will rightfully treat as the authoritative source.
- refined smallest_change: Remove the parenthetical hedges in §14.3 and §14.4 to perfectly align with the §7.1 table.

**F-retro-3 (Wait frame omits session_id):**
- crux_id: `Wait_Session_ID_Promotion`
- defended_relevance: ADJACENT
- reasoning: §14.3 lists a "Wait-connect carrying a valid TELEX_SESSION_ID" as a promotion path, which contradicts §6.2's `Wait` frame (which lacks `session_id`), but §14.4 supplies the concrete resolution: `Wait` on a `suspect` row returns `UnknownSession`, forcing a `ReRegister` (which *does* carry `session_id`); the spec's state machine thus successfully closes the loop without needing to alter the frame definitions.
- refined smallest_change: Strike "or an authenticated Wait-connect carrying a valid TELEX_SESSION_ID" from §14.3, as the promotion is always technically performed by the resulting `ReRegister` detailed in §14.4.

### Convergence
Under cross-examination and a holistic reading, all three of my initial findings degrade to ADJACENT clarity sharpenings, as the daemon's underlying state machine and explicitly tabulated definitions (§2.3, §7.1, §14.4) successfully disambiguate the localized phrasing errors.

---

# Council closed after round 2 (full convergence: all CORE findings degraded to ADJACENT; no new CORE).


# Faithfulness check (gr-premortem / openai, non-author non-Claude)

FAITHFULNESS_CHECK: SUPPORT — The synthesis faithfully tracks the panel's GO-WITH-CHANGES convergence, gr-retrospective's round-2 concessions to ADJACENT wording fixes, the absence of live dissent, role-card coverage, member-cited references, and self-reported no-fallback provenance.

