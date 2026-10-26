# Issue #209 pilot (non-merging). Read-only environment receipt: image, OS,
# CPU, memory, logical volumes and free space, physical disks, and READ-ONLY
# Defender status (Get-MpComputerStatus). Changes nothing on the runner.
# Errors are retained in the receipt; the summarize step fails the job on them.
param(
    [Parameter(Mandatory)] [string] $Out,
    [Parameter(Mandatory)] [string] $Label
)
$ErrorActionPreference = 'Continue'
$receipt = [ordered]@{
    schema = 'pilot209-receipt/v1'
    label = $Label
    utc = (Get-Date).ToUniversalTime().ToString('o')
    errors = @()
}

function Add-Section([string] $Name, [scriptblock] $Read) {
    try {
        $receipt[$Name] = & $Read
    } catch {
        $receipt.errors += "${Name}: $($_.Exception.Message)"
    }
}

Add-Section image {
    [ordered]@{
        ImageOS = $env:ImageOS; ImageVersion = $env:ImageVersion
        RUNNER_NAME = $env:RUNNER_NAME; RUNNER_OS = $env:RUNNER_OS; RUNNER_ARCH = $env:RUNNER_ARCH
        RUNNER_ENVIRONMENT = $env:RUNNER_ENVIRONMENT
    }
}
Add-Section source {
    [ordered]@{
        GITHUB_SHA = $env:GITHUB_SHA; GITHUB_REF = $env:GITHUB_REF
        head = (git rev-parse HEAD 2>&1) -join ' '
        parents = (git log -1 --format=%P 2>&1) -join ' '
        run_id = $env:GITHUB_RUN_ID; run_attempt = $env:GITHUB_RUN_ATTEMPT; job = $env:GITHUB_JOB
    }
}
Add-Section os {
    $os = Get-CimInstance Win32_OperatingSystem -ErrorAction Stop
    [ordered]@{
        caption = $os.Caption; version = $os.Version; build = $os.BuildNumber
        last_boot_utc = $os.LastBootUpTime.ToUniversalTime().ToString('o')
        total_mb = [math]::Round($os.TotalVisibleMemorySize / 1024)
        free_mb = [math]::Round($os.FreePhysicalMemory / 1024)
    }
}
Add-Section cpu {
    $cpus = @(Get-CimInstance Win32_Processor -ErrorAction Stop)
    [ordered]@{
        name = $cpus[0].Name.Trim(); sockets = $cpus.Count
        cores = ($cpus | Measure-Object NumberOfCores -Sum).Sum
        logical = ($cpus | Measure-Object NumberOfLogicalProcessors -Sum).Sum
        dotnet_processor_count = [Environment]::ProcessorCount
        max_clock_mhz = $cpus[0].MaxClockSpeed
    }
}
Add-Section computer_system {
    $cs = Get-CimInstance Win32_ComputerSystem -ErrorAction Stop
    [ordered]@{ manufacturer = $cs.Manufacturer; model = $cs.Model; hypervisor_present = $cs.HypervisorPresent }
}
Add-Section logical_volumes {
    @(Get-CimInstance Win32_LogicalDisk -ErrorAction Stop | ForEach-Object {
        [ordered]@{
            id = $_.DeviceID; drive_type = $_.DriveType; fs = $_.FileSystem; label = $_.VolumeName
            size_gb = [math]::Round($_.Size / 1GB, 2); free_gb = [math]::Round($_.FreeSpace / 1GB, 2)
        }
    })
}
Add-Section partitions {
    @(Get-Partition -ErrorAction Stop | Where-Object DriveLetter | ForEach-Object {
        [ordered]@{ letter = [string]$_.DriveLetter; disk_number = $_.DiskNumber; size_gb = [math]::Round($_.Size / 1GB, 2) }
    })
}
Add-Section physical_disks {
    @(Get-PhysicalDisk -ErrorAction Stop | ForEach-Object {
        [ordered]@{
            device_id = $_.DeviceId; name = $_.FriendlyName; media = [string]$_.MediaType
            bus = [string]$_.BusType; size_gb = [math]::Round($_.Size / 1GB, 2)
        }
    })
}
Add-Section paths {
    [ordered]@{
        TEMP = $env:TEMP; dotnet_temp = [IO.Path]::GetTempPath(); RUNNER_TEMP = $env:RUNNER_TEMP
        GITHUB_WORKSPACE = $env:GITHUB_WORKSPACE; CARGO_HOME = $env:CARGO_HOME
    }
}
Add-Section defender_status_readonly {
    $s = Get-MpComputerStatus -ErrorAction Stop
    [ordered]@{
        AMServiceEnabled = $s.AMServiceEnabled; AntivirusEnabled = $s.AntivirusEnabled
        RealTimeProtectionEnabled = $s.RealTimeProtectionEnabled
        OnAccessProtectionEnabled = $s.OnAccessProtectionEnabled
        BehaviorMonitorEnabled = $s.BehaviorMonitorEnabled; IoavProtectionEnabled = $s.IoavProtectionEnabled
        AMRunningMode = $s.AMRunningMode; IsTamperProtected = $s.IsTamperProtected
        AMProductVersion = $s.AMProductVersion; AMEngineVersion = $s.AMEngineVersion
        AntivirusSignatureVersion = $s.AntivirusSignatureVersion
        AntivirusSignatureLastUpdated = [string]$s.AntivirusSignatureLastUpdated
        QuickScanAge = $s.QuickScanAge; FullScanAge = $s.FullScanAge; ComputerState = $s.ComputerState
    }
}
Add-Section msmpeng_process {
    $p = @(Get-Process -Name MsMpEng -ErrorAction Stop)
    [ordered]@{
        count = $p.Count
        cpu_s = if ($null -ne $p[0].CPU) { [math]::Round(($p | Measure-Object CPU -Sum).Sum, 2) } else { $null }
        working_set_mb = [math]::Round(($p | Measure-Object WorkingSet64 -Sum).Sum / 1MB, 1)
    }
}
if ($Label -ne 'start') {
    Add-Section toolchain {
        [ordered]@{
            rustc = (rustc -Vv 2>&1) -join '; '
            cargo = (cargo -V 2>&1) -join ' '
            rustup_active = (rustup show active-toolchain 2>&1) -join ' '
        }
    }
}

$json = $receipt | ConvertTo-Json -Depth 6
New-Item -ItemType Directory -Force -Path (Split-Path $Out) | Out-Null
[IO.File]::WriteAllText($Out, $json, (New-Object Text.UTF8Encoding $false))
$json
if ($receipt.errors.Count) {
    Write-Host "::warning::pilot209 receipt '$Label' recorded $($receipt.errors.Count) error(s); summarize will fail the job"
}
