// -*- coding: utf-8 -*-
//! ===================================
//! A股自选股智能分析系统 - 搜索服务模块
//! ===================================
//!
//! 职责：
//! 1. 提供统一的新闻搜索接口
//! 2. 仅支持 Tavily、SerpAPI、Bocha 三类通用网页研究接口
//! 3. 通用搜索协议、多 Key 轮询与错误分类由 data_gateway 统一持有
//! 4. 搜索结果缓存和格式化
//!
//! 治理金融/新闻事实只能经 `crate::data_gateway` 获取；本模块不保留
//! 金融站点 URL、协议解析器或本地 provider fallback。
//!
//! 本文件原为 `src/search_service.rs`（2427 行），拆分为：
//! - `types`      — 数据类型 / `SearchProvider` trait
//! - `providers/` — 无网络 Gateway 薄适配器
//! - `service`    — 聚合器与 `get_search_service` 单例

pub(crate) mod macro_news;
pub mod providers;
pub mod service;
pub mod types;

// 保留原扁平路径，兼容 `crate::search_service::XXX` 调用
pub use providers::GeneralWebSearchProvider;
pub use service::{get_search_service, SearchService};
pub use types::{
    NewsType, SearchEvidence, SearchFailureEvidence, SearchProvider, SearchResponse, SearchResult,
    Sentiment,
};
