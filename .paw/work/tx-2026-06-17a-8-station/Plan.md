# Plan — Station Terminology (issue #8)

## Outcome anchor

Adopt **"station"** as the user/agent-facing noun for the running presence that
serves a telex address (holder + waiter, together). Thread the term consistently
through docs, the embedded skill, CLI help, and (light touch) operational output.
**No behavior change, no CLI verb renames.** Satisfies issue #8 acceptance criteria;
PR will `Closes #8`.

## Key design decision (preference fork)

The issue leaves "how far internal renames go" to implementer discretion and asks for
a **light touch on internals** to keep the diff reviewable. Decision:

- **Add** "station" as the umbrella user/agent-facing noun everywhere the running
  presence is described in prose/help.
- **Keep** the precise sub-roles **holder** and **waiter** where the two-process
  mechanics need them: SKILL.md re-arm pattern + exit-code table, DESIGN.md "Waiter
  loop behavior" mechanics, and all `[holder]` operational log prefixes / internal
  symbols. No rename of structs, log prefixes, or the `attach`/`detach`/`wait` verbs.
- Canonical definition + vocabulary table live in **DESIGN.md** (working design, home
  of the address/lease model); SKILL.md gets a short definition and uses the term.

This is the low-spread, issue-endorsed default. Recorded as preference debt only in
that a builder could later choose deeper internal renames.

## Work items

1. **DESIGN.md** (`work:design-canonical`) — Add the canonical "station" definition +
   metaphor vocabulary table (station / address / lease / holder / waiter). Introduce
   "station" in the historical-telex table row and the Waiter-loop section intro;
   frame `attach`/`detach` prose as starting/stopping a station. Retain holder/waiter
   for the two-process mechanics.
2. **SKILL.md** (`work:skill-md`) — Introduce "a **station** keeps the address live
   (its holder owns the lease; its waiter answers)". Update the core-loop intro, the
   detach box, command-reference rows for `attach`/`detach`/`status`/`wait`, and the
   worked example to use station as the umbrella term. Keep holder/waiter in the
   re-arm pattern + exit-code table where precision matters.
3. **src/commands/skill.rs** (`work:skill-rs`) — The `--address` assignment text should
   describe starting a **station** on the address (still via background `telex attach`).
4. **README.md** (`work:readme`) — "How it works" prose: a session attaches to start a
   **station** that holds the lease and answers liveness.
5. **src/cli.rs** (`work:cli-help`) — Help/doc comments for `attach` ("start a station
   on the address"), `detach` ("stop the station, release the lease"), `status`
   ("station/occupancy"). No flag/verb renames.
6. **Supporting docs** (`work:supporting-docs`) — DECISIONS.md (record the station
   terminology decision), DISPATCH.md, TELEX.md, PRODUCT-THESIS.md: weave "station"
   into the running-presence / answerback narrative where natural.
7. **Verify** (`work:verify`) — `cargo build`, `cargo test`, run `telex skill` and
   `telex skill --address <addr>` from the built binary; confirm new term appears and
   skill.rs unit tests still pass.

## Out of scope / non-goals

- No rename of `attach`/`detach`/`wait` or any CLI subcommand.
- No lease/liveness/messaging behavior change.
- `EXTENSIONS.md` is not present in `origin/main` (it is uncommitted WIP elsewhere);
  not editable here — skip.
- `docs/notes/initial-research/*` and `spike/*` are historical/throwaway — leave as-is.
- `[holder]` operational log prefixes kept as-is (internal, light touch).

## Acceptance check (maps to issue)

- [ ] "Station" defined once clearly in a canonical place (DESIGN.md) + vocab table.
- [ ] User/agent-facing docs + CLI help use "station", retaining holder/waiter for
      mechanics.
- [ ] `telex skill` / `telex skill --address` output reflects the term.
- [ ] No behavior change, no verb renames.
- [ ] Build + tests green.
