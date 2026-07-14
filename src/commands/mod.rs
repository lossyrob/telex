//! Command handlers, one module per verb group.

use anyhow::{anyhow, Result};

pub mod address;
pub mod attach;
pub mod backend;
pub mod copilot;
pub mod daemon;
pub mod detach;
pub mod disposition;
pub mod export;
pub mod inbox;
pub mod init;
pub mod read;
pub mod reply;
pub mod send;
pub mod skill;
pub mod station;
pub mod status;
pub mod upgrade;
pub mod wait;

/// Resolve a message body from the mutually exclusive `--body` (inline), `--body-file`,
/// and `--body-stdin` options. Exactly one must be supplied.
///
/// `--body-stdin` is a convenience alias for `--body-file -`.
/// `--body-file` reads a UTF-8 file, or stdin when the path is `-`. File and stdin content is
/// returned exactly as decoded — no trimming — so trailing newlines and embedded formatting are
/// preserved.
///
/// On Windows/PowerShell, piped stdin must be UTF-8. Before piping non-ASCII content, run:
/// `$OutputEncoding = [System.Text.Encoding]::UTF8`
/// or write to a UTF-8 file and use `--body-file <path>` instead.
pub fn resolve_body(
    body: Option<String>,
    body_file: Option<String>,
    body_stdin: bool,
) -> Result<String> {
    // --body-stdin is a convenience alias for --body-file -
    let body_file = match (body_file, body_stdin) {
        (Some(_), true) => {
            return Err(anyhow!(
                "--body-file and --body-stdin are mutually exclusive; pass exactly one"
            ))
        }
        (None, true) => Some("-".to_string()),
        (file, false) => file,
    };
    match (body, body_file) {
        (Some(_), Some(_)) => Err(anyhow!(
            "--body and --body-file/--body-stdin are mutually exclusive; pass exactly one"
        )),
        (None, None) => Err(anyhow!(
            "one of --body, --body-file, or --body-stdin is required"
        )),
        (Some(inline), None) => Ok(inline),
        (None, Some(path)) => read_body_source(&path),
    }
}

fn read_body_source(path: &str) -> Result<String> {
    if path == "-" {
        read_stdin_utf8()
    } else {
        std::fs::read_to_string(path)
            .map_err(|e| anyhow!("failed to read --body-file {path:?}: {e}"))
    }
}

fn read_stdin_utf8() -> Result<String> {
    use std::io::Read;
    let mut raw = Vec::new();
    std::io::stdin()
        .read_to_end(&mut raw)
        .map_err(|e| anyhow!("failed to read body from stdin: {e}"))?;
    String::from_utf8(raw).map_err(|_| {
        anyhow!(
            "stdin body is not valid UTF-8; \
            on Windows/PowerShell run `$OutputEncoding = [System.Text.Encoding]::UTF8` \
            before piping, or write to a UTF-8 file and use `--body-file <path>`"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::resolve_body;
    use std::io::Write;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn temp_path(label: &str) -> std::path::PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "telex-resolve-body-{}-{label}-{n}.txt",
            std::process::id()
        ))
    }

    fn write_temp(label: &str, contents: &str) -> std::path::PathBuf {
        let path = temp_path(label);
        let mut f = std::fs::File::create(&path).expect("create temp file");
        f.write_all(contents.as_bytes()).expect("write temp file");
        path
    }

    #[test]
    fn inline_body_is_returned_verbatim() {
        let got = resolve_body(Some("hello inline".to_string()), None, false).unwrap();
        assert_eq!(got, "hello inline");
    }

    #[test]
    fn body_file_is_read_as_exact_utf8() {
        let path = write_temp("exact", "from a file");
        let got = resolve_body(None, Some(path.to_string_lossy().into_owned()), false).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(got, "from a file");
    }

    #[test]
    fn multiline_body_file_preserves_content_and_trailing_newline() {
        let body = "# Heading\n\n```json\n{\n  \"k\": \"v\"\n}\n```\n\nDone.\n";
        let path = write_temp("multiline", body);
        let got = resolve_body(None, Some(path.to_string_lossy().into_owned()), false).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(got, body);
    }

    #[test]
    fn both_flags_is_an_error() {
        let err =
            resolve_body(Some("x".to_string()), Some("f.txt".to_string()), false).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"));
    }

    #[test]
    fn neither_flag_is_an_error() {
        let err = resolve_body(None, None, false).unwrap_err();
        assert!(err.to_string().contains("required"));
    }

    #[test]
    fn missing_body_file_reports_a_clear_error() {
        let path = temp_path("missing");
        let err = resolve_body(None, Some(path.to_string_lossy().into_owned()), false).unwrap_err();
        assert!(err.to_string().contains("body-file"));
    }

    #[test]
    fn body_stdin_with_body_file_is_an_error() {
        let err = resolve_body(None, Some("f.txt".to_string()), true).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"));
    }

    #[test]
    fn body_stdin_with_body_is_an_error() {
        let err = resolve_body(Some("x".to_string()), None, true).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"));
    }

    #[test]
    fn body_stdin_false_with_no_file_is_an_error() {
        let err = resolve_body(None, None, false).unwrap_err();
        assert!(
            err.to_string().contains("--body-stdin"),
            "error should mention --body-stdin: {err}"
        );
    }

    #[test]
    fn daemon_one_shot_verbs_do_not_call_legacy_registry_or_address_ipc() {
        let verbs = [
            ("attach", include_str!("attach.rs")),
            ("wait", include_str!("wait.rs")),
            ("send", include_str!("send.rs")),
            ("reply", include_str!("reply.rs")),
            ("ack", include_str!("disposition.rs")),
        ];
        for (verb, source) in verbs {
            assert!(
                !source.contains("crate::registry")
                    && !source.contains("registry::")
                    && !source.contains("crate::ipc")
                    && !source.contains("ipc::endpoint"),
                "{verb} must use daemon-scoped IPC, not the legacy holder registry/address endpoint"
            );
        }
    }
}
