use super::super::super::schema;
use super::*;
use crate::database::data_acquisition_audit::DataAcquisitionAuditReceipt;
use rusqlite::types::Value;

type SqlRows = Vec<Vec<Value>>;

#[derive(Clone, Debug, PartialEq)]
struct OwnedDatabaseState {
    catalog: BTreeMap<String, Vec<Value>>,
    tables: BTreeMap<String, SqlRows>,
    application_id: i64,
    user_version: i64,
}

// Read SQL values, not database-file bytes; include every existing table and object.
fn owned_state(connection: &Connection) -> OwnedDatabaseState {
    let catalog = all_rows(
        connection,
        "SELECT name,type,tbl_name,CAST(sql AS BLOB) FROM main.sqlite_schema ORDER BY name",
    )
    .into_iter()
    .map(|row| {
        let Value::Text(name) = &row[0] else {
            panic!("TEST_CODE schema object name");
        };
        (name.clone(), row)
    })
    .collect::<BTreeMap<_, _>>();
    let tables = catalog
        .iter()
        .filter(|(_, row)| row[1] == Value::Text("table".to_owned()))
        .map(|(name, _)| {
            let quoted = name.replace('"', "\"\"");
            let select = format!("SELECT * FROM main.\"{quoted}\"");
            let width = connection.prepare(&select).unwrap().column_count();
            let order = (1..=width)
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join(",");
            (
                name.clone(),
                all_rows(connection, &format!("{select} ORDER BY {order}")),
            )
        })
        .collect();
    OwnedDatabaseState {
        catalog,
        tables,
        application_id: connection
            .query_row("PRAGMA application_id", [], |row| row.get(0))
            .unwrap(),
        user_version: connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap(),
    }
}

// New v6 objects/rows are allowed only outside this complete, pre-migration projection.
fn assert_old_state_unchanged(before: &OwnedDatabaseState, after: &OwnedDatabaseState) {
    assert_eq!(after.application_id, before.application_id);
    assert_eq!(after.user_version, before.user_version);
    for (name, definition) in &before.catalog {
        assert_eq!(
            after.catalog.get(name),
            Some(definition),
            "old object {name}"
        );
    }
    for (name, records) in &before.tables {
        let mut actual = after.tables.get(name).unwrap().clone();
        if matches!(
            name.as_str(),
            "chain_post_close_layouts" | "chain_post_close_layout_objects"
        ) {
            actual.retain(|row| matches!(&row[0], Value::Integer(version) if *version <= 5));
        }
        assert_eq!(&actual, records, "old table {name}");
    }
}

fn expected_after_takeover(
    connection: &Connection,
    before: &OwnedDatabaseState,
    owner: &str,
    generation: i64,
    head: u64,
    now: i64,
    until: i64,
) -> OwnedDatabaseState {
    let mut expected = before.clone();
    let run = expected.tables.get_mut("chain_post_close_runs").unwrap();
    assert_eq!(run.len(), 1);
    let columns = all_rows(
        connection,
        "PRAGMA main.table_info('chain_post_close_runs')",
    );
    for (column, value) in [
        ("lease_owner", Value::Text(owner.to_owned())),
        ("lease_generation", Value::Integer(generation)),
        ("head_version", Value::Integer(i64::try_from(head).unwrap())),
        ("updated_at", Value::Integer(at(now))),
        ("lease_until", Value::Integer(at(until))),
    ] {
        let ordinal = columns
            .iter()
            .position(|row| row[1] == Value::Text(column.to_owned()))
            .unwrap();
        run[0][ordinal] = value;
    }
    expected
}

fn assert_legacy_status(connection: &Connection, expected: &SqlRows) {
    assert_eq!(
        all_rows(connection,
            "SELECT * FROM chain_post_close_board_status_materials ORDER BY intent_id,kind,attempt_ordinal"),
        *expected,
    );
    assert!(all_rows(
        connection,
        "SELECT * FROM chain_post_close_board_error_materials"
    )
    .is_empty());
}

fn assert_original_success_audits(connection: &Connection) {
    let mut statement = connection
        .prepare(
            "SELECT audit_id,audit_record_hash,previous_outcome,current_outcome
         FROM chain_post_close_board_kind_finals
         ORDER BY CASE kind WHEN 'Industry' THEN 0 ELSE 1 END",
        )
        .unwrap();
    let receipts = statement
        .query_map([], |row| {
            Ok(DataAcquisitionAuditReceipt {
                audit_id: row.get(0)?,
                record_hash: row.get(1)?,
                previous_outcome: row.get(2)?,
                current_outcome: row.get(3)?,
            })
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    drop(statement);
    assert_eq!(receipts.len(), 2);
    let transaction = connection.unchecked_transaction().unwrap();
    for (index, receipt) in receipts.iter().enumerate() {
        assert_eq!(receipt.audit_id, if index == 0 { 1 } else { 2 });
        assert_eq!(
            receipt.previous_outcome.as_deref(),
            if index == 0 { None } else { Some("available") }
        );
        assert_eq!(receipt.current_outcome, "available");
        let verified = read_acquisition_in_transaction(&transaction, receipt)
            .expect("TEST_CODE original complete BR159 chain reader");
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
}

async fn exercise_real_v5_migration(commit_failure: bool) {
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
    assert_eq!(
        fixture
            .chain_post_close()
            .migrate_schema_v4_to_v5()
            .unwrap()
            .schema_version(),
        5
    );
    schema::verify_runtime_layout_version(fixture.connection(), 5).unwrap();

    let (client, server) = spawn_board_loopback().await;
    let source = GrpcSource::from_board_loopback_test_client(client);
    let config = local_config(BUILD_A);
    let stocks = cluster_tests::cluster_stocks();
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
        cluster_tests::MAIN_CONCEPT.to_owned(),
        "TEST_CODE_BOARD_MAIN".to_owned(),
    )]);
    let context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_REAL_V5_LEGACY_MIGRATION"),
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
            lease_request("TEST_CODE_LEGACY_OWNER_A", 1_000, 60_000_000, None),
        )
        .unwrap();
    let intent_id = lease.intent_id().clone();
    let provider = cluster_tests::PanicRawProvider {
        calls: Cell::new(0),
    };
    let clock = ControlledClock::new(at(1_100));
    let mut io = local
        .board_preparation_io(
            lease,
            &provider,
            &clock,
            FixedClusterConfiguration::resolve(Some("2")),
            &source,
        )
        .unwrap();
    let error = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks.clone(),
            None,
            &mut io,
        ),
    )
    .await
    .expect("TEST_CODE first real v5 prepare timeout")
    .expect_err("TEST_CODE v5 stops at unmigrated Positions");
    assert_candidates_completed_before_positions(&error, &expected_directory, &expected_selected);
    assert_eq!(provider.calls.get(), 0);
    drop(io);
    let recovery = local.inspect_board_directory(&intent_id).unwrap();
    assert_eq!(recovery.attempts().len(), 3);
    assert!(recovery
        .attempts()
        .iter()
        .all(|attempt| attempt.is_confirmed()));
    let observation = server.snapshot();
    assert_eq!(observation.requests.len(), 3);
    assert_eq!(observation.non_board_requests, 0);
    assert_eq!(
        observation
            .requests
            .iter()
            .map(|r| r.kind.as_str())
            .collect::<Vec<_>>(),
        ["Industry", "Industry", "Concept"]
    );
    assert_eq!(
        observation.requests[0].request_id,
        observation.requests[1].request_id
    );
    assert_ne!(
        observation.requests[1].request_id,
        observation.requests[2].request_id
    );
    assert!(observation.requests.iter().all(|request| {
        request.authorized
            && request.limit == 10_000
            && request.protocol_version == 1
            && request.payload_schema == "board.directory"
            && request.payload_schema_version == 1
            && request.payload_content_type == "application/json; charset=utf-8"
            && !request.allow_unadmitted
    }));
    assert_eq!(
        recovery.attempts()[0].error_detail_bytes(),
        Some(observation.retry_error_detail.as_slice())
    );
    assert!(recovery.attempts()[0].payload_bytes().is_none());
    assert_eq!(
        recovery.attempts()[1].payload_bytes(),
        Some(INDUSTRY_DIRECTORY_BYTES)
    );
    assert_eq!(
        recovery.attempts()[2].payload_bytes(),
        Some(CONCEPT_DIRECTORY_BYTES)
    );
    assert_eq!(recovery.board_directory(), &expected_directory);
    assert_eq!(recovery.selected_board_codes(), &expected_selected);
    assert_eq!(recovery.directories().len(), 2);
    for (index, directory) in recovery.directories().iter().enumerate() {
        let kind = ["Industry", "Concept"][index];
        let evidence = serde_json::json!({
            "provider": "Tdx", "source": "TEST_CODE_LOOPBACK_BOARD_SOURCE",
            "source_at": "2026-07-21T15:30:00+08:00",
            "observed_at": "2026-07-21T15:31:00+08:00", "batch_id": BOARD_BATCH_IDS[index],
        });
        let fact: serde_json::Value = serde_json::from_slice(directory.fact_bytes()).unwrap();
        assert_eq!(
            fact,
            serde_json::json!({
                "schema_version": 1,
                "outcome": { "Available": {
                    "evidence": evidence.clone(),
                    "records": [
                        { "code": "TEST_CODE_BOARD_MAIN", "name": "TEST_CODE_CLUSTER_A_MAIN",
                          "kind": kind, "member_count": 2, "evidence": evidence.clone() },
                        { "code": (["TEST_CODE_BOARD_INDUSTRY_ONLY", "TEST_CODE_BOARD_CONCEPT_ONLY"][index]),
                          "name": (["TEST_CODE_INDUSTRY_ONLY", "TEST_CODE_CONCEPT_ONLY"][index]),
                          "kind": kind, "member_count": ([3, 4][index]), "evidence": evidence },
                    ],
                }},
            })
        );
    }
    let head = local.inspect_run(&intent_id).unwrap().head_version();
    assert_eq!(local.inspect_run(&intent_id).unwrap().lease_generation(), 1);
    drop(recovery);
    drop(local);
    assert_original_success_audits(fixture.connection());
    let original_boards = board_durable_facts(fixture.connection());
    let before = owned_state(fixture.connection());
    let retry = confirmed_retry_fact(fixture.connection());
    assert_eq!(
        raw_digest(&retry.result_bytes).as_str(),
        retry.result_digest
    );
    assert_eq!(
        raw_digest(&retry.request_bytes).as_str(),
        retry.request_digest
    );
    let retry_wire: serde_json::Value = serde_json::from_slice(&retry.result_bytes).unwrap();
    assert_eq!(retry_wire["schema_version"], 1);
    assert_eq!(retry_wire["continuation"], "Retry");
    assert_eq!(retry_wire["retry_decision"], "RetryBackoff");
    assert_eq!(retry_wire["backoff_ms"], 1000);
    assert!(retry_wire["response_wire"].is_null());
    let mut legacy = all_rows(
        fixture.connection(),
        "SELECT intent_id,kind,attempt_ordinal,run_version,result_sha256,request_sha256
         FROM chain_post_close_board_attempt_results WHERE wire_outcome='Status'
         ORDER BY intent_id,kind,attempt_ordinal",
    );
    assert_eq!(legacy.len(), 1);
    assert_eq!(legacy[0][0], Value::Text(intent_id.as_str().to_owned()));
    assert_eq!(legacy[0][1], Value::Text("Industry".to_owned()));
    assert_eq!(legacy[0][2], Value::Integer(1));
    assert_eq!(legacy[0][3], Value::Integer(retry.result_version));
    assert_eq!(legacy[0][4], Value::Text(retry.result_digest));
    assert_eq!(legacy[0][5], Value::Text(retry.request_digest));
    // Exact 21-column A shape, independently projected from the real old parent.
    legacy[0].push(Value::Text("LegacyV5Absent".to_owned()));
    legacy[0].extend(std::iter::repeat(Value::Null).take(13));
    legacy[0].push(Value::Integer(6));

    if commit_failure {
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
        reader.busy_timeout(std::time::Duration::ZERO).unwrap();
        reader.execute_batch("BEGIN DEFERRED;").unwrap();
        assert_eq!(owned_state(&reader), before); // Establish the real shared read lock.
        assert_eq!(
            fixture.chain_post_close().migrate_schema_v5_to_v6(),
            Err(ChainPostCloseError::StorageFailed {
                operation: "commit"
            }),
        );
        assert!(fixture.connection().is_autocommit());
        assert_eq!(
            fixture
                .connection()
                .query_row("PRAGMA query_only", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            0
        );
        // Only the two pre-existing descriptors are used while the reader holds its lock.
        assert_eq!(owned_state(fixture.connection()), before);
        assert_eq!(owned_state(&reader), before);
        reader.execute_batch("ROLLBACK;").unwrap();
        assert_eq!(owned_state(&reader), before); // Fresh SQL view, not the old read transaction.
        reader.close().unwrap();
        fixture.reopen();
        assert_eq!(
            fixture
                .chain_post_close()
                .verify_schema()
                .unwrap()
                .schema_version(),
            5
        );
        schema::verify_runtime_layout_version(fixture.connection(), 5).unwrap();
        assert_eq!(owned_state(fixture.connection()), before);
        assert_eq!(server.snapshot(), observation);
    }

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
    assert_old_state_unchanged(&before, &owned_state(fixture.connection()));
    assert_legacy_status(fixture.connection(), &legacy);
    assert_eq!(board_durable_facts(fixture.connection()), original_boards);
    assert_original_success_audits(fixture.connection());

    // True reopen; the real exact-v5 reader and factory must not become version-agnostic.
    fixture.reopen();
    assert_eq!(
        fixture
            .chain_post_close()
            .verify_schema()
            .unwrap()
            .schema_version(),
        6
    );
    assert_eq!(
        schema::verify_runtime_layout_version(fixture.connection(), 5),
        Err(ChainPostCloseError::UnsupportedVersion)
    );
    assert_old_state_unchanged(&before, &owned_state(fixture.connection()));
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
                "TEST_CODE_LEGACY_OWNER_B",
                60_001_000,
                120_000_000,
                Some(head),
            ),
        )
        .unwrap();
    assert_eq!(lease.generation(), 2);
    assert_eq!(lease.head_version(), head + 1);
    let provider = cluster_tests::PanicRawProvider {
        calls: Cell::new(0),
    };
    let clock = ControlledClock::new(at(60_001_100));
    assert!(matches!(
        local.board_preparation_io(
            lease,
            &provider,
            &clock,
            FixedClusterConfiguration::resolve(Some("2")),
            &source,
        ),
        Err(ChainPostCloseError::UnsupportedVersion)
    ));
    assert_eq!(provider.calls.get(), 0);
    drop(local);
    let after_rejected_factory = expected_after_takeover(
        fixture.connection(),
        &before,
        "TEST_CODE_LEGACY_OWNER_B",
        2,
        head + 1,
        60_001_000,
        120_000_000,
    );
    assert_old_state_unchanged(&after_rejected_factory, &owned_state(fixture.connection()));
    assert_legacy_status(fixture.connection(), &legacy);
    assert_eq!(server.snapshot(), observation);

    // Rejection consumed that lease. Obtain the next genuine expired-lease/CAS grant;
    // do not clone a private lease, or reacquire before expiry merely for the test.
    fixture.reopen();
    assert_eq!(
        fixture
            .chain_post_close()
            .verify_schema()
            .unwrap()
            .schema_version(),
        6
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
                "TEST_CODE_LEGACY_OWNER_C",
                120_001_000,
                180_000_000,
                Some(head + 1),
            ),
        )
        .unwrap();
    assert_eq!(lease.generation(), 3);
    assert_eq!(lease.head_version(), head + 2);
    let provider = cluster_tests::PanicRawProvider {
        calls: Cell::new(0),
    };
    let clock = ControlledClock::new(at(120_001_100));
    let mut io = local
        .board_preparation_io_v6(
            lease,
            &provider,
            &clock,
            FixedClusterConfiguration::resolve(Some("2")),
            &source,
        )
        .expect("TEST_CODE normal v6 factory admits migrated old facts");
    let error = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks,
            None,
            &mut io,
        ),
    )
    .await
    .expect("TEST_CODE migrated prepare timeout")
    .expect_err("TEST_CODE migrated prepare still stops at Positions");
    assert_candidates_completed_before_positions(&error, &expected_directory, &expected_selected);
    assert_eq!(provider.calls.get(), 0);
    drop(io);
    let recovered = local.inspect_board_directory(&intent_id).unwrap();
    assert_eq!(recovered.board_directory(), &expected_directory);
    assert_eq!(recovered.selected_board_codes(), &expected_selected);
    assert_eq!(recovered.attempts().len(), 3);
    assert!(recovered
        .attempts()
        .iter()
        .all(|attempt| attempt.is_confirmed()));
    assert_eq!(
        local.inspect_run(&intent_id).unwrap().head_version(),
        head + 2
    );
    assert_eq!(local.inspect_run(&intent_id).unwrap().lease_generation(), 3);
    drop(recovered);
    drop(local);
    let after_recovery = expected_after_takeover(
        fixture.connection(),
        &before,
        "TEST_CODE_LEGACY_OWNER_C",
        3,
        head + 2,
        120_001_000,
        180_000_000,
    );
    assert_old_state_unchanged(&after_recovery, &owned_state(fixture.connection()));
    assert_eq!(board_durable_facts(fixture.connection()), original_boards);
    assert_legacy_status(fixture.connection(), &legacy);
    assert_original_success_audits(fixture.connection());
    assert_eq!(server.snapshot(), observation);
    drop(source);
    assert_eq!(server.finish().await, observation);
}

#[tokio::test]
async fn real_v5_status_response_and_audits_migrate_to_legacy_v6_without_replay() {
    exercise_real_v5_migration(false).await;
}

#[tokio::test]
async fn real_v5_to_v6_commit_failure_rolls_back_then_reopens_and_recovers_without_rpc() {
    exercise_real_v5_migration(true).await;
}
