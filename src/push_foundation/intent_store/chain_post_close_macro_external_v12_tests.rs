use super::*;
use crate::grpc_client::external_pb::magic::market::v1::QueryRequest;
use crate::push_foundation::intent_store::chain_post_close::{
    macro_codec,
    macro_native::{DataBegin, DataResult, ExpiryBasis, FinalKind, QueryTerminal, TerminalCause},
};
use crate::search_service::macro_news::{runner::QueryKey, NativeOutcome};
use prost::Message as _;

const EXPECTED_ERROR_MACRO: &str = concat!(
    "## 📡 今日宏观 / 市场背景（2026年09月14日）\n\n",
    "### 📰 东方财富财经要闻\n",
    "- 数据不可用：reason_code=invalid_evidence retryable=false\n\n",
    "### 🧭 财联社电报\n",
    "- 数据不可用：reason_code=invalid_evidence retryable=false\n\n",
    "### 📣 金十快讯\n",
    "- 数据不可用：reason_code=invalid_evidence retryable=false\n\n",
    "### 🌐 澎湃财经\n",
    "- 数据不可用：reason_code=invalid_evidence retryable=false\n\n",
    "### 📊 最新经济数据发布（金十）\n",
    "- 数据不可用：reason_code=no_verified_batch retryable=false",
);

fn attempt_begin_bytes(
    connection: &rusqlite::Connection,
    intent: &IntentId,
    gateway: u8,
) -> Vec<u8> {
    connection
        .query_row(
            "SELECT bytes FROM chain_post_close_macro_attempt_begins \
             WHERE intent_id=?1 AND phase='Gateway' AND item_ordinal=?2 \
               AND candidate_ordinal=1 AND attempt_ordinal=1",
            rusqlite::params![intent.as_str(), gateway],
            |row| row.get(0),
        )
        .unwrap()
}

fn attempt_result_bytes(
    connection: &rusqlite::Connection,
    intent: &IntentId,
    gateway: u8,
) -> Vec<u8> {
    connection
        .query_row(
            "SELECT bytes FROM chain_post_close_macro_attempt_results \
             WHERE intent_id=?1 AND phase='Gateway' AND item_ordinal=?2 \
               AND candidate_ordinal=1 AND attempt_ordinal=1",
            rusqlite::params![intent.as_str(), gateway],
            |row| row.get(0),
        )
        .unwrap()
}

fn query_terminal_bytes(
    connection: &rusqlite::Connection,
    intent: &IntentId,
    gateway: u8,
) -> Vec<u8> {
    connection
        .query_row(
            "SELECT bytes FROM chain_post_close_macro_query_terminals \
             WHERE intent_id=?1 AND phase='Gateway' AND item_ordinal=?2 \
               AND candidate_ordinal=1",
            rusqlite::params![intent.as_str(), gateway],
            |row| row.get(0),
        )
        .unwrap()
}

fn assert_models_stop(error: &anyhow::Error) {
    assert!(
        matches!(
            error.downcast_ref::<PreparationStop>(),
            Some(PreparationStop::StageNotMigrated {
                next: UnmigratedStage::ModelsSearchAndReport
            })
        ),
        "TEST_CODE v12 External prepare must complete Macro before Models: {error:?}"
    );
    let failure = error.downcast_ref::<PreparationFailure>().unwrap();
    assert_eq!(failure.stage(), PreparationStage::ModelsSearchAndReport);
    assert_eq!(
        failure.completed_stages().last(),
        Some(&PreparationStage::Macro)
    );
    assert_eq!(
        failure.macro_context().as_bytes(),
        EXPECTED_ERROR_MACRO.as_bytes()
    );
    assert_eq!(
        failure.lhb_map()["TEST_CODE_600001"].to_bits(),
        12.5_f64.to_bits()
    );
}

#[tokio::test(flavor = "current_thread")]
async fn single_user_external_v12_macro_provider_attempts_survive_complete_stage_and_true_reopen() {
    let mut business = V2BusinessFixture::new();
    let mut parent_server = None;
    let mut external_server = None;
    let body = std::panic::AssertUnwindSafe(tokio::time::timeout(
        Duration::from_secs(120),
        async {
            let baseline = control_tests::setup_external_parent(
                &mut business,
                &mut parent_server,
                "TEST_CODE_EXTERNAL_V12_ATTEMPTS_RUN",
            )
            .await;
            assert_eq!(
                business
                    .chain_post_close()
                    .migrate_schema_v11_to_v12()
                    .unwrap()
                    .schema_version(),
                12
            );
            external_server = Some(
                ExternalMtlsMacroFixture::bind_data_provider_attempts_for_test(false)
                    .await
                    .expect("TEST_CODE v12 attempts mTLS fixture"),
            );
            let external = external_server
                .as_ref()
                .expect("TEST_CODE v12 attempts fixture owner");
            let macro_source = GrpcSource::from_external_macro_bundle_for_test(
                external.bundle_path().to_path_buf(),
            );
            let search = macro_search_service(&[]);
            let database = business.database();
            let started_at = micros("2026-09-14T15:31:00+08:00");
            let clock = MacroClock {
                now: Cell::new(UtcMicros::try_new(started_at).unwrap()),
                observation: DateTime::parse_from_rfc3339("2026-09-14T15:31:00+08:00")
                    .unwrap(),
                observation_calls: Cell::new(0),
            };
            let audit_before = business.count("data_acquisition_audit");
            let parent_network_before = parent_server
                .as_ref()
                .unwrap()
                .snapshot_with_tcp_for_test();
            let parent_membership_before =
                parent_server.as_ref().unwrap().membership_snapshot();

            let mut local = business
                .store
                .as_mut()
                .unwrap()
                .single_user_local_chain_post_close(&baseline.config)
                .unwrap();
            let lease = local
                .resume_run(
                    &baseline.intent,
                    macro_lease(
                        "TEST_CODE_EXTERNAL_V12_ATTEMPTS_OWNER",
                        started_at,
                        started_at + 60_000_000,
                        baseline.head,
                    ),
                )
                .unwrap();
            let generation = lease.generation();
            let mut io = local
                .macro_preparation_io_v12(
                    lease,
                    &baseline.queries,
                    &clock,
                    FixedClusterConfiguration::resolve(Some("2")),
                    &baseline.source,
                    &macro_source,
                    &search,
                )
                .unwrap();
            let mut prepared = Box::pin(prepare_chain_analysis_with_io(
                NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
                baseline.stocks.clone(),
                None,
                &mut io,
            ));
            let paced_at = tokio::time::Instant::now();
            let advance_clock = || {
                clock.now.set(
                    UtcMicros::try_new(
                        started_at
                            + i64::try_from(paced_at.elapsed().as_micros()).unwrap(),
                    )
                    .unwrap(),
                );
            };

            let control_deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                tokio::select! {
                    biased;
                    result = &mut prepared => panic!(
                        "TEST_CODE v12 attempts returned before Health receipt: {result:?}"
                    ),
                    _ = tokio::task::yield_now() => {}
                }
                advance_clock();
                if external.snapshot().health_requests.len() == 1 {
                    break;
                }
                assert!(
                    std::time::Instant::now() < control_deadline,
                    "TEST_CODE v12 attempts Health receipt watchdog"
                );
            }
            external.release_health();
            loop {
                tokio::select! {
                    biased;
                    result = &mut prepared => panic!(
                        "TEST_CODE v12 attempts returned before Capabilities receipt: {result:?}"
                    ),
                    _ = tokio::task::yield_now() => {}
                }
                advance_clock();
                if external.snapshot().capabilities_calls == 1 {
                    break;
                }
                assert!(
                    std::time::Instant::now() < control_deadline,
                    "TEST_CODE v12 attempts Capabilities receipt watchdog"
                );
            }
            external.release_capabilities();

            let providers = ["Eastmoney", "Cailianpress", "Jin10", "ThePaper"];
            let data_deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                tokio::select! {
                    biased;
                    result = &mut prepared => panic!(
                        "TEST_CODE v12 attempts returned before four data requests: {result:?}"
                    ),
                    _ = tokio::time::sleep(Duration::from_millis(10)) => {}
                }
                advance_clock();
                if external.snapshot().data_calls >= providers.len() {
                    break;
                }
                assert!(
                    std::time::Instant::now() < data_deadline,
                    "TEST_CODE v12 attempts four-data receipt watchdog"
                );
            }
            let held = external.snapshot();
            assert_eq!(held.data_calls, 4);
            assert_eq!(held.data_requests.len(), 4);
            assert!(held.data_statuses.is_empty());
            let mut requests_by_provider = std::collections::BTreeMap::new();
            let mut request_ids = std::collections::BTreeSet::new();
            for request_bytes in &held.data_requests {
                let request = QueryRequest::decode(request_bytes.as_slice()).unwrap();
                assert_eq!(request.encode_to_vec().as_slice(), request_bytes.as_slice());
                let request_id = request.context.as_ref().unwrap().request_id.clone();
                assert!(request_ids.insert(request_id.clone()));
                assert_eq!(
                    request.payload.as_ref().unwrap().schema,
                    "magic.market.global_news.request"
                );
                assert!(requests_by_provider
                    .insert(
                        request.preferred_provider,
                        (request_id, request_bytes.clone()),
                    )
                    .is_none());
            }
            assert_eq!(requests_by_provider.len(), providers.len());
            assert!(providers
                .iter()
                .all(|provider| requests_by_provider.contains_key(*provider)));
            for _ in 0..providers.len() {
                external.release_data();
            }

            let stopped = loop {
                tokio::select! {
                    result = &mut prepared => break result.expect_err(
                        "TEST_CODE v12 attempts Models stage remains guarded"
                    ),
                    _ = tokio::time::sleep(Duration::from_millis(10)) => {
                        advance_clock();
                    }
                }
            };
            assert_models_stop(&stopped);
            drop(prepared);
            drop(io);

            let wire = external.snapshot();
            assert_eq!(wire.tcp_accepts, 1);
            assert_eq!(wire.health_requests.len(), 1);
            assert_eq!(wire.health_authorized, [true]);
            assert_eq!(wire.health_responses.len(), 1);
            assert!(wire.health_statuses.is_empty());
            assert_eq!(wire.capabilities_calls, 1);
            assert_eq!(wire.capabilities_authorized, [true]);
            assert_eq!(wire.capabilities_requests.len(), 1);
            assert_eq!(
                wire.capabilities_responses,
                [expected_capabilities(
                    &crate::grpc_client::external_pb::magic::market::v1::CapabilitiesRequest::decode(
                        wire.capabilities_requests[0].as_slice(),
                    )
                    .unwrap()
                    .context
                    .unwrap()
                    .request_id,
                )]
            );
            assert!(wire.capabilities_statuses.is_empty());
            assert_eq!(wire.data_calls, 4);
            assert_eq!(wire.data_methods, ["global_news"; 4]);
            assert_eq!(wire.data_authorized, [true; 4]);
            assert!(wire.data_responses.is_empty());
            assert_eq!(wire.data_statuses.len(), 4);
            let mut statuses_by_id = std::collections::BTreeMap::new();
            for status in &wire.data_statuses {
                let detail = ErrorDetail::decode(status.details.as_slice()).unwrap();
                assert!(statuses_by_id
                    .insert(detail.request_id.clone(), (status.clone(), detail))
                    .is_none());
            }
            assert_eq!(
                statuses_by_id.keys().cloned().collect::<std::collections::BTreeSet<_>>(),
                request_ids
            );

            let completed = local.inspect_macro(&baseline.intent).unwrap();
            assert!(completed.is_complete());
            assert!(!completed.has_unconfirmed_effect());
            assert!(completed.pending_source_identities().is_empty());
            assert!(completed.pending_research_queries().is_empty());
            assert_eq!(completed.parent_final_bytes(), baseline.final_bytes);
            assert_eq!(completed.plan().started_at().get(), started_at);
            assert_eq!(completed.plan().deadline_at().get(), started_at + 15_000_000);
            assert!(completed.plan().research_providers().is_empty());
            assert_eq!(completed.readiness_episodes().len(), 1);
            let episode = &completed.readiness_episodes()[0];
            let ready_version = episode.ready_result_version().unwrap();
            assert!(episode
                .controls()
                .iter()
                .all(|control| control.outcome() == Some(MacroControlOutcome::Ready)));
            assert_eq!(completed.attempts().len(), 4);

            let fact_reader = rusqlite::Connection::open(&database).unwrap();
            let mut durable = Vec::new();
            let mut audit_ids = std::collections::BTreeSet::new();
            for (index, provider) in providers.into_iter().enumerate() {
                let gateway = u8::try_from(index + 1).unwrap();
                let key = QueryKey::Gateway(gateway);
                let (request_id, request_bytes) = requests_by_provider.get(provider).unwrap();
                let request = QueryRequest::decode(request_bytes.as_slice()).unwrap();
                assert_eq!(
                    request.context.as_ref().unwrap().request_id.as_str(),
                    request_id
                );
                assert_eq!(request.preferred_provider, provider);

                let (status, detail) = statuses_by_id.get(request_id).unwrap();
                assert_eq!(status.code, tonic::Code::FailedPrecondition as i32);
                assert_eq!(status.trailer, ObservedHealthTrailer::Absent);
                assert_eq!(detail.encode_to_vec(), status.details);
                assert_eq!(detail.request_id.as_str(), request_id);
                assert_eq!(detail.operation, Operation::GlobalNews as i32);
                assert_eq!(detail.provider, "Eastmoney");
                assert_eq!(detail.reason_code, "invalid_evidence");
                assert!(!detail.retryable);
                assert_eq!(detail.admission, AdmissionState::Admitted as i32);
                assert_eq!(
                    detail.provider_attempts.as_slice(),
                    [
                        ProviderAttemptDetail {
                            ordinal: 1,
                            provider: "Eastmoney".to_owned(),
                            outcome: "rejected".to_owned(),
                            reason_code: "query_rejected".to_owned(),
                            retryable: false,
                            terminal: false,
                        },
                        ProviderAttemptDetail {
                            ordinal: 2,
                            provider: "Eastmoney".to_owned(),
                            outcome: "failed".to_owned(),
                            reason_code: "unavailable".to_owned(),
                            retryable: true,
                            terminal: false,
                        },
                        ProviderAttemptDetail {
                            ordinal: 3,
                            provider: "Eastmoney".to_owned(),
                            outcome: "selected".to_owned(),
                            reason_code: "selected".to_owned(),
                            retryable: false,
                            terminal: false,
                        },
                    ]
                );

                let attempt = completed
                    .attempts()
                    .iter()
                    .find(|attempt| attempt.query_key() == key)
                    .expect("TEST_CODE each v12 gateway has one recovered attempt");
                assert_eq!(attempt.attempt_ordinal(), 1);
                assert_eq!(attempt.request_bytes(), request_bytes);
                assert_eq!(attempt.request_id(), request_id);
                assert_eq!(attempt.readiness_result_version(), Some(ready_version));
                assert!(attempt.result_version().is_some());
                assert_eq!(attempt.response_bytes(), None);
                assert_eq!(attempt.continuation(), Some(MacroContinuation::Terminal));
                let material = attempt.result_material().unwrap();
                assert_eq!(material.diagnostic, Some("[redacted-unclassified-status]"));
                assert_eq!(material.retry_decision, RetryDecision::NoRetry);
                assert_eq!(material.continuation, MacroContinuation::Terminal);
                match material.wire {
                    MacroRecoveredWire::Status {
                        code,
                        details,
                        trailer: MacroRecoveredTrailer::Absent,
                    } => {
                        assert_eq!(code, tonic::Code::FailedPrecondition as i32);
                        assert_eq!(details, status.details);
                    }
                    _ => panic!("TEST_CODE v12 attempt must retain exact status material"),
                }
                assert_historical_provider_attempts(
                    material
                        .provider_attempts()
                        .expect("TEST_CODE v12 writer must retain accepted attempts"),
                    provider,
                );

                let terminal = completed.query_terminal(key).unwrap();
                assert_eq!(terminal.query_key(), key);
                assert!(terminal.was_called());
                assert_eq!(terminal.owner(), "TEST_CODE_EXTERNAL_V12_ATTEMPTS_OWNER");
                assert_eq!(terminal.generation(), generation);
                let receipt = terminal.audit_receipt().unwrap();
                assert!(audit_ids.insert(receipt.audit_id));
                assert_eq!(receipt.current_outcome, "partial");
                match terminal.native() {
                    NativeOutcome::News(Err(error)) => {
                        assert_eq!(error.capability(), "GrpcExternalV1");
                        assert_eq!(error.provider(), Some(crate::market_domain::ProviderId::Eastmoney));
                        assert_eq!(error.audit_outcome(), "partial");
                        assert_eq!(error.reason_code(), "invalid_evidence");
                        assert!(!error.retryable());
                    }
                    other => panic!("TEST_CODE expected v12 native news error: {other:?}"),
                }
                let begin_bytes =
                    attempt_begin_bytes(&fact_reader, &baseline.intent, gateway);
                let begin_value: DataBegin = macro_codec::decode(&begin_bytes).unwrap();
                assert_eq!(begin_value.version, 2);
                assert_eq!(begin_value.query, key);
                assert_eq!(begin_value.attempt, 1);
                assert_eq!(begin_value.readiness_result_version, Some(ready_version));
                assert_eq!(begin_value.previous_result_version, None);

                let result_bytes =
                    attempt_result_bytes(&fact_reader, &baseline.intent, gateway);
                let result_value: DataResult = macro_codec::decode(&result_bytes).unwrap();
                assert_eq!(result_value.version, 2);
                assert_eq!(result_value.query, key);
                assert_eq!(result_value.attempt, 1);
                assert_eq!(result_value.native.as_slice(), terminal.native_bytes());
                let result_json: serde_json::Value =
                    serde_json::from_slice(&result_bytes).unwrap();
                assert_eq!(result_json["version"], 2);
                assert_eq!(result_json["raw"]["version"], 2);
                assert!(result_json["raw"]["external_wire"].is_object());

                let terminal_bytes =
                    query_terminal_bytes(&fact_reader, &baseline.intent, gateway);
                let terminal_value: QueryTerminal =
                    macro_codec::decode(&terminal_bytes).unwrap();
                assert_eq!(terminal_value.version, 2);
                assert_eq!(terminal_value.query, key);
                assert_eq!(terminal_value.plan_version, completed.plan_version());
                assert_eq!(
                    terminal_value.request_plan_version,
                    Some(begin_value.request_plan_version)
                );
                assert_eq!(
                    terminal_value.request_sha256.as_deref(),
                    Some(begin_value.request_sha256.as_str())
                );
                assert_eq!(terminal_value.native_sha256, result_value.native_sha256);
                match &terminal_value.cause {
                    TerminalCause::DataResult { version } => {
                        assert_eq!(Some(*version), attempt.result_version())
                    }
                    _ => panic!("TEST_CODE v12 terminal must link its typed DataResult"),
                }
                durable.push((
                    key,
                    request_bytes.clone(),
                    begin_bytes,
                    result_bytes,
                    terminal_bytes,
                    terminal.native_bytes().to_vec(),
                    receipt.clone(),
                ));
            }
            assert_eq!(request_ids.len(), 4);
            assert_eq!(audit_ids.len(), 4);

            let gateway_five = completed.query_terminal(QueryKey::Gateway(5)).unwrap();
            assert!(!gateway_five.was_called());
            let gateway_five_receipt = gateway_five.audit_receipt().unwrap().clone();
            assert!(audit_ids.insert(gateway_five_receipt.audit_id));
            match gateway_five.native() {
                NativeOutcome::Economic(Err(error)) => {
                    assert_eq!(error.capability(), "GrpcBridge");
                    assert_eq!(error.provider(), None);
                    assert_eq!(error.audit_outcome(), "unavailable");
                    assert_eq!(error.reason_code(), "no_verified_batch");
                    assert!(!error.retryable());
                }
                other => panic!("TEST_CODE expected Gateway5 LocalUnavailable: {other:?}"),
            }
            assert_eq!(audit_ids.len(), 5);
            let (dimension_count, no_eligible_count): (i64, i64) = fact_reader
                .query_row(
                    "SELECT count(*),sum(outcome='NoEligibleProviders') \
                     FROM chain_post_close_macro_dimension_terminals WHERE intent_id=?1",
                    [baseline.intent.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!((dimension_count, no_eligible_count), (6, 6));
            fact_reader.close().unwrap();

            let begin = completed.finalize_begin().unwrap();
            let final_ = completed.stage_final().unwrap();
            assert_eq!(begin.kind(), FinalKind::Complete);
            assert_eq!(begin.expiry(), &ExpiryBasis::None);
            assert!(begin.pending().is_empty());
            assert_eq!(final_.kind(), FinalKind::Complete);
            assert_eq!(final_.output_bytes(), EXPECTED_ERROR_MACRO.as_bytes());
            assert_eq!(final_.version(), final_.finalize_begin_version() + 1);
            assert_eq!(final_.plan_version(), completed.plan_version());
            let plan_bytes = completed.plan_bytes().to_vec();
            let begin_bytes = begin.bytes().to_vec();
            let final_bytes = final_.bytes().to_vec();
            drop(completed);
            drop(local);
            assert_eq!(business.count("data_acquisition_audit"), audit_before + 5);
            assert_eq!(clock.observation_calls.get(), 1);
            assert_eq!(
                parent_server
                    .as_ref()
                    .unwrap()
                    .snapshot_with_tcp_for_test(),
                parent_network_before
            );
            assert_eq!(
                parent_server.as_ref().unwrap().membership_snapshot(),
                parent_membership_before
            );

            let state_before = super::super::v11_migration_tests::DatabaseState::capture(
                business.connection(),
            );
            external.set_reject_new_connections_for_test(true);
            let wire_before = external.snapshot();
            business.reopen();
            let mut reopened_local = business
                .store
                .as_mut()
                .unwrap()
                .single_user_local_chain_post_close(&baseline.config)
                .unwrap();
            let reopened = reopened_local.inspect_macro(&baseline.intent).unwrap();
            let reopened_reader = rusqlite::Connection::open(&database).unwrap();
            assert!(reopened.is_complete());
            assert!(!reopened.has_unconfirmed_effect());
            assert_eq!(reopened.plan_bytes(), plan_bytes);
            assert_eq!(reopened.finalize_begin().unwrap().bytes(), begin_bytes);
            assert_eq!(reopened.stage_final().unwrap().bytes(), final_bytes);
            assert_eq!(
                reopened.stage_final().unwrap().output_bytes(),
                EXPECTED_ERROR_MACRO.as_bytes()
            );
            assert_eq!(reopened.attempts().len(), 4);
            for (
                key,
                request,
                begin_bytes,
                result_bytes,
                terminal_bytes,
                native,
                receipt,
            ) in durable
            {
                let attempt = reopened
                    .attempts()
                    .iter()
                    .find(|attempt| attempt.query_key() == key)
                    .unwrap();
                assert_eq!(attempt.request_bytes(), request);
                assert_historical_provider_attempts(
                    attempt
                        .result_material()
                        .unwrap()
                        .provider_attempts()
                        .unwrap(),
                    "fresh v12 full-loader recovery",
                );
                let QueryKey::Gateway(gateway) = key else {
                    unreachable!()
                };
                assert_eq!(
                    attempt_begin_bytes(&reopened_reader, &baseline.intent, gateway),
                    begin_bytes
                );
                assert_eq!(
                    attempt_result_bytes(&reopened_reader, &baseline.intent, gateway),
                    result_bytes
                );
                assert_eq!(
                    query_terminal_bytes(&reopened_reader, &baseline.intent, gateway),
                    terminal_bytes
                );
                let terminal = reopened.query_terminal(key).unwrap();
                assert_eq!(terminal.native_bytes(), native);
                assert_eq!(terminal.audit_receipt(), Some(&receipt));
            }
            assert_eq!(
                reopened
                    .query_terminal(QueryKey::Gateway(5))
                    .unwrap()
                    .audit_receipt(),
                Some(&gateway_five_receipt)
            );
            drop(reopened);
            reopened_reader.close().unwrap();
            drop(reopened_local);
            assert_eq!(
                super::super::v11_migration_tests::DatabaseState::capture(
                    business.connection()
                ),
                state_before
            );
            assert_eq!(external.snapshot(), wire_before);
            assert_eq!(
                parent_server
                    .as_ref()
                    .unwrap()
                    .snapshot_with_tcp_for_test(),
                parent_network_before
            );
            assert_eq!(
                parent_server.as_ref().unwrap().membership_snapshot(),
                parent_membership_before
            );
        },
    ))
    .catch_unwind()
    .await;

    control_tests::cleanup_external_case(
        &mut business,
        &mut parent_server,
        &mut external_server,
        "v12 External provider attempts complete stage",
    )
    .await;
    drop(business);
    match body {
        Ok(result) => result.expect("TEST_CODE v12 External attempts body deadline"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}
