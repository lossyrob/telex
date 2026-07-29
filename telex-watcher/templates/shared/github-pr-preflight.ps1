[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$Repository,
    [Parameter(Mandatory)]
    [int]$PullRequestNumber,
    [string]$FixturePath,
    [string]$Now
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if ($FixturePath) {
    $pr = Get-Content -Raw $FixturePath | ConvertFrom-Json -AsHashtable
}
else {
    $raw = & gh pr view $PullRequestNumber --repo $Repository --json number,state,headRefOid 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw "gh pr view failed: $($raw -join [Environment]::NewLine)"
    }
    $pr = ($raw -join [Environment]::NewLine) | ConvertFrom-Json -AsHashtable
}
$state = [string]$pr.state
$terminal = $state -in @('MERGED', 'CLOSED')
$evidence = [ordered]@{
    schemaVersion = 1
    provider = 'github'
    templateIds = @('github-pr', 'github-pr-external-activity')
    observedAt = $(if ($Now) { $Now } else { [DateTimeOffset]::UtcNow.ToString('o') })
    terminal = $terminal
    state = $state
    identity = [ordered]@{
        repository = $Repository
        pullRequestNumber = [int]$pr.number
        headSha = [string]$pr.headRefOid
    }
}
[Console]::Out.WriteLine(($evidence | ConvertTo-Json -Depth 10 -Compress))
if ($terminal) {
    [Console]::Error.WriteLine("registration aborted: GitHub PR #$PullRequestNumber is $($state.ToLowerInvariant()).")
    exit 3
}
