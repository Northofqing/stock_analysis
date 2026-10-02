use super::*;
use crate::durable_delivery::tests::{fixture_coordinator_arc, Fixture};
use crate::durable_delivery::AuthoritativeSinkPort;
use chrono::TimeZone;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex};

const DATE: &str = "2026-09-23";
fn at(day: u32, h: u32, m: u32, s: u32) -> DateTime<Utc> {
    shanghai_offset()
        .with_ymd_and_hms(2026, 9, day, h, m, s)
        .single()
        .unwrap()
        .with_timezone(&Utc)
}
fn operational() -> (tempfile::TempDir, Arc<DatabaseManager>) {
    let dir = tempfile::tempdir().unwrap();
    let db =
        DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_P05_S2.db")).unwrap();
    (dir, Arc::new(db))
}
fn input(day: u32, codes: &[&str], strong: bool) -> P05ObservedDraftInput {
    let entries = codes
        .iter()
        .map(|code| CandidateEntry {
            code: (*code).into(),
            name: format!("TEST_CODE {code}"),
            sources: vec![CandidateSource::StockPick],
            tier: if strong {
                EvidenceTier::Strong
            } else {
                EvidenceTier::Reference
            },
            evidence: vec!["TEST_CODE observed; no QualifiedFacts".into()],
            current_price: Some(10.0),
            change_pct: None,
            heat_score: Some(80.0),
        })
        .collect::<Vec<_>>();
    P05ObservedDraftInput::from_observed(
        at(day, 9, 21, 3).with_timezone(&shanghai_offset()),
        &entries,
        P05ObservedSourceBytes {
            quote_evidence: Some(b"TEST_CODE original bytes".to_vec()),
            statistics_evidence: None,
            p5_file_witnesses: Vec::new(),
            p5_candidate_refs: Vec::new(),
            chain_query: Vec::new(),
            chain_candidate_refs: Vec::new(),
        },
        b"TEST_CODE actual A02 renderer bytes".to_vec(),
        crate::opportunity::candidate_panel::format_candidate_board(&entries).into_bytes(),
    )
    .unwrap()
}
fn renderer(facts: &P05InvalidationRenderFacts) -> Result<Vec<u8>> {
    // This fixture records exactly what the narrow callback was told. Actual
    // bin renderer wiring belongs to the next slice and is not claimed here.
    Ok(format!(
        "TEST_CODE {} {} {} {} {} {}",
        facts.business_date(),
        facts.hhmmss(),
        facts.name(),
        facts.code(),
        facts.previous_state(),
        facts.reason()
    )
    .into_bytes())
}
fn start(f: &Fixture, db: &DatabaseManager, strong: bool) -> StoredP05Draft {
    f.coordinator
        .initialize_prospective_p05_family_at(db, at(23, 9, 19, 0), f.database_path.parent())
        .unwrap();
    f.coordinator
        .store_p05_observed_draft_rendered(
            &input(23, &["TEST_CODE_600001", "TEST_CODE_000001"], strong),
            Some(at(23, 9, 24, 0)),
            &mut renderer,
        )
        .unwrap()
}
fn begin_on(
    f: &Fixture,
    db: &DatabaseManager,
    decision: &str,
    now: DateTime<Utc>,
) -> Result<Option<AttemptLease>> {
    let route = f.coordinator.decision_mutation_route(decision)?;
    let check = f.coordinator.p05_consumer_check_on(db, &route);
    f.coordinator
        .begin_attempt_with_p05_check(decision, 1, now, Some(&check))
}
fn complete_intent(f: &Fixture, db: &DatabaseManager, draft: &StoredP05Draft) -> StoredP05Intent {
    let started = f.coordinator.claim_p05_prediction_prepare(draft).unwrap();
    if !draft.strong_samples().is_empty() {
        let request = crate::monitor::prediction::CandidateBoardPreparationRequest::new(
            draft.business_date(),
            "09:21",
            draft.board_rendered_bytes().to_vec(),
            draft.strong_samples(),
        )
        .unwrap();
        crate::monitor::prediction::prepare_candidate_board_on(db, &request).unwrap();
    }
    f.coordinator
        .complete_p05_unit_intent_on(draft, &started, db)
        .unwrap()
}
#[derive(Default)]
struct Append {
    rows: Mutex<BTreeMap<String, (Vec<u8>, String)>>,
    calls: AtomicUsize,
}
impl ImmutableAppendPort for Append {
    fn append_exact(&self, kind: &str, id: &str, bytes: &[u8], sha: &str) -> Result<String> {
        assert_eq!(sha256_hex(bytes), sha);
        let mut rows = self.rows.lock().unwrap();
        let reference = format!("TEST_CODE_{kind}_{id}");
        if let Some((old, old_ref)) = rows.get(id) {
            assert_eq!(old, bytes);
            return Ok(old_ref.clone());
        }
        self.calls.fetch_add(1, Ordering::SeqCst);
        rows.insert(id.into(), (bytes.to_vec(), reference.clone()));
        Ok(reference)
    }
}
struct Sink {
    calls: AtomicUsize,
    result: AuthoritativeSinkResult,
}
impl AuthoritativeSinkPort for Sink {
    fn sink_identity(&self) -> &str {
        "TEST_CODE_P05_S2_SINK"
    }
    fn deliver(&self, _: &AuthoritativeDeliveryRequest) -> AuthoritativeSinkResult {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.result.clone()
    }
}
fn accepted(at: DateTime<Utc>) -> AuthoritativeSinkResult {
    AuthoritativeSinkResult::Accepted(super::super::super::super::model::TypedReceipt {
        channel: "TEST_CODE_CHANNEL".into(),
        provider: "TEST_CODE_PROVIDER".into(),
        message_id: format!("TEST_CODE_{}", at.timestamp()),
        platform_message_id: None,
        accepted_at: at,
        latency_ms: Some(1),
    })
}
fn sink(result: AuthoritativeSinkResult) -> (Arc<Sink>, Vec<AuthoritativeSink>) {
    let sink = Arc::new(Sink {
        calls: AtomicUsize::new(0),
        result,
    });
    (sink.clone(), vec![sink])
}
fn deliver_all(
    f: &Fixture,
    db: &DatabaseManager,
    intent: &StoredP05Intent,
    day: u32,
    append: &Append,
) -> Arc<Sink> {
    let (sink, sinks) = sink(accepted(at(day, 9, 22, 0)));
    for index in 0..intent.children.len() {
        let outcome = f
            .coordinator
            .dispatch_p05_unit_child_local(
                db,
                &format!("2026-09-{day:02}"),
                index,
                &sinks,
                append,
                Some(at(day, 9, 22, 0)),
            )
            .unwrap();
        assert_eq!(outcome.sink_calls, 1);
    }
    assert!(f
        .coordinator
        .observe_p05_unit_receipts(&format!("2026-09-{day:02}"))
        .unwrap()
        .children()
        .iter()
        .all(|c| matches!(c, P05ChildReceiptObservation::PhysicallyAccepted { .. })));
    sink
}
fn finalize(f: &Fixture, date: &str) -> P05UnitReceiptObservation {
    let observation = f.coordinator.observe_p05_unit_receipts(date).unwrap();
    f.coordinator
        .finalize_p05_unit_observed(date, &observation)
        .unwrap()
}

#[tokio::test]
async fn p05_shared_unit_runtime_actual_worker_once_freeze_restart_no_resave() {
    let f = Fixture::new("S2_WORKER");
    let (_dir, db) = operational();
    let draft = start(&f, &db, true);
    let intent = f
        .coordinator
        .finish_p05_unit_preparation_on(db.clone(), &draft)
        .await
        .unwrap();
    let original = db
        .read_candidate_board_v2_freeze(&draft.board_occurrence().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(original.ordered_rows().len(), 2);
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM delivery_decisions"), 0);
    let reopened = f.second_coordinator("S2_WORKER_RESTART");
    assert_eq!(
        reopened
            .finish_p05_unit_preparation_on(db.clone(), &draft)
            .await
            .unwrap(),
        intent
    );
    assert_eq!(
        db.read_candidate_board_v2_freeze(&draft.board_occurrence().unwrap())
            .unwrap()
            .unwrap()
            .ordered_rows(),
        original.ordered_rows()
    );
    assert_eq!(
        f.query_i64("SELECT COUNT(*) FROM p05_prediction_prepare_events"),
        1
    );
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM p05_unit_children"), 2);
}
#[tokio::test]
async fn p05_shared_unit_runtime_unknown_started_absent_never_calls_worker_or_fallback() {
    let f = Fixture::new("S2_UNKNOWN");
    let (_dir, db) = operational();
    let draft = start(&f, &db, true);
    drop(f.coordinator.claim_p05_prediction_prepare(&draft).unwrap());
    let reopened = f.second_coordinator("S2_UNKNOWN");
    assert!(reopened
        .finish_p05_unit_preparation_on(db.clone(), &draft)
        .await
        .is_err());
    assert!(!db.p05_preparation_residue_for_date(DATE).unwrap());
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM p05_unit_intents"), 0);
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM delivery_decisions"), 0);
}
#[tokio::test]
async fn p05_shared_unit_runtime_two_concurrent_workers_one_started_original_winner() {
    let f = Fixture::new("S2_WORKER_RACE");
    let (_dir, db) = operational();
    let draft = start(&f, &db, true);
    let other = f.second_coordinator("S2_WORKER_RACE");
    let (a, b) = tokio::join!(
        f.coordinator
            .finish_p05_unit_preparation_on(db.clone(), &draft),
        other.finish_p05_unit_preparation_on(db.clone(), &draft)
    );
    assert!(a.is_ok() || b.is_ok());
    let restored = other
        .finish_p05_unit_preparation_on(db.clone(), &draft)
        .await
        .unwrap();
    if let Ok(a) = a {
        assert_eq!(a, restored);
    }
    if let Ok(b) = b {
        assert_eq!(b, restored);
    }
    assert_eq!(
        f.query_i64("SELECT COUNT(*) FROM p05_prediction_prepare_events"),
        1
    );
    assert_eq!(
        db.read_candidate_board_v2_freeze(&draft.board_occurrence().unwrap())
            .unwrap()
            .unwrap()
            .ordered_rows()
            .len(),
        2
    );
}
#[test]
fn p05_shared_unit_runtime_all_children_commit_before_sink_and_ordinary_gate_no_adoption() {
    let f = Fixture::new("S2_GATE");
    let (_dir, db) = operational();
    let draft = start(&f, &db, false);
    assert!(f
        .coordinator
        .prepare_p05_unit_child_local(&db, DATE, 0, 1, Some(at(23, 9, 22, 0)))
        .is_err());
    let legacy = original_v1_envelope(&draft.data, PushKind::AuctionRepush).unwrap();
    assert!(f.coordinator.prepare(&legacy, 1, at(23, 9, 22, 0)).is_err());
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM delivery_decisions"), 0);
    let intent = complete_intent(&f, &db, &draft);
    let (outcome, envelope) = f
        .coordinator
        .prepare_p05_unit_child_local(&db, DATE, 0, 1, Some(at(23, 9, 22, 0)))
        .unwrap();
    assert_eq!(outcome.state, DecisionState::Reserved);
    assert_eq!(envelope.canonical_bytes().unwrap(), intent.children[0].1);
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM p05_s2_child_owners"), 1);
    assert_eq!(
        f.query_i64("SELECT mutation_revision FROM p05_unit_heads"),
        4
    );
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM sink_results"), 0);
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM p05_unit_children"), 2);
}
#[test]
fn p05_shared_unit_runtime_real_presink_denial_is_owned_but_never_completed() {
    let f = Fixture::new("S2_DENIAL");
    let (_dir, db) = operational();
    let draft = start(&f, &db, false);
    complete_intent(&f, &db, &draft);
    let outcome = f
        .coordinator
        .prepare_p05_unit_child_local(&db, DATE, 0, 0, Some(at(23, 9, 22, 0)))
        .unwrap()
        .0;
    assert_ne!(outcome.state, DecisionState::Delivered);
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM p05_s2_child_owners"), 1);
    let append = Append::default();
    f.coordinator
        .reconcile_all_pending(&append, at(23, 9, 22, 0))
        .unwrap();
    let observed = f.coordinator.observe_p05_unit_receipts(DATE).unwrap();
    assert!(matches!(
        observed.children()[0],
        P05ChildReceiptObservation::NonAccepted {
            disposition: P05NonAcceptedTerminal::Rejected,
            ..
        }
    ));
    assert!(f
        .coordinator
        .finalize_p05_unit_observed(DATE, &observed)
        .is_err());
    assert_eq!(
        f.query_i64("SELECT baseline_revision FROM p05_baseline_heads"),
        0
    );
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM sink_results"), 0);
}
#[test]
fn p05_shared_unit_runtime_exact_prepare_noop_revision_and_single_owner() {
    let f = Fixture::new("S2_NOOP");
    let (_dir, db) = operational();
    let draft = start(&f, &db, false);
    complete_intent(&f, &db, &draft);
    let original = f
        .coordinator
        .prepare_p05_unit_child_local(&db, DATE, 0, 1, Some(at(23, 9, 22, 0)))
        .unwrap();
    let revision = f.query_i64("SELECT mutation_revision FROM p05_unit_heads");
    assert_eq!(
        f.coordinator
            .prepare_p05_unit_child_local(&db, DATE, 0, 1, Some(at(23, 9, 23, 0)))
            .unwrap(),
        original
    );
    assert_eq!(
        f.query_i64("SELECT mutation_revision FROM p05_unit_heads"),
        revision
    );
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM p05_s2_child_owners"), 1);
    assert_eq!(
        f.query_i64("SELECT COUNT(*) FROM p05_s2_mutation_events"),
        1
    );
}
#[test]
fn p05_shared_unit_runtime_contextual_prepare_race_keeps_single_owner_and_revision() {
    let f = Fixture::new("S2_OWNER_RACE");
    let (_dir, db) = operational();
    let draft = start(&f, &db, false);
    complete_intent(&f, &db, &draft);
    let other = f.second_coordinator("S2_OWNER_RACE");
    let barrier = Arc::new(Barrier::new(2));
    let (a, b) = std::thread::scope(|scope| {
        let barrier_a = barrier.clone();
        let db_a = db.clone();
        let first_coordinator = f.coordinator.clone();
        let a = scope.spawn(move || {
            barrier_a.wait();
            first_coordinator
                .prepare_p05_unit_child_local(&db_a, DATE, 0, 1, Some(at(23, 9, 22, 0)))
                .unwrap()
        });
        let b = scope.spawn(move || {
            barrier.wait();
            other
                .prepare_p05_unit_child_local(&db, DATE, 0, 1, Some(at(23, 9, 22, 0)))
                .unwrap()
        });
        (a.join().unwrap(), b.join().unwrap())
    });
    assert_eq!(a, b);
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM p05_s2_child_owners"), 1);
    assert_eq!(
        f.query_i64("SELECT mutation_revision FROM p05_unit_heads"),
        4
    );
}
#[test]
fn p05_shared_unit_runtime_actual_accepted_all_child_refs_baseline_cas_restart_no_resend() {
    let f = Fixture::new("S2_ACCEPTED");
    let (_dir, db) = operational();
    let draft = start(&f, &db, true);
    let intent = complete_intent(&f, &db, &draft);
    let append = Append::default();
    let sink = deliver_all(&f, &db, &intent, 23, &append);
    let completed = finalize(&f, DATE);
    assert!(completed.completion_identity().is_some());
    assert_eq!(
        f.query_i64("SELECT baseline_revision FROM p05_baseline_heads"),
        1
    );
    assert_eq!(
        f.query_i64("SELECT COUNT(*) FROM p05_s2_completion_receipts"),
        1
    );
    let bytes = f.query_blob("SELECT completion_canonical FROM p05_s2_completion_receipts");
    let canonical: CompletionCanonical = decode(&bytes).unwrap();
    assert_eq!(canonical.accepted.len(), 2);
    assert!(canonical
        .accepted
        .iter()
        .all(|r| !r.audit_refs.is_empty() && !r.raw_accepted.is_empty()));
    let reopened = f.second_coordinator("S2_ACCEPTED");
    assert_eq!(reopened.observe_p05_unit_receipts(DATE).unwrap(), completed);
    let sinks: Vec<AuthoritativeSink> = vec![sink.clone()];
    for i in 0..2 {
        assert_eq!(
            reopened
                .dispatch_p05_unit_child_local(
                    &db,
                    DATE,
                    i,
                    &sinks,
                    &append,
                    Some(at(24, 12, 0, 0))
                )
                .unwrap()
                .sink_calls,
            0
        );
    }
    assert_eq!(sink.calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        reopened
            .finalize_p05_unit_observed(DATE, &completed)
            .unwrap(),
        completed
    );
}
#[test]
fn p05_shared_unit_runtime_partial_and_nonaccepted_never_advance_baseline() {
    for result in [
        AuthoritativeSinkResult::Rejected(crate::durable_delivery::TypedRejection {
            reason_code: "TEST_CODE_REJECTED".into(),
            evidence: b"TEST_CODE raw rejection".to_vec(),
            retry_authorized: false,
            observed_at: at(23, 9, 22, 0),
        }),
        AuthoritativeSinkResult::Uncertain(crate::durable_delivery::TypedUncertainty {
            reason_code: "TEST_CODE_UNCERTAIN".into(),
            evidence: b"TEST_CODE raw uncertainty".to_vec(),
            observed_at: at(23, 9, 22, 0),
        }),
    ] {
        let f = Fixture::new("S2_NONACCEPTED");
        let (_dir, db) = operational();
        let draft = start(&f, &db, false);
        complete_intent(&f, &db, &draft);
        let append = Append::default();
        let (sink, sinks) = sink(result);
        f.coordinator
            .dispatch_p05_unit_child_local(&db, DATE, 0, &sinks, &append, Some(at(23, 9, 22, 0)))
            .unwrap();
        let observation = f.coordinator.observe_p05_unit_receipts(DATE).unwrap();
        assert!(matches!(
            observation.children()[0],
            P05ChildReceiptObservation::NonAccepted { .. }
        ));
        assert!(matches!(
            observation.children()[1],
            P05ChildReceiptObservation::NotPrepared { .. }
        ));
        assert!(f
            .coordinator
            .finalize_p05_unit_observed(DATE, &observation)
            .is_err());
        assert_eq!(
            f.query_i64("SELECT baseline_revision FROM p05_baseline_heads"),
            0
        );
        assert_eq!(sink.calls.load(Ordering::SeqCst), 1);
    }
}
#[test]
fn p05_shared_unit_runtime_revision_cas_rejects_stale_reader_and_namespace_alias() {
    let f = Fixture::new("S2_REVISION");
    let (_dir, db) = operational();
    let draft = start(&f, &db, false);
    let intent = complete_intent(&f, &db, &draft);
    let before = f.coordinator.observe_p05_unit_receipts(DATE).unwrap();
    let append = Append::default();
    deliver_all(&f, &db, &intent, 23, &append);
    assert!(f
        .coordinator
        .finalize_p05_unit_observed(DATE, &before)
        .is_err());
    let foreign = Fixture::new("S2_REVISION_OTHER");
    let (_other, db2) = operational();
    let d2 = start(&foreign, &db2, false);
    complete_intent(&foreign, &db2, &d2);
    let foreign_observation = foreign.coordinator.observe_p05_unit_receipts(DATE).unwrap();
    assert_eq!(
        foreign_observation.draft_identity(),
        before.draft_identity()
    );
    assert!(f
        .coordinator
        .finalize_p05_unit_observed(DATE, &foreign_observation)
        .is_err());
    assert_eq!(
        f.query_i64("SELECT COUNT(*) FROM p05_s2_completion_receipts"),
        0
    );
    assert!(finalize(&f, DATE).completion_identity().is_some());
}
#[test]
fn p05_shared_unit_runtime_late_raw_retained_dirty_reclose_without_resend_or_baseline_rollback() {
    let f = Fixture::new("S2_LATE");
    let (_dir, db) = operational();
    let draft = start(&f, &db, false);
    let intent = complete_intent(&f, &db, &draft);
    let append = Append::default();
    let sink = deliver_all(&f, &db, &intent, 23, &append);
    let completed = finalize(&f, DATE);
    let baseline=f.query_blob("SELECT origin_canonical FROM p05_baseline_origins WHERE origin_kind='CompletedUnitV2Baseline'");
    let (attempt, fence): (String, i64) = Connection::open(&f.database_path)
        .unwrap()
        .query_row(
            "SELECT attempt_identity,fence_token FROM delivery_attempts ORDER BY rowid LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    f.coordinator
        .record_sink_result(
            &attempt,
            fence,
            accepted(at(23, 9, 23, 0)),
            at(23, 9, 23, 0),
        )
        .unwrap();
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM sink_results"), 3);
    let dirty = f.coordinator.observe_p05_unit_receipts(DATE).unwrap();
    assert!(dirty.completion_identity().is_none());
    assert!(dirty.mutation_revision() > completed.mutation_revision());
    assert!(f
        .coordinator
        .finalize_p05_unit_observed(DATE, &dirty)
        .is_err());
    f.coordinator
        .reconcile_all_pending(&append, at(23, 9, 24, 0))
        .unwrap();
    let restored = finalize(&f, DATE);
    assert!(restored.completion_identity().is_some());
    assert_ne!(
        restored.completion_identity(),
        completed.completion_identity()
    );
    assert_eq!(
        f.query_i64("SELECT COUNT(*) FROM p05_s2_completion_receipts"),
        2
    );
    assert_eq!(
        f.query_i64("SELECT baseline_revision FROM p05_baseline_heads"),
        1
    );
    assert_eq!(f.query_blob("SELECT origin_canonical FROM p05_baseline_origins WHERE origin_kind='CompletedUnitV2Baseline'"),baseline);
    assert_eq!(sink.calls.load(Ordering::SeqCst), 2);
}
#[test]
fn p05_shared_unit_runtime_completed_crossday_fixed_t08_order_source_scope_and_retry() {
    let f = Fixture::new("S2_CROSSDAY");
    let (_dir, db) = operational();
    let draft = start(&f, &db, false);
    let intent = complete_intent(&f, &db, &draft);
    let append = Append::default();
    deliver_all(&f, &db, &intent, 23, &append);
    let previous = finalize(&f, DATE);
    let mut calls = Vec::new();
    let new = f
        .coordinator
        .store_p05_observed_draft_rendered(
            &input(24, &["TEST_CODE_300001"], false),
            Some(at(24, 9, 24, 0)),
            &mut |facts| {
                calls.push((
                    facts.code().to_owned(),
                    facts.hhmmss().to_owned(),
                    facts.name().to_owned(),
                ));
                renderer(facts)
            },
        )
        .unwrap();
    assert_eq!(
        calls,
        vec![
            (
                "TEST_CODE_000001".into(),
                "09:21:03".into(),
                "TEST_CODE_000001".into()
            ),
            (
                "TEST_CODE_600001".into(),
                "09:21:03".into(),
                "TEST_CODE_600001".into()
            )
        ]
    );
    let next = complete_intent(&f, &db, &new);
    let envelopes = next
        .children
        .iter()
        .map(|(_, raw)| parse_envelope(raw).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        envelopes.iter().map(|e| e.push_kind).collect::<Vec<_>>(),
        vec![
            PushKind::AuctionRepush,
            PushKind::CandidateInvalidated,
            PushKind::CandidateInvalidated,
            PushKind::CandidateBoard
        ]
    );
    assert_eq!(
        envelopes[1].schedule_occurrence_identity,
        "candidate-invalidated:2026-09-24:TEST_CODE_000001"
    );
    assert_eq!(envelopes[1].scope_key, "SHENZHEN:EQUITY:TEST_CODE_000001");
    assert!(envelopes[1].retry_authorized);
    let source: serde_json::Value =
        serde_json::from_slice(&envelopes[1].source_binding_canonical).unwrap();
    assert_eq!(source["prev"], "候选");
    assert_eq!(source["reason"], "从候选台消失");
    let invalidated = new.data.invalidated.clone();
    assert!(
        matches!(invalidated,InvalidatedPreparation::CompletedBaseline {receipt_identity,..} if Some(receipt_identity.as_str())==previous.completion_identity())
    );
    assert_eq!(
        f.query_i64("SELECT baseline_revision FROM p05_baseline_heads"),
        1
    );
    assert_eq!(
        f.query_i64("SELECT COUNT(*) FROM delivery_decisions WHERE business_date='2026-09-24'"),
        0
    );
    deliver_all(&f, &db, &next, 24, &append);
    finalize(&f, "2026-09-24");
    assert_eq!(
        f.query_i64("SELECT baseline_revision FROM p05_baseline_heads"),
        2
    );
}
#[test]
fn p05_shared_unit_runtime_market_empty_difference_has_real_previous_witness_not_prospective() {
    let f = Fixture::new("S2_EMPTY_DIFFERENCE");
    let (_dir, db) = operational();
    let draft = start(&f, &db, false);
    let intent = complete_intent(&f, &db, &draft);
    let append = Append::default();
    deliver_all(&f, &db, &intent, 23, &append);
    finalize(&f, DATE);
    let mut called = false;
    let next = f
        .coordinator
        .store_p05_observed_draft_rendered(
            &input(24, &["TEST_CODE_600001", "TEST_CODE_000001"], false),
            Some(at(24, 9, 24, 0)),
            &mut |_| {
                called = true;
                panic!("no removal must not render")
            },
        )
        .unwrap();
    assert!(!called);
    assert!(
        matches!(next.data.invalidated,InvalidatedPreparation::CompletedBaseline {ref removals,..} if removals.is_empty())
    );
    assert_eq!(complete_intent(&f, &db, &next).children.len(), 2);
}
#[test]
fn p05_shared_unit_runtime_intervening_partial_or_dirty_baseline_blocks_new_day_without_adoption() {
    let f = Fixture::new("S2_PARTIAL_DAY");
    let (_dir, db) = operational();
    let draft = start(&f, &db, false);
    let intent = complete_intent(&f, &db, &draft);
    let append = Append::default();
    deliver_all(&f, &db, &intent, 23, &append);
    finalize(&f, DATE);
    f.coordinator
        .store_p05_observed_draft_rendered(
            &input(24, &["TEST_CODE_600001"], false),
            Some(at(24, 9, 24, 0)),
            &mut renderer,
        )
        .unwrap();
    assert_eq!(
        crate::calendar::verified_next_a_share_trading_day(date_day("2026-09-24").unwrap())
            .unwrap(),
        date_day("2026-09-28").unwrap()
    );
    let error = f
        .coordinator
        .store_p05_observed_draft_rendered(
            &input(28, &["TEST_CODE_600001"], false),
            Some(at(28, 9, 24, 0)),
            &mut renderer,
        )
        .unwrap_err();
    assert!(
        error.to_string().contains("intervening incomplete Unit"),
        "next trading day must be rejected for the unfinished prior Unit: {error}"
    );
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM p05_unit_drafts"), 2);
    assert_eq!(
        f.query_i64("SELECT baseline_revision FROM p05_baseline_heads"),
        1
    );
}
#[test]
fn p05_shared_unit_runtime_actual_freeze_score_drift_never_dispatches() {
    let f = Fixture::new("S2_SCORE_DRIFT");
    let (_dir, db) = operational();
    let draft = start(&f, &db, true);
    complete_intent(&f, &db, &draft);
    // Mutate only the real operational prediction row; immutable frozen bytes
    // retain IDs/card. The actual reader must detect the changed score.
    let path = std::fs::read_dir(_dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .find(|e| e.path().extension().and_then(|s| s.to_str()) == Some("db"))
        .unwrap()
        .path();
    Connection::open(path)
        .unwrap()
        .execute(
            "UPDATE prediction_tracker SET pred_score=81 WHERE stock_code='TEST_CODE_600001'",
            [],
        )
        .unwrap();
    let append = Append::default();
    let (sink, sinks) = sink(accepted(at(23, 9, 22, 0)));
    assert!(f
        .coordinator
        .dispatch_p05_unit_child_local(&db, DATE, 0, &sinks, &append, Some(at(23, 9, 22, 0)))
        .is_err());
    assert_eq!(sink.calls.load(Ordering::SeqCst), 0);
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM delivery_decisions"), 0);
}
#[test]
fn p05_shared_unit_runtime_hook_extra_authority_rolls_back_owner_and_revision() {
    let f = Fixture::new("S2_POSTSQL");
    let (_dir, db) = operational();
    let draft = start(&f, &db, false);
    complete_intent(&f, &db, &draft);
    // This is a real same-connection SQL injection after the shared body,
    // bypassing its revision witness. Global pre-COMMIT validation rejects it.
    f.coordinator
        .install_operation_postvalidation_test_fault(
            OperationPostvalidationTestFault::P05UnitRevisionWithoutEvent,
        )
        .unwrap();
    assert!(f
        .coordinator
        .prepare_p05_unit_child_local(&db, DATE, 0, 1, Some(at(23, 9, 22, 0)))
        .is_err());
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM p05_s2_child_owners"), 0);
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM delivery_decisions"), 0);
    assert_eq!(
        f.query_i64("SELECT mutation_revision FROM p05_unit_heads"),
        3
    );
}
#[test]
fn p05_shared_unit_runtime_schema14_v13_additive_preserves_nonempty_preparation_bytes() {
    let mut f = Fixture::new("S2_MIGRATE13");
    let (_dir, db) = operational();
    let draft = start(&f, &db, false);
    let intent = complete_intent(&f, &db, &draft);
    drop(f.coordinator.take().unwrap());
    let connection = Connection::open(&f.database_path).unwrap();
    crate::durable_delivery::schema_p05_unit_runtime::remove_empty_extension_for_legacy_test(
        &connection,
    );
    connection.pragma_update(None, "user_version", 13).unwrap();
    drop(connection);
    let reopened = f.second_coordinator("S2_MIGRATE13");
    assert_eq!(reopened.read_p05_unit_draft(DATE).unwrap().unwrap(), draft);
    assert_eq!(
        reopened.read_p05_unit_intent(DATE).unwrap().unwrap(),
        intent
    );
    assert_eq!(
        Connection::open(&f.database_path)
            .unwrap()
            .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        14
    );
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM p05_s2_child_owners"), 0);
}
#[test]
fn p05_shared_unit_runtime_schema14_preexisting_reserved_and_current_missing_not_healed() {
    for current in [false, true] {
        let mut f = Fixture::new("S2_SCHEMA_DRIFT");
        drop(f.coordinator.take().unwrap());
        let connection = Connection::open(&f.database_path).unwrap();
        if current {
            connection
                .execute_batch("DROP TRIGGER p05_s2_new_family_decision")
                .unwrap();
        } else {
            crate::durable_delivery::schema_p05_unit_runtime::remove_empty_extension_for_legacy_test(&connection);
            connection.pragma_update(None, "user_version", 13).unwrap();
            connection
                .execute_batch("CREATE TABLE P05_S2_foreign(bytes BLOB)")
                .unwrap();
        }
        let code = f
            .database_path
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap();
        assert!(DurableDeliveryCoordinator::open(CoordinatorConfig::test(
            &f.database_path,
            code,
            "TEST_CODE_S2_BAD_SCHEMA_0123456789abcdef"
        ))
        .is_err());
        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            if current { 14 } else { 13 }
        );
    }
}

fn arm_extra_at_phase(
    coordinator: &Arc<DurableDeliveryCoordinator>,
    second_phase: bool,
) -> Arc<AtomicUsize> {
    let hit = Arc::new(AtomicUsize::new(0));
    if second_phase {
        let weak = Arc::downgrade(coordinator);
        let callback_hit = hit.clone();
        coordinator
            .install_database_operation_test_hook(
                DatabaseOperationTestPhase::AfterSqlBeforePreCommitValidation,
                move || {
                    let second = weak.clone();
                    weak.upgrade()
                        .unwrap()
                        .install_database_operation_test_hook(
                            DatabaseOperationTestPhase::AfterSqlBeforePostValidation,
                            move || {
                                callback_hit.fetch_add(1, Ordering::SeqCst);
                                second
                                    .upgrade()
                                    .unwrap()
                                    .install_operation_postvalidation_test_fault(
                                        OperationPostvalidationTestFault::P05ActualExtraMutation,
                                    )
                            },
                        )
                },
            )
            .unwrap();
    } else {
        coordinator
            .install_operation_postvalidation_test_fault(
                OperationPostvalidationTestFault::P05ActualExtraMutation,
            )
            .unwrap();
    }
    hit
}
#[test]
fn p05_shared_unit_runtime_sql_valid_extra_mutation_changed_and_nochange_roll_back_exact_binding() {
    for second_phase in [false, true] {
        for existing in [false, true] {
            let f = Fixture::new("S2_LAWFUL_EXTRA");
            let (_dir, db) = operational();
            let draft = start(&f, &db, false);
            complete_intent(&f, &db, &draft);
            let coordinator = f.second_coordinator("S2_LAWFUL_EXTRA");
            if existing {
                coordinator
                    .prepare_p05_unit_child_local(&db, DATE, 0, 1, Some(at(23, 9, 22, 0)))
                    .unwrap();
            }
            let revision = f.query_i64("SELECT mutation_revision FROM p05_unit_heads");
            let owners = f.query_i64("SELECT COUNT(*) FROM p05_s2_child_owners");
            let audits = f.query_i64("SELECT COUNT(*) FROM immutable_audit_outbox");
            let hit = arm_extra_at_phase(&coordinator, second_phase);
            let error = coordinator
                .prepare_p05_unit_child_local(&db, DATE, 0, 1, Some(at(23, 9, 22, 0)))
                .unwrap_err();
            assert!(
                error.to_string().contains("exact Unit SQL binding"),
                "{error}"
            );
            if second_phase {
                assert_eq!(hit.load(Ordering::SeqCst), 1);
            }
            assert_eq!(
                f.query_i64("SELECT mutation_revision FROM p05_unit_heads"),
                revision
            );
            assert_eq!(
                f.query_i64("SELECT COUNT(*) FROM p05_s2_child_owners"),
                owners
            );
            assert_eq!(
                f.query_i64("SELECT COUNT(*) FROM immutable_audit_outbox"),
                audits
            );
        }
    }
}
#[test]
fn p05_shared_unit_runtime_nochange_completion_pointer_drift_is_rejected_and_rolled_back() {
    let f = Fixture::new("S2_POINTER_DRIFT");
    let (_dir, db) = operational();
    let draft = start(&f, &db, false);
    let intent = complete_intent(&f, &db, &draft);
    let append = Append::default();
    deliver_all(&f, &db, &intent, 23, &append);
    let completed = finalize(&f, DATE);
    f.coordinator
        .install_operation_postvalidation_test_fault(
            OperationPostvalidationTestFault::P05CompletionPointerDrift,
        )
        .unwrap();
    assert!(f
        .coordinator
        .finalize_p05_unit_observed(DATE, &completed)
        .unwrap_err()
        .to_string()
        .contains("exact Unit SQL binding"));
    assert_eq!(
        f.coordinator.observe_p05_unit_receipts(DATE).unwrap(),
        completed
    );
    assert_eq!(
        f.query_i64("SELECT baseline_revision FROM p05_baseline_heads"),
        1
    );
}
#[test]
fn p05_shared_unit_runtime_real_postcommit_extra_mutation_preserves_committed_owner_and_returns_error(
) {
    let f = Fixture::new("S2_POST_COMMIT");
    let (_dir, db) = operational();
    let draft = start(&f, &db, false);
    complete_intent(&f, &db, &draft);
    let path = f.database_path.clone();
    let hit = Arc::new(AtomicUsize::new(0));
    let callback_hit = hit.clone();
    f.coordinator
        .install_database_operation_test_hook(
            DatabaseOperationTestPhase::AfterCommitBeforePostValidation,
            move || {
                callback_hit.fetch_add(1, Ordering::SeqCst);
                let mut c = Connection::open(path)?;
                crate::durable_delivery::schema::register_sha256_function(&c)?;
                c.pragma_update(None, "foreign_keys", "ON")?;
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                actual_extra_mutation_for_test(&tx)?;
                tx.commit()?;
                Ok(())
            },
        )
        .unwrap();
    let error = f
        .coordinator
        .prepare_p05_unit_child_local(&db, DATE, 0, 1, Some(at(23, 9, 22, 0)))
        .unwrap_err();
    assert!(error.to_string().contains("COMMIT succeeded"), "{error}");
    assert_eq!(hit.load(Ordering::SeqCst), 1);
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM p05_s2_child_owners"), 1);
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM delivery_decisions"), 1);
    assert_eq!(
        f.query_i64("SELECT mutation_revision FROM p05_unit_heads"),
        5
    );
    assert!(f
        .coordinator
        .observe_p05_unit_receipts(DATE)
        .unwrap()
        .completion_identity()
        .is_none());
}
#[test]
fn p05_shared_unit_runtime_identity_conflict_commits_original_audit_and_one_revision() {
    let f = Fixture::new("S2_CONFLICT");
    let (_dir, db) = operational();
    let draft = start(&f, &db, false);
    complete_intent(&f, &db, &draft);
    let (_, mut envelope) = f
        .coordinator
        .prepare_p05_unit_child_local(&db, DATE, 0, 1, Some(at(23, 9, 22, 0)))
        .unwrap();
    let original = envelope.canonical_bytes().unwrap();
    let before = f.query_i64("SELECT mutation_revision FROM p05_unit_heads");
    envelope.rendered_content = b"TEST_CODE conflicting card keeping original identity".to_vec();
    assert!(matches!(
        f.coordinator.prepare(&envelope, 1, at(23, 9, 23, 0)),
        Err(DurableDeliveryError::DecisionIdentityConflict { .. })
    ));
    assert_eq!(
        f.query_i64("SELECT mutation_revision FROM p05_unit_heads"),
        before + 1
    );
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM immutable_audit_outbox WHERE audit_kind='DecisionIdentityConflict'"),1);
    assert_eq!(
        f.query_blob("SELECT envelope_canonical FROM delivery_decisions"),
        original
    );
}
#[test]
fn p05_shared_unit_runtime_manual_accepted_is_not_physical_completion() {
    let f = Fixture::new("S2_MANUAL");
    let (_dir, db) = operational();
    let draft = start(&f, &db, false);
    complete_intent(&f, &db, &draft);
    let append = Append::default();
    let uncertainty =
        AuthoritativeSinkResult::Uncertain(crate::durable_delivery::TypedUncertainty {
            reason_code: "TEST_CODE_UNCERTAIN".into(),
            evidence: b"TEST_CODE observed uncertainty".to_vec(),
            observed_at: at(23, 9, 22, 0),
        });
    let (_, sinks) = sink(uncertainty);
    let outcome = f
        .coordinator
        .dispatch_p05_unit_child_local(&db, DATE, 0, &sinks, &append, Some(at(23, 9, 22, 0)))
        .unwrap();
    let receipt = match accepted(at(23, 9, 23, 0)) {
        AuthoritativeSinkResult::Accepted(r) => r,
        _ => unreachable!(),
    };
    let route = f
        .coordinator
        .decision_mutation_route(&outcome.decision_identity)
        .unwrap();
    let check = f.coordinator.p05_consumer_check_on(&db, &route);
    f.coordinator
        .resolve_uncertain_with_p05_check(
            &crate::durable_delivery::ManualResolutionCommand {
                decision_identity: outcome.decision_identity,
                disposition: crate::durable_delivery::ManualDisposition::Accepted {
                    receipt: Some(receipt),
                },
                operator_identity: "TEST_CODE_OPERATOR_P05_0123456789abcdef".into(),
                reason: "TEST_CODE verified manually".into(),
                external_evidence: b"TEST_CODE external witness".to_vec(),
                resolved_at: at(23, 9, 23, 0),
            },
            &append,
            Some(&check),
        )
        .unwrap();
    f.coordinator
        .reconcile_all_pending(&append, at(23, 9, 23, 0))
        .unwrap();
    let observation = f.coordinator.observe_p05_unit_receipts(DATE).unwrap();
    assert!(matches!(
        observation.children()[0],
        P05ChildReceiptObservation::NonAccepted {
            disposition: P05NonAcceptedTerminal::ManualAccepted,
            ..
        }
    ));
    assert!(f
        .coordinator
        .finalize_p05_unit_observed(DATE, &observation)
        .is_err());
    assert_eq!(
        f.query_i64("SELECT baseline_revision FROM p05_baseline_heads"),
        0
    );
}

#[tokio::test]
async fn p05_shared_unit_runtime_worker_cannot_borrow_identical_foreign_namespace_draft() {
    let first = Fixture::new("S2_NS_FIRST");
    let second = Fixture::new("S2_NS_SECOND");
    let (_one, db1) = operational();
    let (_two, db2) = operational();
    let foreign = start(&first, &db1, true);
    let local = start(&second, &db2, true);
    assert_eq!(foreign.identity(), local.identity());
    assert!(second
        .coordinator
        .finish_p05_unit_preparation_on(db2.clone(), &foreign)
        .await
        .is_err());
    assert_eq!(
        second.query_i64("SELECT COUNT(*) FROM p05_prediction_prepare_events"),
        0
    );
    assert!(!db2.p05_preparation_residue_for_date(DATE).unwrap());
}
#[test]
fn p05_shared_unit_runtime_current14_temp_shadow_and_unknown_runtime_objects_rejected() {
    let f = Fixture::new("S2_TEMP_SHADOW");
    assert!(f
        .coordinator
        .with_connection(|c| {
            c.execute_batch("CREATE TEMP TABLE P05_S2_completion_receipts(bytes BLOB)")?;
            validate_rows(c)
        })
        .is_err());
    let mut unknown = Fixture::new("S2_UNKNOWN_OBJECT");
    drop(unknown.coordinator.take().unwrap());
    let c = Connection::open(&unknown.database_path).unwrap();
    c.execute_batch("CREATE TABLE p05_s2_unknown_foreign(bytes BLOB)")
        .unwrap();
    let code = unknown
        .database_path
        .parent()
        .unwrap()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap();
    assert!(DurableDeliveryCoordinator::open(CoordinatorConfig::test(
        &unknown.database_path,
        code,
        "TEST_CODE_S2_UNKNOWN_0123456789abcdef"
    ))
    .is_err());
}

#[test]
fn p05_shared_unit_runtime_finalizer_postcommit_dirty_keeps_first_receipt_and_cas_but_no_current_complete(
) {
    let f = Fixture::new("S2_FINAL_POSTCOMMIT");
    let (_dir, db) = operational();
    let draft = start(&f, &db, false);
    let intent = complete_intent(&f, &db, &draft);
    let append = Append::default();
    let sink = deliver_all(&f, &db, &intent, 23, &append);
    let observed = f.coordinator.observe_p05_unit_receipts(DATE).unwrap();
    let path = f.database_path.clone();
    let hit = Arc::new(AtomicUsize::new(0));
    let callback_hit = hit.clone();
    f.coordinator
        .install_database_operation_test_hook(
            DatabaseOperationTestPhase::AfterCommitBeforePostValidation,
            move || {
                callback_hit.fetch_add(1, Ordering::SeqCst);
                let mut c = Connection::open(path)?;
                crate::durable_delivery::schema::register_sha256_function(&c)?;
                c.pragma_update(None, "foreign_keys", "ON")?;
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                actual_extra_mutation_for_test(&tx)?;
                tx.commit()?;
                Ok(())
            },
        )
        .unwrap();
    let error = f
        .coordinator
        .finalize_p05_unit_observed(DATE, &observed)
        .unwrap_err();
    assert!(error.to_string().contains("COMMIT succeeded"), "{error}");
    assert_eq!(hit.load(Ordering::SeqCst), 1);
    assert_eq!(
        f.query_i64("SELECT COUNT(*) FROM p05_s2_completion_receipts"),
        1
    );
    assert_eq!(
        f.query_i64("SELECT baseline_revision FROM p05_baseline_heads"),
        1
    );
    assert!(f
        .coordinator
        .observe_p05_unit_receipts(DATE)
        .unwrap()
        .completion_identity()
        .is_none());
    f.coordinator
        .reconcile_all_pending(&append, at(23, 9, 24, 0))
        .unwrap();
    let complete = finalize(&f, DATE);
    assert!(complete.completion_identity().is_some());
    assert_eq!(
        f.query_i64("SELECT baseline_revision FROM p05_baseline_heads"),
        1
    );
    assert_eq!(sink.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn p05_shared_unit_runtime_read_boundary_lawful_late_change_keeps_history_but_cannot_return_old_completion(
) {
    let f = Fixture::new("S2_READ_BOUNDARY");
    let (_dir, db) = operational();
    let draft = start(&f, &db, false);
    let intent = complete_intent(&f, &db, &draft);
    let append = Append::default();
    deliver_all(&f, &db, &intent, 23, &append);
    let original = finalize(&f, DATE);
    let path = f.database_path.clone();
    let hit = Arc::new(AtomicUsize::new(0));
    let callback_hit = hit.clone();
    f.coordinator
        .install_database_operation_test_hook(
            DatabaseOperationTestPhase::AfterSqlBeforePostValidation,
            move || {
                callback_hit.fetch_add(1, Ordering::SeqCst);
                let mut c = Connection::open(path)?;
                crate::durable_delivery::schema::register_sha256_function(&c)?;
                c.pragma_update(None, "foreign_keys", "ON")?;
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                actual_extra_mutation_for_test(&tx)?;
                tx.commit()?;
                Ok(())
            },
        )
        .unwrap();
    let error = f.coordinator.observe_p05_unit_receipts(DATE).unwrap_err();
    assert!(
        error.to_string().contains("exact Unit SQL binding"),
        "{error}"
    );
    assert_eq!(hit.load(Ordering::SeqCst), 1);
    let current = f.coordinator.observe_p05_unit_receipts(DATE).unwrap();
    assert!(current.completion_identity().is_none());
    assert!(current.mutation_revision() > original.mutation_revision());
    assert_eq!(
        f.query_i64("SELECT COUNT(*) FROM p05_s2_completion_receipts"),
        1
    );
    assert_eq!(
        f.query_i64("SELECT baseline_revision FROM p05_baseline_heads"),
        1
    );
}

#[test]
fn p05_shared_unit_runtime_first_nextday_completion_waits_for_prior_reclosure_without_resend() {
    let f = Fixture::new("S2_FINAL_PRIOR_DIRTY");
    let (_dir, db) = operational();
    let day1 = start(&f, &db, false);
    let intent1 = complete_intent(&f, &db, &day1);
    let append = Append::default();
    let sink1 = deliver_all(&f, &db, &intent1, 23, &append);
    let first = finalize(&f, DATE);
    let day2 = f
        .coordinator
        .store_p05_observed_draft_rendered(
            &input(24, &["TEST_CODE_600001", "TEST_CODE_000001"], false),
            Some(at(24, 9, 24, 0)),
            &mut renderer,
        )
        .unwrap();
    let intent2 = complete_intent(&f, &db, &day2);
    let sink2 = deliver_all(&f, &db, &intent2, 24, &append);
    let day2_receipts = f
        .coordinator
        .observe_p05_unit_receipts("2026-09-24")
        .unwrap();
    assert!(day2_receipts
        .children()
        .iter()
        .all(|r| matches!(r, P05ChildReceiptObservation::PhysicallyAccepted { .. })));
    let original_day2=f.query_i64("SELECT COUNT(*) FROM sink_results WHERE decision_identity IN (SELECT decision_identity FROM p05_unit_children WHERE draft_identity=(SELECT draft_identity FROM p05_unit_drafts WHERE business_date='2026-09-24'))");
    let (attempt,fence):(String,i64)=Connection::open(&f.database_path).unwrap().query_row("SELECT a.attempt_identity,a.fence_token FROM delivery_attempts a JOIN delivery_decisions d ON d.decision_identity=a.decision_identity WHERE d.business_date='2026-09-23' ORDER BY a.rowid LIMIT 1",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    f.coordinator
        .record_sink_result(
            &attempt,
            fence,
            accepted(at(24, 9, 23, 0)),
            at(24, 9, 23, 0),
        )
        .unwrap();
    assert!(f
        .coordinator
        .observe_p05_unit_receipts(DATE)
        .unwrap()
        .completion_identity()
        .is_none());
    assert!(f
        .coordinator
        .finalize_p05_unit_observed("2026-09-24", &day2_receipts)
        .is_err());
    assert_eq!(
        f.query_i64("SELECT baseline_revision FROM p05_baseline_heads"),
        1
    );
    assert_eq!(
        f.query_i64("SELECT COUNT(*) FROM p05_s2_completion_receipts"),
        1
    );
    assert_eq!(sink2.calls.load(Ordering::SeqCst), 2);
    f.coordinator
        .reconcile_all_pending(&append, at(24, 9, 24, 0))
        .unwrap();
    let reclosed = finalize(&f, DATE);
    assert_ne!(reclosed.completion_identity(), first.completion_identity());
    let completed = f
        .coordinator
        .finalize_p05_unit_observed("2026-09-24", &day2_receipts)
        .unwrap();
    assert!(completed.completion_identity().is_some());
    assert_eq!(
        f.query_i64("SELECT baseline_revision FROM p05_baseline_heads"),
        2
    );
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM sink_results WHERE decision_identity IN (SELECT decision_identity FROM p05_unit_children WHERE draft_identity=(SELECT draft_identity FROM p05_unit_drafts WHERE business_date='2026-09-24'))"),original_day2);
    assert_eq!(sink1.calls.load(Ordering::SeqCst), 2);
    assert_eq!(sink2.calls.load(Ordering::SeqCst), 2);
}

fn completed_prior(f: &Fixture, db: &DatabaseManager, append: &Append) -> Arc<Sink> {
    let draft = start(f, db, false);
    let intent = complete_intent(f, db, &draft);
    let sink = deliver_all(f, db, &intent, 23, append);
    assert!(finalize(f, DATE).completion_identity().is_some());
    sink
}
fn next_draft(f: &Fixture) -> StoredP05Draft {
    f.coordinator
        .store_p05_observed_draft_rendered(
            &input(24, &["TEST_CODE_600001", "TEST_CODE_000001"], false),
            Some(at(24, 9, 24, 0)),
            &mut renderer,
        )
        .unwrap()
}
fn arm_actual_prior_postcommit(f: &Fixture) -> Arc<AtomicUsize> {
    let path = f.database_path.clone();
    let hit = Arc::new(AtomicUsize::new(0));
    let callback_hit = hit.clone();
    f.coordinator
        .install_database_operation_test_hook(
            DatabaseOperationTestPhase::AfterCommitBeforePostValidation,
            move || {
                callback_hit.fetch_add(1, Ordering::SeqCst);
                let mut c = Connection::open(path)?;
                crate::durable_delivery::schema::register_sha256_function(&c)?;
                c.pragma_update(None, "foreign_keys", "ON")?;
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                actual_extra_mutation_for_test(&tx)?;
                tx.commit()?;
                Ok(())
            },
        )
        .unwrap();
    hit
}
#[test]
fn p05_shared_unit_runtime_prior_closure_hooks_fresh_draft_prepare_begin_rollback() {
    for second_phase in [false, true] {
        for opening in ["draft", "prepare", "begin"] {
            let f = Fixture::new("S2_PRIOR_OPEN_HOOK");
            let (_dir, db) = operational();
            let append = Append::default();
            let prior_sink = completed_prior(&f, &db, &append);
            let first = f.coordinator.observe_p05_unit_receipts(DATE).unwrap();
            let day2 = if opening == "draft" {
                None
            } else {
                Some(next_draft(&f))
            };
            if let Some(draft) = &day2 {
                complete_intent(&f, &db, draft);
            }
            let decision = if opening == "begin" {
                let (_, envelope) = f
                    .coordinator
                    .prepare_p05_unit_child_local(&db, "2026-09-24", 0, 1, Some(at(24, 9, 22, 0)))
                    .unwrap();
                f.coordinator
                    .reconcile_all_pending(&append, at(24, 9, 22, 0))
                    .unwrap();
                Some(envelope.decision_identity)
            } else {
                None
            };
            let day2_owners=f.query_i64("SELECT COUNT(*) FROM p05_s2_child_owners o JOIN p05_unit_drafts d ON d.draft_identity=o.draft_identity WHERE d.business_date='2026-09-24'");
            let audits = f.query_i64("SELECT COUNT(*) FROM immutable_audit_outbox");
            let hit = arm_extra_at_phase(&fixture_coordinator_arc(&f), second_phase);
            let result = match opening {
                "draft" => f
                    .coordinator
                    .store_p05_observed_draft_rendered(
                        &input(24, &["TEST_CODE_600001", "TEST_CODE_000001"], false),
                        Some(at(24, 9, 24, 0)),
                        &mut renderer,
                    )
                    .map(|_| ()),
                "prepare" => f
                    .coordinator
                    .prepare_p05_unit_child_local(&db, "2026-09-24", 0, 1, Some(at(24, 9, 22, 0)))
                    .map(|_| ()),
                "begin" => {
                    begin_on(&f, &db, decision.as_ref().unwrap(), at(24, 9, 22, 1)).map(|_| ())
                }
                _ => unreachable!(),
            };
            let error = result.unwrap_err();
            assert!(
                error.to_string().contains("prior current closure"),
                "{opening}: {error}"
            );
            if second_phase {
                assert_eq!(hit.load(Ordering::SeqCst), 1);
            }
            assert_eq!(
                f.coordinator.observe_p05_unit_receipts(DATE).unwrap(),
                first
            );
            assert_eq!(
                f.query_i64("SELECT baseline_revision FROM p05_baseline_heads"),
                1
            );
            assert_eq!(
                f.query_i64("SELECT COUNT(*) FROM immutable_audit_outbox"),
                audits
            );
            assert_eq!(f.query_i64("SELECT COUNT(*) FROM p05_s2_child_owners o JOIN p05_unit_drafts d ON d.draft_identity=o.draft_identity WHERE d.business_date='2026-09-24'"), day2_owners);
            assert_eq!(f.query_i64("SELECT COUNT(*) FROM delivery_attempts a JOIN delivery_decisions d ON d.decision_identity=a.decision_identity WHERE d.business_date='2026-09-24'"), 0);
            if let Some(decision) = decision {
                assert_eq!(
                    f.coordinator.decision_state(&decision).unwrap(),
                    DecisionState::Reserved
                );
            }
            if opening == "draft" {
                assert!(f
                    .coordinator
                    .read_p05_unit_draft("2026-09-24")
                    .unwrap()
                    .is_none());
            }
            assert_eq!(prior_sink.calls.load(Ordering::SeqCst), 2);
        }
    }
}
#[test]
fn p05_shared_unit_runtime_prior_closure_after_sql_first_finalizer_rolls_back_only_new_completion()
{
    for second_phase in [false, true] {
        let f = Fixture::new("S2_PRIOR_FINAL_HOOK");
        let (_dir, db) = operational();
        let append = Append::default();
        let prior_sink = completed_prior(&f, &db, &append);
        let prior = f.coordinator.observe_p05_unit_receipts(DATE).unwrap();
        let draft = next_draft(&f);
        let intent = complete_intent(&f, &db, &draft);
        let sink = deliver_all(&f, &db, &intent, 24, &append);
        let day2 = f
            .coordinator
            .observe_p05_unit_receipts("2026-09-24")
            .unwrap();
        let raw=f.query_blob("SELECT result_canonical FROM sink_results s JOIN delivery_decisions d ON d.decision_identity=s.decision_identity WHERE d.business_date='2026-09-24' ORDER BY s.rowid LIMIT 1");
        let hit = arm_extra_at_phase(&fixture_coordinator_arc(&f), second_phase);
        let error = f
            .coordinator
            .finalize_p05_unit_observed("2026-09-24", &day2)
            .unwrap_err();
        assert!(
            error.to_string().contains("prior current closure"),
            "{error}"
        );
        if second_phase {
            assert_eq!(hit.load(Ordering::SeqCst), 1);
        }
        assert_eq!(
            f.coordinator.observe_p05_unit_receipts(DATE).unwrap(),
            prior
        );
        assert_eq!(
            f.coordinator
                .observe_p05_unit_receipts("2026-09-24")
                .unwrap(),
            day2
        );
        assert_eq!(
            f.query_i64("SELECT baseline_revision FROM p05_baseline_heads"),
            1
        );
        assert_eq!(
            f.query_i64("SELECT COUNT(*) FROM p05_s2_completion_receipts"),
            1
        );
        assert_eq!(f.query_blob("SELECT result_canonical FROM sink_results s JOIN delivery_decisions d ON d.decision_identity=s.decision_identity WHERE d.business_date='2026-09-24' ORDER BY s.rowid LIMIT 1"), raw);
        assert!(f
            .coordinator
            .finalize_p05_unit_observed("2026-09-24", &day2)
            .unwrap()
            .completion_identity()
            .is_some());
        assert_eq!(
            f.query_i64("SELECT baseline_revision FROM p05_baseline_heads"),
            2
        );
        assert_eq!(sink.calls.load(Ordering::SeqCst), 2);
        assert_eq!(prior_sink.calls.load(Ordering::SeqCst), 2);
    }
}
#[test]
fn p05_shared_unit_runtime_prior_closure_true_postcommit_keeps_facts_and_returns_error() {
    for opening in ["draft", "prepare", "begin", "finalize"] {
        let f = Fixture::new("S2_PRIOR_POSTCOMMIT");
        let (_dir, db) = operational();
        let append = Append::default();
        let prior_sink = completed_prior(&f, &db, &append);
        let draft = if opening == "draft" {
            None
        } else {
            Some(next_draft(&f))
        };
        let intent = draft.as_ref().map(|d| complete_intent(&f, &db, d));
        let decision = if opening == "begin" {
            let (_, envelope) = f
                .coordinator
                .prepare_p05_unit_child_local(&db, "2026-09-24", 0, 1, Some(at(24, 9, 22, 0)))
                .unwrap();
            f.coordinator
                .reconcile_all_pending(&append, at(24, 9, 22, 0))
                .unwrap();
            Some(envelope.decision_identity)
        } else {
            None
        };
        let sink = if opening == "finalize" {
            Some(deliver_all(&f, &db, intent.as_ref().unwrap(), 24, &append))
        } else {
            None
        };
        let observed = if opening == "finalize" {
            Some(
                f.coordinator
                    .observe_p05_unit_receipts("2026-09-24")
                    .unwrap(),
            )
        } else {
            None
        };
        let hit = arm_actual_prior_postcommit(&f);
        let result = match opening {
            "draft" => f
                .coordinator
                .store_p05_observed_draft_rendered(
                    &input(24, &["TEST_CODE_600001", "TEST_CODE_000001"], false),
                    Some(at(24, 9, 24, 0)),
                    &mut renderer,
                )
                .map(|_| ()),
            "prepare" => f
                .coordinator
                .prepare_p05_unit_child_local(&db, "2026-09-24", 0, 1, Some(at(24, 9, 22, 0)))
                .map(|_| ()),
            "begin" => begin_on(&f, &db, decision.as_ref().unwrap(), at(24, 9, 22, 1)).map(|_| ()),
            "finalize" => f
                .coordinator
                .finalize_p05_unit_observed("2026-09-24", observed.as_ref().unwrap())
                .map(|_| ()),
            _ => unreachable!(),
        };
        let error = result.unwrap_err();
        assert!(
            error.to_string().contains("COMMIT succeeded")
                && error.to_string().contains("prior current closure"),
            "{opening}: {error}"
        );
        assert_eq!(hit.load(Ordering::SeqCst), 1);
        assert!(f
            .coordinator
            .read_p05_unit_draft("2026-09-24")
            .unwrap()
            .is_some());
        assert!(f
            .coordinator
            .observe_p05_unit_receipts(DATE)
            .unwrap()
            .completion_identity()
            .is_none());
        assert_eq!(
            f.query_i64("SELECT baseline_revision FROM p05_baseline_heads"),
            if opening == "finalize" { 2 } else { 1 }
        );
        assert_eq!(
            f.query_i64("SELECT COUNT(*) FROM p05_s2_completion_receipts"),
            if opening == "finalize" { 2 } else { 1 }
        );
        assert_eq!(f.query_i64("SELECT COUNT(*) FROM p05_s2_child_owners o JOIN p05_unit_drafts d ON d.draft_identity=o.draft_identity WHERE d.business_date='2026-09-24'"), match opening {"draft"=>0,"prepare"|"begin"=>1,"finalize"=>2,_=>unreachable!()});
        if let Some(decision) = decision {
            assert_eq!(
                f.coordinator.decision_state(&decision).unwrap(),
                DecisionState::AttemptInFlight
            );
        }
        if let Some(sink) = sink {
            assert_eq!(sink.calls.load(Ordering::SeqCst), 2);
        }
        assert_eq!(prior_sink.calls.load(Ordering::SeqCst), 2);
    }
}
#[test]
fn p05_shared_unit_runtime_dirty_prior_keeps_existing_prepare_raw_audit_and_reclosure_reachable() {
    let f = Fixture::new("S2_PRIOR_RECOVERY");
    let (_dir, db) = operational();
    let append = Append::default();
    let prior_sink = completed_prior(&f, &db, &append);
    let draft = next_draft(&f);
    complete_intent(&f, &db, &draft);
    let original = f
        .coordinator
        .prepare_p05_unit_child_local(&db, "2026-09-24", 0, 1, Some(at(24, 9, 22, 0)))
        .unwrap();
    f.coordinator
        .reconcile_all_pending(&append, at(24, 9, 22, 0))
        .unwrap();
    let attempt = begin_on(&f, &db, &original.1.decision_identity, at(24, 9, 22, 1))
        .unwrap()
        .unwrap();
    let (prior_attempt,prior_fence):(String,i64)=Connection::open(&f.database_path).unwrap().query_row("SELECT a.attempt_identity,a.fence_token FROM delivery_attempts a JOIN delivery_decisions d ON d.decision_identity=a.decision_identity WHERE d.business_date='2026-09-23' ORDER BY a.rowid LIMIT 1",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    f.coordinator
        .record_sink_result(
            &prior_attempt,
            prior_fence,
            accepted(at(24, 9, 22, 2)),
            at(24, 9, 22, 2),
        )
        .unwrap();
    assert!(f
        .coordinator
        .observe_p05_unit_receipts(DATE)
        .unwrap()
        .completion_identity()
        .is_none());
    let before=f.query_i64("SELECT mutation_revision FROM p05_unit_heads WHERE draft_identity=(SELECT draft_identity FROM p05_unit_drafts WHERE business_date='2026-09-24')");
    let existing = f
        .coordinator
        .prepare_p05_unit_child_local(&db, "2026-09-24", 0, 1, Some(at(24, 23, 0, 0)))
        .unwrap();
    assert_eq!(existing.1, original.1);
    assert_eq!(f.query_i64("SELECT mutation_revision FROM p05_unit_heads WHERE draft_identity=(SELECT draft_identity FROM p05_unit_drafts WHERE business_date='2026-09-24')"),before);
    f.coordinator
        .record_sink_result(
            &attempt.attempt_identity,
            attempt.fence_token,
            accepted(at(24, 9, 22, 3)),
            at(24, 9, 22, 3),
        )
        .unwrap();
    f.coordinator
        .reconcile_all_pending(&append, at(24, 9, 24, 0))
        .unwrap();
    assert!(matches!(
        f.coordinator
            .observe_p05_unit_receipts("2026-09-24")
            .unwrap()
            .children()[0],
        P05ChildReceiptObservation::PhysicallyAccepted { .. }
    ));
    assert!(finalize(&f, DATE).completion_identity().is_some());
    assert_eq!(
        f.query_i64("SELECT baseline_revision FROM p05_baseline_heads"),
        1
    );
    assert_eq!(prior_sink.calls.load(Ordering::SeqCst), 2);
}

fn corrupt_actual_score(dir: &tempfile::TempDir) {
    Connection::open(dir.path().join("TEST_CODE_P05_S2.db"))
        .unwrap()
        .execute(
            "UPDATE prediction_tracker SET pred_score=81 WHERE stock_code='TEST_CODE_600001'",
            [],
        )
        .unwrap();
}
fn owned_route(f: &Fixture, decision: &str) -> DecisionMutationRoute {
    f.coordinator.decision_mutation_route(decision).unwrap()
}
fn owned_revision(f: &Fixture) -> i64 {
    f.query_i64("SELECT mutation_revision FROM p05_unit_heads ORDER BY rowid LIMIT 1")
}
fn prepare_first(f: &Fixture, db: &DatabaseManager) -> DeliveryEnvelope {
    let draft = start(f, db, true);
    complete_intent(f, db, &draft);
    let (_, envelope) = f
        .coordinator
        .prepare_p05_unit_child_local(db, DATE, 0, 1, Some(at(23, 9, 22, 0)))
        .unwrap();
    f.coordinator
        .reconcile_all_pending(&Append::default(), at(23, 9, 22, 0))
        .unwrap();
    envelope
}

#[test]
fn p05_shared_unit_consumer_isolated_leaf_aliases_rejected_before_any_sqlite_write() {
    use std::os::unix::fs::symlink;
    for suffix in ["", "-wal", "-shm", "-journal"] {
        for hard in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let target = dir.path().join("TEST_CODE_foreign_bytes");
            let bytes = b"TEST_CODE unchanged foreign bytes, not SQLite";
            std::fs::write(&target, bytes).unwrap();
            let path = dir.path().join("TEST_CODE_alias.db");
            let leaf = dir.path().join(format!("TEST_CODE_alias.db{suffix}"));
            if hard {
                std::fs::hard_link(&target, &leaf).unwrap();
            } else {
                symlink(&target, &leaf).unwrap();
            }
            let entries = || {
                std::fs::read_dir(dir.path())
                    .unwrap()
                    .map(|e| e.unwrap().file_name())
                    .collect::<std::collections::BTreeSet<_>>()
            };
            let before = entries();
            assert!(
                DatabaseManager::open_isolated_for_test(path).is_err(),
                "{suffix}, hard={hard}"
            );
            assert_eq!(std::fs::read(&target).unwrap(), bytes);
            assert_eq!(
                entries(),
                before,
                "rejection must not create main/sidecar files"
            );
        }
    }
}

#[test]
fn p05_shared_unit_consumer_isolated_parent_alias_outside_temp_rejected_without_write() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new("S3_OUTSIDE_TEMP");
    let outside = tempfile::tempdir_in(f.database_path.parent().unwrap()).unwrap();
    assert!(!outside
        .path()
        .canonicalize()
        .unwrap()
        .starts_with(std::env::temp_dir().canonicalize().unwrap()));
    let dir = tempfile::tempdir().unwrap();
    let alias = dir.path().join("TEST_CODE_outside");
    symlink(outside.path().canonicalize().unwrap(), &alias).unwrap();
    assert!(DatabaseManager::open_isolated_for_test(alias.join("TEST_CODE_escaped.db")).is_err());
    assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
}

#[test]
fn p05_shared_unit_consumer_constructor_origin_rechecks_replaced_main_before_business_gate() {
    let f = Fixture::new("S3_REPLACED_MAIN");
    let (dir, db) = operational();
    let envelope = prepare_first(&f, &db);
    let path = dir.path().join("TEST_CODE_P05_S2.db");
    std::fs::rename(&path, dir.path().join("TEST_CODE_retained.db")).unwrap();
    let foreign = b"TEST_CODE foreign replacement must never be read as original DB";
    std::fs::write(&path, foreign).unwrap();
    assert!(!db.has_isolated_p05_consumer_origin());
    let check = f
        .coordinator
        .p05_consumer_check_on(&db, &owned_route(&f, &envelope.decision_identity));
    let before = owned_revision(&f);
    assert!(f
        .coordinator
        .begin_attempt_with_p05_check(
            &envelope.decision_identity,
            1,
            at(23, 9, 22, 1),
            Some(&check)
        )
        .is_err());
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM delivery_attempts"), 0);
    assert_eq!(owned_revision(&f), before);
    assert_eq!(std::fs::read(&path).unwrap(), foreign);
}

#[test]
fn p05_shared_unit_consumer_generic_startup_and_direct_begin_block_but_actual_test_cap_sends_once()
{
    let f = Fixture::new("S3_GENERIC_OPEN");
    let (_dir, db) = operational();
    let envelope = prepare_first(&f, &db);
    let before = owned_revision(&f);
    let (port, sinks) = sink(accepted(at(23, 9, 22, 1)));
    assert!(f
        .coordinator
        .begin_attempt(&envelope.decision_identity, 1, at(23, 9, 22, 1))
        .is_err());
    assert!(f
        .coordinator
        .resume_deliverable(&envelope.decision_identity, &sinks, at(23, 9, 22, 1))
        .is_err());
    assert_eq!(owned_revision(&f), before);
    assert_eq!(port.calls.load(Ordering::SeqCst), 0);
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM delivery_attempts"), 0);
    let check = f
        .coordinator
        .p05_consumer_check_on(&db, &owned_route(&f, &envelope.decision_identity));
    let opened = f
        .coordinator
        .resume_deliverable_with_p05_check(
            &envelope.decision_identity,
            &sinks,
            at(23, 9, 22, 1),
            Some(&check),
        )
        .unwrap();
    assert_eq!(opened.sink_calls, 1);
    assert_eq!(port.calls.load(Ordering::SeqCst), 1);
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM delivery_attempts"), 1);
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM sink_results"), 1);
    assert_eq!(owned_revision(&f), before + 2); // original begin + original raw result
    let after = owned_revision(&f);
    let again = f
        .coordinator
        .resume_deliverable(&envelope.decision_identity, &sinks, at(23, 9, 22, 2))
        .unwrap();
    assert_eq!(again.sink_calls, 0);
    assert_eq!(owned_revision(&f), after);
    assert_eq!(port.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn p05_shared_unit_consumer_actual_cap_cannot_cross_identical_namespace_or_member() {
    let f = Fixture::new("S3_CAP_FIRST");
    let other = Fixture::new("S3_CAP_OTHER");
    let (_one, db) = operational();
    let (_two, other_db) = operational();
    let envelope = prepare_first(&f, &db);
    let foreign_envelope = prepare_first(&other, &other_db);
    assert_eq!(envelope, foreign_envelope);
    let check = f
        .coordinator
        .p05_consumer_check_on(&db, &owned_route(&f, &envelope.decision_identity));
    assert!(matches!(check, ActualP05ConsumerCheck::Verified(_)));
    let foreign_before = owned_revision(&other);
    assert!(other
        .coordinator
        .begin_attempt_with_p05_check(
            &foreign_envelope.decision_identity,
            1,
            at(23, 9, 22, 1),
            Some(&check)
        )
        .is_err());
    assert_eq!(owned_revision(&other), foreign_before);
    assert_eq!(other.query_i64("SELECT COUNT(*) FROM delivery_attempts"), 0);
    let (_, board) = f
        .coordinator
        .prepare_p05_unit_child_local(&db, DATE, 1, 1, Some(at(23, 9, 22, 1)))
        .unwrap();
    f.coordinator
        .reconcile_all_pending(&Append::default(), at(23, 9, 22, 1))
        .unwrap();
    let before = owned_revision(&f);
    assert!(f
        .coordinator
        .begin_attempt_with_p05_check(&board.decision_identity, 1, at(23, 9, 22, 2), Some(&check))
        .is_err());
    assert_eq!(owned_revision(&f), before);
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM delivery_attempts"), 0);
    let view = f
        .coordinator
        .inspect_p05_unit_child_global(DATE, 0)
        .unwrap();
    assert!(other
        .coordinator
        .prepare_p05_child_inspection(&view, 1)
        .is_err());
}

#[test]
fn p05_shared_unit_consumer_heartbeat_reader_failure_preserves_noop_and_late_raw_audit() {
    let f = Fixture::new("S3_HEARTBEAT");
    let (dir, db) = operational();
    let envelope = prepare_first(&f, &db);
    let attempt = begin_on(&f, &db, &envelope.decision_identity, at(23, 9, 22, 1))
        .unwrap()
        .unwrap();
    corrupt_actual_score(&dir);
    let before = owned_revision(&f);
    assert!(f
        .coordinator
        .heartbeat_attempt(
            &envelope.decision_identity,
            &attempt.attempt_identity,
            attempt.fence_token,
            at(23, 9, 22, 1)
        )
        .unwrap());
    assert!(!f
        .coordinator
        .heartbeat_attempt(
            &envelope.decision_identity,
            &attempt.attempt_identity,
            attempt.fence_token + 1,
            at(23, 9, 22, 2)
        )
        .unwrap());
    let check = f
        .coordinator
        .p05_consumer_check_on(&db, &owned_route(&f, &envelope.decision_identity));
    assert!(matches!(check, ActualP05ConsumerCheck::Failed(_)));
    assert!(f
        .coordinator
        .heartbeat_attempt_with_p05_check(
            &envelope.decision_identity,
            &attempt.attempt_identity,
            attempt.fence_token,
            at(23, 9, 22, 2),
            Some(&check)
        )
        .is_err());
    assert_eq!(owned_revision(&f), before);
    let original = accepted(at(23, 9, 22, 3));
    f.coordinator
        .record_sink_result(
            &attempt.attempt_identity,
            attempt.fence_token,
            original,
            at(23, 9, 22, 3),
        )
        .unwrap();
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM sink_results"), 1);
    assert_eq!(owned_revision(&f), before + 1);
    f.coordinator
        .reconcile_all_pending(&Append::default(), at(23, 9, 22, 4))
        .unwrap();
    let read = f.coordinator.observe_p05_unit_receipts(DATE).unwrap();
    assert!(matches!(
        read.children()[0],
        P05ChildReceiptObservation::PhysicallyAccepted { .. }
    ));
}

#[test]
fn p05_shared_unit_consumer_authorize_and_reacquire_consume_reader_only_at_real_opening() {
    for automatic_retry in [false, true] {
        let f = Fixture::new("S3_RETRY");
        let (dir, db) = operational();
        let envelope = prepare_first(&f, &db);
        let check = f
            .coordinator
            .p05_consumer_check_on(&db, &owned_route(&f, &envelope.decision_identity));
        let rejected = AuthoritativeSinkResult::Rejected(crate::durable_delivery::TypedRejection {
            reason_code: "TEST_CODE_RETRY".into(),
            evidence: b"TEST_CODE original rejection".to_vec(),
            retry_authorized: automatic_retry,
            observed_at: at(23, 9, 22, 1),
        });
        let (port, sinks) = sink(rejected);
        f.coordinator
            .resume_deliverable_with_p05_check(
                &envelope.decision_identity,
                &sinks,
                at(23, 9, 22, 1),
                Some(&check),
            )
            .unwrap();
        let setup_append = Append::default();
        f.coordinator
            .reconcile_all_pending(&setup_append, at(23, 9, 22, 1))
            .unwrap();
        assert_eq!(
            f.coordinator
                .decision_state(&envelope.decision_identity)
                .unwrap(),
            DecisionState::RejectedDurable
        );
        corrupt_actual_score(&dir);
        let failed = f
            .coordinator
            .p05_consumer_check_on(&db, &owned_route(&f, &envelope.decision_identity));
        assert!(matches!(failed, ActualP05ConsumerCheck::Failed(_)));
        let before = owned_revision(&f);
        if automatic_retry {
            // Already-authorized is a true NoChange, not a new grant.
            f.coordinator
                .authorize_rejected_retry(&envelope.decision_identity)
                .unwrap();
            assert!(f
                .coordinator
                .resume_deliverable_with_p05_check(
                    &envelope.decision_identity,
                    &sinks,
                    at(23, 9, 22, 2),
                    Some(&failed)
                )
                .is_err());
        } else {
            let outcome = f
                .coordinator
                .resume_deliverable(&envelope.decision_identity, &sinks, at(23, 9, 22, 2))
                .unwrap();
            assert_eq!(outcome.sink_calls, 0);
            assert!(f
                .coordinator
                .authorize_rejected_retry_with_p05_check(&envelope.decision_identity, Some(&failed))
                .is_err());
        }
        assert_eq!(owned_revision(&f), before);
        assert_eq!(port.calls.load(Ordering::SeqCst), 1);
        assert_eq!(f.query_i64("SELECT COUNT(*) FROM delivery_attempts"), 1);
        assert_eq!(f.query_i64("SELECT COUNT(*) FROM sink_results"), 1);
    }
}

#[test]
fn p05_shared_unit_consumer_manual_resolution_reader_failure_keeps_external_authorization() {
    let f = Fixture::new("S3_MANUAL_GATE");
    let (dir, db) = operational();
    let envelope = prepare_first(&f, &db);
    let check = f
        .coordinator
        .p05_consumer_check_on(&db, &owned_route(&f, &envelope.decision_identity));
    let (_, sinks) = sink(AuthoritativeSinkResult::Uncertain(
        crate::durable_delivery::TypedUncertainty {
            reason_code: "TEST_CODE_UNKNOWN".into(),
            evidence: b"TEST_CODE original uncertain".to_vec(),
            observed_at: at(23, 9, 22, 1),
        },
    ));
    f.coordinator
        .resume_deliverable_with_p05_check(
            &envelope.decision_identity,
            &sinks,
            at(23, 9, 22, 1),
            Some(&check),
        )
        .unwrap();
    let setup_append = Append::default();
    f.coordinator
        .reconcile_all_pending(&setup_append, at(23, 9, 22, 1))
        .unwrap();
    assert_eq!(
        f.coordinator
            .decision_state(&envelope.decision_identity)
            .unwrap(),
        DecisionState::UncertainManualReview
    );
    corrupt_actual_score(&dir);
    let failed = f
        .coordinator
        .p05_consumer_check_on(&db, &owned_route(&f, &envelope.decision_identity));
    assert!(matches!(
        failed,
        ActualP05ConsumerCheck::Failed(
            "P05 actual operational freeze/score unavailable or mismatched"
        )
    ));
    let append = Append::default();
    let before = owned_revision(&f);
    let command = crate::durable_delivery::ManualResolutionCommand {
        decision_identity: envelope.decision_identity.clone(),
        disposition: crate::durable_delivery::ManualDisposition::Rejected,
        operator_identity: "TEST_CODE_OPERATOR_S3_0123456789abcdef".into(),
        reason: "TEST_CODE operator rejected".into(),
        external_evidence: b"TEST_CODE independent authorization bytes".to_vec(),
        resolved_at: at(23, 9, 23, 0),
    };
    let error = f
        .coordinator
        .resolve_uncertain_with_p05_check(&command, &append, Some(&failed))
        .unwrap_err();
    assert!(matches!(
        error,
        DurableDeliveryError::PolicyMismatch(reason)
            if reason == "P05 preparation: P05 actual operational freeze/score unavailable or mismatched"
    ));
    assert_eq!(append.calls.load(Ordering::SeqCst), 1);
    let records = append.rows.lock().unwrap();
    assert_eq!(records.len(), 1);
    assert!(records
        .values()
        .next()
        .unwrap()
        .1
        .contains("ManualResolutionAuthorization"));
    assert_eq!(owned_revision(&f), before);
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM manual_resolutions"), 0);
    assert_eq!(
        f.coordinator
            .decision_state(&envelope.decision_identity)
            .unwrap(),
        DecisionState::UncertainManualReview
    );
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM sink_results"), 1);
}

#[tokio::test]
async fn p05_shared_unit_consumer_public_passive_restore_existing_accepted_reclose_without_resampling(
) {
    use crate::p05_auction_unit::P05AuctionUnit;
    let f = Fixture::new("S3_PASSIVE_RESTORE");
    let (dir, db) = operational();
    let draft = start(&f, &db, true);
    let intent = complete_intent(&f, &db, &draft);
    let append = Append::default();
    let port = deliver_all(&f, &db, &intent, 23, &append);
    let complete = finalize(&f, DATE);
    corrupt_actual_score(&dir);
    let counted = fixture_coordinator_arc(&f);
    let unit = P05AuctionUnit::restore(counted.clone(), DATE)
        .await
        .unwrap()
        .unwrap();
    unit.require_counted_owner(&counted).unwrap();
    let foreign = f.second_coordinator("S3_FOREIGN_ARC");
    assert!(unit.require_counted_owner(&foreign).is_err());
    let view = unit.inspect_child(0).unwrap();
    assert_eq!(
        view.envelope().canonical_bytes().unwrap(),
        intent.children[0].1
    );
    view.require_counted_owner(&counted).unwrap();
    assert!(view.require_counted_owner(&foreign).is_err());
    let before = owned_revision(&f);
    let prepared = view.prepare_child(1).unwrap();
    assert_eq!(prepared.envelope(), view.envelope());
    assert!(prepared.require_counted_owner(&foreign).is_err());
    let (_, sinks) = sink(accepted(at(23, 9, 24, 0)));
    let noop = prepared.resume(&sinks).unwrap();
    assert_eq!(noop.sink_calls, 0);
    assert_eq!(owned_revision(&f), before);
    assert_eq!(port.calls.load(Ordering::SeqCst), 2);
    let (attempt, fence): (String, i64) = Connection::open(&f.database_path)
        .unwrap()
        .query_row(
            "SELECT attempt_identity,fence_token FROM delivery_attempts ORDER BY rowid LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let raw_before = f.query_i64("SELECT COUNT(*) FROM sink_results");
    f.coordinator
        .record_sink_result(
            &attempt,
            fence,
            accepted(at(23, 9, 24, 1)),
            at(23, 9, 24, 1),
        )
        .unwrap();
    f.coordinator
        .reconcile_all_pending(&append, at(23, 9, 24, 2))
        .unwrap();
    let observed = unit.observe().unwrap();
    assert!(observed.completion_identity().is_none());
    let reclosed = unit.finalize(&observed).unwrap();
    assert!(reclosed.completion_identity().is_some());
    assert_ne!(
        reclosed.completion_identity(),
        complete.completion_identity()
    );
    assert_eq!(
        f.query_i64("SELECT baseline_revision FROM p05_baseline_heads"),
        1
    );
    assert_eq!(
        f.query_i64("SELECT COUNT(*) FROM sink_results"),
        raw_before + 1
    );
    assert_eq!(port.calls.load(Ordering::SeqCst), 2);
    assert_eq!(db.count_predictions().unwrap(), 2);
    // Public fresh production work must never adopt Test/global fixture DB.
    assert!(P05AuctionUnit::initialize_prospective_family(&f.coordinator).is_err());
}

#[test]
fn p05_shared_unit_consumer_readonly_view_original_metadata_and_no_authority_before_prepare() {
    let f = Fixture::new("S3_READONLY_VIEW");
    let (_dir, db) = operational();
    let draft = start(&f, &db, true);
    let intent = complete_intent(&f, &db, &draft);
    let before = owned_revision(&f);
    for (ordinal, (child, raw)) in intent.children.iter().enumerate() {
        let view = f
            .coordinator
            .inspect_p05_unit_child_global(DATE, ordinal)
            .unwrap();
        assert_eq!(view.ordinal(), ordinal);
        assert_eq!(view.business_date(), DATE);
        assert_eq!(view.child_identity(), child);
        assert_eq!(view.unit_identity(), draft.identity());
        assert_eq!(view.intent_identity(), intent.identity());
        assert_eq!(view.envelope().canonical_bytes().unwrap(), *raw);
        assert_eq!(view.governance_code(), None);
        // Test's public/global path cannot grant a new owner or reservation.
        assert!(f
            .coordinator
            .prepare_p05_child_inspection(&view, 1)
            .is_err());
    }
    assert_eq!(owned_revision(&f), before);
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM delivery_decisions"), 0);
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM p05_s2_child_owners"), 0);
    assert_eq!(
        f.query_i64("SELECT COUNT(*) FROM daily_budget_reservations"),
        0
    );
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM sink_results"), 0);
}

#[test]
fn p05_shared_unit_consumer_readonly_list_phases_complete_exclusion_and_dirty_order() {
    let f = Fixture::new("S3_LIST_PHASES");
    let (_dir, db) = operational();
    let draft = start(&f, &db, false);
    let list = f.coordinator.inspect_p05_unfinished_units().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].phase(), P05StoredUnitPhase::Draft);
    assert_eq!(list[0].mutation_revision(), 1);
    assert_eq!(list[0].unit_identity(), draft.identity());
    assert_eq!(list[0].intent_identity(), None);
    let started = f.coordinator.claim_p05_prediction_prepare(&draft).unwrap();
    let list = f.coordinator.inspect_p05_unfinished_units().unwrap();
    assert_eq!(list[0].phase(), P05StoredUnitPhase::Started);
    assert_eq!(list[0].mutation_revision(), 2);
    let intent = f
        .coordinator
        .complete_p05_unit_intent_on(&draft, &started, &db)
        .unwrap();
    let list = f.coordinator.inspect_p05_unfinished_units().unwrap();
    assert_eq!(list[0].phase(), P05StoredUnitPhase::IntentComplete);
    assert_eq!(list[0].intent_identity(), Some(intent.identity()));
    let append = Append::default();
    let port = deliver_all(&f, &db, &intent, 23, &append);
    let completed = finalize(&f, DATE);
    assert!(f
        .coordinator
        .inspect_p05_unfinished_units()
        .unwrap()
        .is_empty());
    next_draft(&f);
    assert_eq!(
        f.coordinator.inspect_p05_unfinished_units().unwrap()[0].business_date(),
        "2026-09-24"
    );
    let (attempt, fence): (String, i64) = Connection::open(&f.database_path)
        .unwrap()
        .query_row(
            "SELECT attempt_identity,fence_token FROM delivery_attempts ORDER BY rowid LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    f.coordinator
        .record_sink_result(
            &attempt,
            fence,
            accepted(at(24, 9, 22, 0)),
            at(24, 9, 22, 0),
        )
        .unwrap();
    let list = f.coordinator.inspect_p05_unfinished_units().unwrap();
    assert_eq!(
        list.iter().map(|s| s.business_date()).collect::<Vec<_>>(),
        [DATE, "2026-09-24"]
    );
    assert_eq!(
        list[0].first_completion_identity(),
        completed.completion_identity()
    );
    assert!(list[0].mutation_revision() > completed.mutation_revision());
    assert_eq!(list[1].phase(), P05StoredUnitPhase::Draft);
    assert_eq!(port.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn p05_shared_unit_consumer_readonly_list_postread_mutation_cannot_return_stale_empty() {
    let f = Fixture::new("S3_LIST_BOUNDARY");
    let (_dir, db) = operational();
    let append = Append::default();
    let port = completed_prior(&f, &db, &append);
    assert!(f
        .coordinator
        .inspect_p05_unfinished_units()
        .unwrap()
        .is_empty());
    let path = f.database_path.clone();
    let hit = Arc::new(AtomicUsize::new(0));
    let callback_hit = hit.clone();
    f.coordinator
        .install_database_operation_test_hook(
            DatabaseOperationTestPhase::AfterSqlBeforePostValidation,
            move || {
                callback_hit.fetch_add(1, Ordering::SeqCst);
                let mut c = Connection::open(path)?;
                crate::durable_delivery::schema::register_sha256_function(&c)?;
                c.pragma_update(None, "foreign_keys", "ON")?;
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                actual_extra_mutation_for_test(&tx)?;
                tx.commit()?;
                Ok(())
            },
        )
        .unwrap();
    let error = f.coordinator.inspect_p05_unfinished_units().unwrap_err();
    assert!(
        error.to_string().contains("readonly Unit list membership"),
        "{error}"
    );
    assert_eq!(hit.load(Ordering::SeqCst), 1);
    let list = f.coordinator.inspect_p05_unfinished_units().unwrap();
    assert_eq!(list.len(), 1);
    assert!(list[0].first_completion_identity().is_some());
    assert_eq!(port.calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        f.query_i64("SELECT COUNT(*) FROM p05_s2_completion_receipts"),
        1
    );
    assert_eq!(
        f.query_i64("SELECT baseline_revision FROM p05_baseline_heads"),
        1
    );
}

#[test]
fn p05_shared_unit_consumer_owned_classifier_validates_actual_owner_unowned_v1_keeps_original_route(
) {
    let f = Fixture::new("S3_CLASSIFIER");
    let (_dir, db) = operational();
    let envelope = prepare_first(&f, &db);
    let inspected = f
        .coordinator
        .inspect_p05_owned_child_global(&envelope.decision_identity)
        .unwrap()
        .unwrap();
    assert_eq!(inspected.envelope(), &envelope);
    assert_eq!(inspected.ordinal(), 0);
    let legacy = Fixture::new("S3_UNOWNED_V1");
    let outcome = legacy
        .coordinator
        .prepare(&envelope, 1, at(23, 9, 22, 0))
        .unwrap();
    assert_eq!(outcome.state, DecisionState::Reserved);
    assert!(legacy
        .coordinator
        .inspect_p05_owned_child_global(&envelope.decision_identity)
        .unwrap()
        .is_none());
    legacy
        .coordinator
        .reconcile_all_pending(&Append::default(), at(23, 9, 22, 0))
        .unwrap();
    let (port, sinks) = sink(accepted(at(23, 9, 22, 1)));
    let original = legacy
        .coordinator
        .resume_deliverable(&envelope.decision_identity, &sinks, at(23, 9, 22, 1))
        .unwrap();
    assert_eq!(original.sink_calls, 1);
    assert_eq!(port.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        legacy.query_i64("SELECT COUNT(*) FROM p05_s2_child_owners"),
        0
    );
    // An invalid owned catalog must return an error, never classify as v1.
    Connection::open(&f.database_path)
        .unwrap()
        .execute("DROP TABLE p05_s2_child_owners", [])
        .unwrap();
    assert!(f
        .coordinator
        .inspect_p05_owned_child_global(&envelope.decision_identity)
        .is_err());
}

#[test]
fn p05_shared_unit_consumer_global_or_production_tag_cannot_issue_test_capability() {
    let f = Fixture::new("S3_ORIGIN_GATE");
    let (_dir, db) = operational();
    let envelope = prepare_first(&f, &db);
    DatabaseManager::init(None).unwrap();
    let check = f.coordinator.p05_consumer_check_on(
        DatabaseManager::get(),
        &owned_route(&f, &envelope.decision_identity),
    );
    assert!(matches!(check, ActualP05ConsumerCheck::Failed(_)));
    let before = owned_revision(&f);
    assert!(f
        .coordinator
        .begin_attempt_with_p05_check(
            &envelope.decision_identity,
            1,
            at(23, 9, 22, 1),
            Some(&check)
        )
        .is_err());
    assert_eq!(owned_revision(&f), before);
    let mut config = f.coordinator.config.clone();
    config.environment = super::super::super::super::model::StoreEnvironment::Production;
    let bytes = std::fs::read(&f.database_path).unwrap();
    assert!(DurableDeliveryCoordinator::open(config).is_err());
    assert_eq!(std::fs::read(&f.database_path).unwrap(), bytes);
}

#[test]
fn p05_shared_unit_consumer_readonly_view_postread_actual_mutation_rejected_without_losing_fact() {
    let f = Fixture::new("S3_VIEW_BOUNDARY");
    let (_dir, db) = operational();
    let envelope = prepare_first(&f, &db);
    let original = f
        .coordinator
        .inspect_p05_unit_child_global(DATE, 0)
        .unwrap();
    let path = f.database_path.clone();
    let hit = Arc::new(AtomicUsize::new(0));
    let callback_hit = hit.clone();
    f.coordinator
        .install_database_operation_test_hook(
            DatabaseOperationTestPhase::AfterSqlBeforePostValidation,
            move || {
                callback_hit.fetch_add(1, Ordering::SeqCst);
                let mut c = Connection::open(path)?;
                crate::durable_delivery::schema::register_sha256_function(&c)?;
                c.pragma_update(None, "foreign_keys", "ON")?;
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                actual_extra_mutation_for_test(&tx)?;
                tx.commit()?;
                Ok(())
            },
        )
        .unwrap();
    let error = match f.coordinator.inspect_p05_unit_child_global(DATE, 0) {
        Ok(_) => panic!("stale readonly child snapshot returned"),
        Err(error) => error,
    };
    assert!(
        error.to_string().contains("exact Unit SQL binding"),
        "{error}"
    );
    assert_eq!(hit.load(Ordering::SeqCst), 1);
    let current = f
        .coordinator
        .inspect_p05_unit_child_global(DATE, 0)
        .unwrap();
    assert_eq!(current.envelope(), &envelope);
    assert_eq!(current.envelope(), original.envelope());
    assert_eq!(
        f.query_i64(
            "SELECT COUNT(*) FROM immutable_audit_outbox WHERE audit_kind='LateReceiptObserved'"
        ),
        1
    );
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM p05_s2_child_owners"), 1);
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM delivery_attempts"), 0);
}

#[test]
fn p05_shared_unit_consumer_crossday_view_retains_actual_t08_ordinal_scope_source_and_occurrence() {
    let f = Fixture::new("S3_T08_VIEW");
    let (_dir, db) = operational();
    let append = Append::default();
    completed_prior(&f, &db, &append);
    let next = f
        .coordinator
        .store_p05_observed_draft_rendered(
            &input(24, &["TEST_CODE_300001"], false),
            Some(at(24, 9, 24, 0)),
            &mut renderer,
        )
        .unwrap();
    let intent = complete_intent(&f, &db, &next);
    let before = owned_revision(&f);
    for (index, kind) in [
        PushKind::AuctionRepush,
        PushKind::CandidateInvalidated,
        PushKind::CandidateInvalidated,
        PushKind::CandidateBoard,
    ]
    .into_iter()
    .enumerate()
    {
        let view = f
            .coordinator
            .inspect_p05_unit_child_global("2026-09-24", index)
            .unwrap();
        assert_eq!(view.envelope().push_kind, kind);
        assert_eq!(view.ordinal(), index);
        assert_eq!(
            view.envelope().canonical_bytes().unwrap(),
            intent.children[index].1
        );
        if kind == PushKind::CandidateInvalidated {
            let code = if index == 1 {
                "TEST_CODE_000001"
            } else {
                "TEST_CODE_600001"
            };
            assert_eq!(view.governance_code(), Some(code));
            assert_eq!(
                view.envelope().schedule_occurrence_identity,
                format!("candidate-invalidated:2026-09-24:{code}")
            );
            assert!(view.envelope().retry_authorized);
            let source: serde_json::Value =
                serde_json::from_slice(&view.envelope().source_binding_canonical).unwrap();
            assert_eq!(source["code"], code);
            assert_eq!(source["prev"], "候选");
            assert_eq!(source["reason"], "从候选台消失");
        } else {
            assert_eq!(view.governance_code(), None);
        }
    }
    assert_eq!(owned_revision(&f), before);
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM p05_s2_child_owners"), 2);
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM sink_results"), 2);
}

fn outcome_report(
    db: &DatabaseManager,
    f: &Fixture,
    target: &str,
) -> std::result::Result<crate::monitor::prediction::OutcomeDailyWeeklyObservation, &'static str> {
    crate::monitor::prediction::OutcomeTracker::new(db, Some(&f.coordinator))
        .read_at_for_test(format!("{target}T15:01:00+08:00").parse().unwrap())
}
fn outcome_target(db: &DatabaseManager, draft: &StoredP05Draft) -> String {
    db.read_candidate_board_v2_freeze(&draft.board_occurrence().unwrap())
        .unwrap()
        .unwrap()
        .target_date()
        .into()
}
fn outcome_linked(
    report: &crate::monitor::prediction::OutcomeDailyWeeklyObservation,
) -> &crate::monitor::prediction::PhysicalLinkedOutcomeCounts {
    match &report.daily.physical_linked {
        crate::monitor::prediction::PhysicalLinkedOutcomeObservation::Observed(c) => c,
        other => panic!("actual original Unit linkage unavailable: {other:?}"),
    }
}
fn deliver_board_only(
    f: &Fixture,
    db: &DatabaseManager,
    intent: &StoredP05Intent,
    append: &Append,
) -> Arc<Sink> {
    let index = intent
        .children
        .iter()
        .position(|(_, raw)| parse_envelope(raw).unwrap().push_kind == PushKind::CandidateBoard)
        .unwrap();
    let (port, sinks) = sink(accepted(at(23, 9, 22, 0)));
    let delivered = f
        .coordinator
        .dispatch_p05_unit_child_local(db, DATE, index, &sinks, append, Some(at(23, 9, 22, 0)))
        .unwrap();
    assert_eq!(delivered.sink_calls, 1);
    port
}

#[test]
fn outcome_tracker_shared_unit_board_only_counts_original_rows_without_unit_completion() {
    let f = Fixture::new("OUTCOME_UNIT_BOARD_ONLY");
    let (_dir, db) = operational();
    let draft = start(&f, &db, true);
    let intent = complete_intent(&f, &db, &draft);
    let append = Append::default();
    let port = deliver_board_only(&f, &db, &intent, &append);
    let target = outcome_target(&db, &draft);
    let original = f.coordinator.observe_p05_unit_receipts(DATE).unwrap();
    assert!(original.completion_identity().is_none());
    assert_eq!(
        original
            .children()
            .iter()
            .filter(|c| matches!(c, P05ChildReceiptObservation::PhysicallyAccepted { .. }))
            .count(),
        1
    );
    assert!(original
        .children()
        .iter()
        .any(|c| matches!(c, P05ChildReceiptObservation::NotPrepared { .. })));
    let before = capture_sql_binding(&Connection::open(&f.database_path).unwrap(), DATE).unwrap();
    let report = outcome_report(&db, &f, &target).unwrap();
    let linked = outcome_linked(&report);
    assert_eq!(
        (
            linked.physically_accepted_cards,
            linked.covered_samples,
            linked.pending_samples,
            linked.awaiting_drain_samples
        ),
        (1, 2, 2, 0)
    );
    assert_eq!(linked.rate, None);
    assert!(outcome_report(&db, &f, &target).unwrap() == report);
    assert!(
        capture_sql_binding(&Connection::open(&f.database_path).unwrap(), DATE).unwrap() == before
    );
    assert_eq!(port.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        f.query_i64("SELECT COUNT(*) FROM p05_s2_completion_receipts"),
        0
    );
}

#[test]
fn outcome_tracker_shared_unit_auction_acceptance_does_not_count_board_samples() {
    let f = Fixture::new("OUTCOME_UNIT_AUCTION");
    let (_dir, db) = operational();
    let draft = start(&f, &db, true);
    let intent = complete_intent(&f, &db, &draft);
    let index = intent
        .children
        .iter()
        .position(|(_, raw)| parse_envelope(raw).unwrap().push_kind == PushKind::AuctionRepush)
        .unwrap();
    let (port, sinks) = sink(accepted(at(23, 9, 22, 0)));
    f.coordinator
        .dispatch_p05_unit_child_local(
            &db,
            DATE,
            index,
            &sinks,
            &Append::default(),
            Some(at(23, 9, 22, 0)),
        )
        .unwrap();
    let report = outcome_report(&db, &f, &outcome_target(&db, &draft)).unwrap();
    assert_eq!(report.daily.observed.due_samples, 2);
    assert_eq!(
        (
            outcome_linked(&report).physically_accepted_cards,
            outcome_linked(&report).covered_samples
        ),
        (0, 0)
    );
    assert_eq!(port.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn outcome_tracker_shared_unit_completed_units_remain_in_report_and_score_drift_is_unavailable() {
    let f = Fixture::new("OUTCOME_UNIT_COMPLETED");
    let (dir, db) = operational();
    let draft = start(&f, &db, true);
    let intent = complete_intent(&f, &db, &draft);
    let append = Append::default();
    let port = deliver_all(&f, &db, &intent, 23, &append);
    assert!(finalize(&f, DATE).completion_identity().is_some());
    assert!(f
        .coordinator
        .inspect_p05_unfinished_units()
        .unwrap()
        .is_empty());
    let target = outcome_target(&db, &draft);
    assert_eq!(
        outcome_linked(&outcome_report(&db, &f, &target).unwrap()).covered_samples,
        2
    );
    let original = f.coordinator.observe_p05_unit_receipts(DATE).unwrap();
    let before = owned_revision(&f);
    corrupt_actual_score(&dir);
    let report = outcome_report(&db, &f, &target).unwrap();
    assert!(matches!(
        report.daily.physical_linked,
        crate::monitor::prediction::PhysicalLinkedOutcomeObservation::Unavailable {
            reason: "outcome_unit_context_unavailable"
        }
    ));
    assert_eq!(owned_revision(&f), before);
    assert_eq!(
        f.coordinator.observe_p05_unit_receipts(DATE).unwrap(),
        original
    );
    assert_eq!(port.calls.load(Ordering::SeqCst), 2);
    Connection::open(dir.path().join("TEST_CODE_P05_S2.db"))
        .unwrap()
        .execute(
            "UPDATE prediction_tracker SET pred_score=80 WHERE stock_code='TEST_CODE_600001'",
            [],
        )
        .unwrap();
    assert_eq!(
        outcome_linked(&outcome_report(&db, &f, &target).unwrap()).covered_samples,
        2
    );
    assert_eq!(port.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn outcome_tracker_shared_unit_late_pending_audit_retains_accepted_until_external_drain() {
    let f = Fixture::new("OUTCOME_UNIT_LATE");
    let (_dir, db) = operational();
    let draft = start(&f, &db, true);
    let intent = complete_intent(&f, &db, &draft);
    let append = Append::default();
    let port = deliver_all(&f, &db, &intent, 23, &append);
    assert!(finalize(&f, DATE).completion_identity().is_some());
    let target = outcome_target(&db, &draft);
    let (attempt,fence):(String,i64)=Connection::open(&f.database_path).unwrap()
        .query_row("SELECT a.attempt_identity,a.fence_token FROM delivery_attempts a JOIN delivery_decisions d ON d.decision_identity=a.decision_identity WHERE d.push_kind='CandidateBoard'",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    f.coordinator
        .record_sink_result(
            &attempt,
            fence,
            accepted(at(23, 9, 22, 2)),
            at(23, 9, 22, 2),
        )
        .unwrap();
    let receipt = f.coordinator.observe_p05_unit_receipts(DATE).unwrap();
    assert!(receipt.completion_identity().is_none());
    assert!(receipt
        .children()
        .iter()
        .all(|c| matches!(c, P05ChildReceiptObservation::PhysicallyAccepted { .. })));
    let before = capture_sql_binding(&Connection::open(&f.database_path).unwrap(), DATE).unwrap();
    let report = outcome_report(&db, &f, &target).unwrap();
    let linked = outcome_linked(&report);
    assert_eq!(
        (
            linked.physically_accepted_cards,
            linked.covered_samples,
            linked.awaiting_drain_samples
        ),
        (1, 2, 2)
    );
    assert_eq!(linked.rate, None);
    assert!(
        capture_sql_binding(&Connection::open(&f.database_path).unwrap(), DATE).unwrap() == before
    );
    assert_eq!(port.calls.load(Ordering::SeqCst), 2);
    f.coordinator
        .reconcile_all_pending(&append, at(23, 9, 22, 3))
        .unwrap();
    assert_eq!(
        outcome_linked(&outcome_report(&db, &f, &target).unwrap()).awaiting_drain_samples,
        0
    );
    assert_eq!(port.calls.load(Ordering::SeqCst), 2);
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM sink_results"), 3);
}

#[test]
fn outcome_tracker_shared_unit_unknown_started_does_not_run_preparation_or_fallback() {
    let f = Fixture::new("OUTCOME_UNIT_STARTED");
    let (_dir, db) = operational();
    let draft = start(&f, &db, true);
    drop(f.coordinator.claim_p05_prediction_prepare(&draft).unwrap());
    let before = capture_sql_binding(&Connection::open(&f.database_path).unwrap(), DATE).unwrap();
    let target = crate::calendar::verified_next_a_share_trading_day(
        chrono::NaiveDate::parse_from_str(DATE, "%Y-%m-%d").unwrap(),
    )
    .unwrap();
    let mut maturity = target;
    for _ in 1..5 {
        maturity = crate::calendar::verified_next_a_share_trading_day(maturity).unwrap();
    }
    let report = outcome_report(&db, &f, &maturity.to_string()).unwrap();
    assert!(matches!(
        report.daily.physical_linked,
        crate::monitor::prediction::PhysicalLinkedOutcomeObservation::Unavailable {
            reason: "outcome_unit_preparation_pending"
        }
    ));
    assert!(
        capture_sql_binding(&Connection::open(&f.database_path).unwrap(), DATE).unwrap() == before
    );
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM delivery_decisions"), 0);
    assert!(!db.p05_preparation_residue_for_date(DATE).unwrap());
}

#[test]
fn outcome_tracker_shared_unit_no_strong_never_adopts_later_actual_freeze() {
    let f = Fixture::new("OUTCOME_UNIT_NO_STRONG");
    let (_dir, db) = operational();
    let draft = start(&f, &db, false);
    let intent = complete_intent(&f, &db, &draft);
    let append = Append::default();
    let port = deliver_all(&f, &db, &intent, 23, &append);
    let request = crate::monitor::prediction::CandidateBoardPreparationRequest::new(
        DATE,
        "09:21",
        draft.board_rendered_bytes().to_vec(),
        vec![("TEST_CODE_600001".into(), 80.)],
    )
    .unwrap();
    let freeze = crate::monitor::prediction::prepare_candidate_board_on(&db, &request).unwrap();
    let crate::monitor::prediction::CandidateBoardPreparation::Frozen { record: freeze, .. } =
        freeze
    else {
        panic!("actual strong fixture must freeze");
    };
    let target = freeze.target_date().to_owned();
    let original = f.coordinator.observe_p05_unit_receipts(DATE).unwrap();
    let report = outcome_report(&db, &f, &target).unwrap();
    assert!(matches!(
        report.daily.physical_linked,
        crate::monitor::prediction::PhysicalLinkedOutcomeObservation::Unavailable {
            reason: "outcome_unit_context_unavailable"
        }
    ));
    assert_eq!(
        f.coordinator.observe_p05_unit_receipts(DATE).unwrap(),
        original
    );
    assert_eq!(port.calls.load(Ordering::SeqCst), 2);
}

fn install_outcome_last_hook(
    coordinator: Arc<DurableDeliveryCoordinator>,
    remaining: usize,
    hits: Arc<AtomicUsize>,
    action: Arc<Mutex<Option<Box<dyn FnOnce() -> Result<()> + Send>>>>,
) {
    let next = coordinator.clone();
    coordinator
        .install_database_operation_test_hook(
            DatabaseOperationTestPhase::AfterSqlBeforePostValidation,
            move || {
                hits.fetch_add(1, Ordering::SeqCst);
                if remaining == 1 {
                    action.lock().unwrap().take().unwrap()()?;
                } else {
                    install_outcome_last_hook(next, remaining - 1, hits, action);
                }
                Ok(())
            },
        )
        .unwrap();
}

#[test]
fn outcome_tracker_all_card_hooks_finish_before_actual_operational_tail_read() {
    let f = Fixture::new("OUTCOME_LAST_CARD_SCORE");
    let (dir, db) = operational();
    let draft = start(&f, &db, true);
    let intent = complete_intent(&f, &db, &draft);
    let append = Append::default();
    let port = deliver_board_only(&f, &db, &intent, &append);
    let target = outcome_target(&db, &draft);
    let original = f.coordinator.observe_p05_unit_receipts(DATE).unwrap();
    let path = dir.path().join("TEST_CODE_P05_S2.db");
    let hits = Arc::new(AtomicUsize::new(0));
    // Five dates each have one Unit core and one card core, followed by drain
    // and five final card cores. Mutate in the last actual card post-SQL hook.
    install_outcome_last_hook(
        fixture_coordinator_arc(&f),
        16,
        hits.clone(),
        Arc::new(Mutex::new(Some(Box::new(move || {
            Connection::open(path)?.execute(
                "UPDATE prediction_tracker SET pred_score=81 WHERE stock_code='TEST_CODE_600001'",
                [],
            )?;
            Ok(())
        })))),
    );
    assert_eq!(
        outcome_report(&db, &f, &target).unwrap_err(),
        "outcome_prediction_changed_at_tail"
    );
    assert_eq!(hits.load(Ordering::SeqCst), 16);
    assert_eq!(
        f.coordinator.observe_p05_unit_receipts(DATE).unwrap(),
        original
    );
    assert_eq!(port.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn outcome_tracker_all_card_hooks_bind_absent_unit_against_real_first_insert() {
    const CHILD_DATABASE: &str = "TEST_CODE_OUTCOME_LAST_CARD_INSERT_CHILD_DATABASE";
    const CASE: &str = "durable_delivery::coordinator::p05_unit::runtime::tests::outcome_tracker_all_card_hooks_bind_absent_unit_against_real_first_insert";
    if let Some(path) = std::env::var_os(CHILD_DATABASE) {
        let path = std::path::PathBuf::from(path);
        let test_code = path
            .parent()
            .and_then(std::path::Path::file_name)
            .and_then(|name| name.to_str())
            .unwrap();
        assert!(test_code.starts_with("TEST_CODE_BR192_OUTCOME_LAST_CARD_INSERT_"));
        let other =
            DurableDeliveryCoordinator::open(crate::durable_delivery::CoordinatorConfig::test(
                &path,
                test_code,
                "owner-TEST_CODE_OUTCOME_INSERT_CHILD-0123456789abcdef",
            ))
            .unwrap();
        other
            .store_p05_observed_draft_rendered(
                &input(23, &["TEST_CODE_600001", "TEST_CODE_000001"], true),
                Some(at(23, 9, 24, 0)),
                &mut renderer,
            )
            .unwrap();
        return;
    }
    let f = Fixture::new("OUTCOME_LAST_CARD_INSERT");
    let (_dir, db) = operational();
    f.coordinator
        .initialize_prospective_p05_family_at(&db, at(23, 9, 19, 0), f.database_path.parent())
        .unwrap();
    let mut target = chrono::NaiveDate::parse_from_str(DATE, "%Y-%m-%d").unwrap();
    for _ in 0..5 {
        target = crate::calendar::verified_next_a_share_trading_day(target).unwrap();
    }
    assert_eq!(
        outcome_linked(&outcome_report(&db, &f, &target.to_string()).unwrap()).covered_samples,
        0
    );
    let database = f.database_path.clone();
    let hits = Arc::new(AtomicUsize::new(0));
    install_outcome_last_hook(
        fixture_coordinator_arc(&f),
        16,
        hits.clone(),
        Arc::new(Mutex::new(Some(Box::new(move || {
            // The parent holds the process-wide attestation operation lease.
            // A real independent process can append through the original
            // coordinator protocol without reentering that local mutex.
            let output = std::process::Command::new(std::env::current_exe()?)
                .args(["--exact", CASE, "--nocapture", "--test-threads=1"])
                .env(CHILD_DATABASE, &database)
                .output()?;
            assert!(
                output.status.success(),
                "actual insertion child failed: {}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
            assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed; 0 failed"));
            Ok(())
        })))),
    );
    let report = outcome_report(&db, &f, &target.to_string()).unwrap();
    assert_eq!(hits.load(Ordering::SeqCst), 16);
    assert!(matches!(
        report.daily.physical_linked,
        crate::monitor::prediction::PhysicalLinkedOutcomeObservation::Unavailable {
            reason: "outcome_sql_binding_changed_at_tail"
        }
    ));
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM p05_unit_drafts"), 1);
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM delivery_decisions"), 0);
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM p05_s2_child_owners"), 0);
    assert!(!db.p05_preparation_residue_for_date(DATE).unwrap());
}
