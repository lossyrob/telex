# Issue #209 pilot (non-merging). Summarizes one job's evidence into
# summary.json and the step summary. Fails (exit 1) on missing or unparseable
# records, sampler errors, or receipt errors so metric problems are never
# reported as a clean pipeline; the raw evidence is uploaded regardless.
param(
    [Parameter(Mandatory)] [string] $OutDir,
    [Parameter(Mandatory)] [string] $Arm,
    [Parameter(Mandatory)] [string] $Shard
)
$ErrorActionPreference = 'Continue'
$problems = [Collections.Generic.List[string]]::new()
function Read-Jsonl([string] $Path) {
    $rows = [Collections.Generic.List[object]]::new()
    if (-not (Test-Path $Path)) { $problems.Add("missing: $(Split-Path -Leaf $Path)"); return $rows }
    $n = 0
    foreach ($line in [IO.File]::ReadAllLines($Path)) {
        $n++
        if (-not $line.Trim()) { continue }
        try { $rows.Add(($line | ConvertFrom-Json -ErrorAction Stop)) }
        catch { $problems.Add("unparseable $(Split-Path -Leaf $Path):${n}") }
    }
    $rows
}
function Stat([double[]] $values) {
    if (-not $values -or $values.Count -eq 0) { return $null }
    $sorted = $values | Sort-Object
    $c = $sorted.Count
    $median = if ($c % 2) { $sorted[[int][math]::Floor($c / 2)] } else { ($sorted[$c / 2 - 1] + $sorted[$c / 2]) / 2 }
    [ordered]@{ n = $c; min = $sorted[0]; median = $median; max = $sorted[$c - 1]; sum = ($sorted | Measure-Object -Sum).Sum }
}

$iterations = Read-Jsonl "$OutDir\iterations.jsonl"
$ends = @($iterations | Where-Object kind -eq 'end')
$loopEnd = @($iterations | Where-Object kind -eq 'loop-end')
if (-not $ends.Count) { $problems.Add('no completed harness iteration') }
if (-not $loopEnd.Count) { $problems.Add('loop did not finish (censored by step timeout or failure)') }
$starts = @($iterations | Where-Object kind -eq 'start')
if ($starts.Count -ne $ends.Count) { $problems.Add("censored iteration: $($starts.Count) started, $($ends.Count) ended") }

$records = [Collections.Generic.List[object]]::new()
foreach ($file in @(Get-ChildItem "$OutDir\phases" -Filter 'opaque-*.jsonl' -ErrorAction SilentlyContinue)) {
    foreach ($r in (Read-Jsonl $file.FullName)) { $records.Add($r) }
}
$okEnds = @($ends | Where-Object exit -eq 0)
$expected = 3 * $okEnds.Count
if ($records.Count -lt $expected) { $problems.Add("phase records $($records.Count) < expected $expected (3 states x $($okEnds.Count) passing iterations)") }
foreach ($r in $records) {
    if ($r.schema -ne 'pilot209-opaque/v1') { $problems.Add("unexpected record schema '$($r.schema)'") }
    if (-not $r.events -or @($r.events).Count -eq 0) { $problems.Add("record iteration=$($r.iteration) state=$($r.state_index) has no events") }
    if ($r.commands_ok -ne $true -or $r.owner_ok -ne $true) { $problems.Add("record iteration=$($r.iteration) state=$($r.state_index) commands_ok=$($r.commands_ok) owner_ok=$($r.owner_ok)") }
}

$byPhase = @{}
$unpairedSql = 0
foreach ($r in $records) {
    foreach ($e in @($r.events)) {
        $key = [string]$e.phase
        if ($key.StartsWith('sql:') -and $null -eq $e.elapsed_us) { $unpairedSql++ }
        if (-not $byPhase.ContainsKey($key)) { $byPhase[$key] = @{ mono = [Collections.Generic.List[double]]::new(); sqlite = [Collections.Generic.List[double]]::new(); cold = [Collections.Generic.List[double]]::new() } }
        if ($null -ne $e.elapsed_us) {
            $ms = [double]$e.elapsed_us / 1000.0
            $byPhase[$key].mono.Add($ms)
            if ($r.iteration -eq '1') { $byPhase[$key].cold.Add($ms) }
        }
        if ($null -ne $e.sqlite_us) { $byPhase[$key].sqlite.Add([double]$e.sqlite_us / 1000.0) }
    }
}
$phases = [ordered]@{}
foreach ($key in ($byPhase.Keys | Sort-Object)) {
    $phases[$key] = [ordered]@{
        elapsed_ms = Stat $byPhase[$key].mono.ToArray()
        sqlite_reported_ms = Stat $byPhase[$key].sqlite.ToArray()
        cold_iteration_elapsed_ms = Stat $byPhase[$key].cold.ToArray()
    }
}

$samplerRows = Read-Jsonl "$OutDir\sampler.jsonl"
$samples = @($samplerRows | Where-Object kind -eq 'sample')
$samplerErrors = @($samples | ForEach-Object { $_.errors } | Where-Object { $_ })
if (-not $samples.Count) { $problems.Add('sampler produced no samples') }
if ($samplerErrors.Count) { $problems.Add("sampler errors: $($samplerErrors.Count)") }
$samplerEnd = @($samplerRows | Where-Object kind -eq 'end')
if (-not $samplerEnd.Count) { $problems.Add('sampler end record missing (killed or censored)') }
$counterStats = [ordered]@{}
$counterNames = @($samples | ForEach-Object { $_.counters.PSObject.Properties.Name } | Sort-Object -Unique)
foreach ($name in $counterNames) {
    $counterStats[$name] = Stat (@($samples | ForEach-Object { $_.counters.$name } | Where-Object { $null -ne $_ }) -as [double[]])
}
$msm = @($samples | ForEach-Object { $_.msmpeng_cpu_s } | Where-Object { $null -ne $_ })
$msmDelta = if ($msm.Count -ge 2) { [math]::Round($msm[-1] - $msm[0], 2) } else { $null }

$receipts = [ordered]@{}
foreach ($label in 'start', 'post-build', 'post-sampling') {
    $path = "$OutDir\receipt-$label.json"
    if (-not (Test-Path $path)) { $problems.Add("missing receipt-$label"); continue }
    try {
        $rc = Get-Content -Raw $path | ConvertFrom-Json -ErrorAction Stop
        $receipts[$label] = $rc
        foreach ($err in @($rc.errors)) { if ($err) { $problems.Add("receipt-${label}: $err") } }
    } catch { $problems.Add("unparseable receipt-$label") }
}
$build = $null
if (Test-Path "$OutDir\build.json") { $build = Get-Content -Raw "$OutDir\build.json" | ConvertFrom-Json } else { $problems.Add('missing build.json') }

$summary = [ordered]@{
    schema = 'pilot209-summary/v1'; arm = $Arm; shard = $Shard
    head = $env:GITHUB_SHA; run_id = $env:GITHUB_RUN_ID; run_attempt = $env:GITHUB_RUN_ATTEMPT
    build = $build
    iterations = [ordered]@{
        started = $starts.Count; ended = $ends.Count; passed = $okEnds.Count
        wall_s = @($ends | ForEach-Object { [ordered]@{ iteration = $_.iteration; cold = $_.cold; exit = $_.exit; wall_s = $_.wall_s } })
        loop_end = if ($loopEnd.Count) { $loopEnd[0] } else { $null }
    }
    phase_records = $records.Count; unpaired_sql_events = $unpairedSql
    phases = $phases
    sampler = [ordered]@{ samples = $samples.Count; errors = $samplerErrors; end = if ($samplerEnd.Count) { $samplerEnd[0] } else { $null }; counters = $counterStats; msmpeng_cpu_delta_s = $msmDelta }
    defender = [ordered]@{
        start = $receipts['start'].defender_status_readonly
        post_sampling = $receipts['post-sampling'].defender_status_readonly
        msmpeng_start = $receipts['start'].msmpeng_process
        msmpeng_post_sampling = $receipts['post-sampling'].msmpeng_process
    }
    problems = $problems
}
[IO.File]::WriteAllText("$OutDir\summary.json", ($summary | ConvertTo-Json -Depth 8), (New-Object Text.UTF8Encoding $false))

$md = [Text.StringBuilder]::new()
[void]$md.AppendLine("## pilot-209 shard $Shard ($Arm)")
[void]$md.AppendLine("head ``$env:GITHUB_SHA``; iterations started/ended/passed: $($starts.Count)/$($ends.Count)/$($okEnds.Count); phase records: $($records.Count); unpaired SQL events: $unpairedSql; sampler samples: $($samples.Count); MsMpEng CPU delta: $msmDelta s")
[void]$md.AppendLine('')
[void]$md.AppendLine('| phase | n | median ms | max ms | cold max ms | sqlite-reported max ms |')
[void]$md.AppendLine('|---|---|---|---|---|---|')
foreach ($key in $phases.Keys) {
    $p = $phases[$key]
    $f = { param($s, $field) if ($s) { '{0:N2}' -f $s[$field] } else { '-' } }
    [void]$md.AppendLine("| ``$key`` | $(if ($p.elapsed_ms) { $p.elapsed_ms.n } else { 0 }) | $(& $f $p.elapsed_ms 'median') | $(& $f $p.elapsed_ms 'max') | $(& $f $p.cold_iteration_elapsed_ms 'max') | $(& $f $p.sqlite_reported_ms 'max') |")
}
if ($problems.Count) {
    [void]$md.AppendLine('')
    [void]$md.AppendLine('### problems (job fails)')
    foreach ($p in $problems) { [void]$md.AppendLine("- $p") }
}
if ($env:GITHUB_STEP_SUMMARY) { [IO.File]::AppendAllText($env:GITHUB_STEP_SUMMARY, $md.ToString(), (New-Object Text.UTF8Encoding $false)) }
$md.ToString()
if ($problems.Count) {
    Write-Host "::error::pilot209 summary found $($problems.Count) problem(s)"
    exit 1
}
