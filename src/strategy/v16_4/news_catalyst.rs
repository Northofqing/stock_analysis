//! v16.4 #2: NewsCatalystStrategy — 新闻/公告催化 (D-01 推送, score 7.0)
//!
//! Fix 2 (review): 真读 push_price + price_chg_pct 算 base_score.
//! 公式: 7.0 (base) + min(price_chg_pct, 0) * 0.3 (低吸偏好, 跌越多分越高)
//! 阈值: price_chg_pct < -2% 加 0.5 加速 (急跌催化)
use super::{Strategy, StrategyInput, StrategyOutput};
use crate::impl_strategy_id;

pub struct NewsCatalystStrategy;

impl Strategy for NewsCatalystStrategy {
    impl_strategy_id!(NewsCatalystStrategy, "NewsCatalyst");
    fn virtual_reason(&self) -> &'static str {
        "NewsCatalyst"
    }
    fn description(&self) -> &'static str {
        "新闻/公告催化 (D-01 推送)"
    }
    fn score(&self, input: &StrategyInput) -> Option<StrategyOutput> {
        if input.push_kind != "D-01" {
            return None;
        }
        let metrics =
            super::_helpers::parse(&input.metric_json, &input.code, input.push_price).ok()?;
        // Fix 2: 真读 price_chg_pct 算 score
        let mut score: f64 = 7.0;
        if let Some(change_pct) = metrics.price_chg_pct {
            if change_pct < 0.0 {
                score += (-change_pct).min(2.0) * 0.3;
            }
            if change_pct < -2.0 {
                score += 0.5;
            }
        }
        score = score.min(9.0);
        let change = metrics
            .price_chg_pct
            .map(|value| format!("{value:.1}%"))
            .unwrap_or_else(|| "暂无".to_string());
        Some(StrategyOutput {
            score,
            reason: format!("新闻驱动 chg={change}"),
            virtual_reason: "NewsCatalyst".into(),
        })
    }
}
