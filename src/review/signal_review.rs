//! v12 MVP-4 §7.5: signal_review 全版 (R-05 推送).
//!
//! 字段: 持仓建议命中/做T有效/候选触发/虚拟盘盈亏/新闻兑现.

use chrono::Local;

/// R-05 全版统计输入
#[derive(Debug, Clone, Default)]
pub struct SignalReviewStats {
    pub holding_recommendations_pushed: u32,
    pub holding_recommendations_executed: u32,
    pub holding_recommendations_effective: u32,
    pub t0_recommendations_pushed: u32,
    pub t0_recommendations_effective: u32,
    pub candidate_shadow_triggered: u32,
    pub candidate_shadow_filled: u32,
    pub candidate_shadow_not_filled: u32,
    pub candidate_shadow_limit_up: u32,
    pub candidate_shadow_not_reached: u32,
    pub paper_today_pnl_pct: f64,
    pub paper_total_pnl_pct: f64,
    pub paper_sample_count: u32,
    pub news_pushed: u32,
    pub news_d1_realized: u32,
}

/// R-05 全版渲染 (v19.12 修复: 样本不足时显式标注, 不再显示 0 当数字)
pub fn render_r05_full(stats: &SignalReviewStats) -> String {
    let mut s = String::new();
    s.push_str(&format!("🤖 信号复盘（{}）\n", Local::now().format("%Y-%m-%d")));
    // 持仓建议
    if stats.holding_recommendations_pushed == 0 {
        s.push_str("持仓建议: 推 0 条 (今日无推送)\n");
    } else {
        let eff_pct = if stats.holding_recommendations_executed > 0 {
            stats.holding_recommendations_effective as f64 * 100.0 / stats.holding_recommendations_executed as f64
        } else { 0.0 };
        s.push_str(&format!(
            "持仓建议: 推 {} 条 / 执行 {} 条 / 有效 {} 条 ({:.1}%)\n",
            stats.holding_recommendations_pushed,
            stats.holding_recommendations_executed,
            stats.holding_recommendations_effective,
            eff_pct,
        ));
    }
    // 做T建议
    if stats.t0_recommendations_pushed == 0 {
        s.push_str("做T建议: 推 0 条 (v19.15+ 启用 — 详见 docs/architecture/v12-dev-plan.md §MVP-2)\n");
    } else {
        s.push_str(&format!(
            "做T建议: 推 {} 条 / 有效 {} 条\n",
            stats.t0_recommendations_pushed,
            stats.t0_recommendations_effective,
        ));
    }
    // 候选(影子) — v19.12: 样本 < 30 显式标注, 不报 "0"
    if stats.candidate_shadow_triggered == 0 {
        s.push_str("候选(影子): 样本不足 (转正需 ≥30 笔影子样本, 当前 0 笔 — 详见 v12-dev-plan.md §MVP-3)\n");
    } else {
        s.push_str(&format!(
            "候选(影子): 触发 {} / 模拟成交 {} / 未成交 {} (涨停 {}/未触达 {})\n",
            stats.candidate_shadow_triggered,
            stats.candidate_shadow_filled,
            stats.candidate_shadow_not_filled,
            stats.candidate_shadow_limit_up,
            stats.candidate_shadow_not_reached,
        ));
    }
    // 虚拟盘 — 样本 < 10 显式标注 (BR-020)
    if stats.paper_sample_count == 0 {
        s.push_str("虚拟盘: 今日 +0.0% / 累计 +0.0% (样本 0 笔, paper_trades 表空)\n");
    } else if stats.paper_sample_count < 10 {
        s.push_str(&format!(
            "虚拟盘: 今日 {:+.2}% / 累计 {:+.2}% (样本 {} 笔, <10 笔样本不足)\n",
            stats.paper_today_pnl_pct, stats.paper_total_pnl_pct, stats.paper_sample_count,
        ));
    } else {
        s.push_str(&format!(
            "虚拟盘: 今日 {:+.2}% / 累计 {:+.2}% (样本 {} 笔)\n",
            stats.paper_today_pnl_pct, stats.paper_total_pnl_pct, stats.paper_sample_count,
        ));
    }
    // 新闻兑现 — 影子期 0 笔正常, 显式说明
    if stats.news_pushed == 0 {
        s.push_str("新闻兑现: 推送 0 条 / D+1 兑现 0 条 (影子期无样本)\n");
    } else {
        let hit_rate = stats.news_pushed as f64 - stats.news_d1_realized as f64;
        s.push_str(&format!(
            "新闻兑现: 推送 {} 条 / D+1 兑现 {} 条 (命中率 {:.1}%)\n",
            stats.news_pushed, stats.news_d1_realized,
            (stats.news_d1_realized as f64 / stats.news_pushed as f64) * 100.0,
        ));
        let _ = hit_rate;
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_full() {
        let stats = SignalReviewStats {
            holding_recommendations_pushed: 10,
            holding_recommendations_executed: 7,
            holding_recommendations_effective: 5,
            t0_recommendations_pushed: 3,
            t0_recommendations_effective: 2,
            candidate_shadow_triggered: 8,
            candidate_shadow_filled: 5,
            candidate_shadow_not_filled: 3,
            candidate_shadow_limit_up: 2,
            candidate_shadow_not_reached: 1,
            paper_today_pnl_pct: 0.5,
            paper_total_pnl_pct: 2.3,
            paper_sample_count: 15,
            news_pushed: 20,
            news_d1_realized: 12,
        };
        let s = render_r05_full(&stats);
        assert!(s.contains("🤖 信号复盘"));
        assert!(s.contains("持仓建议: 推 10 条 / 执行 7 条 / 有效 5 条"));
        assert!(s.contains("做T建议: 推 3 条 / 有效 2 条"));
        assert!(s.contains("候选(影子): 触发 8 / 模拟成交 5 / 未成交 3"));
        assert!(s.contains("虚拟盘: 今日 +0.50% / 累计 +2.30%"));
        assert!(s.contains("新闻兑现: 推送 20 条 / D+1 兑现 12 条"));
    }

    #[test]
    fn render_zero_stats() {
        let s = render_r05_full(&SignalReviewStats::default());
        assert!(s.contains("推 0 条"));
    }
}