//! v16.4 #2: LLMSelectStrategy — LLM 选股 (Gemini 6 分析师, score 6.5, requires LLM)

use super::{Strategy, StrategyInput, StrategyOutput};

pub struct LLMSelectStrategy;

impl Strategy for LLMSelectStrategy {
    fn id(&self) -> crate::bus::StrategyId { crate::bus::new_strategy_id("LLMSelect", "v1") }
    fn virtual_reason(&self) -> &'static str { "LLMSelect" }
    fn description(&self) -> &'static str { "LLM 选股 (Gemini 6 分析师多空辩论 → 仲裁看多)" }
    fn score(&self, input: &StrategyInput) -> Option<StrategyOutput> {
        if input.push_kind == "LLMSelect" {
            Some(StrategyOutput { score: 6.5, reason: "LLM 多空辩论看多".into(), virtual_reason: "LLMSelect".into() })
        } else { None }
    }
}
