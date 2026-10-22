#requires -Version 7.4
$ErrorActionPreference = 'Stop'
$request = [Console]::In.ReadToEnd() | ConvertFrom-Json -AsHashtable
if ($request.schemaVersion -ne 2) { throw 'Expected a v2 detector request.' }
$p = $request.watch.parameters

if ($p.fixturePath) {
    $document = Get-Content -LiteralPath $p.fixturePath -Raw | ConvertFrom-Json -AsHashtable -NoEnumerate
} else {
    $uri = [Uri]$p.url
    if (-not $uri.IsAbsoluteUri -or $uri.Scheme -ne 'https' -or $uri.UserInfo) {
        throw 'Set url to an HTTPS JSON endpoint without embedded credentials.'
    }
    $document = Invoke-RestMethod -Uri $uri -TimeoutSec 10 -MaximumRedirection 0
}
if (($document -isnot [System.Collections.IDictionary] -and $document -isnot [System.Management.Automation.PSCustomObject]) -or
    $document.ready -isnot [bool]) { throw 'Expected a JSON object with boolean ready.' }
$result = @{ schemaVersion = 2; outcome = 'idle' }
if ($document.ready) {
    # This example intentionally emits on every successful ready observation.
    $result.outcome = 'event'
    $result.event = @{
        kind = 'http.json.ready'
        subject = 'HTTP JSON condition is ready'
        body = 'The endpoint returned ready: true.'
    }
}
$result | ConvertTo-Json -Depth 10 -Compress
