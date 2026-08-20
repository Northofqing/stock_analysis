//! v16.4 #4: Performance module 入口

pub mod attribution;
pub mod snapshot;

pub use snapshot::{compute_snapshot, ensure_table, PerformanceEngine, PerformanceSnapshot};
