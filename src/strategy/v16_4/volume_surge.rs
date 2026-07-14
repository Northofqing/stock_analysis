//! v16.4 #2: VolumeSurgeStrategy — 放量 (P-02 推送, score 6.5)

use super::{Strategy, StrategyInput, StrategyOutput};
use crate::impl_strategy_id;

pub struct VolumeSurgeStrategy;

impl Strategy for VolumeSurgeStrategy {
    impl_strategy_id!(VolumeSurgeStrategy, "VolumeSurge");
    fn virtual_reason(&self) -> &'static str { "VolumeSurge" }
    fn description(&self) -> &'static str { "放量 (P-02 推送)" }
    fn score(&self, input: &StrategyInput) -> Option<StrategyOutput> {
        if input.push_kind == "P-02" {
            Some(StrategyOutput { score: 6.5, reason: "量比异动".into(), virtual_reason: "VolumeSurge".into() })
        } else { None }
    }
}
