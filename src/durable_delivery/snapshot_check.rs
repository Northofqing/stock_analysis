//! Logical checks of a caller-supplied isolated snapshot. No coordinator,
//! migration, delivery capability or production approval is created here.
use super::model::{DurableDeliveryError, Result};
use rusqlite::{Connection, DatabaseName, TransactionBehavior};
use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Schema14ExtensionObservation {
    pub schema_version: i64,
    pub extension_catalog_sha256: String,
    pub extension_row_counts: Vec<(String, i64)>,
    pub scope: &'static str,
}

fn invalid(message: &str) -> DurableDeliveryError {
    DurableDeliveryError::InvalidConfiguration(message.to_owned())
}

/// Check the exact schema12/13/14 catalogs and stored G5b/P05 evidence with
/// the same validators used by the coordinator. The connection must be read
/// only, unattached, and outside a caller transaction. Filesystem stability
/// remains the caller's responsibility; this observation is not an authority.
pub fn inspect_schema14_extensions(
    connection: &mut Connection,
) -> Result<Schema14ExtensionObservation> {
    if super::schema::SCHEMA_VERSION != 14 {
        return Err(invalid(
            "snapshot checker requires an explicit schema14 contract",
        ));
    }
    if !connection.is_readonly(DatabaseName::Main)? || !connection.is_autocommit() {
        return Err(invalid(
            "snapshot checker requires an idle read-only connection",
        ));
    }
    let attached: i64 = connection.query_row(
        "SELECT COUNT(*) FROM pragma_database_list WHERE name NOT IN ('main','temp')",
        [],
        |row| row.get(0),
    )?;
    let temporary: i64 =
        connection.query_row("SELECT COUNT(*) FROM temp.sqlite_master", [], |row| {
            row.get(0)
        })?;
    if attached != 0 || temporary != 0 {
        return Err(invalid(
            "snapshot checker rejects attachments and temporary objects",
        ));
    }
    super::schema::register_sha256_function(connection)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
    super::coordinator::require_current_schema_version(&transaction)?;
    let integrity: String = transaction.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    let foreign_key_failures: i64 =
        transaction.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })?;
    if integrity != "ok" || foreign_key_failures != 0 {
        return Err(invalid("snapshot integrity or foreign key check failed"));
    }
    let mut row_counts = Vec::new();
    for table in super::schema_g5b_cohort::TABLES
        .into_iter()
        .chain(super::schema_p05_unit::TABLES)
        .chain(super::schema_p05_unit_runtime::TABLES)
    {
        let count =
            transaction.query_row(&format!("SELECT COUNT(*) FROM main.{table}"), [], |row| {
                row.get(0)
            })?;
        row_counts.push((table.to_owned(), count));
    }
    let mut catalog_hash = Sha256::new();
    let mut statement = transaction.prepare(
        "SELECT type,name,tbl_name,sql FROM main.sqlite_master
         WHERE sql IS NOT NULL AND (lower(name) GLOB 'g5b_*'
         OR lower(tbl_name) GLOB 'g5b_*' OR lower(name) GLOB 'p05_*'
         OR lower(tbl_name) GLOB 'p05_*') ORDER BY type,name,tbl_name",
    )?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        for column in 0..4 {
            let value: String = row.get(column)?;
            catalog_hash.update((value.len() as u64).to_be_bytes());
            catalog_hash.update(value.as_bytes());
        }
    }
    drop(rows);
    drop(statement);
    transaction.rollback()?;
    Ok(Schema14ExtensionObservation {
        schema_version: 14,
        extension_catalog_sha256: hex::encode(catalog_hash.finalize()),
        extension_row_counts: row_counts,
        scope: "IsolatedSnapshotObservation: schema12/13/14 catalogs, G5b/P05 persisted content and joins, quick_check and foreign keys. No full BR-194 external-audit join, migration qualification, Financial qualification, runtime authority or production approval.",
    })
}
