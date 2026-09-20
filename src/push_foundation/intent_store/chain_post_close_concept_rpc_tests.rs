use super::*;
use crate::grpc_client::client::board_loopback_fixture::{
    spawn_membership_commit_failure_loopback, BoardMembershipLoopbackRequest,
};
use crate::grpc_client::pb::magic::market::v1::QueryRequest;
use prost::Message as _;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
struct ConceptRpcFacts {
    configurations: Vec<(Vec<u8>, i64)>,
    occurrences: Vec<(i64, String, String, i64, Vec<u8>, i64)>,
    attempt_begins: Vec<(i64, i64, i64)>,
    results: i64,
    status_materials: i64,
    error_materials: i64,
    finals: i64,
    outer_begins: i64,
    outer_results: i64,
    cache_writes: i64,
    cached_concepts: i64,
    cluster_materials: i64,
    chain_daily_applications: i64,
    audits: i64,
    audit_chain: i64,
}

fn count(connection: &Connection, table: &str) -> i64 {
    connection
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}

fn concept_rpc_facts(connection: &Connection) -> ConceptRpcFacts {
    let mut statement = connection
        .prepare(
            "SELECT CAST(configuration_bytes AS BLOB),run_version \
             FROM chain_post_close_cluster_configurations ORDER BY intent_id",
        )
        .unwrap();
    let configurations = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    let mut statement = connection
        .prepare(
            "SELECT outer_ordinal,code,request_id,request_codec_version, \
                    CAST(request_bytes AS BLOB),run_version \
             FROM chain_post_close_concept_rpc_occurrences ORDER BY outer_ordinal",
        )
        .unwrap();
    let occurrences = statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    let mut statement = connection
        .prepare(
            "SELECT outer_ordinal,attempt_ordinal,run_version \
             FROM chain_post_close_concept_rpc_attempt_begins \
             ORDER BY outer_ordinal,attempt_ordinal",
        )
        .unwrap();
    let attempt_begins = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    ConceptRpcFacts {
        configurations,
        occurrences,
        attempt_begins,
        results: count(connection, "chain_post_close_concept_rpc_attempt_results"),
        status_materials: count(connection, "chain_post_close_concept_rpc_status_materials"),
        error_materials: count(connection, "chain_post_close_concept_rpc_error_materials"),
        finals: count(connection, "chain_post_close_concept_rpc_finals"),
        outer_begins: count(connection, "chain_post_close_stage_begins"),
        outer_results: count(connection, "chain_post_close_stage_results"),
        cache_writes: count(connection, "chain_post_close_concept_cache_writes"),
        cached_concepts: connection
            .query_row(
                "SELECT count(*) FROM stock_concepts WHERE code='TEST_CODE_600001'",
                [],
                |row| row.get(0),
            )
            .unwrap(),
        cluster_materials: count(connection, "chain_post_close_cluster_materials"),
        chain_daily_applications: count(connection, "chain_post_close_chain_daily_applications"),
        audits: count(connection, "data_acquisition_audit"),
        audit_chain: count(connection, "data_acquisition_audit_chain"),
    }
}

fn install_v7(fixture: &mut V2BusinessFixture) {
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
    install_br159_in_owned_database(fixture);
    fixture
        .chain_post_close()
        .migrate_schema_v4_to_v5()
        .unwrap();
    fixture
        .chain_post_close()
        .migrate_schema_v5_to_v6()
        .unwrap();
    assert_eq!(
        fixture
            .chain_post_close()
            .migrate_schema_v6_to_v7()
            .unwrap()
            .schema_version(),
        7
    );
}

fn stock() -> TopStock {
    TopStock {
        code: "TEST_CODE_600001".to_owned(),
        name: "TEST_CODE_600001_NAME".to_owned(),
        change_pct: 10.0,
        price: 12.34,
        ..TopStock::default()
    }
}

fn assert_request_material(facts: &ConceptRpcFacts, observed: &BoardMembershipLoopbackRequest) {
    assert_eq!(facts.occurrences.len(), 1);
    let (ordinal, code, request_id, codec, bytes, _) = &facts.occurrences[0];
    assert_eq!(code, "TEST_CODE_600001");
    assert_eq!(*ordinal, 0);
    assert_eq!(request_id, &observed.request_id);
    assert_eq!(*codec, 1);
    let envelope: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(envelope["schema_version"], 1);
    assert_eq!(envelope["request_id"], observed.request_id);
    assert_eq!(envelope["profile"], "LocalBridgeV1");
    assert!(envelope["acquisition_authority"].is_null());
    assert_eq!(envelope["retry_max_attempts"], 4);
    assert_eq!(envelope["retry_base_delay_ms"], 1_000);
    assert_eq!(envelope["retry_max_delay_ms"], 60_000);
    assert_eq!(envelope["retry_jitter_ms"], 200);
    let token = b"TEST_CODE_BOARD_LOOPBACK_TOKEN";
    assert!(!bytes.windows(token.len()).any(|window| window == token));
    let wire: Vec<u8> = serde_json::from_value(envelope["request_wire"].clone()).unwrap();
    assert!(!wire.windows(token.len()).any(|window| window == token));
    let request = QueryRequest::decode(wire.as_slice()).unwrap();
    assert!(!request
        .encode_to_vec()
        .windows(token.len())
        .any(|window| window == token));
    let context = request.context.unwrap();
    let payload = request.payload.unwrap();
    assert_eq!(context.request_id, observed.request_id);
    assert_eq!(context.protocol_version, 1);
    assert_eq!(payload.schema, "board.constituents");
    assert_eq!(payload.schema_version, 1);
    assert_eq!(payload.content_type, "application/json; charset=utf-8");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&payload.data).unwrap(),
        serde_json::json!({ "codes": ["TEST_CODE_600001"] })
    );
    assert!(request.preferred_provider.is_empty());
    assert!(!request.allow_unadmitted);
}

fn assert_concept_stop(error: &anyhow::Error, intent_id: &IntentId, reopened: bool) {
    if reopened {
        assert!(matches!(
            error.downcast_ref::<PreparationStop>(),
            Some(PreparationStop::IncompleteOnReopen { intent_id: actual })
                if actual.as_str() == intent_id.as_str()
        ));
        assert!(matches!(
            error.downcast_ref::<ChainPostCloseError>(),
            Some(ChainPostCloseError::IncompleteEffect { intent_id: actual })
                if actual.as_str() == intent_id.as_str()
        ));
    } else {
        assert!(matches!(
            error.downcast_ref::<PreparationStop>(),
            Some(PreparationStop::ResultUnconfirmed { intent_id: actual })
                if actual.as_str() == intent_id.as_str()
        ));
    }
    let failure = error
        .downcast_ref::<PreparationFailure>()
        .expect("TEST_CODE concept RPC stop retains stage observations");
    assert_eq!(failure.stage(), PreparationStage::Concepts);
    assert!(failure.completed_stages().is_empty());
}

#[tokio::test]
async fn membership_result_commit_failure_never_replays_after_reentry_or_reopen() {
    let mut fixture = V2BusinessFixture::new();
    install_v7(&mut fixture);
    let journal: String = fixture
        .connection()
        .query_row("PRAGMA journal_mode=DELETE", [], |row| row.get(0))
        .unwrap();
    assert_eq!(journal.to_ascii_lowercase(), "delete");
    fixture.connection().busy_timeout(Duration::ZERO).unwrap();
    let database = fixture.database();
    let reader = Connection::open_with_flags(
        &database,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .unwrap();
    reader.busy_timeout(Duration::ZERO).unwrap();

    let (client, server) = spawn_membership_commit_failure_loopback().await;
    let source = GrpcSource::from_board_loopback_test_client(client);
    let queries = source.connected_board_queries().await.unwrap();
    let stocks = vec![stock()];
    let config = local_config(BUILD_A);
    let context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_RUN_CONCEPT_RPC_COMMIT"),
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
            lease_request("TEST_CODE_CONCEPT_RPC_OWNER_A", 1_000, 5_000, None),
        )
        .unwrap();
    let intent_id = lease.intent_id().clone();
    let clock = ControlledClock::new(at(1_100));
    let mut io = local
        .concept_rpc_preparation_io_v7(
            lease,
            &queries,
            &clock,
            FixedClusterConfiguration::resolve(Some("2")),
        )
        .unwrap();
    let mut prepare = Box::pin(prepare_chain_analysis_with_io(
        NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
        stocks.clone(),
        None,
        &mut io,
    ));
    let observed = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::select! {
            request = server.wait_for_membership_request() => request,
            _ = &mut prepare => panic!("TEST_CODE membership RPC returned before release"),
        }
    })
    .await
    .expect("TEST_CODE membership request observation timeout");
    assert_eq!(observed.codes, ["TEST_CODE_600001"]);
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

    reader.execute_batch("BEGIN DEFERRED;").unwrap();
    let locked_facts = concept_rpc_facts(&reader);
    assert_eq!(locked_facts.configurations.len(), 1);
    assert_eq!(
        locked_facts.configurations[0].0,
        br#"{"schema_version":1,"min_cluster_size":"2"}"#
    );
    assert_request_material(&locked_facts, &observed);
    assert!(locked_facts.configurations[0].1 < locked_facts.occurrences[0].5);
    assert_eq!(locked_facts.attempt_begins.len(), 1);
    assert_eq!(locked_facts.attempt_begins[0].0, 0);
    assert_eq!(locked_facts.attempt_begins[0].1, 1);
    assert_eq!(locked_facts.results, 0);
    assert_eq!(locked_facts.status_materials, 0);
    assert_eq!(locked_facts.error_materials, 0);
    assert_eq!(locked_facts.finals, 0);
    assert_eq!(locked_facts.outer_begins, 0);
    assert_eq!(locked_facts.outer_results, 0);
    assert_eq!(locked_facts.cache_writes, 0);
    assert_eq!(locked_facts.cached_concepts, 0);
    assert_eq!(locked_facts.cluster_materials, 0);
    assert_eq!(locked_facts.chain_daily_applications, 0);
    assert_eq!(locked_facts.audits, 0);
    assert_eq!(locked_facts.audit_chain, 0);
    let locked_head: i64 = reader
        .query_row(
            "SELECT head_version FROM chain_post_close_runs",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(locked_head, locked_facts.attempt_begins[0].2);

    server.release_membership_unavailable();
    let error = tokio::time::timeout(Duration::from_secs(5), &mut prepare)
        .await
        .expect("TEST_CODE membership result COMMIT timeout")
        .expect_err("TEST_CODE membership result COMMIT must stop");
    drop(prepare);
    assert_concept_stop(&error, &intent_id, false);
    assert!(matches!(
        error.downcast_ref::<ChainPostCloseError>(),
        Some(ChainPostCloseError::StorageFailed {
            operation: "commit"
        })
    ));
    assert_eq!(concept_rpc_facts(&reader), locked_facts);
    assert_eq!(server.membership_snapshot().len(), 1);
    assert!(server.snapshot().requests.is_empty());

    let reentry = tokio::time::timeout(
        Duration::from_secs(1),
        prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks.clone(),
            None,
            &mut io,
        ),
    )
    .await
    .expect("TEST_CODE same adapter reentry timeout")
    .expect_err("TEST_CODE same adapter must remain stopped");
    assert!(matches!(
        reentry.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::ResultUnconfirmed { intent_id: actual })
            if actual.as_str() == intent_id.as_str()
    ));
    assert!(reentry.downcast_ref::<PreparationFailure>().is_none());
    assert_eq!(server.membership_snapshot().len(), 1);
    drop(io);

    reader.execute_batch("ROLLBACK;").unwrap();
    assert_eq!(concept_rpc_facts(&reader), locked_facts);
    let fresh_head: i64 = reader
        .query_row(
            "SELECT head_version FROM chain_post_close_runs",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(fresh_head, locked_head);
    reader.close().unwrap();
    drop(local);
    assert_eq!(concept_rpc_facts(fixture.connection()), locked_facts);
    fixture.reopen();
    assert_eq!(concept_rpc_facts(fixture.connection()), locked_facts);

    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let lease = local
        .resume_run(
            &intent_id,
            lease_request(
                "TEST_CODE_CONCEPT_RPC_OWNER_B",
                5_001,
                2_005_001,
                Some(u64::try_from(locked_head).unwrap()),
            ),
        )
        .unwrap();
    let recovery_clock = ControlledClock::new(at(5_100));
    let mut io = local
        .concept_rpc_preparation_io_v7(
            lease,
            &queries,
            &recovery_clock,
            FixedClusterConfiguration::resolve(Some("2")),
        )
        .unwrap();
    let reopened = tokio::time::timeout(
        Duration::from_secs(5),
        prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks,
            None,
            &mut io,
        ),
    )
    .await
    .expect("TEST_CODE reopened membership stop timeout")
    .expect_err("TEST_CODE reopened incomplete membership must stop");
    assert_concept_stop(&reopened, &intent_id, true);
    drop(io);
    drop(local);
    assert_eq!(concept_rpc_facts(fixture.connection()), locked_facts);
    assert_eq!(server.membership_snapshot().len(), 1);
    let observation = server.finish().await;
    assert!(observation.requests.is_empty());
    assert_eq!(observation.non_board_requests, 0);
}

const LEGACY_MEMBERSHIP_FAILURE: &str = "TEST_CODE_LEGACY_MEMBERSHIP_FAILURE: 原始原因  ";

struct LegacyMembershipErrorProvider {
    calls: Cell<usize>,
}

#[async_trait::async_trait(?Send)]
impl ConceptProviderRawIo for LegacyMembershipErrorProvider {
    async fn call_raw(&self, code: &str) -> std::result::Result<String, String> {
        assert_eq!(code, "TEST_CODE_600001");
        assert_eq!(self.calls.replace(self.calls.get() + 1), 0);
        Err(LEGACY_MEMBERSHIP_FAILURE.to_owned())
    }
}

type LegacySqlRows = Vec<Vec<rusqlite::types::Value>>;

fn legacy_table_names(connection: &Connection) -> Vec<String> {
    all_rows(
        connection,
        "SELECT name FROM main.sqlite_schema WHERE type='table' ORDER BY name",
    )
    .into_iter()
    .map(|row| match &row[0] {
        rusqlite::types::Value::Text(name) => name.clone(),
        _ => panic!("TEST_CODE owned table name"),
    })
    .collect()
}

fn legacy_table_values(
    connection: &Connection,
    names: &[String],
    layout_ceiling: i64,
) -> BTreeMap<String, LegacySqlRows> {
    names
        .iter()
        .map(|name| {
            let quoted = name.replace('"', "\"\"");
            let filter = if matches!(
                name.as_str(),
                "chain_post_close_layouts" | "chain_post_close_layout_objects"
            ) {
                format!(" WHERE layout_version<={layout_ceiling}")
            } else {
                String::new()
            };
            let select = format!("SELECT * FROM main.\"{quoted}\"{filter}");
            let width = connection.prepare(&select).unwrap().column_count();
            let order = (1..=width)
                .map(|column| column.to_string())
                .collect::<Vec<_>>()
                .join(",");
            (
                name.clone(),
                all_rows(connection, &format!("{select} ORDER BY {order}")),
            )
        })
        .collect()
}

fn assert_legacy_business_failure(error: &anyhow::Error) {
    assert!(error.downcast_ref::<PreparationStop>().is_none());
    // Ordinary observe_stage retains the formatted original chain only in reason();
    // its safe Display is not the original provider error, and has no raw source chain.
    let failure = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<PreparationFailure>())
        .expect("TEST_CODE ordinary Concepts failure remains observable");
    assert_eq!(failure.stage(), PreparationStage::Concepts);
    assert!(failure.completed_stages().is_empty());
    assert_eq!(
        failure.reason().as_bytes(),
        LEGACY_MEMBERSHIP_FAILURE.as_bytes()
    );
    assert!(!error
        .to_string()
        .contains("TEST_CODE_LEGACY_MEMBERSHIP_FAILURE"));
}

#[tokio::test]
async fn legacy_business_error_reopens_in_v7_without_new_membership_attempt() {
    use rusqlite::types::Value;

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
    install_br159_in_owned_database(&mut fixture);
    fixture
        .chain_post_close()
        .migrate_schema_v4_to_v5()
        .unwrap();
    assert_eq!(
        fixture
            .chain_post_close()
            .migrate_schema_v5_to_v6()
            .unwrap()
            .schema_version(),
        6
    );
    assert_eq!(
        fixture
            .chain_post_close()
            .verify_schema()
            .unwrap()
            .schema_version(),
        6
    );
    assert_eq!(
        fixture
            .connection()
            .query_row(
                "SELECT count(*) FROM stock_concepts WHERE code='TEST_CODE_600001'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );

    let (client, server) = spawn_board_loopback().await;
    let source = GrpcSource::from_board_loopback_test_client(client);
    let queries = tokio::time::timeout(Duration::from_secs(5), source.connected_board_queries())
        .await
        .expect("TEST_CODE connect owned legacy fixture timeout")
        .unwrap();
    let no_requests = server.snapshot();
    assert!(no_requests.requests.is_empty());
    assert_eq!(no_requests.non_board_requests, 0);
    assert!(server.membership_snapshot().is_empty());

    let stocks = vec![stock()];
    let config = local_config(BUILD_A);
    let context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_LEGACY_MEMBERSHIP_ERROR"),
    )
    .unwrap();
    let provider = LegacyMembershipErrorProvider {
        calls: Cell::new(0),
    };
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
            lease_request("TEST_CODE_LEGACY_MEMBERSHIP_OWNER_A", 1_000, 5_000, None),
        )
        .unwrap();
    let intent_id = lease.intent_id().clone();
    let clock = ControlledClock::new(at(1_100));
    let mut io = local
        .board_preparation_io_v6(
            lease,
            &provider,
            &clock,
            FixedClusterConfiguration::resolve(Some("2")),
            &source,
        )
        .unwrap();
    let first = tokio::time::timeout(
        Duration::from_secs(5),
        prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks.clone(),
            None,
            &mut io,
        ),
    )
    .await
    .expect("TEST_CODE real legacy prepare timeout")
    .expect_err("TEST_CODE one real ordinary provider error");
    assert_legacy_business_failure(&first);
    assert_eq!(provider.calls.get(), 1);
    drop(io);
    let recovery = local.inspect_run(&intent_id).unwrap();
    assert_eq!(recovery.begins().len(), 1);
    assert_eq!(recovery.results().len(), 1);
    assert_eq!(recovery.begins()[0].ordinal, 0);
    assert_eq!(recovery.begins()[0].code, "TEST_CODE_600001");
    assert_eq!(recovery.results()[0].outcome, "BusinessError");
    assert_eq!(
        recovery.results()[0].bytes,
        LEGACY_MEMBERSHIP_FAILURE.as_bytes()
    );
    let head = recovery.head_version();
    assert_eq!(head, recovery.results()[0].run_version);
    assert_eq!(recovery.lease_generation(), 1);
    drop(recovery);
    drop(local);
    assert_eq!(
        fixture.stored_result(&intent_id),
        Some((
            "BusinessError".to_owned(),
            LEGACY_MEMBERSHIP_FAILURE.as_bytes().to_vec()
        ))
    );
    assert_eq!(server.snapshot(), no_requests);
    assert!(server.membership_snapshot().is_empty());

    let old_names = legacy_table_names(fixture.connection());
    let old_values = legacy_table_values(fixture.connection(), &old_names, 6);
    let old_catalog = all_rows(
        fixture.connection(),
        "SELECT name,type,tbl_name,CAST(sql AS BLOB) FROM main.sqlite_schema ORDER BY name",
    );
    let original_board_facts = board_durable_facts(fixture.connection());
    let mut qualification = all_rows(
        fixture.connection(),
        "SELECT begun.intent_id,begun.effect_kind,begun.effect_ordinal,begun.effect_key,
                begun.run_version,begun.request_sha256,begun.lease_owner,begun.lease_generation,
                begun.begun_at,result.outcome,result.run_version,result.result_sha256,
                result.lease_owner,result.lease_generation,result.committed_at
         FROM chain_post_close_stage_begins AS begun
         JOIN chain_post_close_stage_results AS result
           ON result.intent_id=begun.intent_id AND result.effect_kind=begun.effect_kind
          AND result.effect_ordinal=begun.effect_ordinal
         ORDER BY begun.intent_id,begun.effect_ordinal",
    );
    assert_eq!(qualification.len(), 1);
    assert_eq!(
        &qualification[0][..4],
        &[
            Value::Text(intent_id.as_str().to_owned()),
            Value::Text("ConceptProvider".to_owned()),
            Value::Integer(0),
            Value::Text("TEST_CODE_600001".to_owned()),
        ]
    );
    assert_eq!(
        qualification[0][6],
        Value::Text("TEST_CODE_LEGACY_MEMBERSHIP_OWNER_A".to_owned())
    );
    assert_eq!(qualification[0][7], Value::Integer(1));
    assert_eq!(qualification[0][8], Value::Integer(at(1_100)));
    assert_eq!(qualification[0][9], Value::Text("BusinessError".to_owned()));
    assert_eq!(
        qualification[0][10],
        Value::Integer(i64::try_from(head).unwrap())
    );
    assert_eq!(
        qualification[0][11],
        Value::Text(
            raw_digest(LEGACY_MEMBERSHIP_FAILURE.as_bytes())
                .as_str()
                .to_owned()
        )
    );
    assert_eq!(
        qualification[0][12],
        Value::Text("TEST_CODE_LEGACY_MEMBERSHIP_OWNER_A".to_owned())
    );
    assert_eq!(qualification[0][13], Value::Integer(1));
    assert_eq!(qualification[0][14], Value::Integer(at(1_100)));
    // Complete 23-column qualification, from the actual old parents, not a v7 encoder.
    qualification[0].insert(4, Value::Text("LegacyBusinessError".to_owned()));
    qualification[0].extend(std::iter::repeat(Value::Null).take(5));
    qualification[0].extend([Value::Integer(6), Value::Integer(7)]);

    assert_eq!(
        fixture
            .chain_post_close()
            .migrate_schema_v6_to_v7()
            .unwrap()
            .schema_version(),
        7
    );
    assert_eq!(
        fixture
            .chain_post_close()
            .verify_schema()
            .unwrap()
            .schema_version(),
        7
    );
    assert_eq!(
        legacy_table_values(fixture.connection(), &old_names, 6),
        old_values
    );
    let migrated_catalog = all_rows(
        fixture.connection(),
        "SELECT name,type,tbl_name,CAST(sql AS BLOB) FROM main.sqlite_schema ORDER BY name",
    );
    assert!(old_catalog.iter().all(|row| migrated_catalog.contains(row)));
    let new_definitions = migrated_catalog
        .iter()
        .filter(|row| !old_catalog.iter().any(|old| old[0] == row[0]) && row[3] != Value::Null)
        .map(|row| vec![row[0].clone(), row[1].clone(), row[3].clone()])
        .collect::<Vec<_>>();
    assert_eq!(new_definitions.len(), 28);
    assert_eq!(
        fixture
            .connection()
            .query_row(
                "SELECT count(*) FROM chain_post_close_layout_objects WHERE layout_version=7",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        99
    );
    // Each registry version stores its complete layout, not only that migration's delta.
    assert_eq!(
        new_definitions,
        all_rows(
            fixture.connection(),
            "SELECT current.name,current.object_type,CAST(current.definition AS BLOB)
         FROM chain_post_close_layout_objects AS current
         WHERE current.layout_version=7
           AND NOT EXISTS (
               SELECT 1 FROM chain_post_close_layout_objects AS previous
               WHERE previous.layout_version=6 AND previous.name=current.name
           )
         ORDER BY current.name"
        )
    );
    assert_eq!(
        all_rows(
            fixture.connection(),
            "SELECT * FROM chain_post_close_concept_rpc_legacy_outer_qualifications
         ORDER BY intent_id,outer_ordinal"
        ),
        qualification
    );
    let rpc_facts = concept_rpc_facts(fixture.connection());
    assert_eq!(rpc_facts.configurations.len(), 1);
    assert!(rpc_facts.occurrences.is_empty());
    assert!(rpc_facts.attempt_begins.is_empty());
    assert_eq!(
        (
            rpc_facts.results,
            rpc_facts.status_materials,
            rpc_facts.error_materials,
            rpc_facts.finals
        ),
        (0, 0, 0, 0)
    );
    assert_eq!((rpc_facts.outer_begins, rpc_facts.outer_results), (1, 1));
    assert_eq!(
        (
            rpc_facts.cache_writes,
            rpc_facts.cached_concepts,
            rpc_facts.cluster_materials,
            rpc_facts.chain_daily_applications,
            rpc_facts.audits,
            rpc_facts.audit_chain
        ),
        (0, 0, 0, 0, 0, 0)
    );
    let all_names = legacy_table_names(fixture.connection());
    let mut expected_after_resume = legacy_table_values(fixture.connection(), &all_names, 7);
    fixture.reopen();
    assert_eq!(
        fixture
            .chain_post_close()
            .verify_schema()
            .unwrap()
            .schema_version(),
        7
    );
    assert_eq!(
        legacy_table_values(fixture.connection(), &all_names, 7),
        expected_after_resume
    );

    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let lease = local
        .resume_run(
            &intent_id,
            lease_request(
                "TEST_CODE_LEGACY_MEMBERSHIP_OWNER_B",
                5_001,
                9_000,
                Some(head),
            ),
        )
        .unwrap();
    assert_eq!(lease.head_version(), head + 1);
    assert_eq!(lease.generation(), 2);
    let clock = ControlledClock::new(at(5_100));
    let mut io = local
        .concept_rpc_preparation_io_v7(
            lease,
            &queries,
            &clock,
            FixedClusterConfiguration::resolve(Some("2")),
        )
        .unwrap();
    let reopened = tokio::time::timeout(
        Duration::from_secs(5),
        prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks,
            None,
            &mut io,
        ),
    )
    .await
    .expect("TEST_CODE legacy v7 recovery timeout")
    .expect_err("TEST_CODE original business error must remain ordinary");
    drop(io);
    let recovery = local.inspect_run(&intent_id).unwrap();
    assert_eq!(recovery.head_version(), head + 1);
    assert_eq!(recovery.lease_generation(), 2);
    assert_eq!(recovery.begins().len(), 1);
    assert_eq!(recovery.results().len(), 1);
    assert_eq!(recovery.results()[0].outcome, "BusinessError");
    assert_eq!(
        recovery.results()[0].bytes,
        LEGACY_MEMBERSHIP_FAILURE.as_bytes()
    );
    drop(recovery);
    drop(local);

    // Only the normal, successful lease takeover may change these five run fields.
    let columns = all_rows(
        fixture.connection(),
        "PRAGMA main.table_info('chain_post_close_runs')",
    );
    let run = expected_after_resume
        .get_mut("chain_post_close_runs")
        .unwrap();
    assert_eq!(run.len(), 1);
    for (name, value) in [
        (
            "lease_owner",
            Value::Text("TEST_CODE_LEGACY_MEMBERSHIP_OWNER_B".to_owned()),
        ),
        ("lease_generation", Value::Integer(2)),
        (
            "head_version",
            Value::Integer(i64::try_from(head + 1).unwrap()),
        ),
        ("lease_until", Value::Integer(at(9_000))),
        ("updated_at", Value::Integer(at(5_001))),
    ] {
        let ordinal = columns
            .iter()
            .position(|column| column[1] == Value::Text(name.to_owned()))
            .unwrap();
        run[0][ordinal] = value;
    }
    assert_eq!(
        legacy_table_values(fixture.connection(), &all_names, 7),
        expected_after_resume
    );
    assert_eq!(
        all_rows(
            fixture.connection(),
            "SELECT name,type,tbl_name,CAST(sql AS BLOB) FROM main.sqlite_schema ORDER BY name"
        ),
        migrated_catalog
    );
    assert_eq!(
        board_durable_facts(fixture.connection()),
        original_board_facts
    );
    assert_eq!(concept_rpc_facts(fixture.connection()), rpc_facts);
    assert_eq!(
        fixture.stored_result(&intent_id),
        Some((
            "BusinessError".to_owned(),
            LEGACY_MEMBERSHIP_FAILURE.as_bytes().to_vec()
        ))
    );
    assert_eq!(provider.calls.get(), 1);
    assert!(server.membership_snapshot().is_empty());
    assert_eq!(server.snapshot(), no_requests);
    drop(queries);
    drop(source);
    let observation = server.finish().await;
    assert_eq!(observation, no_requests);
    // Finish the owned server before the expected regression assertion can fail.
    assert_legacy_business_failure(&reopened);
}

#[tokio::test]
async fn membership_success_reopens_without_rpc_cache_cluster_or_audit_rewrite() {
    use super::super::super::{ConceptEffectRecoveryState, LocalChainPostClose};
    use crate::database::data_acquisition_audit::DataAcquisitionAuditReceipt;
    use crate::grpc_client::client::board_loopback_fixture::spawn_membership_success_loopback;
    use crate::grpc_client::pb::magic::market::v1::{AdmissionState, Operation, QueryResponse};
    use rusqlite::types::Value;
    use sha2::Digest as _;

    // Independent external bytes and complete original Tool JSON; no production renderer oracle.
    const PAYLOAD: &[u8] = br#"[{"instrument_code":"TEST_CODE_600001","board_code":"TEST_CODE_BOARD_MAIN","board_name":"TEST_CODE_CLUSTER_A_MAIN","kind":"Industry"},{"instrument_code":"TEST_CODE_600001","board_code":"TEST_CODE_BOARD_ALIAS","board_name":"TEST_CODE_CLUSTER_B_ALIAS","kind":"Concept"}]"#;
    const TOOL_JSON: &str = r#"{"all_boards":["TEST_CODE_CLUSTER_A_MAIN","TEST_CODE_CLUSTER_B_ALIAS"],"board_count":2,"evidence":{"batch_id":"TEST_CODE_MEMBERSHIP_BATCH","observed_at":"2026-07-21T15:31:00+08:00","provider":"Tdx","source":"TEST_CODE_LOOPBACK_MEMBERSHIP_SOURCE","source_at":"2026-07-21T15:30:00+08:00"},"fetched":true,"memberships":[{"board_code":"TEST_CODE_BOARD_MAIN","board_name":"TEST_CODE_CLUSTER_A_MAIN","category":"Industry"},{"board_code":"TEST_CODE_BOARD_ALIAS","board_name":"TEST_CODE_CLUSTER_B_ALIAS","category":"Concept"}],"note":"统一 Magic TDX 板块归属；行业/概念使用源类别，不再按列表位置猜测。","primary_boards":["TEST_CODE_CLUSTER_A_MAIN"],"secondary_boards":["TEST_CODE_CLUSTER_B_ALIAS"],"secucode":"TEST_CODE_600001"}"#;
    const CACHE: &[u8] = br#"["TEST_CODE_CLUSTER_A_MAIN","TEST_CODE_CLUSTER_B_ALIAS"]"#;
    // SHA-256(BR159_DATA_GATEWAY_REQUEST_V1\0board-memberships\0TEST_CODE_600001).
    const MEMBERSHIP_REQUEST_HASH: &str =
        "b822584c8f4102261d2a3c8b32deee7f956199e51c26424c489ff413339d385e";

    #[track_caller]
    fn assert_success_boundary(
        phase: &str,
        error: &anyhow::Error,
        directory: &BTreeMap<String, String>,
        selected: &BTreeMap<String, String>,
    ) {
        eprintln!("TEST_CODE membership-success phase={phase} boundary");
        assert!(
            matches!(
                error.downcast_ref::<PreparationStop>(),
                Some(PreparationStop::StageNotMigrated {
                    next: UnmigratedStage::Positions
                })
            ),
            "TEST_CODE {phase}: expected Positions stop"
        );
        assert_candidates_completed_before_positions(error, directory, selected);
    }

    #[track_caller]
    fn assert_success_readers(
        phase: &str,
        local: &mut LocalChainPostClose<'_>,
        intent_id: &IntentId,
        stocks: &[TopStock],
        directory: &BTreeMap<String, String>,
        selected: &BTreeMap<String, String>,
    ) -> (u64, Vec<DataAcquisitionAuditReceipt>) {
        eprintln!("TEST_CODE membership-success phase={phase} durable-readers");
        let batch = local
            .inspect_concept_batch(intent_id)
            .expect("TEST_CODE concept reader");
        assert!(
            batch.is_complete(),
            "TEST_CODE {phase}: complete concept batch"
        );
        let expected = [
            (
                "TEST_CODE_CLUSTER_STOCK_A",
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
                "TEST_CODE_600001",
                vec!["TEST_CODE_CLUSTER_A_MAIN", "TEST_CODE_CLUSTER_B_ALIAS"],
            ),
        ]
        .into_iter()
        .map(|(code, concepts)| {
            (
                code.to_owned(),
                concepts.into_iter().map(str::to_owned).collect::<Vec<_>>(),
            )
        })
        .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(
            batch.concepts(),
            &expected,
            "TEST_CODE {phase}: original concept map"
        );
        assert_eq!(batch.applied_codes(), ["TEST_CODE_600001"]);
        assert_eq!(batch.effects().len(), 1);
        assert_eq!(batch.effects()[0].ordinal(), 0);
        assert_eq!(batch.effects()[0].code(), "TEST_CODE_600001");
        assert_eq!(
            batch.effects()[0].state(),
            ConceptEffectRecoveryState::Confirmed
        );
        let cluster = local
            .inspect_cluster_application(intent_id)
            .expect("TEST_CODE cluster reader");
        assert_eq!(cluster.min_cluster_size(), 2);
        assert_eq!(cluster.clusters().len(), 1);
        let main = &cluster.clusters()[0];
        assert_eq!(main.concept, "TEST_CODE_CLUSTER_A_MAIN");
        assert_eq!(main.aliases, ["TEST_CODE_CLUSTER_B_ALIAS"]);
        assert_eq!(
            serde_json::to_value(&main.stocks).unwrap(),
            serde_json::to_value([&stocks[3], &stocks[0], &stocks[1]]).unwrap(),
            "TEST_CODE {phase}: full ordered stock fields"
        );
        assert_eq!(main.continuation_count, 1);
        assert_eq!(main.streak_days, 3);
        assert!(main.candidates.is_empty());
        assert!(main.score.is_none());
        assert!(main.scenario.is_none());
        assert_eq!(
            serde_json::to_value(cluster.isolated()).unwrap(),
            serde_json::to_value([&stocks[2]]).unwrap()
        );
        assert_eq!(
            cluster.lifecycle_days(),
            &BTreeMap::from([("TEST_CODE_CLUSTER_A_MAIN".to_owned(), 3)])
        );
        let board = local
            .inspect_board_directory(intent_id)
            .expect("TEST_CODE board reader");
        assert_eq!(board.board_directory(), directory);
        assert_eq!(board.selected_board_codes(), selected);
        assert_eq!(board.attempts().len(), 3);
        for (index, (kind, ordinal)) in [
            (BoardKind::Industry, 1),
            (BoardKind::Industry, 2),
            (BoardKind::Concept, 1),
        ]
        .into_iter()
        .enumerate()
        {
            assert!(board.attempts()[index].is_confirmed());
            assert_eq!(board.attempts()[index].kind(), kind);
            assert_eq!(board.attempts()[index].attempt_ordinal(), ordinal);
        }
        assert!(board.attempts()[0].payload_bytes().is_none());
        assert_eq!(
            board.attempts()[1].payload_bytes(),
            Some(INDUSTRY_DIRECTORY_BYTES)
        );
        assert_eq!(
            board.attempts()[2].payload_bytes(),
            Some(CONCEPT_DIRECTORY_BYTES)
        );
        assert_eq!(board.directories().len(), 2);
        assert_eq!(board.directories()[0].kind(), BoardKind::Industry);
        assert_eq!(board.directories()[1].kind(), BoardKind::Concept);
        let receipts = board
            .directories()
            .iter()
            .map(|value| value.receipt().clone())
            .collect();
        let run = local
            .inspect_run(intent_id)
            .expect("TEST_CODE original run reader");
        assert_eq!(run.begins().len(), 1);
        assert_eq!(run.results().len(), 1);
        assert_eq!(run.begins()[0].code, "TEST_CODE_600001");
        assert_eq!(run.results()[0].outcome, "Returned");
        assert_eq!(run.results()[0].bytes, TOOL_JSON.as_bytes());
        (run.head_version(), receipts)
    }

    let mut fixture = V2BusinessFixture::new();
    install_v7(&mut fixture);
    let (client, server) = spawn_membership_success_loopback().await;
    let source = GrpcSource::from_board_loopback_test_client(client);
    let queries = tokio::time::timeout(Duration::from_secs(5), source.connected_board_queries())
        .await
        .expect("TEST_CODE success preconnect deadline")
        .unwrap();
    let mut stocks = cluster_tests::cluster_stocks();
    stocks.push(stock());
    let expected_directory = BTreeMap::from([
        (
            "TEST_CODE_CLUSTER_A_MAIN".to_owned(),
            "TEST_CODE_BOARD_MAIN".to_owned(),
        ),
        (
            "TEST_CODE_CONCEPT_ONLY".to_owned(),
            "TEST_CODE_BOARD_CONCEPT_ONLY".to_owned(),
        ),
        (
            "TEST_CODE_INDUSTRY_ONLY".to_owned(),
            "TEST_CODE_BOARD_INDUSTRY_ONLY".to_owned(),
        ),
    ]);
    let expected_selected = BTreeMap::from([(
        "TEST_CODE_CLUSTER_A_MAIN".to_owned(),
        "TEST_CODE_BOARD_MAIN".to_owned(),
    )]);
    let config = local_config(BUILD_A);
    let context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_RUN_MEMBERSHIP_SUCCESS"),
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
            lease_request("TEST_CODE_MEMBERSHIP_SUCCESS_OWNER_A", 1_000, 5_000, None),
        )
        .unwrap();
    let intent_id = lease.intent_id().clone();
    let clock = ControlledClock::new(at(1_100));
    let mut io = local
        .concept_rpc_preparation_io_v7(
            lease,
            &queries,
            &clock,
            FixedClusterConfiguration::resolve(Some("2")),
        )
        .unwrap();
    let first = tokio::time::timeout(
        Duration::from_secs(15),
        prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks.clone(),
            None,
            &mut io,
        ),
    )
    .await
    .expect("TEST_CODE first success prepare deadline")
    .expect_err("TEST_CODE first success stops at Positions");
    assert_success_boundary("first", &first, &expected_directory, &expected_selected);
    drop(io);
    let (head, board_receipts) = assert_success_readers(
        "first",
        &mut local,
        &intent_id,
        &stocks,
        &expected_directory,
        &expected_selected,
    );
    let first_board = local.inspect_board_directory(&intent_id).unwrap();
    let first_observation = server.snapshot();
    for (attempt, observed) in first_board
        .attempts()
        .iter()
        .zip(&first_observation.requests)
    {
        assert_eq!(attempt.request_id(), observed.request_id);
    }
    assert_eq!(
        first_board.attempts()[0].error_detail_bytes(),
        Some(first_observation.retry_error_detail.as_slice())
    );
    drop(first_board);
    drop(local);

    let memberships = server.membership_snapshot();
    assert_eq!(memberships.len(), 1);
    let request = &memberships[0];
    assert_eq!(request.codes, ["TEST_CODE_600001"]);
    assert!(!request.request_id.is_empty());
    assert_eq!(request.protocol_version, 1);
    assert_eq!(request.payload_schema, "board.constituents");
    assert_eq!(request.payload_schema_version, 1);
    assert_eq!(
        request.payload_content_type,
        "application/json; charset=utf-8"
    );
    assert!(request.preferred_provider.is_empty());
    assert!(!request.allow_unadmitted);
    assert!(request.authorized);
    let board_observation = server.snapshot();
    assert_eq!(board_observation.requests.len(), 3);
    assert_eq!(board_observation.non_board_requests, 0);
    assert_eq!(
        board_observation
            .requests
            .iter()
            .map(|value| value.kind.as_str())
            .collect::<Vec<_>>(),
        ["Industry", "Industry", "Concept"]
    );
    assert!(board_observation
        .requests
        .iter()
        .all(|value| value.authorized
            && value.limit == 10_000
            && value.protocol_version == 1
            && value.payload_schema == "board.directory"
            && value.payload_schema_version == 1
            && value.payload_content_type == "application/json; charset=utf-8"
            && !value.allow_unadmitted));
    assert_eq!(
        board_observation.requests[0].request_id,
        board_observation.requests[1].request_id
    );
    assert_ne!(
        board_observation.requests[1].request_id,
        board_observation.requests[2].request_id
    );
    assert!(board_observation
        .requests
        .iter()
        .all(|value| value.request_id != request.request_id));
    let facts = concept_rpc_facts(fixture.connection());
    assert_request_material(&facts, request);
    assert_eq!(facts.configurations.len(), 1);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&facts.configurations[0].0).unwrap(),
        serde_json::json!({"schema_version": 1, "min_cluster_size": "2"})
    );
    assert_eq!(facts.attempt_begins.len(), 1);
    assert_eq!(
        (facts.attempt_begins[0].0, facts.attempt_begins[0].1),
        (0, 1)
    );
    assert_eq!(
        (
            facts.results,
            facts.status_materials,
            facts.error_materials,
            facts.finals
        ),
        (1, 0, 0, 1)
    );
    assert_eq!(
        (
            facts.outer_begins,
            facts.outer_results,
            facts.cache_writes,
            facts.cached_concepts
        ),
        (1, 1, 1, 1)
    );
    assert_eq!(
        (
            facts.cluster_materials,
            facts.chain_daily_applications,
            facts.audits,
            facts.audit_chain
        ),
        (1, 1, 3, 3)
    );
    assert_eq!(
        count(
            fixture.connection(),
            "chain_post_close_concept_rpc_legacy_outer_qualifications"
        ),
        0
    );

    let connection = fixture.connection();
    let (result_bytes, result_version): (Vec<u8>, i64) = connection.query_row(
        "SELECT CAST(result_bytes AS BLOB),run_version FROM chain_post_close_concept_rpc_attempt_results",
        [], |row| Ok((row.get(0)?, row.get(1)?))
    ).unwrap();
    let envelope: serde_json::Value = serde_json::from_slice(&result_bytes).unwrap();
    let wire: Vec<u8> = serde_json::from_value(envelope["response_wire"].clone()).unwrap();
    assert_eq!(
        envelope,
        serde_json::json!({
            "schema_version": 1, "response_wire": wire, "status_code": null, "status_details": null,
            "status_error_detail_trailer": "Absent", "retry_decision": "NoRetry",
            "continuation": "Terminal", "backoff_ms": null
        })
    );
    let response = QueryResponse::decode(wire.as_slice()).unwrap();
    assert_eq!(response.encode_to_vec(), wire);
    assert_eq!(response.request_id, request.request_id);
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
    assert_eq!(
        response.records[0].content_type,
        "application/json; charset=utf-8"
    );
    assert_eq!(response.records[0].data, PAYLOAD);
    let final_bytes: Vec<u8> = connection
        .query_row(
            "SELECT CAST(final_bytes AS BLOB) FROM chain_post_close_concept_rpc_finals",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&final_bytes).unwrap(),
        serde_json::json!({"schema_version": 1, "outcome": "Available", "raw": TOOL_JSON})
    );
    let outer_version: i64 = connection
        .query_row(
            "SELECT run_version FROM chain_post_close_stage_results",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        fixture.stored_result(&intent_id),
        Some(("Returned".to_owned(), TOOL_JSON.as_bytes().to_vec()))
    );

    // Independent SQL links the compatibility projection to every original occurrence/raw/outer fact.
    let bound: i64 = connection.query_row(
        "SELECT count(*) FROM chain_post_close_concept_rpc_finals f \
         JOIN chain_post_close_concept_rpc_occurrences o ON o.intent_id=f.intent_id AND o.outer_ordinal=f.outer_ordinal \
         JOIN chain_post_close_concept_rpc_attempt_begins b ON b.intent_id=f.intent_id AND b.outer_ordinal=f.outer_ordinal AND b.attempt_ordinal=1 \
         JOIN chain_post_close_concept_rpc_attempt_results r ON r.intent_id=b.intent_id AND r.outer_ordinal=b.outer_ordinal AND r.attempt_ordinal=b.attempt_ordinal \
         JOIN chain_post_close_stage_begins ob ON ob.intent_id=f.intent_id AND ob.effect_ordinal=f.outer_ordinal AND ob.effect_kind='ConceptProvider' \
         JOIN chain_post_close_stage_results ore ON ore.intent_id=ob.intent_id AND ore.effect_ordinal=ob.effect_ordinal AND ore.effect_kind=ob.effect_kind \
         JOIN chain_post_close_runs run ON run.intent_id=f.intent_id \
         WHERE f.intent_id=?1 AND f.outer_ordinal=0 AND f.code='TEST_CODE_600001' \
           AND f.provenance='CompatibilityProjection' AND f.final_outcome='Available' AND f.final_codec_version=1 \
           AND f.occurrence_run_version=o.run_version AND f.occurrence_request_sha256=o.request_sha256 \
           AND f.code=o.code AND o.request_id=?2 AND r.begin_run_version=b.run_version \
           AND r.request_sha256=b.request_sha256 AND b.request_sha256=o.request_sha256 \
           AND r.wire_outcome='Response' AND r.continuation='Terminal' AND r.retry_decision='NoRetry' AND r.backoff_ms IS NULL \
           AND r.result_codec_version=1 AND r.result_length=length(r.result_bytes) \
           AND f.terminal_attempt_ordinal=r.attempt_ordinal AND f.terminal_result_run_version=r.run_version \
           AND f.terminal_result_sha256=r.result_sha256 AND f.error_material_run_version IS NULL AND f.error_material_sha256 IS NULL \
           AND f.outer_outcome='Returned' AND ore.outcome='Returned' AND ob.effect_key=f.code \
           AND f.outer_begin_run_version=ob.run_version AND f.outer_begin_sha256=ob.request_sha256 \
           AND f.outer_result_run_version=ore.run_version AND f.outer_result_sha256=ore.result_sha256 \
           AND f.final_length=length(f.final_bytes) AND f.final_sha256=?3 AND r.result_sha256=?4 \
           AND f.outer_result_sha256=?5 AND f.outer_begin_sha256=?6 \
           AND f.outer_begin_run_version+1=f.outer_result_run_version \
           AND f.prior_head_version=ore.run_version AND f.run_version=f.prior_head_version+1 \
           AND f.run_id=run.run_id AND f.run_context_sha256=run.run_context_sha256 AND f.input_sha256=run.input_sha256 \
           AND f.lease_owner='TEST_CODE_MEMBERSHIP_SUCCESS_OWNER_A' AND f.lease_generation=1 AND f.applied_at=?7",
        rusqlite::params![intent_id.as_str(), request.request_id,
            hex::encode(sha2::Sha256::digest(&final_bytes)), hex::encode(sha2::Sha256::digest(&result_bytes)),
            hex::encode(sha2::Sha256::digest(TOOL_JSON.as_bytes())), hex::encode(sha2::Sha256::digest(b"TEST_CODE_600001")), at(1_100)],
        |row| row.get(0)
    ).unwrap();
    assert_eq!(bound, 1);
    assert!(facts.attempt_begins[0].2 < result_version && result_version < outer_version);

    // The original cache policy uses Local; derive only from a fixed literal, never from now.
    let cache_time = chrono::DateTime::parse_from_rfc3339("2026-07-21T15:31:00+08:00")
        .unwrap()
        .with_timezone(&chrono::Local)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    let cache_rows = all_rows(connection,
        "SELECT intent_id,effect_kind,effect_ordinal,code,provider_result_run_version,concepts_codec_version, \
         CAST(concepts_bytes AS BLOB),concepts_length,concepts_sha256,cache_updated_at,lease_owner,lease_generation,run_version,written_at \
         FROM chain_post_close_concept_cache_writes");
    let cache_version: i64 = connection
        .query_row(
            "SELECT run_version FROM chain_post_close_concept_cache_writes",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        cache_rows,
        vec![vec![
            Value::Text(intent_id.as_str().to_owned()),
            Value::Text("ConceptProvider".to_owned()),
            Value::Integer(0),
            Value::Text("TEST_CODE_600001".to_owned()),
            Value::Integer(outer_version),
            Value::Integer(1),
            Value::Blob(CACHE.to_vec()),
            Value::Integer(i64::try_from(CACHE.len()).unwrap()),
            Value::Text(hex::encode(sha2::Sha256::digest(CACHE))),
            Value::Text(cache_time.clone()),
            Value::Text("TEST_CODE_MEMBERSHIP_SUCCESS_OWNER_A".to_owned()),
            Value::Integer(1),
            Value::Integer(cache_version),
            Value::Integer(at(1_100)),
        ]]
    );
    assert_eq!(cache_version, outer_version + 2);
    assert_eq!(
        all_rows(
            connection,
            "SELECT code,concepts,updated_at FROM stock_concepts WHERE code='TEST_CODE_600001'"
        ),
        vec![vec![
            Value::Text("TEST_CODE_600001".to_owned()),
            Value::Text(std::str::from_utf8(CACHE).unwrap().to_owned()),
            Value::Text(cache_time)
        ]]
    );
    assert_eq!(all_rows(connection, "SELECT date,concept,stocks,continuation_count FROM chain_daily WHERE date='2026-07-21' AND concept='TEST_CODE_CLUSTER_A_MAIN'"),
        vec![vec![Value::Text("2026-07-21".to_owned()), Value::Text("TEST_CODE_CLUSTER_A_MAIN".to_owned()),
            Value::Text(r#"["TEST_CODE_600001","TEST_CODE_CLUSTER_STOCK_A","TEST_CODE_CLUSTER_STOCK_B"]"#.to_owned()), Value::Integer(1)]]);
    let membership_receipt = connection.query_row(
        "SELECT audit_id,audit_record_hash,previous_outcome,current_outcome FROM chain_post_close_concept_rpc_finals", [],
        |row| Ok(DataAcquisitionAuditReceipt { audit_id: row.get(0)?, record_hash: row.get(1)?, previous_outcome: row.get(2)?, current_outcome: row.get(3)? })
    ).unwrap();
    let receipts = [&[membership_receipt][..], board_receipts.as_slice()].concat();
    assert_eq!(
        receipts
            .iter()
            .map(|value| value.audit_id)
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(
        receipts
            .iter()
            .map(|value| value.previous_outcome.as_deref())
            .collect::<Vec<_>>(),
        [None, None, Some("available")]
    );
    assert!(receipts
        .iter()
        .all(|value| value.current_outcome == "available"));
    let transaction = connection.unchecked_transaction().unwrap();
    for (index, receipt) in receipts.iter().enumerate() {
        let verified = read_acquisition_in_transaction(&transaction, receipt).unwrap();
        assert_eq!(verified.receipt(), receipt);
        let record = verified.record();
        let (capability, source, request_hash, batch) = [
            (
                "board-memberships",
                "TEST_CODE_LOOPBACK_MEMBERSHIP_SOURCE",
                MEMBERSHIP_REQUEST_HASH,
                "TEST_CODE_MEMBERSHIP_BATCH",
            ),
            (
                "board-directory",
                "TEST_CODE_LOOPBACK_BOARD_SOURCE",
                BOARD_REQUEST_HASHES[0],
                "TEST_CODE_INDUSTRY_BATCH",
            ),
            (
                "board-directory",
                "TEST_CODE_LOOPBACK_BOARD_SOURCE",
                BOARD_REQUEST_HASHES[1],
                "TEST_CODE_CONCEPT_BATCH",
            ),
        ][index];
        assert_eq!(record.capability, capability);
        assert_eq!(record.provider, "Tdx");
        assert_eq!(record.source, source);
        assert_eq!(record.request_hash, request_hash);
        assert_eq!(record.source_at, Some("2026-07-21T15:30:00+08:00"));
        assert_eq!(record.observed_at, "2026-07-21T15:31:00+08:00");
        assert_eq!(record.batch_id, Some(batch));
        assert_eq!(record.outcome, "available");
        assert_eq!(record.request_count, 1);
        assert_eq!(record.accepted_count, 2);
        assert_eq!(record.rejected_count, 0);
        assert_eq!(record.reason_code, "accepted");
        assert!(!record.retryable);
    }
    transaction.rollback().unwrap();

    let names = legacy_table_names(fixture.connection());
    let mut expected_rows = legacy_table_values(fixture.connection(), &names, 7);
    let catalog = all_rows(
        fixture.connection(),
        "SELECT name,type,tbl_name,CAST(sql AS BLOB) FROM main.sqlite_schema ORDER BY name",
    );
    let columns = all_rows(
        fixture.connection(),
        "PRAGMA main.table_info('chain_post_close_runs')",
    );
    let run = expected_rows.get_mut("chain_post_close_runs").unwrap();
    assert_eq!(run.len(), 1);
    for (name, value) in [
        (
            "lease_owner",
            Value::Text("TEST_CODE_MEMBERSHIP_SUCCESS_OWNER_B".to_owned()),
        ),
        ("lease_generation", Value::Integer(2)),
        (
            "head_version",
            Value::Integer(i64::try_from(head + 1).unwrap()),
        ),
        ("lease_until", Value::Integer(at(9_000))),
        ("updated_at", Value::Integer(at(5_001))),
    ] {
        let ordinal = columns
            .iter()
            .position(|column| column[1] == Value::Text(name.to_owned()))
            .unwrap();
        run[0][ordinal] = value;
    }
    fixture.reopen();
    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let lease = local
        .resume_run(
            &intent_id,
            lease_request(
                "TEST_CODE_MEMBERSHIP_SUCCESS_OWNER_B",
                5_001,
                9_000,
                Some(head),
            ),
        )
        .unwrap();
    let clock = ControlledClock::new(at(5_100));
    let mut io = local
        .concept_rpc_preparation_io_v7(
            lease,
            &queries,
            &clock,
            FixedClusterConfiguration::resolve(Some("2")),
        )
        .unwrap();
    let reopened = tokio::time::timeout(
        Duration::from_secs(5),
        prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks.clone(),
            None,
            &mut io,
        ),
    )
    .await
    .expect("TEST_CODE reopened success prepare deadline")
    .expect_err("TEST_CODE reopened success stops at Positions");
    assert_success_boundary(
        "reopened",
        &reopened,
        &expected_directory,
        &expected_selected,
    );
    drop(io);
    let (reopened_head, reopened_receipts) = assert_success_readers(
        "reopened",
        &mut local,
        &intent_id,
        &stocks,
        &expected_directory,
        &expected_selected,
    );
    assert_eq!(reopened_head, head + 1);
    assert_eq!(reopened_receipts, board_receipts);
    drop(local);
    assert_eq!(legacy_table_names(fixture.connection()), names);
    assert_eq!(
        legacy_table_values(fixture.connection(), &names, 7),
        expected_rows
    );
    assert_eq!(
        all_rows(
            fixture.connection(),
            "SELECT name,type,tbl_name,CAST(sql AS BLOB) FROM main.sqlite_schema ORDER BY name"
        ),
        catalog
    );
    assert_eq!(concept_rpc_facts(fixture.connection()), facts);
    let transaction = fixture.connection().unchecked_transaction().unwrap();
    for receipt in &receipts {
        assert_eq!(
            read_acquisition_in_transaction(&transaction, receipt)
                .unwrap()
                .receipt(),
            receipt
        );
    }
    transaction.rollback().unwrap();
    assert_eq!(server.membership_snapshot(), memberships);
    assert_eq!(server.snapshot(), board_observation);
    server.finish().await;
}

#[tokio::test]
async fn historical_nonfinal_audit_damage_is_rejected_without_rpc_or_fact_rewrite() {
    use crate::grpc_client::client::board_loopback_fixture::spawn_membership_success_loopback;
    use rusqlite::types::Value;

    const DAMAGED_HASH: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

    let mut fixture = V2BusinessFixture::new();
    install_v7(&mut fixture);
    let (client, server) = spawn_membership_success_loopback().await;
    let source = GrpcSource::from_board_loopback_test_client(client);
    let queries = tokio::time::timeout(Duration::from_secs(5), source.connected_board_queries())
        .await
        .expect("TEST_CODE historical-audit preconnect deadline")
        .unwrap();
    let mut stocks = cluster_tests::cluster_stocks();
    stocks.push(stock());
    let expected_directory = BTreeMap::from([
        (
            "TEST_CODE_CLUSTER_A_MAIN".to_owned(),
            "TEST_CODE_BOARD_MAIN".to_owned(),
        ),
        (
            "TEST_CODE_CONCEPT_ONLY".to_owned(),
            "TEST_CODE_BOARD_CONCEPT_ONLY".to_owned(),
        ),
        (
            "TEST_CODE_INDUSTRY_ONLY".to_owned(),
            "TEST_CODE_BOARD_INDUSTRY_ONLY".to_owned(),
        ),
    ]);
    let expected_selected = BTreeMap::from([(
        "TEST_CODE_CLUSTER_A_MAIN".to_owned(),
        "TEST_CODE_BOARD_MAIN".to_owned(),
    )]);
    let config = local_config(BUILD_A);
    let context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_RUN_HISTORICAL_AUDIT_DAMAGE"),
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
            lease_request("TEST_CODE_HISTORICAL_AUDIT_OWNER", 1_000, 5_000, None),
        )
        .unwrap();
    let intent_id = lease.intent_id().clone();
    let clock = ControlledClock::new(at(1_100));
    let mut io = local
        .concept_rpc_preparation_io_v7(
            lease,
            &queries,
            &clock,
            FixedClusterConfiguration::resolve(Some("2")),
        )
        .unwrap();
    let stopped = tokio::time::timeout(
        Duration::from_secs(15),
        prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks.clone(),
            None,
            &mut io,
        ),
    )
    .await
    .expect("TEST_CODE historical-audit prepare deadline")
    .expect_err("TEST_CODE historical-audit prepare stops at Positions");
    assert!(matches!(
        stopped.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::StageNotMigrated {
            next: UnmigratedStage::Positions
        })
    ));
    assert_candidates_completed_before_positions(&stopped, &expected_directory, &expected_selected);
    drop(io);
    let healthy = local
        .inspect_run(&intent_id)
        .expect("TEST_CODE healthy public run reader");
    assert_eq!(healthy.begins().len(), 1);
    assert_eq!(healthy.results().len(), 1);
    drop(healthy);
    drop(local);

    let facts = concept_rpc_facts(fixture.connection());
    assert_eq!((facts.finals, facts.audits, facts.audit_chain), (1, 3, 3));
    let final_audit_ids = all_rows(
        fixture.connection(),
        "SELECT audit_id FROM chain_post_close_concept_rpc_finals ORDER BY audit_id",
    );
    assert_eq!(final_audit_ids, vec![vec![Value::Integer(1)]]);
    let chain_ids = all_rows(
        fixture.connection(),
        "SELECT acquisition_audit_id FROM data_acquisition_audit_chain \
         ORDER BY acquisition_audit_id",
    );
    assert_eq!(
        chain_ids,
        vec![
            vec![Value::Integer(1)],
            vec![Value::Integer(2)],
            vec![Value::Integer(3)]
        ]
    );
    let actual_tail: i64 = fixture
        .connection()
        .query_row(
            "SELECT MAX(acquisition_audit_id) FROM data_acquisition_audit_chain",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(actual_tail, 3);
    let target_audit_id = 2_i64;
    assert!(target_audit_id < actual_tail);
    assert_eq!(
        fixture
            .connection()
            .query_row(
                "SELECT count(*) FROM chain_post_close_concept_rpc_finals WHERE audit_id=?1",
                [target_audit_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );

    let names = legacy_table_names(fixture.connection());
    let healthy_rows = legacy_table_values(fixture.connection(), &names, 7);
    let healthy_catalog = all_rows(
        fixture.connection(),
        "SELECT type,name,tbl_name,CAST(sql AS BLOB) FROM main.sqlite_schema \
         ORDER BY type,name,tbl_name,sql",
    );
    let memberships = server.membership_snapshot();
    let board_observation = server.snapshot();
    assert_eq!(memberships.len(), 1);
    assert_eq!(board_observation.requests.len(), 3);

    let trigger_sql: String = fixture
        .connection()
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type='trigger' \
             AND name='trg_data_acquisition_audit_chain_no_update'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let original_hash: String = fixture
        .connection()
        .query_row(
            "SELECT record_hash FROM data_acquisition_audit_chain \
             WHERE acquisition_audit_id=?1",
            [target_audit_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_ne!(original_hash, DAMAGED_HASH);
    fixture.execute("DROP TRIGGER trg_data_acquisition_audit_chain_no_update;");
    fixture
        .connection()
        .execute(
            "UPDATE data_acquisition_audit_chain SET record_hash=?1 \
             WHERE acquisition_audit_id=?2",
            rusqlite::params![DAMAGED_HASH, target_audit_id],
        )
        .unwrap();
    fixture.execute(&trigger_sql);
    assert_eq!(
        all_rows(
            fixture.connection(),
            "SELECT type,name,tbl_name,CAST(sql AS BLOB) FROM main.sqlite_schema \
             ORDER BY type,name,tbl_name,sql"
        ),
        healthy_catalog
    );

    let chain_columns = all_rows(
        fixture.connection(),
        "PRAGMA main.table_info('data_acquisition_audit_chain')",
    );
    let id_column = chain_columns
        .iter()
        .position(|column| column[1] == Value::Text("acquisition_audit_id".to_owned()))
        .unwrap();
    let hash_column = chain_columns
        .iter()
        .position(|column| column[1] == Value::Text("record_hash".to_owned()))
        .unwrap();
    let mut expected_damaged_rows = healthy_rows.clone();
    let chain_rows = expected_damaged_rows
        .get_mut("data_acquisition_audit_chain")
        .unwrap();
    let target_row = chain_rows
        .iter_mut()
        .find(|row| row[id_column] == Value::Integer(target_audit_id))
        .unwrap();
    target_row[hash_column] = Value::Text(DAMAGED_HASH.to_owned());
    let damaged_rows = legacy_table_values(fixture.connection(), &names, 7);
    assert_eq!(damaged_rows, expected_damaged_rows);
    assert_eq!(server.membership_snapshot(), memberships);
    assert_eq!(server.snapshot(), board_observation);

    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let inspected = local
        .inspect_run(&intent_id)
        .map(|run| (run.head_version(), run.begins().len(), run.results().len()));
    drop(local);
    assert_eq!(legacy_table_names(fixture.connection()), names);
    assert_eq!(
        legacy_table_values(fixture.connection(), &names, 7),
        damaged_rows
    );
    assert_eq!(
        all_rows(
            fixture.connection(),
            "SELECT type,name,tbl_name,CAST(sql AS BLOB) FROM main.sqlite_schema \
             ORDER BY type,name,tbl_name,sql"
        ),
        healthy_catalog
    );
    assert_eq!(server.membership_snapshot(), memberships);
    assert_eq!(server.snapshot(), board_observation);
    drop(queries);
    drop(source);
    let finished = server.finish().await;
    assert_eq!(finished, board_observation);
    match inspected {
        Err(ChainPostCloseError::SchemaRejected) => {}
        Ok((head, begins, results)) => panic!(
            "TEST_CODE historical-audit reader unexpectedly succeeded: \
             head={head}, begins={begins}, results={results}"
        ),
        Err(_) => panic!("TEST_CODE historical-audit reader returned wrong safe error category"),
    }
}

#[tokio::test]
async fn legacy_pending_cache_migrates_and_reopens_without_membership_replay() {
    use super::super::super::{ConceptEffectRecoveryState, LocalChainPostClose};
    use rusqlite::types::Value;
    use sha2::Digest as _;

    const RAW: &str = r#"{"all_boards":["TEST_CODE_CLUSTER_A_MAIN","TEST_CODE_CLUSTER_B_ALIAS"]}"#;
    const CACHE: &[u8] = br#"["TEST_CODE_CLUSTER_A_MAIN","TEST_CODE_CLUSTER_B_ALIAS"]"#;
    const QUALIFICATION_SQL: &str =
        "SELECT * FROM chain_post_close_concept_rpc_legacy_outer_qualifications ORDER BY intent_id,outer_ordinal";
    const CATALOG_SQL: &str =
        "SELECT name,type,tbl_name,CAST(sql AS BLOB) FROM main.sqlite_schema ORDER BY name";
    const ORIGINAL_FACT_TABLES: [&str; 3] = [
        "chain_post_close_runs",
        "chain_post_close_stage_begins",
        "chain_post_close_stage_results",
    ];

    struct OnceReturned(Cell<usize>);
    #[async_trait::async_trait(?Send)]
    impl ConceptProviderRawIo for OnceReturned {
        async fn call_raw(&self, code: &str) -> Result<String, String> {
            assert_eq!(code, "TEST_CODE_600001");
            assert_eq!(
                self.0.replace(self.0.get() + 1),
                0,
                "TEST_CODE Raw must be called once"
            );
            Ok(RAW.to_owned())
        }
    }
    // Pre-open one owned descriptor; never open/close another descriptor while its lock is held.
    struct PendingCacheClock {
        reader: Connection,
        intent: String,
        armed: Cell<bool>,
        before_cache: RefCell<Option<BTreeMap<String, LegacySqlRows>>>,
    }
    impl ConceptEffectClock for PendingCacheClock {
        fn now(&self) -> UtcMicros {
            if !self.armed.get() {
                let ready: (i64, i64, i64) = self.reader.query_row(
                    "SELECT \
                     (SELECT count(*) FROM chain_post_close_stage_results r \
                      JOIN chain_post_close_stage_begins b ON b.intent_id=r.intent_id \
                        AND b.effect_kind=r.effect_kind AND b.effect_ordinal=r.effect_ordinal \
                      WHERE r.intent_id=?1 AND r.effect_kind='ConceptProvider' AND r.effect_ordinal=0 \
                        AND b.effect_key='TEST_CODE_600001' AND r.outcome='Returned'), \
                     (SELECT count(*) FROM chain_post_close_concept_cache_writes WHERE intent_id=?1), \
                     (SELECT count(*) FROM stock_concepts WHERE code='TEST_CODE_600001')",
                    [&self.intent], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                ).unwrap();
                if ready == (1, 0, 0) {
                    self.reader.execute_batch("BEGIN DEFERRED").unwrap();
                    // Reading the original facts acquires a real SHARED lock before cache BEGIN IMMEDIATE.
                    let names = ORIGINAL_FACT_TABLES.map(str::to_owned);
                    *self.before_cache.borrow_mut() =
                        Some(legacy_table_values(&self.reader, &names, 6));
                    assert!(!self.reader.is_autocommit());
                    self.armed.set(true);
                }
            }
            UtcMicros::try_new(at(1_100)).unwrap()
        }
    }

    #[track_caller]
    fn assert_pending_progress(
        phase: &str,
        error: &anyhow::Error,
        local: &mut LocalChainPostClose<'_>,
        intent: &IntentId,
        stocks: &[TopStock],
        directory: &BTreeMap<String, String>,
        selected: &BTreeMap<String, String>,
    ) -> u64 {
        eprintln!("TEST_CODE pending-cache phase={phase} completed-boundary/readers");
        assert_candidates_completed_before_positions(error, directory, selected);
        let concepts = local.inspect_concept_batch(intent).unwrap();
        assert!(
            concepts.is_complete(),
            "TEST_CODE {phase}: complete concepts"
        );
        assert_eq!(concepts.applied_codes(), ["TEST_CODE_600001"]);
        assert_eq!(concepts.effects().len(), 1);
        assert_eq!(concepts.effects()[0].ordinal(), 0);
        assert_eq!(concepts.effects()[0].code(), "TEST_CODE_600001");
        assert_eq!(
            concepts.effects()[0].state(),
            ConceptEffectRecoveryState::Confirmed
        );
        let expected = [
            (
                "TEST_CODE_CLUSTER_STOCK_A",
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
                "TEST_CODE_600001",
                vec!["TEST_CODE_CLUSTER_A_MAIN", "TEST_CODE_CLUSTER_B_ALIAS"],
            ),
        ]
        .into_iter()
        .map(|(code, names)| {
            (
                code.to_owned(),
                names.into_iter().map(str::to_owned).collect::<Vec<_>>(),
            )
        })
        .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(concepts.concepts(), &expected);
        let material = local.inspect_cluster_application(intent).unwrap();
        assert_eq!(material.min_cluster_size(), 2);
        assert_eq!(material.clusters().len(), 1);
        let cluster = &material.clusters()[0];
        assert_eq!(cluster.concept, "TEST_CODE_CLUSTER_A_MAIN");
        assert_eq!(cluster.aliases, ["TEST_CODE_CLUSTER_B_ALIAS"]);
        assert_eq!(
            serde_json::to_value(&cluster.stocks).unwrap(),
            serde_json::to_value([&stocks[3], &stocks[0], &stocks[1]]).unwrap()
        );
        assert_eq!(
            serde_json::to_value(material.isolated()).unwrap(),
            serde_json::to_value([&stocks[2]]).unwrap()
        );
        assert_eq!((cluster.continuation_count, cluster.streak_days), (1, 3));
        assert!(
            cluster.candidates.is_empty() && cluster.score.is_none() && cluster.scenario.is_none()
        );
        assert_eq!(
            material.lifecycle_days(),
            &BTreeMap::from([("TEST_CODE_CLUSTER_A_MAIN".to_owned(), 3)])
        );
        let run = local.inspect_run(intent).unwrap();
        assert_eq!(run.begins().len(), 1);
        assert_eq!(run.results().len(), 1);
        assert_eq!(run.results()[0].outcome, "Returned");
        assert_eq!(run.results()[0].bytes, RAW.as_bytes());
        run.head_version()
    }

    fn set_expected_run(
        rows: &mut BTreeMap<String, LegacySqlRows>,
        columns: &LegacySqlRows,
        owner: &str,
        generation: i64,
        head: u64,
        until: i64,
        updated: i64,
    ) {
        let run = rows.get_mut("chain_post_close_runs").unwrap();
        assert_eq!(run.len(), 1);
        for (name, value) in [
            ("lease_owner", Value::Text(owner.to_owned())),
            ("lease_generation", Value::Integer(generation)),
            ("head_version", Value::Integer(i64::try_from(head).unwrap())),
            ("lease_until", Value::Integer(at(until))),
            ("updated_at", Value::Integer(at(updated))),
        ] {
            let ordinal = columns
                .iter()
                .position(|column| column[1] == Value::Text(name.to_owned()))
                .unwrap();
            run[0][ordinal] = value;
        }
    }

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
    install_br159_in_owned_database(&mut fixture);
    fixture
        .chain_post_close()
        .migrate_schema_v4_to_v5()
        .unwrap();
    assert_eq!(
        fixture
            .chain_post_close()
            .migrate_schema_v5_to_v6()
            .unwrap()
            .schema_version(),
        6
    );
    let journal: String = fixture
        .connection()
        .query_row("PRAGMA journal_mode=DELETE", [], |row| row.get(0))
        .unwrap();
    assert_eq!(journal.to_ascii_lowercase(), "delete");
    let reader = Connection::open_with_flags(
        fixture.database(),
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .unwrap();
    reader.busy_timeout(Duration::ZERO).unwrap();

    let (client, server) = tokio::time::timeout(Duration::from_secs(5), spawn_board_loopback())
        .await
        .expect("TEST_CODE pending listener/connect deadline");
    let source = GrpcSource::from_board_loopback_test_client(client);
    let queries = tokio::time::timeout(Duration::from_secs(5), source.connected_board_queries())
        .await
        .expect("TEST_CODE pending preconnect deadline")
        .unwrap();
    let no_requests = server.snapshot();
    assert!(no_requests.requests.is_empty() && no_requests.non_board_requests == 0);
    assert!(server.membership_snapshot().is_empty());
    let mut stocks = cluster_tests::cluster_stocks();
    stocks.push(stock());
    let config = local_config(BUILD_A);
    let context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_LEGACY_PENDING_CACHE"),
    )
    .unwrap();
    let provider = OnceReturned(Cell::new(0));
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
            lease_request("TEST_CODE_PENDING_OWNER_A", 1_000, 5_000, None),
        )
        .unwrap();
    let intent = lease.intent_id().clone();
    let clock = PendingCacheClock {
        reader,
        intent: intent.as_str().to_owned(),
        armed: Cell::new(false),
        before_cache: RefCell::new(None),
    };
    let mut io = local
        .board_preparation_io_v6(
            lease,
            &provider,
            &clock,
            FixedClusterConfiguration::resolve(Some("2")),
            &source,
        )
        .unwrap();
    assert_eq!(
        io.local
            .store
            .connection
            .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        250,
        "TEST_CODE owned writer preserves the required 250ms cache COMMIT timeout"
    );
    let failed = tokio::time::timeout(
        Duration::from_secs(5),
        prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks.clone(),
            None,
            &mut io,
        ),
    )
    .await
    .expect("TEST_CODE v6 pending cache prepare deadline")
    .expect_err("TEST_CODE actual cache COMMIT must be rejected");
    assert!(matches!(failed.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::AuthorityRejected { intent_id }) if intent_id == intent.as_str()));
    assert!(matches!(
        failed.downcast_ref::<ChainPostCloseError>(),
        Some(ChainPostCloseError::StorageFailed {
            operation: "commit"
        })
    ));
    let failure = failed.downcast_ref::<PreparationFailure>().unwrap();
    assert_eq!(failure.stage(), PreparationStage::Concepts);
    assert!(failure.completed_stages().is_empty());
    assert_eq!(provider.0.get(), 1);
    drop(io);
    assert!(
        clock.armed.get(),
        "TEST_CODE shared-lock boundary was reached"
    );
    let before_cache = clock.before_cache.borrow_mut().take().unwrap();
    clock.reader.execute_batch("ROLLBACK").unwrap();
    assert!(clock.reader.is_autocommit());
    // A fresh snapshot proves cache transaction rollback retained complete old raw/head facts.
    assert_eq!(
        legacy_table_values(&clock.reader, &ORIGINAL_FACT_TABLES.map(str::to_owned), 6),
        before_cache
    );
    assert_eq!(
        count(&clock.reader, "chain_post_close_concept_cache_writes"),
        0
    );
    assert_eq!(
        clock
            .reader
            .query_row(
                "SELECT count(*) FROM stock_concepts WHERE code='TEST_CODE_600001'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    clock.reader.close().unwrap();
    let raw_run = local.inspect_run(&intent).unwrap();
    assert_eq!(raw_run.begins().len(), 1);
    assert_eq!(raw_run.results().len(), 1);
    let begin_version = raw_run.begins()[0].run_version;
    let mut head = raw_run.head_version();
    assert_eq!(head, raw_run.results()[0].run_version);
    assert_eq!(begin_version + 1, head);
    assert_eq!(raw_run.results()[0].bytes, RAW.as_bytes());
    assert_eq!(raw_run.results()[0].outcome, "Returned");
    drop(raw_run);
    drop(local);
    assert_eq!(server.snapshot(), no_requests);
    assert!(server.membership_snapshot().is_empty());

    let request_sha = hex::encode(sha2::Sha256::digest(b"TEST_CODE_600001"));
    let raw_sha = hex::encode(sha2::Sha256::digest(RAW.as_bytes()));
    let owner_a = Value::Text("TEST_CODE_PENDING_OWNER_A".to_owned());
    let begin_value = Value::Integer(i64::try_from(begin_version).unwrap());
    let result_value = Value::Integer(i64::try_from(head).unwrap());
    assert_eq!(
        all_rows(
            fixture.connection(),
            "SELECT * FROM chain_post_close_stage_begins"
        ),
        vec![vec![
            Value::Text(intent.as_str().to_owned()),
            Value::Text("ConceptProvider".to_owned()),
            Value::Integer(0),
            Value::Text("TEST_CODE_600001".to_owned()),
            Value::Integer(1),
            Value::Blob(b"TEST_CODE_600001".to_vec()),
            Value::Integer(16),
            Value::Text(request_sha.clone()),
            owner_a.clone(),
            Value::Integer(1),
            begin_value.clone(),
            Value::Integer(at(1_100)),
        ]]
    );
    assert_eq!(
        all_rows(
            fixture.connection(),
            "SELECT * FROM chain_post_close_stage_results"
        ),
        vec![vec![
            Value::Text(intent.as_str().to_owned()),
            Value::Text("ConceptProvider".to_owned()),
            Value::Integer(0),
            Value::Text("Returned".to_owned()),
            Value::Integer(1),
            Value::Blob(RAW.as_bytes().to_vec()),
            Value::Integer(i64::try_from(RAW.len()).unwrap()),
            Value::Text(raw_sha.clone()),
            owner_a.clone(),
            Value::Integer(1),
            result_value.clone(),
            Value::Integer(at(1_100)),
            Value::Integer(at(1_100)),
        ]]
    );
    assert_eq!(
        count(
            fixture.connection(),
            "chain_post_close_cluster_configurations"
        ),
        1
    );
    for name in [
        "chain_post_close_concept_cache_writes",
        "chain_post_close_cluster_materials",
        "chain_post_close_chain_daily_applications",
        "chain_post_close_board_attempt_begins",
        "chain_post_close_board_attempt_results",
        "chain_post_close_board_kind_finals",
        "chain_post_close_board_directory_materials",
        "chain_post_close_board_selections",
        "chain_post_close_board_status_materials",
        "chain_post_close_board_error_materials",
        "data_acquisition_audit",
        "data_acquisition_audit_chain",
    ] {
        assert_eq!(
            count(fixture.connection(), name),
            0,
            "TEST_CODE first failure table={name}"
        );
    }
    let old_names = legacy_table_names(fixture.connection());
    let old_values = legacy_table_values(fixture.connection(), &old_names, 6);
    let old_catalog = all_rows(fixture.connection(), CATALOG_SQL);
    let qualification = vec![vec![
        Value::Text(intent.as_str().to_owned()),
        Value::Text("ConceptProvider".to_owned()),
        Value::Integer(0),
        Value::Text("TEST_CODE_600001".to_owned()),
        Value::Text("LegacyReturnedPendingCache".to_owned()),
        begin_value,
        Value::Text(request_sha),
        owner_a.clone(),
        Value::Integer(1),
        Value::Integer(at(1_100)),
        Value::Text("Returned".to_owned()),
        result_value,
        Value::Text(raw_sha),
        owner_a,
        Value::Integer(1),
        Value::Integer(at(1_100)),
        Value::Null,
        Value::Null,
        Value::Null,
        Value::Null,
        Value::Null,
        Value::Integer(6),
        Value::Integer(7),
    ]];
    assert_eq!(qualification[0].len(), 23);
    assert_eq!(
        fixture
            .chain_post_close()
            .migrate_schema_v6_to_v7()
            .unwrap()
            .schema_version(),
        7
    );
    assert_eq!(
        fixture
            .chain_post_close()
            .verify_schema()
            .unwrap()
            .schema_version(),
        7
    );
    assert_eq!(
        legacy_table_values(fixture.connection(), &old_names, 6),
        old_values
    );
    let catalog = all_rows(fixture.connection(), CATALOG_SQL);
    assert!(old_catalog.iter().all(|row| catalog.contains(row)));
    let definitions = all_rows(fixture.connection(),
        "SELECT name,object_type,CAST(definition AS BLOB) FROM chain_post_close_layout_objects WHERE layout_version=7 ORDER BY name");
    assert_eq!(
        definitions.len(),
        99,
        "TEST_CODE complete v7 registry, not 28-object delta"
    );
    assert_eq!(
        definitions,
        catalog
            .iter()
            .filter(
                |row| matches!(&row[0], Value::Text(name) if name.starts_with("chain_post_close_"))
                    && row[3] != Value::Null
            )
            .map(|row| vec![row[0].clone(), row[1].clone(), row[3].clone()])
            .collect::<Vec<_>>()
    );
    assert_eq!(
        all_rows(fixture.connection(), QUALIFICATION_SQL),
        qualification
    );
    let migrated_rpc = concept_rpc_facts(fixture.connection());
    assert_eq!(
        (
            migrated_rpc.occurrences.len(),
            migrated_rpc.attempt_begins.len(),
            migrated_rpc.results,
            migrated_rpc.status_materials,
            migrated_rpc.error_materials,
            migrated_rpc.finals
        ),
        (0, 0, 0, 0, 0, 0)
    );
    assert_eq!(
        (
            migrated_rpc.cache_writes,
            migrated_rpc.cached_concepts,
            migrated_rpc.cluster_materials,
            migrated_rpc.chain_daily_applications,
            migrated_rpc.audits,
            migrated_rpc.audit_chain
        ),
        (0, 0, 0, 0, 0, 0)
    );

    let names = legacy_table_names(fixture.connection());
    let migrated_rows = legacy_table_values(fixture.connection(), &names, 7);
    let run_columns = all_rows(
        fixture.connection(),
        "PRAGMA main.table_info('chain_post_close_runs')",
    );
    let expected_directory = BTreeMap::from([
        (
            "TEST_CODE_CLUSTER_A_MAIN".to_owned(),
            "TEST_CODE_BOARD_MAIN".to_owned(),
        ),
        (
            "TEST_CODE_CONCEPT_ONLY".to_owned(),
            "TEST_CODE_BOARD_CONCEPT_ONLY".to_owned(),
        ),
        (
            "TEST_CODE_INDUSTRY_ONLY".to_owned(),
            "TEST_CODE_BOARD_INDUSTRY_ONLY".to_owned(),
        ),
    ]);
    let expected_selected = BTreeMap::from([(
        "TEST_CODE_CLUSTER_A_MAIN".to_owned(),
        "TEST_CODE_BOARD_MAIN".to_owned(),
    )]);
    let mut completed_rows = None;
    let mut completed_requests = None;
    for (phase, owner, generation, resume_at, until, now) in [
        (
            "first-v7",
            "TEST_CODE_PENDING_OWNER_B",
            2,
            5_001,
            9_000,
            5_100,
        ),
        (
            "second-v7",
            "TEST_CODE_PENDING_OWNER_C",
            3,
            9_001,
            13_000,
            9_100,
        ),
    ] {
        fixture.reopen();
        let prior_rows = completed_rows.as_ref().unwrap_or(&migrated_rows);
        assert_eq!(
            &legacy_table_values(fixture.connection(), &names, 7),
            prior_rows,
            "TEST_CODE {phase}: true reopen"
        );
        let previous_head = head;
        let mut local = fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .unwrap();
        let lease = local
            .resume_run(&intent, lease_request(owner, resume_at, until, Some(head)))
            .unwrap();
        let clock = ControlledClock::new(at(now));
        let mut io = local
            .concept_rpc_preparation_io_v7(
                lease,
                &queries,
                &clock,
                FixedClusterConfiguration::resolve(Some("2")),
            )
            .unwrap();
        let error = tokio::time::timeout(
            Duration::from_secs(15),
            prepare_chain_analysis_with_io(
                NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
                stocks.clone(),
                None,
                &mut io,
            ),
        )
        .await
        .unwrap_or_else(|_| panic!("TEST_CODE {phase}: prepare deadline"))
        .expect_err("TEST_CODE pending continuation stops before Positions");
        drop(io);
        head = assert_pending_progress(
            phase,
            &error,
            &mut local,
            &intent,
            &stocks,
            &expected_directory,
            &expected_selected,
        );
        let board = local.inspect_board_directory(&intent).unwrap();
        assert_eq!(board.board_directory(), &expected_directory);
        assert_eq!(board.selected_board_codes(), &expected_selected);
        let observed = server.snapshot();
        assert_eq!(
            observed.requests.len(),
            3,
            "TEST_CODE {phase}: only first continuation sends board RPC"
        );
        assert_eq!(observed.non_board_requests, 0);
        assert!(server.membership_snapshot().is_empty());
        assert_eq!(provider.0.get(), 1);
        assert_eq!(
            observed
                .requests
                .iter()
                .map(|r| r.kind.as_str())
                .collect::<Vec<_>>(),
            ["Industry", "Industry", "Concept"]
        );
        assert!(observed.requests.iter().all(|r| r.authorized
            && r.limit == 10_000
            && r.protocol_version == 1
            && r.payload_schema == "board.directory"
            && r.payload_schema_version == 1
            && r.payload_content_type == "application/json; charset=utf-8"
            && !r.allow_unadmitted
            && !r.request_id.is_empty()));
        assert_eq!(
            observed.requests[0].request_id,
            observed.requests[1].request_id
        );
        assert_ne!(
            observed.requests[1].request_id,
            observed.requests[2].request_id
        );
        assert_eq!(board.attempts().len(), 3);
        for (attempt, request) in board.attempts().iter().zip(&observed.requests) {
            assert!(attempt.is_confirmed());
            assert_eq!(attempt.request_id(), request.request_id);
        }
        assert_eq!(
            board.attempts()[0].error_detail_bytes(),
            Some(observed.retry_error_detail.as_slice())
        );
        assert_eq!(
            board.attempts()[1].payload_bytes(),
            Some(INDUSTRY_DIRECTORY_BYTES)
        );
        assert_eq!(
            board.attempts()[2].payload_bytes(),
            Some(CONCEPT_DIRECTORY_BYTES)
        );
        assert_eq!(board.directories().len(), 2);
        let receipts = board
            .directories()
            .iter()
            .map(|d| d.receipt().clone())
            .collect::<Vec<_>>();
        drop(board);
        drop(local);

        let rpc = concept_rpc_facts(fixture.connection());
        assert_eq!(
            (
                rpc.occurrences.len(),
                rpc.attempt_begins.len(),
                rpc.results,
                rpc.status_materials,
                rpc.error_materials,
                rpc.finals
            ),
            (0, 0, 0, 0, 0, 0)
        );
        assert_eq!(
            (
                rpc.outer_begins,
                rpc.outer_results,
                rpc.cache_writes,
                rpc.cluster_materials,
                rpc.chain_daily_applications,
                rpc.audits,
                rpc.audit_chain
            ),
            (1, 1, 1, 1, 1, 2, 2)
        );
        assert_eq!(
            all_rows(fixture.connection(), QUALIFICATION_SQL),
            qualification,
            "TEST_CODE {phase}: migration qualification remains Pending and five NULLs"
        );
        let transaction = fixture.connection().unchecked_transaction().unwrap();
        for (index, receipt) in receipts.iter().enumerate() {
            assert_eq!(receipt.audit_id, i64::try_from(index + 1).unwrap());
            assert_eq!(
                receipt.previous_outcome.as_deref(),
                if index == 0 { None } else { Some("available") }
            );
            assert_eq!(receipt.current_outcome, "available");
            let verified = read_acquisition_in_transaction(&transaction, receipt).unwrap();
            assert_eq!(verified.receipt(), receipt);
            let record = verified.record();
            assert_eq!(record.capability, "board-directory");
            assert_eq!(record.provider, "Tdx");
            assert_eq!(record.source, "TEST_CODE_LOOPBACK_BOARD_SOURCE");
            assert_eq!(record.request_hash, BOARD_REQUEST_HASHES[index]);
            assert_eq!(record.source_at, Some("2026-07-21T15:30:00+08:00"));
            assert_eq!(record.observed_at, "2026-07-21T15:31:00+08:00");
            assert_eq!(record.batch_id, Some(BOARD_BATCH_IDS[index]));
            assert_eq!(record.outcome, "available");
            assert_eq!(record.request_count, 1);
            assert_eq!(record.accepted_count, 2);
            assert_eq!(record.rejected_count, 0);
            assert_eq!(record.reason_code, "accepted");
            assert!(!record.retryable);
        }
        transaction.rollback().unwrap();
        let current = legacy_table_values(fixture.connection(), &names, 7);
        assert_eq!(legacy_table_names(fixture.connection()), names);
        assert_eq!(all_rows(fixture.connection(), CATALOG_SQL), catalog);
        if generation == 2 {
            // Only these first applications may append; all preexisting rows outside business targets stay exact.
            let appends = BTreeMap::from([
                ("chain_post_close_concept_cache_writes", 1),
                ("chain_post_close_cluster_materials", 1),
                ("chain_post_close_chain_daily_applications", 1),
                ("chain_post_close_board_attempt_begins", 3),
                ("chain_post_close_board_attempt_results", 3),
                ("chain_post_close_board_status_materials", 1),
                ("chain_post_close_board_kind_finals", 2),
                ("chain_post_close_board_directory_materials", 1),
                ("chain_post_close_board_selections", 1),
                ("data_acquisition_audit", 2),
                ("data_acquisition_audit_chain", 2),
            ]);
            for (name, before) in &migrated_rows {
                let after = &current[name];
                if let Some(expected) = appends.get(name.as_str()) {
                    assert!(before.is_empty());
                    assert_eq!(
                        after.len(),
                        *expected,
                        "TEST_CODE first-v7 append table={name}"
                    );
                    continue;
                }
                match name.as_str() {
                    "chain_post_close_runs" => {
                        let mut expected = migrated_rows.clone();
                        set_expected_run(
                            &mut expected,
                            &run_columns,
                            owner,
                            generation,
                            head,
                            until,
                            now,
                        );
                        assert_eq!(after, &expected[name]);
                    }
                    "stock_concepts" => {
                        assert_eq!(after.len(), before.len() + 1);
                        assert_eq!(
                            after
                                .iter()
                                .filter(|row| row[0] != Value::Text("TEST_CODE_600001".to_owned()))
                                .cloned()
                                .collect::<LegacySqlRows>(),
                            *before
                        );
                    }
                    "chain_daily" => {
                        let target = |row: &&Vec<Value>| {
                            row[0] == Value::Text("2026-07-21".to_owned())
                                && row[1] == Value::Text("TEST_CODE_CLUSTER_A_MAIN".to_owned())
                        };
                        assert_eq!(after.len(), before.len());
                        assert_eq!(
                            after
                                .iter()
                                .filter(|row| !target(row))
                                .cloned()
                                .collect::<LegacySqlRows>(),
                            before
                                .iter()
                                .filter(|row| !target(row))
                                .cloned()
                                .collect::<LegacySqlRows>()
                        );
                    }
                    "sqlite_sequence" => {
                        let other = |rows: &LegacySqlRows| {
                            rows.iter()
                                .filter(|row| {
                                    row[0] != Value::Text("data_acquisition_audit".to_owned())
                                })
                                .cloned()
                                .collect::<LegacySqlRows>()
                        };
                        assert_eq!(other(after), other(before));
                        assert_eq!(
                            after
                                .iter()
                                .filter(|row| row[0]
                                    == Value::Text("data_acquisition_audit".to_owned()))
                                .cloned()
                                .collect::<LegacySqlRows>(),
                            vec![vec![
                                Value::Text("data_acquisition_audit".to_owned()),
                                Value::Integer(2)
                            ]]
                        );
                    }
                    _ => assert_eq!(after, before, "TEST_CODE first-v7 immutable table={name}"),
                }
            }
            let cache_time = chrono::DateTime::parse_from_rfc3339("2026-07-21T15:31:00+08:00")
                .unwrap()
                .with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string();
            assert_eq!(
                all_rows(
                    fixture.connection(),
                    "SELECT * FROM chain_post_close_concept_cache_writes"
                ),
                vec![vec![
                    Value::Text(intent.as_str().to_owned()),
                    Value::Text("ConceptProvider".to_owned()),
                    Value::Integer(0),
                    Value::Text("TEST_CODE_600001".to_owned()),
                    Value::Integer(i64::try_from(previous_head).unwrap()),
                    Value::Integer(1),
                    Value::Blob(CACHE.to_vec()),
                    Value::Integer(i64::try_from(CACHE.len()).unwrap()),
                    Value::Text(hex::encode(sha2::Sha256::digest(CACHE))),
                    Value::Text(cache_time.clone()),
                    Value::Text(owner.to_owned()),
                    Value::Integer(2),
                    Value::Integer(i64::try_from(previous_head + 2).unwrap()),
                    Value::Integer(at(5_100)),
                ]]
            );
            assert_eq!(all_rows(fixture.connection(), "SELECT code,concepts,updated_at FROM stock_concepts WHERE code='TEST_CODE_600001'"),
                vec![vec![Value::Text("TEST_CODE_600001".to_owned()), Value::Text(std::str::from_utf8(CACHE).unwrap().to_owned()), Value::Text(cache_time)]]);
            assert_eq!(all_rows(fixture.connection(), "SELECT date,concept,stocks,continuation_count FROM chain_daily WHERE date='2026-07-21' AND concept='TEST_CODE_CLUSTER_A_MAIN'"),
                vec![vec![Value::Text("2026-07-21".to_owned()), Value::Text("TEST_CODE_CLUSTER_A_MAIN".to_owned()),
                    Value::Text(r#"["TEST_CODE_600001","TEST_CODE_CLUSTER_STOCK_A","TEST_CODE_CLUSTER_STOCK_B"]"#.to_owned()), Value::Integer(1)]]);
            completed_rows = Some(current);
            completed_requests = Some(observed);
        } else {
            assert_eq!(head, previous_head + 1);
            let mut expected = completed_rows.as_ref().unwrap().clone();
            set_expected_run(
                &mut expected,
                &run_columns,
                owner,
                generation,
                head,
                until,
                resume_at,
            );
            assert_eq!(
                current, expected,
                "TEST_CODE second-v7: only five takeover fields change"
            );
            assert_eq!(&observed, completed_requests.as_ref().unwrap());
        }
    }
    assert_eq!(provider.0.get(), 1);
    assert!(server.membership_snapshot().is_empty());
    drop(queries);
    drop(source);
    let final_observation = server.finish().await;
    assert_eq!(&final_observation, completed_requests.as_ref().unwrap());
}

#[tokio::test]
async fn migration_v7_rejects_legacy_begin_code_mismatch_without_sealing_or_repair() {
    use rusqlite::types::Value;
    use sha2::Digest as _;

    const ORIGINAL_CODE: &str = "TEST_CODE_600001";
    const DAMAGED_BYTES: &[u8] = b"TEST_CODE_600002";
    const CATALOG_SQL: &str =
        "SELECT name,type,tbl_name,CAST(sql AS BLOB) FROM main.sqlite_schema ORDER BY name";

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
    install_br159_in_owned_database(&mut fixture);
    fixture
        .chain_post_close()
        .migrate_schema_v4_to_v5()
        .unwrap();
    assert_eq!(
        fixture
            .chain_post_close()
            .migrate_schema_v5_to_v6()
            .unwrap()
            .schema_version(),
        6
    );
    assert_eq!(
        fixture
            .chain_post_close()
            .verify_schema()
            .unwrap()
            .schema_version(),
        6
    );

    let (client, server) = tokio::time::timeout(Duration::from_secs(5), spawn_board_loopback())
        .await
        .expect("TEST_CODE migration-admission listener deadline");
    let source = GrpcSource::from_board_loopback_test_client(client);
    let queries = tokio::time::timeout(Duration::from_secs(5), source.connected_board_queries())
        .await
        .expect("TEST_CODE migration-admission preconnect deadline")
        .unwrap();
    let no_requests = server.snapshot();
    assert!(no_requests.requests.is_empty());
    assert_eq!(no_requests.non_board_requests, 0);
    assert!(server.membership_snapshot().is_empty());

    let stocks = vec![stock()];
    let config = local_config(BUILD_A);
    let context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_MIGRATION_BEGIN_ADMISSION"),
    )
    .unwrap();
    let provider = LegacyMembershipErrorProvider {
        calls: Cell::new(0),
    };
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
            lease_request("TEST_CODE_MIGRATION_BEGIN_OWNER", 1_000, 5_000, None),
        )
        .unwrap();
    let intent_id = lease.intent_id().clone();
    let clock = ControlledClock::new(at(1_100));
    let mut io = local
        .board_preparation_io_v6(
            lease,
            &provider,
            &clock,
            FixedClusterConfiguration::resolve(Some("2")),
            &source,
        )
        .unwrap();
    let failed = tokio::time::timeout(
        Duration::from_secs(5),
        prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks,
            None,
            &mut io,
        ),
    )
    .await
    .expect("TEST_CODE migration-admission prepare deadline")
    .expect_err("TEST_CODE real legacy provider error");
    assert_legacy_business_failure(&failed);
    assert_eq!(provider.calls.get(), 1);
    drop(io);
    let healthy = local
        .inspect_run(&intent_id)
        .expect("TEST_CODE healthy v6 public reader");
    assert_eq!(healthy.begins().len(), 1);
    assert_eq!(healthy.results().len(), 1);
    assert_eq!(healthy.begins()[0].ordinal, 0);
    assert_eq!(healthy.begins()[0].code, ORIGINAL_CODE);
    assert_eq!(healthy.results()[0].outcome, "BusinessError");
    assert_eq!(
        healthy.results()[0].bytes,
        LEGACY_MEMBERSHIP_FAILURE.as_bytes()
    );
    assert_eq!(healthy.head_version(), healthy.results()[0].run_version);
    assert_eq!(healthy.lease_generation(), 1);
    drop(healthy);
    drop(local);
    assert_eq!(server.snapshot(), no_requests);
    assert!(server.membership_snapshot().is_empty());

    let original_digest = hex::encode(sha2::Sha256::digest(ORIGINAL_CODE.as_bytes()));
    assert_eq!(
        all_rows(
            fixture.connection(),
            "SELECT effect_key,CAST(request_bytes AS BLOB),request_length,request_sha256 \
             FROM chain_post_close_stage_begins \
             WHERE intent_id=(SELECT intent_id FROM chain_post_close_runs) \
               AND effect_kind='ConceptProvider' AND effect_ordinal=0"
        ),
        vec![vec![
            Value::Text(ORIGINAL_CODE.to_owned()),
            Value::Blob(ORIGINAL_CODE.as_bytes().to_vec()),
            Value::Integer(16),
            Value::Text(original_digest)
        ]]
    );
    assert_eq!(
        fixture.stored_result(&intent_id),
        Some((
            "BusinessError".to_owned(),
            LEGACY_MEMBERSHIP_FAILURE.as_bytes().to_vec()
        ))
    );

    let names = legacy_table_names(fixture.connection());
    let healthy_rows = legacy_table_values(fixture.connection(), &names, 6);
    let catalog = all_rows(fixture.connection(), CATALOG_SQL);
    let begin_columns = all_rows(
        fixture.connection(),
        "PRAGMA main.table_info('chain_post_close_stage_begins')",
    );
    let column = |name: &str| {
        begin_columns
            .iter()
            .position(|entry| entry[1] == Value::Text(name.to_owned()))
            .unwrap()
    };
    let intent_column = column("intent_id");
    let kind_column = column("effect_kind");
    let ordinal_column = column("effect_ordinal");
    let bytes_column = column("request_bytes");
    let digest_column = column("request_sha256");
    let trigger_sql: String = fixture
        .connection()
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type='trigger' \
             AND name='chain_post_close_stage_begins_update'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let damaged_digest = hex::encode(sha2::Sha256::digest(DAMAGED_BYTES));
    assert_eq!(ORIGINAL_CODE.as_bytes().len(), DAMAGED_BYTES.len());
    assert_ne!(ORIGINAL_CODE.as_bytes(), DAMAGED_BYTES);
    assert_ne!(
        fixture
            .connection()
            .query_row(
                "SELECT request_sha256 FROM chain_post_close_stage_begins \
                 WHERE intent_id=?1 AND effect_kind='ConceptProvider' AND effect_ordinal=0",
                [intent_id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        damaged_digest
    );
    fixture.execute("DROP TRIGGER chain_post_close_stage_begins_update;");
    assert_eq!(
        fixture
            .connection()
            .execute(
                "UPDATE chain_post_close_stage_begins \
                 SET request_bytes=?1,request_sha256=?2 \
                 WHERE intent_id=?3 AND effect_kind='ConceptProvider' AND effect_ordinal=0",
                rusqlite::params![DAMAGED_BYTES, &damaged_digest, intent_id.as_str()],
            )
            .unwrap(),
        1
    );
    fixture.execute(&trigger_sql);
    assert_eq!(all_rows(fixture.connection(), CATALOG_SQL), catalog);

    let mut expected_damaged_rows = healthy_rows.clone();
    let begins = expected_damaged_rows
        .get_mut("chain_post_close_stage_begins")
        .unwrap();
    let begin = begins
        .iter_mut()
        .find(|row| {
            row[intent_column] == Value::Text(intent_id.as_str().to_owned())
                && row[kind_column] == Value::Text("ConceptProvider".to_owned())
                && row[ordinal_column] == Value::Integer(0)
        })
        .unwrap();
    begin[bytes_column] = Value::Blob(DAMAGED_BYTES.to_vec());
    begin[digest_column] = Value::Text(damaged_digest);
    let damaged_rows = legacy_table_values(fixture.connection(), &names, 6);
    assert_eq!(damaged_rows, expected_damaged_rows);
    assert_eq!(server.snapshot(), no_requests);
    assert!(server.membership_snapshot().is_empty());

    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let inspected = local
        .inspect_run(&intent_id)
        .map(|run| (run.head_version(), run.begins().len(), run.results().len()));
    drop(local);
    assert_eq!(legacy_table_names(fixture.connection()), names);
    assert_eq!(
        legacy_table_values(fixture.connection(), &names, 6),
        damaged_rows
    );
    assert_eq!(all_rows(fixture.connection(), CATALOG_SQL), catalog);
    assert_eq!(server.snapshot(), no_requests);
    assert!(server.membership_snapshot().is_empty());

    let migrated = fixture
        .chain_post_close()
        .migrate_schema_v6_to_v7()
        .map(|receipt| receipt.schema_version());
    let after_names = legacy_table_names(fixture.connection());
    let after_rows = legacy_table_values(fixture.connection(), &after_names, 7);
    let after_catalog = all_rows(fixture.connection(), CATALOG_SQL);
    let layout: i64 = fixture
        .connection()
        .query_row(
            "SELECT MAX(layout_version) FROM chain_post_close_layouts",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let v7_seals: i64 = fixture
        .connection()
        .query_row(
            "SELECT count(*) FROM chain_post_close_layouts WHERE layout_version=7",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let v7_registry: i64 = fixture
        .connection()
        .query_row(
            "SELECT count(*) FROM chain_post_close_layout_objects WHERE layout_version=7",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let qualification_tables: i64 = fixture
        .connection()
        .query_row(
            "SELECT count(*) FROM sqlite_schema \
             WHERE type='table' \
               AND name='chain_post_close_concept_rpc_legacy_outer_qualifications'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let after_board = server.snapshot();
    let after_memberships = server.membership_snapshot();
    let provider_calls = provider.calls.get();
    drop(queries);
    drop(source);
    let finished = tokio::time::timeout(Duration::from_secs(5), server.finish())
        .await
        .expect("TEST_CODE migration-admission server finish deadline");

    match inspected {
        Err(ChainPostCloseError::SchemaRejected) => {}
        Ok((head, begins, results)) => panic!(
            "TEST_CODE damaged legacy reader unexpectedly succeeded: \
             head={head}, begins={begins}, results={results}"
        ),
        Err(_) => panic!("TEST_CODE damaged legacy reader returned wrong safe error category"),
    }
    match migrated {
        Err(ChainPostCloseError::SchemaRejected) => {
            assert_eq!(after_names, names);
            assert_eq!(after_catalog, catalog);
            assert_eq!(after_rows, damaged_rows);
            assert_eq!(layout, 6);
            assert_eq!(v7_seals, 0);
            assert_eq!(v7_registry, 0);
            assert_eq!(qualification_tables, 0);
            assert_eq!(provider_calls, 1);
            assert_eq!(after_memberships.len(), 0);
            assert_eq!(after_board, no_requests);
            assert_eq!(finished, no_requests);
        }
        Ok(layout) => {
            panic!("TEST_CODE migration admitted damaged legacy begin: layout={layout}")
        }
        Err(_) => panic!("TEST_CODE migration returned wrong safe error category"),
    }
}

#[tokio::test]
async fn legacy_applied_migrates_and_reopens_without_effect_replay() {
    use super::super::super::{ConceptEffectRecoveryState, LocalChainPostClose};
    use crate::database::data_acquisition_audit::DataAcquisitionAuditReceipt;
    use crate::grpc_client::client::board_loopback_fixture::BoardLoopbackObservation;
    use rusqlite::types::Value;
    use sha2::Digest as _;

    const RAW: &str = r#"{"all_boards":["TEST_CODE_CLUSTER_A_MAIN","TEST_CODE_CLUSTER_B_ALIAS"]}"#;
    const CACHE: &[u8] = br#"["TEST_CODE_CLUSTER_A_MAIN","TEST_CODE_CLUSTER_B_ALIAS"]"#;
    const CATALOG_SQL: &str =
        "SELECT name,type,tbl_name,CAST(sql AS BLOB) FROM main.sqlite_schema ORDER BY name";
    const QUALIFICATION_SQL: &str =
        "SELECT * FROM chain_post_close_concept_rpc_legacy_outer_qualifications ORDER BY intent_id,outer_ordinal";

    struct AppliedRaw(Cell<usize>);
    #[async_trait::async_trait(?Send)]
    impl ConceptProviderRawIo for AppliedRaw {
        async fn call_raw(&self, code: &str) -> std::result::Result<String, String> {
            assert_eq!(code, "TEST_CODE_600001");
            assert_eq!(
                self.0.replace(self.0.get() + 1),
                0,
                "TEST_CODE applied Raw is one-shot"
            );
            Ok(RAW.to_owned())
        }
    }

    #[track_caller]
    fn inspect_applied(
        phase: &str,
        error: &anyhow::Error,
        local: &mut LocalChainPostClose<'_>,
        intent: &IntentId,
        stocks: &[TopStock],
        directory: &BTreeMap<String, String>,
        selected: &BTreeMap<String, String>,
        observed: &BoardLoopbackObservation,
    ) -> (u64, u64, u64, Vec<DataAcquisitionAuditReceipt>) {
        eprintln!("TEST_CODE legacy-applied phase={phase} boundary/readers");
        assert_candidates_completed_before_positions(error, directory, selected);
        let batch = local.inspect_concept_batch(intent).unwrap();
        assert!(batch.is_complete());
        let expected = [
            (
                "TEST_CODE_CLUSTER_STOCK_A",
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
                "TEST_CODE_600001",
                vec!["TEST_CODE_CLUSTER_A_MAIN", "TEST_CODE_CLUSTER_B_ALIAS"],
            ),
        ]
        .into_iter()
        .map(|(code, names)| {
            (
                code.to_owned(),
                names.into_iter().map(str::to_owned).collect::<Vec<_>>(),
            )
        })
        .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(batch.concepts(), &expected);
        assert_eq!(batch.applied_codes(), ["TEST_CODE_600001"]);
        assert_eq!(batch.effects().len(), 1);
        assert_eq!(
            (
                batch.effects()[0].ordinal(),
                batch.effects()[0].code(),
                batch.effects()[0].state()
            ),
            (0, "TEST_CODE_600001", ConceptEffectRecoveryState::Confirmed)
        );
        let material = local.inspect_cluster_application(intent).unwrap();
        assert_eq!(material.min_cluster_size(), 2);
        assert_eq!(material.clusters().len(), 1);
        let cluster = &material.clusters()[0];
        assert_eq!(cluster.concept, "TEST_CODE_CLUSTER_A_MAIN");
        assert_eq!(cluster.aliases, ["TEST_CODE_CLUSTER_B_ALIAS"]);
        assert_eq!(
            serde_json::to_value(&cluster.stocks).unwrap(),
            serde_json::to_value([&stocks[3], &stocks[0], &stocks[1]]).unwrap()
        );
        assert_eq!(
            serde_json::to_value(material.isolated()).unwrap(),
            serde_json::to_value([&stocks[2]]).unwrap()
        );
        assert_eq!((cluster.continuation_count, cluster.streak_days), (1, 3));
        assert!(
            cluster.candidates.is_empty() && cluster.score.is_none() && cluster.scenario.is_none()
        );
        assert_eq!(
            material.lifecycle_days(),
            &BTreeMap::from([("TEST_CODE_CLUSTER_A_MAIN".to_owned(), 3)])
        );
        let board = local.inspect_board_directory(intent).unwrap();
        assert_eq!(board.board_directory(), directory);
        assert_eq!(board.selected_board_codes(), selected);
        assert_eq!(observed.requests.len(), 3);
        assert_eq!(observed.non_board_requests, 0);
        assert_eq!(
            observed
                .requests
                .iter()
                .map(|r| r.kind.as_str())
                .collect::<Vec<_>>(),
            ["Industry", "Industry", "Concept"]
        );
        assert!(observed.requests.iter().all(|r| r.authorized
            && r.limit == 10_000
            && r.protocol_version == 1
            && r.payload_schema == "board.directory"
            && r.payload_schema_version == 1
            && r.payload_content_type == "application/json; charset=utf-8"
            && !r.allow_unadmitted
            && !r.request_id.is_empty()));
        assert_eq!(
            observed.requests[0].request_id,
            observed.requests[1].request_id
        );
        assert_ne!(
            observed.requests[1].request_id,
            observed.requests[2].request_id
        );
        assert_eq!(board.attempts().len(), 3);
        for (index, (kind, ordinal)) in [
            (BoardKind::Industry, 1),
            (BoardKind::Industry, 2),
            (BoardKind::Concept, 1),
        ]
        .into_iter()
        .enumerate()
        {
            let attempt = &board.attempts()[index];
            assert!(attempt.is_confirmed());
            assert_eq!(attempt.kind(), kind);
            assert_eq!(attempt.attempt_ordinal(), ordinal);
            assert_eq!(attempt.request_id(), observed.requests[index].request_id);
        }
        assert!(board.attempts()[0].payload_bytes().is_none());
        assert_eq!(
            board.attempts()[0].error_detail_bytes(),
            Some(observed.retry_error_detail.as_slice())
        );
        assert_eq!(
            board.attempts()[1].payload_bytes(),
            Some(INDUSTRY_DIRECTORY_BYTES)
        );
        assert_eq!(
            board.attempts()[2].payload_bytes(),
            Some(CONCEPT_DIRECTORY_BYTES)
        );
        assert_eq!(board.directories().len(), 2);
        assert_eq!(board.directories()[0].kind(), BoardKind::Industry);
        assert_eq!(board.directories()[1].kind(), BoardKind::Concept);
        let receipts = board
            .directories()
            .iter()
            .map(|d| d.receipt().clone())
            .collect();
        let run = local.inspect_run(intent).unwrap();
        assert_eq!(run.begins().len(), 1);
        assert_eq!(run.results().len(), 1);
        assert_eq!(run.begins()[0].code, "TEST_CODE_600001");
        assert_eq!(run.results()[0].outcome, "Returned");
        assert!(
            run.results()[0].bytes == RAW.as_bytes(),
            "TEST_CODE {phase}: original Raw bytes"
        );
        let begin = run.begins()[0].run_version;
        let result = run.results()[0].run_version;
        assert_eq!(begin + 1, result);
        (run.head_version(), begin, result, receipts)
    }

    #[track_caller]
    fn verify_applied_audits(
        phase: &str,
        connection: &Connection,
        receipts: &[DataAcquisitionAuditReceipt],
    ) {
        eprintln!("TEST_CODE legacy-applied phase={phase} complete-audit-reader");
        assert_eq!(receipts.len(), 2);
        let transaction = connection.unchecked_transaction().unwrap();
        for (index, receipt) in receipts.iter().enumerate() {
            assert_eq!(receipt.audit_id, i64::try_from(index + 1).unwrap());
            assert_eq!(
                receipt.previous_outcome.as_deref(),
                if index == 0 { None } else { Some("available") }
            );
            assert_eq!(receipt.current_outcome, "available");
            let verified = read_acquisition_in_transaction(&transaction, receipt).unwrap();
            assert_eq!(verified.receipt(), receipt);
            let record = verified.record();
            assert_eq!(
                (
                    record.capability,
                    record.provider,
                    record.source,
                    record.request_hash,
                    record.source_at,
                    record.observed_at,
                    record.batch_id
                ),
                (
                    "board-directory",
                    "Tdx",
                    "TEST_CODE_LOOPBACK_BOARD_SOURCE",
                    BOARD_REQUEST_HASHES[index],
                    Some("2026-07-21T15:30:00+08:00"),
                    "2026-07-21T15:31:00+08:00",
                    Some(BOARD_BATCH_IDS[index])
                )
            );
            assert_eq!(
                (
                    record.outcome,
                    record.request_count,
                    record.accepted_count,
                    record.rejected_count,
                    record.reason_code,
                    record.retryable
                ),
                ("available", 1, 2, 0, "accepted", false)
            );
        }
        transaction.rollback().unwrap();
    }

    fn assert_no_inner_rpc(connection: &Connection) {
        for table in [
            "chain_post_close_concept_rpc_occurrences",
            "chain_post_close_concept_rpc_attempt_begins",
            "chain_post_close_concept_rpc_attempt_results",
            "chain_post_close_concept_rpc_status_materials",
            "chain_post_close_concept_rpc_error_materials",
            "chain_post_close_concept_rpc_finals",
        ] {
            assert_eq!(
                count(connection, table),
                0,
                "TEST_CODE legacy applied table={table}"
            );
        }
    }

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
    install_br159_in_owned_database(&mut fixture);
    fixture
        .chain_post_close()
        .migrate_schema_v4_to_v5()
        .unwrap();
    assert_eq!(
        fixture
            .chain_post_close()
            .migrate_schema_v5_to_v6()
            .unwrap()
            .schema_version(),
        6
    );
    let (client, server) = tokio::time::timeout(Duration::from_secs(5), spawn_board_loopback())
        .await
        .expect("TEST_CODE applied listener/connect deadline");
    let source = GrpcSource::from_board_loopback_test_client(client);
    let queries = tokio::time::timeout(Duration::from_secs(5), source.connected_board_queries())
        .await
        .expect("TEST_CODE applied preconnect deadline")
        .unwrap();
    let mut stocks = cluster_tests::cluster_stocks();
    stocks.push(stock());
    let directory = BTreeMap::from([
        (
            "TEST_CODE_CLUSTER_A_MAIN".to_owned(),
            "TEST_CODE_BOARD_MAIN".to_owned(),
        ),
        (
            "TEST_CODE_CONCEPT_ONLY".to_owned(),
            "TEST_CODE_BOARD_CONCEPT_ONLY".to_owned(),
        ),
        (
            "TEST_CODE_INDUSTRY_ONLY".to_owned(),
            "TEST_CODE_BOARD_INDUSTRY_ONLY".to_owned(),
        ),
    ]);
    let selected = BTreeMap::from([(
        "TEST_CODE_CLUSTER_A_MAIN".to_owned(),
        "TEST_CODE_BOARD_MAIN".to_owned(),
    )]);
    let config = local_config(BUILD_A);
    let context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_LEGACY_APPLIED"),
    )
    .unwrap();
    let provider = AppliedRaw(Cell::new(0));
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
            lease_request("TEST_CODE_APPLIED_OWNER_A", 1_000, 5_000, None),
        )
        .unwrap();
    let intent = lease.intent_id().clone();
    let clock = ControlledClock::new(at(1_100));
    let mut io = local
        .board_preparation_io_v6(
            lease,
            &provider,
            &clock,
            FixedClusterConfiguration::resolve(Some("2")),
            &source,
        )
        .unwrap();
    assert_eq!(
        io.local
            .store
            .connection
            .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        250
    );
    let first = tokio::time::timeout(
        Duration::from_secs(15),
        prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks.clone(),
            None,
            &mut io,
        ),
    )
    .await
    .expect("TEST_CODE first v6 applied prepare deadline")
    .expect_err("TEST_CODE v6 Positions boundary");
    drop(io);
    let requests = server.snapshot();
    let (head, begin_version, result_version, receipts) = inspect_applied(
        "v6-first", &first, &mut local, &intent, &stocks, &directory, &selected, &requests,
    );
    drop(local);
    assert_eq!(provider.0.get(), 1);
    assert!(server.membership_snapshot().is_empty());
    assert_eq!(
        fixture.stored_result(&intent),
        Some(("Returned".to_owned(), RAW.as_bytes().to_vec()))
    );
    verify_applied_audits("v6-first", fixture.connection(), &receipts);

    // Independent full cache14 oracle, before deriving the five Applied qualification fields.
    let cache_time = chrono::DateTime::parse_from_rfc3339("2026-07-21T15:31:00+08:00")
        .unwrap()
        .with_timezone(&chrono::Local)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    let cache_sha = hex::encode(sha2::Sha256::digest(CACHE));
    let expected_cache = vec![
        Value::Text(intent.as_str().to_owned()),
        Value::Text("ConceptProvider".to_owned()),
        Value::Integer(0),
        Value::Text("TEST_CODE_600001".to_owned()),
        Value::Integer(i64::try_from(result_version).unwrap()),
        Value::Integer(1),
        Value::Blob(CACHE.to_vec()),
        Value::Integer(i64::try_from(CACHE.len()).unwrap()),
        Value::Text(cache_sha),
        Value::Text(cache_time.clone()),
        Value::Text("TEST_CODE_APPLIED_OWNER_A".to_owned()),
        Value::Integer(1),
        Value::Integer(i64::try_from(result_version + 1).unwrap()),
        Value::Integer(at(1_100)),
    ];
    assert_eq!(
        all_rows(
            fixture.connection(),
            "SELECT * FROM chain_post_close_concept_cache_writes"
        ),
        vec![expected_cache.clone()]
    );
    assert_eq!(
        all_rows(
            fixture.connection(),
            "SELECT code,concepts,updated_at FROM stock_concepts WHERE code='TEST_CODE_600001'"
        ),
        vec![vec![
            Value::Text("TEST_CODE_600001".to_owned()),
            Value::Text(std::str::from_utf8(CACHE).unwrap().to_owned()),
            Value::Text(cache_time)
        ]]
    );
    assert_eq!(all_rows(fixture.connection(), "SELECT date,concept,stocks,continuation_count FROM chain_daily WHERE date='2026-07-21' AND concept='TEST_CODE_CLUSTER_A_MAIN'"),
        vec![vec![Value::Text("2026-07-21".to_owned()), Value::Text("TEST_CODE_CLUSTER_A_MAIN".to_owned()),
            Value::Text(r#"["TEST_CODE_600001","TEST_CODE_CLUSTER_STOCK_A","TEST_CODE_CLUSTER_STOCK_B"]"#.to_owned()), Value::Integer(1)]]);
    let old_names = legacy_table_names(fixture.connection());
    let old_rows = legacy_table_values(fixture.connection(), &old_names, 6);
    let old_catalog = all_rows(fixture.connection(), CATALOG_SQL);
    let qualification = vec![vec![
        Value::Text(intent.as_str().to_owned()),
        Value::Text("ConceptProvider".to_owned()),
        Value::Integer(0),
        Value::Text("TEST_CODE_600001".to_owned()),
        Value::Text("LegacyReturnedApplied".to_owned()),
        Value::Integer(i64::try_from(begin_version).unwrap()),
        Value::Text(hex::encode(sha2::Sha256::digest(b"TEST_CODE_600001"))),
        Value::Text("TEST_CODE_APPLIED_OWNER_A".to_owned()),
        Value::Integer(1),
        Value::Integer(at(1_100)),
        Value::Text("Returned".to_owned()),
        Value::Integer(i64::try_from(result_version).unwrap()),
        Value::Text(hex::encode(sha2::Sha256::digest(RAW.as_bytes()))),
        Value::Text("TEST_CODE_APPLIED_OWNER_A".to_owned()),
        Value::Integer(1),
        Value::Integer(at(1_100)),
        expected_cache[12].clone(),
        expected_cache[8].clone(),
        expected_cache[10].clone(),
        expected_cache[11].clone(),
        expected_cache[13].clone(),
        Value::Integer(6),
        Value::Integer(7),
    ]];
    assert_eq!(qualification[0].len(), 23);
    assert!(qualification[0][16..21]
        .iter()
        .all(|value| value != &Value::Null));
    assert_eq!(
        fixture
            .chain_post_close()
            .migrate_schema_v6_to_v7()
            .unwrap()
            .schema_version(),
        7
    );
    assert_eq!(
        fixture
            .chain_post_close()
            .verify_schema()
            .unwrap()
            .schema_version(),
        7
    );
    assert_eq!(
        legacy_table_values(fixture.connection(), &old_names, 6),
        old_rows
    );
    let catalog = all_rows(fixture.connection(), CATALOG_SQL);
    assert!(old_catalog.iter().all(|row| catalog.contains(row)));
    let registry = all_rows(fixture.connection(),
        "SELECT name,object_type,CAST(definition AS BLOB) FROM chain_post_close_layout_objects WHERE layout_version=7 ORDER BY name");
    assert_eq!(registry.len(), 99);
    assert_eq!(
        registry,
        catalog
            .iter()
            .filter(
                |row| matches!(&row[0], Value::Text(name) if name.starts_with("chain_post_close_"))
                    && row[3] != Value::Null
            )
            .map(|row| vec![row[0].clone(), row[1].clone(), row[3].clone()])
            .collect::<Vec<_>>()
    );
    assert_eq!(
        all_rows(fixture.connection(), QUALIFICATION_SQL),
        qualification
    );
    assert_no_inner_rpc(fixture.connection());
    let rpc = concept_rpc_facts(fixture.connection());
    assert_eq!(
        (
            rpc.configurations.len(),
            rpc.outer_begins,
            rpc.outer_results,
            rpc.cache_writes,
            rpc.cached_concepts,
            rpc.cluster_materials,
            rpc.chain_daily_applications,
            rpc.audits,
            rpc.audit_chain
        ),
        (1, 1, 1, 1, 1, 1, 1, 2, 2)
    );
    let names = legacy_table_names(fixture.connection());
    let mut expected_reopened = legacy_table_values(fixture.connection(), &names, 7);
    let columns = all_rows(
        fixture.connection(),
        "PRAGMA main.table_info('chain_post_close_runs')",
    );
    fixture.reopen();
    assert_eq!(
        legacy_table_values(fixture.connection(), &names, 7),
        expected_reopened,
        "TEST_CODE true reopen preserves all migrated facts before takeover"
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
            lease_request("TEST_CODE_APPLIED_OWNER_B", 5_001, 9_000, Some(head)),
        )
        .unwrap();
    let clock = ControlledClock::new(at(5_100));
    let mut io = local
        .concept_rpc_preparation_io_v7(
            lease,
            &queries,
            &clock,
            FixedClusterConfiguration::resolve(Some("2")),
        )
        .unwrap();
    let replayed = tokio::time::timeout(
        Duration::from_secs(5),
        prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks.clone(),
            None,
            &mut io,
        ),
    )
    .await
    .expect("TEST_CODE v7 applied replay deadline")
    .expect_err("TEST_CODE v7 Positions boundary");
    drop(io);
    let replay_requests = server.snapshot();
    let (reopened_head, reopened_begin, reopened_result, reopened_receipts) = inspect_applied(
        "v7-reopened",
        &replayed,
        &mut local,
        &intent,
        &stocks,
        &directory,
        &selected,
        &replay_requests,
    );
    assert_eq!(
        (reopened_head, reopened_begin, reopened_result),
        (head + 1, begin_version, result_version)
    );
    assert_eq!(local.inspect_run(&intent).unwrap().lease_generation(), 2);
    drop(local);
    assert_eq!(reopened_receipts, receipts);
    verify_applied_audits("v7-reopened", fixture.connection(), &reopened_receipts);
    let run = expected_reopened.get_mut("chain_post_close_runs").unwrap();
    assert_eq!(run.len(), 1);
    for (name, value) in [
        (
            "lease_owner",
            Value::Text("TEST_CODE_APPLIED_OWNER_B".to_owned()),
        ),
        ("lease_generation", Value::Integer(2)),
        (
            "head_version",
            Value::Integer(i64::try_from(head + 1).unwrap()),
        ),
        ("lease_until", Value::Integer(at(9_000))),
        ("updated_at", Value::Integer(at(5_001))),
    ] {
        let ordinal = columns
            .iter()
            .position(|column| column[1] == Value::Text(name.to_owned()))
            .unwrap();
        run[0][ordinal] = value;
    }
    assert_eq!(legacy_table_names(fixture.connection()), names);
    assert_eq!(
        legacy_table_values(fixture.connection(), &names, 7),
        expected_reopened
    );
    assert_eq!(all_rows(fixture.connection(), CATALOG_SQL), catalog);
    assert_eq!(
        all_rows(fixture.connection(), QUALIFICATION_SQL),
        qualification
    );
    assert_no_inner_rpc(fixture.connection());
    assert_eq!(concept_rpc_facts(fixture.connection()), rpc);
    assert_eq!(provider.0.get(), 1);
    assert_eq!(replay_requests, requests);
    assert!(server.membership_snapshot().is_empty());
    drop(queries);
    drop(source);
    assert_eq!(server.finish().await, requests);
}

#[tokio::test]
async fn migration_and_reader_reject_legacy_selection_outside_saved_directory() {
    use rusqlite::types::Value;
    use sha2::Digest as _;

    const RAW: &str = r#"{"all_boards":["TEST_CODE_CLUSTER_A_MAIN","TEST_CODE_CLUSTER_B_ALIAS"]}"#;
    const CATALOG_SQL: &str =
        "SELECT name,type,tbl_name,CAST(sql AS BLOB) FROM main.sqlite_schema ORDER BY name";
    const ORIGINAL_CODE: &str = "TEST_CODE_BOARD_MAIN";
    const DAMAGED_CODE: &str = "TEST_CODE_BOARD_FAKE";
    const ORIGINAL_SELECTION: &[u8] = br#"{"schema_version":1,"cluster_ordinal":0,"cluster_concept":"TEST_CODE_CLUSTER_A_MAIN","selected_code":"TEST_CODE_BOARD_MAIN"}"#;
    const DAMAGED_SELECTION: &[u8] = br#"{"schema_version":1,"cluster_ordinal":0,"cluster_concept":"TEST_CODE_CLUSTER_A_MAIN","selected_code":"TEST_CODE_BOARD_FAKE"}"#;

    struct SelectionParentRaw(Cell<usize>);

    #[async_trait::async_trait(?Send)]
    impl ConceptProviderRawIo for SelectionParentRaw {
        async fn call_raw(&self, code: &str) -> std::result::Result<String, String> {
            assert_eq!(code, "TEST_CODE_600001");
            assert_eq!(
                self.0.replace(self.0.get() + 1),
                0,
                "TEST_CODE selection-parent Raw is one-shot"
            );
            Ok(RAW.to_owned())
        }
    }

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
    install_br159_in_owned_database(&mut fixture);
    fixture
        .chain_post_close()
        .migrate_schema_v4_to_v5()
        .unwrap();
    assert_eq!(
        fixture
            .chain_post_close()
            .migrate_schema_v5_to_v6()
            .unwrap()
            .schema_version(),
        6
    );

    let (client, server) = tokio::time::timeout(Duration::from_secs(5), spawn_board_loopback())
        .await
        .expect("TEST_CODE selection-parent listener/connect deadline");
    let source = GrpcSource::from_board_loopback_test_client(client);
    let mut stocks = cluster_tests::cluster_stocks();
    stocks.push(stock());
    let expected_directory = BTreeMap::from([
        (
            "TEST_CODE_CLUSTER_A_MAIN".to_owned(),
            ORIGINAL_CODE.to_owned(),
        ),
        (
            "TEST_CODE_CONCEPT_ONLY".to_owned(),
            "TEST_CODE_BOARD_CONCEPT_ONLY".to_owned(),
        ),
        (
            "TEST_CODE_INDUSTRY_ONLY".to_owned(),
            "TEST_CODE_BOARD_INDUSTRY_ONLY".to_owned(),
        ),
    ]);
    let expected_selected = BTreeMap::from([(
        "TEST_CODE_CLUSTER_A_MAIN".to_owned(),
        ORIGINAL_CODE.to_owned(),
    )]);
    assert!(!expected_directory.values().any(|code| code == DAMAGED_CODE));
    assert!(!INDUSTRY_DIRECTORY_BYTES
        .windows(DAMAGED_CODE.len())
        .any(|window| window == DAMAGED_CODE.as_bytes()));
    assert!(!CONCEPT_DIRECTORY_BYTES
        .windows(DAMAGED_CODE.len())
        .any(|window| window == DAMAGED_CODE.as_bytes()));

    let config = local_config(BUILD_A);
    let context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_SELECTION_PARENT"),
    )
    .unwrap();
    let provider = SelectionParentRaw(Cell::new(0));
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
            lease_request("TEST_CODE_SELECTION_PARENT_OWNER", 1_000, 5_000, None),
        )
        .unwrap();
    let intent = lease.intent_id().clone();
    let clock = ControlledClock::new(at(1_100));
    let mut io = local
        .board_preparation_io_v6(
            lease,
            &provider,
            &clock,
            FixedClusterConfiguration::resolve(Some("2")),
            &source,
        )
        .unwrap();
    assert_eq!(
        io.local
            .store
            .connection
            .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        250
    );
    let stopped = tokio::time::timeout(
        Duration::from_secs(15),
        prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks,
            None,
            &mut io,
        ),
    )
    .await
    .expect("TEST_CODE selection-parent prepare deadline")
    .expect_err("TEST_CODE v6 preparation stops at Positions");
    assert_candidates_completed_before_positions(&stopped, &expected_directory, &expected_selected);
    drop(io);

    let healthy_observation = server.snapshot();
    assert_eq!(healthy_observation.requests.len(), 3);
    assert_eq!(healthy_observation.non_board_requests, 0);
    assert!(server.membership_snapshot().is_empty());
    assert_eq!(provider.0.get(), 1);
    let healthy_board = local
        .inspect_board_directory(&intent)
        .expect("TEST_CODE healthy v6 public board reader");
    assert_eq!(healthy_board.board_directory(), &expected_directory);
    assert_eq!(healthy_board.selected_board_codes(), &expected_selected);
    assert_eq!(healthy_board.attempts().len(), 3);
    for (index, (kind, ordinal)) in [
        (BoardKind::Industry, 1),
        (BoardKind::Industry, 2),
        (BoardKind::Concept, 1),
    ]
    .into_iter()
    .enumerate()
    {
        assert!(healthy_board.attempts()[index].is_confirmed());
        assert_eq!(healthy_board.attempts()[index].kind(), kind);
        assert_eq!(healthy_board.attempts()[index].attempt_ordinal(), ordinal);
    }
    assert!(healthy_board.attempts()[0].payload_bytes().is_none());
    assert_eq!(
        healthy_board.attempts()[1].payload_bytes(),
        Some(INDUSTRY_DIRECTORY_BYTES)
    );
    assert_eq!(
        healthy_board.attempts()[2].payload_bytes(),
        Some(CONCEPT_DIRECTORY_BYTES)
    );
    assert_eq!(healthy_board.directories().len(), 2);
    for (index, kind) in [BoardKind::Industry, BoardKind::Concept]
        .into_iter()
        .enumerate()
    {
        assert_eq!(healthy_board.directories()[index].kind(), kind);
        assert_eq!(
            healthy_board.directories()[index].receipt().audit_id,
            i64::try_from(index + 1).unwrap()
        );
        assert_eq!(
            healthy_board.directories()[index].receipt().current_outcome,
            "available"
        );
    }
    drop(healthy_board);
    drop(local);

    assert_eq!(ORIGINAL_CODE.len(), DAMAGED_CODE.len());
    assert_eq!(ORIGINAL_SELECTION.len(), DAMAGED_SELECTION.len());
    assert_eq!(
        ORIGINAL_SELECTION
            .windows(ORIGINAL_CODE.len())
            .filter(|window| *window == ORIGINAL_CODE.as_bytes())
            .count(),
        1
    );
    let replaced =
        std::str::from_utf8(ORIGINAL_SELECTION)
            .unwrap()
            .replacen(ORIGINAL_CODE, DAMAGED_CODE, 1);
    assert_eq!(replaced.as_bytes(), DAMAGED_SELECTION);
    let original_sha = hex::encode(sha2::Sha256::digest(ORIGINAL_SELECTION));
    let damaged_sha = hex::encode(sha2::Sha256::digest(DAMAGED_SELECTION));
    assert_ne!(original_sha, damaged_sha);

    let healthy_selection = all_rows(
        fixture.connection(),
        "SELECT selection_outcome,selected_code,CAST(selection_bytes AS BLOB), \
                selection_length,selection_sha256 FROM chain_post_close_board_selections \
         WHERE intent_id=(SELECT intent_id FROM chain_post_close_runs) AND cluster_ordinal=0",
    );
    assert_eq!(
        healthy_selection,
        vec![vec![
            Value::Text("Selected".to_owned()),
            Value::Text(ORIGINAL_CODE.to_owned()),
            Value::Blob(ORIGINAL_SELECTION.to_vec()),
            Value::Integer(i64::try_from(ORIGINAL_SELECTION.len()).unwrap()),
            Value::Text(original_sha),
        ]]
    );
    let old_names = legacy_table_names(fixture.connection());
    let healthy_rows = legacy_table_values(fixture.connection(), &old_names, 6);
    let healthy_catalog = all_rows(fixture.connection(), CATALOG_SQL);
    let selection_columns = all_rows(
        fixture.connection(),
        "PRAGMA main.table_info('chain_post_close_board_selections')",
    );
    let column = |name: &str| {
        selection_columns
            .iter()
            .position(|entry| entry[1] == Value::Text(name.to_owned()))
            .unwrap()
    };
    let selected_code_column = column("selected_code");
    let selection_bytes_column = column("selection_bytes");
    let selection_sha_column = column("selection_sha256");
    let trigger_sql: String = fixture
        .connection()
        .query_row(
            "SELECT sql FROM main.sqlite_schema WHERE type='trigger' \
             AND name='chain_post_close_board_selections_update'",
            [],
            |row| row.get(0),
        )
        .unwrap();

    fixture.execute("DROP TRIGGER chain_post_close_board_selections_update;");
    assert_eq!(
        fixture
            .connection()
            .execute(
                "UPDATE chain_post_close_board_selections \
                 SET selected_code=?1,selection_bytes=?2,selection_sha256=?3 \
                 WHERE intent_id=?4 AND cluster_ordinal=0",
                rusqlite::params![
                    DAMAGED_CODE,
                    DAMAGED_SELECTION,
                    &damaged_sha,
                    intent.as_str()
                ],
            )
            .unwrap(),
        1
    );
    fixture.execute(&trigger_sql);
    assert_eq!(
        fixture
            .connection()
            .query_row(
                "SELECT sql FROM main.sqlite_schema WHERE type='trigger' \
                 AND name='chain_post_close_board_selections_update'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        trigger_sql
    );
    assert_eq!(all_rows(fixture.connection(), CATALOG_SQL), healthy_catalog);

    let mut expected_damaged_rows = healthy_rows.clone();
    let selection_rows = expected_damaged_rows
        .get_mut("chain_post_close_board_selections")
        .unwrap();
    assert_eq!(selection_rows.len(), 1);
    selection_rows[0][selected_code_column] = Value::Text(DAMAGED_CODE.to_owned());
    selection_rows[0][selection_bytes_column] = Value::Blob(DAMAGED_SELECTION.to_vec());
    selection_rows[0][selection_sha_column] = Value::Text(damaged_sha);
    let damaged_rows = legacy_table_values(fixture.connection(), &old_names, 6);
    assert_eq!(damaged_rows, expected_damaged_rows);
    assert_eq!(provider.0.get(), 1);
    assert_eq!(server.snapshot(), healthy_observation);
    assert!(server.membership_snapshot().is_empty());

    let reader = {
        let mut local = fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .unwrap();
        let result = local.inspect_board_directory(&intent).map(|board| {
            (
                board.board_directory().clone(),
                board.selected_board_codes().clone(),
            )
        });
        drop(local);
        result
    };
    if let Ok((directory, selected)) = &reader {
        assert_eq!(directory, &expected_directory);
        assert_eq!(
            selected.get("TEST_CODE_CLUSTER_A_MAIN").map(String::as_str),
            Some(DAMAGED_CODE)
        );
    }
    assert_eq!(
        legacy_table_values(fixture.connection(), &old_names, 6),
        damaged_rows
    );
    assert_eq!(all_rows(fixture.connection(), CATALOG_SQL), healthy_catalog);
    assert_eq!(provider.0.get(), 1);
    assert_eq!(server.snapshot(), healthy_observation);
    assert!(server.membership_snapshot().is_empty());

    let migration = fixture
        .chain_post_close()
        .migrate_schema_v6_to_v7()
        .map(|receipt| receipt.schema_version());
    let after_names = legacy_table_names(fixture.connection());
    let after_rows = legacy_table_values(fixture.connection(), &after_names, 7);
    let after_catalog = all_rows(fixture.connection(), CATALOG_SQL);
    let after_original_rows = legacy_table_values(fixture.connection(), &old_names, 6);
    let after_selection = all_rows(
        fixture.connection(),
        "SELECT selection_outcome,selected_code,CAST(selection_bytes AS BLOB), \
                selection_length,selection_sha256 FROM chain_post_close_board_selections \
         WHERE intent_id=(SELECT intent_id FROM chain_post_close_runs) AND cluster_ordinal=0",
    );
    let layout: i64 = fixture
        .connection()
        .query_row(
            "SELECT MAX(layout_version) FROM chain_post_close_layouts",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let v7_seals: i64 = fixture
        .connection()
        .query_row(
            "SELECT count(*) FROM chain_post_close_layouts WHERE layout_version=7",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let v7_registry: i64 = fixture
        .connection()
        .query_row(
            "SELECT count(*) FROM chain_post_close_layout_objects WHERE layout_version=7",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let qualification_exists: bool = fixture
        .connection()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM main.sqlite_schema WHERE type='table' \
             AND name='chain_post_close_concept_rpc_legacy_outer_qualifications')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let qualification_rows = if qualification_exists {
        fixture
            .connection()
            .query_row(
                "SELECT count(*) FROM chain_post_close_concept_rpc_legacy_outer_qualifications",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
    } else {
        0
    };
    assert_eq!(after_original_rows, damaged_rows);
    assert_eq!(
        after_selection,
        expected_damaged_rows["chain_post_close_board_selections"]
            .iter()
            .map(|row| {
                vec![
                    row[column("selection_outcome")].clone(),
                    row[selected_code_column].clone(),
                    row[selection_bytes_column].clone(),
                    row[column("selection_length")].clone(),
                    row[selection_sha_column].clone(),
                ]
            })
            .collect::<Vec<_>>()
    );
    assert_eq!(
        fixture
            .connection()
            .query_row(
                "SELECT sql FROM main.sqlite_schema WHERE type='trigger' \
                 AND name='chain_post_close_board_selections_update'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        trigger_sql
    );
    assert_eq!(provider.0.get(), 1);
    assert_eq!(server.snapshot(), healthy_observation);
    assert!(server.membership_snapshot().is_empty());
    drop(source);
    let finished = server.finish().await;
    assert_eq!(finished, healthy_observation);

    let reader_rejected = matches!(&reader, Err(ChainPostCloseError::SchemaRejected));
    let migration_rejected = matches!(&migration, Err(ChainPostCloseError::SchemaRejected));
    if reader_rejected && migration_rejected {
        assert_eq!(after_names, old_names);
        assert_eq!(after_rows, expected_damaged_rows);
        assert_eq!(after_catalog, healthy_catalog);
        assert_eq!(layout, 6);
        assert_eq!(v7_seals, 0);
        assert_eq!(v7_registry, 0);
        assert!(!qualification_exists);
        assert_eq!(qualification_rows, 0);
    }
    let reader_state = match &reader {
        Err(ChainPostCloseError::SchemaRejected) => "SchemaRejected".to_owned(),
        Err(error) => format!("wrong-error:{error}"),
        Ok((_, selected)) => format!(
            "accepted:{}",
            selected
                .get("TEST_CODE_CLUSTER_A_MAIN")
                .map(String::as_str)
                .unwrap_or("missing")
        ),
    };
    let migration_state = match &migration {
        Err(ChainPostCloseError::SchemaRejected) => "SchemaRejected".to_owned(),
        Err(error) => format!("wrong-error:{error}"),
        Ok(version) => format!("accepted-layout:{version}"),
    };
    assert!(
        reader_rejected && migration_rejected,
        "TEST_CODE selection parent mismatch must reject: reader={reader_state}; migration={migration_state}; layout={layout}; v7_seals={v7_seals}; v7_registry={v7_registry}; qualification_rows={qualification_rows}"
    );
}

#[tokio::test]
async fn legacy_unconfirmed_migrates_and_reopens_without_membership_replay() {
    use super::super::super::{ConceptEffectRecoveryState, LocalChainPostClose};
    use super::super::PendingRawProvider;
    use rusqlite::types::Value;
    use sha2::Digest as _;
    use std::future::Future as _;
    use std::task::Poll;

    const CATALOG_SQL: &str =
        "SELECT name,type,tbl_name,CAST(sql AS BLOB) FROM main.sqlite_schema ORDER BY name";
    const QUALIFICATION_SQL: &str =
        "SELECT * FROM chain_post_close_concept_rpc_legacy_outer_qualifications ORDER BY intent_id,outer_ordinal";

    #[track_caller]
    fn inspect_unconfirmed(
        phase: &str,
        local: &mut LocalChainPostClose<'_>,
        intent: &IntentId,
        generation: u64,
    ) -> (u64, u64) {
        eprintln!("TEST_CODE legacy-unconfirmed phase={phase} persisted-readers");
        let batch = local.inspect_concept_batch(intent).unwrap();
        assert!(!batch.is_complete());
        let expected = std::collections::HashMap::from([
            (
                "TEST_CODE_CLUSTER_STOCK_A".to_owned(),
                vec![
                    "TEST_CODE_CLUSTER_A_MAIN".to_owned(),
                    "TEST_CODE_CLUSTER_B_ALIAS".to_owned(),
                    "昨日涨停".to_owned(),
                ],
            ),
            (
                "TEST_CODE_CLUSTER_STOCK_B".to_owned(),
                vec![
                    "TEST_CODE_CLUSTER_A_MAIN".to_owned(),
                    "TEST_CODE_CLUSTER_B_ALIAS".to_owned(),
                ],
            ),
            (
                "TEST_CODE_CLUSTER_STOCK_ISOLATED".to_owned(),
                vec!["TEST_CODE_CLUSTER_Z_ISOLATED".to_owned()],
            ),
        ]);
        assert_eq!(batch.concepts(), &expected);
        assert!(!batch.concepts().contains_key("TEST_CODE_600001"));
        assert!(batch.applied_codes().is_empty());
        assert_eq!(batch.effects().len(), 1);
        assert_eq!(
            (
                batch.effects()[0].ordinal(),
                batch.effects()[0].code(),
                batch.effects()[0].state()
            ),
            (
                0,
                "TEST_CODE_600001",
                ConceptEffectRecoveryState::BegunUnconfirmed
            )
        );
        let run = local.inspect_run(intent).unwrap();
        assert_eq!(run.lease_generation(), generation);
        assert_eq!(run.begins().len(), 1);
        assert!(run.results().is_empty());
        let begin = &run.begins()[0];
        assert_eq!(
            (
                begin.ordinal,
                begin.code.as_str(),
                begin.owner.as_str(),
                begin.generation,
                begin.begun_at
            ),
            (
                0,
                "TEST_CODE_600001",
                "TEST_CODE_UNCONFIRMED_OWNER_A",
                1,
                at(1_100)
            )
        );
        (run.head_version(), begin.run_version)
    }

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
    install_br159_in_owned_database(&mut fixture);
    fixture
        .chain_post_close()
        .migrate_schema_v4_to_v5()
        .unwrap();
    assert_eq!(
        fixture
            .chain_post_close()
            .migrate_schema_v5_to_v6()
            .unwrap()
            .schema_version(),
        6
    );
    let (client, server) = tokio::time::timeout(Duration::from_secs(5), spawn_board_loopback())
        .await
        .expect("TEST_CODE unconfirmed listener/connect deadline");
    let source = GrpcSource::from_board_loopback_test_client(client);
    let queries = tokio::time::timeout(Duration::from_secs(5), source.connected_board_queries())
        .await
        .expect("TEST_CODE unconfirmed preconnect deadline")
        .unwrap();
    let no_requests = server.snapshot();
    assert!(no_requests.requests.is_empty());
    assert_eq!(no_requests.non_board_requests, 0);
    assert!(server.membership_snapshot().is_empty());
    let stocks = vec![stock()];
    let config = local_config(BUILD_A);
    let context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_LEGACY_UNCONFIRMED"),
    )
    .unwrap();
    // Ancestor provider independently reads committed ordinal0/code, closes that connection,
    // and only then awaits Pending. Its fields and implementation are not modified.
    let provider = PendingRawProvider {
        database: fixture.database(),
        calls: RefCell::new(Vec::new()),
    };
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
            lease_request("TEST_CODE_UNCONFIRMED_OWNER_A", 1_000, 5_000, None),
        )
        .unwrap();
    let intent = lease.intent_id().clone();
    let clock = ControlledClock::new(at(1_100));
    let mut io = local
        .board_preparation_io_v6(
            lease,
            &provider,
            &clock,
            FixedClusterConfiguration::resolve(Some("2")),
            &source,
        )
        .unwrap();
    assert_eq!(
        io.local
            .store
            .connection
            .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        250
    );
    let mut future = Box::pin(prepare_chain_analysis_with_io(
        NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
        stocks.clone(),
        None,
        &mut io,
    ));
    tokio::time::timeout(
        Duration::from_secs(5),
        futures::future::poll_fn(|cx| {
            assert!(
                future.as_mut().poll(cx).is_pending(),
                "TEST_CODE old provider must remain Pending"
            );
            if provider.calls.borrow().is_empty() {
                Poll::Pending
            } else {
                assert_eq!(provider.calls.borrow().as_slice(), ["TEST_CODE_600001"]);
                Poll::Ready(())
            }
        }),
    )
    .await
    .expect("TEST_CODE bounded drive must reach old provider Pending");
    // Actual cancellation: drop the still-pending public prepare and its armed EffectGuard.
    drop(future);
    let reentry = tokio::time::timeout(
        Duration::from_secs(5),
        prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks.clone(),
            None,
            &mut io,
        ),
    )
    .await
    .expect("TEST_CODE same-adapter reentry deadline")
    .expect_err("TEST_CODE cancelled v6 adapter remains stopped");
    assert!(matches!(reentry.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::ResultUnconfirmed { intent_id }) if intent_id == intent.as_str()));
    // The cancellation fence is in validate_fixed_input, before a PreparationFailure is created.
    assert!(reentry.downcast_ref::<PreparationFailure>().is_none());
    assert_eq!(provider.calls.borrow().as_slice(), ["TEST_CODE_600001"]);
    drop(io);
    drop(local);

    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let (head, begin_version) = inspect_unconfirmed("v6-after-cancel", &mut local, &intent, 1);
    assert_eq!(head, begin_version);
    drop(local);
    let request_hash = hex::encode(sha2::Sha256::digest(b"TEST_CODE_600001"));
    let expected_begin = vec![vec![
        Value::Text(intent.as_str().to_owned()),
        Value::Text("ConceptProvider".to_owned()),
        Value::Integer(0),
        Value::Text("TEST_CODE_600001".to_owned()),
        Value::Integer(1),
        Value::Blob(b"TEST_CODE_600001".to_vec()),
        Value::Integer(16),
        Value::Text(request_hash.clone()),
        Value::Text("TEST_CODE_UNCONFIRMED_OWNER_A".to_owned()),
        Value::Integer(1),
        Value::Integer(i64::try_from(begin_version).unwrap()),
        Value::Integer(at(1_100)),
    ]];
    assert_eq!(
        all_rows(
            fixture.connection(),
            "SELECT * FROM chain_post_close_stage_begins"
        ),
        expected_begin
    );
    assert_eq!(fixture.stored_result(&intent), None);
    assert_eq!(
        count(
            fixture.connection(),
            "chain_post_close_cluster_configurations"
        ),
        1
    );
    for table in [
        "chain_post_close_stage_results",
        "chain_post_close_concept_cache_writes",
        "chain_post_close_cluster_materials",
        "chain_post_close_chain_daily_applications",
        "chain_post_close_board_attempt_begins",
        "chain_post_close_board_attempt_results",
        "chain_post_close_board_status_materials",
        "chain_post_close_board_error_materials",
        "chain_post_close_board_kind_finals",
        "chain_post_close_board_directory_materials",
        "chain_post_close_board_selections",
        "data_acquisition_audit",
        "data_acquisition_audit_chain",
    ] {
        assert_eq!(
            count(fixture.connection(), table),
            0,
            "TEST_CODE unconfirmed downstream table={table}"
        );
    }
    assert_eq!(
        fixture
            .connection()
            .query_row(
                "SELECT count(*) FROM stock_concepts WHERE code='TEST_CODE_600001'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(server.snapshot(), no_requests);
    assert!(server.membership_snapshot().is_empty());

    let old_names = legacy_table_names(fixture.connection());
    let old_values = legacy_table_values(fixture.connection(), &old_names, 6);
    let old_catalog = all_rows(fixture.connection(), CATALOG_SQL);
    // Independent 23 columns: begin identity plus the literal qualification kind,
    // six absent result fields, five absent cache fields, and source/seal versions; no CASE/join.
    let mut expected_qualification = vec![
        Value::Text(intent.as_str().to_owned()),
        Value::Text("ConceptProvider".to_owned()),
        Value::Integer(0),
        Value::Text("TEST_CODE_600001".to_owned()),
        Value::Text("LegacyUnconfirmed".to_owned()),
        Value::Integer(i64::try_from(begin_version).unwrap()),
        Value::Text(request_hash),
        Value::Text("TEST_CODE_UNCONFIRMED_OWNER_A".to_owned()),
        Value::Integer(1),
        Value::Integer(at(1_100)),
    ];
    expected_qualification.extend(std::iter::repeat(Value::Null).take(11));
    expected_qualification.extend([Value::Integer(6), Value::Integer(7)]);
    assert_eq!(expected_qualification.len(), 23);
    let qualification = vec![expected_qualification];
    assert_eq!(
        fixture
            .chain_post_close()
            .migrate_schema_v6_to_v7()
            .unwrap()
            .schema_version(),
        7
    );
    assert_eq!(
        fixture
            .chain_post_close()
            .verify_schema()
            .unwrap()
            .schema_version(),
        7
    );
    assert_eq!(
        legacy_table_values(fixture.connection(), &old_names, 6),
        old_values
    );
    let catalog = all_rows(fixture.connection(), CATALOG_SQL);
    assert!(old_catalog.iter().all(|row| catalog.contains(row)));
    let registered = all_rows(fixture.connection(),
        "SELECT name,object_type,CAST(definition AS BLOB) FROM chain_post_close_layout_objects WHERE layout_version=7 ORDER BY name");
    assert_eq!(registered.len(), 99);
    assert_eq!(
        registered,
        catalog
            .iter()
            .filter(
                |row| matches!(&row[0], Value::Text(name) if name.starts_with("chain_post_close_"))
                    && row[3] != Value::Null
            )
            .map(|row| vec![row[0].clone(), row[1].clone(), row[3].clone()])
            .collect::<Vec<_>>()
    );
    // Every new catalog object must belong to the complete v7 registry or its SQLite autoindexes.
    assert!(catalog.iter().filter(|row| !old_catalog.contains(row)).all(|row|
        registered.iter().any(|item| item[0] == row[0] && item[1] == row[1] && item[2] == row[3])
        || (row[1] == Value::Text("index".to_owned()) && row[3] == Value::Null
            && matches!(&row[0], Value::Text(name) if name.starts_with("sqlite_autoindex_chain_post_close_"))
            && registered.iter().any(|item| item[0] == row[2] && item[1] == Value::Text("table".to_owned())))
    ), "TEST_CODE only registered v7 additions and their autoindexes are permitted");
    assert_eq!(
        all_rows(fixture.connection(), QUALIFICATION_SQL),
        qualification
    );
    let rpc = concept_rpc_facts(fixture.connection());
    assert!(rpc.occurrences.is_empty() && rpc.attempt_begins.is_empty());
    assert_eq!(
        (
            rpc.results,
            rpc.status_materials,
            rpc.error_materials,
            rpc.finals,
            rpc.outer_begins,
            rpc.outer_results
        ),
        (0, 0, 0, 0, 1, 0)
    );
    assert_eq!(
        (
            rpc.cache_writes,
            rpc.cached_concepts,
            rpc.cluster_materials,
            rpc.chain_daily_applications,
            rpc.audits,
            rpc.audit_chain
        ),
        (0, 0, 0, 0, 0, 0)
    );
    let names = legacy_table_names(fixture.connection());
    let mut expected_after_reopen = legacy_table_values(fixture.connection(), &names, 7);
    let run_columns = all_rows(
        fixture.connection(),
        "PRAGMA main.table_info('chain_post_close_runs')",
    );
    fixture.reopen();
    assert_eq!(
        legacy_table_values(fixture.connection(), &names, 7),
        expected_after_reopen,
        "TEST_CODE reopened v7 includes all qualifications and all version7 registry rows"
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
            lease_request("TEST_CODE_UNCONFIRMED_OWNER_B", 5_001, 9_000, Some(head)),
        )
        .unwrap();
    let clock = ControlledClock::new(at(5_100));
    let mut io = local
        .concept_rpc_preparation_io_v7(
            lease,
            &queries,
            &clock,
            FixedClusterConfiguration::resolve(Some("2")),
        )
        .unwrap();
    let error = tokio::time::timeout(
        Duration::from_secs(5),
        prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks,
            None,
            &mut io,
        ),
    )
    .await
    .expect("TEST_CODE v7 unconfirmed reopen deadline")
    .expect_err("TEST_CODE v7 must not replay old unconfirmed effect");
    assert!(matches!(error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::IncompleteOnReopen { intent_id }) if intent_id == intent.as_str()));
    let failure = error.downcast_ref::<PreparationFailure>().unwrap();
    assert_eq!(failure.stage(), PreparationStage::Concepts);
    assert!(failure.completed_stages().is_empty());
    drop(io);
    let (reopened_head, reopened_begin) =
        inspect_unconfirmed("v7-reopened", &mut local, &intent, 2);
    assert_eq!((reopened_head, reopened_begin), (head + 1, begin_version));
    drop(local);
    let run = expected_after_reopen
        .get_mut("chain_post_close_runs")
        .unwrap();
    assert_eq!(run.len(), 1);
    for (name, value) in [
        (
            "lease_owner",
            Value::Text("TEST_CODE_UNCONFIRMED_OWNER_B".to_owned()),
        ),
        ("lease_generation", Value::Integer(2)),
        (
            "head_version",
            Value::Integer(i64::try_from(head + 1).unwrap()),
        ),
        ("lease_until", Value::Integer(at(9_000))),
        ("updated_at", Value::Integer(at(5_001))),
    ] {
        let index = run_columns
            .iter()
            .position(|column| column[1] == Value::Text(name.to_owned()))
            .unwrap();
        run[0][index] = value;
    }
    assert_eq!(legacy_table_names(fixture.connection()), names);
    assert_eq!(
        legacy_table_values(fixture.connection(), &names, 7),
        expected_after_reopen
    );
    assert_eq!(all_rows(fixture.connection(), CATALOG_SQL), catalog);
    assert_eq!(
        all_rows(fixture.connection(), QUALIFICATION_SQL),
        qualification
    );
    assert_eq!(
        all_rows(
            fixture.connection(),
            "SELECT * FROM chain_post_close_stage_begins"
        ),
        expected_begin
    );
    assert_eq!(concept_rpc_facts(fixture.connection()), rpc);
    assert_eq!(provider.calls.borrow().as_slice(), ["TEST_CODE_600001"]);
    assert!(server.membership_snapshot().is_empty());
    assert_eq!(server.snapshot(), no_requests);
    drop(queries);
    drop(source);
    assert_eq!(server.finish().await, no_requests);
}
