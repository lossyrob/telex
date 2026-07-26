param(
    [Parameter(Mandatory = $true, Position = 0)]
    [string[]] $Path,

    [switch] $Write
)

$strictUtf8 = [System.Text.UTF8Encoding]::new($false, $true)
$outputUtf8 = [System.Text.UTF8Encoding]::new($false)

foreach ($inputPath in $Path) {
    $resolvedPath = (Resolve-Path -LiteralPath $inputPath).Path
    $originalBytes = [System.IO.File]::ReadAllBytes($resolvedPath)

    $offset = 0
    if ($originalBytes.Length -ge 3 -and
        $originalBytes[0] -eq 0xEF -and
        $originalBytes[1] -eq 0xBB -and
        $originalBytes[2] -eq 0xBF) {
        $offset = 3
    }

    $text = $strictUtf8.GetString(
        $originalBytes,
        $offset,
        $originalBytes.Length - $offset
    )
    $canonicalText = $text.Replace("`r`n", "`n").Replace("`r", "`n")
    $canonicalText = $canonicalText.TrimEnd([char[]]@("`r", "`n")) + "`n"
    $canonicalBytes = $outputUtf8.GetBytes($canonicalText)

    if ($Write) {
        [System.IO.File]::WriteAllBytes($resolvedPath, $canonicalBytes)
    }

    $digest = [System.Security.Cryptography.SHA256]::HashData($canonicalBytes)
    [pscustomobject]@{
        path = $resolvedPath
        bytes = $canonicalBytes.Length
        sha256 = [System.Convert]::ToHexString($digest).ToLowerInvariant()
        changed = -not [System.Linq.Enumerable]::SequenceEqual(
            [byte[]] $originalBytes,
            [byte[]] $canonicalBytes
        )
    }
}
