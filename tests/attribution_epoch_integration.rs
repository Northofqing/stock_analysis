use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Text};
use stock_analysis::database::attribution_epochs::{AttributionEpochStore, EpochActivationRequest};
use stock_analysis::database::order_audit::OrderAuditRecord;
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
    drop(conn);
    database
        .record_order_audit(&OrderAuditRecord {
            business_order_id: plan_id,
            source: "PaperTrade",
            decision_basis: "TEST_CODE terminal",
            side: direction,
            code: "TEST_CODE_600001",
            requested_price: 10.0,
            execution_price: Some(10.0),
            quantity,
            quote_observed_at: Some(&format!(
                "{}T{}+08:00",
                &occurred_at[..10],
                &occurred_at[11..]
            )),
            outcome: "Filled",
            failure_reason: None,
        })
        .expect("TEST_CODE canonical terminal audit");
}

fn source_snapshot(database: &DatabaseManager) -> (String, String, String) {
    let mut conn = database.get_conn().expect("TEST_CODE snapshot connection");
    let paper = diesel::sql_query(
        "SELECT COALESCE(group_concat(value, '|'), '') AS value FROM (
            SELECT printf('%d,%s,%s,%s,%s,%.17g,%d,%s,%.17g,%s,%s,%s,%s,%s',
                          id,plan_id,code,name,direction,price,quantity,status,fill_price,
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
