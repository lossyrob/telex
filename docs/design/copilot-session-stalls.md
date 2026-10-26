# Copilot session-group stall detection

This records the design agreed with the operator on 2026-10-25 in
[issue #197](https://github.com/lossyrob/telex/issues/197), with classifier
details from [issue #198](https://github.com/lossyrob/telex/issues/198).
[ADR 0056](DECISIONS.md#0056---copilot-stall-detection-uses-a-binary-owned-conservative-local-log-classifier)
records the architectural decision. The classifier and group detector are
implemented here; agent guidance and workstream validation remain separate items.
This chain does not gate the first Watcher dogfood release.

## Outcome and terms

A group can stop progressing when all its sessions are idle and nothing will
wake them. The root should receive one snapshot per stall episode, confirm
the stall through the App, diagnose it and ring the affected members. The
root hosts a status view over the same observations so the operator can see
who is watching what.

A **group** is a set of same-machine, same-user Copilot CLI sessions working
toward one outcome. Its **root** owns membership and receives stall events;
any session can be a root. Members are `active`, `parked`, `operator-held`
or `done`. Only active members count. Planned waits, including App session
automations and operator gates, belong in parked or operator-held state.

## Local session classifier

```text
telex --json copilot session-state --session <local-copilot-cli-id>
```

The id is explicit, 1-128 ASCII letters, digits, hyphens or underscores. It
names one directory under the current user's `~/.copilot/session-state/`;
it is not an App project/session id, path, cloud id or remote reference.
The command reads `events.jsonl` without writes, locks, backend access or
daemon interaction. No alternate log root is exposed.

The result is:

```json
{
  "session_id": "local-session-id",
  "state": "idle",
  "reason": "agent-stop-allowed",
  "activity": {
    "id": "last-non-accounting-event-id",
    "timestamp": "2026-10-25T20:00:02Z"
  }
}
```

`state` is one of the five values below. `reason` is a safe diagnostic code,
never log content or an operating-system error containing local paths.
`activity` identifies the latest non-accounting event considered by the
classifier, or is null when evidence is unavailable or malformed. The
detector must use this marker to notice activity between polls, including a
busy-to-idle cycle; usage accounting alone does not reset a stall episode.
The command exits successfully for an observation of unknown. Invalid CLI
arguments are errors. `--text` prints the session id, state and reason.

| State | Recognized evidence |
|---|---|
| `busy` | A user message, assistant turn start, or a tool start without its matching completion. Assistant messages and turn ends preserve an already-observed busy turn. Tool completion never establishes idle. |
| `idle` | Root `hook.end` with `data.hookType: "agentStop"`, a nonempty hook invocation id, `data.success: true` and `data.output.decision: "allow"`, followed only by recognized usage accounting. Outstanding tools, pending stop hooks or a blocked/failed stop prevent idle. |
| `waiting-on-operator` | A tool start with `data.toolName: "ask_user"` whose `data.toolCallId` has no matching completion. Other parallel tools completing do not clear it. |
| `not-loaded` | A root `session.shutdown` with no later resume or contrary activity. It is separately represented because an App message can resume the session. |
| `unknown` | Missing/unreadable log, unavailable home, incomplete or malformed tail, insufficient evidence, resume without subsequent activity, or an unrecognized event pattern. Unknown is never idle. |

Events require a string type, nonempty id and timestamp, and object data.
Tool starts/completions pair by tool call id, not tool name or adjacency.
Child-agent stop hooks never establish root idleness. The recognized
post-stop bookkeeping is `session.usage_checkpoint` and
`session.usage_record`; there is no wildcard that trusts all `session.*`
events. Unsupported activity invalidates prior idle evidence. Later explicit
activity or a successful allowing stop can establish a new state, but a
malformed retained record makes the observation unknown.

If a retained root stop start is followed by newer root user activity, turn
start or a lifecycle transition, its invocation id is invalidated. A later
completion of that attempt returns `unknown` (`agent-stop-invalidated`) and
cannot establish idle. A distinct subsequent allowing stop can establish idle.
Tracking is limited to the same suffix; it adds no historical scan.

The reader seeks to at most the final **256 KiB** at a snapshotted file
length. When truncated at the front it discards the first potentially
partial record. It never retries farther back. A partial final record,
invalid UTF-8 or a concurrent truncate is unknown; appends after the snapshot
are outside that observation. If the relevant signal or a needed tool start
is outside the suffix, the result can be unknown. This trades detection
coverage for bounded work and fewer false stall claims.

### Evidence and limitations

The event log is a private Copilot CLI format with no documented compatibility
contract. Fixtures are synthetic and pin the observed control fields, not
private message contents. Changes in log shape can increase unknown results
and require one binary update, rather than changes to every agent's script.

An explicitly allowing successful root stop is assumed to end the agent loop;
this producer guarantee is **unverified**. Overlapping root hooks and their
scheduling relative to newer messages were not observed. As confirmed by the
[design steward](https://github.com/lossyrob/telex/pull/202#issuecomment-6403276332),
the stop remains decisive evidence when its start or other pending starts are
outside the bounded suffix. Front clipping can therefore change a retained
pending-tool/hook observation to idle when an allowing stop remains in view.
Boundary regressions pin that heuristic at 262144 and 262145 total bytes; they
prove classifier behavior, not producer scheduling. Explicit invalidation
handles contrary ordering only when that ordering is retained in the suffix.

A same-user, read-only sample of the 20 most recently modified local logs on
2026-10-25 produced 8 idle, 2 busy, 1 waiting-on-operator, 2 not-loaded and
7 unknown results. All seven unknown reasons were unrecognized events;
none reported missing state evidence at the 256 KiB bound in that snapshot.
This is one convenience sample, not an estimate of fleet coverage or a
guarantee that the chosen bound retains every signal. Tests explicitly cover
unknown when the signal falls outside the bound.

A finite asynchronous PowerShell probe in the owner's real log showed
`tool.execution_complete` as soon as the shell launched, before the process
finished. Thus unmatched starts cannot prove that no background shell exists.
The sample contained no background-agent launch pair; background-agent
completion semantics remain unverified. The classifier does not parse tool
result text or invent a background-work signal. Wake-source completeness
needs separate evidence in the detector/validation items; the root's App
confirmation remains necessary.

## Group stall policy

The detector reports a stall only when, for longer than the grace period:

1. At least one member is active.
2. Every active member is idle, including the separately classified
   not-loaded case; none is busy, waiting on the operator or unknown.
3. No active member has an unacknowledged/deferred Telex message, an armed
   Watcher watch targeting it, or an unfinished background shell/agent.

Waiting on `ask_user` is not a stall; the App already signals required input.
An unknown active member makes the detector degraded rather than stalled.
App session automations do not appear in the log; roots record those planned
waits in membership. An armed watch counts as a wake source even if it
depends on another idle member: the detector cannot infer a watch's meaning.

The root owns a group directory with one file per member, creates/removes
membership files, and members update only their own state. The detector
reads an explicit absolute group directory; one file per member avoids a
shared concurrent-writer file. The root chooses the directory and owns file
creation, removal, session identity and address. Each `<session-id>.json` is:

```json
{
  "schemaVersion": 1,
  "sessionId": "local-copilot-cli-id",
  "address": "project:example/worker:one",
  "state": "active",
  "reason": "Optional explanation of the current membership state"
}
```

Members update only their own `state` and optional `reason`, preserving the
other fields. Write a temporary non-JSON file in the same directory and
atomically replace the member file. Files are strict JSON with no duplicate or
unknown fields. The filename must match the validated local session id.
Addresses must be distinct, nonempty, free of whitespace/control characters,
and at most 256 UTF-8 bytes. Reasons are at most 256 UTF-8 bytes. The detector
admits at most 32 members, 4096 bytes per record and 64 directory entries
(including temporary files). Symlinks and directories with a `.json` extension
are rejected. The detector rereads membership after observation; a change
during the read is degraded. Missing, malformed or over-limit membership is
degraded, never an empty group.

Inactive members remain in event snapshots but do not require session logs or
wake-source queries. An empty group, or one with no active members, is
successfully evaluated as non-stalled and clears the prior episode. If the
root includes itself as active, its own armed monitoring watch counts as a
wake source; there is no special exemption for circular watches.

## Detection and distribution

The root registers one ordinary Watcher v2 watch per group with a fixed route
to its Telex address, a detector command, group parameters and grace period.
Each run reads membership, classifies active members and collects wake
sources from Telex status and Watcher management. The detector fingerprints
the snapshot, retaining the fingerprint and first-seen time in opaque state.
After the same stalled fingerprint exceeds the grace period, it emits one
event (proposed kind `copilot.group.stalled`) with the snapshot. It emits
no duplicate for that episode; activity or membership changes reset it.

Telex delivers the event as a turn. Before acting, the root confirms using
the App's `get_sessions_status`, because the local log is not authoritative.
The detector only observes and alerts. Consequential action belongs to the
root, and Watcher remains provider-neutral and unchanged.

### Detector interface

`telex copilot stall-detect` reads one strict Watcher v2 request on stdin and
writes exactly one v2 result on stdout, regardless of the global text/JSON
presentation option. It uses the same parser, canonicalizer and validators as
Watcher: `telex::watcher_protocol`, re-exported at
`telex_watcher::protocol_v2`. The normative wire contract remains
[watcher.md](watcher.md).

The watch's opaque `parameters` are a strict object:

| Field | Meaning |
|---|---|
| `groupDirectory` | Absolute path to the root-owned membership directory. |
| `backendProfile` | Explicit configured backend name used by this group. |
| `watcherExecutable` | Absolute path to the trusted local `telex-watcher` binary. |
| `watcherRegistry` | Absolute path to the group's Watcher registry. |
| `graceSeconds` | Required grace, 1-86400 seconds. |
| `pollIntervalSeconds` | Expected watch cadence, 1-86400 seconds; default 30. Set it to the registration's interval. |

One group observes one configured backend and one Watcher registry. Register
the detector with its fixed target set to the root's address and its backend
set to the same profile. Backend profiles must be available to the detector
through the controlled launch environment; allowlist `TELEX_HOME` and, when
used, `TELEX_CONFIG` and backend credential environment names. Paths in profile
configuration should be absolute. The detector deliberately does not apply
ambient `TELEX_DB` or global CLI database overrides to this explicit profile.
Profiles naming the same effective store also match watches targeting a member.

The detector opens Telex using `build_readonly`, which skips schema
initialization and uses a read-only SQLite connection. It reads aggregate
unconsumed primary-recipient counts and at most 1025 recent inbox entries per
active member to find deferred dispositions, including messages already
transport-consumed. If the suffix is full and contains no known message wake
source, the absence of older deferred work cannot be established and the run
is degraded. Message bodies and tool result text never enter the event snapshot.
Postgres uses the existing read-only consumer connection path with SELECT
queries; a SELECT-only database role is needed for a credential-level guarantee.

Watcher observations use only its public
`--json --registry <path> list --active` management command. Active watches
targeting a member at the selected store count even when their source depends
on another idle member. The command has a five-second deadline and 256 KiB
stdout/64 KiB stderr bounds. A failed, missing, malformed or unsupported
inventory degrades the observation; it never establishes no watches. The
detector starts no daemon or runtime and writes no membership or session files.
All asynchronous observations share a 15-second budget; registration timeout
must allow that budget plus command startup. Group/session file reads and the
underlying database queries still rely on Watcher's process timeout for a hard
host-level bound.

Background shells and agents are **unobserved** by this implementation.
Every member snapshot explicitly carries `"backgroundWork": "unobserved"`;
this is not proof of absence. This implements the steward's
[background-work decision](https://github.com/lossyrob/telex/issues/197#issuecomment-6402758809).
Planned background waits belong in parked membership.

### Episodes, degraded observations and advisories

Opaque state version 1 contains `fingerprint`, `firstSeenAt`, `lastObservedAt`,
`gapRestarted` and `emitted`, with times in Unix milliseconds. The fingerprint
covers parameters, activation time, sorted membership, classifications,
non-accounting activity markers and wake-source observations. Poll time and
usage accounting do not affect it. Non-stalled successful observations clear
the episode. A membership/evidence change starts a new episode, including an
idle-to-busy-to-idle cycle between polls whose final activity marker changed.

A known quiet group emits `copilot.group.stalled` only after the unchanged
candidate has persisted for **strictly more** than `graceSeconds`. Freshness
is twice `pollIntervalSeconds` (60 seconds by default). An observation gap
longer than freshness restarts grace at most once per unchanged episode.
Later gaps cannot keep alternating degraded/successful runs from alerting
indefinitely. A backward clock adjustment restarts elapsed-time measurement
without clearing an already-emitted event. Wall-clock time is not monotonic.

`degraded` never advances opaque state. Under the steward's
[quiet-unknown decision](https://github.com/lossyrob/telex/issues/197#issuecomment-6418380723),
unknown members instead produce one `copilot.group.attention-needed` advisory
when every active member is idle/not-loaded/unknown, no wake source is observed,
and every unknown member has a valid evidence timestamp strictly older than
grace. This includes a quiet `session.error`, `session.start` or any other
unsupported event; the classifier still reports unknown. The advisory does
not need a second observation grace after the evidence has already aged.
An unknown with no retained evidence timestamp, or one with invalid/future
evidence time, cannot establish this trigger and remains degraded. Other
unknown runs, including unchanged already-notified unknown episodes, are degraded.

Both event kinds share the fingerprint and emitted bit. Watcher commits the
event-producing `nextState` only after durable Telex acceptance; an unresolved
send cannot suppress a notification by prematurely committing `emitted`.
Re-evaluation with uncommitted prior state offers the episode again, while
normal delivery recovery retains Watcher's exact staged event identity.

The event body is a JSON snapshot with `schemaVersion: 1`, `groupDirectory`,
`degraded`, `members`, `firstSeenAt`, `observedAt` and `fingerprint`. Each member
contains its membership record, optional classifier result, optional message
counts/watch ids, and explicit background uncertainty. Advisory bodies and
metadata set `degraded: true` and never claim a stall. Both require root App
confirmation before any response. Oversized snapshots produce a diagnostic
and a degraded result instead of violating the v2 output limits.

Both the classifier and detector live in the versioned binary under
`telex copilot`. The detector speaks Watcher detector protocol v2 and is
registered like any other detector. Agent guidance belongs in binary-owned
`telex copilot skill` instructions and a guide page; the plugin remains a
thin bootstrap. The status view is root behavior, not a new product service
or session that could itself stall.

The implementation chain is classifier
[#198](https://github.com/lossyrob/telex/issues/198), detector/membership/event
[#199](https://github.com/lossyrob/telex/issues/199), guidance
[#200](https://github.com/lossyrob/telex/issues/200), then workstream validation
[#201](https://github.com/lossyrob/telex/issues/201). The final gate requires
exactly one event per staged/real stall episode, no stall for busy,
operator-waiting or watch-armed members, App confirmation and member wakeup,
and a status view over the same observations.

## Open questions

- Whether a watch dependent only on another idle member should count as a
  wake source, and how to detect such internal deadlocks.
- Whether the root should also be told when all active members await input.
- How unknown observations with no usable evidence timestamp should eventually
  reach the root. Quiet unknowns with retained old evidence are covered by
  `copilot.group.attention-needed`.
