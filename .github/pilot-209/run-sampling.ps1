# Issue #209 pilot (non-merging). Bounded sampling loop. Runs the focused
# workload (telex-watcher lib harness, filter runtime::tests::) repeatedly on
# this VM until an admission cutoff, with a background low-frequency sampler.
# The 5-minute hard bound is the hosted step's timeout-minutes, not this
# loop; the cutoff only stops admitting new iterations. Iteration 1 is cold
# (first launch of the fresh harness binary) and is labeled, not discarded.
param(
    [Parameter(Mandatory)] [string] $Exe,
    [Parameter(Mandatory)] [string] $OutDir,
    [Parameter(Mandatory)] [ValidateSet('normal', 'reduced')] [string] $Arm,
    [int] $AdmitSeconds = 225
)
$ErrorActionPreference = 'Continue'
$enc = New-Object Text.UTF8Encoding $false
New-Item -ItemType Directory -Force -Path "$OutDir\phases", "$OutDir\logs" | Out-Null
$stopFile = "$OutDir\sampler.stop"
$iterLog = "$OutDir\iterations.jsonl"
function Write-Iter($record) {
    [IO.File]::AppendAllText($iterLog, (($record | ConvertTo-Json -Depth 4 -Compress) + "`n"), $enc)
}

$sampler = Start-Process -FilePath pwsh -PassThru -NoNewWindow `
    -RedirectStandardOutput "$OutDir\logs\sampler.out.txt" -RedirectStandardError "$OutDir\logs\sampler.err.txt" `
    -ArgumentList @('-NoProfile', '-File', "$PSScriptRoot\sampler.ps1", '-Out', "$OutDir\sampler.jsonl", '-StopFile', $stopFile)
[IO.File]::WriteAllText("$OutDir\sampler.pid", [string]$sampler.Id, $enc)

$harnessArgs = @('runtime::tests::')
if ($Arm -eq 'reduced') { $harnessArgs += '--test-threads=1' }
$clock = [Diagnostics.Stopwatch]::StartNew()
$failures = 0
$iteration = 0
while ($clock.Elapsed.TotalSeconds -lt $AdmitSeconds) {
    $iteration++
    $start = $clock.Elapsed.TotalSeconds
    Write-Iter ([ordered]@{ kind = 'start'; iteration = $iteration; arm = $Arm; cold = ($iteration -eq 1); t_s = [math]::Round($start, 3); args = ($harnessArgs -join ' ') })
    $env:PILOT209_OUT = "$OutDir\phases"
    $env:PILOT209_ITERATION = [string]$iteration
    $env:PILOT209_ARM = $Arm
    $watch = [Diagnostics.Stopwatch]::StartNew()
    & $Exe @harnessArgs > "$OutDir\logs\iter-$iteration.out.txt" 2> "$OutDir\logs\iter-$iteration.err.txt"
    $code = $LASTEXITCODE
    $watch.Stop()
    if ($code -ne 0) { $failures++ }
    Write-Iter ([ordered]@{ kind = 'end'; iteration = $iteration; arm = $Arm; cold = ($iteration -eq 1); exit = $code; wall_s = [math]::Round($watch.Elapsed.TotalSeconds, 3) })
    Write-Host ("iteration {0} arm={1} exit={2} wall={3:N3}s" -f $iteration, $Arm, $code, $watch.Elapsed.TotalSeconds)
}
Write-Iter ([ordered]@{ kind = 'loop-end'; iterations = $iteration; failures = $failures; t_s = [math]::Round($clock.Elapsed.TotalSeconds, 3); admit_s = $AdmitSeconds })
if ($failures) {
    Write-Host "::error::pilot209 $failures of $iteration harness iteration(s) failed"
    exit 1
}
