//! Breakout Engine — 多维度放量启动识别。
//!
//! 统一四套放量指标为一个引擎：
//! - 盘中模式：东财 push2 + ignition，不需要 K 线
//! - 盘后模式：K 线 + trend_analyzer + classify_volume，含启动/出货判断

pub mod signal;
pub mod position;
pub mod engine;
