# Operating telex

## The daemon lifecycle

The exchange auto-spawns on the first daemon-backed command; there is no manual
start. Inspect the resolved backend and address projection with:

```sh
telex status --address <addr>
```

The daemon runs per user. Inspect and control it with the `telex daemon` family
(run `telex daemon --help` for the full set):

```sh
telex daemon status            # daemon internals
telex daemon version           # running daemon version
telex daemon stop --drain      # stop after draining in-flight work
```

Runtime state (the IPC socket and lease state) lives in a per-user runtime
directory: on Windows under `%LOCALAPPDATA%\telex\run`, on Unix a per-user socket
directory. The local message store is the SQLite file at `~/.telex/telex.db`.

## Stopping a station

`station stop` is the symmetric inverse of the attach and wait loop. It marks the
station non-attending, releases membership durably, and waits for tracked waiters
to exit:

```sh
telex station stop --address <addr>
```

After it returns, a later message to the address stays queued until a future
attach or wait; it is not consumed by an orphaned waiter.

## Teardown: which command to use

| Command | Effect |
|---|---|
| `telex detach --address <addr>` | Drop this session's membership of the address, non-destructively. The station and durable buffer remain. |
| `telex station stop --address <addr>` | Mark the station non-attending, release membership durably, and wait for tracked waiters to exit. |
| `telex address retire --address <addr>` | Retire the address so it drops from directory listings. |
| `telex daemon stop --drain` | Stop the local exchange after draining in-flight work. |
| `telex copilot detach` | Copilot push sessions: detach the address and remove the bridge files. |

None of these delete durable messages; a later attach or wait resumes against the
retained buffer.

## Upgrading the binary

Release installs use a versioned layout instead of overwriting the binary on
`PATH` in place. A stable launcher lives under the install root's `bin/`, immutable
binaries live under `versions/<tag>/`, and `current` selects the version new
invocations use. Old in-flight processes keep running on their version while new
shells use the selected one.

```text
<install-root>/
  bin/telex(.exe)
  versions/<tag>/telex(.exe)
  current
  previous
```

Upgrade, roll back, and inspect versions:

```sh
telex version --json

# Fetch, verify, and install the latest compatible public release:
telex upgrade

# Install a specific public release by tag:
telex upgrade --version vX.Y.Z

# Install a local/manual build (no download):
telex upgrade --from <path-to-telex-binary> --version vX.Y.Z

telex rollback
telex gc --dry-run
```

Without `--from`, `telex upgrade` discovers a GitHub release (the latest full release by
default, or `--version <tag>`), selects this platform's asset, downloads the archive and its
`.sha256` sidecar, and **verifies the checksum before installing** — then installs through the
same versioned layout as the local path. It is **fail-closed**: a missing or mismatched
checksum, a missing platform asset, an unsupported platform, an incompatible version, or a
network/rate-limit error aborts without changing `current`. Set `GITHUB_TOKEN` to raise the API
rate limit. Prebuilt binaries are published for Windows (x86_64, ARM64), Linux (x86_64), and
macOS (Apple Silicon, Intel); on other platforms install from source with
`cargo install --git https://github.com/lossyrob/telex --features entra`. If telex is already on
the resolved release it reports "already current" and does nothing (override with `--force`).

`telex upgrade` reads the downloaded (checksum-verified) binary's own metadata by running it
once (`telex --json version`) before installing; in locked-down environments an OS quarantine
prompt (macOS Gatekeeper, Windows SmartScreen) on that step is the likely cause if an upgrade
stalls. The checksum verifies **integrity**, not authenticity — it protects against a corrupted
or truncated download, and the trust root is the GitHub repository the asset comes from.

`telex upgrade` and `telex rollback` drain the current local daemon before
switching `current`, unless `--skip-drain` is passed. Rollback refuses installed
versions whose manifest is incompatible with this build's protocol/schema floor.

For a manual in-place replacement, drain and replace in this order:

```sh
telex station stop --address <addr>
telex daemon stop --drain
# replace the telex binary
telex attach --address <addr> --description "<s>"
telex wait --address <addr> --out-dir <dir>
```

If a session resumes without an armed waiter, recovery is durable: inspect
`telex inbox --address <addr>` and `telex read --id <id>`, then arm a fresh wait.

## Auditing

Export messages and disposition history as JSON lines for provenance:

```sh
telex export --address <addr>
telex export --thread <id>
telex export --since <id>
```

## Recovering from a lost daemon

A `wait` that finds no daemon exits with a distinct code (see
[Exit codes](../reference/exit-codes.md)). Run `telex attach` (the spawning and
recovery verb) and re-arm the wait. If a replacement daemon already exists, a
wait can reconnect during its bounded reconnect grace.

## Turn-end and resume reconciliation

For turn-end guards or resume reconciliation, use
`telex station status --session <id>` to get a compact JSON projection of the
session's attended addresses, waiter counts, station health, and pending
unconsumed counts.

## Uninstall and cleanup

1. Stop the daemon: `telex daemon stop --drain`.
2. Remove local state: delete `~/.telex/` (the SQLite store and config).
3. Remove the Copilot plugin, if installed: `copilot plugin uninstall telex@telex`.
4. For a Postgres backend, drop the telex schema in the database if it is no
   longer needed.
