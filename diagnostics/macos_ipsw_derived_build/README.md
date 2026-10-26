# Isolated derived ipsw build

Preparation only for issue #172. This route never invokes the resulting `ipsw`.
It needs its own exact-source BUILD review and freeze, permits attempt 1 only,
and changes no existing component pin or workflow.

The unchanged owner patch separates the three `sw_vers` stdout results from
private stderr. It does not establish the cause of the earlier failed run.
The resulting executable is derived tooling, not the published release.

Capacity follows dispositions 6419297367 and 6419617835: sequential, counted
HTTP-body prefetch into a frozen file proxy; a fixed non-sparse read-write
image; and an initial provisioned reserve before transfers. No process-socket
isolation, host-wide disk quota, live reserve floor or reproducibility is claimed.
The controller records known outside-volume writes and all failure evidence.
It detaches but never unlinks the backing image, on success or failure; runner
teardown is its lifetime limit. Persisted provenance is provisional: collection
requires a zero return after manifest finalization and a verified artifact set.
The binary remains subject to independent review, not pre-finish acceptance.

Synthetic tests use mocked processes, HTTP transports and filesystem metadata.
Do not run `build.py` locally. Review `bounds.json` and the exact source packet
before any hosted action. A failure consumes the one attempt and returns to
design; there is no infrastructure retry.
