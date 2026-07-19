//! v16.4 #5 完整化: BreakoutStrategy 真读 chg + vol (I-03 推送, score 7.5 + 真实数据)

use super::_helpers;
use super::{Strategy, StrategyInput, StrategyOutput};
use crate::impl_strategy_id;

pub struct BreakoutStrategy;

impl Strategy for BreakoutStrategy {
    impl_strategy_id!(BreakoutStrategy, "Breakout");
    fn virtual_reason(&self) -> &'static str {
        "Breakout"
    }
    fn description(&self) -> &'static str {
        "突破 (I-03 涨停扩散)"
    }
    fn score(&self, input: &StrategyInput) -> Option<StrategyOutput> {
        if input.push_kind != "I-03" {
            return None;
        }
        let m = _helpers::parse(&input.metric_json, &input.code, input.push_price).ok()?;
        let price_chg_pct = m.price_chg_pct?;
        let vol_ratio = m.vol_ratio?;
        if price_chg_pct < 5.0 || vol_ratio < 3.0 {
            return None;
        }
        let score = 7.5 + (price_chg_pct - 5.0).min(4.0) * 0.1;
        Some(StrategyOutput {
            score,
            reason: format!("涨停扩散 chg={:.1}% vol={:.1}", price_chg_pct, vol_ratio),
            virtual_reason: "Breakout".into(),
        })
    }
}
