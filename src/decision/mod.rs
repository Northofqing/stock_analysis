//! Decision Context — 策略决策引擎。
//!
//! 在 v3 数据层之上做投资决策：排除 → 分档 → 资金验证 → 龙头识别 → 轮动。

pub mod exclusion;
pub mod sector_score;
pub mod capital_verify;
pub mod rotation;
pub mod leader;
pub mod decision_panel;
