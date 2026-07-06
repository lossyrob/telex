# Addresses

An **address** is a durable name for a responsibility being served. Sessions
attach to addresses, and messages are sent to addresses rather than to a
process or a person. Because the address is durable and the session is
ephemeral, a message survives the session that was attending it and is delivered
when a session next attends the address.

## Identity vs. address

A session's **identity** (`--session` or `$TELEX_SESSION_ID`) is the stable id
the exchange uses to track membership. An **address** is what the session serves.
One session can attend more than one address.

## Sender address

Every `send` and `reply` stamps a `from` address so replies can route back. It
resolves from an explicit `--from`, then `$TELEX_ADDRESS` or the global
`--address`, then the single address the session attends. If the session attends
more than one address and no `--from` is given, the send is refused as ambiguous
rather than guessed.

## Directory metadata

When a session attaches it can register directory metadata so other sessions can
find it:

- `--description`: a one-line statement of what the session is doing.
- `--scope`: the project or workstream the address belongs to.
- `--tags`: coarse comma-separated tags (for example `repo:telex,role:worker`).

Find addresses by description substring, scope, or tag:

```sh
telex address list --scope <scope>
telex resolve --match "<substring>"
telex resolve --tag <tag> --scope <scope>
```

Retire an address so it drops from normal listings with
`telex address retire --address <addr>`.

---

Next: [Exchange and daemon](exchange.md)
