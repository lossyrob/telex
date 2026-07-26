param(
    [string] $RepositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..\..")).Path
)

$canonicalizer = Join-Path $PSScriptRoot "canonicalize.ps1"
$outputPath = Join-Path $RepositoryRoot "docs\design\application-client.bundle.json"
$utf8 = [System.Text.UTF8Encoding]::new($false)

$relativePaths = [string[]] @(
    "docs/design/application-client.md",
    "docs/design/application-client-crosswalk.md",
    "docs/design/DECISIONS.md",
    "docs/design/history/application-client-issue-12-original.md",
    "docs/design/index.md"
)
[Array]::Sort($relativePaths, [StringComparer]::Ordinal)

$files = foreach ($relativePath in $relativePaths) {
    $fullPath = Join-Path $RepositoryRoot ($relativePath.Replace("/", "\"))
    if (-not (Test-Path -LiteralPath $fullPath -PathType Leaf)) {
        throw "Manifest input does not exist: $relativePath"
    }

    $canonical = & $canonicalizer -Path $fullPath
    if ($canonical.changed) {
        throw "Manifest input is not canonical: $relativePath"
    }

    [ordered] @{
        path = $relativePath
        byteLength = [int64] $canonical.bytes
        sha256 = $canonical.sha256
    }
}

$historicalPath = "docs/design/history/application-client-issue-12-original.md"
$historical = $files | Where-Object { $_.path -eq $historicalPath }
if ($null -eq $historical) {
    throw "Historical issue #12 artifact is missing from the manifest file set"
}
if ($historical.byteLength -ne 16175 -or
    $historical.sha256 -ne "c0a5fed4a1fa894ccc6accedee3e0e66af318a10df4ad27e6f2f065f6881c3dc") {
    throw "Historical issue #12 identity does not match the approved pre-convergence body"
}

$manifest = [ordered] @{
    schemaVersion = 1
    checkpointScope = "design-only"
    adrAllocation = [ordered] @{
        number = 49
        title = "One API-neutral Application Client contract governs explicit station capabilities and forbids private fallbacks"
        decisionSlug = "application-client-semantic-boundary"
        requestMessageId = 1812
        dispositionId = 909
        responseMessageId = 1817
        allocationBaseCommit = "7a568c43413fc7aeab6a484b07dce0f0db11d68f"
        allocationHighWaterBefore = 48
        status = "reserved-not-landed"
    }
    sourceProvenance = @(
        [ordered] @{
            domain = "operator-station"
            requirementExportComment = 5042612298
            mergedSourceAddendumComment = 5044388908
            mergeCommit = "0722051760bab569d3f947fd7b29f2dabe13ef77"
            canonicalFinalHead = "2d99e552292a4401d3403540b6d2eaa90272282d"
            sourceCommentDigests = @(
                [ordered] @{
                    commentId = 5042612298
                    role = "requirement-export"
                    sha256 = "adf2f8e439e5c224059ca51142701f604a82203c39c9829d1323a88c58889f7e"
                },
                [ordered] @{
                    commentId = 5044388908
                    role = "merged-source-addendum"
                    sha256 = "702ebdb1ea81329294c35a452670b2313625142bc0281c9100d8ae892890c9ea"
                }
            )
        },
        [ordered] @{
            domain = "watcher"
            requirementExportComment = 5042702401
            mergedSourceAddendumComment = 5043498697
            mergeCommit = "09aa6f45f213b45207adc4cf80676dcce91250da"
            canonicalFinalHead = "e007a8067b3b91b5c57a2a756ce878e310595a05"
            sourceCommentDigests = @(
                [ordered] @{
                    commentId = 5042702401
                    role = "requirement-export"
                    sha256 = "9a037f94af84516592a56dc9c0c701ce0277e305c83dad368227fc25a5b18d9a"
                },
                [ordered] @{
                    commentId = 5043498697
                    role = "merged-source-addendum"
                    sha256 = "fa02b844c62eef17f4c08b9bc1d7d94539e525034e3f0d474b2bfe2d45caed94"
                }
            )
        }
    )
    historicalIssue12 = [ordered] @{
        path = $historicalPath
        byteLength = [int64] $historical.byteLength
        sha256 = $historical.sha256
    }
    files = @($files)
}

$json = $manifest | ConvertTo-Json -Depth 10 -Compress
$bytes = $utf8.GetBytes($json + "`n")
[System.IO.File]::WriteAllBytes($outputPath, $bytes)

$digest = [System.Security.Cryptography.SHA256]::HashData($bytes)
[pscustomobject] @{
    path = $outputPath
    byteLength = $bytes.Length
    sha256 = [Convert]::ToHexString($digest).ToLowerInvariant()
    fileCount = $files.Count
}
