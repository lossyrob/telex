# Operator Station spike: Windows builder walkthrough

This walkthrough exercises the issue #93 loop on one fresh local SQLite store:

`worker -> attention:rob -> operator agent -> operator:rob -> human -> operator agent -> worker`

The assignment, Station, worker, and harness must all use the same explicit
`TELEX_OPERATOR_SPIKE_DB`. Do not use a configured/default Telex store.

## 1. Prerequisites and build

Use Windows with:

- current `telex` on `PATH`;
- PowerShell 7 or Windows PowerShell 5.1;
- Node.js/npm;
- Python 3 (only for reproducible read-only Action Center evidence extraction);
- Rust stable plus the MSVC build tools;
- WebView2 and the normal Tauri 2 Windows prerequisites;
- Copilot CLI with the Telex plugin and Copilot Extensions enabled.

From the repository root:

```powershell
Get-Command telex -ErrorAction Stop

Push-Location .\spike\operator-station
npm install
npm test
npm run build
cargo test --manifest-path .\src-tauri\Cargo.toml
Pop-Location
```

If the Station package is still being assembled in a shared worktree, wait
until `src-tauri\Cargo.toml` and the package README exist before running the
Tauri commands. The harness and assignment can be parser-checked independently.

## 2. Select and initialize a fresh store

Run this setup in each terminal that will participate. Choose the path once;
the first terminal creates the directory and store. Do not print the raw path
in captured evidence.

```powershell
$runName = "operator-loop-" + (Get-Date -Format "yyyyMMdd-HHmmss")
$runRoot = Join-Path $PWD ".local\$runName"
New-Item -ItemType Directory -Path $runRoot -Force | Out-Null
$env:TELEX_OPERATOR_SPIKE_DB = Join-Path $runRoot "operator-loop.db"

# A store-scoped status call creates the selected SQLite schema.
telex status --db $env:TELEX_OPERATOR_SPIKE_DB --json | Out-Null

$store = & .\spike\operator-station\harness\Get-OperatorSpikeStoreFingerprint.ps1 `
    -DatabasePath $env:TELEX_OPERATOR_SPIKE_DB `
    -IncludeCanonicalPath
$env:TELEX_OPERATOR_SPIKE_DB = $store.CanonicalPath
$store.Fingerprint

telex --db $env:TELEX_OPERATOR_SPIKE_DB --version
telex copilot skill
```

The displayed value must be `sha256:` plus 64 lowercase hexadecimal
characters. Pass the canonical variable value to the other terminals without
recording it in evidence. Every Telex command below explicitly supplies
`--db $env:TELEX_OPERATOR_SPIKE_DB`.

Run the helper self-check once:

```powershell
& .\spike\operator-station\harness\Test-OperatorSpikeStoreFingerprint.ps1
```

To reproduce the Action Center evidence after a live toast:

```powershell
$sourceHead = git rev-parse HEAD
& .\spike\operator-station\harness\Get-OperatorSpikeToastRecord.ps1 `
    -SourceHead $sourceHead `
    -OutputPath .\spike\operator-station\evidence\windows-action-center-record.json
```

## 3. Launch the Station

In Terminal 1, set the same database variable, then:

```powershell
$env:TELEX_ADDRESS = "operator:rob"
Push-Location .\spike\operator-station
npm run tauri dev
```

Keep this foreground process visible. Do not background it. Confirm the
Station feed opens for `operator:rob` and shows its store fingerprint, not the
database path.

## 4. Launch the real operator-agent session

In Terminal 2, set the same database variable and start Copilot CLI in the
repository. Assign it:

> Follow `spike/operator-station/OPERATOR-AGENT.md`, assignment version 1.
> Attend `attention:rob` on the explicit isolated store and use the Copilot push
> bridge.

The agent must run:

```powershell
telex copilot attach --db $env:TELEX_OPERATOR_SPIKE_DB `
    --address attention:rob `
    --copilot-bridge `
    --description "Operator agent assignment v1"
```

It must then invoke `extensions_reload`. Confirm Station occupancy shows both
`attention:rob` and `operator:rob` attended.

Copilot CLI sets `COPILOT_AGENT_SESSION_ID` for the operator-agent session. The
assignment preflight refuses to start if that identity is absent.

## 5. Send a worker decision

In Terminal 3, set the same database variable and create a worker identity:

```powershell
$workerSession = "operator-loop-worker-" + [guid]::NewGuid().ToString("N")
telex attach --db $env:TELEX_OPERATOR_SPIKE_DB `
    --address worker:builder `
    --session $workerSession `
    --description "Issue 93 walkthrough worker" --json | Out-Null

$workerBody = Join-Path $PWD ".local\worker-decision.txt"
[System.IO.File]::WriteAllText(
    $workerBody,
    "Choose Tuesday or Thursday for rollout. Both are technically viable; scheduling judgment is required.",
    [System.Text.UTF8Encoding]::new($false)
)

$rawReceipt = telex send --db $env:TELEX_OPERATOR_SPIKE_DB `
    --from worker:builder `
    --to attention:rob `
    --session $workerSession `
    --subject "Rollout-window decision" `
    --body-file $workerBody `
    --kind decision-request `
    --attention next-checkpoint `
    --requires-disposition --json |
    ConvertFrom-Json
```

Record only `$rawReceipt.id` and `$rawReceipt.thread_id`.

## 6. Observe escalation, toast, and feed

The operator agent should dedupe the pushed message by ID, decide that human
judgment is required, send a new
`operator-station-spike.escalation` from `attention:rob` to `operator:rob`, and
immediately mark the raw message `escalated`.

For an `interrupt` escalation requiring disposition, verify:

1. a Windows toast is attempted;
2. the Station feed shows the escalation even if toast registration fails;
3. sender, kind, attention, subject/body, and disposition requirement are
   visible;
4. the source card shows the captured worker fields and the full matching store
   fingerprint;
5. raw metadata remains inspectable.

Record a toast failure as a visible spike result; do not treat it as permission
to hide the feed message.

## 7. Reply as the human and route back

Select the escalation in Station and reply:

> Use Thursday. Avoid the Tuesday dependency freeze and notify the release
> owner before scheduling.

Station sends the reply from `operator:rob` in the mediated thread. The
operator agent receives it on `attention:rob`, validates the exact v1
experimental envelope and fingerprint, then uses `telex reply` against the
original raw message ID. It closes the raw obligation only after that route
succeeds. The Station marks human replies disposition-required; the operator
must route and verify the receipt before acking and terminally handling that
human-reply obligation.

The scripted smoke harness also exercises a failure boundary: it detaches the
operator after the human reply exists but before route-back, reattaches the same
operator session, proves the unacked human reply remains actionable, then routes,
acks and handles the human reply, and closes the raw request in that order.

In Terminal 3, inspect the worker inbox:

```powershell
telex inbox --db $env:TELEX_OPERATOR_SPIKE_DB `
    --address worker:builder --all --limit 20 --json
```

The routed outcome must be from `attention:rob`, not `operator:rob` and not the
worker.

## 8. Inspect threads, sources, and dispositions

Capture the mediated escalation ID from Station or its feed, then:

```powershell
$raw = telex read --db $env:TELEX_OPERATOR_SPIKE_DB `
    --address attention:rob --id $rawReceipt.id --full --json |
    ConvertFrom-Json

$mediated = telex read --db $env:TELEX_OPERATOR_SPIKE_DB `
    --address operator:rob --id $mediatedEscalationId --full --json |
    ConvertFrom-Json

telex export --db $env:TELEX_OPERATOR_SPIKE_DB `
    --address attention:rob --thread $rawReceipt.thread_id --since 0 --json

telex export --db $env:TELEX_OPERATOR_SPIKE_DB `
    --address operator:rob --thread $mediated.message.thread_id --since 0 --json
```

Verify:

- raw and mediated thread IDs differ;
- the human reply has the mediated thread ID;
- the routed result has the raw thread ID;
- escalation metadata contains only the recognized experimental v1 envelope,
  the complete safe `sourceMessages` fields, full fingerprint, assignment
  version, and model ID;
- unknown namespaces remain raw and are not interpreted;
- raw disposition history contains `escalated` followed later by `closed`.

Also exercise the other assignment branches with separate raw messages:
routine resolution ends `handled`; a missing-evidence clarification remains
`deferred` until evidence arrives.

## 9. Station restart

Stop Terminal 1 with Ctrl+C and wait for the foreground Tauri process to exit.
Do not kill processes by name. Relaunch it:

```powershell
Push-Location .\spike\operator-station
npm run tauri dev
```

Verify the feed and complete mediated thread rebuild from Telex history, the
same store fingerprint is shown, unresolved obligations remain visible, and
startup backfill does not emit a duplicate toast.

## 10. Operator-agent detach occupancy warning

In the operator-agent Copilot session:

```powershell
telex copilot detach --db $env:TELEX_OPERATOR_SPIKE_DB `
    --address attention:rob
```

Invoke `extensions_reload`. Within the Station occupancy refresh interval,
verify a visible warning that `attention:rob` is unattended. Messages sent
during this interval must remain durable.

Before continuing, reattach and reload:

```powershell
telex copilot attach --db $env:TELEX_OPERATOR_SPIKE_DB `
    --address attention:rob `
    --copilot-bridge `
    --description "Operator agent assignment v1"
```

Do not leave the operator-agent role detached while interpreting the demo.

## 11. Bounded scripted smoke harness

The stand-in supplements the real prompt-driven run; it does not replace it.
Use another fresh database path:

```powershell
$smokeRoot = Join-Path $PWD (".local\smoke-" + (Get-Date -Format "yyyyMMdd-HHmmss"))
New-Item -ItemType Directory -Path $smokeRoot -Force | Out-Null
$env:TELEX_OPERATOR_SPIKE_DB = Join-Path $smokeRoot "smoke.db"

& .\spike\operator-station\harness\Invoke-OperatorLoopSmoke.ps1 `
    -EvidencePath (Join-Path $smokeRoot "smoke-evidence.json")
```

The harness refuses an existing database, runs no waiter or infinite loop,
detaches all three temporary identities, and outputs evidence containing only
the safe fingerprint, fixed addresses, IDs, thread IDs, and assertions.

Parser-check all scripts:

```powershell
Get-ChildItem .\spike\operator-station\harness\*.ps1 | ForEach-Object {
    $tokens = $null
    $errors = $null
    [void][System.Management.Automation.Language.Parser]::ParseFile(
        $_.FullName,
        [ref]$tokens,
        [ref]$errors
    )
    if ($errors.Count) { throw "$($_.Name): $($errors -join '; ')" }
}
```

## 12. Optional >1,000-message restart stress

This is separate because starting more than 1,050 CLI processes may take
several minutes. Select another fresh store and run:

```powershell
$stressRoot = Join-Path $PWD (".local\stress-" + (Get-Date -Format "yyyyMMdd-HHmmss"))
New-Item -ItemType Directory -Path $stressRoot -Force | Out-Null
$env:TELEX_OPERATOR_SPIKE_DB = Join-Path $stressRoot "stress.db"

& .\spike\operator-station\harness\Invoke-OperatorLoopSmoke.ps1 `
    -Stress `
    -StressCount 1055 `
    -EvidencePath (Join-Path $stressRoot "stress-evidence.json")
```

The optional path leaves an older unresolved sentinel behind more than 1,050
newer FYI messages, simulates Station detach/reattach, verifies the sentinel is
outside `inbox --all --limit 200`, and verifies full export still recovers it.
Launch Station on that retained isolated store and restart it once more to
observe the UI projection and restart-quiet toast behavior.

## 13. End the run

Detach the worker and operator agent with their explicit database:

```powershell
telex detach --db $env:TELEX_OPERATOR_SPIKE_DB `
    --address worker:builder --session $workerSession --json

telex copilot detach --db $env:TELEX_OPERATOR_SPIKE_DB `
    --address attention:rob
```

Stop Station normally with Ctrl+C. No walkthrough step starts an infinite
waiter or unmanaged background process.

To inspect accumulated local scope records, run
`harness\Reset-OperatorSpikeLocalScope.ps1`. Add `-Apply -Confirm` only when
intentionally clearing spike-local session/high-water state.

## Builder viability is a separate gate

Completing this walkthrough demonstrates that the experimental loop can run.
It does **not** self-pass the separate builder viability gate, approve a
production Station contract, or promote the experimental namespace. That
judgment must be recorded by the follow-on gate using its own evidence and
review.
