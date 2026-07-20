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
        $raw = & gh pr view $number --repo $repository --json number,title,url,author,comments,reviews 2>&1
        if ($LASTEXITCODE -ne 0) {
            throw "gh pr view failed: $($raw -join [Environment]::NewLine)"
        }
        $pr = ($raw -join [Environment]::NewLine) | ConvertFrom-Json -AsHashtable
    }

    $ignored = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
    foreach ($login in @((Get-DetectorParameter -Request $request -Name 'ignoredLogins' -Default @())) + @((Get-DetectorParameter -Request $request -Name 'selfLogin' -Default ''), $pr.author.login)) {
        if (-not [string]::IsNullOrWhiteSpace([string]$login)) {
            [void]$ignored.Add([string]$login)
        }
    }
    $externalReviews = @($pr.reviews | Where-Object {
        -not $ignored.Contains([string]$_.author.login) -and [string]$_.state -in @('APPROVED', 'CHANGES_REQUESTED', 'COMMENTED', 'DISMISSED')
    } | ForEach-Object {
        [ordered]@{ id = [string]$_.id; author = [string]$_.author.login; state = [string]$_.state }
    } | Sort-Object id)
    $externalComments = @($pr.comments | Where-Object {
        -not $ignored.Contains([string]$_.author.login) -and -not [string]::IsNullOrWhiteSpace([string]$_.body)
    } | ForEach-Object {
        [ordered]@{ id = [string]$_.id; author = [string]$_.author.login; body = [string]$_.body }
    } | Sort-Object id)

    $evidence = [ordered]@{
        provider = 'github'
        number = [int]$pr.number
        ignoredLogins = @($ignored | Sort-Object)
        externalReviews = $externalReviews
        externalComments = $externalComments
    }
    $cursor = Get-OpaqueCursor $evidence
    $event = $null
    if ($externalReviews.Count -gt 0 -or $externalComments.Count -gt 0) {
        $event = [ordered]@{
            id = New-EventId -Provider 'github-pr-activity' -Scope ([string]$pr.number) -Cursor $cursor
            kind = 'github.pull-request.external-activity'
            subject = "GitHub PR #$($pr.number): external reviewer activity"
            body = "$($externalReviews.Count) external review(s), $($externalComments.Count) external comment(s)`n$($pr.url)"
            metadata = [ordered]@{
                provider = 'github'
                pullRequest = [ordered]@{ number = [int]$pr.number; url = [string]$pr.url }
                ignoredLogins = @($ignored | Sort-Object)
                externalReviews = $externalReviews
                externalComments = $externalComments
            }
        }
    }
    Write-SnapshotResult -Request $request -Evidence $evidence -Event $event
}
catch {
    Write-Degraded $_.Exception.Message
}
