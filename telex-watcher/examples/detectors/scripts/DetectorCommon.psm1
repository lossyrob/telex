Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Read-DetectorRequest {
    $raw = [Console]::In.ReadToEnd()
    if ([string]::IsNullOrWhiteSpace($raw)) {
        throw 'Detector stdin must contain a version-1 request object.'
    }

    $request = $raw | ConvertFrom-Json -AsHashtable
    if ($request.schemaVersion -ne 1) {
        throw "Unsupported request schemaVersion '$($request.schemaVersion)'."
    }
    foreach ($field in 'attempt', 'watch', 'script', 'state') {
        if (-not $request.ContainsKey($field)) {
            throw "Request is missing '$field'."
        }
    }
    return $request
}

function Get-DetectorParameter {
    param(
        [hashtable]$Request,
        [string]$Name,
        $Default = $null
    )

    $parameters = $Request.watch.parameters
    if ($parameters -is [System.Collections.IDictionary] -and $parameters.Contains($Name)) {
        return $parameters[$Name]
    }
    return $Default
}

function Resolve-DetectorPath {
    param([string]$Path)

    if ([System.IO.Path]::IsPathRooted($Path)) {
        return $Path
    }
    return [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot $Path))
}

function ConvertTo-CompactJson {
    param($Value)
    return [string](ConvertTo-Json -InputObject $Value -Depth 30 -Compress)
}

function Get-Sha256 {
    param([string]$Text)

    $bytes = [System.Text.Encoding]::UTF8.GetBytes($Text)
    $hash = [System.Security.Cryptography.SHA256]::HashData($bytes)
    return ([System.BitConverter]::ToString($hash)).Replace('-', '').ToLowerInvariant()
}

function Get-OpaqueCursor {
    param($Evidence)
    $json = ConvertTo-CompactJson -Value $Evidence
    return Get-Sha256 -Text $json
}

function Get-StateCursor {
    param([hashtable]$Request)

    if ($Request.state -is [System.Collections.IDictionary] -and $Request.state.Contains('cursor')) {
        return [string]$Request.state.cursor
    }
    return $null
}

function New-EventId {
    param(
        [string]$Provider,
        [string]$Scope,
        [string]$Cursor
    )

    return "$Provider`:$Scope`:$($Cursor.Substring(0, 24))"
}

function Write-DetectorResult {
    param(
        [ValidateSet('idle', 'event', 'terminal', 'degraded')]
        [string]$Outcome,
        $NextState = $null,
        $Event = $null
    )

    $result = [ordered]@{
        schemaVersion = 1
        outcome = $Outcome
    }
    if ($null -ne $NextState) {
        $result.nextState = $NextState
    }
    if ($null -ne $Event) {
        $result.event = $Event
    }
    [Console]::Out.WriteLine((ConvertTo-CompactJson $result))
}

function Write-Degraded {
    param([string]$Message)

    [Console]::Error.WriteLine("detector degraded: $Message")
    Write-DetectorResult -Outcome degraded
}

function Write-SnapshotResult {
    param(
        [hashtable]$Request,
        [System.Collections.IDictionary]$Evidence,
        $Event,
        [switch]$Terminal
    )

    $cursor = Get-OpaqueCursor $Evidence
    $nextState = [ordered]@{ cursor = $cursor }
    $previousCursor = Get-StateCursor $Request
    $emitInitialSnapshot = [bool](Get-DetectorParameter -Request $Request -Name 'emitInitialSnapshot' -Default $false)

    if ($previousCursor -eq $cursor) {
        Write-DetectorResult -Outcome idle -NextState $nextState
        return
    }
    if ($null -eq $previousCursor -and -not $emitInitialSnapshot) {
        Write-DetectorResult -Outcome idle -NextState $nextState
        return
    }
    if ($null -eq $Event) {
        Write-DetectorResult -Outcome idle -NextState $nextState
        return
    }

    Write-DetectorResult -Outcome $(if ($Terminal) { 'terminal' } else { 'event' }) -NextState $nextState -Event $Event
}

Export-ModuleMember -Function Read-DetectorRequest, Get-DetectorParameter, Resolve-DetectorPath, ConvertTo-CompactJson, Get-Sha256, Get-OpaqueCursor, Get-StateCursor, New-EventId, Write-DetectorResult, Write-Degraded, Write-SnapshotResult
