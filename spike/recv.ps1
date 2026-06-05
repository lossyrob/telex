# Receive: block on your holder until one message arrives, print it as JSON
# (with full timing breakdown), then exit. Run as an attached background task.
# Exit codes: 0 = delivered, 2 = idle timeout, 3 = holder gone, 4 = holder hung.
param(
    [Parameter(Mandatory)][string]$Address,
    [int]$Port = 47700,
    [int]$TimeoutMs = 600000
)
. "$PSScriptRoot\_env.ps1"
& "$PSScriptRoot\target\debug\waiter.exe" --address $Address --port $Port --timeout-ms $TimeoutMs --hang-ms 20000
