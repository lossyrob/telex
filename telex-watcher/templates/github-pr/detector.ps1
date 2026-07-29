[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$helperPath = Join-Path $PSScriptRoot '..\shared\DetectorCommon.psm1'
$expectedHelperSha256 = 'd7fcef49f32f4057a2495f741d5ecc5e8146ba4609f401723f2d753a71d37c0c'
if ((Get-FileHash $helperPath -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expectedHelperSha256) {
    [Console]::Error.WriteLine('detectorDiagnostic={"schemaVersion":1,"code":"shared-helper-digest-mismatch","message":"Pinned shared helper digest mismatch."}')
    [Console]::Out.WriteLine('{"schemaVersion":1,"outcome":"degraded"}')
    exit 0
}
Import-Module $helperPath -Force

function Get-GitHubPrData {
    param(
        [hashtable]$Request,
        [string[]]$Fields
    )

    Assert-DetectorTestMode -Request $Request -ParameterNames @('fixturePath', 'testTransportCapturePath')
    $fixturePath = [string](Get-DetectorParameter -Request $Request -Name 'fixturePath' -Default '')
    $capturePath = [string](Get-DetectorParameter -Request $Request -Name 'testTransportCapturePath' -Default '')
    if (-not [string]::IsNullOrWhiteSpace($fixturePath) -and [string]::IsNullOrWhiteSpace($capturePath)) {
        return Get-Content -Raw (Resolve-DetectorPath $fixturePath) | ConvertFrom-Json -AsHashtable
    }

    $repository = [string](Get-DetectorParameter -Request $Request -Name 'repository')
    $number = Get-DetectorParameter -Request $Request -Name 'pullRequestNumber'
    if (
        [string]::IsNullOrWhiteSpace($repository) -or
        $repository -in @('OWNER/REPOSITORY', '<GITHUB-REPOSITORY>') -or
        $null -eq $number
    ) {
        throw 'configuration-invalid: set concrete repository and pullRequestNumber values.'
    }
    $arguments = @('pr', 'view', [string]$number, '--repo', $repository, '--json', ($Fields -join ','))
    if (-not [string]::IsNullOrWhiteSpace($capturePath)) {
        if ([string]::IsNullOrWhiteSpace($fixturePath)) {
            throw 'test-transport-invalid: fixturePath is required with testTransportCapturePath.'
        }
        Write-TestTransportRecord -Request $Request -Record ([ordered]@{
            transport = 'gh-cli'
            executable = 'gh'
            arguments = $arguments
            credentialEnvironment = @($(if (-not [string]::IsNullOrWhiteSpace($env:GH_TOKEN)) { 'GH_TOKEN' }))
            headers = [ordered]@{}
            body = $null
        })
        return Get-Content -Raw (Resolve-DetectorPath $fixturePath) | ConvertFrom-Json -AsHashtable
    }

    $raw = & gh @arguments 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw "provider-failure: gh pr view failed: $($raw -join [Environment]::NewLine)"
    }
    return ($raw -join [Environment]::NewLine) | ConvertFrom-Json -AsHashtable
}

function Test-GitHubPreflight {
    param(
        [System.Collections.IDictionary]$Preflight,
        [string]$TemplateId,
        [string]$Repository,
        [int]$PullRequestNumber
    )

    if (
        [int](Get-OptionalValue -Object $Preflight -Name 'schemaVersion' -Default 0) -ne 1 -or
        [string](Get-OptionalValue -Object $Preflight -Name 'provider' -Default '') -ne 'github' -or
        -not (Test-Rfc3339Timestamp -Value (Get-OptionalValue -Object $Preflight -Name 'observedAt' -Default '')) -or
        $TemplateId -notin @((Get-OptionalValue -Object $Preflight -Name 'templateIds' -Default @())) -or
        [string](Get-OptionalValue -Object (Get-OptionalValue -Object $Preflight -Name 'identity') -Name 'repository' -Default '') -ne $Repository -or
        [int](Get-OptionalValue -Object (Get-OptionalValue -Object $Preflight -Name 'identity') -Name 'pullRequestNumber' -Default 0) -ne $PullRequestNumber
    ) {
        return $false
    }
    return $true
}

try {
    $request = Read-DetectorRequest
    $pr = Get-GitHubPrData -Request $request -Fields @(
        'number', 'title', 'url', 'state', 'isDraft', 'mergeStateStatus', 'reviewDecision',
        'statusCheckRollup', 'author', 'comments', 'reviews', 'headRefOid'
    )

    $checks = @($pr.statusCheckRollup | ForEach-Object {
        [ordered]@{
            name = [string]$_.name
            status = [string]$_.status
            conclusion = [string]$_.conclusion
        }
    } | Sort-Object name)
    $failingChecks = @($checks | Where-Object { $_.conclusion -in @('FAILURE', 'TIMED_OUT', 'ACTION_REQUIRED', 'STARTUP_FAILURE', 'CANCELLED') })
    $mergeState = [string]$pr.mergeStateStatus
    $reviewDecision = [string]$pr.reviewDecision
    $state = [string]$pr.state
    $reason = $null
    $kind = $null
    $terminal = $false

    if ($state -in @('MERGED', 'CLOSED')) {
        $reason = "pull request is $($state.ToLowerInvariant())"
        $kind = 'github.pull-request.completed'
        $terminal = $true
    }
    elseif (-not [bool]$pr.isDraft -and $reviewDecision -eq 'CHANGES_REQUESTED') {
        $reason = 'changes were requested'
        $kind = 'github.pull-request.attention'
    }
    elseif ($failingChecks.Count -gt 0) {
        $reason = "checks are failing: $(@($failingChecks | ForEach-Object { $_.name }) -join ', ')"
        $kind = 'github.pull-request.attention'
    }
    elseif ($mergeState -in @('BLOCKED', 'DIRTY', 'BEHIND', 'UNSTABLE')) {
        $reason = "merge state is $mergeState"
        $kind = 'github.pull-request.attention'
    }
    elseif (-not [bool]$pr.isDraft -and $reviewDecision -eq 'APPROVED' -and $mergeState -eq 'CLEAN' -and $failingChecks.Count -eq 0) {
        $reason = 'approved, checks are not failing, and the merge state is clean'
        $kind = 'github.pull-request.ready-to-merge'
    }

    $evidence = [ordered]@{
        evidenceNormalizationVersion = 2
        provider = 'github'
        repository = [string](Get-DetectorParameter -Request $request -Name 'repository' -Default '')
        number = [int]$pr.number
        headSha = [string](Get-OptionalValue -Object $pr -Name 'headRefOid' -Default '')
        state = $state
        draft = [bool]$pr.isDraft
        mergeState = $mergeState
        reviewDecision = $reviewDecision
        checks = $checks
    }
    $cursor = Get-OpaqueCursor $evidence
    $preflight = Get-PreflightEvidence -Request $request
    if ($null -eq (Get-StateCursor $request) -and $preflight -is [System.Collections.IDictionary]) {
        $repository = [string](Get-DetectorParameter -Request $request -Name 'repository' -Default '')
        if (-not (Test-GitHubPreflight -Preflight $preflight -TemplateId 'github-pr' -Repository $repository -PullRequestNumber ([int]$pr.number))) {
            Write-Degraded -Code 'preflight-identity-mismatch' -Message 'GitHub preflight evidence does not match this watch, template, or RFC3339 timestamp.'
            return
        }
        if ([bool](Get-OptionalValue -Object $preflight -Name 'terminal' -Default $false)) {
            Write-EventlessTerminal -Evidence $evidence
            return
        }
        if ($terminal) {
            Write-EventlessTerminal -Evidence $evidence
            return
        }
    }
    $event = $null
    if ($null -eq $kind -and [bool](Get-DetectorParameter -Request $request -Name 'emitInitialSnapshot' -Default $false) -and $null -eq (Get-StateCursor $request)) {
        $reason = 'initial read-only snapshot'
        $kind = 'github.pull-request.snapshot'
    }
    if ($kind) {
        $event = [ordered]@{
            id = New-EventId -Provider 'github-pr' -Scope ([string]$pr.number) -Cursor $cursor
            kind = $kind
            subject = "GitHub PR #$($pr.number): $($pr.title)"
            body = "$reason`n$($pr.url)"
            metadata = [ordered]@{
                provider = 'github'
                pullRequest = [ordered]@{ number = [int]$pr.number; url = [string]$pr.url }
                reviewDecision = $reviewDecision
                mergeState = $mergeState
                failingChecks = @($failingChecks | ForEach-Object { $_.name })
            }
        }
    }
    Write-SnapshotResult -Request $request -Evidence $evidence -Event $event -Terminal:$terminal
}
catch {
    Write-Degraded $_.Exception.Message
}
