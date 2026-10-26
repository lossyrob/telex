#Requires -Version 7.0
# Branch-only #203 diagnostic. Not for main.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$EvidenceRoot
)
Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
if ($env:GITHUB_ACTIONS -ne "true" -or $env:RUNNER_ENVIRONMENT -ne "github-hosted") {
    throw "restricted to disposable GitHub-hosted runners"
}

$tools = Join-Path $env:RUNNER_TEMP "diag203-tools"
$dumps = Join-Path $EvidenceRoot "dumps"
$probeDir = Join-Path $EvidenceRoot "capture-probe"
New-Item -ItemType Directory -Path $tools, $dumps, $probeDir -Force | Out-Null

$archive = Join-Path $tools "Procdump.zip"
Invoke-WebRequest -Uri "https://download.sysinternals.com/files/Procdump.zip" -OutFile $archive
Expand-Archive -LiteralPath $archive -DestinationPath $tools -Force
$procdump = Join-Path $tools "procdump64.exe"
$signature = Get-AuthenticodeSignature -LiteralPath $procdump
if ($signature.Status -ne "Valid" -or
    $signature.SignerCertificate.Subject -notmatch "O=Microsoft Corporation") {
    throw "ProcDump must have a valid Microsoft signature"
}

# WER LocalDumps as a second capture path (same as main CI).
$wer = "HKLM:\SOFTWARE\Microsoft\Windows\Windows Error Reporting\LocalDumps"
New-Item -Path $wer -Force | Out-Null
New-ItemProperty -Path $wer -Name DumpFolder -Value $dumps -PropertyType ExpandString -Force | Out-Null
New-ItemProperty -Path $wer -Name DumpType -Value 2 -PropertyType DWord -Force | Out-Null
New-ItemProperty -Path $wer -Name DumpCount -Value 20 -PropertyType DWord -Force | Out-Null

# AeDebug postmortem (ProcDump) captures second-chance and fail-fast exits.
& $procdump -accepteula -ma -i $dumps
if ($LASTEXITCODE -ne 0) { throw "installing postmortem capture failed" }

# Preflight: a fail-fast probe must exit 0xc0000409 and leave a dump.
$probe = Join-Path $probeDir "diag203-fastfail-probe.exe"
rustc .github\diag-203\fastfail_probe.rs -g -o $probe
if ($LASTEXITCODE -ne 0) { throw "probe did not compile" }
$child = Start-Process -FilePath $probe -PassThru -NoNewWindow
if (-not $child.WaitForExit(60000)) { Stop-Process -Id $child.Id -Force; throw "probe timed out" }
$child.Refresh()
$status = [BitConverter]::ToUInt32([BitConverter]::GetBytes([int]$child.ExitCode), 0)
Start-Sleep -Seconds 10
$probeDumps = @(Get-ChildItem -LiteralPath $dumps -Filter "diag203-fastfail-probe*.dmp" -File)
$result = "probe exit=0x{0:x8} dumps=$($probeDumps.Count)" -f $status
$result | Tee-Object -FilePath (Join-Path $probeDir "result.txt")
if ($status -ne [Convert]::ToUInt32("c0000409", 16)) { throw "probe exit status changed: $result" }
if ($probeDumps.Count -eq 0) { throw "fail-fast capture produced no dump: $result" }
$probeDumps | Move-Item -Destination $probeDir
"DIAG203_PROCDUMP=$procdump" >> $env:GITHUB_ENV
