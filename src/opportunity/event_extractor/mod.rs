//! Registered business rules: BR-068.
pub mod adapter;
pub mod classifier;
pub mod core;
pub mod rule_filter;

use crate::analyzer::GeminiAnalyzer;
use crate::search_service::SearchResult;
use crate::signal::market_event::MarketEvent;
pub use adapter::SearchResultAdapter;
use chrono::{Duration, Local};

use core::EventExtractorCore;
use rule_filter::RuleFilter;

/// B-003 (2026-07-09): simhash 汉明距离 ≤ 3 视为同一事件 (spec §4.3 P1-1)
const SIMHASH_DEDUP_HAMMING_MAX: u32 = 3;

/// CR-11 (review): 改用 crate::signal::market_event::hamming_distance (已 pub)
///   删除本地 simhash_hamming 重复实现.
use crate::signal::market_event::hamming_distance as simhash_hamming;

/// B-003: 去除 HTML 标签后取 simhash, 避免 `<em>...` 等标签污染 bigram
/// 修复: 之前 simhash 用 raw title, HTML 标签让相同事件算成不同 simhash
fn simhash_for_dedup(title: &str) -> u64 {
    use crate::signal::market_event::compute_simhash;
    // CR-9 (review): 改用 crate::util::strip_html_tags 共享实现
    compute_simhash(&crate::util::strip_html_tags(title), "")
}

/// B-003: simhash 汉明距离 + LCS 双重判定. simhash 适合长文本, LCS 适合短中文标题.
/// 返回 true 表示两个 title 描述同一事件 (应去重).
fn is_same_event(title_a: &str, sh_a: u64, title_b: &str, sh_b: u64) -> bool {
    // 路径 1: simhash 汉明距离 ≤ 阈值
    if simhash_hamming(sh_a, sh_b) <= SIMHASH_DEDUP_HAMMING_MAX {
        return true;
    }
    // 路径 2: 短中文标题公共子串 ≥ 5 字 (例如 "苹果折叠屏" 跨多源稳定保留)
    if titles_share_substring(title_a, title_b, 5) {
        return true;
    }
    false
}

/// B-003: 两个标题是否共享 ≥ `min_chars` 字的最长公共子串.
/// 用字符级 substring 扫描 (O(N²), batch 内 30 条以下可接受).
fn titles_share_substring(a: &str, b: &str, min_chars: usize) -> bool {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    if a_chars.len() < min_chars || b_chars.len() < min_chars {
        return false;
    }
    let max_len = a_chars.len().min(b_chars.len());
    for len in (min_chars..=max_len).rev() {
        for start in 0..=a_chars.len() - len {
            let sub: String = a_chars[start..start + len].iter().collect();
            if b.contains(&sub) {
                return true;
            }
        }
    }
    false
}

/// 修复 P0-2: 盘前 batch 默认 1 个交易日阈值
/// FIX-6 (review): 收紧 batch max_age 2d → 4h.
/// FIX-MAX-AGE (review): 4h → 1 day (放宽, 真实新闻流合理范围).
///   4h 太严: 大新闻事件 (例政策) 到晚上 8 点后已 > 4h 会被判 stale.
///   1 day (24h): 覆盖盘后 + 晚间所有真实新闻流, 排除过夜 1 周前的旧闻。
///   配合 FIX-B-002 (板块联动真接数据) 后, 误中已消, 不再需要严苛 max_age 防误推.
pub const BATCH_DEFAULT_MAX_AGE: Duration = Duration::hours(24);
/// 修复 P0-2: 盘中增量默认 5 分钟阈值 (spec §5.1)
pub const INCREMENTAL_DEFAULT_MAX_AGE: Duration = Duration::minutes(5);

// === Rules-only (无 AI 依赖, 降级路径) ===

/// 修复 P0-2: Batch — rules-only
pub fn extract_batch_rules_only(items: &[SearchResult]) -> (Vec<MarketEvent>, Vec<MarketEvent>) {
    extract_batch_rules_only_with_max_age(items, BATCH_DEFAULT_MAX_AGE)
}

pub fn extract_batch_rules_only_with_max_age(
    items: &[SearchResult],
    max_age: Duration,
) -> (Vec<MarketEvent>, Vec<MarketEvent>) {
    let now = Local::now();
    let mut fresh = Vec::new();
    let mut stale = Vec::new();
    for sr in items {
        let raw = match SearchResultAdapter::to_raw(sr) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let rm = RuleFilter::filter(&raw);
        if !rm.matched {
            continue;
        }
        let mut me = EventExtractorCore::build_degraded(&raw, rm.event_type);
        if now - raw.published_at > max_age {
            me.stale = true;
            stale.push(me);
        } else {
            fresh.push(me);
        }
    }
    (fresh, stale)
}

pub fn extract_incremental_rules_only(
    items: &[SearchResult],
    max_age: Duration,
) -> (Vec<MarketEvent>, Vec<MarketEvent>) {
    let now = Local::now();
    let mut fresh = Vec::new();
    let mut stale = Vec::new();
    for sr in items {
        let raw = match SearchResultAdapter::to_raw(sr) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let rm = RuleFilter::filter(&raw);
        if !rm.matched {
            continue;
        }
        let mut me = EventExtractorCore::build_degraded(&raw, rm.event_type);
        if now - raw.published_at > max_age {
            me.stale = true;
            stale.push(me);
        } else {
            fresh.push(me);
        }
    }
    (fresh, stale)
}

// === B-003 (2026-07-09): 去重增强版 ===

/// B-003: 去重版 extract_batch_rules_only。
///
/// 双重去重:
/// 1. **跨批次**: `seen_events` 中已见过的 (simhash, title) 列表 (用 simhash + LCS 双重判定) → 跳过
/// 2. **批次内**: 同批次内已保留的事件 → 跳过 (防止 6 sector query 各返 1 次「苹果折叠屏」)
///
/// `seen_events` 由调用方从 DB `event_seen_simhash` 表加载, 函数返回后调用方再保存.
/// 调用方负责生命周期管理 (避免此处直接依赖 DB, 保持纯函数易测).
///
/// CR-14 (review): 加 `max_age: Duration` 参数, 让调用方自定义时间窗口 (之前硬编码 2 天).
///   例: incremental 模式 5min, backfill 模式 7d.
pub fn extract_batch_rules_only_with_seen(
    items: &[SearchResult],
    seen_events: &[(u64, String)],
    max_age: Duration,
) -> (Vec<MarketEvent>, Vec<MarketEvent>) {
    let now = Local::now();
    let mut fresh: Vec<MarketEvent> = Vec::new();
    let mut stale: Vec<MarketEvent> = Vec::new();
    // 批次内已保留的 (simhash, title) 列表, 同时用于 simhash 快速过滤 + LCS 精确判定
    let mut seen_in_batch: Vec<(u64, String)> = Vec::new();

    for sr in items {
        let raw = match SearchResultAdapter::to_raw(sr) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let rm = RuleFilter::filter(&raw);
        if !rm.matched {
            continue;
        }
        let mut me = EventExtractorCore::build_degraded(&raw, rm.event_type);
        if now - raw.published_at > max_age {
            me.stale = true;
            stale.push(me);
            continue;
        }

        // B-003 修复: simhash 重算自 HTML-stripped 完整 title,
        // 避免 HTML 标签截断导致 simhash 漂移
        let sh = simhash_for_dedup(&raw.title);
        me.simhash = sh;

        // 跨批次去重: 与已见事件 (simhash + LCS) → 跳过
        if seen_events
            .iter()
            .any(|(kept_sh, kept_title)| is_same_event(&raw.title, sh, kept_title, *kept_sh))
        {
            continue;
        }
        // 批次内去重: 用 simhash 快速过滤 + LCS 精确判定 (短中文标题)
        let is_dup = seen_in_batch
            .iter()
            .any(|(kept_sh, kept_title)| is_same_event(&raw.title, sh, kept_title, *kept_sh));
        if is_dup {
            continue;
        }

        seen_in_batch.push((sh, raw.title.clone()));
        fresh.push(me);
    }
    (fresh, stale)
}

// === AI 集成 (修复 v9.1 集成缺口) ===

/// 修复 v9.1 集成: 盘前 batch 走完整 AI 路径
/// spec §1.3: adapter → ① 规则预筛 → ② Quick AI → ③ Deep AI
/// 失败 → 退化到 rules-only + ai_degraded=true
pub async fn extract_batch(
    gemini: &GeminiAnalyzer,
    items: &[SearchResult],
    max_age: Duration,
) -> (Vec<MarketEvent>, Vec<MarketEvent>) {
    let now = Local::now();
    let mut fresh = Vec::new();
    let mut stale = Vec::new();
    for sr in items {
        let raw = match SearchResultAdapter::to_raw(sr) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let rm = RuleFilter::filter(&raw);
        if !rm.matched {
            continue;
        }
        if now - raw.published_at > max_age {
            let mut me = EventExtractorCore::build_degraded(&raw, rm.event_type);
            me.stale = true;
            stale.push(me);
            continue;
        }
        let co = crate::opportunity::event_extractor::classifier::EventClassifier::classify_with(
            gemini, &raw.title, &raw.body,
        )
        .await;
        if !co.is_event {
            continue;
        }
        let mut me = EventExtractorCore::extract_with(gemini, &raw).await;
        // 覆盖 classifier 判定的 event_type (Quick AI 更准)
        if let Some(event_type) = co.event_type {
            me.event_type = event_type;
        }
        if let Some(d) = co.direction {
            me.direction = d;
        }
        fresh.push(me);
    }
    (fresh, stale)
}

/// 修复 v9.1 集成: 盘中增量 (不调 Deep, 节省 token)
/// spec §1.3: adapter → ① 规则预筛 → ② Quick AI 分类 → 确定性映射
pub async fn extract_incremental(
    gemini: &GeminiAnalyzer,
    items: &[SearchResult],
    max_age: Duration,
) -> (Vec<MarketEvent>, Vec<MarketEvent>) {
    let now = Local::now();
    let mut fresh = Vec::new();
    let mut stale = Vec::new();
    for sr in items {
        let raw = match SearchResultAdapter::to_raw(sr) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let rm = RuleFilter::filter(&raw);
        if !rm.matched {
            continue;
        }
        if now - raw.published_at > max_age {
            let mut me = EventExtractorCore::from_quick_only(
                &raw,
                &crate::opportunity::event_extractor::classifier::ClassifierOutput {
                    is_event: true,
                    event_type: rm.event_type,
                    direction: None,
                    subject: None,
                    confidence: 0.0,
                },
            );
            me.stale = true;
            stale.push(me);
            continue;
        }
        let co = crate::opportunity::event_extractor::classifier::EventClassifier::classify_with(
            gemini, &raw.title, &raw.body,
        )
        .await;
        if !co.is_event {
            continue;
        }
        let me = EventExtractorCore::from_quick_only(&raw, &co);
        fresh.push(me);
    }
    (fresh, stale)
}

#[cfg(test)]
mod tests {
    //! B-003 (2026-07-09) 事件抽取去重回归基线
    //!
    //! 历史 bug: 「苹果折叠屏」在同批次被 3 个不同 sector query 各返 1 次,
    //! 导致 push log 一行 3 个重复 MarketEvent, 跨日仍持续.
    //!
    //! 修复: intra-batch simhash 汉明距离 ≤ 3 去重 + 跨批次 seen_simhashes 去重.

    use super::*;
    use crate::search_service::{NewsType, Sentiment};
    use chrono::{Duration, Local};

    /// 构造 SearchResult (含 published_date 才能过 adapter).
    /// `body` 留空, 因 build_degraded 用 title 而非 body 算 simhash.
    fn make_sr(title: &str, source: &str, hours_ago: i64) -> SearchResult {
        let published_at = (Local::now() - Duration::hours(hours_ago))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        SearchResult {
            title: title.to_string(),
            snippet: String::new(),
            url: String::new(),
            source: source.to_string(),
            published_date: Some(published_at),
            news_type: NewsType::Industry,
            sentiment: Sentiment::Positive,
            importance: 5,
            relevance: 0.5,
            keywords: vec![],
        }
    }

    /// B-003 场景 1: 同批次 3 条相似「苹果折叠屏」新闻 → 应只产 1 条 (simhash 汉明距 ≤3 去重).
    #[test]
    fn test_extract_batch_dedup_similar_titles() {
        let items = vec![
            make_sr("苹果折叠屏手机已在<em>量产</em>", "东方财富", 1),
            make_sr("苹果折叠屏手机已在<em>量产</em>", "新浪", 2),
            make_sr("苹果折叠屏进入<em>量产 </em>供货阶段", "财联社", 1),
        ];
        let (fresh, stale) =
            extract_batch_rules_only_with_seen(&items, &[], chrono::Duration::days(2));
        assert_eq!(
            fresh.len(),
            1,
            "同批次 3 条相似 title 应只产 1 条 MarketEvent (实际 {} 条)",
            fresh.len()
        );
        assert!(
            stale.is_empty(),
            "1-2 小时前 news 仍在 max_age=2 days 内, 不应 stale"
        );
    }

    /// B-003 场景 2: 完全不同的 2 条新闻 → 都保留.
    #[test]
    fn test_extract_batch_keeps_distinct_titles() {
        let items = vec![
            make_sr("PCB全线涨价20%, HDI高多层板持续紧缺", "东方财富", 1),
            make_sr("国务院通过城市更新十五五规划, 地下管网改造加速", "新华", 1),
        ];
        let (fresh, _stale) =
            extract_batch_rules_only_with_seen(&items, &[], chrono::Duration::days(2));
        assert_eq!(
            fresh.len(),
            2,
            "2 条完全不同的 news 应分别保留 (实际 {} 条)",
            fresh.len()
        );
    }

    /// B-003 场景 3: 跨批次去重 — seen_events 含相似事件 → 应跳过 (用 LCS 而非纯 simhash).
    #[test]
    fn test_extract_batch_dedup_against_seen_events() {
        let seen = vec![(
            simhash_for_dedup("苹果折叠屏手机已在<em>量产</em>"),
            "苹果折叠屏手机已在<em>量产</em>".to_string(),
        )];
        let items = vec![make_sr("苹果折叠屏进入<em>量产 </em>供货阶段", "财联社", 1)];
        let (fresh, _stale) =
            extract_batch_rules_only_with_seen(&items, &seen, chrono::Duration::days(2));
        assert_eq!(
            fresh.len(),
            0,
            "seen_events 含「苹果折叠屏」应通过 LCS 跳过今天的相似事件 (实际 {} 条)",
            fresh.len()
        );
    }

    /// B-003 场景 4: 空 seen_events → 不过滤 (向后兼容).
    #[test]
    fn test_extract_batch_no_seen_returns_all() {
        let items = vec![make_sr("国务院发布数字经济发展规划", "新华", 1)];
        let (fresh, _stale) =
            extract_batch_rules_only_with_seen(&items, &[], chrono::Duration::days(2));
        assert_eq!(fresh.len(), 1);
    }

    /// B-003 场景 5: 旧版函数 extract_batch_rules_only() 仍工作 (向后兼容, 不去重).
    #[test]
    fn test_extract_batch_rules_only_backward_compat() {
        let items = vec![
            make_sr("PCB全线涨价20%, HDI高多层板持续紧缺", "东方财富", 1),
            make_sr("PCB全线涨价20%, HDI高多层板持续紧缺", "新浪", 2),
        ];
        let (fresh, _stale) = extract_batch_rules_only(&items);
        // 向后兼容: 旧 API 不去重, 返 2 条 (新 API _with_seen 才去重)
        assert_eq!(fresh.len(), 2, "旧 API 不做 simhash 去重");
    }
}
