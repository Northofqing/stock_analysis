//! push_l1 — v14.2 Layer 1: Signal Source (信号源层)
//!
//! 严格按 `docs/architecture/v14.2-push-architecture.md` v14.2 §3.1 + §3.1.1 实现.
//! 当前 W2.1 状态: 只落地 `event.rs` (SignalEvent + EventBucket + make_event_id).
//! 后续 W3-W4 会加: market_session_emitter, preopen_window_emitter, etc.

pub mod event;

// 重新导出主要类型, 方便 L4 dispatcher 一行 use
pub use event::{
    make_event_id, make_source_fact_event_id, DataSourceDownPayload, EventBucket,
    HoldingHealthPayload, LimitUpPayload, LimitUpTier, LimitUpTierPayload, NewsCatalystPayload,
    PositionChangedPayload, PostSessionReviewPayload, QuietHourPayload, RiskViolationPayload,
    SectorRotationPayload, Severity, SignalEvent, SignalPayload, SignalSource,
};
