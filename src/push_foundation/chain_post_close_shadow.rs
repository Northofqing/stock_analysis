//! Read-only policy comparison for the legacy 15:30 chain report.

use anyhow::Result;
use chrono::{DateTime, FixedOffset};

use crate::app::chain_schedule::{ChainPhase, ChainScheduleStore};

use super::chain_preopen_shadow::{observe_chain_schedule_shadow, ChainPreopenShadowReport};

pub type ChainPostCloseShadowReport = ChainPreopenShadowReport;

pub fn observe_chain_post_close_shadow(
    store: &ChainScheduleStore,
    observed_at: DateTime<FixedOffset>,
    trading_day: bool,
) -> Result<ChainPostCloseShadowReport> {
    observe_chain_schedule_shadow(store, observed_at, trading_day, ChainPhase::Postclose)
}

#[cfg(test)]
mod tests {
    use chrono::{FixedOffset, NaiveDate, TimeZone};

    use crate::app::chain_schedule::{ChainPhase, ChainScheduleStatus, ChainScheduleStore};
    use crate::push_foundation::observe_chain_preopen_shadow;

    use super::observe_chain_post_close_shadow;

    fn at(day: u32, hour: u32, minute: u32) -> chrono::DateTime<FixedOffset> {
        FixedOffset::east_opt(8 * 3600)
            .unwrap()
            .with_ymd_and_hms(2026, 9, day, hour, minute, 0)
            .unwrap()
    }

    #[test]
    fn postclose_boundaries_identity_and_missing_store_are_read_only() {
        let root = tempfile::tempdir().unwrap();
        let store = ChainScheduleStore::new(root.path().join("missing/chain.sqlite3"));
        let before = observe_chain_post_close_shadow(&store, at(28, 15, 29), true).unwrap();
        let open = observe_chain_post_close_shadow(&store, at(28, 15, 30), true).unwrap();
        let last = observe_chain_post_close_shadow(&store, at(28, 15, 34), true).unwrap();
        let expired = observe_chain_post_close_shadow(&store, at(28, 15, 35), true).unwrap();
        let next_day = observe_chain_post_close_shadow(&store, at(29, 15, 30), true).unwrap();
        let preopen = observe_chain_preopen_shadow(&store, at(28, 9, 5), true).unwrap();
        assert!(!store.path().exists());
        assert!(!root.path().join("missing").exists());
        assert_eq!(before.legacy_status, ChainScheduleStatus::Ready);
        assert_eq!(before.foundation_status, "expected");
        assert!(!before.legacy_window_open && !before.foundation_window_open);
        assert!(!before.has_decision_diff());
        assert_eq!(open.foundation_status, "eligible");
        assert!(open.legacy_due && open.foundation_due);
        assert_eq!(open.foundation_occurrence_id, last.foundation_occurrence_id);
        assert_ne!(
            open.foundation_occurrence_id,
            next_day.foundation_occurrence_id
        );
        assert_ne!(
            open.foundation_occurrence_id,
            preopen.foundation_occurrence_id
        );
        assert_eq!(open.legacy_occurrence_key, "postclose:2026-09-28");
        assert!(!last.has_decision_diff());
        assert_eq!(expired.foundation_status, "missed");
        assert!(!expired.legacy_window_open && !expired.foundation_window_open);
        assert!(expired.legacy_miss_due && expired.foundation_miss_due);
        assert!(!expired.has_decision_diff());
    }

    #[test]
    fn persisted_unknown_and_weak_acceptance_are_visible() {
        let root = tempfile::tempdir().unwrap();
        let store = ChainScheduleStore::new(root.path().join("chain.sqlite3"));
        let date = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
        let attempt = store
            .begin_send(ChainPhase::Postclose, date, "reports/postclose.md")
            .unwrap();
        let uncertain = observe_chain_post_close_shadow(&store, at(28, 15, 31), true).unwrap();
        assert_eq!(uncertain.legacy_status, ChainScheduleStatus::Uncertain);
        assert!(!uncertain.legacy_due && uncertain.foundation_due);
        assert!(uncertain.has_decision_diff());
        store
            .mark_weak_accepted(ChainPhase::Postclose, date, attempt)
            .unwrap();
        let accepted = observe_chain_post_close_shadow(&store, at(28, 15, 31), true).unwrap();
        assert_eq!(accepted.legacy_status, ChainScheduleStatus::Closed);
        assert!(accepted.legacy_closed && !accepted.foundation_closed);
        assert!(accepted.has_decision_diff());
    }

    #[test]
    fn recorded_miss_and_nontrading_day_are_visible() {
        let root = tempfile::tempdir().unwrap();
        let store = ChainScheduleStore::new(root.path().join("chain.sqlite3"));
        let date = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
        store
            .record_missed_window(ChainPhase::Postclose, date, at(28, 15, 35))
            .unwrap();
        let missed = observe_chain_post_close_shadow(&store, at(28, 15, 35), true).unwrap();
        assert!(missed.legacy_miss_recorded);
        assert!(!missed.legacy_miss_due && missed.foundation_miss_due);
        assert!(missed.has_decision_diff());

        let nontrading = observe_chain_post_close_shadow(&store, at(27, 15, 30), false).unwrap();
        assert_eq!(nontrading.foundation_status, "no_occurrence");
        assert!(nontrading.foundation_occurrence_id.is_none());
        assert!(!nontrading.legacy_due && !nontrading.foundation_due);
        assert!(!nontrading.has_decision_diff());
    }
}
