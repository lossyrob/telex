---
name: telex
description: Coordinate messages between AI agent sessions by attaching a session to a durable address, sending and receiving typed messages (delivered as turns in Copilot CLI), and recording auditable disposition. Use when the operator gives you a telex address or tells you to "set up telex" or run `telex skill`; when you need to message, hand off to, or receive work from another agent session; or when orchestrating worker sessions that coordinate over telex instead of polling GitHub comments. This is a bootstrap that loads the version-matched workflow and command syntax from the installed telex binary.
---

# Telex skill (bootstrap)

Telex is a CLI-first message fabric for AI agent sessions: ephemeral sessions attach to
durable addresses, exchange typed operational messages with answerback liveness, and leave
auditable disposition records. Use the single binary as `telex` (`telex.exe` on Windows).

**This file is only a bootstrap.** It deliberately does not embed the detailed workflow or
flag syntax, which change with the binary and would go stale here. Load the real,
version-matched instructions from the installed `telex` binary instead of trusting any
static copy in this plugin.

## Load runtime instructions from the binary

- **Copilot CLI (push delivery):** run **`telex copilot skill`**. It prints the
  version-matched Copilot workflow -- bind, load the in-session bridge, receive messages as
  turns, record disposition, and tear down -- plus the installed version and plugin/binary
  compatibility notes. This is the source of truth for the Copilot path.
- **Generic / non-Copilot harnesses (pull):** run **`telex skill`** (or
  `telex skill --address <addr>` for instructions tailored to an assigned address).

For an explicit plugin/binary compatibility check, pass this plugin's version:

```sh
telex copilot skill --plugin-version 0.1.0
```

If the binary reports that the plugin is older than it supports, follow the printed warning
and update the mismatched side rather than trusting these bootstrap notes.

## Command help is the source of truth for syntax

Do not rely on this file for exact flags. Before using specific commands, run the relevant
help from the installed binary:

```sh
telex --version
telex copilot skill
telex copilot --help
telex copilot <subcommand> --help
telex <core-command> --help   # e.g. telex send, telex status, telex ack, telex handle, telex wait
```

The installed `telex` binary owns the detailed, version-accurate behavior; this plugin only
tells you how to find it.
