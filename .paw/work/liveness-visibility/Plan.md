# Plan

## Approach Summary

Implement the liveness-visibility node as an operator-facing daemon observability pass, not as prerequisite-only cleanup. The node outcome anchor is that deaf stations, abnormal waiter exits, and foreign/ghost leases become self-evident through stable CLI/status surfaces and focused tests.

The implementation will extend daemon status data with thresholded deaf-station signal and daemon-authored terminal waiter outcomes, expose all-session/foreign lease visibility through `station status --all-sessions` and richer address/status output, and prove the JSON/text contracts with focused daemon unit and real-process SQLite tests.

### Data and CLI Contract

- Extend daemon/member status with explicit terminal waiter fields: `last_waiter_exit_at_ms`, `last_waiter_outcome`, `last_waiter_exit_code`, `last_waiter_detail`, and `last_waiter_pid`. Preserve existing fields and make new fields additive/nullable.
- Define terminal outcomes from a single source of truth (`WaiterOutcome` enum or shared constants with serde/value tests), not inline strings. Member-recorded outcomes are `message` (exit `0`), `idle-timeout` (exit `2`), `presence-ended` (exit `5`, detail disambiguates `session-end`, `station-stop`, `idle-ttl-reap`, or `reset`), `daemon-error` (exit `1` when the daemon can associate the error with a member), and `abnormal-exit` (no telex exit code when the waiter process dies before daemon response).
- Keep rejected/no-op attempts out of the member terminal-exit slot when another live waiter remains healthy. Record `concurrent-waiter-rejected`, `unacked-delivery-blocked`, and missing-member `needs-attach` as recent errors or a distinct rejected-attempt/history surface so they do not overwrite delivery-success bookkeeping.
- Separate delivery-success bookkeeping from terminal status metadata, or explicitly preserve the current `message`/`last_delivered_message_id` gate semantics when writing non-message outcomes. A non-message outcome must not clear the unacked-delivery guard until the delivered message is acked; subsequent success should visibly supersede stale error badges.
- Record terminal outcomes at daemon write sites, not only in waiter-authored `--out-dir` artifacts: successful message delivery, timeout, presence-ended/idling paths, associated daemon errors, and bounded pid-pruned/dead waiter records. Abnormal-exit detection must have a defined trigger and latency target; prefer a daemon-side prune/reaper path that is not solely caused by the operator's first status query.
- Add thresholded deaf-station fields for `unattended_with_backlog`: per-member `unattended_since_ms`, `unattended_for_ms`, `deaf_warn`, and a top-level daemon `deaf_stations` summary with `count`, `warn`, and `warn_threshold_ms`. Define a runtime override following existing warn-threshold patterns, a documented default, and reset behavior across re-attach/restart.
- Keep default `station status` session-scoped. Add `station status --all-sessions` to show all active stations in the selected store, optionally narrowed by global `--address`, with `foreign_session` markers relative to the querying session.
- Add foreign/deaf projections to address/status surfaces without mutating leases: address-scoped `status`, `address show`, and `address list` JSON should expose current-store foreign members and deaf warning state. Text output is human-readable operator aid; JSON is the stable machine contract.
- Preserve authorization posture explicitly: `--all-sessions` uses the existing admin-cap-backed status path and only widens the local per-user daemon view intentionally; document its relationship to existing `also_active_on` alternate-store hints.

### Exit Path and Proof Matrix

| Path | Daemon-authored terminal signal | Proof target |
| --- | --- | --- |
| Message delivered | `last_waiter_outcome=message`, exit `0`, message id preserved | Existing delivery tests plus member status assertion |
| Idle timeout | `last_waiter_outcome=idle-timeout`, exit `2` | Daemon unit test |
| Station/session reset or idle TTL | `last_waiter_outcome=presence-ended`, exit `5`, detail token identifies source | Existing station/session tests plus member status assertion |
| Concurrent second waiter / no-op | recent error or distinct rejected-attempt record, not terminal slot when first waiter remains armed | Daemon unit test |
| Re-arm before ack / lost-race non-delivery | recent error or distinct rejected-attempt record; prior `message` delivery guard remains intact | Daemon unit test |
| Missing member / explicit re-attach required | daemon `recent_errors` includes `NeedsAttach`; waiter out-dir remains client-side transport | Existing NeedsAttach tests plus status assertion |
| Waiter killed or pruned before response | `last_waiter_outcome=abnormal-exit`, no telex exit code, pid/detail captured, visible within bounded prune/reaper latency | Daemon unit test plus real-process SQLite kill/prune visibility |
| Visibility-only queries | no lease, membership, pending message, delivery, or ownership mutation | Before/after daemon/unit or real-process invariant test |

### Test Matrix

- Daemon unit tests: typed outcome/value round-trip; thresholded deaf summary/member fields including override; terminal outcome recording for timeout, presence-ended with detail, and pruned abnormal waiter; rejected/no-op attempts do not pollute a healthy armed member; non-message outcomes do not break the unacked-delivery re-arm guard; default session status compatibility where applicable.
- Real-process SQLite tests: `station status --all-sessions` shows a foreign session and default `station status` still filters; `status --address`, `address show`, and `address list` expose deaf/foreign state in JSON; text output makes deaf/foreign state visible; killing a waiter leaves daemon-authored abnormal terminal status even when `exit.code` is absent; visibility-only commands do not mutate lease/fence/pending state.
- Documentation proof: update `docs/design/daemon.md` status-surface and out-dir/liveness sections only for the additive daemon-authored fields and `--all-sessions`/foreign visibility surfaces.

## Work Items

- [x] Add daemon-authored visibility data for terminal waiter outcomes and thresholded `unattended_with_backlog` stations, including deterministic status fields for abnormal/no-op waiter exits.
- [x] Expose all-session and foreign-lease status through CLI surfaces: `station status --all-sessions`, address/status JSON projections, and text output that makes deaf stations and foreign owners obvious.
- [x] Add focused tests for the exit-path/proof matrix: deaf-station visibility, abnormal waiter terminal status, no-op/lost-race terminal outcomes, and foreign/all-session lease visibility, including JSON shape checks and operator text checks where visibility is the contract.
- [x] Update the normative design documentation only where the surfaced status fields or CLI flags become part of the daemon observability contract.
- [x] Run targeted validation for the changed daemon/status/wait surfaces and commit the implementation with selective staging.

## Key Decisions

- Preserve the node outcome anchor: the final PR should use `Closes #46` only if it includes demonstrable CLI/status visibility and tests for all three acceptance surfaces; otherwise it must use `Refs #46` and report partial status.
- Keep liveness non-destructive. Visibility should report unattended/backlog, terminal waiter outcomes, and foreign ownership without taking ownership, detaching sessions, consuming messages, or mutating leases.
- Prefer additive JSON fields and CLI flags over replacing existing status behavior. Existing session-scoped `station status` remains the default; `--all-sessions` intentionally opts into broader visibility.
- Treat daemon-authored terminal waiter status as queryable daemon state. `--out-dir` remains transport output written by the waiter process; the new daemon-side state must remain useful even when a waiter cannot write its artifacts.
- Defer bounded waiter-exit history and deaf-warn transition logs/metrics unless they fall out cheaply from the implementation; the node must still expose current status and the last terminal/rejected signal needed for issue #46.

## Open Questions

None.
