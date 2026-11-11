# Derived-tool offline component: local preparation only

**R2 local preparation:** [PREPARATION.md](PREPARATION.md) is the operative
supersession index and host-first refusal specification. The host policy is
unbound: no new decoder host, label or target is selected. No push or run is
authorized. The source closure now includes the five R2 preparation/test files
(17 files total); the 12-file references below describe historical3a evidence,
not authority for this new source. All consumed attempts and zero retries stand.
Only guarded offline tests are permitted locally.

This component prepares **Intel25 only** on a native ARM64 decoder host.
Intel25 describes an offline Intel image, not native x86 execution evidence.
The job uses explicit `macos-26` and fails unless architecture is arm64,
product is 26.6.2 and build is 25G83. The unchanged ARM24 configuration and
offline fixtures remain in source but ARM24 is not admitted by the hosted path.
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
observed and immediately rejected). Under
[6429885615](https://github.com/lossyrob/telex/issues/172#issuecomment-6429885615),
total response reads never exceed 27545841 bytes (cap + 1). Reaching cap + 1
is terminal: no second read or retry, and no excess byte is written, extracted
or used. A running cap + 1 hash is labeled **OVERFLOW FAILURE EVIDENCE**, never
a ZIP identity and never compared against or substituted for the pinned ZIP
hash. Success still requires exactly 27545840 bytes and EOF without a sentinel.
ZIP length and full hash must match before
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

Under [6468291693](https://github.com/lossyrob/telex/issues/172#issuecomment-6468291693),
the only recognized authority syntax is the versioned
`intel25-families-authority-v1` comment schema implemented in `gate.py`.
Old dual-context/static retry markers and BUILD approvals are not recognized. This document
does not create an authority record or supply a launch marker.

All three owner-routed comments must be canonical issue-172 comments authored
by `lossyrob` (account ID 2320142), unedited, distinct and ordered by creation time:

1. A fresh independent component-source **APPROVE**.
2. A new steward finite component disposition.
3. A new genuine owner freeze.

Every record binds `component=intel25-families-v1`, the exact current
`sha`, `bounds_sha256`, ordered `source_manifest_sha256`, workflow
`.github/workflows/macos-cross-static.yml`, jobs exactly `[Intel25]`,
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

**One Intel25-only component attempt, zero retries, including infrastructure.**
The workflow admits attempt 1 only. A rerun, failed-job retry, repeat same-source
push, expiry or any failure returns to design. The workflow contains only the
Intel25 job; the gate's `ATTEMPT_JOBS = ("Intel25",)` is also the run admission
source of truth, so an ARM24 argv fails before host, gate or filesystem work.
The gate rejects bounds with a different component identity. The 12-file
source closure, authority headers/keys/order/refetch rules and candidate
bindings are unchanged. All five prior BUILD attempts and both a06 and 3d
component attempts are consumed.

## All-present Intel families, no selection

The operator authorized this local preparation and at most one future gated
Intel25 attempt on 2026-10-28. It does not authorize another ARM24 capture.
[6432495216](https://github.com/lossyrob/telex/issues/172#issuecomment-6432495216)
requires classification of all four original image-relative locations:
the `dyld_shared_cache_x86_64` and `dyld_shared_cache_x86_64h` primaries under
`System/Library/dyld` and `System/Library/Caches/com.apple.dyld`.
The unchanged confinement helper treats only ENOENT as absence and rejects
symlinked ancestors/entries, nonregular or unreadable files, aliases and
cross-device traversal. Each present primary's path, lstat type, size, full
SHA256 and magic bytes are retained in `intel-family-inventory.json`.
File identity/size are checked again after bounded deadline-controlled hashing.

Every present family is analyzed independently; none is selected over another.
Zero families, duplicates within a family (even identical bytes), wrong
location/content family, or any failed domain check remains decision-needed.
The header's full family token must match its domain. Every parsed library or
helper image requires raw CPU `0x1000007`, raw subtype 3 for x86_64 or 8 for
x86_64h, and zero capability bits, without masking. Layout, declared subcache
UUIDs, both selected system libraries, roots, wrappers and helpers are checked
by the same complete reader for each domain.

Domain files and disassembly command records have `x86_64-` or `x86_64h-`
prefixes in the one flat evidence directory. Names within each domain report
are relative to its recorded `evidence_prefix`; equal UUIDs/addresses/body
indices across domains cannot overwrite each other. The inherited subcache
table write is routed through that same bounded sink.

Both domains share the existing job's function, entry-analysis, code-byte,
image-metadata count/byte and fixup-page budgets. File count, per-file and total
evidence size use one job sink; all work shares the original processing
deadline, and Intel cache reads check it. Per-analysis CFG, per-function and
per-image limits remain unchanged. Exhaustion returns to design, never a
larger cap or partial success. No real image fit is established by tiny
synthetic tests.

The image is decoded once. Existing exact-child confirmation and the full
post-decoder native snapshot must complete before *either* domain; both domain
records bind that same `native-binding-acceptance.json`. No extra decoder
invocation or native snapshot is introduced. Final native/tool checks remain
after all domains and cleanup. `intel-domain-results.json` is provisional
domain status, not final native or semantic acceptance. Every present domain
must have no missing wrapper records and pass; a failed domain cannot be
replaced by a successful sibling. Runtime callbacks and helper gaps are not
automatically exempted.

ARM24's reader/CPU rules, execution branch and callback classifications stay
unchanged, but its hosted entry is unreachable. Its separate retained-only
classification is owner work, not implemented or adjudicated here. Intel
results remain offline candidates, with no claim about which family a real
Intel host loads, native correspondence or physical Intel coverage.

## Preserved native and input contract

The three inherited dependencies remain unchanged. `cache.py` adds the Intel
domain inventory, namespace and shared-budget path without changing ARM24
semantics. `native.py` retains the previously reviewed narrow trace grammar
described below without further changes.
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

## Exact-child delayed-image non-evidence record

[6430907843](https://github.com/lossyrob/telex/issues/172#issuecomment-6430907843)
classifies the consumed a06 component attempt as a controller trace-grammar
gap. It permits only the literal exact-child form
`move loaded to delayed: <name>` when the name is nonempty, has no whitespace
or `/`, and has no trailing text. The name must exactly match the last path
component of at least one **earlier UUID/path row of the same exact child**.
No matching earlier row may be the selected library; the delayed record must
also fail both the existing `related_path` and `selected_hint` relevance tests.
Any violation rejects the record.

These delayed records supply **no identity evidence** and never enter the
returned selected-library records. Exactly one selected exact-child UUID/path
row is still mandatory; delayed records cannot supply a missing row or excuse
a duplicate. All other non-UUID/path exact-child forms remain rejected.
Non-child and non-dyld handling, return shape, UUID/path validation, SDK/otool,
post-decoder confirmation and semantic acceptance order are unchanged.

Optional offline replay of the two retained stderr streams exercises only this
pure parser under effect guards. It is source-review evidence, not acceptance
of the old decoded output, a native recheck or a new candidate invocation.
Both old decoder children returned zero with complete pipes, but the consumed
attempt reached no post-decoder confirmation, semantics or native acceptance.
The grammar correction makes no claim about dyld's internal delayed-loading
behavior or decoder correctness.

All five BUILDs, a06 component attempt 40425113395/1 and 3d component attempt
40435086116/1 remain consumed, with
zero retries and HOLD unchanged. Before any future attempt: fresh independent
WHOLE component-source review, a NEW steward finite disposition binding it,
then a NEW genuine owner freeze. No approval transfers from source review
6430317350 or ACTUAL verdict 6430817786; this local correction grants no use,
push, freeze or attempt authority.
