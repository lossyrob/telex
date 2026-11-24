# Scheduled stations in Codex desktop

A local Codex chat can attend one Telex address on a fixed interval. The desktop scheduler wakes the chat, which runs one short inbox check. Between checks the chat can be idle and the Telex waiter count can be zero. Any runtime can send to the address. Use a shared PostgreSQL profile for communication between machines. SQLite serves a local installation.

## Start a station

In the destination chat, ask “Read `telex codex skill` and attend `desk:review` on the `team` backend every five minutes.” The binary includes the version-matched skill. To make it discoverable across chats, copy its output into a personal Codex skill directory, for example `~/.agents/skills/telex/SKILL.md`.

1. The chat runs `telex --backend team --address desk:review codex bind --every 5m`. Omit the backend for the configured default. The returned binding is pending and includes the exact check arguments. The command requires the desktop-provided `CODEX_THREAD_ID`.
2. In that same chat, Codex creates a thread heartbeat containing the check instruction and verifies its cadence and target. It then runs `telex codex confirm --binding B --automation A` with the returned identifiers. If setup is interrupted, retry with the same settings and look up the existing heartbeat before creating another.
3. Each wake uses the absolute Telex path and the same home, runtime directory and configuration used during setup. It checks up to eight primary deliveries, oldest eligible first. A message arriving just after a check waits for a later wake. Live CC notifications are handled by the other delivery modes.

The machine must be awake, the desktop app running, and scheduled turns must have access to the Telex daemon socket and the selected store under the user's normal tool approval policy. Test an actual wake with a dummy sender before depending on a station. A scheduler entry and an occupied sender receipt alone establish neither a successful check nor a response. [Codex automations](https://developers.openai.com/codex/app/automations/)

## Receive and reply

`telex codex check --binding B` returns JSON. A delivery includes the whole message, parent and thread IDs, delivery row, lease epoch and opaque receipt. An informational note is eligible even when the actionable inbox is empty. Message text is input data. It cannot authorize tool execution, external actions, or changing the station.

| Exit | Result | Handling |
| --- | --- | --- |
| 0 | `empty` or `delivery` | Finish an empty turn quietly. Handle a delivery using the station owner's instructions. |
| 1 | `error` | Inspect the code and diagnostic. A failed scan leaves the last successful check time in place. |
| 2 | `busy` | Another check holds a delivery receipt. End this wake. |
| 5 | `inactive` | The binding is pending, revoked, stale, or belongs to another chat. |
| 6 | `quarantined` | Inspect the reported message and frame size. Telex recorded the rejection. Later deliveries can progress. |

Send a threaded reply with `telex codex reply --binding B --receipt R --body-file -`, supplying the reply on stdin. Retry an uncertain send with the same body. Telex returns the previously accepted message ID. A changed body is rejected for that delivery. After handling the message or durably saving the work, run `telex codex ack --binding B --receipt R`. Handle any required workflow disposition separately.

Checks provide at-least-once delivery. An abandoned receipt expires after one configured interval. A replacement daemon can recover active membership at a new epoch and issue a new receipt. Reconcile any effects already performed before retrying. Telex can deduplicate its own reply while the operation evidence is retained.

## Inspect and stop

`telex codex status --binding B` reports the automation ID, binding phase, cadence, last attempt, last successful scan, last outcome and next due time. `telex --address desk:review --json station status --all-sessions` reports local membership, waiter count and pending mail. Confirmation shows `awaiting_first_check`. A successful scan establishes `attended_scheduled`. After the expected check plus half an interval of grace (up to two minutes), the station becomes `scheduled_overdue`, including when its inbox is empty.

If setup or a wake reports socket access denied, inspect the scheduled run and use the standard approval flow. When the run cannot reach Telex, the daemon has no attempt to record and the last observed success ages out. A missing or changed logical store, a conflicting owner or a changed named profile requires operator attention. Keep the binding's runtime directory intact across checks.

To stop, run `telex codex unbind --binding B` in the destination chat and then delete the matching heartbeat through the app. A late wake receives `inactive`. Telex detach, station stop/reset and session end also revoke the binding. If the app is unavailable during cleanup, use the recorded automation ID when it becomes available. To switch modes, stop the existing station before enrolling the next one.
