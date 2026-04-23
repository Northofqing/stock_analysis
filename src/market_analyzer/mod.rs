// -*- coding: utf-8 -*-
//! 大盘复盘分析模块
//!
//! 职责：
//! 1. 获取大盘指数数据（上证、深证、创业板）
//! 2. 搜索市场新闻形成复盘情报
//! 3. 使用大模型生成每日大盘复盘报告

use anyhow::{Context, Result};
use chrono::{Datelike, Local};
use log::{error, info, warn};
use reqwest::blocking::Client;
use serde_json::Value;
use std::collections::HashMap;
use std::thread;
use std::time::Duration;

use crate::market_data::MarketOverview;
use crate::search_service::{SearchResponse, SearchService};

/// AI 分析器接口（委托给 `traits::AiContentGenerator`）
///
/// 保留此类型别名供本模块内部及现有调用方使用，避免修改调用处签名。
pub use crate::traits::AiContentGenerator as AiAnalyzer;

/// 大盘复盘分析器
pub struct MarketAnalyzer {
    /// HTTP客户端
    client: Client,
    /// 搜索服务（可选）
    search_service: Option<&'static SearchService>,
    /// AI分析器（可选）
    ai_analyzer: Option<Box<dyn AiAnalyzer>>,
    /// 主要指数代码映射
    main_indices: HashMap<String, String>,
}

mod indices;
mod limit_up;
mod review;
mod statistics;

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
    /// 创建新的大盘分析器
    pub fn new(search_service: Option<&'static SearchService>) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("创建HTTP客户端失败")?;

        let mut main_indices = HashMap::new();
        for (code, name) in Self::MAIN_INDICES_LIST {
            main_indices.insert(code.to_string(), name.to_string());
        }

        Ok(Self {
            client,
            search_service,
            ai_analyzer: None,
            main_indices,
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

        Ok(overview)
    }

    /// 获取当日涨停股票列表
    /// 优先使用东方财富行情API（覆盖沪深两市），失败时回退到新浪API
    pub fn get_limit_up_stocks(&self) -> Result<Vec<crate::market_data::TopStock>> {
        info!("[大盘] 获取当日涨停股票列表...");

        // 优先使用东方财富行情API
        match self.get_limit_up_from_eastmoney() {
            Ok(stocks) if !stocks.is_empty() => {
                info!("[大盘] 东方财富API发现 {} 只涨停股票", stocks.len());
                return Ok(stocks);
            }
            Ok(_) => {
                info!("[大盘] 东方财富API返回空，回退到新浪API");
            }
            Err(e) => {
                warn!("[大盘] 东方财富API失败: {}，回退到新浪API", e);
            }
        }

        // 回退：从新浪API按涨幅倒序获取涨停股票
        self.get_limit_up_from_sina()
    }

    /// 带重试的API调用
    pub(super) fn call_api_with_retry<F>(&self, name: &str, attempts: u32, f: F) -> Option<Value>
    where
        F: Fn() -> Result<Value>,
    {
        let mut last_error: Option<anyhow::Error> = None;

        for attempt in 1..=attempts {
            match f() {
                Ok(data) => return Some(data),
                Err(e) => {
                    last_error = Some(e);
                    warn!("[大盘] {} 获取失败 (attempt {}/{}): {:?}", name, attempt, attempts, last_error);
                    if attempt < attempts {
                        let sleep_duration = Duration::from_secs(2u64.pow(attempt).min(5));
                        thread::sleep(sleep_duration);
                    }
                }
            }
        }

        error!("[大盘] {} 最终失败: {:?}", name, last_error);
        None
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

#[cfg(test)]
mod tests {
    use super::*;

    // #[test]
    // fn test_parse_sina_line() {
    //     let analyzer = MarketAnalyzer::new(None).unwrap();
    //     let line = r#"var hq_str_sh000001="上证指数,3089.26,3104.14,3077.65";"#;
    //     let result = analyzer.parse_sina_line(line);
    //     assert!(result.is_some());
    //     assert_eq!(result.unwrap(), "上证指数,3089.26,3104.14,3077.65");
    // }
}
