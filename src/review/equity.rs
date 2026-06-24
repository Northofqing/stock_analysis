//! 净值统计 — 从 ledger 和 trades 计算核心指标。

use crate::portfolio::LedgerEntry;

#[derive(Debug, Clone)]
pub struct EquityStats {
    pub total_return_pct: f64,
    pub annualized_return_pct: f64,
    pub max_drawdown_pct: f64,
    pub sharpe_ratio: f64,
    pub win_rate: f64,
    pub total_trades: u32,
    pub winning_trades: u32,
    pub avg_win_pct: f64,
    pub avg_loss_pct: f64,
    pub profit_factor: f64,
    /// 日度 VaR(95%)，单位 %（正数表示潜在损失）
    pub var95_pct: f64,
    /// 日度 CVaR(95%)，单位 %（正数表示尾部期望损失）
    pub cvar95_pct: f64,
}

/// 从净值序列计算统计指标
pub fn compute_stats(curve: &[LedgerEntry]) -> EquityStats {
    let total_return = if let (Some(first), Some(last)) = (curve.first(), curve.last()) {
        if first.total_value > 0.0 {
            (last.total_value - first.total_value) / first.total_value * 100.0
        } else { 0.0 }
    } else { 0.0 };

    let days = curve.len().max(1) as f64;
    let annualized = if days > 1.0 {
        ((1.0 + total_return / 100.0).powf(252.0 / days) - 1.0) * 100.0
    } else { 0.0 };

    // 最大回撤
    let mut max_dd = 0.0_f64;
    let mut peak = f64::MIN;
    for e in curve {
        if e.total_value > peak { peak = e.total_value; }
        if peak > 0.0 {
            let dd = (peak - e.total_value) / peak * 100.0;
            if dd > max_dd { max_dd = dd; }
        }
    }

    let mut daily_returns = Vec::new();
    if curve.len() >= 2 {
        for w in curve.windows(2) {
            let prev = w[0].total_value;
            let curr = w[1].total_value;
            if prev > 0.0 {
                daily_returns.push((curr - prev) / prev);
            }
        }
    }

    // 夏普（简化：日收益率 std * sqrt(252)）
    let sharpe = {
        let n = daily_returns.len() as f64;
        if n > 0.0 {
            let mean = daily_returns.iter().sum::<f64>() / n;
            let variance = daily_returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / n;
            let std = variance.sqrt();
            if std > 0.0 { mean / std * 252.0_f64.sqrt() } else { 0.0 }
        } else { 0.0 }
    };

    // 日度 VaR/CVaR（95%）：对收益分布左尾 5% 估计损失
    let (var95_pct, cvar95_pct) = if daily_returns.is_empty() {
        (0.0, 0.0)
    } else {
        let mut sorted = daily_returns.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = sorted.len();
        let idx = (((n as f64) * 0.05).floor() as usize).min(n - 1);
        let q = sorted[idx];
        let tail = &sorted[..=idx];
        let tail_mean = tail.iter().sum::<f64>() / tail.len() as f64;
        (q.min(0.0).abs() * 100.0, tail_mean.min(0.0).abs() * 100.0)
    };

    EquityStats {
        total_return_pct: total_return,
        annualized_return_pct: annualized,
        max_drawdown_pct: max_dd,
        sharpe_ratio: sharpe,
        win_rate: 0.0,      // 由外部填充
        total_trades: 0,
        winning_trades: 0,
        avg_win_pct: 0.0,
        avg_loss_pct: 0.0,
        profit_factor: 0.0,
        var95_pct,
        cvar95_pct,
    }
}

/// 从交易复盘数据补充胜率等指标
pub fn enrich_with_trades(stats: &mut EquityStats, reviews: &[super::journal::TradeReview]) {
    stats.total_trades = reviews.len() as u32;
    if reviews.is_empty() { return; }

    let wins: Vec<_> = reviews.iter().filter(|r| r.pnl_pct > 0.0).collect();
    let losses: Vec<_> = reviews.iter().filter(|r| r.pnl_pct <= 0.0).collect();

    stats.winning_trades = wins.len() as u32;
    stats.win_rate = if stats.total_trades > 0 {
        stats.winning_trades as f64 / stats.total_trades as f64 * 100.0
    } else { 0.0 };

    stats.avg_win_pct = if !wins.is_empty() {
        wins.iter().map(|r| r.pnl_pct).sum::<f64>() / wins.len() as f64
    } else { 0.0 };

    stats.avg_loss_pct = if !losses.is_empty() {
        losses.iter().map(|r| r.pnl_pct).sum::<f64>() / losses.len() as f64
    } else { 0.0 };

    let total_win = wins.iter().map(|r| r.pnl_pct).sum::<f64>();
    let total_loss = losses.iter().map(|r| r.pnl_pct.abs()).sum::<f64>();
    stats.profit_factor = if total_loss > 0.0 { total_win / total_loss } else { 0.0 };
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn date(d: &str) -> NaiveDate { NaiveDate::parse_from_str(d, "%Y-%m-%d").unwrap() }

    #[test]
    fn test_max_drawdown() {
        let curve = vec![
            LedgerEntry { date: date("2026-01-01"), total_value: 100_000.0, cash: 0.0, market_value: 100_000.0, daily_pnl: 0.0 },
            LedgerEntry { date: date("2026-01-02"), total_value: 110_000.0, cash: 0.0, market_value: 110_000.0, daily_pnl: 10_000.0 },
            LedgerEntry { date: date("2026-01-03"), total_value: 90_000.0, cash: 0.0, market_value: 90_000.0, daily_pnl: -20_000.0 },
            LedgerEntry { date: date("2026-01-04"), total_value: 105_000.0, cash: 0.0, market_value: 105_000.0, daily_pnl: 15_000.0 },
        ];
        let stats = compute_stats(&curve);
        // max drawdown: from 110k peak to 90k = (110-90)/110 ≈ 18.18%
        assert!((stats.max_drawdown_pct - 18.18).abs() < 0.1);
    }

    #[test]
    fn test_sharpe_positive() {
        let mut curve = Vec::new();
        let mut val = 100_000.0;
        for i in 1..=20 {
            val *= 1.005; // 0.5% daily return
            curve.push(LedgerEntry {
                date: date("2026-01-01") + chrono::Duration::days(i),
                total_value: val, cash: 0.0, market_value: val, daily_pnl: val * 0.005,
            });
        }
        let stats = compute_stats(&curve);
        // consistent positive returns → positive sharpe
        assert!(stats.sharpe_ratio > 0.0);
    }
}
