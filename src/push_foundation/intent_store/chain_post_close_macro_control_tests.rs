use super::*;
use crate::grpc_client::client::board_loopback_fixture::{
    BoardLoopbackObservation, BoardLoopbackServer,
};
use crate::grpc_client::client::external_control_attempt::ExternalControlKind;
use crate::grpc_client::client::external_control_loopback_fixture::{
    test_external_build_identity, test_external_observability, CapabilitiesReply,
    ExternalMtlsMacroFixture, HealthReply, HealthStatusCase, ObservedHealthStatus,
    ObservedHealthTrailer,
};
use crate::grpc_client::client::ContractProfile;
use crate::grpc_client::external_pb::magic::market::v1::{
    AdmissionState, CapabilitiesRequest, CapabilitiesResponse, Capability, ErrorDetail,
    HealthRequest, HealthResponse, Operation,
};
use crate::grpc_client::pb::magic::market::v1::{
    AdmissionState as LocalAdmissionState, Operation as LocalOperation,
};
use crate::push_foundation::intent_store::chain_post_close::macro_codec;
use crate::push_foundation::intent_store::chain_post_close::macro_stage::MacroControlOutcome;

const AUTHORITY: &str = "grpc-mtls:macro.test.invalid";
const STARTED_LOCAL: &str = "2026-09-14T15:31:00+08:00";
const OBSERVED_UTC: &str = "2026-09-14T07:31:00+00:00";
const REQUEST_HASH: &str = "fb86badeeebfca14c04928026fe295416a2a47c6534250291c066f3a74b67b9c";

pub(super) struct ExternalParentBaseline {
    pub(super) endpoint: String,
    pub(super) source: GrpcSource,
    pub(super) queries: crate::data_gateway::grpc_source::ConnectedBoardQueries,
    pub(super) stocks: Vec<crate::market_data::TopStock>,
    pub(super) config: crate::monitor::push_job::LocalChainPostCloseConfig,
    pub(super) intent: crate::monitor::push_job::IntentId,
    pub(super) final_bytes: Vec<u8>,
    pub(super) receipt: crate::database::data_acquisition_audit::DataAcquisitionAuditReceipt,
    pub(super) context: Vec<u8>,
    pub(super) head: u64,
    pub(super) tables: Vec<String>,
    pub(super) facts: BTreeMap<String, Vec<Vec<rusqlite::types::Value>>>,
    pub(super) audit: Vec<Vec<rusqlite::types::Value>>,
    pub(super) network: BoardLoopbackObservation,
    pub(super) memberships:
        Vec<crate::grpc_client::client::board_loopback_fixture::BoardMembershipLoopbackRequest>,
}

pub(super) async fn setup_external_parent(
    business: &mut V2BusinessFixture,
    parent_server: &mut Option<BoardLoopbackServer>,
    run_id: &str,
) -> ExternalParentBaseline {
    let baseline = setup_v10_parent(business, parent_server, run_id).await;
    assert_eq!(
        business
            .chain_post_close()
            .migrate_schema_v10_to_v11()
            .unwrap()
            .schema_version(),
        11
    );
    baseline
}

pub(super) async fn setup_v10_parent(
    business: &mut V2BusinessFixture,
    parent_server: &mut Option<BoardLoopbackServer>,
    run_id: &str,
) -> ExternalParentBaseline {
    let (endpoint, server) = tokio::time::timeout(
        Duration::from_secs(5),
        spawn_macro_parent_listener(DRAGON_TIGER_RECORDS.as_bytes()),
    )
    .await
    .expect("TEST_CODE External parent listener deadline");
    *parent_server = Some(server);
    let source =
        GrpcSource::from_board_loopback_test_client(connect_parent_instance(&endpoint).await);
    let queries = source.connected_board_queries().await.unwrap();
    let (stocks, config, intent, v9_head) =
        populate_completed_v9_parent(business, &queries, run_id).await;
    business
        .chain_post_close()
        .migrate_schema_v9_to_v10()
        .unwrap();
    let mut local = business
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let lease = local
        .resume_run(
            &intent,
            lease_request(
                "TEST_CODE_EXTERNAL_MACRO_PARENT_OWNER",
                68_300_000_000,
                90_000_000_000,
                Some(v9_head),
            ),
        )
        .unwrap();
    let clock = DragonTigerClock {
        now: UtcMicros::try_new(micros("2026-07-22T10:30:00+08:00")).unwrap(),
        request_observed_at: DateTime::parse_from_rfc3339("2026-07-22T10:30:00+08:00").unwrap(),
        request_calls: Cell::new(0),
        cache_calls: Cell::new(0),
    };
    let mut io = local
        .dragon_tiger_preparation_io_v10(
            lease,
            &queries,
            &clock,
            FixedClusterConfiguration::resolve(Some("2")),
            &source,
        )
        .unwrap();
    let stopped = prepare_chain_analysis_with_io(
        NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
        stocks.clone(),
        None,
        &mut io,
    )
    .await
    .expect_err("TEST_CODE External real parent reaches Macro guard");
    assert_partial_macro_stop(&stopped);
    drop(io);
    let parent = local.inspect_dragon_tiger(&intent).unwrap();
    assert_complete_batch(parent.batch().unwrap());
    let final_bytes = parent.final_bytes().unwrap().to_vec();
    let receipt = parent.audit_receipt().unwrap().clone();
    let run = local.inspect_run(&intent).unwrap();
    let context = run.context().canonical_bytes();
    let head = run.head_version();
    drop(local);
    let tables = table_names(business.connection());
    let facts = old_fact_rows(business.connection(), &tables);
    // Test-only fixed-table snapshot: the production audit API reads by receipt and
    // cannot prove that no additional audit row appeared while an RPC was gated.
    let audit = all_rows(
        business.connection(),
        "SELECT * FROM data_acquisition_audit ORDER BY id",
    );
    let network = parent_server.as_ref().unwrap().snapshot();
    let memberships = parent_server.as_ref().unwrap().membership_snapshot();
    assert_eq!(network.dragon_tiger_requests.len(), 1);
    ExternalParentBaseline {
        endpoint,
        source,
        queries,
        stocks,
        config,
        intent,
        final_bytes,
        receipt,
        context,
        head,
        tables,
        facts,
        audit,
        network,
        memberships,
    }
}

pub(super) struct ConfirmedExternalBaseline {
    pub(super) parent_endpoint: String,
    pub(super) config: crate::monitor::push_job::LocalChainPostCloseConfig,
    pub(super) intent: crate::monitor::push_job::IntentId,
}

pub(super) async fn establish_confirmed_external_first_source(
    business: &mut V2BusinessFixture,
    parent_server: &mut Option<BoardLoopbackServer>,
    external_server: &mut Option<ExternalMtlsMacroFixture>,
    run_id: &str,
) -> ConfirmedExternalBaseline {
    use crate::grpc_client::client::external_control_attempt::ExternalControlKind;
    use crate::grpc_client::client::external_control_loopback_fixture::ExternalMtlsMacroFixture;
    use crate::grpc_client::client::ContractProfile;
    use crate::grpc_client::external_pb::magic::market::v1::{
        AdmissionState, CapabilitiesRequest, CapabilitiesResponse, Capability, HealthRequest,
        HealthResponse,
    };
    use crate::grpc_client::pb::magic::market::v1::CanonicalPayload;
    use crate::push_foundation::intent_store::chain_post_close::macro_stage::MacroControlOutcome;

    const AUTHORITY: &str = "grpc-mtls:macro.test.invalid";
    const BATCH_ID: &str = "TEST_CODE_EXTERNAL_DATA_BATCH";
    const OBSERVED_AT: &str = "2026-09-14T15:31:00+08:00";
    const SOURCE_AT: &str = "2026-09-14 15:30";
    const RECORD_DATA: &[u8] = br#"{"item_id":"TEST_CODE_EXTERNAL_NEWS_001","title":"TEST_CODE external data title","summary":"TEST_CODE external data summary","content":"TEST_CODE external data content","publisher":"TEST_CODE Eastmoney publisher","url":"https://example.com/TEST_CODE_EXTERNAL_NEWS_001","published_at":"2026-09-14T15:30:00+08:00","instruments":[{"exchange":"Shanghai","code":"TEST_CODE_600001","asset_class":"Equity"}],"topics":["TEST_CODE_external_topic"],"language":"zh-CN","evidence":{"provider":"Eastmoney","source_at":"2026-09-14 15:30","observed_at":"2026-09-14T15:31:00+08:00","batch_id":"TEST_CODE_EXTERNAL_DATA_BATCH"}}"#;
    const EXPECTED_NATIVE: &[u8] = br#"{"evidence":{"batch_id":"TEST_CODE_EXTERNAL_DATA_BATCH","observed_at":"2026-09-14T15:31:00+08:00","provider":"Eastmoney","source":"eastmoney-web","source_at":"2026-09-14 15:30"},"kind":"Available","records":[{"canonical_url":"https://example.com/TEST_CODE_EXTERNAL_NEWS_001","content":"TEST_CODE external data content","evidence":{"batch_id":"TEST_CODE_EXTERNAL_DATA_BATCH","observed_at":"2026-09-14T15:31:00+08:00","provider":"Eastmoney","source_at":"2026-09-14 15:30"},"instruments":["TEST_CODE_600001"],"item_id":"TEST_CODE_EXTERNAL_NEWS_001","language":"zh-CN","observed_at":"2026-09-14T07:31:00+00:00","published_at":"2026-09-14T07:30:00+00:00","publisher":"TEST_CODE Eastmoney publisher","summary":"TEST_CODE external data summary","title":"TEST_CODE external data title","topics":["TEST_CODE_external_topic"]}],"version":1}"#;

    let baseline = setup_external_parent(
        business,
        parent_server,
        run_id,
    )
    .await;
    *external_server = Some(
        ExternalMtlsMacroFixture::bind_data_success_for_test()
            .await
            .expect("TEST_CODE External mTLS same-store fixture"),
    );
    let external = external_server
        .as_ref()
        .expect("TEST_CODE External mTLS owner");
    let parent_endpoint = baseline.endpoint;
    let parent_source = baseline.source;
    let queries = baseline.queries;
    let stocks = baseline.stocks;
    let config = baseline.config;
    let intent = baseline.intent;
    let parent_final = baseline.final_bytes;
    let parent_receipt = baseline.receipt;
    let parent_context = baseline.context;
    let parent_head = baseline.head;
    let earlier_tables = baseline.tables;
    let earlier_facts = baseline.facts;
    let audit_count = business.count("data_acquisition_audit");
    let old_network = baseline.network;
    let old_memberships = baseline.memberships;
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
    let started_at = micros("2026-09-14T15:31:00+08:00");
    let clock = MacroClock {
        now: Cell::new(UtcMicros::try_new(started_at).unwrap()),
        observation: DateTime::parse_from_rfc3339("2026-09-14T15:31:00+08:00")
            .unwrap(),
        observation_calls: Cell::new(0),
    };
    let registered = [
        GeneralWebResearchProvider::SerpApi,
        GeneralWebResearchProvider::Bocha,
        GeneralWebResearchProvider::Tavily,
    ];
    let search_service = macro_search_service(&registered);
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
                "TEST_CODE_EXTERNAL_MACRO_OWNER_A",
                started_at,
                started_at + 2_000_000,
                parent_head,
            ),
        )
        .unwrap();
    let mut io = local
        .macro_preparation_io_v11(
            lease,
            &queries,
            &clock,
            FixedClusterConfiguration::resolve(Some("2")),
            &parent_source,
            &macro_source,
            &search_service,
        )
        .unwrap();
    let inspect = || {
        let mut reader = BusinessIntentStore::open(&database).unwrap();
        let mut read_local = reader
            .single_user_local_chain_post_close(&config)
            .unwrap();
        let recovery = read_local.inspect_macro(&intent).unwrap();
        drop(read_local);
        reader.connection.close().unwrap();
        recovery
    };

    let captured = {
        let mut prepared = Box::pin(prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks.clone(),
            None,
            &mut io,
        ));
        let health_deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            tokio::select! {
                biased;
                result = &mut prepared => panic!(
                    "TEST_CODE External prepare returned before Health receipt: {result:?}"
                ),
                _ = tokio::task::yield_now() => {}
            }
            if !external.snapshot().health_requests.is_empty() {
                break;
            }
            assert!(
                std::time::Instant::now() < health_deadline,
                "TEST_CODE External Health receipt watchdog"
            );
        }
        let health_pending = inspect();
        assert!(health_pending.has_unconfirmed_effect());
        assert!(health_pending.attempts().is_empty());
        assert!(health_pending
            .global_news(GlobalNewsProvider::Eastmoney)
            .is_none());
        let plan = health_pending.plan();
        assert_eq!(plan.profile(), ContractProfile::ExternalV1);
        assert_eq!(plan.acquisition_authority(), Some(AUTHORITY));
        assert_eq!(plan.endpoint(), external.endpoint());
        assert_eq!(plan.started_at().get(), started_at);
        assert_eq!(plan.deadline_at().get(), started_at + 15_000_000);
        assert_eq!(plan.observed_local(), "2026-09-14T15:31:00+08:00");
        assert_eq!(plan.research_providers(), registered);
        assert_eq!(plan.research_decisions().len(), 3);
        assert!(plan
            .research_decisions()
            .iter()
            .all(|decision| decision.supports_general_web_search()));
        assert!(
            plan.research_decisions()
                .iter()
                .all(|decision| !decision.is_available()),
            "TEST_CODE_External_only_has_no_Local_SemanticSearch_connection"
        );
        assert_eq!(
            plan.research_decision_provenance(),
            macro_codec::ResearchDecisionProvenance::ExplicitRegistryV2
        );
        for (index, decision) in plan.research_decisions().iter().enumerate() {
            assert_eq!(
                decision.registration_ordinal(),
                Some(u32::try_from(index + 1).unwrap())
            );
            assert_eq!(
                decision.availability_source(),
                "explicit-registry-local-semantic-search-disconnected"
            );
            assert_eq!(decision.local_transport_endpoint(), None);
            assert_eq!(decision.remote_health(), Some("Unknown"));
        }
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
        assert!(controls[0].begin_version().is_some());
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
        let health_wire = HealthRequest::decode(health_bytes.as_slice()).unwrap();
        assert_eq!(health_wire.encode_to_vec(), health_bytes);
        assert_eq!(health_wire.context.as_ref().unwrap().protocol_version, 1);
        assert_eq!(health_wire.context.as_ref().unwrap().request_id, health_id);
        let capabilities_wire =
            CapabilitiesRequest::decode(capabilities_bytes.as_slice()).unwrap();
        assert_eq!(capabilities_wire.encode_to_vec(), capabilities_bytes);
        assert_eq!(
            capabilities_wire.context.as_ref().unwrap().protocol_version,
            1
        );
        assert_eq!(
            capabilities_wire.context.as_ref().unwrap().request_id,
            capabilities_id
        );
        assert_ne!(health_id, capabilities_id);
        assert_ne!(data_id, health_id);
        assert_ne!(data_id, capabilities_id);
        let plan_bytes = health_pending.plan_bytes().to_vec();
        for bytes in [
            plan_bytes.as_slice(),
            data_bytes.as_slice(),
            health_bytes.as_slice(),
            capabilities_bytes.as_slice(),
        ] {
            assert!(!bytes
                .windows(b"TEST_CODE_EXTERNAL_CONTROL_TOKEN".len())
                .any(|part| part == b"TEST_CODE_EXTERNAL_CONTROL_TOKEN"));
            assert!(!bytes
                .windows(b"authorization".len())
                .any(|part| part == b"authorization"));
        }
        let wire = external.snapshot();
        assert_eq!(wire.tcp_accepts, 1);
        assert_eq!(wire.health_requests, vec![health_bytes.clone()]);
        assert_eq!(wire.health_authorized, vec![true]);
        assert!(wire.health_responses.is_empty());
        assert_eq!(wire.capabilities_calls, 0);
        assert_eq!(wire.data_calls, 0);

        external.release_health();
        let capabilities_deadline =
            std::time::Instant::now() + Duration::from_secs(5);
        loop {
            tokio::select! {
                biased;
                result = &mut prepared => panic!(
                    "TEST_CODE External prepare returned before Capabilities receipt: {result:?}"
                ),
                _ = tokio::task::yield_now() => {}
            }
            if external.snapshot().capabilities_calls > 0 {
                break;
            }
            assert!(
                std::time::Instant::now() < capabilities_deadline,
                "TEST_CODE External Capabilities receipt watchdog"
            );
        }
        let expected_health = HealthResponse {
            request_id: health_id.clone(),
            live: true,
            ready: true,
            state: "TEST_CODE_HEALTH_RUNNING".to_owned(),
            observability: Some(test_external_observability()),
            build_identity: Some(test_external_build_identity()),
        };
        let expected_health_bytes = expected_health.encode_to_vec();
        let capabilities_pending = inspect();
        assert!(capabilities_pending.has_unconfirmed_effect());
        assert!(capabilities_pending.attempts().is_empty());
        assert_eq!(capabilities_pending.plan_bytes(), plan_bytes);
        let episode = &capabilities_pending.readiness_episodes()[0];
        assert_eq!(episode.ready_result_version(), None);
        let controls = episode.controls();
        assert_eq!(controls[0].request_bytes(), health_bytes);
        assert_eq!(controls[0].request_id(), health_id);
        assert!(controls[0].result_version().is_some());
        assert_eq!(controls[0].outcome(), Some(MacroControlOutcome::Ready));
        assert_eq!(
            controls[0].response_bytes(),
            Some(expected_health_bytes.as_slice())
        );
        assert_eq!(controls[1].request_bytes(), capabilities_bytes);
        assert_eq!(controls[1].request_id(), capabilities_id);
        assert!(controls[1].begin_version().is_some());
        assert_eq!(controls[1].result_version(), None);
        assert_eq!(controls[1].outcome(), None);
        assert_eq!(controls[1].response_bytes(), None);
        let wire = external.snapshot();
        assert_eq!(wire.tcp_accepts, 1);
        assert_eq!(wire.health_responses, vec![expected_health_bytes]);
        assert_eq!(wire.capabilities_calls, 1);
        assert_eq!(wire.capabilities_requests, vec![capabilities_bytes.clone()]);
        assert_eq!(wire.capabilities_authorized, vec![true]);
        assert!(wire.capabilities_responses.is_empty());
        assert_eq!(wire.data_calls, 0);

        clock
            .now
            .set(UtcMicros::try_new(started_at + 1_000_000).unwrap());
        external.release_capabilities();
        let observer = async {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                let recovery = inspect();
                assert_eq!(
                    external.snapshot().data_calls,
                    0,
                    "TEST_CODE data began before confirmed-control observer"
                );
                let ready = recovery.readiness_episodes().len() == 1
                    && recovery.readiness_episodes()[0].controls().len() == 2
                    && recovery.readiness_episodes()[0].controls()[0].outcome()
                        == Some(MacroControlOutcome::Ready)
                    && recovery.readiness_episodes()[0].controls()[1].outcome()
                        == Some(MacroControlOutcome::Ready)
                    && recovery.readiness_episodes()[0]
                        .ready_result_version()
                        .is_some()
                    && !recovery.has_unconfirmed_effect()
                    && recovery.attempts().is_empty()
                    && recovery
                        .global_news(GlobalNewsProvider::Eastmoney)
                        .is_none();
                if ready {
                    break recovery;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "TEST_CODE External confirmed-control observer watchdog"
                );
                tokio::task::yield_now().await;
            }
        };
        let ready = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::select! {
                biased;
                snapshot = observer => snapshot,
                result = &mut prepared => panic!(
                    "TEST_CODE External prepare escaped confirmed-control yield: {result:?}"
                )
            }
        })
        .await
        .expect("TEST_CODE External confirmed-control select watchdog");
        (
            ready,
            plan_bytes,
            data_id,
            data_bytes,
            health_id,
            health_bytes,
            capabilities_id,
            capabilities_bytes,
        )
    };
    drop(io);
    let (
        ready_recovery,
        plan_bytes,
        data_id,
        data_bytes,
        health_id,
        health_bytes,
        capabilities_id,
        capabilities_bytes,
    ) = captured;
    let episode = &ready_recovery.readiness_episodes()[0];
    let controls = episode.controls();
    let health_result_version = controls[0].result_version().unwrap();
    let ready_result_version = episode.ready_result_version().unwrap();
    assert_eq!(controls[1].result_version(), Some(ready_result_version));
    assert!(ready_result_version > health_result_version);
    let expected_capabilities = CapabilitiesResponse {
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
    };
    let expected_capabilities_bytes = expected_capabilities.encode_to_vec();
    assert_eq!(
        controls[1].response_bytes(),
        Some(expected_capabilities_bytes.as_slice())
    );
    assert_eq!(ready_recovery.plan_bytes(), plan_bytes);
    assert_eq!(ready_recovery.plan().deadline_at().get(), started_at + 15_000_000);
    assert_eq!(ready_recovery.parent_final_bytes(), parent_final);
    assert!(!ready_recovery.has_unconfirmed_effect());
    assert!(ready_recovery.attempts().is_empty());
    assert!(ready_recovery
        .global_news(GlobalNewsProvider::Eastmoney)
        .is_none());
    let ready_head = local.inspect_run(&intent).unwrap().head_version();
    assert_eq!(local.inspect_run(&intent).unwrap().lease_generation(), 3);
    assert_eq!(clock.observation_calls.get(), 1);
    drop(local);
    assert_eq!(business.count("data_acquisition_audit"), audit_count);
    let confirmed_wire = external.snapshot();
    assert_eq!(confirmed_wire.tcp_accepts, 1);
    assert_eq!(confirmed_wire.health_requests, vec![health_bytes]);
    assert_eq!(confirmed_wire.health_authorized, vec![true]);
    assert_eq!(confirmed_wire.capabilities_calls, 1);
    assert_eq!(confirmed_wire.capabilities_requests, vec![capabilities_bytes]);
    assert_eq!(confirmed_wire.capabilities_authorized, vec![true]);
    assert_eq!(
        confirmed_wire.capabilities_responses,
        vec![expected_capabilities_bytes]
    );
    assert_eq!(confirmed_wire.data_calls, 0);

    drop(queries);
    drop(parent_source);
    drop(macro_source);
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
                "TEST_CODE_EXTERNAL_MACRO_OWNER_B",
                started_at + 3_000_000,
                started_at + 10_000_000,
                ready_head,
            ),
        )
        .unwrap();
    let resume_head = local.inspect_run(&intent).unwrap().head_version();
    let changed_search_service =
        crate::search_service::SearchService::from_untyped_general_web_provider_for_test(
            GeneralWebResearchProvider::Bocha,
        );
    let mut io = local
        .macro_preparation_io_v11(
            lease,
            &reopened_queries,
            &reopened_clock,
            FixedClusterConfiguration::resolve(Some("2")),
            &reopened_parent_source,
            &reopened_macro_source,
            &changed_search_service,
        )
        .unwrap();
    let mut prepared = Box::pin(prepare_chain_analysis_with_io(
        NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
        stocks,
        None,
        &mut io,
    ));
    let data_deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        tokio::select! {
            biased;
            result = &mut prepared => panic!(
                "TEST_CODE External reopened prepare returned before data receipt: {result:?}"
            ),
            _ = tokio::task::yield_now() => {}
        }
        if external.snapshot().data_calls > 0 {
            break;
        }
        assert!(
            std::time::Instant::now() < data_deadline,
            "TEST_CODE External data receipt watchdog"
        );
    }
    let (data_pending, pending_audit_count) = {
        let mut reader = BusinessIntentStore::open(&database).unwrap();
        let mut read_local = reader
            .single_user_local_chain_post_close(&config)
            .unwrap();
        let recovery = read_local.inspect_macro(&intent).unwrap();
        drop(read_local);
        let count = reader
            .connection
            .query_row("SELECT COUNT(*) FROM data_acquisition_audit", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        reader.connection.close().unwrap();
        (recovery, count)
    };
    assert_eq!(pending_audit_count, audit_count);
    assert!(data_pending.has_unconfirmed_effect());
    assert_eq!(data_pending.plan_bytes(), plan_bytes);
    assert_eq!(data_pending.plan().started_at().get(), started_at);
    assert_eq!(data_pending.plan().deadline_at().get(), started_at + 15_000_000);
    assert_eq!(data_pending.plan().observed_local(), "2026-09-14T15:31:00+08:00");
    assert_eq!(reopened_clock.observation_calls.get(), 0);
    assert_eq!(data_pending.readiness_episodes().len(), 1);
    assert_eq!(
        data_pending.readiness_episodes()[0].ready_result_version(),
        Some(ready_result_version)
    );
    assert!(data_pending.readiness_episodes()[0]
        .controls()
        .iter()
        .all(|control| control.outcome() == Some(MacroControlOutcome::Ready)));
    assert_eq!(data_pending.attempts().len(), 1);
    let pending_attempt = &data_pending.attempts()[0];
    assert_eq!(pending_attempt.attempt_ordinal(), 1);
    assert_eq!(pending_attempt.request_id(), data_id);
    assert_eq!(pending_attempt.request_bytes(), data_bytes);
    assert_eq!(
        pending_attempt.readiness_result_version(),
        Some(ready_result_version)
    );
    assert!(pending_attempt.begin_version() > resume_head);
    assert_eq!(pending_attempt.result_version(), None);
    assert_eq!(pending_attempt.response_bytes(), None);
    assert!(data_pending
        .global_news(GlobalNewsProvider::Eastmoney)
        .is_none());
    let data_receipt = external.snapshot();
    assert_eq!(data_receipt.tcp_accepts, 2);
    assert_eq!(data_receipt.health_requests.len(), 1);
    assert_eq!(data_receipt.capabilities_calls, 1);
    assert_eq!(data_receipt.data_calls, 1);
    assert_eq!(data_receipt.data_methods, vec!["global_news"]);
    assert_eq!(data_receipt.data_requests, vec![data_bytes.clone()]);
    assert_eq!(data_receipt.data_authorized, vec![true]);
    assert!(data_receipt.data_responses.is_empty());

    external.release_data();
    let stopped = match tokio::time::timeout(Duration::from_secs(5), &mut prepared).await {
        Ok(Err(error)) => error,
        Ok(Ok(_)) => panic!("TEST_CODE External unexpectedly completed all Macro sources"),
        Err(_) => panic!("TEST_CODE External data completion watchdog elapsed"),
    };
    assert_partial_macro_stop(&stopped);
    drop(prepared);
    drop(io);
    let recovered = local.inspect_macro(&intent).unwrap();
    assert!(!recovered.is_complete());
    assert!(!recovered.has_unconfirmed_effect());
    assert_eq!(recovered.plan_bytes(), plan_bytes);
    assert_eq!(recovered.parent_final_bytes(), parent_final);
    assert_eq!(
        recovered.readiness_episodes()[0].ready_result_version(),
        Some(ready_result_version)
    );
    assert_eq!(recovered.attempts().len(), 1);
    let attempt = &recovered.attempts()[0];
    assert_eq!(attempt.request_id(), data_id);
    assert_eq!(attempt.request_bytes(), data_bytes);
    assert_eq!(
        attempt.readiness_result_version(),
        Some(ready_result_version)
    );
    assert!(attempt.result_version().unwrap() > attempt.begin_version());
    let response_bytes = attempt.response_bytes().unwrap().to_vec();
    let expected_response = QueryResponse {
        request_id: data_id,
        operation: LocalOperation::GlobalNews as i32,
        admission: LocalAdmissionState::Admitted as i32,
        selected_provider: "Eastmoney".to_owned(),
        batch_id: BATCH_ID.to_owned(),
        complete: true,
        observed_at: OBSERVED_AT.to_owned(),
        source_at: SOURCE_AT.to_owned(),
        records: vec![CanonicalPayload {
            schema: "magic.market.news_item".to_owned(),
            schema_version: 2,
            content_type: "application/json; charset=utf-8".to_owned(),
            data: RECORD_DATA.to_vec(),
        }],
        source: String::new(),
        diagnostic_blocker: String::new(),
    };
    assert_eq!(response_bytes, expected_response.encode_to_vec());
    let source = recovered
        .global_news(GlobalNewsProvider::Eastmoney)
        .unwrap();
    assert!(source.is_complete());
    assert_eq!(source.profile(), "ExternalV1");
    assert_eq!(source.acquisition_authority(), Some(AUTHORITY));
    assert_eq!(source.retry_policy(), (4, 1000, 60_000, 200));
    let batch = source.batch().unwrap();
    assert!(!batch.is_verified_empty());
    assert_eq!(batch.evidence().provider, ProviderId::Eastmoney);
    assert_eq!(batch.evidence().source, "eastmoney-web");
    assert_eq!(batch.evidence().source_at.as_deref(), Some(SOURCE_AT));
    assert_eq!(batch.evidence().observed_at, OBSERVED_AT);
    assert_eq!(batch.evidence().batch_id, BATCH_ID);
    assert_eq!(batch.records().len(), 1);
    let record = &batch.records()[0];
    assert_eq!(record.item_id, "TEST_CODE_EXTERNAL_NEWS_001");
    assert_eq!(record.title, "TEST_CODE external data title");
    assert_eq!(record.summary.as_deref(), Some("TEST_CODE external data summary"));
    assert_eq!(record.content.as_deref(), Some("TEST_CODE external data content"));
    assert_eq!(record.publisher, "TEST_CODE Eastmoney publisher");
    assert_eq!(
        record.canonical_url,
        "https://example.com/TEST_CODE_EXTERNAL_NEWS_001"
    );
    assert_eq!(
        record.published_at,
        DateTime::parse_from_rfc3339("2026-09-14T15:30:00+08:00")
            .unwrap()
            .with_timezone(&chrono::Utc)
    );
    assert_eq!(
        record.observed_at,
        DateTime::parse_from_rfc3339(OBSERVED_AT)
            .unwrap()
            .with_timezone(&chrono::Utc)
    );
    assert_eq!(record.instruments, vec!["TEST_CODE_600001"]);
    assert_eq!(record.topics, vec!["TEST_CODE_external_topic"]);
    assert_eq!(record.language, "zh-CN");
    assert_eq!(record.evidence.provider(), ProviderId::Eastmoney);
    assert_eq!(record.evidence.source_at(), Some(SOURCE_AT));
    assert_eq!(record.evidence.observed_at(), OBSERVED_AT);
    assert_eq!(record.evidence.batch_id(), BATCH_ID);
    assert_eq!(source.final_bytes().unwrap(), EXPECTED_NATIVE);
    let receipt = source.audit_receipt().unwrap().clone();
    assert_eq!(receipt.previous_outcome, None);
    assert_eq!(receipt.current_outcome, "available");
    assert_eq!(recovered.pending_source_identities().len(), 4);
    assert_eq!(recovered.pending_research_queries().len(), 6);
    assert_eq!(reopened_clock.observation_calls.get(), 0);
    assert_eq!(
        local
            .inspect_run(&intent)
            .unwrap()
            .context()
            .canonical_bytes(),
        parent_context
    );
    drop(local);
    assert_eq!(business.count("data_acquisition_audit"), audit_count + 1);

    let final_wire = external.snapshot();
    assert_eq!(final_wire.tcp_accepts, 2);
    assert_eq!(final_wire.health_requests.len(), 1);
    assert_eq!(final_wire.capabilities_calls, 1);
    assert_eq!(final_wire.data_calls, 1);
    assert_eq!(final_wire.data_requests, vec![data_bytes]);
    assert_eq!(final_wire.data_authorized, vec![true]);
    assert_eq!(final_wire.data_responses, vec![response_bytes]);
    assert!(final_wire.data_statuses.is_empty());
    assert_eq!(parent_server.as_ref().unwrap().snapshot(), old_network);
    assert_eq!(
        parent_server.as_ref().unwrap().membership_snapshot(),
        old_memberships
    );
    assert_eq!(
        old_fact_rows(business.connection(), &earlier_tables),
        earlier_facts
    );
    let transaction = business.connection().unchecked_transaction().unwrap();
    let verified = read_acquisition_in_transaction(&transaction, &receipt).unwrap();
    assert_eq!(verified.receipt(), &receipt);
    let audit = verified.record();
    assert_eq!(audit.capability, "GlobalNews-Eastmoney");
    assert_eq!(audit.provider, "Eastmoney");
    assert_eq!(audit.source, "eastmoney-web");
    assert_eq!(
        audit.request_hash,
        "fb86badeeebfca14c04928026fe295416a2a47c6534250291c066f3a74b67b9c"
    );
    assert_eq!(audit.source_at, Some(SOURCE_AT));
    assert_eq!(audit.observed_at, OBSERVED_AT);
    assert_eq!(audit.batch_id, Some(BATCH_ID));
    assert_eq!(audit.outcome, "available");
    assert_eq!(
        (
            audit.request_count,
            audit.accepted_count,
            audit.rejected_count
        ),
        (1, 1, 0)
    );
    assert_eq!(audit.reason_code, "accepted");
    assert!(!audit.retryable);
    read_acquisition_in_transaction(&transaction, &parent_receipt).unwrap();
    transaction.commit().unwrap();
    drop(reopened_queries);
    drop(reopened_parent_source);
    drop(reopened_macro_source);

    ConfirmedExternalBaseline {
        parent_endpoint,
        config,
        intent,
    }
}

pub(super) fn assert_connect_unavailable_source(
    recovery: &crate::push_foundation::intent_store::chain_post_close::macro_stage::MacroRecovery,
) -> crate::database::data_acquisition_audit::DataAcquisitionAuditReceipt {
    let source = recovery.global_news(GlobalNewsProvider::Eastmoney).unwrap();
    assert!(source.is_complete());
    assert_eq!(source.profile(), "ExternalV1");
    assert_eq!(source.acquisition_authority(), Some(AUTHORITY));
    assert_eq!(source.retry_policy(), (4, 1000, 60_000, 200));
    assert!(source.batch().is_none());
    let error = source.error().unwrap();
    assert_eq!(error.capability(), "GrpcExternalV1");
    assert_eq!(error.provider(), None);
    assert_eq!(error.audit_outcome(), "unavailable");
    assert_eq!(error.reason_code(), "external_transport_unavailable");
    assert!(error.retryable());
    assert_eq!(
        error.message(),
        "ExternalV1 client-bundle 连接或 readiness 检查失败"
    );
    assert_eq!(source.final_bytes().unwrap(), RejectionCase::HealthConnectUnavailable.native());
    let receipt = source.audit_receipt().unwrap().clone();
    assert_eq!(receipt.previous_outcome, None);
    assert_eq!(receipt.current_outcome, "unavailable");
    assert_eq!(recovery.pending_source_identities(), pending_sources().as_slice());
    assert_eq!(recovery.pending_research_queries(), pending_research().as_slice());
    receipt
}

pub(super) fn assert_connect_unavailable_audit(
    connection: &Connection,
    receipt: &crate::database::data_acquisition_audit::DataAcquisitionAuditReceipt,
    parent_receipt: &crate::database::data_acquisition_audit::DataAcquisitionAuditReceipt,
    expected_observed_utc: &str,
) {
    let transaction = connection.unchecked_transaction().unwrap();
    let verified = read_acquisition_in_transaction(&transaction, receipt).unwrap();
    assert_eq!(verified.receipt(), receipt);
    let audit = verified.record();
    assert_eq!(audit.capability, "GlobalNews-Eastmoney");
    assert_eq!(audit.provider, "Eastmoney");
    assert_eq!(audit.source, "review-data-gateway");
    assert_eq!(audit.request_hash, REQUEST_HASH);
    assert_eq!(audit.source_at, None);
    assert_eq!(audit.observed_at, expected_observed_utc);
    assert_eq!(audit.batch_id, None);
    assert_eq!(audit.outcome, "unavailable");
    assert_eq!((audit.request_count, audit.accepted_count, audit.rejected_count), (1, 0, 1));
    assert_eq!(audit.reason_code, "external_transport_unavailable");
    assert!(audit.retryable);
    read_acquisition_in_transaction(&transaction, parent_receipt).unwrap();
    transaction.commit().unwrap();
}

pub(super) fn assert_same_terminal_recovery(
    before: &crate::push_foundation::intent_store::chain_post_close::macro_stage::MacroRecovery,
    after: &crate::push_foundation::intent_store::chain_post_close::macro_stage::MacroRecovery,
) {
    assert_eq!(after.is_complete(), before.is_complete());
    assert_eq!(after.has_unconfirmed_effect(), before.has_unconfirmed_effect());
    assert_eq!(after.plan_bytes(), before.plan_bytes());
    assert_eq!(after.parent_final_bytes(), before.parent_final_bytes());
    let (before_plan, after_plan) = (before.plan(), after.plan());
    assert_eq!(after_plan.profile(), before_plan.profile());
    assert_eq!(after_plan.acquisition_authority(), before_plan.acquisition_authority());
    assert_eq!(after_plan.endpoint(), before_plan.endpoint());
    assert_eq!(after_plan.started_at(), before_plan.started_at());
    assert_eq!(after_plan.deadline_at(), before_plan.deadline_at());
    assert_eq!(after_plan.observed_local(), before_plan.observed_local());
    let (before_request, after_request) =
        (before_plan.first_source_request(), after_plan.first_source_request());
    assert_eq!(after_request.request_id(), before_request.request_id());
    assert_eq!(after_request.request_bytes(), before_request.request_bytes());
    assert_eq!(after_request.retry_policy(), before_request.retry_policy());
    assert_eq!(after.attempts().len(), before.attempts().len());
    assert!(after.attempts().is_empty());
    let (before_episodes, after_episodes) =
        (before.readiness_episodes(), after.readiness_episodes());
    assert_eq!(after_episodes.len(), before_episodes.len());
    for (before_episode, after_episode) in before_episodes.iter().zip(after_episodes) {
        assert_eq!(after_episode.episode_ordinal(), before_episode.episode_ordinal());
        assert_eq!(after_episode.initiating_source(), before_episode.initiating_source());
        assert_eq!(after_episode.ready_result_version(), before_episode.ready_result_version());
        assert_eq!(after_episode.controls().len(), before_episode.controls().len());
        for (before_control, after_control) in
            before_episode.controls().iter().zip(after_episode.controls())
        {
            assert_eq!(after_control.kind(), before_control.kind());
            assert_eq!(after_control.request_id(), before_control.request_id());
            assert_eq!(after_control.request_bytes(), before_control.request_bytes());
            assert_eq!(after_control.begin_version(), before_control.begin_version());
            assert_eq!(after_control.result_version(), before_control.result_version());
            assert_eq!(after_control.outcome(), before_control.outcome());
            assert_eq!(after_control.response_bytes(), before_control.response_bytes());
        }
    }
    let before_source = before.global_news(GlobalNewsProvider::Eastmoney).unwrap();
    let after_source = after.global_news(GlobalNewsProvider::Eastmoney).unwrap();
    assert_eq!(after_source.is_complete(), before_source.is_complete());
    assert_eq!(after_source.profile(), before_source.profile());
    assert_eq!(after_source.acquisition_authority(), before_source.acquisition_authority());
    assert_eq!(after_source.retry_policy(), before_source.retry_policy());
    assert!(before_source.batch().is_none());
    assert!(after_source.batch().is_none());
    let (before_error, after_error) = (before_source.error().unwrap(), after_source.error().unwrap());
    assert_eq!(after_error.capability(), before_error.capability());
    assert_eq!(after_error.provider(), before_error.provider());
    assert_eq!(after_error.audit_outcome(), before_error.audit_outcome());
    assert_eq!(after_error.reason_code(), before_error.reason_code());
    assert_eq!(after_error.retryable(), before_error.retryable());
    assert_eq!(after_error.message(), before_error.message());
    assert_eq!(after_source.final_bytes(), before_source.final_bytes());
    assert_eq!(after_source.audit_receipt(), before_source.audit_receipt());
    assert_eq!(after.pending_source_identities(), before.pending_source_identities());
    assert_eq!(after.pending_research_queries(), before.pending_research_queries());
}

pub(super) async fn cleanup_external_case(
    business: &mut V2BusinessFixture,
    parent_server: &mut Option<BoardLoopbackServer>,
    external_server: &mut Option<ExternalMtlsMacroFixture>,
    label: &str,
) {
    let external_cleanup = match external_server.take() {
        Some(server) => {
            std::panic::AssertUnwindSafe(server.finish())
                .catch_unwind()
                .await
        }
        None => Ok(Ok(())),
    };
    let parent_cleanup = match parent_server.take() {
        Some(server) => std::panic::AssertUnwindSafe(server.finish())
            .catch_unwind()
            .await
            .map(|_| ()),
        None => Ok(()),
    };
    let database_cleanup = business.store.take().map(|store| {
        store.connection.close().map_err(|(connection, error)| {
            drop(connection);
            error
        })
    });
    external_cleanup
        .unwrap_or_else(|_| panic!("TEST_CODE {label} mTLS cleanup panic"))
        .unwrap_or_else(|error| panic!("TEST_CODE {label} mTLS cleanup: {error}"));
    parent_cleanup.unwrap_or_else(|_| panic!("TEST_CODE {label} parent cleanup panic"));
    if let Some(result) = database_cleanup {
        result.unwrap_or_else(|error| panic!("TEST_CODE {label} database cleanup: {error}"));
    }
}

#[derive(Clone, Copy)]
pub(super) enum RejectionCase {
    HealthConnectUnavailable,
    HealthNotReady,
    HealthStatus(HealthStatusCase),
    HealthMismatchedId,
    CapabilitiesStatus(HealthStatusCase),
    CapabilitiesMismatchedId,
    CapabilitiesMissingGlobalNews,
    CapabilitiesUnadmitted,
    CapabilitiesRuntimeUnavailable,
}

impl RejectionCase {
    fn is_health(self) -> bool {
        matches!(
            self,
            Self::HealthConnectUnavailable
                | Self::HealthNotReady
                | Self::HealthStatus(_)
                | Self::HealthMismatchedId
        )
    }

    fn is_connect_unavailable(self) -> bool {
        matches!(self, Self::HealthConnectUnavailable)
    }

    fn label(self) -> &'static str {
        match self {
            Self::HealthConnectUnavailable => "Health-connect unavailable",
            Self::HealthNotReady => "Health-not-ready",
            Self::HealthStatus(HealthStatusCase::Absent) => "Health-status-absent",
            Self::HealthStatus(HealthStatusCase::Bytes) => "Health-status-bytes",
            Self::HealthStatus(HealthStatusCase::Malformed) => "Health-status-malformed",
            Self::HealthMismatchedId => "Health-mismatched-id",
            Self::CapabilitiesStatus(HealthStatusCase::Absent) => "Caps-status-absent",
            Self::CapabilitiesStatus(HealthStatusCase::Bytes) => "Caps-status-bytes",
            Self::CapabilitiesStatus(HealthStatusCase::Malformed) => "Caps-status-malformed",
            Self::CapabilitiesMismatchedId => "Caps-mismatched-id",
            Self::CapabilitiesMissingGlobalNews => "Caps-missing-GlobalNews",
            Self::CapabilitiesUnadmitted => "Caps-unadmitted-GlobalNews",
            Self::CapabilitiesRuntimeUnavailable => "Caps-runtime-unavailable",
        }
    }

    fn run_id(self) -> &'static str {
        match self {
            Self::HealthConnectUnavailable => {
                "TEST_CODE_RUN_EXTERNAL_MACRO_HEALTH_CONNECT_UNAVAILABLE"
            }
            Self::HealthNotReady => "TEST_CODE_RUN_EXTERNAL_HEALTH_NOT_READY",
            Self::HealthStatus(HealthStatusCase::Absent) => {
                "TEST_CODE_RUN_EXTERNAL_HEALTH_STATUS_ABSENT"
            }
            Self::HealthStatus(HealthStatusCase::Bytes) => {
                "TEST_CODE_RUN_EXTERNAL_HEALTH_STATUS_BYTES"
            }
            Self::HealthStatus(HealthStatusCase::Malformed) => {
                "TEST_CODE_RUN_EXTERNAL_HEALTH_STATUS_MALFORMED"
            }
            Self::HealthMismatchedId => "TEST_CODE_RUN_EXTERNAL_HEALTH_MISMATCHED_ID",
            Self::CapabilitiesStatus(HealthStatusCase::Absent) => {
                "TEST_CODE_RUN_EXTERNAL_CAPS_STATUS_ABSENT"
            }
            Self::CapabilitiesStatus(HealthStatusCase::Bytes) => {
                "TEST_CODE_RUN_EXTERNAL_CAPS_STATUS_BYTES"
            }
            Self::CapabilitiesStatus(HealthStatusCase::Malformed) => {
                "TEST_CODE_RUN_EXTERNAL_CAPS_STATUS_MALFORMED"
            }
            Self::CapabilitiesMismatchedId => "TEST_CODE_RUN_EXTERNAL_CAPS_MISMATCHED_ID",
            Self::CapabilitiesMissingGlobalNews => {
                "TEST_CODE_RUN_EXTERNAL_CAPS_MISSING_GLOBAL_NEWS"
            }
            Self::CapabilitiesUnadmitted => "TEST_CODE_RUN_EXTERNAL_CAPS_UNADMITTED",
            Self::CapabilitiesRuntimeUnavailable => {
                "TEST_CODE_RUN_EXTERNAL_CAPS_RUNTIME_UNAVAILABLE"
            }
        }
    }

    fn owner_a(self) -> &'static str {
        if self.is_connect_unavailable() {
            "TEST_CODE_EXTERNAL_HEALTH_CONNECT_OWNER_A"
        } else {
            "TEST_CODE_EXTERNAL_REJECTION_OWNER_A"
        }
    }

    fn owner_b(self) -> &'static str {
        if self.is_connect_unavailable() {
            "TEST_CODE_EXTERNAL_HEALTH_CONNECT_OWNER_B"
        } else {
            "TEST_CODE_EXTERNAL_REJECTION_OWNER_B"
        }
    }

    fn outcome(self) -> &'static str {
        match self {
            Self::CapabilitiesMissingGlobalNews | Self::CapabilitiesUnadmitted => "invalid_request",
            _ => "unavailable",
        }
    }

    fn provider(self) -> Option<ProviderId> {
        match self {
            Self::HealthStatus(HealthStatusCase::Absent | HealthStatusCase::Bytes)
            | Self::CapabilitiesStatus(HealthStatusCase::Absent | HealthStatusCase::Bytes) => {
                Some(ProviderId::Eastmoney)
            }
            _ => None,
        }
    }

    fn reason(self) -> &'static str {
        match self {
            Self::HealthConnectUnavailable => "external_transport_unavailable",
            Self::HealthNotReady => "external_health_not_ready",
            Self::HealthStatus(HealthStatusCase::Absent | HealthStatusCase::Bytes)
            | Self::CapabilitiesStatus(HealthStatusCase::Absent | HealthStatusCase::Bytes) => {
                "unavailable"
            }
            Self::HealthStatus(HealthStatusCase::Malformed)
            | Self::CapabilitiesStatus(HealthStatusCase::Malformed) => {
                "external_transport_unavailable"
            }
            Self::HealthMismatchedId | Self::CapabilitiesMismatchedId => "internal",
            Self::CapabilitiesMissingGlobalNews => "external_capability_missing",
            Self::CapabilitiesUnadmitted => "external_capability_unadmitted",
            Self::CapabilitiesRuntimeUnavailable => "external_capability_runtime_unavailable",
        }
    }

    fn retryable(self) -> bool {
        matches!(
            self,
            Self::HealthConnectUnavailable
                | Self::HealthNotReady
                | Self::HealthStatus(HealthStatusCase::Malformed)
                | Self::CapabilitiesStatus(HealthStatusCase::Malformed)
                | Self::CapabilitiesRuntimeUnavailable
        )
    }

    fn message(self) -> &'static str {
        match self {
            Self::HealthConnectUnavailable => "ExternalV1 client-bundle 连接或 readiness 检查失败",
            Self::HealthNotReady => "ExternalV1 health 未达到 live+ready",
            Self::HealthStatus(_) | Self::CapabilitiesStatus(_) => {
                "ExternalV1 client-bundle 连接或 readiness 检查失败: [redacted-unclassified-status]"
            }
            Self::HealthMismatchedId | Self::CapabilitiesMismatchedId => {
                "ExternalV1 client-bundle 连接或 readiness 检查失败"
            }
            Self::CapabilitiesMissingGlobalNews => {
                "ExternalV1 GlobalNews: 服务端没有发布该语义族合同"
            }
            Self::CapabilitiesUnadmitted => "ExternalV1 GlobalNews: 该语义族只有未准入或诊断能力",
            Self::CapabilitiesRuntimeUnavailable => {
                "ExternalV1 GlobalNews: 已准入语义族的 runtime provider 暂不可用"
            }
        }
    }

    fn native(self) -> &'static [u8] {
        match self {
            Self::HealthConnectUnavailable => r#"{"error":{"audit_outcome":"unavailable","capability":"GrpcExternalV1","message":"ExternalV1 client-bundle 连接或 readiness 检查失败","provider":null,"reason_code":"external_transport_unavailable","retryable":true},"kind":"Error","version":1}"#.as_bytes(),
            Self::HealthNotReady => r#"{"error":{"audit_outcome":"unavailable","capability":"GrpcExternalV1","message":"ExternalV1 health 未达到 live+ready","provider":null,"reason_code":"external_health_not_ready","retryable":true},"kind":"Error","version":1}"#.as_bytes(),
            Self::HealthStatus(HealthStatusCase::Absent | HealthStatusCase::Bytes)
            | Self::CapabilitiesStatus(HealthStatusCase::Absent | HealthStatusCase::Bytes) => r#"{"error":{"audit_outcome":"unavailable","capability":"GrpcExternalV1","message":"ExternalV1 client-bundle 连接或 readiness 检查失败: [redacted-unclassified-status]","provider":"Eastmoney","reason_code":"unavailable","retryable":false},"kind":"Error","version":1}"#.as_bytes(),
            Self::HealthStatus(HealthStatusCase::Malformed)
            | Self::CapabilitiesStatus(HealthStatusCase::Malformed) => r#"{"error":{"audit_outcome":"unavailable","capability":"GrpcExternalV1","message":"ExternalV1 client-bundle 连接或 readiness 检查失败: [redacted-unclassified-status]","provider":null,"reason_code":"external_transport_unavailable","retryable":true},"kind":"Error","version":1}"#.as_bytes(),
            Self::HealthMismatchedId | Self::CapabilitiesMismatchedId => r#"{"error":{"audit_outcome":"unavailable","capability":"GrpcExternalV1","message":"ExternalV1 client-bundle 连接或 readiness 检查失败","provider":null,"reason_code":"internal","retryable":false},"kind":"Error","version":1}"#.as_bytes(),
            Self::CapabilitiesMissingGlobalNews => r#"{"error":{"audit_outcome":"invalid_request","capability":"GrpcExternalV1","message":"ExternalV1 GlobalNews: 服务端没有发布该语义族合同","provider":null,"reason_code":"external_capability_missing","retryable":false},"kind":"Error","version":1}"#.as_bytes(),
            Self::CapabilitiesUnadmitted => r#"{"error":{"audit_outcome":"invalid_request","capability":"GrpcExternalV1","message":"ExternalV1 GlobalNews: 该语义族只有未准入或诊断能力","provider":null,"reason_code":"external_capability_unadmitted","retryable":false},"kind":"Error","version":1}"#.as_bytes(),
            Self::CapabilitiesRuntimeUnavailable => r#"{"error":{"audit_outcome":"unavailable","capability":"GrpcExternalV1","message":"ExternalV1 GlobalNews: 已准入语义族的 runtime provider 暂不可用","provider":null,"reason_code":"external_capability_runtime_unavailable","retryable":true},"kind":"Error","version":1}"#.as_bytes(),
        }
    }
}

fn expected_capabilities(request_id: String, case: RejectionCase) -> CapabilitiesResponse {
    let global_news = Capability {
        operation: Operation::GlobalNews as i32,
        repository_admission: AdmissionState::Admitted as i32,
        runtime_available: true,
        provider: "Eastmoney".to_owned(),
        exact_scope: "TEST_CODE_GLOBAL_NEWS_EASTMONEY_20".to_owned(),
        blocker: String::new(),
        diagnostic_available: true,
    };
    let semantic = Capability {
        operation: Operation::SemanticSearch as i32,
        repository_admission: AdmissionState::Unadmitted as i32,
        runtime_available: false,
        provider: "Bocha".to_owned(),
        exact_scope: "TEST_CODE_UNDELIVERED_SEMANTIC_SEARCH".to_owned(),
        blocker: "TEST_CODE_CAPABILITY_BLOCKED".to_owned(),
        diagnostic_available: false,
    };
    let capabilities = match case {
        RejectionCase::CapabilitiesMissingGlobalNews => vec![semantic],
        RejectionCase::CapabilitiesUnadmitted => vec![
            Capability {
                repository_admission: AdmissionState::Unadmitted as i32,
                ..global_news
            },
            semantic,
        ],
        RejectionCase::CapabilitiesRuntimeUnavailable => vec![
            Capability {
                runtime_available: false,
                ..global_news
            },
            semantic,
        ],
        _ => vec![global_news, semantic],
    };
    CapabilitiesResponse {
        request_id: if matches!(case, RejectionCase::CapabilitiesMismatchedId) {
            "TEST_CODE_WRONG_CAPABILITIES_REQUEST_ID".to_owned()
        } else {
            request_id
        },
        capabilities,
    }
}

pub(super) fn pending_sources() -> Vec<MacroQueryIdentity> {
    vec![
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

pub(super) fn pending_research() -> Vec<String> {
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

fn audit_snapshot(connection: &Connection) -> Vec<Vec<rusqlite::types::Value>> {
    // cfg(test)-only fixed reader: see setup_external_parent for the API boundary.
    all_rows(
        connection,
        "SELECT * FROM data_acquisition_audit ORDER BY id",
    )
}

pub(super) fn audit_snapshot_at(
    database: &std::path::Path,
) -> Vec<Vec<rusqlite::types::Value>> {
    let reader = BusinessIntentStore::open(database).unwrap();
    let rows = audit_snapshot(&reader.connection);
    reader.connection.close().unwrap();
    rows
}

pub(super) fn raw_control_bytes(database: &std::path::Path, intent: &IntentId, ordinal: i64) -> Vec<u8> {
    let reader = BusinessIntentStore::open(database).unwrap();
    let bytes = reader
        .connection
        .query_row(
            "SELECT bytes FROM chain_post_close_macro_control_attempt_results \
             WHERE intent_id=?1 AND episode_ordinal=1 AND control_ordinal=?2",
            rusqlite::params![intent.as_str(), ordinal],
            |row| row.get(0),
        )
        .unwrap();
    reader.connection.close().unwrap();
    bytes
}

pub(super) fn assert_response_raw(bytes: &[u8], response: &[u8]) {
    let actual: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(actual.as_object().unwrap().len(), 7);
    assert_eq!(actual, serde_json::json!({
        "version": 1, "connect_unavailable": false, "response": response,
        "code": null, "details": null, "trailer": "Absent", "diagnostic": null,
    }));
}

pub(super) fn assert_connect_unavailable_raw(bytes: &[u8]) {
    let actual: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(actual.as_object().unwrap().len(), 7);
    assert_eq!(actual, serde_json::json!({
        "version": 1, "connect_unavailable": true, "response": null,
        "code": null, "details": null, "trailer": "Absent", "diagnostic": null,
    }));
}

fn expected_status(case: HealthStatusCase, request_id: &str) -> (Vec<u8>, ObservedHealthStatus) {
    let detail = ErrorDetail {
        request_id: request_id.to_owned(),
        provider: "Eastmoney".to_owned(),
        reason_code: "unavailable".to_owned(),
        retryable: false,
        ..ErrorDetail::default()
    }
    .encode_to_vec();
    let details = if case == HealthStatusCase::Bytes {
        Vec::new()
    } else {
        detail.clone()
    };
    let trailer = match case {
        HealthStatusCase::Absent => ObservedHealthTrailer::Absent,
        HealthStatusCase::Bytes => ObservedHealthTrailer::Bytes(detail),
        HealthStatusCase::Malformed => ObservedHealthTrailer::Malformed,
    };
    (
        details.clone(),
        ObservedHealthStatus {
            code: 14,
            details,
            trailer,
        },
    )
}

fn assert_raw(
    case: RejectionCase,
    bytes: &[u8],
    expected_response: Option<&[u8]>,
    request_id: &str,
) {
    if case.is_connect_unavailable() {
        assert_connect_unavailable_raw(bytes);
        return;
    }
    let actual: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(actual.as_object().map(|object| object.len()), Some(7));
    let status_case = match case {
        RejectionCase::HealthStatus(value) | RejectionCase::CapabilitiesStatus(value) => {
            Some(value)
        }
        _ => None,
    };
    let (details, trailer) = if let Some(value) = status_case {
        let (details, observed) = expected_status(value, request_id);
        let trailer = match observed.trailer {
            ObservedHealthTrailer::Absent => serde_json::json!("Absent"),
            ObservedHealthTrailer::Bytes(bytes) => serde_json::json!({ "Bytes": bytes }),
            ObservedHealthTrailer::Malformed => serde_json::json!("Malformed"),
        };
        (Some(details), trailer)
    } else {
        (None, serde_json::json!("Absent"))
    };
    assert_eq!(actual.get("version"), Some(&serde_json::json!(1)));
    assert_eq!(
        actual.get("connect_unavailable"),
        Some(&serde_json::json!(case.is_connect_unavailable()))
    );
    assert_eq!(
        actual.get("response"),
        Some(&serde_json::json!(expected_response))
    );
    assert_eq!(
        actual.get("code"),
        Some(&serde_json::json!(status_case.map(|_| 14)))
    );
    assert_eq!(actual.get("details"), Some(&serde_json::json!(details)));
    assert_eq!(actual.get("trailer"), Some(&trailer));
    assert_eq!(
        actual.get("diagnostic"),
        Some(&if status_case.is_some() {
            serde_json::json!("[redacted-unclassified-status]")
        } else {
            serde_json::Value::Null
        })
    );
}

async fn bind_case(case: RejectionCase) -> Result<ExternalMtlsMacroFixture, String> {
    match case {
        RejectionCase::HealthConnectUnavailable => {
            let fixture = ExternalMtlsMacroFixture::bind_data_success_for_test().await?;
            fixture.set_reject_new_connections_for_test(true);
            Ok(fixture)
        }
        RejectionCase::HealthNotReady => {
            ExternalMtlsMacroFixture::bind_health_not_ready_for_test().await
        }
        RejectionCase::HealthStatus(value) => {
            ExternalMtlsMacroFixture::bind_health_reply_for_test(HealthReply::Status(value)).await
        }
        RejectionCase::HealthMismatchedId => {
            ExternalMtlsMacroFixture::bind_health_reply_for_test(HealthReply::MismatchedId).await
        }
        RejectionCase::CapabilitiesStatus(value) => {
            ExternalMtlsMacroFixture::bind_capabilities_reply_for_test(CapabilitiesReply::Status(
                value,
            ))
            .await
        }
        RejectionCase::CapabilitiesMismatchedId => {
            ExternalMtlsMacroFixture::bind_capabilities_reply_for_test(
                CapabilitiesReply::MismatchedId,
            )
            .await
        }
        RejectionCase::CapabilitiesMissingGlobalNews => {
            ExternalMtlsMacroFixture::bind_capabilities_reply_for_test(
                CapabilitiesReply::MissingGlobalNews,
            )
            .await
        }
        RejectionCase::CapabilitiesUnadmitted => {
            ExternalMtlsMacroFixture::bind_capabilities_reply_for_test(
                CapabilitiesReply::GlobalNewsUnadmitted,
            )
            .await
        }
        RejectionCase::CapabilitiesRuntimeUnavailable => {
            ExternalMtlsMacroFixture::bind_capabilities_reply_for_test(
                CapabilitiesReply::GlobalNewsRuntimeUnavailable,
            )
            .await
        }
    }
}

pub(super) async fn run_control_rejection(case: RejectionCase) {
    let mut business = V2BusinessFixture::new();
    let mut parent_server = None;
    let mut external_server = None;
    let body = std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(120), async {
        let baseline = setup_external_parent(
            &mut business,
            &mut parent_server,
            case.run_id(),
        ).await;
        external_server = Some(bind_case(case).await.expect("TEST_CODE rejection mTLS fixture"));
        let external = external_server.as_ref().unwrap();
        let database = business.database();
        let macro_source = GrpcSource::from_external_macro_bundle_for_test(external.bundle_path().to_path_buf());
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;
        assert_eq!(external.snapshot().tcp_accepts, 0);
        assert!(external.snapshot().health_requests.is_empty());
        assert_eq!(external.snapshot().capabilities_calls, 0);
        assert_eq!(external.snapshot().data_calls, 0);
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
        let mut local = business.store.as_mut().unwrap().single_user_local_chain_post_close(&baseline.config).unwrap();
        let lease = local.resume_run(&baseline.intent, macro_lease(
            case.owner_a(), started_at, started_at + 2_000_000, baseline.head,
        )).unwrap();
        let mut io = local.macro_preparation_io_v11(
            lease, &baseline.queries, &clock, FixedClusterConfiguration::resolve(Some("2")),
            &baseline.source, &macro_source, &search_service,
        ).unwrap();
        let mut prepared = Box::pin(prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(), baseline.stocks.clone(), None, &mut io,
        ));
        let early_stop = if case.is_connect_unavailable() {
            let stopped = match tokio::time::timeout(Duration::from_secs(5), &mut prepared).await {
                Ok(Err(error)) => error,
                Ok(Ok(_)) => panic!("TEST_CODE {} unexpectedly completed Macro", case.label()),
                Err(_) => panic!("TEST_CODE {} response watchdog elapsed", case.label()),
            };
            assert_partial_macro_stop(&stopped);
            Some(stopped)
        } else {
            let receipt_deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                tokio::select! {
                    biased;
                    result = &mut prepared => panic!("TEST_CODE {} returned before target receipt: {result:?}", case.label()),
                    _ = tokio::task::yield_now() => {}
                }
                let wire = external.snapshot();
                if wire.health_requests.len() == 1 {
                    break;
                }
                assert!(std::time::Instant::now() < receipt_deadline, "TEST_CODE {} receipt watchdog", case.label());
            }
            let health_pending = {
                let mut reader = BusinessIntentStore::open(&database).unwrap();
                let mut read_local = reader.single_user_local_chain_post_close(&baseline.config).unwrap();
                let recovery = read_local.inspect_macro(&baseline.intent).unwrap();
                drop(read_local);
                reader.connection.close().unwrap();
                recovery
            };
            assert!(health_pending.has_unconfirmed_effect());
            assert!(health_pending.attempts().is_empty());
            assert!(health_pending.global_news(GlobalNewsProvider::Eastmoney).is_none());
            assert_eq!(health_pending.readiness_episodes().len(), 1);
            let health_episode = &health_pending.readiness_episodes()[0];
            assert_eq!(health_episode.ready_result_version(), None);
            let health_controls = health_episode.controls();
            assert_eq!(health_controls.len(), 2);
            assert_eq!(health_controls[0].kind(), ExternalControlKind::Health);
            assert!(health_controls[0].begin_version().is_some());
            assert_eq!(health_controls[0].result_version(), None);
            assert_eq!(health_controls[0].outcome(), None);
            assert_eq!(health_controls[0].response_bytes(), None);
            assert_eq!(health_controls[1].kind(), ExternalControlKind::Capabilities);
            assert_eq!(health_controls[1].begin_version(), None);
            assert_eq!(health_controls[1].result_version(), None);
            assert_eq!(health_controls[1].outcome(), None);
            assert_eq!(health_controls[1].response_bytes(), None);
            assert_eq!(audit_snapshot_at(&database), baseline.audit);
            let health_receipt = external.snapshot();
            assert_eq!(health_receipt.tcp_accepts, 1);
            assert_eq!(health_receipt.health_requests, vec![health_controls[0].request_bytes().to_vec()]);
            assert_eq!(health_receipt.health_authorized, vec![true]);
            assert!(health_receipt.health_responses.is_empty());
            assert!(health_receipt.health_statuses.is_empty());
            assert_eq!(health_receipt.capabilities_calls, 0);
            assert!(health_receipt.capabilities_requests.is_empty());
            assert!(health_receipt.capabilities_responses.is_empty());
            assert!(health_receipt.capabilities_statuses.is_empty());
            assert_eq!(health_receipt.data_calls, 0);
            assert!(health_receipt.data_requests.is_empty());
            assert!(health_receipt.data_responses.is_empty());
            assert!(health_receipt.data_statuses.is_empty());
            if !case.is_health() {
                external.release_health();
                let capabilities_deadline = std::time::Instant::now() + Duration::from_secs(5);
                loop {
                    tokio::select! {
                        biased;
                        result = &mut prepared => panic!("TEST_CODE {} returned before Capabilities receipt: {result:?}", case.label()),
                        _ = tokio::task::yield_now() => {}
                    }
                    if external.snapshot().capabilities_requests.len() == 1 {
                        break;
                    }
                    assert!(std::time::Instant::now() < capabilities_deadline, "TEST_CODE {} Capabilities receipt watchdog", case.label());
                }
            }
            None
        };
        let pending = {
            let mut reader = BusinessIntentStore::open(&database).unwrap();
            let mut read_local = reader.single_user_local_chain_post_close(&baseline.config).unwrap();
            let recovery = read_local.inspect_macro(&baseline.intent).unwrap();
            drop(read_local);
            reader.connection.close().unwrap();
            recovery
        };
        if !case.is_connect_unavailable() {
            assert!(pending.has_unconfirmed_effect());
            assert!(pending.attempts().is_empty());
            assert!(pending.global_news(GlobalNewsProvider::Eastmoney).is_none());
            assert_eq!(audit_snapshot_at(&database), baseline.audit);
        }
        let plan = pending.plan();
        assert_eq!(plan.profile(), ContractProfile::ExternalV1);
        assert_eq!(plan.acquisition_authority(), Some(AUTHORITY));
        assert_eq!(plan.endpoint(), external.endpoint());
        assert_eq!(plan.started_at().get(), started_at);
        assert_eq!(plan.deadline_at().get(), started_at + 15_000_000);
        assert_eq!(plan.observed_local(), STARTED_LOCAL);
        assert_eq!(plan.first_source_request().retry_policy(), (4, 1000, 60_000, 200));
        let plan_bytes = pending.plan_bytes().to_vec();
        let data_id = plan.first_source_request().request_id().to_owned();
        let data_bytes = plan.first_source_request().request_bytes().to_vec();
        let data_request = QueryRequest::decode(data_bytes.as_slice()).unwrap();
        assert_eq!(data_request.encode_to_vec(), data_bytes);
        assert_eq!(data_request.context.as_ref().unwrap().protocol_version, 1);
        assert_eq!(data_request.context.as_ref().unwrap().request_id, data_id);
        assert_eq!(data_request.preferred_provider, "Eastmoney");
        assert!(!data_request.allow_unadmitted);
        let payload = data_request.payload.as_ref().unwrap();
        assert_eq!(payload.schema, "magic.market.global_news.request");
        assert_eq!(payload.schema_version, 2);
        assert_eq!(payload.content_type, "application/json; charset=utf-8");
        assert_eq!(payload.data, br#"{"limit":20}"#);
        assert_eq!(pending.readiness_episodes().len(), 1);
        let episode = &pending.readiness_episodes()[0];
        assert_eq!(episode.episode_ordinal(), 1);
        assert_eq!(episode.initiating_source(), &MacroQueryIdentity::GlobalNews {
            provider: GlobalNewsProvider::Eastmoney, limit: 20,
        });
        assert_eq!(episode.ready_result_version(), None);
        let controls = episode.controls();
        assert_eq!(controls.len(), 2);
        assert_eq!(controls[0].kind(), ExternalControlKind::Health);
        assert_eq!(controls[1].kind(), ExternalControlKind::Capabilities);
        let health_id = controls[0].request_id().to_owned();
        let health_bytes = controls[0].request_bytes().to_vec();
        let capabilities_id = controls[1].request_id().to_owned();
        let capabilities_bytes = controls[1].request_bytes().to_vec();
        let health_request = HealthRequest::decode(health_bytes.as_slice()).unwrap();
        assert_eq!(health_request.encode_to_vec(), health_bytes);
        assert_eq!(health_request.context.as_ref().unwrap().protocol_version, 1);
        assert_eq!(health_request.context.as_ref().unwrap().request_id, health_id);
        let capabilities_request = CapabilitiesRequest::decode(capabilities_bytes.as_slice()).unwrap();
        assert_eq!(capabilities_request.encode_to_vec(), capabilities_bytes);
        assert_eq!(capabilities_request.context.as_ref().unwrap().protocol_version, 1);
        assert_eq!(capabilities_request.context.as_ref().unwrap().request_id, capabilities_id);
        assert_ne!(health_id, capabilities_id);
        assert_ne!(data_id, health_id);
        assert_ne!(data_id, capabilities_id);
        assert!(controls[0].begin_version().is_some());
        if case.is_connect_unavailable() {
            assert!(controls[0].result_version().is_some());
            assert_eq!(controls[0].outcome(), Some(MacroControlOutcome::Rejected));
            assert_eq!(controls[0].response_bytes(), None);
            assert_eq!(controls[1].begin_version(), None);
            assert_eq!(controls[1].result_version(), None);
            assert_eq!(controls[1].outcome(), None);
            assert_eq!(controls[1].response_bytes(), None);
        } else if case.is_health() {
            assert_eq!(controls[0].result_version(), None);
            assert_eq!(controls[0].outcome(), None);
            assert_eq!(controls[0].response_bytes(), None);
            assert_eq!(controls[1].begin_version(), None);
            assert_eq!(controls[1].result_version(), None);
            assert_eq!(controls[1].outcome(), None);
            assert_eq!(controls[1].response_bytes(), None);
        } else {
            assert_eq!(controls[0].outcome(), Some(MacroControlOutcome::Ready));
            assert!(controls[0].result_version().is_some());
            assert!(controls[1].begin_version().unwrap() > controls[0].result_version().unwrap());
            let expected_health = HealthResponse {
                request_id: health_id.clone(),
                live: true,
                ready: true,
                state: "TEST_CODE_HEALTH_RUNNING".to_owned(),
                observability: Some(test_external_observability()),
                build_identity: Some(test_external_build_identity()),
            }
            .encode_to_vec();
            assert_eq!(controls[0].response_bytes(), Some(expected_health.as_slice()));
            assert_eq!(controls[1].result_version(), None);
            assert_eq!(controls[1].outcome(), None);
            assert_eq!(controls[1].response_bytes(), None);
        }
        assert_eq!(external.snapshot().tcp_accepts, 1);
        if case.is_connect_unavailable() {
            assert!(external.snapshot().health_requests.is_empty());
            assert!(external.snapshot().health_authorized.is_empty());
        } else {
            assert_eq!(external.snapshot().health_requests, vec![health_bytes.clone()]);
            assert_eq!(external.snapshot().health_authorized, vec![true]);
        }
        assert_eq!(external.snapshot().capabilities_calls, usize::from(!case.is_health()));
        assert_eq!(external.snapshot().data_calls, 0);
        if !case.is_health() {
            let capabilities_receipt = external.snapshot();
            assert_eq!(
                capabilities_receipt.health_responses,
                vec![HealthResponse {
                    request_id: health_id.clone(),
                    live: true,
                    ready: true,
                    state: "TEST_CODE_HEALTH_RUNNING".to_owned(),
                    observability: Some(test_external_observability()),
                    build_identity: Some(test_external_build_identity()),
                }
                .encode_to_vec()]
            );
            assert!(capabilities_receipt.health_statuses.is_empty());
            assert_eq!(capabilities_receipt.capabilities_requests, vec![capabilities_bytes.clone()]);
            assert_eq!(capabilities_receipt.capabilities_authorized, vec![true]);
            assert!(capabilities_receipt.capabilities_responses.is_empty());
            assert!(capabilities_receipt.capabilities_statuses.is_empty());
            assert_eq!(capabilities_receipt.data_calls, 0);
            assert!(capabilities_receipt.data_requests.is_empty());
            assert!(capabilities_receipt.data_responses.is_empty());
            assert!(capabilities_receipt.data_statuses.is_empty());
        }

        let stopped = if let Some(stopped) = early_stop {
            stopped
        } else {
            if case.is_health() { external.release_health(); } else { external.release_capabilities(); }
            match tokio::time::timeout(Duration::from_secs(5), &mut prepared).await {
                Ok(Err(error)) => error,
                Ok(Ok(_)) => panic!("TEST_CODE {} unexpectedly completed Macro", case.label()),
                Err(_) => panic!("TEST_CODE {} response watchdog elapsed", case.label()),
            }
        };
        assert_partial_macro_stop(&stopped);
        drop(prepared);
        drop(io);
        let recovered = local.inspect_macro(&baseline.intent).unwrap();
        assert!(!recovered.is_complete());
        assert!(!recovered.has_unconfirmed_effect());
        assert_eq!(recovered.plan_bytes(), plan_bytes);
        assert_eq!(recovered.parent_final_bytes(), baseline.final_bytes);
        assert_eq!(recovered.readiness_episodes().len(), 1);
        let episode = &recovered.readiness_episodes()[0];
        assert_eq!(episode.ready_result_version(), None);
        let controls = episode.controls();
        let target = if case.is_health() { &controls[0] } else { &controls[1] };
        let target_id = target.request_id().to_owned();
        assert_eq!(target_id.as_str(), if case.is_health() { health_id.as_str() } else { capabilities_id.as_str() });
        assert_eq!(target.request_bytes(), if case.is_health() { health_bytes.as_slice() } else { capabilities_bytes.as_slice() });
        let result_version = target.result_version().expect("TEST_CODE rejection result version");
        assert!(result_version > target.begin_version().unwrap());
        assert_eq!(target.outcome(), Some(MacroControlOutcome::Rejected));
        if case.is_health() {
            assert_eq!(controls[1].begin_version(), None);
            assert_eq!(controls[1].result_version(), None);
            assert_eq!(controls[1].outcome(), None);
            assert_eq!(controls[1].response_bytes(), None);
        } else {
            assert_eq!(controls[0].outcome(), Some(MacroControlOutcome::Ready));
            assert!(controls[0].result_version().unwrap() < controls[1].begin_version().unwrap());
        }
        let expected_response = match case {
            RejectionCase::HealthConnectUnavailable => None,
            RejectionCase::HealthNotReady => Some(HealthResponse {
                request_id: health_id.clone(), live: true, ready: false,
                state: "TEST_CODE_HEALTH_NOT_READY".to_owned(),
                observability: Some(test_external_observability()),
                build_identity: Some(test_external_build_identity()),
            }.encode_to_vec()),
            RejectionCase::HealthMismatchedId => Some(HealthResponse {
                request_id: "TEST_CODE_WRONG_HEALTH_REQUEST_ID".to_owned(), live: true, ready: true,
                state: "TEST_CODE_HEALTH_RUNNING".to_owned(),
                observability: Some(test_external_observability()),
                build_identity: Some(test_external_build_identity()),
            }.encode_to_vec()),
            RejectionCase::CapabilitiesMismatchedId
            | RejectionCase::CapabilitiesMissingGlobalNews
            | RejectionCase::CapabilitiesUnadmitted
            | RejectionCase::CapabilitiesRuntimeUnavailable => {
                Some(expected_capabilities(capabilities_id.clone(), case).encode_to_vec())
            }
            RejectionCase::HealthStatus(_) | RejectionCase::CapabilitiesStatus(_) => None,
        };
        assert_eq!(target.response_bytes(), expected_response.as_deref());
        let raw = raw_control_bytes(&database, &baseline.intent, if case.is_health() { 1 } else { 2 });
        assert_raw(case, &raw, expected_response.as_deref(), &target_id);
        let receipt = if case.is_connect_unavailable() {
            assert_connect_unavailable_source(&recovered)
        } else {
            let source = recovered.global_news(GlobalNewsProvider::Eastmoney).unwrap();
            assert!(source.is_complete());
            assert_eq!(source.profile(), "ExternalV1");
            assert_eq!(source.acquisition_authority(), Some(AUTHORITY));
            assert_eq!(source.retry_policy(), (4, 1000, 60_000, 200));
            assert!(source.batch().is_none());
            let error = source.error().unwrap();
            assert_eq!(error.capability(), "GrpcExternalV1");
            assert_eq!(error.provider(), case.provider());
            assert_eq!(error.audit_outcome(), case.outcome());
            assert_eq!(error.reason_code(), case.reason());
            assert_eq!(error.retryable(), case.retryable());
            assert_eq!(error.message(), case.message());
            if matches!(case, RejectionCase::HealthMismatchedId) {
                assert!(!format!("{error:?}").contains("TEST_CODE_WRONG_HEALTH_REQUEST_ID"));
            }
            if matches!(case, RejectionCase::CapabilitiesMismatchedId) {
                assert!(!format!("{error:?}").contains("TEST_CODE_WRONG_CAPABILITIES_REQUEST_ID"));
            }
            assert_eq!(source.final_bytes().unwrap(), case.native());
            let receipt = source.audit_receipt().unwrap().clone();
            assert_eq!(receipt.previous_outcome, None);
            assert_eq!(receipt.current_outcome, case.outcome());
            receipt
        };
        assert_eq!(recovered.pending_source_identities(), pending_sources().as_slice());
        assert_eq!(recovered.pending_research_queries(), pending_research().as_slice());
        assert_eq!(clock.observation_calls.get(), 1);
        let terminal_head = local.inspect_run(&baseline.intent).unwrap().head_version();
        assert_eq!(result_version + 1, terminal_head);
        assert_eq!(local.inspect_run(&baseline.intent).unwrap().lease_generation(), 3);
        assert_eq!(local.inspect_run(&baseline.intent).unwrap().context().canonical_bytes(), baseline.context);
        drop(local);
        assert_eq!(business.count("chain_post_close_macro_source_finals"), 1);
        let terminal_audit = audit_snapshot(business.connection());
        assert_eq!(terminal_audit.len(), baseline.audit.len() + 1);
        assert_eq!(&terminal_audit[..baseline.audit.len()], baseline.audit.as_slice());
        let first_wire = external.snapshot();
        assert_eq!(first_wire.tcp_accepts, 1);
        assert_eq!(
            first_wire.health_requests.len(),
            usize::from(!case.is_connect_unavailable())
        );
        assert_eq!(first_wire.capabilities_calls, usize::from(!case.is_health()));
        assert_eq!(first_wire.data_calls, 0);
        assert!(first_wire.data_requests.is_empty());
        assert!(first_wire.data_responses.is_empty());
        assert!(first_wire.data_statuses.is_empty());
        if case.is_health() {
            assert!(first_wire.capabilities_requests.is_empty());
            assert!(first_wire.capabilities_responses.is_empty());
            assert!(first_wire.capabilities_statuses.is_empty());
        } else {
            assert_eq!(first_wire.capabilities_requests, vec![capabilities_bytes.clone()]);
            assert_eq!(first_wire.capabilities_authorized, vec![true]);
            assert_eq!(
                first_wire.health_responses,
                vec![HealthResponse {
                    request_id: health_id.clone(),
                    live: true,
                    ready: true,
                    state: "TEST_CODE_HEALTH_RUNNING".to_owned(),
                    observability: Some(test_external_observability()),
                    build_identity: Some(test_external_build_identity()),
                }
                .encode_to_vec()]
            );
            assert!(first_wire.health_statuses.is_empty());
        }
        match case {
            RejectionCase::HealthConnectUnavailable => {
                assert!(first_wire.health_authorized.is_empty());
                assert!(first_wire.health_responses.is_empty());
                assert!(first_wire.health_statuses.is_empty());
            }
            RejectionCase::HealthStatus(status_case) => {
                let (_, expected) = expected_status(status_case, &health_id);
                assert_eq!(first_wire.health_statuses, vec![expected]);
                assert!(first_wire.health_responses.is_empty());
            }
            RejectionCase::CapabilitiesStatus(status_case) => {
                let (_, expected) = expected_status(status_case, &capabilities_id);
                assert_eq!(first_wire.capabilities_statuses, vec![expected]);
                assert!(first_wire.capabilities_responses.is_empty());
            }
            _ if case.is_health() => {
                assert_eq!(first_wire.health_responses, vec![expected_response.clone().unwrap()]);
                assert!(first_wire.health_statuses.is_empty());
            }
            _ => {
                assert_eq!(first_wire.capabilities_responses, vec![expected_response.clone().unwrap()]);
                assert!(first_wire.capabilities_statuses.is_empty());
            }
        }
        assert_eq!(parent_server.as_ref().unwrap().snapshot(), baseline.network);
        assert_eq!(parent_server.as_ref().unwrap().membership_snapshot(), baseline.memberships);
        assert_eq!(old_fact_rows(business.connection(), &baseline.tables), baseline.facts);
        if case.is_connect_unavailable() {
            assert_connect_unavailable_audit(
                business.connection(), &receipt, &baseline.receipt,
                "2026-09-14T07:31:00+00:00",
            );
        } else {
            let transaction = business.connection().unchecked_transaction().unwrap();
            let verified = read_acquisition_in_transaction(&transaction, &receipt).unwrap();
            assert_eq!(verified.receipt(), &receipt);
            let audit = verified.record();
            assert_eq!(audit.capability, "GlobalNews-Eastmoney");
            assert_eq!(audit.provider, "Eastmoney");
            assert_eq!(audit.source, "review-data-gateway");
            assert_eq!(audit.request_hash, REQUEST_HASH);
            assert_eq!(audit.source_at, None);
            assert_eq!(audit.observed_at, OBSERVED_UTC);
            assert_eq!(audit.batch_id, None);
            assert_eq!(audit.outcome, case.outcome());
            assert_eq!((audit.request_count, audit.accepted_count, audit.rejected_count), (1, 0, 1));
            assert_eq!(audit.reason_code, case.reason());
            assert_eq!(audit.retryable, case.retryable());
            read_acquisition_in_transaction(&transaction, &baseline.receipt).unwrap();
            transaction.commit().unwrap();
        }

        if case.is_connect_unavailable() {
            external.set_reject_new_connections_for_test(false);
            external.release_health();
            external.release_capabilities();
            external.release_data();
        }
        drop(baseline.queries);
        drop(baseline.source);
        drop(macro_source);
        business.reopen();
        let reopened_at = started_at + 3_000_000;
        assert!(reopened_at > started_at + 2_000_000);
        assert!(reopened_at < started_at + 15_000_000);
        let reopened_macro_source = GrpcSource::from_external_macro_bundle_for_test(external.bundle_path().to_path_buf());
        let reopened_parent_source = GrpcSource::from_board_loopback_test_client(connect_parent_instance(&baseline.endpoint).await);
        let reopened_queries = reopened_parent_source.connected_board_queries().await.unwrap();
        let reopened_clock = MacroClock {
            now: Cell::new(UtcMicros::try_new(reopened_at).unwrap()),
            observation: DateTime::parse_from_rfc3339("2026-09-14T15:32:00+08:00").unwrap(),
            observation_calls: Cell::new(0),
        };
        let mut local = business.store.as_mut().unwrap().single_user_local_chain_post_close(&baseline.config).unwrap();
        let lease = local.resume_run(&baseline.intent, macro_lease(
            case.owner_b(), reopened_at, started_at + 10_000_000, terminal_head,
        )).unwrap();
        let owner_b_baseline = local.inspect_run(&baseline.intent).unwrap().head_version();
        let mut io = local.macro_preparation_io_v11(
            lease, &reopened_queries, &reopened_clock, FixedClusterConfiguration::resolve(Some("2")),
            &reopened_parent_source, &reopened_macro_source, &search_service,
        ).unwrap();
        let reopened_stop = match tokio::time::timeout(Duration::from_secs(5), prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(), baseline.stocks, None, &mut io,
        )).await {
            Ok(Err(error)) => error,
            Ok(Ok(_)) => panic!("TEST_CODE {} reopen unexpectedly completed Macro", case.label()),
            Err(_) => panic!("TEST_CODE {} reopen watchdog elapsed", case.label()),
        };
        assert_partial_macro_stop(&reopened_stop);
        drop(io);
        let reopened = local.inspect_macro(&baseline.intent).unwrap();
        assert_same_terminal_recovery(&recovered, &reopened);
        let reopened_episode = &reopened.readiness_episodes()[0];
        let reopened_controls = reopened_episode.controls();
        let reopened_target = if case.is_health() {
            &reopened_controls[0]
        } else {
            &reopened_controls[1]
        };
        assert_eq!(reopened_target.request_id(), target_id.as_str());
        assert_eq!(reopened_target.request_bytes(), target.request_bytes());
        assert_eq!(reopened_target.result_version(), Some(result_version));
        assert_eq!(reopened_target.outcome(), Some(MacroControlOutcome::Rejected));
        assert_eq!(reopened_target.response_bytes(), expected_response.as_deref());
        if case.is_health() {
            assert_eq!(reopened_controls[1].begin_version(), None);
            assert_eq!(reopened_controls[1].result_version(), None);
            assert_eq!(reopened_controls[1].outcome(), None);
            assert_eq!(reopened_controls[1].response_bytes(), None);
        } else {
            assert_eq!(reopened_controls[0].outcome(), Some(MacroControlOutcome::Ready));
        }
        assert_eq!(reopened_clock.observation_calls.get(), 0);
        assert_eq!(local.inspect_run(&baseline.intent).unwrap().head_version(), owner_b_baseline);
        assert_eq!(local.inspect_run(&baseline.intent).unwrap().context().canonical_bytes(), baseline.context);
        drop(local);
        assert_eq!(business.count("chain_post_close_macro_source_finals"), 1);
        assert_eq!(audit_snapshot(business.connection()), terminal_audit);
        assert_eq!(raw_control_bytes(&database, &baseline.intent, if case.is_health() { 1 } else { 2 }), raw);
        assert_eq!(external.snapshot(), first_wire);
        assert_eq!(parent_server.as_ref().unwrap().snapshot(), baseline.network);
        assert_eq!(parent_server.as_ref().unwrap().membership_snapshot(), baseline.memberships);
        assert_eq!(old_fact_rows(business.connection(), &baseline.tables), baseline.facts);
        let transaction = business.connection().unchecked_transaction().unwrap();
        read_acquisition_in_transaction(&transaction, &receipt).unwrap();
        read_acquisition_in_transaction(&transaction, &baseline.receipt).unwrap();
        transaction.commit().unwrap();
        drop(reopened_queries);
        drop(reopened_parent_source);
        drop(reopened_macro_source);
    })).catch_unwind().await;

    cleanup_external_case(
        &mut business,
        &mut parent_server,
        &mut external_server,
        case.label(),
    )
    .await;
    drop(business);
    match body {
        Ok(result) => result.unwrap_or_else(|_| panic!("TEST_CODE {} body deadline", case.label())),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test]
async fn single_user_external_macro_control_health_status_absent_reopens_without_replay() {
    run_control_rejection(RejectionCase::HealthStatus(HealthStatusCase::Absent)).await;
}

#[tokio::test]
async fn single_user_external_macro_control_health_status_bytes_reopens_without_replay() {
    run_control_rejection(RejectionCase::HealthStatus(HealthStatusCase::Bytes)).await;
}

#[tokio::test]
async fn single_user_external_macro_control_health_status_malformed_reopens_without_replay() {
    run_control_rejection(RejectionCase::HealthStatus(HealthStatusCase::Malformed)).await;
}

#[tokio::test]
async fn single_user_external_macro_control_health_mismatched_id_reopens_without_replay() {
    run_control_rejection(RejectionCase::HealthMismatchedId).await;
}

#[tokio::test]
async fn single_user_external_macro_control_capabilities_status_absent_reopens_without_replay() {
    run_control_rejection(RejectionCase::CapabilitiesStatus(HealthStatusCase::Absent)).await;
}

#[tokio::test]
async fn single_user_external_macro_control_capabilities_status_bytes_reopens_without_replay() {
    run_control_rejection(RejectionCase::CapabilitiesStatus(HealthStatusCase::Bytes)).await;
}

#[tokio::test]
async fn single_user_external_macro_control_capabilities_status_malformed_reopens_without_replay() {
    run_control_rejection(RejectionCase::CapabilitiesStatus(
        HealthStatusCase::Malformed,
    ))
    .await;
}

#[tokio::test]
async fn single_user_external_macro_control_capabilities_mismatched_id_reopens_without_replay() {
    run_control_rejection(RejectionCase::CapabilitiesMismatchedId).await;
}

#[tokio::test]
async fn single_user_external_macro_control_capabilities_missing_global_news_reopens_without_replay(
) {
    run_control_rejection(RejectionCase::CapabilitiesMissingGlobalNews).await;
}

#[tokio::test]
async fn single_user_external_macro_control_capabilities_unadmitted_reopens_without_replay() {
    run_control_rejection(RejectionCase::CapabilitiesUnadmitted).await;
}

#[tokio::test]
async fn single_user_external_macro_control_capabilities_runtime_unavailable_reopens_without_replay(
) {
    run_control_rejection(RejectionCase::CapabilitiesRuntimeUnavailable).await;
}
