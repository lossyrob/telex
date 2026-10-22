param([string]$Detector)
$ErrorActionPreference = 'Stop'

function gh {
    if (($args -join '|') -cne 'pr|view|123|--repo|OWNER/REPO|--json|state,headRefOid,isDraft') {
        throw 'Unexpected GitHub observation arguments.'
    }
    $global:LASTEXITCODE = [int]$env:V2_TEST_EXIT
    if ($global:LASTEXITCODE -eq 0) { Get-Content -LiteralPath $env:V2_TEST_RESPONSE -Raw }
}

function Invoke-RestMethod {
    param([Uri]$Uri, [int]$TimeoutSec, [int]$MaximumRedirection, [hashtable]$Headers)
    if ($TimeoutSec -ne 10 -or $MaximumRedirection -ne 0) { throw 'Unbounded HTTP request.' }
    if ($Uri.AbsoluteUri -ceq 'https://example.invalid/status.json') {
        if ($Headers) { throw 'Unexpected HTTP credentials.' }
    } elseif ($Uri.AbsoluteUri -ceq 'https://dev.azure.com/ORG/Project%20Name/_apis/build/builds/42?api-version=7.1') {
        if ($Headers.Authorization -cne 'Bearer fixture-token') { throw 'Missing explicit ADO bearer token.' }
    } else { throw 'Unexpected observation URL.' }
    if ($env:V2_TEST_EXIT -ne '0') { throw 'Fixture HTTP request failed.' }
    Get-Content -LiteralPath $env:V2_TEST_RESPONSE -Raw | ConvertFrom-Json
}

. $Detector
