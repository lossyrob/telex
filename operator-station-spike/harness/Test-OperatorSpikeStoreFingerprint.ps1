[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$helper = Join-Path $PSScriptRoot 'Get-OperatorSpikeStoreFingerprint.ps1'
$selfCheckRoot = Join-Path $PSScriptRoot ('.fingerprint-self-check-' + [guid]::NewGuid().ToString('N'))
$databasePath = Join-Path $selfCheckRoot 'Fingerprint.db'
$originalLocation = Get-Location

try {
    New-Item -ItemType Directory -Path $selfCheckRoot | Out-Null
    [System.IO.File]::WriteAllBytes($databasePath, [byte[]](0x54, 0x65, 0x6c, 0x65, 0x78))

    $details = & $helper -DatabasePath $databasePath -IncludeCanonicalPath
    $absoluteFingerprint = & $helper -DatabasePath $databasePath

    Push-Location $selfCheckRoot
    try {
        $relativeFingerprint = & $helper -DatabasePath '.\Fingerprint.db'
    }
    finally {
        Pop-Location
    }

    $caseAliasFingerprint = & $helper -DatabasePath $databasePath.ToUpperInvariant()

    if ($absoluteFingerprint -notmatch '^sha256:[0-9a-f]{64}$') {
        throw 'Fingerprint format is not sha256:<64 lowercase hexadecimal characters>.'
    }
    if ($absoluteFingerprint -ne $relativeFingerprint) {
        throw 'Relative and absolute aliases produced different fingerprints.'
    }
    if ($absoluteFingerprint -ne $caseAliasFingerprint) {
        throw 'Windows case aliases produced different fingerprints.'
    }
    if ($details.CanonicalPath.StartsWith('\\?\', [System.StringComparison]::Ordinal)) {
        throw 'The canonical path retained the Windows verbatim-path prefix.'
    }

    $normalizedPath = $details.CanonicalPath.Replace('\', '/').ToLowerInvariant()
    $sha256 = [System.Security.Cryptography.SHA256]::Create()
    try {
        $expectedHex = ([System.BitConverter]::ToString(
            $sha256.ComputeHash([System.Text.UTF8Encoding]::new($false).GetBytes($normalizedPath))
        )).Replace('-', '').ToLowerInvariant()
    }
    finally {
        $sha256.Dispose()
    }

    if ($absoluteFingerprint -ne "sha256:$expectedHex") {
        throw 'The fingerprint does not match SHA-256 of the normalized canonical path.'
    }

    $missingRejected = $false
    try {
        & $helper -DatabasePath (Join-Path $selfCheckRoot 'missing.db') | Out-Null
    }
    catch {
        $missingRejected = $true
    }
    if (-not $missingRejected) {
        throw 'A missing database path was not rejected.'
    }

    [pscustomobject]@{
        passed      = $true
        fingerprint = $absoluteFingerprint
        checks      = @(
            'format'
            'relative-alias'
            'case-alias'
            'verbatim-prefix-stripped'
            'sha256-normalized-path'
            'missing-file-rejected'
        )
    } | ConvertTo-Json -Depth 4
}
finally {
    Set-Location $originalLocation
    Remove-Item -LiteralPath $selfCheckRoot -Recurse -Force -ErrorAction SilentlyContinue
}
