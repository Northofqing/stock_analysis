//! 盘后复盘模块。
//!
//! 纯消费者 — 读 portfolio（交易历史 + 净值快照）和 data_provider（K 线），
//! 不做写入。输出格式化的复盘报告文本。
//!
//! ## v2: 因子 IC 分析 (P0-3)
//!
//! `factor_ic` + `factor_report` 提供 AI 评分各因子的 IC/IR 诊断。

pub mod journal;
pub mod equity;
pub mod report;
pub mod sop;
pub mod factor_ic;
pub mod factor_report;
// v12 MVP-4 §7 + MVP-5 §8
pub mod market_stage;        // §7.1
pub mod limit_chain_review; // §7.3 R-03
pub mod lhb_review;         // §7.4 R-04
pub mod tomorrow_watchlist; // §7.6 R-07
pub mod signal_review;      // §7.5 R-05
pub mod failure_attribution; // §8.4 R-06
pub mod performance_feedback; // §8.3
