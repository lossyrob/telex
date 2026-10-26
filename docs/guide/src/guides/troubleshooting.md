# Troubleshooting

## `no session id available`

`send`, `reply`, `ack`, and the disposition verbs need a stable session id. Pass
`--session <id>` or set `TELEX_SESSION_ID`. In Copilot CLI, pass
`--session "$COPILOT_AGENT_SESSION_ID"`. Telex fails closed rather than guessing
an identity.

## `wait` exits 3 (daemon gone / not running)

`wait` does not spawn a missing daemon. Run `telex attach --address <addr> ...`
(the spawning and recovery verb), then re-arm the wait. See
[Exit codes](../reference/exit-codes.md).

## `one live waiter is already armed`

Only one live waiter per station is allowed. Let the current waiter complete, or
run `telex station stop --address <addr>`, then re-attach and arm the new mode.

## Cannot re-arm because the prior message is unacked

Ack the delivered message first (`telex ack --id <id> --session <id>`), then arm a
fresh wait.

## Send refused as ambiguous, or a warning that it is un-repliable

Every send stamps a `from`. If your session attends more than one address, pass
`--from <addr>`. If no `--from`, `--address`, or attended station is set, the send
warns that it is un-repliable; attach the address first or pass `--from`.

## `address <addr> is retired`

The target address was retired and dropped from listings. Use a live address, or
the owner can attend it again.

## Copilot: messages do not arrive as turns

The push bridge may not be loaded; provisioning files and registering push do not
prove it is live. On a host with a supported live loader, re-provision with
`telex --address <addr> copilot resume` and load the bridge (`extensions_reload`
where available). On Copilot CLI only, enabling Copilot Extensions under
`/experimental` may expose that tool. For an App session without a supported live
loader, startup extension discovery is another loading path: provision with
`telex --address <addr> copilot attach --copilot-bridge`, restart the App session,
then run `telex --address <addr> copilot resume`. Verify
`telex --address <addr> --json status` reports `station_health: attended_push` and
`push_registered: true`; where available, call `telex_bridge_info` for supporting
bridge evidence.

Keep an already-loaded, live bridge in push mode regardless of host. A restart is
not proof by itself. [Issue #230](https://github.com/lossyrob/telex/issues/230)
reports startup loading and pushed delivery on one Windows App build, not every
App build or a plain conversation clear. Liveness and end-to-end delivery are
separate checks.

When a live bridge is unavailable and a restart is unacceptable or liveness does
not verify, use the supported
[Copilot pull fallback](copilot-push.md#fallback) with
`telex --address <addr> copilot fallback prepare`, or detach with
`telex --address <addr> copilot detach`.
Preparing fallback does not prove a waiter started or woke the App: follow the generated
single-shot launch and completion-artifact procedure, and report any setup/backend
failure rather than treating it as successful delivery.

## Backend authentication failures (Postgres / Entra)

Check the connection string and credentials. `telex init --backend <name>`
validates connectivity and creates the schema, surfacing errors early. For Entra,
ensure `az login` has run, or use `--entra-cred managed` on a host with a managed
identity.

## Inspecting state

- `telex status --address <addr>`: resolved backend and address projection.
- `telex station status --session <id>`: attended addresses, waiter counts, and
  station health for a session.
- `telex daemon status`: daemon internals.

## Copilot: push was working, then stopped after an upgrade or daemon restart

This should recover on its own. A push-attended station records a durable **station intent**, and a
successor daemon restores it within about 10 seconds of being up (or `liveness_window_secs()` + 10 s
after a hard crash, because the crashed daemon's lease has to go stale first). See
[Push recovery after a daemon replacement](operating.md#push-recovery-after-a-daemon-replacement).

If it has not recovered, check the station:

```bash
telex --address <addr> status
```

Look at the `station_intent` line:

| State | Meaning | What to do |
|---|---|---|
| `deferred_lease` | Waiting for a crashed predecessor's lease to go stale. **Not an error.** | Wait; `next_attempt_ms` says when it retries. |
| `deferred_pull_waiter` | A live `telex wait` owns the address; pull wins. | Stop the waiter (`telex station stop`) if you want push back. |
| `legacy_producer` | The bridge predates the liveness probe, so it cannot be proved alive. | Follow [bridge recovery](#copilot-messages-do-not-arrive-as-turns) using live reload or attach/restart/resume and verify liveness; otherwise use [pull fallback](copilot-push.md#fallback). |
| `unverifiable` | The producer or its credential could not be resolved (bridge gone, registry stale). | Follow [bridge recovery](#copilot-messages-do-not-arrive-as-turns) using live reload or attach/restart/resume and verify liveness; otherwise use [pull fallback](copilot-push.md#fallback). |
| `insecure` | A permissions check failed on the bridge directory or credential file. | Fix the permissions (owner-only), then resume. |
| `incompatible` | This daemon cannot reconcile the recorded intent (version skew). | `telex --address <addr> copilot resume`. |
| `quarantined` | Ten consecutive genuine failures; now retrying hourly. | Fix the underlying cause, then resume to reset it. |
| `revoked` | The station was explicitly detached or its session ended. | Intentional. Explicit attach is the only way back. |

If reconciliation reports a lock or local-filesystem failure, keep the intent scope on owner-private
fixed local storage. Network shares, NFS/SMB, 9p, and filesystems whose advisory-lock semantics
cannot be proven are intentionally refused. A busy live writer is bounded and reported; it is never
stolen. Retry after that writer exits.

## Station-intent capacity needs an exact inventory or reclamation

An automatic reconcile report marked truncated or degraded is not an exact inventory: its count and
over-cap indication are lower bounds. Do not retry it until it happens to look complete. Stop the
daemon with `telex daemon stop --drain`, stop other intent writers, then run
`telex --json daemon recover-intents` for an exact supported-local inventory. Add `--gc` only when
you want eligible TTL-expired records reclaimed. The command refuses if it cannot establish the
daemon-stopped and supported-local-storage floors; fix that condition rather than trusting a partial
count.

Messages are durable the whole time — read them with `telex inbox --address <addr>` — and the turn
guard warns rather than blocking, so an unrestored intent never wedges a session.

## Copilot: `telex attach` refuses with `PushIntentUnrecoverable`

This code covers two refusals, both of which fail *toward* telling you rather than quietly
degrading.

**A pull attach over a live push intent.** The address has a live push intent that could not be
restored, and telex refused to create a **pull-only** member over it rather than silently
downgrading your delivery mode.

Either restore push with an already-live bridge or a supported live loader:

```bash
telex --address <addr> copilot resume   # use the supported loader if needed
```

or follow the [attach/restart/resume startup-discovery path](#copilot-messages-do-not-arrive-as-turns)
and verify liveness. When a restart is unacceptable or liveness does not verify,
use the [prepared pull fallback](copilot-push.md#fallback). To give up attendance explicitly:

```bash
telex --address <addr> copilot detach
```

after which a plain `telex attach` works normally.

**A push registration whose durable proof could not be written.** A push attach for a binding that
has a station-intent record only succeeds once the daemon has durably recorded that it armed
delivery — that record is the *only* thing that carries push across a daemon replacement, so a
registration that cannot write it is refused instead of leaving you with push that works now and no
recovery later. The message names the write that failed. Causes are almost always local:

- the station-intent record for this station is present but unreadable (corrupt, or written by a
  build this one cannot verify), or
- the station-intent record or the scope that holds it exists but the operating system will not
  report on it — a permissions change, a file lock, a profile on a network volume that went away.
  Telex refuses here rather than assuming the station is new, because assuming that would commit
  push with nothing recoverable behind it, and
- the station-intent scope is not writable (check the permissions on the daemon run directory), or
- a concurrent `copilot attach`/`copilot resume` for the same station raced this one and rolled its
  own record back.

A station with no intent record at all — a pull attach, or a plain `telex attach --on-deliver` —
owes no proof and is never refused by this, even if the intent scope cannot be created on this host.

Nothing is left half-armed: no member is created, and any epoch lease the attempt claimed is
released. Re-run the attach — `telex --address <addr> copilot resume` — once the scope is writable
or the competing attach has finished.
