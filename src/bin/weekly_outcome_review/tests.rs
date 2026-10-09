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
        registry: None,
        evidence_manifest: None,
        database: source,
        snapshot_source_manifest: None,
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
        registry: None,
        evidence_manifest: None,
        database: source.clone(),
        snapshot_source_manifest: None,
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

fn scorecard_json(mut review: Review) -> serde_json::Value {
    scorecard::attach(
        &mut review,
        registry::RegistryInput::load(None).unwrap(),
        &"a".repeat(64),
    );
    serde_json::to_value(review).unwrap()
}

#[test]
fn registry_rejects_duplicate_missing_unknown_and_invalid_contracts() {
    let original = registry::DEFAULT_REGISTRY;
    assert_eq!(registry::Registry::parse(original).unwrap().signal.len(), 6);
    for bad in [
        original.replace("id = \"main_net_inflow\"", "id = \"news_catalyst\""),
        original.replace("name = \"MainNetInflow\"", "name = \"NewsCatalyst\""),
        original.replace("name = \"MainNetInflow\"", "name = \"Unknown\""),
        original.replace("action = \"info_only\"", "action = \"promote_live\""),
        original.replace("status = \"watch\"", "status = \"live\""),
        original.replace(
            "observation_window = [\"t1\", \"t3\", \"t5\"]",
            "observation_window = [\"t0\", \"t3\", \"t5\"]",
        ),
        original.replace(
            "observation_window = [\"t1\", \"t3\", \"t5\"]",
            "observation_window = [\"t1\", \"t1\"]",
        ),
        original.replace(
            "observation_window = [\"t1\", \"t3\", \"t5\"]",
            "observation_window = []",
        ),
        original.replace(
            "cost_model_version = \"lot-rates-v1\"",
            "cost_model_version = \"\"",
        ),
        original.replace(
            "exit_rule_version = \"existing-paper-exits-review-v0\"\n",
            "",
        ),
        original.replace("signal_version = \"news-catalyst-review-v0\"\n", ""),
        original.replace("schema_version = 1", "schema_version = 2"),
    ] {
        assert!(
            registry::Registry::parse(&bad).is_err(),
            "accepted invalid registry: {bad}"
        );
    }
}

#[test]
fn scorecard_qualification_and_family_join_fail_closed_without_free_text_attribution() {
    for sql in [
        QUALIFIED_T1.to_owned() + BUY,
        QUALIFIED_T1.to_owned() + "INSERT INTO qualified_daily_trading_status SELECT * FROM qualified_daily_trading_status;",
        QUALIFIED_T1.replace("test-authority-v2", stock_analysis::data_gateway::qualified_trading_facts::QUALIFIED_TRADING_FACTS_CONTRACT_V1),
    ] {
        let json = scorecard_json(snapshot(&sql, standard_period()));
        let pooled = &json["scorecard"]["pooled_descriptive_evidence"]["price_observation"];
        assert_ne!(pooled[0]["grade"], "reliable");
        for card in json["scorecard"]["families"].as_array().unwrap() {
            for section in ["price_observation", "simulated_fill", "net_return"] {
                assert_eq!(card["sections"][section][0]["grade"], "unavailable");
                assert!(card["sections"][section][0]["value"].is_null());
                assert!(card["sections"][section][0]["reason"].as_str().unwrap().contains("missing authoritative"));
            }
        }
        assert!(json["physical_delivery"]["value"].is_null());
    }
}

#[test]
fn scorecard_distinguishes_usable_zero_counts_from_unavailable_and_excludes_future() {
    let empty = scorecard_json(snapshot("", standard_period()));
    let pooled = &empty["scorecard"]["pooled_descriptive_evidence"];
    assert_eq!(pooled["price_observation"][0]["value"], 0);
    assert_eq!(pooled["price_observation"][0]["grade"], "observational");
    assert_eq!(pooled["simulated_fill"][0]["value"], 0);
    assert!(pooled["price_observation"][1]["value"].is_null());
    assert!(pooled["net_return"][0]["value"].is_null());
    assert!(pooled["net_return"][0]["reason"]
        .as_str()
        .unwrap()
        .contains("no closed cycles"));
    let future = scorecard_json(snapshot(
        &(QUALIFIED_T1.to_owned()+"UPDATE qualified_daily_trading_status SET observed_at='2026-10-09T20:00:00+08:00'; INSERT INTO prediction_tracker VALUES(2,'2026-10-09','2026-10-12','TEST_CODE_future','up',100,NULL,NULL,1,NULL,NULL);"), standard_period()));
    let pred = &future["predictions"]["value"];
    assert_eq!(pred["windows"].as_array().unwrap().len(), 3);
    assert_eq!(
        future["scorecard"]["pooled_descriptive_evidence"]["price_observation"][0]["value"],
        0
    );
    assert!(
        future["scorecard"]["pooled_descriptive_evidence"]["price_observation"][1]["value"]
            .is_null()
    );
}

#[test]
fn owner_disputed_summary_blocks_aggregate_amounts_even_with_unrelated_weekly_cycle() {
    use stock_analysis::performance::economic_position::NetSummary;
    let mut review = snapshot(&(BUY.to_owned() + SELL), standard_period());
    let paper = review.verified_paper.value.as_mut().unwrap();
    // Typed owner guard represents an unresolved open/history dispute: an
    // unrelated closed cycle can retain its amount, never certify the aggregate.
    let summary = NetSummary::Unavailable { reason: "original legacy price dispute is unresolved; complete dependent lifecycle net/account amounts are unavailable".into() };
    let originals = serde_json::to_value(&paper.exits).unwrap();
    let (cost, net, reason) = report::closed_scenario_amounts(&paper.exits, &summary);
    assert!(cost.is_none() && net.is_none());
    assert_eq!(serde_json::to_value(&paper.exits).unwrap(), originals);
    paper.closed_cycle_scenario_cost_cny = cost;
    paper.closed_cycle_scenario_net_pnl_cny = net;
    paper.scenario_amount_unavailable_reason = reason;
    let json = scorecard_json(review);
    let metric = &json["scorecard"]["pooled_descriptive_evidence"]["net_return"][1];
    assert!(metric["value"].is_null());
    assert_eq!(metric["grade"], "unavailable");
    assert!(metric["reason"].as_str().unwrap().contains("price dispute"));
}

#[test]
fn cli_formats_and_manifest_share_exact_snapshot_clock_registry_and_reader_identity() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("TEST_CODE_identity.db");
    let writer = rusqlite::Connection::open(&source).unwrap();
    writer.execute_batch(FIXTURE_SCHEMA).unwrap();
    writer.execute_batch(QUALIFIED_T1).unwrap();
    drop(writer);
    let original = std::fs::read(&source).unwrap();
    let registry_path = dir.path().join("registry.toml");
    std::fs::write(
        &registry_path,
        registry::DEFAULT_REGISTRY
            .replace("action = \"info_only\"", "action = \"disabled\"")
            .replace("status = \"watch\"", "status = \"active\""),
    )
    .unwrap();
    let manifest_path = dir.path().join("evidence-manifest.json");
    for (format, name) in [
        (Format::Json, "review.json"),
        (Format::Markdown, "review.md"),
    ] {
        run(Args {
            database: source.clone(),
            registry: Some(registry_path.clone()),
            evidence_manifest: Some(manifest_path.clone()),
            snapshot_source_manifest: None,
            source_label: None,
            temporary_snapshot: false,
            from: canonical_date("2026-09-28").unwrap(),
            to: canonical_date("2026-10-04").unwrap(),
            observed_at: Some(shanghai_clock("2026-10-08T00:52:00+08:00").unwrap()),
            format,
            output: Some(dir.path().join(name)),
        })
        .unwrap();
    }
    let json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("review.json")).unwrap()).unwrap();
    let manifest_bytes = std::fs::read(&manifest_path).unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
    let markdown = std::fs::read_to_string(dir.path().join("review.md")).unwrap();
    assert_eq!(json["evidence_manifest"], manifest);
    assert_eq!(json["period"], manifest["period"]);
    assert_eq!(manifest["input_snapshot_sha256"], bytes_sha256(&original));
    assert_eq!(
        manifest["registry"]["sha256"],
        file_sha256(&registry_path).unwrap()
    );
    assert!(markdown.contains(std::str::from_utf8(&manifest_bytes).unwrap().trim_end()));
    assert_eq!(
        json["scorecard"]["pooled_descriptive_evidence"]["price_observation"][0]["value"], 1,
        "manual disabled action does not change reader observations"
    );
    for evidence in manifest["metric_scopes"].as_object().unwrap().values() {
        assert_eq!(evidence["input_snapshot_sha256"], bytes_sha256(&original));
        for field in ["reader_id", "sample_scope", "exclusions", "meaning"] {
            assert!(!evidence[field].as_str().unwrap().is_empty());
        }
    }
    assert_eq!(std::fs::read(source).unwrap(), original);
    // A changed clock cannot reuse the old sidecar and writes no new report.
    let output = dir.path().join("mismatched.json");
    assert!(run(Args {
        database: dir.path().join("TEST_CODE_identity.db"),
        registry: Some(registry_path),
        evidence_manifest: Some(manifest_path),
        snapshot_source_manifest: None,
        source_label: None,
        temporary_snapshot: false,
        from: canonical_date("2026-09-28").unwrap(),
        to: canonical_date("2026-10-04").unwrap(),
        observed_at: Some(shanghai_clock("2026-10-08T00:53:00+08:00").unwrap()),
        format: Format::Json,
        output: Some(output.clone())
    })
    .is_err());
    assert!(!output.exists());
}

#[test]
fn cli_invalid_registry_and_nonempty_wal_fail_without_report_or_source_writes() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("TEST_CODE_reject.db");
    let writer = rusqlite::Connection::open(&source).unwrap();
    writer.execute_batch(FIXTURE_SCHEMA).unwrap();
    drop(writer);
    let original = std::fs::read(&source).unwrap();
    let invalid = dir.path().join("invalid.toml");
    std::fs::write(&invalid, "schema_version = 9").unwrap();
    let output = dir.path().join("rejected.json");
    let args = |registry| Args {
        database: source.clone(),
        registry,
        evidence_manifest: None,
        snapshot_source_manifest: None,
        source_label: None,
        temporary_snapshot: false,
        from: canonical_date("2026-09-28").unwrap(),
        to: canonical_date("2026-10-04").unwrap(),
        observed_at: Some(shanghai_clock("2026-10-08T00:52:00+08:00").unwrap()),
        format: Format::Json,
        output: Some(output.clone()),
    };
    assert!(run(args(Some(invalid))).is_err());
    std::fs::write(
        format!("{}-wal", source.display()),
        b"uncheckpointed frames",
    )
    .unwrap();
    assert!(run(args(None))
        .unwrap_err()
        .to_string()
        .contains("nonempty WAL"));
    assert!(!output.exists());
    assert_eq!(std::fs::read(source).unwrap(), original);
}

#[test]
fn binding_plan_shaped_registry_accepts_prescribed_enums_and_independent_eligibility() {
    let families = [
        "NewsCatalyst",
        "MainNetInflow",
        "VolumeSurge",
        "PostCloseFundInflow",
        "StreakLeader",
        "ThemePrediction",
    ];
    let actions = ["paper_buy", "info_only", "disabled"];
    let statuses = ["active", "watch", "demoted"];
    // Exactly the §3.1 minimum per-family shape: no optional ID or envelope.
    let input = families
        .iter()
        .enumerate()
        .map(|(i, name)| {
            format!(
                r#"
[[signal]]
name = "{name}"
signal_version = "v1"
exit_rule_version = "BR-234.v1"
cost_model_version = "cost.v1"
source_module = "monitor.news"
action = "{}"
eligibility_price_observation = "close_available"
eligibility_simulated_fill = "paper_fill_valid"
eligibility_net_return = "cost_model_complete"
entry_assumption = "close_t0"
observation_window = ["t1", "t3", "t5"]
status = "{}"
"#,
                actions[i % 3],
                statuses[i % 3]
            )
        })
        .collect::<String>();
    let parsed = registry::Registry::parse(&input).unwrap();
    assert_eq!(parsed.schema_version, 1);
    assert_eq!(parsed.registry_version, "signal-registry-v0");
    assert_eq!(parsed.signal[0].id, "news_catalyst");
    assert_eq!(parsed.signal[5].id, "theme_prediction");
    let encoded = serde_json::to_value(parsed).unwrap();
    for (index, signal) in encoded["signal"].as_array().unwrap().iter().enumerate() {
        assert_eq!(signal["action"], actions[index % 3]);
        assert_eq!(signal["status"], statuses[index % 3]);
        assert_eq!(signal["eligibility_price_observation"], "close_available");
        assert_eq!(signal["eligibility_simulated_fill"], "paper_fill_valid");
        assert_eq!(signal["eligibility_net_return"], "cost_model_complete");
        assert_eq!(
            signal["observation_window"],
            serde_json::json!(["t1", "t3", "t5"])
        );
        assert!(signal.get("eligibility").is_none());
    }
    assert!(registry::Registry::parse(&(input + "\n")).is_ok());
}

#[test]
fn cli_rejects_relative_parent_and_symlink_directory_artifact_aliases_before_writing() {
    // Avoid process-global cwd mutations while testing relative versus absolute.
    let cwd = std::env::current_dir().unwrap().canonicalize().unwrap();
    let dir = tempfile::tempdir_in(&cwd).unwrap();
    let root = dir.path().canonicalize().unwrap();
    let source = root.join("TEST_CODE_alias.db");
    let writer = rusqlite::Connection::open(&source).unwrap();
    writer.execute_batch(FIXTURE_SCHEMA).unwrap();
    drop(writer);
    let original = std::fs::read(&source).unwrap();
    let output = root.join("review.json");
    let relative = output.strip_prefix(&cwd).unwrap().to_path_buf();
    std::fs::create_dir(root.join("child")).unwrap();
    let mut aliases = vec![relative, root.join("child/../review.json")];
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&root, root.join("linked")).unwrap();
        aliases.push(root.join("linked/review.json"));
    }
    for alias in aliases {
        let error = run(Args {
            database: source.clone(),
            registry: None,
            evidence_manifest: Some(output.clone()),
            snapshot_source_manifest: None,
            source_label: None,
            temporary_snapshot: false,
            from: canonical_date("2026-09-28").unwrap(),
            to: canonical_date("2026-10-04").unwrap(),
            observed_at: Some(shanghai_clock("2026-10-08T00:52:00+08:00").unwrap()),
            format: Format::Json,
            output: Some(alias.clone()),
        })
        .unwrap_err();
        assert!(
            error.to_string().contains("paths must differ"),
            "unexpected alias failure for {alias:?}: {error}"
        );
        assert!(
            !alias.exists() && !output.exists(),
            "alias rejection wrote an artifact"
        );
        assert_eq!(std::fs::read(&source).unwrap(), original);
    }
}

#[test]
fn newly_appeared_or_changed_manifest_is_never_silently_reused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("manifest.json");
    let expected = b"{\"schema_version\":\"weekly-outcome-evidence-manifest-v1\"}\n";
    // This represents a path appearing after the initial absent-path preflight.
    std::fs::write(&path, b"unvalidated appeared file").unwrap();
    assert!(persist_manifest(&path, expected, false).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"unvalidated appeared file");
    // Pre-existing byte-identical manifests can be reused, but must still match
    // at completion. A change after preflight cannot yield successful reuse.
    std::fs::write(&path, expected).unwrap();
    persist_manifest(&path, expected, true).unwrap();
    std::fs::write(&path, b"changed after validation").unwrap();
    assert!(persist_manifest(&path, expected, true).is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"changed after validation");
}

/// The actual test subprocess owns its environment, so parallel old-report
/// tests never inherit an active binding. This runs the real CLI run() boundary.
#[test]
fn weekly_native_process_fixture_entry() {
    let Some(request_path) = std::env::var_os("TEST_CODE_WEEKLY_CHILD_REQUEST") else {
        return;
    };
    let request: serde_json::Value =
        serde_json::from_slice(&std::fs::read(request_path).unwrap()).unwrap();
    run(Args {
        database: PathBuf::from(request["database"].as_str().unwrap()),
        registry: None,
        snapshot_source_manifest: request["manifest"].as_str().map(PathBuf::from),
        evidence_manifest: None,
        source_label: Some("TEST_CODE_original_target_provenance".into()),
        temporary_snapshot: true,
        from: canonical_date("2026-10-05").unwrap(),
        to: canonical_date("2026-10-11").unwrap(),
        observed_at: Some(shanghai_clock(request["observed_at"].as_str().unwrap()).unwrap()),
        format: Format::Json,
        output: Some(PathBuf::from(request["output"].as_str().unwrap())),
    })
    .unwrap();
}

struct NativeWeeklyFixture {
    _dir: tempfile::TempDir,
    original: PathBuf,
    snapshot: PathBuf,
    binding: stock_analysis::trading::paper_ledger::AccountBinding,
}
fn native_weekly_fixture() -> NativeWeeklyFixture {
    // Use an explicit reason supported by the original legacy codec. This
    // fixture tests a valid independent old scope, not missing strategy facts.
    native_weekly_fixture_with_legacy_reason("NewsCatalyst")
}
fn native_weekly_fixture_with_legacy_reason(legacy_reason: &str) -> NativeWeeklyFixture {
    use serde_json::json;
    use std::os::unix::fs::MetadataExt;
    use stock_analysis::trading::paper_ledger::{Mark, Money, RiskPolicyV1, SeedLot, SeedManifest};
    use stock_analysis::trading::paper_snapshot_activation::*;
    let dir = tempfile::tempdir().unwrap();
    let directory = dir.path().canonicalize().unwrap();
    let original = directory.join("TEST_CODE_native_weekly_original.db");
    let writer = rusqlite::Connection::open(&original).unwrap();
    // Frozen native DDL, without running DatabaseManager init/catalog migration.
    for line in include_str!("../../database/fixtures/global_schema_legacy_ddl_v1.tsv")
        .lines()
        .filter(|l| !l.starts_with('#'))
    {
        let (_, hex) = line.split_once('|').unwrap();
        writer
            .execute_batch(&String::from_utf8(hex::decode(hex).unwrap()).unwrap())
            .unwrap();
    }
    writer.execute("INSERT INTO paper_trades(plan_id,code,name,direction,price,quantity,status,fill_price,virtual_reason,account_mode,data_mode,ts) VALUES('TEST_CODE_old_trade','TEST_CODE_000001','TEST_CODE_fixture','buy',9,100,'Filled',9,?1,'Normal','Full','2026-10-08 02:00:00')", [legacy_reason]).unwrap();
    writer.execute_batch("INSERT INTO user_account_summary(effective_at,total_assets,securities_market_value,available_cash,position_ratio_pct,daily_pnl,source) VALUES('2026-10-09T21:13:00+08:00',11000,1000,10000,9.090909,-100,'TEST_CODE_user_confirmed_account');").unwrap();
    let input=stock_analysis::portfolio::user_position_snapshot::user_position_snapshot_input_from_json(
        r#"{"schema_version":1,"effective_at":"2026-10-09T21:13:00+08:00","confirm_empty":false,"items":[{"code":"TEST_CODE_000001","name":"TEST_CODE_fixture","quantity":100,"cost_price":12.0}]}"#,
        DateTime::parse_from_rfc3339("2026-10-09T21:20:00+08:00").unwrap()).unwrap();
    writer.execute("INSERT INTO user_position_snapshot(snapshot_id,effective_at,confirmed_at,source,confirm_empty,evidence_sha256,item_count) VALUES(?1,'2026-10-09T21:13:00+08:00','2026-10-09T21:20:00+08:00','user_confirmed_full_snapshot',0,?2,1)",rusqlite::params![input.snapshot_id,input.evidence_sha256]).unwrap();
    writer.execute("INSERT INTO user_position_snapshot_item VALUES(?1,'TEST_CODE_000001','TEST_CODE_fixture',100,12)",[&input.snapshot_id]).unwrap();
    let summary=writer.query_row("SELECT id,effective_at,total_assets,securities_market_value,available_cash,position_ratio_pct,daily_pnl,source,recorded_at FROM user_account_summary WHERE id=1",[],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"effective_at":r.get::<_,String>(1)?,"total_assets":r.get::<_,f64>(2)?,"securities_market_value":r.get::<_,f64>(3)?,"available_cash":r.get::<_,f64>(4)?,"position_ratio_pct":r.get::<_,f64>(5)?,"daily_pnl":r.get::<_,f64>(6)?,"source":r.get::<_,String>(7)?,"recorded_at":r.get::<_,String>(8)?}))).unwrap();
    let snapshot_row=writer.query_row("SELECT id,snapshot_id,effective_at,confirmed_at,source,confirm_empty,evidence_sha256,item_count,recorded_at FROM user_position_snapshot WHERE id=1",[],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"snapshot_id":r.get::<_,String>(1)?,"effective_at":r.get::<_,String>(2)?,"confirmed_at":r.get::<_,String>(3)?,"source":r.get::<_,String>(4)?,"confirm_empty":r.get::<_,i64>(5)?,"evidence_sha256":r.get::<_,String>(6)?,"item_count":r.get::<_,i64>(7)?,"recorded_at":r.get::<_,String>(8)?}))).unwrap();
    let image = directory.join("TEST_CODE_original.jpg");
    std::fs::write(&image, b"TEST_CODE_same_batch_image_bytes").unwrap();
    let image_sha = bytes_sha256(b"TEST_CODE_same_batch_image_bytes");
    let evidence=json!({"original_image_sha256":image_sha,"effective_at":snapshot_row["effective_at"],"confirmed_at":snapshot_row["confirmed_at"],"snapshot_id":input.snapshot_id,"position_evidence_sha256":input.evidence_sha256,"market_prices_role":SNAPSHOT_MARK_SOURCE,"positions":[{"code":"TEST_CODE_000001","name":"TEST_CODE_fixture","quantity":100,"cost_price":"12.000","current_price":"10.000","market_value":"1000.00"}]}).to_string();
    let metadata = std::fs::metadata(&original).unwrap();
    let receipt=json!({"status":"committed_and_verified","database":original,"database_identity":{"device":metadata.dev(),"inode":metadata.ino()},"effective_at":snapshot_row["effective_at"],"confirmed_at":snapshot_row["confirmed_at"],"position_row_id":1,"account_summary_row_id":1,"snapshot_id":input.snapshot_id,"image_sha256":image_sha,"readback":{"summary":summary,"positions":snapshot_row,"items":[{"code":"TEST_CODE_000001","name":"TEST_CODE_fixture","quantity":100,"cost_price":12.0}]}}).to_string();
    let evidence_path = directory.join("TEST_CODE_evidence.json");
    let receipt_path = directory.join("TEST_CODE_import_receipt.json");
    std::fs::write(&evidence_path, &evidence).unwrap();
    std::fs::write(&receipt_path, &receipt).unwrap();
    let cutover = DateTime::parse_from_rfc3339("2026-10-09T21:13:00+08:00")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let request = SnapshotPaperActivationRequest {
        schema_version: 1,
        summary_row_id: 1,
        snapshot_id: input.snapshot_id,
        source_batch_reference: "TEST_CODE_user_snapshot_image_batch".into(),
        source_evidence: SnapshotSourceEvidence {
            image_path: image,
            image_sha256: image_sha,
            snapshot_evidence_path: evidence_path,
            snapshot_evidence_sha256: bytes_sha256(evidence.as_bytes()),
            import_receipt_path: receipt_path,
            import_receipt_sha256: bytes_sha256(receipt.as_bytes()),
        },
        establish_after_hours_close: true,
        seed: SeedManifest {
            account_id: "TEST_CODE_paper_account".into(),
            epoch_id: "TEST_CODE_paper_epoch".into(),
            command_id: "TEST_CODE_activate".into(),
            cutover_at: cutover,
            account_effective_at: cutover,
            positions_effective_at: cutover,
            source_reference: "TEST_CODE_user_snapshot_image_batch".into(),
            source_hash: String::new(),
            approved_by: "TEST_CODE_explicit_user".into(),
            cash: Money::from_cny(10000.0).unwrap(),
            original_total: Money::from_cny(11000.0).unwrap(),
            excluded_residual: None,
            lots: vec![SeedLot {
                code: "TEST_CODE_000001".into(),
                name: "TEST_CODE_fixture".into(),
                quantity: 100,
                reported_cost: Some(Money::from_cny(12.0).unwrap()),
                sellable_from: None,
                sellability_evidence: None,
            }],
            marks: vec![Mark {
                code: "TEST_CODE_000001".into(),
                price: Money::from_cny(10.0).unwrap(),
                observed_at: cutover,
                source: SNAPSHOT_MARK_SOURCE.into(),
            }],
            policy: RiskPolicyV1::default(),
        },
    };
    drop(writer);
    let now = DateTime::parse_from_rfc3339("2026-10-09T21:30:00+08:00")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let mut conn = open_snapshot_activation_database(&original, true).unwrap();
    let prepared = preview_snapshot_paper_activation(&mut conn, &request, now)
        .unwrap()
        .prepared_request;
    let outcome = apply_snapshot_paper_activation(&mut conn, &prepared, now).unwrap();
    drop(conn);
    let snapshot = directory.join("TEST_CODE_detached_snapshot.db");
    std::fs::copy(&original, &snapshot).unwrap();
    NativeWeeklyFixture {
        _dir: dir,
        original,
        snapshot,
        binding: outcome.binding,
    }
}
fn native_weekly_child(
    fixture: &NativeWeeklyFixture,
    case: &str,
    observed_at: &str,
    binding: Option<&str>,
    origin: Option<&PathBuf>,
    include_manifest: bool,
) -> serde_json::Value {
    use serde_json::json;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let directory = fixture.original.parent().unwrap();
    let output = directory.join(format!("{case}.json"));
    let manifest = directory.join(format!("{case}.source.json"));
    let original = origin.unwrap_or(&fixture.original);
    let origin_meta = std::fs::metadata(original).unwrap();
    let snapshot_meta = std::fs::metadata(&fixture.snapshot).unwrap();
    let source = paper_account::SnapshotSource {
        schema_version: 1,
        original_database: paper_account::DatabaseIdentity {
            path: original.clone(),
            device: origin_meta.dev(),
            inode: origin_meta.ino(),
        },
        snapshot_database: paper_account::DatabaseIdentity {
            path: fixture.snapshot.clone(),
            device: snapshot_meta.dev(),
            inode: snapshot_meta.ino(),
        },
        snapshot_sha256: file_sha256(&fixture.snapshot).unwrap(),
        paper_binding_sha256: binding.map(|b| bytes_sha256(b.as_bytes())),
    };
    std::fs::write(&manifest, serde_json::to_vec(&source).unwrap()).unwrap();
    std::fs::set_permissions(&manifest, std::fs::Permissions::from_mode(0o400)).unwrap();
    let request = directory.join(format!("{case}.request.json"));
    std::fs::write(&request,json!({"database":fixture.snapshot,"manifest":include_manifest.then_some(manifest),"output":output,"observed_at":observed_at}).to_string()).unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap());
    child
        .args([
            "--exact",
            "tests::weekly_native_process_fixture_entry",
            "--nocapture",
        ])
        .env("TEST_CODE_WEEKLY_CHILD_REQUEST", request)
        .env_remove(stock_analysis::trading::paper_ledger_runtime::BINDING_ENV);
    if let Some(binding) = binding {
        child.env(
            stock_analysis::trading::paper_ledger_runtime::BINDING_ENV,
            binding,
        );
    }
    let result = child.output().unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    serde_json::from_slice(&std::fs::read(output).unwrap()).unwrap()
}

#[test]
fn weekly_native_snapshot_epoch_reports_accurate_independent_account_and_legacy_separately() {
    let fixture = native_weekly_fixture();
    let original_before = std::fs::read(&fixture.original).unwrap();
    let snapshot_before = std::fs::read(&fixture.snapshot).unwrap();
    let binding = serde_json::to_string(&fixture.binding).unwrap();
    let review = native_weekly_child(
        &fixture,
        "valid",
        "2026-10-09T22:00:00+08:00",
        Some(&binding),
        None,
        true,
    );
    let account = &review["paper_account"]["value"];
    assert_eq!(review["paper_account"]["status"], "available");
    assert_eq!(account["ledger_kind"], "NativeSnapshotPaperV1");
    assert_eq!(account["cash_cny"], 10000.0);
    assert_eq!(account["market_value_cny"], 1000.0);
    assert_eq!(account["total_equity_cny"], 11000.0);
    assert_eq!(account["seed_equity_cny"], 11000.0);
    assert_eq!(account["since_cutover_net_pnl_cny"], 0.0);
    assert_eq!(account["modeled_fees_cny"], 0.0);
    assert_eq!(account["holdings"][0]["reported_actual_cost_cny"], 12.0);
    assert_eq!(account["holdings"][0]["basis_price_cny"], 10.0);
    assert_eq!(account["valuation_effective_at"], "2026-10-09T13:13:00Z");
    assert_eq!(account["daily_pnl_cny"]["status"], "unavailable");
    assert_eq!(review["verified_paper"]["value"]["source_fill_rows"], 0);
    assert_eq!(
        review["legacy_verified_paper"]["status"], "available",
        "bound legacy read failed: {}",
        review["legacy_verified_paper"]
    );
    assert_eq!(
        review["legacy_verified_paper"]["value"]["source_fill_rows"],
        1
    );
    assert_eq!(review["raw_paper"]["value"]["historical_filled_rows"], 1);
    let stale = native_weekly_child(
        &fixture,
        "stale_marks",
        "2026-10-12T22:00:00+08:00",
        Some(&binding),
        None,
        true,
    );
    assert_eq!(stale["paper_account"]["status"], "available");
    assert_eq!(
        stale["paper_account"]["value"]["valuation_is_current_observation_day"],
        false
    );
    assert_eq!(
        stale["paper_account"]["value"]["daily_pnl_cny"]["status"],
        "unavailable"
    );
    assert_eq!(
        stale["paper_account"]["value"]["valuation_effective_at"],
        "2026-10-09T13:13:00Z"
    );
    assert_eq!(std::fs::read(&fixture.original).unwrap(), original_before);
    assert_eq!(std::fs::read(&fixture.snapshot).unwrap(), snapshot_before);
}

#[test]
fn weekly_native_snapshot_unknown_legacy_family_stays_unavailable_and_independent() {
    let fixture = native_weekly_fixture_with_legacy_reason("TEST_CODE_legacy");
    let original_before = std::fs::read(&fixture.original).unwrap();
    let snapshot_before = std::fs::read(&fixture.snapshot).unwrap();
    let binding = serde_json::to_string(&fixture.binding).unwrap();
    let review = native_weekly_child(
        &fixture,
        "unknown_legacy_family",
        "2026-10-09T22:00:00+08:00",
        Some(&binding),
        None,
        true,
    );
    assert_eq!(review["paper_account"]["status"], "available");
    assert_eq!(review["verified_paper"]["value"]["source_fill_rows"], 0);
    assert_eq!(review["legacy_verified_paper"]["status"], "unavailable");
    assert!(review["legacy_verified_paper"]["value"].is_null());
    assert!(review["legacy_verified_paper"]["reason"]
        .as_str()
        .unwrap()
        .contains("entry strategy family unavailable"));
    assert_eq!(review["raw_paper"]["value"]["historical_filled_rows"], 1);
    assert_eq!(std::fs::read(&fixture.original).unwrap(), original_before);
    assert_eq!(std::fs::read(&fixture.snapshot).unwrap(), snapshot_before);
}

#[test]
fn weekly_native_snapshot_missing_wrong_binding_fixture_source_and_historical_head_never_fallback()
{
    let fixture = native_weekly_fixture();
    let binding = serde_json::to_string(&fixture.binding).unwrap();
    let mut wrong = fixture.binding.clone();
    wrong.epoch_id = "TEST_CODE_wrong_epoch".into();
    let wrong = serde_json::to_string(&wrong).unwrap();
    let copied = fixture
        .original
        .parent()
        .unwrap()
        .join("TEST_CODE_fixture_copy.db");
    std::fs::copy(&fixture.original, &copied).unwrap();
    for (case, clock, binding, origin, manifest) in [
        (
            "missing_binding",
            "2026-10-09T22:00:00+08:00",
            None,
            None,
            true,
        ),
        (
            "wrong_binding",
            "2026-10-09T22:00:00+08:00",
            Some(wrong.as_str()),
            None,
            true,
        ),
        (
            "copied_original",
            "2026-10-09T22:00:00+08:00",
            Some(binding.as_str()),
            Some(&copied),
            true,
        ),
        (
            "missing_manifest",
            "2026-10-09T22:00:00+08:00",
            Some(binding.as_str()),
            None,
            false,
        ),
        (
            "before_activation_commit",
            "2026-10-09T21:25:00+08:00",
            Some(binding.as_str()),
            None,
            true,
        ),
        (
            "historical_future_head",
            "2026-10-08T22:00:00+08:00",
            Some(binding.as_str()),
            None,
            true,
        ),
    ] {
        let review = native_weekly_child(&fixture, case, clock, binding, origin, manifest);
        assert_eq!(
            review["paper_account"]["status"], "unavailable",
            "{case}: {review}"
        );
        assert_eq!(review["verified_paper"]["status"], "unavailable", "{case}");
        assert!(
            review["raw_paper"]["value"].is_object(),
            "raw facts retained: {case}"
        );
    }
}
