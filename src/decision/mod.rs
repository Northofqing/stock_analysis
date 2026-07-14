//! Decision Context — 策略决策引擎。
//!
//! 在 v3 数据层之上做投资决策：排除 → 分档 → 资金验证 → 龙头识别 → 轮动。

pub mod capital_verify;
pub mod decision_decide;
pub mod decision_panel;
pub mod decision_render;
pub mod exclusion;
pub mod holding_plan; // v12 PR4-4.1
pub mod intraday_monitor; // v16.3 Commit 3: 盘中监控 + 盘后整盘 (R4+R5)
pub mod leader;
pub mod live_plan; // v12 PR4-4.2
pub mod pre_trade_filter; // v12 PR2-2.3
pub mod rotation;
pub mod sector_score;
pub mod t0_advisor; // v12 MVP2-2.1
