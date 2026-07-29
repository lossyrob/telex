[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$helperPath = Join-Path $PSScriptRoot '..\shared\DetectorCommon.psm1'
$boundedCommandPath = Join-Path $PSScriptRoot '..\shared\BoundedCommand.psm1'
$expectedHelperSha256 = 'd7fcef49f32f4057a2495f741d5ecc5e8146ba4609f401723f2d753a71d37c0c'
$expectedBoundedCommandSha256 = '656274b91788bb95aec585f6ed099a4754ad2722c6576bd2ce9c521faf960bdf'
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
