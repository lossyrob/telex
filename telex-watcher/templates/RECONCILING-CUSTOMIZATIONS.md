# Reconciling a customized detector

Use this recipe when a copied detector records `derivedFrom` an older library
version.

1. Pause the watch. Save its registration, committed state, installed detector
   digest, helper digest, and upstream `derivedFrom` record.
2. Read `CHANGELOG.md` from the old version through the candidate version.
   Identify protocol, manifest, evidence-normalization, event-kind, credential,
   provider API, cadence, terminal, and downtime changes.
3. Diff three inputs: the old library template, the user-owned copy, and the new
   library template. Preserve intentional local policy rather than replacing
   the copy wholesale.
4. Merge provider query and safety fixes first. Keep the result observational;
   reject any provider mutation or configurable reaction.
5. Reconcile normalized evidence deliberately. If its composition changes,
   increment the copied evidence-normalization/template version and decide
   whether old cursor state can be migrated. Otherwise establish a reviewed new
   baseline. The current helper recursively sorts object keys before hashing,
   so source-code insertion order is irrelevant; do not replace it with plain
   compact JSON hashing.
6. Reconcile allowed kinds, credentials, calls per attempt, interval, terminal
   semantics, and `maxSafeDowntimeSeconds`. Kind changes require an operator
   checkpoint before resume. Enabling `emitInitialSnapshot` or
   `emitInitialCreatedEvent` requires authorizing the matching synthetic kind;
   disabling it should remove that kind from the registration policy.
7. Refresh sanitized fixtures. Run the copied detector twice per fixture to
   prove stable cursor/event IDs and replay suppression, plus terminal preflight
   race cases for PR templates.
8. Recompute the helper digest embedded in the detector, the detector digest in
   the copied manifest, and the pinned registration digest.
9. Update `derivedFrom` to the new upstream template/version/digests and record
   retained local differences. `derivedFrom.reconciliation` is a non-empty
   relative path: point it at the copied guidance location rather than assuming
   the shipped `../RECONCILING-CUSTOMIZATIONS.md` layout.
10. For a PR watch, run the provider preflight as the final step immediately
    before update/registration, and put its output only in
    `initialState.preflight`. Install the new pinned bytes and explicitly
    resume. Monitor the first attempt and do not send or perform a provider
    action from this reconciliation process.
