# Trusted-local detector examples

These editable PowerShell detectors implement the Watcher version-1 stdin/stdout contract. They only observe provider or local data: none merges, comments, approves, queues, or otherwise changes a provider.

## Run the fixture smoke test

From this directory's parent repository:

```powershell
pwsh -NoLogo -NoProfile -File .\telex-watcher\examples\detectors\tests\smoke-detectors.ps1
cargo test -p telex-watcher --test example_detectors_protocol
```

The PowerShell smoke invokes every detector against local committed fixtures only. It verifies a version-1-shaped result, deterministic initial event emission, cursor replay suppression, the custom GitHub author/comment policy, and actual `telex-watcher add` acceptance of rewritten local copies of every sample registration. The Rust integration test passes each output through the runtime's version-1 `parse_result` validator. Neither starts Telex or delivers a message.

## Detector protocol

The Watcher writes one JSON request to stdin. Each detector writes exactly one JSON result to stdout and uses exit code zero after successfully communicating that result:

```json
{
  "schemaVersion": 1,
  "outcome": "idle | event | terminal | degraded",
  "nextState": { "cursor": "opaque-sha256" },
  "event": {
    "id": "provider:scope:stable-evidence-hash",
    "kind": "provider.namespaced-kind",
    "subject": "short summary",
    "body": "details",
    "metadata": {}
  }
}
```

`event` is present only for `event` or `terminal`. `degraded` intentionally has neither `event` nor `nextState`; provider/authentication/read errors are represented this way and leave the stored cursor unchanged. A nonzero process exit indicates that the detector itself could not run or write a result, not that a watched PR needs attention.

Each detector hashes normalized observed evidence into an opaque `nextState.cursor`. The first event is suppressed by default to establish a baseline. For the generic GitHub and Azure DevOps PR detectors, `emitInitialSnapshot: true` emits one deterministic read-only snapshot on the first observation even when no attention, ready-to-merge, or terminal condition applies. Those events use `github.pull-request.snapshot` and `azure-devops.pull-request.snapshot`; they describe the observed PR but request no action. Passing the returned state into the same detector with unchanged evidence returns `idle`, preventing replay.

## Examples

| Script | Observation and event condition |
| --- | --- |
| `scripts\gh-pr-detector.ps1` | Uses `gh pr view` review decision, check conclusions, and merge state. Emits attention for requested changes, failed checks, or blocked merge state; emits ready-to-merge for approved, clean PRs; emits a terminal completion observation for closed/merged PRs; and can emit an initial read-only snapshot for live proof. |
| `scripts\gh-pr-external-activity-detector.ps1` | Repository policy example. Ignores the PR author, `selfLogin`, and `ignoredLogins`; wakes only for an external substantive review or issue comment. |
| `scripts\azure-devops-pr-detector.ps1` | Calls Azure DevOps Git REST API `7.1` for the PR and its threads. It observes required-review votes, merge status, completion, and threads. Live use is explicitly opt-in bearer or PAT authentication, never both, and can emit an initial read-only snapshot for live proof. |
| `scripts\file-json-detector.ps1` | Non-PR viability-gate example. Watches a local JSON document and emits when its configured boolean `readyField` is true. |

All scripts accept `fixturePath` (or `inputPath` for the file detector) to avoid network access during tests.

## Registering a copy

1. Copy the appropriate `registrations\*.json` to a private local location.
2. Replace **every** `C:\path\to\...` value: `scriptPath`, the matching item in `command`, and `workingDirectory`. The command is an argv array, not a shell snippet; the runtime requires it to contain the exact registered script path.
3. Replace the placeholder Telex sender/target and provider coordinates. Do not commit the copied file.
4. Choose `follow-path` for editable trusted-local scripts. For immutable scripts, calculate a SHA-256 and use `pinned` plus `scriptDigest`.
5. Add it with `telex-watcher --registry C:\private\watcher.sqlite add --file C:\private\watch.json`.

The JSON samples use the actual `WatchSpec` schema (`id`, `command`, `scriptPath`, `workingDirectory`, `scriptMode`, `sender`, `target`, interval/timeout, attention, disposition, environment allowlist, parameters, and state). Their addresses, paths, and provider coordinates are deliberate placeholders.

### Live GitHub

Set `repository` and `pullRequestNumber`. `gh` must already be available and authenticated in the trusted local environment. `GH_TOKEN` is optional; only add it to `environmentAllowlist` when intentionally using it. `emitInitialSnapshot: true` sends exactly one deterministic `github.pull-request.snapshot` for a neutral initial observation, or the normal state-specific event when it is already event-worthy.

### Live Azure DevOps

Set `organization`, `project`, `repositoryId`, and `pullRequestId`. Select exactly one authentication mode:

- **Bearer (sample default):** Set `allowBearerAuthentication: true`, `allowPatAuthentication: false`, and explicitly allowlist `AZURE_DEVOPS_ACCESS_TOKEN`. A trusted local shell can set it without printing it:

  ```powershell
  $env:AZURE_DEVOPS_ACCESS_TOKEN = az account get-access-token --resource 499b84ac-1321-427f-aa17-267ca6975798 --query accessToken --output tsv
  ```

- **PAT:** Set `allowPatAuthentication: true`, `allowBearerAuthentication: false`, and replace the allowlist entry with `AZURE_DEVOPS_EXT_PAT`.

The script rejects both modes enabled together or neither enabled. It does not use another credential source. Neither token type is included in stdout, metadata, diagnostics, fixtures, or sample registrations. With `emitInitialSnapshot: true`, a neutral first observation emits one `azure-devops.pull-request.snapshot`; an unchanged cursor returns `idle`.

The Azure DevOps sample has no real organization, project, repository, PR, URL, or secret. Its fixture follows the REST `7.1` PR/threads response shape.

## Limits

These are v1 spike examples, not a durable provider integration. `gh` field availability and Azure DevOps permissions can cause `degraded`; inspect Watcher attempts for diagnostics. The Azure example recognizes a conservative subset of votes/merge states, and GitHub review-thread comments not returned by `gh pr view --json comments,reviews` are outside this example's policy. The file example is intentionally local-only; it does not prove Telex delivery.
