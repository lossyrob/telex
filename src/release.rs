//! In-binary release upgrade: discover a public GitHub release, download the current
//! platform's asset and its SHA-256 sidecar, verify the checksum, and extract `telex(.exe)`
//! into a staging directory so the versioned installer can promote it.
//!
//! Security posture (see `docs/design` and the release-upgrade plan):
//! - HTTPS-only for real hosts (loopback may be plain HTTP for tests).
//! - Fail closed on a missing/mismatched checksum sidecar.
//! - The `GITHUB_TOKEN` is attached only to API discovery requests, never to asset
//!   downloads, so it cannot leak across the `github.com` -> object-store redirect.
//! - Extraction writes only the expected `telex(.exe)` entry to a controlled dir and
//!   rejects path traversal (zip-slip / tar `..` / absolute / symlink).
//!
//! Compiled only under the `self-update` feature.

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use crate::install;

/// Default GitHub REST API base (override with `TELEX_UPGRADE_API_BASE` for tests/mirrors).
const DEFAULT_API_BASE: &str = "https://api.github.com";
/// Default release-asset download host (override with `TELEX_UPGRADE_DOWNLOAD_BASE`).
const DEFAULT_DOWNLOAD_BASE: &str = "https://github.com";
const API_BASE_ENV: &str = "TELEX_UPGRADE_API_BASE";
const DOWNLOAD_BASE_ENV: &str = "TELEX_UPGRADE_DOWNLOAD_BASE";
const CONNECT_TIMEOUT_ENV: &str = "TELEX_UPGRADE_CONNECT_TIMEOUT_MS";
const READ_TIMEOUT_ENV: &str = "TELEX_UPGRADE_READ_TIMEOUT_MS";
const TOTAL_TIMEOUT_ENV: &str = "TELEX_UPGRADE_TIMEOUT_MS";
const USER_AGENT: &str = concat!("telex/", env!("CARGO_PKG_VERSION"));

/// Upper bound on a downloaded archive (defends against a hostile/misbehaving mirror
/// OOMing telex before the checksum is even computed). Release archives are a few MB.
pub const MAX_ARCHIVE_BYTES: usize = 256 * 1024 * 1024;
/// Upper bound on a checksum sidecar (a single hex line).
pub const MAX_SIDECAR_BYTES: usize = 64 * 1024;
/// Upper bound on a release-discovery JSON body.
const MAX_DISCOVERY_BYTES: usize = 4 * 1024 * 1024;
/// Upper bound on the extracted binary (defends against a zip/gzip bomb or an accidentally
/// huge asset filling the disk during extraction).
const MAX_EXTRACTED_BYTES: u64 = 256 * 1024 * 1024;

/// Archive container used by a platform's release asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    Zip,
    TarGz,
}

impl ArchiveKind {
    pub fn extension(self) -> &'static str {
        match self {
            ArchiveKind::Zip => "zip",
            ArchiveKind::TarGz => "tar.gz",
        }
    }
}

/// A platform telex can self-upgrade to: the `std::env::consts` os/arch pair, its Rust target
/// triple, and its release archive kind.
#[derive(Debug, Clone, Copy)]
pub struct TargetSpec {
    pub os: &'static str,
    pub arch: &'static str,
    pub triple: &'static str,
    pub kind: ArchiveKind,
}

/// Every platform telex can self-upgrade to. This is the single source of truth for both the
/// current-platform lookup and the release-contract coupling test, which asserts set equality
/// with the `.github/workflows/release.yml` build matrix — so a matrix change (either direction)
/// breaks a repo test rather than a user's `telex upgrade`. Also kept in sync with the installers.
pub const SUPPORTED_TARGETS: &[TargetSpec] = &[
    TargetSpec {
        os: "windows",
        arch: "x86_64",
        triple: "x86_64-pc-windows-msvc",
        kind: ArchiveKind::Zip,
    },
    TargetSpec {
        os: "windows",
        arch: "aarch64",
        triple: "aarch64-pc-windows-msvc",
        kind: ArchiveKind::Zip,
    },
    TargetSpec {
        os: "linux",
        arch: "x86_64",
        triple: "x86_64-unknown-linux-gnu",
        kind: ArchiveKind::TarGz,
    },
    TargetSpec {
        os: "macos",
        arch: "aarch64",
        triple: "aarch64-apple-darwin",
        kind: ArchiveKind::TarGz,
    },
    TargetSpec {
        os: "macos",
        arch: "x86_64",
        triple: "x86_64-apple-darwin",
        kind: ArchiveKind::TarGz,
    },
];

/// The target triple + archive kind for the *current* platform, or `None` if this platform
/// is not built by the release workflow (self-update unsupported; install from source). Resolved
/// from the `SUPPORTED_TARGETS` table by the running `std::env::consts` os/arch, so every entry
/// (not just the current one) is exercised by the table tests on any runner.
pub fn current_target() -> Option<(&'static str, ArchiveKind)> {
    SUPPORTED_TARGETS
        .iter()
        .find(|t| t.os == std::env::consts::OS && t.arch == std::env::consts::ARCH)
        .map(|t| (t.triple, t.kind))
}

/// A minimal view of a GitHub release payload.
#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Asset {
    pub name: String,
}

impl Release {
    pub fn asset_names(&self) -> Vec<String> {
        self.assets.iter().map(|a| a.name.clone()).collect()
    }
}

/// The platform asset + its checksum sidecar selected for a release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedAsset {
    pub archive_name: String,
    pub sidecar_name: String,
}

/// The expected asset file name for a tag/target/kind (matches release.yml packaging).
pub fn asset_name(tag: &str, target: &str, kind: ArchiveKind) -> String {
    format!("telex-{tag}-{target}.{}", kind.extension())
}

/// Select the platform archive and its `.sha256` sidecar from a release's asset names.
/// Fails closed if the platform archive is absent (likely unsupported platform for this
/// release) or if the checksum sidecar is missing (refuse to install unverified).
pub fn select_asset(
    asset_names: &[String],
    tag: &str,
    target: &str,
    kind: ArchiveKind,
) -> Result<SelectedAsset> {
    let archive_name = asset_name(tag, target, kind);
    let sidecar_name = format!("{archive_name}.sha256");
    if !asset_names.iter().any(|n| n == &archive_name) {
        bail!(
            "release {tag} has no asset for this platform ({target}); expected {archive_name}. \
             This platform may be unsupported by this release — install from source with \
             `cargo install --git https://github.com/lossyrob/telex --features entra`."
        );
    }
    if !asset_names.iter().any(|n| n == &sidecar_name) {
        bail!(
            "release {tag} is missing checksum sidecar {sidecar_name}; refusing to install \
             without checksum verification"
        );
    }
    Ok(SelectedAsset {
        archive_name,
        sidecar_name,
    })
}

/// Parse a `<hash>  <filename>` checksum sidecar and return the lowercase hex SHA-256
/// (field 1). Strict: requires a 64-character hex digest.
pub fn parse_sha256_sidecar(text: &str) -> Result<String> {
    let first = text
        .split_whitespace()
        .next()
        .ok_or_else(|| anyhow!("empty checksum sidecar"))?;
    let hash = first.to_ascii_lowercase();
    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("checksum sidecar does not contain a 64-char hex SHA-256: {first:?}");
    }
    Ok(hash)
}

/// Verify the SHA-256 of `bytes` equals `expected_hex` (lowercase hex). Fail closed.
pub fn verify_checksum(bytes: &[u8], expected_hex: &str) -> Result<()> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let actual = hex_lower(hasher.finalize().as_slice());
    if actual != expected_hex {
        bail!(
            "checksum mismatch: expected {expected_hex}, computed {actual}. \
             The download is corrupt or tampered; refusing to install."
        );
    }
    Ok(())
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Normalize a release tag to `vX...` form: accepts an optional leading `v`, prepends one
/// if absent, and validates it against the release-tag grammar. Keeps `--version 0.1.0` and
/// `--version v0.1.0` equivalent and makes the already-current comparison stable. Rejects
/// anything outside `v<digit>[0-9A-Za-z.+-]*` so a tag can never carry URL metacharacters
/// (`?`, `#`, `%`, space, ...) that would silently change the fetched path/query.
pub fn normalize_tag(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("empty release tag");
    }
    let tag = if trimmed.starts_with('v') {
        trimmed.to_string()
    } else {
        format!("v{trimmed}")
    };
    let after_v = &tag[1..];
    if !after_v.starts_with(|c: char| c.is_ascii_digit()) {
        bail!("invalid release tag {raw:?}; expected a version like v0.1.0");
    }
    // Semver-shaped tags only: digits/letters and `.`, `-`, `+`. This excludes path/query
    // separators and URL metacharacters, so the tag interpolates safely into the fetch URLs.
    if !after_v
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
    {
        bail!("invalid release tag {raw:?}; expected a version like v0.1.0");
    }
    Ok(tag)
}

/// Repo + base URLs + optional token for a release fetch. Bases are injectable via env for
/// tests and enterprise mirrors; they default to the public GitHub hosts.
#[derive(Debug, Clone)]
pub struct FetchConfig {
    pub repo: String,
    pub api_base: String,
    pub download_base: String,
    pub token: Option<String>,
}

impl FetchConfig {
    pub fn from_repo(repo: &str) -> Self {
        FetchConfig {
            repo: repo.to_string(),
            api_base: std::env::var(API_BASE_ENV).unwrap_or_else(|_| DEFAULT_API_BASE.to_string()),
            download_base: std::env::var(DOWNLOAD_BASE_ENV)
                .unwrap_or_else(|_| DEFAULT_DOWNLOAD_BASE.to_string()),
            token: std::env::var("GITHUB_TOKEN").ok().filter(|t| !t.is_empty()),
        }
    }
}

fn is_loopback_host(url: &reqwest::Url) -> bool {
    matches!(
        url.host_str(),
        Some("127.0.0.1") | Some("localhost") | Some("::1")
    )
}

/// True when the API base is the default public GitHub host, i.e. the only host the
/// `GITHUB_TOKEN` may be sent to. A configured mirror (`TELEX_UPGRADE_API_BASE`) is not
/// trusted with the token.
fn is_default_github_api(api_base: &str) -> bool {
    api_base.trim_end_matches('/') == DEFAULT_API_BASE
}

/// Reject a non-HTTPS URL unless it targets loopback (the test fixture host).
fn require_secure(url: &reqwest::Url) -> Result<()> {
    if url.scheme() == "https" || is_loopback_host(url) {
        Ok(())
    } else {
        bail!("refusing insecure (non-HTTPS) upgrade URL: {url}");
    }
}

fn env_timeout_ms(key: &str, default_ms: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(default_ms)
}

fn http_client() -> Result<reqwest::Client> {
    // Bound every request so a stalled/slow-loris mirror or half-open socket cannot hang
    // `telex upgrade` forever. connect_timeout bounds the TCP/TLS handshake; read_timeout fails a
    // request that stalls mid-stream; the total timeout caps a slow-drip transfer overall. All are
    // overridable for tests via env.
    let connect_ms = env_timeout_ms(CONNECT_TIMEOUT_ENV, 15_000);
    let read_ms = env_timeout_ms(READ_TIMEOUT_ENV, 30_000);
    let total_ms = env_timeout_ms(TOTAL_TIMEOUT_ENV, 300_000);
    // Follow redirects but never downgrade https -> http for non-loopback hosts.
    let redirect = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 10 {
            return attempt.error("too many redirects");
        }
        if attempt.url().scheme() != "https" && !is_loopback_host(attempt.url()) {
            return attempt.error("refusing to follow a plain-HTTP redirect");
        }
        attempt.follow()
    });
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_millis(connect_ms))
        .read_timeout(Duration::from_millis(read_ms))
        .timeout(Duration::from_millis(total_ms))
        .redirect(redirect)
        .build()
        .map_err(|e| anyhow!("building HTTP client: {e}"))
}

fn map_network_err(e: reqwest::Error) -> anyhow::Error {
    let host = e
        .url()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| "the release host".to_string());
    if e.is_connect() || e.is_timeout() {
        anyhow!("network error reaching {host} ({e}); check your connection and try again")
    } else {
        anyhow!("HTTP error: {e}")
    }
}

/// Fetch a release by explicit tag, or the latest full (non-draft, non-prerelease) release
/// when `tag` is `None`.
pub async fn discover_release(cfg: &FetchConfig, tag: Option<&str>) -> Result<Release> {
    let client = http_client()?;
    let api = cfg.api_base.trim_end_matches('/');
    let url = match tag {
        Some(t) => format!("{api}/repos/{}/releases/tags/{t}", cfg.repo),
        None => format!("{api}/repos/{}/releases/latest", cfg.repo),
    };
    let parsed = reqwest::Url::parse(&url).with_context(|| format!("invalid API URL {url}"))?;
    require_secure(&parsed)?;
    let mut req = client
        .get(parsed)
        .header("Accept", "application/vnd.github+json");
    // Token only on the default GitHub API host, and never on asset downloads — a configured
    // mirror (TELEX_UPGRADE_API_BASE) is not trusted with the user's credential.
    if let Some(token) = &cfg.token {
        if is_default_github_api(&cfg.api_base) {
            req = req.bearer_auth(token);
        }
    }
    let resp = req.send().await.map_err(map_network_err)?;
    let status = resp.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        match tag {
            Some(t) => bail!(
                "release {t} not found in {} (is the tag published and public?)",
                cfg.repo
            ),
            None => bail!(
                "no published release found in {} (is a public release available?)",
                cfg.repo
            ),
        }
    }
    if status == reqwest::StatusCode::FORBIDDEN || status == reqwest::StatusCode::TOO_MANY_REQUESTS
    {
        bail!(
            "GitHub API returned {status} (rate limit or authorization). \
             Set GITHUB_TOKEN to raise the rate limit and retry."
        );
    }
    if !status.is_success() {
        bail!("GitHub API request failed: {status} for {}", cfg.repo);
    }
    if let Some(len) = resp.content_length() {
        if len > MAX_DISCOVERY_BYTES as u64 {
            bail!("release metadata is implausibly large ({len} bytes); refusing");
        }
    }
    let body = read_capped(resp, MAX_DISCOVERY_BYTES, "release metadata").await?;
    let release: Release =
        serde_json::from_slice(&body).context("parsing GitHub release JSON response")?;
    // A draft is never installable; a prerelease is only installable via an explicit `--version`
    // (the tag itself is the opt-in), never on the default "latest" path — even if a mirror lies.
    ensure_publishable(&release, tag.is_some())?;
    Ok(release)
}

/// Read a response body with a running byte cap (a server that lies about `Content-Length`
/// still cannot exceed the cap).
async fn read_capped(mut resp: reqwest::Response, max_bytes: usize, what: &str) -> Result<Vec<u8>> {
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(map_network_err)? {
        if buf.len() + chunk.len() > max_bytes {
            bail!("{what} exceeded the {max_bytes}-byte limit; refusing to continue");
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

/// Refuse to install a release that is not a normal published artifact: a draft is never
/// installable; a prerelease is only allowed when the caller asked for an explicit tag.
fn ensure_publishable(rel: &Release, explicit_tag: bool) -> Result<()> {
    if rel.draft {
        bail!(
            "release {} is a draft, not a published release; refusing to install",
            rel.tag_name
        );
    }
    if rel.prerelease && !explicit_tag {
        bail!(
            "latest release {} is a prerelease; refusing to auto-install it. \
             Install it explicitly with `telex upgrade --version {}` if intended.",
            rel.tag_name,
            rel.tag_name
        );
    }
    Ok(())
}

/// Download a named release asset for `tag`, bounded by `max_bytes`. No auth header is
/// attached (public assets), which also avoids leaking a token across the download redirect.
pub async fn download_asset(
    cfg: &FetchConfig,
    tag: &str,
    asset: &str,
    max_bytes: usize,
) -> Result<Vec<u8>> {
    let client = http_client()?;
    let base = cfg.download_base.trim_end_matches('/');
    let url = format!("{base}/{}/releases/download/{tag}/{asset}", cfg.repo);
    let parsed =
        reqwest::Url::parse(&url).with_context(|| format!("invalid download URL {url}"))?;
    require_secure(&parsed)?;
    let resp = client.get(parsed).send().await.map_err(map_network_err)?;
    let status = resp.status();
    if !status.is_success() {
        bail!("failed to download {asset}: HTTP {status}");
    }
    if let Some(len) = resp.content_length() {
        if len > max_bytes as u64 {
            bail!(
                "{asset} advertises {len} bytes, exceeding the {max_bytes}-byte limit; \
                 refusing to download"
            );
        }
    }
    // Stream with a running cap so a server that lies about Content-Length still can't OOM us.
    read_capped(resp, max_bytes, asset).await
}

/// Extract exactly the expected `telex(.exe)` entry from an in-memory archive into `out_dir`,
/// rejecting path traversal. On Unix, marks the extracted binary executable (0o755). Returns
/// the path to the staged binary.
pub fn safe_extract(kind: ArchiveKind, archive: &[u8], out_dir: &Path) -> Result<PathBuf> {
    let expected = install::exe_name();
    let dest = out_dir.join(expected);
    match kind {
        ArchiveKind::Zip => extract_zip(archive, expected, &dest, MAX_EXTRACTED_BYTES)?,
        ArchiveKind::TarGz => extract_tar_gz(archive, expected, &dest, MAX_EXTRACTED_BYTES)?,
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))
            .with_context(|| format!("setting executable bit on {}", dest.display()))?;
    }
    Ok(dest)
}

/// True if a tar entry path escapes the extraction dir (absolute, prefix, or `..`).
fn path_component_is_unsafe(path: &Path) -> bool {
    path.components().any(|c| {
        matches!(
            c,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    })
}

/// Copy at most `cap` bytes from `reader` to `writer`, failing closed if the source exceeds
/// the cap (defends against a zip/gzip bomb or an accidentally huge asset).
fn copy_capped<R: Read, W: std::io::Write>(reader: R, writer: &mut W, cap: u64) -> Result<u64> {
    let mut limited = reader.take(cap + 1);
    let written = std::io::copy(&mut limited, writer).context("writing extracted binary")?;
    if written > cap {
        bail!("extracted entry exceeds the {cap}-byte size limit; refusing to install");
    }
    Ok(written)
}

const SYMLINK_MODE_BITS: u32 = 0o120000;
const FILE_TYPE_MASK: u32 = 0o170000;

fn extract_zip(archive: &[u8], expected: &str, dest: &Path, cap: u64) -> Result<()> {
    let reader = std::io::Cursor::new(archive);
    let mut zip = zip::ZipArchive::new(reader).context("opening downloaded zip archive")?;
    // Scan the whole archive and require exactly one regular-file top-level `expected` entry,
    // so a self-updater fails closed on an ambiguous (duplicate) or non-regular (dir/symlink)
    // archive rather than trusting archive order.
    let mut match_index: Option<usize> = None;
    for i in 0..zip.len() {
        let entry = zip.by_index(i).context("reading zip entry")?;
        // `enclosed_name` returns None for traversal/absolute paths (zip-slip guard).
        let enclosed = entry.enclosed_name().ok_or_else(|| {
            anyhow!(
                "archive entry {:?} escapes the extraction directory (rejected)",
                entry.name()
            )
        })?;
        let is_named = enclosed.components().count() == 1
            && enclosed.file_name().and_then(|n| n.to_str()) == Some(expected);
        if !is_named {
            continue;
        }
        if entry.is_dir() {
            bail!("archive entry {expected:?} is a directory, not a file (rejected)");
        }
        if entry
            .unix_mode()
            .is_some_and(|m| m & FILE_TYPE_MASK == SYMLINK_MODE_BITS)
        {
            bail!("archive entry {expected:?} is a symlink, not a regular file (rejected)");
        }
        if match_index.is_some() {
            bail!("archive contains multiple `{expected}` entries (ambiguous); refusing");
        }
        match_index = Some(i);
    }
    let i = match_index.ok_or_else(|| {
        anyhow!("downloaded archive did not contain a top-level `{expected}` entry")
    })?;
    let mut entry = zip.by_index(i).context("reading zip entry")?;
    let mut out = std::fs::File::create(dest)
        .with_context(|| format!("creating staged binary {}", dest.display()))?;
    copy_capped(&mut entry, &mut out, cap)?;
    Ok(())
}

fn extract_tar_gz(archive: &[u8], expected: &str, dest: &Path, cap: u64) -> Result<()> {
    // Cap total decompressed bytes across ALL scanned entries so a gzip bomb (or a bomb entry
    // placed before `telex`) cannot decompress unbounded through the non-seekable stream.
    let gz = flate2::read::GzDecoder::new(std::io::Cursor::new(archive));
    let mut tar = tar::Archive::new(gz.take(cap.saturating_add(1)));
    let mut found = false;
    for entry in tar.entries().context("reading tar archive")? {
        let mut entry = entry.context("reading tar entry")?;
        let path = entry.path().context("reading tar entry path")?.into_owned();
        if path_component_is_unsafe(&path) {
            bail!(
                "archive entry {:?} escapes the extraction directory (rejected)",
                path
            );
        }
        let is_named = path.components().count() == 1
            && path.file_name().and_then(|n| n.to_str()) == Some(expected);
        if !is_named {
            continue;
        }
        if entry.header().entry_type() != tar::EntryType::Regular {
            bail!("archive entry {expected:?} is not a regular file (rejected)");
        }
        if found {
            bail!("archive contains multiple `{expected}` entries (ambiguous); refusing");
        }
        found = true;
        let mut out = std::fs::File::create(dest)
            .with_context(|| format!("creating staged binary {}", dest.display()))?;
        copy_capped(&mut entry, &mut out, cap)?;
        // Keep scanning (bounded by the cumulative cap) to detect a duplicate entry.
    }
    if !found {
        bail!("downloaded archive did not contain a top-level `{expected}` entry");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn normalize_tag_prefixes_and_validates() {
        assert_eq!(normalize_tag("0.1.0").unwrap(), "v0.1.0");
        assert_eq!(normalize_tag("v0.1.0").unwrap(), "v0.1.0");
        assert_eq!(normalize_tag(" v0.2.3 ").unwrap(), "v0.2.3");
        // Prerelease/build suffixes are preserved.
        assert_eq!(normalize_tag("v0.1.0-rc.1").unwrap(), "v0.1.0-rc.1");
        assert!(normalize_tag("").is_err());
        assert!(normalize_tag("vfoo").is_err());
        assert!(normalize_tag("../evil").is_err());
    }

    #[test]
    fn normalize_tag_treats_prefixed_and_unprefixed_identically() {
        assert_eq!(
            normalize_tag("0.1.0").unwrap(),
            normalize_tag("v0.1.0").unwrap()
        );
    }

    #[test]
    fn parse_sidecar_takes_field_one_strict_hex() {
        let hex = "a".repeat(64);
        assert_eq!(
            parse_sha256_sidecar(&format!("{hex}  telex-v0.1.0-x.zip")).unwrap(),
            hex
        );
        // Uppercase is normalized to lowercase.
        let upper = "A".repeat(64);
        assert_eq!(parse_sha256_sidecar(&upper).unwrap(), "a".repeat(64));
        assert!(parse_sha256_sidecar("").is_err());
        assert!(parse_sha256_sidecar("deadbeef  x").is_err()); // too short
        assert!(parse_sha256_sidecar(&format!("{}  x", "z".repeat(64))).is_err());
        // non-hex
    }

    #[test]
    fn verify_checksum_matches_and_rejects() {
        // SHA-256 of "telex" (lowercase hex).
        let data = b"telex";
        use sha2::{Digest, Sha256};
        let expected = hex_lower(Sha256::digest(data).as_slice());
        assert!(verify_checksum(data, &expected).is_ok());
        assert!(verify_checksum(b"tampered", &expected).is_err());
    }

    #[test]
    fn select_asset_requires_archive_and_sidecar() {
        let archive = asset_name("v0.1.0", "x86_64-unknown-linux-gnu", ArchiveKind::TarGz);
        let sidecar = format!("{archive}.sha256");
        let names = vec![archive.clone(), sidecar.clone(), "LICENSE".to_string()];
        let sel = select_asset(
            &names,
            "v0.1.0",
            "x86_64-unknown-linux-gnu",
            ArchiveKind::TarGz,
        )
        .unwrap();
        assert_eq!(sel.archive_name, archive);
        assert_eq!(sel.sidecar_name, sidecar);

        // Missing sidecar -> fail closed.
        let no_sidecar = vec![archive.clone()];
        assert!(select_asset(
            &no_sidecar,
            "v0.1.0",
            "x86_64-unknown-linux-gnu",
            ArchiveKind::TarGz
        )
        .is_err());

        // Missing archive (unsupported platform) -> error.
        let other = vec!["telex-v0.1.0-other.zip".to_string()];
        assert!(select_asset(
            &other,
            "v0.1.0",
            "x86_64-unknown-linux-gnu",
            ArchiveKind::TarGz
        )
        .is_err());
    }

    #[test]
    fn current_platform_target_is_supported() {
        // On the CI/dev platforms telex is built for, current_target must resolve and be in
        // the supported table.
        let (target, _kind) = current_target().expect("current platform should be supported");
        assert!(SUPPORTED_TARGETS.iter().any(|t| t.triple == target));
    }

    #[test]
    fn supported_targets_table_is_well_formed() {
        // Every non-current cfg branch is exercised here (data-driven table), catching a typo
        // in a macOS/ARM triple or archive kind on any runner.
        let triples: std::collections::BTreeSet<&str> =
            SUPPORTED_TARGETS.iter().map(|t| t.triple).collect();
        assert_eq!(triples.len(), SUPPORTED_TARGETS.len(), "duplicate triple");
        for t in SUPPORTED_TARGETS {
            let expect_zip = t.os == "windows";
            assert_eq!(
                t.kind == ArchiveKind::Zip,
                expect_zip,
                "{} should use {} archive",
                t.triple,
                if expect_zip { "zip" } else { "tar.gz" }
            );
        }
    }

    #[test]
    fn normalize_tag_rejects_url_metacharacters() {
        for bad in [
            "v1.2.3?evil",
            "v1.2.3#frag",
            "v1.2.3%2e",
            "v1.2 3",
            "v1/2",
            "v1\\2",
        ] {
            assert!(normalize_tag(bad).is_err(), "should reject {bad:?}");
        }
        // Legitimate semver shapes are accepted.
        assert_eq!(
            normalize_tag("v1.2.3-rc.1+build.5").unwrap(),
            "v1.2.3-rc.1+build.5"
        );
    }

    #[test]
    fn require_secure_rejects_plain_http_for_real_hosts() {
        let http = reqwest::Url::parse("http://api.github.com/x").unwrap();
        assert!(require_secure(&http).is_err());
        let https = reqwest::Url::parse("https://api.github.com/x").unwrap();
        assert!(require_secure(&https).is_ok());
        // Loopback is exempt (test fixture only).
        let loop_http = reqwest::Url::parse("http://127.0.0.1:8080/x").unwrap();
        assert!(require_secure(&loop_http).is_ok());
    }

    #[test]
    fn is_default_github_api_only_matches_default_host() {
        assert!(is_default_github_api("https://api.github.com"));
        assert!(is_default_github_api("https://api.github.com/"));
        assert!(!is_default_github_api("https://evil.example.com"));
        assert!(!is_default_github_api("http://127.0.0.1:1234"));
    }

    fn build_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let opts: zip::write::FileOptions<()> = zip::write::FileOptions::default();
            for (name, data) in entries {
                w.start_file(*name, opts).unwrap();
                w.write_all(data).unwrap();
            }
            w.finish().unwrap();
        }
        buf
    }

    fn build_tar_gz(entries: &[(&str, &[u8])]) -> Vec<u8> {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        {
            let mut builder = tar::Builder::new(&mut gz);
            for (name, data) in entries {
                let mut header = tar::Header::new_gnu();
                header.set_size(data.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                builder.append_data(&mut header, name, *data).unwrap();
            }
            builder.finish().unwrap();
        }
        gz.finish().unwrap()
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "telex-release-test-{}-{tag}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn safe_extract_zip_happy_path() {
        let exe = install::exe_name();
        let zip = build_zip(&[(exe, b"binary-bytes"), ("LICENSE", b"MIT")]);
        let dir = temp_dir("zip-happy");
        let out = safe_extract(ArchiveKind::Zip, &zip, &dir).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"binary-bytes");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn safe_extract_tar_gz_happy_path() {
        let exe = install::exe_name();
        let tgz = build_tar_gz(&[(exe, b"binary-bytes"), ("LICENSE", b"MIT")]);
        let dir = temp_dir("tar-happy");
        let out = safe_extract(ArchiveKind::TarGz, &tgz, &dir).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"binary-bytes");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&out).unwrap().permissions().mode();
            assert!(mode & 0o111 != 0, "extracted binary should be executable");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn safe_extract_rejects_zip_traversal() {
        // A zip whose only entry is a traversal path is rejected outright.
        let zip = build_zip(&[("../evil", b"x")]);
        let dir = temp_dir("zip-traversal");
        let err = safe_extract(ArchiveKind::Zip, &zip, &dir).unwrap_err();
        assert!(
            format!("{err}").contains("escapes") || format!("{err}").contains("did not contain"),
            "unexpected error: {err}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn safe_extract_rejects_nested_binary() {
        // A binary nested under a subdir is not the top-level entry we accept.
        let exe = install::exe_name();
        let zip = build_zip(&[(&format!("sub/{exe}"), b"x")]);
        let dir = temp_dir("zip-nested");
        assert!(safe_extract(ArchiveKind::Zip, &zip, &dir).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn safe_extract_rejects_duplicate_binary_entries() {
        // Two top-level `telex` entries are ambiguous; fail closed rather than trusting order.
        // (The zip writer de-duplicates same-named entries, so this is exercised via tar, whose
        // format permits genuine duplicates — the same guard exists in both extractors.)
        let exe = install::exe_name();
        let tgz = build_tar_gz(&[(exe, b"first"), (exe, b"second")]);
        let dir = temp_dir("tar-dup");
        let err = safe_extract(ArchiveKind::TarGz, &tgz, &dir).unwrap_err();
        assert!(format!("{err}").contains("multiple"), "unexpected: {err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn safe_extract_rejects_zip_directory_named_like_binary() {
        // A directory entry named `telex/` must not satisfy the binary match.
        let exe = install::exe_name();
        let zip = build_zip(&[(&format!("{exe}/"), b"")]);
        let dir = temp_dir("zip-dir");
        assert!(safe_extract(ArchiveKind::Zip, &zip, &dir).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn extract_tar_gz_bounds_cumulative_decompression() {
        // A junk entry before `telex` must not decompress unbounded: a small cap trips before
        // the target entry is reached.
        let exe = install::exe_name();
        let junk = vec![0u8; 8192];
        let tgz = build_tar_gz(&[("junk.bin", &junk), (exe, b"payload")]);
        let dir = temp_dir("tar-bomb");
        let dest = dir.join(exe);
        let err = extract_tar_gz(&tgz, exe, &dest, 1024).unwrap_err();
        assert!(
            format!("{err}").contains("exceed")
                || format!("{err}").contains("did not contain")
                || format!("{err}").contains("tar"),
            "small cap should stop cumulative decompression: {err}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn tar_unsafe_path_predicate() {
        assert!(path_component_is_unsafe(Path::new("../telex")));
        assert!(path_component_is_unsafe(Path::new("a/../../telex")));
        assert!(!path_component_is_unsafe(Path::new("telex")));
        assert!(!path_component_is_unsafe(Path::new("sub/telex")));
    }

    #[test]
    fn copy_capped_rejects_oversized_source() {
        let mut out = Vec::new();
        // Under the cap: ok.
        assert!(copy_capped(&b"hello"[..], &mut out, 10).is_ok());
        assert_eq!(out, b"hello");
        // Over the cap: fail closed.
        let mut out2 = Vec::new();
        let err = copy_capped(&[0u8; 20][..], &mut out2, 10).unwrap_err();
        assert!(format!("{err}").contains("exceeds"), "unexpected: {err}");
    }

    #[test]
    fn ensure_publishable_rejects_draft_and_bare_prerelease() {
        let draft = Release {
            tag_name: "v9.9.9".into(),
            draft: true,
            prerelease: false,
            assets: vec![],
        };
        // Draft is never installable.
        assert!(ensure_publishable(&draft, false).is_err());
        assert!(ensure_publishable(&draft, true).is_err());

        let pre = Release {
            tag_name: "v9.9.9-rc.1".into(),
            draft: false,
            prerelease: true,
            assets: vec![],
        };
        // Prerelease on the latest path is rejected; explicit --version opts in.
        assert!(ensure_publishable(&pre, false).is_err());
        assert!(ensure_publishable(&pre, true).is_ok());

        let published = Release {
            tag_name: "v9.9.9".into(),
            draft: false,
            prerelease: false,
            assets: vec![],
        };
        assert!(ensure_publishable(&published, false).is_ok());
    }
}
