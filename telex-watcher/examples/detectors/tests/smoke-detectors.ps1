[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$examplesRoot = Split-Path -Parent $PSScriptRoot
$scriptsRoot = Join-Path $examplesRoot 'scripts'
$fixturesRoot = Join-Path $examplesRoot 'fixtures'
$registrationsRoot = Join-Path $examplesRoot 'registrations'

function Assert-True {
    param([bool]$Condition, [string]$Message)
    if (-not $Condition) {
        throw "Assertion failed: $Message"
    }
}

function Assert-ProtocolResult {
    param([hashtable]$Result)

    Assert-True ($Result.schemaVersion -eq 1) 'schemaVersion must be 1'
    Assert-True ($Result.outcome -in @('idle', 'event', 'terminal', 'degraded')) 'outcome must be a protocol v1 outcome'
    if ($Result.outcome -eq 'event') {
        Assert-True ($null -ne $Result.event) 'event outcome needs event'
    }
    if ($Result.outcome -eq 'idle') {
        Assert-True (-not $Result.Contains('event')) 'idle must not contain event'
    }
    if ($Result.outcome -eq 'degraded') {
        Assert-True (-not $Result.Contains('event') -and -not $Result.Contains('nextState')) 'degraded must not advance state or send event'
    }
    if ($Result.Contains('event')) {
        foreach ($field in 'id', 'kind', 'subject', 'body', 'metadata') {
            Assert-True ($Result.event.Contains($field)) "event must contain '$field'"
        }
        Assert-True ($Result.event.kind -match '^[^.]+\..+$') 'event kind must be namespaced'
    }
}

function Invoke-DetectorFixture {
    param(
        [string]$ScriptName,
        [hashtable]$Parameters,
        [hashtable]$State = @{}
    )

    $request = [ordered]@{
        schemaVersion = 1
        attempt = @{ id = 'fixture-attempt'; now = '2026-07-19T00:00:00Z' }
        watch = @{ id = 'fixture-watch'; parameters = $Parameters }
        script = @{ mode = 'follow-path'; sha256 = 'fixture' }
        state = $State
    }
    $raw = $request | ConvertTo-Json -Depth 20 -Compress
    $output = $raw | & pwsh -NoLogo -NoProfile -File (Join-Path $scriptsRoot $ScriptName)
    Assert-True ($LASTEXITCODE -eq 0) "$ScriptName should exit successfully for its fixture"
    $text = $output -join [Environment]::NewLine
    $result = $text | ConvertFrom-Json -AsHashtable
    Assert-ProtocolResult $result
    return $result
}

function Test-ReplaySuppression {
    param([string]$ScriptName, [hashtable]$Parameters)

    $first = Invoke-DetectorFixture -ScriptName $ScriptName -Parameters $Parameters
    Assert-True ($first.outcome -eq 'event') "$ScriptName fixture should deterministically emit an initial event"
    Assert-True ($first.nextState.Contains('cursor')) "$ScriptName event should produce an opaque cursor"
    $second = Invoke-DetectorFixture -ScriptName $ScriptName -Parameters $Parameters -State $first.nextState
    Assert-True ($second.outcome -eq 'idle') "$ScriptName should suppress a replay for the same cursor"
    Assert-True ($second.nextState.cursor -eq $first.nextState.cursor) "$ScriptName cursor must remain stable on replay"
    return $first
}

function Test-DefaultBaseline {
    param([string]$ScriptName, [hashtable]$Parameters)

    $result = Invoke-DetectorFixture -ScriptName $ScriptName -Parameters $Parameters
    Assert-True ($result.outcome -eq 'idle') "$ScriptName should preserve the default baseline behavior"
    Assert-True ($result.nextState.Contains('cursor')) "$ScriptName baseline should persist an opaque cursor"
}

$github = Test-ReplaySuppression -ScriptName 'gh-pr-detector.ps1' -Parameters @{
    fixturePath = (Join-Path $fixturesRoot 'github-pr-ready.json')
    repository = 'OWNER/REPOSITORY'
    emitInitialSnapshot = $true
}
Assert-True ($github.event.kind -eq 'github.pull-request.ready-to-merge') 'generic GitHub fixture should use ready-to-merge logic'
$githubSnapshot = Test-ReplaySuppression -ScriptName 'gh-pr-detector.ps1' -Parameters @{
    fixturePath = (Join-Path $fixturesRoot 'github-pr-neutral.json')
    repository = 'OWNER/REPOSITORY'
    emitInitialSnapshot = $true
}
Assert-True ($githubSnapshot.event.kind -eq 'github.pull-request.snapshot') 'generic GitHub neutral fixture should emit an initial snapshot'
Test-DefaultBaseline -ScriptName 'gh-pr-detector.ps1' -Parameters @{
    fixturePath = (Join-Path $fixturesRoot 'github-pr-neutral.json')
    repository = 'OWNER/REPOSITORY'
    emitInitialSnapshot = $false
}

$custom = Test-ReplaySuppression -ScriptName 'gh-pr-external-activity-detector.ps1' -Parameters @{
    fixturePath = (Join-Path $fixturesRoot 'github-pr-external-activity.json')
    selfLogin = 'self-login'
    ignoredLogins = @('example-bot')
    emitInitialSnapshot = $true
}
Assert-True (@($custom.event.metadata.externalReviews).Count -eq 1) 'custom policy must ignore the PR author review'
Assert-True (@($custom.event.metadata.externalReviews)[0].author -eq 'external-reviewer') 'custom policy must retain external reviewer activity'
Assert-True (@($custom.event.metadata.externalComments).Count -eq 1) 'custom policy must ignore the PR author comment'
Assert-True (@($custom.event.metadata.externalComments)[0].author -eq 'external-commenter') 'custom policy must retain external comments'

$azure = Test-ReplaySuppression -ScriptName 'azure-devops-pr-detector.ps1' -Parameters @{
    fixturePath = (Join-Path $fixturesRoot 'azure-devops-pr-ready.json')
    emitInitialSnapshot = $true
}
Assert-True ($azure.event.kind -eq 'azure-devops.pull-request.ready-to-merge') 'Azure DevOps fixture should use REST review and merge logic'
$azureSnapshot = Test-ReplaySuppression -ScriptName 'azure-devops-pr-detector.ps1' -Parameters @{
    fixturePath = (Join-Path $fixturesRoot 'azure-devops-pr-neutral.json')
    emitInitialSnapshot = $true
}
Assert-True ($azureSnapshot.event.kind -eq 'azure-devops.pull-request.snapshot') 'Azure DevOps neutral fixture should emit an initial snapshot'
Test-DefaultBaseline -ScriptName 'azure-devops-pr-detector.ps1' -Parameters @{
    fixturePath = (Join-Path $fixturesRoot 'azure-devops-pr-neutral.json')
    emitInitialSnapshot = $false
}

$file = Test-ReplaySuppression -ScriptName 'file-json-detector.ps1' -Parameters @{
    inputPath = (Join-Path $fixturesRoot 'file-json-ready.json')
    readyField = 'ready'
    emitInitialSnapshot = $true
}
Assert-True ($file.event.kind -eq 'local.file-json.ready') 'file fixture should emit the local JSON event'

$requiredRegistrationFields = @(
    'id', 'command', 'scriptPath', 'workingDirectory', 'scriptMode', 'sender', 'target',
    'intervalSeconds', 'timeoutSeconds', 'attention', 'requiresDisposition',
    'environmentAllowlist', 'parameters', 'state'
)
Get-ChildItem -Path $registrationsRoot -Filter '*.json' | ForEach-Object {
    $registration = Get-Content -Raw $_.FullName | ConvertFrom-Json -AsHashtable
    foreach ($field in $requiredRegistrationFields) {
        Assert-True ($registration.Contains($field)) "$($_.Name) must use WatchSpec field '$field'"
    }
    Assert-True ($registration.command -contains $registration.scriptPath) "$($_.Name) command must include scriptPath"
    Assert-True ($registration.scriptMode -in @('pinned', 'follow-path')) "$($_.Name) scriptMode must be valid"
}

$watcherRoot = Split-Path -Parent (Split-Path -Parent $examplesRoot)
$watcher = Join-Path (Split-Path -Parent $watcherRoot) 'target\debug\telex-watcher.exe'
Assert-True (Test-Path $watcher) 'build telex-watcher before running the registration schema smoke test'
$scratch = Join-Path $PSScriptRoot "_registration-smoke-$PID"
New-Item -ItemType Directory -Path $scratch | Out-Null
try {
    $registry = Join-Path $scratch 'watcher.sqlite'
    Get-ChildItem -Path $registrationsRoot -Filter '*.json' | ForEach-Object {
        $registration = Get-Content -Raw $_.FullName | ConvertFrom-Json -AsHashtable
        $scriptPath = Join-Path $scriptsRoot (Split-Path -Leaf $registration.scriptPath)
        $registration.scriptPath = $scriptPath
        $registration.workingDirectory = $scriptsRoot
        $registration.command = @($registration.command | ForEach-Object {
            if ($_ -like 'C:\path\to\*') { $scriptPath } else { $_ }
        })
        $file = Join-Path $scratch $_.Name
        $registration | ConvertTo-Json -Depth 20 | Set-Content -Encoding utf8 -Path $file
        & $watcher --registry $registry add --file $file | Out-Null
        Assert-True ($LASTEXITCODE -eq 0) "$($_.Name) must be accepted by the actual WatchSpec parser"
    }
}
finally {
    Remove-Item -Recurse -Force $scratch -ErrorAction SilentlyContinue
}

Write-Host 'Detector fixture smoke tests passed.'
