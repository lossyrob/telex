#requires -Version 7.4
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$request = [Console]::In.ReadToEnd() | ConvertFrom-Json -AsHashtable
if ($request.schemaVersion -ne 2) { throw 'Expected a v2 detector request.' }
$command = $request.watch.parameters.command
if ($command -isnot [array] -or $command.Count -eq 0 -or
    @($command | Where-Object { $_ -isnot [string] }).Count -gt 0 -or -not $command[0]) {
    throw 'Set command to a nonempty argv array of strings.'
}
if ($null -ne $request.state -and
    ($request.state -isnot [System.Collections.IDictionary] -or $request.state.ready -isnot [bool])) {
    throw 'Expected null initial state or the prior boolean ready state.'
}
$executable = (Get-Command -Name $command[0] -CommandType Application -ErrorAction Stop).Source
$arguments = @($command | Select-Object -Skip 1)
& $executable @arguments 1>$null
if ($LASTEXITCODE -notin @(0, 1)) { throw "Observation command failed (exit $LASTEXITCODE); expected 0 or 1." }
$ready = $LASTEXITCODE -eq 0
$result = @{ schemaVersion = 2; outcome = 'idle'; nextState = @{ ready = $ready } }
if ($ready -and $request.state.ready -ne $true) {
    $result.outcome = 'event'
    $result.event = @{
        kind = 'local.command.ready'
        subject = 'Local command condition became ready'
        body = 'The observational command returned exit code 0.'
    }
}
$result | ConvertTo-Json -Depth 10 -Compress
