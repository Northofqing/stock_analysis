//! Facade regressions reuse the real private protocol fixture. No successful
//! seal is constructed by these tests or decoded from caller-supplied bytes.
use super::physical_seal_behavior_tests::{attempt, deliver, envelopes, frozen, prepare, snapshot};
use super::*;
use crate::monitor::g5b_physical_v2::{self as physical, G5bPhysicalAttemptV2, G5bPhysicalSealV2};

fn completed(value: G5bPhysicalAttemptV2) -> G5bPhysicalSealV2 {
    match value {
        G5bPhysicalAttemptV2::Sealed(value) => value,
        G5bPhysicalAttemptV2::Incomplete => {
            panic!("actual accepted/drained original members must seal")
        }
    }
}

#[tokio::test]
async fn g5b_physical_v2_facade_actual_same_arc_refresh_foreign_arc_and_restart_read() {
    let fixture = Fixture::new("D5_ACTUAL_OWNER");
    let (_, calls) = frozen(&fixture, 2).await;
    prepare(&fixture, 2, 1);
    let append = MemoryAppendPort::default();
    let sink = deliver(
        &fixture,
        &append,
        AuthoritativeSinkResult::Accepted(receipt(clock("15:13:00"))),
    );
    let coordinator = owner(&fixture);
    let cap =
        completed(physical::try_seal_physical_day_v2(coordinator.clone(), date(), None).unwrap());
    assert_eq!(cap.selected_count(), 2);
    assert_eq!(cap.reason(), "AllSelectedPhysicalAccepted");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(sink.calls.load(Ordering::SeqCst), 2);
    let before = snapshot(&fixture);
    let same = physical::refresh_physical_day_v2(coordinator.clone(), &cap).unwrap();
    assert_eq!(
        (same.identity(), same.sha256(), same.revision()),
        (cap.identity(), cap.sha256(), cap.revision())
    );
    let foreign = fixture.second_coordinator("D5_FOREIGN_ARC");
    assert!(physical::refresh_physical_day_v2(foreign.clone(), &cap).is_err());
    assert!(physical::try_seal_physical_day_v2(foreign.clone(), date(), Some(&cap)).is_err());
    assert_eq!(snapshot(&fixture), before);
    // A real restart reader creates its own actual-owner-bound observation;
    // it does not transplant the older runtime's capability.
    let restarted = physical::read_physical_day_v2(foreign.clone(), date())
        .unwrap()
        .unwrap();
    assert_eq!(restarted.identity(), cap.identity());
    physical::refresh_physical_day_v2(foreign, &restarted).unwrap();
    assert_eq!(
        physical::list_physical_cohort_dates_v2(coordinator).unwrap(),
        vec![date()]
    );
    assert_eq!(envelopes(&fixture).len(), 2);
    assert_eq!(snapshot(&fixture), before);
}

#[tokio::test]
async fn g5b_physical_v2_facade_late_raw_invalidates_current_then_known_recloses() {
    let fixture = Fixture::new("D5_LATE_RECLOSE");
    frozen(&fixture, 1).await;
    prepare(&fixture, 1, 1);
    let append = MemoryAppendPort::default();
    let sink = deliver(
        &fixture,
        &append,
        AuthoritativeSinkResult::Accepted(receipt(clock("15:13:00"))),
    );
    let coordinator = owner(&fixture);
    let cap =
        completed(physical::try_seal_physical_day_v2(coordinator.clone(), date(), None).unwrap());
    let original_raw = fixture
        .query_blob("SELECT result_canonical FROM sink_results WHERE authoritative_for_state=1");
    let original_seal =
        fixture.query_blob("SELECT seal_canonical FROM g5b_day_seals ORDER BY revision LIMIT 1");
    let (attempt, fence) = attempt(&fixture);
    let mut late = receipt(clock("15:15:00"));
    late.message_id = "TEST_CODE_D5_DISTINCT_LATE".into();
    coordinator
        .record_sink_result(
            &attempt,
            fence,
            AuthoritativeSinkResult::Accepted(late),
            clock("15:15:00"),
        )
        .unwrap();
    assert!(physical::read_physical_day_v2(coordinator.clone(), date())
        .unwrap()
        .is_none());
    assert!(physical::refresh_physical_day_v2(coordinator.clone(), &cap).is_err());
    let before = snapshot(&fixture);
    assert!(matches!(
        physical::try_seal_physical_day_v2(coordinator.clone(), date(), Some(&cap)).unwrap(),
        G5bPhysicalAttemptV2::Incomplete
    ));
    assert_eq!(snapshot(&fixture), before);
    coordinator
        .reconcile_all_pending(&append, clock("15:15:01"))
        .unwrap();
    let current = completed(
        physical::try_seal_physical_day_v2(coordinator.clone(), date(), Some(&cap)).unwrap(),
    );
    assert_eq!(current.cohort_identity(), cap.cohort_identity());
    assert_ne!(current.identity(), cap.identity());
    assert!(current.revision() > cap.revision());
    assert_eq!(
        current.revision(),
        fixture.query_i64("SELECT revision FROM g5b_day_heads")
    );
    assert_eq!(sink.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture.query_blob(
            "SELECT result_canonical FROM sink_results WHERE authoritative_for_state=1"
        ),
        original_raw
    );
    assert_eq!(
        fixture.query_blob("SELECT seal_canonical FROM g5b_day_seals ORDER BY revision LIMIT 1"),
        original_seal
    );
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM sink_results WHERE late_after_fence=1 AND authoritative_for_state=0"),1);
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM g5b_day_seals"), 2);
    assert_eq!(
        completed(physical::try_seal_physical_day_v2(coordinator, date(), Some(&cap)).unwrap())
            .identity(),
        current.identity()
    );
}

#[tokio::test]
async fn g5b_physical_v2_facade_observed_suffix_rollback_cannot_reset_known_prefix() {
    let fixture = Fixture::new("D5_KNOWN_SUFFIX");
    let (log, _) = frozen(&fixture, 1).await;
    prepare(&fixture, 1, 1);
    let append = MemoryAppendPort::default();
    deliver(
        &fixture,
        &append,
        AuthoritativeSinkResult::Accepted(receipt(clock("15:13:00"))),
    );
    let coordinator = owner(&fixture);
    let cap =
        completed(physical::try_seal_physical_day_v2(coordinator.clone(), date(), None).unwrap());
    let source = namespace(&fixture).join("20260928.jsonl");
    let head = namespace(&fixture).join("20260928.input-head.v1.json");
    let original_source = std::fs::read(&source).unwrap();
    let original_head = std::fs::read(&head).unwrap();
    log.append_test_date_raw_production_fixture(date(), &raw("600002"))
        .unwrap();
    fixture.cleanup.record(&head, OwnedPathKind::FileOrSymlink);
    let observed = physical::refresh_physical_day_v2(coordinator.clone(), &cap).unwrap();
    assert_eq!(
        observed.identity(),
        cap.identity(),
        "suffix is not a new selected cohort"
    );
    let before = snapshot(&fixture);
    // Replay genuinely observed older bytes into the same owned source inode;
    // no fabricated head/hash serves as a successful capability fixture.
    std::fs::write(&source, &original_source).unwrap();
    std::fs::write(&head, &original_head).unwrap();
    assert!(
        physical::try_seal_physical_day_v2(coordinator.clone(), date(), Some(&observed)).is_err()
    );
    assert!(physical::refresh_physical_day_v2(coordinator, &observed).is_err());
    assert_eq!(snapshot(&fixture), before);
    assert_eq!(std::fs::read(&source).unwrap(), original_source);
    assert_eq!(std::fs::read(&head).unwrap(), original_head);
}

#[tokio::test]
async fn g5b_physical_v2_facade_uncertain_is_incomplete_not_an_empty_or_success_seal() {
    let fixture = Fixture::new("D5_UNCERTAIN");
    frozen(&fixture, 1).await;
    prepare(&fixture, 1, 1);
    let append = MemoryAppendPort::default();
    let sink = deliver(
        &fixture,
        &append,
        AuthoritativeSinkResult::Uncertain(uncertainty(clock("15:13:00"))),
    );
    let before = snapshot(&fixture);
    let coordinator = owner(&fixture);
    assert!(physical::read_physical_day_v2(coordinator.clone(), date())
        .unwrap()
        .is_none());
    assert!(matches!(
        physical::try_seal_physical_day_v2(coordinator.clone(), date(), None).unwrap(),
        G5bPhysicalAttemptV2::Incomplete
    ));
    assert_eq!(
        physical::list_physical_cohort_dates_v2(coordinator).unwrap(),
        vec![date()]
    );
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM g5b_day_seals"), 0);
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM sink_results WHERE result_kind='Uncertain'"),
        1
    );
    assert_eq!(sink.calls.load(Ordering::SeqCst), 1);
    assert_eq!(snapshot(&fixture), before);
}
