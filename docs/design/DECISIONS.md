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
- **Status:** Superseded by 0014 (the per-user local exchange replaces the resident per-session holder; the ephemeral one-shot `wait` client is retained)

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
- **Status:** Accepted (poll-with-cursor delivery superseded by 0013; TTL-heartbeat liveness narrowed by 0017 to the daemon-down backstop role)

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
- **Amended:** the "no per-recipient delivery table" clause is superseded by 0011 (durable delivery tracking, issue #10); the rest of this entry stands.

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
- **Status:** Amended by 0014 ("station" recast as a registration in the local exchange — lease row + attendance record — rather than a resident holder + waiter pair; the term and its lease-verb framing are retained)

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
- **Status:** Accepted (the `from`-default *policy* stands; its *mechanism* — the `run_dir()/holders/` registry keyed off a resident holder — is superseded by 0019, which resolves `from` via the exchange's daemon-native **`ResolveFrom(store_key, session_id)`**, scoped to that store only, never across sessions or stores)

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
- **Status:** Accepted (live-holder drain mechanism superseded by 0013; the `deliveries` table and its restart-recovery role are retained and reinforced)
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

## 0012 — Holder self-binds to its launching session (pid-watch); ppid-default declined, fd path deferred

- **Date:** 2026-06-17
- **Status:** Accepted (pending validation) (relocated by 0017: pid-watch moves from the per-session holder into the exchange as the typed `--watch-pid` backstop; the ppid-default rejection and fd-path deferral stand, reaffirmed by the OQ4 probe)

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

## 0013 — Live-holder visibility via per-recipient delivery state (drop the high-water cursor)

- **Date:** 2026-06-18
- **Status:** Accepted (the undelivered-set drain is retained and reused by the exchange; the **never-prune in-memory `seen`** rationale — which relied on holder restart — is superseded by 0016's bounded, epoch-keyed fast-path for the long-lived daemon)
- **Supersedes:** the live-holder **drain mechanism** of 0005 (poll-with-cursor) and 0011 (the
  in-memory high-water `id > cursor` streaming drain). The `deliveries` table from 0011 is retained
  and promoted from a restart-recovery aid to the live drain's source of truth.

**Context.** 0011 closed the *restart* case of the Postgres commit-order gap but explicitly left the
*live-holder* window open (0011 Consequences: "a Postgres id allocated before the snapshot but
committed after it … self-heals on the next restart"). On Postgres an id is allocated at insert time
but only becomes visible at commit time, and concurrent transactions can commit out of id order, so
an id allocated before — but committed after — a higher id becomes visible *behind* the holder's
monotonic cursor (`fetch_after`: `id > cursor`) and is skipped by the **live** holder until the next
restart (issue #18). SQLite serializes writes (commit order == id order) and was never affected. The
root flaw is that "have I delivered this?" was answered by id ordering, which Postgres MVCC does not
guarantee. A cursor cannot both deliver a high id *now* and re-detect a late lower id later without
re-delivering the high id — so any robust fix needs per-message delivery state, which 0011 already
gives us.

**Decision.** Make the live holder drain the **undelivered set**, authoritative on delivery state,
never on id ordering. A new `Backend::fetch_undelivered(address)` returns every message addressed to
`address` with no `deliveries` record for that recipient and a non-terminal latest disposition,
ordered by id (the 0011 `undelivered_backlog` predicate with the `id <= upto_id` bound removed). The
holder's poll, optional LISTEN/NOTIFY push, and startup all run this one `drain`; the in-memory
high-water cursor is deleted, and with it the now-obsolete `fetch_after`, `max_id`, and
`undelivered_backlog` Backend methods (the trait-surface enumerations in 0006/0011 that named them
are superseded). Intra-holder dedup (don't re-queue a message already buffered but not yet handed
off) uses a **monotonic** in-memory `seen: HashSet<i64>` — an id is queued at most once per holder
lifetime; the `HashSet::insert` under its lock is the drain serialization point. `seen` is
deliberately **never pruned**: pruning on `mark_delivered` would re-open a TOCTOU where a drain whose
`fetch_undelivered` snapshot predates a concurrent mark re-queues the just-pruned id (a duplicate the
cursor model never had). Startup backlog seeding and the live drain thereby unify into the same
query.

**Robustness.** Whether a message is queued depends only on two durable facts — a delivery record
(primary) and a terminal disposition (secondary) — never on its id relative to a cursor. MVCC commit
order is therefore irrelevant: the instant a lower id becomes visible (its txn commits), it is by
definition undelivered and non-terminal, so the next drain tick (poll backstop, or immediately on
push) queues and delivers it. There is no value of any cursor that can exclude it, because there is
no cursor. The only ordering still used, `ORDER BY id`, is presentation-only. This is verified
against real Postgres by a test (`postgres_out_of_order_commit_delivers_lower_id`) that forces two
transactions to commit in reverse id order on independent connections and asserts the lower id is
returned after the higher one is delivered — alongside a deterministic SQLite holder-level test
asserting a waiter actually *receives* the lower id with no restart.

**Consequences.** Behavior delta: the live drain now excludes a message whose latest disposition is
already terminal (e.g. an out-of-band `telex handle` via `inbox` before any drain has queued it; a
message already buffered in the holder's queue is still delivered, since `handle_conn` does not
re-check disposition at the handoff),
making the live path consistent with the backlog path — a deliberate, minor improvement. Cost: with
no id floor, each poll/push tick is O(address history) rather than O(new) — it anti-joins
`deliveries` and the latest-disposition subquery over the address's messages; the **cost is
proportional to the scanned history, not the (small) undelivered result**. Acceptable at telex's
single-user pre-beta scale. A safe id floor is **deliberately
deferred**: advancing a floor to the max delivered/visible id would re-introduce exactly this bug,
because a late-committing lower id sits *below* that floor; a correct floor needs the
contiguous-delivered prefix accounting for the in-flight commit horizon (snapshot `xmin`) — the same
complexity a snapshot-aware cursor was rejected for — and an optional
`dispositions(message_id, recipient, id)` / partial-undelivered index is the eventual mitigation
(target before beta / multi-user / hot-address use).
`seen` grows by one `i64` per distinct message this holder queues over its lifetime (negligible;
holders are session-bound and restart regularly; the drain logs `seen` size for observability on a
pinned long-lived holder); a bounded prune is left for the watermark work.
**Isolation precondition.** Correctness rests on each poll re-snapshotting the latest committed
state — i.e. the backend connection reads under READ COMMITTED in autocommit. A non-default
`default_transaction_isolation` (REPEATABLE READ / SERIALIZABLE, set at the server or role — a
one-liner on managed Postgres) would freeze the snapshot and re-open this race, *without touching
telex code*. `PgBackend::connect_with` therefore pins `SET SESSION CHARACTERISTICS AS TRANSACTION
ISOLATION LEVEL READ COMMITTED`; the holder never drains inside a long-lived transaction.
**Ordering / receipt contract.** Delivery is no longer strictly id-monotonic under out-of-order
commits (the lower id can be delivered after the higher one) — this is the *required* behavior for
the acceptance bar. Receipt order is best-effort-by-id, at-least-once; no `wait`-side consumer treats
the message `id` as a receive high-water mark (the `Request::Wait { since }` cursor field is accepted
but ignored). The pre-existing swallow-and-log on a persistent `mark_delivered` failure means an
un-recorded id stays eligible and is **re-delivered on every restart** (no loss, but the "narrow
duplicate window" framing of 0011 understates this persistent-failure case); a failure counter/cap
is possible future work.
Greenfield: no migration machinery added (pre-first-non-beta, single-user). **CI gap:** the
build/test matrix does not run a real Postgres (issue #19), so green CI does *not* validate this
fix's core behavior — it is validated by the gated Postgres tests, run locally against a real server
(`TELEX_PG_URL=… TELEX_PG_REQUIRE=1 cargo test --test conformance`). See issue #18.

## 0014 — Per-user local exchange (daemon); zero persistent session processes

- **Date:** 2026-06-22
- **Status:** Accepted (design; gated by the design-foundation design-gate)
- **Supersedes:** 0004 (resident holder + ephemeral client) — the holder is removed.
- **Amends:** 0009 (station recast); moots #3 (binary relay / `wait --loop`).

**Context.** Binding presence + delivery transport to an ephemeral per-session resident
holder (0004) is the root cause of telex's recurring staleness: orphaned holders, zombie
`occupied` leases, holder/waiter startup races, dismiss leaving a holder attached, and a
forever-listener starving a session's turn loop. Presence ("address A is attended by a
live agent") is irreducible, but it does **not** need to live in a per-session process;
delivery transport is not session-coupled at all.

**Decision.** Introduce an auto-spawned, single-instance, supervised **per-user local
exchange** (a `telex daemon`) that owns the backend connection(s), the poll/LISTEN-NOTIFY
loop, the durable delivery buffer, the attendance registry, the lease heartbeat (single
writer), the IPC endpoint, and pid-watch. Delete the resident holder: `attach`/`detach`
become one-shot register/deregister and `wait` a one-shot per-turn block, all against the
exchange. The exchange is implicit and zero-config (like `rust-analyzer`/`gopls`). A
**station** is recast as a registration in the exchange (lease row + attendance record),
not a resident process. Full contract in [daemon.md](daemon.md).

**Consequences.** A whole class of per-session-process bugs is removed rather than
hardened: one supervised process instead of N ad-hoc tasks, a single writer of liveness,
a cleaner crash signal, and a freed session turn loop. The cost is the new machinery this
ADR series specifies — singleton identity (0018), the epoch fence (0015), capability/
version IPC and session ownership (0019), the liveness model (0017), and an upgrade floor
(0020). Reopen only if a server-side ownership guard proves un-implementable on both
backends (see 0015). PRODUCT-THESIS.md and DESIGN.md are updated to the local-exchange
framing.

## 0015 — Server-side lease-epoch fence, ordered handoff, and epoch lifecycle

- **Date:** 2026-06-22
- **Status:** Accepted (design; `fencing-proof` node must prove it executable on both backends)
- **Revised by:** 0023 (delivery-commit model).

> **Revised by ADR 0023 (minimal model, 2026-06-23).** The lease-epoch fence + ordered handoff +
> epoch lifecycle in this ADR **stand** (the fence is active for the multi-writer Postgres
> backend). But the **delivery-commit model** it described — `EMIT → waiter-ACK → MARK` with a
> waiter `DeliveryAck` / `delivery_nonce` / `AlreadyDelivered` and "delivered = stdout flush" — is
> **superseded**: the durable consumed-MARK is now triggered by an **explicit agent
> `Ack{address, message_id}`** (epoch-guarded, idempotent on `(message_id, recipient)`), the
> waiter stdout flush is transport-only, and outcomes are `Marked` / `AlreadyConsumed` / `AckNoOp`
> (no delivery row because the recipient was **never delivered**; a **consumed** `(message_id, recipient)`
> row is **retained in v1** and returns `AlreadyConsumed`, **never** `AckNoOp` — any future deletion of
> consumed rows requires the deferred #24 safe per-recipient id-floor / GC) / `NotOwner`. See [daemon.md](daemon.md) §11.3.

**Context.** The lease row is keyed by `address` only with **no owner generation**
(verified: `src/registry.rs`, the backend `claim_lease`/`heartbeat`/`release_lease`), so
on stall/crash/handoff/reclaim an old daemon can write a row it no longer owns
(duplicate delivery, ownership flip-flop). Worse, lease-row fencing alone is insufficient
for *delivery*: the holder emits the `Frame::Message` **before** `mark_delivered` commits
(verified `src/commands/attach.rs:477` vs `:485`), so a graceful handoff or crash can
double-deliver.

**Decision.** Add a monotonic, **never-reused** `lease_epoch` + `owner_instance_id` to the
lease row. Claim/takeover is a **compare-and-set that pins the observed epoch AND owner and
increments the epoch in the backend** (not the client; `NULL` epoch ≠ `0` — a separate
legacy path). **Release does not delete the row** — it clears the owner and **retains the
epoch high-water** (a normative no-delete invariant; deleting would reset the epoch and
break the waiter epoch-filter and "higher epoch wins"). Heartbeat is epoch/owner-guarded,
returns a rowcount, and updates **lease-liveness only** (not `attendance_last_confirmed_at`
— see 0017); a 0-row heartbeat → **self-demote = stop emitting AND stop heartbeating
(relinquish)**. Fence delivery server-side via
`mark_delivered_if_current_owner(...) -> {Marked | AlreadyDelivered | NotOwner}` with the
**at-least-once-preserving order EMIT → waiter-ACK → MARK**: the durable mark commits only
after the wait client has flushed the message to its stdout boundary; any crash/rotation
before MARK redelivers (a duplicate, never a loss); `NotOwner` is fatal (self-demote),
`AlreadyDelivered` is success. Graceful handoff is an **owner-directed atomic transfer**
(one guarded `UPDATE` `P@E → S@E+1`, no ownerless gap, no third-party hijack). Postgres
cross-machine reclaim is expressed **in epochs**, with the stale precondition on a single
backend clock domain. Full contract in [daemon.md](daemon.md) §11.

**Consequences.** This is the real single-writer guarantee and the spine of daemon-down
recovery, upgrade handoff, and Postgres reclaim, and it **strengthens ADR 0011**: the
at-least-once commit point moves from the bare frame-handoff to the **waiter ACK**, closing
a waiter-death-after-frame-write loss window (the earlier "mark-before-frame" ordering,
caught at the design-gate, would have flipped 0011 into at-most-once loss). A distinct
executable `fencing-proof` gate must prove the emit→ack→mark failpoints, epoch monotonicity
across release/cleanup/re-claim, and the handoff crash matrix on both backends
([daemon.md](daemon.md) §17 tests 5/6/7/12/13). Backends gain a rowcount-returning heartbeat, a
non-deleting release, and the typed delivery method (backend-API changes). **Round-2
sharpening:** the `DeliveryAck` is correlated to the exact in-flight
`(connection, store_key, address, message_id, lease_epoch, delivery_nonce)` under a bounded
ACK deadline (so a wrong/late ACK cannot mark the wrong delivery and a wedged waiter cannot
hang the address or `stop --drain`); `mark_delivered_if_current_owner` has **outcome
precedence** (`NotOwner` is returned even if already delivered, so a superseded owner
self-demotes instead of treating `AlreadyDelivered` as success); the graceful handoff adds a
successor-readiness precondition; and a separate **takeover CAS** gated on the
`occupied_stale` *attendance* predicate is specified (the stale-heartbeat claim predicate
does not fit takeover). **Round-3 sharpening (R3-5, R3-S1):** the MARK's ownership-check and
mark are frozen as **one atomic step** — a lease-row lock (`SELECT … FOR UPDATE` on Postgres /
`BEGIN IMMEDIATE` on SQLite) taken before the mark, the **same** lock the owner-directed
transfer and the takeover CAS take, so an ownership rotation cannot interpose *between* the
check and the mark under `READ COMMITTED` (a two-step read-then-mark would reopen the
`AlreadyDelivered`-masks-ownership race at the transaction level); and the ACK deadline's
semantics are frozen (monotonic clock, exclusive boundary, ACK-vs-timer first-wins, and a
repeated-timeout connection quarantine that bounds duplicate-redelivery storms). Reopen if
the guard cannot be implemented/tested for both backends.

## 0016 — `seen`-dedup redesign for a long-lived daemon

- **Date:** 2026-06-22
- **Status:** Accepted (design)
- **Supersedes:** the never-prune in-memory `seen` rationale of 0013 (the drain is retained).
- **Elevates:** #26 from carry to a satisfied design prerequisite.

**Context.** 0013 made the live drain authoritative on per-recipient delivery state and
kept an in-memory `seen: HashSet<i64>` **deliberately never pruned**, justified *because
holders restart regularly*. A long-lived exchange voids that assumption: `seen` would
grow unbounded and carry stale identity across epochs/handoffs.

**Decision.** Keep the durable `deliveries(message_id, recipient)` table (0011/0013) as
the **cross-epoch/cross-restart dedup authority** (unchanged). Replace the unbounded
in-memory set with a **bounded fast-path keyed by `(recipient, message_id, lease_epoch)`**,
seeded from `fetch_undelivered` on claim, evicted on durable mark / terminal disposition /
epoch transition, and **dropped wholesale on epoch loss** (self-demote, takeover). Full
contract in [daemon.md](daemon.md) §13.

**Consequences.** Dedup stays bounded and correct without relying on process restart, and
the epoch-scoped key prevents a stale in-flight identity surviving a fence. #27
(`mark_delivered` cap) and #24 (registry GC) remain carries.

## 0017 — Liveness: sessionEnd hook + typed `--watch-pid`; minimal stale-attendance/takeover; no idle-TTL teardown

- **Date:** 2026-06-22
- **Status:** Accepted (design)
- **Relocates:** 0012 (pid-watch moves from the holder into the exchange).
- **Narrows:** 0005 (TTL survives only as the daemon-down backstop).
- **Revised by:** 0023 (minimal model).

> **Revised by ADR 0023 (minimal model, 2026-06-23).** The non-authoritative-hook +
> `occupied_stale` seq-fenced attendance + epoch-minting takeover model in the Decision below is
> **superseded**, and the heading's "no idle-TTL teardown" no longer holds. The current liveness
> model is: an **authoritative, non-destructive** `sessionEnd` hook (release waiters + mark idle,
> never destroy); the watched **loader pid as a negative-only signal** (pid + start-time guard);
> and a single **idle-TTL ≥ 1 day** non-destructive backstop. Liveness is a UX/latency dial, not a
> correctness gate (delivery is agent-acked). The text below is retained for provenance.

**Context.** With the holder gone, the exchange needs a liveness model that keeps
idle-but-alive sessions instantly wakeable (the operator's explicit requirement) while
not leaving zombies when a dismiss is unhooked. Empirically (live Copilot CLI probe,
1.0.64-1): `copilot.exe` is a supervisor that re-execs an identical-argv inner worker
whose PID is **not** env-exposed **and spawns lazily**, so there is **no usable distinct
per-session PID** beyond `COPILOT_LOADER_PID` — and finding the inner pid would need the
ppid-walk 0012 rejects. Loader-only liveness is therefore weak.

**Decision.** Two paths: the **sessionEnd hook** is the healthy-disconnect *accelerator* —
**non-authoritative (R6-1):** because the separately-spawned hook is given only a *recurring*
`session_id`, it cannot identify its own life, so it sends a `SessionEndHint` (no incarnation)
that triggers a **latched, liveness-vetoed, double-checked** teardown of the exact proven-dead
life (never a live one); a **typed `--watch-pid`** backstop catches the ungraceful case —
`anchor` (any-sufficient) vs `required` (all-necessary) predicates plus a **pid + start-time
reuse guard** (today `process_alive` is pid-only), v1 floor = a single loader **anchor** +
start-time. There is **no idle-TTL teardown**; the precise rule is "no time-based
dismissal of a *live* session, but **positive death evidence triggers immediate
teardown**" via a four-case dismissal-path matrix (hint / watch-pid failure / takeover /
daemon-down TTL). `occupied_stale` (derived from `attendance_last_confirmed_at`, refreshed
**only by a current-seq session-carrying action** — **not** the daemon's own heartbeat
(heartbeat updates lease-liveness only), **not** a bare sessionless `Wait`, **not** a
merely-surviving process, and **not** the hint; on a single backend clock domain) is reserved
for the unobserved-death residual, and **operator takeover** (epoch-minting, atomic at the
exchange) is the **load-bearing** recovery for it. Full contract in [daemon.md](daemon.md) §9–10.

**Consequences.** OQ3/OQ4 resolved with empirical grounding; the hook becomes necessary
and stale-attendance/takeover load-bearing (council E). TTL's only remaining role is the
daemon-down backstop. A design-gate gating test asserts the must-fix case: an unhooked
dismiss whose loader survives goes `occupied_stale` (the daemon's heartbeat does **not**
keep it fresh) and offers takeover, with no teardown of a live-but-idle session. Reopen if
a distinct, reliably-capturable per-session PID appears, or if takeover cannot give a safe
recovery contract.

## 0018 — Daemon singleton identity, lifecycle contract, and Status surface

- **Date:** 2026-06-22
- **Status:** Accepted (design); the gating tests (§17) are `daemon-core` acceptance.

**Context.** "Per-user" must not mean globally user-wide: distinct config roots and
protocol-majors must not collide on one exchange, and auto-spawn must survive a
thundering herd, crashes mid-`wait`, and competing daemons without lying about liveness.

**Decision.** Key the singleton by **`(user SID, config root, protocol-major)`**; the
exchange serves multiple stores and clients pass store identity explicitly (so the
endpoint is daemon-scoped, not address-keyed). Make auto-spawn normative: an OS-level
**spawn-lock** (bind-the-endpoint-as-the-lock), **connect-or-spawn**, **readiness ACK**,
a **`wait` reconnect-on-EOF grace** (a restart/handoff is not a turn failure),
retry/backoff/crashloop guards, daemon-down **exit codes** (0/2/3/4, extended), and a
bounded, **frozen Status field set** (epoch, instance, attendees with last-confirmed/
stale, backoff, recent errors, protocol version). The **Status freeze line** (OQ7):
freeze the field set + the gating tests' observable assertions; `daemon-core` owns
rendering/format. Specify the gating tests (initially five — concurrent first-use, crash-during-`wait`,
competing daemons, handoff duplicates + ownership-loss-around-delivery, intra-daemon
takeover local-eviction). Full contract in [daemon.md](daemon.md) §2–4, §17.

**Consequences.** Cross-profile/version collisions are avoided; the lifecycle is testable
as `daemon-core` acceptance. The Status surface is stable enough for downstream tooling
without over-freezing implementation detail. The design-gate review expanded the gating
set from five to **twelve** and added an explicit **per-backend conformance matrix**
([daemon.md](daemon.md) §17), since the fence's whole point is cross-backend single-writer
correctness.

## 0019 — Daemon-scoped capability/version IPC and daemon-native session ownership

- **Date:** 2026-06-22
- **Status:** Accepted (design)
- **Scope.** This ADR covers **two** linked concerns and was kept as one entry for log
  brevity (splitting was considered and declined): (a) the daemon-scoped, versioned,
  capability-authorized **IPC**; and (b) **daemon-native session ownership** (the
  in-memory `session_id → addresses` authority, `Register`/`Re-register`/
  `DeregisterSession`, the `from`-default rule, and crash recovery).
- **Reshapes:** #23 / PR #31 (drop the filesystem `session_registry` as authority).
- **Supersedes:** 0010's `from`-default mechanism (the local holder registry).
- **Revised by:** 0023 (minimal model) — for concern (b), session ownership.

> **Revised by ADR 0023 (minimal model, 2026-06-23).** Concern **(a)** — the daemon-scoped,
> versioned, capability-authorized IPC and the OS user-private trust boundary — **stands
> unchanged**. Concern **(b)** — session ownership — is **superseded**: the durable `sessions`
> incarnation-currency authority, `(session_seq, nonce)`, the prior-seq CAS / `establish_nonce` /
> `Establish`-`Continue` modes, per-address tombstones, the non-authoritative `SessionEndHint`
> teardown, `occupied_stale`/seq-fenced attendance, and `Takeover{force}` are all **replaced** by:
> the unique ambient `session_id` as identity, **explicit-only in-memory membership** with a
> **`NeedsAttach`** error and no implicit rebuild-from-history (so no resurrection and no
> tombstones), an **authoritative non-destructive** hook, and **explicit agent-acked at-least-once
> delivery** + `message_id` dedup. The lease-epoch fence is retained for the multi-writer Postgres
> backend. The session-ownership text below is retained for provenance.

**Context.** Today `Wait`/`Shutdown` are **unauthenticated** (verified `src/ipc.rs`) and
the endpoint is address-keyed. One exchange serves multiple sessions/stores for one user,
so privileged operations need a proof and the protocol needs skew handling. The sessionEnd
hook (verified on `feature/copilot-session-end-plugin`) runs as a **separate process** and
cannot inherit a secret minted in the earlier `attach` process, so a per-session
capability "held in the session env" is **not obtainable** in v1.

**Decision.** A **daemon-scoped** endpoint with a **Hello/HelloAck version + capability
handshake** that **fails closed** on security-sensitive incompatibility
(`required_capabilities` + `auth_policy_version`; unknown required field/op →
`Incompatible`, never a silently weaker path). Requests carry `store_key` (one exchange
serves multiple stores). A **scoped-capability** model: an **instance `admin_cap`** —
written to the **singleton-scoped** user-private file `<run_dir>/daemon-<H>.cap` (so two
protocol-major-parallel daemons don't clobber one cap) — authorizes privileged RPCs;
`Register`/`ReRegister`/`Wait` are unprivileged. The **OS enforces the user-private trust
boundary** (Windows pipe DACL current-SID-only / Unix `0700` run dir; canonical
owner-private runtime directory; `config_root`'s identity-only role is later clarified by
0025; `O_NOFOLLOW`+atomic cap/lock; **peer-credential
check** before `admin_cap`/data frames; spawn only the canonical executable). v1 is
**same-user trust with NO intra-user isolation** (documented as a deliberate choice;
`per_session_cap` reserved as the path to it). Session ownership is **daemon-native** and
keyed by **`(store_key, session_id)`** with a durable **`sessions` currency authority — a
daemon-assigned monotonic `(session_seq, nonce)` (R6-2) + per-address tombstones** —
**serialized on the `sessions` row** (all `Register`/`ReRegister`/`DeregisterSession`/`Detach`
lock the row and gate on the carried `(session_seq, nonce)`; `Register` carries a **positive
`Establish`/`Continue` mode** — `Establish` is a **prior-seq CAS** keyed by a high-entropy
single-use `establish_nonce` + `expected_prior_seq`, so a live session cannot self-supersede
**and** a previously-used nonce / stale replayed establish can never allocate a new seq or
supersede a quiet-but-live later life, R7-1/R8-1), so a removal is
neither resurrected by — nor applied by — a stale op. The **healthy-disconnect `sessionEnd`
hook is non-authoritative** (R6-1): it sends `SessionEndHint(store_key, session_id, admin_cap)`
with **no incarnation** (a recurring-`session_id`-only hook cannot identify its life) and the
daemon runs a **latched, liveness-vetoed, double-checked** teardown of the exact proven-dead
life; **there is no token-file**. **Explicit** removals (`DeregisterSession`/`Detach`) that hold
the current token are seq-gated. `from` defaults via `ResolveFrom(store_key, session_id)` (never
across sessions/stores) with **opportunistic re-register on `send`/`reply`/`ack`** so a mid-turn
crash does not reintroduce ADR 0010's unrepliable-`from` foot-gun; crash recovery uses a
**`suspect`/`verified`/`lapsed`** state machine. Full contract in [daemon.md](daemon.md)
§6–7, §14. *(The round-3→5 sharpening paragraphs below are retained for **provenance**; where
they describe an earlier `current_incarnation` single column, a loader-minted `<mint_ms>.<nonce>`,
a mandatory token-file, or an incarnation-carrying/authoritative hook, they are **superseded by
this Decision and the round-6 paragraph** — the final model is `(session_seq, nonce)` +
non-authoritative `SessionEndHint` + no token-file.)*

**Consequences.** Resolves OQ6 (proof without an external registry) and OQ8 (durable vs
rebuilt attendance), and closes the design-gate review's must-resolve items on the IPC
trust boundary, sessionless-`Wait`-as-presence, `ReRegister` resurrection, missing
`store_key`, and cap-singleton-clobbering. **Round-2 sharpening:** the anti-resurrection
guard is made **durable** (lease-row columns), **fail-closed** (`ReRegister` MUST carry a
current token), and **frozen** (no daemon-core alternative); `ReRegister` is **session-scoped**
(address-optional) so a foreground `send`/`reply` with no known address can still re-prove
presence after a crash;
client **MUST** verify the server peer + canonical-exe **before** sending `admin_cap`, with a
reuse-safe peer credential and a Windows first-instance exclusivity primitive; and `admin_cap`
carries a no-log/redaction contract. **Round-3 sharpening (R3-2/3/6/7, spar-driven):** a
cross-model spar showed the round-2 per-`(session_id, address)` *generation* was unsound — it
either falsely invalidated a live sibling-address waiter, or (without a session-keyed
authority) let a GC'd tombstone or a same-`session_id` respawn resurrect a removed address. So
incarnation **currency** now lives in a durable **`sessions(store_key, session_id,
current_incarnation)`** authority, and `ReRegister` is **two-gate**
(currency-against-`sessions` **then** union-of-non-tombstoned-rows). `Takeover` is
**fence-then-register** (it fences/evicts/tombstones and leaves a
**bounded pending-bind** reservation; a follow-up `Register` binds — Takeover carries no
session identity), with `owner_instance_id IS NOT NULL` partitioning it from the ownerless
claim. `ReleaseOwnership` (daemon-stop/handoff) is split from **station-removal**: it clears
ownership only, **preserves** the session binding, and does **not** tombstone (so §16 upgrade
continuity holds); only station-removal tombstones. Heartbeat is **bound-rows-only**
(`session_id IS NOT NULL`) so a pending-bind reservation ages into reclaimability instead of
wedging. The **client→server** auth primitive is corrected to `GetNamedPipeServerProcessId` /
connected-socket `SO_PEERCRED` (the prior `ImpersonateNamedPipeClient` is server-side), run
**before any metadata disclosure**. **Round-4 sharpening (R4-1..R4-7):** the `sessions`
authority is **current-only** (one row per `(store_key, session_id)`; a superseded token is
simply `!= current_incarnation`, so no history column and no GC-horizon proof are needed);
`Register` **carries** the `session_incarnation` (a Telex-loader-minted per-life token in
`TELEX_SESSION_INCARNATION`, not a Copilot value) and the **removals are incarnation-gated**
(a delayed old-life `sessionEnd`/`Detach` with a non-current token is a `Stale` no-op, closing
the mirror resurrection where a stale removal kills a *new* same-id life); tombstoned lease
rows are **not GC'd in v1** (consistent with the no-delete invariant — this, not a GC horizon,
is what closes the same-incarnation sibling case), and **automatic** recovery never recreates
an address on `UnknownSession`; the **pending-bind** row is frozen **non-deliverable**
(delivery requires a verified bound session, so a `Wait` cannot bypass fence-then-register);
the SQLite `BackendClock` is a **durable persisted high-water** clock (a process-monotonic
clock would rebase across the restart its persisted timestamps are compared over); `Register`
commits its `sessions`+`leases` writes in **one transaction**; and `Takeover` gains a typed
`TookOver` response. **Round-5 sharpening (R5-1/2, spar-of-record closed):** "one transaction"
is atomic but not serializable under `READ COMMITTED`, so all four currency operations
(`Register`/`ReRegister`/`DeregisterSession`/`Detach`) **serialize on the `sessions` row**
(`SELECT … FOR UPDATE` / SQLite write-tx) with **conditional lease DML**, the incarnation is
**monotonic `<mint_ms>.<nonce>`** and the `Register` bump **rejects a non-newer token** (a
delayed old-life `Register` cannot overwrite a live newer current — the same atomicity-vs-
isolation gap the delivery-mark lock closed); and the `session_incarnation` the gated removals
need is carried in a **mandatory owner-private session token-file** at a path the
separately-spawned `sessionEnd` hook can derive, so the healthy-disconnect path does not
degrade to the TTL backstop (the incarnation is a same-user-readable token, an accidental-race
guard, not a security boundary — v1 no-intra-user-isolation). The daemon-down TTL's dependence
on a trustworthy respawn wall clock is made explicit and **fail-closed via operator
`Takeover`** under backward/slept skew. **Round-6 sharpening (R6-1/2/3, council-of-record):**
the round-5 mandatory token-file is **removed** — it could not give the separately-spawned
`sessionEnd` hook (which has only a *recurring* `session_id`, no per-life env) a trustworthy
per-life token, and a shared file reopened the stale-hook race. The hook is therefore
**demoted to a non-authoritative `SessionEndHint`** (no incarnation) that triggers a
**latched, liveness-vetoed, double-checked** teardown of the exact proven-dead life — liveness
is a *veto*, never authorization; the unhooked-dead residual is reclaimed by stale-attendance /
takeover (which is now **seq-fenced**: only a *current-seq* telex action refreshes attendance,
so a merely-surviving old process cannot keep a dismissed life fresh). The incarnation order is
now a **daemon-assigned durable monotonic `session_seq`** (not a loader wall-clock — removing
the equal-ms/backward-skew defects). `Takeover` gains a **`force` break-glass** mode that
seq-bumps and bypasses the `occupied_stale` time proof, resolving the daemon-down-TTL recovery
self-contradiction. The authoritative-hook path is the **reopen condition**: a Copilot API to
inject the current incarnation into the `sessionEnd` hook env would restore an immediate
seq-gated `DeregisterSession`. Copilot JSON parsing never becomes a core protocol
dependency. The Layer-1 protocol shape is specified here and stabilizes for #12-SDK reuse at
`daemon-core` (the compatibility table is daemon-core-owned). Reopen if a plugin API appears
that lets the hook env be pre-populated from an `attach`-time value (then an authoritative
seq-gated hook returns), or if `wait` Re-register is impossible because the IPC transport masks
socket-EOF.

## 0020 — Minimal upgrade floor and the two-phase legacy/non-epoch cutover rule

- **Date:** 2026-06-22
- **Status:** Accepted (design)
- **Splits:** #6 (minimal floor here; full platform in `seamless-upgrade`, last).
- **Revised by:** 0023 (schema/migration).

> **Revised by ADR 0023 (minimal model, 2026-06-23).** The minimal upgrade floor and the two-phase
> legacy/non-epoch cutover rule **stand**, but the **schema/migration instruction** to create a
> `sessions` currency table atomically with the lease columns is **superseded**: there is **no
> `sessions` table** — the durable layer is lease-ownership (epoch) + the
> `deliveries(message_id, recipient)` message/ack buffer only. The migration creates the lease
> columns + the per-message consumed-state together under one schema-version bump. See
> [daemon.md](daemon.md) §5.1, §14.

**Context.** The first daemon-aware install hits the Windows binary-lock (a running
`telex` process locks the binary during swap — hit live this workstream), and the first
rollout meets **legacy holders** and **non-epoch lease rows**. Occupant-rotation alone
cannot fence a live legacy holder: it ships `Frame::Message` *before* its post-emit
`mark_delivered`, and its `heartbeat` returns no rowcount so it cannot observe
self-demotion (verified `attach.rs`, `sqlite.rs`/`postgres.rs`).

**Decision.** A **minimal upgrade floor** lands in `daemon-core`: a versioned install +
launcher shim, `telex daemon stop --drain` (quiesce + flush in-flight EMIT→ACK→MARK +
owner-directed transfer or non-deleting release), and next-call respawn; **v1 cutover is
forward-only** (a too-old pre-epoch binary is gated closed by the store schema-version).
The legacy cutover is **two-phase, prove-unbound**: **Phase 1** must *prove* no legacy
waiter endpoint is bound — an address-keyed IPC probe with quit/handover that observes the
endpoint gone, **or** quiescing the legacy process; a **bounded stale-window alone is
removed** (a stale heartbeat does not prove the endpoint is unbound — a paused/partitioned/
GC'd/suspended legacy holder can resume emitting). **Phase 2** claims `NULL → 1` via the
explicit legacy CAS (`NULL` is never `0`). Frozen assertion (sharpened round 2, M9): *no
legacy (non-epoch) holder **emits** a new `Frame::Message` after the daemon binds* — an
already-in-flight legacy frame is bounded by at-least-once + `message_id` dedupe (a deduped
duplicate, never loss); the stronger "no frame reaches a recipient" needs a new legacy
drain-barrier verb (trips the reopen condition) and is flagged for `daemon-core`. The
forward-only downgrade barrier is made **executable** (round 2, M10) by an external gate the
old binary cannot bypass: the **store-level legacy-write hard-fail is MANDATORY** (R3-S2/R4 —
the migration renames/constrains the legacy lease columns so a directly-invoked pre-epoch
binary errors before writing a non-epoch row), with the launcher/store lock as **additional**
defense (it is bypassable by direct invocation, so it does not replace the hard-fail); plus a
per-store exclusive, crash-safe migration that creates **both** the new lease columns and the
`sessions` table atomically at one **schema-version publish point** (R6-Se). Exercised by
dedicated real-legacy-holder and schema-downgrade gating tests on both backends, including a
**directly-invoked** (not via the shim) pre-epoch binary. Full rollback/gc/UX and any
epoch-aware downgrade *framework* are deferred to `seamless-upgrade`. Full contract in
[daemon.md](daemon.md) §16 + §3.4 and ADR 0024 (legacy-holder cutover).

**Consequences.** The binary-lock is handled and the cutover is deterministic and
*verified*; hard, forward-only cutover of existing sessions is acceptable (ratified).
*Preserved minority:* one reviewer held occupant-rotation alone suffices; the
prove-unbound rule was adopted because the legacy heartbeat cannot self-demote and a stale
heartbeat is not proof of an unbound endpoint (sharpened by the design-gate review). Reopen
if Phase-1 prove-unbound cannot be realized via the address-keyed IPC probe + process
quiesce (i.e. it needs a new IPC verb), which would make this an architectural change.

## 0021 — Verb + docs/SKILL cutover; design-layer relocation to `docs/design/`

- **Date:** 2026-06-22
- **Status:** Accepted (design)

**Context.** The cutover must not rename verbs or describe a dead holder/waiter model
mid-workstream, and the design layer needed a home that the Streamliner manifest expects
(`docs/design/*.md`) and that node-worker sessions own — distinct from the loose,
ad-hoc-edited root vision docs.

**Decision.** **Keep the verb names** (`attach`/`detach`/`wait`; now one-shot against the
exchange); Register/Detach/Ack are IPC operations, not CLI renames (the round-1..6
`Re-register`/`DeregisterSession` are superseded by ADR 0023's `NeedsAttach`-re-register /
agent-`Ack` / `Detach` model); the
held-stream `SessionConnect` is not adopted (preserved dissent). **Hide** the `telex
daemon` entrypoint from normal help. **Single-source the skill**: root `SKILL.md` stays
the canonical file (embedded via `include_str!` for `telex skill`, with a `--raw` form);
the plugin consumes the same file (manifest pointer, else a thin wrapper `exec`ing `telex
skill --raw`) — no divergent copy. `SKILL.md` + plugin-doc narrative updates land **with
`daemon-core`**, never mid-workstream. **Relocate the design layer**: `DESIGN.md` and
`DECISIONS.md` move to `docs/design/` (joined by the new `daemon.md` and `index.md`);
`PRODUCT-THESIS.md`, `TELEX.md`, `DISPATCH.md`, `README.md`, and the binary-embedded
`SKILL.md` stay at the repository root.

**Consequences.** No rename/deprecation debt; instructions never describe a dead model
mid-workstream. The relocation **deviates from issue #34's "keep the design layer at the
repo root"**: it was builder-directed during shaping and is flagged for orchestrator
reconciliation (updating the workstream brief/issue text is an orchestrator action). The
`telex skill` embed path is preserved (`SKILL.md` did not move). Full verb/skill detail in
[daemon.md](daemon.md) §15.

## 0022 — Fail-closed startup portability and path-resolution policy (deferred to `daemon-core`)

- **Date:** 2026-06-23
- **Status:** Accepted (design)
- **Refines:** 0018 (singleton identity / startup).

**Context.** [daemon.md](daemon.md) §7.2 makes startup **fail closed** when `config_root`/
`run_dir` are not owner-private or the `daemon-<H>.cap` cannot be created owner-only. That
requirement and its fail-closed behavior are correct and frozen — but the design never pinned
*where* those paths resolve from, nor what to do on a filesystem that **cannot represent**
owner-only permissions. On a normal laptop install this never fires; the post-merge review
surfaced that the **unattended environments where agents increasingly run** — arbitrary-uid
containers, NFS/SMB/9p mounts (WSL2, Docker Desktop), unset `$HOME`/`$XDG_RUNTIME_DIR`, redirected
Windows profiles — are exactly the ones that trip it, where the failure is **total and
unwatched**.

**Decision.** Keep the **requirement** (owner-private paths, owner-only cap) and the
**fail-closed** behavior frozen (§7.2/§2.3), with owner-only framed as an **effective
permission / ACL / DACL postcondition** (explicit `0700`/DACL is necessary but not sufficient;
ambiguous/inconclusive representations classify as cannot-enforce → fail closed). **Defer the
path-resolution algorithm and the filesystem-portability policy to `daemon-core`**, with a
recorded recommended direction ([daemon.md](daemon.md) §7.4): (1) **platform-scoped**
deterministic resolution with an explicit override and explicit owner-only creation — Unix
`TELEX_RUN_DIR` → `$XDG_RUNTIME_DIR` → `$HOME/.local/state`, Windows `TELEX_RUN_DIR` → local
`%LOCALAPPDATA%` (never a redirected profile by default); (2) a distinct, **actionable** error
for "cannot enforce owner-only" vs "permission denied"; (3) prefer `$XDG_RUNTIME_DIR`/tmpfs or
local `%LOCALAPPDATA%` for runtime artifacts; (4) the remedy is **path-first** (`TELEX_RUN_DIR`/
tmpfs on local owner-private storage), and the **single-tenant opt-out** (e.g.
`TELEX_TRUST_ENV=single-tenant`) is a **narrow last resort** — asserting no shared/host-mounted
`run_dir`/socket/lock/cap (not a blanket container/VM relaxation), **opt-in and audited, never a
silent fallback**, and a builder/operator policy call. Fail-closed **actionability** is part of
the operability contract even though the message text is `daemon-core`'s.

**Consequences.** The owner-private-rejection failpoint stays gated (§17 test 14), extended with
the **cannot-enforce-owner-only** filesystem case and the **actionable-error** requirement (the
error **names the configured run-dir override** generically; `TELEX_RUN_DIR` is the recommended
example, not a frozen knob); the resolution order and the single-tenant opt-out get conformance
points when `daemon-core` fixes them. The single-tenant opt-out is a **new trust-model surface**
flagged for builder/operator sign-off (this ADR recommends, it does not freeze the knob). Reopen
if a target deployment needs owner-only relaxation by default, or if a portable
owner-only-enforcement primitive removes the need for the opt-out.

## 0023 — Minimal session/presence/delivery model: supersede the incarnation-currency machinery

- **Date:** 2026-06-23
- **Status:** Accepted (design)
- **Revises:** 0017 (liveness), 0019 (daemon-native session ownership), 0015 (the delivery-commit
  model — waiter-ACK → agent-ACK), and 0020 (the `sessions`-table migration instruction) —
  supersedes their session-incarnation/currency / `occupied_stale` / force-takeover / waiter-ACK /
  `sessions`-schema machinery.
- **Process:** post-merge builder re-examination + a multi-model advisory **council** (5
  heterogeneous members across GPT/Claude/Gemini; HIGH-confidence, genuine-sharper convergence),
  then two builder refinements. The council synthesis is retained out-of-repo for audit.

**Context.** ADRs 0017/0019 (ratified over 11 review rounds + a cross-model spar) built an
elaborate **session-incarnation currency** machinery — a durable `sessions` table with a
daemon-assigned `(session_seq, nonce)`, a prior-seq CAS with
`establish_nonce`/idempotency-horizon/observe-retry/cap-exhaustion, a **non-authoritative**
`sessionEnd` hook with latched/double-checked/liveness-vetoed teardown, `occupied_stale`
seq-fenced attendance, `Takeover{force}` nonce rotation, per-address tombstones, and an implicit
re-register-from-history. Post-merge, the builder (the authority on the actual Copilot CLI
harness) established that the **premises this machinery rests on are false or were never needed**:

1. **`session_id` is unique and stable** — one session's id, preserved across dismiss/exit/resume,
   **never reused for a different session**. (The "`session_id` recurs across sequential lives"
   premise behind the incarnation token is false.)
2. **The agent runs `telex attach` itself**; there is **no loader/plugin establisher** and **no
   env-injected per-life token** (the plugin only adds the `sessionEnd` hook). The
   incarnation-threading the design assumed has no implementer.
3. **Liveness need not be a correctness gate.** Detached waiters reaped by the exchange (sessionEnd
   hook OR loader-pid death) with **non-destructive** presence (release waiters + mark idle, never
   destroy) make imperfect liveness a UX/latency dial.
4. **Delivery correctness comes from explicit agent ack + at-least-once + `message_id` dedup**, not
   from a per-EMIT waiter event. The waiter's stdout flush is transport only.

Under these, almost the entire incarnation edifice defends a problem that cannot occur.

**Decision (the minimal model — full contract in [daemon.md](daemon.md) §5–6, §9–11, §14).**

- **Identity = the unique, ambient `session_id`.** No incarnation token, no `(session_seq,
  nonce)`, no `TELEX_SESSION_INCARNATION`.
- **Membership is explicit-only and in-memory.** `telex attach` (`Register`) establishes it; the
  exchange returns a **`NeedsAttach`** error for an unknown session/address and **never implicitly
  rebuilds membership from history** — so a removed address is **never silently resurrected**, and
  **tombstones are unnecessary** (the over-correction guard the council required is satisfied by
  *removing implicit rebuild* rather than by *adding durable tombstones*). Only an explicit
  re-attach re-establishes a station.
- **The `sessionEnd` hook is authoritative but NON-DESTRUCTIVE** (release waiters + mark idle).
  With a unique `session_id` and a non-destructive, self-healing action, the round-6 "the hook
  cannot identify its life" problem **dissolves**; a late/spurious hook costs at most one waiter
  re-arm.
- **The watched LOADER pid is a negative-only liveness signal** (pid + start-time reuse guard);
  loader-alive is never positive presence.
- **Delivery = durable at-least-once + explicit `telex ack <message_id>` (immediately on read) +
  dedup by `message_id`.** The durable consumed-MARK is triggered by the **agent ack** (not the
  waiter flush) and is **epoch-guarded** on the multi-writer path; unacked → redeliver.
- **A single idle-TTL ≥ 1 day** is a non-destructive backstop releasing only presumed-dead waiters
  (the unhooked-dismiss + loader-alive case); it **never caps legitimate idle** — station
  membership + the durable message buffer persist indefinitely, so a session may idle for days and
  still wake on a new message. (Replaces `occupied_stale`/`stale_after`/seq-fenced attendance.)
- **Writer authority:** an **OS-singleton** (Unix flock/fcntl + AF_UNIX bind / Windows
  named-mutex + named-pipe first-instance, per config root) **plus a canonical-store advisory
  lock** (per SQLite store, keyed by a **config-root-invariant** file-id lock namespace — not under
  `run_dir` — closing the cross-config-root aliasing hole) are the
  single-host writer authority; the **lease-epoch fence is KEPT and active for the multi-writer
  Postgres backend** (Postgres is in v1 scope — not deferred), arbitrating delivery ownership
  across per-host exchanges. The **live** ordered handoff is the Postgres story; SQLite upgrades
  use release + next-call respawn (no live two-daemon overlap). A simple **operator reset**
  (mark idle / release waiters) replaces `Takeover{force}` (no epoch-minting-for-eviction, no
  force-nonce rotation — those existed only to invalidate incarnation tokens).

**DELETE** (from 0017/0019 / daemon.md): the `sessions` incarnation-currency table +
`(session_seq, nonce)`; prior-seq CAS, `establish_nonce`, idempotency horizon, observe/retry,
cap-exhaustion; the non-authoritative-hook demotion + latched/double-checked/liveness-vetoed
teardown; `Takeover{force}` + force-nonce rotation; `occupied_stale` / attendance-staleness /
`stale_after` / seq-fenced attendance; the waiter `DeliveryAck` / "delivered = stdout flush" /
`delivery_nonce`-as-delivery; the `Register{Establish/Continue}` modes, `ReRegister` /
`DeregisterSession` incarnation params, `Stale`/`Conflict`/`NeedsEstablish` errors,
`TELEX_SESSION_INCARNATION`; per-address tombstones and implicit re-register-from-history.
**KEEP:** the daemon-scoped capability/version IPC + OS trust boundary of 0019 (unchanged); the
lease-epoch fence (active for Postgres); durable at-least-once + `message_id` dedup;
same-user-trust / no-intra-user-isolation.

**Consequences.** A large reduction in accidental complexity, elimination of the opaque
client-threaded token (and its manual-threading UX wrinkle), and a delivery model that is safe for
detached waiters. The change reverses a spar-validated decision, but that spar reasoned under
premises (1) and (4), which were revised; the council re-derived the one genuine residual
(membership resurrection) and confirmed it is closed by *removing implicit rebuild*. This ADR is a
**post-merge design revision** to the design-foundation deliverable (Refs #34), flagged for
orchestrator reconciliation.

**Reopen conditions.** *(Summary; the **canonical** reopen register is [daemon.md](daemon.md)
"Design assumptions and revisit conditions", which this and the PR body link to.)*

- A proven mechanism by which `session_id` is reused across distinct sessions → the incarnation
  fence returns.
- A proven path where the plugin/loader can inject a per-life token into every verb + the hook →
  an authoritative seq-gated hook + threading become viable again.
- A multi-writer / non-self-serializing **single-host** backend, or zero-downtime hot handoff →
  the OS-singleton alone is insufficient; the epoch's role expands.
- Evidence that same-session membership mutations can arrive reordered over IPC → add a
  **server-side** (never client-threaded) monotonic membership op-seq.
- The `sessionEnd` hook does **not** fire on in-terminal dismiss (to be spiked) → the idle-TTL
  becomes the primary dismiss bound rather than a backstop.

## 0024 — Legacy-holder cutover (one-time migration off the resident holder)

- **Date:** 2026-06-24
- **Status:** Accepted (migration; `daemon-core` acceptance)

This is a **one-time migration** concern, not part of the standing daemon design (which owns
lease epochs from `1`). It is recorded here so the durable contract in [daemon.md](daemon.md)
stays free of transitional framing while the cutover mechanism is preserved.

**Context.** The first daemon-aware rollout meets **legacy holders** (resident `attach`
processes) and **non-epoch lease rows** (`lease_epoch IS NULL`). Occupant-rotation alone is
insufficient: a legacy holder ships `Frame::Message` (`attach.rs:~477`) before its post-emit
`mark_delivered` (`~485`), and its `heartbeat` returns `Result<()>` with no rowcount so it
cannot observe self-demotion; if the daemon rebinds the address's waiter endpoint, two
endpoints emit independently regardless of any post-emit row fence.

**Decision — two-phase, prove-unbound:**

- **Phase 1 — prove-unbound (drain).** Before binding its own waiter, the daemon-aware claimant
  MUST establish that **no legacy waiter endpoint is bound** for the address, by one of: (1) an
  **address-keyed IPC probe** to the legacy endpoint carrying a quit/handover signal, observing
  the endpoint gone/closed; or (2) **terminating/quiescing** the legacy holder process (same
  user). A bounded **stale-window wait alone is NOT sufficient** — a `SIGSTOP`/paused process,
  partitioned backend, long GC, host sleep, or clock skew can age the heartbeat out while the
  endpoint stays bound and later resumes emitting. A stale-window MAY be used only as a
  *secondary* timeout after a probe has already shown the endpoint gone.
- **Phase 2 — claim.** Only after Phase 1 proves unbound, claim via the explicit legacy CAS
  `UPDATE ... SET lease_epoch=1, owner_instance_id=:me WHERE address=:addr AND lease_epoch IS
  NULL` (ownership/fence columns only). `NULL` is never treated as `0` in the normal claim
  predicate; the row gets its first epoch (`1`) exactly once, after which the rowcount-returning
  epoch-guarded heartbeat/release apply.

**Cutover gating assertion.** *No legacy (non-epoch) holder **emits** a new `Frame::Message`
after the daemon's waiter binds.* Exercised by a dedicated migration test that starts a real
legacy holder / non-epoch lease on both backends — a **migration** acceptance test, separate from
the standing-design [daemon.md](daemon.md) §17 matrix. Hard cutover of existing sessions is
acceptable (ratified).

**In-flight legacy frame (M9).** Phase-1 prove-unbound proves the legacy *endpoint* is closed,
not that zero frames are in flight: a legacy holder may have already written a frame to its own
wait client before the endpoint closed, and that client can flush to the recipient after the
daemon binds. The assertion is therefore "no legacy holder **emits** after the barrier," not "no
frame reaches a recipient": an already-in-flight legacy frame is bounded by **at-least-once +
`message_id` dedupe** (the recipient dedupes it against the daemon's redelivery of the same
`message_id`), so it is a deduped duplicate, never loss.

**Stronger alternative (reopen).** A real drain barrier (quiesced + zero in-flight legacy `Wait`
handlers + endpoint closed) would let the assertion be "no frame reaches a recipient," but needs
a **new legacy IPC verb** (the legacy IPC exposes only `Shutdown`). `daemon-core` / the builder
may adopt it; this decision takes the in-place at-least-once-dedupe resolution and flags the
stronger option.

**Forward-compatibility note (durable).** The standing contract retains only this: a
`lease_epoch IS NULL` row is treated as **unowned/foreign** and is **never conflated with `0`**
([daemon.md](daemon.md) §5.1 / §11.1). Once rows carry `lease_epoch >= 1`, an old pre-epoch
binary must not run against the store (it would write non-epoch rows and reset the fence); the
store schema-version gates a too-old binary closed.

## 0025 — Directory role taxonomy and Windows local runtime state

- **Date:** 2026-06-25
- **Status:** Accepted (`daemon-core` acceptance)

**Context.** Owner-private validation on Windows exposed two different concerns during
`daemon-core`: the security-critical runtime directory (`run_dir`) contains the daemon capability
file and singleton runtime artifacts, while `config_root` currently contributes only canonicalized
identity material to the singleton key. Treating both paths as one frozen owner-private invariant
made ordinary same-user Windows profile ACL inheritance look like a runtime security failure and
encouraged relaxing the wrong surface. At the same time, Windows home/profile locations can be
redirected or roaming, while runtime secrets and lock state should be local-only.

**Decision.** Keep `run_dir` and cap files strict: repair owner/protected DACL and then fail closed
with binary owner/DACL/ACE read-back. On Windows, resolve the default `run_dir` under local app data
(`%LOCALAPPDATA%\telex\run`) instead of under `TELEX_HOME`; an explicit `TELEX_RUN_DIR` override
still wins. Treat `config_root` as identity-only: create and canonicalize it, but do not require
owner-private validation unless a future change stores authority material there.

**Consequences.** This is a security improvement, not a weakening: the authority-bearing cap leaves
roaming/profile-managed home by default, while the identity-only path no longer trips strict runtime
validation. Unix is intentionally unchanged because its daemon socket lives under `run_dir` and the
path is part of rendezvous compatibility. If `config_root` later stores caps, tokens, locks, private
keys, or other authority material, this decision must be revisited and `config_root` must become
owner-private + fail-closed. Existing pre-upgrade Windows cap/runtime files under the old home-based
default become stale/inert; the named-pipe singleton and canonical-store lock still preserve
correctness, and upgrade guidance should prefer `daemon stop --drain` before replacing a running
binary.

## 0026 — `telex wait --out-dir` outcome artifacts for detached delivery

- **Date:** 2026-06-25
- **Status:** Accepted (`daemon-core` acceptance)

**Context.** The local-daemon waiter UX runs `telex wait` as a single-shot **detached** background
task: the host runtime wakes the agent on the waiter's completion, and the agent reads the delivered
message and re-arms. Dogfooding on Copilot CLI (Windows) showed two host-runtime facts that break a
naive "capture stdout / read the shell exit code" approach: (1) the detached shell does **not** return
the child's stdout, and (2) the detached PowerShell wrapper string-interpolates the command before the
child runs, so any `$variable` (e.g. `$run`, `$LASTEXITCODE`) is stripped and an inline "redirect
stdout to a file, capture `$LASTEXITCODE`" waiter silently writes nothing. The previous skill guidance
recommended exactly that inline-variable pattern, so it failed in practice. A `.ps1 -File` wrapper
works, but pushes shell-specific scaffolding onto every agent and runtime.

**Decision.** Make robust detached delivery a first-class telex feature instead of a shell recipe. Add
`telex wait --out-dir <DIR>`, which writes the outcome to files in `<DIR>`: `message.json` (on delivery
only), `status.json` (always: `outcome`, `exit_code`, `detail`, `address`, `written_at_ms`),
`exit.code` (always, the integer code, written **last** as the completion marker via temp-file +
rename), and `wait.pid` (at startup, before blocking). stdout/stderr behaviour and exit codes are
unchanged; the artifacts are additive. The skill now arms the waiter with a single **variable-free**
command — `telex wait --address <addr> --out-dir
<literal-dir>` — so there is nothing for a detached wrapper to mangle, and instructs agents to trust
the artifact `exit.code` over the runtime-reported detached exit code, and never to use task-list
status (`list_powershell`) as the armed/done signal.

**Consequences.** Detached delivery no longer depends on host stdout capture or shell-variable
survival, which makes the waiter portable beyond Copilot CLI. The file artifacts are **transport
only**, exactly like the stdout flush ([daemon.md §3.2.1](daemon.md), §11.3): they are not the consumed
mark, which still fires only on the explicit agent `ack`, so an at-least-once redelivery after a crash
is preserved. `exit.code` ordering is the documented "fully written" contract; readers that observe it
can read the sibling files without a partial-write race (the host only wakes the agent after the child
exits, so there is no concurrent reader/writer). Because `message.json` may contain the message body,
artifacts are owner-only on Unix (dir `0700`, files `0600`), and a reused `--out-dir` clears any stale
`message.json` on a non-delivery outcome; Windows local app data / `%TEMP%` are already per-user. The
legacy stdout JSON path is retained for attached and `--file`-based callers, so this is purely additive
and back-compatible.

## 0027 — Station stop, live waiter registry, and status reconciliation

- **Date:** 2026-06-25
- **Status:** Accepted (`daemon-core` acceptance)

**Context.** Dogfooding the detached waiter UX exposed teardown friction: `detach` releases durable
membership, but the launching agent does not get a usable process handle for its detached waiter from
Copilot CLI (`pid: unknown`). Operators had to enumerate OS processes and match command lines to stop
a waiter, and `telex daemon status` was misleading because the CLI requested the daemon's intentionally
minimal projection (`members: []`, `stores: []`) even while a one-shot attached station was live.
There was also a correctness concern: if a waiter survived teardown, the next message might be
delivered into an unread detached process.

**Decision.** Add first-class station teardown and waiter observability. `Wait` IPC now carries the
client waiter's pid/start-time, and the daemon records live waiters in an independent registry keyed by
a daemon-assigned `waiter_id` (not pid, so pidless/protocol-valid waiters and same-pid edge cases are
still tracked). Detailed status exposes the top-level `live_waiters` list and per-member
`live_waiters`, and `telex daemon status` requests that detailed projection using the local admin cap.
`telex status --address`, `telex address show`, and `telex address list` overlay live daemon membership
on durable lease occupancy so those operator views agree. Add `telex station stop --address <addr>` as
the symmetric teardown command: mark the station idle so blocked waiters return `PresenceEnded`, wait
for tracked waiters to drain for a short grace window, then perform the durable detach/tombstone. The
command returns a typed `StationStopped` summary with waiter counts and any remaining live waiter
records.

**Consequences.** Normal teardown no longer requires OS process hunting, and a stopped station is
provably unoccupied: a message sent after `station stop` remains queued until a future attach/wait
rather than being consumed by an orphan waiter. Plain `detach` remains available and remains terminal;
tests now prove it does not consume a later message either, but `station stop` is the recommended
operator flow because it waits for the live waiter to exit and reports any leftover waiter. This is a
minor IPC bump (`1.1`) with a required `station_lifecycle_p8` capability, so new clients fail closed
against older daemons instead of sending unknown teardown requests.

## 0028 — Only `attach` auto-spawns the local daemon

- **Date:** 2026-06-25
- **Status:** Accepted (`daemon-core` acceptance)

**Context.** Detached-wait dogfood showed that allowing `wait` to auto-spawn is too permissive: if a
waiter is launched from the wrong environment/profile (or from an old hardcoded binary path), it can
create a parallel daemon and station instead of failing loudly. The intended recovery loop already has
a clear spawning verb: `attach`.

**Decision.** Restrict normal auto-spawn to `attach` / `request_connect_or_spawn`. Other verbs connect
to an existing daemon only. `wait` may reconnect/re-register during its grace window if a replacement
daemon already exists, but if no daemon is running it exits 3; the agent must run `telex attach` and
then re-arm. `send`/`reply`/`ack`/`detach`/`station stop` likewise do not create a daemon as a side
effect.

**Consequences.** A missing daemon is now an explicit station-recovery event instead of a hidden
side-effect. This slightly reduces transparent restart recovery for non-attach verbs, but prevents the
worse failure mode where a detached waiter silently creates or talks to the wrong singleton/profile.
Real-process tests assert that `wait` and `send` without a daemon do not spawn one.

## 0029 — One live waiter per station

- **Date:** 2026-06-25
- **Status:** Accepted (`daemon-core` acceptance)

**Context.** Dogfooding found that the natural "arm next waiter before ack" loop could create multiple
concurrent waiters for the same `(store_key, session_id, address)`. Because the durable delivery row is
not consumed until explicit `ack`, each concurrently armed waiter could be handed the same
`message_id`, which looked like a consumed message redelivering indefinitely.

**Decision.** The daemon accepts at most one live waiter per station. A concurrent second `Wait` for
the same `(store_key, session_id, address)` returns `PresenceEnded` and records a `ConcurrentWaiter`
status/audit entry; it is not allowed to fetch or emit a message. Skill guidance now says to read the
message, `ack`, dedupe by id, then re-arm before longer processing.

**Consequences.** The documented detached waiter loop is duplicate-free without relying solely on
operator discipline. Telex remains at-least-once across crashes and unacked delivery, but it no longer
fans one unacked `(message_id, recipient)` to sibling live waiters for the same station. Multi-recipient
fan-out is unchanged: acking recipient A does not consume recipient B.

## 0030 — Attention-gated waits for focused work

- **Date:** 2026-06-26
- **Status:** Accepted (`daemon-core` acceptance)

**Context.** The detached waiter loop should not make every background/fyi message feel urgent while
an agent is already doing foreground work. At the same time, truly urgent messages still need a wake at
the next turn boundary. The daemon already carries message attention levels and a `Wait` attention
field, but the CLI had no way to request a threshold filter.

**Decision.** Add `telex wait --min-attention <interrupt|next-checkpoint|background|fyi>` as an
inclusive priority threshold. Priority order is `interrupt` > `next-checkpoint` > `background` > `fyi`.
Bare `telex wait` remains unfiltered. Filtering is eligibility-only and preserves oldest-first order
among eligible messages; lower-priority skipped messages remain pending in the durable buffer. The
focused-work skill pattern is: arm `--min-attention interrupt`, do the current work, then at a
checkpoint drain/ack/disposition buffered lower-priority messages and re-arm in the appropriate mode.

**Consequences.** Agents get a first-class "urgent-only while busy" phase without multiple concurrent
waiters. Because older daemons would ignore unknown Wait fields, this is protocol/capability gated with
minor `1.2` and `wait_min_attention_p9`. New clients fail closed against older daemons rather than
silently treating an interrupt-only wait as unfiltered.

## 0031 — Station health exposes unattended backlog

- **Date:** 2026-06-26
- **Status:** Accepted (`daemon-core` acceptance)

**Context.** A station can hold membership and an epoch lease while having no live waiter. That is a
normal short-lived state immediately after a waiter delivers and the agent is handling the message, but
it becomes operationally dangerous when unconsumed messages are queued and no waiter is armed. In the
poll-based local daemon this looks "occupied" to senders but no one is attending the queue.

**Decision.** Derive station health in status from live waiter count, recent waiter delivery, idle
state, and pending unconsumed delivery count. The status values are `armed`, `recently_delivered`,
`unattended`, `unattended_with_backlog`, and `idle`. `recently_delivered` is a short grace state after
a waiter emits a message, so normal handling before re-arm does not look unhealthy. The high-signal
warning is `unattended_with_backlog`: no live waiter and at least one pending unconsumed delivery.

**Consequences.** Operators and orchestrators can detect a stalled station without querying SQLite
tables directly. The daemon does not auto-deliver or auto-rearm; it surfaces the condition so an agent
can run `attach`, drain/ack the backlog, and arm a waiter.

## 0032 — Delivery role metadata for primary vs CC recipients

- **Date:** 2026-06-26
- **Status:** Accepted (`daemon-core` acceptance)

**Context.** In multi-party coordination, `--to` is the primary actor and `--cc` recipients are often
visibility-only observers. A waiter woken for a CC delivery previously saw `to` as another address and
`requires_disposition` as a message-level flag, with no cheap way to tell whether the current station
was the primary recipient or a CC observer.

**Decision.** Add delivery context to wait/read/inbox surfaces: `delivered_to`, `primary_to`, parsed
`cc`, `delivery_role` (`to` / `cc` / `unknown`), and
`requires_disposition_for_current_recipient`. Wait's flat `message.json` preserves existing fields and
adds these context fields; `read --address` exposes a `delivery` object; inbox items include the same
context inline.

**Consequences.** Agents can branch correctly before ack/disposition: primary recipients can act on
required dispositions, while CC recipients can observe without mistaking message-level `to`/required
flags as their own workflow obligation. Later CC auto-seen/disposition-safety changes build on this
metadata.

## 0033 — CC deliveries are visibility-only and auto-seen

- **Date:** 2026-06-26
- **Status:** Accepted (`daemon-core` acceptance)

**Context.** Dogfooding used CC as visibility-only fan-out. Under the transport ack model, CC delivery
rows were pending like primary rows, so a CC recipient that did not manually `ack` would receive the
same observer copy on every `wait`, wedging its waiter behind traffic it was not expected to act on.

**Decision.** Treat CC delivery rows as auto-consumed/seen for transport. They remain visible through
`inbox --all` and `read` with `delivery_role: "cc"`, but they are not eligible for `wait` delivery and
do not require manual `ack`. Primary `--to` deliveries remain pending until explicit ack, and
multi-recipient visibility is still durable/auditable via the message row and inbox/read views.

**Consequences.** CC observers no longer need to run transport `ack` just to advance their waiter.
This preserves the intended convention: the `--to` recipient acts/dispositions, CC recipients observe.
It also reduces the chance of CC recipients accidentally treating message-level `requires_disposition`
as their own workflow obligation.

## 0034 — Workflow dispositions default to the current recipient

- **Date:** 2026-06-26
- **Status:** Accepted (`daemon-core` acceptance)

**Context.** `ack` is per-recipient, but terminal workflow dispositions previously defaulted to the
message's primary `--to` address. A CC observer running `telex handle --id <id>` could therefore mark
the primary recipient's work handled. This violated the dogfood convention that CC recipients observe
while the `--to` recipient owns action/disposition.

**Decision.** Disposition commands (`handle`, `reject`, `close`, `defer`, `escalate`) default to the
current global `--address` when it is a recipient of the message (`to` or `cc`). If no current address
is provided, or the current address is not a recipient, the command fails and asks for explicit
`--recipient`. Explicit `--recipient` remains available for intentional cross-recipient recording.

**Consequences.** A CC observer can no longer accidentally clobber the primary recipient's disposition.
Primary actors keep the natural `--address <me> handle --id <id>` flow, and scripts without any address
must be explicit about whose workflow state they are changing.

## 0035 — Replies support CC visibility

- **Date:** 2026-06-26
- **Status:** Accepted (`daemon-core` acceptance)

**Context.** Before this change, `send` supported CC visibility recipients but `reply` did not. Agents
had to choose between preserving thread context (`reply`) and notifying observers (`send --cc`), which
was a poor fit for multi-party coordination.

**Decision.** Add `reply --cc` with the same repeated/comma-separated parsing as `send --cc`. The reply
keeps its parent/thread linkage and stores CC recipients on the reply message, so the same fan-out and
CC visibility semantics apply.

**Consequences.** Threaded conversations can include observers without losing history. CC recipients of
replies remain visibility-only under ADR 0033.

## 0036 — Status hints at activity on another store

- **Date:** 2026-06-26
- **Status:** Accepted (`daemon-core` acceptance)

**Context.** With multiple configured backends, a user can run `status --address <addr>` against the
wrong selected backend and see an empty/unoccupied projection while the same daemon has live membership
for that address on another store. Dogfooding showed this is easy to miss when the default backend is
not the local SQLite store used by active stations.

**Decision.** When `status --address` has no live member for the selected store, inspect the daemon's
detailed member set for the same address on other store keys. Report `also_active_on` plus a
`backend_warning` instead of silently presenting the selected backend as the whole truth.

**Consequences.** Wrong-backend calls become self-diagnosing without changing command routing or
failing if no alternate activity exists. This is best-effort observability; it does not scan arbitrary
offline backends.

## 0037 — Wait out-dir has both flat and enveloped delivery artifacts

- **Date:** 2026-06-26
- **Status:** Accepted (`daemon-core` acceptance)

**Context.** `wait --out-dir/message.json` is a flat message object while `read --id` returns an
envelope (`message`, `dispositions`, optional thread context). Consumers that assumed one shape for the
other saw null fields. Changing `message.json` would break existing wait consumers.

**Decision.** Preserve flat `message.json` and add `delivery.json` on wait delivery. The new artifact is
an envelope `{ message, delivery, status }`, where `delivery` contains the role/context metadata and
`status` mirrors the wait status object. Non-delivery outcomes remove stale `delivery.json` alongside
`message.json`.

**Consequences.** Existing consumers keep working, and new consumers can use the more explicit envelope
shape without special-casing wait artifacts versus read responses.

## 0038 — Session-filtered station status projection

- **Date:** 2026-06-26
- **Status:** Accepted (`daemon-core` acceptance)

**Context.** A downstream Copilot plugin turn-end guard needs a cheap, stable JSON signal for the
current session: which addresses it attends, whether each has an armed waiter, and whether backlog is
pending. Parsing full daemon status or human text on every turn end is brittle and noisy.

**Decision.** Add `telex station status --session <id>` as a compact machine-readable projection over
the daemon's detailed status. It returns only the selected session's stations for the current store,
including address, health, waiter counts, pending unconsumed count, last waiter delivery metadata, and
live waiter details.

**Consequences.** The plugin can implement an agent-stop/re-arm guard without new daemon state or a
separate lifecycle protocol. The projection is read-only and requires the same same-user/admin-cap
access as detailed daemon status.

## 0039 — Push delivery via a generic on-deliver exec + Copilot session bridge

- **Date:** 2026-06-30
- **Status:** Accepted (`push-delivery` node / PR #55)

**Context.** Delivery to a Copilot CLI session depends on an **agent-armed** `wait` waiter that
must be re-armed at every turn boundary. A single missed re-arm makes the station **deaf** while
messages queue durably — the exact friction #53 targets. We want a message to arrive as an agent
**turn** without the agent owning a listener, while keeping two invariants sacred: the daemon /
Rust core stays **harness-agnostic** (no Copilot/SDK coupling — ADR-level boundary), and the
durable buffer + explicit-agent-`Ack` fence ([sec.11.3](daemon.md#113-server-side-delivery-fence-mr1--at-least-once-preserving))
never regress.

**Decision.** Add a **generic daemon on-deliver exec** primitive: `Register` gains an optional
`on_deliver: Vec<String>` **argv** the daemon runs when a message is durably committed for that
member — **after** the `deliveries` commit and `wait` notify, strictly **off the ack critical
path**, for the **primary** recipient only (never cc, per ADR 0032/0033). It is **liveness-only**:
a wake signal that **never** marks delivered or consumed, so the fence is unchanged and
**at-least-once with duplicates is the safe direction**. Concurrency and per-exec timeout are
capped; a per-heartbeat bounded sweep retries pushes whose target was briefly absent; repeat-fire
dedup is a lifecycle-scoped fast-path (reset on removal / re-register), never the authority. The
**Copilot** binding lives entirely **outside core**: `telex copilot push` reads the harness-neutral
message descriptor on stdin, derives the session's bridge endpoint (not trusted from the registry
path), and hands it to an **in-session extension bridge** that injects it as a turn via the CLI's
`session.send`. Attention maps two ways — `interrupt → immediate`, everything else → `enqueue`
(delivered after the current turn). The `telex wait` CLI is **retained** as the harness-agnostic
**pull fallback**; the Copilot skill defaults to push.

**Consequences.** Push-capable sessions lose the deaf-station failure mode — no agent re-arm is
required. The daemon gains exactly one opaque, harness-neutral hook (an argv + a descriptor on
stdin; **no** Copilot/SDK types in core), preserving the boundary. A slow/hung exec can never block
delivery, the fence, or another member. Deferred and documented in PR #55: refcount
multi-store + atomic (SF-2), descriptor size cap (SF-5), stale-exe guard (C-4), bridge protocol
negotiation/enforcement (C-5 — PR #55 ships only an informational `COPILOT_BRIDGE_PROTOCOL`
number in the `telex copilot skill` header, not wire negotiation), and a `telex copilot gc`
for orphaned endpoints. See
[copilot-bridge-push.md](copilot-bridge-push.md) and
[daemon.md sec.13.2](daemon.md#132-on-deliver-push-opt-in-harness-neutral).

## 0040 — Copilot skill is binary-owned; the plugin skill is a bootstrap

- **Date:** 2026-07-01
- **Status:** Accepted (`push-delivery` node / PR #55)

**Context.** The #53 skill rewrite risks baking a long, detailed copy of the Copilot
workflow into the static plugin skill (`skills/telex/SKILL.md`). Because #53 moves the
Copilot path from waiter/re-arm to bind → bridge → pushed turns → disposition, a static
skill is especially prone to drifting from the installed binary and misleading the agent —
the detailed command syntax and workflow should come from the exact `telex` binary
installed in that session.

**Decision.** Invert ownership. The plugin `skills/telex/SKILL.md` becomes a small, stable
**bootstrap**: it says what Telex is, tells the agent to load version-matched instructions
from the installed binary (`telex copilot skill` for the Copilot push path, `telex skill`
for the generic pull path), and names command `--help` as the syntax source of truth. The
installed binary owns the detail: `telex copilot skill` prints the version-matched Copilot
workflow from an embedded `COPILOT.md`, headed by `telex v..`, the Copilot **bridge
protocol** version, and the **minimum compatible plugin** version. It accepts
`--plugin-version` (or `TELEX_PLUGIN_VERSION`) and prints a clear compatibility **warning**
when the plugin is older than the binary supports. The detailed Copilot section in the root
`SKILL.md` is likewise reduced to a pointer at `telex copilot skill`, so the Copilot flow
has a single source of truth.

**Consequences.** Future protocol/bridge changes update the binary (and its embedded
`COPILOT.md`) without a coordinated edit to a static plugin file, and a stale plugin is
flagged rather than silently trusted. The former byte-identical plugin↔root mirror
invariant (and its test) is replaced by a bootstrap invariant: the plugin skill stays
small, defers to the binary, and embeds no detailed recipes. `telex copilot skill` no
longer dumps the whole generic skill; agents wanting the generic/pull reference still run
`telex skill`. The plugin's own version is the one version fact the bootstrap legitimately
carries (it matches `plugin.json`), used only to drive the compatibility check.

## 0041 — On-deliver re-delivery is re-provision-triggered, not timer-until-ack

- **Date:** 2026-07-01
- **Status:** Accepted (`push-delivery` node / PR #55)

**Context.** ADR 0039's on-deliver push kept re-pushing a still-unacked message on a fixed
per-message backoff (base 15s, doubling) until the agent acked, to guarantee no loss between
"harness accepted the turn" and "agent durably consumed it." A live two-terminal dogfood test
showed this over-fires: a **successful** `session.send` only means the turn was *queued*, so
against a busy or slow recipient the daemon re-pushes the same message every ~15s, enqueuing
duplicate turns the agent must dedupe — the agent can fall behind acking while it burns turns on
dupes (correct via id-dedupe, but wasteful and confusing). The re-push conflated two situations it
should treat differently: (a) the queue is intact and the message is simply pending/seen —
re-pushing is pure duplication; (b) the queue was lost (crash / reattach / reload) or the push
never landed — re-delivery is genuinely needed.

**Decision.** Split the re-push cadence by the last push's **outcome**, and make re-delivery
**re-provision-triggered** rather than timer-driven:
- A **failed** push (bridge unreachable / target absent) keeps the fast `on_deliver_backoff`
  (15s doubling to a cap) so a transiently dead bridge recovers quickly.
- An **accepted** push is already queued in the live session, so it is **not** re-pushed on the
  fast cadence. Re-delivery of un-acked messages happens on **attachment change** — a reattach, a
  new session taking the address, or a `/clear` bridge-reload re-provision — which already calls
  `on_deliver_forget_member` + rescans `fetch_undelivered`. A long `ON_DELIVER_ACCEPTED_BACKSTOP`
  (5 min) is retained only as a backstop against a silent in-session drop of a queued turn.
- A **seen-but-unacked** message is the agent-stop **turn guard's** job (it already lists unacked
  deliveries and nudges the agent to ack), not the daemon's to re-push.
The COPILOT.md workflow now tells the agent to **re-provision after `/clear`** (re-run
`copilot attach --copilot-bridge` before `extensions_reload`) so a reload re-delivers the backlog.

**Consequences.** The redelivery amplification is gone: a continuously-held session gets each
message once (plus rare backstop re-checks), not every 15s. The **sacred durable+ack invariant is
preserved** — the durable store still never loses a message, and every queue-loss path (crash,
reattach, reload, new holder) re-delivers un-acked messages at-least-once (a duplicate to a fresh
attachment is the safe direction). The daemon stays **harness-neutral**: "attachment generation"
is realized as re-provision events (lease-epoch bump + re-`Register`), not a bridge-specific token
plumbed through the core. Residual risk — an accepted turn silently dropped while the same session
stays attached without a reload — is covered by the 5-min backstop and the existing degraded status.
