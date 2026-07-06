#!/usr/bin/env bash
# Generate the telex CLI reference (src/reference/cli.md) from the installed
# binary's --help output.
#
# Do NOT edit the generated file by hand. Run this script instead. CI regenerates
# it on every docs build so the reference stays matched to the binary and never
# drifts (see docs/design/DECISIONS.md ADR 0040 for the single-source principle).
#
# Usage: generate-reference.sh [path-to-telex-binary] [output-file]
set -euo pipefail

TELEX="${1:-${TELEX_BIN:-telex}}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
OUT="${2:-$SCRIPT_DIR/src/reference/cli.md}"

version="$("$TELEX" --version 2>/dev/null || echo telex)"

emit_help() {
  # Args: subcommand path (empty for the top level).
  local title="telex${*:+ $*}"
  printf '## `%s`\n\n' "$title"
  printf '```text\n'
  "$TELEX" "$@" --help </dev/null 2>&1 | tr -d '\r'
  printf '```\n\n'
}

subcommands() {
  # Print child subcommand names for the given command path (empty = top level).
  "$TELEX" "$@" --help </dev/null 2>/dev/null | tr -d '\r' | awk '
    /^Commands:/ { inc = 1; next }
    inc && (/^[A-Za-z]/ || NF == 0) { inc = 0 }
    inc { print $1 }
  ' | grep -vE '^help$' || true
}

{
  printf '# CLI reference\n\n'
  printf '> This page is generated from the installed `telex` binary (`%s`) by\n' "$version"
  printf '> `docs/guide/generate-reference.sh`. Do not edit it by hand; it is\n'
  printf '> regenerated on every docs build so it stays matched to the binary.\n'
  printf '> For the workflow narrative, see the [Guides](../guides/agent-pull.md).\n\n'

  emit_help

  # Read command lists into arrays up front so no pipe/fd stays open while the
  # binary runs (a Windows binary under WSL interop can otherwise drain stdin).
  mapfile -t cmds < <(subcommands)
  for cmd in "${cmds[@]}"; do
    [ -z "$cmd" ] && continue
    emit_help "$cmd"
    mapfile -t subs < <(subcommands "$cmd")
    for sub in "${subs[@]}"; do
      [ -z "$sub" ] && continue
      emit_help "$cmd" "$sub"
    done
  done
} > "$OUT"

echo "wrote $OUT"
