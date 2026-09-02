# First supported Rust binding

- **Workstream:** `application-client`
- **Node:** `first-binding`
- **Type:** implementation
- **Status:** completed
- **Attention:** watch
- **Depends on:** completed `client-core`
- **Tracker:** [lossyrob/telex#149](https://github.com/lossyrob/telex/issues/149)
- **Merged:** [lossyrob/telex#151](https://github.com/lossyrob/telex/pull/151)
  at `ddedfab57cc305a1e91a81d7e49e712bb36d32fd` from exact reviewed head
  `c03db454781164f47a20e997665fe1251e07bd15`
- **Parent workstream:** [lossyrob/telex#117](https://github.com/lossyrob/telex/issues/117)
- **Campaign:** [Addressable Attention #102](https://github.com/lossyrob/telex/issues/102)

## Outcome

A Rust application can consume the supported Application Client from the root
`telex` crate through `telex::application_client`, with package defaults disabled
and an explicit supported backend profile. The published Rust surface defines
its compatibility, caller-owned runtime, and cancellation behavior while
preserving the complete API-neutral contract and avoiding a new language, ABI,
process, or product-specific boundary.

This node promoted the existing Rust core as the first supported binding. It did
not complete cross-backend conformance or authorize consumer integration.

## Design References

- [`../design/current-design.md`](../design/current-design.md) - canonical
  integrated design and first-binding promotion boundary.
- [`../../../../docs/design/application-client.md`](../../../../docs/design/application-client.md)
  - sole normative API-neutral contract, AC-C01 through AC-C20.
- [`../../../../docs/application-client-core.md`](../../../../docs/application-client-core.md)
  - merged Rust core, public semantic model, operating guidance, and required
  later conformance coverage.
- [`../../../../docs/design/DECISIONS.md`](../../../../docs/design/DECISIONS.md)
  - ADR 0049 shared-client boundary and private-fallback prohibition.
- [`../../../../Cargo.toml`](../../../../Cargo.toml) - root crate, current backend
  features, and package defaults.
- [Issue #12](https://github.com/lossyrob/telex/issues/12) - semantic ownership
  and binding publication revision 4.

## Inputs

- The merged `telex::application_client` core from PR #132, including its public
  contract-bearing Rust types and SQLite/Postgres implementations.
- The accepted API-neutral contract and non-normative requirements traceability.

## Exports

- A documented supported Rust binding at `telex::application_client` in the root
  `telex` crate.
- Supported `default-features = false` consumer profiles for SQLite, Postgres,
  Postgres with Entra, and dual-backend consumers, with `self-update` excluded.
- A clear compatibility contract for public contract-bearing Rust types and
  behavior, without promoting private implementation surfaces.
- A caller-owned Tokio runtime contract and cancellation rules that preserve
  recovery, exact-delivery acknowledgement, partial results, and compensation.
- Binding-level evidence that `client-conformance` can reuse without weakening
  or reinterpreting the accepted semantic matrix.

## Boundaries

### In scope

- Promote the existing Rust module as the first supported application binding.
- Make each supported backend profile buildable and consumable with package
  defaults disabled.
- Define and document which Rust types and behaviors carry compatibility
  commitments.
- Define runtime ownership and cancellation behavior for lifecycle, receive,
  acknowledgement, send/reply, and recovery operations.
- Preserve typed errors, opaque identities and metadata, exact-delivery handles,
  prepared recovery handles, evidence axes, versioned records, and
  backend-neutral behavior.
- Add the focused checks and durable guidance needed for downstream conformance.

### Out of scope

- Completing the SQLite/Postgres semantic matrix; owned by
  `client-conformance`.
- Watcher or Operator Station integration; blocked on `client-conformance` and
  `consumer-integration-gate`.
- napi-rs or another TypeScript binding, a separate client crate, C ABI, public
  socket or sidecar protocol, and consumer-specific DTOs; deferred pending later
  design authority.
- Product-specific convenience, routing, detector, mediation, notification, or
  workflow semantics; owned by the product workstreams.
- Packaging, install/upgrade, diagnostics, and production hardening beyond the
  root crate's Rust consumer contract; owned by `operational-hardening`.
- A private fallback through CLI parsing, raw daemon IPC, subprocess couriers,
  spike helpers, or product-private clients.

## Inherited decisions

- **Rust is the first supported binding.** Keep the binding in the root `telex`
  crate and preserve `telex::application_client`. A second crate or translation
  boundary would add versioning and maintenance cost without improving semantic
  fidelity for the current Rust-hosted consumers.
- **Application consumers disable defaults.** Supported profiles select `sqlite`,
  `postgres`, `entra` (which includes `postgres`), or `sqlite` plus `postgres`
  with optional `entra`. None includes `self-update` implicitly.
- **The caller owns Tokio.** The binding runs in the consumer's runtime and does
  not create a hidden runtime, daemon, or sidecar. Lifecycle operation objects
  preserve completed, uncertain in-flight, untouched, and compensation evidence.
- **Cancellation preserves uncertainty.** Cancellation does not prove
  non-acceptance. Retryable operations use persisted prepared recovery handles;
  cancelled receive work does not acknowledge delivery; lifecycle cancellation
  retains typed partial and compensation evidence.
- **Only contract-bearing Rust surfaces stabilize.** Backend rows, daemon frames,
  CLI types, private helpers, and product DTOs remain outside the public
  compatibility contract. Until a release contains the binding, supported
  source consumption uses an exact full commit SHA; unpinned Git dependencies
  remain outside the compatibility promise.
- **External bindings remain deferred, not rejected.** napi-rs/TypeScript and
  other ABI or process boundaries require separate authority after consumer
  architecture is known.
- **Conformance is not reduced or absorbed.** This node must preserve AC-C01
  through AC-C20 but does not mark the cross-backend matrix complete.

## Design-impact result

The corrected exact head remained within accepted authority. It documented the
supported Rust consumer, compatibility, runtime, and cancellation contracts
without changing the API-neutral semantic contract.

## Success criteria

- A minimal external Rust consumer imports `telex::application_client` from the
  root crate with `default-features = false` under every declared supported
  backend profile.
- No supported profile implicitly enables `self-update` or an unselected backend.
- Public documentation distinguishes stable contract-bearing Rust surfaces from
  private backend, daemon, CLI, and consumer implementation details.
- Runtime ownership and cancellation behavior preserve operation reconciliation,
  exact-delivery acknowledgement, and typed partial or compensation evidence.
- The binding preserves typed errors, opaque identities and metadata, recovery
  handles, independent evidence axes, and versioned records without stringifying
  or collapsing them.
- The implementation introduces no TypeScript/napi-rs, separate crate, C ABI,
  public socket/sidecar protocol, product DTO, or private fallback.
- `client-conformance` can start from the exported Rust surface without changing
  its semantic matrix or declared dependencies.
- Documentation makes no claim of completed conformance, consumer integration,
  production packaging, upgrade readiness, or operational hardening.

## Completion

PR #151 completed this node from exact reviewed head
`c03db454781164f47a20e997665fe1251e07bd15`, merged as
`ddedfab57cc305a1e91a81d7e49e712bb36d32fd`. Issue #149 closed as
completed at `2026-09-02T13:56:22Z`.

Full PAW review
[5052221269](https://github.com/lossyrob/telex/pull/151#pullrequestreview-5052221269)
and exact-head delta review
[5052369562](https://github.com/lossyrob/telex/pull/151#pullrequestreview-5052369562)
accepted the implementation after the three-way compensation correction. All
six checks succeeded, all three review threads were resolved, and
[field report 5454362721](https://github.com/lossyrob/telex/issues/149#issuecomment-5454362721)
records validation and downstream limits.

This completion does not advance `client-conformance`, consumer integration,
the `supported-client` checkpoint, packaging, operational hardening, or any
gate. `client-conformance` requires separate shaping, a tracker, a reviewed task
spec, and launch preparation.
