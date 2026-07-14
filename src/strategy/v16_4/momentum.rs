//! v16.4 #5 完整化: MomentumStrategy 真读 vol + chg (Momentum 推送, score 8.0 + 真实数据)

use super::_helpers;
use super::{Strategy, StrategyInput, StrategyOutput};
use crate::impl_strategy_id;

pub struct MomentumStrategy;

impl Strategy for MomentumStrategy {
    impl_strategy_id!(MomentumStrategy, "Momentum");
    fn virtual_reason(&self) -> &'static str { "Momentum" }
    fn description(&self) -> &'static str { "动量整合 (air_refuel 形态分 ≥ 7 AND 3 指标金叉共振)" }
    fn score(&self, input: &StrategyInput) -> Option<StrategyOutput> {
        if input.push_kind != "Momentum" {
            return None;
        }
        let m = _helpers::parse(&input.metric_json, &input.code, input.push_price);
        if m.vol_ratio < 5.0 || m.price_chg_pct <= 0.0 || m.quote_price <= 0.0 {
            return None;
        }
        let score = 8.0 + m.price_chg_pct.min(5.0) * 0.2;
        Some(StrategyOutput {
            score: score.min(9.5),
            reason: format!("Momentum 强共振 vol={:.1} chg={:.1}% quote={:.1}", m.vol_ratio, m.price_chg_pct, m.quote_price),
            virtual_reason: "Momentum".into(),
        })
    }
}
