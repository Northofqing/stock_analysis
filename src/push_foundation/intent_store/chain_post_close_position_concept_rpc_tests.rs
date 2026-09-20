use super::*;
use crate::database::data_acquisition_audit::{
    read_acquisition_in_transaction, DataAcquisitionAuditReceipt,
};
use crate::grpc_client::client::board_loopback_fixture::{
    spawn_membership_commit_failure_loopback, spawn_membership_success_loopback,
};
use crate::grpc_client::pb::magic::market::v1::{
    AdmissionState, ErrorDetail, Operation, QueryRequest, QueryResponse,
};
use prost::Message as _;

const MISSING: &str = "TEST_CODE_600001";
const OWNER_V9: &str = "TEST_CODE_POSITION_RPC_OWNER_C";
const OWNER_REOPENED: &str = "TEST_CODE_POSITION_RPC_OWNER_D";
const PAYLOAD: &[u8] = br#"[{"instrument_code":"TEST_CODE_600001","board_code":"TEST_CODE_BOARD_MAIN","board_name":"TEST_CODE_CLUSTER_A_MAIN","kind":"Industry"},{"instrument_code":"TEST_CODE_600001","board_code":"TEST_CODE_BOARD_ALIAS","board_name":"TEST_CODE_CLUSTER_B_ALIAS","kind":"Concept"}]"#;
const TOOL_JSON: &str = r#"{"all_boards":["TEST_CODE_CLUSTER_A_MAIN","TEST_CODE_CLUSTER_B_ALIAS"],"board_count":2,"evidence":{"batch_id":"TEST_CODE_MEMBERSHIP_BATCH","observed_at":"2026-07-21T15:31:00+08:00","provider":"Tdx","source":"TEST_CODE_LOOPBACK_MEMBERSHIP_SOURCE","source_at":"2026-07-21T15:30:00+08:00"},"fetched":true,"memberships":[{"board_code":"TEST_CODE_BOARD_MAIN","board_name":"TEST_CODE_CLUSTER_A_MAIN","category":"Industry"},{"board_code":"TEST_CODE_BOARD_ALIAS","board_name":"TEST_CODE_CLUSTER_B_ALIAS","category":"Concept"}],"note":"统一 Magic TDX 板块归属；行业/概念使用源类别，不再按列表位置猜测。","primary_boards":["TEST_CODE_CLUSTER_A_MAIN"],"secondary_boards":["TEST_CODE_CLUSTER_B_ALIAS"],"secucode":"TEST_CODE_600001"}"#;
const CACHE: &[u8] = br#"["TEST_CODE_CLUSTER_A_MAIN","TEST_CODE_CLUSTER_B_ALIAS"]"#;
const MEMBERSHIP_REQUEST_HASH: &str =
    "b822584c8f4102261d2a3c8b32deee7f956199e51c26424c489ff413339d385e";

const V9_TABLES: [&str; 7] = [
    "chain_post_close_position_concept_rpc_occurrences",
    "chain_post_close_position_concept_rpc_attempt_begins",
    "chain_post_close_position_concept_rpc_attempt_results",
    "chain_post_close_position_concept_rpc_status_materials",
    "chain_post_close_position_concept_rpc_error_materials",
    "chain_post_close_position_concept_rpc_finals",
    "chain_post_close_position_concept_cache_writes",
];

fn integer(value: &Value) -> i64 {
    match value {
        Value::Integer(value) => *value,
        other => panic!("TEST_CODE expected integer, got {other:?}"),
    }
}

fn expected_v9_new_objects() -> Vec<(String, String)> {
    let mut objects = Vec::new();
    for table in V9_TABLES {
        objects.push((table.to_owned(), "table".to_owned()));
        for suffix in ["guard", "update", "delete"] {
            objects.push((format!("{table}_{suffix}"), "trigger".to_owned()));
        }
    }
    objects.sort();
    objects
}

fn v9_fact_rows(connection: &Connection) -> BTreeMap<String, Vec<Vec<Value>>> {
    V9_TABLES
        .into_iter()
        .map(|table| {
            (
                table.to_owned(),
                all_rows(connection, &format!("SELECT * FROM \"{table}\"")),
            )
        })
        .collect()
}

fn expected_position_bytes_v9(observed_at: i64) -> Vec<u8> {
    format!(
        concat!(
            "{{\"schema_version\":1,",
            "\"source_contract\":\"stock_position/open/buy_date_desc/v1\",",
            "\"business_date\":\"2026-07-21\",",
            "\"query_observed_at\":{observed_at},\"rows\":[",
            "{{\"id\":1,\"code\":\"TEST_CODE_CLUSTER_STOCK_A\",",
            "\"name\":\"成员\",\"buy_date\":\"2026-07-21\",",
            "\"buy_price_bits\":\"4024000000000000\",\"quantity\":100,",
            "\"status\":\"open\",\"sell_date\":null,\"sell_price_bits\":null,",
            "\"return_rate_bits\":\"3ff8000000000000\",",
            "\"created_at\":\"2026-07-21 09:01:02\",",
            "\"updated_at\":\"2026-07-21 14:01:02\",",
            "\"chain_name\":\"TEST_CODE原链\",\"st_type\":null}},",
            "{{\"id\":2,\"code\":\"TEST_CODE_600001\",",
            "\"name\":\"缺失\",\"buy_date\":\"2026-07-20\",",
            "\"buy_price_bits\":\"4034000000000000\",\"quantity\":200,",
            "\"status\":\"open\",\"sell_date\":null,\"sell_price_bits\":null,",
            "\"return_rate_bits\":null,",
            "\"created_at\":\"2026-07-20 09:02:03\",",
            "\"updated_at\":\"2026-07-21 14:02:03\",",
            "\"chain_name\":null,\"st_type\":\"ST\"}},",
            "{{\"id\":3,\"code\":\"TEST_CODE_CLUSTER_POS_OTHER\",",
            "\"name\":\"无关\",\"buy_date\":\"2026-07-19\",",
            "\"buy_price_bits\":\"403e000000000000\",\"quantity\":100,",
            "\"status\":\"open\",\"sell_date\":null,\"sell_price_bits\":null,",
            "\"return_rate_bits\":\"c000000000000000\",",
            "\"created_at\":\"2026-07-19 09:03:04\",",
            "\"updated_at\":\"2026-07-21 14:03:04\",",
            "\"chain_name\":null,\"st_type\":\"*ST\"}}]}}"
        ),
        observed_at = observed_at
    )
    .into_bytes()
}

fn expected_cache_bytes_v9(
    observed_at: i64,
    positions_version: u64,
    positions_sha: &str,
) -> Vec<u8> {
    format!(
        concat!(
            "{{\"schema_version\":1,",
            "\"source_contract\":\"stock_concepts/updated_at_gte_local_7d/v1\",",
            "\"query_observed_at\":{observed_at},",
            "\"local_offset_seconds\":28800,\"cutoff_local_offset_seconds\":28800,",
            "\"cache_cutoff\":\"2026-07-14 15:33:00\",",
            "\"positions_run_version\":{positions_version},",
            "\"positions_sha256\":\"{positions_sha}\",",
            "\"requested_codes\":[\"TEST_CODE_CLUSTER_STOCK_A\",",
            "\"TEST_CODE_600001\",\"TEST_CODE_CLUSTER_POS_OTHER\"],",
            "\"cache_rows\":[",
            "{{\"code\":\"TEST_CODE_CLUSTER_STOCK_A\",",
            "\"concepts\":\"[\\\"TEST_CODE_CLUSTER_A_MAIN\\\",\\\"TEST_CODE_CLUSTER_B_ALIAS\\\",\\\"昨日涨停\\\"]\",",
            "\"updated_at\":\"2026-07-21 14:00:00\"}},",
            "{{\"code\":\"TEST_CODE_CLUSTER_STOCK_B\",",
            "\"concepts\":\"[\\\"TEST_CODE_CLUSTER_A_MAIN\\\",\\\"TEST_CODE_CLUSTER_B_ALIAS\\\"]\",",
            "\"updated_at\":\"2026-07-21 14:00:00\"}},",
            "{{\"code\":\"TEST_CODE_CLUSTER_STOCK_ISOLATED\",",
            "\"concepts\":\"[\\\"TEST_CODE_CLUSTER_Z_ISOLATED\\\"]\",",
            "\"updated_at\":\"2026-07-21 14:00:00\"}},",
            "{{\"code\":\"TEST_CODE_CLUSTER_POS_OTHER\",",
            "\"concepts\":\"[\\\"TEST_CODE_CLUSTER_Z_OTHER\\\"]\",",
            "\"updated_at\":\"2026-07-21 14:05:00\"}}]}}"
        ),
        observed_at = observed_at,
        positions_version = positions_version,
        positions_sha = positions_sha,
    )
    .into_bytes()
}

fn expected_position_concepts_v9() -> BTreeMap<String, Vec<String>> {
    [
        (
            STOCK_A,
            vec![
                "TEST_CODE_CLUSTER_A_MAIN",
                "TEST_CODE_CLUSTER_B_ALIAS",
                "昨日涨停",
            ],
        ),
        (
            "TEST_CODE_CLUSTER_STOCK_B",
            vec!["TEST_CODE_CLUSTER_A_MAIN", "TEST_CODE_CLUSTER_B_ALIAS"],
        ),
        (
            "TEST_CODE_CLUSTER_STOCK_ISOLATED",
            vec!["TEST_CODE_CLUSTER_Z_ISOLATED"],
        ),
        (
            MISSING,
            vec!["TEST_CODE_CLUSTER_A_MAIN", "TEST_CODE_CLUSTER_B_ALIAS"],
        ),
        (POS_OTHER, vec!["TEST_CODE_CLUSTER_Z_OTHER"]),
    ]
    .into_iter()
    .map(|(code, concepts)| {
        (
            code.to_owned(),
            concepts.into_iter().map(str::to_owned).collect(),
        )
    })
    .collect()
}

#[track_caller]
fn assert_v9_position_failure(error: &anyhow::Error) {
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::StageNotMigrated {
            next: UnmigratedStage::DragonTiger
        })
    ));
    let failure = error
        .downcast_ref::<PreparationFailure>()
        .expect("TEST_CODE v9 DragonTiger stop retains observations");
    assert_eq!(
        failure.business_date(),
        NaiveDate::from_ymd_opt(2026, 7, 21).unwrap()
    );
    assert_eq!(failure.stage(), PreparationStage::DragonTiger);
    assert_eq!(
        failure.completed_stages(),
        [
            PreparationStage::Concepts,
            PreparationStage::ClusterWritesAndLifecycle,
            PreparationStage::Candidates,
            PreparationStage::Positions,
            PreparationStage::PositionConcepts,
        ]
    );
    assert_eq!(
        failure
            .positions()
            .iter()
            .map(|position| (
                position.code(),
                position.name(),
                position.return_rate().map(f64::to_bits),
            ))
            .collect::<Vec<_>>(),
        [
            (STOCK_A, "成员", Some(0x3ff8000000000000)),
            (MISSING, "缺失", None),
            (POS_OTHER, "无关", Some(0xc000000000000000)),
        ]
    );
    assert_eq!(
        failure.position_concepts(),
        &expected_position_concepts_v9()
    );
    assert_eq!(
        failure
            .position_diags()
            .iter()
            .map(|diag| (
                diag.code.as_str(),
                diag.name.as_str(),
                diag.return_rate.map(f64::to_bits),
                diag.mainline
                    .as_ref()
                    .map(|(name, days)| (name.as_str(), *days)),
                diag.in_limit_pool,
            ))
            .collect::<Vec<_>>(),
        [
            (
                STOCK_A,
                "成员",
                Some(0x3ff8000000000000),
                Some(("TEST_CODE_CLUSTER_A_MAIN", 3)),
                true,
            ),
            (
                MISSING,
                "缺失",
                None,
                Some(("TEST_CODE_CLUSTER_A_MAIN", 3)),
                false,
            ),
            (POS_OTHER, "无关", Some(0xc000000000000000), None, false,),
        ]
    );
    assert_eq!(failure.clusters().len(), 1);
    assert_eq!(failure.clusters()[0].concept, "TEST_CODE_CLUSTER_A_MAIN");
    assert_eq!(failure.clusters()[0].stocks.len(), 2);
    assert_eq!(failure.clusters()[0].streak_days, 3);
    assert_eq!(
        failure.candidate_board_codes()["TEST_CODE_CLUSTER_A_MAIN"],
        "TEST_CODE_BOARD_MAIN"
    );
    let source = &failure.candidate_sources()["TEST_CODE_CLUSTER_A_MAIN"];
    assert_eq!(source.status(), &SourceStatus::Unavailable);
    assert!(source.reason().unwrap().contains("unsupported"));
    assert!(failure.lhb_map().is_empty());
    assert_eq!(failure.lhb_source().status(), &SourceStatus::Unknown);
    assert_eq!(failure.macro_input(), None);
}

#[tokio::test]
async fn single_user_local_position_concept_rpc_reopens_without_repeating_membership_or_cache_write(
) {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut fixture = V2BusinessFixture::new();
        fixture.install_v2();
        fixture.chain_post_close().migrate_schema_v2_to_v3().unwrap();
        cluster_tests::install_business_rows(&fixture);
        fixture.chain_post_close().migrate_schema_v3_to_v4().unwrap();
        install_br159(&mut fixture);
        fixture.chain_post_close().migrate_schema_v4_to_v5().unwrap();
        fixture.chain_post_close().migrate_schema_v5_to_v6().unwrap();
        fixture.chain_post_close().migrate_schema_v6_to_v7().unwrap();
        for table in V9_TABLES {
            assert_eq!(
                fixture.connection().query_row(
                    "SELECT count(*) FROM sqlite_schema WHERE type='table' AND name=?1",
                    [table],
                    |row| row.get::<_, i64>(0),
                ).unwrap(),
                0,
                "TEST_CODE fixture must not preinstall {table}"
            );
        }

        let (client, server) = spawn_membership_success_loopback().await;
        let source = GrpcSource::from_board_loopback_test_client(client);
        let queries = source.connected_board_queries().await.unwrap();
        let stocks = cluster_tests::cluster_stocks();
        let config = local_config(BUILD_A);
        let context = build_single_user_local_chain_post_close_context(
            &config,
            run_input("TEST_CODE_RUN_POSITION_CONCEPT_V9"),
        )
        .unwrap();
        let mut local = fixture.store.as_mut().unwrap()
            .single_user_local_chain_post_close(&config).unwrap();
        let lease = local.acquire_run(
            context,
            fixed_input(stocks.clone()),
            lease_request(OWNER_A, 1_000_000, 60_000_000, None),
        ).unwrap();
        let intent = lease.intent_id().clone();
        let v7_clock = ControlledClock::new(at(2_000_000));
        let mut io = local.concept_rpc_preparation_io_v7(
            lease, &queries, &v7_clock, FixedClusterConfiguration::resolve(Some("2")),
        ).unwrap();
        let v7_error = prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(), stocks.clone(), None, &mut io,
        ).await.expect_err("TEST_CODE v7 stops at Positions");
        assert!(matches!(
            v7_error.downcast_ref::<PreparationStop>(),
            Some(PreparationStop::StageNotMigrated { next: UnmigratedStage::Positions })
        ));
        drop(io);
        let v7_head = local.inspect_run(&intent).unwrap().head_version();
        drop(local);
        let board_requests = server.snapshot();
        assert_eq!(
            board_requests.requests.iter().map(|request| request.kind.as_str()).collect::<Vec<_>>(),
            ["Industry", "Industry", "Concept"]
        );
        assert_eq!(board_requests.non_board_requests, 0);
        assert!(server.membership_snapshot().is_empty());

        fixture.execute(
            "CREATE TABLE stock_position ( \
               id INTEGER PRIMARY KEY AUTOINCREMENT, code TEXT NOT NULL, name TEXT NOT NULL, \
               buy_date TEXT NOT NULL, buy_price REAL NOT NULL, quantity INTEGER NOT NULL, \
               status TEXT NOT NULL, sell_date TEXT, sell_price REAL, return_rate REAL, \
               created_at TEXT NOT NULL, updated_at TEXT NOT NULL, chain_name TEXT, st_type TEXT, \
               UNIQUE(code,buy_date)); \
             INSERT INTO stock_position VALUES \
               (1,'TEST_CODE_CLUSTER_STOCK_A','成员','2026-07-21',10.0,100,'open',NULL,NULL,1.5, \
                '2026-07-21 09:01:02','2026-07-21 14:01:02','TEST_CODE原链',NULL), \
               (2,'TEST_CODE_600001','缺失','2026-07-20',20.0,200,'open',NULL,NULL,NULL, \
                '2026-07-20 09:02:03','2026-07-21 14:02:03',NULL,'ST'), \
               (3,'TEST_CODE_CLUSTER_POS_OTHER','无关','2026-07-19',30.0,100,'open',NULL,NULL,-2.0, \
                '2026-07-19 09:03:04','2026-07-21 14:03:04',NULL,'*ST'), \
               (4,'TEST_CODE_CLUSTER_CLOSED','关闭','2026-07-22',40.0,100,'closed','2026-07-22',41.0,2.0, \
                '2026-07-22 09:04:05','2026-07-22 14:04:05',NULL,NULL); \
             INSERT INTO stock_concepts(code,concepts,updated_at) VALUES \
               ('TEST_CODE_CLUSTER_POS_OTHER','[\"TEST_CODE_CLUSTER_Z_OTHER\"]','2026-07-21 14:05:00');"
        );
        assert_eq!(cache_rows(fixture.connection()).len(), 4);
        assert!(!cache_rows(fixture.connection()).iter().any(|row| row.0 == MISSING));

        assert_eq!(
            fixture.chain_post_close().migrate_schema_v7_to_v8().unwrap().schema_version(), 8
        );
        let mut local = fixture.store.as_mut().unwrap()
            .single_user_local_chain_post_close(&config).unwrap();
        let lease = local.resume_run(
            &intent, lease_request(OWNER_B, 61_000_000, 360_000_000, Some(v7_head)),
        ).unwrap();
        let v8_clock = PositionsClock {
            now: UtcMicros::try_new(micros("2026-07-21T15:33:00+08:00")).unwrap(),
            observation: Some((
                DateTime::parse_from_rfc3339("2026-07-21T15:33:00+08:00").unwrap(),
                DateTime::parse_from_rfc3339("2026-07-14T15:33:00+08:00").unwrap(),
            )),
            cache_calls: Cell::new(0),
        };
        let mut io = local.positions_preparation_io_v8(
            lease, &queries, &v8_clock, FixedClusterConfiguration::resolve(Some("2")),
        ).unwrap();
        let v8_error = prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(), stocks.clone(), None, &mut io,
        ).await.expect_err("TEST_CODE v8 stops at PositionConceptProvider");
        assert!(matches!(
            v8_error.downcast_ref::<PreparationStop>(),
            Some(PreparationStop::StageNotMigrated {
                next: UnmigratedStage::PositionConceptProvider
            })
        ));
        assert_eq!(
            v8_error.downcast_ref::<PreparationFailure>().unwrap()
                .positions().iter().map(|position| position.code()).collect::<Vec<_>>(),
            [STOCK_A, MISSING, POS_OTHER]
        );
        drop(io);
        assert_eq!(v8_clock.cache_calls.get(), 1);
        let v8_head = local.inspect_run(&intent).unwrap().head_version();
        assert_eq!(v8_head, v7_head + 3);
        drop(local);
        assert!(server.membership_snapshot().is_empty());
        assert_eq!(server.snapshot(), board_requests);

        let (position_columns, position_row) =
            single_named_row(fixture.connection(), "chain_post_close_position_materials");
        let position_bytes = expected_position_bytes_v9(micros("2026-07-21T15:33:00+08:00"));
        let position_sha = hex::encode(Sha256::digest(&position_bytes));
        assert_eq!(value_at(&position_columns, &position_row, "material_bytes"), &Value::Blob(position_bytes));
        assert_eq!(value_at(&position_columns, &position_row, "material_sha256"), &Value::Text(position_sha.clone()));
        assert_eq!(value_at(&position_columns, &position_row, "row_count"), &Value::Integer(3));
        let positions_version = u64::try_from(integer(
            value_at(&position_columns, &position_row, "run_version")
        )).unwrap();
        assert_eq!(positions_version, v7_head + 2);
        let (cache_columns, cache_row) = single_named_row(
            fixture.connection(), "chain_post_close_position_concept_materials"
        );
        let cache_bytes = expected_cache_bytes_v9(
            micros("2026-07-21T15:33:00+08:00"), positions_version, &position_sha
        );
        let cache_sha = hex::encode(Sha256::digest(&cache_bytes));
        assert_eq!(value_at(&cache_columns, &cache_row, "material_bytes"), &Value::Blob(cache_bytes));
        assert_eq!(value_at(&cache_columns, &cache_row, "material_sha256"), &Value::Text(cache_sha.clone()));
        assert_eq!(value_at(&cache_columns, &cache_row, "positions_run_version"), &Value::Integer(i64::try_from(positions_version).unwrap()));
        assert_eq!(value_at(&cache_columns, &cache_row, "positions_sha256"), &Value::Text(position_sha.clone()));
        assert_eq!(value_at(&cache_columns, &cache_row, "requested_count"), &Value::Integer(3));
        assert_eq!(value_at(&cache_columns, &cache_row, "cache_row_count"), &Value::Integer(4));
        let cache_version = u64::try_from(integer(
            value_at(&cache_columns, &cache_row, "run_version")
        )).unwrap();
        assert_eq!(cache_version, v8_head);

        let old_names = table_names(fixture.connection());
        let old_facts = old_fact_rows(fixture.connection(), &old_names);
        let old_audits = (
            all_rows(fixture.connection(), "SELECT * FROM data_acquisition_audit ORDER BY id"),
            all_rows(fixture.connection(), "SELECT * FROM data_acquisition_audit_chain ORDER BY acquisition_audit_id"),
        );
        assert_eq!((old_audits.0.len(), old_audits.1.len()), (2, 2));
        let old_context = fixture.stored_context(&intent);
        let old_input = fixture.stored_input(&intent);
        let old_foundation = fixture.foundation_catalog();
        let old_schema = all_rows(fixture.connection(), "SELECT * FROM chain_post_close_schema ORDER BY schema_version");
        let old_layouts = all_rows(fixture.connection(), "SELECT * FROM chain_post_close_layouts ORDER BY layout_version");
        let old_registry = all_rows(fixture.connection(), "SELECT * FROM chain_post_close_layout_objects ORDER BY layout_version,name");
        let layout8_registry = all_rows(
            fixture.connection(),
            "SELECT name,object_type,CAST(definition AS BLOB) \
             FROM chain_post_close_layout_objects WHERE layout_version=8 ORDER BY name",
        );
        assert_eq!(layout8_registry.len(), 107);
        let old_catalog = all_rows(
            fixture.connection(),
            "SELECT name,type,tbl_name,CAST(sql AS BLOB) FROM sqlite_schema \
             WHERE name LIKE 'chain_post_close_%' ORDER BY name",
        );
        let old_identity = (
            fixture.connection().query_row("PRAGMA main.user_version", [], |row| row.get::<_, i64>(0)).unwrap(),
            fixture.connection().query_row("PRAGMA main.application_id", [], |row| row.get::<_, i64>(0)).unwrap(),
        );
        let saved_position_rows = all_rows(fixture.connection(), "SELECT * FROM chain_post_close_position_materials");
        let saved_cache_material_rows = all_rows(fixture.connection(), "SELECT * FROM chain_post_close_position_concept_materials");

        let head_before_migration: i64 = fixture.connection()
            .query_row("SELECT head_version FROM chain_post_close_runs", [], |row| row.get(0)).unwrap();
        assert_eq!(u64::try_from(head_before_migration).unwrap(), v8_head);
        assert_eq!(
            fixture.chain_post_close().migrate_schema_v8_to_v9().unwrap().schema_version(), 9
        );
        let head_after_migration: i64 = fixture.connection()
            .query_row("SELECT head_version FROM chain_post_close_runs", [], |row| row.get(0)).unwrap();
        assert_eq!(head_after_migration, head_before_migration);
        let v9_registry = all_rows(
            fixture.connection(),
            "SELECT name,object_type,CAST(definition AS BLOB) \
             FROM chain_post_close_layout_objects \
             WHERE layout_version=9 ORDER BY name",
        );
        assert_eq!(v9_registry.len(), 135);
        assert!(layout8_registry.iter().all(|row| v9_registry.contains(row)));
        let v9_new_objects = v9_registry
            .iter()
            .filter(|row| !layout8_registry.contains(row))
            .map(|row| {
                let [Value::Text(name), Value::Text(kind), Value::Blob(_)] = row.as_slice() else {
                    panic!("TEST_CODE v9 registry identity row: {row:?}");
                };
                (name.clone(), kind.clone())
            })
            .collect::<Vec<_>>();
        assert_eq!(v9_new_objects, expected_v9_new_objects());
        assert_eq!(
            fixture.connection().query_row(
                "SELECT count(*) FROM chain_post_close_layouts \
                 WHERE layout_version=9 AND predecessor_layout_version=8",
                [], |row| row.get::<_, i64>(0),
            ).unwrap(),
            1
        );
        assert_eq!(all_rows(fixture.connection(), "SELECT * FROM chain_post_close_schema ORDER BY schema_version"), old_schema);
        assert_eq!(fixture.stored_context(&intent), old_context);
        assert_eq!(fixture.stored_input(&intent), old_input);
        assert_eq!(fixture.foundation_catalog(), old_foundation);
        assert_eq!(old_fact_rows(fixture.connection(), &old_names), old_facts);
        assert_eq!(
            (
                all_rows(fixture.connection(), "SELECT * FROM data_acquisition_audit ORDER BY id"),
                all_rows(fixture.connection(), "SELECT * FROM data_acquisition_audit_chain ORDER BY acquisition_audit_id"),
            ),
            old_audits
        );
        assert!(v9_fact_rows(fixture.connection()).values().all(Vec::is_empty));
        let migrated_layouts = all_rows(fixture.connection(), "SELECT * FROM chain_post_close_layouts ORDER BY layout_version");
        assert_eq!(&migrated_layouts[..old_layouts.len()], old_layouts.as_slice());
        let migrated_registry = all_rows(fixture.connection(), "SELECT * FROM chain_post_close_layout_objects ORDER BY layout_version,name");
        assert_eq!(&migrated_registry[..old_registry.len()], old_registry.as_slice());
        assert_eq!(migrated_registry.len(), old_registry.len() + 135);
        let migrated_catalog = all_rows(
            fixture.connection(),
            "SELECT name,type,tbl_name,CAST(sql AS BLOB) FROM sqlite_schema \
             WHERE name LIKE 'chain_post_close_%' ORDER BY name",
        );
        assert!(old_catalog.iter().all(|row| migrated_catalog.contains(row)));
        assert_eq!(migrated_catalog.len(), old_catalog.len() + 28);

        let mut local = fixture.store.as_mut().unwrap()
            .single_user_local_chain_post_close(&config).unwrap();
        let lease = local.resume_run(
            &intent, lease_request(OWNER_V9, 361_000_000, 660_000_000, Some(v8_head)),
        ).unwrap();
        let v9_clock = PositionsClock {
            now: UtcMicros::try_new(micros("2026-07-21T15:37:02+08:00")).unwrap(),
            observation: None,
            cache_calls: Cell::new(0),
        };
        let mut io = local.position_concept_rpc_preparation_io_v9(
            lease, &queries, &v9_clock, FixedClusterConfiguration::resolve(Some("2")),
        ).unwrap();
        let v9_error = prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(), stocks.clone(), None, &mut io,
        ).await.expect_err("TEST_CODE v9 stops at DragonTiger");
        assert_v9_position_failure(&v9_error);
        drop(io);
        assert_eq!(v9_clock.cache_calls.get(), 0);
        let v9_head = local.inspect_run(&intent).unwrap().head_version();
        assert_eq!(v9_head, v8_head + 6);
        drop(local);

        let memberships = server.membership_snapshot();
        assert_eq!(memberships.len(), 1);
        let observed = &memberships[0];
        assert_eq!(observed.codes, [MISSING]);
        assert!(!observed.request_id.is_empty());
        assert_eq!(observed.protocol_version, 1);
        assert_eq!(observed.payload_schema, "board.constituents");
        assert_eq!(observed.payload_schema_version, 1);
        assert_eq!(observed.payload_content_type, "application/json; charset=utf-8");
        assert!(observed.preferred_provider.is_empty());
        assert!(!observed.allow_unadmitted);
        assert!(observed.authorized);
        assert_eq!(server.snapshot(), board_requests);

        let connection = fixture.connection();
        let (occurrence_columns, occurrence) = single_named_row(
            connection, "chain_post_close_position_concept_rpc_occurrences"
        );
        let request_bytes = blob(value_at(&occurrence_columns, &occurrence, "request_bytes")).to_vec();
        let request_sha = hex::encode(Sha256::digest(&request_bytes));
        let request_envelope: serde_json::Value = serde_json::from_slice(&request_bytes).unwrap();
        let request_wire_json = request_envelope["request_wire"].clone();
        assert_eq!(request_envelope, serde_json::json!({
            "schema_version": 1,
            "code": MISSING,
            "operation": "BoardConstituents",
            "request_id": observed.request_id,
            "request_wire": request_wire_json.clone(),
            "profile": "LocalBridgeV1",
            "acquisition_authority": null,
            "retry_max_attempts": 4,
            "retry_base_delay_ms": 1_000,
            "retry_max_delay_ms": 60_000,
            "retry_jitter_ms": 200,
        }));
        let request_wire: Vec<u8> = serde_json::from_value(request_wire_json).unwrap();
        let decoded_request = QueryRequest::decode(request_wire.as_slice()).unwrap();
        assert_eq!(decoded_request.encode_to_vec(), request_wire);
        let request_context = decoded_request.context.as_ref().unwrap();
        let request_payload = decoded_request.payload.as_ref().unwrap();
        assert_eq!(request_context.request_id, observed.request_id);
        assert_eq!(request_context.protocol_version, 1);
        assert_eq!(request_payload.schema, "board.constituents");
        assert_eq!(request_payload.schema_version, 1);
        assert_eq!(request_payload.content_type, "application/json; charset=utf-8");
        assert_eq!(request_payload.data, br#"{"codes":["TEST_CODE_600001"]}"#);
        assert!(decoded_request.preferred_provider.is_empty());
        assert!(!decoded_request.allow_unadmitted);
        assert!(!request_bytes.windows(b"TEST_CODE_BOARD_LOOPBACK_TOKEN".len())
            .any(|window| window == b"TEST_CODE_BOARD_LOOPBACK_TOKEN"));

        let (run_id, context_sha, input_sha): (String, String, String) = connection.query_row(
            "SELECT run_id,run_context_sha256,input_sha256 FROM chain_post_close_runs WHERE intent_id=?1",
            [intent.as_str()], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).unwrap();
        let rpc_at = micros("2026-07-21T15:37:02+08:00");
        for (name, expected) in [
            ("intent_id", Value::Text(intent.as_str().to_owned())),
            ("cache_material_run_version", Value::Integer(i64::try_from(cache_version).unwrap())),
            ("position_ordinal", Value::Integer(1)),
            ("code", Value::Text(MISSING.to_owned())),
            ("cache_material_sha256", Value::Text(cache_sha.clone())),
            ("positions_run_version", Value::Integer(i64::try_from(positions_version).unwrap())),
            ("positions_sha256", Value::Text(position_sha.clone())),
            ("operation", Value::Text("BoardConstituents".to_owned())),
            ("request_id", Value::Text(observed.request_id.clone())),
            ("request_codec_version", Value::Integer(1)),
            ("request_length", Value::Integer(i64::try_from(request_bytes.len()).unwrap())),
            ("request_sha256", Value::Text(request_sha.clone())),
            ("acquisition_request_hash", Value::Text(MEMBERSHIP_REQUEST_HASH.to_owned())),
            ("profile", Value::Text("LocalBridgeV1".to_owned())),
            ("acquisition_authority", Value::Null),
            ("retry_max_attempts", Value::Integer(4)),
            ("retry_base_delay_ms", Value::Integer(1_000)),
            ("retry_max_delay_ms", Value::Integer(60_000)),
            ("retry_jitter_ms", Value::Integer(200)),
            ("run_id", Value::Text(run_id.clone())),
            ("run_context_sha256", Value::Text(context_sha.clone())),
            ("input_sha256", Value::Text(input_sha.clone())),
            ("lease_owner", Value::Text(OWNER_V9.to_owned())),
            ("lease_generation", Value::Integer(3)),
            ("prior_head_version", Value::Integer(i64::try_from(v8_head + 1).unwrap())),
            ("run_version", Value::Integer(i64::try_from(v8_head + 2).unwrap())),
            ("planned_at", Value::Integer(rpc_at)),
        ] {
            assert_eq!(value_at(&occurrence_columns, &occurrence, name), &expected, "TEST_CODE occurrence {name}");
        }
        assert_eq!(value_at(&occurrence_columns, &occurrence, "request_bytes"), &Value::Blob(request_bytes.clone()));

        let (begin_columns, begin) = single_named_row(
            connection, "chain_post_close_position_concept_rpc_attempt_begins"
        );
        for (name, expected) in [
            ("intent_id", Value::Text(intent.as_str().to_owned())),
            ("cache_material_run_version", Value::Integer(i64::try_from(cache_version).unwrap())),
            ("position_ordinal", Value::Integer(1)),
            ("attempt_ordinal", Value::Integer(1)),
            ("occurrence_run_version", Value::Integer(i64::try_from(v8_head + 2).unwrap())),
            ("request_id", Value::Text(observed.request_id.clone())),
            ("request_sha256", Value::Text(request_sha.clone())),
            ("previous_attempt_ordinal", Value::Null),
            ("previous_result_run_version", Value::Null),
            ("previous_result_sha256", Value::Null),
            ("run_id", Value::Text(run_id.clone())),
            ("run_context_sha256", Value::Text(context_sha.clone())),
            ("input_sha256", Value::Text(input_sha.clone())),
            ("lease_owner", Value::Text(OWNER_V9.to_owned())),
            ("lease_generation", Value::Integer(3)),
            ("prior_head_version", Value::Integer(i64::try_from(v8_head + 2).unwrap())),
            ("run_version", Value::Integer(i64::try_from(v8_head + 3).unwrap())),
            ("begun_at", Value::Integer(rpc_at)),
        ] {
            assert_eq!(value_at(&begin_columns, &begin, name), &expected, "TEST_CODE begin {name}");
        }

        let (result_columns, result) = single_named_row(
            connection, "chain_post_close_position_concept_rpc_attempt_results"
        );
        let result_bytes = blob(value_at(&result_columns, &result, "result_bytes")).to_vec();
        let result_sha = hex::encode(Sha256::digest(&result_bytes));
        let result_envelope: serde_json::Value = serde_json::from_slice(&result_bytes).unwrap();
        let response_wire: Vec<u8> = serde_json::from_value(result_envelope["response_wire"].clone()).unwrap();
        assert_eq!(result_envelope, serde_json::json!({
            "schema_version": 1,
            "response_wire": response_wire,
            "status_code": null,
            "status_details": null,
            "status_error_detail_trailer": "Absent",
            "retry_decision": "NoRetry",
            "continuation": "Terminal",
            "backoff_ms": null,
        }));
        let response = QueryResponse::decode(response_wire.as_slice()).unwrap();
        assert_eq!(response.encode_to_vec(), response_wire);
        assert_eq!(response.request_id, observed.request_id);
        assert_eq!(response.operation, Operation::BoardConstituents as i32);
        assert_eq!(response.admission, AdmissionState::Admitted as i32);
        assert_eq!(response.selected_provider, "Tdx");
        assert!(response.complete);
        assert_eq!(response.source, "TEST_CODE_LOOPBACK_MEMBERSHIP_SOURCE");
        assert_eq!(response.source_at, "2026-07-21T15:30:00+08:00");
        assert_eq!(response.observed_at, "2026-07-21T15:31:00+08:00");
        assert_eq!(response.batch_id, "TEST_CODE_MEMBERSHIP_BATCH");
        assert!(response.diagnostic_blocker.is_empty());
        assert_eq!(response.records.len(), 1);
        assert_eq!(response.records[0].schema, "board.constituents");
        assert_eq!(response.records[0].schema_version, 1);
        assert_eq!(response.records[0].content_type, "application/json; charset=utf-8");
        assert_eq!(response.records[0].data, PAYLOAD);
        for (name, expected) in [
            ("intent_id", Value::Text(intent.as_str().to_owned())),
            ("cache_material_run_version", Value::Integer(i64::try_from(cache_version).unwrap())),
            ("position_ordinal", Value::Integer(1)),
            ("attempt_ordinal", Value::Integer(1)),
            ("begin_run_version", Value::Integer(i64::try_from(v8_head + 3).unwrap())),
            ("request_sha256", Value::Text(request_sha.clone())),
            ("wire_outcome", Value::Text("Response".to_owned())),
            ("result_codec_version", Value::Integer(1)),
            ("result_length", Value::Integer(i64::try_from(result_bytes.len()).unwrap())),
            ("result_sha256", Value::Text(result_sha.clone())),
            ("continuation", Value::Text("Terminal".to_owned())),
            ("retry_decision", Value::Text("NoRetry".to_owned())),
            ("backoff_ms", Value::Null),
            ("run_id", Value::Text(run_id.clone())),
            ("run_context_sha256", Value::Text(context_sha.clone())),
            ("input_sha256", Value::Text(input_sha.clone())),
            ("lease_owner", Value::Text(OWNER_V9.to_owned())),
            ("lease_generation", Value::Integer(3)),
            ("prior_head_version", Value::Integer(i64::try_from(v8_head + 3).unwrap())),
            ("run_version", Value::Integer(i64::try_from(v8_head + 4).unwrap())),
            ("returned_at", Value::Integer(rpc_at)),
            ("committed_at", Value::Integer(rpc_at)),
        ] {
            assert_eq!(value_at(&result_columns, &result, name), &expected, "TEST_CODE result {name}");
        }
        assert_eq!(value_at(&result_columns, &result, "result_bytes"), &Value::Blob(result_bytes));
        assert!(all_rows(connection, "SELECT * FROM chain_post_close_position_concept_rpc_status_materials").is_empty());
        assert!(all_rows(connection, "SELECT * FROM chain_post_close_position_concept_rpc_error_materials").is_empty());

        let (final_columns, final_row) = single_named_row(
            connection, "chain_post_close_position_concept_rpc_finals"
        );
        let final_bytes = blob(value_at(&final_columns, &final_row, "final_bytes")).to_vec();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&final_bytes).unwrap(),
            serde_json::json!({"schema_version": 1, "outcome": "Available", "raw": TOOL_JSON})
        );
        let final_sha = hex::encode(Sha256::digest(&final_bytes));
        let receipt = DataAcquisitionAuditReceipt {
            audit_id: integer(value_at(&final_columns, &final_row, "audit_id")),
            record_hash: match value_at(&final_columns, &final_row, "audit_record_hash") {
                Value::Text(value) => value.clone(),
                other => panic!("TEST_CODE receipt hash text: {other:?}"),
            },
            previous_outcome: None,
            current_outcome: "available".to_owned(),
        };
        assert_eq!(receipt.audit_id, 3);
        for (name, expected) in [
            ("intent_id", Value::Text(intent.as_str().to_owned())),
            ("cache_material_run_version", Value::Integer(i64::try_from(cache_version).unwrap())),
            ("position_ordinal", Value::Integer(1)),
            ("code", Value::Text(MISSING.to_owned())),
            ("provenance", Value::Text("PositionConceptRpc".to_owned())),
            ("occurrence_run_version", Value::Integer(i64::try_from(v8_head + 2).unwrap())),
            ("occurrence_request_sha256", Value::Text(request_sha.clone())),
            ("terminal_attempt_ordinal", Value::Integer(1)),
            ("terminal_result_run_version", Value::Integer(i64::try_from(v8_head + 4).unwrap())),
            ("terminal_result_sha256", Value::Text(result_sha)),
            ("error_material_run_version", Value::Null),
            ("error_material_sha256", Value::Null),
            ("final_outcome", Value::Text("Available".to_owned())),
            ("final_codec_version", Value::Integer(1)),
            ("final_length", Value::Integer(i64::try_from(final_bytes.len()).unwrap())),
            ("final_sha256", Value::Text(final_sha.clone())),
            ("audit_id", Value::Integer(receipt.audit_id)),
            ("audit_record_hash", Value::Text(receipt.record_hash.clone())),
            ("previous_outcome", Value::Null),
            ("current_outcome", Value::Text("available".to_owned())),
            ("run_id", Value::Text(run_id.clone())),
            ("run_context_sha256", Value::Text(context_sha.clone())),
            ("input_sha256", Value::Text(input_sha.clone())),
            ("lease_owner", Value::Text(OWNER_V9.to_owned())),
            ("lease_generation", Value::Integer(3)),
            ("prior_head_version", Value::Integer(i64::try_from(v8_head + 4).unwrap())),
            ("run_version", Value::Integer(i64::try_from(v8_head + 5).unwrap())),
            ("applied_at", Value::Integer(rpc_at)),
        ] {
            assert_eq!(value_at(&final_columns, &final_row, name), &expected, "TEST_CODE final {name}");
        }
        assert_eq!(value_at(&final_columns, &final_row, "final_bytes"), &Value::Blob(final_bytes));

        let cache_time = DateTime::parse_from_rfc3339("2026-07-21T15:37:02+08:00")
            .unwrap().with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string();
        let (write_columns, write) = single_named_row(
            connection, "chain_post_close_position_concept_cache_writes"
        );
        for (name, expected) in [
            ("intent_id", Value::Text(intent.as_str().to_owned())),
            ("cache_material_run_version", Value::Integer(i64::try_from(cache_version).unwrap())),
            ("position_ordinal", Value::Integer(1)),
            ("code", Value::Text(MISSING.to_owned())),
            ("final_run_version", Value::Integer(i64::try_from(v8_head + 5).unwrap())),
            ("final_sha256", Value::Text(final_sha)),
            ("terminal_result_run_version", Value::Integer(i64::try_from(v8_head + 4).unwrap())),
            ("concepts_codec_version", Value::Integer(1)),
            ("concepts_length", Value::Integer(i64::try_from(CACHE.len()).unwrap())),
            ("concepts_sha256", Value::Text(hex::encode(Sha256::digest(CACHE)))),
            ("cache_updated_at", Value::Text(cache_time.clone())),
            ("run_id", Value::Text(run_id)),
            ("run_context_sha256", Value::Text(context_sha)),
            ("input_sha256", Value::Text(input_sha)),
            ("lease_owner", Value::Text(OWNER_V9.to_owned())),
            ("lease_generation", Value::Integer(3)),
            ("prior_head_version", Value::Integer(i64::try_from(v8_head + 5).unwrap())),
            ("run_version", Value::Integer(i64::try_from(v8_head + 6).unwrap())),
            ("written_at", Value::Integer(rpc_at)),
        ] {
            assert_eq!(value_at(&write_columns, &write, name), &expected, "TEST_CODE cache {name}");
        }
        assert_eq!(value_at(&write_columns, &write, "concepts_bytes"), &Value::Blob(CACHE.to_vec()));
        assert_eq!(
            all_rows(connection, "SELECT code,concepts,CAST(updated_at AS TEXT) FROM stock_concepts WHERE code='TEST_CODE_600001'"),
            vec![vec![Value::Text(MISSING.to_owned()), Value::Text(std::str::from_utf8(CACHE).unwrap().to_owned()), Value::Text(cache_time)]]
        );

        assert_eq!(
            connection.query_row(
                "SELECT count(*) FROM chain_post_close_position_concept_rpc_finals AS final \
                 JOIN data_acquisition_audit AS audit ON audit.id=final.audit_id \
                 JOIN data_acquisition_audit_chain AS chain ON chain.acquisition_audit_id=final.audit_id \
                 WHERE final.audit_record_hash=chain.record_hash \
                   AND final.current_outcome=audit.outcome AND audit.request_hash=?1",
                [MEMBERSHIP_REQUEST_HASH], |row| row.get::<_, i64>(0),
            ).unwrap(),
            1
        );
        let transaction = connection.unchecked_transaction().unwrap();
        let verified = read_acquisition_in_transaction(&transaction, &receipt).unwrap();
        assert_eq!(verified.receipt(), &receipt);
        let record = verified.record();
        assert_eq!(record.capability, "board-memberships");
        assert_eq!(record.provider, "Tdx");
        assert_eq!(record.source, "TEST_CODE_LOOPBACK_MEMBERSHIP_SOURCE");
        assert_eq!(record.request_hash, MEMBERSHIP_REQUEST_HASH);
        assert_eq!(record.source_at, Some("2026-07-21T15:30:00+08:00"));
        assert_eq!(record.observed_at, "2026-07-21T15:31:00+08:00");
        assert_eq!(record.batch_id, Some("TEST_CODE_MEMBERSHIP_BATCH"));
        assert_eq!(record.outcome, "available");
        assert_eq!((record.request_count, record.accepted_count, record.rejected_count), (1, 2, 0));
        assert_eq!(record.reason_code, "accepted");
        assert!(!record.retryable);
        transaction.rollback().unwrap();

        assert_eq!(old_fact_rows(connection, &old_names), old_facts);
        assert_eq!(all_rows(connection, "SELECT * FROM chain_post_close_position_materials"), saved_position_rows);
        assert_eq!(all_rows(connection, "SELECT * FROM chain_post_close_position_concept_materials"), saved_cache_material_rows);
        let audits_after_v9 = all_rows(connection, "SELECT * FROM data_acquisition_audit ORDER BY id");
        let chain_after_v9 = all_rows(connection, "SELECT * FROM data_acquisition_audit_chain ORDER BY acquisition_audit_id");
        assert_eq!(&audits_after_v9[..old_audits.0.len()], old_audits.0.as_slice());
        assert_eq!(&chain_after_v9[..old_audits.1.len()], old_audits.1.as_slice());
        assert_eq!((audits_after_v9.len(), chain_after_v9.len()), (3, 3));
        assert_eq!(fixture.stored_context(&intent), old_context);
        assert_eq!(fixture.stored_input(&intent), old_input);
        assert_eq!(fixture.foundation_catalog(), old_foundation);
        assert_eq!(
            (
                fixture.connection().query_row("PRAGMA main.user_version", [], |row| row.get::<_, i64>(0)).unwrap(),
                fixture.connection().query_row("PRAGMA main.application_id", [], |row| row.get::<_, i64>(0)).unwrap(),
            ),
            old_identity
        );
        let saved_v9_facts = v9_fact_rows(connection);
        assert_eq!(
            saved_v9_facts.iter().map(|(name, rows)| (name.as_str(), rows.len())).collect::<Vec<_>>(),
            [
                ("chain_post_close_position_concept_cache_writes", 1),
                ("chain_post_close_position_concept_rpc_attempt_begins", 1),
                ("chain_post_close_position_concept_rpc_attempt_results", 1),
                ("chain_post_close_position_concept_rpc_error_materials", 0),
                ("chain_post_close_position_concept_rpc_finals", 1),
                ("chain_post_close_position_concept_rpc_occurrences", 1),
                ("chain_post_close_position_concept_rpc_status_materials", 0),
            ]
        );
        let saved_audits = (audits_after_v9, chain_after_v9);

        fixture.execute(
            "UPDATE stock_position SET name='TEST_CODE_CHANGED_POSITION',status='closed' WHERE id=1; \
             INSERT INTO stock_position VALUES \
               (5,'TEST_CODE_NEW_POSITION','TEST_CODE新增','2026-07-23',50.0,100,'open', \
                NULL,NULL,77.0,'2026-07-23 09:05:06','2026-07-23 14:05:06',NULL,NULL); \
             UPDATE stock_concepts \
               SET concepts='[\"TEST_CODE_CHANGED_CACHE\"]',updated_at='2026-07-23 14:05:00' \
               WHERE code='TEST_CODE_600001';"
        );
        let changed_positions = all_rows(fixture.connection(), "SELECT * FROM stock_position ORDER BY id");
        let changed_cache = all_rows(fixture.connection(), "SELECT code,concepts,CAST(updated_at AS TEXT) FROM stock_concepts ORDER BY code");
        let (run_columns, run_before_reopen) = single_named_row(fixture.connection(), "chain_post_close_runs");
        fixture.reopen();

        let mut local = fixture.store.as_mut().unwrap()
            .single_user_local_chain_post_close(&config).unwrap();
        let lease = local.resume_run(
            &intent,
            lease_request(OWNER_REOPENED, 661_000_000, 960_000_000, Some(v9_head)),
        ).unwrap();
        let reopened_clock = PositionsClock {
            now: UtcMicros::try_new(micros("2026-07-21T15:42:02+08:00")).unwrap(),
            observation: None,
            cache_calls: Cell::new(0),
        };
        let mut io = local.position_concept_rpc_preparation_io_v9(
            lease, &queries, &reopened_clock, FixedClusterConfiguration::resolve(Some("2")),
        ).unwrap();
        let reopened = prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(), stocks, None, &mut io,
        ).await.expect_err("TEST_CODE reopened v9 stops at DragonTiger");
        assert_v9_position_failure(&reopened);
        drop(io);
        assert_eq!(reopened_clock.cache_calls.get(), 0);
        assert_eq!(local.inspect_run(&intent).unwrap().head_version(), v9_head + 1);
        drop(local);

        assert_eq!(server.membership_snapshot(), memberships);
        assert_eq!(server.snapshot(), board_requests);
        assert_eq!(v9_fact_rows(fixture.connection()), saved_v9_facts);
        assert_eq!(old_fact_rows(fixture.connection(), &old_names), old_facts);
        assert_eq!(
            (
                all_rows(fixture.connection(), "SELECT * FROM data_acquisition_audit ORDER BY id"),
                all_rows(fixture.connection(), "SELECT * FROM data_acquisition_audit_chain ORDER BY acquisition_audit_id"),
            ),
            saved_audits
        );
        assert_eq!(all_rows(fixture.connection(), "SELECT * FROM chain_post_close_position_materials"), saved_position_rows);
        assert_eq!(all_rows(fixture.connection(), "SELECT * FROM chain_post_close_position_concept_materials"), saved_cache_material_rows);
        assert_eq!(all_rows(fixture.connection(), "SELECT * FROM stock_position ORDER BY id"), changed_positions);
        assert_eq!(all_rows(fixture.connection(), "SELECT code,concepts,CAST(updated_at AS TEXT) FROM stock_concepts ORDER BY code"), changed_cache);
        assert_eq!(fixture.stored_context(&intent), old_context);
        assert_eq!(fixture.stored_input(&intent), old_input);
        assert_eq!(fixture.foundation_catalog(), old_foundation);
        assert_eq!(all_rows(fixture.connection(), "SELECT * FROM chain_post_close_schema ORDER BY schema_version"), old_schema);
        assert_eq!(all_rows(fixture.connection(), "SELECT * FROM chain_post_close_layouts ORDER BY layout_version"), migrated_layouts);
        assert_eq!(all_rows(fixture.connection(), "SELECT * FROM chain_post_close_layout_objects ORDER BY layout_version,name"), migrated_registry);
        assert_eq!(
            all_rows(
                fixture.connection(),
                "SELECT name,type,tbl_name,CAST(sql AS BLOB) FROM sqlite_schema \
                 WHERE name LIKE 'chain_post_close_%' ORDER BY name",
            ),
            migrated_catalog
        );
        let (after_columns, run_after_reopen) = single_named_row(fixture.connection(), "chain_post_close_runs");
        assert_only_run_cas_changed(
            &run_columns, &run_before_reopen, &after_columns, &run_after_reopen,
        );
        assert_eq!(value_at(&after_columns, &run_after_reopen, "lease_owner"), &Value::Text(OWNER_REOPENED.to_owned()));
        assert_eq!(value_at(&after_columns, &run_after_reopen, "lease_generation"), &Value::Integer(4));
        assert_eq!(value_at(&after_columns, &run_after_reopen, "head_version"), &Value::Integer(i64::try_from(v9_head + 1).unwrap()));
        assert_eq!(value_at(&after_columns, &run_after_reopen, "lease_until"), &Value::Integer(at(960_000_000)));
        assert_eq!(value_at(&after_columns, &run_after_reopen, "updated_at"), &Value::Integer(at(661_000_000)));
        assert_eq!(
            (
                fixture.connection().query_row("PRAGMA main.user_version", [], |row| row.get::<_, i64>(0)).unwrap(),
                fixture.connection().query_row("PRAGMA main.application_id", [], |row| row.get::<_, i64>(0)).unwrap(),
            ),
            old_identity
        );
        drop(queries);
        drop(source);
        assert_eq!(server.finish().await, board_requests);
    }).await.expect("TEST_CODE position concept v9 scenario deadline");
}

#[tokio::test]
async fn single_user_local_position_concept_rpc_exhausted_retry_reopens_without_provider_replay() {
    const OWNER_ERROR: &str = "TEST_CODE_POSITION_RPC_ERROR_OWNER_C";
    const OWNER_ERROR_REOPENED: &str = "TEST_CODE_POSITION_RPC_ERROR_OWNER_D";
    const GATEWAY_MESSAGE: &str =
        "gRPC BoardConstituents 查询失败: 服务不可用 (指数退避, 重新检查 health/capabilities)";
    const BUSINESS_ERROR: &str = "产业链 TEST_CODE_600001 板块拉取失败: GrpcBridge data gateway failed reason_code=no_verified_batch provider=Some(Tdx) retryable=true: gRPC BoardConstituents 查询失败: 服务不可用 (指数退避, 重新检查 health/capabilities)";
    const STATUS_BYTES: &[u8] =
        br#"{"schema_version":1,"projection_version":1,"safe_diagnostic":"[redacted-unclassified-status]"}"#;

    tokio::time::timeout(Duration::from_secs(30), async {
        let mut fixture = V2BusinessFixture::new();
        fixture.install_v2();
        fixture
            .chain_post_close()
            .migrate_schema_v2_to_v3()
            .unwrap();
        cluster_tests::install_business_rows(&fixture);
        fixture
            .chain_post_close()
            .migrate_schema_v3_to_v4()
            .unwrap();
        install_br159(&mut fixture);
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

        let (client, server) = spawn_membership_commit_failure_loopback().await;
        let source = GrpcSource::from_board_loopback_test_client(client);
        let queries = source.connected_board_queries().await.unwrap();
        let stocks = cluster_tests::cluster_stocks();
        let config = local_config(BUILD_A);
        let context = build_single_user_local_chain_post_close_context(
            &config,
            run_input("TEST_CODE_RUN_POSITION_CONCEPT_V9_EXHAUSTED"),
        )
        .unwrap();
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
                lease_request(OWNER_A, 1_000_000, 60_000_000, None),
            )
            .unwrap();
        let intent = lease.intent_id().clone();
        let v7_clock = ControlledClock::new(at(2_000_000));
        let mut io = local
            .concept_rpc_preparation_io_v7(
                lease,
                &queries,
                &v7_clock,
                FixedClusterConfiguration::resolve(Some("2")),
            )
            .unwrap();
        let v7_error = prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks.clone(),
            None,
            &mut io,
        )
        .await
        .expect_err("TEST_CODE v7 stops at Positions");
        assert!(matches!(
            v7_error.downcast_ref::<PreparationStop>(),
            Some(PreparationStop::StageNotMigrated {
                next: UnmigratedStage::Positions
            })
        ));
        drop(io);
        let v7_head = local.inspect_run(&intent).unwrap().head_version();
        drop(local);
        let board_requests = server.snapshot();
        assert_eq!(
            board_requests
                .requests
                .iter()
                .map(|request| request.kind.as_str())
                .collect::<Vec<_>>(),
            ["Industry", "Industry", "Concept"]
        );
        assert!(server.membership_snapshot().is_empty());

        fixture.execute(
            "CREATE TABLE stock_position ( \
               id INTEGER PRIMARY KEY AUTOINCREMENT, code TEXT NOT NULL, name TEXT NOT NULL, \
               buy_date TEXT NOT NULL, buy_price REAL NOT NULL, quantity INTEGER NOT NULL, \
               status TEXT NOT NULL, sell_date TEXT, sell_price REAL, return_rate REAL, \
               created_at TEXT NOT NULL, updated_at TEXT NOT NULL, chain_name TEXT, st_type TEXT, \
               UNIQUE(code,buy_date)); \
             INSERT INTO stock_position VALUES \
               (1,'TEST_CODE_CLUSTER_STOCK_A','成员','2026-07-21',10.0,100,'open',NULL,NULL,1.5, \
                '2026-07-21 09:01:02','2026-07-21 14:01:02','TEST_CODE原链',NULL), \
               (2,'TEST_CODE_600001','缺失','2026-07-20',20.0,200,'open',NULL,NULL,NULL, \
                '2026-07-20 09:02:03','2026-07-21 14:02:03',NULL,'ST'), \
               (3,'TEST_CODE_CLUSTER_POS_OTHER','无关','2026-07-19',30.0,100,'open',NULL,NULL,-2.0, \
                '2026-07-19 09:03:04','2026-07-21 14:03:04',NULL,'*ST'), \
               (4,'TEST_CODE_CLUSTER_CLOSED','关闭','2026-07-22',40.0,100,'closed','2026-07-22',41.0,2.0, \
                '2026-07-22 09:04:05','2026-07-22 14:04:05',NULL,NULL); \
             INSERT INTO stock_concepts(code,concepts,updated_at) VALUES \
               ('TEST_CODE_CLUSTER_POS_OTHER','[\"TEST_CODE_CLUSTER_Z_OTHER\"]','2026-07-21 14:05:00');",
        );
        fixture
            .chain_post_close()
            .migrate_schema_v7_to_v8()
            .unwrap();
        let mut local = fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .unwrap();
        let lease = local
            .resume_run(
                &intent,
                lease_request(OWNER_B, 61_000_000, 360_000_000, Some(v7_head)),
            )
            .unwrap();
        let v8_clock = PositionsClock {
            now: UtcMicros::try_new(micros("2026-07-21T15:33:00+08:00")).unwrap(),
            observation: Some((
                DateTime::parse_from_rfc3339("2026-07-21T15:33:00+08:00").unwrap(),
                DateTime::parse_from_rfc3339("2026-07-14T15:33:00+08:00").unwrap(),
            )),
            cache_calls: Cell::new(0),
        };
        let mut io = local
            .positions_preparation_io_v8(
                lease,
                &queries,
                &v8_clock,
                FixedClusterConfiguration::resolve(Some("2")),
            )
            .unwrap();
        let v8_error = prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks.clone(),
            None,
            &mut io,
        )
        .await
        .expect_err("TEST_CODE v8 stops at PositionConceptProvider");
        assert!(matches!(
            v8_error.downcast_ref::<PreparationStop>(),
            Some(PreparationStop::StageNotMigrated {
                next: UnmigratedStage::PositionConceptProvider
            })
        ));
        drop(io);
        assert_eq!(v8_clock.cache_calls.get(), 1);
        let v8_head = local.inspect_run(&intent).unwrap().head_version();
        assert_eq!(v8_head, v7_head + 3);
        drop(local);

        let old_names = table_names(fixture.connection());
        let old_facts = old_fact_rows(fixture.connection(), &old_names);
        let old_audits = (
            all_rows(
                fixture.connection(),
                "SELECT * FROM data_acquisition_audit ORDER BY id",
            ),
            all_rows(
                fixture.connection(),
                "SELECT * FROM data_acquisition_audit_chain ORDER BY acquisition_audit_id",
            ),
        );
        assert_eq!((old_audits.0.len(), old_audits.1.len()), (2, 2));
        let parent_positions = all_rows(
            fixture.connection(),
            "SELECT * FROM chain_post_close_position_materials",
        );
        let parent_cache = all_rows(
            fixture.connection(),
            "SELECT * FROM chain_post_close_position_concept_materials",
        );
        let positions_source = all_rows(
            fixture.connection(),
            "SELECT * FROM stock_position ORDER BY id",
        );
        let cache_source = all_rows(
            fixture.connection(),
            "SELECT code,concepts,CAST(updated_at AS TEXT) FROM stock_concepts ORDER BY code",
        );
        assert!(!cache_source
            .iter()
            .any(|row| row.first() == Some(&Value::Text(MISSING.to_owned()))));

        fixture
            .chain_post_close()
            .migrate_schema_v8_to_v9()
            .unwrap();
        for _ in 0..4 {
            server.release_membership_unavailable();
        }
        let mut local = fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .unwrap();
        let lease = local
            .resume_run(
                &intent,
                lease_request(OWNER_ERROR, 361_000_000, 900_000_000, Some(v8_head)),
            )
            .unwrap();
        let error_clock = PositionsClock {
            now: UtcMicros::try_new(micros("2026-07-21T15:37:02+08:00")).unwrap(),
            observation: None,
            cache_calls: Cell::new(0),
        };
        let mut io = local
            .position_concept_rpc_preparation_io_v9(
                lease,
                &queries,
                &error_clock,
                FixedClusterConfiguration::resolve(Some("2")),
            )
            .unwrap();
        let error = prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks.clone(),
            None,
            &mut io,
        )
        .await
        .expect_err("TEST_CODE exhausted membership retry is a business failure");
        assert!(error.downcast_ref::<PreparationStop>().is_none());
        assert!(error.downcast_ref::<ChainPostCloseError>().is_none());
        let failure = error
            .downcast_ref::<PreparationFailure>()
            .expect("TEST_CODE business failure retains preparation observations");
        assert_eq!(failure.stage(), PreparationStage::PositionConcepts);
        assert_eq!(
            failure.completed_stages(),
            [
                PreparationStage::Concepts,
                PreparationStage::ClusterWritesAndLifecycle,
                PreparationStage::Candidates,
                PreparationStage::Positions,
            ]
        );
        assert_eq!(failure.reason(), BUSINESS_ERROR);
        assert_eq!(
            failure
                .positions()
                .iter()
                .map(|position| position.code())
                .collect::<Vec<_>>(),
            [STOCK_A, MISSING, POS_OTHER]
        );
        assert!(failure.position_concepts().is_empty());
        drop(io);
        assert_eq!(error_clock.cache_calls.get(), 0);
        let error_head = local.inspect_run(&intent).unwrap().head_version();
        assert_eq!(error_head, v8_head + 16);
        drop(local);

        let memberships = server.membership_snapshot();
        assert_eq!(memberships.len(), 4);
        assert!(memberships.iter().all(|request| request == &memberships[0]));
        let observed = &memberships[0];
        assert_eq!(observed.codes, [MISSING]);
        assert!(!observed.request_id.is_empty());
        assert!(observed.authorized);
        assert_eq!(observed.protocol_version, 1);
        assert_eq!(observed.payload_schema, "board.constituents");
        assert_eq!(observed.payload_schema_version, 1);
        assert_eq!(
            observed.payload_content_type,
            "application/json; charset=utf-8"
        );
        assert!(observed.preferred_provider.is_empty());
        assert!(!observed.allow_unadmitted);
        assert_eq!(server.snapshot(), board_requests);

        let connection = fixture.connection();
        let (occurrence_columns, occurrence) = single_named_row(
            connection,
            "chain_post_close_position_concept_rpc_occurrences",
        );
        let request_bytes =
            blob(value_at(&occurrence_columns, &occurrence, "request_bytes")).to_vec();
        let request_wire: Vec<u8> = serde_json::from_value(
            serde_json::from_slice::<serde_json::Value>(&request_bytes).unwrap()["request_wire"]
                .clone(),
        )
        .unwrap();
        let request = QueryRequest::decode(request_wire.as_slice()).unwrap();
        assert_eq!(request.encode_to_vec(), request_wire);
        let request_context = request.context.as_ref().unwrap();
        let request_payload = request.payload.as_ref().unwrap();
        assert_eq!(request_context.request_id, observed.request_id);
        assert_eq!(request_context.protocol_version, 1);
        assert_eq!(request_payload.schema, "board.constituents");
        assert_eq!(request_payload.schema_version, 1);
        assert_eq!(
            request_payload.content_type,
            "application/json; charset=utf-8"
        );
        assert_eq!(
            request_payload.data,
            br#"{"codes":["TEST_CODE_600001"]}"#
        );
        assert!(request.preferred_provider.is_empty());
        assert!(!request.allow_unadmitted);
        for (name, expected) in [
            ("position_ordinal", Value::Integer(1)),
            ("code", Value::Text(MISSING.to_owned())),
            ("operation", Value::Text("BoardConstituents".to_owned())),
            ("request_id", Value::Text(observed.request_id.clone())),
            ("profile", Value::Text("LocalBridgeV1".to_owned())),
            ("acquisition_authority", Value::Null),
            ("retry_max_attempts", Value::Integer(4)),
            ("retry_base_delay_ms", Value::Integer(1_000)),
            ("retry_max_delay_ms", Value::Integer(60_000)),
            ("retry_jitter_ms", Value::Integer(200)),
            (
                "run_version",
                Value::Integer(i64::try_from(v8_head + 2).unwrap()),
            ),
        ] {
            assert_eq!(
                value_at(&occurrence_columns, &occurrence, name),
                &expected,
                "TEST_CODE occurrence {name}"
            );
        }
        assert_eq!(
            value_at(&occurrence_columns, &occurrence, "request_sha256"),
            &Value::Text(hex::encode(Sha256::digest(&request_bytes)))
        );

        let mut statement = connection
            .prepare(
                "SELECT attempt_ordinal,CAST(result_bytes AS BLOB),result_length,result_sha256, \
                        continuation,retry_decision,backoff_ms,run_version,returned_at,committed_at \
                 FROM chain_post_close_position_concept_rpc_attempt_results \
                 ORDER BY attempt_ordinal",
            )
            .unwrap();
        let results = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, i64>(9)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        drop(statement);
        assert_eq!(results.len(), 4);
        let result_versions = [v8_head + 4, v8_head + 7, v8_head + 10, v8_head + 13];
        let continuations = ["Retry", "Retry", "Retry", "Terminal"];
        let backoffs = [Some(1_000), Some(2_000), Some(4_000), None];
        let mut result_shas = Vec::new();
        for (index, row) in results.iter().enumerate() {
            let (attempt, bytes, length, sha, continuation, decision, backoff, version, returned, committed) = row;
            assert_eq!(*attempt, i64::try_from(index + 1).unwrap());
            assert_eq!(*length, i64::try_from(bytes.len()).unwrap());
            assert_eq!(sha, &hex::encode(Sha256::digest(bytes)));
            assert_eq!(continuation, continuations[index]);
            assert_eq!(decision, "RetryBackoff");
            assert_eq!(*backoff, backoffs[index]);
            assert_eq!(*version, i64::try_from(result_versions[index]).unwrap());
            assert_eq!((*returned, *committed), (micros("2026-07-21T15:37:02+08:00"), micros("2026-07-21T15:37:02+08:00")));
            let raw: serde_json::Value = serde_json::from_slice(bytes).unwrap();
            assert!(raw["response_wire"].is_null());
            assert_eq!(raw["status_code"], 14);
            assert_eq!(raw["retry_decision"], "RetryBackoff");
            assert_eq!(raw["continuation"], continuations[index]);
            assert_eq!(
                raw["backoff_ms"],
                serde_json::to_value(backoffs[index]).unwrap()
            );
            let details: Vec<u8> = serde_json::from_value(raw["status_details"].clone()).unwrap();
            let trailer: Vec<u8> = serde_json::from_value(
                raw["status_error_detail_trailer"]["Bytes"].clone(),
            ).unwrap();
            assert_eq!(trailer, details);
            let detail = ErrorDetail::decode(details.as_slice()).unwrap();
            assert_eq!(detail.request_id, observed.request_id);
            assert_eq!(detail.operation, Operation::BoardConstituents as i32);
            assert_eq!(detail.provider, "Tdx");
            assert_eq!(detail.reason_code, "no_verified_batch");
            assert!(detail.retryable);
            result_shas.push(sha.clone());
        }

        let begins = all_rows(
            connection,
            "SELECT attempt_ordinal,occurrence_run_version,previous_attempt_ordinal, \
                    previous_result_run_version,previous_result_sha256,run_version \
             FROM chain_post_close_position_concept_rpc_attempt_begins ORDER BY attempt_ordinal",
        );
        assert_eq!(begins.len(), 4);
        for index in 0..4 {
            assert_eq!(begins[index][0], Value::Integer(i64::try_from(index + 1).unwrap()));
            assert_eq!(begins[index][1], Value::Integer(i64::try_from(v8_head + 2).unwrap()));
            let begin_version = [v8_head + 3, v8_head + 6, v8_head + 9, v8_head + 12][index];
            assert_eq!(begins[index][5], Value::Integer(i64::try_from(begin_version).unwrap()));
            if index == 0 {
                assert_eq!(&begins[index][2..5], &[Value::Null, Value::Null, Value::Null]);
            } else {
                assert_eq!(begins[index][2], Value::Integer(i64::try_from(index).unwrap()));
                assert_eq!(begins[index][3], Value::Integer(i64::try_from(result_versions[index - 1]).unwrap()));
                assert_eq!(begins[index][4], Value::Text(result_shas[index - 1].clone()));
            }
        }

        let status_rows = all_rows(
            connection,
            "SELECT attempt_ordinal,result_run_version,result_sha256,provenance,projection_version, \
                    material_codec_version,CAST(material_bytes AS BLOB), \
                    material_length,material_sha256,run_version,captured_at \
             FROM chain_post_close_position_concept_rpc_status_materials ORDER BY attempt_ordinal",
        );
        assert_eq!(status_rows.len(), 4);
        let status_sha = hex::encode(Sha256::digest(STATUS_BYTES));
        let status_versions = [v8_head + 5, v8_head + 8, v8_head + 11, v8_head + 14];
        for index in 0..4 {
            assert_eq!(status_rows[index][0], Value::Integer(i64::try_from(index + 1).unwrap()));
            assert_eq!(status_rows[index][1], Value::Integer(i64::try_from(result_versions[index]).unwrap()));
            assert_eq!(status_rows[index][2], Value::Text(result_shas[index].clone()));
            assert_eq!(status_rows[index][3], Value::Text("Captured".to_owned()));
            assert_eq!(status_rows[index][4], Value::Integer(1));
            assert_eq!(status_rows[index][5], Value::Integer(1));
            assert_eq!(status_rows[index][6], Value::Blob(STATUS_BYTES.to_vec()));
            assert_eq!(status_rows[index][7], Value::Integer(i64::try_from(STATUS_BYTES.len()).unwrap()));
            assert_eq!(status_rows[index][8], Value::Text(status_sha.clone()));
            assert_eq!(status_rows[index][9], Value::Integer(i64::try_from(status_versions[index]).unwrap()));
            assert_eq!(status_rows[index][10], Value::Integer(micros("2026-07-21T15:37:02+08:00")));
        }

        let (material_columns, material) = single_named_row(
            connection,
            "chain_post_close_position_concept_rpc_error_materials",
        );
        let material_bytes = blob(value_at(&material_columns, &material, "material_bytes")).to_vec();
        let material_json: serde_json::Value = serde_json::from_slice(&material_bytes).unwrap();
        assert_eq!(material_json, serde_json::json!({
            "schema_version": 1,
            "gateway": {
                "capability": "GrpcBridge", "provider": "Tdx", "audit_outcome": "unavailable",
                "reason_code": "no_verified_batch", "retryable": true, "message": GATEWAY_MESSAGE,
            },
            "audit": {
                "provider": "Tdx", "source": "review-data-gateway",
                "request_hash": MEMBERSHIP_REQUEST_HASH, "source_at": null,
                "observed_at": "2026-07-21T07:37:02.000Z", "batch_id": null,
                "outcome": "unavailable", "request_count": 1, "accepted_count": 0,
                "rejected_count": 1, "reason_code": "no_verified_batch", "retryable": true,
            },
        }));
        let material_sha = hex::encode(Sha256::digest(&material_bytes));
        for (name, expected) in [
            ("position_ordinal", Value::Integer(1)),
            ("terminal_attempt_ordinal", Value::Integer(4)),
            ("terminal_result_run_version", Value::Integer(i64::try_from(v8_head + 13).unwrap())),
            ("terminal_result_sha256", Value::Text(result_shas[3].clone())),
            ("status_material_attempt_ordinal", Value::Integer(4)),
            ("status_material_run_version", Value::Integer(i64::try_from(v8_head + 14).unwrap())),
            ("status_material_sha256", Value::Text(status_sha)),
            ("material_length", Value::Integer(i64::try_from(material_bytes.len()).unwrap())),
            ("material_sha256", Value::Text(material_sha.clone())),
            (
                "observed_fallback",
                Value::Text("2026-07-21T07:37:02.000Z".to_owned()),
            ),
            ("lease_owner", Value::Text(OWNER_ERROR.to_owned())),
            ("lease_generation", Value::Integer(3)),
            ("run_version", Value::Integer(i64::try_from(v8_head + 15).unwrap())),
            (
                "captured_at",
                Value::Integer(micros("2026-07-21T15:37:02+08:00")),
            ),
        ] {
            assert_eq!(value_at(&material_columns, &material, name), &expected, "TEST_CODE error material {name}");
        }

        let (final_columns, final_row) = single_named_row(
            connection,
            "chain_post_close_position_concept_rpc_finals",
        );
        let final_bytes = blob(value_at(&final_columns, &final_row, "final_bytes")).to_vec();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&final_bytes).unwrap(),
            serde_json::json!({"schema_version": 1, "outcome": "Error", "raw": BUSINESS_ERROR})
        );
        let receipt = DataAcquisitionAuditReceipt {
            audit_id: integer(value_at(&final_columns, &final_row, "audit_id")),
            record_hash: match value_at(&final_columns, &final_row, "audit_record_hash") {
                Value::Text(value) => value.clone(),
                other => panic!("TEST_CODE error receipt hash: {other:?}"),
            },
            previous_outcome: None,
            current_outcome: "unavailable".to_owned(),
        };
        assert_eq!(receipt.audit_id, 3);
        for (name, expected) in [
            ("position_ordinal", Value::Integer(1)),
            ("code", Value::Text(MISSING.to_owned())),
            ("terminal_attempt_ordinal", Value::Integer(4)),
            ("terminal_result_run_version", Value::Integer(i64::try_from(v8_head + 13).unwrap())),
            ("terminal_result_sha256", Value::Text(result_shas[3].clone())),
            ("error_material_run_version", Value::Integer(i64::try_from(v8_head + 15).unwrap())),
            ("error_material_sha256", Value::Text(material_sha)),
            ("final_outcome", Value::Text("Error".to_owned())),
            ("final_length", Value::Integer(i64::try_from(final_bytes.len()).unwrap())),
            ("final_sha256", Value::Text(hex::encode(Sha256::digest(&final_bytes)))),
            ("audit_id", Value::Integer(3)),
            ("current_outcome", Value::Text("unavailable".to_owned())),
            ("lease_owner", Value::Text(OWNER_ERROR.to_owned())),
            ("lease_generation", Value::Integer(3)),
            ("run_version", Value::Integer(i64::try_from(v8_head + 16).unwrap())),
            (
                "applied_at",
                Value::Integer(micros("2026-07-21T15:37:02+08:00")),
            ),
        ] {
            assert_eq!(value_at(&final_columns, &final_row, name), &expected, "TEST_CODE error final {name}");
        }
        assert_eq!(value_at(&final_columns, &final_row, "previous_outcome"), &Value::Null);
        let transaction = connection.unchecked_transaction().unwrap();
        let verified = read_acquisition_in_transaction(&transaction, &receipt).unwrap();
        let audit = verified.record();
        assert_eq!(audit.capability, "board-memberships");
        assert_eq!(audit.provider, "Tdx");
        assert_eq!(audit.source, "review-data-gateway");
        assert_eq!(audit.request_hash, MEMBERSHIP_REQUEST_HASH);
        assert_eq!(audit.source_at, None);
        assert_eq!(audit.observed_at, "2026-07-21T07:37:02.000Z");
        assert_eq!(audit.batch_id, None);
        assert_eq!(audit.outcome, "unavailable");
        assert_eq!((audit.request_count, audit.accepted_count, audit.rejected_count), (1, 0, 1));
        assert_eq!(audit.reason_code, "no_verified_batch");
        assert!(audit.retryable);
        transaction.rollback().unwrap();

        let saved_v9_facts = v9_fact_rows(connection);
        assert_eq!(
            saved_v9_facts
                .iter()
                .map(|(name, rows)| (name.as_str(), rows.len()))
                .collect::<Vec<_>>(),
            [
                ("chain_post_close_position_concept_cache_writes", 0),
                ("chain_post_close_position_concept_rpc_attempt_begins", 4),
                ("chain_post_close_position_concept_rpc_attempt_results", 4),
                ("chain_post_close_position_concept_rpc_error_materials", 1),
                ("chain_post_close_position_concept_rpc_finals", 1),
                ("chain_post_close_position_concept_rpc_occurrences", 1),
                ("chain_post_close_position_concept_rpc_status_materials", 4),
            ]
        );
        assert_eq!(old_fact_rows(connection, &old_names), old_facts);
        assert_eq!(all_rows(connection, "SELECT * FROM chain_post_close_position_materials"), parent_positions);
        assert_eq!(all_rows(connection, "SELECT * FROM chain_post_close_position_concept_materials"), parent_cache);
        let saved_audits = (
            all_rows(connection, "SELECT * FROM data_acquisition_audit ORDER BY id"),
            all_rows(connection, "SELECT * FROM data_acquisition_audit_chain ORDER BY acquisition_audit_id"),
        );
        assert_eq!(&saved_audits.0[..2], old_audits.0.as_slice());
        assert_eq!(&saved_audits.1[..2], old_audits.1.as_slice());
        assert_eq!((saved_audits.0.len(), saved_audits.1.len()), (3, 3));
        assert_eq!(all_rows(connection, "SELECT * FROM stock_position ORDER BY id"), positions_source);
        assert_eq!(all_rows(connection, "SELECT code,concepts,CAST(updated_at AS TEXT) FROM stock_concepts ORDER BY code"), cache_source);
        let (run_columns, run_before_reopen) = single_named_row(connection, "chain_post_close_runs");

        fixture.reopen();
        let mut local = fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .unwrap();
        let lease = local
            .resume_run(
                &intent,
                lease_request(OWNER_ERROR_REOPENED, 901_000_000, 1_200_000_000, Some(error_head)),
            )
            .unwrap();
        let reopened_clock = PositionsClock {
            now: UtcMicros::try_new(micros("2026-07-21T15:46:02+08:00")).unwrap(),
            observation: None,
            cache_calls: Cell::new(0),
        };
        let mut io = local
            .position_concept_rpc_preparation_io_v9(
                lease,
                &queries,
                &reopened_clock,
                FixedClusterConfiguration::resolve(Some("2")),
            )
            .unwrap();
        let reopened = prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks,
            None,
            &mut io,
        )
        .await
        .expect_err("TEST_CODE reopened exhausted retry remains business failure");
        assert!(reopened.downcast_ref::<PreparationStop>().is_none());
        assert!(reopened.downcast_ref::<ChainPostCloseError>().is_none());
        let reopened_failure = reopened.downcast_ref::<PreparationFailure>().unwrap();
        assert_eq!(reopened_failure.stage(), PreparationStage::PositionConcepts);
        assert_eq!(reopened_failure.reason(), BUSINESS_ERROR);
        assert_eq!(
            reopened_failure.completed_stages(),
            [
                PreparationStage::Concepts,
                PreparationStage::ClusterWritesAndLifecycle,
                PreparationStage::Candidates,
                PreparationStage::Positions,
            ]
        );
        assert!(reopened_failure.position_concepts().is_empty());
        drop(io);
        assert_eq!(reopened_clock.cache_calls.get(), 0);
        assert_eq!(local.inspect_run(&intent).unwrap().head_version(), error_head + 1);
        drop(local);

        assert_eq!(server.membership_snapshot(), memberships);
        assert_eq!(server.snapshot(), board_requests);
        assert_eq!(v9_fact_rows(fixture.connection()), saved_v9_facts);
        assert_eq!(old_fact_rows(fixture.connection(), &old_names), old_facts);
        assert_eq!(
            (
                all_rows(fixture.connection(), "SELECT * FROM data_acquisition_audit ORDER BY id"),
                all_rows(fixture.connection(), "SELECT * FROM data_acquisition_audit_chain ORDER BY acquisition_audit_id"),
            ),
            saved_audits
        );
        assert_eq!(all_rows(fixture.connection(), "SELECT * FROM chain_post_close_position_materials"), parent_positions);
        assert_eq!(all_rows(fixture.connection(), "SELECT * FROM chain_post_close_position_concept_materials"), parent_cache);
        assert_eq!(all_rows(fixture.connection(), "SELECT * FROM stock_position ORDER BY id"), positions_source);
        assert_eq!(all_rows(fixture.connection(), "SELECT code,concepts,CAST(updated_at AS TEXT) FROM stock_concepts ORDER BY code"), cache_source);
        let (after_columns, run_after_reopen) = single_named_row(fixture.connection(), "chain_post_close_runs");
        assert_only_run_cas_changed(&run_columns, &run_before_reopen, &after_columns, &run_after_reopen);
        assert_eq!(value_at(&after_columns, &run_after_reopen, "lease_owner"), &Value::Text(OWNER_ERROR_REOPENED.to_owned()));
        assert_eq!(value_at(&after_columns, &run_after_reopen, "lease_generation"), &Value::Integer(4));
        assert_eq!(value_at(&after_columns, &run_after_reopen, "head_version"), &Value::Integer(i64::try_from(error_head + 1).unwrap()));
        assert_eq!(value_at(&after_columns, &run_after_reopen, "lease_until"), &Value::Integer(at(1_200_000_000)));
        assert_eq!(value_at(&after_columns, &run_after_reopen, "updated_at"), &Value::Integer(at(901_000_000)));
        drop(queries);
        drop(source);
        assert_eq!(server.finish().await, board_requests);
    })
    .await
    .expect("TEST_CODE exhausted position concept retry deadline");
}

#[path = "chain_post_close_dragon_tiger_tests.rs"]
mod dragon_tiger_tests;

#[path = "chain_post_close_position_concept_order_tests.rs"]
mod position_concept_order_tests;
