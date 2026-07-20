[CmdletBinding()]
param(
    [string] $EvidencePath,

    [switch] $Stress,

    [ValidateRange(1051, 5000)]
    [int] $StressCount = 1055,

    [string] $ModelId = 'scripted-operator-stand-in'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$assignmentVersion = '1'
$ingressAddress = 'attention:rob'
$stationAddress = 'operator:rob'
$workerAddress = 'worker:operator-loop-smoke'
$escalationKind = 'operator-station-spike.escalation'
$humanReplyKind = 'operator-station-spike.human-reply'
$routedOutcomeKind = 'operator-station-spike.routed-outcome'
$experimentalUrn = 'urn:telex:experimental:operator-station-spike:v1'
$helper = Join-Path $PSScriptRoot 'Get-OperatorSpikeStoreFingerprint.ps1'
$script:Telex = (Get-Command telex -ErrorAction Stop).Source
$script:SensitivePaths = @()

function Protect-SensitiveText {
    param([AllowEmptyString()][string] $Text)

    $protected = $Text
    foreach ($path in $script:SensitivePaths) {
        if (-not [string]::IsNullOrWhiteSpace($path)) {
            $protected = [regex]::Replace(
                $protected,
                [regex]::Escape($path),
                '<isolated-db>',
                [System.Text.RegularExpressions.RegexOptions]::IgnoreCase
            )
        }
    }
    return $protected
}

function Invoke-TelexText {
    param(
        [Parameter(Mandatory)]
        [object[]] $Arguments
    )

    $output = @(& $script:Telex @Arguments 2>&1)
    $exitCode = $LASTEXITCODE
    $text = ($output | ForEach-Object { $_.ToString() }) -join [Environment]::NewLine
    if ($exitCode -ne 0) {
        $verb = if ($Arguments.Count -gt 0) { [string]$Arguments[0] } else { 'command' }
        throw "telex '$verb' failed with exit code $exitCode`: $(Protect-SensitiveText $text)"
    }
    return $text
}

function Invoke-TelexJson {
    param(
        [Parameter(Mandatory)]
        [object[]] $Arguments
    )

    $text = Invoke-TelexText -Arguments $Arguments
    try {
        return $text | ConvertFrom-Json
    }
    catch {
        $verb = if ($Arguments.Count -gt 0) { [string]$Arguments[0] } else { 'command' }
        throw "telex '$verb' returned invalid JSON."
    }
}

function Assert-Condition {
    param(
        [Parameter(Mandatory)]
        [bool] $Condition,

        [Parameter(Mandatory)]
        [string] $Message
    )

    if (-not $Condition) {
        throw $Message
    }
}

function Write-Utf8Body {
    param(
        [Parameter(Mandatory)]
        [string] $Path,

        [Parameter(Mandatory)]
        [string] $Content
    )

    [System.IO.File]::WriteAllText(
        $Path,
        $Content,
        [System.Text.UTF8Encoding]::new($false)
    )
}

if ([string]::IsNullOrWhiteSpace($env:TELEX_OPERATOR_SPIKE_DB)) {
    throw 'TELEX_OPERATOR_SPIKE_DB is required. Set it to a fresh, explicit local SQLite path.'
}

$requestedDatabasePath = [System.IO.Path]::GetFullPath($env:TELEX_OPERATOR_SPIKE_DB)
$script:SensitivePaths += $requestedDatabasePath

if (Test-Path -LiteralPath $requestedDatabasePath) {
    throw 'The smoke harness refuses to modify an existing database. Choose a fresh TELEX_OPERATOR_SPIKE_DB path.'
}

if (-not [string]::IsNullOrWhiteSpace($EvidencePath)) {
    $EvidencePath = [System.IO.Path]::GetFullPath($EvidencePath)
    if (Test-Path -LiteralPath $EvidencePath) {
        throw 'The evidence output already exists. Choose a new EvidencePath.'
    }
}

$databaseParent = Split-Path -Parent $requestedDatabasePath
if (-not (Test-Path -LiteralPath $databaseParent -PathType Container)) {
    New-Item -ItemType Directory -Path $databaseParent -Force | Out-Null
}

# A store-scoped status call creates the selected SQLite schema without attaching
# an address or sending a message.
Invoke-TelexJson -Arguments @(
    'status', '--db', $env:TELEX_OPERATOR_SPIKE_DB, '--json'
) | Out-Null

Assert-Condition -Condition (Test-Path -LiteralPath $requestedDatabasePath -PathType Leaf) `
    -Message 'Telex did not initialize the explicit SQLite database.'

$store = & $helper -DatabasePath $requestedDatabasePath -IncludeCanonicalPath
$env:TELEX_OPERATOR_SPIKE_DB = $store.CanonicalPath
$script:SensitivePaths += $store.CanonicalPath
$storeFingerprint = $store.Fingerprint

$runId = [guid]::NewGuid().ToString('N')
$workerSession = "operator-loop-smoke-worker-$runId"
$operatorSession = "operator-loop-smoke-operator-$runId"
$stationSession = "operator-loop-smoke-station-$runId"
$bodyRoot = Join-Path $databaseParent ".operator-loop-smoke-$runId"
$attached = [System.Collections.Generic.List[object]]::new()
$stressEvidence = $null
$completed = $false

New-Item -ItemType Directory -Path $bodyRoot | Out-Null

try {
    foreach ($binding in @(
        @{ Address = $workerAddress; Session = $workerSession; Description = 'Operator-loop smoke worker' }
        @{ Address = $ingressAddress; Session = $operatorSession; Description = 'Scripted operator-agent stand-in' }
        @{ Address = $stationAddress; Session = $stationSession; Description = 'Operator Station smoke stand-in' }
    )) {
        Invoke-TelexJson -Arguments @(
            'attach',
            '--db', $env:TELEX_OPERATOR_SPIKE_DB,
            '--address', $binding.Address,
            '--session', $binding.Session,
            '--description', $binding.Description,
            '--json'
        ) | Out-Null
        $attached.Add($binding)
    }

    $workerBody = Join-Path $bodyRoot 'worker-request.txt'
    Write-Utf8Body -Path $workerBody -Content @'
Please choose the safe rollout window. Evidence supports either Tuesday or Thursday; human scheduling judgment is required.
'@

    $workerReceipt = Invoke-TelexJson -Arguments @(
        'send',
        '--db', $env:TELEX_OPERATOR_SPIKE_DB,
        '--from', $workerAddress,
        '--to', $ingressAddress,
        '--session', $workerSession,
        '--subject', 'Rollout-window decision required',
        '--body-file', $workerBody,
        '--kind', 'decision-request',
        '--attention', 'next-checkpoint',
        '--requires-disposition',
        '--json'
    )

    $rawId = [long]$workerReceipt.id
    $raw = (Invoke-TelexJson -Arguments @(
        'read',
        '--db', $env:TELEX_OPERATOR_SPIKE_DB,
        '--address', $ingressAddress,
        '--id', $rawId,
        '--full',
        '--json'
    )).message

    $metadata = [ordered]@{
        extensions = [ordered]@{
            'operator-station-spike' = $experimentalUrn
        }
        dataschema = "$experimentalUrn#escalation"
        ext = [ordered]@{
            'operator-station-spike' = [ordered]@{
                sourceMessages = @(
                    [ordered]@{
                        id               = [long]$raw.id
                        threadId         = [long]$raw.thread_id
                        from             = [string]$raw.from_addr
                        to               = [string]$raw.to_addr
                        subject          = [string]$raw.subject
                        sentAtMs         = [long]$raw.sent_at_ms
                        storeFingerprint = $storeFingerprint
                    }
                )
                ingressAddress = $ingressAddress
                operatorAgent = [ordered]@{
                    assignmentVersion = $assignmentVersion
                    modelId            = $ModelId
                }
            }
        }
    }
    $metadataJson = $metadata | ConvertTo-Json -Depth 12 -Compress

    $escalationBody = Join-Path $bodyRoot 'escalation.txt'
    Write-Utf8Body -Path $escalationBody -Content @'
The worker supplied two viable rollout windows. Scheduling ownership and impact tradeoffs require human judgment. Please select Tuesday or Thursday and add any constraint the worker should follow.
'@

    $escalationReceipt = Invoke-TelexJson -Arguments @(
        'send',
        '--db', $env:TELEX_OPERATOR_SPIKE_DB,
        '--from', $ingressAddress,
        '--to', $stationAddress,
        '--session', $operatorSession,
        '--subject', 'Decision: rollout window',
        '--body-file', $escalationBody,
        '--kind', $escalationKind,
        '--attention', 'interrupt',
        '--requires-disposition',
        '--metadata', $metadataJson,
        '--json'
    )

    $escalationId = [long]$escalationReceipt.id
    $mediatedThreadId = [long]$escalationReceipt.thread_id

    Invoke-TelexJson -Arguments @(
        'escalate',
        '--db', $env:TELEX_OPERATOR_SPIKE_DB,
        '--address', $ingressAddress,
        '--session', $operatorSession,
        '--recipient', $ingressAddress,
        '--id', $rawId,
        '--note', "Escalated to $stationAddress as message $escalationId.",
        '--json'
    ) | Out-Null

    $rawAfterEscalation = Invoke-TelexJson -Arguments @(
        'read',
        '--db', $env:TELEX_OPERATOR_SPIKE_DB,
        '--address', $ingressAddress,
        '--id', $rawId,
        '--full',
        '--json'
    )
    $rawStatesAfterEscalation = @($rawAfterEscalation.dispositions | ForEach-Object { $_.state })
    Assert-Condition -Condition ($rawStatesAfterEscalation -contains 'escalated') `
        -Message 'The raw message did not record an escalated disposition.'

    $storedEscalation = Invoke-TelexJson -Arguments @(
        'read',
        '--db', $env:TELEX_OPERATOR_SPIKE_DB,
        '--address', $stationAddress,
        '--id', $escalationId,
        '--full',
        '--json'
    )
    $storedMetadata = $storedEscalation.message.metadata | ConvertFrom-Json
    $storedExtension = $storedMetadata.ext.'operator-station-spike'
    $storedSource = @($storedExtension.sourceMessages)[0]

    Assert-Condition -Condition (
        $storedMetadata.extensions.'operator-station-spike' -eq $experimentalUrn -and
        $storedMetadata.dataschema -eq "$experimentalUrn#escalation" -and
        $storedSource.id -eq $rawId -and
        $storedSource.threadId -eq [long]$raw.thread_id -and
        $storedSource.from -eq $raw.from_addr -and
        $storedSource.to -eq $raw.to_addr -and
        $storedSource.subject -eq $raw.subject -and
        $storedSource.sentAtMs -eq [long]$raw.sent_at_ms -and
        $storedSource.storeFingerprint -eq $storeFingerprint -and
        $storedExtension.operatorAgent.assignmentVersion -eq $assignmentVersion -and
        $storedExtension.operatorAgent.modelId -eq $ModelId
    ) -Message 'The escalation metadata did not round-trip with the required experimental source reference.'

    $humanBody = Join-Path $bodyRoot 'human-reply.txt'
    Write-Utf8Body -Path $humanBody -Content @'
Use Thursday. Avoid the Tuesday dependency freeze and notify the release owner before scheduling.
'@

    $humanReplyReceipt = Invoke-TelexJson -Arguments @(
        'reply',
        '--db', $env:TELEX_OPERATOR_SPIKE_DB,
        '--address', $stationAddress,
        '--from', $stationAddress,
        '--session', $stationSession,
        '--to-message', $escalationId,
        '--body-file', $humanBody,
        '--kind', $humanReplyKind,
        '--attention', 'next-checkpoint',
        '--requires-disposition',
        '--json'
    )

    $humanReplyId = [long]$humanReplyReceipt.id
    $storedHumanReply = Invoke-TelexJson -Arguments @(
        'read',
        '--db', $env:TELEX_OPERATOR_SPIKE_DB,
        '--address', $stationAddress,
        '--id', $humanReplyId,
        '--full',
        '--json'
    )
    Assert-Condition -Condition (
        [long]$humanReplyReceipt.thread_id -eq $mediatedThreadId -and
        [long]$humanReplyReceipt.thread_id -ne [long]$raw.thread_id -and
        $storedHumanReply.message.requires_disposition
    ) -Message 'The human reply did not remain in the distinct mediated thread.'

    Invoke-TelexJson -Arguments @(
        'handle',
        '--db', $env:TELEX_OPERATOR_SPIKE_DB,
        '--address', $stationAddress,
        '--session', $stationSession,
        '--recipient', $stationAddress,
        '--id', $escalationId,
        '--note', "Human response sent as message $humanReplyId.",
        '--json'
    ) | Out-Null

    # Simulate the operator session ending after receiving the human obligation
    # but before route-back. Because the message is disposition-required and has
    # not been acked, reattachment must recover it as actionable.
    $operatorBinding = @($attached | Where-Object { $_.Address -eq $ingressAddress })[0]
    Invoke-TelexJson -Arguments @(
        'detach',
        '--db', $env:TELEX_OPERATOR_SPIKE_DB,
        '--address', $ingressAddress,
        '--session', $operatorSession,
        '--json'
    ) | Out-Null
    $attached.Remove($operatorBinding) | Out-Null
    Invoke-TelexJson -Arguments @(
        'attach',
        '--db', $env:TELEX_OPERATOR_SPIKE_DB,
        '--address', $ingressAddress,
        '--session', $operatorSession,
        '--description', 'Scripted operator-agent stand-in after return-path restart',
        '--json'
    ) | Out-Null
    $attached.Add($operatorBinding)

    $recoveredHumanReply = Invoke-TelexJson -Arguments @(
        'inbox',
        '--db', $env:TELEX_OPERATOR_SPIKE_DB,
        '--address', $ingressAddress,
        '--limit', 20,
        '--json'
    )
    Assert-Condition -Condition (
        @(
            $recoveredHumanReply.items |
                Where-Object { [long]$_.id -eq $humanReplyId -and $_.actionable }
        ).Count -eq 1
    ) -Message 'The unacked human reply was not recoverable after operator reattachment.'

    $routedBody = Join-Path $bodyRoot 'routed-outcome.txt'
    Write-Utf8Body -Path $routedBody -Content @'
Human decision: use Thursday. Avoid the Tuesday dependency freeze and notify the release owner before scheduling.
'@

    $routedReceipt = Invoke-TelexJson -Arguments @(
        'reply',
        '--db', $env:TELEX_OPERATOR_SPIKE_DB,
        '--address', $ingressAddress,
        '--from', $ingressAddress,
        '--session', $operatorSession,
        '--to-message', $rawId,
        '--body-file', $routedBody,
        '--kind', $routedOutcomeKind,
        '--attention', 'next-checkpoint',
        '--json'
    )

    $routedId = [long]$routedReceipt.id
    Assert-Condition -Condition (
        [long]$routedReceipt.thread_id -eq [long]$raw.thread_id -and
        [long]$routedReceipt.thread_id -ne $mediatedThreadId -and
        $routedReceipt.to -eq $workerAddress
    ) -Message 'The routed outcome did not return to the worker in the original raw thread.'

    Invoke-TelexJson -Arguments @(
        'ack',
        '--db', $env:TELEX_OPERATOR_SPIKE_DB,
        '--address', $ingressAddress,
        '--session', $operatorSession,
        '--recipient', $ingressAddress,
        '--id', $humanReplyId,
        '--note', 'Route-back succeeded before human-reply acknowledgment.',
        '--json'
    ) | Out-Null

    Invoke-TelexJson -Arguments @(
        'close',
        '--db', $env:TELEX_OPERATOR_SPIKE_DB,
        '--address', $ingressAddress,
        '--session', $operatorSession,
        '--recipient', $ingressAddress,
        '--id', $rawId,
        '--note', "Human outcome routed to the worker as message $routedId.",
        '--json'
    ) | Out-Null

    $rawFinal = Invoke-TelexJson -Arguments @(
        'read',
        '--db', $env:TELEX_OPERATOR_SPIKE_DB,
        '--address', $ingressAddress,
        '--id', $rawId,
        '--full',
        '--json'
    )
    $rawStates = @($rawFinal.dispositions | ForEach-Object { $_.state })
    $rawThreadIds = @($rawFinal.thread | ForEach-Object { [long]$_.message.thread_id } | Select-Object -Unique)

    Assert-Condition -Condition (
        $rawStates -contains 'escalated' -and
        $rawStates[-1] -eq 'closed' -and
        $rawThreadIds.Count -eq 1 -and
        $rawThreadIds[0] -eq [long]$raw.thread_id
    ) -Message 'The raw thread did not preserve escalated -> closed disposition history.'

    $mediatedFinal = Invoke-TelexJson -Arguments @(
        'read',
        '--db', $env:TELEX_OPERATOR_SPIKE_DB,
        '--address', $stationAddress,
        '--id', $escalationId,
        '--full',
        '--json'
    )
    $mediatedThreadIds = @(
        $mediatedFinal.thread |
            ForEach-Object { [long]$_.message.thread_id } |
            Select-Object -Unique
    )
    Assert-Condition -Condition (
        $mediatedThreadIds.Count -eq 1 -and
        $mediatedThreadIds[0] -eq $mediatedThreadId
    ) -Message 'The mediated thread was not internally consistent.'

    if ($Stress) {
        $stressObligation = Invoke-TelexJson -Arguments @(
            'send',
            '--db', $env:TELEX_OPERATOR_SPIKE_DB,
            '--from', $ingressAddress,
            '--to', $stationAddress,
            '--session', $operatorSession,
            '--subject', 'Stress sentinel: keep unresolved across restart',
            '--body', 'This bounded stress sentinel must remain unresolved.',
            '--kind', $escalationKind,
            '--attention', 'next-checkpoint',
            '--requires-disposition',
            '--metadata', $metadataJson,
            '--json'
        )
        $stressObligationId = [long]$stressObligation.id

        for ($index = 1; $index -le $StressCount; $index++) {
            Invoke-TelexJson -Arguments @(
                'send',
                '--db', $env:TELEX_OPERATOR_SPIKE_DB,
                '--from', $ingressAddress,
                '--to', $stationAddress,
                '--session', $operatorSession,
                '--subject', "Stress FYI $index of $StressCount",
                '--body', 'Resolved/FYI restart-load message.',
                '--kind', 'operator-station-spike.stress-fyi',
                '--attention', 'fyi',
                '--json'
            ) | Out-Null
        }

        $recent = Invoke-TelexJson -Arguments @(
            'inbox',
            '--db', $env:TELEX_OPERATOR_SPIKE_DB,
            '--address', $stationAddress,
            '--all',
            '--limit', 200,
            '--json'
        )
        $recentIds = @($recent.items | ForEach-Object { [long]$_.id })
        Assert-Condition -Condition ($recentIds -notcontains $stressObligationId) `
            -Message 'The stress sentinel unexpectedly remained in the 200-message recent tail.'

        Invoke-TelexJson -Arguments @(
            'detach',
            '--db', $env:TELEX_OPERATOR_SPIKE_DB,
            '--address', $stationAddress,
            '--session', $stationSession,
            '--json'
        ) | Out-Null
        $stationBinding = @($attached | Where-Object { $_.Address -eq $stationAddress })[0]
        $attached.Remove($stationBinding) | Out-Null

        Invoke-TelexJson -Arguments @(
            'attach',
            '--db', $env:TELEX_OPERATOR_SPIKE_DB,
            '--address', $stationAddress,
            '--session', $stationSession,
            '--description', 'Operator Station smoke stand-in after restart',
            '--json'
        ) | Out-Null
        $attached.Add($stationBinding)

        $exportText = Invoke-TelexText -Arguments @(
            'export',
            '--db', $env:TELEX_OPERATOR_SPIKE_DB,
            '--address', $stationAddress,
            '--since', 0,
            '--json'
        )
        $exportRows = @(
            $exportText -split '\r?\n' |
                Where-Object { -not [string]::IsNullOrWhiteSpace($_) } |
                ForEach-Object { $_ | ConvertFrom-Json }
        )
        $exportedSentinel = @(
            $exportRows |
                Where-Object { [long]$_.message.id -eq $stressObligationId }
        )
        Assert-Condition -Condition ($exportedSentinel.Count -eq 1) `
            -Message 'The export-backed restart projection could not recover the old unresolved sentinel.'
        $sentinelStates = @($exportedSentinel[0].dispositions | ForEach-Object { $_.state })
        Assert-Condition -Condition (
            $sentinelStates -notcontains 'handled' -and
            $sentinelStates -notcontains 'closed' -and
            $sentinelStates -notcontains 'rejected'
        ) -Message 'The stress sentinel was unexpectedly terminally dispositioned.'

        $stressEvidence = [ordered]@{
            enabled                     = $true
            newerMessageCount           = $StressCount
            exceedsOneThousand          = ($StressCount -gt 1000)
            sentinelOutsideRecent200    = $true
            sentinelRecoveredByExport   = $true
            stationDetachAttachSimulated = $true
        }
    }
    else {
        $stressEvidence = [ordered]@{
            enabled = $false
            note    = 'Run with -Stress to add 1,055 bounded FYI messages and exercise export-backed restart recovery.'
        }
    }

    $version = (Invoke-TelexText -Arguments @(
        '--db', $env:TELEX_OPERATOR_SPIKE_DB, '--version'
    )).Trim()
    $evidence = [ordered]@{
        schema                 = 'operator-station-spike.smoke-evidence.v1'
        passed                 = $true
        assignmentVersion      = $assignmentVersion
        modelId                = $ModelId
        telexVersion           = $version
        storeFingerprint       = $storeFingerprint
        addresses              = [ordered]@{
            worker  = $workerAddress
            ingress = $ingressAddress
            station = $stationAddress
        }
        raw                    = [ordered]@{
            messageId    = $rawId
            threadId     = [long]$raw.thread_id
            dispositions = $rawStates
            routedReplyId = $routedId
        }
        mediated               = [ordered]@{
            escalationId = $escalationId
            threadId     = $mediatedThreadId
            humanReplyId = $humanReplyId
            kind         = $escalationKind
            attention    = 'interrupt'
        }
        assertions             = [ordered]@{
            metadataRoundTrip        = $true
            fullFingerprintPresent   = ($storeFingerprint -match '^sha256:[0-9a-f]{64}$')
            rawEscalatedBeforeClose  = $true
            humanReplyMediatedThread = $true
            routeBackRawThread       = $true
            rawClosedAfterRouteBack  = $true
            humanReplyRecoveredBeforeAck = $true
        }
        stress                  = $stressEvidence
    }
    $evidenceJson = $evidence | ConvertTo-Json -Depth 10

    if (-not [string]::IsNullOrWhiteSpace($EvidencePath)) {
        $evidenceParent = Split-Path -Parent $EvidencePath
        if (-not (Test-Path -LiteralPath $evidenceParent -PathType Container)) {
            New-Item -ItemType Directory -Path $evidenceParent -Force | Out-Null
        }
        [System.IO.File]::WriteAllText(
            $EvidencePath,
            $evidenceJson,
            [System.Text.UTF8Encoding]::new($false)
        )
    }

    $completed = $true
    $evidenceJson
}
finally {
    for ($index = $attached.Count - 1; $index -ge 0; $index--) {
        $binding = $attached[$index]
        try {
            Invoke-TelexJson -Arguments @(
                'detach',
                '--db', $env:TELEX_OPERATOR_SPIKE_DB,
                '--address', $binding.Address,
                '--session', $binding.Session,
                '--json'
            ) | Out-Null
        }
        catch {
            if ($completed) {
                Write-Warning 'A smoke identity could not be detached; inspect station status for the isolated store.'
            }
        }
    }
    Remove-Item -LiteralPath $bodyRoot -Recurse -Force -ErrorAction SilentlyContinue
}
