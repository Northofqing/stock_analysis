//! Synthetic SQLite facts test settlement, never Provider or trading admission.
use super::*;
use diesel::RunQueryDsl;

fn date(value: &str) -> NaiveDate {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").unwrap()
}
fn database() -> (tempfile::TempDir, DatabaseManager) {
    let dir = tempfile::tempdir().unwrap();
    let db =
        DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_windows.db")).unwrap();
    (dir, db)
}
fn sample(db: &DatabaseManager, code: &str, direction: &str) -> i32 {
    assert!(code.starts_with("TEST_CODE_"));
    db.save_prediction_legacy(
        "2026-09-30",
        "2026-10-14",
        None,
        Some(code),
        direction,
        75.,
        None,
    )
    .unwrap();
    db.get_prediction_by_code_date(code, "2026-09-30")
        .unwrap()
        .id
}
fn bar(db: &DatabaseManager, code: &str, day: &str, close: f64, status: Option<&str>) {
    db.save_daily_record(
        code,
        date(day),
        Some(close),
        Some(close),
        Some(close),
        Some(close),
        Some(100.),
        Some(close * 100.),
        None,
        None,
        None,
        None,
        None,
        Some("TEST_CODE_window_bar"),
    )
    .unwrap();
    if let Some(status) = status {
        let mut conn = db.get_conn().unwrap();
        diesel::sql_query("INSERT INTO qualified_daily_trading_status(code,date,status,contract_version,source,source_at,observed_at,batch_id) VALUES (?1,?2,?3,'TEST_CODE_status','TEST_CODE_fixture','2026-10-14T07:00:00Z','2026-10-14T07:00:01Z','TEST_CODE_batch')")
            .bind::<diesel::sql_types::Text,_>(code)
            .bind::<diesel::sql_types::Text,_>(day)
            .bind::<diesel::sql_types::Text,_>(status).execute(&mut conn).unwrap();
    }
}
fn row(db: &DatabaseManager, id: i32) -> Row {
    let mut conn = db.get_conn().unwrap();
    diesel::sql_query(format!(
        "SELECT {COLUMNS} FROM prediction_tracker WHERE id=?1"
    ))
    .bind::<diesel::sql_types::Integer, _>(id)
    .get_result(&mut conn)
    .unwrap()
}

#[test]
fn horizon_settlement_uses_sessions_matures_incrementally_and_preserves_primary() {
    let (_dir, db) = database();
    let code = "TEST_CODE_window_holiday";
    let id = sample(&db, code, "up");
    db.update_prediction_result_by_id(id, 88., true).unwrap();
    for (day, close) in [
        ("2026-09-30", 100.),
        ("2026-10-08", 102.),
        ("2026-10-12", 99.),
        ("2026-10-14", 104.),
    ] {
        bar(&db, code, day, close, Some("trading"));
    }
    let first = verify_windows(&db, date("2026-10-08"), id, 1).unwrap();
    assert_eq!(
        (
            first.verified_t1,
            first.verified_t3,
            first.verified_t5,
            first.deferred_windows
        ),
        (1, 0, 0, 2)
    );
    assert!(first.errors.is_empty());
    let second = verify_windows(&db, date("2026-10-12"), id, 1).unwrap();
    assert_eq!(
        (second.verified_t1, second.verified_t3, second.verified_t5),
        (0, 1, 0)
    );
    let third = verify_windows(&db, date("2026-10-14"), id, 1).unwrap();
    assert_eq!(
        (third.verified_t1, third.verified_t3, third.verified_t5),
        (0, 0, 1)
    );
    let saved = row(&db, id);
    assert!((saved.actual_change_t1.unwrap() - 2.).abs() < 1e-9);
    assert!((saved.actual_change_t3.unwrap() + 1.).abs() < 1e-9);
    assert!((saved.actual_change_t5.unwrap() - 4.).abs() < 1e-9);
    assert_eq!(
        (saved.hit_t1, saved.hit_t3, saved.hit_t5),
        (Some(1), Some(0), Some(1))
    );
    let original = db.get_prediction_by_code_date(code, "2026-09-30").unwrap();
    assert_eq!(
        (original.actual_change, original.hit, original.target_date),
        (Some(88.), Some(1), "2026-10-14".into())
    );
    assert_eq!(
        verify_windows(&db, date("2026-10-14"), id, 1)
            .unwrap()
            .scanned_rows,
        0
    );
}

#[test]
fn horizon_settlement_requires_status_and_exact_close_and_honors_direction() {
    let (_dir, db) = database();
    for (code, direction, close, status) in [
        ("TEST_CODE_window_unknown", "up", 102., None),
        ("TEST_CODE_window_suspended", "up", 102., Some("suspended")),
        ("TEST_CODE_window_down", "down", 98., Some("trading")),
        (
            "TEST_CODE_window_neutral",
            "neutral",
            100.2,
            Some("trading"),
        ),
        ("TEST_CODE_window_up_miss", "up", 98., Some("trading")),
    ] {
        sample(&db, code, direction);
        bar(&db, code, "2026-09-30", 100., Some("trading"));
        bar(&db, code, "2026-10-08", close, status);
    }
    let high = db.prediction_verification_high_water_id().unwrap();
    let report = verify_windows(&db, date("2026-10-08"), high, 1).unwrap();
    assert_eq!(
        (
            report.scanned_rows,
            report.verified_t1,
            report.deferred_windows
        ),
        (5, 3, 12)
    );
    assert!(report.errors.is_empty());
    for (id, hit) in [
        (1, None),
        (2, None),
        (3, Some(1)),
        (4, Some(1)),
        (5, Some(0)),
    ] {
        assert_eq!(row(&db, id).hit_t1, hit);
    }
    // A later arbitrary daily overwrite revokes status and cannot settle T+3.
    bar(&db, "TEST_CODE_window_down", "2026-10-12", 97., None);
    let report = verify_windows(&db, date("2026-10-12"), high, 1).unwrap();
    assert_eq!(report.verified_t3, 0);
    assert_eq!(row(&db, 3).hit_t1, Some(1));
}

#[test]
fn horizon_settlement_retains_partial_legacy_and_excludes_beyond_high_water() {
    let (_dir, db) = database();
    let code = "TEST_CODE_window_partial";
    let id = sample(&db, code, "up");
    for day in ["2026-09-30", "2026-10-08", "2026-10-12", "2026-10-14"] {
        bar(&db, code, day, 100., Some("trading"));
    }
    let mut conn = db.get_conn().unwrap();
    diesel::sql_query("UPDATE prediction_tracker SET actual_change_t1=2 WHERE id=?1")
        .bind::<diesel::sql_types::Integer, _>(id)
        .execute(&mut conn)
        .unwrap();
    let later = sample(&db, "TEST_CODE_window_later", "up");
    let report = verify_windows(&db, date("2026-10-14"), id, 1).unwrap();
    assert_eq!(
        (
            report.scanned_rows,
            report.verified_t1,
            report.verified_t3,
            report.verified_t5
        ),
        (1, 0, 1, 1)
    );
    assert_eq!(report.errors.len(), 1);
    let saved = row(&db, id);
    assert_eq!((saved.actual_change_t1, saved.hit_t1), (Some(2.), None));
    assert!(row(&db, later).actual_change_t1.is_none());
}

#[test]
fn horizon_settlement_write_failure_rolls_back_prior_window_and_ack_counts() {
    let (_dir, db) = database();
    let code = "TEST_CODE_window_rollback";
    let id = sample(&db, code, "up");
    for day in ["2026-09-30", "2026-10-08", "2026-10-12", "2026-10-14"] {
        bar(&db, code, day, 100., Some("trading"));
    }
    let mut conn = db.get_conn().unwrap();
    diesel::sql_query("CREATE TRIGGER deny_t3 BEFORE UPDATE OF hit_t3 ON prediction_tracker BEGIN SELECT RAISE(ABORT,'TEST_CODE_window_update_failure'); END")
        .execute(&mut conn).unwrap();
    let report = verify_windows(&db, date("2026-10-14"), id, 1).unwrap();
    assert_eq!(
        (report.verified_t1, report.verified_t3, report.verified_t5),
        (0, 0, 0)
    );
    assert_eq!(report.errors.len(), 1);
    let saved = row(&db, id);
    assert_eq!(
        (saved.hit_t1, saved.hit_t3, saved.hit_t5),
        (None, None, None)
    );
    diesel::sql_query("DROP TRIGGER deny_t3")
        .execute(&mut conn)
        .unwrap();
    let retried = verify_windows(&db, date("2026-10-14"), id, 1).unwrap();
    assert_eq!(
        (
            retried.verified_t1,
            retried.verified_t3,
            retried.verified_t5
        ),
        (1, 1, 1)
    );
}
