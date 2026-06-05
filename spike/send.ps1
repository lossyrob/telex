# Send a message to an address. Prints total send time (token + insert + notify)
# so we can attribute per-send lag.
param(
    [Parameter(Mandatory)][string]$To,
    [Parameter(Mandatory)][string]$Body,
    [string]$Attention = "next-checkpoint"
)
$swTotal = [System.Diagnostics.Stopwatch]::StartNew()
. "$PSScriptRoot\_env.ps1"
& "$PSScriptRoot\target\debug\sender.exe" --address $To --body $Body --attention $Attention
$swTotal.Stop()
Write-Host "[send] total $($swTotal.ElapsedMilliseconds)ms (includes env/token + insert + notify)"
