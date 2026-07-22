# Operator loop spike report

## Outcome

**Complete for issue #93's implementation node; not a builder viability pass.**

The Windows spike demonstrated the complete mediated loop on a real isolated
Telex store:

`worker -> attention:rob -> operator agent -> operator:rob Station -> human reply -> operator agent -> worker`

The Tauri Station is runnable, the reusable operator-agent assignment is in
`spike/operator-station/OPERATOR-AGENT.md`, and the builder walkthrough is in
`spike/operator-station/WALKTHROUGH.md`.

## Demonstrated scenario

| Leg | Evidence |
|---|---|
| Worker to operator agent | Message `#1`, raw thread `#1`, `worker:builder -> attention:rob` |
| Operator escalation | Message `#2`, mediated thread `#2`, kind `operator-station-spike.escalation` |
| Healthy Station attendance | `station_health=armed`, one live waiter, zero pending unconsumed after ingest/ack |
| Feed and source provenance | `spike/operator-station/evidence/station-mediated-thread.png` |
| Windows notification publication | Reproducible Action Center record `spike/operator-station/evidence/windows-action-center-record.json` |
| Human reply authored in Station | Message `#3`, `operator:rob -> attention:rob`, mediated thread `#2` |
| Route back to worker | Message `#4`, `attention:rob -> worker:builder`, raw thread `#1` |
| Raw lifecycle | Message `#1` dispositions: `escalated`, then `closed` |
| Human-facing disposition | Escalation `#2` marked `handled` through Station UI |
| Restart continuity | Three Station restarts retained the same scoped session identity and backfilled the unresolved/recent thread |
| >1,000-message cutoff | 1,055 newer FYI rows pushed an unresolved sentinel outside the 200 recent tail; export recovery still found it |
| Operator absence | Station rendered `Operator agent: unattended`, then returned online after reattach |

The sanitized record is
`spike/operator-station/evidence/demo-transcript.json`.

## What the spike built

- A standalone Tauri v2 Windows application outside the Telex workspace.
- One supervised application-owned `telex wait` courier at a time.
- `wait -> read --full -> ingest/dedupe -> ack -> re-arm` delivery ordering.
- Export-backed unresolved startup recovery plus a 200-message recent inbox.
- Feed, mediated thread, reply, defer/handle/close, raw metadata, and source
  provenance rendering.
- Windows AUMID/toast emission that cannot block feed ingestion.
- Address health for the Station and operator-agent ingress.
- A versioned operator-agent assignment and bounded PowerShell smoke/stress
  harness.

## Experimental source convention

Only this namespace is interpreted:

```text
extension key: operator-station-spike
extension ID:  urn:telex:experimental:operator-station-spike:v1
schema:        urn:telex:experimental:operator-station-spike:v1#escalation
```

Each source reference carries:

- numeric message and thread ID;
- captured sender, recipient, subject, and sent time;
- full safe store fingerprint.

The Station opens a numeric source only when the fingerprint matches the active
store. Missing/mismatched identity renders `unavailable in current store`.
Unknown namespaces remain raw metadata. No production `operator-station`
namespace is reserved.

### Experimental string inventory

| String | Role | Interpretation status |
|---|---|---|
| `operator-station-spike.escalation` | Operator agent to human Station | Interpreted for toast eligibility and v1 source provenance |
| `operator-station-spike.human-reply` | Station to operator agent | Emitted by the Station; application convention only |
| `operator-station-spike.clarification` | Operator agent to worker | Assignment convention only |
| `operator-station-spike.routed-outcome` | Operator agent to worker | Assignment/harness convention only |
| `operator-station-spike.stress-fyi` | Restart stress traffic | Harness-only |
| `urn:telex:experimental:operator-station-spike:v1` | Extension identity | Interpreted only by the spike |
| `urn:telex:experimental:operator-station-spike:v1#escalation` | Escalation schema marker | Interpreted only by the spike |
| `operator-station-spike.demo-evidence.v1` | Sanitized evidence file | Evidence-only |
| `operator-station-spike.smoke-evidence.v1` | Harness result | Evidence-only |
| `operator-station-spike.windows-action-center-evidence.v2` | Action Center extraction | Evidence-only |

All are replaceable experimental strings. Issue #12 and `station-contract` own
promotion, renaming, or retirement.

## Key decisions and pivots

1. **Inbox polling was rejected as live attendance.** External plan review
   correctly identified that `attach + inbox` proves visibility but not delivery
   consumption or healthy attendance. The implementation moved to a supervised
   one-shot wait courier with ack only after application ingestion.
2. **Unresolved recovery uses export, not actionable inbox limits.** Current
   inbox limits apply before actionable filtering. Startup scans selected-address
   JSONL export to retain every unresolved primary obligation plus a recent tail.
3. **Source IDs are store-local.** The same fingerprint scopes local Station
   state and numeric source resolution.
4. **CLI additions are compatible.** Required fields/types fail visibly;
   additive fields are tolerated. Captured fixtures document the tested private
   shapes without declaring a public contract.

## Temporary integration shortcuts

- Every application operation is a `telex` subprocess.
- Live delivery is a repeatedly supervised one-shot waiter.
- Startup export materializes all selected-address history in the current CLI
  process before the Station retains unresolved/recent rows.
- Startup export has a hard 10-second subprocess budget. The 1,055-message
  stress store completed within it; a larger/slower store fails visibly and
  leaves the courier paused. Issue #12 needs a paged unresolved query/cursor.
- SQLite path selection is process environment configuration; only its
  fingerprint is persisted/displayed.
- Local SQLite and Windows are the only exercised deployment.
- Development launch uses `npm run tauri dev`; there is no installer, upgrade,
  auto-start, signing, or production packaging.
- The Station session UUID/high-water marker is local app-data state rather than
  a shared Application Client facility.
- The daemon-hung automatic recovery probe uses the same Telex CLI/daemon path;
  persistent hangs may require operator repair plus the visible **Retry courier**
  action.
- Windows AUMID registration is written under HKCU at startup. Production
  packaging must own install, upgrade, and removal of that registration.

## Product observations

- A distilled escalation with recommendation context is understandable without
  opening the raw thread, while the source card preserves auditability.
- Separating raw and mediated threads made the return path unambiguous: the
  Station reply stayed in thread `#2`; the routed result returned in thread `#1`.
- Delivery/ack health is important product state. `armed` and zero pending
  unconsumed rows are materially stronger evidence than a message appearing in
  a read-only feed.
- Operator-agent occupancy is valuable human context. The explicit unattended
  warning prevented quiet operator failure from looking like "no news."
- Reply plus separate disposition is usable for the spike, but a production
  Station should decide whether common reply/disposition combinations need one
  higher-level operation.
- Station-authored human replies are disposition-required. The operator
  assignment verifies and sends the worker route-back before acknowledging and
  terminally handling the human reply, preserving recovery if routing or the
  operator session fails without leaving a completed reply actionable.

## Failures and limitations observed

- The first live UI reply failed because the installed Telex 0.1.0 CLI does not
  accept `reply --body-stdin`; the adapter changed to the supported
  `--body-file -` stdin form.
- The current wait payload does not include metadata, requiring a second
  `read --full` call before ingest/ack.
- The Windows toast API returned success and no toast error reached the UI. A
  reproducible read-only extraction from the Windows Action Center database
  contains the live escalation title/body/attribution and arrival timestamp.
  A transient flyout screenshot was not captured, so Focus Assist/quiet-hours
  perception was not independently verified.
- Notification evidence was regenerated from Station head
  `c29dac8278324a90fe789b33fe843654bb24958c` using the same 231-character
  escalation content. The persisted Action Center body is the expected
  200-character runtime truncation (199 characters plus `…`) and exactly matches
  the XML payload. Evidence schema v2 names this code revision
  `stationReplayHead`. Its `extractor.currentCheckoutPath` is deliberately an
  independent locator in the checkout containing the artifact, not a
  capture-time path that must resolve at the replay head.
- Full export can become slow or memory-heavy on a large store.
- Postgres, remote principals, spoofing resistance, noisy production traffic,
  delayed/stale replies, and security hardening were not validated.
- No production telemetry or log rotation subsystem was added; diagnostics are
  bounded in memory.
- `spike/operator-station/evidence/return-path-recovery-evidence.json` exercises
  operator detach/reattach after the human reply exists but before route-back,
  then proves the unacked obligation remains actionable and becomes terminal
  and non-actionable only after the successful route-back.

## Requirements for issue #12

1. One supported application station identity and attach/detach/recovery
   lifecycle, including stable store identity without exposing credentials or
   paths.
2. A streaming/callback/async-iterator receive API that yields message,
   delivery-role context, metadata, and ack capability without a subprocess
   courier and follow-up read.
3. Explicit ack-after-ingest semantics, duplicate/redelivery identity, and
   observable ack-pending/deaf states.
4. A query/cursor for all unresolved obligations plus bounded recent history,
   without full-store materialization or inbox pre-filter limit ambiguity.
5. Typed send, reply, read-thread, and disposition operations with additive
   response compatibility.
6. Service/application identity and backend selection suitable for SQLite and
   credentialed Postgres.
7. Safe source-reference/store identity conventions that can be promoted,
   revised, or rejected after the viability gate.
8. Clear reply/disposition atomicity and recovery behavior when a human answers
   after the originating session changes.
9. Delta-oriented application events instead of serializing the complete feed
   on every status or courier-state mutation.
10. Reply attention selection and richer operator notes rather than the spike's
    fixed background reply/default disposition note.
11. Receipt identity cross-checks and explicit retry throttling for application
    commands.
12. Local scope discovery/cleanup and replacement-store identity beyond a path
    fingerprint.

## Success-criterion evidence matrix

| Issue #93 criterion | Result | Evidence |
|---|---|---|
| Worker message reaches operator agent through durable address | demonstrated | Raw message `#1`, push-attended `attention:rob` |
| Human escalation is understandable and preserves provenance | demonstrated | Escalation `#2`, source card/screenshot, metadata fixture |
| Desktop feed and Windows notification | publication demonstrated; transient flyout not independently observed | Feed screenshot plus reproducible persisted Windows Action Center toast record |
| Station reply reaches operator agent and routes back | demonstrated | Messages `#3` and `#4` |
| Raw and mediated threads remain separately auditable | demonstrated | Thread IDs `#1` and `#2`, dispositions in transcript |
| Restart preserves unresolved/recent conversation | demonstrated | Stable Station identity across three restarts and UI backfill |
| Old unresolved obligation survives >1,000 newer IDs | demonstrated | `spike/operator-station/evidence/stress-evidence.json`: 1,055 FYI rows, sentinel absent from recent 200 and recovered by export |
| Report separates value from temporary integration | demonstrated | This report's observations, shortcuts, and #12 requirements |
| Builder can launch viability gate without implementation | demonstrated | README, walkthrough, assignment, harness, fixtures |

## Viability gate observations

Append future builder observations here with date, scenario, notification volume,
operator-agent behavior, reply quality, defects, and whether terminal-tab
polling was reduced. This section is an evidence log; appending an observation
does not promote the experimental namespace or pass the gate automatically.

### Dogfood: 2026-07-21 — guided synthetic session

- **Duration / participants:** One live Windows Station, one real Copilot
  operator-agent session, and one synthetic worker identity on a fresh isolated
  SQLite store. The run included several hours of idle continuity before final
  verification.
- **Raw messages:** One genuine human-priority decision, one routine completion,
  and one missing-evidence release question followed by evidence.
- **Human escalations:** One. The operator correctly escalated the
  learning-speed versus dependency-freeze risk instead of choosing for the
  builder.
- **Human outcome:** The builder selected Tuesday. The reply stayed in the
  mediated thread; the operator verified route-back before acknowledging and
  handling the human reply, returned the decision to the original raw thread,
  and closed the raw obligation.
- **Routine filtering:** The completion was handled locally with a concise
  proceed response and no human escalation.
- **Clarification filtering:** The release question was deferred with a precise
  request for CI, rollback, and mergeability evidence. After the worker replied
  to that clarification with all three facts, the operator handled the original
  obligation and sent a release-ready response.
- **Restart / continuity:** The Station restarted with the same scoped session
  identity, returned to `armed`, backfilled the mediated conversation, and did
  not restore a terminal message as actionable.
- **Notification:** A Windows Action Center record matched the live escalation
  title, truncated body, attribution, and arrival time.
- **Observed friction:** The first automated operator launch truncated a
  multiline prompt, and subsequent retries left orphaned session occupancy that
  required explicit cleanup before a clean operator could attach. This is
  launch/orchestration tooling friction rather than evidence against the
  mediated product loop.
- **Observed UX gap:** After the builder replied and reported clicking Handle,
  the root human escalation still had no Station disposition and required CLI
  cleanup. The run did not establish whether the click targeted a changed
  selection or whether the reply/handle interaction is unclear. Production
  contract work should test reply-plus-disposition as one higher-level action.
- **Terminal inspection:** The builder did not need to inspect the worker
  thread to understand or answer the decision; verification after the run did
  inspect Telex state to assess gate evidence.
- **Decision:** Pending. The loop worked and showed useful filtering behavior,
  but one guided synthetic worker is insufficient to pass the gate. Run a
  focused session with multiple real workers and measure escalation quality and
  terminal checks avoided before deciding pass, reshape, or stop.

## Deferred carry-forward items

- Production notification-policy validation under Focus Assist, quiet hours, and
  user-disabled notification settings.
- Paged unresolved/history APIs and delta events for large stores.
- Remaining failure-injection coverage for process shutdown, duplicate live
  ingest, restart-quiet high-water, and receipt identity mismatches.
- Optional optimistic display of a just-sent reply.
- Reply attention selection, retry-button throttling, and richer disposition
  notes.
- Persisted-scope before/after restart artifact capture; the current proof uses
  observed stable session identity and backfilled UI state.
