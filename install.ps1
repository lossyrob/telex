<#
.SYNOPSIS
  Install telex from GitHub Releases.

  irm https://raw.githubusercontent.com/lossyrob/telex/main/install.ps1 | iex

  Environment variables:
    TELEX_INSTALL_ROOT versioned install root (default: $env:LOCALAPPDATA\telex)
    TELEX_INSTALL_DIR  legacy override; if it ends in \bin, its parent is used as TELEX_INSTALL_ROOT
    TELEX_VERSION      version tag to install (default: latest)
    GITHUB_TOKEN       optional, raises GitHub API rate limits
#>
$ErrorActionPreference = 'Stop'

$repo = 'lossyrob/telex'
$installRoot = if ($env:TELEX_INSTALL_ROOT) {
    $env:TELEX_INSTALL_ROOT
} elseif ($env:TELEX_INSTALL_DIR) {
    if ((Split-Path -Leaf $env:TELEX_INSTALL_DIR) -eq 'bin') {
        Split-Path -Parent $env:TELEX_INSTALL_DIR
    } else {
        $env:TELEX_INSTALL_DIR
    }
} else {
    Join-Path $env:LOCALAPPDATA 'telex'
}
$binDir = Join-Path $installRoot 'bin'

$arch = $env:PROCESSOR_ARCHITECTURE
switch ($arch) {
    'AMD64' { $target = 'x86_64-pc-windows-msvc' }
    'ARM64' { $target = 'aarch64-pc-windows-msvc' }
    default {
        throw "no prebuilt Windows binary for $arch yet - install with: cargo install --git https://github.com/$repo --features entra"
    }
}

$headers = @{ 'User-Agent' = 'telex-install' }
if ($env:GITHUB_TOKEN) { $headers['Authorization'] = "Bearer $($env:GITHUB_TOKEN)" }

# Resolve the version tag.
$tag = $env:TELEX_VERSION
if (-not $tag) {
    $rel = Invoke-RestMethod -Headers $headers "https://api.github.com/repos/$repo/releases/latest"
    $tag = $rel.tag_name
    if (-not $tag) { throw 'could not determine the latest release tag (is a release published?)' }
}

$asset = "telex-$tag-$target.zip"
$url = "https://github.com/$repo/releases/download/$tag/$asset"

$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("telex-install-" + [System.Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
try {
    Write-Host "Downloading $asset ..."
    $zip = Join-Path $tmp $asset
    Invoke-WebRequest -Headers $headers -Uri $url -OutFile $zip

    # Best-effort checksum verification.
    try {
        $sumFile = "$zip.sha256"
        Invoke-WebRequest -Headers $headers -Uri "$url.sha256" -OutFile $sumFile -ErrorAction Stop
        $expected = ((Get-Content $sumFile -Raw).Trim() -split '\s+')[0].ToLower()
        $actual = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
        if ($expected -ne $actual) { throw "checksum mismatch for $asset" }
        Write-Host 'Checksum OK.'
    } catch [System.Net.WebException] { } # no checksum published; skip

    Expand-Archive -Path $zip -DestinationPath $tmp -Force
    $payload = Join-Path $tmp 'telex.exe'
    & $payload --json upgrade --from $payload --version $tag --root $installRoot | Out-Null

    Write-Host ""
    Write-Host "Installed telex $tag under $installRoot"
    Write-Host "Launcher: $binDir\telex.exe"

    # Add to the user PATH if it is not already there.
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (($userPath -split ';') -notcontains $binDir) {
        [Environment]::SetEnvironmentVariable('Path', "$userPath;$binDir", 'User')
        Write-Host "Added $binDir to your user PATH (restart your terminal to pick it up)."
    }
    Write-Host "Next:  telex skill"
    Write-Host "Copilot plugin marketplace:"
    Write-Host "  copilot plugin marketplace add lossyrob/telex#$tag"
    Write-Host "  copilot plugin install telex@telex"
}
finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
