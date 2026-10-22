#requires -Version 7.4
$ErrorActionPreference = 'Stop'
$request = [Console]::In.ReadToEnd() | ConvertFrom-Json -AsHashtable
if ($request.schemaVersion -ne 2) { throw 'Expected a v2 detector request.' }
$path = $request.watch.parameters.path
if (-not $path) { throw 'Set path to the JSON file to observe.' }
$document = Get-Content -LiteralPath $path -Raw | ConvertFrom-Json -AsHashtable -NoEnumerate
if ($document -isnot [System.Collections.IDictionary] -or
    $document.ready -isnot [bool]) { throw 'Expected a JSON object with boolean ready.' }
if ($null -ne $request.state -and
    ($request.state -isnot [System.Collections.IDictionary] -or $request.state.ready -isnot [bool])) {
    throw 'Expected null initial state or the prior boolean ready state.'
}
$result = @{ schemaVersion = 2; outcome = 'idle'; nextState = @{ ready = $document.ready } }
if ($document.ready -and $request.state.ready -ne $true) {
    $result.outcome = 'event'
    $result.event = @{
        kind = 'local.file-json.ready'
        subject = 'Local JSON condition became ready'
        body = 'The file changed to ready: true.'
    }
}
$result | ConvertTo-Json -Depth 10 -Compress
