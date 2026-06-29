# Plan: CC Stream Wake

## Approach Summary

Node outcome anchor: implement an explicit opt-in daemon delivery-semantics change so a seat can be woken by CC/observer traffic for the conversation/table it belongs to, while default CC remains visibility-only/no-wake, primary `--to` remains ack-required, and the branch ends with live proof of the opted-in wake behavior.

Use the smallest backend-facing primitive that preserves the outcome: add an explicit per-wait CC wake opt-in rather than making all CC wake by default or building a full stream subscription subsystem in this node. The implementation will keep default CC delivery rows auto-seen, add a live-windowed CC notification candidate path for opted-in waits, document the resulting ack/visibility contract, and add tests that prove both the default and opted-in paths.

The load-bearing safety mechanism is a per-wait CC lower bound captured when the waiter is registered. `--wake-on-cc` may surface only CC candidates whose delivery timestamp is strictly after that lower bound; it must never replay the already-seen CC backlog or make CC rows pending/ack-required.

## Work Items

- [x] **Design contract** - Update `docs/design/daemon.md` and `docs/design/DECISIONS.md` with the explicit per-wait CC wake opt-in semantics. Done when ADR 0033 is superseded or amended with: default CC remains visibility-only/no-wake, `--wake-on-cc` wakes only for live CC traffic after the per-wait lower bound, `--min-attention` composes with but does not imply CC wake, CC wake responses carry `delivery_role: "cc"` and `requires_disposition_for_current_recipient: false`, and primary `--to` ack-required semantics remain unchanged. Cross-reference ADRs 0032, 0034, and 0035 where needed so the accepted CC baseline is not contradictory.
- [x] **CLI and IPC opt-in** - Add `telex wait --wake-on-cc` and IPC `Request::Wait { wake_on_cc }` with serde-default/backward-compatible false, protocol minor/capability metadata, and live-waiter diagnostics. Done when the capability follows the existing naming convention (expected shape: `wait_wake_on_cc_p10`), is advertised in the handshake/status metadata, old request shapes deserialize as `wake_on_cc=false`, and default wait behavior is unchanged.
- [x] **Delivery semantics** - Add a separate wait-candidate path that preserves `fetch_undelivered` for ack-required primary backlog while adding live-windowed, non-ack-required CC candidates only when `wake_on_cc=true`. Done when the daemon captures the CC lower-bound timestamp at wait registration, SQLite CC candidates require `delivered_at_ms > cc_after_ms` (or equivalent) and post-filter through existing CC parsing/delivery-role logic, consumed CC notifications do not trigger the primary unacked re-arm guard, and unacked primary `--to` messages are still rejected on re-arm until acked.
- [x] **Postgres parity or tested unsupported gate** - Decide and implement the non-SQLite axis before claiming issue closure. Done when either Postgres has equivalent observable `wake_on_cc` live-window behavior with a matching test/proof, or `wake_on_cc=true` against unsupported daemon-backend combinations returns a typed, tested unsupported response and the PR/field report explicitly uses `Refs #40` instead of `Closes #40`.
- [x] **Daemon-core tests** - Add targeted daemon/core tests proving default CC no-wake, opted-in CC wake with `delivery_role="cc"` and no disposition requirement, no duplicate/re-arm wedge after a CC wake, primary ack regression safety including a CC-wake-then-primary sequence, and `--min-attention` composition.
- [x] **Process-level live proof** - Extend `tests/daemon_process_sqlite.rs` to prove an observer wait with `--wake-on-cc` wakes for a CC send while a default observer wait remains timeout/visibility-only. Done when the proof exercises the real CLI/daemon path and includes the Postgres-equivalent proof or unsupported gate evidence from the Postgres work item.
- [x] **Docs and PR evidence** - Prepare the PR Docs.md content and concise evidence for issue #40; keep raw field notes outside committed artifacts. Done when the PR title follows `[local-daemon] ... (#40)`, the PR body starts with the required collapsible `Docs.md`, closure wording uses `Closes #40` only if all node outcome checks pass (live SQLite proof, Postgres parity or accepted tested gate, docs/ADR updated, no reopen condition triggered) otherwise `Refs #40`, Review Response mode is started immediately after PR creation, and the post-merge field report records outcome plus any per-wait friction that may justify a future standing opt-in node.

## Key Decisions

- Preserve the node outcome: prerequisite schema or documentation work is not sufficient unless the plan still lands live opted-in CC wake proof.
- Treat a full durable stream/table subscription model and persistent station-level setting as adjacent scope unless implementation proves per-wait live CC wake cannot satisfy the node outcome.
- Preserve CC no-wake as the default to avoid reintroducing notification churn.
- Keep primary `--to` ack-required semantics unchanged.
- Do not make CC delivery rows pending for `--wake-on-cc`; consumed CC rows are notification candidates only inside a live wait boundary, so old CC backlog is not replayed and CC remains safe to ack as already consumed/no-op.
- SQLite backend assumptions to preserve: primary fanout rows use `consumed_at_ms = NULL`, CC fanout rows use `consumed_at_ms = Some(now)`, and any CC matching must validate through the existing parsed CC/delivery-role logic rather than trusting raw comma-string `LIKE` matches alone.
- Postgres parity is a closure gate, not a footnote. If current `main` lacks enough daemon-core delivery-fence surface for runtime parity, the implementation must add a typed unsupported gate and the PR must be partial (`Refs #40`) unless the issue is amended.

## Open Questions

None. The Postgres axis is no longer an open question in the plan: implementation must either deliver parity or ship a tested unsupported gate and mark the node partial.
