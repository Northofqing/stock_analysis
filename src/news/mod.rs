//! news 模块 — v15.1 业务核心
//!
//! 目标: 全量搜索新闻 + 分析新闻相关股票 (Phase D)
//! Phase B/C 提供 IPO 监测作为子集 (未上市公司 → 上市标的映射)
//!
//! 子模块:
//! - ipo: pre-IPO 公司 → 上市标的静态映射 + cninfo 待发行抓取 (Phase B)
//! - aggregator: 12 路 NewsFeed 收敛 (Phase D1) — 2026-07-13 v15.3 新建
//! - entity_extractor: 2 层实体抽取 (Phase D2) — promote 复用 opportunity::event_extractor
//! - stock_mapper: news → 股票引擎 (Phase D3) — 复用 chain_registry + bom_kb
//! - impact: 影响打分 (Phase D4)
//! - dispatcher: 推 v14 (Phase D5)

pub mod ipo;
pub mod aggregator;
pub mod impact;
pub mod dispatcher;
pub mod sink;