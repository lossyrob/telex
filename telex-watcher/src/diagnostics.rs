use std::error::Error;

const MAX_BYTES: usize = 4096;
const REDACTED: &str = "[REDACTED]";
const TRUNCATED: &str = "\n[truncated]\n";

pub(super) fn runtime_failure(error: &anyhow::Error) -> String {
    let values: Vec<_> = std::env::vars_os()
        .map(|(_, value)| value.to_string_lossy().into_owned())
        .filter(|value| !value.is_empty())
        .collect();
    render(error, &values)
}

fn render(error: &anyhow::Error, values: &[String]) -> String {
    let mut output = String::from("runtime-failed");
    for cause in error.chain() {
        output.push_str("\n  caused by: ");
        let text = safe_cause(cause);
        let lower = text.to_ascii_lowercase();
        // Unstructured credentials and DSNs have no reliable value boundary.
        // Omit that cause rather than risk printing part of a quoted secret.
        if text.contains(['=', '@'])
            || text.contains("://")
            || [
                "token",
                "password",
                "secret",
                "credential",
                "authorization",
                "bearer",
                "api_key",
                "api-key",
                "key:",
                "\"key\"",
                "pat:",
                "\"pat\"",
                "private key",
                "dsn",
                "ghp_",
                "github_pat_",
                "sk-",
            ]
            .iter()
            .any(|word| lower.contains(word))
        {
            output.push_str(REDACTED);
        } else {
            let mut rest = text.as_str();
            while !rest.is_empty() {
                if let Some(value) = values
                    .iter()
                    .filter(|value| rest.starts_with(value.as_str()))
                    .max_by_key(|value| value.len())
                {
                    output.push_str(REDACTED);
                    rest = &rest[value.len()..];
                } else {
                    let ch = rest.chars().next().expect("nonempty cause");
                    if ch.is_control() {
                        output.extend(ch.escape_default());
                    } else {
                        output.push(ch);
                    }
                    rest = &rest[ch.len_utf8()..];
                }
            }
        }
    }
    output.push('\n');
    if output.len() > MAX_BYTES {
        let mut end = MAX_BYTES - TRUNCATED.len();
        while !output.is_char_boundary(end) {
            end -= 1;
        }
        output.truncate(end);
        output.push_str(TRUNCATED);
    }
    output
}

fn safe_cause(cause: &(dyn Error + 'static)) -> String {
    if let Some(error) = cause.downcast_ref::<rusqlite::Error>() {
        return match error.sqlite_error() {
            Some(code) => format!("SQLite: {code}"),
            None => "SQLite query or conversion failed (detail redacted)".into(),
        };
    }
    if let Some(error) = cause.downcast_ref::<serde_json::Error>() {
        return format!(
            "JSON {:?} error at line {} column {} (detail redacted)",
            error.classify(),
            error.line(),
            error.column()
        );
    }
    if let Some(error) = cause.downcast_ref::<tokio::task::JoinError>() {
        return if error.is_panic() {
            "task panicked (payload redacted)"
        } else {
            "task cancelled"
        }
        .into();
    }
    cause.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cause_chain_retains_stage_and_operating_system_error() {
        let error =
            anyhow::Error::new(std::io::Error::from_raw_os_error(2)).context("registry open");
        let output = render(&error, &[]);
        assert!(output.starts_with("runtime-failed\n  caused by: registry open\n"));
        assert!(output.contains(&std::io::Error::from_raw_os_error(2).to_string()));
    }

    #[test]
    fn environment_values_are_redacted_before_truncation_and_not_recursively() {
        let value = "sensitive-value".repeat(400);
        let error = anyhow::anyhow!("failed at {value}").context("runtime start");
        let output = render(&error, &[value, "REDACTED".into()]);
        assert!(output.contains("runtime start"));
        assert!(output.contains("failed at [REDACTED]"));
        assert!(!output.contains("sensitive-value"));
        assert!(!output.contains("[truncated]"));
    }

    #[test]
    fn unstructured_credentials_and_connection_strings_are_omitted() {
        for text in [
            "postgres://user:pass@host/database",
            "host=localhost user=reader password='two words'",
            r#"{"token": "opaque-value"}"#,
            "Authorization: Bearer opaque-value",
            "API-key: opaque-value",
            "PAT: opaque-value",
            r#"{"key": "opaque-value"}"#,
            "ghp_opaque",
            "innocent_name=an-environment-value",
        ] {
            let output = render(&anyhow::anyhow!("{text}").context("runtime run"), &[]);
            assert!(output.contains("runtime run"));
            assert!(output.contains(REDACTED), "{output}");
            assert!(!output.contains(text));
        }
    }

    #[test]
    fn sqlite_and_json_errors_do_not_export_arbitrary_data() {
        let sqlite = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(1),
            Some("opaque private database text".into()),
        );
        let output = render(&anyhow::Error::new(sqlite).context("registry open"), &[]);
        assert!(output.contains("SQLite"));
        assert!(!output.contains("opaque private database text"));
        let json = serde_json::from_str::<u64>("\"opaque private value\"").unwrap_err();
        let output = render(&json.into(), &[]);
        assert!(output.contains("JSON Data error"));
        assert!(!output.contains("opaque private value"));
    }

    #[test]
    fn cap_includes_marker_and_newline_on_utf8_boundaries() {
        let error = anyhow::anyhow!("{}", "\u{754c}".repeat(5000)).context("management server");
        let output = render(&error, &[]);
        assert!(output.len() <= MAX_BYTES);
        assert!(output.len() >= MAX_BYTES - 3);
        assert!(output.contains("management server"));
        assert!(output.ends_with(TRUNCATED));
    }

    #[test]
    fn exact_byte_limit_is_not_marked_truncated() {
        let overhead = render(&anyhow::anyhow!(""), &[]).len();
        let output = render(
            &anyhow::anyhow!("{}", "x".repeat(MAX_BYTES - overhead)),
            &[],
        );
        assert_eq!(output.len(), MAX_BYTES);
        assert!(!output.contains("[truncated]"));
        let output = render(
            &anyhow::anyhow!("{}", "x".repeat(MAX_BYTES - overhead + 1)),
            &[],
        );
        assert_eq!(output.len(), MAX_BYTES);
        assert!(output.ends_with(TRUNCATED));
    }

    #[test]
    fn control_characters_cannot_inject_terminal_output_or_extra_lines() {
        let output = render(&anyhow::anyhow!("bad\nvalue\u{1b}[31m"), &[]);
        assert!(output.contains(r"bad\nvalue\u{1b}[31m"));
        assert_eq!(output.lines().count(), 2);
    }
}
