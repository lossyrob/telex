# CLI reference

> This page is generated from the installed `telex` binary (`telex 0.1.0`) by
> `docs/guide/generate-reference.sh`. Do not edit it by hand; it is
> regenerated on every docs build so it stays matched to the binary.
> For the workflow narrative, see the [Guides](../guides/agent-pull.md).

## `telex`

```text
Telex lets ephemeral agent sessions attach to durable addresses, exchange typed operational messages with answerback liveness, and leave an auditable disposition record. Run `telex skill` to load agent usage instructions for this build.

Usage: telex.exe [OPTIONS] <COMMAND>

Commands:
  init      Initialize ~/.telex and the backend schema
  status    Show config, backend, address, station/occupancy status
  skill     Print the agent usage skill (how to use telex) for this build
  attach    Attach this session to an address and exit
  detach    Detach this session's address membership
  station   Station lifecycle operations
  wait      Block until an actionable message arrives, print it as JSON, and exit
  inbox     List actionable and recent messages for an address
  read      Read a message (optionally with thread context)
  send      Send a message to an address
  reply     Reply to a message; threads under it
  ack       Acknowledge a message
  handle    Mark a message handled (terminal)
  defer     Defer a message
  reject    Reject a message (terminal)
  close     Close a message/thread (terminal)
  escalate  Escalate a message
  address   Address directory operations
  resolve   Resolve target address(es) by description match or tag
  backend   Manage configured backends (named profiles in ~/.telex/config.toml)
  export    Export messages and disposition history as JSON lines
  help      Print this message or the help of the given subcommand(s)

Options:
      --backend <BACKEND>
          Configured backend to use, by name (default: the configured default backend)
          
          [env: TELEX_BACKEND=]

      --db <DB>
          Override the SQLite path for this invocation (sqlite backends only)
          
          [env: TELEX_DB=]

      --address <ADDRESS>
          Address to operate on (default for commands that act on one address)
          
          [env: TELEX_ADDRESS=]

      --json
          Force JSON output

      --text
          Force concise text output

  -h, --help
          Print help (see a summary with '-h')

  -V, --version
          Print version
```

## `telex init`

```text
Initialize ~/.telex and the backend schema

Usage: telex.exe init [OPTIONS]

Options:
      --backend <BACKEND>  Configured backend to use, by name (default: the configured default backend) [env: TELEX_BACKEND=]
      --db <DB>            Override the SQLite path for this invocation (sqlite backends only) [env: TELEX_DB=]
      --address <ADDRESS>  Address to operate on (default for commands that act on one address) [env: TELEX_ADDRESS=]
      --json               Force JSON output
      --text               Force concise text output
  -h, --help               Print help
```

## `telex status`

```text
Show config, backend, address, station/occupancy status

Usage: telex.exe status [OPTIONS]

Options:
      --backend <BACKEND>  Configured backend to use, by name (default: the configured default backend) [env: TELEX_BACKEND=]
      --db <DB>            Override the SQLite path for this invocation (sqlite backends only) [env: TELEX_DB=]
      --address <ADDRESS>  Address to operate on (default for commands that act on one address) [env: TELEX_ADDRESS=]
      --json               Force JSON output
      --text               Force concise text output
  -h, --help               Print help
```

## `telex skill`

```text
Print the agent usage skill (how to use telex) for this build

Usage: telex.exe skill [OPTIONS]

Options:
      --address <ADDRESS>  Tailor the instructions for a specific assigned address
      --backend <BACKEND>  Configured backend to use, by name (default: the configured default backend) [env: TELEX_BACKEND=]
      --db <DB>            Override the SQLite path for this invocation (sqlite backends only) [env: TELEX_DB=]
      --raw                Print the embedded SKILL.md verbatim (including frontmatter)
      --json               Force JSON output
      --text               Force concise text output
  -h, --help               Print help
```

## `telex attach`

```text
Attach this session to an address and exit

Usage: telex.exe attach [OPTIONS]

Options:
      --backend <BACKEND>
          Configured backend to use, by name (default: the configured default backend) [env: TELEX_BACKEND=]
      --description <DESCRIPTION>
          One-line directory description of what this session is doing
      --db <DB>
          Override the SQLite path for this invocation (sqlite backends only) [env: TELEX_DB=]
      --scope <SCOPE>
          Project/workstream scope this address belongs to
      --address <ADDRESS>
          Address to operate on (default for commands that act on one address) [env: TELEX_ADDRESS=]
      --tags <TAGS>
          Comma-separated coarse tags (e.g. issue:215,repo:telex)
      --heartbeat-secs <HEARTBEAT_SECS>
          Deprecated compatibility flag; the daemon owns lease heartbeat cadence [default: 5]
      --json
          Force JSON output
      --poll-secs <POLL_SECS>
          Deprecated compatibility flag; the daemon owns backend polling [default: 1]
      --text
          Force concise text output
      --keepalive-secs <KEEPALIVE_SECS>
          Deprecated compatibility flag; daemon waiters use daemon IPC frames [default: 3]
      --occupant <OCCUPANT>
          Occupant identity recorded on the lease (default: session host/pid)
      --session <SESSION>
          Stable session identity for daemon membership [env: TELEX_SESSION_ID=]
      --push
          Deprecated compatibility flag; daemon delivery owns push/poll behavior
      --session-pid <SESSION_PID>
          Back-compat watch pid. Converted to an anchor watch-pid for daemon liveness
      --watch-pid <WATCH_PID>
          Watch a pid as a typed liveness predicate. Accepts PID, anchor:PID, required:PID, PID:anchor, or PID:required. Repeat to add multiple watch pids
      --session-poll-secs <SESSION_POLL_SECS>
          Deprecated compatibility flag; daemon liveness cadence is internal [default: 2]
      --no-session-bind
          Do not convert `$TELEX_SESSION_PID` into a daemon watch-pid
  -h, --help
          Print help
```

## `telex detach`

```text
Detach this session's address membership

Usage: telex.exe detach [OPTIONS]

Options:
      --backend <BACKEND>  Configured backend to use, by name (default: the configured default backend) [env: TELEX_BACKEND=]
      --session <SESSION>  Stable session identity for daemon membership [env: TELEX_SESSION_ID=]
      --db <DB>            Override the SQLite path for this invocation (sqlite backends only) [env: TELEX_DB=]
      --address <ADDRESS>  Address to operate on (default for commands that act on one address) [env: TELEX_ADDRESS=]
      --json               Force JSON output
      --text               Force concise text output
  -h, --help               Print help
```

## `telex station`

```text
Station lifecycle operations

Usage: telex.exe station [OPTIONS] <COMMAND>

Commands:
  status  Show this session's attended addresses and waiter state
  stop    Stop this session's station: release membership and drain its live waiters
  help    Print this message or the help of the given subcommand(s)

Options:
      --backend <BACKEND>  Configured backend to use, by name (default: the configured default backend) [env: TELEX_BACKEND=]
      --db <DB>            Override the SQLite path for this invocation (sqlite backends only) [env: TELEX_DB=]
      --address <ADDRESS>  Address to operate on (default for commands that act on one address) [env: TELEX_ADDRESS=]
      --json               Force JSON output
      --text               Force concise text output
  -h, --help               Print help
```

## `telex station status`

```text
Show this session's attended addresses and waiter state

Usage: telex.exe station status [OPTIONS]

Options:
      --backend <BACKEND>  Configured backend to use, by name (default: the configured default backend) [env: TELEX_BACKEND=]
      --session <SESSION>  Stable session identity for daemon membership [env: TELEX_SESSION_ID=]
      --all-sessions       Show all stations in the selected store instead of only this session
      --db <DB>            Override the SQLite path for this invocation (sqlite backends only) [env: TELEX_DB=]
      --address <ADDRESS>  Address to operate on (default for commands that act on one address) [env: TELEX_ADDRESS=]
      --json               Force JSON output
      --text               Force concise text output
  -h, --help               Print help
```

## `telex station stop`

```text
Stop this session's station: release membership and drain its live waiters

Usage: telex.exe station stop [OPTIONS]

Options:
      --backend <BACKEND>              Configured backend to use, by name (default: the configured default backend) [env: TELEX_BACKEND=]
      --session <SESSION>              Stable session identity for daemon membership [env: TELEX_SESSION_ID=]
      --db <DB>                        Override the SQLite path for this invocation (sqlite backends only) [env: TELEX_DB=]
      --wait-grace-ms <WAIT_GRACE_MS>  How long to wait for live waiter processes to exit after teardown is signaled (ms) [default: 3000]
      --address <ADDRESS>              Address to operate on (default for commands that act on one address) [env: TELEX_ADDRESS=]
      --json                           Force JSON output
      --text                           Force concise text output
  -h, --help                           Print help
```

## `telex wait`

```text
Block until an actionable message arrives, print it as JSON, and exit

Usage: telex.exe wait [OPTIONS]

Options:
      --backend <BACKEND>
          Configured backend to use, by name (default: the configured default backend) [env: TELEX_BACKEND=]
      --session <SESSION>
          Stable session identity for daemon membership [env: TELEX_SESSION_ID=]
      --db <DB>
          Override the SQLite path for this invocation (sqlite backends only) [env: TELEX_DB=]
      --timeout-ms <TIMEOUT_MS>
          Give up waiting after this many milliseconds (exit code 2); default is no idle timeout
      --address <ADDRESS>
          Address to operate on (default for commands that act on one address) [env: TELEX_ADDRESS=]
      --min-attention <MIN_ATTENTION>
          Only wake for messages at this attention or higher priority
      --json
          Force JSON output
      --since <SINCE>
          Resume delivery strictly after this message id [default: 0]
      --hang-ms <HANG_MS>
          Deprecated idle-wait compatibility watchdog. For daemon waits, only applies after timeout-ms [default: 8000]
      --text
          Force concise text output
      --reconnect-grace-ms <RECONNECT_GRACE_MS>
          Retry daemon reconnect/re-register for this long after EOF/restart (ms) [env: TELEX_RECONNECT_GRACE_MS=]
      --stale-heartbeat-ms <STALE_HEARTBEAT_MS>
          Holder DB-heartbeat age beyond which it is considered degraded (ms) [default: 15000]
      --out-dir <OUT_DIR>
          Write outcome artifacts into this directory so a detached, variable-free invocation can deliver results without relying on captured stdout. Writes `message.json` (on delivery), `status.json` (always), and `exit.code` (always, written last as the completion marker)
  -h, --help
          Print help
```

## `telex inbox`

```text
List actionable and recent messages for an address

Usage: telex.exe inbox [OPTIONS]

Options:
      --all                Include all recent messages, not just actionable ones
      --backend <BACKEND>  Configured backend to use, by name (default: the configured default backend) [env: TELEX_BACKEND=]
      --db <DB>            Override the SQLite path for this invocation (sqlite backends only) [env: TELEX_DB=]
      --limit <LIMIT>      Maximum messages to list [default: 50]
      --address <ADDRESS>  Address to operate on (default for commands that act on one address) [env: TELEX_ADDRESS=]
      --json               Force JSON output
      --text               Force concise text output
  -h, --help               Print help
```

## `telex read`

```text
Read a message (optionally with thread context)

Usage: telex.exe read [OPTIONS] --id <ID>

Options:
      --backend <BACKEND>  Configured backend to use, by name (default: the configured default backend) [env: TELEX_BACKEND=]
      --id <ID>            Message id to read
      --db <DB>            Override the SQLite path for this invocation (sqlite backends only) [env: TELEX_DB=]
      --thread             Include compact thread context
      --address <ADDRESS>  Address to operate on (default for commands that act on one address) [env: TELEX_ADDRESS=]
      --full               Include full thread history and dispositions
      --json               Force JSON output
      --text               Force concise text output
  -h, --help               Print help
```

## `telex send`

```text
Send a message to an address

Usage: telex.exe send [OPTIONS] --to <TO>

Options:
      --backend <BACKEND>      Configured backend to use, by name (default: the configured default backend) [env: TELEX_BACKEND=]
      --to <TO>                Destination address
      --db <DB>                Override the SQLite path for this invocation (sqlite backends only) [env: TELEX_DB=]
      --subject <SUBJECT>      Subject line
      --address <ADDRESS>      Address to operate on (default for commands that act on one address) [env: TELEX_ADDRESS=]
      --body <BODY>            Message body (inline). Body/subject/metadata are capped below the 1 MiB IPC frame
      --body-file <BODY_FILE>  Read the message body from UTF-8 (`-` stdin); capped below the 1 MiB IPC frame
      --json                   Force JSON output
      --cc <CC>                CC addresses (visible observers). May be repeated and/or comma-separated
      --text                   Force concise text output
      --kind <KIND>            Message kind/profile label [default: note]
      --attention <ATTENTION>  Attention level: interrupt | next-checkpoint | background | fyi [default: background]
      --requires-disposition   Mark that the recipient must disposition this message
      --from <FROM>            Sender address (defaults to the global --address if set)
      --metadata <METADATA>    Arbitrary JSON metadata; counted with body/subject against the IPC payload cap
      --session <SESSION>      Stable session identity for daemon membership [env: TELEX_SESSION_ID=]
  -h, --help                   Print help
```

## `telex reply`

```text
Reply to a message; threads under it

Usage: telex.exe reply [OPTIONS] --to-message <TO_MESSAGE>

Options:
      --backend <BACKEND>        Configured backend to use, by name (default: the configured default backend) [env: TELEX_BACKEND=]
      --to-message <TO_MESSAGE>  The message id being replied to
      --body <BODY>              Reply body (inline). Body/subject are capped below the 1 MiB IPC frame
      --db <DB>                  Override the SQLite path for this invocation (sqlite backends only) [env: TELEX_DB=]
      --address <ADDRESS>        Address to operate on (default for commands that act on one address) [env: TELEX_ADDRESS=]
      --body-file <BODY_FILE>    Read the reply body from UTF-8 (`-` stdin); capped below the 1 MiB IPC frame
      --json                     Force JSON output
      --subject <SUBJECT>        Subject (defaults to "Re: <parent subject>")
      --cc <CC>                  CC addresses (visible observers). May be repeated and/or comma-separated
      --text                     Force concise text output
      --attention <ATTENTION>    Attention level [default: background]
      --requires-disposition     Mark that the recipient must disposition this reply
      --from <FROM>              Sender address (defaults to the global --address if set)
      --kind <KIND>              Message kind/profile label [default: note]
      --session <SESSION>        Stable session identity for daemon membership [env: TELEX_SESSION_ID=]
  -h, --help                     Print help
```

## `telex ack`

```text
Acknowledge a message

Usage: telex.exe ack [OPTIONS] --id <ID>

Options:
      --backend <BACKEND>      Configured backend to use, by name (default: the configured default backend) [env: TELEX_BACKEND=]
      --id <ID>                Message id to disposition
      --db <DB>                Override the SQLite path for this invocation (sqlite backends only) [env: TELEX_DB=]
      --note <NOTE>            Optional note recorded with the disposition
      --address <ADDRESS>      Address to operate on (default for commands that act on one address) [env: TELEX_ADDRESS=]
      --recipient <RECIPIENT>  Recipient address whose disposition this is (defaults to the message's to_addr)
      --json                   Force JSON output
      --session <SESSION>      Stable session identity for daemon membership [env: TELEX_SESSION_ID=]
      --text                   Force concise text output
  -h, --help                   Print help
```

## `telex handle`

```text
Mark a message handled (terminal)

Usage: telex.exe handle [OPTIONS] --id <ID>

Options:
      --backend <BACKEND>      Configured backend to use, by name (default: the configured default backend) [env: TELEX_BACKEND=]
      --id <ID>                Message id to disposition
      --db <DB>                Override the SQLite path for this invocation (sqlite backends only) [env: TELEX_DB=]
      --note <NOTE>            Optional note recorded with the disposition
      --address <ADDRESS>      Address to operate on (default for commands that act on one address) [env: TELEX_ADDRESS=]
      --recipient <RECIPIENT>  Recipient address whose disposition this is (defaults to the message's to_addr)
      --json                   Force JSON output
      --session <SESSION>      Stable session identity for daemon membership [env: TELEX_SESSION_ID=]
      --text                   Force concise text output
  -h, --help                   Print help
```

## `telex defer`

```text
Defer a message

Usage: telex.exe defer [OPTIONS] --id <ID>

Options:
      --backend <BACKEND>      Configured backend to use, by name (default: the configured default backend) [env: TELEX_BACKEND=]
      --id <ID>                Message id to disposition
      --db <DB>                Override the SQLite path for this invocation (sqlite backends only) [env: TELEX_DB=]
      --note <NOTE>            Optional note recorded with the disposition
      --address <ADDRESS>      Address to operate on (default for commands that act on one address) [env: TELEX_ADDRESS=]
      --recipient <RECIPIENT>  Recipient address whose disposition this is (defaults to the message's to_addr)
      --json                   Force JSON output
      --session <SESSION>      Stable session identity for daemon membership [env: TELEX_SESSION_ID=]
      --text                   Force concise text output
  -h, --help                   Print help
```

## `telex reject`

```text
Reject a message (terminal)

Usage: telex.exe reject [OPTIONS] --id <ID>

Options:
      --backend <BACKEND>      Configured backend to use, by name (default: the configured default backend) [env: TELEX_BACKEND=]
      --id <ID>                Message id to disposition
      --db <DB>                Override the SQLite path for this invocation (sqlite backends only) [env: TELEX_DB=]
      --note <NOTE>            Optional note recorded with the disposition
      --address <ADDRESS>      Address to operate on (default for commands that act on one address) [env: TELEX_ADDRESS=]
      --recipient <RECIPIENT>  Recipient address whose disposition this is (defaults to the message's to_addr)
      --json                   Force JSON output
      --session <SESSION>      Stable session identity for daemon membership [env: TELEX_SESSION_ID=]
      --text                   Force concise text output
  -h, --help                   Print help
```

## `telex close`

```text
Close a message/thread (terminal)

Usage: telex.exe close [OPTIONS] --id <ID>

Options:
      --backend <BACKEND>      Configured backend to use, by name (default: the configured default backend) [env: TELEX_BACKEND=]
      --id <ID>                Message id to disposition
      --db <DB>                Override the SQLite path for this invocation (sqlite backends only) [env: TELEX_DB=]
      --note <NOTE>            Optional note recorded with the disposition
      --address <ADDRESS>      Address to operate on (default for commands that act on one address) [env: TELEX_ADDRESS=]
      --recipient <RECIPIENT>  Recipient address whose disposition this is (defaults to the message's to_addr)
      --json                   Force JSON output
      --session <SESSION>      Stable session identity for daemon membership [env: TELEX_SESSION_ID=]
      --text                   Force concise text output
  -h, --help                   Print help
```

## `telex escalate`

```text
Escalate a message

Usage: telex.exe escalate [OPTIONS] --id <ID>

Options:
      --backend <BACKEND>      Configured backend to use, by name (default: the configured default backend) [env: TELEX_BACKEND=]
      --id <ID>                Message id to disposition
      --db <DB>                Override the SQLite path for this invocation (sqlite backends only) [env: TELEX_DB=]
      --note <NOTE>            Optional note recorded with the disposition
      --address <ADDRESS>      Address to operate on (default for commands that act on one address) [env: TELEX_ADDRESS=]
      --recipient <RECIPIENT>  Recipient address whose disposition this is (defaults to the message's to_addr)
      --json                   Force JSON output
      --session <SESSION>      Stable session identity for daemon membership [env: TELEX_SESSION_ID=]
      --text                   Force concise text output
  -h, --help                   Print help
```

## `telex address`

```text
Address directory operations

Usage: telex.exe address [OPTIONS] <COMMAND>

Commands:
  list    List addresses with description, occupancy, and liveness
  show    Show detail for one address (uses --address)
  retire  Retire an address (drops from normal listings)
  help    Print this message or the help of the given subcommand(s)

Options:
      --backend <BACKEND>  Configured backend to use, by name (default: the configured default backend) [env: TELEX_BACKEND=]
      --db <DB>            Override the SQLite path for this invocation (sqlite backends only) [env: TELEX_DB=]
      --address <ADDRESS>  Address to operate on (default for commands that act on one address) [env: TELEX_ADDRESS=]
      --json               Force JSON output
      --text               Force concise text output
  -h, --help               Print help
```

## `telex address list`

```text
List addresses with description, occupancy, and liveness

Usage: telex.exe address list [OPTIONS]

Options:
      --backend <BACKEND>  Configured backend to use, by name (default: the configured default backend) [env: TELEX_BACKEND=]
      --scope <SCOPE>      Limit to addresses in this scope
      --db <DB>            Override the SQLite path for this invocation (sqlite backends only) [env: TELEX_DB=]
      --match <MATCH>      Substring match against address or description
      --address <ADDRESS>  Address to operate on (default for commands that act on one address) [env: TELEX_ADDRESS=]
      --tag <TAG>          Match a tag (substring of the tags field)
      --all                Include retired addresses
      --json               Force JSON output
      --text               Force concise text output
  -h, --help               Print help
```

## `telex address show`

```text
Show detail for one address (uses --address)

Usage: telex.exe address show [OPTIONS]

Options:
      --backend <BACKEND>  Configured backend to use, by name (default: the configured default backend) [env: TELEX_BACKEND=]
      --db <DB>            Override the SQLite path for this invocation (sqlite backends only) [env: TELEX_DB=]
      --address <ADDRESS>  Address to operate on (default for commands that act on one address) [env: TELEX_ADDRESS=]
      --json               Force JSON output
      --text               Force concise text output
  -h, --help               Print help
```

## `telex address retire`

```text
Retire an address (drops from normal listings)

Usage: telex.exe address retire [OPTIONS]

Options:
      --backend <BACKEND>  Configured backend to use, by name (default: the configured default backend) [env: TELEX_BACKEND=]
      --db <DB>            Override the SQLite path for this invocation (sqlite backends only) [env: TELEX_DB=]
      --address <ADDRESS>  Address to operate on (default for commands that act on one address) [env: TELEX_ADDRESS=]
      --json               Force JSON output
      --text               Force concise text output
  -h, --help               Print help
```

## `telex resolve`

```text
Resolve target address(es) by description match or tag

Usage: telex.exe resolve [OPTIONS]

Options:
      --backend <BACKEND>  Configured backend to use, by name (default: the configured default backend) [env: TELEX_BACKEND=]
      --match <MATCH>      Substring to match against address or description
      --db <DB>            Override the SQLite path for this invocation (sqlite backends only) [env: TELEX_DB=]
      --tag <TAG>          Tag to match
      --address <ADDRESS>  Address to operate on (default for commands that act on one address) [env: TELEX_ADDRESS=]
      --scope <SCOPE>      Limit to a scope
      --json               Force JSON output
      --text               Force concise text output
  -h, --help               Print help
```

## `telex backend`

```text
Manage configured backends (named profiles in ~/.telex/config.toml)

Usage: telex.exe backend [OPTIONS] <COMMAND>

Commands:
  add      Add (or update) a named backend
  list     List configured backends
  show     Show one backend's configuration (secrets redacted)
  remove   Remove a configured backend
  default  Set the default backend
  kinds    List the backend kinds compiled into this build
  help     Print this message or the help of the given subcommand(s)

Options:
      --backend <BACKEND>  Configured backend to use, by name (default: the configured default backend) [env: TELEX_BACKEND=]
      --db <DB>            Override the SQLite path for this invocation (sqlite backends only) [env: TELEX_DB=]
      --address <ADDRESS>  Address to operate on (default for commands that act on one address) [env: TELEX_ADDRESS=]
      --json               Force JSON output
      --text               Force concise text output
  -h, --help               Print help
```

## `telex backend add`

```text
Add (or update) a named backend

Usage: telex.exe backend add [OPTIONS] <NAME>

Arguments:
  <NAME>  Name (key) for this backend

Options:
      --backend <BACKEND>
          Configured backend to use, by name (default: the configured default backend) [env: TELEX_BACKEND=]
      --sqlite
          Configure a SQLite backend (path defaults to ~/.telex/telex.db)
      --db <DB>
          Override the SQLite path for this invocation (sqlite backends only) [env: TELEX_DB=]
      --postgres <CONN>
          Configure a Postgres backend from this connection string (libpq URI or key=value DSN)
      --address <ADDRESS>
          Address to operate on (default for commands that act on one address) [env: TELEX_ADDRESS=]
      --path <PATH>
          SQLite file path (with --sqlite)
      --json
          Force JSON output
      --schema <SCHEMA>
          Postgres schema to isolate telex tables in
      --password-env <PASSWORD_ENV>
          Read the Postgres password from this environment variable
      --text
          Force concise text output
      --password-command <PASSWORD_COMMAND>
          Obtain the Postgres password by running this shell command (its stdout)
      --entra
          Use Microsoft Entra auth for Postgres (token fetched via the Azure SDK)
      --entra-cred <MODE>
          Entra credential mode: auto (dev/CLI login), cli, or managed (devbox/VM identity)
      --entra-scope <ENTRA_SCOPE>
          Override the Entra token scope
      --default
          Make this the default backend
  -h, --help
          Print help
```

## `telex backend list`

```text
List configured backends

Usage: telex.exe backend list [OPTIONS]

Options:
      --backend <BACKEND>  Configured backend to use, by name (default: the configured default backend) [env: TELEX_BACKEND=]
      --db <DB>            Override the SQLite path for this invocation (sqlite backends only) [env: TELEX_DB=]
      --address <ADDRESS>  Address to operate on (default for commands that act on one address) [env: TELEX_ADDRESS=]
      --json               Force JSON output
      --text               Force concise text output
  -h, --help               Print help
```

## `telex backend show`

```text
Show one backend's configuration (secrets redacted)

Usage: telex.exe backend show [OPTIONS] <NAME>

Arguments:
  <NAME>  Backend name

Options:
      --backend <BACKEND>  Configured backend to use, by name (default: the configured default backend) [env: TELEX_BACKEND=]
      --db <DB>            Override the SQLite path for this invocation (sqlite backends only) [env: TELEX_DB=]
      --address <ADDRESS>  Address to operate on (default for commands that act on one address) [env: TELEX_ADDRESS=]
      --json               Force JSON output
      --text               Force concise text output
  -h, --help               Print help
```

## `telex backend remove`

```text
Remove a configured backend

Usage: telex.exe backend remove [OPTIONS] <NAME>

Arguments:
  <NAME>  Backend name

Options:
      --backend <BACKEND>  Configured backend to use, by name (default: the configured default backend) [env: TELEX_BACKEND=]
      --db <DB>            Override the SQLite path for this invocation (sqlite backends only) [env: TELEX_DB=]
      --address <ADDRESS>  Address to operate on (default for commands that act on one address) [env: TELEX_ADDRESS=]
      --json               Force JSON output
      --text               Force concise text output
  -h, --help               Print help
```

## `telex backend default`

```text
Set the default backend

Usage: telex.exe backend default [OPTIONS] <NAME>

Arguments:
  <NAME>  Backend name

Options:
      --backend <BACKEND>  Configured backend to use, by name (default: the configured default backend) [env: TELEX_BACKEND=]
      --db <DB>            Override the SQLite path for this invocation (sqlite backends only) [env: TELEX_DB=]
      --address <ADDRESS>  Address to operate on (default for commands that act on one address) [env: TELEX_ADDRESS=]
      --json               Force JSON output
      --text               Force concise text output
  -h, --help               Print help
```

## `telex backend kinds`

```text
List the backend kinds compiled into this build

Usage: telex.exe backend kinds [OPTIONS]

Options:
      --backend <BACKEND>  Configured backend to use, by name (default: the configured default backend) [env: TELEX_BACKEND=]
      --db <DB>            Override the SQLite path for this invocation (sqlite backends only) [env: TELEX_DB=]
      --address <ADDRESS>  Address to operate on (default for commands that act on one address) [env: TELEX_ADDRESS=]
      --json               Force JSON output
      --text               Force concise text output
  -h, --help               Print help
```

## `telex export`

```text
Export messages and disposition history as JSON lines

Usage: telex.exe export [OPTIONS]

Options:
      --address <ADDRESS>  Limit to messages to/from this address (defaults to the global --address)
      --backend <BACKEND>  Configured backend to use, by name (default: the configured default backend) [env: TELEX_BACKEND=]
      --db <DB>            Override the SQLite path for this invocation (sqlite backends only) [env: TELEX_DB=]
      --thread <THREAD>    Limit to a thread id
      --since <SINCE>      Only messages with id greater than this [default: 0]
      --json               Force JSON output
      --text               Force concise text output
  -h, --help               Print help
```

## Advanced commands

The `telex copilot` and `telex daemon` command families serve the plugin
adapter and operators, and are omitted from the list above. Document them
from the binary with `telex copilot --help` and `telex daemon --help`.

