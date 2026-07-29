Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Read-DetectorRequest {
    $raw = [Console]::In.ReadToEnd()
    if ([string]::IsNullOrWhiteSpace($raw)) {
        throw 'Detector stdin must contain a version-1 request object.'
    }

    $request = $raw | ConvertFrom-Json -AsHashtable -NoEnumerate
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

function Get-OptionalValue {
    param(
        $Object,
        [string]$Name,
        $Default = $null
    )

    if ($null -eq $Object) {
        return $Default
    }
    if ($Object -is [System.Collections.IDictionary]) {
        if ($Object.Contains($Name)) {
            return $Object[$Name]
        }
        return $Default
    }
    $property = $Object.PSObject.Properties[$Name]
    if ($null -eq $property) {
        return $Default
    }
    return $property.Value
}

function Resolve-DetectorPath {
    param([string]$Path)

    if ([System.IO.Path]::IsPathRooted($Path)) {
        return $Path
    }
    return [System.IO.Path]::GetFullPath((Join-Path (Get-Location).Path $Path))
}

function ConvertTo-CompactJson {
    param($Value)
    return [string](ConvertTo-Json -InputObject $Value -Depth 30 -Compress)
}

function ConvertTo-CanonicalJsonValue {
    param($Value)

    if ($Value -is [DateTimeOffset]) {
        return $Value.ToUniversalTime().ToString('o', [Globalization.CultureInfo]::InvariantCulture)
    }
    if ($Value -is [DateTime]) {
        if ($Value.Kind -eq [DateTimeKind]::Unspecified) {
            throw 'canonical-json-invalid: DateTime values must include an offset or UTC kind.'
        }
        return ([DateTimeOffset]$Value).ToUniversalTime().ToString('o', [Globalization.CultureInfo]::InvariantCulture)
    }
    if ($Value -is [System.Collections.IDictionary]) {
        $keys = [string[]]@($Value.Keys | ForEach-Object { [string]$_ })
        [Array]::Sort($keys, [System.StringComparer]::Ordinal)
        $ordered = [ordered]@{}
        foreach ($key in $keys) {
            $ordered[$key] = ConvertTo-CanonicalJsonValue -Value $Value[$key]
        }
        return $ordered
    }
    if ($Value -is [System.Collections.IList] -and $Value -isnot [string]) {
        $items = [System.Collections.Generic.List[object]]::new()
        foreach ($item in $Value) {
            $items.Add((ConvertTo-CanonicalJsonValue -Value $item))
        }
        return ,$items.ToArray()
    }
    return $Value
}

function ConvertTo-CanonicalJson {
    param($Value)

    $canonical = ConvertTo-CanonicalJsonValue -Value $Value
    if ($null -eq $canonical) {
        return 'null'
    }
    return [System.Text.Json.JsonSerializer]::Serialize($canonical, $canonical.GetType())
}

function Get-Sha256 {
    param([string]$Text)

    $bytes = [System.Text.Encoding]::UTF8.GetBytes($Text)
    $hash = [System.Security.Cryptography.SHA256]::HashData($bytes)
    return ([System.BitConverter]::ToString($hash)).Replace('-', '').ToLowerInvariant()
}

function Get-OpaqueCursor {
    param($Evidence)
    $json = ConvertTo-CanonicalJson -Value $Evidence
    return Get-Sha256 -Text $json
}

function Get-StateCursor {
    param([hashtable]$Request)

    if ($Request.state -is [System.Collections.IDictionary] -and $Request.state.Contains('cursor')) {
        return [string]$Request.state.cursor
    }
    return $null
}

function Get-PreflightEvidence {
    param([hashtable]$Request)

    if ($Request.state -is [System.Collections.IDictionary] -and $Request.state.Contains('preflight')) {
        return $Request.state.preflight
    }
    return $null
}

function Test-Rfc3339Timestamp {
    param($Value)

    if ($Value -is [DateTimeOffset]) {
        return $true
    }
    if ($Value -is [DateTime]) {
        return $Value.Kind -ne [DateTimeKind]::Unspecified
    }
    $text = [string]$Value
    if ($text -notmatch '^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$') {
        return $false
    }
    $parsed = [DateTimeOffset]::MinValue
    return [DateTimeOffset]::TryParse(
        $text,
        [Globalization.CultureInfo]::InvariantCulture,
        [Globalization.DateTimeStyles]::RoundtripKind,
        [ref]$parsed
    )
}

function ConvertTo-Rfc3339Utc {
    param($Value)

    if ($Value -is [DateTimeOffset]) {
        $parsed = $Value
    }
    elseif ($Value -is [DateTime]) {
        if ($Value.Kind -eq [DateTimeKind]::Unspecified) {
            throw 'invalid-rfc3339: timestamp must include an explicit offset.'
        }
        $parsed = [DateTimeOffset]$Value
    }
    else {
        $text = [string]$Value
        if (-not (Test-Rfc3339Timestamp -Value $text)) {
            throw 'invalid-rfc3339: timestamp must include an explicit offset.'
        }
        $parsed = [DateTimeOffset]::Parse(
            $text,
            [Globalization.CultureInfo]::InvariantCulture,
            [Globalization.DateTimeStyles]::RoundtripKind
        )
    }
    return $parsed.ToUniversalTime().ToString('o', [Globalization.CultureInfo]::InvariantCulture)
}

function Assert-DetectorTestMode {
    param(
        [hashtable]$Request,
        [string[]]$ParameterNames
    )

    $parameters = $Request.watch.parameters
    foreach ($name in $ParameterNames) {
        if (
            $parameters -is [System.Collections.IDictionary] -and
            $parameters.Contains($name) -and
            $null -ne $parameters[$name] -and
            -not [string]::IsNullOrWhiteSpace([string]$parameters[$name]) -and
            $env:TELEX_WATCHER_TEST -ne '1'
        ) {
            throw "test-mode-required: parameters.$name is available only when TELEX_WATCHER_TEST=1."
        }
    }
}

function Write-TestTransportRecord {
    param(
        [hashtable]$Request,
        [System.Collections.IDictionary]$Record
    )

    $capturePath = [string](Get-DetectorParameter -Request $Request -Name 'testTransportCapturePath' -Default '')
    if ([string]::IsNullOrWhiteSpace($capturePath)) {
        return
    }
    Assert-DetectorTestMode -Request $Request -ParameterNames @('testTransportCapturePath')
    $resolved = Resolve-DetectorPath $capturePath
    $parent = Split-Path -Parent $resolved
    if (-not [string]::IsNullOrWhiteSpace($parent)) {
        [IO.Directory]::CreateDirectory($parent) | Out-Null
    }
    [IO.File]::AppendAllText(
        $resolved,
        "$(ConvertTo-CompactJson -Value $Record)$([Environment]::NewLine)",
        [Text.UTF8Encoding]::new($false)
    )
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
    param(
        [string]$Message,
        [string]$Code = ''
    )

    if ([string]::IsNullOrWhiteSpace($Code)) {
        $prefix = ($Message -split ':', 2)[0]
        $Code = if ($prefix -match '^[a-z0-9]+(?:-[a-z0-9]+)+$') { $prefix } else { 'detector-failure' }
    }
    $diagnostic = [ordered]@{
        schemaVersion = 1
        code = $Code
        message = $Message
    }
    [Console]::Error.WriteLine("detectorDiagnostic=$(ConvertTo-CompactJson -Value $diagnostic)")
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

    if ($previousCursor -eq $cursor) {
        Write-DetectorResult -Outcome idle -NextState $nextState
        return
    }
    if ($Terminal) {
        Write-DetectorResult -Outcome terminal -NextState $nextState -Event $Event
        return
    }
    if ($null -eq $Event) {
        Write-DetectorResult -Outcome idle -NextState $nextState
        return
    }

    Write-DetectorResult -Outcome event -NextState $nextState -Event $Event
}

function Write-EventlessTerminal {
    param([System.Collections.IDictionary]$Evidence)

    Write-DetectorResult -Outcome terminal -NextState ([ordered]@{
        cursor = Get-OpaqueCursor $Evidence
    })
}

Export-ModuleMember -Function Read-DetectorRequest, Get-DetectorParameter, Get-OptionalValue, Resolve-DetectorPath, ConvertTo-CompactJson, ConvertTo-CanonicalJson, Get-Sha256, Get-OpaqueCursor, Get-StateCursor, Get-PreflightEvidence, Test-Rfc3339Timestamp, ConvertTo-Rfc3339Utc, Assert-DetectorTestMode, Write-TestTransportRecord, New-EventId, Write-DetectorResult, Write-Degraded, Write-SnapshotResult, Write-EventlessTerminal
