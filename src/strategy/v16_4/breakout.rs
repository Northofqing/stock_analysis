//! v16.4 #2: BreakoutStrategy — 突破 (I-03 推送, score 7.5)

use super::{Strategy, StrategyInput, StrategyOutput};

pub struct BreakoutStrategy;

impl Strategy for BreakoutStrategy {
    fn id(&self) -> crate::bus::StrategyId { crate::bus::new_strategy_id("Breakout", "v1") }
    fn virtual_reason(&self) -> &'static str { "Breakout" }
    fn description(&self) -> &'static str { "突破 (I-03 涨停扩散)" }
    fn score(&self, input: &StrategyInput) -> Option<StrategyOutput> {
        if input.push_kind == "I-03" {
            Some(StrategyOutput { score: 7.5, reason: "涨停扩散龙头".into(), virtual_reason: "Breakout".into() })
        } else { None }
    }
}
