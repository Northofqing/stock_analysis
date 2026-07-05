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

/// R-05 全版渲染
pub fn render_r05_full(stats: &SignalReviewStats) -> String {
    let mut s = String::new();
    s.push_str(&format!("🤖 信号复盘（{}）\n", Local::now().format("%Y-%m-%d")));
    s.push_str(&format!(
        "持仓建议: 推{}条 执行{}条 有效{}条\n",
        stats.holding_recommendations_pushed,
        stats.holding_recommendations_executed,
        stats.holding_recommendations_effective,
    ));
    s.push_str(&format!(
        "做T建议: 推{} 有效{} [MVP-2起]\n",
        stats.t0_recommendations_pushed,
        stats.t0_recommendations_effective,
    ));
    s.push_str(&format!(
        "候选(影子): 触发{} 模拟成交{} 未成交{}（涨停{}/未触达{}）\n",
        stats.candidate_shadow_triggered,
        stats.candidate_shadow_filled,
        stats.candidate_shadow_not_filled,
        stats.candidate_shadow_limit_up,
        stats.candidate_shadow_not_reached,
    ));
    s.push_str(&format!(
        "虚拟盘: 今日{:+.1}% 累计{:+.1}%（样本{}笔）\n",
        stats.paper_today_pnl_pct,
        stats.paper_total_pnl_pct,
        stats.paper_sample_count,
    ));
    s.push_str(&format!(
        "新闻兑现: 推送{}条 D+1兑现{}条\n",
        stats.news_pushed,
        stats.news_d1_realized,
    ));
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
        assert!(s.contains("持仓建议: 推10条 执行7条 有效5条"));
        assert!(s.contains("做T建议: 推3 有效2"));
        assert!(s.contains("候选(影子): 触发8 模拟成交5 未成交3"));
        assert!(s.contains("虚拟盘: 今日+0.5% 累计+2.3%"));
        assert!(s.contains("新闻兑现: 推送20条 D+1兑现12条"));
    }

    #[test]
    fn render_zero_stats() {
        let s = render_r05_full(&SignalReviewStats::default());
        assert!(s.contains("推0条"));
    }
}