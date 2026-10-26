#Requires -Version 7.0
# Branch-only #203 diagnostic. Not for main.
# Repeats main's exact Windows `cargo test --workspace --verbose` step and
# optionally attaches ProcDump -e to every telex_watcher test harness process.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$EvidenceRoot,
    [Parameter(Mandatory = $true)][int]$Minutes,
    [Parameter(Mandatory = $true)][ValidateSet("on", "off")][string]$Attach,
    [Parameter(Mandatory = $true)][string]$Label
)
Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
if ($env:GITHUB_ACTIONS -ne "true" -or $env:RUNNER_ENVIRONMENT -ne "github-hosted") {
    throw "restricted to disposable GitHub-hosted runners"
}
$logs = Join-Path $EvidenceRoot "iterations"
$dumps = Join-Path $EvidenceRoot "dumps"
New-Item -ItemType Directory -Path $logs, $dumps -Force | Out-Null
(Get-Content Cargo.lock -Raw) -match '(?m)^name = "mio"\r?\nversion = "([^"]+)"' | Out-Null
$mio = $Matches[1]
"mode=workspace mio=$mio attach=$Attach label=$Label" | Tee-Object -FilePath (Join-Path $EvidenceRoot "config.txt")

$attacher = $null
if ($Attach -eq "on") {
    $script = Join-Path $env:RUNNER_TEMP "diag203-attacher.ps1"
    @'
param($ProcDump, $Dumps, $Log)
$seen = @{}
while ($true) {
    foreach ($p in @(Get-Process -Name "telex_watcher-*" -ErrorAction SilentlyContinue)) {
        if (-not $seen.ContainsKey($p.Id)) {
            $seen[$p.Id] = $true
            $others = (@(Get-Process -Name "telex*" -ErrorAction SilentlyContinue) |
                Where-Object { $_.Id -ne $p.Id } | ForEach-Object { "$($_.ProcessName):$($_.Id)" }) -join ' '
            "$(Get-Date -Format o) attach pid=$($p.Id) name=$($p.ProcessName) others=[$others]" | Add-Content -LiteralPath $Log
            Start-Process -FilePath $ProcDump -NoNewWindow -ArgumentList "-accepteula -ma -e $($p.Id) `"$Dumps`"" `
                -RedirectStandardOutput (Join-Path (Split-Path $Log) "procdump-$($p.Id).txt") | Out-Null
        }
    }
    Start-Sleep -Milliseconds 100
}
'@ | Set-Content -LiteralPath $script -Encoding utf8
    $attacher = Start-Process -FilePath pwsh -PassThru -NoNewWindow -ArgumentList `
        "-NoProfile -File `"$script`" -ProcDump `"$env:DIAG203_PROCDUMP`" -Dumps `"$dumps`" -Log `"$(Join-Path $logs 'attacher.txt')`""
}

$deadline = (Get-Date).AddMinutes($Minutes)
$summary = Join-Path $EvidenceRoot "iterations.csv"
"iteration,exit,seconds,watcherCrash,otherFailure" | Set-Content -LiteralPath $summary
$iteration = 0
$crashes = 0
$otherFailures = 0
try {
    while ((Get-Date) -lt $deadline) {
        $iteration += 1
        $out = Join-Path $logs "iter-$iteration.txt"
        $started = Get-Date
        & cargo test --workspace --verbose *> $out
        $code = $LASTEXITCODE
        $seconds = [int]((Get-Date) - $started).TotalSeconds
        $text = Get-Content -LiteralPath $out -Raw
        $watcherCrash = $text -match 'telex_watcher-[0-9a-f]+\.exe` \(exit code: 0xc'
        $other = ($code -ne 0) -and -not $watcherCrash
        "$iteration,$code,$seconds,$watcherCrash,$other" | Add-Content -LiteralPath $summary
        Write-Output "iteration $iteration exit=$code seconds=$seconds watcherCrash=$watcherCrash otherFailure=$other"
        if ($watcherCrash) {
            $crashes += 1
            ($text -split "`r?`n") | Select-String -Pattern "exit code: 0xc|didn't exit|panicked at" |
                ForEach-Object { $_.Line }
        } elseif ($other) {
            $otherFailures += 1
            ($text -split "`r?`n") | Select-String -Pattern "exit code: |didn't exit|panicked at|FAILED" |
                Select-Object -First 10 | ForEach-Object { $_.Line }
        } else {
            Remove-Item -LiteralPath $out
        }
    }
} finally {
    if ($null -ne $attacher) { Stop-Process -Id $attacher.Id -Force -ErrorAction SilentlyContinue }
}
$line = "label=$Label mode=workspace mio=$mio attach=$Attach iterations=$iteration watcherCrashes=$crashes otherFailures=$otherFailures"
$line | Tee-Object -FilePath (Join-Path $EvidenceRoot "result.txt")
"### $line" >> $env:GITHUB_STEP_SUMMARY
if ($crashes -gt 0) {
    Get-ChildItem target\debug\deps -Filter "telex_watcher-*" -File |
        Where-Object { $_.Extension -in ".exe", ".pdb" } |
        Copy-Item -Destination $EvidenceRoot
}
