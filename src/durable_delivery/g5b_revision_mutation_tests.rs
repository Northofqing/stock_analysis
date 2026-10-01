//! D1 changes only transaction effects; these fixtures mint no seal/Empty authority.
#![cfg(unix)]
use super::*;
use chrono::NaiveDate;

const DATE: &str = "2026-08-18";

struct NoModelCall;
#[async_trait::async_trait]
impl crate::llm::LlmProvider for NoModelCall {
    fn name(&self) -> &'static str {
        "TEST_CODE_D1_CONFIGURED_ONLY"
    }
    fn model(&self) -> &str {
        "TEST_CODE_D1_NOT_CALLED"
    }
    async fn chat_json(
        &self,
        _: &str,
        _: &str,
    ) -> std::result::Result<serde_json::Value, crate::llm::LlmError> {
        panic!("D1 revision mutation must not call a model")
    }
}

fn clock(date: &str, local: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(&format!("{date}T{local}+08:00"))
        .unwrap()
        .with_timezone(&Utc)
}

fn prospective_head(fixture: &Fixture, date: &str) {
    let day = NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap();
    let log = fixture.g5b_input_log(date);
    log.initialize_date_input_head(day).unwrap();
    let parent = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(&fixture.database_path)
        .parent()
        .unwrap()
        .to_owned();
    fixture.cleanup.record(
        parent.join(format!("{}.input-head.v1.json", day.format("%Y%m%d"))),
        OwnedPathKind::FileOrSymlink,
    );
    fixture
        .coordinator
        .g5b_day_session(day)
        .unwrap()
        .prospective_for_test(clock(date, "09:00:00"))
        .unwrap();
}

fn revision(fixture: &Fixture, date: &str) -> i64 {
    Connection::open(&fixture.database_path)
        .unwrap()
        .query_row(
            "SELECT revision FROM g5b_day_heads WHERE business_date=?1",
            [date],
            |row| row.get(0),
        )
        .unwrap()
}

fn snapshot_at(path: &Path) -> BTreeMap<String, Vec<Vec<String>>> {
    let connection = Connection::open(path).unwrap();
    let mut statement = connection
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .unwrap();
    let names = statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    authority_snapshot(
        &connection,
        &names.iter().map(String::as_str).collect::<Vec<_>>(),
    )
}

fn snapshot(fixture: &Fixture) -> BTreeMap<String, Vec<Vec<String>>> {
    snapshot_at(&fixture.database_path)
}

fn changed<T>(fixture: &Fixture, operation: impl FnOnce() -> T) -> T {
    let before_revision = revision(fixture, DATE);
    let before = snapshot(fixture);
    let value = operation();
    assert_eq!(revision(fixture, DATE), before_revision + 1);
    assert_ne!(snapshot(fixture), before);
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM g5b_day_seals"), 0);
    assert_eq!(
        fixture.query_i64(
            "SELECT COUNT(*) FROM g5b_day_heads WHERE current_seal_identity IS NOT NULL"
        ),
        0
    );
    value
}

fn unchanged<T>(fixture: &Fixture, operation: impl FnOnce() -> T) -> T {
    let before = snapshot(fixture);
    let value = operation();
    assert_eq!(
        snapshot(fixture),
        before,
        "no-op retains every typed SQLite cell"
    );
    value
}

// Public reconcile contains several independent acknowledgements/finalizers.
// Observe committed real rows through another connection, never reenter the
// coordinator's retained mutex from its hook. No production/test SQL is written.
fn arm_commit_trace(
    weak: std::sync::Weak<DurableDeliveryCoordinator>,
    path: PathBuf,
    previous: Arc<Mutex<(i64, BTreeMap<String, Vec<Vec<String>>>)>>,
    trace: Arc<Mutex<Vec<BTreeSet<String>>>>,
) {
    let coordinator = weak.upgrade().unwrap();
    coordinator
        .install_database_operation_test_hook(
            DatabaseOperationTestPhase::AfterCommitBeforePostValidation,
            move || {
                let connection = Connection::open(&path).unwrap();
                let current_revision: i64 = connection
                    .query_row("SELECT revision FROM g5b_day_heads", [], |row| row.get(0))
                    .unwrap();
                drop(connection);
                let mut current = snapshot_at(&path);
                current.remove("g5b_day_heads");
                let mut last = previous.lock().unwrap();
                let deltas = current
                    .iter()
                    .filter(|(name, rows)| last.1.get(*name) != Some(*rows))
                    .map(|(name, _)| name.clone())
                    .collect::<BTreeSet<_>>();
                assert_eq!(current_revision, last.0 + i64::from(!deltas.is_empty()));
                *last = (current_revision, current);
                trace.lock().unwrap().push(deltas);
                drop(last);
                arm_commit_trace(weak, path, previous, trace);
                Ok(())
            },
        )
        .unwrap();
}

fn start_trace(fixture: &Fixture) -> Arc<Mutex<Vec<BTreeSet<String>>>> {
    let mut rows = snapshot(fixture);
    rows.remove("g5b_day_heads");
    let previous = Arc::new(Mutex::new((revision(fixture, DATE), rows)));
    let trace = Arc::new(Mutex::new(Vec::new()));
    arm_commit_trace(
        Arc::downgrade(&fixture_coordinator_arc(fixture)),
        fixture.database_path.clone(),
        previous,
        trace.clone(),
    );
    trace
}

#[test]
fn g5b_revision_mutation_prepare_attempt_heartbeat_result_have_one_effect_and_exact_noops() {
    let fixture = Fixture::new("REVISION_DIRECT");
    prospective_head(&fixture, DATE);
    let candidate = g5b_frozen_envelope("REVISION_DIRECT", false);
    changed(&fixture, || {
        fixture.coordinator.prepare(&candidate, 1, now()).unwrap()
    });
    unchanged(&fixture, || {
        fixture.coordinator.prepare(&candidate, 1, now()).unwrap()
    });
    let attempt = changed(&fixture, || {
        fixture
            .coordinator
            .begin_attempt(&candidate.decision_identity, 1, now())
            .unwrap()
            .unwrap()
    });
    unchanged(&fixture, || {
        assert!(fixture
            .coordinator
            .begin_attempt(&candidate.decision_identity, 1, now())
            .unwrap()
            .is_none());
    });
    let beat = now() + chrono::Duration::seconds(30);
    changed(&fixture, || {
        assert!(fixture
            .coordinator
            .heartbeat_attempt(
                &candidate.decision_identity,
                &attempt.attempt_identity,
                attempt.fence_token,
                beat
            )
            .unwrap());
    });
    unchanged(&fixture, || {
        assert!(fixture
            .coordinator
            .heartbeat_attempt(
                &candidate.decision_identity,
                &attempt.attempt_identity,
                attempt.fence_token,
                beat
            )
            .unwrap());
        assert!(!fixture
            .coordinator
            .heartbeat_attempt(
                &candidate.decision_identity,
                &attempt.attempt_identity,
                attempt.fence_token + 1,
                beat
            )
            .unwrap());
    });
    assert_eq!(
        fixture.query_i64(
            "SELECT COUNT(*) FROM delivery_attempt_events WHERE event_kind='LeaseHeartbeat'"
        ),
        1
    );
    let accepted = AuthoritativeSinkResult::Accepted(receipt(beat));
    changed(&fixture, || {
        fixture
            .coordinator
            .record_sink_result(
                &attempt.attempt_identity,
                attempt.fence_token,
                accepted.clone(),
                beat,
            )
            .unwrap()
    });
    unchanged(&fixture, || {
        assert!(fixture
            .coordinator
            .record_sink_result(
                &attempt.attempt_identity,
                attempt.fence_token,
                accepted,
                beat
            )
            .is_err());
    });
    assert_eq!(revision(&fixture, DATE), 4);
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM sink_results WHERE authoritative_for_state=1 AND late_after_fence=0"), 1);
}

#[test]
fn g5b_revision_mutation_stored_date_conflict_audit_commits_before_public_error() {
    let fixture = Fixture::new("REVISION_CONFLICT");
    prospective_head(&fixture, DATE);
    prospective_head(&fixture, "2026-08-19");
    let candidate = g5b_frozen_envelope("REVISION_CONFLICT", false);
    changed(&fixture, || {
        fixture.coordinator.prepare(&candidate, 1, now()).unwrap()
    });
    let original = fixture.query_blob("SELECT envelope_canonical FROM delivery_decisions");
    let mut incoming = envelope(
        "REVISION_FOREIGN",
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        "2026-08-19",
        false,
    );
    incoming.decision_identity = candidate.decision_identity.clone();
    changed(&fixture, || {
        assert!(matches!(
            fixture.coordinator.prepare(&incoming, 1, now()),
            Err(DurableDeliveryError::DecisionIdentityConflict { .. })
        ));
    });
    assert_eq!(revision(&fixture, "2026-08-19"), 0);
    assert_eq!(
        fixture.query_blob("SELECT envelope_canonical FROM delivery_decisions"),
        original
    );
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM immutable_audit_outbox WHERE audit_kind='DecisionIdentityConflict'"), 1);
}

#[test]
fn g5b_revision_mutation_reconcile_acks_finalizer_and_hydration_advance_each_transaction_once() {
    let fixture = Fixture::new("REVISION_ACKS");
    prospective_head(&fixture, DATE);
    let mut candidate = g5b_frozen_envelope("REVISION_ACKS", false);
    candidate.task_binding = Some(
        TaskBinding::new(
            "TEST_CODE_REVISION_TASK",
            b"TEST_CODE_REVISION_BASIS".to_vec(),
        )
        .unwrap(),
    );
    candidate.validate().unwrap();
    let append = MemoryAppendPort::default();
    fixture.coordinator.prepare(&candidate, 1, now()).unwrap();
    let attempt = fixture
        .coordinator
        .begin_attempt(&candidate.decision_identity, 1, now())
        .unwrap()
        .unwrap();
    fixture
        .coordinator
        .record_sink_result(
            &attempt.attempt_identity,
            attempt.fence_token,
            AuthoritativeSinkResult::Accepted(receipt(now())),
            now(),
        )
        .unwrap();
    let trace = start_trace(&fixture);
    let summary = fixture
        .coordinator
        .reconcile_all_pending(&append, now())
        .unwrap();
    assert_eq!(
        fixture
            .coordinator
            .decision_state(&candidate.decision_identity)
            .unwrap(),
        DecisionState::Delivered
    );
    assert_eq!(summary.schedule_hydrations.len(), 1);
    let deltas = trace.lock().unwrap().clone();
    for required in [
        "immutable_audit_outbox",
        "delivery_disposition_payloads",
        "sink_results",
        "task_transition_payloads",
        "delivery_decisions",
    ] {
        assert!(
            deltas.iter().any(|delta| delta.contains(required)),
            "no actual {required} acknowledgement/finalizer was observed"
        );
    }
    let hydration = &summary.schedule_hydrations[0];
    changed(&fixture, || {
        assert!(fixture
            .coordinator
            .acknowledge_schedule_hydration(
                &hydration.transition_identity,
                &hydration.transition_sha256,
                now()
            )
            .unwrap())
    });
    unchanged(&fixture, || {
        assert!(!fixture
            .coordinator
            .acknowledge_schedule_hydration(
                &hydration.transition_identity,
                &hydration.transition_sha256,
                now()
            )
            .unwrap())
    });
    fixture
        .coordinator
        .reconcile_all_pending(&append, now())
        .unwrap();
    unchanged(&fixture, || {
        fixture
            .coordinator
            .reconcile_all_pending(&append, now())
            .unwrap()
    });
    assert_eq!(
        fixture
            .query_i64("SELECT COUNT(*) FROM immutable_audit_outbox WHERE append_state='Pending'"),
        0
    );
}

#[test]
fn g5b_revision_mutation_retry_authorization_reacquire_and_invalid_sink_are_real_effects() {
    let fixture = Fixture::new("REVISION_RETRY");
    prospective_head(&fixture, DATE);
    let candidate = g5b_frozen_envelope("REVISION_RETRY", false);
    let append = MemoryAppendPort::default();
    fixture.coordinator.prepare(&candidate, 1, now()).unwrap();
    let attempt = fixture
        .coordinator
        .begin_attempt(&candidate.decision_identity, 1, now())
        .unwrap()
        .unwrap();
    fixture
        .coordinator
        .record_sink_result(
            &attempt.attempt_identity,
            attempt.fence_token,
            AuthoritativeSinkResult::Rejected(rejection(now(), false)),
            now(),
        )
        .unwrap();
    fixture
        .coordinator
        .reconcile_all_pending(&append, now())
        .unwrap();
    changed(&fixture, || {
        fixture
            .coordinator
            .authorize_rejected_retry(&candidate.decision_identity)
            .unwrap()
    });
    unchanged(&fixture, || {
        fixture
            .coordinator
            .authorize_rejected_retry(&candidate.decision_identity)
            .unwrap()
    });
    let trace = start_trace(&fixture);
    let sink = StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(now())));
    let sinks: [AuthoritativeSink; 1] = [sink.clone()];
    assert_eq!(
        fixture
            .coordinator
            .resume_deliverable(&candidate.decision_identity, &sinks, now())
            .unwrap()
            .sink_calls,
        1
    );
    assert_eq!(sink.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        trace
            .lock()
            .unwrap()
            .iter()
            .filter(|delta| !delta.is_empty())
            .count(),
        3,
        "reacquire, begin and raw result are separate single-effect transactions"
    );
    let denied = g5b_frozen_envelope("REVISION_BAD_SINK", false);
    changed(&fixture, || {
        fixture.coordinator.prepare(&denied, 1, now()).unwrap()
    });
    changed(&fixture, || {
        assert!(fixture
            .coordinator
            .begin_attempt(&denied.decision_identity, 0, now())
            .unwrap()
            .is_none())
    });
    assert_eq!(
        fixture
            .coordinator
            .decision_state(&denied.decision_identity)
            .unwrap(),
        DecisionState::RejectedAuditPending
    );
}

#[test]
fn g5b_revision_mutation_expiry_distinct_late_raw_and_manual_preserve_original_evidence() {
    let fixture = Fixture::new("REVISION_LATE_MANUAL");
    prospective_head(&fixture, DATE);
    let candidate = g5b_frozen_envelope("REVISION_LATE_MANUAL", false);
    let append = MemoryAppendPort::default();
    fixture.coordinator.prepare(&candidate, 1, now()).unwrap();
    let attempt = fixture
        .coordinator
        .begin_attempt(&candidate.decision_identity, 1, now())
        .unwrap()
        .unwrap();
    let recovery = now() + chrono::Duration::seconds(121);
    let trace = start_trace(&fixture);
    fixture
        .coordinator
        .reconcile_all_pending(&append, recovery)
        .unwrap();
    assert!(
        trace
            .lock()
            .unwrap()
            .iter()
            .any(|delta| delta.contains("delivery_attempts")
                && delta.contains("delivery_attempt_events")),
        "real expired attempt revocation must be observed"
    );
    assert_eq!(
        fixture
            .coordinator
            .decision_state(&candidate.decision_identity)
            .unwrap(),
        DecisionState::UncertainManualReview
    );
    unchanged(&fixture, || {
        fixture
            .coordinator
            .reconcile_all_pending(&append, recovery)
            .unwrap()
    });
    let first = AuthoritativeSinkResult::Accepted(receipt(recovery));
    let second =
        AuthoritativeSinkResult::Uncertain(uncertainty(recovery + chrono::Duration::seconds(1)));
    let expected = [first.clone(), second.clone()].map(|result| {
        serde_json::to_vec(&match result {
            AuthoritativeSinkResult::Accepted(receipt) => {
                serde_json::json!({"kind":"Accepted","receipt":receipt})
            }
            AuthoritativeSinkResult::Uncertain(uncertainty) => {
                serde_json::json!({"kind":"Uncertain","uncertainty":uncertainty})
            }
            _ => unreachable!(),
        })
        .unwrap()
    });
    changed(&fixture, || {
        fixture
            .coordinator
            .record_sink_result(
                &attempt.attempt_identity,
                attempt.fence_token,
                first,
                recovery,
            )
            .unwrap()
    });
    changed(&fixture, || {
        fixture
            .coordinator
            .record_sink_result(
                &attempt.attempt_identity,
                attempt.fence_token,
                second,
                recovery,
            )
            .unwrap()
    });
    let connection = Connection::open(&fixture.database_path).unwrap();
    let mut statement = connection.prepare("SELECT result_canonical,result_sha256,authoritative_for_state,late_after_fence FROM sink_results ORDER BY rowid").unwrap();
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(rows.len(), 2);
    for (row, expected) in rows.iter().zip(expected) {
        assert_eq!(row.0, expected);
        assert_eq!(row.1, sha256_hex(&expected));
        assert_eq!((row.2, row.3), (0, 1));
    }
    drop(statement);
    drop(connection);
    let command = ManualResolutionCommand {
        decision_identity: candidate.decision_identity.clone(),
        disposition: ManualDisposition::Accepted { receipt: None },
        operator_identity: "TEST_CODE_REVISION_OPERATOR".to_owned(),
        reason: "TEST_CODE_REVISION_MANUAL".to_owned(),
        external_evidence: b"TEST_CODE_EXTERNAL_MANUAL".to_vec(),
        resolved_at: recovery,
    };
    changed(&fixture, || {
        fixture
            .coordinator
            .resolve_uncertain(&command, &append)
            .unwrap()
    });
    fixture
        .coordinator
        .reconcile_all_pending(&append, recovery)
        .unwrap();
    assert_eq!(
        fixture
            .coordinator
            .decision_state(&candidate.decision_identity)
            .unwrap(),
        DecisionState::Delivered
    );
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM sink_results WHERE authoritative_for_state=1"),
        0,
        "manual acceptance does not promote late raw"
    );
}

#[test]
fn g5b_revision_mutation_rollbacks_and_postcommit_error_retain_the_actual_effect_boundary() {
    for phase in [
        DatabaseOperationTestPhase::AfterSqlBeforePreCommitValidation,
        DatabaseOperationTestPhase::AfterSqlBeforePostValidation,
        DatabaseOperationTestPhase::AfterCommitBeforePostValidation,
    ] {
        let fixture = Fixture::new("REVISION_FAULT");
        prospective_head(&fixture, DATE);
        let candidate = g5b_frozen_envelope("REVISION_FAULT", false);
        let before = snapshot(&fixture);
        fixture
            .coordinator
            .install_database_operation_test_hook(phase, || {
                Err(DurableDeliveryError::InvalidConfiguration(
                    "TEST_CODE_REVISION_FAULT".to_owned(),
                ))
            })
            .unwrap();
        let error = fixture
            .coordinator
            .prepare(&candidate, 1, now())
            .unwrap_err();
        if matches!(
            phase,
            DatabaseOperationTestPhase::AfterCommitBeforePostValidation
        ) {
            assert!(error.to_string().contains("after COMMIT succeeded"));
            assert_eq!(revision(&fixture, DATE), 1);
            assert_eq!(
                fixture.query_i64("SELECT COUNT(*) FROM delivery_decisions"),
                1
            );
            unchanged(&fixture, || {
                fixture.coordinator.prepare(&candidate, 1, now()).unwrap()
            });
        } else {
            assert_eq!(snapshot(&fixture), before);
            assert_eq!(revision(&fixture, DATE), 0);
        }
    }
}

#[test]
fn g5b_revision_mutation_non_g5b_and_legacy_without_head_do_not_adopt_the_day() {
    let fixture = Fixture::new("REVISION_COMPAT");
    prospective_head(&fixture, DATE);
    let before = authority_table_rows(
        &Connection::open(&fixture.database_path).unwrap(),
        "g5b_day_heads",
    );
    let ordinary = envelope(
        "REVISION_ORDINARY",
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        DATE,
        false,
    );
    fixture.coordinator.prepare(&ordinary, 1, now()).unwrap();
    assert_eq!(
        authority_table_rows(
            &Connection::open(&fixture.database_path).unwrap(),
            "g5b_day_heads"
        ),
        before
    );
    let legacy = Fixture::new("REVISION_NO_HEAD");
    let candidate = g5b_frozen_envelope("REVISION_NO_HEAD", false);
    legacy.g5b_input_log(DATE);
    legacy.coordinator.prepare(&candidate, 1, now()).unwrap();
    let attempt = legacy
        .coordinator
        .begin_attempt(&candidate.decision_identity, 1, now())
        .unwrap()
        .unwrap();
    legacy
        .coordinator
        .record_sink_result(
            &attempt.attempt_identity,
            attempt.fence_token,
            AuthoritativeSinkResult::Accepted(receipt(now())),
            now(),
        )
        .unwrap();
    legacy
        .coordinator
        .reconcile_all_pending(&MemoryAppendPort::default(), now())
        .unwrap();
    assert_eq!(legacy.query_i64("SELECT COUNT(*) FROM g5b_day_heads"), 0);
    assert_eq!(
        legacy
            .coordinator
            .decision_state(&candidate.decision_identity)
            .unwrap(),
        DecisionState::Delivered
    );
}

#[test]
fn g5b_revision_mutation_business_write_preserves_unmatched_artifact_dirty_state() {
    let fixture = Fixture::new("REVISION_DIRTY");
    prospective_head(&fixture, DATE);
    let day = NaiveDate::parse_from_str(DATE, "%Y-%m-%d").unwrap();
    let log = fixture.g5b_input_log(DATE);
    let record: crate::monitor::alert_log::AlertRecord =
        serde_json::from_value(serde_json::json!({
            "origin":"production", "triggered_at":"2026-08-18T15:00:00+08:00",
            "code":"600001", "name":"TEST_CODE_D1", "level":"重要", "category":"TEST_CODE_D1",
            "message":"TEST_CODE_D1_INPUT", "t1_locked":false
        }))
        .unwrap();
    let mut raw = serde_json::to_vec(&record).unwrap();
    raw.push(b'\n');
    log.append_test_date_raw_production_fixture(day, &raw)
        .unwrap();
    let parent = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(&fixture.database_path)
        .parent()
        .unwrap()
        .to_owned();
    fixture
        .cleanup
        .record(parent.join("20260818.jsonl"), OwnedPathKind::FileOrSymlink);
    fixture.cleanup.record(
        parent.join("20260818.input-head.v1.json"),
        OwnedPathKind::FileOrSymlink,
    );
    {
        let session = fixture.coordinator.g5b_day_session(day).unwrap();
        let ready = session
            .configured_analysis_for_test(Arc::new(NoModelCall), clock(DATE, "15:10:00"))
            .unwrap();
        session.prepare_cohort(&ready).unwrap();
    }
    assert_eq!(revision(&fixture, DATE), 1);
    // This is the existing v1 mutation route under D1. It grants no v2 member,
    // owner, model/handoff or seal qualification; C2 closes new v1 admissions.
    let candidate = g5b_frozen_envelope("REVISION_DIRTY_LEGACY", false);
    changed(&fixture, || {
        fixture.coordinator.prepare(&candidate, 1, now()).unwrap()
    });
    assert_eq!(
        fixture.query_strings("SELECT artifact_state FROM g5b_day_heads"),
        vec!["Dirty"]
    );
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM g5b_artifact_events WHERE phase='Prepared'"),
        1
    );
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM g5b_artifact_events WHERE phase='Committed'"),
        0
    );
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM g5b_occurrence_owners"),
        0
    );
}
