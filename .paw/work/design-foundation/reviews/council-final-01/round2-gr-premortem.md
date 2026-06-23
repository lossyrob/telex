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
