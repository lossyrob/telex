<p align="center">
  <img src="assets/telex.png" alt="Telex" width="360">
</p>

A CLI-first **message fabric for AI agent sessions**: durable addresses, typed
messages with answerback liveness, and an auditable record — over SQLite (local,
zero-config) or Postgres (networked, with or without Microsoft Entra auth).

One small binary, `telex`. It even carries its own usage instructions: run
`telex skill` (or `telex skill --raw` for the exact embedded skill file).

The repository also ships a Copilot CLI plugin manifest (`plugin.json` +
`hooks.json`). The plugin maps Copilot session env into generic telex session
inputs, handles non-destructive `sessionEnd`, and guards turn-end re-arming.

## Install

**macOS / Linux:**

```sh
curl -fsSL https://raw.githubusercontent.com/lossyrob/telex/main/install.sh | sh
```

**Windows (PowerShell):**

```powershell
irm https://raw.githubusercontent.com/lossyrob/telex/main/install.ps1 | iex
```

**With Rust (any platform):**

```sh
cargo install --git https://github.com/lossyrob/telex --features entra
```

Or grab a prebuilt binary from [Releases](https://github.com/lossyrob/telex/releases).

## Quickstart

```sh
telex skill                          # print the usage guide (also embedded for agents)
telex send --to me --body "hello"    # zero-config: a local SQLite store, no setup
telex --address me inbox             # read it back
```

That's it — no manual server setup and no config required. The first daemon-backed
verb auto-spawns a per-user local exchange for the default local SQLite store at
`~/.telex/telex.db`.

## For agents

Tell your agent: **"set up telex — run `telex skill`."** The binary self-describes,
so the agent learns to attach to an address, wait for messages, disposition them, and
message peers. To hand an agent a specific assignment in one command:

```sh
telex skill --address workstream:proj/node:issue-215
```

In Copilot CLI, install/use the telex plugin and attach with:

```sh
telex --address workstream:proj/node:issue-215 copilot attach --description "<work>"
telex --address workstream:proj/node:issue-215 wait --session "$COPILOT_AGENT_SESSION_ID" --out-dir <dir>
```

The adapter maps `$COPILOT_AGENT_SESSION_ID` to the generic telex session id and
`$COPILOT_LOADER_PID` to a loader watch-pid. Generic telex commands intentionally
do not read Copilot-specific env variables directly, so follow-up generic commands
must pass `--session "$COPILOT_AGENT_SESSION_ID"` or run in a shell/script that
sets `TELEX_SESSION_ID`.

The bounded-timeout notification heartbeat was evaluated and left out: sync and
agent-read shell completions already carry stdout in context, while the
notification hook payload exposes only notification metadata and not a stable
`--out-dir` path. Classic waiter robustness in this PR comes from the default-on
`agentStop` guard; overnight/AFK deterministic wake belongs to the ACP track.

The plugin shape is validated against GitHub Copilot CLI 1.0.66-1; see
[`docs/design/copilot-plugin-validation.md`](docs/design/copilot-plugin-validation.md)
for the acceptance matrix and live hook smoke evidence.

Release archives include a `copilot-plugin/` directory containing `plugin.json`,
`hooks.json`, and the plugin skill mirror. Install scripts copy it next to the
binary and print the plugin install command.

## Networked backends

Add a Postgres backend once; then select it by name (or make it the default):

```sh
# Azure Postgres with Entra (telex fetches the token itself — uses `az login`,
# or `--entra-cred managed` on a devbox/VM with a managed identity):
telex backend add prod \
  --postgres "host=myserver.postgres.database.azure.com port=5432 user=me@example.com dbname=postgres sslmode=require" \
  --entra --schema telex --default

telex backend list
telex --backend prod send --to node:x --body "hi"
```

Secrets are referenced (`--entra`, `--password-env`, `--password-command`), never
stored in the config file.

## How it works (in one breath)

A durable **address** is the responsibility being served; a per-user local
**exchange** daemon owns SQLite presence, lease heartbeats, delivery buffering,
and local IPC; a session `attach`es once to register an in-memory **station** for
its stable `TELEX_SESSION_ID`; `wait` is a one-shot daemon client that receives
one message and exits; `ack` is the explicit durable consumed mark. If the daemon
restarts, the next verb reconnects, re-registers on `NeedsAttach`, and continues
against the retained delivery buffer.

## Learn more

- **[SKILL.md](SKILL.md)** — how agents use telex (also `telex skill`)
- **[DESIGN.md](docs/design/DESIGN.md)** — the working design
- **[DECISIONS.md](docs/design/DECISIONS.md)** — the decision log
- **[DISPATCH.md](DISPATCH.md)** — forward-looking discovery & dispatch (broadcast, Contract-Net)
- **[EXTENSIONS.md](EXTENSIONS.md)** — proposal: extensions & capability cards (how addresses advertise what they do)
- **[TELEX.md](TELEX.md)** / **[PRODUCT-THESIS.md](PRODUCT-THESIS.md)** — the name, the metaphor, the thesis
- **[spike/](spike/)** — the throwaway validation spike that de-risked the design
