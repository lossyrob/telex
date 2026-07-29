[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$Organization,
    [Parameter(Mandatory)]
    [string]$Project,
    [Parameter(Mandatory)]
    [string]$RepositoryId,
    [Parameter(Mandatory)]
    [int]$PullRequestId,
    [ValidateSet('bearer', 'pat')]
    [string]$Authentication = 'bearer',
    [string]$FixturePath,
    [string]$Now
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if ($FixturePath) {
    $data = Get-Content -Raw $FixturePath | ConvertFrom-Json -AsHashtable
    $pr = $data.pullRequest
}
else {
    if ($Authentication -eq 'bearer') {
        if ([string]::IsNullOrWhiteSpace($env:AZURE_DEVOPS_ACCESS_TOKEN)) {
            throw 'AZURE_DEVOPS_ACCESS_TOKEN is required for bearer preflight.'
        }
        $headers = @{ Authorization = "Bearer $($env:AZURE_DEVOPS_ACCESS_TOKEN)" }
    }
    else {
        if ([string]::IsNullOrWhiteSpace($env:AZURE_DEVOPS_EXT_PAT)) {
            throw 'AZURE_DEVOPS_EXT_PAT is required for PAT preflight.'
        }
        $encodedPat = [Convert]::ToBase64String([Text.Encoding]::ASCII.GetBytes(":$($env:AZURE_DEVOPS_EXT_PAT)"))
        $headers = @{ Authorization = "Basic $encodedPat" }
    }
    $base = "https://dev.azure.com/$([Uri]::EscapeDataString($Organization))/$([Uri]::EscapeDataString($Project))/_apis/git/repositories/$([Uri]::EscapeDataString($RepositoryId))/pullRequests/$PullRequestId"
    $pr = Invoke-RestMethod -Method Get -Headers $headers -Uri "$base`?api-version=7.1"
}
$status = [string]$pr.status
$terminal = $status -in @('completed', 'abandoned')
$evidence = [ordered]@{
    schemaVersion = 1
    provider = 'azure-devops'
    templateIds = @('azure-devops-pr')
    observedAt = $(if ($Now) { $Now } else { [DateTimeOffset]::UtcNow.ToString('o') })
    terminal = $terminal
    state = $status
    identity = [ordered]@{
        organization = $Organization
        project = $Project
        repositoryId = $RepositoryId
        pullRequestId = [int]$pr.pullRequestId
        sourceCommit = [string]$pr.lastMergeSourceCommit.commitId
    }
}
[Console]::Out.WriteLine(($evidence | ConvertTo-Json -Depth 10 -Compress))
if ($terminal) {
    [Console]::Error.WriteLine("registration aborted: Azure DevOps PR #$PullRequestId is $status.")
    exit 3
}
