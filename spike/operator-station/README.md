# Telex Operator Station Spike

> **Spike-only:** do not depend on this package as a supported Telex client or
> production Station contract.

This standalone Tauri v2 application proves the issue #93 Windows attention
loop without adding desktop dependencies or human-UI behavior to Telex core.
It attends `operator:rob`, supervises the current one-shot `telex wait`/`ack`
contract, shows a feed and mediated thread, renders experimental source
provenance, emits Windows toasts, accepts replies, and supports defer/handle/close
dispositions.

The subprocess courier and full-history export are temporary evidence for the
future Application Client work in issue #12.

## Prerequisites

- Windows, WebView2, Node.js/npm
- Rust stable with MSVC build tools
- Python 3 for the optional Action Center evidence extractor
- `telex` 0.1.0-compatible CLI on `PATH`
- An existing, isolated SQLite store in `TELEX_OPERATOR_SPIKE_DB`

The database path is used only to invoke Telex. The UI and persisted local state
expose a SHA-256 store fingerprint, never the path.

## Run

```powershell
$env:TELEX_OPERATOR_SPIKE_DB = "C:\path\to\existing\operator-loop.db"

Push-Location .\spike\operator-station
npm install
npm test
npm run build
cargo test --manifest-path .\src-tauri\Cargo.toml
npm run tauri dev
Pop-Location
```

Optional configuration:

| Variable | Default |
|---|---|
| `TELEX_OPERATOR_SPIKE_ADDRESS` | `operator:rob` |
| `TELEX_OPERATOR_SPIKE_INGRESS` | `attention:rob` |
| `TELEX_OPERATOR_SPIKE_TELEX` | `telex` |

## How the runtime works

1. Attach the Station address with a persisted session UUID and the Tauri PID as
   an anchor watch.
2. Rebuild startup state from full selected-address JSONL export plus
   `inbox --all --limit 200`.
3. Supervise one 30-second `telex wait` child at a time.
4. On delivery, parse the wait payload, enrich it with `read --full`, ingest and
   emit it, then ack. At-least-once duplicates are deduped by message ID.
5. Surface waiter/ack/re-attach failures and refresh ingress/Station occupancy.

See [WALKTHROUGH.md](WALKTHROUGH.md) for the complete multi-session exercise and
[OPERATOR-AGENT.md](OPERATOR-AGENT.md) for the reusable experimental mediator
assignment.

## Validation

```powershell
npm test
npm run build
cargo fmt --manifest-path .\src-tauri\Cargo.toml --check
cargo test --manifest-path .\src-tauri\Cargo.toml

Get-ChildItem .\harness\*.ps1 | ForEach-Object {
    $tokens = $null
    $errors = $null
    [void][System.Management.Automation.Language.Parser]::ParseFile(
        $_.FullName,
        [ref]$tokens,
        [ref]$errors
    )
    if ($errors.Count) { throw "$($_.Name): $($errors -join '; ')" }
}
```

The Action Center evidence can be regenerated with
`harness\Get-OperatorSpikeToastRecord.ps1`. Spike-local session/high-water files
can be listed, or explicitly removed with confirmation, through
`harness\Reset-OperatorSpikeLocalScope.ps1`.
