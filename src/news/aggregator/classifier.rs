//! v17.7 Task 1: Classification result types
//!
//! Skeleton types for earnings and analyst rating classification results.
//! The actual classification logic will be implemented in Task 3 (earnings)
//! and Task 4 (analyst). These types provide the data contracts that downstream
//! code (Task 5 adapter) will consume.

use chrono::NaiveDate;
use super::SourcePushKind;

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

#[cfg(test)]
mod tests {
    use super::*;

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
