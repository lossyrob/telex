#Requires -Version 7.0
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$EvidenceRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
if ($env:GITHUB_ACTIONS -ne "true" -or $env:RUNNER_ENVIRONMENT -ne "github-hosted") {
    throw "Postmortem setup is restricted to disposable GitHub-hosted runners"
}
if (-not [System.IO.Path]::IsPathFullyQualified($EvidenceRoot)) {
    throw "EvidenceRoot must be absolute"
}

$tools = Join-Path $env:RUNNER_TEMP "telex-postmortem-tools"
$provenance = Join-Path $EvidenceRoot "tool-provenance"
$probe = Join-Path $EvidenceRoot "capture-probe"
$dumps = Join-Path $EvidenceRoot "dumps"
New-Item -ItemType Directory -Path $tools, $provenance, $probe, $dumps -Force | Out-Null
$archive = Join-Path $tools "Procdump.zip"
Invoke-WebRequest -Uri "https://download.sysinternals.com/files/Procdump.zip" `
    -OutFile $archive
Expand-Archive -LiteralPath $archive -DestinationPath $tools -Force
$procdump = Join-Path $tools "procdump64.exe"
$signature = Get-AuthenticodeSignature -LiteralPath $procdump
if ($signature.Status -ne "Valid" -or
    $signature.SignerCertificate.Subject -notmatch "O=Microsoft Corporation") {
    throw "ProcDump must have a valid Microsoft signature"
}
Get-FileHash -Algorithm SHA256 -LiteralPath $archive, $procdump |
    Format-List | Out-File (Join-Path $provenance "hashes.txt") -Encoding utf8
$signature | Format-List Status, SignerCertificate |
    Out-File (Join-Path $provenance "signature.txt") -Encoding utf8

$probeExe = Join-Path $probe "telex-capture-probe.exe"
rustc tests\fixtures\windows_crash_probe.rs -g -o $probeExe
if ($LASTEXITCODE -ne 0) {
    throw "native crash capture probe did not compile"
}

# AeDebug captures second-chance failures even when WER produces no dump.
# Verify capture and the original abnormal exit before running any Telex tests.
$probeWer = "HKLM:\SOFTWARE\Microsoft\Windows\Windows Error Reporting\LocalDumps\telex-capture-probe.exe"
New-Item -Path $probeWer -Force | Out-Null
New-ItemProperty -Path $probeWer -Name DumpFolder -Value $probe `
    -PropertyType ExpandString -Force | Out-Null
New-ItemProperty -Path $probeWer -Name DumpType -Value 2 `
    -PropertyType DWord -Force | Out-Null
& $procdump -accepteula -ma -i $probe
if ($LASTEXITCODE -ne 0) {
    throw "installing probe postmortem capture failed"
}
try {
    $child = Start-Process -FilePath $probeExe -PassThru
    if (-not $child.WaitForExit(60000)) {
        Stop-Process -Id $child.Id -Force
        throw "native crash capture probe timed out"
    }
    $child.Refresh()
    $status = [BitConverter]::ToUInt32([BitConverter]::GetBytes([int]$child.ExitCode), 0)
    # PowerShell's unsuffixed 0xc0000005 literal is signed; compare unsigned values.
    $expectedStatus = [Convert]::ToUInt32("c0000005", 16)
    if ($status -ne $expectedStatus) {
        throw ("capture changed the probe exit status to 0x{0:x8}" -f $status)
    }
    $captured = @(Get-ChildItem -LiteralPath $probe -Filter "*.dmp" -File)
    if ($captured.Count -eq 0) {
        throw "postmortem preflight produced no native dump"
    }
    "pid=$($child.Id) exit=0x{0:x8} dumps=$($captured.Count)" -f $status |
        Set-Content -LiteralPath (Join-Path $probe "result.txt") -Encoding utf8
} finally {
    & $procdump -u
    if ($LASTEXITCODE -ne 0) {
        throw "uninstalling probe postmortem capture failed"
    }
    Remove-Item -LiteralPath $probeWer
}

& $procdump -accepteula -ma -i $dumps
if ($LASTEXITCODE -ne 0) {
    throw "installing Telex postmortem capture failed"
}
"TELEX_POSTMORTEM_EXE=$procdump" >> $env:GITHUB_ENV
