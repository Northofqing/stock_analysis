//! v16.4 #2: MainNetInflowStrategy — 主力净流入 (盘后资金, score 6.0)

use super::{Strategy, StrategyInput, StrategyOutput};
use crate::impl_strategy_id;

pub struct MainNetInflowStrategy;

impl Strategy for MainNetInflowStrategy {
    impl_strategy_id!(MainNetInflowStrategy, "MainNetInflow");
    fn virtual_reason(&self) -> &'static str { "MainNetInflow" }
    fn description(&self) -> &'static str { "主力净流入 (盘后资金推送)" }
    fn score(&self, input: &StrategyInput) -> Option<StrategyOutput> {
        if input.push_kind == "盘后资金" {
            Some(StrategyOutput { score: 6.0, reason: "主力净流入 Top10".into(), virtual_reason: "MainNetInflow".into() })
        } else { None }
    }
}
