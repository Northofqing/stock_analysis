//! 净值统计 — 从 ledger 和 trades 计算核心指标。

use crate::portfolio::LedgerEntry;

#[derive(Debug, Clone, Default)]
pub struct EquityStats {
    pub total_return_pct: Option<f64>,
    pub annualized_return_pct: Option<f64>,
    pub max_drawdown_pct: Option<f64>,
    pub sharpe_ratio: Option<f64>,
    pub win_rate: Option<f64>,
    pub total_trades: u32,
    pub winning_trades: u32,
    pub avg_win_pct: Option<f64>,
    pub avg_loss_pct: Option<f64>,
    pub profit_factor: Option<f64>,
    /// 日度 VaR(95%)，单位 %（正数表示潜在损失）
    pub var95_pct: Option<f64>,
    /// 日度 CVaR(95%)，单位 %（正数表示尾部期望损失）
    pub cvar95_pct: Option<f64>,
}

/// BR-103：从完整、连续的净值序列计算统计指标。
pub fn compute_stats(curve: &[LedgerEntry]) -> Result<EquityStats, String> {
    if curve.is_empty() {
        return Ok(EquityStats::default());
    }
    for (index, entry) in curve.iter().enumerate() {
        if !entry.total_value.is_finite()
            || !entry.cash.is_finite()
            || !entry.market_value.is_finite()
            || !entry.daily_pnl.is_finite()
            || entry.total_value <= 0.0
            || entry.cash < 0.0
            || entry.market_value < 0.0
        {
            return Err(format!("净值第 {} 行包含非法数值", index + 1));
        }
        let accounted = entry.cash + entry.market_value;
        if (entry.total_value - accounted).abs() > accounted.abs().max(1.0) * 1e-6 {
            return Err(format!(
                "净值 {} 会计恒等式不成立: total={} cash={} market={}",
                entry.date, entry.total_value, entry.cash, entry.market_value
            ));
        }
        if !crate::calendar::is_trading_day(entry.date) {
            return Err(format!("净值日期 {} 不是交易日", entry.date));
        }
    }
    for pair in curve.windows(2) {
        if pair[1].date <= pair[0].date {
            return Err(format!(
                "净值日期无序或重复: {} -> {}",
                pair[0].date, pair[1].date
            ));
        }
        let expected = crate::calendar::next_trading_day(pair[0].date);
        if pair[1].date != expected {
            return Err(format!(
                "净值交易日断档: {} 后应为 {}, 实际为 {}",
                pair[0].date, expected, pair[1].date
            ));
        }
    }

    if curve.len() == 1 {
        return Ok(EquityStats::default());
    }
    let first = &curve[0];
    let last = &curve[curve.len() - 1];
    let total_return = (last.total_value - first.total_value) / first.total_value * 100.0;
    if !total_return.is_finite() || total_return <= -100.0 {
        return Err(format!("净值总收益非法: {total_return}"));
    }

    let days = curve.len() as f64;
    let annualized = ((1.0 + total_return / 100.0).powf(252.0 / days) - 1.0) * 100.0;

    // 最大回撤
    let mut max_dd = 0.0_f64;
    let mut peak = f64::MIN;
    for e in curve {
        if e.total_value > peak {
            peak = e.total_value;
        }
        if peak > 0.0 {
            let dd = (peak - e.total_value) / peak * 100.0;
            if dd > max_dd {
                max_dd = dd;
            }
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
            let variance = daily_returns
                .iter()
                .map(|r| (r - mean).powi(2))
                .sum::<f64>()
                / n;
            let std = variance.sqrt();
            if std > 0.0 {
                Some(mean / std * 252.0_f64.sqrt())
            } else {
                None
            }
        } else {
            None
        }
    };

    // 日度 VaR/CVaR（95%）：对收益分布左尾 5% 估计损失
    let (var95_pct, cvar95_pct) = if daily_returns.len() < 2 {
        (None, None)
    } else {
        let mut sorted = daily_returns.clone();
        sorted.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = sorted.len();
        let idx = (((n as f64) * 0.05).floor() as usize).min(n - 1);
        let q = sorted[idx];
        let tail = &sorted[..=idx];
        let tail_mean = tail.iter().sum::<f64>() / tail.len() as f64;
        (
            Some(q.min(0.0).abs() * 100.0),
            Some(tail_mean.min(0.0).abs() * 100.0),
        )
    };

    Ok(EquityStats {
        total_return_pct: Some(total_return),
        annualized_return_pct: Some(annualized),
        max_drawdown_pct: Some(max_dd),
        sharpe_ratio: sharpe,
        win_rate: None,
        total_trades: 0,
        winning_trades: 0,
        avg_win_pct: None,
        avg_loss_pct: None,
        profit_factor: None,
        var95_pct,
        cvar95_pct,
    })
}

/// 从交易复盘数据补充胜率等指标
pub fn enrich_with_trades(
    stats: &mut EquityStats,
    reviews: &[super::journal::TradeReview],
) -> Result<(), String> {
    stats.total_trades = reviews.len() as u32;
    if reviews.is_empty() {
        return Ok(());
    }
    if let Some(review) = reviews.iter().find(|review| !review.pnl_pct.is_finite()) {
        return Err(format!(
            "交易复盘 {} 的收益率非有限值",
            review.sell_trade_id
        ));
    }

    let wins: Vec<_> = reviews.iter().filter(|r| r.pnl_pct > 0.0).collect();
    let losses: Vec<_> = reviews.iter().filter(|r| r.pnl_pct <= 0.0).collect();

    stats.winning_trades = wins.len() as u32;
    stats.win_rate = Some(stats.winning_trades as f64 / stats.total_trades as f64 * 100.0);

    stats.avg_win_pct = if wins.is_empty() {
        None
    } else {
        Some(wins.iter().map(|r| r.pnl_pct).sum::<f64>() / wins.len() as f64)
    };

    stats.avg_loss_pct = if losses.is_empty() {
        None
    } else {
        Some(losses.iter().map(|r| r.pnl_pct).sum::<f64>() / losses.len() as f64)
    };

    let total_win = wins.iter().map(|r| r.pnl_pct).sum::<f64>();
    let total_loss = losses.iter().map(|r| r.pnl_pct.abs()).sum::<f64>();
    stats.profit_factor = if total_loss > 0.0 {
        Some(total_win / total_loss)
    } else {
        None
    };
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn date(d: &str) -> NaiveDate {
        NaiveDate::parse_from_str(d, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn test_max_drawdown() {
        let curve = vec![
            LedgerEntry {
                date: date("2026-01-05"),
                total_value: 100_000.0,
                cash: 0.0,
                market_value: 100_000.0,
                daily_pnl: 0.0,
            },
            LedgerEntry {
                date: date("2026-01-06"),
                total_value: 110_000.0,
                cash: 0.0,
                market_value: 110_000.0,
                daily_pnl: 10_000.0,
            },
            LedgerEntry {
                date: date("2026-01-07"),
                total_value: 90_000.0,
                cash: 0.0,
                market_value: 90_000.0,
                daily_pnl: -20_000.0,
            },
            LedgerEntry {
                date: date("2026-01-08"),
                total_value: 105_000.0,
                cash: 0.0,
                market_value: 105_000.0,
                daily_pnl: 15_000.0,
            },
        ];
        let stats = compute_stats(&curve).unwrap();
        // max drawdown: from 110k peak to 90k = (110-90)/110 ≈ 18.18%
        assert!((stats.max_drawdown_pct.unwrap() - 18.18).abs() < 0.1);
    }

    #[test]
    fn test_sharpe_positive() {
        let mut curve = Vec::new();
        let mut val = 100_000.0;
        let mut trading_date = date("2026-01-05");
        for i in 0..20 {
            let rate = if i % 2 == 0 { 1.004 } else { 1.006 };
            val *= rate;
            curve.push(LedgerEntry {
                date: trading_date,
                total_value: val,
                cash: 0.0,
                market_value: val,
                daily_pnl: val * (rate - 1.0),
            });
            trading_date = crate::calendar::next_trading_day(trading_date);
        }
        let stats = compute_stats(&curve).unwrap();
        // consistent positive returns → positive sharpe
        assert!(stats.sharpe_ratio.unwrap() > 0.0);
    }

    #[test]
    fn br103_empty_and_single_point_metrics_are_unavailable() {
        assert!(compute_stats(&[]).unwrap().total_return_pct.is_none());
        let single = LedgerEntry {
            date: date("2026-01-05"),
            total_value: 100_000.0,
            cash: 50_000.0,
            market_value: 50_000.0,
            daily_pnl: 0.0,
        };
        let stats = compute_stats(&[single]).unwrap();
        assert!(stats.total_return_pct.is_none());
        assert!(stats.sharpe_ratio.is_none());
        assert!(stats.var95_pct.is_none());
    }
}
