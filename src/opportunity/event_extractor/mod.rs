pub mod adapter;
pub mod rule_filter;
pub mod classifier;
pub mod core;

pub use adapter::SearchResultAdapter;
use crate::analyzer::GeminiAnalyzer;
use crate::search_service::SearchResult;
use crate::signal::market_event::MarketEvent;
use chrono::{Local, Duration};

use rule_filter::RuleFilter;
use core::EventExtractorCore;

/// 修复 P0-2: 盘前 batch 默认 1 个交易日阈值
pub const BATCH_DEFAULT_MAX_AGE: Duration = Duration::days(2);
/// 修复 P0-2: 盘中增量默认 5 分钟阈值 (spec §5.1)
pub const INCREMENTAL_DEFAULT_MAX_AGE: Duration = Duration::minutes(5);

// === Rules-only (无 AI 依赖, 降级路径) ===

/// 修复 P0-2: Batch — rules-only
pub fn extract_batch_rules_only(items: &[SearchResult]) -> (Vec<MarketEvent>, Vec<MarketEvent>) {
    extract_batch_rules_only_with_max_age(items, BATCH_DEFAULT_MAX_AGE)
}

pub fn extract_batch_rules_only_with_max_age(items: &[SearchResult], max_age: Duration) -> (Vec<MarketEvent>, Vec<MarketEvent>) {
    let now = Local::now();
    let mut fresh = Vec::new();
    let mut stale = Vec::new();
    for sr in items {
        let raw = match SearchResultAdapter::to_raw(sr) { Ok(r) => r, Err(_) => continue };
        let rm = RuleFilter::filter(&raw);
        if !rm.matched { continue; }
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

pub fn extract_incremental_rules_only(items: &[SearchResult], max_age: Duration) -> (Vec<MarketEvent>, Vec<MarketEvent>) {
    let now = Local::now();
    let mut fresh = Vec::new();
    let mut stale = Vec::new();
    for sr in items {
        let raw = match SearchResultAdapter::to_raw(sr) { Ok(r) => r, Err(_) => continue };
        let rm = RuleFilter::filter(&raw);
        if !rm.matched { continue; }
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
        let raw = match SearchResultAdapter::to_raw(sr) { Ok(r) => r, Err(_) => continue };
        let rm = RuleFilter::filter(&raw);
        if !rm.matched { continue; }
        if now - raw.published_at > max_age {
            let mut me = EventExtractorCore::build_degraded(&raw, rm.event_type);
            me.stale = true;
            stale.push(me);
            continue;
        }
        let co = crate::opportunity::event_extractor::classifier::EventClassifier::classify_with(
            gemini, &raw.title, &raw.body,
        ).await;
        if !co.is_event { continue; }
        let mut me = EventExtractorCore::extract_with(gemini, &raw).await;
        // 覆盖 classifier 判定的 event_type (Quick AI 更准)
        if co.is_event && co.event_type.is_some() {
            me.event_type = co.event_type.unwrap();
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
        let raw = match SearchResultAdapter::to_raw(sr) { Ok(r) => r, Err(_) => continue };
        let rm = RuleFilter::filter(&raw);
        if !rm.matched { continue; }
        if now - raw.published_at > max_age {
            let mut me = EventExtractorCore::from_quick_only(&raw, &crate::opportunity::event_extractor::classifier::ClassifierOutput {
                is_event: true, event_type: rm.event_type, direction: None,
                subject: None, confidence: 0.0,
            });
            me.stale = true;
            stale.push(me);
            continue;
        }
        let co = crate::opportunity::event_extractor::classifier::EventClassifier::classify_with(
            gemini, &raw.title, &raw.body,
        ).await;
        if !co.is_event { continue; }
        let me = EventExtractorCore::from_quick_only(&raw, &co);
        fresh.push(me);
    }
    (fresh, stale)
}
