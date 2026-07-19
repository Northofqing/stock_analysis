//! v16.4 #5 完整化: SectorLeaderStrategy 真读 sector + chg (I-01 推送, score 7.0 + 真实数据)

use super::_helpers;
use super::{Strategy, StrategyInput, StrategyOutput};
use crate::impl_strategy_id;

pub struct SectorLeaderStrategy;

impl Strategy for SectorLeaderStrategy {
    impl_strategy_id!(SectorLeaderStrategy, "SectorLeader");
    fn virtual_reason(&self) -> &'static str {
        "SectorLeader"
    }
    fn description(&self) -> &'static str {
        "行业龙头 (I-01 推送)"
    }
    fn score(&self, input: &StrategyInput) -> Option<StrategyOutput> {
        if input.push_kind != "I-01" {
            return None;
        }
        let m = _helpers::parse(&input.metric_json, &input.code, input.push_price).ok()?;
        let sector = m.sector?;
        let price_chg_pct = m.price_chg_pct?;
        if price_chg_pct <= 0.0 {
            return None;
        }
        let score = 7.0 + price_chg_pct.min(3.0) * 0.2;
        Some(StrategyOutput {
            score,
            reason: format!("板块 {sector} 龙头 chg={price_chg_pct:.1}%"),
            virtual_reason: "SectorLeader".into(),
        })
    }
}
