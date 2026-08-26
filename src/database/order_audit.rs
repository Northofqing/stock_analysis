//! BR-086 immutable order-attempt audit persistence.

use diesel::prelude::*;
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::position_chain::{append_assignment_and_link_on_conn, PositionChainStoreError};
use super::DatabaseManager;
use crate::data_gateway::PositionChainAssignment;
use crate::models::NewStockPosition;
use crate::schema::stock_position;

#[derive(Debug, Clone)]
pub struct OrderAuditRecord<'a> {
    pub business_order_id: &'a str,
    pub source: &'a str,
    pub decision_basis: &'a str,
    pub side: &'a str,
    pub code: &'a str,
    pub requested_price: f64,
    pub execution_price: Option<f64>,
    pub quantity: i64,
    pub quote_observed_at: Option<&'a str>,
    pub outcome: &'a str,
    pub failure_reason: Option<&'a str>,
}

pub(crate) const AUDIT_CHAIN_GENESIS: &str = "BR086_ORDER_AUDIT_GENESIS_V1";

#[derive(Debug, Clone, PartialEq, QueryableByName, Serialize)]
pub(crate) struct CanonicalOrderAuditRow {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub(crate) id: i64,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub(crate) business_order_id: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub(crate) source: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub(crate) decision_basis: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub(crate) side: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub(crate) code: String,
    #[diesel(sql_type = diesel::sql_types::Double)]
    pub(crate) requested_price: f64,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    pub(crate) execution_price: Option<f64>,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub(crate) quantity: i64,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    pub(crate) quote_observed_at: Option<String>,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub(crate) outcome: String,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    pub(crate) failure_reason: Option<String>,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub(crate) created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, QueryableByName)]
pub(crate) struct CanonicalOrderAuditChainRow {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub(crate) order_audit_id: i64,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub(crate) previous_hash: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub(crate) record_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OrderAuditAppendReceipt {
    pub order_audit_id: i64,
    pub previous_hash: String,
    pub record_hash: String,
    pub created_at: String,
}

fn audit_chain_error(message: impl Into<String>) -> diesel::result::Error {
    diesel::result::Error::QueryBuilderError(Box::new(std::io::Error::other(message.into())))
}

fn load_audit_rows(
    conn: &mut SqliteConnection,
) -> diesel::QueryResult<Vec<CanonicalOrderAuditRow>> {
    diesel::sql_query(
        "SELECT id, business_order_id, source, decision_basis, side, code,
                requested_price, execution_price, quantity, quote_observed_at,
                outcome, failure_reason, created_at
         FROM order_audit ORDER BY id ASC",
    )
    .load(conn)
}

fn load_chain_rows(
    conn: &mut SqliteConnection,
) -> diesel::QueryResult<Vec<CanonicalOrderAuditChainRow>> {
    diesel::sql_query(
        "SELECT order_audit_id, previous_hash, record_hash
         FROM order_audit_chain ORDER BY order_audit_id ASC",
    )
    .load(conn)
}

pub(crate) fn canonical_order_audit_record_hash(
    previous_hash: &str,
    record: &CanonicalOrderAuditRow,
) -> Result<String, String> {
    let payload = serde_json::to_vec(record)
        .map_err(|error| format!("BR-086 serialize audit row: {error}"))?;
    let mut hasher = Sha256::new();
    hasher.update(b"BR086_ORDER_AUDIT_V1\0");
    hasher.update(previous_hash.as_bytes());
    hasher.update(b"\0");
    hasher.update(payload);
    Ok(hex::encode(hasher.finalize()))
}

pub(crate) fn validate_canonical_order_audit_chain(
    audits: &[CanonicalOrderAuditRow],
    chain: &[CanonicalOrderAuditChainRow],
) -> Result<String, String> {
    if audits.len() != chain.len() {
        return Err(format!(
            "BR-086 order audit hash chain length mismatch: audit_rows={}, chain_rows={}",
            audits.len(),
            chain.len()
        ));
    }

    let mut previous = AUDIT_CHAIN_GENESIS.to_string();
    for (audit, evidence) in audits.iter().zip(chain.iter()) {
        if evidence.order_audit_id != audit.id || evidence.previous_hash != previous {
            return Err(format!(
                "BR-086 order audit hash chain linkage mismatch at audit id {}",
                audit.id
            ));
        }
        let expected = canonical_order_audit_record_hash(&previous, audit)?;
        if evidence.record_hash != expected {
            return Err(format!(
                "BR-086 order audit hash mismatch at audit id {}",
                audit.id
            ));
        }
        previous = evidence.record_hash.clone();
    }
    Ok(previous)
}

fn validate_order_audit_chain(conn: &mut SqliteConnection) -> diesel::QueryResult<String> {
    let audits = load_audit_rows(conn)?;
    let chain = load_chain_rows(conn)?;
    validate_canonical_order_audit_chain(&audits, &chain).map_err(audit_chain_error)
}

fn append_chain_row(
    conn: &mut SqliteConnection,
    previous_hash: &str,
    audit: &CanonicalOrderAuditRow,
) -> diesel::QueryResult<()> {
    let record_hash =
        canonical_order_audit_record_hash(previous_hash, audit).map_err(audit_chain_error)?;
    let rows = diesel::sql_query(
        "INSERT INTO order_audit_chain
         (order_audit_id, previous_hash, record_hash)
         VALUES (?, ?, ?)",
    )
    .bind::<diesel::sql_types::BigInt, _>(audit.id)
    .bind::<diesel::sql_types::Text, _>(previous_hash)
    .bind::<diesel::sql_types::Text, _>(&record_hash)
    .execute(conn)?;
    if rows != 1 {
        return Err(audit_chain_error(format!(
            "BR-086 append hash chain affected {rows} rows"
        )));
    }
    Ok(())
}

pub(super) fn initialize_order_audit_chain(conn: &mut SqliteConnection) -> diesel::QueryResult<()> {
    conn.transaction::<_, diesel::result::Error, _>(|conn| {
        let audits = load_audit_rows(conn)?;
        let chain = load_chain_rows(conn)?;
        if chain.is_empty() && !audits.is_empty() {
            let mut previous = AUDIT_CHAIN_GENESIS.to_string();
            for audit in &audits {
                append_chain_row(conn, &previous, audit)?;
                previous = canonical_order_audit_record_hash(&previous, audit)
                    .map_err(audit_chain_error)?;
            }
        } else if audits.len() != chain.len() {
            return Err(audit_chain_error(format!(
                "BR-086 refuses implicit repair of partial order audit hash chain: audit_rows={}, chain_rows={}",
                audits.len(),
                chain.len()
            )));
        }
        validate_order_audit_chain(conn).map(|_| ())
    })
}

pub(crate) fn insert_order_audit_with_receipt_query(
    conn: &mut SqliteConnection,
    record: &OrderAuditRecord<'_>,
) -> diesel::QueryResult<OrderAuditAppendReceipt> {
    let previous_hash = validate_order_audit_chain(conn)?;
    let rows = diesel::sql_query(
        "INSERT INTO order_audit
         (business_order_id, source, decision_basis, side, code, requested_price,
          execution_price, quantity, quote_observed_at, outcome, failure_reason)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind::<diesel::sql_types::Text, _>(record.business_order_id)
    .bind::<diesel::sql_types::Text, _>(record.source)
    .bind::<diesel::sql_types::Text, _>(record.decision_basis)
    .bind::<diesel::sql_types::Text, _>(record.side)
    .bind::<diesel::sql_types::Text, _>(record.code)
    .bind::<diesel::sql_types::Double, _>(record.requested_price)
    .bind::<diesel::sql_types::Nullable<diesel::sql_types::Double>, _>(record.execution_price)
    .bind::<diesel::sql_types::BigInt, _>(record.quantity)
    .bind::<diesel::sql_types::Nullable<diesel::sql_types::Text>, _>(record.quote_observed_at)
    .bind::<diesel::sql_types::Text, _>(record.outcome)
    .bind::<diesel::sql_types::Nullable<diesel::sql_types::Text>, _>(record.failure_reason)
    .execute(conn)?;
    if rows != 1 {
        return Err(audit_chain_error(format!(
            "BR-086 insert order_audit affected {rows} rows"
        )));
    }
    let audit = diesel::sql_query(
        "SELECT id, business_order_id, source, decision_basis, side, code,
                requested_price, execution_price, quantity, quote_observed_at,
                outcome, failure_reason, created_at
         FROM order_audit WHERE id = last_insert_rowid()",
    )
    .get_result::<CanonicalOrderAuditRow>(conn)?;
    append_chain_row(conn, &previous_hash, &audit)?;
    let record_hash =
        canonical_order_audit_record_hash(&previous_hash, &audit).map_err(audit_chain_error)?;
    Ok(OrderAuditAppendReceipt {
        order_audit_id: audit.id,
        previous_hash,
        record_hash,
        created_at: audit.created_at,
    })
}

pub(crate) fn insert_order_audit_query(
    conn: &mut SqliteConnection,
    record: &OrderAuditRecord<'_>,
) -> diesel::QueryResult<usize> {
    insert_order_audit_with_receipt_query(conn, record).map(|_| 1)
}

pub(super) fn insert_order_audit(
    conn: &mut SqliteConnection,
    record: &OrderAuditRecord<'_>,
) -> Result<(), String> {
    conn.transaction::<_, diesel::result::Error, _>(|conn| {
        let rows = insert_order_audit_query(conn, record)?;
        if rows != 1 {
            return Err(diesel::result::Error::RollbackTransaction);
        }
        Ok(())
    })
    .map_err(|error| format!("insert order_audit and hash evidence: {error}"))
}

#[cfg(test)]
fn save_position_with_audit_on_conn(
    conn: &mut SqliteConnection,
    position: &NewStockPosition,
    audit: &OrderAuditRecord<'_>,
) -> diesel::QueryResult<()> {
    conn.transaction::<_, diesel::result::Error, _>(|conn| {
        diesel::insert_into(stock_position::table)
            .values(position)
            .execute(conn)?;
        if insert_order_audit_query(conn, audit)? != 1 {
            return Err(diesel::result::Error::RollbackTransaction);
        }
        Ok(())
    })
}

fn save_position_with_audit_and_assignment_on_conn(
    conn: &mut SqliteConnection,
    position: &NewStockPosition,
    audit: &OrderAuditRecord<'_>,
    assignment: &PositionChainAssignment,
) -> Result<(), PositionChainStoreError> {
    if position.code != assignment.code || position.code != audit.code {
        return Err(PositionChainStoreError::InvalidInput(format!(
            "position/audit/assignment code mismatch: position={} audit={} assignment={}",
            position.code, audit.code, assignment.code
        )));
    }
    if position.chain_name.is_some() {
        return Err(PositionChainStoreError::InvalidInput(
            "BR-170 new position must not carry a raw chain_name".to_string(),
        ));
    }
    conn.immediate_transaction::<_, PositionChainStoreError, _>(|conn| {
        let inserted = diesel::insert_into(stock_position::table)
            .values(position)
            .execute(conn)?;
        if inserted != 1 || insert_order_audit_query(conn, audit)? != 1 {
            return Err(PositionChainStoreError::Database(
                diesel::result::Error::RollbackTransaction,
            ));
        }
        append_assignment_and_link_on_conn(conn, assignment)?;
        Ok(())
    })
}

impl DatabaseManager {
    /// Atomically reserve a business order ID in shared persistence.
    ///
    /// `true` means the caller owns the ID for the next 60 seconds; `false`
    /// means another process/thread already reserved it in that window.
    pub fn reserve_business_order_id(&self, business_order_id: &str) -> Result<bool, String> {
        if business_order_id.trim().is_empty() {
            return Err("BR-084 business_order_id must not be blank".to_string());
        }
        let mut conn = self
            .get_conn()
            .map_err(|error| format!("order idempotency DB connection: {error}"))?;
        let rows = diesel::sql_query(
            "INSERT INTO order_idempotency (business_order_id, reserved_at)
             VALUES (?, CURRENT_TIMESTAMP)
             ON CONFLICT(business_order_id) DO UPDATE SET reserved_at = CURRENT_TIMESTAMP
             WHERE order_idempotency.reserved_at <= datetime('now', '-60 seconds')",
        )
        .bind::<diesel::sql_types::Text, _>(business_order_id)
        .execute(&mut conn)
        .map_err(|error| format!("reserve business order ID: {error}"))?;
        match rows {
            0 => Ok(false),
            1 => Ok(true),
            other => Err(format!(
                "reserve business order ID affected unexpected row count {other}"
            )),
        }
    }

    pub fn record_order_audit(&self, record: &OrderAuditRecord<'_>) -> Result<(), String> {
        let mut conn = self
            .get_conn()
            .map_err(|error| format!("order audit DB connection: {error}"))?;
        insert_order_audit(&mut conn, record)
    }

    pub fn save_position_with_audit_and_assignment(
        &self,
        position: &NewStockPosition,
        audit: &OrderAuditRecord<'_>,
        assignment: &PositionChainAssignment,
    ) -> Result<(), String> {
        crate::risk::env_guard::validate_symbol_for_current_env(&position.code)?;
        let mut conn = self
            .get_conn()
            .map_err(|error| format!("audited position-chain DB connection: {error}"))?;
        save_position_with_audit_and_assignment_on_conn(&mut conn, position, audit, assignment)
            .map_err(|error| format!("BR-170 atomic open-position transaction: {error}"))
    }

    pub fn close_position_with_audit(
        &self,
        position_id: i32,
        code: &str,
        sell_price: f64,
        sell_date: &str,
        audit: &OrderAuditRecord<'_>,
    ) -> Result<(), String> {
        crate::risk::env_guard::validate_symbol_for_current_env(code)?;
        let mut conn = self
            .get_conn()
            .map_err(|error| format!("audited close DB connection: {error}"))?;
        conn.transaction::<_, diesel::result::Error, _>(|conn| {
            let buy_price = stock_position::table
                .filter(stock_position::id.eq(position_id))
                .filter(stock_position::code.eq(code))
                .filter(stock_position::status.eq("open"))
                .select(stock_position::buy_price)
                .first::<f64>(conn)?;
            let return_rate = (sell_price / buy_price - 1.0) * 100.0;
            let updated = diesel::update(
                stock_position::table
                    .filter(stock_position::id.eq(position_id))
                    .filter(stock_position::code.eq(code))
                    .filter(stock_position::status.eq("open")),
            )
            .set((
                stock_position::status.eq("closed"),
                stock_position::sell_date.eq(sell_date),
                stock_position::sell_price.eq(sell_price),
                stock_position::return_rate.eq(return_rate),
                stock_position::updated_at.eq(diesel::dsl::now),
            ))
            .execute(conn)?;
            if updated != 1 || insert_order_audit_query(conn, audit)? != 1 {
                return Err(diesel::result::Error::RollbackTransaction);
            }
            Ok(())
        })
        .map_err(|error| format!("audited close-position transaction: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn br086_shared_canonical_validator_rejects_every_chain_mutation() {
        let first = CanonicalOrderAuditRow {
            id: 1,
            business_order_id: "TEST_CODE_ORDER_SHARED_1".to_owned(),
            source: "PaperTrade".to_owned(),
            decision_basis: "TEST_CODE shared validator".to_owned(),
            side: "buy".to_owned(),
            code: "TEST_CODE_SHARED".to_owned(),
            requested_price: 10.0,
            execution_price: Some(10.1),
            quantity: 100,
            quote_observed_at: Some("2026-08-21T09:31:05+08:00".to_owned()),
            outcome: "Filled".to_owned(),
            failure_reason: None,
            created_at: "2026-08-21 01:31:06".to_owned(),
        };
        let second = CanonicalOrderAuditRow {
            id: 2,
            business_order_id: "TEST_CODE_ORDER_SHARED_2".to_owned(),
            side: "sell".to_owned(),
            quote_observed_at: Some("2026-08-22T14:20:00+08:00".to_owned()),
            created_at: "2026-08-22 06:20:01".to_owned(),
            ..first.clone()
        };
        let first_hash = canonical_order_audit_record_hash(AUDIT_CHAIN_GENESIS, &first).unwrap();
        let second_hash = canonical_order_audit_record_hash(&first_hash, &second).unwrap();
        let rows = vec![first.clone(), second.clone()];
        let chain = vec![
            CanonicalOrderAuditChainRow {
                order_audit_id: 1,
                previous_hash: AUDIT_CHAIN_GENESIS.to_owned(),
                record_hash: first_hash,
            },
            CanonicalOrderAuditChainRow {
                order_audit_id: 2,
                previous_hash: chain_previous_for_test(&rows[0]),
                record_hash: second_hash,
            },
        ];
        assert!(validate_canonical_order_audit_chain(&rows, &chain).is_ok());

        let mut mutations = Vec::new();
        mutations.push((rows.clone(), chain[..1].to_vec()));
        let mut extra = chain.clone();
        extra.push(CanonicalOrderAuditChainRow {
            order_audit_id: 3,
            previous_hash: extra[1].record_hash.clone(),
            record_hash: "0".repeat(64),
        });
        mutations.push((rows.clone(), extra));
        let mut reordered = rows.clone();
        reordered.swap(0, 1);
        mutations.push((reordered, chain.clone()));
        let mut bad_link = chain.clone();
        bad_link[1].previous_hash = "1".repeat(64);
        mutations.push((rows.clone(), bad_link));
        let mut bad_hash = chain.clone();
        bad_hash[1].record_hash = "2".repeat(64);
        mutations.push((rows.clone(), bad_hash));
        let mut bad_content = rows.clone();
        bad_content[0].quantity = 200;
        mutations.push((bad_content, chain));

        for (mutated_rows, mutated_chain) in mutations {
            assert!(validate_canonical_order_audit_chain(&mutated_rows, &mutated_chain).is_err());
        }
    }

    #[test]
    fn br086_shared_validator_preserves_hash_valid_legacy_startup_semantics() {
        let legacy = CanonicalOrderAuditRow {
            id: 7,
            business_order_id: String::new(),
            source: String::new(),
            decision_basis: String::new(),
            side: "TEST_CODE_LEGACY_SIDE".to_owned(),
            code: String::new(),
            requested_price: 0.0,
            execution_price: None,
            quantity: 0,
            quote_observed_at: Some(String::new()),
            outcome: "TEST_CODE_LEGACY_OUTCOME".to_owned(),
            failure_reason: None,
            created_at: String::new(),
        };
        let record_hash = canonical_order_audit_record_hash(AUDIT_CHAIN_GENESIS, &legacy).unwrap();
        let chain = CanonicalOrderAuditChainRow {
            order_audit_id: legacy.id,
            previous_hash: AUDIT_CHAIN_GENESIS.to_owned(),
            record_hash: record_hash.clone(),
        };

        assert_eq!(
            validate_canonical_order_audit_chain(&[legacy], &[chain]).unwrap(),
            record_hash
        );
    }

    fn chain_previous_for_test(row: &CanonicalOrderAuditRow) -> String {
        canonical_order_audit_record_hash(AUDIT_CHAIN_GENESIS, row).unwrap()
    }

    #[derive(Debug, QueryableByName)]
    struct CountRow {
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        count: i64,
    }

    fn isolated_connection() -> SqliteConnection {
        let mut conn = SqliteConnection::establish(":memory:").expect("in-memory SQLite");
        DatabaseManager::run_migrations_for_test(&mut conn).expect("test migrations");
        conn
    }

    fn insert_legacy_audit(conn: &mut SqliteConnection, business_order_id: &str) {
        diesel::sql_query(
            "INSERT INTO order_audit
             (business_order_id, source, decision_basis, side, code,
              requested_price, execution_price, quantity, quote_observed_at,
              outcome, failure_reason)
             VALUES (?, 'DatabaseTest', 'legacy import', 'buy', 'TEST_CODE_BR086',
                     10.0, NULL, 100, '2026-07-18T09:30:00+08:00',
                     'Rejected', 'TEST_CODE legacy rejection')",
        )
        .bind::<diesel::sql_types::Text, _>(business_order_id)
        .execute(conn)
        .expect("insert legacy audit without chain evidence");
    }

    fn table_count(conn: &mut SqliteConnection, table: &str) -> i64 {
        diesel::sql_query(format!("SELECT COUNT(*) AS count FROM {table}"))
            .get_result::<CountRow>(conn)
            .expect("count table")
            .count
    }

    fn audit_record<'a>(business_order_id: &'a str) -> OrderAuditRecord<'a> {
        OrderAuditRecord {
            business_order_id,
            source: "DatabaseTest",
            decision_basis: "TEST_CODE complete audit append",
            side: "buy",
            code: "TEST_CODE_BR086_APPEND",
            requested_price: 10.0,
            execution_price: Some(10.0),
            quantity: 100,
            quote_observed_at: Some("2026-07-18T09:30:00+08:00"),
            outcome: "Filled",
            failure_reason: None,
        }
    }

    #[test]
    fn br086_empty_legacy_chain_is_backfilled_once_and_validated() {
        let mut conn = isolated_connection();
        insert_legacy_audit(&mut conn, "TEST_ORDER_BR086_BACKFILL");

        initialize_order_audit_chain(&mut conn).expect("one-time empty-chain backfill");
        assert_eq!(table_count(&mut conn, "order_audit"), 1);
        assert_eq!(table_count(&mut conn, "order_audit_chain"), 1);
        validate_order_audit_chain(&mut conn).expect("backfilled chain validates");
        initialize_order_audit_chain(&mut conn).expect("validated restart is idempotent");
        assert_eq!(table_count(&mut conn, "order_audit_chain"), 1);
    }

    #[test]
    fn br086_partial_chain_is_rejected_without_implicit_repair() {
        let mut conn = isolated_connection();
        insert_legacy_audit(&mut conn, "TEST_ORDER_BR086_PARTIAL_1");
        insert_legacy_audit(&mut conn, "TEST_ORDER_BR086_PARTIAL_2");
        let audits = load_audit_rows(&mut conn).expect("load legacy audits");
        append_chain_row(&mut conn, AUDIT_CHAIN_GENESIS, &audits[0])
            .expect("create deliberate partial chain");

        let error = initialize_order_audit_chain(&mut conn)
            .expect_err("partial chain must fail closed")
            .to_string();
        assert!(error.contains("refuses implicit repair"), "{error}");
        assert_eq!(table_count(&mut conn, "order_audit_chain"), 1);
    }

    #[test]
    fn br086_bad_hash_is_rejected_on_startup_validation() {
        let mut conn = isolated_connection();
        insert_legacy_audit(&mut conn, "TEST_ORDER_BR086_BAD_HASH");
        initialize_order_audit_chain(&mut conn).expect("initial backfill");
        diesel::sql_query("DROP TRIGGER trg_order_audit_chain_no_update")
            .execute(&mut conn)
            .expect("test-only tamper setup");
        diesel::sql_query("UPDATE order_audit_chain SET record_hash = 'TEST_CODE_TAMPERED'")
            .execute(&mut conn)
            .expect("test-only tamper");

        let error = initialize_order_audit_chain(&mut conn)
            .expect_err("bad hash must reject startup")
            .to_string();
        assert!(error.contains("hash mismatch"), "{error}");
    }

    #[test]
    fn br086_validation_rejects_length_and_linkage_mismatches() {
        let mut length_mismatch = isolated_connection();
        insert_legacy_audit(&mut length_mismatch, "TEST_ORDER_BR086_LENGTH");
        let error = validate_order_audit_chain(&mut length_mismatch)
            .expect_err("missing chain row must fail")
            .to_string();
        assert!(error.contains("length mismatch"), "{error}");

        let mut linkage_mismatch = isolated_connection();
        insert_legacy_audit(&mut linkage_mismatch, "TEST_ORDER_BR086_LINK");
        initialize_order_audit_chain(&mut linkage_mismatch).expect("initial chain");
        diesel::sql_query("DROP TRIGGER trg_order_audit_chain_no_update")
            .execute(&mut linkage_mismatch)
            .expect("test-only linkage tamper setup");
        diesel::sql_query(
            "UPDATE order_audit_chain SET previous_hash = 'TEST_CODE_WRONG_PREVIOUS'",
        )
        .execute(&mut linkage_mismatch)
        .expect("test-only linkage tamper");
        let error = validate_order_audit_chain(&mut linkage_mismatch)
            .expect_err("wrong previous hash must fail")
            .to_string();
        assert!(error.contains("linkage mismatch"), "{error}");
    }

    #[test]
    fn br086_successful_append_commits_one_audit_and_one_hash_row() {
        let mut conn = isolated_connection();
        let record = audit_record("TEST_ORDER_BR086_APPEND");
        insert_order_audit(&mut conn, &record).expect("atomic audit append");
        assert_eq!(table_count(&mut conn, "order_audit"), 1);
        assert_eq!(table_count(&mut conn, "order_audit_chain"), 1);
        validate_order_audit_chain(&mut conn).expect("appended chain validates");
    }

    #[test]
    fn br086_chain_insert_failure_rolls_back_audit_and_position() {
        let mut conn = isolated_connection();
        diesel::sql_query(
            "CREATE TRIGGER test_fail_order_audit_chain_insert
             BEFORE INSERT ON order_audit_chain
             BEGIN SELECT RAISE(ABORT, 'TEST_CODE forced chain failure'); END",
        )
        .execute(&mut conn)
        .expect("install chain failure trigger");
        let position = NewStockPosition {
            code: "TEST_CODE_BR086_ROLLBACK".to_string(),
            name: "审计回滚测试".to_string(),
            buy_date: "2026-07-18".to_string(),
            buy_price: 10.0,
            quantity: 100,
            status: "open".to_string(),
            st_type: None,
            chain_name: None,
        };
        let audit = OrderAuditRecord {
            business_order_id: "TEST_ORDER_BR086_ROLLBACK",
            source: "DatabaseTest",
            decision_basis: "TEST_CODE forced rollback",
            side: "buy",
            code: "TEST_CODE_BR086_ROLLBACK",
            requested_price: 10.0,
            execution_price: Some(10.0),
            quantity: 100,
            quote_observed_at: Some("2026-07-18T09:30:00+08:00"),
            outcome: "Filled",
            failure_reason: None,
        };

        save_position_with_audit_on_conn(&mut conn, &position, &audit)
            .expect_err("chain failure must roll back the whole fill transaction");
        assert_eq!(table_count(&mut conn, "stock_position"), 0);
        assert_eq!(table_count(&mut conn, "order_audit"), 0);
        assert_eq!(table_count(&mut conn, "order_audit_chain"), 0);
    }

    fn br170_assignment() -> crate::data_gateway::PositionChainAssignment {
        crate::data_gateway::derive_position_chain(
            "TEST_CODE_600396",
            crate::data_gateway::GatewayBatch::Available {
                records: vec![crate::data_gateway::BoardMembershipRecord {
                    instrument_code: "TEST_CODE_600396".to_string(),
                    board_code: "TEST_CODE_INDUSTRY".to_string(),
                    board_name: "测试电力".to_string(),
                    kind: crate::data_gateway::BoardKind::Industry,
                }],
                evidence: crate::data_gateway::BatchEvidence {
                    provider: crate::magic_compat::ProviderId::Tdx,
                    source: "TEST_CODE_tdx-board-memberships".to_string(),
                    source_at: None,
                    observed_at: "2026-07-27T09:30:01+08:00".to_string(),
                    batch_id: "TEST_CODE_BR170_ATOMIC_BATCH".to_string(),
                },
            },
        )
        .expect("valid assignment batch")
        .expect("one assignment")
    }

    fn br170_position() -> NewStockPosition {
        NewStockPosition {
            code: "TEST_CODE_600396".to_string(),
            name: "原子开仓测试".to_string(),
            buy_date: "2026-07-27".to_string(),
            buy_price: 10.0,
            quantity: 100,
            status: "open".to_string(),
            st_type: None,
            chain_name: None,
        }
    }

    fn br170_audit() -> OrderAuditRecord<'static> {
        OrderAuditRecord {
            business_order_id: "TEST_ORDER_BR170_ATOMIC",
            source: "DatabaseTest",
            decision_basis: "TEST_CODE complete candidate assignment",
            side: "buy",
            code: "TEST_CODE_600396",
            requested_price: 10.0,
            execution_price: Some(10.0),
            quantity: 100,
            quote_observed_at: Some("2026-07-27T09:30:00+08:00"),
            outcome: "Filled",
            failure_reason: None,
        }
    }

    #[test]
    fn br170_open_fill_atomically_commits_audit_position_assignment_and_link() {
        let mut conn = isolated_connection();
        let assignment = br170_assignment();

        save_position_with_audit_and_assignment_on_conn(
            &mut conn,
            &br170_position(),
            &br170_audit(),
            &assignment,
        )
        .expect("atomic BR-170 open fill");

        assert_eq!(table_count(&mut conn, "stock_position"), 1);
        assert_eq!(table_count(&mut conn, "order_audit"), 1);
        assert_eq!(table_count(&mut conn, "order_audit_chain"), 1);
        assert_eq!(table_count(&mut conn, "position_chain_assignment"), 1);
        let linked = crate::database::position_chain::PositionChainStore::new(&mut conn)
            .linked("TEST_CODE_600396")
            .expect("linked assignment")
            .expect("linked position");
        assert_eq!(linked.assignment_id, assignment.assignment_id);
        assert_eq!(linked.board_name, "测试电力");
        validate_order_audit_chain(&mut conn).expect("order chain remains valid");
    }

    #[test]
    fn br170_assignment_failure_rolls_back_audit_position_and_link() {
        let mut conn = isolated_connection();
        diesel::sql_query(
            "CREATE TRIGGER test_fail_position_chain_assignment_insert
             BEFORE INSERT ON position_chain_assignment
             BEGIN SELECT RAISE(ABORT, 'TEST_CODE forced assignment failure'); END",
        )
        .execute(&mut conn)
        .expect("install assignment failure trigger");

        save_position_with_audit_and_assignment_on_conn(
            &mut conn,
            &br170_position(),
            &br170_audit(),
            &br170_assignment(),
        )
        .expect_err("assignment failure must roll back the full fill");

        assert_eq!(table_count(&mut conn, "stock_position"), 0);
        assert_eq!(table_count(&mut conn, "order_audit"), 0);
        assert_eq!(table_count(&mut conn, "order_audit_chain"), 0);
        assert_eq!(table_count(&mut conn, "position_chain_assignment"), 0);
    }
}
