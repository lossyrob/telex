#Requires -Version 7.0
Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

. (Join-Path $PSScriptRoot "released_v021_process_helpers.ps1")

$root = Join-Path $env:RUNNER_TEMP ("telex-v021-process-test-" + [guid]::NewGuid().ToString("N"))
$logs = Join-Path $root "logs"
$commands = Join-Path $root "commands.txt"
New-Item -ItemType Directory -Path $logs -Force | Out-Null
[IO.File]::WriteAllText($commands, "", [Text.UTF8Encoding]::new($false))

try {
    $heldPipeScript = @'
$child = Start-Process -FilePath (Join-Path $PSHOME 'pwsh.exe') `
    -ArgumentList '-NoProfile', '-NonInteractive', '-Command', 'Start-Sleep -Seconds 3' `
    -NoNewWindow -PassThru
Write-Output 'held-pipe-parent-complete'
'@
    $started = [Diagnostics.Stopwatch]::StartNew()
    $heldPipe = Invoke-BoundedLoggedProcess `
        -LogsDirectory $logs `
        -CommandsFile $commands `
        -Name "held-pipe" `
        -Executable (Join-Path $PSHOME "pwsh.exe") `
        -Arguments @("-NoProfile", "-NonInteractive", "-Command", $heldPipeScript) `
        -TimeoutSeconds 5
    $started.Stop()
    if ($heldPipe.ExitCode -ne 0 -or
        $heldPipe.Stdout -notmatch "held-pipe-parent-complete" -or
        $started.Elapsed -ge [TimeSpan]::FromSeconds(2.5)) {
        throw "direct file capture waited for a descendant-held output handle"
    }
    Start-Sleep -Seconds 4

    $timeoutObserved = $false
    try {
        Invoke-BoundedLoggedProcess `
            -LogsDirectory $logs `
            -CommandsFile $commands `
            -Name "direct-timeout" `
            -Executable (Join-Path $PSHOME "pwsh.exe") `
            -Arguments @(
                "-NoProfile", "-NonInteractive", "-Command", "Start-Sleep -Seconds 30"
            ) `
            -TimeoutSeconds 3 | Out-Null
    } catch {
        $timeoutObserved = $_.Exception.Message -match "process/capture deadline"
    }
    if (-not $timeoutObserved) {
        throw "direct process timeout was not reported"
    }
    $timeoutMetadata =
        Get-Content -Raw (Join-Path $logs "direct-timeout.metadata.json") |
        ConvertFrom-Json
    if (-not $timeoutMetadata.timedOut -or
        -not $timeoutMetadata.treeKillAttempted -or
        -not $timeoutMetadata.treeKillSucceeded -or
        -not $timeoutMetadata.terminationComplete) {
        throw "timeout metadata did not prove bounded owned-process cleanup"
    }

    $unexpectedLookupFailure = $false
    try {
        Wait-ExactProcessExit -DaemonPid 41 -StartTime 1 -Seconds 1 `
            -ObserveProcess {
                throw [UnauthorizedAccessException]::new("observation denied")
            } | Out-Null
    } catch {
        $unexpectedLookupFailure = $_.Exception.Message -eq "observation denied"
    }
    if (-not $unexpectedLookupFailure) {
        throw "unexpected process observation failure was treated as absence"
    }
    if (-not (Wait-ExactProcessExit -DaemonPid 41 -StartTime 1 -Seconds 1 `
        -ObserveProcess {
            [pscustomobject]@{ State = "absent"; Process = $null }
        })) {
        throw "explicit no-such-process observation was not accepted"
    }

    $volumeRoot = [IO.Path]::GetPathRoot($env:RUNNER_TEMP)
    $existing = Join-Path $volumeRoot (
        "telex-v021-ci-" + [guid]::NewGuid().ToString("N")
    )
    New-Item -ItemType Directory -Path $existing | Out-Null
    $owned = $false
    try {
        New-ExclusiveProofRoot -Path $existing -VolumeRoot $volumeRoot
        $owned = $true
    } catch {
        if ($_.Exception.Message -notmatch "refusing to reuse existing proof root") {
            throw
        }
    } finally {
        if ($owned) {
            throw "existing proof root was incorrectly claimed"
        }
        if (-not (Test-Path -LiteralPath $existing)) {
            throw "existing proof root was modified or removed"
        }
        Remove-Item -LiteralPath $existing -Recurse -Force
    }
} finally {
    if (Test-Path -LiteralPath $root) {
        Remove-Item -LiteralPath $root -Recurse -Force
    }
}
