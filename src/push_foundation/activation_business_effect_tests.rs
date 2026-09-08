use super::super::activation_fence::{
    EffectBroker, EffectResult, OperationFact, OperationState, TestHooks,
};
use super::super::activation_fence_tests::{fixture_clients, fixture_scope};
use super::super::activation_generic_effect::GenericEffectFixture;
use super::super::activation_generic_effect_tests::{claimed_fixture_at, micros};
use super::super::{
    IntentState, IntentTransitionCommand, LeaseAction, LeaseOwnerId, TransitionActor,
};
use super::*;
use crate::durable_delivery::{
    AuthoritativeDeliveryRequest, AuthoritativeSinkPort, AuthoritativeSinkResult,
    CoordinatorConfig, ImmutableAppendPort, TypedReceipt, TypedRejection, TypedUncertainty,
};
use crate::monitor::push_job::{raw_digest, w09_completion_policy_fixture, ReasonCode};
use chrono::{TimeZone, Utc};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const NOW: i64 = 1_788_743_104_000_000;
const UNTIL: i64 = 1_788_743_400_000_000;

struct LocalSink {
    calls: AtomicUsize,
    disposition: u8,
}
impl AuthoritativeSinkPort for LocalSink {
    fn sink_identity(&self) -> &str {
        "TEST_CODE_W16_CHANNEL"
    }
    fn deliver(&self, _: &AuthoritativeDeliveryRequest) -> AuthoritativeSinkResult {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let at = Utc.timestamp_micros(NOW - 2_000_000).single().unwrap();
        match self.disposition {
            1 => AuthoritativeSinkResult::Rejected(TypedRejection {
                reason_code: "TEST_CODE_rejected".into(),
                evidence: b"actual local refusal".to_vec(),
                retry_authorized: false,
                observed_at: at,
            }),
            2 => AuthoritativeSinkResult::Uncertain(TypedUncertainty {
                reason_code: "TEST_CODE_uncertain".into(),
                evidence: b"actual local uncertainty".to_vec(),
                observed_at: at,
            }),
            _ => AuthoritativeSinkResult::Accepted(TypedReceipt {
                channel: self.sink_identity().into(),
                provider: "TEST_CODE_LOCAL".into(),
                message_id: "TEST_CODE_ACTUAL".into(),
                platform_message_id: None,
                accepted_at: at,
                latency_ms: None,
            }),
        }
    }
}

#[derive(Default)]
struct LocalAppend {
    calls: AtomicUsize,
    fail: AtomicBool,
}
impl ImmutableAppendPort for LocalAppend {
    fn append_exact(
        &self,
        kind: &str,
        identity: &str,
        _: &[u8],
        _: &str,
    ) -> crate::durable_delivery::Result<String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail.load(Ordering::SeqCst) {
            return Err(
                crate::durable_delivery::DurableDeliveryError::ImmutableAppendConflict(
                    "TEST_CODE_BLOCKED".into(),
                ),
            );
        }
        Ok(format!("immutable://{kind}/{identity}"))
    }
}

struct Fixture {
    temporary: tempfile::TempDir,
    _durable_root: tempfile::TempDir,
    database: PathBuf,
    durable_database: PathBuf,
    control: PathBuf,
    code: String,
    coordinator: Arc<DurableDeliveryCoordinator>,
    sink: Arc<LocalSink>,
    append: Arc<LocalAppend>,
    initial: IntentSnapshot,
    template: TerminalTemplateBinding,
}

fn config(owner: &str, now: i64, until: i64, page: usize, iterations: usize) -> RecoveryConfig {
    RecoveryConfig::try_new(
        LeaseOwnerId::try_new(owner.into()).unwrap(),
        TransitionActor::try_new("w16-recovery".into()).unwrap(),
        micros(now),
        micros(until),
        page,
        iterations,
    )
    .unwrap()
}

impl Fixture {
    fn new(disposition: u8) -> Self {
        std::fs::create_dir_all("data/test").unwrap();
        let durable_root = tempfile::Builder::new()
            .prefix("TEST_CODE_W16_BUSINESS_")
            .tempdir_in("data/test")
            .unwrap();
        let code = durable_root
            .path()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        let durable_database = PathBuf::from("data/test")
            .join(&code)
            .join("durable_delivery.sqlite3");
        let coordinator = Arc::new(
            DurableDeliveryCoordinator::open(CoordinatorConfig::test(
                &durable_database,
                &code,
                format!("owner-{code}"),
            ))
            .unwrap(),
        );
        let temporary = tempfile::Builder::new()
            .prefix("TEST_CODE_W16_BUSINESS_")
            .tempdir_in("/private/tmp")
            .unwrap();
        let database = temporary.path().join("business.sqlite3");
        let control = temporary.path().join("control.sqlite3");
        let (initial, route, _, _) = claimed_fixture_at(&database, &code);
        Self {
            temporary,
            _durable_root: durable_root,
            database,
            durable_database,
            control,
            code,
            coordinator,
            sink: Arc::new(LocalSink {
                calls: AtomicUsize::new(0),
                disposition,
            }),
            append: Arc::new(LocalAppend::default()),
            initial,
            template: route.template().clone(),
        }
    }
    fn scope(&self) -> Scope {
        Scope {
            namespace: format!("Test:{}", self.code),
            unit: "MU-auction".into(),
            ..fixture_scope()
        }
    }
    fn snapshot(&self) -> IntentSnapshot {
        BusinessIntentStore::open(&self.database)
            .unwrap()
            .inspect(&self.initial.attested_ready_binding().unwrap().intent_id)
            .unwrap()
            .unwrap()
    }
    fn effect(&self) -> BusinessEffectFixture {
        BusinessEffectFixture {
            database: self.database.clone(),
            coordinator: Arc::clone(&self.coordinator),
            snapshot: self.snapshot(),
            template: self.template.clone(),
            completion_policy: w09_completion_policy_fixture(
                "MU-auction",
                "owner-auction",
                vec![AuthorityClass::GenericCounted],
            ),
            config: config("w16-dispatcher", NOW, UNTIL, 10, 5),
        }
    }
    fn broker(&self, epoch: &str, hooks: TestHooks) -> EffectBroker {
        EffectBroker::test_business_fixture(
            &self.control,
            self.scope(),
            epoch.into(),
            self.effect(),
            fixture_clients(0, 0),
            hooks,
        )
        .unwrap()
    }
    fn counts(&self) -> (Vec<u64>, usize, usize) {
        let db = rusqlite::Connection::open(&self.durable_database).unwrap();
        let rows = [
            "delivery_decisions",
            "delivery_attempts",
            "sink_results",
            "immutable_audit_outbox",
            "delivery_disposition_payloads",
        ]
        .into_iter()
        .map(|table| {
            db.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
        })
        .collect();
        (
            rows,
            self.sink.calls.load(Ordering::SeqCst),
            self.append.calls.load(Ordering::SeqCst),
        )
    }
    async fn send(&self) {
        let (snapshot, route, fence, completion_policy) =
            claimed_fixture_at(&self.database, &self.code);
        let broker = EffectBroker::test_generic_fixture(
            &self.temporary.path().join("generic-control.sqlite3"),
            self.scope(),
            "generic".into(),
            GenericEffectFixture {
                database: self.database.clone(),
                coordinator: Arc::clone(&self.coordinator),
                snapshot,
                route,
                fence,
                completion_policy,
                sink: self.sink.clone(),
                append_port: self.append.clone(),
                dispatched_at: micros(NOW - 2_000_000),
                verified_at: micros(NOW - 1_000_000),
            },
            fixture_clients(0, 0),
            TestHooks::default(),
        )
        .unwrap();
        let request = broker.test_generic_request("actual-send", WorkClass::NewWork);
        broker.execute_current(request.clone()).unwrap();
        let fact = finish(&broker, &request).await;
        assert_eq!(
            fact.state,
            if self.sink.disposition == 2 || self.append.fail.load(Ordering::SeqCst) {
                OperationState::Unresolved
            } else {
                OperationState::Succeeded
            }
        );
    }
}

async fn finish(broker: &EffectBroker, request: &EffectRequest) -> OperationFact {
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            let fact = broker.query_bounded(request).await.unwrap().unwrap();
            if fact.state != OperationState::Running {
                break fact;
            }
        }
    })
    .await
    .unwrap()
}

fn result(fact: &OperationFact) -> &BusinessRecoveryResult {
    let EffectResult::BusinessRecovery(result) = fact.result.as_ref().unwrap() else {
        panic!("business result required")
    };
    result
}

fn assert_proof(fixture: &Fixture, request: &EffectRequest, fact: &OperationFact) {
    let result = result(fact);
    let expected = serde_json::json!({
        "operation_id": request.operation_id, "original_epoch": request.broker_epoch, "request_sha256": request.digest(), "state": "Succeeded",
        "result": { "intent_id": result.intent_id, "decision_id":result.decision_id, "effect_sha256":result.effect_sha256, "state":result.state,
        "version": result.version, "lease_generation":result.lease_generation, "snapshot_sha256":result.snapshot_sha256, "transition_chain_sha256":result.transition_chain_sha256,
        "transition_count":result.transition_count, "last_event_id":result.last_event_id, "last_event_sha256":result.last_event_sha256,"recovery_boundary":result.recovery_boundary }
    });
    let expected = [
        b"ActivationBusinessRecoveryWorkerCompletion/v1\0".as_slice(),
        serde_json::to_vec(&expected).unwrap().as_slice(),
    ]
    .concat();
    let db = rusqlite::Connection::open(&fixture.control).unwrap();
    let (bytes, digest): (Vec<u8>, String) = db.query_row("SELECT completion_bytes,completion_sha256 FROM effect_worker_completions WHERE operation_id=?", [&request.operation_id], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert_eq!(bytes, expected);
    assert_eq!(digest, raw_digest(&expected).as_str());
}

#[tokio::test]
async fn actual_accepted_completes_once_replays_and_confirms_completed_without_reclaim() {
    let fixture = Fixture::new(0);
    fixture.send().await;
    let before = fixture.counts();
    let broker = fixture.broker("first", TestHooks::default());
    broker
        .quiesce(&fixture.scope(), "first", WorkClass::NewWork)
        .unwrap();
    let request = broker.test_business_request("one");
    broker.execute_current(request.clone()).unwrap();
    let fact = finish(&broker, &request).await;
    assert_eq!(fact.state, OperationState::Succeeded);
    assert_eq!(result(&fact).state, "Completed");
    assert_eq!(result(&fact).version, 3);
    assert_eq!(result(&fact).lease_generation, 1);
    assert_proof(&fixture, &request, &fact);
    let completed = fixture.snapshot();
    assert_eq!(completed.state(), IntentState::Completed);
    assert!(completed.lease_owner().is_none());
    assert_eq!(broker.execute_current(request.clone()).unwrap(), fact);
    assert_eq!(fixture.snapshot(), completed);
    assert_eq!(fixture.counts(), before);
    drop(broker);
    let restarted = fixture.broker("second", TestHooks::default());
    assert_eq!(restarted.query_operation(&request).unwrap(), Some(fact));
    let new_control = fixture.temporary.path().join("readonly-control.sqlite3");
    let readonly = EffectBroker::test_business_fixture(
        &new_control,
        fixture.scope(),
        "third".into(),
        fixture.effect(),
        fixture_clients(0, 0),
        TestHooks::default(),
    )
    .unwrap();
    let read = readonly.test_business_request("completed-read");
    readonly.execute_current(read.clone()).unwrap();
    assert_eq!(
        finish(&readonly, &read).await.state,
        OperationState::Succeeded
    );
    assert_eq!(fixture.snapshot(), completed);
    assert_eq!(fixture.counts(), before);
}

#[tokio::test]
async fn all_request_scope_identity_action_and_class_mutations_refuse_before_business() {
    let fixture = Fixture::new(0);
    let broker = fixture.broker("first", TestHooks::default());
    let original = broker.test_business_request("one");
    let before = fixture.counts();
    for index in 0..16 {
        let mut bad = original.clone();
        match index {
            0 => bad.scope.namespace.push('x'),
            1 => bad.scope.unit.push('x'),
            2 => bad.scope.generation += 1,
            3 => bad.scope.manifest.push('x'),
            4 => bad.scope.physical_owner.push('x'),
            5 => bad.scope.deployment.push('x'),
            6 => bad.scope.incarnation.push('x'),
            7 => bad.broker_epoch.push('x'),
            8 => bad.client.push('x'),
            9 => bad.client_incarnation.push('x'),
            10 => bad.actor = "Dispatcher".into(),
            11 => bad.action = "ReconcileGeneric".into(),
            12 => bad.work_class = WorkClass::NewWork,
            13 => bad.effect_id = "generic-reconcile".into(),
            14 => bad.effect_sha256.push('x'),
            15 => bad.operation_id.clear(),
            _ => unreachable!(),
        }
        assert!(broker.execute_current(bad).is_err(), "mutation {index}");
        assert_eq!(fixture.snapshot(), fixture.initial);
        assert_eq!(fixture.counts(), before);
    }
    broker
        .quiesce(&fixture.scope(), "first", WorkClass::Recovery)
        .unwrap();
    assert_eq!(broker.execute_current(original), Err(FenceError::Closed));
    assert_eq!(fixture.snapshot(), fixture.initial);
}

#[test]
fn registration_rejects_scope_template_policy_and_actual_database_mismatches() {
    let fixture = Fixture::new(0);
    let other = Fixture::new(0);
    for index in 0..7 {
        let mut scope = fixture.scope();
        let mut source = fixture.effect();
        match index {
            0 => scope.namespace = "Prod".into(),
            1 => scope.namespace.push('x'),
            2 => scope.unit.push('x'),
            3 => {
                source.template = TerminalTemplateBinding::new(
                    crate::monitor::push_job::TemplateId::try_new("other".into()).unwrap(),
                    crate::monitor::push_job::TemplateVersion::try_new("v1".into()).unwrap(),
                )
            }
            4 => {
                source.completion_policy = w09_completion_policy_fixture(
                    "other",
                    "owner-auction",
                    vec![AuthorityClass::GenericCounted],
                )
            }
            5 => source.database = other.database.clone(),
            6 => source.coordinator = Arc::clone(&other.coordinator),
            _ => unreachable!(),
        }
        assert!(
            BusinessEffect::bind_fixture(&scope, source).is_err(),
            "mutation {index}"
        );
        assert_eq!(fixture.snapshot(), fixture.initial);
    }
}

fn renew(fixture: &Fixture, at: i64) {
    let current = fixture.snapshot();
    let mut store = BusinessIntentStore::open(&fixture.database).unwrap();
    store
        .apply_nonterminal_transition(
            &IntentTransitionCommand::try_new(
                current.attested_ready_binding().unwrap().intent_id,
                current.state(),
                current.state(),
                current.version(),
                TransitionActor::try_new("competing-renewal".into()).unwrap(),
                ReasonCode::IntentDispatchClaimed,
                micros(at),
                LeaseAction::Acquire {
                    owner: LeaseOwnerId::try_new("w16-dispatcher".into()).unwrap(),
                    until: micros(UNTIL + 1),
                },
            )
            .unwrap(),
        )
        .unwrap();
}

#[tokio::test]
async fn initial_snapshot_drift_and_replaced_business_inode_refuse_without_recovery() {
    for replace in [false, true] {
        let fixture = Fixture::new(0);
        let broker = fixture.broker("first", TestHooks::default());
        let before = fixture.counts();
        if replace {
            let original = fixture.temporary.path().join("old-business.sqlite3");
            std::fs::rename(&fixture.database, &original).unwrap();
            std::fs::copy(&original, &fixture.database).unwrap();
        } else {
            renew(&fixture, NOW - 1);
        }
        let changed = fixture.snapshot();
        let request = broker.test_business_request("one");
        broker.execute_current(request.clone()).unwrap();
        assert_eq!(
            finish(&broker, &request).await.state,
            OperationState::Unresolved
        );
        assert_eq!(fixture.snapshot(), changed);
        assert_eq!(fixture.counts(), before);
    }
}

#[tokio::test]
async fn rejected_uncertain_missing_and_pending_terminal_keep_recovery_boundary_without_send() {
    for case in 0..4 {
        let fixture = Fixture::new(if case < 2 { case + 1 } else { 0 });
        if case == 3 {
            fixture.append.fail.store(true, Ordering::SeqCst);
        }
        if case != 2 {
            fixture.send().await;
        }
        let before = fixture.counts();
        let broker = fixture.broker("first", TestHooks::default());
        let request = broker.test_business_request("one");
        broker.execute_current(request.clone()).unwrap();
        let fact = finish(&broker, &request).await;
        assert_eq!(fact.state, OperationState::Unresolved);
        assert_eq!(
            result(&fact).recovery_boundary,
            match case {
                0 => "RejectedAuthorizationRequired",
                1 => "ManualResolutionRequired",
                _ => "AuthorityBlocked",
            }
        );
        assert_eq!(fixture.snapshot().version(), 2);
        assert_eq!(
            fixture.snapshot().state(),
            if case == 1 {
                IntentState::ResolutionRequired
            } else {
                IntentState::AwaitingAuthority
            }
        );
        let after = fixture.snapshot();
        assert_eq!(broker.execute_current(request.clone()).unwrap(), fact);
        assert_eq!(fixture.snapshot(), after);
        assert_eq!(fixture.counts(), before);
        drop(broker);
        let restarted = fixture.broker("second", TestHooks::default());
        assert_eq!(restarted.query_operation(&request).unwrap(), Some(fact));
        assert_eq!(fixture.snapshot(), after);
    }
}

#[tokio::test]
async fn live_foreign_lease_never_queries_or_writes() {
    let fixture = Fixture::new(0);
    let mut source = fixture.effect();
    source.config = config("foreign-worker", NOW, UNTIL, 10, 5);
    let broker = EffectBroker::test_business_fixture(
        &fixture.control,
        fixture.scope(),
        "first".into(),
        source,
        fixture_clients(0, 0),
        TestHooks::default(),
    )
    .unwrap();
    let request = broker.test_business_request("one");
    broker.execute_current(request.clone()).unwrap();
    let fact = finish(&broker, &request).await;
    assert_eq!(fact.state, OperationState::Unresolved);
    assert_eq!(result(&fact).recovery_boundary, "LiveForeignLease");
    assert_eq!(fixture.snapshot(), fixture.initial);
    assert_eq!(fixture.sink.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn actual_qualification_query_failure_and_cas_conflict_stay_guarded() {
    for conflict in [false, true] {
        let fixture = Fixture::new(0);
        fixture.send().await;
        let before = fixture.counts();
        let pause = fixture.temporary.path().join("pause.sock");
        let listener = tokio::net::UnixListener::bind(&pause).unwrap();
        let broker = fixture.broker(
            "first",
            TestHooks {
                business_requery_pause_socket: Some(pause),
                ..TestHooks::default()
            },
        );
        let request = broker.test_business_request("one");
        broker.execute_current(request.clone()).unwrap();
        let (mut stream, _) =
            tokio::time::timeout(std::time::Duration::from_secs(20), listener.accept())
                .await
                .unwrap()
                .unwrap();
        let mut signal = [0];
        stream.read_exact(&mut signal).await.unwrap();
        assert_eq!(signal, *b"P");
        assert_eq!(fixture.snapshot().state(), IntentState::AwaitingFinalizer);
        assert_eq!(fixture.snapshot().version(), 2);
        broker
            .quiesce(&fixture.scope(), "first", WorkClass::NewWork)
            .unwrap();
        assert!(
            !broker
                .quiesce(&fixture.scope(), "first", WorkClass::Recovery)
                .unwrap()
                .drained
        );
        if conflict {
            renew(&fixture, NOW);
        }
        stream
            .write_all(if conflict { b"G" } else { b"X" })
            .await
            .unwrap();
        drop(stream);
        let fact = finish(&broker, &request).await;
        assert_eq!(fact.state, OperationState::Unresolved);
        assert_eq!(
            fixture.snapshot().state(),
            if conflict {
                IntentState::ResolutionRequired
            } else {
                IntentState::AwaitingFinalizer
            }
        );
        assert_eq!(
            fixture.snapshot().reason(),
            if conflict {
                ReasonCode::FinalizerCasConflict
            } else {
                ReasonCode::FinalizerTerminalRefInvalid
            }
        );
        assert_eq!(fixture.counts(), before);
    }
}

#[tokio::test]
async fn business_commit_confirmation_and_worker_proof_failures_never_upgrade_on_restart() {
    for fault in 0..4 {
        let fixture = Fixture::new(0);
        fixture.send().await;
        let before = fixture.counts();
        let broker = fixture.broker(
            "first",
            TestHooks {
                business_commit_ack_lost: fault == 0,
                result_confirmation_lost: fault == 1,
                final_result_read_failure: fault == 2,
                completion_write_ack_lost: fault == 3,
                ..TestHooks::default()
            },
        );
        let request = broker.test_business_request("one");
        broker.execute_current(request.clone()).unwrap();
        let fact = finish(&broker, &request).await;
        assert_eq!(fixture.snapshot().state(), IntentState::Completed);
        assert_eq!(
            fact.state,
            if fault == 3 {
                OperationState::Succeeded
            } else {
                OperationState::Unresolved
            }
        );
        if fault == 3 {
            assert_proof(&fixture, &request, &fact);
        }
        drop(broker);
        let restarted = fixture.broker("second", TestHooks::default());
        assert_eq!(restarted.query_operation(&request).unwrap(), Some(fact));
        assert_eq!(fixture.counts(), before);
    }
}

#[tokio::test]
async fn result_schema_requires_every_field_rejects_mixed_shapes_and_proof_tampering() {
    let fixture = Fixture::new(0);
    fixture.send().await;
    let broker = fixture.broker("first", TestHooks::default());
    let request = broker.test_business_request("one");
    broker.execute_current(request.clone()).unwrap();
    let fact = finish(&broker, &request).await;
    let encoded = serde_json::to_value(fact.result.as_ref().unwrap()).unwrap();
    assert_eq!(encoded["kind"], "BusinessRecovery");
    for field in encoded["result"].as_object().unwrap().keys() {
        let mut missing = encoded.clone();
        missing["result"].as_object_mut().unwrap().remove(field);
        assert!(
            serde_json::from_value::<EffectResult>(missing).is_err(),
            "required {field}"
        );
    }
    for kind in ["InitialIntent", "GenericTransport", "Unknown", ""] {
        let mut mixed = encoded.clone();
        mixed["kind"] = kind.into();
        assert!(serde_json::from_value::<EffectResult>(mixed).is_err());
    }
    for extra in ["initial_intent_sha256", "terminal_ref", "unknown"] {
        let mut mixed = encoded.clone();
        mixed["result"][extra] = "x".into();
        assert!(serde_json::from_value::<EffectResult>(mixed).is_err());
    }
    for field in [
        "state",
        "recovery_boundary",
        "lease_generation",
        "last_event_id",
        "decision_id",
        "effect_sha256",
    ] {
        let mut bad = result(&fact).clone();
        match field {
            "state" => bad.state = "ResolutionRequired".into(),
            "recovery_boundary" => bad.recovery_boundary = "ManualResolutionRequired".into(),
            "lease_generation" => bad.lease_generation = bad.version + 1,
            "last_event_id" => bad.last_event_id = Some("invalid".into()),
            "decision_id" => bad.decision_id = "0".repeat(64),
            "effect_sha256" => bad.effect_sha256 = "0".repeat(64),
            _ => unreachable!(),
        }
        assert!(bad.validate(&request).is_err(), "invalid {field}");
    }
    let db = rusqlite::Connection::open(&fixture.control).unwrap();
    let original_json: String = db
        .query_row(
            "SELECT result_json FROM effect_operations WHERE operation_id='one'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    for field in encoded["result"].as_object().unwrap().keys() {
        let mut tampered = encoded.clone();
        tampered["result"][field] = match tampered["result"][field].as_u64() {
            Some(value) => (value + 1).into(),
            None => "e".repeat(64).into(),
        };
        db.execute(
            "UPDATE effect_operations SET result_json=? WHERE operation_id='one'",
            [serde_json::to_string(&tampered).unwrap()],
        )
        .unwrap();
        assert!(
            !matches!(
                broker.query_operation(&request),
                Ok(Some(OperationFact {
                    state: OperationState::Succeeded,
                    ..
                }))
            ),
            "tampered {field}"
        );
    }
    db.execute(
        "UPDATE effect_operations SET result_json=? WHERE operation_id='one'",
        [original_json],
    )
    .unwrap();
    let (proof, sha): (Vec<u8>, String) = db.query_row("SELECT completion_bytes,completion_sha256 FROM effect_worker_completions WHERE operation_id='one'", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    for domain in [
        "ActivationEffectWorkerCompletion/v1",
        "ActivationGenericTransportWorkerCompletion/v1",
        "ActivationBusinessRecoveryEffect/v1",
    ] {
        let json = proof.splitn(2, |byte| *byte == 0).nth(1).unwrap();
        let wrong = [domain.as_bytes(), &[0], json].concat();
        db.execute("UPDATE effect_worker_completions SET completion_bytes=?,completion_sha256=? WHERE operation_id='one'", rusqlite::params![wrong, raw_digest(&wrong).as_str()]).unwrap();
        assert!(!matches!(
            broker.query_operation(&request),
            Ok(Some(OperationFact {
                state: OperationState::Succeeded,
                ..
            }))
        ));
    }
    db.execute("UPDATE effect_worker_completions SET completion_bytes=?,completion_sha256=? WHERE operation_id='one'", rusqlite::params![proof, sha]).unwrap();
    assert_eq!(broker.query_operation(&request).unwrap(), Some(fact));
}

#[test]
fn effect_has_independent_full_literal_golden_and_actual_encoder_variants() {
    use crate::monitor::push_job::{TemplateId, TemplateVersion};
    let fixture = Fixture::new(0);
    let source = fixture.effect();
    let attested = source.snapshot.attested_ready_binding().unwrap();
    let database = std::fs::canonicalize(&fixture.database).unwrap();
    let metadata = std::fs::metadata(&database).unwrap();
    let durable = std::fs::metadata(&fixture.durable_database).unwrap();
    let template_hash = raw_digest(b"TemplateBinding/v1\0{\"template_id\":\"auction-card\",\"template_version\":\"auction-card-v3\"}");
    let policy = concat!(
        "ActivationCompletionPolicy/v1\0{\"advance_event\":\"AcceptedOrManualBound\",\"allowed_authority\":[\"GenericCounted\"],\"already_terminal_policy\":\"RequeryExactBinding\",",
        "\"completion_owner\":{\"catalog_sha256\":\"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd\",\"completion_owner\":\"owner-auction\",\"unit_id\":\"MU-auction\"},",
        "\"disabled_policy\":\"CloseDisabledOccurrence\",\"finalizer_kind\":\"BoundCursor\",\"id\":\"fixture-policy\",\"no_data_policy\":\"CloseVerifiedOccurrence\",",
        "\"notification_cursor_policy\":\"AcceptedBoundOnly\",\"retention_class\":\"Trading\",\"retry_policy\":{\"kind\":\"Never\",\"max_attempts\":null,\"not_before\":null},",
        "\"schedule_close_policy\":[\"ExplicitDisabled\",\"OnAccepted\",\"SuppressedOccurrence\",\"VerifiedNoData\"],\"uncertain_manual_policy\":\"QuarantineThenVerifiedManual\",\"version\":\"v1\"}"
    );
    assert_eq!(
        source.completion_policy.activation_binding_bytes(),
        policy.as_bytes()
    );
    let snapshot = serde_json::json!({
        "audience":"portfolio-owner","business_date":"2026-09-07","completion_owner":"owner-auction","created_at":"1788743100000000",
        "decision_id":attested.decision_id.as_str(),"decision_kind":"Ready","evidence_sha256":"b".repeat(64),"intent_id":attested.intent_id.as_str(),
        "lease_generation":1,"lease_owner":"w16-dispatcher","lease_until":"1788743400000000","namespace":format!("Test:{}",fixture.code),
        "occurrence_family":"auction-session","occurrence_key":"main","payload_sha256":raw_digest(source.snapshot.prepared_push_bytes().unwrap()).as_str(),
        "prepared_push_bytes":source.snapshot.prepared_push_bytes().unwrap(),"previous_state":"PendingDispatch","reason":"intent.dispatch_claimed",
        "rendered_bytes":b"first render  \nline two!".to_vec(),"rendered_sha256":raw_digest(b"first render  \nline two!").as_str(),
        "source_contract_id":"auction-source","source_contract_sha256":"f".repeat(64),"state":"AwaitingAuthority","subject":"Entity:000001.SZ",
        "template_sha256":template_hash.as_str(),"unit_id":"MU-auction","updated_at":"1788743101000000","version":1
    });
    let expected = serde_json::json!({
        "action":"ReconcileBusiness","actor":"Finalizer","business_store_device":metadata.dev(),"business_store_inode":metadata.ino(),"business_store_path":database.to_str().unwrap(),
        "completion_policy_bytes":policy.as_bytes(),"deployment":"fixture-deployment","durable_environment":format!("Test:{}",fixture.code),"durable_owner":format!("owner-{}",fixture.code),
        "durable_store_device":durable.dev(),"durable_store_inode":durable.ino(),"durable_store_path":fixture.durable_database.to_str().unwrap(),
        "effect_id":"business-reconcile","generation":11,"incarnation":"deployment-one","manifest":"a".repeat(64),"namespace":format!("Test:{}",fixture.code),"physical_owner":"generic-owner",
        "recovery_config":{"actor":"w16-recovery","lease_until":"1788743400000000","max_iterations":5,"now":"1788743104000000","owner":"w16-dispatcher","page_size":10},
        "snapshot":snapshot,"template_id":"auction-card","template_version":"auction-card-v3","template_sha256":template_hash.as_str(),"unit":"MU-auction","work_class":"Recovery"
    });
    let expected = [
        b"ActivationBusinessRecoveryEffect/v1\0".as_slice(),
        serde_json::to_vec(&expected).unwrap().as_slice(),
    ]
    .concat();
    let effect = BusinessEffect::bind_fixture(&fixture.scope(), source).unwrap();
    assert_eq!(effect.canonical_bytes().unwrap(), expected);
    assert_eq!(effect.digest(), raw_digest(&expected).as_str());
    for index in 0..23 {
        let mut changed = BusinessEffect::bind_fixture(&fixture.scope(), fixture.effect()).unwrap();
        match index {
            0 => changed.scope.namespace.push('x'),
            1 => changed.scope.unit.push('x'),
            2 => changed.scope.generation += 1,
            3 => changed.scope.manifest.push('x'),
            4 => changed.scope.physical_owner.push('x'),
            5 => changed.scope.deployment.push('x'),
            6 => changed.scope.incarnation.push('x'),
            7 => changed.database = changed.database.with_extension("other"),
            8 => changed.business_device += 1,
            9 => changed.business_inode += 1,
            10 => changed.durable_binding.0.push('x'),
            11 => changed.durable_binding.1 += 1,
            12 => changed.durable_binding.2 += 1,
            13 => changed.durable_binding.3.push('x'),
            14 => changed.durable_binding.4.push('x'),
            15 => {
                changed.template = TerminalTemplateBinding::new(
                    TemplateId::try_new("different".into()).unwrap(),
                    TemplateVersion::try_new("auction-card-v3".into()).unwrap(),
                )
            }
            16 => {
                changed.template = TerminalTemplateBinding::new(
                    TemplateId::try_new("auction-card".into()).unwrap(),
                    TemplateVersion::try_new("v4".into()).unwrap(),
                )
            }
            17 => {
                changed.completion_policy = w09_completion_policy_fixture(
                    "MU-auction",
                    "owner-auction",
                    vec![AuthorityClass::GenericCounted, AuthorityClass::P01Dedicated],
                )
            }
            18 => changed.config = config("other-owner", NOW, UNTIL, 10, 5),
            19 => changed.config = config("w16-dispatcher", NOW + 1, UNTIL, 10, 5),
            20 => changed.config = config("w16-dispatcher", NOW, UNTIL + 1, 10, 5),
            21 => changed.config = config("w16-dispatcher", NOW, UNTIL, 11, 5),
            22 => changed.config = config("w16-dispatcher", NOW, UNTIL, 10, 6),
            _ => unreachable!(),
        }
        assert_ne!(
            changed.canonical_bytes().unwrap(),
            expected,
            "actual encoder mutation {index}"
        );
    }
    let mut changed = BusinessEffect::bind_fixture(&fixture.scope(), fixture.effect()).unwrap();
    changed.config = RecoveryConfig::try_new(
        LeaseOwnerId::try_new("w16-dispatcher".into()).unwrap(),
        TransitionActor::try_new("other-actor".into()).unwrap(),
        micros(NOW),
        micros(UNTIL),
        10,
        5,
    )
    .unwrap();
    assert_ne!(changed.canonical_bytes().unwrap(), expected);
    renew(&fixture, NOW - 1);
    changed.snapshot = fixture.snapshot();
    assert_ne!(changed.canonical_bytes().unwrap(), expected);
}

fn add_pending(fixture: &Fixture) -> IntentSnapshot {
    use super::super::{InitialIntentDraft, InitialIntentIdentity};
    use crate::monitor::push_job::{
        w08_prepared_push_fixture_for_namespace, AudienceId, BusinessDate, CompletionOwnerId,
        Namespace, OccurrenceFamily, OccurrenceIdentityMaterial, OccurrenceKey, RunId,
        SourceContractId, SubjectId, UnitId,
    };
    let namespace = Namespace::test(RunId::try_new(fixture.code.clone()).unwrap());
    let prepared = w08_prepared_push_fixture_for_namespace(namespace.clone());
    let draft = InitialIntentDraft::ready(
        InitialIntentIdentity::new(
            namespace,
            UnitId::try_new("MU-auction".into()).unwrap(),
            OccurrenceIdentityMaterial::new(
                BusinessDate::parse("2026-09-07").unwrap(),
                OccurrenceFamily::try_new("auction-session".into()).unwrap(),
                OccurrenceKey::try_new("other-pending".into()).unwrap(),
            ),
            CompletionOwnerId::try_new("owner-auction".into()).unwrap(),
            SourceContractId::try_new("auction-source".into()).unwrap(),
            SubjectId::entity("000002.SZ".into()).unwrap(),
            AudienceId::try_new("portfolio-owner".into()).unwrap(),
        ),
        &prepared,
        fixture.template.sha256().clone(),
        Sha256Digest::parse("fixture", &"f".repeat(64)).unwrap(),
        micros(NOW - 4_000_000),
    )
    .unwrap();
    let mut store = BusinessIntentStore::open(&fixture.database).unwrap();
    store.record_initial(&draft).unwrap();
    store.inspect(draft.intent_id()).unwrap().unwrap()
}

#[tokio::test]
async fn exact_recovery_leaves_other_pending_intent_and_entire_chain_unchanged() {
    let fixture = Fixture::new(0);
    fixture.send().await;
    let other = add_pending(&fixture);
    let other_id = other.attested_ready_binding().unwrap().intent_id;
    let before = fixture.counts();
    let store = BusinessIntentStore::open(&fixture.database).unwrap();
    let chain = store.inspect_transition_chain(&other_id).unwrap();
    let broker = fixture.broker("first", TestHooks::default());
    let request = broker.test_business_request("one");
    broker.execute_current(request.clone()).unwrap();
    assert_eq!(
        finish(&broker, &request).await.state,
        OperationState::Succeeded
    );
    assert_eq!(store.inspect(&other_id).unwrap(), Some(other));
    assert_eq!(store.inspect_transition_chain(&other_id).unwrap(), chain);
    assert_eq!(fixture.counts(), before);
}

#[tokio::test]
async fn exact_pending_intent_only_acquires_recovery_lease_and_never_dispatches() {
    let fixture = Fixture::new(0);
    let pending = add_pending(&fixture);
    let target = pending.attested_ready_binding().unwrap().intent_id;
    let mut source = fixture.effect();
    source.snapshot = pending;
    let broker = EffectBroker::test_business_fixture(
        &fixture.control,
        fixture.scope(),
        "first".into(),
        source,
        fixture_clients(0, 0),
        TestHooks::default(),
    )
    .unwrap();
    let before = fixture.counts();
    let request = broker.test_business_request("one");
    broker.execute_current(request.clone()).unwrap();
    let fact = finish(&broker, &request).await;
    assert_eq!(fact.state, OperationState::Unresolved);
    assert_eq!(result(&fact).recovery_boundary, "DispatchPending");
    assert_eq!(result(&fact).version, 1);
    let store = BusinessIntentStore::open(&fixture.database).unwrap();
    let current = store.inspect(&target).unwrap().unwrap();
    assert_eq!(current.state(), IntentState::PendingDispatch);
    assert_eq!(current.lease_owner(), Some("w16-dispatcher"));
    assert_eq!(current.lease_generation(), 1);
    assert_eq!(fixture.snapshot(), fixture.initial);
    assert_eq!(fixture.counts(), before);
}

#[tokio::test]
async fn repeated_missing_authority_observation_does_not_append_again_or_upgrade_old_operation() {
    let fixture = Fixture::new(0);
    let broker = fixture.broker("first", TestHooks::default());
    let request = broker.test_business_request("one");
    broker.execute_current(request.clone()).unwrap();
    let fact = finish(&broker, &request).await;
    let after = fixture.snapshot();
    assert_eq!(after.reason(), ReasonCode::FinalizerTerminalRefInvalid);
    assert_eq!(after.version(), 2);
    let second = EffectBroker::test_business_fixture(
        &fixture.temporary.path().join("second-control.sqlite3"),
        fixture.scope(),
        "second".into(),
        fixture.effect(),
        fixture_clients(0, 0),
        TestHooks::default(),
    )
    .unwrap();
    let another = second.test_business_request("two");
    second.execute_current(another.clone()).unwrap();
    let observed = finish(&second, &another).await;
    assert_eq!(observed.state, OperationState::Unresolved);
    assert_eq!(result(&observed).recovery_boundary, "AuthorityBlocked");
    assert_eq!(fixture.snapshot(), after);
    assert_eq!(broker.query_operation(&request).unwrap(), Some(fact));
    assert_eq!(fixture.sink.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn pending_from_another_registered_operation_cannot_authorize_actual_commit() {
    let fixture = Fixture::new(0);
    fixture.send().await;
    let before = fixture.counts();
    let pause = fixture.temporary.path().join("before-effect.sock");
    let listener = tokio::net::UnixListener::bind(&pause).unwrap();
    let broker = fixture.broker(
        "first",
        TestHooks {
            pause_socket: Some(pause),
            business_pending_source_operation: Some("source-operation".into()),
            ..TestHooks::default()
        },
    );
    // Both identities belong to actual registered broker workers. The source stays
    // before its first business read while the target enters real qualification.
    let source = broker.test_business_request("source-operation");
    broker.execute_current(source.clone()).unwrap();
    let (mut source_pause, _) =
        tokio::time::timeout(std::time::Duration::from_secs(20), listener.accept())
            .await
            .unwrap()
            .unwrap();
    let mut signal = [0];
    source_pause.read_exact(&mut signal).await.unwrap();
    assert_eq!(signal, *b"P");
    let target = broker.test_business_request("target-operation");
    broker.execute_current(target.clone()).unwrap();
    let (mut target_pause, _) =
        tokio::time::timeout(std::time::Duration::from_secs(20), listener.accept())
            .await
            .unwrap()
            .unwrap();
    target_pause.read_exact(&mut signal).await.unwrap();
    assert_eq!(signal, *b"P");
    target_pause.write_all(b"G").await.unwrap();
    let target_fact = finish(&broker, &target).await;
    assert_eq!(target_fact.state, OperationState::Unresolved);
    assert!(target_fact.result.is_none());
    let qualified = fixture.snapshot();
    assert_eq!(qualified.state(), IntentState::AwaitingFinalizer);
    assert_eq!(qualified.version(), 2);
    assert_eq!(qualified.reason(), ReasonCode::IntentAuthorityVerified);
    assert_eq!(qualified.lease_generation(), 1);
    let store = BusinessIntentStore::open(&fixture.database).unwrap();
    let chain = store
        .inspect_transition_chain(&qualified.attested_ready_binding().unwrap().intent_id)
        .unwrap();
    assert_eq!(chain.len(), 2);
    assert_eq!(
        chain.last().unwrap().reason(),
        ReasonCode::IntentAuthorityVerified
    );
    // Another valid operation supplies only provenance, never a reusable permit.
    // The denied commit adds no completion, terminal-invalid or conflict event.
    assert_eq!(
        broker.query_operation(&source).unwrap().unwrap().state,
        OperationState::Running
    );
    source_pause.write_all(b"G").await.unwrap();
    assert_eq!(
        finish(&broker, &source).await.state,
        OperationState::Unresolved
    );
    assert_eq!(fixture.snapshot(), qualified);
    assert_eq!(fixture.counts(), before);
    let control = rusqlite::Connection::open(&fixture.control).unwrap();
    let proofs: u64 = control
        .query_row(
            "SELECT count(*) FROM effect_worker_completions",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(proofs, 0);
}
