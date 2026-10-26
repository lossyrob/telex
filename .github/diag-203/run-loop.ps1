#Requires -Version 7.0
# Branch-only #203 diagnostic. Not for main.
# Repeatedly runs the exact telex_watcher lib test harness that cargo built.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$EvidenceRoot,
    [Parameter(Mandatory = $true)][int]$Minutes,
    [Parameter(Mandatory = $true)][ValidateSet("on", "off")][string]$PageHeap,
    [Parameter(Mandatory = $true)][string]$Label,
    [Parameter(Mandatory = $true)][ValidateSet("on", "off")][string]$Attach
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
$exeName = Split-Path -Leaf $exe
$logs = Join-Path $EvidenceRoot "iterations"
$dumps = Join-Path $EvidenceRoot "dumps"
New-Item -ItemType Directory -Path $logs -Force | Out-Null
"harness=$exe" | Tee-Object -FilePath (Join-Path $EvidenceRoot "harness.txt")
(Get-Content Cargo.lock -Raw) -match '(?m)^name = "mio"\r?\nversion = "([^"]+)"' | Out-Null
$mio = $Matches[1]
"mio=$mio pageheap=$PageHeap attach=$Attach label=$Label" | Tee-Object -FilePath (Join-Path $EvidenceRoot "config.txt")

$ifeo = "HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Image File Execution Options\$exeName"
if ($PageHeap -eq "on") {
    New-Item -Path $ifeo -Force | Out-Null
    New-ItemProperty -Path $ifeo -Name GlobalFlag -Value 0x02000000 -PropertyType DWord -Force | Out-Null
    New-ItemProperty -Path $ifeo -Name PageHeapFlags -Value 0x3 -PropertyType DWord -Force | Out-Null
}

$workdir = (Resolve-Path "telex-watcher").Path
$env:CARGO_MANIFEST_DIR = $workdir
$env:PATH = "$((Resolve-Path 'target\debug\deps').Path);$((Resolve-Path 'target\debug').Path);$env:PATH"
$deadline = (Get-Date).AddMinutes($Minutes)
$summary = Join-Path $EvidenceRoot "iterations.csv"
"iteration,exit,seconds,passed,failed,lastline" | Set-Content -LiteralPath $summary
$iteration = 0
$crashes = 0
$testFailures = 0
try {
    while ((Get-Date) -lt $deadline) {
        $iteration += 1
        $out = Join-Path $logs "iter-$iteration.out.txt"
        $err = Join-Path $logs "iter-$iteration.err.txt"
        $started = Get-Date
        $child = Start-Process -FilePath $exe -WorkingDirectory $workdir -PassThru -NoNewWindow `
            -RedirectStandardOutput $out -RedirectStandardError $err
        $handle = $child.Handle
        $debugger = $null
        if ($Attach -eq "on") {
            $debugger = Start-Process -FilePath $env:DIAG203_PROCDUMP -PassThru -NoNewWindow `
                -ArgumentList "-accepteula -ma -e $($child.Id) `"$dumps`"" `
                -RedirectStandardOutput (Join-Path $logs "iter-$iteration.procdump.txt")
        }
        if (-not $child.WaitForExit(1200000)) {
            Stop-Process -Id $child.Id -Force
            $child.WaitForExit()
        }
        $child.Refresh()
        if ($null -ne $debugger -and -not $debugger.WaitForExit(180000)) {
            Stop-Process -Id $debugger.Id -Force
        }
        $status = [BitConverter]::ToUInt32([BitConverter]::GetBytes([int]$child.ExitCode), 0)
        $seconds = [int]((Get-Date) - $started).TotalSeconds
        $text = Get-Content -LiteralPath $out -Raw -ErrorAction SilentlyContinue
        if ($null -eq $text) { $text = "" }
        $passed = ([regex]::Matches($text, '(?m) \.\.\. ok\s*$')).Count
        $failed = ([regex]::Matches($text, '(?m) \.\.\. FAILED\s*$')).Count
        $last = (($text -split "`r?`n") | Where-Object { $_ -match '^test ' } | Select-Object -Last 1) -replace ',', ';'
        $hex = "0x{0:x8}" -f $status
        "$iteration,$hex,$seconds,$passed,$failed,$last" | Add-Content -LiteralPath $summary
        Write-Output "iteration $iteration exit=$hex seconds=$seconds ok=$passed failed=$failed last=[$last]"
        if ($status -ge [Convert]::ToUInt32("c0000000", 16)) {
            $crashes += 1
            Get-Content -LiteralPath $err -Tail 40 -ErrorAction SilentlyContinue
        } elseif ($status -ne 0) {
            $testFailures += 1
        } else {
            Remove-Item -LiteralPath $out, $err -ErrorAction SilentlyContinue
            Remove-Item -LiteralPath (Join-Path $logs "iter-$iteration.procdump.txt") -ErrorAction SilentlyContinue
        }
    }
} finally {
    if ($PageHeap -eq "on") { Remove-Item -Path $ifeo -Recurse -Force -ErrorAction SilentlyContinue }
}

$line = "label=$Label mio=$mio pageheap=$PageHeap attach=$Attach iterations=$iteration crashes=$crashes testFailures=$testFailures"
$line | Tee-Object -FilePath (Join-Path $EvidenceRoot "result.txt")
"### $line" >> $env:GITHUB_STEP_SUMMARY
"DIAG203_HARNESS=$exe" >> $env:GITHUB_ENV
if ($crashes -gt 0) {
    Copy-Item -LiteralPath $exe -Destination $EvidenceRoot
    $pdb = [IO.Path]::ChangeExtension($exe, ".pdb")
    if (Test-Path -LiteralPath $pdb) { Copy-Item -LiteralPath $pdb -Destination $EvidenceRoot }
}

