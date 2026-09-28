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
    ];
    let report = save_candidate_samples(&db, "2026-02-02", "2026-02-25", &samples);
    assert_eq!((report.attempted, report.saved, report.unknown), (3, 2, 0));
    assert_eq!(report.failures.len(), 1);
    assert_eq!(report.failures[0].code, "TEST_CODE_invalid");
    assert!(report.failures[0]
        .error
        .contains("injected row storage failure"));
    assert!(!report.is_complete());
    assert_eq!(db.get_pending_predictions("2026-02-02").unwrap().len(), 2);
    let worker = tokio::task::spawn_blocking(|| -> CandidateSampleSaveReport {
        panic!("injected worker failure")
    });
    let report = collect_candidate_save_worker(worker, 3).await;
    assert_eq!((report.saved, report.unknown), (0, 3));
    assert!(report.worker_error.as_deref().unwrap().contains("panic"));
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
    assert_eq!(report.errors.len(), 1);
    assert!(report.errors[0].contains("stock_daily"));
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
