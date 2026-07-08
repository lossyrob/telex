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
use std::path::{Component, Path, PathBuf};

use crate::install;

/// Default GitHub REST API base (override with `TELEX_UPGRADE_API_BASE` for tests/mirrors).
const DEFAULT_API_BASE: &str = "https://api.github.com";
/// Default release-asset download host (override with `TELEX_UPGRADE_DOWNLOAD_BASE`).
const DEFAULT_DOWNLOAD_BASE: &str = "https://github.com";
const API_BASE_ENV: &str = "TELEX_UPGRADE_API_BASE";
const DOWNLOAD_BASE_ENV: &str = "TELEX_UPGRADE_DOWNLOAD_BASE";
const USER_AGENT: &str = concat!("telex/", env!("CARGO_PKG_VERSION"));

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

/// All platform target triples telex can self-upgrade to. Kept in lockstep with the
/// `.github/workflows/release.yml` build matrix and `install.sh`/`install.ps1`; a
/// contract test asserts this is a subset of the release matrix so drift fails at repo test.
pub const SUPPORTED_TARGETS: &[&str] = &[
    "x86_64-pc-windows-msvc",
    "aarch64-pc-windows-msvc",
    "x86_64-unknown-linux-gnu",
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
];

/// The target triple + archive kind for the *current* platform, or `None` if this platform
/// is not built by the release workflow (self-update unsupported; install from source).
pub fn current_target() -> Option<(&'static str, ArchiveKind)> {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        Some(("x86_64-pc-windows-msvc", ArchiveKind::Zip))
    }
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    {
        Some(("aarch64-pc-windows-msvc", ArchiveKind::Zip))
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        Some(("x86_64-unknown-linux-gnu", ArchiveKind::TarGz))
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        Some(("aarch64-apple-darwin", ArchiveKind::TarGz))
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        Some(("x86_64-apple-darwin", ArchiveKind::TarGz))
    }
    #[cfg(not(any(
        all(target_os = "windows", target_arch = "x86_64"),
        all(target_os = "windows", target_arch = "aarch64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
    )))]
    {
        None
    }
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
/// if absent, and rejects obvious garbage / path-dangerous tags. Keeps `--version 0.1.0`
/// and `--version v0.1.0` equivalent and makes the already-current comparison stable.
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
    if tag.contains(['/', '\\']) || tag.contains("..") {
        bail!("invalid release tag {raw:?}");
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

/// Reject a non-HTTPS URL unless it targets loopback (the test fixture host).
fn require_secure(url: &reqwest::Url) -> Result<()> {
    if url.scheme() == "https" || is_loopback_host(url) {
        Ok(())
    } else {
        bail!("refusing insecure (non-HTTPS) upgrade URL: {url}");
    }
}

fn http_client() -> Result<reqwest::Client> {
    // Follow redirects but never downgrade https -> http for non-loopback hosts.
    let redirect = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 10 {
            return attempt.error("too many redirects");
        }
        let url = attempt.url();
        let loopback = matches!(
            url.host_str(),
            Some("127.0.0.1") | Some("localhost") | Some("::1")
        );
        if url.scheme() != "https" && !loopback {
            return attempt.error("refusing to follow a plain-HTTP redirect");
        }
        attempt.follow()
    });
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .redirect(redirect)
        .build()
        .map_err(|e| anyhow!("building HTTP client: {e}"))
}

fn map_network_err(e: reqwest::Error) -> anyhow::Error {
    if e.is_connect() || e.is_timeout() {
        anyhow!("network error reaching GitHub ({e}); check your connection and try again")
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
    // Token only on the API host — never forwarded to asset downloads.
    if let Some(token) = &cfg.token {
        req = req.bearer_auth(token);
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
    let body = resp.bytes().await.map_err(map_network_err)?;
    let release: Release =
        serde_json::from_slice(&body).context("parsing GitHub release JSON response")?;
    Ok(release)
}

/// Download a named release asset for `tag`. No auth header is attached (public assets),
/// which also avoids leaking a token across the download redirect.
pub async fn download_asset(cfg: &FetchConfig, tag: &str, asset: &str) -> Result<Vec<u8>> {
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
    let bytes = resp.bytes().await.map_err(map_network_err)?;
    Ok(bytes.to_vec())
}

/// Extract exactly the expected `telex(.exe)` entry from an in-memory archive into `out_dir`,
/// rejecting path traversal. On Unix, marks the extracted binary executable (0o755). Returns
/// the path to the staged binary.
pub fn safe_extract(kind: ArchiveKind, archive: &[u8], out_dir: &Path) -> Result<PathBuf> {
    let expected = install::exe_name();
    let dest = out_dir.join(expected);
    match kind {
        ArchiveKind::Zip => extract_zip(archive, expected, &dest)?,
        ArchiveKind::TarGz => extract_tar_gz(archive, expected, &dest)?,
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

fn extract_zip(archive: &[u8], expected: &str, dest: &Path) -> Result<()> {
    let reader = std::io::Cursor::new(archive);
    let mut zip = zip::ZipArchive::new(reader).context("opening downloaded zip archive")?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).context("reading zip entry")?;
        // `enclosed_name` returns None for traversal/absolute paths (zip-slip guard).
        let enclosed = entry.enclosed_name().ok_or_else(|| {
            anyhow!(
                "archive entry {:?} escapes the extraction directory (rejected)",
                entry.name()
            )
        })?;
        let is_top_level_binary = enclosed.components().count() == 1
            && enclosed.file_name().and_then(|n| n.to_str()) == Some(expected);
        if is_top_level_binary {
            let mut out = std::fs::File::create(dest)
                .with_context(|| format!("creating staged binary {}", dest.display()))?;
            std::io::copy(&mut entry, &mut out).context("writing extracted binary")?;
            return Ok(());
        }
    }
    bail!("downloaded archive did not contain a top-level `{expected}` entry");
}

fn extract_tar_gz(archive: &[u8], expected: &str, dest: &Path) -> Result<()> {
    let gz = flate2::read::GzDecoder::new(std::io::Cursor::new(archive));
    let mut tar = tar::Archive::new(gz);
    for entry in tar.entries().context("reading tar archive")? {
        let mut entry = entry.context("reading tar entry")?;
        let path = entry.path().context("reading tar entry path")?.into_owned();
        if path_component_is_unsafe(&path) {
            bail!(
                "archive entry {:?} escapes the extraction directory (rejected)",
                path
            );
        }
        let is_top_level_binary = path.components().count() == 1
            && path.file_name().and_then(|n| n.to_str()) == Some(expected);
        if is_top_level_binary {
            if entry.header().entry_type() != tar::EntryType::Regular {
                bail!("archive entry {expected:?} is not a regular file (rejected)");
            }
            let mut out = std::fs::File::create(dest)
                .with_context(|| format!("creating staged binary {}", dest.display()))?;
            std::io::copy(&mut entry, &mut out).context("writing extracted binary")?;
            return Ok(());
        }
    }
    bail!("downloaded archive did not contain a top-level `{expected}` entry");
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
        // the supported set.
        let (target, _kind) = current_target().expect("current platform should be supported");
        assert!(SUPPORTED_TARGETS.contains(&target));
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
    fn tar_unsafe_path_predicate() {
        assert!(path_component_is_unsafe(Path::new("../telex")));
        assert!(path_component_is_unsafe(Path::new("a/../../telex")));
        assert!(!path_component_is_unsafe(Path::new("telex")));
        assert!(!path_component_is_unsafe(Path::new("sub/telex")));
    }
}
