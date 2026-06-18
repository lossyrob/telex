# Telex — Copilot CLI plugin (experiment)

A minimal Copilot CLI plugin that closes a session's telex stations when the session
ends, using the CLI's **`sessionEnd` lifecycle hook**. This lets you run telex holders
as **detached** background tasks (clean UX — no perpetual "Working" spinner) without
risking an **orphaned** holder that outlives its session.

> Status: implements the dismiss half of session-binding (issue #23), complementing the
> in-binary pid-watch (#17, DECISIONS 0011) which handles ungraceful death. `telex attach`
> writes the ownership registry; the `sessionEnd` hook detaches on dismiss/quit.

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
telex attach        ─►  write a per-station registry file under sessions/<sessionId>/
session ends         ─►  Copilot fires sessionEnd hook (stdin: {sessionId, endReason})
sessionEnd hook      ─►  enumerate sessions/<sessionId>/* ─► telex detach each station
```

### Station registry

`telex attach` writes a registry entry for each station it holds; the `sessionEnd` hook
reads them. The session id is taken from `$TELEX_SESSION_ID`, else `$COPILOT_AGENT_SESSION_ID`
(the same id the hook receives on stdin). When neither is set the registry is disabled and
telex still works normally.

**One file per station** (so concurrent attaches within a session never race on a shared
file). Override the dir with `TELEX_SESSION_DIR`:

```
<TELEX_SESSION_DIR | $HOME/.telex/sessions>/<sessionId>/<sanitized-address>.json
```

Each file:

```json
{
  "address": "station:x",
  "telex": "<path to the telex binary that holds the station>",
  "env": { "TELEX_HOME": "...", "TELEX_CONFIG": "...", "TELEX_DB": "...", "TELEX_BACKEND": "..." }
}
```

`env` captures the backend-selecting variables present at attach time, and `telex` is the
holding binary's own path — so the hook resolves the same store and build even for isolated
or named backends, and even when the holder is already gone and only the lease lingers.

The hook enumerates the session's files, applies each station's `env`, runs
`<telex> --address <address> detach`, then removes the whole session directory. Each holder
also unregisters its own record on clean shutdown / `telex detach`.

### Re-attendance on resume

Closing on dismiss is intentional, but a **resumed** session does not auto-reopen its
stations (no `sessionStart` hook). Re-attend **manually** by re-running `telex attach` (and
re-arming `telex wait`) for the address. Messages queued while the station was closed are
delivered on the next attach via durable backlog delivery (see DECISIONS 0010).

## Files

- `plugin.json` — plugin manifest (declares `hooks.json`).
- `hooks.json` — registers the `sessionEnd` hook (powershell + bash).
- `scripts/session-end.ps1` / `session-end.sh` — read the stdin payload, detach the
  session's registered stations, log to `$TELEX_HOOK_LOG`
  (default `$HOME/.telex/logs/session-end-hook.log`).

## Environment overrides (for testing)

- `TELEX_SESSION_DIR` — registry directory (default `$HOME/.telex/sessions`).
- `TELEX_HOOK_LOG` — hook log file (default `$HOME/.telex/logs/session-end-hook.log`).

## Install (local)

```
copilot
/plugin            # install from this local directory
```

Hooks load at session start, so a newly installed plugin takes effect in the next
session.

## Not yet covered (future work)

- A dedicated re-attend command (V1: re-run `telex attach` manually on resume).
- A `sessionStart` (`source=resume`) hook to **re-attach** stations on resume.
- `detach`'s receipt reporting `lease_released:false` even though the holder self-releases
  its lease over IPC (occupancy frees within the TTL window) — cosmetic; the lease *is*
  released.

Ungraceful end (crash, kill, power loss) where no hook runs is covered separately by the
in-binary pid-watch (`--session-pid` / `$TELEX_SESSION_PID`, DECISIONS 0011).
