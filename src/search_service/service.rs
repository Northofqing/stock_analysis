//! 搜索服务聚合器（原 search_service.rs 尾部）

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;
use std::time::Duration;

use log::{info, warn};

use crate::config::get_monitor_config;

use super::providers::{
    BochaSearchProvider, ClsProvider, EastmoneyNewsProvider, Jin10CalendarEvent, Jin10Provider,
    SerpAPISearchProvider, TavilySearchProvider, WallStreetCnProvider,
};
use super::types::{SearchProvider, SearchResponse, SearchResult};

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
    /// 财联社直连（免费，A股电报）
    cls: ClsProvider,
    /// 金十数据直连（免费，快讯 + 财经日历）
    jin10: Jin10Provider,
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
    timeout: u64,
    error: u64,
    empty: u64,
    items: u64,
}

#[derive(Clone, Copy)]
enum SourceFetchOutcome {
    Success,
    Timeout,
    Error,
    Empty,
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
        // 修复 P1.4: 免费源先于付费源, 避免 429/403/432 时无谓重试
        // 原因: SerpAPI/Bocha/Tavily 都是有额度的付费/限免 API, 失败率高
        //        而 东方财富/华尔街见闻/财联社 是免费直连, 优先用它们能保证稳定

        // 1. 东方财富（免费，A股专业，无需API Key）
        if enable_eastmoney {
            info!("已启用 东方财富 新闻搜索（免费，无限制）");
            providers.push(Box::new(EastmoneyNewsProvider::new()));
        }

        // 2. 华尔街见闻（免费直连，补充全球财经资讯）
        providers.push(Box::new(WallStreetCnProvider::new()));

        // 3. 财联社（免费直连，补充A股电报）
        providers.push(Box::new(ClsProvider::new()));

        // 4. 金十数据（免费直连，补充快讯）
        // 见 providers/jin10.rs - 默认就是免费直连, 无 API Key

        // 5. SerpAPI（付费，Google搜索结果，作为质量补充）
        if let Some(keys) = serpapi_keys {
            if !keys.is_empty() {
                info!("已配置 SerpAPI 搜索，共 {} 个 API Key", keys.len());
                providers.push(Box::new(SerpAPISearchProvider::new(keys)));
            }
        }

        // 6. Bocha（付费，中文搜索优化，AI摘要）
        if let Some(keys) = bocha_keys {
            if !keys.is_empty() {
                info!("已配置 Bocha 搜索，共 {} 个 API Key", keys.len());
                providers.push(Box::new(BochaSearchProvider::new(keys)));
            }
        }

        // 7. Tavily（限免，作为最后补充）
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
        info!("已启用 金十数据 直连（免费，快讯 + 财经日历）");
        let cfg = get_monitor_config();
        Self {
            providers,
            wscn: WallStreetCnProvider::new(),
            cls: ClsProvider::new(),
            jin10: Jin10Provider::new(),
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
                SourceFetchOutcome::Timeout => stat.timeout += 1,
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
                    "{}: 成功 {}/{} ({:.1}%), 超时 {}, 错误 {}, 空结果 {}, 累计条数 {}",
                    source,
                    stat.success,
                    stat.attempts,
                    success_rate,
                    stat.timeout,
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

    /// 获取金十财经日历（未来 `days_ahead` 天，重要性 >= `min_star`）
    pub async fn fetch_financial_calendar(&self, days_ahead: u32, min_star: u8) -> Vec<Jin10CalendarEvent> {
        match self.jin10.fetch_calendar(days_ahead, min_star).await {
            Ok(v) => v,
            Err(e) => {
                warn!("[金十日历] 抓取失败: {}", e);
                Vec::new()
            }
        }
    }

    /// 获取原始快讯标题列表（供 NewsMonitor 路径A 使用）
    pub async fn fetch_flash_titles(&self, limit: usize) -> Vec<String> {
        let source_timeout = Duration::from_secs(get_monitor_config().topic_search_timeout_sec.max(3));
        let (jin10_res, wscn_res, cls_res) = tokio::join!(
            tokio::time::timeout(source_timeout, self.jin10.fetch_flash_news(limit, true)),
            tokio::time::timeout(source_timeout, self.wscn.fetch_live_news(limit)),
            tokio::time::timeout(source_timeout, self.cls.fetch_live_news(limit)),
        );

        let mut titles = Vec::new();

        match jin10_res {
            Ok(Ok(lst)) => {
                info!("[flash][jin10] 成功 {} 条", lst.len());
                if lst.is_empty() {
                    self.record_source_health("jin10", SourceFetchOutcome::Empty, 0);
                } else {
                    self.record_source_health("jin10", SourceFetchOutcome::Success, lst.len());
                }
                for r in lst {
                    titles.push(r.title);
                }
            }
            Ok(Err(e)) => {
                self.record_source_health("jin10", SourceFetchOutcome::Error, 0);
                warn!("[flash][jin10] 失败: {}", e)
            }
            Err(_) => {
                self.record_source_health("jin10", SourceFetchOutcome::Timeout, 0);
                warn!("[flash][jin10] 超时（>{}s）", source_timeout.as_secs())
            }
        }

        match wscn_res {
            Ok(Ok(lst)) => {
                info!("[flash][wscn] 成功 {} 条", lst.len());
                if lst.is_empty() {
                    self.record_source_health("wscn", SourceFetchOutcome::Empty, 0);
                } else {
                    self.record_source_health("wscn", SourceFetchOutcome::Success, lst.len());
                }
                for r in lst {
                    titles.push(r.title);
                }
            }
            Ok(Err(e)) => {
                self.record_source_health("wscn", SourceFetchOutcome::Error, 0);
                warn!("[flash][wscn] 失败: {}", e)
            }
            Err(_) => {
                self.record_source_health("wscn", SourceFetchOutcome::Timeout, 0);
                warn!("[flash][wscn] 超时（>{}s）", source_timeout.as_secs())
            }
        }

        match cls_res {
            Ok(Ok(lst)) => {
                info!("[flash][cls] 成功 {} 条", lst.len());
                if lst.is_empty() {
                    self.record_source_health("cls", SourceFetchOutcome::Empty, 0);
                } else {
                    self.record_source_health("cls", SourceFetchOutcome::Success, lst.len());
                }
                for r in lst {
                    titles.push(r.title);
                }
            }
            Ok(Err(e)) => {
                self.record_source_health("cls", SourceFetchOutcome::Error, 0);
                warn!("[flash][cls] 失败: {}", e)
            }
            Err(_) => {
                self.record_source_health("cls", SourceFetchOutcome::Timeout, 0);
                warn!("[flash][cls] 超时（>{}s）", source_timeout.as_secs())
            }
        }

        let mut seen = HashSet::new();
        let mut deduped = Vec::new();
        for title in titles {
            let sig = Self::normalize_text(&title);
            if sig.is_empty() || !seen.insert(sig) {
                continue;
            }
            deduped.push(title);
            if deduped.len() >= limit {
                break;
            }
        }

        self.maybe_log_source_health_summary("fetch_flash_titles");

        deduped
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
        if s.contains("今天") || s.contains("刚刚") || s.contains("分钟前") || s.contains("小时前") {
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

        let available: Vec<_> = self
            .providers
            .iter()
            .filter(|p| p.is_available())
            .collect();
        if available.is_empty() {
            return Vec::new();
        }

        let cfg = get_monitor_config();
        let timeout_sec = cfg.topic_search_timeout_sec.max(3);
        let intent_cap = usize::from(cfg.topic_search_intent_count.clamp(2, 8));
        let rerank_params = Self::topic_rerank_params();

        let expanded_queries = Self::build_topic_queries(query, max_results, intent_cap);
        let per_provider_max = (max_results / 2).max(2).min(4);

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
                        warn!("[topic] {} 查询超时: {}", provider.name(), q);
                        continue;
                    }
                };

                if !resp.success || resp.results.is_empty() {
                    warn!(
                        "[topic] {} 查询失败: {} ({})",
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
        let reranked = Self::rerank_topic_results(
            query,
            aggregated,
            &history,
            max_results,
            rerank_params,
        );
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
                let base_score = (item.importance as f32) * 0.45 + item.relevance * 5.0 + lexical * 2.5;
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

        let db_hist = std::panic::catch_unwind(|| crate::database::DatabaseManager::get())
            .ok()
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

        let _ = std::panic::catch_unwind(|| crate::database::DatabaseManager::get())
            .ok()
            .and_then(|db| {
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
        let hit = query_terms.iter().filter(|t| text.contains(t.as_str())).count();
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
        let invest_name = if short_name != stock_name { &short_name } else { stock_name };
        queries.push(format!("{} 持股 投资 收购 参股", invest_name));

        // 维度3: 行业/合作/订单（简称搜索）
        queries.push(format!("{} 合作 中标 订单 签约", invest_name));

        // 维度4: 负面风险排查（简称 + 代码）
        queries.push(format!("{} {} 减持 处罚 风险", stock_name, stock_code));

        // 维度5: 业绩预期（简称 + 代码）
        queries.push(format!("{} {} 年报预告 业绩预告 业绩快报", stock_name, stock_code));

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

        // ── 第一步：免费直连源（华尔街见闻 + 金十数据）并发拉取 ──
        let (live_res, article_res, cls_res, jin10_flash_res, jin10_imp_res, calendar_res) = tokio::join!(
            self.wscn.fetch_live_news(30),
            self.wscn.fetch_articles(10),
            self.cls.fetch_live_news(30),
            self.jin10.fetch_flash_news(20, false),
            self.jin10.fetch_flash_news(10, true),
            self.jin10.fetch_calendar(1, 2), // 今天+明天，重要性≥2
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

        let mut cls_items: Vec<String> = Vec::new();
        if let Ok(lst) = cls_res {
            for r in lst.iter().take(8) {
                let t = r.published_date.as_deref().unwrap_or("");
                let snippet: String = r.snippet.chars().take(120).collect();
                cls_items.push(format!("- **{}** {}  \n  {}", r.title, t, snippet));
            }
        }
        if !cls_items.is_empty() {
            sections.push(format!("### 🧭 财联社电报（今日实时）\n{}", cls_items.join("\n")));
            info!("[宏观新闻][cls] 财联社获取 {} 条", cls_items.len());
        } else {
            warn!("[宏观新闻][cls] 财联社返回为空或超时");
        }

        // ── 金十快讯（标星重要优先，普通快讯补充）──
        let mut jin10_imp_items: Vec<String> = Vec::new();
        if let Ok(lst) = jin10_imp_res {
            for r in lst.iter().take(5) {
                let t = r.published_date.as_deref().unwrap_or("");
                let snippet: String = r.snippet.chars().take(160).collect();
                jin10_imp_items.push(format!("- **{}** {}  \n  {}", r.title, t, snippet));
            }
        }
        if !jin10_imp_items.is_empty() {
            sections.push(format!("### ⭐ 金十重磅快讯（近6小时标星）\n{}", jin10_imp_items.join("\n")));
            info!("[宏观新闻][jin10] 金十标星 {} 条", jin10_imp_items.len());
        }

        let mut jin10_flash_items: Vec<String> = Vec::new();
        if let Ok(lst) = jin10_flash_res {
            // 去重：不再包含已在 important 列表里的
            let imp_titles: std::collections::HashSet<String> = jin10_imp_items.iter()
                .map(|s| s.chars().take(40).collect())
                .collect();
            for r in lst.iter() {
                let key: String = format!("- **{}**", r.title).chars().take(40).collect();
                if imp_titles.contains(&key) { continue; }
                let t = r.published_date.as_deref().unwrap_or("");
                let snippet: String = r.snippet.chars().take(140).collect();
                jin10_flash_items.push(format!("- **{}** {}  \n  {}", r.title, t, snippet));
                if jin10_flash_items.len() >= 6 { break; }
            }
        }
        if !jin10_flash_items.is_empty() {
            sections.push(format!("### 📣 金十快讯（今日实时）\n{}", jin10_flash_items.join("\n")));
            info!("[宏观新闻][jin10] 金十快讯补充 {} 条", jin10_flash_items.len());
        } else if jin10_imp_items.is_empty() {
            warn!("[宏观新闻][jin10] 金十快讯为空或超时");
        }

        // ── 金十财经日历（今天 + 明天的重要经济数据 / 事件）──
        match calendar_res {
            Ok(events) if !events.is_empty() => {
                let mut cal_lines: Vec<String> = Vec::new();
                for ev in events.iter().take(15) {
                    let stars = "★".repeat(ev.star.min(3) as usize);
                    let mut extra: Vec<String> = Vec::new();
                    if let Some(p) = &ev.previous { extra.push(format!("前值 {}", p)); }
                    if let Some(f) = &ev.forecast { extra.push(format!("预期 {}", f)); }
                    if let Some(a) = &ev.actual { extra.push(format!("公布 {}", a)); }
                    let tail = if extra.is_empty() { String::new() } else { format!("  \n  {}", extra.join(" | ")) };
                    cal_lines.push(format!(
                        "- `{} {}` {} **[{}]** {}{}",
                        ev.date, ev.time, stars, ev.country, ev.name, tail
                    ));
                }
                sections.push(format!("### 📅 财经日历（金十，未来48h重要事件）\n{}", cal_lines.join("\n")));
                info!("[宏观新闻][jin10] 财经日历 {} 条", events.len());
            }
            Ok(_) => {
                info!("[宏观新闻][jin10] 财经日历窗口内无重要事件");
            }
            Err(e) => {
                warn!("[宏观新闻][jin10] 财经日历抓取失败: {}", e);
            }
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

// extract_domain 已迁移至 super::types::extract_domain

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
}
