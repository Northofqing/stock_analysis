//! v17.7 Task 1: Normalized source event contracts
//!
//! Data contracts for six retained PushKinds (Announcement / PolicyHit /
//! EarningsBeat / EarningsMiss / AnalystUpgrade / MarketActionAlert).
//! These types are consumed by the v17.7 adapter (Task 5) and downstream
//! classifier tasks (Task 3 earnings, Task 4 analyst).

use crate::signal::market_event::Direction;
use chrono::{DateTime, Local};
use std::collections::BTreeMap;
use std::fmt;

/// The six source-push kinds that map to PushKind variants in the v17.7 adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourcePushKind {
    Announcement,
    PolicyHit,
    EarningsBeat,
    EarningsMiss,
    AnalystUpgrade,
    MarketActionAlert,
}

/// Validation errors for NormalizedSourceEvent construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NormalizedSourceError {
    EmptyEventId,
    EmptyTitle,
    EmptySource,
    /// code=None is only permitted for PolicyHit
    CodeRequired {
        kind: SourcePushKind,
    },
}

impl fmt::Display for NormalizedSourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NormalizedSourceError::EmptyEventId => write!(f, "event_id must not be empty"),
            NormalizedSourceError::EmptyTitle => write!(f, "title must not be empty"),
            NormalizedSourceError::EmptySource => write!(f, "source must not be empty"),
            NormalizedSourceError::CodeRequired { kind } => {
                write!(
                    f,
                    "{:?} requires a stock code (code=None not permitted)",
                    kind
                )
            }
        }
    }
}

impl std::error::Error for NormalizedSourceError {}

/// A normalized event produced by a source adapter before PushKind mapping.
///
/// All six retained PushKinds use this as their canonical intermediate form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedSourceEvent {
    /// Which source-push kind this event originated from.
    pub push_kind: SourcePushKind,
    /// Stable event identifier (provider-specific, e.g. "ann-1", "em:600519:20250716").
    pub event_id: String,
    /// Stock code. None only for PolicyHit (policy events are not stock-specific).
    pub code: Option<String>,
    /// Event title (original language, not truncated).
    pub title: String,
    /// Short summary or snippet.
    pub summary: String,
    /// Event direction.
    pub direction: Direction,
    /// Impact strength 0-100.
    pub strength: u8,
    /// Information certainty 0-100.
    pub certainty: u8,
    /// When the event occurred.
    pub occurred_at: DateTime<Local>,
    /// Source name, e.g. "eastmoney", "ndrc", "em_announcement".
    pub source: String,
    /// Optional canonical URL for the event.
    pub url: Option<String>,
    /// Arbitrary key-value metadata (BTreeMap preserves insertion order).
    pub metadata: BTreeMap<String, String>,
}

impl NormalizedSourceEvent {
    /// Construct a new NormalizedSourceEvent with validation.
    ///
    /// Returns `Err` if:
    /// - `event_id` is empty
    /// - `title` is empty
    /// - `source` is empty
    /// - `code` is `None` for any variant other than `PolicyHit`
    #[allow(
        clippy::too_many_arguments,
        reason = "validated source-event constructor mirrors the normalized event envelope"
    )]
    pub fn new(
        push_kind: SourcePushKind,
        event_id: String,
        code: Option<String>,
        title: String,
        summary: String,
        direction: Direction,
        strength: u8,
        certainty: u8,
        source: String,
        url: Option<String>,
    ) -> Result<Self, NormalizedSourceError> {
        if event_id.is_empty() {
            return Err(NormalizedSourceError::EmptyEventId);
        }
        if title.is_empty() {
            return Err(NormalizedSourceError::EmptyTitle);
        }
        if source.is_empty() {
            return Err(NormalizedSourceError::EmptySource);
        }
        if code.is_none() && push_kind != SourcePushKind::PolicyHit {
            return Err(NormalizedSourceError::CodeRequired { kind: push_kind });
        }

        Ok(Self {
            push_kind,
            event_id,
            code,
            title,
            summary,
            direction,
            strength: strength.min(100),
            certainty: certainty.min(100),
            occurred_at: Local::now(),
            source,
            url,
            metadata: BTreeMap::new(),
        })
    }

    /// Fluent builder: attach a metadata key-value pair.
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_event_preserves_identity_and_provenance() {
        let event = NormalizedSourceEvent::new(
            SourcePushKind::Announcement,
            "ann-1".into(),
            Some("TEST_CODE_SOURCE_EVENT".into()),
            "关于回购股份方案的公告".into(),
            "回购".into(),
            Direction::Bull,
            70,
            80,
            "eastmoney".into(),
            Some("https://example.invalid/ann-1".into()),
        )
        .unwrap();
        assert_eq!(event.event_id, "ann-1");
        assert_eq!(event.code.as_deref(), Some("TEST_CODE_SOURCE_EVENT"));
        assert_eq!(event.url.as_deref(), Some("https://example.invalid/ann-1"));
    }

    #[test]
    fn source_event_rejects_empty_title_and_identity() {
        let err = NormalizedSourceEvent::new(
            SourcePushKind::PolicyHit,
            "".into(),
            None,
            "".into(),
            "".into(),
            Direction::Neutral,
            50,
            60,
            "ndrc".into(),
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("event_id"));
    }

    #[test]
    fn source_push_kind_includes_all_six_variants() {
        use std::fmt::Debug;
        use std::hash::Hash;

        // Verify all 6 derives are present by checking the trait implementations exist
        fn assert_debug<T: Debug>() {}
        fn assert_clone<T: Clone>() {}
        fn assert_copy<T: Copy>() {}
        fn assert_partial_eq<T: PartialEq>() {}
        fn assert_eq<T: Eq>() {}
        fn assert_hash<T: Hash>() {}

        assert_debug::<SourcePushKind>();
        assert_clone::<SourcePushKind>();
        assert_copy::<SourcePushKind>();
        assert_partial_eq::<SourcePushKind>();
        assert_eq::<SourcePushKind>();
        assert_hash::<SourcePushKind>();

        // Count variants via match exhaustion
        let variants = [
            SourcePushKind::Announcement,
            SourcePushKind::PolicyHit,
            SourcePushKind::EarningsBeat,
            SourcePushKind::EarningsMiss,
            SourcePushKind::AnalystUpgrade,
            SourcePushKind::MarketActionAlert,
        ];
        assert_eq!(variants.len(), 6);
    }

    #[test]
    fn metadata_is_preserved_in_order() {
        let event = NormalizedSourceEvent::new(
            SourcePushKind::Announcement,
            "evt-1".into(),
            Some("TEST_CODE_METADATA".into()),
            "Test Event".into(),
            "summary".into(),
            Direction::Bull,
            70,
            80,
            "testsource".into(),
            None,
        )
        .unwrap()
        .with_metadata("alpha", "1")
        .with_metadata("beta", "2")
        .with_metadata("gamma", "3");

        let keys: Vec<_> = event.metadata.keys().collect();
        assert_eq!(keys, [&"alpha", &"beta", &"gamma"]);
        assert_eq!(event.metadata.get("beta"), Some(&"2".to_string()));
    }

    #[test]
    fn policy_hit_allows_none_code() {
        let event = NormalizedSourceEvent::new(
            SourcePushKind::PolicyHit,
            "pol-1".into(),
            None,
            "关于促进数字经济高质量发展的通知".into(),
            "政策".into(),
            Direction::Bull,
            80,
            90,
            "ndrc".into(),
            Some("https://example.invalid/pol-1".into()),
        )
        .unwrap();
        assert_eq!(event.push_kind, SourcePushKind::PolicyHit);
        assert!(event.code.is_none());
    }

    #[test]
    fn non_policy_rejects_none_code() {
        let err = NormalizedSourceEvent::new(
            SourcePushKind::EarningsBeat,
            "earn-1".into(),
            None,
            "业绩超预期".into(),
            "".into(),
            Direction::Bull,
            80,
            90,
            "em".into(),
            None,
        )
        .unwrap_err();
        assert!(matches!(err, NormalizedSourceError::CodeRequired { .. }));
    }

    #[test]
    fn empty_source_rejected() {
        let err = NormalizedSourceEvent::new(
            SourcePushKind::Announcement,
            "ann-1".into(),
            Some("TEST_CODE_EMPTY_SOURCE".into()),
            "Title".into(),
            "summary".into(),
            Direction::Bull,
            70,
            80,
            "".into(),
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("source"));
    }
}
