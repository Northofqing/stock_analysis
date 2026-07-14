//! v16.4 #2: BreakoutStrategy — 突破 (I-03 推送, score 7.5)

use super::{Strategy, StrategyInput, StrategyOutput};
use crate::impl_strategy_id;

pub struct BreakoutStrategy;

impl Strategy for BreakoutStrategy {
    impl_strategy_id!(BreakoutStrategy, "Breakout");
    fn virtual_reason(&self) -> &'static str { "Breakout" }
    fn description(&self) -> &'static str { "突破 (I-03 涨停扩散)" }
    fn score(&self, input: &StrategyInput) -> Option<StrategyOutput> {
        if input.push_kind == "I-03" {
            Some(StrategyOutput { score: 7.5, reason: "涨停扩散龙头".into(), virtual_reason: "Breakout".into() })
        } else { None }
    }
}
