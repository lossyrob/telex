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

The push bridge may not be loaded. If `extensions_reload` is unavailable, enable
Copilot Extensions under `/experimental`. Then re-provision with
`telex --address <addr> copilot resume` and run `extensions_reload`. If Copilot
Extensions cannot be enabled, push is unavailable; use the supported
[Copilot pull fallback](copilot-push.md#fallback) with
`telex --address <addr> copilot fallback prepare`, or detach with
`telex --address <addr> copilot detach`.

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
| `legacy_producer` | The bridge predates the liveness probe, so it cannot be proved alive. | `telex --address <addr> copilot resume`, then `extensions_reload`. |
| `unverifiable` | The producer or its credential could not be resolved (bridge gone, registry stale). | `telex --address <addr> copilot resume`, then `extensions_reload`. |
| `insecure` | A permissions check failed on the bridge directory or credential file. | Fix the permissions (owner-only), then resume. |
| `incompatible` | This daemon cannot reconcile the recorded intent (version skew). | `telex --address <addr> copilot resume`. |
| `quarantined` | Ten consecutive genuine failures; now retrying hourly. | Fix the underlying cause, then resume to reset it. |
| `revoked` | The station was explicitly detached or its session ended. | Intentional. Explicit attach is the only way back. |

If reconciliation reports a lock or local-filesystem failure, keep the intent scope on owner-private
fixed local storage. Network shares, NFS/SMB, 9p, and filesystems whose advisory-lock semantics
cannot be proven are intentionally refused. A busy live writer is bounded and reported; it is never
stolen. Retry after that writer exits.

Messages are durable the whole time — read them with `telex inbox --address <addr>` — and the turn
guard warns rather than blocking, so an unrestored intent never wedges a session.

## Copilot: `telex attach` refuses with `PushIntentUnrecoverable`

This code covers two refusals, both of which fail *toward* telling you rather than quietly
degrading.

**A pull attach over a live push intent.** The address has a live push intent that could not be
restored, and telex refused to create a **pull-only** member over it rather than silently
downgrading your delivery mode.

Either restore push:

```bash
telex --address <addr> copilot resume   # then run extensions_reload in the session
```

or give up push for this station explicitly:

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
