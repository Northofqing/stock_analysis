//! v16.4 #4: Performance module 入口

pub mod attribution;
pub mod attribution_epoch;
pub mod attribution_replay;
pub mod economic_position;
pub mod fee_evidence;
pub mod fee_policy;
pub mod paper_decision_outcomes_v1;
pub mod report;
pub mod snapshot;

pub use snapshot::{compute_snapshot, PerformanceEngine, PerformanceSnapshot};
