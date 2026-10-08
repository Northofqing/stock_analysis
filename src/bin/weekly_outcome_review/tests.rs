use super::*;
use report::{Period, Review};

const FIXTURE_SCHEMA: &str = r#"
CREATE TABLE prediction_tracker(id INTEGER PRIMARY KEY,pred_date TEXT,target_date TEXT,stock_code TEXT,pred_direction TEXT,actual_change_t1 REAL,actual_change_t3 REAL,actual_change_t5 REAL,hit_t1 INTEGER,hit_t3 INTEGER,hit_t5 INTEGER);
CREATE TABLE stock_daily(code TEXT,date TEXT,close REAL,is_suspended INTEGER);
CREATE TABLE qualified_daily_trading_status(code TEXT,date TEXT,status TEXT,contract_version TEXT,source TEXT,source_at TEXT,observed_at TEXT,batch_id TEXT);
CREATE TABLE paper_trades(id INTEGER PRIMARY KEY,plan_id TEXT,code TEXT,name TEXT,direction TEXT,price REAL,quantity INTEGER,status TEXT,fill_price REAL,not_fill_reason TEXT,virtual_reason TEXT,account_mode TEXT,data_mode TEXT,ts TEXT,updated_at TEXT);
CREATE TABLE order_audit(id INTEGER PRIMARY KEY,business_order_id TEXT,source TEXT,decision_basis TEXT,side TEXT,code TEXT,requested_price REAL,execution_price REAL,quantity INTEGER,quote_observed_at TEXT,outcome TEXT,failure_reason TEXT,created_at TEXT);
CREATE TABLE order_audit_chain(order_audit_id INTEGER,previous_hash TEXT,record_hash TEXT,created_at TEXT);
"#;

fn period(now: &str) -> Period {
    Period::new(
        canonical_date("2026-09-28").unwrap(),
        canonical_date("2026-10-04").unwrap(),
        shanghai_clock(now).unwrap(),
    )
    .unwrap()
}
fn snapshot(sql: &str, p: Period) -> Review {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("TEST_CODE_weekly.db");
    let writer = rusqlite::Connection::open(&path).unwrap();
    writer.execute_batch(FIXTURE_SCHEMA).unwrap();
    writer.execute_batch(sql).unwrap();
    drop(writer);
    let before = std::fs::read(&path).unwrap();
    let session =
        AttributionDatabaseSession::open(&path, AttributionDatabaseAccess::ReadOnly).unwrap();
    let review = report::read(session.database(), p);
    drop(session);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "source bytes must remain exact"
    );
    assert!(!path.with_extension("db-wal").exists());
    assert!(!path.with_extension("db-shm").exists());
    review
}
fn standard_period() -> Period {
    period("2026-10-08T00:52:00+08:00")
}

#[test]
fn weekly_holiday_and_intraday_ranges_use_actual_completed_sessions() {
    let p = standard_period();
    assert_eq!(p.latest_completed_session.to_string(), "2026-09-30");
    assert_eq!(p.period_completed_through.to_string(), "2026-09-30");
    assert_eq!(
        p.completed_sessions
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["2026-09-28", "2026-09-29", "2026-09-30"]
    );
    let p = period("2026-09-29T14:59:59+08:00");
    assert_eq!(
        p.completed_sessions
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["2026-09-28"]
    );
    let holiday = Period::new(
        canonical_date("2026-10-01").unwrap(),
        canonical_date("2026-10-07").unwrap(),
        shanghai_clock("2026-10-07T16:00:00+08:00").unwrap(),
    )
    .unwrap();
    assert!(holiday.completed_sessions.is_empty());
    assert_eq!(holiday.period_completed_through.to_string(), "2026-09-30");
    assert!(shanghai_clock("2026-10-08T16:00:00Z").is_err());
    assert!(Period::new(
        canonical_date("2026-09-28").unwrap(),
        canonical_date("2026-10-05").unwrap(),
        shanghai_clock("2026-10-08T16:00:00+08:00").unwrap()
    )
    .is_err());
}

#[test]
fn weekly_observed_only_prices_and_old_stored_returns_never_grant_qualification() {
    let review=snapshot("INSERT INTO prediction_tracker VALUES(1,'2026-09-28','2026-09-29','TEST_CODE_000001','up',5,NULL,NULL,1,NULL,NULL); INSERT INTO stock_daily VALUES('TEST_CODE_000001','2026-09-28',10,0),('TEST_CODE_000001','2026-09-29',10.5,0);",standard_period());
    let pred = review.predictions.value.as_ref().unwrap();
    let t1 = &pred.horizons[0].history_through_period;
    assert_eq!(t1.recorded_pairs, 1);
    assert_eq!(t1.revalidated_observations, 0);
    assert_eq!(t1.missing_qualification, 1);
    assert_eq!(t1.observation_hit_rate, None);
    assert_eq!(pred.reliable_prediction_samples.status, "unavailable");
    assert_eq!(pred.reliable_prediction_samples.value, None);
    assert_eq!(pred.horizons[1].history_through_period.not_mature, 1);
    assert_eq!(pred.horizons[2].history_through_period.not_mature, 1);
    assert_eq!(review.physical_delivery.status, "unavailable");
    assert!(review.markdown().contains("原 Filled 总数不能作可靠胜率"));
}

const QUALIFIED_T1: &str = "INSERT INTO prediction_tracker VALUES(1,'2026-09-28','2026-09-29','TEST_CODE_000001','up',5,NULL,NULL,1,NULL,NULL); INSERT INTO stock_daily VALUES('TEST_CODE_000001','2026-09-28',10,0),('TEST_CODE_000001','2026-09-29',10.5,0); INSERT INTO qualified_daily_trading_status VALUES('TEST_CODE_000001','2026-09-28','trading','test-authority-v2','fixture','2026-09-28T15:00:00+08:00','2026-09-30T16:00:00+08:00','b1'),('TEST_CODE_000001','2026-09-29','trading','test-authority-v2','fixture','2026-09-29T15:00:00+08:00','2026-09-30T16:00:00+08:00','b1');";

#[test]
fn weekly_later_qualified_snapshot_is_descriptive_and_never_historical_pit() {
    let review = snapshot(QUALIFIED_T1, standard_period());
    let pred = review.predictions.value.as_ref().unwrap();
    let c = &pred.horizons[0].maturing_this_week;
    assert_eq!(c.revalidated_observations, 1);
    assert_eq!(c.observation_hit_rate, Some(1.));
    assert!((c.observation_mean_change_pct.unwrap() - 5.).abs() < 1e-9);
    assert_eq!(pred.reliable_prediction_samples.value, None);
    assert!(review.markdown().contains("不能授予历史可用时刻/PIT"));
    let markdown = review.markdown();
    assert!(markdown.contains("日线表原始行"));
    assert!(markdown.contains("状态表原始行（未核验资格）"));
    assert!(markdown.contains("不等于独立资格"));
    assert!(
        markdown.find("| T+5 / 截至本期历史").unwrap()
            < markdown.find("T+1 本周成熟描述性观察").unwrap(),
        "all horizon table rows precede descriptive paragraphs"
    );
}

#[test]
fn weekly_missing_middle_status_suspension_bad_result_and_future_evidence_are_distinct() {
    let mut sql = QUALIFIED_T1.to_owned();
    sql.push_str("UPDATE prediction_tracker SET actual_change_t1=6; INSERT INTO prediction_tracker VALUES(2,'2026-09-23','2026-09-29','TEST_CODE_000002','up',NULL,5,NULL,NULL,1,NULL); INSERT INTO stock_daily VALUES('TEST_CODE_000002','2026-09-23',10,0),('TEST_CODE_000002','2026-09-29',10.5,0); INSERT INTO qualified_daily_trading_status VALUES('TEST_CODE_000002','2026-09-23','trading','test-authority-v2','fixture','2026-09-23T15:00:00+08:00','2026-09-30T16:00:00+08:00','b2'),('TEST_CODE_000002','2026-09-29','trading','test-authority-v2','fixture','2026-09-29T15:00:00+08:00','2026-09-30T16:00:00+08:00','b2');");
    let review = snapshot(&sql, standard_period());
    let pred = review.predictions.value.unwrap();
    assert_eq!(pred.windows[0].status, "invalid");
    let middle = pred
        .windows
        .iter()
        .find(|w| w.prediction_row_id == 2 && w.trading_days == 3)
        .unwrap();
    assert_eq!(middle.status, "missing_qualification");
    assert!(middle.gaps.iter().any(|v| v.contains("2026-09-24")));
    let review=snapshot(&(QUALIFIED_T1.to_owned()+"UPDATE qualified_daily_trading_status SET status='suspended' WHERE date='2026-09-29';"),standard_period());
    assert_eq!(
        review.predictions.value.unwrap().windows[0].status,
        "suspended"
    );
    let review = snapshot(
        &(QUALIFIED_T1.to_owned()
            + "UPDATE qualified_daily_trading_status SET observed_at='2026-10-09T00:00:00+08:00';"),
        standard_period(),
    );
    assert_eq!(
        review.predictions.value.unwrap().windows[0].status,
        "missing_qualification"
    );
}

#[test]
fn weekly_ready_but_unrecorded_stays_null_and_invalid_original_dates_are_retained() {
    let review=snapshot(&(QUALIFIED_T1.to_owned()+"UPDATE prediction_tracker SET actual_change_t1=NULL,hit_t1=NULL; INSERT INTO prediction_tracker VALUES(2,'2026-10-01','2026-10-08','TEST_CODE_000002','up',NULL,NULL,NULL,NULL,NULL,NULL);"),standard_period());
    let pred = review.predictions.value.unwrap();
    assert_eq!(pred.windows[0].status, "ready_unrecorded");
    assert_eq!(pred.windows[0].change_pct, None);
    assert_eq!(pred.windows[0].hit, None);
    assert_eq!(
        pred.windows
            .iter()
            .filter(|w| w.prediction_row_id == 2 && w.status == "invalid")
            .count(),
        3
    );
}

#[test]
fn weekly_invalid_target_direction_or_id_remain_in_original_week() {
    let review = snapshot(
        "INSERT INTO prediction_tracker VALUES(1,'2026-09-28','bad-target','TEST_CODE_000001','up',NULL,NULL,NULL,NULL,NULL,NULL); INSERT INTO prediction_tracker VALUES(2,'2026-09-29','2026-09-30','TEST_CODE_000002','bad-direction',NULL,NULL,NULL,NULL,NULL,NULL); INSERT INTO prediction_tracker VALUES(0,'2026-09-30','2026-09-30','TEST_CODE_000003','up',NULL,NULL,NULL,NULL,NULL,NULL);",
        standard_period(),
    );
    let pred = review.predictions.value.unwrap();
    for horizon in &pred.horizons {
        assert_eq!(horizon.weekly_origins.rows, 3);
        assert_eq!(horizon.weekly_origins.invalid, 3);
        assert_eq!(horizon.history_through_period.invalid, 3);
    }
    assert!(pred
        .windows
        .iter()
        .all(|w| w.weekly_origin && w.status == "invalid"));
    let bad_target = pred
        .windows
        .iter()
        .find(|w| w.prediction_row_id == 1)
        .unwrap();
    assert_eq!(bad_target.original_pred_date, "2026-09-28");
    assert_eq!(bad_target.original_target_date, "bad-target");
}

#[test]
fn weekly_old_schema_is_reported_without_initialization_or_ddl() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("TEST_CODE_old.db");
    let writer = rusqlite::Connection::open(&path).unwrap();
    writer.execute_batch("CREATE TABLE prediction_tracker(id INTEGER PRIMARY KEY,pred_date TEXT,target_date TEXT,stock_code TEXT,pred_direction TEXT); INSERT INTO prediction_tracker VALUES(1,'2026-09-28','2026-09-29','TEST_CODE_000001','up');").unwrap();
    drop(writer);
    let before = std::fs::read(&path).unwrap();
    let session =
        AttributionDatabaseSession::open(&path, AttributionDatabaseAccess::ReadOnly).unwrap();
    let review = report::read(session.database(), standard_period());
    let pred = review.predictions.value.unwrap();
    assert_eq!(pred.schema_gaps.len(), 6);
    assert_eq!(pred.windows[0].status, "missing_qualification");
    assert_eq!(review.daily_bars.status, "unavailable");
    assert_eq!(review.raw_paper.status, "unavailable");
    drop(session);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let missing = dir.path().join("TEST_CODE_missing.db");
    assert!(
        AttributionDatabaseSession::open(&missing, AttributionDatabaseAccess::ReadOnly).is_err()
    );
    assert!(!missing.exists());
}

const BUY: &str = "INSERT INTO paper_trades VALUES(1,'test-buy','TEST_CODE_000001','fixture','buy',10,100,'Filled',10,NULL,'NewsCatalyst','Normal','Full','2026-09-28 02:00:00','2026-09-28 02:00:00');";
const SELL: &str = "INSERT INTO paper_trades VALUES(2,'test-sell','TEST_CODE_000001','fixture','sell',11,100,'Filled',11,NULL,'StopLoss','ReduceOnly','Full','2026-09-29 02:00:00','2026-09-29 02:00:00');";

#[test]
fn weekly_paper_oversell_blocks_reliable_net_metrics_but_preserves_raw_counts() {
    let sql = BUY.to_owned() + &SELL.replace("11,100,'Filled'", "11,200,'Filled'");
    let review = snapshot(&sql, standard_period());
    assert_eq!(
        review
            .raw_paper
            .value
            .as_ref()
            .unwrap()
            .historical_filled_rows,
        2
    );
    assert_eq!(review.verified_paper.status, "unavailable");
    assert!(review
        .verified_paper
        .reason
        .as_ref()
        .unwrap()
        .contains("cumulative_oversell"));
    assert!(review.verified_paper.value.is_none());
    assert!(review
        .markdown()
        .contains("可靠 paper 样本、费用和净收益保持不可用"));
}

#[test]
fn weekly_verified_paper_reports_existing_scenario_costs_and_closure_not_actual_fees() {
    let review = snapshot(&(BUY.to_owned() + SELL), standard_period());
    assert_eq!(
        review.verified_paper.status, "available",
        "{:?}",
        review.verified_paper.reason
    );
    let paper = review
        .verified_paper
        .value
        .as_ref()
        .expect("valid legacy effective projection");
    assert_eq!(paper.closed_cycles_in_week, 1);
    assert_eq!(paper.period_fill_rows, 2);
    assert_eq!(paper.legacy_without_terminal_rows, 2);
    assert_eq!(paper.exits[0].exit_reasons, ["StopLoss"]);
    // Existing PaperLedger fee model: minimum commission 5 on each side,
    // plus sell stamp tax 1100 * .001 = 1.10. Gross gain is 100.
    assert!((paper.closed_cycle_scenario_cost_cny.unwrap() - 11.10).abs() < 1e-9);
    assert!((paper.closed_cycle_scenario_net_pnl_cny.unwrap() - 88.90).abs() < 1e-9);
    assert_eq!(paper.actual_settlement_costs.value, None);
    assert!(review.markdown().contains("情景净盈亏 88.90 元"));
    assert_eq!(paper.executable_net_return.value, None);
}

#[test]
fn weekly_empty_paper_does_not_render_zero_return_or_zero_settlement_costs() {
    let review = snapshot("", standard_period());
    let paper = review.verified_paper.value.as_ref().unwrap();
    assert_eq!(paper.closed_cycles_in_week, 0);
    assert_eq!(paper.period_scenario_fill_cost_cny, None);
    assert_eq!(paper.closed_cycle_scenario_net_pnl_cny, None);
    assert_eq!(paper.actual_settlement_costs.value, None);
}

#[test]
fn weekly_nonfill_terminal_time_and_bad_timestamp_do_not_invent_dates() {
    let sql="INSERT INTO paper_trades VALUES(1,'blocked','TEST_CODE_000001','fixture','sell',10,100,'NotFilled',NULL,'T+1 locked','StopLoss','ReduceOnly','Full','2026-09-25 02:00:00','2026-09-28 02:00:00'); INSERT INTO paper_trades VALUES(2,'bad-time','TEST_CODE_000002','fixture','buy',10,100,'Filled',10,NULL,'CandidateBuy','Normal','Full','2026-09-28','2026-09-28'); INSERT INTO order_audit VALUES(1,'blocked','PaperTrade','StopLoss','sell','TEST_CODE_000001',10,NULL,100,NULL,'Rejected','T+1 locked','2026-09-28 02:00:00');";
    let review = snapshot(sql, standard_period());
    let raw = review.raw_paper.value.unwrap();
    assert_eq!(raw.weekly_not_filled_reasons.get("T+1 locked"), Some(&1));
    assert_eq!(
        raw.weekly_sell_rows[0].terminal_at_shanghai.to_string(),
        "2026-09-28 10:00:00"
    );
    assert_eq!(raw.malformed_timestamp_rows, 1);
    assert_eq!(
        review
            .original_order_attempts
            .value
            .unwrap()
            .failure_reasons
            .get("T+1 locked"),
        Some(&1)
    );
    assert_eq!(review.verified_paper.status, "unavailable");
}

#[test]
fn weekly_same_day_future_paper_and_audit_are_diagnostic_and_block_verified_metrics() {
    let future_fills = BUY.to_owned() + &SELL.replace("2026-09-29 02:00:00", "2026-09-29 10:00:00");
    let sql = future_fills.clone()
        + "INSERT INTO paper_trades VALUES(3,'future-rejection','TEST_CODE_000002','fixture','sell',10,100,'NotFilled',NULL,'future denial','StopLoss','ReduceOnly','Full','2026-09-28 02:00:00','2026-09-29 10:01:00'); INSERT INTO order_audit VALUES(1,'past','PaperTrade','StopLoss','sell','TEST_CODE_000001',10,NULL,100,NULL,'Rejected','past denial','2026-09-29 07:59:00'); INSERT INTO order_audit VALUES(2,'future','PaperTrade','StopLoss','sell','TEST_CODE_000001',10,NULL,100,NULL,'Rejected','future denial','2026-09-29 10:00:00');";
    let review = snapshot(&sql, period("2026-09-29T16:00:00+08:00"));
    let raw = review.raw_paper.value.as_ref().unwrap();
    assert_eq!(raw.weekly_states.get("Filled"), Some(&1));
    assert!(!raw.weekly_states.contains_key("NotFilled"));
    assert!(raw.weekly_not_filled_reasons.is_empty());
    assert!(raw.weekly_sell_rows.is_empty());
    // Whole-snapshot diagnostics retain originals, including future rows.
    assert_eq!(raw.historical_filled_rows, 2);
    assert_eq!(
        raw.latest_filled_utc.as_deref(),
        Some("2026-09-29 10:00:00")
    );
    let attempts = review.original_order_attempts.value.as_ref().unwrap();
    assert_eq!(
        attempts
            .source_side_outcomes
            .get("PaperTrade / sell / Rejected"),
        Some(&1)
    );
    assert_eq!(attempts.failure_reasons.get("past denial"), Some(&1));
    assert!(!attempts.failure_reasons.contains_key("future denial"));
    let json = serde_json::to_value(&review).unwrap();
    assert_eq!(json["raw_paper"]["value"]["future_timestamp_rows"], 2);
    assert_eq!(
        json["raw_paper"]["value"]["future_timestamps"][0]["row_id"],
        2
    );
    assert_eq!(
        json["original_order_attempts"]["value"]["future_timestamp_rows"],
        1
    );
    assert_eq!(
        json["original_order_attempts"]["value"]["future_timestamps"][0]["row_id"],
        2
    );
    // Raw audit diagnostics do not fabricate a valid audit chain. Check the
    // future effective boundary separately with a valid LegacyNoTerminal set.
    let paper_review = snapshot(&future_fills, period("2026-09-29T16:00:00+08:00"));
    assert_eq!(paper_review.verified_paper.status, "unavailable");
    assert!(
        paper_review.verified_paper.value.is_none(),
        "no cost/net metrics from a future effective fact"
    );
    assert!(
        paper_review
            .verified_paper
            .reason
            .as_deref()
            .unwrap()
            .contains("future_effective_fill"),
        "{:?}",
        paper_review.verified_paper.reason
    );
    let markdown = review.markdown();
    assert!(markdown.contains("非截至观察时刻"));
    assert!(markdown.contains("未来时间原行：2"));
    assert!(markdown.contains("未来时间原行：1"));
    assert!(paper_review
        .markdown()
        .contains("可靠 paper 样本、费用和净收益保持不可用"));
}

#[test]
fn weekly_cli_requires_explicit_scope_and_refuses_output_overwrite() {
    assert!(Args::try_parse_from([
        "weekly",
        "--database",
        "source.db",
        "--from",
        "2026-09-28",
        "--to",
        "2026-10-04",
        "--format",
        "json"
    ])
    .is_ok());
    assert!(Args::try_parse_from(["weekly", "--database", "source.db"]).is_err());
    assert!(Args::try_parse_from([
        "weekly",
        "--database",
        "source.db",
        "--from",
        "2026-9-28",
        "--to",
        "2026-10-04"
    ])
    .is_err());
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("TEST_CODE_output.db");
    let writer = rusqlite::Connection::open(&source).unwrap();
    writer.execute_batch(FIXTURE_SCHEMA).unwrap();
    drop(writer);
    let output = dir.path().join("review.json");
    std::fs::write(&output, "existing").unwrap();
    let args = Args {
        database: source,
        source_label: None,
        temporary_snapshot: false,
        from: canonical_date("2026-09-28").unwrap(),
        to: canonical_date("2026-10-04").unwrap(),
        observed_at: Some(shanghai_clock("2026-10-08T00:52:00+08:00").unwrap()),
        format: Format::Json,
        output: Some(output.clone()),
    };
    assert!(run(args).is_err());
    assert_eq!(std::fs::read_to_string(output).unwrap(), "existing");
}

#[test]
fn weekly_empty_week_retains_all_76_historical_pending_windows() {
    let sql=(1..=76).map(|id|format!("INSERT INTO prediction_tracker VALUES({id},'2026-09-11','2026-09-18','TEST_CODE_000001','up',NULL,NULL,NULL,NULL,NULL,NULL);"))
        .collect::<String>();
    let review = snapshot(&sql, standard_period());
    let pred = review.predictions.value.unwrap();
    assert_eq!(pred.original_rows, 76);
    for horizon in pred.horizons {
        assert_eq!(horizon.weekly_origins.rows, 0);
        assert_eq!(horizon.maturing_this_week.rows, 0);
        assert_eq!(horizon.history_through_period.pending_pairs, 76);
        assert_eq!(horizon.history_through_period.missing_qualification, 76);
        assert_eq!(horizon.history_through_period.revalidated_observations, 0);
        assert_eq!(horizon.history_through_period.not_mature, 0);
        assert_eq!(
            horizon.history_through_period.observation_mean_change_pct,
            None
        );
    }
}

#[test]
fn weekly_cli_creates_machine_report_with_source_identity_and_unavailable_states() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("TEST_CODE_json.db");
    let writer = rusqlite::Connection::open(&source).unwrap();
    writer.execute_batch(FIXTURE_SCHEMA).unwrap();
    drop(writer);
    let output = dir.path().join("review.json");
    let before = std::fs::read(&source).unwrap();
    run(Args {
        database: source.clone(),
        source_label: Some("original input label".into()),
        temporary_snapshot: true,
        from: canonical_date("2026-09-28").unwrap(),
        to: canonical_date("2026-10-04").unwrap(),
        observed_at: Some(shanghai_clock("2026-10-08T00:52:00+08:00").unwrap()),
        format: Format::Json,
        output: Some(output.clone()),
    })
    .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(output).unwrap()).unwrap();
    assert_eq!(
        json["input_source"]["source_main_sha256"],
        file_sha256(&source).unwrap()
    );
    assert_eq!(
        json["input_source"]["original_source_label"],
        "original input label"
    );
    assert_eq!(
        json["input_source"]["temporary_snapshot_deleted_after_run"],
        true
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(dir.path().join("review.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    assert_eq!(json["physical_delivery"]["status"], "unavailable");
    assert!(json["source_extent_boundary"]
        .as_str()
        .unwrap()
        .contains("do not establish independent qualification"));
    assert!(json["source_extent_boundary"]
        .as_str()
        .unwrap()
        .contains("kline-inferred status and OHLC"));
    assert_eq!(
        json["predictions"]["value"]["reliable_prediction_samples"]["value"],
        serde_json::Value::Null
    );
    assert_eq!(std::fs::read(source).unwrap(), before);
}
