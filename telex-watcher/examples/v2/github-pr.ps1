#requires -Version 7.4
$ErrorActionPreference = 'Stop'
$request = [Console]::In.ReadToEnd() | ConvertFrom-Json -AsHashtable
if ($request.schemaVersion -ne 2) { throw 'Expected a v2 detector request.' }
$p = $request.watch.parameters
if ($null -ne $request.state -and
    ($request.state -isnot [System.Collections.IDictionary] -or $request.state.snapshot -isnot [string])) {
    throw 'Expected null initial state or the prior snapshot state.'
}

if ($p.fixturePath) {
    $pr = Get-Content -LiteralPath $p.fixturePath -Raw | ConvertFrom-Json -AsHashtable -NoEnumerate
} else {
    if ($p.repository -notmatch '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$' -or
        "$($p.pullRequestNumber)" -notmatch '^[1-9][0-9]*$') {
        throw 'Set repository (owner/repo) and a positive pullRequestNumber.'
    }
    $raw = & gh pr view "$($p.pullRequestNumber)" --repo $p.repository --json 'state,headRefOid,isDraft'
    if ($LASTEXITCODE -ne 0) { throw 'gh pr view failed; check authentication and repository access.' }
    $pr = $raw | ConvertFrom-Json -AsHashtable -NoEnumerate
}
if ($pr -isnot [System.Collections.IDictionary] -or
    @('OPEN', 'CLOSED', 'MERGED') -cnotcontains $pr.state -or
    $pr.headRefOid -cnotmatch '^[0-9a-f]{40}$' -or $pr.isDraft -isnot [bool]) {
    throw 'Expected a GitHub PR state, full head SHA, and boolean isDraft.'
}
$snapshot = [ordered]@{ state = $pr.state; head = $pr.headRefOid; draft = $pr.isDraft }
$cursor = $snapshot | ConvertTo-Json -Compress
$result = @{ schemaVersion = 2; outcome = 'idle'; nextState = @{ snapshot = $cursor } }
if ($request.state.snapshot -cne $cursor) {
    $result.outcome = 'event'
    $result.event = @{
        kind = 'github.pull-request.snapshot'
        subject = 'GitHub pull request changed'
        body = $cursor
        metadata = $snapshot
    }
}
if ($pr.state -cne 'OPEN') { $result.outcome = 'terminal' }
$result | ConvertTo-Json -Depth 10 -Compress
