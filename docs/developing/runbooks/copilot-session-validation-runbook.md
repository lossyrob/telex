# Copilot session validation runbook

Use this runbook to prove that real Copilot CLI sessions can coordinate through Telex stations. The proof is intentionally session-level: the worker Copilot sessions must run Telex CLI commands themselves, exchange messages through the selected backend, and report results back through Telex. Do not replace the worker sessions with scripts that send and receive on their behalf.

## What this validates

- A coordinator station and two worker stations can coexist on one Telex backend.
- Visible Copilot CLI sessions can join stations, wait, send, reply, acknowledge, and report results using direct `telex` commands.
- The backend stores delivery state durably enough for the coordinator to observe READY and DONE messages.
- The daemon status surface reports the expected store, station, lease, waiter, retention, and recent-error state.

This runbook can validate any backend. The core protocol is backend-agnostic; only the setup section changes.

## Backend setup variants

### Local SQLite smoke proof

Use this for the fastest visible-session smoke test.

1. Build Telex normally:
   ```powershell
   cargo build --quiet
   ```
2. Create an isolated Telex root:
   ```powershell
   $root = Join-Path $env:TEMP ("telex-session-proof-" + [guid]::NewGuid().ToString("N"))
   $env:TELEX_HOME = Join-Path $root "home"
   $env:TELEX_RUN_DIR = Join-Path $root "run"
   $env:LOCALAPPDATA = Join-Path $root "state"
   $env:TELEX_DB = Join-Path $root "local.db"
   New-Item -ItemType Directory -Force -Path $env:TELEX_HOME,$env:TELEX_RUN_DIR,$env:LOCALAPPDATA | Out-Null
   ```
3. Use `target\debug\telex.exe` for all coordinator and worker commands.

### Docker Postgres proof

Use this for repeatable Postgres semantics without managed-service dependencies.

1. Start Postgres:
   ```powershell
   docker run -d --rm --name telex-session-proof-pg -e POSTGRES_PASSWORD=postgres -p 127.0.0.1::5432 postgres:16-alpine
   ```
2. Resolve the mapped port:
   ```powershell
   $port = (docker port telex-session-proof-pg 5432/tcp) -split ":" | Select-Object -Last 1
   ```
3. Build Telex with Postgres support:
   ```powershell
   cargo build --all-features --quiet
   ```
4. Add an isolated backend profile:
   ```powershell
   $schema = "telex_session_proof_" + [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
   target\debug\telex.exe --json backend add docker-pg `
     --postgres "postgres://postgres:postgres@127.0.0.1:$port/postgres" `
     --schema $schema `
     --default
   ```

### Azure Postgres with Entra proof

Use this for managed Azure PostgreSQL Flexible Server and Microsoft Entra authentication.

1. Confirm Azure CLI is logged in as a principal that can authenticate to the server.
2. Build Telex with Entra support:
   ```powershell
   cargo build --features entra --quiet
   ```
3. Create an isolated backend profile:
   ```powershell
   $schema = "telex_session_proof_" + [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
   target\debug\telex.exe --json backend add azure-entra-proof `
     --postgres "postgres://<user-upn>@<server>.postgres.database.azure.com/postgres?sslmode=require" `
     --schema $schema `
     --entra `
     --entra-cred cli `
     --default
   ```
4. Clean up the schema after the proof. With an Entra access token:
   ```powershell
   $token = az account get-access-token --resource-type oss-rdbms --query accessToken -o tsv
   ```
   Use `psql` or a short client program to run:
   ```sql
   DROP SCHEMA IF EXISTS <schema> CASCADE;
   ```

## Shared coordinator setup

Create unique addresses and sessions:

```powershell
$nonce = [guid]::NewGuid().ToString("N").Substring(0, 12)
$coordSession = "proof-coord-$nonce"
$receiverSession = "proof-receiver-$nonce"
$senderSession = "proof-sender-$nonce"
$coordAddr = "addr:proof-coord-$nonce"
$receiverAddr = "addr:proof-receiver-$nonce"
$senderAddr = "addr:proof-sender-$nonce"
$payload = "SESSION_PROOF_PAYLOAD_$nonce"
```

Join the coordinator station:

```powershell
target\debug\telex.exe --json --address $coordAddr attach `
  --session $coordSession `
  --description "Coordinator station for Copilot session validation"
```

## Worker session launch

Launch two visible Copilot terminals. The prompts may include environment variables and command examples, but the sessions must run direct Telex commands themselves. Do not give the workers a role script that performs the proof on their behalf.

Recommended launch flags:

```powershell
--allow-all --no-ask-user --autopilot
```

### Receiver prompt requirements

Tell the receiver session:

1. Set the shared Telex environment for the selected backend.
2. Join its receiver station:
   ```powershell
   target\debug\telex.exe --json --address <receiver> attach --session <receiver-session> --description "visible receiver station"
   ```
3. Send READY to the coordinator:
   ```powershell
   target\debug\telex.exe --json --address <receiver> send --session <receiver-session> --from <receiver> --to <coord> --body "READY receiver <receiver>"
   ```
4. Wait for the sender payload:
   ```powershell
   target\debug\telex.exe --json --address <receiver> wait --session <receiver-session> --timeout-ms 300000 --out-dir <receiver-out-dir>
   ```
5. Verify the delivered body equals the expected payload.
6. Ack the delivered message.
7. Reply to the sender message with body:
   ```text
   RECEIVER_REPLY <payload>
   ```
8. Send DONE to the coordinator with a JSON summary containing address, received message id, ack outcome, reply id, and daemon recent errors.

### Sender prompt requirements

Tell the sender session:

1. Set the shared Telex environment for the selected backend.
2. Join its sender station.
3. Send READY to the coordinator:
   ```powershell
   target\debug\telex.exe --json --address <sender> send --session <sender-session> --from <sender> --to <coord> --body "READY sender <sender>"
   ```
4. Wait for a GO message from the coordinator and ack it.
5. Send the expected payload to the receiver with `--attention interrupt`.
6. Wait for the receiver reply and verify it equals `RECEIVER_REPLY <payload>`.
7. Ack the reply.
8. Send DONE to the coordinator with a JSON summary containing address, sent id, reply id, ack outcomes, and daemon recent errors.

## Coordinator protocol

The coordinator session does not send or receive on behalf of the workers. It only drives the protocol boundary and verifies messages that workers send through Telex.

1. Wait for two READY messages at the coordinator address and ack both.
2. Send GO to the sender:
   ```powershell
   target\debug\telex.exe --json --address <coord> send --session <coord-session> --from <coord> --to <sender> --attention interrupt --subject GO --body "GO <payload>"
   ```
3. Wait for two DONE messages at the coordinator address and ack both.
4. Capture daemon status:
   ```powershell
   target\debug\telex.exe --json daemon status
   ```
5. Confirm:
   - READY receiver was received.
   - READY sender was received.
   - DONE receiver reports payload verification and an ack outcome.
   - DONE sender reports reply verification and ack outcomes.
   - daemon status has the expected backend store.
   - daemon status has no unexpected `recent_errors`.
6. Stop the daemon:
   ```powershell
   target\debug\telex.exe --json daemon stop --drain
   ```

## Passing evidence

A passing run should produce at least these artifacts:

- coordinator READY message transcript
- coordinator DONE message transcript
- daemon status before drain
- daemon status after drain
- backend setup details: backend kind, profile name, schema/path, binary path
- cleanup result for temporary backend resources

For Postgres, retain the proof that the temporary schema was dropped.

## Harness guidance

A helper may prepare environment variables, backend profiles, unique addresses, role prompts, and cleanup commands. A helper may also watch the coordinator address and validate READY/DONE invariants.

A helper must not send, wait, ack, or reply on behalf of the sender or receiver roles. If it does, the run becomes a CLI subprocess test rather than a real Copilot-session Telex validation.
