use super::*;
use crate::data_gateway::grpc_source::GrpcSource;
use crate::data_gateway::GatewayBatch;
use crate::database::data_acquisition_audit::read_acquisition_in_transaction;
use crate::grpc_client::client::board_loopback_fixture::spawn_dragon_tiger_success_loopback;
use crate::market_domain::{DragonTigerSide, Exchange, ProviderId};
use crate::pipeline::chain_analysis::preparation::{DragonTigerObservationClock, PreparationStage};
use rusqlite::{params, Connection, OpenFlags};
use std::cell::Cell;
use std::time::Duration;

const OWNER_V10_FIRST: &str = "TEST_CODE_DRAGON_TIGER_OWNER_FIRST";
const OWNER_V10_REOPENED: &str = "TEST_CODE_DRAGON_TIGER_OWNER_REOPENED";
const DRAGON_TIGER_RECORDS: &str = r#"[{
  "exchange":"Shanghai","code":"TEST_CODE_600001",
  "ranking_net_amount_yuan":125000,
  "disclosures":[{
    "entry_id":"TEST_CODE_LHB_ENTRY_一","trade_id":"TEST_CODE_LHB_TRADE_001",
    "reason":"TEST_CODE_REASON  ","buy_amount_yuan":200000.5,
    "sell_amount_yuan":75000.25,"net_amount_yuan":125000.25,
    "turnover_rate_pct":8.75,"seats":[
      {"side":"Buy","rank":1,"seat_name":"TEST_CODE_SEAT_BUY",
       "amount_yuan":200000.5,"buy_amount_yuan":200000.5,
       "sell_amount_yuan":null,"net_amount_yuan":200000.5},
      {"side":"Sell","rank":2,"seat_name":"TEST_CODE_SEAT_SELL",
       "amount_yuan":75000.25,"buy_amount_yuan":null,
       "sell_amount_yuan":75000.25,"net_amount_yuan":-75000.25}
    ]
  }]
}]"#;
const CHANGED_RECORDS: &str = r#"[{
  "exchange":"Shanghai","code":"TEST_CODE_CHANGED_SOURCE",
  "ranking_net_amount_yuan":999999,"disclosures":[]
}]"#;

struct DragonTigerClock {
    now: UtcMicros,
    request_observed_at: DateTime<chrono::FixedOffset>,
    request_calls: Cell<usize>,
    cache_calls: Cell<usize>,
}

impl ConceptEffectClock for DragonTigerClock {
    fn now(&self) -> UtcMicros {
        self.now
    }
}

impl PositionObservationClock for DragonTigerClock {
    fn cache_observation(&self) -> PositionCacheObservation {
        self.cache_calls.set(self.cache_calls.get() + 1);
        panic!("TEST_CODE v10 must reload the sealed position parent")
    }
}

impl DragonTigerObservationClock for DragonTigerClock {
    fn dragon_tiger_request_observation(&self) -> DateTime<chrono::FixedOffset> {
        self.request_calls.set(self.request_calls.get() + 1);
        self.request_observed_at
    }
}

fn seed_position_source(fixture: &V2BusinessFixture) {
    fixture.execute(
        "CREATE TABLE stock_position (
           id INTEGER PRIMARY KEY AUTOINCREMENT, code TEXT NOT NULL, name TEXT NOT NULL,
           buy_date TEXT NOT NULL, buy_price REAL NOT NULL, quantity INTEGER NOT NULL,
           status TEXT NOT NULL, sell_date TEXT, sell_price REAL, return_rate REAL,
           created_at TEXT NOT NULL, updated_at TEXT NOT NULL, chain_name TEXT, st_type TEXT,
           UNIQUE(code,buy_date));
         INSERT INTO stock_position VALUES
           (1,'TEST_CODE_CLUSTER_STOCK_A','成员','2026-07-21',10.0,100,'open',NULL,NULL,1.5,
            '2026-07-21 09:01:02','2026-07-21 14:01:02','TEST_CODE原链',NULL),
           (2,'TEST_CODE_600001','缺失','2026-07-20',20.0,200,'open',NULL,NULL,NULL,
            '2026-07-20 09:02:03','2026-07-21 14:02:03',NULL,'ST'),
           (3,'TEST_CODE_CLUSTER_POS_OTHER','无关','2026-07-19',30.0,100,'open',NULL,NULL,-2.0,
            '2026-07-19 09:03:04','2026-07-21 14:03:04',NULL,'*ST');
         INSERT INTO stock_concepts(code,concepts,updated_at) VALUES
           ('TEST_CODE_CLUSTER_POS_OTHER','[\"TEST_CODE_CLUSTER_Z_OTHER\"]',
            '2026-07-21 14:05:00');",
    );
}

fn assert_complete_batch(batch: &GatewayBatch<crate::data_gateway::DragonTigerStockReview>) {
    assert_eq!(batch.evidence().provider, ProviderId::Eastmoney);
    assert_eq!(batch.evidence().source, "TEST_CODE_LHB_SOURCE_FIRST");
    assert_eq!(
        batch.evidence().source_at.as_deref(),
        Some("2026-07-22T15:30:00+08:00")
    );
    assert_eq!(batch.evidence().observed_at, "2026-07-22T15:31:02+08:00");
    assert_eq!(batch.evidence().batch_id, "TEST_CODE_LHB_BATCH_FIRST");
    assert_eq!(batch.records().len(), 1);
    let stock = &batch.records()[0];
    assert_eq!(stock.exchange, Exchange::Shanghai);
    assert_eq!(stock.code, "TEST_CODE_600001");
    assert_eq!(
        stock.ranking_net_amount_yuan.to_bits(),
        125000.0_f64.to_bits()
    );
    assert_eq!(stock.disclosures.len(), 1);
    let disclosure = &stock.disclosures[0];
    assert_eq!(disclosure.entry_id, "TEST_CODE_LHB_ENTRY_一");
    assert_eq!(disclosure.trade_id, "TEST_CODE_LHB_TRADE_001");
    assert_eq!(disclosure.reason.as_deref(), Some("TEST_CODE_REASON  "));
    assert_eq!(disclosure.buy_amount_yuan, Some(200000.5));
    assert_eq!(disclosure.sell_amount_yuan, Some(75000.25));
    assert_eq!(disclosure.net_amount_yuan, Some(125000.25));
    assert_eq!(disclosure.turnover_rate_pct, Some(8.75));
    assert_eq!(disclosure.seats.len(), 2);
    assert_eq!(disclosure.seats[0].side, DragonTigerSide::Buy);
    assert_eq!(disclosure.seats[0].rank, 1);
    assert_eq!(disclosure.seats[0].seat_name, "TEST_CODE_SEAT_BUY");
    assert_eq!(disclosure.seats[0].amount_yuan, 200000.5);
    assert_eq!(disclosure.seats[0].buy_amount_yuan, Some(200000.5));
    assert_eq!(disclosure.seats[0].sell_amount_yuan, None);
    assert_eq!(disclosure.seats[0].net_amount_yuan, Some(200000.5));
    assert_eq!(disclosure.seats[1].side, DragonTigerSide::Sell);
    assert_eq!(disclosure.seats[1].rank, 2);
    assert_eq!(disclosure.seats[1].seat_name, "TEST_CODE_SEAT_SELL");
    assert_eq!(disclosure.seats[1].amount_yuan, 75000.25);
    assert_eq!(disclosure.seats[1].buy_amount_yuan, None);
    assert_eq!(disclosure.seats[1].sell_amount_yuan, Some(75000.25));
    assert_eq!(disclosure.seats[1].net_amount_yuan, Some(-75000.25));
}

async fn fixture_with_completed_v9_parent(
    queries: &crate::data_gateway::grpc_source::ConnectedBoardQueries,
    run_id: &str,
) -> (
    V2BusinessFixture,
    Vec<crate::market_data::TopStock>,
    crate::monitor::push_job::LocalChainPostCloseConfig,
    crate::monitor::push_job::IntentId,
    u64,
) {
    let mut fixture = V2BusinessFixture::new();
    let (stocks, config, intent, head) =
        populate_completed_v9_parent(&mut fixture, queries, run_id).await;
    (fixture, stocks, config, intent, head)
}

// Borrowed ownership lets a caller retain the test directory through async
// failure cleanup and join its servers before releasing the database root.
async fn populate_completed_v9_parent(
    fixture: &mut V2BusinessFixture,
    queries: &crate::data_gateway::grpc_source::ConnectedBoardQueries,
    run_id: &str,
) -> (
    Vec<crate::market_data::TopStock>,
    crate::monitor::push_job::LocalChainPostCloseConfig,
    crate::monitor::push_job::IntentId,
    u64,
) {
    fixture.install_v2();
    fixture
        .chain_post_close()
        .migrate_schema_v2_to_v3()
        .unwrap();
    cluster_tests::install_business_rows(fixture);
    fixture
        .chain_post_close()
        .migrate_schema_v3_to_v4()
        .unwrap();
    install_br159(fixture);
    fixture
        .chain_post_close()
        .migrate_schema_v4_to_v5()
        .unwrap();
    fixture
        .chain_post_close()
        .migrate_schema_v5_to_v6()
        .unwrap();
    fixture
        .chain_post_close()
        .migrate_schema_v6_to_v7()
        .unwrap();
    seed_position_source(fixture);
    fixture
        .chain_post_close()
        .migrate_schema_v7_to_v8()
        .unwrap();
    fixture
        .chain_post_close()
        .migrate_schema_v8_to_v9()
        .unwrap();
    let stocks = cluster_tests::cluster_stocks();
    let config = local_config(BUILD_A);
    let context =
        build_single_user_local_chain_post_close_context(&config, run_input(run_id)).unwrap();
    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let lease = local
        .acquire_run(
            context,
            fixed_input(stocks.clone()),
            lease_request(OWNER_A, 1_000_000, 1_000_000_000, None),
        )
        .unwrap();
    let intent = lease.intent_id().clone();
    let v9_clock = PositionsClock {
        now: UtcMicros::try_new(micros("2026-07-21T15:33:00+08:00")).unwrap(),
        observation: Some((
            DateTime::parse_from_rfc3339("2026-07-21T15:33:00+08:00").unwrap(),
            DateTime::parse_from_rfc3339("2026-07-14T15:33:00+08:00").unwrap(),
        )),
        cache_calls: Cell::new(0),
    };
    let mut io = local
        .position_concept_rpc_preparation_io_v9(
            lease,
            queries,
            &v9_clock,
            FixedClusterConfiguration::resolve(Some("2")),
        )
        .unwrap();
    let stopped = prepare_chain_analysis_with_io(
        NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
        stocks.clone(),
        None,
        &mut io,
    )
    .await
    .expect_err("TEST_CODE v9 parent must stop at DragonTiger");
    assert!(
        matches!(
            stopped.downcast_ref::<PreparationStop>(),
            Some(PreparationStop::StageNotMigrated {
                next: UnmigratedStage::DragonTiger
            })
        ),
        "TEST_CODE v9 actual stop: {stopped:#?}"
    );
    drop(io);
    let v9_head = local.inspect_run(&intent).unwrap().head_version();
    drop(local);
    (stocks, config, intent, v9_head)
}

#[path = "chain_post_close_macro_tests.rs"]
mod macro_tests;
#[derive(Debug, PartialEq)]
struct DragonTigerFactSnapshot {
    catalog: Vec<Vec<rusqlite::types::Value>>,
    occurrences: Vec<Vec<rusqlite::types::Value>>,
    begins: Vec<Vec<rusqlite::types::Value>>,
    results: Vec<Vec<rusqlite::types::Value>>,
    statuses: Vec<Vec<rusqlite::types::Value>>,
    errors: Vec<Vec<rusqlite::types::Value>>,
    finals: Vec<Vec<rusqlite::types::Value>>,
    audits: Vec<Vec<rusqlite::types::Value>>,
    audit_chain: Vec<Vec<rusqlite::types::Value>>,
    layout_objects: Vec<Vec<rusqlite::types::Value>>,
}

fn dragon_tiger_fact_snapshot(connection: &Connection) -> DragonTigerFactSnapshot {
    DragonTigerFactSnapshot {
        catalog: all_rows(
            connection,
            "SELECT name,type,tbl_name,CAST(sql AS BLOB) FROM sqlite_schema \
             WHERE name LIKE 'chain_post_close_%' ORDER BY name",
        ),
        occurrences: all_rows(
            connection,
            "SELECT * FROM chain_post_close_dragon_tiger_occurrences",
        ),
        begins: all_rows(
            connection,
            "SELECT * FROM chain_post_close_dragon_tiger_attempt_begins ORDER BY attempt_ordinal",
        ),
        results: all_rows(
            connection,
            "SELECT * FROM chain_post_close_dragon_tiger_attempt_results ORDER BY attempt_ordinal",
        ),
        statuses: all_rows(
            connection,
            "SELECT * FROM chain_post_close_dragon_tiger_status_materials ORDER BY attempt_ordinal",
        ),
        errors: all_rows(
            connection,
            "SELECT * FROM chain_post_close_dragon_tiger_error_materials",
        ),
        finals: all_rows(
            connection,
            "SELECT * FROM chain_post_close_dragon_tiger_finals",
        ),
        audits: all_rows(
            connection,
            "SELECT * FROM data_acquisition_audit ORDER BY id",
        ),
        audit_chain: all_rows(
            connection,
            "SELECT * FROM data_acquisition_audit_chain ORDER BY acquisition_audit_id",
        ),
        layout_objects: all_rows(
            connection,
            "SELECT * FROM chain_post_close_layout_objects ORDER BY layout_version,name",
        ),
    }
}

fn corrupt_dragon_tiger_projection(
    connection: &Connection,
    intent: &IntentId,
) -> DragonTigerFactSnapshot {
    let before = dragon_tiger_fact_snapshot(connection);
    let transaction = connection.unchecked_transaction().unwrap();
    let trigger_sql: String = transaction
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type='trigger' \
             AND name='chain_post_close_dragon_tiger_finals_update'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let (original_bytes, original_length, original_sha): (Vec<u8>, i64, String) = transaction
        .query_row(
            "SELECT final_bytes,final_length,final_sha256 \
             FROM chain_post_close_dragon_tiger_finals WHERE intent_id=?1",
            [intent.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        original_length,
        i64::try_from(original_bytes.len()).unwrap()
    );
    assert_eq!(original_sha, raw_digest(&original_bytes).as_str());
    const ORIGINAL_PROJECTION: &[u8] = br#""TEST_CODE_600001":4623226492472524800"#;
    const CORRUPTED_PROJECTION: &[u8] = br#""TEST_CODE_600001":4623507967449235456"#;
    assert_eq!(ORIGINAL_PROJECTION.len(), CORRUPTED_PROJECTION.len());
    let offsets = original_bytes
        .windows(ORIGINAL_PROJECTION.len())
        .enumerate()
        .filter_map(|(offset, bytes)| (bytes == ORIGINAL_PROJECTION).then_some(offset))
        .collect::<Vec<_>>();
    assert_eq!(offsets.len(), 1);
    let mut corrupted_bytes = original_bytes.clone();
    let offset = offsets[0];
    corrupted_bytes[offset..offset + CORRUPTED_PROJECTION.len()]
        .copy_from_slice(CORRUPTED_PROJECTION);
    let original_json: serde_json::Value = serde_json::from_slice(&original_bytes).unwrap();
    let corrupted_json: serde_json::Value = serde_json::from_slice(&corrupted_bytes).unwrap();
    let mut expected_json = original_json.clone();
    expected_json["projection"]["Available"]["lhb_bits"]["TEST_CODE_600001"] =
        serde_json::Value::from(0x402a_0000_0000_0000_u64);
    assert_eq!(corrupted_json, expected_json);
    assert_eq!(corrupted_json["gateway"], original_json["gateway"]);
    assert_eq!(
        original_json["projection"]["Available"]["lhb_bits"]["TEST_CODE_600001"],
        serde_json::Value::from(0x4029_0000_0000_0000_u64)
    );
    let corrupted_sha = raw_digest(&corrupted_bytes).as_str().to_owned();
    transaction
        .execute_batch("DROP TRIGGER chain_post_close_dragon_tiger_finals_update")
        .unwrap();
    assert_eq!(
        transaction
            .execute(
                "UPDATE chain_post_close_dragon_tiger_finals \
                 SET final_bytes=?1,final_length=?2,final_sha256=?3 WHERE intent_id=?4",
                params![
                    &corrupted_bytes,
                    i64::try_from(corrupted_bytes.len()).unwrap(),
                    &corrupted_sha,
                    intent.as_str()
                ],
            )
            .unwrap(),
        1
    );
    transaction.execute_batch(&trigger_sql).unwrap();
    transaction.commit().unwrap();
    let stored_corruption: (Vec<u8>, i64, String) = connection
        .query_row(
            "SELECT final_bytes,final_length,final_sha256 \
             FROM chain_post_close_dragon_tiger_finals WHERE intent_id=?1",
            [intent.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        stored_corruption,
        (corrupted_bytes, original_length, corrupted_sha,)
    );
    let after = dragon_tiger_fact_snapshot(connection);
    assert_eq!(&after.catalog, &before.catalog);
    assert_eq!(&after.occurrences, &before.occurrences);
    assert_eq!(&after.begins, &before.begins);
    assert_eq!(&after.results, &before.results);
    assert_eq!(&after.statuses, &before.statuses);
    assert_eq!(&after.errors, &before.errors);
    assert_eq!(&after.audits, &before.audits);
    assert_eq!(&after.audit_chain, &before.audit_chain);
    assert_eq!(&after.layout_objects, &before.layout_objects);
    assert_ne!(&after.finals, &before.finals);
    after
}

#[tokio::test]
async fn single_user_local_dragon_tiger_reopens_without_repeating_rpc_or_audit() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let (client, server) = spawn_dragon_tiger_success_loopback(
            DRAGON_TIGER_RECORDS.as_bytes(),
            "TEST_CODE_LHB_BATCH_FIRST",
            "TEST_CODE_LHB_SOURCE_FIRST",
        )
        .await;
        let source = GrpcSource::from_board_loopback_test_client(client);
        let queries = source.connected_board_queries().await.unwrap();
        let (mut fixture, stocks, config, intent, v9_head) =
            fixture_with_completed_v9_parent(&queries, "TEST_CODE_RUN_DRAGON_TIGER_V10").await;

        let v9_facts = v9_fact_rows(fixture.connection());
        assert_eq!(server.dragon_tiger_snapshot().len(), 0);
        let network_before_v10 = server.snapshot();
        let memberships_before_v10 = server.membership_snapshot();
        let old_audit_count = fixture.count("data_acquisition_audit");

        assert_eq!(
            fixture
                .chain_post_close()
                .migrate_schema_v9_to_v10()
                .unwrap()
                .schema_version(),
            10
        );
        let mut local = fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .unwrap();
        let lease = local
            .resume_run(
                &intent,
                lease_request(
                    OWNER_V10_FIRST,
                    68_300_000_000,
                    90_000_000_000,
                    Some(v9_head),
                ),
            )
            .unwrap();
        let first_clock = DragonTigerClock {
            now: UtcMicros::try_new(micros("2026-07-22T10:30:00+08:00")).unwrap(),
            request_observed_at: DateTime::parse_from_rfc3339("2026-07-22T10:30:00+08:00").unwrap(),
            request_calls: Cell::new(0),
            cache_calls: Cell::new(0),
        };
        let mut io = local
            .dragon_tiger_preparation_io_v10(
                lease,
                &queries,
                &first_clock,
                FixedClusterConfiguration::resolve(Some("2")),
                &source,
            )
            .unwrap();
        let stopped = prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks.clone(),
            None,
            &mut io,
        )
        .await
        .expect_err("TEST_CODE v10 must stop at Macro after DragonTiger");
        assert!(matches!(
            stopped.downcast_ref::<PreparationStop>(),
            Some(PreparationStop::StageNotMigrated {
                next: UnmigratedStage::Macro
            })
        ));
        let failure = stopped.downcast_ref::<PreparationFailure>().unwrap();
        assert_eq!(failure.stage(), PreparationStage::Macro);
        assert!(failure
            .completed_stages()
            .contains(&PreparationStage::DragonTiger));
        assert_eq!(
            failure.lhb_map()["TEST_CODE_600001"].to_bits(),
            12.5_f64.to_bits()
        );
        assert_eq!(failure.lhb_source().status(), &SourceStatus::Available);
        assert_eq!(
            failure.lhb_source().source(),
            Some("TEST_CODE_LHB_SOURCE_FIRST")
        );
        assert_eq!(
            failure.lhb_source().request_date(),
            NaiveDate::from_ymd_opt(2026, 7, 22)
        );
        assert_eq!(
            failure.lhb_source().request_observed_at(),
            Some("2026-07-22T10:30:00+08:00")
        );
        assert_eq!(first_clock.request_calls.get(), 1);
        assert_eq!(first_clock.cache_calls.get(), 0);
        drop(io);

        let recovered = local.inspect_dragon_tiger(&intent).unwrap();
        assert!(recovered.is_complete());
        assert_eq!(
            recovered.request_date(),
            NaiveDate::from_ymd_opt(2026, 7, 22).unwrap()
        );
        assert_eq!(recovered.request_observed_at(), "2026-07-22T10:30:00+08:00");
        assert_eq!(recovered.retry_policy(), (4, 1_000, 60_000, 200));
        assert_eq!(recovered.attempts().len(), 1);
        assert_complete_batch(recovered.batch().unwrap());
        assert_eq!(
            recovered.projection().unwrap().lhb_map()["TEST_CODE_600001"],
            12.5
        );
        let receipt = recovered.audit_receipt().unwrap().clone();
        let first_request_bytes = recovered.request_bytes().to_vec();
        let first_final_bytes = recovered.final_bytes().unwrap().to_vec();
        let first_result_bytes = recovered.attempts()[0].result_bytes().unwrap().to_vec();
        let first_head = local.inspect_run(&intent).unwrap().head_version();
        drop(local);

        let transaction = fixture.connection().unchecked_transaction().unwrap();
        let verified = read_acquisition_in_transaction(&transaction, &receipt).unwrap();
        let audit = verified.record();
        assert_eq!(audit.capability, "R-04");
        assert_eq!(audit.provider, "Eastmoney");
        assert_eq!(audit.source, "TEST_CODE_LHB_SOURCE_FIRST");
        // Independently calculated SHA-256 of the 54 legacy request bytes:
        // b"BR159_DATA_GATEWAY_REQUEST_V1\0R-04\02026-07-22:100:5000".
        assert_eq!(
            audit.request_hash,
            "f68a028995f72a4d228b9de67375ecd2bee7ccf1942a1de98c37a592043dd679"
        );
        assert_eq!(audit.source_at, Some("2026-07-22T15:30:00+08:00"));
        assert_eq!(audit.observed_at, "2026-07-22T15:31:02+08:00");
        assert_eq!(audit.batch_id, Some("TEST_CODE_LHB_BATCH_FIRST"));
        assert_eq!(audit.outcome, "available");
        assert_eq!(audit.request_count, 1);
        assert_eq!(audit.accepted_count, 1);
        assert_eq!(audit.rejected_count, 0);
        assert_eq!(audit.reason_code, "accepted");
        assert!(!audit.retryable);
        transaction.commit().unwrap();
        assert_eq!(fixture.count("data_acquisition_audit"), old_audit_count + 1);
        assert_eq!(
            all_rows(
                fixture.connection(),
                "SELECT capability FROM data_acquisition_audit WHERE capability='R-04'"
            )
            .len(),
            1
        );

        let first_network = server.snapshot();
        assert_eq!(first_network.dragon_tiger_requests.len(), 1);
        let request = &first_network.dragon_tiger_requests[0];
        assert_eq!(first_network.requests, network_before_v10.requests);
        assert_eq!(server.membership_snapshot(), memberships_before_v10);
        assert_eq!(request.date, "2026-07-22");
        assert_eq!(request.disclosure_limit, 100);
        assert_eq!(request.stock_limit, 5000);
        assert_eq!(request.payload_schema, "market.dragon_tiger");
        assert_eq!(request.payload_schema_version, 1);
        assert_eq!(
            request.payload_content_type,
            "application/json; charset=utf-8"
        );
        assert!(request.preferred_provider.is_empty());
        assert!(!request.allow_unadmitted);
        assert!(request.authorized);
        assert_eq!(request.request_bytes, first_request_bytes);
        assert!(!first_request_bytes
            .windows(b"TEST_CODE_BOARD_LOOPBACK_TOKEN".len())
            .any(|bytes| bytes == b"TEST_CODE_BOARD_LOOPBACK_TOKEN"));
        assert!(!first_result_bytes
            .windows(b"TEST_CODE_BOARD_LOOPBACK_TOKEN".len())
            .any(|bytes| bytes == b"TEST_CODE_BOARD_LOOPBACK_TOKEN"));
        assert_eq!(first_network.non_board_requests, 0);
        drop(queries);
        drop(source);
        assert_eq!(server.finish().await, first_network);

        fixture.reopen();
        let (changed_client, changed_server) = spawn_dragon_tiger_success_loopback(
            CHANGED_RECORDS.as_bytes(),
            "TEST_CODE_LHB_BATCH_CHANGED",
            "TEST_CODE_LHB_SOURCE_CHANGED",
        )
        .await;
        let changed_source = GrpcSource::from_board_loopback_test_client(changed_client);
        let changed_queries = changed_source.connected_board_queries().await.unwrap();
        let mut local = fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .unwrap();
        let lease = local
            .resume_run(
                &intent,
                lease_request(
                    OWNER_V10_REOPENED,
                    159_200_000_000,
                    180_000_000_000,
                    Some(first_head),
                ),
            )
            .unwrap();
        let changed_clock = DragonTigerClock {
            now: UtcMicros::try_new(micros("2026-07-23T11:45:00+08:00")).unwrap(),
            request_observed_at: DateTime::parse_from_rfc3339("2026-07-23T11:45:00+08:00").unwrap(),
            request_calls: Cell::new(0),
            cache_calls: Cell::new(0),
        };
        let mut reopened_io = local
            .dragon_tiger_preparation_io_v10(
                lease,
                &changed_queries,
                &changed_clock,
                FixedClusterConfiguration::resolve(Some("2")),
                &changed_source,
            )
            .unwrap();
        let reopened_stop = prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks,
            None,
            &mut reopened_io,
        )
        .await
        .expect_err("TEST_CODE reopened v10 must stop at Macro");
        assert!(matches!(
            reopened_stop.downcast_ref::<PreparationStop>(),
            Some(PreparationStop::StageNotMigrated {
                next: UnmigratedStage::Macro
            })
        ));
        let reopened_failure = reopened_stop.downcast_ref::<PreparationFailure>().unwrap();
        assert_eq!(reopened_failure.lhb_map()["TEST_CODE_600001"], 12.5);
        assert_eq!(
            reopened_failure.lhb_source().request_date(),
            NaiveDate::from_ymd_opt(2026, 7, 22)
        );
        drop(reopened_io);
        assert_eq!(changed_clock.request_calls.get(), 0);
        assert_eq!(changed_clock.cache_calls.get(), 0);

        let reopened = local.inspect_dragon_tiger(&intent).unwrap();
        assert_complete_batch(reopened.batch().unwrap());
        assert_eq!(reopened.request_observed_at(), "2026-07-22T10:30:00+08:00");
        assert_eq!(reopened.audit_receipt(), Some(&receipt));
        assert_eq!(reopened.request_bytes(), first_request_bytes);
        assert_eq!(reopened.final_bytes().unwrap(), first_final_bytes);
        assert_eq!(
            reopened.attempts()[0].result_bytes().unwrap(),
            first_result_bytes
        );
        assert_eq!(local.inspect_run(&intent).unwrap().lease_generation(), 3);
        drop(local);
        assert_eq!(v9_fact_rows(fixture.connection()), v9_facts);
        assert_eq!(fixture.count("data_acquisition_audit"), old_audit_count + 1);
        assert_eq!(
            all_rows(
                fixture.connection(),
                "SELECT capability FROM data_acquisition_audit WHERE capability='R-04'"
            )
            .len(),
            1
        );
        let changed_network = changed_server.snapshot();
        assert!(changed_network.requests.is_empty());
        assert!(changed_network.dragon_tiger_requests.is_empty());
        assert!(changed_server.membership_snapshot().is_empty());
        assert_eq!(changed_network.non_board_requests, 0);
        drop(changed_queries);
        drop(changed_source);
        assert_eq!(changed_server.finish().await, changed_network);
    })
    .await
    .expect("TEST_CODE DragonTiger v10 scenario deadline");
}

#[tokio::test]
async fn dragon_tiger_rejects_a_self_consistent_final_with_a_projection_not_derived_from_terminal()
{
    tokio::time::timeout(Duration::from_secs(60), async {
        let (client, server) = spawn_dragon_tiger_success_loopback(
            DRAGON_TIGER_RECORDS.as_bytes(),
            "TEST_CODE_LHB_BATCH_FIRST",
            "TEST_CODE_LHB_SOURCE_FIRST",
        )
        .await;
        let source = GrpcSource::from_board_loopback_test_client(client);
        let queries = source.connected_board_queries().await.unwrap();
        let (mut fixture, stocks, config, intent, v9_head) =
            fixture_with_completed_v9_parent(&queries, "TEST_CODE_RUN_DRAGON_TIGER_F2").await;
        assert_eq!(
            fixture
                .chain_post_close()
                .migrate_schema_v9_to_v10()
                .unwrap()
                .schema_version(),
            10
        );
        let mut local = fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .unwrap();
        let lease = local
            .resume_run(
                &intent,
                lease_request(
                    "TEST_CODE_DRAGON_TIGER_F2_FIRST",
                    68_300_000_000,
                    90_000_000_000,
                    Some(v9_head),
                ),
            )
            .unwrap();
        let first_clock = DragonTigerClock {
            now: UtcMicros::try_new(micros("2026-07-22T10:30:00+08:00")).unwrap(),
            request_observed_at: DateTime::parse_from_rfc3339("2026-07-22T10:30:00+08:00").unwrap(),
            request_calls: Cell::new(0),
            cache_calls: Cell::new(0),
        };
        let mut io = local
            .dragon_tiger_preparation_io_v10(
                lease,
                &queries,
                &first_clock,
                FixedClusterConfiguration::resolve(Some("2")),
                &source,
            )
            .unwrap();
        let first_stop = prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks.clone(),
            None,
            &mut io,
        )
        .await
        .expect_err("TEST_CODE F2 first v10 prepare stops after DragonTiger");
        assert!(matches!(
            first_stop.downcast_ref::<PreparationStop>(),
            Some(PreparationStop::StageNotMigrated {
                next: UnmigratedStage::Macro
            })
        ));
        assert_eq!(
            first_stop
                .downcast_ref::<PreparationFailure>()
                .unwrap()
                .lhb_map()["TEST_CODE_600001"]
                .to_bits(),
            0x4029_0000_0000_0000
        );
        drop(io);
        let healthy = local.inspect_dragon_tiger(&intent).unwrap();
        assert_eq!(
            healthy.projection().unwrap().lhb_map()["TEST_CODE_600001"].to_bits(),
            0x4029_0000_0000_0000
        );
        let first_head = local.inspect_run(&intent).unwrap().head_version();
        drop(local);

        fixture.reopen();
        let mut local = fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .unwrap();
        let verification_lease = local
            .resume_run(
                &intent,
                lease_request(
                    "TEST_CODE_DRAGON_TIGER_F2_VERIFY",
                    159_200_000_000,
                    180_000_000_000,
                    Some(first_head),
                ),
            )
            .unwrap();
        drop(local);
        let first_network = server.snapshot();
        assert_eq!(first_network.dragon_tiger_requests.len(), 1);
        drop(queries);
        drop(source);
        assert_eq!(server.finish().await, first_network);

        let corrupted_snapshot =
            corrupt_dragon_tiger_projection(fixture.connection(), &intent);

        fixture.reopen();
        let (changed_client, changed_server) = spawn_dragon_tiger_success_loopback(
            CHANGED_RECORDS.as_bytes(),
            "TEST_CODE_LHB_BATCH_CHANGED",
            "TEST_CODE_LHB_SOURCE_CHANGED",
        )
        .await;
        let changed_source = GrpcSource::from_board_loopback_test_client(changed_client);
        let changed_queries = changed_source.connected_board_queries().await.unwrap();
        let audit_count = fixture.count("data_acquisition_audit");
        // A corrupt store may reject at the facade admission boundary.
        let inspect_result = fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .and_then(|mut local| local.inspect_dragon_tiger(&intent));
        let inspect_rejected = matches!(
            &inspect_result,
            Err(ChainPostCloseError::SchemaRejected)
        );
        let changed_clock = DragonTigerClock {
            now: UtcMicros::try_new(micros("2026-07-23T11:45:00+08:00")).unwrap(),
            request_observed_at: DateTime::parse_from_rfc3339("2026-07-23T11:45:00+08:00").unwrap(),
            request_calls: Cell::new(0),
            cache_calls: Cell::new(0),
        };
        let prepare_result = match fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
        {
            Err(error) => Err(anyhow::Error::new(error)),
            Ok(mut local) => match local.dragon_tiger_preparation_io_v10(
                verification_lease,
                &changed_queries,
                &changed_clock,
                FixedClusterConfiguration::resolve(Some("2")),
                &changed_source,
            ) {
                Err(error) => Err(anyhow::Error::new(error)),
                Ok(mut io) => prepare_chain_analysis_with_io(
                    NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
                    stocks,
                    None,
                    &mut io,
                )
                .await
                .map(|_| ()),
            },
        };
        let prepare_rejected = matches!(
            prepare_result
                .as_ref()
                .err()
                .and_then(|error| error.downcast_ref::<ChainPostCloseError>()),
            Some(ChainPostCloseError::SchemaRejected)
        );
        let inspect_class = match &inspect_result {
            Err(ChainPostCloseError::SchemaRejected) => "SchemaRejected",
            Ok(_) => "accepted",
            Err(_) => "other_error",
        };
        let prepare_class = if prepare_rejected {
            "SchemaRejected"
        } else if prepare_result.is_ok() {
            "completed"
        } else {
            "other_error"
        };
        assert_eq!(fixture.count("data_acquisition_audit"), audit_count);
        let changed_network = changed_server.snapshot();
        assert!(changed_network.requests.is_empty());
        assert!(changed_network.dragon_tiger_requests.is_empty());
        assert!(changed_server.membership_snapshot().is_empty());
        assert_eq!(changed_network.non_board_requests, 0);
        assert_eq!(changed_clock.request_calls.get(), 0);
        assert_eq!(changed_clock.cache_calls.get(), 0);
        drop(changed_queries);
        drop(changed_source);
        assert_eq!(changed_server.finish().await, changed_network);
        assert_eq!(
            dragon_tiger_fact_snapshot(fixture.connection()),
            corrupted_snapshot
        );
        assert_eq!(
            (inspect_rejected, prepare_rejected),
            (true, true),
            "TEST_CODE corrupted projection rejection: inspect={inspect_class}, prepare={prepare_class}"
        );
    })
    .await
    .expect("TEST_CODE DragonTiger F2 corruption scenario deadline");
}

#[tokio::test]
async fn dragon_tiger_held_facade_and_io_reject_projection_corrupted_after_admission() {
    assert_held_facade_rejects_corrupted_dragon_tiger(corrupt_dragon_tiger_projection).await;
}

#[tokio::test]
async fn dragon_tiger_rejects_a_saved_result_returned_before_its_request_began() {
    assert_held_facade_rejects_corrupted_dragon_tiger(corrupt_dragon_tiger_returned_at).await;
}

#[tokio::test]
async fn dragon_tiger_rejects_saved_results_bound_to_another_run_context_or_input() {
    for column in ["run_id", "run_context_sha256", "input_sha256"] {
        eprintln!("TEST_CODE result binding case: {column}");
        assert_held_facade_rejects_corrupted_dragon_tiger(|connection, intent| {
            corrupt_dragon_tiger_result_binding(connection, intent, column)
        })
        .await;
    }
}

fn corrupt_dragon_tiger_result_binding(
    connection: &Connection,
    intent: &IntentId,
    column: &str,
) -> DragonTigerFactSnapshot {
    let replacement = match column {
        "run_id" => "TEST_CODE_OTHER_RUN",
        "run_context_sha256" | "input_sha256" => {
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
        }
        _ => panic!("TEST_CODE unsupported result binding field"),
    };
    let mut expected = dragon_tiger_fact_snapshot(connection);
    assert_eq!(expected.results.len(), 1);
    let column_index = connection
        .prepare("SELECT * FROM chain_post_close_dragon_tiger_attempt_results")
        .unwrap()
        .column_index(column)
        .unwrap();
    let replacement_value = rusqlite::types::Value::Text(replacement.to_owned());
    assert_ne!(expected.results[0][column_index], replacement_value);
    let transaction = connection.unchecked_transaction().unwrap();
    let trigger_sql: String = transaction
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type='trigger' \
             AND name='chain_post_close_dragon_tiger_attempt_results_update'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    transaction
        .execute_batch("DROP TRIGGER chain_post_close_dragon_tiger_attempt_results_update")
        .unwrap();
    assert_eq!(
        transaction
            .execute(
                &format!(
                    "UPDATE chain_post_close_dragon_tiger_attempt_results \
                     SET {column}=?1 WHERE intent_id=?2 AND attempt_ordinal=1"
                ),
                params![replacement, intent.as_str()],
            )
            .unwrap(),
        1
    );
    transaction.execute_batch(&trigger_sql).unwrap();
    transaction.commit().unwrap();
    expected.results[0][column_index] = replacement_value;
    let after = dragon_tiger_fact_snapshot(connection);
    assert_eq!(after, expected);
    after
}

fn corrupt_dragon_tiger_returned_at(
    connection: &Connection,
    intent: &IntentId,
) -> DragonTigerFactSnapshot {
    let mut expected = dragon_tiger_fact_snapshot(connection);
    assert_eq!(expected.results.len(), 1);
    let returned_column = connection
        .prepare("SELECT * FROM chain_post_close_dragon_tiger_attempt_results")
        .unwrap()
        .column_index("returned_at")
        .unwrap();
    let transaction = connection.unchecked_transaction().unwrap();
    let (begun_at, returned_at, committed_at): (i64, i64, i64) = transaction
        .query_row(
            "SELECT begun.begun_at,result.returned_at,result.committed_at \
             FROM chain_post_close_dragon_tiger_attempt_begins AS begun \
             JOIN chain_post_close_dragon_tiger_attempt_results AS result \
               ON result.intent_id=begun.intent_id \
              AND result.attempt_ordinal=begun.attempt_ordinal \
             WHERE result.intent_id=?1 AND result.attempt_ordinal=1",
            [intent.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(begun_at, micros("2026-07-22T10:30:00+08:00"));
    assert_eq!(returned_at, begun_at);
    assert_eq!(committed_at, returned_at);
    assert!(begun_at > 0);
    let trigger_sql: String = transaction
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type='trigger' \
             AND name='chain_post_close_dragon_tiger_attempt_results_update'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    transaction
        .execute_batch("DROP TRIGGER chain_post_close_dragon_tiger_attempt_results_update")
        .unwrap();
    assert_eq!(
        transaction
            .execute(
                "UPDATE chain_post_close_dragon_tiger_attempt_results \
                 SET returned_at=0 WHERE intent_id=?1 AND attempt_ordinal=1",
                [intent.as_str()],
            )
            .unwrap(),
        1
    );
    transaction.execute_batch(&trigger_sql).unwrap();
    transaction.commit().unwrap();
    expected.results[0][returned_column] = rusqlite::types::Value::Integer(0);
    let after = dragon_tiger_fact_snapshot(connection);
    assert_eq!(after, expected);
    after
}

async fn assert_held_facade_rejects_corrupted_dragon_tiger(
    corrupt: impl Fn(&Connection, &IntentId) -> DragonTigerFactSnapshot,
) {
    tokio::time::timeout(Duration::from_secs(60), async {
        let (client, server) = spawn_dragon_tiger_success_loopback(
            DRAGON_TIGER_RECORDS.as_bytes(),
            "TEST_CODE_LHB_BATCH_FIRST",
            "TEST_CODE_LHB_SOURCE_FIRST",
        )
        .await;
        let source = GrpcSource::from_board_loopback_test_client(client);
        let queries = source.connected_board_queries().await.unwrap();
        let (mut fixture, stocks, config, intent, v9_head) = fixture_with_completed_v9_parent(
            &queries,
            "TEST_CODE_RUN_DRAGON_TIGER_HELD_FACADE",
        )
        .await;
        assert_eq!(
            fixture
                .chain_post_close()
                .migrate_schema_v9_to_v10()
                .unwrap()
                .schema_version(),
            10
        );
        let mut local = fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .unwrap();
        let lease = local
            .resume_run(
                &intent,
                lease_request(
                    "TEST_CODE_DRAGON_TIGER_HELD_FIRST",
                    68_300_000_000,
                    90_000_000_000,
                    Some(v9_head),
                ),
            )
            .unwrap();
        let first_clock = DragonTigerClock {
            now: UtcMicros::try_new(micros("2026-07-22T10:30:00+08:00")).unwrap(),
            request_observed_at: DateTime::parse_from_rfc3339("2026-07-22T10:30:00+08:00")
                .unwrap(),
            request_calls: Cell::new(0),
            cache_calls: Cell::new(0),
        };
        let mut io = local
            .dragon_tiger_preparation_io_v10(
                lease,
                &queries,
                &first_clock,
                FixedClusterConfiguration::resolve(Some("2")),
                &source,
            )
            .unwrap();
        let first_stop = prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks.clone(),
            None,
            &mut io,
        )
        .await
        .expect_err("TEST_CODE held-facade setup must stop at Macro");
        assert!(matches!(
            first_stop.downcast_ref::<PreparationStop>(),
            Some(PreparationStop::StageNotMigrated {
                next: UnmigratedStage::Macro
            })
        ));
        assert_eq!(
            first_stop
                .downcast_ref::<PreparationFailure>()
                .unwrap()
                .lhb_map()["TEST_CODE_600001"]
                .to_bits(),
            0x4029_0000_0000_0000
        );
        drop(io);
        let healthy = local.inspect_dragon_tiger(&intent).unwrap();
        assert_complete_batch(healthy.batch().unwrap());
        assert_eq!(
            healthy.projection().unwrap().lhb_map()["TEST_CODE_600001"].to_bits(),
            0x4029_0000_0000_0000
        );
        assert_eq!(healthy.request_observed_at(), "2026-07-22T10:30:00+08:00");
        let first_head = local.inspect_run(&intent).unwrap().head_version();
        drop(local);

        fixture.reopen();
        let first_network = server.snapshot();
        assert_eq!(first_network.dragon_tiger_requests.len(), 1);
        drop(queries);
        drop(source);
        assert_eq!(server.finish().await, first_network);

        let (changed_client, changed_server) = spawn_dragon_tiger_success_loopback(
            CHANGED_RECORDS.as_bytes(),
            "TEST_CODE_LHB_BATCH_CHANGED",
            "TEST_CODE_LHB_SOURCE_CHANGED",
        )
        .await;
        let changed_source = GrpcSource::from_board_loopback_test_client(changed_client);
        let changed_queries = changed_source.connected_board_queries().await.unwrap();
        let database = fixture.database();
        let audit_count = fixture.count("data_acquisition_audit");
        let mut local = fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .unwrap();
        let verification_lease = local
            .resume_run(
                &intent,
                lease_request(
                    "TEST_CODE_DRAGON_TIGER_HELD_VERIFY",
                    159_200_000_000,
                    180_000_000_000,
                    Some(first_head),
                ),
            )
            .unwrap();
        let changed_clock = DragonTigerClock {
            now: UtcMicros::try_new(micros("2026-07-23T11:45:00+08:00")).unwrap(),
            request_observed_at: DateTime::parse_from_rfc3339("2026-07-23T11:45:00+08:00")
                .unwrap(),
            request_calls: Cell::new(0),
            cache_calls: Cell::new(0),
        };
        let mut held_io = local
            .dragon_tiger_preparation_io_v10(
                verification_lease,
                &changed_queries,
                &changed_clock,
                FixedClusterConfiguration::resolve(Some("2")),
                &changed_source,
            )
            .unwrap();

        let injector = Connection::open_with_flags(
            database,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .unwrap();
        injector.busy_timeout(Duration::ZERO).unwrap();
        let corrupted_snapshot = corrupt(&injector, &intent);
        drop(injector);

        let prepare_result = prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks,
            None,
            &mut held_io,
        )
        .await;
        let prepare_rejected = matches!(
            prepare_result
                .as_ref()
                .err()
                .and_then(|error| error.downcast_ref::<ChainPostCloseError>()),
            Some(ChainPostCloseError::SchemaRejected)
        );
        drop(held_io);
        let inspect_result = local.inspect_dragon_tiger(&intent);
        let inspect_rejected = matches!(
            &inspect_result,
            Err(ChainPostCloseError::SchemaRejected)
        );
        let prepare_class = if prepare_rejected {
            "SchemaRejected"
        } else if prepare_result.is_ok() {
            "completed"
        } else {
            "other_error"
        };
        let inspect_class = match &inspect_result {
            Err(ChainPostCloseError::SchemaRejected) => "SchemaRejected",
            Ok(_) => "accepted",
            Err(_) => "other_error",
        };
        drop(local);

        assert_eq!(fixture.count("data_acquisition_audit"), audit_count);
        assert_eq!(
            dragon_tiger_fact_snapshot(fixture.connection()),
            corrupted_snapshot
        );
        let changed_network = changed_server.snapshot();
        assert!(changed_network.requests.is_empty());
        assert!(changed_network.dragon_tiger_requests.is_empty());
        assert!(changed_server.membership_snapshot().is_empty());
        assert_eq!(changed_network.non_board_requests, 0);
        assert_eq!(changed_clock.request_calls.get(), 0);
        assert_eq!(changed_clock.cache_calls.get(), 0);
        drop(changed_queries);
        drop(changed_source);
        assert_eq!(changed_server.finish().await, changed_network);
        assert_eq!(
            (prepare_rejected, inspect_rejected),
            (true, true),
            "TEST_CODE post-admission corruption rejection: prepare={prepare_class}, inspect={inspect_class}"
        );
    })
    .await
    .expect("TEST_CODE DragonTiger held-facade corruption scenario deadline");
}
