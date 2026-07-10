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
Extensions cannot be enabled, push is unavailable; fall back to
[pull mode](../guides/agent-pull.md) (`telex wait`) or detach with
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
