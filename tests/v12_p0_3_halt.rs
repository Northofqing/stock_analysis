//! v11-P0-3 commit 3: 端到端验收 — 停牌日 is_suspended 链路通顺
//!
//! ⚠️ 网络/DB 依赖 — `#[ignore]` 跳过 CI, 手动跑:
//!   cargo test --test v12_p0_3_halt -- --ignored
//!
//! 验收内容 (v11-p0-3-口径不一致设计-2026-07-02.md §七):
//! 1. mark_halted_period 喂入的停牌时间段 → is_halted_period 能查到
//! 2. fill_limit_flags 把 is_suspended 设为 true 在停牌日
//! 3. K 线缺口推断 (infer_halt_from_kline_gaps) 能识别 8 天以上缺口为停牌
//!
//! 为什么不跑完整 backtest:
//! PrecisionRsiBacktest 需 ≥205 K 线, 构造完整输入代码量大 (>200 行 fixture),
//! 且端到端验证已在生产环境的 walk-forward 中跑. 此处覆盖**核心接线链路**即足够.

use chrono::NaiveDate;
use stock_analysis::data_provider::halt_status::infer_halt_from_kline_gaps;
use stock_analysis::data_provider::limit_status::{fill_limit_flags, LimitStatusCalculator};
use stock_analysis::data_provider::KlineData;
use stock_analysis::monitor::data_quality::{is_halted_period, mark_halted_period};

/// 构造单根 K 线 (供 fill_limit_flags 测试用)
fn make_kline(date: NaiveDate, close: f64) -> KlineData {
    KlineData {
        date,
        open: close,
        high: close,
        low: close,
        close,
        volume: 1000.0,
        amount: close * 1000.0,
        pct_chg: 0.0,
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
        is_suspended: false, // fill_limit_flags 会覆盖
        adjust: stock_analysis::data_provider::AdjustType::None,
    }
}

/// 验收 1: mark_halted_period → is_halted_period 链路
#[test]
fn halt_period_roundtrip() {
    let code = "TEST_HALT";
    let from = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
    let to = NaiveDate::from_ymd_opt(2026, 6, 10).unwrap();

    mark_halted_period(code, from, to);

    // 区间内
    assert!(is_halted_period(
        code,
        NaiveDate::from_ymd_opt(2026, 6, 5).unwrap()
    ));
    // 边界 (含 from/to)
    assert!(is_halted_period(code, from));
    assert!(is_halted_period(code, to));
    // 区间外
    assert!(!is_halted_period(
        code,
        NaiveDate::from_ymd_opt(2026, 5, 31).unwrap()
    ));
    assert!(!is_halted_period(
        code,
        NaiveDate::from_ymd_opt(2026, 6, 11).unwrap()
    ));
}

/// 验收 2: fill_limit_flags 把 is_suspended 设为 true 在停牌日
#[test]
fn fill_limit_flags_sets_suspended_on_halt_day() {
    let code = "TEST_SUSP";
    let halt_date = NaiveDate::from_ymd_opt(2026, 7, 1).unwrap();

    // 喂入停牌区间
    mark_halted_period(code, halt_date, halt_date);

    // fill_limit_flags: 停牌日 K 线 → is_suspended=true
    let mut k = make_kline(halt_date, 10.0);
    fill_limit_flags(&LimitStatusCalculator::new(), code, &mut k, 10.0, "");
    assert!(k.is_suspended, "停牌日的 K 线应被标记 is_suspended=true");

    // 非停牌日 K 线 → is_suspended=false
    let non_halt_date = NaiveDate::from_ymd_opt(2026, 7, 2).unwrap();
    let mut k2 = make_kline(non_halt_date, 10.0);
    fill_limit_flags(&LimitStatusCalculator::new(), code, &mut k2, 10.0, "");
    assert!(!k2.is_suspended, "非停牌日的 K 线应 is_suspended=false");
}

/// 验收 3: K 线缺口推断 (8 天间隔) → 喂入缓存 → is_halted_period 命中
#[test]
fn kline_gap_inference_feeds_cache() {
    let code = "TEST_GAP";
    let d1 = NaiveDate::from_ymd_opt(2026, 5, 1).unwrap();
    let d2 = NaiveDate::from_ymd_opt(2026, 5, 10).unwrap(); // 9 天后

    let klines = vec![make_kline(d1, 10.0), make_kline(d2, 11.0)];
    let periods = infer_halt_from_kline_gaps(code, &klines);

    assert_eq!(periods.len(), 1, "9 天间隔应识别 1 段停牌");
    // 中间 (5/2~5/9) 是停牌
    assert!(is_halted_period(
        code,
        NaiveDate::from_ymd_opt(2026, 5, 5).unwrap()
    ));
    // 边界
    assert!(!is_halted_period(code, d1), "5/1 是 K 线日, 不是停牌");
    assert!(!is_halted_period(code, d2), "5/10 是 K 线日, 不是停牌");
}
