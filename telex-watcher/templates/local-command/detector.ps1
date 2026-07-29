[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$sharedPath = Join-Path (Split-Path -Parent $PSScriptRoot) 'shared'
$helperPath = Join-Path $sharedPath 'DetectorCommon.psm1'
$boundedCommandPath = Join-Path $sharedPath 'BoundedCommand.psm1'
$expectedHelperSha256 = 'cca5ae57123142df3b7bd053cb6a1d88e0436ca38dd769533d5d4591987201b1'
$expectedBoundedCommandSha256 = '2ee2894ba3ca0e7cb4e3a5ccf6e05dc9a7a31b305aa5c0334b3fe5bf39e5b0a9'
if (
    (Get-FileHash $helperPath -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expectedHelperSha256 -or
    (Get-FileHash $boundedCommandPath -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expectedBoundedCommandSha256
) {
    [Console]::Error.WriteLine('detectorDiagnostic={"schemaVersion":1,"code":"shared-helper-digest-mismatch","message":"Pinned shared helper digest mismatch."}')
    [Console]::Out.WriteLine('{"schemaVersion":1,"outcome":"degraded"}')
    exit 0
}
Import-Module $helperPath -Force
Import-Module $boundedCommandPath -Force

try {
    $request = Read-DetectorRequest
    $command = @(Get-DetectorParameter -Request $request -Name 'command')
    if (
        $command.Count -lt 1 -or
        [string]::IsNullOrWhiteSpace([string]$command[0]) -or
        [string]$command[0] -eq '<OBSERVATIONAL-COMMAND>'
    ) {
        throw 'command-policy: parameters.command must be a concrete non-empty argv array.'
    }
    $workingDirectory = [string](Get-DetectorParameter -Request $request -Name 'workingDirectory' -Default (Get-Location).Path)
    if ($workingDirectory -eq '<COMMAND-WORKING-DIRECTORY>') {
        throw 'command-policy: replace the sample workingDirectory placeholder.'
    }
    $timeoutSeconds = [int](Get-DetectorParameter -Request $request -Name 'commandTimeoutSeconds' -Default 20)
    $maxOutputChars = [int](Get-DetectorParameter -Request $request -Name 'maxOutputChars' -Default 16384)
    if ($timeoutSeconds -lt 1 -or $timeoutSeconds -gt 60) {
        throw 'command-policy: commandTimeoutSeconds must be between 1 and 60.'
    }
    if ($maxOutputChars -lt 256 -or $maxOutputChars -gt 65536) {
        throw 'command-policy: maxOutputChars must be between 256 and 65536.'
    }
    $result = Invoke-BoundedCommand `
        -FileName ([string]$command[0]) `
        -Arguments ([string[]]@($command | Select-Object -Skip 1)) `
        -WorkingDirectory (Resolve-DetectorPath $workingDirectory) `
        -TimeoutSeconds $timeoutSeconds `
        -MaxChars $maxOutputChars
    $conditionExitCodes = @((Get-DetectorParameter -Request $request -Name 'conditionExitCodes' -Default @(0))) | ForEach-Object { [int]$_ }
    $successExitCodes = @((Get-DetectorParameter -Request $request -Name 'successExitCodes' -Default @(1))) | ForEach-Object { [int]$_ }
    if ($result.ExitCode -notin $conditionExitCodes -and $result.ExitCode -notin $successExitCodes) {
        throw "wrapped-command-failed: exit code $($result.ExitCode); stderr: $($result.Stderr.Trim())"
    }
    $conditionMet = $result.ExitCode -in $conditionExitCodes
    $sourceId = [string](Get-DetectorParameter -Request $request -Name 'sourceId' -Default 'local-command')
    $evidence = [ordered]@{
        evidenceNormalizationVersion = 2
        provider = 'local-command'
        sourceId = $sourceId
        exitCode = $result.ExitCode
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
