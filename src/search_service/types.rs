//! 搜索服务共享类型与抽象（原 search_service.rs 头部）

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

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
        } else if text.contains("财报")
            || text.contains("业绩")
            || text.contains("营收")
            || text.contains("利润")
            || text.contains("季报")
            || text.contains("年报")
        {
            self.news_type = NewsType::Earnings;
        } else if text.contains("政策")
            || text.contains("监管")
            || text.contains("证监会")
            || text.contains("交易所")
        {
            self.news_type = NewsType::Policy;
        } else if text.contains("行业") || text.contains("板块") || text.contains("赛道") {
            self.news_type = NewsType::Industry;
        } else if text.contains("分析")
            || text.contains("研报")
            || text.contains("评级")
            || text.contains("研究")
        {
            self.news_type = NewsType::Analysis;
        } else if text.contains("风险")
            || text.contains("警示")
            || text.contains("违规")
            || text.contains("调查")
            || text.contains("处罚")
        {
            self.news_type = NewsType::Risk;
        }
    }

    /// 分析并设置情感倾向
    pub fn analyze_sentiment(&mut self) {
        let text = format!("{} {}", self.title, self.snippet).to_lowercase();

        // 利好关键词
        let positive_keywords = [
            "涨",
            "上涨",
            "增长",
            "突破",
            "利好",
            "盈利",
            "增加",
            "提升",
            "创新高",
            "超预期",
            "中标",
            "合作",
            "签约",
            "订单",
            "扩产",
            "收购",
            "增持",
            "买入",
            "推荐",
            "看好",
            "龙头",
        ];

        // 利空关键词
        let negative_keywords = [
            "跌",
            "下跌",
            "下滑",
            "亏损",
            "利空",
            "风险",
            "警示",
            "违规",
            "处罚",
            "调查",
            "减持",
            "卖出",
            "业绩预警",
            "商誉减值",
            "诉讼",
            "质押",
            "停牌",
            "ST",
            "退市",
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
            "重大", "重要", "紧急", "突发", "独家", "首次", "涨停", "跌停", "停牌", "复牌",
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
            "涨停", "跌停", "增长", "下滑", "业绩", "财报", "营收", "利润", "市值", "股价", "研发",
            "创新", "合作", "订单", "中标", "政策", "监管", "风险", "违规", "重组", "并购",
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

    /// 是否支持"主题词/自然语言"搜索.
    /// false = 该 provider 只能按股票代码/公告检索 (如交易所/巨潮), 主题搜索时应排除,
    ///          避免对宽泛主题词反复报"需提供代码/空结果"噪声.
    /// BR-036: 主题搜索能力位过滤规则.
    fn supports_topic_search(&self) -> bool {
        true
    }

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
        warn!(
            "API Key {}... 错误计数: {}",
            &key[..8.min(key.len())],
            error_count
        );
    }
}

/// 提取 key 或返回错误响应（消除重复的 lock→get_next_key→unwrap 模式）
pub(crate) fn get_key_or_error(
    key_manager: &Arc<Mutex<ApiKeyManager>>,
    provider_name: &str,
    query: &str,
) -> Result<String, SearchResponse> {
    let mut manager = key_manager.lock().unwrap();
    manager.get_next_key().ok_or_else(|| {
        SearchResponse::error(
            query.to_string(),
            provider_name.to_string(),
            format!("{provider_name} 未配置 API Key"),
        )
    })
}

/// 记录 API 调用结果到 key_manager
pub(crate) fn record_key_result(
    key_manager: &Arc<Mutex<ApiKeyManager>>,
    api_key: &str,
    success: bool,
) {
    let mut manager = key_manager.lock().unwrap();
    if success {
        manager.record_success(api_key);
    } else {
        manager.record_error(api_key);
    }
}

/// key_manager 是否有可用 key（消除三个 provider 中重复的 is_available 锁模式）
pub(crate) fn key_manager_available(key_manager: &Arc<Mutex<ApiKeyManager>>) -> bool {
    !key_manager.lock().unwrap().keys.is_empty()
}

/// 模板方法：封装「取 key → do_search → 计时 → 记录成功/失败」统一流程，
/// 消除 bocha / serpapi / tavily 三个 provider 中重复的编排代码。
pub(crate) async fn run_key_managed_search<F, Fut>(
    key_manager: &Arc<Mutex<ApiKeyManager>>,
    provider_name: &str,
    query: &str,
    do_search: F,
) -> SearchResponse
where
    F: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<SearchResponse>> + Send,
{
    let api_key = match get_key_or_error(key_manager, provider_name, query) {
        Ok(k) => k,
        Err(resp) => return resp,
    };

    let start_time = std::time::Instant::now();
    let mut response = match do_search(api_key.clone()).await {
        Ok(resp) => resp,
        Err(e) => {
            record_key_result(key_manager, &api_key, false);
            log::error!("[{}] 搜索 '{}' 失败: {}", provider_name, query, e);
            return SearchResponse::error(
                query.to_string(),
                provider_name.to_string(),
                e.to_string(),
            );
        }
    };

    response.search_time = start_time.elapsed().as_secs_f64();
    record_key_result(key_manager, &api_key, response.success);
    if response.success {
        log::info!(
            "[{}] 搜索 '{}' 成功，返回 {} 条结果，耗时 {:.2}s",
            provider_name,
            query,
            response.results.len(),
            response.search_time
        );
    }

    response
}

// ============================================================================
// 共享工具函数
// ============================================================================

/// 从 URL 提取域名作为来源（原 search_service.rs 中的 extract_domain）
pub(crate) fn extract_domain(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(parsed) => parsed.host_str().unwrap_or("未知来源").replace("www.", ""),
        Err(_) => "未知来源".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(title: &str, snippet: &str) -> SearchResult {
        SearchResult::new(
            title.to_string(),
            snippet.to_string(),
            "https://example.invalid/TEST_CODE".to_string(),
            "测试来源".to_string(),
        )
    }

    #[test]
    fn result_analysis_covers_every_registered_type_and_sentiment_state() {
        let type_cases = [
            ("发布公告", NewsType::Announcement),
            ("季度业绩增长", NewsType::Earnings),
            ("监管政策", NewsType::Policy),
            ("行业赛道", NewsType::Industry),
            ("研究评级", NewsType::Analysis),
            ("违规处罚", NewsType::Risk),
            ("普通消息", NewsType::Other),
        ];
        for (text, expected) in type_cases {
            let mut item = result(text, "");
            item.analyze_type();
            assert_eq!(item.news_type, expected);
        }

        let sentiment_cases = [
            ("盈利增长突破", Sentiment::Positive),
            ("亏损下滑处罚", Sentiment::Negative),
            ("增长但有亏损", Sentiment::Neutral),
            ("普通消息", Sentiment::Unknown),
        ];
        for (text, expected) in sentiment_cases {
            let mut item = result(text, "");
            item.analyze_sentiment();
            assert_eq!(item.sentiment, expected);
        }
    }

    #[test]
    fn result_render_importance_keywords_and_builders_preserve_explicit_evidence() {
        let type_cases = [
            NewsType::Announcement,
            NewsType::Earnings,
            NewsType::Policy,
            NewsType::Industry,
            NewsType::Analysis,
            NewsType::Risk,
            NewsType::Other,
        ];
        let sentiment_cases = [
            Sentiment::Positive,
            Sentiment::Neutral,
            Sentiment::Negative,
            Sentiment::Unknown,
        ];
        for news_type in type_cases {
            for sentiment in &sentiment_cases {
                let mut item = result("测试标题", "测试摘要");
                item.news_type = news_type.clone();
                item.sentiment = sentiment.clone();
                item.importance = 9;
                item.relevance = 0.75;
                item.keywords = vec!["TEST_CODE".to_string()];
                let text = item.to_text();
                assert!(text.contains("测试来源"));
                assert!(text.contains("★★★★★"));
                assert!(text.contains("75%"));
                assert!(text.contains("TEST_CODE"));
            }
        }

        let mut empty_date = result("x", "y").with_date(String::new());
        assert_eq!(empty_date.published_date, None);
        empty_date = empty_date.with_date("2026-07-19".to_string());
        assert_eq!(empty_date.published_date.as_deref(), Some("2026-07-19"));

        for (news_type, base) in [
            (NewsType::Announcement, 7),
            (NewsType::Earnings, 8),
            (NewsType::Risk, 8),
            (NewsType::Policy, 7),
            (NewsType::Other, 5),
        ] {
            let mut item = result("普通", "消息");
            item.news_type = news_type;
            item.calculate_importance();
            assert_eq!(item.importance, base);
        }
        let mut capped = result("重大重要紧急突发独家首次涨停跌停停牌复牌", "");
        capped.news_type = NewsType::Earnings;
        capped.calculate_importance();
        assert_eq!(capped.importance, 10);

        let mut keyword_item = result(
            "TEST_CODE_600000 测试公司涨停并购",
            "业绩增长、研发创新、合作订单中标，政策监管风险违规",
        );
        keyword_item.extract_keywords("测试公司", "TEST_CODE_600000");
        assert_eq!(keyword_item.keywords[0], "TEST_CODE_600000");
        assert_eq!(keyword_item.keywords[1], "测试公司");
        assert!(keyword_item.keywords.contains(&"涨停".to_string()));
        assert!(keyword_item.keywords.contains(&"并购".to_string()));
        let mut no_identity = result("普通", "消息");
        no_identity.extract_keywords("未出现公司", "TEST_CODE_missing");
        assert!(no_identity.keywords.is_empty());
    }

    #[test]
    fn response_constructors_and_context_cover_empty_success_and_limits() {
        let error = SearchResponse::error(
            "测试查询".to_string(),
            "测试引擎".to_string(),
            "来源失败".to_string(),
        );
        assert!(!error.success);
        assert_eq!(error.error_message.as_deref(), Some("来源失败"));
        assert!(error.to_context(3).contains("未找到相关结果"));

        let empty =
            SearchResponse::success("测试查询".to_string(), "测试引擎".to_string(), Vec::new());
        assert!(empty.success);
        assert!(empty.to_context(3).contains("未找到相关结果"));

        let success = SearchResponse::success(
            "测试查询".to_string(),
            "测试引擎".to_string(),
            vec![result("第一条", "摘要一"), result("第二条", "摘要二")],
        );
        let context = success.to_context(1);
        assert!(context.contains("【测试查询 搜索结果】"));
        assert!(context.contains("1. "));
        assert!(context.contains("第一条"));
        assert!(!context.contains("第二条"));
    }

    #[test]
    fn api_key_manager_rotates_skips_resets_and_records_results() {
        let mut manager = ApiKeyManager::new(vec!["key-a".to_string(), "key-b".to_string()]);
        assert_eq!(manager.get_next_key().as_deref(), Some("key-a"));
        assert_eq!(manager.get_next_key().as_deref(), Some("key-b"));
        manager.record_error("key-a");
        manager.record_error("key-a");
        manager.record_error("key-a");
        assert_eq!(manager.get_next_key().as_deref(), Some("key-b"));

        let mut exhausted = ApiKeyManager::new(vec!["short".to_string()]);
        for _ in 0..3 {
            exhausted.record_error("short");
        }
        assert_eq!(exhausted.get_next_key().as_deref(), Some("short"));
        assert_eq!(exhausted.error_count["short"], 0);
        exhausted.record_error("short");
        exhausted.record_success("short");
        assert_eq!(exhausted.error_count["short"], 0);
        assert_eq!(exhausted.usage_count["short"], 1);
        assert_eq!(ApiKeyManager::new(Vec::new()).get_next_key(), None);
    }

    #[tokio::test]
    async fn managed_search_preserves_success_failure_and_missing_key_states() {
        let manager = Arc::new(Mutex::new(ApiKeyManager::new(vec![
            "TEST_CODE_key".to_string()
        ])));
        assert!(key_manager_available(&manager));
        assert_eq!(
            get_key_or_error(&manager, "测试引擎", "测试查询").unwrap(),
            "TEST_CODE_key"
        );

        let response = run_key_managed_search(&manager, "测试引擎", "成功查询", |key| async move {
            assert_eq!(key, "TEST_CODE_key");
            Ok(SearchResponse::success(
                "成功查询".to_string(),
                "测试引擎".to_string(),
                vec![result("命中", "证据")],
            ))
        })
        .await;
        assert!(response.success);
        assert!(response.search_time >= 0.0);

        let response = run_key_managed_search(&manager, "测试引擎", "空查询", |_| async {
            Ok(SearchResponse::error(
                "空查询".to_string(),
                "测试引擎".to_string(),
                "明确无匹配".to_string(),
            ))
        })
        .await;
        assert!(!response.success);

        let response = run_key_managed_search(&manager, "测试引擎", "失败查询", |_| async {
            Err(anyhow::anyhow!("TEST_CODE transport failed"))
        })
        .await;
        assert!(!response.success);
        assert_eq!(
            response.error_message.as_deref(),
            Some("TEST_CODE transport failed")
        );

        record_key_result(&manager, "TEST_CODE_key", true);
        record_key_result(&manager, "TEST_CODE_key", false);
        let empty = Arc::new(Mutex::new(ApiKeyManager::new(Vec::new())));
        assert!(!key_manager_available(&empty));
        let missing = run_key_managed_search(&empty, "测试引擎", "缺密钥", |_| async {
            unreachable!("missing key must stop before provider call")
        })
        .await;
        assert!(!missing.success);
        assert!(missing.error_message.unwrap().contains("未配置 API Key"));
    }

    #[test]
    fn domain_extraction_handles_valid_missing_host_and_invalid_urls() {
        assert_eq!(extract_domain("https://www.example.com/a"), "example.com");
        assert_eq!(extract_domain("mailto:test@example.com"), "未知来源");
        assert_eq!(extract_domain("not a url"), "未知来源");
    }
}
