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
pub mod advice_feedback_report; // 2026-10-16: 建议质量报告 (自进化回路第③段, 仅审计)
pub mod advice_outcome_backfill; // 2026-10-16: 建议结果滚动回填 (T+1/T+3/T+5)
pub mod ticket_watch_pool; // 2026-10-17: 观察池生成 (新闻催化; 板块轮动待接写入方)
pub mod ticket_event_sync; // 2026-10-17: 投递记账 (Delivered → ticket_event; 观察池/持仓才建票)
pub mod trading_fact_snapshot; // 2026-10-17: op 67 价格带每日落库 (快照日有值, 错过永久丢)
pub mod daily_bars_indicators; // 2026-10-17: 日线+均线补齐 (与 authority 门解耦, P2 数据基础)
pub mod buy_zone; // 2026-10-17: P2 可买价 (按票: 位置/买点区间/放弃价)
pub mod ticket_stage_sync; // 2026-10-18: 持仓快照驱动 stage 自动流转 (观察→持有→退出)
pub mod ticket_eviction; // 2026-10-18: 观察池淘汰 (超期/跌幅, 只管 observing)
pub mod board_ranking_snapshot; // 2026-10-18: 板块榜每日落库 (f3 涨幅 / f62 净流入)
pub mod alert;
pub mod alert_log;
pub mod attribution; // v10 P1 G5a 异动即时归因 (规则快归因, P95 ≤ 2s)
pub mod attribution_deep; // G5b 深链归因 (盘后 LLM 深链, 2026-08-22 落地)
pub mod auction;
pub mod checklist;
pub mod data_mode; // v12 PR2-2.1
pub mod data_quality;
pub mod detector;
pub mod entity_linker;
pub mod event_bus;
pub mod g5b_analysis_v2;
pub mod g5b_empty_v2;
pub mod g5b_physical_v2;
pub(crate) mod g5b_selection_v2;
mod integration;
pub mod news_ai;
pub mod news_monitor;
pub mod news_outcomes;
pub mod outcome_data;
pub mod prediction;
pub mod push_job;
pub mod rate_budget;
pub mod risk;
pub mod scanner;
pub mod signal_fusion;
pub mod signal_state;
