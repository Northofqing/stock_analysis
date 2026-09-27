//! Legacy K-line gap diagnostics.
//!
//! ## 背景
//!
//! `LimitStatus.is_suspended` 之前永远 false (limit_status.rs:60), 停牌检测是死代码.
//! A gap is not authority for a suspension. This module may identify ranges
//! worth investigating, but it must never write the formal halted-period cache
//! or authorize trading/backtest behavior.
//!
//! ## 诊断规则（没有交易授权能力）
//!
//! 给定某只股票的 K 线列表 (按日期**升序**, 来自 `validate_daily_kline_quality` 排序后),
//! 如果相邻 K 线的日期间隔 > 7 个自然日，仅输出待人工调查的缺口范围：
//! - date[i+1] - date[i] > 7 天 → 中间 days = (date[i]+1, date[i+1]-1)
//!
//! **为什么是 7 天**: A 股最长连续休市是春节 (通常 7 天), 7 天以上必有停牌.
//! 用 7 天阈值避免误判春节/国庆.
//!
//! 调用方只能记录/展示这些范围，不得写入交易门或替代
//! `QualifiedSuspensionEvidence`。

use chrono::NaiveDate;

use crate::data_provider::KlineData;
/// Diagnostic threshold: an interval longer than this is worth investigating.
///
/// 7 天 = A 股最长连续休市 (春节), 7 天以上必有停牌.
pub const HALT_GAP_THRESHOLD_DAYS: i64 = 7;

/// K-line gaps → diagnostic candidate ranges [(from, to), ...].
///
/// # Arguments
/// - `code`: 股票代码（仅用于诊断日志）
/// - `klines`: K 线列表, **任意顺序** (内部按日期 sort 升序)
///
/// # Returns
/// Candidate ranges only. Every (from, to) is inclusive; no returned range is
/// a qualified suspension fact.
pub fn infer_halt_from_kline_gaps(code: &str, klines: &[KlineData]) -> Vec<(NaiveDate, NaiveDate)> {
    let mut periods = Vec::new();
    if klines.len() < 2 {
        return periods;
    }
    // 内部排序 (provider 返回的可能降序, 也可能升序)
    let mut sorted: Vec<&KlineData> = klines.iter().collect();
    sorted.sort_by_key(|k| k.date);
    for w in sorted.windows(2) {
        let prev = w[0];
        let cur = w[1];
        let gap_days = (cur.date - prev.date).num_days();
        if gap_days > HALT_GAP_THRESHOLD_DAYS {
            // 中间 (prev.date + 1, cur.date - 1) 是停牌
            let from = prev.date + chrono::Duration::days(1);
            let to = cur.date - chrono::Duration::days(1);
            log::warn!(
                "[{code}] diagnostic_only kline_gap_candidate={from}..={to}; not suspension authority"
            );
            periods.push((from, to));
        }
    }
    periods
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitor::data_quality::is_halted_period;

    fn make_kline(date: NaiveDate) -> KlineData {
        KlineData {
            date,
            open: 10.0,
            high: 10.5,
            low: 9.8,
            close: 10.0,
            volume: 1000.0,
            amount: 10000.0,
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
            is_suspended: false,
            adjust: crate::data_provider::AdjustType::None,
        }
    }

    /// v11-P0-3 commit 2: 7 天阈值 (春节) 不算停牌, 8+ 天算停牌
    #[test]
    fn test_gap_threshold() {
        // 用独特 code 避免与其他测试污染
        let code = "999991";
        let d1 = NaiveDate::from_ymd_opt(2026, 1, 20).unwrap();
        let d8 = NaiveDate::from_ymd_opt(2026, 1, 28).unwrap(); // 8 天后 (春节回)
        let d10 = NaiveDate::from_ymd_opt(2026, 1, 30).unwrap(); // 10 天后
        let klines = vec![make_kline(d1), make_kline(d8), make_kline(d10)];
        let periods = infer_halt_from_kline_gaps(code, &klines);
        // d1→d8 = 8 天 → 中间 7 天 (1/21~1/27) 停牌 (春节只有 7 天, 8 天间隔意味着更长停牌)
        assert_eq!(periods.len(), 1, "8 天间隔应识别为停牌");
        assert!(
            !is_halted_period(code, NaiveDate::from_ymd_opt(2026, 1, 25).unwrap()),
            "diagnostic gap must not populate formal halt cache"
        );
    }

    /// v11-P0-3 commit 2: 7 天间隔 (春节正常) 不算停牌
    #[test]
    fn test_7_days_no_halt() {
        let code = "999992";
        let d1 = NaiveDate::from_ymd_opt(2026, 2, 6).unwrap();
        let d8 = NaiveDate::from_ymd_opt(2026, 2, 13).unwrap(); // 7 天后 (春节后)
        let klines = vec![make_kline(d1), make_kline(d8)];
        let periods = infer_halt_from_kline_gaps(code, &klines);
        assert!(periods.is_empty(), "7 天间隔 (春节) 不应算停牌");
    }

    /// v11-P0-3 commit 2: 多段连续停牌
    #[test]
    fn test_multiple_halts() {
        let code = "999993";
        // 模拟两次停牌: 10 天 + 15 天
        let klines = vec![
            make_kline(NaiveDate::from_ymd_opt(2026, 3, 1).unwrap()),
            make_kline(NaiveDate::from_ymd_opt(2026, 3, 11).unwrap()), // 10 天
            make_kline(NaiveDate::from_ymd_opt(2026, 3, 26).unwrap()), // 15 天
        ];
        let periods = infer_halt_from_kline_gaps(code, &klines);
        assert_eq!(periods.len(), 2, "应识别 2 段停牌");
        assert!(!is_halted_period(
            code,
            NaiveDate::from_ymd_opt(2026, 3, 5).unwrap()
        ));
        assert!(!is_halted_period(
            code,
            NaiveDate::from_ymd_opt(2026, 3, 20).unwrap()
        ));
        assert!(
            !is_halted_period(code, NaiveDate::from_ymd_opt(2026, 3, 11).unwrap()),
            "3/11 是 K 线日, 不是停牌日"
        );
    }
}
