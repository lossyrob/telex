//! Generic handler-kind and producer-root registry, plus the single shared push-argv builder.
//!
//! Two boundaries are enforced here, and they are what keep the daemon core harness-agnostic
//! (ADR 0039) while still letting it restore a Copilot push handler:
//!
//! 1. **Handler kinds.** A persisted intent names a *kind*, never an executable and never argv.
//!    The daemon will only rebuild argv for a kind registered at composition time by the harness
//!    layer. An unknown or unregistered kind is never launched.
//! 2. **Producer roots.** A persisted credential descriptor names a *registered root id*, never a
//!    free-form absolute path. The daemon resolves the id to a root it was told about at
//!    composition time, then requires the credential path to canonicalize strictly under it. The
//!    daemon core therefore learns "root X is registered", never a Copilot path or filename.
//!
//! `build_push_argv` is the single owner of argv shape. Both the attach path and the daemon-side
//! restore path call it, so the two can never disagree — in particular about `--daemon-instance`,
//! the fence flag that stops a helper spawned by a dying daemon from injecting into a session the
//! successor now owns.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use crate::station_intent::validate_session_id;

/// The one handler kind this build registers. Declared here (not in `commands::copilot`) so the
/// daemon can compare against a constant without depending on the harness module, but *registered*
/// by the harness layer so the daemon core never assumes it exists.
pub const COPILOT_PUSH_HANDLER_KIND: &str = "telex_copilot_push_v1";
/// Root id for the Copilot bridge registry directory.
pub const COPILOT_BRIDGE_ROOT_ID: &str = "copilot_bridge_root";

#[derive(Debug)]
pub enum RegistryError {
    UnknownHandlerKind(String),
    UnknownProducerRoot(String),
    InvalidParameter(String),
    Containment(String),
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistryError::UnknownHandlerKind(kind) => {
                write!(f, "handler kind {kind:?} is not registered")
            }
            RegistryError::UnknownProducerRoot(root) => {
                write!(f, "producer root {root:?} is not registered")
            }
            RegistryError::InvalidParameter(msg) => write!(f, "invalid handler parameter: {msg}"),
            RegistryError::Containment(msg) => write!(f, "credential path rejected: {msg}"),
        }
    }
}

impl std::error::Error for RegistryError {}

pub type Result<T> = std::result::Result<T, RegistryError>;

/// Which store a rebuilt handler should target, expressed the way the CLI expresses it. Resolved
/// per side (`ctx.cfg` on the client, `store_selector_for_key` on the daemon) and then funnelled
/// through one argv builder, so there is exactly one argv shape and one selector mapping per side.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoreSelector {
    pub backend: Option<String>,
    pub db: Option<String>,
}

impl StoreSelector {
    pub fn new(backend: Option<String>, db: Option<String>) -> Self {
        Self {
            backend: backend.filter(|s| !s.is_empty()),
            db: db.filter(|s| !s.is_empty()),
        }
    }
}

/// Build the on-deliver push handler argv.
///
/// Pure and dependency-free: no config, no environment, no filesystem. The only caller-supplied
/// parameter is `session_id`, validated here as well as at descriptor load, so a tampered manifest
/// cannot introduce a flag or a path. `--daemon-instance` is appended **only** here, which is what
/// guarantees the attach path and the daemon-side restore path cannot diverge on the fence flag.
pub fn build_push_argv(
    exe: &Path,
    selector: &StoreSelector,
    session_id: &str,
    instance_id: &str,
) -> Result<Vec<String>> {
    validate_session_id(session_id)
        .map_err(|e| RegistryError::InvalidParameter(format!("session id: {e}")))?;
    validate_instance_id(instance_id)?;
    let mut argv = vec![exe.to_string_lossy().to_string()];
    if let Some(backend) = selector.backend.as_deref().filter(|s| !s.is_empty()) {
        argv.push("--backend".to_string());
        argv.push(backend.to_string());
    }
    if let Some(db) = selector.db.as_deref().filter(|s| !s.is_empty()) {
        argv.push("--db".to_string());
        argv.push(db.to_string());
    }
    argv.push("copilot".to_string());
    argv.push("push".to_string());
    argv.push("--session".to_string());
    argv.push(session_id.to_string());
    // Epoch fencing (issue #106 / decision 8): the helper re-reads the daemon cap file immediately
    // before injecting and aborts if the instance changed, so a helper spawned by a dying daemon
    // cannot inject into a session its successor now owns.
    argv.push("--daemon-instance".to_string());
    argv.push(instance_id.to_string());
    Ok(argv)
}

fn validate_instance_id(instance_id: &str) -> Result<()> {
    if instance_id.is_empty() || instance_id.len() > 128 {
        return Err(RegistryError::InvalidParameter(format!(
            "daemon instance id must be 1..=128 characters, got {}",
            instance_id.len()
        )));
    }
    // A leading `-` would make the value parse as a flag rather than a value if argv were ever
    // reordered or re-split, so it is refused outright rather than escaped.
    if instance_id.starts_with('-') {
        return Err(RegistryError::InvalidParameter(
            "daemon instance id must not start with '-'".to_string(),
        ));
    }
    if !instance_id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(RegistryError::InvalidParameter(
            "daemon instance id may only contain ASCII alphanumerics, '-' and '_'".to_string(),
        ));
    }
    Ok(())
}

/// A registered handler kind: the daemon knows only that this kind is trusted and how to turn its
/// single `session_id` parameter into argv.
#[derive(Clone)]
pub struct HandlerKind {
    pub id: &'static str,
}

#[derive(Debug, Clone)]
pub struct ProducerRoot {
    pub id: String,
    pub path: PathBuf,
}

#[derive(Default)]
struct Registry {
    handler_kinds: BTreeMap<String, HandlerKind>,
    producer_roots: BTreeMap<String, ProducerRoot>,
}

fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(Registry::default()))
}

/// Register a handler kind at composition time. Idempotent.
pub fn register_handler_kind(kind: HandlerKind) {
    registry()
        .lock()
        .expect("handler kind registry")
        .handler_kinds
        .insert(kind.id.to_string(), kind);
}

/// Register a producer root at composition time. Idempotent; a later registration replaces an
/// earlier one for the same id (the harness resolves the path from the user's home directory,
/// which is stable within a process).
pub fn register_producer_root(id: impl Into<String>, path: impl Into<PathBuf>) {
    let root = ProducerRoot {
        id: id.into(),
        path: path.into(),
    };
    registry()
        .lock()
        .expect("handler kind registry")
        .producer_roots
        .insert(root.id.clone(), root);
}

pub fn handler_kind_registered(id: &str) -> bool {
    registry()
        .lock()
        .expect("handler kind registry")
        .handler_kinds
        .contains_key(id)
}

pub fn producer_root(id: &str) -> Option<ProducerRoot> {
    registry()
        .lock()
        .expect("handler kind registry")
        .producer_roots
        .get(id)
        .cloned()
}

/// Resolve a credential descriptor's `(root_id, path)` pair to a canonical path proven to live
/// strictly under the registered root.
///
/// Every failure mode is explicit and fail-closed: an unregistered root, a path that escapes the
/// root, a `..` component, and any symlink or reparse point on the resolved chain are all refused
/// without touching the file's contents.
pub fn resolve_credential_path(root_id: &str, path: &Path) -> Result<PathBuf> {
    let root = producer_root(root_id)
        .ok_or_else(|| RegistryError::UnknownProducerRoot(root_id.to_string()))?;
    crate::platform_fs::contained_under(&root.path, path)
        .map_err(|e| RegistryError::Containment(e.to_string()))
}

/// Rebuild the argv for a registered handler kind.
///
/// The daemon calls this with *its own* executable and *its own* store resolution; nothing about
/// the command line comes from the persisted manifest except the validated session id.
pub fn build_handler_argv(
    kind: &str,
    exe: &Path,
    selector: &StoreSelector,
    session_id: &str,
    instance_id: &str,
) -> Result<Vec<String>> {
    if !handler_kind_registered(kind) {
        return Err(RegistryError::UnknownHandlerKind(kind.to_string()));
    }
    build_push_argv(exe, selector, session_id, instance_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argv_shape_is_stable_and_always_carries_the_fence_flag() {
        let exe = PathBuf::from("/opt/telex/versions/v1/telex");
        let argv = build_push_argv(&exe, &StoreSelector::default(), "sess-1", "inst-1")
            .expect("default selector");
        assert_eq!(
            argv,
            vec![
                exe.to_string_lossy().to_string(),
                "copilot".to_string(),
                "push".to_string(),
                "--session".to_string(),
                "sess-1".to_string(),
                "--daemon-instance".to_string(),
                "inst-1".to_string(),
            ]
        );
        assert!(argv.contains(&"--daemon-instance".to_string()));

        let selected = build_push_argv(
            &exe,
            &StoreSelector::new(Some("prod".into()), Some("/tmp/x.db".into())),
            "sess-1",
            "inst-1",
        )
        .expect("named selector");
        assert_eq!(
            selected,
            vec![
                exe.to_string_lossy().to_string(),
                "--backend".to_string(),
                "prod".to_string(),
                "--db".to_string(),
                "/tmp/x.db".to_string(),
                "copilot".to_string(),
                "push".to_string(),
                "--session".to_string(),
                "sess-1".to_string(),
                "--daemon-instance".to_string(),
                "inst-1".to_string(),
            ]
        );
    }

    #[test]
    fn argv_refuses_injected_parameters() {
        let exe = PathBuf::from("telex");
        assert!(build_push_argv(&exe, &StoreSelector::default(), "--backend evil", "i").is_err());
        assert!(build_push_argv(&exe, &StoreSelector::default(), "a/../b", "i").is_err());
        assert!(build_push_argv(&exe, &StoreSelector::default(), "", "i").is_err());
        assert!(build_push_argv(&exe, &StoreSelector::default(), "ok", "--evil").is_err());
        assert!(build_push_argv(&exe, &StoreSelector::default(), "--evil", "i").is_err());
        assert!(build_push_argv(&exe, &StoreSelector::default(), "ok", "").is_err());
    }

    #[test]
    fn unregistered_kinds_and_roots_never_resolve() {
        assert!(matches!(
            build_handler_argv(
                "totally_unregistered_kind_v9",
                Path::new("telex"),
                &StoreSelector::default(),
                "sess",
                "inst"
            ),
            Err(RegistryError::UnknownHandlerKind(_))
        ));
        assert!(matches!(
            resolve_credential_path("unregistered_root_v9", Path::new("/etc/passwd")),
            Err(RegistryError::UnknownProducerRoot(_))
        ));
    }

    #[test]
    fn credential_paths_must_live_under_the_registered_root() {
        let base = std::env::temp_dir().join(format!(
            "telex-root-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        let root = base.join("root");
        let outside = base.join("outside");
        std::fs::create_dir_all(&root).expect("root");
        std::fs::create_dir_all(&outside).expect("outside");
        let inside_file = root.join("registry.json");
        let outside_file = outside.join("registry.json");
        std::fs::write(&inside_file, b"{}").expect("inside");
        std::fs::write(&outside_file, b"{}").expect("outside file");

        let root_id = format!("test_root_{}", std::process::id());
        register_producer_root(root_id.clone(), &root);
        assert!(resolve_credential_path(&root_id, &inside_file).is_ok());
        assert!(matches!(
            resolve_credential_path(&root_id, &outside_file),
            Err(RegistryError::Containment(_))
        ));
        assert!(matches!(
            resolve_credential_path(&root_id, &root.join("..").join("outside/registry.json")),
            Err(RegistryError::Containment(_))
        ));
        let _ = std::fs::remove_dir_all(&base);
    }
}
