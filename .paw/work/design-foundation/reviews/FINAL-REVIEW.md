# Final Review — local-daemon / design-foundation (telex #34)

- **Mode:** society-of-thought via the council skill (builder-preferred engine).
- **Roster:** 3 general-reviewer members, full provider diversity, nested clean —
  gr-premortem (`gpt-5.5`, self-reported OpenAI), gr-retrospective
  (`gemini-3.1-pro-preview`, Google), gr-baseline (`claude-opus-4.7`, Anthropic).
- **Interaction:** deliberate / medium; 2 rounds; full convergence.
- **Verdict:** **GO-WITH-CHANGES (HIGH confidence).**
- **Artifacts:** `reviews/council-final-01/{brief,transcript,synthesis}.md`.

## Outcome

**0 CORE findings** after cross-pressure. Every round-1 CORE candidate (all from
gr-retrospective) was **conceded to ADJACENT** in round 2; gr-baseline and gr-premortem
found 0 CORE on their surfaces both rounds. No live minority dissent. Faithfulness check:
**SUPPORT** by gr-premortem (non-author, non-Claude).

The council confirmed: all 10 plan-review CORE contracts (DA-1..DA-10) are present at
contract strength; the node outcome anchor (9 deliverables + 8 OQ resolutions) is
satisfied; the ADR supersession chain is coherent; the intentional non-issues
(README/SKILL deferred cutover, append-only historical ADR bodies, builder-directed
docs/design relocation) are correctly handled.

## ADJACENT findings — all applied to the diff

All seven were cheap, cross-doc-consistency/clarity improvements that reduce ambiguity
for the downstream `daemon-core` implementer. Applied:

1. **DA-D.1** — `daemon.md` §2.2 step 3: rewrote the readiness-ACK ordering so it reads as
   "retry connect-and-Hello until `HelloAck`" (the `HelloAck` *is* the readiness ACK; no
   out-of-band signal), removing a trap-shape vs §2.3/§6.2.
2. **DA-D.3** — `daemon.md` §14.3: removed the literal-but-unsatisfiable "authenticated
   `Wait`-connect carrying `TELEX_SESSION_ID`" promotion path (the §6.2 `Wait` frame is
   sessionless); routed Wait-induced promotion explicitly through the §14.4 auto-`ReRegister`.
3. **DA-D.2** — `daemon.md` §14.3/§14.4: struck the `admin_cap`-for-unprivileged-ops
   hedges that contradicted the §7.1 truth table.
4. **DA-E** — `DECISIONS.md`:279 (pre-existing): ADR 0007's "Amended" line cited 0010;
   corrected to **0011** (durable delivery tracking). Tightly coupled to the ADR series
   this node extends.
5. **F-baseline-2** — `daemon.md` §9.3: the watch-pid-failure row now re-states the §9.1
   anchor/required quantifiers.
6. **F-premortem-1** — `daemon.md` §14.4: tightened the `ReRegister` merge rule to **union**
   (default), removing the "last-writer-wins or union" ambiguity.
7. **F-premortem-2** — `DESIGN.md` §v0-baseline: added an inline pointer to ADR 0017's
   narrowing of TTL-heartbeat to the daemon-down backstop.

Post-fix: all markdown links/anchors re-validated (0 dead). No production code touched.

## Reopen conditions (carried to the field report)

- If `daemon-core` adds `session_id` to the `Wait` frame: the §14.4 lazy auto-`ReRegister`
  is the deliberate design — re-read with the DA-D.3 fix applied first.
- If `daemon-core` adds an `admin_cap` check on `Wait`/`ReRegister`: the §7.1 table governs
  (DA-D.2 fix applied).
- (Plus the four design-level reopen conditions from the planning council, recorded in
  `daemon.md` "Reopen conditions".)
