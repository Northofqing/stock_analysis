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
        let h = data[..w].iter().map(|k| k.high).fold(f64::NEG_INFINITY, f64::max);
        let l = data[..w].iter().map(|k| k.low).fold(f64::INFINITY, f64::min);
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
