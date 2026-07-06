# Operating telex

## The daemon lifecycle

The exchange auto-spawns on the first daemon-backed command; there is no manual
start. Inspect the resolved backend and address projection with:

```sh
telex status --address <addr>
```

## Stopping a station

`station stop` is the symmetric inverse of the attach and wait loop. It marks the
station non-attending, releases membership durably, and waits for tracked waiters
to exit:

```sh
telex station stop --address <addr>
```

After it returns, a later message to the address stays queued until a future
attach or wait; it is not consumed by an orphaned waiter.

## Upgrading the binary

On a local upgrade, drain and replace in this order:

```sh
telex station stop --address <addr>
telex daemon stop --drain
# replace the telex binary
telex attach --address <addr> --description "<s>"
telex wait --address <addr> --out-dir <dir>
```

If a session resumes without an armed waiter, recovery is durable: inspect
`telex inbox --address <addr>` and `telex read --id <id>`, then arm a fresh wait.

## Auditing

Export messages and disposition history as JSON lines for provenance:

```sh
telex export --address <addr>
telex export --thread <id>
telex export --since <id>
```

## Recovering from a lost daemon

A `wait` that finds no daemon exits with a distinct code (see
[Exit codes](../reference/exit-codes.md)). Run `telex attach` (the spawning and
recovery verb) and re-arm the wait. If a replacement daemon already exists, a
wait can reconnect during its bounded reconnect grace.

## Turn-end and resume reconciliation

For turn-end guards or resume reconciliation, use
`telex station status --session <id>` to get a compact JSON projection of the
session's attended addresses, waiter counts, station health, and pending
unconsumed counts.
