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

#[derive(Debug, QueryableByName, Serialize)]
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

pub(super) fn validate_data_acquisition_audit_chain(
    conn: &mut SqliteConnection,
) -> diesel::QueryResult<String> {
    let audits = load_audit_rows(conn)?;
    let chain = load_chain_rows(conn)?;
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, QueryableByName)]
    struct CountRow {
        #[diesel(sql_type = BigInt)]
        count: i64,
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
