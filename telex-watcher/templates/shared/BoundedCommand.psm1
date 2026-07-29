Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Invoke-BoundedCommand {
    param(
        [string]$FileName,
        [string[]]$Arguments,
        [string]$WorkingDirectory,
        [int]$TimeoutSeconds,
        [int]$MaxChars
    )

    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $FileName
    $startInfo.WorkingDirectory = $WorkingDirectory
    $startInfo.UseShellExecute = $false
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.CreateNoWindow = $true
    foreach ($argument in $Arguments) {
        $startInfo.ArgumentList.Add($argument)
    }

    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    if (-not $process.Start()) {
        throw 'wrapped-command-start-failed'
    }

    $stdout = [Text.StringBuilder]::new()
    $stderr = [Text.StringBuilder]::new()
    $stdoutBuffer = [char[]]::new(4096)
    $stderrBuffer = [char[]]::new(4096)
    $stdoutTask = $process.StandardOutput.ReadAsync($stdoutBuffer, 0, $stdoutBuffer.Length)
    $stderrTask = $process.StandardError.ReadAsync($stderrBuffer, 0, $stderrBuffer.Length)
    $stdoutDone = $false
    $stderrDone = $false
    $clock = [Diagnostics.Stopwatch]::StartNew()

    try {
        while (-not ($process.HasExited -and $stdoutDone -and $stderrDone)) {
            if ($clock.Elapsed.TotalSeconds -ge $TimeoutSeconds) {
                try { $process.Kill($true) } catch {}
                throw 'wrapped-command-timeout'
            }

            if (-not $stdoutDone -and $stdoutTask.IsCompleted) {
                $count = $stdoutTask.GetAwaiter().GetResult()
                if ($count -eq 0) {
                    $stdoutDone = $true
                }
                else {
                    if ($stdout.Length + $count -gt $MaxChars) {
                        try { $process.Kill($true) } catch {}
                        throw 'wrapped-command-output-too-large'
                    }
                    [void]$stdout.Append($stdoutBuffer, 0, $count)
                    $stdoutTask = $process.StandardOutput.ReadAsync($stdoutBuffer, 0, $stdoutBuffer.Length)
                }
            }

            if (-not $stderrDone -and $stderrTask.IsCompleted) {
                $count = $stderrTask.GetAwaiter().GetResult()
                if ($count -eq 0) {
                    $stderrDone = $true
                }
                else {
                    if ($stderr.Length + $count -gt $MaxChars) {
                        try { $process.Kill($true) } catch {}
                        throw 'wrapped-command-output-too-large'
                    }
                    [void]$stderr.Append($stderrBuffer, 0, $count)
                    $stderrTask = $process.StandardError.ReadAsync($stderrBuffer, 0, $stderrBuffer.Length)
                }
            }

            if (-not ($process.HasExited -and $stdoutDone -and $stderrDone)) {
                Start-Sleep -Milliseconds 5
            }
        }

        return [pscustomobject]@{
            ExitCode = $process.ExitCode
            Stdout = $stdout.ToString()
            Stderr = $stderr.ToString()
        }
    }
    finally {
        $clock.Stop()
        $process.Dispose()
    }
}

Export-ModuleMember -Function Invoke-BoundedCommand
