//! v16.4 #5 完整化: AuctionAnomalyStrategy 真读 vol_ratio (P-02 推送, score 6.5 + 真实数据)

use super::_helpers;
use super::{Strategy, StrategyInput, StrategyOutput};
use crate::impl_strategy_id;

pub struct AuctionAnomalyStrategy;

impl Strategy for AuctionAnomalyStrategy {
    impl_strategy_id!(AuctionAnomalyStrategy, "AuctionAnomaly");
    fn virtual_reason(&self) -> &'static str { "AuctionAnomaly" }
    fn description(&self) -> &'static str { "竞价量能异动 (P-02 推送)" }
    fn score(&self, input: &StrategyInput) -> Option<StrategyOutput> {
        if input.push_kind != "P-02" {
            return None;
        }
        let m = _helpers::parse(&input.metric_json, &input.code, input.push_price);
        if m.vol_ratio < 5.0 {
            return None;
        }
        let score = 6.5 + ((m.vol_ratio - 5.0) * 0.1).min(1.0);
        Some(StrategyOutput {
            score,
            reason: format!("竞价量能 vol={:.1} chg={:.1}%", m.vol_ratio, m.price_chg_pct),
            virtual_reason: "AuctionAnomaly".into(),
        })
    }
}
