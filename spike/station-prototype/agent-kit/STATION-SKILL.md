# Reaching your operator through Telex Station

Your operator reads agent messages in Telex Station. Use `station-notify` to
reach them. It replaces any "post to Teams" script.

```sh
node <path>/station-notify.mjs --priority <level> --subject "<headline>" --body "<message>"
```

`TELEX_ADDRESS` must be your own Telex address. `STATION_ADDRESS` is the
operator's (default `operator:$USER`).

## Pick the priority honestly

The operator's Station sorts and interrupts by this level alone. Inflating it
trains them to ignore you.

| `--priority` | Use when | What the operator experiences |
|---|---|---|
| `blocked` | You have **stopped** and cannot continue without them. | Interrupts them now, with sound, and reminds until answered. |
| `needs-reply` | You want an answer at your next checkpoint and are **still working**. | A quiet banner; sits in their queue. |
| `routine` | Worth knowing. Nothing waits on them. | No interruption; a count in their menu bar. |
| `fyi` | Purely informational. | Silent. Read if they look. |

## Write for someone switching in cold

- **Subject**: the decision or fact, not your activity. "terraform apply wants
  to destroy 3 resources", not "Question".
- **Body**: what you need, what you already tried, and what you will do with
  each answer. Lead with the ask.
- **`--options "A|B|C"`**: when a short answer will do, offer the choices. They
  appear as one-click replies.
- **`--url`**: the PR, run or document they would otherwise have to find.
- **`--project "area/sub"`**: where you would file yourself. A hint only.

## When you are blocked

```sh
node station-notify.mjs --priority blocked --subject "..." --body "..." \
  --options "Yes|No" --wait 900
```

`--wait` blocks until the operator replies in that thread and prints their
reply. Exit code `2` means no reply in time: your message is still in their
queue, so do not resend it. Decide whether to keep waiting or do other safe
work.

## Following up

- To continue a thread **after the operator has replied**, pass
  `--reply-to <id of their message>`.
- Before they have replied, send a new message. (A Telex reply goes to the
  sender of the message you reply to, so replying to your own message does not
  reach them.)
- Raise the priority only if the situation changed, e.g. from `needs-reply` to
  `blocked` when you have actually run out of other work.

## What is shared about you

The helper reports your harness, machine, repository, branch, working folder,
terminal and session id so the operator can tell who is asking and find your
session. It is shown to them marked as self-reported.
