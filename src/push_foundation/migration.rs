use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use rusqlite::{Connection, OpenFlags};
use sha2::{Digest, Sha256};

use crate::monitor::push_job::Sha256Digest;

const DDL_BYTES: &[u8] = include_bytes!("../../docs/push-system/push-system-foundation.v1.sql");
const DDL_SHA256: &str = "4bac8e58caa2f5d2362137b5e96dd087649044f45484a1284dbd7e1fd7baa953";
const SCHEMA_VERSION: u32 = 1;
const SCHEMA_DESCRIPTION: &str = "push-foundation-v1";
const SCHEMA_SIGNATURE: &str = "dd5f49a1f4e02ee1d585793cc2eff9c8b98b087b2ffd267f40c873c83960ecdd";
const MANAGED_OBJECT_COUNT: usize = 25;
const SQLITE3_PATH: &str = "/usr/bin/sqlite3";

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum FoundationMigrationError {
    #[error("bundled push-foundation digest metadata is invalid")]
    InvalidBundledDigest,
    #[error("bundled push-foundation DDL digest mismatch")]
    ScriptDigestMismatch {
        expected: Sha256Digest,
        actual: Sha256Digest,
    },
    #[error("business database path must be absolute")]
    DatabasePathNotAbsolute,
    #[error("business database parent directory does not exist")]
    DatabaseParentMissing,
    #[error("business database parent directory metadata is unreadable")]
    DatabaseParentUnreadable,
    #[error("business database parent directory must not be a symbolic link")]
    DatabaseParentSymlink,
    #[error("business database parent is not a directory")]
    DatabaseParentNotDirectory,
    #[error("business database target must not be a symbolic link")]
    DatabaseTargetSymlink,
    #[error("business database target is not a regular file")]
    DatabaseTargetNotRegular,
    #[error("business database target metadata is unreadable")]
    DatabaseTargetUnreadable,
    #[error("fixed SQLite CLI is unavailable")]
    SqliteCliUnavailable,
    #[error("failed to write exact DDL bytes to SQLite CLI")]
    ScriptWriteFailed,
    #[error("failed to wait for SQLite CLI")]
    SqliteCliWaitFailed,
    #[error("push-foundation migration was rejected")]
    MigrationRejected {
        exit_code: Option<i32>,
        stderr_sha256: Sha256Digest,
    },
    #[error("cannot open migrated database for read-only attestation")]
    AttestationOpenFailed,
    #[error("push-foundation post-migration attestation failed: {check}")]
    AttestationFailed { check: &'static str },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FoundationSchemaMigration {
    ddl_sha256: Sha256Digest,
}

impl FoundationSchemaMigration {
    pub fn bundled() -> Result<Self, FoundationMigrationError> {
        let expected = parse_digest(DDL_SHA256)?;
        let actual = digest(DDL_BYTES)?;
        if actual != expected {
            return Err(FoundationMigrationError::ScriptDigestMismatch { expected, actual });
        }
        Ok(Self { ddl_sha256: actual })
    }

    pub fn schema_version(&self) -> u32 {
        SCHEMA_VERSION
    }

    pub fn schema_description(&self) -> &'static str {
        SCHEMA_DESCRIPTION
    }

    pub fn schema_signature(&self) -> &'static str {
        SCHEMA_SIGNATURE
    }

    pub fn ddl_sha256(&self) -> &Sha256Digest {
        &self.ddl_sha256
    }

    pub fn managed_object_count(&self) -> usize {
        MANAGED_OBJECT_COUNT
    }

    #[cfg(test)]
    pub(super) fn ddl_bytes(&self) -> &'static [u8] {
        DDL_BYTES
    }

    pub fn apply_to(&self, database: &Path) -> Result<MigrationReceipt, FoundationMigrationError> {
        validate_database_path(database)?;

        let mut child = Command::new(SQLITE3_PATH)
            .arg(database)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|_| FoundationMigrationError::SqliteCliUnavailable)?;

        let write_result = child
            .stdin
            .take()
            .ok_or(FoundationMigrationError::ScriptWriteFailed)
            .and_then(|mut stdin| {
                stdin
                    .write_all(DDL_BYTES)
                    .map_err(|_| FoundationMigrationError::ScriptWriteFailed)
            });
        let output = child
            .wait_with_output()
            .map_err(|_| FoundationMigrationError::SqliteCliWaitFailed)?;
        write_result?;

        if !output.status.success() {
            return Err(FoundationMigrationError::MigrationRejected {
                exit_code: output.status.code(),
                stderr_sha256: digest(&output.stderr)?,
            });
        }

        attest(database, &self.ddl_sha256)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MigrationReceipt {
    schema_version: u32,
    schema_signature: &'static str,
    ddl_sha256: Sha256Digest,
    managed_object_count: usize,
}

impl MigrationReceipt {
    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    pub fn schema_signature(&self) -> &str {
        self.schema_signature
    }

    pub fn ddl_sha256(&self) -> &Sha256Digest {
        &self.ddl_sha256
    }

    pub fn managed_object_count(&self) -> usize {
        self.managed_object_count
    }
}

fn validate_database_path(database: &Path) -> Result<(), FoundationMigrationError> {
    if !database.is_absolute() {
        return Err(FoundationMigrationError::DatabasePathNotAbsolute);
    }
    if database.file_name().is_none() {
        return Err(FoundationMigrationError::DatabaseTargetNotRegular);
    }

    let parent = database
        .parent()
        .ok_or(FoundationMigrationError::DatabaseParentMissing)?;
    let parent_metadata = fs::symlink_metadata(parent).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => FoundationMigrationError::DatabaseParentMissing,
        _ => FoundationMigrationError::DatabaseParentUnreadable,
    })?;
    if parent_metadata.file_type().is_symlink() {
        return Err(FoundationMigrationError::DatabaseParentSymlink);
    }
    if !parent_metadata.is_dir() {
        return Err(FoundationMigrationError::DatabaseParentNotDirectory);
    }

    match fs::symlink_metadata(database) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(FoundationMigrationError::DatabaseTargetSymlink)
        }
        Ok(metadata) if !metadata.is_file() => {
            Err(FoundationMigrationError::DatabaseTargetNotRegular)
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(FoundationMigrationError::DatabaseTargetUnreadable),
    }
}

fn attest(
    database: &Path,
    ddl_sha256: &Sha256Digest,
) -> Result<MigrationReceipt, FoundationMigrationError> {
    let connection = Connection::open_with_flags(
        database,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| FoundationMigrationError::AttestationOpenFailed)?;
    connection
        .execute_batch("PRAGMA query_only=ON;")
        .map_err(|_| FoundationMigrationError::AttestationFailed {
            check: "query_only",
        })?;
    if query_count(&connection, "PRAGMA query_only", "query_only")? != 1 {
        return Err(FoundationMigrationError::AttestationFailed {
            check: "query_only",
        });
    }

    let header_count = query_count(
        &connection,
        "SELECT count(*) FROM push_foundation_schema WHERE version=1 AND description='push-foundation-v1' AND schema_signature='dd5f49a1f4e02ee1d585793cc2eff9c8b98b087b2ffd267f40c873c83960ecdd'",
        "schema_header",
    )?;
    let total_header_count = query_count(
        &connection,
        "SELECT count(*) FROM push_foundation_schema",
        "schema_header_count",
    )?;
    if header_count != 1 || total_header_count != 1 {
        return Err(FoundationMigrationError::AttestationFailed {
            check: "schema_header",
        });
    }

    let registry_count = query_count(
        &connection,
        "SELECT count(*) FROM push_foundation_objects",
        "managed_object_count",
    )?;
    if registry_count != MANAGED_OBJECT_COUNT as i64 {
        return Err(FoundationMigrationError::AttestationFailed {
            check: "managed_object_count",
        });
    }

    let matching_objects = query_count(
        &connection,
        "SELECT count(*) FROM push_foundation_objects r JOIN sqlite_master s ON s.name=r.name AND s.type=r.object_type AND CAST(s.sql AS BLOB)=CAST(r.definition AS BLOB)",
        "managed_object_definitions",
    )?;
    if matching_objects != MANAGED_OBJECT_COUNT as i64 {
        return Err(FoundationMigrationError::AttestationFailed {
            check: "managed_object_definitions",
        });
    }

    let unregistered_attached = query_count(
        &connection,
        "SELECT count(*) FROM sqlite_master s WHERE s.sql IS NOT NULL AND s.type IN ('index','trigger') AND s.name NOT LIKE 'sqlite_autoindex_%' AND s.tbl_name IN (SELECT name FROM push_foundation_objects WHERE object_type='table') AND NOT EXISTS(SELECT 1 FROM push_foundation_objects r WHERE r.name=s.name AND r.object_type=s.type)",
        "unregistered_attached_objects",
    )?;
    if unregistered_attached != 0 {
        return Err(FoundationMigrationError::AttestationFailed {
            check: "unregistered_attached_objects",
        });
    }

    Ok(MigrationReceipt {
        schema_version: SCHEMA_VERSION,
        schema_signature: SCHEMA_SIGNATURE,
        ddl_sha256: ddl_sha256.clone(),
        managed_object_count: MANAGED_OBJECT_COUNT,
    })
}

fn query_count(
    connection: &Connection,
    sql: &'static str,
    check: &'static str,
) -> Result<i64, FoundationMigrationError> {
    connection
        .query_row(sql, [], |row| row.get(0))
        .map_err(|_| FoundationMigrationError::AttestationFailed { check })
}

fn digest(bytes: &[u8]) -> Result<Sha256Digest, FoundationMigrationError> {
    parse_digest(&hex::encode(Sha256::digest(bytes)))
}

fn parse_digest(value: &str) -> Result<Sha256Digest, FoundationMigrationError> {
    Sha256Digest::parse("push_foundation_ddl_sha256", value)
        .map_err(|_| FoundationMigrationError::InvalidBundledDigest)
}
