# Copilot stall-detection gate: Windows evidence

The [workstream gate](https://github.com/lossyrob/telex/issues/201) is **not
passed**. Isolated scheduled-host coverage passes, but the real campaign root
cannot remain attended on the selected Postgres backend. The background-shell
and real root confirm/ring cases are held pending
[#224](https://github.com/lossyrob/telex/issues/224). Staged evidence does not
replace those cases.

## Build and scope

The workstation dogfood uses Telex 0.3.0 and Watcher built with Entra support
from main `9e971fd9ee877a20d13c97ad59b25d477b5c6ad0`, which includes the
classifier, detector, guidance and bounded-inbox-query changes. The retained
binaries have these SHA-256 identities:

| Binary | SHA-256 |
| --- | --- |
| Installed Telex | `fe3d09c80afe0ba294d9f77936af6df280f196e615d1194b48b374e23b7d761c` |
| Running Watcher | `343b9d9509d72cb28dacd9be0f9fe3ad51822d079e8e44b5bd07fa624b24521a` |

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

The final local staged target completed with four passing tests and one ignored
subprocess entrypoint, which its parent test executes. Raw output SHA-256:
`af6b1ce39a3d457c2f150b9f2739fb852a26dae7dfe6be15cb79ab6eb89b8f62`.
The existing 20 detector unit tests also passed locally. These are supporting
local observations, not exact-head hosted CI or final independent review.

On Windows, after building the workspace binaries, reproduce the new target:

```text
cargo build --locked --workspace --bins
cargo test --locked -p telex-watcher --test stall_gate
```

The fixture uses SQLite; the real installation additionally requires both
packages' `entra` features. The source identity and hosted-run evidence for a
final PR must be recorded separately when this gate can resume.

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
not exercised. Each participant used `copilot fallback prepare` and exactly one
generated detached launcher per run; no custom polling or relay was added.

| Participant | Generated fallback runs | Observed final health |
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

No real detector event was deliberately staged to an unattended root. The
background-shell case has not run. Real event delivery, fresh App confirmation,
member ring/reply and the root's integrated status view remain unproven. The
staged snapshot/health data demonstrate the available view inputs, not the
completed real-root view.

## Installation changes and undo

Each change was separately authorized for the workstation dogfood. No existing
directory ACLs or permissions were changed.

| Change | Undo or preservation |
| --- | --- |
| Built current-main Telex and Watcher with Entra support | Build artifacts are separate from the prior installed version. |
| Changed the installed selector from v0.1.2 to `v0.3.0-gate201-9e971fd9` with `telex upgrade --from` | `telex rollback --version v0.1.2` with the same install root restores executable selection. The old version cannot read the already-existing schema-v3 backend; rollback is not a schema downgrade. |
| Started a new Watcher with a new registry and retained exact binary copy | Use that exact executable and registry with `shutdown`; retain registry and receipts. Do not stop another runtime. |
| Registered a new fixed-route stall watch and send-only sender | The root manages its watch; preserve its id, registry and history through pause/remove. |
| Attached dedicated participant receive addresses and generated one-shot fallbacks | Each owner stops/detaches only its own attendance; do not bulk-ack, reset history or replace addresses to hide the failure. |

The new candidate CLI initially refused the legacy installed predecessor
manifest before drain. Retrying the same approved upgrade through the existing
installed CLI succeeded. That CLI warned that it could not refresh its own
running launcher image; the existing launcher nevertheless resolved the new
current selector. No incompatible-manifest check or drain was bypassed.

The new runtime is retained for the campaign's authorized adoption. Its private
installation record includes absolute paths, environment, hashes, runtime id,
registry, profile, sender and exact fallback/undo commands.

## Failure dispositions and remaining gate

| Finding | Disposition |
| --- | --- |
| App first-time bridge loading / stall-guide push prerequisite | [#223](https://github.com/lossyrob/telex/issues/223); supported pull accepted for this gate, push remains untested. |
| Guide assumes an existing Watcher setup but workstation has none | [#222](https://github.com/lossyrob/telex/issues/222); operator authorized this installation separately. |
| Generated fallback exhausts candidate-fetch recovery and leaves the root unattended | [#224](https://github.com/lossyrob/telex/issues/224); evidence routed to Local. The [steward made it upstream of #201](https://github.com/lossyrob/telex/issues/201#issuecomment-6425382009). |

There is no closing-PR or operator-readiness claim. Resume the real cases only
after the steward identifies a repaired or otherwise supported root-delivery
path: an accepted #224 resolution, or environmental closure with evidence that a
supported wait stays armed. No further retries or timeout/auth/installation
changes are part of this hold. On resume, prepare and arm the root freshly
before staging an event. The final field report on #201 must state the actual root/member delivery
modes, event/store counts, any acceptance while no wait was armed, fresh App
confirmation, ring/reply channels, background-process continuity and status
view evidence.
