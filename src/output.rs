//! Output shaping. Telex defaults to JSON when stdout is not a TTY (agents) and to
//! concise text when interactive (humans). `--json`/`--text` override the default.

use serde::Serialize;
use std::io::IsTerminal;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Json,
    Text,
}

impl Format {
    pub fn resolve(json: bool, text: bool) -> Format {
        if json {
            Format::Json
        } else if text || std::io::stdout().is_terminal() {
            Format::Text
        } else {
            Format::Json
        }
    }
}

/// Print a serializable value as pretty JSON.
pub fn print_json<T: Serialize>(value: &T) {
    match serde_json::to_string_pretty(value) {
        Ok(s) => println!("{s}"),
        Err(e) => eprintln!("[telex] failed to serialize output: {e}"),
    }
}

/// Print one compact JSON object per line (for streaming / jsonl).
pub fn print_jsonl<T: Serialize>(value: &T) {
    match serde_json::to_string(value) {
        Ok(s) => println!("{s}"),
        Err(e) => eprintln!("[telex] failed to serialize output: {e}"),
    }
}

/// Emit either JSON (pretty) or a text closure depending on the chosen format.
pub fn emit<T: Serialize>(fmt: Format, value: &T, text: impl FnOnce()) {
    match fmt {
        Format::Json => print_json(value),
        Format::Text => text(),
    }
}
