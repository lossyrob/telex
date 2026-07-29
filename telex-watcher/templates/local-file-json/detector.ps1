[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$helperPath = Join-Path $PSScriptRoot '..\shared\DetectorCommon.psm1'
$expectedHelperSha256 = 'd7fcef49f32f4057a2495f741d5ecc5e8146ba4609f401723f2d753a71d37c0c'
if ((Get-FileHash $helperPath -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expectedHelperSha256) {
    [Console]::Error.WriteLine('detectorDiagnostic={"schemaVersion":1,"code":"shared-helper-digest-mismatch","message":"Pinned shared helper digest mismatch."}')
    [Console]::Out.WriteLine('{"schemaVersion":1,"outcome":"degraded"}')
    exit 0
}
Import-Module $helperPath -Force

try {
    $request = Read-DetectorRequest
    $inputPath = [string](Get-DetectorParameter -Request $request -Name 'inputPath')
    if ([string]::IsNullOrWhiteSpace($inputPath) -or $inputPath -eq '<INPUT-JSON-PATH>') {
        throw 'configuration-invalid: set parameters.inputPath to a concrete local JSON file.'
    }
    $document = Get-Content -Raw (Resolve-DetectorPath $inputPath) | ConvertFrom-Json -AsHashtable
    $field = [string](Get-DetectorParameter -Request $request -Name 'field' -Default 'ready')
    $expectedValue = Get-DetectorParameter -Request $request -Name 'expectedValue' -Default $true
    if (
        $expectedValue -is [System.Collections.IDictionary] -or
        $expectedValue -is [System.Collections.IList]
    ) {
        throw 'configuration-invalid: parameters.expectedValue must be a JSON scalar or null.'
    }
    $fieldPresent = $document.Contains($field)
    $observedValue = if ($fieldPresent) { $document[$field] } else { $null }
    $matched = $fieldPresent -and (ConvertTo-CanonicalJson $observedValue) -eq (ConvertTo-CanonicalJson $expectedValue)
    $sourceId = [string](Get-DetectorParameter -Request $request -Name 'sourceId' -Default 'local-json')
    $version = [string](Get-OptionalValue -Object $document -Name 'version' -Default '')
    $evidence = [ordered]@{
        evidenceNormalizationVersion = 2
        provider = 'local-file-json'
        sourceId = $sourceId
        field = $field
        expectedValue = $expectedValue
        fieldPresent = $fieldPresent
        observedValue = $observedValue
        matched = $matched
        version = $version
    }
    $cursor = Get-OpaqueCursor $evidence
    $event = $null
    if ($matched) {
        $event = [ordered]@{
            id = New-EventId -Provider 'local-file-json' -Scope $sourceId -Cursor $cursor
            kind = 'local.file-json.condition-met'
            subject = "Local JSON condition '$field' matched"
            body = [string](Get-OptionalValue -Object $document -Name 'message' -Default 'The configured local JSON condition matched.')
            metadata = [ordered]@{
                provider = 'local-file-json'
                sourceId = $sourceId
                field = $field
                version = $version
            }
        }
    }
    Write-SnapshotResult -Request $request -Evidence $evidence -Event $event
}
catch {
    Write-Degraded $_.Exception.Message
}
