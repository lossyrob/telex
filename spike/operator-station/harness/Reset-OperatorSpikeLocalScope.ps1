[CmdletBinding(SupportsShouldProcess, ConfirmImpact = 'High')]
param(
    [string] $AppDataDirectory = (Join-Path $env:APPDATA 'com.lossyrob.telex.operatorstationspike'),

    [switch] $Apply
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$runtimeDirectory = Join-Path $AppDataDirectory 'runtime'
$files = @(
    if (Test-Path -LiteralPath $runtimeDirectory -PathType Container) {
        Get-ChildItem -LiteralPath $runtimeDirectory -Filter '*.json' -File
    }
)

if (-not $Apply) {
    [pscustomobject]@{
        runtimeDirectory = $runtimeDirectory
        count            = $files.Count
        files            = @($files | ForEach-Object Name)
        applied          = $false
    } | ConvertTo-Json -Depth 4
    return
}

$removed = @()
foreach ($file in $files) {
    if ($PSCmdlet.ShouldProcess($file.FullName, 'Remove Operator Station spike local scope')) {
        Remove-Item -LiteralPath $file.FullName -Force
        $removed += $file.Name
    }
}

[pscustomobject]@{
    runtimeDirectory = $runtimeDirectory
    count            = $removed.Count
    files            = $removed
    applied          = $true
} | ConvertTo-Json -Depth 4
