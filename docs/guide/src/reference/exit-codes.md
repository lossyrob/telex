# Exit codes

`telex wait` uses distinct exit codes so a caller can react without parsing
output. When a waiter writes `--out-dir`, the integer code is also written to
`exit.code` (last, as the completion marker); trust that artifact rather than a
detached task's reported exit code.

| Exit | Meaning | What to do |
|---:|---|---|
| 0 | Delivered | Read `delivery.json` (or `message.json`), `ack` and dedupe by id, then re-arm a fresh `wait` before longer processing. |
| 1 | Error | Unexpected failure. Inspect stderr and `status.json`. |
| 2 | Idle timeout | Nothing arrived before `--timeout-ms`. Re-arm if still attending. |
| 3 | Daemon gone / not running | Run `telex attach` (the spawning and recovery verb), then re-arm. |
| 4 | Daemon hung / no response after the `--timeout-ms + --hang-ms` watchdog | Re-arm, or restart the daemon if it repeats. |
| 5 | Presence ended | Non-destructive reap. A live session should `attach` and `wait` again. |

`wait` does not spawn a missing daemon; that is what `attach` is for. If a
replacement daemon already exists, a `wait` can reconnect and re-register during
its bounded reconnect grace.

For the authoritative flags of any command, run its `--help` (see the
[CLI reference](cli.md)). The hidden `telex copilot` and `telex daemon` families
are documented by `telex copilot --help` and `telex daemon --help`.
