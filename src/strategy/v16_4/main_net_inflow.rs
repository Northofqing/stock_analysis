//! v16.4 #2: MainNetInflowStrategy — 主力净流入 (盘后资金, score 6.0)

use super::{Strategy, StrategyInput, StrategyOutput};

pub struct MainNetInflowStrategy;

impl Strategy for MainNetInflowStrategy {
    fn id(&self) -> crate::bus::StrategyId { crate::bus::new_strategy_id("MainNetInflow", "v1") }
    fn virtual_reason(&self) -> &'static str { "MainNetInflow" }
    fn description(&self) -> &'static str { "主力净流入 (盘后资金推送)" }
    fn score(&self, input: &StrategyInput) -> Option<StrategyOutput> {
        if input.push_kind == "盘后资金" {
            Some(StrategyOutput { score: 6.0, reason: "主力净流入 Top10".into(), virtual_reason: "MainNetInflow".into() })
        } else { None }
    }
}
