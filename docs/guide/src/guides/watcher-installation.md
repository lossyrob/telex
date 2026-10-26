# Install and launch Watcher

Watcher is a separate per-user process on **Windows and Linux**. This procedure
creates a discoverable installation record, then starts the ordinary runtime.
It requires **PowerShell 7.4 or newer on either platform**. It does not install a
service or restart supervisor. macOS bootstrap remains unsupported.

An agent reads an operator-supplied absolute record path, or otherwise exactly
`<config root>/watcher/installation.json`. The config root is `TELEX_HOME` when
set, otherwise `~/.telex`, following Telex's own home rule. There is one default
record per config root; additional installations use explicit paths. Do not
search PATH or other directories for a substitute. If the record is missing,
unreadable, invalid or unsupported, stop and report that bootstrap is needed.

Use the [installation tool](https://github.com/lossyrob/telex/blob/main/telex-watcher/installation.ps1)
from a trusted checkout matching the Watcher build. `create` writes a new record
exclusively; `verify` reads it and checks its authority, paths, selected Telex
build, Watcher hash, backend and authentication features. Neither command starts
the runtime, connects to the backend, changes a profile or upgrades a binary.
The tool prints the record path on stderr and the JSON record on stdout.

## Prepare binaries and the backend

Choose and record the exact trusted binaries before running bootstrap:

- Install or upgrade **Telex** through the existing [release installation and
  upgrade surface](../getting-started/install.md). Record the install root and
  the exact selected versioned executable from `telex --json version --root
  <root>` (`version.install.current_binary`), not the mutable `bin` launcher.
  `copilot stall-detect --help` must succeed for session-group monitoring.
- Build **Watcher** from a trusted pinned checkout and retain its executable
  outside `target` so subsequent builds cannot replace a running binary. Watcher
  is not included in the single-binary Telex release install. Record the source
  commit and build command with your operational evidence.
- Choose an **existing named backend profile** holding the members' addresses.
  Use absolute paths in profile configuration. Bootstrap does not invent a
  default profile or initialize/migrate a backend.
- Choose a new, distinct **sender address** for Watcher's send-only responsibility,
  never a member's receive address. Watcher attaches it as send-only when a watch
  is registered; an empty runtime does not establish sender/backend readiness.

For the existing Entra authentication path, **both packages need `entra`**:

```sh
cargo build --locked --release -p telex -p telex-watcher --features telex/entra,telex-watcher/entra
```

For SQLite or ordinary password/credential-command Postgres profiles, the
default package features suffice. `telex-watcher --version` reports its compiled
backends and Entra support. Bootstrap refuses an older Watcher without this
feature report or either binary missing the profile's required feature.
Installing a local Telex build still uses `telex upgrade --from <binary>`;
building alone does not populate the installed-current layout.

Keep the existing authentication method: password environment reference,
one-shot credential command, or Entra (`cli`, `auto`, or `managed`). See
[Postgres authentication](postgres.md). An Entra CLI profile needs the existing
Azure CLI login and credential environment in the runtime's user context.
Bootstrap grants no access and creates no credentials.

## Create the installation record

The operator approves these values. Paths below are placeholders, not defaults
to infer. Prepare an existing owner-controlled config root and a new private
runtime directory for the registry and logs. Keep the registry distinct from
any existing registry. The installation tool creates only the final record
directory if absent, with owner-only permissions; it never repairs permissions
on an existing directory. On Windows, private authority allows the current user,
SYSTEM and local Administrators; on Linux it requires the current uid and no
group/other access. Symlinks/reparse points and foreign-mutable authority
ancestors are rejected; existing permissions are never repaired. Linux sticky
temporary ancestors are allowed, as in the existing install-authority policy.

In PowerShell 7 on Windows or Linux, use native absolute paths:

```powershell
$ErrorActionPreference = 'Stop'
$tool = '<absolute trusted checkout>/telex-watcher/installation.ps1'
$env:TELEX_HOME = '<absolute existing config root>'
# If this profile uses an explicit configuration file, retain that existing value:
# $env:TELEX_CONFIG = '<absolute existing config.toml>'
$telex = '<absolute selected versioned telex executable>'
$watcher = '<absolute retained telex-watcher executable>'
$installRoot = '<absolute trusted Telex install root>'
$registry = '<absolute NEW runtime directory>/registry.sqlite'
$backendProfile = '<existing named backend profile>'
$sender = '<new distinct Watcher send-only address>'
$environmentNames = @('TELEX_HOME')
if (Test-Path Env:TELEX_CONFIG) { $environmentNames += 'TELEX_CONFIG' }
if ($IsWindows) { $environmentNames += 'LOCALAPPDATA' }
# Add the profile's password variable and any existing credential configuration
# names needed by its command/Entra provider. Never put credential VALUES here.
# For example, add AZURE_CONFIG_DIR if the existing Azure CLI setup uses it.
& $tool -Action create -TelexExecutable $telex -WatcherExecutable $watcher `
    -WatcherRegistry $registry -InstallRoot $installRoot `
    -BackendProfile $backendProfile -Sender $sender `
    -EnvironmentNames $environmentNames
```

By default this writes `$env:TELEX_HOME/watcher/installation.json`. To use another
location, pass `-RecordPath '<absolute record path>'` and give that exact path to
the root agent. Its parent must exist; the final record directory may be new.
A pre-existing record or registry is a refusal, not permission to overwrite.
There is no `--force` path. If preparation fails, repair its reported condition;
do not start the runtime or quietly substitute paths.

### Record schema v1

The record contains only these fields:

| Field | Meaning |
| --- | --- |
| `schemaVersion` | `1`; unknown versions are refused. |
| `configRoot` | Effective absolute Telex home. |
| `telexExecutable` | Exact trusted selected versioned Telex path. |
| `watcherExecutable` | Trusted retained Watcher path. |
| `watcherRegistry` | Absolute registry for this installation. |
| `installRoot` | Trusted installed-current Telex root used by the Application Client. |
| `backendProfile` | Explicit existing profile name. |
| `sender` | Distinct send-only address, not a secret. |
| `pollIntervalSeconds` | Default watch cadence, 1-86400; default 30. |
| `environmentNames` | Required launch/detector environment **names**, never values. |
| `telexBuildId` | Selected Telex's reported build identity. |
| `watcherSha256` | Hash of the retained Watcher binary. |

The record is configuration, not a live-process claim or credential store.
It does not contain tokens, connection strings, password commands or environment
values. Restore any required environment from the existing approved profile and
credential setup; do not save secrets in a launch log. Verify refuses an altered
Watcher or changed Telex selector instead of silently adopting it.

## Verify and start

Restore the launch environment first. In particular, set `TELEX_HOME` to the
record's config root. If used, set `TELEX_CONFIG` and credential environment
variables to their existing approved values. Every `environmentNames` entry
must be available. Do not carry a temporary test `TELEX_DB` into a real install;
the Watcher Application Client and stall detector use the explicit profile,
not ambient database overrides.

Read the installation through `verify`, never just `Get-Content` followed by
execution of unverified record paths:

```powershell
$recordPath = '<absolute operator-supplied record path, or default described above>'
$installation = (& $tool -Action verify -RecordPath $recordPath) |
    ConvertFrom-Json -AsHashtable
$telex = $installation.telexExecutable
$watcher = $installation.watcherExecutable
$registry = $installation.watcherRegistry
$env:TELEX_HOME = $installation.configRoot
$env:TELEX_INSTALL_ROOT = $installation.installRoot
$backendProfile = $installation.backendProfile
$sender = $installation.sender
$pollSeconds = $installation.pollIntervalSeconds
```

Check that `status` reports `runtime-not-running` before launching. A responsive
existing owner is not an invitation to start a replacement. A lost management
reply is not proof the owner is absent; inspect before retrying.

Launch the ordinary process **outside the agent's lifetime** with the host's
detached-process facility. For a Copilot task runner, launch this command once
with `detach: true`, preserving the environment above and recording the returned
task handle and separate stdout/stderr paths in the launch evidence:

```powershell
& $watcher --registry $registry --install-root $installation.installRoot --json run
```

Telex does not detach this process for you. If the host refuses detached launch,
stop and report that limitation rather than leaving an attached shell that dies
with the agent. An operator can use `Start-Process` on Windows as described in
[runtime administration](https://github.com/lossyrob/telex/blob/main/docs/watcher-runtime.md#detached-user-process),
or a persistent user terminal on Linux:

```sh
/absolute/telex-watcher --registry /absolute/registry.sqlite --install-root /absolute/telex-install --json run
```

For an unattended Linux shell launch, use the existing OS `nohup` facility, with
the same environment and explicit paths, and capture its PID and logs:

```sh
nohup /absolute/telex-watcher --registry /absolute/registry.sqlite --install-root /absolute/telex-install --json run \
  </dev/null >/absolute/runtime.stdout.log 2>/absolute/runtime.stderr.log &
watcher_pid=$!
```

Neither launch is a restart supervisor. Keep the record, source/build identity,
launch timestamp, task handle/PID, log paths and returned runtime ID together in
your operational notes. A PID alone is not durable identity.

### Establish readiness

Use the record's same binary and registry:

```powershell
& $watcher --registry $registry --json status
if ($LASTEXITCODE -ne 0) { throw 'Runtime is not ready; inspect its retained startup logs.' }
& $watcher --registry $registry --json list --active
if ($LASTEXITCODE -ne 0) { throw 'Runtime administration failed.' }
```

An initial endpoint race may report `runtime-not-running`; retry a bounded
read-only status check, inspecting the launched task for exit. Require the
expected runtime ID, current heartbeat and `productionReady: true`. Do not
register on a failed startup or claim that process creation proves readiness.
`runtime-failed` requires inspection of the retained stderr/logs; older builds
can omit the underlying cause. Preserve that failure instead of guessing.

Then register an ordinary v2 watch, for example the
[session-group stall watch](copilot-stalls.md). Copy `environmentNames` into its
`environmentAllowlist`; these variables must exist in the **runtime's** launch
environment, not just the registration shell. On Windows, `LOCALAPPDATA` is
needed by detectors that use Telex's local daemon rendezvous; custom
`TELEX_CONFIG` and credential configuration must also be allowlisted when used.
Inspect the registered watch's status, sender readiness, last successful attempt
and any pending operation. Empty-runtime readiness proves none of these.

Root delivery is independent: use either a live Copilot bridge for push, or the
documented `copilot fallback prepare` single-shot pull path when the host cannot
load a bridge. Preparing fallback is not delivery proof; a completed wait needs
inspection, processing and explicit re-arm. Keep the modes and their evidence
distinct. See [Copilot delivery](copilot-push.md).

## Shutdown, rollback and existing installations

Stop exactly this registry owner with its recorded binary:

```powershell
& $watcher --registry $registry --json shutdown
if ($LASTEXITCODE -ne 0) { throw 'Shutdown unconfirmed; inspect the owner before retrying.' }
```

Wait for the recorded task/process to exit, then confirm `status` returns
`runtime-not-running`. Never kill a process by name or reuse an old PID without
checking identity. Shutdown detaches known senders; it does not retire watches,
erase pending receipts, stop the user's Telex daemon or tear down root attendance.
The root owns watch retirement; each receiving participant owns its attendance.

For a failed **new** installation, stop its exact owner, retain registry,
receipts and logs, and keep or remove only the new record once no root uses it.
Do not delete history or reset detector state to obtain a green result.

For an **existing or older** installation, first inventory its selected/previous
Telex version, Watcher path/hash, registry, profile, senders, pending operations,
runtime identity and launch environment. If no record exists, those are operator
inputs; do not adopt the installation by guessing. Preserve its records and
registry. Initial bootstrap deliberately refuses an existing registry.

Create a candidate with an explicit new record path, a retained candidate
Watcher and a new registry. Stop the old owner only with explicit operator
approval. Decide how permanent watch IDs and opaque state move before registering
on the candidate; never copy a live registry or discard unresolved sends. This
procedure does not automate registry/state migration. A new empty registry does
not preserve an old watch's recurrence history.

Telex upgrade/rollback stays on the existing binary surface, with approval and
backend backup where required. An older CLI may be the required invoker for a
legacy installation's upgrade; inspect its output and verify the final selected
binary afterwards. Never bypass manifest admission or ordered daemon replacement.
Rollback does **not** downgrade the backend schema: notably v0.1.2 cannot read a
schema-3 store. Retaining an old executable does not make its rollback safe.
The [operating guide](operating.md) documents those compatibility limits.

After an approved selector change, retain the old record as evidence and create
a reviewed candidate record for the new exact executable; verification of the
old record must fail while its selector differs. Only after explicit cutover,
with the old owner stopped and its evidence preserved, may the operator move the
old default record aside and create a new default. Bootstrap never overwrites it.

## Isolated validation

Any test or field proof uses unique absolute `TELEX_HOME`, `TELEX_DB`,
`TELEX_INSTALL_ROOT` and a temporary `LOCALAPPDATA`, and absolute branch binaries.
On Linux, also use the private short test-home setup in `CONTRIBUTING.md` for
management endpoint tests. Use a dedicated profile with an absolute fixture
database path. Do not touch the operator's installed binaries, daemon or real
directory permissions. Teardown stops only the owners created by the test and
removes only its named disposable paths. Local evidence supports, but does not
replace, Windows/Linux CI or real root-turn delivery evidence.
