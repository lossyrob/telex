<p align="center">
  <img src="assets/telex.png" alt="Telex" width="360">
</p>

A CLI-first **message fabric for AI agent sessions**: durable addresses, typed
messages with answerback liveness, and an auditable record over SQLite (local,
zero-config) or Postgres (networked, with or without Microsoft Entra auth).

One small binary, `telex`. Carries its own usage instructions for agents: run
`telex skill`.

The repository also ships a Copilot CLI plugin marketplace (`.github/plugin/`)
and plugin manifest (`plugin.json` + `hooks.json`). The plugin maps Copilot
session env into generic telex session inputs, handles non-destructive
`sessionEnd`, and guards turn-end re-arming.

## Documentation

Full documentation (install, concepts, guides, and the generated CLI reference)
is at **<https://lossyrob.github.io/telex/>**.

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
export TELEX_SESSION_ID=quickstart   # PowerShell: $env:TELEX_SESSION_ID = "quickstart"
telex --address me send --to me --body "hello"   # from resolves to `me`
telex --address me inbox --all                   # read it back
```

No manual server setup and no config required. The first daemon-backed
verb auto-spawns a per-user local exchange for the default local SQLite store at
`~/.telex/telex.db`.

## For agents

Tell your agent: **"set up telex: run `telex skill`."** The binary self-describes,
so the agent learns to attach to an address, wait for messages, disposition them, and
message peers. To hand an agent a specific assignment in one command:

```sh
telex skill --address workstream:proj/node:issue-215
```

In Copilot CLI, install/use the telex plugin and bind with push delivery so
messages arrive as turns (no waiter to run or re-arm):

```sh
copilot plugin marketplace add lossyrob/telex
copilot plugin install telex@telex
telex --address workstream:proj/node:issue-215 copilot attach --copilot-bridge --description "<work>"
# then run the `extensions_reload` tool once; delivered telex messages arrive as turns.
telex --address workstream:proj/node:issue-215 copilot detach   # tear down when done
```

`telex wait` remains the generic pull primitive for scripts, CI, and non-extension
harnesses. See the [Copilot CLI push delivery guide](https://lossyrob.github.io/telex/guides/copilot-push.html)
for the session-env mapping, teardown, compatibility notes, and the marketplace
release-pinning convention.

## Networked backends

Add a Postgres backend once; then select it by name (or make it the default):

```sh
# Azure Postgres with Entra (telex fetches the token itself; uses `az login`,
# or `--entra-cred managed` on a devbox/VM with a managed identity):
telex backend add prod \
  --postgres "host=myserver.postgres.database.azure.com port=5432 user=me@example.com dbname=postgres sslmode=require" \
  --entra --schema telex --default

telex backend list
telex --backend prod send --to node:x --body "hi"
```

Secrets are referenced (`--entra`, `--password-env`, `--password-command`), never
stored in the config file. See the
[networked Postgres guide](https://lossyrob.github.io/telex/guides/postgres.html).

## How it works

A durable **address** is the responsibility being served; a per-user local
**exchange** daemon owns SQLite presence, lease heartbeats, delivery buffering,
and local IPC; a session `attach`es once to register an in-memory **station** for
its stable `TELEX_SESSION_ID`; `wait` is a one-shot daemon client that receives
one message and exits; `ack` is the explicit durable consumed mark. If the daemon
restarts, the next verb reconnects, re-registers on `NeedsAttach`, and continues
against the retained delivery buffer.

## Learn more

- **[User guide](https://lossyrob.github.io/telex/)**: install, concepts, guides, and the CLI reference
- **[SKILL.md](SKILL.md)**: how agents use telex (also `telex skill`)
- **[DESIGN.md](docs/design/DESIGN.md)**: the working design
- **[DECISIONS.md](docs/design/DECISIONS.md)**: the decision log
- **[DISPATCH.md](DISPATCH.md)**: forward-looking discovery & dispatch (broadcast, Contract-Net)
- **[EXTENSIONS.md](EXTENSIONS.md)**: proposal for extensions & capability cards (how addresses advertise what they do)
- **[TELEX.md](TELEX.md)** / **[PRODUCT-THESIS.md](PRODUCT-THESIS.md)**: the name, the metaphor, the thesis
- **[spike/](spike/)**: the validation spike that tested the design
