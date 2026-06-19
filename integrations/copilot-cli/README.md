# Telex — Copilot CLI plugin (experiment)

A minimal Copilot CLI plugin that closes a session's telex stations when the session
ends, using the CLI's **`sessionEnd` lifecycle hook**. This lets you run telex holders
as **detached** background tasks (clean UX — no perpetual "Working" spinner) without
risking an **orphaned** holder that outlives its session.

> Status: implements the dismiss half of session-binding (issue #23), complementing the
> in-binary pid-watch (#17, DECISIONS 0011) which handles ungraceful death. `telex attach`
> writes the ownership registry; the `sessionEnd` hook runs `telex session-end`, which
> detaches the session's stations on dismiss/quit.

## Why hooks (vs. process-watching)

We measured Copilot CLI session lifecycle empirically:

- `session.ended` fires on **both dismiss and quit**, and is **decoupled from process
  lifecycle** — it fires on a dismiss even though the `copilot.exe` process keeps
  running. (Verified: a dismiss fired `session.ended reason=user_exit` while a PID
  watcher showed the process still alive; a quit fired it and the process died.)
- Each end carries a JSON payload on stdin including `sessionId` and `endReason`
  (`complete | error | abort | timeout | user_exit`).
- Resume fires `session.started` with `source=resume`.

A PID watch can only catch a hard process exit (quit), **not** a dismiss. The hook
catches both — so it is the correct signal for "this session is no longer serving its
station."

## How it works

```
telex attach     ─►  writes a per-station registry file under sessions/<sessionId>/
session ends      ─►  Copilot fires the sessionEnd hook, piping {sessionId, endReason} to stdin
sessionEnd hook   ─►  runs `telex session-end`, which reads sessionId and stops each
                      registered station for that session over local IPC
```

The cleanup logic lives in the **binary** (`telex session-end`), not in shell scripts. The
hook is just `telex session-end` for both `powershell` and `bash` (see `hooks.json`). This
keeps a single source of truth for the path/registry contract, needs no shell JSON parsing,
and is **backend-independent**: stopping a holder is an address-keyed IPC shutdown, so it
works even when the configured backend is unavailable (e.g. Entra offline). The holder then
releases its own lease on shutdown; any already-gone holder's lease lapses via the TTL window.

> **Requires `telex` on `PATH`** when the hook runs (it invokes `telex session-end`). If
> `telex` is not found, the hook simply no-ops and cleanup falls back to the pid-watch (on
> quit) / TTL (on crash).

### Station registry

`telex attach` writes a registry entry for each station it holds. The session id is taken from
`$TELEX_SESSION_ID`, else `$COPILOT_AGENT_SESSION_ID` (the same id the hook receives on stdin).
When neither is set the registry is disabled and telex still works normally.

**One file per station** (so concurrent attaches within a session never race on a shared file).
The filename suffixes a hash of the full address so distinct addresses that sanitize alike
(`a:b`, `a.b`, `a_b`) never collide. Override the registry dir with `TELEX_SESSION_DIR`:

```
<TELEX_SESSION_DIR | <telex_home>/sessions>/<sessionId>/<sanitized-address>-<hash>.json
```

`<telex_home>` follows the binary: `$TELEX_HOME` else `~/.telex`. Each file:

```json
{
  "address": "station:x",
  "telex": "<path to the telex binary that wrote the record>",
  "env": { "TELEX_HOME": "...", "TELEX_CONFIG": "...", "TELEX_DB": "...", "TELEX_BACKEND": "..." }
}
```

`address` is authoritative. `telex` and `env` are **informational** — `telex session-end` stops
holders in-process over IPC and does not execute the recorded binary or apply the recorded env,
so a stale path or a crafted record cannot redirect execution.

`telex session-end` lists the session's records, stops each holder, removes a record only once
its holder is confirmed stopped/gone (a holder that is present but unresponsive keeps its record
for a later retry), and drops the session dir when it is empty. Each holder also unregisters its
own record on clean shutdown / `telex detach`.

### Re-attendance on resume

Closing on dismiss is intentional, but a **resumed** session does not auto-reopen its
stations (no `sessionStart` hook). Re-attend **manually** by re-running `telex attach` (and
re-arming `telex wait`) for the address. Messages queued while the station was closed are
delivered on the next attach via durable backlog delivery (see DECISIONS 0010).

## Files

- `plugin.json` — plugin manifest (declares `hooks.json`).
- `hooks.json` — registers the `sessionEnd` hook; both `powershell` and `bash` run
  `telex session-end`.

(No shell scripts: the logic is the `telex session-end` subcommand, covered by the crate's
unit tests, `clippy`, and `fmt` in CI.)

## Environment overrides

- `TELEX_SESSION_DIR` — registry directory (default `<telex_home>/sessions`).
- `TELEX_HOOK_LOG` — append a log line per action here (default: stderr).
- `TELEX_SESSION_ID` — overrides `$COPILOT_AGENT_SESSION_ID` as the registry key.

Keep `TELEX_SESSION_DIR` user-private; the records are read at session end.

## Install (local)

```
copilot
/plugin            # install from this local directory
```

Hooks load at session start, so a newly installed plugin takes effect in the next session.

You can exercise the cleanup directly without the CLI:

```sh
echo '{"sessionId":"<id>","endReason":"user_exit"}' | telex session-end
# or
telex session-end --session-id <id>
```

## Not yet covered (future work)

- A dedicated re-attend command (V1: re-run `telex attach` manually on resume).
- A `sessionStart` (`source=resume`) hook to **re-attach** stations on resume.
- Registry GC/TTL prune for session dirs left behind by failed detaches (`telex session prune`).
- Packaging the plugin in the release archive (`release.yml` is unchanged by this PR).
- `detach`'s receipt reporting `lease_released:false` even though the holder self-releases
  its lease over IPC (occupancy frees within the TTL window) — cosmetic; the lease *is* released.

Ungraceful end (crash, kill, power loss) where no hook runs is covered separately by the
in-binary pid-watch (`--session-pid` / `$TELEX_SESSION_PID`, DECISIONS 0011).
