#Requires -Version 7.0
# Branch-only #203 diagnostic. Not for main.
# Keeps $Parallel instances of the telex_watcher lib harness running only the
# named-pipe transport tests, under optional CPU contention, with ProcDump -e
# attached to each instance. Records every exit status and test failure.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$EvidenceRoot,
    [Parameter(Mandatory = $true)][int]$Minutes,
    [Parameter(Mandatory = $true)][int]$Parallel,
    [Parameter(Mandatory = $true)][int]$Hogs,
    [Parameter(Mandatory = $true)][ValidateSet("on", "off")][string]$Attach,
    [Parameter(Mandatory = $true)][string]$Filter,
    [Parameter(Mandatory = $true)][string]$Label
)
Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
if ($env:GITHUB_ACTIONS -ne "true" -or $env:RUNNER_ENVIRONMENT -ne "github-hosted") {
    throw "restricted to disposable GitHub-hosted runners"
}

$artifacts = cargo test --workspace --no-run --message-format=json 2>$null |
    ForEach-Object { try { $_ | ConvertFrom-Json } catch { } } |
    Where-Object {
        $_.reason -eq "compiler-artifact" -and $_.target.name -eq "telex_watcher" -and
        ($_.target.kind -contains "lib") -and $_.profile.test -and $_.executable
    }
$exe = @($artifacts | ForEach-Object { $_.executable } | Select-Object -Unique)
if ($exe.Count -ne 1) { throw "expected exactly one telex_watcher lib harness, found $($exe.Count)" }
$exe = $exe[0]
$logs = Join-Path $EvidenceRoot "iterations"
$dumps = Join-Path $EvidenceRoot "dumps"
New-Item -ItemType Directory -Path $logs, $dumps -Force | Out-Null
(Get-Content Cargo.lock -Raw) -match '(?m)^name = "mio"\r?\nversion = "([^"]+)"' | Out-Null
$mio = $Matches[1]
"mode=pipe-stress harness=$exe mio=$mio parallel=$Parallel hogs=$Hogs attach=$Attach filter=$Filter label=$Label" |
    Tee-Object -FilePath (Join-Path $EvidenceRoot "config.txt")

$workdir = (Resolve-Path "telex-watcher").Path
$env:CARGO_MANIFEST_DIR = $workdir
$env:PATH = "$((Resolve-Path 'target\debug\deps').Path);$((Resolve-Path 'target\debug').Path);$env:PATH"

$hogProcs = @()
for ($i = 0; $i -lt $Hogs; $i++) {
    $hogProcs += Start-Process -FilePath pwsh -PassThru -NoNewWindow -ArgumentList '-NoProfile -Command "while ($true) { }"'
}

$summary = Join-Path $EvidenceRoot "iterations.csv"
"iteration,exit,ms,passed,failed" | Set-Content -LiteralPath $summary
$running = @{}
$iteration = 0
$crashes = 0
$testFailures = 0
$deadline = (Get-Date).AddMinutes($Minutes)
$nextReport = 200

function Start-Instance {
    $script:iteration += 1
    $n = $script:iteration
    $out = Join-Path $logs "iter-$n.out.txt"
    $err = Join-Path $logs "iter-$n.err.txt"
    $child = Start-Process -FilePath $exe -WorkingDirectory $workdir -PassThru -NoNewWindow `
        -ArgumentList "$Filter --test-threads 16" -RedirectStandardOutput $out -RedirectStandardError $err
    $null = $child.Handle
    $debugger = $null
    if ($Attach -eq "on") {
        $debugger = Start-Process -FilePath $env:DIAG203_PROCDUMP -PassThru -NoNewWindow `
            -ArgumentList "-accepteula -ma -e $($child.Id) `"$dumps`"" `
            -RedirectStandardOutput (Join-Path $logs "iter-$n.procdump.txt")
    }
    $script:running[$n] = [pscustomobject]@{ Child = $child; Debugger = $debugger; Out = $out; Err = $err; Started = Get-Date }
}

function Complete-Instance($n) {
    $entry = $script:running[$n]
    $script:running.Remove($n)
    $child = $entry.Child
    $child.WaitForExit()
    if ($null -ne $entry.Debugger -and -not $entry.Debugger.WaitForExit(180000)) {
        Stop-Process -Id $entry.Debugger.Id -Force -ErrorAction SilentlyContinue
    }
    $status = [BitConverter]::ToUInt32([BitConverter]::GetBytes([int]$child.ExitCode), 0)
    $ms = [int]((Get-Date) - $entry.Started).TotalMilliseconds
    $text = Get-Content -LiteralPath $entry.Out -Raw -ErrorAction SilentlyContinue
    if ($null -eq $text) { $text = "" }
    $passed = ([regex]::Matches($text, '(?m) \.\.\. ok\s*$')).Count
    $failed = ([regex]::Matches($text, '(?m) \.\.\. FAILED\s*$')).Count
    $hex = "0x{0:x8}" -f $status
    "$n,$hex,$ms,$passed,$failed" | Add-Content -LiteralPath $summary
    if ($status -ge [Convert]::ToUInt32("c0000000", 16)) {
        $script:crashes += 1
        $lastTests = (($text -split "`r?`n") | Where-Object { $_ -match '^test ' } | Select-Object -Last 3) -join ' | '
        Write-Output "CRASH iteration $n exit=$hex ms=$ms ok=$passed last=[$lastTests]"
        Get-Content -LiteralPath $entry.Err -Tail 40 -ErrorAction SilentlyContinue
    } elseif ($status -ne 0) {
        $script:testFailures += 1
        Write-Output "TEST FAILURE iteration $n exit=$hex ok=$passed failed=$failed"
        ($text -split "`r?`n") | Select-String -Pattern 'FAILED|panicked at' | Select-Object -First 8 |
            ForEach-Object { "  $($_.Line)" }
    } else {
        Remove-Item -LiteralPath $entry.Out, $entry.Err -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath (Join-Path $logs "iter-$n.procdump.txt") -ErrorAction SilentlyContinue
    }
}

try {
    while ((Get-Date) -lt $deadline) {
        while ($running.Count -lt $Parallel) { Start-Instance }
        foreach ($n in @($running.Keys)) {
            if ($running[$n].Child.HasExited) { Complete-Instance $n }
        }
        if ($iteration -ge $nextReport) {
            Write-Output "progress iterations=$iteration crashes=$crashes testFailures=$testFailures"
            $nextReport += 200
        }
        Start-Sleep -Milliseconds 50
    }
    foreach ($n in @($running.Keys)) { Complete-Instance $n }
} finally {
    foreach ($h in $hogProcs) { Stop-Process -Id $h.Id -Force -ErrorAction SilentlyContinue }
}

$completed = $iteration
$line = "label=$Label mode=pipe-stress mio=$mio parallel=$Parallel hogs=$Hogs attach=$Attach iterations=$completed crashes=$crashes testFailures=$testFailures"
$line | Tee-Object -FilePath (Join-Path $EvidenceRoot "result.txt")
"### $line" >> $env:GITHUB_STEP_SUMMARY
if ($crashes -gt 0) {
    Copy-Item -LiteralPath $exe -Destination $EvidenceRoot
    $pdb = [IO.Path]::ChangeExtension($exe, ".pdb")
    if (Test-Path -LiteralPath $pdb) { Copy-Item -LiteralPath $pdb -Destination $EvidenceRoot }
}
