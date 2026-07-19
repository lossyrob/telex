$ErrorActionPreference = 'Continue'

$neutralDecision = '{}'
$blockDecision = '{"decision":"block","reason":"Telex plugin/binary version skew: the telex binary resolved from PATH could not run `copilot drain`. Run `telex copilot drain --help` and `telex --json version`, then use `Get-Command telex` on Windows or `command -v telex` on POSIX to identify the PATH winner. Reinstall a matched plugin/binary release through the versioned installer, ensure its bin directory precedes stale shims such as cargo-installed copies, and restart Copilot. If intentionally rolling back the binary, roll back the plugin to the same release first. `TELEX_COPILOT_DRAIN=off` is only a temporary escape hatch."}'

$drainSetting = $env:TELEX_COPILOT_DRAIN
if ($null -ne $drainSetting) {
    $drainSetting = $drainSetting.Trim().ToLowerInvariant()
}
if ($drainSetting -in @('off', '0', 'false')) {
    [Console]::Out.WriteLine($neutralDecision)
    exit 0
}

try {
    $payload = [Console]::In.ReadToEnd()
    $payload | & telex --json copilot drain > $null 2> $null
    $exitCode = $LASTEXITCODE
} catch {
    $exitCode = 1
}

if ($exitCode -eq 0) {
    [Console]::Out.WriteLine($neutralDecision)
} else {
    [Console]::Out.WriteLine($blockDecision)
}
exit 0
