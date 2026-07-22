$tool = Join-Path $PSScriptRoot "canonicalize.ps1"
$testRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("telex-canonicalize-" + [guid]::NewGuid())
$utf8Bom = [System.Text.UTF8Encoding]::new($true)
$utf8NoBom = [System.Text.UTF8Encoding]::new($false)

try {
    New-Item -ItemType Directory -Path $testRoot | Out-Null

    $cases = @(
        @{ name = "bom-crlf"; text = "alpha`r`nbeta`r`n`r`n"; encoding = $utf8Bom },
        @{ name = "lone-cr"; text = "alpha`rbeta"; encoding = $utf8NoBom },
        @{ name = "no-newline"; text = "alpha"; encoding = $utf8NoBom }
    )

    foreach ($case in $cases) {
        $path = Join-Path $testRoot ($case.name + ".txt")
        [System.IO.File]::WriteAllText($path, $case.text, $case.encoding)
        & $tool -Write -Path $path | Out-Null

        $actual = [System.IO.File]::ReadAllBytes($path)
        $expectedText = $case.text.Replace("`r`n", "`n").Replace("`r", "`n")
        $expectedText = $expectedText.TrimEnd([char[]]@("`r", "`n")) + "`n"
        $expected = $utf8NoBom.GetBytes($expectedText)

        if (-not [System.Linq.Enumerable]::SequenceEqual(
            [byte[]] $actual,
            [byte[]] $expected
        )) {
            throw "Canonicalization failed for $($case.name)"
        }
    }

    Write-Output "canonicalize tests passed"
}
finally {
    if (Test-Path -LiteralPath $testRoot) {
        Remove-Item -LiteralPath $testRoot -Recurse -Force
    }
}
