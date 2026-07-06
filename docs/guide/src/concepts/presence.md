# Stations and presence

A **station** is the in-memory registration a session holds while it attends an
address. `attach` creates the station and claims a liveness **lease** (an epoch)
for the address; the exchange tracks the station's health and any live waiter.

## Presence is non-destructive

Detaching or a lease reap is non-destructive: the station and the durable message
buffer remain. A blocked `wait` returns a distinct presence-ended result rather
than losing the buffered messages. A later `attach` re-registers and continues.

## Liveness backstop

`attach --watch-pid anchor:<pid>` registers a non-destructive liveness backstop.
When the watched process dies, blocked waits return a presence-ended result, but
the station and durable buffer are retained. This lets a station be cleaned up
when its owning process goes away without dropping messages.

## Inspecting stations

Get a machine-readable projection of a session's attended addresses, waiter
counts, and station health:

```sh
telex station status --session <id>
```

Stop a station as the symmetric inverse of the attach and wait loop; it marks the
station non-attending, releases membership durably, and waits for tracked live
waiters to exit:

```sh
telex station stop --address <addr>
```

After `station stop`, a later message to the address stays queued until a future
attach or wait; it is not consumed by an orphaned waiter.

---

Next: [Messages and threads](messages.md)
