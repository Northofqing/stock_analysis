//! Real ordinary-library SQLite tests, with one singleton per isolated process.
//! Synthetic status rows exercise the verifier's reader/join/CAS only. They do
//! not constitute Gateway admission, source qualification or PIT evidence.
use super::*;
use chrono::NaiveDate;
use diesel::RunQueryDsl;
use std::process::Command;

const CHILD_ROOT: &str = "STOCK_ANALYSIS_TEST_CODE_MANUAL_BACKFILL_CHILD_ROOT";

fn isolated_case(name: &str, run: impl FnOnce(&DatabaseManager)) {
    if let Some(root) = std::env::var_os(CHILD_ROOT) {
        let root = std::path::PathBuf::from(root).canonicalize().unwrap();
        let temp = std::env::temp_dir().canonicalize().unwrap();
        assert!(root.starts_with(&temp));
        assert_ne!(root, temp);
        let path = root.join("TEST_CODE_manual_backfill.db");
        assert!(!path.exists());
        assert!(DatabaseManager::try_get().is_none());
        assert_eq!(
            stock_analysis::risk::env_guard::current_env(),
            stock_analysis::risk::env_guard::TradingEnv::Test
        );
        DatabaseManager::init(Some(path.clone())).unwrap();
        assert!(path.is_file());
        run(DatabaseManager::get());
        println!("MANUAL_BACKFILL_COMPLETED_SESSION_CASE_OK:{name}");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &format!("tests::{name}"), "--nocapture"])
        .env(CHILD_ROOT, dir.path())
        .current_dir(dir.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}\n{stderr}");
    assert!(stdout.contains("running 1 test"), "{stdout}");
    assert!(
        stdout.contains(&format!("MANUAL_BACKFILL_COMPLETED_SESSION_CASE_OK:{name}")),
        "{stdout}"
    );
}

fn now(value: &str) -> DateTime<FixedOffset> {
    value.parse().unwrap()
}

fn sample(db: &DatabaseManager, code: &str, start: &str, target: &str) {
    assert!(code.starts_with("TEST_CODE_"));
    db.save_prediction_legacy(start, target, None, Some(code), "up", 80., None)
        .unwrap();
}

fn close(db: &DatabaseManager, code: &str, date: &str, value: f64) {
    assert!(code.starts_with("TEST_CODE_"));
    db.save_daily_record(
        code,
        NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap(),
        Some(value),
        Some(value),
        Some(value),
        Some(value),
        Some(100.),
        Some(value * 100.),
        None,
        None,
        None,
        None,
        None,
        Some("TEST_CODE_BACKFILL_FIXTURE"),
    )
    .unwrap();
}

fn synthetic_status(db: &DatabaseManager, code: &str, date: &str, status: &str) {
    assert!(code.starts_with("TEST_CODE_"));
    let mut connection = db.get_conn().unwrap();
    diesel::sql_query(
        "INSERT INTO qualified_daily_trading_status \
         (code,date,status,contract_version,source,source_at,observed_at,batch_id) \
         VALUES (?1,?2,?3,'TEST_CODE_AUTHORITY_V1','TEST_CODE_BACKFILL_FIXTURE', \
                 '2026-10-09T07:00:00Z','2026-10-09T07:00:01Z','TEST_CODE_BACKFILL_BATCH')",
    )
    .bind::<diesel::sql_types::Text, _>(code)
    .bind::<diesel::sql_types::Text, _>(date)
    .bind::<diesel::sql_types::Text, _>(status)
    .execute(&mut connection)
    .unwrap();
}

fn qualified_fixture_close(db: &DatabaseManager, code: &str, date: &str, value: f64) {
    close(db, code, date, value);
    synthetic_status(db, code, date, "trading");
}

fn row_bytes(db: &DatabaseManager, code: &str, date: &str) -> Vec<u8> {
    serde_json::to_vec(&db.get_prediction_by_code_date(code, date).unwrap()).unwrap()
}

#[test]
fn manual_backfill_completed_session_before_close_after_close_and_replay() {
    isolated_case(
        "manual_backfill_completed_session_before_close_after_close_and_replay",
        |db| {
            let code = "TEST_CODE_manual_close_boundary";
            sample(db, code, "2026-10-08", "2026-10-09");
            qualified_fixture_close(db, code, "2026-10-08", 100.);
            qualified_fixture_close(db, code, "2026-10-09", 102.);
            let original = row_bytes(db, code, "2026-10-08");
            let before = run_on_at(db, now("2026-10-09T14:59:59+08:00")).unwrap();
            assert_eq!((before.pending, before.verified), (0, 0));
            assert_eq!(row_bytes(db, code, "2026-10-08"), original);

            let at_close = run_on_at(db, now("2026-10-09T15:00:00+08:00")).unwrap();
            assert_eq!(
                (at_close.pending, at_close.verified, at_close.hits),
                (1, 1, 1)
            );
            assert!(at_close.errors.is_empty());
            let row = db.get_prediction_by_code_date(code, "2026-10-08").unwrap();
            assert_eq!(row.target_date, "2026-10-09");
            assert_eq!(row.hit, Some(1));
            assert!((row.actual_change.unwrap() - 2.).abs() < 1e-12);
            let completed = row_bytes(db, code, "2026-10-08");
            let replay = run_on_at(db, now("2026-10-09T15:01:00+08:00")).unwrap();
            assert_eq!((replay.pending, replay.verified, replay.hits), (0, 0, 0));
            assert_eq!(row_bytes(db, code, "2026-10-08"), completed);
            assert_eq!(db.count_predictions().unwrap(), 1);
        },
    );
}

#[test]
fn manual_backfill_completed_session_holiday_uses_original_calendar() {
    isolated_case(
        "manual_backfill_completed_session_holiday_uses_original_calendar",
        |db| {
            let due = "TEST_CODE_manual_holiday_due";
            let future = "TEST_CODE_manual_holiday_future";
            sample(db, due, "2026-09-29", "2026-09-30");
            sample(db, future, "2026-09-30", "2026-10-08");
            qualified_fixture_close(db, due, "2026-09-29", 100.);
            qualified_fixture_close(db, due, "2026-09-30", 102.);
            qualified_fixture_close(db, future, "2026-09-30", 100.);
            qualified_fixture_close(db, future, "2026-10-08", 102.);
            let holiday = now("2026-10-07T16:00:00+08:00");
            let previous = prediction::completed_session_as_of_at(holiday).unwrap();
            assert_eq!(previous.to_string(), "2026-09-30");
            assert_eq!(
                stock_analysis::calendar::verified_next_a_share_trading_day(previous)
                    .unwrap()
                    .to_string(),
                "2026-10-08"
            );
            let future_bytes = row_bytes(db, future, "2026-09-30");
            let report = run_on_at(db, holiday).unwrap();
            assert_eq!((report.pending, report.verified, report.hits), (1, 1, 1));
            assert_eq!(
                db.get_prediction_by_code_date(due, "2026-09-29")
                    .unwrap()
                    .hit,
                Some(1)
            );
            assert_eq!(row_bytes(db, future, "2026-09-30"), future_bytes);
        },
    );
}

#[test]
fn manual_backfill_completed_session_unknown_calendar_has_zero_updates() {
    isolated_case(
        "manual_backfill_completed_session_unknown_calendar_has_zero_updates",
        |db| {
            let code = "TEST_CODE_manual_unknown_calendar";
            sample(db, code, "2026-10-08", "2026-10-09");
            qualified_fixture_close(db, code, "2026-10-08", 100.);
            qualified_fixture_close(db, code, "2026-10-09", 102.);
            let original = row_bytes(db, code, "2026-10-08");
            let error = run_on_at(db, now("2027-01-04T16:00:00+08:00")).unwrap_err();
            assert!(error.contains("coverage unavailable"), "{error}");
            assert_eq!(row_bytes(db, code, "2026-10-08"), original);
            let error = run_on_at(db, now("2026-10-09T16:00:00+00:00")).unwrap_err();
            assert!(error.contains("上海 +08:00"), "{error}");
            assert_eq!(row_bytes(db, code, "2026-10-08"), original);
            assert_eq!(db.count_predictions().unwrap(), 1);
        },
    );
}

#[test]
fn manual_backfill_completed_session_missing_or_unqualified_close_stays_pending() {
    isolated_case(
        "manual_backfill_completed_session_missing_or_unqualified_close_stays_pending",
        |db| {
            let codes = [
                "TEST_CODE_manual_missing_target",
                "TEST_CODE_manual_unknown_target",
                "TEST_CODE_manual_suspended_target",
                "TEST_CODE_manual_unknown_start",
            ];
            for (index, code) in codes.iter().enumerate() {
                sample(db, code, "2026-10-08", "2026-10-09");
                close(db, code, "2026-10-08", 100.);
                if index != 3 {
                    synthetic_status(db, code, "2026-10-08", "trading");
                }
                if index != 0 {
                    close(db, code, "2026-10-09", 102.);
                }
                if index == 2 {
                    synthetic_status(db, code, "2026-10-09", "suspended");
                } else if index == 3 {
                    synthetic_status(db, code, "2026-10-09", "trading");
                }
                qualified_fixture_close(db, code, "2026-10-12", 120.);
            }
            let original: Vec<_> = codes
                .iter()
                .map(|code| row_bytes(db, code, "2026-10-08"))
                .collect();
            let report = run_on_at(db, now("2026-10-12T15:00:00+08:00")).unwrap();
            assert_eq!(
                (
                    report.pending,
                    report.verified,
                    report.hits,
                    report.deferred
                ),
                (4, 0, 0, 4)
            );
            assert!(report.errors.is_empty());
            for (code, original) in codes.iter().zip(original) {
                assert_eq!(row_bytes(db, code, "2026-10-08"), original);
            }
        },
    );
}
