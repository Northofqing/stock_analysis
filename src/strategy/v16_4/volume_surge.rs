//! v16.4 #5 完整化: VolumeSurgeStrategy 真读 vol (P-02 推送, score 6.5 + 真实数据)

use super::_helpers;
use super::{Strategy, StrategyInput, StrategyOutput};
use crate::impl_strategy_id;

pub struct VolumeSurgeStrategy;

impl Strategy for VolumeSurgeStrategy {
    impl_strategy_id!(VolumeSurgeStrategy, "VolumeSurge");
    fn virtual_reason(&self) -> &'static str {
        "VolumeSurge"
    }
    fn description(&self) -> &'static str {
        "放量 (P-02 推送)"
    }
    fn score(&self, input: &StrategyInput) -> Option<StrategyOutput> {
        if input.push_kind != "P-02" {
            return None;
        }
        let m = _helpers::parse(&input.metric_json, &input.code, input.push_price).ok()?;
        let vol_ratio = m.vol_ratio?;
        if vol_ratio < 2.0 {
            return None;
        }
        let score = 6.5 + ((vol_ratio - 2.0) * 0.15).min(1.5);
        Some(StrategyOutput {
            score,
            reason: format!("量比异动 vol={vol_ratio:.1}"),
            virtual_reason: "VolumeSurge".into(),
        })
    }
}
