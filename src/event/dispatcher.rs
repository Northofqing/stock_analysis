//! Registered business rules: BR-043, BR-051, BR-091, BR-111, BR-130, BR-141, BR-142, BR-144, BR-192.
//! Exact-match dispatcher registry — v17.1-r2 Task 3
//!
//! Provides a `Dispatcher` trait, `DispatcherRegistry` with exact-match routing,
//! and `AuditDispatcher` for observing `push.delivery.audit` without producing side-effects.

use std::collections::HashSet;
use std::ffi::{CString, OsStr};
#[cfg(test)]
use std::fs;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use thiserror::Error;

use super::envelope::EventEnvelope;

const DELIVERY_AUDIT_RECORD_HASH_DOMAIN: &str = "stock_analysis.delivery_audit_record.v2";
const PRODUCTION_AUDIT_DIR: &str = "data/event_audit";
const AUDIT_O_RDONLY: i32 = 0;
const AUDIT_O_RDWR: i32 = 2;
const AUDIT_O_APPEND: i32 = 0x0008;
const AUDIT_O_CREAT: i32 = 0x0200;
const AUDIT_O_EXCL: i32 = 0x0800;
#[cfg(target_os = "macos")]
const AUDIT_O_CLOEXEC: i32 = 0x0100_0000;
#[cfg(target_os = "macos")]
const AUDIT_O_DIRECTORY: i32 = 0x0010_0000;
#[cfg(target_os = "macos")]
const AUDIT_O_NOFOLLOW: i32 = 0x0100;
#[cfg(target_os = "linux")]
const AUDIT_O_CLOEXEC: i32 = 0x0008_0000;
#[cfg(target_os = "linux")]
const AUDIT_O_DIRECTORY: i32 = 0x0001_0000;
#[cfg(target_os = "linux")]
const AUDIT_O_NOFOLLOW: i32 = 0x0002_0000;

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!(
    "BR-192 delivery audit requires openat/mkdirat/flock Unix semantics; \
     this target must implement and test an equivalent retained-capability boundary"
);

unsafe extern "C" {
    fn openat(dirfd: i32, path: *const std::os::raw::c_char, flags: i32, ...) -> i32;
    fn mkdirat(dirfd: i32, path: *const std::os::raw::c_char, mode: u32) -> i32;
    fn geteuid() -> u32;
}

// ========================================================================
// DispatchResult
// ========================================================================

/// Result of a dispatcher handling an envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchResult {
    /// The dispatcher handled the event.
    Handled,
    /// No dispatcher was registered for this event type.
    Skipped(String),
    /// The dispatcher encountered a failure.
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditHealth {
    Unverified,
    Healthy,
    Degraded { reason_code: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditPreflightReceipt {
    pub year: i32,
    pub previous_hash: Option<String>,
}

// ========================================================================
// RegistryError
// ========================================================================

/// Errors from `DispatcherRegistry::validate`.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    #[error("duplicate event_type registered: {0}")]
    DuplicateEventType(String),
}

// ========================================================================
// Dispatcher trait
// ========================================================================

/// Trait implemented by event handlers that can be registered in the registry.
///
/// Each dispatcher handles one specific `event_type` and is selected by exact
/// equality — NOT prefix matching.
pub trait Dispatcher: Send + Sync {
    /// Human-readable name of this dispatcher.
    fn name(&self) -> &'static str;

    /// The event type this dispatcher handles, e.g. `"push.delivery.audit"`.
    fn event_type(&self) -> &'static str;

    /// Returns true when this dispatcher can handle the given envelope.
    ///
    /// The default implementation uses exact equality on `event_type`.
    fn accepts(&self, envelope: &EventEnvelope) -> bool {
        self.event_type() == envelope.event_type
    }

    /// Handle the envelope.
    fn dispatch(&self, envelope: EventEnvelope) -> DispatchResult;
}

// ========================================================================
// DispatcherRegistry
// ========================================================================

/// A registry of dispatchers selected by exact `event_type` match.
///
/// Iteration order is registration order; the first dispatcher whose
/// `event_type` matches is used.
#[derive(Default)]
pub struct DispatcherRegistry {
    dispatchers: Vec<Arc<dyn Dispatcher>>,
}

impl DispatcherRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            dispatchers: Vec::new(),
        }
    }

    /// Register a dispatcher.
    ///
    /// Duplicates are not rejected immediately — call `validate()` to check.
    pub fn register(&mut self, dispatcher: Arc<dyn Dispatcher>) {
        self.dispatchers.push(dispatcher);
    }

    /// Validate that no two dispatchers share the same `event_type`.
    ///
    /// # Errors
    ///
    /// Returns `RegistryError::DuplicateEventType` if a duplicate is found.
    pub fn validate(&self) -> Result<(), RegistryError> {
        let mut seen: HashSet<&'static str> = HashSet::new();
        for d in &self.dispatchers {
            let et = d.event_type();
            if !seen.insert(et) {
                return Err(RegistryError::DuplicateEventType(et.to_string()));
            }
        }
        Ok(())
    }

    /// Dispatch an envelope to the first registered handler with a matching
    /// `event_type`.
    ///
    /// Returns `DispatchResult::Skipped("no_dispatcher")` when no handler
    /// matches.
    pub fn dispatch(&self, envelope: EventEnvelope) -> DispatchResult {
        for d in &self.dispatchers {
            if d.accepts(&envelope) {
                return d.dispatch(envelope);
            }
        }
        DispatchResult::Skipped("no_dispatcher".into())
    }
}

// ========================================================================
// AuditDispatcher
// ========================================================================

/// BR-091 durable audit dispatcher for `push.delivery.audit`.
#[derive(Debug)]
pub struct AuditDispatcher {
    handled_count: AtomicU64,
    base_dir: PathBuf,
    capability: std::result::Result<Arc<PinnedAuditRoot>, String>,
    chain_state: Mutex<AuditChainState>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AuditObjectIdentity {
    device: u64,
    inode: u64,
    mode: u32,
    uid: u32,
    links: u64,
    is_directory: bool,
    is_file: bool,
}

#[derive(Debug)]
struct RetainedAuditDirectory {
    path: PathBuf,
    file: File,
    identity: AuditObjectIdentity,
}

#[derive(Debug)]
struct PinnedAuditRoot {
    directories: Vec<RetainedAuditDirectory>,
    root_path: PathBuf,
}

impl PinnedAuditRoot {
    fn root(&self) -> &File {
        &self
            .directories
            .last()
            .expect("pinned audit root has at least slash")
            .file
    }

    fn validate_complete_chain(&self) -> Result<(), String> {
        for directory in &self.directories {
            let current = audit_identity(&directory.file, &directory.path)?;
            validate_audit_directory_identity(&current, &directory.path)?;
            if !same_audit_directory_object(&current, &directory.identity) {
                return Err(format!(
                    "BR-192 audit ancestor identity changed: {}",
                    directory.path.display()
                ));
            }
        }
        let rebound = bind_absolute_audit_root(&self.root_path, None)?;
        if rebound.directories.len() != self.directories.len()
            || rebound
                .directories
                .iter()
                .zip(&self.directories)
                .any(|(current, retained)| {
                    !same_audit_directory_object(&current.identity, &retained.identity)
                })
        {
            return Err(format!(
                "BR-192 audit namespace rebind changed: {}",
                self.root_path.display()
            ));
        }
        Ok(())
    }
}

#[derive(Debug)]
struct AuditChainState {
    poisoned: Option<String>,
    health: AuditHealth,
}

impl AuditDispatcher {
    /// Bind one of the two fixed BR-192 authorities. Arbitrary caller paths are
    /// retained only as an explicit failed capability for legacy call sites;
    /// they can never become a writable audit authority.
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        let base_dir = base_dir.into();
        let capability = classify_and_bind_audit_root(&base_dir).map(Arc::new);
        Self {
            handled_count: AtomicU64::new(0),
            base_dir,
            capability,
            chain_state: Mutex::new(AuditChainState {
                poisoned: None,
                health: AuditHealth::Unverified,
            }),
        }
    }

    pub fn for_production() -> Result<Self, String> {
        let base_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(PRODUCTION_AUDIT_DIR);
        let dispatcher = Self::new(base_dir);
        dispatcher.capability.as_ref().map_err(Clone::clone)?;
        Ok(dispatcher)
    }

    pub fn for_test_code(test_code: &str) -> Result<Self, String> {
        validate_audit_test_code(test_code)?;
        let base_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("data/test")
            .join(test_code)
            .join("event_audit");
        let dispatcher = Self::new(base_dir);
        dispatcher.capability.as_ref().map_err(Clone::clone)?;
        Ok(dispatcher)
    }

    /// Runtime constructor. Environment/CWD/path overrides never select the
    /// audit root. An invalid runtime namespace produces a fail-closed
    /// dispatcher whose every preflight/dispatch returns the captured error.
    pub fn for_runtime() -> Self {
        let result = if std::env::var_os("EVENT_AUDIT_DIR").is_some() {
            Err("BR-192 EVENT_AUDIT_DIR override is forbidden".to_owned())
        } else if crate::risk::env_guard::current_env() == crate::risk::env_guard::TradingEnv::Test
            || crate::risk::env_guard::runtime_is_test_process()
        {
            std::env::var("DURABLE_DELIVERY_TEST_CODE")
                .map_err(|_| {
                    "BR-192 test delivery audit requires DURABLE_DELIVERY_TEST_CODE".to_owned()
                })
                .and_then(|test_code| Self::for_test_code(&test_code))
        } else {
            Self::for_production()
        };
        result.unwrap_or_else(Self::failed)
    }

    fn failed(error: String) -> Self {
        Self {
            handled_count: AtomicU64::new(0),
            base_dir: PathBuf::new(),
            capability: Err(error.clone()),
            chain_state: Mutex::new(AuditChainState {
                poisoned: Some(error.clone()),
                health: AuditHealth::Degraded { reason_code: error },
            }),
        }
    }

    /// Returns the number of envelopes this dispatcher has handled.
    pub fn handled_count(&self) -> u64 {
        self.handled_count.load(Ordering::SeqCst)
    }

    pub fn health(&self) -> AuditHealth {
        self.chain_state
            .lock()
            .map(|s| s.health.clone())
            .unwrap_or(AuditHealth::Degraded {
                reason_code: "state_lock_poisoned".into(),
            })
    }

    pub fn preflight(&self) -> Result<AuditPreflightReceipt, String> {
        self.preflight_inner(false)
    }
    pub fn recover_with_canary(&self) -> Result<AuditPreflightReceipt, String> {
        self.preflight_inner(true)
    }

    fn preflight_inner(&self, recovery: bool) -> Result<AuditPreflightReceipt, String> {
        use fs2::FileExt;
        let year = chrono::Local::now().format("%Y").to_string();
        let result: Result<AuditPreflightReceipt, String> = (|| {
            let capability = self.capability.as_ref().map_err(Clone::clone)?;
            let _process_guard = audit_process_mutex()
                .lock()
                .map_err(|_| "audit process mutex poisoned".to_owned())?;
            capability.validate_complete_chain()?;
            let lock_name = format!("{year}.lock");
            let (lock, lock_identity) =
                open_or_create_audit_file(capability, OsStr::new(&lock_name), false)?;
            FileExt::lock_exclusive(&lock).map_err(|e| format!("lock audit: {e}"))?;
            capability.validate_complete_chain()?;
            revalidate_audit_leaf(capability, OsStr::new(&lock_name), &lock_identity)?;
            let json_name = format!("{year}.jsonl");
            let (jsonl, json_identity) =
                open_or_create_audit_file(capability, OsStr::new(&json_name), true)?;
            let previous_hash =
                validate_existing_chain_file(&jsonl, &self.base_dir.join(&json_name))?;
            jsonl.sync_all().map_err(|error| {
                format!("sync audit preflight {}: {error}", self.base_dir.display())
            })?;
            capability
                .root()
                .sync_all()
                .map_err(|error| format!("sync audit root {}: {error}", self.base_dir.display()))?;
            capability.validate_complete_chain()?;
            revalidate_audit_leaf(capability, OsStr::new(&lock_name), &lock_identity)?;
            revalidate_audit_leaf(capability, OsStr::new(&json_name), &json_identity)?;
            FileExt::unlock(&lock).map_err(|error| format!("unlock audit: {error}"))?;
            Ok(AuditPreflightReceipt {
                year: year.parse().unwrap_or_default(),
                previous_hash,
            })
        })();
        match result {
            Ok(receipt) => {
                let mut state = self
                    .chain_state
                    .lock()
                    .map_err(|_| "audit chain state lock poisoned".to_string())?;
                if recovery || !matches!(state.health, AuditHealth::Degraded { .. }) {
                    state.health = AuditHealth::Healthy;
                    state.poisoned = None;
                }
                Ok(receipt)
            }
            Err(error) => {
                if let Ok(mut s) = self.chain_state.lock() {
                    s.health = AuditHealth::Degraded {
                        reason_code: error.clone(),
                    };
                }
                Err(error)
            }
        }
    }

    fn persist(&self, envelope: &EventEnvelope) -> Result<(), String> {
        use fs2::FileExt;
        let year = envelope.ts.format("%Y").to_string();
        let mut state = self
            .chain_state
            .lock()
            .map_err(|_| "audit chain state lock poisoned".to_string())?;
        if let Some(reason) = state.poisoned.as_deref() {
            return Err(format!(
                "audit chain is poisoned after an earlier persistence failure: {reason}"
            ));
        }

        let persistence_result = (|| -> Result<(), String> {
            let capability = self.capability.as_ref().map_err(Clone::clone)?;
            let _process_guard = audit_process_mutex()
                .lock()
                .map_err(|_| "audit process mutex poisoned".to_owned())?;
            capability.validate_complete_chain()?;
            let lock_name = format!("{year}.lock");
            let (lock_file, lock_identity) =
                open_or_create_audit_file(capability, OsStr::new(&lock_name), false)?;
            FileExt::lock_exclusive(&lock_file)
                .map_err(|error| format!("lock audit {lock_name}: {error}"))?;
            capability.validate_complete_chain()?;
            revalidate_audit_leaf(capability, OsStr::new(&lock_name), &lock_identity)?;

            // The kernel lock spans full-chain validation, append and fsync.
            // Revalidate on every append because another monitor process may
            // have extended the chain since this dispatcher last wrote.
            let json_name = format!("{year}.jsonl");
            let path = self.base_dir.join(&json_name);
            let (mut file, json_identity) =
                open_or_create_audit_file(capability, OsStr::new(&json_name), true)?;
            let previous_hash = validate_existing_chain_file(&file, &path)?
                .unwrap_or_else(|| "GENESIS".to_string());
            let mut record = serde_json::json!({
                "envelope": envelope,
                "hash_domain": DELIVERY_AUDIT_RECORD_HASH_DOMAIN,
                "previous_hash": previous_hash,
            });
            let record_hash = calculate_record_hash(&record)?;
            record.as_object_mut().expect("json object literal").insert(
                "record_hash".to_string(),
                serde_json::Value::String(record_hash.clone()),
            );
            let mut line = serde_json::to_vec(&record)
                .map_err(|error| format!("serialize audit line: {error}"))?;
            line.push(b'\n');

            file.write_all(&line)
                .map_err(|error| format!("append {}: {error}", path.display()))?;
            file.flush()
                .map_err(|error| format!("flush {}: {error}", path.display()))?;
            file.sync_all()
                .map_err(|error| format!("sync {}: {error}", path.display()))?;
            capability
                .root()
                .sync_all()
                .map_err(|error| format!("sync audit root {}: {error}", self.base_dir.display()))?;
            capability.validate_complete_chain()?;
            revalidate_audit_leaf(capability, OsStr::new(&lock_name), &lock_identity)?;
            revalidate_audit_leaf(capability, OsStr::new(&json_name), &json_identity)?;
            FileExt::unlock(&lock_file)
                .map_err(|error| format!("unlock audit {lock_name}: {error}"))?;
            Ok(())
        })();
        match persistence_result {
            Ok(()) => Ok(()),
            Err(error) => {
                state.poisoned = Some(error.clone());
                Err(error)
            }
        }
    }

    /// Re-open the retained BR-192 authority and prove that exactly one
    /// byte-equivalent schema-v3 counted audit exists for `expected`.
    ///
    /// This verifier never creates a lock or JSONL file. Missing, duplicated,
    /// tampered, legacy or merely same-ID records all fail closed.
    pub fn verify_exact_counted_event(
        &self,
        expected: &EventEnvelope,
    ) -> Result<super::push_record::PushRecord, String> {
        use fs2::FileExt;

        let expected_record = super::push_record::PushRecord::try_from_authoritative(expected)
            .map_err(|error| {
                format!("BR-192 terminal verifier rejected expected audit: {error}")
            })?;
        if expected_record.audit_schema_version
            != Some(super::envelope::COUNTED_DELIVERY_AUDIT_SCHEMA_VERSION)
        {
            return Err("BR-192 terminal verifier requires schema-v3 counted audit".to_owned());
        }
        let expected_value = serde_json::to_value(expected)
            .map_err(|error| format!("serialize expected counted audit: {error}"))?;
        let year = expected.ts.format("%Y").to_string();
        let capability = self.capability.as_ref().map_err(Clone::clone)?;
        let _process_guard = audit_process_mutex()
            .lock()
            .map_err(|_| "audit process mutex poisoned".to_owned())?;
        capability.validate_complete_chain()?;

        let lock_name = format!("{year}.lock");
        let (lock, lock_identity) =
            open_existing_audit_file(capability, OsStr::new(&lock_name), false)?;
        FileExt::lock_exclusive(&lock)
            .map_err(|error| format!("lock counted audit verifier {lock_name}: {error}"))?;
        capability.validate_complete_chain()?;
        revalidate_audit_leaf(capability, OsStr::new(&lock_name), &lock_identity)?;

        let json_name = format!("{year}.jsonl");
        let path = self.base_dir.join(&json_name);
        let (jsonl, json_identity) =
            open_existing_audit_file(capability, OsStr::new(&json_name), false)?;
        validate_existing_chain_file(&jsonl, &path)?;

        let mut reader = jsonl
            .try_clone()
            .map_err(|error| format!("clone counted audit verifier {}: {error}", path.display()))?;
        reader
            .seek(SeekFrom::Start(0))
            .map_err(|error| format!("seek counted audit verifier {}: {error}", path.display()))?;
        let mut content = String::new();
        reader
            .read_to_string(&mut content)
            .map_err(|error| format!("read counted audit verifier {}: {error}", path.display()))?;
        let mut exact_matches = 0_u32;
        let mut same_id_mismatches = 0_u32;
        for (index, line) in content.lines().enumerate() {
            let record: serde_json::Value = serde_json::from_str(line).map_err(|error| {
                format!("parse counted audit verifier line {}: {error}", index + 1)
            })?;
            let Some(envelope_value) = record.get("envelope") else {
                return Err(format!(
                    "counted audit verifier line {} has no envelope",
                    index + 1
                ));
            };
            if envelope_value.get("id").and_then(serde_json::Value::as_str)
                == Some(expected.id.as_str())
            {
                if envelope_value == &expected_value {
                    exact_matches += 1;
                } else {
                    same_id_mismatches += 1;
                }
            }
        }
        capability.validate_complete_chain()?;
        revalidate_audit_leaf(capability, OsStr::new(&lock_name), &lock_identity)?;
        revalidate_audit_leaf(capability, OsStr::new(&json_name), &json_identity)?;
        FileExt::unlock(&lock)
            .map_err(|error| format!("unlock counted audit verifier {lock_name}: {error}"))?;

        if exact_matches != 1 || same_id_mismatches != 0 {
            return Err(format!(
                "BR-192 counted audit terminal cardinality mismatch: event_id={} exact={} mismatched={}",
                expected.id, exact_matches, same_id_mismatches
            ));
        }
        Ok(expected_record)
    }
}

fn audit_process_mutex() -> &'static Mutex<()> {
    static PROCESS_MUTEX: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();
    PROCESS_MUTEX.get_or_init(|| Mutex::new(()))
}

fn validate_audit_test_code(test_code: &str) -> Result<(), String> {
    if !test_code.starts_with("TEST_CODE")
        || test_code.is_empty()
        || !test_code
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(
            "BR-192 audit test identity must be one path-safe TEST_CODE component".to_owned(),
        );
    }
    Ok(())
}

fn classify_and_bind_audit_root(path: &Path) -> Result<PinnedAuditRoot, String> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    if path == manifest.join(PRODUCTION_AUDIT_DIR) {
        return bind_absolute_audit_root(path, Some(&manifest.join("data")));
    }
    let test_parent = manifest.join("data/test");
    let relative = path.strip_prefix(&test_parent).map_err(|_| {
        format!(
            "BR-192 audit path is not a fixed production/TEST_CODE authority: {}",
            path.display()
        )
    })?;
    let mut components = relative.components();
    let test_code = match (components.next(), components.next(), components.next()) {
        (
            Some(std::path::Component::Normal(test_code)),
            Some(std::path::Component::Normal(boundary)),
            None,
        ) if boundary == OsStr::new("event_audit") => test_code.to_string_lossy().into_owned(),
        _ => {
            return Err(format!(
                "BR-192 audit path is not exact data/test/TEST_CODE*/event_audit: {}",
                path.display()
            ))
        }
    };
    validate_audit_test_code(&test_code)?;
    // `data/` is the immutable manifest-anchored authority.  Allow the
    // no-follow descriptor walk to create `test/TEST_CODE*/event_audit`
    // itself so a fresh checkout does not depend on a pre-created `data/test`
    // directory.  Every component below `data/` is still owner/mode/link
    // validated before it becomes authority.
    bind_absolute_audit_root(path, Some(&manifest.join("data")))
}

fn bind_absolute_audit_root(
    path: &Path,
    creation_boundary: Option<&Path>,
) -> Result<PinnedAuditRoot, String> {
    use std::os::unix::fs::MetadataExt;
    use std::path::Component;

    if !path.is_absolute() {
        return Err(format!(
            "BR-192 audit root must be manifest-anchored absolute: {}",
            path.display()
        ));
    }
    let components = path
        .components()
        .filter_map(|component| match component {
            Component::RootDir => None,
            Component::Normal(value) => Some(value.to_os_string()),
            _ => None,
        })
        .collect::<Vec<_>>();
    if components.is_empty() {
        return Err("BR-192 audit root cannot be filesystem root".to_owned());
    }
    let creation_depth = creation_boundary
        .map(|boundary| {
            if !boundary.is_absolute() || !path.starts_with(boundary) || path == boundary {
                return Err(format!(
                    "BR-192 audit creation boundary does not contain root: boundary={} root={}",
                    boundary.display(),
                    path.display()
                ));
            }
            Ok(boundary
                .components()
                .filter(|component| matches!(component, Component::Normal(_)))
                .count())
        })
        .transpose()?;
    let slash = OpenOptions::new()
        .read(true)
        .open("/")
        .map_err(|error| format!("open audit filesystem root: {error}"))?;
    let slash_metadata = slash
        .metadata()
        .map_err(|error| format!("inspect audit filesystem root: {error}"))?;
    let slash_identity = AuditObjectIdentity {
        device: slash_metadata.dev(),
        inode: slash_metadata.ino(),
        mode: slash_metadata.mode(),
        uid: slash_metadata.uid(),
        links: slash_metadata.nlink(),
        is_directory: slash_metadata.is_dir(),
        is_file: slash_metadata.is_file(),
    };
    validate_audit_directory_identity(&slash_identity, Path::new("/"))?;
    let mut directories = vec![RetainedAuditDirectory {
        path: PathBuf::from("/"),
        file: slash,
        identity: slash_identity,
    }];
    let mut current_path = PathBuf::from("/");
    for (index, component) in components.iter().enumerate() {
        current_path.push(component);
        let parent = &directories
            .last()
            .expect("audit traversal always retains slash")
            .file;
        let is_leaf = index + 1 == components.len();
        let may_create = creation_depth.is_some_and(|depth| index >= depth);
        let directory = match audit_openat(parent, component, AUDIT_O_RDONLY | AUDIT_O_DIRECTORY, 0)
        {
            Ok(directory) => directory,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && may_create => {
                let c_component = audit_component(component)?;
                // SAFETY: parent is a retained directory fd and component is one
                // validated NUL-free path component.
                let created =
                    unsafe { mkdirat(parent.as_raw_fd(), c_component.as_ptr(), 0o700_u32) };
                if created < 0 {
                    let error = std::io::Error::last_os_error();
                    if error.kind() != std::io::ErrorKind::AlreadyExists {
                        return Err(format!(
                            "create fixed audit root {}: {error}",
                            current_path.display()
                        ));
                    }
                }
                parent.sync_all().map_err(|error| {
                    format!(
                        "fsync audit parent after create/EEXIST {}: {error}",
                        current_path.display()
                    )
                })?;
                audit_openat(parent, component, AUDIT_O_RDONLY | AUDIT_O_DIRECTORY, 0).map_err(
                    |error| {
                        format!(
                            "open fixed audit directory {}: {error}",
                            current_path.display()
                        )
                    },
                )?
            }
            Err(error) => {
                return Err(format!(
                    "open fixed audit directory {}: {error}",
                    current_path.display()
                ))
            }
        };
        let identity = audit_identity(&directory, &current_path)?;
        validate_audit_directory_identity(&identity, &current_path)?;
        if (is_leaf || may_create) && identity.uid != unsafe { geteuid() } {
            return Err(format!(
                "writable audit authority component is not owned by effective uid: {}",
                current_path.display()
            ));
        }
        directories.push(RetainedAuditDirectory {
            path: current_path.clone(),
            file: directory,
            identity,
        });
    }
    Ok(PinnedAuditRoot {
        directories,
        root_path: path.to_path_buf(),
    })
}

fn validate_audit_directory_identity(
    identity: &AuditObjectIdentity,
    path: &Path,
) -> Result<(), String> {
    let euid = unsafe { geteuid() };
    if !identity.is_directory {
        return Err(format!(
            "audit namespace component is not a directory: {}",
            path.display()
        ));
    }
    if identity.links == 0 {
        return Err(format!(
            "audit namespace component has no physical links: {}",
            path.display()
        ));
    }
    if !audit_directory_owner_allowed(identity.uid, euid) {
        return Err(format!(
            "audit namespace component owner uid={} is neither root nor effective uid={euid}: {}",
            identity.uid,
            path.display()
        ));
    }
    if identity.mode & 0o022 != 0 {
        return Err(format!(
            "audit namespace component is group/other writable mode={:o}: {}",
            identity.mode & 0o7777,
            path.display()
        ));
    }
    Ok(())
}

fn audit_directory_owner_allowed(uid: u32, effective_uid: u32) -> bool {
    uid == 0 || uid == effective_uid
}

fn same_audit_directory_object(
    current: &AuditObjectIdentity,
    retained: &AuditObjectIdentity,
) -> bool {
    current.device == retained.device
        && current.inode == retained.inode
        && current.mode == retained.mode
        && current.uid == retained.uid
        && current.is_directory
        && retained.is_directory
        && !current.is_file
        && !retained.is_file
}

fn audit_component(component: &OsStr) -> Result<CString, String> {
    if component.is_empty() || component.as_bytes().contains(&b'/') {
        return Err("audit path must use one non-empty component".to_owned());
    }
    CString::new(component.as_bytes()).map_err(|_| "audit path component contains NUL".to_owned())
}

fn audit_openat(parent: &File, component: &OsStr, flags: i32, mode: u32) -> std::io::Result<File> {
    let component = audit_component(component)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
    // SAFETY: parent is retained and component is one live C string.
    let fd = unsafe {
        openat(
            parent.as_raw_fd(),
            component.as_ptr(),
            flags | AUDIT_O_NOFOLLOW | AUDIT_O_CLOEXEC,
            mode,
        )
    };
    if fd < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        // SAFETY: successful openat returns one owned descriptor.
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

fn audit_identity(file: &File, path: &Path) -> Result<AuditObjectIdentity, String> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file
        .metadata()
        .map_err(|error| format!("inspect audit object {}: {error}", path.display()))?;
    Ok(AuditObjectIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        mode: metadata.mode(),
        uid: metadata.uid(),
        links: metadata.nlink(),
        is_directory: metadata.is_dir(),
        is_file: metadata.is_file(),
    })
}

fn open_or_create_audit_file(
    capability: &PinnedAuditRoot,
    name: &OsStr,
    append: bool,
) -> Result<(File, AuditObjectIdentity), String> {
    open_or_create_audit_file_with_hook(capability, name, append, || Ok(()))
}

fn open_or_create_audit_file_with_hook<F>(
    capability: &PinnedAuditRoot,
    name: &OsStr,
    append: bool,
    before_create: F,
) -> Result<(File, AuditObjectIdentity), String>
where
    F: FnOnce() -> Result<(), String>,
{
    let mut flags = AUDIT_O_RDWR;
    if append {
        flags |= AUDIT_O_APPEND;
    }
    let path = capability.root_path.join(name);
    let file = match audit_openat(capability.root(), name, flags, 0) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            before_create()?;
            match audit_openat(
                capability.root(),
                name,
                flags | AUDIT_O_CREAT | AUDIT_O_EXCL,
                0o600,
            ) {
                Ok(file) => {
                    capability.root().sync_all().map_err(|error| {
                        format!(
                            "fsync audit parent after create {}: {error}",
                            path.display()
                        )
                    })?;
                    file
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    capability.root().sync_all().map_err(|error| {
                        format!(
                            "fsync audit parent after EEXIST {}: {error}",
                            path.display()
                        )
                    })?;
                    audit_openat(capability.root(), name, flags, 0).map_err(|error| {
                        format!("open raced audit file {}: {error}", path.display())
                    })?
                }
                Err(error) => return Err(format!("create audit file {}: {error}", path.display())),
            }
        }
        Err(error) => return Err(format!("open audit file {}: {error}", path.display())),
    };
    let identity = audit_identity(&file, &path)?;
    let euid = unsafe { geteuid() };
    if !identity.is_file
        || identity.links != 1
        || identity.uid != euid
        || identity.mode & 0o022 != 0
    {
        return Err(format!(
            "audit file owner/mode/type/link validation failed: {}",
            path.display()
        ));
    }
    Ok((file, identity))
}

fn open_existing_audit_file(
    capability: &PinnedAuditRoot,
    name: &OsStr,
    append: bool,
) -> Result<(File, AuditObjectIdentity), String> {
    let mut flags = AUDIT_O_RDONLY;
    if append {
        flags = AUDIT_O_RDWR | AUDIT_O_APPEND;
    }
    let path = capability.root_path.join(name);
    let file = audit_openat(capability.root(), name, flags, 0)
        .map_err(|error| format!("open existing audit file {}: {error}", path.display()))?;
    let identity = audit_identity(&file, &path)?;
    let euid = unsafe { geteuid() };
    if !identity.is_file
        || identity.links != 1
        || identity.uid != euid
        || identity.mode & 0o022 != 0
    {
        return Err(format!(
            "existing audit file owner/mode/type/link validation failed: {}",
            path.display()
        ));
    }
    Ok((file, identity))
}

fn revalidate_audit_leaf(
    capability: &PinnedAuditRoot,
    name: &OsStr,
    expected: &AuditObjectIdentity,
) -> Result<(), String> {
    let path = capability.root_path.join(name);
    let rebound = audit_openat(capability.root(), name, AUDIT_O_RDONLY, 0)
        .map_err(|error| format!("re-open audit leaf {}: {error}", path.display()))?;
    let current = audit_identity(&rebound, &path)?;
    if &current != expected {
        return Err(format!(
            "BR-192 audit leaf identity changed: {}",
            path.display()
        ));
    }
    Ok(())
}

impl Default for AuditDispatcher {
    fn default() -> Self {
        Self::for_runtime()
    }
}

#[cfg(test)]
fn validate_existing_chain(path: &Path) -> Result<Option<String>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let file = File::open(path)
        .map_err(|error| format!("open existing audit {}: {error}", path.display()))?;
    validate_existing_chain_file(&file, path)
}

fn validate_existing_chain_file(file: &File, path: &Path) -> Result<Option<String>, String> {
    let mut reader = file
        .try_clone()
        .map_err(|error| format!("clone existing audit {}: {error}", path.display()))?;
    reader
        .seek(SeekFrom::Start(0))
        .map_err(|error| format!("seek existing audit {}: {error}", path.display()))?;
    let mut content = String::new();
    reader
        .read_to_string(&mut content)
        .map_err(|error| format!("read existing audit {}: {error}", path.display()))?;
    if !content.is_empty() && !content.ends_with('\n') {
        return Err(format!(
            "audit {} has an incomplete trailing record",
            path.display()
        ));
    }
    let mut expected_parent = "GENESIS".to_string();
    let mut last_hash = None;
    let mut saw_v2 = false;
    for (index, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            return Err(format!("audit line {} is blank", index + 1));
        }
        let mut record: serde_json::Value = serde_json::from_str(line)
            .map_err(|error| format!("parse audit line {}: {error}", index + 1))?;
        let record_hash = record
            .get("record_hash")
            .and_then(serde_json::Value::as_str)
            .filter(|hash| !hash.is_empty())
            .ok_or_else(|| format!("audit line {} has no record_hash", index + 1))?
            .to_string();
        let parent = record
            .get("previous_hash")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| format!("audit line {} has no previous_hash", index + 1))?;
        if parent != expected_parent {
            return Err(format!(
                "audit chain mismatch at line {}: expected {}, found {}",
                index + 1,
                expected_parent,
                parent
            ));
        }
        record
            .as_object_mut()
            .ok_or_else(|| format!("audit line {} is not an object", index + 1))?
            .remove("record_hash");
        let is_v2 = match record.get("hash_domain") {
            None if saw_v2 => {
                return Err(format!("legacy audit after v2 at line {}", index + 1));
            }
            None => false,
            Some(serde_json::Value::String(domain))
                if domain == DELIVERY_AUDIT_RECORD_HASH_DOMAIN =>
            {
                saw_v2 = true;
                true
            }
            Some(serde_json::Value::String(domain)) => {
                return Err(format!("unsupported audit hash domain: {domain}"));
            }
            Some(_) => return Err("audit hash_domain must be a string".into()),
        };
        validate_closed_object(
            &record,
            if is_v2 {
                &["envelope", "hash_domain", "previous_hash"]
            } else {
                &["envelope", "previous_hash"]
            },
            "audit record",
        )
        .map_err(|error| format!("audit line {}: {error}", index + 1))?;
        let calculated = calculate_record_hash(&record)
            .map_err(|error| format!("audit line {}: {error}", index + 1))?;
        if calculated != record_hash {
            return Err(format!("audit hash mismatch at line {}", index + 1));
        }

        let envelope_value = record
            .get("envelope")
            .ok_or_else(|| format!("audit line {} has no envelope", index + 1))?;
        validate_closed_object(
            envelope_value,
            &[
                "id",
                "ts",
                "trace_id",
                "source",
                "event_type",
                "entity_key",
                "payload",
                "version",
                "replay_of",
            ],
            if is_v2 {
                "v2 delivery envelope"
            } else {
                "legacy delivery envelope"
            },
        )
        .map_err(|error| format!("audit line {}: {error}", index + 1))?;
        let envelope: EventEnvelope = serde_json::from_value(envelope_value.clone())
            .map_err(|error| format!("parse audit envelope at line {}: {error}", index + 1))?;
        if is_v2 {
            super::push_record::PushRecord::try_from_authoritative(&envelope).map_err(|error| {
                format!("invalid v2 delivery audit at line {}: {error}", index + 1)
            })?;
        } else {
            super::push_record::PushRecord::try_from(&envelope).map_err(|error| {
                format!(
                    "invalid legacy delivery audit at line {}: {error}",
                    index + 1
                )
            })?;
        }
        expected_parent = record_hash.clone();
        last_hash = Some(record_hash);
    }
    Ok(last_hash)
}

fn validate_closed_object(
    value: &serde_json::Value,
    expected_fields: &[&str],
    context: &str,
) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{context} is not an object"))?;
    for field in object.keys() {
        if !expected_fields.contains(&field.as_str()) {
            return Err(format!("{context} has unknown field: {field}"));
        }
    }
    for field in expected_fields {
        if !object.contains_key(*field) {
            return Err(format!("{context} has no {field}"));
        }
    }
    Ok(())
}

fn calculate_record_hash(record: &serde_json::Value) -> Result<String, String> {
    use sha2::{Digest, Sha256};

    let canonical =
        serde_json::to_vec(record).map_err(|error| format!("serialize audit record: {error}"))?;
    let mut hasher = Sha256::new();
    match record.get("hash_domain") {
        None => {
            // Read-only compatibility for records emitted before BR-142.
        }
        Some(serde_json::Value::String(domain)) if domain == DELIVERY_AUDIT_RECORD_HASH_DOMAIN => {
            hasher.update(domain.as_bytes());
            hasher.update([0]);
        }
        Some(serde_json::Value::String(domain)) => {
            return Err(format!("unsupported audit hash domain: {domain}"));
        }
        Some(_) => return Err("audit hash_domain must be a string".into()),
    }
    hasher.update(canonical);
    Ok(format!("{:x}", hasher.finalize()))
}

impl Dispatcher for AuditDispatcher {
    fn name(&self) -> &'static str {
        "AuditDispatcher"
    }

    fn event_type(&self) -> &'static str {
        "push.delivery.audit"
    }

    fn dispatch(&self, envelope: EventEnvelope) -> DispatchResult {
        // Reject non-matching event types (supports direct dispatch testing).
        if !self.accepts(&envelope) {
            return DispatchResult::Skipped("no_dispatcher".into());
        }

        let record = match super::push_record::PushRecord::try_from_authoritative(&envelope) {
            Ok(record) => record,
            Err(error) => {
                return DispatchResult::Failed(format!("invalid delivery audit: {error}"));
            }
        };

        if let Err(error) = self.persist(&envelope) {
            return DispatchResult::Failed(error);
        }

        // Extract fields for operational logging after durable persistence.
        let id = &envelope.id;
        let event_type = &envelope.event_type;
        let source = &envelope.source;
        let outcome = envelope
            .payload
            .get("outcome")
            .and_then(|v| v.as_str())
            .expect("PushRecord validation requires string outcome");

        let identity_hash = record
            .identity_hash
            .as_deref()
            .expect("authoritative PushRecord validation requires identity_hash");
        println!(
            "[AuditDispatcher] id={} event_type={} source={} kind={} outcome={} channel={} identity_hash={}",
            id, event_type, source, record.kind, outcome, record.channel, identity_hash
        );

        self.handled_count.fetch_add(1, Ordering::SeqCst);
        DispatchResult::Handled
    }
}

#[cfg(test)]
pub(crate) struct TestAuditNamespace {
    test_code: String,
    root: PathBuf,
    audit_path: PathBuf,
    retained_root: File,
    root_identity: AuditObjectIdentity,
}

#[cfg(test)]
impl TestAuditNamespace {
    pub(crate) fn new(label: &str) -> Self {
        static NONCE: AtomicU64 = AtomicU64::new(0);
        let safe_label = label
            .bytes()
            .map(|byte| {
                if byte.is_ascii_alphanumeric() {
                    char::from(byte)
                } else {
                    '_'
                }
            })
            .collect::<String>();
        let test_code = format!(
            "TEST_CODE_EVENT_AUDIT_{}_{}_{}",
            safe_label,
            std::process::id(),
            NONCE.fetch_add(1, Ordering::SeqCst)
        );
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("data/test")
            .join(&test_code);
        fs::create_dir_all(&root).expect("create exact TEST_CODE audit fixture root");
        let retained_root = OpenOptions::new()
            .read(true)
            .open(&root)
            .expect("retain TEST_CODE audit fixture root");
        let root_identity =
            audit_identity(&retained_root, &root).expect("validate TEST_CODE audit fixture root");
        Self {
            audit_path: root.join("event_audit"),
            test_code,
            root,
            retained_root,
            root_identity,
        }
    }

    pub(crate) fn dispatcher(&self) -> AuditDispatcher {
        AuditDispatcher::for_test_code(&self.test_code)
            .expect("bind exact TEST_CODE audit authority")
    }

    pub(crate) fn audit_path(&self) -> &Path {
        &self.audit_path
    }

    pub(crate) fn test_code(&self) -> &str {
        &self.test_code
    }
}

#[cfg(test)]
impl Drop for TestAuditNamespace {
    fn drop(&mut self) {
        let observed = audit_identity(&self.retained_root, &self.root)
            .expect("revalidate exact TEST_CODE audit fixture before cleanup");
        validate_audit_directory_identity(&observed, &self.root)
            .expect("TEST_CODE audit fixture root remains a safe directory");
        assert!(
            same_audit_directory_object(&observed, &self.root_identity),
            "TEST_CODE audit fixture root identity changed before cleanup"
        );
        if self.root.exists() {
            fs::remove_dir_all(&self.root)
                .expect("remove exact retained TEST_CODE audit fixture root");
        }
    }
}

// ========================================================================
// Tests
// ========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::envelope::EventEnvelope;
    use std::os::unix::fs::PermissionsExt;

    /// A dispatcher that records every dispatch for inspection in tests.
    #[derive(Debug, Default)]
    struct RecordingDispatcher {
        event_type_: &'static str,
        name_: &'static str,
        calls: std::sync::Mutex<Vec<EventEnvelope>>,
    }

    impl RecordingDispatcher {
        fn for_type(event_type: &'static str) -> Self {
            Self {
                event_type_: event_type,
                name_: event_type,
                calls: std::sync::Mutex::new(Vec::new()),
            }
        }
    }

    impl Dispatcher for RecordingDispatcher {
        fn name(&self) -> &'static str {
            self.name_
        }

        fn event_type(&self) -> &'static str {
            self.event_type_
        }

        fn dispatch(&self, envelope: EventEnvelope) -> DispatchResult {
            self.calls.lock().unwrap().push(envelope.clone());
            DispatchResult::Handled
        }
    }

    fn test_envelope_type(event_type: &str) -> EventEnvelope {
        let event = crate::event::PushDeliveryEvent::new(
            "test_kind".into(),
            Some("TEST_CODE_AUDIT".into()),
            "Pushed".into(),
            "wechat".into(),
            42,
            10,
        );
        let mut envelope = EventEnvelope::from_event(
            &event,
            format!("evt-{event_type}"),
            "trace-1".into(),
            chrono::Local::now(),
        )
        .unwrap();
        envelope.event_type = event_type.into();
        envelope
    }

    #[test]
    fn registry_routes_only_exact_event_type() {
        let mut registry = DispatcherRegistry::new();
        registry.register(Arc::new(RecordingDispatcher::for_type(
            "push.delivery.audit",
        )));
        registry.register(Arc::new(RecordingDispatcher::for_type(
            "push.delivery.retry",
        )));
        registry.validate().unwrap();

        assert_eq!(
            registry.dispatch(test_envelope_type("push.delivery.audit")),
            DispatchResult::Handled
        );
        assert_eq!(
            registry.dispatch(test_envelope_type("push.delivery.retry")),
            DispatchResult::Handled
        );
        assert_eq!(
            registry.dispatch(test_envelope_type("push.delivery.retry.extra")),
            DispatchResult::Skipped("no_dispatcher".into())
        );
    }

    #[test]
    fn duplicate_exact_types_are_rejected_at_validation() {
        let mut registry = DispatcherRegistry::new();
        registry.register(Arc::new(RecordingDispatcher::for_type(
            "push.delivery.audit",
        )));
        registry.register(Arc::new(RecordingDispatcher::for_type(
            "push.delivery.audit",
        )));
        assert!(registry.validate().is_err());
    }

    #[test]
    fn duplicate_error_names_the_offending_event_type() {
        let mut registry = DispatcherRegistry::new();
        registry.register(Arc::new(RecordingDispatcher::for_type(
            "push.delivery.audit",
        )));
        registry.register(Arc::new(RecordingDispatcher::for_type(
            "push.delivery.audit",
        )));
        let err = registry.validate().unwrap_err();
        assert!(err.to_string().contains("push.delivery.audit"));
    }

    #[test]
    fn dispatch_returns_skipped_when_no_matching_handler() {
        let registry = DispatcherRegistry::new();
        let result = registry.dispatch(test_envelope_type("unknown.event"));
        assert_eq!(result, DispatchResult::Skipped("no_dispatcher".into()));
    }

    #[test]
    fn dispatch_returns_failed_when_handler_reports_failure() {
        struct FailingDispatcher;
        impl Dispatcher for FailingDispatcher {
            fn name(&self) -> &'static str {
                "FailingDispatcher"
            }
            fn event_type(&self) -> &'static str {
                "push.delivery.audit"
            }
            fn dispatch(&self, _envelope: EventEnvelope) -> DispatchResult {
                DispatchResult::Failed("sink unavailable".into())
            }
        }

        let mut registry = DispatcherRegistry::new();
        registry.register(Arc::new(FailingDispatcher));
        let result = registry.dispatch(test_envelope_type("push.delivery.audit"));
        assert_eq!(result, DispatchResult::Failed("sink unavailable".into()));
    }

    #[test]
    fn audit_dispatcher_increments_counter() {
        let fixture = TestAuditNamespace::new("COUNT");
        let dispatcher = fixture.dispatcher();
        assert_eq!(dispatcher.handled_count(), 0);

        dispatcher.dispatch(test_envelope_type("push.delivery.audit"));
        dispatcher.dispatch(test_envelope_type("push.delivery.audit"));

        assert_eq!(dispatcher.handled_count(), 2);
        let path = fixture
            .audit_path()
            .join(format!("{}.jsonl", chrono::Local::now().format("%Y")));
        let content = fs::read_to_string(path).unwrap();
        assert_eq!(content.lines().count(), 2);
    }

    #[test]
    fn br142_authoritative_record_uses_domain_hash_and_redacts_identity() {
        let fixture = TestAuditNamespace::new("V2");
        let dispatcher = fixture.dispatcher();
        let envelope = crate::event::persist_delivery_with(
            &dispatcher,
            "announcement_v1",
            Some("TEST_CODE_SECRET_AUDIT"),
            "Pushed",
            "dry_run",
            42,
            10,
        )
        .unwrap();

        let path = fixture
            .audit_path()
            .join(format!("{}.jsonl", envelope.ts.format("%Y")));
        let content = fs::read_to_string(&path).unwrap();
        let record: serde_json::Value = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(
            record["hash_domain"],
            "stock_analysis.delivery_audit_record.v2"
        );
        assert_eq!(record["envelope"]["payload"]["audit_schema_version"], 2);
        assert!(record["envelope"]["payload"].get("code").is_none());
        assert!(record["envelope"]["entity_key"].is_null());
        assert!(!content.contains("TEST_CODE_SECRET_AUDIT"));
        assert!(validate_existing_chain(&path).unwrap().is_some());
    }

    #[test]
    fn audit_dispatcher_rejects_non_push_delivery() {
        let fixture = TestAuditNamespace::new("REJECT_NON_DELIVERY");
        let dispatcher = fixture.dispatcher();
        let envelope = test_envelope_type("announcement.new");
        let result = dispatcher.dispatch(envelope);
        assert_eq!(result, DispatchResult::Skipped("no_dispatcher".into()));
    }

    #[test]
    fn br130_audit_dispatcher_rejects_invalid_payload_before_persistence() {
        let fixture = TestAuditNamespace::new("INVALID_PAYLOAD");
        let dispatcher = fixture.dispatcher();
        let mut envelope = test_envelope_type("push.delivery.audit");
        envelope.payload["outcome"] = serde_json::json!("Unknown");

        let result = dispatcher.dispatch(envelope);

        assert!(matches!(
            result,
            DispatchResult::Failed(error) if error.contains("outcome=Unknown")
        ));
        assert_eq!(dispatcher.handled_count(), 0);
        assert!(
            !fixture
                .audit_path()
                .join(format!("{}.jsonl", chrono::Local::now().format("%Y")))
                .exists(),
            "invalid audit must not create persistence output"
        );
    }

    #[test]
    fn audit_dispatcher_rejects_tampered_existing_chain() {
        let fixture = TestAuditNamespace::new("TAMPER");
        let dispatcher = fixture.dispatcher();
        let path = fixture
            .audit_path()
            .join(format!("{}.jsonl", chrono::Local::now().format("%Y")));
        fs::write(&path, "{not-json}\n").unwrap();

        let result = dispatcher.dispatch(test_envelope_type("push.delivery.audit"));

        assert!(
            matches!(result, DispatchResult::Failed(error) if error.contains("parse audit line 1"))
        );
        assert_eq!(dispatcher.handled_count(), 0);
        assert_eq!(fs::read_to_string(path).unwrap(), "{not-json}\n");
    }

    #[test]
    fn br091_persistence_failure_poisons_followup_writes() {
        let fixture = TestAuditNamespace::new("POISON");
        let dispatcher = fixture.dispatcher();
        let path = fixture
            .audit_path()
            .join(format!("{}.jsonl", chrono::Local::now().format("%Y")));
        fs::create_dir_all(&path).unwrap();

        let first = dispatcher.dispatch(test_envelope_type("push.delivery.audit"));
        assert!(matches!(first, DispatchResult::Failed(_)));
        assert_eq!(dispatcher.handled_count(), 0);

        fs::remove_dir_all(&path).unwrap();
        let second = dispatcher.dispatch(test_envelope_type("push.delivery.audit"));
        assert!(
            matches!(second, DispatchResult::Failed(error) if error.contains("audit chain is poisoned"))
        );
        assert_eq!(dispatcher.handled_count(), 0);
        assert!(!path.exists(), "poisoned dispatcher must not retry writing");
    }

    #[test]
    fn br091_existing_valid_chain_is_verified_and_extended() {
        let fixture = TestAuditNamespace::new("RESUME");
        let first = fixture.dispatcher();
        assert_eq!(
            first.dispatch(test_envelope_type("push.delivery.audit")),
            DispatchResult::Handled
        );
        drop(first);

        let second = fixture.dispatcher();
        assert_eq!(second.name(), "AuditDispatcher");
        assert_eq!(
            second.dispatch(test_envelope_type("push.delivery.audit")),
            DispatchResult::Handled
        );

        let path = fixture
            .audit_path()
            .join(format!("{}.jsonl", chrono::Local::now().format("%Y")));
        assert_eq!(fs::read_to_string(&path).unwrap().lines().count(), 2);
        assert!(validate_existing_chain(&path).unwrap().is_some());
    }

    #[test]
    fn br142_legacy_parent_is_read_only_and_extended_with_v2_domain() {
        use sha2::{Digest, Sha256};

        let fixture = TestAuditNamespace::new("LEGACY_PARENT");
        let dispatcher = fixture.dispatcher();
        let path = fixture
            .audit_path()
            .join(format!("{}.jsonl", chrono::Local::now().format("%Y")));
        let legacy_envelope = EventEnvelope {
            id: "legacy-event".into(),
            ts: chrono::Local::now(),
            trace_id: "legacy-trace".into(),
            source: "push_l4".into(),
            event_type: "push.delivery.audit".into(),
            entity_key: Some("TEST_CODE_LEGACY".into()),
            payload: serde_json::json!({
                "kind": "legacy_kind",
                "code": "TEST_CODE_LEGACY",
                "outcome": "Pushed",
                "channel": "dry_run",
                "rendered_len": 1,
                "latency_ms": 1,
            }),
            version: 1,
            replay_of: None,
        };
        let mut legacy = serde_json::json!({
            "envelope": legacy_envelope,
            "previous_hash": "GENESIS",
        });
        let legacy_hash = format!("{:x}", Sha256::digest(serde_json::to_vec(&legacy).unwrap()));
        legacy
            .as_object_mut()
            .unwrap()
            .insert("record_hash".into(), serde_json::Value::String(legacy_hash));
        fs::write(
            &path,
            format!("{}\n", serde_json::to_string(&legacy).unwrap()),
        )
        .unwrap();

        assert_eq!(
            dispatcher.dispatch(test_envelope_type("push.delivery.audit")),
            DispatchResult::Handled
        );
        let lines = fs::read_to_string(&path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].get("hash_domain").is_none());
        assert_eq!(lines[1]["hash_domain"], DELIVERY_AUDIT_RECORD_HASH_DOMAIN);
        assert!(validate_existing_chain(&path).unwrap().is_some());
    }

    #[test]
    fn br142_unknown_hash_domain_is_rejected() {
        let fixture = TestAuditNamespace::new("UNKNOWN_DOMAIN");
        let _dispatcher = fixture.dispatcher();
        let path = fixture
            .audit_path()
            .join(format!("{}.jsonl", chrono::Local::now().format("%Y")));
        let record = serde_json::json!({
            "envelope": test_envelope_type("push.delivery.audit"),
            "hash_domain": "stock_analysis.delivery_audit_record.unknown",
            "previous_hash": "GENESIS",
            "record_hash": "deadbeef",
        });
        fs::write(
            &path,
            format!("{}\n", serde_json::to_string(&record).unwrap()),
        )
        .unwrap();

        let error = validate_existing_chain(&path).unwrap_err();
        assert!(error.contains("unsupported audit hash domain"));
    }

    #[test]
    fn br142_v2_chain_rejects_a_later_legacy_row() {
        use sha2::{Digest, Sha256};

        let fixture = TestAuditNamespace::new("DOWNGRADE");
        let dispatcher = fixture.dispatcher();
        assert_eq!(
            dispatcher.dispatch(test_envelope_type("push.delivery.audit")),
            DispatchResult::Handled
        );
        let path = fixture
            .audit_path()
            .join(format!("{}.jsonl", chrono::Local::now().format("%Y")));
        let v2: serde_json::Value =
            serde_json::from_str(fs::read_to_string(&path).unwrap().lines().next().unwrap())
                .unwrap();
        let mut legacy = serde_json::json!({
            "envelope": EventEnvelope {
                id: "legacy-after-v2".into(),
                ts: chrono::Local::now(),
                trace_id: "legacy-after-v2-trace".into(),
                source: "push_l4".into(),
                event_type: "push.delivery.audit".into(),
                entity_key: Some("TEST_CODE_LEGACY".into()),
                payload: serde_json::json!({
                    "kind": "legacy_kind",
                    "code": "TEST_CODE_LEGACY",
                    "outcome": "Pushed",
                    "channel": "dry_run",
                    "rendered_len": 1,
                    "latency_ms": 1,
                }),
                version: 1,
                replay_of: None,
            },
            "previous_hash": v2["record_hash"].as_str().unwrap(),
        });
        let hash = format!("{:x}", Sha256::digest(serde_json::to_vec(&legacy).unwrap()));
        legacy
            .as_object_mut()
            .unwrap()
            .insert("record_hash".into(), serde_json::Value::String(hash));
        writeln!(
            OpenOptions::new().append(true).open(&path).unwrap(),
            "{}",
            serde_json::to_string(&legacy).unwrap()
        )
        .unwrap();

        let error = validate_existing_chain(&path).unwrap_err();
        assert!(error.contains("legacy audit after v2"));
    }

    #[test]
    fn br142_legacy_row_still_requires_a_complete_delivery_envelope() {
        use sha2::{Digest, Sha256};

        let fixture = TestAuditNamespace::new("INVALID_LEGACY");
        let _dispatcher = fixture.dispatcher();
        let path = fixture
            .audit_path()
            .join(format!("{}.jsonl", chrono::Local::now().format("%Y")));
        let mut record = serde_json::json!({
            "envelope": {},
            "previous_hash": "GENESIS",
        });
        let hash = format!("{:x}", Sha256::digest(serde_json::to_vec(&record).unwrap()));
        record
            .as_object_mut()
            .unwrap()
            .insert("record_hash".into(), serde_json::Value::String(hash));
        fs::write(
            &path,
            format!("{}\n", serde_json::to_string(&record).unwrap()),
        )
        .unwrap();

        let error = validate_existing_chain(&path).unwrap_err();
        assert!(error.contains("legacy delivery envelope"));
    }

    #[test]
    fn br142_authoritative_dispatch_rejects_unknown_or_unbound_identity_fields() {
        let fixture = TestAuditNamespace::new("INJECTION");
        let dispatcher = fixture.dispatcher();
        let mut injected = test_envelope_type("push.delivery.audit");
        injected.payload["announcement_id"] = serde_json::json!("TEST_CODE_SECRET");
        assert!(matches!(
            dispatcher.dispatch(injected),
            DispatchResult::Failed(error) if error.contains("unknown")
        ));

        let second_fixture = TestAuditNamespace::new("UNBOUND_IDENTITY");
        let second = second_fixture.dispatcher();
        let mut unbound = test_envelope_type("push.delivery.audit");
        unbound.payload["identity_hash"] = serde_json::json!("a".repeat(64));
        assert!(matches!(
            second.dispatch(unbound),
            DispatchResult::Failed(error) if error.contains("identity_hash")
        ));
        assert!(!fixture
            .audit_path()
            .join(format!("{}.jsonl", chrono::Local::now().format("%Y")))
            .exists());
        assert!(!second_fixture
            .audit_path()
            .join(format!("{}.jsonl", chrono::Local::now().format("%Y")))
            .exists());
    }

    #[test]
    fn br192_arbitrary_audit_path_is_a_failed_capability() {
        let dispatcher = AuditDispatcher::new(PathBuf::from("audit-override"));
        assert!(dispatcher
            .preflight()
            .unwrap_err()
            .contains("not a fixed production/TEST_CODE authority"));
    }

    #[test]
    fn br192_audit_file_eexist_race_is_reopened_after_parent_sync() {
        let fixture = TestAuditNamespace::new("EEXIST_PARENT_SYNC");
        let dispatcher = fixture.dispatcher();
        let capability = dispatcher
            .capability
            .as_ref()
            .expect("bind exact TEST_CODE audit capability");
        let name = OsStr::new("TEST_CODE_EEXIST.lock");
        let path = fixture.audit_path().join(name);
        let (file, identity) = open_or_create_audit_file_with_hook(capability, name, false, || {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map(|_| ())
                .map_err(|error| format!("TEST_CODE wins audit create race: {error}"))
        })
        .expect("audit EEXIST winner must be reopened after parent fsync");
        assert!(file.metadata().unwrap().is_file());
        assert!(identity.is_file);
        assert_eq!(identity.links, 1);
    }

    #[test]
    fn br192_audit_directory_link_count_is_revalidated_not_frozen() {
        let fixture = TestAuditNamespace::new("DIRECTORY_NLINK");
        let dispatcher = fixture.dispatcher();
        fs::create_dir(fixture.root.join("push_log"))
            .expect("TEST_CODE sibling changes parent directory link count");
        dispatcher
            .preflight()
            .expect("safe sibling creation must not impersonate the retained audit directory");
    }

    #[test]
    fn br192_audit_directory_safe_mode_drift_is_rejected() {
        let fixture = TestAuditNamespace::new("SAFE_MODE_DRIFT");
        let dispatcher = fixture.dispatcher();
        let path = fixture.audit_path();
        let original = fs::metadata(path).unwrap().permissions();
        let original_mode = original.mode() & 0o7777;
        let drifted_mode = if original_mode == 0o700 { 0o750 } else { 0o700 };
        fs::set_permissions(path, fs::Permissions::from_mode(drifted_mode))
            .expect("apply another safe TEST_CODE directory mode");
        let result = dispatcher.preflight();
        fs::set_permissions(path, original).expect("restore TEST_CODE audit permissions");
        assert!(
            result
                .unwrap_err()
                .contains("audit ancestor identity changed"),
            "allowed-to-allowed mode drift must still invalidate the retained identity"
        );
    }

    #[test]
    fn br192_audit_directory_allowed_owner_drift_is_rejected() {
        let effective_uid = 501;
        assert!(audit_directory_owner_allowed(0, effective_uid));
        assert!(audit_directory_owner_allowed(effective_uid, effective_uid));
        let root_owned = AuditObjectIdentity {
            device: 1,
            inode: 2,
            mode: 0o040700,
            uid: 0,
            links: 2,
            is_directory: true,
            is_file: false,
        };
        let mut effective_owned = root_owned.clone();
        effective_owned.uid = effective_uid;
        assert!(
            !same_audit_directory_object(&root_owned, &effective_owned),
            "two individually allowed owners are not the same retained authority"
        );
    }

    #[test]
    fn br141_existing_valid_record_without_newline_is_rejected() {
        let fixture = TestAuditNamespace::new("TRAILING_RECORD");
        let first = fixture.dispatcher();
        assert_eq!(
            first.dispatch(test_envelope_type("push.delivery.audit")),
            DispatchResult::Handled
        );
        drop(first);

        let path = fixture
            .audit_path()
            .join(format!("{}.jsonl", chrono::Local::now().format("%Y")));
        let complete = fs::read_to_string(&path).unwrap();
        fs::write(&path, complete.strip_suffix('\n').unwrap()).unwrap();
        let second = fixture.dispatcher();
        let result = second.dispatch(test_envelope_type("push.delivery.audit"));
        assert!(
            matches!(result, DispatchResult::Failed(error) if error.contains("incomplete trailing record"))
        );
        assert_eq!(fs::read_to_string(&path).unwrap().lines().count(), 1);
    }

    #[test]
    #[ignore = "invoked as a child by the cross-process audit locking test"]
    fn br141_event_audit_process_writer_helper() {
        let Ok(test_code) = std::env::var("BR141_EVENT_AUDIT_HELPER_TEST_CODE") else {
            return;
        };
        let identity = std::env::var("BR141_EVENT_AUDIT_HELPER_ID").unwrap();
        let mut envelope = test_envelope_type("push.delivery.audit");
        envelope.id = format!("event-audit-{identity}");
        let dispatcher =
            AuditDispatcher::for_test_code(&test_code).expect("bind child TEST_CODE audit");
        assert_eq!(dispatcher.dispatch(envelope), DispatchResult::Handled);
    }

    #[test]
    fn br141_event_audit_serializes_independent_process_writers() {
        let fixture = TestAuditNamespace::new("CROSS_PROCESS");
        let executable = std::env::current_exe().unwrap();
        let mut children = (0..4)
            .map(|index| {
                std::process::Command::new(&executable)
                    .args([
                        "--exact",
                        "event::dispatcher::tests::br141_event_audit_process_writer_helper",
                        "--ignored",
                    ])
                    .env("BR141_EVENT_AUDIT_HELPER_TEST_CODE", fixture.test_code())
                    .env("BR141_EVENT_AUDIT_HELPER_ID", format!("writer-{index}"))
                    .spawn()
                    .unwrap()
            })
            .collect::<Vec<_>>();
        for child in &mut children {
            assert!(child.wait().unwrap().success());
        }

        let path = fixture
            .audit_path()
            .join(format!("{}.jsonl", chrono::Local::now().format("%Y")));
        assert!(validate_existing_chain(&path).unwrap().is_some());
        assert_eq!(fs::read_to_string(path).unwrap().lines().count(), 4);
    }

    #[test]
    fn br091_existing_chain_rejects_every_structural_corruption_class() {
        let fixture = TestAuditNamespace::new("STRUCTURAL_CORRUPTION");
        let _dispatcher = fixture.dispatcher();
        let path = fixture.audit_path().join("audit.jsonl");

        for (line, expected) in [
            ("\n", "is blank"),
            (
                "{\"previous_hash\":\"GENESIS\",\"envelope\":{}}\n",
                "no record_hash",
            ),
            (
                "{\"record_hash\":\"x\",\"envelope\":{}}\n",
                "no previous_hash",
            ),
            (
                "{\"record_hash\":\"x\",\"previous_hash\":\"WRONG\",\"envelope\":{}}\n",
                "chain mismatch",
            ),
            (
                "{\"record_hash\":\"deadbeef\",\"previous_hash\":\"GENESIS\",\"envelope\":{}}\n",
                "hash mismatch",
            ),
        ] {
            fs::write(&path, line).unwrap();
            let error = validate_existing_chain(&path).unwrap_err();
            assert!(error.contains(expected), "{error}");
        }
    }

    #[test]
    fn audit_runtime_constructor_and_directory_failure_are_explicit() {
        let runtime = AuditDispatcher::default();
        assert_eq!(runtime.name(), "AuditDispatcher");
        assert_eq!(runtime.event_type(), "push.delivery.audit");

        let dispatcher = AuditDispatcher::new(PathBuf::from("TEST_CODE_NOT_AN_AUTHORITY"));
        let result = dispatcher.dispatch(test_envelope_type("push.delivery.audit"));
        assert!(
            matches!(result, DispatchResult::Failed(error) if error.contains("not a fixed production/TEST_CODE authority")),
            "caller-selected authority rejection must be visible"
        );
        assert_eq!(dispatcher.handled_count(), 0);
    }
}
