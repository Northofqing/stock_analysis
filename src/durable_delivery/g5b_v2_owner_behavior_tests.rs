//! C2 actual isolated model/input/archive -> immutable counted owner behavior.
use super::model_archive_v2_tests::{
    arm_nth_sql, artifact, capture_replaced, freeze_member, replace, replacement,
};
use super::*;
use crate::monitor::g5b_analysis_v2::{
    archive_model_observations_v2, prepare_model_owner_v2_for_test,
};

fn owners(fixture: &Fixture) -> i64 {
    fixture.query_i64("SELECT COUNT(*) FROM g5b_occurrence_owners")
}
fn revision(fixture: &Fixture) -> i64 {
    fixture.query_i64("SELECT revision FROM g5b_day_heads")
}
fn snapshot(fixture: &Fixture) -> BTreeMap<String, Vec<Vec<String>>> {
    let connection = Connection::open(&fixture.database_path).unwrap();
    let mut query=connection.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap();
    let names = query
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    authority_snapshot(
        &connection,
        &names.iter().map(String::as_str).collect::<Vec<_>>(),
    )
}
fn actual_envelope(fixture: &Fixture, index: usize) -> DeliveryEnvelope {
    let connection = Connection::open(&fixture.database_path).unwrap();
    let frozen:Vec<u8>=connection.query_row("SELECT e.desired_bytes FROM g5b_artifact_events e JOIN g5b_selected_occurrences m ON m.occurrence_identity=e.occurrence_identity WHERE e.artifact_role='Frozen' AND e.phase='Prepared' AND m.selection_index=?1",[index as i64],|r|r.get(0)).unwrap();
    let handoff: serde_json::Value = serde_json::from_slice(&frozen).unwrap();
    let bytes: Vec<u8> = serde_json::from_value(handoff["envelope_canonical"].clone()).unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn valid_legacy_extra() -> DeliveryEnvelope {
    let code = "600002";
    let triggered_at = format!("{DATE}T15:00:00+08:00");
    let category = "额外事件";
    let message = "未被选定";
    let rendered = b"TEST_CODE_LEGACY_EXTRA".to_vec();
    let source=serde_json::to_vec(&serde_json::json!({"schema":"g5b-attribution-v1","business_date":DATE,"code":code,
        "triggered_at":triggered_at,"level":"重要","category":category,"message":message,"rendered_sha256":sha256_hex(&rendered)})).unwrap();
    let occurrence = format!(
        "g5b-attribution:{DATE}:{code}:{}",
        sha256_hex(format!("{triggered_at}|{code}|{category}|{message}").as_bytes())
    );
    DeliveryEnvelope::new(
        DATE,
        PushKind::G5bAttribution,
        DeliverySubKind::None,
        "GLOBAL",
        &occurrence,
        sha256_hex(&source),
        source.clone(),
        sha256_hex(&source),
        rendered,
        true,
        None,
    )
    .unwrap()
}

#[tokio::test]
async fn g5b_v2_owner_partial_member_exact_replay_and_identical_raw_two_owners() {
    let fixture = Fixture::new("C2_IDENTICAL_MEMBERS");
    let line = raw("600001");
    let log = input(&fixture, &[line.clone(), line]);
    let (provider, calls) = model(&log, Mode::Good);
    freeze_member(&fixture, &provider, 0).await;
    archive_model_observations_v2(owner(&fixture), date())
        .unwrap()
        .unwrap();
    capture_snapshots(&fixture);
    let envelope = actual_envelope(&fixture, 0);
    let before = snapshot(&fixture);
    assert!(
        fixture
            .coordinator
            .prepare(&envelope, 1, clock("15:12:00"))
            .is_err(),
        "public bytes cannot mint a fresh v2 owner"
    );
    assert_eq!(snapshot(&fixture), before);
    let revision_before = revision(&fixture);
    let outcome =
        prepare_model_owner_v2_for_test(owner(&fixture), date(), 0, 1, clock("15:12:00")).unwrap();
    assert_eq!(outcome.state, DecisionState::Reserved);
    assert_eq!(owners(&fixture), 1);
    assert_eq!(
        revision(&fixture),
        revision_before + 1,
        "decision, reservation and owner advance once together"
    );
    let saved = snapshot(&fixture);
    assert_eq!(
        prepare_model_owner_v2_for_test(owner(&fixture), date(), 0, 1, clock("16:00:00")).unwrap(),
        outcome
    );
    assert_eq!(
        fixture
            .coordinator
            .prepare(&envelope, 1, clock("16:00:00"))
            .unwrap(),
        outcome,
        "existing generic exact replay remains compatible"
    );
    assert!(
        prepare_model_owner_v2_for_test(owner(&fixture), date(), 1, 1, clock("15:12:00")).is_err()
    );
    let extra = valid_legacy_extra();
    extra.validate().unwrap();
    assert!(fixture
        .coordinator
        .prepare(&extra, 1, clock("15:12:00"))
        .is_err());
    assert_eq!(snapshot(&fixture), saved);
    freeze_member(&fixture, &provider, 1).await;
    archive_model_observations_v2(owner(&fixture), date())
        .unwrap()
        .unwrap();
    capture_snapshots(&fixture);
    prepare_model_owner_v2_for_test(owner(&fixture), date(), 1, 1, clock("15:13:00")).unwrap();
    assert_eq!(owners(&fixture), 2);
    assert_ne!(
        actual_envelope(&fixture, 0).schedule_occurrence_identity,
        actual_envelope(&fixture, 1).schedule_occurrence_identity
    );
    assert_eq!(
        fixture
            .coordinator
            .g5b_counted_day_snapshot(DATE)
            .unwrap()
            .facts()
            .len(),
        2
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn g5b_v2_owner_real_pre_sink_denial_is_owned_and_replayed_without_model_or_send() {
    let fixture = Fixture::new("C2_OWNED_DENIAL");
    let log = input(&fixture, &[raw("600001")]);
    let (provider, calls) = model(&log, Mode::Good);
    freeze_member(&fixture, &provider, 0).await;
    archive_model_observations_v2(owner(&fixture), date())
        .unwrap()
        .unwrap();
    capture_snapshots(&fixture);
    let before = revision(&fixture);
    let outcome =
        prepare_model_owner_v2_for_test(owner(&fixture), date(), 0, 0, clock("15:12:00")).unwrap();
    assert_eq!(outcome.state, DecisionState::RejectedAuditPending);
    assert_eq!(owners(&fixture), 1);
    assert_eq!(revision(&fixture), before + 1);
    fixture
        .coordinator
        .reconcile_all_pending(&MemoryAppendPort::default(), clock("15:12:01"))
        .unwrap();
    let replay =
        prepare_model_owner_v2_for_test(owner(&fixture), date(), 0, 1, clock("16:00:00")).unwrap();
    assert_eq!(
        replay.state,
        DecisionState::RejectedDurable,
        "replay cannot re-admit a denied original owner"
    );
    let snapshot = fixture.coordinator.g5b_counted_day_snapshot(DATE).unwrap();
    assert_eq!(
        snapshot.facts()[0].observation().terminal(),
        crate::durable_delivery::G5bCountedTerminalV1::Rejected
    );
    assert!(!snapshot.facts()[0]
        .observation()
        .is_authoritative_accepted());
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM delivery_attempts"),
        0
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn g5b_v2_owner_last_admission_sql_file_or_prefix_mutation_rolls_back_entire_decision() {
    for role in ["Attempt", "Frozen", "Archive", "Source"] {
        let fixture = Fixture::new("C2_ADMISSION_LAST_SQL");
        let log = input(&fixture, &[raw("600001")]);
        let (provider, _) = model(&log, Mode::Good);
        freeze_member(&fixture, &provider, 0).await;
        archive_model_observations_v2(owner(&fixture), date())
            .unwrap()
            .unwrap();
        capture_snapshots(&fixture);
        let path = if role == "Source" {
            namespace(&fixture).join("20260928.jsonl")
        } else {
            artifact(&fixture, role)
        };
        let (new, aside) = replacement(&fixture, &path);
        let original = path.clone();
        let new_path = new.clone();
        let aside_path = aside.clone();
        let before = snapshot(&fixture);
        let hit = Arc::new(AtomicUsize::new(0));
        // The first SQL transaction captures the actual bundle. The second is
        // the real decision/owner transaction, after all INSERTs have run.
        arm_nth_sql(
            Arc::downgrade(&owner(&fixture)),
            2,
            Arc::new(Mutex::new(Some(Box::new(move || {
                replace(&original, &new_path, &aside_path)
            })))),
            Arc::clone(&hit),
        );
        assert!(
            prepare_model_owner_v2_for_test(owner(&fixture), date(), 0, 1, clock("15:12:00"))
                .is_err()
        );
        assert_eq!(hit.load(Ordering::SeqCst), 1);
        capture_replaced(&fixture, &path, &aside);
        assert_eq!(
            snapshot(&fixture),
            before,
            "{role} boundary must roll back owner, reservations, audits and revision"
        );
        assert_eq!(owners(&fixture), 0);
    }
}

#[tokio::test]
async fn g5b_v2_owner_restart_missing_model_file_refuses_attempt_without_healing() {
    let fixture = Fixture::new("C2_RESTART_MISSING");
    let log = input(&fixture, &[raw("600001")]);
    let (provider, calls) = model(&log, Mode::Good);
    freeze_member(&fixture, &provider, 0).await;
    archive_model_observations_v2(owner(&fixture), date())
        .unwrap()
        .unwrap();
    capture_snapshots(&fixture);
    let outcome =
        prepare_model_owner_v2_for_test(owner(&fixture), date(), 0, 1, clock("15:12:00")).unwrap();
    let path = artifact(&fixture, "Frozen");
    std::fs::remove_file(&path).unwrap();
    let before = snapshot(&fixture);
    let sink = StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(clock(
        "15:12:01",
    ))));
    let sinks: [AuthoritativeSink; 1] = [sink.clone()];
    assert!(fixture
        .coordinator
        .resume_deliverable(&outcome.decision_identity, &sinks, clock("15:12:01"))
        .is_err());
    assert_eq!(sink.calls.load(Ordering::SeqCst), 0);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(snapshot(&fixture), before);
    assert!(!path.exists());
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM delivery_attempts"),
        0
    );
}

#[tokio::test]
async fn g5b_v2_owner_last_begin_sql_model_mutation_rolls_back_attempt_and_zero_sink() {
    for role in ["Attempt", "Frozen", "Archive"] {
        let fixture = Fixture::new("C2_BEGIN_LAST_SQL");
        let log = input(&fixture, &[raw("600001")]);
        let (provider, _) = model(&log, Mode::Good);
        freeze_member(&fixture, &provider, 0).await;
        archive_model_observations_v2(owner(&fixture), date())
            .unwrap()
            .unwrap();
        capture_snapshots(&fixture);
        let outcome =
            prepare_model_owner_v2_for_test(owner(&fixture), date(), 0, 1, clock("15:12:00"))
                .unwrap();
        let path = artifact(&fixture, role);
        let (new, aside) = replacement(&fixture, &path);
        let original = path.clone();
        let new_path = new.clone();
        let aside_path = aside.clone();
        let hit = Arc::new(AtomicUsize::new(0));
        let before = snapshot(&fixture);
        arm_nth_sql(
            Arc::downgrade(&owner(&fixture)),
            2,
            Arc::new(Mutex::new(Some(Box::new(move || {
                replace(&original, &new_path, &aside_path)
            })))),
            Arc::clone(&hit),
        );
        let sink = StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(clock(
            "15:12:01",
        ))));
        let sinks: [AuthoritativeSink; 1] = [sink.clone()];
        assert!(fixture
            .coordinator
            .begin_attempt(&outcome.decision_identity, sinks.len(), clock("15:12:01"))
            .is_err());
        assert_eq!(hit.load(Ordering::SeqCst), 1);
        capture_replaced(&fixture, &path, &aside);
        assert!(fixture
            .coordinator
            .resume_deliverable(&outcome.decision_identity, &sinks, clock("15:12:02"))
            .is_err());
        assert_eq!(sink.calls.load(Ordering::SeqCst), 0);
        assert_eq!(snapshot(&fixture), before);
    }
}

struct FenceCheckingSink {
    expected_content: Vec<u8>,
    log: AlertLog,
    calls: AtomicUsize,
    result: AuthoritativeSinkResult,
}
impl AuthoritativeSinkPort for FenceCheckingSink {
    fn sink_identity(&self) -> &str {
        "TEST_CODE_C2_FENCE_FREE_SINK"
    }
    fn deliver(&self, request: &AuthoritativeDeliveryRequest) -> AuthoritativeSinkResult {
        assert_eq!(request.rendered_content, self.expected_content);
        let guard = self.log.acquire_date_writer_fence(date()).unwrap();
        drop(guard);
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.result.clone()
    }
}
#[tokio::test]
async fn g5b_v2_owner_real_accepted_receipt_uses_saved_summary_and_releases_fence_before_sink() {
    let fixture = Fixture::new("C2_REAL_ACCEPTED");
    let log = input(&fixture, &[raw("600001")]);
    let (provider, calls) = model(&log, Mode::Good);
    freeze_member(&fixture, &provider, 0).await;
    archive_model_observations_v2(owner(&fixture), date())
        .unwrap()
        .unwrap();
    capture_snapshots(&fixture);
    let envelope = actual_envelope(&fixture, 0);
    let outcome =
        prepare_model_owner_v2_for_test(owner(&fixture), date(), 0, 1, clock("15:12:00")).unwrap();
    let sink = Arc::new(FenceCheckingSink {
        expected_content: envelope.rendered_content.clone(),
        log: log.clone(),
        calls: AtomicUsize::new(0),
        result: AuthoritativeSinkResult::Accepted(receipt(clock("15:12:01"))),
    });
    let sinks: [AuthoritativeSink; 1] = [sink.clone()];
    let result = fixture
        .coordinator
        .resume_deliverable(&outcome.decision_identity, &sinks, clock("15:12:01"))
        .unwrap();
    assert_eq!(result.sink_calls, 1);
    assert!(result.persisted_receipt);
    let append = MemoryAppendPort::default();
    fixture
        .coordinator
        .reconcile_all_pending(&append, clock("15:12:02"))
        .unwrap();
    let snapshot = fixture.coordinator.g5b_counted_day_snapshot(DATE).unwrap();
    assert_eq!(
        snapshot.facts()[0].observation().terminal(),
        crate::durable_delivery::G5bCountedTerminalV1::Accepted
    );
    assert!(snapshot.facts()[0]
        .observation()
        .is_authoritative_accepted());
    assert_eq!(sink.calls.load(Ordering::SeqCst), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let saved: DeliveryEnvelope = serde_json::from_slice(
        &fixture.query_blob("SELECT envelope_canonical FROM delivery_decisions"),
    )
    .unwrap();
    assert_eq!(saved.rendered_content, envelope.rendered_content);
    assert_eq!(owners(&fixture), 1);
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM g5b_day_seals"), 0);
}

#[tokio::test]
async fn g5b_v2_owner_original_conflict_audit_and_late_raw_result_survive_missing_model_file() {
    let fixture = Fixture::new("C2_CONFLICT_RAW_LATE");
    let log = input(&fixture, &[raw("600001")]);
    let (provider, _) = model(&log, Mode::Good);
    freeze_member(&fixture, &provider, 0).await;
    archive_model_observations_v2(owner(&fixture), date())
        .unwrap()
        .unwrap();
    capture_snapshots(&fixture);
    let outcome =
        prepare_model_owner_v2_for_test(owner(&fixture), date(), 0, 1, clock("15:12:00")).unwrap();
    let mut conflict = actual_envelope(&fixture, 0);
    conflict.rendered_content.push(b'!');
    let audits = fixture.query_i64(
        "SELECT COUNT(*) FROM immutable_audit_outbox WHERE audit_kind='DecisionIdentityConflict'",
    );
    assert!(matches!(
        fixture.coordinator.prepare(&conflict, 1, clock("15:12:01")),
        Err(DurableDeliveryError::DecisionIdentityConflict { .. })
    ));
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM immutable_audit_outbox WHERE audit_kind='DecisionIdentityConflict'"),audits+1);
    let attempt = fixture
        .coordinator
        .begin_attempt(&outcome.decision_identity, 1, clock("15:12:02"))
        .unwrap()
        .unwrap();
    std::fs::remove_file(artifact(&fixture, "Frozen")).unwrap();
    let late_at = clock("15:12:02") + chrono::Duration::seconds(121);
    fixture
        .coordinator
        .reconcile_all_pending(&MemoryAppendPort::default(), late_at)
        .unwrap();
    let late_receipt = receipt(late_at + chrono::Duration::seconds(1));
    let exact_raw =
        serde_json::to_vec(&serde_json::json!({"kind":"Accepted","receipt":late_receipt})).unwrap();
    let raw = AuthoritativeSinkResult::Accepted(late_receipt);
    fixture
        .coordinator
        .record_sink_result(
            &attempt.attempt_identity,
            attempt.fence_token,
            raw,
            late_at + chrono::Duration::seconds(1),
        )
        .unwrap();
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM sink_results WHERE authoritative_for_state=0 AND late_after_fence=1"),1);
    assert_eq!(
        fixture.query_blob("SELECT result_canonical FROM sink_results WHERE late_after_fence=1"),
        exact_raw
    );
    assert_eq!(
        fixture.query_i64(
            "SELECT COUNT(*) FROM immutable_audit_outbox WHERE audit_kind='LateReceiptObserved'"
        ),
        1
    );
    assert_eq!(owners(&fixture), 1);
    assert_eq!(
        fixture
            .coordinator
            .g5b_counted_day_snapshot(DATE)
            .unwrap()
            .facts()[0]
            .observation()
            .terminal(),
        crate::durable_delivery::G5bCountedTerminalV1::Uncertain
    );
}

#[tokio::test]
async fn g5b_v2_owner_uncommitted_archive_is_not_adopted_then_exact_committed_archive_qualifies() {
    let fixture = Fixture::new("C2_PREPARED_ARCHIVE");
    let log = input(&fixture, &[raw("600001")]);
    let (provider, calls) = model(&log, Mode::Good);
    freeze_member(&fixture, &provider, 0).await;
    let prepared =
        crate::monitor::g5b_analysis_v2::prepare_model_archive_for_test(owner(&fixture), date())
            .unwrap();
    let before = snapshot(&fixture);
    assert!(
        prepare_model_owner_v2_for_test(owner(&fixture), date(), 0, 1, clock("15:12:00")).is_err()
    );
    assert_eq!(snapshot(&fixture), before);
    assert_eq!(owners(&fixture), 0);
    let archived = archive_model_observations_v2(owner(&fixture), date())
        .unwrap()
        .unwrap();
    capture_snapshots(&fixture);
    assert_eq!(archived.archive_identity(), prepared.identity());
    assert_eq!(archived.canonical_bytes(), prepared.desired_bytes());
    prepare_model_owner_v2_for_test(owner(&fixture), date(), 0, 1, clock("15:12:00")).unwrap();
    assert_eq!(owners(&fixture), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn g5b_v2_owner_valid_unowned_direct_sql_row_fails_global_reader_and_is_never_adopted() {
    let fixture = Fixture::new("C2_VALID_ORPHAN");
    let log = input(&fixture, &[raw("600001")]);
    let (provider, _) = model(&log, Mode::Good);
    freeze_member(&fixture, &provider, 0).await;
    archive_model_observations_v2(owner(&fixture), date())
        .unwrap()
        .unwrap();
    capture_snapshots(&fixture);
    let envelope = actual_envelope(&fixture, 0);
    envelope.validate().unwrap();
    let canonical = envelope.canonical_bytes().unwrap();
    let connection = Connection::open(&fixture.database_path).unwrap();
    connection.execute("INSERT INTO delivery_decisions(decision_identity,business_date,push_kind,sub_kind,cooldown_scope,scope_key,state,envelope_version,envelope_canonical,envelope_sha256,task_binding_present,reservation_generation,fence_generation,retry_authorized,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,'Reserved',?7,?8,?9,0,0,0,1,?10,?10)",params![envelope.decision_identity,DATE,envelope.push_kind.as_str(),envelope.sub_kind.as_str(),envelope.cooldown_scope.as_str(),envelope.scope_key,envelope.envelope_version,canonical,sha256_hex(&canonical),clock("15:12:00").to_rfc3339()]).unwrap();
    drop(connection);
    let before = snapshot(&fixture);
    let error = fixture
        .coordinator
        .g5b_counted_day_snapshot(DATE)
        .unwrap_err()
        .to_string();
    assert!(error.contains("no unique immutable owner"), "{error}");
    assert!(
        prepare_model_owner_v2_for_test(owner(&fixture), date(), 0, 1, clock("15:12:00")).is_err()
    );
    assert_eq!(snapshot(&fixture), before);
    assert_eq!(owners(&fixture), 0);
}

#[tokio::test]
async fn g5b_v2_owner_dispatch_view_is_read_only_and_stale_view_cannot_mint_owner() {
    let fixture = Fixture::new("C2_VIEW_READ_ONLY");
    let log = input(&fixture, &[raw("600001")]);
    let (provider, calls) = model(&log, Mode::Good);
    freeze_member(&fixture, &provider, 0).await;
    archive_model_observations_v2(owner(&fixture), date())
        .unwrap()
        .unwrap();
    capture_snapshots(&fixture);
    let before = snapshot(&fixture);
    let view =
        crate::monitor::g5b_analysis_v2::inspect_model_dispatch_v2(owner(&fixture), date(), 0)
            .unwrap()
            .unwrap();
    assert_eq!(view.envelope(), &actual_envelope(&fixture, 0));
    assert_eq!(view.business_date(), date());
    assert_eq!(view.selection_index(), 0);
    assert_eq!(view.source_sha256(), sha256_hex(view.source_canonical()));
    assert_eq!(view.rendered_sha256(), sha256_hex(view.rendered_content()));
    assert_eq!(snapshot(&fixture), before);
    assert_eq!(owners(&fixture), 0);
    let path = artifact(&fixture, "Attempt");
    let (new, aside) = replacement(&fixture, &path);
    replace(&path, &new, &aside);
    capture_replaced(&fixture, &path, &aside);
    assert!(prepare_model_owner_v2_for_test(
        owner(&fixture),
        view.business_date(),
        view.selection_index(),
        1,
        clock("15:12:00")
    )
    .is_err());
    assert!(fixture
        .coordinator
        .prepare(view.envelope(), 1, clock("15:12:00"))
        .is_err());
    assert_eq!(snapshot(&fixture), before);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn g5b_v2_owner_retry_reacquisition_checks_actual_archive_before_new_reservation_or_send() {
    let fixture = Fixture::new("C2_RETRY_FILE_BOUNDARY");
    let log = input(&fixture, &[raw("600001")]);
    let (provider, calls) = model(&log, Mode::Good);
    freeze_member(&fixture, &provider, 0).await;
    archive_model_observations_v2(owner(&fixture), date())
        .unwrap()
        .unwrap();
    capture_snapshots(&fixture);
    let outcome =
        prepare_model_owner_v2_for_test(owner(&fixture), date(), 0, 1, clock("15:12:00")).unwrap();
    let sink = StaticSink::new(AuthoritativeSinkResult::Rejected(rejection(
        clock("15:12:01"),
        true,
    )));
    let sinks: [AuthoritativeSink; 1] = [sink.clone()];
    assert_eq!(
        fixture
            .coordinator
            .resume_deliverable(&outcome.decision_identity, &sinks, clock("15:12:01"))
            .unwrap()
            .sink_calls,
        1
    );
    fixture
        .coordinator
        .reconcile_all_pending(&MemoryAppendPort::default(), clock("15:12:02"))
        .unwrap();
    assert_eq!(
        fixture
            .coordinator
            .decision_state(&outcome.decision_identity)
            .unwrap(),
        DecisionState::RejectedDurable
    );
    let path = artifact(&fixture, "Archive");
    let (new, aside) = replacement(&fixture, &path);
    replace(&path, &new, &aside);
    capture_replaced(&fixture, &path, &aside);
    let before = snapshot(&fixture);
    assert!(fixture
        .coordinator
        .resume_deliverable(&outcome.decision_identity, &sinks, clock("15:13:00"))
        .is_err());
    assert_eq!(snapshot(&fixture), before);
    assert_eq!(sink.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM delivery_attempts"),
        1
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn g5b_v2_owner_generic_duplicate_schema_cannot_hide_v2_without_any_cohort() {
    for declared in [
        br#"{"schema":"g5b-attribution-v2","schema":"g5b-attribution-v1"}"#.as_slice(),
        br#"{"schema":"g5b-attribution-v2","x":[}"#.as_slice(),
    ] {
        let fixture = Fixture::new("C2_DUPLICATE_SOURCE_CLASSIFICATION");
        let _log = fixture.g5b_input_log(DATE);
        let source = declared.to_vec();
        let envelope = DeliveryEnvelope::new(
            DATE,
            PushKind::G5bAttribution,
            DeliverySubKind::None,
            "GLOBAL",
            "TEST_CODE_DUPLICATE_SCHEMA_OCCURRENCE",
            sha256_hex(&source),
            source.clone(),
            sha256_hex(&source),
            b"TEST_CODE_DUPLICATE_SCHEMA_CONTENT".to_vec(),
            true,
            None,
        )
        .unwrap();
        envelope.validate().unwrap();
        let before = snapshot(&fixture);
        let error = fixture
            .coordinator
            .prepare(&envelope, 1, clock("15:12:00"))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("requires private actual-cohort admission"),
            "{error}"
        );
        assert_eq!(snapshot(&fixture), before);
    }
}

fn arm_sql_phase_fault(
    weak: std::sync::Weak<DurableDeliveryCoordinator>,
    phase: DatabaseOperationTestPhase,
    remaining: usize,
    hit: Arc<AtomicUsize>,
    fault: OperationPostvalidationTestFault,
) {
    let coordinator = weak.upgrade().unwrap();
    coordinator
        .install_database_operation_test_hook(phase, move || {
            if remaining == 1 {
                hit.fetch_add(1, Ordering::SeqCst);
                weak.upgrade()
                    .unwrap()
                    .install_operation_postvalidation_test_fault(fault)?;
            } else {
                arm_sql_phase_fault(weak, phase, remaining - 1, hit, fault);
            }
            Ok(())
        })
        .unwrap();
}
#[tokio::test]
async fn g5b_v2_owner_both_post_sql_hooks_extra_revision_roll_back_changed_and_nochange_replay() {
    for phase in [
        DatabaseOperationTestPhase::AfterSqlBeforePreCommitValidation,
        DatabaseOperationTestPhase::AfterSqlBeforePostValidation,
    ] {
        for already_owned in [false, true] {
            let fixture = Fixture::new("C2_POST_SQL_REVISION");
            let log = input(&fixture, &[raw("600001")]);
            let (provider, _) = model(&log, Mode::Good);
            freeze_member(&fixture, &provider, 0).await;
            archive_model_observations_v2(owner(&fixture), date())
                .unwrap()
                .unwrap();
            capture_snapshots(&fixture);
            if already_owned {
                prepare_model_owner_v2_for_test(owner(&fixture), date(), 0, 1, clock("15:12:00"))
                    .unwrap();
            }
            let before = snapshot(&fixture);
            let hit = Arc::new(AtomicUsize::new(0));
            arm_sql_phase_fault(
                Arc::downgrade(&owner(&fixture)),
                phase,
                2,
                Arc::clone(&hit),
                OperationPostvalidationTestFault::G5bHeadRevisionAdvance,
            );
            let error =
                prepare_model_owner_v2_for_test(owner(&fixture), date(), 0, 1, clock("15:12:01"))
                    .unwrap_err()
                    .to_string();
            assert_eq!(
                hit.load(Ordering::SeqCst),
                1,
                "must hit the real decision/owner TX after SQL"
            );
            assert!(
                error.contains("revision differs from own mutation effect"),
                "{error}"
            );
            assert_eq!(
                snapshot(&fixture),
                before,
                "{phase:?}, existing={already_owned}"
            );
        }
    }
}

fn arm_post_commit_revision(
    weak: std::sync::Weak<DurableDeliveryCoordinator>,
    path: PathBuf,
    remaining: usize,
    hit: Arc<AtomicUsize>,
) {
    let coordinator = weak.upgrade().unwrap();
    coordinator.install_database_operation_test_hook(DatabaseOperationTestPhase::AfterCommitBeforePostValidation,move|| {
        if remaining==1 {
            let connection=Connection::open(&path)?;
            assert_eq!(connection.execute("UPDATE g5b_day_heads SET revision=revision+1,current_seal_identity=NULL",[])?,1);
            hit.fetch_add(1,Ordering::SeqCst);
        } else {arm_post_commit_revision(weak,path,remaining-1,hit);}
        Ok(())
    }).unwrap();
}
#[tokio::test]
async fn g5b_v2_owner_post_commit_sql_drift_reports_committed_fact_and_does_not_rollback() {
    let fixture = Fixture::new("C2_POST_COMMIT_REVISION");
    let log = input(&fixture, &[raw("600001")]);
    let (provider, _) = model(&log, Mode::Good);
    freeze_member(&fixture, &provider, 0).await;
    archive_model_observations_v2(owner(&fixture), date())
        .unwrap()
        .unwrap();
    capture_snapshots(&fixture);
    let original = revision(&fixture);
    let hit = Arc::new(AtomicUsize::new(0));
    arm_post_commit_revision(
        Arc::downgrade(&owner(&fixture)),
        fixture.database_path.clone(),
        2,
        Arc::clone(&hit),
    );
    let error = prepare_model_owner_v2_for_test(owner(&fixture), date(), 0, 1, clock("15:12:00"))
        .unwrap_err()
        .to_string();
    assert_eq!(hit.load(Ordering::SeqCst), 1);
    assert!(error.contains("after COMMIT succeeded"), "{error}");
    assert_eq!(owners(&fixture), 1);
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM delivery_decisions"),
        1
    );
    assert_eq!(
        revision(&fixture),
        original + 2,
        "the actual admitted owner and independent later SQL change remain facts"
    );
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM delivery_attempts"),
        0
    );
}

#[test]
fn g5b_v2_owner_malformed_legacy_without_cohort_stays_unknown_and_preserves_other_kind_admission() {
    let fixture = Fixture::new("C2_LEGACY_UNKNOWN");
    let _log = fixture.g5b_input_log(DATE);
    let legacy = valid_legacy_extra();
    fixture
        .coordinator
        .prepare(&legacy, 1, clock("15:12:00"))
        .unwrap();
    let unknown = vec![0xff, 0x80, b'\n'];
    let sha = sha256_hex(&unknown);
    let connection = Connection::open(&fixture.database_path).unwrap();
    // The ordinary immutable envelope guard must reject this write. Inject
    // historical corruption only in this isolated Test database, then restore
    // the exact original trigger before exercising any runtime operation.
    assert!(connection.execute("UPDATE delivery_decisions SET envelope_canonical=?1,envelope_sha256=?2 WHERE decision_identity=?3",params![unknown,sha,legacy.decision_identity]).is_err());
    let trigger: String = connection.query_row(
        "SELECT sql FROM sqlite_master WHERE type='trigger' AND name='immutable_decision_envelope_update'",
        [], |row| row.get(0),
    ).unwrap();
    connection
        .execute_batch("BEGIN IMMEDIATE; DROP TRIGGER immutable_decision_envelope_update;")
        .unwrap();
    assert_eq!(connection.execute("UPDATE delivery_decisions SET envelope_canonical=?1,envelope_sha256=?2 WHERE decision_identity=?3",params![unknown,sha,legacy.decision_identity]).unwrap(),1);
    connection.execute_batch(&trigger).unwrap();
    connection.execute_batch("COMMIT;").unwrap();
    assert_eq!(connection.query_row::<String, _, _>(
        "SELECT sql FROM sqlite_master WHERE type='trigger' AND name='immutable_decision_envelope_update'",
        [], |row| row.get(0),
    ).unwrap(), trigger);
    drop(connection);
    let other = envelope(
        "C2_OTHER_KIND_AFTER_UNKNOWN",
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        DATE,
        false,
    );
    fixture
        .coordinator
        .prepare(&other, 1, clock("15:13:00"))
        .unwrap();
    assert_eq!(
        fixture.query_blob(&format!(
            "SELECT envelope_canonical FROM delivery_decisions WHERE decision_identity='{}'",
            legacy.decision_identity
        )),
        unknown
    );
    assert_eq!(
        fixture.query_strings(&format!(
            "SELECT envelope_sha256 FROM delivery_decisions WHERE decision_identity='{}'",
            legacy.decision_identity
        )),
        vec![sha]
    );
    assert_eq!(owners(&fixture), 0);
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM g5b_cohorts"), 0);
    assert!(
        fixture.coordinator.g5b_counted_day_snapshot(DATE).is_err(),
        "old bytes remain locally Unknown, never qualify v2"
    );
    assert_eq!(
        crate::monitor::g5b_analysis_v2::inspect_analysis_cohort_v2(owner(&fixture), date())
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn g5b_v2_owner_post_sql_head_state_drift_is_not_hidden_by_revision_normalization() {
    let fixture = Fixture::new("C2_POST_SQL_STATE_BINDING");
    let log = input(&fixture, &[raw("600001")]);
    let (provider, _) = model(&log, Mode::Good);
    freeze_member(&fixture, &provider, 0).await;
    archive_model_observations_v2(owner(&fixture), date())
        .unwrap()
        .unwrap();
    capture_snapshots(&fixture);
    prepare_model_owner_v2_for_test(owner(&fixture), date(), 0, 1, clock("15:12:00")).unwrap();
    let before = snapshot(&fixture);
    let hit = Arc::new(AtomicUsize::new(0));
    arm_sql_phase_fault(
        Arc::downgrade(&owner(&fixture)),
        DatabaseOperationTestPhase::AfterSqlBeforePostValidation,
        2,
        Arc::clone(&hit),
        OperationPostvalidationTestFault::G5bHeadArtifactStateDrift,
    );
    let error = prepare_model_owner_v2_for_test(owner(&fixture), date(), 0, 1, clock("15:12:01"))
        .unwrap_err()
        .to_string();
    assert_eq!(hit.load(Ordering::SeqCst), 1);
    assert!(
        error.contains("actual model bundle SQL snapshot changed"),
        "{error}"
    );
    assert_eq!(
        snapshot(&fixture),
        before,
        "legal Dirty head cannot silently replace the captured head"
    );
}
