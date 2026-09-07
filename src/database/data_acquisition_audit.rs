//! BR-159 immutable market-data acquisition audit and provider-state evidence.

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Integer, Nullable, Text};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::DatabaseManager;

const AUDIT_SCHEMA_VERSION: i32 = 1;
const AUDIT_CHAIN_GENESIS: &str = "BR159_DATA_ACQUISITION_AUDIT_GENESIS_V1";

#[derive(Debug, Clone)]
pub struct DataAcquisitionAuditRecord<'a> {
    pub capability: &'a str,
    pub provider: &'a str,
    pub source: &'a str,
    pub request_hash: &'a str,
    pub source_at: Option<&'a str>,
    pub observed_at: &'a str,
    pub batch_id: Option<&'a str>,
    pub outcome: &'a str,
    pub request_count: i64,
    pub accepted_count: i64,
    pub rejected_count: i64,
    pub reason_code: &'a str,
    pub retryable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataAcquisitionAuditReceipt {
    pub audit_id: i64,
    pub record_hash: String,
    pub previous_outcome: Option<String>,
    pub current_outcome: String,
}

impl DataAcquisitionAuditReceipt {
    pub fn provider_state_changed(&self) -> bool {
        self.previous_outcome
            .as_deref()
            .is_some_and(|previous| previous != self.current_outcome)
    }
}

/// Complete BR-159 facts validated from the connection's snapshot, not a W15 attestation.
///
/// The caller still has to authenticate the connection's source and bind its
/// namespace, business date, source contract/version and readiness scope. This
/// does not certify the commit state of an enclosing caller-owned transaction.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct VerifiedAcquisitionAudit {
    receipt: DataAcquisitionAuditReceipt,
    audit: PersistedAcquisitionAudit,
}

impl std::fmt::Debug for VerifiedAcquisitionAudit {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VerifiedAcquisitionAudit")
            .field("audit_id", &self.receipt.audit_id)
            .field("record_hash", &self.receipt.record_hash)
            .finish_non_exhaustive()
    }
}

#[cfg_attr(not(test), allow(dead_code))]
impl VerifiedAcquisitionAudit {
    pub(crate) fn receipt(&self) -> &DataAcquisitionAuditReceipt {
        &self.receipt
    }

    /// Explicit access to protected raw fields; do not log or publish this view.
    /// Outcomes and timestamps retain BR-159 semantics, not W15 interpretations.
    pub(crate) fn record(&self) -> DataAcquisitionAuditRecord<'_> {
        self.audit.record()
    }
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("acquisition audit facts could not be verified")]
pub(crate) struct AcquisitionAuditReadError;

/// Read both audit tables in one DEFERRED transaction and reuse the existing
/// complete-chain/receipt verifier. No database opening, initialization,
/// migration, checkpoint, or observation interpretation occurs here.
///
/// A source-owned, side-effect-safe opener is a separate prerequisite. This
/// function cannot certify the identity or configuration of an arbitrary connection.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn read_verified_acquisition_audit(
    connection: &mut SqliteConnection,
    receipt: &DataAcquisitionAuditReceipt,
) -> Result<VerifiedAcquisitionAudit, AcquisitionAuditReadError> {
    connection
        .transaction::<_, diesel::result::Error, _>(|connection| {
            let audits = load_audit_rows(connection)?;
            let chain = load_chain_rows(connection)?;
            let audit = audits
                .iter()
                .find(|audit| audit.id == receipt.audit_id)
                .ok_or_else(|| audit_error("acquisition audit record missing"))?;
            verify_data_acquisition_receipt_snapshot(&audits, &chain, receipt, &audit.record())?;
            Ok(VerifiedAcquisitionAudit {
                receipt: receipt.clone(),
                audit: audit.clone(),
            })
        })
        .map_err(|_| AcquisitionAuditReadError)
}

#[derive(Clone, Debug, QueryableByName, Serialize)]
struct PersistedAcquisitionAudit {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = Integer)]
    schema_version: i32,
    #[diesel(sql_type = Text)]
    capability: String,
    #[diesel(sql_type = Text)]
    provider: String,
    #[diesel(sql_type = Text)]
    source: String,
    #[diesel(sql_type = Text)]
    request_hash: String,
    #[diesel(sql_type = Nullable<Text>)]
    source_at: Option<String>,
    #[diesel(sql_type = Text)]
    observed_at: String,
    #[diesel(sql_type = Nullable<Text>)]
    batch_id: Option<String>,
    #[diesel(sql_type = Text)]
    outcome: String,
    #[diesel(sql_type = BigInt)]
    request_count: i64,
    #[diesel(sql_type = BigInt)]
    accepted_count: i64,
    #[diesel(sql_type = BigInt)]
    rejected_count: i64,
    #[diesel(sql_type = Text)]
    reason_code: String,
    #[diesel(sql_type = Integer)]
    retryable: i32,
    #[diesel(sql_type = Text)]
    created_at: String,
}

impl PersistedAcquisitionAudit {
    fn record(&self) -> DataAcquisitionAuditRecord<'_> {
        DataAcquisitionAuditRecord {
            capability: &self.capability,
            provider: &self.provider,
            source: &self.source,
            request_hash: &self.request_hash,
            source_at: self.source_at.as_deref(),
            observed_at: &self.observed_at,
            batch_id: self.batch_id.as_deref(),
            outcome: &self.outcome,
            request_count: self.request_count,
            accepted_count: self.accepted_count,
            rejected_count: self.rejected_count,
            reason_code: &self.reason_code,
            retryable: self.retryable != 0,
        }
    }
}

#[derive(Debug, QueryableByName)]
struct AuditChainRow {
    #[diesel(sql_type = BigInt)]
    acquisition_audit_id: i64,
    #[diesel(sql_type = Text)]
    previous_hash: String,
    #[diesel(sql_type = Text)]
    record_hash: String,
}

#[derive(Debug, QueryableByName)]
struct PreviousOutcomeRow {
    #[diesel(sql_type = Text)]
    outcome: String,
}

fn audit_error(message: impl Into<String>) -> diesel::result::Error {
    diesel::result::Error::QueryBuilderError(Box::new(std::io::Error::other(message.into())))
}

fn validate_record(record: &DataAcquisitionAuditRecord<'_>) -> Result<(), String> {
    for (field, value) in [
        ("capability", record.capability),
        ("provider", record.provider),
        ("source", record.source),
        ("observed_at", record.observed_at),
        ("outcome", record.outcome),
        ("reason_code", record.reason_code),
    ] {
        if value.trim().is_empty() {
            return Err(format!(
                "BR-159 acquisition audit {field} must not be blank"
            ));
        }
    }
    if record.request_hash.len() != 64
        || !record
            .request_hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(
            "BR-159 acquisition audit request_hash must be 64 lowercase hex characters".to_string(),
        );
    }
    if !matches!(
        record.outcome,
        "available"
            | "verified_empty"
            | "invalid_request"
            | "unavailable"
            | "stale"
            | "partial"
            | "conflict"
            | "unsupported"
    ) {
        return Err(format!(
            "BR-159 acquisition audit has unknown outcome {}",
            record.outcome
        ));
    }
    if record.request_count < 0 || record.accepted_count < 0 || record.rejected_count < 0 {
        return Err("BR-159 acquisition audit counters must be non-negative".to_string());
    }
    if matches!(record.outcome, "available" | "verified_empty")
        && record
            .batch_id
            .is_none_or(|batch_id| batch_id.trim().is_empty())
    {
        return Err(format!(
            "BR-159 {} acquisition audit must retain a provider batch_id",
            record.outcome
        ));
    }
    Ok(())
}

fn load_audit_rows(
    conn: &mut SqliteConnection,
) -> diesel::QueryResult<Vec<PersistedAcquisitionAudit>> {
    diesel::sql_query(
        "SELECT id, schema_version, capability, provider, source, request_hash,
                source_at, observed_at, batch_id, outcome, request_count,
                accepted_count, rejected_count, reason_code, retryable, created_at
         FROM data_acquisition_audit ORDER BY id ASC",
    )
    .load(conn)
}

fn load_chain_rows(conn: &mut SqliteConnection) -> diesel::QueryResult<Vec<AuditChainRow>> {
    diesel::sql_query(
        "SELECT acquisition_audit_id, previous_hash, record_hash
         FROM data_acquisition_audit_chain ORDER BY acquisition_audit_id ASC",
    )
    .load(conn)
}

fn load_audit_tail(
    conn: &mut SqliteConnection,
) -> diesel::QueryResult<Option<PersistedAcquisitionAudit>> {
    diesel::sql_query(
        "SELECT id, schema_version, capability, provider, source, request_hash,
                source_at, observed_at, batch_id, outcome, request_count,
                accepted_count, rejected_count, reason_code, retryable, created_at
         FROM data_acquisition_audit ORDER BY id DESC LIMIT 1",
    )
    .get_result(conn)
    .optional()
}

fn load_chain_tail(conn: &mut SqliteConnection) -> diesel::QueryResult<Option<AuditChainRow>> {
    diesel::sql_query(
        "SELECT acquisition_audit_id, previous_hash, record_hash
         FROM data_acquisition_audit_chain ORDER BY acquisition_audit_id DESC LIMIT 1",
    )
    .get_result(conn)
    .optional()
}

fn calculate_record_hash(
    previous_hash: &str,
    record: &PersistedAcquisitionAudit,
) -> diesel::QueryResult<String> {
    let payload = serde_json::to_vec(record)
        .map_err(|error| audit_error(format!("BR-159 serialize acquisition audit row: {error}")))?;
    let mut hasher = Sha256::new();
    hasher.update(b"BR159_DATA_ACQUISITION_AUDIT_V1\0");
    hasher.update(previous_hash.as_bytes());
    hasher.update(b"\0");
    hasher.update(payload);
    Ok(hex::encode(hasher.finalize()))
}

fn validate_data_acquisition_audit_chain_rows(
    audits: &[PersistedAcquisitionAudit],
    chain: &[AuditChainRow],
) -> diesel::QueryResult<String> {
    if audits.len() != chain.len() {
        return Err(audit_error(format!(
            "BR-159 acquisition audit hash chain length mismatch: audit_rows={}, chain_rows={}",
            audits.len(),
            chain.len()
        )));
    }

    let mut previous = AUDIT_CHAIN_GENESIS.to_string();
    for (audit, evidence) in audits.iter().zip(chain.iter()) {
        if audit.schema_version != AUDIT_SCHEMA_VERSION
            || evidence.acquisition_audit_id != audit.id
            || evidence.previous_hash != previous
        {
            return Err(audit_error(format!(
                "BR-159 acquisition audit linkage/schema mismatch at audit id {}",
                audit.id
            )));
        }
        let expected = calculate_record_hash(&previous, audit)?;
        if evidence.record_hash != expected {
            return Err(audit_error(format!(
                "BR-159 acquisition audit hash mismatch at audit id {}",
                audit.id
            )));
        }
        previous = evidence.record_hash.clone();
    }
    Ok(previous)
}

pub(super) fn validate_data_acquisition_audit_chain(
    conn: &mut SqliteConnection,
) -> diesel::QueryResult<String> {
    // 2026-08-12: 两次 SELECT 包进同一 DEFERRED 事务, 保证同一快照 —
    // 并发写者 (如回填工具 vs 运行中 monitor) 在两次读取之间提交时,
    // 裸 SELECT 会看到 audit/chain 各一瞬, 误报 length mismatch。
    let (audits, chain) = conn.transaction::<_, diesel::result::Error, _>(|conn| {
        Ok((load_audit_rows(conn)?, load_chain_rows(conn)?))
    })?;
    validate_data_acquisition_audit_chain_rows(&audits, &chain)
}

fn verify_data_acquisition_receipt_snapshot(
    audits: &[PersistedAcquisitionAudit],
    chain: &[AuditChainRow],
    receipt: &DataAcquisitionAuditReceipt,
    expected: &DataAcquisitionAuditRecord<'_>,
) -> diesel::QueryResult<()> {
    validate_record(expected).map_err(audit_error)?;
    validate_data_acquisition_audit_chain_rows(audits, chain)?;
    let Some((position, audit)) = audits
        .iter()
        .enumerate()
        .find(|(_, audit)| audit.id == receipt.audit_id)
    else {
        return Err(audit_error(format!(
            "BR-159 acquisition receipt references missing audit id {}",
            receipt.audit_id
        )));
    };
    let evidence = chain
        .get(position)
        .ok_or_else(|| audit_error("BR-159 acquisition receipt has no chain row"))?;
    let previous_outcome = audits[..position]
        .iter()
        .rev()
        .find(|candidate| {
            candidate.capability == audit.capability && candidate.provider == audit.provider
        })
        .map(|candidate| candidate.outcome.as_str());
    let expected_retryable = i32::from(expected.retryable);
    let facts_match = audit.capability == expected.capability
        && audit.provider == expected.provider
        && audit.source == expected.source
        && audit.request_hash == expected.request_hash
        && audit.source_at.as_deref() == expected.source_at
        && audit.observed_at == expected.observed_at
        && audit.batch_id.as_deref() == expected.batch_id
        && audit.outcome == expected.outcome
        && audit.request_count == expected.request_count
        && audit.accepted_count == expected.accepted_count
        && audit.rejected_count == expected.rejected_count
        && audit.reason_code == expected.reason_code
        && audit.retryable == expected_retryable;
    let receipt_matches = evidence.acquisition_audit_id == receipt.audit_id
        && evidence.record_hash == receipt.record_hash
        && previous_outcome == receipt.previous_outcome.as_deref()
        && audit.outcome == receipt.current_outcome;
    if !facts_match || !receipt_matches {
        return Err(audit_error(format!(
            "BR-159 acquisition receipt does not bind the expected audit facts at id {}",
            receipt.audit_id
        )));
    }
    Ok(())
}

fn validate_data_acquisition_audit_tail(
    conn: &mut SqliteConnection,
) -> diesel::QueryResult<String> {
    match (load_audit_tail(conn)?, load_chain_tail(conn)?) {
        (None, None) => Ok(AUDIT_CHAIN_GENESIS.to_string()),
        (Some(audit), Some(chain))
            if audit.id == chain.acquisition_audit_id
                && audit.schema_version == AUDIT_SCHEMA_VERSION =>
        {
            let expected = calculate_record_hash(&chain.previous_hash, &audit)?;
            if expected != chain.record_hash {
                return Err(audit_error(format!(
                    "BR-159 acquisition audit tail hash mismatch at audit id {}",
                    audit.id
                )));
            }
            Ok(chain.record_hash)
        }
        (audit, chain) => Err(audit_error(format!(
            "BR-159 acquisition audit tail linkage mismatch: audit_id={:?} chain_id={:?}",
            audit.map(|row| row.id),
            chain.map(|row| row.acquisition_audit_id)
        ))),
    }
}

fn append_chain_row(
    conn: &mut SqliteConnection,
    previous_hash: &str,
    audit: &PersistedAcquisitionAudit,
) -> diesel::QueryResult<String> {
    let record_hash = calculate_record_hash(previous_hash, audit)?;
    let rows = diesel::sql_query(
        "INSERT INTO data_acquisition_audit_chain
         (acquisition_audit_id, previous_hash, record_hash)
         VALUES (?, ?, ?)",
    )
    .bind::<BigInt, _>(audit.id)
    .bind::<Text, _>(previous_hash)
    .bind::<Text, _>(&record_hash)
    .execute(conn)?;
    if rows != 1 {
        return Err(audit_error(format!(
            "BR-159 append acquisition audit hash chain affected {rows} rows"
        )));
    }
    Ok(record_hash)
}

pub(super) fn create_schema(conn: &mut SqliteConnection) -> diesel::QueryResult<()> {
    diesel::sql_query(
        "CREATE TABLE IF NOT EXISTS data_acquisition_audit (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            schema_version INTEGER NOT NULL CHECK(schema_version = 1),
            capability TEXT NOT NULL,
            provider TEXT NOT NULL,
            source TEXT NOT NULL,
            request_hash TEXT NOT NULL CHECK(length(request_hash) = 64),
            source_at TEXT,
            observed_at TEXT NOT NULL,
            batch_id TEXT,
            outcome TEXT NOT NULL CHECK(outcome IN (
                'available', 'verified_empty', 'invalid_request', 'unavailable', 'stale',
                'partial', 'conflict', 'unsupported'
            )),
            request_count INTEGER NOT NULL CHECK(request_count >= 0),
            accepted_count INTEGER NOT NULL CHECK(accepted_count >= 0),
            rejected_count INTEGER NOT NULL CHECK(rejected_count >= 0),
            reason_code TEXT NOT NULL,
            retryable INTEGER NOT NULL CHECK(retryable IN (0, 1)),
            created_at TEXT NOT NULL DEFAULT (
                strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
            )
        )",
    )
    .execute(conn)?;
    diesel::sql_query(
        "CREATE INDEX IF NOT EXISTS idx_data_acquisition_audit_provider
         ON data_acquisition_audit(capability, provider, id)",
    )
    .execute(conn)?;
    diesel::sql_query(
        "CREATE TRIGGER IF NOT EXISTS trg_data_acquisition_audit_no_update
         BEFORE UPDATE ON data_acquisition_audit
         BEGIN SELECT RAISE(ABORT, 'BR-159 acquisition audit is immutable'); END",
    )
    .execute(conn)?;
    diesel::sql_query(
        "CREATE TRIGGER IF NOT EXISTS trg_data_acquisition_audit_no_delete
         BEFORE DELETE ON data_acquisition_audit
         BEGIN SELECT RAISE(ABORT, 'BR-159 acquisition audit retention is at least five years'); END",
    )
    .execute(conn)?;
    diesel::sql_query(
        "CREATE TABLE IF NOT EXISTS data_acquisition_audit_chain (
            acquisition_audit_id INTEGER PRIMARY KEY NOT NULL,
            previous_hash TEXT NOT NULL,
            record_hash TEXT NOT NULL UNIQUE,
            created_at TEXT NOT NULL DEFAULT (
                strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
            ),
            FOREIGN KEY(acquisition_audit_id) REFERENCES data_acquisition_audit(id)
        )",
    )
    .execute(conn)?;
    diesel::sql_query(
        "CREATE TRIGGER IF NOT EXISTS trg_data_acquisition_audit_chain_no_update
         BEFORE UPDATE ON data_acquisition_audit_chain
         BEGIN SELECT RAISE(ABORT, 'BR-159 acquisition audit hash chain is immutable'); END",
    )
    .execute(conn)?;
    diesel::sql_query(
        "CREATE TRIGGER IF NOT EXISTS trg_data_acquisition_audit_chain_no_delete
         BEFORE DELETE ON data_acquisition_audit_chain
         BEGIN SELECT RAISE(ABORT, 'BR-159 acquisition audit hash chain retention is at least five years'); END",
    )
    .execute(conn)?;
    validate_data_acquisition_audit_chain(conn).map(|_| ())
}

fn insert_acquisition_audit_query(
    conn: &mut SqliteConnection,
    record: &DataAcquisitionAuditRecord<'_>,
) -> diesel::QueryResult<DataAcquisitionAuditReceipt> {
    validate_record(record).map_err(audit_error)?;
    // `create_schema` performs a complete chain validation at process startup.
    // Append runs inside an IMMEDIATE transaction and validates only the tail,
    // keeping the five-year audit from turning every request into an O(n) scan.
    let previous_hash = validate_data_acquisition_audit_tail(conn)?;
    let previous_outcome = diesel::sql_query(
        "SELECT outcome FROM data_acquisition_audit
         WHERE capability = ? AND provider = ? ORDER BY id DESC LIMIT 1",
    )
    .bind::<Text, _>(record.capability)
    .bind::<Text, _>(record.provider)
    .get_result::<PreviousOutcomeRow>(conn)
    .optional()?
    .map(|row| row.outcome);

    let rows = diesel::sql_query(
        "INSERT INTO data_acquisition_audit (
            schema_version, capability, provider, source, request_hash, source_at,
            observed_at, batch_id, outcome, request_count, accepted_count,
            rejected_count, reason_code, retryable
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind::<Integer, _>(AUDIT_SCHEMA_VERSION)
    .bind::<Text, _>(record.capability)
    .bind::<Text, _>(record.provider)
    .bind::<Text, _>(record.source)
    .bind::<Text, _>(record.request_hash)
    .bind::<Nullable<Text>, _>(record.source_at)
    .bind::<Text, _>(record.observed_at)
    .bind::<Nullable<Text>, _>(record.batch_id)
    .bind::<Text, _>(record.outcome)
    .bind::<BigInt, _>(record.request_count)
    .bind::<BigInt, _>(record.accepted_count)
    .bind::<BigInt, _>(record.rejected_count)
    .bind::<Text, _>(record.reason_code)
    .bind::<Integer, _>(i32::from(record.retryable))
    .execute(conn)?;
    if rows != 1 {
        return Err(audit_error(format!(
            "BR-159 insert acquisition audit affected {rows} rows"
        )));
    }
    let audit = diesel::sql_query(
        "SELECT id, schema_version, capability, provider, source, request_hash,
                source_at, observed_at, batch_id, outcome, request_count,
                accepted_count, rejected_count, reason_code, retryable, created_at
         FROM data_acquisition_audit WHERE id = last_insert_rowid()",
    )
    .get_result::<PersistedAcquisitionAudit>(conn)?;
    let record_hash = append_chain_row(conn, &previous_hash, &audit)?;
    Ok(DataAcquisitionAuditReceipt {
        audit_id: audit.id,
        record_hash,
        previous_outcome,
        current_outcome: audit.outcome,
    })
}

impl DatabaseManager {
    pub fn record_data_acquisition(
        &self,
        record: &DataAcquisitionAuditRecord<'_>,
    ) -> Result<DataAcquisitionAuditReceipt, String> {
        let mut conn = self
            .get_conn()
            .map_err(|error| format!("BR-159 acquisition audit DB connection: {error}"))?;
        conn.immediate_transaction::<_, diesel::result::Error, _>(|conn| {
            insert_acquisition_audit_query(conn, record)
        })
        .map_err(|error| format!("BR-159 acquisition audit append: {error}"))
    }

    /// Verify a remotely supplied receipt against the immutable local BR-159
    /// audit and hash-chain rows without creating or changing audit state.
    pub(crate) fn verify_data_acquisition_receipt(
        &self,
        receipt: &DataAcquisitionAuditReceipt,
        expected: &DataAcquisitionAuditRecord<'_>,
    ) -> Result<(), String> {
        let mut conn = self
            .get_conn()
            .map_err(|error| format!("BR-159 acquisition receipt DB connection: {error}"))?;
        conn.transaction::<_, diesel::result::Error, _>(|conn| {
            let audits = load_audit_rows(conn)?;
            let chain = load_chain_rows(conn)?;
            verify_data_acquisition_receipt_snapshot(&audits, &chain, receipt, expected)
        })
        .map_err(|error| format!("BR-159 acquisition receipt verification: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, QueryableByName)]
    struct CountRow {
        #[diesel(sql_type = BigInt)]
        count: i64,
    }

    fn audit_directory_bytes(
        root: &std::path::Path,
    ) -> std::collections::BTreeMap<std::ffi::OsString, Vec<u8>> {
        std::fs::read_dir(root)
            .expect("TEST_CODE directory")
            .map(|entry| {
                let entry = entry.expect("TEST_CODE file entry");
                (
                    entry.file_name(),
                    std::fs::read(entry.path()).expect("TEST_CODE bytes"),
                )
            })
            .collect()
    }

    fn connection() -> SqliteConnection {
        let mut conn = SqliteConnection::establish(":memory:").expect("in-memory sqlite");
        create_schema(&mut conn).expect("acquisition audit schema");
        conn
    }

    fn record<'a>(outcome: &'a str, batch_id: Option<&'a str>) -> DataAcquisitionAuditRecord<'a> {
        DataAcquisitionAuditRecord {
            capability: "TEST_CODE_A01",
            provider: "TEST_CODE_TDX",
            source: "TEST_CODE_tdx-smart",
            request_hash: "a2b2c2d2e2f2a2b2c2d2e2f2a2b2c2d2e2f2a2b2c2d2e2f2a2b2c2d2e2f2a2b2",
            source_at: Some("2099-01-02"),
            observed_at: "2099-01-02T10:00:00+08:00",
            batch_id,
            outcome,
            request_count: 1,
            accepted_count: i64::from(outcome == "available"),
            rejected_count: 0,
            reason_code: "TEST_CODE_ok",
            retryable: false,
        }
    }

    #[test]
    fn w15_acquisition_reader_reopens_verified_empty_facts_without_writes() {
        let root = tempfile::tempdir().expect("TEST_CODE audit root");
        let database = root
            .path()
            .canonicalize()
            .expect("TEST_CODE canonical root")
            .join("acquisition.sqlite3");
        let path = database.to_str().expect("TEST_CODE UTF-8 fixture path");
        let mut expected = record("verified_empty", Some("TEST_CODE_SECRET_batch"));
        expected.source = "TEST_CODE_SECRET_source";
        let receipt = {
            let mut writer = SqliteConnection::establish(path).expect("TEST_CODE fixture writer");
            create_schema(&mut writer).expect("TEST_CODE fixture schema");
            writer
                .immediate_transaction::<_, diesel::result::Error, _>(|conn| {
                    insert_acquisition_audit_query(conn, &expected)
                })
                .expect("TEST_CODE append fixture")
        };
        let before = audit_directory_bytes(root.path());
        let verified = {
            let mut reader = SqliteConnection::establish(&format!("file:{path}?mode=ro"))
                .expect("TEST_CODE explicit readonly connection");
            diesel::sql_query("PRAGMA query_only=ON")
                .execute(&mut reader)
                .expect("TEST_CODE query-only connection");
            read_verified_acquisition_audit(&mut reader, &receipt)
                .expect("TEST_CODE query original audited facts")
        };
        assert_eq!(verified.receipt(), &receipt);
        let actual = verified.record();
        assert_eq!(actual.capability, expected.capability);
        assert_eq!(actual.provider, expected.provider);
        assert_eq!(actual.source, expected.source);
        assert_eq!(actual.request_hash, expected.request_hash);
        assert_eq!(actual.source_at, expected.source_at);
        assert_eq!(actual.observed_at, expected.observed_at);
        assert_eq!(actual.batch_id, expected.batch_id);
        assert_eq!(actual.outcome, "verified_empty");
        assert_eq!(actual.request_count, 1);
        assert_eq!(actual.accepted_count, 0);
        assert_eq!(actual.rejected_count, 0);
        assert_eq!(actual.reason_code, expected.reason_code);
        assert!(!actual.retryable);
        assert!(!format!("{verified:?}").contains("TEST_CODE_SECRET"));
        assert_eq!(audit_directory_bytes(root.path()), before);
    }

    #[test]
    fn w15_acquisition_reader_preserves_all_raw_outcomes_and_exact_receipts() {
        let mut conn = connection();
        for outcome in [
            "available",
            "verified_empty",
            "invalid_request",
            "unavailable",
            "stale",
            "partial",
            "conflict",
            "unsupported",
        ] {
            let has_batch = matches!(outcome, "available" | "verified_empty");
            let mut expected = record(outcome, has_batch.then_some("TEST_CODE_verified_batch"));
            expected.retryable = !has_batch;
            let receipt = conn
                .immediate_transaction::<_, diesel::result::Error, _>(|conn| {
                    insert_acquisition_audit_query(conn, &expected)
                })
                .expect("TEST_CODE append outcome");
            let verified = read_verified_acquisition_audit(&mut conn, &receipt)
                .expect("TEST_CODE preserve original outcome");
            let actual = verified.record();
            assert_eq!(verified.receipt(), &receipt);
            assert_eq!(actual.outcome, outcome);
            assert_eq!(actual.batch_id, expected.batch_id);
            assert_eq!(actual.accepted_count, i64::from(outcome == "available"));
            assert_eq!(actual.retryable, !has_batch);
        }
    }

    #[test]
    fn w15_acquisition_reader_rejects_every_receipt_field_drift_with_closed_errors() {
        let mut conn = connection();
        let first = conn
            .immediate_transaction::<_, diesel::result::Error, _>(|conn| {
                insert_acquisition_audit_query(conn, &record("available", Some("TEST_CODE_first")))
            })
            .expect("TEST_CODE first receipt");
        let receipt = conn
            .immediate_transaction::<_, diesel::result::Error, _>(|conn| {
                insert_acquisition_audit_query(conn, &record("unavailable", None))
            })
            .expect("TEST_CODE second receipt");
        for field in ["id", "hash", "previous", "current"] {
            let mut altered = receipt.clone();
            match field {
                "id" => altered.audit_id += 100,
                "hash" => altered.record_hash = "a".repeat(64),
                "previous" => altered.previous_outcome = Some("TEST_CODE_SECRET_previous".into()),
                "current" => altered.current_outcome = "TEST_CODE_SECRET_current".into(),
                _ => unreachable!("TEST_CODE fixed cases"),
            }
            let error = read_verified_acquisition_audit(&mut conn, &altered)
                .expect_err("TEST_CODE receipt drift cannot certify facts");
            assert_eq!(error, AcquisitionAuditReadError);
            assert_eq!(
                error.to_string(),
                "acquisition audit facts could not be verified"
            );
            assert!(!format!("{error:?}").contains("TEST_CODE_SECRET"));
        }
        assert_eq!(
            read_verified_acquisition_audit(&mut conn, &receipt)
                .expect("TEST_CODE current receipt remains valid")
                .receipt(),
            &receipt
        );
        assert_eq!(
            read_verified_acquisition_audit(&mut conn, &first)
                .expect("TEST_CODE historical receipt remains valid")
                .record()
                .outcome,
            "available"
        );
        let mut missing_schema =
            SqliteConnection::establish(":memory:").expect("TEST_CODE empty db");
        diesel::sql_query("PRAGMA query_only=ON")
            .execute(&mut missing_schema)
            .expect("TEST_CODE query only");
        assert_eq!(
            read_verified_acquisition_audit(&mut missing_schema, &receipt)
                .expect_err("TEST_CODE cannot initialize missing schema"),
            AcquisitionAuditReadError
        );
    }

    #[test]
    fn w15_acquisition_reader_rejects_middle_audit_or_chain_damage_without_repair() {
        #[derive(QueryableByName)]
        struct TriggerDefinition {
            #[diesel(sql_type = Text)]
            sql: String,
        }
        for damage in ["audit", "chain"] {
            let root = tempfile::tempdir().expect("TEST_CODE corruption root");
            let database = root
                .path()
                .canonicalize()
                .expect("TEST_CODE canonical root")
                .join("acquisition.sqlite3");
            let path = database.to_str().expect("TEST_CODE fixture path");
            let receipts = {
                let mut conn = SqliteConnection::establish(path).expect("TEST_CODE fixture writer");
                create_schema(&mut conn).expect("TEST_CODE schema");
                let mut receipts = Vec::new();
                for outcome in ["available", "unavailable", "verified_empty"] {
                    receipts.push(
                        conn.immediate_transaction::<_, diesel::result::Error, _>(|conn| {
                            insert_acquisition_audit_query(
                                conn,
                                &record(outcome, Some("TEST_CODE_batch")),
                            )
                        })
                        .expect("TEST_CODE append history"),
                    );
                }
                let (trigger_name, drop_sql, tamper_sql) = if damage == "audit" {
                    ("trg_data_acquisition_audit_no_update",
                     "DROP TRIGGER trg_data_acquisition_audit_no_update",
                     "UPDATE data_acquisition_audit SET source='TEST_CODE_SECRET_damage' WHERE id=?")
                } else {
                    ("trg_data_acquisition_audit_chain_no_update",
                     "DROP TRIGGER trg_data_acquisition_audit_chain_no_update",
                     "UPDATE data_acquisition_audit_chain SET previous_hash='TEST_CODE_SECRET_damage' WHERE acquisition_audit_id=?")
                };
                let original = diesel::sql_query(
                    "SELECT sql FROM sqlite_master WHERE type='trigger' AND name=?",
                )
                .bind::<Text, _>(trigger_name)
                .get_result::<TriggerDefinition>(&mut conn)
                .expect("TEST_CODE save immutable trigger");
                diesel::sql_query(drop_sql)
                    .execute(&mut conn)
                    .expect("TEST_CODE drop selected trigger");
                assert_eq!(
                    diesel::sql_query(tamper_sql)
                        .bind::<BigInt, _>(receipts[1].audit_id)
                        .execute(&mut conn)
                        .expect("TEST_CODE damage middle history"),
                    1
                );
                diesel::sql_query(original.sql)
                    .execute(&mut conn)
                    .expect("TEST_CODE restore original trigger");
                receipts
            };
            let damaged = audit_directory_bytes(root.path());
            {
                let mut conn = SqliteConnection::establish(&format!("file:{path}?mode=ro"))
                    .expect("TEST_CODE readonly connection");
                diesel::sql_query("PRAGMA query_only=ON")
                    .execute(&mut conn)
                    .expect("TEST_CODE query only");
                for receipt in receipts {
                    assert_eq!(
                        read_verified_acquisition_audit(&mut conn, &receipt).expect_err(
                            "TEST_CODE complete chain damage rejects even old receipts"
                        ),
                        AcquisitionAuditReadError
                    );
                }
            }
            assert_eq!(audit_directory_bytes(root.path()), damaged);
        }
    }

    #[test]
    fn br159_append_is_atomic_and_provider_transitions_are_explicit() {
        let mut conn = connection();
        let first = conn
            .immediate_transaction::<_, diesel::result::Error, _>(|conn| {
                insert_acquisition_audit_query(
                    conn,
                    &record("available", Some("TEST_CODE_batch_1")),
                )
            })
            .expect("first append");
        assert!(!first.provider_state_changed());
        let second = conn
            .immediate_transaction::<_, diesel::result::Error, _>(|conn| {
                insert_acquisition_audit_query(conn, &record("unavailable", None))
            })
            .expect("second append");
        assert!(second.provider_state_changed());
        assert_eq!(second.previous_outcome.as_deref(), Some("available"));
        assert_eq!(second.current_outcome, "unavailable");
        validate_data_acquisition_audit_chain(&mut conn).expect("valid chain");
    }

    #[test]
    fn br159_invalid_success_without_batch_id_writes_nothing() {
        let mut conn = connection();
        conn.immediate_transaction::<_, diesel::result::Error, _>(|conn| {
            insert_acquisition_audit_query(conn, &record("available", None))
        })
        .expect_err("missing success batch ID");
        for table in ["data_acquisition_audit", "data_acquisition_audit_chain"] {
            let row = diesel::sql_query(format!("SELECT COUNT(*) AS count FROM {table}"))
                .get_result::<CountRow>(&mut conn)
                .expect("count rows");
            assert_eq!(row.count, 0);
        }
    }

    #[test]
    fn br159_hash_chain_detects_tampering() {
        let mut conn = connection();
        conn.immediate_transaction::<_, diesel::result::Error, _>(|conn| {
            insert_acquisition_audit_query(conn, &record("available", Some("TEST_CODE_batch_1")))
        })
        .expect("append");
        diesel::sql_query("DROP TRIGGER trg_data_acquisition_audit_chain_no_update")
            .execute(&mut conn)
            .expect("test-only tamper setup");
        diesel::sql_query(
            "UPDATE data_acquisition_audit_chain
             SET record_hash = 'TEST_CODE_tampered'",
        )
        .execute(&mut conn)
        .expect("test-only tamper");
        validate_data_acquisition_audit_chain(&mut conn).expect_err("tamper must fail");
    }

    #[test]
    fn br159_tampered_tail_blocks_the_next_append() {
        let mut conn = connection();
        conn.immediate_transaction::<_, diesel::result::Error, _>(|conn| {
            insert_acquisition_audit_query(conn, &record("available", Some("TEST_CODE_batch_1")))
        })
        .expect("append");
        diesel::sql_query("DROP TRIGGER trg_data_acquisition_audit_chain_no_update")
            .execute(&mut conn)
            .expect("test-only tamper setup");
        diesel::sql_query(
            "UPDATE data_acquisition_audit_chain
             SET previous_hash = 'TEST_CODE_wrong_previous'",
        )
        .execute(&mut conn)
        .expect("test-only tamper");

        conn.immediate_transaction::<_, diesel::result::Error, _>(|conn| {
            insert_acquisition_audit_query(conn, &record("unavailable", None))
        })
        .expect_err("tampered tail must block append");
    }
}
