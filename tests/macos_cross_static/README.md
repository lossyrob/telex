# Derived-tool offline component: local preparation only

This component prepares two offline input contexts, ARM24 and Intel25, on
**native ARM64 decoder hosts**. Intel25 describes the x86_64 offline image
family, not native x86 execution evidence. Both jobs use explicit `macos-26`
and fail unless architecture is arm64, product is 26.6.2 and build is 25G83.
Runner image version remains recorded.

The derived BUILD stage is complete under
[6429489865](https://github.com/lossyrob/telex/issues/172#issuecomment-6429489865).
This source preparation grants no component attempt, decoder invocation,
source approval, freeze, product/admission/merge or HOLD credit.

## Sole candidate and retrieval

Only artifact **12694487625**, from BUILD **40414940708/1**, job
**121894755319**, source `6eeaeb5c73458ac9fd88c4792d5e95d10a6038f2`,
tree `21ba65ff07544135a821e78a0b972df539a81f27`, is permitted:

- Artifact name: `derived-ipsw-6eeaeb5c73458ac9fd88c4792d5e95d10a6038f2-attempt1`.
- ZIP: 27545840 bytes, SHA256
  `7e65107b8312ea2ec79aff47431d15fc40372da82d3798a0688baef7dcbcef0c`.
- Post-sign `ipsw`: 83115984 bytes, SHA256
  `89914d079265befacac4e648627a3c64c64d1bb54667b2c96c849fa9966309b2`.
- AppVersion: `3.1.739-telex-swvers-b401e5777696`.
- AppBuildCommit: `1f8b657858d7510219f59bf9ba183bf4c0a59104+telex-b401e5777696`.
- Expiration: **2026-11-25T22:53:07Z**.

The pre-sign controller record and old published release hash are not candidate
identities. No local override, substituted copy, rebuild, new release, fallback
source or retry exists. Artifact expiry requires a new steward decision.

The future path verifies the authenticated artifact metadata and successful
originating BUILD run/job, retention and exact source/name/digest. The artifact
metadata schema is exact; unknown/missing fields fail closed. Run/job responses
may contain unrelated API fields, but every identity/status field used by the
contract must have the expected type and exact value.

[Transport disposition 6429608421](https://github.com/lossyrob/telex/issues/172#issuecomment-6429608421)
supersedes the unobserved, proposed storage-host allowlist. The exact GitHub
artifact ZIP API must return one HTTP 302 with exactly one Location. Any ASCII
HTTPS hostname is allowed, without userinfo or a non-443 explicit port.
Malformed URLs, control characters, fragments and ambiguous Location headers
are rejected. This does not claim knowledge of a historical storage host.
A fresh opener sends no Authorization, GitHub token or Cookie to storage and
follows no further redirects. Both API and storage openers explicitly disable
environment/system proxies. API metadata redirects are refused.

Reads have deadlines and exact byte limits (one overflow sentinel byte may be
observed and immediately rejected). ZIP length and full hash must match before
ZIP parsing or extraction. All 55 flat regular members, total 86459578 expanded
bytes, and the complete 54-entry manifest are checked; duplicate keys/names,
unsafe paths, links, encryption, oversized members, missing candidate and hash
mismatches fail. No member path is extracted. Only `ipsw` is copied to a fresh
private path, then its exact post-sign size/hash is checked before executable
mode is set. No candidate is invoked by this verifier.

The verification receipt contains status, redirect **hostname only**, byte
counts/hashes and phase. A failure remains unverified. It never stores the
signed URL, query, token or response headers. HTTP/transport errors are
sanitized. No confidentiality/availability beyond these protocol rules, TLS
and credential isolation is claimed.

Every component command using `ipsw`, including `version`, requires successful
artifact acquisition, current retention and a fresh exact candidate rehash
before dispatch. The existing native decoder-launch rehash/ARM64 slice check
also remains. The version command must return exactly the two derived values
in the upstream `Version: ..., BuildCommit: ...` line, not base-version
substring matches. Acceptance checks retention again.

## Fresh authority sequence, zero retries

The only recognized authority syntax is the versioned
`arm24-intel25-derived-authority-v1` comment schema implemented in `gate.py`.
Old static retry markers and BUILD approvals are not recognized. This document
does not create an authority record or supply a launch marker.

All three owner-routed comments must be canonical issue-172 comments authored
by `lossyrob` (account ID 2320142), unedited, distinct and ordered by creation time:

1. A fresh independent component-source **APPROVE**.
2. A new steward finite component disposition.
3. A new genuine owner freeze.

Every record binds `component=arm24-intel25-derived-v1`, the exact current
`sha`, `bounds_sha256`, ordered `source_manifest_sha256`, workflow
`.github/workflows/macos-cross-static.yml`, jobs `[ARM24, Intel25]`,
artifact ID, candidate hash/size, expiry, `attempt=1` and `retries=0`.
Unknown fields, missing keys, wrong types or ambiguous matches fail closed.

The review adds `kind=review`, `verdict=APPROVE`,
`scope=whole-component-source`; its body starts
`APPROVE - independent component-source review`.
The finite record adds `kind=finite`, `source_review` (canonical comment URL)
and `source_review_body_sha256`; its body starts
`Design steward finite component disposition`.
The freeze adds `kind=freeze`, the same review URL/hash,
`finite_disposition` and `finite_body_sha256`, plus `allow_launch=true`;
its body starts `NEW genuine owner component freeze`.

The gate requires review < finite < freeze < actual run creation < expiry,
then rereads all three complete comments and verifies no change. It binds the
current dedicated push/branch/workflow/attempt and rejects another same-source
component run. Comments are read through bounded, authenticated, no-redirect
GitHub reads. The exact ordered closure includes these component sources/docs,
the three existing dependencies and workflow.

Semantic independence of the owner-routed reviewer and steward remains an
owner/reviewer obligation; these syntactic bindings are not a substitute for
their decisions. No earlier BUILD/source/actual approval transfers.

**One two-context component attempt, zero retries, including infrastructure.**
The workflow admits attempt 1 only. A rerun, failed-job retry, repeat same-source
push, expiry or any failure returns to design. Both jobs are contexts within
that one attempt, not two independently renewable budgets. All five prior
BUILD attempts are consumed.

## Preserved native and input contract

`native.py`, `cache.py` and the three inherited dependencies remain unchanged.
Native resolution/before-transfer binding precedes acquisition and all input
transfers. Candidate verification/version checks remain before the original
Apple input chain. Seven roots, query-wrapper exclusions, family selection,
raw CPU/subtype rules, image inputs/caps/times and 7zz pin are unchanged.

The SDK A/B and otool C/D policies, exact-child library trace, effective decoder
environment including DYLD tracing, post-decoder checks **before semantic
processing**, and final native/tool hashes are unchanged. No DYLD clearing,
selected-image execution, native validation relaxation or BUILD-to-loader
success inference is made.

Closed-source sandbox features are absent. Decoder correctness, native
containment, original failure cause, release-feature/whole-binary equivalence,
reproducibility, physical/admission/product readiness and HOLD clearance are
not established. Guarded local tests use tiny synthetic archives, API objects,
timestamps and denied effects, not the retained candidate executable.
