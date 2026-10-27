[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateSet('Build', 'Run')][string]$Phase,
    [Parameter(Mandatory)][ValidatePattern('^[0-9a-f]{40}$')][string]$ExpectedSourceHead
)
$ErrorActionPreference = 'Stop'

if (-not $IsMacOS) {
    throw 'This runner exercises native macOS management; use the existing workspace checks elsewhere.'
}

$cargoArguments = @('test', '--locked', '-p', 'telex-watcher', '--no-default-features', '--features', 'sqlite')
$nativeCases = @(
    'transport::tests::native_same_user_round_trip_preserves_state_and_registration_bytes'
    'transport::tests::native_alias_and_hard_link_reach_the_same_owner'
    'transport::tests::native_missing_owner_is_typed_bounded_and_never_creates_registry'
    'transport::tests::native_oversized_malformed_versioned_and_slow_clients_are_bounded'
    'transport::tests::native_cleanup_drops_active_clients_and_allows_rebind'
    'transport::tests::native_first_instance_rejects_squatting_or_a_second_server'
    'transport::tests::native_shutdown_reply_is_written_before_server_finishes'
    'transport::unix::tests::stale_socket_is_not_running_and_next_owner_reclaims_it'
)
$cliCases = @(
    'executable_not_running_and_parser_errors_are_stable_json_and_do_not_open_registry'
    'executable_runtime_supports_complete_ordinary_lifecycle_and_safe_defaults'
)

function Invoke-SelectedTests {
    param([string[]]$Target, [string[]]$Cases)
    $arguments = $cargoArguments + $Target + @('--', '--list', '--format', 'terse', '--exact') + $Cases
    $inventory = & cargo @arguments
    if ($LASTEXITCODE -ne 0) {
        throw "Watcher test inventory failed for $Target (exit $LASTEXITCODE)."
    }
    $actual = @($inventory | Where-Object { $_ -cmatch ': test$' } |
        ForEach-Object { $_ -creplace ': test$', '' } | Sort-Object -CaseSensitive)
    $expected = @($Cases | Sort-Object -CaseSensitive)
    if ($actual.Count -ne $expected.Count -or
        (Compare-Object -ReferenceObject $expected -DifferenceObject $actual -CaseSensitive)) {
        throw "Watcher test selection changed: expected [$($expected -join ', ')], got [$($actual -join ', ')]."
    }
    Write-Output "Verified $($actual.Count) exact management cases for $Target."
    $arguments = $cargoArguments + $Target + @('--', '--exact', '--nocapture') + $Cases
    & cargo @arguments
    if ($LASTEXITCODE -ne 0) {
        throw "Watcher management tests failed for $Target (exit $LASTEXITCODE)."
    }
}

Push-Location (Split-Path -Parent $PSScriptRoot)
try {
    $checkout = git rev-parse HEAD
    if ($LASTEXITCODE -ne 0 -or $checkout -cne $ExpectedSourceHead) {
        throw "Source mismatch: expected $ExpectedSourceHead, got $checkout."
    }
    $elapsed = [Diagnostics.Stopwatch]::StartNew()
    $testHome = $null
    try {
        if ($Phase -eq 'Build') {
            $arguments = $cargoArguments + @('--lib', '--test', 'cli_v2', '--no-run')
            & cargo @arguments
            if ($LASTEXITCODE -ne 0) {
                throw "Building Watcher management targets failed (exit $LASTEXITCODE)."
            }
        } else {
            foreach ($name in @('TELEX_HOME', 'TELEX_DB', 'TELEX_INSTALL_ROOT')) {
                $value = [Environment]::GetEnvironmentVariable($name)
                if ([string]::IsNullOrWhiteSpace($value) -or -not [IO.Path]::IsPathFullyQualified($value)) {
                    throw "$name must name an isolated absolute path."
                }
            }
            $originalHome = [IO.Path]::GetFullPath($env:HOME).TrimEnd('/')
            if (-not $env:CARGO_HOME) { $env:CARGO_HOME = Join-Path $originalHome '.cargo' }
            if (-not $env:RUSTUP_HOME) { $env:RUSTUP_HOME = Join-Path $originalHome '.rustup' }
            $candidate = & mktemp -d (Join-Path $originalHome 'w.XXXXXX')
            if ($LASTEXITCODE -ne 0) {
                throw "Creating the private management home failed (exit $LASTEXITCODE)."
            }
            if (-not [IO.Path]::IsPathFullyQualified($candidate) -or
                [IO.Path]::GetDirectoryName($candidate) -cne $originalHome -or
                [IO.Path]::GetFileName($candidate) -cnotmatch '^w\.[A-Za-z0-9]{6}$') {
                throw "Refusing to use or remove an unverified temporary home: $candidate"
            }
            $testHome = $candidate
            if (([int][IO.File]::GetUnixFileMode($testHome) -band 0x3f) -ne 0) {
                throw 'The new management home is not owner-private.'
            }
            $endpoint = Join-Path (Join-Path $testHome '.telex-watcher') ('0' * 64)
            if ([Text.Encoding]::UTF8.GetByteCount($endpoint) -ge 104) {
                throw 'The private management home exceeds the macOS socket-path budget.'
            }
            $env:HOME = $testHome
            $env:USERPROFILE = $testHome
            $env:LOCALAPPDATA = Join-Path $testHome 'local-appdata'
            New-Item -ItemType Directory -Path $env:LOCALAPPDATA | Out-Null
            Invoke-SelectedTests -Target @('--lib') -Cases $nativeCases
            Invoke-SelectedTests -Target @('--test', 'cli_v2') -Cases $cliCases
            $endpointRoot = Join-Path $testHome '.telex-watcher'
            if (Test-Path -LiteralPath $endpointRoot) {
                $leaked = @(Get-ChildItem -LiteralPath $endpointRoot -Force)
                if ($leaked.Count -ne 0) {
                    throw "Management endpoint cleanup left: $($leaked.Name -join ', ')"
                }
            }
        }
    } finally {
        if ($null -ne $testHome) {
            Remove-Item -LiteralPath $testHome -Recurse -Force
        }
        $elapsed.Stop()
        $record = [ordered]@{
            phase = $Phase
            candidate_sha = $ExpectedSourceHead
            provider_sha = $env:GITHUB_SHA
            checkout_sha = $checkout
            elapsed_ms = $elapsed.ElapsedMilliseconds
        }
        Write-Output ("TELEX_WATCHER_MACOS_MANAGEMENT " + ($record | ConvertTo-Json -Compress))
    }
} finally {
    Pop-Location
}
