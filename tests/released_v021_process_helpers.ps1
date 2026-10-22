Set-StrictMode -Version Latest

function Format-WindowsCommand([string]$Executable, [string[]]$Arguments) {
    '"' + $Executable + '" ' + (($Arguments | ForEach-Object {
        if ($_ -match '[\s"]') {
            '"' + ($_ -replace '"', '\"') + '"'
        } else {
            $_
        }
    }) -join " ")
}

function Read-SharedText([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        return ""
    }
    $share = [IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete
    $stream = [IO.FileStream]::new(
        $Path,
        [IO.FileMode]::Open,
        [IO.FileAccess]::Read,
        $share
    )
    try {
        $reader = [IO.StreamReader]::new($stream, [Text.Encoding]::UTF8, $true)
        try {
            return $reader.ReadToEnd()
        } finally {
            $reader.Dispose()
        }
    } finally {
        $stream.Dispose()
    }
}

function New-ExclusiveProofRoot([string]$Path, [string]$VolumeRoot) {
    if ((Split-Path -Parent $Path).TrimEnd("\") -ne $VolumeRoot.TrimEnd("\")) {
        throw "proof root must be a direct child of the volume root"
    }
    if (Test-Path -LiteralPath $Path) {
        throw "refusing to reuse existing proof root $Path"
    }
    New-Item -ItemType Directory -Path $Path -ErrorAction Stop | Out-Null
}

function Get-ExactProcessObservation([int]$DaemonPid) {
    try {
        $process = Get-Process -Id $DaemonPid -ErrorAction Stop
        return [pscustomobject]@{
            State = "present"
            Process = $process
        }
    } catch {
        if ($_.FullyQualifiedErrorId -like "NoProcessFoundForGivenId,*") {
            return [pscustomobject]@{
                State = "absent"
                Process = $null
            }
        }
        throw
    }
}

function Wait-ExactProcessExit {
    param(
        [int]$DaemonPid,
        [long]$StartTime,
        [int]$Seconds,
        [scriptblock]$ObserveProcess = {
            param($ObservedPid)
            Get-ExactProcessObservation $ObservedPid
        }
    )

    $deadline = [DateTime]::UtcNow.AddSeconds($Seconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        $observation = & $ObserveProcess $DaemonPid
        if ($observation.State -eq "absent") {
            return $true
        }
        if ($observation.State -ne "present" -or $null -eq $observation.Process) {
            throw "process observation returned an unsupported state"
        }
        if ($observation.Process.StartTime.ToUniversalTime().ToFileTimeUtc() -ne
            $StartTime) {
            return $true
        }
        Start-Sleep -Milliseconds 100
    }
    return $false
}

function Invoke-BoundedLoggedProcess {
    param(
        [Parameter(Mandatory = $true)]
        [string]$LogsDirectory,

        [Parameter(Mandatory = $true)]
        [string]$CommandsFile,

        [Parameter(Mandatory = $true)]
        [string]$Name,

        [Parameter(Mandatory = $true)]
        [string]$Executable,

        [string[]]$Arguments = @(),

        [hashtable]$Environment = @{},

        [switch]$Isolated,

        [int]$TimeoutSeconds = 120
    )

    if ($TimeoutSeconds -lt 2) {
        throw "process timeout must leave at least one second for owned cleanup"
    }
    $stdout = Join-Path $LogsDirectory "$Name.stdout.txt"
    $stderr = Join-Path $LogsDirectory "$Name.stderr.txt"
    $metadata = Join-Path $LogsDirectory "$Name.metadata.json"
    [IO.File]::WriteAllText($stdout, "", [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText($stderr, "", [Text.UTF8Encoding]::new($false))

    $directCommand = Format-WindowsCommand $Executable $Arguments
    $command = "$directCommand 1> `"$stdout`" 2> `"$stderr`""
    Add-Content -LiteralPath $CommandsFile -Value $directCommand -Encoding utf8

    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $env:ComSpec
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.ArgumentList.Add("/d")
    $start.ArgumentList.Add("/s")
    $start.ArgumentList.Add("/c")
    $start.ArgumentList.Add($command)
    if ($Isolated) {
        $allowed = @(
            "PATH", "SYSTEMROOT", "WINDIR", "COMSPEC", "PATHEXT", "SYSTEMDRIVE",
            "PROCESSOR_ARCHITECTURE", "NUMBER_OF_PROCESSORS", "PSMODULEPATH"
        )
        $start.Environment.Clear()
        foreach ($variable in $allowed) {
            $value = [Environment]::GetEnvironmentVariable($variable)
            if ($null -ne $value) {
                $start.Environment[$variable] = $value
            }
        }
    }
    foreach ($entry in $Environment.GetEnumerator()) {
        $start.Environment[$entry.Key] = [string]$entry.Value
    }

    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $start
    $started = $false
    $timedOut = $false
    $treeKillAttempted = $false
    $treeKillSucceeded = $false
    $terminationComplete = $false
    $exitCode = $null
    $primaryError = $null
    $startedAt = [DateTime]::UtcNow
    $deadline = $startedAt.AddSeconds($TimeoutSeconds)
    $cleanupReserveMs = [Math]::Min(
        5000,
        [Math]::Max(1000, [int]($TimeoutSeconds * 100))
    )
    $runBudgetMs = ($TimeoutSeconds * 1000) - $cleanupReserveMs

    try {
        if (-not $process.Start()) {
            throw "failed to start $Name"
        }
        $started = $true
        if ($process.WaitForExit($runBudgetMs)) {
            $terminationComplete = $true
        } else {
            $timedOut = $true
            $treeKillAttempted = $true
            try {
                $process.Kill($true)
                $treeKillSucceeded = $true
            } catch {
                $primaryError = $_
            }
            $remainingMs = [Math]::Max(
                0,
                [int]($deadline - [DateTime]::UtcNow).TotalMilliseconds
            )
            if ($remainingMs -gt 0) {
                $terminationComplete = $process.WaitForExit($remainingMs)
            }
        }
        if ($terminationComplete) {
            $exitCode = $process.ExitCode
        }
    } catch {
        if ($null -eq $primaryError) {
            $primaryError = $_
        }
    } finally {
        $stdoutText = Read-SharedText $stdout
        $stderrText = Read-SharedText $stderr
        $record = [ordered]@{
            name = $Name
            executable = $Executable
            arguments = $Arguments
            timeoutSeconds = $TimeoutSeconds
            started = $started
            timedOut = $timedOut
            treeKillAttempted = $treeKillAttempted
            treeKillSucceeded = $treeKillSucceeded
            terminationComplete = $terminationComplete
            exitCode = $exitCode
            elapsedMilliseconds =
                [Math]::Round(([DateTime]::UtcNow - $startedAt).TotalMilliseconds)
            stdoutBytes = [Text.Encoding]::UTF8.GetByteCount($stdoutText)
            stderrBytes = [Text.Encoding]::UTF8.GetByteCount($stderrText)
            error = if ($null -eq $primaryError) {
                $null
            } else {
                $primaryError.Exception.Message
            }
        }
        [IO.File]::WriteAllText(
            $metadata,
            ($record | ConvertTo-Json -Depth 6),
            [Text.UTF8Encoding]::new($false)
        )
        $process.Dispose()
    }

    if ($timedOut) {
        if (-not $treeKillSucceeded -or -not $terminationComplete) {
            throw "$Name timed out and owned process-tree cleanup did not complete"
        }
        throw "$Name exceeded its ${TimeoutSeconds}s process/capture deadline"
    }
    if ($null -ne $primaryError) {
        throw $primaryError
    }
    if (-not $terminationComplete) {
        throw "$Name did not establish direct-process termination"
    }

    [pscustomobject]@{
        Name = $Name
        ExitCode = $exitCode
        Stdout = $stdoutText
        Stderr = $stderrText
        StdoutPath = $stdout
        StderrPath = $stderr
        MetadataPath = $metadata
    }
}
