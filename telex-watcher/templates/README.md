# Telex Watcher detector template library

This directory contains editable PowerShell 7 detector templates for the
version 1 Watcher request/result protocol. The templates are trusted same-user
observational code. They are not sandboxed runtime providers, they do not
schedule themselves, and they do not perform provider or follow-up actions.
Watcher owns cadence, timeout, routing, allowed event kinds, state commits, and
Telex delivery.

The canonical protocol is defined by
[`watcher-detector-request-v1.schema.json`](../../docs/design/schemas/watcher-detector-request-v1.schema.json)
and
[`watcher-detector-result-v1.schema.json`](../../docs/design/schemas/watcher-detector-result-v1.schema.json).
Every detector reads one request from stdin, writes exactly one result to
stdout, and exits zero after communicating `idle`, `event`, `terminal`, or
`degraded`.

Agents can use the concise [detector template checklist](AGENT.md) while
working through this authoritative guide.

## Trust and product boundary

- Review every copied script, manifest, fixture, argv, environment allowlist,
  sender, target, and event-kind policy before registration.
- Provider logic remains in the copied detector. Do not add provider branches
  to the Watcher runtime.
- A detector observes only. It must not merge, approve, comment, mutate a file
  as a reaction, queue a build, or expose a configurable follow-up action.
- Watcher clears the detector environment, restores its documented safe
  process baseline, and then adds only registration-allowlisted variables.
  Credential values never belong in requests, state, fixtures, manifests,
  registrations, event metadata, or diagnostics.
- Event-producing state is committed by Watcher only after durable Telex
  acceptance. Templates do not implement delivery or receipt logic.
- Committed template state contains an opaque evidence `cursor` and
  non-negative `occurrence`. The occurrence is derived only from prior
  committed state, never from attempt IDs or wall-clock time.
- Detector stderr is local health evidence only. Watcher redacts allowlisted
  secret values before retaining diagnostics; stderr is never a Telex event
  body or cursor input.

## Select a template

| Template | Use it for | Calls per attempt | Recommended interval | Safe downtime |
| --- | --- | ---: | ---: | ---: |
| `github-pr` | PR readiness, attention, completion, and optional synthetic first snapshot | 1 | 300 s | 900 s |
| `github-pr-external-activity` | Editable policy for substantive activity by identities outside an ignore set | 1 | 300 s | 3600 s |
| `azure-devops-pr` | Azure DevOps PR review, merge, thread, creation, and completion state | 2 | 300 s | 1800 s |
| `http-json` | One scalar condition in a bounded read-only HTTPS JSON response | 1 | 60 s | 300 s |
| `local-file-json` | One scalar condition in a local JSON document | 1 | 30 s | 300 s |
| `local-command` | A fixed observational argv whose exit code classifies a condition | 1 | 60 s | 300 s |

The manifest is authoritative for versions, source digests, allowed kinds,
credential policy, provider API shape, rate cost, interval floor, cursor
behavior, and downtime rationale.

## Copy and customize

1. Copy one template directory and every `librarySource.supportFiles` entry
   into a private, reviewed location. This always includes
   `shared/DetectorCommon.psm1`; `local-command` also includes
   `shared/BoundedCommand.psm1`. Do not edit the library copy in place for one
   watch.
2. Record `derivedFrom` with the upstream template ID, template version,
   detector digest, helper digest, and evidence-normalization version. The
   reconciliation path is any non-empty relative path; the shipped layout uses
   `../RECONCILING-CUSTOMIZATIONS.md`.
3. Edit provider policy, parameters, event text, and fixtures. Keep the command
   observational and preserve the canonical stdin/stdout envelope.
4. If normalized evidence changes, increment the evidence-normalization version
   and template version. Pause existing watches, update/repin, then explicitly
   resume them because cursor and event IDs may change.
5. Recompute the helper digest embedded near the top of `detector.ps1`, then
   recompute the detector digest. The embedded check makes a pinned detector
   fail closed as `degraded` if its imported helper bytes drift.
6. Update the copied manifest and registration kind policy, credentials,
   cadence, and downtime bound.
7. Refresh sanitized fixtures and run [validation](#validation).
8. Use the pinned sample for production. Use follow-path only while developing.

See [Reconciling customized templates](RECONCILING-CUSTOMIZATIONS.md) when a
new library version is available.

## Manifests and provenance

`manifest.schema.json` is strict: required keys are enforced and unknown
top-level or nested keys are rejected. Each `manifest.json` declares:

- manifest schema, template, detector protocol, and evidence-normalization
  versions;
- PowerShell runtime requirements;
- detector and helper paths with lowercase SHA-256 digests;
- `derivedFrom` guidance;
- exact allowed event kinds;
- cursor/replay, terminal, duplicate, initial-emission, and diagnostic
  semantics;
- credential policy, conditional credential requirements, and provider API
  version;
- provider calls per attempt; and
- recommended/minimum interval plus `maxSafeDowntimeSeconds` and rationale.

The source digest is over the exact shipped bytes. A customized copy must not
claim the library digest. Its installed pinned digest is the hash of its
reviewed detector bytes.

## Registration samples

Every template ships:

- `registrations/pinned.json`, the recommended production shape with
  `scriptDigest`; and
- `registrations/development.json`, an explicit `follow-path` shape without
  `scriptDigest`.

Both include exact `allowedEventKinds`, an empty prefix list,
`maxSafeDowntimeSeconds`, interval, credentials, and opaque initial state.
Replace every angle-bracket path/command placeholder, address, backend
profile, and provider identity placeholder. The path tokens are intentionally
platform-neutral; substitute native absolute paths before registration.

Pinned PR samples are least-privilege. They do not authorize synthetic
`snapshot` or `created` kinds while their corresponding parameters are false.
Development samples may opt into and authorize the wider vocabulary. Changing
an initial-emission parameter requires the coupled allowed-kind policy change,
which is an operator pause/update/resume checkpoint.

These samples target the production registration contract in
[`watcher.md`](../../docs/design/watcher.md). The current experimental
`WatchSpec` may not yet accept production-only fields such as
`detectorSchemaVersion`, `backendProfile`, allowed-kind policy, `initialState`,
or downtime. Template conformance validates the samples independently; it does
not expand or start the experimental runtime.

## Event and cursor stability

The shared helper recursively sorts object keys using ordinal comparison,
serializes compact JSON, and stores its lowercase SHA-256 in
`nextState.cursor`. Detectors use explicit ordinal total comparators for
set-like provider arrays before hashing; genuinely ordered provider arrays
retain their order. Human-facing reason text is not evidence.

`nextState.occurrence` starts from zero when absent and advances once for every
changed observation that Watcher commits. Event IDs combine provider scope,
the candidate occurrence, and the first 24 hex characters of the cursor. The
candidate is derived from prior committed state, so retries before commit
produce the same ID. After A -> B -> A commits, the second A has a higher
occurrence and therefore a different ID even though its cursor matches the
first A.

- An unchanged cursor returns `idle`, preserves occurrence, and suppresses
  replay.
- A changed observation with no event still returns `idle.nextState`; that
  means the observation was intentionally classified. If Watcher commits that
  idle result, it also commits the incremented occurrence. Changed idle
  observations therefore consume an occurrence without emitting an event.
- `degraded` never contains `event` or `nextState`, so failed evaluation cannot
  advance the cursor.
- At-least-once delivery may repeat an accepted event. Recipients deduplicate
  by the stable event ID.
- Changing evidence composition can change cursors and event IDs. Version and
  operate that change explicitly.

## Initial emission semantics

`emitInitialSnapshot` and `emitInitialCreatedEvent` control only synthetic
baseline events that a PR detector explicitly constructs. They never suppress
a naturally actionable or terminal condition:

- first-attempt attention, ready-to-merge, completion, matched HTTP/local
  conditions, and external activity emit normally;
- GitHub snapshot emits only when `emitInitialSnapshot` is true;
- Azure DevOps snapshot/created emit only when their respective parameter is
  true; and
- templates without a synthetic kind do not accept either parameter.

Preflight-declared terminal remains eventless regardless of those flags.

## Terminal behavior and PR preflight

The generic GitHub and Azure DevOps templates emit terminal completion events
without preflight or after an established watch. The customized GitHub
activity template ends with an eventless terminal result. All three consume
preflight evidence only from `initialState.preflight`.

Run the appropriate helper as the final numbered step immediately before
registration:

1. Finish every path, credential, policy, and digest edit.
2. Run fixture and conformance validation.
3. Prepare the final registration JSON, but do not register it.
4. Run `shared/github-pr-preflight.ps1` for either GitHub template, or
   `shared/azure-devops-pr-preflight.ps1` for Azure DevOps.
5. Apply the preflight exit table below. Only exit 0 permits registration.
6. Put the helper's JSON object in `initialState.preflight` without editing its
   identity evidence.
7. Register immediately. Do not perform unrelated work between preflight and
   registration.

Example:

```powershell
$preflight = pwsh -NoLogo -NoProfile -File .\shared\github-pr-preflight.ps1 `
  -Repository OWNER/REPOSITORY -PullRequestNumber 123 | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw 'PR preflight rejected registration' }
$registration.initialState = @{ preflight = $preflight }
```

This is not a lock on the provider. If the PR terminalizes after preflight, the
first detector attempt recognizes the terminal provider state and returns
eventless `terminal`, so the race cannot leave an active stale watch or send a
misleading initial completion event.

| Exit | Meaning | Operator action |
| ---: | --- | --- |
| 0 | Safe non-terminal observation | Put stdout JSON in `initialState.preflight` and register immediately |
| 3 | Provider object is already terminal | Abort registration |
| 4 | Provider authentication or transport failed | Repair credentials/connectivity and rerun |
| 5 | Test-mode misuse, invalid RFC3339 input, fixture/provider JSON parse, or shape failure | Repair input/tooling and rerun |

`-TestMode`, `-FixturePath`, and `-Now` are hidden conformance-only helper
arguments. `FixturePath` and `Now` are refused without `-TestMode`, and `Now`
must be RFC3339. Production registration must use a live helper call.

On preflight identity/template/timestamp mismatch, the detector returns
schema-valid `degraded` and emits a structured
`detectorDiagnostic.code=preflight-identity-mismatch` record on stderr. The
diagnostic repeats until the watch is re-registered with fresh matching
preflight; it is not a new result-schema or blocked-reason vocabulary.

## Credentials and rate budgets

The runtime provides generic cadence, jitter, concurrency, and backoff. The
manifest owns provider-specific budget assumptions.

- GitHub uses one `gh pr view` call. Use the GitHub CLI credential store or
  allowlist `GH_TOKEN`.
- The customized GitHub external-activity template hashes complete normalized
  review/comment activity, including comment-body digests, into compact cursor
  evidence. Event metadata contains counts, a bounded PR URL, and at most 16
  body-free review/comment entries with explicit field/list truncation flags,
  keeping serialized detector metadata below the 64 KiB protocol cap.
- Azure DevOps uses two REST 7.1 GETs. Select bearer
  `AZURE_DEVOPS_ACCESS_TOKEN` or PAT `AZURE_DEVOPS_EXT_PAT`, never both.
- HTTP/JSON uses one HTTPS GET, rejects redirects, limits content to 1 MiB, and
  supports no auth, bearer `HTTP_JSON_BEARER_TOKEN`, or one named header
  `HTTP_JSON_HEADER_VALUE`.
- Local templates inherit no credentials by default. `local-command` starts
  its child with the detector environment that Watcher already sanitized and
  allowlisted; it has no parameter-driven secret pass-through.

The HTTP/JSON and local-file JSON matchers require an explicit scalar
`expectedValue` (including explicit JSON null). Omitting it is a configuration
error. A missing field is distinct from a present null and never matches.
Azure DevOps vote `-5` is waiting-for-author, not rejection; the default
`blockingReviewerVoteAtMost` is `-10`.

Increasing frequency multiplies manifest `callsPerAttempt` across every watch
sharing a credential. Account for provider quotas and keep intervals at or
above the manifest minimum.

## Downtime and restart

Watcher runs one overdue attempt after restart, not one attempt per missed
interval. Each sample copies the manifest downtime limit. Exceeding it must
block execution until an operator reconciles the source and explicitly
resumes. Do not replace a finite value with `null` unless the customized
provider query proves durable, complete replay from committed state.

## Pinned and follow-path operation

Pinned mode is the production default. Verify the detector and helper hashes,
register the detector digest, and use atomic replacement plus explicit repin
for upgrades. Follow-path is for active development only; byte drift is
expected to make attempts non-committing until the file is stable.

After registration:

- inspect degraded diagnostics without publishing secrets;
- monitor provider quotas and downtime;
- treat kind-policy changes as pause/update/resume operations;
- remove terminal watches when their retained provenance is no longer needed;
  and
- never interpret a detector event as authorization for an automatic action.

## Fixture maintenance

Fixtures are frozen, sanitized provider-shape examples. They use invalid hosts,
example identities, placeholder IDs, and no credentials. Tests are network
free. To refresh a fixture:

1. capture the documented provider API version manually;
2. remove tokens, headers, organizations, repositories, URLs, identities,
   comments, and IDs that are not deliberate examples;
3. stamp top-level `apiVersion` and `capturedAgainst` metadata compatible with
   the manifest;
4. preserve only fields used by normalization or event metadata;
5. add/update terminal, neutral, missing/null, and no-external-activity cases
   when policy changes; generate large-cardinality cases in tests rather than
   checking in unbounded comment bodies; and
6. run the full template conformance test before committing.

## Validation

From the repository root:

```powershell
cargo fmt --check
cargo test -p telex-watcher --test template_conformance
```

The Rust test invokes all six detectors with local fixtures, validates generated
requests/results against the canonical schemas, validates strict manifests,
checks source and pinned digests, proves stable cursors/event IDs and replay
suppression, exercises terminal preflight races and explicit exits, tests
provider transports without network calls, compares emitted kinds with
least-privilege registration policy, checks cadence/downtime/credential modes,
validates fixture API metadata, verifies changelog versions and documentation
links, and rejects unsanitized fixtures, registrations, manifests, and helper
scripts.

## Library releases

See [CHANGELOG.md](CHANGELOG.md). A release changes template versions only when
source behavior or guidance changes. Evidence composition changes also require
an evidence-normalization version change.
