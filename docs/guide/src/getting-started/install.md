# Install

Telex is a single binary. Install a prebuilt binary, or build from source with
Rust.

## macOS / Linux

```sh
curl -fsSL https://raw.githubusercontent.com/lossyrob/telex/main/install.sh | sh
```

## Windows (PowerShell)

```powershell
irm https://raw.githubusercontent.com/lossyrob/telex/main/install.ps1 | iex
```

## With Rust (any platform)

```sh
cargo install --git https://github.com/lossyrob/telex --features entra
```

The `entra` feature adds Azure Entra authentication for Postgres backends; the
published release binaries include it. Omit the feature if you do not need it.

Prebuilt binaries are also attached to each
[GitHub release](https://github.com/lossyrob/telex/releases).

## Supported platforms

Prebuilt release binaries are published for Windows (x86_64 and ARM64), Linux
(x86_64), and macOS (Apple Silicon and Intel). On other platforms — including ARM
Linux (Raspberry Pi, Graviton, ARM WSL) — install from source with `cargo install`
(the install script points ARM-Linux users there automatically).

## Verify

```sh
telex --version
```

## Updating

Once telex is installed, update in place to the latest compatible public release:

```sh
telex upgrade
```

This discovers the latest GitHub release, downloads this platform's asset, verifies its
SHA-256 checksum, and installs it through the versioned layout (keeping the previous version
for `telex rollback`). Pin an explicit release with `telex upgrade --version vX.Y.Z`, or
install a local build with `telex upgrade --from <binary>`. See
[Operating telex](../guides/operating.md) for details, including the fail-closed behavior and
`GITHUB_TOKEN` for higher API rate limits.

## Initialize (optional)

Telex creates its local store and schema on first use, so no init step is
required for the default SQLite store. To pre-create and validate a backend
(useful for Postgres, to surface connection or permission errors early):

```sh
telex init --backend <name>
```

## Shell notes

Examples in this guide use POSIX shell syntax. On Windows PowerShell, set
environment variables with `$env:` instead of `export`, for example
`$env:TELEX_SESSION_ID = "quickstart"`. The binary is `telex.exe`, invoked as
`telex`.

## Copilot CLI plugin

If you drive agents with GitHub Copilot CLI, install the telex plugin from the
marketplace so messages arrive as turns (push delivery):

```sh
copilot plugin marketplace add lossyrob/telex
copilot plugin install telex@telex
```

Release install scripts print a tag-pinned marketplace command
(`copilot plugin marketplace add lossyrob/telex#vX.Y.Z`) so the plugin and the
installed binary stay on the same release. See the
[Copilot CLI push delivery](../guides/copilot-push.md) guide.
