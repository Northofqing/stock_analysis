//! v16.4 #2: SectorLeaderStrategy — 行业龙头 (I-01 推送, score 7.0)

use super::{Strategy, StrategyInput, StrategyOutput};
use crate::impl_strategy_id;

pub struct SectorLeaderStrategy;

impl Strategy for SectorLeaderStrategy {
    impl_strategy_id!(SectorLeaderStrategy, "SectorLeader");
    fn virtual_reason(&self) -> &'static str { "SectorLeader" }
    fn description(&self) -> &'static str { "行业龙头 (I-01 推送)" }
    fn score(&self, input: &StrategyInput) -> Option<StrategyOutput> {
        if input.push_kind == "I-01" {
            Some(StrategyOutput { score: 7.0, reason: "板块轮动 top1".into(), virtual_reason: "SectorLeader".into() })
        } else { None }
    }
}
