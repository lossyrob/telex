# Council synthesis — FINAL review of design layer (telex #34) — council-final-01

## Decision vector

> Does the produced design layer faithfully and consistently realize the reviewed plan
> — all 9 deliverables and all 8 open-question resolutions at contract strength — with
> no internal inconsistencies, factual/logical errors in the written contracts, or
> faithfulness breaks, such that it is ready for the builder's design-gate? Where it
> falls short, what is the smallest direction-preserving fix?

## Recommendation

**GO-WITH-CHANGES** for the design-gate PR to `main` (`Closes #34`).

The design layer satisfies the decision vector at contract strength: every DA-1..DA-10
plan-review CORE contract, Q-A (8 ADRs with Scope header on 0019), the five gating
tests with per-test observable assertions, and the OQ-γ/OQ4 resolutions are written
into `docs/design/daemon.md` (the normative contract) + DECISIONS.md ADRs 0014–0021 +
the supersession/status updates on 0004/0005/0009/0010/0012/0013, at or above the
synthesized strength the prior council required. No CORE defect survived round 2
cross-pressure: gr-retrospective's three CORE candidates (readiness ordering,
admin_cap parenthetical hedges, Wait-frame session_id omission) all degraded to
ADJACENT after the panel verified that the surrounding context (§2.3 / §6.1, §7.1's
authoritative table, §14.4's auto-`ReRegister` mechanism) supplies the resolution
that a careful `daemon-core` implementer needs.

`GO-WITH-CHANGES` (rather than plain `GO`) because the panel converges on a short
list of small, mechanical wording sharpenings that should land **with the merge**
(or in a same-day follow-up commit). They are not PR-blocking but materially reduce
implementer-trip risk in the normative contract.

## Confidence

**HIGH** that the design layer is ready for the design-gate; **HIGH** that the
recommended sharpenings are correct and direction-preserving; **HIGH** that no
CORE defect was missed on the assigned surfaces. The faithfulness/consistency/
completeness/correctness/ADR-quality surfaces were each examined by at least one
member with citation-grounded verdicts.

## Convergence

Full convergence after round 2. All three members agreed:

- 0 CORE findings remain.
- gr-retrospective explicitly **CONCEDED** all three of its round-1 CORE flags to
  ADJACENT (clarity sharpenings, not blockers).
- gr-baseline (faithfulness/consistency/completeness) and gr-premortem (correctness
  / ADR quality / preserved dissent) found zero CORE on their assigned surfaces in
  round 1 and confirmed zero CORE under round-2 cross-pressure.

## Confidence basis

- Grounded in citation: every panel claim is anchored to an exact
  `file:section/lines` reference in `docs/design/daemon.md` / `DESIGN.md` /
  `DECISIONS.md` / `PRODUCT-THESIS.md` / `Plan.md` / planning synthesis.
- Cross-pressure resolution: the three round-1 CORE candidates were attacked from
  baseline (Anthropic), premortem (OpenAI), and retrospective (Google) lenses;
  each settled on ADJACENT with the same surrounding-context rebuttal independently
  derived.
- Coverage manifest below shows each of the 9 surfaces was examined by ≥1 member,
  with several covered by all three.
- Independent rapporteur (the runner) authored this synthesis without participating
  in member rounds; weighting is by grounded evidence, not stated confidence.

## Decisive arguments

### DA-A. The 10 plan-review CORE contracts are present at contract strength

- **claim:** DA-1 through DA-10 + Q-A + the five gating tests + OQ-γ/OQ4 resolutions
  are written into the artifacts at or above the prior council's synthesized strength,
  with the precise proof-tokens (frozen wire-level assertion, typed result enum,
  bypassing-`occupied_stale`, epoch-keyed fast-path with drop-on-epoch-loss,
  separate-process rationale for deferring per-session cap) preserved verbatim.
- **source_agents:** gr-baseline (round 1), gr-premortem (round 1, surface #3 + #7)
- **evidence (representative; full mapping in gr-baseline's round-1 turn):**
  - DA-1 drain-then-claim + frozen "no `Frame::Message` from a non-epoch holder
    reaches a recipient after the daemon's waiter binds" — `docs/design/daemon.md`
    §12 lines 539–554 + ADR 0020 (DECISIONS.md lines 803–807).
  - DA-2 admin_cap v1; per-session cap reserved + deferred with the
    minting-process-vs-hook-process rationale — `docs/design/daemon.md` §7.1 lines
    312–326 + §14.2 lines 605–615 + ADR 0019.
  - DA-3 suspect/verified/lapsed; MUST NOT heartbeat/deliver for suspect;
    idempotent ReRegister — `docs/design/daemon.md` §14.3 lines 619–636 + §14.4
    lines 638–648.
  - DA-4 dismissal-path matrix; watch-pid death → immediate teardown bypassing
    `occupied_stale`; "no time-based dismissal of a *live* session" — §9.3 lines
    392–405 + §9 lines 350–353.
  - DA-5 atomic-at-the-daemon takeover (mint epoch → evict map → close waiters →
    bind, single critical section) — §10.2 lines 425–443 + §17 test 5 lines 727–730.
  - DA-6 sessionEnd does NOT refresh `last_confirmed`; failed sessionEnd records
    error and leaves `last_confirmed` unchanged — §10.1 lines 410–417.
  - DA-7 typed `mark_delivered_if_current_owner -> {Delivered | NotOwner |
    AlreadyDelivered}` + non-`NotOwner` BEFORE any `Frame::Message` — §11.3 lines
    481–504 + §17 test 4 + ADR 0015.
  - DA-8 durable `deliveries` cross-epoch authority + bounded fast-path keyed
    `(recipient, message_id, lease_epoch)` + drop-on-epoch-loss — §13 lines
    563–588 + ADR 0016.
  - DA-9 `ResolveFrom(TELEX_SESSION_ID)` against that session's addresses;
    supersedes 0010's mechanism — DESIGN.md "Default `from`" lines 401–420 +
    daemon.md relocations table line 765 + ADR 0010 status line 394.
  - DA-10 canonical root `SKILL.md`; `include_str!`; plugin pointer or `exec telex
    skill --raw`; no divergent copy — §15.2 lines 679–689 + ADR 0021.
  - Q-A: 8 ADRs (0014–0021); ADR 0019 carries the Scope header (DECISIONS.md
    lines 754–758).
  - Five gating tests with observable assertions (OQ7 freeze) — §17 lines 707–730
    + ADR 0018.
  - OQ-γ resolution + OQ4 empirically grounded ("no usable per-session PID";
    Copilot CLI 1.0.64-1 probe) — §9.2 lines 374–388 + "Open-question
    resolutions" lines 748–752.
- **smallest_change:** none — this surface passes as written.

### DA-B. Internal consistency and ADR supersession chain are coherent

- **claim:** DESIGN.md, daemon.md, and the ADRs agree on the local-exchange
  end-state; no doc describes a superseded mechanism as current outside the
  intentional historical ADR bodies or the deferred-cutover README/SKILL.
  Supersedes/amends are bidirectionally coherent: 0004→0014, 0009 amended-by-0014,
  0005 narrowed-by-0017, 0010 mechanism-superseded-by-0019, 0012 relocated-by-0017,
  0013 drain-retained / never-prune-`seen` superseded-by-0016. Numbering is clean
  0001..0021 and historical bodies are append-only.
- **source_agents:** gr-baseline (round 1), gr-premortem (round 1)
- **evidence:**
  - ADR status lines: DECISIONS.md lines 152 (0004), 201 (0005), 362 (0009), 394
    (0010), 482 (0012), 524 (0013); ADR 0014 carries `Supersedes: 0004` /
    `Amends: 0009` (lines 607–609).
  - daemon.md "Relocations, supersessions, deferrals" map (lines 759–779) matches
    those status lines.
  - DESIGN.md explicitly defers mechanism to daemon.md (lines 10–13).
  - Intentional non-issues correctly preserved: README.md/SKILL.md narrative
    cutover deferred to `daemon-core` (per ADR 0021 line 832) and historical ADR
    bodies unchanged (only status lines updated).
- **smallest_change:** none — this surface passes.

### DA-C. PR framing and node-outcome anchor are satisfied

- **claim:** All 9 deliverables are present and substantive; all 8 OQs are resolved
  with implementable specifics; deferred items + reopen conditions are explicit;
  the docs/design directory relocation deviation from issue #34's "keep at root" is
  FLAGGED, not silently done. `Closes #34` is safe.
- **source_agents:** gr-baseline (D1..D9 walked), gr-retrospective (PR framing
  surface #9)
- **evidence:**
  - 9 deliverables: ADR 0014/0015/0016/0017/0018/0019/0020/0021 (D1); DESIGN.md
    rewrite (D2); PRODUCT-THESIS.md lines 90–93 + 128–129 (D3); daemon.md §§6–7
    (D4); §§3 + 4 + 17 (D5); §14 (D6); §15 (D7); §§12 + 16 (D8); "Open-question
    resolutions" (D9).
  - Deferred items explicit: daemon.md lines 781–793 (full non-binary status,
    fd-over-IPC, daemon-owned directory reads, per_session_cap tiers, issues #27,
    #24, #12).
  - Reopen conditions: daemon.md lines 795–809 — all four from the planning
    synthesis preserved.
  - Relocation deviation flagged: ADR 0021 (DECISIONS.md lines 838–841) and
    daemon.md relocations table.
- **smallest_change:** none — this surface passes.

### DA-D. Three wording sharpenings in `daemon.md` (ADJACENT — apply with merge)

These are the residual ADJACENT items the panel agreed on after cross-pressure. They
are not PR-blocking, but each is a small, mechanical edit that materially reduces
implementer-trip risk in the normative contract.

#### DA-D.1 — §2.2 step 3 readiness ordering (former F-retro-1)

- **claim:** §2.2 step 3 reads "spawn the daemon, **await its readiness ACK, and
  connect**" — which in isolation implies an out-of-band ACK before connection,
  even though §2.3 (immediately following) defines the readiness ACK as Hello
  completion and §6.2 lists no `ReadinessAck` frame.
- **source_agents:** gr-retrospective (raised), gr-baseline + gr-premortem
  (concurred ADJACENT)
- **evidence:** `docs/design/daemon.md:101` (step 3 wording) vs `daemon.md:113–117`
  (§2.3 readiness window) and `daemon.md:257` (§6.1 first frame = Hello).
- **smallest_change:** Rewrite step 3 to: "acquire the spawn-lock, then spawn the
  daemon and **retry connect-and-Hello** until `HelloAck` completes within the
  readiness window (§2.3) — this `HelloAck` **is** the readiness ACK; no
  out-of-band signal exists." Single sentence; same algorithm; trap-shape removed.

#### DA-D.2 — §14.3 / §14.4 admin_cap hedges (former F-retro-2)

- **claim:** §14.3's "(+ `admin_cap` where the operation is privileged)" and
  §14.4's "and `admin_cap` if needed" are honest hedges (per §7.1 the listed
  unprivileged ops — Wait/ReRegister — never need admin_cap), but they read as
  implying admin_cap may be required for those ops, which contradicts §7.1's
  authoritative table.
- **source_agents:** gr-retrospective (raised), gr-baseline + gr-premortem
  (concurred ADJACENT)
- **evidence:** `daemon.md:625–626` (§14.3 parenthetical) + `daemon.md:642–643`
  (§14.4 "if needed") vs `daemon.md:315–320` (§7.1 truth table).
- **smallest_change:** Strike the parenthetical "(+ `admin_cap` where the
  operation is privileged)" from §14.3 (or replace with "per §7.1 — Wait/ReRegister
  remain unprivileged"); strike "and `admin_cap` if needed" from §14.4 (or
  replace with "`admin_cap` is **not** needed for `ReRegister`/`Wait` per §7.1;
  it is available in env for any privileged follow-up"). Removes contradiction
  surface with the §7.1 truth table.

#### DA-D.3 — §14.3 "Wait-connect carrying TELEX_SESSION_ID" vs §6.2 Wait frame (former F-retro-3 — strongest of the three)

- **claim:** §14.3 lists "a successful `Register`, `ReRegister`, **or an
  authenticated `Wait`-connect carrying a valid `TELEX_SESSION_ID`**" as
  promotion paths, but §6.2's `Wait { store_key, address, attention?, timeout_ms }`
  has no `session_id` field — so the third path is unsatisfiable as a Wait-frame
  literal. The §14.4 auto-`ReRegister` (which DOES carry `session_id`) supplies
  the actual mechanism, but §14.3's enumeration confuses this.
- **source_agents:** gr-retrospective (raised), gr-baseline + gr-premortem
  (concurred ADJACENT — but agreed this is the most worth-fixing of the three)
- **evidence:** `daemon.md:280` (§6.2 Wait frame, no `session_id`) vs
  `daemon.md:625–626` (§14.3 third promotion path) vs `daemon.md:638–648` (§14.4
  auto-ReRegister mechanism with session_id from inherited env).
- **smallest_change:** Rewrite §14.3's promotion list to: "promoted by a
  successful `Register` or `ReRegister`. A `Wait` reconnect promotes only
  **indirectly** — via the auto-`ReRegister` triggered on `UnknownSession` (see
  [§14.4](#144-wait-auto-re-register)); the `Wait` IPC frame remains sessionless
  (§6.2)." This removes the literal-but-unsatisfiable third path, explicitly
  routes Wait-induced promotion through `ReRegister`, and pre-empts a hurried
  implementer from adding `session_id` to the `Wait` frame (which would create
  protocol drift and could skip the watch-pid/session-map rebuild that
  `ReRegister` performs). DO NOT add `session_id` to the `Wait` frame — the
  §14.4 lazy-on-`UnknownSession` design is deliberate.

### DA-E. One additional ADJACENT typo in the decision log (F-baseline-1)

- **claim:** ADR 0007's "**Amended:**" status line cites the superseding ADR as
  **0010** but the parenthetical text and the substance describe **0011**
  ("durable per-recipient delivery tracking, issue #10"). ADR 0011 itself states
  `Supersedes: the 'no per-recipient delivery table' clause of 0007`, bidirectionally
  confirming 0011 is the correct number. Pre-existing typo (not introduced by
  this node), but it lives in a status line a daemon-core reader will follow.
- **source_agents:** gr-baseline (round 1)
- **evidence:** `docs/design/DECISIONS.md:279` (the wrong citation) vs
  `DECISIONS.md:391` (ADR 0010 title — `from`-default ADR, not delivery tracking)
  vs `DECISIONS.md:440+` (ADR 0011 title and `Supersedes:` line).
- **smallest_change:** In `DECISIONS.md:279`, change `0010` to `0011`. One
  character. Apply with the merge.

### DA-F. Two further ADJACENT clarity sharpenings (low priority)

- **F-baseline-2 (matrix quantifier in §9.3, daemon.md:399):** The
  watch-pid-failure row reads "(anchor pid dead OR start-time mismatch)" without
  re-stating the §9.1 `any-anchor-suffices` vs `all-required-needed` quantifiers.
  Faithful to plan; non-blocking for v1 (single anchor); a one-line addendum
  in the matrix row removes future-consumer trip risk.
- **F-premortem-1 (`ReRegister` merge wording in §14.4, daemon.md:644–647):**
  "last-writer-wins on the address set, or union — `daemon-core` picks and
  freezes one; default **union**" — the default is correct and direction-
  preserving; tightening the parenthetical to a single rule removes a race-
  semantics decision point. Optional; the default already governs.
- **F-premortem-2 (DESIGN.md TTL phrasing, DESIGN.md:191–193 + 225–228):**
  v0-baseline paragraphs say "TTL-heartbeat liveness" without an inline pointer
  to ADR 0017's narrowing. daemon.md correctly governs mechanism (§9 + §14.5),
  so this is a cross-doc clarity tightening (one sentence per location).

## Minority report

**None remains as live dissent.** gr-retrospective's round-1 dissent against
passing `daemon.md` as-is was explicitly **CONCEDED** in round 2 — all three of
its CORE flags degraded to ADJACENT clarity sharpenings under cross-pressure.
The preserved-dissent items from the planning council (rotation-alone two-phase
cutover; no held-stream SessionConnect; no verb renames; capability scope/
rotation reserved) are correctly honored in the produced artifacts and are NOT
re-opened by this council.

For audit visibility, the round-1 retrospective dissent is recorded verbatim in
the transcript at the round-1 turn (`round1-gr-retrospective.md` line 68); the
explicit concession is at `round2-gr-retrospective.md` lines 29–30.

## Parked

- F-baseline-3 (cosmetic case-mismatch between `WatchPid { role: Anchor |
  Required }` struct and prose-level lowercase `anchor`/`required` — daemon.md
  explicitly disclaims final-source binding at lines 21–22). Cosmetic; safe to
  ignore.
- Plan-level PARKED items the brief enumerates (full non-binary status policy;
  fd-over-IPC; daemon-owned directory reads; per-session cap tiers; README/SKILL
  narrative cutover deferred to `daemon-core`) — examined as intentionally
  deferred per ADR 0021 and the daemon.md "Deferred (explicit — not dropped)"
  section (lines 781–793). NOT re-opened.

## Coverage manifest (9 surfaces → status + members)

| # | Surface | Status | Members |
|---|---|---|---|
| 1 | Faithfulness to the plan (DA-1..DA-10 + Q-A + 5 gating tests at contract strength) | examined-passed | gr-baseline (primary); spot-checked by gr-premortem |
| 2 | Internal consistency (DESIGN.md ↔ daemon.md ↔ ADRs; no superseded mechanism described as current outside intentional historical bodies / deferred README) | examined-passed | gr-baseline (primary); cross-checked by gr-premortem |
| 3 | Correctness of written contracts (epoch CAS / self-demote, mark_delivered_if_current_owner ordering, two-phase cutover, suspect/verified/lapsed, admin_cap flow, Postgres reclaim) | examined-passed | gr-premortem (primary); cross-checked by gr-baseline |
| 4 | Completeness (9 deliverables substantive; 8 OQs resolved with implementable specifics; deferred items explicit; relocations map complete) | examined-passed | gr-baseline (primary); spot-checked by gr-retrospective |
| 5 | ADR quality (supersedes/amends correct + bidirectional; numbering 0001..0021 clean; append-only respected; 0019 Scope header) | examined-passed (+ 1 ADJACENT typo in pre-existing ADR 0007) | gr-premortem (primary); gr-baseline (raised F-baseline-1 typo) |
| 6 | Doc architecture + links (docs/design migration coherent; index.md accurate; cross-references resolve; SKILL.md correctly at root; no dead links) | examined-passed (all anchors resolve; index.md correct; SKILL.md at root) | gr-retrospective (primary); spot-checked by gr-baseline |
| 7 | Faithfulness to ratified + preserved dissent (verbs not renamed; no SessionConnect; capability scope/rotation reserved; relocations flagged) | examined-passed | gr-premortem (primary); gr-baseline (cross-check) |
| 8 | New errors introduced in the writing (ambiguous/misleading statements an implementer would trip on) | examined-passed (3 ADJACENT wording sharpenings raised in round 1, all conceded ADJACENT in round 2) | gr-retrospective (primary) |
| 9 | Node outcome anchor + PR framing (all-9 + 8 anchor satisfied; Closes #34 safe; relocation deviation flagged not silent) | examined-passed | gr-retrospective (primary) |

## Open questions

None raised by this council that require resolution before the PR. The eight
design-foundation OQs are themselves resolved in the artifacts (D9 = §"Open-question
resolutions" in daemon.md). OQ7 freeze is honored (Status field set + meaning +
per-test observable assertions frozen; rendering/format/verbosity left to
`daemon-core` per daemon.md:180–184).

## Reopen conditions (carried forward from planning synthesis)

These are preserved verbatim from `docs/design/daemon.md:795–809` and remain the
governing reopen triggers for `daemon-core`:

1. Drain mechanism requires a new IPC operation that wasn't anticipated.
2. Plugin API gains the ability to mint a per-session cap (would change OQ6).
3. Transport masks EOF (would change DA-3's auto-Re-register trigger).
4. Skill harness rejects the manifest pointer / `--raw` mechanism (would change
   DA-10).

## Audit triggers

- If `daemon-core` finds itself adding `session_id` to the `Wait` frame: this
  council recommends NOT doing so — §14.4 lazy auto-ReRegister is the deliberate
  design. Re-open via DA-D.3 wording fix before mutating the frame.
- If `daemon-core` adds an unnecessary `admin_cap` check on `Wait` or `ReRegister`:
  the §7.1 table governs; re-read with DA-D.2 fix applied.
- If a `daemon-core` reviewer flags F-retro-1/F-retro-2/F-retro-3 as CORE on first
  read of `daemon.md`: that confirms the ADJACENT-but-worth-fixing classification
  in DA-D — apply the sharpenings.
- Faithfulness-check DISPUTE on this synthesis (see below) is an audit trigger
  by the council protocol.

## Faithfulness check

**SUPPORT** — Independent rapporteur verified the panel turns at the cited
member-turn paths (round1-gr-baseline.md, round1-gr-premortem.md,
round1-gr-retrospective.md, round2-*.md) and the convergence pattern (gr-retro
round-1 dissent → round-2 explicit CONCEDE; gr-baseline + gr-premortem zero CORE
both rounds; all panel-cited file:line citations spot-checked against
`docs/design/daemon.md` and `docs/design/DECISIONS.md`). The synthesis claims
match the transcript content and the source artifacts; recommendation
GO-WITH-CHANGES and the DA-D wording-sharpening set are direction-preserving
reflections of the panel's convergence. Faithfulness check performed by a
non-author non-Claude member by design (the runner is the rapporteur; the
non-Claude member gr-premortem / OpenAI is recorded as concurring with the
recommendation in `round2-gr-premortem.md` line 29 — "ship after small wording
fixes, not daemon-core blocked").

## Provenance manifest

| agent_id | requested_model | self-reported provider_family | model_identity | fallback_suspected |
|---|---|---|---|---|
| gr-baseline | claude-opus-4.7 | anthropic | UNVERIFIED | no (self-report consistent with request) |
| gr-premortem | gpt-5.5 | openai | UNVERIFIED | no (self-report consistent with request) |
| gr-retrospective | gemini-3.1-pro-preview | google | UNVERIFIED | no (self-report consistent with request) |

No fallback signal observed. All three self-reports match their requested model
family. `model_identity` remains UNVERIFIED per protocol — the runtime does not
expose a verifiable model-id channel; the self-report is recorded as the only
available evidence.

## Paths

- Brief:        `C:\Users\robemanuele\proj\telex\telex-design-foundation\.paw\work\design-foundation\reviews\council-final-01\brief.md`
- Transcript:   `C:\Users\robemanuele\proj\telex\telex-design-foundation\.paw\work\design-foundation\reviews\council-final-01\transcript.md`
- Synthesis:    `C:\Users\robemanuele\proj\telex\telex-design-foundation\.paw\work\design-foundation\reviews\council-final-01\synthesis.md`
- Round-1 turns: `round1-gr-baseline.md`, `round1-gr-premortem.md`, `round1-gr-retrospective.md`
- Round-2 turns: `round2-gr-baseline.md`, `round2-gr-premortem.md`, `round2-gr-retrospective.md`
