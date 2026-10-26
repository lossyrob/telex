#Requires -Version 7.0
# Branch-only #203 diagnostic. Not for main.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$EvidenceRoot
)
Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$dumps = @(Get-ChildItem -LiteralPath (Join-Path $EvidenceRoot "dumps") -Filter "*.dmp" -File -ErrorAction SilentlyContinue)
if ($dumps.Count -eq 0) {
    Write-Output "No native dumps to analyze"
    exit 0
}

$cdb = Join-Path ${env:ProgramFiles(x86)} "Windows Kits\10\Debuggers\x64\cdb.exe"
if (-not (Test-Path -LiteralPath $cdb)) {
    $installer = Join-Path $env:RUNNER_TEMP "diag203-sdksetup.exe"
    Invoke-WebRequest -Uri "https://go.microsoft.com/fwlink/?linkid=2382321" -OutFile $installer
    $signature = Get-AuthenticodeSignature -LiteralPath $installer
    if ($signature.Status -ne "Valid" -or
        $signature.SignerCertificate.Subject -notmatch "O=Microsoft Corporation") {
        throw "Windows SDK installer must have a valid Microsoft signature"
    }
    $setup = Start-Process -FilePath $installer -PassThru -ArgumentList `
        "/features OptionId.WindowsDesktopDebuggers /quiet /norestart /ceip off"
    if (-not $setup.WaitForExit(600000)) { Stop-Process -Id $setup.Id -Force; throw "debugger install timed out" }
    if (-not (Test-Path -LiteralPath $cdb)) { throw "debugger install failed: $($setup.ExitCode)" }
}

$analysis = Join-Path $EvidenceRoot "native-stacks"
$cache = Join-Path $env:RUNNER_TEMP "diag203-symcache"
New-Item -ItemType Directory -Path $analysis, $cache -Force | Out-Null
$deps = (Resolve-Path "target\debug\deps").Path
$symbols = "$deps;srv*$cache*https://msdl.microsoft.com/download/symbols"
# Only analyze a bounded number of dumps; they are all from the same harness.
foreach ($dump in ($dumps | Sort-Object LastWriteTime | Select-Object -First 6)) {
    $log = Join-Path $analysis "$($dump.BaseName).txt"
    & $cdb -z $dump.FullName -y $symbols -i $deps -logo $log `
        -c ".symopt+0x40; !analyze -v; .exr -1; .ecxr; kn 80; ~*kn 60; !heap -s; q" | Out-Null
    Write-Output "==== $($dump.Name)"
    Get-Content -LiteralPath $log |
        Select-String -Pattern 'ExceptionCode|FAILURE_BUCKET_ID|EXCEPTION_CODE_STR|FAST_FAIL|STACK_TEXT|SYMBOL_NAME|mio|named_pipe|tokio' |
        Select-Object -First 60 | ForEach-Object { $_.Line }
}
