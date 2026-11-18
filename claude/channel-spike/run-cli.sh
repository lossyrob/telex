#!/usr/bin/env bash
# Launch Claude Code with the Telex channel spike loaded as a development channel.
#   CLAUDE_BIN=/path/to/claude ./run-cli.sh [extra claude args]
#   TELEX_CHANNEL_HELLO=1 pushes a self-test message after connect.
# Channels need Claude Code >= 2.1.80; the PATH `claude` may be older than the Desktop-bundled one.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
cfg="$(mktemp -t telex-channel-mcp).json"
node -e '
const env = { TELEX_CHANNEL_ID: process.env.TELEX_CHANNEL_ID || "dev" };
if (process.env.TELEX_CHANNEL_HELLO) env.TELEX_CHANNEL_HELLO = "1";
console.log(JSON.stringify({ mcpServers: { telex: { command: "node", args: [process.argv[1] + "/channel.mjs"], env } } }, null, 2));
' "$here" > "$cfg"
exec "${CLAUDE_BIN:-claude}" --mcp-config "$cfg" --strict-mcp-config \
  --dangerously-load-development-channels server:telex "$@"
