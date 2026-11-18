# Telex Station: design carried forward from the Mac prototype

This is what the prototype argues the product should be. Each element below was
built and exercised against real Telex; the last sections list what Telex needs
to grow, what conflicts with the accepted Station contract, and what is still
open.

## The product in one paragraph

Station is an **attention inbox for a fleet of agents**. It is not chat. Agents
do not talk to you; they hand you obligations with an honest urgency, and you
clear them. Every screen answers four questions in order: *what needs me most,
who is asking, where does that session live, and what happens if I answer now.*
Telex stays the durable, semantically dumb transport. Everything that makes the
inbox pleasant (tiers, projects, notification policy, read state) is local to
the Station and never changes what Telex delivers.

## 1. One model of urgency, owned by the agent

Priority is the Telex attention level and nothing else. Station never infers
urgency from text, and metadata cannot raise it.

| Station tier | Telex attention | Agent's promise | Default reach |
|---|---|---|---|
| **Blocked** | `interrupt` | I have stopped and cannot continue | banner + sound + demand attention + badge, reminder every 10 min |
| **Needs reply** | `next-checkpoint` | Answer when you can; I am still working | quiet banner + badge |
| **Routine** | `background` | Worth knowing; nothing waits | badge only |
| **FYI** | `fyi` | Informational | silent; a feed, never the inbox |

Carry forward:

- **Human words for the tiers**, each with a one-line meaning in the agent's
  terms. `interrupt` / `next-checkpoint` are the agent's vocabulary, not the
  operator's.
- **Glyph + colour + position** all encode the tier (`!` `?` `•` `i`), so it
  reads at a glance and without colour.
- **The agent-side skill is half the design.** The tiers only work if agents
  choose them honestly, so the helper's documentation is written around "what
  the operator experiences" for each level.

## 2. The ball-in-court thread model

A thread is in exactly one state, derived from facts:

| State | When |
|---|---|
| **Needs me** | The agent's latest non-FYI message is unanswered and unresolved. |
| **Sent, awaiting pickup** | I replied; the session has not consumed it yet. |
| **Snoozed** | I deferred it until a time. New work from the agent wakes it. |
| **Done** | Answered and picked up, or resolved. |
| **FYI** | Only informational or CC traffic. |

Carry forward:

- The inbox is "Needs me", grouped by tier, **longest-waiting first inside a
  tier**. Newest-first is wrong for a queue of blocked workers.
- **"Awaiting pickup" is the useful half of "waiting"**: it shows replies an
  agent has not seen. Once consumed, the thread is done. A thread I want to
  watch gets a **flag**, which pins it in Needs me.
- An agent that escalates re-enters the same thread at the higher tier.
- **Reply and Done are one gesture by default** ("Send & done"), with a visible
  toggle to send without resolving.
- Verbs map onto Telex dispositions so agents can see the outcome:
  Done → `handled`, Snooze → `deferred` (obligation stays open), Decline →
  `rejected` with a note the agent can read, plus Escalated and Closed per
  message.

## 3. Who is asking, and where the session lives

Every thread carries a one-line **Where** strip, always visible:

`Claude · devbox-2 · lossyrob/telex @ feature/station-app · tmux main:3.1`

It expands into a card with three provenance groups, visibly labelled:

- **Recorded by Telex** (from the exchange): address, presence, occupant host,
  directory description and tags.
- **Self-reported by the agent** (unverified): harness, model, machine, repo,
  branch, folder, terminal, session, and a copyable **"open with"** command.
- **Mine** (only on this Station): the name I use, **"how I get to it"** (a
  note that outranks the self-report in the strip), notes, filing, and a
  notification override.

Carry forward:

- **Provenance is part of the visual design.** When Telex records a host, it
  replaces the self-reported one in the strip.
- **A local "how I get to it" note** is how the operator's own way of organizing
  sessions wins over whatever the agent claims.
- **Presence is phrased as a consequence**, next to Send:
  *Listening now* → "will reach the session right away";
  *Attached, not listening* → "reads this at its next check";
  *Session gone* → "kept until one re-attaches".
- **Receipts say only what is known**: sending, queued (nobody attending),
  delivered to the session queue, picked up, handled by the agent,
  unconfirmed. No fake "seen".
- A **fleet roster** groups all agents by machine, project or harness.

### The card convention

Opaque Telex metadata, one namespaced key, presentation only:

```json
{ "station": { "v": 0,
    "card": { "name": "impl-134", "harness": "claude-code", "host": "devbox-2",
              "repo": "lossyrob/telex", "branch": "feature/x", "cwd": "/home/rob/proj/telex",
              "terminal": "tmux main:3.1", "session_id": "…",
              "open_command": "ssh devbox-2 -t tmux attach -t main",
              "project_hint": "telex/station", "url": "https://…", "tags": ["impl"] },
    "replies": ["Approve", "Hold"] } }
```

Rules the prototype enforces: every value is bounded, control characters are
stripped, everything is rendered as inert text, `open_command` is only ever
copied (never run), unknown keys are preserved and shown as "not interpreted",
and nothing here can change tier, recipient, or policy.

**Suggested replies** are chips that fill the composer. The human still sends.

## 4. Getting attention, and not

Three intensities, which the operator can see and tune:

1. **Demand** (Blocked): banner, sound, dock bounce, a red strip across the top
   of the window until looked at, and a reminder while unanswered.
2. **Notify** (Needs reply): a quiet banner.
3. **Ambient** (Routine): a count in the menu bar and dock. No interruption.

Carry forward:

- **A tier × channel matrix** is the whole notification settings page: Banner,
  Sound, Demand attention, Badge, Remind, with a **Test** button per tier.
- **Overrides nest, most specific wins**: thread › agent › nearest project ›
  defaults. Values: follow defaults, mute, only Blocked, elevate.
- **One "what may interrupt me" control** at the top of the sidebar and in the
  menu bar: normal, only Blocked (1 hour / until off), nothing (timed). Timed
  modes expire on their own. Quiet hours with "except Blocked".
- **Muting never hides or re-tiers a message.** It only removes the
  interruption; the item still counts and still shows as Blocked.
- **Bursts fold into one summary banner**; Blocked is never folded.
- **Coming back after being closed produces one summary** ("3 waiting, 1
  blocked"), not a banner per message.
- **A notification log** records every decision including the quiet ones and
  why. "Shown" means submitted to macOS, not seen.
- **The menu-bar item** shows the count (or "2 blocked"), lists the top of the
  queue, and jumps straight to a thread. Closing the window keeps attending.

## 5. Organizing, with almost no setup

Projects are nested folders. **Agents are filed, and their threads follow.**

Filing precedence: a thread I filed › an agent I filed › the first matching
rule › the agent's own hint (which may also refine a rule into a subfolder of
the project the rule chose, but never move the agent elsewhere).

Carry forward:

- **Organize sheet with one-click suggestions** from the fleet itself: same
  repo, same address prefix, same machine, and folders agents asked for.
  Accepting one creates the project (and subfolders) plus the rule, so future
  agents file themselves. Suggestions stay available for subfolders even after a
  broader rule exists.
- **Inline everywhere**: drag a thread or agent onto a project (drop files the
  agent; Option-drop files only the thread), a "📁 project ▾" chip on every
  thread with an inline "new project" field, and `m` on the keyboard.
- **Rules are readable sentences** with a live match count and explicit order.
  A rule for a subfolder is automatically placed before its parent's.
- Counts **roll up** the tree showing the most urgent tier underneath.
- Deleting a project moves its contents up a level; nothing is orphaned.
- "Unsorted" is a first-class view with a count, so new agents are noticed.

## 6. Smaller decisions worth keeping

- Keyboard triage: `j/k`, `r`, `⌘↩`, `e` done, `s` snooze, `f` flag, `m` move,
  `/` filter, `g i` / `g b` / `g a`.
- Reply urgency is a choice on the composer: interrupt now / at next checkpoint
  / whenever. Default follows the tier being answered.
- A health dot and a plain-language strip when Station is not really attending.
  Attending an address means a Station is running, not that a person is there.
- Links in messages open only after a confirmation that shows the destination.

## What Telex needs for the real product

Found by building against it:

1. **macOS cannot host an Application Client.** The daemon bootstrap is refused
   on non-Linux Unix, and the legacy path needs the same executable. A Mac
   Station is blocked on that Local Daemon / Application Client decision. The
   prototype drives the `telex` CLI on macOS instead, behind a port.
2. **No way to read another address's presence** through the Application
   Client. "Is this session listening?" is central to the reply experience. The
   prototype reads `telex address list --json`.
3. **History excludes messages the Station authored** (it is recipient-scoped),
   so a Station must keep its own sent messages to draw a thread.
4. **An agent cannot follow up in its own thread** before the human replies: a
   reply goes to the parent's sender. Follow-ups become new threads. Either
   Telex allows it or Station groups them.
5. **CLI `reply` takes no metadata**, so a reply cannot be marked human-authored
   on that path.
6. **There is no "a person has seen this" signal** on the CLI path (`ack` is
   transport consumption). The prototype has the setting but cannot honour it on
   macOS.
7. **A crashed session leaves a dead anchor pid and an orphaned waiter** that
   end every later `wait` with "presence ended" until the membership is stopped
   and re-attached. Worth a first-class recovery.

## Where this departs from the accepted Station contract

`docs/design/operator-station.md` and ADR 0051 are the authority. These points
need a decision before `station-app` adopts them:

- **Interpreting a metadata convention at all.** The contract keeps metadata
  opaque. The card is presentation-only and labelled unverified, but it is still
  a Station-read convention, in a namespace other than the retired
  `urn:telex:operator-station:v1`.
- **Suggested replies** resemble the deferred "structured decision widgets".
  Here they are inert text prefill.
- **Organization and filing rules** are new local policy, as is the
  thread/agent/project notification override chain.
- **Session locator and "open with"** touches the deferred "rich
  session-opening". Here it is display and copy only.
- **macOS first**, where the contract is Windows first.
- **The CLI port** is exactly what the contract rules out for production.

## Open questions

- Away from the desk: the Teams hack reached a phone. Station does not.
- Should Station learn to open a session (ssh / `--resume`) rather than only
  show how?
- One Station across several Telex stores. The prototype assumes one.
- Should a self-reported card be remembered per agent (as now) or per message?
- Should repeated follow-up threads from one agent be grouped visually?
