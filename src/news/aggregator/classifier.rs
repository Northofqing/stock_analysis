//! v17.7 Task 1: Classification result types
//!
//! Skeleton types for earnings and analyst rating classification results.
//! The actual classification logic will be implemented in Task 3 (earnings)
//! and Task 4 (analyst). These types provide the data contracts that downstream
//! code (Task 5 adapter) will consume.

use chrono::NaiveDate;
use crate::data_provider::announcement::Announcement;
use crate::search_service::SearchResult;
use super::{NormalizedSourceEvent, NormalizedSourceError, SourcePushKind};

/// Result of earnings classification (actual vs reference).
///
/// Task 3 will populate the `kind` field with a more precise variant
/// (Beat/Miss/In-Line/Pre-Announced) based on actual vs consensus data.
#[derive(Debug, Clone, PartialEq)]
pub struct EarningsClassification {
    /// Classification kind (to be filled by Task 3).
    pub kind: EarningsKind,
    /// Percentage delta between actual and reference (e.g. +15.3 or -8.7).
    pub delta_pct: f64,
    /// Actual reported value.
    pub actual: f64,
    /// Reference value (consensus forecast or prior period).
    pub reference: f64,
    /// The report date (e.g. earnings release date).
    pub report_date: NaiveDate,
}

/// Earnings classification kinds (expanded by Task 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EarningsKind {
    /// Unclassified / not yet determined.
    #[default]
    Unclassified,
    // Additional variants will be added by Task 3:
    // Beat,
    // Miss,
    // InLine,
    // PreAnnounced,
}

impl EarningsClassification {
    /// Returns the default unclassified result.
    pub fn unclassified() -> Self {
        Self {
            kind: EarningsKind::Unclassified,
            delta_pct: 0.0,
            actual: 0.0,
            reference: 0.0,
            report_date: NaiveDate::from_ymd_opt(1970, 1, 1).unwrap(),
        }
    }
}

/// Result of analyst rating classification (previous vs current).
///
/// Task 4 will populate the `kind`, `previous`, and `current` fields based
/// on xueqiu / sina analyst data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RatingClassification {
    /// The source push kind (AnalystUpgrade or AnalystDowngrade).
    pub kind: SourcePushKind,
    /// Previous rating label (e.g. "持有", "增持", "买入").
    pub previous: String,
    /// Current/new rating label.
    pub current: String,
    /// Date the rating was observed.
    pub observed_at: NaiveDate,
}

impl RatingClassification {
    /// Returns the default unclassified result.
    pub fn unclassified() -> Self {
        Self {
            kind: SourcePushKind::AnalystUpgrade,
            previous: String::new(),
            current: String::new(),
            observed_at: NaiveDate::from_ymd_opt(1970, 1, 1).unwrap(),
        }
    }
}

// ============================================================================
// Announcement and Policy classifiers (v17.7 Task 2)
// ============================================================================

use crate::signal::market_event::Direction;

/// Classify an `Announcement` into a `NormalizedSourceEvent`.
///
/// Rejects announcements with empty title or empty code.
/// Uses `a.external_id` for event_id if present, otherwise a deterministic
/// fallback. Direction is derived from `a.level` (Emergency/Important → Bull,
/// Info → Neutral, Skip → reject).
pub fn classify_announcement(a: &Announcement) -> Result<NormalizedSourceEvent, NormalizedSourceError> {
    if a.title.is_empty() {
        return Err(NormalizedSourceError::EmptyTitle);
    }
    if a.code.is_empty() {
        return Err(NormalizedSourceError::CodeRequired {
            kind: SourcePushKind::Announcement,
        });
    }

    // Direction from level
    let direction = match a.level {
        crate::data_provider::announcement::AnnLevel::Emergency
        | crate::data_provider::announcement::AnnLevel::Important => Direction::Bull,
        crate::data_provider::announcement::AnnLevel::Info => Direction::Neutral,
        crate::data_provider::announcement::AnnLevel::Skip => {
            return Err(NormalizedSourceError::EmptyTitle);
        }
    };

    // event_id: use external_id if available, else deterministic fallback
    let event_id = a
        .external_id
        .clone()
        .unwrap_or_else(|| format!("{}:{}:{}", a.code, a.date, a.title));

    let source = "eastmoney".to_string();
    let url = a.url.clone();

    NormalizedSourceEvent::new(
        SourcePushKind::Announcement,
        event_id,
        Some(a.code.clone()),
        a.title.clone(),
        a.summary.clone(),
        direction,
        70,
        80,
        source,
        url,
    )
}

/// Classify a policy `SearchResult` into a `NormalizedSourceEvent`.
///
/// Policy events have `code=None` (global, not stock-specific).
/// Rejects if title or source is empty.
pub fn classify_policy(r: &SearchResult) -> Result<NormalizedSourceEvent, NormalizedSourceError> {
    if r.title.is_empty() {
        return Err(NormalizedSourceError::EmptyTitle);
    }
    if r.source.is_empty() {
        return Err(NormalizedSourceError::EmptySource);
    }

    // deterministic event_id from source + title
    let event_id = format!("policy:{}:{}", r.source, r.title);
    let url = if r.url.is_empty() {
        None
    } else {
        Some(r.url.clone())
    };

    NormalizedSourceEvent::new(
        SourcePushKind::PolicyHit,
        event_id,
        None, // policy is global, no stock code
        r.title.clone(),
        r.snippet.clone(),
        Direction::Bull, // policy is generally bullish
        80,
        90,
        r.source.clone(),
        url,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_provider::announcement::AnnLevel;

    // -------------------------------------------------------------------------
    // Test helpers (production code does not use these)
    // -------------------------------------------------------------------------

    fn test_important_announcement(
        external_id: &str,
        code: &str,
        title: &str,
    ) -> Announcement {
        Announcement {
            code: code.to_string(),
            name: "测试股票".to_string(),
            title: title.to_string(),
            date: "2026-07-16".to_string(),
            summary: "公告摘要".to_string(),
            content: "正文内容".to_string(),
            level: AnnLevel::Important,
            reason: "重要公告".to_string(),
            external_id: Some(external_id.to_string()),
            url: Some(format!(
                "https://data.eastmoney.com/notices/detail/{}.html",
                external_id
            )),
        }
    }

    fn test_search_result(title: &str, source: &str, url: &str) -> SearchResult {
        SearchResult {
            title: title.to_string(),
            snippet: "政策内容摘要".to_string(),
            url: url.to_string(),
            source: source.to_string(),
            published_date: Some("2026-07-16".to_string()),
            news_type: crate::search_service::NewsType::Policy,
            sentiment: crate::search_service::Sentiment::Positive,
            importance: 5,
            relevance: 1.0,
            keywords: vec![],
        }
    }

    // -------------------------------------------------------------------------
    // Tests from brief
    // -------------------------------------------------------------------------

    #[test]
    fn announcement_maps_to_announcement_not_policy() {
        let a = test_important_announcement("ann-1", "600519", "关于回购公司股份方案的公告");
        let event = classify_announcement(&a).unwrap();
        assert_eq!(event.push_kind, SourcePushKind::Announcement);
        assert_eq!(event.event_id, "ann-1");
        assert_eq!(event.code.as_deref(), Some("600519"));
    }

    #[test]
    fn policy_result_requires_source_and_title() {
        let result = test_search_result("国务院发布产业政策", "发改委通知公告", "https://example.invalid/policy");
        let event = classify_policy(&result).unwrap();
        assert_eq!(event.push_kind, SourcePushKind::PolicyHit);
        assert_eq!(event.url.as_deref(), Some("https://example.invalid/policy"));
    }

    // -------------------------------------------------------------------------
    // Sanity tests
    // -------------------------------------------------------------------------

    #[test]
    fn announcement_with_empty_title_is_rejected() {
        let a = test_important_announcement("ann-x", "600519", "");
        let err = classify_announcement(&a).unwrap_err();
        assert!(matches!(err, NormalizedSourceError::EmptyTitle));
    }

    #[test]
    fn announcement_external_id_is_preserved() {
        let a = test_important_announcement("art-12345", "600519", "回购公告");
        let event = classify_announcement(&a).unwrap();
        // url should reflect the external_id
        assert!(event.url.is_some());
        assert!(event.url.unwrap().contains("art-12345"));
    }

    // -------------------------------------------------------------------------
    // Existing skeleton tests (preserved)
    // -------------------------------------------------------------------------

    #[test]
    fn earnings_classification_default_is_unclassified() {
        let default = EarningsClassification::unclassified();
        assert!(matches!(default.kind, EarningsKind::Unclassified));
    }

    #[test]
    fn rating_classification_default_is_unclassified() {
        let default = RatingClassification::unclassified();
        assert!(default.previous.is_empty());
        assert!(default.current.is_empty());
    }
}
