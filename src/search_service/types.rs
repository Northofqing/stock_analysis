//! 搜索服务共享类型与抽象（原 search_service.rs 头部）

use std::collections::HashMap;

use async_trait::async_trait;
use log::warn;
use serde::{Deserialize, Serialize};

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
pub(crate) struct ApiKeyManager {
    pub(crate) keys: Vec<String>,
    current_index: usize,
    usage_count: HashMap<String, usize>,
    error_count: HashMap<String, usize>,
}

impl ApiKeyManager {
    pub(crate) fn new(keys: Vec<String>) -> Self {
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
    pub(crate) fn get_next_key(&mut self) -> Option<String> {
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
    pub(crate) fn record_success(&mut self, key: &str) {
        *self.usage_count.entry(key.to_string()).or_insert(0) += 1;
        // 成功后减少错误计数
        if let Some(count) = self.error_count.get_mut(key) {
            if *count > 0 {
                *count -= 1;
            }
        }
    }

    /// 记录错误
    pub(crate) fn record_error(&mut self, key: &str) {
        *self.error_count.entry(key.to_string()).or_insert(0) += 1;
        let error_count = self.error_count.get(key).unwrap_or(&0);
        warn!("API Key {}... 错误计数: {}", &key[..8.min(key.len())], error_count);
    }
}


// ============================================================================
// 共享工具函数
// ============================================================================

/// 从 URL 提取域名作为来源（原 search_service.rs 中的 extract_domain）
pub(crate) fn extract_domain(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(parsed) => parsed
            .host_str()
            .unwrap_or("未知来源")
            .replace("www.", ""),
        Err(_) => "未知来源".to_string(),
    }
}
