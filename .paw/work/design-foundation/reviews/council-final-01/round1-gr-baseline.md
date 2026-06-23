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
