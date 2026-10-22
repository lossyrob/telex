#Requires -Version 7.0
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Candidate,

    [Parameter(Mandatory = $true)]
    [string]$ExpectedHead,

    [Parameter(Mandatory = $true)]
    [string]$Report
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

. (Join-Path $PSScriptRoot "released_v021_process_helpers.ps1")

$releaseTag = "v0.2.1"
$releaseCommit = "212b76a4c586101bdc2a53264e2a4c3e2326671a"
$assetName = "telex-v0.2.1-x86_64-pc-windows-msvc.zip"
$assetDigest = "f5ffdaaa252d21edeb10d086a34bdcc67327b96db541e422ea725458d33090d0"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$candidatePath = (Resolve-Path -LiteralPath $Candidate).Path
$reportPath = [IO.Path]::GetFullPath($Report)
$evidenceRoot = Split-Path -Parent $reportPath
$logs = Join-Path $evidenceRoot "logs"
$downloads = Join-Path $evidenceRoot "downloads"
$extract = Join-Path $evidenceRoot "released"
$volumeRoot = [IO.Path]::GetPathRoot($env:RUNNER_TEMP)
$executionRoot = Join-Path $volumeRoot ("telex-v021-ci-" + [guid]::NewGuid().ToString("N"))
$rootMarker = Join-Path $evidenceRoot "execution-root.txt"
$commands = Join-Path $evidenceRoot "commands.txt"
$failure = $null
$installedCandidate = $null
$cap = $null
$runDir = $null
$daemonExited = $false
$rootOwned = $false
$processCleanupSafe = $true

New-Item -ItemType Directory -Path $evidenceRoot, $logs, $downloads, $extract -Force |
    Out-Null
[IO.File]::WriteAllText($rootMarker, $executionRoot, [Text.UTF8Encoding]::new($false))
[IO.File]::WriteAllText($commands, "", [Text.UTF8Encoding]::new($false))

$result = [ordered]@{
    schemaVersion = 1
    result = "failed"
    expectedHead = $ExpectedHead
    candidate = $null
    releasedWriter = $null
    isolation = [ordered]@{
        executionRoot = $executionRoot
        executionRootParentIsVolumeRoot =
            ((Split-Path -Parent $executionRoot).TrimEnd("\") -eq $volumeRoot.TrimEnd("\"))
        protectedRoot = $executionRoot
        parentAclMutation = $false
        workstationState = $false
        sharedDb = $false
        userInstall = $false
        rootOwned = $false
        processCleanupSafe = $true
        cleanupSafe = $false
        cleanupCompleted = $false
    }
    oldWriterManifest = $null
    runtimeProof = $null
    commandsFile = $commands
    logsDirectory = $logs
}

function Protect-ProofRootAcl([string]$Path) {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    & icacls.exe $Path /inheritance:r /grant:r `
        "$($identity.Name):(OI)(CI)F" `
        "*S-1-5-18:(OI)(CI)F" `
        "*S-1-5-32-544:(OI)(CI)F" | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "failed to protect proof root $Path"
    }
}

function Invoke-Logged {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name,

        [Parameter(Mandatory = $true)]
        [string]$Executable,

        [string[]]$Arguments = @(),

        [hashtable]$Environment = @{},

        [switch]$Isolated,

        [int]$TimeoutSeconds = 120
    )

    Invoke-BoundedLoggedProcess `
        -LogsDirectory $logs `
        -CommandsFile $commands `
        -Name $Name `
        -Executable $Executable `
        -Arguments $Arguments `
        -Environment $Environment `
        -Isolated:$Isolated `
        -TimeoutSeconds $TimeoutSeconds
}

function Require-Success([object]$Run) {
    if ($Run.ExitCode -ne 0) {
        throw "$($Run.Name) failed with exit code $($Run.ExitCode)"
    }
}

function Get-FileIdentity([string]$Path) {
    $fileId = (& fsutil.exe file queryfileid $Path 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -ne 0) {
        throw "failed to read file identity for $Path"
    }
    $drive = [IO.Path]::GetPathRoot($Path).TrimEnd("\")
    $volume = Get-CimInstance Win32_LogicalDisk -Filter "DeviceID='$drive'"
    [ordered]@{
        fileId = $fileId
        volumeSerial = $volume.VolumeSerialNumber
    }
}

try {
    $repositoryHead = (git -C $repoRoot rev-parse HEAD).Trim()
    if ($repositoryHead -ne $ExpectedHead) {
        throw "repository HEAD $repositoryHead does not match expected candidate $ExpectedHead"
    }
    if (@(git -C $repoRoot status --porcelain).Count -ne 0) {
        throw "repository is not clean"
    }
    if (-not $result.isolation.executionRootParentIsVolumeRoot) {
        throw "proof root is not a direct child of the volume root"
    }

    New-ExclusiveProofRoot -Path $executionRoot -VolumeRoot $volumeRoot
    $rootOwned = $true
    Protect-ProofRootAcl $executionRoot
    $rootAcl = Get-Acl -LiteralPath $executionRoot
    $result.isolation.owner = $rootAcl.Owner
    $result.isolation.sddl = $rootAcl.GetSecurityDescriptorSddlForm(
        [Security.AccessControl.AccessControlSections]::All
    )

    $telexHome = Join-Path $executionRoot "home"
    $runDir = Join-Path $executionRoot "run"
    $state = Join-Path $executionRoot "state"
    $install = Join-Path $executionRoot "install"
    $fixtureTarget = Join-Path $executionRoot "fixture-target"
    $db = Join-Path $executionRoot "telex.db"
    New-Item -ItemType Directory `
        -Path $telexHome, $runDir, $state, $install, $fixtureTarget -Force |
        Out-Null
    $config = Join-Path $telexHome "config.toml"
    [IO.File]::WriteAllText($config, "", [Text.UTF8Encoding]::new($false))

    $archive = Join-Path $downloads $assetName
    $sidecar = "$archive.sha256"
    $assetUrl = "https://github.com/lossyrob/telex/releases/download/$releaseTag/$assetName"
    Invoke-WebRequest -Uri $assetUrl -OutFile $archive
    Invoke-WebRequest -Uri "$assetUrl.sha256" -OutFile $sidecar
    $archiveHash = (Get-FileHash -Algorithm SHA256 $archive).Hash.ToLowerInvariant()
    $sidecarHash = ((Get-Content -Raw $sidecar).Trim() -split "\s+")[0].ToLowerInvariant()
    if ($archiveHash -ne $assetDigest -or $sidecarHash -ne $assetDigest) {
        throw "released v0.2.1 archive checksum mismatch"
    }
    Expand-Archive -LiteralPath $archive -DestinationPath $extract -Force
    $releasedExe = Join-Path $extract "telex.exe"

    $probeCounter = Join-Path $executionRoot "admission-probe.log"
    $childMarker = Join-Path $executionRoot "child-admission-held.marker"
    $parentMarker = Join-Path $executionRoot "parent-admission-held.marker"
    $environment = @{
        TELEX_HOME = $telexHome
        TELEX_RUN_DIR = $runDir
        TELEX_CONFIG = $config
        TELEX_DB = $db
        TELEX_INSTALL_ROOT = $install
        TELEX_SESSION_ID = "released-v021-ci-proof"
        TELEX_RECONNECT_GRACE_MS = "3000"
        TELEX_LIVENESS_WINDOW_SECS = "0"
        LOCALAPPDATA = $state
        APPDATA = (Join-Path $executionRoot "appdata")
        USERPROFILE = (Join-Path $executionRoot "user")
        HOME = (Join-Path $executionRoot "user")
        TEMP = (Join-Path $executionRoot "tmp")
        TMP = (Join-Path $executionRoot "tmp")
        TELEX_LAUNCHER_ACTIVE = "1"
        TELEX_NO_MODIFY_PATH = "1"
        TELEX_TEST_ADMISSION_PROBE_COUNTER = $probeCounter
        TELEX_TEST_CHILD_ADMISSION_HELD_DELAY_MS = "1"
        TELEX_TEST_CHILD_ADMISSION_HELD_MARKER = $childMarker
        TELEX_TEST_PARENT_ADMISSION_MARKER = $parentMarker
    }
    New-Item -ItemType Directory `
        -Path $environment.APPDATA, $environment.USERPROFILE, $environment.TEMP -Force |
        Out-Null

    $releasedVersionRun = Invoke-Logged "released-version" $releasedExe `
        @("--json", "version", "--root", $install) $environment -Isolated
    Require-Success $releasedVersionRun
    $releasedVersion = $releasedVersionRun.Stdout | ConvertFrom-Json
    if ($releasedVersion.version.package_version -ne "0.2.1" -or
        $releasedVersion.version.build_id -ne $releaseCommit) {
        throw "downloaded binary is not the expected released v0.2.1 build"
    }

    $candidateVersionRun = Invoke-Logged "candidate-version" $candidatePath `
        @("--json", "version", "--root", $install) $environment -Isolated
    Require-Success $candidateVersionRun
    $candidateVersion = $candidateVersionRun.Stdout | ConvertFrom-Json
    if ($candidateVersion.version.build_id -ne $ExpectedHead -or
        $candidateVersion.version.package_version -ne "0.3.0" -or
        $candidateVersion.version.application_bootstrap_admission_version -ne 1) {
        throw "candidate version metadata does not bind the exact 0.3.0 proof head"
    }

    $tag = "v0.3.0-ci-$($ExpectedHead.Substring(0, 8))"
    $upgradeRun = Invoke-Logged "released-upgrade" $releasedExe @(
        "--json", "upgrade",
        "--from", $candidatePath,
        "--version", $tag,
        "--root", $install,
        "--skip-drain",
        "--force"
    ) $environment -Isolated
    Require-Success $upgradeRun

    $manifestPath = Join-Path $install "versions\$tag\manifest.json"
    $installedCandidate = Join-Path $install "versions\$tag\telex.exe"
    $manifestBytesBefore = [IO.File]::ReadAllBytes($manifestPath)
    $manifestHashBefore = (Get-FileHash -Algorithm SHA256 $manifestPath).Hash.ToLowerInvariant()
    $manifestMtimeBefore = (Get-Item $manifestPath).LastWriteTimeUtc.ToString("O")
    $manifestIdentityBefore = Get-FileIdentity $manifestPath
    $manifest = Get-Content -Raw $manifestPath | ConvertFrom-Json
    if ($manifest.PSObject.Properties.Name -contains
        "application_bootstrap_admission_version") {
        throw "released v0.2.1 unexpectedly persisted the admission field"
    }
    if ($manifest.package_version -ne "0.3.0" -or
        $manifest.build_id -ne $ExpectedHead) {
        throw "released writer did not bind the exact candidate"
    }
    $candidateHash = (Get-FileHash -Algorithm SHA256 $candidatePath).Hash.ToLowerInvariant()
    $installedHash =
        (Get-FileHash -Algorithm SHA256 $installedCandidate).Hash.ToLowerInvariant()
    if ($candidateHash -ne $installedHash) {
        throw "installed candidate bytes differ from the exact built candidate"
    }

    $fixtureManifest =
        Join-Path $repoRoot "tests\fixtures\application-client-consumer\Cargo.toml"
    $fixtureBuild = Invoke-Logged "fixture-build" "cargo.exe" @(
        "build",
        "--manifest-path", $fixtureManifest,
        "--no-default-features",
        "--features", "sqlite"
    ) @{ CARGO_TARGET_DIR = $fixtureTarget } -TimeoutSeconds 900
    Require-Success $fixtureBuild
    $fixtureExe =
        Join-Path $fixtureTarget "debug\telex-application-client-consumer.exe"

    $fixtureEnvironment = $environment.Clone()
    $fixtureEnvironment.Remove("TELEX_LAUNCHER_ACTIVE")
    $fixtureEnvironment["TELEX_FIXTURE_TRUSTED_ROOT"] = $install
    $fixtureEnvironment["TELEX_FIXTURE_RUN_ID"] = "released-v021-ci"
    $processCleanupSafe = $false
    $fixtureRun = Invoke-Logged "public-attach" $fixtureExe @("attach") `
        $fixtureEnvironment -Isolated
    Require-Success $fixtureRun
    if ($fixtureRun.Stdout -notmatch "attach=ready" -or
        $fixtureRun.Stdout -notmatch "probe-result=ok") {
        throw "public Application Client attach did not report successful use"
    }

    $manifestHashAfter = (Get-FileHash -Algorithm SHA256 $manifestPath).Hash.ToLowerInvariant()
    $manifestMtimeAfter = (Get-Item $manifestPath).LastWriteTimeUtc.ToString("O")
    $manifestIdentityAfter = Get-FileIdentity $manifestPath
    $manifestBytesAfter = [IO.File]::ReadAllBytes($manifestPath)
    if ($manifestHashAfter -ne $manifestHashBefore -or
        $manifestMtimeAfter -ne $manifestMtimeBefore -or
        $manifestIdentityAfter.fileId -ne $manifestIdentityBefore.fileId -or
        $manifestIdentityAfter.volumeSerial -ne $manifestIdentityBefore.volumeSerial -or
        -not [Linq.Enumerable]::SequenceEqual($manifestBytesBefore, $manifestBytesAfter)) {
        throw "runtime admission mutated the released-writer manifest"
    }

    $probeCount = if (Test-Path -LiteralPath $probeCounter) {
        @(Get-Content -LiteralPath $probeCounter | Where-Object { $_ -eq "probe" }).Count
    } else {
        0
    }
    if ($probeCount -ne 1) {
        throw "expected one exact pre-spawn admission probe, saw $probeCount"
    }
    if (-not (Test-Path -LiteralPath $parentMarker) -or
        -not (Test-Path -LiteralPath $childMarker)) {
        throw "parent or child selector-admission evidence is missing"
    }

    $capFiles = @(Get-ChildItem -LiteralPath $runDir -Recurse -File -Filter "*.cap")
    if ($capFiles.Count -ne 1) {
        throw "expected one capability record, saw $($capFiles.Count)"
    }
    $cap = Get-Content -Raw $capFiles[0].FullName | ConvertFrom-Json
    if ($cap.bootstrap_admission_version -ne 1 -or
        $null -eq $cap.startup_executable_file_identity -or
        $null -eq $cap.server_pid -or
        $null -eq $cap.server_start_time) {
        throw "capability record lacks admitted startup identity"
    }
    $cleanupSafe = $false
    $stopRun = Invoke-Logged "daemon-stop" $installedCandidate `
        @("--json", "daemon", "stop", "--drain") $environment -Isolated
    Require-Success $stopRun
    $daemonExited = Wait-ExactProcessExit `
        ([int]$cap.server_pid) ([long]$cap.server_start_time) 20
    if (-not $daemonExited) {
        throw "proof daemon did not exit"
    }
    $processCleanupSafe = $true

    $result.candidate = [ordered]@{
        commit = $ExpectedHead
        tree = (git -C $repoRoot show -s --format=%T $ExpectedHead).Trim()
        packageVersion = $candidateVersion.version.package_version
        buildId = $candidateVersion.version.build_id
        executable = $candidatePath
        sha256 = $candidateHash
        installedExecutable = $installedCandidate
        installedSha256 = $installedHash
    }
    $result.releasedWriter = [ordered]@{
        release = "https://github.com/lossyrob/telex/releases/tag/v0.2.1"
        tag = $releaseTag
        commit = $releaseCommit
        asset = $assetName
        assetSha256 = $archiveHash
        sidecarSha256 = $sidecarHash
        executable = $releasedExe
        executableSha256 =
            (Get-FileHash -Algorithm SHA256 $releasedExe).Hash.ToLowerInvariant()
        reportedPackageVersion = $releasedVersion.version.package_version
        reportedBuildId = $releasedVersion.version.build_id
    }
    $result.oldWriterManifest = [ordered]@{
        path = $manifestPath
        tag = $manifest.tag
        packageVersion = $manifest.package_version
        buildId = $manifest.build_id
        admissionFieldPresent = $false
        sha256Before = $manifestHashBefore
        sha256After = $manifestHashAfter
        mtimeBefore = $manifestMtimeBefore
        mtimeAfter = $manifestMtimeAfter
        fileIdentityBefore = $manifestIdentityBefore
        fileIdentityAfter = $manifestIdentityAfter
        bytesUnchanged = $true
    }
    $result.runtimeProof = [ordered]@{
        preSpawnProbeCount = $probeCount
        parentAdmissionObserved = $true
        childAdmissionObserved = $true
        capPath = $capFiles[0].FullName
        capBootstrapAdmissionVersion = $cap.bootstrap_admission_version
        capStartupExecutableFileIdentity = $cap.startup_executable_file_identity
        capServerPid = $cap.server_pid
        capServerStartTime = $cap.server_start_time
        publicAttachReady = $true
        daemonStopExitCode = $stopRun.ExitCode
        daemonExited = $daemonExited
    }
    $result.result = "pass"
} catch {
    $failure = $_
    $result.error = $_.Exception.Message
} finally {
    if ($null -eq $cap -and $null -ne $runDir -and
        (Test-Path -LiteralPath $runDir -ErrorAction SilentlyContinue)) {
        $cleanupCaps = @(Get-ChildItem -LiteralPath $runDir -Recurse -File `
            -Filter "*.cap" -ErrorAction SilentlyContinue)
        if ($cleanupCaps.Count -eq 1) {
            try {
                $cap = Get-Content -Raw $cleanupCaps[0].FullName | ConvertFrom-Json
            } catch {
                $result.cleanupError = "could not read cleanup capability evidence"
            }
        }
    }
    if (-not $processCleanupSafe -and $null -ne $installedCandidate -and
        (Test-Path -LiteralPath $installedCandidate)) {
        try {
            $recoveryStop = Invoke-Logged "daemon-stop-after-failure" $installedCandidate `
                @("--json", "daemon", "stop", "--drain") $environment -Isolated
            if ($recoveryStop.ExitCode -eq 0 -and $null -ne $cap) {
                $daemonExited = Wait-ExactProcessExit `
                    ([int]$cap.server_pid) ([long]$cap.server_start_time) 20
                $processCleanupSafe = $daemonExited
            }
        } catch {
            $result.cleanupError = $_.Exception.Message
        }
    }
    $result.isolation.rootOwned = $rootOwned
    $result.isolation.processCleanupSafe = $processCleanupSafe
    $result.isolation.cleanupSafe = $rootOwned -and $processCleanupSafe
    [IO.File]::WriteAllText(
        $reportPath,
        ($result | ConvertTo-Json -Depth 12),
        [Text.UTF8Encoding]::new($false)
    )
}

if ($null -ne $failure) {
    throw $failure
}

Write-Output "report=$reportPath"
Write-Output "execution_root=$executionRoot"
