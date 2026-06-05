# Attach: start the resident holder for an address. This BLOCKS (it is the
# long-lived answerback drum), so run it as an attached background task.
# Use -Push to enable LISTEN/NOTIFY delivery in addition to the poll backstop.
param(
    [Parameter(Mandatory)][string]$Address,
    [int]$Port = 47700,
    [switch]$Push
)
. "$PSScriptRoot\_env.ps1"
$extra = @()
if ($Push) { $extra += '--push' }
& "$PSScriptRoot\target\debug\holder.exe" --address $Address --port $Port --heartbeat-secs 3 @extra
