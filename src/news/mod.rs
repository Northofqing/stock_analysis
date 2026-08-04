//! news 模块 — v15.1 业务核心
//!
//! 目标: 消费统一新闻事实并分析来源明确关联的股票。
//!
//! 子模块:
//! - ipo: pre-IPO 供应链关系知识（不负责公共数据采集）
//! - aggregator: 四路统一 GlobalNewsGateway feed 收敛
//! - entity_extractor: 2 层实体抽取 (Phase D2) — promote 复用 opportunity::event_extractor
//! - stock_mapper: news → 股票引擎 (Phase D3)
//! - impact: 影响打分 (Phase D4)
//! - dispatcher: 推 v14 (Phase D5)

pub mod aggregator;
pub mod dispatcher;
pub mod impact;
pub mod ipo;
pub mod sink;
pub mod stock_mapper;
