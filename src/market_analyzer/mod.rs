// -*- coding: utf-8 -*-
//! Registered business rule: BR-213.
//! 大盘复盘分析模块
//!
//! 职责：
//! 1. 获取大盘指数数据（上证、深证、创业板）
//! 2. 搜索市场新闻形成复盘情报
//! 3. 使用大模型生成每日大盘复盘报告

use anyhow::Result;
use chrono::{Datelike, Local};
use log::{info, warn};

use crate::market_data::MarketOverview;
use crate::search_service::{SearchResponse, SearchService};

/// AI 分析器接口（委托给 `traits::AiContentGenerator`）
///
/// 保留此类型别名供本模块内部及现有调用方使用，避免修改调用处签名。
pub use crate::traits::AiContentGenerator as AiAnalyzer;

/// 大盘复盘分析器
pub struct MarketAnalyzer {
    /// 搜索服务（可选）
    search_service: Option<&'static SearchService>,
    /// AI分析器（可选）
    ai_analyzer: Option<Box<dyn AiAnalyzer>>,
}

pub mod async_overview;
mod indices;
pub mod limit_chain_review; // v12 MVP4-4.2
mod limit_up;
pub mod market_stage_confidence; // v12 MVP4-4.1
pub mod performance_feedback; // v12 MVP5-5.1
pub mod post_close_review; // v12 MVP4-4.4
pub mod review;
pub mod sector_monitor;
mod statistics;

pub use async_overview::{generate_market_overview_text_blocking, get_market_overview_blocking};

impl MarketAnalyzer {
    /// 主要指数代码
    const MAIN_INDICES_LIST: &'static [(&'static str, &'static str)] = &[
        ("sh000001", "上证指数"),
        ("sz399001", "深证成指"),
        ("sz399006", "创业板指"),
        ("sh000688", "科创50"),
        ("sh000016", "上证50"),
        ("sh000300", "沪深300"),
    ];

    /// 创建新的大盘分析器
    pub fn new(search_service: Option<&'static SearchService>) -> Result<Self> {
        Ok(Self {
            search_service,
            ai_analyzer: None,
        })
    }

    /// 设置AI分析器
    pub fn with_ai_analyzer(mut self, analyzer: Box<dyn AiAnalyzer>) -> Self {
        self.ai_analyzer = Some(analyzer);
        self
    }

    /// 获取市场概览数据
    pub fn get_market_overview(&self) -> Result<MarketOverview> {
        let today = Local::now().format("%Y-%m-%d").to_string();
        let mut overview = MarketOverview::new(today);

        // 1. 获取主要指数行情
        overview.indices = self.get_main_indices()?;

        // 2. 获取涨跌统计
        self.get_market_statistics(&mut overview)?;

        // 3. 获取板块涨跌榜
        self.get_sector_rankings(&mut overview)?;

        // 4. 统一 HKEX 契约仅提供成交额/配额，不提供净买入。
        // `north_flow` 的语义是净流入（亿元），禁止将成交额错误映射为净流入。
        overview.north_flow = None;
        warn!("[大盘][BR-164] 北向净流入缺失：统一 HKEX 契约仅提供成交额/配额");

        Ok(overview)
    }

    /// 获取当日涨停股票列表。
    ///
    /// 只允许统一 Gateway 提供完整批次；当前上游契约不完整时显式失败，
    /// 不再在分析层维护第二套协议或跨来源拼字段。
    pub fn get_limit_up_stocks(
        &self,
        trading_date: chrono::NaiveDate,
    ) -> Result<Vec<crate::market_data::TopStock>> {
        info!("[大盘] 获取 {trading_date} 涨停股票列表...");
        self.get_limit_up_from_gateway(trading_date)
    }

    /// 搜索市场新闻（异步方法）
    pub async fn search_market_news(&self) -> Vec<SearchResponse> {
        if self.search_service.is_none() {
            warn!("[大盘] 搜索服务未配置，跳过新闻搜索");
            return Vec::new();
        }

        let search_service = self.search_service.as_ref().unwrap();
        let mut all_news = Vec::new();

        let now = Local::now();
        let month_str = format!("{}年{}月", now.year(), now.month());

        let search_queries = vec![
            format!("A股 大盘 复盘 {}", month_str),
            format!("股市 行情 分析 今日 {}", month_str),
            format!("A股 市场 热点 板块 {}", month_str),
        ];

        info!("[大盘] 开始搜索市场新闻...");

        for query in search_queries {
            let result = search_service.search_stock_news("market", "大盘", 3).await;

            let count = result.results.len();
            all_news.push(result);
            info!("[大盘] 搜索 '{}' 获取 {} 条结果", query, count);
        }

        let total = all_news.iter().map(|r| r.results.len()).sum::<usize>();
        info!("[大盘] 共获取 {} 条市场新闻", total);

        all_news
    }

    /// 格式化涨幅前十个股
    fn format_top_stocks(&self, stocks: &[crate::market_data::TopStock]) -> String {
        let mut result = String::new();
        for (i, stock) in stocks.iter().enumerate() {
            result.push_str(&format!(
                "| {} | {} | {} | {:+.2}% | {:.2} |\n",
                i + 1,
                stock.code,
                stock.name,
                stock.change_pct,
                stock.price
            ));
        }
        result
    }

    /// 执行每日大盘复盘流程
    pub async fn run_daily_review(&self) -> Result<String> {
        info!("========== 开始大盘复盘分析 ==========");

        // 1. 获取市场概览
        let overview = self.get_market_overview()?;

        // 2. 搜索市场新闻
        let news = self.search_market_news().await;

        // 3. 生成复盘报告
        let report = self.generate_market_review(&overview, &news);

        info!("========== 大盘复盘分析完成 ==========");

        Ok(report)
    }
}
