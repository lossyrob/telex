[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'DetectorCommon.psm1') -Force

try {
    $request = Read-DetectorRequest
    $inputPath = [string](Get-DetectorParameter -Request $request -Name 'inputPath')
    if ([string]::IsNullOrWhiteSpace($inputPath)) {
        throw 'Set parameters.inputPath to a local JSON file.'
    }
    $document = Get-Content -Raw (Resolve-DetectorPath $inputPath) | ConvertFrom-Json -AsHashtable
    $readyField = [string](Get-DetectorParameter -Request $request -Name 'readyField' -Default 'ready')
    $ready = $document.Contains($readyField) -and [bool]$document[$readyField]
    $evidence = [ordered]@{
        provider = 'local-file-json'
        path = (Resolve-DetectorPath $inputPath)
        readyField = $readyField
        ready = $ready
        version = [string]$document.version
        message = [string]$document.message
    }
    $cursor = Get-OpaqueCursor $evidence
    $event = $null
    if ($ready) {
        $event = [ordered]@{
            id = New-EventId -Provider 'file-json' -Scope $readyField -Cursor $cursor
            kind = 'local.file-json.ready'
            subject = "Local JSON condition '$readyField' is ready"
            body = [string]$document.message
            metadata = [ordered]@{ provider = 'local-file-json'; path = (Resolve-DetectorPath $inputPath); version = [string]$document.version }
        }
    }
    Write-SnapshotResult -Request $request -Evidence $evidence -Event $event
}
catch {
    Write-Degraded $_.Exception.Message
}
