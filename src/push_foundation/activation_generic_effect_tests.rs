use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{TimeZone, Utc};

use super::activation_fence::*;
use super::activation_fence_tests::{fixture_clients, fixture_scope};
use super::activation_generic_effect::*;
use super::generic_transport::{GenericDispatchFence, GenericTransportRoute};
use super::{
    BusinessIntentStore, FoundationSchemaMigration, InitialIntentDraft, InitialIntentIdentity,
    IntentSnapshot, IntentState, IntentTransitionCommand, LeaseAction, LeaseOwnerId,
    TerminalTemplateBinding, TransitionActor,
};
use crate::durable_delivery::{
    AuthoritativeDeliveryRequest, AuthoritativeSinkPort, AuthoritativeSinkResult,
    CoordinatorConfig, DeliverySubKind, DurableDeliveryCoordinator, FoundationTerminalQuery,
    ImmutableAppendPort, PushKind, TypedReceipt, TypedUncertainty,
};
use crate::monitor::push_job::{
    w08_prepared_push_fixture_for_namespace, w09_completion_policy_fixture, AudienceId,
    AuthorityClass, BusinessDate, ChannelId, CompletionOwnerId, CompletionPolicy, Namespace,
    OccurrenceFamily, OccurrenceIdentityMaterial, OccurrenceKey, ReasonCode, RunId, Sha256Digest,
    SourceContractId, SubjectId, TemplateId, TemplateVersion, UnitId, UtcMicros,
};

pub(super) fn micros(value: i64) -> UtcMicros {
    UtcMicros::try_new(value).unwrap()
}

/// Restart uses the existing exact claimed snapshot; no lease acquisition or rewrite on replay.
pub(super) fn claimed_fixture_at(
    database: &Path,
    test_code: &str,
) -> (
    IntentSnapshot,
    GenericTransportRoute,
    GenericDispatchFence,
    CompletionPolicy,
) {
    if !database.exists() {
        FoundationSchemaMigration::bundled()
            .unwrap()
            .apply_to(database)
            .unwrap();
    }
    let namespace = Namespace::test(RunId::try_new(test_code.into()).unwrap());
    let prepared = w08_prepared_push_fixture_for_namespace(namespace.clone());
    let template = TerminalTemplateBinding::new(
        TemplateId::try_new("auction-card".into()).unwrap(),
        TemplateVersion::try_new("auction-card-v3".into()).unwrap(),
    );
    let identity = InitialIntentIdentity::new(
        namespace,
        UnitId::try_new("MU-auction".into()).unwrap(),
        OccurrenceIdentityMaterial::new(
            BusinessDate::parse("2026-09-07").unwrap(),
            OccurrenceFamily::try_new("auction-session".into()).unwrap(),
            OccurrenceKey::try_new("main".into()).unwrap(),
        ),
        CompletionOwnerId::try_new("owner-auction".into()).unwrap(),
        SourceContractId::try_new("auction-source".into()).unwrap(),
        SubjectId::entity("000001.SZ".into()).unwrap(),
        AudienceId::try_new("portfolio-owner".into()).unwrap(),
    );
    let draft = InitialIntentDraft::ready(
        identity,
        &prepared,
        template.sha256().clone(),
        Sha256Digest::parse("fixture source", &"f".repeat(64)).unwrap(),
        micros(1_788_743_100_000_000),
    )
    .unwrap();
    let mut store = BusinessIntentStore::open(database).unwrap();
    if store.inspect(draft.intent_id()).unwrap().is_none() {
        store.record_initial(&draft).unwrap();
        let snapshot = store.inspect(draft.intent_id()).unwrap().unwrap();
        store
            .apply_nonterminal_transition(
                &IntentTransitionCommand::try_new(
                    draft.intent_id().clone(),
                    IntentState::PendingDispatch,
                    IntentState::AwaitingAuthority,
                    snapshot.version(),
                    TransitionActor::try_new("w16-dispatcher".into()).unwrap(),
                    ReasonCode::IntentDispatchClaimed,
                    micros(1_788_743_101_000_000),
                    LeaseAction::Acquire {
                        owner: LeaseOwnerId::try_new("w16-dispatcher".into()).unwrap(),
                        until: micros(1_788_743_400_000_000),
                    },
                )
                .unwrap(),
            )
            .unwrap();
    }
    let snapshot = store.inspect(draft.intent_id()).unwrap().unwrap();
    assert_eq!(snapshot.namespace(), format!("Test:{test_code}"));
    assert_eq!(snapshot.state(), IntentState::AwaitingAuthority);
    let fence = GenericDispatchFence::try_new(
        LeaseOwnerId::try_new(snapshot.lease_owner().unwrap().into()).unwrap(),
        snapshot.lease_generation(),
        snapshot.lease_until().unwrap(),
    )
    .unwrap();
    let route = GenericTransportRoute::try_new(
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        "GLOBAL".into(),
        ChannelId::try_new("TEST_CODE_W16_CHANNEL".into()).unwrap(),
        template,
    )
    .unwrap();
    let policy = w09_completion_policy_fixture(
        "MU-auction",
        "owner-auction",
        vec![AuthorityClass::GenericCounted],
    );
    (snapshot, route, fence, policy)
}

struct Fixture {
    _temporary: tempfile::TempDir,
    _durable_root: tempfile::TempDir,
    database: PathBuf,
    control: PathBuf,
    durable_database: PathBuf,
    code: String,
    coordinator: Arc<DurableDeliveryCoordinator>,
    sink: Arc<CountingSink>,
    append: Arc<CountingAppend>,
}

impl Fixture {
    fn new(uncertain: bool) -> Self {
        std::fs::create_dir_all("data/test").unwrap();
        let root = tempfile::Builder::new()
            .prefix("TEST_CODE_W16_GENERIC_")
            .tempdir_in("data/test")
            .unwrap();
        let code = root
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
        let temporary = tempfile::tempdir().unwrap();
        let database = temporary.path().join("business.sqlite3");
        let control = temporary.path().join("control.sqlite3");
        Self {
            _temporary: temporary,
            _durable_root: root,
            database,
            control,
            durable_database,
            code,
            coordinator,
            sink: Arc::new(CountingSink {
                calls: AtomicUsize::new(0),
                uncertain,
                requests: Mutex::new(Vec::new()),
            }),
            append: Arc::new(CountingAppend::default()),
        }
    }
    fn scope(&self) -> Scope {
        Scope {
            namespace: format!("Test:{}", self.code),
            unit: "MU-auction".into(),
            ..fixture_scope()
        }
    }
    fn effect(&self) -> GenericEffectFixture {
        let (snapshot, route, fence, completion_policy) =
            claimed_fixture_at(&self.database, &self.code);
        GenericEffectFixture {
            database: self.database.clone(),
            coordinator: Arc::clone(&self.coordinator),
            snapshot,
            route,
            fence,
            completion_policy,
            sink: self.sink.clone(),
            append_port: self.append.clone(),
            dispatched_at: micros(1_788_743_102_000_000),
            verified_at: micros(1_788_743_103_000_000),
        }
    }
    fn broker(&self, epoch: &str, hooks: TestHooks) -> EffectBroker {
        EffectBroker::test_generic_fixture(
            &self.control,
            self.scope(),
            epoch.into(),
            self.effect(),
            fixture_clients(0, 0),
            hooks,
        )
        .unwrap()
    }
    fn counts(&self) -> Vec<u64> {
        let connection = rusqlite::Connection::open(&self.durable_database).unwrap();
        [
            "delivery_decisions",
            "delivery_attempts",
            "sink_results",
            "immutable_audit_outbox",
            "delivery_disposition_payloads",
        ]
        .into_iter()
        .map(|table| {
            connection
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
                .unwrap()
        })
        .collect()
    }
}

struct CountingSink {
    calls: AtomicUsize,
    uncertain: bool,
    requests: Mutex<Vec<(Vec<u8>, String)>>,
}
impl AuthoritativeSinkPort for CountingSink {
    fn sink_identity(&self) -> &str {
        "TEST_CODE_W16_CHANNEL"
    }
    fn deliver(&self, request: &AuthoritativeDeliveryRequest) -> AuthoritativeSinkResult {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.requests.lock().unwrap().push((
            request.rendered_content.clone(),
            request.stable_template_id.clone(),
        ));
        let at = Utc.with_ymd_and_hms(2026, 9, 7, 8, 0, 0).single().unwrap();
        if self.uncertain {
            AuthoritativeSinkResult::Uncertain(TypedUncertainty {
                reason_code: "TEST_CODE_uncertain".into(),
                evidence: b"actual fixture uncertainty".to_vec(),
                observed_at: at,
            })
        } else {
            AuthoritativeSinkResult::Accepted(TypedReceipt {
                channel: self.sink_identity().into(),
                provider: "TEST_CODE_PROVIDER".into(),
                message_id: "TEST_CODE_MESSAGE".into(),
                platform_message_id: None,
                accepted_at: at,
                latency_ms: None,
            })
        }
    }
}

#[derive(Default)]
struct CountingAppend {
    fail: AtomicBool,
    calls: AtomicUsize,
    records: Mutex<BTreeMap<String, Vec<u8>>>,
}
impl ImmutableAppendPort for CountingAppend {
    fn append_exact(
        &self,
        kind: &str,
        identity: &str,
        bytes: &[u8],
        _: &str,
    ) -> crate::durable_delivery::Result<String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail.load(Ordering::SeqCst) {
            return Err(
                crate::durable_delivery::DurableDeliveryError::ImmutableAppendConflict(
                    "fixture append failure".into(),
                ),
            );
        }
        let mut records = self.records.lock().unwrap();
        if let Some(previous) = records.get(identity) {
            assert_eq!(previous, bytes);
        }
        records.insert(identity.into(), bytes.to_vec());
        Ok(format!("immutable://{kind}/{identity}"))
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

#[tokio::test]
async fn actual_generic_dispatch_persists_exact_bytes_once_across_both_replays() {
    let fixture = Fixture::new(false);
    let broker = fixture.broker("first", TestHooks::default());
    let request = broker.test_generic_request("one", WorkClass::NewWork);
    broker.execute_current(request.clone()).unwrap();
    let fact = finish(&broker, &request).await;
    assert_eq!(fact.state, OperationState::Succeeded);
    let EffectResult::Generic(result) = fact.result.as_ref().unwrap() else {
        panic!("Generic result")
    };
    assert_eq!(result.terminal_disposition, GenericDisposition::Accepted);
    assert!(matches!(
        fixture
            .coordinator
            .inspect_foundation_terminal(&result.decision_id)
            .unwrap(),
        FoundationTerminalQuery::Terminal(_)
    ));
    assert_eq!(fixture.sink.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture.sink.requests.lock().unwrap()[0].0,
        b"first render  \nline two!"
    );
    let counts = fixture.counts();
    assert_eq!(&counts[..3], &[1, 1, 1]);
    assert_eq!(broker.execute_current(request.clone()).unwrap(), fact);
    let replay = broker.test_generic_request("two", WorkClass::NewWork);
    broker.execute_current(replay.clone()).unwrap();
    assert_eq!(
        finish(&broker, &replay).await.state,
        OperationState::Succeeded
    );
    assert_eq!(fixture.counts(), counts);
    assert_eq!(fixture.sink.calls.load(Ordering::SeqCst), 1);
    drop(broker);
    let restarted = fixture.broker("second", TestHooks::default());
    assert_eq!(restarted.query_operation(&request).unwrap(), Some(fact));
}

#[tokio::test]
async fn all_request_tuple_and_action_mutations_refuse_without_real_effects() {
    let fixture = Fixture::new(false);
    let broker = fixture.broker("first", TestHooks::default());
    let original = broker.test_generic_request("one", WorkClass::NewWork);
    let before = fixture.counts();
    for index in 0..14 {
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
            8 => bad.actor = "Producer".into(),
            9 => bad.action = "ReconcileGeneric".into(),
            10 => bad.work_class = WorkClass::Recovery,
            11 => bad.effect_sha256.push('0'),
            12 => bad.effect_id = "generic-reconcile".into(),
            13 => bad.operation_id.clear(),
            _ => unreachable!(),
        }
        assert!(broker.execute_current(bad).is_err(), "mutation {index}");
        assert_eq!(fixture.counts(), before, "mutation {index}");
        assert_eq!(fixture.sink.calls.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.append.calls.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn registered_business_lease_drift_rejects_at_actual_worker_seam() {
    let fixture = Fixture::new(false);
    let broker = fixture.broker("first", TestHooks::default());
    let request = broker.test_generic_request("one", WorkClass::NewWork);
    let before = fixture.counts();
    let connection = rusqlite::Connection::open(&fixture.database).unwrap();
    connection
        .execute(
            "UPDATE push_intents SET lease_generation=lease_generation+1",
            [],
        )
        .unwrap();
    broker.execute_current(request.clone()).unwrap();
    assert_eq!(
        finish(&broker, &request).await.state,
        OperationState::Unresolved
    );
    assert_eq!(fixture.counts(), before);
    assert_eq!(fixture.sink.calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.append.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn actual_uncertainty_has_observation_but_no_completion_or_drain() {
    let fixture = Fixture::new(true);
    let broker = fixture.broker("first", TestHooks::default());
    let request = broker.test_generic_request("one", WorkClass::NewWork);
    broker.execute_current(request.clone()).unwrap();
    let fact = finish(&broker, &request).await;
    assert_eq!(fact.state, OperationState::Unresolved);
    let EffectResult::Generic(result) = fact.result.as_ref().unwrap() else {
        panic!("Generic")
    };
    assert_eq!(result.terminal_disposition, GenericDisposition::Uncertain);
    assert!(result.terminal_ref.is_some());
    assert_eq!(fixture.sink.calls.load(Ordering::SeqCst), 1);
    assert_eq!(broker.execute_current(request.clone()).unwrap(), fact);
    let control = rusqlite::Connection::open(&fixture.control).unwrap();
    assert_eq!(
        control
            .query_row("SELECT count(*) FROM effect_worker_completions", [], |r| {
                r.get::<_, u64>(0)
            })
            .unwrap(),
        0
    );
    assert!(
        !broker
            .quiesce(&fixture.scope(), "first", WorkClass::Recovery)
            .unwrap()
            .drained
    );
    drop(broker);
    let restarted = fixture.broker("second", TestHooks::default());
    assert_eq!(restarted.query_operation(&request).unwrap(), Some(fact));
}

#[tokio::test]
async fn pending_seal_recovers_with_new_work_closed_and_never_calls_sink_again() {
    let fixture = Fixture::new(false);
    fixture.append.fail.store(true, Ordering::SeqCst);
    let broker = fixture.broker("first", TestHooks::default());
    let request = broker.test_generic_request("send", WorkClass::NewWork);
    broker.execute_current(request.clone()).unwrap();
    let original = finish(&broker, &request).await;
    assert_eq!(original.state, OperationState::Unresolved);
    assert_eq!(fixture.sink.calls.load(Ordering::SeqCst), 1);
    assert!(matches!(
        original.result,
        Some(EffectResult::Generic(GenericEffectResult {
            terminal_disposition: GenericDisposition::PendingSeal,
            ..
        }))
    ));
    assert_eq!(
        broker.execute_current(broker.test_generic_request("new-send", WorkClass::NewWork)),
        Err(FenceError::Closed)
    );
    fixture.append.fail.store(false, Ordering::SeqCst);
    let recovery = broker.test_generic_request("recover", WorkClass::Recovery);
    broker.execute_current(recovery.clone()).unwrap();
    assert_eq!(
        finish(&broker, &recovery).await.state,
        OperationState::Succeeded
    );
    assert_eq!(fixture.sink.calls.load(Ordering::SeqCst), 1);
    assert_eq!(&fixture.counts()[..3], &[1, 1, 1]);
    assert_eq!(
        broker.query_operation(&request).unwrap().unwrap().state,
        OperationState::Unresolved
    );
    broker
        .quiesce(&fixture.scope(), "first", WorkClass::Recovery)
        .unwrap();
    let before = fixture.counts();
    let appends = fixture.append.calls.load(Ordering::SeqCst);
    assert_eq!(
        broker.execute_current(broker.test_generic_request("closed-recovery", WorkClass::Recovery)),
        Err(FenceError::Closed)
    );
    assert_eq!(fixture.counts(), before);
    assert_eq!(fixture.append.calls.load(Ordering::SeqCst), appends);
}

#[tokio::test]
async fn recovery_of_missing_decision_does_not_prepare_or_send() {
    let fixture = Fixture::new(false);
    let broker = fixture.broker("first", TestHooks::default());
    broker
        .quiesce(&fixture.scope(), "first", WorkClass::NewWork)
        .unwrap();
    let request = broker.test_generic_request("missing-recovery", WorkClass::Recovery);
    let before = fixture.counts();
    broker.execute_current(request.clone()).unwrap();
    let fact = finish(&broker, &request).await;
    assert_eq!(fact.state, OperationState::Unresolved);
    assert!(matches!(
        fact.result,
        Some(EffectResult::Generic(GenericEffectResult {
            terminal_disposition: GenericDisposition::MissingAuthority,
            ..
        }))
    ));
    assert_eq!(fixture.counts(), before);
    assert_eq!(fixture.sink.calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.append.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn exact_recovery_does_not_touch_another_pending_foundation_decision() {
    use super::generic_transport::{GenericDispatchRequest, GenericTransportAuthorityAdapter};
    let fixture = Fixture::new(false);
    let broker = fixture.broker("first", TestHooks::default());
    fixture.append.fail.store(true, Ordering::SeqCst);
    let request = broker.test_generic_request("one", WorkClass::NewWork);
    broker.execute_current(request.clone()).unwrap();
    assert_eq!(
        finish(&broker, &request).await.state,
        OperationState::Unresolved
    );

    let other_database = fixture._temporary.path().join("other-business.sqlite3");
    let (snapshot, route, fence, policy) =
        claimed_fixture_at(&other_database, &format!("{}-other", fixture.code));
    let other_sink = Arc::new(CountingSink {
        calls: AtomicUsize::new(0),
        uncertain: false,
        requests: Mutex::new(Vec::new()),
    });
    let other_append = CountingAppend::default();
    other_append.fail.store(true, Ordering::SeqCst);
    let _ = GenericTransportAuthorityAdapter::new(&fixture.coordinator).dispatch(
        GenericDispatchRequest::new(
            &snapshot,
            &route,
            &fence,
            &policy,
            other_sink.clone(),
            &other_append,
            micros(1_788_743_102_000_000),
            micros(1_788_743_103_000_000),
        ),
    );
    let decision = snapshot.attested_ready_binding().unwrap().decision_id;
    assert!(matches!(
        fixture
            .coordinator
            .inspect_foundation_terminal(decision.as_str())
            .unwrap(),
        FoundationTerminalQuery::PendingSeal { .. }
    ));
    let capture = || {
        let connection = rusqlite::Connection::open(&fixture.durable_database).unwrap();
        let mut all = Vec::new();
        for table in [
            "delivery_decisions",
            "delivery_attempts",
            "delivery_attempt_events",
            "delivery_state_events",
            "immutable_audit_outbox",
            "sink_results",
            "delivery_disposition_payloads",
            "task_transition_payloads",
            "daily_budget_reservations",
            "cooldown_reservations",
        ] {
            let mut statement = connection
                .prepare(&format!(
                    "SELECT * FROM {table} WHERE decision_identity=?1 ORDER BY rowid"
                ))
                .unwrap();
            let count = statement.column_count();
            let rows = statement
                .query_map([decision.as_str()], |row| {
                    (0..count)
                        .map(|index| row.get::<_, rusqlite::types::Value>(index))
                        .collect::<rusqlite::Result<Vec<_>>>()
                })
                .unwrap();
            all.extend(rows.collect::<rusqlite::Result<Vec<_>>>().unwrap());
        }
        all
    };
    let before = capture();
    let other_sends = other_sink.calls.load(Ordering::SeqCst);
    fixture.append.fail.store(false, Ordering::SeqCst);
    let recovery = broker.test_generic_request("recover", WorkClass::Recovery);
    broker.execute_current(recovery.clone()).unwrap();
    assert_eq!(
        finish(&broker, &recovery).await.state,
        OperationState::Succeeded
    );
    assert_eq!(capture(), before);
    assert_eq!(other_sink.calls.load(Ordering::SeqCst), other_sends);
    assert_eq!(fixture.sink.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn generic_result_confirmation_faults_never_create_unobserved_completion() {
    for fault in 0..3 {
        let fixture = Fixture::new(false);
        let hooks = TestHooks {
            result_confirmation_lost: fault == 0,
            final_result_read_failure: fault == 1,
            completion_write_ack_lost: fault == 2,
            ..TestHooks::default()
        };
        let broker = fixture.broker("first", hooks);
        let request = broker.test_generic_request("one", WorkClass::NewWork);
        broker.execute_current(request.clone()).unwrap();
        let fact = finish(&broker, &request).await;
        assert_eq!(fixture.sink.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            fact.state,
            if fault == 2 {
                OperationState::Succeeded
            } else {
                OperationState::Unresolved
            }
        );
        drop(broker);
        let restarted = fixture.broker("second", TestHooks::default());
        assert_eq!(
            restarted.query_operation(&request).unwrap().unwrap().state,
            fact.state
        );
        assert_eq!(fixture.sink.calls.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn result_decoder_rejects_unknown_mixed_and_action_confused_shapes() {
    let initial = r#"{"intent_id":"i","initial_intent_sha256":"s","effect_sha256":"e"}"#;
    let decoded: EffectResult = serde_json::from_str(initial).unwrap();
    assert_eq!(serde_json::to_string(&decoded).unwrap(), initial);
    for value in [
        r#"{"intent_id":"i","initial_intent_sha256":"s","effect_sha256":"e","unknown":1}"#,
        r#"{"kind":"Other","result":{}}"#,
        r#"{"kind":"GenericTransport","result":{},"initial_intent_sha256":"s"}"#,
    ] {
        assert!(serde_json::from_str::<EffectResult>(value).is_err());
    }
    let fixture = Fixture::new(false);
    let broker = fixture.broker("first", TestHooks::default());
    assert_eq!(
        decoded.validate(&broker.test_generic_request("one", WorkClass::NewWork)),
        Err(FenceError::Store)
    );
}

#[test]
fn effect_binding_joins_actual_store_namespace_unit_and_sink() {
    let fixture = Fixture::new(false);
    let mut scope = fixture.scope();
    scope.unit = "other-unit".into();
    assert!(matches!(
        GenericEffect::bind_fixture(&scope, fixture.effect()),
        Err(FenceError::EffectMismatch)
    ));
    scope = fixture.scope();
    scope.namespace.push('x');
    assert!(matches!(
        GenericEffect::bind_fixture(&scope, fixture.effect()),
        Err(FenceError::EffectMismatch)
    ));
    let mut effect = fixture.effect();
    effect.route = GenericTransportRoute::try_new(
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        "GLOBAL".into(),
        ChannelId::try_new("OTHER_CHANNEL".into()).unwrap(),
        TerminalTemplateBinding::new(
            TemplateId::try_new("auction-card".into()).unwrap(),
            TemplateVersion::try_new("auction-card-v3".into()).unwrap(),
        ),
    )
    .unwrap();
    assert!(matches!(
        GenericEffect::bind_fixture(&fixture.scope(), effect),
        Err(FenceError::EffectMismatch)
    ));
    assert_eq!(fixture.counts(), vec![0, 0, 0, 0, 0]);
    assert_eq!(fixture.sink.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn generic_effect_canonical_has_independent_literal_field_golden_and_changes() {
    use crate::monitor::push_job::raw_digest;
    use std::os::unix::fs::MetadataExt;
    let fixture = Fixture::new(false);
    let source = fixture.effect();
    let attested = source.snapshot.attested_ready_binding().unwrap();
    let database = std::fs::canonicalize(&fixture.database).unwrap();
    let metadata = std::fs::metadata(&database).unwrap();
    let durable = std::fs::metadata(&fixture.durable_database).unwrap();
    // Independent JSON literal: no Scope/snapshot/route/fence effect encoder is reused.
    let dispatch_fence = serde_json::json!({
        "generation":1,"owner":"w16-dispatcher","until":"1788743400000000"
    });
    let route = serde_json::json!({
        "push_kind":"HoldingEvent","sub_kind":"NONE","scope_key":"GLOBAL",
        "required_channel":"TEST_CODE_W16_CHANNEL","template_id":"auction-card",
        "template_version":"auction-card-v3",
        "template_sha256":raw_digest(b"TemplateBinding/v1\0{\"template_id\":\"auction-card\",\"template_version\":\"auction-card-v3\"}").as_str()
    });
    let snapshot = serde_json::json!({
        "audience":"portfolio-owner","business_date":"2026-09-07","completion_owner":"owner-auction","created_at":"1788743100000000",
        "decision_id":attested.decision_id.as_str(),"decision_kind":"Ready","evidence_sha256":"b".repeat(64),"intent_id":attested.intent_id.as_str(),
        "lease_generation":1,"lease_owner":"w16-dispatcher","lease_until":"1788743400000000","namespace":format!("Test:{}",fixture.code),
        "occurrence_family":"auction-session","occurrence_key":"main","payload_sha256":raw_digest(source.snapshot.prepared_push_bytes().unwrap()).as_str(),
        "prepared_push_bytes":source.snapshot.prepared_push_bytes().unwrap(),"previous_state":"PendingDispatch","reason":"intent.dispatch_claimed",
        "rendered_bytes":b"first render  \nline two!".to_vec(),"rendered_sha256":raw_digest(b"first render  \nline two!").as_str(),
        "source_contract_id":"auction-source","source_contract_sha256":"f".repeat(64),"state":"AwaitingAuthority","subject":"Entity:000001.SZ",
        "template_sha256":raw_digest(b"TemplateBinding/v1\0{\"template_id\":\"auction-card\",\"template_version\":\"auction-card-v3\"}").as_str(),
        "unit_id":"MU-auction","updated_at":"1788743101000000","version":1
    });
    let expected = serde_json::json!({
        "action":"DispatchGeneric", "actor":"Dispatcher",
        "business_store_device": metadata.dev(), "business_store_inode": metadata.ino(), "business_store_path":database.to_str().unwrap(),
        "completion_policy_bytes":source.completion_policy.activation_binding_bytes(),
        "decision_id":attested.decision_id.as_str(), "deployment":"fixture-deployment",
        "dispatch_fence":dispatch_fence,
        "dispatched_at":"1788743102000000",
        "durable_environment":format!("Test:{}",fixture.code),"durable_owner":format!("owner-{}",fixture.code),
        "durable_store_device":durable.dev(),"durable_store_inode":durable.ino(),"durable_store_path":fixture.durable_database.to_str().unwrap(),
        "effect_id":"generic-dispatch","generation":11,"incarnation":"deployment-one",
        "intent_id":attested.intent_id.as_str(),"manifest":"a".repeat(64),"namespace":format!("Test:{}",fixture.code),
        "occurrence":attested.occurrence.as_str(),"physical_owner":"generic-owner",
        "route":route,
        "sink_identity":"TEST_CODE_W16_CHANNEL",
        "snapshot":snapshot,
        "unit":"MU-auction","verified_at":"1788743103000000","work_class":"NewWork"
    });
    let expected_bytes = [
        b"ActivationGenericTransportEffect/v1\0".as_slice(),
        serde_json::to_vec(&expected).unwrap().as_slice(),
    ]
    .concat();
    let [dispatch, reconcile] = GenericEffect::bind_fixture(&fixture.scope(), source).unwrap();
    assert_eq!(dispatch.canonical_bytes().unwrap(), expected_bytes);
    assert_eq!(dispatch.digest(), raw_digest(&expected_bytes).as_str());
    assert_ne!(dispatch.digest(), reconcile.digest());
    for field in 0..8 {
        let mut scope = fixture.scope();
        let mut source = fixture.effect();
        match field {
            0 => scope.generation += 1,
            1 => scope.manifest = "d".repeat(64),
            2 => scope.physical_owner.push('x'),
            3 => scope.deployment.push('x'),
            4 => scope.incarnation.push('x'),
            5 => source.dispatched_at = micros(1_788_743_102_000_001),
            6 => source.verified_at = micros(1_788_743_103_000_001),
            7 => {
                source.completion_policy = w09_completion_policy_fixture(
                    "MU-auction",
                    "owner-auction",
                    vec![AuthorityClass::GenericCounted, AuthorityClass::P01Dedicated],
                )
            }
            _ => unreachable!(),
        }
        let [changed, _] = GenericEffect::bind_fixture(&scope, source).unwrap();
        assert_ne!(dispatch.digest(), changed.digest(), "field {field}");
    }
}

#[tokio::test]
async fn generic_completion_exact_domain_and_terminal_tampering_are_detected() {
    use crate::monitor::push_job::raw_digest;
    let fixture = Fixture::new(false);
    let broker = fixture.broker("first", TestHooks::default());
    let request = broker.test_generic_request("one", WorkClass::NewWork);
    broker.execute_current(request.clone()).unwrap();
    let fact = finish(&broker, &request).await;
    let EffectResult::Generic(result) = fact.result.as_ref().unwrap() else {
        panic!("Generic result")
    };
    let connection = rusqlite::Connection::open(&fixture.control).unwrap();
    let (bytes, hash): (Vec<u8>, String) = connection.query_row("SELECT completion_bytes,completion_sha256 FROM effect_worker_completions WHERE operation_id='one'", [], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
    let expected = format!(concat!("ActivationGenericTransportWorkerCompletion/v1\0{{",
        "\"decision_id\":\"{}\",\"effect_sha256\":\"{}\",\"intent_id\":\"{}\",",
        "\"operation_id\":\"one\",\"original_epoch\":\"first\",\"request_sha256\":\"{}\",\"state\":\"Succeeded\",",
        "\"terminal_binding_sha256\":\"{}\",\"terminal_disposition\":\"Accepted\",\"terminal_evidence_sha256\":\"{}\",\"terminal_ref\":\"{}\"}}"),
        result.decision_id,result.effect_sha256,result.intent_id,request.digest(),result.terminal_binding_sha256.as_ref().unwrap(),result.terminal_evidence_sha256.as_ref().unwrap(),result.terminal_ref.as_ref().unwrap());
    assert_eq!(bytes, expected.as_bytes());
    assert_eq!(hash, raw_digest(expected.as_bytes()).as_str());
    let encoded = serde_json::to_value(fact.result.as_ref().unwrap()).unwrap();
    for field in [
        "terminal_ref",
        "terminal_binding_sha256",
        "terminal_evidence_sha256",
    ] {
        let mut missing = encoded.clone();
        missing["result"].as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<EffectResult>(missing).is_err());
    }
    let mut mixed = encoded.clone();
    mixed["result"]["initial_intent_sha256"] = serde_json::Value::String("a".repeat(64));
    assert!(serde_json::from_value::<EffectResult>(mixed).is_err());
    let mut unknown = encoded;
    unknown["kind"] = serde_json::Value::String("Other".into());
    assert!(serde_json::from_value::<EffectResult>(unknown).is_err());
    for field in [
        "terminal_ref",
        "terminal_binding_sha256",
        "terminal_evidence_sha256",
        "decision_id",
        "effect_sha256",
    ] {
        let mut encoded = serde_json::to_value(fact.result.as_ref().unwrap()).unwrap();
        encoded["result"][field] = serde_json::Value::String("e".repeat(64));
        connection
            .execute(
                "UPDATE effect_operations SET result_json=?1 WHERE operation_id='one'",
                [serde_json::to_string(&encoded).unwrap()],
            )
            .unwrap();
        assert_eq!(
            broker.query_operation(&request),
            Err(FenceError::Store),
            "field {field}"
        );
    }
    connection
        .execute(
            "UPDATE effect_operations SET result_json=?1 WHERE operation_id='one'",
            [serde_json::to_string(fact.result.as_ref().unwrap()).unwrap()],
        )
        .unwrap();
    assert_eq!(broker.query_operation(&request).unwrap(), Some(fact));
}
