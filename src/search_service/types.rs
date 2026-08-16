//! 搜索服务共享类型与抽象（原 search_service.rs 头部）

use async_trait::async_trait;
use crate::magic_compat::ProviderId;
use serde::{Deserialize, Serialize};

use crate::data_gateway::{BatchEvidence, GlobalNewsRecord};

// ============================================================================
// 数据结构
// ============================================================================

/// One current-day global-news fact with the exact immutable Gateway batch
/// that admitted it.
///
/// The record retains its provider-owned [`crate::magic_compat::SourceEvidence`].
/// Keeping both levels together prevents downstream candidate scoring from
/// turning a title into an unattributed string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FreshFlashFact {
    pub record: GlobalNewsRecord,
    pub batch_evidence: BatchEvidence,
}

/// Explicit outcome for every independently requested global-news provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlashSourceStatus {
    Available {
        evidence: BatchEvidence,
        admitted_records: usize,
        stale_records: usize,
        macro_records: usize,
    },
    VerifiedEmpty(BatchEvidence),
    Unavailable {
        provider: ProviderId,
        source: String,
        reason_code: String,
        retryable: bool,
        message: String,
    },
}

/// Evidence-preserving aggregate over independently complete provider batches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlashFactBatch {
    pub facts: Vec<FreshFlashFact>,
    pub source_statuses: Vec<FlashSourceStatus>,
}

/// All global-news providers were unavailable or returned invalid projection
/// evidence. A verified empty batch is not represented by this error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("flash facts unavailable reason_code={reason_code} retryable={retryable}")]
pub struct FlashFactsUnavailable {
    pub reason_code: &'static str,
    pub retryable: bool,
    pub source_statuses: Vec<FlashSourceStatus>,
}

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

/// Provenance and allowed use carried by a legacy search-shaped record.
///
/// BR-175: deserialized legacy values are deliberately `Unverified`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(tag = "scope", rename_all = "snake_case")]
pub enum SearchEvidence {
    #[default]
    Unverified,
    ResearchOnly {
        provider: crate::data_gateway::GeneralWebResearchProvider,
        source: String,
        observed_at: String,
        batch_id: String,
        item_id: String,
        publication_quality: crate::data_gateway::PublicationTimeQuality,
    },
    GovernedSourceFact {
        provider: String,
        source: String,
        observed_at: String,
        source_at: String,
        batch_id: String,
        item_id: String,
    },
}

impl SearchEvidence {
    pub fn is_research_only(&self) -> bool {
        matches!(self, Self::ResearchOnly { .. })
    }

    pub fn is_complete_governed_source_fact(&self) -> bool {
        match self {
            Self::GovernedSourceFact {
                provider,
                source,
                observed_at,
                source_at,
                batch_id,
                item_id,
            } => [provider, source, observed_at, source_at, batch_id, item_id]
                .into_iter()
                .all(|value| !value.trim().is_empty()),
            Self::Unverified | Self::ResearchOnly { .. } => false,
        }
    }
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
    /// 来源证据和允许用途。
    #[serde(default)]
    pub evidence: SearchEvidence,
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
            evidence: SearchEvidence::Unverified,
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
    /// 结构化失败；成功或 verified-empty 时为空。
    #[serde(default)]
    pub failure: Option<SearchFailureEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SearchFailureEvidence {
    pub reason_code: String,
    pub retryable: bool,
    pub stage: String,
}

impl SearchResponse {
    /// 将搜索结果转换为可用于 AI 分析的上下文
    pub fn to_context(&self, max_results: usize) -> String {
        if !self.success || self.results.is_empty() {
            return format!("搜索 '{}' 未找到相关结果。", self.query);
        }
        if self
            .results
            .iter()
            .any(|result| !result.evidence.is_research_only())
        {
            return format!(
                "搜索 '{}' 的研究上下文被拒绝：缺少完整 ResearchOnly evidence。",
                self.query
            );
        }

        let mut lines = vec![format!(
            "【{} 研究发现】（ResearchOnly；来源：{}；不得作为金融事实或交易/选股依据）",
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
            failure: Some(SearchFailureEvidence {
                reason_code: "search_failed".to_string(),
                retryable: true,
                stage: "search_service".to_string(),
            }),
        }
    }

    pub fn typed_error(
        query: String,
        provider: String,
        error_message: String,
        reason_code: impl Into<String>,
        retryable: bool,
        stage: impl Into<String>,
    ) -> Self {
        Self {
            query,
            results: Vec::new(),
            provider,
            success: false,
            error_message: Some(error_message),
            search_time: 0.0,
            failure: Some(SearchFailureEvidence {
                reason_code: reason_code.into(),
                retryable,
                stage: stage.into(),
            }),
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
            failure: None,
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

    /// Whether this adapter is a user-authorized general web-search engine.
    ///
    /// Financial-source adapters stay false so they cannot become an implicit
    /// fallback when a governed data Gateway is unavailable.
    fn supports_general_web_search(&self) -> bool {
        false
    }

    /// 执行搜索
    async fn search(&self, query: &str, max_results: usize) -> SearchResponse;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(title: &str, snippet: &str) -> SearchResult {
        let mut result = SearchResult::new(
            title.to_string(),
            snippet.to_string(),
            "https://example.invalid/TEST_CODE".to_string(),
            "测试来源".to_string(),
        );
        result.evidence = SearchEvidence::ResearchOnly {
            provider: crate::data_gateway::GeneralWebResearchProvider::Bocha,
            source: "TEST_CODE_research".to_string(),
            observed_at: "2026-07-28T08:00:00Z".to_string(),
            batch_id: "TEST_CODE_batch".to_string(),
            item_id: title.to_string(),
            publication_quality: crate::data_gateway::PublicationTimeQuality::Missing,
        };
        result
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
        assert!(context.contains("ResearchOnly"));
        assert!(context.contains("不得作为金融事实"));
        assert!(context.contains("1. "));
        assert!(context.contains("第一条"));
        assert!(!context.contains("第二条"));

        let mut unverified = result("未验证", "摘要");
        unverified.evidence = SearchEvidence::Unverified;
        let rejected = SearchResponse::success(
            "测试查询".to_string(),
            "测试引擎".to_string(),
            vec![unverified],
        )
        .to_context(1);
        assert!(rejected.contains("被拒绝"));
        assert!(rejected.contains("ResearchOnly evidence"));
    }
}
