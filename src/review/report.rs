//! 复盘报告格式化 — 生成微信推送文本。

use crate::portfolio::Position;
use super::journal::TradeReview;
use super::equity::EquityStats;

/// 生成每日复盘报告。`prices` 为 code→当前价 映射，用于计算持仓浮盈。
pub fn generate_daily_report(
    reviews: &[TradeReview],
    stats: &EquityStats,
    holdings: &[Position],
    prices: &std::collections::HashMap<String, f64>,
) -> String {
    generate_daily_report_with_ledger(reviews, stats, holdings, prices, None)
}

/// 修复 B-005: 可选传 ledger 数据计算 rolling Sharpe
pub fn generate_daily_report_with_ledger(
    reviews: &[TradeReview],
    stats: &EquityStats,
    holdings: &[Position],
    prices: &std::collections::HashMap<String, f64>,
    ledger: Option<&[crate::portfolio::LedgerEntry]>,
) -> String {
    let today = chrono::Local::now().format("%Y-%m-%d");
    let mut lines: Vec<String> = Vec::new();

    // ── 标题 ──
    lines.push(format!("📊 交易复盘 {}", today));
    lines.push("━━━━━━━━━━━━━━━━━━━━━━━━".to_string());

    // ── 净值概览 ──
    lines.push(format!(
        "📈 累计收益：{:+.1}% | 年化：{:+.1}% | 最大回撤：{:.1}% | 夏普：{:.2}",
        stats.total_return_pct,
        stats.annualized_return_pct,
        stats.max_drawdown_pct,
        stats.sharpe_ratio,
    ));
    lines.push(format!(
        "🛡️ 风险值：VaR95={:.2}% | CVaR95={:.2}%（日度）",
        stats.var95_pct,
        stats.cvar95_pct,
    ));

    // ── 交易统计 ──
    if stats.total_trades > 0 {
        lines.push(format!(
            "🎯 胜率：{:.0}% ({}/{}) | 盈亏比：{:.1} | 均盈：{:+.1}% | 均亏：{:.1}%",
            stats.win_rate,
            stats.winning_trades,
            stats.total_trades,
            stats.profit_factor,
            stats.avg_win_pct,
            stats.avg_loss_pct,
        ));
    } else {
        lines.push("🎯 暂无已平仓交易记录".to_string());
    }

    // ── 已平仓复盘 ──
    if !reviews.is_empty() {
        lines.push(String::new());
        lines.push("📝 已平仓复盘：".to_string());
        for r in reviews {
            let emoji = if r.pnl_pct > 0.0 { "🟢" } else { "🔴" };
            let mut detail = format!(
                "  {} {}({}) 持{}天 {:+.1}%  ¥{:.2}→¥{:.2}",
                emoji, r.name, r.code,
                r.holding_days, r.pnl_pct,
                r.buy_price, r.sell_price,
            );

            // 卖出后走势
            if let Some(chg5) = r.post_exit_chg_5d {
                detail.push_str(&format!("  后5日：{:+.1}%", chg5));
            }
            if let Some(chg20) = r.post_exit_chg_20d {
                detail.push_str(&format!("  后20日：{:+.1}%", chg20));
            }
            lines.push(detail);

            // 自评
            if let Some(rating) = r.self_rating {
                let stars = "⭐".repeat(rating as usize);
                lines.push(format!("    自评：{}/5 {}", rating, stars));
            }
            if let Some(ref lesson) = r.lesson {
                if !lesson.is_empty() {
                    lines.push(format!("    教训：{}", lesson));
                }
            }
        }
    }

    // ── 持仓中 ──
    if !holdings.is_empty() {
        lines.push(String::new());
        lines.push("📌 持仓中：".to_string());
        let today = chrono::Local::now().date_naive();
        for p in holdings {
            let holding_days = (today - p.added_at).num_days().max(0);
            // v13.10.1 P0-#4: 拉不到实时价时显式标注 "数据不足", 避免误导 (之前用 cost_price 顶替导致浮盈 0.0% 看起来像"持仓未动")
            let (price, price_note) = match prices.get(&p.code).copied() {
                Some(v) if (v - p.cost_price).abs() > 0.001 || v == 0.0 => (v, String::new()),
                _ => (p.cost_price, " 数据不足".to_string()),
            };
            let pnl_pct = if p.cost_price > 0.0 { (price - p.cost_price) / p.cost_price * 100.0 } else { 0.0 };
            let emoji = if pnl_pct > 0.0 { "🔺" } else if pnl_pct < -5.0 { "🔻" } else { "→" };
            lines.push(format!(
                "  {} {}({}) 持{}天 {:+.1}%  ¥{:.2}→¥{:.2}  {}股{}",
                emoji, p.name, p.code, holding_days, pnl_pct, p.cost_price, price, p.shares, price_note,
            ));
        }
    }

    // 修复 B-005: 追加 live rolling Sharpe (如有 ledger 数据)
    if let Some(ledger) = ledger {
        if !ledger.is_empty() {
            lines.push(String::new());
            lines.push("📈 滚动风险指标".to_string());
            if let Some(s30) = crate::portfolio::live_rolling_sharpe(ledger, 30) {
                lines.push(format!("  - 30 日滚动 Sharpe: {:+.2}", s30));
            } else {
                lines.push("  - 30 日滚动 Sharpe: 样本不足 (需 ≥ 30 日)".to_string());
            }
            if let Some(s60) = crate::portfolio::live_rolling_sharpe(ledger, 60) {
                lines.push(format!("  - 60 日滚动 Sharpe: {:+.2}", s60));
            } else {
                lines.push("  - 60 日滚动 Sharpe: 样本不足 (需 ≥ 30 日)".to_string());
            }
        }
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use crate::portfolio::{Position, PositionStatus};
    use super::super::journal::TradeReview;

    fn date(d: &str) -> NaiveDate { NaiveDate::parse_from_str(d, "%Y-%m-%d").unwrap() }
    fn ndt(d: &str) -> chrono::NaiveDateTime {
        chrono::NaiveDateTime::parse_from_str(&format!("{} 10:00:00", d), "%Y-%m-%d %H:%M:%S").unwrap()
    }

    #[test]
    fn test_report_formatting() {
        let reviews = vec![TradeReview {
            code: "000547".into(), name: "航天发展".into(),
            buy_date: date("2026-06-01"), sell_date: date("2026-06-10"),
            buy_datetime: ndt("2026-06-01"), sell_datetime: ndt("2026-06-10"),
            buy_price: 10.0, sell_price: 12.0, holding_days: 9,
            pnl_pct: 20.0, post_exit_chg_5d: Some(2.1),
            post_exit_chg_20d: None, self_rating: Some(4),
            lesson: Some("买点可再等一天".into()),
        }];

        let stats = EquityStats {
            total_return_pct: 8.4, annualized_return_pct: 12.0,
            max_drawdown_pct: 5.2, sharpe_ratio: 0.92,
            win_rate: 58.0, total_trades: 12, winning_trades: 7,
            avg_win_pct: 8.2, avg_loss_pct: -4.1, profit_factor: 2.1,
            var95_pct: 1.8, cvar95_pct: 2.4,
        };

        let holdings = vec![Position {
            code: "603618".into(), name: "杭电股份".into(),
            shares: 1000, cost_price: 8.2, hard_stop: 7.5,
            added_at: date("2026-06-05"), status: PositionStatus::Holding,
            sector: "其他".into(), ..Default::default()
        }];

        let mut prices = std::collections::HashMap::new();
        prices.insert("603618".to_string(), 9.0_f64);
        let report = generate_daily_report(&reviews, &stats, &holdings, &prices);
        assert!(report.contains("交易复盘"));
        assert!(report.contains("航天发展"));
        assert!(report.contains("+20.0%"));
        assert!(report.contains("杭电股份"));
        assert!(report.contains("+9.8%"));
        assert!(report.contains("胜率：58%"));
        assert!(report.contains("VaR95"));
    }
}
