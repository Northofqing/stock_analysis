//! Risk Context — 决策硬约束。
//!
//! 与 monitor/risk.rs 并存：monitor 做实时风险告警与计算 (StopLoss/PositionSizer)，
//! risk 做决策硬约束 (HardLimits/check_stops/VetoChain)。
//!
//! ## 模块分工
//!
//! | 模块 | 职责 |
//! |------|------|
//! | `monitor/risk.rs` | 实时计算: StopLoss, PositionSizer, MarketRegime 分类 |
//! | `risk/limits.rs` | 硬约束检查: 单票/板块/止损/现金底线 |
//! | `risk/stop_loss.rs` | 三级止损信号: 技术/结构/硬止损 |
//! | `risk/veto_chain.rs` | 实时否决链框架: VetoRule trait + VetoChain |
//! | `risk/veto_rules_live.rs` | 三条实时否决规则: 乖离率/资金面/基本面 |

pub mod limits;
pub mod cash_guard;
pub mod stop_loss;
pub mod sector_exit;
pub mod veto_chain;
pub mod veto_rules_live;
