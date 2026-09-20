use super::*;

struct ErrorMaterialCommitClock {
    base: TerminalErrorCommitClock,
    locked_head: Cell<Option<u64>>,
    locked_facts: RefCell<Option<ErrorFacts>>,
    fallback_sample: Cell<Option<i64>>,
    after_arm_calls: Cell<u32>,
}

impl ErrorMaterialCommitClock {
    fn new(database: &std::path::Path) -> Self {
        Self {
            base: TerminalErrorCommitClock::new(database),
            locked_head: Cell::new(None),
            locked_facts: RefCell::new(None),
            fallback_sample: Cell::new(None),
            after_arm_calls: Cell::new(0),
        }
    }
}

impl ConceptEffectClock for ErrorMaterialCommitClock {
    fn now(&self) -> UtcMicros {
        if self.base.armed.get() {
            // Observation only, never the fault trigger.
            self.after_arm_calls.set(self.after_arm_calls.get() + 1);
            return UtcMicros::try_new(at(FALLBACK_OFFSET_US)).unwrap();
        }
        let reader = self.base.reader.borrow();
        let reader = reader.as_ref().unwrap();
        let raw_a_without_b: bool = reader
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
             ) AND NOT EXISTS(
                 SELECT 1 FROM chain_post_close_board_error_materials WHERE kind='Concept'
             ) AND NOT EXISTS(
                 SELECT 1 FROM chain_post_close_board_kind_finals WHERE kind='Concept'
             )",
                [],
                |row| row.get(0),
            )
            .unwrap();
        if !raw_a_without_b {
            return UtcMicros::try_new(at(RAW_OFFSET_US)).unwrap();
        }
        // Already-open FD only: hold a DELETE-journal read lock across the real B COMMIT.
        reader.execute_batch("BEGIN DEFERRED;").unwrap();
        self.locked_head.set(Some(run_head(reader)));
        *self.locked_facts.borrow_mut() = Some(error_facts(reader));
        self.fallback_sample.set(Some(at(FALLBACK_OFFSET_US)));
        self.base.armed.set(true);
        UtcMicros::try_new(at(FALLBACK_OFFSET_US)).unwrap()
    }
}

fn assert_candidates_control_stop(error: &anyhow::Error) {
    let failure = error.downcast_ref::<PreparationFailure>().unwrap();
    assert_eq!(failure.stage(), PreparationStage::Candidates);
    assert_eq!(
        failure.completed_stages().last(),
        Some(&PreparationStage::ClusterWritesAndLifecycle),
    );
    assert!(failure.board_directory().is_empty());
    assert!(failure.candidate_board_codes().is_empty());
    assert!(failure.board_evidence().is_empty());
    assert!(failure.positions().is_empty());
    assert!(failure.lhb_map().is_empty());
}

fn assert_raw_a_without_b(connection: &Connection, expected_detail: &[u8]) -> u64 {
    let facts = error_facts(connection);
    assert_eq!(facts.board.begins.len(), 3);
    assert_eq!(facts.board.results.len(), 3);
    assert_eq!(facts.status_materials.len(), 2);
    assert!(facts.error_materials.is_empty());
    assert_eq!(facts.board.finals.len(), 1);
    assert_eq!(facts.board.audits.len(), 1);
    assert_eq!(facts.board.audit_chain.len(), 1);
    assert!(facts.board.directories.is_empty());
    assert!(facts.board.selections.is_empty());
    assert_terminal_wire(connection, expected_detail);
    let (bytes, captured_at, version, parent_version, prior_head): (Vec<u8>, i64, i64, i64, i64) =
        connection
            .query_row(
                "SELECT safety.material_bytes,safety.captured_at,safety.run_version,
                    result.run_version,safety.prior_head_version
             FROM chain_post_close_board_status_materials AS safety
             JOIN chain_post_close_board_attempt_results AS result
               ON result.intent_id=safety.intent_id AND result.kind=safety.kind
              AND result.attempt_ordinal=safety.attempt_ordinal
              AND result.run_version=safety.result_run_version
              AND result.result_sha256=safety.result_sha256
             WHERE safety.kind='Concept' AND safety.provenance='Captured'",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .unwrap();
    assert_eq!(
        bytes,
        br#"{"schema_version":1,"projection_version":1,"safe_diagnostic":"[redacted-unclassified-status]"}"#,
    );
    assert_eq!(captured_at, at(RAW_OFFSET_US));
    assert_eq!(prior_head, parent_version);
    assert_eq!(version, parent_version + 1);
    u64::try_from(version).unwrap()
}

// Reuse the original full BR159 reader; this phase has only Industry, not the parent's two rows.
fn assert_only_industry_audit(connection: &Connection) {
    let receipt = connection
        .query_row(
            "SELECT audit_id,audit_record_hash,previous_outcome,current_outcome
         FROM chain_post_close_board_kind_finals WHERE kind='Industry'",
            [],
            |row| {
                Ok(DataAcquisitionAuditReceipt {
                    audit_id: row.get(0)?,
                    record_hash: row.get(1)?,
                    previous_outcome: row.get(2)?,
                    current_outcome: row.get(3)?,
                })
            },
        )
        .unwrap();
    assert_eq!(receipt.audit_id, 1);
    assert_eq!(receipt.previous_outcome, None);
    assert_eq!(receipt.current_outcome, "available");
    let transaction = connection.unchecked_transaction().unwrap();
    let verified = read_acquisition_in_transaction(&transaction, &receipt).unwrap();
    let record = verified.record();
    assert_eq!(record.capability, "board-directory");
    assert_eq!(record.provider, "Tdx");
    assert_eq!(record.source, "TEST_CODE_LOOPBACK_BOARD_SOURCE");
    assert_eq!(record.request_hash, BOARD_REQUEST_HASHES[0]);
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
    transaction.rollback().unwrap();
}

#[tokio::test]
async fn unconfirmed_error_material_commit_preserves_raw_a_and_stops_reopen_without_rpc() {
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
    fixture.connection().busy_timeout(Duration::ZERO).unwrap();
    let clock = ErrorMaterialCommitClock::new(&fixture.database());
    let (client, server) = spawn_board_terminal_error_loopback().await;
    let source = GrpcSource::from_board_loopback_test_client(client);
    let stocks = cluster_tests::cluster_stocks();
    let config = local_config(BUILD_A);
    let context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_RUN_ERROR_MATERIAL_COMMIT"),
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
            lease_request("TEST_CODE_B_COMMIT_OWNER_A", 1_000_000, 60_000_000, None),
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
    .expect("TEST_CODE B COMMIT prepare timeout")
    .expect_err("TEST_CODE actual error-material COMMIT must fail");
    assert!(
        clock.base.armed.get(),
        "TEST_CODE must reach raw+A-confirmed B COMMIT"
    );
    assert_eq!(clock.fallback_sample.get(), Some(at(FALLBACK_OFFSET_US)));
    assert_eq!(clock.after_arm_calls.get(), 0);
    assert!(matches!(error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::ResultUnconfirmed { intent_id: stopped })
            if stopped.as_str() == intent_id.as_str()));
    assert!(matches!(
        error.downcast_ref::<ChainPostCloseError>(),
        Some(ChainPostCloseError::StorageFailed {
            operation: "commit"
        })
    ));
    assert_candidates_control_stop(&error);
    assert_eq!(provider.calls.get(), 0);
    let failed = clock.base.facts();
    assert_eq!(Some(&failed), clock.locked_facts.borrow().as_ref());
    let terminal_detail = server.terminal_error_detail();
    let observation = server.snapshot();
    assert_server_requests(&observation);
    let detail =
        <crate::grpc_client::pb::magic::market::v1::ErrorDetail as prost::Message>::decode(
            terminal_detail.as_slice(),
        )
        .unwrap();
    assert_eq!(detail.request_id, observation.requests[2].request_id);
    assert_eq!(
        detail.operation,
        crate::grpc_client::pb::magic::market::v1::Operation::BoardDirectory as i32
    );
    assert_eq!(detail.provider, "Tdx");
    assert_eq!(detail.reason_code, "invalid_evidence");
    assert!(!detail.retryable);
    let failed_head = {
        let reader = clock.base.reader.borrow();
        let reader = reader.as_ref().unwrap();
        let status_head = assert_raw_a_without_b(reader, &terminal_detail);
        assert_eq!(run_head(reader), status_head);
        assert_eq!(clock.locked_head.get(), Some(status_head));
        status_head
    };

    // No lease/capability is returned by the failed B writer; do not regrant either.
    let again = tokio::time::timeout(
        Duration::from_secs(5),
        prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks.clone(),
            None,
            &mut io,
        ),
    )
    .await
    .expect("TEST_CODE consumed-adapter prepare timeout")
    .expect_err("TEST_CODE consumed adapter must stop before effects");
    assert!(matches!(again.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::AuthorityRejected { intent_id: stopped })
            if stopped.as_str() == intent_id.as_str()));
    assert!(again.downcast_ref::<PreparationFailure>().is_none());
    assert_eq!(clock.after_arm_calls.get(), 0);
    assert_eq!(clock.base.facts(), failed);
    assert_eq!(server.snapshot(), observation);
    assert_eq!(provider.calls.get(), 0);
    drop(io);
    clock.base.release(); // rollback first, only then close this FD
    assert_eq!(
        local.inspect_run(&intent_id).unwrap().head_version(),
        failed_head
    );
    assert!(local
        .inspect_board_error_material(&intent_id, BoardKind::Concept)
        .unwrap()
        .is_none());
    drop(local);

    // Fresh SELECTs after the lock release must see full rollback, not the old reader snapshot.
    assert_eq!(error_facts(fixture.connection()), failed);
    assert_eq!(
        assert_raw_a_without_b(fixture.connection(), &terminal_detail),
        failed_head
    );
    assert_eq!(run_head(fixture.connection()), failed_head);
    assert_only_industry_audit(fixture.connection());
    fixture.reopen();
    assert_eq!(error_facts(fixture.connection()), failed);
    assert_eq!(run_head(fixture.connection()), failed_head);
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
                "TEST_CODE_B_COMMIT_OWNER_B",
                61_000_000,
                120_000_000,
                Some(failed_head),
            ),
        )
        .unwrap();
    let resumed_head = lease.head_version();
    assert_eq!(resumed_head, failed_head + 1); // resume, not a business-material confirmation
    assert_eq!(local.inspect_run(&intent_id).unwrap().lease_generation(), 2);
    let recovery_clock = ControlledClock::new(at(62_000_000));
    let mut io = local
        .board_preparation_io_v6(
            lease,
            &provider,
            &recovery_clock,
            FixedClusterConfiguration::resolve(Some("2")),
            &source,
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
    .expect("TEST_CODE B-unconfirmed reopen timeout")
    .expect_err("TEST_CODE terminal raw+A without B cannot recover fallback");
    assert!(matches!(reopened.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::IncompleteOnReopen { intent_id: stopped })
            if stopped.as_str() == intent_id.as_str()));
    assert!(matches!(reopened.downcast_ref::<ChainPostCloseError>(),
        Some(ChainPostCloseError::IncompleteEffect { intent_id: stopped })
            if stopped.as_str() == intent_id.as_str()));
    assert_candidates_control_stop(&reopened);
    assert_eq!(provider.calls.get(), 0);
    drop(io);
    assert_eq!(
        local.inspect_run(&intent_id).unwrap().head_version(),
        resumed_head
    );
    assert!(local
        .inspect_board_error_material(&intent_id, BoardKind::Concept)
        .unwrap()
        .is_none());
    drop(local);
    assert_eq!(error_facts(fixture.connection()), failed);
    assert_eq!(
        assert_raw_a_without_b(fixture.connection(), &terminal_detail),
        failed_head
    );
    assert_only_industry_audit(fixture.connection());
    assert_eq!(server.snapshot(), observation);
    drop(source);
    assert_eq!(server.finish().await, observation);
}
