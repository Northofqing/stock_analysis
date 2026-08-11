//! BR-166/BR-174 global-news source and notification projections.
//!
//! Raw acquisition is owned by [`raw_v2`]. Notification projection and
//! simhash mutation are owned by [`projection_v2`] and require a verified
//! ingress receipt. The compatibility feed projection remains available to
//! consumers that need an inseparable `GlobalNewsRecord + BatchEvidence` view,
//! but it is no longer a scheduler or a source-completeness authority.

pub mod analyst_state;
pub mod classifier;
pub mod feed;
pub mod projection_v2;
pub mod raw_v2;
pub mod source_event;

pub use analyst_state::{AnalystKey, AnalystObservation, AnalystStateStore, ObservationDecision};
pub use source_event::{NormalizedSourceError, NormalizedSourceEvent, SourcePushKind};

use crate::data_gateway::{BatchEvidence, GlobalNewsRecord};
use crate::signal::market_event::{EventType, MarketEvent};
use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};

/// 新闻源类型 (用于 dispatcher 多源共振 + 影响打分加权)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceKind {
    Flash,
    ActiveSearch,
    Policy,
    Earnings,
    MarketAction,
    AnalystView,
}

impl SourceKind {
    pub fn label(&self) -> &'static str {
        match self {
            SourceKind::Flash => "flash",
            SourceKind::ActiveSearch => "active_search",
            SourceKind::Policy => "policy",
            SourceKind::Earnings => "earnings",
            SourceKind::MarketAction => "market_action",
            SourceKind::AnalystView => "analyst_view",
        }
    }
}

/// Compatibility projection seam for the released unified Gateway feeds.
///
/// This trait is not the BR-174 acquisition scheduler. Production selection
/// acquisition must use [`raw_v2::fetch_raw_global_news_batch`].
#[async_trait]
pub trait NewsFeed: Send + Sync {
    fn name(&self) -> &str;
    fn source_kind(&self) -> SourceKind;
    async fn fetch(&self, limit: usize) -> Result<NewsFeedOutput>;
}

/// One admitted global-news batch kept inseparable from the exact provider
/// evidence that produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmittedGlobalNewsBatch {
    records: Vec<GlobalNewsRecord>,
    evidence: BatchEvidence,
}

impl AdmittedGlobalNewsBatch {
    pub fn records(&self) -> &[GlobalNewsRecord] {
        &self.records
    }

    pub const fn evidence(&self) -> &BatchEvidence {
        &self.evidence
    }

    pub fn into_parts(self) -> (Vec<GlobalNewsRecord>, BatchEvidence) {
        (self.records, self.evidence)
    }

    /// 从同一 tick 的 raw acquisition 终端状态构造 admitted batch（BR-172 shadow
    /// 接线用）。仅接受来源已绑定 records + evidence，不伪造。
    pub fn from_parts(records: Vec<GlobalNewsRecord>, evidence: BatchEvidence) -> Self {
        Self { records, evidence }
    }

    #[cfg(test)]
    pub fn test_fixture(records: Vec<GlobalNewsRecord>, evidence: BatchEvidence) -> Self {
        Self { records, evidence }
    }
}

/// Compatibility dual view from one unified Gateway response.
///
/// `admitted_global_news` retains the exact source records and batch evidence.
/// Its `events` field is a lossy notification projection and must never be used
/// as BR-174 selection ingress.
#[derive(Debug, Clone, Default)]
pub struct NewsFeedOutput {
    events: Vec<MarketEvent>,
    admitted_global_news: Vec<AdmittedGlobalNewsBatch>,
}

impl NewsFeedOutput {
    pub fn events(&self) -> &[MarketEvent] {
        &self.events
    }

    pub fn admitted_global_news(&self) -> &[AdmittedGlobalNewsBatch] {
        &self.admitted_global_news
    }

    pub fn into_parts(self) -> (Vec<MarketEvent>, Vec<AdmittedGlobalNewsBatch>) {
        (self.events, self.admitted_global_news)
    }

    pub(super) fn from_global_gateway(
        events: Vec<MarketEvent>,
        records: Vec<GlobalNewsRecord>,
        evidence: BatchEvidence,
    ) -> Self {
        Self {
            events,
            admitted_global_news: vec![AdmittedGlobalNewsBatch { records, evidence }],
        }
    }
}

/// 辅助: MarketEvent 构造
pub fn build_market_event(
    event_type: EventType,
    source_kind: SourceKind,
    title: String,
    subject: String,
    object: String,
    direction: crate::signal::market_event::Direction,
) -> MarketEvent {
    let now: DateTime<Utc> = Utc::now();
    let simhash = {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        title.hash(&mut h);
        h.finish()
    };
    MarketEvent {
        event_id: format!("{}-{:x}", source_kind.label(), simhash),
        simhash,
        full_title: title,
        event_type,
        subject,
        object: Some(object),
        direction,
        strength: 50,
        certainty: 50,
        chains: vec![],
        occurred_at: now.with_timezone(&chrono::Local),
        provider_publication: None,
        provenance: vec![crate::signal::market_event::SourceRef {
            provider: source_kind.label().to_string(),
            url: None,
            fetched_at: now.with_timezone(&chrono::Local),
        }],
        ai_degraded: false,
        stale: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal::market_event::{Direction, EventType};
    use chrono::{TimeZone, Utc};
    use magic_market_core::{ProviderId, SourceEvidence};

    #[test]
    fn source_bound_batch_keeps_record_and_batch_evidence_inseparable() {
        let published_at = Utc
            .with_ymd_and_hms(2026, 7, 27, 9, 30, 0)
            .single()
            .expect("TEST_CODE publication");
        let observed_at = published_at + chrono::Duration::seconds(1);
        let record = GlobalNewsRecord {
            item_id: "TEST_CODE_item".to_owned(),
            title: "TEST_CODE source-bound title".to_owned(),
            summary: Some("TEST_CODE summary".to_owned()),
            content: None,
            publisher: "TEST_CODE publisher".to_owned(),
            canonical_url: "https://example.com/TEST_CODE_item".to_owned(),
            published_at,
            observed_at,
            instruments: vec!["TEST_CODE_600396".to_owned()],
            topics: Vec::new(),
            language: "zh-CN".to_owned(),
            evidence: SourceEvidence::new(
                ProviderId::Eastmoney,
                published_at.to_rfc3339(),
                "TEST_CODE_batch",
            )
            .expect("TEST_CODE source evidence"),
        };
        let evidence = BatchEvidence {
            provider: ProviderId::Eastmoney,
            source: "eastmoney-web".to_owned(),
            source_at: Some(published_at.to_rfc3339()),
            observed_at: observed_at.to_rfc3339(),
            batch_id: "TEST_CODE_batch".to_owned(),
        };
        let output =
            NewsFeedOutput::from_global_gateway(Vec::new(), vec![record.clone()], evidence.clone());

        assert!(output.events().is_empty());
        assert_eq!(output.admitted_global_news().len(), 1);
        assert_eq!(output.admitted_global_news()[0].records(), &[record]);
        assert_eq!(output.admitted_global_news()[0].evidence(), &evidence);
    }

    #[test]
    fn build_market_event_basic() {
        let event = build_market_event(
            EventType::Policy,
            SourceKind::Policy,
            "TEST_CODE policy".into(),
            "TEST_CODE subject".into(),
            "TEST_CODE object".into(),
            Direction::Bull,
        );
        assert_eq!(event.event_type, EventType::Policy);
        assert_eq!(event.provenance[0].provider, "policy");
    }
}
