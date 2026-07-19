$ErrorActionPreference = 'Continue'

$allowDecision = '{"decision":"allow"}'
$blockDecision = '{"decision":"block","reason":"Telex plugin/binary version skew: the telex binary on PATH lacks or failed `copilot drain`, so this plugin cannot safely complete agentStop. Run `telex copilot drain --help` and `telex --json version`. Upgrade/reinstall through the versioned `telex upgrade --force` path or install a matched plugin/binary pair, then reload/restart Copilot. `TELEX_COPILOT_DRAIN=off` is only a temporary escape hatch."}'

$drainSetting = $env:TELEX_COPILOT_DRAIN
if ($null -ne $drainSetting) {
    $drainSetting = $drainSetting.Trim().ToLowerInvariant()
}
if ($drainSetting -in @('off', '0', 'false')) {
    [Console]::Out.WriteLine($allowDecision)
    exit 0
}

try {
    $payload = [Console]::In.ReadToEnd()
    $telex = @(Get-Command telex -CommandType Application -ErrorAction Stop)[0]
    $payload | & $telex.Source --json copilot drain > $null 2> $null
    $exitCode = $LASTEXITCODE
} catch {
    $exitCode = 1
}

if ($exitCode -eq 0) {
    [Console]::Out.WriteLine($allowDecision)
} else {
    [Console]::Out.WriteLine($blockDecision)
}
exit 0
