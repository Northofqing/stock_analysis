use super::*;
use diesel::RunQueryDsl;

fn private_db() -> (tempfile::TempDir, DatabaseManager) {
    let dir = tempfile::tempdir().unwrap();
    let db = DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_prediction.db"))
        .unwrap();
    (dir, db)
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
