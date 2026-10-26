# Watch a Copilot session group for stalls

A local group can stop progressing when all its active members are idle and
nothing will wake them. Register one Watcher v2 watch to notify the group's root,
then let the root confirm through the App and ring the affected members.
The detector is shipped as `telex copilot stall-detect`; agents supply group
membership, a root address and a grace period, rather than writing a detector.

This works only for same-machine, same-user Copilot CLI sessions. The classifier
reads bounded local log tails conservatively; its result is not authoritative App
state. Cloud and remote sessions are outside this group. The installed binary
prints the version-matched instructions:

```sh
telex copilot skill
telex --json copilot session-state --session <local-session-id>
```

The classifier returns `busy`, `idle`, `waiting-on-operator`, `not-loaded` or
`unknown`, with a reason and optional activity marker. Unknown is never idle.
The session id is the local CLI id (`COPILOT_AGENT_SESSION_ID` in that member),
not an App session/project id or a path. Keep a separate mapping to the App ids
the root uses for confirmation and wakeup.

## Create the group

The root creates an absolute, private directory and one
`<local-session-id>.json` file for each member it launches:

```json
{
  "schemaVersion": 1,
  "sessionId": "local-session-id",
  "address": "project:example/worker:one",
  "state": "active",
  "reason": "Working on the assigned item"
}
```

The root owns membership creation/removal, session ids and distinct Telex
addresses. Members update only their own `state` and optional `reason`. Preserve
the other fields, write a temporary non-`.json` file beside the record, and
atomically replace the record. Do not rewrite a shared group file or truncate a
member file in place.

Records are strict JSON with no duplicate or extra fields. The filename must
match `sessionId`, which is 1-128 ASCII letters, digits, hyphens or underscores.
The detector accepts at most 32 members, 4096 bytes per record, and 64 total
directory entries including temporary files. Address/reason values are at most
256 UTF-8 bytes each. Addresses must be distinct and contain no whitespace or
control characters. Membership entries must be regular files, not symlinks.

| State | Meaning |
| --- | --- |
| `active` | Expected to make progress; only active members count toward a stall. |
| `parked` | Waiting on an identified wakeup: a watch, App automation, background shell or background agent. Put the wakeup in `reason`. |
| `operator-held` | Waiting for an operator decision or gate. Identify it in `reason`. |
| `done` | Assigned work is complete. The root removes the file when the member leaves. |

Before ending a turn to wait for a background shell or agent, the member sets
itself parked. On wakeup to resume work it sets itself active. If rung while
legitimately waiting, it replies with what it is waiting for and stays parked.
The detector reports `backgroundWork: "unobserved"` for every member: neither
the classifier nor App idleness proves background work is absent.

For example, a member can park itself with **PowerShell 7**:

```powershell
$ErrorActionPreference = 'Stop'
$memberFile = Join-Path $groupDirectory "$env:COPILOT_AGENT_SESSION_ID.json"
$member = Get-Content -LiteralPath $memberFile -Raw | ConvertFrom-Json -AsHashtable
$member.state = 'parked'
$member.reason = 'Waiting for the background build completion notification'
$temporary = Join-Path $groupDirectory "$([guid]::NewGuid()).tmp"
$member | ConvertTo-Json -Compress |
    Set-Content -LiteralPath $temporary -Encoding utf8NoBOM -NoNewline
[System.IO.File]::Move($temporary, $memberFile, $true)
```

Use the root-provided `$groupDirectory`. To resume, repeat the replacement with
`state = 'active'` and a current reason. A failed write/rename needs repair before
the member assumes its state changed; remove only its own abandoned temporary
file. The root may also be an active member. Empty groups and groups with no
active members are evaluated as non-stalled.

## Use the root's existing Watcher installation

The Watcher runtime must already be running, and the root's Telex address must
already receive turns through [Copilot push delivery](copilot-push.md). Obtain
installation values from the root's recorded runtime launch and registration
policy:

| Value | Source |
| --- | --- |
| `telex` executable | Absolute trusted installed binary path in the installation record. |
| `watcherExecutable` | Absolute trusted `telex-watcher` path used to run this Watcher. |
| `watcherRegistry` | Absolute registry path used by this runtime and by the registration command below. |
| `backendProfile` | Existing configured profile holding the members' Telex addresses; use it for both the registration and detector parameter. |
| Sender | Existing Watcher send-only address from the root's routing policy, distinct from member receive addresses. |
| `pollIntervalSeconds` | The registration's own interval, copied from one `$pollSeconds` value below. The detector default is 30 seconds. |

If the root does not know a value, it asks the operator through `ask_user` rather
than guessing paths, credentials or a default profile. Profile paths should be
absolute. The detector uses its explicit backend profile and ignores `TELEX_DB`
or global database overrides. All members belong to this one backend and one
Watcher registry; another registry's watches are not observed.

### Register the watch

Choose the group directory, root address and grace for this group. Use a new
permanent lowercase ASCII watch id (`[a-z0-9][a-z0-9._-]{0,127}`); removed ids
cannot be reused. This **PowerShell 7** template writes registration data, not a
wrapper or detector script. Replace the angle-bracket installation values from
the table above. The 30-second cadence is shared by the watch and detector; the
30-second timeout allows the detector's 15-second observation budget plus startup.

```powershell
$ErrorActionPreference = 'Stop'
$groupDirectory = '<absolute group directory>'
$rootAddress = '<root Telex address>'
$graceSeconds = 300
$telex = '<absolute installed telex executable>'
$watcher = '<absolute installed telex-watcher executable>'
$registry = '<absolute running Watcher registry>'
$backendProfile = '<existing configured backend profile>'
$sender = '<existing Watcher send-only address>'
$watchId = '<new permanent watch id>'
$pollSeconds = 30
$registrationFile = Join-Path $groupDirectory 'stall-watch-registration.txt'
$registration = @{
    schemaVersion = 2
    id = $watchId
    command = @($telex, 'copilot', 'stall-detect')
    intervalSeconds = $pollSeconds
    timeoutSeconds = 30
    backendProfile = $backendProfile
    sender = $sender
    target = $rootAddress
    environmentAllowlist = @('TELEX_HOME')
    parameters = @{
        groupDirectory = $groupDirectory
        backendProfile = $backendProfile
        watcherExecutable = $watcher
        watcherRegistry = $registry
        graceSeconds = $graceSeconds
        pollIntervalSeconds = $pollSeconds
    }
}
$registration | ConvertTo-Json -Depth 4 |
    Set-Content -LiteralPath $registrationFile -Encoding utf8NoBOM
& $watcher --registry $registry --json register $registrationFile
if ($LASTEXITCODE -ne 0) { throw 'Watch registration failed; inspect the error before retrying.' }
& $watcher --registry $registry --json show $watchId
```

`graceSeconds` is required, from 1 to 86400. `pollIntervalSeconds` is also bounded
to 1-86400 and must match the watch interval. The `.txt` suffix keeps the
registration out of the membership scan. Omitted `workingDirectory` captures the
registration command's current absolute directory.

Watcher controls the detector's environment. Allowlist `TELEX_HOME` and, if used,
`TELEX_CONFIG` and the profile's credential environment names. These values must
exist in the **runtime's launch environment**, not merely the registration shell.
Keep secret values out of parameters. The detector reads one Watcher v2 request
on stdin and writes one v2 result; it has no group/root/grace command-line flags.
On other shells, write the same v2 JSON fields and register the file through the
same management command; no detector script is needed.

## Check monitoring and show status

Use the same trusted Watcher binary and registry:

```powershell
& $watcher --registry $registry --json status $watchId
& $watcher --registry $registry --json health $watchId
& $watcher --registry $registry --json attempts $watchId --limit 5
& $watcher --registry $registry --json events $watchId --limit 5
& $watcher --registry $registry --json list --active
```

A registered watch does not prove successful observations. Inspect last attempt,
last success, health, diagnostic category and any pending operation. A stalled
group's unchanged known-idle/not-loaded snapshot must persist for strictly more
than grace. Busy members and members waiting on `ask_user` prevent a stall.
Unacknowledged/deferred Telex messages and active watches targeting a member are
wake sources. Exactly this detector's own watch id is excluded. Other watches
count even when their source depends on another idle member, so circular waits
can hide a stall.

The root's status view uses these same observations: group/watch id, membership
state/reason, classifier state/reason/activity time, message counts, targeting
watch ids, background uncertainty, and the last observation time/health. Pair them
with fresh App status when acting. Event snapshots can be stale; absence of an
event does not mean everyone is busy or monitoring is healthy. A view belongs to
the root, not a new always-running agent or service.

## Confirm, then ring

Both event kinds deliver a JSON snapshot in the Telex message body. Each member
contains `member`, optional `session`, optional `wakeSources`, and
`backgroundWork`; inactive members can have null observations. Verify the
registered sender, expected store and `metadata.watcher.watchId`, then ack and
dedupe by message id using the [normal receive procedure](copilot-push.md#receive-and-disposition).

**Call the App's `get_sessions_status` before acting on either kind.** Use the
root's local-id-to-App-id mapping; a local CLI id is not an App id. Check current
activity, questions, planned waits and completed work.

- **`copilot.group.stalled`:** diagnose the confirmed lack of progress, then ring
  the affected members with a specific next action or a request for their wait
  reason. Use the App's `send_session_message` to resume a not-loaded member.
  For an attended Telex member, `send` is also suitable; include a meaningful
  `--subject` and the root's explicit `--session`.
- **`copilot.group.attention-needed`:** inspect the named unknown members and
  their classifier reasons after App confirmation. Ask what stopped progress or
  what they are waiting for. This snapshot has `degraded: true` and **does not
  establish a stall**. Quiet unknowns with old evidence can trigger it, including
  unsupported error/start events. Unknowns without usable timestamps may stay
  degraded without an advisory.

A member rung during a legitimate wait replies with its wait reason and stays
parked. A member resuming work sets itself active. If the App shows progress or a
known gate, record that diagnosis rather than restarting work. Finish with the
appropriate terminal disposition (`handle`, `reject` or `close`); ack alone is
not workflow completion.

Both kinds share receipt-gated episode deduplication: one event per unchanged
episode. Activity or membership changes can produce a new episode; transport
redelivery still requires message-id deduplication. Do not reset detector state
just to obtain another reminder. Observation gaps longer than twice the poll
interval can restart grace once per unchanged episode.

## Recover degraded monitoring without losing history

An observation failure leaves the watch active/degraded, with no event or state
advance. Default safe management output shows `detector-degraded` but omits the
detector's diagnostic text. Check membership limits/format, local session ids,
backend/environment access and the running Watcher management endpoint. An
unchanged already-advised unknown episode can also remain degraded.

Inspect each active member's history using the same configured backend:

```powershell
& $telex --backend $backendProfile --address '<member address>' --json inbox --all
& $telex --backend $backendProfile export --address '<member address>'
```

The first command lists the newest 50 messages by default, including consumed
messages. Export includes older messages with disposition history. Review
outstanding/deferred work, including older messages.

While observations are unavailable, mark monitoring coverage incomplete in the
root's view and use App inspection. Keep the existing routing identity and
message history. Do not delete the store/logs or bulk-ack work to make the watch
green. A real planned wait may be parked, but do not park or finish active work
merely to bypass degradation. A missing event is not a clean bill of health.
