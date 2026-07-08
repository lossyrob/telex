# Dot-source to set Postgres connection env for the spike.
# Caches the Entra token in TEMP (valid ~1h) so we don't pay the az CLI
# token-fetch cost on every call — and reports whether it hit cache and, if not,
# how long the fetch took (a real source of per-call lag).
$tokenFile = Join-Path $env:TEMP 'telex_pg_token.txt'
$useCache = $false
if (Test-Path $tokenFile) {
    $age = (Get-Date) - (Get-Item $tokenFile).LastWriteTime
    if ($age.TotalMinutes -lt 10) { $useCache = $true }
}
if ($useCache) {
    $env:TELEX_PG_PASSWORD = (Get-Content $tokenFile -Raw).Trim()
    $ageMin = [int]((Get-Date) - (Get-Item $tokenFile).LastWriteTime).TotalMinutes
    Write-Host "[env] token: cached (age ${ageMin}m)"
} else {
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $tok = az account get-access-token --resource https://ossrdbms-aad.database.windows.net --query accessToken --output tsv
    $sw.Stop()
    if (-not $tok -or $tok.Length -lt 100) { Write-Error "Failed to acquire Entra token. Run: az login"; return }
    $tok | Set-Content $tokenFile -NoNewline
    $env:TELEX_PG_PASSWORD = $tok
    Write-Host "[env] token: fetched in $($sw.ElapsedMilliseconds)ms"
}
$env:TELEX_PG_HOST = "your-server.postgres.database.azure.com"
$env:TELEX_PG_USER = "you@example.com"
$env:TELEX_PG_DB   = "postgres"
