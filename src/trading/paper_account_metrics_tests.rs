use super::*;
use crate::trading::paper_trade::{Direction, PaperRiskContext};
use chrono::{Duration, TimeZone};
use diesel::connection::SimpleConnection;

fn friday() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 9, 13, 13, 0).unwrap()
}

fn monday() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 12, 2, 0, 0).unwrap()
}

fn fixture() -> (tempfile::TempDir, DatabaseManager, AccountBinding) {
    let dir = tempfile::tempdir().unwrap();
    let db = DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_paper_metrics.db"))
        .unwrap();
    db.get_conn()
        .unwrap()
        .batch_execute("PRAGMA application_id=1398035265; PRAGMA user_version=2")
        .unwrap();
    // An unrelated, unmatched legacy sell cannot affect the independent
    // opening inventory, cash or account loss streak.
    db.get_conn().unwrap().batch_execute("INSERT INTO paper_trades(plan_id,code,name,direction,price,quantity,status,fill_price,virtual_reason,account_mode,data_mode,ts) VALUES ('TEST_CODE_old','TEST_CODE_OLD','old','sell',0.073,100,'Filled',0.073,'TEST_CODE_old','Normal','Full','2026-09-01 10:00:00')").unwrap();
    let seed = SeedManifest {
        account_id: "TEST_CODE_PAPER_METRICS_ACCOUNT".into(),
        epoch_id: "TEST_CODE_PAPER_METRICS_EPOCH".into(),
        command_id: "TEST_CODE_explicit_seed".into(),
        cutover_at: friday(),
        account_effective_at: friday(),
        positions_effective_at: friday(),
        source_reference: "TEST_CODE_confirmed_snapshot".into(),
        source_hash: "a".repeat(64),
        approved_by: "TEST_CODE_operator".into(),
        cash: Money::from_cny(7254.94).unwrap(),
        original_total: Money::from_cny(17254.94).unwrap(),
        excluded_residual: None,
        lots: vec![SeedLot {
            code: "TEST_CODE_000001".into(),
            name: "fixture".into(),
            quantity: 1000,
            reported_cost: Some(Money::from_cny(50.).unwrap()),
            sellable_from: None,
            sellability_evidence: None,
        }],
        marks: vec![Mark {
            code: "TEST_CODE_000001".into(),
            price: Money::from_cny(10.).unwrap(),
            observed_at: friday(),
            source: "TEST_CODE_snapshot_price".into(),
        }],
        policy: RiskPolicyV1::default(),
    };
    let binding = seed.binding().unwrap();
    PaperLedger::open(&db, &friday)
        .apply(PaperCommand::Seed(seed))
        .unwrap();
    (dir, db, binding)
}

fn quote(price: f64, at: DateTime<Utc>) -> ExecutionQuote {
    ExecutionQuote {
        price,
        limit_down_price: 1.,
        limit_up_price: 100.,
        observed_at: at,
    }
}

fn seed_close(db: &DatabaseManager, binding: &AccountBinding) {
    settle_closing_valuation_on(db, binding, china_day(friday()), &friday, &|_, _| {
        Ok((
            Money::from_cny(10.).unwrap(),
            "TEST_CODE_approved_opening_close".into(),
        ))
    })
    .unwrap();
}

fn sell_seed(db: &DatabaseManager, binding: &AccountBinding, price: f64, quantity: u32) {
    let ledger = PaperLedger::open(db, &monday);
    let before = ledger.read(binding).unwrap();
    let signal = PaperSignal {
        plan_id: format!("paper:{}:TEST_CODE_seed_exit", binding.epoch_id),
        code: "TEST_CODE_000001".into(),
        name: "fixture".into(),
        direction: Direction::Sell,
        price,
        quantity,
        virtual_reason: "TEST_CODE_exit".into(),
        is_limit_up: false,
        is_limit_down: false,
        is_suspended: false,
        limit_up_price: Some(20.),
        limit_down_price: Some(1.),
        secondary_confirmed: false,
        quote_observed_at: monday(),
        risk_context: PaperRiskContext::new(
            crate::risk::action_gate::AccountMode::Normal,
            crate::monitor::data_mode::DataMode::Full,
        ),
    };
    let fill = ledger
        .apply(PaperCommand::Execute(ExecuteIntent {
            binding: binding.clone(),
            command_id: "TEST_CODE_exit".into(),
            expected_version: before.version,
            inventory_fingerprint: before.inventory_fingerprint().unwrap(),
            signal,
            price_intent: PriceIntent::SignalQuoteMarketV1,
            quote_price: Money::from_cny(price).unwrap(),
            price_qualification: ExecutionPriceQualification::for_test(
                "TEST_CODE_000001",
                china_day(monday()),
            ),
            marks: vec![Mark {
                code: "TEST_CODE_000001".into(),
                price: Money::from_cny(price).unwrap(),
                observed_at: monday(),
                source: "TEST_CODE_qualified_quote".into(),
            }],
        }))
        .unwrap();
    assert_eq!(fill.status, LedgerStatus::Filled);
}

#[test]
fn paper_metrics_seed_fact_preserves_time_and_ignores_later_real_cash_import() {
    let (_dir, db, binding) = fixture();
    db.get_conn().unwrap().batch_execute("INSERT INTO user_account_summary(effective_at,total_assets,securities_market_value,available_cash,position_ratio_pct,daily_pnl,source) VALUES ('2026-10-09T21:14:00+08:00',999999,0,999999,0,-568,'TEST_CODE_later_real_snapshot')").unwrap();
    let later = || friday() + Duration::minutes(10);
    let result = account_metrics_on(&db, &binding, &later, None).unwrap();
    assert_eq!(result.cash, 7254.94);
    assert_eq!(result.market_value, 10000.);
    assert_eq!(result.total_assets, 17254.94);
    assert_eq!(result.effective_at, friday());
    assert_eq!(result.valuation_source, "初始快照估值");
    assert_eq!(result.metrics.today_pnl_pct, Some(0.));
    assert_eq!(result.metrics.consecutive_stop_loss_n, Some(0));
    assert_eq!(
        result.pnl_period,
        Some(PaperPnlPeriod::SinceCutover {
            cutover_at: friday()
        })
    );
    assert_eq!(
        PaperLedger::open(&db, &later)
            .read(&binding)
            .unwrap()
            .version,
        1
    );
}

#[test]
fn paper_metrics_live_quotes_use_previous_paper_close_without_refreshing_stored_marks() {
    let (_dir, db, binding) = fixture();
    seed_close(&db, &binding);
    let ledger = PaperLedger::open(&db, &monday);
    let before = ledger.read(&binding).unwrap();
    let quotes = |_: &str| Ok(quote(12., monday() - Duration::seconds(2)));
    let result = account_metrics_on(&db, &binding, &monday, Some(&quotes)).unwrap();
    assert_eq!(result.cash, 7254.94);
    assert_eq!(result.total_assets, 19254.94);
    assert_eq!(result.effective_at, monday() - Duration::seconds(2));
    assert_eq!(
        result.pnl_period,
        Some(PaperPnlPeriod::PreviousClose {
            price_date: china_day(friday())
        })
    );
    assert!((result.metrics.today_pnl_pct.unwrap() - 2000. / 17254.94 * 100.).abs() < 1e-10);
    assert_eq!(ledger.read(&binding).unwrap(), before);
    let frozen = account_metrics_on(&db, &binding, &monday, None).unwrap();
    assert!(
        frozen.metrics.today_pnl_pct.is_none(),
        "old paper marks are not today's price facts"
    );
    assert_eq!(frozen.effective_at, friday());
}

#[test]
fn paper_metrics_missing_previous_close_and_stale_quotes_fail_closed() {
    let (_dir, db, binding) = fixture();
    let quotes = |_: &str| Ok(quote(12., monday()));
    let result = account_metrics_on(&db, &binding, &monday, Some(&quotes)).unwrap();
    assert!(result.metrics.today_pnl_pct.is_none());
    assert!(result.pnl_period.is_none());
    assert!(!result.metrics.is_complete());
    let stale = |_: &str| Ok(quote(12., monday() - Duration::seconds(6)));
    assert!(account_metrics_on(&db, &binding, &monday, Some(&stale))
        .unwrap_err()
        .contains("fresh qualified quotes"));
    assert_eq!(
        PaperLedger::open(&db, &monday)
            .read(&binding)
            .unwrap()
            .version,
        1
    );
}

#[test]
fn paper_metrics_live_quote_acquisition_rejects_concurrent_head_change() {
    let (_dir, db, binding) = fixture();
    let quotes = |_: &str| {
        settle_closing_valuation_on(&db, &binding, china_day(friday()), &friday, &|_, _| {
            Ok((
                Money::from_cny(10.).unwrap(),
                "TEST_CODE_raced_close".into(),
            ))
        })?;
        Ok(quote(12., monday()))
    };
    let error = account_metrics_on(&db, &binding, &monday, Some(&quotes)).unwrap_err();
    assert!(error.contains("version changed"), "{error}");
}

#[test]
fn paper_close_is_complete_idempotent_and_changes_only_valuation() {
    let (_dir, db, binding) = fixture();
    let ledger = PaperLedger::open(&db, &monday);
    let before = ledger.read(&binding).unwrap();
    assert!(
        settle_closing_valuation_on(&db, &binding, china_day(monday()), &monday, &|_, _| Ok((
            Money::from_cny(12.).unwrap(),
            "TEST_CODE_early".into()
        )))
        .is_err()
    );
    let after_close = || monday() + Duration::hours(6);
    assert!(settle_closing_valuation_on(
        &db,
        &binding,
        china_day(monday()),
        &after_close,
        &|_, _| Err("TEST_CODE_missing_exact_close".into())
    )
    .is_err());
    assert_eq!(ledger.read(&binding).unwrap(), before);
    let acquired_source = serde_json::json!({"kind":"qualified_daily_close_v1","price_date":"2026-10-12","source_at":"2026-10-12T15:00:00+08:00","observed_at":"2026-10-12T16:00:00+08:00","batch_id":"TEST_CODE_batch"}).to_string();
    let recorded =
        settle_closing_valuation_on(&db, &binding, china_day(monday()), &after_close, &|_, _| {
            Ok((Money::from_cny(12.).unwrap(), acquired_source.clone()))
        })
        .unwrap()
        .unwrap();
    assert_eq!(recorded.status, LedgerStatus::Marked);
    let after = ledger.read(&binding).unwrap();
    assert_eq!(after.cash, before.cash);
    assert_eq!(after.lots, before.lots);
    assert_eq!(after.fees, before.fees);
    assert_eq!(after.realized_pnl, before.realized_pnl);
    assert_eq!(after.closes[&china_day(monday())].cny(), 19254.94);
    assert_eq!(after.marks["TEST_CODE_000001"].source, acquired_source);
    assert!(settle_closing_valuation_on(
        &db,
        &binding,
        china_day(monday()),
        &after_close,
        &|_, _| panic!("already settled date must not reacquire prices")
    )
    .unwrap()
    .is_none());
    assert_eq!(ledger.read(&binding).unwrap(), after);
}

#[test]
fn paper_metrics_loss_streak_includes_seed_exits_with_fees_and_excludes_raw_history() {
    let (_dir, db, binding) = fixture();
    seed_close(&db, &binding);
    // Gross gain is CNY 1, but commission/stamp make this an account-net loss.
    sell_seed(&db, &binding, 10.01, 100);
    let result = account_metrics_on(&db, &binding, &monday, None).unwrap();
    assert_eq!(result.metrics.consecutive_stop_loss_n, Some(1));
    assert!(
        result.cash > 7254.94,
        "sale proceeds remain in independent paper cash"
    );
}

#[test]
fn paper_metrics_full_sell_cash_only_does_not_require_an_obsolete_quote() {
    let (_dir, db, binding) = fixture();
    seed_close(&db, &binding);
    sell_seed(&db, &binding, 12., 1000);
    let later = || monday() + Duration::seconds(10);
    let result = account_metrics_on(
        &db,
        &binding,
        &later,
        Some(&|_: &str| panic!("cash-only paper account has no quote dependency")),
    )
    .unwrap();
    assert_eq!(result.metrics.total_pos_cheng, Some(0));
    assert!(result.metrics.is_complete());
    assert_eq!(result.market_value, 0.);
    assert_eq!(result.total_assets, result.cash);
    assert_eq!(
        result.effective_at,
        monday(),
        "balance provenance remains the fill instant"
    );
    assert!(result.valuation_source.contains("纯现金"));
}

#[test]
fn paper_close_qualification_requires_exact_instrument_date_settlement_and_adjustment() {
    use crate::data_gateway::{AdmittedDailyBars, BatchEvidence};
    use crate::data_provider::{AdjustType, KlineData};
    let date = china_day(monday());
    let bar = KlineData {
        date,
        open: 12.,
        high: 12.,
        low: 12.,
        close: 12.,
        volume: 100.,
        amount: 1200.,
        pct_chg: 0.,
        intraday_price: None,
        settled: true,
        pe_ratio: None,
        pb_ratio: None,
        turnover_rate: None,
        market_cap: None,
        circulating_cap: None,
        eps: None,
        roe: None,
        revenue_yoy: None,
        net_profit_yoy: None,
        gross_margin: None,
        net_margin: None,
        sharpe_ratio: None,
        financials_history: None,
        valuation_history: None,
        consensus: None,
        industry: None,
        is_limit_up: false,
        is_limit_down: false,
        is_suspended: false,
        adjust: AdjustType::None,
    };
    let evidence = BatchEvidence {
        provider: crate::market_domain::ProviderId::Tdx,
        source: "TEST_CODE_qualified_close".into(),
        source_at: Some("2026-10-12T15:00:00+08:00".into()),
        observed_at: "2026-10-12T16:00:00+08:00".into(),
        batch_id: "TEST_CODE_close_batch".into(),
    };
    let admitted = |code: &str, records| {
        AdmittedDailyBars::from_test_fixture(code, records, evidence.clone()).unwrap()
    };
    let valid = admitted("TEST_CODE_000001", vec![bar.clone()]);
    let (price, source) = close_from_admitted("TEST_CODE_000001", date, &valid).unwrap();
    assert_eq!(price.cny(), 12.);
    let source: serde_json::Value = serde_json::from_str(&source).unwrap();
    assert_eq!(source["price_date"], "2026-10-12");
    assert_eq!(source["source_at"], "2026-10-12T15:00:00+08:00");
    assert_eq!(source["observed_at"], "2026-10-12T16:00:00+08:00");
    assert_eq!(source["batch_id"], "TEST_CODE_close_batch");
    assert!(close_from_admitted("TEST_CODE_other", date, &valid).is_err());
    assert!(close_from_admitted("TEST_CODE_000001", date + Duration::days(1), &valid).is_err());
    for mutation in [0, 1, 2, 3] {
        let mut invalid = bar.clone();
        match mutation {
            0 => invalid.settled = false,
            1 => invalid.adjust = AdjustType::Qfq,
            2 => invalid.close = 0.,
            _ => invalid.close = f64::NAN,
        }
        let invalid = admitted("TEST_CODE_000001", vec![invalid]);
        assert!(close_from_admitted("TEST_CODE_000001", date, &invalid).is_err());
    }
    let duplicate = admitted("TEST_CODE_000001", vec![bar.clone(), bar]);
    assert!(close_from_admitted("TEST_CODE_000001", date, &duplicate).is_err());
}
