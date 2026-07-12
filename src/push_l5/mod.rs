//! push_l5 — v14.2 Layer 5: Governance (治理层)
//!
//! 严格按 `docs/architecture/v14.2-push-architecture.md` v14.2 §3.5 + b-008 §4.1 落地.
//!
//! W5.1 状态: 落地 `governance.rs` (GovernanceEngine + 16 个单测).
//! 后续 W5.2/W6 会加: monitor_loop 集成 + GovernanceContext 构造 + 与 L4 dispatcher 联动.

pub mod governance;

// 重新导出
pub use governance::{
    data_mode_severity, event_kind_exempt_from_cooldown, event_severity, is_data_source_down_event,
    is_quiet_hour, GovernanceContext, GovernanceDecision, GovernanceEngine,
};