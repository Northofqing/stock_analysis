//! BR-166 typed global-news feeds backed only by the unified data Gateway.

use super::{NewsFeed, NewsFeedOutput, SourceKind};
use crate::data_gateway::{GatewayBatch, GlobalNewsGateway, GlobalNewsProvider, GlobalNewsRecord};
use crate::signal::market_event::{
    compute_simhash, Direction, EventType, MarketEvent, ProviderPublication, SourceRef,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::Local;
use sha2::{Digest, Sha256};

/// One thin registered feed over a released typed upstream provider.
#[derive(Debug, Clone, Copy)]
pub struct UnifiedGlobalNewsFeed {
    provider: GlobalNewsProvider,
    gateway: GlobalNewsGateway,
}

impl UnifiedGlobalNewsFeed {
    pub const fn new(provider: GlobalNewsProvider) -> Self {
        Self {
            provider,
            gateway: GlobalNewsGateway::new(),
        }
    }

    pub const fn provider(&self) -> GlobalNewsProvider {
        self.provider
    }
}

#[async_trait]
impl NewsFeed for UnifiedGlobalNewsFeed {
    fn name(&self) -> &str {
        self.provider.feed_name()
    }

    fn source_kind(&self) -> SourceKind {
        SourceKind::Flash
    }

    async fn fetch(&self, limit: usize) -> Result<NewsFeedOutput> {
        let limit = u32::try_from(limit).context("global-news limit exceeds u32")?;
        let batch = self
            .gateway
            .global_news(self.provider, limit)
            .await
            .with_context(|| format!("{} fetch failed", self.provider.feed_name()))?;
        project_gateway_batch(self.provider, batch)
    }
}

fn project_gateway_batch(
    provider: GlobalNewsProvider,
    batch: GatewayBatch<GlobalNewsRecord>,
) -> Result<NewsFeedOutput> {
    match batch {
        GatewayBatch::Available { records, evidence } => {
            let events = records
                .iter()
                .map(|record| record_to_market_event(provider, record))
                .collect::<Result<Vec<_>>>()?;
            Ok(NewsFeedOutput::from_global_gateway(
                events, records, evidence,
            ))
        }
        GatewayBatch::VerifiedEmpty(_) => Ok(NewsFeedOutput::default()),
    }
}

pub(super) fn record_to_market_event(
    provider: GlobalNewsProvider,
    record: &GlobalNewsRecord,
) -> Result<MarketEvent> {
    let occurred_at = record.published_at.with_timezone(&Local);
    let fetched_at = record.observed_at.with_timezone(&Local);
    if fetched_at < occurred_at {
        anyhow::bail!(
            "BR-166 {} observation precedes publication",
            provider.feed_name()
        );
    }

    let body = record
        .summary
        .as_deref()
        .or(record.content.as_deref())
        .unwrap_or("");
    let simhash = compute_simhash(&record.title, body);
    let mut event_hasher = Sha256::new();
    event_hasher.update(b"BR166_GLOBAL_NEWS_EVENT_V1\0");
    event_hasher.update(provider.source().as_bytes());
    event_hasher.update(b"\0");
    event_hasher.update(record.item_id.as_bytes());
    let event_id = hex::encode(event_hasher.finalize());
    let stale = occurred_at.date_naive() != fetched_at.date_naive();

    Ok(MarketEvent {
        event_id,
        simhash,
        full_title: record.title.clone(),
        event_type: EventType::Other,
        subject: record.publisher.clone(),
        object: Some(record.title.clone()),
        // The source contract proves publication, not price direction or
        // impact. Keep those semantics explicit until downstream classifiers
        // add independently audited evidence.
        direction: Direction::Neutral,
        strength: 0,
        certainty: 100,
        chains: Vec::new(),
        occurred_at,
        provider_publication: Some(ProviderPublication {
            published_on: occurred_at.date_naive(),
            published_at: Some(occurred_at),
        }),
        provenance: vec![SourceRef {
            provider: provider.source().to_string(),
            url: Some(record.canonical_url.clone()),
            fetched_at,
        }],
        ai_degraded: false,
        stale,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_gateway::BatchEvidence;
    use chrono::{DateTime, Utc};
    use crate::magic_compat::{ProviderId, SourceEvidence};

    fn test_record(provider: GlobalNewsProvider) -> GlobalNewsRecord {
        let published_at = DateTime::parse_from_rfc3339("2026-07-25T10:00:00+08:00")
            .expect("TEST_CODE publication")
            .with_timezone(&Utc);
        let observed_at = DateTime::parse_from_rfc3339("2026-07-25T10:00:01+08:00")
            .expect("TEST_CODE observation")
            .with_timezone(&Utc);
        GlobalNewsRecord {
            item_id: "TEST_CODE_item".to_string(),
            title: "TEST_CODE global news title".to_string(),
            summary: Some("TEST_CODE summary".to_string()),
            content: None,
            publisher: "TEST_CODE publisher".to_string(),
            canonical_url: "https://example.com/TEST_CODE_item".to_string(),
            published_at,
            observed_at,
            instruments: Vec::new(),
            topics: Vec::new(),
            language: "zh-CN".to_string(),
            evidence: SourceEvidence::new(
                provider.provider_id(),
                format!(
                    "{}.{:09}",
                    published_at.timestamp(),
                    published_at.timestamp_subsec_nanos()
                ),
                "TEST_CODE_batch",
            )
            .expect("TEST_CODE evidence"),
        }
    }

    #[test]
    fn released_feeds_report_exact_provider_identity() {
        let cases = [
            (
                GlobalNewsProvider::Eastmoney,
                ProviderId::Eastmoney,
                "eastmoney_global_news",
            ),
            (
                GlobalNewsProvider::Cailianpress,
                ProviderId::Cailianpress,
                "cls_global_news",
            ),
            (
                GlobalNewsProvider::Jin10,
                ProviderId::Jin10,
                "jin10_global_news",
            ),
            (
                GlobalNewsProvider::ThePaper,
                ProviderId::ThePaper,
                "thepaper_global_news",
            ),
        ];
        for (provider, provider_id, name) in cases {
            let feed = UnifiedGlobalNewsFeed::new(provider);
            assert_eq!(feed.provider().provider_id(), provider_id);
            assert_eq!(feed.name(), name);
            assert_eq!(feed.source_kind(), SourceKind::Flash);
        }
    }

    #[test]
    fn gateway_projection_retains_records_with_the_same_batch_evidence() {
        let provider = GlobalNewsProvider::Eastmoney;
        let record = test_record(provider);
        let evidence = BatchEvidence {
            provider: provider.provider_id(),
            source: provider.source().to_owned(),
            source_at: record.evidence.source_at().map(str::to_owned),
            observed_at: record.observed_at.to_rfc3339(),
            batch_id: record.evidence.batch_id().to_owned(),
        };

        let output = project_gateway_batch(
            provider,
            GatewayBatch::Available {
                records: vec![record.clone()],
                evidence: evidence.clone(),
            },
        )
        .expect("TEST_CODE projection");

        assert_eq!(output.events().len(), 1);
        assert_eq!(output.admitted_global_news().len(), 1);
        assert_eq!(output.admitted_global_news()[0].records(), &[record]);
        assert_eq!(output.admitted_global_news()[0].evidence(), &evidence);
    }

    #[test]
    fn event_projection_does_not_invent_direction_or_impact() {
        let provider = GlobalNewsProvider::Jin10;
        let record = test_record(provider);
        let event = record_to_market_event(provider, &record).expect("TEST_CODE event projection");

        assert_eq!(event.event_type, EventType::Other);
        assert_eq!(event.direction, Direction::Neutral);
        assert_eq!(event.strength, 0);
        assert_eq!(event.certainty, 100);
        assert_eq!(event.provenance[0].provider, provider.source());
        assert!(!event.stale);
        assert!(event.provider_publication.is_some());
    }

    #[test]
    fn event_identity_is_stable_and_old_publication_is_stale() {
        let provider = GlobalNewsProvider::ThePaper;
        let current = test_record(provider);
        let first = record_to_market_event(provider, &current).expect("TEST_CODE first event");
        let second = record_to_market_event(provider, &current).expect("TEST_CODE second event");
        assert_eq!(first.event_id, second.event_id);
        assert_eq!(first.simhash, second.simhash);

        let mut old = test_record(provider);
        old.published_at -= chrono::Duration::days(1);
        let old = record_to_market_event(provider, &old).expect("TEST_CODE old event");
        assert!(old.stale);
    }
}
