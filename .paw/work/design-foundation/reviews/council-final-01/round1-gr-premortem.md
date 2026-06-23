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
