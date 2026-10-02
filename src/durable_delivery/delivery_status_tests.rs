use super::*;

fn database_rows(fixture: &Fixture) -> BTreeMap<String, Vec<Vec<String>>> {
    let connection = Connection::open(&fixture.database_path).unwrap();
    let tables = connection
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    tables
        .into_iter()
        .map(|table| {
            let rows = authority_table_rows(&connection, &table);
            (table, rows)
        })
        .collect()
}

fn state_count(status: &DeliveryStatusSnapshot, state: DecisionState) -> u64 {
    status
        .state_counts
        .iter()
        .find(|entry| entry.state == state)
        .unwrap()
        .count
}

#[test]
fn delivery_status_actual_empty_and_reserved_are_observed_without_mutation() {
    let fixture = Fixture::new("M3_STATUS_EMPTY_RESERVED");
    let empty_before = database_rows(&fixture);
    let before = Utc::now();
    let empty = fixture.coordinator.read_delivery_status().unwrap();
    assert!(empty.observed_at >= before && empty.observed_at <= Utc::now());
    assert_eq!(empty.total_decisions, 0);
    assert!(empty.state_counts.iter().all(|entry| entry.count == 0));
    assert_eq!(database_rows(&fixture), empty_before);
    let append = MemoryAppendPort::default();
    let candidate = envelope(
        "M3_RESERVED",
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        "2026-07-30",
        false,
    );
    prepare_reserved(&fixture, &candidate, &append);
    let before = database_rows(&fixture);
    let records_before = append.records.lock().unwrap().clone();
    let status = fixture.coordinator.read_delivery_status().unwrap();
    assert_eq!(status.total_decisions, 1);
    assert_eq!(state_count(&status, DecisionState::Reserved), 1);
    assert_eq!(status.deliverable_decisions, 1);
    assert_eq!(status.locally_pending_decisions, 0);
    assert_eq!(database_rows(&fixture), before);
    assert_eq!(*append.records.lock().unwrap(), records_before);
    assert!(!serde_json::to_string(&status)
        .unwrap()
        .contains("TEST_CODE"));
}

#[test]
fn delivery_status_actual_live_foreign_local_and_expired_leases_use_original_rules() {
    let fixture = Fixture::new("M3_STATUS_LEASES");
    let append = MemoryAppendPort::default();
    let candidates = ["LOCAL", "FOREIGN", "EXPIRED"].map(|label| {
        envelope(
            label,
            PushKind::HoldingEvent,
            DeliverySubKind::None,
            "2026-07-30",
            false,
        )
    });
    for candidate in &candidates {
        prepare_reserved(&fixture, candidate, &append)
    }
    let foreign = fixture.second_coordinator("M3_STATUS_FOREIGN");
    let actual = Utc::now();
    fixture
        .coordinator
        .begin_attempt(&candidates[0].decision_identity, 1, actual)
        .unwrap()
        .unwrap();
    foreign
        .begin_attempt(&candidates[1].decision_identity, 1, actual)
        .unwrap()
        .unwrap();
    foreign
        .begin_attempt(&candidates[2].decision_identity, 1, now())
        .unwrap()
        .unwrap();
    let before = database_rows(&fixture);
    let status = fixture.coordinator.read_delivery_status().unwrap();
    assert_eq!(state_count(&status, DecisionState::AttemptInFlight), 3);
    assert_eq!(status.non_progressable_foreign_attempts, 1);
    assert_eq!(status.locally_pending_decisions, 2);
    assert_eq!(status.deliverable_decisions, 0);
    assert_eq!(status.non_progressable_manual_reviews, 0);
    assert_eq!(database_rows(&fixture), before);
}

#[test]
fn delivery_status_actual_uncertain_and_delivered_remain_distinct_without_ports() {
    let fixture = Fixture::new("M3_STATUS_TERMINALS");
    let append = MemoryAppendPort::default();
    let candidates = ["UNCERTAIN", "DELIVERED"].map(|label| {
        envelope(
            label,
            PushKind::HoldingEvent,
            DeliverySubKind::None,
            "2026-07-30",
            false,
        )
    });
    for candidate in &candidates {
        prepare_reserved(&fixture, candidate, &append)
    }
    let uncertain = StaticSink::new(AuthoritativeSinkResult::Uncertain(uncertainty(now())));
    let accepted = StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(now())));
    let uncertain_sinks: Vec<AuthoritativeSink> = vec![uncertain.clone()];
    let accepted_sinks: Vec<AuthoritativeSink> = vec![accepted.clone()];
    fixture
        .coordinator
        .resume_deliverable(&candidates[0].decision_identity, &uncertain_sinks, now())
        .unwrap();
    fixture
        .coordinator
        .resume_deliverable(&candidates[1].decision_identity, &accepted_sinks, now())
        .unwrap();
    fixture
        .coordinator
        .reconcile_all_pending(&append, now())
        .unwrap();
    let before = database_rows(&fixture);
    let uncertain_calls = uncertain.calls.load(Ordering::SeqCst);
    let accepted_calls = accepted.calls.load(Ordering::SeqCst);
    let status = fixture.coordinator.read_delivery_status().unwrap();
    assert_eq!(state_count(&status, DecisionState::Delivered), 1);
    assert_eq!(
        state_count(&status, DecisionState::UncertainManualReview),
        1
    );
    assert_eq!(status.non_progressable_manual_reviews, 1);
    assert_eq!(status.locally_pending_decisions, 0);
    assert_eq!(status.deliverable_decisions, 0);
    assert_eq!(database_rows(&fixture), before);
    assert_eq!(uncertain.calls.load(Ordering::SeqCst), uncertain_calls);
    assert_eq!(accepted.calls.load(Ordering::SeqCst), accepted_calls);
}

#[test]
fn delivery_status_actual_rejected_retry_authorization_is_observed_without_retrying() {
    let fixture = Fixture::new("M3_STATUS_RETRY");
    let append = MemoryAppendPort::default();
    let candidates = ["RETRY", "NO_RETRY"].map(|label| {
        envelope(
            label,
            PushKind::HoldingEvent,
            DeliverySubKind::None,
            "2026-07-30",
            false,
        )
    });
    for candidate in &candidates {
        prepare_reserved(&fixture, candidate, &append)
    }
    let mut sink_calls = Vec::new();
    for (candidate, retry) in candidates.iter().zip([true, false]) {
        let sink = StaticSink::new(AuthoritativeSinkResult::Rejected(rejection(now(), retry)));
        let sinks: Vec<AuthoritativeSink> = vec![sink.clone()];
        fixture
            .coordinator
            .resume_deliverable(&candidate.decision_identity, &sinks, now())
            .unwrap();
        sink_calls.push(sink);
    }
    fixture
        .coordinator
        .reconcile_all_pending(&append, now())
        .unwrap();
    let before = database_rows(&fixture);
    let status = fixture.coordinator.read_delivery_status().unwrap();
    assert_eq!(state_count(&status, DecisionState::RejectedDurable), 2);
    assert_eq!(status.deliverable_decisions, 1);
    assert_eq!(status.locally_pending_decisions, 0);
    assert_eq!(database_rows(&fixture), before);
    assert!(sink_calls
        .iter()
        .all(|sink| sink.calls.load(Ordering::SeqCst) == 1));
}
