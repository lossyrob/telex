# Operator Station Postgres dogfood evidence

## Result and authority

This report preserves implementation and UX lessons from
[PR #143](https://github.com/lossyrob/telex/pull/143) at exact head
[`949b43eefaea8c26c2f8e9b72587493d1fd68b40`](https://github.com/lossyrob/telex/commit/949b43eefaea8c26c2f8e9b72587493d1fd68b40).
It is non-normative evidence from the historical issue
[#93](https://github.com/lossyrob/telex/issues/93) spike. PR #143 is historical,
non-production source evidence and will close unmerged only after the
direct-main evidence reconciliation commit is verified. Frozen PR #147 will
then close unmerged as superseded by that verified direct-main commit.

The direct human-attended Station direction remains owned by issue
[#134](https://github.com/lossyrob/telex/issues/134), design PR
[#136](https://github.com/lossyrob/telex/pull/136), and the separate builder-owned
direction gate. Production `station-app` work remains blocked on that gate and
Application Client `client-conformance`. This report does not advance #134,
#136, the direction gate, `station-app`, the direct usability gate, operational
hardening, or Application Client conformance. Merging this evidence does not
pass a gate or establish production readiness.

## Evidence method

The source review used PR #143's exact head, commit list, 14-file PR diff, code
tests, PR body, comments, reviews, and the earlier spike artifacts that are
present at that head.

The evidence labels in this report mean:

- **Observed:** a runtime artifact or recorded operator observation documents
  the behavior. The report names whether it predates PR #143.
- **Code-backed:** source or a code test at the exact head implements or checks
  the behavior, but no matching runtime artifact was found.
- **Inference:** the code or commit rationale supports a likely effect, but no
  measurement or scenario proves it.
- **Hypothesis:** a useful criterion for future direct Station validation, not a
  result established by PR #143.

PR #143 has no comments, submitted reviews, screenshots, scenario transcripts,
timing captures, or other new runtime artifacts. Its body says the changes were
made during real Operator Station use, and its commit messages describe the
problems being addressed. Those statements explain provenance but do not prove
the resulting behavior.

Issue #146 comment
[`5444201410`](https://github.com/lossyrob/telex/issues/146#issuecomment-5444201410)
contains the false artifact SHA
`d876612337459044299af5666312dc5b1bfb5f6e`. Frozen PR #147's correct head is
`d87661230ee7f739ea10b20b38ad3abe49b7df58`; the local direct-main proposal has
a different head. The issue comment must be superseded or corrected only after
the direct-main evidence commit is verified and explicit GitHub mutation is
authorized.

The earlier merged spike report and artifacts at
[`1614f010ddc0d6e787ede618e82c5bbb432ee338`](https://github.com/lossyrob/telex/commit/1614f010ddc0d6e787ede618e82c5bbb432ee338)
provide observed Windows/SQLite evidence: a feed and mediated thread, source
provenance, a Station-authored reply and routed outcome, three restarts,
unresolved recovery beyond a 200-message recent tail, operator-agent absence,
and Windows Action Center publication. They do not exercise the PR #143
changes or a Postgres-backed Station.

## Exact source map

### Commits

| Commit | Change represented in this report |
|---|---|
| [`f621ea6`](https://github.com/lossyrob/telex/commit/f621ea6be317354dc8a54b2af68d72fbcb58a1eb) | Named backend selection and an opaque backend-profile fingerprint |
| [`b0db142`](https://github.com/lossyrob/telex/commit/b0db1427ffeed84585bb9addaf2f1924d3692d96) | 60-second foreground/export command budgets and 30-second status polling |
| [`70fead6`](https://github.com/lossyrob/telex/commit/70fead6fd027bf469762cb3efc843ca7795ecd9d) | Thread cache/prefetch and embedded WAV playback |
| [`3d76332`](https://github.com/lossyrob/telex/commit/3d76332cc3d030a054806b0d5ca025fcfbd30715) | Receipt-backed optimistic reply and in-place disposition updates |
| [`b68c220`](https://github.com/lossyrob/telex/commit/b68c220cedf01cde985d7a214d41f7df5ed9c143) | Persistent local read state, unread presentation/count, and mark-unread |
| [`eeeab51`](https://github.com/lossyrob/telex/commit/eeeab51235e88110f7fb2c38444d34db49ea1fa6) | 30-minute wait, sound for every new primary delivery, and broader next-checkpoint toast policy |
| [`4e6be47`](https://github.com/lossyrob/telex/commit/4e6be47b9680ffef2d117b39bf00587903942ddf) | Named-backend startup limited to the recent 200-message inbox and delayed status polling |
| [`8bae444`](https://github.com/lossyrob/telex/commit/8bae4445c420221fcf550bbf021dac88350c58f2) | Synthetic named-backend health presentation and disabled ingress polling |
| [`3f1dc17`](https://github.com/lossyrob/telex/commit/3f1dc1751afaff51f61695832d57a95b5e8e7cc3) | Reply-receipt identity checks without requiring an optional disposition echo |
| [`d8eeefb`](https://github.com/lossyrob/telex/commit/d8eeefb47c65d7818d0e10ab268223625d004f06) | Retry classification for selected Postgres disconnect text |
| [`90f890c`](https://github.com/lossyrob/telex/commit/90f890c54f4259f41e31a04af7f541ba7c525687) | Additional retry classification for `connection closed` |
| [`989ef5a`](https://github.com/lossyrob/telex/commit/989ef5aa0f05cde5ea4692b31f9c0e799328799f) | Neutral `Operator agent: not monitored` presentation |
| [`949b43e`](https://github.com/lossyrob/telex/commit/949b43eefaea8c26c2f8e9b72587493d1fd68b40) | Merge of then-current `main`; the final PR changed-file set remains the 14 files below |

### Changed files at the exact head

| File | Role |
|---|---|
| [`spike/operator-station/README.md`](https://github.com/lossyrob/telex/blob/949b43eefaea8c26c2f8e9b72587493d1fd68b40/spike/operator-station/README.md) | Named-backend setup and bounded-startup description |
| [`src-tauri/Cargo.toml`](https://github.com/lossyrob/telex/blob/949b43eefaea8c26c2f8e9b72587493d1fd68b40/spike/operator-station/src-tauri/Cargo.toml) | Windows audio API feature |
| [`telex-new-msg-1.wav`](https://github.com/lossyrob/telex/blob/949b43eefaea8c26c2f8e9b72587493d1fd68b40/spike/operator-station/src-tauri/sounds/telex-new-msg-1.wav) | Embedded 302,152-byte notification sound, blob `58cd01dd5c09f98f7d82f5d69a285268ae1f24e1` |
| [`cli.rs`](https://github.com/lossyrob/telex/blob/949b43eefaea8c26c2f8e9b72587493d1fd68b40/spike/operator-station/src-tauri/src/cli.rs) | Backend arguments, command budgets, long wait, and receipt validation |
| [`config.rs`](https://github.com/lossyrob/telex/blob/949b43eefaea8c26c2f8e9b72587493d1fd68b40/spike/operator-station/src-tauri/src/config.rs) | Mutually exclusive SQLite/backend configuration and scoped fingerprint |
| [`courier.rs`](https://github.com/lossyrob/telex/blob/949b43eefaea8c26c2f8e9b72587493d1fd68b40/spike/operator-station/src-tauri/src/courier.rs) | Startup projection, waiter, alert dispatch, status, and disconnect handling |
| [`lib.rs`](https://github.com/lossyrob/telex/blob/949b43eefaea8c26c2f8e9b72587493d1fd68b40/spike/operator-station/src-tauri/src/lib.rs) | Sound module registration |
| [`model.rs`](https://github.com/lossyrob/telex/blob/949b43eefaea8c26c2f8e9b72587493d1fd68b40/spike/operator-station/src-tauri/src/model.rs) | Primary-delivery sound/toast policy |
| [`sound.rs`](https://github.com/lossyrob/telex/blob/949b43eefaea8c26c2f8e9b72587493d1fd68b40/spike/operator-station/src-tauri/src/sound.rs) | Asynchronous Windows `PlaySoundW` adapter |
| [`state.rs`](https://github.com/lossyrob/telex/blob/949b43eefaea8c26c2f8e9b72587493d1fd68b40/spike/operator-station/src-tauri/src/state.rs) | High-water-gated alert eligibility and optional occupancy |
| [`App.css`](https://github.com/lossyrob/telex/blob/949b43eefaea8c26c2f8e9b72587493d1fd68b40/spike/operator-station/src/App.css) | Unread accents and neutral status styling |
| [`App.test.tsx`](https://github.com/lossyrob/telex/blob/949b43eefaea8c26c2f8e9b72587493d1fd68b40/spike/operator-station/src/App.test.tsx) | Read-state and unmonitored-status component checks |
| [`App.tsx`](https://github.com/lossyrob/telex/blob/949b43eefaea8c26c2f8e9b72587493d1fd68b40/spike/operator-station/src/App.tsx) | Feed/thread interaction and user-visible presentation |
| [`types.ts`](https://github.com/lossyrob/telex/blob/949b43eefaea8c26c2f8e9b72587493d1fd68b40/spike/operator-station/src/types.ts) | Reply-receipt and disposition result types |

## Behavior evidence

| Behavior | Classification and exact source | Relevant code test | Runtime evidence and limit |
|---|---|---|---|
| Persistent read/unread state | **Code-backed.** [`b68c220`](https://github.com/lossyrob/telex/commit/b68c220cedf01cde985d7a214d41f7df5ed9c143), `src/App.tsx`, and `src/App.css` store read message IDs in browser `localStorage`, keyed by displayed store fingerprint and Station address. Opening a thread marks it read; the thread header can mark it unread. Read state remains separate from Telex acknowledgment and disposition. | `App.test.tsx`: `loads the feed and sends a reply through the Station command` checks the unread count, open-to-read transition, and stored ID. It does not check reload persistence, mark-unread, corrupt storage, or store/address switching. | No PR #143 screenshot, reload transcript, or runtime state capture. Earlier screenshots predate this change. |
| Unread count and visual treatment | **Code-backed.** [`b68c220`](https://github.com/lossyrob/telex/commit/b68c220cedf01cde985d7a214d41f7df5ed9c143), `src/App.tsx`, and `src/App.css` add `N unread`, an unread dot, left accent, background treatment, and Read/Unread labels. They also stop automatically selecting the first or newly delivered item, avoiding an immediate read transition. | The same `App.test.tsx` test checks count changes. No test asserts the CSS treatment, accessibility beyond the dot label, or no-auto-select behavior. | No exact-head screenshot or usability observation. |
| Thread navigation responsiveness | **Code-backed; responsiveness is inferred.** [`70fead6`](https://github.com/lossyrob/telex/commit/70fead6fd027bf469762cb3efc843ca7795ecd9d), `src/App.tsx`, caches completed thread reads, coalesces duplicate in-flight requests, and sequentially prefetches the first 20 feed messages. Delivery invalidates the delivered message's cache entry. Reply and disposition mutate the cached thread in place. The remaining `runAction` invalidation applies to courier retry. Foreground errors remain visible while speculative prefetch errors are suppressed. | No targeted cache, invalidation, concurrency, eviction, or latency test. | No before/after timing, trace, large-feed result, or cache memory measurement. The implementation can issue up to 20 serial CLI reads after each full state event. |
| Optimistic reply UX | **Code-backed.** [`3d76332`](https://github.com/lossyrob/telex/commit/3d76332cc3d030a054806b0d5ca025fcfbd30715), `src/App.tsx`, and `src/types.ts` append a reply to the selected cached thread only after the CLI returns a receipt, then clear the composer and re-enable controls without a full thread reload. [`3f1dc17`](https://github.com/lossyrob/telex/commit/3f1dc1751afaff51f61695832d57a95b5e8e7cc3), `src-tauri/src/cli.rs`, accepts only `delivered` or `queued-unoccupied` receipts with positive message/thread IDs and matching parent/sender identity. | `reply_receipt_does_not_require_optional_disposition_echo` and `reply_receipt_rejects_wrong_parent_or_sender` check receipt acceptance. The component test checks command invocation, not optimistic rendering or later reconciliation. | No scenario proves the optimistic row matches a subsequent authoritative read. The row fills missing receipt fields with local defaults and has no explicit pending/reconciled/error state. |
| Optimistic disposition UX | **Code-backed.** [`3d76332`](https://github.com/lossyrob/telex/commit/3d76332cc3d030a054806b0d5ca025fcfbd30715), `src/App.tsx`, updates the feed, selected message, thread disposition list, and actionable state from the returned disposition record without a full reload. | No component or Rust test exercises the in-place disposition result or reconciliation. | No runtime artifact proves behavior after concurrent disposition, stale selection, restart, duplicate response, or authoritative mismatch. |
| Sound behavior | **Code-backed.** [`70fead6`](https://github.com/lossyrob/telex/commit/70fead6fd027bf469762cb3efc843ca7795ecd9d), `src-tauri/src/sound.rs`, `courier.rs`, and `lib.rs` add asynchronous embedded WAV playback; [`eeeab51`](https://github.com/lossyrob/telex/commit/eeeab51235e88110f7fb2c38444d34db49ea1fa6), `model.rs` and `state.rs`, expands eligibility to every newly ingested primary delivery above the persisted high-water mark. Sound failure becomes a diagnostic and does not block toast, delivery emission, or acknowledgment. | `embedded_new_message_sound_is_a_wave_file` checks only the RIFF/WAVE header. `sound_policy_covers_every_new_primary_delivery` checks primary-versus-CC policy. | No exact-head auditory capture, OS result, volume/mute result, duplicate/restart result, or pressure test. The WAV source and license provenance are unresolved. |
| Toast behavior | **Code-backed.** [`eeeab51`](https://github.com/lossyrob/telex/commit/eeeab51235e88110f7fb2c38444d34db49ea1fa6), `src-tauri/src/model.rs`, `state.rs`, and `courier.rs`, makes every new primary `next-checkpoint` delivery toast-eligible, even without a disposition requirement; interrupt remains eligible, FYI remains excluded, and selected actionable experimental escalations remain eligible. High-water and dedupe checks suppress old/repeated alerts. | `toast_policy_is_narrow_and_primary_only` checks the policy branches. | The older SQLite Action Center record proves publication for one interrupt escalation, not the widened policy at PR #143. No notification-volume, quiet-hours, Focus Assist, disabled-notification, duplicate, or mixed-role evidence exists. |
| Status and health presentation | **Code-backed, with a synthetic limitation.** [`8bae444`](https://github.com/lossyrob/telex/commit/8bae4445c420221fcf550bbf021dac88350c58f2), `src-tauri/src/courier.rs`, `state.rs`, and `src/App.tsx`, stops Postgres status CLI probes and projects the Station as occupied with `application-attached`; it omits backlog and live-waiter counts and does not monitor ingress. [`989ef5a`](https://github.com/lossyrob/telex/commit/989ef5aa0f05cde5ea4692b31f9c0e799328799f), `src/App.tsx`, `App.css`, and `App.test.tsx`, renders the absent ingress status as neutral `Operator agent: not monitored`. Courier state, runtime banners, diagnostics, version, and displayed store fingerprint remain visible. | `shows a neutral status when operator agent polling is disabled` checks the neutral ingress label. `diagnostics_are_bounded_without_message_log_storage` checks only the 200-entry in-memory diagnostic bound. | No artifact proves real Postgres membership, waiter health, backlog, or principal identity. `application-attached` is local synthetic presentation, not backend health. |
| Postgres startup/backfill latency | **Code-backed; improvement is inferred.** [`4e6be47`](https://github.com/lossyrob/telex/commit/4e6be47b9680ffef2d117b39bf00587903942ddf), `src-tauri/src/courier.rs`, skips full export for named backends and loads only `inbox --all --limit 200`; [`b0db142`](https://github.com/lossyrob/telex/commit/b0db1427ffeed84585bb9addaf2f1924d3692d96), `cli.rs`, `config.rs`, and `courier.rs`, raises command/export budgets from 10 to 60 seconds and status cadence from 5 to 30 seconds. | `startup_keeps_all_unresolved_plus_recent_two_hundred` checks the generic projection function, not the named-backend branch. Backend selector tests check CLI arguments and environment. | No startup duration, query duration, row count, restart/backlog scenario, or before/after capture exists. On Postgres the waiter still starts after attach, version, and the bounded inbox call, so "prompt" is not measured and live receive is not proven to precede backfill. |
| Live-receive continuity | **Code-backed; gap reduction is inferred.** [`eeeab51`](https://github.com/lossyrob/telex/commit/eeeab51235e88110f7fb2c38444d34db49ea1fa6), `src-tauri/src/cli.rs` and `courier.rs`, changes the one-shot wait timeout from 30 seconds to 30 minutes. Delivery remains `wait -> read --full -> ingest -> sound/toast -> frontend event -> ack -> re-arm`. | `delivery_cannot_ack_before_ingest_and_frontend_emit` checks ordering. No test measures re-arm gaps or delivery during startup/backfill. | No Postgres trace proves uninterrupted attendance, live-receive priority, losslessness, or duplicate behavior. This is still a child-process waiter. |
| Transient disconnect handling | **Code-backed.** [`d8eeefb`](https://github.com/lossyrob/telex/commit/d8eeefb47c65d7818d0e10ab268223625d004f06), [`90f890c`](https://github.com/lossyrob/telex/commit/90f890c54f4259f41e31a04af7f541ba7c525687), and `src-tauri/src/courier.rs` classify exit code 1 plus five stderr substrings as retryable backoff. Other exit-code-1 failures pause for manual retry. The banner says the Postgres connection was interrupted. | `courier_exit_decisions_match_recovery_contract` checks administrator termination and `connection closed`; it does not cover every accepted string, real reconnect, backoff timing, delivery after recovery, or repeated flaps. | No fault-injection transcript or live disconnect/recovery artifact exists. String matching can misclassify changed/localized stderr. |
| Named backend and local scope | **Code-backed.** [`f621ea6`](https://github.com/lossyrob/telex/commit/f621ea6be317354dc8a54b2af68d72fbcb58a1eb), `src-tauri/src/config.rs`, `cli.rs`, and `README.md`, makes SQLite path and named backend mutually exclusive, passes `--backend` and `TELEX_OPERATOR_SPIKE_BACKEND`, and derives local scope from Station address plus a case-insensitive SHA-256 hash of the backend profile name. The UI displays that hash. | `backend_child_gets_required_environment_and_explicit_selector`, `backend_fingerprint_is_opaque_and_case_insensitive`, and `persisted_state_is_scoped_by_address_and_fingerprint` check local mechanics. | The backend profile name is not a logical store or principal identity. No artifact records the actual Postgres store, profile target, credentials boundary, server identity, or attached principal. |
| Foreground failure visibility | **Code-backed.** [`b0db142`](https://github.com/lossyrob/telex/commit/b0db1427ffeed84585bb9addaf2f1924d3692d96), [`8bae444`](https://github.com/lossyrob/telex/commit/8bae4445c420221fcf550bbf021dac88350c58f2), `src-tauri/src/cli.rs`, `courier.rs`, `state.rs`, and `src/App.tsx` surface longer command failures, courier backoff/paused detail, bounded diagnostics, and manual Retry courier. | Existing parser/fixture tests check required response shapes; no UI scenario tests timeouts or retry recovery. | No screenshot or operator study shows whether these states are understandable or actionable. |

## CI and test limits

PR #143 reports six successful repository checks:

- `Build and test (ubuntu-latest)`
- `Build and test (windows-latest)`
- `Copilot fallback E2E (macos-latest)`
- `Copilot fallback E2E (windows-latest)`
- `Feature combinations build`
- `Live Postgres parity tests`

The repository workflows do not reference `spike/operator-station`, and the
nested package has its own `npm test`, `npm run build`, `cargo fmt`, and
`cargo test` commands. The six green repository checks therefore do not
exercise the nested Station npm/Tauri package or prove live Postgres Station
behavior. Code tests in the changed package are relevant source evidence only;
PR #143 does not preserve an exact-head run result for them.

## Missing evidence

PR #143 does not preserve:

- exact startup, backfill, thread-open, prefetch, reply, disposition, alert, or
  reconnect timings;
- exact-head screenshots, video, traces, logs, or scenario transcripts;
- the Postgres backend profile, logical store identity, server identity,
  Station principal, operator-agent principal, or credential boundary;
- restart continuity, unresolved-backlog completeness, delivery during startup,
  live-receive priority, duplicate delivery, or reconnect-after-failure results;
- optimistic reply/disposition reconciliation against later authoritative
  state, including mismatch and process-restart outcomes;
- notification-pressure results across attention levels, primary and CC roles,
  bursts, Focus Assist, quiet hours, user-disabled notifications, audio volume,
  mute state, or sound/toast failures;
- before/after captures for the latency, responsiveness, and contention claims;
- source and license provenance for `telex-new-msg-1.wav`.

## Carry-forward criteria

These are advisory criteria, not accepted architecture or gate results.

### Operator Station

- **Keep local read state separate from Telex acknowledgment and disposition.**
  Validate persistence, store/address scoping, explicit mark-unread, visual and
  accessible treatment, retention/cleanup, and multi-message behavior.
- **Keep thread navigation responsive with bounded work.** Measure cold and warm
  thread-open latency on realistic stores, bound prefetch concurrency and memory,
  invalidate stale entries, and surface foreground failures without hiding
  speculative failures needed for diagnosis.
- **Present optimistic actions only after durable receipt identity.** Show
  pending/reconciled/failed states and reconcile replies and dispositions with
  authoritative updates, duplicates, restarts, concurrent actions, and identity
  mismatches.
- **Make audio and toast policy configurable and evidence-bearing.** Validate
  attention pressure, OS suppression, primary/CC roles, burst behavior,
  deduplication, restart quietness, accessibility, and licensed asset provenance.
- **Prioritize live receive before expensive backfill.** Establish the supported
  receive path promptly, then recover unresolved and bounded history through
  paged/cursor semantics. Measure both paths on credentialed Postgres.
- **Present honest health.** Distinguish application attachment, backend
  reachability, live receive, backlog, membership, and unmonitored dependencies.
  Do not substitute local synthetic occupancy for backend evidence.

### Application Client advisory routing

The generic seams should inform issue
[#12](https://github.com/lossyrob/telex/issues/12), client-core
[#129](https://github.com/lossyrob/telex/issues/129), and later
`client-conformance` as advisory evidence only:

- stable logical-store and source identity that does not depend on a backend
  profile-name hash;
- supported streaming receive without CLI child-process/stdout parsing;
- typed transient disconnect and retryability results without stderr substring
  or exit-code classification;
- paged/cursor unresolved and recent-history recovery rather than a fixed
  200-message approximation;
- durable receipt identity and authoritative reconciliation for optimistic UI;
- exact backend and principal identity in health and evidence.

Application Client remains the authority for these shared semantics. This report
does not add requirements to its contract or mark any dependency complete.

## Non-promoted implementation choices

PR #143 does not make the following intended architecture:

- the mediated operator-agent topology or experimental message kinds;
- CLI process invocation, stdout parsing, stderr substring matching, exit-code
  classification, or the 30-minute child-process waiter;
- backend-profile hashing as logical-store identity;
- synthetic `application-attached` health;
- bounded 200-message startup recovery.

The direct Station may preserve the user need behind an experiment without
preserving its mechanism.

## Hypotheses for later validation

- Starting supported live receive before history recovery will reduce missed or
  delayed attention during large Postgres backfills.
- Bounded, concurrent prefetch with measured cache limits will improve perceived
  thread navigation without creating a burst of backend work.
- Receipt-gated optimistic presentation plus explicit reconciliation states will
  feel immediate without misleading the operator about durable outcomes.
- A configurable, role-aware sound/toast policy with retained decision evidence
  will reduce notification pressure while preserving urgent attention.

These hypotheses require direct Station implementation and runtime validation.
No hypothesis in this report passes the direction, usability, hardening, or
closure gate.
