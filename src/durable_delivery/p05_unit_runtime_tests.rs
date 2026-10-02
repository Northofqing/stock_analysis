use super::*;
use crate::durable_delivery::tests::Fixture;
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
    assert!(f
        .coordinator
        .store_p05_observed_draft_rendered(
            &input(25, &["TEST_CODE_600001"], false),
            Some(at(25, 9, 24, 0)),
            &mut renderer
        )
        .is_err());
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
    f.coordinator
        .resolve_uncertain(
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
            let hit = arm_extra_at_phase(&f.coordinator, second_phase);
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
                "begin" => f
                    .coordinator
                    .begin_attempt(decision.as_ref().unwrap(), 1, at(24, 9, 22, 1))
                    .map(|_| ()),
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
        let hit = arm_extra_at_phase(&f.coordinator, second_phase);
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
            "begin" => f
                .coordinator
                .begin_attempt(decision.as_ref().unwrap(), 1, at(24, 9, 22, 1))
                .map(|_| ()),
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
    let attempt = f
        .coordinator
        .begin_attempt(&original.1.decision_identity, 1, at(24, 9, 22, 1))
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
