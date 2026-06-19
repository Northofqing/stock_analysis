//! 盘后复盘模块。
//!
//! 纯消费者 — 读 portfolio（交易历史 + 净值快照）和 data_provider（K 线），
//! 不做写入。输出格式化的复盘报告文本。

pub mod journal;
pub mod equity;
pub mod report;
pub mod sop;
pub mod falsify;
