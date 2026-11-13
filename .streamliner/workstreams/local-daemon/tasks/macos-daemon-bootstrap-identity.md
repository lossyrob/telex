# Identity-verified macOS daemon bootstrap

- **Workstream/node:** local-daemon / macos-daemon-bootstrap-identity
- **Tracker:** [#259](https://github.com/lossyrob/telex/issues/259); one separate product PR says `Closes #259`.
- **Status/attention:** planned / watch; not implemented or accepted.
- **Accountable coordinator:** Local aeae4fa9-c5fc-4da9-b60e-bcf5b1a98f57.
- **Implementation owner:** PENDING external MacBook routing. Campaign withdrew the
  conditional normal App-owner launch. No Windows/App session is created and the
  existing #257 containment writer is not implicitly reassigned.
- **Depends on:** none. **Blocks:** full Watcher #172 acceptance through its pending
  external dependency; not independent containment development/review, Local RC2
  or the next release. No checkpoint is added.

## Outcome

Implement a supported macOS binding between the selected executable image and
the running shared daemon peer, without reducing bootstrap identity/admission
guarantees. Preserve selected build/version/protocol/capability evidence and
user/process identity checks. An unsupported or indeterminate guarantee remains
a concrete fail-closed result. If no equivalent unprivileged primitive meets
the contract, return the missing guarantee and a narrow design decision; do not
trust a path string, PID alone or a successful handshake, or remove the refusal.

## Inputs and evidence boundary

The complete current [#259 specification](https://github.com/lossyrob/telex/issues/259) owns this bounded outcome.
Its b0d63dbb source classification is Local-reported Windows source observation;
the macOS symptom is the external #257 author's disclosure. Local and this
reconciler have not reproduced it on a Mac. This is not a flaky-readiness finding,
native proof, accepted repair, CI result or containment acceptance.

## Acceptance and handoff

1. Independent exact-head implementation/design review accepts the actual
   selected-image/running-peer identity guarantee, with clear fail-closed
   unsupported/indeterminate behavior.
2. Native macOS evidence covers both supported bootstrap selection modes,
   existing-peer connection and spawn/reconnect, including load-bearing negative
   identity, selection and replacement cases. Record actual architecture, OS and
   source without generalizing unexecuted coverage or creating a version gate.
   Use task-owned isolated config/backend/install/runtime roots and the candidate
   binary by absolute path; do not stop or change the operator's installed daemon.
3. The shared bootstrapped ApplicationClient path reaches legitimate sender
   readiness without bypassing admission or identity checks. App coordinates the
   downstream readiness/backend integration and exact consumer proof; do not
   duplicate that lane or fork a private Watcher daemon connection.
4. Return the actual merged commit and source-bound evidence to App and Watcher.
   #172 still needs full CI, independent review and SQLite/Postgres lifecycle and
   health parity with unchanged sender, receipt, store-binding and readiness
   fences. Upstream acceptance does not close those consumer gates.
5. Preserve Windows/Linux identity behavior and unrelated daemon guarantees.
   Directly related product documentation and focused regression tests match the
   supported/unsupported behavior in the closing #259 PR.

App confirms the production consumer seam as
`ApplicationClient::connect_with_daemon` with
`ApplicationDaemonBootstrap::InstalledCurrent { trusted_root }`: the explicit
absolute trusted root is canonicalized/frozen before connect/reconnect, using
Local selector admission with no PATH search or fallback to ExactExecutable or
legacy connect. App's later consumer proof must attach the actual sender and
observe the exact-address health record registered and sender-ready; send-only
addresses remain non-receiving/non-attended-but-deaf with no inbound pending or
actionable evidence. App owns isolated SQLite and credentialed Postgres parity
through this public seam; these are downstream duties, not new #259 proof credit.

## Launch and scope

Canonical publication of this node, tracker and #172 dependency is a real launch
prerequisite. Handoff preparation is not an implementation assignment or kickoff.
Actual external owner label, branch and source binding remain pending operator
routing. No reused App launch binding or implicit #257 ownership transfer.
The eventual #259 change uses a separate task branch/checkout and its own closing
PR, not the existing containment product PR.

No privilege, entitlement, persistent/shared helper, host restart, shared
installation, polling, release or native run is authorized by this promotion.
Do not absorb #251 RC2, #254/#255 policy documentation, #256 fixture repair or
#172 containment implementation. Product #256/#255 and existing #257 work do not
wait on optional bookkeeping. No finite/per-kernel-proof or version-only
admission gate, new App graph gate or release dependency is introduced.
