//! 实盘监控模块（Phase 0-5 全部实现）。
//!
//! 子模块：
//! - data_quality:   数据质量门
//! - rate_budget:    请求预算与退避
//! - scanner:        阶梯轮询扫描器
//! - detector:       异动检测规则引擎
//! - signal_state:   信号状态机
//! - auction:        集合竞价扫描
//! - alert:          分级告警推送
//! - risk:           风控叠加
//! - signal_fusion:  信号融合
//! - checklist:      盘前/收盘总结
//! - prediction:     预测追踪闭环

pub mod adaptive;
pub mod alert;
pub mod alert_log;
pub mod attribution; // v10 P1 G5a 异动即时归因 (规则快归因, P95 ≤ 2s)
pub mod auction;
pub mod checklist;
pub mod data_mode; // v12 PR2-2.1
pub mod data_quality;
pub mod detector;
pub mod entity_linker;
pub mod event_bus;
mod integration;
pub mod news_ai;
pub mod news_monitor;
pub mod prediction;
pub mod rate_budget;
pub mod risk;
pub mod scanner;
pub mod signal_fusion;
pub mod signal_state;
