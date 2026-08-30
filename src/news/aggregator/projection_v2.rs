//! BR-174 notification projection gated by a verified ingress receipt.
//!
//! Raw provider results cannot cross this boundary. The database owner creates
//! [`ReceiptedRawNewsBatch`] only after validating the immutable ingress
//! manifest, receipt and external audit chain. Simhash is advanced only here.

use super::feed::record_to_market_event;
use super::raw_v2::RawNewsAggregationBatch;
use crate::signal::market_event::MarketEvent;
use chrono::{DateTime, Utc};
use std::collections::HashSet;
use std::fmt;
use std::sync::Mutex;

#[derive(Debug, Clone)]
pub struct ReceiptedRawNewsBatch {
    raw: RawNewsAggregationBatch,
    source_batch_content_hash: String,
    ingress_receipt_hash: String,
    committed_at: DateTime<Utc>,
}

/// Sealed capability produced only by the verified database read model.
#[derive(Debug, Clone, Copy)]
pub struct VerifiedIngressReceiptToken {
    _private: (),
}

impl VerifiedIngressReceiptToken {
    #[cfg(test)]
    const fn fixture() -> Self {
        Self { _private: () }
    }
}

impl ReceiptedRawNewsBatch {
    /// The private token field prevents provider adapters and the monitor
    /// binary from manufacturing an ingress receipt capability.
    pub fn verified(
        _verified_receipt: VerifiedIngressReceiptToken,
        raw: RawNewsAggregationBatch,
        source_batch_content_hash: String,
        ingress_receipt_hash: String,
        committed_at: DateTime<Utc>,
    ) -> Result<Self, NotificationProjectionError> {
        require_hash(&source_batch_content_hash, "source_batch_content_hash")?;
        require_hash(&ingress_receipt_hash, "ingress_receipt_hash")?;
        if committed_at < raw.observed_at() {
            return Err(NotificationProjectionError::new(
                "ingress_receipt_precedes_acquisition",
            ));
        }
        Ok(Self {
            raw,
            source_batch_content_hash,
            ingress_receipt_hash,
            committed_at,
        })
    }

    pub fn source_batch_content_hash(&self) -> &str {
        &self.source_batch_content_hash
    }

    pub fn ingress_receipt_hash(&self) -> &str {
        &self.ingress_receipt_hash
    }

    pub const fn committed_at(&self) -> DateTime<Utc> {
        self.committed_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationProjectionError {
    reason_code: &'static str,
}

impl NotificationProjectionError {
    const fn new(reason_code: &'static str) -> Self {
        Self { reason_code }
    }

    pub const fn reason_code(&self) -> &'static str {
        self.reason_code
    }
}

impl fmt::Display for NotificationProjectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.reason_code)
    }
}

impl std::error::Error for NotificationProjectionError {}

#[derive(Debug, Default)]
pub struct NotificationProjectionState {
    seen_simhash: Mutex<HashSet<u64>>,
}

impl NotificationProjectionState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Project only ingress-receipted provider records, then advance simhash.
    pub fn project_after_ingress(
        &self,
        batch: ReceiptedRawNewsBatch,
    ) -> Result<Vec<MarketEvent>, NotificationProjectionError> {
        let mut events = Vec::new();
        for attempt in batch.raw.attempts() {
            if let Some(records) = attempt.terminal().records() {
                for record in records {
                    events.push(
                        record_to_market_event(attempt.registration().provider, record).map_err(
                            |_| NotificationProjectionError::new("market_event_projection_failed"),
                        )?,
                    );
                }
            }
        }

        let mut seen = self
            .seen_simhash
            .lock()
            .map_err(|_| NotificationProjectionError::new("notification_simhash_lock_poisoned"))?;
        events.retain(|event| seen.insert(event.simhash));
        events.sort_by_key(|event| std::cmp::Reverse(event.occurred_at));
        Ok(events)
    }
}

fn require_hash(value: &str, field: &'static str) -> Result<(), NotificationProjectionError> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(NotificationProjectionError::new(match field {
            "source_batch_content_hash" => "source_batch_content_hash_invalid",
            "ingress_receipt_hash" => "ingress_receipt_hash_invalid",
            _ => "receipt_hash_invalid",
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_gateway::{BatchEvidence, GlobalNewsProvider, GlobalNewsRecord};
    use crate::market_domain::SourceEvidence;
    use crate::news::aggregator::raw_v2::{
        registered_global_news_feeds, RawGlobalNewsFeedAttempt, TestRawGlobalNewsTerminal,
    };
    use chrono::TimeZone;

    const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn record(provider: GlobalNewsProvider) -> GlobalNewsRecord {
        let published_at = Utc
            .with_ymd_and_hms(2026, 7, 28, 1, 0, 0)
            .single()
            .expect("valid fixture time");
        let observed_at = published_at + chrono::Duration::seconds(1);
        GlobalNewsRecord {
            item_id: "TEST_CODE_item".to_owned(),
            title: "TEST_CODE title".to_owned(),
            summary: Some("TEST_CODE body".to_owned()),
            content: None,
            publisher: "TEST_CODE publisher".to_owned(),
            canonical_url: "https://example.com/TEST_CODE_item".to_owned(),
            published_at,
            observed_at,
            instruments: Vec::new(),
            topics: Vec::new(),
            language: "zh-CN".to_owned(),
            evidence: SourceEvidence::new(
                provider.provider_id(),
                "1785200400.000000000",
                "TEST_CODE_batch",
            )
            .expect("valid source evidence"),
        }
    }

    fn raw_batch() -> RawNewsAggregationBatch {
        let registrations = registered_global_news_feeds();
        let observed_at = Utc
            .with_ymd_and_hms(2026, 7, 28, 1, 0, 2)
            .single()
            .expect("valid fixture time");
        let attempts = registrations
            .into_iter()
            .map(|registration| {
                let terminal = if registration.provider == GlobalNewsProvider::Eastmoney {
                    let news = record(registration.provider);
                    TestRawGlobalNewsTerminal::Available {
                        evidence: BatchEvidence {
                            provider: registration.provider.provider_id(),
                            source: registration.provider.source().to_owned(),
                            source_at: news.evidence.source_at().map(str::to_owned),
                            observed_at: news.observed_at.to_rfc3339(),
                            batch_id: news.evidence.batch_id().to_owned(),
                        },
                        records: vec![news],
                    }
                } else {
                    TestRawGlobalNewsTerminal::VerifiedEmpty {
                        evidence: BatchEvidence {
                            provider: registration.provider.provider_id(),
                            source: registration.provider.source().to_owned(),
                            source_at: None,
                            observed_at: observed_at.to_rfc3339(),
                            batch_id: format!("TEST_CODE_{:?}", registration.provider),
                        },
                    }
                };
                RawGlobalNewsFeedAttempt::test_fixture(
                    "TEST_CODE_projection_attempt",
                    registration,
                    observed_at,
                    terminal,
                )
            })
            .collect();
        RawNewsAggregationBatch::test_fixture("TEST_CODE_projection_batch", attempts, observed_at)
    }

    #[test]
    fn receipt_must_follow_acquisition_and_have_canonical_hashes() {
        let raw = raw_batch();
        let before = raw.observed_at() - chrono::Duration::seconds(1);
        assert_eq!(
            ReceiptedRawNewsBatch::verified(
                VerifiedIngressReceiptToken::fixture(),
                raw.clone(),
                HASH.into(),
                HASH.into(),
                before,
            )
            .unwrap_err()
            .reason_code(),
            "ingress_receipt_precedes_acquisition"
        );
        assert_eq!(
            ReceiptedRawNewsBatch::verified(
                VerifiedIngressReceiptToken::fixture(),
                raw,
                "bad".into(),
                HASH.into(),
                before,
            )
            .unwrap_err()
            .reason_code(),
            "source_batch_content_hash_invalid"
        );
    }

    #[test]
    fn simhash_advances_only_after_receipted_projection() {
        let raw = raw_batch();
        let committed_at = raw.observed_at() + chrono::Duration::seconds(1);
        let state = NotificationProjectionState::new();

        let first = ReceiptedRawNewsBatch::verified(
            VerifiedIngressReceiptToken::fixture(),
            raw.clone(),
            HASH.into(),
            HASH.into(),
            committed_at,
        )
        .expect("verified receipt");
        assert_eq!(state.project_after_ingress(first).unwrap().len(), 1);

        let replay = ReceiptedRawNewsBatch::verified(
            VerifiedIngressReceiptToken::fixture(),
            raw,
            HASH.into(),
            HASH.into(),
            committed_at,
        )
        .expect("verified replay receipt");
        assert!(state.project_after_ingress(replay).unwrap().is_empty());
    }
}
