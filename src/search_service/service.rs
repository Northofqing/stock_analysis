//! 搜索服务聚合器（原 search_service.rs 尾部）

use std::collections::{HashMap, HashSet, VecDeque};
// 修复 Top10#6 (2026-06-29 audit): 保留 std::sync::Mutex — `recent_topic_signatures: VecDeque`,
// `source_health: HashMap`, `source_health_ticks: u64` 都是微秒级内存修改, lock 持有 < 100ns.
// 改 tokio Mutex 会要求所有调用方改 async；当前锁只保护微秒级内存状态，保留 std.
// audit 列的 5 处 std::sync::Mutex 中 analyzer/mod.rs:454 **实际已是** tokio::sync::Mutex.
// 其他 4 处 (本文件 + adaptive + rate_budget + industry) 都是 sync API + 微秒级持有, 保留 std.
use std::sync::Mutex;
use std::time::Duration;

use log::{debug, info, warn};

use crate::config::get_monitor_config;
use crate::data_gateway::{
    EconomicCalendarGateway, GatewayBatch, GatewayError, GeneralWebResearchProvider,
    GlobalNewsGateway, GlobalNewsProvider, GlobalNewsRecord,
};

use super::macro_news::render_gateway_sections;
use super::providers::GeneralWebSearchProvider;
use super::types::{
    FlashFactBatch, FlashFactsUnavailable, FlashSourceStatus, FreshFlashFact, SearchProvider,
    SearchResponse, SearchResult,
};

#[derive(Debug)]
struct ProjectedFlashOutcome {
    facts: Vec<FreshFlashFact>,
    status: FlashSourceStatus,
}

fn project_gateway_flash_outcome(
    provider: GlobalNewsProvider,
    outcome: Result<GatewayBatch<GlobalNewsRecord>, GatewayError>,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<ProjectedFlashOutcome, String> {
    let expected_provider = provider.provider_id();
    let expected_source = provider.source();
    match outcome {
        Ok(GatewayBatch::Available { records, evidence }) => {
            if evidence.provider != expected_provider
                || evidence.source != expected_source
                || evidence.source_at.as_deref().is_none_or(str::is_empty)
                || evidence.observed_at.trim().is_empty()
                || evidence.batch_id.trim().is_empty()
            {
                return Err(format!(
                    "{} returned mismatched or incomplete batch evidence",
                    provider.feed_name()
                ));
            }

            let mut facts = Vec::with_capacity(records.len());
            let mut stale_records = 0usize;
            let mut macro_records = 0usize;
            for record in records {
                if record.evidence.provider() != evidence.provider
                    || record.evidence.batch_id() != evidence.batch_id
                    || record.evidence.observed_at() != evidence.observed_at
                    || record.evidence.source_at().is_none_or(str::is_empty)
                {
                    return Err(format!(
                        "{} record {} evidence differs from batch",
                        provider.feed_name(),
                        record.item_id
                    ));
                }
                if record.published_at > now || record.observed_at > now {
                    return Err(format!(
                        "{} record {} has future publication or observation time",
                        provider.feed_name(),
                        record.item_id
                    ));
                }
                if record
                    .published_at
                    .with_timezone(&chrono::Local)
                    .date_naive()
                    != now.with_timezone(&chrono::Local).date_naive()
                {
                    stale_records += 1;
                    continue;
                }
                if is_macro_title(&record.title) {
                    macro_records += 1;
                    continue;
                }
                facts.push(FreshFlashFact {
                    record,
                    batch_evidence: evidence.clone(),
                });
            }
            Ok(ProjectedFlashOutcome {
                status: FlashSourceStatus::Available {
                    evidence,
                    admitted_records: facts.len(),
                    stale_records,
                    macro_records,
                },
                facts,
            })
        }
        Ok(GatewayBatch::VerifiedEmpty(evidence)) => {
            if evidence.provider != expected_provider
                || evidence.source != expected_source
                || evidence.source_at.as_deref().is_none_or(str::is_empty)
                || evidence.observed_at.trim().is_empty()
                || evidence.batch_id.trim().is_empty()
            {
                return Err(format!(
                    "{} returned mismatched or incomplete empty-batch evidence",
                    provider.feed_name()
                ));
            }
            Ok(ProjectedFlashOutcome {
                facts: Vec::new(),
                status: FlashSourceStatus::VerifiedEmpty(evidence),
            })
        }
        Err(error) => Ok(ProjectedFlashOutcome {
            facts: Vec::new(),
            status: FlashSourceStatus::Unavailable {
                provider: expected_provider,
                source: expected_source.to_string(),
                reason_code: error.reason_code().to_string(),
                retryable: error.retryable(),
                message: error.to_string(),
            },
        }),
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
    /// 最近入选主题新闻标题特征（用于抑制重复推送）
    recent_topic_signatures: Mutex<VecDeque<String>>,
    /// 新闻源健康统计（成功/超时/失败/空结果）
    source_health: Mutex<HashMap<String, SourceHealthStats>>,
    /// 汇总日志触发计数（每 N 次打印一次）
    source_health_ticks: Mutex<u64>,
}

#[derive(Clone, Copy)]
struct TopicRerankParams {
    relevance_weight: f32,
    diversity_penalty: f32,
    history_penalty: f32,
}

#[derive(Default, Clone)]
struct SourceHealthStats {
    attempts: u64,
    success: u64,
    error: u64,
    empty: u64,
    items: u64,
}

#[derive(Clone, Copy)]
enum SourceFetchOutcome {
    Success,
    Error,
    Empty,
}

impl SearchService {
    /// 创建新的搜索服务
    pub fn new(
        bocha_keys: Option<Vec<String>>,
        tavily_keys: Option<Vec<String>>,
        serpapi_keys: Option<Vec<String>>,
    ) -> Self {
        let mut providers: Vec<Box<dyn SearchProvider>> = Vec::new();

        // BR-164: this registry is only generic, user-authorized web research.
        // Governed financial facts are acquired exclusively by data_gateway.
        if let Some(keys) = serpapi_keys {
            if !keys.is_empty() {
                info!("已配置 SerpAPI 搜索，共 {} 个 API Key", keys.len());
                providers.push(Box::new(GeneralWebSearchProvider::new(
                    GeneralWebResearchProvider::SerpApi,
                    keys,
                )));
            }
        }

        // 6. Bocha（付费，中文搜索优化，AI摘要）
        if let Some(keys) = bocha_keys {
            if !keys.is_empty() {
                info!("已配置 Bocha 搜索，共 {} 个 API Key", keys.len());
                providers.push(Box::new(GeneralWebSearchProvider::new(
                    GeneralWebResearchProvider::Bocha,
                    keys,
                )));
            }
        }

        // 7. Tavily（限免，作为最后补充）
        if let Some(keys) = tavily_keys {
            if !keys.is_empty() {
                info!("已配置 Tavily 搜索，共 {} 个 API Key", keys.len());
                providers.push(Box::new(GeneralWebSearchProvider::new(
                    GeneralWebResearchProvider::Tavily,
                    keys,
                )));
            }
        }

        if providers.is_empty() {
            warn!("未配置任何搜索引擎，新闻搜索功能将不可用");
        }

        let cfg = get_monitor_config();
        Self {
            providers,
            recent_topic_signatures: Mutex::new(VecDeque::with_capacity(
                cfg.topic_history_memory_size.max(50),
            )),
            source_health: Mutex::new(HashMap::new()),
            source_health_ticks: Mutex::new(0),
        }
    }

    /// Production factory. BR-175 keeps credential names and parsing inside
    /// `GeneralWebResearchGateway`.
    pub fn from_environment() -> Self {
        let mut providers: Vec<Box<dyn SearchProvider>> = Vec::new();
        for provider in [
            GeneralWebResearchProvider::SerpApi,
            GeneralWebResearchProvider::Bocha,
            GeneralWebResearchProvider::Tavily,
        ] {
            let adapter = GeneralWebSearchProvider::from_environment(provider);
            if adapter.is_available() {
                info!("已配置 {} 通用网页研究 Gateway", adapter.name());
                providers.push(Box::new(adapter));
            }
        }
        if providers.is_empty() {
            warn!("未配置任何通用网页研究 Gateway");
        }
        let cfg = get_monitor_config();
        Self {
            providers,
            recent_topic_signatures: Mutex::new(VecDeque::with_capacity(
                cfg.topic_history_memory_size.max(50),
            )),
            source_health: Mutex::new(HashMap::new()),
            source_health_ticks: Mutex::new(0),
        }
    }

    fn record_source_health(&self, source: &str, outcome: SourceFetchOutcome, items: usize) {
        if let Ok(mut guard) = self.source_health.lock() {
            let stat = guard.entry(source.to_string()).or_default();
            stat.attempts += 1;
            stat.items += items as u64;
            match outcome {
                SourceFetchOutcome::Success => stat.success += 1,
                SourceFetchOutcome::Error => stat.error += 1,
                SourceFetchOutcome::Empty => stat.empty += 1,
            }
        }
    }

    fn maybe_log_source_health_summary(&self, reason: &str) {
        let should_log = if let Ok(mut ticks) = self.source_health_ticks.lock() {
            *ticks += 1;
            *ticks % 20 == 0
        } else {
            false
        };
        if !should_log {
            return;
        }

        if let Ok(guard) = self.source_health.lock() {
            if guard.is_empty() {
                return;
            }

            let mut lines = Vec::new();
            for (source, stat) in guard.iter() {
                if stat.attempts == 0 {
                    continue;
                }
                let success_rate = stat.success as f64 * 100.0 / stat.attempts as f64;
                lines.push(format!(
                    "{}: 成功 {}/{} ({:.1}%), 错误 {}, 空结果 {}, 累计条数 {}",
                    source,
                    stat.success,
                    stat.attempts,
                    success_rate,
                    stat.error,
                    stat.empty,
                    stat.items
                ));
            }

            if !lines.is_empty() {
                info!("[source-health][{}] {}", reason, lines.join(" | "));
            }
        }
    }

    /// Fetch four independently audited global-news batches without reducing
    /// records to unattributed title strings.
    ///
    /// Independent source failures remain in `source_statuses`. The aggregate
    /// is unavailable only when every source failed; a verified empty response
    /// remains distinguishable from unavailability.
    pub async fn fetch_flash_facts(
        &self,
        per_source_limit: usize,
    ) -> Result<FlashFactBatch, FlashFactsUnavailable> {
        if !(1..=20).contains(&per_source_limit) {
            return Err(FlashFactsUnavailable {
                reason_code: "invalid_limit",
                retryable: false,
                source_statuses: Vec::new(),
            });
        }
        let per_source_limit =
            u32::try_from(per_source_limit).expect("validated global-news limit <= 20");
        let gateway = GlobalNewsGateway::new();
        let (eastmoney, cailianpress, jin10, thepaper) = tokio::join!(
            gateway.global_news(GlobalNewsProvider::Eastmoney, per_source_limit),
            gateway.global_news(GlobalNewsProvider::Cailianpress, per_source_limit),
            gateway.global_news(GlobalNewsProvider::Jin10, per_source_limit),
            gateway.global_news(GlobalNewsProvider::ThePaper, per_source_limit),
        );
        let now = chrono::Utc::now();
        let outcomes = [
            (GlobalNewsProvider::Eastmoney, eastmoney),
            (GlobalNewsProvider::Cailianpress, cailianpress),
            (GlobalNewsProvider::Jin10, jin10),
            (GlobalNewsProvider::ThePaper, thepaper),
        ];
        let mut facts = Vec::new();
        let mut source_statuses = Vec::with_capacity(outcomes.len());
        let mut complete_sources = 0usize;
        for (provider, outcome) in outcomes {
            let projected = match project_gateway_flash_outcome(provider, outcome, now) {
                Ok(projected) => projected,
                Err(message) => ProjectedFlashOutcome {
                    facts: Vec::new(),
                    status: FlashSourceStatus::Unavailable {
                        provider: provider.provider_id(),
                        source: provider.source().to_string(),
                        reason_code: "invalid_projection_evidence".to_string(),
                        retryable: false,
                        message,
                    },
                },
            };
            match &projected.status {
                FlashSourceStatus::Available {
                    evidence,
                    admitted_records,
                    stale_records,
                    macro_records,
                } => {
                    complete_sources += 1;
                    self.record_source_health(
                        provider.feed_name(),
                        SourceFetchOutcome::Success,
                        *admitted_records,
                    );
                    info!(
                        "[flash][gateway][BR-164] provider={} source={} batch_id={} \
                         admitted={} stale_excluded={} macro_excluded={}",
                        provider.feed_name(),
                        evidence.source,
                        evidence.batch_id,
                        admitted_records,
                        stale_records,
                        macro_records
                    );
                }
                FlashSourceStatus::VerifiedEmpty(evidence) => {
                    complete_sources += 1;
                    self.record_source_health(provider.feed_name(), SourceFetchOutcome::Empty, 0);
                    info!(
                        "[flash][gateway][BR-164] provider={} verified_empty source={} batch_id={}",
                        provider.feed_name(),
                        evidence.source,
                        evidence.batch_id
                    );
                }
                FlashSourceStatus::Unavailable {
                    reason_code,
                    retryable,
                    message,
                    ..
                } => {
                    self.record_source_health(provider.feed_name(), SourceFetchOutcome::Error, 0);
                    warn!(
                        "[flash][gateway][BR-164] provider={} unavailable \
                         reason_code={} retryable={}",
                        provider.feed_name(),
                        reason_code,
                        retryable,
                    );
                    debug!(
                        "[flash][gateway][BR-164] provider={} unavailable_detail={}",
                        provider.feed_name(),
                        message
                    );
                }
            }
            facts.extend(projected.facts);
            source_statuses.push(projected.status);
        }

        self.maybe_log_source_health_summary("fetch_flash_facts");
        if complete_sources == 0 {
            return Err(FlashFactsUnavailable {
                reason_code: "all_sources_unavailable",
                retryable: source_statuses.iter().any(|status| {
                    matches!(
                        status,
                        FlashSourceStatus::Unavailable {
                            retryable: true,
                            ..
                        }
                    )
                }),
                source_statuses,
            });
        }
        Ok(FlashFactBatch {
            facts,
            source_statuses,
        })
    }

    /// 检查是否有可用的搜索引擎
    pub fn is_available(&self) -> bool {
        self.providers.iter().any(|p| p.is_available())
    }

    /// 尽力解析新闻发布日期，返回距今天数（用于主题新闻新鲜度过滤）。
    ///
    /// 兼容多种 provider 的日期格式：
    /// - ISO / RFC3339 / RFC2822（Tavily、Bocha）
    /// - 中文相对时间（百度/SerpAPI）："今天/昨天/前天/N分钟前/N小时前/N天前/N周前/N个月前"
    /// - 中文绝对日期："YYYY年M月D日" / "M月D日"（无年份按今年推断）
    /// - 英文 "Jun 20, 2026"
    ///
    /// 解析失败返回 `None`——调用方应保留该结果，不得静默丢弃（数据红线）。
    fn topic_news_age_days(date_str: &str) -> Option<i64> {
        use chrono::{Datelike, NaiveDate};

        let s = date_str.trim();
        if s.is_empty() {
            return None;
        }
        let today = chrono::Local::now().date_naive();

        // 1) 中文相对时间
        if s.contains("今天") || s.contains("刚刚") || s.contains("分钟前") || s.contains("小时前")
        {
            return Some(0);
        }
        if s.contains("昨天") {
            return Some(1);
        }
        if s.contains("前天") {
            return Some(2);
        }
        let lead_num: Option<i64> = s
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse()
            .ok();
        if let Some(n) = lead_num {
            if s.contains("天前") {
                return Some(n);
            }
            if s.contains("周前") || s.contains("星期前") {
                return Some(n * 7);
            }
            if s.contains("个月前") || s.contains("月前") {
                return Some(n * 30);
            }
            if s.contains("年前") {
                return Some(n * 365);
            }
        }

        // 2) 中文绝对日期（先于 ISO 处理，避免对多字节串做字节切片）
        if s.contains('年') || s.contains('月') {
            let digits: Vec<i32> = s
                .split(|c: char| !c.is_ascii_digit())
                .filter(|x| !x.is_empty())
                .filter_map(|x| x.parse().ok())
                .collect();
            if s.contains('年') && digits.len() >= 3 {
                if let Some(d) =
                    NaiveDate::from_ymd_opt(digits[0], digits[1] as u32, digits[2] as u32)
                {
                    return Some((today - d).num_days());
                }
            } else if s.contains('月') && digits.len() >= 2 {
                let (m, day) = (digits[0] as u32, digits[1] as u32);
                if let Some(cand) = NaiveDate::from_ymd_opt(today.year(), m, day) {
                    // 无年份时按今年推断；若落在未来说明是去年的，回退一年
                    let d = if cand > today {
                        NaiveDate::from_ymd_opt(today.year() - 1, m, day).unwrap_or(cand)
                    } else {
                        cand
                    };
                    return Some((today - d).num_days());
                }
            }
        }

        // 3) ISO 前缀 YYYY-MM-DD（仅在前 10 字节为合法字符边界时切片）
        if s.len() >= 10 && s.is_char_boundary(10) {
            if let Ok(d) = NaiveDate::parse_from_str(&s[..10], "%Y-%m-%d") {
                return Some((today - d).num_days());
            }
        }
        // 4) RFC3339 / RFC2822
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
            return Some((today - dt.date_naive()).num_days());
        }
        if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(s) {
            return Some((today - dt.date_naive()).num_days());
        }
        // 5) 英文 "Jun 20, 2026"
        if let Ok(d) = NaiveDate::parse_from_str(s, "%b %d, %Y") {
            return Some((today - d).num_days());
        }

        None
    }

    /// 通用主题搜索（去同质化）：
    /// 1) 单 query 自动扩展为多意图查询；
    /// 2) 多 provider 聚合而非首个成功即返回；
    /// 3) MMR 重排抑制相似标题；
    /// 4) 参考近期已推送标题做新颖性惩罚。
    pub async fn search_topic(&self, query: &str, max_results: usize) -> Vec<SearchResult> {
        if max_results == 0 {
            return Vec::new();
        }

        // BR-036: 仅纳入声明支持主题搜索的 provider，避免结构错配源进入主题池。
        let available: Vec<_> = self
            .providers
            .iter()
            .filter(|p| p.is_available() && p.supports_topic_search())
            .collect();
        if available.is_empty() {
            return Vec::new();
        }

        let cfg = get_monitor_config();
        let timeout_sec = cfg.topic_search_timeout_sec.max(3);
        let intent_cap = usize::from(cfg.topic_search_intent_count.clamp(2, 8));
        let rerank_params = Self::topic_rerank_params();

        let expanded_queries = Self::build_topic_queries(query, max_results, intent_cap);
        let per_provider_max = (max_results / 2).clamp(2, 4);

        let mut aggregated: Vec<SearchResult> = Vec::new();
        for q in &expanded_queries {
            for provider in &available {
                let resp = match tokio::time::timeout(
                    Duration::from_secs(timeout_sec),
                    provider.search(q, per_provider_max),
                )
                .await
                {
                    Ok(r) => r,
                    Err(_) => {
                        debug!("[topic] {} 查询超时: {}", provider.name(), q);
                        continue;
                    }
                };

                if !resp.success || resp.results.is_empty() {
                    debug!(
                        "[topic] {} 无结果: {} ({})",
                        provider.name(),
                        q,
                        resp.error_message.as_deref().unwrap_or("空结果")
                    );
                    continue;
                }

                for mut r in resp.results {
                    // 统一补齐分析字段，便于后续打分。
                    r.analyze_type();
                    r.analyze_sentiment();
                    r.calculate_importance();
                    aggregated.push(r);
                }
            }
        }

        if aggregated.is_empty() {
            return Vec::new();
        }

        // 新鲜度门（AGENTS.md §2.4）：主题/Web 新闻超过 N 天视为过期（窗口可配置）。
        // 能解析出发布日期且超阈值 → 丢弃并告警；解析不出 → 保留（不静默当成功）。
        let max_age_days = get_monitor_config().topic_news_max_age_days.max(1);
        let before = aggregated.len();
        aggregated.retain(|r| {
            match r
                .published_date
                .as_deref()
                .and_then(Self::topic_news_age_days)
            {
                Some(age) if age > max_age_days => {
                    warn!(
                        "[topic] 丢弃过期新闻（{}天前）: {}",
                        age,
                        r.title.chars().take(40).collect::<String>()
                    );
                    false
                }
                _ => true,
            }
        });
        let dropped = before - aggregated.len();
        if dropped > 0 {
            info!(
                "[topic] 新鲜度过滤丢弃 {} 条（>{}天）",
                dropped, max_age_days
            );
        }

        if aggregated.is_empty() {
            return Vec::new();
        }

        // 先做一次粗去重（URL + 标题签名），再做 MMR 多样性重排。
        let mut seen_url: HashSet<String> = HashSet::new();
        let mut seen_title_sig: HashSet<String> = HashSet::new();
        aggregated.retain(|r| {
            let title_sig = Self::normalize_text(&r.title);
            if title_sig.is_empty() {
                return false;
            }
            let url_ok = if r.url.trim().is_empty() {
                true
            } else {
                seen_url.insert(r.url.clone())
            };
            let title_ok = seen_title_sig.insert(title_sig);
            url_ok && title_ok
        });

        if aggregated.is_empty() {
            return Vec::new();
        }

        let history = self.snapshot_recent_topic_signatures();
        let reranked =
            Self::rerank_topic_results(query, aggregated, &history, max_results, rerank_params);
        self.remember_topic_results(&reranked);
        reranked
    }

    fn build_topic_queries(query: &str, max_results: usize, intent_cap: usize) -> Vec<String> {
        let base = query.trim();
        if base.is_empty() {
            return Vec::new();
        }

        let mut queries = vec![base.to_string()];

        // 紧凑锚点：通用宏观 base（含「重大新闻」）会让每条意图查询都背着同一段
        // 泛化前缀（其中「政策 产业」还与意图词自我重复），导致 provider 拿到的是
        // 一组高度同质的查询、结果大量重叠且浪费配额。此处仅对通用 base 压缩为
        // 「今日 A股」锚点；调用方若传入的是具体主题（如「06月27日 机器人 最新
        // 突发 催化」）则保持原文，确保产业链催化检索的针对性不被削弱。
        let anchor: &str = if base.contains("重大新闻") {
            "今日 A股"
        } else {
            base
        };

        // 维度顺序即采样优先级（max_intents 会截断尾部）。「技术突破」此前缺失，
        // 导致科技/新品/研发类催化在源头被欠采样，故置于首位优先采集。
        let intents = [
            "科技 技术突破 新品 研发 专利 量产",
            // 修复 B-002: 科创板/半导体/光刻/CO2/新能源等"行业垂媒"关键词
            // 之前 search query 是泛宏观("今日 A股 重大新闻"), 行业技术新闻永远搜不到
            "科创板 半导体 光刻 晶圆 CO2 激光 芯片制造",
            "新能源 电池 光伏 储能 充电桩 材料突破",
            "政策 监管 会议 文件",
            "产业链 上游 下游 供需 价格",
            "公司 公告 订单 中标 并购 合作",
            "资金 北向 龙虎榜 主力",
            "海外 美联储 美股 大宗商品 汇率",
            "风险 减持 处罚 违约 诉讼",
        ];

        let max_intents = max_results.clamp(3, 6).min(intent_cap);
        for intent in intents.iter().take(max_intents) {
            queries.push(format!("{} {}", anchor, intent));
        }

        queries
    }

    fn rerank_topic_results(
        query: &str,
        candidates: Vec<SearchResult>,
        history: &[String],
        max_results: usize,
        params: TopicRerankParams,
    ) -> Vec<SearchResult> {
        #[derive(Clone)]
        struct Scored {
            item: SearchResult,
            base_score: f32,
            signature: String,
        }

        let query_terms = Self::extract_query_terms(query);
        let mut pool: Vec<Scored> = candidates
            .into_iter()
            .map(|item| {
                let signature = Self::normalize_text(&format!("{} {}", item.title, item.snippet));
                let lexical = Self::query_match_score(&signature, &query_terms);
                let base_score =
                    (item.importance as f32) * 0.45 + item.relevance * 5.0 + lexical * 2.5;
                Scored {
                    item,
                    base_score,
                    signature,
                }
            })
            .collect();

        let mut selected: Vec<Scored> = Vec::new();
        while selected.len() < max_results && !pool.is_empty() {
            let mut best_idx = 0usize;
            let mut best_score = f32::MIN;

            for (idx, cand) in pool.iter().enumerate() {
                let sim_to_selected = selected
                    .iter()
                    .map(|s| Self::text_similarity(&cand.signature, &s.signature))
                    .fold(0.0_f32, f32::max);
                let sim_to_history = history
                    .iter()
                    .map(|h| Self::text_similarity(&cand.signature, h))
                    .fold(0.0_f32, f32::max);

                // MMR: 兼顾相关性与多样性，并额外惩罚近期重复主题。
                let mmr_score = params.relevance_weight * cand.base_score
                    - params.diversity_penalty * sim_to_selected
                    - params.history_penalty * sim_to_history;
                if mmr_score > best_score {
                    best_score = mmr_score;
                    best_idx = idx;
                }
            }

            selected.push(pool.swap_remove(best_idx));
        }

        selected.into_iter().map(|s| s.item).collect()
    }

    fn snapshot_recent_topic_signatures(&self) -> Vec<String> {
        let cfg = get_monitor_config();
        let mut merged: Vec<String> = match self.recent_topic_signatures.lock() {
            Ok(guard) => guard.iter().cloned().collect(),
            Err(_) => Vec::new(),
        };

        let db_hist = crate::database::DatabaseManager::try_get()
            .and_then(|db| {
                db.get_recent_topic_history_signatures(
                    cfg.topic_history_window_hours.max(24),
                    cfg.topic_history_db_limit.max(100),
                )
                .ok()
            })
            .unwrap_or_default();

        if db_hist.is_empty() {
            return merged;
        }

        let mut seen: HashSet<String> = merged.iter().cloned().collect();
        for sig in db_hist {
            if seen.insert(sig.clone()) {
                merged.push(sig);
            }
        }
        merged
    }

    fn remember_topic_results(&self, results: &[SearchResult]) {
        let mut signatures: Vec<String> = results
            .iter()
            .map(|r| Self::normalize_text(&format!("{} {}", r.title, r.snippet)))
            .filter(|s| !s.is_empty())
            .collect();
        if signatures.is_empty() {
            return;
        }

        if let Ok(mut guard) = self.recent_topic_signatures.lock() {
            for sig in signatures.drain(..) {
                guard.push_back(sig);
            }
            let cap = get_monitor_config().topic_history_memory_size.max(50);
            while guard.len() > cap {
                let _ = guard.pop_front();
            }
        }

        let cfg = get_monitor_config();
        let to_store: Vec<String> = results
            .iter()
            .map(|r| Self::normalize_text(&format!("{} {}", r.title, r.snippet)))
            .filter(|s| !s.is_empty())
            .collect();
        if to_store.is_empty() {
            return;
        }

        let _ = crate::database::DatabaseManager::try_get().and_then(|db| {
            db.upsert_topic_history_signatures(&to_store, cfg.topic_history_db_limit.max(100))
                .ok()
        });
    }

    fn topic_rerank_params() -> TopicRerankParams {
        let cfg = get_monitor_config();
        TopicRerankParams {
            relevance_weight: cfg.topic_mmr_relevance_weight.clamp(0.1, 2.0),
            diversity_penalty: cfg.topic_mmr_diversity_penalty.clamp(0.1, 5.0),
            history_penalty: cfg.topic_mmr_history_penalty.clamp(0.0, 5.0),
        }
    }

    fn extract_query_terms(query: &str) -> Vec<String> {
        let mut terms: Vec<String> = query
            .split_whitespace()
            .map(Self::normalize_text)
            .filter(|s| !s.is_empty())
            .collect();

        if terms.len() <= 1 {
            let compact = Self::normalize_text(query);
            if compact.chars().count() >= 2 {
                // 中文查询常无空格，补充 2~4 字片段提升匹配鲁棒性。
                let chars: Vec<char> = compact.chars().collect();
                for size in [2_usize, 3, 4] {
                    for w in chars.windows(size).take(10) {
                        terms.push(w.iter().collect::<String>());
                    }
                }
            }
        }

        terms.truncate(18);
        terms
    }

    fn query_match_score(text: &str, query_terms: &[String]) -> f32 {
        if query_terms.is_empty() || text.is_empty() {
            return 0.0;
        }
        let hit = query_terms
            .iter()
            .filter(|t| text.contains(t.as_str()))
            .count();
        hit as f32 / query_terms.len() as f32
    }

    fn normalize_text(text: &str) -> String {
        text.chars()
            .filter(|c| c.is_alphanumeric() || ('\u{4e00}'..='\u{9fff}').contains(c))
            .flat_map(|c| c.to_lowercase())
            .collect::<String>()
    }

    fn text_similarity(a: &str, b: &str) -> f32 {
        if a.is_empty() || b.is_empty() {
            return 0.0;
        }
        if a == b {
            return 1.0;
        }

        let a_grams = Self::char_ngrams(a, 2);
        let b_grams = Self::char_ngrams(b, 2);
        if a_grams.is_empty() || b_grams.is_empty() {
            return 0.0;
        }

        let inter = a_grams.intersection(&b_grams).count() as f32;
        let union = a_grams.union(&b_grams).count() as f32;
        if union == 0.0 {
            0.0
        } else {
            inter / union
        }
    }

    fn char_ngrams(text: &str, n: usize) -> HashSet<String> {
        let chars: Vec<char> = text.chars().collect();
        if chars.is_empty() {
            return HashSet::new();
        }
        if chars.len() < n {
            return [text.to_string()].into_iter().collect();
        }
        chars
            .windows(n)
            .map(|w| w.iter().collect::<String>())
            .collect()
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
        let invest_name = if short_name != stock_name {
            &short_name
        } else {
            stock_name
        };
        queries.push(format!("{} 持股 投资 收购 参股", invest_name));

        // 维度3: 行业/合作/订单（简称搜索）
        queries.push(format!("{} 合作 中标 订单 签约", invest_name));

        // 维度4: 负面风险排查（简称 + 代码）
        queries.push(format!("{} {} 减持 处罚 风险", stock_name, stock_code));

        // 维度5: 业绩预期（简称 + 代码）
        queries.push(format!(
            "{} {} 年报预告 业绩预告 业绩快报",
            stock_name, stock_code
        ));

        let mut all_results: Vec<SearchResult> = Vec::new();
        let mut success_provider = String::new();
        let mut total_search_time = 0.0;

        for (dim_idx, query) in queries.iter().enumerate() {
            // 每个维度取少量结果，合并后再截断
            let per_query_max = if dim_idx == 0 {
                max_results
            } else {
                3_usize.min(max_results)
            };

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
                    info!(
                        "[维度{}] 使用 {} 搜索 '{}' 获得 {} 条结果",
                        dim_idx + 1,
                        response.provider,
                        query,
                        response.results.len()
                    );
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
            let high_value_keywords = [
                "持股", "投资", "收购", "参股", "并购", "入股", "中标", "签约", "订单",
            ];
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
            score_b
                .partial_cmp(&score_a)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // 截断到max_results + 2（多保留一点给AI更多上下文）
        all_results.truncate(max_results + 2);

        info!(
            "多维度搜索完成: {} 条结果（去重后），来源: {}",
            all_results.len(),
            success_provider
        );

        SearchResponse {
            query: format!("{} {} 多维度搜索", stock_name, stock_code),
            results: all_results,
            success: true,
            provider: success_provider,
            error_message: None,
            search_time: total_search_time,
            failure: None,
        }
    }

    /// 从股票全称中提取简称/核心词
    /// 例如: "金风科技" -> "金风", "越秀资本" -> "越秀", "贵州茅台" -> "茅台"
    fn extract_short_name(stock_name: &str) -> String {
        // 常见后缀词（按长度从长到短排列，优先匹配长的）
        let suffixes = [
            "电子科技",
            "高新技术",
            "信息技术",
            "新材料",
            "新能源",
            "生物科技",
            "科技",
            "集团",
            "股份",
            "控股",
            "实业",
            "产业",
            "资本",
            "投资",
            "金融",
            "银行",
            "证券",
            "保险",
            "医药",
            "制药",
            "生物",
            "电气",
            "电子",
            "电力",
            "能源",
            "环保",
            "汽车",
            "机械",
            "材料",
            "化工",
            "建设",
            "通信",
            "传媒",
            "文化",
            "教育",
            "旅游",
            "食品",
            "乳业",
            "酿酒",
            "地产",
            "置业",
            "物流",
            "航空",
            "航天",
        ];

        // 常见地名前缀
        let prefixes = [
            "贵州", "云南", "四川", "山东", "江苏", "浙江", "广东", "福建", "河南", "河北", "湖南",
            "湖北", "安徽", "江西", "陕西", "山西", "辽宁", "吉林", "黑龙", "甘肃", "青海", "海南",
            "广西", "内蒙", "新疆", "西藏", "宁夏", "上海", "北京", "天津", "重庆", "深圳",
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

        info!(
            "搜索股票事件: {}({}) - {:?}",
            stock_name, stock_code, events
        );

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

        let available_providers: Vec<_> =
            self.providers.iter().filter(|p| p.is_available()).collect();

        if available_providers.is_empty() {
            return results;
        }

        for (provider_index, (dim_name, query, desc)) in
            search_dimensions.into_iter().take(max_searches).enumerate()
        {
            let provider = available_providers[provider_index % available_providers.len()];

            info!("[情报搜索] {}: 使用 {}", desc, provider.name());

            let response = provider.search(&query, 3).await;

            if response.success {
                info!(
                    "[情报搜索] {}: 获取 {} 条结果",
                    desc,
                    response.results.len()
                );
            } else {
                warn!(
                    "[情报搜索] {}: 搜索失败 - {}",
                    desc,
                    response.error_message.as_deref().unwrap_or("未知错误")
                );
            }

            results.insert(dim_name.to_string(), response);
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
        let mut lines = vec![format!(
            "【{} 通用网页研究发现】\nResearchOnly：未经 Magic 金融数据 Gateway 证实，不得作为金融事实或交易/选股依据。",
            stock_name
        )];

        // 最新消息
        if let Some(resp) = intel_results.get("latest_news") {
            lines.push(format!("\n📰 最新消息 (来源: {}):", resp.provider));
            if resp.success && !resp.results.is_empty() {
                for (i, r) in resp
                    .results
                    .iter()
                    .filter(|result| result.evidence.is_research_only())
                    .take(3)
                    .enumerate()
                {
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
                for (i, r) in resp
                    .results
                    .iter()
                    .filter(|result| result.evidence.is_research_only())
                    .take(3)
                    .enumerate()
                {
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
                for (i, r) in resp
                    .results
                    .iter()
                    .filter(|result| result.evidence.is_research_only())
                    .take(3)
                    .enumerate()
                {
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

        // ── 第一步：统一 Gateway 的独立真实批次 ──
        let news_gateway = GlobalNewsGateway::new();
        let economic_gateway = EconomicCalendarGateway::new();
        let (eastmoney, cls, jin10, thepaper, releases) = tokio::join!(
            news_gateway.global_news(GlobalNewsProvider::Eastmoney, 20),
            news_gateway.global_news(GlobalNewsProvider::Cailianpress, 20),
            news_gateway.global_news(GlobalNewsProvider::Jin10, 20),
            news_gateway.global_news(GlobalNewsProvider::ThePaper, 20),
            economic_gateway.latest_releases(20, None),
        );
        sections.extend(render_gateway_sections(
            [
                (GlobalNewsProvider::Eastmoney, eastmoney),
                (GlobalNewsProvider::Cailianpress, cls),
                (GlobalNewsProvider::Jin10, jin10),
                (GlobalNewsProvider::ThePaper, thepaper),
            ],
            releases,
        ));

        tokio::time::sleep(Duration::from_millis(200)).await;

        // ── 第二步：搜索引擎多维度查询 ──
        // (维度key, 查询关键词, 展示标题)
        let search_dims: Vec<(&str, String, &str)> = vec![
            (
                "a_market",
                format!("{}A股 大盘 股市 最新动态", today),
                "### 🇨🇳 A股市场动态",
            ),
            (
                "global",
                format!("{}国际财经 地缘政治 最新消息", today),
                "### 🌍 国际财经 / 地缘政治",
            ),
            (
                "us_market",
                format!("{}美股 美联储 大宗商品 今日", today),
                "### 🇺🇸 美股 / 大宗商品",
            ),
            (
                "cn_policy",
                format!("{}中国 央行 财政 产业政策 重要新闻", today),
                "### 📋 宏观政策",
            ),
            (
                "institution",
                format!(
                    "{}高盛 摩根 大摩 美银 JPMorgan 中国A股 市场观点 研报",
                    today
                ),
                "### 🏦 投行观点（高盛/摩根/美银）",
            ),
            (
                "fin_media",
                format!("{}证券时报 第一财经 21世纪经济报道 重要财经", today),
                "### 📰 财经媒体要闻",
            ),
        ];

        for (dim, query, header) in &search_dims {
            let mut found = false;
            for provider in &self.providers {
                if !provider.supports_general_web_search() || !provider.is_available() {
                    continue;
                }
                let resp = provider.search(query, max_results.min(3)).await;
                let lines: Vec<String> = if resp.success {
                    resp.results
                        .iter()
                        .filter(|result| result.evidence.is_research_only())
                        .take(3)
                        .map(|r| {
                            let date_tag = r.published_date.as_deref().unwrap_or("");
                            let snippet_short: String = r.snippet.chars().take(150).collect();
                            format!("- **{}** {}  \n  {}", r.title, date_tag, snippet_short)
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                if !lines.is_empty() {
                    sections.push(format!(
                        "### 🔎 通用网页研究发现（ResearchOnly；不得作为金融事实）\n{}\n{}",
                        header,
                        lines.join("\n")
                    ));
                    info!(
                        "[宏观新闻][{}] {} 获取 {} 条",
                        dim,
                        resp.provider,
                        resp.results.len()
                    );
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

        format!(
            "## 📡 今日宏观 / 市场背景（{}）\n\n{}",
            today,
            sections.join("\n\n")
        )
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

            let response = self
                .search_stock_news(code, name, max_results_per_stock)
                .await;
            results.insert(code.to_string(), response);
        }

        results
    }
}

// ============================================================================
// 工具函数
// ============================================================================

// extract_domain 已迁移至 super::types::extract_domain

/// 修复 v9.2 BR-003: 宏观新闻关键词 (美联储/美股/汇率/大宗 等)
/// 修复 I-4 (2026-06-29 codex review): 补全缺失关键词 — 纳指/A50/恒指/日股/欧股
/// (原表缺这些, 实际快讯常出现"A50 期指跌"/"恒指收跌"等仍会进 chain_mapper 假信号).
/// 注: codex 建议加 "美元" 单字, 但实测会误伤 "公司美元收入占比" 等公司层面信息,
/// 故**不**加 "美元", 仅保留 "美元指数" (避免假阳性, 详见 flash_filter.rs
/// test_filter_macro_titles_dollar_keyword 注释).
pub const MACRO_KEYWORDS: &[&str] = &[
    "美联储",
    "鲍威尔",
    "FOMC",
    "美股",
    "纳斯达克",
    "纳指",
    "标普",
    "道琼斯",
    "汇率",
    "人民币兑",
    "美元指数",
    "大宗商品",
    "原油",
    "黄金",
    "铜价",
    "欧央行",
    "日银",
    "英国央行",
    "日股",
    "欧股",
    "A50",
    "富时中国",
    "恒指",
    "恒生指数",
];

/// 修复 v9.2 M3 + BR-003: 纯函数过滤宏观新闻, 返回 (filtered_titles, macro_count).
/// 抽出来便于 e2e 测试, 避免依赖真实网络.
pub fn filter_macro_titles(titles: Vec<String>) -> (Vec<String>, usize) {
    let mut macro_count = 0usize;
    let filtered: Vec<String> = titles
        .into_iter()
        .filter(|t| {
            if is_macro_title(t) {
                macro_count += 1;
                log::debug!(
                    "[flash] 宏观新闻 (BR-003): {}",
                    t.chars().take(40).collect::<String>()
                );
                false
            } else {
                true
            }
        })
        .collect();
    (filtered, macro_count)
}

fn is_macro_title(title: &str) -> bool {
    MACRO_KEYWORDS.iter().any(|keyword| title.contains(keyword))
}

// ============================================================================
// 单例服务
// ============================================================================

use once_cell::sync::OnceCell;

static SEARCH_SERVICE: OnceCell<SearchService> = OnceCell::new();

/// 获取搜索服务单例
pub fn get_search_service() -> &'static SearchService {
    SEARCH_SERVICE.get_or_init(SearchService::from_environment)
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct FixtureProvider {
        available: bool,
        topic: bool,
        calls: AtomicUsize,
    }

    impl FixtureProvider {
        fn available() -> Self {
            Self {
                available: true,
                topic: true,
                calls: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl SearchProvider for FixtureProvider {
        fn name(&self) -> &str {
            "TEST_CODE_fixture_search"
        }

        fn is_available(&self) -> bool {
            self.available
        }

        fn supports_topic_search(&self) -> bool {
            self.topic
        }

        async fn search(&self, query: &str, max_results: usize) -> SearchResponse {
            let call = self.calls.fetch_add(1, Ordering::Relaxed);
            let mut result = SearchResult::new(
                format!("测试科技重大订单 {query}"),
                "TEST_CODE_公司获得中标订单，行业技术突破".to_string(),
                format!("https://example.invalid/{call}"),
                self.name().to_string(),
            )
            .with_date(chrono::Local::now().format("%Y-%m-%d").to_string());
            result.importance = 8;
            result.relevance = 0.8;
            SearchResponse {
                query: query.to_string(),
                results: vec![result; max_results.min(1)],
                provider: self.name().to_string(),
                success: true,
                error_message: None,
                search_time: 0.01,
                failure: None,
            }
        }
    }

    fn fixture_service() -> SearchService {
        let mut service = SearchService::new(None, None, None);
        service.providers = vec![Box::new(FixtureProvider::available())];
        service
    }

    fn flash_gateway_fixture(
        published_at_raw: &str,
        batch_id: &str,
    ) -> (
        GlobalNewsProvider,
        GlobalNewsRecord,
        crate::data_gateway::BatchEvidence,
    ) {
        use crate::magic_compat::{ProviderId, SourceEvidence};
        use chrono::{DateTime, Utc};

        let provider = GlobalNewsProvider::Jin10;
        let published_at = DateTime::parse_from_rfc3339(published_at_raw)
            .expect("TEST_CODE publication")
            .with_timezone(&Utc);
        let observed_at = DateTime::parse_from_rfc3339("2026-07-27T09:30:01+08:00")
            .expect("TEST_CODE observation")
            .with_timezone(&Utc);
        let observed_raw = format!(
            "{}.{:09}",
            observed_at.timestamp(),
            observed_at.timestamp_subsec_nanos()
        );
        let batch_evidence = crate::data_gateway::BatchEvidence {
            provider: ProviderId::Jin10,
            source: provider.source().to_string(),
            source_at: Some(published_at_raw.to_string()),
            observed_at: observed_raw.clone(),
            batch_id: batch_id.to_string(),
        };
        let record = GlobalNewsRecord {
            item_id: "TEST_CODE_FLASH_ITEM".to_string(),
            title: "TEST_CODE 芯片设备订单增长".to_string(),
            summary: None,
            content: None,
            publisher: "TEST_CODE_PROVIDER".to_string(),
            canonical_url: "https://example.com/TEST_CODE_FLASH_ITEM".to_string(),
            published_at,
            observed_at,
            instruments: Vec::new(),
            topics: Vec::new(),
            language: "zh-CN".to_string(),
            evidence: SourceEvidence::new(ProviderId::Jin10, observed_raw, batch_id)
                .and_then(|evidence| evidence.with_source_at(published_at_raw))
                .expect("TEST_CODE record evidence"),
        };
        (provider, record, batch_evidence)
    }

    #[test]
    fn admitted_flash_fact_keeps_exact_batch_evidence() {
        use chrono::{DateTime, Utc};
        let (provider, record, batch_evidence) =
            flash_gateway_fixture("2026-07-27T09:30:00+08:00", "TEST_CODE_FLASH_BATCH");

        let projected = project_gateway_flash_outcome(
            provider,
            Ok(GatewayBatch::Available {
                records: vec![record],
                evidence: batch_evidence.clone(),
            }),
            DateTime::parse_from_rfc3339("2026-07-27T09:31:00+08:00")
                .expect("TEST_CODE now")
                .with_timezone(&Utc),
        )
        .expect("TEST_CODE admitted flash facts");

        assert_eq!(projected.facts.len(), 1);
        assert_eq!(projected.facts[0].batch_evidence, batch_evidence);
        assert_eq!(projected.facts[0].record.item_id, "TEST_CODE_FLASH_ITEM");
    }

    #[test]
    fn stale_flash_fact_is_explicitly_excluded() {
        use chrono::{DateTime, Utc};

        let (provider, record, batch_evidence) =
            flash_gateway_fixture("2026-07-26T23:59:00+08:00", "TEST_CODE_STALE_BATCH");
        let projected = project_gateway_flash_outcome(
            provider,
            Ok(GatewayBatch::Available {
                records: vec![record],
                evidence: batch_evidence,
            }),
            DateTime::parse_from_rfc3339("2026-07-27T09:31:00+08:00")
                .expect("TEST_CODE now")
                .with_timezone(&Utc),
        )
        .expect("TEST_CODE stale exclusion");

        assert!(projected.facts.is_empty());
        assert!(matches!(
            projected.status,
            FlashSourceStatus::Available {
                admitted_records: 0,
                stale_records: 1,
                macro_records: 0,
                ..
            }
        ));
    }

    #[test]
    fn flash_fact_with_mismatched_record_batch_is_rejected() {
        use crate::magic_compat::{ProviderId, SourceEvidence};
        use chrono::{DateTime, Utc};

        let (provider, mut record, batch_evidence) =
            flash_gateway_fixture("2026-07-27T09:30:00+08:00", "TEST_CODE_FLASH_BATCH");
        record.evidence = SourceEvidence::new(
            ProviderId::Jin10,
            batch_evidence.observed_at.clone(),
            "TEST_CODE_OTHER_BATCH",
        )
        .and_then(|evidence| evidence.with_source_at("2026-07-27T09:30:00+08:00"))
        .expect("TEST_CODE mismatched record evidence");

        let error = project_gateway_flash_outcome(
            provider,
            Ok(GatewayBatch::Available {
                records: vec![record],
                evidence: batch_evidence,
            }),
            DateTime::parse_from_rfc3339("2026-07-27T09:31:00+08:00")
                .expect("TEST_CODE now")
                .with_timezone(&Utc),
        )
        .expect_err("TEST_CODE mismatched record evidence must be rejected");

        assert!(error.contains("evidence differs from batch"), "{error}");
    }

    #[tokio::test]
    async fn test_search_service() {
        env_logger::init();

        let service = get_search_service();

        if service.is_available() {
            println!("=== 测试股票新闻搜索 ===");
            let response = service
                .search_stock_news("TEST_CODE_300389", "艾比森", 5)
                .await;
            println!(
                "搜索状态: {}",
                if response.success { "成功" } else { "失败" }
            );
            println!("搜索引擎: {}", response.provider);
            println!("结果数量: {}", response.results.len());
            println!("耗时: {:.2}s", response.search_time);
            println!("\n{}", response.to_context(5));
        } else {
            println!("未配置搜索引擎 API Key，跳过测试");
        }
    }

    #[test]
    fn test_topic_news_age_days_parsing() {
        use chrono::{Datelike, Duration};
        let today = chrono::Local::now().date_naive();

        // 中文相对时间
        assert_eq!(SearchService::topic_news_age_days("3小时前"), Some(0));
        assert_eq!(SearchService::topic_news_age_days("昨天 10:30"), Some(1));
        assert_eq!(SearchService::topic_news_age_days("前天"), Some(2));
        assert_eq!(SearchService::topic_news_age_days("5天前"), Some(5));
        assert_eq!(SearchService::topic_news_age_days("2周前"), Some(14));

        // ISO 与 RFC3339
        let iso = (today - Duration::days(3)).format("%Y-%m-%d").to_string();
        assert_eq!(SearchService::topic_news_age_days(&iso), Some(3));
        let rfc = format!("{}T08:00:00+08:00", iso);
        assert_eq!(SearchService::topic_news_age_days(&rfc), Some(3));

        // 中文绝对日期（带年份）
        let d = today - Duration::days(10);
        let cn = format!("{}年{}月{}日", d.year(), d.month(), d.day());
        assert_eq!(SearchService::topic_news_age_days(&cn), Some(10));

        // 无法解析 → None（保留，不静默丢弃）
        assert_eq!(SearchService::topic_news_age_days(""), None);
        assert_eq!(SearchService::topic_news_age_days("近期"), None);
    }

    #[test]
    fn registry_contains_only_general_web_search_adapters() {
        let service = SearchService::new(
            Some(vec!["TEST_CODE_bocha".to_owned()]),
            Some(vec!["TEST_CODE_tavily".to_owned()]),
            Some(vec!["TEST_CODE_serpapi".to_owned()]),
        );
        assert_eq!(service.providers.len(), 3);
        assert!(service
            .providers
            .iter()
            .all(|provider| provider.supports_general_web_search()));
    }

    #[test]
    fn topic_helpers_cover_query_similarity_rerank_and_health_states() {
        let service = fixture_service();
        assert!(service.is_available());
        for outcome in [
            SourceFetchOutcome::Success,
            SourceFetchOutcome::Error,
            SourceFetchOutcome::Empty,
        ] {
            service.record_source_health("TEST_CODE_source", outcome, 2);
        }
        for _ in 0..20 {
            service.maybe_log_source_health_summary("TEST_CODE_reason");
        }

        assert!(SearchService::build_topic_queries("  ", 5, 5).is_empty());
        let generic = SearchService::build_topic_queries("今日重大新闻", 8, 8);
        assert_eq!(generic.len(), 7);
        assert!(generic[1].starts_with("今日 A股"));
        let specific = SearchService::build_topic_queries("机器人催化", 3, 2);
        assert_eq!(specific.len(), 3);
        assert!(specific[1].starts_with("机器人催化"));

        assert_eq!(SearchService::normalize_text("A 股-测试!"), "a股测试");
        assert_eq!(SearchService::text_similarity("", "测试"), 0.0);
        assert_eq!(SearchService::text_similarity("同一", "同一"), 1.0);
        assert!(SearchService::text_similarity("半导体突破", "半导体量产") > 0.0);
        assert_eq!(SearchService::char_ngrams("", 2).len(), 0);
        assert_eq!(SearchService::char_ngrams("单", 2).len(), 1);
        assert!(!SearchService::extract_query_terms("机器人").is_empty());
        assert_eq!(
            SearchService::query_match_score("", &["测试".to_string()]),
            0.0
        );

        let candidates = vec![
            SearchResult::new(
                "机器人技术突破".to_string(),
                "量产".to_string(),
                "https://example.invalid/a".to_string(),
                "TEST_CODE_source".to_string(),
            ),
            SearchResult::new(
                "机器人订单".to_string(),
                "中标".to_string(),
                "https://example.invalid/b".to_string(),
                "TEST_CODE_source".to_string(),
            ),
        ];
        let ranked = SearchService::rerank_topic_results(
            "机器人 技术",
            candidates,
            &["机器人技术突破量产".to_string()],
            2,
            TopicRerankParams {
                relevance_weight: 1.0,
                diversity_penalty: 1.0,
                history_penalty: 1.0,
            },
        );
        assert_eq!(ranked.len(), 2);

        assert_eq!(SearchService::extract_short_name("贵州茅台"), "茅台");
        assert_eq!(SearchService::extract_short_name("金风科技"), "金风");
        assert_eq!(SearchService::extract_short_name("A银行"), "A银行");

        for text in ["今天", "刚刚", "3分钟前", "2个月前", "1年前"] {
            assert!(SearchService::topic_news_age_days(text).is_some(), "{text}");
        }
        let english = (chrono::Local::now().date_naive() - chrono::Duration::days(2))
            .format("%b %d, %Y")
            .to_string();
        assert_eq!(SearchService::topic_news_age_days(&english), Some(2));
    }

    #[tokio::test]
    async fn fixture_provider_executes_topic_stock_event_intel_and_batch_flows() {
        let service = fixture_service();
        assert!(service.search_topic("TEST_CODE_机器人", 0).await.is_empty());
        let topic = service.search_topic("TEST_CODE_机器人", 4).await;
        assert_eq!(topic.len(), 4);
        assert!(topic
            .iter()
            .all(|item| !item.keywords.is_empty() || item.importance >= 5));

        let stock = service
            .search_stock_news("TEST_CODE_000001", "测试科技", 2)
            .await;
        assert!(stock.success);
        assert!(!stock.results.is_empty());
        assert!(stock.provider.contains("TEST_CODE_fixture_search"));
        assert!(stock.results.iter().all(|item| item.relevance >= 0.8));

        let events = service
            .search_stock_events("TEST_CODE_000001", "测试科技", None)
            .await;
        assert!(events.success);
        let custom_events = service
            .search_stock_events(
                "TEST_CODE_000001",
                "测试科技",
                Some(vec!["TEST_CODE_重大合同"]),
            )
            .await;
        assert!(custom_events.query.contains("TEST_CODE_重大合同"));

        let intel = service
            .search_comprehensive_intel("TEST_CODE_000001", "测试科技", 3)
            .await;
        assert_eq!(intel.len(), 3);
        let report = SearchService::format_intel_report(&intel, "测试科技");
        assert!(report.contains("ResearchOnly"));
        assert!(report.contains("不得作为金融事实"));
        assert!(report.contains("最新消息"));
        assert!(report.contains("风险排查"));
        assert!(report.contains("业绩预期"));

        let batch = service
            .batch_search(
                vec![
                    ("TEST_CODE_000001", "测试科技"),
                    ("TEST_CODE_000002", "测试材料"),
                ],
                1,
                Duration::ZERO,
            )
            .await;
        assert_eq!(batch.len(), 2);
    }

    #[tokio::test]
    async fn unavailable_provider_and_empty_reports_remain_explicit() {
        let mut service = SearchService::new(None, None, None);
        service.providers = vec![Box::new(FixtureProvider {
            available: false,
            topic: false,
            calls: AtomicUsize::new(0),
        })];
        assert!(!service.is_available());
        assert!(service.search_topic("TEST_CODE_主题", 3).await.is_empty());
        assert!(service
            .search_comprehensive_intel("TEST_CODE_000001", "测试科技", 3)
            .await
            .is_empty());
        assert!(
            !service
                .search_stock_news("TEST_CODE_000001", "测试科技", 2)
                .await
                .success
        );
        assert!(
            !service
                .search_stock_events("TEST_CODE_000001", "测试科技", Some(Vec::new()))
                .await
                .success
        );

        let report = SearchService::format_intel_report(
            &HashMap::from([
                (
                    "latest_news".to_string(),
                    SearchResponse::error("q".to_string(), "none".to_string(), "e".to_string()),
                ),
                (
                    "risk_check".to_string(),
                    SearchResponse::error("q".to_string(), "none".to_string(), "e".to_string()),
                ),
                (
                    "earnings".to_string(),
                    SearchResponse::error("q".to_string(), "none".to_string(), "e".to_string()),
                ),
            ]),
            "测试科技",
        );
        assert!(report.contains("未找到相关消息"));
        assert!(report.contains("未发现明显风险信号"));
        assert!(report.contains("未找到业绩相关信息"));
    }
}
