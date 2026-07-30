[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$helperPath = Join-Path $PSScriptRoot '..\shared\DetectorCommon.psm1'
$expectedHelperSha256 = '03072d00f5b343d6a19c5fe40c7365c6286fea5035546763a8c753b0399cf189'
if ((Get-FileHash $helperPath -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expectedHelperSha256) {
    [Console]::Error.WriteLine('detectorDiagnostic={"schemaVersion":1,"code":"shared-helper-digest-mismatch","message":"Pinned shared helper digest mismatch."}')
    [Console]::Out.WriteLine('{"schemaVersion":1,"outcome":"degraded"}')
    exit 0
}
Import-Module $helperPath -Force

function Get-JsonFieldResult {
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
        return [ordered]@{ present = $false; value = $null }
    }
    return [ordered]@{ present = $true; value = $value }
}

function Test-JsonScalar {
    param($Value)
    return $null -eq $Value -or (
        $Value -isnot [System.Collections.IDictionary] -and
        $Value -isnot [System.Collections.IList]
    )
}

function Invoke-HttpJsonGet {
    param(
        [hashtable]$Request,
        [string]$Uri,
        [hashtable]$Headers,
        [string]$FixtureContent
    )

    $capturePath = [string](Get-DetectorParameter -Request $Request -Name 'testTransportCapturePath' -Default '')
    if (-not [string]::IsNullOrWhiteSpace($capturePath)) {
        Write-TestTransportRecord -Request $Request -Record ([ordered]@{
            transport = 'https'
            method = 'GET'
            uri = $Uri
            headers = $Headers
            body = $null
            maximumRedirection = 0
            timeoutSeconds = 20
        })
        return [pscustomobject]@{ Content = $FixtureContent }
    }
    return Invoke-WebRequest -Method Get -Uri $Uri -Headers $Headers -MaximumRedirection 0 -TimeoutSec 20
}

try {
    $request = Read-DetectorRequest
    Assert-DetectorTestMode -Request $request -ParameterNames @('fixturePath', 'testTransportCapturePath')
    $fixturePath = [string](Get-DetectorParameter -Request $request -Name 'fixturePath' -Default '')
    $capturePath = [string](Get-DetectorParameter -Request $request -Name 'testTransportCapturePath' -Default '')
    $fixtureContent = $null
    if (-not [string]::IsNullOrWhiteSpace($fixturePath)) {
        $fixtureContent = Get-Content -Raw (Resolve-DetectorPath $fixturePath)
        if ([string]::IsNullOrWhiteSpace($capturePath)) {
            $content = $fixtureContent
        }
    }

    if ([string]::IsNullOrWhiteSpace($fixturePath) -or -not [string]::IsNullOrWhiteSpace($capturePath)) {
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
        if (-not [string]::IsNullOrWhiteSpace($capturePath) -and $null -eq $fixtureContent) {
            throw 'test-transport-invalid: fixturePath is required with testTransportCapturePath.'
        }
        try {
            $response = Invoke-HttpJsonGet -Request $request -Uri $url -Headers $headers -FixtureContent $fixtureContent
        }
        catch {
            $status = 0
            $responseProperty = $_.Exception.PSObject.Properties['Response']
            if ($null -ne $responseProperty -and $null -ne $responseProperty.Value) {
                $status = [int]$responseProperty.Value.StatusCode
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
            if ($_.Exception.Message -match '^(?:test-transport-invalid|missing-credential|credential-policy|transport-policy):') {
                throw
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
        throw 'configuration-invalid: set parameters.fieldPath to a dot-separated JSON field path.'
    }
    $parameters = $request.watch.parameters
    if ($parameters -isnot [System.Collections.IDictionary] -or -not $parameters.Contains('expectedValue')) {
        throw 'configuration-invalid: parameters.expectedValue is required and may be an explicit JSON null.'
    }
    $expectedValue = $parameters['expectedValue']
    if (-not (Test-JsonScalar -Value $expectedValue)) {
        throw 'configuration-invalid: parameters.expectedValue must be a JSON scalar or null.'
    }
    $fieldResult = Get-JsonFieldResult -Document $document -Path $fieldPath
    $observedValue = $fieldResult.value
    $matched = [bool]$fieldResult.present -and (ConvertTo-CanonicalJson $observedValue) -eq (ConvertTo-CanonicalJson $expectedValue)
    $sourceId = [string](Get-DetectorParameter -Request $request -Name 'sourceId' -Default 'https-json')
    $evidence = [ordered]@{
        evidenceNormalizationVersion = 2
        provider = 'http-json'
        sourceId = $sourceId
        fieldPath = $fieldPath
        expectedValue = $expectedValue
        fieldPresent = [bool]$fieldResult.present
        observedValue = $observedValue
        matched = $matched
    }
    $cursor = Get-OpaqueCursor $evidence
    $event = $null
    if ($matched) {
        $event = [ordered]@{
            id = New-EventId -Provider 'http-json' -Scope $sourceId -Cursor $cursor -Request $request
            kind = 'http.json.condition-met'
            subject = "HTTP JSON condition '$fieldPath' matched"
            body = 'The configured read-only JSON scalar condition matched its expected value.'
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
