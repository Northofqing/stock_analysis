//! Dedicated SQLite ownership boundary for operational-readiness recovery facts.

#![cfg_attr(not(test), allow(dead_code))]

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

use rusqlite::{
    params, Connection, DatabaseName, OpenFlags, OptionalExtension, Transaction,
    TransactionBehavior,
};
use sha2::{Digest, Sha256};

use crate::monitor::push_job::{canonical_digest, namespace_value, Namespace};

use super::migration::validate_database_path;
use super::readiness_sqlite_io::{lock_main_file_and_read_header, SQLITE_HEADER_LEN};

const SCHEMA_VERSION: i64 = 1;
const NAMESPACE_DOMAIN: &str = "OperationalReadinessNamespace/v1";
const SQLITE_MAGIC: &[u8; 16] = b"SQLite format 3\0";
const SQLITE_WRITE_VERSION_OFFSET: usize = 18;
const SQLITE_READ_VERSION_OFFSET: usize = 19;
const ROLLBACK_JOURNAL_VERSION: u8 = 1;
const DDL: &str = r#"
CREATE TABLE operational_readiness_schema(
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    schema_version INTEGER NOT NULL,
    ddl_sha256 TEXT NOT NULL CHECK(typeof(ddl_sha256)='text' AND length(ddl_sha256)=64 AND ddl_sha256 NOT GLOB '*[^0-9a-f]*'),
    namespace_sha256 TEXT NOT NULL CHECK(typeof(namespace_sha256)='text' AND length(namespace_sha256)=64 AND namespace_sha256 NOT GLOB '*[^0-9a-f]*')
);

CREATE TABLE operational_readiness_snapshot(
    snapshot_id TEXT PRIMARY KEY NOT NULL CHECK(typeof(snapshot_id)='text' AND length(snapshot_id)=64 AND snapshot_id NOT GLOB '*[^0-9a-f]*'),
    event_id TEXT NOT NULL UNIQUE CHECK(typeof(event_id)='text' AND length(event_id)=64 AND event_id NOT GLOB '*[^0-9a-f]*'),
    canonical_bytes BLOB NOT NULL CHECK(typeof(canonical_bytes)='blob' AND length(canonical_bytes)>0),
    FOREIGN KEY(event_id) REFERENCES operational_readiness_recovery_event(event_id)
        DEFERRABLE INITIALLY DEFERRED
);

CREATE TABLE operational_readiness_recovery_event(
    event_id TEXT PRIMARY KEY NOT NULL CHECK(typeof(event_id)='text' AND length(event_id)=64 AND event_id NOT GLOB '*[^0-9a-f]*'),
    event_sha256 TEXT NOT NULL CHECK(typeof(event_sha256)='text' AND length(event_sha256)=64 AND event_sha256 NOT GLOB '*[^0-9a-f]*'),
    before_snapshot_id TEXT CHECK(before_snapshot_id IS NULL OR (typeof(before_snapshot_id)='text' AND length(before_snapshot_id)=64 AND before_snapshot_id NOT GLOB '*[^0-9a-f]*')),
    after_snapshot_id TEXT NOT NULL UNIQUE CHECK(typeof(after_snapshot_id)='text' AND length(after_snapshot_id)=64 AND after_snapshot_id NOT GLOB '*[^0-9a-f]*'),
    canonical_bytes BLOB NOT NULL CHECK(typeof(canonical_bytes)='blob' AND length(canonical_bytes)>0),
    FOREIGN KEY(before_snapshot_id) REFERENCES operational_readiness_snapshot(snapshot_id)
        DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY(after_snapshot_id) REFERENCES operational_readiness_snapshot(snapshot_id)
        DEFERRABLE INITIALLY DEFERRED
);

CREATE TABLE operational_readiness_head(
    scope_key TEXT PRIMARY KEY NOT NULL CHECK(typeof(scope_key)='text' AND length(scope_key)=64 AND scope_key NOT GLOB '*[^0-9a-f]*'),
    version INTEGER NOT NULL CHECK(version>0),
    snapshot_id TEXT NOT NULL CHECK(typeof(snapshot_id)='text' AND length(snapshot_id)=64 AND snapshot_id NOT GLOB '*[^0-9a-f]*'),
    event_id TEXT NOT NULL CHECK(typeof(event_id)='text' AND length(event_id)=64 AND event_id NOT GLOB '*[^0-9a-f]*'),
    FOREIGN KEY(snapshot_id) REFERENCES operational_readiness_snapshot(snapshot_id),
    FOREIGN KEY(event_id) REFERENCES operational_readiness_recovery_event(event_id)
);

CREATE TRIGGER operational_readiness_schema_no_update
BEFORE UPDATE ON operational_readiness_schema
BEGIN
    SELECT RAISE(ABORT, 'operational readiness schema is immutable');
END;

CREATE TRIGGER operational_readiness_schema_no_delete
BEFORE DELETE ON operational_readiness_schema
BEGIN
    SELECT RAISE(ABORT, 'operational readiness schema is immutable');
END;

CREATE TRIGGER operational_readiness_schema_no_replace
BEFORE INSERT ON operational_readiness_schema
WHEN EXISTS(SELECT 1 FROM operational_readiness_schema)
BEGIN
    SELECT RAISE(ABORT, 'operational readiness schema cannot be replaced');
END;

CREATE TRIGGER operational_readiness_snapshot_no_update
BEFORE UPDATE ON operational_readiness_snapshot
BEGIN
    SELECT RAISE(ABORT, 'operational readiness snapshots are append-only');
END;

CREATE TRIGGER operational_readiness_snapshot_no_delete
BEFORE DELETE ON operational_readiness_snapshot
BEGIN
    SELECT RAISE(ABORT, 'operational readiness snapshots are append-only');
END;

CREATE TRIGGER operational_readiness_snapshot_no_replace
BEFORE INSERT ON operational_readiness_snapshot
WHEN EXISTS(
    SELECT 1 FROM operational_readiness_snapshot
    WHERE snapshot_id=NEW.snapshot_id OR event_id=NEW.event_id
)
BEGIN
    SELECT RAISE(ABORT, 'operational readiness snapshots cannot be replaced');
END;

CREATE TRIGGER operational_readiness_recovery_event_no_update
BEFORE UPDATE ON operational_readiness_recovery_event
BEGIN
    SELECT RAISE(ABORT, 'operational readiness recovery events are append-only');
END;

CREATE TRIGGER operational_readiness_recovery_event_no_delete
BEFORE DELETE ON operational_readiness_recovery_event
BEGIN
    SELECT RAISE(ABORT, 'operational readiness recovery events are append-only');
END;

CREATE TRIGGER operational_readiness_recovery_event_no_replace
BEFORE INSERT ON operational_readiness_recovery_event
WHEN EXISTS(
    SELECT 1 FROM operational_readiness_recovery_event
    WHERE event_id=NEW.event_id OR after_snapshot_id=NEW.after_snapshot_id
)
BEGIN
    SELECT RAISE(ABORT, 'operational readiness recovery events cannot be replaced');
END;
"#;

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum ReadinessSchemaError {
    #[error("operational readiness database path was rejected: {check}")]
    InvalidDatabasePath { check: &'static str },
    #[error("operational readiness database does not exist")]
    DatabaseMissing,
    #[error("cannot open operational readiness database")]
    DatabaseOpenFailed,
    #[error("cannot initialize operational readiness database: {check}")]
    InitializationFailed { check: &'static str },
    #[error("operational readiness schema validation failed: {check}")]
    ValidationFailed { check: &'static str },
    #[error("operational readiness connection safeguard failed: {check}")]
    ConnectionSafeguardFailed { check: &'static str },
}

pub(crate) fn initialize_database(
    path: &Path,
    namespace: &Namespace,
) -> Result<(), ReadinessSchemaError> {
    initialize_database_inner(path, namespace, || {}, || {}, || {}, || {})
}

#[cfg(test)]
pub(super) fn initialize_database_with_before_create_hook<F>(
    path: &Path,
    namespace: &Namespace,
    before_create: F,
) -> Result<(), ReadinessSchemaError>
where
    F: FnOnce(),
{
    initialize_database_inner(path, namespace, before_create, || {}, || {}, || {})
}

#[cfg(test)]
pub(super) fn initialize_database_with_creation_hooks<F, G>(
    path: &Path,
    namespace: &Namespace,
    before_create: F,
    after_create: G,
) -> Result<(), ReadinessSchemaError>
where
    F: FnOnce(),
    G: FnOnce(),
{
    initialize_database_inner(path, namespace, before_create, after_create, || {}, || {})
}

#[cfg(test)]
pub(super) fn initialize_database_with_connection_open_hooks<F, G>(
    path: &Path,
    namespace: &Namespace,
    before_connection_open: F,
    after_connection_open: G,
) -> Result<(), ReadinessSchemaError>
where
    F: FnOnce(),
    G: FnOnce(),
{
    initialize_database_inner(
        path,
        namespace,
        || {},
        || {},
        before_connection_open,
        after_connection_open,
    )
}

fn initialize_database_inner<F, G, H, I>(
    path: &Path,
    namespace: &Namespace,
    before_create: F,
    after_create: G,
    before_connection_open: H,
    after_connection_open: I,
) -> Result<(), ReadinessSchemaError>
where
    F: FnOnce(),
    G: FnOnce(),
    H: FnOnce(),
    I: FnOnce(),
{
    validate_path(path)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => {
            return with_write_transaction(path, namespace, |_| Ok::<(), ReadinessSchemaError>(()))
        }
        Ok(_) => {
            return Err(ReadinessSchemaError::InvalidDatabasePath {
                check: "target_regular_file",
            })
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => {
            return Err(ReadinessSchemaError::InvalidDatabasePath {
                check: "target_metadata",
            })
        }
    }

    let image = build_database_image(namespace)?;
    before_create();
    let mut owned_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| ReadinessSchemaError::InitializationFailed {
            check: "claim_new_target",
        })?;
    let owned_identity = owned_file_identity(&owned_file)?;
    after_create();
    verify_owned_target(path, owned_identity)?;
    before_connection_open();
    owned_file
        .write_all(&image)
        .map_err(|_| ReadinessSchemaError::InitializationFailed {
            check: "write_owned_image",
        })?;
    owned_file
        .sync_all()
        .map_err(|_| ReadinessSchemaError::InitializationFailed {
            check: "sync_owned_image",
        })?;
    after_connection_open();
    verify_owned_target(path, owned_identity)?;
    validate_rollback_journal_header_bytes(&image)
}

pub(crate) fn with_read_only<T, E, F>(
    path: &Path,
    namespace: &Namespace,
    operation: F,
) -> Result<T, E>
where
    E: From<ReadinessSchemaError>,
    F: for<'transaction> FnOnce(&Transaction<'transaction>) -> Result<T, E>,
{
    with_read_only_inner(path, namespace, || {}, || {}, operation)
}

#[cfg(test)]
pub(super) fn with_read_only_hooks<T, E, F, G, H>(
    path: &Path,
    namespace: &Namespace,
    after_header: G,
    after_shared_lock: H,
    operation: F,
) -> Result<T, E>
where
    E: From<ReadinessSchemaError>,
    F: for<'transaction> FnOnce(&Transaction<'transaction>) -> Result<T, E>,
    G: FnOnce(),
    H: FnOnce(),
{
    with_read_only_inner(path, namespace, after_header, after_shared_lock, operation)
}

fn with_read_only_inner<T, E, F, G, H>(
    path: &Path,
    namespace: &Namespace,
    after_header: G,
    after_shared_lock: H,
    operation: F,
) -> Result<T, E>
where
    E: From<ReadinessSchemaError>,
    F: for<'transaction> FnOnce(&Transaction<'transaction>) -> Result<T, E>,
    G: FnOnce(),
    H: FnOnce(),
{
    let connection =
        open_prechecked_connection(path, OpenFlags::SQLITE_OPEN_READ_ONLY, after_header, true)
            .map_err(E::from)?;
    let (file_lock, header) = lock_main_file_and_read_header(&connection)
        .map_err(|check| E::from(ReadinessSchemaError::ValidationFailed { check }))?;
    validate_rollback_journal_header_bytes(&header).map_err(E::from)?;
    after_shared_lock();
    file_lock.handoff_to_connection();
    let transaction = Transaction::new_unchecked(&connection, TransactionBehavior::Deferred)
        .map_err(|_| E::from(transaction_error("begin_read_transaction")))?;
    validate_schema(&transaction, namespace).map_err(E::from)?;
    finish_transaction(transaction, operation)
}

pub(crate) fn with_write_transaction<T, E, F>(
    path: &Path,
    namespace: &Namespace,
    operation: F,
) -> Result<T, E>
where
    E: From<ReadinessSchemaError>,
    F: for<'transaction> FnOnce(&Transaction<'transaction>) -> Result<T, E>,
{
    with_write_transaction_inner(path, namespace, || {}, || {}, operation)
}

#[cfg(test)]
pub(super) fn with_write_transaction_hooks<T, E, F, G, H>(
    path: &Path,
    namespace: &Namespace,
    after_header: G,
    after_shared_lock: H,
    operation: F,
) -> Result<T, E>
where
    E: From<ReadinessSchemaError>,
    F: for<'transaction> FnOnce(&Transaction<'transaction>) -> Result<T, E>,
    G: FnOnce(),
    H: FnOnce(),
{
    with_write_transaction_inner(path, namespace, after_header, after_shared_lock, operation)
}

fn with_write_transaction_inner<T, E, F, G, H>(
    path: &Path,
    namespace: &Namespace,
    after_header: G,
    after_shared_lock: H,
    operation: F,
) -> Result<T, E>
where
    E: From<ReadinessSchemaError>,
    F: for<'transaction> FnOnce(&Transaction<'transaction>) -> Result<T, E>,
    G: FnOnce(),
    H: FnOnce(),
{
    let connection =
        open_prechecked_connection(path, OpenFlags::SQLITE_OPEN_READ_WRITE, after_header, false)
            .map_err(E::from)?;
    let (file_lock, header) = lock_main_file_and_read_header(&connection)
        .map_err(|check| E::from(ReadinessSchemaError::ValidationFailed { check }))?;
    validate_rollback_journal_header_bytes(&header).map_err(E::from)?;
    after_shared_lock();
    file_lock.handoff_to_connection();
    let transaction = Transaction::new_unchecked(&connection, TransactionBehavior::Immediate)
        .map_err(|_| E::from(transaction_error("begin_write_transaction")))?;
    validate_schema(&transaction, namespace).map_err(E::from)?;
    finish_transaction(transaction, operation)
}

fn validate_path(path: &Path) -> Result<(), ReadinessSchemaError> {
    validate_database_path(path).map_err(|_| ReadinessSchemaError::InvalidDatabasePath {
        check: "database_path",
    })
}

fn validate_existing_path(path: &Path) -> Result<(), ReadinessSchemaError> {
    validate_path(path)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(()),
        Ok(_) => Err(ReadinessSchemaError::InvalidDatabasePath {
            check: "target_regular_file",
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Err(ReadinessSchemaError::DatabaseMissing)
        }
        Err(_) => Err(ReadinessSchemaError::InvalidDatabasePath {
            check: "target_metadata",
        }),
    }
}

fn build_database_image(namespace: &Namespace) -> Result<Vec<u8>, ReadinessSchemaError> {
    let mut connection =
        Connection::open_in_memory().map_err(|_| ReadinessSchemaError::InitializationFailed {
            check: "open_image_database",
        })?;
    enable_and_verify_foreign_keys(&connection)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| ReadinessSchemaError::InitializationFailed {
            check: "begin_image_transaction",
        })?;
    transaction
        .execute_batch(DDL)
        .map_err(|_| ReadinessSchemaError::InitializationFailed {
            check: "create_image_schema",
        })?;
    transaction
        .execute(
            "INSERT INTO operational_readiness_schema(\
                 singleton,schema_version,ddl_sha256,namespace_sha256\
             ) VALUES(1,?1,?2,?3)",
            params![SCHEMA_VERSION, ddl_sha256(), namespace_sha256(namespace)],
        )
        .map_err(|_| ReadinessSchemaError::InitializationFailed {
            check: "write_image_header",
        })?;
    transaction
        .commit()
        .map_err(|_| ReadinessSchemaError::InitializationFailed {
            check: "commit_image_schema",
        })?;
    validate_schema(&connection, namespace)?;
    let image = connection
        .serialize(DatabaseName::Main)
        .map_err(|_| ReadinessSchemaError::InitializationFailed {
            check: "serialize_image",
        })?
        .to_vec();
    validate_rollback_journal_header_bytes(&image)?;
    Ok(image)
}

fn open_prechecked_connection<F>(
    path: &Path,
    access: OpenFlags,
    after_header: F,
    read_only: bool,
) -> Result<Connection, ReadinessSchemaError>
where
    F: FnOnce(),
{
    validate_existing_path(path)?;
    after_header();
    let connection = Connection::open_with_flags(
        path,
        access
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW
            | OpenFlags::SQLITE_OPEN_PRIVATE_CACHE,
    )
    .map_err(|_| ReadinessSchemaError::DatabaseOpenFailed)?;
    let locking_mode: String = connection
        .query_row("PRAGMA main.locking_mode=EXCLUSIVE", [], |row| row.get(0))
        .map_err(|_| ReadinessSchemaError::ConnectionSafeguardFailed {
            check: "locking_mode",
        })?;
    if locking_mode != "exclusive" {
        return Err(ReadinessSchemaError::ConnectionSafeguardFailed {
            check: "locking_mode",
        });
    }
    if read_only {
        connection
            .execute_batch("PRAGMA query_only=ON;")
            .map_err(|_| ReadinessSchemaError::ConnectionSafeguardFailed {
                check: "enable_query_only",
            })?;
        require_pragma(&connection, "PRAGMA query_only", 1, "query_only")?;
    } else {
        enable_and_verify_foreign_keys(&connection)?;
    }
    Ok(connection)
}

fn transaction_error(check: &'static str) -> ReadinessSchemaError {
    ReadinessSchemaError::ConnectionSafeguardFailed { check }
}

fn finish_transaction<T, E, F>(transaction: Transaction<'_>, operation: F) -> Result<T, E>
where
    E: From<ReadinessSchemaError>,
    F: for<'connection> FnOnce(&Transaction<'connection>) -> Result<T, E>,
{
    match operation(&transaction) {
        Ok(value) => {
            transaction
                .commit()
                .map_err(|_| E::from(transaction_error("commit_transaction")))?;
            Ok(value)
        }
        Err(error) => {
            transaction
                .rollback()
                .map_err(|_| E::from(transaction_error("rollback_transaction")))?;
            Err(error)
        }
    }
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileIdentity {
    device: u64,
    inode: u64,
}

#[cfg(unix)]
fn owned_file_identity(file: &File) -> Result<FileIdentity, ReadinessSchemaError> {
    let metadata = file
        .metadata()
        .map_err(|_| ReadinessSchemaError::InitializationFailed {
            check: "owned_target_metadata",
        })?;
    Ok(FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(unix)]
fn verify_owned_target(path: &Path, expected: FileIdentity) -> Result<(), ReadinessSchemaError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| ReadinessSchemaError::InitializationFailed {
            check: "owned_target_identity",
        })?;
    let actual = FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() || actual != expected {
        return Err(ReadinessSchemaError::InitializationFailed {
            check: "owned_target_identity",
        });
    }
    Ok(())
}

#[cfg(not(unix))]
#[derive(Clone, Copy, Debug)]
struct FileIdentity;

#[cfg(not(unix))]
fn owned_file_identity(_file: &File) -> Result<FileIdentity, ReadinessSchemaError> {
    Err(ReadinessSchemaError::InitializationFailed {
        check: "owned_target_identity_unsupported",
    })
}

#[cfg(not(unix))]
fn verify_owned_target(_path: &Path, _expected: FileIdentity) -> Result<(), ReadinessSchemaError> {
    Err(ReadinessSchemaError::InitializationFailed {
        check: "owned_target_identity_unsupported",
    })
}

fn validate_rollback_journal_header_bytes(header: &[u8]) -> Result<(), ReadinessSchemaError> {
    if header.len() < SQLITE_HEADER_LEN
        || &header[..SQLITE_MAGIC.len()] != SQLITE_MAGIC
        || header[SQLITE_WRITE_VERSION_OFFSET] != ROLLBACK_JOURNAL_VERSION
        || header[SQLITE_READ_VERSION_OFFSET] != ROLLBACK_JOURNAL_VERSION
    {
        return Err(ReadinessSchemaError::ValidationFailed {
            check: "database_header",
        });
    }
    Ok(())
}

fn enable_and_verify_foreign_keys(connection: &Connection) -> Result<(), ReadinessSchemaError> {
    connection
        .execute_batch("PRAGMA foreign_keys=ON;")
        .map_err(|_| ReadinessSchemaError::ConnectionSafeguardFailed {
            check: "enable_foreign_keys",
        })?;
    require_pragma(connection, "PRAGMA foreign_keys", 1, "foreign_keys")
}

fn require_pragma(
    connection: &Connection,
    query: &'static str,
    expected: i64,
    check: &'static str,
) -> Result<(), ReadinessSchemaError> {
    let actual = connection
        .query_row(query, [], |row| row.get::<_, i64>(0))
        .map_err(|_| ReadinessSchemaError::ConnectionSafeguardFailed { check })?;
    if actual != expected {
        return Err(ReadinessSchemaError::ConnectionSafeguardFailed { check });
    }
    Ok(())
}

fn validate_schema(
    connection: &Connection,
    namespace: &Namespace,
) -> Result<(), ReadinessSchemaError> {
    let actual_objects = schema_objects(connection)?;
    let expected_connection =
        Connection::open_in_memory().map_err(|_| ReadinessSchemaError::ValidationFailed {
            check: "expected_schema",
        })?;
    expected_connection
        .execute_batch(DDL)
        .map_err(|_| ReadinessSchemaError::ValidationFailed {
            check: "expected_schema",
        })?;
    let expected_objects = schema_objects(&expected_connection)?;
    if actual_objects != expected_objects {
        return Err(ReadinessSchemaError::ValidationFailed {
            check: "schema_objects",
        });
    }

    let header = connection
        .query_row(
            "SELECT schema_version,ddl_sha256,namespace_sha256 \
             FROM operational_readiness_schema WHERE singleton=1",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|_| ReadinessSchemaError::ValidationFailed {
            check: "schema_header",
        })?;
    let header_count = connection
        .query_row(
            "SELECT count(*) FROM operational_readiness_schema",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|_| ReadinessSchemaError::ValidationFailed {
            check: "schema_header",
        })?;
    let expected_header = (SCHEMA_VERSION, ddl_sha256(), namespace_sha256(namespace));
    if header_count != 1 || header != Some(expected_header) {
        return Err(ReadinessSchemaError::ValidationFailed {
            check: "schema_header",
        });
    }

    let foreign_key_violation = connection
        .query_row("PRAGMA foreign_key_check", [], |_| Ok(()))
        .optional()
        .map_err(|_| ReadinessSchemaError::ValidationFailed {
            check: "foreign_key_check",
        })?;
    if foreign_key_violation.is_some() {
        return Err(ReadinessSchemaError::ValidationFailed {
            check: "foreign_key_check",
        });
    }
    Ok(())
}

type SchemaObject = (String, String, String, Option<String>);

fn schema_objects(connection: &Connection) -> Result<Vec<SchemaObject>, ReadinessSchemaError> {
    let mut statement = connection
        .prepare(
            "SELECT type,name,tbl_name,sql FROM sqlite_master \
             WHERE substr(name,1,7)<>'sqlite_' ORDER BY type,name",
        )
        .map_err(|_| ReadinessSchemaError::ValidationFailed {
            check: "schema_objects",
        })?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .map_err(|_| ReadinessSchemaError::ValidationFailed {
            check: "schema_objects",
        })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| ReadinessSchemaError::ValidationFailed {
            check: "schema_objects",
        })
}

fn ddl_sha256() -> String {
    hex::encode(Sha256::digest(DDL.as_bytes()))
}

fn namespace_sha256(namespace: &Namespace) -> String {
    canonical_digest(
        NAMESPACE_DOMAIN,
        &BTreeMap::from([("namespace", namespace_value(namespace))]),
    )
    .as_str()
    .to_owned()
}
