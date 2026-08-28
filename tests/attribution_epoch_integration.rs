use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Double, Nullable, Text};
use serde::Serialize;
use sha2::{Digest, Sha256};
use stock_analysis::database::attribution_epochs::{AttributionEpochStore, EpochActivationRequest};
use stock_analysis::database::DatabaseManager;
use stock_analysis::performance::attribution_epoch::EpochActivationSource;

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
    DatabaseManager::init(Some(path)).expect("TEST_CODE isolated database initialization");
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
