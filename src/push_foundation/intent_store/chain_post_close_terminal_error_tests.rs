use super::*;
use crate::data_gateway::GatewayError;
use crate::database::data_acquisition_audit::{
    DataAcquisitionAuditReceipt, DataAcquisitionAuditRecord,
};
use crate::grpc_client::client::board_loopback_fixture::{
    spawn_board_terminal_error_loopback, BoardLoopbackObservation,
};
use std::time::Duration;

#[path = "chain_post_close_error_material_commit_tests.rs"]
mod error_material_commit_tests;

// Offsets are UTC microseconds from CAPTURED_AT, not seconds.
// The first lease covers the real one-second Industry retry.
const RAW_OFFSET_US: i64 = 2_000_000;
const FALLBACK_OFFSET_US: i64 = 3_456_789;
const APPLY_OFFSET_US: i64 = 4_567_890;
const FIRST_FALLBACK: &str = "2026-07-21T07:31:03.456Z";
const GATEWAY_MESSAGE: &str = "gRPC BoardDirectory 查询失败: 数据完整性/连续性失败 (不能当空成功)";

#[derive(Clone, Debug, PartialEq)]
struct ErrorFacts {
    board: BoardDurableFacts,
    status_materials: Vec<Vec<rusqlite::types::Value>>,
    error_materials: Vec<Vec<rusqlite::types::Value>>,
    upstream: Vec<Vec<Vec<rusqlite::types::Value>>>,
}

fn error_facts(connection: &Connection) -> ErrorFacts {
    ErrorFacts {
        board: board_durable_facts(connection),
        status_materials: all_rows(
            connection,
            "SELECT * FROM chain_post_close_board_status_materials
             ORDER BY kind,attempt_ordinal",
        ),
        error_materials: all_rows(
            connection,
            "SELECT * FROM chain_post_close_board_error_materials ORDER BY kind",
        ),
        upstream: [
            "SELECT * FROM stock_concepts ORDER BY code",
            "SELECT * FROM chain_daily ORDER BY date,concept",
            "SELECT * FROM chain_post_close_cluster_configurations ORDER BY intent_id",
            "SELECT * FROM chain_post_close_cluster_materials ORDER BY intent_id",
            "SELECT * FROM chain_post_close_chain_daily_applications ORDER BY intent_id",
            "SELECT context_bytes,input_bytes FROM chain_post_close_runs ORDER BY intent_id",
        ]
        .into_iter()
        .map(|sql| all_rows(connection, sql))
        .collect(),
    }
}

// This clock never opens/closes a database FD while holding its read lock.
// Only durable phase predicates select the time; there is no call-count trigger.
struct TerminalErrorCommitClock {
    reader: RefCell<Option<Connection>>,
    armed: Cell<bool>,
}

impl TerminalErrorCommitClock {
    fn new(database: &std::path::Path) -> Self {
        let reader = Connection::open_with_flags(
            database,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .expect("TEST_CODE open pre-existing terminal error observer");
        reader.busy_timeout(Duration::ZERO).unwrap();
        let journal: String = reader
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(journal.to_ascii_lowercase(), "delete");
        Self {
            reader: RefCell::new(Some(reader)),
            armed: Cell::new(false),
        }
    }

    fn facts(&self) -> ErrorFacts {
        error_facts(self.reader.borrow().as_ref().unwrap())
    }

    fn release(&self) {
        let reader = self.reader.borrow_mut().take().unwrap();
        reader.execute_batch("ROLLBACK;").unwrap();
        reader.close().unwrap();
    }
}

impl ConceptEffectClock for TerminalErrorCommitClock {
    fn now(&self) -> UtcMicros {
        if self.armed.get() {
            return UtcMicros::try_new(at(APPLY_OFFSET_US)).unwrap();
        }
        let reader = self.reader.borrow();
        let reader = reader.as_ref().unwrap();
        let raw_and_safety: bool = reader
            .query_row(
                "SELECT EXISTS(
                 SELECT 1 FROM chain_post_close_board_attempt_results AS result
                 JOIN chain_post_close_board_status_materials AS safety
                   ON safety.intent_id=result.intent_id AND safety.kind=result.kind
                  AND safety.attempt_ordinal=result.attempt_ordinal
                  AND safety.result_run_version=result.run_version
                  AND safety.result_sha256=result.result_sha256
                 WHERE result.kind='Concept' AND result.wire_outcome='Status'
                   AND result.continuation='Terminal' AND safety.provenance='Captured'
             )",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let prepared_error: bool = reader
            .query_row(
                "SELECT EXISTS(
                 SELECT 1 FROM chain_post_close_board_error_materials AS material
                 JOIN chain_post_close_board_attempt_results AS result
                   ON result.intent_id=material.intent_id AND result.kind=material.kind
                  AND result.attempt_ordinal=material.terminal_attempt_ordinal
                  AND result.run_version=material.terminal_result_run_version
                  AND result.result_sha256=material.terminal_result_sha256
                 WHERE material.kind='Concept' AND result.continuation='Terminal'
             ) AND NOT EXISTS(
                 SELECT 1 FROM chain_post_close_board_kind_finals WHERE kind='Concept'
             )",
                [],
                |row| row.get(0),
            )
            .unwrap();
        if prepared_error {
            assert!(raw_and_safety, "TEST_CODE B must follow confirmed raw+A");
            let captured_at: i64 = reader
                .query_row(
                    "SELECT captured_at FROM chain_post_close_board_error_materials
                 WHERE kind='Concept'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(captured_at, at(FALLBACK_OFFSET_US));
            reader.execute_batch("BEGIN DEFERRED;").unwrap();
            reader
                .query_row(
                    "SELECT count(*) FROM chain_post_close_board_error_materials",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap();
            self.armed.set(true);
            UtcMicros::try_new(at(APPLY_OFFSET_US)).unwrap()
        } else if raw_and_safety {
            UtcMicros::try_new(at(FALLBACK_OFFSET_US)).unwrap()
        } else {
            UtcMicros::try_new(at(RAW_OFFSET_US)).unwrap()
        }
    }
}

fn assert_gateway_error(error: &GatewayError) {
    assert_eq!(error.capability(), "GrpcBridge");
    assert_eq!(error.provider(), None);
    assert_eq!(error.audit_outcome(), "invalid_request");
    assert_eq!(error.reason_code(), "invalid_request");
    assert!(!error.retryable());
    assert_eq!(error.message(), GATEWAY_MESSAGE);
}

fn assert_error_record(record: DataAcquisitionAuditRecord<'_>) {
    assert_eq!(record.capability, "board-directory");
    assert_eq!(record.provider, "Tdx");
    assert_eq!(record.source, "review-data-gateway");
    assert_eq!(record.request_hash, BOARD_REQUEST_HASHES[1]);
    assert_eq!(record.source_at, None);
    assert_eq!(record.observed_at, FIRST_FALLBACK);
    assert_eq!(record.batch_id, None);
    assert_eq!(record.outcome, "invalid_request");
    assert_eq!(record.request_count, 1);
    assert_eq!(record.accepted_count, 0);
    assert_eq!(record.rejected_count, 1);
    assert_eq!(record.reason_code, "invalid_request");
    assert!(!record.retryable);
}

fn assert_confirmed_error_material(
    local: &mut super::super::super::LocalChainPostClose<'_>,
    intent_id: &IntentId,
) {
    // Future normal owned inspection interface: intentional compile RED until delivered.
    let material = local
        .inspect_board_error_material(intent_id, BoardKind::Concept)
        .expect("TEST_CODE read validated terminal error material")
        .expect("TEST_CODE confirmed terminal error material");
    assert_gateway_error(material.gateway_error());
    assert_eq!(
        material.status_diagnostic(),
        Some("[redacted-unclassified-status]")
    );
    assert_error_record(material.audit_record());
}

fn assert_failed_board_observations(error: &anyhow::Error) -> String {
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::StageNotMigrated {
            next: UnmigratedStage::Positions
        })
    ));
    let failure = error.downcast_ref::<PreparationFailure>().unwrap();
    assert_eq!(failure.stage(), PreparationStage::Positions);
    assert_eq!(
        failure.completed_stages().last(),
        Some(&PreparationStage::Candidates)
    );
    assert_eq!(failure.board_source().status(), &SourceStatus::Unavailable);
    assert!(failure.board_directory().is_empty());
    assert!(failure.candidate_board_codes().is_empty());
    assert!(failure.board_evidence().is_empty());
    let reason = failure.board_source().reason().unwrap();
    assert!(reason.contains("invalid_request"), "{reason}");
    assert!(!reason.contains("unsupported"), "{reason}");
    let candidate = failure
        .candidate_sources()
        .get(cluster_tests::MAIN_CONCEPT)
        .unwrap();
    assert_eq!(candidate.status(), &SourceStatus::Unavailable);
    assert!(candidate.reason().unwrap().contains("invalid_request"));
    assert!(failure.positions().is_empty());
    assert!(failure.lhb_map().is_empty());
    assert_eq!(failure.lhb_source().status(), &SourceStatus::Unknown);
    assert_eq!(failure.clusters().len(), 1);
    assert_eq!(failure.clusters()[0].concept, cluster_tests::MAIN_CONCEPT);
    reason.to_owned()
}

fn assert_server_requests(observation: &BoardLoopbackObservation) {
    assert_eq!(observation.requests.len(), 3);
    assert_eq!(observation.non_board_requests, 0);
    assert_eq!(
        observation
            .requests
            .iter()
            .map(|request| request.kind.as_str())
            .collect::<Vec<_>>(),
        ["Industry", "Industry", "Concept"],
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
}

fn assert_terminal_wire(connection: &Connection, expected_detail: &[u8]) {
    let (bytes, returned_at, committed_at): (Vec<u8>, i64, i64) = connection
        .query_row(
            "SELECT result_bytes,returned_at,committed_at
         FROM chain_post_close_board_attempt_results WHERE kind='Concept'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    let raw: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(raw["schema_version"], 1);
    assert!(raw["response_wire"].is_null());
    assert_eq!(raw["status_code"], 9); // tonic FailedPrecondition
    assert_eq!(raw["retry_decision"], "NoRetry");
    assert_eq!(raw["continuation"], "Terminal");
    assert!(raw["backoff_ms"].is_null());
    let details: Vec<u8> = serde_json::from_value(raw["status_details"].clone()).unwrap();
    let trailer: Vec<u8> =
        serde_json::from_value(raw["status_error_detail_trailer"]["Bytes"].clone()).unwrap();
    assert_eq!(details, expected_detail);
    assert_eq!(trailer, expected_detail);
    assert_eq!(returned_at, at(RAW_OFFSET_US));
    assert_eq!(committed_at, at(RAW_OFFSET_US));
}

fn assert_original_audits(connection: &Connection) {
    let receipts = {
        let mut statement = connection
            .prepare(
                "SELECT kind,audit_id,audit_record_hash,previous_outcome,current_outcome
             FROM chain_post_close_board_kind_finals
             ORDER BY CASE kind WHEN 'Industry' THEN 0 ELSE 1 END",
            )
            .unwrap();
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    DataAcquisitionAuditReceipt {
                        audit_id: row.get(1)?,
                        record_hash: row.get(2)?,
                        previous_outcome: row.get(3)?,
                        current_outcome: row.get(4)?,
                    },
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        rows
    };
    assert_eq!(receipts.len(), 2);
    assert_eq!(receipts[0].1.audit_id, 1);
    assert_eq!(receipts[1].1.audit_id, 2);
    assert_eq!(receipts[0].0, "Industry");
    assert_eq!(receipts[0].1.previous_outcome, None);
    assert_eq!(receipts[0].1.current_outcome, "available");
    assert_eq!(receipts[1].0, "Concept");
    assert_eq!(receipts[1].1.previous_outcome.as_deref(), Some("available"));
    assert_eq!(receipts[1].1.current_outcome, "invalid_request");
    let transaction = connection.unchecked_transaction().unwrap();
    let industry = read_acquisition_in_transaction(&transaction, &receipts[0].1).unwrap();
    let record = industry.record();
    assert_eq!(record.capability, "board-directory");
    assert_eq!(record.provider, "Tdx");
    assert_eq!(record.request_hash, BOARD_REQUEST_HASHES[0]);
    assert_eq!(record.source, "TEST_CODE_LOOPBACK_BOARD_SOURCE");
    assert_eq!(record.source_at, Some("2026-07-21T15:30:00+08:00"));
    assert_eq!(record.observed_at, "2026-07-21T15:31:00+08:00");
    assert_eq!(record.batch_id, Some("TEST_CODE_INDUSTRY_BATCH"));
    assert_eq!(record.outcome, "available");
    assert_eq!(record.reason_code, "accepted");
    assert_eq!(
        (
            record.request_count,
            record.accepted_count,
            record.rejected_count
        ),
        (1, 2, 0)
    );
    assert!(!record.retryable);
    let concept = read_acquisition_in_transaction(&transaction, &receipts[1].1).unwrap();
    assert_error_record(concept.record());
    transaction.rollback().unwrap();
}

#[tokio::test]
async fn confirmed_terminal_error_recovers_after_final_commit_failure_without_rpc_or_time_drift() {
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

    // No future-table query precedes the real migration. The first draft RED is
    // missing normal v6 interfaces, never "no such table" on an old v5 fixture.
    let migrated = fixture
        .chain_post_close()
        .migrate_schema_v5_to_v6()
        .expect("TEST_CODE normal explicit v5-to-v6 migration");
    assert_eq!(migrated.schema_version(), 6);
    assert_eq!(
        fixture
            .chain_post_close()
            .verify_schema()
            .unwrap()
            .schema_version(),
        6
    );
    let journal: String = fixture
        .connection()
        .query_row("PRAGMA journal_mode=DELETE", [], |row| row.get(0))
        .unwrap();
    assert_eq!(journal.to_ascii_lowercase(), "delete");
    fixture.connection().busy_timeout(Duration::ZERO).unwrap();
    let clock = TerminalErrorCommitClock::new(&fixture.database());
    let (client, server) = spawn_board_terminal_error_loopback().await;
    let source = GrpcSource::from_board_loopback_test_client(client);
    let stocks = cluster_tests::cluster_stocks();
    let config = local_config(BUILD_A);
    let context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_RUN_TERMINAL_ERROR_FINAL_COMMIT"),
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
            lease_request("TEST_CODE_ERROR_OWNER_A", 1_000_000, 60_000_000, None),
        )
        .unwrap();
    let intent_id = lease.intent_id().clone();
    let provider = cluster_tests::PanicRawProvider {
        calls: Cell::new(0),
    };
    let mut io = local
        .board_preparation_io_v6(
            lease,
            &provider,
            &clock,
            FixedClusterConfiguration::resolve(Some("2")),
            &source,
        )
        .expect("TEST_CODE construct normal v6 adapter");
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
    .expect("TEST_CODE terminal error first prepare timeout")
    .expect_err("TEST_CODE true final COMMIT must fail");
    assert!(
        clock.armed.get(),
        "TEST_CODE must reach B-confirmed final-COMMIT fault"
    );
    assert!(matches!(error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::ResultUnconfirmed { intent_id: stopped })
            if stopped.as_str() == intent_id.as_str()));
    assert!(matches!(
        error.downcast_ref::<ChainPostCloseError>(),
        Some(ChainPostCloseError::StorageFailed {
            operation: "commit"
        })
    ));
    let failure = error
        .downcast_ref::<PreparationFailure>()
        .expect("TEST_CODE final COMMIT stop retains public preparation evidence");
    assert_eq!(failure.stage(), PreparationStage::Candidates);
    assert_eq!(
        failure.completed_stages().last(),
        Some(&PreparationStage::ClusterWritesAndLifecycle),
    );
    assert_eq!(provider.calls.get(), 0);
    drop(io);

    // The lock still exists; all reads use already-open connections.
    let failed = clock.facts();
    assert_eq!(failed.board.begins.len(), 3);
    assert_eq!(failed.board.results.len(), 3);
    assert_eq!(failed.status_materials.len(), 2); // Industry Retry and Concept terminal
    assert_eq!(failed.error_materials.len(), 1);
    assert_eq!(failed.board.finals.len(), 1);
    assert_eq!(failed.board.audits.len(), 1);
    assert_eq!(failed.board.audit_chain.len(), 1);
    assert!(failed.board.directories.is_empty());
    assert!(failed.board.selections.is_empty());
    assert_confirmed_error_material(&mut local, &intent_id);
    let mut head = local.inspect_run(&intent_id).unwrap().head_version();
    assert_server_requests(&server.snapshot());
    let terminal_detail = server.terminal_error_detail();
    assert!(!terminal_detail.is_empty());
    let detail =
        <crate::grpc_client::pb::magic::market::v1::ErrorDetail as prost::Message>::decode(
            terminal_detail.as_slice(),
        )
        .unwrap();
    assert_eq!(detail.request_id, server.snapshot().requests[2].request_id);
    assert_eq!(
        detail.operation,
        crate::grpc_client::pb::magic::market::v1::Operation::BoardDirectory as i32
    );
    assert_eq!(detail.provider, "Tdx");
    assert_eq!(detail.reason_code, "invalid_evidence");
    assert!(!detail.retryable);

    clock.release();
    drop(local);
    assert_eq!(error_facts(fixture.connection()), failed);
    assert_terminal_wire(fixture.connection(), &terminal_detail);
    let material_head: i64 = fixture
        .connection()
        .query_row(
            "SELECT run_version FROM chain_post_close_board_error_materials WHERE kind='Concept'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(head, u64::try_from(material_head).unwrap());

    let mut first_recovery_facts: Option<ErrorFacts> = None;
    let mut first_recovery_reason: Option<String> = None;
    for (owner, start, until, observed) in [
        (
            "TEST_CODE_ERROR_OWNER_B",
            61_000_000,
            120_000_000,
            62_000_000,
        ),
        (
            "TEST_CODE_ERROR_OWNER_C",
            121_000_000,
            180_000_000,
            122_000_000,
        ),
    ] {
        fixture.reopen(); // actually closes and reopens the owned BusinessIntentStore
        let before_resume = error_facts(fixture.connection());
        assert_eq!(before_resume.status_materials, failed.status_materials);
        assert_eq!(before_resume.error_materials, failed.error_materials);
        let mut local = fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .unwrap();
        let lease = local
            .resume_run(&intent_id, lease_request(owner, start, until, Some(head)))
            .unwrap();
        let recovery_clock = ControlledClock::new(at(observed));
        let mut io = local
            .board_preparation_io_v6(
                lease,
                &provider,
                &recovery_clock,
                FixedClusterConfiguration::resolve(Some("2")),
                &source,
            )
            .unwrap();
        let error = tokio::time::timeout(
            Duration::from_secs(5),
            prepare_chain_analysis_with_io(
                NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
                stocks.clone(),
                None,
                &mut io,
            ),
        )
        .await
        .expect("TEST_CODE terminal error recovery timeout")
        .expect_err("TEST_CODE original ordinary board failure precedes Positions gate");
        let reason = assert_failed_board_observations(&error);
        assert_eq!(provider.calls.get(), 0);
        drop(io);
        assert_confirmed_error_material(&mut local, &intent_id);
        head = local.inspect_run(&intent_id).unwrap().head_version();
        drop(local);

        let recovered = error_facts(fixture.connection());
        assert_eq!(recovered.board.begins, failed.board.begins);
        assert_eq!(recovered.board.results, failed.board.results);
        assert_eq!(recovered.status_materials, failed.status_materials);
        assert_eq!(recovered.error_materials, failed.error_materials);
        assert_eq!(recovered.upstream, failed.upstream);
        assert_eq!(recovered.board.finals.len(), 2);
        assert_eq!(recovered.board.finals[0], failed.board.finals[0]);
        assert_eq!(recovered.board.audits.len(), 2);
        assert_eq!(recovered.board.audits[0], failed.board.audits[0]);
        assert_eq!(recovered.board.audit_chain.len(), 2);
        assert_eq!(recovered.board.audit_chain[0], failed.board.audit_chain[0]);
        assert!(recovered.board.directories.is_empty());
        assert!(recovered.board.selections.is_empty());
        let final_shape: (String, i64) = fixture
            .connection()
            .query_row(
                "SELECT final_outcome,applied_at FROM chain_post_close_board_kind_finals
             WHERE kind='Concept'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(final_shape, ("Error".to_owned(), at(62_000_000)));
        assert_original_audits(fixture.connection());
        assert_terminal_wire(fixture.connection(), &terminal_detail);
        assert_server_requests(&server.snapshot());
        if let Some(first) = &first_recovery_facts {
            assert_eq!(
                &recovered, first,
                "TEST_CODE final replay must not rewrite any facts"
            );
            assert_eq!(Some(&reason), first_recovery_reason.as_ref());
        } else {
            first_recovery_facts = Some(recovered);
            first_recovery_reason = Some(reason);
        }
    }
    assert_server_requests(&server.finish().await);
}
