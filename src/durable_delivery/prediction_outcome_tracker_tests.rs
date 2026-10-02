//! Actual isolated stores and original delivery/verifier APIs. No successful
//! report is assembled from a fabricated receipt, completion or row DTO.
use super::*;
use crate::database::DatabaseManager;
use crate::monitor::prediction::{OutcomeTracker, PhysicalLinkedOutcomeObservation};
use chrono::{FixedOffset, NaiveDate};
use diesel::RunQueryDsl;

fn report_at(target: &str) -> DateTime<FixedOffset> {
    format!("{target}T15:01:00+08:00").parse().unwrap()
}
fn original_source() -> (
    tempfile::TempDir,
    DatabaseManager,
    crate::database::p05_prediction_freeze::FrozenCandidateBoardV2,
) {
    let (dir, frozen) = p05_v2_frozen_source();
    let db =
        DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_p05_counted_source.db"))
            .unwrap();
    (dir, db, frozen)
}
fn qualified_close(db: &DatabaseManager, code: &str, date: &str, close: f64) {
    let mut c = db.get_conn().unwrap();
    diesel::sql_query("INSERT INTO stock_daily(code,date,close) VALUES(?1,?2,?3)")
        .bind::<diesel::sql_types::Text, _>(code)
        .bind::<diesel::sql_types::Text, _>(date)
        .bind::<diesel::sql_types::Double, _>(close)
        .execute(&mut c)
        .unwrap();
    diesel::sql_query("INSERT INTO qualified_daily_trading_status(code,date,status,contract_version,source,source_at,observed_at,batch_id) VALUES(?1,?2,'trading','TEST_CODE_AUTHORITY_V1','TEST_CODE_AUTHORITY','2026-10-08T07:00:00Z','2026-10-08T07:00:01Z','TEST_CODE_BATCH')")
        .bind::<diesel::sql_types::Text,_>(code)
        .bind::<diesel::sql_types::Text,_>(date).execute(&mut c).unwrap();
}
fn observed_counts(
    report: &crate::monitor::prediction::OutcomePeriodObservation,
) -> &crate::monitor::prediction::PhysicalLinkedOutcomeCounts {
    match &report.physical_linked {
        PhysicalLinkedOutcomeObservation::Observed(c) => c,
        other => panic!("actual link unavailable: {other:?}"),
    }
}

#[test]
fn outcome_tracker_original_v2_one_card_two_rows_and_unsent_signal_have_separate_denominators() {
    let (_dir, db, frozen) = original_source();
    let f = Fixture::new("OUTCOME_ORIGINAL_V2");
    let append = MemoryAppendPort::default();
    let envelope = p05_v2_envelope(&frozen, frozen.source_canonical().to_vec());
    prepare_reserved(&f, &envelope, &append);
    let sink = StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(now())));
    let sinks: Vec<AuthoritativeSink> = vec![sink.clone()];
    f.coordinator
        .resume_deliverable(&envelope.decision_identity, &sinks, now())
        .unwrap();
    reconcile_terminal(
        &f,
        &append,
        DecisionState::Delivered,
        &envelope.decision_identity,
    );
    for (index, row) in frozen.ordered_rows().iter().enumerate() {
        qualified_close(&db, row.code(), frozen.business_date(), 10.);
        qualified_close(
            &db,
            row.code(),
            frozen.target_date(),
            if index == 0 { 11. } else { 9. },
        );
    }
    db.save_prediction_legacy(
        frozen.business_date(),
        frozen.target_date(),
        None,
        Some("TEST_CODE_UNSENT"),
        "down",
        50.,
        None,
    )
    .unwrap();
    qualified_close(&db, "TEST_CODE_UNSENT", frozen.business_date(), 10.);
    qualified_close(&db, "TEST_CODE_UNSENT", frozen.target_date(), 9.);
    let verification = crate::monitor::prediction::verify_due_predictions(
        &db,
        NaiveDate::parse_from_str(frozen.target_date(), "%Y-%m-%d").unwrap(),
    )
    .unwrap();
    assert_eq!((verification.verified, verification.hits), (3, 2));
    let durable_before =
        audit_v4_upgrade_database_snapshot(&Connection::open(&f.database_path).unwrap());
    let snapshot = db
        .read_outcome_prediction_window(&[frozen.target_date().into()])
        .unwrap();
    let tracker = OutcomeTracker::new(&db, Some(&f.coordinator));
    let report = tracker
        .read_at_for_test(report_at(frozen.target_date()))
        .unwrap();
    assert_eq!(
        (
            report.daily.observed.due_samples,
            report.daily.observed.recorded_samples,
            report.daily.observed.hits
        ),
        (3, 3, 2)
    );
    let linked = observed_counts(&report.daily);
    assert_eq!(
        (
            linked.physically_accepted_cards,
            linked.covered_samples,
            linked.recorded_samples,
            linked.hits
        ),
        (1, 2, 2, 1)
    );
    assert_eq!(linked.rate, Some(0.5));
    assert_eq!(
        tracker
            .read_at_for_test(report_at(frozen.target_date()))
            .unwrap(),
        report
    );
    assert_eq!(
        db.read_outcome_prediction_window(&[frozen.target_date().into()])
            .unwrap(),
        snapshot
    );
    assert_eq!(
        audit_v4_upgrade_database_snapshot(&Connection::open(&f.database_path).unwrap()),
        durable_before
    );
    assert_eq!(sink.calls.load(Ordering::SeqCst), 1);
    assert!(report.render().contains("非执行结算/ReviewTask完成"));
}

#[test]
fn outcome_tracker_actual_accepted_without_qualified_history_remains_pending_not_zero_rate() {
    let (_dir, db, frozen) = original_source();
    let f = Fixture::new("OUTCOME_NO_HISTORY");
    let append = MemoryAppendPort::default();
    let e = p05_v2_envelope(&frozen, frozen.source_canonical().to_vec());
    prepare_reserved(&f, &e, &append);
    let port = StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(now())));
    let sinks: Vec<AuthoritativeSink> = vec![port.clone()];
    f.coordinator
        .resume_deliverable(&e.decision_identity, &sinks, now())
        .unwrap();
    reconcile_terminal(&f, &append, DecisionState::Delivered, &e.decision_identity);
    let report = OutcomeTracker::new(&db, Some(&f.coordinator))
        .read_at_for_test(report_at(frozen.target_date()))
        .unwrap();
    assert_eq!(
        (
            report.daily.observed.recorded_samples,
            report.daily.observed.pending_samples
        ),
        (0, 2)
    );
    let linked = observed_counts(&report.daily);
    assert_eq!(
        (
            linked.physically_accepted_cards,
            linked.covered_samples,
            linked.pending_samples
        ),
        (1, 2, 2)
    );
    assert_eq!(linked.rate, None);
    assert_eq!(port.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn outcome_tracker_frozen_pending_rejected_uncertain_and_manual_are_not_original_physical_acceptance(
) {
    for stage in [
        "FrozenOnly",
        "Pending",
        "Rejected",
        "Uncertain",
        "ManualAccepted",
    ] {
        let (_dir, db, frozen) = original_source();
        let f = Fixture::new("OUTCOME_TERMINAL");
        let append = MemoryAppendPort::default();
        let e = p05_v2_envelope(&frozen, frozen.source_canonical().to_vec());
        if stage != "FrozenOnly" {
            prepare_reserved(&f, &e, &append);
        }
        if matches!(stage, "Rejected" | "Uncertain" | "ManualAccepted") {
            let result = if stage == "Rejected" {
                AuthoritativeSinkResult::Rejected(rejection(now(), false))
            } else {
                AuthoritativeSinkResult::Uncertain(uncertainty(now()))
            };
            let port = StaticSink::new(result);
            let sinks: Vec<AuthoritativeSink> = vec![port.clone()];
            f.coordinator
                .resume_deliverable(&e.decision_identity, &sinks, now())
                .unwrap();
            reconcile_terminal(
                &f,
                &append,
                if stage == "Rejected" {
                    DecisionState::RejectedDurable
                } else {
                    DecisionState::UncertainManualReview
                },
                &e.decision_identity,
            );
            if stage == "ManualAccepted" {
                f.coordinator
                    .resolve_uncertain(
                        &ManualResolutionCommand {
                            decision_identity: e.decision_identity.clone(),
                            disposition: ManualDisposition::Accepted {
                                receipt: Some(receipt(now())),
                            },
                            operator_identity: "TEST_CODE_OUTCOME_OPERATOR_0123456789".into(),
                            reason: "TEST_CODE original manual evidence".into(),
                            external_evidence: b"TEST_CODE independent external evidence".to_vec(),
                            resolved_at: now(),
                        },
                        &append,
                    )
                    .unwrap();
                reconcile_terminal(&f, &append, DecisionState::Delivered, &e.decision_identity);
            }
            assert_eq!(port.calls.load(Ordering::SeqCst), 1);
        }
        let report = OutcomeTracker::new(&db, Some(&f.coordinator))
            .read_at_for_test(report_at(frozen.target_date()))
            .unwrap();
        assert_eq!(report.daily.observed.due_samples, 2, "{stage}");
        let linked = observed_counts(&report.daily);
        assert_eq!(
            (
                linked.physically_accepted_cards,
                linked.covered_samples,
                linked.rate
            ),
            (0, 0, None),
            "{stage}"
        );
    }
}

#[test]
fn outcome_tracker_legacy_v1_never_adopts_later_frozen_row_membership() {
    let (_dir, db, frozen) = original_source();
    let f = Fixture::new("OUTCOME_LEGACY_V1");
    let append = MemoryAppendPort::default();
    let e = p05_v1_envelope_at(
        frozen.business_date(),
        frozen.occurrence_identity(),
        "OUTCOME_V1",
        false,
    );
    prepare_reserved(&f, &e, &append);
    let port = StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(now())));
    let sinks: Vec<AuthoritativeSink> = vec![port.clone()];
    f.coordinator
        .resume_deliverable(&e.decision_identity, &sinks, now())
        .unwrap();
    reconcile_terminal(&f, &append, DecisionState::Delivered, &e.decision_identity);
    let report = OutcomeTracker::new(&db, Some(&f.coordinator))
        .read_at_for_test(report_at(frozen.target_date()))
        .unwrap();
    assert_eq!(observed_counts(&report.daily).covered_samples, 0);
    assert_eq!(report.daily.observed.due_samples, 2);
    assert_eq!(port.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn outcome_tracker_target_sessions_not_signal_dates_and_cache_absence_is_explicit() {
    let dir = tempfile::tempdir().unwrap();
    let db = DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_OUTCOME_DATES.db"))
        .unwrap();
    for (code, pred, target, direction, change, hit) in [
        (
            "TEST_CODE_OLD_T5",
            "2026-09-23",
            "2026-10-08",
            "up",
            2.,
            true,
        ),
        (
            "TEST_CODE_DOWN",
            "2026-09-30",
            "2026-10-08",
            "short",
            -2.,
            true,
        ),
        (
            "TEST_CODE_FLAT",
            "2026-10-08",
            "2026-10-09",
            "neutral",
            0.5,
            true,
        ),
        (
            "TEST_CODE_FUTURE",
            "2026-10-09",
            "2026-10-12",
            "up",
            2.,
            true,
        ),
    ] {
        db.save_prediction_legacy(pred, target, None, Some(code), direction, 80., None)
            .unwrap();
        let id = db.get_prediction_by_code_date(code, pred).unwrap().id;
        db.update_prediction_result_by_id(id, change, hit).unwrap();
    }
    let tracker = OutcomeTracker::new(&db, None);
    let report = tracker
        .read_at_for_test("2026-10-09T15:01:00+08:00".parse().unwrap())
        .unwrap();
    assert_eq!(
        (
            report.daily.observed.due_samples,
            report.weekly.observed.due_samples
        ),
        (1, 3)
    );
    assert!(
        report.weekly.window_start < NaiveDate::parse_from_str("2026-10-08", "%Y-%m-%d").unwrap()
    );
    assert!(matches!(
        report.daily.physical_linked,
        PhysicalLinkedOutcomeObservation::Unavailable {
            reason: "outcome_counted_cache_absent"
        }
    ));
    let before_close = tracker
        .read_at_for_test("2026-10-09T14:59:59+08:00".parse().unwrap())
        .unwrap();
    assert_eq!(before_close.daily.as_of.to_string(), "2026-10-08");
    assert_eq!(before_close.daily.observed.due_samples, 2);
    assert!(tracker
        .read_at_for_test("2027-01-04T15:01:00+08:00".parse().unwrap())
        .is_err());
    assert_eq!(db.count_predictions().unwrap(), 4);
}

#[test]
fn outcome_tracker_invalid_recorded_hit_and_preallocation_budget_are_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("TEST_CODE_OUTCOME_BUDGET.db");
    let db = DatabaseManager::open_isolated_for_test(path.clone()).unwrap();
    db.save_prediction_legacy(
        "2026-10-08",
        "2026-10-09",
        None,
        Some("TEST_CODE_BAD_HIT"),
        "down",
        80.,
        None,
    )
    .unwrap();
    let id = db
        .get_prediction_by_code_date("TEST_CODE_BAD_HIT", "2026-10-08")
        .unwrap()
        .id;
    db.update_prediction_result_by_id(id, 2., true).unwrap();
    let tracker = OutcomeTracker::new(&db, None);
    assert_eq!(
        tracker
            .read_at_for_test(report_at("2026-10-09"))
            .unwrap_err(),
        "outcome_recorded_hit_mismatch"
    );
    let c = Connection::open(&path).unwrap();
    c.execute("UPDATE prediction_tracker SET actual_change=NULL,hit=NULL,stock_code=CAST(zeroblob(?1) AS TEXT) WHERE id=?2",params![crate::database::p05_prediction_freeze::OUTCOME_REPORT_MAX_BYTES+1,id]).unwrap();
    assert_eq!(
        tracker
            .read_at_for_test(report_at("2026-10-09"))
            .unwrap_err(),
        "outcome_prediction_snapshot_unavailable"
    );
    c.execute(
        "UPDATE prediction_tracker SET stock_code='TEST_CODE_BAD_HIT' WHERE id=?1",
        [id],
    )
    .unwrap();
    c.execute("WITH RECURSIVE seq(i) AS(VALUES(1) UNION ALL SELECT i+1 FROM seq WHERE i<4096) INSERT INTO prediction_tracker(pred_date,target_date,pred_direction,pred_score) SELECT '2026-10-08','2026-10-09','up',80 FROM seq",[]).unwrap();
    assert_eq!(
        tracker
            .read_at_for_test(report_at("2026-10-09"))
            .unwrap_err(),
        "outcome_prediction_snapshot_unavailable"
    );
    assert_eq!(db.count_predictions().unwrap(), 4097);
}

fn require_preflight_budget(db: &DatabaseManager, target: &str) {
    let error = db
        .read_outcome_prediction_window(&[target.into()])
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("outcome operational snapshot exceeds budget"),
        "must fail the scalar preflight, not a later decoder: {error}"
    );
}

#[test]
fn outcome_tracker_member_count_and_member_code_budget_precedes_frozen_decoder() {
    for count_fault in [true, false] {
        let (dir, db, frozen) = original_source();
        let c = Connection::open(dir.path().join("TEST_CODE_p05_counted_source.db")).unwrap();
        c.pragma_update(None, "foreign_keys", "ON").unwrap();
        if count_fault {
            let outside = crate::calendar::verified_next_a_share_trading_day(
                NaiveDate::parse_from_str(frozen.target_date(), "%Y-%m-%d").unwrap(),
            )
            .unwrap()
            .to_string();
            // All extra member references have actual FK-backed prediction IDs.
            // Their target is outside the report's raw-row selector, so that
            // selector alone cannot detect this physical member collection.
            c.execute("WITH RECURSIVE seq(i) AS(VALUES(1) UNION ALL SELECT i+1 FROM seq WHERE i<4095) INSERT INTO prediction_tracker(pred_date,target_date,stock_code,pred_direction,pred_detail) SELECT ?1,?2,'TEST_CODE_BUDGET_MEMBER_'||i,'up','candidate-strong' FROM seq",params![frozen.business_date(),outside]).unwrap();
            c.execute("INSERT INTO candidate_board_prediction_member_v2(prediction_row_id,occurrence_identity,ordinal,code) SELECT id,?1,1+ROW_NUMBER() OVER(ORDER BY id),stock_code FROM prediction_tracker WHERE stock_code LIKE 'TEST_CODE_BUDGET_MEMBER_%'",[frozen.occurrence_identity()]).unwrap();
            let members: i64 = c
                .query_row(
                    "SELECT COUNT(*) FROM candidate_board_prediction_member_v2",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(members, 4097);
        } else {
            c.execute(
                "DROP TRIGGER trg_candidate_board_prediction_member_v2_no_update",
                [],
            )
            .unwrap();
            c.execute("UPDATE candidate_board_prediction_member_v2 SET code=CAST(zeroblob(?1) AS TEXT) WHERE prediction_row_id=?2",params![crate::database::p05_prediction_freeze::OUTCOME_REPORT_MAX_BYTES+1,frozen.ordered_rows()[0].prediction_row_id()]).unwrap();
        }
        require_preflight_budget(&db, frozen.target_date());
    }
}

#[test]
fn outcome_tracker_associated_saved_detail_outside_target_range_is_budgeted_before_read() {
    let (dir, db, frozen) = original_source();
    let c = Connection::open(dir.path().join("TEST_CODE_p05_counted_source.db")).unwrap();
    let outside = crate::calendar::verified_next_a_share_trading_day(
        NaiveDate::parse_from_str(frozen.target_date(), "%Y-%m-%d").unwrap(),
    )
    .unwrap()
    .to_string();
    c.execute("UPDATE prediction_tracker SET target_date=?1,pred_detail=CAST(zeroblob(?2) AS TEXT) WHERE id=?3",params![outside,crate::database::p05_prediction_freeze::OUTCOME_REPORT_MAX_BYTES+1,frozen.ordered_rows()[0].prediction_row_id()]).unwrap();
    require_preflight_budget(&db, frozen.target_date());
    assert_eq!(db.count_predictions().unwrap(), 2);
}

#[test]
fn outcome_tracker_unbounded_freeze_header_is_budgeted_before_read() {
    let (dir, db, frozen) = original_source();
    let c = Connection::open(dir.path().join("TEST_CODE_p05_counted_source.db")).unwrap();
    let original_hashes: (String, String, String) = c
        .query_row(
            "SELECT calendar_authority_hash,rendered_sha256,source_sha256 FROM candidate_board_prediction_freeze_v2 WHERE occurrence_identity=?1",
            [frozen.occurrence_identity()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    for hash in [&original_hashes.0, &original_hashes.1, &original_hashes.2] {
        assert_eq!(hash.len(), 64);
        assert!(hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
    }
    c.execute(
        "DROP TRIGGER trg_candidate_board_prediction_freeze_v2_no_update",
        [],
    )
    .unwrap();
    // The table permits this oversized, corrupted business date. Preserve the
    // original hashes, target, occurrence and source/rendered fields: the scalar
    // byte budget must reject the header before the semantic date decoder.
    let oversized_bytes = crate::database::p05_prediction_freeze::OUTCOME_REPORT_MAX_BYTES + 1;
    assert_eq!(
        c.execute(
            "UPDATE candidate_board_prediction_freeze_v2 SET business_date=CAST(zeroblob(?1) AS TEXT) WHERE occurrence_identity=?2",
            params![oversized_bytes, frozen.occurrence_identity()],
        )
        .unwrap(),
        1,
    );
    let (stored_bytes, original_hashes_and_target): (i64, i64) = c
        .query_row(
            "SELECT length(CAST(business_date AS BLOB)),calendar_authority_hash=?2 AND rendered_sha256=?3 AND source_sha256=?4 AND target_date=?5 FROM candidate_board_prediction_freeze_v2 WHERE occurrence_identity=?1",
            params![
                frozen.occurrence_identity(),
                &original_hashes.0,
                &original_hashes.1,
                &original_hashes.2,
                frozen.target_date(),
            ],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(stored_bytes, oversized_bytes);
    assert_eq!(original_hashes_and_target, 1);
    require_preflight_budget(&db, frozen.target_date());
}
