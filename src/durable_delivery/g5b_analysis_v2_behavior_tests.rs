//! C1 runs only against the attested isolated Test store and actual input writer.
#![cfg(unix)]
use super::*;
use crate::llm::{LlmError, LlmProvider, ReceiptBearingJson};
use crate::monitor::alert_log::{AlertLog, AlertRecord};
use crate::monitor::g5b_analysis_v2::{
    claim_analysis_v2, claim_for_test, G5bAnalysisClaimV2, G5bAnalysisWorkV2,
};
use crate::monitor::g5b_selection_v2::G5bSelectionEvidence;
use chrono::NaiveDate;

const DATE: &str = "2026-09-28";
const CONTENT: &str = "{\n  \"risk_note\": \"证据有限\",\n  \"confidence\": \"low\",\n  \"capital_logic\": \"观测资金\",\n  \"catalyst_chain\": [\"第一条\"],\n  \"main_reason\": \"模型原文\"\n}";
fn date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 9, 28).unwrap()
}
fn clock(time: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(&format!("{DATE}T{time}+08:00"))
        .unwrap()
        .with_timezone(&Utc)
}
fn owner(fixture: &Fixture) -> Arc<DurableDeliveryCoordinator> {
    Arc::clone(fixture.coordinator.0.as_ref().unwrap())
}
fn namespace(fixture: &Fixture) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(&fixture.database_path)
        .parent()
        .unwrap()
        .to_owned()
}
fn raw(code: &str) -> Vec<u8> {
    let record: AlertRecord = serde_json::from_value(serde_json::json!({
        "origin":"production", "triggered_at":format!("{DATE}T15:00:00+08:00"),
        "code":code, "name":"TEST_CODE_C1", "level":"重要", "category":"TEST_CODE_C1",
        "message":"exact observed occurrence", "t1_locked":false
    }))
    .unwrap();
    let mut bytes = serde_json::to_vec(&record).unwrap();
    bytes.push(b'\n');
    bytes
}
fn input(fixture: &Fixture, lines: &[Vec<u8>]) -> AlertLog {
    let log = fixture.g5b_input_log(DATE);
    log.initialize_date_input_head(date()).unwrap();
    fixture.cleanup.record(
        namespace(fixture).join("20260928.input-head.v1.json"),
        OwnedPathKind::FileOrSymlink,
    );
    for line in lines {
        log.append_test_date_raw_production_fixture(date(), line)
            .unwrap();
        fixture.cleanup.record(
            namespace(fixture).join("20260928.jsonl"),
            OwnedPathKind::FileOrSymlink,
        );
        fixture.cleanup.record(
            namespace(fixture).join("20260928.input-head.v1.json"),
            OwnedPathKind::FileOrSymlink,
        );
    }
    log
}
fn capture_snapshots(fixture: &Fixture) {
    // Enumerate only exact immutable intents minted by this fixture. Never
    // sweep a namespace or adopt an unrecognized temporary file for cleanup.
    let connection = Connection::open(&fixture.database_path).unwrap();
    let mut query = connection.prepare("SELECT cohort_identity,logical_intent,artifact_role FROM g5b_artifact_events WHERE phase='Prepared'").unwrap();
    let rows = query
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    for (cohort, intent, role) in rows {
        fixture.cleanup.record_if_present(
            namespace(fixture).join(format!(
                "20260928.{cohort}.{intent}.g5b-{}.v2",
                role.to_ascii_lowercase()
            )),
            OwnedPathKind::FileOrSymlink,
        );
    }
}
fn original_count(fixture: &Fixture, role: &str) -> i64 {
    assert!(matches!(role, "Attempt" | "Frozen"));
    fixture.query_i64(&format!("SELECT COUNT(*) FROM g5b_artifact_events WHERE artifact_role='{role}' AND phase='Prepared'"))
}
fn work(claim: G5bAnalysisClaimV2) -> G5bAnalysisWorkV2 {
    match claim {
        G5bAnalysisClaimV2::Ready(work) => work,
        _ => panic!("expected newly owned work"),
    }
}
fn unproven(claim: G5bAnalysisClaimV2) -> String {
    match claim {
        G5bAnalysisClaimV2::CompletionUnproven {
            occurrence_identity,
        } => occurrence_identity,
        _ => panic!("a saved original attempt must not grant a new model call"),
    }
}

#[derive(Clone, Copy)]
enum Mode {
    Good,
    WrongPrompt,
    DuplicateField,
    UnknownField,
    MissingReceipt,
    PanicIfCalled,
}
struct Model {
    input_log: AlertLog,
    calls: Arc<AtomicUsize>,
    mode: Mode,
}
#[async_trait::async_trait]
impl LlmProvider for Model {
    fn name(&self) -> &'static str {
        "TEST_CODE_C1_REAL_RECEIPT"
    }
    fn model(&self) -> &str {
        "configured-model"
    }
    async fn chat_json(
        &self,
        _: &str,
        _: &str,
    ) -> std::result::Result<serde_json::Value, LlmError> {
        panic!("C1 must use the receipt-bearing adapter")
    }
    async fn chat_json_with_receipt(
        &self,
        system: &str,
        user: &str,
    ) -> std::result::Result<ReceiptBearingJson, LlmError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if matches!(self.mode, Mode::PanicIfCalled) {
            panic!("recovery cannot call a model")
        }
        // A separate physical lock acquisition proves the async model phase
        // does not carry the short owner's guard across the provider await.
        let log = self.input_log.clone();
        let (tx, rx) = mpsc::channel();
        let task = std::thread::spawn(move || {
            let fence = log.acquire_date_writer_fence(date()).unwrap();
            tx.send(()).unwrap();
            drop(fence);
        });
        rx.recv_timeout(std::time::Duration::from_secs(2))
            .expect("model phase released date fence");
        task.join().unwrap();
        if matches!(self.mode, Mode::MissingReceipt) {
            return Err(LlmError::ReceiptUnavailable {
                provider: self.name().to_owned(),
                model: self.model().to_owned(),
            });
        }
        let content = match self.mode {
            Mode::DuplicateField => "{\"main_reason\":\"one\",\"main_reason\":\"two\",\"catalyst_chain\":[],\"capital_logic\":\"c\",\"confidence\":\"low\",\"risk_note\":\"r\"}",
            Mode::UnknownField => "{\"main_reason\":\"one\",\"catalyst_chain\":[],\"capital_logic\":\"c\",\"confidence\":\"low\",\"risk_note\":\"r\",\"caller_authority\":true}",
            _ => CONTENT,
        };
        Ok(ReceiptBearingJson::test_fixture(
            self.name(),
            "reported-model-from-upstream",
            Some("actual-request"),
            "actual-response",
            if matches!(self.mode, Mode::WrongPrompt) {
                "different-actual-system"
            } else {
                system
            },
            user,
            content,
            clock("15:12:00"),
            clock("15:12:01"),
        ))
    }
}
fn model(log: &AlertLog, mode: Mode) -> (Arc<dyn LlmProvider>, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    (
        Arc::new(Model {
            input_log: log.clone(),
            calls: Arc::clone(&calls),
            mode,
        }),
        calls,
    )
}

#[tokio::test]
async fn g5b_analysis_v2_behavior_real_claim_raw_content_unlocked_model_and_no_decision_authority()
{
    let fixture = Fixture::new("C1_REAL_RAW_UNLOCKED");
    let log = input(&fixture, &[raw("600001")]);
    let (provider, calls) = model(&log, Mode::Good);
    let claim = claim_for_test(owner(&fixture), date(), 0, provider, clock("15:10:00")).unwrap();
    capture_snapshots(&fixture);
    assert_eq!(original_count(&fixture, "Attempt"), 1);
    let completed = work(claim).assess().await.unwrap();
    let frozen = completed.freeze().unwrap();
    capture_snapshots(&fixture);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(frozen.model_content_utf8(), CONTENT.as_bytes());
    assert_eq!(original_count(&fixture, "Frozen"), 1);
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM delivery_decisions"),
        0
    );
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM immutable_audit_outbox"),
        0
    );
    let bytes = frozen.canonical_bytes().to_vec();
    let recovered_owner = fixture.second_coordinator("C1_RECOVERY");
    let (forbidden, recovery_calls) = model(&log, Mode::PanicIfCalled);
    let recovered =
        claim_for_test(recovered_owner, date(), 0, forbidden, clock("16:00:00")).unwrap();
    let G5bAnalysisClaimV2::Frozen(recovered) = recovered else {
        panic!("expected actual committed observation")
    };
    assert_eq!(recovered.canonical_bytes(), bytes);
    assert_eq!(recovered.model_content_utf8(), CONTENT.as_bytes());
    assert_eq!(recovery_calls.load(Ordering::SeqCst), 0);
    // Production facade also observes this actual saved result without
    // needing today's date/window or selecting a newly configured provider.
    assert!(matches!(
        claim_analysis_v2(owner(&fixture), date(), 0).unwrap(),
        G5bAnalysisClaimV2::Frozen(_)
    ));
}

#[tokio::test]
async fn g5b_analysis_v2_behavior_identical_raw_occurrences_each_consume_only_one_original_attempt()
{
    let fixture = Fixture::new("C1_OCCURRENCE_COST");
    let raw = raw("600001");
    let log = input(&fixture, &[raw.clone(), raw]);
    let (provider, calls) = model(&log, Mode::Good);
    let first = work(
        claim_for_test(
            owner(&fixture),
            date(),
            0,
            Arc::clone(&provider),
            clock("15:10:00"),
        )
        .unwrap(),
    );
    capture_snapshots(&fixture);
    let blocked = claim_for_test(
        owner(&fixture),
        date(),
        0,
        Arc::clone(&provider),
        clock("15:11:00"),
    )
    .unwrap();
    let first_id = unproven(blocked);
    let second = work(
        claim_for_test(
            owner(&fixture),
            date(),
            1,
            Arc::clone(&provider),
            clock("15:10:00"),
        )
        .unwrap(),
    );
    capture_snapshots(&fixture);
    let second_id = unproven(
        claim_for_test(
            owner(&fixture),
            date(),
            1,
            Arc::clone(&provider),
            clock("15:11:00"),
        )
        .unwrap(),
    );
    assert_ne!(first_id, second_id);
    let first = first.assess().await.unwrap().freeze().unwrap();
    capture_snapshots(&fixture);
    let second = second.assess().await.unwrap().freeze().unwrap();
    capture_snapshots(&fixture);
    assert_ne!(first.occurrence_identity(), second.occurrence_identity());
    assert_ne!(first.canonical_bytes(), second.canonical_bytes());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(original_count(&fixture, "Attempt"), 2);
    assert_eq!(original_count(&fixture, "Frozen"), 2);
    assert!(matches!(
        claim_for_test(owner(&fixture), date(), 0, provider, clock("15:20:59")).unwrap(),
        G5bAnalysisClaimV2::Frozen(_)
    ));
    assert_eq!(original_count(&fixture, "Attempt"), 2);
}

#[test]
fn g5b_analysis_v2_behavior_dropped_live_work_cannot_be_reminted_with_a_new_clock_or_owner() {
    let fixture = Fixture::new("C1_DROPPED_WORK");
    let log = input(&fixture, &[raw("600001")]);
    let (provider, calls) = model(&log, Mode::PanicIfCalled);
    let live = work(
        claim_for_test(
            owner(&fixture),
            date(),
            0,
            Arc::clone(&provider),
            clock("15:10:00"),
        )
        .unwrap(),
    );
    capture_snapshots(&fixture);
    drop(live);
    let first = unproven(
        claim_for_test(
            fixture.second_coordinator("C1_NEW_OWNER"),
            date(),
            0,
            provider,
            clock("15:11:00"),
        )
        .unwrap(),
    );
    let again = unproven(claim_analysis_v2(owner(&fixture), date(), 0).unwrap());
    assert_eq!(first, again);
    assert_eq!(original_count(&fixture, "Attempt"), 1);
    assert_eq!(original_count(&fixture, "Frozen"), 0);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn g5b_analysis_v2_behavior_receipt_or_strict_model_failure_leaves_attempt_consumed_without_frozen(
) {
    for (label, mode) in [
        ("WRONG_PROMPT", Mode::WrongPrompt),
        ("DUPLICATE", Mode::DuplicateField),
        ("UNKNOWN", Mode::UnknownField),
        ("NO_RECEIPT", Mode::MissingReceipt),
    ] {
        let fixture = Fixture::new(&format!("C1_FAIL_{label}"));
        let log = input(&fixture, &[raw("600001")]);
        let (provider, calls) = model(&log, mode);
        let live =
            work(claim_for_test(owner(&fixture), date(), 0, provider, clock("15:10:00")).unwrap());
        capture_snapshots(&fixture);
        assert!(live.assess().await.is_err(), "{label}");
        let (forbidden, recovery_calls) = model(&log, Mode::PanicIfCalled);
        unproven(claim_for_test(owner(&fixture), date(), 0, forbidden, clock("15:11:00")).unwrap());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(recovery_calls.load(Ordering::SeqCst), 0);
        assert_eq!(original_count(&fixture, "Attempt"), 1);
        assert_eq!(original_count(&fixture, "Frozen"), 0);
        assert_eq!(
            fixture.query_i64("SELECT COUNT(*) FROM delivery_decisions"),
            0
        );
    }
}

#[test]
fn g5b_analysis_v2_behavior_prepared_attempt_is_not_a_call_and_cannot_change_desired_bits() {
    let fixture = Fixture::new("C1_PREPARED_COST_GUARD");
    let log = input(&fixture, &[raw("600001")]);
    let (provider, calls) = model(&log, Mode::PanicIfCalled);
    let session = fixture.coordinator.g5b_day_session(date()).unwrap();
    let ready = session
        .configured_analysis_for_test(Arc::clone(&provider), clock("15:10:00"))
        .unwrap();
    let selection = session.prepare_cohort(&ready).unwrap();
    session.publish_prepared_artifact(&selection).unwrap();
    capture_snapshots(&fixture);
    session.commit_prepared_artifact(&selection).unwrap();
    let cohort = session.read_cohort().unwrap().unwrap();
    let opaque = b"opaque bytes are not an authorized model result";
    let first = session
        .prepare_opaque_snapshot(&cohort, G5bSnapshotKind::Attempt, Some(0), opaque)
        .unwrap();
    let replay = session
        .prepare_opaque_snapshot(&cohort, G5bSnapshotKind::Attempt, Some(0), opaque)
        .unwrap();
    assert_eq!(first.identity(), replay.identity());
    assert!(session
        .prepare_opaque_snapshot(
            &cohort,
            G5bSnapshotKind::Attempt,
            Some(0),
            b"different-clock-and-desired-bytes"
        )
        .is_err());
    assert!(session
        .prepare_new_analysis_attempt(&cohort, 0, opaque, &ready)
        .is_err());
    drop(session);
    unproven(claim_for_test(owner(&fixture), date(), 0, provider, clock("15:11:00")).unwrap());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(original_count(&fixture, "Attempt"), 1);
    assert_eq!(original_count(&fixture, "Frozen"), 0);
}

#[test]
fn g5b_analysis_v2_behavior_window_and_top_three_member_bounds_precede_work() {
    let fixture = Fixture::new("C1_WINDOW_MEMBER_BOUNDS");
    let log = input(
        &fixture,
        &[raw("600001"), raw("600002"), raw("600003"), raw("600004")],
    );
    let (provider, calls) = model(&log, Mode::PanicIfCalled);
    assert!(claim_for_test(
        owner(&fixture),
        date(),
        0,
        Arc::clone(&provider),
        clock("15:04:59")
    )
    .is_err());
    assert!(claim_for_test(
        owner(&fixture),
        date(),
        0,
        Arc::clone(&provider),
        clock("15:21:00")
    )
    .is_err());
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM g5b_cohorts"), 0);
    assert!(claim_for_test(
        owner(&fixture),
        date(),
        3,
        Arc::clone(&provider),
        clock("15:10:00")
    )
    .is_err());
    capture_snapshots(&fixture);
    let evidence = G5bSelectionEvidence::decode(
        &fixture.query_blob("SELECT selection_canonical FROM g5b_cohorts"),
    )
    .unwrap();
    assert_eq!(evidence.encoded().selected.len(), 3);
    assert_eq!(original_count(&fixture, "Attempt"), 0);
    let live =
        work(claim_for_test(owner(&fixture), date(), 2, provider, clock("15:20:59")).unwrap());
    capture_snapshots(&fixture);
    drop(live);
    assert_eq!(original_count(&fixture, "Attempt"), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn g5b_analysis_v2_behavior_delayed_live_work_rechecks_window_and_never_calls_after_close() {
    for delayed in ["2026-09-28T15:21:00+08:00", "2026-09-29T15:10:00+08:00"] {
        let fixture = Fixture::new("C1_DELAYED_WORK");
        let log = input(&fixture, &[raw("600001")]);
        let (provider, calls) = model(&log, Mode::PanicIfCalled);
        let live = work(
            claim_for_test(
                owner(&fixture),
                date(),
                0,
                Arc::clone(&provider),
                clock("15:20:59"),
            )
            .unwrap(),
        );
        capture_snapshots(&fixture);
        let delayed = DateTime::parse_from_rfc3339(delayed)
            .unwrap()
            .with_timezone(&Utc);
        let error = live
            .assess_at_for_test(delayed)
            .await
            .err()
            .expect("fresh invocation gate rejects delayed work");
        assert!(error.to_string().contains("model invocation window closed"));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(original_count(&fixture, "Attempt"), 1);
        assert_eq!(original_count(&fixture, "Frozen"), 0);
        unproven(claim_for_test(owner(&fixture), date(), 0, provider, clock("15:20:59")).unwrap());
        assert_eq!(original_count(&fixture, "Attempt"), 1);
    }
}

#[tokio::test]
async fn g5b_analysis_v2_behavior_live_work_rechecks_actual_original_file_before_model() {
    let fixture = Fixture::new("C1_CHANGED_ORIGINAL_WORK");
    let log = input(&fixture, &[raw("600001")]);
    let (provider, calls) = model(&log, Mode::PanicIfCalled);
    let live =
        work(claim_for_test(owner(&fixture), date(), 0, provider, clock("15:10:00")).unwrap());
    capture_snapshots(&fixture);
    let connection = Connection::open(&fixture.database_path).unwrap();
    let (cohort,intent):(String,String) = connection.query_row("SELECT cohort_identity,logical_intent FROM g5b_artifact_events WHERE artifact_role='Attempt' AND phase='Prepared'",[],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
    drop(connection);
    let path = namespace(&fixture).join(format!("20260928.{cohort}.{intent}.g5b-attempt.v2"));
    std::fs::remove_file(&path).unwrap();
    assert!(live.assess().await.is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(!path.exists());
    assert_eq!(original_count(&fixture, "Attempt"), 1);
    assert_eq!(original_count(&fixture, "Frozen"), 0);
}

#[tokio::test]
async fn g5b_analysis_v2_behavior_committed_frozen_missing_file_is_not_healed_or_recalled() {
    let fixture = Fixture::new("C1_MISSING_FROZEN");
    let log = input(&fixture, &[raw("600001")]);
    let (provider, calls) = model(&log, Mode::Good);
    let live =
        work(claim_for_test(owner(&fixture), date(), 0, provider, clock("15:10:00")).unwrap());
    capture_snapshots(&fixture);
    live.assess().await.unwrap().freeze().unwrap();
    capture_snapshots(&fixture);
    let connection = Connection::open(&fixture.database_path).unwrap();
    let (cohort,intent):(String,String) = connection.query_row("SELECT cohort_identity,logical_intent FROM g5b_artifact_events WHERE artifact_role='Frozen' AND phase='Prepared'",[],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
    drop(connection);
    let path = namespace(&fixture).join(format!("20260928.{cohort}.{intent}.g5b-frozen.v2"));
    std::fs::remove_file(&path).unwrap();
    let (forbidden, recovery_calls) = model(&log, Mode::PanicIfCalled);
    assert!(claim_for_test(owner(&fixture), date(), 0, forbidden, clock("16:00:00")).is_err());
    assert!(!path.exists());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(recovery_calls.load(Ordering::SeqCst), 0);
    assert_eq!(original_count(&fixture, "Attempt"), 1);
    assert_eq!(original_count(&fixture, "Frozen"), 1);
}
