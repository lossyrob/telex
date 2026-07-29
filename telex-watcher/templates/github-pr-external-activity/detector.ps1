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
        $raw = & gh pr view $number --repo $repository --json number,title,url,state,headRefOid,author,comments,reviews 2>&1
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

    $normalizedComments = @($externalComments | ForEach-Object {
        [ordered]@{ id = $_.id; author = $_.author }
    })
    $evidence = [ordered]@{
        evidenceNormalizationVersion = 1
        provider = 'github'
        repository = [string](Get-DetectorParameter -Request $request -Name 'repository' -Default '')
        number = [int]$pr.number
        headSha = [string](Get-OptionalValue -Object $pr -Name 'headRefOid' -Default '')
        state = [string](Get-OptionalValue -Object $pr -Name 'state' -Default 'OPEN')
        ignoredLogins = @($ignored | Sort-Object)
        externalReviews = $externalReviews
        externalComments = $normalizedComments
    }
    $cursor = Get-OpaqueCursor $evidence
    $terminal = [string](Get-OptionalValue -Object $pr -Name 'state' -Default 'OPEN') -in @('MERGED', 'CLOSED')
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
    }
    if ($terminal) {
        Write-EventlessTerminal -Evidence $evidence
        return
    }
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
