# Copilot stall-detection gate: Windows evidence

The Windows behavioral evidence covers the four
[workstream gate criteria](https://github.com/lossyrob/telex/issues/201).
The real campaign root received two scheduled detector events, confirmed fresh
App state, rang the member and received replies. Background work continued
unchanged through the ring and completed naturally. Campaign accepted the
[behavioral packet](https://github.com/lossyrob/telex/issues/201#issuecomment-6429700587)
[against the unchanged criteria](https://github.com/lossyrob/telex/issues/201#issuecomment-6429773031).
Final-head review, design inspection and hosted CI remain separate closing-PR gates.

| Pass criterion | Evidence |
| --- | --- |
| Exactly one event per episode | Two real episodes, one durably stored event each, with later committed idle observations and no duplicates; isolated coverage also asserts exact counts. |
| Busy, operator-waiting and armed-watch suppression | Isolated scheduled-host cases remain event-free beyond grace. |
| Root confirms and rings | Real push messages 4193 and 4194, fresh App confirmation before each App ring, member replies and terminal handled dispositions. |
| Status view from the same data | Root snapshot/App view below, joined with public watch health; membership, activity, wakes, background uncertainty and observation time remain explicit. |

## Build and scope

The initial workstation dogfood used Telex 0.3.0 and Watcher built with Entra support
from main `9e971fd9ee877a20d13c97ad59b25d477b5c6ad0`, which includes the
classifier, detector, guidance and bounded-inbox-query changes. The retained
binaries have these SHA-256 identities:

| Binary | SHA-256 |
| --- | --- |
| Initial Telex, retained as rollback | `fe3d09c80afe0ba294d9f77936af6df280f196e615d1194b48b374e23b7d761c` |
| Retained Watcher | `343b9d9509d72cb28dacd9be0f9fe3ad51822d079e8e44b5bd07fa624b24521a` |
| Active Telex after #224, build `c4f6ac8c9c43f82aeb861d46ee44f2f7160f275a` | `17c36bc42642f4b6f3ab1dae1ce5cc0d16e99f5631799c7cf5af896256f43a7e` |

This report concerns Windows only. It adds no product changes, does not prove
macOS containment, and does not assert Postgres optimizer index use. Raw logs,
private installation paths and connection details remain outside the repository.

## Repeatable staged evidence

`telex-watcher/tests/stall_gate.rs` runs the branch detector through a real
scheduled Watcher host and an isolated installed-current Telex daemon. Each
process uses unique absolute runtime/database/install roots, an isolated user
home and temporary LOCALAPPDATA. The existing shared process harness handles
bounded subprocess execution and daemon cleanup; its documented retained-root
policy is preserved.

| Case | Observation and assertion |
| --- | --- |
| Busy member | Successful scheduled evaluations after more than the two-second grace; zero stored events. |
| Waiting on `ask_user` | Same successful-evaluation and zero-event assertions, using the shipped waiting fixture. |
| Armed watch targeting the member | Zero stored stall events beyond grace; removing that watch allows the next quiet episode. |
| Quiet member with 1,100 consumed retained messages | A scheduled detector reaches its own host's public `list --active` endpoint and produces a durably accepted stall event without history-size degradation. |
| Episode deduplication | Exactly one stored event and one inbox message; a later committed idle attempt leaves the count at one. |
| Quiet `session.error` | One additional stored attention-needed advisory, explicitly degraded, and no additional stall; a subsequent unchanged evaluation is detector-degraded. |
| Alternating degraded/successful reads | Direct branch-binary invocations against the live management endpoint use request times 0, 30, 70, 100 and 140 seconds. Invalid membership at 30 and 100 yields degraded with no state advance. With grace/freshness 60 seconds, the first gap restarts grace at 70; the second successful read emits at 140. A later read does not duplicate it. |
| Background uncertainty | Both staged event snapshots identify background work as unobserved. This is not the real background-shell proof. |

The host-scheduled callback is the explicit proof previously left as #199
gate debt 6. The alternating-read case uses controlled request timestamps,
not a claim about real scheduler elapsed time.

The initial local staged target completed with four passing tests and one ignored
subprocess entrypoint, which its parent test executes. Raw output SHA-256:
`af6b1ce39a3d457c2f150b9f2739fb852a26dae7dfe6be15cb79ab6eb89b8f62`.
The existing 20 detector unit tests also passed locally. These are supporting
local observations, not exact-head hosted CI or final independent review.

After integrating main `a12649aa844134f3212588fa8332208d56c52e55`, the gate
branch rebuilt Telex with Entra and repeated the scheduled-host target
(four passed, one parent-executed subprocess entrypoint ignored), all 20
detector tests, formatting and whitespace checks successfully. Test runtime
roots and LOCALAPPDATA were isolated; no installed runtime was replaced.
This integrated-source validation log has SHA-256
`6d8cb03d0431bf668a89948204b553d34370b826f58bf702a51a86f901044ab1`.

On Windows, after building the workspace binaries, reproduce the new target:

```text
cargo build --locked --workspace --bins
cargo test --locked -p telex-watcher --test stall_gate
```

The fixture uses SQLite; the real installation additionally requires both
packages' `entra` features. The source identity and hosted-run evidence for a
final PR must be recorded separately.

## Real campaign setup and failure

The operator selected the Telex campaign itself as the real workstream and
Campaign as root. The root created the two strict membership records and kept
the local-CLI-to-App mapping outside the membership directory. One owner was
active; the wait coordinator was parked on its intended generated fallback.

The root executed the shipped PowerShell registration template with installation
values substituted, a 30-second cadence/grace, and TELEX_CONFIG allowlisted
alongside TELEX_HOME as the guide requires. Watch
`campaign-stalls-201-20261026`, revision 1, became ready with zero consecutive
failures. An idle attempt committed at 19:26:14 UTC on 2026-10-26. Querying the
watch's public events endpoint before the hold found zero stored events.

App-hosted sessions expose no `extensions_reload` tool here. The
[steward accepted](https://github.com/lossyrob/telex/issues/201#issuecomment-6425143128)
the supported generated pull fallback for the real-root criterion. Push was
not exercised during these initial runs. Each participant used `copilot fallback prepare` and exactly one
generated detached launcher per run; no custom polling or relay was added.

| Participant | Generated fallback runs | Observed failure health |
| --- | --- | --- |
| Owner | `1793042657859-83154eafc09756e3`; one re-arm `1793042722283-01fcd5d2b4bc7687` | pull, unattended, zero live waiters, exit 7 |
| Campaign root | `1793042572542-c8903866107167a1`; one re-arm `1793042640067-5700450788a59922` | pull, unattended, zero live waiters, exit 7 |
| Wait coordinator | `1793042546147-82d8d1bea621074e`; no re-arm | pull, unattended, zero live waiters, exit 7; later detached by its owner |

All reported `backend-unavailable`: recovery grace expired while fetching wait
candidates because a backend query or reconnect attempt timed out. Status
queries continued to work. No authentication/PIM cause or fleet-wide prevalence
is established. The configured idle timeout was 1,800,000 ms; the failure output
did not emit a numeric recovery-grace value. The owner's first before/armed
health snapshot was not captured, so successful preparation is not presented
as proof of sustained attendance.

The owner's original status artifact was written at `1793042689524` ms and has
SHA-256 `fe86bb98d25501033d675cd16c61c546b552e4487863a8469e7ab41734c49cae`.
The re-arm status was written at `1793042760337` ms and has SHA-256
`c3ce26f269093add6ce54ea6ed2c5c71394d1067cd446804177cf1047f039f07`.
Both exit artifacts contain 7 and have SHA-256
`10159baf262b43a92d95db59dae1f72c645127301661e0a3ce4e38b295a97c58`.
Original private artifacts are preserved; only sanitized facts are published.

No real detector event was deliberately staged to an unattended root. At that
hold, the background-shell case had not run, and real event delivery, fresh App
confirmation, member ring/reply and the root's integrated status view remained
unproven. The later real episodes below provide that separate evidence.

## Accepted #224 build and remaining readiness checks

Campaign authorized a Telex-only upgrade from exact merge
`c4f6ac8c9c43f82aeb861d46ee44f2f7160f275a`, retaining the initial dogfood build
as rollback. Its [main CI](https://github.com/lossyrob/telex/actions/runs/40414741263)
and [Docs](https://github.com/lossyrob/telex/actions/runs/40414741325) both
completed successfully before the build or installation began. No jobs were
rerun for this sequence.

The source was exported directly from that commit, without moving the gate
branch. The source archive SHA-256 is
`99901a45fba22b6520cfcc67f6940eae0384b465faef132b6476ed89d5d8ca5e`.
Only Telex was built, with Entra support, isolated build roots and an explicit
build id equal to the full merge SHA. The active tag is now
`v0.3.0-gate201-c4f6ac8c`; the launcher, selected binary hash and rollback
selector were checked independently.

The supported installer drained the predecessor and invoked successor intent
reconciliation. It reported one recoverable intent, zero restored intents and
zero failures in that pass. No manual daemon restart, drain bypass, timeout or
authentication change was made. The installer repeated its running-launcher
refresh warning; the retained launcher nevertheless selected the new build.

Before this upgrade, Campaign reported that an App restart loaded the
provisioned bridge, `copilot resume` reported `attended_push`, and test message
4191 arrived as a pushed turn. This establishes reported transport evidence,
not receipt of a real detector event. After the upgrade, read-only daemon
status reported the root intent as push/unverifiable with no member and
`per_intent_timeout`. Campaign has the new build identity and owns the required
fresh root check, optional supported resume and one push test.

A separate read-only Watcher observation at 23:05:43 UTC on 2026-10-26 returned
exit 1, `runtime-not-running`. The original runtime process was absent. Its
retained log contains a Postgres connection-ended diagnostic; neither the stop
time nor cause is established. This observation is not attributed to the
upgrade or to #225's distinct `runtime-failed` case. The Watcher binary and
registry were retained, with no restart or registration change attempted.
Campaign and the Watcher coordinator received the blocker and the need for
fresh restart approval.

Campaign subsequently authorized restarting the same Watcher binary and registry,
without upgrading Watcher or changing the registration. The original log was
left intact and copied before restart; the new process writes a distinct log.
At 23:07:16.985 UTC the new runtime
`watcher-runtime-f56976b5-2d5e-4090-8732-093843616f11` started. Its readiness
snapshot reported running, production-ready, restart-reconciled, a fresh
heartbeat, sender ready and no diagnostics. Registry and reconciled revisions
both remained 1; the active watch reported ready, zero failures, and allocated
and committed event sequences of zero. The registration still pins the original
detector build, which includes #216; the installed transport now selects
`c4f6ac8c`. Campaign and the coordinator received this restart evidence.
At this point staging remained held pending Campaign's fresh root test.

Raw post-upgrade artifacts remain private. The installer result has SHA-256
`45a2ffce280008b68bc3fac359b65b79602af6a1d322425fc98bad247fe7c4fc`;
the stopped-Watcher status has SHA-256
`e94498d35966c8acb15b92ee84cb00f07d210c844069383b375f33ff537af2dd`.
The original log and preserved copy share SHA-256
`8d6212f3df62965e512da59a09770307563c3333d2b994cc0e31415ec642c0b7`;
the restarted readiness status has SHA-256
`a1af4ae31c97d624a55133fa6ab30f7dd3ace739cf127f6624e6c10b88ce09ed`.
No real-event or background-shell case was staged during this sequence.

## Real detector episodes after the readiness gate

Campaign's fresh root push check passed on `c4f6ac8c`: test message 4192 arrived
after `copilot resume`, with lease epoch 4 and `attended_push/restored`. Its
first ack falsely failed and a retry returned already-consumed, consistent with
the separately tracked #231 ambiguity. Campaign then cleared staging.

The original real watch, registration revision 1, ran both cases with its
unchanged 30-second grace/cadence. The root received by push; the observed
owner's Telex station remained detached, so the root rang and received replies
through App `send_session_message`.

| Episode | Scheduled, durably stored event | Root action and member reply |
| --- | --- | --- |
| Ordinary active/quiet member | Sequence 1, message 4193, event `watcher-event-20cfbc78-4d2f-4611-97f6-44d2f17fd360`; observed 23:11:49.825 UTC, accepted 23:12:03.610 UTC | Root reported push arrival about 23:12:10 UTC and fresh `get_sessions_status` about 23:12:16 UTC: owner idle, not running, no input/plan wait. App ring woke the member, which replied and resumed validation. |
| Required active-idle member with background work | Sequence 2, message 4194, event `watcher-event-8127588f-caab-4814-a4c5-2ba546eadfc0`; observed 23:15:29.762 UTC, accepted 23:15:43.536 UTC | Snapshot reported `backgroundWork: unobserved`, degraded false, and no observed wake sources. Root reported push arrival about 23:15:50 UTC and fresh App inspection about 23:16 UTC: busy, not running, no input/plan wait. Root rang rather than assuming background work absent. Member replied with its actual shell wait and atomically parked. |

Both receipts show durable acceptance, occupied at acceptance, and no replay.
Exactly two event rows existed after both cases. Later scheduled attempts
committed idle without allocating another event: 23:12:31.463 UTC after the
first episode and 23:16:08.140 UTC after the second. Root ack retries returned
already-consumed after false errors. Both events were handled: disposition
2003 at server time 23:13:05.461 UTC and disposition 2004 at 23:16:47.906 UTC.
The first event's original root receipt had a conflicting local-clock label;
the root corrected it against the server `at_ms` before this report used it.

The root reports continuously attended push from its post-upgrade resume
through both events, with no attendance gap, no backlog and zero pending
unconsumed messages afterward. No root pull waiter was armed: these were push
deliveries, not uncovered pull windows. The initial failed-pull period had no
stored real events.

The root's status view joins the delivered snapshot with fresh App inspection
and the same watch's public health:

| View field | Ordinary episode | Background episode |
| --- | --- | --- |
| Membership | Owner active; coordinator parked | Owner deliberately active for this case; coordinator parked |
| Classifier | Idle, agent-stop-allowed; activity 23:10:42.658 UTC | Idle, agent-stop-allowed; activity 23:14:33.527 UTC |
| Observed wake sources | Armed watches empty, deferred 0, unacked 0 | Armed watches empty, deferred 0, unacked 0 |
| Background certainty | Unobserved for both members | Unobserved for both members; actual shell independently verified |
| Fresh App state before ring | Idle, not running; no input or plan wait | Busy, not running; no input or plan wait |
| Monitoring observation | Event observed 23:11:49.825 UTC | Event observed 23:15:29.762 UTC |
| Root delivery after handling | Attended push, no backlog, pending 0 | Attended push, no backlog, pending 0 |

An owner-side public health read after both rings reported ready, zero failures,
sender ready, registration revision 1 and allocated/committed sequence 2, with
last success at 23:17:54.588 UTC. This fills the watch-health input that the root
did not independently re-read in its receipt. The corrected root packet's
SHA-256 is `d10f1f8f8dab124ffa6fb1331e23ffe6459cc3851feca506874d60a094361aa2`.
The original packet is preserved separately; no raw message bodies are
published.

The background shell performed bounded hash work, not delivery polling. It
started at 23:13:43.0317179 UTC and had completed 20 of 180 work units at
staging. After the root ring, at 23:16:56.286 UTC, the same PID and exact
process-start identity remained alive with the same run id, 96 units complete
and no completion marker. No stop, restart or signal was sent. The deliberately
active membership was the specific #197 validation case; after the ring the
member followed normal guidance by parking on the natural-completion
notification. The shell completed naturally at 23:19:45.968 UTC, exit 0,
after 362.348 seconds. All 180 work units had the original run/PID identity,
strictly increasing timestamps, consecutive sequence numbers and independently
verified SHA-256 results. The owner resumed active work on that completion;
no bounded background helper was left running.

The raw two-event snapshot has SHA-256
`564de3da96cb85ea268a244559d6b108d3fc915795221a57d78817464a95903f`.
The post-ring background snapshot has SHA-256
`9957d35b77aa9f04691475667d9e1f3bb28a2a5c9b80f542f7705fe076a3b851`.
The complete work log has SHA-256
`7aea597768e6be63d2945a805055cf85790aef5ffb4973d6efb41de3921a0762`;
its completion artifact has SHA-256
`c5acefa0ddeaee51e0f4090793cefc26e4860976a095b6aa5975fcbd210d5e8c`.
The final event query after natural completion still returned exactly the same
two accepted rows.
Raw event bodies, paths and work artifacts remain private.

The [steward disposition](https://github.com/lossyrob/telex/issues/201#issuecomment-6429667078)
classifies two recovered `detector-degraded` observations as expected behavior,
not defects, discovered work or dependencies:

| Degraded attempt and start (UTC) | First subsequent successful attempt | Event timing after recovery |
| --- | --- | --- |
| `attempt-94a18160-8933-4188-b0a5-7031092db243`, 23:10:39.889 | `attempt-1aa69acd-8643-4ead-8f9b-2c60cfc48e85`, 23:11:14.892, committed idle | Episode 1 emitted 23:11:49.825: 34.933 seconds after recovery, 67.167 seconds after its idle activity marker. |
| `attempt-900e84c2-44d4-4c98-b49c-71d22fa3bcb2`, 23:14:19.915 | `attempt-dac3a782-ab08-4cb9-97ac-b608d8026858`, 23:14:54.832, committed idle | Episode 2 emitted 23:15:29.762: 34.930 seconds after recovery, 56.235 seconds after its idle activity marker. |

Neither degraded attempt committed state or sent an event. Both retained
attempt records have the canonical null prior-state hash,
`sha256:74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b`:
no durable detector episode was open. Their idle markers followed the failed
starts by 2.769 and 13.612 seconds, and their recorded `firstSeenAt` values are
the successful recovery observations above.
Before the second gap, episode 1 had already emitted and been handled; there was
no outstanding un-emitted event for that earlier episode. Active membership
alone is not evidence that quiet grace was already running.

The next successful observations followed the failures by 35.003 and 34.917
seconds. Each event then emitted on the next observation strictly beyond the
30-second grace (4.933 and 4.930 seconds beyond grace). These are measured
end-to-end waits, not a controlled estimate of extra delay caused by a failed
observation. No missed or duplicate event was observed. The isolated
alternating-read case supplies the controlled bounded-alert proof. Causes
remain unknown because safe output omits diagnostic text; neither gap is
attributed to the daemon restart or another component.

## Installation changes and undo

Each change was separately authorized for the workstation dogfood. No existing
directory ACLs or permissions were changed.

| Change | Undo or preservation |
| --- | --- |
| Built current-main Telex and Watcher with Entra support | Build artifacts are separate from the prior installed version. |
| Changed the installed selector from v0.1.2 to `v0.3.0-gate201-9e971fd9` with `telex upgrade --from` | `telex rollback --version v0.1.2` with the same install root restores executable selection. The old version cannot read the already-existing schema-v3 backend; rollback is not a schema downgrade. |
| After accepted #224 and green exact-merge CI, changed the selector to `v0.3.0-gate201-c4f6ac8c` through the selected predecessor's supported upgrade | `telex rollback --version v0.3.0-gate201-9e971fd9` with the same install root. The prior executable and its original hash remain intact. |
| Started a new Watcher with a new registry and retained exact binary copy | Use that exact executable and registry with `shutdown`; retain registry and receipts. Do not stop another runtime. |
| Separately authorized restart of that unchanged Watcher and registry after its process was found absent | Preserve the original log/status and the new log; use the exact executable/registry with `shutdown` only when authorized. No registration edit or Watcher upgrade was made. |
| Registered a new fixed-route stall watch and send-only sender | The root manages its watch; preserve its id, registry and history through pause/remove. |
| Attached dedicated participant receive addresses and generated one-shot fallbacks | Each owner stops/detaches only its own attendance; do not bulk-ack, reset history or replace addresses to hide the failure. |

The new candidate CLI initially refused the legacy installed predecessor
manifest before drain. Retrying the same approved upgrade through the existing
installed CLI succeeded. That CLI warned that it could not refresh its own
running launcher image; the existing launcher nevertheless resolved the new
current selector. No incompatible-manifest check or drain was bypassed.

The runtime's binary and registry are retained for the campaign's authorized adoption. Its private
installation record includes absolute paths, environment, hashes, runtime id,
registry, profile, sender and exact fallback/undo commands.

## Failure dispositions

| Finding | Disposition |
| --- | --- |
| App first-time bridge loading / stall-guide push prerequisite | [#223](https://github.com/lossyrob/telex/issues/223); initial pull was accepted. Later startup discovery loaded the root bridge, and the final real cases exercised push. No live-loader capability is claimed for this owner. |
| Guide assumes an existing Watcher setup but workstation has none | [#222](https://github.com/lossyrob/telex/issues/222); operator authorized this installation separately. |
| Generated fallback exhausts candidate-fetch recovery and leaves the root unattended | [#224](https://github.com/lossyrob/telex/issues/224); the [steward made it upstream of #201](https://github.com/lossyrob/telex/issues/201#issuecomment-6425382009). Accepted fix merged in [#227](https://github.com/lossyrob/telex/pull/227) and was installed after exact-merge CI. The later real-root proof uses push; pull was not re-tested. |
| Ack reports an error after consumption | [#231](https://github.com/lossyrob/telex/issues/231); root recorded false failures, then explicit idempotent retries returned already-consumed. Both real events reached terminal handled disposition. |
| Retained Watcher found no longer running | Routed to [#234](https://github.com/lossyrob/telex/issues/234), not a #201 dependency or pass-criterion failure. Restarted under separate authorization; original log/status preserved. Stop time and cause are unattributed; this pre-#225 binary establishes no cause chain. |
| Two recovered detector-degraded observations | [Steward disposition](https://github.com/lossyrob/telex/issues/201#issuecomment-6429667078): expected behavior, report only, not a defect or dependency. Exact attempts, recovery/episode timing and unknown cause recorded above. |

Campaign accepted the Windows behavioral packet; closing-PR acceptance
additionally requires exact-head review, design inspection and green CI.
The installed binaries, runtime and registry remain retained. No further
timeout/authentication/runtime changes or fallback retries were made.
