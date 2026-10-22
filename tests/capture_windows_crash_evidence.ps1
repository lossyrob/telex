#Requires -Version 7.0
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern("^[A-Za-z0-9._-]+$")]
    [string]$EvidenceName,

    [string]$DumpDirectory = (Join-Path $env:RUNNER_TEMP "telex-crash-dumps"),

    [string]$SymbolDirectory = (Join-Path $env:RUNNER_TEMP "telex-crash-symbols"),

    [switch]$SnapshotSymbols
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$exe = "target\debug\telex.exe"
$pdb = "target\debug\telex.pdb"
if ($SnapshotSymbols) {
    if (-not (Test-Path -LiteralPath $exe) -or -not (Test-Path -LiteralPath $pdb)) {
        throw "matching telex.exe/telex.pdb is unavailable for $EvidenceName"
    }
    New-Item -ItemType Directory -Path $SymbolDirectory -Force | Out-Null
    Copy-Item -LiteralPath $exe `
        -Destination (Join-Path $SymbolDirectory "$EvidenceName-telex.exe") -Force
    Copy-Item -LiteralPath $pdb `
        -Destination (Join-Path $SymbolDirectory "$EvidenceName-telex.pdb") -Force
}

if (-not (Test-Path -LiteralPath $DumpDirectory -PathType Container)) {
    throw "crash dump directory is unavailable: $DumpDirectory"
}
$dumps = @(Get-ChildItem -LiteralPath $DumpDirectory -Filter "*.dmp" -File `
    -ErrorAction Stop)
if ($dumps.Count -eq 0) {
    exit 0
}

if (-not (Test-Path -LiteralPath $exe) -or -not (Test-Path -LiteralPath $pdb)) {
    throw "crash dump exists but current telex.exe/telex.pdb is unavailable"
}

New-Item -ItemType Directory -Path $SymbolDirectory -Force | Out-Null
Copy-Item -LiteralPath $exe `
    -Destination (Join-Path $SymbolDirectory "$EvidenceName-detected-telex.exe") -Force
Copy-Item -LiteralPath $pdb `
    -Destination (Join-Path $SymbolDirectory "$EvidenceName-detected-telex.pdb") -Force
$dumps.FullName | Set-Content `
    -LiteralPath (Join-Path $SymbolDirectory "$EvidenceName-dumps.txt") `
    -Encoding utf8
$dumps | ForEach-Object { Write-Error "native crash dump: $($_.FullName)" }
throw "Windows child process produced $($dumps.Count) crash dump(s)"
