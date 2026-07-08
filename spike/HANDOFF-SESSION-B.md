# Telex Spike — Handoff for Session B (lag-hunt edition)

You are **Session B** in a live test of the Telex spike: two independent Copilot CLI
sessions exchanging messages through a shared, Postgres-backed message fabric. The
other participant (**Session A**) is already running and waiting for you.

This run has one job beyond "does it work": **measure where the lag comes from.** We
run the increment game **twice** — once with **poll** delivery, once with
**LISTEN/NOTIFY push** delivery — and collect timestamps at every hop so A can
decompose the end-to-end latency.

## Your identity

- **Your address:** `workstream:spike/session:B`
- **Your local holder port:** `47702`
- **Peer (Session A) address:** `workstream:spike/session:A`
- Work from: `cd <repo>\spike`

## Prerequisites (already true on this machine)

- `az` is logged in as a principal with access to the target server. The scripts cache the Entra
  token in TEMP, so only the first call pays the fetch cost.
- The spike binaries are already built at `...\spike\target\debug`.

## Process pattern (match exactly)

Run **both** the holder and the waiter as **attached background tasks** (Copilot
CLI: `mode="async"`, `detach: false`) so they die with your session. Do **not**
detach them and do **not** run the waiter as a blocking foreground command.

- **Holder** = long-lived attached background task. Start once per round; it never
  exits on its own.
- **Waiter** = attached background task that **exits when one message arrives**. On
  exit your runtime notifies you; you wake, capture timing, reply, and **restart the
  waiter**.

## CRITICAL — timing capture on every receive

When you are notified that the waiter task completed, do these IN THIS ORDER:

1. **FIRST**, before anything else, run `.\nowms.ps1` and record the number as
   `T_wake`. (Doing this first measures how long the notification took to wake you.)
2. Read the waiter task's stdout — a single JSON line like:
   `{"id":N,"body":"...","sent_at_ms":...,"buffered_at_ms":...,"waiter_exit_ms":E,"backend_ms":..,"holder_to_exit_ms":..,"send_to_exit_ms":..}`
3. Compute `agent_wake_ms = T_wake - waiter_exit_ms`.
4. **Record** the full JSON line plus your computed `agent_wake_ms` (label which
   round and which message). You will paste these to your human at the end.
5. Then reply (next section). Note the `[send] total ...ms` line that `send.ps1`
   prints — record that too.

## ROUND 1 — POLL

1. Start your holder as an attached background task (no `-Push`):
   ```powershell
   cd <repo>\spike
   .\attach.ps1 -Address "workstream:spike/session:B" -Port 47702
   ```
   Confirm `[holder] ... push=false` and `listening on 127.0.0.1:47702`.

2. Send the opener to A:
   ```powershell
   .\send.ps1 -To "workstream:spike/session:A" -Body "R1 B: 1" -Attention interrupt
   ```

3. Start the waiter as an attached background task:
   ```powershell
   .\recv.ps1 -Address "workstream:spike/session:B" -Port 47702
   ```

4. When notified, do the **timing capture** above. Read the number A sent in `body`,
   add 1, and reply:
   ```powershell
   .\send.ps1 -To "workstream:spike/session:A" -Body "R1 B: <number+1>" -Attention interrupt
   ```
   Then restart the waiter (step 3) for the next message.

5. Repeat for **3 received messages** in round 1. On A's 3rd reply the body will
   contain **`SWITCH-TO-PUSH`**. When you see that, go to Round 2.

## ROUND 2 — PUSH

1. **Stop your round-1 holder** (stop the attached holder background task), then
   start a new holder **with `-Push`**:
   ```powershell
   .\attach.ps1 -Address "workstream:spike/session:B" -Port 47702 -Push
   ```
   Confirm `[holder] ... push=true` and `push enabled (LISTEN telex_messages)`.

2. Tell A you're ready and open round 2:
   ```powershell
   .\send.ps1 -To "workstream:spike/session:A" -Body "R2 B: 100" -Attention interrupt
   ```

3. Run the same **waiter → timing capture → increment reply → restart** loop, using
   body `"R2 B: <number+1>"`, for **3 received messages**.

4. On A's 3rd round-2 reply the body will contain **`SIGN-OFF`**. When you see it,
   send one last message and stop:
   ```powershell
   .\send.ps1 -To "workstream:spike/session:A" -Body "R2 B: done, signing off" -Attention interrupt
   ```

## What to report back to your human

Paste, grouped by round:

- every received **JSON line** (they contain `backend_ms`, `holder_to_exit_ms`,
  `send_to_exit_ms`);
- your computed **`agent_wake_ms`** for each;
- the **`[send] total ...ms`** value for each send;
- the **`[env] token:`** line the first time it appeared (fetch cost) vs. later
  (cached).

Then one sentence: did messages flow both ways in both rounds, and did push feel
faster than poll?

## Notes

- Start the holder before sending; keep it running for the whole round.
- If `recv` exits with code 2 (idle timeout) before A replies, just restart it.
- All timestamps are epoch milliseconds on this one machine, so they are directly
  comparable across sessions.
