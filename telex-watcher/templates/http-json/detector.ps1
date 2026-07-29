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

function Get-JsonField {
    param($Document, [string]$Path)

    $value = $Document
    foreach ($segment in $Path.Split('.', [System.StringSplitOptions]::RemoveEmptyEntries)) {
        if ($value -is [System.Collections.IDictionary] -and $value.Contains($segment)) {
            $value = $value[$segment]
            continue
        }
        if ($value -is [System.Collections.IList] -and $segment -match '^\d+$' -and [int]$segment -lt $value.Count) {
            $value = $value[[int]$segment]
            continue
        }
        return $null
    }
    return $value
}

try {
    $request = Read-DetectorRequest
    $fixturePath = [string](Get-DetectorParameter -Request $request -Name 'fixturePath' -Default '')
    if (-not [string]::IsNullOrWhiteSpace($fixturePath)) {
        $content = Get-Content -Raw (Resolve-DetectorPath $fixturePath)
    }
    else {
        $url = [string](Get-DetectorParameter -Request $request -Name 'url')
        if (-not $url.StartsWith('https://', [System.StringComparison]::OrdinalIgnoreCase)) {
            throw 'transport-policy: parameters.url must use HTTPS.'
        }
        $authMode = [string](Get-DetectorParameter -Request $request -Name 'authentication' -Default 'none')
        $headers = @{}
        if ($authMode -eq 'bearer') {
            if ([string]::IsNullOrWhiteSpace($env:HTTP_JSON_BEARER_TOKEN)) {
                throw 'missing-credential: HTTP_JSON_BEARER_TOKEN was not supplied by the environment allowlist.'
            }
            $headers.Authorization = "Bearer $($env:HTTP_JSON_BEARER_TOKEN)"
        }
        elseif ($authMode -eq 'header') {
            $headerName = [string](Get-DetectorParameter -Request $request -Name 'headerName')
            if ($headerName -notmatch '^[A-Za-z0-9-]+$') {
                throw 'credential-policy: header authentication requires a safe parameters.headerName.'
            }
            if ([string]::IsNullOrWhiteSpace($env:HTTP_JSON_HEADER_VALUE)) {
                throw 'missing-credential: HTTP_JSON_HEADER_VALUE was not supplied by the environment allowlist.'
            }
            $headers[$headerName] = $env:HTTP_JSON_HEADER_VALUE
        }
        elseif ($authMode -ne 'none') {
            throw "credential-policy: unsupported authentication mode '$authMode'."
        }
        try {
            $response = Invoke-WebRequest -Method Get -Uri $url -Headers $headers -MaximumRedirection 0 -TimeoutSec 20
        }
        catch {
            $status = 0
            if ($null -ne $_.Exception.Response) {
                $status = [int]$_.Exception.Response.StatusCode
            }
            if ($status -in @(401, 403)) {
                throw "authorization-denied: HTTPS GET returned status $status."
            }
            if ($status -eq 429) {
                throw 'rate-limited: HTTPS GET returned status 429.'
            }
            if ($status -ge 300 -and $status -lt 400) {
                throw "redirect-rejected: HTTPS GET returned status $status."
            }
            throw "provider-failure: HTTPS GET failed with status $status."
        }
        $content = [string]$response.Content
    }
    if ([Text.Encoding]::UTF8.GetByteCount($content) -gt 1048576) {
        throw 'response-too-large: response exceeded the 1 MiB template limit.'
    }
    try {
        $document = $content | ConvertFrom-Json -AsHashtable -NoEnumerate
    }
    catch {
        throw "parse-drift: response was not valid JSON: $($_.Exception.Message)"
    }
    $fieldPath = [string](Get-DetectorParameter -Request $request -Name 'fieldPath')
    if ([string]::IsNullOrWhiteSpace($fieldPath)) {
        throw 'Set parameters.fieldPath to a dot-separated JSON field path.'
    }
    $expectedValue = Get-DetectorParameter -Request $request -Name 'expectedValue'
    $observedValue = Get-JsonField -Document $document -Path $fieldPath
    $matched = (ConvertTo-CompactJson $observedValue) -eq (ConvertTo-CompactJson $expectedValue)
    $sourceId = [string](Get-DetectorParameter -Request $request -Name 'sourceId' -Default 'https-json')
    $evidence = [ordered]@{
        evidenceNormalizationVersion = 1
        provider = 'http-json'
        sourceId = $sourceId
        fieldPath = $fieldPath
        expectedValue = $expectedValue
        observedValue = $observedValue
        matched = $matched
    }
    $cursor = Get-OpaqueCursor $evidence
    $event = $null
    if ($matched) {
        $event = [ordered]@{
            id = New-EventId -Provider 'http-json' -Scope $sourceId -Cursor $cursor
            kind = 'http.json.condition-met'
            subject = "HTTP JSON condition '$fieldPath' matched"
            body = 'The configured read-only JSON condition matched its expected value.'
            metadata = [ordered]@{
                provider = 'http-json'
                sourceId = $sourceId
                fieldPath = $fieldPath
            }
        }
    }
    Write-SnapshotResult -Request $request -Evidence $evidence -Event $event
}
catch {
    Write-Degraded $_.Exception.Message
}
