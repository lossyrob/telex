# Copilot CLI push delivery

In GitHub Copilot CLI, telex delivers messages to the agent as **turns**. The
agent does not run or re-arm a waiter. The full, version-matched Copilot workflow
is printed by the installed binary:

```sh
telex copilot skill
```

That command is the source of truth for the Copilot path. This guide is the
operator's overview.

## Install the plugin

```sh
copilot plugin marketplace add lossyrob/telex
copilot plugin install telex@telex
```

The plugin contributes session lifecycle hooks and provisions the push bridge. It
maps `$COPILOT_AGENT_SESSION_ID` to the generic telex session id. In bridge mode,
the extension heartbeat, not `$COPILOT_LOADER_PID`, is the push liveness signal.

## Bind and provision the bridge

First enable **Copilot Extensions** under `/experimental`. Copilot exposes the
`extensions_reload` tool only when this experimental feature is enabled.

```sh
telex --address <addr> copilot attach --copilot-bridge --description "<work>"
```

For first-time provisioning into the running session, run the `extensions_reload`
tool once (the agent does this; telex cannot trigger a reload). After that,
delivered telex messages arrive as new turns labelled
`[telex] from <addr> (<attention>)`.

If `extensions_reload` is unavailable, enable Copilot Extensions under
`/experimental`, re-provision with
`telex --address <addr> copilot resume --description "<work>"`, and then run
`extensions_reload`. If Copilot Extensions cannot be enabled, use the supported
[pull fallback](#fallback) or detach with
`telex --address <addr> copilot detach`.

## Receive and disposition

A pushed turn includes the `ack` and `handle` commands with the address, id, and
session filled in. The generic verbs do not read Copilot env vars, so they need
`--session "$COPILOT_AGENT_SESSION_ID"`:

```sh
telex ack --address <addr> --id <message-id> --session "$COPILOT_AGENT_SESSION_ID"
telex handle --address <addr> --id <message-id> --session "$COPILOT_AGENT_SESSION_ID" --note "completed"
```

Sending is not push: `telex send` and `telex reply` also need `--session`.

## CC observers

A session that should receive CC (observer) copies as turns opts in at bind time
with `--wake-on-cc`:

```sh
telex --address <addr> copilot attach --copilot-bridge --wake-on-cc --description "<work>"
```

Run `extensions_reload` only if this is first-time provisioning or recovery in the
already-running session. Without `--wake-on-cc`, CC copies are still buffered and
visible in `telex inbox --all`, but are not delivered as turns.
(`telex wait --wake-on-cc` is the separate pull-mode equivalent for non-Copilot
harnesses.)

## Tear down

```sh
telex --address <addr> copilot detach
```

This detaches the address and, when it was the last binding, removes the bridge
files so nothing reloads on a later resume.

Ordinary session end is resumable: it marks daemon attendance idle and clears
transient turn-guard state while retaining the extension, bindings, and registry.
On resume, Copilot discovers the retained extension during startup; run
`telex --address <addr> copilot resume --description "<work>"` to re-arm push and
rescan unacknowledged backlog. Use `extensions_reload` only for first-time
provisioning or recovery when the retained bridge is not live in an already-running
session, including the first resume after a Telex upgrade when the live bridge reports
an older build or protocol.

Inspect stale bridge files left by other sessions with `telex copilot gc --dry-run`.
Use `telex copilot gc --force` only after verifying a retained session will not resume.

## Fallback

If the bridge cannot load because Copilot Extensions cannot be enabled, push is
unavailable. Surface that plainly and prepare one Telex-owned pull fallback run:

```sh
telex --address <addr> copilot fallback prepare --description "<work>"
```

The JSON result contains a unique `run_dir` plus `launcher.program`,
`launcher.args`, and a ready-to-run `launcher.command`. Run that launcher as one
fully detached Copilot task. Unix uses the current telex binary directly; Windows
uses a generated PowerShell file for the detached-task compatibility path. Telex
does not detach the task itself and does not run an internal delivery loop.

Preparation is idempotent until the run writes `exit.code`, and it leaves push
unchanged if the launcher never starts. The running launcher atomically clears
push before entering exactly one [pull-mode wait](agent-pull.md). On completion,
read `exit.code` first, then the exact run's `delivery.json`/`message.json`; ack
and dedupe primary deliveries by message id before preparing the next run.

`telex --address <addr> status` reports `delivery_mode` separately from
`station_health`: `push` is bridge delivery, `pull` is the Copilot fallback, and
`conflict` is a version-skew/race tripwire. The daemon rejects simultaneous push
and pull coverage.

To return to push, stop the waiter before binding the bridge:

```sh
telex --address <addr> station stop --session "$COPILOT_AGENT_SESSION_ID"
telex --address <addr> copilot attach --copilot-bridge --description "<work>"
```

For an already-running session, run `extensions_reload` after the attach. The
version-matched `telex copilot skill` contains the full artifact, timeout,
recovery, and re-arm procedure.

## Compatibility

`telex copilot skill` prints the installed version and source build identifier, the
bridge protocol version, and the minimum compatible plugin version, and warns if the
plugin is older than the binary supports. `telex --version` includes the same build
identifier, while `telex --json version` exposes it as `version.build_id`. Published
release binaries are gated to report the release commit; source builds without Git
metadata may report `unknown`. Git fallback is accepted only for a standalone Telex
checkout, not an unrelated ancestor repository, and the value is diagnostic rather
than an attestation.

If the drain hook reports skew, inspect `version.current_exe` plus `Get-Command telex`
or `command -v telex`, reinstall the plugin and binary from the same release, and make
the versioned launcher precede stale shims on PATH before restarting Copilot. A binary
rollback must be paired with the plugin from the same release.

Release install scripts pin the plugin and binary to the same tag. The plugin shape is
validated against a specific Copilot CLI version; see the
[acceptance matrix](https://github.com/lossyrob/telex/blob/main/docs/design/copilot-plugin-validation.md).
