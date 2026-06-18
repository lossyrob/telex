# WorkflowContext

Durable configuration for this PAW Lite work item. (No runtime/bookkeeping state here.)

| Field | Value |
|-------|-------|
| Work ID | `tx-2026-06-17a-18` |
| Workstream | `tx-2026-06-17a-18` |
| Repo | `lossyrob/telex` |
| Issue | #18 |
| Base branch | `main` |
| Branch | `paw/tx-2026-06-17a-18-pg-commit-order` |
| Workflow Identity | paw-lite |
| Workflow Mode | custom |
| Plan Generation | single-model |
| Planning Docs Review | enabled |
| Planning Review Mode | multi-model |
| Planning Review Models | gpt-5.5, claude-opus-4.8 |
| Final Agent Review | enabled |
| Final Review Mode | multi-model |
| Final Review Models | gpt-5.5, claude-opus-4.8 |
| Implementation Model | claude-opus-4.8 |
| Review Strategy | local |
| Review Policy | final-pr-only |
| Artifact Lifecycle | commit-and-clean |

## Outcome anchor

The **live holder** delivers a concurrently-committed lower `id` (a Postgres message whose
id was allocated before, but committed after, a higher id that the holder already drained)
**without requiring a holder restart**. Planning/review may add prerequisites but must not
replace this anchor.
