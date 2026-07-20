# Live demonstration summary

The Windows demonstration used a real Copilot operator-agent session, the Tauri
Station, and an independently attached worker on one isolated SQLite store.

## Observed loop

1. Worker message `#1` opened raw thread `#1` at `attention:rob`.
2. The operator agent sent escalation `#2` in separate mediated thread `#2`,
   with the experimental source envelope and matching store fingerprint.
3. Station attendance reported `armed`, one live waiter, and zero pending
   unconsumed rows after ingestion/ack. The feed showed the escalation and raw
   source provenance.
4. A reply was entered through the Station WebView and sent as message `#3` in
   mediated thread `#2`.
5. The operator agent routed message `#4` to `worker:builder` in raw thread `#1`.
6. Raw disposition history is `escalated -> closed`; Station marked escalation
   `#2` handled.

The Station was restarted three times while retaining the same local
address/store-scoped session identity. The unresolved escalation and later
mediated conversation backfilled correctly. Detaching the operator agent caused
the Station header to render `Operator agent: unattended`; reattachment restored
the online state.

The bounded stress harness added 1,055 newer FYI messages after an unresolved
sentinel. The sentinel fell outside `inbox --all --limit 200` and remained
recoverable through the export-backed unresolved projection after simulated
Station detach/reattach.

`station-mediated-thread.png` captures the feed, source provenance, human reply,
healthy courier, and address status. `station-operator-unattended.png` captures
the occupancy warning.

The Windows toast API returned success for the live interrupt escalation and
feed ingestion continued. A read-only capture from the Windows Action Center
database records the Station AUMID, notification type `toast`, arrival time
`2026-07-19T09:29:10.182738Z`, and the exact live escalation title/body/
attribution. See `windows-action-center-record.json`. No screenshot of the
transient toast flyout was captured; the persisted OS Action Center record is
the notification-publication evidence. The record is reproducible with
`harness/Get-OperatorSpikeToastRecord.ps1`; Focus Assist/quiet-hours perception
was not independently observed.

The same 231-character title/body/attribution was replayed through Station head
`c29dac8278324a90fe789b33fe843654bb24958c`. The resulting Action Center body is
exactly 200 characters, ends in the runtime's ellipsis, and is byte-for-text
identical to the body embedded in the captured XML payload.
