# Detector template library changelog

## 2026-07-29 recurring event identity and bounded GitHub activity

- `github-pr` template version 1.2.0, detector protocol 1, evidence
  normalization 3.
- `github-pr-external-activity` template version 1.2.0, detector protocol 1,
  evidence normalization 3.
- `azure-devops-pr` template version 1.2.0, detector protocol 1, evidence
  normalization 3.
- `http-json` template version 1.2.0, detector protocol 1, evidence
  normalization 2.
- `local-file-json` template version 1.2.0, detector protocol 1, evidence
  normalization 2.
- `local-command` template version 1.2.0, detector protocol 1, evidence
  normalization 2.
- Added committed-state-derived occurrence discriminators to all event IDs.
  Retries before commit retain IDs, committed A -> B -> A cycles receive a new
  ID for the second A, unchanged replay remains idle, and changed idle
  observations explicitly consume an occurrence.
- Replaced ineffective ordered-dictionary sorting in both GitHub templates
  with ordinal total comparators and deterministic tie-breakers.
- Replaced unbounded external-activity metadata with counts, a bounded PR URL,
  capped body-free projections, and explicit truncation flags. Complete
  normalized review/comment activity is represented by compact hashes in
  cursor evidence.

## 2026-07-29 cross-platform conformance

- `azure-devops-pr` template version 1.1.1, detector protocol 1, evidence
  normalization 3.
- Normalized provider timestamps without local-time conversion, made canonical
  cursor serialization and evidence ordering culture-independent, and pinned
  detector product files to LF bytes on every Git checkout.
- Made local-command helper resolution platform-neutral and strengthened
  conformance coverage for timezone, culture, line-ending, and argv execution
  assumptions.

## 2026-07-29 guide placement

- Moved the concise agent checklist from the reserved `SKILL.md` filename to
  [AGENT.md](AGENT.md), with the template README remaining authoritative.

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
