//! v16.4 #2: NewsCatalystStrategy — 新闻/公告催化 (D-01 推送, score 7.0)

use super::{Strategy, StrategyInput, StrategyOutput};

pub struct NewsCatalystStrategy;

impl Strategy for NewsCatalystStrategy {
    fn id(&self) -> crate::bus::StrategyId { crate::bus::new_strategy_id("NewsCatalyst", "v1") }
    fn virtual_reason(&self) -> &'static str { "NewsCatalyst" }
    fn description(&self) -> &'static str { "新闻/公告催化 (D-01 推送)" }
    fn score(&self, input: &StrategyInput) -> Option<StrategyOutput> {
        if input.push_kind == "D-01" {
            Some(StrategyOutput { score: 7.0, reason: "新闻驱动".into(), virtual_reason: "NewsCatalyst".into() })
        } else { None }
    }
}
