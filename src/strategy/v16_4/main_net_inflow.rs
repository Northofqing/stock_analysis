//! v16.4 #5 完整化: MainNetInflowStrategy 真读 price_chg_pct (盘后资金, score 6.0 + 真实数据)

use super::_helpers;
use super::{Strategy, StrategyInput, StrategyOutput};
use crate::impl_strategy_id;

pub struct MainNetInflowStrategy;

impl Strategy for MainNetInflowStrategy {
    impl_strategy_id!(MainNetInflowStrategy, "MainNetInflow");
    fn virtual_reason(&self) -> &'static str {
        "MainNetInflow"
    }
    fn description(&self) -> &'static str {
        "主力净流入 (盘后资金推送)"
    }
    fn score(&self, input: &StrategyInput) -> Option<StrategyOutput> {
        if input.push_kind != "盘后资金" {
            return None;
        }
        let m = _helpers::parse(&input.metric_json, &input.code, input.push_price).ok()?;
        let main_net_yi = m.main_net_yi?;
        if main_net_yi <= 0.0 {
            return None;
        }
        let price_chg_pct = m.price_chg_pct?;
        let chg_penalty = price_chg_pct.min(0.0).abs() * 0.1;
        let score = (6.0 - chg_penalty).max(5.0);
        Some(StrategyOutput {
            score,
            reason: format!("主力净流入 {main_net_yi:.2}亿 chg={price_chg_pct:.1}%"),
            virtual_reason: "MainNetInflow".into(),
        })
    }
}
