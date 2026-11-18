# Windows daemon lifecycle evidence and truthful attendance

- Workstream: local-daemon
- Node: windows-daemon-lifecycle-visibility
- Tracker: https://github.com/lossyrob/telex/issues/265
- State: planned; attention: watch; dependencies: none
- Proposal baseline: 091942b6eed0a4781e71db249e2b29492e0d8c82
- Accountable coordinator: Local aeae4fa9-c5fc-4da9-b60e-bcf5b1a98f57
- Sole candidate owner: 7c642fe4-3aa1-4d50-8cb8-363409a1706a

## Outcome

Distinguish a live Copilot bridge from a live authenticated daemon and an
attended push binding. Provide bounded owner-private evidence of daemon
readiness and intentional drain, truthful observation of daemon absence,
and actionable guidance for an existing authorized binding.

The #265 field report describes two unexplained daemon disappearances on
Windows/Postgres while the bridge remained loaded. Their causes are unknown.
This task does not diagnose those exits or promise that logging can observe
an abrupt termination. A missing terminal log record is not a crash cause,
proof that a lifecycle event did not occur, or proof of a complete audit.

## Existing surfaces and claim limits

At the proposal baseline, `daemon status` uses `connect_existing` and does
not spawn a daemon. Its authenticated connection/Hello and status response
can establish current readiness; a pipe, PID, successful launcher exit,
bridge tool response, or log line alone cannot.

Address `status` separately exposes `daemon_member_present`,
`durable_lease_live`, `station_health`, and `push_registered`. Preserve
those distinctions, including absent/unknown data. A durable lease can
outlive a daemon and does not by itself establish push attendance or its
loss. A live daemon alone also does not establish a registered producer.
The existing intent-based warning depends on a daemon status snapshot;
it cannot diagnose a missing daemon when that snapshot is unavailable.
Do not treat an unavailable snapshot or empty projection as authoritative
evidence that an intent was intentionally stopped or never existed.

Prefer these existing no-spawn status surfaces and human-readable guidance.
Keep `telex_bridge_info` as bridge-process evidence, not an attendance
certificate. No proactive bridge notification or automatic recovery is
promised by this slice. No new public JSON field, enum, receipt or protocol
schema is selected.

## Lifecycle evidence design checkpoint

Before adding or extending a product sink, report the smallest concrete
mapping of required lifecycle evidence onto existing capture surfaces,
including what each surface cannot establish.

The per-singleton `reconcile-events.ndjson` and its rotation are a CANDIDATE
only. Its current appends are deadline-budgeted narration, can be skipped,
and are not a complete audit. Reusing the name does not authorize new
lifecycle event semantics, a new writer, retention guarantees, a new store,
or additional blocking I/O on readiness or drain.

If existing surfaces cannot carry the needed evidence without changing
that contract, adding a store/public schema, or adding blocking I/O to
readiness/drain, surface that exact design fork and its bounded alternative
before adding the sink. Do not silently select it. Preserve owner-private
access, singleton/instance attribution, finite retention and existing
budgets. Evidence must distinguish observed readiness, requested drain,
confirmed exact-process exit, and externally observed abrupt loss.
An exit reason is reported only when actually observed.

Capture only necessary diagnostic facts. Do not retain capabilities,
credentials, DSNs, environment values, raw argv, message contents or helper
stderr. Diagnostics are not recovery or ownership authority. Failure or
absence of diagnostics must remain explicit rather than become fabricated
successful lifecycle evidence.

## Recovery guidance

For a confirmed existing, currently authorized, desired push binding,
guidance may name the supported explicit `copilot resume` path with the
same selected backend, address and existing options, followed by no-spawn
daemon status and address status verification. Do not recommend resume
for an intentionally stopped/detached station, another owner's binding,
or a binding whose ownership/desire cannot be established.

When that information is unavailable, state what is unknown and direct
the user to verify the intended binding; do not guess an address/backend,
claim lost push from lease alone, bypass identity mismatch, or initiate
registration. Preserve intentional stop/detach and all existing refusal,
authentication, fencing and ownership rules.

## Unchanged boundaries

Preserve byte-for-byte all three embedded modules:
`copilot/bridge/extension.mjs`, `busy-state.mjs`, and `probe-protocol.mjs`.
Preserve their fixture hash pins. Do not refresh or reload a producer,
alter its registration/heartbeat protocol, or weaken producer identity.

No automatic restart/resume/re-registration, supervision, service,
scheduled task, launch flag, Windows job/process-lifetime change, new
recovery authority, deadline extension, shared install/root/ACL change,
or release action belongs to this task. Keep #264's replacement-producer
recovery separate. Do not retarget historical #66/#106 or make #264,
#231 or #153 dependencies without a concrete dependency proof and review.
No existing gate, checkpoint, dependency or unrelated node is changed.

## Required Windows proof after separate write/runtime authorization

Use the exact branch binary by absolute path, disposable stations, isolated
TELEX_HOME, TELEX_DB, TELEX_INSTALL_ROOT and TELEX_RUN_DIR, and temporary
LOCALAPPDATA. Control only fixture-owned process identities. Never stop or
switch the installed user daemon or infer authority over a process by name.

1. Ready: establish a fixture-owned daemon and authorized push binding.
   Prove readiness by authenticated Hello/status, then show the distinct
   membership, lease, station-health and push-registration observations.
   Label a fixture bridge as such; it is not real App acceptance.
2. Intentional drain: issue supported `stop --drain`; require the existing
   identity-bound exact-process exit proof, not merely endpoint loss or
   drain acknowledgement. Keep intentional shutdown distinct from a
   diagnosed crash; preserve stopped/detached bindings.
3. Abrupt loss: terminate only the fixture-owned daemon instance, leave the
   bridge fixture live, and observe absence through no-spawn status.
   Verify that observation did not spawn a successor, report stale lease
   separately, and do not invent an exit cause or final daemon log event.
4. Guidance negatives: no lost-push inference from lease alone; no resume
   recommendation for stopped, detached, unowned or unknown bindings.
   Authentication/transport uncertainty is not silently reported as absence.
5. Evidence: bind the source/tree, binary, commands, actual outcomes,
   process identity and relied-on raw paths/lengths/SHA-256. Verify the
   unchanged embedded-module bytes and existing fixture pins. Exercise the
   selected bounded capture design and its refusal/failure limits.

These are future acceptance obligations, not authorization to run them now.
SQLite isolation may establish daemon-lifecycle behavior; it does not
reproduce the reported Postgres/Entra field exits or prove backend parity.
Any required shared-path/backend coverage remains explicit and must use
separately authorized isolated resources, not a new provider job.

## Launch and delivery gates

This task is a proposed launch prerequisite only. The existing candidate,
described by Campaign as GPT-6.2 Sol, remains read-only. Do not create a
second owner or change its model to match an older default.

After actual reviewed canonical landing, Campaign/Local must verify
exact-path preparation for that SAME existing owner, its actual branch,
HEAD, context paths and single-writer state, receive session-online, and
issue separate bounded write authorization. Preparation or this packet
alone grants none of those permissions. An exact-path preparation mismatch
must be reported, not bypassed by creating another owner.

Later product delivery still requires directly related docs/tests,
exact-head implementation review and CI, scoped design inspection and
the normal Campaign merge decision. This preparation opens no PR and
authorizes no product write, process experiment, provider job or release.
