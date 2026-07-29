# apiVersion: Local argv process contract v1
# capturedAgainst: PowerShell 7.4+
if ($env:TELEX_ALLOWED_SENTINEL -eq 'allowed' -and [string]::IsNullOrEmpty($env:TELEX_BLOCKED_SENTINEL)) {
    [Console]::Out.WriteLine('sanitized detector environment inherited')
    exit 0
}
[Console]::Error.WriteLine('unexpected child environment')
exit 2
