# Copilot session-group stall detection

This records the design agreed with the operator on 2026-10-25 in
[issue #197](https://github.com/lossyrob/telex/issues/197), with classifier
details from [issue #198](https://github.com/lossyrob/telex/issues/198).
[ADR 0056](DECISIONS.md#0056---copilot-stall-detection-uses-a-binary-owned-conservative-local-log-classifier)
records the architectural decision. The classifier is implemented here; the
group detector, guidance and workstream validation remain separate items.
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
item defines the exact schema and location; one file per member avoids a
shared concurrent-writer file.

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
- Whether persistent unknown state should eventually alert the root rather
  than only making the detector degraded.
