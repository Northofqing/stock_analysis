pub mod adapter;
pub mod rule_filter;
pub mod classifier;
pub mod core;

pub use adapter::SearchResultAdapter;
use crate::search_service::SearchResult;
use crate::signal::market_event::MarketEvent;
use chrono::{Local, Duration};

use rule_filter::RuleFilter;
use core::EventExtractorCore;

/// 修复 P0-2: 盘前 batch 默认 1 个交易日阈值
/// 1 交易日 ≈ 1.5 自然日 (含周末), 用 2 自然日作上限保险
pub const BATCH_DEFAULT_MAX_AGE: Duration = Duration::days(2);
/// 修复 P0-2: 盘中增量默认 5 分钟阈值 (spec §5.1)
pub const INCREMENTAL_DEFAULT_MAX_AGE: Duration = Duration::minutes(5);

/// 修复 P0-2: Batch — rules-only, no AI call.
/// 返回 (fresh, stale) 两个桶:
/// - fresh: 在 max_age 范围内, 走后续评分
/// - stale: 超 max_age, 标记 stale=true, 入审计但不参与评分
/// spec §5.1: 盘前 batch published_at 超过 1 个交易日视为过期, 标记 stale=true 并丢弃
pub fn extract_batch_rules_only(items: &[SearchResult]) -> (Vec<MarketEvent>, Vec<MarketEvent>) {
    extract_batch_rules_only_with_max_age(items, BATCH_DEFAULT_MAX_AGE)
}

/// 修复 P0-2: Batch + 自定义 max_age (供测试或特殊场景)
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

/// 修复 P0-2: Incremental — rules-only with freshness age filter.
/// 返回 (fresh, stale) 两个桶.
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
