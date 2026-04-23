// -*- coding: utf-8 -*-
//! ===================================
//! A股自选股智能分析系统 - 搜索服务模块
//! ===================================
//!
//! 职责：
//! 1. 提供统一的新闻搜索接口
//! 2. 支持 Tavily、SerpAPI、Bocha、华尔街见闻、金十、东方财富 六类数据源
//! 3. 多 Key 负载均衡和故障转移
//! 4. 搜索结果缓存和格式化
//!
//! 本文件原为 `src/search_service.rs`（2427 行），拆分为：
//! - `types`      — 数据类型 / `SearchProvider` trait / `ApiKeyManager`
//! - `providers/` — 各引擎实现
//! - `service`    — 聚合器与 `get_search_service` 单例

pub mod providers;
pub mod service;
pub mod types;

// 保留原扁平路径，兼容 `crate::search_service::XXX` 调用
pub use providers::{
    BochaSearchProvider, EastmoneyNewsProvider, Jin10CalendarEvent, Jin10Provider,
    SerpAPISearchProvider, TavilySearchProvider, WallStreetCnProvider,
};
pub use service::{get_search_service, SearchService};
pub use types::{NewsType, SearchProvider, SearchResponse, SearchResult, Sentiment};
