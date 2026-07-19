[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'DetectorCommon.psm1') -Force

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
        $raw = & gh pr view $number --repo $repository --json number,title,url,state,isDraft,mergeStateStatus,reviewDecision,statusCheckRollup,author,comments,reviews 2>&1
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
        provider = 'github'
        repository = [string](Get-DetectorParameter -Request $request -Name 'repository' -Default '')
        number = [int]$pr.number
        state = $state
        draft = [bool]$pr.isDraft
        mergeState = $mergeState
        reviewDecision = $reviewDecision
        checks = $checks
        reason = $reason
    }
    $cursor = Get-OpaqueCursor $evidence
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
