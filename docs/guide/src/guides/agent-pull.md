# Set up an agent (pull)

This guide covers pointing an agent session at telex in a generic harness (scripts,
CI, or any harness without an in-session extension). Copilot CLI uses push delivery
instead; see [Copilot CLI push delivery](copilot-push.md).

Agents read the full, version-matched pull workflow from the binary with
`telex skill`. This guide is the operator's view of how the pieces fit together.

## 1. Give the session a stable identity

Generic telex commands need a stable session id on each invocation, from
`--session` or `$TELEX_SESSION_ID`. Telex fails closed rather than guessing.

```sh
export TELEX_SESSION_ID=<stable-session-id>   # PowerShell: $env:TELEX_SESSION_ID = "<stable-session-id>"
```

## 2. Attach once

```sh
telex attach --address <addr> --description "<what this session is doing>"
```

`attach` registers the session and address and exits. Add `--scope` and `--tags`
so other sessions can resolve this one.

## 3. Wait for one message, then re-arm

`wait` is one-shot: it blocks for a single delivery and exits. Drive the loop from
the agent's own turn cycle, one wait per delivery. Write the result to files with
`--out-dir` so a detached waiter's result is readable after it exits:

```sh
telex wait --address <addr> --session <id> --out-dir <dir>
```

On exit the waiter writes into `<dir>`: `message.json` and `delivery.json` (on
exit 0), `status.json` (always), and `exit.code` (written last, as the completion
marker). Read `exit.code` first; see [Exit codes](../reference/exit-codes.md).

Do not wrap `wait` in an infinite shell loop. Many harnesses surface output only
when a command completes, so an internal loop hides delivered messages. Arm one
wait, handle its completion, then arm the next.

## 4. Ack and disposition

After reading the delivered JSON, ack it (transport consumption), then record the
workflow [disposition](../concepts/disposition.md):

```sh
telex ack --address <addr> --session <id> --id <message-id>
telex handle --address <addr> --session <id> --id <message-id> --note "completed"
```

Dedupe by message id: delivery is at-least-once.

## 5. Stop the station when done

```sh
telex station stop --address <addr>
```

This releases membership durably and waits for tracked waiters to exit. Later
messages stay queued until a future attach or wait.
