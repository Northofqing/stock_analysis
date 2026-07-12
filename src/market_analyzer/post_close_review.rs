//! v12 MVP4-4.4: 盘后接通 (R-05/R-07/R-08).
//!
//! 设计: 把 push_templates 已有模板的渲染数据从 main 循环抽出来.
//!       数据来源:
//!         - R-05: prediction_tracker (已有) + execution_tracking (PR3 新增)
//!         - R-07: news_audit + candidate_state.shadow_rank_hits
//!         - R-08: announcement 公告 + 经济日历 (轻量版)

use serde::{Deserialize, Serialize};

/// R-05 信号复盘
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SignalReviewInput {
    pub date: String,
    pub holding_n: u32,
    pub holding_exec: u32,
    pub holding_eff: u32,
    pub t0_n: u32,
    pub t0_eff: u32,
    pub cand_trigger: u32,
    pub cand_filled: u32,
    pub cand_notfilled: u32,
    pub cand_limitup: u32,
    pub cand_notreach: u32,
    pub paper_pnl_pct: f64,
    pub paper_total_pct: f64,
    pub paper_n: u32,
    pub news_push_n: u32,
    pub news_d1_eff: u32,
}

/// R-07 明日观察池项
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WatchItemInput {
    pub name: String,
    pub code: String,
    pub topic: String,
    pub source: String, // "A档未触发" / "龙虎榜" / "涨停链" / "持仓做T"
    pub trigger: String,
    pub lo: f64,
    pub hi: f64,
    pub stop: f64,
    pub reason: String,
}

/// R-08 事件日历
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HoldingEventItem {
    pub name: String,
    pub kind: String, // "解禁{n}亿" / "财报预告" / "减持到期"
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventCalendarInput {
    pub date: String,
    pub holdings: Vec<HoldingEventItem>,
    pub macro_events: String,
    pub us_chg: String,
    pub fx: String,
}

/// MVP4-4.4: R-05 数据聚合 (从 prediction_tracker + paper_trades)
pub fn aggregate_signal_review(
    holding: (u32, u32, u32),             // (推, 执行, 有效)
    t0: (u32, u32),                       // (推, 有效)
    candidate: (u32, u32, u32, u32, u32), // (触发, 成交, 未成交, 涨停, 未触达)
    paper: (f64, f64, u32),               // (今日pnl%, 累计%, 样本)
    news: (u32, u32),                     // (推送, D+1兑现)
    date: String,
) -> SignalReviewInput {
    SignalReviewInput {
        date,
        holding_n: holding.0,
        holding_exec: holding.1,
        holding_eff: holding.2,
        t0_n: t0.0,
        t0_eff: t0.1,
        cand_trigger: candidate.0,
        cand_filled: candidate.1,
        cand_notfilled: candidate.2,
        cand_limitup: candidate.3,
        cand_notreach: candidate.4,
        paper_pnl_pct: paper.0,
        paper_total_pct: paper.1,
        paper_n: paper.2,
        news_push_n: news.0,
        news_d1_eff: news.1,
    }
}

/// MVP4-4.4: R-05 → 模板字段转换 (由 bin/monitor 侧构造 push_templates::SignalReview)
/// 本函数返回结构化字段, 跨 crate 边界友好.
pub fn signal_review_to_template_fields(inp: &SignalReviewInput) -> SignalReviewFields {
    SignalReviewFields {
        holding_n: inp.holding_n,
        holding_exec: inp.holding_exec,
        holding_eff: inp.holding_eff,
        t0_n: inp.t0_n,
        t0_eff: inp.t0_eff,
        cand_trigger: inp.cand_trigger,
        cand_filled: inp.cand_filled,
        cand_notfilled: inp.cand_notfilled,
        cand_limitup: inp.cand_limitup,
        cand_notreach: inp.cand_notreach,
        paper_pnl_pct: inp.paper_pnl_pct,
        paper_total_pct: inp.paper_total_pct,
        paper_n: inp.paper_n,
        news_push_n: inp.news_push_n,
        news_d1_eff: inp.news_d1_eff,
    }
}

/// 字段直通结构 (供 bin/monitor 拼 push_templates::SignalReview)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalReviewFields {
    pub holding_n: u32,
    pub holding_exec: u32,
    pub holding_eff: u32,
    pub t0_n: u32,
    pub t0_eff: u32,
    pub cand_trigger: u32,
    pub cand_filled: u32,
    pub cand_notfilled: u32,
    pub cand_limitup: u32,
    pub cand_notreach: u32,
    pub paper_pnl_pct: f64,
    pub paper_total_pct: f64,
    pub paper_n: u32,
    pub news_push_n: u32,
    pub news_d1_eff: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_review_aggregate() {
        let r = aggregate_signal_review(
            (5, 4, 3),       // holding
            (2, 1),          // t0
            (6, 3, 3, 2, 1), // candidate
            (0.5, 3.2, 12),  // paper
            (4, 2),          // news
            "2026-07-05".to_string(),
        );
        assert_eq!(r.date, "2026-07-05");
        assert_eq!(r.holding_n, 5);
        assert_eq!(r.cand_trigger, 6);
        assert_eq!(r.paper_n, 12);
    }

    #[test]
    fn signal_review_to_template_basic() {
        let r = aggregate_signal_review(
            (5, 4, 3),
            (2, 1),
            (6, 3, 3, 2, 1),
            (0.5, 3.2, 12),
            (4, 2),
            "2026-07-05".to_string(),
        );
        let t = signal_review_to_template_fields(&r);
        assert_eq!(t.holding_n, 5);
        assert_eq!(t.cand_limitup, 2);
    }

    #[test]
    fn signal_review_all_zero() {
        let r = aggregate_signal_review(
            (0, 0, 0),
            (0, 0),
            (0, 0, 0, 0, 0),
            (0.0, 0.0, 0),
            (0, 0),
            "2026-07-05".to_string(),
        );
        assert_eq!(r.holding_n, 0);
        assert_eq!(r.paper_n, 0);
    }
}
