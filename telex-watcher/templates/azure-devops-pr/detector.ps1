[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$helperPath = Join-Path $PSScriptRoot '..\shared\DetectorCommon.psm1'
$expectedHelperSha256 = '611f0dc780fd771db29cd95187c6d79d9b527ea1581f3dbb466ccc8883bc8428'
if ((Get-FileHash $helperPath -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expectedHelperSha256) {
    [Console]::Error.WriteLine('detector degraded: shared-helper-digest-mismatch')
    [Console]::Out.WriteLine('{"schemaVersion":1,"outcome":"degraded"}')
    exit 0
}
Import-Module $helperPath -Force

function Get-AzureDevOpsPrData {
    param([hashtable]$Request)

    $fixturePath = Get-DetectorParameter -Request $Request -Name 'fixturePath'
    if ($fixturePath) {
        return Get-Content -Raw (Resolve-DetectorPath ([string]$fixturePath)) | ConvertFrom-Json -AsHashtable
    }

    $organization = [string](Get-DetectorParameter -Request $Request -Name 'organization')
    $project = [string](Get-DetectorParameter -Request $Request -Name 'project')
    $repositoryId = [string](Get-DetectorParameter -Request $Request -Name 'repositoryId')
    $pullRequestId = Get-DetectorParameter -Request $Request -Name 'pullRequestId'
    if ([string]::IsNullOrWhiteSpace($organization) -or [string]::IsNullOrWhiteSpace($project) -or [string]::IsNullOrWhiteSpace($repositoryId) -or $null -eq $pullRequestId) {
        throw 'Set organization, project, repositoryId, and pullRequestId, or provide fixturePath.'
    }
    $allowPat = [bool](Get-DetectorParameter -Request $Request -Name 'allowPatAuthentication' -Default $false)
    $allowBearer = [bool](Get-DetectorParameter -Request $Request -Name 'allowBearerAuthentication' -Default $false)
    if ($allowPat -and $allowBearer) {
        throw 'allowPatAuthentication and allowBearerAuthentication are mutually exclusive.'
    }
    if ($allowBearer) {
        if ([string]::IsNullOrWhiteSpace($env:AZURE_DEVOPS_ACCESS_TOKEN)) {
            throw 'AZURE_DEVOPS_ACCESS_TOKEN was not supplied by the explicit environment allowlist.'
        }
        $headers = @{ Authorization = "Bearer $($env:AZURE_DEVOPS_ACCESS_TOKEN)" }
    }
    elseif ($allowPat) {
        if ([string]::IsNullOrWhiteSpace($env:AZURE_DEVOPS_EXT_PAT)) {
            throw 'AZURE_DEVOPS_EXT_PAT was not supplied by the explicit environment allowlist.'
        }
        $encodedPat = [Convert]::ToBase64String([Text.Encoding]::ASCII.GetBytes(":$($env:AZURE_DEVOPS_EXT_PAT)"))
        $headers = @{ Authorization = "Basic $encodedPat" }
    }
    else {
        throw 'Set exactly one of allowBearerAuthentication or allowPatAuthentication and allowlist its matching environment variable.'
    }
    $base = "https://dev.azure.com/$([Uri]::EscapeDataString($organization))/$([Uri]::EscapeDataString($project))/_apis/git/repositories/$([Uri]::EscapeDataString($repositoryId))/pullRequests/$pullRequestId"
    $pr = Invoke-RestMethod -Method Get -Headers $headers -Uri "$base`?api-version=7.1"
    $threads = Invoke-RestMethod -Method Get -Headers $headers -Uri "$base/threads?api-version=7.1"
    return @{ pullRequest = $pr; threads = @($threads.value) }
}

try {
    $request = Read-DetectorRequest
    $data = Get-AzureDevOpsPrData -Request $request
    $pr = $data.pullRequest
    $reviewers = @($pr.reviewers | ForEach-Object {
        [ordered]@{
            id = [string](Get-OptionalValue -Object $_ -Name 'id' -Default '')
            displayName = [string](Get-OptionalValue -Object $_ -Name 'displayName' -Default '')
            vote = [int](Get-OptionalValue -Object $_ -Name 'vote' -Default 0)
            required = [bool](Get-OptionalValue -Object $_ -Name 'isRequired' -Default $false)
        }
    } | Sort-Object id)
    $threads = @($data.threads | ForEach-Object {
        [ordered]@{
            id = [int](Get-OptionalValue -Object $_ -Name 'id' -Default 0)
            status = [string](Get-OptionalValue -Object $_ -Name 'status' -Default '')
            isDeleted = [bool](Get-OptionalValue -Object $_ -Name 'isDeleted' -Default $false)
        }
    } | Sort-Object id)
    $blockingVotes = @($reviewers | Where-Object { $_.vote -le -5 })
    $creationDate = ([DateTimeOffset]$pr.creationDate).ToUniversalTime().ToString('o')
    $reason = $null
    $kind = $null
    $terminal = $false
    if ([string]$pr.status -in @('completed', 'abandoned')) {
        $reason = "pull request is $($pr.status)"
        $kind = 'azure-devops.pull-request.completed'
        $terminal = $true
    }
    elseif ([string]$pr.mergeStatus -eq 'conflicts') {
        $reason = 'merge status is conflicts'
        $kind = 'azure-devops.pull-request.attention'
    }
    elseif ($blockingVotes.Count -gt 0) {
        $reason = "blocking reviewer vote(s): $(@($blockingVotes | ForEach-Object { $_.displayName }) -join ', ')"
        $kind = 'azure-devops.pull-request.attention'
    }
    elseif ([string]$pr.status -eq 'active' -and -not [bool]$pr.isDraft -and @($reviewers | Where-Object { $_.required }).Count -gt 0 -and @($reviewers | Where-Object { $_.required -and $_.vote -lt 5 }).Count -eq 0 -and [string]$pr.mergeStatus -eq 'succeeded') {
        $reason = 'required reviewers approved and merge status succeeded'
        $kind = 'azure-devops.pull-request.ready-to-merge'
    }

    $evidence = [ordered]@{
        evidenceNormalizationVersion = 1
        provider = 'azure-devops'
        organization = [string](Get-DetectorParameter -Request $request -Name 'organization' -Default '')
        project = [string](Get-DetectorParameter -Request $request -Name 'project' -Default '')
        repositoryId = [string](Get-DetectorParameter -Request $request -Name 'repositoryId' -Default '')
        pullRequestId = [int]$pr.pullRequestId
        creationDate = $creationDate
        status = [string]$pr.status
        draft = [bool]$pr.isDraft
        mergeStatus = [string]$pr.mergeStatus
        sourceCommit = [string]$pr.lastMergeSourceCommit.commitId
        reviewers = $reviewers
        threads = $threads
    }
    $cursor = Get-OpaqueCursor $evidence
    $preflight = Get-PreflightEvidence -Request $request
    if ($null -eq (Get-StateCursor $request) -and $preflight -is [System.Collections.IDictionary]) {
        if (
            [string]$preflight.provider -ne 'azure-devops' -or
            [string]$preflight.identity.organization -ne [string](Get-DetectorParameter -Request $request -Name 'organization' -Default '') -or
            [string]$preflight.identity.project -ne [string](Get-DetectorParameter -Request $request -Name 'project' -Default '') -or
            [string]$preflight.identity.repositoryId -ne [string](Get-DetectorParameter -Request $request -Name 'repositoryId' -Default '') -or
            [int]$preflight.identity.pullRequestId -ne [int]$pr.pullRequestId
        ) {
            throw 'preflight-identity-mismatch: Azure DevOps evidence does not match this watch.'
        }
        if (-not [bool]$preflight.terminal -and $terminal) {
            Write-EventlessTerminal -Evidence $evidence
            return
        }
    }
    $event = $null
    if ($null -eq $kind -and [bool](Get-DetectorParameter -Request $request -Name 'emitInitialCreatedEvent' -Default $false) -and $null -eq (Get-StateCursor $request)) {
        $reason = "pull request was created at $creationDate"
        $kind = 'azure-devops.pull-request.created'
    }
    if ($null -eq $kind -and [bool](Get-DetectorParameter -Request $request -Name 'emitInitialSnapshot' -Default $false) -and $null -eq (Get-StateCursor $request)) {
        $reason = 'initial read-only snapshot'
        $kind = 'azure-devops.pull-request.snapshot'
    }
    if ($kind) {
        $event = [ordered]@{
            id = New-EventId -Provider 'azure-devops-pr' -Scope ([string]$pr.pullRequestId) -Cursor $cursor
            kind = $kind
            subject = "Azure DevOps PR #$($pr.pullRequestId): $($pr.title)"
            body = $reason
            metadata = [ordered]@{
                provider = 'azure-devops'
                pullRequestId = [int]$pr.pullRequestId
                creationDate = $creationDate
                status = [string]$pr.status
                mergeStatus = [string]$pr.mergeStatus
                blockingReviewers = @($blockingVotes | ForEach-Object { $_.displayName })
                threadCount = $threads.Count
            }
        }
    }
    Write-SnapshotResult -Request $request -Evidence $evidence -Event $event -Terminal:$terminal
}
catch {
    Write-Degraded $_.Exception.Message
}
