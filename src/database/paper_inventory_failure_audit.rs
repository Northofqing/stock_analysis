//! BR-249 immutable audit for paper-inventory reconstruction failures.

use chrono::{NaiveDate, SecondsFormat, Utc};
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Integer, Text};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const SCHEMA_VERSION: i32 = 1;
const MINIMUM_RETENTION_YEARS: i32 = 5;
const REASON_CODE: &str = "paper_inventory_rebuild_failed";
const CHAIN_GENESIS: &str = "BR249_PAPER_INVENTORY_FAILURE_AUDIT_GENESIS_V1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PaperInventoryFailureStage {
    ParseRawFill,
    RebuildFifo,
    ProjectSellablePosition,
}

impl PaperInventoryFailureStage {
    fn as_str(self) -> &'static str {
        match self {
            Self::ParseRawFill => "parse_raw_fill",
            Self::RebuildFifo => "rebuild_fifo",
            Self::ProjectSellablePosition => "project_sellable_position",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PaperInventorySourceFact {
    pub id: i64,
    pub code: String,
    pub name: String,
    pub direction: String,
    pub fill_price_bits: Option<String>,
    pub quantity: i64,
    pub occurred_at: String,
}

impl PaperInventorySourceFact {
    pub(crate) fn new(
        id: i64,
        code: String,
        name: String,
        direction: String,
        fill_price: Option<f64>,
        quantity: i64,
        occurred_at: String,
    ) -> Self {
        Self {
            id,
            code,
            name,
            direction,
            fill_price_bits: fill_price.map(|price| format!("{:016x}", price.to_bits())),
            quantity,
            occurred_at,
        }
    }
}

#[derive(Debug)]
pub(crate) struct PaperInventoryFailureRecord<'a> {
    pub as_of_date: NaiveDate,
    pub stage: PaperInventoryFailureStage,
    pub diagnostic: &'a str,
    pub source_facts: &'a [PaperInventorySourceFact],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PaperInventoryAuditDisposition {
    Appended,
    Existing,
}

impl PaperInventoryAuditDisposition {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Appended => "appended",
            Self::Existing => "existing",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PaperInventoryFailureAuditReceipt {
    pub audit_id: i64,
    pub record_hash: String,
    pub disposition: PaperInventoryAuditDisposition,
}

#[derive(Debug)]
struct CanonicalFailure {
    failure_identity: String,
    as_of_date: String,
    stage: String,
    diagnostic: String,
    source_row_count: i64,
    source_fill_ids_json: String,
    source_facts_json: String,
    source_snapshot_hash: String,
    diagnostic_hash: String,
}

#[derive(Debug, QueryableByName, Serialize)]
struct PersistedFailure {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = Integer)]
    schema_version: i32,
    #[diesel(sql_type = Text)]
    failure_identity: String,
    #[diesel(sql_type = Text)]
    as_of_date: String,
    #[diesel(sql_type = Text)]
    stage: String,
    #[diesel(sql_type = Text)]
    reason_code: String,
    #[diesel(sql_type = Text)]
    diagnostic: String,
    #[diesel(sql_type = BigInt)]
    source_row_count: i64,
    #[diesel(sql_type = Text)]
    source_fill_ids_json: String,
    #[diesel(sql_type = Text)]
    source_facts_json: String,
    #[diesel(sql_type = Text)]
    source_snapshot_hash: String,
    #[diesel(sql_type = Text)]
    diagnostic_hash: String,
    #[diesel(sql_type = Text)]
    observed_at: String,
    #[diesel(sql_type = Integer)]
    minimum_retention_years: i32,
    #[diesel(sql_type = Text)]
    created_at: String,
}

#[derive(Debug, QueryableByName)]
struct ChainRow {
    #[diesel(sql_type = BigInt)]
    failure_audit_id: i64,
    #[diesel(sql_type = Text)]
    previous_hash: String,
    #[diesel(sql_type = Text)]
    record_hash: String,
}

fn audit_error(message: impl Into<String>) -> diesel::result::Error {
    diesel::result::Error::QueryBuilderError(Box::new(std::io::Error::other(message.into())))
}

fn hash_fields(domain: &[u8], fields: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    for field in fields {
        hasher.update((field.len() as u64).to_be_bytes());
        hasher.update(field);
    }
    hex::encode(hasher.finalize())
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn calculate_failure_identity(
    as_of_date: &str,
    stage: &str,
    source_snapshot_hash: &str,
    diagnostic_hash: &str,
) -> String {
    hash_fields(
        b"BR249_PAPER_INVENTORY_FAILURE_IDENTITY_V1\0",
        &[
            as_of_date.as_bytes(),
            stage.as_bytes(),
            source_snapshot_hash.as_bytes(),
            diagnostic_hash.as_bytes(),
        ],
    )
}

fn canonicalize(record: &PaperInventoryFailureRecord<'_>) -> Result<CanonicalFailure, String> {
    if record.diagnostic.trim().is_empty() {
        return Err("BR-249 failure diagnostic must not be blank".to_string());
    }
    let source_row_count = i64::try_from(record.source_facts.len())
        .map_err(|_| "BR-249 source row count exceeds i64".to_string())?;
    let source_fill_ids = record
        .source_facts
        .iter()
        .map(|fact| fact.id)
        .collect::<Vec<_>>();
    let source_fill_ids_json = serde_json::to_string(&source_fill_ids)
        .map_err(|error| format!("BR-249 serialize source fill ids: {error}"))?;
    let source_facts_json = serde_json::to_string(record.source_facts)
        .map_err(|error| format!("BR-249 serialize source facts: {error}"))?;
    let source_snapshot_hash = hash_fields(
        b"BR249_PAPER_INVENTORY_SOURCE_SNAPSHOT_V1\0",
        &[source_facts_json.as_bytes()],
    );
    let diagnostic_hash = hash_fields(
        b"BR249_PAPER_INVENTORY_DIAGNOSTIC_V1\0",
        &[record.diagnostic.as_bytes()],
    );
    let as_of_date = record.as_of_date.format("%Y-%m-%d").to_string();
    let stage = record.stage.as_str().to_string();
    let failure_identity =
        calculate_failure_identity(&as_of_date, &stage, &source_snapshot_hash, &diagnostic_hash);
    Ok(CanonicalFailure {
        failure_identity,
        as_of_date,
        stage,
        diagnostic: record.diagnostic.to_string(),
        source_row_count,
        source_fill_ids_json,
        source_facts_json,
        source_snapshot_hash,
        diagnostic_hash,
    })
}

fn load_failures(conn: &mut SqliteConnection) -> diesel::QueryResult<Vec<PersistedFailure>> {
    diesel::sql_query(
        "SELECT id, schema_version, failure_identity, as_of_date, stage, reason_code,
                diagnostic, source_row_count, source_fill_ids_json, source_facts_json,
                source_snapshot_hash, diagnostic_hash, observed_at,
                minimum_retention_years, created_at
         FROM paper_inventory_failure_audit ORDER BY id ASC",
    )
    .load(conn)
}

fn load_chains(conn: &mut SqliteConnection) -> diesel::QueryResult<Vec<ChainRow>> {
    diesel::sql_query(
        "SELECT failure_audit_id, previous_hash, record_hash
         FROM paper_inventory_failure_audit_chain ORDER BY failure_audit_id ASC",
    )
    .load(conn)
}

fn calculate_record_hash(
    previous_hash: &str,
    failure: &PersistedFailure,
) -> diesel::QueryResult<String> {
    let payload = serde_json::to_vec(failure)
        .map_err(|error| audit_error(format!("BR-249 serialize persisted failure: {error}")))?;
    Ok(hash_fields(
        b"BR249_PAPER_INVENTORY_FAILURE_RECORD_V1\0",
        &[previous_hash.as_bytes(), &payload],
    ))
}

fn validate_persisted_failure(failure: &PersistedFailure) -> diesel::QueryResult<()> {
    if failure.schema_version != SCHEMA_VERSION
        || failure.reason_code != REASON_CODE
        || !matches!(
            failure.stage.as_str(),
            "parse_raw_fill" | "rebuild_fifo" | "project_sellable_position"
        )
        || failure.diagnostic.trim().is_empty()
        || failure.source_row_count < 0
        || failure.observed_at.trim().is_empty()
        || failure.minimum_retention_years < MINIMUM_RETENTION_YEARS
        || !is_lower_hex(&failure.failure_identity, 64)
        || !is_lower_hex(&failure.source_snapshot_hash, 64)
        || !is_lower_hex(&failure.diagnostic_hash, 64)
    {
        return Err(audit_error(format!(
            "BR-249 persisted failure row {} violates schema invariants",
            failure.id
        )));
    }
    NaiveDate::parse_from_str(&failure.as_of_date, "%Y-%m-%d").map_err(|error| {
        audit_error(format!(
            "BR-249 persisted failure row {} has invalid as_of_date: {error}",
            failure.id
        ))
    })?;
    chrono::DateTime::parse_from_rfc3339(&failure.observed_at).map_err(|error| {
        audit_error(format!(
            "BR-249 persisted failure row {} has invalid observed_at: {error}",
            failure.id
        ))
    })?;

    let facts = serde_json::from_str::<Vec<PaperInventorySourceFact>>(&failure.source_facts_json)
        .map_err(|error| {
        audit_error(format!(
            "BR-249 persisted failure row {} has invalid source facts: {error}",
            failure.id
        ))
    })?;
    if facts.iter().any(|fact| {
        fact.fill_price_bits
            .as_deref()
            .is_some_and(|bits| !is_lower_hex(bits, 16))
    }) {
        return Err(audit_error(format!(
            "BR-249 persisted failure row {} has invalid fill price bits",
            failure.id
        )));
    }
    let ids = serde_json::from_str::<Vec<i64>>(&failure.source_fill_ids_json).map_err(|error| {
        audit_error(format!(
            "BR-249 persisted failure row {} has invalid source ids: {error}",
            failure.id
        ))
    })?;
    if i64::try_from(facts.len()).ok() != Some(failure.source_row_count)
        || ids != facts.iter().map(|fact| fact.id).collect::<Vec<_>>()
        || serde_json::to_string(&facts).ok().as_deref() != Some(&failure.source_facts_json)
        || serde_json::to_string(&ids).ok().as_deref() != Some(&failure.source_fill_ids_json)
    {
        return Err(audit_error(format!(
            "BR-249 persisted failure row {} source snapshot is non-canonical",
            failure.id
        )));
    }
    let expected_source_hash = hash_fields(
        b"BR249_PAPER_INVENTORY_SOURCE_SNAPSHOT_V1\0",
        &[failure.source_facts_json.as_bytes()],
    );
    let expected_diagnostic_hash = hash_fields(
        b"BR249_PAPER_INVENTORY_DIAGNOSTIC_V1\0",
        &[failure.diagnostic.as_bytes()],
    );
    let expected_identity = calculate_failure_identity(
        &failure.as_of_date,
        &failure.stage,
        &expected_source_hash,
        &expected_diagnostic_hash,
    );
    if failure.source_snapshot_hash != expected_source_hash
        || failure.diagnostic_hash != expected_diagnostic_hash
        || failure.failure_identity != expected_identity
    {
        return Err(audit_error(format!(
            "BR-249 persisted failure row {} hash identity mismatch",
            failure.id
        )));
    }
    Ok(())
}

fn validate_chain(conn: &mut SqliteConnection) -> diesel::QueryResult<String> {
    let failures = load_failures(conn)?;
    let chains = load_chains(conn)?;
    if failures.len() != chains.len() {
        return Err(audit_error(format!(
            "BR-249 audit chain length mismatch: failure_rows={}, chain_rows={}",
            failures.len(),
            chains.len()
        )));
    }
    let mut previous = CHAIN_GENESIS.to_string();
    for (failure, chain) in failures.iter().zip(chains.iter()) {
        validate_persisted_failure(failure)?;
        if chain.failure_audit_id != failure.id || chain.previous_hash != previous {
            return Err(audit_error(format!(
                "BR-249 audit chain linkage mismatch at failure id {}",
                failure.id
            )));
        }
        let expected = calculate_record_hash(&previous, failure)?;
        if chain.record_hash != expected {
            return Err(audit_error(format!(
                "BR-249 audit chain hash mismatch at failure id {}",
                failure.id
            )));
        }
        previous = chain.record_hash.clone();
    }
    Ok(previous)
}

pub(super) fn create_schema(conn: &mut SqliteConnection) -> diesel::QueryResult<()> {
    diesel::sql_query(
        "CREATE TABLE IF NOT EXISTS paper_inventory_failure_audit (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            schema_version INTEGER NOT NULL CHECK(schema_version = 1),
            failure_identity TEXT NOT NULL UNIQUE CHECK(length(failure_identity) = 64),
            as_of_date TEXT NOT NULL,
            stage TEXT NOT NULL CHECK(stage IN (
                'parse_raw_fill', 'rebuild_fifo', 'project_sellable_position'
            )),
            reason_code TEXT NOT NULL CHECK(reason_code = 'paper_inventory_rebuild_failed'),
            diagnostic TEXT NOT NULL CHECK(length(trim(diagnostic)) > 0),
            source_row_count INTEGER NOT NULL CHECK(source_row_count >= 0),
            source_fill_ids_json TEXT NOT NULL,
            source_facts_json TEXT NOT NULL,
            source_snapshot_hash TEXT NOT NULL CHECK(length(source_snapshot_hash) = 64),
            diagnostic_hash TEXT NOT NULL CHECK(length(diagnostic_hash) = 64),
            observed_at TEXT NOT NULL CHECK(length(trim(observed_at)) > 0),
            minimum_retention_years INTEGER NOT NULL CHECK(minimum_retention_years >= 5),
            created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
        )",
    )
    .execute(conn)?;
    diesel::sql_query(
        "CREATE INDEX IF NOT EXISTS idx_paper_inventory_failure_audit_date_stage
         ON paper_inventory_failure_audit(as_of_date, stage, id)",
    )
    .execute(conn)?;
    diesel::sql_query(
        "CREATE TRIGGER IF NOT EXISTS trg_paper_inventory_failure_audit_no_update
         BEFORE UPDATE ON paper_inventory_failure_audit
         BEGIN SELECT RAISE(ABORT, 'BR-249 paper inventory failure audit is immutable'); END",
    )
    .execute(conn)?;
    diesel::sql_query(
        "CREATE TRIGGER IF NOT EXISTS trg_paper_inventory_failure_audit_no_delete
         BEFORE DELETE ON paper_inventory_failure_audit
         BEGIN SELECT RAISE(ABORT, 'BR-249 paper inventory failure audit retention is at least five years'); END",
    )
    .execute(conn)?;
    diesel::sql_query(
        "CREATE TABLE IF NOT EXISTS paper_inventory_failure_audit_chain (
            failure_audit_id INTEGER PRIMARY KEY NOT NULL,
            previous_hash TEXT NOT NULL,
            record_hash TEXT NOT NULL UNIQUE,
            created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
            FOREIGN KEY(failure_audit_id) REFERENCES paper_inventory_failure_audit(id)
        )",
    )
    .execute(conn)?;
    diesel::sql_query(
        "CREATE TRIGGER IF NOT EXISTS trg_paper_inventory_failure_audit_chain_no_update
         BEFORE UPDATE ON paper_inventory_failure_audit_chain
         BEGIN SELECT RAISE(ABORT, 'BR-249 paper inventory failure audit chain is immutable'); END",
    )
    .execute(conn)?;
    diesel::sql_query(
        "CREATE TRIGGER IF NOT EXISTS trg_paper_inventory_failure_audit_chain_no_delete
         BEFORE DELETE ON paper_inventory_failure_audit_chain
         BEGIN SELECT RAISE(ABORT, 'BR-249 paper inventory failure audit chain retention is at least five years'); END",
    )
    .execute(conn)?;
    validate_chain(conn).map(|_| ())
}

fn persisted_matches(failure: &PersistedFailure, expected: &CanonicalFailure) -> bool {
    failure.schema_version == SCHEMA_VERSION
        && failure.failure_identity == expected.failure_identity
        && failure.as_of_date == expected.as_of_date
        && failure.stage == expected.stage
        && failure.reason_code == REASON_CODE
        && failure.diagnostic == expected.diagnostic
        && failure.source_row_count == expected.source_row_count
        && failure.source_fill_ids_json == expected.source_fill_ids_json
        && failure.source_facts_json == expected.source_facts_json
        && failure.source_snapshot_hash == expected.source_snapshot_hash
        && failure.diagnostic_hash == expected.diagnostic_hash
        && failure.minimum_retention_years == MINIMUM_RETENTION_YEARS
}

fn append_query(
    conn: &mut SqliteConnection,
    canonical: &CanonicalFailure,
) -> diesel::QueryResult<PaperInventoryFailureAuditReceipt> {
    let previous_hash = validate_chain(conn)?;
    let existing = diesel::sql_query(
        "SELECT id, schema_version, failure_identity, as_of_date, stage, reason_code,
                diagnostic, source_row_count, source_fill_ids_json, source_facts_json,
                source_snapshot_hash, diagnostic_hash, observed_at,
                minimum_retention_years, created_at
         FROM paper_inventory_failure_audit WHERE failure_identity = ?",
    )
    .bind::<Text, _>(&canonical.failure_identity)
    .get_result::<PersistedFailure>(conn)
    .optional()?;
    if let Some(existing) = existing {
        if !persisted_matches(&existing, canonical) {
            return Err(audit_error(format!(
                "BR-249 existing failure identity {} has conflicting content",
                canonical.failure_identity
            )));
        }
        let chain = diesel::sql_query(
            "SELECT failure_audit_id, previous_hash, record_hash
             FROM paper_inventory_failure_audit_chain WHERE failure_audit_id = ?",
        )
        .bind::<BigInt, _>(existing.id)
        .get_result::<ChainRow>(conn)?;
        return Ok(PaperInventoryFailureAuditReceipt {
            audit_id: existing.id,
            record_hash: chain.record_hash,
            disposition: PaperInventoryAuditDisposition::Existing,
        });
    }

    let observed_at = Utc::now().to_rfc3339_opts(SecondsFormat::Nanos, true);
    let affected = diesel::sql_query(
        "INSERT INTO paper_inventory_failure_audit (
            schema_version, failure_identity, as_of_date, stage, reason_code, diagnostic,
            source_row_count, source_fill_ids_json, source_facts_json, source_snapshot_hash,
            diagnostic_hash, observed_at, minimum_retention_years
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind::<Integer, _>(SCHEMA_VERSION)
    .bind::<Text, _>(&canonical.failure_identity)
    .bind::<Text, _>(&canonical.as_of_date)
    .bind::<Text, _>(&canonical.stage)
    .bind::<Text, _>(REASON_CODE)
    .bind::<Text, _>(&canonical.diagnostic)
    .bind::<BigInt, _>(canonical.source_row_count)
    .bind::<Text, _>(&canonical.source_fill_ids_json)
    .bind::<Text, _>(&canonical.source_facts_json)
    .bind::<Text, _>(&canonical.source_snapshot_hash)
    .bind::<Text, _>(&canonical.diagnostic_hash)
    .bind::<Text, _>(&observed_at)
    .bind::<Integer, _>(MINIMUM_RETENTION_YEARS)
    .execute(conn)?;
    if affected != 1 {
        return Err(audit_error(format!(
            "BR-249 insert failure audit affected {affected} rows"
        )));
    }
    let failure = diesel::sql_query(
        "SELECT id, schema_version, failure_identity, as_of_date, stage, reason_code,
                diagnostic, source_row_count, source_fill_ids_json, source_facts_json,
                source_snapshot_hash, diagnostic_hash, observed_at,
                minimum_retention_years, created_at
         FROM paper_inventory_failure_audit WHERE id = last_insert_rowid()",
    )
    .get_result::<PersistedFailure>(conn)?;
    if !persisted_matches(&failure, canonical) {
        return Err(audit_error(format!(
            "BR-249 inserted failure row {} differs from canonical input",
            failure.id
        )));
    }
    validate_persisted_failure(&failure)?;
    let record_hash = calculate_record_hash(&previous_hash, &failure)?;
    let chain_affected = diesel::sql_query(
        "INSERT INTO paper_inventory_failure_audit_chain
         (failure_audit_id, previous_hash, record_hash) VALUES (?, ?, ?)",
    )
    .bind::<BigInt, _>(failure.id)
    .bind::<Text, _>(&previous_hash)
    .bind::<Text, _>(&record_hash)
    .execute(conn)?;
    if chain_affected != 1 {
        return Err(audit_error(format!(
            "BR-249 insert failure audit chain affected {chain_affected} rows"
        )));
    }
    Ok(PaperInventoryFailureAuditReceipt {
        audit_id: failure.id,
        record_hash,
        disposition: PaperInventoryAuditDisposition::Appended,
    })
}

pub(crate) fn append_failure_on_conn(
    conn: &mut SqliteConnection,
    record: &PaperInventoryFailureRecord<'_>,
) -> Result<PaperInventoryFailureAuditReceipt, String> {
    let canonical = canonicalize(record)?;
    conn.immediate_transaction::<_, diesel::result::Error, _>(|conn| append_query(conn, &canonical))
        .map_err(|error| format!("BR-249 failure audit append: {error}"))
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
        create_schema(&mut conn).expect("paper inventory audit schema");
        conn
    }

    fn facts(price: f64) -> Vec<PaperInventorySourceFact> {
        vec![PaperInventorySourceFact::new(
            1,
            "TEST_CODE_600001".to_string(),
            "测试股票".to_string(),
            "buy".to_string(),
            Some(price),
            100,
            "2026-08-11 09:31:00".to_string(),
        )]
    }

    fn record<'a>(
        facts: &'a [PaperInventorySourceFact],
        diagnostic: &'a str,
    ) -> PaperInventoryFailureRecord<'a> {
        PaperInventoryFailureRecord {
            as_of_date: NaiveDate::from_ymd_opt(2026, 8, 23).unwrap(),
            stage: PaperInventoryFailureStage::RebuildFifo,
            diagnostic,
            source_facts: facts,
        }
    }

    fn count(conn: &mut SqliteConnection, table: &str) -> i64 {
        diesel::sql_query(format!("SELECT COUNT(*) AS count FROM {table}"))
            .get_result::<CountRow>(conn)
            .expect("count audit rows")
            .count
    }

    #[test]
    fn exact_replay_returns_existing_receipt_without_appending() {
        let mut conn = connection();
        let facts = facts(10.0);

        let first = append_failure_on_conn(&mut conn, &record(&facts, "TEST_CODE T+1 violation"))
            .expect("first append");
        let replay = append_failure_on_conn(&mut conn, &record(&facts, "TEST_CODE T+1 violation"))
            .expect("exact replay");

        assert_eq!(first.disposition, PaperInventoryAuditDisposition::Appended);
        assert_eq!(replay.disposition, PaperInventoryAuditDisposition::Existing);
        assert_eq!(first.audit_id, replay.audit_id);
        assert_eq!(first.record_hash, replay.record_hash);
        assert_eq!(count(&mut conn, "paper_inventory_failure_audit"), 1);
        assert_eq!(count(&mut conn, "paper_inventory_failure_audit_chain"), 1);
    }

    #[test]
    fn source_or_diagnostic_change_appends_new_failure() {
        let mut conn = connection();
        let first_facts = facts(10.0);
        let changed_facts = facts(10.5);

        append_failure_on_conn(&mut conn, &record(&first_facts, "TEST_CODE T+1 violation"))
            .expect("first append");
        append_failure_on_conn(
            &mut conn,
            &record(&changed_facts, "TEST_CODE T+1 violation"),
        )
        .expect("changed source append");
        append_failure_on_conn(&mut conn, &record(&changed_facts, "TEST_CODE oversell"))
            .expect("changed diagnostic append");

        assert_eq!(count(&mut conn, "paper_inventory_failure_audit"), 3);
        assert_eq!(count(&mut conn, "paper_inventory_failure_audit_chain"), 3);
    }

    #[test]
    fn audit_and_chain_are_immutable_with_five_year_retention() {
        let mut conn = connection();
        let facts = facts(10.0);
        append_failure_on_conn(&mut conn, &record(&facts, "TEST_CODE T+1 violation"))
            .expect("append audit");

        let update = diesel::sql_query(
            "UPDATE paper_inventory_failure_audit SET diagnostic = 'changed' WHERE id = 1",
        )
        .execute(&mut conn)
        .expect_err("audit update must fail");
        let delete = diesel::sql_query(
            "DELETE FROM paper_inventory_failure_audit_chain WHERE failure_audit_id = 1",
        )
        .execute(&mut conn)
        .expect_err("chain delete must fail");

        assert!(update.to_string().contains("immutable"), "{update}");
        assert!(delete.to_string().contains("five years"), "{delete}");
    }

    #[test]
    fn tampered_history_blocks_later_append() {
        let mut conn = connection();
        let first_facts = facts(10.0);
        append_failure_on_conn(&mut conn, &record(&first_facts, "TEST_CODE T+1 violation"))
            .expect("append audit");
        diesel::sql_query("DROP TRIGGER trg_paper_inventory_failure_audit_no_update")
            .execute(&mut conn)
            .expect("test-only remove immutability trigger");
        diesel::sql_query(
            "UPDATE paper_inventory_failure_audit SET diagnostic = 'tampered' WHERE id = 1",
        )
        .execute(&mut conn)
        .expect("test-only tamper");

        let changed_facts = facts(11.0);
        let error =
            append_failure_on_conn(&mut conn, &record(&changed_facts, "TEST_CODE new failure"))
                .expect_err("tampered chain must block append");

        assert!(error.contains("hash identity mismatch"), "{error}");
        assert_eq!(count(&mut conn, "paper_inventory_failure_audit"), 1);
    }

    #[test]
    fn chain_insert_failure_rolls_back_audit_row() {
        let mut conn = connection();
        diesel::sql_query(
            "CREATE TRIGGER TEST_CODE_fail_paper_inventory_chain
             BEFORE INSERT ON paper_inventory_failure_audit_chain
             BEGIN SELECT RAISE(ABORT, 'TEST_CODE chain insert failed'); END",
        )
        .execute(&mut conn)
        .expect("install test failure trigger");
        let facts = facts(10.0);

        let error = append_failure_on_conn(&mut conn, &record(&facts, "TEST_CODE T+1 violation"))
            .expect_err("chain failure must fail append");

        assert!(error.contains("TEST_CODE chain insert failed"), "{error}");
        assert_eq!(count(&mut conn, "paper_inventory_failure_audit"), 0);
        assert_eq!(count(&mut conn, "paper_inventory_failure_audit_chain"), 0);
    }

    #[test]
    fn all_failure_stages_have_stable_names() {
        assert_eq!(
            PaperInventoryFailureStage::ParseRawFill.as_str(),
            "parse_raw_fill"
        );
        assert_eq!(
            PaperInventoryFailureStage::ProjectSellablePosition.as_str(),
            "project_sellable_position"
        );
    }
}
