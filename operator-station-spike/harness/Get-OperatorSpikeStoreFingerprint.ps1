[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string] $DatabasePath,

    [Alias('ShowPath')]
    [switch] $IncludeCanonicalPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if ($env:OS -ne 'Windows_NT') {
    throw 'The operator-station spike fingerprint helper supports Windows paths only.'
}

$databaseExists = try {
    Test-Path -LiteralPath $DatabasePath -PathType Leaf
}
catch {
    throw 'The database path could not be inspected.'
}

if (-not $databaseExists) {
    throw 'The database must already exist before its store fingerprint is computed.'
}

if (-not ('OperatorStationSpike.NativePath' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Win32.SafeHandles;

namespace OperatorStationSpike
{
    public static class NativePath
    {
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern uint GetFinalPathNameByHandleW(
            SafeFileHandle hFile,
            StringBuilder lpszFilePath,
            uint cchFilePath,
            uint dwFlags);

        public static string GetFinalPath(string path)
        {
            using (var stream = new FileStream(
                path,
                FileMode.Open,
                FileAccess.Read,
                FileShare.ReadWrite | FileShare.Delete))
            {
                var buffer = new StringBuilder(32768);
                uint length = GetFinalPathNameByHandleW(
                    stream.SafeFileHandle,
                    buffer,
                    (uint)buffer.Capacity,
                    0);

                if (length == 0)
                {
                    throw new Win32Exception(Marshal.GetLastWin32Error());
                }

                if (length >= buffer.Capacity)
                {
                    buffer = new StringBuilder((int)length + 1);
                    length = GetFinalPathNameByHandleW(
                        stream.SafeFileHandle,
                        buffer,
                        (uint)buffer.Capacity,
                        0);
                    if (length == 0 || length >= buffer.Capacity)
                    {
                        throw new Win32Exception(Marshal.GetLastWin32Error());
                    }
                }

                return buffer.ToString();
            }
        }
    }
}
'@
}

$finalPath = try {
    $providerPath = (Resolve-Path -LiteralPath $DatabasePath).ProviderPath
    [OperatorStationSpike.NativePath]::GetFinalPath($providerPath)
}
catch {
    throw 'The existing database path could not be canonicalized.'
}

if ($finalPath.StartsWith('\\?\', [System.StringComparison]::Ordinal)) {
    $finalPath = $finalPath.Substring(4)
}

$normalizedPath = $finalPath.Replace('\', '/').ToLowerInvariant()
$utf8 = [System.Text.UTF8Encoding]::new($false)
$sha256 = [System.Security.Cryptography.SHA256]::Create()
try {
    $hash = $sha256.ComputeHash($utf8.GetBytes($normalizedPath))
}
finally {
    $sha256.Dispose()
}

$hex = ([System.BitConverter]::ToString($hash)).Replace('-', '').ToLowerInvariant()
$fingerprint = "sha256:$hex"

if ($IncludeCanonicalPath) {
    [pscustomobject]@{
        CanonicalPath = $finalPath
        Fingerprint   = $fingerprint
    }
}
else {
    $fingerprint
}
