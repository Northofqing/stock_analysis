//! v16.4 #2: NewsCatalystStrategy — 新闻/公告催化 (D-01 推送, score 7.0)

use super::{Strategy, StrategyInput, StrategyOutput};
use crate::impl_strategy_id;

pub struct NewsCatalystStrategy;

impl Strategy for NewsCatalystStrategy {
    impl_strategy_id!(NewsCatalystStrategy, "NewsCatalyst");
    fn virtual_reason(&self) -> &'static str { "NewsCatalyst" }
    fn description(&self) -> &'static str { "新闻/公告催化 (D-01 推送)" }
    fn score(&self, input: &StrategyInput) -> Option<StrategyOutput> {
        if input.push_kind == "D-01" {
            Some(StrategyOutput { score: 7.0, reason: "新闻驱动".into(), virtual_reason: "NewsCatalyst".into() })
        } else { None }
    }
}
