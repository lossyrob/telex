# Local presence/transport daemon (eliminate the per-session holder)

## Purpose

Most of telex's recurring staleness (orphaned holders, zombie `occupied` leases,
holder/waiter startup races, dismiss leaving a holder attached, a forever-listener
starving an orchestrator's turn loop) traces to one structural choice: a
**per-session resident holder** whose lifetime must track a fuzzy agent session.
This workstream eliminates that holder by introducing an **auto-spawned per-user
local daemon** that owns presence and delivery for all locally-attended addresses,
and ships the surrounding pieces (Copilot plugin, seamless upgrade) needed for a
real, end-to-end unblock so idle long-lived sessions stay wakeable and stations
stop going stale. It resolves [issue #32](https://github.com/lossyrob/telex/issues/32).

## Approach

The work is a single complete deliverable across **both SQLite and Postgres** (the
operator runs both and has stations idle waiting on this), not a thin V1 slice. A
SQLite-only spike is an internal step inside the daemon-core node, not the shippable
boundary.

Formation orders the work as confidence transitions expressed through the node DAG.
**design-foundation** (a research node, written and spar-pressure-tested) locked the
hard contracts up front - the daemon-scoped capability/version-handshake IPC, the
**server-side lease-epoch fence**, the **seen-dedup redesign**, explicit membership
+ non-destructive liveness, negative-only watched-process evidence, the daemon
**singleton identity** + **lifecycle contract**, and daemon-native session RPCs -
behind a builder **design-gate**. Then the **daemon core**
(the centerpiece: daemon process, durable buffer, one-shot verbs, server-side
epoch-fenced delivery, the lifecycle contract, and a minimal upgrade floor) on
SQLite, which with the **Copilot plugin** is the first slice that can unblock the
operator (reached when the plugin lands on SQLite). A distinct **fencing-proof** gate
(epoch-guarded emission + ordered handoff, proven on SQLite) then blocks downstream
work. **Postgres parity** extends the core under that proof and adds the cross-machine
reclaim (competing daemons); **seamless upgrade** (#6) lands
**last**, after Postgres and the plugin, so the full upgrade platform never blocks
the unblock. The original large validation-harness and AKS-scale shape was later
replaced by a practical **release-confidence-validation** node, which is complete.
Issue #106 / PR #138 is the active hardening repair discovered after that
validation. The operator accepted persistent OS-lock containment and a truthful
degraded-enumeration contract for that PR. The mandatory downstream
**station-intent-transactional-authority** node
([#153](https://github.com/lossyrob/telex/issues/153)) closes the accepted gap before
the final **closure gate**, without blocking PR #138 or the builder
**hardening gate**. Nodes are coarse and PAW-sized; the completeness split is
justified by a transactional migration boundary and an independently useful,
safe PR #138 outcome.

The schema-3 recovery release is a separate, bounded chain. Issue #154 covers
the two unsafe Windows `TOKEN_USER` reads on exact main. Issue #155 adopts draft
PR #156 for PostgreSQL query and `LISTEN` reset recovery. After both repairs merge
and the campaign verifies dependency closure, one isolated worker prepares an
immutable v0.2.0 candidate for the campaign/operator-owned
`schema3-release-gate`. This chain does not reopen the completed release nodes and
does not accept hardening or closure.

The richer design rationale and the full decision ledger that led here live in
[`docs/initial-shaping.md`](docs/initial-shaping.md). The brief stays current and
distilled.

## Design References

The authoritative design layer (merged from `design-foundation`) lives under
`docs/design/`:

- `telex:docs/design/daemon.md` - the **normative daemon contract** the implementation
  nodes build against (17 sections + the sec.17 gating tests).
- [`design/current-design.md`](design/current-design.md) - the canonical integrated
  workstream design. It summarizes merged authority and keeps accepted issue #106 /
  PR #138 direction explicitly behind a product-promotion boundary.
- `telex:docs/design/DESIGN.md` - the local-exchange architecture.
- `telex:docs/design/DECISIONS.md` - the ADR log; **0014-0024** are this workstream's
  decisions (0023 = the minimal session/presence/delivery model; 0021 = the
  `docs/design/` relocation).
- `telex:docs/design/index.md` / `docs/design/ARCHITECTURE.md` - the entry point and the
  5-diagram visual on-ramp.
- `telex:PRODUCT-THESIS.md` (root) - the "no server" -> "auto-spawned local exchange"
  framing.

## Boundaries

- **In scope:** the per-user daemon (presence + transport) for SQLite **and**
  Postgres; one-shot `attach`/`detach`/`wait` against the daemon; durable buffer
  (reuse 0011/0013) with the **seen-dedup redesign** for a long-lived daemon; the
  **lease-epoch fencing token** with a **server-side fence on delivery emission +
  ordered handoff** (`mark_delivered_if_current_owner`) proven by a distinct
  **fencing-proof** gate; the daemon **singleton identity** (user SID + config root +
  protocol-major) and **lifecycle contract** (spawn-lock, connect-or-spawn, readiness
  ACK, `wait` reconnect-on-EOF grace, exit codes, Status surface); the **daemon-scoped
  capability + version-handshake IPC**; the ADR 0023 liveness model
  (authoritative, non-destructive `sessionEnd`, negative-only watched-process
  evidence with start-time, and a non-destructive idle backstop); explicit-only
  membership removed only by explicit **`Detach`**; the Copilot CLI plugin as the
  harness adapter and one shared source for `telex skill`; the **minimal upgrade
  floor** (versioned shim + `daemon stop
  --drain` + next-call respawn + legacy/non-epoch cutover rule) in `daemon-core` with
  full seamless upgrade (#6) last; retiring superseded mechanisms (#3 relay, pid-watch
  as a per-session holder, the re-arm dance) and updating the docs **with**
  `daemon-core`, not at closure; desired station-intent recovery with bounded
  OS-lock safety and a degraded partial-scan contract; and the downstream
  transactional-authority closure. The bounded recovery-release addition covers
  issue #154 Windows token-buffer alignment, issue #155 PostgreSQL wait-reset
  recovery, and isolated v0.2.0 schema-3 candidate preparation and publication
  gating after both repairs merge.
- **Out of scope:** the embeddable SDK client (#12) - it shares the
  collapse-into-one-process theme and should reuse the stabilized Layer-1 IPC, but
  is a separate solve; response windows / TTL deadlines (#2); the `store_key` helper
  (#25). The recovery-release packet also excludes PR #138, issues #152/#153,
  Watcher and Operator Station runtimes, campaign closure, schema-policy redesign,
  shared or production database mutation, and operator-installed daemon operations.
- **Deferred:** a richer non-binary occupant status policy beyond the accepted
  non-destructive liveness states; the pid-reuse-immune fd-over-IPC backstop
  (#28-flavored), awkward with a singleton daemon (the accepted process evidence
  uses PID + start-time); and the daemon subsuming directory/occupancy reads
  (`address list`).

## Current State

The design foundation, daemon core, fencing proof, Postgres parity, Copilot plugin and
push bridge, lifecycle hardening, versioned/release upgrade paths, public release, and
release-confidence validation are merged and recorded complete. The normative design
remains `docs/design/daemon.md`, with ADR 0023 governing explicit-only membership and
non-destructive liveness; merged PR #139 additionally defines the Copilot App
turn-idle and bridge-host lifecycle behavior.

Dogfooding then exposed issue #106: daemon replacement can preserve durable messages
while losing a still-live bridge's desired push registration. Existing PR #138 is the
adopted `station-intent-reconciliation` repair. The operator selected persistent
owner-private OS advisory locking to prevent stale pathname mutation and accepted a
degraded contract for bounded partial directory scans. PR #138 remains open,
merge-unapproved, and excluded from this recovery packet; this authority does not
revive its worker. The **hardening gate is not ready** until that narrowed repair is
merged and presented with isolated restart/drain/upgrade and push-recovery evidence.

Unconditional transactional generation authority, seekable fair discovery and
garbage collection, exact counts, and exact over-cap recovery belong to the planned
XL `station-intent-transactional-authority` node
([#153](https://github.com/lossyrob/telex/issues/153)). That node follows PR #138 and
blocks the final **closure gate**, not PR #138 or the hardening gate.

Exact main `ed417c6b938f92fe3bfb84f3b0cc0bea719fbbd0` is schema 3 and protocol
1.5, while released v0.1.2 supports schema 2 and protocol 1.4. Exact main also
contains unsafe `Vec<u8>`-backed `TOKEN_USER` reads in both
`src/backend/sqlite.rs` and `src/daemon.rs`; the daemon alignment repair exists
only on unmerged PR #138. Draft PR #156 is published at
`5c302dacb1c3e659e7f89c3c9670e3cf5cbc5105` for issue #155 and must be adopted
without replacing its branch or history. The recovery release prepares v0.2.0 only
after issues #154 and #155 merge, using disposable isolated roots and databases.
Tagging and publication remain a separate explicit operator decision.

Both recovery repairs launched against main
`7ed886b07620e8ab8adba68249ab84f96ea26013` and are in progress. Issue #154 has one
worker on `feature/windows-token-buffer-alignment`, with no PR yet. Issue #155 has
one worker completing adopted draft PR #156 in place on
`copilot/fix-postgres-connection-reset`; its local starting head `3423cd6` merges
main into published head `5c302dac` without rewriting history. At launch, neither
repair had completed review, CI, or merge. Issue #157 is not launched, and the
release gate, hardening gate, and closure gate remain planned.

As of 2026-09-25T00:09Z, PR #156 was no longer draft, and its complete candidate
`25aea118` was under initial full PAW review (`8cda7626`). Exact-head CI run
36071395738 succeeded on that head. The #154 candidate `f8363be` is pushed and
clean but has no PR; publication is held on an operator-owned App link. Both nodes
remain in progress.

The full independent PAW review of `25aea118` is complete and was posted as
GitHub COMMENT review 5311857915. Its verdict is changes requested: two P2
blockers, no warnings, and two optional low-priority observations. The blockers
are stop-outcome precedence during drain and cancellation of the credential
helper; both are absorbed into PR #156. The same #155 worker is repairing them.
There is no new head yet, and design inspection and merge remain held. The
earlier CI success covers only `25aea118`.

The operator selected the one-shot `--password-command` lifecycle contract for
M2. Normal, error, and cancellation completion must clean up Telex-owned helpers
that remain within the supported invocation scope. Intentionally persistent,
escaped, or broker-launched background work is outside that command lifecycle.
Querying an already-running credential agent does not make Telex its owner or
authorize Telex to terminate it. This policy does not promise universal
all-descendant containment or fail-closed detection of every Unix escape.

The operator also selected `Approve bounded orderly-exit drain and explicit
failure hold (Recommended)`. Logical command and Wait result deadlines remain
unchanged. Normal exit of the host that owns credential work may take up to
three additional seconds to drain and join owned work concurrently against one
absolute current cleanup budget. The budget is not renewed or multiplied by
capacity. Waiting three seconds is not proof of cleanup. If receipt remains
missing, Telex must report failure and retain named ownership and escalation,
potentially beyond three seconds, rather than report a clean exit or promise an
unconditional three-second OS exit bound.

Campaign accepted the exact reviewed intended M2 technical design on
2026-09-25T13:41:31-04:00. The accepted engineering limits include the
credential-specific process-local native owner and registry, finite admission,
aggregate capacity `C=2`, same-source barrier, owned nonblocking I/O, atomic
cancellation and publication, platform receipt conditions, join and shutdown
ownership, and internal cleanup-observation target `B=3s`. These limits are not
measured performance or universal finite OS cleanup guarantees.

The accepted design uses the steward's technically feasible Unix receipt
sequence with supported-API conditions, superseding the earlier blanket
feasibility blocker.
Telex would retain the unreaped group leader through every mutating group signal,
send the final termination signal while that identity anchor is valid, enter an
irreversible no-more-group-signals state, reap the exact leader, and then use the
former group ID only for read-only absence observation. Linux would use
`getpriority(PRIO_PGRP, P)` with exact `errno` handling; macOS would use the
supported libc POSIX/UNIX03 `kill(-P, 0)` binding. Receipt would require
conclusive group absence, exact leader reap, closed or completed owned I/O, and
native owner completion. Presence, denial, ambiguity, or possible reuse would
retain the cleanup obligation without restoring signal authority.

The accepted intended design landed on main at `a6eabba`. Campaign then granted
the same #155 worker M2 product-write authority at 2026-09-25T14:27:39-04:00.
After a context refresh that preserved the three unpublished M1, C1, and C2
commits, the worker acknowledged the grant before editing at local head
`2e2f3b99`, which merges `5d956618` and `a6eabba`, and began the accepted plan
phases P1-P7. The intentionally red credential regression may now become an
actual regression test; its original negative evidence is retained.

Two platform receipt axes were held for adjudication. On Windows,
worker-supplied runtime evidence shows the current job accounting can report
zero active processes while a retained helper process
handle is still unsignaled. The steward's consolidated review of the frozen
packet found no code error that explains this away and kept the verdict
design-feasibility-blocked: job accounting reaching zero is not a documented
completion fence for every member, and no supported replacement is justified yet.
A final documentation-only `DEBUG_PROCESS` assessment found a documented
per-process exit fence, but a child can start a new debugging chain inside the
job, so debug inventory does not cover the complete scope; debugger-visible
behavior would also be new semantics. Windows receipt work remains held, and
further replacement research stops pending campaign disposition.

On macOS, the frozen Unix code confirms a
conditional source defect: a final group `SIGKILL` that fails with `EPERM` on a
zombie-only group is latched as failure before the exact reap and independent
absence check. Campaign then authorized the steward's narrow correction as an
ordinary implementation fix: record the attempt, seal signals, reap the exact
leader, and require independent absence. `EPERM` is never absence. The worker
acknowledged at 2026-09-25T15:20:59-04:00 before editing. No macOS program has
run, and all Linux and macOS runtime proof remains unrun. Any material change to
intended authority requires reviewed Tier B reconciliation.

M2 is not implemented, measured, runtime-proven, reviewed, or merged. PR #156
remains published at `25aea118`, and all new M2 work is uncommitted and
unpushed. The #154 candidate `f8363be` is complete, but publication remains
blocked by the actual App EMU 403 and the latest same-worker quota result. Any
retry requires restored credit, deduplication, and explicit authority. Issue #157
is not launched.

Workstream and design-steward branches are proposal/integration workspaces, not
silent authority. Streamliner artifact changes become durable only through the
campaign's sole artifact reconciler applying the reviewed, operator-authorized
packet directly to `main`.

## Decisions

- **One complete deliverable, both backends:** SQLite and Postgres ship together;
  the SQLite spike is an internal step, not the boundary. Rationale: the operator
  runs both with stations idle waiting; a partial cutover does not unblock them.
- **Coarse, PAW-sized nodes (~one per confidence transition):** bias to fewer,
  heavier nodes; the three completeness tracks are the one deliberate split for
  parallelism + distinct expertise.
- **Local-spec-first tracking:** node specs live under `tasks/`; promote to GitHub
  issues at wave promotion. The umbrella issue #32 is the workstream's parent
  tracker.
- **Project design authority lives under `docs/design/`:** `daemon.md` is normative,
  `DESIGN.md` supplies architecture, and `DECISIONS.md` is the append-only ADR log.
  The workstream's canonical integrated summary lives at
  `design/current-design.md`; pending proposals are not project authority.
- **Spar at arm's length:** critique informs the design but pivots are surfaced for
  builder confirmation, not auto-applied.
- **Lease-epoch fencing is the spine (from spar):** daemon-down recovery, upgrade
  handoff, and Postgres reclaim are all made safe by one monotonic
  `lease_epoch`/`owner_instance_id` rather than by timing. `design-foundation` owns
  the epoch lifecycle.
- **Fencing-first sequencing (from spar):** lock the hard contracts (fencing,
  explicit membership, non-destructive liveness, watched-process identity) in
  `design-foundation`; gate Postgres on fencing proven under competing daemons;
  land seamless-upgrade last. Keeps both backends + #6 in the deliverable while
  limiting blast radius.
- **Server-side epoch fence + a distinct `fencing-proof` gate (council):** lease-row
  fencing alone is insufficient - delivery emission is fenced server-side
  (`mark_delivered_if_current_owner`; no frame unless the daemon owns the epoch) and
  handoff is ordered; a distinct executable `fencing-proof` gate blocks
  Postgres/plugin/upgrade until proven. Verified: the holder emits the frame *before*
  `mark_delivered` commits, and per-process `seen` resets across a handoff.
- **Minimal upgrade floor early (council):** a versioned shim + `daemon stop --drain`
  + next-call respawn + a legacy-holder/non-epoch-lease cutover rule land in
  `daemon-core` (the first daemon-aware install hits the Windows binary-lock); full
  rollback/gc/UX stays last.
- **Daemon-native session ownership (revised by ADR 0023):** the daemon's
  in-memory `session->addresses` map is the authority. Reuse the hook plumbing as
  a non-destructive liveness input; explicit detach, not sessionEnd, owns
  membership removal.
- **Station-intent safety now, transactional convergence downstream:** PR #138
  replaces age-stealable intent locks with persistent owner-private OS advisory
  locks and exposes partial-scan degradation under the four-second response
  contract. A connected XL node restores unconditional transactional generation
  authority and fair maintenance before workstream closure. This preserves one
  complete, useful PR #138 outcome while keeping the accepted gap durable.
- **Recovery release is an isolated confidence chain:** Repair #154 and adopted
  PR #156/#155 first. After both merge, one isolated worker prepares one immutable
  v0.2.0 candidate and evidence packet. The campaign/operator publication gate is
  distinct from hardening and closure; green evidence never implies publication
  approval.
- **Docs/SKILL cutover with `daemon-core` (council):** keep the verb names; update
  `SKILL.md` + plugin docs when behavior changes, not at closure, so instructions
  never describe a dead holder/waiter model mid-workstream.

## Resolved questions and superseded validation shape

All eight design-foundation questions are **resolved** as ADRs 0014-0024 (see
`docs/design/DECISIONS.md` and `daemon.md`): epoch lifecycle (0015), session
presence/reaping + crash durability (0017/0023), watched-process evidence (0017),
legacy cutover (0020/0024), explicit membership and agent acknowledgement
(0019/0023), and the Status freeze line (0018).

The earlier `validation-harness`, Entra multi-host campaign, and AKS scale-rig
concepts were superseded by the completed `release-confidence-validation` node
(issue #78). The operator resolved the PR #138 M3/M5 fork on 2026-09-02:
PR #138 owns safe desired push restoration with persistent OS locking and an
explicit degraded-enumeration contract; the downstream transactional node owns
unconditional generation, discovery, GC, and over-cap authority.

## Imports and Exports

### Imports

- **PR #31 / issue #23 (sessionEnd hook plumbing):** the plugin reuses the hook
  wiring, but not its filesystem `session_registry` as attendance authority. The
  daemon owns `session_id->addresses` in memory; under ADR 0023 the hook supplies
  non-destructive liveness input while explicit detach owns membership removal.
- **Decisions 0011/0013 durable delivery (`deliveries` table, `fetch_undelivered`):**
  reused as the daemon's durable buffer. Available in `main`.
- **Harness env contract (consumed only by the plugin layer):**
  `COPILOT_AGENT_SESSION_ID` and `COPILOT_LOADER_PID`, verified present and reliable
  (explicit env vars, not ppid-walk). telex core stays harness-agnostic - it takes an
  opaque `$TELEX_SESSION_ID` and one or more generic `--watch-pid`s; the Copilot
  plugin maps these env vars onto them.

### Exports

- **Stabilized Layer-1 IPC/attendance protocol:** the daemon's documented control
  protocol, intended for reuse by the embeddable SDK client (#12).
- **Seamless-upgrade install layout + launcher shim:** the versioned-install
  mechanism (#6), reusable for any future telex distribution.

### External Dependencies

- None outside telex itself. Building/installing from source on Windows is locked by
  running `telex` processes during the binary swap - the very pain #6 fixes - so
  validating `seamless-upgrade` requires care during dogfooding.

## Closeout Observations

(parking lot - populated during execution)
