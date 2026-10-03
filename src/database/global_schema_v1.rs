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

#[path = "global_schema_backup_v1.rs"]
mod backup;
#[path = "global_schema_paper_v6.rs"]
pub(crate) mod paper_v6;
#[path = "global_schema_prospective_v1.rs"]
mod prospective;
#[path = "global_schema_rows_v1.rs"]
mod rows;

pub(crate) const STOCK_ANALYSIS_SQLITE_APPLICATION_ID: i64 = 1_398_035_265;
pub(crate) const STOCK_ANALYSIS_DB_SCHEMA_GENERATION: i64 = 1;
const PAPER_LEDGER_CATALOG_GENERATION: i64 = super::paper_ledger_schema_v1::CATALOG_GENERATION;
const REVIEW_CATALOG_GENERATION: i64 = super::daily_change_review_schema_v1::CATALOG_GENERATION;
const PAPER_BOOK_OWNER_CATALOG_GENERATION: i64 =
    super::paper_book_owner_schema_v1::CATALOG_GENERATION;
const PAPER_BOOK_PREPARED_CATALOG_GENERATION: i64 =
    super::paper_book_owner_schema_v2::CATALOG_GENERATION;

const PAPER_BOOK_EXECUTION_CATALOG_GENERATION: i64 = 6;

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
        "unsupported global schema identity application_id={application_id},user_version={user_version}; expected application_id=1398035265,user_version=1 or explicitly qualified 2, 3, or 4"
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
    let mut prepare = false;
    let mut prepare_backup = false;
    let mut test_rehearsal = false;
    let mut apply = false;
    let mut help = false;
    for raw in args {
        let raw = raw.into();
        let argument = raw
            .to_str()
            .ok_or_else(|| "migration argument is not valid UTF-8".to_owned())?;
        match argument {
            "--prepare" if !prepare => prepare = true,
            "--prepare-backup" if !prepare_backup => prepare_backup = true,
            "--test" if !test_rehearsal => test_rehearsal = true,
            "--apply" if !apply => apply = true,
            "--help" | "-h" if !help => help = true,
            "--prepare" | "--prepare-backup" | "--test" | "--apply" | "--help" | "-h" => {
                return Err(format!("duplicate migration argument: {argument}"));
            }
            _ => return Err(format!("unsupported migration argument: {argument}")),
        }
    }
    if help {
        if test_rehearsal || apply || prepare || prepare_backup {
            return Err("--help cannot be combined with migration actions".to_owned());
        }
        return Ok(selection_v2_migration_help().to_owned());
    }
    if prepare && (test_rehearsal || apply) {
        return Err("--prepare cannot be combined with other migration actions".into());
    }
    if prepare_backup && (prepare || test_rehearsal || apply) {
        return Err("--prepare-backup cannot be combined with other migration actions".into());
    }
    if apply && !test_rehearsal {
        return Err(super::selection_v2::SELECTION_V2_APPLY_BLOCKER.to_owned());
    }

    let owner = new_global_schema_version_owner();
    if prepare_backup {
        return owner
            .prepare_fixed_selection_backup()
            .and_then(backup::VerifiedUnapprovedByteBackup::render_unapproved)
            .map_err(|error| prospective::render_error(&error));
    }
    if prepare {
        return owner
            .prepare_fixed_selection_prospective()
            .and_then(prospective::PreparedGlobalSchemaProspective::render_unapproved)
            .map_err(|error| prospective::render_error(&error));
    }
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
    "Usage: migrate_selection_v2 [--test] [--apply] | --prepare | --prepare-backup\n\
\n\
Default: owner-locked diagnostic against the fixed production database/audit.\n\
--prepare: unapproved prospective source/target observation only; no migration,\n\
           backup, receipt, or apply authority. Cannot combine with other actions.\n\
--prepare-backup: fixed single-operation original-byte backup and recovery;\n\
                  unapproved, no row-preservation, restore or apply authority.\n\
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
        SelectionSchemaAuthorityDiagnostic::CatalogV2RequalificationRequired => {
            "catalog_v2_requalification_required"
        }
        SelectionSchemaAuthorityDiagnostic::CatalogV3RequalificationRequired => {
            "catalog_v3_requalification_required"
        }
        SelectionSchemaAuthorityDiagnostic::CatalogV4RequalificationRequired => {
            "catalog_v4_requalification_required"
        }
        SelectionSchemaAuthorityDiagnostic::CatalogV6RequalificationRequired => {
            "catalog_v6_requalification_required"
        }
        SelectionSchemaAuthorityDiagnostic::CatalogV5RequalificationRequired => {
            "catalog_v5_requalification_required"
        }
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
        match self.observe_selection_with_optional_pinned_root(
            paths,
            root,
            audit_writer,
            catalog_mode,
            SelectionSnapshotPurpose::Diagnostic,
        )? {
            SelectionOwnerObservation::Inspected(outcome) => Ok(outcome),
            SelectionOwnerObservation::Prospective(_)
            | SelectionOwnerObservation::Backup(_)
            | SelectionOwnerObservation::RowsBackup(_) => {
                Err(prospective::refusal("unexpected prospective owner branch"))
            }
        }
    }

    fn prepare_fixed_selection_backup(
        &self,
    ) -> Result<backup::VerifiedUnapprovedByteBackup, GlobalSchemaV1Error> {
        let audit_writer = SelectionAuditWriter::production()
            .map_err(|source| GlobalSchemaV1Error::SelectionAudit { source })?;
        self.prepare_backup_with_bound_paths(
            ModeBoundPaths::production(),
            &audit_writer,
            GlobalSchemaCatalogMode::Production,
            backup::Options::production(),
        )
    }

    fn prepare_backup_with_bound_paths(
        &self,
        paths: ModeBoundPaths,
        audit_writer: &SelectionAuditWriter,
        catalog_mode: GlobalSchemaCatalogMode,
        mut options: backup::Options,
    ) -> Result<backup::VerifiedUnapprovedByteBackup, GlobalSchemaV1Error> {
        options.validate_mode(paths.mode)?;
        options.bind_common_budget()?;
        match self.observe_selection_with_optional_pinned_root(
            paths,
            None,
            audit_writer,
            catalog_mode,
            SelectionSnapshotPurpose::Backup(options),
        )? {
            SelectionOwnerObservation::Backup(result) => Ok(result),
            _ => Err(prospective::refusal("unexpected backup owner branch")),
        }
    }

    fn prepare_fixed_selection_rows_backup(
        &self,
    ) -> Result<rows::VerifiedUnapprovedOriginalRowsBackup, GlobalSchemaV1Error> {
        let writer = SelectionAuditWriter::production()
            .map_err(|source| GlobalSchemaV1Error::SelectionAudit { source })?;
        self.prepare_rows_backup_with_bound_paths(
            ModeBoundPaths::production(),
            &writer,
            GlobalSchemaCatalogMode::Production,
            rows::Options::production(),
        )
    }

    fn prepare_rows_backup_with_bound_paths(
        &self,
        paths: ModeBoundPaths,
        writer: &SelectionAuditWriter,
        catalog_mode: GlobalSchemaCatalogMode,
        mut options: rows::Options,
    ) -> Result<rows::VerifiedUnapprovedOriginalRowsBackup, GlobalSchemaV1Error> {
        options.validate_mode(paths.mode)?;
        options.backup.bind_common_budget()?;
        match self.observe_selection_with_optional_pinned_root(
            paths,
            None,
            writer,
            catalog_mode,
            SelectionSnapshotPurpose::RowsBackup(options),
        )? {
            SelectionOwnerObservation::RowsBackup(result) => Ok(result),
            _ => Err(prospective::refusal("unexpected rows backup owner branch")),
        }
    }

    fn prepare_fixed_selection_prospective(
        &self,
    ) -> Result<prospective::PreparedGlobalSchemaProspective, GlobalSchemaV1Error> {
        let audit_writer = SelectionAuditWriter::production()
            .map_err(|source| GlobalSchemaV1Error::SelectionAudit { source })?;
        self.prepare_selection_with_bound_paths(
            ModeBoundPaths::production(),
            &audit_writer,
            GlobalSchemaCatalogMode::Production,
            prospective::Options::production(),
        )
    }

    fn prepare_selection_with_bound_paths(
        &self,
        paths: ModeBoundPaths,
        audit_writer: &SelectionAuditWriter,
        catalog_mode: GlobalSchemaCatalogMode,
        options: prospective::Options,
    ) -> Result<prospective::PreparedGlobalSchemaProspective, GlobalSchemaV1Error> {
        match self.observe_selection_with_optional_pinned_root(
            paths,
            None,
            audit_writer,
            catalog_mode,
            SelectionSnapshotPurpose::Prospective(options),
        )? {
            SelectionOwnerObservation::Prospective(observation) => Ok(observation),
            SelectionOwnerObservation::Inspected(_)
            | SelectionOwnerObservation::Backup(_)
            | SelectionOwnerObservation::RowsBackup(_) => {
                Err(prospective::refusal("unexpected diagnostic owner branch"))
            }
        }
    }

    fn observe_selection_with_optional_pinned_root(
        &self,
        paths: ModeBoundPaths,
        root: Option<PinnedRoot>,
        audit_writer: &SelectionAuditWriter,
        catalog_mode: GlobalSchemaCatalogMode,
        purpose: SelectionSnapshotPurpose,
    ) -> Result<SelectionOwnerObservation, GlobalSchemaV1Error> {
        let bound_mode = paths.mode;
        if let Some(options) = purpose.options() {
            options.validate_mode(bound_mode)?;
        }
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
        let backup_workspace = match &purpose {
            SelectionSnapshotPurpose::Backup(options) => {
                options.validate_mode(bound_mode)?;
                Some(backup::Workspace::open(&maintenance, &options.settings)?)
            }
            SelectionSnapshotPurpose::RowsBackup(options) => {
                options.validate_mode(bound_mode)?;
                Some(backup::Workspace::open(
                    &maintenance,
                    &options.backup.settings,
                )?)
            }
            _ => None,
        };
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
            let mut audit_session = match purpose.options() {
                Some(options) => audit_writer.locked_session_bounded(options.audit_limits),
                None => audit_writer.locked_session(),
            }
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
            if let Some(options) = purpose.options() {
                prospective::require_zero_owned_wal(&inspection_sidecars)?;
                options.phase(prospective::Phase::BeforeInitialCapture)?;
                super::global_schema_catalog_v1::prospective_catalog_extent(
                    &transaction,
                    options.max_catalog_objects,
                    options.max_catalog_bytes,
                )
                .map_err(|source| GlobalSchemaV1Error::SelectionCatalog { source })?;
            }
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
                prospective_options: purpose.options(),
                backup_options: purpose.backup_options(),
                rows_options: purpose.rows_options(),
                backup_workspace,
                bound_mode,
            }
            .consume_authority()?;
            Ok((prepared, audit_parent, audit_file))
        })();

        drop(connection);
        let cleanup = inspection_sidecars
            .cleanup_after_connection_close(&maintenance.namespace, &database_path);
        match (inspection_result, cleanup) {
            (Ok((mut prepared, audit_parent, audit_file)), Ok(())) => match purpose {
                SelectionSnapshotPurpose::Diagnostic => Ok(SelectionOwnerObservation::Inspected(prepared.issue(database_file, database_identity, audit_parent, audit_file, maintenance))),
                SelectionSnapshotPurpose::Prospective(options) => {
                    let pending = prepared.prospective_pending.take().ok_or_else(|| prospective::refusal("prospective owner material missing"))?;
                    Ok(SelectionOwnerObservation::Prospective(pending.issue(options, database_file, database_identity, audit_parent, audit_file, maintenance)?))
                }
                SelectionSnapshotPurpose::Backup(options) => {
                    let pending = prepared.prospective_pending.take().ok_or_else(|| prospective::refusal("backup source material missing"))?;
                    let backup_pending = prepared.backup_pending.take().ok_or_else(|| prospective::refusal("backup prepared IO missing"))?;
                    let source = pending.issue(options.source, database_file, database_identity, audit_parent, audit_file, maintenance)?;
                    Ok(SelectionOwnerObservation::Backup(backup_pending.issue(source, options.settings)?))
                }
                SelectionSnapshotPurpose::RowsBackup(options) => {
                    let pending=prepared.prospective_pending.take().ok_or_else(||prospective::refusal("rows source material missing"))?;
                    let backup_pending=prepared.backup_pending.take().ok_or_else(||prospective::refusal("rows actual backup missing"))?;
                    let rows_pending=prepared.rows_pending.take().ok_or_else(||prospective::refusal("rows original pairs missing"))?;
                    let source=pending.issue(options.backup.source,database_file,database_identity,audit_parent,audit_file,maintenance)?;
                    let byte_backup=backup_pending.issue(source,options.backup.settings)?;
                    Ok(SelectionOwnerObservation::RowsBackup(rows_pending.issue(byte_backup)?))
                }
            },
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
    /// Existing selection receipts only prove generation 1. Task10 must add an
    /// explicit whole-catalog maintenance receipt before issuing V2 authority.
    CatalogV2RequalificationRequired,
    CatalogV3RequalificationRequired,
    CatalogV4RequalificationRequired,
    CatalogV5RequalificationRequired,
    CatalogV6RequalificationRequired,
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
enum SelectionSnapshotPurpose {
    Diagnostic,
    Prospective(prospective::Options),
    Backup(backup::Options),
    RowsBackup(rows::Options),
}
impl SelectionSnapshotPurpose {
    fn options(&self) -> Option<&prospective::Options> {
        match self {
            Self::Diagnostic => None,
            Self::Prospective(options) => Some(options),
            Self::Backup(options) => Some(&options.source),
            Self::RowsBackup(options) => Some(&options.backup.source),
        }
    }
    fn backup_options(&self) -> Option<&backup::Options> {
        match self {
            Self::Backup(options) => Some(options),
            Self::RowsBackup(options) => Some(&options.backup),
            _ => None,
        }
    }
}
impl SelectionSnapshotPurpose {
    fn rows_options(&self) -> Option<&rows::Options> {
        match self {
            Self::RowsBackup(options) => Some(options),
            _ => None,
        }
    }
}
enum SelectionOwnerObservation {
    Inspected(SelectionSchemaInspectionOutcome),
    Prospective(prospective::PreparedGlobalSchemaProspective),
    Backup(backup::VerifiedUnapprovedByteBackup),
    RowsBackup(rows::VerifiedUnapprovedOriginalRowsBackup),
}

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
    prospective_options: Option<&'sidecars prospective::Options>,
    backup_options: Option<&'sidecars backup::Options>,
    rows_options: Option<&'sidecars rows::Options>,
    backup_workspace: Option<backup::Workspace>,
    bound_mode: BoundMode,
}

struct PreparedSelectionSchemaInspection {
    database_half: DatabaseHalfDiagnostic,
    authority_state: SelectionSchemaAuthorityDiagnostic,
    audit_high_water: AuditValidationReceipt,
    selection_row_counts: std::collections::BTreeMap<String, i64>,
    exact_amended: bool,
    prospective_pending: Option<prospective::Pending>,
    backup_pending: Option<backup::Pending>,
    rows_pending: Option<rows::Pending>,
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

        let mut prospective_pending = match self.prospective_options {
            Some(options) => {
                let material = prospective::Pending::capture(
                    &self,
                    &references,
                    &database_half,
                    exact_amended,
                    self.bound_mode,
                    options,
                )?;
                options.phase(prospective::Phase::AfterInitialCapture)?;
                options.phase(prospective::Phase::BeforeFinalCapture)?;
                super::global_schema_catalog_v1::prospective_catalog_extent(
                    &self.transaction,
                    options.max_catalog_objects,
                    options.max_catalog_bytes,
                )
                .map_err(|source| GlobalSchemaV1Error::SelectionCatalog { source })?;
                Some(material)
            }
            None => None,
        };
        // Close the whole family and reserve all six row streams before
        // Workspace.prepare can create either durable role.
        let mut rows_pending = match (self.rows_options, prospective_pending.as_ref()) {
            (Some(options), Some(source)) => {
                Some(rows::Pending::capture(&self, &references, source, options)?)
            }
            (None, _) => None,
            _ => return Err(prospective::refusal("rows pre-copy source missing")),
        };
        let mut backup_pending = match (
            self.backup_workspace.take(),
            self.backup_options,
            prospective_pending.as_mut(),
            self.prospective_options,
        ) {
            (Some(workspace), Some(options), Some(source), Some(source_options)) => {
                Some(workspace.prepare(
                    source,
                    source_options,
                    self.database_file,
                    self.audit_file,
                    &options.settings,
                )?)
            }
            (None, None, _, _) => None,
            _ => return Err(prospective::refusal("backup owner state mismatch")),
        };
        if let (Some(row_match), Some(backup), Some(source), Some(options), Some(settings)) = (
            &mut rows_pending,
            &mut backup_pending,
            &mut prospective_pending,
            self.prospective_options,
            self.backup_options,
        ) {
            row_match.pair(
                &self,
                &references,
                backup,
                source,
                options,
                &settings.settings,
                false,
            )?;
        }
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

        if let (Some(material), Some(options)) =
            (&mut prospective_pending, self.prospective_options)
        {
            material.validate_snapshot(&self, options)?;
            if let (Some(backup), Some(settings)) = (&mut backup_pending, self.backup_options) {
                backup.validate_before_commit(material, options, &settings.settings)?;
                // Output reads precede the last actual original source reader.
                material.validate_snapshot(&self, options)?;
            }
        }
        // Every hookful validation above completes before this last direct
        // pair. From reader close through COMMIT only full hook-free tails run.
        if let (Some(row_match), Some(backup), Some(source), Some(options), Some(settings)) = (
            &mut rows_pending,
            &mut backup_pending,
            &mut prospective_pending,
            self.prospective_options,
            self.backup_options,
        ) {
            row_match.pair(
                &self,
                &references,
                backup,
                source,
                options,
                &settings.settings,
                true,
            )?;
            backup.validate_rows_before_commit_without_hooks(
                source,
                options,
                &settings.settings,
            )?;
            row_match.before_commit_tail(&mut self, source, options)?;
        }
        self.transaction
            .commit()
            .map_err(|source| GlobalSchemaV1Error::SelectionSqlite {
                operation: "finish read-only inspection transaction",
                source,
            })?;
        if let Some(options) = self.prospective_options {
            options.phase(prospective::Phase::AfterReadOnlyCommit)?;
        }
        let finished_audit = self
            .audit_session
            .finish()
            .map_err(|source| GlobalSchemaV1Error::SelectionAudit { source })?;
        if finished_audit != audit_high_water {
            return Err(GlobalSchemaV1Error::SelectionSnapshotChanged {
                detail: "audit finish high-water differs from captured high-water".to_owned(),
            });
        }
        if let Some(pending) = &mut rows_pending {
            pending.transaction_finished(references)?;
        }
        Ok(PreparedSelectionSchemaInspection {
            database_half,
            authority_state,
            audit_high_water,
            selection_row_counts,
            exact_amended,
            prospective_pending,
            backup_pending,
            rows_pending,
        })
    }
}

fn classify_selection_authority_state(
    database_half: &DatabaseHalfDiagnostic,
    audit_present: bool,
    audit: &ValidatedAuditChainSnapshot,
) -> Result<SelectionSchemaAuthorityDiagnostic, GlobalSchemaV1Error> {
    let evidence = match database_half {
        DatabaseHalfDiagnostic::AbsentDatabaseHalf(e)
        | DatabaseHalfDiagnostic::PreAmendment(e)
        | DatabaseHalfDiagnostic::Transitional(e)
        | DatabaseHalfDiagnostic::AmendedDatabaseHalf(e) => e,
    };
    if evidence.identity.user_version == PAPER_BOOK_EXECUTION_CATALOG_GENERATION {
        return Ok(SelectionSchemaAuthorityDiagnostic::CatalogV6RequalificationRequired);
    }
    if evidence.identity.user_version == PAPER_BOOK_PREPARED_CATALOG_GENERATION {
        return Ok(SelectionSchemaAuthorityDiagnostic::CatalogV5RequalificationRequired);
    }
    if evidence.identity.user_version == PAPER_BOOK_OWNER_CATALOG_GENERATION {
        return Ok(SelectionSchemaAuthorityDiagnostic::CatalogV4RequalificationRequired);
    }
    if evidence.identity.user_version == PAPER_LEDGER_CATALOG_GENERATION {
        return Ok(SelectionSchemaAuthorityDiagnostic::CatalogV2RequalificationRequired);
    }
    if evidence.identity.user_version == REVIEW_CATALOG_GENERATION {
        return Ok(SelectionSchemaAuthorityDiagnostic::CatalogV3RequalificationRequired);
    }
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

/// Header-only proof that the mode-bound database was read as `STSA/1` or
/// `STSA/2` (not CatalogV2 qualification) while
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
        let root = crate::production_root::production_root().to_path_buf();
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
                let fixed = crate::production_root::production_root();
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
                        detail: "production paths differ from build-time fixed identities"
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
        && matches!(
            user_version,
            STOCK_ANALYSIS_DB_SCHEMA_GENERATION
                | PAPER_LEDGER_CATALOG_GENERATION
                | REVIEW_CATALOG_GENERATION
                | PAPER_BOOK_OWNER_CATALOG_GENERATION
                | PAPER_BOOK_PREPARED_CATALOG_GENERATION
                | PAPER_BOOK_EXECUTION_CATALOG_GENERATION
        )
    {
        return Ok(GlobalSchemaIdentity {
            application_id,
            user_version,
        });
    }
    if application_id == STOCK_ANALYSIS_SQLITE_APPLICATION_ID
        && user_version > PAPER_BOOK_EXECUTION_CATALOG_GENERATION
    {
        return Err(GlobalSchemaV1Error::UnsupportedFutureGeneration {
            actual: user_version,
            supported: PAPER_BOOK_EXECUTION_CATALOG_GENERATION,
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
    use sha2::Digest;
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

    pub(super) static PROSPECTIVE_TEST_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn prospective_test_fixture(
        label: &str,
        generation: i64,
        final_catalog: bool,
    ) -> (TestFixture, SelectionAuditWriter) {
        let fixture = TestFixture::new(label, 0, 0);
        if final_catalog {
            fixture.install_final_selection_catalog();
        } else {
            let connection = Connection::open(fixture.database()).unwrap();
            super::super::global_schema_catalog_v1::install_legacy_catalog_for_prospective_test(
                &connection,
            )
            .unwrap();
            drop(connection);
        }
        let connection = Connection::open(fixture.database()).unwrap();
        if generation >= 2 {
            for (_, _, _, sql) in super::super::paper_ledger_schema_v1::STATEMENTS {
                connection.execute_batch(sql).unwrap();
            }
        }
        if generation >= 3 {
            for (_, _, _, sql) in super::super::daily_change_review_schema_v1::STATEMENTS {
                connection.execute_batch(sql).unwrap();
            }
        }
        if generation >= 4 {
            for statements in [
                super::super::paper_book_v2_schema::STATEMENTS,
                if generation == 5 {
                    super::super::paper_book_owner_schema_v2::OWNER_STATEMENTS
                } else {
                    super::super::paper_book_owner_schema_v1::STATEMENTS
                },
                if generation == 5 {
                    super::super::paper_book_owner_schema_v2::V1_GUARD_STATEMENTS
                } else {
                    super::super::paper_book_owner_schema_v1::V1_GUARD_STATEMENTS
                },
            ] {
                for (_, _, _, sql) in statements {
                    connection.execute_batch(sql).unwrap();
                }
            }
        }
        if generation == 5 {
            for (_, _, _, sql) in super::super::paper_book_v2_ledger_schema_v1::STATEMENTS {
                connection.execute_batch(sql).unwrap();
            }
        }
        if generation > 0 {
            connection
                .pragma_update(None, "application_id", STOCK_ANALYSIS_SQLITE_APPLICATION_ID)
                .unwrap();
            connection
                .pragma_update(None, "user_version", generation)
                .unwrap();
        }
        connection
            .execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_checkpoint(TRUNCATE)")
            .unwrap();
        drop(connection);
        for suffix in ["-wal", "-shm"] {
            let path = sidecar_path(&fixture.database(), suffix);
            if path.exists() {
                fs::remove_file(path).unwrap();
            }
        }
        let writer = fixture.pinned_audit_writer();
        (fixture, writer)
    }

    fn rows_test_prepare(
        fixture: &TestFixture,
        writer: &SelectionAuditWriter,
        options: rows::Options,
    ) -> Result<rows::VerifiedUnapprovedOriginalRowsBackup, GlobalSchemaV1Error> {
        GlobalSchemaVersionOwner::for_test_code().prepare_rows_backup_with_bound_paths(
            fixture.binding(),
            writer,
            GlobalSchemaCatalogMode::Test,
            options,
        )
    }
    fn rows_test_seed(fixture: &TestFixture) {
        prospective_with_offline_fixture_connection(fixture, |c| {
            c.execute("INSERT INTO ledger(date,total_value,cash,market_value,daily_pnl,created_at) VALUES('2026-09-28',123.25,12.25,111,0,'TEST_CODE_ROWS_PRIVATE_TEXT')",[]).unwrap();
            c.execute("INSERT INTO stock_daily(code,date,open,high,low,close,volume) VALUES('TEST_CODE_ROWS_SENTINEL','2026-09-28',10,10,10,10,100)",[]).unwrap();
            // Preserve a deleted high allocation and duplicate/unknown actual
            // sequence facts; an AUTOINCREMENT owner set is not a row set.
            c.execute("INSERT INTO ledger(id,date,total_value,cash,market_value,daily_pnl,created_at) VALUES(90,'2026-09-29',0,0,0,0,'deleted')",[]).unwrap();
            c.execute("DELETE FROM ledger WHERE id=90", []).unwrap();
            c.execute("INSERT INTO sqlite_sequence(name,seq) VALUES('ledger',91),('TEST_CODE_UNKNOWN_COUNTER',X'3100FF')",[]).unwrap();
        });
    }
    fn rows_test_report(rendered: &str) -> serde_json::Value {
        serde_json::from_str(rendered).unwrap()
    }
    fn rows_test_names(report: &serde_json::Value) -> Vec<String> {
        report["tables"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_owned())
            .collect()
    }
    #[test]
    fn rows_backup_actual_all_legacy_tables_and_sequence_keep_source_and_lease() {
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        let (fixture, writer) = prospective_test_fixture("rows-all-legacy", 0, false);
        rows_test_seed(&fixture);
        backup_test_append_audit(&writer);
        let expected = prospective_with_offline_fixture_connection(&fixture, |c| {
            c.prepare("SELECT name FROM sqlite_schema WHERE type='table' ORDER BY name")
                .unwrap()
                .query_map([], |r| r.get::<_, String>(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        });
        let before = fs::read(fixture.database()).unwrap();
        let audit = fs::read(writer.path()).unwrap();
        let cap = rows_test_prepare(&fixture, &writer, rows::Options::production()).unwrap();
        assert!(matches!(
            fixture.acquire_exclusive(),
            Err(GlobalSchemaV1Error::ExclusiveProcessMaintenanceLeaseUnavailable)
        ));
        let rendered = cap.render_unapproved().unwrap();
        let report = rows_test_report(&rendered);
        assert_eq!(rows_test_names(&report), expected);
        let sequence = report["tables"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == "sqlite_sequence")
            .unwrap();
        assert!(sequence["rows"].as_u64().unwrap() >= 3);
        assert_eq!(report["streams"], 6);
        assert_eq!(report["row_preservation_proof"], true);
        assert_eq!(report["target_row_preservation_proof"], false);
        assert_eq!(report["approval"], "not_granted");
        assert_eq!(report["apply_supported"], false);
        assert_eq!(
            report["apply_blocker"],
            crate::database::selection_v2::SELECTION_V2_APPLY_BLOCKER
        );
        assert!(!rendered.contains("TEST_CODE_ROWS_PRIVATE_TEXT"));
        assert!(!rendered.contains("TEST_CODE_ROWS_SENTINEL"));
        assert_eq!(fs::read(fixture.database()).unwrap(), before);
        assert_eq!(fs::read(writer.path()).unwrap(), audit);
        prospective_assert_fixture_offline(&fixture);
        drop(fixture.acquire_exclusive().unwrap());
    }
    #[test]
    fn rows_backup_exact_catalog_families_include_whole_selection_and_paper() {
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        for generation in [0, 2, 3, 4, 5] {
            for final_catalog in [false, true] {
                if generation == 0 && final_catalog {
                    continue;
                } // Amended1 requires original receipts.
                let (fixture, writer) =
                    prospective_test_fixture("rows-families", generation, final_catalog);
                let expected = prospective_with_offline_fixture_connection(&fixture, |c| {
                    c.prepare("SELECT name FROM sqlite_schema WHERE type='table' ORDER BY name")
                        .unwrap()
                        .query_map([], |r| r.get::<_, String>(0))
                        .unwrap()
                        .collect::<Result<Vec<_>, _>>()
                        .unwrap()
                });
                let rendered = rows_test_prepare(&fixture, &writer, rows::Options::production())
                    .unwrap()
                    .render_unapproved()
                    .unwrap();
                assert_eq!(
                    rows_test_names(&rows_test_report(&rendered)),
                    expected,
                    "family {generation}/{final_catalog}"
                );
            }
        }
    }
    #[test]
    fn rows_backup_restart_requires_fresh_original_pairs_and_preserves_terminal() {
        use std::sync::{Arc, Mutex};
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        let (fixture, writer) = prospective_test_fixture("rows-restart", 0, false);
        rows_test_seed(&fixture);
        rows_test_prepare(&fixture, &writer, rows::Options::production())
            .unwrap()
            .render_unapproved()
            .unwrap();
        let original = backup_test_known_bytes(&fixture);
        let trace = Arc::new(Mutex::new(Vec::new()));
        let mut options = rows::Options::production();
        options.trace = Some(Arc::clone(&trace));
        rows_test_prepare(&fixture, &writer, options)
            .unwrap()
            .render_unapproved()
            .unwrap();
        let events = trace.lock().unwrap();
        assert!(events.contains(&"rows_initial_pair_closed"));
        assert!(events.contains(&"rows_final_pair_closed"));
        assert!(events.contains(&"rows_issue_reader_closed"));
        assert!(events.contains(&"rows_render_reader_closed"));
        assert_eq!(backup_test_known_bytes(&fixture), original);
        prospective_with_offline_fixture_connection(&fixture, |c| {
            c.execute("UPDATE ledger SET cash=cash+1", []).unwrap();
        });
        assert!(rows_test_prepare(&fixture, &writer, rows::Options::production()).is_err());
        assert_eq!(backup_test_known_bytes(&fixture), original);
    }
    #[test]
    fn rows_backup_shared_preflight_limits_fail_before_any_role_or_copy() {
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        for boundary in [
            "table", "column", "cell", "row", "rows", "bytes", "metadata",
        ] {
            let (fixture, writer) = prospective_test_fixture("rows-precopy-budget", 0, false);
            rows_test_seed(&fixture);
            let before = fs::read(fixture.database()).unwrap();
            let mut options = rows::Options::production();
            match boundary {
                "table" => options.limits.tables = 1,
                "column" => options.limits.columns = 1,
                "cell" => options.limits.cell_bytes = 1,
                "row" => options.limits.row_bytes = 1,
                "rows" => options.limits.row_observations = 1,
                "bytes" => options.limits.observation_bytes = 1,
                _ => options.limits.metadata_bytes = 1,
            }
            assert!(
                rows_test_prepare(&fixture, &writer, options).is_err(),
                "{boundary}"
            );
            let directory = backup_test_directory(&fixture);
            for leaf in [
                "stock_analysis.db.backup",
                "selection-audit.jsonl.backup",
                "000-intent.json",
                "001-created-db.json",
                "002-copied-db.json",
            ] {
                assert!(!directory.join(leaf).exists(), "{boundary}/{leaf}");
            }
            assert_eq!(fs::read(fixture.database()).unwrap(), before);
            prospective_assert_fixture_offline(&fixture);
        }
    }
    #[test]
    fn rows_backup_six_pass_reservation_and_finite_review_are_enforced() {
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        let (fixture, writer) = prospective_test_fixture("rows-six-budget", 0, false);
        rows_test_seed(&fixture);
        // Count actual rows in every actual main table, including allocation
        // facts, and leave enough for five streams but not the promised six.
        let count = prospective_with_offline_fixture_connection(&fixture, |c| {
            let names = c
                .prepare("SELECT name FROM sqlite_schema WHERE type='table'")
                .unwrap()
                .query_map([], |r| r.get::<_, String>(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            names
                .iter()
                .map(|n| {
                    c.query_row(
                        &format!("SELECT COUNT(*) FROM \"{}\"", n.replace('"', "\"\"")),
                        [],
                        |r| r.get::<_, u64>(0),
                    )
                    .unwrap()
                })
                .sum::<u64>()
        });
        assert!(count > 0);
        let mut options = rows::Options::production();
        options.limits.row_observations = count * 6 - 1;
        assert!(rows_test_prepare(&fixture, &writer, options).is_err());
        assert!(!backup_test_directory(&fixture)
            .join("stock_analysis.db.backup")
            .exists());
        let gap_directory = backup_test_directory(&fixture);
        assert!(gap_directory.is_dir());
        assert!(!gap_directory.join("000-intent.json").exists());
        // Workspace.open already created this invocation's no-intent gap.
        // Preserve it; an independent review-limit case needs a fresh owner.
        let (review_fixture, review_writer) =
            prospective_test_fixture("rows-review-budget", 0, false);
        rows_test_seed(&review_fixture);
        let mut options = rows::Options::production();
        options.limits.review_bytes = 8;
        let cap = rows_test_prepare(&review_fixture, &review_writer, options).unwrap();
        assert!(cap.render_unapproved().is_err());
        drop(review_fixture.acquire_exclusive().unwrap());
        assert!(gap_directory.is_dir());
        assert!(!gap_directory.join("000-intent.json").exists());
    }
    fn rows_test_mutate_same_inode_text(copy: &Path) {
        use std::os::unix::fs::FileExt as UnixFileExt;
        let marker = b"TEST_CODE_ROWS_PRIVATE_TEXT";
        let bytes = fs::read(copy).unwrap();
        let hits = bytes
            .windows(marker.len())
            .enumerate()
            .filter_map(|(i, b)| (b == marker).then_some(i))
            .collect::<Vec<_>>();
        assert_eq!(hits.len(), 1);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(O_NOFOLLOW_FLAG)
            .open(copy)
            .unwrap();
        let before = file.metadata().unwrap();
        let mut replacement = marker.to_vec();
        replacement[0] = b'U';
        file.write_all_at(&replacement, hits[0] as u64).unwrap();
        file.sync_all().unwrap();
        let mut actual = vec![0; marker.len()];
        assert_eq!(
            file.read_at(&mut actual, hits[0] as u64).unwrap(),
            marker.len()
        );
        assert_eq!(actual, replacement);
        let after = file.metadata().unwrap();
        assert_eq!(
            (before.dev(), before.ino(), before.len()),
            (after.dev(), after.ino(), after.len())
        );
    }
    #[test]
    fn rows_backup_issue_last_source_hook_mutation_cannot_keep_old_typed_proof() {
        use std::sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            Arc,
        };
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        let (fixture, writer) = prospective_test_fixture("rows-issue-last-hook", 0, false);
        rows_test_seed(&fixture);
        let source = fs::read(fixture.database()).unwrap();
        let directory = backup_test_directory(&fixture);
        let hit = Arc::new(AtomicBool::new(false));
        let mark = Arc::clone(&hit);
        let seen = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&seen);
        let mut options = rows::Options::production();
        options.backup.source.hook = Some(Box::new(move |phase| {
            if phase == prospective::Phase::AfterPhysicalReads
                && directory.join("005-backup-verified.json").exists()
            {
                // Byte issue's two probes, then rows preliminary's two probes.
                // The fourth is the last hook after its preceding copy hash.
                if count.fetch_add(1, Ordering::SeqCst) + 1 == 4 {
                    rows_test_mutate_same_inode_text(&directory.join("stock_analysis.db.backup"));
                    mark.store(true, Ordering::SeqCst);
                }
            }
            Ok(())
        }));
        assert!(rows_test_prepare(&fixture, &writer, options).is_err());
        assert!(hit.load(Ordering::SeqCst));
        assert_eq!(seen.load(Ordering::SeqCst), 4);
        assert_eq!(fs::read(fixture.database()).unwrap(), source);
        assert!(
            fs::read(backup_test_directory(&fixture).join("stock_analysis.db.backup"))
                .unwrap()
                .windows(b"UEST_CODE_ROWS_PRIVATE_TEXT".len())
                .any(|b| b == b"UEST_CODE_ROWS_PRIVATE_TEXT")
        );
    }
    #[test]
    fn rows_backup_render_last_source_hook_mutation_cannot_keep_old_typed_proof() {
        use std::sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            Arc,
        };
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        let (fixture, writer) = prospective_test_fixture("rows-render-last-hook", 0, false);
        rows_test_seed(&fixture);
        let copy = backup_test_directory(&fixture).join("stock_analysis.db.backup");
        let armed = Arc::new(AtomicBool::new(false));
        let trigger = Arc::clone(&armed);
        let hit = Arc::new(AtomicBool::new(false));
        let mark = Arc::clone(&hit);
        let seen = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&seen);
        let mut options = rows::Options::production();
        options.backup.source.hook = Some(Box::new(move |phase| {
            if trigger.load(Ordering::SeqCst)
                && phase == prospective::Phase::AfterPhysicalReads
                && count.fetch_add(1, Ordering::SeqCst) + 1 == 2
            {
                rows_test_mutate_same_inode_text(&copy);
                mark.store(true, Ordering::SeqCst);
            }
            Ok(())
        }));
        let cap = rows_test_prepare(&fixture, &writer, options).unwrap();
        armed.store(true, Ordering::SeqCst);
        assert!(cap.render_unapproved().is_err());
        assert!(hit.load(Ordering::SeqCst));
        assert_eq!(seen.load(Ordering::SeqCst), 2);
    }
    #[test]
    fn rows_backup_final_reader_trace_has_no_later_phase_hooks() {
        use std::sync::{Arc, Mutex};
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        let (fixture, writer) = prospective_test_fixture("rows-trace", 0, false);
        rows_test_seed(&fixture);
        let trace = Arc::new(Mutex::new(Vec::new()));
        let source = Arc::clone(&trace);
        let backup = Arc::clone(&trace);
        let mut options = rows::Options::production();
        options.trace = Some(Arc::clone(&trace));
        options.backup.source.hook = Some(Box::new(move |_| {
            source.lock().unwrap().push("source_hook");
            Ok(())
        }));
        options.backup.settings.hook = Some(Box::new(move |_| {
            backup.lock().unwrap().push("backup_hook");
            Ok(())
        }));
        let cap = rows_test_prepare(&fixture, &writer, options).unwrap();
        {
            let events = trace.lock().unwrap();
            let last = events
                .iter()
                .rposition(|e| *e == "rows_issue_reader_closed")
                .unwrap();
            assert_eq!(
                &events[last..],
                &["rows_issue_reader_closed", "rows_hook_free_tail_complete"]
            );
        }
        cap.render_unapproved().unwrap();
        let events = trace.lock().unwrap();
        let last = events
            .iter()
            .rposition(|e| *e == "rows_render_reader_closed")
            .unwrap();
        assert_eq!(
            &events[last..],
            &["rows_render_reader_closed", "rows_hook_free_tail_complete"]
        );
        let last_pair = events
            .iter()
            .position(|e| *e == "rows_final_pair_closed")
            .unwrap();
        assert_eq!(events[last_pair + 1], "rows_precommit_tail_complete");
    }
    #[test]
    fn rows_backup_render_rejects_sidecar_new_inode_and_replaced_lock_without_heal() {
        use std::os::unix::fs::PermissionsExt;
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        for attack in ["sidecar", "inode", "lock"] {
            let (fixture, writer) = prospective_test_fixture("rows-final-physical", 0, false);
            rows_test_seed(&fixture);
            let cap = rows_test_prepare(&fixture, &writer, rows::Options::production()).unwrap();
            let directory = backup_test_directory(&fixture);
            let copy = directory.join("stock_analysis.db.backup");
            let before = fs::read(&copy).unwrap();
            match attack {
                "sidecar" => {
                    fs::write(sidecar_path(&copy, "-wal"), b"UNKNOWN_SIDECAR").unwrap();
                }
                "inode" => {
                    fs::rename(&copy, directory.join("held-original.db")).unwrap();
                    fs::write(&copy, &before).unwrap();
                    fs::set_permissions(&copy, fs::Permissions::from_mode(0o600)).unwrap();
                }
                _ => {
                    let lock = fixture.lock_file();
                    fs::rename(&lock, lock.with_extension("displaced")).unwrap();
                    fs::write(&lock, b"").unwrap();
                }
            }
            assert!(cap.render_unapproved().is_err(), "{attack}");
            assert_eq!(fs::read(&copy).unwrap(), before);
            if attack == "sidecar" {
                assert_eq!(
                    fs::read(sidecar_path(&copy, "-wal")).unwrap(),
                    b"UNKNOWN_SIDECAR"
                );
            }
        }
    }
    #[test]
    fn rows_backup_old_byte_renderer_still_denies_rows_and_target_authority() {
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        let (fixture, writer) = prospective_test_fixture("rows-old-byte", 0, false);
        rows_test_seed(&fixture);
        let rendered = backup_test_prepare(&fixture, &writer, backup::Options::production())
            .unwrap()
            .render_unapproved()
            .unwrap();
        let report: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        assert_eq!(report["row_preservation_proof"], false);
        assert_eq!(report["apply_supported"], false);
        assert_eq!(
            report["apply_blocker"],
            crate::database::selection_v2::SELECTION_V2_APPLY_BLOCKER
        );
        assert_eq!(
            backup_test_known_bytes(&fixture)
                .iter()
                .filter(|(n, _)| n.ends_with(".json"))
                .count(),
            4
        );
    }

    #[test]
    fn rows_backup_actual_v6_seed_genesis_financial_family_closes_original_pools() {
        use crate::trading::paper_book_v2::{
            cutover_for_isolated_test, TestCutoverFault, TestCutoverRequest,
        };
        use crate::trading::paper_ledger::{
            Money, PaperCommand, PaperLedger, RiskPolicyV1, SeedManifest,
        };
        use chrono::{TimeZone, Utc};
        use diesel::connection::SimpleConnection;
        use std::sync::Arc;
        fn instant() -> chrono::DateTime<Utc> {
            Utc.with_ymd_and_hms(2026, 9, 28, 1, 30, 0).unwrap()
        }
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        let (fixture, writer) = prospective_test_fixture("rows-real-v6", 2, false);
        // The real descriptor-pool fixture requires its own TEST_CODE leaf.
        // Relocate only this already closed Pre2 inode; preserve the fixed
        // Rows-owner fixture path and the constructor's original path guard.
        prospective_assert_fixture_offline(&fixture);
        let fixed_database = fixture.database();
        let pool_database = fixture.root.join("TEST_CODE_rows_v6.db");
        assert!(matches!(
            fs::symlink_metadata(&pool_database),
            Err(error) if error.kind() == io::ErrorKind::NotFound
        ));
        for suffix in ["-wal", "-shm", "-journal"] {
            assert!(matches!(
                fs::symlink_metadata(sidecar_path(&pool_database, suffix)),
                Err(error) if error.kind() == io::ErrorKind::NotFound
            ));
        }
        let pre2_identity = FileIdentity::from_metadata(&fs::metadata(&fixed_database).unwrap());
        assert!(pre2_identity.length > 0);
        let pre2_bytes = fs::read(&fixed_database).unwrap();
        fs::rename(&fixed_database, &pool_database).unwrap();
        assert!(matches!(
            fs::symlink_metadata(&fixed_database),
            Err(error) if error.kind() == io::ErrorKind::NotFound
        ));
        assert_eq!(
            FileIdentity::from_metadata(&fs::metadata(&pool_database).unwrap()),
            pre2_identity
        );
        assert_eq!(fs::read(&pool_database).unwrap(), pre2_bytes);
        let db = Arc::new(
            DatabaseManager::open_frozen_catalog_for_isolated_test(pool_database.clone()).unwrap(),
        );
        let ledger = PaperLedger::open(&db, &instant);
        let seed = SeedManifest {
            account_id: "TEST_CODE_ACCOUNT_G6".into(),
            epoch_id: "TEST_CODE_EPOCH_G6_V1".into(),
            command_id: "TEST_CODE_SEED_G6".into(),
            cutover_at: instant(),
            account_effective_at: instant(),
            positions_effective_at: instant(),
            source_reference: "TEST_CODE_explicit_seed".into(),
            source_hash: "a".repeat(64),
            approved_by: "TEST_CODE_explicit_approval".into(),
            cash: Money::from_cny(100_000.0).unwrap(),
            original_total: Money::from_cny(100_000.0).unwrap(),
            excluded_residual: None,
            lots: vec![],
            marks: vec![],
            policy: RiskPolicyV1::default(),
        };
        let binding = seed.binding().unwrap();
        ledger.apply(PaperCommand::Seed(seed)).unwrap();
        // Preserve a real nonempty original V1 position and its order audit,
        // not only an empty seed. This is the original isolated recipe.
        let view = ledger.read(&binding).unwrap();
        ledger
            .apply(PaperCommand::Execute(
                crate::trading::paper_ledger::ExecuteIntent {
                    price_intent: crate::trading::paper_ledger::PriceIntent::FixedSignalPriceV1,
                    binding: binding.clone(),
                    command_id: "TEST_CODE_G6_BUY_V1".into(),
                    expected_version: view.version,
                    inventory_fingerprint: view.inventory_fingerprint().unwrap(),
                    signal: crate::trading::paper_trade::PaperSignal {
                        plan_id: format!("paper:{}:TEST_CODE_G6_PLAN_V1", binding.epoch_id),
                        code: "TEST_CODE_000001".into(),
                        name: "fixture".into(),
                        direction: crate::trading::paper_trade::Direction::Buy,
                        price: 10.0,
                        quantity: 100,
                        virtual_reason: "TEST_CODE_evidence".into(),
                        is_limit_up: false,
                        is_limit_down: false,
                        is_suspended: false,
                        limit_up_price: Some(11.0),
                        limit_down_price: Some(9.0),
                        secondary_confirmed: false,
                        quote_observed_at: instant(),
                        risk_context: crate::trading::paper_trade::PaperRiskContext::new(
                            crate::risk::action_gate::AccountMode::Normal,
                            crate::monitor::data_mode::DataMode::Full,
                        ),
                    },
                    quote_price: Money::from_cny(10.0).unwrap(),
                    marks: vec![crate::trading::paper_ledger::Mark {
                        code: "TEST_CODE_000001".into(),
                        price: Money::from_cny(10.0).unwrap(),
                        observed_at: instant(),
                        source: "TEST_CODE_realtime".into(),
                    }],
                },
            ))
            .unwrap();

        let policy =
            crate::performance::fee_policy::AShareFeePolicyV2::fixed_compatibility_assumption();
        {
            let mut conn = db.get_conn().unwrap();
            crate::database::daily_change_review_schema_v1::create_schema(&mut conn).unwrap();
            conn.batch_execute("PRAGMA user_version=3").unwrap();
            crate::database::paper_book_owner_schema_v1::install_catalog_v4_for_isolated_test(
                &mut conn, &policy,
            )
            .unwrap();
            crate::database::paper_book_owner_schema_v2::install_catalog_v5_for_isolated_test(
                &mut conn,
            )
            .unwrap();
        }
        let old = crate::trading::paper_ledger::verified_v1_snapshot_on(
            &mut db.get_conn().unwrap(),
            &binding,
        )
        .unwrap();
        cutover_for_isolated_test(
            &db,
            &TestCutoverRequest {
                old_binding: binding.clone(),
                new_epoch_id: "TEST_CODE_EPOCH_G6_V2".into(),
                cutover_id: "TEST_CODE_CUTOVER_G6".into(),
                command_id: "TEST_CODE_GENESIS_G6".into(),
                expected_v1_version: old.version,
                expected_v1_head_hash: old.event_hash,
                expected_v1_projection_hash: old.projection_hash,
                reviewed_fee_policy: policy,
            },
            TestCutoverFault::None,
        )
        .unwrap();
        paper_v6::prepare_final_selection_for_isolated_v5_test(&db).unwrap();

        paper_v6::migrate_catalog6_for_isolated_test(&db).unwrap();
        {
            let mut session = paper_v6::paper_catalog6_session(&db).unwrap();
            session
                .with_immediate_catalog6(
                    |conn, _, proof| {
                        proof.validate_on(conn)?;
                        Ok::<_, paper_v6::PaperCatalog6Error>(())
                    },
                    |_, _, _, _| Ok(()),
                )
                .unwrap();
        }
        drop(ledger);
        let source = Arc::clone(db.attribution_connection_source.as_ref().unwrap());
        let mut pool_paths = fixture.binding();
        pool_paths.database = pool_database.clone();
        pool_paths.wal = sidecar_path(&pool_database, "-wal");
        pool_paths.shm = sidecar_path(&pool_database, "-shm");
        let namespace = PinnedNamespace::open(&pool_paths).unwrap();
        let sidecars = {
            let mut conn = db.get_conn().unwrap();
            conn.batch_execute("PRAGMA wal_checkpoint(TRUNCATE)")
                .unwrap();
            crate::database::registered_descriptor_connection_authority(&source, &mut conn)
                .unwrap();
            let evidence = crate::database::current_descriptor_pool_evidence(&source)
                .unwrap()
                .unwrap();
            let wal = pin_owner_created_sidecar(&namespace, &pool_database, "-wal").unwrap();
            let shm = pin_owner_created_sidecar(&namespace, &pool_database, "-shm").unwrap();
            assert_eq!(
                crate::database::FileObjectIdentity::from_file(&wal.file).unwrap(),
                evidence
                    .expected_objects
                    .identity(crate::database::SqliteObjectRole::Wal)
            );
            assert_eq!(
                crate::database::FileObjectIdentity::from_file(&shm.file).unwrap(),
                evidence
                    .expected_objects
                    .identity(crate::database::SqliteObjectRole::Shm)
            );
            let sidecars = OwnerCreatedSqliteSidecars { wal, shm };
            prospective::require_zero_owned_wal(&sidecars).unwrap();
            sidecars
        };
        assert_eq!(
            db.pool.state().connections,
            db.pool.state().idle_connections
        );
        let attribution = db.attribution_pool.as_ref().unwrap().state();
        assert_eq!(attribution.connections, attribution.idle_connections);
        assert_eq!(Arc::strong_count(&db), 1);
        drop(db);
        // A residual background r2d2 owner is a concrete unavailable fixture,
        // not permission to sleep/retry or remove unknown sidecars.
        assert_eq!(
            Arc::strong_count(&source),
            1,
            "all original source pool owners must actually close"
        );
        let readback = source.readback_connection.lock().unwrap().take();
        if let Some(readback) = readback {
            crate::database::release_descriptor_connection(&source, readback);
        }
        // r2d2's final idle-pool destruction closes native connections without
        // calling on_release. Its registry may retain stale raw-FD metadata;
        // prove actual closure rather than clearing or trusting that registry.
        fn require_exact_object_fds(
            snapshot: &crate::database::ProcessDescriptorSnapshot,
            identity: crate::database::FileObjectIdentity,
            known: &std::collections::BTreeSet<std::os::fd::RawFd>,
            role: &str,
        ) {
            for descriptor in known {
                assert_eq!(
                    snapshot.identity_of(*descriptor),
                    Some(identity),
                    "{role}: a concrete retained FD disappeared or changed identity"
                );
            }
            // Known-subset membership plus exact cardinality proves set
            // equality; an extra same-object FD is never accepted by count.
            assert_eq!(
                snapshot.count_matching(identity),
                known.len(),
                "{role}: extra native/unknown descriptors remain"
            );
        }
        let main_identity =
            crate::database::FileObjectIdentity::from_file(&source.database_anchor).unwrap();
        let wal_identity =
            crate::database::FileObjectIdentity::from_file(&sidecars.wal.file).unwrap();
        let shm_identity =
            crate::database::FileObjectIdentity::from_file(&sidecars.shm.file).unwrap();
        let main_pins = std::collections::BTreeSet::from([source.database_anchor.as_raw_fd()]);
        let wal_pins = std::collections::BTreeSet::from([sidecars.wal.file.as_raw_fd()]);
        let shm_pins = {
            let evidence = source.pool_evidence.lock().unwrap();
            let evidence = evidence.as_ref().unwrap();
            assert_eq!(
                main_identity,
                evidence
                    .expected_objects
                    .identity(crate::database::SqliteObjectRole::Main)
            );
            assert_eq!(
                wal_identity,
                evidence
                    .expected_objects
                    .identity(crate::database::SqliteObjectRole::Wal)
            );
            assert_eq!(
                shm_identity,
                evidence
                    .expected_objects
                    .identity(crate::database::SqliteObjectRole::Shm)
            );
            assert_eq!(
                crate::database::FileObjectIdentity::from_file(&evidence.shared_shm_anchor)
                    .unwrap(),
                shm_identity
            );
            std::collections::BTreeSet::from([
                evidence.shared_shm_anchor.as_raw_fd(),
                sidecars.shm.file.as_raw_fd(),
            ])
        };
        let source_pins_only = crate::database::ProcessDescriptorSnapshot::capture().unwrap();
        require_exact_object_fds(
            &source_pins_only,
            main_identity,
            &main_pins,
            "main/source pins",
        );
        require_exact_object_fds(
            &source_pins_only,
            wal_identity,
            &wal_pins,
            "WAL/fixture pin",
        );
        require_exact_object_fds(
            &source_pins_only,
            shm_identity,
            &shm_pins,
            "SHM/source and fixture pins",
        );
        drop(source);
        let fixture_pins_only = crate::database::ProcessDescriptorSnapshot::capture().unwrap();
        require_exact_object_fds(
            &fixture_pins_only,
            main_identity,
            &std::collections::BTreeSet::new(),
            "main/closed source",
        );
        require_exact_object_fds(
            &fixture_pins_only,
            wal_identity,
            &wal_pins,
            "WAL/fixture pin after source drop",
        );
        require_exact_object_fds(
            &fixture_pins_only,
            shm_identity,
            &std::collections::BTreeSet::from([sidecars.shm.file.as_raw_fd()]),
            "SHM/fixture pin after source drop",
        );
        sidecars
            .cleanup_after_connection_close(&namespace, &pool_database)
            .unwrap();
        require_no_live_sidecars_for_bound_namespace(&namespace, &pool_database).unwrap();
        let v6_identity = FileIdentity::from_metadata(&fs::metadata(&pool_database).unwrap());
        assert_eq!(v6_identity.device, pre2_identity.device);
        assert_eq!(v6_identity.inode, pre2_identity.inode);
        let v6_bytes = fs::read(&pool_database).unwrap();
        drop(namespace);
        // All original SQL owners closed; only the exact fixture pins remained
        // before their owned sidecar cleanup. No registry metadata was cleared.
        // Return this exact offline V6 inode to the fixed Rows-owner path.
        assert!(matches!(
            fs::symlink_metadata(&fixed_database),
            Err(error) if error.kind() == io::ErrorKind::NotFound
        ));
        prospective_assert_fixture_offline(&fixture);
        fs::rename(&pool_database, &fixed_database).unwrap();
        assert_eq!(
            FileIdentity::from_metadata(&fs::metadata(&fixed_database).unwrap()),
            v6_identity
        );
        assert_eq!(fs::read(&fixed_database).unwrap(), v6_bytes);
        assert!(matches!(
            fs::symlink_metadata(&pool_database),
            Err(error) if error.kind() == io::ErrorKind::NotFound
        ));
        for suffix in ["-wal", "-shm", "-journal"] {
            assert!(matches!(
                fs::symlink_metadata(sidecar_path(&pool_database, suffix)),
                Err(error) if error.kind() == io::ErrorKind::NotFound
            ));
        }
        prospective_assert_fixture_offline(&fixture);
        let before = fs::read(fixture.database()).unwrap();
        let rendered = rows_test_prepare(&fixture, &writer, rows::Options::production())
            .unwrap()
            .render_unapproved()
            .unwrap();
        let names = rows_test_names(&rows_test_report(&rendered));
        let expected = prospective_with_offline_fixture_connection(&fixture, |c| {
            c.prepare("SELECT name FROM sqlite_schema WHERE type='table' ORDER BY name")
                .unwrap()
                .query_map([], |r| r.get::<_, String>(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        });
        assert_eq!(names, expected);
        assert_eq!(fs::read(fixture.database()).unwrap(), before);
        assert!(
            names
                .iter()
                .filter(|n| n.starts_with("paper_book_v2_"))
                .count()
                > 4
        );
    }

    fn backup_test_prepare(
        fixture: &TestFixture,
        writer: &SelectionAuditWriter,
        options: backup::Options,
    ) -> Result<backup::VerifiedUnapprovedByteBackup, GlobalSchemaV1Error> {
        GlobalSchemaVersionOwner::for_test_code().prepare_backup_with_bound_paths(
            fixture.binding(),
            writer,
            GlobalSchemaCatalogMode::Test,
            options,
        )
    }
    fn backup_test_directory(fixture: &TestFixture) -> PathBuf {
        fixture
            .root
            .join("data/global-schema-operations/selection-byte-backup-v1")
    }
    fn backup_test_known_bytes(fixture: &TestFixture) -> Vec<(String, Vec<u8>)> {
        let root = backup_test_directory(fixture);
        [
            "000-intent.json",
            "001-created-db.json",
            "002-copied-db.json",
            "003-created-audit.json",
            "004-copied-audit.json",
            "005-backup-verified.json",
            "stock_analysis.db.backup",
            "selection-audit.jsonl.backup",
        ]
        .into_iter()
        .filter_map(|leaf| match fs::read(root.join(leaf)) {
            Ok(bytes) => Some((leaf.to_owned(), bytes)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => panic!("read actual fixture known leaf: {e}"),
        })
        .collect()
    }
    fn backup_test_append_audit(writer: &SelectionAuditWriter) {
        writer
            .append(SelectionAuditRecord::new(
                SelectionAuditPhase::Prepared,
                "TEST_CODE_REAL_BACKUP_AUDIT",
                "a".repeat(64),
                chrono::DateTime::parse_from_rfc3339("2026-07-29T00:02:00+08:00").unwrap(),
            ))
            .unwrap();
    }
    fn backup_test_stop(
        phase: backup::Phase,
        hit: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> backup::Options {
        let mut options = backup::Options::production();
        options.settings.hook = Some(Box::new(move |actual| {
            if actual == phase {
                hit.store(true, std::sync::atomic::Ordering::SeqCst);
                return Err(prospective::refusal(
                    "TEST_CODE scoped backup IO interruption",
                ));
            }
            Ok(())
        }));
        options
    }

    #[test]
    fn backup_prepare_actual_nonempty_bytes_audit_and_original_rows_are_preserved() {
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        let (fixture, writer) = prospective_test_fixture("backup-real-original", 0, false);
        prospective_with_offline_fixture_connection(&fixture, |connection| {
            connection.execute("INSERT INTO ledger(date,total_value,cash,market_value,daily_pnl,created_at) VALUES ('2026-09-28',123.25,12.25,111,0,'TEST_CODE_PRIVATE_BACKUP_ROW')", []).unwrap();
        });
        backup_test_append_audit(&writer);
        let row = prospective_ledger_row(&fixture);
        prospective_assert_fixture_offline(&fixture);
        let before = fs::read(fixture.database()).unwrap();
        let audit = fs::read(writer.path()).unwrap();
        let cap = backup_test_prepare(&fixture, &writer, backup::Options::production()).unwrap();
        assert!(matches!(
            fixture.acquire_exclusive(),
            Err(GlobalSchemaV1Error::ExclusiveProcessMaintenanceLeaseUnavailable)
        ));
        let rendered = cap.render_unapproved().unwrap();
        let report: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        assert_eq!(report["records"], 6);
        assert_eq!(report["approval"], "not_granted");
        assert_eq!(report["row_preservation_proof"], false);
        assert_eq!(report["apply_supported"], false);
        assert_eq!(
            report["database"]["sha256"],
            hex::encode(sha2::Sha256::digest(&before))
        );
        assert!(!rendered.contains("TEST_CODE_PRIVATE_BACKUP_ROW"));
        let root = backup_test_directory(&fixture);
        assert_eq!(
            fs::read(root.join("stock_analysis.db.backup")).unwrap(),
            before
        );
        assert_eq!(
            fs::read(root.join("selection-audit.jsonl.backup")).unwrap(),
            audit
        );
        assert_eq!(fs::read(fixture.database()).unwrap(), before);
        assert_eq!(fs::read(writer.path()).unwrap(), audit);
        assert_eq!(prospective_ledger_row(&fixture), row);
        let restored = Connection::open(root.join("stock_analysis.db.backup")).unwrap();
        assert_eq!(
            restored
                .query_row("SELECT created_at FROM ledger", [], |r| r
                    .get::<_, String>(0))
                .unwrap(),
            "TEST_CODE_PRIVATE_BACKUP_ROW"
        );
        drop(restored);
        drop(fixture.acquire_exclusive().unwrap());
    }

    #[test]
    fn backup_prepare_absent_and_present_empty_audit_have_distinct_real_branches() {
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        for present in [false, true] {
            let (fixture, writer) = prospective_test_fixture("backup-audit-branch", 0, false);
            if present {
                fs::write(writer.path(), []).unwrap();
            }
            let rendered = backup_test_prepare(&fixture, &writer, backup::Options::production())
                .unwrap()
                .render_unapproved()
                .unwrap();
            let report: serde_json::Value = serde_json::from_str(&rendered).unwrap();
            assert_eq!(report["records"], if present { 6 } else { 4 });
            assert_eq!(
                backup_test_directory(&fixture)
                    .join("selection-audit.jsonl.backup")
                    .exists(),
                present
            );
            assert_eq!(report["audit"].is_null(), !present);
            if present {
                assert_eq!(report["audit"]["length"], 0);
            }
        }
    }

    #[test]
    fn backup_prepare_extended_actual_catalog_families_remain_unqualified() {
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        for generation in 2..=5 {
            for amended in [false, true] {
                let (fixture, writer) =
                    prospective_test_fixture("backup-family", generation, amended);
                let before = fs::read(fixture.database()).unwrap();
                let rendered =
                    backup_test_prepare(&fixture, &writer, backup::Options::production())
                        .unwrap()
                        .render_unapproved()
                        .unwrap();
                assert_eq!(fs::read(fixture.database()).unwrap(), before);
                assert!(rendered.contains("not_granted"));
                assert!(rendered.contains("\"apply_supported\":false"));
                let intent: serde_json::Value = serde_json::from_slice(
                    &fs::read(backup_test_directory(&fixture).join("000-intent.json")).unwrap(),
                )
                .unwrap();
                let binding: serde_json::Value = serde_json::from_str(
                    intent["record"]["transition"]["source_canonical"]
                        .as_str()
                        .unwrap(),
                )
                .unwrap();
                assert_eq!(binding["original_user_version"], generation);
                assert_eq!(binding["target"]["user_version"], generation);
            }
        }
    }

    #[test]
    fn backup_prepare_real_checkpoint_restarts_only_original_recorded_inodes() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        for phase in [
            backup::Phase::AfterRecordParentSync(0),
            backup::Phase::AfterCreated(backup::Role::Database),
            backup::Phase::AfterCopyChunk(backup::Role::Database),
            backup::Phase::AfterRoleSync(backup::Role::Database),
            backup::Phase::AfterCopied(backup::Role::Database),
            backup::Phase::AfterTerminalSync,
        ] {
            let (fixture, writer) = prospective_test_fixture("backup-checkpoint", 0, false);
            let before = fs::read(fixture.database()).unwrap();
            let hit = Arc::new(AtomicBool::new(false));
            assert!(backup_test_prepare(
                &fixture,
                &writer,
                backup_test_stop(phase, Arc::clone(&hit))
            )
            .is_err());
            assert!(hit.load(Ordering::SeqCst), "{phase:?}");
            assert_eq!(fs::read(fixture.database()).unwrap(), before);
            let root = backup_test_directory(&fixture);
            let intent_bytes = fs::read(root.join("000-intent.json")).unwrap();
            let original_anchor: serde_json::Value = serde_json::from_slice(&intent_bytes).unwrap();
            let original_anchor = original_anchor["record"]["directory"].clone();
            let intent_inode = fs::metadata(root.join("000-intent.json")).unwrap().ino();
            let role_inode = fs::metadata(root.join("stock_analysis.db.backup"))
                .ok()
                .map(|m| m.ino());
            let cap =
                backup_test_prepare(&fixture, &writer, backup::Options::production()).unwrap();
            cap.render_unapproved().unwrap();
            assert_eq!(
                fs::metadata(root.join("000-intent.json")).unwrap().ino(),
                intent_inode
            );
            if let Some(inode) = role_inode {
                assert_eq!(
                    fs::metadata(root.join("stock_analysis.db.backup"))
                        .unwrap()
                        .ino(),
                    inode
                );
            }
            assert_eq!(
                fs::read(root.join("stock_analysis.db.backup")).unwrap(),
                before
            );
            assert_eq!(
                fs::read(root.join("000-intent.json")).unwrap(),
                intent_bytes
            );
            for (leaf, bytes) in backup_test_known_bytes(&fixture) {
                if leaf.ends_with(".json") {
                    let wire: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                    assert_eq!(wire["record"]["directory"], original_anchor);
                }
            }
        }
    }

    #[test]
    fn backup_prepare_empty_inode_sync_and_created_failures_prevent_first_copy() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        for phase in [
            backup::Phase::BeforeEmptyRoleSync(backup::Role::Database),
            backup::Phase::AfterEmptyRoleSync(backup::Role::Database),
            backup::Phase::AfterEmptyRoleParentSync(backup::Role::Database),
            backup::Phase::BeforeCreated(backup::Role::Database),
            backup::Phase::BeforeRecordSync(1),
            backup::Phase::AfterRecordSync(1),
            backup::Phase::AfterRecordReadback(1),
        ] {
            let (fixture, writer) = prospective_test_fixture("backup-empty-sync-fault", 0, false);
            let before = fs::read(fixture.database()).unwrap();
            let hit = Arc::new(AtomicBool::new(false));
            assert!(backup_test_prepare(
                &fixture,
                &writer,
                backup_test_stop(phase, Arc::clone(&hit))
            )
            .is_err());
            assert!(hit.load(Ordering::SeqCst));
            let root = backup_test_directory(&fixture);
            assert_eq!(
                fs::metadata(root.join("stock_analysis.db.backup"))
                    .unwrap()
                    .len(),
                0
            );
            assert!(!root.join("002-copied-db.json").exists());
            assert!(!root.join("005-backup-verified.json").exists());
            assert_eq!(fs::read(fixture.database()).unwrap(), before);
        }
    }

    #[test]
    fn backup_prepare_unrecorded_empty_role_and_partial_journal_gaps_are_preserved() {
        use std::sync::atomic::AtomicBool;
        use std::sync::Arc;
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        for journal_gap in [false, true] {
            let (fixture, writer) = prospective_test_fixture("backup-unrecorded-gap", 0, false);
            let phase = if journal_gap {
                backup::Phase::BeforeRecordSync(0)
            } else {
                backup::Phase::BeforeCreated(backup::Role::Database)
            };
            assert!(backup_test_prepare(
                &fixture,
                &writer,
                backup_test_stop(phase, Arc::new(AtomicBool::new(false)))
            )
            .is_err());
            let root = backup_test_directory(&fixture);
            if journal_gap {
                let path = root.join("000-intent.json");
                let file = OpenOptions::new().write(true).open(path).unwrap();
                file.set_len(7).unwrap();
                file.sync_all().unwrap();
            }
            let before = backup_test_known_bytes(&fixture);
            let original = fs::read(fixture.database()).unwrap();
            assert!(backup_test_prepare(&fixture, &writer, backup::Options::production()).is_err());
            assert_eq!(backup_test_known_bytes(&fixture), before);
            assert_eq!(fs::read(fixture.database()).unwrap(), original);
        }
    }

    #[test]
    fn backup_prepare_created_only_overlong_inode_is_never_truncated() {
        use std::sync::atomic::AtomicBool;
        use std::sync::Arc;
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        let (fixture, writer) = prospective_test_fixture("backup-overlong-created", 0, false);
        let original = fs::read(fixture.database()).unwrap();
        assert!(backup_test_prepare(
            &fixture,
            &writer,
            backup_test_stop(
                backup::Phase::AfterCreated(backup::Role::Database),
                Arc::new(AtomicBool::new(false))
            )
        )
        .is_err());
        let path = backup_test_directory(&fixture).join("stock_analysis.db.backup");
        let file = OpenOptions::new().write(true).open(&path).unwrap();
        file.set_len(original.len() as u64 + 1).unwrap();
        file.sync_all().unwrap();
        drop(file);
        let before = backup_test_known_bytes(&fixture);
        assert!(backup_test_prepare(&fixture, &writer, backup::Options::production()).is_err());
        assert_eq!(fs::metadata(path).unwrap().len(), original.len() as u64 + 1);
        assert_eq!(backup_test_known_bytes(&fixture), before);
    }

    #[test]
    fn backup_prepare_all_immutable_record_same_bytes_replacements_refuse_restart() {
        use std::os::unix::fs::PermissionsExt;
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        for leaf in [
            "000-intent.json",
            "001-created-db.json",
            "002-copied-db.json",
            "005-backup-verified.json",
        ] {
            let (fixture, writer) = prospective_test_fixture("backup-record-replaced", 0, false);
            backup_test_prepare(&fixture, &writer, backup::Options::production())
                .unwrap()
                .render_unapproved()
                .unwrap();
            let path = backup_test_directory(&fixture).join(leaf);
            let displaced = path.with_extension("owned-displaced");
            let bytes = fs::read(&path).unwrap();
            let inode = fs::metadata(&path).unwrap().ino();
            fs::rename(&path, &displaced).unwrap();
            fs::write(&path, &bytes).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            assert_ne!(fs::metadata(&path).unwrap().ino(), inode);
            let before = backup_test_known_bytes(&fixture);
            assert!(backup_test_prepare(&fixture, &writer, backup::Options::production()).is_err());
            assert_eq!(backup_test_known_bytes(&fixture), before);
            assert_eq!(fs::read(displaced).unwrap(), bytes);
        }
    }

    #[test]
    fn backup_prepare_committed_role_missing_replaced_and_hardlinked_never_heal() {
        use std::os::unix::fs::PermissionsExt;
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        for attack in ["missing", "replacement", "hardlink"] {
            let (fixture, writer) = prospective_test_fixture("backup-role-attack", 0, false);
            backup_test_prepare(&fixture, &writer, backup::Options::production())
                .unwrap()
                .render_unapproved()
                .unwrap();
            let path = backup_test_directory(&fixture).join("stock_analysis.db.backup");
            let bytes = fs::read(&path).unwrap();
            match attack {
                "missing" => fs::remove_file(&path).unwrap(),
                "replacement" => {
                    fs::rename(&path, path.with_extension("original")).unwrap();
                    fs::write(&path, &bytes).unwrap();
                    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
                }
                _ => fs::hard_link(&path, path.with_extension("extra-link")).unwrap(),
            }
            let before = backup_test_known_bytes(&fixture);
            assert!(backup_test_prepare(&fixture, &writer, backup::Options::production()).is_err());
            assert_eq!(backup_test_known_bytes(&fixture), before);
        }
    }

    #[test]
    fn backup_prepare_terminal_source_or_audit_drift_never_rotates_operation() {
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        for audit_drift in [false, true] {
            let (fixture, writer) = prospective_test_fixture("backup-terminal-drift", 0, false);
            backup_test_append_audit(&writer);
            backup_test_prepare(&fixture, &writer, backup::Options::production())
                .unwrap()
                .render_unapproved()
                .unwrap();
            let outputs = backup_test_known_bytes(&fixture);
            if audit_drift {
                writer
                    .append(SelectionAuditRecord::new(
                        SelectionAuditPhase::Prepared,
                        "TEST_CODE_NEXT_AUDIT_BINDING",
                        "b".repeat(64),
                        chrono::DateTime::parse_from_rfc3339("2026-07-29T00:03:00+08:00").unwrap(),
                    ))
                    .unwrap();
            } else {
                let connection = Connection::open(fixture.database()).unwrap();
                connection.execute("INSERT INTO ledger(date,total_value,cash,market_value,daily_pnl,created_at) VALUES ('2026-09-29',1,1,0,0,'TEST_CODE_NEW_SOURCE')", []).unwrap();
                drop(connection);
            }
            let before = fs::read(fixture.database()).unwrap();
            let audit = fs::read(writer.path()).unwrap();
            assert!(backup_test_prepare(&fixture, &writer, backup::Options::production()).is_err());
            assert_eq!(backup_test_known_bytes(&fixture), outputs);
            assert_eq!(fs::read(fixture.database()).unwrap(), before);
            assert_eq!(fs::read(writer.path()).unwrap(), audit);
        }
    }

    #[test]
    fn backup_prepare_exact_terminal_replay_has_no_new_records_or_bytes() {
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        let (fixture, writer) = prospective_test_fixture("backup-terminal-replay", 0, false);
        backup_test_prepare(&fixture, &writer, backup::Options::production())
            .unwrap()
            .render_unapproved()
            .unwrap();
        let outputs = backup_test_known_bytes(&fixture);
        let original = fs::read(fixture.database()).unwrap();
        let replay = backup_test_prepare(&fixture, &writer, backup::Options::production()).unwrap();
        replay.render_unapproved().unwrap();
        assert_eq!(backup_test_known_bytes(&fixture), outputs);
        assert_eq!(fs::read(fixture.database()).unwrap(), original);
    }

    #[test]
    fn backup_prepare_directory_entry_growth_and_restart_keep_original_anchor() {
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        let (fixture, writer) = prospective_test_fixture("backup-directory-growth", 0, false);
        let operation = backup_test_directory(&fixture);
        let initial = std::rc::Rc::new(std::cell::RefCell::new(None));
        let hook_initial = initial.clone();
        let hook_path = operation.clone();
        let mut options = backup::Options::production();
        options.settings.hook = Some(Box::new(move |phase| {
            if phase == backup::Phase::AfterDirectoryCreated {
                let m = fs::symlink_metadata(&hook_path).unwrap();
                *hook_initial.borrow_mut() =
                    Some((m.dev(), m.ino(), m.uid(), m.mode() & 0o7777, m.nlink()));
            }
            Ok(())
        }));
        backup_test_prepare(&fixture, &writer, options)
            .unwrap()
            .render_unapproved()
            .unwrap();
        let initial = initial.borrow().as_ref().copied().unwrap();
        let actual = fs::symlink_metadata(&operation).unwrap();
        assert!(actual.is_dir() && actual.nlink() > 0);
        assert_eq!(
            (
                actual.dev(),
                actual.ino(),
                actual.uid(),
                actual.mode() & 0o7777
            ),
            (initial.0, initial.1, initial.2, initial.3)
        );
        let outputs = backup_test_known_bytes(&fixture);
        let original = fs::read(fixture.database()).unwrap();
        let first: serde_json::Value =
            serde_json::from_slice(&fs::read(operation.join("000-intent.json")).unwrap()).unwrap();
        let anchor = first["record"]["directory"].clone();
        let directory = &anchor["operation"];
        assert_eq!(directory["device"].as_u64(), Some(initial.0));
        assert_eq!(directory["inode"].as_u64(), Some(initial.1));
        assert_eq!(directory["owner"].as_u64(), Some(u64::from(initial.2)));
        assert_eq!(directory["mode"].as_u64(), Some(u64::from(initial.3)));
        assert_eq!(directory["links"].as_u64(), Some(initial.4));
        for (leaf, bytes) in &outputs {
            if leaf.ends_with(".json") {
                let wire: serde_json::Value = serde_json::from_slice(bytes).unwrap();
                assert_eq!(wire["record"]["version"], 1);
                assert_eq!(wire["record"]["directory"], anchor);
            }
        }
        // A new owner observes the larger live directory but must bind only
        // the original complete Intent and must not re-encode its old facts.
        backup_test_prepare(&fixture, &writer, backup::Options::production())
            .unwrap()
            .render_unapproved()
            .unwrap();
        assert_eq!(backup_test_known_bytes(&fixture), outputs);
        assert_eq!(fs::read(fixture.database()).unwrap(), original);
    }

    #[test]
    fn backup_prepare_owned_namespace_symlink_mode_and_role_gaps_fail_closed() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        for attack in ["operation-mode", "role-symlink", "existing-empty-operation"] {
            let (fixture, writer) = prospective_test_fixture("backup-namespace-attack", 0, false);
            if attack == "existing-empty-operation" {
                let root = backup_test_directory(&fixture);
                fs::create_dir_all(&root).unwrap();
                fs::set_permissions(root.parent().unwrap(), fs::Permissions::from_mode(0o700))
                    .unwrap();
                fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
            } else {
                backup_test_prepare(&fixture, &writer, backup::Options::production())
                    .unwrap()
                    .render_unapproved()
                    .unwrap();
                let root = backup_test_directory(&fixture);
                if attack == "operation-mode" {
                    fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
                } else {
                    fs::remove_file(root.join("stock_analysis.db.backup")).unwrap();
                    symlink(fixture.database(), root.join("stock_analysis.db.backup")).unwrap();
                }
            }
            let before = fs::read(fixture.database()).unwrap();
            assert!(backup_test_prepare(&fixture, &writer, backup::Options::production()).is_err());
            assert_eq!(fs::read(fixture.database()).unwrap(), before);
        }
    }

    #[test]
    fn backup_prepare_tight_copy_extent_and_record_budgets_preserve_source() {
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        for cap in [
            "main",
            "audit",
            "roles",
            "copy",
            "intent",
            "record-total",
            "record-count",
            "journal",
            "common",
            "raise",
        ] {
            let (fixture, writer) = prospective_test_fixture("backup-budget", 0, false);
            backup_test_append_audit(&writer);
            let before = fs::read(fixture.database()).unwrap();
            let audit = fs::read(writer.path()).unwrap();
            let mut options = backup::Options::production();
            match cap {
                "main" => options.settings.limits.main_extent = before.len() as u64 - 1,
                "audit" => options.settings.limits.audit_extent = audit.len() as u64 - 1,
                "roles" => {
                    options.settings.limits.role_total =
                        before.len() as u64 + audit.len() as u64 - 1
                }
                "copy" => options.settings.limits.copy_work = before.len() as u64 * 2 - 1,
                "intent" => options.settings.limits.intent_bytes = 8,
                "record-total" => options.settings.limits.record_total = 8,
                "record-count" => options.settings.limits.record_count = 1,
                "journal" => options.settings.limits.journal_work = 8,
                "common" => options.settings.limits.common_work = 0,
                _ => options.settings.limits.main_extent += 1,
            }
            assert!(
                backup_test_prepare(&fixture, &writer, options).is_err(),
                "{cap}"
            );
            assert_eq!(fs::read(fixture.database()).unwrap(), before);
            assert_eq!(fs::read(writer.path()).unwrap(), audit);
            if cap == "copy" {
                let root = backup_test_directory(&fixture);
                assert_eq!(
                    fs::metadata(root.join("stock_analysis.db.backup"))
                        .unwrap()
                        .len(),
                    0
                );
                assert!(!root.join("002-copied-db.json").exists());
            }
        }
    }

    #[test]
    fn backup_prepare_restart_and_render_charge_existing_physical_and_journal_work() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        let (fixture, writer) = prospective_test_fixture("backup-restart-budget", 0, false);
        backup_test_prepare(&fixture, &writer, backup::Options::production())
            .unwrap()
            .render_unapproved()
            .unwrap();
        let before = backup_test_known_bytes(&fixture);
        let total = before
            .iter()
            .filter(|(leaf, _)| leaf.ends_with(".json"))
            .map(|(_, b)| b.len() as u64)
            .sum::<u64>();
        let mut options = backup::Options::production();
        options.settings.limits.journal_work = total * 3 + 512;
        assert!(backup_test_prepare(&fixture, &writer, options).is_err());
        assert_eq!(backup_test_known_bytes(&fixture), before);
        let (fresh, writer) = prospective_test_fixture("backup-render-budget", 0, false);
        let length = fs::metadata(fresh.database()).unwrap().len();
        let hit = Arc::new(AtomicBool::new(false));
        let marker = Arc::clone(&hit);
        let mut options = backup::Options::production();
        options.source.max_total_hash_bytes = length * 18 + 2;
        options.settings.hook = Some(Box::new(move |phase| {
            if phase == backup::Phase::BeforeRender {
                marker.store(true, Ordering::SeqCst);
            }
            Ok(())
        }));
        let cap = backup_test_prepare(&fresh, &writer, options).unwrap();
        assert!(cap.render_unapproved().is_err());
        assert!(hit.load(Ordering::SeqCst));
        assert!(backup_test_directory(&fresh)
            .join("005-backup-verified.json")
            .exists());
    }

    #[test]
    fn backup_prepare_after_terminal_and_render_faults_preserve_synced_facts() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        for phase in [
            backup::Phase::AfterTerminalSync,
            backup::Phase::BeforeRender,
        ] {
            let (fixture, writer) = prospective_test_fixture("backup-post-sync-fault", 0, false);
            let before = fs::read(fixture.database()).unwrap();
            let hit = Arc::new(AtomicBool::new(false));
            assert!(backup_test_prepare(
                &fixture,
                &writer,
                backup_test_stop(phase, Arc::clone(&hit))
            )
            .and_then(backup::VerifiedUnapprovedByteBackup::render_unapproved)
            .is_err());
            assert!(hit.load(Ordering::SeqCst));
            let outputs = backup_test_known_bytes(&fixture);
            assert!(backup_test_directory(&fixture)
                .join("005-backup-verified.json")
                .exists());
            backup_test_prepare(&fixture, &writer, backup::Options::production())
                .unwrap()
                .render_unapproved()
                .unwrap();
            assert_eq!(backup_test_known_bytes(&fixture), outputs);
            assert_eq!(fs::read(fixture.database()).unwrap(), before);
        }
    }

    #[test]
    fn backup_prepare_source_mutation_after_copy_cannot_write_terminal() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        let (fixture, writer) = prospective_test_fixture("backup-source-tail", 0, false);
        let path = fixture.database();
        let before = fs::read(&path).unwrap();
        let hit = Arc::new(AtomicBool::new(false));
        let marker = Arc::clone(&hit);
        let mut options = backup::Options::production();
        options.settings.hook = Some(Box::new(move |phase| {
            if phase == backup::Phase::AfterCopied(backup::Role::Database) {
                let mut bytes = fs::read(&path).unwrap();
                let n = bytes.len();
                bytes[n - 1] ^= 1;
                fs::write(&path, bytes).unwrap();
                marker.store(true, Ordering::SeqCst);
            }
            Ok(())
        }));
        assert!(backup_test_prepare(&fixture, &writer, options).is_err());
        assert!(hit.load(Ordering::SeqCst));
        assert!(!backup_test_directory(&fixture)
            .join("005-backup-verified.json")
            .exists());
        assert_eq!(
            fs::read(backup_test_directory(&fixture).join("stock_analysis.db.backup")).unwrap(),
            before
        );
        // The external fixture attack is a real persisted mutation; the owner
        // neither rolls it back nor falsely attests the changed source.
        fs::write(fixture.database(), before).unwrap();
    }

    #[test]
    fn backup_prepare_actual_shared_owner_blocks_before_operation_creation() {
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        let (fixture, writer) = prospective_test_fixture("backup-shared-owner", 1, true);
        let shared = fixture.inspect().unwrap();
        assert!(matches!(
            backup_test_prepare(&fixture, &writer, backup::Options::production()),
            Err(GlobalSchemaV1Error::SharedToExclusiveUpgradeForbidden)
        ));
        assert!(!backup_test_directory(&fixture).exists());
        drop(shared);
    }

    #[test]
    fn backup_prepare_actual_child_flock_blocks_before_operation_creation() {
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        let (fixture, writer) = prospective_test_fixture("backup-child-held", 1, true);
        let before = fs::read(fixture.database()).unwrap();
        let shared = fixture.inspect().unwrap();
        drop(shared);
        let mut child = Command::new(std::env::current_exe().unwrap())
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
            .unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let mut ready = false;
        for _ in 0..20 {
            let mut line = String::new();
            if stdout.read_line(&mut line).unwrap() == 0 {
                break;
            }
            if line.contains("TEST_CODE_GLOBAL_SCHEMA_SHARED_LOCKED") {
                ready = true;
                break;
            }
        }
        assert!(ready);
        let result = backup_test_prepare(&fixture, &writer, backup::Options::production());
        drop(child.stdin.take());
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(matches!(
            result,
            Err(GlobalSchemaV1Error::ExclusiveMaintenanceLeaseUnavailable {
                retryable: true,
                ..
            })
        ));
        assert!(!backup_test_directory(&fixture).exists());
        assert_eq!(fs::read(fixture.database()).unwrap(), before);
    }

    #[test]
    fn backup_prepare_exact_codec_and_predecessor_corruption_cannot_recover_prefix() {
        use sha2::{Digest, Sha256};
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        for attack in ["duplicate", "noncanonical", "predecessor"] {
            let (fixture, writer) = prospective_test_fixture("backup-journal-codec", 0, false);
            backup_test_prepare(&fixture, &writer, backup::Options::production())
                .unwrap()
                .render_unapproved()
                .unwrap();
            let path = backup_test_directory(&fixture).join("002-copied-db.json");
            let mut wire = String::from_utf8(fs::read(&path).unwrap()).unwrap();
            if attack == "duplicate" {
                wire = wire.replacen("\"version\":1", "\"version\":1,\"version\":1", 1);
            } else if attack == "noncanonical" {
                wire = wire.replacen("\"version\":1", "\"version\": 1", 1);
            } else {
                let start = wire.find("\"predecessor\":\"").unwrap() + "\"predecessor\":\"".len();
                wire.replace_range(start..start + 64, &"f".repeat(64));
                // Preserve field order and exact closed codec while producing
                // a cryptographically self-consistent wrong predecessor.
                let body_start = wire.find("\"record\":").unwrap() + "\"record\":".len();
                let mut canonical = wire[body_start..wire.len() - 2].as_bytes().to_vec();
                canonical.push(b'\n');
                let mut digest = Sha256::new();
                digest.update(b"stock_analysis.global_schema.byte_backup_record.v1");
                digest.update([0]);
                digest.update(&canonical);
                let new_hash = hex::encode(digest.finalize());
                let hash_start = "{\"sha256\":\"".len();
                wire.replace_range(hash_start..hash_start + 64, &new_hash);
            }
            fs::write(&path, wire.as_bytes()).unwrap();
            let before = backup_test_known_bytes(&fixture);
            assert!(
                backup_test_prepare(&fixture, &writer, backup::Options::production()).is_err(),
                "{attack}"
            );
            assert_eq!(backup_test_known_bytes(&fixture), before);
        }
    }

    #[test]
    fn backup_prepare_last_output_read_faults_reject_changed_bytes_and_record_inode() {
        use std::os::unix::fs::PermissionsExt;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        for replace_record in [false, true] {
            let (fixture, writer) = prospective_test_fixture("backup-output-tail", 0, false);
            let original = fs::read(fixture.database()).unwrap();
            let root = backup_test_directory(&fixture);
            let hit = Arc::new(AtomicBool::new(false));
            let marker = Arc::clone(&hit);
            let mut options = backup::Options::production();
            options.settings.hook = Some(Box::new(move |phase| {
                if phase == backup::Phase::AfterOutputReads && !marker.swap(true, Ordering::SeqCst)
                {
                    let path = root.join(if replace_record {
                        "000-intent.json"
                    } else {
                        "stock_analysis.db.backup"
                    });
                    let mut bytes = fs::read(&path).unwrap();
                    if replace_record {
                        fs::rename(&path, path.with_extension("original-owned")).unwrap();
                        fs::write(&path, &bytes).unwrap();
                        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
                    } else {
                        let n = bytes.len();
                        bytes[n - 1] ^= 1;
                        fs::write(&path, &bytes).unwrap();
                    }
                }
                Ok(())
            }));
            assert!(backup_test_prepare(&fixture, &writer, options).is_err());
            assert!(hit.load(Ordering::SeqCst));
            assert!(!backup_test_directory(&fixture)
                .join("005-backup-verified.json")
                .exists());
            assert_eq!(fs::read(fixture.database()).unwrap(), original);
        }
    }

    #[test]
    fn backup_prepare_absent_audit_records_appearing_at_output_tail_are_rejected() {
        use std::os::unix::fs::PermissionsExt;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        for leaf in ["003-created-audit.json", "004-copied-audit.json"] {
            let (fixture, writer) = prospective_test_fixture("backup-absent-audit-tail", 0, false);
            assert!(!writer.path().exists());
            let original = fs::read(fixture.database()).unwrap();
            let root = backup_test_directory(&fixture);
            let injected = root.join(leaf);
            let hit = Arc::new(AtomicBool::new(false));
            let marker = Arc::clone(&hit);
            let mut options = backup::Options::production();
            options.settings.hook = Some(Box::new(move |phase| {
                if phase == backup::Phase::AfterOutputReads && !marker.swap(true, Ordering::SeqCst)
                {
                    fs::write(root.join(leaf), b"TEST_CODE_UNRECORDED_AUDIT_RECORD").unwrap();
                    fs::set_permissions(root.join(leaf), fs::Permissions::from_mode(0o600))
                        .unwrap();
                }
                Ok(())
            }));
            assert!(backup_test_prepare(&fixture, &writer, options).is_err());
            assert!(hit.load(Ordering::SeqCst));
            assert_eq!(
                fs::read(injected).unwrap(),
                b"TEST_CODE_UNRECORDED_AUDIT_RECORD"
            );
            assert!(!backup_test_directory(&fixture)
                .join("005-backup-verified.json")
                .exists());
            assert_eq!(fs::read(fixture.database()).unwrap(), original);
            let preserved = backup_test_known_bytes(&fixture);
            assert!(backup_test_prepare(&fixture, &writer, backup::Options::production()).is_err());
            assert_eq!(backup_test_known_bytes(&fixture), preserved);
            assert_eq!(fs::read(fixture.database()).unwrap(), original);
        }
    }

    #[test]
    fn backup_prepare_same_operation_inode_under_replaced_managed_ancestor_refuses() {
        use std::os::unix::fs::PermissionsExt;
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        let (fixture, writer) = prospective_test_fixture("backup-parent-rebind", 0, false);
        backup_test_prepare(&fixture, &writer, backup::Options::production())
            .unwrap()
            .render_unapproved()
            .unwrap();
        let op = backup_test_directory(&fixture);
        let parent = op.parent().unwrap().to_path_buf();
        let displaced = parent.with_extension("original-owned");
        let before = backup_test_known_bytes(&fixture);
        let op_inode = fs::metadata(&op).unwrap().ino();
        let original = fs::read(fixture.database()).unwrap();
        fs::rename(&parent, &displaced).unwrap();
        fs::create_dir(&parent).unwrap();
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
        fs::rename(displaced.join("selection-byte-backup-v1"), &op).unwrap();
        assert_eq!(fs::metadata(&op).unwrap().ino(), op_inode);
        assert!(backup_test_prepare(&fixture, &writer, backup::Options::production()).is_err());
        assert_eq!(backup_test_known_bytes(&fixture), before);
        assert_eq!(fs::read(fixture.database()).unwrap(), original);
    }

    #[test]
    fn backup_prepare_cli_conflicts_preserve_original_production_apply_blocker() {
        for arguments in [
            vec!["--prepare-backup", "--prepare"],
            vec!["--prepare-backup", "--test"],
            vec!["--prepare-backup", "--apply"],
            vec!["--prepare-backup", "--prepare-backup"],
            vec!["--prepare-backup", "--root=/tmp"],
            vec!["--prepare-backup", "--approval=true"],
            vec!["--prepare-backup", "--help"],
        ] {
            assert!(run_selection_v2_migration_command(arguments).is_err());
        }
        assert_eq!(
            run_selection_v2_migration_command(["--apply"]).unwrap_err(),
            super::super::selection_v2::SELECTION_V2_APPLY_BLOCKER
        );
        assert!(run_selection_v2_migration_command(["--help"])
            .unwrap()
            .contains("--prepare-backup"));
    }

    fn prospective_test_prepare(
        fixture: &TestFixture,
        writer: &SelectionAuditWriter,
        options: prospective::Options,
    ) -> Result<prospective::PreparedGlobalSchemaProspective, GlobalSchemaV1Error> {
        GlobalSchemaVersionOwner::for_test_code().prepare_selection_with_bound_paths(
            fixture.binding(),
            writer,
            GlobalSchemaCatalogMode::Test,
            options,
        )
    }

    // Synthetic fixture writes and row reads must return the source to the
    // offline state before the prospective owner freezes its main bytes.
    fn prospective_with_offline_fixture_connection<T>(
        fixture: &TestFixture,
        operation: impl FnOnce(&Connection) -> T,
    ) -> T {
        let maintenance = fixture.acquire_exclusive().unwrap();
        let database = fixture.database();
        require_no_live_sidecars_for_bound_namespace(&maintenance.namespace, &database)
            .expect("synthetic row operation starts with an offline fixture");
        let (main, identity) = open_pinned_regular_read_write(
            &maintenance.namespace.database_parent,
            &maintenance.namespace.database_leaf,
            &database,
        )
        .unwrap();
        let connection = open_pinned_sqlite_read_write(
            &maintenance.namespace.database_parent,
            &maintenance.namespace.database_leaf,
            &main,
            identity,
            &database,
        )
        .unwrap();
        let sidecars = OwnerCreatedSqliteSidecars::materialize_and_pin(
            &connection,
            &maintenance.namespace,
            &database,
        )
        .unwrap();
        prospective::require_zero_owned_wal(&sidecars).unwrap();
        let result = operation(&connection);
        let checkpoint: (i64, i64, i64) = connection
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .expect("checkpoint synthetic fixture row operation");
        assert_eq!(checkpoint.0, 0, "synthetic fixture checkpoint is not busy");
        assert!(checkpoint.1 >= 0 && checkpoint.2 >= 0);
        assert_eq!(checkpoint.1, checkpoint.2);
        prospective::require_zero_owned_wal(&sidecars).unwrap();
        sidecars
            .validate_present_exact(&maintenance.namespace, &database)
            .unwrap();
        connection
            .close()
            .expect("synthetic fixture connection closes successfully");
        sidecars
            .cleanup_after_connection_close(&maintenance.namespace, &database)
            .expect("remove only original pinned, closed fixture sidecars");
        require_no_live_sidecars_for_bound_namespace(&maintenance.namespace, &database)
            .expect("synthetic fixture is offline after exact sidecar cleanup");
        result
    }

    fn prospective_assert_fixture_offline(fixture: &TestFixture) {
        for suffix in ["-wal", "-shm", "-journal"] {
            assert!(matches!(
                fs::symlink_metadata(sidecar_path(&fixture.database(), suffix)),
                Err(error) if error.kind() == io::ErrorKind::NotFound
            ));
        }
    }

    fn prospective_ledger_row(fixture: &TestFixture) -> Vec<rusqlite::types::Value> {
        prospective_with_offline_fixture_connection(fixture, |connection| {
            connection
                .query_row(
                    "SELECT id,date,total_value,cash,market_value,daily_pnl,created_at FROM ledger",
                    [],
                    |row| (0..7).map(|column| row.get(column)).collect(),
                )
                .unwrap()
        })
    }

    #[test]
    fn prospective_prepare_legacy_source_and_target_are_exact_without_mutation() {
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        let (fixture, writer) = prospective_test_fixture("prospective-legacy", 0, false);
        prospective_with_offline_fixture_connection(&fixture, |connection| {
            connection.execute("INSERT INTO ledger(date,total_value,cash,market_value,daily_pnl,created_at) VALUES ('2026-09-28',123.25,12.25,111,0,'TEST_CODE_PRIVATE_ROW_PAYLOAD')", []).unwrap();
        });
        // All row values are synthetic fixture facts, not a Paper seed/approval.
        let row = prospective_ledger_row(&fixture);
        prospective_assert_fixture_offline(&fixture);
        let before = fs::read(fixture.database()).unwrap();
        let before_capture_hit = std::rc::Rc::new(std::cell::Cell::new(false));
        let hook_hit = before_capture_hit.clone();
        let mut options = prospective::Options::production();
        options.hook = Some(Box::new(move |phase| {
            if phase == prospective::Phase::BeforeInitialCapture {
                hook_hit.set(true);
            }
            Ok(())
        }));
        let prepared_result = prospective_test_prepare(&fixture, &writer, options);
        assert!(
            before_capture_hit.get(),
            "offline fixture must reach the actual owner capture boundary"
        );
        let prepared = prepared_result.unwrap();
        assert!(matches!(
            fixture.acquire_exclusive(),
            Err(GlobalSchemaV1Error::ExclusiveProcessMaintenanceLeaseUnavailable)
        ));
        let rendered = prepared.render_unapproved().unwrap();
        let report: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        assert_eq!(
            report["review"]["database"]["sha256"],
            hex::encode(sha2::Sha256::digest(&before))
        );
        assert_eq!(report["review"]["target"]["user_version"], 1);
        assert_eq!(
            report["review"]["target"]["target_kind"],
            "install_final_selection_same_catalog_family"
        );
        assert_eq!(report["review"]["approval"], "not_granted");
        assert_eq!(report["review"]["backup"], "not_created");
        assert_eq!(report["review"]["apply_supported"], false);
        assert_eq!(
            report["review"]["legacy_counts_diagnostic_only"]["ledger"],
            1
        );
        assert!(!rendered.contains("TEST_CODE_PRIVATE_ROW_PAYLOAD"));
        assert!(!rendered.contains("CREATE TABLE"));
        assert_eq!(prospective_ledger_row(&fixture), row);
        assert_eq!(fs::read(fixture.database()).unwrap(), before);
        assert!(!writer.path().exists());
        drop(fixture.acquire_exclusive().unwrap());
        assert!(!fixture.database().with_extension("db-wal").exists());
    }

    #[test]
    fn prospective_prepare_extended_targets_preserve_actual_family() {
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        for generation in 2..=5 {
            for final_catalog in [false, true] {
                let (fixture, writer) =
                    prospective_test_fixture("prospective-extended", generation, final_catalog);
                if final_catalog {
                    writer
                        .append(SelectionAuditRecord::new(
                            SelectionAuditPhase::V2GateDCanaryVerified,
                            "TEST_CODE_OLD_RECEIPT",
                            "c".repeat(64),
                            chrono::DateTime::parse_from_rfc3339("2026-07-29T00:02:00+08:00")
                                .unwrap(),
                        ))
                        .unwrap();
                }
                let before = fs::read(fixture.database()).unwrap();
                let audit_before = fs::read(writer.path()).ok();
                let rendered =
                    prospective_test_prepare(&fixture, &writer, prospective::Options::production())
                        .unwrap()
                        .render_unapproved()
                        .unwrap();
                let report: serde_json::Value = serde_json::from_str(&rendered).unwrap();
                assert_eq!(report["review"]["original_user_version"], generation);
                assert_eq!(report["review"]["target"]["user_version"], generation);
                assert_eq!(
                    report["review"]["target"]["target_kind"],
                    if final_catalog {
                        "requalify_exact_existing_catalog"
                    } else {
                        "install_final_selection_same_catalog_family"
                    }
                );
                assert_eq!(report["review"]["maintenance_receipt"], "not_created");
                assert_eq!(report["review"]["apply_supported"], false);
                assert_eq!(fs::read(fixture.database()).unwrap(), before);
                assert_eq!(fs::read(writer.path()).ok(), audit_before);
            }
        }
    }

    #[test]
    fn prospective_prepare_actual_amended_requires_original_reconciliation() {
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        let (fixture, writer) = prospective_test_fixture("prospective-reconciled", 1, true);
        assert!(
            prospective_test_prepare(&fixture, &writer, prospective::Options::production())
                .is_err()
        );
        writer
            .append(SelectionAuditRecord::new(
                SelectionAuditPhase::V2GateDCanaryVerified,
                "TEST_CODE_ORIGINAL_CANARY",
                "d".repeat(64),
                chrono::DateTime::parse_from_rfc3339("2026-07-29T00:02:00+08:00").unwrap(),
            ))
            .unwrap();
        let report: serde_json::Value = serde_json::from_str(
            &prospective_test_prepare(&fixture, &writer, prospective::Options::production())
                .unwrap()
                .render_unapproved()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            report["review"]["target"]["target_kind"],
            "already_qualified"
        );
        // It is still the same low-authority observation, not the pool capability.
        assert_eq!(
            report["review"]["capability_scope"],
            "unapproved_prospective_read_only"
        );
    }

    #[test]
    fn prospective_prepare_unknown_absent_transitional_and_bad_receipt_never_heal() {
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        for kind in ["absent", "unknown", "transitional", "bad_receipt"] {
            let (fixture, writer) = if kind == "absent" {
                let fixture = TestFixture::new("prospective-absent", 0, 0);
                fixture.enable_wal_without_selection_catalog();
                let writer = fixture.pinned_audit_writer();
                (fixture, writer)
            } else if kind == "transitional" {
                let fixture = TestFixture::new("prospective-transitional", 0, 0);
                let connection = Connection::open(fixture.database()).unwrap();
                install_exact_selection_catalog_for_test(
                    &connection,
                    GlobalSchemaCatalogMode::Test,
                    false,
                )
                .unwrap();
                connection
                    .execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_checkpoint(TRUNCATE)")
                    .unwrap();
                drop(connection);
                let writer = fixture.pinned_audit_writer();
                writer
                    .append(SelectionAuditRecord::new(
                        SelectionAuditPhase::V2GateDCanaryVerified,
                        "TEST_CODE_TRANSITIONAL",
                        "e".repeat(64),
                        chrono::DateTime::parse_from_rfc3339("2026-07-29T00:02:00+08:00").unwrap(),
                    ))
                    .unwrap();
                (fixture, writer)
            } else {
                prospective_test_fixture(
                    "prospective-invalid",
                    if kind == "bad_receipt" { 1 } else { 0 },
                    kind == "bad_receipt",
                )
            };
            if kind == "unknown" {
                Connection::open(fixture.database())
                    .unwrap()
                    .execute_batch("CREATE TABLE TEST_CODE_extra(value TEXT)")
                    .unwrap();
            }
            if kind == "bad_receipt" {
                writer
                    .append(SelectionAuditRecord::new(
                        SelectionAuditPhase::V2ConfigActivationCommitted,
                        "TEST_CODE_BAD_RECEIPT",
                        "a".repeat(64),
                        chrono::DateTime::parse_from_rfc3339("2026-07-29T00:02:00+08:00").unwrap(),
                    ))
                    .unwrap();
            }
            let before = fs::read(fixture.database()).unwrap();
            let audit_before = fs::read(writer.path()).ok();
            assert!(
                prospective_test_prepare(&fixture, &writer, prospective::Options::production())
                    .is_err(),
                "{kind}"
            );
            assert_eq!(fs::read(fixture.database()).unwrap(), before);
            assert_eq!(fs::read(writer.path()).ok(), audit_before);
            drop(fixture.acquire_exclusive().unwrap());
        }
    }

    #[test]
    fn prospective_prepare_audit_absent_empty_and_real_high_water_are_distinct() {
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        for records in [None, Some(0), Some(2)] {
            let (fixture, writer) = prospective_test_fixture("prospective-audit", 0, false);
            if let Some(count) = records {
                File::create(writer.path()).unwrap();
                for index in 0..count {
                    writer
                        .append(SelectionAuditRecord::new(
                            SelectionAuditPhase::Prepared,
                            format!("TEST_CODE_ORIGINAL_{index}"),
                            "b".repeat(64),
                            chrono::DateTime::parse_from_rfc3339("2026-07-29T00:02:00+08:00")
                                .unwrap(),
                        ))
                        .unwrap();
                }
            }
            let audit_before = fs::read(writer.path()).ok();
            let report: serde_json::Value = serde_json::from_str(
                &prospective_test_prepare(&fixture, &writer, prospective::Options::production())
                    .unwrap()
                    .render_unapproved()
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(
                report["review"]["audit"]["state"],
                if records.is_none() {
                    "absent"
                } else {
                    "present"
                }
            );
            if let Some(count) = records {
                assert_eq!(report["review"]["audit"]["record_count"], count);
            }
            assert_eq!(fs::read(writer.path()).ok(), audit_before);
        }
    }

    #[test]
    fn prospective_prepare_failures_release_readonly_transaction_and_owner() {
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        for fail_phase in [
            prospective::Phase::AfterInitialCapture,
            prospective::Phase::BeforeFinalCapture,
            prospective::Phase::AfterReadOnlyCommit,
            prospective::Phase::AfterSidecarCleanup,
            prospective::Phase::BeforeRender,
        ] {
            let (fixture, writer) =
                prospective_test_fixture("prospective-readonly-abort", 0, false);
            let before = fs::read(fixture.database()).unwrap();
            let hit = std::rc::Rc::new(std::cell::Cell::new(false));
            let hook_hit = hit.clone();
            let mut options = prospective::Options::production();
            options.hook = Some(Box::new(move |phase| {
                if phase == fail_phase {
                    hook_hit.set(true);
                    return Err(prospective::refusal("TEST_CODE readonly owner fault"));
                }
                Ok(())
            }));
            let result = prospective_test_prepare(&fixture, &writer, options)
                .and_then(prospective::PreparedGlobalSchemaProspective::render_unapproved);
            assert!(result.is_err());
            assert!(hit.get());
            assert_eq!(fs::read(fixture.database()).unwrap(), before);
            assert!(!writer.path().exists());
            let connection = Connection::open(fixture.database()).unwrap();
            connection
                .execute_batch("BEGIN IMMEDIATE; ROLLBACK")
                .unwrap();
            drop(connection);
            drop(fixture.acquire_exclusive().unwrap());
        }
    }

    #[test]
    fn prospective_prepare_main_bytes_drift_at_each_last_boundary_is_rejected() {
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        for mutation_phase in [
            prospective::Phase::AfterInitialCapture,
            prospective::Phase::AfterReadOnlyCommit,
            prospective::Phase::AfterSidecarCleanup,
            prospective::Phase::BeforeRender,
        ] {
            let (fixture, writer) = prospective_test_fixture("prospective-last-main", 0, false);
            let before = fs::read(fixture.database()).unwrap();
            let path = fixture.database();
            let hit = std::rc::Rc::new(std::cell::Cell::new(false));
            let hook_hit = hit.clone();
            let mut changed = before.clone();
            let last = changed.len() - 1;
            changed[last] ^= 1;
            let mut options = prospective::Options::production();
            options.hook = Some(Box::new(move |phase| {
                if phase == mutation_phase {
                    fs::write(&path, &changed).unwrap();
                    hook_hit.set(true);
                }
                Ok(())
            }));
            assert!(prospective_test_prepare(&fixture, &writer, options)
                .and_then(prospective::PreparedGlobalSchemaProspective::render_unapproved)
                .is_err());
            assert!(hit.get()); // The attack persists; readonly owner does not pretend to roll it back.
            assert_ne!(fs::read(fixture.database()).unwrap(), before);
            fs::write(fixture.database(), before).unwrap();
            drop(fixture.acquire_exclusive().unwrap());
        }
    }

    #[test]
    fn prospective_prepare_same_bytes_replacement_and_new_audit_are_rejected() {
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        for attack in ["database", "audit", "namespace", "lock"] {
            let (fixture, writer) = prospective_test_fixture("prospective-replacement", 0, false);
            let path = match attack {
                "database" => fixture.database(),
                "audit" => writer.path().to_path_buf(),
                "namespace" => fixture.root.clone(),
                _ => fixture.lock_file(),
            };
            let hit = std::rc::Rc::new(std::cell::Cell::new(false));
            let hook_hit = hit.clone();
            let attack_owned = attack.to_owned();
            let mut options = prospective::Options::production();
            options.hook = Some(Box::new(move |phase| {
                if phase == prospective::Phase::AfterSidecarCleanup {
                    if attack_owned == "namespace" {
                        fs::rename(&path, path.with_extension("displaced")).unwrap();
                        fs::create_dir(&path).unwrap();
                    } else if attack_owned == "audit" {
                        fs::write(&path, b"").unwrap();
                    } else {
                        let bytes = fs::read(&path).unwrap();
                        fs::rename(&path, path.with_extension("displaced")).unwrap();
                        fs::write(&path, bytes).unwrap();
                    }
                    hook_hit.set(true);
                }
                Ok(())
            }));
            assert!(prospective_test_prepare(&fixture, &writer, options).is_err());
            assert!(hit.get());
            if attack == "namespace" {
                fs::remove_dir(&fixture.root).unwrap();
                fs::rename(fixture.root.with_extension("displaced"), &fixture.root).unwrap();
            }
        }
    }

    #[test]
    fn prospective_prepare_named_leaf_swap_after_physical_reads_is_rejected() {
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        for attack_audit in [false, true] {
            let (fixture, writer) =
                prospective_test_fixture("prospective-after-physical", 0, false);
            writer
                .append(SelectionAuditRecord::new(
                    SelectionAuditPhase::Prepared,
                    "TEST_CODE_NAMED_LEAF",
                    "a".repeat(64),
                    chrono::DateTime::parse_from_rfc3339("2026-07-29T00:02:00+08:00").unwrap(),
                ))
                .unwrap();
            let path = if attack_audit {
                writer.path().to_path_buf()
            } else {
                fixture.database()
            };
            let before = fs::read(&path).unwrap();
            let expected = before.clone();
            let hit = std::rc::Rc::new(std::cell::Cell::new(false));
            let hook_hit = hit.clone();
            let mut options = prospective::Options::production();
            options.hook = Some(Box::new(move |phase| {
                if phase == prospective::Phase::AfterPhysicalReads && !hook_hit.get() {
                    fs::rename(&path, path.with_extension("displaced")).unwrap();
                    fs::write(&path, &expected).unwrap();
                    hook_hit.set(true);
                }
                Ok(())
            }));
            assert!(prospective_test_prepare(&fixture, &writer, options).is_err());
            assert!(hit.get());
            let current = if attack_audit {
                writer.path().to_path_buf()
            } else {
                fixture.database()
            };
            assert_eq!(fs::read(&current).unwrap(), before);
            assert_ne!(
                fs::metadata(&current).unwrap().ino(),
                fs::metadata(current.with_extension("displaced"))
                    .unwrap()
                    .ino()
            );
        }
    }

    #[test]
    fn prospective_prepare_rejects_preexisting_sidecars_and_nonempty_owned_wal() {
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        for suffix in ["-wal", "-shm", "-journal"] {
            let (fixture, writer) = prospective_test_fixture("prospective-sidecar", 0, false);
            let path = sidecar_path(&fixture.database(), suffix);
            fs::write(&path, b"TEST_CODE_UNKNOWN").unwrap();
            let before = fs::read(fixture.database()).unwrap();
            assert!(prospective_test_prepare(
                &fixture,
                &writer,
                prospective::Options::production()
            )
            .is_err());
            assert_eq!(fs::read(path).unwrap(), b"TEST_CODE_UNKNOWN");
            assert_eq!(fs::read(fixture.database()).unwrap(), before);
        }
        let (fixture, writer) = prospective_test_fixture("prospective-owned-wal", 0, false);
        let wal = sidecar_path(&fixture.database(), "-wal");
        let hit = std::rc::Rc::new(std::cell::Cell::new(false));
        let hook_hit = hit.clone();
        let mut options = prospective::Options::production();
        options.hook = Some(Box::new(move |phase| {
            if phase == prospective::Phase::AfterInitialCapture {
                OpenOptions::new()
                    .append(true)
                    .open(&wal)
                    .unwrap()
                    .write_all(b"TEST_CODE_NOT_A_WAL_SNAPSHOT")
                    .unwrap();
                hook_hit.set(true);
            }
            Ok(())
        }));
        assert!(prospective_test_prepare(&fixture, &writer, options).is_err());
        assert!(hit.get());
    }

    #[test]
    fn prospective_prepare_actual_shared_and_child_leases_block_before_snapshot() {
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        let (fixture, writer) = prospective_test_fixture("prospective-held-leases", 1, true);
        let before = fs::read(fixture.database()).unwrap();
        let shared = fixture.inspect().unwrap();
        assert!(matches!(
            prospective_test_prepare(&fixture, &writer, prospective::Options::production()),
            Err(GlobalSchemaV1Error::SharedToExclusiveUpgradeForbidden)
        ));
        drop(shared);
        let mut child = Command::new(std::env::current_exe().unwrap())
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
            .unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let mut ready = false;
        for _ in 0..20 {
            let mut line = String::new();
            if stdout.read_line(&mut line).unwrap() == 0 {
                break;
            }
            if line.contains("TEST_CODE_GLOBAL_SCHEMA_SHARED_LOCKED") {
                ready = true;
                break;
            }
        }
        assert!(ready);
        let result =
            prospective_test_prepare(&fixture, &writer, prospective::Options::production());
        drop(child.stdin.take());
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(matches!(
            result,
            Err(GlobalSchemaV1Error::ExclusiveMaintenanceLeaseUnavailable {
                retryable: true,
                ..
            })
        ));
        assert_eq!(fs::read(fixture.database()).unwrap(), before);
        assert!(!writer.path().exists());
        drop(fixture.acquire_exclusive().unwrap());
    }

    #[test]
    fn prospective_prepare_bounds_fail_without_source_mutation() {
        let _serial = PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        for limit in [
            "file",
            "total_hash",
            "catalog_count",
            "catalog_bytes",
            "review",
            "audit_setup",
            "audit_total",
            "test_upper_bound",
        ] {
            let (fixture, writer) = prospective_test_fixture("prospective-budget", 0, false);
            writer
                .append(SelectionAuditRecord::new(
                    SelectionAuditPhase::Prepared,
                    "TEST_CODE_BUDGET",
                    "a".repeat(64),
                    chrono::DateTime::parse_from_rfc3339("2026-07-29T00:02:00+08:00").unwrap(),
                ))
                .unwrap();
            let before = fs::read(fixture.database()).unwrap();
            let audit_before = fs::read(writer.path()).unwrap();
            let mut options = prospective::Options::production();
            match limit {
                "file" => options.max_file_bytes = before.len() as u64 - 1,
                "total_hash" => {
                    options.max_total_hash_bytes = before.len() as u64 + audit_before.len() as u64
                }
                "catalog_count" => options.max_catalog_objects = 0,
                "catalog_bytes" => options.max_catalog_bytes = 1,
                "review" => options.max_review_bytes = 8,
                "audit_setup" => {
                    options.audit_limits.max_scan_bytes = audit_before.len() as u64 - 1
                }
                "test_upper_bound" => options.max_catalog_objects += 1,
                _ => options.audit_limits.max_total_scan_bytes = audit_before.len() as u64 * 2,
            }
            assert!(
                prospective_test_prepare(&fixture, &writer, options)
                    .and_then(prospective::PreparedGlobalSchemaProspective::render_unapproved)
                    .is_err(),
                "{limit}"
            );
            assert_eq!(fs::read(fixture.database()).unwrap(), before);
            assert_eq!(fs::read(writer.path()).unwrap(), audit_before);
            drop(fixture.acquire_exclusive().unwrap());
        }
    }

    #[test]
    fn prospective_prepare_failure_renderer_never_echoes_raw_error_content() {
        let error = GlobalSchemaV1Error::SelectionAudit {
            source: SelectionAuditError::ChainInvalid("TEST_CODE_SECRET_AUDIT_ROW".into()),
        };
        let rendered = prospective::render_error(&error);
        assert!(rendered.contains("audit_chain_invalid"));
        assert!(!rendered.contains("TEST_CODE_SECRET_AUDIT_ROW"));
    }

    #[test]
    fn prospective_prepare_cli_rejects_conflicts_and_keeps_apply_blocker_before_io() {
        for arguments in [
            vec!["--prepare", "--test"],
            vec!["--prepare", "--apply"],
            vec!["--prepare", "--prepare"],
            vec!["--prepare", "--help"],
            vec!["--prepare", "--root=/tmp"],
            vec!["--prepare", "--approval=true"],
        ] {
            assert!(run_selection_v2_migration_command(arguments).is_err());
        }
        assert_eq!(
            run_selection_v2_migration_command(["--apply"]).unwrap_err(),
            super::super::selection_v2::SELECTION_V2_APPLY_BLOCKER
        );
        assert!(run_selection_v2_migration_command(["--help"])
            .unwrap()
            .contains("unapproved prospective"));
    }

    #[test]
    fn identity_matrix_accepts_explicit_generations_and_rejects_future() {
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
        for generation in [2, 3, 4] {
            assert_eq!(
                classify_identity(STOCK_ANALYSIS_SQLITE_APPLICATION_ID, generation).unwrap(),
                GlobalSchemaIdentity {
                    application_id: STOCK_ANALYSIS_SQLITE_APPLICATION_ID,
                    user_version: generation,
                }
            );
        }

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
            classify_identity(STOCK_ANALYSIS_SQLITE_APPLICATION_ID, 7),
            Err(GlobalSchemaV1Error::UnsupportedFutureGeneration {
                actual: 7,
                supported: 6
            })
        ));
        assert!(classify_identity(
            STOCK_ANALYSIS_SQLITE_APPLICATION_ID,
            PAPER_BOOK_PREPARED_CATALOG_GENERATION,
        )
        .is_ok());
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
    fn paper_ledger_catalog_v2_old_receipt_cannot_issue_extended_authority() {
        for generation in [PAPER_LEDGER_CATALOG_GENERATION, REVIEW_CATALOG_GENERATION] {
            let fixture = TestFixture::new(
                "paper-catalog-old-receipt",
                STOCK_ANALYSIS_SQLITE_APPLICATION_ID,
                1,
            );
            fixture.install_final_selection_catalog();
            let writer = fixture.pinned_audit_writer();
            writer
                .append(SelectionAuditRecord::new(
                    SelectionAuditPhase::V2GateDCanaryVerified,
                    "TEST_CODE_OLD_RECEIPT",
                    "c".repeat(64),
                    chrono::DateTime::parse_from_rfc3339("2026-07-29T00:02:00+08:00").unwrap(),
                ))
                .unwrap();
            let old = GlobalSchemaVersionOwner::for_test_code()
                .inspect_selection_with_audit_for_test(&fixture.root, &writer)
                .unwrap();
            assert!(matches!(old, SelectionSchemaInspectionOutcome::Amended(_)));
            drop(old);
            let conn = Connection::open(fixture.database()).unwrap();
            for (_, _, _, sql) in super::super::paper_ledger_schema_v1::STATEMENTS {
                conn.execute_batch(sql).unwrap();
            }
            if generation == REVIEW_CATALOG_GENERATION {
                for (_, _, _, sql) in super::super::daily_change_review_schema_v1::STATEMENTS {
                    conn.execute_batch(sql).unwrap();
                }
            }
            conn.pragma_update(None, "user_version", generation)
                .unwrap();
            conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
                .unwrap();
            drop(conn);
            // Only the just-created, closed private fixture sidecars are cleaned.
            // The production inspector must continue rejecting unknown sidecars.
            for suffix in ["-wal", "-shm"] {
                let path = sidecar_path(&fixture.database(), suffix);
                match fs::remove_file(path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => panic!("closed TEST_CODE fixture sidecar: {error}"),
                }
            }
            let before = fs::read(fixture.database()).unwrap();
            let outcome = GlobalSchemaVersionOwner::for_test_code()
                .inspect_selection_with_audit_for_test(&fixture.root, &writer)
                .unwrap();
            assert!(matches!(
                outcome,
                SelectionSchemaInspectionOutcome::Diagnostic(_)
            ));
            assert_eq!(
                outcome.authority_state(),
                if generation == REVIEW_CATALOG_GENERATION {
                    SelectionSchemaAuthorityDiagnostic::CatalogV3RequalificationRequired
                } else {
                    SelectionSchemaAuthorityDiagnostic::CatalogV2RequalificationRequired
                }
            );
            drop(outcome);
            assert_eq!(
                fs::read(fixture.database()).unwrap(),
                before,
                "classification must not auto migrate/qualify"
            );
        }
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
                PAPER_BOOK_PREPARED_CATALOG_GENERATION + 1,
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
