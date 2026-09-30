use super::*;
use diesel::RunQueryDsl;

fn private_db() -> (tempfile::TempDir, DatabaseManager) {
    let dir = tempfile::tempdir().unwrap();
    let db =
        DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_scheduled.db")).unwrap();
    (dir, db)
}

fn qualified_close(db: &DatabaseManager, code: &str, date: &str, close: f64) {
    let mut connection = db.get_conn().unwrap();
    diesel::sql_query("INSERT INTO stock_daily (code,date,close) VALUES (?1,?2,?3)")
        .bind::<diesel::sql_types::Text, _>(code)
        .bind::<diesel::sql_types::Text, _>(date)
        .bind::<diesel::sql_types::Double, _>(close)
        .execute(&mut connection)
        .unwrap();
    diesel::sql_query(
        "INSERT INTO qualified_daily_trading_status \
         (code,date,status,contract_version,source,source_at,observed_at,batch_id) \
         VALUES (?1,?2,'trading','TEST_CODE_AUTHORITY_V1','TEST_CODE_AUTHORITY', \
                 '2026-10-09T00:00:00Z','2026-10-09T00:00:01Z','TEST_CODE_BATCH')",
    )
    .bind::<diesel::sql_types::Text, _>(code)
    .bind::<diesel::sql_types::Text, _>(date)
    .execute(&mut connection)
    .unwrap();
}

#[test]
fn scheduled_verifier_waits_for_target_session_close_even_with_a_qualified_price() {
    let (_dir, db) = private_db();
    let code = "TEST_CODE_scheduled_boundary";
    db.save_prediction_legacy(
        "2026-10-08",
        "2026-10-09",
        None,
        Some(code),
        "up",
        80.,
        None,
    )
    .unwrap();
    qualified_close(&db, code, "2026-10-08", 100.);
    qualified_close(&db, code, "2026-10-09", 102.);

    let before = "2026-10-09T14:59:59+08:00"
        .parse::<DateTime<FixedOffset>>()
        .unwrap();
    let report = verify_predictions_on_at(&db, before).unwrap();
    assert_eq!((report.pending, report.verified), (0, 0));
    assert_eq!(
        db.get_prediction_by_code_date(code, "2026-10-08")
            .unwrap()
            .hit,
        None
    );

    let at_close = "2026-10-09T15:00:00+08:00"
        .parse::<DateTime<FixedOffset>>()
        .unwrap();
    let report = verify_predictions_on_at(&db, at_close).unwrap();
    assert_eq!((report.pending, report.verified, report.hits), (1, 1, 1));
    assert_eq!(
        db.get_prediction_by_code_date(code, "2026-10-08")
            .unwrap()
            .hit,
        Some(1)
    );
}

#[test]
fn scheduled_verifier_keeps_pending_when_calendar_authority_is_unavailable() {
    let (_dir, db) = private_db();
    let code = "TEST_CODE_scheduled_calendar_gap";
    db.save_prediction_legacy(
        "2026-10-08",
        "2026-10-09",
        None,
        Some(code),
        "up",
        80.,
        None,
    )
    .unwrap();
    qualified_close(&db, code, "2026-10-08", 100.);
    qualified_close(&db, code, "2026-10-09", 102.);

    let outside_coverage = "2027-01-04T15:00:00+08:00"
        .parse::<DateTime<FixedOffset>>()
        .unwrap();
    assert!(verify_predictions_on_at(&db, outside_coverage).is_err());
    assert_eq!(
        db.get_prediction_by_code_date(code, "2026-10-08")
            .unwrap()
            .hit,
        None
    );
}

#[test]
fn scheduled_verifier_reports_database_failure_without_completing_rows() {
    let (_dir, db) = private_db();
    let mut connection = db.get_conn().unwrap();
    diesel::sql_query("DROP TABLE prediction_tracker")
        .execute(&mut connection)
        .unwrap();
    drop(connection);

    let after_close = "2026-10-09T15:00:00+08:00"
        .parse::<DateTime<FixedOffset>>()
        .unwrap();
    assert!(verify_predictions_on_at(&db, after_close).is_err());
}
