//! Registered business rules: BR-001, BR-016, BR-017, BR-050, BR-066, BR-129.
// -*- coding: utf-8 -*-
//! ===================================
//! A股自选股智能分析系统 - 数据库管理
//! ===================================
//!
//! 职责：
//! 1. 管理 SQLite 数据库连接（单例模式）
//! 2. 提供数据存取接口
//! 3. 实现智能更新逻辑（断点续传）

use chrono::NaiveDate;
use diesel::prelude::*;
use diesel::r2d2::ManageConnection;
use diesel::r2d2::{
    ConnectionManager, CustomizeConnection, Error as ConnectionManagerError, Pool, PoolError,
    PooledConnection,
};
use log::info;
use once_cell::sync::OnceCell;
use std::collections::HashMap;
use std::ffi::{CString, OsStr, OsString};
use std::fs::{File, Metadata};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
#[cfg(target_os = "macos")]
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use self::sqlite_descriptor_attestation::{
    validate_wal_journal_mode, AttestedSqliteHandles, DescriptorAttestationError,
    FileObjectIdentity, PinnedSqliteObjectSet, ProcessDescriptorSnapshot, SqliteObjectRole,
};
use crate::models::MaStatus;

pub(super) type DbPool = Pool<SqliteConnectionManager>;

#[cfg(target_os = "linux")]
const O_NOFOLLOW_FLAG: i32 = 0x0002_0000;
#[cfg(target_os = "macos")]
const O_NOFOLLOW_FLAG: i32 = 0x0000_0100;
#[cfg(target_os = "linux")]
const O_NONBLOCK_FLAG: i32 = 0x0000_0800;
#[cfg(target_os = "macos")]
const O_NONBLOCK_FLAG: i32 = 0x0000_0004;
#[cfg(target_os = "linux")]
const O_CLOEXEC_FLAG: i32 = 0x0008_0000;
#[cfg(target_os = "macos")]
const O_CLOEXEC_FLAG: i32 = 0x0100_0000;

unsafe extern "C" {
    fn openat(directory_fd: i32, path: *const std::ffi::c_char, flags: i32, ...) -> i32;
}

pub type DbConnection = PooledConnection<SqliteConnectionManager>;

/// Opaque identity of the actual SQLite main/WAL/SHM objects used by one
/// descriptor-attested checkout. Callers cannot construct this value from a
/// pathname or from source contents.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct DatabaseConnectionAuthority {
    objects: Arc<PinnedSqliteObjectSet>,
}

pub(crate) struct AttestedAttributionCheckout {
    connection: DbConnection,
    source: Arc<DescriptorSqliteSource>,
}

impl std::fmt::Debug for DatabaseConnectionAuthority {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DatabaseConnectionAuthority")
            .finish_non_exhaustive()
    }
}

impl AttestedAttributionCheckout {
    pub(crate) fn authority(
        &mut self,
    ) -> Result<DatabaseConnectionAuthority, DatabaseAuthorityError> {
        registered_descriptor_connection_authority(&self.source, &mut self.connection)
    }

    pub(crate) fn transaction_with_authority<T, E, F, M>(
        &mut self,
        map_authority: M,
        operation: F,
    ) -> Result<T, E>
    where
        E: From<diesel::result::Error>,
        F: FnOnce(&mut SqliteConnection, &DatabaseConnectionAuthority) -> Result<T, E>,
        M: Fn(DatabaseAuthorityError) -> E + Copy,
    {
        let source = Arc::clone(&self.source);
        self.connection.transaction(|connection| {
            let before = registered_descriptor_connection_authority(&source, connection)
                .map_err(map_authority)?;
            let operation_result = operation(connection, &before);
            let exit_result = registered_descriptor_connection_authority(&source, connection);
            match exit_result {
                Err(error) => Err(map_authority(error)),
                Ok(after) if after != before => {
                    Err(map_authority(latch_descriptor_integrity_failure(
                        &source,
                        "attribution database authority changed during transaction".into(),
                    )))
                }
                Ok(_) => operation_result,
            }
        })
    }

    pub(crate) fn immediate_transaction_with_authority<T, E, F, M>(
        &mut self,
        map_authority: M,
        operation: F,
    ) -> Result<T, E>
    where
        E: From<diesel::result::Error>,
        F: FnOnce(&mut SqliteConnection, &DatabaseConnectionAuthority) -> Result<T, E>,
        M: Fn(DatabaseAuthorityError) -> E + Copy,
    {
        let source = Arc::clone(&self.source);
        self.connection.immediate_transaction(|connection| {
            let before = registered_descriptor_connection_authority(&source, connection)
                .map_err(map_authority)?;
            let operation_result = operation(connection, &before);
            let exit_result = registered_descriptor_connection_authority(&source, connection);
            match exit_result {
                Err(error) => Err(map_authority(error)),
                Ok(after) if after != before => {
                    Err(map_authority(latch_descriptor_integrity_failure(
                        &source,
                        "attribution database authority changed during transaction".into(),
                    )))
                }
                Ok(_) => operation_result,
            }
        })
    }

    /// Serializes the just-committed database while this branded writer still
    /// proves its main/WAL/SHM authority, then runs one fresh deferred
    /// transaction on a new read-only, query-only in-memory connection.
    ///
    /// The SQLite route is never used for read-back identity. Authority is
    /// checked before and after serialization and again after the detached
    /// snapshot transaction, so an object replacement latches integrity even
    /// when the serialized bytes would otherwise remain logically valid.
    pub(crate) fn authority_bound_readonly_snapshot<T, E, F, M>(
        &mut self,
        expected: &DatabaseConnectionAuthority,
        map_authority: M,
        operation: F,
    ) -> Result<T, E>
    where
        E: From<diesel::result::Error>,
        F: FnOnce(&mut SqliteConnection) -> Result<T, E>,
        M: Fn(DatabaseAuthorityError) -> E + Copy,
    {
        let before = registered_descriptor_connection_authority(&self.source, &mut self.connection)
            .map_err(map_authority)?;
        require_matching_database_authority(
            &self.source,
            expected,
            &before,
            "serialized read-back writer differs from committed authority",
        )
        .map_err(map_authority)?;

        let serialized = self.connection.serialize_database_to_buffer();
        let after_serialize =
            registered_descriptor_connection_authority(&self.source, &mut self.connection)
                .map_err(map_authority)?;
        require_matching_database_authority(
            &self.source,
            expected,
            &after_serialize,
            "database authority changed while serializing read-back snapshot",
        )
        .map_err(map_authority)?;
        let detached_bytes = detached_readonly_snapshot_bytes(&self.source, serialized.as_slice())
            .map_err(map_authority)?;

        let snapshot_result = (|| {
            let mut reader = SqliteConnection::establish(":memory:").map_err(|error| {
                map_authority(DatabaseAuthorityError::DescriptorAttestationUnavailable {
                    detail: format!("cannot establish detached read-back connection: {error}"),
                })
            })?;
            reader.deserialize_readonly_database_from_buffer(&detached_bytes)?;
            diesel::sql_query("PRAGMA query_only=ON").execute(&mut reader)?;
            let query_only = diesel::sql_query("PRAGMA query_only")
                .get_result::<QueryOnlyPragmaRow>(&mut reader)?
                .query_only;
            if query_only != 1 {
                return Err(map_authority(
                    DatabaseAuthorityError::DescriptorAttestationUnavailable {
                        detail: "detached read-back connection is not query-only".into(),
                    },
                ));
            }
            reader.transaction(operation)
        })();
        let after_snapshot =
            registered_descriptor_connection_authority(&self.source, &mut self.connection)
                .map_err(map_authority)?;
        require_matching_database_authority(
            &self.source,
            expected,
            &after_snapshot,
            "database authority changed during detached read-back snapshot",
        )
        .map_err(map_authority)?;
        snapshot_result
    }

    #[cfg(test)]
    pub(crate) fn connection_for_test(&mut self) -> &mut SqliteConnection {
        &mut self.connection
    }
}

// ============================================================================
// 数据库管理器 - 单例模式
// ============================================================================

/// 数据库管理器
///
/// 职责：
/// 1. 管理数据库连接池
/// 2. 提供数据存取操作
/// 3. 实现断点续传逻辑
pub struct DatabaseManager {
    // Drop order is a safety contract: all pooled SQLite connections close
    // before the owner capability releases its database/audit descriptors and
    // exclusive GlobalSchema maintenance lease.
    pool: DbPool,
    attribution_pool: Option<DbPool>,
    attribution_connection_source: Option<Arc<DescriptorSqliteSource>>,
    readonly_attribution_snapshot: Option<TemporaryAttributionSnapshot>,
    #[allow(dead_code)]
    selection_connection_source: Option<Arc<DescriptorSqliteSource>>,
    #[allow(dead_code)]
    selection_schema_authority: Option<Box<global_schema_v1::VerifiedAmendedSelectionSchema>>,
}

static DB_INSTANCE: OnceCell<DatabaseManager> = OnceCell::new();

#[cfg(test)]
fn unit_test_database_path() -> &'static PathBuf {
    use once_cell::sync::Lazy;
    use std::time::{SystemTime, UNIX_EPOCH};

    static PATH: Lazy<PathBuf> = Lazy::new(|| {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock must be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "stock-analysis-unit-{}-{nonce}.db",
            std::process::id()
        ))
    });
    &PATH
}

#[cfg(test)]
fn unit_test_init_lock() -> &'static std::sync::Mutex<()> {
    use once_cell::sync::Lazy;
    static LOCK: Lazy<std::sync::Mutex<()>> = Lazy::new(|| std::sync::Mutex::new(()));
    &LOCK
}

#[derive(QueryableByName)]
struct JournalModeRow {
    #[diesel(sql_type = diesel::sql_types::Text)]
    journal_mode: String,
}

#[derive(QueryableByName)]
struct ForeignKeysPragmaRow {
    #[diesel(sql_type = diesel::sql_types::Integer)]
    foreign_keys: i32,
}

#[derive(QueryableByName)]
struct SynchronousPragmaRow {
    #[diesel(sql_type = diesel::sql_types::Integer)]
    synchronous: i32,
}

#[derive(QueryableByName)]
struct BusyTimeoutPragmaRow {
    #[diesel(sql_type = diesel::sql_types::Integer)]
    timeout: i32,
}

#[derive(QueryableByName)]
struct WalAutocheckpointPragmaRow {
    #[diesel(sql_type = diesel::sql_types::Integer)]
    wal_autocheckpoint: i32,
}

#[derive(QueryableByName)]
struct WalCheckpointRow {
    #[diesel(sql_type = diesel::sql_types::Integer)]
    busy: i32,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    log: i32,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    checkpointed: i32,
}

#[derive(QueryableByName)]
struct QueryOnlyPragmaRow {
    #[diesel(sql_type = diesel::sql_types::Integer)]
    query_only: i32,
}

#[derive(QueryableByName)]
struct SqliteDatabaseListRow {
    #[diesel(sql_type = diesel::sql_types::Text)]
    name: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    file: String,
}

#[derive(QueryableByName)]
struct SqliteAttestationTokenRow {
    #[diesel(sql_type = diesel::sql_types::Text)]
    token: String,
}

#[derive(QueryableByName)]
struct SqliteAttestationTriggerCountRow {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    count: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SqliteFileIdentity {
    device: u64,
    inode: u64,
}

impl SqliteFileIdentity {
    fn from_metadata(metadata: &Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SqliteObjectIdentity {
    device: u64,
    inode: u64,
    mode: u32,
}

impl SqliteObjectIdentity {
    fn from_metadata(metadata: &Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            mode: metadata.mode(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum DatabaseAuthorityError {
    #[error("descriptor_attestation_unavailable: {detail}")]
    DescriptorAttestationUnavailable { detail: String },
    #[error("descriptor_integrity_failed: {detail}")]
    DescriptorIntegrityFailed { detail: String },
}

#[derive(Debug)]
pub(crate) enum AttributionReadTransactionError<E> {
    Operation(E),
    StorageUnavailable { detail: String },
    Authority(DatabaseAuthorityError),
    SnapshotIntegrity { detail: String },
    Transaction { detail: String },
}

impl<E> From<diesel::result::Error> for AttributionReadTransactionError<E> {
    fn from(error: diesel::result::Error) -> Self {
        Self::Transaction {
            detail: error.to_string(),
        }
    }
}

impl DatabaseAuthorityError {
    #[allow(dead_code)]
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::DescriptorAttestationUnavailable { .. } => "descriptor_attestation_unavailable",
            Self::DescriptorIntegrityFailed { .. } => "descriptor_integrity_failed",
        }
    }
}

/// Internal hand-off from the GlobalSchema owner to the connection pool.
///
/// This value has no pathname constructor. The only non-test producer is the
/// owner-issued amended capability, which clones its already pinned root,
/// database-parent and database descriptors.
pub(super) struct PinnedSqliteDatabase {
    root: File,
    parent: File,
    leaf: OsString,
    #[cfg(not(test))]
    relative_identity: PathBuf,
    database_file: File,
    root_identity: SqliteObjectIdentity,
    parent_identity: SqliteObjectIdentity,
    database_object_identity: SqliteObjectIdentity,
    identity: SqliteFileIdentity,
}

impl PinnedSqliteDatabase {
    pub(super) fn from_owner_descriptors(
        root: File,
        parent: File,
        leaf: OsString,
        relative_identity: PathBuf,
        database_file: File,
    ) -> Result<Self, std::io::Error> {
        if leaf.is_empty() || Path::new(&leaf).components().count() != 1 {
            return Err(std::io::Error::other(
                "GlobalSchema database leaf is not one path component",
            ));
        }
        if relative_identity.is_absolute()
            || relative_identity.components().count() == 0
            || relative_identity
                .components()
                .any(|component| !matches!(component, std::path::Component::Normal(_)))
        {
            return Err(std::io::Error::other(
                "GlobalSchema database relative identity is not a normal relative path",
            ));
        }
        let root_metadata = root.metadata()?;
        if !root_metadata.is_dir() {
            return Err(std::io::Error::other(
                "GlobalSchema root descriptor is not a directory",
            ));
        }
        let parent_metadata = parent.metadata()?;
        if !parent_metadata.is_dir() {
            return Err(std::io::Error::other(
                "GlobalSchema database parent descriptor is not a directory",
            ));
        }
        let metadata = database_file.metadata()?;
        if !metadata.is_file() {
            return Err(std::io::Error::other(
                "GlobalSchema database descriptor is not a regular file",
            ));
        }
        Ok(Self {
            root,
            parent,
            leaf,
            #[cfg(not(test))]
            relative_identity,
            database_file,
            root_identity: SqliteObjectIdentity::from_metadata(&root_metadata),
            parent_identity: SqliteObjectIdentity::from_metadata(&parent_metadata),
            database_object_identity: SqliteObjectIdentity::from_metadata(&metadata),
            identity: SqliteFileIdentity::from_metadata(&metadata),
        })
    }

    #[cfg(test)]
    fn from_test_descriptors(
        root: File,
        parent: File,
        leaf: OsString,
        relative_identity: PathBuf,
        database_file: File,
    ) -> Result<Self, std::io::Error> {
        Self::from_owner_descriptors(root, parent, leaf, relative_identity, database_file)
    }
}

#[derive(Debug)]
struct DescriptorConnectionProof {
    handles: AttestedSqliteHandles,
    expected_objects: Arc<PinnedSqliteObjectSet>,
    shared_shm_anchor: Arc<File>,
}

#[derive(Debug, Clone)]
struct DescriptorPoolEvidence {
    expected_objects: Arc<PinnedSqliteObjectSet>,
    shared_shm_anchor: Arc<File>,
}

/// One checkout-scoped proof that the actual Diesel connection is attached to
/// the database inode retained by the GlobalSchema owner.
#[cfg(not(test))]
pub(super) struct SelectionConnectionBoundProof {
    root: SqliteObjectIdentity,
    parent: SqliteObjectIdentity,
    database: SqliteObjectIdentity,
    database_relative_identity: String,
}

#[cfg(not(test))]
impl SelectionConnectionBoundProof {
    pub(super) fn into_preimage(
        self,
    ) -> crate::selection::schema_v2::VerifiedOutcomeDueDatabaseObjectBindingPreimage {
        crate::selection::schema_v2::VerifiedOutcomeDueDatabaseObjectBindingPreimage {
            domain: crate::selection::schema_v2::DOMAIN_OUTCOME_DUE_DATABASE_OBJECT.into(),
            // This legacy field now carries an owner-scoped logical identity,
            // not a reopened/canonicalized filesystem path. Device/inode/mode
            // values below are all fstat results from retained descriptors.
            manifest_root_canonical_path: format!(
                "owner-retained://root/{:x}:{:x}:{:x}/parent/{:x}:{:x}:{:x}",
                self.root.device,
                self.root.inode,
                self.root.mode,
                self.parent.device,
                self.parent.inode,
                self.parent.mode,
            ),
            manifest_root_device: self.root.device,
            manifest_root_inode: self.root.inode,
            manifest_root_mode: self.root.mode,
            database_relative_path: self.database_relative_identity,
            database_device: self.database.device,
            database_inode: self.database.inode,
            database_mode: self.database.mode,
        }
    }
}

#[derive(Debug)]
struct DescriptorSqliteSource {
    health: ConnectionManager<SqliteConnection>,
    root: Arc<File>,
    parent: Arc<File>,
    leaf: OsString,
    root_identity: SqliteObjectIdentity,
    parent_identity: SqliteObjectIdentity,
    database_object_identity: SqliteObjectIdentity,
    identity: SqliteFileIdentity,
    database_anchor: Arc<File>,
    connect_lock: Mutex<()>,
    pool_evidence: Mutex<Option<DescriptorPoolEvidence>>,
    connection_proofs: Mutex<HashMap<String, DescriptorConnectionProof>>,
    first_integrity_failure: Mutex<Option<String>>,
    registration_namespace: String,
    next_connection_id: AtomicU64,
}

#[derive(Debug, Clone)]
enum SqliteConnectionSource {
    Legacy(Arc<ConnectionManager<SqliteConnection>>),
    Descriptor(Arc<DescriptorSqliteSource>),
}

/// r2d2 manager whose descriptor variant proves every newly established
/// SQLite connection is attached to the GlobalSchema owner's pinned inode.
///
/// The legacy path variant exists only for the pre-existing initializer. The
/// amended-schema constructor below cannot accept a path and always selects
/// the descriptor variant.
#[derive(Debug, Clone)]
#[doc(hidden)]
pub struct SqliteConnectionManager {
    source: SqliteConnectionSource,
}

impl SqliteConnectionManager {
    fn legacy(database_url: String) -> Self {
        Self {
            source: SqliteConnectionSource::Legacy(Arc::new(ConnectionManager::new(database_url))),
        }
    }

    fn descriptor(database: PinnedSqliteDatabase) -> Result<Self, DatabaseAuthorityError> {
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = database;
            return Err(DatabaseAuthorityError::DescriptorAttestationUnavailable {
                detail: format!(
                    "platform {} has no implemented SQLite descriptor attestation route",
                    std::env::consts::OS
                ),
            });
        }

        #[cfg(any(target_os = "linux", target_os = "macos"))]
        Ok(Self {
            source: SqliteConnectionSource::Descriptor(Arc::new(DescriptorSqliteSource {
                health: ConnectionManager::new(":memory:"),
                root: Arc::new(database.root),
                parent: Arc::new(database.parent),
                leaf: database.leaf,
                root_identity: database.root_identity,
                parent_identity: database.parent_identity,
                database_object_identity: database.database_object_identity,
                identity: database.identity,
                database_anchor: Arc::new(database.database_file),
                connect_lock: Mutex::new(()),
                pool_evidence: Mutex::new(None),
                connection_proofs: Mutex::new(HashMap::new()),
                first_integrity_failure: Mutex::new(None),
                registration_namespace: descriptor_registration_namespace()?,
                next_connection_id: AtomicU64::new(0),
            })),
        })
    }

    fn descriptor_source(&self) -> Option<Arc<DescriptorSqliteSource>> {
        match &self.source {
            SqliteConnectionSource::Legacy(_) => None,
            SqliteConnectionSource::Descriptor(source) => Some(Arc::clone(source)),
        }
    }

    fn verify_descriptor_connection(
        source: &DescriptorSqliteSource,
        connection: &mut SqliteConnection,
        expected_route: &Path,
    ) -> Result<SqliteFileIdentity, ConnectionManagerError> {
        verify_sqlite_connection_object(source.identity, connection, expected_route)
    }
}

fn descriptor_registration_namespace() -> Result<String, DatabaseAuthorityError> {
    let mut random = [0_u8; 32];
    File::open("/dev/urandom")
        .and_then(|mut source| source.read_exact(&mut random))
        .map_err(
            |error| DatabaseAuthorityError::DescriptorAttestationUnavailable {
                detail: format!("cannot issue descriptor source registration identity: {error}"),
            },
        )?;
    Ok(hex::encode(random))
}

fn verify_sqlite_connection_object(
    expected_identity: SqliteFileIdentity,
    connection: &mut SqliteConnection,
    expected_route: &Path,
) -> Result<SqliteFileIdentity, ConnectionManagerError> {
    let main = diesel::sql_query("PRAGMA database_list")
        .load::<SqliteDatabaseListRow>(connection)
        .map_err(|error| sqlite_configuration_error("read database_list", error))?
        .into_iter()
        .find(|row| row.name == "main")
        .ok_or_else(|| {
            sqlite_manager_error("descriptor-anchored SQLite connection has no main database")
        })?;
    if Path::new(&main.file) != expected_route {
        return Err(sqlite_manager_error(format!(
            "descriptor-anchored SQLite connection escaped owner route: expected {:?}, got {:?}",
            expected_route, main.file
        )));
    }
    let actual = std::fs::metadata(&main.file)
        .map(|metadata| SqliteFileIdentity::from_metadata(&metadata))
        .map_err(|error| {
            sqlite_manager_error(format!(
                "stat SQLite main descriptor route {:?}: {error}",
                main.file
            ))
        })?;
    if actual != expected_identity {
        return Err(sqlite_manager_error(format!(
            "descriptor-anchored SQLite inode mismatch: expected {:?}, got {:?}",
            expected_identity, actual
        )));
    }
    Ok(actual)
}

#[cfg(target_os = "macos")]
const F_GETPATH_COMMAND: i32 = 50;
#[cfg(target_os = "macos")]
const DARWIN_MAX_PATH_BYTES: usize = 1_024;

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn fcntl(descriptor: i32, command: i32, ...) -> i32;
}

/// Produces only the transient pathname SQLite needs to invoke its native VFS.
/// It is never identity evidence; descriptor attestation proves the actual
/// main/WAL/SHM objects opened by that VFS.
pub(super) fn sqlite_open_route_from_retained_parent(
    parent: &File,
    leaf: &std::ffi::OsStr,
) -> Result<PathBuf, ConnectionManagerError> {
    #[cfg(target_os = "linux")]
    {
        Ok(PathBuf::from(format!(
            "/proc/self/fd/{}/{}",
            parent.as_raw_fd(),
            leaf.to_string_lossy()
        )))
    }

    #[cfg(target_os = "macos")]
    {
        let mut bytes = [0_u8; DARWIN_MAX_PATH_BYTES];
        // SAFETY: F_GETPATH writes a NUL-terminated path into the supplied
        // writable buffer for this live retained directory descriptor.
        let result = unsafe { fcntl(parent.as_raw_fd(), F_GETPATH_COMMAND, bytes.as_mut_ptr()) };
        if result < 0 {
            return Err(sqlite_manager_error(format!(
                "descriptor_attestation_unavailable: resolve retained parent for SQLite open: {}",
                std::io::Error::last_os_error()
            )));
        }
        let length = bytes.iter().position(|byte| *byte == 0).ok_or_else(|| {
            sqlite_manager_error(
                "descriptor_attestation_unavailable: F_GETPATH returned no NUL terminator",
            )
        })?;
        Ok(PathBuf::from(OsString::from_vec(bytes[..length].to_vec())).join(leaf))
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (parent, leaf);
        Err(sqlite_manager_error(
            "descriptor_attestation_unavailable: SQLite open route is unsupported on this platform",
        ))
    }
}

fn sqlite_open_route(source: &DescriptorSqliteSource) -> Result<PathBuf, ConnectionManagerError> {
    sqlite_open_route_from_retained_parent(&source.parent, &source.leaf)
}

fn sqlite_sidecar_leaf(database_leaf: &OsStr, suffix: &str) -> OsString {
    let mut leaf = database_leaf.to_os_string();
    leaf.push(suffix);
    leaf
}

fn openat_regular_for_attestation(
    parent: &File,
    leaf: &OsStr,
    role: &str,
) -> Result<File, ConnectionManagerError> {
    let name = CString::new(leaf.as_bytes()).map_err(|_| {
        sqlite_manager_error(format!(
            "descriptor_attestation_unavailable: {role} leaf contains NUL"
        ))
    })?;
    // SAFETY: name is NUL-terminated, parent is a retained directory fd, and
    // success returns a new owned descriptor.
    let descriptor = unsafe {
        openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            O_NOFOLLOW_FLAG | O_NONBLOCK_FLAG | O_CLOEXEC_FLAG,
        )
    };
    if descriptor < 0 {
        return Err(sqlite_manager_error(format!(
            "descriptor_attestation_unavailable: open pinned {role} with openat(O_NOFOLLOW): {}",
            std::io::Error::last_os_error()
        )));
    }
    // SAFETY: openat returned a new owned descriptor.
    let file = unsafe { File::from_raw_fd(descriptor) };
    if !file
        .metadata()
        .map_err(|error| {
            sqlite_manager_error(format!(
                "descriptor_attestation_unavailable: fstat pinned {role}: {error}"
            ))
        })?
        .is_file()
    {
        return Err(sqlite_manager_error(format!(
            "descriptor_identity_changed: pinned {role} is not a regular file"
        )));
    }
    Ok(file)
}

fn capture_pinned_sqlite_object_set(
    source: &DescriptorSqliteSource,
) -> Result<PinnedSqliteObjectSet, ConnectionManagerError> {
    let wal_leaf = sqlite_sidecar_leaf(&source.leaf, "-wal");
    let shm_leaf = sqlite_sidecar_leaf(&source.leaf, "-shm");
    let wal = openat_regular_for_attestation(&source.parent, &wal_leaf, "SQLite WAL")?;
    let shm = openat_regular_for_attestation(&source.parent, &shm_leaf, "SQLite SHM")?;
    PinnedSqliteObjectSet::from_files(&source.database_anchor, &wal, &shm)
        .map_err(descriptor_attestation_manager_error)
}

fn open_shared_shm_anchor(
    source: &DescriptorSqliteSource,
    expected: &PinnedSqliteObjectSet,
) -> Result<File, ConnectionManagerError> {
    let shm_leaf = sqlite_sidecar_leaf(&source.leaf, "-shm");
    let file =
        openat_regular_for_attestation(&source.parent, &shm_leaf, "SQLite shared SHM anchor")?;
    let actual =
        FileObjectIdentity::from_file(&file).map_err(descriptor_attestation_manager_error)?;
    if actual != expected.identity(SqliteObjectRole::Shm) {
        return Err(sqlite_manager_error(
            "descriptor_identity_changed: separately pinned SHM anchor differs from attested SQLite SHM",
        ));
    }
    Ok(file)
}

fn current_descriptor_pool_evidence(
    source: &DescriptorSqliteSource,
) -> Result<Option<DescriptorPoolEvidence>, ConnectionManagerError> {
    source
        .pool_evidence
        .lock()
        .map(|evidence| evidence.clone())
        .map_err(|_| sqlite_manager_error("descriptor pool-evidence lock is poisoned"))
}

fn commit_descriptor_pool_evidence(
    source: &DescriptorSqliteSource,
    proposed: DescriptorPoolEvidence,
) -> Result<DescriptorPoolEvidence, ConnectionManagerError> {
    let mut state = source
        .pool_evidence
        .lock()
        .map_err(|_| sqlite_manager_error("descriptor pool-evidence lock is poisoned"))?;
    match state.as_ref() {
        Some(existing) if existing.expected_objects != proposed.expected_objects => {
            Err(sqlite_manager_error(
                "descriptor_identity_changed: SQLite main/WAL/SHM objects changed between connections",
            ))
        }
        Some(existing) => {
            let actual = FileObjectIdentity::from_file(&existing.shared_shm_anchor)
                .map_err(descriptor_attestation_manager_error)?;
            if actual != existing.expected_objects.identity(SqliteObjectRole::Shm) {
                return Err(sqlite_manager_error(
                    "descriptor_identity_changed: process-shared SHM anchor changed identity",
                ));
            }
            Ok(existing.clone())
        }
        None => {
            *state = Some(proposed.clone());
            Ok(proposed)
        }
    }
}

const DESCRIPTOR_ATTESTATION_TEMP_TABLE: &str =
    "__stock_analysis_descriptor_connection_attestation";
const DESCRIPTOR_ATTESTATION_NO_UPDATE_TRIGGER: &str =
    "__stock_analysis_descriptor_connection_attestation_no_update";
const DESCRIPTOR_ATTESTATION_NO_DELETE_TRIGGER: &str =
    "__stock_analysis_descriptor_connection_attestation_no_delete";

fn install_connection_attestation_token(
    connection: &mut SqliteConnection,
    token: &str,
) -> Result<(), ConnectionManagerError> {
    diesel::sql_query(format!(
        "CREATE TEMP TABLE {DESCRIPTOR_ATTESTATION_TEMP_TABLE} \
         (slot INTEGER NOT NULL PRIMARY KEY CHECK(slot = 1), \
          token TEXT NOT NULL UNIQUE)"
    ))
    .execute(connection)
    .map_err(|error| sqlite_configuration_error("create attestation token table", error))?;
    diesel::sql_query(format!(
        "INSERT INTO {DESCRIPTOR_ATTESTATION_TEMP_TABLE}(slot, token) VALUES (1, ?)"
    ))
    .bind::<diesel::sql_types::Text, _>(token)
    .execute(connection)
    .map_err(|error| sqlite_configuration_error("install attestation token", error))?;
    for (name, operation) in [
        (DESCRIPTOR_ATTESTATION_NO_UPDATE_TRIGGER, "UPDATE"),
        (DESCRIPTOR_ATTESTATION_NO_DELETE_TRIGGER, "DELETE"),
    ] {
        diesel::sql_query(format!(
            "CREATE TEMP TRIGGER {name} BEFORE {operation} ON {DESCRIPTOR_ATTESTATION_TEMP_TABLE} \
             BEGIN SELECT RAISE(ABORT, 'descriptor attestation registration is immutable'); END"
        ))
        .execute(connection)
        .map_err(|error| sqlite_configuration_error("protect attestation token", error))?;
    }
    Ok(())
}

fn connection_attestation_token(
    connection: &mut SqliteConnection,
) -> Result<String, ConnectionManagerError> {
    diesel::sql_query(format!(
        "SELECT token FROM {DESCRIPTOR_ATTESTATION_TEMP_TABLE} WHERE slot = 1"
    ))
    .get_result::<SqliteAttestationTokenRow>(connection)
    .map(|row| row.token)
    .map_err(|error| sqlite_configuration_error("read attestation token", error))
}

fn validate_connection_attestation_token_protection(
    connection: &mut SqliteConnection,
) -> Result<(), ConnectionManagerError> {
    let count = diesel::sql_query(
        "SELECT COUNT(*) AS count FROM sqlite_temp_master \
         WHERE type='trigger' AND name IN (?, ?)"
            .to_string(),
    )
    .bind::<diesel::sql_types::Text, _>(DESCRIPTOR_ATTESTATION_NO_UPDATE_TRIGGER)
    .bind::<diesel::sql_types::Text, _>(DESCRIPTOR_ATTESTATION_NO_DELETE_TRIGGER)
    .get_result::<SqliteAttestationTriggerCountRow>(connection)
    .map_err(|error| sqlite_configuration_error("read attestation token protection", error))?
    .count;
    if count != 2 {
        return Err(sqlite_manager_error(
            "descriptor_identity_changed: attestation token protection changed",
        ));
    }
    Ok(())
}

fn validate_registered_descriptor_connection(
    source: &DescriptorSqliteSource,
    connection: &mut SqliteConnection,
) -> Result<SqliteFileIdentity, ConnectionManagerError> {
    validate_retained_namespace(source)?;
    let route = sqlite_open_route(source)?;
    let actual = SqliteConnectionManager::verify_descriptor_connection(source, connection, &route)?;
    validate_connection_attestation_token_protection(connection)?;
    let token = connection_attestation_token(connection)?;
    let proofs = source
        .connection_proofs
        .lock()
        .map_err(|_| sqlite_manager_error("descriptor connection-proof lock is poisoned"))?;
    let proof = proofs.get(&token).ok_or_else(|| {
        sqlite_manager_error("descriptor-bound connection has no registered fd attestation")
    })?;
    proof
        .handles
        .validate(&proof.expected_objects)
        .map_err(descriptor_attestation_manager_error)?;
    let shared_shm = FileObjectIdentity::from_file(&proof.shared_shm_anchor)
        .map_err(descriptor_attestation_manager_error)?;
    if shared_shm != proof.expected_objects.identity(SqliteObjectRole::Shm) {
        return Err(sqlite_manager_error(
            "descriptor_identity_changed: process-shared SHM proof changed identity",
        ));
    }
    let current_objects = capture_pinned_sqlite_object_set(source)?;
    if &current_objects != proof.expected_objects.as_ref() {
        return Err(sqlite_manager_error(
            "descriptor_identity_changed: current main/WAL/SHM leaves differ from retained connection proof",
        ));
    }
    Ok(actual)
}

fn descriptor_integrity_latch(
    source: &DescriptorSqliteSource,
) -> Result<(), DatabaseAuthorityError> {
    let failure = source.first_integrity_failure.lock().map_err(|_| {
        DatabaseAuthorityError::DescriptorIntegrityFailed {
            detail: "descriptor source integrity latch is poisoned".into(),
        }
    })?;
    match failure.as_ref() {
        Some(detail) => Err(DatabaseAuthorityError::DescriptorIntegrityFailed {
            detail: format!("latched descriptor source integrity failure: {detail}"),
        }),
        None => Ok(()),
    }
}

fn latch_descriptor_integrity_failure(
    source: &DescriptorSqliteSource,
    detail: String,
) -> DatabaseAuthorityError {
    match source.first_integrity_failure.lock() {
        Ok(mut failure) => {
            let first = failure.get_or_insert(detail);
            DatabaseAuthorityError::DescriptorIntegrityFailed {
                detail: format!("latched descriptor source integrity failure: {first}"),
            }
        }
        Err(_) => DatabaseAuthorityError::DescriptorIntegrityFailed {
            detail: "descriptor source integrity latch is poisoned".into(),
        },
    }
}

fn require_matching_database_authority(
    source: &DescriptorSqliteSource,
    expected: &DatabaseConnectionAuthority,
    actual: &DatabaseConnectionAuthority,
    context: &str,
) -> Result<(), DatabaseAuthorityError> {
    if actual == expected {
        Ok(())
    } else {
        Err(latch_descriptor_integrity_failure(
            source,
            context.to_owned(),
        ))
    }
}

fn detached_readonly_snapshot_bytes(
    source: &DescriptorSqliteSource,
    serialized: &[u8],
) -> Result<Vec<u8>, DatabaseAuthorityError> {
    const SQLITE_HEADER_BYTES: usize = 100;
    const SQLITE_MAGIC: &[u8; 16] = b"SQLite format 3\0";
    const HEADER_WRITE_VERSION: usize = 18;
    const HEADER_READ_VERSION: usize = 19;

    if serialized.len() < SQLITE_HEADER_BYTES || serialized.get(..16) != Some(SQLITE_MAGIC) {
        return Err(latch_descriptor_integrity_failure(
            source,
            "serialized read-back snapshot has an invalid SQLite header".into(),
        ));
    }
    let versions = (
        serialized[HEADER_READ_VERSION],
        serialized[HEADER_WRITE_VERSION],
    );
    if !matches!(versions, (1, 1) | (2, 2)) {
        return Err(latch_descriptor_integrity_failure(
            source,
            format!(
                "serialized read-back snapshot has unsupported read/write versions {versions:?}"
            ),
        ));
    }

    // sqlite3_serialize returns the complete logical database image, but a
    // WAL-backed source retains (2,2) in the file-format header. A detached
    // read-only deserialization has no WAL route, so normalize only those two
    // format bytes to rollback-journal (1,1); all business pages remain exact.
    let mut detached = serialized.to_vec();
    detached[HEADER_READ_VERSION] = 1;
    detached[HEADER_WRITE_VERSION] = 1;
    Ok(detached)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReadonlyAttributionFileSnapshot {
    identity: SqliteObjectIdentity,
    modified: std::time::SystemTime,
    bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReadonlyAttributionSourceSnapshot {
    main: ReadonlyAttributionFileSnapshot,
    wal: Option<ReadonlyAttributionFileSnapshot>,
    shm: Option<ReadonlyAttributionFileSnapshot>,
}

fn sqlite_sidecar_path(database: &Path, suffix: &str) -> PathBuf {
    let mut path = database.as_os_str().to_os_string();
    path.push(suffix);
    PathBuf::from(path)
}

fn capture_readonly_attribution_file(
    path: &Path,
    required: bool,
) -> Result<Option<ReadonlyAttributionFileSnapshot>, String> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if !required && error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("inspect {}: {error}", path.display())),
    };
    if !metadata.file_type().is_file() {
        return Err(format!(
            "read-only attribution snapshot source is not a regular file: {}",
            path.display()
        ));
    }
    let identity = SqliteObjectIdentity::from_metadata(&metadata);
    let modified = metadata
        .modified()
        .map_err(|error| format!("read modification time for {}: {error}", path.display()))?;
    let expected_len = metadata.len();
    let mut file = File::open(path)
        .map_err(|error| format!("open read-only snapshot file {}: {error}", path.display()))?;
    let opened_metadata = file
        .metadata()
        .map_err(|error| format!("inspect opened snapshot file {}: {error}", path.display()))?;
    if SqliteObjectIdentity::from_metadata(&opened_metadata) != identity
        || opened_metadata.modified().map_err(|error| {
            format!(
                "read opened snapshot modification time for {}: {error}",
                path.display()
            )
        })? != modified
        || opened_metadata.len() != expected_len
    {
        return Err(format!(
            "read-only attribution snapshot file changed while opening: {}",
            path.display()
        ));
    }
    let mut bytes = Vec::with_capacity(expected_len.try_into().unwrap_or(0));
    file.read_to_end(&mut bytes)
        .map_err(|error| format!("read snapshot file {}: {error}", path.display()))?;
    let after_read = file
        .metadata()
        .map_err(|error| format!("re-inspect snapshot file {}: {error}", path.display()))?;
    if SqliteObjectIdentity::from_metadata(&after_read) != identity
        || after_read.modified().map_err(|error| {
            format!(
                "re-read snapshot modification time for {}: {error}",
                path.display()
            )
        })? != modified
        || after_read.len() != expected_len
        || bytes.len() as u64 != expected_len
    {
        return Err(format!(
            "read-only attribution snapshot file changed while reading: {}",
            path.display()
        ));
    }
    let named_after_read = std::fs::symlink_metadata(path)
        .map_err(|error| format!("re-inspect snapshot pathname {}: {error}", path.display()))?;
    if SqliteObjectIdentity::from_metadata(&named_after_read) != identity {
        return Err(format!(
            "read-only attribution snapshot pathname changed while reading: {}",
            path.display()
        ));
    }
    Ok(Some(ReadonlyAttributionFileSnapshot {
        identity,
        modified,
        bytes,
    }))
}

fn capture_readonly_attribution_source(
    database: &Path,
) -> Result<ReadonlyAttributionSourceSnapshot, String> {
    Ok(ReadonlyAttributionSourceSnapshot {
        main: capture_readonly_attribution_file(database, true)?
            .expect("required read-only attribution main database must be present"),
        wal: capture_readonly_attribution_file(&sqlite_sidecar_path(database, "-wal"), false)?,
        shm: capture_readonly_attribution_file(&sqlite_sidecar_path(database, "-shm"), false)?,
    })
}

static NEXT_READONLY_ATTRIBUTION_SNAPSHOT: AtomicU64 = AtomicU64::new(0);

struct TemporaryAttributionSnapshot {
    directory: PathBuf,
    database: PathBuf,
    cleaned: bool,
}

impl TemporaryAttributionSnapshot {
    fn create(source: &ReadonlyAttributionSourceSnapshot) -> Result<Self, String> {
        let directory = loop {
            let sequence = NEXT_READONLY_ATTRIBUTION_SNAPSHOT.fetch_add(1, Ordering::Relaxed);
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or_default();
            let candidate = std::env::temp_dir().join(format!(
                "stock-analysis-attribution-snapshot-{}-{nonce}-{sequence}",
                std::process::id()
            ));
            let mut builder = std::fs::DirBuilder::new();
            builder.mode(0o700);
            match builder.create(&candidate) {
                Ok(()) => break candidate,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(format!(
                        "create private attribution snapshot directory: {error}"
                    ));
                }
            }
        };
        let database = directory.join("snapshot.db");
        let mut snapshot = Self {
            directory,
            database,
            cleaned: false,
        };
        snapshot.write_file(&snapshot.database.clone(), &source.main.bytes)?;
        let copied_wal = source.wal.as_ref().filter(|wal| !wal.bytes.is_empty());
        if let Some(wal) = copied_wal {
            snapshot.write_file(&sqlite_sidecar_path(&snapshot.database, "-wal"), &wal.bytes)?;
            snapshot.materialize_copied_wal()?;
        }
        Ok(snapshot)
    }

    fn write_file(&mut self, path: &Path, bytes: &[u8]) -> Result<(), String> {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)
            .map_err(|error| {
                format!("create detached snapshot file {}: {error}", path.display())
            })?;
        file.write_all(bytes)
            .map_err(|error| format!("write detached snapshot file {}: {error}", path.display()))
    }

    fn database_path(&self) -> &Path {
        &self.database
    }

    fn materialize_copied_wal(&self) -> Result<(), String> {
        // This writer is confined to the private detached directory. It does
        // not execute schema or application writes; it only materializes the
        // already captured logical snapshot so the later mode=ro pool never
        // needs to rebuild source WAL state.
        let database_url = self.database.to_string_lossy().into_owned();
        let mut bootstrap = SqliteConnection::establish(&database_url).map_err(|error| {
            format!(
                "establish private attribution WAL bootstrap {}: {error}",
                self.database.display()
            )
        })?;
        let checkpoint = diesel::sql_query("PRAGMA wal_checkpoint(TRUNCATE)")
            .get_result::<WalCheckpointRow>(&mut bootstrap)
            .map_err(|error| {
                format!(
                    "checkpoint private attribution WAL {}: {error}",
                    self.database.display()
                )
            })?;
        if checkpoint.busy != 0
            || checkpoint.log < 0
            || checkpoint.checkpointed < 0
            || checkpoint.log != checkpoint.checkpointed
        {
            return Err(format!(
                "private attribution WAL checkpoint incomplete (requires busy=0 and non-negative equal frame counts): busy={}, log={}, checkpointed={}",
                checkpoint.busy, checkpoint.log, checkpoint.checkpointed
            ));
        }
        let journal_mode = diesel::sql_query("PRAGMA journal_mode=DELETE")
            .get_result::<JournalModeRow>(&mut bootstrap)
            .map_err(|error| {
                format!(
                    "normalize private attribution snapshot journal mode {}: {error}",
                    self.database.display()
                )
            })?
            .journal_mode;
        if !journal_mode.eq_ignore_ascii_case("delete") {
            return Err(format!(
                "private attribution snapshot retained journal_mode={journal_mode} after WAL checkpoint"
            ));
        }
        drop(bootstrap);
        for suffix in ["-wal", "-shm"] {
            let sidecar = sqlite_sidecar_path(&self.database, suffix);
            match std::fs::metadata(&sidecar) {
                Ok(metadata) if metadata.len() == 0 => {}
                Ok(metadata) => {
                    return Err(format!(
                        "private attribution {suffix} retained {} bytes after DELETE normalization",
                        metadata.len()
                    ));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(format!(
                        "inspect private attribution {suffix} after DELETE normalization: {error}"
                    ));
                }
            }
        }
        Ok(())
    }

    fn cleanup(&mut self) -> Result<(), String> {
        if self.cleaned {
            return Ok(());
        }
        let known = [
            self.database.clone(),
            sqlite_sidecar_path(&self.database, "-wal"),
            sqlite_sidecar_path(&self.database, "-shm"),
        ];
        let mut unknown = Vec::new();
        for entry in std::fs::read_dir(&self.directory).map_err(|error| {
            format!(
                "inspect detached snapshot directory {}: {error}",
                self.directory.display()
            )
        })? {
            let path = entry
                .map_err(|error| format!("inspect detached snapshot entry: {error}"))?
                .path();
            if !known.contains(&path) {
                unknown.push(path);
            }
        }
        if !unknown.is_empty() {
            return Err(format!(
                "detached snapshot directory contains unknown entries: {}",
                unknown
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        for path in known {
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(format!(
                        "remove detached snapshot file {}: {error}",
                        path.display()
                    ));
                }
            }
        }
        std::fs::remove_dir(&self.directory).map_err(|error| {
            format!(
                "remove detached snapshot directory {}: {error}",
                self.directory.display()
            )
        })?;
        self.cleaned = true;
        Ok(())
    }
}

impl Drop for TemporaryAttributionSnapshot {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

fn validate_live_registered_descriptor_connection(
    source: &DescriptorSqliteSource,
    connection: &mut SqliteConnection,
) -> Result<SqliteFileIdentity, ConnectionManagerError> {
    registered_descriptor_connection_identity(source, connection)
        .map_err(|error| sqlite_manager_error(error.to_string()))
}

fn registered_descriptor_connection_identity(
    source: &DescriptorSqliteSource,
    connection: &mut SqliteConnection,
) -> Result<SqliteFileIdentity, DatabaseAuthorityError> {
    descriptor_integrity_latch(source)?;
    validate_registered_descriptor_connection(source, connection)
        .map_err(|error| latch_descriptor_integrity_failure(source, error.to_string()))
}

fn registered_descriptor_connection_authority(
    source: &DescriptorSqliteSource,
    connection: &mut SqliteConnection,
) -> Result<DatabaseConnectionAuthority, DatabaseAuthorityError> {
    descriptor_integrity_latch(source)?;
    let result = (|| -> Result<DatabaseConnectionAuthority, String> {
        validate_retained_namespace(source).map_err(|error| error.to_string())?;
        validate_connection_attestation_token_protection(connection)
            .map_err(|error| error.to_string())?;
        let token = connection_attestation_token(connection).map_err(|error| error.to_string())?;
        let objects = {
            let proofs = source
                .connection_proofs
                .lock()
                .map_err(|_| "descriptor connection-proof lock is poisoned".to_owned())?;
            let proof = proofs.get(&token).ok_or_else(|| {
                "descriptor-bound connection has no registered fd attestation".to_owned()
            })?;
            proof
                .handles
                .validate(&proof.expected_objects)
                .map_err(|error| error.to_string())?;
            let shared_shm = FileObjectIdentity::from_file(&proof.shared_shm_anchor)
                .map_err(|error| error.to_string())?;
            if shared_shm != proof.expected_objects.identity(SqliteObjectRole::Shm) {
                return Err("process-shared SHM proof changed identity".into());
            }
            Arc::clone(&proof.expected_objects)
        };
        let current_objects =
            capture_pinned_sqlite_object_set(source).map_err(|error| error.to_string())?;
        if current_objects != *objects {
            return Err("current main/WAL/SHM objects differ from the registered checkout".into());
        }
        Ok(DatabaseConnectionAuthority { objects })
    })();
    result.map_err(|detail| latch_descriptor_integrity_failure(source, detail))
}

fn validate_attribution_source_namespace(
    source: &DescriptorSqliteSource,
) -> Result<(), DatabaseAuthorityError> {
    descriptor_integrity_latch(source)?;
    let result = (|| -> Result<(), String> {
        validate_retained_namespace(source).map_err(|error| error.to_string())?;
        if let Some(evidence) =
            current_descriptor_pool_evidence(source).map_err(|error| error.to_string())?
        {
            let current =
                capture_pinned_sqlite_object_set(source).map_err(|error| error.to_string())?;
            if current != *evidence.expected_objects {
                return Err(
                    "current main/WAL/SHM objects differ from attribution pool evidence".into(),
                );
            }
        }
        Ok(())
    })();
    result.map_err(|detail| latch_descriptor_integrity_failure(source, detail))
}

fn validate_retained_namespace(
    source: &DescriptorSqliteSource,
) -> Result<(), ConnectionManagerError> {
    let root = SqliteObjectIdentity::from_metadata(&source.root.metadata().map_err(|error| {
        sqlite_manager_error(format!(
            "descriptor_identity_changed: fstat retained root: {error}"
        ))
    })?);
    let parent =
        SqliteObjectIdentity::from_metadata(&source.parent.metadata().map_err(|error| {
            sqlite_manager_error(format!(
                "descriptor_identity_changed: fstat retained database parent: {error}"
            ))
        })?);
    let database = SqliteObjectIdentity::from_metadata(
        &source.database_anchor.metadata().map_err(|error| {
            sqlite_manager_error(format!(
                "descriptor_identity_changed: fstat retained database: {error}"
            ))
        })?,
    );
    if root != source.root_identity
        || parent != source.parent_identity
        || database != source.database_object_identity
    {
        return Err(sqlite_manager_error(
            "descriptor_identity_changed: retained owner namespace changed identity",
        ));
    }
    let reopened =
        openat_regular_for_attestation(&source.parent, &source.leaf, "SQLite main database")?;
    if FileObjectIdentity::from_file(&reopened).map_err(descriptor_attestation_manager_error)?
        != FileObjectIdentity::from_file(&source.database_anchor)
            .map_err(descriptor_attestation_manager_error)?
    {
        return Err(sqlite_manager_error(
            "descriptor_identity_changed: fixed database leaf no longer names owner database",
        ));
    }
    Ok(())
}

fn descriptor_attestation_manager_error(
    error: DescriptorAttestationError,
) -> ConnectionManagerError {
    sqlite_manager_error(error.to_string())
}

impl ManageConnection for SqliteConnectionManager {
    type Connection = SqliteConnection;
    type Error = ConnectionManagerError;

    fn connect(&self) -> Result<Self::Connection, Self::Error> {
        match &self.source {
            SqliteConnectionSource::Legacy(manager) => manager.connect(),
            SqliteConnectionSource::Descriptor(source) => {
                let _guard = source
                    .connect_lock
                    .lock()
                    .map_err(|_| sqlite_manager_error("descriptor connect lock is poisoned"))?;
                descriptor_integrity_latch(source)
                    .map_err(|error| sqlite_manager_error(error.to_string()))?;
                source
                    .connection_proofs
                    .lock()
                    .map_err(|_| {
                        sqlite_manager_error("descriptor connection-proof lock is poisoned")
                    })?
                    .retain(|_, proof| {
                        proof.handles.validate(&proof.expected_objects).is_ok()
                            && FileObjectIdentity::from_file(&proof.shared_shm_anchor)
                                .map(|identity| {
                                    identity
                                        == proof.expected_objects.identity(SqliteObjectRole::Shm)
                                })
                                .unwrap_or(false)
                    });
                validate_retained_namespace(source)?;
                let before = ProcessDescriptorSnapshot::capture()
                    .map_err(descriptor_attestation_manager_error)?;
                let route = sqlite_open_route(source)?;
                let manager = ConnectionManager::<SqliteConnection>::new(format!(
                    "file:{}?mode=rw",
                    route.to_string_lossy()
                ));
                let mut connection = manager.connect()?;
                configure_sqlite_connection(&mut connection)?;
                let actual = Self::verify_descriptor_connection(source, &mut connection, &route)?;
                if actual != source.identity {
                    return Err(sqlite_manager_error(
                        "descriptor_identity_changed: SQLite main connection escaped owner inode",
                    ));
                }
                let journal_mode = diesel::sql_query("PRAGMA journal_mode")
                    .get_result::<JournalModeRow>(&mut connection)
                    .map_err(|error| sqlite_configuration_error("read journal_mode", error))?
                    .journal_mode;
                validate_wal_journal_mode(&journal_mode)
                    .map_err(descriptor_attestation_manager_error)?;
                diesel::sql_query("BEGIN IMMEDIATE")
                    .execute(&mut connection)
                    .map_err(|error| sqlite_configuration_error("prime WAL begin", error))?;
                diesel::sql_query("ROLLBACK")
                    .execute(&mut connection)
                    .map_err(|error| sqlite_configuration_error("prime WAL rollback", error))?;
                let after = ProcessDescriptorSnapshot::capture()
                    .map_err(descriptor_attestation_manager_error)?;
                // Open pins only after the fd snapshot. Otherwise our own
                // no-follow pins would enter the delta and make ownership
                // ambiguous.
                let observed_objects = capture_pinned_sqlite_object_set(source)?;
                let existing_evidence = current_descriptor_pool_evidence(source)?;
                let expected_objects = match existing_evidence.as_ref() {
                    Some(existing) if existing.expected_objects.as_ref() != &observed_objects => {
                        return Err(sqlite_manager_error(
                            "descriptor_identity_changed: SQLite main/WAL/SHM objects changed between connections",
                        ));
                    }
                    Some(existing) => Arc::clone(&existing.expected_objects),
                    None => Arc::new(observed_objects),
                };
                let handles = AttestedSqliteHandles::from_delta_with_shared_shm(
                    &before,
                    &after,
                    &expected_objects,
                    existing_evidence
                        .as_ref()
                        .map(|evidence| evidence.shared_shm_anchor.as_ref()),
                )
                .map_err(descriptor_attestation_manager_error)?;
                handles
                    .validate(&expected_objects)
                    .map_err(descriptor_attestation_manager_error)?;
                let shared_shm_anchor = match existing_evidence {
                    Some(evidence) => evidence.shared_shm_anchor,
                    None => Arc::new(open_shared_shm_anchor(source, &expected_objects)?),
                };
                let pool_evidence = commit_descriptor_pool_evidence(
                    source,
                    DescriptorPoolEvidence {
                        expected_objects,
                        shared_shm_anchor,
                    },
                )?;
                validate_retained_namespace(source)?;
                Self::verify_descriptor_connection(source, &mut connection, &route)?;
                let token = format!(
                    "descriptor-source-{}-connection-{:016x}",
                    source.registration_namespace,
                    source.next_connection_id.fetch_add(1, Ordering::Relaxed)
                );
                install_connection_attestation_token(&mut connection, &token)?;
                source
                    .connection_proofs
                    .lock()
                    .map_err(|_| {
                        sqlite_manager_error("descriptor connection-proof lock is poisoned")
                    })?
                    .insert(
                        token,
                        DescriptorConnectionProof {
                            handles,
                            expected_objects: Arc::clone(&pool_evidence.expected_objects),
                            shared_shm_anchor: Arc::clone(&pool_evidence.shared_shm_anchor),
                        },
                    );
                validate_live_registered_descriptor_connection(source, &mut connection)?;
                Ok(connection)
            }
        }
    }

    fn is_valid(&self, connection: &mut Self::Connection) -> Result<(), Self::Error> {
        match &self.source {
            SqliteConnectionSource::Legacy(manager) => manager.is_valid(connection),
            SqliteConnectionSource::Descriptor(source) => {
                validate_live_registered_descriptor_connection(source, connection)?;
                source.health.is_valid(connection)
            }
        }
    }

    fn has_broken(&self, connection: &mut Self::Connection) -> bool {
        match &self.source {
            SqliteConnectionSource::Legacy(manager) => manager.has_broken(connection),
            SqliteConnectionSource::Descriptor(source) => source.health.has_broken(connection),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SqliteConnectionConfiguration {
    foreign_keys: i32,
    synchronous: i32,
    busy_timeout: i32,
    wal_autocheckpoint: i32,
}

#[derive(Debug)]
struct SqliteConnectionCustomizer;

#[derive(Debug)]
struct ReadonlySqliteConnectionCustomizer;

const SQLITE_POOL_SIZE: u32 = 10;
const REQUIRED_SQLITE_CONFIGURATION: SqliteConnectionConfiguration =
    SqliteConnectionConfiguration {
        foreign_keys: 1,
        synchronous: 2,
        busy_timeout: 5_000,
        wal_autocheckpoint: 1_000,
    };

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReadonlySqliteConnectionConfiguration {
    foreign_keys: i32,
    busy_timeout: i32,
    query_only: i32,
}

const REQUIRED_READONLY_SQLITE_CONFIGURATION: ReadonlySqliteConnectionConfiguration =
    ReadonlySqliteConnectionConfiguration {
        foreign_keys: 1,
        busy_timeout: 5_000,
        query_only: 1,
    };

fn validate_required_text(field: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{field} 不能为空"))
    } else {
        Ok(())
    }
}

fn validate_date_text(field: &str, value: &str) -> Result<(), String> {
    validate_required_text(field, value)?;
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map(|_| ())
        .map_err(|error| format!("{field} 不是合法 YYYY-MM-DD 日期: {value}: {error}"))
}

fn validate_evidence_code(code: &str) -> Result<(), String> {
    validate_required_text("stock_code", code)?;
    if !code
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        return Err(format!("stock_code 含非法字符: {code:?}"));
    }
    crate::risk::env_guard::validate_symbol_for_current_env(code)
}

fn invalid_input(error: String) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, error)
}

fn configure_sqlite_connection(conn: &mut SqliteConnection) -> Result<(), ConnectionManagerError> {
    for (label, statement) in [
        ("foreign_keys=ON", "PRAGMA foreign_keys = ON"),
        ("busy_timeout=5000", "PRAGMA busy_timeout = 5000"),
        ("synchronous=FULL", "PRAGMA synchronous = FULL"),
        (
            "wal_autocheckpoint=1000",
            "PRAGMA wal_autocheckpoint = 1000",
        ),
    ] {
        diesel::sql_query(statement)
            .execute(conn)
            .map_err(|error| sqlite_configuration_error(label, error))?;
    }
    let actual = read_sqlite_connection_configuration(conn)?;
    if actual != REQUIRED_SQLITE_CONFIGURATION {
        return Err(sqlite_configuration_mismatch(actual));
    }
    Ok(())
}

fn configure_readonly_sqlite_connection(
    conn: &mut SqliteConnection,
) -> Result<(), ConnectionManagerError> {
    for (label, statement) in [
        ("query_only=ON", "PRAGMA query_only = ON"),
        ("foreign_keys=ON", "PRAGMA foreign_keys = ON"),
        ("busy_timeout=5000", "PRAGMA busy_timeout = 5000"),
    ] {
        diesel::sql_query(statement)
            .execute(conn)
            .map_err(|error| sqlite_configuration_error(label, error))?;
    }
    let actual = ReadonlySqliteConnectionConfiguration {
        foreign_keys: diesel::sql_query("PRAGMA foreign_keys")
            .get_result::<ForeignKeysPragmaRow>(conn)
            .map_err(|error| sqlite_configuration_error("read foreign_keys", error))?
            .foreign_keys,
        busy_timeout: diesel::sql_query("PRAGMA busy_timeout")
            .get_result::<BusyTimeoutPragmaRow>(conn)
            .map_err(|error| sqlite_configuration_error("read busy_timeout", error))?
            .timeout,
        query_only: diesel::sql_query("PRAGMA query_only")
            .get_result::<QueryOnlyPragmaRow>(conn)
            .map_err(|error| sqlite_configuration_error("read query_only", error))?
            .query_only,
    };
    if actual != REQUIRED_READONLY_SQLITE_CONFIGURATION {
        return Err(sqlite_manager_error(format!(
            "read-only SQLite PRAGMA verification failed: expected {REQUIRED_READONLY_SQLITE_CONFIGURATION:?}, got {actual:?}"
        )));
    }
    Ok(())
}

fn read_sqlite_connection_configuration(
    conn: &mut SqliteConnection,
) -> Result<SqliteConnectionConfiguration, ConnectionManagerError> {
    let foreign_keys = diesel::sql_query("PRAGMA foreign_keys")
        .get_result::<ForeignKeysPragmaRow>(conn)
        .map_err(|error| sqlite_configuration_error("read foreign_keys", error))?
        .foreign_keys;
    let synchronous = diesel::sql_query("PRAGMA synchronous")
        .get_result::<SynchronousPragmaRow>(conn)
        .map_err(|error| sqlite_configuration_error("read synchronous", error))?
        .synchronous;
    let busy_timeout = diesel::sql_query("PRAGMA busy_timeout")
        .get_result::<BusyTimeoutPragmaRow>(conn)
        .map_err(|error| sqlite_configuration_error("read busy_timeout", error))?
        .timeout;
    let wal_autocheckpoint = diesel::sql_query("PRAGMA wal_autocheckpoint")
        .get_result::<WalAutocheckpointPragmaRow>(conn)
        .map_err(|error| sqlite_configuration_error("read wal_autocheckpoint", error))?
        .wal_autocheckpoint;
    Ok(SqliteConnectionConfiguration {
        foreign_keys,
        synchronous,
        busy_timeout,
        wal_autocheckpoint,
    })
}

fn sqlite_configuration_error(label: &str, error: diesel::result::Error) -> ConnectionManagerError {
    ConnectionManagerError::QueryError(diesel::result::Error::QueryBuilderError(Box::new(
        std::io::Error::other(format!("SQLite PRAGMA {label} failed: {error}")),
    )))
}

fn sqlite_configuration_mismatch(actual: SqliteConnectionConfiguration) -> ConnectionManagerError {
    ConnectionManagerError::QueryError(diesel::result::Error::QueryBuilderError(Box::new(
        std::io::Error::other(format!(
            "SQLite PRAGMA verification failed: expected {REQUIRED_SQLITE_CONFIGURATION:?}, got {actual:?}"
        )),
    )))
}

fn sqlite_manager_error(detail: impl Into<String>) -> ConnectionManagerError {
    ConnectionManagerError::QueryError(diesel::result::Error::QueryBuilderError(Box::new(
        std::io::Error::other(detail.into()),
    )))
}

impl CustomizeConnection<SqliteConnection, ConnectionManagerError> for SqliteConnectionCustomizer {
    fn on_acquire(&self, conn: &mut SqliteConnection) -> Result<(), ConnectionManagerError> {
        configure_sqlite_connection(conn)
    }
}

impl CustomizeConnection<SqliteConnection, ConnectionManagerError>
    for ReadonlySqliteConnectionCustomizer
{
    fn on_acquire(&self, conn: &mut SqliteConnection) -> Result<(), ConnectionManagerError> {
        configure_readonly_sqlite_connection(conn)
    }
}

fn build_sqlite_pool(database_url: String) -> Result<DbPool, PoolError> {
    build_sqlite_pool_with_size(database_url, SQLITE_POOL_SIZE)
}

fn build_sqlite_pool_with_size(database_url: String, max_size: u32) -> Result<DbPool, PoolError> {
    build_sqlite_pool_from_manager(SqliteConnectionManager::legacy(database_url), max_size)
}

fn build_readonly_sqlite_pool_with_size(
    database_url: String,
    max_size: u32,
) -> Result<DbPool, PoolError> {
    Pool::builder()
        .max_size(max_size)
        .test_on_check_out(true)
        .connection_customizer(Box::new(ReadonlySqliteConnectionCustomizer))
        .build(SqliteConnectionManager::legacy(database_url))
}

fn build_attested_sqlite_pool_with_size(
    database_path: &Path,
    max_size: u32,
) -> Result<(DbPool, Arc<DescriptorSqliteSource>), Box<dyn std::error::Error>> {
    let parent_path = database_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let leaf = database_path.file_name().ok_or_else(|| {
        std::io::Error::other("descriptor-attested SQLite database has no leaf component")
    })?;
    let parent = File::open(parent_path)?;
    let root = parent.try_clone()?;
    let database_file = openat_regular_for_attestation(&parent, leaf, "SQLite main database")?;
    let pinned = PinnedSqliteDatabase::from_owner_descriptors(
        root,
        parent,
        leaf.to_os_string(),
        PathBuf::from(leaf),
        database_file,
    )?;
    let manager = SqliteConnectionManager::descriptor(pinned)?;
    let source = manager.descriptor_source().ok_or_else(|| {
        DatabaseAuthorityError::DescriptorAttestationUnavailable {
            detail: "descriptor manager did not retain its attestation source".into(),
        }
    })?;
    let pool = build_attribution_sqlite_pool_from_manager(manager, max_size)?;
    Ok((pool, source))
}

fn build_sqlite_pool_from_manager(
    manager: SqliteConnectionManager,
    max_size: u32,
) -> Result<DbPool, PoolError> {
    build_sqlite_pool_from_manager_with_checkout_validation(manager, max_size, true)
}

fn build_attribution_sqlite_pool_from_manager(
    manager: SqliteConnectionManager,
    max_size: u32,
) -> Result<DbPool, PoolError> {
    build_sqlite_pool_from_manager_with_checkout_validation(manager, max_size, false)
}

fn build_sqlite_pool_from_manager_with_checkout_validation(
    manager: SqliteConnectionManager,
    max_size: u32,
    test_on_check_out: bool,
) -> Result<DbPool, PoolError> {
    Pool::builder()
        .max_size(max_size)
        .test_on_check_out(test_on_check_out)
        .connection_customizer(Box::new(SqliteConnectionCustomizer))
        .build(manager)
}

pub mod factor_snapshot;
pub mod repository;
// v12 MVP-5 §8.1
pub(crate) mod agent_logs;
pub mod attribution_epochs;
pub mod attribution_reports;
pub mod benchmark_segments;
pub mod chain_intelligence;
pub mod concepts; // v15.1: 公开供 push_templates 集成使用
pub mod daily_change_confirmation;
pub mod data_acquisition_audit;
pub mod execution_tracking;
pub(crate) mod global_schema_catalog_v1;
pub(crate) mod global_schema_v1;
mod kline;
mod lhb;
pub mod news_ai;
pub mod order_audit;
pub(crate) mod paper_inventory_failure_audit;
pub mod position_chain;
mod positions;
// BR-215: projection reconciliation is a tool-facing entry point.
pub use positions::{reconcile_stock_position_from_confirmed_snapshot, PositionReconciliation};
mod sqlite_descriptor_attestation;
// v12 PR1-1.5 (BR-021)
pub mod account_mode_log;
/// BR-103 real-account evidence boundary; nullable fields stay nullable.
pub mod account_snapshot;
// v12 PR3-3.2/3.3 (BR-023/024)
pub mod catalyst_watchlist;
pub mod closing_valuation;
pub mod position_shares;
pub mod selection;
pub mod selection_v2;
pub(crate) mod selection_v2_generation_journal;
pub mod selection_v2_read_model;
pub mod selection_v2_repository;
pub mod user_account_summary;
pub mod user_position_snapshot;

/// BR-180 migration operator façade.
///
/// The binary receives only rendered diagnostics. The global schema owner
/// retains every lock, descriptor, SQLite transaction and audit snapshot; no
/// raw migration authority crosses this public boundary.
pub fn run_selection_v2_migration_command<I, S>(args: I) -> Result<String, String>
where
    I: IntoIterator<Item = S>,
    S: Into<std::ffi::OsString>,
{
    global_schema_v1::run_selection_v2_migration_command(args)
}

// ============================================================================
// 数据库管理器 - 单例模式
// ============================================================================

impl DatabaseManager {
    /// 初始化数据库管理器
    ///
    /// # Arguments
    ///
    /// * `db_path` - 数据库文件路径（如果为None，默认使用 "./data/stock.db"）
    pub fn init(db_path: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
        #[cfg(test)]
        let _init_guard = unit_test_init_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        #[cfg(test)]
        if DB_INSTANCE.get().is_some() {
            return Ok(());
        }

        #[cfg(test)]
        let path = {
            let _ = db_path;
            unit_test_database_path().clone()
        };

        #[cfg(not(test))]
        let path = db_path.unwrap_or_else(|| {
            let mut p = PathBuf::from("./data");
            std::fs::create_dir_all(&p).ok();
            p.push("stock.db");
            p
        });

        let database_url = path.to_string_lossy().to_string();
        info!("初始化数据库: {}", database_url);

        // WAL is database-wide and requires a lock. Configure it once before
        // r2d2 opens connections concurrently.
        let mut bootstrap_conn = SqliteConnection::establish(&database_url)?;
        diesel::sql_query("PRAGMA busy_timeout = 5000").execute(&mut bootstrap_conn)?;
        let journal_mode = diesel::sql_query("PRAGMA journal_mode = WAL")
            .get_result::<JournalModeRow>(&mut bootstrap_conn)?
            .journal_mode;
        if !journal_mode.eq_ignore_ascii_case("wal") {
            return Err(
                format!("SQLite journal_mode mismatch: expected WAL, got {journal_mode}").into(),
            );
        }
        configure_sqlite_connection(&mut bootstrap_conn)?;
        drop(bootstrap_conn);

        let (attribution_pool, attribution_connection_source) =
            match build_attested_sqlite_pool_with_size(&path, 2) {
                Ok((pool, source)) => (Some(pool), Some(source)),
                Err(error) => {
                    log::warn!(
                        "BR-255 attribution descriptor attestation unavailable; operational database remains active: {error}"
                    );
                    (None, None)
                }
            };
        let pool = build_sqlite_pool(database_url)?;

        // 运行迁移
        let mut conn = pool.get()?;
        configure_sqlite_connection(&mut conn)?;
        Self::run_migrations(&mut conn)?;

        info!(
            "SQLite PRAGMAs 已设置: WAL + foreign_keys=ON + synchronous=FULL + busy_timeout=5000"
        );

        // 创建 agent_scratchpad 表 (Agent 内部思考和工具执行记录)
        diesel::sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS agent_scratchpad (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                step INTEGER NOT NULL,
                log_type TEXT NOT NULL,
                content TEXT,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&mut *conn)?;
        diesel::sql_query(
            r#"
            CREATE TRIGGER IF NOT EXISTS agent_scratchpad_no_update
            BEFORE UPDATE ON agent_scratchpad
            BEGIN
                SELECT RAISE(ABORT, 'agent_scratchpad is append-only');
            END;
            "#,
        )
        .execute(&mut *conn)?;
        diesel::sql_query(
            r#"
            CREATE TRIGGER IF NOT EXISTS agent_scratchpad_no_delete
            BEFORE DELETE ON agent_scratchpad
            BEGIN
                SELECT RAISE(ABORT, 'agent_scratchpad is append-only');
            END;
            "#,
        )
        .execute(&mut *conn)?;

        // BR-126 / v16.x R3: the push pool is a durable audit boundary shared by
        // push_recorder and both intraday/evening consumers. Initialization must
        // fail if any part of the table/index contract cannot be installed.
        diesel::sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS pushed_stocks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                push_time TIMESTAMP NOT NULL,
                push_kind TEXT NOT NULL,
                code TEXT NOT NULL,
                name TEXT NOT NULL,
                push_price REAL NOT NULL,
                metric_json TEXT NOT NULL,
                source TEXT NOT NULL,
                consumed_at TIMESTAMP,
                consumed_by TEXT,
                outcome TEXT,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&mut *conn)?;
        for statement in [
            "CREATE INDEX IF NOT EXISTS idx_pushed_stocks_time ON pushed_stocks (push_time, push_kind)",
            "CREATE INDEX IF NOT EXISTS idx_pushed_stocks_code ON pushed_stocks (code, push_time)",
            "CREATE INDEX IF NOT EXISTS idx_pushed_stocks_uncon ON pushed_stocks (consumed_at) WHERE consumed_at IS NULL",
        ] {
            diesel::sql_query(statement).execute(&mut *conn)?;
        }

        // 创建 stock_concepts 表（概念板块标签缓存，产业链聚类用）
        diesel::sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS stock_concepts (
                code TEXT PRIMARY KEY,
                concepts TEXT NOT NULL,
                updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&mut *conn)?;

        // 创建 chain_daily 表（每日涨停主线簇，供单股分析注入主线上下文 + 主线生命周期追踪）
        diesel::sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS chain_daily (
                date TEXT NOT NULL,
                concept TEXT NOT NULL,
                stocks TEXT NOT NULL,
                continuation_count INTEGER NOT NULL DEFAULT 0,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (date, concept)
            )
            "#,
        )
        .execute(&mut *conn)?;

        // B-002 板块联动归因 (Board hit) 落库表 — 与 chain_daily 并列,
        //       供 NewsCatalyst 推送读取今日 top cluster.
        diesel::sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS board_rotation_daily (
                date TEXT NOT NULL,
                board_code TEXT NOT NULL,
                board_name TEXT NOT NULL,
                news_title TEXT NOT NULL,
                board_change_pct REAL NOT NULL DEFAULT 0,
                board_main_net_pct REAL NOT NULL DEFAULT 0,
                stocks TEXT NOT NULL,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (date, board_code)
            )
            "#,
        )
        .execute(&mut *conn)?;

        // B-003 事件抽取去重 (simhash + LCS) — 跨批次跨日去重,
        //       防「苹果折叠屏」类事件在 3+ 天内重复推送.
        diesel::sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS event_seen_simhash (
                simhash INTEGER NOT NULL,
                title TEXT NOT NULL,
                seen_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (simhash)
            )
            "#,
        )
        .execute(&mut *conn)?;
        // CR-8 (review): get_recent_event_seen 用 `WHERE seen_at >= ?` 全表扫,
        //              表行数 > 5000 时变慢. 加 (seen_at) 索引.
        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS idx_event_seen_simhash_seen_at \
             ON event_seen_simhash (seen_at)",
        )
        .execute(&mut *conn)?;

        // 主题新闻去同质化历史（跨重启持久化）
        diesel::sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS topic_novelty_history (
                signature TEXT PRIMARY KEY,
                created_at INTEGER NOT NULL
            )
            "#,
        )
        .execute(&mut *conn)?;
        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS ix_topic_novelty_created_at ON topic_novelty_history(created_at)",
        )
        .execute(&mut *conn)?;

        drop(conn);
        let db = DatabaseManager {
            pool,
            attribution_pool,
            attribution_connection_source,
            readonly_attribution_snapshot: None,
            selection_connection_source: None,
            selection_schema_authority: None,
        };
        DB_INSTANCE.set(db).map_err(|_| "数据库已经初始化")?;
        info!("数据库初始化完成");

        Ok(())
    }

    /// Construct the operational pool from the opaque GlobalSchema owner
    /// capability. No caller path, canonical path, environment value, or CWD
    /// participates in this binding.
    ///
    /// Unlike the legacy initializer this does not run DDL: the capability is
    /// issued only after the owner verified the exact final catalog and its
    /// audit receipt closure inside one retained snapshot.
    #[allow(dead_code)]
    pub(crate) fn from_verified_amended_selection_schema(
        authority: Box<global_schema_v1::VerifiedAmendedSelectionSchema>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let database = authority.pinned_database_for_pool()?;
        let manager = SqliteConnectionManager::descriptor(database)?;
        let selection_connection_source = manager.descriptor_source().ok_or_else(|| {
            DatabaseAuthorityError::DescriptorAttestationUnavailable {
                detail: "descriptor manager did not retain its attestation source".into(),
            }
        })?;
        let attribution_pool = build_attribution_sqlite_pool_from_manager(manager.clone(), 2)?;
        let pool = build_sqlite_pool_from_manager(manager, SQLITE_POOL_SIZE)?;
        {
            let mut connection = pool.get()?;
            configure_sqlite_connection(&mut connection)?;
        }
        Ok(Self {
            pool,
            attribution_pool: Some(attribution_pool),
            attribution_connection_source: Some(Arc::clone(&selection_connection_source)),
            readonly_attribution_snapshot: None,
            selection_connection_source: Some(selection_connection_source),
            selection_schema_authority: Some(authority),
        })
    }

    fn selection_connection_bound_identity(
        &self,
        connection: &mut DbConnection,
    ) -> Result<SqliteFileIdentity, DatabaseAuthorityError> {
        let source = self.selection_connection_source.as_ref().ok_or_else(|| {
            DatabaseAuthorityError::DescriptorAttestationUnavailable {
                detail: "selection checkout has no descriptor connection source".into(),
            }
        })?;
        registered_descriptor_connection_identity(source, &mut *connection)
    }

    /// Proves the exact checkout used by an authoritative selection read is
    /// attached to the GlobalSchema owner's retained database inode.
    ///
    /// The returned material is derived from retained descriptors plus
    /// `PRAGMA database_list` on `connection`; no caller path can mint it.
    #[cfg(not(test))]
    pub(super) fn selection_connection_bound_proof(
        &self,
        connection: &mut DbConnection,
    ) -> Result<SelectionConnectionBoundProof, Box<dyn std::error::Error>> {
        let authority = self.selection_schema_authority.as_ref().ok_or_else(|| {
            std::io::Error::other(
                "authoritative selection reads require amended-schema descriptor authority",
            )
        })?;
        let pinned = authority.pinned_database_for_pool()?;
        let actual = self.selection_connection_bound_identity(connection)?;
        let source = self.selection_connection_source.as_ref().ok_or_else(|| {
            std::io::Error::other(
                "authoritative selection reads require descriptor connection source",
            )
        })?;
        let evidence = current_descriptor_pool_evidence(source)?.ok_or_else(|| {
            std::io::Error::other("authoritative selection pool has no descriptor evidence")
        })?;
        let attested_main = evidence.expected_objects.identity(SqliteObjectRole::Main);
        if attested_main.device() != actual.device
            || attested_main.inode() != actual.inode
            || attested_main.mode() != pinned.database_object_identity.mode
        {
            return Err(std::io::Error::other(
                "authoritative selection checkout descriptor proof changed identity",
            )
            .into());
        }
        let root_metadata = pinned.root.metadata()?;
        let parent_metadata = pinned.parent.metadata()?;
        let database_metadata = pinned.database_file.metadata()?;
        let root_identity = SqliteObjectIdentity::from_metadata(&root_metadata);
        let parent_identity = SqliteObjectIdentity::from_metadata(&parent_metadata);
        let database_identity = SqliteObjectIdentity::from_metadata(&database_metadata);
        if !root_metadata.is_dir()
            || !parent_metadata.is_dir()
            || !database_metadata.is_file()
            || SqliteFileIdentity::from_metadata(&database_metadata) != actual
            || root_identity != pinned.root_identity
            || parent_identity != pinned.parent_identity
            || database_identity != pinned.database_object_identity
        {
            return Err(std::io::Error::other(
                "selection connection proof changed retained descriptor identity",
            )
            .into());
        }
        let database_relative_identity = pinned.relative_identity.to_str().ok_or_else(|| {
            std::io::Error::other(
                "owner-fixed database relative identity cannot be represented as UTF-8",
            )
        })?;
        Ok(SelectionConnectionBoundProof {
            root: root_identity,
            parent: parent_identity,
            database: database_identity,
            database_relative_identity: database_relative_identity.to_owned(),
        })
    }

    /// Acquires a branded attribution checkout and its matching descriptor
    /// source from this manager. No API accepts a caller-supplied connection.
    pub(crate) fn attribution_checkout(
        &self,
    ) -> Result<AttestedAttributionCheckout, DatabaseAuthorityError> {
        let pool = self.attribution_pool.as_ref().ok_or_else(|| {
            DatabaseAuthorityError::DescriptorAttestationUnavailable {
                detail: "manager has no descriptor-attested attribution pool".into(),
            }
        })?;
        let source = self.attribution_connection_source.as_ref().ok_or_else(|| {
            DatabaseAuthorityError::DescriptorAttestationUnavailable {
                detail: "manager has no descriptor-attested attribution source".into(),
            }
        })?;
        descriptor_integrity_latch(source)?;
        validate_attribution_source_namespace(source)?;
        let connection = match pool.get() {
            Ok(connection) => connection,
            Err(error) => {
                descriptor_integrity_latch(source)?;
                validate_attribution_source_namespace(source)?;
                return Err(DatabaseAuthorityError::DescriptorAttestationUnavailable {
                    detail: format!(
                        "descriptor-attested attribution checkout unavailable: {error}"
                    ),
                });
            }
        };
        let mut checkout = AttestedAttributionCheckout {
            connection,
            source: Arc::clone(source),
        };
        checkout.authority()?;
        Ok(checkout)
    }

    /// Runs one deferred attribution read transaction from this manager's
    /// owned database. Append-only managers use their descriptor-attested
    /// checkout. Read-only CLI managers use only the primary pool backed by
    /// their session-owned detached snapshot. The callback cannot supply or
    /// switch a pathname.
    pub(crate) fn attribution_read_transaction<T, E, F>(
        &self,
        operation: F,
    ) -> Result<T, AttributionReadTransactionError<E>>
    where
        F: FnOnce(&mut SqliteConnection) -> Result<T, E>,
    {
        if self.attribution_pool.is_some() || self.attribution_connection_source.is_some() {
            let mut checkout = self
                .attribution_checkout()
                .map_err(AttributionReadTransactionError::Authority)?;
            checkout.transaction_with_authority(
                AttributionReadTransactionError::Authority,
                |connection, _authority| {
                    operation(connection).map_err(AttributionReadTransactionError::Operation)
                },
            )
        } else if self.readonly_attribution_snapshot.is_some() {
            let mut connection = self.get_conn().map_err(|error| {
                AttributionReadTransactionError::StorageUnavailable {
                    detail: error.to_string(),
                }
            })?;
            let query_only = diesel::sql_query("PRAGMA query_only")
                .get_result::<QueryOnlyPragmaRow>(&mut connection)?
                .query_only;
            if query_only != 1 {
                return Err(AttributionReadTransactionError::SnapshotIntegrity {
                    detail: format!(
                        "session-owned attribution snapshot has invalid query_only={query_only}"
                    ),
                });
            }
            connection.transaction(|connection| {
                operation(connection).map_err(AttributionReadTransactionError::Operation)
            })
        } else {
            let mut connection = self.get_conn().map_err(|error| {
                AttributionReadTransactionError::StorageUnavailable {
                    detail: error.to_string(),
                }
            })?;
            connection.transaction(|connection| {
                operation(connection).map_err(AttributionReadTransactionError::Operation)
            })
        }
    }

    #[cfg(test)]
    pub(crate) fn retains_verified_selection_authority(&self) -> bool {
        self.selection_schema_authority.is_some()
    }

    /// 获取数据库管理器单例
    pub fn get() -> &'static DatabaseManager {
        DB_INSTANCE
            .get()
            .expect("数据库未初始化，请先调用 DatabaseManager::init()")
    }

    /// 尝试获取数据库管理器单例（返回 Option，不 panic）.
    /// review #14: 取代之前各处 `catch_unwind(DatabaseManager::get)` 的反 pattern.
    /// catch_unwind 强制 panic = unwind + 还要 AssertUnwindSafe wrap, 而且静默吞
    /// init 失败, 让 operator 看到「数据全空」但不知道 DB 没起来.
    /// 显式 Option 让调用方必须处理 None 路径 (早返回 / log warn).
    pub fn try_get() -> Option<&'static DatabaseManager> {
        DB_INSTANCE.get()
    }

    /// 在 DB 已初始化前提下执行闭包; 否则记录一次 warn 并返回 None.
    /// review #15: 取代 13+ 处 `let Some(db) = DatabaseManager::try_get() else { return; };`
    /// 重复模板. 调用方写 `DatabaseManager::with_db(|db| { ... })?` 比手写 Option 处理更清晰.
    ///
    /// 闭包返回 `Option<T>` 表示 DB 操作本身的成功/失败 (None = 操作失败/缺数据, 不一定是 DB 不可用).
    /// 用 `Once` 状态确保 DB 未初始化只 warn 一次 (避免每 tick 重复刷屏).
    pub fn with_db<F, T>(caller: &str, f: F) -> Option<T>
    where
        F: FnOnce(&DatabaseManager) -> Option<T>,
    {
        match DB_INSTANCE.get() {
            Some(db) => f(db),
            None => {
                use std::sync::atomic::{AtomicBool, Ordering};
                static WARNED: AtomicBool = AtomicBool::new(false);
                if !WARNED.swap(true, Ordering::Relaxed) {
                    log::warn!(
                        "[{}] DatabaseManager 未初始化, 跳过 (后续同路径 DB 错误不再 warn)",
                        caller
                    );
                }
                None
            }
        }
    }

    /// 获取数据库连接
    pub fn get_conn(&self) -> Result<DbConnection, Box<dyn std::error::Error>> {
        let mut conn = self.pool.get()?;
        if self.readonly_attribution_snapshot.is_some() {
            configure_readonly_sqlite_connection(&mut conn)?;
        } else {
            configure_sqlite_connection(&mut conn)?;
        }
        Ok(conn)
    }

    /// 给已存在的表增量添加列（如果列不存在）。
    /// SQLite 没有原生的 `ADD COLUMN IF NOT EXISTS`；通过 PRAGMA table_info 读列名判断。
    /// 用于把老库升级到新 schema，不破坏现有数据。
    ///
    /// 修复 (2026-07-05 MVP0-A): 如果表本身不存在 (CREATE 还没跑到), 静默跳过,
    ///   等表建好后再 ALTER. 避免 "no such table: X" 错误导致 init 失败.
    pub fn add_column_if_missing(
        conn: &mut SqliteConnection,
        table: &str,
        column: &str,
        column_def: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if !table_exists(conn, table)? {
            // 表还没建, 跳过. 等 CREATE TABLE 之后再回头补.
            return Ok(());
        }
        if column_exists(conn, table, column)? {
            return Ok(());
        }
        let alter = format!("ALTER TABLE {} ADD COLUMN {} {}", table, column, column_def);
        diesel::sql_query(&alter).execute(conn)?;
        Ok(())
    }

    /// review #16: news_items 详存 (与 news_dedup 5min 去重互补, 永久详存)
    pub const NEWS_ITEMS_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS news_items (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source TEXT NOT NULL,
    external_id TEXT NOT NULL,
    category TEXT NOT NULL,
    code TEXT,
    title TEXT NOT NULL,
    summary TEXT,
    url TEXT NOT NULL,
    source_name TEXT,
    published_at INTEGER NOT NULL,
    fetched_at INTEGER NOT NULL,
    content_hash TEXT NOT NULL,
    UNIQUE(source, external_id)
);
CREATE INDEX IF NOT EXISTS idx_news_items_code_time ON news_items(code, published_at);
CREATE INDEX IF NOT EXISTS idx_news_items_published ON news_items(published_at);
"#;

    /// 运行数据库迁移
    #[cfg(test)]
    pub(crate) fn run_migrations_for_test(
        conn: &mut SqliteConnection,
    ) -> Result<(), Box<dyn std::error::Error>> {
        Self::run_migrations(conn)
    }

    fn run_migrations(conn: &mut SqliteConnection) -> Result<(), Box<dyn std::error::Error>> {
        user_position_snapshot::create_schema(conn).map_err(std::io::Error::other)?;
        user_account_summary::create_schema(conn).map_err(std::io::Error::other)?;
        closing_valuation::create_schema(conn).map_err(std::io::Error::other)?;
        catalyst_watchlist::create_schema(conn).map_err(std::io::Error::other)?;
        selection::create_schema(conn).map_err(std::io::Error::other)?;
        chain_intelligence::create_schema(conn).map_err(std::io::Error::other)?;
        // 创建 stock_daily 表
        diesel::sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS stock_daily (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                code TEXT NOT NULL,
                date DATE NOT NULL,
                open REAL,
                high REAL,
                low REAL,
                close REAL,
                volume REAL,
                amount REAL,
                pct_chg REAL,
                ma5 REAL,
                ma10 REAL,
                ma20 REAL,
                volume_ratio REAL,
                data_source TEXT,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                is_limit_up TINYINT NOT NULL DEFAULT 0,
                is_limit_down TINYINT NOT NULL DEFAULT 0,
                is_suspended TINYINT NOT NULL DEFAULT 0,
                UNIQUE(code, date)
            )
            "#,
        )
        .execute(&mut *conn)?;

        // 老库升级：增量添加 3 列（QUANT_ANALYST_REVIEW §1.1）
        Self::add_column_if_missing(
            conn,
            "stock_daily",
            "is_limit_up",
            "TINYINT NOT NULL DEFAULT 0",
        )?;
        Self::add_column_if_missing(
            conn,
            "stock_daily",
            "is_limit_down",
            "TINYINT NOT NULL DEFAULT 0",
        )?;
        Self::add_column_if_missing(
            conn,
            "stock_daily",
            "is_suspended",
            "TINYINT NOT NULL DEFAULT 0",
        )?;

        // 老库升级：增量添加 6 列 (修复 P1.3 trades 业绩归因)
        // 量化分析师要求: 必须能算真实 PnL (扣除 commission/stamp_tax/slippage)
        Self::add_column_if_missing(conn, "trades", "commission_amount", "REAL DEFAULT 0")?;
        Self::add_column_if_missing(conn, "trades", "stamp_tax_amount", "REAL DEFAULT 0")?;
        Self::add_column_if_missing(conn, "trades", "slippage_amount", "REAL DEFAULT 0")?;
        Self::add_column_if_missing(conn, "trades", "realized_pnl", "REAL DEFAULT 0")?;
        Self::add_column_if_missing(conn, "trades", "strategy_tag", "TEXT DEFAULT ''")?;
        Self::add_column_if_missing(conn, "trades", "signal_id", "TEXT DEFAULT ''")?;

        // 创建索引
        diesel::sql_query("CREATE INDEX IF NOT EXISTS ix_stock_daily_code ON stock_daily(code)")
            .execute(&mut *conn)?;

        diesel::sql_query("CREATE INDEX IF NOT EXISTS ix_stock_daily_date ON stock_daily(date)")
            .execute(&mut *conn)?;

        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS ix_stock_daily_code_date ON stock_daily(code, date)",
        )
        .execute(&mut *conn)?;

        // 创建 lhb_daily 表
        diesel::sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS lhb_daily (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                code TEXT NOT NULL,
                name TEXT NOT NULL,
                trade_date TEXT NOT NULL,
                reason TEXT NOT NULL,
                pct_change REAL NOT NULL,
                close_price REAL NOT NULL,
                buy_amount REAL NOT NULL,
                sell_amount REAL NOT NULL,
                net_amount REAL NOT NULL,
                total_amount REAL NOT NULL,
                lhb_ratio REAL NOT NULL,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                UNIQUE(code, trade_date)
            )
            "#,
        )
        .execute(&mut *conn)?;

        // 创建龙虎榜索引
        diesel::sql_query("CREATE INDEX IF NOT EXISTS ix_lhb_daily_code ON lhb_daily(code)")
            .execute(&mut *conn)?;

        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS ix_lhb_daily_trade_date ON lhb_daily(trade_date)",
        )
        .execute(&mut *conn)?;

        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS ix_lhb_daily_code_date ON lhb_daily(code, trade_date)",
        )
        .execute(&mut *conn)?;

        // 创建 analysis_result 表
        diesel::sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS analysis_result (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                code TEXT NOT NULL,
                name TEXT NOT NULL,
                date DATE NOT NULL,
                sentiment_score INTEGER NOT NULL,
                operation_advice TEXT NOT NULL,
                trend_prediction TEXT NOT NULL,
                pe_ratio REAL,
                pb_ratio REAL,
                turnover_rate REAL,
                market_cap REAL,
                circulating_cap REAL,
                close_price REAL,
                pct_chg REAL,
                data_source TEXT,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                UNIQUE(code, date)
            )
            "#,
        )
        .execute(&mut *conn)?;

        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS ix_analysis_result_code ON analysis_result(code)",
        )
        .execute(&mut *conn)?;

        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS ix_analysis_result_date ON analysis_result(date)",
        )
        .execute(&mut *conn)?;

        // Phase 1 增量：多维评分 + 风险否决（SQLite 不支持 IF NOT EXISTS，忽略已存在错误）
        for sql in [
            "ALTER TABLE analysis_result ADD COLUMN score_breakdown_json TEXT",
            "ALTER TABLE analysis_result ADD COLUMN original_advice TEXT",
            "ALTER TABLE analysis_result ADD COLUMN veto_flags_json TEXT",
        ] {
            let _ = diesel::sql_query(sql).execute(&mut *conn);
        }

        // 创建 stock_position 表（模拟持仓）
        diesel::sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS stock_position (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                code TEXT NOT NULL,
                name TEXT NOT NULL,
                buy_date TEXT NOT NULL,
                buy_price REAL NOT NULL CHECK(buy_price > 0),
                quantity INTEGER NOT NULL CHECK(quantity > 0 AND quantity % 100 = 0),
                status TEXT NOT NULL DEFAULT 'open',
                sell_date TEXT,
                sell_price REAL CHECK(sell_price IS NULL OR sell_price > 0),
                return_rate REAL,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                st_type TEXT,
                UNIQUE(code, buy_date)
            )
            "#,
        )
        .execute(&mut *conn)?;

        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS ix_stock_position_code ON stock_position(code)",
        )
        .execute(&mut *conn)?;

        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS ix_stock_position_status ON stock_position(status)",
        )
        .execute(&mut *conn)?;
        diesel::sql_query(
            "CREATE TRIGGER IF NOT EXISTS trg_stock_position_order_safety_insert
             BEFORE INSERT ON stock_position
             WHEN NEW.buy_price <= 0 OR NEW.quantity <= 0 OR NEW.quantity % 100 != 0
               OR NEW.buy_price * NEW.quantity > 1000000
               OR (NEW.sell_price IS NOT NULL AND NEW.sell_price <= 0)
             BEGIN SELECT RAISE(ABORT, 'BR-084 invalid stock_position order'); END",
        )
        .execute(&mut *conn)?;
        diesel::sql_query(
            "CREATE TRIGGER IF NOT EXISTS trg_stock_position_order_safety_update
             BEFORE UPDATE OF buy_price, quantity, sell_price ON stock_position
             WHEN NEW.buy_price <= 0 OR NEW.quantity <= 0 OR NEW.quantity % 100 != 0
               OR NEW.buy_price * NEW.quantity > 1000000
               OR (NEW.sell_price IS NOT NULL AND NEW.sell_price <= 0)
             BEGIN SELECT RAISE(ABORT, 'BR-084 invalid stock_position order'); END",
        )
        .execute(&mut *conn)?;

        // BR-123: stock_position.chain_name 缺失值必须保留为 NULL。
        // 旧库可能没有, 用 add_column_if_missing 包一层 (SQLite 1.06 无 ADD COLUMN IF NOT EXISTS)
        Self::add_column_if_missing(conn, "stock_position", "chain_name", "TEXT")?;
        diesel::sql_query(
            "UPDATE stock_position SET chain_name = NULL
             WHERE chain_name IS NOT NULL
               AND (trim(chain_name) = '' OR chain_name = '其他')",
        )
        .execute(&mut *conn)?;
        diesel::sql_query("CREATE INDEX IF NOT EXISTS idx_stock_position_chain_name ON stock_position(chain_name)")
            .execute(&mut *conn)?;

        // v14.1 F7: stock_position 加 st_type 列 (TEXT: 'ST' / '*ST' / NULL)
        // T-16 ST 涨跌幅变更 dispatcher 数据源. 由 --backfill-st-type 从 name 字段回填,
        // 后续 broker/exchange 推送时更新. 无 CHECK 约束 (SQLite ALTER ADD COLUMN 不支持)
        Self::add_column_if_missing(conn, "stock_position", "st_type", "TEXT")?;
        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS idx_stock_position_st_type ON stock_position(st_type)",
        )
        .execute(&mut *conn)
        .ok();

        // BR-170: only a linked, immutable Magic TDX assignment may populate
        // the current chain_name projection.
        position_chain::create_schema(conn).map_err(std::io::Error::other)?;

        // trades 表（v3 每笔买卖独立记录，与 stock_position 互补）
        diesel::sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS trades (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                code TEXT NOT NULL,
                name TEXT NOT NULL DEFAULT '',
                direction TEXT NOT NULL CHECK(direction IN ('buy', 'sell')),
                price REAL NOT NULL,
                shares INTEGER NOT NULL,
                amount REAL NOT NULL,
                reason TEXT NOT NULL DEFAULT '',
                traded_at TEXT NOT NULL,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&mut *conn)?;
        diesel::sql_query("CREATE INDEX IF NOT EXISTS ix_trades_code ON trades(code)")
            .execute(&mut *conn)?;

        diesel::sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS order_idempotency (
                business_order_id TEXT PRIMARY KEY NOT NULL,
                reserved_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&mut *conn)?;

        diesel::sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS order_audit (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                business_order_id TEXT NOT NULL,
                source TEXT NOT NULL,
                decision_basis TEXT NOT NULL,
                side TEXT NOT NULL CHECK(side IN ('buy', 'sell', 'cancel')),
                code TEXT NOT NULL,
                requested_price REAL NOT NULL,
                execution_price REAL,
                quantity INTEGER NOT NULL,
                quote_observed_at TEXT,
                outcome TEXT NOT NULL CHECK(outcome IN ('Filled', 'Rejected', 'Canceled')),
                failure_reason TEXT,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&mut *conn)?;
        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS idx_order_audit_business_id
             ON order_audit(business_order_id, created_at)",
        )
        .execute(&mut *conn)?;
        diesel::sql_query(
            "CREATE TRIGGER IF NOT EXISTS trg_order_audit_validate_insert
             BEFORE INSERT ON order_audit
             WHEN trim(NEW.business_order_id) = ''
               OR trim(NEW.source) = ''
               OR trim(NEW.decision_basis) = ''
               OR trim(NEW.code) = ''
               OR (NEW.outcome = 'Filled' AND (
                    NEW.requested_price <= 0
                    OR NEW.execution_price IS NULL
                    OR NEW.execution_price <= 0
                    OR NEW.quantity <= 0
                    OR NEW.quantity % 100 != 0
                    OR NEW.quote_observed_at IS NULL
                    OR trim(NEW.quote_observed_at) = ''
               ))
               OR (NEW.outcome = 'Rejected' AND (
                    NEW.failure_reason IS NULL OR trim(NEW.failure_reason) = ''
               ))
             BEGIN SELECT RAISE(ABORT, 'BR-086 invalid order_audit record'); END",
        )
        .execute(&mut *conn)?;
        diesel::sql_query(
            "CREATE TRIGGER IF NOT EXISTS trg_order_audit_no_update
             BEFORE UPDATE ON order_audit
             BEGIN SELECT RAISE(ABORT, 'BR-086 order_audit is immutable'); END",
        )
        .execute(&mut *conn)?;
        diesel::sql_query(
            "CREATE TRIGGER IF NOT EXISTS trg_order_audit_no_delete
             BEFORE DELETE ON order_audit
             BEGIN SELECT RAISE(ABORT, 'BR-086 order_audit retention is at least five years'); END",
        )
        .execute(&mut *conn)?;
        diesel::sql_query(
            "CREATE TABLE IF NOT EXISTS order_audit_chain (
                order_audit_id INTEGER PRIMARY KEY NOT NULL,
                previous_hash TEXT NOT NULL,
                record_hash TEXT NOT NULL UNIQUE,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY(order_audit_id) REFERENCES order_audit(id)
            )",
        )
        .execute(&mut *conn)?;
        diesel::sql_query(
            "CREATE TRIGGER IF NOT EXISTS trg_order_audit_chain_no_update
             BEFORE UPDATE ON order_audit_chain
             BEGIN SELECT RAISE(ABORT, 'BR-086 order audit hash chain is immutable'); END",
        )
        .execute(&mut *conn)?;
        diesel::sql_query(
            "CREATE TRIGGER IF NOT EXISTS trg_order_audit_chain_no_delete
             BEFORE DELETE ON order_audit_chain
             BEGIN SELECT RAISE(ABORT, 'BR-086 order audit hash chain retention is at least five years'); END",
        )
        .execute(&mut *conn)?;
        order_audit::initialize_order_audit_chain(&mut *conn)?;

        // BR-159: every unified-Gateway acquisition attempt is append-only,
        // hash-chained, and retains provider/batch evidence plus aggregate
        // acceptance counters. Initialization fails on any chain mismatch.
        data_acquisition_audit::create_schema(&mut *conn)?;
        benchmark_segments::create_schema(&mut *conn)?;
        attribution_reports::create_schema(&mut *conn)?;
        attribution_epochs::create_schema(&mut *conn)?;
        // BR-171: exact operator confirmations for >±20% adjacent daily-close
        // moves are immutable, hash-chained and validated at startup.
        daily_change_confirmation::create_schema(&mut *conn)?;
        // BR-172: NewsAI assessments plus exact reservation/sink/delivery/
        // prediction-link events are independently immutable SHA-256 chains.
        news_ai::create_schema(&mut *conn)?;

        // ledger 表（v3 每日净值快照）
        diesel::sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS ledger (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                date TEXT NOT NULL UNIQUE,
                total_value REAL NOT NULL,
                cash REAL NOT NULL DEFAULT 0,
                market_value REAL NOT NULL DEFAULT 0,
                daily_pnl REAL NOT NULL DEFAULT 0,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&mut *conn)?;

        // BR-103: real-account facts are append-only and preserve nullable P&L.
        account_snapshot::create_schema(conn)?;

        // v5 状态持久化：新闻去重
        diesel::sql_query(
            "CREATE TABLE IF NOT EXISTS news_dedup (key TEXT PRIMARY KEY, created_at TEXT NOT NULL DEFAULT (datetime('now')))",
        ).execute(&mut *conn)?;

        // v5 状态持久化：信号状态
        diesel::sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS signal_state (
                key TEXT PRIMARY KEY,
                state TEXT NOT NULL DEFAULT 'idle',
                last_alert TEXT,
                last_change TEXT,
                daily_important_count INTEGER DEFAULT 0,
                daily_info_count INTEGER DEFAULT 0
            )
            "#,
        )
        .execute(&mut *conn)?;

        // 预测追踪表（Phase 5 预测闭环）
        diesel::sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS prediction_tracker (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                pred_date TEXT NOT NULL,
                target_date TEXT NOT NULL,
                theme_name TEXT,
                stock_code TEXT,
                pred_direction TEXT NOT NULL,
                pred_score REAL,
                pred_detail TEXT,
                actual_change REAL,
                actual_result TEXT,
                hit INTEGER,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&mut *conn)?;
        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS ix_pred_date ON prediction_tracker(pred_date)",
        )
        .execute(&mut *conn)?;

        // 2026-08-07 BR-192 收尾 (T-07): P-03 候选触发选中决策持久化 —
        // counted binding 的真实证据 (见 record_candidate_trigger 文档)。
        diesel::sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS candidate_trigger_selection (
                trigger_date TEXT NOT NULL,
                code TEXT NOT NULL,
                name TEXT NOT NULL,
                basis TEXT NOT NULL,
                selected_at TEXT NOT NULL,
                PRIMARY KEY (trigger_date, code)
            )
            "#,
        )
        .execute(&mut *conn)?;

        // v10 P0.1 (G0) — prediction_tracker 加 12 列 (idempotent ALTER, 2026-07-01)
        // 设计: BR-016/017/020 落表; 12 列 = 1+1+3+3+3+1
        // 1+1 = reason / reason_secondary (主/副理由, 枚举, v10 §10.3)
        // 3   = actual_change_t1/t3/t5 (T+1/T+3/T+5 实际涨跌幅, BC-3)
        // 3   = hit_t1/t3/t5 (三窗口命中布尔, BC-3)
        // 3   = market_up_rate_t1/t3/t5 (同日同窗市场基准, BC-1, Q2=B 全市场上涨家数占比)
        // 1   = t1_special_case (停牌/涨停/跌停/正常, BC-3)
        //
        // BR-016/017/020 落表; 幂等: 列已存在时 SQLite 报 "duplicate column name"
        //
        // BUG FIX (codex B1): 之前用 `let _ = ...` 吞错, DB 损坏/权限不足时静默 fail
        // 现在区分: "duplicate column" → 静默 (幂等), 其他错误 → 返回 Err 显式报错
        for col_def in [
            "ALTER TABLE prediction_tracker ADD COLUMN reason TEXT",
            "ALTER TABLE prediction_tracker ADD COLUMN reason_secondary TEXT",
            "ALTER TABLE prediction_tracker ADD COLUMN actual_change_t1 REAL",
            "ALTER TABLE prediction_tracker ADD COLUMN actual_change_t3 REAL",
            "ALTER TABLE prediction_tracker ADD COLUMN actual_change_t5 REAL",
            "ALTER TABLE prediction_tracker ADD COLUMN hit_t1 INTEGER",
            "ALTER TABLE prediction_tracker ADD COLUMN hit_t3 INTEGER",
            "ALTER TABLE prediction_tracker ADD COLUMN hit_t5 INTEGER",
            "ALTER TABLE prediction_tracker ADD COLUMN market_up_rate_t1 REAL",
            "ALTER TABLE prediction_tracker ADD COLUMN market_up_rate_t3 REAL",
            "ALTER TABLE prediction_tracker ADD COLUMN market_up_rate_t5 REAL",
            "ALTER TABLE prediction_tracker ADD COLUMN t1_special_case TEXT",
        ] {
            match diesel::sql_query(col_def).execute(&mut *conn) {
                Ok(_) => {}
                Err(e) => {
                    let err_str = e.to_string();
                    if err_str.contains("duplicate column") {
                        // 幂等: 列已存在, 跳过 (这是 re-run 期望行为)
                    } else {
                        // 真错误: DB 损坏/权限不足/磁盘满, 显式返回 Err
                        eprintln!(
                            "[DatabaseManager::init_schema] ✗ 真错误 (col_def={}): {}",
                            col_def, e
                        );
                        return Err(Box::new(e));
                    }
                }
            }
        }

        // 概念共振表（Phase 4 动态产业链拓扑）
        diesel::sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS concept_cooccurrence (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                stock_code TEXT NOT NULL,
                concept_name TEXT NOT NULL,
                cooccur_weight REAL DEFAULT 0.0,
                evidence_level TEXT DEFAULT 'C',
                last_updated TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                UNIQUE(stock_code, concept_name)
            )
            "#,
        )
        .execute(&mut *conn)?;

        // 创建 factor_snapshot 表（修复 QUANT_ANALYST_REVIEW §1.5）
        diesel::sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS factor_snapshot (
                code TEXT NOT NULL,
                snapshot_date TEXT NOT NULL,
                pe_ttm REAL,
                pb REAL,
                roe REAL,
                market_cap REAL,
                turnover_rate REAL,
                source TEXT,
                created_at TEXT NOT NULL,
                PRIMARY KEY (code, snapshot_date)
            )
            "#,
        )
        .execute(&mut *conn)?;
        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS idx_factor_snapshot_date ON factor_snapshot(snapshot_date)",
        )
        .execute(&mut *conn)?;

        // ===== v12 PR1/PR3 表 (idempotent CREATE IF NOT EXISTS) =====
        // Bug A fix (2026-07-05): 原 run_migrations() 不读 migrations/*.sql,
        // v12 表必须在此手写 CREATE IF NOT EXISTS.

        // account_mode_log
        diesel::sql_query(
            "CREATE TABLE IF NOT EXISTS account_mode_log (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                ts              TIMESTAMP NOT NULL,
                prev_mode       TEXT NOT NULL,
                new_mode        TEXT NOT NULL,
                trigger_reason  TEXT NOT NULL,
                today_pnl_pct   REAL,
                consecutive_n   INTEGER,
                total_pos_cheng INTEGER,
                data_complete   INTEGER NOT NULL DEFAULT 1,
                pushed          INTEGER NOT NULL DEFAULT 0,
                push_attempted_at TIMESTAMP
            )",
        )
        .execute(&mut *conn)?;
        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS idx_account_mode_log_ts ON account_mode_log(ts)",
        )
        .execute(&mut *conn)
        .ok();
        diesel::sql_query("CREATE INDEX IF NOT EXISTS idx_account_mode_log_new_mode ON account_mode_log(new_mode)")
            .execute(&mut *conn).ok();

        // paper_trades (PR3-3.5)
        diesel::sql_query(
            "CREATE TABLE IF NOT EXISTS paper_trades (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                plan_id         TEXT NOT NULL,
                code            TEXT NOT NULL,
                name            TEXT NOT NULL,
                direction       TEXT NOT NULL CHECK(direction IN ('buy','sell')),
                price           REAL NOT NULL CHECK(price > 0),
                quantity        INTEGER NOT NULL CHECK(quantity > 0 AND quantity % 100 = 0),
                status          TEXT NOT NULL CHECK(status IN ('SignalTriggered','Filled','NotFilled','Invalidated')),
                fill_price      REAL CHECK(fill_price IS NULL OR fill_price > 0),
                not_fill_reason TEXT,
                virtual_reason  TEXT NOT NULL,
                account_mode    TEXT NOT NULL,
                data_mode       TEXT NOT NULL,
                ts              TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at      TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
            )",
        )
        .execute(&mut *conn)?;
        diesel::sql_query(
            "CREATE UNIQUE INDEX IF NOT EXISTS uniq_paper_trades_plan_id ON paper_trades(plan_id)",
        )
        .execute(&mut *conn)?;
        diesel::sql_query("CREATE INDEX IF NOT EXISTS idx_paper_trades_code ON paper_trades(code)")
            .execute(&mut *conn)
            .ok();
        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS idx_paper_trades_status ON paper_trades(status)",
        )
        .execute(&mut *conn)
        .ok();
        diesel::sql_query(
            "CREATE TRIGGER IF NOT EXISTS trg_paper_trades_order_safety_insert
             BEFORE INSERT ON paper_trades
             WHEN NEW.price <= 0 OR NEW.quantity <= 0 OR NEW.quantity % 100 != 0
               OR NEW.price * NEW.quantity > 1000000
               OR (NEW.fill_price IS NOT NULL AND NEW.fill_price <= 0)
             BEGIN SELECT RAISE(ABORT, 'BR-084 invalid paper trade order'); END",
        )
        .execute(&mut *conn)?;
        diesel::sql_query(
            "CREATE TRIGGER IF NOT EXISTS trg_paper_trades_order_safety_update
             BEFORE UPDATE OF price, quantity, fill_price ON paper_trades
             WHEN NEW.price <= 0 OR NEW.quantity <= 0 OR NEW.quantity % 100 != 0
               OR NEW.price * NEW.quantity > 1000000
               OR (NEW.fill_price IS NOT NULL AND NEW.fill_price <= 0)
             BEGIN SELECT RAISE(ABORT, 'BR-084 invalid paper trade order'); END",
        )
        .execute(&mut *conn)?;
        // BR-249: paper-inventory reconstruction failures occur before an
        // order attempt, so they use an independent immutable hash chain.
        // Startup refuses a missing, partial or tampered chain.
        paper_inventory_failure_audit::create_schema(&mut *conn)?;

        // execution_tracking (PR3-3.5)
        diesel::sql_query(
            "CREATE TABLE IF NOT EXISTS execution_tracking (
                id                  INTEGER PRIMARY KEY AUTOINCREMENT,
                paper_trade_id      INTEGER NOT NULL,
                plan_id             TEXT NOT NULL,
                code                TEXT NOT NULL,
                expected_price      REAL NOT NULL,
                actual_change_t1    REAL,
                actual_change_t3    REAL,
                actual_change_t5    REAL,
                mfe                 REAL,
                mae                 REAL,
                t1_special_case     TEXT,
                created_at          TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at          TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
            )",
        )
        .execute(&mut *conn)?;
        diesel::sql_query("CREATE INDEX IF NOT EXISTS idx_execution_tracking_plan_id ON execution_tracking(plan_id)")
            .execute(&mut *conn).ok();
        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS idx_execution_tracking_code ON execution_tracking(code)",
        )
        .execute(&mut *conn)
        .ok();

        // position_adjustments (PR3-3.3)
        diesel::sql_query(
            "CREATE TABLE IF NOT EXISTS position_adjustments (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                code            TEXT NOT NULL,
                delta           INTEGER NOT NULL,
                source          TEXT NOT NULL CHECK(source IN ('manual_confirm','import')),
                reason          TEXT NOT NULL DEFAULT '',
                effective_date  TEXT NOT NULL,
                applied_immediately INTEGER NOT NULL DEFAULT 0,
                operator        TEXT,
                created_at      TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
            )",
        )
        .execute(&mut *conn)?;
        diesel::sql_query("CREATE INDEX IF NOT EXISTS idx_position_adjustments_code ON position_adjustments(code)")
            .execute(&mut *conn).ok();
        diesel::sql_query("CREATE INDEX IF NOT EXISTS idx_position_adjustments_effective ON position_adjustments(effective_date)")
            .execute(&mut *conn).ok();

        // review #16: news_items 详存 schema (idempotent CREATE IF NOT EXISTS)
        diesel::sql_query(Self::NEWS_ITEMS_SCHEMA).execute(&mut *conn)?;

        Ok(())
    }

    /// review #16: 插入单条 NewsItem (INSERT OR IGNORE 走 UNIQUE 约束去重).
    ///
    /// 同 `(source, external_id)` 已存在则跳过 (UNIQUE constraint + INSERT OR IGNORE).
    /// `code` 为 None 时写空串 (schema 列允许 TEXT 无默认值, 实际查询时按 code IS NULL 或 code = '' 过滤).
    /// 时间戳写 unix seconds (i64, 落 INTEGER 列).
    pub fn insert_news_item(
        &self,
        item: &crate::data_provider::news_item::NewsItem,
    ) -> Result<(), String> {
        for (field, value) in [
            ("source", item.source.as_str()),
            ("external_id", item.external_id.as_str()),
            ("category", item.category.as_str()),
            ("title", item.title.as_str()),
            ("url", item.url.as_str()),
            ("source_name", item.source_name.as_str()),
            ("content_hash", item.content_hash.as_str()),
        ] {
            validate_required_text(field, value)?;
        }
        if let Some(code) = item.code.as_deref() {
            validate_evidence_code(code)?;
        }
        if item.fetched_at < item.published_at {
            return Err("fetched_at 不能早于 published_at".to_string());
        }
        let expected_hash =
            crate::data_provider::news_item::content_hash(&item.title, &item.summary);
        if item.content_hash != expected_hash {
            return Err(format!(
                "content_hash 与标题/摘要不一致: expected={expected_hash}, actual={}",
                item.content_hash
            ));
        }

        use diesel::sql_types::{BigInt, Nullable, Text};
        let mut conn = self.get_conn().map_err(|e| e.to_string())?;
        diesel::sql_query(
            "INSERT OR IGNORE INTO news_items (source, external_id, category, code, title, summary, url, source_name, published_at, fetched_at, content_hash) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        )
        .bind::<Text, _>(&item.source)
        .bind::<Text, _>(&item.external_id)
        .bind::<Text, _>(&item.category)
        .bind::<Nullable<Text>, _>(item.code.as_deref())
        .bind::<Text, _>(&item.title)
        .bind::<Text, _>(&item.summary)
        .bind::<Text, _>(&item.url)
        .bind::<Text, _>(&item.source_name)
        .bind::<BigInt, _>(item.published_at.timestamp())
        .bind::<BigInt, _>(item.fetched_at.timestamp())
        .bind::<Text, _>(&item.content_hash)
        .execute(&mut *conn)
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// 保存预测记录（Phase 5 预测闭环）
    ///
    /// v10 P0.2 (BR-016): 加 `reason` + `reason_secondary` 参数, 写盘口时记主/副理由
    /// 向后兼容: reason/reason_secondary 默认为 None (走 v9 旧路径)
    #[allow(
        clippy::too_many_arguments,
        reason = "stable audit persistence boundary mirrors prediction_tracker columns"
    )]
    pub fn save_prediction(
        &self,
        pred_date: &str,
        target_date: &str,
        theme_name: Option<&str>,
        stock_code: Option<&str>,
        direction: &str,
        score: f64,
        detail: Option<&str>,
        reason: Option<&str>,
        reason_secondary: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        validate_date_text("pred_date", pred_date).map_err(invalid_input)?;
        validate_date_text("target_date", target_date).map_err(invalid_input)?;
        validate_required_text("pred_direction", direction).map_err(invalid_input)?;
        if !score.is_finite() || !(0.0..=100.0).contains(&score) {
            return Err(invalid_input(format!("pred_score 超出 0..=100: {score}")).into());
        }
        if theme_name.is_none_or(|theme| theme.trim().is_empty())
            && stock_code.is_none_or(|code| code.trim().is_empty())
        {
            return Err(invalid_input("theme_name 与 stock_code 不能同时缺失".to_string()).into());
        }
        if let Some(code) = stock_code {
            validate_evidence_code(code).map_err(invalid_input)?;
        }
        if let Some(reason) = reason {
            validate_required_text("reason", reason).map_err(invalid_input)?;
        }
        if let Some(reason_secondary) = reason_secondary {
            validate_required_text("reason_secondary", reason_secondary).map_err(invalid_input)?;
        }

        use diesel::sql_types::{Double, Nullable, Text};
        let mut conn = self.get_conn()?;
        diesel::sql_query(
            "INSERT INTO prediction_tracker (pred_date, target_date, theme_name, stock_code, pred_direction, pred_score, pred_detail, reason, reason_secondary) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )
        .bind::<Text, _>(pred_date)
        .bind::<Text, _>(target_date)
        .bind::<Nullable<Text>, _>(theme_name)
        .bind::<Nullable<Text>, _>(stock_code)
        .bind::<Text, _>(direction)
        .bind::<Double, _>(score)
        .bind::<Nullable<Text>, _>(detail)
        .bind::<Nullable<Text>, _>(reason)
        .bind::<Nullable<Text>, _>(reason_secondary)
        .execute(&mut *conn)?;
        Ok(())
    }

    /// v10 P0.2 便捷重载: 不带 reason (旧调用路径, 走 v9 旧行为)
    #[allow(
        clippy::too_many_arguments,
        reason = "legacy compatibility wrapper retains its published scalar call contract"
    )]
    pub fn save_prediction_legacy(
        &self,
        pred_date: &str,
        target_date: &str,
        theme_name: Option<&str>,
        stock_code: Option<&str>,
        direction: &str,
        score: f64,
        detail: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.save_prediction(
            pred_date,
            target_date,
            theme_name,
            stock_code,
            direction,
            score,
            detail,
            None,
            None,
        )
    }

    /// 统计 prediction_tracker 总记录数 (用于 sample_threshold 动态计算)
    pub fn count_predictions(&self) -> Result<i64, Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;
        #[derive(diesel::QueryableByName)]
        struct PredCountRow {
            #[diesel(sql_type = diesel::sql_types::BigInt)]
            cnt: i64,
        }
        let result = diesel::sql_query("SELECT COUNT(*) AS cnt FROM prediction_tracker")
            .get_result::<PredCountRow>(&mut *conn)?;
        Ok(result.cnt)
    }

    /// 统计某 reason 的记录数 (用于 sample_threshold 判断)
    pub fn count_predictions_by_reason(
        &self,
        reason: &str,
    ) -> Result<i64, Box<dyn std::error::Error>> {
        validate_required_text("reason", reason).map_err(invalid_input)?;
        let mut conn = self.get_conn()?;
        #[derive(diesel::QueryableByName)]
        struct PredReasonCountRow {
            #[diesel(sql_type = diesel::sql_types::BigInt)]
            cnt: i64,
        }
        let result =
            diesel::sql_query("SELECT COUNT(*) AS cnt FROM prediction_tracker WHERE reason = ?1")
                .bind::<diesel::sql_types::Text, _>(reason)
                .get_result::<PredReasonCountRow>(&mut *conn)?;
        Ok(result.cnt)
    }

    /// 更新预测结果（次日收盘后回调）
    pub fn update_prediction_result(
        &self,
        pred_date: &str,
        stock_code: Option<&str>,
        actual_change: f64,
        hit: bool,
    ) -> Result<usize, Box<dyn std::error::Error>> {
        validate_date_text("pred_date", pred_date).map_err(invalid_input)?;
        if !actual_change.is_finite() || actual_change.abs() > 20.0 {
            return Err(invalid_input(format!(
                "actual_change 必须有限且绝对值不超过 20%: {actual_change}"
            ))
            .into());
        }
        if let Some(code) = stock_code {
            validate_evidence_code(code).map_err(invalid_input)?;
        }

        use diesel::sql_types::{Double, Integer, Text};
        let mut conn = self.get_conn()?;
        let result_text = if hit { "命中" } else { "未命中" };
        let rows = if let Some(code) = stock_code {
            diesel::sql_query(
                "UPDATE prediction_tracker SET actual_change = ?1, hit = ?2, actual_result = ?3 WHERE pred_date = ?4 AND stock_code = ?5",
            )
            .bind::<Double, _>(actual_change)
            .bind::<Integer, _>(hit as i32)
            .bind::<Text, _>(result_text)
            .bind::<Text, _>(pred_date)
            .bind::<Text, _>(code)
            .execute(&mut *conn)?
        } else {
            diesel::sql_query(
                "UPDATE prediction_tracker SET actual_change = ?1, hit = ?2, actual_result = ?3 WHERE pred_date = ?4 AND theme_name IS NOT NULL AND trim(theme_name) != ''",
            )
            .bind::<Double, _>(actual_change)
            .bind::<Integer, _>(hit as i32)
            .bind::<Text, _>(result_text)
            .bind::<Text, _>(pred_date)
            .execute(&mut *conn)?
        };
        Ok(rows)
    }

    /// 按 stock_code + pred_date 查询 prediction 记录
    ///
    /// 修复 R-1: 用于 verify_predictions 真实回填后, 测试断言 hit/actual_change。
    /// 返回最新的一条 (LIMIT 1) — 同一 (code, pred_date) 只期望一条。
    pub fn get_prediction_by_code_date(
        &self,
        stock_code: &str,
        pred_date: &str,
    ) -> Result<PredictionRow, Box<dyn std::error::Error>> {
        validate_evidence_code(stock_code).map_err(invalid_input)?;
        validate_date_text("pred_date", pred_date).map_err(invalid_input)?;
        let mut conn = self.get_conn()?;
        let row = diesel::sql_query(
            "SELECT id, pred_date, target_date, stock_code, pred_direction, pred_score, actual_change, hit, actual_result FROM prediction_tracker WHERE stock_code = ?1 AND pred_date = ?2 ORDER BY id DESC LIMIT 1",
        )
        .bind::<diesel::sql_types::Text, _>(stock_code)
        .bind::<diesel::sql_types::Text, _>(pred_date)
        .get_result::<PredictionRow>(&mut *conn)?;
        Ok(row)
    }

    /// 查某日所有未 verify 的 prediction（hit IS NULL）
    ///
    /// 修复 R-1: verify_predictions 真实拉取 stock_daily 后,
    /// 用此函数找到待回填的预测记录 (替代之前硬编码 0.0, false 的假实现)。
    pub fn get_pending_predictions(
        &self,
        pred_date: &str,
    ) -> Result<Vec<PredictionRow>, Box<dyn std::error::Error>> {
        validate_date_text("pred_date", pred_date).map_err(invalid_input)?;
        let mut conn = self.get_conn()?;
        let rows = diesel::sql_query(
            "SELECT id, pred_date, target_date, stock_code, pred_direction, pred_score, actual_change, hit, actual_result FROM prediction_tracker WHERE pred_date = ?1 AND hit IS NULL",
        )
        .bind::<diesel::sql_types::Text, _>(pred_date)
        .load::<PredictionRow>(&mut *conn)?;
        Ok(rows)
    }

    /// BR-232: 候选样本证据聚合 (SignalTracker, pred_detail='candidate-strong')。
    /// 返回 (去重样本数, 命中数); 按 (pred_date, stock_code) 去重取首行,
    /// 只统计已回填 (actual_change NOT NULL) 且 pred_date <= business_date 的行。
    pub fn candidate_promotion_samples(
        &self,
        business_date: &str,
    ) -> Result<(usize, usize), Box<dyn std::error::Error>> {
        validate_date_text("business_date", business_date).map_err(invalid_input)?;
        let mut conn = self.get_conn()?;
        #[derive(QueryableByName, Debug)]
        struct SampleCounts {
            #[diesel(sql_type = diesel::sql_types::BigInt)]
            sample_count: i64,
            #[diesel(sql_type = diesel::sql_types::BigInt)]
            hit_sum: i64,
        }
        let row = diesel::sql_query(
            "SELECT COUNT(*) AS sample_count, COALESCE(SUM(hit), 0) AS hit_sum \
             FROM prediction_tracker \
             WHERE id IN ( \
               SELECT MIN(id) FROM prediction_tracker \
               WHERE pred_detail = 'candidate-strong' AND actual_change IS NOT NULL \
                 AND pred_date <= ?1 \
               GROUP BY pred_date, stock_code \
             )",
        )
        .bind::<diesel::sql_types::Text, _>(business_date)
        .get_result::<SampleCounts>(&mut *conn)?;
        Ok((row.sample_count as usize, row.hit_sum as usize))
    }

    /// BR-192 收尾 (2026-08-07): P-03/T-07 候选触发的选中决策持久化。
    /// 原 push_candidate_triggered 恒 CANDIDATE_COUNTED_BINDING_UNAVAILABLE —
    /// 候选选择 (从 chain_daily + P5 文件组装的批次) 无 durable 生命周期
    /// 所有者, 无法构造不可变审计 binding。本表记录"哪个候选在何时被选中
    /// 及其真实选择依据" (basis = sources_label/trigger_desc), 使 counted
    /// binding 有真实持久化证据 (不伪造批次身份), origin=InternalDurable。
    /// (trigger_date, code) 主键: 同一候选当日只触发一次 (与 counted
    /// occurrence 一致)。
    pub fn record_candidate_trigger(
        &self,
        trigger_date: &str,
        code: &str,
        name: &str,
        basis: &str,
    ) -> Result<(), String> {
        validate_date_text("trigger_date", trigger_date)?;
        if code.trim().is_empty() || name.trim().is_empty() || basis.trim().is_empty() {
            return Err("candidate_trigger 行非法: code/name/basis 均不可空".to_string());
        }
        let mut conn = self.get_conn().map_err(|e| e.to_string())?;
        diesel::sql_query(
            "INSERT OR REPLACE INTO candidate_trigger_selection \
             (trigger_date, code, name, basis, selected_at) \
             VALUES (?1, ?2, ?3, ?4, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
        )
        .bind::<diesel::sql_types::Text, _>(trigger_date)
        .bind::<diesel::sql_types::Text, _>(code)
        .bind::<diesel::sql_types::Text, _>(name)
        .bind::<diesel::sql_types::Text, _>(basis)
        .execute(&mut *conn)
        .map_err(|e| format!("候选触发选中落库失败: {e}"))?;
        Ok(())
    }

    /// 读取当日候选触发选中记录 (counted binding 证据)。
    pub fn latest_candidate_trigger(
        &self,
        trigger_date: &str,
        code: &str,
    ) -> Result<Option<(String, String)>, String> {
        validate_date_text("trigger_date", trigger_date)?;
        let mut conn = self.get_conn().map_err(|e| e.to_string())?;
        #[derive(QueryableByName, Debug)]
        struct TriggerRow {
            #[diesel(sql_type = diesel::sql_types::Text)]
            name: String,
            #[diesel(sql_type = diesel::sql_types::Text)]
            basis: String,
        }
        let row = diesel::sql_query(
            "SELECT name, basis FROM candidate_trigger_selection \
             WHERE trigger_date = ?1 AND code = ?2 LIMIT 1",
        )
        .bind::<diesel::sql_types::Text, _>(trigger_date)
        .bind::<diesel::sql_types::Text, _>(code)
        .get_result::<TriggerRow>(&mut *conn)
        .optional()
        .map_err(|e| format!("候选触发选中读取失败: {e}"))?;
        Ok(row.map(|r| (r.name, r.basis)))
    }

    /// 获取近 `days` 天已验证预测的真实命中率。
    pub fn get_prediction_hit_rate(&self, days: i32) -> Result<f64, Box<dyn std::error::Error>> {
        if days <= 0 {
            return Err("命中率窗口 days 必须 > 0".into());
        }
        let mut conn = self.get_conn()?;
        #[derive(QueryableByName, Debug)]
        struct HitRate {
            #[diesel(sql_type = diesel::sql_types::BigInt)]
            sample_count: i64,
            #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
            hit_sum: Option<f64>,
        }

        let row = diesel::sql_query(
            "SELECT COUNT(*) AS sample_count, SUM(CAST(hit AS REAL)) AS hit_sum \
             FROM prediction_tracker \
             WHERE hit IS NOT NULL AND date(pred_date) >= date('now', '-' || ? || ' days')",
        )
        .bind::<diesel::sql_types::Integer, _>(days)
        .get_result::<HitRate>(&mut *conn)?;
        if row.sample_count <= 0 {
            return Err(format!("近 {days} 天没有已验证预测样本").into());
        }
        let hit_sum = row.hit_sum.ok_or("命中数聚合结果缺失")?;
        let rate = hit_sum / row.sample_count as f64;
        if !rate.is_finite() || !(0.0..=1.0).contains(&rate) {
            return Err(format!("命中率超出有效域: {rate}").into());
        }
        Ok(rate)
    }

    /// 保存主题签名用于去同质化（重复签名更新 created_at）
    pub fn upsert_topic_history_signatures(
        &self,
        signatures: &[String],
        max_rows: usize,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if signatures.is_empty() {
            return Ok(());
        }
        if signatures
            .iter()
            .any(|signature| signature.trim().is_empty())
        {
            return Err(invalid_input("topic signature 批次包含空值".to_string()).into());
        }

        let mut conn = self.get_conn()?;
        let now_ts = chrono::Local::now().timestamp();

        // 事务内批量写入，避免逐行 fsync
        conn.transaction::<_, Box<dyn std::error::Error>, _>(|conn| {
            for sig in signatures {
                diesel::sql_query(
                    "INSERT INTO topic_novelty_history(signature, created_at) VALUES (?1, ?2) ON CONFLICT(signature) DO UPDATE SET created_at=excluded.created_at",
                )
                .bind::<diesel::sql_types::Text, _>(sig)
                .bind::<diesel::sql_types::BigInt, _>(now_ts)
                .execute(conn)?;
            }
            Ok(())
        })?;

        let keep = max_rows.max(50) as i64;
        diesel::sql_query(
            "DELETE FROM topic_novelty_history WHERE signature NOT IN (SELECT signature FROM topic_novelty_history ORDER BY created_at DESC LIMIT ?1)",
        )
        .bind::<diesel::sql_types::BigInt, _>(keep)
        .execute(&mut *conn)?;

        Ok(())
    }

    /// 修复 v9.2 BR-001: 统计某只票近 N 天被 push 的次数
    pub fn count_recent_pushes(
        &self,
        stock_code: &str,
        days: i64,
    ) -> Result<i64, Box<dyn std::error::Error>> {
        validate_evidence_code(stock_code).map_err(invalid_input)?;
        if days <= 0 {
            return Err(invalid_input("count_recent_pushes days 必须 > 0".to_string()).into());
        }
        let mut conn = self.get_conn()?;
        let cutoff = (chrono::Local::now() - chrono::Duration::days(days))
            .format("%Y-%m-%d")
            .to_string();
        #[derive(serde::Serialize, serde::Deserialize, diesel::QueryableByName)]
        struct CountRow {
            #[diesel(sql_type = diesel::sql_types::BigInt)]
            cnt: i64,
        }
        let row = diesel::sql_query(
            "SELECT COUNT(*) as cnt FROM prediction_tracker WHERE stock_code = ?1 AND pred_date >= ?2",
        )
        .bind::<diesel::sql_types::Text, _>(stock_code)
        .bind::<diesel::sql_types::Text, _>(&cutoff)
        .get_result::<CountRow>(&mut *conn)?;
        Ok(row.cnt)
    }

    /// 修复 v9.2 M1 性能: 批量查询近 N 天被 push 的 stock_code 集合
    /// 一次 SQL 查所有 stock_code, 避免 discover() 内 N×M 次 sync DB round-trip
    /// 阻塞 async runtime. 返回 HashSet 含所有近 N 天内被 push 过的 stock_code.
    pub fn count_recent_pushes_batch(
        &self,
        stock_codes: &[String],
        days: i64,
    ) -> Result<std::collections::HashSet<String>, Box<dyn std::error::Error>> {
        if days <= 0 {
            return Err(
                invalid_input("count_recent_pushes_batch days 必须 > 0".to_string()).into(),
            );
        }
        if stock_codes.is_empty() {
            return Ok(std::collections::HashSet::new());
        }
        // 修复 I-5 (2026-06-29 codex review) + review #14:
        // 1. 防 SQL 注入 — 显式 if 校验 stock_code 是 ASCII alphanumeric + 下划线.
        //    原 assert! 在 release 默认被优化掉 (除非显式 panic=abort + debug-assertions),
        //    防护失效. 改为返回 Result 错误, 调用方决定如何处理.
        // 2. 用 diesel prepared statement + ? bind 走参数化, 彻底消除字符串拼接风险.
        for c in stock_codes {
            if !c
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
            {
                return Err(format!(
                    "count_recent_pushes_batch: stock_code must be alphanumeric/_/-, got {:?}",
                    c
                )
                .into());
            }
        }
        let mut conn = self.get_conn()?;
        let cutoff = (chrono::Local::now() - chrono::Duration::days(days))
            .format("%Y-%m-%d")
            .to_string();
        // 用 IN (?, ?, ...) + bind 走 prepared statement, 字符串拼接为零.
        // SQLite parameter binding 类型安全, 无 escape 风险.
        use diesel::sql_types::Text;
        let placeholders = std::iter::repeat_n("?", stock_codes.len())
            .collect::<Vec<_>>()
            .join(",");
        let raw = format!(
            "SELECT DISTINCT stock_code FROM prediction_tracker WHERE stock_code IN ({}) AND pred_date >= ?",
            placeholders
        );
        let mut q = diesel::sql_query(raw).into_boxed::<diesel::sqlite::Sqlite>();
        for c in stock_codes {
            q = q.bind::<Text, _>(c.clone());
        }
        q = q.bind::<Text, _>(cutoff);
        #[derive(serde::Serialize, serde::Deserialize, diesel::QueryableByName)]
        struct CodeRow {
            #[diesel(sql_type = diesel::sql_types::Text)]
            stock_code: String,
        }
        let rows: Vec<CodeRow> = q.load::<CodeRow>(&mut *conn)?;
        Ok(rows.into_iter().map(|r| r.stock_code).collect())
    }

    /// 读取近窗期主题签名（按最新时间倒序）
    pub fn get_recent_topic_history_signatures(
        &self,
        lookback_hours: u64,
        limit: usize,
    ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        #[derive(QueryableByName, Debug)]
        struct SigRow {
            #[diesel(sql_type = diesel::sql_types::Text)]
            signature: String,
        }

        let mut conn = self.get_conn()?;
        let since_ts = chrono::Local::now().timestamp() - (lookback_hours as i64 * 3600);
        let lim = limit.max(20) as i64;
        let rows = diesel::sql_query(
            "SELECT signature FROM topic_novelty_history WHERE created_at >= ?1 ORDER BY created_at DESC LIMIT ?2",
        )
        .bind::<diesel::sql_types::BigInt, _>(since_ts)
        .bind::<diesel::sql_types::BigInt, _>(lim)
        .load::<SigRow>(&mut *conn)?;

        Ok(rows.into_iter().map(|r| r.signature).collect())
    }
}

// 辅助数据结构

/// 股票日线记录（用于批量插入）
#[derive(Debug, Clone)]
pub struct StockDailyRecord {
    pub code: String,
    pub date: NaiveDate,
    pub open: Option<f64>,
    pub high: Option<f64>,
    pub low: Option<f64>,
    pub close: Option<f64>,
    pub volume: Option<f64>,
    pub amount: Option<f64>,
    pub pct_chg: Option<f64>,
    pub ma5: Option<f64>,
    pub ma10: Option<f64>,
    pub ma20: Option<f64>,
    pub volume_ratio: Option<f64>,
    pub data_source: Option<String>,
}

/// 分析上下文
#[derive(Debug, Clone)]
pub struct AnalysisContext {
    pub code: String,
    pub date: NaiveDate,
    pub today: HashMap<String, serde_json::Value>,
    pub yesterday: Option<HashMap<String, serde_json::Value>>,
    pub volume_change_ratio: Option<f64>,
    pub price_change_ratio: Option<f64>,
    pub ma_status: MaStatus,
}

// ============================================================================
// 便捷函数
// ============================================================================

/// 获取数据库管理器实例的快捷方式
pub fn get_db() -> &'static DatabaseManager {
    DatabaseManager::get()
}

// ============================================================================
// P0-3: FactorIC 因子分析 — 已平仓交易 + 评分 JOIN
// ============================================================================

/// 预测记录查询返回行 (公开类型, monitor 模块 + 测试使用)
///
/// 修复 R-1: verify_predictions 真实拉取 stock_daily 之后回填,
/// 测试和 verify 函数本身都需要读这条记录。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, diesel::QueryableByName)]
pub struct PredictionRow {
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub id: i32,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub pred_date: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub target_date: String,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    pub stock_code: Option<String>,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub pred_direction: String,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    pub pred_score: Option<f64>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    pub actual_change: Option<f64>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Integer>)]
    pub hit: Option<i32>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    pub actual_result: Option<String>,
}

/// 因子 IC 分析的查询结果行 (公开类型, review 模块使用)
#[derive(Debug, Clone)]
pub struct FactorIcRow {
    pub buy_price: f64,
    pub sell_price: f64,
    pub sentiment_score: Option<i32>,
    pub score_breakdown_json: Option<String>,
}

/// Diesel 返回的内部行
#[derive(QueryableByName, Debug)]
struct FactorIcRowDb {
    #[diesel(sql_type = diesel::sql_types::Double)]
    buy_price: f64,
    #[diesel(sql_type = diesel::sql_types::Double)]
    sell_price: f64,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Integer>)]
    sentiment_score: Option<i32>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    score_breakdown_json: Option<String>,
}

/// PRAGMA table_info 返回的列名行（只取 name 列）
#[derive(QueryableByName, Debug)]
struct ColumnNameRow {
    #[diesel(sql_type = diesel::sql_types::Text)]
    name: String,
}

/// 判断表是否存在 (PRAGMA table_info 返回空 = 不存在)
/// 修复 (2026-07-05 MVP0-A): 用于 add_column_if_missing 跳过未建表, 避免 init 失败
fn table_exists(
    conn: &mut SqliteConnection,
    table: &str,
) -> Result<bool, Box<dyn std::error::Error>> {
    use diesel::RunQueryDsl;
    let pragma_sql = format!("PRAGMA table_info({})", table);
    let cols: Vec<ColumnNameRow> = diesel::sql_query(pragma_sql).load(conn)?;
    Ok(!cols.is_empty())
}

/// 判断表中是否存在指定列（用于增量 schema 升级）
fn column_exists(
    conn: &mut SqliteConnection,
    table: &str,
    column: &str,
) -> Result<bool, Box<dyn std::error::Error>> {
    use diesel::RunQueryDsl;
    let pragma_sql = format!("PRAGMA table_info({})", table);
    let cols: Vec<ColumnNameRow> = diesel::sql_query(&pragma_sql).load(conn)?;
    Ok(cols.iter().any(|c| c.name.eq_ignore_ascii_case(column)))
}

impl DatabaseManager {
    /// 获取已平仓交易的因子分析数据。
    /// 最多 500 条, 用于 `--review` 路径的因子 IC 诊断。
    pub fn get_factor_ic_data(&self) -> Result<Vec<FactorIcRow>, Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;
        let rows = diesel::sql_query(
            "SELECT sp.buy_price, sp.sell_price, ar.sentiment_score, ar.score_breakdown_json
             FROM stock_position sp
             LEFT JOIN analysis_result ar ON sp.code = ar.code AND sp.buy_date = ar.date
             WHERE sp.status = 'closed'
               AND sp.buy_price > 0
               AND sp.sell_price IS NOT NULL
             ORDER BY sp.buy_date DESC
             LIMIT 500",
        )
        .load::<FactorIcRowDb>(&mut conn)?;

        Ok(rows
            .into_iter()
            .map(|r| FactorIcRow {
                buy_price: r.buy_price,
                sell_price: r.sell_price,
                sentiment_score: r.sentiment_score,
                score_breakdown_json: r.score_breakdown_json,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::StockPosition;
    use chrono::NaiveDate;

    #[derive(QueryableByName)]
    struct BusyTimeoutValue {
        #[diesel(sql_type = diesel::sql_types::Integer)]
        timeout: i32,
    }

    #[derive(QueryableByName)]
    struct ForeignKeysValue {
        #[diesel(sql_type = diesel::sql_types::Integer)]
        foreign_keys: i32,
    }

    #[derive(QueryableByName)]
    struct SynchronousValue {
        #[diesel(sql_type = diesel::sql_types::Integer)]
        synchronous: i32,
    }

    #[derive(QueryableByName)]
    struct WalAutocheckpointValue {
        #[diesel(sql_type = diesel::sql_types::Integer)]
        wal_autocheckpoint: i32,
    }

    // v14.1 review fix: RAII test DB guard, panic 时 Drop 兜底清理
    struct TestDbGuard(&'static str);
    impl Drop for TestDbGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(self.0);
        }
    }

    struct TemporarySqliteDatabase(PathBuf);

    impl TemporarySqliteDatabase {
        fn new(prefix: &str) -> Self {
            Self(std::env::temp_dir().join(format!("{}.db", unique_test_label(prefix))))
        }

        fn database_url(&self) -> String {
            self.0.to_string_lossy().into_owned()
        }
    }

    impl Drop for TemporarySqliteDatabase {
        fn drop(&mut self) {
            for suffix in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{suffix}", self.0.to_string_lossy()));
            }
        }
    }

    struct SharedSelectionAttributionDatabase {
        manager: DatabaseManager,
        _database: TemporarySqliteDatabase,
    }

    impl SharedSelectionAttributionDatabase {
        fn new(prefix: &str) -> Self {
            let database = TemporarySqliteDatabase::new(prefix);
            let database_url = database.database_url();
            let mut bootstrap = SqliteConnection::establish(&database_url)
                .expect("create shared selection/attribution SQLite database");
            let journal_mode = diesel::sql_query("PRAGMA journal_mode = WAL")
                .get_result::<JournalModeRow>(&mut bootstrap)
                .expect("enable WAL for shared selection/attribution database")
                .journal_mode;
            assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
            configure_sqlite_connection(&mut bootstrap)
                .expect("configure shared selection/attribution database");
            drop(bootstrap);

            let (attribution_pool, source) = build_attested_sqlite_pool_with_size(&database.0, 1)
                .expect("build no-self-heal attribution pool");
            let selection_manager = SqliteConnectionManager {
                source: SqliteConnectionSource::Descriptor(Arc::clone(&source)),
            };
            let selection_pool = build_sqlite_pool_from_manager(selection_manager, 1)
                .expect("build checkout-validating selection pool");
            Self {
                manager: DatabaseManager {
                    pool: selection_pool,
                    attribution_pool: Some(attribution_pool),
                    attribution_connection_source: Some(Arc::clone(&source)),
                    readonly_attribution_snapshot: None,
                    selection_connection_source: Some(source),
                    selection_schema_authority: None,
                },
                _database: database,
            }
        }
    }

    // OnceCell 单例全局共享，测试共用同一路径避免竞态
    static TEST_DB: &str = "./test_data/test.db";

    fn init_db_for_test() {
        std::fs::create_dir_all("./test_data").ok();
        let _ = DatabaseManager::init(Some(PathBuf::from(TEST_DB)));
    }

    #[test]
    fn test_database_init() {
        init_db_for_test();
        let db = DatabaseManager::get();
        assert!(db.get_conn().is_ok());
    }

    #[test]
    fn operational_pool_is_isolated_from_optional_attribution_attestation() {
        #[derive(QueryableByName)]
        struct CountRow {
            #[diesel(sql_type = diesel::sql_types::BigInt)]
            count: i64,
        }

        init_db_for_test();
        let database = DatabaseManager::get();
        let mut operational = database
            .get_conn()
            .expect("operational database remains available");
        let count = diesel::sql_query(format!(
            "SELECT COUNT(*) AS count FROM sqlite_temp_master WHERE type='table' AND name='{}'",
            DESCRIPTOR_ATTESTATION_TEMP_TABLE
        ))
        .get_result::<CountRow>(&mut operational)
        .expect("inspect operational TEMP schema")
        .count;
        assert_eq!(count, 0, "operational checkout must remain legacy");

        if let Ok(mut attribution) = database.attribution_checkout() {
            let count = diesel::sql_query(format!(
                "SELECT COUNT(*) AS count FROM sqlite_temp_master WHERE type='table' AND name='{}'",
                DESCRIPTOR_ATTESTATION_TEMP_TABLE
            ))
            .get_result::<CountRow>(attribution.connection_for_test())
            .expect("inspect attribution TEMP schema")
            .count;
            assert_eq!(count, 1, "attribution checkout is separately attested");
        }
    }

    #[test]
    fn detached_readonly_snapshot_normalizes_only_valid_sqlite_journal_versions() {
        let database = TemporarySqliteDatabase::new("detached_snapshot_header");
        let database_url = database.database_url();
        let mut bootstrap = SqliteConnection::establish(&database_url).unwrap();
        let journal_mode = diesel::sql_query("PRAGMA journal_mode = WAL")
            .get_result::<JournalModeRow>(&mut bootstrap)
            .unwrap()
            .journal_mode;
        assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
        drop(bootstrap);
        let (_pool, source) = build_attested_sqlite_pool_with_size(&database.0, 1).unwrap();

        let mut wal_image = vec![0xa5; 100];
        wal_image[..16].copy_from_slice(b"SQLite format 3\0");
        wal_image[18] = 2;
        wal_image[19] = 2;
        let normalized = detached_readonly_snapshot_bytes(&source, &wal_image).unwrap();
        assert_eq!(&normalized[..18], &wal_image[..18]);
        assert_eq!(normalized[18..20], [1, 1]);
        assert_eq!(&normalized[20..], &wal_image[20..]);

        let mut rollback_image = wal_image.clone();
        rollback_image[18] = 1;
        rollback_image[19] = 1;
        assert_eq!(
            detached_readonly_snapshot_bytes(&source, &rollback_image).unwrap(),
            rollback_image
        );

        for (label, invalid) in [("short", vec![0_u8; 99]), ("bad magic", vec![0_u8; 100])] {
            let error = detached_readonly_snapshot_bytes(&source, &invalid)
                .expect_err("invalid SQLite header must fail closed");
            assert!(
                matches!(
                    error,
                    DatabaseAuthorityError::DescriptorIntegrityFailed { .. }
                ),
                "TEST_CODE {label} image returned {error:?}"
            );
        }

        let mut mixed = wal_image;
        mixed[19] = 1;
        let error = detached_readonly_snapshot_bytes(&source, &mixed)
            .expect_err("mixed SQLite journal versions must fail closed");
        assert!(matches!(
            error,
            DatabaseAuthorityError::DescriptorIntegrityFailed { .. }
        ));
    }

    #[test]
    fn attribution_latch_blocks_an_already_checked_out_selection_connection() {
        let database = SharedSelectionAttributionDatabase::new(
            "selection_checkout_observes_attribution_latch",
        );
        let mut selection = database.manager.get_conn().unwrap();
        database
            .manager
            .selection_connection_bound_identity(&mut selection)
            .expect("selection checkout starts attested");
        let mut attribution = database.manager.attribution_checkout().unwrap();
        diesel::sql_query(format!(
            "DROP TRIGGER {DESCRIPTOR_ATTESTATION_NO_UPDATE_TRIGGER}"
        ))
        .execute(attribution.connection_for_test())
        .expect("remove attribution registration protection");
        let first = attribution
            .authority()
            .expect_err("attribution drift must latch integrity");

        let selection_error = database
            .manager
            .selection_connection_bound_identity(&mut selection)
            .expect_err("already checked-out selection must observe the shared latch");
        assert!(matches!(
            selection_error,
            DatabaseAuthorityError::DescriptorIntegrityFailed { .. }
        ));
        assert_eq!(selection_error.to_string(), first.to_string());
    }

    #[test]
    fn selection_integrity_failure_latches_for_later_authority_operations() {
        let database =
            SharedSelectionAttributionDatabase::new("selection_checkout_sets_shared_latch");
        let source = Arc::clone(
            database
                .manager
                .selection_connection_source
                .as_ref()
                .expect("selection source"),
        );
        let mut selection = database.manager.get_conn().unwrap();
        diesel::sql_query(format!(
            "DROP TRIGGER {DESCRIPTOR_ATTESTATION_NO_DELETE_TRIGGER}"
        ))
        .execute(&mut selection)
        .expect("remove selection registration protection");
        let first = database
            .manager
            .selection_connection_bound_identity(&mut selection)
            .expect_err("selection-time registration drift must fail integrity");
        assert!(matches!(
            first,
            DatabaseAuthorityError::DescriptorIntegrityFailed { .. }
        ));

        let attribution_error = match database.manager.attribution_checkout() {
            Ok(_) => panic!("later attribution checkout must observe selection latch"),
            Err(error) => error,
        };
        assert_eq!(attribution_error.to_string(), first.to_string());
        let later_selection = database
            .manager
            .selection_connection_bound_identity(&mut selection)
            .expect_err("later selection validation must preserve first failure");
        assert_eq!(later_selection.to_string(), first.to_string());

        let connector = SqliteConnectionManager {
            source: SqliteConnectionSource::Descriptor(source),
        };
        assert!(
            connector.connect().is_err(),
            "later descriptor connect must observe selection latch"
        );
    }

    #[test]
    fn br126_database_init_creates_complete_pushed_stocks_contract() {
        #[derive(QueryableByName)]
        struct TableInfoRow {
            #[diesel(sql_type = diesel::sql_types::Text)]
            name: String,
        }
        #[derive(QueryableByName)]
        struct IndexRow {
            #[diesel(sql_type = diesel::sql_types::Text)]
            name: String,
        }

        init_db_for_test();
        let mut conn = DatabaseManager::get()
            .get_conn()
            .expect("test database connection");
        let columns = diesel::sql_query("PRAGMA table_info(pushed_stocks)")
            .load::<TableInfoRow>(&mut conn)
            .expect("read pushed_stocks columns")
            .into_iter()
            .map(|row| row.name)
            .collect::<Vec<_>>();
        assert_eq!(
            columns,
            [
                "id",
                "push_time",
                "push_kind",
                "code",
                "name",
                "push_price",
                "metric_json",
                "source",
                "consumed_at",
                "consumed_by",
                "outcome",
                "created_at",
            ]
        );
        let mut indexes = diesel::sql_query("PRAGMA index_list(pushed_stocks)")
            .load::<IndexRow>(&mut conn)
            .expect("read pushed_stocks indexes")
            .into_iter()
            .map(|row| row.name)
            .collect::<Vec<_>>();
        indexes.sort();
        assert_eq!(
            indexes,
            [
                "idx_pushed_stocks_code",
                "idx_pushed_stocks_time",
                "idx_pushed_stocks_uncon",
            ]
        );
    }

    #[test]
    fn checked_out_connections_have_required_sqlite_pragmas() {
        init_db_for_test();
        let mut conn = DatabaseManager::get()
            .get_conn()
            .expect("configured SQLite connection");

        let busy_timeout = diesel::sql_query("PRAGMA busy_timeout")
            .get_result::<BusyTimeoutValue>(&mut conn)
            .expect("read configured busy_timeout")
            .timeout;
        let foreign_keys = diesel::sql_query("PRAGMA foreign_keys")
            .get_result::<ForeignKeysValue>(&mut conn)
            .expect("read configured foreign_keys")
            .foreign_keys;
        let synchronous = diesel::sql_query("PRAGMA synchronous")
            .get_result::<SynchronousValue>(&mut conn)
            .expect("read configured synchronous")
            .synchronous;
        let wal_autocheckpoint = diesel::sql_query("PRAGMA wal_autocheckpoint")
            .get_result::<WalAutocheckpointValue>(&mut conn)
            .expect("read configured wal_autocheckpoint")
            .wal_autocheckpoint;

        assert_eq!(busy_timeout, 5000);
        assert_eq!(foreign_keys, 1);
        assert_eq!(synchronous, 2);
        assert_eq!(wal_autocheckpoint, 1000);
    }

    #[test]
    fn newly_created_and_rebuilt_pool_connections_have_required_sqlite_pragmas() {
        let database = TemporarySqliteDatabase::new("sqlite_pool_rebuild");
        let database_url = database.database_url();
        let mut bootstrap =
            SqliteConnection::establish(&database_url).expect("create temporary SQLite database");
        let journal_mode = diesel::sql_query("PRAGMA journal_mode = WAL")
            .get_result::<JournalModeRow>(&mut bootstrap)
            .expect("enable WAL for temporary SQLite database")
            .journal_mode;
        assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
        drop(bootstrap);

        let pool =
            build_sqlite_pool_with_size(database_url, 1).expect("build one-connection SQLite pool");
        let mut initial = pool.get().expect("get newly created pool connection");
        assert_eq!(
            read_sqlite_connection_configuration(&mut initial)
                .expect("read newly created connection configuration"),
            REQUIRED_SQLITE_CONFIGURATION
        );
        drop(initial);

        let panic_pool = pool.clone();
        let panic_result = std::thread::spawn(move || {
            let mut conn = panic_pool.get().expect("get connection to discard");
            diesel::sql_query("PRAGMA foreign_keys = OFF")
                .execute(&mut conn)
                .expect("alter foreign_keys before discard");
            diesel::sql_query("PRAGMA synchronous = NORMAL")
                .execute(&mut conn)
                .expect("alter synchronous before discard");
            panic!("discard pooled connection");
        })
        .join();
        assert!(
            panic_result.is_err(),
            "test thread must discard its connection"
        );

        let mut rebuilt = pool.get().expect("get replacement pool connection");
        assert_eq!(
            read_sqlite_connection_configuration(&mut rebuilt)
                .expect("read rebuilt connection configuration"),
            REQUIRED_SQLITE_CONFIGURATION
        );
    }

    #[derive(QueryableByName)]
    struct MarkerValue {
        #[diesel(sql_type = diesel::sql_types::Text)]
        value: String,
    }

    #[test]
    fn descriptor_manager_supports_wal_reopen_multi_connection_and_namespace_swap() {
        let namespace = std::env::temp_dir().join(unique_test_label("descriptor_pool_namespace"));
        let moved_namespace = namespace.with_extension("owner-pinned");
        std::fs::create_dir(&namespace).expect("create owner namespace");
        let database_path = namespace.join("stock_analysis.db");
        let database_url = database_path.to_string_lossy().into_owned();
        let mut bootstrap =
            SqliteConnection::establish(&database_url).expect("create empty owner SQLite database");
        let journal_mode = diesel::sql_query("PRAGMA journal_mode = WAL")
            .get_result::<JournalModeRow>(&mut bootstrap)
            .expect("enable WAL before descriptor attestation")
            .journal_mode;
        assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
        drop(bootstrap);

        let parent_file = File::open(&namespace).expect("pin owner database parent");
        let owner_file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&database_path)
            .expect("pin owner database descriptor");
        let expected = SqliteFileIdentity::from_metadata(
            &owner_file.metadata().expect("fstat owner descriptor"),
        );
        let root_file = parent_file
            .try_clone()
            .expect("clone owner root descriptor");
        let manager = SqliteConnectionManager::descriptor(
            PinnedSqliteDatabase::from_test_descriptors(
                root_file,
                parent_file,
                OsString::from("stock_analysis.db"),
                PathBuf::from("stock_analysis.db"),
                owner_file,
            )
            .expect("bind test owner descriptors"),
        )
        .expect("Linux has a descriptor-relative SQLite sidecar route");

        std::fs::rename(&namespace, &moved_namespace).expect("move owner namespace away");
        std::fs::create_dir(&namespace).expect("create replacement namespace");
        let replacement_url = namespace
            .join("stock_analysis.db")
            .to_string_lossy()
            .into_owned();
        let mut replacement = SqliteConnection::establish(&replacement_url)
            .expect("create replacement SQLite database");
        diesel::sql_query("CREATE TABLE marker (value TEXT NOT NULL)")
            .execute(&mut replacement)
            .expect("create replacement marker");
        diesel::sql_query("INSERT INTO marker(value) VALUES ('replacement')")
            .execute(&mut replacement)
            .expect("insert replacement marker");
        drop(replacement);

        let pool = build_sqlite_pool_from_manager(manager.clone(), 2)
            .expect("build descriptor-anchored pool after path swap");
        let mut first = pool.get().expect("get first descriptor-bound connection");
        let journal_mode = diesel::sql_query("PRAGMA journal_mode = WAL")
            .get_result::<JournalModeRow>(&mut first)
            .expect("enable WAL through descriptor-bound route")
            .journal_mode;
        assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
        diesel::sql_query("CREATE TABLE marker (value TEXT NOT NULL)")
            .execute(&mut first)
            .expect("create owner marker through descriptor-bound route");
        diesel::sql_query("INSERT INTO marker(value) VALUES ('owner')")
            .execute(&mut first)
            .expect("insert owner marker through descriptor-bound route");

        let mut second = pool
            .get()
            .expect("get simultaneous second descriptor-bound connection");
        let marker = diesel::sql_query("SELECT value FROM marker")
            .get_result::<MarkerValue>(&mut second)
            .expect("read owner marker from second connection");
        assert_eq!(marker.value, "owner");
        diesel::sql_query("INSERT INTO marker(value) VALUES ('owner-second')")
            .execute(&mut second)
            .expect("write owner marker from second connection");
        assert!(moved_namespace.join("stock_analysis.db-wal").exists());
        assert!(moved_namespace.join("stock_analysis.db-shm").exists());

        manager
            .is_valid(&mut *first)
            .expect("checkout revalidates registered descriptor proof");
        let source = manager
            .descriptor_source()
            .expect("descriptor manager retains proof registry");
        let first_token =
            connection_attestation_token(&mut first).expect("read first connection token");
        {
            let proofs = source
                .connection_proofs
                .lock()
                .expect("lock descriptor proof registry");
            let proof = proofs
                .get(&first_token)
                .expect("first connection retains registered fd proof");
            proof
                .handles
                .validate(&proof.expected_objects)
                .expect("main/WAL/SHM fds remain attested");
            let attested_main = proof.expected_objects.identity(SqliteObjectRole::Main);
            assert_eq!(attested_main.device(), expected.device);
            assert_eq!(attested_main.inode(), expected.inode);
            assert_ne!(
                proof.handles.main().descriptor(),
                proof.handles.wal().descriptor()
            );
            assert_ne!(
                proof.handles.main().descriptor(),
                proof.handles.shm().descriptor()
            );
        }

        drop(second);
        drop(first);
        let mut reopened_connection = pool.get().expect("recheckout pooled connection");
        let count = diesel::sql_query("SELECT value FROM marker ORDER BY value")
            .load::<MarkerValue>(&mut reopened_connection)
            .expect("read owner rows after pooled connection reopen");
        assert_eq!(
            count.into_iter().map(|row| row.value).collect::<Vec<_>>(),
            vec!["owner".to_owned(), "owner-second".to_owned()]
        );
        drop(reopened_connection);
        drop(pool);
        drop(manager);

        let mut replacement =
            SqliteConnection::establish(&replacement_url).expect("reopen replacement database");
        let replacement_marker = diesel::sql_query("SELECT value FROM marker")
            .get_result::<MarkerValue>(&mut replacement)
            .expect("replacement database remains readable");
        assert_eq!(replacement_marker.value, "replacement");
        drop(replacement);

        std::fs::remove_dir_all(&namespace).expect("remove replacement namespace");
        std::fs::remove_dir_all(&moved_namespace).expect("remove owner namespace");
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    #[test]
    fn descriptor_manager_fails_closed_without_descriptor_relative_wal_vfs() {
        let namespace = std::env::temp_dir().join(unique_test_label("descriptor_pool_fail_closed"));
        std::fs::create_dir(&namespace).expect("create isolated namespace");
        let database_path = namespace.join("stock_analysis.db");
        let database_url = database_path.to_string_lossy().into_owned();
        drop(SqliteConnection::establish(&database_url).expect("create isolated SQLite database"));
        let root_file = File::open(&namespace).expect("pin owner root");
        let parent_file = root_file.try_clone().expect("pin owner database parent");
        let database_file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&database_path)
            .expect("pin owner database");

        let error = SqliteConnectionManager::descriptor(
            PinnedSqliteDatabase::from_test_descriptors(
                root_file,
                parent_file,
                OsString::from("stock_analysis.db"),
                PathBuf::from("stock_analysis.db"),
                database_file,
            )
            .expect("bind owner descriptors"),
        )
        .expect_err("unproven descriptor-relative WAL routing must fail closed");
        assert!(matches!(
            error,
            DatabaseAuthorityError::DescriptorAttestationUnavailable { .. }
        ));
        assert!(!namespace.join("stock_analysis.db-wal").exists());
        assert!(!namespace.join("stock_analysis.db-shm").exists());
        std::fs::remove_dir_all(&namespace).expect("remove isolated namespace");
    }

    #[test]
    fn sqlite_customizer_propagates_configuration_failure() {
        let mut conn =
            SqliteConnection::establish(":memory:").expect("establish in-memory SQLite connection");
        diesel::sql_query("BEGIN IMMEDIATE")
            .execute(&mut conn)
            .expect("open transaction that forbids synchronous PRAGMA changes");

        let error = SqliteConnectionCustomizer
            .on_acquire(&mut conn)
            .expect_err("configuration failure must be propagated");
        let message = error.to_string();
        assert!(
            message.contains("SQLite PRAGMA synchronous=FULL failed"),
            "unexpected customizer error: {message}"
        );
    }

    fn unique_test_label(prefix: &str) -> String {
        format!(
            "{prefix}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )
    }

    #[test]
    fn br129_news_items_preserve_nullable_identity_and_verified_hash() {
        use crate::data_provider::news_item::{content_hash, NewsItem};
        use chrono::{Duration, Utc};

        #[derive(QueryableByName)]
        struct StoredNews {
            #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
            code: Option<String>,
            #[diesel(sql_type = diesel::sql_types::BigInt)]
            count: i64,
        }

        init_db_for_test();
        let db = DatabaseManager::get();
        let suffix = unique_test_label("NEWS");
        let source = format!("TEST_SOURCE_{suffix}");
        let external_id = format!("TEST_EXTERNAL_{suffix}");
        let code = format!("TEST_CODE_{suffix}");
        let title = "测试新闻标题".to_string();
        let summary = "测试新闻摘要".to_string();
        let fetched_at = Utc::now();
        let item = NewsItem {
            source: source.clone(),
            external_id: external_id.clone(),
            category: "测试分类".to_string(),
            code: Some(code.clone()),
            title: title.clone(),
            summary: summary.clone(),
            url: format!("https://example.invalid/{suffix}"),
            source_name: "测试来源".to_string(),
            published_at: fetched_at - Duration::seconds(1),
            fetched_at,
            content_hash: content_hash(&title, &summary),
        };

        db.insert_news_item(&item).unwrap();
        db.insert_news_item(&item).unwrap();
        let mut conn = db.get_conn().unwrap();
        let stored = diesel::sql_query(
            "SELECT code, COUNT(*) AS count FROM news_items WHERE source = ?1 AND external_id = ?2",
        )
        .bind::<diesel::sql_types::Text, _>(&source)
        .bind::<diesel::sql_types::Text, _>(&external_id)
        .get_result::<StoredNews>(&mut conn)
        .unwrap();
        assert_eq!(stored.code.as_deref(), Some(code.as_str()));
        assert_eq!(
            stored.count, 1,
            "source/external_id duplicate is idempotent"
        );

        let mut without_code = item.clone();
        without_code.external_id = format!("{external_id}_NULL");
        without_code.code = None;
        db.insert_news_item(&without_code).unwrap();
        let stored_null = diesel::sql_query(
            "SELECT code, COUNT(*) AS count FROM news_items WHERE source = ?1 AND external_id = ?2",
        )
        .bind::<diesel::sql_types::Text, _>(&source)
        .bind::<diesel::sql_types::Text, _>(&without_code.external_id)
        .get_result::<StoredNews>(&mut conn)
        .unwrap();
        assert_eq!(stored_null.code, None, "missing code must stay SQL NULL");
        assert_eq!(stored_null.count, 1);

        let mut bad_hash = item.clone();
        bad_hash.external_id = format!("{external_id}_BAD_HASH");
        bad_hash.content_hash = "0".repeat(64);
        assert!(db.insert_news_item(&bad_hash).is_err());
        let mut bad_time = item.clone();
        bad_time.external_id = format!("{external_id}_BAD_TIME");
        bad_time.fetched_at = bad_time.published_at - Duration::seconds(1);
        assert!(db.insert_news_item(&bad_time).is_err());
        let mut bad_identity = item.clone();
        bad_identity.external_id.clear();
        assert!(db.insert_news_item(&bad_identity).is_err());

        diesel::sql_query("DELETE FROM news_items WHERE source = ?1")
            .bind::<diesel::sql_types::Text, _>(&source)
            .execute(&mut conn)
            .unwrap();
    }

    #[test]
    fn br129_prediction_round_trip_is_bound_validated_and_traceable() {
        #[derive(QueryableByName)]
        struct ThemeOutcome {
            #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
            actual_change: Option<f64>,
            #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Integer>)]
            hit: Option<i32>,
        }

        init_db_for_test();
        let db = DatabaseManager::get();
        let suffix = unique_test_label("PREDICTION");
        let code = format!("TEST_CODE_{suffix}");
        let reason = format!("TEST_REASON_O'CLOCK_{suffix}");
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let target = (chrono::Local::now() + chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string();

        db.save_prediction(
            &today,
            &target,
            Some("测试主题"),
            Some(&code),
            "看多",
            75.0,
            Some("完整预测证据"),
            Some(&reason),
            None,
        )
        .unwrap();
        assert_eq!(db.count_predictions_by_reason(&reason).unwrap(), 1);
        assert!(db.count_predictions().unwrap() >= 1);
        assert!(db
            .get_pending_predictions(&today)
            .unwrap()
            .iter()
            .any(|row| row.stock_code.as_deref() == Some(code.as_str())));
        assert_eq!(db.count_recent_pushes(&code, 1).unwrap(), 1);
        assert!(db
            .count_recent_pushes_batch(std::slice::from_ref(&code), 1)
            .unwrap()
            .contains(&code));

        assert_eq!(
            db.update_prediction_result(&today, Some(&code), 1.25, true)
                .unwrap(),
            1
        );
        let stored = db.get_prediction_by_code_date(&code, &today).unwrap();
        assert_eq!(stored.actual_change, Some(1.25));
        assert_eq!(stored.hit, Some(1));
        assert_eq!(stored.actual_result.as_deref(), Some("命中"));
        assert!((0.0..=1.0).contains(&db.get_prediction_hit_rate(1).unwrap()));
        assert_eq!(
            db.update_prediction_result(&today, Some("TEST_CODE_MISSING"), 0.5, false)
                .unwrap(),
            0
        );

        let theme = format!("TEST_THEME_{suffix}");
        db.save_prediction(
            "1999-01-04",
            "1999-01-05",
            Some(&theme),
            None,
            "看空",
            60.0,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            db.update_prediction_result("1999-01-04", None, -0.75, false)
                .unwrap(),
            1
        );
        let mut conn = db.get_conn().unwrap();
        let theme_outcome = diesel::sql_query(
            "SELECT actual_change, hit FROM prediction_tracker WHERE pred_date = '1999-01-04' AND theme_name = ?1",
        )
        .bind::<diesel::sql_types::Text, _>(&theme)
        .get_result::<ThemeOutcome>(&mut conn)
        .unwrap();
        assert_eq!(theme_outcome.actual_change, Some(-0.75));
        assert_eq!(theme_outcome.hit, Some(0));

        assert!(db
            .save_prediction(
                "bad-date",
                &target,
                Some("测试"),
                Some(&code),
                "看多",
                75.0,
                None,
                None,
                None,
            )
            .is_err());
        assert!(db
            .save_prediction(&today, &target, None, None, "看多", 75.0, None, None, None,)
            .is_err());
        assert!(db
            .save_prediction(
                &today,
                &target,
                Some("测试"),
                Some(&code),
                "看多",
                f64::NAN,
                None,
                None,
                None,
            )
            .is_err());
        assert!(db
            .update_prediction_result(&today, Some(&code), 20.01, true)
            .is_err());
        assert!(db.get_pending_predictions("x' OR 1=1 --").is_err());
        assert!(db.count_recent_pushes(&code, 0).is_err());
        assert!(db.count_predictions_by_reason(" ").is_err());
        assert!(db.get_prediction_hit_rate(0).is_err());
        assert!(db
            .save_prediction(
                &today,
                &target,
                Some("测试"),
                Some("TEST_CODE_BAD'"),
                "看多",
                75.0,
                None,
                None,
                Some(" "),
            )
            .is_err());
        assert!(db
            .update_prediction_result(&today, Some("TEST_CODE_BAD'"), 1.0, true)
            .is_err());
        assert!(db
            .get_prediction_by_code_date("TEST_CODE_BAD'", &today)
            .is_err());
        assert!(db.count_recent_pushes("TEST_CODE_BAD'", 1).is_err());
        assert!(db
            .count_recent_pushes_batch(std::slice::from_ref(&code), 0)
            .is_err());
        assert!(db.count_recent_pushes_batch(&[], 1).unwrap().is_empty());
        assert!(db
            .count_recent_pushes_batch(&["TEST_CODE_BAD'".to_string()], 1)
            .is_err());

        diesel::sql_query(
            "DELETE FROM prediction_tracker WHERE stock_code = ?1 OR theme_name = ?2",
        )
        .bind::<diesel::sql_types::Text, _>(&code)
        .bind::<diesel::sql_types::Text, _>(&theme)
        .execute(&mut conn)
        .unwrap();
    }

    #[test]
    fn br129_topic_history_rejects_partial_bad_batches_and_is_idempotent() {
        #[derive(QueryableByName)]
        struct Count {
            #[diesel(sql_type = diesel::sql_types::BigInt)]
            count: i64,
        }

        init_db_for_test();
        let db = DatabaseManager::get();
        let suffix = unique_test_label("TOPIC");
        let first = format!("TEST_TOPIC_FIRST_{suffix}");
        let second = format!("TEST_TOPIC_SECOND_{suffix}");
        db.upsert_topic_history_signatures(&[], 0)
            .expect("empty complete batch is a no-op");
        db.upsert_topic_history_signatures(&[first.clone(), second.clone()], 50)
            .unwrap();
        db.upsert_topic_history_signatures(std::slice::from_ref(&first), 50)
            .unwrap();
        let recent = db.get_recent_topic_history_signatures(1, 20).unwrap();
        assert!(recent.contains(&first));
        assert!(recent.contains(&second));

        let rejected = format!("TEST_TOPIC_REJECTED_{suffix}");
        assert!(db
            .upsert_topic_history_signatures(&[rejected.clone(), " ".to_string()], 50)
            .is_err());
        let mut conn = db.get_conn().unwrap();
        let rejected_count = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM topic_novelty_history WHERE signature = ?1",
        )
        .bind::<diesel::sql_types::Text, _>(&rejected)
        .get_result::<Count>(&mut conn)
        .unwrap();
        assert_eq!(
            rejected_count.count, 0,
            "bad batch must not partially persist"
        );

        let persisted = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM topic_novelty_history WHERE signature IN (?1, ?2)",
        )
        .bind::<diesel::sql_types::Text, _>(&first)
        .bind::<diesel::sql_types::Text, _>(&second)
        .get_result::<Count>(&mut conn)
        .unwrap();
        assert_eq!(persisted.count, 2, "duplicate signature remains idempotent");

        diesel::sql_query("DELETE FROM topic_novelty_history WHERE signature IN (?1, ?2, ?3)")
            .bind::<diesel::sql_types::Text, _>(&first)
            .bind::<diesel::sql_types::Text, _>(&second)
            .bind::<diesel::sql_types::Text, _>(&rejected)
            .execute(&mut conn)
            .unwrap();
    }

    #[test]
    #[serial_test::serial]
    fn remaining_root_accessors_and_factor_ic_query_use_real_sql_rows() {
        init_db_for_test();
        let db = DatabaseManager::get();
        assert!(std::ptr::eq(get_db(), db));
        assert_eq!(
            DatabaseManager::with_db("TEST_CODE_ROOT", |_| Some(7)),
            Some(7)
        );

        let suffix = unique_test_label("FACTOR_IC");
        let code = format!("TEST_CODE_{suffix}");
        let buy_price = 1234.567;
        let sell_price = 1300.0;
        let buy_date = "2198-01-02";
        let mut conn = db.get_conn().expect("test database connection");
        diesel::sql_query(
            "INSERT INTO stock_position
             (code, name, buy_date, buy_price, quantity, status, sell_date, sell_price, return_rate)
             VALUES (?, 'TEST_CODE_因子样本', ?, ?, 100, 'closed', '2198-01-03', ?, 5.0)",
        )
        .bind::<diesel::sql_types::Text, _>(&code)
        .bind::<diesel::sql_types::Text, _>(buy_date)
        .bind::<diesel::sql_types::Double, _>(buy_price)
        .bind::<diesel::sql_types::Double, _>(sell_price)
        .execute(&mut conn)
        .expect("insert complete closed position");
        drop(conn);

        let rows = db.get_factor_ic_data().expect("factor IC repository query");
        let row = rows
            .iter()
            .find(|row| (row.buy_price - buy_price).abs() < f64::EPSILON)
            .expect("inserted factor IC row");
        assert_eq!(row.sell_price, sell_price);
        assert_eq!(row.sentiment_score, None);
        assert_eq!(row.score_breakdown_json, None);

        let mut conn = db.get_conn().expect("cleanup database connection");
        diesel::sql_query("DELETE FROM stock_position WHERE code = ?")
            .bind::<diesel::sql_types::Text, _>(&code)
            .execute(&mut conn)
            .expect("cleanup factor IC row");
    }

    #[test]
    fn test_order_tables_reject_invalid_direct_writes() {
        init_db_for_test();
        let mut conn = DatabaseManager::get()
            .get_conn()
            .expect("test DB connection");

        let invalid_position = diesel::sql_query(
            "INSERT INTO stock_position
             (code, name, buy_date, buy_price, quantity, status)
             VALUES ('TEST_CODE_INVALID_LOT', '测试', '2026-07-17', 10.0, 99, 'open')",
        )
        .execute(&mut conn);
        assert!(invalid_position.is_err());

        let invalid_paper = diesel::sql_query(
            "INSERT INTO paper_trades
             (plan_id, code, name, direction, price, quantity, status,
              virtual_reason, account_mode, data_mode)
             VALUES ('TEST_PLAN_INVALID_PRICE', 'TEST_CODE_000001', '测试', 'buy',
                     0.0, 100, 'Invalidated', 'NewsCatalyst', 'Normal', 'Full')",
        )
        .execute(&mut conn);
        assert!(invalid_paper.is_err());

        let rejected_without_reason = diesel::sql_query(
            "INSERT INTO order_audit
             (business_order_id, source, decision_basis, side, code,
              requested_price, execution_price, quantity, quote_observed_at,
              outcome, failure_reason)
             VALUES ('TEST_ORDER_INVALID_REJECT', 'DatabaseTest', 'test', 'buy',
                     'TEST_CODE_INVALID', 0.0, NULL, 0, NULL, 'Rejected', NULL)",
        )
        .execute(&mut conn);
        assert!(rejected_without_reason.is_err());

        let filled_without_quote = diesel::sql_query(
            "INSERT INTO order_audit
             (business_order_id, source, decision_basis, side, code,
              requested_price, execution_price, quantity, quote_observed_at,
              outcome, failure_reason)
             VALUES ('TEST_ORDER_INVALID_FILL', 'DatabaseTest', 'test', 'buy',
                     'TEST_CODE_INVALID', 10.0, 10.0, 100, NULL, 'Filled', NULL)",
        )
        .execute(&mut conn);
        assert!(filled_without_quote.is_err());
    }

    #[test]
    fn br094_agent_decision_audit_is_append_only() {
        init_db_for_test();
        let mut conn = DatabaseManager::get()
            .get_conn()
            .expect("test DB connection");
        let session = format!("TEST_CODE_AGENT_AUDIT_{}", std::process::id());
        diesel::sql_query(
            "INSERT INTO agent_scratchpad (session_id, step, log_type, content) \
             VALUES (?, 1, 'decision', 'TEST_CODE immutable evidence')",
        )
        .bind::<diesel::sql_types::Text, _>(&session)
        .execute(&mut conn)
        .expect("append agent audit row");

        let update = diesel::sql_query(
            "UPDATE agent_scratchpad SET content = 'tampered' WHERE session_id = ?",
        )
        .bind::<diesel::sql_types::Text, _>(&session)
        .execute(&mut conn);
        let delete = diesel::sql_query("DELETE FROM agent_scratchpad WHERE session_id = ?")
            .bind::<diesel::sql_types::Text, _>(&session)
            .execute(&mut conn);

        assert!(update.is_err(), "agent decision audit must reject UPDATE");
        assert!(delete.is_err(), "agent decision audit must reject DELETE");
    }

    #[test]
    fn br084_business_order_reservation_is_persistent_and_atomic() {
        init_db_for_test();
        let id = format!(
            "TEST_ORDER_RESERVATION_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let db = DatabaseManager::get();
        assert!(db
            .reserve_business_order_id(&id)
            .expect("first reservation"));
        assert!(
            !db.reserve_business_order_id(&id)
                .expect("duplicate reservation query"),
            "the same ID must be rejected by shared persistence within 60 seconds"
        );
    }

    #[test]
    fn test_order_audit_is_immutable_and_atomic_with_position_fill() {
        use crate::database::order_audit::OrderAuditRecord;
        use crate::models::NewStockPosition;

        init_db_for_test();
        let db = DatabaseManager::get();
        let code = "TEST_CODE_600398";
        let position = NewStockPosition {
            code: code.to_string(),
            name: "审计测试".to_string(),
            buy_date: "2026-07-17".to_string(),
            buy_price: 10.0,
            quantity: 100,
            status: "open".to_string(),
            st_type: None,
            chain_name: None,
        };
        let audit = OrderAuditRecord {
            business_order_id: "TEST_ORDER_AUDIT_ATOMIC",
            source: "DatabaseTest",
            decision_basis: "test",
            side: "buy",
            code,
            requested_price: 10.0,
            execution_price: Some(10.0),
            quantity: 100,
            quote_observed_at: Some("2026-07-17T09:30:00+08:00"),
            outcome: "Filled",
            failure_reason: None,
        };
        let assignment = crate::data_gateway::derive_position_chain(
            code,
            crate::data_gateway::GatewayBatch::Available {
                records: vec![crate::data_gateway::BoardMembershipRecord {
                    instrument_code: code.to_string(),
                    board_code: "TEST_CODE_INDUSTRY".to_string(),
                    board_name: "测试产业链".to_string(),
                    kind: crate::data_gateway::BoardKind::Industry,
                }],
                evidence: crate::data_gateway::BatchEvidence {
                    provider: crate::magic_compat::ProviderId::Tdx,
                    source: "TEST_CODE_tdx-board-memberships".to_string(),
                    source_at: None,
                    observed_at: "2026-07-27T09:30:01+08:00".to_string(),
                    batch_id: "TEST_CODE_AUDIT_ATOMIC_BATCH".to_string(),
                },
            },
        )
        .expect("valid position-chain batch")
        .expect("position-chain assignment");
        db.save_position_with_audit_and_assignment(&position, &audit, &assignment)
            .expect("atomic audited position fill");

        let mut conn = db.get_conn().expect("test DB connection");
        #[derive(diesel::QueryableByName)]
        struct Count {
            #[diesel(sql_type = diesel::sql_types::BigInt)]
            count: i64,
        }
        let count: Count = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM order_audit
             WHERE business_order_id = 'TEST_ORDER_AUDIT_ATOMIC' AND outcome = 'Filled'",
        )
        .get_result(&mut conn)
        .expect("query audit");
        assert_eq!(count.count, 1);
        let chain_count: Count = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM order_audit_chain
             WHERE order_audit_id IN (
                 SELECT id FROM order_audit
                 WHERE business_order_id = 'TEST_ORDER_AUDIT_ATOMIC'
             )",
        )
        .get_result(&mut conn)
        .expect("query audit chain evidence");
        assert_eq!(chain_count.count, 1);

        assert!(diesel::sql_query(
            "UPDATE order_audit SET outcome = 'Rejected'
             WHERE business_order_id = 'TEST_ORDER_AUDIT_ATOMIC'",
        )
        .execute(&mut conn)
        .is_err());
        assert!(diesel::sql_query(
            "UPDATE order_audit_chain SET record_hash = 'tampered'
             WHERE order_audit_id IN (
                 SELECT id FROM order_audit
                 WHERE business_order_id = 'TEST_ORDER_AUDIT_ATOMIC'
             )",
        )
        .execute(&mut conn)
        .is_err());
        assert!(diesel::sql_query(
            "DELETE FROM order_audit WHERE business_order_id = 'TEST_ORDER_AUDIT_ATOMIC'",
        )
        .execute(&mut conn)
        .is_err());

        diesel::sql_query("DELETE FROM stock_position WHERE code = 'TEST_CODE_AUDIT_ATOMIC'")
            .execute(&mut conn)
            .expect("cleanup audited position");
    }

    #[test]
    fn test_save_and_retrieve() {
        init_db_for_test();
        let db = DatabaseManager::get();

        // 保存数据
        let date = NaiveDate::from_ymd_opt(2026, 1, 22).unwrap();
        db.save_daily_record(
            "TEST_CODE_600519",
            date,
            Some(1800.0),
            Some(1850.0),
            Some(1780.0),
            Some(1820.0),
            Some(10000000.0),
            Some(18200000000.0),
            Some(1.5),
            Some(1810.0),
            Some(1800.0),
            Some(1790.0),
            Some(1.2),
            Some("TestSource"),
        )
        .expect("保存数据失败");

        // 检查数据是否存在
        let has_data = db
            .has_data_for_date("TEST_CODE_600519", date)
            .expect("查询失败");
        assert!(has_data);

        // 获取数据
        let data = db
            .get_latest_data("TEST_CODE_600519", 1)
            .expect("获取数据失败");
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].code, "TEST_CODE_600519");
        assert_eq!(data[0].close, Some(1820.0));

        // 清理数据（不删DB文件，并行测试可能还在用）
        db.delete_stock_data("TEST_CODE_600519").ok();
    }

    // v14.1 task #167: stock_position.st_type round-trip DB 集成测试
    // 路径: save_position(NewStockPosition{ st_type: Some("*ST") }) →
    //       get_all_open_positions → StockPosition.st_type 真读出
    // 用独立 DB 文件避免与上面 test_save_and_query_stock_data 竞态.
    #[test]
    fn test_st_type_db_round_trip() {
        use crate::models::{NewStockPosition, StockPosition};
        use crate::schema::stock_position;
        use diesel::prelude::*;

        init_db_for_test();

        let db = DatabaseManager::get();

        // 1. insert 一只 *ST 持仓
        let new_pos = NewStockPosition {
            code: "TEST_CODE_600090".to_string(),
            name: "*ST测试".to_string(),
            buy_date: "2026-07-01".to_string(),
            buy_price: 5.0,
            quantity: 1000,
            status: "open".to_string(),
            st_type: Some("*ST".to_string()),
            chain_name: None,
        };
        db.save_position(&new_pos).expect("save_position 失败");

        // 2. 读回 — 验证 st_type 真写入
        let mut conn = db.get_conn().expect("get_conn 失败");
        let row: StockPosition = stock_position::table
            .filter(stock_position::code.eq("TEST_CODE_600090"))
            .first(&mut conn)
            .expect("query 失败");
        assert_eq!(
            row.st_type.as_deref(),
            Some("*ST"),
            "st_type 写入/读出不一致"
        );
        assert_eq!(row.code, "TEST_CODE_600090");
        assert_eq!(row.name, "*ST测试");
        assert_eq!(row.quantity, 1000);

        // 3. 测试 upsert: 同 (code, buy_date) 再 save 不报错, st_type 应被 excluded 同步
        let update_pos = NewStockPosition {
            code: "TEST_CODE_600090".to_string(),
            name: "*ST测试改名".to_string(),
            buy_date: "2026-07-01".to_string(),
            buy_price: 5.5,
            quantity: 1500,
            status: "open".to_string(),
            st_type: Some("ST".to_string()), // 改 ST
            chain_name: None,
        };
        db.save_position(&update_pos).expect("upsert 失败");

        let row2: StockPosition = stock_position::table
            .filter(stock_position::code.eq("TEST_CODE_600090"))
            .first(&mut conn)
            .expect("re-query 失败");
        assert_eq!(row2.st_type.as_deref(), Some("ST"), "upsert st_type 未同步");
        assert_eq!(row2.chain_name, None, "raw chain_name must remain absent");
        assert_eq!(row2.name, "*ST测试改名", "upsert name 未同步");

        diesel::delete(stock_position::table.filter(stock_position::code.eq("TEST_CODE_600090")))
            .execute(&mut conn)
            .expect("cleanup test position");
    }

    // v14.1 review fix: 测试 backfill_st_type 前缀锚定 (LIKE 'ST%' / 'ST*%' 而非 '%ST%')
    // 之前 '%ST%' 子串匹配会把 'BEST' / 'GST' 误判成 ST 类
    #[test]
    fn test_backfill_st_type_prefix_anchored() {
        use crate::models::NewStockPosition;
        use crate::schema::stock_position;
        use diesel::prelude::*;

        let test_db = "./test_data/test_backfill_st_type.db";
        std::fs::create_dir_all("./test_data").ok();
        let _ = std::fs::remove_file(test_db);
        let _ = DatabaseManager::init(Some(PathBuf::from(test_db)));
        // review fix: RAII guard, panic 时 Drop 清理 test_db
        let _guard = TestDbGuard(test_db);

        let db = DatabaseManager::get();

        // Insert 4 测试持仓: 真正 ST 开头 + 子串含 ST (非 ST 类) + 普通 + *ST
        let cases = vec![
            ("TEST_CODE_001", "ST康美", Some("ST")),
            ("TEST_CODE_002", "*ST华微", Some("*ST")),
            ("TEST_CODE_003", "BEST新材", None), // 子串含 ST 但不是 ST 类
            ("TEST_CODE_004", "GST电子", None),  // 子串含 ST 但不是 ST 类
            ("TEST_CODE_005", "浦发银行", None), // 普通
            ("TEST_CODE_006", "SST集成", Some("ST")),
            ("TEST_CODE_007", "S*ST海伦", Some("*ST")),
        ];
        for (code, name, _expected) in &cases {
            db.save_position(&NewStockPosition {
                code: code.to_string(),
                name: name.to_string(),
                buy_date: "2026-07-01".to_string(),
                buy_price: 10.0,
                quantity: 100,
                status: "open".to_string(),
                st_type: None,
                chain_name: None,
            })
            .expect("save 失败");
        }

        // 跑 backfill
        let updated = db.backfill_st_type().expect("backfill 失败");
        assert!(updated > 0, "至少应更新 4 条真 ST 类");

        // 验证每个 case
        let mut conn = db.get_conn().unwrap();
        for (code, name, expected) in &cases {
            let row: StockPosition = stock_position::table
                .filter(stock_position::code.eq(code as &str))
                .first(&mut conn)
                .expect("query 失败");
            assert_eq!(
                row.st_type.as_deref(),
                *expected,
                "code={code} name={name} expected={expected:?} got={:?}",
                row.st_type
            );
        }

        for (code, _, _) in &cases {
            diesel::delete(stock_position::table.filter(stock_position::code.eq(*code)))
                .execute(&mut conn)
                .expect("cleanup backfill test position");
        }
    }

    // v14.1 review fix: 测试 save_position upsert 不覆盖 st_type (COALESCE 行为)
    // 之前 excluded(st_type) 会把 backfill 写好的 *ST 清成 NULL
    #[test]
    fn test_save_position_upsert_preserves_st_type() {
        use crate::models::NewStockPosition;
        use crate::schema::stock_position;
        use diesel::prelude::*;

        let test_db = "./test_data/test_upsert_preserve_st.db";
        std::fs::create_dir_all("./test_data").ok();
        let _ = std::fs::remove_file(test_db);
        let _ = DatabaseManager::init(Some(PathBuf::from(test_db)));
        // review fix: RAII guard
        let _guard = TestDbGuard(test_db);

        let db = DatabaseManager::get();

        // 1. 首次 insert, st_type=None
        db.save_position(&NewStockPosition {
            code: "TEST_CODE_600519".to_string(),
            name: "贵州茅台".to_string(),
            buy_date: "2026-07-01".to_string(),
            buy_price: 1800.0,
            quantity: 100,
            status: "open".to_string(),
            st_type: None,
            chain_name: None,
        })
        .expect("save 1 失败");

        // 2. 模拟 broker 推送 *ST (用 raw SQL 写, 模拟 broker update path)
        let mut conn = db.get_conn().unwrap();
        diesel::sql_query(
            "UPDATE stock_position SET st_type = '*ST' WHERE code = 'TEST_CODE_600519'",
        )
        .execute(&mut conn)
        .expect("st_type set 失败");

        // 3. trading::open_position re-buy 同 (code, buy_date) — 传 None
        db.save_position(&NewStockPosition {
            code: "TEST_CODE_600519".to_string(),
            name: "贵州茅台".to_string(),
            buy_date: "2026-07-01".to_string(),
            buy_price: 1850.0, // 价格变 (新买入)
            quantity: 200,     // 数量变
            status: "open".to_string(),
            st_type: None,    // 重买不带 st_type
            chain_name: None, // 重买不带 chain
        })
        .expect("save 2 失败");

        // 4. 验证: st_type 应保持 '*ST' (COALESCE 保 NULL 时不覆盖), 价格/数量更新
        let row: StockPosition = stock_position::table
            .filter(stock_position::code.eq("TEST_CODE_600519"))
            .first(&mut conn)
            .expect("re-query 失败");
        assert_eq!(
            row.st_type.as_deref(),
            Some("*ST"),
            "st_type 应保持 broker 推送的 *ST, 不应被 re-buy NULL 覆盖"
        );
        assert_eq!(row.buy_price, 1850.0, "价格应更新");
        assert_eq!(row.quantity, 200, "数量应更新");

        diesel::delete(stock_position::table.filter(stock_position::code.eq("TEST_CODE_600519")))
            .execute(&mut conn)
            .expect("cleanup upsert test position");
    }
}
