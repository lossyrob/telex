# Telex Decision Log

A lightweight running record of significant decisions. This is the trail for the
*big* choices — language, architecture, protocol shape, backend strategy — not
every small implementation call.

## When to add an entry

Add an entry when a decision is **load-bearing and would be costly or confusing to
relitigate later**. A good test: *would a future contributor (or future you) want
to know why this was chosen, and be tempted to undo it without that context?*

- **Do** record: language, architecture boundaries, protocol/schema shape, backend
  choices, security/auth direction, major scope cuts, anything reversing a prior
  entry.
- **Don't** record: naming, file layout, routine refactors, library micro-choices,
  anything easily changed and locally obvious.

Keep entries short. Three or four sentences per field is plenty. The point is a
trail, not a thesis.

## Conventions

- Entries are **append-only** and numbered sequentially (`0001`, `0002`, …).
- Don't rewrite a past decision — supersede it. Add a new entry, set the old one's
  status to `Superseded by NNNN`, and note what changed.
- **Status** is one of: `Proposed`, `Accepted`, `Accepted (pending validation)`,
  `Superseded by NNNN`, `Deprecated`.
- If a single file ever gets unwieldy, split into a `decisions/` directory with one
  file per entry, preserving the numbers.

### Entry template

```markdown
## NNNN — Title

- **Date:** YYYY-MM-DD
- **Status:** Proposed | Accepted | Accepted (pending validation) | Superseded by NNNN | Deprecated

**Context.** Why this came up; the forces and constraints in play.

**Decision.** What we chose.

**Consequences.** Trade-offs accepted, follow-ups, and what would cause us to revisit.
```

---

## 0001 — Keep a lightweight decision log

- **Date:** 2026-06-05
- **Status:** Accepted

**Context.** The project is early and the design is still fluid across several
top-level documents (thesis, design, dispatch). Big decisions are being made in
conversation and risk being lost or silently relitigated. Full per-file ADRs felt
like too much ceremony for a project this young and would invite over-recording.

**Decision.** Maintain a single append-only `DECISIONS.md` with short numbered
entries (Context / Decision / Consequences + Status), reserved for load-bearing
decisions. Graduate to a `decisions/` directory only if the single file becomes
unwieldy.

**Consequences.** A low-friction trail for the choices that matter, without
documenting every small call. Requires the discipline to actually add an entry when
a big decision lands. Superseding rather than editing preserves history.

## 0002 — Implement Telex in Rust

- **Date:** 2026-06-05
- **Status:** Accepted

**Context.** Telex needs a single, fast-starting native binary: agents invoke the
CLI constantly and in loops, so per-invocation startup cost matters, ruling out
Node/Python/TS for the shipped artifact. The semantic core is fundamentally a set
of state machines — delivery, disposition, attention, address lifecycle, lease, and
answerback grades — whose whole value is keeping distinct states honestly distinct.
The waiter is a long-running daemon that must not lie about liveness (reconnect,
lease holding, crash-safety, LISTEN/NOTIFY). Go was the main alternative, trading
type-modeling rigor for faster iteration and a more mature Azure SDK.

**Decision.** Build Telex in Rust. Rust's sum types and exhaustive `match` enforce
at compile time the exact state distinctions the product is selling (making illegal
states unrepresentable), and tokio gives a trustworthy, low-overhead background
waiter. The CLI-first, backend-as-broker boundary decouples Telex's language from
any consumer (e.g. Streamliner), making this a low-regret choice on the
integration axis.

**Consequences.** We accept slower compile times and higher cost of churn while the
design is still moving — a real tax given the design is actively evolving. We also
accept that the Azure SDK for Rust (Entra → Postgres auth) is younger than Go's;
this is the main technical risk.

This decision is **pending validation by a spike** before committing the full
implementation. The spike must prove, in Rust, the riskiest assumptions behind
answerback and the networked backend:

1. a Postgres **session-scoped advisory lock that auto-releases on connection
   drop** (kill the process → lock gone → address unoccupied, with no reaper
   daemon);
2. **LISTEN/NOTIFY** waking a blocked waiter;
3. **Entra token auth** to an Azure Postgres Flexible Server.

If the spike (especially the Entra/async path) proves unworkable or unpleasant
enough to outweigh the modeling benefits, this entry will be superseded — most
likely in favor of Go.

**Validation outcome (2026-06-05).** The spike (`spike/`) passed and this decision
is accepted. Rust reached the Azure Postgres Flexible Server with Entra auth from a
normal `az login` (TLS via Windows schannel, no OpenSSL), the one item flagged as the
main technical risk — the only friction was an incorrect token resource on the first
attempt (the correct one is `https://ossrdbms-aad.database.windows.net`). The
liveness/answerback assumptions were re-shaped during the spike: items 1 and 2
(advisory lock, `LISTEN/NOTIFY`) were superseded by a simpler TTL-heartbeat +
poll-with-cursor baseline (see 0005), which the spike validated instead, including a
live two-session cross-machine-style messaging test.

## 0003 — Telex owns long-duration waiting (native waiter, not agent-authored loops)

- **Date:** 2026-06-05
- **Status:** Accepted

**Context.** Agent loop skills (e.g. the `loop` skill) work well for dynamic,
agent-authored checks, and it is tempting to implement Telex's waiting the same way —
have the agent script repeated short-lived CLI calls. But the strongest answerback
grade, a connection-bound lease that releases the instant a session dies, requires a
single long-lived process to hold the backend connection (and Postgres advisory lock)
for the whole mission. Repeated short-lived invocations open and close a connection
each time and structurally cannot hold such a lease, silently degrading answerback to
the weaker heartbeat/TTL grade. Implementing the wait in agent scripts would also tie
the liveness guarantee to a specific agent platform, undercutting vendor-neutrality.

**Decision.** Telex provides the blocking wait as a native primitive (`telex wait`)
owned by the Telex binary, holding the lease and blocking efficiently (`LISTEN/NOTIFY`
on Postgres, poll on SQLite). Agents and sub-agents **supervise** the native waiter —
launch, restart, reconnect, relay actionable messages, refresh the work-scope brief —
but do not reimplement it. Generic loop/skill mechanisms remain appropriate for other
dynamic, agent-invented checks; they are just not how Telex message-waiting and
answerback are built.

**Consequences.** Telex must implement robust native blocking, reconnection, and
cursor resume rather than leaning on external scripts. The native waiter's process
identity becomes the thing that backs connection-bound liveness. This adds two items
to the spike (ref 0002): a durable native `telex wait` process, and a Copilot CLI
sub-agent that supervises it (restart/reconnect/relay) and accepts a mid-run steer to
update the work-scope brief — which also exercises the CLI's steerable sub-agent
functionality.

## 0004 — Split the waiter into a resident holder and an ephemeral delivery client

- **Date:** 2026-06-05
- **Status:** Accepted

**Context.** Connection-bound liveness (0003) wants one long-lived process holding
the backend connection and advisory lock. But agent runtimes can only reason about a
message once the delivering call **returns**: delivery requires a process exit and a
turn, after which the agent acts and resumes waiting. A single call cannot both block
indefinitely and invoke agent turns mid-wait. If the lease-holding process were the
one that exits to deliver each message, the lease would release during exactly the
window when the agent is most alive — handling the message — falsely reporting the
line dead.

**Decision.** Split the waiter into two processes. A **resident holder** holds the
backend connection and lease, buffers actionable messages locally, and never takes an
agent turn — so it stays up for the whole mission. An **ephemeral delivery client**
(`telex wait`) blocks on the holder over fast local IPC and exits the instant a
message is ready, handing it to the agent; the agent dispositions and calls
`telex wait` again. The exit that delivers to the agent happens at the client layer,
so the agent's turn never drops the backend connection and the address stays
`occupied` while a message is being handled. This preserves the familiar
exit-with-info-then-restart loop cadence, but only the cheap local client exits per
turn.

**Consequences.** The holder's lifecycle must track the session's — it must be a
session-owned process, **not** a fully detached daemon, so that session/terminal/
machine death kills it and releases the lease promptly; a detached holder would
outlive a dead session and lie about liveness. The supervising sub-agent launches and
monitors the holder, runs the delivery-client loop, and relays to the foreground. The
spike (ref 0002, 0003) must validate the two-process model: killing the holder
releases the lock fast, while repeatedly exiting/restarting the delivery client keeps
the lock held, and binding the holder to session lifetime makes session-kill release
the lock. The pure-TTL alternative (let `telex wait` hold the lock, cover agent-turn
gaps with a heartbeat grace) was rejected as the default because it reintroduces TTL
and still flips to "dead" on any turn longer than the grace window; TTL remains the
SQLite-grade path where no resident holder exists.

**Validation outcome (2026-06-05).** The spike validated the two-process model. The
holder survived repeated waiter exit/restart cycles with the address remaining
`occupied` throughout; killing the holder dropped liveness (after the TTL window);
holder-gone and a wedged/hung holder were detected by the client via distinct exit
codes (`3` gone, `4` hung). A live two-session "increment game" exchanged messages
both directions through the shared backend using exactly this topology — each session
running an attached holder plus an attached waiter that exits on delivery, notifies
the agent, and is restarted. Note: with the TTL baseline (0005) the holder no longer
holds an advisory lock, but the two-process split is, if anything, more necessary,
because the holder is now what keeps the TTL heartbeat alive across agent turns.

## 0005 — TTL-heartbeat + poll-with-cursor as the v0 baseline; defer LISTEN/NOTIFY and advisory locks

- **Date:** 2026-06-05
- **Status:** Accepted

**Context.** Early notes (and the research brief) treated Postgres `LISTEN/NOTIFY`
(push delivery) and session-scoped advisory locks (connection-bound liveness) as
central design wins. Both are Postgres-only, add real complexity, and have no SQLite
equivalent — so each is a second code path on top of the portable one that must be
written anyway (poll-with-cursor for delivery; TTL heartbeat for liveness, which
SQLite needs regardless). Telex's workload is agent-turn-scale (seconds), where
sub-second push buys nothing and a 1–2s indexed poll is trivial load. The capability
model already anticipates both grades (`push: native | poll`, `lease: connection |
ttl | advisory`).

**Decision.** Make **TTL-heartbeat liveness** and **poll-with-cursor delivery** the
single portable v0 baseline for both SQLite and Postgres. Treat `LISTEN/NOTIFY` and
advisory locks as **later, optional Postgres-only upgrades** that raise push latency
and the answerback grade — added behind the existing capability flags only if a
measured need appears — not v0 prerequisites.

**Consequences.** One delivery path and one liveness path across both backends,
substantially less Postgres-specific complexity, and an honest-but-weaker liveness
grade in v0 ("last seen within the TTL window" rather than "dead the instant the
connection drops"). Receipts must state the grade honestly. The spike confirmed
poll + TTL is sufficient end-to-end — including the live two-session test — and that
neither `LISTEN/NOTIFY` nor advisory locks were needed to make the model work. This
supersedes the liveness/push portions of 0002's original spike plan (items 1 and 2);
the advisory-lock-on-disconnect upgrade remains a documented future option, not a
commitment.

**Measured latency (2026-06-05).** An instrumented spike run compared poll
vs push delivery and decomposed end-to-end lag (see `spike/README.md`). Backend
delivery: poll ~0.6 s avg (≈ ½ the 1 s poll interval, ~1 s worst case), push ~0.14 s
(≈ cloud round-trip floor); `bench` standalone showed poll median ~500 ms vs push
~65 ms. But the dominant end-to-end term was **agent-wake latency — the runtime
waking the agent after the waiter exits — at ~6–26 s**, one to two orders of
magnitude larger than any backend lag and entirely above the telex layer. This both
confirms the choice (push cannot fix the lag that actually dominates agent-to-agent
messaging) and reframes it: push is worth having for machine-to-machine dispatch
(DISPATCH.md), not for perceived agent-loop latency. The run also surfaced two
fixable transport costs — per-call Entra token fetch (~2.7 s, fixed by caching) and
per-`send` connection setup to a cloud DB (~0.4 s warm, up to ~2.8 s cold) — noting a
future option to route sends through a warm/pooled connection rather than a fresh
short-lived one.

## 0006 — Backend trait validated over Postgres and SQLite; SQLite is concurrency-safe for the local case

- **Date:** 2026-06-05
- **Status:** Accepted

**Context.** Before this spike, every validation had been Postgres-only. SQLite is
half the v0 surface (decision 0005's "same semantic core, two backends"), and its
real risk — multiple processes coordinating over one shared file for the local
two-session case — was entirely unproven. SQLite is single-writer and can surface
`SQLITE_BUSY` under contention.

**Decision.** Build the v0 backend layer behind a small `Backend` trait
(ensure-address, claim-lease, heartbeat, max-id, fetch-after-cursor, insert, notify,
occupancy), with Postgres and SQLite implementations selected at runtime; keep the
ephemeral delivery client backend-agnostic (it speaks only the local holder socket).
For SQLite, use WAL mode with a `busy_timeout`; store lease heartbeat as epoch-ms and
compute occupancy in code. Push (`LISTEN/NOTIFY`) stays a Postgres-only extra; SQLite
relies on poll, which is the v0 baseline anyway.

**Consequences.** The spike implemented this trait with both backends and ran the
same generic `holder`/`waiter`/`sender` binaries over each. SQLite multi-process
concurrency held under stress: 6 concurrent writer processes (90 inserts) alongside 2
holders heartbeating and polling the same file produced **0 write failures, 91/91
distinct and monotonic ids, no corruption**, with WAL + `busy_timeout` absorbing
contention. Monotonic AUTOINCREMENT under concurrent writes is confirmed, which the
cursor delivery model depends on. With this, the last major v0 unknown is closed:
SQLite (local) and Postgres (networked, with or without Entra) are both viable under
one semantic core, and full v0 implementation can proceed. The spike also noted a
small operational fix for the build: the Entra token cache TTL must be shorter than
the token's actual lifetime (a 50-min cache outlived a token during testing).

## 0007 — v0 command surface settled (attach blocks as holder; flat disposition verbs)

- **Date:** 2026-06-05
- **Status:** Accepted
- **Amended:** the "no per-recipient delivery table" clause is superseded by 0010 (durable delivery tracking, issue #10); the rest of this entry stands.

**Context.** Before implementing v0, Telex needed the full `telex` command surface
and distributable shape aligned across the design docs, agent instructions, and release
plan. Two open questions were whether `attach` should block as the holder or whether a
separate `serve` verb should exist, and whether disposition verbs should be flat or
nested under a `disp` parent.

**Decision.** `attach` blocks and IS the resident holder; there is no `serve` verb.
Disposition verbs are flat: `telex ack`, `telex handle`, `telex defer`, `telex reject`,
`telex close`, and `telex escalate`. V0 is a single `telex` binary with all subcommands;
SQLite is the zero-config default, while Postgres (with or without Entra) is the
networked backend. The holder and ephemeral `telex wait` client communicate through
named-pipe IPC on Windows and unix-socket IPC elsewhere. V0 ships a derived inbox (no
per-recipient delivery table), single primary `to` plus optional `cc`, threading via
`thread_id`/`parent_id`, and a dispositions table.

**Consequences.** The holder's lifetime equals the blocking `attach` invocation: it is
a session-attached background task, not a detached daemon. Flat verbs keep the
agent-facing surface terse. The spike's throwaway crate stays in `spike/`, while v0 is
a fresh implementation at the repo root. Distribution will be GitHub Releases of the
single binary via GitHub Actions.

## 0008 — Modular backends via Cargo features; storage and auth as separate pluggable axes

- **Date:** 2026-06-05
- **Status:** Accepted

**Context.** Telex should invite contributors (and AI coding agents) to add whatever
backend they need without forcing everyone to pull in everyone else's dependencies — a
SQLite-only or AWS user should not compile the Azure SDK (measured at ~185 transitive
crates: reqwest/hyper/rustls), and vice versa. Python expresses this with extras
(`pip install pkg[flex]`), but Rust compiles to a single static binary and selects
dependencies at *compile time*; it has no ergonomic runtime plugin system (dynamic
loading exists via `libloading`/WASM but is fragile — no stable ABI — and not worth it
now). The author expects to maintain only the Azure Postgres Flexible Server backend for
a while but wants the door open for community AWS/GCP backends.

**Decision.** Use **Cargo feature flags as the extras analog**: backend dependencies are
`optional` and gated behind named features (`sqlite`, `postgres`, `entra`, future
`aws-iam`/`gcp-iam`), so `cargo install telex --features postgres,entra` pulls exactly
what's wanted and nothing else. Model **two orthogonal pluggable axes**: *storage
backends* (the existing `Backend` trait) and *auth/credential providers* (a `Credential`
trait — `password`, `entra`, later `aws-iam`/`gcp-iam`), so a Postgres backend can pair
with any cloud's auth and each cloud SDK is isolated behind its own feature. The backend
factory gates each arm with `#[cfg(feature = ...)]` and returns an actionable error
("not in this build — `cargo install telex --features <kind>`") when a backend isn't
compiled in; a `telex backend kinds` lists what's compiled in. Because prebuilt binaries
bake features in, distribution serves two audiences: `cargo install --features …` for the
à la carte path, plus a small curated set of prebuilt release variants (batteries-included
default = sqlite+postgres+entra; a minimal sqlite-only build), expandable per-cloud later.
Entra auth uses the **now-GA Azure SDK** (`azure_identity`/`azure_core` 1.0;
`DeveloperToolsCredential` for `az login`, `ManagedIdentityCredential` for no-login
devboxes), gated behind the `entra` feature; non-Azure secrets use `--password-env` /
`--password-command` rather than plaintext in config. The **backend conformance test
suite** (issue #1) is the contribution contract test.

**Consequences.** Each build pays only for the backends/auth it enables; the Azure SDK
lands only with `entra`. Trade-off: features are additive and chosen at build time, not
hot-swappable at runtime; prebuilt-binary users pick a variant or `cargo install`.
Implementation lands incrementally: (1) feature-gate `sqlite`/`postgres` now; (2) add the
`entra` feature + Azure SDK with the config/profile system; (3) conformance suite and a
later `telex-core` + `telex-backend-*` crate split (for out-of-tree backends) when an
external backend actually needs it. The Azure SDK was validated empirically before this
entry: GA at 1.0.0 (retiring 0002's churn risk), ~15 lines to fetch a token, worked with
the existing `az login`, and `ManagedIdentityCredential` enables true zero-login devbox
setup with internal token caching.

**Validation outcome (2026-06-06).** Implemented and validated. Backends are now optional
Cargo features (`sqlite`, `postgres`, `entra`); `cargo build` proven for default, sqlite-only,
postgres-only, and `--features entra`, with the sqlite-only dep tree confirmed free of the
postgres stack and a clear "reinstall with --features" error for unavailable kinds. The
storage axis shipped as named backend profiles (`telex backend add/list/show/remove/default/
kinds`, config.toml). The auth axis shipped: `--entra` fetches a token via the GA Azure SDK
(`DeveloperToolsCredential`/`ManagedIdentityCredential`) behind the `entra` feature, validated
live against Azure Postgres Flexible Server; non-entra builds refuse entra backends with an
actionable error. Published release binaries build `--features entra` (batteries-included);
the crate default stays lean (sqlite+postgres) for fast iteration. Remaining from this entry:
the conformance suite (issue #1) and the eventual `telex-core` + `telex-backend-*` crate split.

## 0009 — "Station" as the user/agent-facing name for the running presence (holder + waiter)

- **Date:** 2026-06-17
- **Status:** Accepted

**Context.** The two-process model (0004) gave us precise internal roles — the resident
**holder** and the **waiter** (`telex wait`) loop — but no single user/agent-facing noun
for *the thing you set up to serve an address*. Prose drifted between "a resident holder
keeps the address live," "the waiter loop," and "listener," none of which name the pair as
one concept. The passive directory act ("register an address") and metaphor-losing generics
("listener") were both inadequate. A real telex **station** was the staffed installation
that served a telex **number** — which maps exactly onto holder (holds the line/lease) +
waiter (answerback drum), and gives plain-language invariants a clean noun ("two stations
can't hold one number").

**Decision.** Adopt **station** as the umbrella user/agent-facing term: *the running
presence a session sets up to serve an address — holder + waiter, together.* Frame `attach`
as "start a station on the address" and `detach` as "stop the station and release the
lease," **without renaming any CLI verb** (`attach`/`detach`/`wait` are unchanged — this is
vocabulary, not behavior). Keep **holder** and **waiter** as the precise terms wherever the
two-process mechanics need precision (the SKILL.md re-arm pattern and exit-code table, the
DESIGN.md waiter-loop section, and the `[holder]` operational logs). The canonical
definition and the metaphor vocabulary table live in [DESIGN.md](DESIGN.md) ("Station: the
running presence serving an address"); SKILL.md, README.md, and CLI help adopt the term.

**Consequences.** A terminology/docs pass only — no lease, liveness, or messaging behavior
change and no verb renames. Internal symbol renames (e.g. the `[holder]` log prefix, struct
jargon) were intentionally left untouched to keep the diff reviewable; a future contributor
could deepen the rename if desired. This entry records a naming choice (normally out of
scope per the log conventions) because the term is load-bearing for cross-document
consistency and was an explicit deliverable (issue #8).

## 0010 — Default message `from` to the locally-held lease via a local holder registry; guard un-repliable disposition-required sends

- **Date:** 2026-06-17
- **Status:** Accepted

**Context.** `send`/`reply` derived `from` only from `--from` or `$TELEX_ADDRESS`/`--address`,
with no link to the lease a session actually holds. Forget to set it and the message goes out
`from = None` — **un-repliable** (`telex reply` hard-errors, replies have nowhere to go). A real
session hit exactly this. telex couldn't infer the held address: the holder (`attach`) and `send`
are separate processes, the holder kept no local record, the IPC endpoint name is a *lossy*
`sanitize()` that can't be reverse-mapped, and the backend lease row has no reverse index from
"this session" to "the address it holds" (issue #4).

**Decision.** The holder publishes a **local registry record** once its endpoint is live —
`run_dir()/holders/<sanitized-address>-<pid>.json` carrying
`{ address, backend, host, pid, socket, started_at_ms }` — and `send`/`reply` resolve `from` with
precedence **`--from` > `$TELEX_ADDRESS`/`--address` > the uniquely live local station** for the
current backend. Specific forks chosen (alternatives in parentheses): (a) **liveness by `ipc::ping`,
not pid-alive** — dependency-free, cross-platform, and semantically tighter ("replies here are
answerable"); a hard-killed holder's record is ignored because its endpoint no longer answers.
(b) **`Frame::Pong` now echoes `served_address`** and a ping is "live" only if it matches, closing
a soundness gap where the lossy `sanitize()` could let a probe reach a *different* holder whose
endpoint name collides. (c) **Filename keyed by `(sanitized, pid)`** (not bare `<sanitized>`) so
distinct addresses that sanitize alike don't overwrite each other; the file's `address` field is
authoritative. (d) **Records scoped to `(backend, host)`** so a station on one backend is never
inferred for a send on another (a real cross-backend foot-gun); prune-on-claim and remove-on-clean-
exit keep them tidy. (e) **Guardrails:** a would-be un-repliable send that *requires disposition* is
**refused** (`refused-unrepliable`, exit 4); inference with more than one live station is **refused**
listing candidates (`refused-ambiguous-from`, exit 4); an explicit/env `from` not served locally
**warns** ("replies will queue unwatched") but proceeds. Identity is *defaulted, never forced* —
explicit `--from`/env always win, preserving one-shot reply-to senders, multi-address supervisors,
and operator-as-system sends.

**Consequences.** After `attach`, plain `telex send` "just works" and `$TELEX_ADDRESS` becomes
optional convenience rather than a required convention; SKILL.md's identity section collapses
accordingly. Inference is **local and same-backend only** — a holder on another host or backend
can't be inferred (intended scope). No new runtime dependencies (registry is `serde_json` + `std::fs`;
liveness reuses the existing IPC `Ping`/`Pong`). Resolution touches IPC only when `from` is otherwise
unresolved (or once, to validate a set `from` for the soft-warn), bounded by a ≤250 ms ping timeout,
so configured senders pay ~one local round-trip and unconfigured-but-attached senders pay one ping.
The registry scope key is the **effective store identity** (`profiles::store_key`): the resolved
sqlite path after `--db` override and `~` expansion, and the postgres connection plus `schema` — not
the human-readable `target()` — so schema-isolated multi-store and `--db`-overridden deployments are
correctly distinguished, and the `Pong` echoes the holder's store key so a same-address holder on a
*different* store can't be mistaken for live. Known follow-ups: records left by hard-killed holders
for addresses never re-attached are ignored but not yet garbage-collected (bounded by prune-on-claim
+ the fast-fail ping); `reply` could additionally default `from` from the parent's `to_addr`
(deferred — a preference call, not done here to keep `send`/`reply` uniform).

## 0011 — Durable per-recipient delivery tracking for restart-safe backlog delivery

- **Date:** 2026-06-17
- **Status:** Accepted
- **Supersedes:** the "no per-recipient delivery table" clause of 0007

**Context.** 0007 shipped a derived inbox with *no* delivery table: the holder tracked
delivery only through an in-memory cursor seeded to `max_id` at startup. That made the
`queued-unoccupied` receipt non-durable — a message sent while an address was unoccupied was
skipped on the next holder start and never delivered by `telex wait` (issue #10). Recovering
delivery state across ephemeral holder restarts needs a *persistent* record of what has
actually been handed to a waiter; a derived inbox (disposition-based) conflates "delivered"
with "acted on" and cannot answer "was this ever delivered?".

**Decision.** Add a durable `deliveries(message_id, recipient, occupant, delivered_at_ms)`
table (`UNIQUE(message_id, recipient)`, idempotent `ON CONFLICT DO NOTHING`) plus two
`Backend` methods, `mark_delivered` and `undelivered_backlog(address, upto_id)`. Delivery is
committed at the holder→waiter frame handoff (recorded there). On holder start the queue is
seeded with the undelivered, non-terminally-dispositioned backlog bounded by
`id <= max_id`, which partitions cleanly against the `fetch_after` (`id > cursor`) drain so
nothing is delivered twice. The contract is **at-least-once across holder restarts**, with two
independent do-not-redeliver signals: a delivery record (primary) and a terminal disposition
(secondary, for messages recovered out-of-band via `telex inbox`). A separate table — not a
`delivered` flag on `messages`, nor a per-address watermark — was chosen because it preserves
per-recipient delivery facts for audit and future `cc` fan-out and keeps the two signals
orthogonal.

**Consequences.** Schema grows by one table, auto-migrated via `CREATE TABLE IF NOT EXISTS`
in `init_schema`. The first holder started after upgrading an *existing* database replays that
address's full undelivered, non-terminal history once (including fire-and-forget `fyi`/`note`),
because no prior delivery records exist; a one-time per-address watermark migration (seed
delivery records for pre-existing `id <= max_id`) would avoid this and is the recommended
follow-up before rollout against a database with real history (e.g. telex's own `local.db`).
Exactly-once is explicitly *not* provided — there is no transactional waiter-ack, so a duplicate
is possible only in the narrow window between frame-write and the durable mark. A pre-existing
poll-with-cursor live-holder gap (a Postgres id allocated before the snapshot but committed
after it) is unchanged but now self-heals on the next restart. Conformance covers both
backends; see issue #10 / PR #15.

## 0011 — Holder self-binds to its launching session (pid-watch); ppid-default declined, fd path deferred

- **Date:** 2026-06-17
- **Status:** Accepted (pending validation)

**Context.** Decision 0004 requires the holder's lifetime to track its session — "a
session-owned process, **not** a fully detached daemon" — so session death releases the lease
promptly. Until now that was enforced only by convention (SKILL.md guidance to launch the holder
background + session-bound). A single mis-launch (e.g. Copilot CLI `detach: true`, or any
"daemonize" path) silently orphans the holder: it keeps heartbeating and the address falsely
reports `occupied` for a session that no longer exists. Because the holder is exactly what keeps
the TTL heartbeat alive across turns (0005), an orphaned holder defeats the TTL backstop — the
failure is not self-correcting (issue #5).

**Decision.** Make the binding enforceable **inside the binary**. The holder accepts
`--session-pid <pid>` (env `TELEX_SESSION_PID`); a background watch task polls that pid
(`--session-poll-secs`, default 2s, clamped to the lease liveness window) using a cross-platform
liveness check — `kill(pid, 0)` on Unix, `OpenProcess(SYNCHRONIZE)` + `WaitForSingleObject` on
Windows (`src/session_watch.rs`). When the pid is gone, the task triggers the **existing**
`state.shutdown` signal, so release runs through the *same* tail as `detach`/ctrl-c
(`release_lease` + IPC-endpoint cleanup) — no second release path. The liveness check is
conservative: only a definite "no such process" releases; an existing-but-unqueryable process or
any ambiguous probe error is treated as alive. `--no-session-bind` runs a deliberately persistent
holder and overrides `--session-pid` / the env var. Both the binding **precedence** and the
`$TELEX_SESSION_PID` **parse** happen at runtime (not via clap `conflicts_with`/`env`), so a
malformed or conflicting env value never fails `attach` — `--no-session-bind` always wins
cleanly. Default behavior with no flag/env is unchanged (no binding).

**Consequences.** Even a mis-launched detached holder cannot outlive its session: this turns 0004
from advisory into enforceable and complements the TTL/occupancy model (0005) by stopping the
heartbeat at session death. Scope deliberately bounded against the issue's broader proposal: (a)
**a ppid/parent-pid default is declined** — launchers that spawn-and-return (common for
async/background launches) would leave the holder watching a dead or reparented parent and make
it self-exit immediately, breaking the primary use case; the issue itself names ppid "the central
risk." The sanctioned binding is therefore the explicit `--session-pid`/env, not an implicit
default. (b) The **inherited-fd / `--session-fd` path is deferred** — it is the pid-reuse-immune
upgrade, but the pid-watch satisfies every acceptance criterion; it remains a documented future
option. Known limitation: raw-pid watching is theoretically vulnerable to pid reuse within the
poll window; the fd path is the future fix. Revisit if a runtime needs zero-config binding (would
argue for the fd path) or if pid reuse proves to bite in practice. Cross-references: 0004 (holder
lifetime tracks session), 0005 (TTL/poll baseline).
