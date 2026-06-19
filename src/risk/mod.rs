//! Risk Context — 决策硬约束。
//!
//! 与 monitor/risk.rs 并存：monitor 做实时风险告警，risk 做决策硬约束。

pub mod limits;
pub mod cash_guard;
pub mod stop_loss;
pub mod sector_exit;
