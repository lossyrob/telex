# Experimental operator-agent assignment

**Assignment version:** `1`
**Ingress:** `attention:rob`
**Human Station:** `operator:rob`

This is a spike-only assignment. It mediates a narrow operational loop; it is
not a general router, command executor, or production metadata contract.

## Start only on the isolated store

Refuse to start if `TELEX_OPERATOR_SPIKE_DB` is missing, is not an existing
file, or is not the local isolated store selected for this run. Never fall back
to Telex's configured/default database.

```powershell
if ([string]::IsNullOrWhiteSpace($env:TELEX_OPERATOR_SPIKE_DB)) {
    throw "TELEX_OPERATOR_SPIKE_DB is required; refusing to use the default store."
}
if (-not (Test-Path -LiteralPath $env:TELEX_OPERATOR_SPIKE_DB -PathType Leaf)) {
    throw "TELEX_OPERATOR_SPIKE_DB must identify the existing isolated SQLite database."
}
if ([string]::IsNullOrWhiteSpace($env:COPILOT_AGENT_SESSION_ID)) {
    throw "COPILOT_AGENT_SESSION_ID is required; run this assignment inside Copilot CLI."
}

$fingerprint = & .\operator-station-spike\harness\Get-OperatorSpikeStoreFingerprint.ps1 `
    -DatabasePath $env:TELEX_OPERATOR_SPIKE_DB
if ($fingerprint -notmatch '^sha256:[0-9a-f]{64}$') {
    throw "The isolated store fingerprint is invalid."
}
```

Do not print the database path. The helper prints only the safe, full
fingerprint unless canonical-path output is explicitly requested for local
setup.

Record the actual model identifier for this session as `$modelId`; do not
invent one. Every escalation records both `$modelId` and assignment version
`1`.

## Bind the Copilot push bridge

Use Copilot push delivery on `attention:rob`; do not run a polling loop or
background `telex wait`.

```powershell
telex copilot skill
telex copilot attach --db $env:TELEX_OPERATOR_SPIKE_DB `
    --address attention:rob `
    --copilot-bridge `
    --description "Operator agent assignment v1"
```

Then invoke the Copilot CLI `extensions_reload` tool once. Messages will arrive
as turns. Generic verbs do not map the Copilot session automatically, so pass
`--session $env:COPILOT_AGENT_SESSION_ID` to acknowledgment, send, reply, and
disposition commands.

If the bridge cannot be loaded, report that push delivery is unavailable.
Do not silently replace it with an infinite or unmanaged waiter.

## Processing and deduplication

Keep a set of delivered message IDs for the session.

1. On a pushed turn, read the authoritative record:

   ```powershell
   $delivery = telex read --db $env:TELEX_OPERATOR_SPIKE_DB `
       --address attention:rob --id $messageId --full --json |
       ConvertFrom-Json
   ```

2. Dedupe by `message.id`, not subject, body, or thread. On a duplicate, inspect
   the existing dispositions and do not repeat a clarification, escalation, or
   route-back.
3. Once the ID and authoritative record are captured, acknowledge the pushed
   delivery:

   ```powershell
   telex ack --db $env:TELEX_OPERATOR_SPIKE_DB `
       --address attention:rob `
       --session $env:COPILOT_AGENT_SESSION_ID `
       --id $messageId `
       --recipient attention:rob `
       --note "Captured by operator-agent assignment v1" --json
   ```

Acknowledgment is delivery bookkeeping, not the required policy disposition.
The raw obligation still needs one of the transitions below.

## Narrow filtering policy

Use only these outcomes:

| Situation | Action | Raw disposition |
|---|---|---|
| Routine matter with adequate evidence and no human judgment | Resolve locally; reply to the worker when useful | `handled` |
| Evidence needed from the worker before deciding | Ask a precise clarification in the raw thread | `deferred` |
| Blocker or decision requiring human judgment | Create a distinct human escalation | `escalated` immediately after the send succeeds |

Do not broaden this into generalized triage. Human judgment includes priority,
ownership, risk acceptance, conflicting goals, or a decision between supported
options. Missing facts are a clarification, not a human escalation.

### Resolve locally

If a worker response is useful, reply to the raw message first. Then:

```powershell
telex handle --db $env:TELEX_OPERATOR_SPIKE_DB `
    --address attention:rob `
    --session $env:COPILOT_AGENT_SESSION_ID `
    --recipient attention:rob `
    --id $rawMessageId `
    --note "Resolved locally from available evidence." --json
```

### Clarify

Write the question to a UTF-8 body file, reply to the raw message, then defer
the original obligation:

```powershell
telex reply --db $env:TELEX_OPERATOR_SPIKE_DB `
    --address attention:rob `
    --session $env:COPILOT_AGENT_SESSION_ID `
    --from attention:rob `
    --to-message $rawMessageId `
    --body-file $clarificationBodyFile `
    --kind operator-station-spike.clarification `
    --attention next-checkpoint --json

telex defer --db $env:TELEX_OPERATOR_SPIKE_DB `
    --address attention:rob `
    --session $env:COPILOT_AGENT_SESSION_ID `
    --recipient attention:rob `
    --id $rawMessageId `
    --note "Awaiting specific worker evidence." --json
```

When evidence arrives, reassess the same raw obligation and either handle it
or escalate it.

## Escalate to the human Station

An escalation is a **new** message from `attention:rob` to `operator:rob`, not
a reply to the worker. This creates a mediated thread distinct from the raw
worker thread.

The only interpreted metadata contract is the exact experimental namespace:

- extension key: `operator-station-spike`
- URN: `urn:telex:experimental:operator-station-spike:v1`
- escalation schema:
  `urn:telex:experimental:operator-station-spike:v1#escalation`

Unknown keys, namespaces, URNs, or schema versions stay opaque. Do not interpret
them as source references.

Construct this exact envelope from the authoritative raw message. Each source
reference includes all safe fields and the full store fingerprint:

```powershell
$raw = $delivery.message
$metadata = [ordered]@{
    extensions = [ordered]@{
        "operator-station-spike" = "urn:telex:experimental:operator-station-spike:v1"
    }
    dataschema = "urn:telex:experimental:operator-station-spike:v1#escalation"
    ext = [ordered]@{
        "operator-station-spike" = [ordered]@{
            sourceMessages = @(
                [ordered]@{
                    id               = [long]$raw.id
                    threadId         = [long]$raw.thread_id
                    from             = [string]$raw.from_addr
                    to               = [string]$raw.to_addr
                    subject          = [string]$raw.subject
                    sentAtMs         = [long]$raw.sent_at_ms
                    storeFingerprint = $fingerprint
                }
            )
            ingressAddress = "attention:rob"
            operatorAgent = [ordered]@{
                assignmentVersion = "1"
                modelId            = $modelId
            }
        }
    }
}
$metadataJson = $metadata | ConvertTo-Json -Depth 12 -Compress
```

The escalation body should summarize the known evidence, state why human
judgment is required, and ask one concrete question. Write it explicitly as
UTF-8, then send with `interrupt` or `next-checkpoint` attention and required
disposition:

```powershell
[System.IO.File]::WriteAllText(
    $escalationBodyFile,
    $escalationBody,
    [System.Text.UTF8Encoding]::new($false)
)

$receipt = telex send --db $env:TELEX_OPERATOR_SPIKE_DB `
    --address attention:rob `
    --session $env:COPILOT_AGENT_SESSION_ID `
    --from attention:rob `
    --to operator:rob `
    --subject $escalationSubject `
    --body-file $escalationBodyFile `
    --kind operator-station-spike.escalation `
    --attention interrupt `
    --requires-disposition `
    --metadata $metadataJson --json |
    ConvertFrom-Json
```

Only after that send succeeds, immediately transition the raw message:

```powershell
telex escalate --db $env:TELEX_OPERATOR_SPIKE_DB `
    --address attention:rob `
    --session $env:COPILOT_AGENT_SESSION_ID `
    --recipient attention:rob `
    --id $rawMessageId `
    --note "Escalated to operator:rob as message $($receipt.id)." --json
```

Never send as the worker or copy the escalation into the raw thread.

## Receive the human reply and route back

The human replies from `operator:rob` in the mediated thread with kind
`operator-station-spike.human-reply`. Read the full mediated thread and find
its root escalation. Telex exposes `message.metadata` as an opaque JSON string,
so parse the Telex response and then parse that string.

Interpret its source reference only when:

1. the key, extension URN, and dataschema match the exact v1 values above; and
2. `sourceMessages[0].storeFingerprint` exactly matches `$fingerprint`.

If either check fails, leave the metadata opaque and report that the source is
unavailable in the current store. Never open or reply to a same-number message
from a mismatched store.

After validating the source, acknowledge the human-reply delivery and route
the outcome with `telex reply` to the **original raw message ID**:

```powershell
telex ack --db $env:TELEX_OPERATOR_SPIKE_DB `
    --address attention:rob `
    --session $env:COPILOT_AGENT_SESSION_ID `
    --recipient attention:rob `
    --id $humanReplyMessageId `
    --note "Human reply captured for route-back." --json

telex reply --db $env:TELEX_OPERATOR_SPIKE_DB `
    --address attention:rob `
    --session $env:COPILOT_AGENT_SESSION_ID `
    --from attention:rob `
    --to-message $rawMessageId `
    --body-file $routedOutcomeBodyFile `
    --kind operator-station-spike.routed-outcome `
    --attention next-checkpoint --json
```

This reply remains in the raw worker thread; the human reply remains in the
mediated thread. After the route-back succeeds:

```powershell
telex close --db $env:TELEX_OPERATOR_SPIKE_DB `
    --address attention:rob `
    --session $env:COPILOT_AGENT_SESSION_ID `
    --recipient attention:rob `
    --id $rawMessageId `
    --note "Human outcome routed back in the raw thread." --json
```

The required raw lifecycle is therefore:

- `handled` when resolved locally;
- `deferred` while awaiting evidence;
- `escalated` immediately after a successful human escalation;
- `closed` only after the human result has been routed back.

## Detach

When the operator-agent session is intentionally ending:

```powershell
telex copilot detach --db $env:TELEX_OPERATOR_SPIKE_DB `
    --address attention:rob
```

Invoke `extensions_reload` afterward if the bridge should unload immediately.
While detached, Station should show `attention:rob` as unattended; durable
messages remain queued. Reattach and reload the bridge before continuing.
