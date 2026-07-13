//! news 模块 — v15.1 业务核心
//!
//! 目标: 全量搜索新闻 + 分析新闻相关股票 (Phase D)
//! Phase B/C 提供 IPO 监测作为子集 (未上市公司 → 上市标的映射)
//!
//! 子模块:
//! - ipo: pre-IPO 公司 → 上市标的静态映射 + cninfo 待发行抓取 (Phase B)
//! - aggregator: 19 路 source 收敛 (Phase D1)
//! - entity_extractor: 2 层实体抽取 (Phase D2)
//! - stock_mapper: news → 股票引擎 (Phase D3)
//! - impact: 影响打分 (Phase D4)
//! - dispatcher: 推 v14 (Phase D5)

pub mod ipo;