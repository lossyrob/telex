# Detector template library changelog

## 2026-07-29 cycle-2 quick wins

- `local-file-json` template version 1.1.1, detector protocol 1, evidence
  normalization 2.
- `local-file-json` now fails closed with a configuration diagnostic when
  `expectedValue` is omitted, matching `http-json`.
- Added direct conformance coverage for external-activity GitHub PR preflight
  terminal and identity-mismatch handling.

## 2026-07-29 review fixes

- `github-pr` template version 1.1.0, detector protocol 1, evidence
  normalization 2.
- `github-pr-external-activity` template version 1.1.0, detector protocol 1,
  evidence normalization 2.
- `azure-devops-pr` template version 1.1.0, detector protocol 1, evidence
  normalization 2.
- `http-json` template version 1.1.0, detector protocol 1, evidence
  normalization 2.
- `local-file-json` template version 1.1.0, detector protocol 1, evidence
  normalization 2.
- `local-command` template version 1.1.0, detector protocol 1, evidence
  normalization 2.
- First-attempt actionable events now emit independently of synthetic
  snapshot/created opt-ins. Added canonical cursor hashing, structured
  diagnostics, network-free provider transport tests, strict scalar/null
  matching, portable samples, fixture API metadata, and bounded PowerShell
  local-command execution without per-attempt C# compilation.

## 2026-07-29

- Manifest schema version 1.
- `github-pr` template version 1.0.0, detector protocol 1, evidence
  normalization 1.
- `github-pr-external-activity` template version 1.0.0, detector protocol 1,
  evidence normalization 1.
- `azure-devops-pr` template version 1.0.0, detector protocol 1, evidence
  normalization 1.
- `http-json` template version 1.0.0, detector protocol 1, evidence
  normalization 1.
- `local-file-json` template version 1.0.0, detector protocol 1, evidence
  normalization 1.
- `local-command` template version 1.0.0, detector protocol 1, evidence
  normalization 1.
- Added strict manifests, pinned/development registration samples, sanitized
  fixtures, source/helper digests, PR terminal preflight, and executable
  conformance coverage.
