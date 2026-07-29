[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$Repository,
    [Parameter(Mandatory)]
    [int]$PullRequestNumber,
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
        $pr = Get-Content -Raw $FixturePath | ConvertFrom-Json -AsHashtable
    }
    catch {
        Exit-PreflightFailure -Code 5 -Diagnostic 'fixture-parse-failure' -Message $_.Exception.Message
    }
}
else {
    $raw = & gh pr view $PullRequestNumber --repo $Repository --json number,state,headRefOid 2>&1
    if ($LASTEXITCODE -ne 0) {
        Exit-PreflightFailure -Code 4 -Diagnostic 'provider-auth-transport-failure' -Message "gh pr view failed: $($raw -join [Environment]::NewLine)"
    }
    try {
        $pr = ($raw -join [Environment]::NewLine) | ConvertFrom-Json -AsHashtable
    }
    catch {
        Exit-PreflightFailure -Code 5 -Diagnostic 'provider-parse-failure' -Message $_.Exception.Message
    }
}

try {
    $state = [string]$pr.state
    $terminal = $state -in @('MERGED', 'CLOSED')
    $evidence = [ordered]@{
        schemaVersion = 1
        provider = 'github'
        templateIds = @('github-pr', 'github-pr-external-activity')
        observedAt = $(if ($Now) { $Now } else { [DateTimeOffset]::UtcNow.ToString('o') })
        terminal = $terminal
        state = $state
        identity = [ordered]@{
            repository = $Repository
            pullRequestNumber = [int]$pr.number
            headSha = [string]$pr.headRefOid
        }
    }
    [Console]::Out.WriteLine(($evidence | ConvertTo-Json -Depth 10 -Compress))
}
catch {
    Exit-PreflightFailure -Code 5 -Diagnostic 'provider-shape-failure' -Message $_.Exception.Message
}
if ($terminal) {
    [Console]::Error.WriteLine("registration aborted: GitHub PR #$PullRequestNumber is $($state.ToLowerInvariant()).")
    exit 3
}
exit 0
