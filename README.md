<p align="center">
  <img src="assets/telex.png" alt="Telex" width="360">
</p>

A CLI-first **message fabric for AI agent sessions**: durable addresses, typed
messages with answerback liveness, and an auditable record — over SQLite (local,
zero-config) or Postgres (networked, with or without Microsoft Entra auth).

One small binary, `telex`. It even carries its own usage instructions: run
`telex skill`.

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

That's it — no server, no config. The default backend is a local SQLite store at
`~/.telex/telex.db`.

## For agents

Tell your agent: **"set up telex — run `telex skill`."** The binary self-describes,
so the agent learns to attach to an address, wait for messages, disposition them, and
message peers. To hand an agent a specific assignment in one command:

```sh
telex skill --address workstream:proj/node:issue-215
```

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

A durable **address** is the responsibility being served; an ephemeral **lease** is the
live session serving it; a typed **message** carries coordination; a **disposition**
records what happened. A session `attach`es to an address to start a **station** — the
running presence serving it (a resident **holder** that holds the lease and answers
liveness in the background, plus a **waiter** loop) — and loops `telex wait` to receive
messages, acting and dispositioning each at its next turn.

## Learn more

- **[SKILL.md](SKILL.md)** — how agents use telex (also `telex skill`)
- **[DESIGN.md](DESIGN.md)** — the working design
- **[DECISIONS.md](DECISIONS.md)** — the decision log
- **[DISPATCH.md](DISPATCH.md)** — forward-looking discovery & dispatch (broadcast, Contract-Net)
- **[TELEX.md](TELEX.md)** / **[PRODUCT-THESIS.md](PRODUCT-THESIS.md)** — the name, the metaphor, the thesis
- **[spike/](spike/)** — the throwaway validation spike that de-risked the design
