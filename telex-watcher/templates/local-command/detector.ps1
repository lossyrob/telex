[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$helperPath = Join-Path $PSScriptRoot '..\shared\DetectorCommon.psm1'
$expectedHelperSha256 = '611f0dc780fd771db29cd95187c6d79d9b527ea1581f3dbb466ccc8883bc8428'
if ((Get-FileHash $helperPath -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expectedHelperSha256) {
    [Console]::Error.WriteLine('detector degraded: shared-helper-digest-mismatch')
    [Console]::Out.WriteLine('{"schemaVersion":1,"outcome":"degraded"}')
    exit 0
}
Import-Module $helperPath -Force

if (-not ('TelexWatcher.BoundedCommand' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Diagnostics;
using System.Text;
using System.Threading;
using System.Threading.Tasks;

namespace TelexWatcher {
    public sealed class CommandResult {
        public int ExitCode { get; set; }
        public string Stdout { get; set; } = "";
        public string Stderr { get; set; } = "";
    }

    public static class BoundedCommand {
        private static async Task<string> ReadBoundedAsync(
            System.IO.StreamReader reader,
            int maxChars,
            Process process,
            CancellationToken token) {
            var builder = new StringBuilder();
            var buffer = new char[4096];
            while (true) {
                var read = await reader.ReadAsync(buffer, 0, buffer.Length).ConfigureAwait(false);
                if (read == 0) {
                    return builder.ToString();
                }
                if (builder.Length + read > maxChars) {
                    try { process.Kill(true); } catch { }
                    throw new InvalidOperationException("wrapped-command-output-too-large");
                }
                builder.Append(buffer, 0, read);
                token.ThrowIfCancellationRequested();
            }
        }

        public static CommandResult Run(
            string fileName,
            string[] arguments,
            string workingDirectory,
            int timeoutSeconds,
            int maxChars) {
            using var process = new Process();
            process.StartInfo = new ProcessStartInfo {
                FileName = fileName,
                WorkingDirectory = workingDirectory,
                UseShellExecute = false,
                RedirectStandardOutput = true,
                RedirectStandardError = true,
                CreateNoWindow = true
            };
            foreach (var argument in arguments) {
                process.StartInfo.ArgumentList.Add(argument);
            }
            process.Start();
            using var cancellation = new CancellationTokenSource(TimeSpan.FromSeconds(timeoutSeconds));
            var stdout = ReadBoundedAsync(process.StandardOutput, maxChars, process, cancellation.Token);
            var stderr = ReadBoundedAsync(process.StandardError, maxChars, process, cancellation.Token);
            try {
                process.WaitForExitAsync(cancellation.Token).GetAwaiter().GetResult();
                Task.WhenAll(stdout, stderr).GetAwaiter().GetResult();
            } catch (OperationCanceledException) {
                try { process.Kill(true); } catch { }
                throw new TimeoutException("wrapped-command-timeout");
            }
            return new CommandResult {
                ExitCode = process.ExitCode,
                Stdout = stdout.Result,
                Stderr = stderr.Result
            };
        }
    }
}
'@
}

try {
    $request = Read-DetectorRequest
    $command = @(Get-DetectorParameter -Request $request -Name 'command')
    if ($command.Count -lt 1 -or [string]::IsNullOrWhiteSpace([string]$command[0])) {
        throw 'command-policy: parameters.command must be a non-empty argv array.'
    }
    $workingDirectory = [string](Get-DetectorParameter -Request $request -Name 'workingDirectory' -Default (Get-Location).Path)
    $timeoutSeconds = [int](Get-DetectorParameter -Request $request -Name 'commandTimeoutSeconds' -Default 20)
    $maxOutputChars = [int](Get-DetectorParameter -Request $request -Name 'maxOutputChars' -Default 16384)
    if ($timeoutSeconds -lt 1 -or $timeoutSeconds -gt 60) {
        throw 'command-policy: commandTimeoutSeconds must be between 1 and 60.'
    }
    if ($maxOutputChars -lt 256 -or $maxOutputChars -gt 65536) {
        throw 'command-policy: maxOutputChars must be between 256 and 65536.'
    }
    $result = [TelexWatcher.BoundedCommand]::Run(
        [string]$command[0],
        [string[]]@($command | Select-Object -Skip 1),
        (Resolve-DetectorPath $workingDirectory),
        $timeoutSeconds,
        $maxOutputChars
    )
    $conditionExitCodes = @((Get-DetectorParameter -Request $request -Name 'conditionExitCodes' -Default @(0))) | ForEach-Object { [int]$_ }
    $successExitCodes = @((Get-DetectorParameter -Request $request -Name 'successExitCodes' -Default @(1))) | ForEach-Object { [int]$_ }
    if ($result.ExitCode -notin $conditionExitCodes -and $result.ExitCode -notin $successExitCodes) {
        throw "wrapped-command-failed: exit code $($result.ExitCode); stderr: $($result.Stderr.Trim())"
    }
    $conditionMet = $result.ExitCode -in $conditionExitCodes
    $sourceId = [string](Get-DetectorParameter -Request $request -Name 'sourceId' -Default 'local-command')
    $evidence = [ordered]@{
        evidenceNormalizationVersion = 1
        provider = 'local-command'
        sourceId = $sourceId
        exitCode = $result.ExitCode
        stdoutSha256 = Get-Sha256 $result.Stdout
        conditionMet = $conditionMet
    }
    $cursor = Get-OpaqueCursor $evidence
    $event = $null
    if ($conditionMet) {
        $event = [ordered]@{
            id = New-EventId -Provider 'local-command' -Scope $sourceId -Cursor $cursor
            kind = 'local.command.condition-met'
            subject = "Local command condition '$sourceId' matched"
            body = $result.Stdout.Trim()
            metadata = [ordered]@{
                provider = 'local-command'
                sourceId = $sourceId
                exitCode = $result.ExitCode
            }
        }
    }
    Write-SnapshotResult -Request $request -Evidence $evidence -Event $event
}
catch {
    Write-Degraded $_.Exception.Message
}
