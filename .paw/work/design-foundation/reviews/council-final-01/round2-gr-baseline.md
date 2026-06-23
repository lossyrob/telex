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
