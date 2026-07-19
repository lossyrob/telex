# Captured Telex CLI fixtures

These sanitized fixtures represent the Telex 0.1.0 response shapes consumed by
the experimental Station adapter. Required fields are validated; additive
unknown fields are intentionally tolerated.

- `version.json`
- `attach.json`
- `wait-delivery.json`
- `read-full.json`
- `inbox.json`
- `export.jsonl`
- `ack.json`
- `disposition.json`
- `reply.json`
- `status.json`
- `station-status.json`
- `station-stop.json`

They are spike evidence, not a frozen public CLI JSON contract.
The source fingerprint in `read-full.json` is illustrative; callers must
substitute the active store fingerprint before asserting that a source is
available.
