use super::*;
use crate::grpc_client::client::board_loopback_fixture::spawn_custom_provider_board_loopback;
use crate::grpc_client::pb::magic::market::v1::QueryResponse;
use crate::market_domain::ProviderId;
use prost::Message as _;

#[tokio::test]
async fn successful_custom_provider_survives_v5_and_v6_prepare_and_reopen() {
    for layout in [5, 6] {
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
        if layout == 6 {
            assert_eq!(
                fixture
                    .chain_post_close()
                    .migrate_schema_v5_to_v6()
                    .unwrap()
                    .schema_version(),
                6
            );
        }
        assert_eq!(
            fixture
                .chain_post_close()
                .verify_schema()
                .unwrap()
                .schema_version(),
            layout
        );

        let (client, server) = spawn_custom_provider_board_loopback().await;
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
        let mut intent_id = None;
        let mut head = None;
        let mut original_facts = None;
        let mut original_audits = None;
        let mut original_observation = None;

        for reopened in [false, true] {
            if reopened {
                fixture.reopen();
            }
            let mut local = fixture
                .store
                .as_mut()
                .unwrap()
                .single_user_local_chain_post_close(&config)
                .unwrap();
            let lease = if reopened {
                local
                    .resume_run(
                        intent_id.as_ref().unwrap(),
                        lease_request(
                            "TEST_CODE_CUSTOM_PROVIDER_OWNER_B",
                            60_001_000,
                            120_000_000,
                            head,
                        ),
                    )
                    .unwrap()
            } else {
                let context = build_single_user_local_chain_post_close_context(
                    &config,
                    run_input(&format!("TEST_CODE_RUN_CUSTOM_PROVIDER_V{layout}")),
                )
                .unwrap();
                local
                    .acquire_run(
                        context,
                        fixed_input(stocks.clone()),
                        lease_request("TEST_CODE_CUSTOM_PROVIDER_OWNER_A", 1_000, 60_000_000, None),
                    )
                    .unwrap()
            };
            intent_id = Some(lease.intent_id().clone());
            let provider = cluster_tests::PanicRawProvider {
                calls: Cell::new(0),
            };
            let clock = ControlledClock::new(at(if reopened { 60_001_100 } else { 1_100 }));
            let configuration = FixedClusterConfiguration::resolve(Some("2"));
            let mut io = if layout == 5 {
                local.board_preparation_io(lease, &provider, &clock, configuration, &source)
            } else {
                local.board_preparation_io_v6(lease, &provider, &clock, configuration, &source)
            }
            .expect("TEST_CODE layout-specific board adapter");
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
            .expect("TEST_CODE custom-provider prepare timeout")
            .expect_err("TEST_CODE custom-provider prepare stops at Positions");
            assert_candidates_completed_before_positions(
                &error,
                &expected_directory,
                &expected_selected,
            );
            let failure = error.downcast_ref::<PreparationFailure>().unwrap();
            assert_eq!(failure.board_evidence().len(), 2);
            for (index, evidence) in failure.board_evidence().iter().enumerate() {
                assert_eq!(evidence.status(), &SourceStatus::Available);
                assert_eq!(evidence.provider(), Some(ProviderId::Custom));
                assert_eq!(evidence.source(), Some("TEST_CODE_LOOPBACK_BOARD_SOURCE"));
                assert_eq!(evidence.batch_id(), Some(BOARD_BATCH_IDS[index]));
                assert_eq!(evidence.source_at(), Some("2026-07-21T15:30:00+08:00"));
                assert_eq!(evidence.observed_at(), Some("2026-07-21T15:31:00+08:00"));
            }
            assert_eq!(provider.calls.get(), 0);
            drop(io);

            let recovery = local
                .inspect_board_directory(intent_id.as_ref().unwrap())
                .unwrap();
            assert_eq!(recovery.board_directory(), &expected_directory);
            assert_eq!(recovery.selected_board_codes(), &expected_selected);
            assert_eq!(recovery.attempts().len(), 3);
            assert!(recovery
                .attempts()
                .iter()
                .all(|attempt| attempt.is_confirmed()));
            assert_eq!(recovery.directories().len(), 2);
            let mut receipts = Vec::new();
            for (index, directory) in recovery.directories().iter().enumerate() {
                assert_eq!(
                    directory.kind(),
                    [BoardKind::Industry, BoardKind::Concept][index]
                );
                // Independent JSON oracle over the public recovery bytes; never call the mapper.
                let fact: serde_json::Value =
                    serde_json::from_slice(directory.fact_bytes()).unwrap();
                assert_eq!(fact["schema_version"], 1);
                let available = &fact["outcome"]["Available"];
                assert_eq!(available["evidence"]["provider"], "Custom");
                assert_eq!(available["evidence"]["batch_id"], BOARD_BATCH_IDS[index]);
                let records = available["records"].as_array().unwrap();
                assert_eq!(records.len(), 2);
                for record in records {
                    assert_eq!(record["evidence"]["provider"], "Custom");
                    assert_eq!(record["evidence"]["batch_id"], BOARD_BATCH_IDS[index]);
                }
                let receipt = directory.receipt();
                assert_eq!(receipt.audit_id, if index == 0 { 1 } else { 2 });
                assert_eq!(receipt.current_outcome, "available");
                assert_eq!(
                    receipt.previous_outcome.as_deref(),
                    if index == 0 { None } else { Some("available") }
                );
                receipts.push(receipt.clone());
            }
            // The successful raw response itself must also retain the admitted provider.
            for (index, attempt) in recovery.attempts()[1..].iter().enumerate() {
                let mut facts = serde_json::Deserializer::from_slice(attempt.fact_bytes())
                    .into_iter::<serde_json::Value>();
                let request = facts.next().expect("stored request material").unwrap();
                let result = facts.next().expect("stored result material").unwrap();
                assert!(facts.next().is_none(), "exactly two stored JSON values");
                assert_eq!(request["schema_version"], 1);
                assert_eq!(request["request_id"].as_str(), Some(attempt.request_id()));
                assert_eq!(result["schema_version"], 1);
                let wire: Vec<u8> =
                    serde_json::from_value(result["response_wire"].clone()).unwrap();
                let response = QueryResponse::decode(wire.as_slice()).unwrap();
                assert_eq!(response.selected_provider, "Custom");
                assert_eq!(response.batch_id, BOARD_BATCH_IDS[index]);
                assert_eq!(
                    attempt.payload_bytes(),
                    Some([INDUSTRY_DIRECTORY_BYTES, CONCEPT_DIRECTORY_BYTES][index])
                );
            }
            head = Some(
                local
                    .inspect_run(intent_id.as_ref().unwrap())
                    .unwrap()
                    .head_version(),
            );
            drop(recovery);
            drop(local);

            assert_eq!(fixture.count("data_acquisition_audit"), 2);
            assert_eq!(fixture.count("data_acquisition_audit_chain"), 2);
            let transaction = fixture.connection().unchecked_transaction().unwrap();
            for (index, receipt) in receipts.iter().enumerate() {
                let verified = read_acquisition_in_transaction(&transaction, receipt)
                    .expect("TEST_CODE original full-chain BR159 reader");
                let record = verified.record();
                assert_eq!(record.capability, "board-directory");
                assert_eq!(record.provider, "Custom");
                assert_eq!(record.source, "TEST_CODE_LOOPBACK_BOARD_SOURCE");
                assert_eq!(record.request_hash, BOARD_REQUEST_HASHES[index]);
                assert_eq!(record.source_at, Some("2026-07-21T15:30:00+08:00"));
                assert_eq!(record.observed_at, "2026-07-21T15:31:00+08:00");
                assert_eq!(record.batch_id, Some(BOARD_BATCH_IDS[index]));
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
            }
            transaction.rollback().unwrap();
            let facts = board_durable_facts(fixture.connection());
            let audits = audit_snapshot(&fixture);
            let observation = server.snapshot();
            assert_eq!(observation.requests.len(), 3);
            assert_eq!(observation.non_board_requests, 0);
            assert_eq!(
                observation
                    .requests
                    .iter()
                    .map(|request| request.kind.as_str())
                    .collect::<Vec<_>>(),
                ["Industry", "Industry", "Concept"]
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
                observation.requests[0].request_id,
                observation.requests[1].request_id
            );
            assert_ne!(
                observation.requests[1].request_id,
                observation.requests[2].request_id
            );
            if reopened {
                assert_eq!(Some(&facts), original_facts.as_ref());
                assert_eq!(Some(&audits), original_audits.as_ref());
                assert_eq!(Some(&observation), original_observation.as_ref());
            } else {
                original_facts = Some(facts);
                original_audits = Some(audits);
                original_observation = Some(observation);
            }
        }
        drop(source);
        let final_observation = server.finish().await;
        assert_eq!(Some(&final_observation), original_observation.as_ref());
    }
}
