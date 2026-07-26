## Watcher domain contract merged

The Watcher production domain contract from
[#110](https://github.com/lossyrob/telex/issues/110) merged in
[PR #115](https://github.com/lossyrob/telex/pull/115).

Authoritative domain sources:

- [`docs/design/watcher.md`](https://github.com/lossyrob/telex/blob/main/docs/design/watcher.md)
- ADR 0046
- canonical detector request/result, event metadata, and health schemas
- Watcher requirements r2:
  https://github.com/lossyrob/telex/issues/12#issuecomment-5042702401

The Telex Watcher workstream is now explicitly waiting. Neither
`watcher-runtime` nor `detector-template-library` will be promoted until this
issue/campaign records accepted/deferred/rejected dispositions for the shared
requirements and publishes `application-client-ready`.

No spike-private CLI, environment, raw IPC, or sender-occupancy seam is an
allowed production fallback.
