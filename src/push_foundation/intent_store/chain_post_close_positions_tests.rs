use super::*;
use crate::data_gateway::grpc_source::GrpcSource;
use crate::database::data_acquisition_audit::install_acquisition_schema_for_test;
use crate::grpc_client::client::board_loopback_fixture::spawn_board_loopback;
use crate::pipeline::chain_analysis::preparation::{
    FixedClusterConfiguration, PositionCacheObservation, PositionObservationClock, PreparationStage,
};
use diesel::{Connection as _, SqliteConnection};
use rusqlite::types::Value;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::time::Duration;

const STOCK_A: &str = "TEST_CODE_CLUSTER_STOCK_A";
const POS_ALIAS: &str = "TEST_CODE_CLUSTER_POS_ALIAS";
const POS_OTHER: &str = "TEST_CODE_CLUSTER_POS_OTHER";
const OWNER_A: &str = "TEST_CODE_POSITIONS_OWNER_A";
const OWNER_B: &str = "TEST_CODE_POSITIONS_OWNER_B";
const OWNER_C: &str = "TEST_CODE_POSITIONS_OWNER_C";

fn micros(value: &str) -> i64 {
    DateTime::parse_from_rfc3339(value)
        .unwrap()
        .timestamp_micros()
}

fn install_br159(fixture: &mut V2BusinessFixture) {
    let store = fixture.store.take().expect("TEST_CODE owned intent store");
    store
        .connection
        .close()
        .expect("TEST_CODE close before BR159 installation");
    let database = fixture.database();
    let path = database
        .to_str()
        .expect("TEST_CODE UTF-8 owned business database");
    let mut connection =
        SqliteConnection::establish(path).expect("TEST_CODE owned Diesel connection");
    install_acquisition_schema_for_test(&mut connection)
        .expect("TEST_CODE install original BR159 schema");
    drop(connection);
    fixture.store = Some(BusinessIntentStore::open(&database).expect("TEST_CODE reopen store"));
}

fn all_rows(connection: &Connection, sql: &str) -> Vec<Vec<Value>> {
    let mut statement = connection.prepare(sql).expect("TEST_CODE SQL snapshot");
    let width = statement.column_count();
    statement
        .query_map([], |row| {
            (0..width)
                .map(|column| row.get(column))
                .collect::<rusqlite::Result<Vec<Value>>>()
        })
        .expect("TEST_CODE SQL snapshot query")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("TEST_CODE SQL snapshot rows")
}

fn table_names(connection: &Connection) -> Vec<String> {
    connection
        .prepare(
            "SELECT name FROM sqlite_schema WHERE type='table' \
             AND name LIKE 'chain_post_close_%' ORDER BY name",
        )
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn old_fact_rows(connection: &Connection, names: &[String]) -> BTreeMap<String, Vec<Vec<Value>>> {
    names
        .iter()
        .filter(|name| {
            !matches!(
                name.as_str(),
                "chain_post_close_runs"
                    | "chain_post_close_schema"
                    | "chain_post_close_layouts"
                    | "chain_post_close_layout_objects"
            )
        })
        .map(|name| {
            (
                name.clone(),
                all_rows(connection, &format!("SELECT * FROM \"{name}\"")),
            )
        })
        .collect()
}

fn single_named_row(connection: &Connection, table: &str) -> (Vec<String>, Vec<Value>) {
    let columns = connection
        .prepare(&format!("PRAGMA main.table_info('{table}')"))
        .unwrap()
        .query_map([], |row| row.get(1))
        .unwrap()
        .collect::<rusqlite::Result<Vec<String>>>()
        .unwrap();
    let rows = all_rows(connection, &format!("SELECT * FROM \"{table}\""));
    assert_eq!(rows.len(), 1, "TEST_CODE one {table} row");
    (columns, rows.into_iter().next().unwrap())
}

fn value_at<'a>(columns: &[String], row: &'a [Value], name: &str) -> &'a Value {
    &row[columns.iter().position(|column| column == name).unwrap()]
}

fn blob(value: &Value) -> &[u8] {
    match value {
        Value::Blob(bytes) => bytes,
        other => panic!("TEST_CODE expected material blob, got {other:?}"),
    }
}

fn assert_only_run_cas_changed(
    before_columns: &[String],
    before: &[Value],
    after_columns: &[String],
    after: &[Value],
) {
    assert_eq!(before_columns, after_columns);
    for (name, (left, right)) in before_columns.iter().zip(before.iter().zip(after)) {
        if !matches!(
            name.as_str(),
            "lease_owner" | "lease_generation" | "head_version" | "lease_until" | "updated_at"
        ) {
            assert_eq!(left, right, "TEST_CODE immutable run column {name}");
        }
    }
}

fn cache_rows(connection: &Connection) -> Vec<(String, String, String)> {
    connection
        .prepare(
            "SELECT code,concepts,CAST(updated_at AS TEXT) FROM stock_concepts \
             WHERE updated_at>='2026-07-14 15:33:00'",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn expected_cache_map() -> BTreeMap<String, Vec<String>> {
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
        (POS_ALIAS, vec!["TEST_CODE_CLUSTER_B_ALIAS"]),
        (POS_OTHER, vec!["TEST_CODE_CLUSTER_Z_OTHER"]),
    ]
    .into_iter()
    .map(|(code, tags)| {
        (
            code.to_owned(),
            tags.into_iter().map(str::to_owned).collect(),
        )
    })
    .collect()
}

fn expected_position_bytes(observed_at: i64) -> Vec<u8> {
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
            "{{\"id\":2,\"code\":\"TEST_CODE_CLUSTER_POS_ALIAS\",",
            "\"name\":\"别名\",\"buy_date\":\"2026-07-20\",",
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

fn expected_cache_bytes(
    observed_at: i64,
    positions_version: u64,
    positions_sha: &str,
    rows: &[(String, String, String)],
) -> Vec<u8> {
    let encoded_rows = rows
        .iter()
        .map(|(code, concepts, updated_at)| {
            format!(
                "{{\"code\":{},\"concepts\":{},\"updated_at\":{}}}",
                serde_json::to_string(code).unwrap(),
                serde_json::to_string(concepts).unwrap(),
                serde_json::to_string(updated_at).unwrap()
            )
        })
        .collect::<Vec<_>>()
        .join(",");
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
            "\"TEST_CODE_CLUSTER_POS_ALIAS\",\"TEST_CODE_CLUSTER_POS_OTHER\"],",
            "\"cache_rows\":[{encoded_rows}]}}"
        ),
        observed_at = observed_at,
        positions_version = positions_version,
        positions_sha = positions_sha,
        encoded_rows = encoded_rows
    )
    .into_bytes()
}

fn assert_position_failure(error: &anyhow::Error) {
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::StageNotMigrated {
            next: UnmigratedStage::DragonTiger
        })
    ));
    let failure = error
        .downcast_ref::<PreparationFailure>()
        .expect("TEST_CODE DragonTiger stop retains observations");
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
    assert_eq!(failure.positions().len(), 3);
    assert_eq!(
        failure
            .positions()
            .iter()
            .map(|position| (
                position.code(),
                position.name(),
                position.return_rate().map(f64::to_bits)
            ))
            .collect::<Vec<_>>(),
        [
            (STOCK_A, "成员", Some(0x3ff8000000000000)),
            (POS_ALIAS, "别名", None),
            (POS_OTHER, "无关", Some(0xc000000000000000)),
        ]
    );
    assert_eq!(failure.position_concepts(), &expected_cache_map());
    assert_eq!(failure.position_diags().len(), 3);
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
                POS_ALIAS,
                "别名",
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
    let candidate = &failure.candidate_sources()["TEST_CODE_CLUSTER_A_MAIN"];
    assert_eq!(candidate.status(), &SourceStatus::Unavailable);
    assert!(candidate.reason().unwrap().contains("unsupported"));
    assert!(failure.lhb_map().is_empty());
    assert_eq!(failure.lhb_source().status(), &SourceStatus::Unknown);
    assert_eq!(failure.macro_input(), None);
}

struct PositionsClock {
    now: UtcMicros,
    observation: Option<(
        chrono::DateTime<chrono::FixedOffset>,
        chrono::DateTime<chrono::FixedOffset>,
    )>,
    cache_calls: Cell<usize>,
}

impl ConceptEffectClock for PositionsClock {
    fn now(&self) -> UtcMicros {
        self.now
    }
}

impl PositionObservationClock for PositionsClock {
    fn cache_observation(&self) -> PositionCacheObservation {
        self.cache_calls.set(self.cache_calls.get() + 1);
        let (observed_local, cutoff_local) = self
            .observation
            .as_ref()
            .expect("TEST_CODE recovered position cache must not be resampled");
        PositionCacheObservation {
            observed_local: observed_local.to_owned(),
            cutoff_local: cutoff_local.to_owned(),
        }
    }
}

#[tokio::test]
async fn single_user_local_positions_and_fresh_position_cache_reopen_without_source_replay() {
    tokio::time::timeout(Duration::from_secs(45), async {
        let mut fixture = V2BusinessFixture::new();
        fixture.install_v2();
        fixture.chain_post_close().migrate_schema_v2_to_v3().unwrap();
        cluster_tests::install_business_rows(&fixture);
        fixture.chain_post_close().migrate_schema_v3_to_v4().unwrap();
        install_br159(&mut fixture);
        fixture.chain_post_close().migrate_schema_v4_to_v5().unwrap();
        fixture.chain_post_close().migrate_schema_v5_to_v6().unwrap();
        fixture.chain_post_close().migrate_schema_v6_to_v7().unwrap();

        let (client, server) = spawn_board_loopback().await;
        let source = GrpcSource::from_board_loopback_test_client(client);
        let queries = source.connected_board_queries().await.unwrap();
        let stocks = cluster_tests::cluster_stocks();
        let config = local_config(BUILD_A);
        let context = build_single_user_local_chain_post_close_context(
            &config,
            run_input("TEST_CODE_RUN_POSITIONS_V8"),
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

        let original_requests = server.snapshot();
        assert_eq!(
            original_requests
                .requests
                .iter()
                .map(|request| request.kind.as_str())
                .collect::<Vec<_>>(),
            ["Industry", "Industry", "Concept"]
        );
        assert_eq!(original_requests.non_board_requests, 0);
        assert!(server.membership_snapshot().is_empty());
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
        let foundation_catalog = fixture.foundation_catalog();
        let database_identity = (
            fixture
                .connection()
                .query_row("PRAGMA main.user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            fixture
                .connection()
                .query_row("PRAGMA main.application_id", [], |row| row.get::<_, i64>(0))
                .unwrap(),
        );
        let old_schema = all_rows(
            fixture.connection(),
            "SELECT * FROM chain_post_close_schema WHERE schema_version<=7 ORDER BY schema_version",
        );
        let old_layouts = all_rows(
            fixture.connection(),
            "SELECT * FROM chain_post_close_layouts WHERE layout_version<=7 ORDER BY layout_version",
        );
        let old_registry = all_rows(
            fixture.connection(),
            "SELECT * FROM chain_post_close_layout_objects WHERE layout_version<=7 \
             ORDER BY layout_version,name",
        );
        let old_catalog = all_rows(
            fixture.connection(),
            "SELECT name,type,tbl_name,CAST(sql AS BLOB) FROM sqlite_schema \
             WHERE name LIKE 'chain_post_close_%' ORDER BY name",
        );
        let stored_context = fixture.stored_context(&intent);
        let stored_input = fixture.stored_input(&intent);

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
               (2,'TEST_CODE_CLUSTER_POS_ALIAS','别名','2026-07-20',20.0,200,'open',NULL,NULL,NULL, \
                '2026-07-20 09:02:03','2026-07-21 14:02:03',NULL,'ST'), \
               (3,'TEST_CODE_CLUSTER_POS_OTHER','无关','2026-07-19',30.0,100,'open',NULL,NULL,-2.0, \
                '2026-07-19 09:03:04','2026-07-21 14:03:04',NULL,'*ST'), \
               (4,'TEST_CODE_CLUSTER_CLOSED','关闭','2026-07-22',40.0,100,'closed','2026-07-22',41.0,2.0, \
                '2026-07-22 09:04:05','2026-07-22 14:04:05',NULL,NULL); \
             INSERT INTO stock_concepts(code,concepts,updated_at) VALUES \
               ('TEST_CODE_CLUSTER_POS_ALIAS','[\"TEST_CODE_CLUSTER_B_ALIAS\"]','2026-07-21 14:05:00'), \
               ('TEST_CODE_CLUSTER_POS_OTHER','[\"TEST_CODE_CLUSTER_Z_OTHER\"]','2026-07-21 14:05:00');",
        );
        let first_cache_rows = cache_rows(fixture.connection());
        assert_eq!(first_cache_rows.len(), 5);
        let parsed_cache = first_cache_rows
            .iter()
            .map(|(code, raw, _)| {
                (
                    code.clone(),
                    serde_json::from_str::<Vec<String>>(raw).unwrap(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(parsed_cache, expected_cache_map());

        assert_eq!(
            fixture
                .chain_post_close()
                .migrate_schema_v7_to_v8()
                .unwrap()
                .schema_version(),
            8
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
                lease_request(OWNER_B, 61_000_000, 360_000_000, Some(v7_head)),
            )
            .unwrap();
        let observed_local = DateTime::parse_from_rfc3339("2026-07-21T15:33:00+08:00").unwrap();
        let first_clock = PositionsClock {
            now: UtcMicros::try_new(micros("2026-07-21T15:33:00+08:00")).unwrap(),
            observation: Some((
                observed_local,
                DateTime::parse_from_rfc3339("2026-07-14T15:33:00+08:00").unwrap(),
            )),
            cache_calls: Cell::new(0),
        };
        let mut io = local
            .positions_preparation_io_v8(
                lease,
                &queries,
                &first_clock,
                FixedClusterConfiguration::resolve(Some("2")),
            )
            .unwrap();
        let first_error = prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks.clone(),
            None,
            &mut io,
        )
        .await
        .expect_err("TEST_CODE v8 stops at DragonTiger");
        assert_position_failure(&first_error);
        drop(io);
        assert_eq!(first_clock.cache_calls.get(), 1);
        // Resume advances once; the two material CAS writes advance twice more.
        let v8_head = local.inspect_run(&intent).unwrap().head_version();
        assert_eq!(v8_head, v7_head + 3);
        drop(local);
        assert_eq!(server.snapshot(), original_requests);
        assert!(server.membership_snapshot().is_empty());

        let (run_id, context_sha, input_sha): (String, String, String) = fixture
            .connection()
            .query_row(
                "SELECT run_id,run_context_sha256,input_sha256 FROM chain_post_close_runs \
                 WHERE intent_id=?1",
                [intent.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        let position_rows = all_rows(
            fixture.connection(),
            "SELECT intent_id,run_id,run_context_sha256,input_sha256,material_codec_version, \
                    material_bytes,material_length,material_sha256,lease_owner,lease_generation, \
                    prior_head_version,run_version,observed_at,committed_at,row_count \
             FROM chain_post_close_position_materials",
        );
        assert_eq!(position_rows.len(), 1);
        let positions = &position_rows[0];
        let position_bytes = expected_position_bytes(micros("2026-07-21T15:33:00+08:00"));
        let position_sha = hex::encode(Sha256::digest(&position_bytes));
        assert_eq!(serde_json::from_slice::<serde_json::Value>(blob(&positions[5])).unwrap(), serde_json::from_slice::<serde_json::Value>(&position_bytes).unwrap());
        assert_eq!(
            positions,
            &vec![
                Value::Text(intent.as_str().to_owned()),
                Value::Text(run_id.clone()),
                Value::Text(context_sha.clone()),
                Value::Text(input_sha.clone()),
                Value::Integer(1),
                Value::Blob(position_bytes.clone()),
                Value::Integer(i64::try_from(position_bytes.len()).unwrap()),
                Value::Text(position_sha.clone()),
                Value::Text(OWNER_B.to_owned()),
                Value::Integer(2),
                Value::Integer(i64::try_from(v7_head + 1).unwrap()),
                Value::Integer(i64::try_from(v7_head + 2).unwrap()),
                Value::Integer(micros("2026-07-21T15:33:00+08:00")),
                Value::Integer(micros("2026-07-21T15:33:00+08:00")),
                Value::Integer(3),
            ]
        );

        let concept_rows = all_rows(
            fixture.connection(),
            "SELECT intent_id,run_id,run_context_sha256,input_sha256,material_codec_version, \
                    material_bytes,material_length,material_sha256,lease_owner,lease_generation, \
                    prior_head_version,run_version,observed_at,committed_at,batch_kind, \
                    positions_run_version,positions_sha256,requested_count,cache_row_count, \
                    cache_max_age_days,cache_cutoff,local_offset_seconds,cutoff_local_offset_seconds \
             FROM chain_post_close_position_concept_materials",
        );
        assert_eq!(concept_rows.len(), 1);
        let concepts = &concept_rows[0];
        let concept_bytes = expected_cache_bytes(
            micros("2026-07-21T15:33:00+08:00"),
            v7_head + 2,
            &position_sha,
            &first_cache_rows,
        );
        let concept_sha = hex::encode(Sha256::digest(&concept_bytes));
        assert_eq!(serde_json::from_slice::<serde_json::Value>(blob(&concepts[5])).unwrap(), serde_json::from_slice::<serde_json::Value>(&concept_bytes).unwrap());
        assert_eq!(
            concepts,
            &vec![
                Value::Text(intent.as_str().to_owned()),
                Value::Text(run_id),
                Value::Text(context_sha),
                Value::Text(input_sha),
                Value::Integer(1),
                Value::Blob(concept_bytes.clone()),
                Value::Integer(i64::try_from(concept_bytes.len()).unwrap()),
                Value::Text(concept_sha),
                Value::Text(OWNER_B.to_owned()),
                Value::Integer(2),
                Value::Integer(i64::try_from(v7_head + 2).unwrap()),
                Value::Integer(i64::try_from(v7_head + 3).unwrap()),
                Value::Integer(micros("2026-07-21T15:33:00+08:00")),
                Value::Integer(micros("2026-07-21T15:33:00+08:00")),
                Value::Text("PositionConcepts".to_owned()),
                Value::Integer(i64::try_from(v7_head + 2).unwrap()),
                Value::Text(position_sha),
                Value::Integer(3),
                Value::Integer(5),
                Value::Integer(7),
                Value::Text("2026-07-14 15:33:00".to_owned()),
                Value::Integer(28_800),
                Value::Integer(28_800),
            ]
        );
        assert_eq!(fixture.stored_context(&intent), stored_context);
        assert_eq!(fixture.stored_input(&intent), stored_input);
        assert_eq!(old_fact_rows(fixture.connection(), &old_names), old_facts);
        assert_eq!(
            (
                all_rows(
                    fixture.connection(),
                    "SELECT * FROM data_acquisition_audit ORDER BY id"
                ),
                all_rows(
                    fixture.connection(),
                    "SELECT * FROM data_acquisition_audit_chain ORDER BY acquisition_audit_id"
                ),
            ),
            old_audits
        );
        assert_eq!(fixture.foundation_catalog(), foundation_catalog);
        assert_eq!(
            (
                fixture
                    .connection()
                    .query_row("PRAGMA main.user_version", [], |row| row.get::<_, i64>(0))
                    .unwrap(),
                fixture
                    .connection()
                    .query_row("PRAGMA main.application_id", [], |row| row.get::<_, i64>(0))
                    .unwrap(),
            ),
            database_identity
        );
        assert_eq!(
            all_rows(
                fixture.connection(),
                "SELECT * FROM chain_post_close_schema WHERE schema_version<=7 ORDER BY schema_version"
            ),
            old_schema
        );
        assert_eq!(
            all_rows(
                fixture.connection(),
                "SELECT * FROM chain_post_close_layouts WHERE layout_version<=7 ORDER BY layout_version"
            ),
            old_layouts
        );
        assert_eq!(
            all_rows(
                fixture.connection(),
                "SELECT * FROM chain_post_close_layout_objects WHERE layout_version<=7 \
                 ORDER BY layout_version,name"
            ),
            old_registry
        );
        let migrated_catalog = all_rows(
            fixture.connection(),
            "SELECT name,type,tbl_name,CAST(sql AS BLOB) FROM sqlite_schema \
             WHERE name LIKE 'chain_post_close_%' ORDER BY name",
        );
        assert!(old_catalog.iter().all(|row| migrated_catalog.contains(row)));
        let migrated_layouts = all_rows(
            fixture.connection(),
            "SELECT * FROM chain_post_close_layouts ORDER BY layout_version",
        );

        fixture.execute(
            "UPDATE stock_position SET name='TEST_CODE_CHANGED',return_rate=99.0,status='closed' \
               WHERE id=1; \
             UPDATE stock_position SET name='TEST_CODE_CHANGED_ALIAS',return_rate=88.0 WHERE id=2; \
             INSERT INTO stock_position VALUES \
               (5,'TEST_CODE_CLUSTER_NEW_SOURCE','TEST_CODE新增','2026-07-23',50.0,100,'open', \
                NULL,NULL,77.0,'2026-07-23 09:05:06','2026-07-23 14:05:06',NULL,NULL); \
             UPDATE stock_concepts SET concepts='[\"TEST_CODE_CHANGED_CACHE\"]' \
               WHERE code IN ('TEST_CODE_CLUSTER_STOCK_A','TEST_CODE_CLUSTER_POS_ALIAS'); \
             INSERT OR REPLACE INTO stock_concepts(code,concepts,updated_at) VALUES \
               ('TEST_CODE_CLUSTER_NEW_SOURCE','[\"TEST_CODE_NEW_CACHE\"]','2026-07-23 14:05:00');",
        );
        let changed_positions = all_rows(
            fixture.connection(),
            "SELECT * FROM stock_position ORDER BY id",
        );
        let changed_cache = all_rows(
            fixture.connection(),
            "SELECT code,concepts,CAST(updated_at AS TEXT) FROM stock_concepts ORDER BY code",
        );
        let (run_columns, run_before_reopen) =
            single_named_row(fixture.connection(), "chain_post_close_runs");
        let saved_position_rows = position_rows;
        let saved_concept_rows = concept_rows;
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
                lease_request(OWNER_C, 361_000_000, 660_000_000, Some(v8_head)),
            )
            .unwrap();
        let recovered_clock = PositionsClock {
            now: UtcMicros::try_new(micros("2026-07-21T15:37:02+08:00")).unwrap(),
            observation: None,
            cache_calls: Cell::new(0),
        };
        let mut io = local
            .positions_preparation_io_v8(
                lease,
                &queries,
                &recovered_clock,
                FixedClusterConfiguration::resolve(Some("2")),
            )
            .unwrap();
        let reopened_error = prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks,
            None,
            &mut io,
        )
        .await
        .expect_err("TEST_CODE reopened v8 stops at DragonTiger");
        assert_position_failure(&reopened_error);
        drop(io);
        assert_eq!(recovered_clock.cache_calls.get(), 0);
        // Reopen takeover advances once; material recovery performs no further CAS.
        assert_eq!(local.inspect_run(&intent).unwrap().head_version(), v8_head + 1);
        drop(local);

        assert_eq!(
            all_rows(
                fixture.connection(),
                "SELECT intent_id,run_id,run_context_sha256,input_sha256,material_codec_version, \
                        material_bytes,material_length,material_sha256,lease_owner,lease_generation, \
                        prior_head_version,run_version,observed_at,committed_at,row_count \
                 FROM chain_post_close_position_materials"
            ),
            saved_position_rows
        );
        assert_eq!(
            all_rows(
                fixture.connection(),
                "SELECT intent_id,run_id,run_context_sha256,input_sha256,material_codec_version, \
                        material_bytes,material_length,material_sha256,lease_owner,lease_generation, \
                        prior_head_version,run_version,observed_at,committed_at,batch_kind, \
                        positions_run_version,positions_sha256,requested_count,cache_row_count, \
                        cache_max_age_days,cache_cutoff,local_offset_seconds,cutoff_local_offset_seconds \
                 FROM chain_post_close_position_concept_materials"
            ),
            saved_concept_rows
        );
        assert_eq!(
            all_rows(fixture.connection(), "SELECT * FROM stock_position ORDER BY id"),
            changed_positions
        );
        assert_eq!(
            all_rows(
                fixture.connection(),
                "SELECT code,concepts,CAST(updated_at AS TEXT) FROM stock_concepts ORDER BY code"
            ),
            changed_cache
        );
        assert_eq!(old_fact_rows(fixture.connection(), &old_names), old_facts);
        assert_eq!(
            (
                all_rows(
                    fixture.connection(),
                    "SELECT * FROM data_acquisition_audit ORDER BY id"
                ),
                all_rows(
                    fixture.connection(),
                    "SELECT * FROM data_acquisition_audit_chain ORDER BY acquisition_audit_id"
                ),
            ),
            old_audits
        );
        assert_eq!(fixture.foundation_catalog(), foundation_catalog);
        assert_eq!(
            (
                fixture
                    .connection()
                    .query_row("PRAGMA main.user_version", [], |row| row.get::<_, i64>(0))
                    .unwrap(),
                fixture
                    .connection()
                    .query_row("PRAGMA main.application_id", [], |row| row.get::<_, i64>(0))
                    .unwrap(),
            ),
            database_identity
        );
        assert_eq!(
            all_rows(
                fixture.connection(),
                "SELECT * FROM chain_post_close_schema WHERE schema_version<=7 ORDER BY schema_version"
            ),
            old_schema
        );
        assert_eq!(
            all_rows(
                fixture.connection(),
                "SELECT * FROM chain_post_close_layouts WHERE layout_version<=7 ORDER BY layout_version"
            ),
            old_layouts
        );
        assert_eq!(
            all_rows(
                fixture.connection(),
                "SELECT * FROM chain_post_close_layout_objects WHERE layout_version<=7 \
                 ORDER BY layout_version,name"
            ),
            old_registry
        );
        assert_eq!(
            all_rows(
                fixture.connection(),
                "SELECT name,type,tbl_name,CAST(sql AS BLOB) FROM sqlite_schema \
                 WHERE name LIKE 'chain_post_close_%' ORDER BY name"
            ),
            migrated_catalog
        );
        assert_eq!(
            all_rows(
                fixture.connection(),
                "SELECT * FROM chain_post_close_layouts ORDER BY layout_version"
            ),
            migrated_layouts
        );
        let (after_columns, run_after_reopen) =
            single_named_row(fixture.connection(), "chain_post_close_runs");
        assert_only_run_cas_changed(
            &run_columns,
            &run_before_reopen,
            &after_columns,
            &run_after_reopen,
        );
        assert_eq!(
            value_at(&after_columns, &run_after_reopen, "lease_owner"),
            &Value::Text(OWNER_C.to_owned())
        );
        assert_eq!(
            value_at(&after_columns, &run_after_reopen, "lease_generation"),
            &Value::Integer(3)
        );
        assert_eq!(
            value_at(&after_columns, &run_after_reopen, "head_version"),
            &Value::Integer(i64::try_from(v8_head + 1).unwrap())
        );
        assert_eq!(
            value_at(&after_columns, &run_after_reopen, "lease_until"),
            &Value::Integer(at(660_000_000))
        );
        assert_eq!(
            value_at(&after_columns, &run_after_reopen, "updated_at"),
            &Value::Integer(at(361_000_000))
        );
        assert_eq!(server.snapshot(), original_requests);
        assert!(server.membership_snapshot().is_empty());
        drop(queries);
        drop(source);
        assert_eq!(server.finish().await, original_requests);
    })
    .await
    .expect("TEST_CODE positions v8 scenario deadline");
}

#[path = "chain_post_close_positions_migration_tests.rs"]
mod migration_tests;

#[path = "chain_post_close_position_concept_rpc_tests.rs"]
mod position_concept_rpc_tests;

#[path = "chain_post_close_catalog_proof_tests.rs"]
mod catalog_proof_tests;
