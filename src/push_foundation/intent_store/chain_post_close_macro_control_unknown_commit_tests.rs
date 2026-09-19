use super::*;
use crate::data_gateway::grpc_source::macro_queries::PreparedMacroQueries;
use crate::grpc_client::client::board_loopback_fixture::BoardLoopbackServer;
use crate::grpc_client::client::external_control_attempt::ExternalControlKind;
use crate::grpc_client::client::external_control_loopback_fixture::{
    test_external_build_identity, test_external_observability, CapabilitiesReply,
    ExternalControlObservation, ExternalMtlsMacroFixture, HealthReply,
};
use crate::grpc_client::client::ContractProfile;
use crate::grpc_client::external_pb::magic::market::v1::{
    AdmissionState, CapabilitiesResponse, Capability, HealthResponse, Operation,
};
use crate::push_foundation::intent_store::chain_post_close::macro_codec;
use crate::push_foundation::intent_store::chain_post_close::macro_stage::{
    MacroControlOutcome, MacroRecovery,
};
use rusqlite::{Connection, OpenFlags};

const AUTHORITY: &str = "grpc-mtls:macro.test.invalid";
const STARTED_LOCAL: &str = "2026-09-14T15:31:00+08:00";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ControlTarget {
    Health,
    Capabilities,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FaultPoint {
    ReceiptCancelled,
    BeginCommit,
    ResultCommit,
}

#[derive(Clone, Copy)]
struct Case {
    target: ControlTarget,
    fault: FaultPoint,
}

impl Case {
    fn label(self) -> &'static str {
        match (self.target, self.fault) {
            (ControlTarget::Health, FaultPoint::ReceiptCancelled) => "Health receipt cancel",
            (ControlTarget::Capabilities, FaultPoint::ReceiptCancelled) => {
                "Capabilities receipt cancel"
            }
            (ControlTarget::Health, FaultPoint::BeginCommit) => "Health begin COMMIT",
            (ControlTarget::Capabilities, FaultPoint::BeginCommit) => "Capabilities begin COMMIT",
            (ControlTarget::Health, FaultPoint::ResultCommit) => "Health result COMMIT",
            (ControlTarget::Capabilities, FaultPoint::ResultCommit) => "Capabilities result COMMIT",
        }
    }

    fn run_id(self) -> &'static str {
        match (self.target, self.fault) {
            (ControlTarget::Health, FaultPoint::ReceiptCancelled) => {
                "TEST_CODE_RUN_EXTERNAL_HEALTH_RECEIPT_CANCEL"
            }
            (ControlTarget::Capabilities, FaultPoint::ReceiptCancelled) => {
                "TEST_CODE_RUN_EXTERNAL_CAPS_RECEIPT_CANCEL"
            }
            (ControlTarget::Health, FaultPoint::BeginCommit) => {
                "TEST_CODE_RUN_EXTERNAL_HEALTH_BEGIN_COMMIT"
            }
            (ControlTarget::Capabilities, FaultPoint::BeginCommit) => {
                "TEST_CODE_RUN_EXTERNAL_CAPS_BEGIN_COMMIT"
            }
            (ControlTarget::Health, FaultPoint::ResultCommit) => {
                "TEST_CODE_RUN_EXTERNAL_HEALTH_RESULT_COMMIT"
            }
            (ControlTarget::Capabilities, FaultPoint::ResultCommit) => {
                "TEST_CODE_RUN_EXTERNAL_CAPS_RESULT_COMMIT"
            }
        }
    }

    fn owner(self, phase: &str) -> String {
        format!(
            "TEST_CODE_{}_{}_{}",
            match self.target {
                ControlTarget::Health => "HEALTH",
                ControlTarget::Capabilities => "CAPS",
            },
            match self.fault {
                FaultPoint::ReceiptCancelled => "CANCEL",
                FaultPoint::BeginCommit => "BEGIN_COMMIT",
                FaultPoint::ResultCommit => "RESULT_COMMIT",
            },
            phase
        )
    }

    fn start_offset(self) -> i64 {
        match self.target {
            ControlTarget::Health => 0,
            ControlTarget::Capabilities => 3_000_000,
        }
    }

    fn begin_fault_offset(self) -> i64 {
        match self.target {
            ControlTarget::Health => 2_000_000,
            ControlTarget::Capabilities => 3_000_000,
        }
    }

    fn reopen_offset(self) -> i64 {
        match (self.target, self.fault) {
            (ControlTarget::Health, FaultPoint::BeginCommit) => 4_000_000,
            (ControlTarget::Health, _) => 3_000_000,
            (ControlTarget::Capabilities, _) => 6_000_000,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RequestEvidence {
    pub(super) id: String,
    pub(super) bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct OriginalEvidence {
    pub(super) plan_bytes: Vec<u8>,
    pub(super) endpoint: String,
    pub(super) data: RequestEvidence,
    pub(super) health: RequestEvidence,
    pub(super) capabilities: RequestEvidence,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RunSnapshot {
    pub(super) head: u64,
    pub(super) owner: String,
    pub(super) generation: u64,
    pub(super) context: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct FixedSnapshot {
    pub(super) health_raw: Option<Vec<u8>>,
    pub(super) capabilities_raw: Option<Vec<u8>>,
    pub(super) control_results: i64,
    pub(super) data_begins: i64,
    pub(super) source_finals: i64,
}

struct FaultEvidence {
    original: OriginalEvidence,
    target_begin: u64,
    target_head: u64,
    target_generation: u64,
    wire: ExternalControlObservation,
    stop: Option<anyhow::Error>,
}

pub(super) fn registered() -> [GeneralWebResearchProvider; 3] {
    [
        GeneralWebResearchProvider::SerpApi,
        GeneralWebResearchProvider::Bocha,
        GeneralWebResearchProvider::Tavily,
    ]
}

fn all_pending_sources() -> Vec<MacroQueryIdentity> {
    let mut sources = vec![MacroQueryIdentity::GlobalNews {
        provider: GlobalNewsProvider::Eastmoney,
        limit: 20,
    }];
    sources.extend(control_tests::pending_sources());
    sources
}

pub(super) fn inspect_at(
    database: &std::path::Path,
    config: &crate::monitor::push_job::LocalChainPostCloseConfig,
    intent: &IntentId,
) -> (MacroRecovery, RunSnapshot) {
    let mut reader = BusinessIntentStore::open(database).unwrap();
    let mut local = reader.single_user_local_chain_post_close(config).unwrap();
    let recovery = local.inspect_macro(intent).unwrap();
    let run = local.inspect_run(intent).unwrap();
    let head = run.head_version();
    let generation = run.lease_generation();
    let context = run.context().canonical_bytes();
    drop(local);
    let owner = reader
        .connection
        .query_row(
            "SELECT lease_owner FROM chain_post_close_runs WHERE intent_id=?1",
            [intent.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    let snapshot = RunSnapshot {
        head,
        owner,
        generation,
        context,
    };
    reader.connection.close().unwrap();
    (recovery, snapshot)
}

pub(super) fn fixed_snapshot(database: &std::path::Path, intent: &IntentId) -> FixedSnapshot {
    let reader = BusinessIntentStore::open(database).unwrap();
    let raw = |ordinal| {
        reader
            .connection
            .query_row(
                "SELECT bytes FROM chain_post_close_macro_control_attempt_results \
                 WHERE intent_id=?1 AND episode_ordinal=1 AND control_ordinal=?2",
                rusqlite::params![intent.as_str(), ordinal],
                |row| row.get(0),
            )
            .optional()
            .unwrap()
    };
    let count = |sql: &str| {
        reader
            .connection
            .query_row(sql, [intent.as_str()], |row| row.get(0))
            .unwrap()
    };
    let snapshot = FixedSnapshot {
        health_raw: raw(1),
        capabilities_raw: raw(2),
        control_results: count(
            "SELECT count(*) FROM chain_post_close_macro_control_attempt_results \
             WHERE intent_id=?1",
        ),
        data_begins: count(
            "SELECT count(*) FROM chain_post_close_macro_attempt_begins WHERE intent_id=?1",
        ),
        source_finals: count(
            "SELECT count(*) FROM chain_post_close_macro_source_finals WHERE intent_id=?1",
        ),
    };
    reader.connection.close().unwrap();
    snapshot
}

fn capture_original(
    recovery: &MacroRecovery,
    baseline: &control_tests::ExternalParentBaseline,
    external: &ExternalMtlsMacroFixture,
) -> OriginalEvidence {
    assert!(!recovery.is_complete());
    assert!(recovery.attempts().is_empty());
    assert!(recovery
        .global_news(GlobalNewsProvider::Eastmoney)
        .is_none());
    assert_eq!(recovery.parent_final_bytes(), baseline.final_bytes);
    let plan = recovery.plan();
    assert_eq!(plan.profile(), ContractProfile::ExternalV1);
    assert_eq!(plan.acquisition_authority(), Some(AUTHORITY));
    assert_eq!(plan.endpoint(), external.endpoint());
    let started_at = micros(STARTED_LOCAL);
    assert_eq!(plan.started_at().get(), started_at);
    assert_eq!(plan.deadline_at().get(), started_at + 15_000_000);
    assert_eq!(plan.observed_local(), STARTED_LOCAL);
    assert_eq!(
        plan.first_source_request().retry_policy(),
        (4, 1000, 60_000, 200)
    );
    assert_eq!(
        recovery.pending_source_identities(),
        all_pending_sources().as_slice()
    );
    assert_eq!(
        recovery.pending_research_queries(),
        control_tests::pending_research().as_slice()
    );
    let episodes = recovery.readiness_episodes();
    assert_eq!(episodes.len(), 1);
    assert_eq!(episodes[0].episode_ordinal(), 1);
    assert_eq!(
        episodes[0].initiating_source(),
        &MacroQueryIdentity::GlobalNews {
            provider: GlobalNewsProvider::Eastmoney,
            limit: 20,
        }
    );
    let controls = episodes[0].controls();
    assert_eq!(controls.len(), 2);
    assert_eq!(controls[0].kind(), ExternalControlKind::Health);
    assert_eq!(controls[1].kind(), ExternalControlKind::Capabilities);
    OriginalEvidence {
        plan_bytes: recovery.plan_bytes().to_vec(),
        endpoint: plan.endpoint().to_owned(),
        data: RequestEvidence {
            id: plan.first_source_request().request_id().to_owned(),
            bytes: plan.first_source_request().request_bytes().to_vec(),
        },
        health: RequestEvidence {
            id: controls[0].request_id().to_owned(),
            bytes: controls[0].request_bytes().to_vec(),
        },
        capabilities: RequestEvidence {
            id: controls[1].request_id().to_owned(),
            bytes: controls[1].request_bytes().to_vec(),
        },
    }
}

pub(super) fn assert_original(recovery: &MacroRecovery, expected: &OriginalEvidence) {
    assert_eq!(recovery.plan_bytes(), expected.plan_bytes.as_slice());
    let plan = recovery.plan();
    assert_eq!(plan.profile(), ContractProfile::ExternalV1);
    assert_eq!(plan.acquisition_authority(), Some(AUTHORITY));
    assert_eq!(plan.endpoint(), expected.endpoint);
    assert_eq!(plan.started_at().get(), micros(STARTED_LOCAL));
    assert_eq!(plan.deadline_at().get(), micros(STARTED_LOCAL) + 15_000_000);
    assert_eq!(plan.observed_local(), STARTED_LOCAL);
    assert_eq!(
        plan.first_source_request().request_id(),
        expected.data.id.as_str()
    );
    assert_eq!(
        plan.first_source_request().request_bytes(),
        expected.data.bytes.as_slice()
    );
    assert_eq!(
        plan.first_source_request().retry_policy(),
        (4, 1000, 60_000, 200)
    );
    let controls = recovery.readiness_episodes()[0].controls();
    assert_eq!(controls[0].request_id(), expected.health.id.as_str());
    assert_eq!(
        controls[0].request_bytes(),
        expected.health.bytes.as_slice()
    );
    assert_eq!(controls[1].request_id(), expected.capabilities.id.as_str());
    assert_eq!(
        controls[1].request_bytes(),
        expected.capabilities.bytes.as_slice()
    );
    assert_eq!(
        recovery.pending_source_identities(),
        all_pending_sources().as_slice()
    );
    assert_eq!(
        recovery.pending_research_queries(),
        control_tests::pending_research().as_slice()
    );
}

fn expected_health_ready(request_id: &str) -> Vec<u8> {
    HealthResponse {
        request_id: request_id.to_owned(),
        live: true,
        ready: true,
        state: "TEST_CODE_HEALTH_RUNNING".to_owned(),
        observability: Some(test_external_observability()),
        build_identity: Some(test_external_build_identity()),
    }
    .encode_to_vec()
}

fn expected_rejection(target: ControlTarget, request_id: &str) -> Vec<u8> {
    match target {
        ControlTarget::Health => HealthResponse {
            request_id: request_id.to_owned(),
            live: true,
            ready: false,
            state: "TEST_CODE_HEALTH_NOT_READY".to_owned(),
            observability: Some(test_external_observability()),
            build_identity: Some(test_external_build_identity()),
        }
        .encode_to_vec(),
        ControlTarget::Capabilities => CapabilitiesResponse {
            request_id: request_id.to_owned(),
            capabilities: vec![
                Capability {
                    operation: Operation::GlobalNews as i32,
                    repository_admission: AdmissionState::Admitted as i32,
                    runtime_available: false,
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
        .encode_to_vec(),
    }
}

fn assert_checkpoint_health(
    recovery: &MacroRecovery,
    checkpoint: &control_recovery_tests::ConfirmedHealthCheckpoint,
) {
    let episode = &recovery.readiness_episodes()[0];
    let controls = episode.controls();
    assert_eq!(
        controls[0].begin_version(),
        Some(checkpoint.health.begin_version)
    );
    assert_eq!(
        controls[0].result_version(),
        Some(checkpoint.health.result_version)
    );
    assert_eq!(controls[0].outcome(), Some(MacroControlOutcome::Ready));
    assert_eq!(
        controls[0].response_bytes(),
        Some(checkpoint.health.response_bytes.as_slice())
    );
    assert_eq!(controls[0].request_id(), checkpoint.health.request.id);
    assert_eq!(controls[0].request_bytes(), checkpoint.health.request.bytes);
}

fn assert_target_pending(
    case: Case,
    recovery: &MacroRecovery,
    checkpoint: Option<&control_recovery_tests::ConfirmedHealthCheckpoint>,
) -> u64 {
    assert!(recovery.has_unconfirmed_effect());
    assert!(recovery.attempts().is_empty());
    assert!(recovery
        .global_news(GlobalNewsProvider::Eastmoney)
        .is_none());
    let episode = &recovery.readiness_episodes()[0];
    assert_eq!(episode.ready_result_version(), None);
    let controls = episode.controls();
    let target = match case.target {
        ControlTarget::Health => {
            assert_eq!(controls[1].begin_version(), None);
            assert_eq!(controls[1].result_version(), None);
            assert_eq!(controls[1].outcome(), None);
            assert_eq!(controls[1].response_bytes(), None);
            &controls[0]
        }
        ControlTarget::Capabilities => {
            assert_checkpoint_health(recovery, checkpoint.unwrap());
            &controls[1]
        }
    };
    let begin = target
        .begin_version()
        .expect("TEST_CODE control begin committed before receipt");
    assert_eq!(target.result_version(), None);
    assert_eq!(target.outcome(), None);
    assert_eq!(target.response_bytes(), None);
    begin
}

fn assert_fixed_pending(
    case: Case,
    fixed: &FixedSnapshot,
    checkpoint: Option<&control_recovery_tests::ConfirmedHealthCheckpoint>,
) {
    match case.target {
        ControlTarget::Health => {
            assert_eq!(fixed.health_raw, None);
            assert_eq!(fixed.capabilities_raw, None);
            assert_eq!(fixed.control_results, 0);
        }
        ControlTarget::Capabilities => {
            assert_eq!(
                fixed.health_raw.as_deref(),
                Some(checkpoint.unwrap().raw.as_slice())
            );
            assert_eq!(fixed.capabilities_raw, None);
            assert_eq!(fixed.control_results, 1);
        }
    }
    assert_eq!(fixed.data_begins, 0);
    assert_eq!(fixed.source_finals, 0);
}

async fn bind_case(case: Case) -> Result<ExternalMtlsMacroFixture, String> {
    match (case.target, case.fault) {
        (ControlTarget::Health, FaultPoint::ResultCommit) => {
            ExternalMtlsMacroFixture::bind_health_reply_for_test(HealthReply::NotReady).await
        }
        (ControlTarget::Capabilities, FaultPoint::ResultCommit) => {
            ExternalMtlsMacroFixture::bind_capabilities_reply_for_test(
                CapabilitiesReply::GlobalNewsRuntimeUnavailable,
            )
            .await
        }
        _ => ExternalMtlsMacroFixture::bind_data_success_for_test().await,
    }
}

fn open_fault_reader(database: &std::path::Path) -> Connection {
    let reader = Connection::open_with_flags(
        database,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .unwrap();
    reader.busy_timeout(Duration::from_millis(250)).unwrap();
    reader
}

fn rollback_fault_reader(reader: &mut Option<Connection>) {
    if let Some(reader) = reader.take() {
        if !reader.is_autocommit() {
            reader.execute_batch("ROLLBACK;").unwrap();
        }
        reader.close().unwrap();
    }
}

fn assert_result_unconfirmed(error: &anyhow::Error, intent: &IntentId) {
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::ResultUnconfirmed { intent_id })
            if intent_id == intent.as_str()
    ));
    assert!(matches!(
        error.downcast_ref::<ChainPostCloseError>(),
        Some(ChainPostCloseError::StorageFailed {
            operation: "macro control result commit"
        })
    ));
    let failure = error.downcast_ref::<PreparationFailure>().unwrap();
    assert_eq!(failure.stage(), PreparationStage::Macro);
    assert_eq!(
        failure.completed_stages().last(),
        Some(&PreparationStage::DragonTiger)
    );
    assert!(!failure
        .completed_stages()
        .contains(&PreparationStage::Macro));
}

fn assert_begin_commit_failed(error: &anyhow::Error, intent: &IntentId) {
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::AuthorityRejected { intent_id })
            if intent_id == intent.as_str()
    ));
    assert!(matches!(
        error.downcast_ref::<ChainPostCloseError>(),
        Some(ChainPostCloseError::StorageFailed {
            operation: "macro control begin commit"
        })
    ));
    let failure = error.downcast_ref::<PreparationFailure>().unwrap();
    assert_eq!(failure.stage(), PreparationStage::Macro);
    assert_eq!(
        failure.completed_stages().last(),
        Some(&PreparationStage::DragonTiger)
    );
    assert!(!failure
        .completed_stages()
        .contains(&PreparationStage::Macro));
}

pub(super) fn release_all(external: &ExternalMtlsMacroFixture) {
    external.set_reject_new_connections_for_test(false);
    external.release_health();
    external.release_capabilities();
    external.release_data();
}

async fn release_and_observe_fixed_reply(
    case: Case,
    external: &ExternalMtlsMacroFixture,
    original: &OriginalEvidence,
) -> ExternalControlObservation {
    release_all(external);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let wire = external.snapshot();
        let response = match case.target {
            ControlTarget::Health => wire.health_responses.first(),
            ControlTarget::Capabilities => wire.capabilities_responses.first(),
        };
        if let Some(response) = response {
            assert_eq!(
                match case.target {
                    ControlTarget::Health => wire.health_responses.len(),
                    ControlTarget::Capabilities => wire.capabilities_responses.len(),
                },
                1
            );
            let expected = if case.fault == FaultPoint::ResultCommit {
                expected_rejection(
                    case.target,
                    match case.target {
                        ControlTarget::Health => &original.health.id,
                        ControlTarget::Capabilities => &original.capabilities.id,
                    },
                )
            } else {
                match case.target {
                    ControlTarget::Health => expected_health_ready(&original.health.id),
                    ControlTarget::Capabilities => {
                        expected_capabilities_ready(&original.capabilities.id)
                    }
                }
            };
            assert_eq!(response, &expected);
            assert!(wire.health_statuses.is_empty());
            assert!(wire.capabilities_statuses.is_empty());
            return wire;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "TEST_CODE {} fixed reply watchdog",
            case.label()
        );
        tokio::task::yield_now().await;
    }
}

pub(super) fn assert_parent_unchanged(
    business: &mut V2BusinessFixture,
    parent_server: &BoardLoopbackServer,
    baseline: &control_tests::ExternalParentBaseline,
) {
    assert_eq!(parent_server.snapshot(), baseline.network);
    assert_eq!(parent_server.membership_snapshot(), baseline.memberships);
    assert_eq!(
        old_fact_rows(business.connection(), &baseline.tables),
        baseline.facts
    );
    let transaction = business.connection().unchecked_transaction().unwrap();
    read_acquisition_in_transaction(&transaction, &baseline.receipt).unwrap();
    transaction.commit().unwrap();
}

pub(super) fn plan_health_without_begin_for_owner(
    business: &mut V2BusinessFixture,
    baseline: &control_tests::ExternalParentBaseline,
    external: &ExternalMtlsMacroFixture,
    owner: &str,
) -> (OriginalEvidence, u64) {
    let source =
        GrpcSource::from_external_macro_bundle_for_test(external.bundle_path().to_path_buf());
    let PreparedMacroQueries::External(prepared) = source.prepare_macro_queries().unwrap() else {
        panic!("TEST_CODE expected External prepared endpoint");
    };
    let data = prepared
        .prepare_macro_query(MacroQueryIdentity::GlobalNews {
            provider: GlobalNewsProvider::Eastmoney,
            limit: 20,
        })
        .unwrap();
    let health = prepared.prepare_health_attempt().unwrap();
    let capabilities = prepared.prepare_capabilities_attempt().unwrap();
    let episode = macro_codec::ReadinessEpisodePlan::new(
        health.request_material(),
        capabilities.request_material(),
    )
    .unwrap();
    let started_at = micros(STARTED_LOCAL);
    let clock = MacroClock {
        now: Cell::new(UtcMicros::try_new(started_at).unwrap()),
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
            macro_lease(
                owner,
                started_at,
                started_at + 1_000_000,
                baseline.head,
            ),
        )
        .unwrap();
    let registered = registered();
    let search_service = macro_search_service(&registered);
    let web = search_service.macro_web_snapshot(&source).unwrap();
    let lease = local
        .plan_macro_request(
            lease,
            clock.now(),
            clock.macro_request_observation(),
            prepared.endpoint_uri(),
            macro_codec::Request::capture_prepared(&data).unwrap(),
            Some(episode),
            &web,
            clock.now(),
        )
        .unwrap();
    let plan_head = lease.head_version();
    let recovery = local.inspect_macro(&baseline.intent).unwrap();
    let original = capture_original(&recovery, baseline, external);
    let controls = recovery.readiness_episodes()[0].controls();
    assert_eq!(controls[0].begin_version(), None);
    assert_eq!(controls[1].begin_version(), None);
    assert!(!recovery.has_unconfirmed_effect());
    assert_eq!(clock.observation_calls.get(), 1);
    drop(local);
    drop(capabilities);
    drop(health);
    drop(data);
    drop(prepared);
    drop(source);
    assert_eq!(external.snapshot(), ExternalControlObservation::default());
    (original, plan_head)
}

async fn exercise_receipt_or_result_fault(
    business: &mut V2BusinessFixture,
    baseline: &control_tests::ExternalParentBaseline,
    external: &ExternalMtlsMacroFixture,
    checkpoint: Option<&control_recovery_tests::ConfirmedHealthCheckpoint>,
    case: Case,
    expected_head: u64,
    fault_reader: &mut Option<Connection>,
) -> FaultEvidence {
    let database = business.database();
    let source =
        GrpcSource::from_external_macro_bundle_for_test(external.bundle_path().to_path_buf());
    let started_at = micros(STARTED_LOCAL);
    let now = started_at + case.start_offset();
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
            macro_lease(&case.owner("FAULT"), now, now + 2_000_000, expected_head),
        )
        .unwrap();
    let registered = registered();
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
    let mut prepared = Box::pin(prepare_chain_analysis_with_io(
        NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
        baseline.stocks.clone(),
        None,
        &mut io,
    ));
    let receipt_deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        match futures::poll!(&mut prepared) {
            std::task::Poll::Pending => {}
            std::task::Poll::Ready(result) => {
                panic!(
                    "TEST_CODE {} returned before receipt: {result:?}",
                    case.label()
                )
            }
        }
        let wire = external.snapshot();
        let seen = match case.target {
            ControlTarget::Health => wire.health_requests.len() == 1,
            ControlTarget::Capabilities => wire.capabilities_requests.len() == 1,
        };
        if seen {
            break;
        }
        assert!(
            std::time::Instant::now() < receipt_deadline,
            "TEST_CODE {} receipt watchdog",
            case.label()
        );
        tokio::task::yield_now().await;
    }
    let (pending, run) = inspect_at(&database, &baseline.config, &baseline.intent);
    let original = capture_original(&pending, baseline, external);
    if let Some(checkpoint) = checkpoint {
        assert_eq!(original.plan_bytes, checkpoint.plan_bytes);
        assert_eq!(original.data.id, checkpoint.data.id);
        assert_eq!(original.data.bytes, checkpoint.data.bytes);
        assert_eq!(original.health.id, checkpoint.health.request.id);
        assert_eq!(original.health.bytes, checkpoint.health.request.bytes);
        assert_eq!(original.capabilities.id, checkpoint.capabilities.id);
        assert_eq!(original.capabilities.bytes, checkpoint.capabilities.bytes);
    }
    let target_begin = assert_target_pending(case, &pending, checkpoint);
    assert_eq!(run.head, target_begin);
    assert_eq!(run.owner, case.owner("FAULT"));
    assert_eq!(
        run.generation,
        match case.target {
            ControlTarget::Health => 3,
            ControlTarget::Capabilities => 4,
        }
    );
    assert_eq!(run.context, baseline.context);
    assert_eq!(control_tests::audit_snapshot_at(&database), baseline.audit);
    assert_fixed_pending(
        case,
        &fixed_snapshot(&database, &baseline.intent),
        checkpoint,
    );
    let receipt = external.snapshot();
    assert_eq!(receipt.data_calls, 0);
    assert!(receipt.data_requests.is_empty());
    assert!(receipt.data_responses.is_empty());
    match case.target {
        ControlTarget::Health => {
            assert_eq!(receipt.tcp_accepts, 1);
            assert_eq!(receipt.health_requests, vec![original.health.bytes.clone()]);
            assert_eq!(receipt.health_authorized, vec![true]);
            assert!(receipt.health_responses.is_empty());
            assert!(receipt.health_statuses.is_empty());
            assert_eq!(receipt.capabilities_calls, 0);
            assert!(receipt.capabilities_requests.is_empty());
        }
        ControlTarget::Capabilities => {
            let checkpoint = checkpoint.unwrap();
            assert_eq!(receipt.tcp_accepts, 2);
            assert_eq!(
                receipt.health_requests,
                vec![checkpoint.health.request.bytes.clone()]
            );
            assert_eq!(
                receipt.health_responses,
                vec![checkpoint.health.response_bytes.clone()]
            );
            assert_eq!(
                receipt.capabilities_requests,
                vec![original.capabilities.bytes.clone()]
            );
            assert_eq!(receipt.capabilities_authorized, vec![true]);
            assert!(receipt.capabilities_responses.is_empty());
            assert!(receipt.capabilities_statuses.is_empty());
        }
    }

    let stop = if case.fault == FaultPoint::ResultCommit {
        *fault_reader = Some(open_fault_reader(&database));
        let reader = fault_reader.as_ref().unwrap();
        reader.execute_batch("BEGIN DEFERRED;").unwrap();
        let locked_head: u64 = reader
            .query_row(
                "SELECT head_version FROM chain_post_close_runs WHERE intent_id=?1",
                [baseline.intent.as_str()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(locked_head, target_begin);
        clock.now.set(UtcMicros::try_new(now + 1_000_000).unwrap());
        match case.target {
            ControlTarget::Health => external.release_health(),
            ControlTarget::Capabilities => external.release_capabilities(),
        }
        let error = tokio::time::timeout(Duration::from_secs(5), &mut prepared)
            .await
            .expect("TEST_CODE bounded control result COMMIT")
            .expect_err("TEST_CODE SHARED reader must block control result COMMIT");
        assert_result_unconfirmed(&error, &baseline.intent);
        let wire = external.snapshot();
        let expected = expected_rejection(
            case.target,
            match case.target {
                ControlTarget::Health => &original.health.id,
                ControlTarget::Capabilities => &original.capabilities.id,
            },
        );
        match case.target {
            ControlTarget::Health => {
                assert_eq!(wire.health_responses, vec![expected]);
                assert_eq!(wire.capabilities_calls, 0);
            }
            ControlTarget::Capabilities => {
                assert_eq!(wire.capabilities_responses, vec![expected]);
            }
        }
        Some(error)
    } else {
        None
    };
    drop(prepared);
    drop(io);
    drop(local);
    drop(source);
    assert_eq!(
        clock.observation_calls.get(),
        usize::from(case.target == ControlTarget::Health)
    );
    FaultEvidence {
        original,
        target_begin,
        target_head: run.head,
        target_generation: run.generation,
        wire: external.snapshot(),
        stop,
    }
}

async fn exercise_begin_commit_failure(
    business: &mut V2BusinessFixture,
    baseline: &control_tests::ExternalParentBaseline,
    external: &ExternalMtlsMacroFixture,
    checkpoint: Option<&control_recovery_tests::ConfirmedHealthCheckpoint>,
    case: Case,
    original: OriginalEvidence,
    expected_head: u64,
    fault_reader: &mut Option<Connection>,
) -> (OriginalEvidence, u64, u64, ExternalControlObservation) {
    let database = business.database();
    *fault_reader = Some(open_fault_reader(&database));
    let source =
        GrpcSource::from_external_macro_bundle_for_test(external.bundle_path().to_path_buf());
    let started_at = micros(STARTED_LOCAL);
    let now = started_at + case.begin_fault_offset();
    let clock = MacroClock {
        now: Cell::new(UtcMicros::try_new(now).unwrap()),
        observation: DateTime::parse_from_rfc3339("2026-09-14T15:32:00+08:00").unwrap(),
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
            macro_lease(&case.owner("FAULT"), now, now + 1_000_000, expected_head),
        )
        .unwrap();
    let fault_head = local.inspect_run(&baseline.intent).unwrap().head_version();
    let fault_generation = local
        .inspect_run(&baseline.intent)
        .unwrap()
        .lease_generation();
    let registered = registered();
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
    let mut fault_io = super::MacroBeginCommitFaultIo {
        inner: &mut io,
        reader: fault_reader.as_ref().unwrap(),
        expected_intent: baseline.intent.as_str(),
        expected_head: fault_head,
        injections: 0,
    };
    let error = tokio::time::timeout(
        Duration::from_secs(5),
        prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            baseline.stocks.clone(),
            None,
            &mut fault_io,
        ),
    )
    .await
    .expect("TEST_CODE bounded control begin COMMIT")
    .expect_err("TEST_CODE SHARED reader must block control begin COMMIT");
    assert_eq!(fault_io.injections, 1);
    assert_begin_commit_failed(&error, &baseline.intent);
    assert_eq!(clock.observation_calls.get(), 0);
    let wire = external.snapshot();
    match case.target {
        ControlTarget::Health => {
            assert_eq!(wire, ExternalControlObservation::default());
        }
        ControlTarget::Capabilities => {
            let checkpoint = checkpoint.unwrap();
            assert_eq!(wire.tcp_accepts, 1);
            assert_eq!(
                wire.health_requests,
                vec![checkpoint.health.request.bytes.clone()]
            );
            assert_eq!(
                wire.health_responses,
                vec![checkpoint.health.response_bytes.clone()]
            );
            assert_eq!(wire.capabilities_calls, 0);
            assert!(wire.capabilities_requests.is_empty());
            assert_eq!(wire.data_calls, 0);
        }
    }
    drop(fault_io);
    drop(io);
    drop(local);
    drop(source);
    rollback_fault_reader(fault_reader);

    let (rolled_back, run) = inspect_at(&database, &baseline.config, &baseline.intent);
    assert_original(&rolled_back, &original);
    assert!(!rolled_back.has_unconfirmed_effect());
    assert!(!rolled_back.is_complete());
    let episode = &rolled_back.readiness_episodes()[0];
    assert_eq!(episode.ready_result_version(), None);
    let controls = episode.controls();
    let target = match case.target {
        ControlTarget::Health => &controls[0],
        ControlTarget::Capabilities => {
            assert_checkpoint_health(&rolled_back, checkpoint.unwrap());
            &controls[1]
        }
    };
    assert_eq!(target.begin_version(), None);
    assert_eq!(target.result_version(), None);
    assert_eq!(target.outcome(), None);
    assert_eq!(target.response_bytes(), None);
    assert_eq!(run.head, fault_head);
    assert_eq!(run.owner, case.owner("FAULT"));
    assert_eq!(run.generation, fault_generation);
    assert_eq!(run.context, baseline.context);
    assert_eq!(control_tests::audit_snapshot_at(&database), baseline.audit);
    assert_fixed_pending(
        case,
        &fixed_snapshot(&database, &baseline.intent),
        checkpoint,
    );
    (original, fault_head, fault_generation, wire)
}

async fn drive_original_control_to_ready(
    business: &mut V2BusinessFixture,
    baseline: &control_tests::ExternalParentBaseline,
    external: &ExternalMtlsMacroFixture,
    checkpoint: Option<&control_recovery_tests::ConfirmedHealthCheckpoint>,
    case: Case,
    original: &OriginalEvidence,
    expected_head: u64,
    expected_generation: u64,
) -> (u64, ExternalControlObservation) {
    let database = business.database();
    let source =
        GrpcSource::from_external_macro_bundle_for_test(external.bundle_path().to_path_buf());
    let started_at = micros(STARTED_LOCAL);
    let now = started_at + case.reopen_offset();
    let clock = MacroClock {
        now: Cell::new(UtcMicros::try_new(now).unwrap()),
        observation: DateTime::parse_from_rfc3339("2026-09-14T15:33:00+08:00").unwrap(),
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
            macro_lease(&case.owner("RECOVER"), now, now + 2_000_000, expected_head),
        )
        .unwrap();
    let owner_run = local.inspect_run(&baseline.intent).unwrap();
    let owner_baseline = owner_run.head_version();
    assert!(owner_baseline > expected_head);
    assert!(owner_run.lease_generation() > expected_generation);
    let registered = registered();
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
    let mut prepared = Box::pin(prepare_chain_analysis_with_io(
        NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
        baseline.stocks.clone(),
        None,
        &mut io,
    ));
    let receipt_deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        match futures::poll!(&mut prepared) {
            std::task::Poll::Pending => {}
            std::task::Poll::Ready(result) => {
                panic!(
                    "TEST_CODE {} returned before recovered receipt: {result:?}",
                    case.label()
                )
            }
        }
        let wire = external.snapshot();
        let seen = match case.target {
            ControlTarget::Health => wire.health_requests.len() == 1,
            ControlTarget::Capabilities => wire.capabilities_requests.len() == 1,
        };
        if seen {
            break;
        }
        assert!(
            std::time::Instant::now() < receipt_deadline,
            "TEST_CODE {} recovered receipt watchdog",
            case.label()
        );
        tokio::task::yield_now().await;
    }
    let (pending, run) = inspect_at(&database, &baseline.config, &baseline.intent);
    assert_original(&pending, original);
    assert_eq!(run.owner, case.owner("RECOVER"));
    let begin = assert_target_pending(case, &pending, checkpoint);
    assert!(begin > owner_baseline);
    assert_eq!(run.head, begin);
    let target_request = match case.target {
        ControlTarget::Health => &original.health.bytes,
        ControlTarget::Capabilities => &original.capabilities.bytes,
    };
    match case.target {
        ControlTarget::Health => {
            let wire = external.snapshot();
            assert_eq!(wire.health_requests, vec![target_request.to_vec()]);
            assert_eq!(wire.health_authorized, vec![true]);
            external.release_health();
        }
        ControlTarget::Capabilities => {
            let wire = external.snapshot();
            assert_eq!(wire.capabilities_requests, vec![target_request.to_vec()]);
            assert_eq!(wire.capabilities_authorized, vec![true]);
            external.release_capabilities();
        }
    }
    let ready_deadline = std::time::Instant::now() + Duration::from_secs(5);
    let result_version = loop {
        match futures::poll!(&mut prepared) {
            std::task::Poll::Pending => {}
            std::task::Poll::Ready(result) => {
                panic!(
                    "TEST_CODE {} returned before Ready checkpoint: {result:?}",
                    case.label()
                )
            }
        }
        let wire = external.snapshot();
        let responded = match case.target {
            ControlTarget::Health => wire.health_responses.len() == 1,
            ControlTarget::Capabilities => wire.capabilities_responses.len() == 1,
        };
        if responded {
            let (ready, run) = inspect_at(&database, &baseline.config, &baseline.intent);
            assert_original(&ready, original);
            let episode = &ready.readiness_episodes()[0];
            let controls = episode.controls();
            let target = match case.target {
                ControlTarget::Health => {
                    assert_eq!(controls[1].begin_version(), None);
                    &controls[0]
                }
                ControlTarget::Capabilities => {
                    assert_checkpoint_health(&ready, checkpoint.unwrap());
                    assert!(ready.attempts().is_empty());
                    &controls[1]
                }
            };
            if target.outcome() != Some(MacroControlOutcome::Ready) {
                assert_eq!(episode.ready_result_version(), None);
            }
            if target.outcome() == Some(MacroControlOutcome::Ready) {
                let result = target.result_version().unwrap();
                match case.target {
                    ControlTarget::Health => {
                        assert_eq!(episode.ready_result_version(), None);
                    }
                    ControlTarget::Capabilities => {
                        assert_eq!(episode.ready_result_version(), Some(result));
                    }
                }
                let expected_response = match case.target {
                    ControlTarget::Health => expected_health_ready(&original.health.id),
                    ControlTarget::Capabilities => {
                        expected_capabilities_ready(&original.capabilities.id)
                    }
                };
                assert!(result > begin);
                assert_eq!(run.head, result);
                assert!(!ready.has_unconfirmed_effect());
                assert_eq!(target.response_bytes(), Some(expected_response.as_slice()));
                break result;
            }
        }
        assert!(
            std::time::Instant::now() < ready_deadline,
            "TEST_CODE {} Ready checkpoint watchdog",
            case.label()
        );
        tokio::task::yield_now().await;
    };
    drop(prepared);
    drop(io);
    drop(local);
    drop(source);
    assert_eq!(clock.observation_calls.get(), 0);
    let fixed = fixed_snapshot(&database, &baseline.intent);
    assert_eq!(fixed.source_finals, 0);
    assert_eq!(fixed.data_begins, 0);
    match case.target {
        ControlTarget::Health => {
            control_tests::assert_response_raw(
                fixed.health_raw.as_ref().unwrap(),
                &expected_health_ready(&original.health.id),
            );
            assert_eq!(fixed.capabilities_raw, None);
            assert_eq!(fixed.control_results, 1);
        }
        ControlTarget::Capabilities => {
            assert_eq!(
                fixed.health_raw.as_deref(),
                Some(checkpoint.unwrap().raw.as_slice())
            );
            control_tests::assert_response_raw(
                fixed.capabilities_raw.as_ref().unwrap(),
                &expected_capabilities_ready(&original.capabilities.id),
            );
            assert_eq!(fixed.control_results, 2);
        }
    }
    assert_eq!(control_tests::audit_snapshot_at(&database), baseline.audit);
    (result_version, external.snapshot())
}

fn expected_capabilities_ready(request_id: &str) -> Vec<u8> {
    CapabilitiesResponse {
        request_id: request_id.to_owned(),
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
    .encode_to_vec()
}

async fn assert_unknown_after_reopen(
    business: &mut V2BusinessFixture,
    baseline: &control_tests::ExternalParentBaseline,
    external: &ExternalMtlsMacroFixture,
    checkpoint: Option<&control_recovery_tests::ConfirmedHealthCheckpoint>,
    case: Case,
    evidence: &FaultEvidence,
    before_reopen_wire: &ExternalControlObservation,
) {
    let database = business.database();
    let source =
        GrpcSource::from_external_macro_bundle_for_test(external.bundle_path().to_path_buf());
    let started_at = micros(STARTED_LOCAL);
    let now = started_at + case.reopen_offset();
    let clock = MacroClock {
        now: Cell::new(UtcMicros::try_new(now).unwrap()),
        observation: DateTime::parse_from_rfc3339("2026-09-14T15:34:00+08:00").unwrap(),
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
                &case.owner("REOPEN"),
                now,
                now + 2_000_000,
                evidence.target_head,
            ),
        )
        .unwrap();
    let reopened_head = local.inspect_run(&baseline.intent).unwrap().head_version();
    let reopened_generation = local
        .inspect_run(&baseline.intent)
        .unwrap()
        .lease_generation();
    assert!(reopened_head > evidence.target_head);
    assert!(reopened_generation > evidence.target_generation);
    let search_service =
        crate::search_service::SearchService::from_untyped_general_web_provider_for_test(
            GeneralWebResearchProvider::Tavily,
        );
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
    let stopped = tokio::time::timeout(
        Duration::from_secs(5),
        prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            baseline.stocks.clone(),
            None,
            &mut io,
        ),
    )
    .await
    .expect("TEST_CODE bounded Unknown reopen")
    .expect_err("TEST_CODE Unknown control must not be replayed");
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
    let failure = stopped.downcast_ref::<PreparationFailure>().unwrap();
    assert_eq!(failure.stage(), PreparationStage::Macro);
    drop(io);
    let reopened = local.inspect_macro(&baseline.intent).unwrap();
    assert_original(&reopened, &evidence.original);
    let begin = assert_target_pending(case, &reopened, checkpoint);
    assert_eq!(begin, evidence.target_begin);
    let reopened_run = local.inspect_run(&baseline.intent).unwrap();
    assert_eq!(reopened_run.head_version(), reopened_head);
    assert_eq!(reopened_run.lease_generation(), reopened_generation);
    assert_eq!(reopened_run.context().canonical_bytes(), baseline.context);
    drop(local);
    let reopened_owner: String = business
        .connection()
        .query_row(
            "SELECT lease_owner FROM chain_post_close_runs WHERE intent_id=?1",
            [baseline.intent.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(reopened_owner, case.owner("REOPEN"));
    drop(source);
    assert_eq!(clock.observation_calls.get(), 0);
    assert_eq!(&external.snapshot(), before_reopen_wire);
    assert_eq!(control_tests::audit_snapshot_at(&database), baseline.audit);
    assert_fixed_pending(
        case,
        &fixed_snapshot(&database, &baseline.intent),
        checkpoint,
    );
}

async fn run_case(case: Case) {
    let mut business = V2BusinessFixture::new();
    let mut parent_server = None;
    let mut external_server = None;
    let mut fault_reader: Option<Connection> = None;
    let body =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(120), async {
            let baseline = control_tests::setup_external_parent(
                &mut business,
                &mut parent_server,
                case.run_id(),
            )
            .await;
            external_server = Some(
                bind_case(case)
                    .await
                    .unwrap_or_else(|error| panic!("TEST_CODE {} fixture: {error}", case.label())),
            );
            let external = external_server.as_ref().unwrap();
            let database = business.database();
            let journal: String = business
                .connection()
                .query_row("PRAGMA journal_mode=DELETE", [], |row| row.get(0))
                .unwrap();
            assert_eq!(journal.to_ascii_lowercase(), "delete");
            let busy_ms: i64 = business
                .connection()
                .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
                .unwrap();
            assert_eq!(busy_ms, 250);

            let checkpoint = if case.target == ControlTarget::Capabilities {
                Some(
                    control_recovery_tests::reach_confirmed_health_checkpoint(
                        &mut business,
                        &baseline,
                        external,
                        &case.owner("HEALTH"),
                    )
                    .await,
                )
            } else {
                None
            };
            let target_base_head = checkpoint
                .as_ref()
                .map_or(baseline.head, |checkpoint| checkpoint.head_version);

            if case.fault == FaultPoint::BeginCommit {
                let (original, plan_head) = if case.target == ControlTarget::Health {
                    plan_health_without_begin_for_owner(
                        &mut business,
                        &baseline,
                        external,
                        &case.owner("PLAN"),
                    )
                } else {
                    let checkpoint = checkpoint.as_ref().unwrap();
                    let (recovery, _) = inspect_at(&database, &baseline.config, &baseline.intent);
                    (
                        capture_original(&recovery, &baseline, external),
                        checkpoint.head_version,
                    )
                };
                business.reopen();
                let (original, fault_head, fault_generation, fault_wire) =
                    exercise_begin_commit_failure(
                        &mut business,
                        &baseline,
                        external,
                        checkpoint.as_ref(),
                        case,
                        original,
                        plan_head,
                        &mut fault_reader,
                    )
                    .await;
                assert_eq!(fixed_snapshot(&database, &baseline.intent).source_finals, 0);
                assert_eq!(control_tests::audit_snapshot_at(&database), baseline.audit);
                business.reopen();
                let (result_version, ready_wire) = drive_original_control_to_ready(
                    &mut business,
                    &baseline,
                    external,
                    checkpoint.as_ref(),
                    case,
                    &original,
                    fault_head,
                    fault_generation,
                )
                .await;
                assert!(result_version > fault_head);
                match case.target {
                    ControlTarget::Health => {
                        assert_eq!(fault_wire, ExternalControlObservation::default());
                        assert_eq!(ready_wire.tcp_accepts, 1);
                        assert_eq!(ready_wire.health_requests, vec![original.health.bytes]);
                        assert_eq!(ready_wire.capabilities_calls, 0);
                        assert_eq!(ready_wire.data_calls, 0);
                    }
                    ControlTarget::Capabilities => {
                        assert_eq!(fault_wire.tcp_accepts, 1);
                        assert_eq!(fault_wire.health_requests.len(), 1);
                        assert_eq!(fault_wire.capabilities_calls, 0);
                        assert_eq!(ready_wire.tcp_accepts, 2);
                        assert_eq!(ready_wire.health_requests.len(), 1);
                        assert_eq!(
                            ready_wire.capabilities_requests,
                            vec![original.capabilities.bytes]
                        );
                        assert_eq!(ready_wire.data_calls, 0);
                    }
                }
                assert_parent_unchanged(&mut business, parent_server.as_ref().unwrap(), &baseline);
                release_all(external);
                return;
            }

            if case.target == ControlTarget::Capabilities {
                business.reopen();
            }
            let evidence = exercise_receipt_or_result_fault(
                &mut business,
                &baseline,
                external,
                checkpoint.as_ref(),
                case,
                target_base_head,
                &mut fault_reader,
            )
            .await;
            if let Some(error) = evidence.stop.as_ref() {
                assert_result_unconfirmed(error, &baseline.intent);
            } else {
                assert_eq!(case.fault, FaultPoint::ReceiptCancelled);
            }
            rollback_fault_reader(&mut fault_reader);
            let (unknown, run) = inspect_at(&database, &baseline.config, &baseline.intent);
            assert_original(&unknown, &evidence.original);
            assert_eq!(
                assert_target_pending(case, &unknown, checkpoint.as_ref()),
                evidence.target_begin
            );
            assert_eq!(run.head, evidence.target_head);
            assert_eq!(run.owner, case.owner("FAULT"));
            assert_eq!(run.generation, evidence.target_generation);
            assert_eq!(run.context, baseline.context);
            assert_eq!(control_tests::audit_snapshot_at(&database), baseline.audit);
            assert_fixed_pending(
                case,
                &fixed_snapshot(&database, &baseline.intent),
                checkpoint.as_ref(),
            );

            let released_wire =
                release_and_observe_fixed_reply(case, external, &evidence.original).await;
            assert_eq!(released_wire.tcp_accepts, evidence.wire.tcp_accepts);
            assert_eq!(released_wire.health_requests, evidence.wire.health_requests);
            assert_eq!(
                released_wire.capabilities_requests,
                evidence.wire.capabilities_requests
            );
            assert_eq!(released_wire.data_calls, 0);
            business.reopen();
            assert_unknown_after_reopen(
                &mut business,
                &baseline,
                external,
                checkpoint.as_ref(),
                case,
                &evidence,
                &released_wire,
            )
            .await;
            assert_parent_unchanged(&mut business, parent_server.as_ref().unwrap(), &baseline);
        }))
        .catch_unwind()
        .await;

    let reader_cleanup = fault_reader.take().map(|reader| {
        let rollback = if reader.is_autocommit() {
            Ok(())
        } else {
            reader.execute_batch("ROLLBACK;")
        };
        let close = reader.close().map_err(|(connection, error)| {
            drop(connection);
            error
        });
        (rollback, close)
    });
    let cleanup = std::panic::AssertUnwindSafe(control_tests::cleanup_external_case(
        &mut business,
        &mut parent_server,
        &mut external_server,
        case.label(),
    ))
    .catch_unwind()
    .await;
    if let Some((rollback, close)) = reader_cleanup {
        rollback.expect("TEST_CODE fault reader rollback");
        close.expect("TEST_CODE fault reader close");
    }
    cleanup.unwrap_or_else(|_| panic!("TEST_CODE {} cleanup panic", case.label()));
    match body {
        Ok(result) => result.unwrap_or_else(|_| panic!("TEST_CODE {} body timeout", case.label())),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test]
async fn single_user_external_macro_control_health_receipt_cancelled_reopens_unknown_without_rpc() {
    run_case(Case {
        target: ControlTarget::Health,
        fault: FaultPoint::ReceiptCancelled,
    })
    .await;
}

#[tokio::test]
async fn single_user_external_macro_control_capabilities_receipt_cancelled_reopens_unknown_without_rpc(
) {
    run_case(Case {
        target: ControlTarget::Capabilities,
        fault: FaultPoint::ReceiptCancelled,
    })
    .await;
}

#[tokio::test]
async fn single_user_external_macro_control_health_begin_commit_failure_retries_original_request() {
    run_case(Case {
        target: ControlTarget::Health,
        fault: FaultPoint::BeginCommit,
    })
    .await;
}

#[tokio::test]
async fn single_user_external_macro_control_capabilities_begin_commit_failure_retries_original_request(
) {
    run_case(Case {
        target: ControlTarget::Capabilities,
        fault: FaultPoint::BeginCommit,
    })
    .await;
}

#[tokio::test]
async fn single_user_external_macro_control_health_result_commit_failure_reopens_unknown_without_rpc(
) {
    run_case(Case {
        target: ControlTarget::Health,
        fault: FaultPoint::ResultCommit,
    })
    .await;
}

#[tokio::test]
async fn single_user_external_macro_control_capabilities_result_commit_failure_reopens_unknown_without_rpc(
) {
    run_case(Case {
        target: ControlTarget::Capabilities,
        fault: FaultPoint::ResultCommit,
    })
    .await;
}
