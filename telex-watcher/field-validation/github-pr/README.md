# V2 GitHub PR detector

This trusted-local Python example observes one PR and returns a Watcher v2
result. It never merges, approves, comments, changes labels, or launches
follow-up work. Provider logic stays in this example, outside the Watcher core.
Python 3.10 or later is required; there are no third-party Python dependencies.

## Start with committed fixtures

From the repository root, run:

```powershell
Get-Content -Raw telex-watcher\field-validation\github-pr\request.json |
  python -B telex-watcher\field-validation\github-pr\github_pr_detector.py
python -B -m unittest discover -s telex-watcher\field-validation\github-pr -p "test_*.py"
cargo test -p telex-watcher --test protocol_v2
```

Use `py -3` instead of `python` if that is how Python is installed. Fixture mode
does not invoke `gh`, use credentials, or contact GitHub. Relative fixture paths
resolve beside the detector, not against the runtime's working directory.
The committed fixtures cover an open baseline, changed reviews/comments/checks/
merge status, closed PR, and merged PR.

The first open observation returns `idle` with a baseline `nextState`. Pass that
state into the next request's `state`, then change `fixture` to
`fixtures/pr-changed.json` to produce an event. Passing the returned state back
against the same fixture returns `idle` without `nextState`. Set
`emitInitialSnapshot: true` to emit an initial open-PR snapshot instead.
A closed or merged PR always produces a terminal event, including on the first
observation.

`registration.json` is a Windows-shaped, fixture-only registration example.
Replace the backend and fixed sender/target with your own values before
registering. Run registration from the repository root so its omitted
`workingDirectory` captures that directory, or use an explicit canonical
absolute directory and absolute script path. On other platforms, replace the
command's path separators or use an absolute native path. The command is argv,
not a shell expression.

## Live observation is opt-in

Remove `fixture` from `watch.parameters` (or registration `parameters`) and set
`repo` and `pullRequest` to the intended PR. The only provider command is:

```text
gh pr view <number> --repo <owner/name> --json <fixed field list>
```

It runs through `subprocess.Popen` with an argv list and `shell=False`.
Authentication belongs to the locally configured `gh` profile. If needed,
explicitly allowlist `GH_CONFIG_DIR`, `GH_TOKEN`, or other required credential
environment names in your registration; do not put credential values into
parameters or fixtures. No live GitHub access is needed for the tests.

Parameters:

| Name | Contract |
|---|---|
| `repo` | Required `owner/name`; bounded, validated before command construction. |
| `pullRequest` | Required integer, 1 through 2147483647. |
| `fixture` | Optional trusted-local file path; presence disables live queries. |
| `emitInitialSnapshot` | Optional boolean; default `false`. |
| `ghTimeoutSeconds` | Optional integer, 1 through 20; default 15. |

The live query has a 1 MiB stdout cap, 64 KiB stderr cap, and bounded timeout.
Both streams are drained concurrently into bounded buffers. The example
registration gives the whole detector 30 seconds; keep the outer Watcher
timeout larger than the provider timeout. Watcher remains responsible for
containing the entire process tree, including any provider descendants.
The detector reads at most 768 KiB of request input and emits at most 256 KiB
of result output. Failure emits only `degraded`, no state or partial event, and
a fixed short diagnostic that does not echo provider stderr.

## What it observes, and what it does not

The cursor stores a compact snapshot of PR state, head, draft flag, title,
review decision/reviews, issue comments, check rollup, and mergeability/merge
status. Array ordering alone does not create an event. Edits and deletions
change the snapshot as well as additions. Provider bodies are hashed rather
than copied into the cursor or event body.

Each changed snapshot produces one summary event listing changed categories.
Closed and merged PRs produce `terminal`. The detector neither interprets a
green check as permission to merge nor acts on review instructions.

**Inline review threads are excluded.** The `reviews` collection describes
submitted review records, not every inline comment, reply, resolution, or
thread state. The `comments` collection covers PR issue comments. This is a
polling snapshot detector, not a complete GitHub activity log: intermediate
changes that revert between polls may be missed, and a batch of changes is
summarized rather than replayed individually.

`gh pr view` uses bounded connections. Any reviews/comments/checks collection
with 100 or more entries is treated as potentially truncated and degrades
without advancing the cursor. There is no hidden pagination or unbounded
history fetch. A PR needing larger collections needs a separate detector with
an explicit bounded pagination/cursor policy. Changes in `gh` response shape
may also require updating this example. An incompatible or foreign cursor
fails closed instead of silently resetting its baseline.

## Core protocol decisions

The authority is [the Watcher contract](../../../docs/design/watcher.md) and its
three v2 schemas. The `protocol_v2` module provides concrete strict envelopes,
while opaque parameters/state and object metadata remain provider-owned.

* Registration intervals are **1..86400 seconds** and timeouts are
  **1..3600 seconds**, inclusive. These are product bounds, not provider policy.
* Backend profile names are nonempty, control/whitespace-free strings of at
  most 256 UTF-8 bytes. Sender and target use the same lexical rule with a
  1024-byte cap. Existence of the configured backend is checked by integration,
  not by reading shared configuration during protocol validation.
* Omitted working directories are captured from the supplied absolute
  registration cwd and canonicalized. Explicit directories must already be
  canonical, absolute, existing directories. Windows ordinary and
  extended-length spellings are accepted for the same canonical path; the
  persisted `PathBuf` uses the filesystem's canonical spelling.
* Duplicate keys are rejected recursively before conversion into a JSON value,
  including duplicates spelled with equivalent Unicode escapes. Invalid
  Unicode, lone surrogates, Unicode noncharacters, number overflow, and nonzero
  numeric underflow are rejected. Integral JSON tokens must be within
  **-9007199254740991..9007199254740991**; encode larger IDs as strings.
  Finite floating-point JSON uses binary64 with correctly rounded parsing.
* Persistent hashes are `sha256:<lowercase hex>` over RFC 8785 bytes from
  `serde_json_canonicalizer` 0.3, including UTF-16 property ordering and
  ECMAScript number formatting. Strings are not Unicode-normalized.
* Request timestamps require calendar-valid UTC, uppercase `T`/`Z`, and
  zero through three fractional digits. Leap-second `:60`, offsets, and finer
  precision are rejected. The timestamp helper emits milliseconds.
* Explicit `nextState: null` is `Some(Value::Null)`; omitted `nextState` is
  `None`. Serialization preserves that distinction. Explicit `event: null`
  is invalid. Event metadata, when omitted, defaults to `{}`.
* Public byte parsers and `validate_*` functions enforce the full contract.
  Serde models reject unknown envelope fields; callers constructing models in
  Rust must validate before execution, send, or persistence. Error variants
  distinguish malformed envelopes, canonicalization failures, and output
  limits. Bounds use UTF-8 bytes, not character counts.

No live-provider or runtime-daemon behavior is asserted by the fixture suite.
The tests verify the versioned boundary and read-only detector behavior.

### Executable schema conformance

`test_schema_conformance.py` requires Python's `jsonschema` package. It registers
an explicit RFC 3339 lexical and standard-library calendar checker, so optional
`jsonschema` format dependencies are not required and formats cannot silently
become annotations. It checks all Watcher v2 schemas against the Draft
2020-12 metaschema and exercises the registration/request/result schemas with
calendar-format validation enabled:

```powershell
python -B telex-watcher\field-validation\github-pr\test_schema_conformance.py
cargo test -p telex-watcher --test protocol_v2
```

Both runners consume `protocol-schema-cases.json`. Each case applies documented
JSON Pointer replacements/removals to a concrete template; `schemaValid` records
the schema result and `protocolValid` records the concrete Rust model result.
These expectations differ where the contract explicitly adds runtime checks,
including absolute directories, byte/numeric limits, route syntax, and product
timing bounds. Schema validation alone cannot reject duplicate JSON members
after parsing; the Rust parser tests cover that boundary separately.

The Python runner also validates actual fixture-detector results and checks
that an invalid leap-day timestamp fails specifically on `format`.
`load_validator(schema_name)` is available for additional health/metadata
instance tests; it validates the schema and always enables date-time checking.
