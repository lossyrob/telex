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
