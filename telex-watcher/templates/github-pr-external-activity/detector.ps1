[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$helperPath = Join-Path $PSScriptRoot '..\shared\DetectorCommon.psm1'
$expectedHelperSha256 = 'cca5ae57123142df3b7bd053cb6a1d88e0436ca38dd769533d5d4591987201b1'
if ((Get-FileHash $helperPath -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expectedHelperSha256) {
    [Console]::Error.WriteLine('detectorDiagnostic={"schemaVersion":1,"code":"shared-helper-digest-mismatch","message":"Pinned shared helper digest mismatch."}')
    [Console]::Out.WriteLine('{"schemaVersion":1,"outcome":"degraded"}')
    exit 0
}
Import-Module $helperPath -Force

function Get-GitHubPrData {
    param([hashtable]$Request)

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
    $fields = 'number,title,url,state,headRefOid,author,comments,reviews'
    $arguments = @('pr', 'view', [string]$number, '--repo', $repository, '--json', $fields)
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
        [string]$Repository,
        [int]$PullRequestNumber
    )

    if (
        [int](Get-OptionalValue -Object $Preflight -Name 'schemaVersion' -Default 0) -ne 1 -or
        [string](Get-OptionalValue -Object $Preflight -Name 'provider' -Default '') -ne 'github' -or
        -not (Test-Rfc3339Timestamp -Value (Get-OptionalValue -Object $Preflight -Name 'observedAt' -Default '')) -or
        'github-pr-external-activity' -notin @((Get-OptionalValue -Object $Preflight -Name 'templateIds' -Default @())) -or
        [string](Get-OptionalValue -Object (Get-OptionalValue -Object $Preflight -Name 'identity') -Name 'repository' -Default '') -ne $Repository -or
        [int](Get-OptionalValue -Object (Get-OptionalValue -Object $Preflight -Name 'identity') -Name 'pullRequestNumber' -Default 0) -ne $PullRequestNumber
    ) {
        return $false
    }
    return $true
}

try {
    $request = Read-DetectorRequest
    $pr = Get-GitHubPrData -Request $request

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
        evidenceNormalizationVersion = 2
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
        if (-not (Test-GitHubPreflight -Preflight $preflight -Repository $repository -PullRequestNumber ([int]$pr.number))) {
            Write-Degraded -Code 'preflight-identity-mismatch' -Message 'GitHub preflight evidence does not match this watch, template, or RFC3339 timestamp.'
            return
        }
        if ([bool](Get-OptionalValue -Object $preflight -Name 'terminal' -Default $false)) {
            Write-EventlessTerminal -Evidence $evidence
            return
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
