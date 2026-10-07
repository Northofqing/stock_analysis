use super::*;
use diesel::RunQueryDsl;

fn private_db() -> (tempfile::TempDir, DatabaseManager) {
    let dir = tempfile::tempdir().unwrap();
    let db = DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_prediction.db"))
        .unwrap();
    (dir, db)
}

#[test]
fn legacy_t1_requires_trading_start_and_available_verified_target_before_write() {
    let (_dir, db) = private_db();
    for (today, target, code) in [
        ("2026-08-28", "2026-08-31", "TEST_CODE_t1_weekend"),
        ("2026-09-30", "2026-10-08", "TEST_CODE_t1_holiday"),
    ] {
        let today = NaiveDate::parse_from_str(today, "%Y-%m-%d").unwrap();
        let projected = save_prediction_on(&db, today, None, Some(code), "up", 75., None)
            .expect("checked-in T+1 must be available");
        assert_eq!(projected.to_string(), target);
        let row = db
            .get_prediction_by_code_date(code, &today.to_string())
            .unwrap();
        assert_eq!(row.target_date, target);
    }
    let before = db.count_predictions().unwrap();
    for (closed, code) in [
        ("2026-10-04", "TEST_CODE_t1_weekend_start"),
        ("2026-10-07", "TEST_CODE_t1_holiday_start"),
    ] {
        let closed = NaiveDate::parse_from_str(closed, "%Y-%m-%d").unwrap();
        assert!(
            save_prediction_on(&db, closed, None, Some(code), "up", 75., None)
                .unwrap_err()
                .contains("不是已核验 A 股交易日")
        );
        assert_eq!(db.count_predictions().unwrap(), before);
    }
    assert!(save_prediction_on(
        &db,
        NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
        None,
        Some("TEST_CODE_t1_missing_next_year"),
        "up",
        75.,
        None,
    )
    .unwrap_err()
    .contains("coverage unavailable"));
    assert_eq!(db.count_predictions().unwrap(), before);
}

#[test]
fn completed_shanghai_session_uses_close_boundary_and_verified_calendar() {
    let at = |text: &str| text.parse::<DateTime<FixedOffset>>().unwrap();
    assert_eq!(
        completed_session_as_of_at(at("2026-10-09T14:59:59+08:00")).unwrap(),
        NaiveDate::from_ymd_opt(2026, 10, 8).unwrap()
    );
    assert_eq!(
        completed_session_as_of_at(at("2026-10-09T15:00:00+08:00")).unwrap(),
        NaiveDate::from_ymd_opt(2026, 10, 9).unwrap()
    );
    assert_eq!(
        completed_session_as_of_at(at("2026-10-10T08:00:00+08:00")).unwrap(),
        NaiveDate::from_ymd_opt(2026, 10, 9).unwrap()
    );
    assert_eq!(
        completed_session_as_of_at(at("2026-10-07T08:00:00+08:00")).unwrap(),
        NaiveDate::from_ymd_opt(2026, 9, 30).unwrap()
    );
    assert!(completed_session_as_of_at(at("2026-10-09T15:00:00+00:00")).is_err());
    assert!(completed_session_as_of_at(at("2027-01-04T15:00:00+08:00")).is_err());
}

#[test]
fn hit_rate_window_counts_only_due_rows_on_verified_trading_dates() {
    let (_dir, db) = private_db();
    for (index, pred_date, target_date, hit) in [
        (0, "2026-09-30", "2026-10-08", false),
        (1, "2026-10-07", "2026-10-08", true), // closure inside natural-date range
        (2, "2026-10-08", "2026-10-09", true),
        (3, "2026-10-09", "2026-10-12", false),
        (4, "2026-10-09", "2026-10-13", true), // premature result must not look ahead
        (5, "2026-10-10", "2026-10-12", true), // weekend inside natural-date range
        (6, "2026-10-12", "2026-10-13", true), // current signal has no due target
        (7, "2026-10-13", "2026-10-14", true), // after as_of
    ] {
        let code = format!("TEST_CODE_window_{index}");
        db.save_prediction_legacy(pred_date, target_date, None, Some(&code), "up", 75., None)
            .unwrap();
        let id = db.get_prediction_by_code_date(&code, pred_date).unwrap().id;
        assert_eq!(
            db.update_prediction_result_by_id(id, if hit { 1.0 } else { -1.0 }, hit)
                .unwrap(),
            1
        );
    }
    let as_of = NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
    assert!(db
        .get_verified_prediction_sample_hit_rate(as_of, 1)
        .is_err());
    let two = db
        .get_verified_prediction_sample_hit_rate(as_of, 2)
        .unwrap();
    assert_eq!(
        (two.window_start.to_string(), two.samples, two.hits),
        ("2026-10-09".to_string(), 1, 0)
    );
    assert_eq!(two.rate, 0.);
    let three = db
        .get_verified_prediction_sample_hit_rate(as_of, 3)
        .unwrap();
    assert_eq!(
        (three.window_start.to_string(), three.samples, three.hits),
        ("2026-10-08".to_string(), 2, 1)
    );
    assert_eq!(three.rate, 0.5);
    let four = db
        .get_verified_prediction_sample_hit_rate(as_of, 4)
        .unwrap();
    assert_eq!(
        (four.window_start.to_string(), four.samples, four.hits),
        ("2026-09-30".to_string(), 3, 1)
    );
    assert!((four.rate - 1. / 3.).abs() < f64::EPSILON);
    assert!(db
        .get_verified_prediction_sample_hit_rate(as_of, 0)
        .is_err());
    assert!(db
        .get_verified_prediction_sample_hit_rate(NaiveDate::from_ymd_opt(2026, 10, 10).unwrap(), 1)
        .is_err());
    assert!(db
        .get_verified_prediction_sample_hit_rate(NaiveDate::from_ymd_opt(2027, 1, 4).unwrap(), 1)
        .is_err());
}

#[test]
fn hit_rate_rejects_a_recorded_hit_without_a_valid_return() {
    let (_dir, db) = private_db();
    let code = "TEST_CODE_incomplete_outcome";
    db.save_prediction_legacy(
        "2026-10-08",
        "2026-10-09",
        None,
        Some(code),
        "up",
        75.,
        None,
    )
    .unwrap();
    let id = db
        .get_prediction_by_code_date(code, "2026-10-08")
        .unwrap()
        .id;
    db.update_prediction_result_by_id(id, 1.0, true).unwrap();
    let as_of = NaiveDate::from_ymd_opt(2026, 10, 9).unwrap();
    assert_eq!(
        db.get_verified_prediction_sample_hit_rate(as_of, 2)
            .unwrap()
            .samples,
        1
    );

    let mut conn = db.get_conn().unwrap();
    diesel::sql_query("UPDATE prediction_tracker SET actual_change = NULL WHERE id = ?")
        .bind::<diesel::sql_types::Integer, _>(id)
        .execute(&mut conn)
        .unwrap();
    drop(conn);
    let error = db
        .get_verified_prediction_sample_hit_rate(as_of, 2)
        .unwrap_err()
        .to_string();
    assert!(error.contains("缺少有效实际收益"), "{error}");
}

#[test]
fn candidate_promotion_counts_only_complete_outcomes_after_their_session() {
    let (_dir, db) = private_db();
    for (code, pred_date, target_date, detail, hit) in [
        (
            "TEST_CODE_due_win",
            "2026-09-23",
            "2026-10-08",
            "candidate-strong",
            Some(true),
        ),
        (
            "TEST_CODE_due_win",
            "2026-09-23",
            "2026-10-08",
            "candidate-strong",
            Some(false),
        ),
        (
            "TEST_CODE_due_loss",
            "2026-09-23",
            "2026-10-08",
            "candidate-strong",
            Some(false),
        ),
        (
            "TEST_CODE_today_early",
            "2026-09-24",
            "2026-10-09",
            "candidate-strong",
            Some(true),
        ),
        (
            "TEST_CODE_pending",
            "2026-09-23",
            "2026-10-08",
            "candidate-strong",
            None,
        ),
        (
            "TEST_CODE_other",
            "2026-09-24",
            "2026-10-08",
            "other",
            Some(true),
        ),
    ] {
        db.save_prediction_legacy(
            pred_date,
            target_date,
            None,
            Some(code),
            "up",
            75.,
            Some(detail),
        )
        .unwrap();
        if let Some(hit) = hit {
            let id = db.get_prediction_by_code_date(code, pred_date).unwrap().id;
            assert_eq!(
                db.update_prediction_result_by_id(id, if hit { 1.0 } else { -1.0 }, hit)
                    .unwrap(),
                1
            );
        }
    }
    db.save_prediction_legacy(
        "2026-09-23",
        "2026-10-08",
        None,
        Some("TEST_CODE_partial"),
        "up",
        75.,
        Some("candidate-strong"),
    )
    .unwrap();
    let mut conn = db.get_conn().unwrap();
    diesel::sql_query(
        "UPDATE prediction_tracker SET actual_change = 1.0 WHERE stock_code = 'TEST_CODE_partial'",
    )
    .execute(&mut conn)
    .unwrap();
    drop(conn);

    let before_close = "2026-10-09T14:59:59+08:00"
        .parse::<DateTime<FixedOffset>>()
        .unwrap();
    let after_close = "2026-10-09T15:00:00+08:00"
        .parse::<DateTime<FixedOffset>>()
        .unwrap();
    let previous_session = completed_session_as_of_at(before_close).unwrap();
    assert_eq!(previous_session.to_string(), "2026-10-08");
    assert_eq!(
        db.candidate_promotion_samples(&previous_session.to_string())
            .unwrap(),
        (2, 1),
        "prematurely written target-day results must not open the promotion gate"
    );
    let completed_today = completed_session_as_of_at(after_close).unwrap();
    assert_eq!(
        db.candidate_promotion_samples(&completed_today.to_string())
            .unwrap(),
        (3, 2),
        "one complete row per candidate and prediction day is counted"
    );

    // Legacy SQLite content can bypass save_prediction's date validation.
    let mut conn = db.get_conn().unwrap();
    diesel::sql_query(
        "INSERT INTO prediction_tracker \
         (pred_date,target_date,stock_code,pred_direction,pred_score,pred_detail,actual_change,hit) \
         VALUES ('2026-09-24','2026-10-99','TEST_CODE_bad_date','up',75,'candidate-strong',1,1)",
    )
    .execute(&mut conn)
    .unwrap();
    drop(conn);
    assert!(matches!(
        db.candidate_promotion_samples("2026-10-09"),
        Err(
            crate::database::CandidatePromotionEvidenceError::IneligibleFirstSample {
                reason: "target_date_invalid",
                ..
            }
        )
    ));
}

#[test]
fn candidate_promotion_never_substitutes_a_later_duplicate_for_the_first_row() {
    let (_dir, db) = private_db();
    for (code, pred_date, target_date, hit) in [
        ("TEST_CODE_control", "2026-09-23", "2026-10-08", Some(true)),
        ("TEST_CODE_first_pending", "2026-09-23", "2026-10-08", None),
        (
            "TEST_CODE_first_pending",
            "2026-09-23",
            "2026-10-08",
            Some(true),
        ),
        (
            "TEST_CODE_first_future",
            "2026-09-24",
            "2026-10-09",
            Some(false),
        ),
        (
            "TEST_CODE_first_future",
            "2026-09-24",
            "2026-10-08",
            Some(true),
        ),
    ] {
        db.save_prediction_legacy(
            pred_date,
            target_date,
            None,
            Some(code),
            "up",
            75.,
            Some("candidate-strong"),
        )
        .unwrap();
        if let Some(hit) = hit {
            let id = db.get_prediction_by_code_date(code, pred_date).unwrap().id;
            assert_eq!(
                db.update_prediction_result_by_id(id, if hit { 1.0 } else { -1.0 }, hit)
                    .unwrap(),
                1
            );
        }
    }

    assert_eq!(
        db.candidate_promotion_samples("2026-10-08").unwrap(),
        (1, 1),
        "the complete later rows cannot replace pending or not-yet-due first rows"
    );
    assert_eq!(
        db.candidate_promotion_samples("2026-10-09").unwrap(),
        (2, 1),
        "once due, the first future row contributes its own miss, not the later hit"
    );
}

#[test]
fn candidate_promotion_requires_first_row_fifth_trading_day_authority() {
    use crate::database::CandidatePromotionEvidenceError;

    for (pred_date, target_date, reason) in [
        ("2026-10-01", "2026-10-09", "pred_date_not_trading"),
        ("2026-09-23", "2026-10-09", "target_not_fifth_trading_day"),
        ("2026-09-24", "2026-09-23", "target_not_fifth_trading_day"),
    ] {
        let (_dir, db) = private_db();
        db.save_prediction_legacy(
            pred_date,
            target_date,
            None,
            Some("TEST_CODE_ineligible"),
            "up",
            75.,
            Some("candidate-strong"),
        )
        .unwrap();
        assert!(matches!(
            db.candidate_promotion_samples("2026-10-09"),
            Err(CandidatePromotionEvidenceError::IneligibleFirstSample {
                reason: found,
                ..
            }) if found == reason
        ));
    }

    let (_dir, db) = private_db();
    db.save_prediction_legacy(
        "2024-12-30",
        "2025-01-07",
        None,
        Some("TEST_CODE_legacy_calendar"),
        "up",
        75.,
        Some("candidate-strong"),
    )
    .unwrap();
    assert!(matches!(
        db.candidate_promotion_samples("2026-10-09"),
        Err(CandidatePromotionEvidenceError::CalendarUnavailable(_))
    ));

    let (_dir, db) = private_db();
    db.save_prediction_legacy(
        "2026-10-01",
        "2026-10-02",
        None,
        Some("TEST_CODE_unrelated"),
        "up",
        75.,
        Some("other-prediction"),
    )
    .unwrap();
    assert_eq!(
        db.candidate_promotion_samples("2026-10-09").unwrap(),
        (0, 0)
    );

    let (_dir, db) = private_db();
    db.save_prediction_legacy(
        "2026-10-09",
        "2026-10-13",
        None,
        Some("TEST_CODE_future"),
        "up",
        75.,
        Some("candidate-strong"),
    )
    .unwrap();
    assert_eq!(
        db.candidate_promotion_samples("2026-10-08").unwrap(),
        (0, 0),
        "future prediction dates are outside the completed candidate cohort"
    );
    let mut conn = db.get_conn().unwrap();
    diesel::sql_query(
        "INSERT INTO prediction_tracker \
         (pred_date,target_date,stock_code,pred_direction,pred_score,pred_detail,actual_change,hit) \
         VALUES ('not-a-date','2026-10-08','TEST_CODE_bad_pred_date','up',75,'candidate-strong',1,1)",
    )
    .execute(&mut conn)
    .unwrap();
    drop(conn);
    assert!(matches!(
        db.candidate_promotion_samples("2026-10-08"),
        Err(CandidatePromotionEvidenceError::IneligibleFirstSample {
            reason: "pred_date_invalid",
            ..
        })
    ));
}

#[test]
fn candidate_promotion_rejects_invalid_due_first_row_outcomes() {
    use crate::database::CandidatePromotionEvidenceError;
    use diesel::sql_types::{Double, Integer, Nullable, Text};

    for (case, code, direction, actual_change, hit, expected_reason) in [
        (
            "theme_only",
            None,
            "up",
            Some(1.0),
            Some(1),
            "stock_code_missing",
        ),
        (
            "invalid_code",
            Some("INVALID SYMBOL"),
            "up",
            Some(1.0),
            Some(1),
            "stock_code_invalid",
        ),
        (
            "infinite_return",
            Some("TEST_CODE_inf"),
            "up",
            Some(f64::INFINITY),
            Some(1),
            "actual_change_invalid",
        ),
        (
            "below_return_floor",
            Some("TEST_CODE_floor"),
            "up",
            Some(-101.0),
            Some(0),
            "actual_change_invalid",
        ),
        (
            "hit_above_boolean",
            Some("TEST_CODE_hit_two"),
            "up",
            Some(1.0),
            Some(2),
            "hit_invalid",
        ),
        (
            "negative_hit",
            Some("TEST_CODE_hit_negative"),
            "up",
            Some(1.0),
            Some(-1),
            "hit_invalid",
        ),
        (
            "wrong_direction",
            Some("TEST_CODE_direction"),
            "down",
            Some(1.0),
            Some(1),
            "pred_direction_not_up",
        ),
        (
            "false_win_at_threshold",
            Some("TEST_CODE_threshold"),
            "up",
            Some(0.5),
            Some(1),
            "hit_outcome_mismatch",
        ),
        (
            "false_miss_above_threshold",
            Some("TEST_CODE_false_miss"),
            "up",
            Some(1.0),
            Some(0),
            "hit_outcome_mismatch",
        ),
    ] {
        let (_dir, db) = private_db();
        let mut conn = db.get_conn().unwrap();
        diesel::sql_query(
            "INSERT INTO prediction_tracker \
             (pred_date,target_date,theme_name,stock_code,pred_direction,pred_score,pred_detail,actual_change,hit) \
             VALUES ('2026-09-23','2026-10-08','candidate-test',?1,?2,75,'candidate-strong',?3,?4)",
        )
        .bind::<Nullable<Text>, _>(code)
        .bind::<Text, _>(direction)
        .bind::<Nullable<Double>, _>(actual_change)
        .bind::<Nullable<Integer>, _>(hit)
        .execute(&mut conn)
        .unwrap();
        drop(conn);

        let observed = db.candidate_promotion_samples("2026-10-08");
        assert!(
            matches!(
                &observed,
                Err(CandidatePromotionEvidenceError::IneligibleFirstSample {
                    reason: found,
                    ..
                }) if *found == expected_reason
            ),
            "{case}: {observed:?}"
        );
    }
}

fn close(db: &DatabaseManager, code: &str, date: &str, value: f64) {
    let mut conn = db.get_conn().unwrap();
    diesel::sql_query("INSERT INTO stock_daily (code,date,close) VALUES (?1,?2,?3)")
        .bind::<diesel::sql_types::Text, _>(code)
        .bind::<diesel::sql_types::Text, _>(date)
        .bind::<diesel::sql_types::Double, _>(value)
        .execute(&mut conn)
        .unwrap();
}

fn qualified_status(db: &DatabaseManager, code: &str, date: &str, status: &str) {
    assert!(code.starts_with("TEST_CODE_"));
    let mut conn = db.get_conn().unwrap();
    diesel::sql_query(
        "INSERT INTO qualified_daily_trading_status \
         (code,date,status,contract_version,source,source_at,observed_at,batch_id) \
         VALUES (?1,?2,?3,'TEST_CODE_AUTHORITY_V1','TEST_CODE_AUTHORITY', \
                 '2026-02-26T00:00:00Z','2026-02-26T00:00:01Z','TEST_CODE_BATCH')",
    )
    .bind::<diesel::sql_types::Text, _>(code)
    .bind::<diesel::sql_types::Text, _>(date)
    .bind::<diesel::sql_types::Text, _>(status)
    .execute(&mut conn)
    .unwrap();
}

fn qualified_close(db: &DatabaseManager, code: &str, date: &str, value: f64) {
    close(db, code, date, value);
    qualified_status(db, code, date, "trading");
}

#[tokio::test]
async fn task2_candidate_save_reports_each_row_and_worker_failure() {
    let (_dir, db) = private_db();
    diesel::sql_query("CREATE TRIGGER reject_candidate BEFORE INSERT ON prediction_tracker WHEN NEW.stock_code = 'TEST_CODE_invalid' BEGIN SELECT RAISE(ABORT, 'injected row storage failure'); END")
        .execute(&mut db.get_conn().unwrap()).unwrap();
    let samples = vec![
        ("TEST_CODE_valid1".into(), 70.),
        ("TEST_CODE_invalid".into(), 75.),
        ("TEST_CODE_valid2".into(), 80.),
        ("TEST_CODE_valid1".into(), 71.),
    ];
    let report = save_candidate_samples(&db, "2026-09-23", "2026-10-08", &samples);
    assert_eq!((report.attempted, report.saved, report.unknown), (4, 3, 0));
    assert_eq!(report.failures.len(), 1);
    assert_eq!(report.failures[0].code, "TEST_CODE_invalid");
    assert!(report.failures[0]
        .error
        .contains("injected row storage failure"));
    assert!(!report.is_complete());
    let rows = db.get_pending_predictions("2026-09-23").unwrap();
    assert_eq!(
        rows.len(),
        3,
        "each successful save remains a prediction row"
    );
    assert_eq!(report.saved_rows.len(), rows.len());
    for saved in &report.saved_rows {
        let row = rows
            .iter()
            .find(|row| i64::from(row.id) == saved.prediction_row_id)
            .expect("reported ID must be the row actually inserted");
        assert_eq!(row.stock_code.as_deref(), Some(saved.code.as_str()));
        assert_eq!(row.pred_date, "2026-09-23");
        assert_eq!(row.target_date, "2026-10-08");
    }
    assert_ne!(
        report.saved_rows[0].prediction_row_id, report.saved_rows[2].prediction_row_id,
        "same-code resampling remains distinct in the historical ledger"
    );
    for (saved, actual_change, hit) in [
        (&report.saved_rows[0], 1.0, true),
        (&report.saved_rows[1], -1.0, false),
        (&report.saved_rows[2], -1.0, false),
    ] {
        let row_id = i32::try_from(saved.prediction_row_id).unwrap();
        assert_eq!(
            db.update_prediction_result_by_id(row_id, actual_change, hit)
                .unwrap(),
            1
        );
    }
    assert_eq!(
        db.candidate_promotion_samples("2026-10-08").unwrap(),
        (2, 1),
        "the later same-code row retains its ID but cannot change the promotion denominator"
    );
    let worker = tokio::task::spawn_blocking(|| -> CandidateSampleSaveReport {
        panic!("injected worker failure")
    });
    let report = collect_candidate_save_worker(worker, 3).await;
    assert_eq!((report.saved, report.unknown), (0, 3));
    assert!(report.saved_rows.is_empty());
    assert!(report.worker_error.as_deref().unwrap().contains("panic"));
}

#[test]
fn candidate_sample_id_is_not_reused_when_insert_is_ignored() {
    let (_dir, db) = private_db();
    let first_id = db
        .save_prediction_with_id(
            "2026-02-02",
            "2026-02-25",
            None,
            Some("TEST_CODE_first"),
            "up",
            70.,
            None,
            None,
            None,
        )
        .unwrap();
    diesel::sql_query("CREATE TRIGGER ignore_candidate BEFORE INSERT ON prediction_tracker WHEN NEW.stock_code = 'TEST_CODE_ignored' BEGIN SELECT RAISE(IGNORE); END")
        .execute(&mut db.get_conn().unwrap())
        .unwrap();
    assert!(db
        .save_prediction_with_id(
            "2026-02-02",
            "2026-02-25",
            None,
            Some("TEST_CODE_ignored"),
            "up",
            75.,
            None,
            None,
            None,
        )
        .is_err());
    let rows = db.get_pending_predictions("2026-02-02").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(i64::from(rows[0].id), first_id);
}

#[test]
fn task2_zero_update_errors_and_invalid_inputs_remain_pending() {
    let (_dir, db) = private_db();
    for (code, direction, price) in [
        ("TEST_CODE_zero", "up", 125.),
        ("TEST_CODE_error", "up", 125.),
        ("TEST_CODE_unknown", "new-direction", 125.),
        ("TEST_CODE_invalid_close", "up", 0.),
        ("TEST_CODE_accepted", "UP", 125.),
    ] {
        db.save_prediction_legacy(
            "2026-02-02",
            "2026-02-25",
            None,
            Some(code),
            direction,
            80.,
            None,
        )
        .unwrap();
        qualified_close(&db, code, "2026-02-02", 100.);
        qualified_close(&db, code, "2026-02-25", price);
    }
    let mut conn = db.get_conn().unwrap();
    diesel::sql_query("CREATE TRIGGER skip_update BEFORE UPDATE ON prediction_tracker WHEN OLD.stock_code = 'TEST_CODE_zero' BEGIN SELECT RAISE(IGNORE); END").execute(&mut conn).unwrap();
    diesel::sql_query("CREATE TRIGGER fail_update BEFORE UPDATE ON prediction_tracker WHEN OLD.stock_code = 'TEST_CODE_error' BEGIN SELECT RAISE(ABORT, 'injected update failure'); END").execute(&mut conn).unwrap();
    drop(conn);
    let report =
        verify_due_predictions(&db, chrono::NaiveDate::from_ymd_opt(2026, 2, 26).unwrap()).unwrap();
    assert_eq!(
        (report.pending, report.verified, report.hits, report.raced),
        (5, 1, 1, 1)
    );
    assert_eq!(report.errors.len(), 3);
    let accepted = db
        .get_prediction_by_code_date("TEST_CODE_accepted", "2026-02-02")
        .unwrap();
    assert_eq!(
        db.update_prediction_result_by_id(accepted.id, -25., false)
            .unwrap(),
        0
    );
    assert_eq!(
        db.get_prediction_by_code_date("TEST_CODE_accepted", "2026-02-02")
            .unwrap()
            .actual_change,
        Some(25.)
    );
    let zero = db
        .get_prediction_by_code_date("TEST_CODE_zero", "2026-02-02")
        .unwrap();
    assert_eq!(zero.hit, None);
    for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -100.01] {
        assert!(db
            .update_prediction_result_by_id(zero.id, invalid, false)
            .is_err());
    }
}

#[test]
fn task2_due_scan_distinguishes_storage_read_error_from_missing_close() {
    let (_dir, db) = private_db();
    db.save_prediction_legacy(
        "2026-02-02",
        "2026-02-25",
        None,
        Some("TEST_CODE_read_error"),
        "up",
        80.,
        None,
    )
    .unwrap();
    diesel::sql_query("ALTER TABLE stock_daily RENAME TO test_unavailable_daily")
        .execute(&mut db.get_conn().unwrap())
        .unwrap();
    let report =
        verify_due_predictions(&db, chrono::NaiveDate::from_ymd_opt(2026, 2, 26).unwrap()).unwrap();
    assert_eq!(
        (report.pending, report.verified, report.deferred),
        (1, 0, 0)
    );
    // The frozen target and horizon scan independently report storage failure;
    // neither may turn it into a missing-price deferral or acknowledge a write.
    assert_eq!(report.errors.len(), 2);
    assert!(report
        .errors
        .iter()
        .all(|error| error.contains("stock_daily")));
    assert_eq!(report.windows.errors.len(), 1);
    assert_eq!(report.windows.deferred_windows, 0);
    assert_eq!(report.windows.verified_t1, 0);
}

#[tokio::test]
async fn task2_due_rows_exact_dates_large_returns_direction_and_keyset() {
    let (_dir, db) = private_db();
    for (code, direction, target, value) in [
        ("TEST_CODE_up", "up", "2026-02-25", Some(125.)),
        ("TEST_CODE_down", "看空", "2026-02-25", Some(75.)),
        ("TEST_CODE_cn", "看多", "2026-02-25", Some(125.)),
        ("TEST_CODE_missing", "up", "2026-02-25", None),
        ("TEST_CODE_future", "up", "2026-02-27", Some(125.)),
    ] {
        db.save_prediction_legacy("2026-02-02", target, None, Some(code), direction, 80., None)
            .unwrap();
        qualified_close(&db, code, "2026-02-02", 100.);
        if let Some(value) = value {
            qualified_close(&db, code, target, value);
        }
    }
    close(&db, "TEST_CODE_missing", "2026-02-26", 130.);
    db.save_prediction_legacy(
        "2026-02-02",
        "2026-02-25",
        Some("theme-only"),
        None,
        "up",
        80.,
        None,
    )
    .unwrap();
    let report = verify_due_predictions_with_page_size(
        &db,
        chrono::NaiveDate::from_ymd_opt(2026, 2, 26).unwrap(),
        1,
    )
    .unwrap();
    assert_eq!(
        (
            report.pending,
            report.verified,
            report.hits,
            report.deferred
        ),
        (5, 3, 3, 2)
    );
    assert!(report.errors.is_empty());
    assert_eq!(
        db.get_prediction_by_code_date("TEST_CODE_up", "2026-02-02")
            .unwrap()
            .actual_change,
        Some(25.)
    );
    assert_eq!(
        db.get_prediction_by_code_date("TEST_CODE_down", "2026-02-02")
            .unwrap()
            .actual_change,
        Some(-25.)
    );
    for code in ["TEST_CODE_missing", "TEST_CODE_future"] {
        assert_eq!(
            db.get_prediction_by_code_date(code, "2026-02-02")
                .unwrap()
                .hit,
            None
        );
    }
    let rerun = verify_due_predictions_with_page_size(
        &db,
        chrono::NaiveDate::from_ymd_opt(2026, 2, 26).unwrap(),
        1,
    )
    .unwrap();
    assert_eq!((rerun.pending, rerun.verified, rerun.deferred), (2, 0, 2));
}

#[tokio::test]
async fn neutral_direction_counts_verified_sideways_moves_as_hits() {
    let (_dir, db) = private_db();
    let pred_date = "2026-10-08";
    let target_date = "2026-10-09";
    for (code, target_close, expected_hit) in [
        ("TEST_CODE_neutral_flat", 100.0, true),
        ("TEST_CODE_neutral_up_edge", 100.5, true),
        ("TEST_CODE_neutral_down_edge", 99.5, true),
        ("TEST_CODE_neutral_up_miss", 100.75, false),
        ("TEST_CODE_neutral_down_miss", 99.25, false),
    ] {
        db.save_prediction_legacy(pred_date, target_date, None, Some(code), "中性", 80., None)
            .unwrap();
        qualified_close(&db, code, pred_date, 100.);
        qualified_close(&db, code, target_date, target_close);
        let outcome = verify_one(&db, code, pred_date, target_date, "中性")
            .await
            .unwrap();
        assert_eq!(outcome.hit, expected_hit, "{code}");
    }

    for code in ["TEST_CODE_neutral_missing", "TEST_CODE_neutral_unqualified"] {
        db.save_prediction_legacy(pred_date, target_date, None, Some(code), "中性", 80., None)
            .unwrap();
        qualified_close(&db, code, pred_date, 100.);
    }
    close(&db, "TEST_CODE_neutral_unqualified", target_date, 100.);

    let as_of = NaiveDate::from_ymd_opt(2026, 10, 9).unwrap();
    let report = verify_due_predictions(&db, as_of).unwrap();
    assert_eq!(
        (
            report.pending,
            report.verified,
            report.hits,
            report.deferred
        ),
        (7, 5, 3, 2)
    );
    assert!(report.errors.is_empty());
    assert!(db
        .get_verified_prediction_sample_hit_rate(as_of, 1)
        .is_err());
    let week = db
        .get_verified_prediction_sample_hit_rate(as_of, 5)
        .unwrap();
    assert_eq!((week.samples, week.hits), (5, 3));
    assert_eq!(week.rate, 0.6);
}

#[tokio::test]
async fn task2_missing_target_close_never_uses_future_price() {
    let (_dir, db) = private_db();
    qualified_close(&db, "TEST_CODE_exact", "2026-02-02", 100.);
    qualified_close(&db, "TEST_CODE_exact", "2026-02-26", 125.);
    assert!(
        verify_one(&db, "TEST_CODE_exact", "2026-02-02", "2026-02-25", "看多")
            .await
            .is_none()
    );
}

#[test]
fn task2_suspended_target_close_keeps_prediction_pending() {
    let (_dir, db) = private_db();
    let code = "TEST_CODE_suspended";
    db.save_prediction_legacy(
        "2026-02-02",
        "2026-02-25",
        None,
        Some(code),
        "up",
        80.,
        None,
    )
    .unwrap();
    qualified_close(&db, code, "2026-02-02", 100.);
    close(&db, code, "2026-02-25", 125.);
    qualified_status(&db, code, "2026-02-25", "trading");
    diesel::sql_query("UPDATE stock_daily SET is_suspended = 1 WHERE code = ?1 AND date = ?2")
        .bind::<diesel::sql_types::Text, _>(code)
        .bind::<diesel::sql_types::Text, _>("2026-02-25")
        .execute(&mut db.get_conn().unwrap())
        .unwrap();

    let report =
        verify_due_predictions(&db, chrono::NaiveDate::from_ymd_opt(2026, 2, 26).unwrap()).unwrap();
    assert_eq!(
        (report.pending, report.verified, report.deferred),
        (1, 0, 1)
    );
    assert_eq!(
        db.get_prediction_by_code_date(code, "2026-02-02")
            .unwrap()
            .hit,
        None
    );
}

#[test]
fn d10_suspended_status_blocks_stale_close_even_when_legacy_flag_is_false() {
    let (_dir, db) = private_db();
    let code = "TEST_CODE_authority_suspended";
    db.save_prediction_legacy(
        "2026-02-02",
        "2026-02-25",
        None,
        Some(code),
        "up",
        80.,
        None,
    )
    .unwrap();
    qualified_close(&db, code, "2026-02-02", 100.);
    close(&db, code, "2026-02-25", 125.);
    qualified_status(&db, code, "2026-02-25", "suspended");

    let report =
        verify_due_predictions(&db, chrono::NaiveDate::from_ymd_opt(2026, 2, 26).unwrap()).unwrap();
    assert_eq!(
        (report.pending, report.verified, report.deferred),
        (1, 0, 1)
    );
    assert_eq!(
        db.get_prediction_by_code_date(code, "2026-02-02")
            .unwrap()
            .actual_change,
        None
    );
}

#[test]
fn d10_legacy_default_false_and_unknown_start_day_cannot_verify() {
    let (_dir, db) = private_db();
    let code = "TEST_CODE_unknown_trade_state";
    db.save_prediction_legacy(
        "2026-02-02",
        "2026-02-25",
        None,
        Some(code),
        "up",
        80.,
        None,
    )
    .unwrap();
    close(&db, code, "2026-02-02", 100.);
    close(&db, code, "2026-02-25", 125.);

    let as_of = chrono::NaiveDate::from_ymd_opt(2026, 2, 26).unwrap();
    let first = verify_due_predictions(&db, as_of).unwrap();
    assert_eq!((first.pending, first.verified, first.deferred), (1, 0, 1));

    qualified_status(&db, code, "2026-02-25", "trading");
    let second = verify_due_predictions(&db, as_of).unwrap();
    assert_eq!(
        (second.pending, second.verified, second.deferred),
        (1, 0, 1)
    );

    qualified_status(&db, code, "2026-02-02", "trading");
    let third = verify_due_predictions(&db, as_of).unwrap();
    assert_eq!((third.pending, third.verified, third.deferred), (1, 1, 0));
    assert_eq!(
        db.get_prediction_by_code_date(code, "2026-02-02")
            .unwrap()
            .actual_change,
        Some(25.)
    );
}
