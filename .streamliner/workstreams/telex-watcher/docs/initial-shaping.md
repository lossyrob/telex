# Initial shaping: Telex Watcher

> Support document. This preserves the product reasoning that produced the
> workstream brief and graph. It is background, not authoritative current state.
> Promote changes to intended Telex behavior into the project design layer.

## Origin

The Lossyrob Loop skill can run a detached worker, but the worker remains owned
by the agent session and requires an attached `Wait-LoopDetached.ps1` observer to
wake the agent. In practice that long-lived task interacts poorly with the
session's message/task queue, just as the old per-session Telex holder and waiter
interacted poorly with agent turns.

Telex already solved the wakeup half of the problem by relocating durable waiting
to a per-user local exchange and pushing committed messages through a
harness-specific bridge. What remains is external observation: polling GitHub,
Azure DevOps, a service, a file, or any other condition until something deserves
agent attention.

The idea is a second Telex application, provisionally called **Telex Watcher**.
Agents register trusted local detector scripts with it. The Watcher runs those
scripts outside all agent sessions and sends a Telex message to the configured
address when the detector reports an event.

## Core loop

```text
external source
      |
      | queried by agent-authored detector
      v
Telex Watcher
      |
      | normalized event, durable send
      v
Telex address
      |
      +--> worker or orchestrator session
      +--> operator agent
      +--> Operator Station / human
```

There is no attached waiter in the originating session. The target may be
occupied now, occupied later by a replacement session, or unoccupied when the
event occurs; Telex store-and-forward handles all three.

## Why genericity is the product value

PR watches are rarely uniform. One repository may need to ignore author comments
but wake for external reviewer comments. Another may treat a `+1` from a specific
person as approval. A third may need Azure DevOps policy state, a bespoke bot
comment, or several status checks combined.

Agents can author and refine these detectors quickly because they already know
the repository and current task. Baking GitHub or Azure DevOps policy into the
Watcher would move rapidly changing application judgment into the wrong layer
and make broad dogfooding impossible.

The correct split is:

- **Detector script:** arbitrary observation policy.
- **Watcher runtime:** deterministic execution and state transition.
- **Telex:** durable delivery and wakeup.
- **Agent:** reasoning and consequential action after wakeup.

## Guardrail: generic detector, fixed action

The Watcher is not a general automation engine. A detector cannot ask the daemon
to merge a PR, modify a repository, call an arbitrary action, or launch an agent.
It can only report zero or one normalized event for Telex delivery.

The detector itself is trusted local code and could perform side effects, just as
any locally executed script could. The product contract nevertheless defines it
as observational. The Watcher does not add an action phase, hide side effects
behind configuration, or claim to provide a security sandbox.

Registration is local-only initially. A remote Telex message cannot register or
replace executable code.

## Detector protocol sketch

The Watcher invokes a command with bounded runtime and supplies a versioned JSON
request over stdin or a request file:

```json
{
  "schemaVersion": 1,
  "watch": {
    "id": "telex-pr-93",
    "parameters": {
      "repo": "lossyrob/telex",
      "pullRequest": 93
    }
  },
  "state": {
    "lastReviewId": 8123
  },
  "now": "2026-07-18T23:45:00Z"
}
```

The detector emits one JSON result to stdout; stderr remains diagnostic:

```json
{
  "schemaVersion": 1,
  "outcome": "event",
  "nextState": {
    "lastReviewId": 8372
  },
  "event": {
    "id": "github:review:8372",
    "kind": "watch.github-pr.review",
    "subject": "External review received on PR #93",
    "body": "Reviewer requested changes.",
    "attention": "next-checkpoint",
    "requiresDisposition": true,
    "metadata": {
      "url": "https://github.com/lossyrob/telex/pull/93",
      "reviewer": "example",
      "reviewState": "CHANGES_REQUESTED"
    }
  }
}
```

Candidate outcomes:

| Outcome | Meaning |
|---|---|
| `idle` | Successful observation; no Telex event. |
| `event` | Send the normalized event and continue watching. |
| `terminal` | Optionally send a final event, commit state, and stop. |
| `degraded` | The source could not be evaluated; retry under Watcher policy. |

The exact envelope is owned by the spike. The important separation is that exit
codes report command execution, while structured output reports detector state.

## State and delivery transaction

The safe event path is:

```text
read prior state
→ execute detector
→ validate event + proposed next state
→ send Telex message
→ receive durable send receipt
→ atomically commit next state and sent-event record
```

If Telex sending fails, the prior state remains current. The detector therefore
reports the event again. If Telex accepted the message but the Watcher lost the
receipt, a duplicate remains possible; stable `watchId` and `event.id` metadata
make that visible and deduplicable. At-least-once is safer than consume-before-
send.

For an `idle` result, the production contract must decide whether state may
advance immediately. Allowing it is useful for recording ignored comments or
source cursors, but it makes the detector responsible for intentionally
classifying those observations as non-actionable.

## Watch registration sketch

```yaml
id: telex-pr-93
command:
  - pwsh
  - -File
  - C:\watchers\telex-pr-93.ps1
workingDirectory: C:\watchers
intervalSeconds: 300
timeoutSeconds: 30
target: project:telex/node:93
sender: service:watcher/github
lifecycle: until-terminal
scriptMode: follow-path
parameters:
  repo: lossyrob/telex
  pullRequest: 93
```

The Watcher, not the detector, owns `target`, `sender`, cadence, timeout, and
credential/environment policy. This prevents an ordinary detector result from
silently rerouting messages or impersonating another address.

## Script lifecycle

Two useful modes emerged:

- **Pinned:** record or copy a specific script digest. Updates are explicit and
  auditable. Production default.
- **Follow path:** execute the current file at the registered path and record its
  digest on every attempt and emitted event. Useful while an agent is actively
  developing and dogfooding a detector.

The spike may start with follow-path behavior for speed, but must preserve enough
digest/provenance information to make the production decision honestly.

## Template model

Templates are editable examples of the detector protocol, not providers compiled
into the daemon:

```text
telex-watcher/
  templates/
    github-pr/
    ado-pr/
    http-json/
    command-status/
```

The existing Lossyrob Loop PR scripts are inputs to the GitHub template. Their
domain state checks and tests remain useful; their session-owned lifecycle,
attached waiter, and action runner do not.

An eventual agent skill can scaffold a detector, explain the protocol, and
register the result:

```text
telex-watcher init github-pr ./watch-pr.ps1
telex-watcher watch add --file ./watch.yaml
```

## Relationship to Operator Station

The products are siblings:

- Watcher is a deterministic event producer.
- Operator Station is a human-attended recipient and reply surface.

Neither owns the other. A watch may target a worker directly, an operator agent
that filters events, or the human Station. The address determines the route.

Both products need a supported long-lived non-agent Telex client. Their viability
spikes may use current CLI/library seams, but production work should converge on
the application-client contract tracked by Telex issue #12.

## Campaign staging

```text
Operator Station spike ──┐
                         ├── viability evidence
Watcher generic spike ───┘
                                 |
                  Telex Application Client (#12)
                         /                 \
             production Station     production Watcher
```

The spikes run in parallel because their highest uncertainties are different:
human attention UX versus reliable external detector hosting. The shared client
contract is accepted after real evidence from both rather than designed from
one imagined consumer.

## Risks

- **Arbitrary local code:** detector scripts run with the user's authority.
  Local-only registration, clear provenance, bounded execution, and explicit
  trust are required.
- **Credential leakage:** environment inheritance and logs can expose tokens.
  Credentials need an allowlist or wrapper strategy and redaction discipline.
- **Hung or overlapping checks:** each watch needs a timeout and single-flight
  execution; global concurrency must be capped.
- **Rate limits:** provider-specific scripts can poll too aggressively. The
  runtime supplies minimum cadence, jitter, and backoff even though it does not
  understand provider semantics.
- **Duplicate events:** stable event IDs and a sent-event ledger are necessary.
- **Silent degradation:** repeated detector failures need visible local status and
  eventually a Telex notification policy.
- **Scope drift:** adding arbitrary actions would turn the product into a workflow
  engine and invalidate the simple trust and failure model.

## Viability demonstration

The first gate should exercise several materially different watches:

1. GitHub PR external review or changes-requested event.
2. GitHub detector with repository-specific author/comment filtering.
3. Azure DevOps PR review or policy event.
4. One non-PR detector written or adapted during the dogfood session.

For each case, the original agent session must remain free of long-lived loop or
waiter tasks, the Watcher must survive its own restart, and the event must wake or
queue durably for the configured Telex address.
