// -*- coding: utf-8 -*-
//! ===================================
//! A股自选股智能分析系统 - 搜索服务模块
//! ===================================
//!
//! 职责：
//! 1. 提供统一的新闻搜索接口
//! 2. 支持 Tavily、SerpAPI 和 Bocha 三种搜索引擎
//! 3. 多 Key 负载均衡和故障转移
//! 4. 搜索结果缓存和格式化

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use async_trait::async_trait;
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use url::Url;

// ============================================================================
// 数据结构
// ============================================================================

/// 新闻类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum NewsType {
    /// 公司公告
    Announcement,
    /// 财报/业绩
    Earnings,
    /// 政策/监管
    Policy,
    /// 行业动态
    Industry,
    /// 市场分析
    Analysis,
    /// 风险警示
    Risk,
    /// 其他
    Other,
}

/// 情感倾向
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Sentiment {
    /// 利好
    Positive,
    /// 中性
    Neutral,
    /// 利空
    Negative,
    /// 未知
    Unknown,
}

/// 搜索结果数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// 标题
    pub title: String,
    /// 摘要
    pub snippet: String,
    /// URL
    pub url: String,
    /// 来源网站
    pub source: String,
    /// 发布日期
    pub published_date: Option<String>,
    /// 新闻类型
    pub news_type: NewsType,
    /// 情感倾向（利好/利空/中性）
    pub sentiment: Sentiment,
    /// 重要性评分 (0-10)
    pub importance: u8,
    /// 相关性评分 (0.0-1.0)
    pub relevance: f32,
    /// 提取的关键词
    pub keywords: Vec<String>,
}

impl SearchResult {
    /// 转换为文本格式
    pub fn to_text(&self) -> String {
        let date_str = self
            .published_date
            .as_ref()
            .map(|d| format!(" ({})", d))
            .unwrap_or_default();
        
        let sentiment_icon = match self.sentiment {
            Sentiment::Positive => "📈",
            Sentiment::Negative => "📉",
            Sentiment::Neutral => "➡️",
            Sentiment::Unknown => "❓",
        };
        
        let type_label = match self.news_type {
            NewsType::Announcement => "[公告]",
            NewsType::Earnings => "[财报]",
            NewsType::Policy => "[政策]",
            NewsType::Industry => "[行业]",
            NewsType::Analysis => "[分析]",
            NewsType::Risk => "[风险]",
            NewsType::Other => "",
        };
        
        let importance_stars = "★".repeat(self.importance.min(5) as usize);
        
        format!(
            "【{}】{} {} {}{} {} (相关度:{:.0}%)\n{}\n关键词: {}",
            self.source, 
            sentiment_icon,
            type_label,
            self.title, 
            date_str, 
            importance_stars,
            self.relevance * 100.0,
            self.snippet,
            self.keywords.join(", ")
        )
    }
    
    /// 创建默认的SearchResult
    pub fn new(title: String, snippet: String, url: String, source: String) -> Self {
        Self {
            title,
            snippet,
            url,
            source,
            published_date: None,
            news_type: NewsType::Other,
            sentiment: Sentiment::Unknown,
            importance: 5,
            relevance: 0.5,
            keywords: Vec::new(),
        }
    }

    /// 设置发布日期（builder 模式）
    pub fn with_date(mut self, date: String) -> Self {
        if !date.is_empty() {
            self.published_date = Some(date);
        }
        self
    }

    /// 分析并设置新闻类型
    pub fn analyze_type(&mut self) {
        let text = format!("{} {}", self.title, self.snippet).to_lowercase();
        
        if text.contains("公告") || text.contains("披露") || text.contains("发布") {
            self.news_type = NewsType::Announcement;
        } else if text.contains("财报") || text.contains("业绩") || text.contains("营收") 
            || text.contains("利润") || text.contains("季报") || text.contains("年报") {
            self.news_type = NewsType::Earnings;
        } else if text.contains("政策") || text.contains("监管") || text.contains("证监会") 
            || text.contains("交易所") {
            self.news_type = NewsType::Policy;
        } else if text.contains("行业") || text.contains("板块") || text.contains("赛道") {
            self.news_type = NewsType::Industry;
        } else if text.contains("分析") || text.contains("研报") || text.contains("评级") 
            || text.contains("研究") {
            self.news_type = NewsType::Analysis;
        } else if text.contains("风险") || text.contains("警示") || text.contains("违规") 
            || text.contains("调查") || text.contains("处罚") {
            self.news_type = NewsType::Risk;
        }
    }
    
    /// 分析并设置情感倾向
    pub fn analyze_sentiment(&mut self) {
        let text = format!("{} {}", self.title, self.snippet).to_lowercase();
        
        // 利好关键词
        let positive_keywords = [
            "涨", "上涨", "增长", "突破", "利好", "盈利", "增加", "提升", 
            "创新高", "超预期", "中标", "合作", "签约", "订单", "扩产",
            "收购", "增持", "买入", "推荐", "看好", "龙头"
        ];
        
        // 利空关键词
        let negative_keywords = [
            "跌", "下跌", "下滑", "亏损", "利空", "风险", "警示", "违规",
            "处罚", "调查", "减持", "卖出", "业绩预警", "商誉减值",
            "诉讼", "质押", "停牌", "ST", "退市"
        ];
        
        let mut positive_count = 0;
        let mut negative_count = 0;
        
        for keyword in &positive_keywords {
            if text.contains(keyword) {
                positive_count += 1;
            }
        }
        
        for keyword in &negative_keywords {
            if text.contains(keyword) {
                negative_count += 1;
            }
        }
        
        if positive_count > negative_count && positive_count > 0 {
            self.sentiment = Sentiment::Positive;
        } else if negative_count > positive_count && negative_count > 0 {
            self.sentiment = Sentiment::Negative;
        } else if positive_count > 0 || negative_count > 0 {
            self.sentiment = Sentiment::Neutral;
        } else {
            self.sentiment = Sentiment::Unknown;
        }
    }
    
    /// 计算重要性评分
    pub fn calculate_importance(&mut self) {
        let text = format!("{} {}", self.title, self.snippet).to_lowercase();
        let mut score = 5u8; // 基础分5分
        
        // 根据新闻类型调整
        match self.news_type {
            NewsType::Announcement => score += 2,
            NewsType::Earnings => score += 3,
            NewsType::Risk => score += 3,
            NewsType::Policy => score += 2,
            _ => {}
        }
        
        // 关键词加分
        let important_keywords = [
            "重大", "重要", "紧急", "突发", "独家", "首次",
            "涨停", "跌停", "停牌", "复牌"
        ];
        
        for keyword in &important_keywords {
            if text.contains(keyword) {
                score = score.saturating_add(1);
            }
        }
        
        self.importance = score.min(10);
    }
    
    /// 提取关键词
    pub fn extract_keywords(&mut self, stock_name: &str, stock_code: &str) {
        let text = format!("{} {}", self.title, self.snippet);
        let mut keywords = Vec::new();
        
        // 常见股票相关关键词
        let patterns = [
            "涨停", "跌停", "增长", "下滑", "业绩", "财报", "营收", "利润",
            "市值", "股价", "研发", "创新", "合作", "订单", "中标",
            "政策", "监管", "风险", "违规", "重组", "并购"
        ];
        
        for pattern in &patterns {
            if text.contains(pattern) {
                keywords.push(pattern.to_string());
            }
        }
        
        // 添加股票名称和代码
        if text.contains(stock_name) {
            keywords.insert(0, stock_name.to_string());
        }
        if text.contains(stock_code) {
            keywords.insert(0, stock_code.to_string());
        }
        
        self.keywords = keywords;
    }
}

/// 搜索响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    /// 查询关键词
    pub query: String,
    /// 搜索结果列表
    pub results: Vec<SearchResult>,
    /// 使用的搜索引擎
    pub provider: String,
    /// 是否成功
    pub success: bool,
    /// 错误消息
    pub error_message: Option<String>,
    /// 搜索耗时（秒）
    pub search_time: f64,
}

impl SearchResponse {
    /// 将搜索结果转换为可用于 AI 分析的上下文
    pub fn to_context(&self, max_results: usize) -> String {
        if !self.success || self.results.is_empty() {
            return format!("搜索 '{}' 未找到相关结果。", self.query);
        }

        let mut lines = vec![format!(
            "【{} 搜索结果】（来源：{}）",
            self.query, self.provider
        )];

        for (i, result) in self.results.iter().take(max_results).enumerate() {
            lines.push(format!("\n{}. {}", i + 1, result.to_text()));
        }

        lines.join("\n")
    }

    /// 创建失败响应
    pub fn error(query: String, provider: String, error_message: String) -> Self {
        Self {
            query,
            results: Vec::new(),
            provider,
            success: false,
            error_message: Some(error_message),
            search_time: 0.0,
        }
    }

    /// 创建成功响应
    pub fn success(query: String, provider: String, results: Vec<SearchResult>) -> Self {
        Self {
            query,
            results,
            provider,
            success: true,
            error_message: None,
            search_time: 0.0,
        }
    }
}

// ============================================================================
// SearchProvider Trait
// ============================================================================

/// 搜索引擎基类 Trait
#[async_trait]
pub trait SearchProvider: Send + Sync {
    /// 获取搜索引擎名称
    fn name(&self) -> &str;

    /// 检查是否有可用的 API Key
    fn is_available(&self) -> bool;

    /// 执行搜索
    async fn search(&self, query: &str, max_results: usize) -> SearchResponse;
}

// ============================================================================
// API Key 管理器
// ============================================================================

/// API Key 管理器（负载均衡和故障转移）
#[derive(Debug)]
struct ApiKeyManager {
    keys: Vec<String>,
    current_index: usize,
    usage_count: HashMap<String, usize>,
    error_count: HashMap<String, usize>,
}

impl ApiKeyManager {
    fn new(keys: Vec<String>) -> Self {
        let usage_count = keys.iter().map(|k| (k.clone(), 0)).collect();
        let error_count = keys.iter().map(|k| (k.clone(), 0)).collect();

        Self {
            keys,
            current_index: 0,
            usage_count,
            error_count,
        }
    }

    /// 获取下一个可用的 API Key（轮询 + 跳过错误过多的 key）
    fn get_next_key(&mut self) -> Option<String> {
        if self.keys.is_empty() {
            return None;
        }

        // 最多尝试所有 key
        for _ in 0..self.keys.len() {
            let key = &self.keys[self.current_index];
            self.current_index = (self.current_index + 1) % self.keys.len();

            // 跳过错误次数过多的 key（超过 3 次）
            if *self.error_count.get(key).unwrap_or(&0) < 3 {
                return Some(key.clone());
            }
        }

        // 所有 key 都有问题，重置错误计数并返回第一个
        warn!("所有 API Key 都有错误记录，重置错误计数");
        for count in self.error_count.values_mut() {
            *count = 0;
        }
        self.keys.first().cloned()
    }

    /// 记录成功使用
    fn record_success(&mut self, key: &str) {
        *self.usage_count.entry(key.to_string()).or_insert(0) += 1;
        // 成功后减少错误计数
        if let Some(count) = self.error_count.get_mut(key) {
            if *count > 0 {
                *count -= 1;
            }
        }
    }

    /// 记录错误
    fn record_error(&mut self, key: &str) {
        *self.error_count.entry(key.to_string()).or_insert(0) += 1;
        let error_count = self.error_count.get(key).unwrap_or(&0);
        warn!("API Key {}... 错误计数: {}", &key[..8.min(key.len())], error_count);
    }
}

// ============================================================================
// Tavily 搜索引擎
// ============================================================================

/// Tavily 搜索引擎
///
/// 特点：
/// - 专为 AI/LLM 优化的搜索 API
/// - 免费版每月 1000 次请求
/// - 返回结构化的搜索结果
///
/// 文档：https://docs.tavily.com/
pub struct TavilySearchProvider {
    name: String,
    key_manager: Arc<Mutex<ApiKeyManager>>,
    client: reqwest::Client,
}

impl TavilySearchProvider {
    pub fn new(api_keys: Vec<String>) -> Self {
        Self {
            name: "Tavily".to_string(),
            key_manager: Arc::new(Mutex::new(ApiKeyManager::new(api_keys))),
            client: reqwest::Client::new(),
        }
    }

    async fn do_search(&self, query: &str, api_key: &str, max_results: usize) -> Result<SearchResponse> {
        #[derive(Serialize)]
        struct TavilyRequest {
            query: String,
            search_depth: String,
            max_results: usize,
            include_answer: bool,
            include_raw_content: bool,
            days: u32,
        }

        #[derive(Deserialize, Debug)]
        struct TavilyResult {
            title: String,
            content: String,
            url: String,
            published_date: Option<String>,
        }

        #[derive(Deserialize, Debug)]
        struct TavilyResponse {
            results: Vec<TavilyResult>,
        }

        let request = TavilyRequest {
            query: query.to_string(),
            search_depth: "advanced".to_string(),
            max_results,
            include_answer: false,
            include_raw_content: false,
            days: 7,
        };

        let response = self
            .client
            .post("https://api.tavily.com/search")
            .header("Content-Type", "application/json")
            .json(&request)
            .bearer_auth(api_key)
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .context("Tavily API 请求失败")?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            
            let error_msg = if status.as_u16() == 429 || error_text.to_lowercase().contains("rate limit") {
                format!("API 配额已用尽: {}", error_text)
            } else {
                format!("HTTP {}: {}", status, error_text)
            };
            
            return Ok(SearchResponse::error(
                query.to_string(),
                self.name.clone(),
                error_msg,
            ));
        }

        let tavily_response: TavilyResponse = response.json().await.context("解析 Tavily 响应失败")?;

        info!(
            "[Tavily] 搜索完成，query='{}', 返回 {} 条结果",
            query,
            tavily_response.results.len()
        );
        debug!("[Tavily] 原始响应: {:?}", tavily_response);

        let results = tavily_response
            .results
            .into_iter()
            .map(|item| {
                let source = extract_domain(&item.url);
                let mut result = SearchResult {
                    title: item.title,
                    snippet: item.content.chars().take(500).collect(),
                    url: item.url.clone(),
                    source,
                    published_date: item.published_date,
                    news_type: NewsType::Other,
                    sentiment: Sentiment::Unknown,
                    importance: 5,
                    relevance: 0.6, // Tavily默认相关度
                    keywords: Vec::new(),
                };
                result.analyze_type();
                result.analyze_sentiment();
                result.calculate_importance();
                result
            })
            .collect();

        Ok(SearchResponse::success(
            query.to_string(),
            self.name.clone(),
            results,
        ))
    }
}

#[async_trait]
impl SearchProvider for TavilySearchProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_available(&self) -> bool {
        let manager = self.key_manager.lock().unwrap();
        !manager.keys.is_empty()
    }

    async fn search(&self, query: &str, max_results: usize) -> SearchResponse {
        let api_key = {
            let mut manager = self.key_manager.lock().unwrap();
            manager.get_next_key()
        };

        let api_key = match api_key {
            Some(key) => key,
            None => {
                return SearchResponse::error(
                    query.to_string(),
                    self.name.clone(),
                    "Tavily 未配置 API Key".to_string(),
                );
            }
        };

        let start_time = Instant::now();
        let mut response = match self.do_search(query, &api_key, max_results).await {
            Ok(resp) => resp,
            Err(e) => {
                let mut manager = self.key_manager.lock().unwrap();
                manager.record_error(&api_key);
                error!("[Tavily] 搜索 '{}' 失败: {}", query, e);
                SearchResponse::error(query.to_string(), self.name.clone(), e.to_string())
            }
        };

        response.search_time = start_time.elapsed().as_secs_f64();

        if response.success {
            let mut manager = self.key_manager.lock().unwrap();
            manager.record_success(&api_key);
            info!(
                "[Tavily] 搜索 '{}' 成功，返回 {} 条结果，耗时 {:.2}s",
                query,
                response.results.len(),
                response.search_time
            );
        } else {
            let mut manager = self.key_manager.lock().unwrap();
            manager.record_error(&api_key);
        }

        response
    }
}

// ============================================================================
// SerpAPI 搜索引擎
// ============================================================================

/// SerpAPI 搜索引擎
///
/// 特点：
/// - 支持 Google、Bing、百度等多种搜索引擎
/// - 免费版每月 100 次请求
/// - 返回真实的搜索结果
///
/// 文档：https://serpapi.com/
pub struct SerpAPISearchProvider {
    name: String,
    key_manager: Arc<Mutex<ApiKeyManager>>,
    client: reqwest::Client,
}

impl SerpAPISearchProvider {
    pub fn new(api_keys: Vec<String>) -> Self {
        Self {
            name: "SerpAPI".to_string(),
            key_manager: Arc::new(Mutex::new(ApiKeyManager::new(api_keys))),
            client: reqwest::Client::new(),
        }
    }

    async fn do_search(&self, query: &str, api_key: &str, max_results: usize) -> Result<SearchResponse> {
        #[derive(Deserialize, Debug)]
        struct OrganicResult {
            title: String,
            snippet: Option<String>,
            link: String,
            source: Option<String>,
            date: Option<String>,
        }

        #[derive(Deserialize, Debug)]
        struct SerpAPIResponse {
            organic_results: Option<Vec<OrganicResult>>,
        }

        let response = self
            .client
            .get("https://serpapi.com/search")
            .query(&[
                ("engine", "baidu"),
                ("q", query),
                ("api_key", api_key),
            ])
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .context("SerpAPI 请求失败")?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Ok(SearchResponse::error(
                query.to_string(),
                self.name.clone(),
                format!("HTTP {}: {}", status, error_text),
            ));
        }

        let serp_response: SerpAPIResponse = response.json().await.context("解析 SerpAPI 响应失败")?;

        debug!("[SerpAPI] 原始响应: {:?}", serp_response);

        let organic_results = serp_response.organic_results.unwrap_or_default();
        let results = organic_results
            .into_iter()
            .take(max_results)
            .map(|item| {
                let source = item
                    .source
                    .clone()
                    .unwrap_or_else(|| extract_domain(&item.link));
                let mut result = SearchResult {
                    title: item.title,
                    snippet: item.snippet.unwrap_or_default().chars().take(500).collect(),
                    url: item.link,
                    source,
                    published_date: item.date,
                    news_type: NewsType::Other,
                    sentiment: Sentiment::Unknown,
                    importance: 5,
                    relevance: 0.7, // SerpAPI默认相关度较高
                    keywords: Vec::new(),
                };
                result.analyze_type();
                result.analyze_sentiment();
                result.calculate_importance();
                result
            })
            .collect();

        Ok(SearchResponse::success(
            query.to_string(),
            self.name.clone(),
            results,
        ))
    }
}

#[async_trait]
impl SearchProvider for SerpAPISearchProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_available(&self) -> bool {
        let manager = self.key_manager.lock().unwrap();
        !manager.keys.is_empty()
    }

    async fn search(&self, query: &str, max_results: usize) -> SearchResponse {
        let api_key = {
            let mut manager = self.key_manager.lock().unwrap();
            manager.get_next_key()
        };

        let api_key = match api_key {
            Some(key) => key,
            None => {
                return SearchResponse::error(
                    query.to_string(),
                    self.name.clone(),
                    "SerpAPI 未配置 API Key".to_string(),
                );
            }
        };

        let start_time = Instant::now();
        let mut response = match self.do_search(query, &api_key, max_results).await {
            Ok(resp) => resp,
            Err(e) => {
                let mut manager = self.key_manager.lock().unwrap();
                manager.record_error(&api_key);
                error!("[SerpAPI] 搜索 '{}' 失败: {}", query, e);
                SearchResponse::error(query.to_string(), self.name.clone(), e.to_string())
            }
        };

        response.search_time = start_time.elapsed().as_secs_f64();

        if response.success {
            let mut manager = self.key_manager.lock().unwrap();
            manager.record_success(&api_key);
            info!(
                "[SerpAPI] 搜索 '{}' 成功，返回 {} 条结果，耗时 {:.2}s",
                query,
                response.results.len(),
                response.search_time
            );
        } else {
            let mut manager = self.key_manager.lock().unwrap();
            manager.record_error(&api_key);
        }

        response
    }
}

// ============================================================================
// Bocha 搜索引擎
// ============================================================================

/// Bocha 搜索引擎
///
/// 特点：
/// - 专为AI优化的中文搜索API
/// - 结果准确、摘要完整
/// - 支持时间范围过滤和AI摘要
/// - 兼容Bing Search API格式
///
/// 文档：https://bocha-ai.feishu.cn/wiki/RXEOw02rFiwzGSkd9mUcqoeAnNK
pub struct BochaSearchProvider {
    name: String,
    key_manager: Arc<Mutex<ApiKeyManager>>,
    client: reqwest::Client,
}

impl BochaSearchProvider {
    pub fn new(api_keys: Vec<String>) -> Self {
        Self {
            name: "Bocha".to_string(),
            key_manager: Arc::new(Mutex::new(ApiKeyManager::new(api_keys))),
            client: reqwest::Client::new(),
        }
    }

    async fn do_search(&self, query: &str, api_key: &str, max_results: usize) -> Result<SearchResponse> {
        #[derive(Serialize)]
        struct BochaRequest {
            query: String,
            freshness: String,
            summary: bool,
            count: usize,
        }

        #[derive(Deserialize, Debug)]
        struct WebPage {
            name: String,
            snippet: Option<String>,
            summary: Option<String>,
            url: String,
            #[serde(rename = "siteName")]
            site_name: Option<String>,
            #[serde(rename = "datePublished")]
            date_published: Option<String>,
        }

        #[derive(Deserialize, Debug)]
        struct WebPages {
            value: Vec<WebPage>,
        }

        #[derive(Deserialize, Debug)]
        struct BochaData {
            #[serde(rename = "webPages")]
            web_pages: Option<WebPages>,
        }

        #[derive(Deserialize, Debug)]
        struct BochaResponse {
            code: u32,
            msg: Option<String>,
            data: Option<BochaData>,
        }

        let request = BochaRequest {
            query: query.to_string(),
            freshness: "oneMonth".to_string(),
            summary: true,
            count: max_results.min(50),
        };

        let response = self
            .client
            .post("https://api.bocha.cn/v1/web-search")
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .context("Bocha API 请求失败")?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            
            let error_msg = match status.as_u16() {
                403 => format!("余额不足: {}", error_text),
                401 => format!("API KEY无效: {}", error_text),
                400 => format!("请求参数错误: {}", error_text),
                429 => format!("请求频率达到限制: {}", error_text),
                _ => format!("HTTP {}: {}", status, error_text),
            };
            
            warn!("[Bocha] 搜索失败: {}", error_msg);
            return Ok(SearchResponse::error(
                query.to_string(),
                self.name.clone(),
                error_msg,
            ));
        }

        let bocha_response: BochaResponse = response
            .json()
            .await
            .context("解析 Bocha 响应失败")?;

        if bocha_response.code != 200 {
            let error_msg = bocha_response
                .msg
                .unwrap_or_else(|| format!("API返回错误码: {}", bocha_response.code));
            return Ok(SearchResponse::error(
                query.to_string(),
                self.name.clone(),
                error_msg,
            ));
        }

        info!("[Bocha] 搜索完成，query='{}'", query);
        debug!("[Bocha] 原始响应: {:?}", bocha_response);

        let results = bocha_response
            .data
            .and_then(|d| d.web_pages)
            .map(|wp| wp.value)
            .unwrap_or_default()
            .into_iter()
            .take(max_results)
            .map(|item| {
                let snippet = item
                    .summary
                    .or(item.snippet)
                    .unwrap_or_default()
                    .chars()
                    .take(500)
                    .collect();

                let source = item
                    .site_name
                    .unwrap_or_else(|| extract_domain(&item.url));

                let mut result = SearchResult {
                    title: item.name,
                    snippet,
                    url: item.url,
                    source,
                    published_date: item.date_published,
                    news_type: NewsType::Other,
                    sentiment: Sentiment::Unknown,
                    importance: 5,
                    relevance: 0.8, // Bocha中文搜索优化，相关度较高
                    keywords: Vec::new(),
                };
                result.analyze_type();
                result.analyze_sentiment();
                result.calculate_importance();
                result
            })
            .collect::<Vec<_>>();

        info!("[Bocha] 成功解析 {} 条结果", results.len());

        Ok(SearchResponse::success(
            query.to_string(),
            self.name.clone(),
            results,
        ))
    }
}

#[async_trait]
impl SearchProvider for BochaSearchProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_available(&self) -> bool {
        let manager = self.key_manager.lock().unwrap();
        !manager.keys.is_empty()
    }

    async fn search(&self, query: &str, max_results: usize) -> SearchResponse {
        let api_key = {
            let mut manager = self.key_manager.lock().unwrap();
            manager.get_next_key()
        };

        let api_key = match api_key {
            Some(key) => key,
            None => {
                return SearchResponse::error(
                    query.to_string(),
                    self.name.clone(),
                    "Bocha 未配置 API Key".to_string(),
                );
            }
        };

        let start_time = Instant::now();
        let mut response = match self.do_search(query, &api_key, max_results).await {
            Ok(resp) => resp,
            Err(e) => {
                let mut manager = self.key_manager.lock().unwrap();
                manager.record_error(&api_key);
                error!("[Bocha] 搜索 '{}' 失败: {}", query, e);
                SearchResponse::error(query.to_string(), self.name.clone(), e.to_string())
            }
        };

        response.search_time = start_time.elapsed().as_secs_f64();

        if response.success {
            let mut manager = self.key_manager.lock().unwrap();
            manager.record_success(&api_key);
            info!(
                "[Bocha] 搜索 '{}' 成功，返回 {} 条结果，耗时 {:.2}s",
                query,
                response.results.len(),
                response.search_time
            );
        } else {
            let mut manager = self.key_manager.lock().unwrap();
            manager.record_error(&api_key);
        }

        response
    }
}

// ============================================================================
// 华尔街见闻 直连API
// ============================================================================

/// 华尔街见闻 快讯/文章直连抓取
///
/// 特点：
/// - 完全免费，无需 API Key
/// - 覆盖全球财经资讯、美联储、高盛研报、地缘政治等
/// - 快讯更新及时（分钟级）
pub struct WallStreetCnProvider {
    name: String,
    client: reqwest::Client,
}

impl WallStreetCnProvider {
    pub fn new() -> Self {
        Self {
            name: "华尔街见闻".to_string(),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
                .build()
                .unwrap(),
        }
    }

    /// 获取最新快讯（全球财经）
    pub async fn fetch_live_news(&self, page_size: usize) -> Result<Vec<SearchResult>> {
        #[derive(Deserialize, Debug)]
        struct LiveItem {
            content_text: Option<String>,
            created_at: Option<i64>,
        }

        #[derive(Deserialize, Debug)]
        struct LiveData {
            items: Option<Vec<LiveItem>>,
        }

        #[derive(Deserialize, Debug)]
        struct LiveResp {
            code: Option<i64>,
            data: Option<LiveData>,
        }

        let url = format!(
            "https://api.wallstreetcn.com/apiv1/content/lives?channel=global-channel&cursor=0&pageSize={}&accept=0",
            page_size
        );

        let resp: LiveResp = self.client
            .get(&url)
            .header("Origin", "https://wallstreetcn.com")
            .header("Referer", "https://wallstreetcn.com/")
            .send().await
            .context("华尔街见闻快讯请求失败")?
            .json().await
            .context("华尔街见闻快讯解析失败")?;

        if resp.code != Some(20000) {
            return Err(anyhow::anyhow!("华尔街见闻API返回错误码: {:?}", resp.code));
        }

        let items = resp.data.and_then(|d| d.items).unwrap_or_default();
        let now = chrono::Local::now().timestamp();

        let results: Vec<SearchResult> = items.into_iter()
            .filter_map(|item| {
                let text = item.content_text.filter(|t| !t.is_empty())?;
                // 过滤6小时以外的旧新闻
                if let Some(ts) = item.created_at {
                    if now - ts > 6 * 3600 {
                        return None;
                    }
                }
                let date_tag = item.created_at.map(|ts| {
                    let dt = chrono::DateTime::from_timestamp(ts, 0)
                        .unwrap_or_default()
                        .with_timezone(&chrono::Local);
                    dt.format("%H:%M").to_string()
                });
                let title: String = text.chars().take(60).collect();
                let snippet: String = text.chars().take(200).collect();
                Some(SearchResult::new(
                    title,
                    snippet,
                    format!("https://wallstreetcn.com/"),
                    "华尔街见闻".to_string(),
                ).with_date(date_tag.unwrap_or_default()))
            })
            .collect();

        Ok(results)
    }

    /// 获取最新文章（深度分析）
    pub async fn fetch_articles(&self, page_size: usize) -> Result<Vec<SearchResult>> {
        #[derive(Deserialize, Debug)]
        struct ArticleItem {
            title: Option<String>,
            summary: Option<String>,
            display_time: Option<i64>,
            content_uri: Option<String>,
        }

        #[derive(Deserialize, Debug)]
        struct ArticleData {
            items: Option<Vec<ArticleItem>>,
        }

        #[derive(Deserialize, Debug)]
        struct ArticleResp {
            code: Option<i64>,
            data: Option<ArticleData>,
        }

        let url = format!(
            "https://api.wallstreetcn.com/apiv1/content/articles?channel=global-channel&accept=article&cursor=0&pageSize={}",
            page_size
        );

        let resp: ArticleResp = self.client
            .get(&url)
            .header("Origin", "https://wallstreetcn.com")
            .header("Referer", "https://wallstreetcn.com/")
            .send().await
            .context("华尔街见闻文章请求失败")?
            .json().await
            .context("华尔街见闻文章解析失败")?;

        if resp.code != Some(20000) {
            return Err(anyhow::anyhow!("华尔街见闻文章API错误: {:?}", resp.code));
        }

        let items = resp.data.and_then(|d| d.items).unwrap_or_default();
        let now = chrono::Local::now().timestamp();

        let results: Vec<SearchResult> = items.into_iter()
            .filter_map(|item| {
                let title = item.title.filter(|t| !t.is_empty())?;
                if let Some(ts) = item.display_time {
                    if now - ts > 24 * 3600 { return None; }
                }
                let snippet = item.summary.unwrap_or_default();
                let uri = item.content_uri.unwrap_or_default();
                let url = if uri.starts_with("http") { uri } else {
                    format!("https://wallstreetcn.com/articles/{}", uri)
                };
                let date_tag = item.display_time.map(|ts| {
                    let dt = chrono::DateTime::from_timestamp(ts, 0)
                        .unwrap_or_default()
                        .with_timezone(&chrono::Local);
                    dt.format("%H:%M").to_string()
                });
                Some(SearchResult::new(title, snippet, url, "华尔街见闻".to_string())
                    .with_date(date_tag.unwrap_or_default()))
            })
            .collect();

        Ok(results)
    }
}

#[async_trait]
impl SearchProvider for WallStreetCnProvider {
    fn name(&self) -> &str { &self.name }
    fn is_available(&self) -> bool { true }

    async fn search(&self, query: &str, max_results: usize) -> SearchResponse {
        let start = Instant::now();
        // 搜索逻辑：同时拉快讯和文章，按标题/内容过滤关键词
        let keywords: Vec<&str> = query.split_whitespace()
            .filter(|w| w.len() >= 2)
            .take(5)
            .collect();

        let (live_res, article_res) = tokio::join!(
            self.fetch_live_news(30),
            self.fetch_articles(20),
        );

        let mut all: Vec<SearchResult> = Vec::new();
        if let Ok(items) = live_res { all.extend(items); }
        if let Ok(items) = article_res { all.extend(items); }

        // 过滤相关条目
        let filtered: Vec<SearchResult> = if keywords.is_empty() {
            all.into_iter().take(max_results).collect()
        } else {
            all.into_iter()
                .filter(|r| {
                    let text = format!("{} {}", r.title, r.snippet).to_lowercase();
                    keywords.iter().any(|kw| text.contains(&kw.to_lowercase()))
                })
                .take(max_results)
                .collect()
        };

        let success = !filtered.is_empty();
        SearchResponse {
            query: query.to_string(),
            results: filtered,
            provider: self.name.clone(),
            success,
            error_message: if success { None } else { Some("无相关结果".to_string()) },
            search_time: start.elapsed().as_secs_f64(),
        }
    }
}

// ============================================================================
// 东方财富 新闻API
// ============================================================================

/// 东方财富新闻API
///
/// 特点：
/// - 完全免费，无需API Key
/// - A股专业财经资讯
/// - 实时公告、新闻、研报
/// - 数据权威，更新快
///
/// 接口文档：https://quote.eastmoney.com/
pub struct EastmoneyNewsProvider {
    name: String,
    client: reqwest::Client,
}

impl EastmoneyNewsProvider {
    pub fn new() -> Self {
        Self {
            name: "东方财富".to_string(),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap(),
        }
    }

    async fn do_search(&self, query: &str, max_results: usize) -> Result<SearchResponse> {
        // 从查询中提取股票代码和名称
        let (_stock_code, stock_name) = Self::parse_query(query);
        
        // 东方财富支持纯关键词搜索，不强制要求股票代码
        let search_keyword = if !stock_name.is_empty() {
            stock_name
        } else {
            // 使用原始查询作为关键词（去掉"股票""最新""消息"等停用词后的部分）
            query.to_string()
        };

        if search_keyword.is_empty() {
            return Ok(SearchResponse::error(
                query.to_string(),
                self.name.clone(),
                "无法从查询中提取搜索关键词".to_string(),
            ));
        }

        let start = Instant::now();
        
        // 东方财富新闻搜索API
        // 示例: http://search-api-web.eastmoney.com/search/jsonp?cb=jQuery&param=...
        let url = format!(
            "http://search-api-web.eastmoney.com/search/jsonp?cb=jQuery&param=%7B%22uid%22%3A%22%22%2C%22keyword%22%3A%22{}%22%2C%22type%22%3A%5B%22cmsArticleWebOld%22%5D%2C%22client%22%3A%22web%22%2C%22clientType%22%3A%22web%22%2C%22clientVersion%22%3A%22curr%22%2C%22param%22%3A%7B%22cmsArticleWebOld%22%3A%7B%22searchScope%22%3A%22default%22%2C%22sort%22%3A%22default%22%2C%22pageIndex%22%3A1%2C%22pageSize%22%3A{}%7D%7D%7D",
            urlencoding::encode(&search_keyword),
            max_results
        );

        let response_text = self
            .client
            .get(&url)
            .header("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36")
            .header("Referer", "http://so.eastmoney.com/")
            .send()
            .await
            .context("东方财富API请求失败")?
            .text()
            .await
            .context("读取响应失败")?;

        // 移除 JSONP 包装：jQuery(...) -> ...
        let json_text = response_text
            .trim()
            .strip_prefix("jQuery(")
            .and_then(|s| s.strip_suffix(")"))
            .unwrap_or(&response_text);

        #[derive(Deserialize, Debug)]
        struct EastmoneyArticle {
            title: String,
            content: Option<String>,
            url: String,
            date: Option<String>,
            #[serde(rename = "mediaName")]
            media_name: Option<String>,
        }

        #[derive(Deserialize, Debug)]
        struct EastmoneyResult {
            #[serde(rename = "cmsArticleWebOld")]
            cms_article: Option<Vec<EastmoneyArticle>>,
        }

        #[derive(Deserialize, Debug)]
        struct EastmoneyResponse {
            code: i32,
            msg: String,
            result: Option<EastmoneyResult>,
        }

        let parsed: EastmoneyResponse = serde_json::from_str(json_text)
            .context("解析东方财富响应失败")?;

        if parsed.code != 0 {
            return Ok(SearchResponse::error(
                query.to_string(),
                self.name.clone(),
                format!("API返回错误: {}", parsed.msg),
            ));
        }

        let articles = parsed
            .result
            .and_then(|r| r.cms_article)
            .unwrap_or_default();

        info!(
            "[东方财富] 搜索完成，query='{}', 返回 {} 条结果",
            query,
            articles.len()
        );

        let results = articles
            .into_iter()
            .map(|item| {
                let source = item.media_name.unwrap_or_else(|| "东方财富".to_string());
                let snippet = item.content
                    .unwrap_or_else(|| item.title.clone())
                    .chars()
                    .take(500)
                    .collect();

                let mut result = SearchResult {
                    title: item.title,
                    snippet,
                    url: item.url,
                    source,
                    published_date: item.date,
                    news_type: NewsType::Other,
                    sentiment: Sentiment::Unknown,
                    importance: 6, // 东方财富权威性较高
                    relevance: 0.8, // A股专业平台，相关性高
                    keywords: Vec::new(),
                };
                result.analyze_type();
                result.analyze_sentiment();
                result
            })
            .collect();

        Ok(SearchResponse {
            query: query.to_string(),
            results,
            success: true,
            provider: self.name.clone(),
            error_message: None,
            search_time: start.elapsed().as_secs_f64(),
        })
    }

    /// 从查询中解析股票代码和名称/关键词
    /// 例如: "贵州茅台 600519 股票 最新消息" -> ("600519", "贵州茅台")
    /// 也支持: "金风 持股 投资 收购 参股" -> ("", "金风 持股 投资 收购 参股")
    fn parse_query(query: &str) -> (String, String) {
        let parts: Vec<&str> = query.split_whitespace().collect();
        
        let mut code = String::new();
        let mut name_parts: Vec<String> = Vec::new();
        let stop_words = ["股票", "最新", "消息", "新闻"];
        
        for part in &parts {
            // 匹配6位数字的股票代码
            if part.len() == 6 && part.chars().all(|c| c.is_ascii_digit()) {
                code = part.to_string();
            } else if !stop_words.iter().any(|w| part.contains(w)) {
                // 收集非停用词作为搜索关键词
                name_parts.push(part.to_string());
            }
        }
        
        (code, name_parts.join(" "))
    }
}

#[async_trait]
impl SearchProvider for EastmoneyNewsProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_available(&self) -> bool {
        true // 东方财富API免费，始终可用
    }

    async fn search(&self, query: &str, max_results: usize) -> SearchResponse {
        match self.do_search(query, max_results).await {
            Ok(response) => {
                info!(
                    "[东方财富] 搜索 '{}' 成功，返回 {} 条结果，耗时 {:.2}s",
                    query,
                    response.results.len(),
                    response.search_time
                );
                response
            }
            Err(e) => {
                error!("[东方财富] 搜索失败: {}", e);
                SearchResponse::error(query.to_string(), self.name.clone(), e.to_string())
            }
        }
    }
}

// ============================================================================
// SearchService 主服务
// ============================================================================

/// 搜索服务
///
/// 功能：
/// 1. 管理多个搜索引擎
/// 2. 自动故障转移
/// 3. 结果聚合和格式化
pub struct SearchService {
    providers: Vec<Box<dyn SearchProvider>>,
    /// 华尔街见闻直连（免费，用于宏观新闻）
    wscn: WallStreetCnProvider,
}

impl SearchService {
    /// 创建新的搜索服务
    pub fn new(
        bocha_keys: Option<Vec<String>>,
        tavily_keys: Option<Vec<String>>,
        serpapi_keys: Option<Vec<String>>,
        enable_eastmoney: bool,
    ) -> Self {
        let mut providers: Vec<Box<dyn SearchProvider>> = Vec::new();

        // 按优先级添加搜索引擎
        // 1. SerpAPI 最优先（Google搜索结果，质量高）
        if let Some(keys) = serpapi_keys {
            if !keys.is_empty() {
                info!("已配置 SerpAPI 搜索，共 {} 个 API Key", keys.len());
                providers.push(Box::new(SerpAPISearchProvider::new(keys)));
            }
        }

        // 2. 东方财富（免费，A股专业，无需API Key）
        if enable_eastmoney {
            info!("已启用 东方财富 新闻搜索（免费，无限制）");
            providers.push(Box::new(EastmoneyNewsProvider::new()));
        }

        // 3. Bocha（中文搜索优化，AI摘要）
        if let Some(keys) = bocha_keys {
            if !keys.is_empty() {
                info!("已配置 Bocha 搜索，共 {} 个 API Key", keys.len());
                providers.push(Box::new(BochaSearchProvider::new(keys)));
            }
        }

        // 4. Tavily（免费额度更多，每月 1000 次）
        if let Some(keys) = tavily_keys {
            if !keys.is_empty() {
                info!("已配置 Tavily 搜索，共 {} 个 API Key", keys.len());
                providers.push(Box::new(TavilySearchProvider::new(keys)));
            }
        }

        if providers.is_empty() {
            warn!("未配置任何搜索引擎，新闻搜索功能将不可用");
        }

        info!("已启用 华尔街见闻 直连（免费，全球财经快讯）");
        Self { providers, wscn: WallStreetCnProvider::new() }
    }

    /// 检查是否有可用的搜索引擎
    pub fn is_available(&self) -> bool {
        self.providers.iter().any(|p| p.is_available())
    }

    /// 搜索股票相关新闻（多维度扩展关键词）
    pub async fn search_stock_news(
        &self,
        stock_code: &str,
        stock_name: &str,
        max_results: usize,
    ) -> SearchResponse {
        info!("搜索股票新闻: {}({})", stock_name, stock_code);

        // 提取股票简称（去掉常见后缀，如"科技"、"集团"、"股份"等保留核心词）
        let short_name = Self::extract_short_name(stock_name);

        // 构建多维度搜索查询
        let mut queries = vec![
            // 维度1: 基本新闻（全称 + 代码）
            format!("{} {} 股票 最新消息", stock_name, stock_code),
        ];

        // 维度2: 持股/投资/并购相关（用简称扩大搜索范围）
        let invest_name = if short_name != stock_name { &short_name } else { stock_name };
        queries.push(format!("{} 持股 投资 收购 参股", invest_name));

        // 维度3: 行业/合作/订单（简称搜索）
        queries.push(format!("{} 合作 中标 订单 签约", invest_name));

        let mut all_results: Vec<SearchResult> = Vec::new();
        let mut success_provider = String::new();
        let mut total_search_time = 0.0;

        for (dim_idx, query) in queries.iter().enumerate() {
            // 每个维度取少量结果，合并后再截断
            let per_query_max = if dim_idx == 0 { max_results } else { 3_usize.min(max_results) };

            for provider in &self.providers {
                if !provider.is_available() {
                    continue;
                }

                let response = provider.search(query, per_query_max).await;
                total_search_time += response.search_time;

                if response.success && !response.results.is_empty() {
                    if success_provider.is_empty() {
                        success_provider = response.provider.clone();
                    } else if !success_provider.contains(&response.provider) {
                        success_provider = format!("{}+{}", success_provider, response.provider);
                    }
                    info!("[维度{}] 使用 {} 搜索 '{}' 获得 {} 条结果",
                        dim_idx + 1, response.provider, query, response.results.len());
                    all_results.extend(response.results);
                    break; // 该维度搜索成功，不再尝试其他引擎
                } else {
                    warn!(
                        "[维度{}] {} 搜索失败: {}，尝试下一个引擎",
                        dim_idx + 1,
                        provider.name(),
                        response.error_message.as_deref().unwrap_or("未知错误")
                    );
                }
            }

            // 维度之间短暂延迟，避免请求过快
            if dim_idx < queries.len() - 1 {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }

        if all_results.is_empty() {
            return SearchResponse::error(
                queries[0].clone(),
                "None".to_string(),
                "所有搜索引擎都不可用或搜索失败".to_string(),
            );
        }

        // 去重（按URL去重）
        let mut seen_urls = std::collections::HashSet::new();
        all_results.retain(|r| seen_urls.insert(r.url.clone()));

        // 为每个结果提取关键词并计算相关性
        for result in &mut all_results {
            result.extract_keywords(stock_name, stock_code);

            let title_lower = result.title.to_lowercase();
            let stock_name_lower = stock_name.to_lowercase();
            let short_name_lower = short_name.to_lowercase();
            // 全称匹配加分最多
            if title_lower.contains(&stock_name_lower) || title_lower.contains(stock_code) {
                result.relevance = (result.relevance + 0.3).min(1.0);
            }
            // 简称匹配也加分
            if short_name_lower != stock_name_lower && title_lower.contains(&short_name_lower) {
                result.relevance = (result.relevance + 0.2).min(1.0);
            }
            // 包含持股/投资/并购等高价值关键词加重要性
            let high_value_keywords = ["持股", "投资", "收购", "参股", "并购", "入股", "中标", "签约", "订单"];
            for kw in &high_value_keywords {
                if title_lower.contains(kw) || result.snippet.contains(kw) {
                    result.importance = result.importance.saturating_add(1).min(10);
                    break;
                }
            }
        }

        // 按重要性和相关性排序
        all_results.sort_by(|a, b| {
            let score_a = (a.importance as f32) * a.relevance;
            let score_b = (b.importance as f32) * b.relevance;
            score_b.partial_cmp(&score_a).unwrap_or(std::cmp::Ordering::Equal)
        });

        // 截断到max_results + 2（多保留一点给AI更多上下文）
        all_results.truncate(max_results + 2);

        info!("多维度搜索完成: {} 条结果（去重后），来源: {}", all_results.len(), success_provider);

        SearchResponse {
            query: format!("{} {} 多维度搜索", stock_name, stock_code),
            results: all_results,
            success: true,
            provider: success_provider,
            error_message: None,
            search_time: total_search_time,
        }
    }

    /// 从股票全称中提取简称/核心词
    /// 例如: "金风科技" -> "金风", "越秀资本" -> "越秀", "贵州茅台" -> "茅台"
    fn extract_short_name(stock_name: &str) -> String {
        // 常见后缀词（按长度从长到短排列，优先匹配长的）
        let suffixes = [
            "电子科技", "高新技术", "信息技术",
            "新材料", "新能源", "生物科技",
            "科技", "集团", "股份", "控股", "实业", "产业",
            "资本", "投资", "金融", "银行", "证券", "保险",
            "医药", "制药", "生物",
            "电气", "电子", "电力", "能源", "环保",
            "汽车", "机械", "材料", "化工", "建设",
            "通信", "传媒", "文化", "教育", "旅游",
            "食品", "乳业", "酿酒", "地产", "置业",
            "物流", "航空", "航天",
        ];

        // 常见地名前缀
        let prefixes = [
            "贵州", "云南", "四川", "山东", "江苏", "浙江", "广东", "福建",
            "河南", "河北", "湖南", "湖北", "安徽", "江西", "陕西", "山西",
            "辽宁", "吉林", "黑龙", "甘肃", "青海", "海南", "广西", "内蒙",
            "新疆", "西藏", "宁夏", "上海", "北京", "天津", "重庆", "深圳",
        ];

        let mut name = stock_name.to_string();

        // 先去后缀
        for suffix in &suffixes {
            if name.ends_with(suffix) && name.len() > suffix.len() {
                name = name[..name.len() - suffix.len()].to_string();
                break;
            }
        }

        // 再去地名前缀
        for prefix in &prefixes {
            if name.starts_with(prefix) && name.chars().count() > prefix.chars().count() {
                let prefix_len = prefix.len();
                name = name[prefix_len..].to_string();
                break;
            }
        }

        // 如果处理后太短（<2个字），返回原名
        if name.chars().count() < 2 {
            return stock_name.to_string();
        }

        name
    }

    /// 搜索股票特定事件
    pub async fn search_stock_events(
        &self,
        stock_code: &str,
        stock_name: &str,
        event_types: Option<Vec<&str>>,
    ) -> SearchResponse {
        let events = event_types.unwrap_or_else(|| vec!["年报预告", "减持公告", "业绩快报"]);
        let event_query = events.join(" OR ");
        let query = format!("{} ({})", stock_name, event_query);

        info!("搜索股票事件: {}({}) - {:?}", stock_name, stock_code, events);

        for provider in &self.providers {
            if !provider.is_available() {
                continue;
            }

            let response = provider.search(&query, 5).await;

            if response.success {
                return response;
            }
        }

        SearchResponse::error(query, "None".to_string(), "事件搜索失败".to_string())
    }

    /// 多维度情报搜索
    pub async fn search_comprehensive_intel(
        &self,
        stock_code: &str,
        stock_name: &str,
        max_searches: usize,
    ) -> HashMap<String, SearchResponse> {
        let mut results = HashMap::new();
        let mut search_count = 0;

        // 定义搜索维度
        let search_dimensions = vec![
            (
                "latest_news",
                format!("{} {} 最新 新闻 2026年1月", stock_name, stock_code),
                "最新消息",
            ),
            (
                "risk_check",
                format!("{} 减持 处罚 利空 风险", stock_name),
                "风险排查",
            ),
            (
                "earnings",
                format!("{} 年报预告 业绩预告 业绩快报 2025年报", stock_name),
                "业绩预期",
            ),
        ];

        info!("开始多维度情报搜索: {}({})", stock_name, stock_code);

        let mut provider_index = 0;
        let available_providers: Vec<_> = self
            .providers
            .iter()
            .filter(|p| p.is_available())
            .collect();

        if available_providers.is_empty() {
            return results;
        }

        for (dim_name, query, desc) in search_dimensions {
            if search_count >= max_searches {
                break;
            }

            let provider = available_providers[provider_index % available_providers.len()];
            provider_index += 1;

            info!("[情报搜索] {}: 使用 {}", desc, provider.name());

            let response = provider.search(&query, 3).await;

            if response.success {
                info!("[情报搜索] {}: 获取 {} 条结果", desc, response.results.len());
            } else {
                warn!(
                    "[情报搜索] {}: 搜索失败 - {}",
                    desc,
                    response.error_message.as_deref().unwrap_or("未知错误")
                );
            }

            results.insert(dim_name.to_string(), response);
            search_count += 1;

            // 短暂延迟避免请求过快
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        results
    }

    /// 格式化情报搜索结果为报告
    pub fn format_intel_report(
        intel_results: &HashMap<String, SearchResponse>,
        stock_name: &str,
    ) -> String {
        let mut lines = vec![format!("【{} 情报搜索结果】", stock_name)];

        // 最新消息
        if let Some(resp) = intel_results.get("latest_news") {
            lines.push(format!("\n📰 最新消息 (来源: {}):", resp.provider));
            if resp.success && !resp.results.is_empty() {
                for (i, r) in resp.results.iter().take(3).enumerate() {
                    let date_str = r
                        .published_date
                        .as_ref()
                        .map(|d| format!(" [{}]", d))
                        .unwrap_or_default();
                    lines.push(format!("  {}. {}{}", i + 1, r.title, date_str));
                    lines.push(format!(
                        "     {}...",
                        r.snippet.chars().take(100).collect::<String>()
                    ));
                }
            } else {
                lines.push("  未找到相关消息".to_string());
            }
        }

        // 风险排查
        if let Some(resp) = intel_results.get("risk_check") {
            lines.push(format!("\n⚠️ 风险排查 (来源: {}):", resp.provider));
            if resp.success && !resp.results.is_empty() {
                for (i, r) in resp.results.iter().take(3).enumerate() {
                    lines.push(format!("  {}. {}", i + 1, r.title));
                    lines.push(format!(
                        "     {}...",
                        r.snippet.chars().take(100).collect::<String>()
                    ));
                }
            } else {
                lines.push("  未发现明显风险信号".to_string());
            }
        }

        // 业绩预期
        if let Some(resp) = intel_results.get("earnings") {
            lines.push(format!("\n📊 业绩预期 (来源: {}):", resp.provider));
            if resp.success && !resp.results.is_empty() {
                for (i, r) in resp.results.iter().take(3).enumerate() {
                    lines.push(format!("  {}. {}", i + 1, r.title));
                    lines.push(format!(
                        "     {}...",
                        r.snippet.chars().take(100).collect::<String>()
                    ));
                }
            } else {
                lines.push("  未找到业绩相关信息".to_string());
            }
        }

        lines.join("\n")
    }

    /// 搜索当日宏观/国际/市场最新新闻（所有股票共享，只搜索一次）
    ///
    /// 搜索维度：
    /// 1. 今日 A 股市场 + 大盘动态
    /// 2. 国际财经 + 地缘政治最新要闻
    /// 3. 美股 / 欧股 / 大宗商品今日行情
    /// 4. 国内宏观政策（央行、财政、产业）
    pub async fn search_macro_news(&self, max_results: usize) -> String {
        let today = chrono::Local::now().format("%Y年%m月%d日").to_string();

        let mut sections: Vec<String> = Vec::new();

        // ── 第一步：华尔街见闻直连快讯（最新6小时，无需API Key）── 并发拉取
        let (live_res, article_res) = tokio::join!(
            self.wscn.fetch_live_news(30),
            self.wscn.fetch_articles(10),
        );
        let mut wscn_items: Vec<String> = Vec::new();
        if let Ok(lives) = live_res {
            for r in lives.iter().take(5) {
                let t = r.published_date.as_deref().unwrap_or("");
                let snippet: String = r.snippet.chars().take(120).collect();
                wscn_items.push(format!("- **{}** {}  \n  {}", r.title, t, snippet));
            }
        }
        if let Ok(articles) = article_res {
            for r in articles.iter().take(3) {
                let t = r.published_date.as_deref().unwrap_or("");
                let snippet: String = r.snippet.chars().take(120).collect();
                wscn_items.push(format!("- **{}** {}  \n  {}", r.title, t, snippet));
            }
        }
        if !wscn_items.is_empty() {
            sections.push(format!("### 🌐 华尔街见闻快讯（今日实时）\n{}", wscn_items.join("\n")));
            info!("[宏观新闻][wscn] 华尔街见闻获取 {} 条", wscn_items.len());
        } else {
            warn!("[宏观新闻][wscn] 华尔街见闻返回为空或超时");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;

        // ── 第二步：搜索引擎多维度查询 ──
        // (维度key, 查询关键词, 展示标题)
        let search_dims: Vec<(&str, String, &str)> = vec![
            ("a_market",    format!("{}A股 大盘 股市 最新动态", today),              "### 🇨🇳 A股市场动态"),
            ("global",      format!("{}国际财经 地缘政治 最新消息", today),           "### 🌍 国际财经 / 地缘政治"),
            ("us_market",   format!("{}美股 美联储 大宗商品 今日", today),            "### 🇺🇸 美股 / 大宗商品"),
            ("cn_policy",   format!("{}中国 央行 财政 产业政策 重要新闻", today),     "### 📋 宏观政策"),
            ("institution", format!("{}高盛 摩根 大摩 美银 JPMorgan 中国A股 市场观点 研报", today),
                                                                                    "### 🏦 投行观点（高盛/摩根/美银）"),
            ("fin_media",   format!("{}证券时报 第一财经 21世纪经济报道 重要财经", today),
                                                                                    "### 📰 财经媒体要闻"),
        ];

        for (dim, query, header) in &search_dims {
            let mut found = false;
            for provider in &self.providers {
                if !provider.is_available() { continue; }
                let resp = provider.search(query, max_results.min(3)).await;
                if resp.success && !resp.results.is_empty() {
                    let lines: Vec<String> = resp.results.iter().take(3).map(|r| {
                        let date_tag = r.published_date.as_deref().unwrap_or("");
                        let snippet_short: String = r.snippet.chars().take(150).collect();
                        format!("- **{}** {}  \n  {}", r.title, date_tag, snippet_short)
                    }).collect();
                    sections.push(format!("{}\n{}", header, lines.join("\n")));
                    info!("[宏观新闻][{}] {} 获取 {} 条", dim, resp.provider, resp.results.len());
                    found = true;
                    break;
                }
            }
            if !found {
                warn!("[宏观新闻][{}] 所有引擎均失败，跳过该维度", dim);
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }

        if sections.is_empty() {
            return String::new();
        }

        format!("## 📡 今日宏观 / 市场背景（{}）\n\n{}", today, sections.join("\n\n"))
    }

    /// 批量搜索多只股票新闻
    pub async fn batch_search(
        &self,
        stocks: Vec<(&str, &str)>, // (code, name)
        max_results_per_stock: usize,
        delay_between: Duration,
    ) -> HashMap<String, SearchResponse> {
        let mut results = HashMap::new();

        for (i, (code, name)) in stocks.iter().enumerate() {
            if i > 0 {
                tokio::time::sleep(delay_between).await;
            }

            let response = self.search_stock_news(code, name, max_results_per_stock).await;
            results.insert(code.to_string(), response);
        }

        results
    }
}

// ============================================================================
// 工具函数
// ============================================================================

/// 从 URL 提取域名作为来源
fn extract_domain(url: &str) -> String {
    match Url::parse(url) {
        Ok(parsed) => parsed
            .host_str()
            .unwrap_or("未知来源")
            .replace("www.", ""),
        Err(_) => "未知来源".to_string(),
    }
}

// ============================================================================
// 单例服务
// ============================================================================

use once_cell::sync::OnceCell;

static SEARCH_SERVICE: OnceCell<SearchService> = OnceCell::new();

/// 获取搜索服务单例
pub fn get_search_service() -> &'static SearchService {
    SEARCH_SERVICE.get_or_init(|| {
        // 从环境变量读取配置
        let bocha_keys = std::env::var("BOCHA_API_KEYS")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.split(',').map(|k| k.trim().to_string()).collect());

        let tavily_keys = std::env::var("TAVILY_API_KEYS")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.split(',').map(|k| k.trim().to_string()).collect());

        let serpapi_keys = std::env::var("SERPAPI_KEYS")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.split(',').map(|k| k.trim().to_string()).collect());

        // 东方财富默认启用（免费无限制）
        let enable_eastmoney = std::env::var("ENABLE_EASTMONEY_NEWS")
            .unwrap_or_else(|_| "true".to_string())
            .to_lowercase() != "false";

        SearchService::new(bocha_keys, tavily_keys, serpapi_keys, enable_eastmoney)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_search_service() {
        env_logger::init();

        let service = get_search_service();

        if service.is_available() {
            println!("=== 测试股票新闻搜索 ===");
            let response = service.search_stock_news("300389", "艾比森", 5).await;
            println!("搜索状态: {}", if response.success { "成功" } else { "失败" });
            println!("搜索引擎: {}", response.provider);
            println!("结果数量: {}", response.results.len());
            println!("耗时: {:.2}s", response.search_time);
            println!("\n{}", response.to_context(5));
        } else {
            println!("未配置搜索引擎 API Key，跳过测试");
        }
    }
}
