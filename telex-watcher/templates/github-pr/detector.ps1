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

try {
    $request = Read-DetectorRequest
    $fixturePath = Get-DetectorParameter -Request $request -Name 'fixturePath'
    if ($fixturePath) {
        $pr = Get-Content -Raw (Resolve-DetectorPath ([string]$fixturePath)) | ConvertFrom-Json -AsHashtable
    }
    else {
        $repository = [string](Get-DetectorParameter -Request $request -Name 'repository')
        $number = Get-DetectorParameter -Request $request -Name 'pullRequestNumber'
        if ([string]::IsNullOrWhiteSpace($repository) -or $null -eq $number) {
            throw 'Set parameters.repository and parameters.pullRequestNumber, or provide parameters.fixturePath.'
        }
        $raw = & gh pr view $number --repo $repository --json number,title,url,state,isDraft,mergeStateStatus,reviewDecision,statusCheckRollup,author,comments,reviews,headRefOid 2>&1
        if ($LASTEXITCODE -ne 0) {
            throw "gh pr view failed: $($raw -join [Environment]::NewLine)"
        }
        $pr = ($raw -join [Environment]::NewLine) | ConvertFrom-Json -AsHashtable
    }

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
        evidenceNormalizationVersion = 1
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
        if (
            [string]$preflight.provider -ne 'github' -or
            [int]$preflight.identity.pullRequestNumber -ne [int]$pr.number -or
            [string]$preflight.identity.repository -ne $repository
        ) {
            throw 'preflight-identity-mismatch: GitHub evidence does not match this watch.'
        }
        if (-not [bool]$preflight.terminal -and $terminal) {
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
