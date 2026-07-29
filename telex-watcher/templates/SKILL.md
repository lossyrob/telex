# Detector template checklist

Use the authoritative [template library README](README.md).

1. Confirm the [trust and product boundary](README.md#trust-and-product-boundary).
2. [Select a template](README.md#select-a-template) and copy it; do not edit the
   runtime into a provider.
3. Follow [copy and customize](README.md#copy-and-customize), including
   `derivedFrom`, evidence versioning, fixtures, and helper/detector hashes.
4. Match the [manifest and provenance](README.md#manifests-and-provenance),
   [registration samples](README.md#registration-samples), allowed kinds,
   credential allowlist, interval, and downtime.
5. Preserve [event and cursor stability](README.md#event-and-cursor-stability),
   replay suppression, duplicate IDs, non-advancing degradation, and
   [initial emission semantics](README.md#initial-emission-semantics).
6. For PRs, perform the final [terminal preflight](README.md#terminal-behavior-and-pr-preflight)
   immediately before registration and seed `initialState.preflight`.
7. Review [credentials and rate budgets](README.md#credentials-and-rate-budgets),
   [downtime and restart](README.md#downtime-and-restart), and
   [pinned and follow-path operation](README.md#pinned-and-follow-path-operation).
8. Maintain sanitized [fixtures](README.md#fixture-maintenance) and run
   [validation](README.md#validation).
9. For upgrades, use the
   [customized-template reconciliation recipe](RECONCILING-CUSTOMIZATIONS.md).
10. Operate the detector as observation only; Watcher scheduling, Application
    Client delivery, provider mutation, and recipient action are outside this
    library.
