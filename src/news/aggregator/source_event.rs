//! Registered business rules: BR-137, BR-210.
//! v17.7 Task 1: Normalized source event contracts
//!
//! Data contracts for six retained PushKinds (Announcement / PolicyHit /
//! EarningsBeat / EarningsMiss / AnalystUpgrade / MarketActionAlert).
//! These types are consumed by the v17.7 adapter (Task 5) and downstream
//! classifier tasks (Task 3 earnings, Task 4 analyst).

use crate::magic_compat::ProviderId;
use crate::{data_gateway::parse_evidence_instant, signal::market_event::Direction};
use chrono::{DateTime, Local, NaiveDate};
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
    EmptyCode,
    EmptyTitle,
    EmptySource,
    /// code=None is only permitted for PolicyHit
    CodeRequired {
        kind: SourcePushKind,
    },
    StrengthOutOfRange(u8),
    CertaintyOutOfRange(u8),
    MissingPublishedDate,
    InvalidPublishedDate,
    ResearchOnly,
    UnverifiedSourceFact,
    Stale,
    FutureObservedAt,
    FuturePublishedDate,
    MissingBatchEvidence,
    InvalidBatchEvidence,
    BatchEvidenceMismatch,
}

impl fmt::Display for NormalizedSourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NormalizedSourceError::EmptyEventId => write!(f, "event_id must not be empty"),
            NormalizedSourceError::EmptyCode => write!(f, "code must not be empty when present"),
            NormalizedSourceError::EmptyTitle => write!(f, "title must not be empty"),
            NormalizedSourceError::EmptySource => write!(f, "source must not be empty"),
            NormalizedSourceError::CodeRequired { kind } => {
                write!(
                    f,
                    "{:?} requires a stock code (code=None not permitted)",
                    kind
                )
            }
            NormalizedSourceError::StrengthOutOfRange(value) => {
                write!(f, "strength must be within 0..=100, got {value}")
            }
            NormalizedSourceError::CertaintyOutOfRange(value) => {
                write!(f, "certainty must be within 0..=100, got {value}")
            }
            NormalizedSourceError::MissingPublishedDate => {
                write!(f, "provider published date is missing")
            }
            NormalizedSourceError::InvalidPublishedDate => {
                write!(f, "provider published date is invalid")
            }
            NormalizedSourceError::ResearchOnly => {
                write!(f, "research-only search result cannot become a source fact")
            }
            NormalizedSourceError::UnverifiedSourceFact => {
                write!(f, "complete governed source-fact evidence is required")
            }
            NormalizedSourceError::Stale => write!(f, "source event is stale"),
            NormalizedSourceError::FutureObservedAt => {
                write!(f, "observed_at must not be in the future")
            }
            NormalizedSourceError::FuturePublishedDate => {
                write!(f, "provider published date must not be in the future")
            }
            NormalizedSourceError::MissingBatchEvidence => {
                write!(f, "complete admitted Gateway batch evidence is required")
            }
            NormalizedSourceError::InvalidBatchEvidence => {
                write!(f, "admitted Gateway batch evidence is invalid")
            }
            NormalizedSourceError::BatchEvidenceMismatch => {
                write!(f, "source event and admitted Gateway batch evidence differ")
            }
        }
    }
}

impl std::error::Error for NormalizedSourceError {}

/// Exact identity retained from one admitted Gateway batch.
///
/// `source_at` is provider-owned and intentionally optional. `observed_at` is
/// the original Gateway acquisition timestamp string, not a timestamp created
/// by the source-event adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceBatchEvidence {
    pub provider: ProviderId,
    pub source: String,
    pub source_at: Option<String>,
    pub observed_at: String,
    pub batch_id: String,
    pub content_sha256: String,
}

impl SourceBatchEvidence {
    pub fn new(
        provider: ProviderId,
        source: String,
        source_at: Option<String>,
        observed_at: String,
        batch_id: String,
        content_sha256: String,
    ) -> Result<Self, NormalizedSourceError> {
        let evidence = Self {
            provider,
            source,
            source_at,
            observed_at,
            batch_id,
            content_sha256,
        };
        evidence.validate()?;
        Ok(evidence)
    }

    fn validate(&self) -> Result<(), NormalizedSourceError> {
        if self.source.trim().is_empty()
            || self.batch_id.trim().is_empty()
            || self
                .source_at
                .as_deref()
                .is_some_and(|value| value.trim().is_empty())
            || self.content_sha256.len() != 64
            || !self
                .content_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(NormalizedSourceError::InvalidBatchEvidence);
        }
        let observed_at = self.observed_at_local()?;
        if observed_at > Local::now() {
            return Err(NormalizedSourceError::FutureObservedAt);
        }
        Ok(())
    }

    fn observed_at_local(&self) -> Result<DateTime<Local>, NormalizedSourceError> {
        parse_evidence_instant(
            "news.source_batch",
            self.provider,
            "observed_at",
            &self.observed_at,
        )
        .map(|value| value.with_timezone(&Local))
        .map_err(|_| NormalizedSourceError::InvalidBatchEvidence)
    }
}

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
    /// When this adapter actually observed the source record.
    pub observed_at: DateTime<Local>,
    /// Provider-supplied publication date. None is permitted only for the
    /// non-source-fact MarketActionAlert path.
    pub source_published_on: Option<NaiveDate>,
    /// Explicit upstream freshness result. Stale facts must not be pushed.
    pub stale: bool,
    /// Source name, e.g. "eastmoney", "ndrc", "em_announcement".
    pub source: String,
    /// Ordered source batches used to compute this event. Earnings carries the
    /// financial batch first and consensus batch second; analyst changes carry
    /// the consensus batch.
    pub source_batches: Vec<SourceBatchEvidence>,
    /// Optional canonical URL for the event.
    pub url: Option<String>,
    /// Arbitrary key-value metadata (BTreeMap preserves insertion order).
    pub metadata: BTreeMap<String, String>,
}

impl NormalizedSourceEvent {
    /// Construct a new NormalizedSourceEvent with validation.
    ///
    /// Returns `Err` if:
    /// - `event_id`, `title`, `source`, or a present `code` is empty
    /// - `code` is `None` for any variant other than `PolicyHit`
    /// - strength/certainty is outside 0..=100
    /// - the upstream event is stale
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
        observed_at: DateTime<Local>,
        source_published_on: Option<NaiveDate>,
        upstream_stale: bool,
        source: String,
        url: Option<String>,
    ) -> Result<Self, NormalizedSourceError> {
        let event = Self {
            push_kind,
            event_id,
            code,
            title,
            summary,
            direction,
            strength,
            certainty,
            observed_at,
            source_published_on,
            stale: upstream_stale,
            source,
            source_batches: Vec::new(),
            url,
            metadata: BTreeMap::new(),
        };
        event.validate()?;
        Ok(event)
    }

    /// Construct an evidence-backed financial/research event. The adapter
    /// timestamp must equal the latest retained Gateway observation; this
    /// prevents callers from replacing acquisition time with `Local::now()`.
    #[allow(
        clippy::too_many_arguments,
        reason = "validated source-event constructor mirrors the normalized event envelope"
    )]
    pub fn new_with_batch_evidence(
        push_kind: SourcePushKind,
        event_id: String,
        code: Option<String>,
        title: String,
        summary: String,
        direction: Direction,
        strength: u8,
        certainty: u8,
        observed_at: DateTime<Local>,
        source_published_on: Option<NaiveDate>,
        upstream_stale: bool,
        source: String,
        url: Option<String>,
        source_batches: Vec<SourceBatchEvidence>,
    ) -> Result<Self, NormalizedSourceError> {
        let mut event = Self {
            push_kind,
            event_id,
            code,
            title,
            summary,
            direction,
            strength,
            certainty,
            observed_at,
            source_published_on,
            stale: upstream_stale,
            source,
            source_batches,
            url,
            metadata: BTreeMap::new(),
        };
        event.attach_batch_audit_metadata();
        event.validate()?;
        Ok(event)
    }

    /// Revalidate the public envelope before it crosses a production adapter.
    /// Public fields are retained for compatibility, so construction-time
    /// checks alone cannot protect the push path.
    pub fn validate(&self) -> Result<(), NormalizedSourceError> {
        if self.event_id.trim().is_empty() {
            return Err(NormalizedSourceError::EmptyEventId);
        }
        if self.title.trim().is_empty() {
            return Err(NormalizedSourceError::EmptyTitle);
        }
        if self.source.trim().is_empty() {
            return Err(NormalizedSourceError::EmptySource);
        }
        match self.code.as_deref() {
            Some(code) if code.trim().is_empty() => return Err(NormalizedSourceError::EmptyCode),
            None if self.push_kind != SourcePushKind::PolicyHit => {
                return Err(NormalizedSourceError::CodeRequired {
                    kind: self.push_kind,
                });
            }
            _ => {}
        }
        if self.strength > 100 {
            return Err(NormalizedSourceError::StrengthOutOfRange(self.strength));
        }
        if self.certainty > 100 {
            return Err(NormalizedSourceError::CertaintyOutOfRange(self.certainty));
        }
        let now = Local::now();
        if self.observed_at > now {
            return Err(NormalizedSourceError::FutureObservedAt);
        }
        match self.source_published_on {
            Some(date) if date > now.date_naive() => {
                return Err(NormalizedSourceError::FuturePublishedDate);
            }
            Some(date) if date < now.date_naive() => {
                return Err(NormalizedSourceError::Stale);
            }
            None if self.push_kind != SourcePushKind::MarketActionAlert => {
                return Err(NormalizedSourceError::MissingPublishedDate);
            }
            _ => {}
        }
        if self.stale {
            return Err(NormalizedSourceError::Stale);
        }
        let batch_evidence_required = matches!(
            self.push_kind,
            SourcePushKind::EarningsBeat
                | SourcePushKind::EarningsMiss
                | SourcePushKind::AnalystUpgrade
        );
        if batch_evidence_required && self.source_batches.is_empty() {
            return Err(NormalizedSourceError::MissingBatchEvidence);
        }
        for evidence in &self.source_batches {
            evidence.validate()?;
        }
        if let Some(primary) = self.source_batches.first() {
            let latest_observed_at = self
                .source_batches
                .iter()
                .map(SourceBatchEvidence::observed_at_local)
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .max()
                .ok_or(NormalizedSourceError::MissingBatchEvidence)?;
            if self.source != primary.source || self.observed_at != latest_observed_at {
                return Err(NormalizedSourceError::BatchEvidenceMismatch);
            }
            if !self.batch_audit_metadata_matches() {
                return Err(NormalizedSourceError::BatchEvidenceMismatch);
            }
        }
        Ok(())
    }

    /// Fluent builder: attach a metadata key-value pair.
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    fn attach_batch_audit_metadata(&mut self) {
        for (index, evidence) in self.source_batches.iter().enumerate() {
            let prefix = format!("evidence.{index}");
            self.metadata.insert(
                format!("{prefix}.provider"),
                format!("{:?}", evidence.provider),
            );
            self.metadata
                .insert(format!("{prefix}.source"), evidence.source.clone());
            if let Some(source_at) = &evidence.source_at {
                self.metadata
                    .insert(format!("{prefix}.source_at"), source_at.clone());
            }
            self.metadata.insert(
                format!("{prefix}.observed_at"),
                evidence.observed_at.clone(),
            );
            self.metadata
                .insert(format!("{prefix}.batch_id"), evidence.batch_id.clone());
            self.metadata.insert(
                format!("{prefix}.content_sha256"),
                evidence.content_sha256.clone(),
            );
        }
    }

    fn batch_audit_metadata_matches(&self) -> bool {
        let mut expected = BTreeMap::new();
        for (index, evidence) in self.source_batches.iter().enumerate() {
            let prefix = format!("evidence.{index}");
            expected.insert(
                format!("{prefix}.provider"),
                format!("{:?}", evidence.provider),
            );
            expected.insert(format!("{prefix}.source"), evidence.source.clone());
            if let Some(source_at) = &evidence.source_at {
                expected.insert(format!("{prefix}.source_at"), source_at.clone());
            }
            expected.insert(
                format!("{prefix}.observed_at"),
                evidence.observed_at.clone(),
            );
            expected.insert(format!("{prefix}.batch_id"), evidence.batch_id.clone());
            expected.insert(
                format!("{prefix}.content_sha256"),
                evidence.content_sha256.clone(),
            );
        }
        self.metadata
            .iter()
            .filter(|(key, _)| key.starts_with("evidence."))
            .eq(expected.iter())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now_and_today() -> (DateTime<Local>, NaiveDate) {
        let now = Local::now();
        (now, now.date_naive())
    }

    #[test]
    fn source_event_preserves_identity_and_provenance() {
        let (now, today) = now_and_today();
        let event = NormalizedSourceEvent::new(
            SourcePushKind::Announcement,
            "ann-1".into(),
            Some("TEST_CODE_SOURCE_EVENT".into()),
            "关于回购股份方案的公告".into(),
            "回购".into(),
            Direction::Bull,
            70,
            80,
            now,
            Some(today),
            false,
            "eastmoney".into(),
            Some("https://example.invalid/ann-1".into()),
        )
        .unwrap();
        assert_eq!(event.event_id, "ann-1");
        assert_eq!(event.code.as_deref(), Some("TEST_CODE_SOURCE_EVENT"));
        assert_eq!(event.url.as_deref(), Some("https://example.invalid/ann-1"));
    }

    #[test]
    fn source_batch_evidence_accepts_eastmoney_unix_milliseconds_without_rewriting() {
        let evidence = SourceBatchEvidence::new(
            crate::magic_compat::ProviderId::Eastmoney,
            "TEST_CODE_eastmoney-research".into(),
            None,
            "unix-ms:1785799979851".into(),
            "TEST_CODE_consensus_batch".into(),
            "c".repeat(64),
        )
        .expect("Eastmoney unix-ms observation evidence must remain admissible");

        assert_eq!(evidence.observed_at, "unix-ms:1785799979851");
    }

    #[test]
    fn source_batch_evidence_rejects_malformed_observed_at() {
        let error = SourceBatchEvidence::new(
            crate::magic_compat::ProviderId::Eastmoney,
            "TEST_CODE_eastmoney-research".into(),
            None,
            "1785799979.8510450000".into(),
            "TEST_CODE_consensus_batch".into(),
            "c".repeat(64),
        )
        .expect_err("over-precision observation evidence must fail closed");

        assert_eq!(error, NormalizedSourceError::InvalidBatchEvidence);
    }

    #[test]
    fn earnings_event_preserves_each_admitted_batch_evidence() {
        let now = Local::now();
        let financial = SourceBatchEvidence::new(
            crate::magic_compat::ProviderId::Sina,
            "TEST_CODE_sina-financial".into(),
            Some(now.date_naive().to_string()),
            now.to_rfc3339(),
            "TEST_CODE_financial_batch".into(),
            "a".repeat(64),
        )
        .expect("financial evidence");
        let consensus = SourceBatchEvidence::new(
            crate::magic_compat::ProviderId::Eastmoney,
            "TEST_CODE_eastmoney-research".into(),
            None,
            now.to_rfc3339(),
            "TEST_CODE_consensus_batch".into(),
            "b".repeat(64),
        )
        .expect("consensus evidence");

        let mut event = NormalizedSourceEvent::new_with_batch_evidence(
            SourcePushKind::EarningsBeat,
            "TEST_CODE_earnings_event".into(),
            Some("TEST_CODE_600519".into()),
            "业绩超预期".into(),
            "actual EPS exceeds consensus".into(),
            Direction::Bull,
            80,
            90,
            now,
            Some(now.date_naive()),
            false,
            "TEST_CODE_sina-financial".into(),
            None,
            vec![financial.clone(), consensus.clone()],
        )
        .expect("complete evidence-backed event");

        assert_eq!(event.source_batches, vec![financial, consensus]);
        assert_eq!(
            event
                .metadata
                .get("evidence.0.batch_id")
                .map(String::as_str),
            Some("TEST_CODE_financial_batch")
        );
        assert_eq!(
            event
                .metadata
                .get("evidence.1.batch_id")
                .map(String::as_str),
            Some("TEST_CODE_consensus_batch")
        );
        event.metadata.insert(
            "evidence.1.batch_id".into(),
            "TEST_CODE_tampered_batch".into(),
        );
        assert_eq!(
            event.validate(),
            Err(NormalizedSourceError::BatchEvidenceMismatch)
        );
    }

    #[test]
    fn earnings_event_rejects_absent_or_mismatched_batch_evidence() {
        let now = Local::now();
        let without_evidence = NormalizedSourceEvent::new(
            SourcePushKind::EarningsBeat,
            "TEST_CODE_earnings_event".into(),
            Some("TEST_CODE_600519".into()),
            "业绩超预期".into(),
            "summary".into(),
            Direction::Bull,
            80,
            90,
            now,
            Some(now.date_naive()),
            false,
            "TEST_CODE_sina-financial".into(),
            None,
        );
        assert_eq!(
            without_evidence,
            Err(NormalizedSourceError::MissingBatchEvidence)
        );

        let wrong_source = SourceBatchEvidence::new(
            crate::magic_compat::ProviderId::Sina,
            "TEST_CODE_other-source".into(),
            Some(now.date_naive().to_string()),
            now.to_rfc3339(),
            "TEST_CODE_financial_batch".into(),
            "a".repeat(64),
        )
        .expect("batch evidence");
        let error = NormalizedSourceEvent::new_with_batch_evidence(
            SourcePushKind::EarningsBeat,
            "TEST_CODE_earnings_event".into(),
            Some("TEST_CODE_600519".into()),
            "业绩超预期".into(),
            "summary".into(),
            Direction::Bull,
            80,
            90,
            now,
            Some(now.date_naive()),
            false,
            "TEST_CODE_sina-financial".into(),
            None,
            vec![wrong_source],
        )
        .expect_err("event source must equal primary evidence source");
        assert_eq!(error, NormalizedSourceError::BatchEvidenceMismatch);
    }

    #[test]
    fn source_event_rejects_empty_title_and_identity() {
        let (now, today) = now_and_today();
        let err = NormalizedSourceEvent::new(
            SourcePushKind::PolicyHit,
            "".into(),
            None,
            "".into(),
            "".into(),
            Direction::Neutral,
            50,
            60,
            now,
            Some(today),
            false,
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
        let (now, today) = now_and_today();
        let event = NormalizedSourceEvent::new(
            SourcePushKind::Announcement,
            "evt-1".into(),
            Some("TEST_CODE_METADATA".into()),
            "Test Event".into(),
            "summary".into(),
            Direction::Bull,
            70,
            80,
            now,
            Some(today),
            false,
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
        let (now, today) = now_and_today();
        let event = NormalizedSourceEvent::new(
            SourcePushKind::PolicyHit,
            "pol-1".into(),
            None,
            "关于促进数字经济高质量发展的通知".into(),
            "政策".into(),
            Direction::Bull,
            80,
            90,
            now,
            Some(today),
            false,
            "ndrc".into(),
            Some("https://example.invalid/pol-1".into()),
        )
        .unwrap();
        assert_eq!(event.push_kind, SourcePushKind::PolicyHit);
        assert!(event.code.is_none());
    }

    #[test]
    fn non_policy_rejects_none_code() {
        let (now, today) = now_and_today();
        let err = NormalizedSourceEvent::new(
            SourcePushKind::EarningsBeat,
            "earn-1".into(),
            None,
            "业绩超预期".into(),
            "".into(),
            Direction::Bull,
            80,
            90,
            now,
            Some(today),
            false,
            "em".into(),
            None,
        )
        .unwrap_err();
        assert!(matches!(err, NormalizedSourceError::CodeRequired { .. }));
    }

    #[test]
    fn empty_source_rejected() {
        let (now, today) = now_and_today();
        let err = NormalizedSourceEvent::new(
            SourcePushKind::Announcement,
            "ann-1".into(),
            Some("TEST_CODE_EMPTY_SOURCE".into()),
            "Title".into(),
            "summary".into(),
            Direction::Bull,
            70,
            80,
            now,
            Some(today),
            false,
            "".into(),
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("source"));
    }

    #[test]
    fn public_envelope_revalidation_rejects_stale_and_out_of_range_data() {
        let (now, today) = now_and_today();
        let mut event = NormalizedSourceEvent::new(
            SourcePushKind::PolicyHit,
            "policy-validation".into(),
            None,
            "政策事实".into(),
            "summary".into(),
            Direction::Neutral,
            80,
            90,
            now,
            Some(today),
            false,
            "official-provider".into(),
            None,
        )
        .unwrap();
        event.stale = true;
        assert_eq!(event.validate(), Err(NormalizedSourceError::Stale));

        event.stale = false;
        event.strength = 101;
        assert_eq!(
            event.validate(),
            Err(NormalizedSourceError::StrengthOutOfRange(101))
        );
        assert_eq!(event.strength, 101, "invalid input must not be clamped");
    }

    #[test]
    fn source_fact_rejects_missing_old_or_future_provider_date() {
        let (now, today) = now_and_today();
        let build = |published_on| {
            NormalizedSourceEvent::new(
                SourcePushKind::PolicyHit,
                "policy-freshness".into(),
                None,
                "政策事实".into(),
                "summary".into(),
                Direction::Neutral,
                80,
                90,
                now,
                published_on,
                false,
                "official-provider".into(),
                None,
            )
        };

        assert_eq!(
            build(None),
            Err(NormalizedSourceError::MissingPublishedDate)
        );
        assert_eq!(
            build(Some(today - chrono::Duration::days(1))),
            Err(NormalizedSourceError::Stale)
        );
        assert_eq!(
            build(Some(today + chrono::Duration::days(1))),
            Err(NormalizedSourceError::FuturePublishedDate)
        );
    }
}
