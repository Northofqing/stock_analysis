use super::*;
use crate::grpc_client::external_pb::magic::market::v1::QueryRequest;
use crate::push_foundation::intent_store::chain_post_close::{
    macro_codec,
    macro_native::{DataBegin, DataResult, ExpiryBasis, FinalKind, QueryTerminal, TerminalCause},
};
use crate::search_service::macro_news::{runner::QueryKey, NativeOutcome};

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
async fn task6_connection_journal_begin_precedes_first_health_and_reopen_is_unknown() {
    assert_task6_connection_journal(0).await;
}

#[tokio::test(flavor = "current_thread")]
async fn task6_connection_journal_result_and_link_precede_capabilities() {
    assert_task6_connection_journal(1).await;
}

#[tokio::test(flavor = "current_thread")]
async fn task6_connection_journal_data_requires_current_capability_and_effect_link() {
    assert_task6_connection_journal(2).await;
}

#[tokio::test(flavor = "current_thread")]
async fn task6_legacy_v11_continuation_qualifies_new_connection_and_finishes_only_original_source()
{
    let mut business = V2BusinessFixture::new();
    let mut parent_server = None;
    let mut external_server = None;
    let body =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(120), async {
            let baseline = control_tests::setup_external_parent(
                &mut business,
                &mut parent_server,
                "TEST_CODE_TASK6_LEGACY_CONTINUATION",
            )
            .await;
            external_server = Some(
                ExternalMtlsMacroFixture::bind_data_success_for_test()
                    .await
                    .unwrap(),
            );
            let external = external_server.as_ref().unwrap();
            let checkpoint = control_recovery_tests::reach_confirmed_health_checkpoint(
                &mut business,
                &baseline,
                external,
                "TEST_CODE_LEGACY_A_OWNER",
            )
            .await;
            business
                .chain_post_close()
                .migrate_schema_v11_to_v12()
                .unwrap();
            business
                .chain_post_close()
                .migrate_schema_v12_to_v13()
                .unwrap();
            business
                .chain_post_close()
                .migrate_schema_v13_to_v14()
                .unwrap();
            business
                .chain_post_close()
                .migrate_schema_v14_to_v15()
                .unwrap();
            business.reopen();
            let source = GrpcSource::from_external_macro_bundle_for_test(
                external.bundle_path().to_path_buf(),
            );
            let started = micros(STARTED_LOCAL);
            let clock = MacroClock {
                now: Cell::new(UtcMicros::try_new(started + 3_000_000).unwrap()),
                observation: DateTime::parse_from_rfc3339(STARTED_LOCAL).unwrap(),
                observation_calls: Cell::new(0),
            };
            let search = macro_search_service(&[]);
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
                        "TEST_CODE_LEGACY_CONTINUE_OWNER",
                        started + 3_000_000,
                        started + 14_000_000,
                        checkpoint.head_version,
                    ),
                )
                .unwrap();
            external.release_health();
            external.release_capabilities();
            external.release_data();
            let continued =
                crate::push_foundation::intent_store::chain_post_close::macro_driver::drive(
                    &mut local,
                    lease,
                    &source,
                    &clock,
                    Rc::new(Cell::new(false)),
                    &search,
                )
                .await;
            assert!(
                continued.is_ok(),
                "verified single-source v11 plan must continue without inventing LocalRoute: {:?}",
                continued.err()
            );
            let recovered = local.inspect_macro(&baseline.intent).unwrap();
            assert!(recovered
                .global_news(GlobalNewsProvider::Eastmoney)
                .is_some());
            assert_eq!(recovered.plan_bytes(), checkpoint.plan_bytes);
            assert_eq!(
                recovered.readiness_episodes()[0].controls()[0].response_bytes(),
                Some(checkpoint.health.response_bytes.as_slice())
            );
            assert_eq!(external.snapshot().health_requests.len(), 2);
            assert_ne!(
                external.snapshot().health_requests[0],
                external.snapshot().health_requests[1]
            );
            assert_eq!(
                external.snapshot().capabilities_requests,
                vec![checkpoint.capabilities.bytes]
            );
            assert_eq!(
                external.snapshot().data_requests,
                vec![checkpoint.data.bytes]
            );
            assert_eq!(external.snapshot().data_methods, vec!["global_news"]);
        }))
        .catch_unwind()
        .await;
    control_tests::cleanup_external_case(
        &mut business,
        &mut parent_server,
        &mut external_server,
        "Task6 legacy continuation",
    )
    .await;
    match body {
        Ok(result) => result.expect("legacy continuation watchdog"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn task6_connection_reopen_health_ready_requires_new_journaled_health_before_business() {
    assert_task6_reopen_qualification(false).await;
}

#[tokio::test(flavor = "current_thread")]
async fn task6_durable_b_legacy_qualified_preserves_a_and_binds_new_results_to_b() {
    assert_task6_durable_b(0).await;
}

#[tokio::test(flavor = "current_thread")]
async fn task6_durable_b_legacy_rejected_sends_zero_business_and_preserves_a() {
    assert_task6_durable_b(1).await;
}

#[tokio::test(flavor = "current_thread")]
async fn task6_durable_b_qualification_unknown_reopen_sends_nothing() {
    assert_task6_durable_b(2).await;
}

#[tokio::test(flavor = "current_thread")]
async fn task6_durable_b_confirmed_retry_preserves_original_identity_due_and_ordinal() {
    assert_task6_durable_b(3).await;
}

#[tokio::test(flavor = "current_thread")]
async fn task6_durable_b_original_business_unknown_blocks_even_new_qualification() {
    assert_task6_durable_b(4).await;
}

pub(super) async fn assert_task6_durable_b(mode: u8) {
    let retry_case = matches!(mode, 3 | 6);
    use crate::grpc_client::build_identity::BuildIdentityTrust;
    use crate::grpc_client::client::external_control_loopback_fixture::{
        ExternalMtlsSwitch, HealthReply,
    };
    use crate::grpc_client::client::GrpcMarketClient;
    use crate::push_foundation::intent_store::chain_post_close::macro_driver;
    let mut business = V2BusinessFixture::new();
    let mut parent_server = None;
    let mut external_server = None;
    let mut server_b = None;
    let mut switch = None;
    let body = std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(120), async {
        let baseline = control_tests::setup_external_parent(&mut business, &mut parent_server, "TEST_CODE_TASK6_DURABLE_B").await;
        external_server = Some(if retry_case {
            ExternalMtlsMacroFixture::bind_data_retry_then_success_for_test().await.unwrap()
        } else {
            ExternalMtlsMacroFixture::bind_data_success_for_test().await.unwrap()
        });
        let a = external_server.as_ref().unwrap();
        switch = Some(ExternalMtlsSwitch::bind(a.endpoint()).await);
        let front = switch.as_ref().unwrap();
        let mut checkpoint = control_recovery_tests::reach_confirmed_health_checkpoint_at_bundle(
            &mut business, &baseline, a, "TEST_CODE_A_FROZEN_OWNER", front.bundle_path(),
        ).await;
        if mode == 4 {
            // A true v11 committed Begin has no result. Whether or not its
            // future was polled is unknowable after restart; never reissue it.
            let started = micros(STARTED_LOCAL);
            let prepared = GrpcMarketClient::prepare_client_bundle(front.bundle_path()).unwrap();
            let material = crate::grpc_client::client::external_control_attempt::ExternalControlRequestMaterial {
                kind: crate::grpc_client::client::external_control_attempt::ExternalControlKind::Capabilities,
                request_bytes: checkpoint.capabilities.bytes.clone(),
                request_id: checkpoint.capabilities.id.clone(),
                profile: crate::grpc_client::client::ContractProfile::ExternalV1,
                endpoint_uri: prepared.endpoint_uri().into(),
                acquisition_authority: AUTHORITY.into(),
            };
            let attempt = prepared.resume_capabilities_attempt(material).unwrap();
            let mut local = business.store.as_mut().unwrap().single_user_local_chain_post_close(&baseline.config).unwrap();
            let lease = local.resume_run(&baseline.intent, macro_lease("TEST_CODE_A_UNKNOWN_OWNER", started + 2_000_000, started + 2_500_000, checkpoint.head_version)).unwrap();
            let (lease, _) = local.begin_capabilities_control(lease, &attempt, UtcMicros::try_new(started + 2_000_000).unwrap()).unwrap();
            checkpoint.head_version = local.inspect_run(&baseline.intent).unwrap().head_version();
            drop(lease); drop(local);
        }
        business.chain_post_close().migrate_schema_v11_to_v12().unwrap();
        business.chain_post_close().migrate_schema_v12_to_v13().unwrap();
        business.chain_post_close().migrate_schema_v13_to_v14().unwrap();
        business.chain_post_close().migrate_schema_v14_to_v15().unwrap();
        business.reopen();
        let started = micros(STARTED_LOCAL);
        let search = macro_search_service(&[]);
        let database = business.database();
        let mut frozen_retry = None;
        if retry_case {
            let source_a = GrpcSource::from_external_macro_bundle_for_test(front.bundle_path().to_path_buf());
            let clock_a = MacroClock { now: Cell::new(UtcMicros::try_new(started + 3_000_000).unwrap()),
                observation: DateTime::parse_from_rfc3339(STARTED_LOCAL).unwrap(), observation_calls: Cell::new(0) };
            let mut local = business.store.as_mut().unwrap().single_user_local_chain_post_close(&baseline.config).unwrap();
            let lease = local.resume_run(&baseline.intent, macro_lease("TEST_CODE_A_RETRY_OWNER", started + 3_000_000, started + 3_250_000, checkpoint.head_version)).unwrap();
            a.release_health(); a.release_capabilities(); a.release_data();
            let mut running = Box::pin(macro_driver::drive(&mut local, lease, &source_a, &clock_a, Rc::new(Cell::new(false)), &search));
            loop {
                tokio::select! {
                    result = &mut running => panic!("A returned before retry checkpoint: {:?}; health={}, caps={}, data={}, statuses={}", result.err(), a.snapshot().health_requests.len(), a.snapshot().capabilities_calls, a.snapshot().data_calls, a.snapshot().data_statuses.len()),
                    _ = tokio::task::yield_now() => {},
                }
                if a.snapshot().data_statuses.is_empty() { continue; }
                let (recovered, run) = control_unknown_commit_tests::inspect_at(&database, &baseline.config, &baseline.intent);
                if recovered.attempts().first().is_some_and(|attempt| attempt.result_version().is_some()) {
                    let attempt = &recovered.attempts()[0];
                    assert_eq!(attempt.continuation(), Some(MacroContinuation::Retry { backoff_ms: 1000 }));
                    assert_eq!(attempt.retry_not_before, Some(started + 4_000_000));
                    assert_eq!(attempt.request_id(), checkpoint.data.id);
                    assert_eq!(attempt.request_bytes(), checkpoint.data.bytes);
                    assert_eq!(recovered.plan().first_source_request().retry_policy(), (4, 1000, 60_000, 200));
                    let status = &a.snapshot().data_statuses[0];
                    let expected = ErrorDetail {
                        request_id: checkpoint.data.id.clone(), operation: Operation::GlobalNews as i32,
                        provider: "Eastmoney".into(), reason_code: "no_verified_batch".into(), retryable: true,
                        ..Default::default()
                    };
                    assert_eq!(status.code, tonic::Code::Unavailable as i32);
                    assert_eq!(status.details, expected.encode_to_vec());
                    assert_eq!(status.trailer, ObservedHealthTrailer::Absent);
                    assert!(!recovered.has_unconfirmed_effect());
                    checkpoint.head_version = run.head;
                    frozen_retry = Some(attempt_result_bytes(&rusqlite::Connection::open(&database).unwrap(), &baseline.intent, 1));
                    break;
                }
            }
            drop(running); drop(local);
            business.reopen();
        }
        server_b = Some(ExternalMtlsMacroFixture::bind_health_reply_for_test(if mode == 6 { HealthReply::Success } else { HealthReply::TrustedBuildB }).await.unwrap());
        let b = server_b.as_ref().unwrap();
        if mode == 6 { b.append_zero_length_source_for_test(); }
        else { b.use_descriptor_b_for_test(); }
        front.switch_to(b.endpoint()).await;
        let source = GrpcSource::from_external_macro_bundle_for_test(front.bundle_path().to_path_buf());
        let resumed_at = started + if retry_case { 3_500_000 } else { 3_000_000 };
        let clock = MacroClock { now: Cell::new(UtcMicros::try_new(resumed_at).unwrap()),
            observation: DateTime::parse_from_rfc3339(STARTED_LOCAL).unwrap(), observation_calls: Cell::new(0) };
        let prepare = || {
            let prepared = GrpcMarketClient::prepare_client_bundle(front.bundle_path()).unwrap();
            if mode == 1 || mode == 6 { prepared } else { prepared.with_test_build_trust(BuildIdentityTrust::test_client_b_with_descriptor()) }
        };
        let mut local = business.store.as_mut().unwrap().single_user_local_chain_post_close(&baseline.config).unwrap();
        let lease = local.resume_run(&baseline.intent, macro_lease("TEST_CODE_B_OWNER", resumed_at, started + 14_000_000, checkpoint.head_version)).unwrap();
        if mode == 2 {
            let mut running = Box::pin(macro_driver::drive_test_prepared(&mut local, lease, &source, &clock, Rc::new(Cell::new(false)), &search, prepare()));
            while b.snapshot().health_requests.is_empty() {
                tokio::select! {
                    result = &mut running => panic!("qualification returned before blocked Health: {:?}", result.err()),
                    _ = tokio::task::yield_now() => {},
                }
            }
            drop(running);
            assert!(local.inspect_macro(&baseline.intent).unwrap().has_unconfirmed_effect());
            let head = local.inspect_run(&baseline.intent).unwrap().head_version();
            drop(local);
            business.reopen();
            let mut local = business.store.as_mut().unwrap().single_user_local_chain_post_close(&baseline.config).unwrap();
            clock.now.set(UtcMicros::try_new(started + 14_000_000).unwrap());
            let lease = local.resume_run(&baseline.intent, macro_lease("TEST_CODE_B_UNKNOWN_OWNER", started + 14_000_000, started + 16_000_000, head)).unwrap();
            let rejected = macro_driver::drive_test_prepared(&mut local, lease, &source, &clock, Rc::new(Cell::new(false)), &search, prepare()).await;
            assert!(rejected.is_err());
            assert_eq!(b.snapshot().health_requests.len(), 1);
            assert_eq!(b.snapshot().capabilities_calls, 0);
            assert_eq!(b.snapshot().data_calls, 0);
        } else {
            b.release_health(); b.release_capabilities(); b.release_data();
            let mut running = Box::pin(macro_driver::drive_test_prepared(&mut local, lease, &source, &clock, Rc::new(Cell::new(false)), &search, prepare()));
            if retry_case {
                while b.snapshot().capabilities_responses.is_empty() {
                    tokio::select! {
                        result = &mut running => panic!("B returned before fresh capability: {:?}", result.err()),
                        _ = tokio::task::yield_now() => {},
                    }
                }
                assert_eq!(b.snapshot().data_calls, 0, "qualification cannot erase original remaining backoff");
                let (recovered, _) = control_unknown_commit_tests::inspect_at(&database, &baseline.config, &baseline.intent);
                assert_eq!(recovered.attempts().len(), 1);
                assert_eq!(recovered.attempts()[0].retry_not_before, Some(started + 4_000_000));
                assert_eq!(recovered.plan().deadline_at().get(), started + 15_000_000);
                assert_eq!(b.snapshot().data_calls, 0);
                clock.now.set(UtcMicros::try_new(started + 4_000_000).unwrap());
            }
            let result = running.as_mut().await;
            drop(running);
            if mode == 0 || retry_case {
                assert!(result.is_ok(), "B current qualification must continue original v11 scope: {:?}", result.err());
                assert!(local.inspect_macro(&baseline.intent).unwrap().global_news(GlobalNewsProvider::Eastmoney).is_some());
                if mode == 0 {
                    assert_eq!(b.snapshot().capabilities_requests, vec![checkpoint.capabilities.bytes.clone()]);
                } else {
                    assert_ne!(b.snapshot().capabilities_requests, vec![checkpoint.capabilities.bytes.clone()]);
                    let recovered = local.inspect_macro(&baseline.intent).unwrap();
                    assert_eq!(recovered.attempts().len(), 2);
                    assert_eq!(recovered.attempts()[1].attempt_ordinal(), 2);
                    assert_eq!(recovered.attempts()[1].request_id(), checkpoint.data.id);
                    assert_eq!(recovered.attempts()[1].request_bytes(), checkpoint.data.bytes);
                    if mode == 6 {
                        assert_eq!(recovered.attempts()[1].response_bytes(),
                            Some(super::expected_data_response(&checkpoint.data.id).as_slice()));
                        assert_eq!(recovered.global_news(GlobalNewsProvider::Eastmoney).unwrap().final_bytes(),
                            Some(super::EXPECTED_NATIVE));
                    }
                }
                assert_eq!(b.snapshot().data_requests, vec![checkpoint.data.bytes.clone()]);
                assert_eq!(b.snapshot().data_methods, vec!["global_news"]);
                if mode != 6 {
                    use crate::grpc_client::external_decoder::test_b;
                    let observed = b.snapshot();
                    assert_ne!(test_b::descriptor(), crate::grpc_client::historical_external::DESCRIPTOR_SHA256);
                    for bytes in &observed.health_responses {
                        assert_eq!(test_b::HealthResponse::decode(bytes.as_slice()).unwrap().test_release_b_note, "B");
                        assert!(crate::grpc_client::historical_external::health(bytes).is_err());
                    }
                    assert_eq!(test_b::CapabilitiesResponse::decode(observed.capabilities_responses[0].as_slice()).unwrap().test_release_b_note, "B");
                    assert_eq!(test_b::QueryResponse::decode(observed.data_responses[0].as_slice()).unwrap().test_release_b_note, "B");
                }
            } else {
                assert!(result.is_err());
                assert_eq!(b.snapshot().capabilities_calls, 0);
                assert_eq!(b.snapshot().data_calls, 0);
                if mode == 4 { assert_eq!(b.snapshot().health_requests.len(), 0); }
            }
            let recovered = local.inspect_macro(&baseline.intent).unwrap();
            assert_eq!(recovered.plan_bytes(), checkpoint.plan_bytes);
            assert_eq!(recovered.readiness_episodes()[0].controls()[0].response_bytes(), Some(checkpoint.health.response_bytes.as_slice()));
            drop(local);
        }
        assert_eq!(control_tests::raw_control_bytes(&business.database(), &baseline.intent, 1), checkpoint.raw);
        assert_eq!(a.snapshot().health_requests.len(), if retry_case { 2 } else { 1 });
        assert_eq!(a.snapshot().capabilities_calls, usize::from(retry_case));
        assert_eq!(a.snapshot().data_calls, usize::from(retry_case));
        if let Some(raw) = frozen_retry {
            assert_eq!(attempt_result_bytes(business.connection(), &baseline.intent, 1), raw);
        }
        if retry_case {
            let mut tables = table_names(business.connection());
            tables.push("data_acquisition_audit".into());
            let frozen = old_fact_rows(business.connection(), &tables);
            let observed = b.snapshot();
            business.reopen();
            let mut local = business.store.as_mut().unwrap().single_user_local_chain_post_close(&baseline.config).unwrap();
            let head = local.inspect_run(&baseline.intent).unwrap().head_version();
            clock.now.set(UtcMicros::try_new(started + 14_000_000).unwrap());
            let lease = local.resume_run(&baseline.intent, macro_lease("TEST_CODE_TERMINAL_REOPEN", started + 14_000_000, started + 16_000_000, head)).unwrap();
            macro_driver::drive_test_prepared(&mut local, lease, &source, &clock, Rc::new(Cell::new(false)), &search, prepare()).await.unwrap();
            drop(local);
            assert_eq!(old_fact_rows(business.connection(), &tables), frozen, "terminal reopen cannot append or rewrite facts, bytes, hashes or audits");
            let after = b.snapshot();
            assert_eq!(after.health_requests, observed.health_requests);
            assert_eq!(after.capabilities_requests, observed.capabilities_requests);
            assert_eq!(after.data_requests, observed.data_requests);
            assert_eq!(after.tcp_accepts, observed.tcp_accepts);
        }
        if mode == 0 {
            let current: Vec<u8> = business.connection().query_row("SELECT bytes FROM chain_post_close_macro_control_attempt_results WHERE intent_id=?1 AND kind='Capabilities'", [baseline.intent.as_str()], |row| row.get(0)).unwrap();
            let value: serde_json::Value = serde_json::from_slice(&current).unwrap();
            assert_eq!(value["version"], 4);
            assert_eq!(value["verified_build_identity"]["source_revision"], "TEST_CODE_TRUSTED_RELEASE_B");
            assert_eq!(value["connection_identity"]["descriptor_sha256"], crate::grpc_client::external_decoder::test_b::descriptor());
            let data: serde_json::Value = serde_json::from_slice(&attempt_result_bytes(business.connection(), &baseline.intent, 1)).unwrap();
            assert_eq!(data["version"], 4);
            assert_eq!(data["wire_identity"]["client_descriptor_sha256"], crate::grpc_client::external_decoder::test_b::descriptor());
            assert_eq!(data["response"], serde_json::json!(b.snapshot().data_responses[0]));
            business.reopen();
            let mut local = business.store.as_mut().unwrap().single_user_local_chain_post_close(&baseline.config).unwrap();
            assert!(local.inspect_macro(&baseline.intent).unwrap().global_news(GlobalNewsProvider::Eastmoney).is_some());
        }
    })).catch_unwind().await;
    drop(switch.take());
    if let Some(b) = server_b.take() {
        b.finish().await.unwrap();
    }
    control_tests::cleanup_external_case(
        &mut business,
        &mut parent_server,
        &mut external_server,
        "Task6 durable B",
    )
    .await;
    match body {
        Ok(result) => result.expect("durable B watchdog"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn task6_connection_reopen_capabilities_ready_requires_fresh_capabilities_before_data() {
    assert_task6_reopen_qualification(true).await;
}

#[tokio::test(flavor = "current_thread")]
async fn task6_durable_b_full_ready_reopen_uses_b_health_capabilities_before_original_data() {
    assert_task6_reopen_qualification_on(true, true).await;
}

async fn assert_task6_reopen_qualification(capabilities_ready: bool) {
    assert_task6_reopen_qualification_on(capabilities_ready, false).await;
}

async fn assert_task6_reopen_qualification_on(capabilities_ready: bool, new_b: bool) {
    use crate::data_gateway::grpc_source::macro_queries::PreparedMacroQueries;
    use crate::grpc_client::build_identity::BuildIdentityTrust;
    use crate::grpc_client::client::external_control_loopback_fixture::{
        ExternalMtlsSwitch, HealthReply,
    };
    use crate::grpc_client::client::GrpcMarketClient;
    use crate::push_foundation::intent_store::chain_post_close::{macro_driver, macro_live::Live};
    let mut business = V2BusinessFixture::new();
    let mut parent_server = None;
    let mut external_server = None;
    let mut second_server = None;
    let mut switch = None;
    let body = std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(120), async {
        let baseline = control_tests::setup_external_parent(
            &mut business, &mut parent_server, "TEST_CODE_TASK6_REOPEN_QUALIFICATION",
        ).await;
        business.chain_post_close().migrate_schema_v11_to_v12().unwrap();
        business.chain_post_close().migrate_schema_v12_to_v13().unwrap();
        business.chain_post_close().migrate_schema_v13_to_v14().unwrap();
        business.chain_post_close().migrate_schema_v14_to_v15().unwrap();
        external_server = Some(ExternalMtlsMacroFixture::bind_data_provider_attempts_for_test(false).await.unwrap());
        let external = external_server.as_ref().unwrap();
        if new_b { switch = Some(ExternalMtlsSwitch::bind(external.endpoint()).await); }
        let bundle = switch.as_ref().map_or(external.bundle_path(), ExternalMtlsSwitch::bundle_path);
        let source = GrpcSource::from_external_macro_bundle_for_test(bundle.to_path_buf());
        let PreparedMacroQueries::External(prepared) = source.prepare_macro_queries().unwrap() else { panic!("External fixture") };
        let started = micros(STARTED_LOCAL);
        let clock = MacroClock {
            now: Cell::new(UtcMicros::try_new(started).unwrap()),
            observation: DateTime::parse_from_rfc3339(STARTED_LOCAL).unwrap(),
            observation_calls: Cell::new(0),
        };
        let search = macro_search_service(&[]);
        let web = search.macro_web_snapshot(&source).unwrap();
        let mut requests = Vec::new();
        for (index, provider) in [GlobalNewsProvider::Eastmoney, GlobalNewsProvider::Cailianpress,
            GlobalNewsProvider::Jin10, GlobalNewsProvider::ThePaper].into_iter().enumerate() {
            let identity = MacroQueryIdentity::GlobalNews { provider, limit: 20 };
            let authorized = prepared.prepare_macro_query(identity.clone()).unwrap();
            requests.push((QueryKey::Gateway(index as u8 + 1), macro_codec::Request::capture_prepared_for(&identity, &authorized).unwrap(), prepared.endpoint_uri().to_owned()));
        }
        let health = prepared.prepare_health_attempt().unwrap();
        let capability = prepared.prepare_capabilities_attempt().unwrap();
        let episode = macro_codec::ReadinessEpisodePlan::new(health.request_material(), capability.request_material()).unwrap();
        let mut local = business.store.as_mut().unwrap().single_user_local_chain_post_close(&baseline.config).unwrap();
        let lease = local.resume_run(&baseline.intent, macro_lease("TEST_CODE_TASK6_OWNER", started, started + 1_000_000, baseline.head)).unwrap();
        let mut live = Live::open(&mut local, lease, &clock, Rc::new(Cell::new(false))).unwrap();
        live.initialize(clock.observation, prepared.endpoint_uri(), requests, Some(episode), &web).unwrap();
        let ticket = live.begin_control_connection(health.request_material(), Some(health.connection_identity())).unwrap();
        external.release_health();
        let completion = health.execute().await;
        live.record_control_connection(ticket, macro_codec::ControlRawResult::capture_health(&completion), completion.connection_identity().cloned()).unwrap();
        let old_health = live.current().unwrap().readiness_episodes()[0].controls()[0].response_bytes().unwrap().to_vec();
        let old_capabilities = if capabilities_ready {
            let client = completion.into_connected_client().unwrap();
            let connection = client.external_connection_identity().unwrap();
            let capability = capability.bind_connected(client).unwrap();
            let ticket = live.begin_control_connection(capability.request_material(), Some(connection)).unwrap();
            external.release_capabilities();
            let completion = capability.execute().await.unwrap();
            live.record_control_connection(ticket, macro_codec::ControlRawResult::capture_capabilities(&completion), completion.connection_identity().cloned()).unwrap();
            Some(live.current().unwrap().readiness_episodes()[0].controls()[1].response_bytes().unwrap().to_vec())
        } else { drop(completion); drop(capability); None };
        let head = live.into_lease().head;
        drop(local);
        business.reopen();
        if new_b {
            second_server = Some(ExternalMtlsMacroFixture::bind_health_reply_for_test(HealthReply::TrustedBuildB).await.unwrap());
            switch.as_ref().unwrap().switch_to(second_server.as_ref().unwrap().endpoint()).await;
        }
        let current_server = second_server.as_ref().unwrap_or(external);
        clock.now.set(UtcMicros::try_new(started + 2_000_000).unwrap());
        let mut local = business.store.as_mut().unwrap().single_user_local_chain_post_close(&baseline.config).unwrap();
        let lease = local.resume_run(&baseline.intent, macro_lease("TEST_CODE_TASK6_REOPEN_OWNER", started + 2_000_000, started + 12_000_000, head)).unwrap();
        let mut call = if new_b {
            let prepared = GrpcMarketClient::prepare_client_bundle(bundle).unwrap().with_test_build_trust(BuildIdentityTrust::test_client_b());
            macro_driver::drive_test_prepared(&mut local, lease, &source, &clock, Rc::new(Cell::new(false)), &search, prepared).boxed_local()
        } else { macro_driver::drive(&mut local, lease, &source, &clock, Rc::new(Cell::new(false)), &search).boxed_local() };
        let watchdog = tokio::time::Instant::now() + Duration::from_secs(if new_b { 60 } else { 5 });
        while current_server.snapshot().health_requests.len() < if new_b { 1 } else { 2 } {
            tokio::select! {
                result = &mut call => panic!("reopen must start independent qualification, not stop: {:?}", result.err()),
                _ = tokio::task::yield_now() => {},
            }
            assert!(tokio::time::Instant::now() < watchdog, "new Health watchdog");
        }
        assert_eq!(current_server.snapshot().capabilities_calls, if new_b { 0 } else { usize::from(capabilities_ready) });
        assert_eq!(current_server.snapshot().data_calls, 0);
        assert_ne!(external.snapshot().health_requests[0], current_server.snapshot().health_requests[if new_b { 0 } else { 1 }]);
        if capabilities_ready {
            current_server.release_health();
            while current_server.snapshot().capabilities_calls < if new_b { 1 } else { 2 } {
                tokio::select! {
                    result = &mut call => panic!("fresh qualified connection must confirm its own capabilities: {:?}", result.err()),
                    _ = tokio::task::yield_now() => {},
                }
                assert!(tokio::time::Instant::now() < watchdog, "new Capabilities watchdog");
            }
            assert_eq!(current_server.snapshot().data_calls, 0);
            assert_ne!(external.snapshot().capabilities_requests[0], current_server.snapshot().capabilities_requests[if new_b { 0 } else { 1 }]);
            if new_b {
                current_server.release_capabilities();
                while current_server.snapshot().data_calls == 0 {
                    tokio::select! {
                        result = &mut call => panic!("B current controls must authorize original full-plan data: {:?}", result.err()),
                        _ = tokio::task::yield_now() => {},
                    }
                    assert!(tokio::time::Instant::now() < watchdog, "B data watchdog");
                }
                assert!(current_server.snapshot().data_methods.iter().all(|method| method == "global_news"));
                assert_eq!(external.snapshot().health_requests.len(), 1);
                assert_eq!(external.snapshot().capabilities_calls, 1);
                assert_eq!(external.snapshot().data_calls, 0);
            }
        }
        drop(call);
        drop(local);
        business.reopen();
        let mut local = business.store.as_mut().unwrap().single_user_local_chain_post_close(&baseline.config).unwrap();
        let recovered = local.inspect_macro(&baseline.intent).unwrap();
        assert!(recovered.has_unconfirmed_effect(), "unconfirmed new qualification must survive reopen");
        assert_eq!(recovered.readiness_episodes()[0].controls()[0].response_bytes(), Some(old_health.as_slice()));
        if let Some(old) = old_capabilities {
            assert_eq!(recovered.readiness_episodes()[0].controls()[1].response_bytes(), Some(old.as_slice()));
        }
    })).catch_unwind().await;
    drop(switch.take());
    if let Some(server) = second_server.take() {
        server.finish().await.unwrap();
    }
    control_tests::cleanup_external_case(
        &mut business,
        &mut parent_server,
        &mut external_server,
        "Task6 qualification reopen",
    )
    .await;
    match body {
        Ok(result) => result.expect("reopen watchdog"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

async fn assert_task6_connection_journal(phase: u8) {
    let confirm_health = phase >= 1;
    let mut business = V2BusinessFixture::new();
    let mut parent_server = None;
    let mut external_server = None;
    let body = std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(120), async {
        let baseline = control_tests::setup_external_parent(
            &mut business, &mut parent_server, "TEST_CODE_TASK6_JOURNAL_BEGIN",
        ).await;
        business.chain_post_close().migrate_schema_v11_to_v12().unwrap();
        business.chain_post_close().migrate_schema_v12_to_v13().unwrap();
        business.chain_post_close().migrate_schema_v13_to_v14().unwrap();
        business.chain_post_close().migrate_schema_v14_to_v15().unwrap();
        external_server = Some(ExternalMtlsMacroFixture::bind_data_provider_attempts_for_test(false).await.unwrap());
        let external = external_server.as_ref().unwrap();
        let source = GrpcSource::from_external_macro_bundle_for_test(external.bundle_path().to_path_buf());
        let started = micros(STARTED_LOCAL);
        let clock = MacroClock {
            now: Cell::new(UtcMicros::try_new(started).unwrap()),
            observation: DateTime::parse_from_rfc3339(STARTED_LOCAL).unwrap(),
            observation_calls: Cell::new(0),
        };
        let search = macro_search_service(&[]);
        let database = business.directory.path().join("business.sqlite");
        let mut local = business.store.as_mut().unwrap().single_user_local_chain_post_close(&baseline.config).unwrap();
        let lease = local.resume_run(&baseline.intent, macro_lease("TEST_CODE_TASK6_OWNER", started, started + 1_000_000, baseline.head)).unwrap();
        let mut call = Box::pin(crate::push_foundation::intent_store::chain_post_close::macro_driver::drive(
            &mut local, lease, &source, &clock, Rc::new(Cell::new(false)), &search,
        ));
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while external.snapshot().health_requests.is_empty() {
            tokio::select! {
                result = &mut call => panic!("TEST_CODE journal driver stopped before Health: {:?}", result.err()),
                _ = tokio::task::yield_now() => {},
            }
            assert!(tokio::time::Instant::now() < deadline, "TEST_CODE Health watchdog");
        }
        let reader = BusinessIntentStore::open(&database).unwrap();
        let count: i64 = reader.connection.query_row(
            "SELECT count(*) FROM chain_post_close_macro_connection_facts WHERE intent_id=?1 AND kind='HealthBegin'",
            [baseline.intent.as_str()], |row| row.get(0),
        ).unwrap();
        drop(reader);
        if confirm_health {
            external.release_health();
            while external.snapshot().capabilities_requests.is_empty() {
                tokio::select! {
                    _ = &mut call => break,
                    _ = tokio::task::yield_now() => {},
                }
                assert!(tokio::time::Instant::now() < deadline, "TEST_CODE Capabilities watchdog");
            }
        }
        if phase == 2 && !external.snapshot().capabilities_requests.is_empty() {
            external.release_capabilities();
            while external.snapshot().data_calls == 0 {
                tokio::select! {
                    _ = &mut call => break,
                    _ = tokio::task::yield_now() => {},
                }
                assert!(tokio::time::Instant::now() < deadline, "TEST_CODE data watchdog");
            }
        }
        drop(call);
        drop(local);
        external.release_health();
        assert_eq!(count, 1, "Health must have a durable current-epoch qualification begin before the RPC");
        if confirm_health {
            let reader = BusinessIntentStore::open(&database).unwrap();
            let result_count: i64 = reader.connection.query_row(
                "SELECT count(*) FROM chain_post_close_macro_connection_facts WHERE intent_id=?1 AND kind='HealthResult' AND outcome='Qualified'",
                [baseline.intent.as_str()], |row| row.get(0),
            ).unwrap();
            let link_count: i64 = reader.connection.query_row(
                "SELECT count(*) FROM chain_post_close_macro_connection_facts q JOIN chain_post_close_macro_control_attempt_begins b ON b.intent_id=q.intent_id AND b.run_version=q.effect_version WHERE q.intent_id=?1 AND q.kind='EffectLink' AND b.kind='Capabilities'",
                [baseline.intent.as_str()], |row| row.get(0),
            ).unwrap();
            drop(reader);
            external.release_capabilities();
            assert_eq!((result_count, link_count), (1, 1), "Capabilities needs a committed same-epoch qualification result and effect link");
        }
        if phase == 2 {
            let reader = BusinessIntentStore::open(&database).unwrap();
            let data_links: i64 = reader.connection.query_row(
                "SELECT count(*) FROM chain_post_close_macro_connection_facts q JOIN chain_post_close_macro_attempt_begins b ON b.intent_id=q.intent_id AND b.run_version=q.effect_version WHERE q.intent_id=?1 AND q.kind='EffectLink'",
                [baseline.intent.as_str()], |row| row.get(0),
            ).unwrap();
            drop(reader);
            assert!(data_links > 0 && external.snapshot().data_calls > 0,
                "data needs a committed current-capability link before RPC: links={data_links}, calls={}", external.snapshot().data_calls);
        }
        business.reopen();
        let mut local = business.store.as_mut().unwrap().single_user_local_chain_post_close(&baseline.config).unwrap();
        let recovery = local.inspect_macro(&baseline.intent).unwrap();
        assert!(recovery.has_unconfirmed_effect());
        assert_eq!(external.snapshot().capabilities_requests.len(), usize::from(confirm_health));
        if phase < 2 { assert_eq!(external.snapshot().data_calls, 0); }
        drop(local);
        if phase == 2 {
            // Test the actual reader boundary, not merely a codec in isolation:
            // append-only enforcement first, then an offline duplicate-column
            // corruption with the exact original trigger restored.
            let connection = business.connection();
            assert!(connection.execute(
                "UPDATE chain_post_close_macro_connection_facts SET connection_epoch='TEST_CODE_TAMPERED' WHERE intent_id=?1",
                [baseline.intent.as_str()],
            ).is_err());
            assert!(connection.execute(
                "DELETE FROM chain_post_close_macro_connection_facts WHERE intent_id=?1",
                [baseline.intent.as_str()],
            ).is_err());
            let guard: String = connection.query_row(
                "SELECT sql FROM sqlite_schema WHERE name='chain_post_close_macro_connection_facts_no_update'",
                [], |row| row.get(0),
            ).unwrap();
            connection.execute_batch("DROP TRIGGER chain_post_close_macro_connection_facts_no_update").unwrap();
            connection.execute(
                "UPDATE chain_post_close_macro_connection_facts SET connection_epoch='TEST_CODE_TAMPERED' WHERE intent_id=?1 AND kind='HealthBegin'",
                [baseline.intent.as_str()],
            ).unwrap();
            connection.execute_batch(&guard).unwrap();
            business.reopen();
            assert!(matches!(
                business.store.as_mut().unwrap().single_user_local_chain_post_close(&baseline.config),
                Err(ChainPostCloseError::SchemaRejected),
            ));
        }
    })).catch_unwind().await;
    control_tests::cleanup_external_case(
        &mut business,
        &mut parent_server,
        &mut external_server,
        "Task6 journal begin",
    )
    .await;
    match body {
        Ok(result) => result.expect("TEST_CODE Task6 journal test deadline"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

async fn assert_v12_control_receipt_cancelled_reopens_unknown(cancel_capabilities: bool) {
    let mut business = V2BusinessFixture::new();
    let mut parent_server = None;
    let mut external_server = None;
    let body =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(120), async {
            let baseline = control_tests::setup_external_parent(
                &mut business,
                &mut parent_server,
                if cancel_capabilities {
                    "TEST_CODE_EXTERNAL_V12_CAPABILITIES_UNKNOWN"
                } else {
                    "TEST_CODE_EXTERNAL_V12_HEALTH_UNKNOWN"
                },
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
            business
                .chain_post_close()
                .migrate_schema_v12_to_v13()
                .unwrap();
            business
                .chain_post_close()
                .migrate_schema_v13_to_v14()
                .unwrap();
            business
                .chain_post_close()
                .migrate_schema_v14_to_v15()
                .unwrap();
            external_server = Some(
                ExternalMtlsMacroFixture::bind_data_provider_attempts_for_test(false)
                    .await
                    .unwrap(),
            );
            let external = external_server.as_ref().unwrap();
            let source = GrpcSource::from_external_macro_bundle_for_test(
                external.bundle_path().to_path_buf(),
            );
            let search = macro_search_service(&[]);
            let started_at = micros("2026-09-14T15:31:00+08:00");
            let clock = MacroClock {
                now: Cell::new(UtcMicros::try_new(started_at).unwrap()),
                observation: DateTime::parse_from_rfc3339("2026-09-14T15:31:00+08:00").unwrap(),
                observation_calls: Cell::new(0),
            };
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
                        "TEST_CODE_EXTERNAL_V12_HEALTH_FAULT",
                        started_at,
                        started_at + 1_000_000,
                        baseline.head,
                    ),
                )
                .unwrap();
            let mut io = local
                .macro_preparation_io_v15(
                    lease,
                    &baseline.queries,
                    &clock,
                    FixedClusterConfiguration::resolve(Some("2")),
                    &baseline.source,
                    &source,
                    &search,
                )
                .unwrap();
            let mut prepared = Box::pin(prepare_chain_analysis_with_io(
                NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
                baseline.stocks.clone(),
                None,
                &mut io,
            ));
            let receipt_deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                tokio::select! {
                    biased;
                    result = &mut prepared => panic!(
                        "TEST_CODE v12 Health returned before receipt: {result:?}"
                    ),
                    _ = tokio::task::yield_now() => {}
                }
                if external.snapshot().health_requests.len() == 1 {
                    break;
                }
                assert!(
                    std::time::Instant::now() < receipt_deadline,
                    "TEST_CODE v12 Health receipt watchdog"
                );
            }
            if cancel_capabilities {
                external.release_health();
                loop {
                    tokio::select! {
                        biased;
                        result = &mut prepared => panic!(
                            "TEST_CODE v12 Capabilities returned before receipt: {result:?}"
                        ),
                        _ = tokio::task::yield_now() => {}
                    }
                    if external.snapshot().capabilities_requests.len() == 1 {
                        break;
                    }
                    assert!(
                        std::time::Instant::now() < receipt_deadline,
                        "TEST_CODE v12 Capabilities receipt watchdog"
                    );
                }
            }
            drop(prepared);
            drop(io);
            let pending = local.inspect_macro(&baseline.intent).unwrap();
            assert!(pending.has_unconfirmed_effect());
            let original_plan = pending.plan_bytes().to_vec();
            let controls = pending.readiness_episodes()[0].controls();
            let original_health_result = controls[0].result_version();
            assert_eq!(original_health_result.is_some(), cancel_capabilities);
            let control = &controls[usize::from(cancel_capabilities)];
            assert!(control.begin_version().is_some());
            assert_eq!(control.result_version(), None);
            let original_request_id = control.request_id().to_owned();
            let original_request = control.request_bytes().to_vec();
            assert_eq!(external.snapshot().data_calls, 0);
            drop(pending);
            drop(local);
            if cancel_capabilities {
                external.release_capabilities();
            } else {
                external.release_health();
            }
            let wire_before = external.snapshot();
            external.set_reject_new_connections_for_test(true);

            business.reopen();
            let reopened_clock = MacroClock {
                now: Cell::new(UtcMicros::try_new(started_at + 3_000_000).unwrap()),
                observation: DateTime::parse_from_rfc3339("2026-09-14T15:34:00+08:00").unwrap(),
                observation_calls: Cell::new(0),
            };
            let mut reopened_local = business
                .store
                .as_mut()
                .unwrap()
                .single_user_local_chain_post_close(&baseline.config)
                .unwrap();
            let reopened_head = reopened_local
                .inspect_run(&baseline.intent)
                .unwrap()
                .head_version();
            let lease = reopened_local
                .resume_run(
                    &baseline.intent,
                    macro_lease(
                        "TEST_CODE_EXTERNAL_V12_CONTROL_REOPEN",
                        started_at + 3_000_000,
                        started_at + 4_000_000,
                        reopened_head,
                    ),
                )
                .unwrap();
            let mut reopened_io = reopened_local
                .macro_preparation_io_v15(
                    lease,
                    &baseline.queries,
                    &reopened_clock,
                    FixedClusterConfiguration::resolve(Some("2")),
                    &baseline.source,
                    &source,
                    &search,
                )
                .unwrap();
            let stopped = prepare_chain_analysis_with_io(
                NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
                baseline.stocks.clone(),
                None,
                &mut reopened_io,
            )
            .await
            .expect_err("TEST_CODE v12 pending control must remain Unknown");
            assert!(matches!(
                stopped.downcast_ref::<PreparationStop>(),
                Some(PreparationStop::IncompleteOnReopen { intent_id })
                    if intent_id == baseline.intent.as_str()
            ));
            assert!(matches!(
                stopped.downcast_ref::<ChainPostCloseError>(),
                Some(ChainPostCloseError::IncompleteEffect { intent_id })
                    if intent_id == baseline.intent.as_str()
            ));
            drop(reopened_io);
            let recovered = reopened_local.inspect_macro(&baseline.intent).unwrap();
            assert!(recovered.has_unconfirmed_effect());
            assert_eq!(recovered.plan_bytes(), original_plan);
            let controls = recovered.readiness_episodes()[0].controls();
            assert_eq!(controls[0].result_version(), original_health_result);
            let recovered_control = &controls[usize::from(cancel_capabilities)];
            assert_eq!(recovered_control.result_version(), None);
            assert_eq!(recovered_control.request_id(), original_request_id);
            assert_eq!(recovered_control.request_bytes(), original_request);
            let wire_after = external.snapshot();
            assert_eq!(wire_after.tcp_accepts, wire_before.tcp_accepts);
            assert_eq!(wire_after.health_requests, wire_before.health_requests);
            assert_eq!(
                wire_after.capabilities_requests,
                wire_before.capabilities_requests
            );
            assert_eq!(wire_after.data_calls, 0);
        }))
        .catch_unwind()
        .await;

    control_tests::cleanup_external_case(
        &mut business,
        &mut parent_server,
        &mut external_server,
        "v12 External control Unknown reopen",
    )
    .await;
    drop(business);
    match body {
        Ok(result) => result.expect("TEST_CODE v12 control Unknown body deadline"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn single_user_external_v12_health_receipt_cancelled_reopens_unknown_without_rpc() {
    assert_v12_control_receipt_cancelled_reopens_unknown(false).await;
}

#[tokio::test(flavor = "current_thread")]
async fn single_user_external_v12_capabilities_receipt_cancelled_reopens_unknown_without_rpc() {
    assert_v12_control_receipt_cancelled_reopens_unknown(true).await;
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
            business.chain_post_close().migrate_schema_v12_to_v13().unwrap();
            business.chain_post_close().migrate_schema_v13_to_v14().unwrap();
            business.chain_post_close().migrate_schema_v14_to_v15().unwrap();
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
                .macro_preparation_io_v15(
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
                assert_eq!(result_json["raw"]["version"], 3);
                assert!(result_json["raw"]["external_wire"].is_object());
                assert_eq!(result_json["raw"]["wire_identity"]["profile"], "ExternalV1");
                assert_eq!(result_json["raw"]["wire_identity"]["method"], "OPERATION_GLOBAL_NEWS");

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
