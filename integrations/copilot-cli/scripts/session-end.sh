#!/usr/bin/env bash
# Telex Copilot CLI plugin: sessionEnd hook (bash).
#
# Invoked by Copilot CLI when a session ends (dismiss OR quit). Receives a JSON payload
# on stdin including the sessionId. We look up the telex stations this session owns
# (one file per station, written by `telex attach`) and `telex detach` each, so a
# detached background holder never orphans past its session.
#
# Registry layout:
#   ${TELEX_SESSION_DIR:-$HOME/.telex/sessions}/<sessionId>/<sanitized-address>.json
#   { "address": "...", "telex": "<binary path>", "env": { "TELEX_DB": "...", ... } }
#
# Hooks must never fail noisily: we always exit 0.

set -u

hook_log() {
  local msg="$1"
  local log="${TELEX_HOOK_LOG:-$HOME/.telex/logs/session-end-hook.log}"
  mkdir -p "$(dirname "$log")" 2>/dev/null || true
  printf '%s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%S.000Z)" "$msg" >>"$log" 2>/dev/null || true
}

raw="$(cat)"
if [ -z "${raw//[[:space:]]/}" ]; then hook_log "sessionEnd: empty stdin; exiting"; exit 0; fi

# Parse {sessionId, endReason} from the payload.
parsed="$(printf '%s' "$raw" | python3 -c '
import sys, json
try:
    d = json.load(sys.stdin)
except Exception:
    sys.exit(0)
sid = d.get("sessionId") or (d.get("data") or {}).get("sessionId") or ""
reason = d.get("endReason") or d.get("reason") or (d.get("data") or {}).get("reason") or "unknown"
print("%s\t%s" % (sid, reason))
' 2>/dev/null)"

session_id="${parsed%%$'\t'*}"
reason="${parsed#*$'\t'}"

if [ -z "$session_id" ]; then hook_log "sessionEnd: no sessionId in payload; exiting"; exit 0; fi
case "$session_id" in *[!a-zA-Z0-9_-]*) hook_log "sessionEnd: invalid sessionId; exiting"; exit 0;; esac

hook_log "sessionEnd: sessionId=$session_id reason=$reason"

reg_dir="${TELEX_SESSION_DIR:-$HOME/.telex/sessions}"
session_dir="$reg_dir/$session_id"
if [ ! -d "$session_dir" ]; then hook_log "sessionEnd: no station registry for $session_id; nothing to detach"; exit 0; fi

count=0
for f in "$session_dir"/*.json; do
  [ -e "$f" ] || continue
  count=$((count + 1))
done
hook_log "sessionEnd: $count station(s) registered for $session_id"

for f in "$session_dir"/*.json; do
  [ -e "$f" ] || continue
  # Emit: address<TAB>telexbin<TAB>KEY=VALUE;KEY=VALUE...
  line="$(python3 -c '
import sys, json
try:
    s = json.load(open(sys.argv[1]))
except Exception:
    sys.exit(0)
addr = s.get("address") or ""
if not addr: sys.exit(0)
binname = s.get("telex") or "telex"
env = s.get("env") or {}
envstr = ";".join("%s=%s" % (k, v) for k, v in env.items())
print("%s\t%s\t%s" % (addr, binname, envstr))
' "$f" 2>/dev/null)"
  [ -z "$line" ] && continue
  IFS=$'\t' read -r addr binname envstr <<<"$line"
  [ -z "$addr" ] && continue
  ( # subshell so per-station env never leaks
    if [ -n "$envstr" ]; then
      IFS=';' read -ra pairs <<<"$envstr"
      for kv in "${pairs[@]}"; do export "$kv"; done
    fi
    out="$("$binname" --address "$addr" detach 2>&1)"
    hook_log "sessionEnd: detached address=$addr exit=$? out=$(printf '%s' "$out" | tr '\n' ' ')"
  )
done

# Drop the whole session directory; each holder also unregisters its own record on clean exit.
rm -rf "$session_dir" 2>/dev/null || true
hook_log "sessionEnd: done for $session_id"
exit 0
