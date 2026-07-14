//! v16.4 #2: LLMSelectStrategy — LLM 选股 (Gemini 6 分析师, score 6.5, requires LLM)

use super::{Strategy, StrategyInput, StrategyOutput};
use crate::impl_strategy_id;

pub struct LLMSelectStrategy;

impl Strategy for LLMSelectStrategy {
    impl_strategy_id!(LLMSelectStrategy, "LLMSelect");
    fn virtual_reason(&self) -> &'static str { "LLMSelect" }
    fn description(&self) -> &'static str { "LLM 选股 (Gemini 6 分析师多空辩论 → 仲裁看多)" }
    fn score(&self, input: &StrategyInput) -> Option<StrategyOutput> {
        if input.push_kind != "LLMSelect" {
            return None;
        }
        // Fix review #5 (MEDIUM): 真读 metric_json 里 LLM confidence + verdict
        let m: serde_json::Value = serde_json::from_str(&input.metric_json).unwrap_or_default();
        let confidence = m.get("llm_confidence").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let verdict = m.get("llm_verdict").and_then(|v| v.as_str()).unwrap_or("");
        // LLM 要求: confidence >= 0.7 AND verdict == "看多" (plan §R1)
        if confidence < 0.7 || verdict != "看多" {
            return None;
        }
        Some(StrategyOutput {
            score: 6.5,
            reason: format!("LLM 看多 confidence={:.2}", confidence),
            virtual_reason: "LLMSelect".into(),
        })
    }
}
