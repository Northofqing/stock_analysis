//! push_l4 — v14.2 Layer 4: Dispatcher (仲裁层)
//!
//! 严格按 `docs/architecture/v14.2-push-architecture.md` v14.2 §3.4 + b-009 R-4 落地.
//!
//! W4.2 状态 (b011 P0-2): dedup 闭环已实装 — 键 (kind, code), 窗口来自
//! PushKind::cooldown_secs(). 模板匹配/渲染仍走 v13 render (L3 未建).

pub mod dispatcher;

// 重新导出
pub use dispatcher::{DispatchOutcome, Dispatcher, DispatcherStats, ReserveOutcome};