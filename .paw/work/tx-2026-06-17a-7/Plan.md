# Plan — Issue #7: `--body-file` for `telex send` / `reply`

## Outcome anchor
Both `telex send` and `telex reply` accept `--body-file <path>` to source the message body
from a UTF-8 file (or stdin via `-`), mutually exclusive with `--body`, exactly one required.
SKILL.md documents and recommends it. Tests cover inline / file / both / neither / multiline.
Satisfying this anchor → PR uses `Closes #7`.

## Approach summary
Introduce one shared helper `resolve_body(body, body_file) -> Result<String>` that both `send`
and `reply` call. Relax the two clap args (`body: String` → `Option<String>`) and add
`body_file: Option<String>`. Validate mutual-exclusion / required-one **manually inside the
helper** (issue allows clap groups *or* manual validation; manual gives clearer errors and makes
all four argument cases unit-testable without invoking the parser).

## Key decisions (low spread — auto-proceeding; flagged in field report)
1. **Manual validation, not clap `ArgGroup`.** Clearer error text; single source of truth;
   helper unit-testable for both/neither cases. Issue explicitly permits this.
2. **Implement `--body-file -` (stdin).** Issue marks it "optional but useful"; trivial and
   matches `gh` ergonomics. Documented in SKILL.md.
3. **Exact content preservation.** `std::fs::read_to_string` (UTF-8 decode, errors on invalid
   UTF-8); **no trimming** — trailing newlines preserved. Matches issue recommendation.
4. **Helper location `src/commands/mod.rs`.** Shared by send + reply; inline `#[cfg(test)]`
   tests per existing repo convention (model.rs/skill.rs). Temp files via `std::env::temp_dir()`
   — no new dev-dependency (repo has none).

## Work items
- **W1 `cli.rs` args**: `SendArgs`/`ReplyArgs`: `body: Option<String>` + new
  `body_file: Option<String>` with `--body-file` doc comment.
- **W2 `commands/mod.rs` helper + tests**: `resolve_body`; `#[cfg(test)]` covering inline, file,
  both→err, neither→err, multiline-exact, stdin path note.
- **W3 `send.rs` / `reply.rs`**: replace `args.body.clone()` with
  `resolve_body(args.body.clone(), args.body_file.clone())?`.
- **W4 `SKILL.md`**: quick send/reply examples, useful send flags, optional reply flags, SEND
  command-reference table rows; recommend `--body-file` for multiline/structured messages.

(Single sequential implementation — work items are tightly coupled in 4 files; no fleet dispatch.)

## Acceptance criteria mapping
- send/reply `--body-file` sends exact UTF-8 content → W1+W2+W3, smoke test.
- `--body` still works → W1+W3 (inline branch).
- both flags → clear error → W2 (manual validation).
- neither flag → clear error → W2.
- `--body-file -` stdin → implemented; documented → W2+W4.
- tests inline/file/both/neither/multiline → W2.
- SKILL.md documents & recommends → W4.

## Verification
`cargo build`, `cargo test`, `cargo clippy`, `cargo fmt --check`; manual smoke test sending a
multiline file body and asserting the round-tripped body via `telex read`.
