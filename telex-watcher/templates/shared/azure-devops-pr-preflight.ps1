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
    [Parameter(DontShow)]
    [switch]$TestMode,
    [Parameter(DontShow)]
    [string]$FixturePath,
    [Parameter(DontShow)]
    [string]$Now
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Exit-PreflightFailure {
    param(
        [int]$Code,
        [string]$Diagnostic,
        [string]$Message
    )
    [Console]::Error.WriteLine("preflightDiagnostic=$(@{
        schemaVersion = 1
        code = $Diagnostic
        message = $Message
    } | ConvertTo-Json -Compress)")
    exit $Code
}

if (($FixturePath -or $Now) -and -not $TestMode) {
    Exit-PreflightFailure -Code 5 -Diagnostic 'test-mode-required' -Message 'FixturePath and Now are test-only and require -TestMode.'
}
if ($Now -and $Now -notmatch '^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$') {
    Exit-PreflightFailure -Code 5 -Diagnostic 'invalid-rfc3339' -Message 'Now must be an RFC3339 timestamp.'
}
if ($Now) {
    try {
        [void][DateTimeOffset]::Parse(
            $Now,
            [Globalization.CultureInfo]::InvariantCulture,
            [Globalization.DateTimeStyles]::RoundtripKind
        )
    }
    catch {
        Exit-PreflightFailure -Code 5 -Diagnostic 'invalid-rfc3339' -Message 'Now must be an RFC3339 timestamp.'
    }
}

if ($FixturePath) {
    try {
        $data = Get-Content -Raw $FixturePath | ConvertFrom-Json -AsHashtable
        $pr = $data.pullRequest
    }
    catch {
        Exit-PreflightFailure -Code 5 -Diagnostic 'fixture-parse-failure' -Message $_.Exception.Message
    }
}
else {
    if ($Authentication -eq 'bearer') {
        if ([string]::IsNullOrWhiteSpace($env:AZURE_DEVOPS_ACCESS_TOKEN)) {
            Exit-PreflightFailure -Code 4 -Diagnostic 'provider-auth-transport-failure' -Message 'AZURE_DEVOPS_ACCESS_TOKEN is required for bearer preflight.'
        }
        $headers = @{ Authorization = "Bearer $($env:AZURE_DEVOPS_ACCESS_TOKEN)" }
    }
    else {
        if ([string]::IsNullOrWhiteSpace($env:AZURE_DEVOPS_EXT_PAT)) {
            Exit-PreflightFailure -Code 4 -Diagnostic 'provider-auth-transport-failure' -Message 'AZURE_DEVOPS_EXT_PAT is required for PAT preflight.'
        }
        $encodedPat = [Convert]::ToBase64String([Text.Encoding]::ASCII.GetBytes(":$($env:AZURE_DEVOPS_EXT_PAT)"))
        $headers = @{ Authorization = "Basic $encodedPat" }
    }
    $base = "https://dev.azure.com/$([Uri]::EscapeDataString($Organization))/$([Uri]::EscapeDataString($Project))/_apis/git/repositories/$([Uri]::EscapeDataString($RepositoryId))/pullRequests/$PullRequestId"
    try {
        $pr = Invoke-RestMethod -Method Get -Headers $headers -Uri "$base`?api-version=7.1"
    }
    catch {
        Exit-PreflightFailure -Code 4 -Diagnostic 'provider-auth-transport-failure' -Message $_.Exception.Message
    }
}

try {
    $status = [string]$pr.status
    $terminal = $status -in @('completed', 'abandoned')
    $lastMergeSourceCommit = if ($pr -is [System.Collections.IDictionary] -and $pr.Contains('lastMergeSourceCommit')) {
        $pr.lastMergeSourceCommit
    }
    else {
        $null
    }
    $sourceCommit = if ($lastMergeSourceCommit -is [System.Collections.IDictionary] -and $lastMergeSourceCommit.Contains('commitId')) {
        [string]$lastMergeSourceCommit.commitId
    }
    else {
        ''
    }
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
            sourceCommit = $sourceCommit
        }
    }
    [Console]::Out.WriteLine(($evidence | ConvertTo-Json -Depth 10 -Compress))
}
catch {
    Exit-PreflightFailure -Code 5 -Diagnostic 'provider-shape-failure' -Message $_.Exception.Message
}
if ($terminal) {
    [Console]::Error.WriteLine("registration aborted: Azure DevOps PR #$PullRequestId is $status.")
    exit 3
}
exit 0
