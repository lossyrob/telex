[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$helperPath = Join-Path $PSScriptRoot '..\shared\DetectorCommon.psm1'
$expectedHelperSha256 = '03072d00f5b343d6a19c5fe40c7365c6286fea5035546763a8c753b0399cf189'
if ((Get-FileHash $helperPath -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expectedHelperSha256) {
    [Console]::Error.WriteLine('detectorDiagnostic={"schemaVersion":1,"code":"shared-helper-digest-mismatch","message":"Pinned shared helper digest mismatch."}')
    [Console]::Out.WriteLine('{"schemaVersion":1,"outcome":"degraded"}')
    exit 0
}
Import-Module $helperPath -Force

function Invoke-AzureDevOpsGet {
    param(
        [hashtable]$Request,
        [string]$Uri,
        [hashtable]$Headers,
        [System.Collections.IDictionary]$Fixture,
        [ValidateSet('pullRequest', 'threads')]
        [string]$ResponseKind
    )

    $capturePath = [string](Get-DetectorParameter -Request $Request -Name 'testTransportCapturePath' -Default '')
    if (-not [string]::IsNullOrWhiteSpace($capturePath)) {
        Write-TestTransportRecord -Request $Request -Record ([ordered]@{
            transport = 'https'
            method = 'GET'
            uri = $Uri
            headers = $Headers
            body = $null
        })
        if ($ResponseKind -eq 'pullRequest') {
            return $Fixture.pullRequest
        }
        return @{ value = @($Fixture.threads) }
    }
    return Invoke-RestMethod -Method Get -Headers $Headers -Uri $Uri
}

function Get-AzureDevOpsPrData {
    param([hashtable]$Request)

    Assert-DetectorTestMode -Request $Request -ParameterNames @('fixturePath', 'testTransportCapturePath')
    $fixturePath = [string](Get-DetectorParameter -Request $Request -Name 'fixturePath' -Default '')
    $capturePath = [string](Get-DetectorParameter -Request $Request -Name 'testTransportCapturePath' -Default '')
    $fixture = $null
    if (-not [string]::IsNullOrWhiteSpace($fixturePath)) {
        $fixture = Get-Content -Raw (Resolve-DetectorPath $fixturePath) | ConvertFrom-Json -AsHashtable
        if ([string]::IsNullOrWhiteSpace($capturePath)) {
            return $fixture
        }
    }

    $organization = [string](Get-DetectorParameter -Request $Request -Name 'organization')
    $project = [string](Get-DetectorParameter -Request $Request -Name 'project')
    $repositoryId = [string](Get-DetectorParameter -Request $Request -Name 'repositoryId')
    $pullRequestId = Get-DetectorParameter -Request $Request -Name 'pullRequestId'
    if (
        [string]::IsNullOrWhiteSpace($organization) -or $organization -eq 'AZURE-DEVOPS-ORGANIZATION' -or
        [string]::IsNullOrWhiteSpace($project) -or $project -eq 'AZURE-DEVOPS-PROJECT' -or
        [string]::IsNullOrWhiteSpace($repositoryId) -or $repositoryId -eq 'AZURE-DEVOPS-REPOSITORY-ID' -or
        $null -eq $pullRequestId
    ) {
        throw 'configuration-invalid: set concrete organization, project, repositoryId, and pullRequestId values.'
    }
    $allowPat = [bool](Get-DetectorParameter -Request $Request -Name 'allowPatAuthentication' -Default $false)
    $allowBearer = [bool](Get-DetectorParameter -Request $Request -Name 'allowBearerAuthentication' -Default $false)
    if ($allowPat -eq $allowBearer) {
        throw 'credential-policy: set exactly one of allowBearerAuthentication or allowPatAuthentication.'
    }
    if ($allowBearer) {
        if ([string]::IsNullOrWhiteSpace($env:AZURE_DEVOPS_ACCESS_TOKEN)) {
            throw 'missing-credential: AZURE_DEVOPS_ACCESS_TOKEN was not supplied by the explicit environment allowlist.'
        }
        $headers = @{ Authorization = "Bearer $($env:AZURE_DEVOPS_ACCESS_TOKEN)" }
    }
    else {
        if ([string]::IsNullOrWhiteSpace($env:AZURE_DEVOPS_EXT_PAT)) {
            throw 'missing-credential: AZURE_DEVOPS_EXT_PAT was not supplied by the explicit environment allowlist.'
        }
        $encodedPat = [Convert]::ToBase64String([Text.Encoding]::ASCII.GetBytes(":$($env:AZURE_DEVOPS_EXT_PAT)"))
        $headers = @{ Authorization = "Basic $encodedPat" }
    }
    if (-not [string]::IsNullOrWhiteSpace($capturePath) -and $null -eq $fixture) {
        throw 'test-transport-invalid: fixturePath is required with testTransportCapturePath.'
    }
    $base = "https://dev.azure.com/$([Uri]::EscapeDataString($organization))/$([Uri]::EscapeDataString($project))/_apis/git/repositories/$([Uri]::EscapeDataString($repositoryId))/pullRequests/$pullRequestId"
    $pr = Invoke-AzureDevOpsGet -Request $Request -Headers $headers -Uri "$base`?api-version=7.1" -Fixture $fixture -ResponseKind pullRequest
    $threads = Invoke-AzureDevOpsGet -Request $Request -Headers $headers -Uri "$base/threads?api-version=7.1" -Fixture $fixture -ResponseKind threads
    return @{ pullRequest = $pr; threads = @($threads.value) }
}

function Test-AzureDevOpsPreflight {
    param(
        [System.Collections.IDictionary]$Preflight,
        [string]$Organization,
        [string]$Project,
        [string]$RepositoryId,
        [int]$PullRequestId
    )

    $identity = Get-OptionalValue -Object $Preflight -Name 'identity'
    if (
        [int](Get-OptionalValue -Object $Preflight -Name 'schemaVersion' -Default 0) -ne 1 -or
        [string](Get-OptionalValue -Object $Preflight -Name 'provider' -Default '') -ne 'azure-devops' -or
        -not (Test-Rfc3339Timestamp -Value (Get-OptionalValue -Object $Preflight -Name 'observedAt' -Default '')) -or
        'azure-devops-pr' -notin @((Get-OptionalValue -Object $Preflight -Name 'templateIds' -Default @())) -or
        [string](Get-OptionalValue -Object $identity -Name 'organization' -Default '') -ne $Organization -or
        [string](Get-OptionalValue -Object $identity -Name 'project' -Default '') -ne $Project -or
        [string](Get-OptionalValue -Object $identity -Name 'repositoryId' -Default '') -ne $RepositoryId -or
        [int](Get-OptionalValue -Object $identity -Name 'pullRequestId' -Default 0) -ne $PullRequestId
    ) {
        return $false
    }
    return $true
}

try {
    $request = Read-DetectorRequest
    $data = Get-AzureDevOpsPrData -Request $request
    $pr = $data.pullRequest
    $reviewers = [object[]]@(@((Get-OptionalValue -Object $pr -Name 'reviewers' -Default @())) | ForEach-Object {
        [ordered]@{
            id = [string](Get-OptionalValue -Object $_ -Name 'id' -Default '')
            displayName = [string](Get-OptionalValue -Object $_ -Name 'displayName' -Default '')
            vote = [int](Get-OptionalValue -Object $_ -Name 'vote' -Default 0)
            required = [bool](Get-OptionalValue -Object $_ -Name 'isRequired' -Default $false)
        }
    })
    [Array]::Sort[object]($reviewers, [System.Comparison[object]]{
        param($left, $right)
        return [StringComparer]::Ordinal.Compare([string]$left.id, [string]$right.id)
    })
    $threads = [object[]]@(@($data.threads) | ForEach-Object {
        [ordered]@{
            id = [int](Get-OptionalValue -Object $_ -Name 'id' -Default 0)
            status = [string](Get-OptionalValue -Object $_ -Name 'status' -Default '')
            isDeleted = [bool](Get-OptionalValue -Object $_ -Name 'isDeleted' -Default $false)
        }
    })
    [Array]::Sort[object]($threads, [System.Comparison[object]]{
        param($left, $right)
        return ([int64]$left.id).CompareTo([int64]$right.id)
    })
    $blockingVoteAtMost = [int](Get-DetectorParameter -Request $request -Name 'blockingReviewerVoteAtMost' -Default -10)
    if ($blockingVoteAtMost -gt -5 -or $blockingVoteAtMost -lt -10) {
        throw 'configuration-invalid: blockingReviewerVoteAtMost must be between -10 and -5.'
    }
    $blockingVotes = @($reviewers | Where-Object { $_.vote -le $blockingVoteAtMost })
    $creationDateValue = Get-OptionalValue -Object $pr -Name 'creationDate' -Default ''
    if ([string]::IsNullOrWhiteSpace([string]$creationDateValue)) {
        throw 'parse-drift: pull request creationDate is missing.'
    }
    $creationDate = ConvertTo-Rfc3339Utc -Value $creationDateValue
    $status = [string](Get-OptionalValue -Object $pr -Name 'status' -Default '')
    $mergeStatus = [string](Get-OptionalValue -Object $pr -Name 'mergeStatus' -Default '')
    $draft = [bool](Get-OptionalValue -Object $pr -Name 'isDraft' -Default $false)
    $lastMergeSourceCommit = Get-OptionalValue -Object $pr -Name 'lastMergeSourceCommit'
    $sourceCommit = [string](Get-OptionalValue -Object $lastMergeSourceCommit -Name 'commitId' -Default '')
    $reason = $null
    $kind = $null
    $terminal = $false
    if ($status -in @('completed', 'abandoned')) {
        $reason = "pull request is $status"
        $kind = 'azure-devops.pull-request.completed'
        $terminal = $true
    }
    elseif ($mergeStatus -eq 'conflicts') {
        $reason = 'merge status is conflicts'
        $kind = 'azure-devops.pull-request.attention'
    }
    elseif ($blockingVotes.Count -gt 0) {
        $reason = "blocking reviewer vote(s): $(@($blockingVotes | ForEach-Object { $_.displayName }) -join ', ')"
        $kind = 'azure-devops.pull-request.attention'
    }
    elseif ($status -eq 'active' -and -not $draft -and @($reviewers | Where-Object { $_.required }).Count -gt 0 -and @($reviewers | Where-Object { $_.required -and $_.vote -lt 5 }).Count -eq 0 -and $mergeStatus -eq 'succeeded') {
        $reason = 'required reviewers approved and merge status succeeded'
        $kind = 'azure-devops.pull-request.ready-to-merge'
    }

    $evidence = [ordered]@{
        evidenceNormalizationVersion = 3
        provider = 'azure-devops'
        organization = [string](Get-DetectorParameter -Request $request -Name 'organization' -Default '')
        project = [string](Get-DetectorParameter -Request $request -Name 'project' -Default '')
        repositoryId = [string](Get-DetectorParameter -Request $request -Name 'repositoryId' -Default '')
        pullRequestId = [int](Get-OptionalValue -Object $pr -Name 'pullRequestId' -Default 0)
        creationDate = $creationDate
        status = $status
        draft = $draft
        mergeStatus = $mergeStatus
        sourceCommit = $sourceCommit
        reviewers = $reviewers
        threads = $threads
    }
    $cursor = Get-OpaqueCursor $evidence
    $preflight = Get-PreflightEvidence -Request $request
    if ($null -eq (Get-StateCursor $request) -and $preflight -is [System.Collections.IDictionary]) {
        $organization = [string](Get-DetectorParameter -Request $request -Name 'organization' -Default '')
        $project = [string](Get-DetectorParameter -Request $request -Name 'project' -Default '')
        $repositoryId = [string](Get-DetectorParameter -Request $request -Name 'repositoryId' -Default '')
        if (-not (Test-AzureDevOpsPreflight -Preflight $preflight -Organization $organization -Project $project -RepositoryId $repositoryId -PullRequestId ([int]$pr.pullRequestId))) {
            Write-Degraded -Code 'preflight-identity-mismatch' -Message 'Azure DevOps preflight evidence does not match this watch, template, or RFC3339 timestamp.'
            return
        }
        if ([bool](Get-OptionalValue -Object $preflight -Name 'terminal' -Default $false)) {
            Write-EventlessTerminal -Request $request -Evidence $evidence
            return
        }
        if ($terminal) {
            Write-EventlessTerminal -Request $request -Evidence $evidence
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
            id = New-EventId -Provider 'azure-devops-pr' -Scope ([string]$pr.pullRequestId) -Cursor $cursor -Request $request
            kind = $kind
            subject = "Azure DevOps PR #$($pr.pullRequestId): $($pr.title)"
            body = $reason
            metadata = [ordered]@{
                provider = 'azure-devops'
                pullRequestId = [int]$pr.pullRequestId
                creationDate = $creationDate
                status = $status
                mergeStatus = $mergeStatus
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
