use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Double, Nullable, Text};
use rusqlite::Connection as RawConnection;
use serde::Serialize;
use sha2::{Digest, Sha256};
use stock_analysis::database::attribution_epochs::{AttributionEpochStore, EpochActivationRequest};
use stock_analysis::database::attribution_reports::{
    AttributionDatabaseAccess, AttributionDatabaseSession,
};
use stock_analysis::database::DatabaseManager;
use stock_analysis::performance::attribution_epoch::{
    AttributionEpochSelector, EpochActivationSource,
};

#[derive(QueryableByName)]
struct CountRow {
    #[diesel(sql_type = BigInt)]
    count: i64,
}

#[derive(QueryableByName)]
struct SnapshotRow {
    #[diesel(sql_type = Text)]
    value: String,
}

#[derive(QueryableByName)]
struct HashRow {
    #[diesel(sql_type = Text)]
    record_hash: String,
}

#[derive(Serialize)]
struct FixedAuditRow {
    id: i64,
    business_order_id: String,
    source: String,
    decision_basis: String,
    side: String,
    code: String,
    requested_price: f64,
    execution_price: Option<f64>,
    quantity: i64,
    quote_observed_at: Option<String>,
    outcome: String,
    failure_reason: Option<String>,
    created_at: String,
}

fn audit_hash(previous_hash: &str, row: &FixedAuditRow) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"BR086_ORDER_AUDIT_V1\0");
    hasher.update(previous_hash.as_bytes());
    hasher.update(b"\0");
    hasher.update(serde_json::to_vec(row).expect("TEST_CODE canonical audit JSON"));
    hex::encode(hasher.finalize())
}

fn isolated_path() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("TEST_CODE clock")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "TEST_CODE_attribution_epoch_integration_{}_{}.sqlite",
        std::process::id(),
        nonce
    ))
}

fn create_test_code_source_schema(path: &std::path::Path) {
    let connection = RawConnection::open(path).expect("TEST_CODE create isolated source database");
    connection
        .execute_batch(
            "CREATE TABLE paper_trades (
                id INTEGER PRIMARY KEY, plan_id TEXT NOT NULL UNIQUE,
                code TEXT NOT NULL, name TEXT NOT NULL, direction TEXT NOT NULL,
                price REAL NOT NULL, quantity INTEGER NOT NULL, status TEXT NOT NULL,
                fill_price REAL, not_fill_reason TEXT, virtual_reason TEXT NOT NULL,
                account_mode TEXT NOT NULL, data_mode TEXT NOT NULL,
                ts TEXT NOT NULL, updated_at TEXT NOT NULL
             );
             CREATE TABLE order_audit (
                id INTEGER PRIMARY KEY, business_order_id TEXT NOT NULL,
                source TEXT NOT NULL, decision_basis TEXT NOT NULL, side TEXT NOT NULL,
                code TEXT NOT NULL, requested_price REAL NOT NULL, execution_price REAL,
                quantity INTEGER NOT NULL, quote_observed_at TEXT, outcome TEXT NOT NULL,
                failure_reason TEXT, created_at TEXT NOT NULL
             );
             CREATE TABLE order_audit_chain (
                order_audit_id INTEGER PRIMARY KEY, previous_hash TEXT NOT NULL,
                record_hash TEXT NOT NULL UNIQUE, created_at TEXT NOT NULL
             );
             CREATE TABLE stock_daily (
                id INTEGER PRIMARY KEY, code TEXT NOT NULL, date TEXT NOT NULL,
                close REAL, data_source TEXT, created_at TEXT NOT NULL, updated_at TEXT NOT NULL
             );",
        )
        .expect("TEST_CODE source schema");
}

fn open_test_code_session(path: &std::path::Path) -> AttributionDatabaseSession {
    AttributionDatabaseSession::open(path, AttributionDatabaseAccess::AppendOnly)
        .expect("TEST_CODE append-only attribution session")
}

fn cleanup_test_code_database(path: &std::path::Path) {
    for candidate in [
        path.to_path_buf(),
        PathBuf::from(format!("{}-wal", path.display())),
        PathBuf::from(format!("{}-shm", path.display())),
    ] {
        if candidate.exists() {
            std::fs::remove_file(candidate).expect("TEST_CODE remove exact temporary database");
        }
    }
}

#[test]
fn append_only_session_commit_uses_descriptor_attested_read_back() {
    let path = isolated_path();
    create_test_code_source_schema(&path);
    let session = open_test_code_session(&path);
    let request = EpochActivationRequest {
        source: EpochActivationSource::Cli,
        invoked_at: chrono::DateTime::parse_from_rfc3339("2026-08-28T15:40:00+08:00").unwrap(),
    };

    let receipt = AttributionEpochStore::new(session.database())
        .activate_once(request)
        .expect("TEST_CODE append-only session commits through attested read-back");
    assert_eq!(
        AttributionEpochStore::new(session.database())
            .verify_active()
            .expect("TEST_CODE retained session receipt"),
        receipt
    );

    drop(session);
    cleanup_test_code_database(&path);
}

fn append_pair(
    database: &DatabaseManager,
    id: i64,
    plan_id: &str,
    direction: &str,
    quantity: i64,
    occurred_at: &str,
) {
    let occurred = chrono::NaiveDateTime::parse_from_str(occurred_at, "%Y-%m-%d %H:%M:%S")
        .expect("TEST_CODE fixed paper timestamp");
    let quote_observed_at = occurred
        .and_utc()
        .with_timezone(&chrono::FixedOffset::east_opt(8 * 60 * 60).unwrap())
        .to_rfc3339();
    let terminal_at = (occurred + chrono::Duration::seconds(1))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    let mut conn = database.get_conn().expect("TEST_CODE source connection");
    diesel::sql_query(
        "INSERT INTO paper_trades
         (id,plan_id,code,name,direction,price,quantity,status,fill_price,not_fill_reason,
          virtual_reason,account_mode,data_mode,ts,updated_at)
         VALUES (?,?,'TEST_CODE_600001','TEST_CODE company',?,10.0,?,'Filled',10.0,NULL,
                 'TEST_CODE activation','Normal','Full',?,?)",
    )
    .bind::<BigInt, _>(id)
    .bind::<Text, _>(plan_id)
    .bind::<Text, _>(direction)
    .bind::<BigInt, _>(quantity)
    .bind::<Text, _>(occurred_at)
    .bind::<Text, _>(occurred_at)
    .execute(&mut conn)
    .expect("TEST_CODE paper source row");
    let previous_hash = diesel::sql_query(
        "SELECT record_hash FROM order_audit_chain ORDER BY order_audit_id DESC LIMIT 1",
    )
    .get_result::<HashRow>(&mut conn)
    .optional()
    .unwrap()
    .map_or_else(
        || "BR086_ORDER_AUDIT_GENESIS_V1".to_owned(),
        |row| row.record_hash,
    );
    let audit = FixedAuditRow {
        id,
        business_order_id: plan_id.to_owned(),
        source: "PaperTrade".to_owned(),
        decision_basis: "TEST_CODE activation".to_owned(),
        side: direction.to_owned(),
        code: "TEST_CODE_600001".to_owned(),
        requested_price: 10.0,
        execution_price: Some(10.0),
        quantity,
        quote_observed_at: Some(quote_observed_at),
        outcome: "Filled".to_owned(),
        failure_reason: None,
        created_at: terminal_at,
    };
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
    .execute(&mut conn)
    .expect("TEST_CODE fixed terminal audit");
    let record_hash = audit_hash(&previous_hash, &audit);
    diesel::sql_query(
        "INSERT INTO order_audit_chain
         (order_audit_id,previous_hash,record_hash,created_at) VALUES (?,?,?,?)",
    )
    .bind::<BigInt, _>(audit.id)
    .bind::<Text, _>(&previous_hash)
    .bind::<Text, _>(&record_hash)
    .bind::<Text, _>(&audit.created_at)
    .execute(&mut conn)
    .expect("TEST_CODE fixed canonical audit chain");
}

fn source_snapshot(database: &DatabaseManager) -> (String, String, String) {
    let mut conn = database.get_conn().expect("TEST_CODE snapshot connection");
    let paper = diesel::sql_query(
        "SELECT COALESCE(group_concat(value, '|'), '') AS value FROM (
            SELECT printf('%d,%s,%s,%s,%s,%.17g,%d,%s,%.17g,%s,%s,%s,%s,%s,%s',
                          id,plan_id,code,name,direction,price,quantity,status,fill_price,
                          CASE WHEN not_fill_reason IS NULL THEN '<NULL>' ELSE hex(not_fill_reason) END,
                          virtual_reason,account_mode,data_mode,CAST(ts AS TEXT),CAST(updated_at AS TEXT)) AS value
            FROM paper_trades ORDER BY id
         )",
    )
    .get_result::<SnapshotRow>(&mut conn)
    .unwrap()
    .value;
    let audit = diesel::sql_query(
        "SELECT COALESCE(group_concat(value, '|'), '') AS value FROM (
            SELECT printf('%d,%s,%s,%s,%s,%s,%.17g,%.17g,%d,%s,%s,%s,%s',
                          id,business_order_id,source,decision_basis,side,code,requested_price,
                          execution_price,quantity,quote_observed_at,outcome,
                          COALESCE(failure_reason,''),CAST(created_at AS TEXT)) AS value
            FROM order_audit ORDER BY id
         )",
    )
    .get_result::<SnapshotRow>(&mut conn)
    .unwrap()
    .value;
    let chain = diesel::sql_query(
        "SELECT COALESCE(group_concat(value, '|'), '') AS value FROM (
            SELECT printf('%d,%s,%s,%s',order_audit_id,previous_hash,record_hash,
                          CAST(created_at AS TEXT)) AS value
            FROM order_audit_chain ORDER BY order_audit_id
         )",
    )
    .get_result::<SnapshotRow>(&mut conn)
    .unwrap()
    .value;
    (paper, audit, chain)
}

#[test]
fn activation_concurrency_freezes_one_receipt_without_source_mutation() {
    let path = isolated_path();
    DatabaseManager::init(Some(path.clone())).expect("TEST_CODE isolated database initialization");
    let database = DatabaseManager::get();
    append_pair(
        database,
        1,
        "TEST_CODE_PLAN_BUY",
        "buy",
        200,
        "2026-08-27 10:00:00",
    );
    append_pair(
        database,
        2,
        "TEST_CODE_PLAN_SELL",
        "sell",
        100,
        "2026-08-28 10:00:00",
    );
    let source_before = source_snapshot(database);
    let barrier = std::sync::Barrier::new(2);
    let receipts = std::thread::scope(|scope| {
        let handles = (0..2)
            .map(|_| {
                scope.spawn(|| {
                    barrier.wait();
                    AttributionEpochStore::new(database)
                        .activate_once(EpochActivationRequest {
                            source: EpochActivationSource::Monitor,
                            invoked_at: chrono::DateTime::parse_from_rfc3339(
                                "2026-08-28T15:40:00+08:00",
                            )
                            .unwrap(),
                        })
                        .expect("TEST_CODE concurrent activation")
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(receipts[0], receipts[1]);
    assert_eq!(source_snapshot(database), source_before);

    let mut conn = database.get_conn().unwrap();
    let receipts =
        diesel::sql_query("SELECT COUNT(*) AS count FROM attribution_sample_epoch_receipt")
            .get_result::<CountRow>(&mut conn)
            .unwrap()
            .count;
    assert_eq!(receipts, 1);
}

#[test]
fn activation_boundary_keeps_real_legacy_t_plus_one_fixture_and_active_exact_fail_closed() {
    let path = isolated_path();
    create_test_code_source_schema(&path);
    let session = open_test_code_session(&path);
    let database = session.database();

    // This is the historical defect shape: the same-day sell consumes the
    // legacy buy, but activation may still freeze its quantity-only carry.
    append_pair(
        database,
        510,
        "TEST_CODE_LEGACY_BUY_510",
        "buy",
        400,
        "2026-08-28 10:00:00",
    );
    append_pair(
        database,
        520,
        "TEST_CODE_LEGACY_SAME_DAY_SELL_520",
        "sell",
        100,
        "2026-08-28 14:00:00",
    );
    let source_before_preview = source_snapshot(database);
    let request = EpochActivationRequest {
        source: EpochActivationSource::Cli,
        invoked_at: chrono::DateTime::parse_from_rfc3339("2026-08-28T15:40:00+08:00").unwrap(),
    };
    let preview = AttributionEpochStore::preview_activation_at_path(&path, &request)
        .expect("TEST_CODE reset-sample preview");
    assert_eq!(preview.paper_trade_high_water, 520);
    assert_eq!(preview.carry.len(), 1);
    assert_eq!(preview.carry[0].code, "TEST_CODE_600001");
    assert_eq!(preview.carry[0].quantity, 300);
    assert_eq!(source_snapshot(database), source_before_preview);

    let receipt = AttributionEpochStore::new(database)
        .activate_once(request)
        .expect("TEST_CODE activation isolates the known legacy T+1 defect");
    assert_eq!(receipt.paper_trade_high_water, 520);
    assert_eq!(receipt.carry_total_quantity, 300);
    assert_eq!(source_snapshot(database), source_before_preview);

    // All boundary-after facts are fixture-owned.  They deliberately consume
    // the carry to zero before the independent, legal new lifecycle begins.
    append_pair(
        database,
        530,
        "TEST_CODE_OVERLAP_BUY_530",
        "buy",
        200,
        "2026-08-31 10:00:00",
    );
    append_pair(
        database,
        540,
        "TEST_CODE_MIXED_EXIT_540",
        "sell",
        400,
        "2026-09-01 10:00:00",
    );
    append_pair(
        database,
        550,
        "TEST_CODE_TERMINAL_CARRY_EXIT_550",
        "sell",
        100,
        "2026-09-01 14:00:00",
    );
    append_pair(
        database,
        560,
        "TEST_CODE_FRESH_BUY_560",
        "buy",
        100,
        "2026-09-02 10:00:00",
    );
    append_pair(
        database,
        570,
        "TEST_CODE_FRESH_SELL_570",
        "sell",
        100,
        "2026-09-03 10:00:00",
    );
    let retry = AttributionEpochStore::new(database)
        .activate_once(EpochActivationRequest {
            source: EpochActivationSource::Monitor,
            invoked_at: chrono::DateTime::parse_from_rfc3339("2026-09-03T15:40:00+08:00").unwrap(),
        })
        .expect("TEST_CODE post-boundary retry verifies only the frozen prefix");
    assert_eq!(retry, receipt);
    assert!(matches!(
        AttributionEpochStore::new(database)
            .load_selector(&AttributionEpochSelector::Active),
        Ok(stock_analysis::database::attribution_epochs::ResolvedAttributionEpoch::Epoch(found))
            if found == receipt
    ));

    // The tamper is constrained to this one temporary TEST_CODE database. A
    // bad retained tail must reject both current and exact reads, never fall
    // back to legacy or an earlier receipt.
    let mut conn = database.get_conn().expect("TEST_CODE tamper connection");
    diesel::sql_query("DROP TRIGGER trg_attribution_sample_epoch_receipt_chain_no_update")
        .execute(&mut conn)
        .expect("TEST_CODE permit isolated retained-tail tamper");
    diesel::sql_query(
        "UPDATE attribution_sample_epoch_receipt_chain SET record_hash=? WHERE epoch_receipt_id=1",
    )
    .bind::<Text, _>("f".repeat(64))
    .execute(&mut conn)
    .expect("TEST_CODE isolated bad receipt tail");
    drop(conn);

    for selector in [
        AttributionEpochSelector::Active,
        AttributionEpochSelector::Exact(receipt.epoch_id.clone()),
    ] {
        let error = AttributionEpochStore::new(database)
            .load_selector(&selector)
            .expect_err("TEST_CODE Active/Exact retained-tail tamper must fail closed");
        assert_eq!(error.reason_code(), "attribution_epoch_integrity_failed");
    }
    drop(session);
    cleanup_test_code_database(&path);
}

#[test]
fn retained_epoch_tamper_matrix_rejects_active_and_exact_without_legacy_fallback() {
    for case in [
        "receipt_tail",
        "carry_item_hash",
        "attempt_chain",
        "receipt_retention",
        "receipt_sequence",
        "canonical_trigger",
    ] {
        let path = isolated_path();
        create_test_code_source_schema(&path);
        let session = open_test_code_session(&path);
        let database = session.database();
        append_pair(
            database,
            510,
            "TEST_CODE_TAMPER_BUY_510",
            "buy",
            400,
            "2026-08-28 10:00:00",
        );
        append_pair(
            database,
            520,
            "TEST_CODE_TAMPER_SELL_520",
            "sell",
            100,
            "2026-08-28 14:00:00",
        );
        let receipt = AttributionEpochStore::new(database)
            .activate_once(EpochActivationRequest {
                source: EpochActivationSource::Cli,
                invoked_at: chrono::DateTime::parse_from_rfc3339("2026-08-28T15:40:00+08:00")
                    .unwrap(),
            })
            .expect("TEST_CODE independent tamper fixture activation");
        let mut conn = database
            .get_conn()
            .expect("TEST_CODE isolated tamper connection");
        match case {
            "receipt_tail" => {
                diesel::sql_query(
                    "DROP TRIGGER trg_attribution_sample_epoch_receipt_chain_no_update",
                )
                .execute(&mut conn)
                .unwrap();
                diesel::sql_query(
                    "UPDATE attribution_sample_epoch_receipt_chain SET record_hash=? WHERE id=1",
                )
                .bind::<Text, _>("f".repeat(64))
                .execute(&mut conn)
                .unwrap();
            }
            "carry_item_hash" => {
                diesel::sql_query("DROP TRIGGER trg_attribution_legacy_carry_item_no_update")
                    .execute(&mut conn)
                    .unwrap();
                diesel::sql_query(
                    "UPDATE attribution_legacy_carry_item SET item_hash=? WHERE id=1",
                )
                .bind::<Text, _>("f".repeat(64))
                .execute(&mut conn)
                .unwrap();
            }
            "attempt_chain" => {
                diesel::sql_query("DROP TRIGGER trg_attribution_epoch_attempt_chain_no_update")
                    .execute(&mut conn)
                    .unwrap();
                diesel::sql_query(
                    "UPDATE attribution_epoch_attempt_chain SET record_hash=? WHERE id=1",
                )
                .bind::<Text, _>("f".repeat(64))
                .execute(&mut conn)
                .unwrap();
            }
            "receipt_retention" => {
                diesel::sql_query("DROP TRIGGER trg_attribution_sample_epoch_receipt_no_update")
                    .execute(&mut conn)
                    .unwrap();
                diesel::sql_query(
                    "UPDATE attribution_sample_epoch_receipt SET retention_deadline=created_at WHERE id=1",
                )
                .execute(&mut conn)
                .unwrap();
            }
            "receipt_sequence" => {
                diesel::sql_query(
                    "UPDATE sqlite_sequence SET seq=0 WHERE name='attribution_sample_epoch_receipt'",
                )
                .execute(&mut conn)
                .unwrap();
            }
            "canonical_trigger" => {
                diesel::sql_query("DROP TRIGGER trg_attribution_epoch_attempt_chain_no_delete")
                    .execute(&mut conn)
                    .unwrap();
            }
            _ => unreachable!("TEST_CODE fixed tamper case"),
        }
        drop(conn);
        for selector in [
            AttributionEpochSelector::Active,
            AttributionEpochSelector::Exact(receipt.epoch_id.clone()),
        ] {
            let error = AttributionEpochStore::new(database)
                .load_selector(&selector)
                .expect_err("TEST_CODE retained tamper must never fall back");
            assert_eq!(
                error.reason_code(),
                "attribution_epoch_integrity_failed",
                "{case}"
            );
        }
        drop(session);
        cleanup_test_code_database(&path);
    }
}
