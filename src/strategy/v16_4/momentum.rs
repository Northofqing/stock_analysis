//! v16.4 #2: MomentumStrategy — 动量整合 (score 8.0, vol_ratio ≥ 5)

use super::{Strategy, StrategyInput, StrategyOutput};

pub struct MomentumStrategy;

impl Strategy for MomentumStrategy {
    fn id(&self) -> crate::bus::StrategyId { crate::bus::new_strategy_id("Momentum", "v1") }
    fn virtual_reason(&self) -> &'static str { "Momentum" }
    fn description(&self) -> &'static str { "动量整合 (air_refuel 形态分 ≥ 7 AND 3 指标金叉共振)" }
    fn score(&self, input: &StrategyInput) -> Option<StrategyOutput> {
        if input.push_kind != "Momentum" {
            return None;
        }
        let m: serde_json::Value = serde_json::from_str(&input.metric_json).unwrap_or_default();
        let vol = m.get("vol_ratio").and_then(|v| v.as_f64()).unwrap_or(0.0);
        if vol < 5.0 {
            return None;
        }
        Some(StrategyOutput {
            score: 8.0,
            reason: format!("Momentum 强共振 vol={}", vol),
            virtual_reason: "Momentum".into(),
        })
    }
}
