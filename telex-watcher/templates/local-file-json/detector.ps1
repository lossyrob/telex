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

try {
    $request = Read-DetectorRequest
    $inputPath = [string](Get-DetectorParameter -Request $request -Name 'inputPath')
    if ([string]::IsNullOrWhiteSpace($inputPath)) {
        throw 'Set parameters.inputPath to a local JSON file.'
    }
    $document = Get-Content -Raw (Resolve-DetectorPath $inputPath) | ConvertFrom-Json -AsHashtable
    $field = [string](Get-DetectorParameter -Request $request -Name 'field' -Default 'ready')
    $expectedValue = Get-DetectorParameter -Request $request -Name 'expectedValue' -Default $true
    $observedValue = if ($document.Contains($field)) { $document[$field] } else { $null }
    $matched = (ConvertTo-CompactJson $observedValue) -eq (ConvertTo-CompactJson $expectedValue)
    $sourceId = [string](Get-DetectorParameter -Request $request -Name 'sourceId' -Default 'local-json')
    $evidence = [ordered]@{
        evidenceNormalizationVersion = 1
        provider = 'local-file-json'
        sourceId = $sourceId
        field = $field
        expectedValue = $expectedValue
        observedValue = $observedValue
        matched = $matched
        version = [string]$document.version
    }
    $cursor = Get-OpaqueCursor $evidence
    $event = $null
    if ($matched) {
        $event = [ordered]@{
            id = New-EventId -Provider 'local-file-json' -Scope $sourceId -Cursor $cursor
            kind = 'local.file-json.condition-met'
            subject = "Local JSON condition '$field' matched"
            body = [string]$document.message
            metadata = [ordered]@{
                provider = 'local-file-json'
                sourceId = $sourceId
                field = $field
                version = [string]$document.version
            }
        }
    }
    Write-SnapshotResult -Request $request -Evidence $evidence -Event $event
}
catch {
    Write-Degraded $_.Exception.Message
}
