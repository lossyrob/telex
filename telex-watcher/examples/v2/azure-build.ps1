#requires -Version 7.4
$ErrorActionPreference = 'Stop'
$request = [Console]::In.ReadToEnd() | ConvertFrom-Json -AsHashtable
if ($request.schemaVersion -ne 2) { throw 'Expected a v2 detector request.' }
$p = $request.watch.parameters
if ("$($p.buildId)" -notmatch '^[1-9][0-9]*$') { throw 'Set a positive buildId.' }

if ($p.fixturePath) {
    $build = Get-Content -LiteralPath $p.fixturePath -Raw | ConvertFrom-Json -AsHashtable -NoEnumerate
} else {
    if (-not $p.organization -or -not $p.project -or -not $env:TELEX_EXAMPLE_ADO_TOKEN) {
        throw 'Set organization, project, and allowlist TELEX_EXAMPLE_ADO_TOKEN (bearer token).'
    }
    $org = [Uri]::EscapeDataString($p.organization)
    $project = [Uri]::EscapeDataString($p.project)
    $uri = "https://dev.azure.com/$org/$project/_apis/build/builds/$($p.buildId)?api-version=7.1"
    $build = Invoke-RestMethod -Uri $uri -TimeoutSec 10 -MaximumRedirection 0 `
        -Headers @{ Authorization = "Bearer $env:TELEX_EXAMPLE_ADO_TOKEN" }
}
if (($build -isnot [System.Collections.IDictionary] -and $build -isnot [System.Management.Automation.PSCustomObject]) -or
    "$($build.id)" -ne "$($p.buildId)" -or
    @('none', 'notStarted', 'inProgress', 'completed', 'cancelling', 'postponed') -cnotcontains $build.status) {
    throw 'Expected the requested Azure DevOps build ID and a known status.'
}
$result = @{ schemaVersion = 2; outcome = 'idle' }
if ($build.status -ceq 'completed') {
    if (@('succeeded', 'partiallySucceeded', 'failed', 'canceled') -cnotcontains $build.result) {
        throw 'Completed build has no recognized result.'
    }
    $result.outcome = 'terminal'
    $result.event = @{
        kind = 'azure-devops.build.completed'
        subject = 'Azure DevOps build completed'
        body = "Build $($p.buildId) completed: $($build.result)."
        metadata = @{ buildId = "$($p.buildId)"; result = $build.result }
    }
}
$result | ConvertTo-Json -Depth 10 -Compress
