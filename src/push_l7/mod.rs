//! push_l7 — v14.2 Layer 7: Analytics (分析层)
//!
//! 严格按 `docs/architecture/v14.2-push-architecture.md` v14.2 §3.7 落地.
//!
//! W7.1 状态: 落地 `analytics.rs` (PushAnalytics + AnalyticsStore trait + InMemoryStore + 13 单测).
//! W7.2 状态: 落地 `sqlite_store.rs` (SqliteStore + push_analytics 表 + 9 单测).

pub mod analytics;
pub mod sqlite_store;

// 重新导出
pub use analytics::{
    build_analytics, AnalyticsStore, InMemoryStore, PushAnalytics, ValidationStatus,
};
pub use sqlite_store::SqliteStore;
