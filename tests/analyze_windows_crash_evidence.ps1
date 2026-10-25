#Requires -Version 7.0
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$EvidenceRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
if ($env:GITHUB_ACTIONS -ne "true" -or $env:RUNNER_ENVIRONMENT -ne "github-hosted") {
    throw "Native dump analysis is restricted to disposable GitHub-hosted runners"
}
$dumps = @(Get-ChildItem -LiteralPath (Join-Path $EvidenceRoot "dumps") `
    -Filter "*.dmp" -File -ErrorAction Stop)
if ($dumps.Count -eq 0) {
    Write-Output "No native Telex dump to analyze"
    exit 0
}

$cdb = Join-Path ${env:ProgramFiles(x86)} "Windows Kits\10\Debuggers\x64\cdb.exe"
if (-not (Test-Path -LiteralPath $cdb)) {
    $installer = Join-Path $env:RUNNER_TEMP "telex-debuggers-sdksetup.exe"
    # Windows SDK 10.0.26100.9457: install only the console debugging tools.
    Invoke-WebRequest -Uri "https://go.microsoft.com/fwlink/?linkid=2382321" `
        -OutFile $installer
    $signature = Get-AuthenticodeSignature -LiteralPath $installer
    if ($signature.Status -ne "Valid" -or
        $signature.SignerCertificate.Subject -notmatch "O=Microsoft Corporation") {
        throw "Windows SDK installer must have a valid Microsoft signature"
    }
    $setup = Start-Process -FilePath $installer -ArgumentList `
        "/features OptionId.WindowsDesktopDebuggers /quiet /norestart /ceip off" -PassThru
    if (-not $setup.WaitForExit(600000)) {
        Stop-Process -Id $setup.Id -Force
        throw "debugging tools installation timed out"
    }
    if ($setup.ExitCode -notin 0, 3010 -or -not (Test-Path -LiteralPath $cdb)) {
        throw "debugging tools installation failed: $($setup.ExitCode)"
    }
}

$analysis = Join-Path $EvidenceRoot "native-stacks"
$cache = Join-Path $env:RUNNER_TEMP "telex-native-symbol-cache"
$symbols = (Resolve-Path -LiteralPath "target\debug").Path
New-Item -ItemType Directory -Path $analysis, $cache -Force | Out-Null
foreach ($dump in $dumps) {
    $log = Join-Path $analysis "$($dump.BaseName).txt"
    & $cdb -z $dump.FullName -y "$symbols;srv*$cache*https://msdl.microsoft.com/download/symbols" `
        -i $symbols -logo $log -c "!analyze -v; .exr -1; .ecxr; kv; ~*kp; q"
    if ($LASTEXITCODE -ne 0) {
        throw "native dump analysis failed for $($dump.Name)"
    }
}
