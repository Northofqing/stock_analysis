//! 从 K 线数据派生价格统计：52 周区间、季度区间、近期涨跌幅与波动率。

use crate::data_provider::KlineData;

/// 聚合价格统计（入参按时间倒序，`data[0]` 为最新）。
pub(super) struct PriceStats {
    pub high_52w: Option<f64>,
    pub low_52w: Option<f64>,
    pub pos_52w: Option<f64>,
    pub high_quarter: Option<f64>,
    pub low_quarter: Option<f64>,
    pub pos_quarter: Option<f64>,
    pub chg_5d: Option<f64>,
    pub chg_10d: Option<f64>,
    pub volatility: Option<f64>,
}

pub(super) fn compute_price_stats(data: &[KlineData]) -> PriceStats {
    let n = data.len();

    let range = |window: usize| -> (Option<f64>, Option<f64>, Option<f64>) {
        let w = n.min(window);
        if w < 5 {
            return (None, None, None);
        }
        let h = data[..w]
            .iter()
            .map(|k| k.high)
            .fold(f64::NEG_INFINITY, f64::max);
        let l = data[..w]
            .iter()
            .map(|k| k.low)
            .fold(f64::INFINITY, f64::min);
        let pos = if (h - l).abs() > 0.001 {
            (data[0].close - l) / (h - l) * 100.0
        } else {
            50.0
        };
        (Some(h), Some(l), Some(pos))
    };

    let (high_52w, low_52w, pos_52w) = range(250);
    let (high_quarter, low_quarter, pos_quarter) = range(60);

    let chg_5d = if n >= 2 {
        Some(data[..n.min(5)].iter().map(|k| k.pct_chg).sum())
    } else {
        None
    };
    let chg_10d = if n >= 10 {
        Some(data[..10].iter().map(|k| k.pct_chg).sum())
    } else {
        None
    };
    let volatility = if n >= 5 {
        let recent = n.min(10);
        let returns: Vec<f64> = data[..recent].iter().map(|k| k.pct_chg).collect();
        let mean = returns.iter().sum::<f64>() / returns.len() as f64;
        let var = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / returns.len() as f64;
        Some(var.sqrt())
    } else {
        None
    };

    PriceStats {
        high_52w,
        low_52w,
        pos_52w,
        high_quarter,
        low_quarter,
        pos_quarter,
        chg_5d,
        chg_10d,
        volatility,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_provider::AdjustType;
    use chrono::{Duration, NaiveDate};

    fn kline(index: i64, close: f64, high: f64, low: f64, pct_chg: f64) -> KlineData {
        KlineData {
            date: NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid fixture date")
                - Duration::days(index),
            open: close,
            high,
            low,
            close,
            volume: 1_000.0,
            amount: close * 1_000.0,
            pct_chg,
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
        }
    }

    #[test]
    fn empty_history_has_no_price_statistics() {
        let stats = compute_price_stats(&[]);

        assert_eq!(stats.high_52w, None);
        assert_eq!(stats.low_52w, None);
        assert_eq!(stats.pos_52w, None);
        assert_eq!(stats.high_quarter, None);
        assert_eq!(stats.low_quarter, None);
        assert_eq!(stats.pos_quarter, None);
        assert_eq!(stats.chg_5d, None);
        assert_eq!(stats.chg_10d, None);
        assert_eq!(stats.volatility, None);
    }

    #[test]
    fn short_history_only_reports_available_cumulative_change() {
        let data = vec![
            kline(0, 10.0, 10.5, 9.5, 1.0),
            kline(1, 9.9, 10.0, 9.0, 2.0),
            kline(2, 9.8, 10.0, 9.0, 3.0),
            kline(3, 9.7, 10.0, 9.0, 4.0),
        ];

        let stats = compute_price_stats(&data);

        assert_eq!(stats.high_52w, None);
        assert_eq!(stats.high_quarter, None);
        assert_eq!(stats.chg_5d, Some(10.0));
        assert_eq!(stats.chg_10d, None);
        assert_eq!(stats.volatility, None);
    }

    #[test]
    fn ten_day_history_reports_ranges_changes_and_population_volatility() {
        let data: Vec<KlineData> = (0..10)
            .map(|index| {
                let i = index as f64;
                kline(index, 15.0 - i * 0.5, 20.0 - i, 10.0 - i, i + 1.0)
            })
            .collect();

        let stats = compute_price_stats(&data);

        assert_eq!(stats.high_52w, Some(20.0));
        assert_eq!(stats.low_52w, Some(1.0));
        assert_eq!(stats.high_quarter, Some(20.0));
        assert_eq!(stats.low_quarter, Some(1.0));
        assert!((stats.pos_52w.expect("52-week position") - 73.684_210_526).abs() < 1e-9);
        assert!((stats.pos_quarter.expect("quarter position") - 73.684_210_526).abs() < 1e-9);
        assert_eq!(stats.chg_5d, Some(15.0));
        assert_eq!(stats.chg_10d, Some(55.0));
        assert!((stats.volatility.expect("volatility") - 2.872_281_323).abs() < 1e-9);
    }

    #[test]
    fn flat_five_day_range_uses_midpoint_and_zero_volatility() {
        let data: Vec<KlineData> = (0..5)
            .map(|index| kline(index, 10.0, 10.0, 10.0, 0.0))
            .collect();

        let stats = compute_price_stats(&data);

        assert_eq!(stats.pos_52w, Some(50.0));
        assert_eq!(stats.pos_quarter, Some(50.0));
        assert_eq!(stats.chg_5d, Some(0.0));
        assert_eq!(stats.chg_10d, None);
        assert_eq!(stats.volatility, Some(0.0));
    }
}
