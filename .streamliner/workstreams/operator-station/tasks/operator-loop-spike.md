# Vertical spike: mediated human-attention loop

- **Workstream:** operator-station
- **Type:** research (product/technical spike)
- **Status:** ready
- **Depends on:** none
- **Blocks:** viability-gate
- **Tracker:** [lossyrob/telex#93](https://github.com/lossyrob/telex/issues/93)
- **Parent workstream:** [lossyrob/telex#92](https://github.com/lossyrob/telex/issues/92)

## Outcome

A developer can exercise one complete experimental loop on Windows in which a
worker sends an operational message to a stable attention address, an operator
agent filters and escalates it to a desktop-attended operator address, the
desktop surfaces the escalation, and a human reply reaches the operator agent
and can be routed back to the worker. The loop preserves enough provenance to
inspect the raw source and is usable for real multi-session dogfooding at the
viability gate.

## Design References

- [`../docs/initial-shaping.md`](../docs/initial-shaping.md) - operating loop,
  alternatives, notification posture, and spike demonstration.
- `telex:PRODUCT-THESIS.md` - durable responsibilities, store-and-forward,
  auditability, and non-chat boundary.
- `telex:docs/design/daemon.md` - current local-exchange contract and supported
  lifecycle expectations.
- `telex:docs/design/proposals/EXTENSIONS.md` - convention space for message
  kinds, opaque metadata, and source references.
- `telex:telex-console/README.md` - feed, address, thread, delivery, and
  disposition presentation precedent.
- `streamliner-pr-122:desktop/` - Tauri tray/feed/notification reference
  implementation available in the builder's local Streamliner worktree.

## Exports

- A runnable experimental Station slice suitable for the builder's viability
  dogfood.
- A reusable experimental operator-agent assignment/prompt that attends the
  ingress address and mediates between workers and the human Station.
- `docs/operator-loop-spike-report.md`, recording the demonstrated flow,
  integration shortcuts, product observations, failures, and concrete
  requirements for the post-gate production contract.
- Source-reference conventions used by the spike, clearly labeled experimental
  so downstream work can accept, revise, or reject them.

## Boundaries

### In scope

- One Windows-first experimental desktop path with feed/backfill, notification,
  thread reading, reply, and the minimum disposition behavior needed to use the
  loop.
- One stable worker-facing attention address and one desktop-attended operator
  address.
- One operator-agent role that can read raw worker messages, ask follow-up
  questions, escalate a distilled request, receive a human reply, and route a
  response back.
- Visible source provenance connecting a human-facing escalation to the raw
  worker message or messages.
- Restart/backfill behavior sufficient for the dogfood session.
- A real end-to-end demonstration using current Telex behavior.

### Out of scope

- Production packaging, upgrade, auto-launch, or multi-platform support; owned by
  `station-app` and `operational-hardening`.
- A final daemon/client/SDK architecture; owned by `station-contract`.
- Direct, assisted, and quiet mode configuration beyond what is needed for the
  single mediated spike; owned by `station-contract` and `station-app`.
- A generalized routing engine, aliases, multi-device occupancy, structured
  decision widgets, or arbitrary action execution; deferred by the workstream.
- Production-grade Postgres, identity, security, recovery, and noisy-load
  guarantees; owned by `operational-hardening`.
- Streamliner workflow-specific behavior or changes to Streamliner Desktop.

## Inherited decisions

- **The spike may use temporary integration seams.** CLI subprocesses, direct
  library reuse, or another bounded experimental approach are acceptable because
  this node earns viability confidence rather than exporting a supported client
  contract. The spike report must identify every shortcut.
- **The filtering agent is application logic.** Telex core must not interpret,
  summarize, prioritize, or semantically route the worker messages.
- **The Station is a separate optional application.** Do not add desktop
  dependencies or human UI behavior to the core `telex` binary.
- **Raw provenance is mandatory even in the spike.** The operator agent sends
  from its own address and links to source message IDs/addresses rather than
  impersonating a worker.
- **Windows is the only required platform for this node.** Cross-platform design
  must not enlarge the first confidence transition.
- **The spike optimizes for a real loop, not UI completeness.** A plain but usable
  surface is preferable to polished cards that do not prove reply routing,
  mediation, and durable context.

## Design-impact expectation

No project design update is required merely to land the experiment. Record
potential Telex design changes as findings in
`docs/operator-loop-spike-report.md`; the post-gate `station-contract` node owns
promoting accepted changes into the design layer and ADR log.

## Success criteria

- A worker message reaches the operator agent through a durable Telex address.
- The operator agent creates a human-facing escalation that preserves source
  message and sender provenance and is understandable without opening the raw
  thread.
- The desktop Station surfaces the escalation in its feed and through a Windows
  notification under the spike's declared policy.
- A reply authored in the Station reaches the operator agent in the expected
  thread, and the operator agent can route the resulting decision or instruction
  back to the worker.
- The raw worker thread and mediated human thread remain separately inspectable
  and auditable.
- Restarting the Station does not lose the unresolved or recent conversation
  needed to continue the loop.
- The spike report distinguishes demonstrated product value from temporary
  integration behavior and gives `station-contract` actionable inputs.
- The builder can launch the viability gate without additional implementation.

## Engagement

- Review the worker's plan before implementation to keep the spike narrow and
  prevent temporary integration choices from becoming architecture by accident.
- Review the first complete worker-to-broker-to-Station-to-broker-to-worker demo
  before the PR is finalized.
- Conduct the multi-session dogfood separately at `viability-gate`; the worker
  should prepare the environment and walkthrough but must not self-pass the gate.
