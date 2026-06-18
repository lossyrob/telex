# Telex Copilot CLI plugin: sessionEnd hook (PowerShell).
#
# Invoked by Copilot CLI when a session ends (dismiss OR quit; reason is one of
# complete | error | abort | timeout | user_exit). Receives a JSON payload on stdin
# that includes the sessionId. We look up the telex stations this session owns and
# `telex detach` each, so a detached background holder never orphans past its session.
#
# Station registry (written by `telex attach`; one file per station so concurrent
# attaches in a session never race):
#   <TELEX_SESSION_DIR | $HOME/.telex/sessions>/<sessionId>/<sanitized-address>.json
#   { "address": "station:x",
#     "telex": "<binary path>",
#     "env": { "TELEX_HOME": "...", "TELEX_CONFIG": "...", "TELEX_DB": "...", "TELEX_BACKEND": "..." } }
#
# Hooks must never fail noisily, so everything is wrapped and we always exit 0.

$ErrorActionPreference = 'Stop'

function Write-HookLog([string]$msg) {
  try {
    $logPath = if ($env:TELEX_HOOK_LOG) { $env:TELEX_HOOK_LOG } else { Join-Path $env:USERPROFILE '.telex\logs\session-end-hook.log' }
    $dir = Split-Path -Parent $logPath
    if (-not (Test-Path $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }
    $stamp = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ss.fffZ')
    Add-Content -Path $logPath -Value ("{0} {1}" -f $stamp, $msg) -Encoding utf8
  } catch { }
}

try {
  $raw = [Console]::In.ReadToEnd()
  if (-not $raw -or $raw.Trim().Length -eq 0) { Write-HookLog "sessionEnd: empty stdin; exiting"; exit 0 }

  $data = $raw | ConvertFrom-Json
  $sessionId = $null
  if ($data.sessionId) { $sessionId = $data.sessionId }
  elseif ($data.data -and $data.data.sessionId) { $sessionId = $data.data.sessionId }
  $reason = 'unknown'
  if ($data.endReason) { $reason = $data.endReason }
  elseif ($data.reason) { $reason = $data.reason }
  elseif ($data.data -and $data.data.reason) { $reason = $data.data.reason }

  if (-not $sessionId) { Write-HookLog "sessionEnd: no sessionId in payload; exiting"; exit 0 }
  if ($sessionId -notmatch '^[a-zA-Z0-9_-]+$') { Write-HookLog "sessionEnd: invalid sessionId; exiting"; exit 0 }

  Write-HookLog ("sessionEnd: sessionId={0} reason={1}" -f $sessionId, $reason)

  $regDir = if ($env:TELEX_SESSION_DIR) { $env:TELEX_SESSION_DIR } else { Join-Path $env:USERPROFILE '.telex\sessions' }
  $sessionDir = Join-Path $regDir $sessionId
  if (-not (Test-Path $sessionDir)) { Write-HookLog ("sessionEnd: no station registry for {0}; nothing to detach" -f $sessionId); exit 0 }

  $files = @(Get-ChildItem -Path $sessionDir -Filter '*.json' -File -ErrorAction SilentlyContinue)
  Write-HookLog ("sessionEnd: {0} station(s) registered for {1}" -f $files.Count, $sessionId)

  foreach ($f in $files) {
    try {
      $s = Get-Content $f.FullName -Raw | ConvertFrom-Json
    } catch {
      Write-HookLog ("sessionEnd: skipping unreadable record {0}: {1}" -f $f.Name, $_.Exception.Message)
      continue
    }
    $addr = $s.address
    if (-not $addr) { continue }
    $bin = if ($s.telex) { $s.telex } else { 'telex' }

    # Apply per-station env (isolated/named backends), detach, then restore env.
    $applied = @{}
    if ($s.env) {
      foreach ($p in $s.env.PSObject.Properties) {
        $applied[$p.Name] = (Get-Item -Path ("Env:{0}" -f $p.Name) -ErrorAction SilentlyContinue).Value
        Set-Item -Path ("Env:{0}" -f $p.Name) -Value $p.Value
      }
    }
    try {
      $out = & $bin --address $addr detach 2>&1 | Out-String
      Write-HookLog ("sessionEnd: detached address={0} exit={1} out={2}" -f $addr, $LASTEXITCODE, ($out.Trim() -replace '\s+', ' '))
    } catch {
      Write-HookLog ("sessionEnd: detach FAILED address={0} error={1}" -f $addr, $_.Exception.Message)
    } finally {
      foreach ($k in $applied.Keys) {
        if ($null -eq $applied[$k]) { Remove-Item -Path ("Env:{0}" -f $k) -ErrorAction SilentlyContinue }
        else { Set-Item -Path ("Env:{0}" -f $k) -Value $applied[$k] }
      }
    }
  }

  # Drop the whole session directory; each holder also unregisters its own record on clean exit.
  Remove-Item -Path $sessionDir -Recurse -Force -ErrorAction SilentlyContinue
  Write-HookLog ("sessionEnd: done for {0}" -f $sessionId)
} catch {
  Write-HookLog ("sessionEnd: unhandled error: {0}" -f $_.Exception.Message)
}

exit 0
