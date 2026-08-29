//! BR-180/BR-185/BR-186 whole-database generation-1 identity owner.
//!
//! The production entry point accepts no root, database path, lock path, mode,
//! connection, or migration authority. It acquires a process/OS shared
//! maintenance lease, pins the fixed database without following symlinks, and
//! reads the two documented global identity header fields from that retained
//! descriptor. Unknown pre-existing WAL/SHM/journal objects fail closed. The
//! exclusive selection inspection path may materialize, pin and later remove
//! only its own exact WAL/SHM pair under BR-189; it never writes either global
//! identity field or substitutes an unattested path.

use fs2::FileExt;
use rusqlite::{Connection, OpenFlags, Transaction, TransactionBehavior};
use std::ffi::{CString, OsStr, OsString};
use std::fmt;
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use thiserror::Error;

use super::global_schema_catalog_v1::{
    build_same_runtime_catalog_references, capture_catalog_snapshot, classify_database_half,
    CatalogSnapshot, DatabaseHalfDiagnostic, GlobalSchemaCatalogError, GlobalSchemaCatalogMode,
};
use super::selection_v2_repository::{
    verify_database_and_audit_in_rusqlite_snapshot, SelectionV2RepositoryError,
};
use super::sqlite_open_route_from_retained_parent;
use crate::selection::audit::{
    AuditValidationReceipt, LockedSelectionAuditSession, SelectionAuditError, SelectionAuditPhase,
    SelectionAuditWriter, ValidatedAuditChainSnapshot,
};

pub(crate) const STOCK_ANALYSIS_SQLITE_APPLICATION_ID: i64 = 1_398_035_265;
pub(crate) const STOCK_ANALYSIS_DB_SCHEMA_GENERATION: i64 = 1;

const PRODUCTION_DATABASE_RELATIVE_PATH: &str = "data/stock_analysis.db";
const PRODUCTION_LOCK_DIRECTORY_RELATIVE_PATH: &str = "data/locks";
const GLOBAL_MAINTENANCE_LOCK_FILE: &str = "global-schema-maintenance.lock";
const O_RDONLY_FLAG: i32 = 0;
const O_WRONLY_FLAG: i32 = 1;
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
const O_CLOEXEC_FLAG: i32 = 0x0008_0000;
#[cfg(any(target_os = "macos", target_os = "ios"))]
const O_CLOEXEC_FLAG: i32 = 0x0100_0000;
#[cfg(target_os = "freebsd")]
const O_CLOEXEC_FLAG: i32 = 0x0010_0000;
#[cfg(target_os = "openbsd")]
const O_CLOEXEC_FLAG: i32 = 0x0001_0000;
#[cfg(target_os = "netbsd")]
const O_CLOEXEC_FLAG: i32 = 0x0040_0000;
#[cfg(target_os = "linux")]
const ELOOP_CODE: i32 = 40;
#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd"
))]
const ELOOP_CODE: i32 = 62;

unsafe extern "C" {
    fn openat(directory_fd: i32, path: *const std::ffi::c_char, flags: i32, ...) -> i32;
    fn mkdirat(directory_fd: i32, path: *const std::ffi::c_char, mode: u32) -> i32;
    fn renameat(
        old_directory_fd: i32,
        old_path: *const std::ffi::c_char,
        new_directory_fd: i32,
        new_path: *const std::ffi::c_char,
    ) -> i32;
    fn unlinkat(directory_fd: i32, path: *const std::ffi::c_char, flags: i32) -> i32;
}

#[cfg(test)]
unsafe extern "C" {
    fn fcntl(descriptor: i32, command: i32, ...) -> i32;
    fn mkfifo(path: *const std::ffi::c_char, mode: u32) -> i32;
}

static PROCESS_SHARED_LEASES: AtomicUsize = AtomicUsize::new(0);
static PROCESS_EXCLUSIVE_LEASE: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GlobalSchemaIdentity {
    application_id: i64,
    user_version: i64,
}

// Operational consumers arrive with the separate bootstrap-integration slice.
#[allow(dead_code)]
impl GlobalSchemaIdentity {
    pub(crate) fn application_id(self) -> i64 {
        self.application_id
    }

    pub(crate) fn user_version(self) -> i64 {
        self.user_version
    }
}

#[derive(Debug, Error)]
pub(crate) enum GlobalSchemaV1Error {
    #[error("fixed global schema path is unsafe: {detail}")]
    UnsafeFixedPath { detail: String },

    #[error("global schema path is not mode-bound: {detail}")]
    ModeBindingViolation { detail: String },

    #[error("global schema I/O failed during {operation} at {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("global maintenance lease unavailable at {path}; retryable={retryable}: {source}")]
    MaintenanceLeaseUnavailable {
        path: PathBuf,
        retryable: bool,
        #[source]
        source: io::Error,
    },

    #[error("global process maintenance lease unavailable; retryable=true")]
    ProcessMaintenanceLeaseUnavailable,

    #[error(
        "global shared maintenance lease cannot be upgraded to exclusive authority in-process"
    )]
    SharedToExclusiveUpgradeForbidden,

    #[error("global exclusive process maintenance lease unavailable; retryable=true")]
    ExclusiveProcessMaintenanceLeaseUnavailable,

    #[error(
        "global exclusive maintenance lease unavailable at {path}; retryable={retryable}: {source}"
    )]
    ExclusiveMaintenanceLeaseUnavailable {
        path: PathBuf,
        retryable: bool,
        #[source]
        source: io::Error,
    },

    #[error("global schema database is not a regular file: {path}")]
    DatabaseNotRegular { path: PathBuf },

    #[error("global schema database is unavailable at {path}: {source}")]
    DatabaseUnavailable {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("global schema object identity changed while pinned: {path}")]
    ObjectIdentityChanged { path: PathBuf },

    #[error("invalid pinned SQLite header at {path}: {detail}")]
    InvalidSqliteHeader { path: PathBuf, detail: String },

    #[error(
        "WAL-backed global identity inspection is unavailable until descriptor-bound WAL/SHM snapshot validation exists: wal={wal},shm={shm}"
    )]
    WalBackedInspectionUnavailable { wal: PathBuf, shm: PathBuf },

    #[error("SQLite sidecar set is incomplete: wal_exists={wal_exists},shm_exists={shm_exists}")]
    IncompleteSidecarSet { wal_exists: bool, shm_exists: bool },

    #[error(
        "unmanaged global schema application_id={application_id},user_version={user_version}; offline migration required"
    )]
    OfflineGlobalMigrationRequired {
        application_id: i64,
        user_version: i64,
    },

    #[error(
        "unsupported future global schema generation {actual}; this binary supports generation {supported}"
    )]
    UnsupportedFutureGeneration { actual: i64, supported: i64 },

    #[error(
        "unsupported global schema identity application_id={application_id},user_version={user_version}; expected application_id=1398035265,user_version=1"
    )]
    UnsupportedIdentity {
        application_id: i64,
        user_version: i64,
    },

    #[error("global selection catalog inspection failed: {source}")]
    SelectionCatalog {
        #[source]
        source: GlobalSchemaCatalogError,
    },

    #[error("global selection audit inspection failed: {source}")]
    SelectionAudit {
        #[source]
        source: SelectionAuditError,
    },

    #[error("global selection SQLite inspection failed during {operation}: {source}")]
    SelectionSqlite {
        operation: &'static str,
        #[source]
        source: rusqlite::Error,
    },

    #[error("global selection snapshot changed while all owner locks were retained: {detail}")]
    SelectionSnapshotChanged { detail: String },

    #[error("global selection receipt/database reconciliation failed: {source}")]
    SelectionReceiptReconciliation {
        #[source]
        source: SelectionV2RepositoryError,
    },

    #[error("global selection database/audit halves are contradictory: {detail}")]
    SelectionAuthorityContradiction { detail: String },
}

#[cfg(test)]
impl GlobalSchemaV1Error {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::UnsafeFixedPath { .. } => "global_schema_unsafe_fixed_path",
            Self::ModeBindingViolation { .. } => "global_schema_mode_binding_violation",
            Self::Io { .. } => "global_schema_io",
            Self::MaintenanceLeaseUnavailable {
                retryable: true, ..
            } => "global_schema_lease_busy",
            Self::MaintenanceLeaseUnavailable {
                retryable: false, ..
            } => "global_schema_lease_unavailable",
            Self::ProcessMaintenanceLeaseUnavailable => "global_schema_process_lease_busy",
            Self::SharedToExclusiveUpgradeForbidden => {
                "global_schema_shared_to_exclusive_upgrade_forbidden"
            }
            Self::ExclusiveProcessMaintenanceLeaseUnavailable => {
                "global_schema_exclusive_process_lease_busy"
            }
            Self::ExclusiveMaintenanceLeaseUnavailable {
                retryable: true, ..
            } => "global_schema_exclusive_lease_busy",
            Self::ExclusiveMaintenanceLeaseUnavailable {
                retryable: false, ..
            } => "global_schema_exclusive_lease_unavailable",
            Self::DatabaseNotRegular { .. } => "global_schema_database_not_regular",
            Self::DatabaseUnavailable { .. } => "global_schema_database_unavailable",
            Self::ObjectIdentityChanged { .. } => "global_schema_object_identity_changed",
            Self::InvalidSqliteHeader { .. } => "global_schema_invalid_sqlite_header",
            Self::WalBackedInspectionUnavailable { .. } => {
                "global_schema_wal_inspection_unavailable"
            }
            Self::IncompleteSidecarSet { .. } => "global_schema_incomplete_sidecar_set",
            Self::OfflineGlobalMigrationRequired { .. } => {
                "global_schema_offline_migration_required"
            }
            Self::UnsupportedFutureGeneration { .. } => {
                "global_schema_unsupported_future_generation"
            }
            Self::UnsupportedIdentity { .. } => "global_schema_unsupported_identity",
            Self::SelectionCatalog { .. } => "global_schema_selection_catalog",
            Self::SelectionAudit { .. } => "global_schema_selection_audit",
            Self::SelectionSqlite { .. } => "global_schema_selection_sqlite",
            Self::SelectionSnapshotChanged { .. } => "global_schema_selection_snapshot_changed",
            Self::SelectionReceiptReconciliation { .. } => {
                "global_schema_selection_receipt_reconciliation"
            }
            Self::SelectionAuthorityContradiction { .. } => {
                "global_schema_selection_authority_contradiction"
            }
        }
    }
}

/// Sole ordinary-startup owner for the fixed global schema identity.
///
/// It is intentionally impossible to construct outside this module. The
/// associated production operation accepts no caller-selected identity.
//
// Operational bootstrap wiring is a separate gated slice. Keep this owner
// compiled now without pretending that ordinary startup already invokes it.
#[allow(dead_code)]
#[derive(Debug)]
pub(crate) struct GlobalSchemaVersionOwner {
    _private: (),
}

/// Non-forgeable permission to capture the selection catalog from the
/// database connection retained by the global owner.
///
/// The constructor is private to this module. The catalog module may require
/// this value, but production callers cannot manufacture it or obtain a raw
/// catalog snapshot without going through `GlobalSchemaVersionOwner`.
pub(super) struct SelectionCatalogCaptureAuthority {
    _private: (),
}

impl SelectionCatalogCaptureAuthority {
    fn new() -> Self {
        Self { _private: () }
    }

    #[cfg(test)]
    pub(super) fn for_test_code() -> Self {
        Self::new()
    }
}

fn new_global_schema_version_owner() -> GlobalSchemaVersionOwner {
    GlobalSchemaVersionOwner { _private: () }
}

pub(super) fn run_selection_v2_migration_command<I, S>(args: I) -> Result<String, String>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let mut test_rehearsal = false;
    let mut apply = false;
    let mut help = false;
    for raw in args {
        let raw = raw.into();
        let argument = raw
            .to_str()
            .ok_or_else(|| "migration argument is not valid UTF-8".to_owned())?;
        match argument {
            "--test" if !test_rehearsal => test_rehearsal = true,
            "--apply" if !apply => apply = true,
            "--help" | "-h" if !help => help = true,
            "--test" | "--apply" | "--help" | "-h" => {
                return Err(format!("duplicate migration argument: {argument}"));
            }
            _ => return Err(format!("unsupported migration argument: {argument}")),
        }
    }
    if help {
        if test_rehearsal || apply {
            return Err("--help cannot be combined with migration actions".to_owned());
        }
        return Ok(selection_v2_migration_help().to_owned());
    }
    if apply && !test_rehearsal {
        return Err(super::selection_v2::SELECTION_V2_APPLY_BLOCKER.to_owned());
    }

    let owner = new_global_schema_version_owner();
    if test_rehearsal {
        let (outcome, rehearsal) = owner
            .inspect_selection_test_code_rehearsal()
            .map_err(|error| error.to_string())?;
        let rendered = render_selection_v2_migration_diagnostic(&outcome, true, apply);
        drop(outcome);
        rehearsal.finish().map_err(|error| error.to_string())?;
        return Ok(rendered);
    }
    let outcome = owner
        .inspect_selection_with_audit()
        .map_err(|error| error.to_string())?;
    Ok(render_selection_v2_migration_diagnostic(
        &outcome, false, apply,
    ))
}

fn selection_v2_migration_help() -> &'static str {
    "Usage: migrate_selection_v2 [--test] [--apply]\n\
\n\
Default: owner-locked diagnostic against the fixed production database/audit.\n\
--test: owner-issued invocation-isolated TEST_CODE temporary-copy rehearsal;\n\
        the copy is removed after inspection and never authorizes production.\n\
--apply: production always fails closed. With --test it records only a\n\
         no-mutation rehearsal request because BR-180 apply remains disabled.\n\
Arbitrary database, audit, root, lock, or output paths are not accepted."
}

fn render_selection_v2_migration_diagnostic(
    outcome: &SelectionSchemaInspectionOutcome,
    test_rehearsal: bool,
    apply_requested: bool,
) -> String {
    let state = match outcome.authority_state() {
        SelectionSchemaAuthorityDiagnostic::DatabaseHalfOnly => "database_half_only",
        SelectionSchemaAuthorityDiagnostic::Absent => "absent",
        SelectionSchemaAuthorityDiagnostic::PreAmendment => "pre_amendment",
        SelectionSchemaAuthorityDiagnostic::TransitionalIncomplete => "transitional_incomplete",
        SelectionSchemaAuthorityDiagnostic::AmendedReceiptVerificationPending => {
            "amended_receipt_verification_pending"
        }
        SelectionSchemaAuthorityDiagnostic::Amended => "amended",
    };
    let nonempty = outcome
        .selection_row_counts()
        .iter()
        .filter(|(_, count)| **count != 0)
        .map(|(table, count)| format!("{table}:{count}"))
        .collect::<Vec<_>>()
        .join(",");
    let authoritative = matches!(outcome, SelectionSchemaInspectionOutcome::Amended(_));
    format!(
        "mode={} authoritative={authoritative} schema_state={state} apply_requested={apply_requested} mutation_performed=false\n\
audit_records={} audit_tail_hash={}\n\
selection_table_count={} nonempty_selection_counts={}\n",
        if test_rehearsal {
            "TEST_CODE_temp_copy_rehearsal"
        } else {
            "production_diagnostic"
        },
        outcome.audit_high_water().record_count,
        outcome
            .audit_high_water()
            .tail_hash
            .as_deref()
            .unwrap_or("none"),
        outcome.selection_row_counts().len(),
        if nonempty.is_empty() {
            "none"
        } else {
            nonempty.as_str()
        }
    )
}

struct TestCodeSelectionRehearsal {
    parent_path: PathBuf,
    parent_file: File,
    parent_identity: DirectoryIdentity,
    root_leaf: OsString,
    root: PinnedRoot,
    cleanup_complete: bool,
}

impl TestCodeSelectionRehearsal {
    fn create() -> Result<Self, GlobalSchemaV1Error> {
        let parent_path =
            fs::canonicalize(std::env::temp_dir()).map_err(|source| GlobalSchemaV1Error::Io {
                operation: "canonicalize TEST_CODE rehearsal parent",
                path: std::env::temp_dir(),
                source,
            })?;
        let parent_file =
            open_absolute_directory_no_follow(&parent_path, "pin TEST_CODE rehearsal parent")?;
        let parent_identity =
            DirectoryIdentity::from_metadata(&parent_file.metadata().map_err(|source| {
                GlobalSchemaV1Error::Io {
                    operation: "fstat TEST_CODE rehearsal parent",
                    path: parent_path.clone(),
                    source,
                }
            })?);
        for _ in 0..32 {
            let nonce = unpredictable_owner_nonce()?;
            let root_leaf = OsString::from(format!(
                "TEST_CODE_selection-v2-rehearsal-{}-{nonce}",
                std::process::id()
            ));
            let root_path = parent_path.join(&root_leaf);
            match mkdirat_new_component(&parent_file, &root_leaf) {
                Ok(()) => {
                    sync_directory_descriptor(&parent_file, &parent_path)?;
                    let root_file =
                        openat_component(&parent_file, &root_leaf, O_RDONLY_FLAG, false).map_err(
                            |source| GlobalSchemaV1Error::Io {
                                operation: "open owner-created TEST_CODE rehearsal root",
                                path: root_path.clone(),
                                source,
                            },
                        )?;
                    let metadata =
                        root_file
                            .metadata()
                            .map_err(|source| GlobalSchemaV1Error::Io {
                                operation: "fstat owner-created TEST_CODE rehearsal root",
                                path: root_path.clone(),
                                source,
                            })?;
                    if !metadata.is_dir() {
                        return Err(GlobalSchemaV1Error::UnsafeFixedPath {
                            detail: format!(
                                "owner-created TEST_CODE rehearsal root is not a directory: {}",
                                root_path.display()
                            ),
                        });
                    }
                    let root = PinnedRoot {
                        path: root_path,
                        file: root_file,
                        identity: DirectoryIdentity::from_metadata(&metadata),
                    };
                    return Ok(Self {
                        parent_path,
                        parent_file,
                        parent_identity,
                        root_leaf,
                        root,
                        cleanup_complete: false,
                    });
                }
                Err(source) if source.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(source) => {
                    return Err(GlobalSchemaV1Error::Io {
                        operation: "create invocation-isolated TEST_CODE rehearsal root",
                        path: root_path,
                        source,
                    });
                }
            }
        }
        Err(GlobalSchemaV1Error::ModeBindingViolation {
            detail: "could not allocate a unique TEST_CODE rehearsal root after 32 attempts"
                .to_owned(),
        })
    }

    fn root(&self) -> &Path {
        &self.root.path
    }

    fn pinned_root(&self) -> Result<PinnedRoot, GlobalSchemaV1Error> {
        self.validate_unchanged()?;
        Ok(PinnedRoot {
            path: self.root.path.clone(),
            file: self
                .root
                .file
                .try_clone()
                .map_err(|source| GlobalSchemaV1Error::Io {
                    operation: "clone owner-pinned TEST_CODE rehearsal root",
                    path: self.root.path.clone(),
                    source,
                })?,
            identity: self.root.identity,
        })
    }

    fn database_path(&self) -> PathBuf {
        self.root().join("stock_analysis.db")
    }

    fn audit_directory(&self) -> PathBuf {
        self.root().join("test")
    }

    fn audit_path(&self) -> PathBuf {
        self.audit_directory().join("selection-audit.jsonl")
    }

    fn create_audit_directory(&self) -> Result<(), GlobalSchemaV1Error> {
        let path = self.audit_directory();
        mkdirat_new_component(&self.root.file, OsStr::new("test")).map_err(|source| {
            GlobalSchemaV1Error::Io {
                operation: "create TEST_CODE rehearsal audit directory descriptor-relative",
                path: path.clone(),
                source,
            }
        })?;
        sync_directory_descriptor(&self.root.file, self.root())?;
        Ok(())
    }

    fn audit_directory_descriptor(&self) -> Result<File, GlobalSchemaV1Error> {
        openat_component(&self.root.file, OsStr::new("test"), O_RDONLY_FLAG, false).map_err(
            |source| GlobalSchemaV1Error::Io {
                operation: "open TEST_CODE rehearsal audit directory descriptor-relative",
                path: self.audit_directory(),
                source,
            },
        )
    }

    fn validate_unchanged(&self) -> Result<(), GlobalSchemaV1Error> {
        let parent =
            DirectoryIdentity::from_metadata(&self.parent_file.metadata().map_err(|source| {
                GlobalSchemaV1Error::Io {
                    operation: "fstat retained TEST_CODE rehearsal parent",
                    path: self.parent_path.clone(),
                    source,
                }
            })?);
        if parent != self.parent_identity {
            return Err(GlobalSchemaV1Error::ObjectIdentityChanged {
                path: self.parent_path.clone(),
            });
        }
        let current = openat_component(&self.parent_file, &self.root_leaf, O_RDONLY_FLAG, false)
            .map_err(|source| GlobalSchemaV1Error::Io {
                operation: "reopen TEST_CODE rehearsal root descriptor-relative",
                path: self.root.path.clone(),
                source,
            })?;
        let current = DirectoryIdentity::from_metadata(&current.metadata().map_err(|source| {
            GlobalSchemaV1Error::Io {
                operation: "fstat reopened TEST_CODE rehearsal root",
                path: self.root.path.clone(),
                source,
            }
        })?);
        let retained =
            DirectoryIdentity::from_metadata(&self.root.file.metadata().map_err(|source| {
                GlobalSchemaV1Error::Io {
                    operation: "fstat retained TEST_CODE rehearsal root",
                    path: self.root.path.clone(),
                    source,
                }
            })?);
        if current != self.root.identity || retained != self.root.identity {
            return Err(GlobalSchemaV1Error::ObjectIdentityChanged {
                path: self.root.path.clone(),
            });
        }
        Ok(())
    }

    fn finish(mut self) -> Result<(), GlobalSchemaV1Error> {
        self.cleanup()?;
        self.cleanup_complete = true;
        Ok(())
    }

    fn cleanup(&mut self) -> Result<(), GlobalSchemaV1Error> {
        if self.cleanup_complete {
            return Ok(());
        }
        self.validate_unchanged()?;
        let cleanup_leaf = OsString::from(format!(
            ".TEST_CODE_selection-v2-cleanup-{}",
            unpredictable_owner_nonce()?
        ));
        renameat_component(&self.parent_file, &self.root_leaf, &cleanup_leaf).map_err(
            |source| GlobalSchemaV1Error::Io {
                operation: "rename TEST_CODE rehearsal root for explicit cleanup",
                path: self.root.path.clone(),
                source,
            },
        )?;
        let cleanup_path = self.parent_path.join(&cleanup_leaf);
        let renamed = openat_component(&self.parent_file, &cleanup_leaf, O_RDONLY_FLAG, false)
            .map_err(|source| GlobalSchemaV1Error::Io {
                operation: "open renamed TEST_CODE cleanup root",
                path: cleanup_path.clone(),
                source,
            })?;
        let renamed_identity =
            DirectoryIdentity::from_metadata(&renamed.metadata().map_err(|source| {
                GlobalSchemaV1Error::Io {
                    operation: "fstat renamed TEST_CODE cleanup root",
                    path: cleanup_path.clone(),
                    source,
                }
            })?);
        if renamed_identity != self.root.identity {
            return Err(GlobalSchemaV1Error::ObjectIdentityChanged { path: cleanup_path });
        }
        fs::remove_dir_all(&cleanup_path).map_err(|source| GlobalSchemaV1Error::Io {
            operation: "remove explicitly finalized TEST_CODE rehearsal root",
            path: cleanup_path,
            source,
        })?;
        sync_directory_descriptor(&self.parent_file, &self.parent_path)?;
        self.cleanup_complete = true;
        Ok(())
    }
}

impl Drop for TestCodeSelectionRehearsal {
    fn drop(&mut self) {
        if !self.cleanup_complete {
            if let Err(error) = self.cleanup() {
                log::error!(
                    "[BR-180] TEST_CODE rehearsal cleanup failed during Drop fallback: {error}"
                );
            }
        }
    }
}

#[allow(dead_code)]
impl GlobalSchemaVersionOwner {
    fn new() -> Self {
        new_global_schema_version_owner()
    }

    #[cfg(test)]
    fn for_test_code() -> Self {
        Self::new()
    }

    pub(crate) fn inspect_fixed_production(
        &self,
    ) -> Result<VerifiedGlobalSchemaV1, GlobalSchemaV1Error> {
        inspect_bound_database(ModeBoundPaths::production())
    }

    /// Acquire exclusive offline authority over the fixed production
    /// namespace. This pins only the root, database parent, lock parent and
    /// maintenance lock; it does not open, initialize, or inspect the database.
    pub(crate) fn acquire_exclusive_fixed_production(
        &self,
    ) -> Result<ExclusiveGlobalSchemaMaintenanceLease, GlobalSchemaV1Error> {
        acquire_exclusive_bound(ModeBoundPaths::production())
    }

    fn selection_catalog_capture_authority(&self) -> SelectionCatalogCaptureAuthority {
        SelectionCatalogCaptureAuthority::new()
    }

    /// Return either a detached non-amended diagnostic or an opaque amended
    /// capability that retains the exclusive owner authority and pinned
    /// database/audit objects.
    pub(crate) fn inspect_selection_with_audit(
        &self,
    ) -> Result<SelectionSchemaInspectionOutcome, GlobalSchemaV1Error> {
        let audit_writer = SelectionAuditWriter::production()
            .map_err(|source| GlobalSchemaV1Error::SelectionAudit { source })?;
        self.inspect_selection_with_bound_paths(
            ModeBoundPaths::production(),
            &audit_writer,
            GlobalSchemaCatalogMode::Production,
        )
    }

    fn inspect_selection_test_code_rehearsal(
        &self,
    ) -> Result<(SelectionSchemaInspectionOutcome, TestCodeSelectionRehearsal), GlobalSchemaV1Error>
    {
        let rehearsal = TestCodeSelectionRehearsal::create()?;
        let outcome = (|| {
            self.copy_fixed_production_selection_snapshot(&rehearsal)?;
            let audit_writer = SelectionAuditWriter::for_test_code_pinned_root(
                &rehearsal.root.file,
                rehearsal.root(),
            )
            .map_err(|source| GlobalSchemaV1Error::SelectionAudit { source })?;
            self.inspect_selection_with_pinned_root(
                ModeBoundPaths::isolated_test(rehearsal.root())?,
                rehearsal.pinned_root()?,
                &audit_writer,
                GlobalSchemaCatalogMode::Production,
            )
        })();
        match outcome {
            Ok(outcome) => Ok((outcome, rehearsal)),
            Err(error) => match rehearsal.finish() {
                Ok(()) => Err(error),
                Err(cleanup) => Err(GlobalSchemaV1Error::ModeBindingViolation {
                    detail: format!(
                        "TEST_CODE rehearsal failed ({error}); explicit cleanup also failed ({cleanup})"
                    ),
                }),
            },
        }
    }

    fn copy_fixed_production_selection_snapshot(
        &self,
        rehearsal: &TestCodeSelectionRehearsal,
    ) -> Result<(), GlobalSchemaV1Error> {
        let production_paths = ModeBoundPaths::production();
        let maintenance = acquire_exclusive_bound(production_paths)?;
        let audit_writer = SelectionAuditWriter::production()
            .map_err(|source| GlobalSchemaV1Error::SelectionAudit { source })?;
        let database_path = maintenance
            .namespace
            .database_parent
            .path
            .join(&maintenance.namespace.database_leaf);
        let (database_file, database_identity) = open_pinned_regular_read_write(
            &maintenance.namespace.database_parent,
            &maintenance.namespace.database_leaf,
            &database_path,
        )?;
        require_no_live_sidecars_for_bound_namespace(&maintenance.namespace, &database_path)?;
        let mut connection = open_pinned_sqlite_read_write(
            &maintenance.namespace.database_parent,
            &maintenance.namespace.database_leaf,
            &database_file,
            database_identity,
            &database_path,
        )?;
        let inspection_sidecars = OwnerCreatedSqliteSidecars::materialize_and_pin(
            &connection,
            &maintenance.namespace,
            &database_path,
        )?;
        let copy_result = (|| {
            let mut audit_session = audit_writer
                .locked_session()
                .map_err(|source| GlobalSchemaV1Error::SelectionAudit { source })?;
            let initial_audit = audit_session
                .validated_records()
                .map_err(|source| GlobalSchemaV1Error::SelectionAudit { source })?;
            let (audit_parent, audit_leaf) = PinnedDirectory::for_parent(
                &maintenance.namespace.root,
                audit_writer.path(),
                "selection audit",
            )?;
            let audit_file =
                pin_optional_selection_audit(&audit_parent, &audit_leaf, audit_writer.path())?;
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|source| GlobalSchemaV1Error::SelectionSqlite {
                    operation: "BEGIN IMMEDIATE for TEST_CODE rehearsal copy",
                    source,
                })?;
            capture_selection_integrity(&transaction)?;

            copy_pinned_file_to_new_descriptor(
                &database_file,
                &database_path,
                &rehearsal.root.file,
                OsStr::new("stock_analysis.db"),
                &rehearsal.database_path(),
                "TEST_CODE rehearsal database",
            )?;
            if let PinnedSelectionAuditFile::Present { file, .. } = &audit_file {
                rehearsal.create_audit_directory()?;
                let audit_directory = rehearsal.audit_directory_descriptor()?;
                copy_pinned_file_to_new_descriptor(
                    file,
                    audit_writer.path(),
                    &audit_directory,
                    OsStr::new("selection-audit.jsonl"),
                    &rehearsal.audit_path(),
                    "TEST_CODE rehearsal selection audit",
                )?;
            }

            require_same_file_identity(
                &maintenance.namespace.database_parent,
                &maintenance.namespace.database_leaf,
                &database_path,
                &database_file,
                database_identity,
                "revalidate rehearsal source database",
            )?;
            revalidate_selection_audit_file(
                &audit_parent,
                &audit_leaf,
                audit_writer.path(),
                &audit_file,
            )?;
            maintenance.namespace.validate_unchanged()?;
            inspection_sidecars.validate_present_exact(&maintenance.namespace, &database_path)?;
            let final_audit = audit_session
                .validated_records()
                .map_err(|source| GlobalSchemaV1Error::SelectionAudit { source })?;
            if final_audit != initial_audit {
                return Err(GlobalSchemaV1Error::SelectionSnapshotChanged {
                    detail: "selection audit changed during TEST_CODE rehearsal copy".to_owned(),
                });
            }

            transaction
                .commit()
                .map_err(|source| GlobalSchemaV1Error::SelectionSqlite {
                    operation: "finish TEST_CODE rehearsal source transaction",
                    source,
                })?;
            let expected_audit = initial_audit.validation().clone();
            let finished_audit = audit_session
                .finish()
                .map_err(|source| GlobalSchemaV1Error::SelectionAudit { source })?;
            if finished_audit != expected_audit {
                return Err(GlobalSchemaV1Error::SelectionSnapshotChanged {
                    detail: "audit finish high-water changed during TEST_CODE rehearsal copy"
                        .to_owned(),
                });
            }
            Ok(())
        })();

        drop(connection);
        let cleanup = inspection_sidecars
            .cleanup_after_connection_close(&maintenance.namespace, &database_path);
        match (copy_result, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(primary), Ok(())) => Err(primary),
            (Ok(()), Err(cleanup)) => Err(cleanup),
            (Err(primary), Err(cleanup)) => Err(GlobalSchemaV1Error::SelectionSnapshotChanged {
                detail: format!(
                    "TEST_CODE rehearsal copy failed ({primary}); exact owner-sidecar cleanup also failed ({cleanup})"
                ),
            }),
        }
    }

    #[cfg(test)]
    fn inspect_selection_with_audit_for_test(
        &self,
        namespace_root: &Path,
        audit_writer: &SelectionAuditWriter,
    ) -> Result<SelectionSchemaInspectionOutcome, GlobalSchemaV1Error> {
        self.inspect_selection_with_bound_paths(
            ModeBoundPaths::isolated_test(namespace_root)?,
            audit_writer,
            GlobalSchemaCatalogMode::Test,
        )
    }

    fn inspect_selection_with_bound_paths(
        &self,
        paths: ModeBoundPaths,
        audit_writer: &SelectionAuditWriter,
        catalog_mode: GlobalSchemaCatalogMode,
    ) -> Result<SelectionSchemaInspectionOutcome, GlobalSchemaV1Error> {
        self.inspect_selection_with_optional_pinned_root(paths, None, audit_writer, catalog_mode)
    }

    fn inspect_selection_with_pinned_root(
        &self,
        paths: ModeBoundPaths,
        root: PinnedRoot,
        audit_writer: &SelectionAuditWriter,
        catalog_mode: GlobalSchemaCatalogMode,
    ) -> Result<SelectionSchemaInspectionOutcome, GlobalSchemaV1Error> {
        self.inspect_selection_with_optional_pinned_root(
            paths,
            Some(root),
            audit_writer,
            catalog_mode,
        )
    }

    fn inspect_selection_with_optional_pinned_root(
        &self,
        paths: ModeBoundPaths,
        root: Option<PinnedRoot>,
        audit_writer: &SelectionAuditWriter,
        catalog_mode: GlobalSchemaCatalogMode,
    ) -> Result<SelectionSchemaInspectionOutcome, GlobalSchemaV1Error> {
        let maintenance = match root {
            Some(root) => acquire_exclusive_with_pinned_root(paths, root)?,
            None => acquire_exclusive_bound(paths)?,
        };
        let database_path = maintenance
            .namespace
            .database_parent
            .path
            .join(&maintenance.namespace.database_leaf);
        let (database_file, database_identity) = open_pinned_regular_read_write(
            &maintenance.namespace.database_parent,
            &maintenance.namespace.database_leaf,
            &database_path,
        )?;
        require_no_live_sidecars_for_bound_namespace(&maintenance.namespace, &database_path)?;
        let mut connection = open_pinned_sqlite_read_write(
            &maintenance.namespace.database_parent,
            &maintenance.namespace.database_leaf,
            &database_file,
            database_identity,
            &database_path,
        )?;
        let inspection_sidecars = OwnerCreatedSqliteSidecars::materialize_and_pin(
            &connection,
            &maintenance.namespace,
            &database_path,
        )?;

        let inspection_result = (|| {
            let mut audit_session = audit_writer
                .locked_session()
                .map_err(|source| GlobalSchemaV1Error::SelectionAudit { source })?;
            let initial_audit = audit_session
                .validated_records()
                .map_err(|source| GlobalSchemaV1Error::SelectionAudit { source })?;
            let (audit_parent, audit_leaf) = PinnedDirectory::for_parent(
                &maintenance.namespace.root,
                audit_writer.path(),
                "selection audit",
            )?;
            let audit_file =
                pin_optional_selection_audit(&audit_parent, &audit_leaf, audit_writer.path())?;
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|source| GlobalSchemaV1Error::SelectionSqlite {
                    operation: "BEGIN IMMEDIATE",
                    source,
                })?;
            let authority = self.selection_catalog_capture_authority();
            // Run connection-initializing probes before freezing the catalog
            // baseline. On SQLite/macOS the integrity probes can materialize
            // the connection-local `temp` schema even though no application
            // data or persistent schema changed.
            let initial_pragmas = capture_selection_pragmas(&transaction)?;
            let initial_integrity = capture_selection_integrity(&transaction)?;
            let initial_catalog = capture_catalog_snapshot(&authority, &transaction, catalog_mode)
                .map_err(|source| GlobalSchemaV1Error::SelectionCatalog { source })?;

            let prepared = VerifiedSelectionSchemaSnapshot {
                transaction,
                audit_session,
                inspection_sidecars: &inspection_sidecars,
                database_file: &database_file,
                database_identity,
                database_path: database_path.clone(),
                audit_parent: &audit_parent,
                audit_leaf,
                audit_file: &audit_file,
                audit_path: audit_writer.path().to_path_buf(),
                initial_catalog,
                initial_audit,
                initial_pragmas,
                initial_integrity,
                authority,
                catalog_mode,
                maintenance: &maintenance,
            }
            .consume_authority()?;
            Ok((prepared, audit_parent, audit_file))
        })();

        drop(connection);
        let cleanup = inspection_sidecars
            .cleanup_after_connection_close(&maintenance.namespace, &database_path);
        match (inspection_result, cleanup) {
            (Ok((prepared, audit_parent, audit_file)), Ok(())) => Ok(prepared.issue(
                database_file,
                database_identity,
                audit_parent,
                audit_file,
                maintenance,
            )),
            (Err(primary), Ok(())) => Err(primary),
            (Ok(_), Err(cleanup)) => Err(cleanup),
            (Err(primary), Err(cleanup)) => Err(GlobalSchemaV1Error::SelectionSnapshotChanged {
                detail: format!(
                    "selection inspection failed ({primary}); exact owner-sidecar cleanup also failed ({cleanup})"
                ),
            }),
        }
    }
}

#[derive(Debug)]
pub(crate) enum SelectionSchemaInspectionOutcome {
    Diagnostic(Box<SelectionSchemaInspectionDiagnostic>),
    Amended(Box<VerifiedAmendedSelectionSchema>),
}

#[derive(Debug)]
pub(crate) struct SelectionSchemaInspectionDiagnostic {
    database_half: DatabaseHalfDiagnostic,
    authority_state: SelectionSchemaAuthorityDiagnostic,
    audit_high_water: AuditValidationReceipt,
    selection_row_counts: std::collections::BTreeMap<String, i64>,
}

/// Detached status issued after the owner has consumed all retained locks.
///
/// It is diagnostic-only. The receipt-pending state is an internal
/// classification consumed before return and can never substitute for the
/// retained amended capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SelectionSchemaAuthorityDiagnostic {
    DatabaseHalfOnly,
    Absent,
    PreAmendment,
    TransitionalIncomplete,
    AmendedReceiptVerificationPending,
    Amended,
}

#[allow(dead_code)]
impl SelectionSchemaInspectionOutcome {
    pub(crate) fn database_half(&self) -> &DatabaseHalfDiagnostic {
        match self {
            Self::Diagnostic(diagnostic) => &diagnostic.database_half,
            Self::Amended(capability) => &capability.database_half,
        }
    }

    pub(crate) fn audit_high_water(&self) -> &AuditValidationReceipt {
        match self {
            Self::Diagnostic(diagnostic) => &diagnostic.audit_high_water,
            Self::Amended(capability) => &capability.audit_high_water,
        }
    }

    pub(crate) fn authority_state(&self) -> SelectionSchemaAuthorityDiagnostic {
        match self {
            Self::Diagnostic(diagnostic) => diagnostic.authority_state,
            Self::Amended(_) => SelectionSchemaAuthorityDiagnostic::Amended,
        }
    }

    pub(crate) fn selection_row_counts(&self) -> &std::collections::BTreeMap<String, i64> {
        match self {
            Self::Diagnostic(diagnostic) => &diagnostic.selection_row_counts,
            Self::Amended(capability) => &capability.selection_row_counts,
        }
    }
}

/// Opaque proof that exact final database rows and the audit chain reconciled
/// inside one retained SQLite transaction.
///
/// The exclusive maintenance lease and pinned objects deliberately remain
/// owned by this non-`Clone` capability. A detached diagnostic can never
/// represent `Amended`.
#[must_use = "dropping the amended capability releases exclusive schema authority"]
pub(crate) struct VerifiedAmendedSelectionSchema {
    database_half: DatabaseHalfDiagnostic,
    audit_high_water: AuditValidationReceipt,
    selection_row_counts: std::collections::BTreeMap<String, i64>,
    _database_file: File,
    _database_identity: FileIdentity,
    _audit_parent: PinnedDirectory,
    _audit_file: PinnedSelectionAuditFile,
    _maintenance: ExclusiveGlobalSchemaMaintenanceLease,
}

impl fmt::Debug for VerifiedAmendedSelectionSchema {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VerifiedAmendedSelectionSchema")
            .field("audit_high_water", &self.audit_high_water)
            .field("selection_row_counts", &self.selection_row_counts)
            .finish_non_exhaustive()
    }
}

impl VerifiedAmendedSelectionSchema {
    /// Clone only already pinned descriptors plus the owner-fixed relative
    /// identity for the internal r2d2 hand-off. No path-bearing API exists at
    /// this boundary.
    pub(super) fn pinned_database_for_pool(
        &self,
    ) -> Result<super::PinnedSqliteDatabase, GlobalSchemaV1Error> {
        let retained =
            FileIdentity::from_metadata(&self._database_file.metadata().map_err(|source| {
                GlobalSchemaV1Error::Io {
                    operation: "fstat amended database descriptor for pool hand-off",
                    path: PathBuf::from("<owner-pinned-selection-database>"),
                    source,
                }
            })?);
        if retained.device != self._database_identity.device
            || retained.inode != self._database_identity.inode
        {
            return Err(GlobalSchemaV1Error::ObjectIdentityChanged {
                path: PathBuf::from("<owner-pinned-selection-database>"),
            });
        }
        let database_descriptor =
            self._database_file
                .try_clone()
                .map_err(|source| GlobalSchemaV1Error::Io {
                    operation: "clone amended database descriptor for pool hand-off",
                    path: PathBuf::from("<owner-pinned-selection-database>"),
                    source,
                })?;
        let root_descriptor =
            self._maintenance
                .namespace
                .root
                .file
                .try_clone()
                .map_err(|source| GlobalSchemaV1Error::Io {
                    operation: "clone amended owner root descriptor for pool hand-off",
                    path: PathBuf::from("<owner-pinned-selection-root>"),
                    source,
                })?;
        let parent_descriptor = self
            ._maintenance
            .namespace
            .database_parent
            .file
            .try_clone()
            .map_err(|source| GlobalSchemaV1Error::Io {
                operation: "clone amended database parent descriptor for pool hand-off",
                path: PathBuf::from("<owner-pinned-selection-database-parent>"),
                source,
            })?;
        let mut database_relative_identity = PathBuf::new();
        for component in &self
            ._maintenance
            .namespace
            .database_parent
            .relative_components
        {
            database_relative_identity.push(component);
        }
        database_relative_identity.push(&self._maintenance.namespace.database_leaf);
        super::PinnedSqliteDatabase::from_owner_descriptors(
            root_descriptor,
            parent_descriptor,
            self._maintenance.namespace.database_leaf.clone(),
            database_relative_identity,
            database_descriptor,
        )
        .map_err(|source| GlobalSchemaV1Error::Io {
            operation: "bind amended database descriptor to pool",
            path: PathBuf::from("<owner-pinned-selection-database>"),
            source,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
struct SelectionPragmaSnapshot {
    application_id: i64,
    user_version: i64,
    foreign_keys: i64,
    journal_mode: String,
    synchronous: i64,
}

#[derive(Debug, PartialEq, Eq)]
struct SelectionIntegritySnapshot {
    integrity_rows: Vec<String>,
    foreign_key_violations: i64,
}

enum PinnedSelectionAuditFile {
    Missing,
    Present { file: File, identity: FileIdentity },
}

struct PinnedOwnerSqliteSidecar {
    file: File,
    identity: FileIdentity,
    leaf: OsString,
    path: PathBuf,
}

struct OwnerCreatedSqliteSidecars {
    wal: PinnedOwnerSqliteSidecar,
    shm: PinnedOwnerSqliteSidecar,
}

impl OwnerCreatedSqliteSidecars {
    fn materialize_and_pin(
        connection: &Connection,
        namespace: &PinnedNamespace,
        database_path: &Path,
    ) -> Result<Self, GlobalSchemaV1Error> {
        let _: i64 = connection
            .query_row("SELECT COUNT(*) FROM sqlite_schema", [], |row| row.get(0))
            .map_err(|source| GlobalSchemaV1Error::SelectionSqlite {
                operation: "materialize owner SQLite WAL/SHM before audit snapshot",
                source,
            })?;
        let wal = pin_owner_created_sidecar(namespace, database_path, "-wal")?;
        let shm = pin_owner_created_sidecar(namespace, database_path, "-shm")?;
        require_sidecar_absent(namespace, database_path, "-journal")?;
        let sidecars = Self { wal, shm };
        sidecars.validate_present_exact(namespace, database_path)?;
        Ok(sidecars)
    }

    fn validate_present_exact(
        &self,
        namespace: &PinnedNamespace,
        database_path: &Path,
    ) -> Result<(), GlobalSchemaV1Error> {
        for sidecar in [&self.wal, &self.shm] {
            require_same_file_identity(
                &namespace.database_parent,
                &sidecar.leaf,
                &sidecar.path,
                &sidecar.file,
                sidecar.identity,
                "revalidate owner-created SQLite sidecar",
            )?;
        }
        require_sidecar_absent(namespace, database_path, "-journal")
    }

    fn cleanup_after_connection_close(
        self,
        namespace: &PinnedNamespace,
        database_path: &Path,
    ) -> Result<(), GlobalSchemaV1Error> {
        require_sidecar_absent(namespace, database_path, "-journal")?;
        for sidecar in [&self.wal, &self.shm] {
            match openat_component(
                &namespace.database_parent.file,
                &sidecar.leaf,
                O_RDONLY_FLAG,
                false,
            ) {
                Ok(reopened) => {
                    let reopened_metadata =
                        reopened
                            .metadata()
                            .map_err(|source| GlobalSchemaV1Error::Io {
                                operation: "fstat owner-created SQLite sidecar before cleanup",
                                path: sidecar.path.clone(),
                                source,
                            })?;
                    if !reopened_metadata.is_file()
                        || FileIdentity::from_metadata(&reopened_metadata) != sidecar.identity
                    {
                        return Err(GlobalSchemaV1Error::ObjectIdentityChanged {
                            path: sidecar.path.clone(),
                        });
                    }
                    unlinkat_component(&namespace.database_parent.file, &sidecar.leaf).map_err(
                        |source| GlobalSchemaV1Error::Io {
                            operation: "remove exact owner-created SQLite sidecar",
                            path: sidecar.path.clone(),
                            source,
                        },
                    )?;
                }
                Err(source) if source.kind() == io::ErrorKind::NotFound => {
                    let retained =
                        sidecar
                            .file
                            .metadata()
                            .map_err(|source| GlobalSchemaV1Error::Io {
                                operation:
                                    "fstat already-removed owner-created SQLite sidecar descriptor",
                                path: sidecar.path.clone(),
                                source,
                            })?;
                    if retained.nlink() != 0 {
                        return Err(GlobalSchemaV1Error::ObjectIdentityChanged {
                            path: sidecar.path.clone(),
                        });
                    }
                }
                Err(source) => {
                    return Err(GlobalSchemaV1Error::Io {
                        operation: "reopen owner-created SQLite sidecar for cleanup",
                        path: sidecar.path.clone(),
                        source,
                    });
                }
            }
        }
        sync_directory_descriptor(
            &namespace.database_parent.file,
            &namespace.database_parent.path,
        )?;
        require_no_live_sidecars_for_bound_namespace(namespace, database_path)?;
        namespace.validate_unchanged()
    }
}

fn pin_owner_created_sidecar(
    namespace: &PinnedNamespace,
    database_path: &Path,
    suffix: &str,
) -> Result<PinnedOwnerSqliteSidecar, GlobalSchemaV1Error> {
    let leaf = sidecar_leaf(&namespace.database_leaf, suffix);
    let path = sidecar_path(database_path, suffix);
    let (file, identity) = open_pinned_regular_read_only(&namespace.database_parent, &leaf, &path)?;
    let metadata = file.metadata().map_err(|source| GlobalSchemaV1Error::Io {
        operation: "fstat owner-created SQLite sidecar",
        path: path.clone(),
        source,
    })?;
    if metadata.nlink() != 1 {
        return Err(GlobalSchemaV1Error::ObjectIdentityChanged { path });
    }
    Ok(PinnedOwnerSqliteSidecar {
        file,
        identity,
        leaf,
        path,
    })
}

fn require_sidecar_absent(
    namespace: &PinnedNamespace,
    database_path: &Path,
    suffix: &str,
) -> Result<(), GlobalSchemaV1Error> {
    let leaf = sidecar_leaf(&namespace.database_leaf, suffix);
    let path = sidecar_path(database_path, suffix);
    match openat_component(&namespace.database_parent.file, &leaf, O_RDONLY_FLAG, false) {
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(GlobalSchemaV1Error::ObjectIdentityChanged { path }),
        Err(source) => Err(GlobalSchemaV1Error::Io {
            operation: "verify SQLite sidecar remains absent",
            path,
            source,
        }),
    }
}

/// Private, non-`Clone` evidence retained until owner revalidation completes.
///
/// Field order mirrors the release order. The explicit consumer finishes the
/// SQLite transaction, then the audit session, leaving the global maintenance
/// authority to drop last.
struct VerifiedSelectionSchemaSnapshot<'locks, 'sidecars> {
    transaction: Transaction<'locks>,
    audit_session: LockedSelectionAuditSession<'locks>,
    inspection_sidecars: &'sidecars OwnerCreatedSqliteSidecars,
    database_file: &'sidecars File,
    database_identity: FileIdentity,
    database_path: PathBuf,
    audit_parent: &'sidecars PinnedDirectory,
    audit_leaf: OsString,
    audit_file: &'sidecars PinnedSelectionAuditFile,
    audit_path: PathBuf,
    initial_catalog: CatalogSnapshot,
    initial_audit: ValidatedAuditChainSnapshot,
    initial_pragmas: SelectionPragmaSnapshot,
    initial_integrity: SelectionIntegritySnapshot,
    authority: SelectionCatalogCaptureAuthority,
    catalog_mode: GlobalSchemaCatalogMode,
    maintenance: &'sidecars ExclusiveGlobalSchemaMaintenanceLease,
}

struct PreparedSelectionSchemaInspection {
    database_half: DatabaseHalfDiagnostic,
    authority_state: SelectionSchemaAuthorityDiagnostic,
    audit_high_water: AuditValidationReceipt,
    selection_row_counts: std::collections::BTreeMap<String, i64>,
    exact_amended: bool,
}

impl PreparedSelectionSchemaInspection {
    fn issue(
        self,
        database_file: File,
        database_identity: FileIdentity,
        audit_parent: PinnedDirectory,
        audit_file: PinnedSelectionAuditFile,
        maintenance: ExclusiveGlobalSchemaMaintenanceLease,
    ) -> SelectionSchemaInspectionOutcome {
        if self.exact_amended {
            SelectionSchemaInspectionOutcome::Amended(Box::new(VerifiedAmendedSelectionSchema {
                database_half: self.database_half,
                audit_high_water: self.audit_high_water,
                selection_row_counts: self.selection_row_counts,
                _database_file: database_file,
                _database_identity: database_identity,
                _audit_parent: audit_parent,
                _audit_file: audit_file,
                _maintenance: maintenance,
            }))
        } else {
            drop(maintenance);
            SelectionSchemaInspectionOutcome::Diagnostic(Box::new(
                SelectionSchemaInspectionDiagnostic {
                    database_half: self.database_half,
                    authority_state: self.authority_state,
                    audit_high_water: self.audit_high_water,
                    selection_row_counts: self.selection_row_counts,
                },
            ))
        }
    }
}

impl VerifiedSelectionSchemaSnapshot<'_, '_> {
    fn consume_authority(
        mut self,
    ) -> Result<PreparedSelectionSchemaInspection, GlobalSchemaV1Error> {
        let references = build_same_runtime_catalog_references(self.catalog_mode)
            .map_err(|source| GlobalSchemaV1Error::SelectionCatalog { source })?;
        let database_half = classify_database_half(&self.initial_catalog, &references)
            .map_err(|source| GlobalSchemaV1Error::SelectionCatalog { source })?;
        let audit_present = matches!(self.audit_file, PinnedSelectionAuditFile::Present { .. });
        let authority_state =
            classify_selection_authority_state(&database_half, audit_present, &self.initial_audit)?;
        let selection_row_counts = self.initial_catalog.selection_row_counts().clone();
        let audit_high_water = self.initial_audit.validation().clone();
        let exact_amended = if authority_state
            == SelectionSchemaAuthorityDiagnostic::AmendedReceiptVerificationPending
        {
            let reconciled = verify_database_and_audit_in_rusqlite_snapshot(
                &self.transaction,
                &mut self.audit_session,
            )
            .map_err(|source| GlobalSchemaV1Error::SelectionReceiptReconciliation { source })?;
            if reconciled != self.initial_audit {
                return Err(GlobalSchemaV1Error::SelectionSnapshotChanged {
                    detail: "receipt reconciliation returned a different audit prefix/high-water"
                        .to_owned(),
                });
            }
            true
        } else {
            false
        };

        let final_catalog =
            capture_catalog_snapshot(&self.authority, &self.transaction, self.catalog_mode)
                .map_err(|source| GlobalSchemaV1Error::SelectionCatalog { source })?;
        if final_catalog != self.initial_catalog {
            return Err(GlobalSchemaV1Error::SelectionSnapshotChanged {
                detail: "catalog, dependency, payload, or row-count evidence changed".to_owned(),
            });
        }
        if capture_selection_pragmas(&self.transaction)? != self.initial_pragmas {
            return Err(GlobalSchemaV1Error::SelectionSnapshotChanged {
                detail: "SQLite PRAGMA evidence changed".to_owned(),
            });
        }
        if capture_selection_integrity(&self.transaction)? != self.initial_integrity {
            return Err(GlobalSchemaV1Error::SelectionSnapshotChanged {
                detail: "SQLite integrity evidence changed".to_owned(),
            });
        }
        let final_audit = self
            .audit_session
            .validated_records()
            .map_err(|source| GlobalSchemaV1Error::SelectionAudit { source })?;
        if final_audit != self.initial_audit {
            return Err(GlobalSchemaV1Error::SelectionSnapshotChanged {
                detail: "selection audit prefix or high-water changed".to_owned(),
            });
        }

        require_same_file_identity(
            &self.maintenance.namespace.database_parent,
            &self.maintenance.namespace.database_leaf,
            &self.database_path,
            self.database_file,
            self.database_identity,
            "revalidate selection database",
        )?;
        revalidate_selection_audit_file(
            self.audit_parent,
            &self.audit_leaf,
            &self.audit_path,
            self.audit_file,
        )?;
        self.maintenance.namespace.validate_unchanged()?;
        self.inspection_sidecars
            .validate_present_exact(&self.maintenance.namespace, &self.database_path)?;

        self.transaction
            .commit()
            .map_err(|source| GlobalSchemaV1Error::SelectionSqlite {
                operation: "finish read-only inspection transaction",
                source,
            })?;
        let finished_audit = self
            .audit_session
            .finish()
            .map_err(|source| GlobalSchemaV1Error::SelectionAudit { source })?;
        if finished_audit != audit_high_water {
            return Err(GlobalSchemaV1Error::SelectionSnapshotChanged {
                detail: "audit finish high-water differs from captured high-water".to_owned(),
            });
        }
        Ok(PreparedSelectionSchemaInspection {
            database_half,
            authority_state,
            audit_high_water,
            selection_row_counts,
            exact_amended,
        })
    }
}

fn classify_selection_authority_state(
    database_half: &DatabaseHalfDiagnostic,
    audit_present: bool,
    audit: &ValidatedAuditChainSnapshot,
) -> Result<SelectionSchemaAuthorityDiagnostic, GlobalSchemaV1Error> {
    if !audit_present {
        if audit.validation().record_count != 0 || !audit.records().is_empty() {
            return Err(GlobalSchemaV1Error::SelectionAuthorityContradiction {
                detail: "missing audit object produced nonempty validated evidence".to_owned(),
            });
        }
        return Ok(SelectionSchemaAuthorityDiagnostic::DatabaseHalfOnly);
    }

    let has_v2_phase = audit
        .records()
        .iter()
        .any(|record| selection_audit_phase_is_v2(record.phase));
    match (database_half, has_v2_phase) {
        (DatabaseHalfDiagnostic::AbsentDatabaseHalf(_), false) => {
            Ok(SelectionSchemaAuthorityDiagnostic::Absent)
        }
        (DatabaseHalfDiagnostic::PreAmendment(_), false) => {
            Ok(SelectionSchemaAuthorityDiagnostic::PreAmendment)
        }
        (DatabaseHalfDiagnostic::Transitional(_), true) => {
            Ok(SelectionSchemaAuthorityDiagnostic::TransitionalIncomplete)
        }
        (DatabaseHalfDiagnostic::AmendedDatabaseHalf(_), true) => {
            Ok(SelectionSchemaAuthorityDiagnostic::AmendedReceiptVerificationPending)
        }
        (DatabaseHalfDiagnostic::AbsentDatabaseHalf(_), true) => {
            Err(GlobalSchemaV1Error::SelectionAuthorityContradiction {
                detail: "selection audit contains a v2 phase while the database half is absent"
                    .to_owned(),
            })
        }
        (DatabaseHalfDiagnostic::PreAmendment(_), true) => {
            Err(GlobalSchemaV1Error::SelectionAuthorityContradiction {
                detail:
                    "selection audit contains a v2 phase while the database is exact historical"
                        .to_owned(),
            })
        }
        (DatabaseHalfDiagnostic::Transitional(_), false) => {
            Err(GlobalSchemaV1Error::SelectionAuthorityContradiction {
                detail: "transitional database half has no matching v2 audit prefix".to_owned(),
            })
        }
        (DatabaseHalfDiagnostic::AmendedDatabaseHalf(_), false) => {
            Err(GlobalSchemaV1Error::SelectionAuthorityContradiction {
                detail: "amended database half has no matching v2 audit prefix".to_owned(),
            })
        }
    }
}

fn selection_audit_phase_is_v2(phase: SelectionAuditPhase) -> bool {
    matches!(
        phase,
        SelectionAuditPhase::V2ConfigActivationPrepared
            | SelectionAuditPhase::V2ConfigActivationCommitted
            | SelectionAuditPhase::V2IngressPrepared
            | SelectionAuditPhase::V2IngressCommitted
            | SelectionAuditPhase::V2GenerationPrepared
            | SelectionAuditPhase::V2GenerationCommitted
            | SelectionAuditPhase::V2OutcomeClaimPrepared
            | SelectionAuditPhase::V2OutcomeClaimCommitted
            | SelectionAuditPhase::V2OutcomePrepared
            | SelectionAuditPhase::V2OutcomeCommitted
            | SelectionAuditPhase::V2BoardBindingAuditPrepared
            | SelectionAuditPhase::V2BoardBindingAuditCommitted
            | SelectionAuditPhase::V2GateDCanaryVerified
    )
}

/// Opaque proof that the mode-bound database was read as exact `STSA/1` while
/// a shared process/OS maintenance lease and pinned database descriptor remain
/// alive.
#[must_use = "the verified schema capability must retain its maintenance lease"]
pub(crate) struct VerifiedGlobalSchemaV1 {
    identity: GlobalSchemaIdentity,
    _database_file: File,
    _namespace: PinnedNamespace,
    _lease: GlobalSchemaMaintenanceLease,
    _mode: BoundMode,
}

impl fmt::Debug for VerifiedGlobalSchemaV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VerifiedGlobalSchemaV1")
            .field("identity", &self.identity)
            .field("mode", &self._mode.label())
            .finish_non_exhaustive()
    }
}

// Operational consumers arrive with the separate bootstrap-integration slice.
#[allow(dead_code)]
impl VerifiedGlobalSchemaV1 {
    pub(crate) fn identity(&self) -> GlobalSchemaIdentity {
        self.identity
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BoundMode {
    Production,
    Test,
}

impl BoundMode {
    fn label(self) -> &'static str {
        match self {
            Self::Production => "production",
            Self::Test => "test",
        }
    }
}

#[derive(Debug)]
struct ModeBoundPaths {
    mode: BoundMode,
    root: PathBuf,
    database: PathBuf,
    wal: PathBuf,
    shm: PathBuf,
    lock_directory: PathBuf,
    lock_file: PathBuf,
}

impl ModeBoundPaths {
    fn production() -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let database = root.join(PRODUCTION_DATABASE_RELATIVE_PATH);
        let wal = sidecar_path(&database, "-wal");
        let shm = sidecar_path(&database, "-shm");
        let lock_directory = root.join(PRODUCTION_LOCK_DIRECTORY_RELATIVE_PATH);
        let lock_file = lock_directory.join(GLOBAL_MAINTENANCE_LOCK_FILE);
        Self {
            mode: BoundMode::Production,
            root,
            database,
            wal,
            shm,
            lock_directory,
            lock_file,
        }
    }

    fn isolated_test(namespace_root: &Path) -> Result<Self, GlobalSchemaV1Error> {
        let root = namespace_root.to_path_buf();
        let database = root.join("stock_analysis.db");
        let wal = sidecar_path(&database, "-wal");
        let shm = sidecar_path(&database, "-shm");
        let lock_directory = root.join("locks");
        let lock_file = lock_directory.join(GLOBAL_MAINTENANCE_LOCK_FILE);
        let paths = Self {
            mode: BoundMode::Test,
            root,
            database,
            wal,
            shm,
            lock_directory,
            lock_file,
        };
        paths.validate_mode_binding()?;
        Ok(paths)
    }

    fn validate_mode_binding(&self) -> Result<(), GlobalSchemaV1Error> {
        validate_absolute_normal_path(&self.root)?;
        validate_absolute_normal_path(&self.database)?;
        validate_absolute_normal_path(&self.wal)?;
        validate_absolute_normal_path(&self.shm)?;
        validate_absolute_normal_path(&self.lock_directory)?;
        validate_absolute_normal_path(&self.lock_file)?;
        for (label, path) in [
            ("database", &self.database),
            ("WAL", &self.wal),
            ("SHM", &self.shm),
            ("lock directory", &self.lock_directory),
            ("lock file", &self.lock_file),
        ] {
            if !path.starts_with(&self.root) {
                return Err(GlobalSchemaV1Error::ModeBindingViolation {
                    detail: format!("{label} escaped the bound root"),
                });
            }
        }

        match self.mode {
            BoundMode::Production => {
                let fixed = Path::new(env!("CARGO_MANIFEST_DIR"));
                let fixed_database = fixed.join(PRODUCTION_DATABASE_RELATIVE_PATH);
                if self.root != fixed
                    || self.database != fixed_database
                    || self.wal != sidecar_path(&fixed_database, "-wal")
                    || self.shm != sidecar_path(&fixed_database, "-shm")
                    || self.lock_directory != fixed.join(PRODUCTION_LOCK_DIRECTORY_RELATIVE_PATH)
                    || self.lock_file
                        != fixed
                            .join(PRODUCTION_LOCK_DIRECTORY_RELATIVE_PATH)
                            .join(GLOBAL_MAINTENANCE_LOCK_FILE)
                {
                    return Err(GlobalSchemaV1Error::ModeBindingViolation {
                        detail: "production paths differ from manifest-root fixed identities"
                            .to_owned(),
                    });
                }
            }
            BoundMode::Test => {
                let leaf = self
                    .root
                    .file_name()
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| GlobalSchemaV1Error::ModeBindingViolation {
                        detail: "test namespace leaf is not UTF-8".to_owned(),
                    })?;
                if !is_exact_test_namespace_leaf(leaf) {
                    return Err(GlobalSchemaV1Error::ModeBindingViolation {
                        detail:
                            "test namespace must be invocation-isolated and begin with TEST_CODE_"
                                .to_owned(),
                    });
                }
                let production = Self::production();
                if self.root.starts_with(&production.root)
                    || production.root.starts_with(&self.root)
                    || self.database == production.database
                    || self.lock_file == production.lock_file
                {
                    return Err(GlobalSchemaV1Error::ModeBindingViolation {
                        detail: "test and production physical identities overlap".to_owned(),
                    });
                }
            }
        }
        Ok(())
    }
}

fn is_exact_test_namespace_leaf(value: &str) -> bool {
    let suffix = match value.strip_prefix("TEST_CODE_") {
        Some(suffix) => suffix,
        None => return false,
    };
    !suffix.is_empty()
        && suffix
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn sidecar_path(database: &Path, suffix: &str) -> PathBuf {
    let mut value = OsString::from(database.as_os_str());
    value.push(suffix);
    PathBuf::from(value)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DirectoryIdentity {
    device: u64,
    inode: u64,
}

impl DirectoryIdentity {
    fn from_metadata(metadata: &Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        }
    }
}

struct PinnedRoot {
    path: PathBuf,
    file: File,
    identity: DirectoryIdentity,
}

impl PinnedRoot {
    fn open(path: &Path) -> Result<Self, GlobalSchemaV1Error> {
        let file = open_absolute_directory_no_follow(path, "mode-bound root")?;
        let metadata = file.metadata().map_err(|source| GlobalSchemaV1Error::Io {
            operation: "fstat pinned mode-bound root",
            path: path.to_path_buf(),
            source,
        })?;
        Ok(Self {
            path: path.to_path_buf(),
            file,
            identity: DirectoryIdentity::from_metadata(&metadata),
        })
    }

    fn validate_unchanged(&self) -> Result<(), GlobalSchemaV1Error> {
        let reopened = open_absolute_directory_no_follow(&self.path, "revalidate mode-bound root")?;
        let reopened_identity =
            DirectoryIdentity::from_metadata(&reopened.metadata().map_err(|source| {
                GlobalSchemaV1Error::Io {
                    operation: "fstat reopened mode-bound root",
                    path: self.path.clone(),
                    source,
                }
            })?);
        let pinned_identity =
            DirectoryIdentity::from_metadata(&self.file.metadata().map_err(|source| {
                GlobalSchemaV1Error::Io {
                    operation: "fstat retained mode-bound root",
                    path: self.path.clone(),
                    source,
                }
            })?);
        if reopened_identity != self.identity || pinned_identity != self.identity {
            return Err(GlobalSchemaV1Error::ObjectIdentityChanged {
                path: self.path.clone(),
            });
        }
        Ok(())
    }
}

struct PinnedDirectory {
    path: PathBuf,
    root: File,
    relative_components: Vec<OsString>,
    file: File,
    identity: DirectoryIdentity,
}

impl PinnedDirectory {
    fn for_parent(
        root: &PinnedRoot,
        path: &Path,
        label: &'static str,
    ) -> Result<(Self, OsString), GlobalSchemaV1Error> {
        let relative = path.strip_prefix(&root.path).map_err(|_| {
            GlobalSchemaV1Error::ModeBindingViolation {
                detail: format!("{label} escaped the retained mode-bound root"),
            }
        })?;
        let mut components = normal_relative_components(relative, path)?;
        let leaf = components
            .pop()
            .ok_or_else(|| GlobalSchemaV1Error::UnsafeFixedPath {
                detail: format!("{label} has no leaf: {}", path.display()),
            })?;
        let directory = Self::open_components(root, components, label, false)?;
        Ok((directory, leaf))
    }

    fn open_or_create_exact_directory(
        root: &PinnedRoot,
        path: &Path,
        label: &'static str,
    ) -> Result<Self, GlobalSchemaV1Error> {
        let relative = path.strip_prefix(&root.path).map_err(|_| {
            GlobalSchemaV1Error::ModeBindingViolation {
                detail: format!("{label} escaped the retained mode-bound root"),
            }
        })?;
        let components = normal_relative_components(relative, path)?;
        Self::open_components(root, components, label, true)
    }

    fn open_components(
        root: &PinnedRoot,
        components: Vec<OsString>,
        label: &'static str,
        create_last: bool,
    ) -> Result<Self, GlobalSchemaV1Error> {
        let mut directory = root
            .file
            .try_clone()
            .map_err(|source| GlobalSchemaV1Error::Io {
                operation: "clone retained mode-bound root",
                path: root.path.clone(),
                source,
            })?;
        let mut directory_path = root.path.clone();
        for (index, component) in components.iter().enumerate() {
            let is_last = index + 1 == components.len();
            let next = match openat_component(&directory, component, O_RDONLY_FLAG, false) {
                Ok(next) => next,
                Err(source)
                    if create_last && is_last && source.kind() == io::ErrorKind::NotFound =>
                {
                    mkdirat_component(&directory, component).map_err(|source| {
                        GlobalSchemaV1Error::Io {
                            operation: "create exact lock directory beneath pinned namespace",
                            path: directory_path.join(component),
                            source,
                        }
                    })?;
                    sync_directory_descriptor(&directory, &directory_path)?;
                    openat_component(&directory, component, O_RDONLY_FLAG, false).map_err(
                        |source| GlobalSchemaV1Error::Io {
                            operation: "open newly created pinned lock directory",
                            path: directory_path.join(component),
                            source,
                        },
                    )?
                }
                Err(source) => {
                    return Err(GlobalSchemaV1Error::Io {
                        operation: "descriptor-traverse pinned directory",
                        path: directory_path.join(component),
                        source,
                    });
                }
            };
            let metadata = next.metadata().map_err(|source| GlobalSchemaV1Error::Io {
                operation: "fstat descriptor-traversed directory",
                path: directory_path.join(component),
                source,
            })?;
            if !metadata.is_dir() {
                return Err(GlobalSchemaV1Error::UnsafeFixedPath {
                    detail: format!(
                        "{label} component is not a directory: {}",
                        directory_path.join(component).display()
                    ),
                });
            }
            directory_path.push(component);
            directory = next;
        }
        let identity =
            DirectoryIdentity::from_metadata(&directory.metadata().map_err(|source| {
                GlobalSchemaV1Error::Io {
                    operation: "fstat retained pinned directory",
                    path: directory_path.clone(),
                    source,
                }
            })?);
        Ok(Self {
            path: directory_path,
            root: root
                .file
                .try_clone()
                .map_err(|source| GlobalSchemaV1Error::Io {
                    operation: "clone pinned root for directory retention",
                    path: root.path.clone(),
                    source,
                })?,
            relative_components: components,
            file: directory,
            identity,
        })
    }

    fn validate_unchanged(&self) -> Result<(), GlobalSchemaV1Error> {
        let mut current = self
            .root
            .try_clone()
            .map_err(|source| GlobalSchemaV1Error::Io {
                operation: "clone pinned root for directory revalidation",
                path: self.path.clone(),
                source,
            })?;
        for component in &self.relative_components {
            current =
                openat_component(&current, component, O_RDONLY_FLAG, false).map_err(|source| {
                    GlobalSchemaV1Error::Io {
                        operation: "re-traverse retained pinned directory",
                        path: self.path.clone(),
                        source,
                    }
                })?;
            if !current
                .metadata()
                .map_err(|source| GlobalSchemaV1Error::Io {
                    operation: "fstat re-traversed pinned directory",
                    path: self.path.clone(),
                    source,
                })?
                .is_dir()
            {
                return Err(GlobalSchemaV1Error::UnsafeFixedPath {
                    detail: format!(
                        "retained namespace component changed type: {}",
                        self.path.display()
                    ),
                });
            }
        }
        let reopened = DirectoryIdentity::from_metadata(&current.metadata().map_err(|source| {
            GlobalSchemaV1Error::Io {
                operation: "fstat reopened pinned directory",
                path: self.path.clone(),
                source,
            }
        })?);
        let retained =
            DirectoryIdentity::from_metadata(&self.file.metadata().map_err(|source| {
                GlobalSchemaV1Error::Io {
                    operation: "fstat retained pinned directory",
                    path: self.path.clone(),
                    source,
                }
            })?);
        if reopened != self.identity || retained != self.identity {
            return Err(GlobalSchemaV1Error::ObjectIdentityChanged {
                path: self.path.clone(),
            });
        }
        Ok(())
    }
}

struct PinnedNamespace {
    root: PinnedRoot,
    database_parent: PinnedDirectory,
    database_leaf: OsString,
    lock_parent: PinnedDirectory,
    lock_leaf: OsString,
}

impl PinnedNamespace {
    fn open(paths: &ModeBoundPaths) -> Result<Self, GlobalSchemaV1Error> {
        let root = PinnedRoot::open(&paths.root)?;
        Self::from_root(paths, root)
    }

    fn from_root(paths: &ModeBoundPaths, root: PinnedRoot) -> Result<Self, GlobalSchemaV1Error> {
        if root.path != paths.root {
            return Err(GlobalSchemaV1Error::ModeBindingViolation {
                detail: "retained root does not match mode-bound TEST_CODE root".to_owned(),
            });
        }
        root.validate_unchanged()?;
        let (database_parent, database_leaf) =
            PinnedDirectory::for_parent(&root, &paths.database, "global database")?;
        let lock_parent = PinnedDirectory::open_or_create_exact_directory(
            &root,
            &paths.lock_directory,
            "global maintenance lock directory",
        )?;
        let lock_leaf = paths
            .lock_file
            .file_name()
            .ok_or_else(|| GlobalSchemaV1Error::UnsafeFixedPath {
                detail: format!(
                    "global maintenance lock has no leaf: {}",
                    paths.lock_file.display()
                ),
            })?
            .to_os_string();
        let namespace = Self {
            root,
            database_parent,
            database_leaf,
            lock_parent,
            lock_leaf,
        };
        namespace.validate_unchanged()?;
        Ok(namespace)
    }

    fn validate_unchanged(&self) -> Result<(), GlobalSchemaV1Error> {
        self.root.validate_unchanged()?;
        self.database_parent.validate_unchanged()?;
        self.lock_parent.validate_unchanged()
    }
}

fn inspect_bound_database(
    paths: ModeBoundPaths,
) -> Result<VerifiedGlobalSchemaV1, GlobalSchemaV1Error> {
    paths.validate_mode_binding()?;
    let namespace = PinnedNamespace::open(&paths)?;
    let lease = GlobalSchemaMaintenanceLease::acquire_shared(&paths, &namespace)?;
    let (database_file, database_identity) = open_pinned_regular_read_only(
        &namespace.database_parent,
        &namespace.database_leaf,
        &paths.database,
    )?;
    require_test_single_link(&paths, &paths.database, &database_file)?;
    require_no_live_sidecars(&paths, &namespace)?;
    let identity = read_identity_from_pinned_database(&database_file, &paths.database)?;
    require_same_file_identity(
        &namespace.database_parent,
        &namespace.database_leaf,
        &paths.database,
        &database_file,
        database_identity,
        "database",
    )?;
    require_sidecars_absent(&paths, &namespace)?;
    namespace.validate_unchanged()?;
    let identity = classify_identity(identity.application_id, identity.user_version)?;
    Ok(VerifiedGlobalSchemaV1 {
        identity,
        _database_file: database_file,
        _namespace: namespace,
        _lease: lease,
        _mode: paths.mode,
    })
}

fn classify_identity(
    application_id: i64,
    user_version: i64,
) -> Result<GlobalSchemaIdentity, GlobalSchemaV1Error> {
    if application_id == STOCK_ANALYSIS_SQLITE_APPLICATION_ID
        && user_version == STOCK_ANALYSIS_DB_SCHEMA_GENERATION
    {
        return Ok(GlobalSchemaIdentity {
            application_id,
            user_version,
        });
    }
    if application_id == STOCK_ANALYSIS_SQLITE_APPLICATION_ID
        && user_version > STOCK_ANALYSIS_DB_SCHEMA_GENERATION
    {
        return Err(GlobalSchemaV1Error::UnsupportedFutureGeneration {
            actual: user_version,
            supported: STOCK_ANALYSIS_DB_SCHEMA_GENERATION,
        });
    }
    if application_id == 0 && user_version == 0 {
        return Err(GlobalSchemaV1Error::OfflineGlobalMigrationRequired {
            application_id,
            user_version,
        });
    }
    Err(GlobalSchemaV1Error::UnsupportedIdentity {
        application_id,
        user_version,
    })
}

struct ProcessSharedLease;

impl ProcessSharedLease {
    fn try_acquire() -> Result<Self, GlobalSchemaV1Error> {
        loop {
            if PROCESS_EXCLUSIVE_LEASE.load(Ordering::Acquire) {
                return Err(GlobalSchemaV1Error::ProcessMaintenanceLeaseUnavailable);
            }
            PROCESS_SHARED_LEASES.fetch_add(1, Ordering::AcqRel);
            if !PROCESS_EXCLUSIVE_LEASE.load(Ordering::Acquire) {
                return Ok(Self);
            }
            let previous = PROCESS_SHARED_LEASES.fetch_sub(1, Ordering::AcqRel);
            debug_assert!(previous > 0, "global process lease count underflow");
        }
    }
}

impl Drop for ProcessSharedLease {
    fn drop(&mut self) {
        let previous = PROCESS_SHARED_LEASES.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0, "global process lease count underflow");
    }
}

struct ProcessExclusiveLease;

impl ProcessExclusiveLease {
    fn try_acquire() -> Result<Self, GlobalSchemaV1Error> {
        if PROCESS_SHARED_LEASES.load(Ordering::Acquire) > 0 {
            return Err(GlobalSchemaV1Error::SharedToExclusiveUpgradeForbidden);
        }
        PROCESS_EXCLUSIVE_LEASE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| GlobalSchemaV1Error::ExclusiveProcessMaintenanceLeaseUnavailable)?;
        if PROCESS_SHARED_LEASES.load(Ordering::Acquire) > 0 {
            PROCESS_EXCLUSIVE_LEASE.store(false, Ordering::Release);
            return Err(GlobalSchemaV1Error::SharedToExclusiveUpgradeForbidden);
        }
        Ok(Self)
    }
}

impl Drop for ProcessExclusiveLease {
    fn drop(&mut self) {
        let held = PROCESS_EXCLUSIVE_LEASE.swap(false, Ordering::AcqRel);
        debug_assert!(held, "global exclusive process lease was not retained");
    }
}

struct GlobalSchemaMaintenanceLease {
    lock_file: File,
    lock_identity: FileIdentity,
    _process: ProcessSharedLease,
}

impl GlobalSchemaMaintenanceLease {
    fn acquire_shared(
        paths: &ModeBoundPaths,
        namespace: &PinnedNamespace,
    ) -> Result<Self, GlobalSchemaV1Error> {
        let process = ProcessSharedLease::try_acquire()?;
        let (lock_file, lock_identity) = open_maintenance_lock(paths, namespace)?;
        FileExt::try_lock_shared(&lock_file).map_err(|source| {
            GlobalSchemaV1Error::MaintenanceLeaseUnavailable {
                path: paths.lock_file.clone(),
                retryable: source.kind() == io::ErrorKind::WouldBlock,
                source,
            }
        })?;
        require_same_file_identity(
            &namespace.lock_parent,
            &namespace.lock_leaf,
            &paths.lock_file,
            &lock_file,
            lock_identity,
            "global maintenance lock",
        )?;
        namespace.validate_unchanged()?;
        Ok(Self {
            lock_file,
            lock_identity,
            _process: process,
        })
    }
}

impl Drop for GlobalSchemaMaintenanceLease {
    fn drop(&mut self) {
        debug_assert_eq!(
            self.lock_file
                .metadata()
                .ok()
                .map(|metadata| FileIdentity::from_metadata(&metadata)),
            Some(self.lock_identity),
            "global maintenance lock descriptor changed identity"
        );
        let _ = FileExt::unlock(&self.lock_file);
    }
}

/// Opaque exclusive authority for offline schema maintenance or an isolated
/// TEST_CODE fresh initialization. Holding it grants no SQLite write API.
#[must_use = "exclusive global schema authority must remain alive for the full maintenance scope"]
pub(crate) struct ExclusiveGlobalSchemaMaintenanceLease {
    // Field order is part of the lifecycle contract. After `Drop` unlocks the
    // OS lease, Rust closes the lock descriptor, then the pinned namespace,
    // and finally releases the in-process exclusive reservation.
    lock_file: File,
    namespace: PinnedNamespace,
    lock_identity: FileIdentity,
    _process: ProcessExclusiveLease,
}

impl fmt::Debug for ExclusiveGlobalSchemaMaintenanceLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExclusiveGlobalSchemaMaintenanceLease")
            .finish_non_exhaustive()
    }
}

fn acquire_exclusive_bound(
    paths: ModeBoundPaths,
) -> Result<ExclusiveGlobalSchemaMaintenanceLease, GlobalSchemaV1Error> {
    paths.validate_mode_binding()?;
    let namespace = PinnedNamespace::open(&paths)?;
    acquire_exclusive_in_namespace(paths, namespace)
}

fn acquire_exclusive_with_pinned_root(
    paths: ModeBoundPaths,
    root: PinnedRoot,
) -> Result<ExclusiveGlobalSchemaMaintenanceLease, GlobalSchemaV1Error> {
    paths.validate_mode_binding()?;
    let namespace = PinnedNamespace::from_root(&paths, root)?;
    acquire_exclusive_in_namespace(paths, namespace)
}

fn acquire_exclusive_in_namespace(
    paths: ModeBoundPaths,
    namespace: PinnedNamespace,
) -> Result<ExclusiveGlobalSchemaMaintenanceLease, GlobalSchemaV1Error> {
    let process = ProcessExclusiveLease::try_acquire()?;
    let (lock_file, lock_identity) = open_maintenance_lock(&paths, &namespace)?;
    FileExt::try_lock_exclusive(&lock_file).map_err(|source| {
        GlobalSchemaV1Error::ExclusiveMaintenanceLeaseUnavailable {
            path: paths.lock_file.clone(),
            retryable: source.kind() == io::ErrorKind::WouldBlock,
            source,
        }
    })?;
    require_same_file_identity(
        &namespace.lock_parent,
        &namespace.lock_leaf,
        &paths.lock_file,
        &lock_file,
        lock_identity,
        "exclusive global maintenance lock",
    )?;
    namespace.validate_unchanged()?;
    Ok(ExclusiveGlobalSchemaMaintenanceLease {
        lock_file,
        namespace,
        lock_identity,
        _process: process,
    })
}

impl Drop for ExclusiveGlobalSchemaMaintenanceLease {
    fn drop(&mut self) {
        debug_assert_eq!(
            self.lock_file
                .metadata()
                .ok()
                .map(|metadata| FileIdentity::from_metadata(&metadata)),
            Some(self.lock_identity),
            "exclusive global maintenance lock descriptor changed identity"
        );
        let _ = FileExt::unlock(&self.lock_file);
    }
}

fn open_maintenance_lock(
    paths: &ModeBoundPaths,
    namespace: &PinnedNamespace,
) -> Result<(File, FileIdentity), GlobalSchemaV1Error> {
    let lock_file = openat_component(
        &namespace.lock_parent.file,
        &namespace.lock_leaf,
        O_RDWR_FLAG,
        true,
    )
    .map_err(|source| {
        if source.raw_os_error() == Some(ELOOP_CODE) {
            return GlobalSchemaV1Error::UnsafeFixedPath {
                detail: format!(
                    "global maintenance lock is a symlink: {}",
                    paths.lock_file.display()
                ),
            };
        }
        GlobalSchemaV1Error::Io {
            operation: "open global maintenance lock no-follow",
            path: paths.lock_file.clone(),
            source,
        }
    })?;
    let metadata = lock_file
        .metadata()
        .map_err(|source| GlobalSchemaV1Error::Io {
            operation: "fstat global maintenance lock",
            path: paths.lock_file.clone(),
            source,
        })?;
    if !metadata.is_file() {
        return Err(GlobalSchemaV1Error::UnsafeFixedPath {
            detail: format!(
                "global maintenance lock is not a regular file: {}",
                paths.lock_file.display()
            ),
        });
    }
    require_test_single_link(paths, &paths.lock_file, &lock_file)?;
    lock_file
        .sync_all()
        .map_err(|source| GlobalSchemaV1Error::Io {
            operation: "sync global maintenance lock",
            path: paths.lock_file.clone(),
            source,
        })?;
    sync_directory_descriptor(&namespace.lock_parent.file, &paths.lock_directory)?;
    let lock_identity = FileIdentity::from_metadata(&lock_file.metadata().map_err(|source| {
        GlobalSchemaV1Error::Io {
            operation: "fstat synced global maintenance lock",
            path: paths.lock_file.clone(),
            source,
        }
    })?);
    require_same_file_identity(
        &namespace.lock_parent,
        &namespace.lock_leaf,
        &paths.lock_file,
        &lock_file,
        lock_identity,
        "opened global maintenance lock",
    )?;
    namespace.validate_unchanged()?;
    Ok((lock_file, lock_identity))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    device: u64,
    inode: u64,
    length: u64,
}

impl FileIdentity {
    fn from_metadata(metadata: &Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            length: metadata.len(),
        }
    }
}

fn validate_absolute_normal_path(path: &Path) -> Result<(), GlobalSchemaV1Error> {
    if !path.is_absolute() {
        return Err(GlobalSchemaV1Error::UnsafeFixedPath {
            detail: format!("path is not absolute: {}", path.display()),
        });
    }
    for component in path.components() {
        match component {
            Component::RootDir | Component::Normal(_) => {}
            Component::CurDir | Component::ParentDir | Component::Prefix(_) => {
                return Err(GlobalSchemaV1Error::UnsafeFixedPath {
                    detail: format!("path contains forbidden component: {}", path.display()),
                });
            }
        }
    }
    Ok(())
}

fn normal_relative_components(
    relative: &Path,
    full_path: &Path,
) -> Result<Vec<OsString>, GlobalSchemaV1Error> {
    relative
        .components()
        .map(|component| match component {
            Component::Normal(value) => Ok(value.to_os_string()),
            _ => Err(GlobalSchemaV1Error::UnsafeFixedPath {
                detail: format!(
                    "mode-bound relative path contains forbidden component: {}",
                    full_path.display()
                ),
            }),
        })
        .collect()
}

fn component_cstring(name: &OsStr) -> Result<CString, io::Error> {
    if name.is_empty() || name.as_bytes().contains(&b'/') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "descriptor-relative component must be one non-empty path segment",
        ));
    }
    CString::new(name.as_bytes()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "descriptor-relative component contains NUL",
        )
    })
}

fn openat_component(
    parent: &File,
    name: &OsStr,
    access: i32,
    create: bool,
) -> Result<File, io::Error> {
    let name = component_cstring(name)?;
    let create_flag = if create { O_CREAT_FLAG } else { 0 };
    // SAFETY: `name` is a live NUL-terminated single component, `parent`
    // retains a valid directory descriptor, and the returned descriptor is
    // immediately transferred to `File`.
    let descriptor = unsafe {
        openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            access | create_flag | O_NOFOLLOW_FLAG | O_NONBLOCK_FLAG | O_CLOEXEC_FLAG,
            0o600_u32,
        )
    };
    if descriptor < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful `openat` returns one newly owned descriptor.
    Ok(unsafe { File::from_raw_fd(descriptor) })
}

fn openat_new_regular(parent: &File, name: &OsStr) -> Result<File, io::Error> {
    let name = component_cstring(name)?;
    // SAFETY: `name` is one live NUL-terminated component, `parent` retains a
    // valid directory descriptor, and O_EXCL prevents replacement/alias reuse.
    let descriptor = unsafe {
        openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            O_WRONLY_FLAG
                | O_CREAT_FLAG
                | O_EXCL_FLAG
                | O_NOFOLLOW_FLAG
                | O_NONBLOCK_FLAG
                | O_CLOEXEC_FLAG,
            0o600_u32,
        )
    };
    if descriptor < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful `openat` returns one newly owned descriptor.
    Ok(unsafe { File::from_raw_fd(descriptor) })
}

fn mkdirat_component(parent: &File, name: &OsStr) -> Result<(), io::Error> {
    let name = component_cstring(name)?;
    // SAFETY: `name` is a live NUL-terminated component and `parent` retains a
    // valid directory descriptor.
    let result = unsafe { mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700_u32) };
    if result < 0 {
        let source = io::Error::last_os_error();
        if source.kind() != io::ErrorKind::AlreadyExists {
            return Err(source);
        }
    }
    Ok(())
}

fn mkdirat_new_component(parent: &File, name: &OsStr) -> Result<(), io::Error> {
    let name = component_cstring(name)?;
    // SAFETY: `name` is a live NUL-terminated component and `parent` retains a
    // valid directory descriptor. EEXIST remains an error for unique roots.
    let result = unsafe { mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700_u32) };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn renameat_component(parent: &File, old_name: &OsStr, new_name: &OsStr) -> Result<(), io::Error> {
    let old_name = component_cstring(old_name)?;
    let new_name = component_cstring(new_name)?;
    // SAFETY: both names are live NUL-terminated single components and the
    // retained parent descriptor scopes both sides of the atomic rename.
    let result = unsafe {
        renameat(
            parent.as_raw_fd(),
            old_name.as_ptr(),
            parent.as_raw_fd(),
            new_name.as_ptr(),
        )
    };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn unlinkat_component(parent: &File, name: &OsStr) -> Result<(), io::Error> {
    let name = component_cstring(name)?;
    // SAFETY: `name` is one live NUL-terminated component, `parent` retains
    // the authoritative directory descriptor, and flags=0 removes only a
    // non-directory entry beneath that descriptor.
    let result = unsafe { unlinkat(parent.as_raw_fd(), name.as_ptr(), 0) };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn unpredictable_owner_nonce() -> Result<String, GlobalSchemaV1Error> {
    let path = PathBuf::from("/dev/urandom");
    let mut source = File::open(&path).map_err(|source| GlobalSchemaV1Error::Io {
        operation: "open OS random source for TEST_CODE owner nonce",
        path: path.clone(),
        source,
    })?;
    let mut bytes = [0_u8; 16];
    source
        .read_exact(&mut bytes)
        .map_err(|source| GlobalSchemaV1Error::Io {
            operation: "read OS random source for TEST_CODE owner nonce",
            path,
            source,
        })?;
    Ok(bytes
        .into_iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(""))
}

fn open_absolute_directory_no_follow(
    path: &Path,
    operation: &'static str,
) -> Result<File, GlobalSchemaV1Error> {
    validate_absolute_normal_path(path)?;
    let mut directory = OpenOptions::new()
        .read(true)
        .custom_flags(O_NOFOLLOW_FLAG | O_NONBLOCK_FLAG | O_CLOEXEC_FLAG)
        .open("/")
        .map_err(|source| GlobalSchemaV1Error::Io {
            operation,
            path: PathBuf::from("/"),
            source,
        })?;
    let mut traversed = PathBuf::from("/");
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => {
                let next =
                    openat_component(&directory, name, O_RDONLY_FLAG, false).map_err(|source| {
                        GlobalSchemaV1Error::Io {
                            operation,
                            path: traversed.join(name),
                            source,
                        }
                    })?;
                let metadata = next.metadata().map_err(|source| GlobalSchemaV1Error::Io {
                    operation,
                    path: traversed.join(name),
                    source,
                })?;
                if !metadata.is_dir() {
                    return Err(GlobalSchemaV1Error::UnsafeFixedPath {
                        detail: format!(
                            "descriptor-traversed root component is not a directory: {}",
                            traversed.join(name).display()
                        ),
                    });
                }
                traversed.push(name);
                directory = next;
            }
            Component::CurDir | Component::ParentDir | Component::Prefix(_) => {
                unreachable!("absolute normal path was validated")
            }
        }
    }
    Ok(directory)
}

fn sync_directory_descriptor(directory: &File, path: &Path) -> Result<(), GlobalSchemaV1Error> {
    if !directory
        .metadata()
        .map_err(|source| GlobalSchemaV1Error::Io {
            operation: "fstat pinned directory for sync",
            path: path.to_path_buf(),
            source,
        })?
        .is_dir()
    {
        return Err(GlobalSchemaV1Error::UnsafeFixedPath {
            detail: format!("sync target is not a pinned directory: {}", path.display()),
        });
    }
    directory
        .sync_all()
        .map_err(|source| GlobalSchemaV1Error::Io {
            operation: "sync pinned directory",
            path: path.to_path_buf(),
            source,
        })
}

fn copy_pinned_file_to_new_descriptor(
    source: &File,
    source_path: &Path,
    destination_parent: &File,
    destination_leaf: &OsStr,
    destination_path: &Path,
    label: &'static str,
) -> Result<(), GlobalSchemaV1Error> {
    let mut source = source
        .try_clone()
        .map_err(|source| GlobalSchemaV1Error::Io {
            operation: "clone pinned rehearsal source",
            path: source_path.to_path_buf(),
            source,
        })?;
    source
        .seek(SeekFrom::Start(0))
        .map_err(|source| GlobalSchemaV1Error::Io {
            operation: "seek pinned rehearsal source",
            path: source_path.to_path_buf(),
            source,
        })?;
    let mut destination =
        openat_new_regular(destination_parent, destination_leaf).map_err(|source| {
            GlobalSchemaV1Error::Io {
                operation: "create TEST_CODE rehearsal copy descriptor-relative no-follow",
                path: destination_path.to_path_buf(),
                source,
            }
        })?;
    io::copy(&mut source, &mut destination).map_err(|source| GlobalSchemaV1Error::Io {
        operation: "copy pinned source into TEST_CODE rehearsal",
        path: destination_path.to_path_buf(),
        source,
    })?;
    destination
        .sync_all()
        .map_err(|source| GlobalSchemaV1Error::Io {
            operation: "fsync TEST_CODE rehearsal copy",
            path: destination_path.to_path_buf(),
            source,
        })?;
    let parent_path =
        destination_path
            .parent()
            .ok_or_else(|| GlobalSchemaV1Error::UnsafeFixedPath {
                detail: format!("{label} destination has no parent"),
            })?;
    sync_directory_descriptor(destination_parent, parent_path)
}

fn open_pinned_regular_read_only(
    parent: &PinnedDirectory,
    leaf: &OsStr,
    path: &Path,
) -> Result<(File, FileIdentity), GlobalSchemaV1Error> {
    let file = openat_component(&parent.file, leaf, O_RDONLY_FLAG, false).map_err(|source| {
        if source.raw_os_error() == Some(ELOOP_CODE) {
            return GlobalSchemaV1Error::UnsafeFixedPath {
                detail: format!("database or sidecar is a symlink: {}", path.display()),
            };
        }
        GlobalSchemaV1Error::DatabaseUnavailable {
            path: path.to_path_buf(),
            source,
        }
    })?;
    let metadata = file.metadata().map_err(|source| GlobalSchemaV1Error::Io {
        operation: "fstat fixed database",
        path: path.to_path_buf(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(GlobalSchemaV1Error::DatabaseNotRegular {
            path: path.to_path_buf(),
        });
    }
    let identity = FileIdentity::from_metadata(&metadata);
    require_same_file_identity(parent, leaf, path, &file, identity, "database")?;
    Ok((file, identity))
}

fn pin_optional_selection_audit(
    parent: &PinnedDirectory,
    leaf: &OsStr,
    path: &Path,
) -> Result<PinnedSelectionAuditFile, GlobalSchemaV1Error> {
    let file = match openat_component(&parent.file, leaf, O_RDONLY_FLAG, false) {
        Ok(file) => file,
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            parent.validate_unchanged()?;
            return Ok(PinnedSelectionAuditFile::Missing);
        }
        Err(source) if source.raw_os_error() == Some(ELOOP_CODE) => {
            return Err(GlobalSchemaV1Error::UnsafeFixedPath {
                detail: format!("selection audit is a symlink: {}", path.display()),
            });
        }
        Err(source) => {
            return Err(GlobalSchemaV1Error::Io {
                operation: "pin optional selection audit",
                path: path.to_path_buf(),
                source,
            });
        }
    };
    let metadata = file.metadata().map_err(|source| GlobalSchemaV1Error::Io {
        operation: "fstat pinned selection audit",
        path: path.to_path_buf(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(GlobalSchemaV1Error::DatabaseNotRegular {
            path: path.to_path_buf(),
        });
    }
    let identity = FileIdentity::from_metadata(&metadata);
    require_same_file_identity(parent, leaf, path, &file, identity, "selection audit")?;
    Ok(PinnedSelectionAuditFile::Present { file, identity })
}

fn revalidate_selection_audit_file(
    parent: &PinnedDirectory,
    leaf: &OsStr,
    path: &Path,
    pinned: &PinnedSelectionAuditFile,
) -> Result<(), GlobalSchemaV1Error> {
    match pinned {
        PinnedSelectionAuditFile::Missing => {
            parent.validate_unchanged()?;
            match openat_component(&parent.file, leaf, O_RDONLY_FLAG, false) {
                Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
                Ok(_) => Err(GlobalSchemaV1Error::ObjectIdentityChanged {
                    path: path.to_path_buf(),
                }),
                Err(source) => Err(GlobalSchemaV1Error::Io {
                    operation: "revalidate missing selection audit",
                    path: path.to_path_buf(),
                    source,
                }),
            }
        }
        PinnedSelectionAuditFile::Present { file, identity } => require_same_file_identity(
            parent,
            leaf,
            path,
            file,
            *identity,
            "revalidate selection audit",
        ),
    }
}

fn open_pinned_regular_read_write(
    parent: &PinnedDirectory,
    leaf: &OsStr,
    path: &Path,
) -> Result<(File, FileIdentity), GlobalSchemaV1Error> {
    let file = openat_component(&parent.file, leaf, O_RDWR_FLAG, false).map_err(|source| {
        if source.raw_os_error() == Some(ELOOP_CODE) {
            return GlobalSchemaV1Error::UnsafeFixedPath {
                detail: format!("database is a symlink: {}", path.display()),
            };
        }
        GlobalSchemaV1Error::DatabaseUnavailable {
            path: path.to_path_buf(),
            source,
        }
    })?;
    let metadata = file.metadata().map_err(|source| GlobalSchemaV1Error::Io {
        operation: "fstat writable fixed database",
        path: path.to_path_buf(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(GlobalSchemaV1Error::DatabaseNotRegular {
            path: path.to_path_buf(),
        });
    }
    let identity = FileIdentity::from_metadata(&metadata);
    require_same_file_identity(
        parent,
        leaf,
        path,
        &file,
        identity,
        "writable selection inspection database",
    )?;
    Ok((file, identity))
}

fn open_pinned_sqlite_read_write(
    database_parent: &PinnedDirectory,
    database_leaf: &OsStr,
    database_file: &File,
    database_identity: FileIdentity,
    database_path: &Path,
) -> Result<Connection, GlobalSchemaV1Error> {
    require_same_file_identity(
        database_parent,
        database_leaf,
        database_path,
        database_file,
        database_identity,
        "revalidate database before retained-parent SQLite open",
    )?;
    let descriptor_route =
        sqlite_open_route_from_retained_parent(&database_parent.file, database_leaf).map_err(
            |source| GlobalSchemaV1Error::Io {
                operation: "derive retained-parent SQLite open route",
                path: database_path.to_path_buf(),
                source: io::Error::other(source.to_string()),
            },
        )?;
    let uri = format!("file:{}?mode=rw", descriptor_route.to_string_lossy());
    let connection = Connection::open_with_flags(
        uri,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|source| GlobalSchemaV1Error::SelectionSqlite {
        operation: "open retained-parent database read-write",
        source,
    })?;
    require_same_file_identity(
        database_parent,
        database_leaf,
        database_path,
        database_file,
        database_identity,
        "revalidate database after retained-parent SQLite open",
    )?;
    let routed_metadata =
        fs::metadata(&descriptor_route).map_err(|source| GlobalSchemaV1Error::Io {
            operation: "stat retained-parent SQLite open route",
            path: database_path.to_path_buf(),
            source,
        })?;
    if !routed_metadata.is_file()
        || FileIdentity::from_metadata(&routed_metadata) != database_identity
    {
        return Err(GlobalSchemaV1Error::ObjectIdentityChanged {
            path: database_path.to_path_buf(),
        });
    }
    Ok(connection)
}

fn require_no_live_sidecars_for_bound_namespace(
    namespace: &PinnedNamespace,
    database_path: &Path,
) -> Result<(), GlobalSchemaV1Error> {
    for suffix in ["-wal", "-shm", "-journal"] {
        let leaf = sidecar_leaf(&namespace.database_leaf, suffix);
        match openat_component(&namespace.database_parent.file, &leaf, O_RDONLY_FLAG, false) {
            Err(source) if source.kind() == io::ErrorKind::NotFound => {}
            Ok(_) => {
                return Err(GlobalSchemaV1Error::ObjectIdentityChanged {
                    path: sidecar_path(database_path, suffix),
                });
            }
            Err(source) => {
                return Err(GlobalSchemaV1Error::Io {
                    operation: "inspect selection SQLite sidecar",
                    path: sidecar_path(database_path, suffix),
                    source,
                });
            }
        }
    }
    Ok(())
}

fn capture_selection_pragmas(
    connection: &Connection,
) -> Result<SelectionPragmaSnapshot, GlobalSchemaV1Error> {
    Ok(SelectionPragmaSnapshot {
        application_id: connection
            .pragma_query_value(None, "application_id", |row| row.get(0))
            .map_err(|source| GlobalSchemaV1Error::SelectionSqlite {
                operation: "capture PRAGMA application_id",
                source,
            })?,
        user_version: connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(|source| GlobalSchemaV1Error::SelectionSqlite {
                operation: "capture PRAGMA user_version",
                source,
            })?,
        foreign_keys: connection
            .pragma_query_value(None, "foreign_keys", |row| row.get(0))
            .map_err(|source| GlobalSchemaV1Error::SelectionSqlite {
                operation: "capture PRAGMA foreign_keys",
                source,
            })?,
        journal_mode: connection
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .map_err(|source| GlobalSchemaV1Error::SelectionSqlite {
                operation: "capture PRAGMA journal_mode",
                source,
            })?,
        synchronous: connection
            .pragma_query_value(None, "synchronous", |row| row.get(0))
            .map_err(|source| GlobalSchemaV1Error::SelectionSqlite {
                operation: "capture PRAGMA synchronous",
                source,
            })?,
    })
}

fn capture_selection_integrity(
    connection: &Connection,
) -> Result<SelectionIntegritySnapshot, GlobalSchemaV1Error> {
    let mut statement = connection
        .prepare("PRAGMA integrity_check")
        .map_err(|source| GlobalSchemaV1Error::SelectionSqlite {
            operation: "prepare PRAGMA integrity_check",
            source,
        })?;
    let integrity_rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|source| GlobalSchemaV1Error::SelectionSqlite {
            operation: "query PRAGMA integrity_check",
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| GlobalSchemaV1Error::SelectionSqlite {
            operation: "read PRAGMA integrity_check",
            source,
        })?;
    if integrity_rows != ["ok"] {
        return Err(GlobalSchemaV1Error::SelectionSnapshotChanged {
            detail: format!("PRAGMA integrity_check failed: {integrity_rows:?}"),
        });
    }
    let mut foreign_key_statement =
        connection
            .prepare("PRAGMA foreign_key_check")
            .map_err(|source| GlobalSchemaV1Error::SelectionSqlite {
                operation: "prepare PRAGMA foreign_key_check",
                source,
            })?;
    let mut foreign_key_rows =
        foreign_key_statement
            .query([])
            .map_err(|source| GlobalSchemaV1Error::SelectionSqlite {
                operation: "query PRAGMA foreign_key_check",
                source,
            })?;
    let mut foreign_key_violations = 0_i64;
    while foreign_key_rows
        .next()
        .map_err(|source| GlobalSchemaV1Error::SelectionSqlite {
            operation: "read PRAGMA foreign_key_check",
            source,
        })?
        .is_some()
    {
        foreign_key_violations = foreign_key_violations.checked_add(1).ok_or_else(|| {
            GlobalSchemaV1Error::SelectionSnapshotChanged {
                detail: "PRAGMA foreign_key_check violation count overflowed i64".to_owned(),
            }
        })?;
    }
    if foreign_key_violations != 0 {
        return Err(GlobalSchemaV1Error::SelectionSnapshotChanged {
            detail: format!("PRAGMA foreign_key_check found {foreign_key_violations} violation(s)"),
        });
    }
    Ok(SelectionIntegritySnapshot {
        integrity_rows,
        foreign_key_violations,
    })
}

fn require_same_file_identity(
    parent: &PinnedDirectory,
    leaf: &OsStr,
    path: &Path,
    file: &File,
    expected: FileIdentity,
    operation: &'static str,
) -> Result<(), GlobalSchemaV1Error> {
    parent.validate_unchanged()?;
    let reopened =
        openat_component(&parent.file, leaf, O_RDONLY_FLAG, false).map_err(|source| {
            GlobalSchemaV1Error::Io {
                operation,
                path: path.to_path_buf(),
                source,
            }
        })?;
    let reopened_metadata = reopened
        .metadata()
        .map_err(|source| GlobalSchemaV1Error::Io {
            operation,
            path: path.to_path_buf(),
            source,
        })?;
    let file_metadata = file.metadata().map_err(|source| GlobalSchemaV1Error::Io {
        operation,
        path: path.to_path_buf(),
        source,
    })?;
    if FileIdentity::from_metadata(&reopened_metadata) != expected
        || FileIdentity::from_metadata(&file_metadata) != expected
    {
        return Err(GlobalSchemaV1Error::ObjectIdentityChanged {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

fn read_identity_from_pinned_database(
    file: &File,
    path: &Path,
) -> Result<GlobalSchemaIdentity, GlobalSchemaV1Error> {
    let mut file = file.try_clone().map_err(|source| GlobalSchemaV1Error::Io {
        operation: "clone pinned database descriptor",
        path: path.to_path_buf(),
        source,
    })?;
    file.seek(SeekFrom::Start(0))
        .map_err(|source| GlobalSchemaV1Error::Io {
            operation: "seek pinned database header",
            path: path.to_path_buf(),
            source,
        })?;
    let mut header = [0_u8; 100];
    file.read_exact(&mut header)
        .map_err(|source| GlobalSchemaV1Error::InvalidSqliteHeader {
            path: path.to_path_buf(),
            detail: format!("cannot read complete 100-byte header: {source}"),
        })?;
    if &header[..16] != b"SQLite format 3\0" {
        return Err(GlobalSchemaV1Error::InvalidSqliteHeader {
            path: path.to_path_buf(),
            detail: "SQLite format-3 magic mismatch".to_owned(),
        });
    }
    let user_version = i64::from(i32::from_be_bytes(
        header[60..64]
            .try_into()
            .expect("fixed SQLite header user_version range"),
    ));
    let application_id = i64::from(i32::from_be_bytes(
        header[68..72]
            .try_into()
            .expect("fixed SQLite header application_id range"),
    ));
    Ok(GlobalSchemaIdentity {
        application_id,
        user_version,
    })
}

fn sidecar_leaf(database_leaf: &OsStr, suffix: &str) -> OsString {
    let mut value = OsString::from(database_leaf);
    value.push(suffix);
    value
}

fn require_no_live_sidecars(
    paths: &ModeBoundPaths,
    namespace: &PinnedNamespace,
) -> Result<(), GlobalSchemaV1Error> {
    let wal_leaf = sidecar_leaf(&namespace.database_leaf, "-wal");
    let shm_leaf = sidecar_leaf(&namespace.database_leaf, "-shm");
    let wal =
        open_optional_pinned_regular(&namespace.database_parent, &wal_leaf, &paths.wal, paths)?;
    let shm =
        open_optional_pinned_regular(&namespace.database_parent, &shm_leaf, &paths.shm, paths)?;
    match (wal, shm) {
        (None, None) => Ok(()),
        (Some((wal_file, wal_identity)), Some((shm_file, shm_identity))) => {
            require_same_file_identity(
                &namespace.database_parent,
                &wal_leaf,
                &paths.wal,
                &wal_file,
                wal_identity,
                "revalidate WAL sidecar",
            )?;
            require_same_file_identity(
                &namespace.database_parent,
                &shm_leaf,
                &paths.shm,
                &shm_file,
                shm_identity,
                "revalidate SHM sidecar",
            )?;
            Err(GlobalSchemaV1Error::WalBackedInspectionUnavailable {
                wal: paths.wal.clone(),
                shm: paths.shm.clone(),
            })
        }
        (wal, shm) => {
            if let Some((file, identity)) = wal {
                require_same_file_identity(
                    &namespace.database_parent,
                    &wal_leaf,
                    &paths.wal,
                    &file,
                    identity,
                    "revalidate incomplete WAL sidecar",
                )?;
            }
            if let Some((file, identity)) = shm {
                require_same_file_identity(
                    &namespace.database_parent,
                    &shm_leaf,
                    &paths.shm,
                    &file,
                    identity,
                    "revalidate incomplete SHM sidecar",
                )?;
            }
            Err(GlobalSchemaV1Error::IncompleteSidecarSet {
                wal_exists: openat_component(
                    &namespace.database_parent.file,
                    &wal_leaf,
                    O_RDONLY_FLAG,
                    false,
                )
                .is_ok(),
                shm_exists: openat_component(
                    &namespace.database_parent.file,
                    &shm_leaf,
                    O_RDONLY_FLAG,
                    false,
                )
                .is_ok(),
            })
        }
    }
}

fn require_sidecars_absent(
    paths: &ModeBoundPaths,
    namespace: &PinnedNamespace,
) -> Result<(), GlobalSchemaV1Error> {
    for (sidecar, leaf) in [
        (&paths.wal, sidecar_leaf(&namespace.database_leaf, "-wal")),
        (&paths.shm, sidecar_leaf(&namespace.database_leaf, "-shm")),
    ] {
        match openat_component(&namespace.database_parent.file, &leaf, O_RDONLY_FLAG, false) {
            Err(source) if source.kind() == io::ErrorKind::NotFound => {}
            Ok(_) => {
                return Err(GlobalSchemaV1Error::ObjectIdentityChanged {
                    path: sidecar.clone(),
                });
            }
            Err(source) => {
                return Err(GlobalSchemaV1Error::Io {
                    operation: "revalidate absent SQLite sidecar",
                    path: sidecar.clone(),
                    source,
                });
            }
        }
    }
    Ok(())
}

fn open_optional_pinned_regular(
    parent: &PinnedDirectory,
    leaf: &OsStr,
    path: &Path,
    paths: &ModeBoundPaths,
) -> Result<Option<(File, FileIdentity)>, GlobalSchemaV1Error> {
    match openat_component(&parent.file, leaf, O_RDONLY_FLAG, false) {
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) if source.raw_os_error() == Some(ELOOP_CODE) => {
            Err(GlobalSchemaV1Error::UnsafeFixedPath {
                detail: format!("SQLite sidecar is a symlink: {}", path.display()),
            })
        }
        Err(source) => Err(GlobalSchemaV1Error::Io {
            operation: "descriptor-open optional SQLite sidecar",
            path: path.to_path_buf(),
            source,
        }),
        Ok(file) => {
            let metadata = file.metadata().map_err(|source| GlobalSchemaV1Error::Io {
                operation: "fstat optional SQLite sidecar",
                path: path.to_path_buf(),
                source,
            })?;
            if !metadata.is_file() {
                return Err(GlobalSchemaV1Error::DatabaseNotRegular {
                    path: path.to_path_buf(),
                });
            }
            require_test_single_link(paths, path, &file)?;
            let identity = FileIdentity::from_metadata(&metadata);
            require_same_file_identity(
                parent,
                leaf,
                path,
                &file,
                identity,
                "optional SQLite sidecar",
            )?;
            Ok(Some((file, identity)))
        }
    }
}

fn require_test_single_link(
    paths: &ModeBoundPaths,
    path: &Path,
    file: &File,
) -> Result<(), GlobalSchemaV1Error> {
    if paths.mode == BoundMode::Production {
        return Ok(());
    }
    let links = file
        .metadata()
        .map_err(|source| GlobalSchemaV1Error::Io {
            operation: "fstat TEST_CODE object link count",
            path: path.to_path_buf(),
            source,
        })?
        .nlink();
    if links != 1 {
        return Err(GlobalSchemaV1Error::ModeBindingViolation {
            detail: format!(
                "TEST_CODE object must have exactly one physical link; path={} nlink={links}",
                path.display()
            ),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::global_schema_catalog_v1::{
        install_exact_selection_catalog_for_test, DatabaseHalfDiagnostic,
    };
    use crate::database::DatabaseManager;
    use crate::selection::audit::{
        SelectionAuditPhase, SelectionAuditRecord, SelectionAuditWriter,
    };
    use rusqlite::Connection;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);
    const CHILD_LOCK_PATH_ENV: &str = "TEST_CODE_GLOBAL_SCHEMA_CHILD_LOCK_PATH";

    fn create_fifo(path: &Path) {
        let path = CString::new(path.as_os_str().as_bytes()).expect("FIFO path contains no NUL");
        // SAFETY: `path` is a live NUL-terminated absolute test path.
        let result = unsafe { mkfifo(path.as_ptr(), 0o600_u32) };
        assert_eq!(
            result,
            0,
            "create test FIFO failed: {}",
            io::Error::last_os_error()
        );
    }

    fn assert_close_on_exec(file: &File, label: &str) {
        const F_GETFD: i32 = 1;
        const FD_CLOEXEC: i32 = 1;
        // SAFETY: `file` retains a valid descriptor and `F_GETFD` takes no
        // variadic argument.
        let flags = unsafe { fcntl(file.as_raw_fd(), F_GETFD) };
        assert!(
            flags >= 0,
            "read {label} descriptor flags failed: {}",
            io::Error::last_os_error()
        );
        assert_ne!(
            flags & FD_CLOEXEC,
            0,
            "{label} descriptor must be close-on-exec"
        );
    }

    struct TestFixture {
        root: PathBuf,
    }

    impl TestFixture {
        fn new(label: &str, application_id: i64, user_version: i64) -> Self {
            let test_parent =
                fs::canonicalize(std::env::temp_dir()).expect("canonicalize test temp parent");
            let root = test_parent.join(format!(
                "TEST_CODE_global-schema-{label}-{}-{}",
                std::process::id(),
                TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&root).expect("create isolated TEST_CODE root");
            let database = root.join("stock_analysis.db");
            let connection = Connection::open(&database).expect("create test database");
            connection
                .pragma_update(None, "application_id", application_id)
                .expect("seed application_id");
            connection
                .pragma_update(None, "user_version", user_version)
                .expect("seed user_version");
            drop(connection);
            Self { root }
        }

        fn binding(&self) -> ModeBoundPaths {
            ModeBoundPaths::isolated_test(&self.root).expect("bind isolated test paths")
        }

        fn database(&self) -> PathBuf {
            self.root.join("stock_analysis.db")
        }

        fn lock_file(&self) -> PathBuf {
            self.root.join("locks").join(GLOBAL_MAINTENANCE_LOCK_FILE)
        }

        fn inspect(&self) -> Result<VerifiedGlobalSchemaV1, GlobalSchemaV1Error> {
            inspect_bound_database(self.binding())
        }

        fn acquire_exclusive(
            &self,
        ) -> Result<ExclusiveGlobalSchemaMaintenanceLease, GlobalSchemaV1Error> {
            acquire_exclusive_bound(self.binding())
        }

        fn pinned_audit_writer(&self) -> SelectionAuditWriter {
            let root_descriptor =
                File::open(&self.root).expect("pin isolated TEST_CODE fixture root");
            SelectionAuditWriter::for_test_code_pinned_root(&root_descriptor, &self.root)
                .expect("bind TEST_CODE audit writer to retained fixture root")
        }

        fn enable_wal_without_selection_catalog(&self) {
            let connection = Connection::open(self.database()).expect("open TEST_CODE database");
            let journal_mode: String = connection
                .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
                .expect("enable WAL for absent selection database half");
            assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
            connection
                .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
                .expect("checkpoint absent selection database-half WAL");
            drop(connection);
            for sidecar in [
                self.database().with_extension("db-wal"),
                self.database().with_extension("db-shm"),
            ] {
                match fs::remove_file(&sidecar) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => panic!(
                        "remove closed absent-half TEST_CODE sidecar {}: {error}",
                        sidecar.display()
                    ),
                }
            }
        }

        fn install_final_selection_catalog(&self) {
            let connection = Connection::open(self.database()).expect("open TEST_CODE database");
            let journal_mode: String = connection
                .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
                .expect("enable WAL for descriptor-attested TEST_CODE pool");
            assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
            install_exact_selection_catalog_for_test(
                &connection,
                crate::database::global_schema_catalog_v1::GlobalSchemaCatalogMode::Test,
                true,
            )
            .expect("install exact final TEST_CODE catalog");
            connection
                .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
                .expect("checkpoint TEST_CODE bootstrap WAL");
            drop(connection);
            for sidecar in [
                self.database().with_extension("db-wal"),
                self.database().with_extension("db-shm"),
            ] {
                match fs::remove_file(&sidecar) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => panic!(
                        "remove closed TEST_CODE bootstrap sidecar {}: {error}",
                        sidecar.display()
                    ),
                }
            }
            assert!(
                !self.database().with_extension("db-wal").exists(),
                "closed TEST_CODE bootstrap must not leave a live WAL sidecar"
            );
            assert!(
                !self.database().with_extension("db-shm").exists(),
                "closed TEST_CODE bootstrap must not leave a live SHM sidecar"
            );
        }
    }

    impl Drop for TestFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn install_attribution_activation_fixture(manager: &DatabaseManager) {
        use crate::database::order_audit::{
            canonical_order_audit_record_hash, CanonicalOrderAuditRow, AUDIT_CHAIN_GENESIS,
        };
        use diesel::sql_types::{BigInt, Double, Nullable, Text};
        use diesel::RunQueryDsl;

        let mut connection = manager
            .get_conn()
            .expect("TEST_CODE authority-owned fixture connection");
        crate::database::attribution_epochs::create_schema(&mut connection)
            .expect("TEST_CODE install attribution epoch schema");

        let audit = CanonicalOrderAuditRow {
            id: 1,
            business_order_id: "TEST_CODE_AUTHORITY_ACTIVATION_BUY".into(),
            source: "PaperTrade".into(),
            decision_basis: "TEST_CODE authority-owned activation".into(),
            side: "buy".into(),
            code: "TEST_CODE_600001".into(),
            requested_price: 10.0,
            execution_price: Some(10.0),
            quantity: 200,
            quote_observed_at: Some("2026-08-27T10:00:00+08:00".into()),
            outcome: "Filled".into(),
            failure_reason: None,
            created_at: "2026-08-27 02:00:01".into(),
        };
        let record_hash = canonical_order_audit_record_hash(AUDIT_CHAIN_GENESIS, &audit)
            .expect("TEST_CODE canonical audit hash");
        diesel::sql_query(
            "INSERT INTO paper_trades
                 (id,plan_id,code,name,direction,price,quantity,status,fill_price,not_fill_reason,
                  virtual_reason,account_mode,data_mode,ts,updated_at)
                 VALUES (1,?,'TEST_CODE_600001','TEST_CODE company','buy',10.0,200,'Filled',10.0,NULL,
                         ?,'Normal','Full','2026-08-27 02:00:00','2026-08-27 02:00:00')",
        )
        .bind::<Text, _>(&audit.business_order_id)
        .bind::<Text, _>(&audit.decision_basis)
        .execute(&mut connection)
        .expect("TEST_CODE insert activation paper fill");
        diesel::sql_query(
            "INSERT INTO order_audit
                 (id,business_order_id,source,decision_basis,side,code,requested_price,
                  execution_price,quantity,quote_observed_at,outcome,failure_reason,created_at)
                 VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind::<BigInt, _>(audit.id)
        .bind::<Text, _>(&audit.business_order_id)
        .bind::<Text, _>(&audit.source)
        .bind::<Text, _>(&audit.decision_basis)
        .bind::<Text, _>(&audit.side)
        .bind::<Text, _>(&audit.code)
        .bind::<Double, _>(audit.requested_price)
        .bind::<Nullable<Double>, _>(audit.execution_price)
        .bind::<BigInt, _>(audit.quantity)
        .bind::<Nullable<Text>, _>(&audit.quote_observed_at)
        .bind::<Text, _>(&audit.outcome)
        .bind::<Nullable<Text>, _>(&audit.failure_reason)
        .bind::<Text, _>(&audit.created_at)
        .execute(&mut connection)
        .expect("TEST_CODE insert activation order audit");
        diesel::sql_query(
            "INSERT INTO order_audit_chain
                 (order_audit_id,previous_hash,record_hash,created_at) VALUES (1,?,?,?)",
        )
        .bind::<Text, _>(AUDIT_CHAIN_GENESIS)
        .bind::<Text, _>(&record_hash)
        .bind::<Text, _>(&audit.created_at)
        .execute(&mut connection)
        .expect("TEST_CODE insert activation audit chain");
    }

    #[test]
    fn identity_matrix_accepts_only_exact_stsa_generation_one() {
        let identity = classify_identity(
            STOCK_ANALYSIS_SQLITE_APPLICATION_ID,
            STOCK_ANALYSIS_DB_SCHEMA_GENERATION,
        )
        .expect("exact STSA/1 must be legal");
        assert_eq!(
            identity,
            GlobalSchemaIdentity {
                application_id: 1_398_035_265,
                user_version: 1,
            }
        );

        assert!(matches!(
            classify_identity(0, 0),
            Err(GlobalSchemaV1Error::OfflineGlobalMigrationRequired {
                application_id: 0,
                user_version: 0
            })
        ));
        for (application_id, user_version) in [
            (0, 1),
            (STOCK_ANALYSIS_SQLITE_APPLICATION_ID, 0),
            (1, 1),
            (-1, 1),
            (STOCK_ANALYSIS_SQLITE_APPLICATION_ID, -1),
        ] {
            assert!(
                matches!(
                    classify_identity(application_id, user_version),
                    Err(GlobalSchemaV1Error::UnsupportedIdentity { .. })
                ),
                "matrix {application_id}/{user_version} must fail closed"
            );
        }
        assert!(matches!(
            classify_identity(STOCK_ANALYSIS_SQLITE_APPLICATION_ID, 2),
            Err(GlobalSchemaV1Error::UnsupportedFutureGeneration {
                actual: 2,
                supported: 1
            })
        ));
    }

    #[test]
    fn owner_rejects_final_catalog_when_v2_audit_has_no_exact_database_receipt_closure() {
        let fixture = TestFixture::new(
            "selection-with-audit",
            STOCK_ANALYSIS_SQLITE_APPLICATION_ID,
            STOCK_ANALYSIS_DB_SCHEMA_GENERATION,
        );
        fixture.install_final_selection_catalog();
        let writer = fixture.pinned_audit_writer();
        writer
            .append(SelectionAuditRecord::new(
                SelectionAuditPhase::V2ConfigActivationCommitted,
                "TEST_CODE_CONFIG_ACTIVATION",
                "a".repeat(64),
                chrono::DateTime::parse_from_rfc3339("2026-07-29T00:00:00+08:00")
                    .expect("fixed timestamp"),
            ))
            .expect("append validated TEST_CODE audit record");

        let owner = GlobalSchemaVersionOwner::for_test_code();
        let error = owner
            .inspect_selection_with_audit_for_test(&fixture.root, &writer)
            .expect_err("v2 audit evidence without matching database receipts must fail closed");

        assert!(matches!(
            error,
            GlobalSchemaV1Error::SelectionReceiptReconciliation { .. }
        ));
    }

    #[test]
    fn owner_issues_amended_capability_only_after_same_snapshot_exact_reconciliation() {
        let fixture = TestFixture::new(
            "selection-amended-capability",
            STOCK_ANALYSIS_SQLITE_APPLICATION_ID,
            STOCK_ANALYSIS_DB_SCHEMA_GENERATION,
        );
        fixture.install_final_selection_catalog();
        let writer =
            SelectionAuditWriter::for_test_code_root(&fixture.root).expect("TEST_CODE audit");
        writer
            .append(SelectionAuditRecord::new(
                SelectionAuditPhase::V2GateDCanaryVerified,
                "TEST_CODE_GATE_D",
                "c".repeat(64),
                chrono::DateTime::parse_from_rfc3339("2026-07-29T00:02:00+08:00")
                    .expect("fixed timestamp"),
            ))
            .expect("append validated non-persistence V2 audit record");

        let outcome = GlobalSchemaVersionOwner::for_test_code()
            .inspect_selection_with_audit_for_test(&fixture.root, &writer)
            .expect("empty exact final database has vacuous receipt closure");
        assert!(matches!(
            &outcome,
            SelectionSchemaInspectionOutcome::Amended(_)
        ));
        assert_eq!(
            outcome.authority_state(),
            SelectionSchemaAuthorityDiagnostic::Amended
        );
        for suffix in ["-wal", "-shm", "-journal"] {
            assert!(
                !sidecar_path(&fixture.database(), suffix).exists(),
                "owner-created inspection sidecar must be gone before capability issuance: {suffix}"
            );
        }
        assert!(matches!(
            fixture.acquire_exclusive(),
            Err(GlobalSchemaV1Error::ExclusiveProcessMaintenanceLeaseUnavailable)
        ));

        let authority = match outcome {
            SelectionSchemaInspectionOutcome::Amended(authority) => authority,
            SelectionSchemaInspectionOutcome::Diagnostic(_) => {
                panic!("exact amended snapshot must issue owner capability")
            }
        };
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let database_manager =
                DatabaseManager::from_verified_amended_selection_schema(authority)
                    .expect("bind pool to owner-pinned database descriptor");
            assert!(database_manager.retains_verified_selection_authority());
            assert!(database_manager.get_conn().is_ok());
            assert!(matches!(
                fixture.acquire_exclusive(),
                Err(GlobalSchemaV1Error::ExclusiveProcessMaintenanceLeaseUnavailable)
            ));

            drop(database_manager);
            drop(
                fixture
                    .acquire_exclusive()
                    .expect("pool drops before capability releases exclusive owner authority"),
            );
        }

        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let error = match DatabaseManager::from_verified_amended_selection_schema(authority) {
                Ok(_) => panic!("unproven descriptor-relative WAL routing must fail closed"),
                Err(error) => error,
            };
            assert!(error
                .to_string()
                .contains("descriptor_attestation_unavailable"));
            drop(
                fixture
                    .acquire_exclusive()
                    .expect("failed operational bind releases owner authority"),
            );
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn amended_schema_manager_completes_attribution_activation_read_back() {
        let fixture = TestFixture::new(
            "selection-amended-attribution-read-back",
            STOCK_ANALYSIS_SQLITE_APPLICATION_ID,
            STOCK_ANALYSIS_DB_SCHEMA_GENERATION,
        );
        fixture.install_final_selection_catalog();
        let writer =
            SelectionAuditWriter::for_test_code_root(&fixture.root).expect("TEST_CODE audit");
        writer
            .append(SelectionAuditRecord::new(
                SelectionAuditPhase::V2GateDCanaryVerified,
                "TEST_CODE_GATE_D",
                "d".repeat(64),
                chrono::DateTime::parse_from_rfc3339("2026-07-29T00:02:00+08:00")
                    .expect("fixed timestamp"),
            ))
            .expect("append validated non-persistence V2 audit record");
        let outcome = GlobalSchemaVersionOwner::for_test_code()
            .inspect_selection_with_audit_for_test(&fixture.root, &writer)
            .expect("TEST_CODE exact amended schema authority");
        let authority = match outcome {
            SelectionSchemaInspectionOutcome::Amended(authority) => authority,
            SelectionSchemaInspectionOutcome::Diagnostic(_) => {
                panic!("TEST_CODE exact amended snapshot must issue owner authority")
            }
        };
        let manager = DatabaseManager::from_verified_amended_selection_schema(authority)
            .expect("TEST_CODE construct authority-owned production manager");
        // Post-construction fixture only: this isolates whether the production
        // constructor itself retained the read-back capability. It does not
        // change or make a claim about the exact GlobalSchema catalog contract.
        install_attribution_activation_fixture(&manager);
        let store = crate::database::attribution_epochs::AttributionEpochStore::new(&manager);
        let receipt = store
            .activate_once(
                crate::database::attribution_epochs::EpochActivationRequest {
                    source: crate::performance::attribution_epoch::EpochActivationSource::Monitor,
                    invoked_at: chrono::DateTime::parse_from_rfc3339("2026-08-28T15:40:00+08:00")
                        .expect("TEST_CODE fixed activation time"),
                },
            )
            .expect("TEST_CODE authority-owned activation and read-back succeed");
        assert_eq!(
            store
                .verify_active()
                .expect("TEST_CODE authority-owned active receipt"),
            receipt
        );
    }

    #[test]
    fn owner_rejects_and_preserves_unknown_preexisting_sqlite_sidecars() {
        for suffix in ["-wal", "-shm", "-journal"] {
            let fixture = TestFixture::new(
                &format!("selection-preexisting-sidecar-{}", &suffix[1..]),
                STOCK_ANALYSIS_SQLITE_APPLICATION_ID,
                STOCK_ANALYSIS_DB_SCHEMA_GENERATION,
            );
            fixture.install_final_selection_catalog();
            let sidecar = sidecar_path(&fixture.database(), suffix);
            File::create(&sidecar).expect("create unknown preexisting TEST_CODE sidecar");
            let writer = fixture.pinned_audit_writer();

            let error = GlobalSchemaVersionOwner::for_test_code()
                .inspect_selection_with_audit_for_test(&fixture.root, &writer)
                .expect_err("unknown preexisting sidecar must block before authority snapshot");
            assert!(matches!(
                error,
                GlobalSchemaV1Error::ObjectIdentityChanged { .. }
            ));
            assert!(
                sidecar.exists(),
                "owner must not auto-clean unknown preexisting sidecar: {suffix}"
            );
        }
    }

    #[test]
    fn missing_audit_returns_database_half_only_and_never_authoritative_absent() {
        let fixture = TestFixture::new("selection-audit-missing", 0, 0);
        fixture.enable_wal_without_selection_catalog();
        let writer = fixture.pinned_audit_writer();
        assert!(!writer.path().exists(), "audit evidence must start absent");

        let diagnostic = GlobalSchemaVersionOwner::for_test_code()
            .inspect_selection_with_audit_for_test(&fixture.root, &writer)
            .expect("missing audit is a diagnostic database half");

        assert!(matches!(
            diagnostic.database_half(),
            DatabaseHalfDiagnostic::AbsentDatabaseHalf(_)
        ));
        assert_eq!(
            diagnostic.authority_state(),
            SelectionSchemaAuthorityDiagnostic::DatabaseHalfOnly
        );
        assert!(
            !writer.path().exists(),
            "read-only inspection must not create a missing audit object"
        );
    }

    #[test]
    fn v2_audit_with_absent_database_half_fails_closed_as_contradictory() {
        let fixture = TestFixture::new("selection-audit-v2-db-absent", 0, 0);
        fixture.enable_wal_without_selection_catalog();
        let writer = fixture.pinned_audit_writer();
        writer
            .append(SelectionAuditRecord::new(
                SelectionAuditPhase::V2IngressCommitted,
                "TEST_CODE_INGRESS",
                "b".repeat(64),
                chrono::DateTime::parse_from_rfc3339("2026-07-29T00:01:00+08:00")
                    .expect("fixed timestamp"),
            ))
            .expect("append contradictory TEST_CODE v2 audit record");

        let error = GlobalSchemaVersionOwner::for_test_code()
            .inspect_selection_with_audit_for_test(&fixture.root, &writer)
            .expect_err("v2 audit plus absent database must fail closed");
        assert!(matches!(
            error,
            GlobalSchemaV1Error::SelectionAuthorityContradiction { .. }
        ));
    }

    #[test]
    fn production_apply_is_rejected_before_owner_opens_any_database_or_audit() {
        let error = run_selection_v2_migration_command(["--apply"])
            .expect_err("production apply must fail closed before inspection");
        assert_eq!(
            error,
            super::super::selection_v2::SELECTION_V2_APPLY_BLOCKER
        );
    }

    #[test]
    fn migration_cli_help_and_argument_parser_have_no_path_override() {
        let help = run_selection_v2_migration_command(["--help"]).expect("render help");
        assert!(help.contains("owner-issued"));
        assert!(help.contains("Arbitrary database"));
        for arguments in [
            vec!["--database", "/tmp/TEST_CODE_override.db"],
            vec!["--test", "--test"],
            vec!["--help", "--apply"],
        ] {
            run_selection_v2_migration_command(arguments)
                .expect_err("unsupported, duplicate, or mixed help argument must fail");
        }
    }

    #[test]
    fn test_code_rehearsal_root_uses_unpredictable_nonce_and_explicit_cleanup() {
        let first = TestCodeSelectionRehearsal::create().expect("create first rehearsal");
        let second = TestCodeSelectionRehearsal::create().expect("create second rehearsal");
        let first_path = first.root().to_path_buf();
        let second_path = second.root().to_path_buf();
        assert_ne!(first_path, second_path);
        for path in [&first_path, &second_path] {
            let leaf = path
                .file_name()
                .and_then(OsStr::to_str)
                .expect("UTF-8 rehearsal leaf");
            let nonce = leaf
                .rsplit_once('-')
                .map(|(_, nonce)| nonce)
                .expect("rehearsal leaf contains nonce");
            assert_eq!(nonce.len(), 32);
            assert!(nonce.bytes().all(|byte| byte.is_ascii_hexdigit()));
        }
        first.finish().expect("explicitly clean first rehearsal");
        second.finish().expect("explicitly clean second rehearsal");
        assert!(!first_path.exists());
        assert!(!second_path.exists());
    }

    #[test]
    fn test_code_copy_remains_bound_to_root_descriptor_during_path_rename() {
        let rehearsal = TestCodeSelectionRehearsal::create().expect("create rehearsal");
        let original_root = rehearsal.root().to_path_buf();
        let moved_root = original_root.with_extension("moved");
        let source_path = rehearsal.parent_path.join(format!(
            "TEST_CODE_copy-source-{}",
            unpredictable_owner_nonce().expect("nonce")
        ));
        fs::write(&source_path, b"owner-pinned-bytes").expect("write copy source");
        let source = File::open(&source_path).expect("open copy source");

        fs::rename(&original_root, &moved_root).expect("rename rehearsal root");
        copy_pinned_file_to_new_descriptor(
            &source,
            &source_path,
            &rehearsal.root.file,
            OsStr::new("descriptor-copy.bin"),
            &original_root.join("descriptor-copy.bin"),
            "TEST_CODE descriptor copy test",
        )
        .expect("copy through retained root descriptor");
        let mut copied = openat_component(
            &rehearsal.root.file,
            OsStr::new("descriptor-copy.bin"),
            O_RDONLY_FLAG,
            false,
        )
        .expect("open copied file through retained root");
        let mut bytes = Vec::new();
        copied.read_to_end(&mut bytes).expect("read copied bytes");
        assert_eq!(bytes, b"owner-pinned-bytes");

        fs::rename(&moved_root, &original_root).expect("restore rehearsal root");
        fs::remove_file(source_path).expect("remove copy source");
        rehearsal.finish().expect("explicit rehearsal cleanup");
    }

    #[test]
    fn test_code_cleanup_failure_is_explicit_and_does_not_delete_replacement() {
        let rehearsal = TestCodeSelectionRehearsal::create().expect("create rehearsal");
        let original_root = rehearsal.root().to_path_buf();
        let moved_root = original_root.with_extension("owner-moved");
        fs::rename(&original_root, &moved_root).expect("move owner root");
        fs::create_dir(&original_root).expect("install replacement root");
        fs::write(original_root.join("must-survive"), b"replacement")
            .expect("write replacement marker");

        let error = rehearsal
            .finish()
            .expect_err("identity-changing cleanup must fail explicitly");
        assert!(
            error.to_string().contains("identity"),
            "unexpected cleanup error: {error}"
        );
        assert_eq!(
            fs::read(original_root.join("must-survive")).expect("replacement survives"),
            b"replacement"
        );

        fs::remove_dir_all(&original_root).expect("remove replacement");
        fs::remove_dir_all(&moved_root).expect("remove moved owner root");
    }

    #[test]
    fn exact_test_database_is_inspected_read_only_and_lease_lives_with_capability() {
        let fixture = TestFixture::new(
            "exact-read-only",
            STOCK_ANALYSIS_SQLITE_APPLICATION_ID,
            STOCK_ANALYSIS_DB_SCHEMA_GENERATION,
        );
        let before = fs::read(fixture.database()).expect("read database before inspection");
        let lease_count_before = PROCESS_SHARED_LEASES.load(Ordering::Acquire);

        let verified = fixture.inspect().expect("inspect exact isolated test DB");
        assert_eq!(
            verified.identity(),
            GlobalSchemaIdentity {
                application_id: STOCK_ANALYSIS_SQLITE_APPLICATION_ID,
                user_version: STOCK_ANALYSIS_DB_SCHEMA_GENERATION,
            }
        );
        assert_eq!(verified.identity().application_id(), 1_398_035_265);
        assert_eq!(verified.identity().user_version(), 1);
        assert_eq!(
            PROCESS_SHARED_LEASES.load(Ordering::Acquire),
            lease_count_before + 1
        );
        assert_eq!(
            fs::read(fixture.database()).expect("read database while capability lives"),
            before
        );
        assert!(!fixture.database().with_extension("db-wal").exists());
        assert!(!fixture.database().with_extension("db-shm").exists());

        let contender = OpenOptions::new()
            .read(true)
            .write(true)
            .open(fixture.lock_file())
            .expect("open lock contender");
        assert!(
            FileExt::try_lock_exclusive(&contender).is_err(),
            "lifetime shared lease must block an exclusive contender"
        );
        drop(verified);
        assert_eq!(
            PROCESS_SHARED_LEASES.load(Ordering::Acquire),
            lease_count_before
        );
        FileExt::try_lock_exclusive(&contender)
            .expect("exclusive contender succeeds after capability drop");
        FileExt::unlock(&contender).expect("unlock contender");
        assert_eq!(
            fs::read(fixture.database()).expect("read database after inspection"),
            before
        );
    }

    #[test]
    fn verified_namespace_and_lease_descriptors_are_close_on_exec() {
        let fixture = TestFixture::new(
            "close-on-exec",
            STOCK_ANALYSIS_SQLITE_APPLICATION_ID,
            STOCK_ANALYSIS_DB_SCHEMA_GENERATION,
        );
        let verified = fixture.inspect().expect("inspect exact TEST_CODE database");
        assert_close_on_exec(&verified._database_file, "database");
        assert_close_on_exec(&verified._namespace.root.file, "namespace root");
        assert_close_on_exec(&verified._namespace.database_parent.file, "database parent");
        assert_close_on_exec(&verified._namespace.lock_parent.file, "lock parent");
        assert_close_on_exec(&verified._lease.lock_file, "maintenance lock");

        let mut child = Command::new("/bin/sh")
            .args(["-c", "read _ || exit 0"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn exec child while verified capability lives");
        drop(verified);

        let contender = OpenOptions::new()
            .read(true)
            .write(true)
            .open(fixture.lock_file())
            .expect("open exclusive contender");
        FileExt::try_lock_exclusive(&contender)
            .expect("exec child must not inherit the shared maintenance lease");
        FileExt::unlock(&contender).expect("unlock exclusive contender");

        drop(child.stdin.take());
        let output = child.wait_with_output().expect("wait for exec child");
        assert!(
            output.status.success(),
            "exec child failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn same_process_shared_to_exclusive_upgrade_is_typed_and_forbidden() {
        let fixture = TestFixture::new(
            "forbid-upgrade",
            STOCK_ANALYSIS_SQLITE_APPLICATION_ID,
            STOCK_ANALYSIS_DB_SCHEMA_GENERATION,
        );
        let shared = fixture.inspect().expect("acquire shared lifetime lease");
        let error = fixture
            .acquire_exclusive()
            .expect_err("shared-to-exclusive upgrade must be forbidden");
        assert_eq!(
            error.code(),
            "global_schema_shared_to_exclusive_upgrade_forbidden"
        );
        drop(shared);

        let exclusive = fixture
            .acquire_exclusive()
            .expect("exclusive succeeds after shared capability drops");
        let error = fixture
            .acquire_exclusive()
            .expect_err("a second in-process exclusive authority must fail");
        assert_eq!(error.code(), "global_schema_exclusive_process_lease_busy");
        let error = fixture
            .inspect()
            .expect_err("shared acquisition must fail while exclusive lives");
        assert_eq!(error.code(), "global_schema_process_lease_busy");
        drop(exclusive);
        drop(
            fixture
                .inspect()
                .expect("shared succeeds after exclusive capability drops"),
        );
    }

    #[test]
    fn cross_process_shared_holder_makes_exclusive_retryable() {
        let fixture = TestFixture::new(
            "cross-process-shared",
            STOCK_ANALYSIS_SQLITE_APPLICATION_ID,
            STOCK_ANALYSIS_DB_SCHEMA_GENERATION,
        );
        drop(fixture.inspect().expect("create and release fixed lock"));

        let mut child = Command::new(std::env::current_exe().expect("current test executable"))
            .args([
                "--ignored",
                "--exact",
                "database::global_schema_v1::tests::TEST_CODE_global_schema_shared_child",
                "--nocapture",
            ])
            .env(CHILD_LOCK_PATH_ENV, fixture.lock_file())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn shared-lock child");
        let stdout = child.stdout.take().expect("shared child stdout");
        let mut stdout = BufReader::new(stdout);
        let mut ready = String::new();
        for _ in 0..20 {
            let mut line = String::new();
            let read = stdout.read_line(&mut line).expect("read child ready line");
            if read == 0 {
                break;
            }
            ready.push_str(&line);
            if line.contains("TEST_CODE_GLOBAL_SCHEMA_SHARED_LOCKED") {
                break;
            }
        }
        assert!(
            ready.contains("TEST_CODE_GLOBAL_SCHEMA_SHARED_LOCKED"),
            "child did not acquire shared lock: {ready:?}"
        );

        let error = fixture
            .acquire_exclusive()
            .expect_err("cross-process shared holder must block exclusive");
        assert!(matches!(
            error,
            GlobalSchemaV1Error::ExclusiveMaintenanceLeaseUnavailable {
                retryable: true,
                ..
            }
        ));

        drop(child.stdin.take());
        let output = child.wait_with_output().expect("wait for shared child");
        assert!(
            output.status.success(),
            "shared child failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        drop(
            fixture
                .acquire_exclusive()
                .expect("exclusive succeeds after shared child exits"),
        );
    }

    #[test]
    fn exclusive_descriptors_are_close_on_exec_and_release_before_process_authority() {
        let fixture = TestFixture::new(
            "exclusive-close-on-exec",
            STOCK_ANALYSIS_SQLITE_APPLICATION_ID,
            STOCK_ANALYSIS_DB_SCHEMA_GENERATION,
        );
        let exclusive = fixture
            .acquire_exclusive()
            .expect("acquire exclusive TEST_CODE authority");
        assert_close_on_exec(&exclusive.namespace.root.file, "exclusive namespace root");
        assert_close_on_exec(
            &exclusive.namespace.database_parent.file,
            "exclusive database parent",
        );
        assert_close_on_exec(
            &exclusive.namespace.lock_parent.file,
            "exclusive lock parent",
        );
        assert_close_on_exec(&exclusive.lock_file, "exclusive maintenance lock");

        let mut child = Command::new("/bin/sh")
            .args(["-c", "read _ || exit 0"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn exec child while exclusive authority lives");
        drop(exclusive);

        drop(
            fixture
                .inspect()
                .expect("exec child must not inherit exclusive maintenance authority"),
        );
        drop(child.stdin.take());
        let output = child.wait_with_output().expect("wait for exec child");
        assert!(
            output.status.success(),
            "exec child failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn exclusive_test_authority_does_not_open_or_create_the_database() {
        let fixture = TestFixture::new(
            "exclusive-missing-database",
            STOCK_ANALYSIS_SQLITE_APPLICATION_ID,
            STOCK_ANALYSIS_DB_SCHEMA_GENERATION,
        );
        fs::remove_file(fixture.database()).expect("remove fixture database");
        assert!(!fixture.database().exists());

        let exclusive = fixture
            .acquire_exclusive()
            .expect("exclusive authority only pins namespace and lock");
        assert!(
            !fixture.database().exists(),
            "exclusive acquisition must not initialize the database"
        );
        drop(exclusive);
        assert!(
            !fixture.database().exists(),
            "exclusive release must not initialize the database"
        );
    }

    #[test]
    fn database_identity_failures_are_typed_and_never_rewritten() {
        for (label, application_id, user_version, expected_code) in [
            (
                "unmanaged",
                0,
                0,
                "global_schema_offline_migration_required",
            ),
            (
                "mixed",
                STOCK_ANALYSIS_SQLITE_APPLICATION_ID,
                0,
                "global_schema_unsupported_identity",
            ),
            ("foreign", 42, 1, "global_schema_unsupported_identity"),
            (
                "future",
                STOCK_ANALYSIS_SQLITE_APPLICATION_ID,
                2,
                "global_schema_unsupported_future_generation",
            ),
        ] {
            let fixture = TestFixture::new(label, application_id, user_version);
            let before = fs::read(fixture.database()).expect("read fixture before rejection");
            let error = fixture.inspect().expect_err("identity must fail closed");
            assert_eq!(error.code(), expected_code);
            assert_eq!(
                fs::read(fixture.database()).expect("read fixture after rejection"),
                before,
                "{label} fixture was unexpectedly rewritten"
            );
        }
    }

    #[test]
    fn test_namespace_and_fixed_paths_are_exact_and_disjoint() {
        let production = ModeBoundPaths::production();
        assert_eq!(
            production.database,
            Path::new(env!("CARGO_MANIFEST_DIR")).join("data/stock_analysis.db")
        );
        assert_eq!(
            production.lock_file,
            Path::new(env!("CARGO_MANIFEST_DIR")).join("data/locks/global-schema-maintenance.lock")
        );

        let invalid = fs::canonicalize(std::env::temp_dir())
            .expect("canonicalize test temp parent")
            .join("caller-selected-test");
        let error =
            ModeBoundPaths::isolated_test(&invalid).expect_err("non TEST_CODE root must fail");
        assert_eq!(error.code(), "global_schema_mode_binding_violation");

        let fixture = TestFixture::new(
            "disjoint",
            STOCK_ANALYSIS_SQLITE_APPLICATION_ID,
            STOCK_ANALYSIS_DB_SCHEMA_GENERATION,
        );
        let test = fixture.binding();
        assert_ne!(test.database, production.database);
        assert_ne!(test.lock_file, production.lock_file);
        assert!(!test.root.starts_with(&production.root));
        assert!(!production.root.starts_with(&test.root));
    }

    #[cfg(unix)]
    #[test]
    fn database_and_lock_symlinks_are_rejected_no_follow() {
        use std::os::unix::fs::symlink;

        let fixture = TestFixture::new(
            "database-symlink",
            STOCK_ANALYSIS_SQLITE_APPLICATION_ID,
            STOCK_ANALYSIS_DB_SCHEMA_GENERATION,
        );
        let target = fixture.root.join("target.db");
        fs::rename(fixture.database(), &target).expect("move database to target");
        symlink(&target, fixture.database()).expect("create database symlink");
        let error = fixture
            .inspect()
            .expect_err("database symlink must fail no-follow");
        assert_eq!(error.code(), "global_schema_unsafe_fixed_path");

        let lock_fixture = TestFixture::new(
            "lock-symlink",
            STOCK_ANALYSIS_SQLITE_APPLICATION_ID,
            STOCK_ANALYSIS_DB_SCHEMA_GENERATION,
        );
        fs::create_dir(lock_fixture.root.join("locks")).expect("create lock directory");
        let lock_target = lock_fixture.root.join("lock-target");
        File::create(&lock_target).expect("create lock target");
        symlink(&lock_target, lock_fixture.lock_file()).expect("create lock symlink");
        let error = lock_fixture
            .inspect()
            .expect_err("lock symlink must fail no-follow");
        assert_eq!(error.code(), "global_schema_unsafe_fixed_path");
    }

    #[test]
    fn pinned_namespace_rejects_root_rename_aba() {
        let fixture = TestFixture::new(
            "root-rename-aba",
            STOCK_ANALYSIS_SQLITE_APPLICATION_ID,
            STOCK_ANALYSIS_DB_SCHEMA_GENERATION,
        );
        let paths = fixture.binding();
        let namespace = PinnedNamespace::open(&paths).expect("pin original TEST_CODE namespace");
        let moved = fixture.root.with_file_name(format!(
            "{}-moved",
            fixture
                .root
                .file_name()
                .expect("TEST_CODE namespace leaf")
                .to_string_lossy()
        ));
        fs::rename(&fixture.root, &moved).expect("rename pinned TEST_CODE root");
        fs::create_dir(&fixture.root).expect("install replacement TEST_CODE root");

        let error = namespace
            .validate_unchanged()
            .expect_err("replacement root must fail namespace validation");
        assert_eq!(error.code(), "global_schema_object_identity_changed");

        drop(namespace);
        fs::remove_dir_all(&moved).expect("remove moved TEST_CODE root");
    }

    #[test]
    fn fifo_database_and_lock_fail_without_blocking() {
        let database_fixture = TestFixture::new(
            "fifo-database",
            STOCK_ANALYSIS_SQLITE_APPLICATION_ID,
            STOCK_ANALYSIS_DB_SCHEMA_GENERATION,
        );
        fs::remove_file(database_fixture.database()).expect("remove database before FIFO");
        create_fifo(&database_fixture.database());
        let error = database_fixture
            .inspect()
            .expect_err("database FIFO must fail closed");
        assert_eq!(error.code(), "global_schema_database_not_regular");

        let lock_fixture = TestFixture::new(
            "fifo-lock",
            STOCK_ANALYSIS_SQLITE_APPLICATION_ID,
            STOCK_ANALYSIS_DB_SCHEMA_GENERATION,
        );
        fs::create_dir(lock_fixture.root.join("locks")).expect("create lock directory");
        create_fifo(&lock_fixture.lock_file());
        let error = lock_fixture
            .inspect()
            .expect_err("lock FIFO must fail closed");
        assert_eq!(error.code(), "global_schema_unsafe_fixed_path");
    }

    #[test]
    fn missing_database_is_explicit_unavailable_and_not_created() {
        let fixture = TestFixture::new(
            "missing",
            STOCK_ANALYSIS_SQLITE_APPLICATION_ID,
            STOCK_ANALYSIS_DB_SCHEMA_GENERATION,
        );
        fs::remove_file(fixture.database()).expect("remove fixture database");
        let error = fixture
            .inspect()
            .expect_err("missing database must not be initialized");
        assert_eq!(error.code(), "global_schema_database_unavailable");
        assert!(!fixture.database().exists());
    }

    #[test]
    fn wal_and_shm_are_pinned_then_fail_closed_without_a_verified_capability() {
        let fixture = TestFixture::new(
            "wal-blocked",
            STOCK_ANALYSIS_SQLITE_APPLICATION_ID,
            STOCK_ANALYSIS_DB_SCHEMA_GENERATION,
        );
        let paths = fixture.binding();
        File::create(&paths.wal).expect("create test WAL sidecar");
        File::create(&paths.shm).expect("create test SHM sidecar");
        let before = fs::read(fixture.database()).expect("read database before WAL rejection");

        let error = fixture
            .inspect()
            .expect_err("WAL-backed identity must not publish a verified capability");
        assert_eq!(error.code(), "global_schema_wal_inspection_unavailable");
        assert_eq!(
            fs::read(fixture.database()).expect("read database after WAL rejection"),
            before
        );
    }

    #[test]
    fn physical_isolation_rejects_hardlink_aliases() {
        let fixture = TestFixture::new(
            "hardlink",
            STOCK_ANALYSIS_SQLITE_APPLICATION_ID,
            STOCK_ANALYSIS_DB_SCHEMA_GENERATION,
        );
        let alias = fixture.root.join("hardlink-alias.db");
        fs::hard_link(fixture.database(), &alias).expect("create test hardlink alias");
        let error = fixture
            .inspect()
            .expect_err("multi-link TEST_CODE database must fail physical isolation");
        assert_eq!(error.code(), "global_schema_mode_binding_violation");
    }

    #[test]
    fn cross_process_exclusive_lock_makes_shared_startup_retryable() {
        let fixture = TestFixture::new(
            "cross-process",
            STOCK_ANALYSIS_SQLITE_APPLICATION_ID,
            STOCK_ANALYSIS_DB_SCHEMA_GENERATION,
        );
        drop(fixture.inspect().expect("create and release fixed lock"));

        let mut child = Command::new(std::env::current_exe().expect("current test executable"))
            .args([
                "--ignored",
                "--exact",
                "database::global_schema_v1::tests::TEST_CODE_global_schema_exclusive_child",
                "--nocapture",
            ])
            .env(CHILD_LOCK_PATH_ENV, fixture.lock_file())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn exclusive lock child");

        let stdout = child.stdout.take().expect("child stdout");
        let mut stdout = BufReader::new(stdout);
        let mut ready = String::new();
        for _ in 0..20 {
            let mut line = String::new();
            let read = stdout.read_line(&mut line).expect("read child ready line");
            if read == 0 {
                break;
            }
            ready.push_str(&line);
            if line.contains("TEST_CODE_GLOBAL_SCHEMA_EXCLUSIVE_LOCKED") {
                break;
            }
        }
        assert!(
            ready.contains("TEST_CODE_GLOBAL_SCHEMA_EXCLUSIVE_LOCKED"),
            "child did not acquire exclusive lock: {ready:?}"
        );

        let error = fixture
            .inspect()
            .expect_err("exclusive holder must block shared startup");
        assert!(matches!(
            error,
            GlobalSchemaV1Error::MaintenanceLeaseUnavailable {
                retryable: true,
                ..
            }
        ));

        drop(child.stdin.take());
        let output = child.wait_with_output().expect("wait for lock child");
        assert!(
            output.status.success(),
            "lock child failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    #[ignore = "helper process for cross_process_exclusive_lock_makes_shared_startup_retryable"]
    #[allow(non_snake_case)]
    fn TEST_CODE_global_schema_exclusive_child() {
        let path = PathBuf::from(
            std::env::var_os(CHILD_LOCK_PATH_ENV).expect("child lock path environment"),
        );
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(O_NOFOLLOW_FLAG)
            .open(&path)
            .expect("child open fixed lock");
        FileExt::try_lock_exclusive(&file).expect("child acquire exclusive lock");
        println!("TEST_CODE_GLOBAL_SCHEMA_EXCLUSIVE_LOCKED");
        std::io::stdout().flush().expect("flush child ready");
        let mut release = String::new();
        std::io::stdin()
            .read_to_string(&mut release)
            .expect("wait for parent release");
        FileExt::unlock(&file).expect("child unlock");
    }

    #[test]
    #[ignore = "helper process for cross_process_shared_holder_makes_exclusive_retryable"]
    #[allow(non_snake_case)]
    fn TEST_CODE_global_schema_shared_child() {
        let path = PathBuf::from(
            std::env::var_os(CHILD_LOCK_PATH_ENV).expect("child lock path environment"),
        );
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(O_NOFOLLOW_FLAG | O_NONBLOCK_FLAG | O_CLOEXEC_FLAG)
            .open(&path)
            .expect("child open fixed lock");
        FileExt::try_lock_shared(&file).expect("child acquire shared lock");
        println!("TEST_CODE_GLOBAL_SCHEMA_SHARED_LOCKED");
        std::io::stdout().flush().expect("flush child ready");
        let mut release = String::new();
        std::io::stdin()
            .read_to_string(&mut release)
            .expect("wait for parent release");
        FileExt::unlock(&file).expect("child unlock");
    }
}
