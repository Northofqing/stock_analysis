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

/// Batch — rules-only, no AI call.
/// Discard rules → filter, unknown → degraded with EventType::Other.
pub fn extract_batch_rules_only(items: &[SearchResult]) -> Vec<MarketEvent> {
    let mut events = Vec::new();
    for sr in items {
        let raw = match SearchResultAdapter::to_raw(sr) { Ok(r) => r, Err(_) => continue };
        let rm = RuleFilter::filter(&raw);
        if !rm.matched { continue; }
        let me = EventExtractorCore::build_degraded(&raw, rm.event_type);
        events.push(me);
    }
    events
}

/// Incremental — rules-only with freshness age filter.
pub fn extract_incremental_rules_only(items: &[SearchResult], max_age: Duration) -> Vec<MarketEvent> {
    let now = Local::now();
    let mut events = Vec::new();
    for sr in items {
        let raw = match SearchResultAdapter::to_raw(sr) { Ok(r) => r, Err(_) => continue };
        if now - raw.published_at > max_age { continue; }
        let rm = RuleFilter::filter(&raw);
        if !rm.matched { continue; }
        let me = EventExtractorCore::build_degraded(&raw, rm.event_type);
        events.push(me);
    }
    events
}
