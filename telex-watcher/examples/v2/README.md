# Optional Watcher v2 examples

Copy one script, edit its observation policy, and supply a command plus Watcher
policy. These standalone PowerShell 7.4+ scripts use the public
[v2 request](../../../docs/design/schemas/watcher-detector-request-v2.schema.json)
and [v2 result](../../../docs/design/schemas/watcher-detector-result-v2.schema.json)
contracts. There is no shared module, manifest, pinning step, or required test
framework to adopt.

This pack can be exercised without a Watcher runtime. The existing spike runtime
and [older examples](../detectors/README.md) speak v1 and cannot run these v2
registrations. Schema and offline checks establish example behavior, not durable
delivery or production operational acceptance.

## Choose an observation

| Script | Configure in `parameters` | Event and local replay choice |
| --- | --- | --- |
| [github-pr.ps1](github-pr.ps1) | `repository`, `pullRequestNumber` | Initial state/head/draft snapshot, then changes only. Closed or merged PRs return `terminal`. |
| [azure-build.ps1](azure-build.ps1) | `organization`, `project`, `buildId` | `idle` while a build runs; `terminal` with its observed result when completed, including failure or cancellation. |
| [http-json.ps1](http-json.ps1) | HTTPS `url` | Reads `{"ready":true}` and emits on **every** ready observation, even with identical content. False is `idle`. |
| [file-json.ps1](file-json.ps1) | Local JSON `path` | Emits on initial true or a false-to-true `ready` edge; remembers the last successfully evaluated boolean. |
| [command.ps1](command.ps1) | `command` argv array | Observational native command: exit 0 means ready, 1 means not ready, anything else is an execution failure. Same edge policy as file. |

All five only observe. The GitHub example does not evaluate CI, reviews, inline
review threads, or merge readiness. Azure observes one build, not a branch's
latest run or PR approval policy. The scripts sample current state rather than
replaying provider history; transitions between polls or during downtime can be
missed. Edit that policy locally if your source needs a cursor or history query.

## Copy and configure

Copy the chosen `.ps1` and its matching JSON from [registrations](registrations)
to a private local directory. The command example also needs an observational
program; [fixtures/condition.ps1](fixtures/condition.ps1) is a harmless always-ready
demonstration to replace.

In the registration, replace the script path, provider coordinates, backend
profile, sender, target, and permanent watch ID. Paths in the samples use
Windows syntax; use absolute native paths on your host, including an absolute
PowerShell executable if you do not want PATH lookup. Each command is argv,
not a shell string. `workingDirectory` is optional; when omitted the runtime
captures the current directory at registration time. Choose a new watch ID when changing
backend or route; removed IDs cannot be reused.

The samples use only the
[v2 registration schema](../../../docs/design/schemas/watcher-registration-v2.schema.json).
Cadence and timeout are Watcher policy. When a v2 runtime is available, submit
the edited registration through its documented registration interface; the v1
`add` command is not a v2 compatibility path.

### Credentials and execution bounds

- **GitHub:** Install `gh` and authenticate for read access to the repository.
  The sample allowlists `GH_TOKEN` and `GH_CONFIG_DIR`; keep only what your
  environment needs. Stored `gh` credentials may also be available through the
  controlled user-home baseline. This script calls only `gh pr view`; it has
  no internal retry or timeout. The registration's 30-second timeout bounds it
  under Watcher.
- **Azure DevOps:** Supply a bearer access token for Azure DevOps in
  `TELEX_EXAMPLE_ADO_TOKEN`, with permission to read the build. The registration
  allowlists that name. Acquire/refresh the token outside the detector; no PAT
  or alternate credential fallback is implemented. The script uses REST `7.1`,
  a 10-second HTTP timeout, and no redirects or retries.
- **HTTP/JSON:** The sample reads an unauthenticated HTTPS endpoint, with a
  10-second HTTP timeout and no redirects or retries. Credentials in URLs are
  rejected. Add any authenticated behavior explicitly in your private copy.
- **File:** Read access to the configured file is sufficient. Missing,
  unreadable, or malformed files fail rather than silently becoming not ready.
  Writers should replace a complete JSON file atomically.
- **Command:** Use a trusted, read-only native executable or an explicit
  interpreter plus script. Stdout is discarded; stderr remains diagnostic.
  Arguments are passed directly, without implied shell evaluation. The command
  inherits the detector's controlled environment; allowlist any needed names in
  registration. Watcher's timeout covers the detector and its child scope.
  Manual runs have no Watcher timeout, so use a known bounded command.

Never store credentials in parameters, state, fixtures, or registrations.
Environment allowlisting is not a sandbox: scripts retain the user's filesystem
and process authority. Treat stderr as potentially sensitive provider diagnostic
text and inspect it locally before sharing it.

## Run an example without Watcher

From the repository root, run this in PowerShell. It observes only committed
local fixtures and prints one v2 event:

```powershell
$root = (Resolve-Path .\telex-watcher\examples\v2).Path
$request = Get-Content (Join-Path $root 'fixtures\request.json') -Raw | ConvertFrom-Json -AsHashtable
$request.watch.parameters = @{ path = Join-Path $root 'fixtures\ready.json' }
$request | ConvertTo-Json -Depth 10 -Compress |
    & pwsh -NoLogo -NoProfile -File (Join-Path $root 'file-json.ps1')
```

For the other examples, keep the same request and substitute:

| Script | Offline `parameters` |
| --- | --- |
| `github-pr.ps1` | `@{ fixturePath = Join-Path $root 'fixtures\github-open.json' }` |
| `azure-build.ps1` | `@{ fixturePath = Join-Path $root 'fixtures\azure-completed.json'; buildId = 42 }` |
| `http-json.ps1` | `@{ fixturePath = Join-Path $root 'fixtures\ready.json' }` |
| `command.ps1` | `@{ command = @((Get-Command pwsh).Source, '-NoProfile', '-File', (Join-Path $root 'fixtures\condition.ps1')) }` |

For live observation, remove `fixturePath` and use the matching registration's
parameters. Fixture input deliberately bypasses network calls and credentials;
do not leave it in a live watch.

Successful execution exits zero and writes one JSON result. Invalid data,
authentication/transport errors, missing commands, and unexpected command exits
produce nonzero exit with stderr and no result. These are execution failures,
not an `idle` evaluation or a `degraded` result. Watcher retains state and
applies its failure/backoff policy. A completed failed Azure build is different:
it is a successful observation with a terminal event describing that failure.

## State and receipt semantics

Scripts never write a cursor file or call Telex. Their `nextState` only proposes
state for Watcher to commit. For event-producing results, including terminal
events, Watcher commits after **durable acceptance**, not merely after the
script exits or a recipient becomes occupied. The runtime owns routing,
permanent watch identity, event sequence/ID, and exact send-operation identity
through the shared Application Client.

GitHub stores the normalized snapshot in opaque state. File and command store
the last boolean. False evaluations may commit immediately as `idle`; a true
edge remains re-emittable from prior state until its event is accepted. HTTP
and Azure omit `nextState`, preserving prior state. Passing returned state back
by hand only simulates a successful commit; it does not prove delivery.

HTTP demonstrates a different valid policy: after each accepted event, the next
true observation is a new occurrence with a new runtime identity, even though
its content is identical. The other scripts' repeat suppression is local
example policy, never a product-wide deduplication rule. If you change a source
or the meaning of a script's state, explicitly decide whether to reset it to
`null` rather than carrying an unrelated cursor forward.

When acceptance is uncertain, the runtime retains the staged operation and
blocks new detection while reconciling that exact operation in the exact
store. Only authoritative `not-recorded` permits an identity-checkable retry
of the same operation; uncertainty or a retention boundary does not. The
examples add no delivery fallback. See the
[Watcher contract](../../../docs/design/watcher.md) for the complete transaction.

## Optional checks

```powershell
pwsh -NoLogo -NoProfile -File .\telex-watcher\examples\v2\tests\check-examples.ps1
```

The checks use PowerShell's `Test-Json` against the repository's canonical v2
schemas, invoke every script, and exercise initial observations, committed
replay, uncommitted re-emission, later edges, identical recurrence, and errors.
In-process provider stubs check read-only call arguments, timeouts, redirect
policy, credential selection, and failure handling. They make no provider calls
and start no Telex daemon. The subprocess harness has a 20-second timeout.
Linux and Windows CI run the same checks. Real provider availability,
permissions, TLS behavior, and runtime delivery remain separate validation.

PR [#131](https://github.com/lossyrob/telex/pull/131) supplied useful lessons:
check JSON booleans by type, distinguish command failure from a false condition,
and test provider transports offline. This pack keeps those lessons without
its mandatory template, provenance, kind-policy, preflight, or downtime machinery.
