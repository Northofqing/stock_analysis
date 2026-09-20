use super::*;
use crate::data_gateway::grpc_source::macro_queries::PreparedMacroQueries;
use crate::grpc_client::client::external_control_loopback_fixture::{
    ExternalControlObservation, ExternalMtlsMacroFixture,
};
use crate::grpc_client::client::macro_loopback_fixture::{
    MacroLoopbackServer, MacroObservation,
};
use crate::grpc_client::client::ContractProfile;
use crate::grpc_client::errors::{ErrorDetail as GrpcErrorDetail, GrpcError};
use crate::pipeline::chain_analysis::preparation::ChainPreparationIo;
use crate::push_foundation::intent_store::chain_post_close::macro_codec;
use crate::push_foundation::intent_store::chain_post_close::macro_stage::MacroRecovery;

const STARTED_LOCAL: &str = "2026-09-14T15:31:00+08:00";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RouteCase {
    LocalToExternal,
    ExternalToLocal,
    ExternalEndpoint,
    ExternalAuthority,
}

impl RouteCase {
    fn label(self) -> &'static str {
        match self {
            Self::LocalToExternal => "Local plan to External source",
            Self::ExternalToLocal => "External plan to Local source",
            Self::ExternalEndpoint => "External endpoint drift",
            Self::ExternalAuthority => "External authority drift",
        }
    }

    fn run_id(self) -> &'static str {
        match self {
            Self::LocalToExternal => "TEST_CODE_RUN_ROUTE_LOCAL_TO_EXTERNAL",
            Self::ExternalToLocal => "TEST_CODE_RUN_ROUTE_EXTERNAL_TO_LOCAL",
            Self::ExternalEndpoint => "TEST_CODE_RUN_ROUTE_EXTERNAL_ENDPOINT",
            Self::ExternalAuthority => "TEST_CODE_RUN_ROUTE_EXTERNAL_AUTHORITY",
        }
    }

    fn owner(self, phase: &str) -> String {
        format!(
            "TEST_CODE_ROUTE_{}_{}",
            match self {
                Self::LocalToExternal => "LOCAL_TO_EXTERNAL",
                Self::ExternalToLocal => "EXTERNAL_TO_LOCAL",
                Self::ExternalEndpoint => "EXTERNAL_ENDPOINT",
                Self::ExternalAuthority => "EXTERNAL_AUTHORITY",
            },
            phase
        )
    }

    fn original_profile(self) -> ContractProfile {
        match self {
            Self::LocalToExternal => ContractProfile::LocalBridgeV1,
            _ => ContractProfile::ExternalV1,
        }
    }
}

#[derive(Clone)]
struct RouteEvidence {
    plan_bytes: Vec<u8>,
    endpoint: String,
    profile: ContractProfile,
    authority: Option<String>,
    request_id: String,
    request_bytes: Vec<u8>,
    pending_sources: Vec<MacroQueryIdentity>,
    pending_research: Vec<String>,
    external: Option<control_unknown_commit_tests::OriginalEvidence>,
}

fn capture_route(
    recovery: &MacroRecovery,
    profile: ContractProfile,
    external: Option<control_unknown_commit_tests::OriginalEvidence>,
) -> RouteEvidence {
    if let Some(original) = external.as_ref() {
        control_unknown_commit_tests::assert_original(recovery, original);
    } else {
        assert_eq!(profile, ContractProfile::LocalBridgeV1);
        assert!(recovery.readiness_episodes().is_empty());
    }
    assert!(!recovery.is_complete());
    assert!(!recovery.has_unconfirmed_effect());
    assert!(recovery.attempts().is_empty());
    assert!(recovery
        .global_news(GlobalNewsProvider::Eastmoney)
        .is_none());
    let plan = recovery.plan();
    assert_eq!(plan.profile(), profile);
    let expected_authority = match profile {
        ContractProfile::LocalBridgeV1 => None,
        ContractProfile::ExternalV1 => Some("grpc-mtls:macro.test.invalid"),
    };
    assert_eq!(plan.acquisition_authority(), expected_authority);
    assert_eq!(plan.started_at().get(), micros(STARTED_LOCAL));
    assert_eq!(plan.deadline_at().get(), micros(STARTED_LOCAL) + 15_000_000);
    assert_eq!(plan.observed_local(), STARTED_LOCAL);
    assert_eq!(
        plan.first_source_request().retry_policy(),
        (4, 1000, 60_000, 200)
    );
    RouteEvidence {
        plan_bytes: recovery.plan_bytes().to_vec(),
        endpoint: plan.endpoint().to_owned(),
        profile,
        authority: plan.acquisition_authority().map(str::to_owned),
        request_id: plan.first_source_request().request_id().to_owned(),
        request_bytes: plan.first_source_request().request_bytes().to_vec(),
        pending_sources: recovery.pending_source_identities().to_vec(),
        pending_research: recovery.pending_research_queries().to_vec(),
        external,
    }
}

fn assert_route(recovery: &MacroRecovery, expected: &RouteEvidence) {
    assert_eq!(recovery.plan_bytes(), expected.plan_bytes.as_slice());
    let plan = recovery.plan();
    assert_eq!(plan.endpoint(), expected.endpoint);
    assert_eq!(plan.profile(), expected.profile);
    assert_eq!(plan.acquisition_authority(), expected.authority.as_deref());
    assert_eq!(plan.started_at().get(), micros(STARTED_LOCAL));
    assert_eq!(plan.deadline_at().get(), micros(STARTED_LOCAL) + 15_000_000);
    assert_eq!(plan.observed_local(), STARTED_LOCAL);
    assert_eq!(
        plan.first_source_request().request_id(),
        expected.request_id
    );
    assert_eq!(
        plan.first_source_request().request_bytes(),
        expected.request_bytes
    );
    assert_eq!(
        plan.first_source_request().retry_policy(),
        (4, 1000, 60_000, 200)
    );
    assert_eq!(
        recovery.pending_source_identities(),
        expected.pending_sources
    );
    assert_eq!(
        recovery.pending_research_queries(),
        expected.pending_research
    );
    assert!(!recovery.is_complete());
    assert!(recovery
        .global_news(GlobalNewsProvider::Eastmoney)
        .is_none());
    if let Some(external) = expected.external.as_ref() {
        control_unknown_commit_tests::assert_original(recovery, external);
    } else {
        assert!(recovery.readiness_episodes().is_empty());
    }
}

fn plan_local_without_begin(
    business: &mut V2BusinessFixture,
    baseline: &control_tests::ExternalParentBaseline,
    source: &GrpcSource,
    owner: &str,
) -> (RouteEvidence, u64) {
    let PreparedMacroQueries::Local(connected) = source.prepare_macro_queries().unwrap() else {
        panic!("TEST_CODE expected Local prepared route");
    };
    let endpoint = connected.endpoint().to_owned();
    let attempt = connected
        .session(MacroQueryIdentity::GlobalNews {
            provider: GlobalNewsProvider::Eastmoney,
            limit: 20,
        })
        .unwrap()
        .authorize_next()
        .unwrap();
    let request = macro_codec::Request::capture(&attempt).unwrap();
    let started = micros(STARTED_LOCAL);
    let clock = MacroClock {
        now: Cell::new(UtcMicros::try_new(started).unwrap()),
        observation: DateTime::parse_from_rfc3339(STARTED_LOCAL).unwrap(),
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
            macro_lease(owner, started, started + 1_000_000, baseline.head),
        )
        .unwrap();
    let registered = control_unknown_commit_tests::registered();
    let search_service = macro_search_service(&registered);
    let web = search_service.macro_web_snapshot(source).unwrap();
    let lease = local
        .plan_macro_request(
            lease,
            clock.now(),
            clock.macro_request_observation(),
            &endpoint,
            request,
            None,
            &web,
            clock.now(),
        )
        .unwrap();
    let recovery = local.inspect_macro(&baseline.intent).unwrap();
    let evidence = capture_route(&recovery, ContractProfile::LocalBridgeV1, None);
    assert_eq!(clock.observation_calls.get(), 1);
    assert_eq!(evidence.endpoint, endpoint);
    let head = lease.head_version();
    drop(local);
    drop(attempt);
    drop(connected);
    (evidence, head)
}

fn assert_route_error(case: RouteCase, error: &anyhow::Error, intent: &IntentId) {
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::AuthorityRejected { intent_id })
            if intent_id == intent.as_str()
    ));
    if case == RouteCase::ExternalAuthority {
        let native = error
            .downcast_ref::<GrpcError>()
            .expect("TEST_CODE authority drift native error");
        assert!(matches!(native, GrpcError::FailedPrecondition { .. }));
        assert_eq!(
            native.details(),
            &GrpcErrorDetail {
                code: "external_control_request_mismatch".to_owned(),
                reason_code: Some("external_control_request_mismatch".to_owned()),
                retryable: Some(false),
                ..GrpcErrorDetail::default()
            }
        );
        assert!(error.downcast_ref::<ChainPostCloseError>().is_none());
    } else {
        assert_eq!(
            error.downcast_ref::<ChainPostCloseError>(),
            Some(&ChainPostCloseError::SchemaRejected)
        );
        assert!(error.downcast_ref::<GrpcError>().is_none());
    }
}

async fn wait_for_local_tcp(server: &MacroLoopbackServer) -> (usize, MacroObservation) {
    let watchdog = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let snapshot = server.snapshot_with_tcp_for_test();
        if snapshot.0 == 1 {
            assert_eq!(snapshot.1, MacroObservation::default());
            return snapshot;
        }
        assert_eq!(snapshot.0, 0);
        assert!(
            std::time::Instant::now() < watchdog,
            "TEST_CODE Local TCP observation watchdog"
        );
        tokio::task::yield_now().await;
    }
}

async fn start_original(
    business: &mut V2BusinessFixture,
    baseline: &control_tests::ExternalParentBaseline,
    local_server: &MacroLoopbackServer,
    external_a: &ExternalMtlsMacroFixture,
    local_source: &GrpcSource,
    external_source: &GrpcSource,
    expected: &RouteEvidence,
    expected_head: u64,
    case: RouteCase,
) {
    business.reopen();
    let now = micros(STARTED_LOCAL) + 4_000_000;
    let clock = MacroClock {
        now: Cell::new(UtcMicros::try_new(now).unwrap()),
        observation: DateTime::parse_from_rfc3339(STARTED_LOCAL).unwrap(),
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
            macro_lease(&case.owner("POSITIVE"), now, now + 4_000_000, expected_head),
        )
        .unwrap();
    let source = match expected.profile {
        ContractProfile::LocalBridgeV1 => local_source,
        ContractProfile::ExternalV1 => external_source,
    };
    let registered = control_unknown_commit_tests::registered();
    let search_service = macro_search_service(&registered);
    let mut io = local
        .macro_preparation_io_v11(
            lease,
            &baseline.queries,
            &clock,
            FixedClusterConfiguration::resolve(Some("2")),
            &baseline.source,
            source,
            &search_service,
        )
        .unwrap();
    let mut pending = Box::pin(io.macro_search_with_budget());
    let watchdog = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        match futures::poll!(&mut pending) {
            std::task::Poll::Pending => {}
            std::task::Poll::Ready(result) => panic!(
                "TEST_CODE {} positive route completed before gate: {result:?}",
                case.label()
            ),
        }
        let observed = match expected.profile {
            ContractProfile::LocalBridgeV1 => local_server.snapshot().requests.len() == 1,
            ContractProfile::ExternalV1 => external_a.snapshot().health_requests.len() == 1,
        };
        if observed {
            break;
        }
        assert!(
            std::time::Instant::now() < watchdog,
            "TEST_CODE {} positive receipt watchdog",
            case.label()
        );
        tokio::task::yield_now().await;
    }
    drop(pending);
    drop(io);
    drop(local);

    let (recovery, run) = control_unknown_commit_tests::inspect_at(
        &business.database(),
        &baseline.config,
        &baseline.intent,
    );
    assert_route(&recovery, expected);
    assert!(recovery.has_unconfirmed_effect());
    match expected.profile {
        ContractProfile::LocalBridgeV1 => {
            assert!(recovery.readiness_episodes().is_empty());
            assert_eq!(recovery.attempts().len(), 1);
            assert_eq!(recovery.attempts()[0].request_bytes(), expected.request_bytes);
            assert_eq!(recovery.attempts()[0].result_version(), None);
            let (tcp, wire) = local_server.snapshot_with_tcp_for_test();
            assert_eq!(tcp, 1);
            assert_eq!(wire.requests, vec![expected.request_bytes.clone()]);
            assert_eq!(wire.authorized, vec![true]);
            assert!(wire.responses.is_empty());
            assert!(wire.statuses.is_empty());
            assert_eq!((wire.health_calls, wire.capabilities_calls), (0, 0));
            assert!(wire.unexpected_data_calls.is_empty());
            assert_eq!(external_a.snapshot(), ExternalControlObservation::default());
        }
        ContractProfile::ExternalV1 => {
            assert!(recovery.attempts().is_empty());
            let controls = recovery.readiness_episodes()[0].controls();
            assert!(controls[0].begin_version().is_some());
            assert_eq!(controls[0].result_version(), None);
            assert_eq!(controls[0].response_bytes(), None);
            assert_eq!(controls[1].begin_version(), None);
            let wire = external_a.snapshot();
            assert_eq!(wire.tcp_accepts, 1);
            assert_eq!(
                wire.health_requests,
                vec![expected.external.as_ref().unwrap().health.bytes.clone()]
            );
            assert_eq!(wire.health_authorized, vec![true]);
            assert!(wire.health_responses.is_empty());
            assert!(wire.health_statuses.is_empty());
            assert_eq!(wire.capabilities_calls, 0);
            assert_eq!(wire.data_calls, 0);
            assert_eq!(local_server.snapshot().requests.len(), 0);
        }
    }
    assert!(run.head > expected_head);
    assert_eq!(run.owner, case.owner("POSITIVE"));
    assert_eq!(run.context, baseline.context);
    assert_eq!(
        control_tests::audit_snapshot_at(&business.database()),
        baseline.audit
    );
    let fixed =
        control_unknown_commit_tests::fixed_snapshot(&business.database(), &baseline.intent);
    assert_eq!(fixed.health_raw, None);
    assert_eq!(fixed.capabilities_raw, None);
    assert_eq!(fixed.control_results, 0);
    assert_eq!(fixed.data_begins, i64::from(expected.profile == ContractProfile::LocalBridgeV1));
    assert_eq!(fixed.source_finals, 0);
}

async fn run_case(case: RouteCase) {
    let mut business = V2BusinessFixture::new();
    let mut parent_server = None;
    let mut local_server = None;
    let mut external_a = None;
    let mut external_b = None;
    let body = std::panic::AssertUnwindSafe(tokio::time::timeout(
        Duration::from_secs(120),
        async {
            let baseline = control_tests::setup_external_parent(
                &mut business,
                &mut parent_server,
                case.run_id(),
            )
            .await;
            local_server = Some(MacroLoopbackServer::bind().await);
            external_a = Some(
                ExternalMtlsMacroFixture::bind_data_success_for_test()
                    .await
                    .unwrap(),
            );
            external_b = Some(
                ExternalMtlsMacroFixture::bind_data_success_for_test()
                    .await
                    .unwrap(),
            );
            let local_server_ref = local_server.as_ref().unwrap();
            let external_a_ref = external_a.as_ref().unwrap();
            let external_b_ref = external_b.as_ref().unwrap();
            let local_source = GrpcSource::from_macro_loopback_test_client(
                local_server_ref.connect().await,
                local_server_ref.endpoint().to_owned(),
            );
            let local_connected = wait_for_local_tcp(local_server_ref).await;
            assert_eq!(local_connected.0, 1);
            let external_source_a = GrpcSource::from_external_macro_bundle_for_test(
                external_a_ref.bundle_path().to_path_buf(),
            );
            let external_source_b = GrpcSource::from_external_macro_bundle_for_test(
                external_b_ref.bundle_path().to_path_buf(),
            );
            let external_source_wrong_authority =
                GrpcSource::from_external_macro_bundle_for_test(
                    external_a_ref.wrong_name_bundle_path().to_path_buf(),
                );
            let (expected, plan_head) = if case == RouteCase::LocalToExternal {
                plan_local_without_begin(
                    &mut business,
                    &baseline,
                    &local_source,
                    &case.owner("PLAN"),
                )
            } else {
                let (original, head) =
                    control_unknown_commit_tests::plan_health_without_begin_for_owner(
                        &mut business,
                        &baseline,
                        external_a_ref,
                        &case.owner("PLAN"),
                    );
                let (recovery, _) = control_unknown_commit_tests::inspect_at(
                    &business.database(),
                    &baseline.config,
                    &baseline.intent,
                );
                (
                    capture_route(&recovery, ContractProfile::ExternalV1, Some(original)),
                    head,
                )
            };
            assert_eq!(expected.profile, case.original_profile());
            assert_eq!(local_server_ref.snapshot_with_tcp_for_test(), local_connected);
            assert_eq!(external_a_ref.snapshot(), ExternalControlObservation::default());
            assert_eq!(external_b_ref.snapshot(), ExternalControlObservation::default());
            business.reopen();

            let now = micros(STARTED_LOCAL) + 2_000_000;
            let clock = MacroClock {
                now: Cell::new(UtcMicros::try_new(now).unwrap()),
                observation: DateTime::parse_from_rfc3339(STARTED_LOCAL).unwrap(),
                observation_calls: Cell::new(0),
            };
            let database = business.database();
            let mut local = business
                .store
                .as_mut()
                .unwrap()
                .single_user_local_chain_post_close(&baseline.config)
                .unwrap();
            let lease = local
                .resume_run(
                    &baseline.intent,
                    macro_lease(&case.owner("FAULT"), now, now + 1_000_000, plan_head),
                )
                .unwrap();
            let fault_source = match case {
                RouteCase::LocalToExternal => &external_source_a,
                RouteCase::ExternalToLocal => &local_source,
                RouteCase::ExternalEndpoint => &external_source_b,
                RouteCase::ExternalAuthority => &external_source_wrong_authority,
            };
            let registered = control_unknown_commit_tests::registered();
            let search_service = macro_search_service(&registered);
            let mut io = local
                .macro_preparation_io_v11(
                    lease,
                    &baseline.queries,
                    &clock,
                    FixedClusterConfiguration::resolve(Some("2")),
                    &baseline.source,
                    fault_source,
                    &search_service,
                )
                .unwrap();
            let (_, before_run) = control_unknown_commit_tests::inspect_at(
                &database,
                &baseline.config,
                &baseline.intent,
            );
            let before_fixed =
                control_unknown_commit_tests::fixed_snapshot(&database, &baseline.intent);
            let before_audit = control_tests::audit_snapshot_at(&database);
            let before_local = local_server_ref.snapshot_with_tcp_for_test();
            let before_a = external_a_ref.snapshot();
            let before_b = external_b_ref.snapshot();
            let stopped = io
                .macro_search_with_budget()
                .await
                .expect("TEST_CODE route refusal has no elapsed timeout")
                .expect_err("TEST_CODE changed route must reject before effect");
            assert_route_error(case, &stopped, &baseline.intent);
            drop(io);
            drop(local);
            tokio::task::yield_now().await;

            let (recovered, after_run) = control_unknown_commit_tests::inspect_at(
                &database,
                &baseline.config,
                &baseline.intent,
            );
            assert_route(&recovered, &expected);
            assert!(!recovered.has_unconfirmed_effect());
            assert!(recovered.attempts().is_empty());
            assert_eq!(after_run, before_run);
            assert_eq!(
                control_unknown_commit_tests::fixed_snapshot(&database, &baseline.intent),
                before_fixed
            );
            assert_eq!(control_tests::audit_snapshot_at(&database), before_audit);
            assert_eq!(before_audit, baseline.audit);
            assert_eq!(local_server_ref.snapshot_with_tcp_for_test(), before_local);
            assert_eq!(external_a_ref.snapshot(), before_a);
            assert_eq!(external_b_ref.snapshot(), before_b);
            assert_eq!(parent_server.as_ref().unwrap().snapshot(), baseline.network);
            assert_eq!(
                parent_server.as_ref().unwrap().membership_snapshot(),
                baseline.memberships
            );

            start_original(
                &mut business,
                &baseline,
                local_server_ref,
                external_a_ref,
                &local_source,
                &external_source_a,
                &expected,
                after_run.head,
                case,
            )
            .await;
            match expected.profile {
                ContractProfile::LocalBridgeV1 => {
                    assert_eq!(external_b_ref.snapshot(), ExternalControlObservation::default());
                }
                ContractProfile::ExternalV1 => {
                    assert_eq!(external_b_ref.snapshot(), ExternalControlObservation::default());
                    assert_eq!(local_server_ref.snapshot_with_tcp_for_test().0, 1);
                }
            }
            control_unknown_commit_tests::assert_parent_unchanged(
                &mut business,
                parent_server.as_ref().unwrap(),
                &baseline,
            );
            drop(external_source_wrong_authority);
            drop(external_source_b);
            drop(external_source_a);
            drop(local_source);
            local_server_ref.release_response();
            control_unknown_commit_tests::release_all(external_a_ref);
            control_unknown_commit_tests::release_all(external_b_ref);
        },
    ))
    .catch_unwind()
    .await;

    let local_cleanup = match local_server.take() {
        Some(server) => std::panic::AssertUnwindSafe(server.finish()).catch_unwind().await,
        None => Ok(Ok(())),
    };
    let external_b_cleanup = match external_b.take() {
        Some(server) => std::panic::AssertUnwindSafe(server.finish()).catch_unwind().await,
        None => Ok(Ok(())),
    };
    let common_cleanup = std::panic::AssertUnwindSafe(control_tests::cleanup_external_case(
        &mut business,
        &mut parent_server,
        &mut external_a,
        case.label(),
    ))
    .catch_unwind()
    .await;
    local_cleanup
        .unwrap_or_else(|_| panic!("TEST_CODE {} Local cleanup panic", case.label()))
        .unwrap_or_else(|error| panic!("TEST_CODE {} Local cleanup: {error}", case.label()));
    external_b_cleanup
        .unwrap_or_else(|_| panic!("TEST_CODE {} External B cleanup panic", case.label()))
        .unwrap_or_else(|error| panic!("TEST_CODE {} External B cleanup: {error}", case.label()));
    common_cleanup.unwrap_or_else(|_| panic!("TEST_CODE {} common cleanup panic", case.label()));
    match body {
        Ok(result) => result.unwrap_or_else(|_| panic!("TEST_CODE {} body timeout", case.label())),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test]
async fn single_user_local_macro_plan_rejects_external_route_before_rpc_then_original_local_starts() {
    run_case(RouteCase::LocalToExternal).await;
}

#[tokio::test]
async fn single_user_external_macro_plan_rejects_local_route_before_rpc_then_original_health_starts() {
    run_case(RouteCase::ExternalToLocal).await;
}

#[tokio::test]
async fn single_user_external_macro_plan_rejects_changed_endpoint_before_rpc_then_original_health_starts() {
    run_case(RouteCase::ExternalEndpoint).await;
}

#[tokio::test]
async fn single_user_external_macro_plan_rejects_changed_authority_before_rpc_then_original_health_starts() {
    run_case(RouteCase::ExternalAuthority).await;
}
