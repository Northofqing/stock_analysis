//! v16.4 #4: Performance module 入口

pub mod attribution;
pub mod attribution_replay;
pub mod economic_position;
pub mod report;
pub mod snapshot;

pub use snapshot::{compute_snapshot, ensure_table, PerformanceEngine, PerformanceSnapshot};
