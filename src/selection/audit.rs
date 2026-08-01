//! BR-157 authoritative append-only hash-chain audit for shadow selection.

use chrono::{DateTime, FixedOffset};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::ffi::{CString, OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, Read, Seek, SeekFrom, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use thiserror::Error;

pub const AUDIT_SCHEMA_VERSION: u16 = 1;
pub const AUDIT_DOMAIN: &str = "stock_analysis.selection_audit.v1";
const PRODUCTION_AUDIT_ROOT_RELATIVE_PATH: &str = "data/audit/production";
const AUDIT_FILE_NAME: &str = "selection-audit.jsonl";
const AUDIT_LOCK_FILE_NAME: &str = "selection-audit.lock";
const O_RDONLY_FLAG: i32 = 0;
const O_RDWR_FLAG: i32 = 2;

#[cfg(target_os = "linux")]
const O_NOFOLLOW_FLAG: i32 = 0x0002_0000;
#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd"
))]
const O_NOFOLLOW_FLAG: i32 = 0x0000_0100;
#[cfg(target_os = "linux")]
const O_NONBLOCK_FLAG: i32 = 0x0000_0800;
#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd"
))]
const O_NONBLOCK_FLAG: i32 = 0x0000_0004;
#[cfg(target_os = "linux")]
const O_CREAT_FLAG: i32 = 0x0000_0040;
#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd"
))]
const O_CREAT_FLAG: i32 = 0x0000_0200;
#[cfg(target_os = "linux")]
const O_EXCL_FLAG: i32 = 0x0000_0080;
#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd"
))]
const O_EXCL_FLAG: i32 = 0x0000_0800;
#[cfg(target_os = "linux")]
const O_APPEND_FLAG: i32 = 0x0000_0400;
#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd"
))]
const O_APPEND_FLAG: i32 = 0x0000_0008;
#[cfg(target_os = "linux")]
const O_CLOEXEC_FLAG: i32 = 0x0008_0000;
#[cfg(any(target_os = "macos", target_os = "ios"))]
const O_CLOEXEC_FLAG: i32 = 0x0100_0000;
#[cfg(target_os = "freebsd")]
const O_CLOEXEC_FLAG: i32 = 0x0010_0000;
#[cfg(target_os = "openbsd")]
const O_CLOEXEC_FLAG: i32 = 0x0001_0000;
#[cfg(target_os = "netbsd")]
const O_CLOEXEC_FLAG: i32 = 0x0040_0000;

unsafe extern "C" {
    fn openat(directory_fd: i32, path: *const std::ffi::c_char, flags: i32, ...) -> i32;
    fn mkdirat(directory_fd: i32, path: *const std::ffi::c_char, mode: u32) -> i32;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionAuditPhase {
    Ingested,
    Prepared,
    Committed,
    Rejected,
    Completed,
    T0Close,
    D1Settled,
    V2ConfigActivationPrepared,
    V2ConfigActivationCommitted,
    V2IngressPrepared,
    V2IngressCommitted,
    V2GenerationPrepared,
    V2GenerationCommitted,
    V2OutcomeClaimPrepared,
    V2OutcomeClaimCommitted,
    V2OutcomePrepared,
    V2OutcomeCommitted,
    V2BoardBindingAuditPrepared,
    V2BoardBindingAuditCommitted,
    V2GateDCanaryVerified,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectionAuditContext {
    pub event_identity_hash: Option<String>,
    pub chain_identity_hash: Option<String>,
    pub security_identity_hash: Option<String>,
    pub provider: Option<String>,
    pub provider_published_at: Option<DateTime<FixedOffset>>,
    pub observed_at: Option<DateTime<FixedOffset>>,
    pub magic_tdx_batch_id: Option<String>,
    pub reason_codes: Vec<String>,
    pub rule_ids: Vec<String>,
    pub retryable: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectionAuditRecord {
    pub schema_version: u16,
    pub domain: String,
    pub phase: SelectionAuditPhase,
    pub subject_id: String,
    pub content_hash: String,
    pub context: SelectionAuditContext,
    pub previous_hash: Option<String>,
    pub recorded_at: DateTime<FixedOffset>,
    pub record_hash: String,
}

impl SelectionAuditRecord {
    pub fn new(
        phase: SelectionAuditPhase,
        subject_id: impl Into<String>,
        content_hash: impl Into<String>,
        recorded_at: DateTime<FixedOffset>,
    ) -> Self {
        Self {
            schema_version: AUDIT_SCHEMA_VERSION,
            domain: AUDIT_DOMAIN.to_owned(),
            phase,
            subject_id: subject_id.into(),
            content_hash: content_hash.into(),
            context: SelectionAuditContext::default(),
            previous_hash: None,
            recorded_at,
            record_hash: String::new(),
        }
    }

    pub fn with_context(mut self, context: SelectionAuditContext) -> Self {
        self.context = context;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditAppendReceipt {
    pub record_hash: String,
    pub previous_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditValidationReceipt {
    pub record_count: usize,
    pub tail_hash: Option<String>,
}

/// Complete, strictly validated audit-chain contents captured while the
/// caller still owns the exclusive audit lock.
///
/// The validation receipt and records are returned together so downstream
/// integrity checks cannot accidentally combine records from one scan with a
/// high-water mark from another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedAuditChainSnapshot {
    validation: AuditValidationReceipt,
    records: Vec<SelectionAuditRecord>,
}

impl ValidatedAuditChainSnapshot {
    pub fn validation(&self) -> &AuditValidationReceipt {
        &self.validation
    }

    pub fn records(&self) -> &[SelectionAuditRecord] {
        &self.records
    }
}

/// Result of an exact recovery lookup performed while the audit lock is held.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use = "recovery callers must handle missing and conflicting audit evidence"]
pub enum AuditExactLookup {
    Missing,
    Exact(SelectionAuditRecord),
    ContentConflict {
        existing_record: SelectionAuditRecord,
    },
}

#[derive(Debug, Error)]
pub enum SelectionAuditError {
    #[error("selection audit chain invalid: {0}")]
    ChainInvalid(String),
    #[error("selection audit record invalid: {0}")]
    InvalidRecord(String),
    #[error("selection audit lock failed: {0}")]
    Lock(String),
    #[error("selection audit I/O failed: {0}")]
    Io(String),
    #[error("selection audit path invalid: {0}")]
    PathInvalid(String),
}

impl SelectionAuditError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::ChainInvalid(_) => "audit_chain_invalid",
            Self::InvalidRecord(_) => "audit_record_invalid",
            Self::Lock(_) => "audit_lock_failed",
            Self::Io(_) => "audit_io_failure",
            Self::PathInvalid(_) => "audit_path_invalid",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SelectionAuditWriter {
    namespace_root: PathBuf,
    path: PathBuf,
    lock_path: PathBuf,
    pinned_test_namespace: Option<Arc<PinnedTestAuditNamespace>>,
}

#[derive(Debug)]
struct PinnedTestAuditNamespace {
    root_file: File,
    root_marker: DirectoryMutationMarker,
    namespace_file: File,
    namespace_identity: FileIdentity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    device: u64,
    inode: u64,
    kind: u32,
}

impl FileIdentity {
    fn from_metadata(metadata: &fs::Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            kind: metadata.mode() & 0o170_000,
        }
    }
}

/// Detects namespace ABA even when the same audit inode is restored before
/// the next operation. File identity alone cannot reveal that remove/rename
/// cycle, while a directory entry mutation changes this retained parent's
/// metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DirectoryMutationMarker {
    identity: FileIdentity,
    length: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

impl DirectoryMutationMarker {
    fn from_metadata(metadata: &fs::Metadata) -> Self {
        Self {
            identity: FileIdentity::from_metadata(metadata),
            length: metadata.len(),
            modified_seconds: metadata.mtime(),
            modified_nanoseconds: metadata.mtime_nsec(),
            changed_seconds: metadata.ctime(),
            changed_nanoseconds: metadata.ctime_nsec(),
        }
    }
}

enum PinnedAuditData {
    /// A missing audit remains missing during read-only validation. The pinned
    /// parent descriptor and mutation marker are the authority for absence.
    Absent,
    Present {
        file: File,
        identity: FileIdentity,
    },
}

/// An exclusive, validated selection-audit critical section.
///
/// The fields are private so callers cannot construct a session without taking
/// both the process-local serialization guard and the cross-process OS lock.
/// The type intentionally does not implement `Clone`.
#[must_use = "authoritative audit operations must call finish() and handle cleanup errors"]
pub struct LockedSelectionAuditSession<'writer> {
    writer: &'writer SelectionAuditWriter,
    process_guard: Option<MutexGuard<'static, ()>>,
    namespace_container: Option<PinnedNamespaceContainer>,
    parent_file: File,
    parent_identity: FileIdentity,
    parent_marker: DirectoryMutationMarker,
    lock_file: Option<File>,
    lock_identity: FileIdentity,
    audit_data: PinnedAuditData,
    validation: AuditValidationReceipt,
    poisoned: bool,
    #[cfg(test)]
    inject_unlock_failure: bool,
}

struct PinnedNamespaceContainer {
    file: File,
    marker: DirectoryMutationMarker,
    enforce_initial_marker: bool,
    namespace_identity: FileIdentity,
    namespace_leaf: OsString,
}

impl SelectionAuditWriter {
    /// Opens the one authoritative production audit namespace.
    ///
    /// The root is compile-time anchored and intentionally accepts no caller
    /// path, environment override, or process-CWD input.
    pub fn production() -> Result<Self, SelectionAuditError> {
        let namespace_root =
            Path::new(env!("CARGO_MANIFEST_DIR")).join(PRODUCTION_AUDIT_ROOT_RELATIVE_PATH);
        Self::for_namespace(namespace_root)
    }

    /// Constructs a physically isolated test writer.
    ///
    /// This crate-private entry point always appends its own `test` namespace.
    /// Production command code may use it only after the global schema owner
    /// has issued an invocation-isolated `TEST_CODE_` rehearsal root.
    #[cfg(test)]
    pub(crate) fn for_test_code_root(root: impl AsRef<Path>) -> Result<Self, SelectionAuditError> {
        Self::for_namespace(root.as_ref().join("test"))
    }

    /// Binds the isolated `test` namespace to the global owner's retained
    /// TEST_CODE root descriptor. `root_path` is diagnostic-only; it is never
    /// reopened to obtain authority.
    pub(crate) fn for_test_code_pinned_root(
        root_descriptor: &File,
        root_path: &Path,
    ) -> Result<Self, SelectionAuditError> {
        validate_absolute_normal_path(root_path)?;
        require_directory_metadata(root_descriptor, root_path, "pin TEST_CODE audit root")?;
        mkdirat_component(
            root_descriptor,
            OsStr::new("test"),
            root_path.join("test").as_path(),
        )?;
        let namespace_file = openat_directory(
            root_descriptor,
            OsStr::new("test"),
            &root_path.join("test"),
            "pin TEST_CODE audit namespace",
        )?;
        let namespace_identity = FileIdentity::from_metadata(&require_directory_metadata(
            &namespace_file,
            &root_path.join("test"),
            "pin TEST_CODE audit namespace",
        )?);
        let root_file = root_descriptor.try_clone().map_err(|error| {
            SelectionAuditError::Io(format!(
                "clone pinned TEST_CODE audit root {}: {error}",
                root_path.display()
            ))
        })?;
        let root_marker = directory_marker(
            &root_file,
            root_path,
            "capture TEST_CODE audit root mutation marker",
        )?;
        let namespace_root = root_path.join("test");
        let writer = Self {
            path: namespace_root.join(AUDIT_FILE_NAME),
            lock_path: namespace_root.join(AUDIT_LOCK_FILE_NAME),
            namespace_root,
            pinned_test_namespace: Some(Arc::new(PinnedTestAuditNamespace {
                root_file,
                root_marker,
                namespace_file,
                namespace_identity,
            })),
        };
        Ok(writer)
    }

    fn for_namespace(namespace_root: PathBuf) -> Result<Self, SelectionAuditError> {
        let writer = Self {
            path: namespace_root.join(AUDIT_FILE_NAME),
            lock_path: namespace_root.join(AUDIT_LOCK_FILE_NAME),
            namespace_root,
            pinned_test_namespace: None,
        };
        writer.validate_bound_paths()?;
        Ok(writer)
    }

    fn validate_bound_paths(&self) -> Result<(), SelectionAuditError> {
        validate_no_symlink_components(&self.namespace_root, FinalPathKind::Directory)?;
        validate_no_symlink_components(&self.path, FinalPathKind::RegularFile)?;
        validate_no_symlink_components(&self.lock_path, FinalPathKind::RegularFile)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn lock_path(&self) -> &Path {
        &self.lock_path
    }

    pub fn append(
        &self,
        record: SelectionAuditRecord,
    ) -> Result<AuditAppendReceipt, SelectionAuditError> {
        let mut session = self.locked_session()?;
        let result = session.append(record);
        finish_session_operation(session, result)
    }

    pub fn validate(&self) -> Result<AuditValidationReceipt, SelectionAuditError> {
        let mut session = self.locked_session()?;
        let result = session.validate();
        finish_session_operation(session, result)
    }

    pub fn locked_session(&self) -> Result<LockedSelectionAuditSession<'_>, SelectionAuditError> {
        // BR-194 test-suite fix: recover from a poisoned process_audit_lock
        // so that one panicking test does not cascade into all sibling
        // tests (verified empirically: a poisoned mutex failed 35 of 45
        // selection_v2_repository tests when run in the same process).
        // This recovery is intentional and limited to the process-local
        // selection-audit mutex; production cross-process correctness
        // still comes from SQLite BEGIN IMMEDIATE + fence tokens, not
        // from this Mutex. The guard is acquired immediately and held
        // for the duration of the locked session; the prior holder's
        // poisoned panic payload is not re-raised here.
        let process_guard = process_audit_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        if self.pinned_test_namespace.is_none() {
            self.validate_bound_paths()?;
        } else {
            validate_absolute_normal_path(&self.namespace_root)?;
        }
        let (parent_file, namespace_container) = if let Some(pinned) = &self.pinned_test_namespace {
            let parent_file = pinned.namespace_file.try_clone().map_err(|error| {
                SelectionAuditError::Io(format!(
                    "clone pinned TEST_CODE audit namespace {}: {error}",
                    self.namespace_root.display()
                ))
            })?;
            let container_file = pinned.root_file.try_clone().map_err(|error| {
                SelectionAuditError::Io(format!(
                    "clone pinned TEST_CODE audit root for {}: {error}",
                    self.namespace_root.display()
                ))
            })?;
            (
                parent_file,
                Some(PinnedNamespaceContainer {
                    file: container_file,
                    marker: pinned.root_marker,
                    enforce_initial_marker: false,
                    namespace_identity: pinned.namespace_identity,
                    namespace_leaf: OsString::from("test"),
                }),
            )
        } else {
            let parent_file = open_or_create_absolute_directory_no_follow(&self.namespace_root)?;
            let container_path = self.namespace_root.parent().ok_or_else(|| {
                SelectionAuditError::PathInvalid(format!(
                    "audit namespace has no parent: {}",
                    self.namespace_root.display()
                ))
            })?;
            let namespace_leaf = self
                .namespace_root
                .file_name()
                .ok_or_else(|| {
                    SelectionAuditError::PathInvalid(format!(
                        "audit namespace has no final component: {}",
                        self.namespace_root.display()
                    ))
                })?
                .to_os_string();
            let container_file = open_absolute_directory_no_follow(container_path)?;
            let marker = directory_marker(
                &container_file,
                container_path,
                "capture audit namespace-container mutation marker",
            )?;
            let namespace_identity = FileIdentity::from_metadata(&require_directory_metadata(
                &parent_file,
                &self.namespace_root,
                "pin audit namespace parent",
            )?);
            (
                parent_file,
                Some(PinnedNamespaceContainer {
                    file: container_file,
                    marker,
                    enforce_initial_marker: true,
                    namespace_identity,
                    namespace_leaf,
                }),
            )
        };
        let parent_metadata = require_directory_metadata(
            &parent_file,
            &self.namespace_root,
            "pin audit namespace parent",
        )?;
        let parent_identity = FileIdentity::from_metadata(&parent_metadata);
        let lock_file = openat_regular(
            &parent_file,
            OsStr::new(AUDIT_LOCK_FILE_NAME),
            O_RDWR_FLAG | O_CREAT_FLAG,
            &self.lock_path,
            "open audit lock",
        )
        .map_err(|error| SelectionAuditError::Lock(error.to_string()))?;
        let lock_identity = file_identity(&lock_file, &self.lock_path, "pin audit lock")?;
        FileExt::lock_exclusive(&lock_file).map_err(|error| {
            SelectionAuditError::Lock(format!(
                "acquire audit lock {}: {error}",
                self.lock_path.display()
            ))
        })?;

        let audit_data = match openat_regular_optional(
            &parent_file,
            OsStr::new(AUDIT_FILE_NAME),
            O_RDWR_FLAG | O_APPEND_FLAG,
            &self.path,
            "pin selection audit",
        ) {
            Ok(Some(file)) => {
                let identity = match file_identity(&file, &self.path, "pin selection audit") {
                    Ok(identity) => identity,
                    Err(error) => {
                        return unlock_after_failed_acquisition(&lock_file, &self.lock_path, error);
                    }
                };
                PinnedAuditData::Present { file, identity }
            }
            Ok(None) => PinnedAuditData::Absent,
            Err(error) => {
                return unlock_after_failed_acquisition(&lock_file, &self.lock_path, error);
            }
        };
        let parent_marker = match directory_marker(
            &parent_file,
            &self.namespace_root,
            "capture audit namespace mutation marker",
        ) {
            Ok(marker) => marker,
            Err(error) => {
                return unlock_after_failed_acquisition(&lock_file, &self.lock_path, error);
            }
        };
        let mut session = LockedSelectionAuditSession {
            writer: self,
            process_guard: Some(process_guard),
            namespace_container,
            parent_file,
            parent_identity,
            parent_marker,
            lock_file: Some(lock_file),
            lock_identity,
            audit_data,
            validation: AuditValidationReceipt {
                record_count: 0,
                tail_hash: None,
            },
            poisoned: false,
            #[cfg(test)]
            inject_unlock_failure: false,
        };
        let setup = session
            .revalidate_namespace_binding()
            .and_then(|()| session.scan_pinned_chain(|_| {}));
        match setup {
            Ok(validation) => {
                session.validation = validation;
                Ok(session)
            }
            Err(error) => {
                let cleanup = session.unlock();
                match cleanup {
                    Ok(()) => Err(error),
                    Err(cleanup_error) => Err(SelectionAuditError::Lock(format!(
                        "audit session setup failed ({error}); cleanup also failed ({cleanup_error})"
                    ))),
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FinalPathKind {
    Directory,
    RegularFile,
}

fn validate_no_symlink_components(
    path: &Path,
    final_kind: FinalPathKind,
) -> Result<(), SelectionAuditError> {
    if !path.is_absolute() {
        return Err(SelectionAuditError::PathInvalid(format!(
            "audit path must be absolute and CARGO_MANIFEST_DIR anchored: {}",
            path.display()
        )));
    }

    let mut cursor = PathBuf::new();
    let mut components = path.components().peekable();
    let mut missing_ancestor = false;
    while let Some(component) = components.next() {
        match component {
            Component::Prefix(prefix) if cursor.as_os_str().is_empty() => {
                cursor.push(prefix.as_os_str());
                continue;
            }
            Component::RootDir => cursor.push(component.as_os_str()),
            Component::Normal(value) => cursor.push(value),
            Component::CurDir | Component::ParentDir | Component::Prefix(_) => {
                return Err(SelectionAuditError::PathInvalid(format!(
                    "audit path contains a forbidden component: {}",
                    path.display()
                )));
            }
        }

        if missing_ancestor {
            continue;
        }
        let is_final = components.peek().is_none();
        let metadata = match fs::symlink_metadata(&cursor) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                missing_ancestor = true;
                continue;
            }
            Err(error) => {
                return Err(SelectionAuditError::Io(format!(
                    "inspect audit path component {}: {error}",
                    cursor.display()
                )));
            }
        };
        if metadata.file_type().is_symlink() {
            return Err(SelectionAuditError::PathInvalid(format!(
                "audit path symlink component is forbidden: {}",
                cursor.display()
            )));
        }
        if !is_final && !metadata.is_dir() {
            return Err(SelectionAuditError::PathInvalid(format!(
                "audit path ancestor is not a directory: {}",
                cursor.display()
            )));
        }
        if is_final {
            let valid = match final_kind {
                FinalPathKind::Directory => metadata.is_dir(),
                FinalPathKind::RegularFile => metadata.is_file(),
            };
            if !valid {
                return Err(SelectionAuditError::PathInvalid(format!(
                    "audit path has unexpected file type: {}",
                    cursor.display()
                )));
            }
        }
    }
    Ok(())
}

fn validate_absolute_normal_path(path: &Path) -> Result<(), SelectionAuditError> {
    if !path.is_absolute() {
        return Err(SelectionAuditError::PathInvalid(format!(
            "audit path must be absolute: {}",
            path.display()
        )));
    }
    for component in path.components() {
        match component {
            Component::RootDir | Component::Normal(_) => {}
            Component::CurDir | Component::ParentDir | Component::Prefix(_) => {
                return Err(SelectionAuditError::PathInvalid(format!(
                    "audit path contains a forbidden component: {}",
                    path.display()
                )));
            }
        }
    }
    Ok(())
}

fn component_cstring(name: &OsStr) -> Result<CString, SelectionAuditError> {
    if name.is_empty() || name.as_bytes().contains(&b'/') {
        return Err(SelectionAuditError::PathInvalid(
            "descriptor-relative audit component must be one non-empty path segment".to_owned(),
        ));
    }
    CString::new(name.as_bytes()).map_err(|_| {
        SelectionAuditError::PathInvalid(
            "descriptor-relative audit component contains NUL".to_owned(),
        )
    })
}

fn openat_file(parent: &File, name: &OsStr, flags: i32, mode: u32) -> Result<File, io::Error> {
    let name = component_cstring(name)
        .map_err(|error| io::Error::new(ErrorKind::InvalidInput, error.to_string()))?;
    // SAFETY: `name` is one live NUL-terminated component, `parent` retains a
    // valid directory descriptor, and successful ownership transfers directly
    // into `File`.
    let descriptor = unsafe {
        openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            flags | O_NOFOLLOW_FLAG | O_NONBLOCK_FLAG | O_CLOEXEC_FLAG,
            mode,
        )
    };
    if descriptor < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful `openat` returns one newly owned descriptor.
    Ok(unsafe { File::from_raw_fd(descriptor) })
}

fn mkdirat_component(parent: &File, name: &OsStr, path: &Path) -> Result<(), SelectionAuditError> {
    let name = component_cstring(name)?;
    // SAFETY: `name` is one live NUL-terminated component and `parent`
    // retains a valid directory descriptor.
    let result = unsafe { mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700_u32) };
    if result < 0 {
        let error = io::Error::last_os_error();
        if error.kind() != ErrorKind::AlreadyExists {
            return Err(SelectionAuditError::Io(format!(
                "create descriptor-relative audit directory {}: {error}",
                path.display()
            )));
        }
    }
    Ok(())
}

fn openat_directory(
    parent: &File,
    name: &OsStr,
    path: &Path,
    operation: &str,
) -> Result<File, SelectionAuditError> {
    let file = openat_file(parent, name, O_RDONLY_FLAG, 0).map_err(|error| {
        SelectionAuditError::PathInvalid(format!("{operation} {}: {error}", path.display()))
    })?;
    require_directory_metadata(&file, path, operation)?;
    Ok(file)
}

fn openat_regular(
    parent: &File,
    name: &OsStr,
    flags: i32,
    path: &Path,
    operation: &str,
) -> Result<File, SelectionAuditError> {
    let file = openat_file(parent, name, flags, 0o600_u32).map_err(|error| {
        SelectionAuditError::PathInvalid(format!("{operation} {}: {error}", path.display()))
    })?;
    let metadata = file.metadata().map_err(|error| {
        SelectionAuditError::Io(format!("{operation} metadata {}: {error}", path.display()))
    })?;
    if !metadata.is_file() {
        return Err(SelectionAuditError::PathInvalid(format!(
            "{operation} did not resolve to a regular file: {}",
            path.display()
        )));
    }
    Ok(file)
}

fn openat_regular_optional(
    parent: &File,
    name: &OsStr,
    flags: i32,
    path: &Path,
    operation: &str,
) -> Result<Option<File>, SelectionAuditError> {
    match openat_file(parent, name, flags, 0o600_u32) {
        Ok(file) => {
            let metadata = file.metadata().map_err(|error| {
                SelectionAuditError::Io(format!("{operation} metadata {}: {error}", path.display()))
            })?;
            if !metadata.is_file() {
                return Err(SelectionAuditError::PathInvalid(format!(
                    "{operation} did not resolve to a regular file: {}",
                    path.display()
                )));
            }
            Ok(Some(file))
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(SelectionAuditError::PathInvalid(format!(
            "{operation} {}: {error}",
            path.display()
        ))),
    }
}

fn open_absolute_directory_no_follow(path: &Path) -> Result<File, SelectionAuditError> {
    validate_absolute_normal_path(path)?;
    let mut directory = OpenOptions::new()
        .read(true)
        .custom_flags(O_NOFOLLOW_FLAG | O_NONBLOCK_FLAG | O_CLOEXEC_FLAG)
        .open("/")
        .map_err(|error| {
            SelectionAuditError::Io(format!("open audit filesystem root /: {error}"))
        })?;
    let mut traversed = PathBuf::from("/");
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => {
                traversed.push(name);
                directory =
                    openat_directory(&directory, name, &traversed, "traverse audit directory")?;
            }
            Component::CurDir | Component::ParentDir | Component::Prefix(_) => {
                unreachable!("absolute normal path was validated")
            }
        }
    }
    Ok(directory)
}

fn open_or_create_absolute_directory_no_follow(path: &Path) -> Result<File, SelectionAuditError> {
    validate_absolute_normal_path(path)?;
    let mut directory = OpenOptions::new()
        .read(true)
        .custom_flags(O_NOFOLLOW_FLAG | O_NONBLOCK_FLAG | O_CLOEXEC_FLAG)
        .open("/")
        .map_err(|error| {
            SelectionAuditError::Io(format!("open audit filesystem root /: {error}"))
        })?;
    let mut traversed = PathBuf::from("/");
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => {
                traversed.push(name);
                match openat_file(&directory, name, O_RDONLY_FLAG, 0) {
                    Ok(next) => {
                        require_directory_metadata(&next, &traversed, "traverse audit directory")?;
                        directory = next;
                    }
                    Err(error) if error.kind() == ErrorKind::NotFound => {
                        mkdirat_component(&directory, name, &traversed)?;
                        directory = openat_directory(
                            &directory,
                            name,
                            &traversed,
                            "pin created audit directory",
                        )?;
                    }
                    Err(error) => {
                        return Err(SelectionAuditError::PathInvalid(format!(
                            "traverse audit directory {}: {error}",
                            traversed.display()
                        )));
                    }
                }
            }
            Component::CurDir | Component::ParentDir | Component::Prefix(_) => {
                unreachable!("absolute normal path was validated")
            }
        }
    }
    Ok(directory)
}

fn require_directory_metadata(
    file: &File,
    path: &Path,
    operation: &str,
) -> Result<fs::Metadata, SelectionAuditError> {
    let metadata = file.metadata().map_err(|error| {
        SelectionAuditError::Io(format!("{operation} metadata {}: {error}", path.display()))
    })?;
    if !metadata.is_dir() {
        return Err(SelectionAuditError::PathInvalid(format!(
            "{operation} did not resolve to a directory: {}",
            path.display()
        )));
    }
    Ok(metadata)
}

fn file_identity(
    file: &File,
    path: &Path,
    operation: &str,
) -> Result<FileIdentity, SelectionAuditError> {
    let metadata = file.metadata().map_err(|error| {
        SelectionAuditError::Io(format!("{operation} metadata {}: {error}", path.display()))
    })?;
    if !metadata.is_file() {
        return Err(SelectionAuditError::PathInvalid(format!(
            "{operation} did not resolve to a regular file: {}",
            path.display()
        )));
    }
    Ok(FileIdentity::from_metadata(&metadata))
}

fn directory_marker(
    file: &File,
    path: &Path,
    operation: &str,
) -> Result<DirectoryMutationMarker, SelectionAuditError> {
    require_directory_metadata(file, path, operation)
        .map(|metadata| DirectoryMutationMarker::from_metadata(&metadata))
}

fn unlock_after_failed_acquisition<T>(
    lock_file: &File,
    lock_path: &Path,
    primary: SelectionAuditError,
) -> Result<T, SelectionAuditError> {
    match FileExt::unlock(lock_file) {
        Ok(()) => Err(primary),
        Err(cleanup) => Err(SelectionAuditError::Lock(format!(
            "audit acquisition failed ({primary}); release audit lock {} also failed: {cleanup}",
            lock_path.display()
        ))),
    }
}

impl LockedSelectionAuditSession<'_> {
    fn ensure_usable(&self) -> Result<(), SelectionAuditError> {
        if self.poisoned {
            return Err(SelectionAuditError::ChainInvalid(
                "locked audit session is unusable after an append/readback failure".to_owned(),
            ));
        }
        Ok(())
    }

    fn revalidate_namespace_binding(&self) -> Result<(), SelectionAuditError> {
        if let Some(container) = &self.namespace_container {
            let before = directory_marker(
                &container.file,
                self.writer
                    .namespace_root
                    .parent()
                    .unwrap_or(&self.writer.namespace_root),
                "revalidate audit namespace container before identity check",
            )?;
            if container.enforce_initial_marker && before != container.marker {
                return Err(SelectionAuditError::PathInvalid(format!(
                    "audit namespace container mutated during locked session: {}",
                    self.writer.namespace_root.display()
                )));
            }
            let reopened = openat_directory(
                &container.file,
                &container.namespace_leaf,
                &self.writer.namespace_root,
                "revalidate pinned audit namespace",
            )?;
            let reopened_identity = FileIdentity::from_metadata(&require_directory_metadata(
                &reopened,
                &self.writer.namespace_root,
                "revalidate pinned audit namespace",
            )?);
            if reopened_identity != container.namespace_identity
                || reopened_identity != self.parent_identity
            {
                return Err(SelectionAuditError::PathInvalid(format!(
                    "audit namespace identity changed during locked session: {}",
                    self.writer.namespace_root.display()
                )));
            }
            let after = directory_marker(
                &container.file,
                self.writer
                    .namespace_root
                    .parent()
                    .unwrap_or(&self.writer.namespace_root),
                "revalidate audit namespace container after identity check",
            )?;
            if after != before {
                return Err(SelectionAuditError::PathInvalid(format!(
                    "audit namespace container changed while identity was checked: {}",
                    self.writer.namespace_root.display()
                )));
            }
        }

        let before = directory_marker(
            &self.parent_file,
            &self.writer.namespace_root,
            "revalidate audit parent before leaf checks",
        )?;
        if before != self.parent_marker || before.identity != self.parent_identity {
            return Err(SelectionAuditError::PathInvalid(format!(
                "audit parent directory mutated during locked session: {}",
                self.writer.namespace_root.display()
            )));
        }

        let lock = openat_regular(
            &self.parent_file,
            OsStr::new(AUDIT_LOCK_FILE_NAME),
            O_RDWR_FLAG,
            &self.writer.lock_path,
            "revalidate audit lock",
        )?;
        if file_identity(&lock, &self.writer.lock_path, "revalidate audit lock")?
            != self.lock_identity
        {
            return Err(SelectionAuditError::PathInvalid(format!(
                "audit lock identity changed during locked session: {}",
                self.writer.lock_path.display()
            )));
        }

        match &self.audit_data {
            PinnedAuditData::Absent => {
                if openat_regular_optional(
                    &self.parent_file,
                    OsStr::new(AUDIT_FILE_NAME),
                    O_RDWR_FLAG | O_APPEND_FLAG,
                    &self.writer.path,
                    "revalidate absent selection audit",
                )?
                .is_some()
                {
                    return Err(SelectionAuditError::PathInvalid(format!(
                        "selection audit appeared during locked session: {}",
                        self.writer.path.display()
                    )));
                }
            }
            PinnedAuditData::Present { identity, .. } => {
                let reopened = openat_regular(
                    &self.parent_file,
                    OsStr::new(AUDIT_FILE_NAME),
                    O_RDWR_FLAG | O_APPEND_FLAG,
                    &self.writer.path,
                    "revalidate selection audit",
                )?;
                if file_identity(&reopened, &self.writer.path, "revalidate selection audit")?
                    != *identity
                {
                    return Err(SelectionAuditError::PathInvalid(format!(
                        "selection audit identity changed during locked session: {}",
                        self.writer.path.display()
                    )));
                }
            }
        }

        let after = directory_marker(
            &self.parent_file,
            &self.writer.namespace_root,
            "revalidate audit parent after leaf checks",
        )?;
        if after != before {
            return Err(SelectionAuditError::PathInvalid(format!(
                "audit parent directory changed while leaf identities were checked: {}",
                self.writer.namespace_root.display()
            )));
        }
        Ok(())
    }

    fn create_and_pin_audit_file(&mut self) -> Result<(), SelectionAuditError> {
        if !matches!(&self.audit_data, PinnedAuditData::Absent) {
            return Ok(());
        }
        self.revalidate_namespace_binding()?;
        let file = openat_regular(
            &self.parent_file,
            OsStr::new(AUDIT_FILE_NAME),
            O_RDWR_FLAG | O_APPEND_FLAG | O_CREAT_FLAG | O_EXCL_FLAG,
            &self.writer.path,
            "create and pin selection audit",
        )?;
        let identity = file_identity(&file, &self.writer.path, "pin created selection audit")?;
        self.audit_data = PinnedAuditData::Present { file, identity };
        self.parent_marker = directory_marker(
            &self.parent_file,
            &self.writer.namespace_root,
            "capture audit parent after creating selection audit",
        )?;
        self.revalidate_namespace_binding()
    }

    fn scan_pinned_chain(
        &mut self,
        inspect_record: impl FnMut(&SelectionAuditRecord),
    ) -> Result<AuditValidationReceipt, SelectionAuditError> {
        match &mut self.audit_data {
            PinnedAuditData::Absent => Ok(AuditValidationReceipt {
                record_count: 0,
                tail_hash: None,
            }),
            PinnedAuditData::Present { file, .. } => {
                scan_validated_chain_file(file, &self.writer.path, inspect_record)
            }
        }
    }

    pub fn append(
        &mut self,
        mut record: SelectionAuditRecord,
    ) -> Result<AuditAppendReceipt, SelectionAuditError> {
        self.ensure_usable()?;
        validate_new_record(&record)?;
        record.previous_hash = self.validation.tail_hash.clone();
        record.record_hash = calculate_record_hash(&record)?;
        let serialized = serde_json::to_vec(&record).map_err(|error| {
            SelectionAuditError::InvalidRecord(format!(
                "serialize strict selection audit record: {error}"
            ))
        })?;

        let write_result = (|| {
            self.revalidate_namespace_binding()?;
            let created_audit_file = matches!(&self.audit_data, PinnedAuditData::Absent);
            self.create_and_pin_audit_file()?;
            let PinnedAuditData::Present { file, .. } = &mut self.audit_data else {
                return Err(SelectionAuditError::Io(
                    "selection audit descriptor was not retained after creation".to_owned(),
                ));
            };
            file.seek(SeekFrom::End(0)).map_err(|error| {
                SelectionAuditError::Io(format!(
                    "seek pinned audit {} for append: {error}",
                    self.writer.path.display()
                ))
            })?;
            file.write_all(&serialized).map_err(|error| {
                SelectionAuditError::Io(format!(
                    "append audit record through pinned descriptor {}: {error}",
                    self.writer.path.display()
                ))
            })?;
            file.write_all(b"\n").map_err(|error| {
                SelectionAuditError::Io(format!(
                    "append audit newline to {}: {error}",
                    self.writer.path.display()
                ))
            })?;
            file.flush().map_err(|error| {
                SelectionAuditError::Io(format!(
                    "flush audit {}: {error}",
                    self.writer.path.display()
                ))
            })?;
            file.sync_data().map_err(|error| {
                SelectionAuditError::Io(format!(
                    "sync audit {}: {error}",
                    self.writer.path.display()
                ))
            })?;
            if created_audit_file {
                self.parent_file.sync_all().map_err(|error| {
                    SelectionAuditError::Io(format!(
                        "sync audit parent {} after file creation: {error}",
                        self.writer.namespace_root.display()
                    ))
                })?;
            }
            self.revalidate_namespace_binding()
        })();
        if let Err(error) = write_result {
            self.poisoned = true;
            return Err(error);
        }

        let expected_count = self.validation.record_count.checked_add(1).ok_or_else(|| {
            self.poisoned = true;
            SelectionAuditError::ChainInvalid(
                "selection audit record count overflow after append".to_owned(),
            )
        })?;
        let readback = self.scan_pinned_chain(|_| {}).inspect_err(|_| {
            self.poisoned = true;
        })?;
        if readback.record_count != expected_count
            || readback.tail_hash.as_deref() != Some(record.record_hash.as_str())
        {
            self.poisoned = true;
            return Err(SelectionAuditError::ChainInvalid(
                "synced audit append did not match descriptor-bound readback".to_owned(),
            ));
        }
        self.validation = readback;

        Ok(AuditAppendReceipt {
            record_hash: record.record_hash,
            previous_hash: record.previous_hash,
        })
    }

    pub fn validate(&mut self) -> Result<AuditValidationReceipt, SelectionAuditError> {
        self.ensure_usable()?;
        self.revalidate_namespace_binding().inspect_err(|_| {
            self.poisoned = true;
        })?;
        let validation = self.scan_pinned_chain(|_| {}).inspect_err(|_| {
            self.poisoned = true;
        })?;
        self.revalidate_namespace_binding().inspect_err(|_| {
            self.poisoned = true;
        })?;
        self.validation = validation.clone();
        Ok(validation)
    }

    /// Reads every audit record only after validating the complete on-disk
    /// chain under this session's process-local and cross-process locks.
    pub fn validated_records(
        &mut self,
    ) -> Result<ValidatedAuditChainSnapshot, SelectionAuditError> {
        self.ensure_usable()?;

        let mut records = Vec::new();
        self.revalidate_namespace_binding().inspect_err(|_| {
            self.poisoned = true;
        })?;
        let validation = self
            .scan_pinned_chain(|record| records.push(record.clone()))
            .inspect_err(|_| {
                self.poisoned = true;
            })?;
        self.revalidate_namespace_binding().inspect_err(|_| {
            self.poisoned = true;
        })?;
        self.validation = validation.clone();
        if records.len() != validation.record_count {
            self.poisoned = true;
            return Err(SelectionAuditError::ChainInvalid(
                "validated audit snapshot record count mismatch".to_owned(),
            ));
        }

        Ok(ValidatedAuditChainSnapshot {
            validation,
            records,
        })
    }

    pub fn lookup_exact(
        &mut self,
        phase: SelectionAuditPhase,
        subject_id: &str,
        content_hash: &str,
    ) -> Result<AuditExactLookup, SelectionAuditError> {
        self.ensure_usable()?;
        for (field, value) in [("subject_id", subject_id), ("content_hash", content_hash)] {
            if value.trim().is_empty() {
                return Err(SelectionAuditError::InvalidRecord(format!(
                    "audit exact lookup {field} must not be empty"
                )));
            }
        }

        let mut same_identity = Vec::new();
        self.revalidate_namespace_binding().inspect_err(|_| {
            self.poisoned = true;
        })?;
        let validation = self
            .scan_pinned_chain(|record| {
                if record.phase == phase && record.subject_id == subject_id {
                    same_identity.push(record.clone());
                }
            })
            .inspect_err(|_| {
                self.poisoned = true;
            })?;
        self.revalidate_namespace_binding().inspect_err(|_| {
            self.poisoned = true;
        })?;
        self.validation = validation;

        match same_identity.len() {
            0 => Ok(AuditExactLookup::Missing),
            1 => {
                let existing_record = same_identity.pop().ok_or_else(|| {
                    self.poisoned = true;
                    SelectionAuditError::ChainInvalid(
                        "audit exact lookup lost its sole validated record".to_owned(),
                    )
                })?;
                if existing_record.content_hash == content_hash {
                    Ok(AuditExactLookup::Exact(existing_record))
                } else {
                    Ok(AuditExactLookup::ContentConflict { existing_record })
                }
            }
            duplicate_count => {
                self.poisoned = true;
                Err(SelectionAuditError::ChainInvalid(format!(
                    "audit exact lookup found {duplicate_count} records for phase {phase:?} and subject_id {subject_id:?}"
                )))
            }
        }
    }

    pub fn finish(mut self) -> Result<AuditValidationReceipt, SelectionAuditError> {
        let validation = if self.poisoned {
            Err(SelectionAuditError::ChainInvalid(
                "locked audit session ended after an append/readback failure".to_owned(),
            ))
        } else {
            self.revalidate_namespace_binding()
                .and_then(|()| self.scan_pinned_chain(|_| {}))
                .and_then(|receipt| {
                    self.revalidate_namespace_binding()?;
                    Ok(receipt)
                })
        };
        let unlock = self.unlock();
        match (validation, unlock) {
            (Ok(receipt), Ok(())) => Ok(receipt),
            (Err(error), Ok(())) => Err(error),
            (Err(error), Err(cleanup)) => Err(SelectionAuditError::Lock(format!(
                "audit validation failed ({error}); explicit cleanup also failed ({cleanup})"
            ))),
            (Ok(_), Err(error)) => Err(error),
        }
    }

    fn unlock(&mut self) -> Result<(), SelectionAuditError> {
        let unlock_result = if let Some(lock_file) = self.lock_file.take() {
            FileExt::unlock(&lock_file).map_err(|error| {
                SelectionAuditError::Lock(format!(
                    "release audit lock {}: {error}",
                    self.writer.lock_path.display()
                ))
            })
        } else {
            Ok(())
        };
        self.process_guard.take();
        #[cfg(test)]
        if self.inject_unlock_failure && unlock_result.is_ok() {
            return Err(SelectionAuditError::Lock(
                "injected TEST_CODE audit unlock failure".to_owned(),
            ));
        }
        unlock_result
    }
}

impl Drop for LockedSelectionAuditSession<'_> {
    fn drop(&mut self) {
        if self.lock_file.is_some() {
            match self.unlock() {
                Ok(()) => {
                    log::warn!(
                        "[selection-audit][BR-157] locked audit session dropped without explicit finish: {}",
                        self.writer.path.display()
                    );
                }
                Err(error) => {
                    log::error!(
                        "[selection-audit][BR-157] implicit audit-session cleanup failed: {error}"
                    );
                }
            }
        }
        self.process_guard.take();
    }
}

fn process_audit_lock() -> &'static Mutex<()> {
    static PROCESS_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    PROCESS_LOCK.get_or_init(|| Mutex::new(()))
}

fn finish_session_operation<T>(
    session: LockedSelectionAuditSession<'_>,
    operation: Result<T, SelectionAuditError>,
) -> Result<T, SelectionAuditError> {
    let finish = session.finish();
    match (operation, finish) {
        (Ok(value), Ok(_)) => Ok(value),
        (Err(error), Ok(_)) => Err(error),
        (Err(error), Err(cleanup)) => Err(SelectionAuditError::Lock(format!(
            "audit operation failed ({error}); explicit finish also failed ({cleanup})"
        ))),
        (Ok(_), Err(error)) => Err(error),
    }
}

#[derive(Serialize)]
struct AuditHashPayload<'a> {
    schema_version: u16,
    domain: &'a str,
    phase: SelectionAuditPhase,
    subject_id: &'a str,
    content_hash: &'a str,
    context: &'a SelectionAuditContext,
    previous_hash: &'a Option<String>,
    recorded_at: &'a DateTime<FixedOffset>,
}

fn calculate_record_hash(record: &SelectionAuditRecord) -> Result<String, SelectionAuditError> {
    let payload = AuditHashPayload {
        schema_version: record.schema_version,
        domain: &record.domain,
        phase: record.phase,
        subject_id: &record.subject_id,
        content_hash: &record.content_hash,
        context: &record.context,
        previous_hash: &record.previous_hash,
        recorded_at: &record.recorded_at,
    };
    let bytes = serde_json::to_vec(&payload).map_err(|error| {
        SelectionAuditError::InvalidRecord(format!(
            "serialize selection audit hash payload: {error}"
        ))
    })?;
    let mut hasher = Sha256::new();
    hasher.update(b"stock_analysis.selection_audit_record.v1\0");
    hasher.update(bytes);
    Ok(hex::encode(hasher.finalize()))
}

fn validate_new_record(record: &SelectionAuditRecord) -> Result<(), SelectionAuditError> {
    if record.previous_hash.is_some() || !record.record_hash.is_empty() {
        return Err(SelectionAuditError::InvalidRecord(
            "caller must not supply previous_hash or record_hash".to_owned(),
        ));
    }
    validate_record_fields(record, false)
}

fn validate_record_fields(
    record: &SelectionAuditRecord,
    persisted: bool,
) -> Result<(), SelectionAuditError> {
    let invalid = |message: String| {
        if persisted {
            SelectionAuditError::ChainInvalid(message)
        } else {
            SelectionAuditError::InvalidRecord(message)
        }
    };
    if record.schema_version != AUDIT_SCHEMA_VERSION {
        return Err(invalid(format!(
            "unsupported schema_version {}, expected {AUDIT_SCHEMA_VERSION}",
            record.schema_version
        )));
    }
    if record.domain != AUDIT_DOMAIN {
        return Err(invalid(format!(
            "invalid audit domain {:?}, expected {AUDIT_DOMAIN:?}",
            record.domain
        )));
    }
    for (field, value) in [
        ("subject_id", record.subject_id.as_str()),
        ("content_hash", record.content_hash.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(invalid(format!("{field} must not be empty")));
        }
    }
    for (field, value) in [
        (
            "event_identity_hash",
            record.context.event_identity_hash.as_deref(),
        ),
        (
            "chain_identity_hash",
            record.context.chain_identity_hash.as_deref(),
        ),
        (
            "security_identity_hash",
            record.context.security_identity_hash.as_deref(),
        ),
        ("provider", record.context.provider.as_deref()),
        (
            "magic_tdx_batch_id",
            record.context.magic_tdx_batch_id.as_deref(),
        ),
    ] {
        if value.is_some_and(|value| value.trim().is_empty()) {
            return Err(invalid(format!("{field} must not be blank when present")));
        }
    }
    if record
        .context
        .reason_codes
        .iter()
        .any(|code| code.trim().is_empty())
    {
        return Err(invalid("reason_codes contain a blank code".to_owned()));
    }
    if record
        .context
        .rule_ids
        .iter()
        .any(|rule| rule.trim().is_empty())
    {
        return Err(invalid("rule_ids contain a blank rule".to_owned()));
    }
    if record.phase == SelectionAuditPhase::Rejected
        && (record.context.event_identity_hash.is_none()
            || record.context.reason_codes.is_empty()
            || record.context.rule_ids.is_empty()
            || record.context.retryable.is_none())
    {
        return Err(invalid(
            "rejected audit requires event identity, reasons, rule IDs, and retryable".to_owned(),
        ));
    }
    if persisted && record.record_hash.trim().is_empty() {
        return Err(SelectionAuditError::ChainInvalid(
            "persisted record_hash is empty".to_owned(),
        ));
    }
    Ok(())
}

fn scan_validated_chain_file(
    file: &mut File,
    diagnostic_path: &Path,
    mut inspect_record: impl FnMut(&SelectionAuditRecord),
) -> Result<AuditValidationReceipt, SelectionAuditError> {
    file.seek(SeekFrom::Start(0)).map_err(|error| {
        SelectionAuditError::Io(format!(
            "seek pinned audit {} for validation: {error}",
            diagnostic_path.display()
        ))
    })?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(|error| {
        SelectionAuditError::Io(format!(
            "read pinned audit {}: {error}",
            diagnostic_path.display()
        ))
    })?;
    if bytes.is_empty() {
        return Ok(AuditValidationReceipt {
            record_count: 0,
            tail_hash: None,
        });
    }
    if bytes.last() != Some(&b'\n') {
        return Err(SelectionAuditError::ChainInvalid(format!(
            "audit {} has a partial tail or missing final newline",
            diagnostic_path.display()
        )));
    }

    let mut expected_previous: Option<String> = None;
    let mut record_count = 0;
    for (index, line) in bytes[..bytes.len() - 1]
        .split(|byte| *byte == b'\n')
        .enumerate()
    {
        if line.is_empty() {
            return Err(SelectionAuditError::ChainInvalid(format!(
                "audit {} line {} is empty",
                diagnostic_path.display(),
                index + 1
            )));
        }
        let record = serde_json::from_slice::<SelectionAuditRecord>(line).map_err(|error| {
            SelectionAuditError::ChainInvalid(format!(
                "audit {} line {} is not a strict record: {error}",
                diagnostic_path.display(),
                index + 1
            ))
        })?;
        validate_record_fields(&record, true)?;
        if record.previous_hash != expected_previous {
            return Err(SelectionAuditError::ChainInvalid(format!(
                "audit {} line {} previous_hash mismatch",
                diagnostic_path.display(),
                index + 1
            )));
        }
        let expected_hash = calculate_record_hash(&record).map_err(|error| {
            SelectionAuditError::ChainInvalid(format!(
                "audit {} line {} hash calculation failed: {error}",
                diagnostic_path.display(),
                index + 1
            ))
        })?;
        if record.record_hash != expected_hash {
            return Err(SelectionAuditError::ChainInvalid(format!(
                "audit {} line {} record_hash mismatch",
                diagnostic_path.display(),
                index + 1
            )));
        }
        inspect_record(&record);
        expected_previous = Some(record.record_hash);
        record_count += 1;
    }
    Ok(AuditValidationReceipt {
        record_count,
        tail_hash: expected_previous,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use std::sync::{mpsc, Arc, Barrier};
    use std::thread;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    struct TempAuditRoot(PathBuf);

    impl TempAuditRoot {
        fn new(label: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos();
            // macOS exposes `/var` as a symlink to `/private/var`. Production
            // audit paths must still reject symlink components, so tests bind
            // to the canonical temporary-directory authority before appending
            // their non-existent isolated leaf.
            let canonical_temp_root =
                fs::canonicalize(std::env::temp_dir()).expect("canonicalize test temp root");
            let path = canonical_temp_root.join(format!(
                "stock-analysis-selection-audit-{label}-{}-{nonce}",
                std::process::id()
            ));
            Self(path)
        }
    }

    impl Drop for TempAuditRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn timestamp(second: u32) -> DateTime<FixedOffset> {
        FixedOffset::east_opt(8 * 60 * 60)
            .expect("offset")
            .with_ymd_and_hms(2026, 7, 23, 10, 0, second)
            .single()
            .expect("timestamp")
    }

    fn record(phase: SelectionAuditPhase, subject: &str) -> SelectionAuditRecord {
        SelectionAuditRecord::new(
            phase,
            subject,
            format!("{subject}-content-hash"),
            timestamp(0),
        )
        .with_context(SelectionAuditContext {
            event_identity_hash: Some(format!("{subject}-event-hash")),
            reason_codes: if phase == SelectionAuditPhase::Rejected {
                vec!["trend_alignment_failed".to_owned()]
            } else {
                Vec::new()
            },
            rule_ids: vec!["BR-157".to_owned()],
            retryable: (phase == SelectionAuditPhase::Rejected).then_some(false),
            ..SelectionAuditContext::default()
        })
    }

    fn test_writer(root: &TempAuditRoot) -> SelectionAuditWriter {
        SelectionAuditWriter::for_test_code_root(&root.0)
            .expect("construct isolated TEST_CODE audit writer")
    }

    #[test]
    fn prepared_then_committed_returns_chain_hash_receipt() {
        let root = TempAuditRoot::new("chain");
        let writer = test_writer(&root);
        let prepared = writer
            .append(record(SelectionAuditPhase::Prepared, "TEST_CODE_run-1"))
            .expect("prepared audit");
        let committed = writer
            .append(record(SelectionAuditPhase::Committed, "TEST_CODE_run-1"))
            .expect("committed audit");
        assert_eq!(committed.previous_hash, Some(prepared.record_hash));
        assert_eq!(
            writer.validate().expect("valid chain"),
            AuditValidationReceipt {
                record_count: 2,
                tail_hash: Some(committed.record_hash),
            }
        );
    }

    #[test]
    fn unknown_field_or_corrupted_hash_blocks_append() {
        for (label, replacement) in [
            (
                "unknown",
                r#"{"schema_version":1,"domain":"stock_analysis.selection_audit.v1","phase":"prepared","subject_id":"TEST_CODE_run","content_hash":"content","context":{"event_identity_hash":null,"chain_identity_hash":null,"security_identity_hash":null,"provider":null,"provider_published_at":null,"observed_at":null,"magic_tdx_batch_id":null,"reason_codes":[],"rule_ids":[],"retryable":null},"previous_hash":null,"recorded_at":"2026-07-23T10:00:00+08:00","record_hash":"bad","unknown":true}
"#,
            ),
            (
                "hash",
                r#"{"schema_version":1,"domain":"stock_analysis.selection_audit.v1","phase":"prepared","subject_id":"TEST_CODE_run","content_hash":"content","context":{"event_identity_hash":null,"chain_identity_hash":null,"security_identity_hash":null,"provider":null,"provider_published_at":null,"observed_at":null,"magic_tdx_batch_id":null,"reason_codes":[],"rule_ids":[],"retryable":null},"previous_hash":null,"recorded_at":"2026-07-23T10:00:00+08:00","record_hash":"bad"}
"#,
            ),
        ] {
            let root = TempAuditRoot::new(label);
            let writer = test_writer(&root);
            fs::create_dir_all(writer.path().parent().expect("parent")).expect("create dir");
            fs::write(writer.path(), replacement).expect("corrupt audit");
            let error = writer
                .append(record(SelectionAuditPhase::Committed, "TEST_CODE_run"))
                .expect_err("corrupt chain must block");
            assert_eq!(error.code(), "audit_chain_invalid");
        }
    }

    #[test]
    fn missing_final_newline_blocks_append() {
        let root = TempAuditRoot::new("newline");
        let writer = test_writer(&root);
        writer
            .append(record(SelectionAuditPhase::Prepared, "TEST_CODE_run"))
            .expect("first record");
        let bytes = fs::read(writer.path()).expect("read audit");
        fs::write(writer.path(), &bytes[..bytes.len() - 1]).expect("remove newline");

        let error = writer
            .append(record(SelectionAuditPhase::Committed, "TEST_CODE_run"))
            .expect_err("partial tail must block");
        assert_eq!(error.code(), "audit_chain_invalid");
    }

    #[test]
    fn caller_cannot_supply_previous_or_record_hash() {
        let root = TempAuditRoot::new("caller-hash");
        let writer = test_writer(&root);
        let mut supplied = record(SelectionAuditPhase::Prepared, "TEST_CODE_run");
        supplied.previous_hash = Some("caller-controlled".to_owned());
        supplied.record_hash = "caller-controlled".to_owned();
        let error = writer
            .append(supplied)
            .expect_err("writer owns linkage and hash");
        assert_eq!(error.code(), "audit_record_invalid");
    }

    #[test]
    fn production_path_is_fixed_to_cargo_manifest_and_test_path_is_isolated() {
        let root = TempAuditRoot::new("isolation");
        let production = SelectionAuditWriter::production().expect("fixed production writer");
        let test = test_writer(&root);
        let expected_production_root =
            Path::new(env!("CARGO_MANIFEST_DIR")).join(PRODUCTION_AUDIT_ROOT_RELATIVE_PATH);
        assert_eq!(production.namespace_root, expected_production_root);
        assert_eq!(
            production.path(),
            expected_production_root.join("selection-audit.jsonl")
        );
        assert_eq!(test.namespace_root, root.0.join("test"));
        assert_ne!(production.path(), test.path());
        assert_ne!(production.lock_path(), test.lock_path());
        assert!(!root.0.join("production").exists());
    }

    #[test]
    fn test_constructor_rejects_parent_components() {
        let root = TempAuditRoot::new("parent-component");
        let supplied = root.0.join("safe").join("..").join("escape");
        let error = SelectionAuditWriter::for_test_code_root(supplied)
            .expect_err("parent components must not escape the TEST_CODE namespace");
        assert_eq!(error.code(), "audit_path_invalid");
    }

    #[cfg(unix)]
    #[test]
    fn test_constructor_rejects_existing_symlink_components() {
        use std::os::unix::fs::symlink;

        let root = TempAuditRoot::new("symlink-component");
        let real = root.0.join("real");
        let linked = root.0.join("linked");
        fs::create_dir_all(&real).expect("create real audit root");
        symlink(&real, &linked).expect("create audit-root symlink");

        let error = SelectionAuditWriter::for_test_code_root(&linked)
            .expect_err("existing symlink component must fail closed");
        assert_eq!(error.code(), "audit_path_invalid");
    }

    #[cfg(unix)]
    #[test]
    fn test_constructor_rejects_symlinked_audit_and_lock_files() {
        use std::os::unix::fs::symlink;

        for file_name in ["selection-audit.jsonl", "selection-audit.lock"] {
            let root = TempAuditRoot::new(file_name);
            let namespace = root.0.join("test");
            let target = root.0.join("target");
            fs::create_dir_all(&namespace).expect("create test namespace");
            fs::write(&target, b"not authoritative").expect("create symlink target");
            symlink(&target, namespace.join(file_name)).expect("create audit-file symlink");

            let error = SelectionAuditWriter::for_test_code_root(&root.0)
                .expect_err("audit and lock file symlinks must fail closed");
            assert_eq!(error.code(), "audit_path_invalid");
        }
    }

    #[cfg(unix)]
    #[test]
    fn locked_session_rejects_symlink_inserted_after_construction() {
        use std::os::unix::fs::symlink;

        let root = TempAuditRoot::new("late-symlink");
        fs::create_dir_all(&root.0).expect("create test container");
        let supplied = root.0.join("supplied");
        let real = root.0.join("real");
        let writer = SelectionAuditWriter::for_test_code_root(&supplied)
            .expect("missing TEST_CODE namespace is valid before use");
        fs::create_dir_all(&real).expect("create symlink target");
        symlink(&real, &supplied).expect("insert symlink after construction");

        let error = match writer.locked_session() {
            Ok(_) => panic!("locked session must revalidate bound paths"),
            Err(error) => error,
        };
        assert_eq!(error.code(), "audit_path_invalid");
    }

    #[test]
    fn concurrent_writers_form_one_valid_serialized_chain() {
        let root = TempAuditRoot::new("concurrent");
        let writer = Arc::new(test_writer(&root));
        let barrier = Arc::new(Barrier::new(8));
        let threads = (0..8)
            .map(|index| {
                let writer = Arc::clone(&writer);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    writer.append(SelectionAuditRecord::new(
                        SelectionAuditPhase::Prepared,
                        format!("TEST_CODE_run-{index}"),
                        format!("content-hash-{index}"),
                        timestamp(u32::try_from(index).expect("test index")),
                    ))
                })
            })
            .collect::<Vec<_>>();

        for handle in threads {
            handle
                .join()
                .expect("writer thread")
                .expect("concurrent append");
        }
        let receipt = writer.validate().expect("serialized chain");
        assert_eq!(receipt.record_count, 8);
        assert!(receipt.tail_hash.is_some());
    }

    #[test]
    fn append_writes_one_complete_json_line_and_sync_contract_is_readable() {
        let root = TempAuditRoot::new("jsonl");
        let writer = test_writer(&root);
        writer
            .append(record(SelectionAuditPhase::Rejected, "TEST_CODE_candidate"))
            .expect("append");
        let bytes = fs::read(writer.path()).expect("read");
        assert_eq!(bytes.last(), Some(&b'\n'));
        let parsed = serde_json::from_slice::<SelectionAuditRecord>(
            bytes.strip_suffix(b"\n").expect("newline"),
        )
        .expect("strict record");
        assert_eq!(
            parsed.context.reason_codes,
            ["trend_alignment_failed".to_owned()]
        );
        assert_ne!(
            parsed.recorded_at.with_timezone(&Utc),
            DateTime::<Utc>::UNIX_EPOCH
        );

        let mut file = OpenOptions::new()
            .append(true)
            .open(writer.path())
            .expect("audit remains appendable");
        file.flush().expect("flush");
    }

    #[test]
    fn record_validation_rejects_every_blank_or_unsupported_identity_field() {
        let mut cases = Vec::new();

        let mut unsupported_schema = record(SelectionAuditPhase::Prepared, "TEST_CODE_schema");
        unsupported_schema.schema_version += 1;
        cases.push(unsupported_schema);

        let mut wrong_domain = record(SelectionAuditPhase::Prepared, "TEST_CODE_domain");
        wrong_domain.domain = "wrong.domain".to_owned();
        cases.push(wrong_domain);

        let mut blank_subject = record(SelectionAuditPhase::Prepared, "TEST_CODE_subject");
        blank_subject.subject_id = " ".to_owned();
        cases.push(blank_subject);

        let mut blank_content = record(SelectionAuditPhase::Prepared, "TEST_CODE_content");
        blank_content.content_hash = String::new();
        cases.push(blank_content);

        for field in [
            "event_identity_hash",
            "chain_identity_hash",
            "security_identity_hash",
            "provider",
            "magic_tdx_batch_id",
        ] {
            let mut item = record(SelectionAuditPhase::Prepared, "TEST_CODE_context");
            match field {
                "event_identity_hash" => item.context.event_identity_hash = Some(" ".to_owned()),
                "chain_identity_hash" => item.context.chain_identity_hash = Some(" ".to_owned()),
                "security_identity_hash" => {
                    item.context.security_identity_hash = Some(" ".to_owned())
                }
                "provider" => item.context.provider = Some(" ".to_owned()),
                "magic_tdx_batch_id" => item.context.magic_tdx_batch_id = Some(" ".to_owned()),
                _ => unreachable!("closed test field table"),
            }
            cases.push(item);
        }

        let mut blank_reason = record(SelectionAuditPhase::Prepared, "TEST_CODE_reason");
        blank_reason.context.reason_codes = vec![" ".to_owned()];
        cases.push(blank_reason);

        let mut blank_rule = record(SelectionAuditPhase::Prepared, "TEST_CODE_rule");
        blank_rule.context.rule_ids = vec![String::new()];
        cases.push(blank_rule);

        for item in cases {
            let error = validate_new_record(&item).expect_err("invalid field must fail");
            assert_eq!(error.code(), "audit_record_invalid");
        }
    }

    #[test]
    fn rejected_record_requires_complete_decision_context() {
        let complete = record(SelectionAuditPhase::Rejected, "TEST_CODE_rejected");
        for mutate in 0..4 {
            let mut incomplete = complete.clone();
            match mutate {
                0 => incomplete.context.event_identity_hash = None,
                1 => incomplete.context.reason_codes.clear(),
                2 => incomplete.context.rule_ids.clear(),
                3 => incomplete.context.retryable = None,
                _ => unreachable!("closed test range"),
            }
            let error = validate_new_record(&incomplete).expect_err("incomplete rejection");
            assert_eq!(error.code(), "audit_record_invalid");
        }
    }

    #[test]
    fn persisted_record_requires_hash_and_empty_audit_is_valid() {
        let root = TempAuditRoot::new("empty");
        let writer = test_writer(&root);
        fs::create_dir_all(writer.path().parent().expect("parent")).expect("create parent");
        fs::write(writer.path(), []).expect("empty file");
        assert_eq!(
            writer.validate().expect("empty chain"),
            AuditValidationReceipt {
                record_count: 0,
                tail_hash: None,
            }
        );

        let persisted = record(SelectionAuditPhase::Prepared, "TEST_CODE_persisted");
        let error =
            validate_record_fields(&persisted, true).expect_err("persisted hash is mandatory");
        assert_eq!(error.code(), "audit_chain_invalid");
    }

    #[test]
    fn blank_jsonl_line_and_uncreatable_namespace_fail_explicitly() {
        let blank_root = TempAuditRoot::new("blank-line");
        let blank_writer = test_writer(&blank_root);
        fs::create_dir_all(blank_writer.path().parent().expect("parent")).expect("create parent");
        fs::write(blank_writer.path(), b"\n").expect("blank line");
        assert_eq!(
            blank_writer
                .validate()
                .expect_err("blank line is corruption")
                .code(),
            "audit_chain_invalid"
        );

        let blocked_root = TempAuditRoot::new("blocked-parent");
        let test_namespace = blocked_root.0.join("test");
        fs::create_dir_all(&blocked_root.0).expect("root");
        fs::write(&test_namespace, b"not a directory").expect("blocking file");
        let error = SelectionAuditWriter::for_test_code_root(&blocked_root.0)
            .expect_err("invalid namespace must fail during construction");
        assert_eq!(error.code(), "audit_path_invalid");
    }

    #[test]
    fn stable_error_codes_cover_all_audit_failure_classes() {
        assert_eq!(
            SelectionAuditError::ChainInvalid("x".to_owned()).code(),
            "audit_chain_invalid"
        );
        assert_eq!(
            SelectionAuditError::InvalidRecord("x".to_owned()).code(),
            "audit_record_invalid"
        );
        assert_eq!(
            SelectionAuditError::Lock("x".to_owned()).code(),
            "audit_lock_failed"
        );
        assert_eq!(
            SelectionAuditError::Io("x".to_owned()).code(),
            "audit_io_failure"
        );
        assert_eq!(
            SelectionAuditError::PathInvalid("x".to_owned()).code(),
            "audit_path_invalid"
        );
    }

    #[test]
    fn permanent_v2_phases_round_trip_with_historical_phases() {
        for (phase, encoded) in [
            (SelectionAuditPhase::Prepared, "\"prepared\""),
            (SelectionAuditPhase::Committed, "\"committed\""),
            (
                SelectionAuditPhase::V2ConfigActivationPrepared,
                "\"v2_config_activation_prepared\"",
            ),
            (
                SelectionAuditPhase::V2ConfigActivationCommitted,
                "\"v2_config_activation_committed\"",
            ),
            (
                SelectionAuditPhase::V2IngressPrepared,
                "\"v2_ingress_prepared\"",
            ),
            (
                SelectionAuditPhase::V2IngressCommitted,
                "\"v2_ingress_committed\"",
            ),
            (
                SelectionAuditPhase::V2GenerationPrepared,
                "\"v2_generation_prepared\"",
            ),
            (
                SelectionAuditPhase::V2GenerationCommitted,
                "\"v2_generation_committed\"",
            ),
            (
                SelectionAuditPhase::V2OutcomePrepared,
                "\"v2_outcome_prepared\"",
            ),
            (
                SelectionAuditPhase::V2OutcomeCommitted,
                "\"v2_outcome_committed\"",
            ),
            (
                SelectionAuditPhase::V2BoardBindingAuditPrepared,
                "\"v2_board_binding_audit_prepared\"",
            ),
            (
                SelectionAuditPhase::V2BoardBindingAuditCommitted,
                "\"v2_board_binding_audit_committed\"",
            ),
            (
                SelectionAuditPhase::V2GateDCanaryVerified,
                "\"v2_gate_d_canary_verified\"",
            ),
        ] {
            assert_eq!(
                serde_json::to_string(&phase).expect("serialize phase"),
                encoded
            );
            assert_eq!(
                serde_json::from_str::<SelectionAuditPhase>(encoded).expect("deserialize phase"),
                phase
            );
        }
    }

    #[test]
    fn locked_session_appends_two_records_under_one_acquisition() {
        let root = TempAuditRoot::new("locked-session-two-appends");
        let writer = test_writer(&root);
        let mut session = writer.locked_session().expect("acquire locked session");

        let prepared = session
            .append(record(
                SelectionAuditPhase::V2BoardBindingAuditPrepared,
                "TEST_CODE_board-binding",
            ))
            .expect("prepared append");
        let committed = session
            .append(record(
                SelectionAuditPhase::V2BoardBindingAuditCommitted,
                "TEST_CODE_board-binding",
            ))
            .expect("committed append");

        assert_eq!(committed.previous_hash, Some(prepared.record_hash));
        assert_eq!(
            session.finish().expect("finish locked session"),
            AuditValidationReceipt {
                record_count: 2,
                tail_hash: Some(committed.record_hash),
            }
        );
    }

    #[test]
    fn locked_session_excludes_a_concurrent_writer_until_release() {
        let root = TempAuditRoot::new("locked-session-exclusion");
        let writer = test_writer(&root);
        let session = writer.locked_session().expect("first session");
        let competing_writer = writer.clone();
        let (started_tx, started_rx) = mpsc::channel();
        let (completed_tx, completed_rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            started_tx.send(()).expect("announce competing writer");
            let result = competing_writer.append(record(
                SelectionAuditPhase::Prepared,
                "TEST_CODE_competing-writer",
            ));
            completed_tx.send(result).expect("send append result");
        });

        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("competing writer started");
        assert!(
            completed_rx
                .recv_timeout(Duration::from_millis(100))
                .is_err(),
            "competing writer must remain blocked while the session owns the lock"
        );

        session.finish().expect("release first session");
        completed_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("competing writer unblocked")
            .expect("competing append");
        handle.join().expect("competing writer thread");
        assert_eq!(writer.validate().expect("valid chain").record_count, 1);
    }

    #[test]
    fn corrupted_chain_blocks_locked_session_acquisition() {
        let root = TempAuditRoot::new("locked-session-corrupt");
        let writer = test_writer(&root);
        fs::create_dir_all(writer.path().parent().expect("parent")).expect("create parent");
        fs::write(writer.path(), b"{}\n").expect("write corrupt chain");

        let error = writer
            .locked_session()
            .err()
            .expect("corrupt chain must prevent session construction");
        assert_eq!(error.code(), "audit_chain_invalid");
    }

    #[test]
    fn locked_session_sync_is_immediately_readable_and_revalidates() {
        let root = TempAuditRoot::new("locked-session-sync-readback");
        let writer = test_writer(&root);
        let mut session = writer.locked_session().expect("locked session");
        let append = session
            .append(record(
                SelectionAuditPhase::V2GateDCanaryVerified,
                "TEST_CODE_gate-d-canary",
            ))
            .expect("synced append");

        assert_eq!(
            session.validate().expect("read back while still locked"),
            AuditValidationReceipt {
                record_count: 1,
                tail_hash: Some(append.record_hash.clone()),
            }
        );
        let bytes = fs::read(writer.path()).expect("synced file is readable");
        assert_eq!(bytes.last(), Some(&b'\n'));
        let persisted = serde_json::from_slice::<SelectionAuditRecord>(&bytes[..bytes.len() - 1])
            .expect("strict persisted record");
        assert_eq!(persisted.record_hash, append.record_hash);
        session.finish().expect("release session");
    }

    #[cfg(unix)]
    #[test]
    fn locked_session_rejects_audit_leaf_path_swap_without_reading_replacement() {
        let root = TempAuditRoot::new("locked-session-audit-path-swap");
        let writer = test_writer(&root);
        writer
            .append(record(
                SelectionAuditPhase::Prepared,
                "TEST_CODE_pinned-audit",
            ))
            .expect("seed valid audit");
        let mut session = writer.locked_session().expect("pin audit descriptors");
        let pinned_identity = match &session.audit_data {
            PinnedAuditData::Present { identity, .. } => *identity,
            PinnedAuditData::Absent => panic!("seeded audit must have a pinned descriptor"),
        };
        assert_eq!(
            pinned_identity,
            FileIdentity::from_metadata(
                &fs::metadata(writer.path()).expect("seeded audit metadata")
            )
        );
        let displaced = writer.path().with_extension("displaced");
        fs::rename(writer.path(), &displaced).expect("displace pinned audit");
        fs::write(writer.path(), b"{\"replacement\":true}\n").expect("install replacement audit");
        assert_ne!(
            pinned_identity,
            FileIdentity::from_metadata(
                &fs::metadata(writer.path()).expect("replacement audit metadata")
            ),
            "replacement path must resolve to a different inode than the retained descriptor"
        );

        let error = session
            .validated_records()
            .expect_err("same session must reject replacement audit identity");
        assert_eq!(error.code(), "audit_path_invalid");
        assert!(
            matches!(&session.audit_data, PinnedAuditData::Present { .. }),
            "the original audit descriptor remains retained until explicit cleanup"
        );
        assert_eq!(
            session
                .finish()
                .expect_err("poisoned session cannot report authoritative success")
                .code(),
            "audit_chain_invalid"
        );
    }

    #[cfg(unix)]
    #[test]
    fn locked_session_rejects_audit_leaf_aba_from_parent_mutation_marker() {
        let root = TempAuditRoot::new("locked-session-audit-aba");
        let writer = test_writer(&root);
        writer
            .append(record(
                SelectionAuditPhase::Prepared,
                "TEST_CODE_pinned-audit-aba",
            ))
            .expect("seed valid audit");
        let mut session = writer.locked_session().expect("pin audit descriptors");
        let initial_marker = session.parent_marker;
        let displaced = writer.path().with_extension("aba");
        thread::sleep(Duration::from_millis(2));
        fs::rename(writer.path(), &displaced).expect("temporarily displace audit");
        fs::rename(&displaced, writer.path()).expect("restore same audit inode");
        let changed_marker = directory_marker(
            &session.parent_file,
            &writer.namespace_root,
            "test audit ABA marker",
        )
        .expect("read retained parent marker");
        assert_ne!(
            changed_marker, initial_marker,
            "the test filesystem must expose the namespace ABA mutation"
        );

        let error = session
            .validate()
            .expect_err("same-inode ABA must fail closed");
        assert_eq!(error.code(), "audit_path_invalid");
        assert_eq!(
            session
                .finish()
                .expect_err("poisoned ABA session cannot succeed")
                .code(),
            "audit_chain_invalid"
        );
    }

    #[cfg(unix)]
    #[test]
    fn locked_session_rejects_namespace_path_swap_against_pinned_container() {
        let root = TempAuditRoot::new("locked-session-namespace-swap");
        let writer = test_writer(&root);
        writer
            .append(record(
                SelectionAuditPhase::Prepared,
                "TEST_CODE_pinned-namespace",
            ))
            .expect("seed valid audit");
        let mut session = writer.locked_session().expect("pin namespace descriptors");
        let displaced = root.0.join("test-displaced");
        fs::rename(&writer.namespace_root, &displaced).expect("displace audit namespace");
        fs::create_dir(&writer.namespace_root).expect("install replacement namespace");

        let error = session
            .lookup_exact(
                SelectionAuditPhase::Prepared,
                "TEST_CODE_pinned-namespace",
                "TEST_CODE_pinned-namespace-content-hash",
            )
            .expect_err("replacement namespace must not redirect exact lookup");
        assert_eq!(error.code(), "audit_path_invalid");
        assert_eq!(
            session
                .finish()
                .expect_err("poisoned namespace session cannot succeed")
                .code(),
            "audit_chain_invalid"
        );
    }

    #[cfg(unix)]
    #[test]
    fn pinned_test_root_descriptor_remains_authority_after_diagnostic_path_swap() {
        let root = TempAuditRoot::new("pinned-test-root-path-swap");
        fs::create_dir(&root.0).expect("create TEST_CODE root");
        let root_descriptor = File::open(&root.0).expect("pin TEST_CODE root");
        let writer = SelectionAuditWriter::for_test_code_pinned_root(&root_descriptor, &root.0)
            .expect("bind audit writer to retained TEST_CODE root");
        writer
            .append(record(
                SelectionAuditPhase::Prepared,
                "TEST_CODE_descriptor-root",
            ))
            .expect("append through descriptor-bound writer");
        let mut session = writer.locked_session().expect("pin audit session");

        let displaced = root.0.with_extension("descriptor-root");
        fs::rename(&root.0, &displaced).expect("displace diagnostic TEST_CODE path");
        fs::create_dir(&root.0).expect("install unrelated replacement root");
        fs::create_dir(root.0.join("test")).expect("install unrelated replacement namespace");
        fs::write(
            root.0.join("test").join(AUDIT_FILE_NAME),
            b"{\"replacement\":true}\n",
        )
        .expect("install unrelated replacement audit");

        let expected_tail = session.validation.tail_hash.clone();
        assert_eq!(
            session
                .validate()
                .expect("retained descriptor stays authoritative"),
            AuditValidationReceipt {
                record_count: 1,
                tail_hash: expected_tail,
            }
        );
        session.finish().expect("explicitly finish pinned session");

        fs::remove_dir_all(&root.0).expect("remove replacement diagnostic root");
        fs::rename(&displaced, &root.0).expect("restore retained TEST_CODE root for cleanup");
    }

    #[test]
    fn explicit_finish_surfaces_unlock_cleanup_failure() {
        let root = TempAuditRoot::new("locked-session-cleanup-failure");
        let writer = test_writer(&root);
        let mut session = writer.locked_session().expect("acquire locked session");
        session.inject_unlock_failure = true;

        let error = session
            .finish()
            .expect_err("authoritative success must expose cleanup failure");
        assert_eq!(error.code(), "audit_lock_failed");
    }

    #[test]
    fn locked_validated_records_returns_one_chain_consistent_snapshot() {
        let root = TempAuditRoot::new("locked-validated-records");
        let writer = test_writer(&root);
        let first = writer
            .append(record(
                SelectionAuditPhase::V2IngressPrepared,
                "TEST_CODE_ingress-run",
            ))
            .expect("first append");
        let second = writer
            .append(record(
                SelectionAuditPhase::V2IngressCommitted,
                "TEST_CODE_ingress-run",
            ))
            .expect("second append");

        let mut session = writer.locked_session().expect("locked session");
        let snapshot = session
            .validated_records()
            .expect("validated record snapshot");

        assert_eq!(
            snapshot.validation,
            AuditValidationReceipt {
                record_count: 2,
                tail_hash: Some(second.record_hash.clone()),
            }
        );
        assert_eq!(snapshot.records.len(), 2);
        assert_eq!(snapshot.records[0].record_hash, first.record_hash);
        assert_eq!(snapshot.records[1].record_hash, second.record_hash);
        assert_eq!(
            snapshot.records[1].previous_hash.as_deref(),
            Some(snapshot.records[0].record_hash.as_str())
        );
        assert_eq!(
            session.finish().expect("release session"),
            snapshot.validation
        );
    }

    #[test]
    fn locked_exact_lookup_recovers_after_restart_and_reports_content_conflict() {
        let root = TempAuditRoot::new("locked-exact-lookup-restart");
        let writer = test_writer(&root);
        let mut persisted = record(
            SelectionAuditPhase::V2IngressPrepared,
            "TEST_CODE_ingress-run",
        );
        persisted.content_hash = "content-a".to_owned();
        let appended = writer.append(persisted).expect("persist audit evidence");

        let restarted_writer = test_writer(&root);
        let mut restarted_session = restarted_writer
            .locked_session()
            .expect("restart acquires and validates chain");
        let exact = restarted_session
            .lookup_exact(
                SelectionAuditPhase::V2IngressPrepared,
                "TEST_CODE_ingress-run",
                "content-a",
            )
            .expect("exact lookup");
        let AuditExactLookup::Exact(exact_record) = exact else {
            panic!("restart must find the exact persisted record");
        };
        assert_eq!(exact_record.record_hash, appended.record_hash);
        assert_eq!(exact_record.content_hash, "content-a");

        let conflict = restarted_session
            .lookup_exact(
                SelectionAuditPhase::V2IngressPrepared,
                "TEST_CODE_ingress-run",
                "content-b",
            )
            .expect("conflict is a typed lookup result");
        let AuditExactLookup::ContentConflict { existing_record } = conflict else {
            panic!("same phase and subject with different content must conflict");
        };
        assert_eq!(existing_record.content_hash, "content-a");
        assert_eq!(existing_record.record_hash, appended.record_hash);
        assert_eq!(
            restarted_session
                .finish()
                .expect("restart chain remains valid"),
            AuditValidationReceipt {
                record_count: 1,
                tail_hash: Some(appended.record_hash),
            }
        );
    }

    #[test]
    fn locked_exact_lookup_rejects_duplicate_identity_and_preserves_hash_chain() {
        let root = TempAuditRoot::new("locked-exact-lookup-duplicate");
        let writer = test_writer(&root);
        let mut first = record(
            SelectionAuditPhase::V2GenerationCommitted,
            "TEST_CODE_generation-run",
        );
        first.content_hash = "same-content".to_owned();
        writer.append(first.clone()).expect("first record");
        let second = writer.append(first).expect("second record");

        let mut session = writer.locked_session().expect("valid hash chain");
        let duplicate = session
            .lookup_exact(
                SelectionAuditPhase::V2GenerationCommitted,
                "TEST_CODE_generation-run",
                "same-content",
            )
            .expect_err("duplicate recovery identity must fail closed");
        assert_eq!(duplicate.code(), "audit_chain_invalid");
        drop(session);

        assert_eq!(
            writer
                .validate()
                .expect("cryptographic chain remains valid"),
            AuditValidationReceipt {
                record_count: 2,
                tail_hash: Some(second.record_hash),
            }
        );
    }

    #[test]
    fn locked_test_session_never_creates_a_production_namespace() {
        let root = TempAuditRoot::new("locked-session-namespace-isolation");
        let test = test_writer(&root);

        let mut test_session = test.locked_session().expect("test session");
        test_session
            .append(record(SelectionAuditPhase::Prepared, "TEST_CODE_600396"))
            .expect("test append");
        test_session.finish().expect("finish test");

        assert_eq!(test.validate().expect("test chain").record_count, 1);
        assert_eq!(test.path(), root.0.join("test/selection-audit.jsonl"));
        assert!(!root.0.join("production").exists());
    }

    #[test]
    fn outcome_claim_audit_phases_are_permanently_parseable() {
        for (phase, token) in [
            (
                SelectionAuditPhase::V2OutcomeClaimPrepared,
                "v2_outcome_claim_prepared",
            ),
            (
                SelectionAuditPhase::V2OutcomeClaimCommitted,
                "v2_outcome_claim_committed",
            ),
        ] {
            assert_eq!(
                serde_json::to_string(&phase).unwrap(),
                format!("\"{token}\"")
            );
            assert_eq!(
                serde_json::from_str::<SelectionAuditPhase>(&format!("\"{token}\"")).unwrap(),
                phase
            );
        }
    }
}
