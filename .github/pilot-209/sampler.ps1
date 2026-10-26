# Issue #209 pilot (non-merging). Cheap, low-frequency, read-only CPU/storage
# observations: one Get-Counter sample plus MsMpEng cumulative CPU every
# ~10s. Stops on a stop file or its own cap; the hosted step timeout bounds
# it independently. Every error is retained as a JSONL record.
param(
    [Parameter(Mandatory)] [string] $Out,
    [Parameter(Mandatory)] [string] $StopFile,
    [int] $IntervalSeconds = 10,
    [int] $CapSeconds = 285
)
$ErrorActionPreference = 'Continue'
$enc = New-Object Text.UTF8Encoding $false
$counters = @(
    '\Processor(_Total)\% Processor Time'
    '\System\Processor Queue Length'
    '\Memory\Available MBytes'
    '\PhysicalDisk(*)\Current Disk Queue Length'
    '\PhysicalDisk(*)\Avg. Disk sec/Write'
    '\PhysicalDisk(*)\Avg. Disk sec/Read'
    '\PhysicalDisk(*)\Disk Writes/sec'
    '\PhysicalDisk(*)\% Idle Time'
)
$clock = [Diagnostics.Stopwatch]::StartNew()
$seq = 0
function Write-Record($record) {
    [IO.File]::AppendAllText($Out, (($record | ConvertTo-Json -Depth 5 -Compress) + "`n"), $enc)
}
Write-Record ([ordered]@{ schema = 'pilot209-sampler/v1'; kind = 'start'; utc = (Get-Date).ToUniversalTime().ToString('o'); interval_s = $IntervalSeconds; cap_s = $CapSeconds })
while ($clock.Elapsed.TotalSeconds -lt $CapSeconds -and -not (Test-Path $StopFile)) {
    $seq++
    $record = [ordered]@{ kind = 'sample'; seq = $seq; t_s = [math]::Round($clock.Elapsed.TotalSeconds, 2); counters = [ordered]@{}; errors = @() }
    try {
        $sample = Get-Counter -Counter $counters -ErrorAction Stop
        foreach ($c in $sample.CounterSamples) {
            $record.counters[$c.Path -replace '^\\\\[^\\]+', ''] = [math]::Round($c.CookedValue, 4)
        }
    } catch {
        $record.errors += "counter: $($_.Exception.Message)"
    }
    try {
        $p = @(Get-Process -Name MsMpEng -ErrorAction Stop)
        $record.msmpeng_cpu_s = if ($null -ne $p[0].CPU) { [math]::Round(($p | Measure-Object CPU -Sum).Sum, 2) } else { $null }
        $record.msmpeng_ws_mb = [math]::Round(($p | Measure-Object WorkingSet64 -Sum).Sum / 1MB, 1)
    } catch {
        $record.errors += "msmpeng: $($_.Exception.Message)"
    }
    Write-Record $record
    $deadline = $clock.Elapsed.TotalSeconds + $IntervalSeconds - 1
    while ($clock.Elapsed.TotalSeconds -lt $deadline -and -not (Test-Path $StopFile)) { Start-Sleep -Milliseconds 500 }
}
$reason = if (Test-Path $StopFile) { 'stop-file' } else { 'cap' }
Write-Record ([ordered]@{ kind = 'end'; reason = $reason; samples = $seq; t_s = [math]::Round($clock.Elapsed.TotalSeconds, 2) })
