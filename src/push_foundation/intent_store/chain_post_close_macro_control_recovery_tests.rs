use super::*;
use crate::grpc_client::client::external_control_attempt::ExternalControlKind;
use crate::grpc_client::client::external_control_loopback_fixture::{
    test_external_build_identity, test_external_observability, ExternalMtlsMacroFixture,
};
use crate::grpc_client::client::ContractProfile;
use crate::grpc_client::external_pb::magic::market::v1::{
    AdmissionState, CapabilitiesRequest, CapabilitiesResponse, Capability, HealthRequest,
    HealthResponse, Operation,
};
use crate::push_foundation::intent_store::chain_post_close::macro_stage::MacroControlOutcome;

const AUTHORITY: &str = "grpc-mtls:macro.test.invalid";
const STARTED_LOCAL: &str = "2026-09-14T15:31:00+08:00";

fn expected_pending_sources() -> Vec<MacroQueryIdentity> {
    vec![
        MacroQueryIdentity::GlobalNews {
            provider: GlobalNewsProvider::Eastmoney,
            limit: 20,
        },
        MacroQueryIdentity::GlobalNews {
            provider: GlobalNewsProvider::Cailianpress,
            limit: 20,
        },
        MacroQueryIdentity::GlobalNews {
            provider: GlobalNewsProvider::Jin10,
            limit: 20,
        },
        MacroQueryIdentity::GlobalNews {
            provider: GlobalNewsProvider::ThePaper,
            limit: 20,
        },
        MacroQueryIdentity::EconomicCalendar,
    ]
}

fn expected_pending_research() -> Vec<String> {
    [
        "2026年09月14日A股 大盘 股市 最新动态",
        "2026年09月14日国际财经 地缘政治 最新消息",
        "2026年09月14日美股 美联储 大宗商品 今日",
        "2026年09月14日中国 央行 财政 产业政策 重要新闻",
        "2026年09月14日高盛 摩根 大摩 美银 JPMorgan 中国A股 市场观点 研报",
        "2026年09月14日证券时报 第一财经 21世纪经济报道 重要财经",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

pub(super) struct RequestEvidence {
    pub(super) id: String,
    pub(super) bytes: Vec<u8>,
}

pub(super) struct ConfirmedHealthEvidence {
    pub(super) request: RequestEvidence,
    pub(super) response_bytes: Vec<u8>,
    pub(super) begin_version: u64,
    pub(super) result_version: u64,
}

pub(super) struct ConfirmedHealthCheckpoint {
    pub(super) plan_bytes: Vec<u8>,
    pub(super) data: RequestEvidence,
    pub(super) health: ConfirmedHealthEvidence,
    pub(super) capabilities: RequestEvidence,
    pub(super) head_version: u64,
    pub(super) raw: Vec<u8>,
}

pub(super) async fn reach_confirmed_health_checkpoint(
    business: &mut V2BusinessFixture,
    baseline: &control_tests::ExternalParentBaseline,
    external: &ExternalMtlsMacroFixture,
    owner: &str,
) -> ConfirmedHealthCheckpoint {
    let parent_source = &baseline.source;
    let queries = &baseline.queries;
    let stocks = &baseline.stocks;
    let config = &baseline.config;
    let intent = &baseline.intent;
    let parent_final = baseline.final_bytes.as_slice();
    let parent_context = baseline.context.as_slice();
    let parent_head = baseline.head;
    let earlier_tables = &baseline.tables;
    let earlier_facts = &baseline.facts;
    let baseline_audit = &baseline.audit;
    let database = business.database();
    let macro_source = GrpcSource::from_external_macro_bundle_for_test(
        external.bundle_path().to_path_buf(),
    );
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;
    let before_prepare = external.snapshot();
    assert_eq!(before_prepare.tcp_accepts, 0);
    assert!(before_prepare.health_requests.is_empty());
    assert_eq!(before_prepare.capabilities_calls, 0);
    assert_eq!(before_prepare.data_calls, 0);

    let started_at = micros(STARTED_LOCAL);
    let clock = MacroClock {
        now: Cell::new(UtcMicros::try_new(started_at).unwrap()),
        observation: DateTime::parse_from_rfc3339(STARTED_LOCAL).unwrap(),
        observation_calls: Cell::new(0),
    };
    let registered = [
        GeneralWebResearchProvider::SerpApi,
        GeneralWebResearchProvider::Bocha,
        GeneralWebResearchProvider::Tavily,
    ];
    let search_service = macro_search_service(&registered);
    let inspect = || {
        let mut reader = BusinessIntentStore::open(&database).unwrap();
        let mut read_local = reader
            .single_user_local_chain_post_close(config)
            .unwrap();
        let recovery = read_local.inspect_macro(intent).unwrap();
        let run = read_local.inspect_run(intent).unwrap();
        let snapshot = (
            run.head_version(),
            run.lease_generation(),
            run.context().canonical_bytes(),
        );
        drop(read_local);
        reader.connection.close().unwrap();
        (recovery, snapshot)
    };
    let mut local = business
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(config)
        .unwrap();
    let lease = local
        .resume_run(
            intent,
            macro_lease(
                owner,
                started_at,
                started_at + 2_000_000,
                parent_head,
            ),
        )
        .unwrap();
    let mut io = local
        .macro_preparation_io_v11(
            lease,
            queries,
            &clock,
            FixedClusterConfiguration::resolve(Some("2")),
            parent_source,
            &macro_source,
            &search_service,
        )
        .unwrap();

    let (
        plan_bytes,
        data_id,
        data_bytes,
        health_id,
        health_bytes,
        health_response_bytes,
        capabilities_id,
        capabilities_bytes,
        health_begin_version,
        health_result_version,
        health_ready_head,
    ) = {
        let mut prepared = Box::pin(prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            (*stocks).clone(),
            None,
            &mut io,
        ));
        let receipt_deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            match futures::poll!(&mut prepared) {
                std::task::Poll::Pending => {}
                std::task::Poll::Ready(result) => panic!(
                    "TEST_CODE Health-ready prepare returned before Health receipt: {result:?}"
                ),
            }
            if external.snapshot().health_requests.len() == 1 {
                break;
            }
            assert!(
                std::time::Instant::now() < receipt_deadline,
                "TEST_CODE Health-ready Health receipt watchdog"
            );
            tokio::task::yield_now().await;
        }

        let (health_pending, (pending_head, pending_generation, pending_context)) =
            inspect();
        assert!(!health_pending.is_complete());
        assert!(health_pending.has_unconfirmed_effect());
        assert!(health_pending.attempts().is_empty());
        assert!(health_pending
            .global_news(GlobalNewsProvider::Eastmoney)
            .is_none());
        assert_eq!(health_pending.parent_final_bytes(), parent_final);
        assert_eq!(pending_generation, 3);
        assert_eq!(pending_context, parent_context);
        let plan = health_pending.plan();
        assert_eq!(plan.profile(), ContractProfile::ExternalV1);
        assert_eq!(plan.acquisition_authority(), Some(AUTHORITY));
        assert_eq!(plan.endpoint(), external.endpoint());
        assert_eq!(plan.started_at().get(), started_at);
        assert_eq!(plan.deadline_at().get(), started_at + 15_000_000);
        assert_eq!(plan.observed_local(), STARTED_LOCAL);
        let first_request = plan.first_source_request();
        assert_eq!(first_request.retry_policy(), (4, 1000, 60_000, 200));
        let data_id = first_request.request_id().to_owned();
        let data_bytes = first_request.request_bytes().to_vec();
        let data_wire = QueryRequest::decode(data_bytes.as_slice()).unwrap();
        assert_eq!(data_wire.encode_to_vec(), data_bytes);
        assert_eq!(data_wire.context.as_ref().unwrap().protocol_version, 1);
        assert_eq!(data_wire.context.as_ref().unwrap().request_id, data_id);
        assert_eq!(data_wire.preferred_provider, "Eastmoney");
        assert!(!data_wire.allow_unadmitted);
        let payload = data_wire.payload.as_ref().unwrap();
        assert_eq!(payload.schema, "magic.market.global_news.request");
        assert_eq!(payload.schema_version, 2);
        assert_eq!(payload.content_type, "application/json; charset=utf-8");
        assert_eq!(payload.data, br#"{"limit":20}"#);
        assert_eq!(
            health_pending.pending_source_identities(),
            expected_pending_sources().as_slice()
        );
        assert_eq!(
            health_pending.pending_research_queries(),
            expected_pending_research().as_slice()
        );
        let episodes = health_pending.readiness_episodes();
        assert_eq!(episodes.len(), 1);
        let episode = &episodes[0];
        assert_eq!(episode.episode_ordinal(), 1);
        assert_eq!(
            episode.initiating_source(),
            &MacroQueryIdentity::GlobalNews {
                provider: GlobalNewsProvider::Eastmoney,
                limit: 20,
            }
        );
        assert_eq!(episode.ready_result_version(), None);
        let controls = episode.controls();
        assert_eq!(controls.len(), 2);
        assert_eq!(controls[0].kind(), ExternalControlKind::Health);
        assert_eq!(controls[1].kind(), ExternalControlKind::Capabilities);
        let health_begin_version = controls[0]
            .begin_version()
            .expect("TEST_CODE Health begin committed before receipt");
        assert_eq!(pending_head, health_begin_version);
        assert_eq!(controls[0].result_version(), None);
        assert_eq!(controls[0].outcome(), None);
        assert_eq!(controls[0].response_bytes(), None);
        assert_eq!(controls[1].begin_version(), None);
        assert_eq!(controls[1].result_version(), None);
        assert_eq!(controls[1].outcome(), None);
        assert_eq!(controls[1].response_bytes(), None);
        let health_id = controls[0].request_id().to_owned();
        let health_bytes = controls[0].request_bytes().to_vec();
        let capabilities_id = controls[1].request_id().to_owned();
        let capabilities_bytes = controls[1].request_bytes().to_vec();
        let health_request = HealthRequest::decode(health_bytes.as_slice()).unwrap();
        assert_eq!(health_request.encode_to_vec(), health_bytes);
        assert_eq!(health_request.context.as_ref().unwrap().protocol_version, 1);
        assert_eq!(health_request.context.as_ref().unwrap().request_id, health_id);
        let capabilities_request =
            CapabilitiesRequest::decode(capabilities_bytes.as_slice()).unwrap();
        assert_eq!(capabilities_request.encode_to_vec(), capabilities_bytes);
        assert_eq!(
            capabilities_request
                .context
                .as_ref()
                .unwrap()
                .protocol_version,
            1
        );
        assert_eq!(
            capabilities_request.context.as_ref().unwrap().request_id,
            capabilities_id
        );
        assert_ne!(health_id, capabilities_id);
        assert_ne!(data_id, health_id);
        assert_ne!(data_id, capabilities_id);
        let plan_bytes = health_pending.plan_bytes().to_vec();
        let receipt = external.snapshot();
        assert_eq!(receipt.tcp_accepts, 1);
        assert_eq!(receipt.health_requests, vec![health_bytes.clone()]);
        assert_eq!(receipt.health_authorized, vec![true]);
        assert!(receipt.health_responses.is_empty());
        assert!(receipt.health_statuses.is_empty());
        assert_eq!(receipt.capabilities_calls, 0);
        assert!(receipt.capabilities_requests.is_empty());
        assert!(receipt.capabilities_responses.is_empty());
        assert!(receipt.capabilities_statuses.is_empty());
        assert_eq!(receipt.data_calls, 0);
        assert!(receipt.data_requests.is_empty());
        assert!(receipt.data_responses.is_empty());
        assert!(receipt.data_statuses.is_empty());
        assert_eq!(
            control_tests::audit_snapshot_at(&database).as_slice(),
            baseline_audit.as_slice()
        );

        let health_response_bytes = HealthResponse {
            request_id: health_id.clone(),
            live: true,
            ready: true,
            state: "TEST_CODE_HEALTH_RUNNING".to_owned(),
            observability: Some(test_external_observability()),
            build_identity: Some(test_external_build_identity()),
        }
        .encode_to_vec();
        external.release_health();
        let checkpoint_deadline =
            std::time::Instant::now() + Duration::from_secs(5);
        let (health_result_version, health_ready_head) = loop {
            match futures::poll!(&mut prepared) {
                std::task::Poll::Pending => {}
                std::task::Poll::Ready(result) => panic!(
                    "TEST_CODE Health-ready prepare returned before checkpoint: {result:?}"
                ),
            }
            let (recovery, (run_head, run_generation, run_context)) = inspect();
            let episode = &recovery.readiness_episodes()[0];
            let controls = episode.controls();
            if controls[0].outcome() == Some(MacroControlOutcome::Ready) {
                assert_eq!(
                    controls[1].begin_version(),
                    None,
                    "TEST_CODE Capabilities began in the confirmed Health checkpoint poll"
                );
                let health_result_version = controls[0]
                    .result_version()
                    .expect("TEST_CODE Health Ready result version");
                assert!(health_result_version > health_begin_version);
                assert_eq!(run_head, health_result_version);
                assert_eq!(run_generation, 3);
                assert_eq!(run_context, parent_context);
                assert!(!recovery.has_unconfirmed_effect());
                assert!(recovery.attempts().is_empty());
                assert!(recovery
                    .global_news(GlobalNewsProvider::Eastmoney)
                    .is_none());
                assert_eq!(recovery.plan_bytes(), plan_bytes);
                assert_eq!(recovery.parent_final_bytes(), parent_final);
                assert_eq!(recovery.plan().deadline_at().get(), started_at + 15_000_000);
                assert_eq!(episode.ready_result_version(), None);
                assert_eq!(controls[0].request_id(), health_id);
                assert_eq!(controls[0].request_bytes(), health_bytes);
                assert_eq!(controls[0].response_bytes(), Some(health_response_bytes.as_slice()));
                assert_eq!(controls[1].request_id(), capabilities_id);
                assert_eq!(controls[1].request_bytes(), capabilities_bytes);
                assert_eq!(controls[1].result_version(), None);
                assert_eq!(controls[1].outcome(), None);
                assert_eq!(controls[1].response_bytes(), None);
                assert_eq!(
                    recovery.pending_source_identities(),
                    expected_pending_sources().as_slice()
                );
                assert_eq!(
                    recovery.pending_research_queries(),
                    expected_pending_research().as_slice()
                );
                let checkpoint_wire = external.snapshot();
                assert_eq!(checkpoint_wire.tcp_accepts, 1);
                assert_eq!(checkpoint_wire.health_requests, vec![health_bytes.clone()]);
                assert_eq!(checkpoint_wire.health_authorized, vec![true]);
                assert_eq!(
                    checkpoint_wire.health_responses,
                    vec![health_response_bytes.clone()]
                );
                assert!(checkpoint_wire.health_statuses.is_empty());
                assert_eq!(checkpoint_wire.capabilities_calls, 0);
                assert!(checkpoint_wire.capabilities_requests.is_empty());
                assert!(checkpoint_wire.capabilities_responses.is_empty());
                assert!(checkpoint_wire.capabilities_statuses.is_empty());
                assert_eq!(checkpoint_wire.data_calls, 0);
                assert!(checkpoint_wire.data_requests.is_empty());
                assert!(checkpoint_wire.data_responses.is_empty());
                assert!(checkpoint_wire.data_statuses.is_empty());
                assert_eq!(
                    control_tests::audit_snapshot_at(&database).as_slice(),
                    baseline_audit.as_slice()
                );
                break (health_result_version, run_head);
            }
            assert!(
                std::time::Instant::now() < checkpoint_deadline,
                "TEST_CODE Health-ready checkpoint watchdog"
            );
            tokio::task::yield_now().await;
        };
        (
            plan_bytes,
            data_id,
            data_bytes,
            health_id,
            health_bytes,
            health_response_bytes,
            capabilities_id,
            capabilities_bytes,
            health_begin_version,
            health_result_version,
            health_ready_head,
        )
    };
    drop(io);
    assert_eq!(clock.observation_calls.get(), 1);
    assert_eq!(local.inspect_run(intent).unwrap().head_version(), health_ready_head);
    drop(local);
    assert_eq!(
        &old_fact_rows(business.connection(), earlier_tables),
        earlier_facts
    );
    assert_eq!(
        control_tests::audit_snapshot_at(&database).as_slice(),
        baseline_audit.as_slice()
    );
    let raw = control_tests::raw_control_bytes(&database, intent, 1);
    control_tests::assert_response_raw(&raw, &health_response_bytes);
    drop(macro_source);
    ConfirmedHealthCheckpoint {
        plan_bytes,
        data: RequestEvidence {
            id: data_id,
            bytes: data_bytes,
        },
        health: ConfirmedHealthEvidence {
            request: RequestEvidence {
                id: health_id,
                bytes: health_bytes,
            },
            response_bytes: health_response_bytes,
            begin_version: health_begin_version,
            result_version: health_result_version,
        },
        capabilities: RequestEvidence {
            id: capabilities_id,
            bytes: capabilities_bytes,
        },
        head_version: health_ready_head,
        raw,
    }
}


#[tokio::test]
async fn single_user_external_macro_confirmed_health_reopens_and_continues_only_original_capabilities(
) {
    let mut business = V2BusinessFixture::new();
    let mut parent_server = None;
    let mut external_server = None;
    let body = std::panic::AssertUnwindSafe(tokio::time::timeout(
        Duration::from_secs(120),
        async {
            let baseline = control_tests::setup_external_parent(
                &mut business,
                &mut parent_server,
                "TEST_CODE_RUN_EXTERNAL_MACRO_HEALTH_READY_CHECKPOINT",
            )
            .await;
            external_server = Some(
                ExternalMtlsMacroFixture::bind_data_success_for_test()
                    .await
                    .expect("TEST_CODE Health-ready mTLS fixture"),
            );
            let external = external_server
                .as_ref()
                .expect("TEST_CODE Health-ready mTLS owner");
            let checkpoint = reach_confirmed_health_checkpoint(
                &mut business,
                &baseline,
                external,
                "TEST_CODE_EXTERNAL_HEALTH_READY_OWNER_A",
            )
            .await;
            let parent_endpoint = baseline.endpoint;
            let parent_source = baseline.source;
            let queries = baseline.queries;
            let stocks = baseline.stocks;
            let config = baseline.config;
            let intent = baseline.intent;
            let parent_final = baseline.final_bytes;
            let parent_receipt = baseline.receipt;
            let parent_context = baseline.context;
            let earlier_tables = baseline.tables;
            let earlier_facts = baseline.facts;
            let baseline_audit = baseline.audit;
            let old_network = baseline.network;
            let old_memberships = baseline.memberships;
            let database = business.database();

            let started_at = micros(STARTED_LOCAL);
            let registered = [
                GeneralWebResearchProvider::SerpApi,
                GeneralWebResearchProvider::Bocha,
                GeneralWebResearchProvider::Tavily,
            ];
            let search_service = macro_search_service(&registered);
            let inspect = || {
                let mut reader = BusinessIntentStore::open(&database).unwrap();
                let mut read_local = reader
                    .single_user_local_chain_post_close(&config)
                    .unwrap();
                let recovery = read_local.inspect_macro(&intent).unwrap();
                let run = read_local.inspect_run(&intent).unwrap();
                let snapshot = (
                    run.head_version(),
                    run.lease_generation(),
                    run.context().canonical_bytes(),
                );
                drop(read_local);
                reader.connection.close().unwrap();
                (recovery, snapshot)
            };
            let ConfirmedHealthCheckpoint {
                plan_bytes,
                data:
                    RequestEvidence {
                        id: data_id,
                        bytes: data_bytes,
                    },
                health:
                    ConfirmedHealthEvidence {
                        request:
                            RequestEvidence {
                                id: health_id,
                                bytes: health_bytes,
                            },
                        response_bytes: health_response_bytes,
                        begin_version: health_begin_version,
                        result_version: health_result_version,
                    },
                capabilities:
                    RequestEvidence {
                        id: capabilities_id,
                        bytes: capabilities_bytes,
                    },
                head_version: health_ready_head,
                raw: _health_raw,
            } = checkpoint;

            drop(queries);
            drop(parent_source);
            business.reopen();
            let reopened_macro_source = GrpcSource::from_external_macro_bundle_for_test(
                external.bundle_path().to_path_buf(),
            );
            let reopened_parent_source = GrpcSource::from_board_loopback_test_client(
                connect_parent_instance(&parent_endpoint).await,
            );
            let reopened_queries = reopened_parent_source.connected_board_queries().await.unwrap();
            let reopened_clock = MacroClock {
                now: Cell::new(UtcMicros::try_new(started_at + 3_000_000).unwrap()),
                observation: DateTime::parse_from_rfc3339("2026-09-14T15:32:00+08:00")
                    .unwrap(),
                observation_calls: Cell::new(0),
            };
            let mut local = business
                .store
                .as_mut()
                .unwrap()
                .single_user_local_chain_post_close(&config)
                .unwrap();
            let lease = local
                .resume_run(
                    &intent,
                    macro_lease(
                        "TEST_CODE_EXTERNAL_HEALTH_READY_OWNER_B",
                        started_at + 3_000_000,
                        started_at + 10_000_000,
                        health_ready_head,
                    ),
                )
                .unwrap();
            let resume_head = local.inspect_run(&intent).unwrap().head_version();
            assert!(resume_head > health_result_version);
            let mut io = local
                .macro_preparation_io_v11(
                    lease,
                    &reopened_queries,
                    &reopened_clock,
                    FixedClusterConfiguration::resolve(Some("2")),
                    &reopened_parent_source,
                    &reopened_macro_source,
                    &search_service,
                )
                .unwrap();
            let mut prepared = Box::pin(prepare_chain_analysis_with_io(
                NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
                stocks,
                None,
                &mut io,
            ));

            let capabilities_receipt_deadline =
                std::time::Instant::now() + Duration::from_secs(5);
            let capabilities_begin_version = loop {
                match futures::poll!(&mut prepared) {
                    std::task::Poll::Pending => {}
                    std::task::Poll::Ready(result) => panic!(
                        "TEST_CODE reopened prepare returned before Capabilities receipt: {result:?}"
                    ),
                }
                let wire = external.snapshot();
                assert_eq!(wire.health_requests, vec![health_bytes.clone()]);
                assert_eq!(wire.health_responses, vec![health_response_bytes.clone()]);
                assert_eq!(wire.data_calls, 0);
                if wire.capabilities_calls == 1 {
                    let (recovery, (run_head, run_generation, run_context)) = inspect();
                    assert_eq!(wire.tcp_accepts, 2);
                    assert_eq!(wire.capabilities_requests, vec![capabilities_bytes.clone()]);
                    assert_eq!(wire.capabilities_authorized, vec![true]);
                    assert!(wire.capabilities_responses.is_empty());
                    assert!(wire.capabilities_statuses.is_empty());
                    let episode = &recovery.readiness_episodes()[0];
                    assert_eq!(episode.ready_result_version(), None);
                    let controls = episode.controls();
                    assert_eq!(controls[0].begin_version(), Some(health_begin_version));
                    assert_eq!(controls[0].request_id(), health_id);
                    assert_eq!(controls[0].request_bytes(), health_bytes);
                    assert_eq!(controls[0].outcome(), Some(MacroControlOutcome::Ready));
                    assert_eq!(controls[0].result_version(), Some(health_result_version));
                    assert_eq!(controls[0].response_bytes(), Some(health_response_bytes.as_slice()));
                    let capabilities_begin_version = controls[1]
                        .begin_version()
                        .expect("TEST_CODE original Capabilities begin committed");
                    assert!(capabilities_begin_version > resume_head);
                    assert_eq!(run_head, capabilities_begin_version);
                    assert_eq!(run_generation, 4);
                    assert_eq!(run_context, parent_context);
                    assert_eq!(controls[1].request_id(), capabilities_id);
                    assert_eq!(controls[1].request_bytes(), capabilities_bytes);
                    assert_eq!(controls[1].result_version(), None);
                    assert_eq!(controls[1].outcome(), None);
                    assert_eq!(controls[1].response_bytes(), None);
                    assert!(recovery.has_unconfirmed_effect());
                    assert!(recovery.attempts().is_empty());
                    assert!(recovery
                        .global_news(GlobalNewsProvider::Eastmoney)
                        .is_none());
                    assert_eq!(recovery.plan_bytes(), plan_bytes);
                    assert_eq!(recovery.plan().profile(), ContractProfile::ExternalV1);
                    assert_eq!(recovery.plan().acquisition_authority(), Some(AUTHORITY));
                    assert_eq!(recovery.plan().endpoint(), external.endpoint());
                    assert_eq!(recovery.plan().started_at().get(), started_at);
                    assert_eq!(recovery.plan().deadline_at().get(), started_at + 15_000_000);
                    assert_eq!(recovery.plan().observed_local(), STARTED_LOCAL);
                    assert_eq!(
                        recovery.plan().first_source_request().request_id(),
                        data_id
                    );
                    assert_eq!(
                        recovery.plan().first_source_request().request_bytes(),
                        data_bytes
                    );
                    assert_eq!(
                        recovery.pending_source_identities(),
                        expected_pending_sources().as_slice()
                    );
                    assert_eq!(
                        recovery.pending_research_queries(),
                        expected_pending_research().as_slice()
                    );
                    assert_eq!(
                        control_tests::audit_snapshot_at(&database),
                        baseline_audit
                    );
                    break capabilities_begin_version;
                }
                assert_eq!(wire.capabilities_calls, 0);
                assert!(
                    std::time::Instant::now() < capabilities_receipt_deadline,
                    "TEST_CODE reopened Capabilities receipt watchdog"
                );
                tokio::task::yield_now().await;
            };

            let expected_capabilities_bytes = CapabilitiesResponse {
                request_id: capabilities_id.clone(),
                capabilities: vec![
                    Capability {
                        operation: Operation::GlobalNews as i32,
                        repository_admission: AdmissionState::Admitted as i32,
                        runtime_available: true,
                        provider: "Eastmoney".to_owned(),
                        exact_scope: "TEST_CODE_GLOBAL_NEWS_EASTMONEY_20".to_owned(),
                        blocker: String::new(),
                        diagnostic_available: true,
                    },
                    Capability {
                        operation: Operation::SemanticSearch as i32,
                        repository_admission: AdmissionState::Unadmitted as i32,
                        runtime_available: false,
                        provider: "Bocha".to_owned(),
                        exact_scope: "TEST_CODE_UNDELIVERED_SEMANTIC_SEARCH".to_owned(),
                        blocker: "TEST_CODE_CAPABILITY_BLOCKED".to_owned(),
                        diagnostic_available: false,
                    },
                ],
            }
            .encode_to_vec();
            external.release_capabilities();
            let capabilities_checkpoint_deadline =
                std::time::Instant::now() + Duration::from_secs(5);
            let capabilities_result_version = loop {
                match futures::poll!(&mut prepared) {
                    std::task::Poll::Pending => {}
                    std::task::Poll::Ready(result) => panic!(
                        "TEST_CODE reopened prepare returned before Capabilities checkpoint: {result:?}"
                    ),
                }
                let (recovery, (run_head, run_generation, run_context)) = inspect();
                let episode = &recovery.readiness_episodes()[0];
                let controls = episode.controls();
                if controls[1].outcome() == Some(MacroControlOutcome::Ready) {
                    let capabilities_result_version = controls[1]
                        .result_version()
                        .expect("TEST_CODE Capabilities Ready result version");
                    assert!(health_result_version < resume_head);
                    assert!(resume_head < capabilities_begin_version);
                    assert!(capabilities_begin_version < capabilities_result_version);
                    assert_eq!(run_head, capabilities_result_version);
                    assert_eq!(run_generation, 4);
                    assert_eq!(run_context, parent_context);
                    assert_eq!(episode.ready_result_version(), Some(capabilities_result_version));
                    assert_eq!(controls[0].begin_version(), Some(health_begin_version));
                    assert_eq!(controls[0].request_id(), health_id);
                    assert_eq!(controls[0].request_bytes(), health_bytes);
                    assert_eq!(controls[0].result_version(), Some(health_result_version));
                    assert_eq!(controls[0].outcome(), Some(MacroControlOutcome::Ready));
                    assert_eq!(controls[0].response_bytes(), Some(health_response_bytes.as_slice()));
                    assert_eq!(controls[1].request_id(), capabilities_id);
                    assert_eq!(controls[1].request_bytes(), capabilities_bytes);
                    assert_eq!(
                        controls[1].response_bytes(),
                        Some(expected_capabilities_bytes.as_slice())
                    );
                    assert!(!recovery.has_unconfirmed_effect());
                    assert!(recovery.attempts().is_empty());
                    assert!(recovery
                        .global_news(GlobalNewsProvider::Eastmoney)
                        .is_none());
                    assert_eq!(recovery.plan_bytes(), plan_bytes);
                    assert_eq!(recovery.parent_final_bytes(), parent_final);
                    assert_eq!(
                        recovery.pending_source_identities(),
                        expected_pending_sources().as_slice()
                    );
                    assert_eq!(
                        recovery.pending_research_queries(),
                        expected_pending_research().as_slice()
                    );
                    assert_eq!(
                        control_tests::audit_snapshot_at(&database),
                        baseline_audit
                    );
                    let wire = external.snapshot();
                    assert_eq!(wire.tcp_accepts, 2);
                    assert_eq!(wire.health_requests, vec![health_bytes.clone()]);
                    assert_eq!(wire.health_authorized, vec![true]);
                    assert_eq!(wire.health_responses, vec![health_response_bytes.clone()]);
                    assert!(wire.health_statuses.is_empty());
                    assert_eq!(wire.capabilities_calls, 1);
                    assert_eq!(wire.capabilities_requests, vec![capabilities_bytes.clone()]);
                    assert_eq!(wire.capabilities_authorized, vec![true]);
                    assert_eq!(
                        wire.capabilities_responses,
                        vec![expected_capabilities_bytes.clone()]
                    );
                    assert!(wire.capabilities_statuses.is_empty());
                    assert_eq!(wire.data_calls, 0);
                    assert!(wire.data_requests.is_empty());
                    assert!(wire.data_responses.is_empty());
                    assert!(wire.data_statuses.is_empty());
                    break capabilities_result_version;
                }
                assert!(
                    std::time::Instant::now() < capabilities_checkpoint_deadline,
                    "TEST_CODE Capabilities Ready checkpoint watchdog"
                );
                tokio::task::yield_now().await;
            };

            drop(prepared);
            drop(io);
            let stable = local.inspect_macro(&intent).unwrap();
            assert!(!stable.has_unconfirmed_effect());
            assert!(stable.attempts().is_empty());
            assert!(stable
                .global_news(GlobalNewsProvider::Eastmoney)
                .is_none());
            assert_eq!(stable.plan_bytes(), plan_bytes);
            assert_eq!(
                stable.readiness_episodes()[0].ready_result_version(),
                Some(capabilities_result_version)
            );
            assert_eq!(
                local.inspect_run(&intent).unwrap().head_version(),
                capabilities_result_version
            );
            assert_eq!(reopened_clock.observation_calls.get(), 0);
            drop(local);
            assert_eq!(control_tests::audit_snapshot_at(&database), baseline_audit);
            assert_eq!(
                parent_server.as_ref().unwrap().snapshot(),
                old_network
            );
            assert_eq!(
                parent_server.as_ref().unwrap().membership_snapshot(),
                old_memberships
            );
            assert_eq!(
                old_fact_rows(business.connection(), &earlier_tables),
                earlier_facts
            );
            let transaction = business.connection().unchecked_transaction().unwrap();
            read_acquisition_in_transaction(&transaction, &parent_receipt).unwrap();
            transaction.commit().unwrap();
            drop(reopened_queries);
            drop(reopened_parent_source);
            drop(reopened_macro_source);
        },
    ))
    .catch_unwind()
    .await;

    control_tests::cleanup_external_case(
        &mut business,
        &mut parent_server,
        &mut external_server,
        "Health-ready checkpoint",
    )
    .await;
    drop(business);
    match body {
        Ok(result) => result.expect("TEST_CODE Health-ready checkpoint body deadline"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}
