use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

const UNKNOWN_BUILD_ID: &str = "unknown";

fn main() {
    println!("cargo:rerun-if-env-changed=TELEX_BUILD_ID");
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");

    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap_or_else(|| ".".into()));
    let git_root = owned_git_root(&manifest_dir);
    if let Some(git_root) = git_root.as_deref() {
        emit_git_rerun_paths(git_root);
    } else {
        // Re-run if a metadata-less source tree later becomes a standalone checkout.
        // Do not watch any rejected ancestor repository metadata.
        println!(
            "cargo:rerun-if-changed={}",
            manifest_dir.join(".git").display()
        );
    }

    let build_id = env_build_id("TELEX_BUILD_ID")
        .or_else(|| env_build_id("GITHUB_SHA"))
        .or_else(|| git_root.as_deref().and_then(git_build_id))
        .unwrap_or_else(|| UNKNOWN_BUILD_ID.to_string());
    println!("cargo:rustc-env=TELEX_BUILD_ID={build_id}");
}

fn env_build_id(name: &str) -> Option<String> {
    env::var(name).ok().and_then(|value| sanitize(&value))
}

fn git_build_id(repo: &Path) -> Option<String> {
    git_stdout(repo, &["rev-parse", "HEAD"]).and_then(|value| sanitize(&value))
}

fn owned_git_root(manifest_dir: &Path) -> Option<PathBuf> {
    let top_level = PathBuf::from(git_stdout(manifest_dir, &["rev-parse", "--show-toplevel"])?);
    let manifest_dir = manifest_dir.canonicalize().ok()?;
    let top_level = top_level.canonicalize().ok()?;
    (top_level == manifest_dir).then_some(top_level)
}

fn sanitize(value: &str) -> Option<String> {
    let mut sanitized = String::with_capacity(value.len());
    let mut pending_separator = false;

    for ch in value.chars() {
        if ch.is_whitespace() || ch.is_control() {
            pending_separator = !sanitized.is_empty();
        } else {
            if pending_separator {
                sanitized.push('-');
                pending_separator = false;
            }
            sanitized.push(ch);
        }
    }

    (!sanitized.is_empty()).then_some(sanitized)
}

fn emit_git_rerun_paths(repo: &Path) {
    let mut paths = Vec::new();
    if let Some(head) = git_path(repo, "HEAD") {
        paths.push(head);
    }
    if let Some(head_ref) = git_stdout(repo, &["symbolic-ref", "-q", "HEAD"]) {
        if let Some(path) = git_path(repo, &head_ref) {
            paths.push(path);
        }
    }
    if let Some(packed_refs) = git_path(repo, "packed-refs") {
        paths.push(packed_refs);
    }

    // A packed branch can later gain a loose ref without changing packed-refs.
    // Register the loose-ref path even while absent so Cargo observes its creation.
    for path in paths {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}

fn git_path(repo: &Path, name: &str) -> Option<PathBuf> {
    let value = git_stdout(repo, &["rev-parse", "--git-path", name])?;
    let path = PathBuf::from(value);
    Some(if path.is_absolute() {
        path
    } else {
        repo.join(path)
    })
}

fn git_stdout(repo: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}
