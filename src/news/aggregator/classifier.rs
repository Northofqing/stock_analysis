//! Registered business rules: BR-137.
//! v17.7 Task 1: Classification result types
//!
//! Skeleton types for earnings and analyst rating classification results.
//! The actual classification logic will be implemented in Task 3 (earnings)
//! and Task 4 (analyst). These types provide the data contracts that downstream
//! code (Task 5 adapter) will consume.

use super::{NormalizedSourceError, NormalizedSourceEvent, SourcePushKind};
use crate::data_provider::announcement::Announcement;
use crate::search_service::SearchResult;
use chrono::{Datelike, Local, NaiveDate};

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
    /// Actual EPS beat the consensus forecast by >= beat_threshold_pct.
    Beat,
    /// Actual EPS missed the consensus forecast by <= miss_threshold_pct.
    Miss,
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

// ============================================================================
// Earnings classifier (v17.7 Task 3)
// ============================================================================

/// Configuration for earnings classification thresholds.
#[derive(Debug, Clone)]
pub struct EarningsConfig {
    /// Metric to compare (e.g. "eps").
    pub metric: String,
    /// Beat threshold: delta_pct >= this value → EarningsBeat.
    pub beat_threshold_pct: f64,
    /// Miss threshold: delta_pct <= this value → EarningsMiss.
    pub miss_threshold_pct: f64,
    /// Poll interval in seconds for earnings data.
    pub poll_interval_secs: u64,
}

impl Default for EarningsConfig {
    fn default() -> Self {
        Self {
            metric: "eps".into(),
            beat_threshold_pct: 10.0,
            miss_threshold_pct: -10.0,
            poll_interval_secs: 900,
        }
    }
}

impl EarningsConfig {
    /// Validate that thresholds are finite and correctly signed.
    pub fn validate(&self) -> Result<(), String> {
        if !self.beat_threshold_pct.is_finite() || self.beat_threshold_pct <= 0.0 {
            return Err(format!(
                "beat_threshold_pct must be finite and > 0, got {}",
                self.beat_threshold_pct
            ));
        }
        if !self.miss_threshold_pct.is_finite() || self.miss_threshold_pct >= 0.0 {
            return Err(format!(
                "miss_threshold_pct must be finite and < 0, got {}",
                self.miss_threshold_pct
            ));
        }
        Ok(())
    }
}

/// Classify an earnings period as Beat, Miss, or None.
///
/// Returns `None` if:
/// - The report year does not match the current year.
/// - The consensus reference is missing, non-finite, or zero.
/// - The delta is within the no-push zone (between miss and beat thresholds).
pub fn classify_earnings(
    actual: &crate::data_provider::financials::FinancialPeriod,
    consensus: &crate::data_provider::consensus::ConsensusData,
    config: &EarningsConfig,
) -> Option<EarningsClassification> {
    use chrono::NaiveDate;

    // Parse report_date → year
    let report_date = actual.report_date.as_ref()?;
    let date = NaiveDate::parse_from_str(report_date, "%Y-%m-%d").ok()?;
    let actual_year = date.year();

    // Must be current year
    let current_year = chrono::Local::now().year();
    if actual_year != current_year {
        log::debug!(
            "[classify_earnings] report year {} != current year {}, skipping",
            actual_year,
            current_year
        );
        return None;
    }

    // Reference EPS from consensus
    let reference = consensus.eps_this_year_avg?;
    if !reference.is_finite() || reference.abs() < 1e-9 {
        log::debug!(
            "[classify_earnings] reference EPS not finite or zero: {:?}",
            consensus.eps_this_year_avg
        );
        return None;
    }

    // Actual EPS
    let actual_eps = actual.eps?;
    if !actual_eps.is_finite() {
        log::debug!(
            "[classify_earnings] actual EPS not finite: {:?}",
            actual.eps
        );
        return None;
    }

    // Delta percentage
    let delta_pct = (actual_eps - reference) / reference.abs() * 100.0;

    // Determine classification
    if delta_pct >= config.beat_threshold_pct {
        Some(EarningsClassification {
            kind: EarningsKind::Beat,
            delta_pct,
            actual: actual_eps,
            reference,
            report_date: date,
        })
    } else if delta_pct <= config.miss_threshold_pct {
        Some(EarningsClassification {
            kind: EarningsKind::Miss,
            delta_pct,
            actual: actual_eps,
            reference,
            report_date: date,
        })
    } else {
        // In-range: no push
        None
    }
}

/// Classify an `Announcement` into a `NormalizedSourceEvent`.
///
/// Rejects announcements with empty title or empty code.
/// Uses `a.external_id` for event_id if present, otherwise a deterministic
/// fallback. Direction is derived from `a.level` (Emergency/Important → Bull,
/// Info → Neutral, Skip → reject).
#[cfg(test)]
pub fn classify_announcement(
    a: &Announcement,
) -> Result<NormalizedSourceEvent, NormalizedSourceError> {
    classify_announcement_with_provenance(a, chrono::Local::now(), "eastmoney")
}

pub fn classify_announcement_with_provenance(
    a: &Announcement,
    observed_at: chrono::DateTime<chrono::Local>,
    source: &str,
) -> Result<NormalizedSourceEvent, NormalizedSourceError> {
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

    if source.trim().is_empty() {
        return Err(NormalizedSourceError::EmptySource);
    }
    let url = a.url.clone();
    let published_on = a
        .published_on()
        .map_err(|_| NormalizedSourceError::InvalidPublishedDate)?;

    NormalizedSourceEvent::new(
        SourcePushKind::Announcement,
        event_id,
        Some(a.code.clone()),
        a.title.clone(),
        a.summary.clone(),
        direction,
        70,
        80,
        observed_at,
        Some(published_on),
        false,
        source.to_string(),
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
    let published_on = parse_provider_publication_date(
        r.published_date
            .as_deref()
            .ok_or(NormalizedSourceError::MissingPublishedDate)?,
    )?;
    let observed_at = chrono::Local::now();

    NormalizedSourceEvent::new(
        SourcePushKind::PolicyHit,
        event_id,
        None, // policy is global, no stock code
        r.title.clone(),
        r.snippet.clone(),
        Direction::Bull, // policy is generally bullish
        80,
        90,
        observed_at,
        Some(published_on),
        false,
        r.source.clone(),
        url,
    )
}

fn parse_provider_publication_date(raw: &str) -> Result<NaiveDate, NormalizedSourceError> {
    let raw = raw.trim();
    if let Ok(date) = NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
        return Ok(date);
    }
    for format in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M:%S"] {
        if let Ok(timestamp) = chrono::NaiveDateTime::parse_from_str(raw, format) {
            return Ok(timestamp.date());
        }
    }
    if let Ok(timestamp) = chrono::DateTime::parse_from_rfc3339(raw) {
        return Ok(timestamp.with_timezone(&Local).date_naive());
    }
    if let Ok(timestamp) = chrono::DateTime::parse_from_rfc2822(raw) {
        return Ok(timestamp.with_timezone(&Local).date_naive());
    }
    Err(NormalizedSourceError::InvalidPublishedDate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_provider::announcement::AnnLevel;
    use chrono::Local;

    // -------------------------------------------------------------------------
    // Test helpers (production code does not use these)
    // -------------------------------------------------------------------------

    fn test_important_announcement(external_id: &str, code: &str, title: &str) -> Announcement {
        Announcement {
            code: code.to_string(),
            name: "测试股票".to_string(),
            title: title.to_string(),
            date: Local::now().date_naive().format("%Y-%m-%d").to_string(),
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
            published_date: Some(Local::now().date_naive().format("%Y-%m-%d").to_string()),
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
        let a = test_important_announcement(
            "ann-1",
            "TEST_CODE_ANNOUNCEMENT",
            "关于回购公司股份方案的公告",
        );
        let event = classify_announcement(&a).unwrap();
        assert_eq!(event.push_kind, SourcePushKind::Announcement);
        assert_eq!(event.event_id, "ann-1");
        assert_eq!(event.code.as_deref(), Some("TEST_CODE_ANNOUNCEMENT"));
    }

    #[test]
    fn policy_result_requires_source_and_title() {
        let result = test_search_result(
            "国务院发布产业政策",
            "发改委通知公告",
            "https://example.invalid/policy",
        );
        let event = classify_policy(&result).unwrap();
        assert_eq!(event.push_kind, SourcePushKind::PolicyHit);
        assert_eq!(event.url.as_deref(), Some("https://example.invalid/policy"));
    }

    #[test]
    fn source_classifiers_reject_old_or_missing_provider_dates() {
        let mut announcement =
            test_important_announcement("ann-stale", "TEST_CODE_STALE_ANNOUNCEMENT", "历史公告");
        announcement.date = (Local::now().date_naive() - chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string();
        assert_eq!(
            classify_announcement(&announcement),
            Err(NormalizedSourceError::Stale)
        );

        let mut policy = test_search_result("政策", "provider", "");
        policy.published_date = None;
        assert_eq!(
            classify_policy(&policy),
            Err(NormalizedSourceError::MissingPublishedDate)
        );

        policy.published_date = Some(format!(
            "{}garbage",
            Local::now().date_naive().format("%Y-%m-%d")
        ));
        assert_eq!(
            classify_policy(&policy),
            Err(NormalizedSourceError::InvalidPublishedDate)
        );
    }

    // -------------------------------------------------------------------------
    // Sanity tests
    // -------------------------------------------------------------------------

    #[test]
    fn announcement_with_empty_title_is_rejected() {
        let a = test_important_announcement("ann-x", "TEST_CODE_EMPTY_TITLE", "");
        let err = classify_announcement(&a).unwrap_err();
        assert!(matches!(err, NormalizedSourceError::EmptyTitle));
    }

    #[test]
    fn announcement_external_id_is_preserved() {
        let a = test_important_announcement("art-12345", "TEST_CODE_FALLBACK_ID", "回购公告");
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

    // =========================================================================
    // Earnings classification tests (v17.7 Task 3)
    // =========================================================================

    /// Test helper: build a FinancialPeriod with given report_date and EPS.
    fn financial_period(
        date_str: &str,
        eps: f64,
    ) -> crate::data_provider::financials::FinancialPeriod {
        crate::data_provider::financials::FinancialPeriod {
            report_date: Some(date_str.to_string()),
            eps: Some(eps),
            roe: None,
            revenue_yoy: None,
            net_profit_yoy: None,
            gross_margin: None,
            net_margin: None,
            op_cash_flow_ps: None,
            total_asset_turnover: None,
            debt_to_assets: None,
        }
    }

    /// Test helper: build a ConsensusData with given eps_this_year_avg.
    fn consensus_with_eps_this_year(eps: f64) -> crate::data_provider::consensus::ConsensusData {
        crate::data_provider::consensus::ConsensusData {
            report_count: 1,
            broker_count: 1,
            eps_this_year_avg: Some(eps),
            eps_next_year_avg: None,
            eps_next2_year_avg: None,
            rating_distribution: Default::default(),
            target_price_high_avg: None,
            target_price_low_avg: None,
            latest_report_date: None,
            recent_reports: vec![],
        }
    }

    #[test]
    fn eps_ten_percent_above_same_year_forecast_is_beat() {
        let actual = financial_period("2026-06-30", 1.10);
        let consensus = consensus_with_eps_this_year(1.00);
        let result = classify_earnings(&actual, &consensus, &EarningsConfig::default()).unwrap();
        assert_eq!(result.kind, EarningsKind::Beat);
        assert!((result.delta_pct - 10.0).abs() < 1e-9);
    }

    #[test]
    fn eps_ten_percent_below_same_year_forecast_is_miss() {
        let actual = financial_period("2026-06-30", 0.89);
        let consensus = consensus_with_eps_this_year(1.00);
        let result = classify_earnings(&actual, &consensus, &EarningsConfig::default()).unwrap();
        assert_eq!(result.kind, EarningsKind::Miss);
    }

    #[test]
    fn mismatched_year_or_missing_forecast_emits_no_classification() {
        let actual = financial_period("2025-12-31", 1.50);
        let consensus = consensus_with_eps_this_year(1.00);
        assert!(classify_earnings(&actual, &consensus, &EarningsConfig::default()).is_none());
        // Missing consensus EPS
        assert!(classify_earnings(
            &financial_period("2026-06-30", 1.10),
            &crate::data_provider::consensus::ConsensusData::default(),
            &EarningsConfig::default()
        )
        .is_none());
    }

    #[test]
    fn eps_within_threshold_emits_no_classification() {
        // 1.05 vs 1.00 = +5%, which is within [-10%, +10%] band
        let actual = financial_period("2026-06-30", 1.05);
        let consensus = consensus_with_eps_this_year(1.00);
        assert!(classify_earnings(&actual, &consensus, &EarningsConfig::default()).is_none());
    }

    #[test]
    fn earnings_config_validate_rejects_inverted_thresholds() {
        // beat <= 0 is invalid
        let bad_beat = EarningsConfig {
            metric: "eps".into(),
            beat_threshold_pct: 0.0,
            miss_threshold_pct: -10.0,
            poll_interval_secs: 900,
        };
        assert!(bad_beat.validate().is_err());

        // miss >= 0 is invalid
        let bad_miss = EarningsConfig {
            metric: "eps".into(),
            beat_threshold_pct: 10.0,
            miss_threshold_pct: 0.0,
            poll_interval_secs: 900,
        };
        assert!(bad_miss.validate().is_err());

        // valid config
        assert!(EarningsConfig::default().validate().is_ok());
    }
}
