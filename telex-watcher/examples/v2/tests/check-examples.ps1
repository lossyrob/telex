#requires -Version 7.4
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot
$schemas = [IO.Path]::GetFullPath((Join-Path $root '..\..\..\docs\design\schemas'))
$pwsh = (Get-Process -Id $PID).Path
$scratch = Join-Path ([IO.Path]::GetTempPath()) "telex-v2-examples-$([Guid]::NewGuid())"
New-Item -ItemType Directory -Path $scratch | Out-Null
$script:checks = 0

function Assert($Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
    $script:checks++
}

function Test-Schema([string]$Json, [string]$Name) {
    Test-Json -Json $Json -SchemaFile (Join-Path $schemas "watcher-$Name-v2.schema.json") `
        -ErrorAction SilentlyContinue
}

function Run-Detector {
    param([string]$Name, [hashtable]$Parameters, $State = $null,
        [switch]$Fails, [switch]$Stub, [string]$Response = 'ready.json', [int]$SourceExit = 0,
        [int]$Version = 2)
    $request = Get-Content -LiteralPath (Join-Path $root 'fixtures\request.json') -Raw | ConvertFrom-Json -AsHashtable
    $request.schemaVersion = $Version
    $request.watch.parameters = $Parameters
    $request.state = $State
    $json = $request | ConvertTo-Json -Depth 20 -Compress
    if ($Version -eq 2) { Assert (Test-Schema $json 'detector-request') "$Name request schema" }
    $start = [Diagnostics.ProcessStartInfo]::new($pwsh)
    $start.UseShellExecute = $false
    $start.RedirectStandardInput = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    foreach ($arg in @('-NoLogo', '-NoProfile', '-File')) { $start.ArgumentList.Add($arg) }
    if ($Stub) {
        $start.ArgumentList.Add((Join-Path $PSScriptRoot 'source-stubs.ps1'))
        $start.ArgumentList.Add('-Detector')
    }
    $start.ArgumentList.Add((Join-Path $root "$Name.ps1"))
    foreach ($key in @('GH_TOKEN', 'GITHUB_TOKEN', 'GH_CONFIG_DIR', 'TELEX_EXAMPLE_ADO_TOKEN')) {
        $start.Environment.Remove($key) | Out-Null
    }
    $start.Environment['V2_TEST_RESPONSE'] = Join-Path $root "fixtures\$Response"
    $start.Environment['V2_TEST_EXIT'] = "$SourceExit"
    if ($Stub) { $start.Environment['TELEX_EXAMPLE_ADO_TOKEN'] = 'fixture-token' }
    $process = [Diagnostics.Process]::Start($start)
    try {
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        $process.StandardInput.Write($json)
        $process.StandardInput.Close()
        if (-not $process.WaitForExit(20000)) {
            $process.Kill($true)
            $process.WaitForExit()
            throw "$Name exceeded the test timeout."
        }
        $out = $stdout.GetAwaiter().GetResult()
        $err = $stderr.GetAwaiter().GetResult()
        if ($Fails) {
            Assert ($process.ExitCode -ne 0 -and $err.Length -gt 0 -and $out.Trim().Length -eq 0) `
                "$Name must fail visibly without success output: $out $err"
            return
        }
        Assert ($process.ExitCode -eq 0) "$Name failed: $err"
        Assert ($err.Length -eq 0) "$Name unexpected diagnostics: $err"
        Assert (Test-Schema $out 'detector-result') "$Name result schema: $out"
        Assert ([Text.Encoding]::UTF8.GetByteCount($out) -lt 262144) "$Name output bound"
        $out | ConvertFrom-Json -AsHashtable
    } finally { $process.Dispose() }
}

try {
    foreach ($registration in Get-ChildItem (Join-Path $root 'registrations') -Filter '*.json') {
        Assert (Test-Schema (Get-Content $registration.FullName -Raw) 'registration') $registration.Name
    }
    $ready = Join-Path $root 'fixtures\ready.json'
    $notReady = Join-Path $root 'fixtures\not-ready.json'
    $gh = @{ fixturePath = Join-Path $root 'fixtures\github-open.json' }
    $first = Run-Detector 'github-pr' $gh
    Assert ($first.outcome -eq 'event') 'GitHub emits initial snapshot'
    $retry = Run-Detector 'github-pr' $gh
    Assert ($retry.nextState.snapshot -ceq $first.nextState.snapshot) 'Uncommitted snapshot re-emits'
    Assert ($retry.event.body -ceq $first.event.body) 'Uncommitted content stays the same'
    $repeat = Run-Detector 'github-pr' $gh $first.nextState
    Assert ($repeat.outcome -eq 'idle') 'Committed GitHub snapshot is idle'
    $changedPath = Join-Path $scratch 'github.json'
    $pr = Get-Content $gh.fixturePath -Raw | ConvertFrom-Json -AsHashtable
    $pr.headRefOid = 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'
    $pr | ConvertTo-Json | Set-Content $changedPath -Encoding utf8
    $changed = Run-Detector 'github-pr' @{ fixturePath = $changedPath } $first.nextState
    Assert ($changed.outcome -eq 'event') 'New GitHub head emits'
    $pr.state = 'MERGED'
    $pr | ConvertTo-Json | Set-Content $changedPath -Encoding utf8
    $terminal = Run-Detector 'github-pr' @{ fixturePath = $changedPath } $changed.nextState
    Assert ($terminal.outcome -eq 'terminal' -and $terminal.event) 'Merged PR is terminal with event'

    $azure = @{ fixturePath = Join-Path $root 'fixtures\azure-completed.json'; buildId = 42 }
    $completed = Run-Detector 'azure-build' $azure
    Assert ($completed.outcome -eq 'terminal' -and -not $completed.ContainsKey('nextState')) 'Build completion preserves state'
    $pendingPath = Join-Path $scratch 'build.json'
    '{"id":42,"status":"inProgress","result":null}' | Set-Content $pendingPath -Encoding utf8
    Assert ((Run-Detector 'azure-build' @{ fixturePath = $pendingPath; buildId = 42 }).outcome -eq 'idle') 'Running build is idle'
    Run-Detector 'azure-build' @{ fixturePath = $azure.fixturePath; buildId = 43 } -Fails
    '{"id":42,"status":"completed","result":null}' | Set-Content $pendingPath -Encoding utf8
    Run-Detector 'azure-build' @{ fixturePath = $pendingPath; buildId = 42 } -Fails

    $http = Run-Detector 'http-json' @{ fixturePath = $ready }
    $httpAgain = Run-Detector 'http-json' @{ fixturePath = $ready } @{ arbitrary = 'opaque prior state' }
    Assert ($http.outcome -eq 'event' -and $httpAgain.outcome -eq 'event') 'HTTP recurrence is not suppressed'
    Assert (-not $http.ContainsKey('nextState') -and $http.event.body -ceq $httpAgain.event.body) 'Identical recurring content preserves state'
    Assert ((Run-Detector 'http-json' @{ fixturePath = $notReady }).outcome -eq 'idle') 'HTTP false is idle'

    $file = Run-Detector 'file-json' @{ path = $ready }
    Assert ($file.outcome -eq 'event') 'File initially true emits'
    Assert ((Run-Detector 'file-json' @{ path = $ready }).outcome -eq 'event') 'File re-emits without committed state'
    Assert ((Run-Detector 'file-json' @{ path = $ready } $file.nextState).outcome -eq 'idle') 'File suppresses committed edge'
    $reset = Run-Detector 'file-json' @{ path = $notReady } $file.nextState
    Assert ($reset.outcome -eq 'idle' -and $reset.nextState.ready -eq $false) 'False resets the edge'
    Assert ((Run-Detector 'file-json' @{ path = $ready } $reset.nextState).outcome -eq 'event') 'Later rising edge emits'
    Run-Detector 'file-json' @{ path = (Join-Path $scratch 'missing.json') } -Fails
    Run-Detector 'file-json' @{ path = $ready } @{ ready = 'false' } -Fails

    $command = @{ command = @($pwsh, '-NoLogo', '-NoProfile', '-File', (Join-Path $root 'fixtures\condition.ps1')) }
    $cmd = Run-Detector 'command' $command
    Assert ($cmd.outcome -eq 'event') 'Command true emits without leaking stdout'
    Assert ((Run-Detector 'command' $command $cmd.nextState).outcome -eq 'idle') 'Command suppresses committed edge'
    $falseCommand = @{ command = @($pwsh, '-NoLogo', '-NoProfile', '-Command', 'exit 1') }
    $cmdReset = Run-Detector 'command' $falseCommand $cmd.nextState
    Assert ($cmdReset.outcome -eq 'idle' -and $cmdReset.nextState.ready -eq $false) 'Command exit 1 resets'
    Assert ((Run-Detector 'command' $command $cmdReset.nextState).outcome -eq 'event') 'Command later edge emits'
    Run-Detector 'command' @{ command = @($pwsh, '-NoProfile', '-Command', 'exit 2') } -Fails
    Run-Detector 'command' @{ command = @((Join-Path $scratch 'missing-executable')) } -Fails
    Run-Detector 'command' @{ command = 'not-an-argv' } -Fails

    foreach ($invalid in @('{"ready":"false"}', '{"ready":null}', '{}', '[{"ready":true}]', 'not-json')) {
        $path = Join-Path $scratch 'invalid.json'
        $invalid | Set-Content $path -Encoding utf8
        Run-Detector 'file-json' @{ path = $path } -Fails
        Run-Detector 'http-json' @{ fixturePath = $path } -Fails
    }
    foreach ($name in @('github-pr', 'azure-build', 'http-json', 'file-json', 'command')) {
        Run-Detector $name @{} -Version 1 -Fails
    }

    $liveGh = @{ repository = 'OWNER/REPO'; pullRequestNumber = 123 }
    Assert ((Run-Detector 'github-pr' $liveGh -Stub -Response 'github-open.json').outcome -eq 'event') 'GitHub read-only call'
    Run-Detector 'github-pr' $liveGh -Stub -SourceExit 1 -Fails
    $liveAzure = @{ organization = 'ORG'; project = 'Project Name'; buildId = 42 }
    Assert ((Run-Detector 'azure-build' $liveAzure -Stub -Response 'azure-completed.json').outcome -eq 'terminal') 'Azure read-only call'
    Run-Detector 'azure-build' $liveAzure -Stub -SourceExit 1 -Fails
    Run-Detector 'azure-build' $liveAzure -Fails
    $liveHttp = @{ url = 'https://example.invalid/status.json' }
    Assert ((Run-Detector 'http-json' $liveHttp -Stub).outcome -eq 'event') 'HTTP read-only call'
    Run-Detector 'http-json' $liveHttp -Stub -SourceExit 1 -Fails
    Run-Detector 'http-json' @{ url = 'http://example.invalid/status.json' } -Fails
    Run-Detector 'http-json' @{ url = 'https://user:password@example.invalid/status.json' } -Fails

    foreach ($field in @('id', 'eventSequence', 'operationId', 'sender', 'target', 'attention', 'action')) {
        $bad = @{ schemaVersion = 2; outcome = 'event'; event = @{ kind = 'example'; subject = 'Example'; body = '' } }
        $bad.event[$field] = 'forbidden'
        Assert (-not (Test-Schema ($bad | ConvertTo-Json -Depth 10) 'detector-result')) "Schema rejects event $field"
    }
    Assert (-not (Test-Schema '{"schemaVersion":2,"outcome":"degraded","nextState":null}' 'detector-result')) 'Degraded cannot advance state'
    Assert (-not (Test-Schema '{"schemaVersion":2,"outcome":"event"}' 'detector-result')) 'Event requires content'
    Write-Output "Passed $script:checks example checks (offline; no Telex runtime or provider access)."
} finally {
    Remove-Item -LiteralPath $scratch -Recurse -Force
}
