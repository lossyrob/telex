# Telex Copilot plugin

The plugin contributes two agent-session skills:

- `telex`: the thin bootstrap for general Telex coordination;
- `operator-station`: the reusable assisted-mode operator role defined by
  [the Operator Station contract](https://github.com/lossyrob/telex/blob/main/docs/design/operator-station.md).

## Invoke the operator role

Ask Copilot to use `operator-station` and provide every required value:

- a named Telex backend;
- the worker-facing ingress address;
- the distinct human-facing address;
- policy `normal` or `quiet`.

The role rejects implicit backends, missing addresses, identical addresses, direct
routing, and unknown policies. It first loads exact command syntax and compatibility
information from `telex copilot skill`; the packaged role defines policy, ordering,
evidence, and capability checks rather than duplicating binary syntax.

Example assignment text:

```text
Use operator-station with backend local-campaign, ingress work:operator,
human attention:operator, and policy quiet.
```

## Durable diagnostics

Use the version-matched Telex workflow to inspect:

- the captured package/build and workflow signature;
- ingress and human-address station health, backlog, and latest receive evidence;
- raw and mediated thread messages with complete disposition history;
- `operator-station-op-v1` mediation and operation records;
- accepted, duplicate, rejected, indeterminate, blocked, and recovery notes;
- quiet pending-digest windows and frozen source sets;
- routed-outcome, stale-origin, transition, and handoff evidence.

An operator never treats delivery, notification, occupancy, or transcript memory as
completion evidence. Missing metadata-bearing replies, exact receipt identity, ordered
history, or foreign-station health is a blocked compatibility condition.

The checked-in
`skills/operator-station/compatibility-v0.1.2.json` fixture ties the role to the Telex
and plugin release. Package version changes require explicit fixture and contract-test
review. Compatibility is capability-gated: the generic v0.1.2 reply surface does not
expose metadata-bearing authoring, so human-response route-back remains visibly blocked
unless the loaded runtime workflow supplies that capability.
