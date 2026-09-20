use super::*;
use crate::grpc_client::client::external_control_loopback_fixture::{
    ExternalControlObservation, ExternalMtlsMacroFixture,
};
use crate::pipeline::chain_analysis::preparation::ChainPreparationIo;
use crate::push_foundation::intent_store::chain_post_close::macro_stage;
use crate::push_foundation::BusinessIntentStore;

const STARTED_LOCAL: &str = "2026-09-14T15:31:00+08:00";
const PLAN_OWNER: &str = "TEST_CODE_PRE_EFFECT_PLAN_OWNER";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RefusalCase {
    ExpiredHeldLease,
    ReplacedHeldLease,
    WrongRunId,
    WrongInput,
}

impl RefusalCase {
    fn label(self) -> &'static str {
        match self {
            Self::ExpiredHeldLease => "expired held lease",
            Self::ReplacedHeldLease => "replaced held lease",
            Self::WrongRunId => "wrong run id",
            Self::WrongInput => "wrong fixed input",
        }
    }

    fn run_id(self) -> &'static str {
        match self {
            Self::ExpiredHeldLease => "TEST_CODE_RUN_PRE_EFFECT_EXPIRED",
            Self::ReplacedHeldLease => "TEST_CODE_RUN_PRE_EFFECT_REPLACED",
            Self::WrongRunId => "TEST_CODE_RUN_PRE_EFFECT_WRONG_RUN",
            Self::WrongInput => "TEST_CODE_RUN_PRE_EFFECT_WRONG_INPUT",
        }
    }

    fn fault_owner(self) -> &'static str {
        match self {
            Self::ExpiredHeldLease => "TEST_CODE_PRE_EFFECT_EXPIRED_OWNER",
            Self::ReplacedHeldLease => "TEST_CODE_PRE_EFFECT_REPLACED_OWNER_A",
            Self::WrongRunId => "TEST_CODE_PRE_EFFECT_WRONG_RUN_OWNER",
            Self::WrongInput => "TEST_CODE_PRE_EFFECT_WRONG_INPUT_OWNER",
        }
    }

    fn positive_owner(self) -> &'static str {
        match self {
            Self::ExpiredHeldLease => "TEST_CODE_PRE_EFFECT_EXPIRED_POSITIVE",
            Self::ReplacedHeldLease => "TEST_CODE_PRE_EFFECT_REPLACED_POSITIVE",
            Self::WrongRunId => "TEST_CODE_PRE_EFFECT_WRONG_RUN_POSITIVE",
            Self::WrongInput => "TEST_CODE_PRE_EFFECT_WRONG_INPUT_POSITIVE",
        }
    }

    fn is_held(self) -> bool {
        matches!(self, Self::ExpiredHeldLease | Self::ReplacedHeldLease)
    }
}

fn assert_pristine_plan(
    recovery: &macro_stage::MacroRecovery,
    original: &control_unknown_commit_tests::OriginalEvidence,
) {
    control_unknown_commit_tests::assert_original(recovery, original);
    assert!(!recovery.is_complete());
    assert!(!recovery.has_unconfirmed_effect());
    assert!(recovery.attempts().is_empty());
    assert!(recovery
        .global_news(GlobalNewsProvider::Eastmoney)
        .is_none());
    let episodes = recovery.readiness_episodes();
    assert_eq!(episodes.len(), 1);
    assert_eq!(episodes[0].ready_result_version(), None);
    let controls = episodes[0].controls();
    assert_eq!(controls.len(), 2);
    for control in controls {
        assert_eq!(control.begin_version(), None);
        assert_eq!(control.result_version(), None);
        assert_eq!(control.outcome(), None);
        assert_eq!(control.response_bytes(), None);
    }
}

fn assert_direct_refusal(case: RefusalCase, error: &ChainPostCloseError, intent: &IntentId) {
    match case {
        RefusalCase::WrongRunId => assert!(matches!(
            error,
            ChainPostCloseError::StaleLease { intent_id }
                if intent_id == intent.as_str()
        )),
        RefusalCase::WrongInput => {
            assert_eq!(error, &ChainPostCloseError::SchemaRejected)
        }
        _ => panic!("TEST_CODE {} is not a constructor refusal", case.label()),
    }
}

fn assert_held_refusal(case: RefusalCase, error: &anyhow::Error, intent: &IntentId) {
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::AuthorityRejected { intent_id })
            if intent_id == intent.as_str()
    ));
    match case {
        RefusalCase::ExpiredHeldLease => assert!(matches!(
            error.downcast_ref::<ChainPostCloseError>(),
            Some(ChainPostCloseError::LeaseExpired { intent_id })
                if intent_id == intent.as_str()
        )),
        RefusalCase::ReplacedHeldLease => assert!(matches!(
            error.downcast_ref::<ChainPostCloseError>(),
            Some(ChainPostCloseError::StaleLease { intent_id })
                if intent_id == intent.as_str()
        )),
        _ => panic!("TEST_CODE {} is not a held-IO refusal", case.label()),
    }
}

fn drift_fixed_input(input: &FixedChainPreparationInput) -> FixedChainPreparationInput {
    let mut bytes = input.encode().unwrap();
    let marker = br#""name":""#;
    let marker_at = bytes
        .windows(marker.len())
        .position(|window| window == marker)
        .expect("TEST_CODE fixed input has a stock name");
    let value_at = marker_at + marker.len();
    let value_end = bytes[value_at..]
        .iter()
        .position(|byte| *byte == b'"')
        .map(|offset| value_at + offset)
        .expect("TEST_CODE fixed input stock name closes");
    bytes.splice(
        value_at..value_end,
        b"TEST_CODE_CHANGED_FIXED_INPUT".iter().copied(),
    );
    FixedChainPreparationInput::decode(&bytes).unwrap()
}

fn take_state(
    database: &std::path::Path,
    baseline: &control_tests::ExternalParentBaseline,
    original: &control_unknown_commit_tests::OriginalEvidence,
) -> (
    control_unknown_commit_tests::RunSnapshot,
    control_unknown_commit_tests::FixedSnapshot,
    Vec<Vec<rusqlite::types::Value>>,
) {
    let (recovery, run) =
        control_unknown_commit_tests::inspect_at(database, &baseline.config, &baseline.intent);
    assert_pristine_plan(&recovery, original);
    (
        run,
        control_unknown_commit_tests::fixed_snapshot(database, &baseline.intent),
        control_tests::audit_snapshot_at(database),
    )
}

fn replace_owner(
    database: &std::path::Path,
    baseline: &control_tests::ExternalParentBaseline,
    expected_head: u64,
    expected_generation: u64,
    now: i64,
) {
    let mut store = BusinessIntentStore::open(database).unwrap();
    let mut local = store
        .single_user_local_chain_post_close(&baseline.config)
        .unwrap();
    let replacement = local
        .resume_run(
            &baseline.intent,
            macro_lease(
                "TEST_CODE_PRE_EFFECT_REPLACED_OWNER_B",
                now,
                now + 2_000_000,
                expected_head,
            ),
        )
        .unwrap();
    assert!(replacement.head_version() > expected_head);
    assert_eq!(replacement.generation(), expected_generation + 1);
    drop(local);
    store.connection.close().unwrap();
}

async fn start_original_health(
    business: &mut V2BusinessFixture,
    baseline: &control_tests::ExternalParentBaseline,
    external: &ExternalMtlsMacroFixture,
    original: &control_unknown_commit_tests::OriginalEvidence,
    expected_head: u64,
    case: RefusalCase,
) {
    business.reopen();
    let source =
        GrpcSource::from_external_macro_bundle_for_test(external.bundle_path().to_path_buf());
    let now = micros(STARTED_LOCAL) + 6_000_000;
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
            macro_lease(case.positive_owner(), now, now + 4_000_000, expected_head),
        )
        .unwrap();
    let registered = control_unknown_commit_tests::registered();
    let search_service = macro_search_service(&registered);
    let mut io = local
        .macro_preparation_io_v11(
            lease,
            &baseline.queries,
            &clock,
            FixedClusterConfiguration::resolve(Some("2")),
            &baseline.source,
            &source,
            &search_service,
        )
        .unwrap();
    let mut pending = Box::pin(io.macro_search_with_budget());
    let watchdog = std::time::Instant::now() + Duration::from_secs(5);
    let wire = loop {
        match futures::poll!(&mut pending) {
            std::task::Poll::Pending => {}
            std::task::Poll::Ready(result) => panic!(
                "TEST_CODE {} positive Health completed before gate: {result:?}",
                case.label()
            ),
        }
        let wire = external.snapshot();
        if wire.health_requests.len() == 1 {
            break wire;
        }
        assert!(
            std::time::Instant::now() < watchdog,
            "TEST_CODE {} positive Health receipt watchdog",
            case.label()
        );
        tokio::task::yield_now().await;
    };
    assert_eq!(wire.tcp_accepts, 1);
    assert_eq!(wire.health_requests, vec![original.health.bytes.clone()]);
    assert_eq!(wire.health_authorized, vec![true]);
    assert!(wire.health_responses.is_empty());
    assert!(wire.health_statuses.is_empty());
    assert_eq!(wire.capabilities_calls, 0);
    assert!(wire.capabilities_requests.is_empty());
    assert_eq!(wire.data_calls, 0);
    assert!(wire.data_requests.is_empty());
    drop(pending);
    drop(io);
    drop(local);
    drop(source);

    let (recovery, run) = control_unknown_commit_tests::inspect_at(
        &business.database(),
        &baseline.config,
        &baseline.intent,
    );
    control_unknown_commit_tests::assert_original(&recovery, original);
    assert!(recovery.has_unconfirmed_effect());
    assert!(recovery.attempts().is_empty());
    assert!(recovery
        .global_news(GlobalNewsProvider::Eastmoney)
        .is_none());
    let controls = recovery.readiness_episodes()[0].controls();
    assert!(controls[0].begin_version().is_some());
    assert_eq!(controls[0].result_version(), None);
    assert_eq!(controls[0].outcome(), None);
    assert_eq!(controls[0].response_bytes(), None);
    assert_eq!(controls[1].begin_version(), None);
    assert_eq!(controls[1].result_version(), None);
    assert!(run.head > expected_head);
    assert_eq!(run.owner, case.positive_owner());
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
    assert_eq!(fixed.data_begins, 0);
    assert_eq!(fixed.source_finals, 0);
}

async fn run_case(case: RefusalCase) {
    let mut business = V2BusinessFixture::new();
    let mut parent_server = None;
    let mut external_server = None;
    let body =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(120), async {
            let baseline = control_tests::setup_external_parent(
                &mut business,
                &mut parent_server,
                case.run_id(),
            )
            .await;
            external_server = Some(
                ExternalMtlsMacroFixture::bind_data_success_for_test()
                    .await
                    .unwrap(),
            );
            let external = external_server.as_ref().unwrap();
            let database = business.database();
            let (original, plan_head) =
                control_unknown_commit_tests::plan_health_without_begin_for_owner(
                    &mut business,
                    &baseline,
                    external,
                    PLAN_OWNER,
                );
            assert_eq!(external.snapshot(), ExternalControlObservation::default());
            business.reopen();

            let started_at = micros(STARTED_LOCAL);
            let fault_now = started_at + 2_000_000;
            let fault_until = if case.is_held() {
                started_at + 3_000_000
            } else {
                started_at + 5_000_000
            };
            let source = GrpcSource::from_external_macro_bundle_for_test(
                external.bundle_path().to_path_buf(),
            );
            let clock = MacroClock {
                now: Cell::new(UtcMicros::try_new(fault_now).unwrap()),
                observation: DateTime::parse_from_rfc3339(STARTED_LOCAL).unwrap(),
                observation_calls: Cell::new(0),
            };
            let mut local = business
                .store
                .as_mut()
                .unwrap()
                .single_user_local_chain_post_close(&baseline.config)
                .unwrap();
            let mut lease = local
                .resume_run(
                    &baseline.intent,
                    macro_lease(case.fault_owner(), fault_now, fault_until, plan_head),
                )
                .unwrap();
            if case == RefusalCase::WrongRunId {
                lease.run_id = RunId::try_new("TEST_CODE_WRONG_RUN_ID".to_owned()).unwrap();
            }
            if case == RefusalCase::WrongInput {
                lease.input = drift_fixed_input(&lease.input);
            }
            let registered = control_unknown_commit_tests::registered();
            let search_service = macro_search_service(&registered);

            let (before_run, before_fixed, before_audit) = if case.is_held() {
                let mut io = local
                    .macro_preparation_io_v11(
                        lease,
                        &baseline.queries,
                        &clock,
                        FixedClusterConfiguration::resolve(Some("2")),
                        &baseline.source,
                        &source,
                        &search_service,
                    )
                    .unwrap();
                if case == RefusalCase::ReplacedHeldLease {
                    let (_, current) = control_unknown_commit_tests::inspect_at(
                        &database,
                        &baseline.config,
                        &baseline.intent,
                    );
                    replace_owner(
                        &database,
                        &baseline,
                        current.head,
                        current.generation,
                        started_at + 3_000_000,
                    );
                }
                clock
                    .now
                    .set(UtcMicros::try_new(started_at + 3_000_000).unwrap());
                let before = take_state(&database, &baseline, &original);
                let stopped = io
                    .macro_search_with_budget()
                    .await
                    .expect("TEST_CODE held refusal has no elapsed timeout")
                    .expect_err("TEST_CODE held refusal must stop before Health");
                assert_held_refusal(case, &stopped, &baseline.intent);
                drop(io);
                before
            } else {
                let before = take_state(&database, &baseline, &original);
                let result = local.macro_preparation_io_v11(
                    lease,
                    &baseline.queries,
                    &clock,
                    FixedClusterConfiguration::resolve(Some("2")),
                    &baseline.source,
                    &source,
                    &search_service,
                );
                let error = match result {
                    Ok(_) => panic!(
                        "TEST_CODE {} constructor must reject before Health",
                        case.label()
                    ),
                    Err(error) => error,
                };
                assert_direct_refusal(case, &error, &baseline.intent);
                before
            };
            drop(local);
            drop(source);

            tokio::task::yield_now().await;
            assert_eq!(external.snapshot(), ExternalControlObservation::default());
            let (after_run, after_fixed, after_audit) = take_state(&database, &baseline, &original);
            assert_eq!(after_run, before_run);
            assert_eq!(after_fixed, before_fixed);
            assert_eq!(after_audit, before_audit);
            assert_eq!(after_audit, baseline.audit);
            assert_eq!(after_run.context, baseline.context);

            start_original_health(
                &mut business,
                &baseline,
                external,
                &original,
                after_run.head,
                case,
            )
            .await;
            control_unknown_commit_tests::assert_parent_unchanged(
                &mut business,
                parent_server.as_ref().unwrap(),
                &baseline,
            );
            control_unknown_commit_tests::release_all(external);
        }))
        .catch_unwind()
        .await;

    let cleanup = std::panic::AssertUnwindSafe(control_tests::cleanup_external_case(
        &mut business,
        &mut parent_server,
        &mut external_server,
        case.label(),
    ))
    .catch_unwind()
    .await;
    cleanup.unwrap_or_else(|_| panic!("TEST_CODE {} cleanup panic", case.label()));
    match body {
        Ok(result) => result.unwrap_or_else(|_| panic!("TEST_CODE {} body timeout", case.label())),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test]
async fn single_user_external_macro_expired_held_io_rejects_before_health_then_original_starts() {
    run_case(RefusalCase::ExpiredHeldLease).await;
}

#[tokio::test]
async fn single_user_external_macro_replaced_held_io_rejects_before_health_then_original_starts() {
    run_case(RefusalCase::ReplacedHeldLease).await;
}

#[tokio::test]
async fn single_user_external_macro_wrong_run_rejects_before_health_then_original_starts() {
    run_case(RefusalCase::WrongRunId).await;
}

#[tokio::test]
async fn single_user_external_macro_wrong_input_rejects_before_health_then_original_starts() {
    run_case(RefusalCase::WrongInput).await;
}
