//! v16.4 #2: AuctionAnomalyStrategy — 竞价量能异动 (P-02 推送, score 6.5, vol_ratio ≥ 5)

use super::{Strategy, StrategyInput, StrategyOutput};
use crate::impl_strategy_id;

pub struct AuctionAnomalyStrategy;

impl Strategy for AuctionAnomalyStrategy {
    impl_strategy_id!(AuctionAnomalyStrategy, "AuctionAnomaly");
    fn virtual_reason(&self) -> &'static str { "AuctionAnomaly" }
    fn description(&self) -> &'static str { "竞价量能异动 (P-02 推送)" }
    fn score(&self, input: &StrategyInput) -> Option<StrategyOutput> {
        let m: serde_json::Value = serde_json::from_str(&input.metric_json).unwrap_or_default();
        let vol = m.get("vol_ratio").and_then(|v| v.as_f64()).unwrap_or(0.0);
        if input.push_kind == "P-02" && vol >= 5.0 {
            Some(StrategyOutput { score: 6.5, reason: format!("竞价量能 vol={}", vol), virtual_reason: "AuctionAnomaly".into() })
        } else { None }
    }
}
