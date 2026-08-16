//! BR-174/BR-176 source-bound global-news acquisition.
//!
//! This module deliberately stops before notification projection and simhash
//! mutation. A caller must durably receipt the returned raw batch before it can
//! construct the compatibility `MarketEvent` view.

use crate::data_gateway::{
    BatchEvidence, GatewayBatch, GatewayError, GlobalNewsGateway, GlobalNewsProvider,
    GlobalNewsRecord,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::future::join_all;
use std::fmt;

pub const REGISTERED_GLOBAL_NEWS_LIMIT: u32 = 20;
pub const MAGIC_MARKET_DATA_REVISION: &str = "5f1ce93656a55854c844065390520cd4aecd9a14";

const REGISTERED_PROVIDERS: [GlobalNewsProvider; 4] = [
    GlobalNewsProvider::Eastmoney,
    GlobalNewsProvider::Cailianpress,
    GlobalNewsProvider::Jin10,
    GlobalNewsProvider::ThePaper,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegisteredGlobalNewsFeed {
    pub provider: GlobalNewsProvider,
    pub feed_name: &'static str,
    pub gateway_provider: &'static str,
    pub provider_id: &'static str,
    pub source_contract: &'static str,
    pub capability_name: &'static str,
    pub max_limit: u32,
    pub upstream_revision: &'static str,
}

impl RegisteredGlobalNewsFeed {
    pub const fn for_provider(provider: GlobalNewsProvider) -> Self {
        match provider {
            GlobalNewsProvider::Eastmoney => Self {
                provider,
                feed_name: "eastmoney_global_news",
                gateway_provider: "eastmoney",
                provider_id: "eastmoney",
                source_contract: "eastmoney-web",
                capability_name: "GlobalNews-Eastmoney",
                max_limit: REGISTERED_GLOBAL_NEWS_LIMIT,
                upstream_revision: MAGIC_MARKET_DATA_REVISION,
            },
            GlobalNewsProvider::Cailianpress => Self {
                provider,
                feed_name: "cls_global_news",
                gateway_provider: "cailianpress",
                provider_id: "cailianpress",
                source_contract: "cls-v1",
                capability_name: "GlobalNews-CLS",
                max_limit: REGISTERED_GLOBAL_NEWS_LIMIT,
                upstream_revision: MAGIC_MARKET_DATA_REVISION,
            },
            GlobalNewsProvider::Jin10 => Self {
                provider,
                feed_name: "jin10_global_news",
                gateway_provider: "jin10",
                provider_id: "jin10",
                source_contract: "jin10-flash-v1",
                capability_name: "GlobalNews-Jin10",
                max_limit: REGISTERED_GLOBAL_NEWS_LIMIT,
                upstream_revision: MAGIC_MARKET_DATA_REVISION,
            },
            GlobalNewsProvider::ThePaper => Self {
                provider,
                feed_name: "thepaper_global_news",
                gateway_provider: "thepaper",
                provider_id: "thepaper",
                source_contract: "thepaper-finance-v1",
                capability_name: "GlobalNews-ThePaper",
                max_limit: REGISTERED_GLOBAL_NEWS_LIMIT,
                upstream_revision: MAGIC_MARKET_DATA_REVISION,
            },
        }
    }
}

pub fn registered_global_news_feeds() -> [RegisteredGlobalNewsFeed; 4] {
    REGISTERED_PROVIDERS.map(RegisteredGlobalNewsFeed::for_provider)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedUnavailable {
    failed_stage: &'static str,
    diagnostic_code: &'static str,
    reason_code: &'static str,
    retryable: bool,
    available_evidence: Option<BatchEvidence>,
}

impl FeedUnavailable {
    pub fn failed_stage(&self) -> &'static str {
        self.failed_stage
    }

    pub fn diagnostic_code(&self) -> &'static str {
        self.diagnostic_code
    }

    pub fn reason_code(&self) -> &'static str {
        self.reason_code
    }

    pub const fn retryable(&self) -> bool {
        self.retryable
    }

    pub fn available_evidence(&self) -> Option<&BatchEvidence> {
        self.available_evidence.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RawGlobalNewsTerminal {
    Available {
        records: Vec<GlobalNewsRecord>,
        evidence: BatchEvidence,
    },
    VerifiedEmpty {
        evidence: BatchEvidence,
    },
    Unavailable(FeedUnavailable),
}

impl RawGlobalNewsTerminal {
    const fn is_complete(&self) -> bool {
        matches!(self, Self::Available { .. } | Self::VerifiedEmpty { .. })
    }

    fn evidence(&self) -> Option<&BatchEvidence> {
        match self {
            Self::Available { evidence, .. } | Self::VerifiedEmpty { evidence } => Some(evidence),
            Self::Unavailable(unavailable) => unavailable.available_evidence.as_ref(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawGlobalNewsTerminalKind {
    Available,
    VerifiedEmpty,
    Unavailable,
}

/// Read-only view over a provider terminal owned by an admitted raw batch.
///
/// The underlying terminal enum stays private to this module, so constructing
/// this view cannot mint a raw acquisition capability.
#[derive(Debug, Clone, Copy)]
pub struct RawGlobalNewsTerminalRef<'a> {
    terminal: &'a RawGlobalNewsTerminal,
}

impl<'a> RawGlobalNewsTerminalRef<'a> {
    pub const fn kind(&self) -> RawGlobalNewsTerminalKind {
        match self.terminal {
            RawGlobalNewsTerminal::Available { .. } => RawGlobalNewsTerminalKind::Available,
            RawGlobalNewsTerminal::VerifiedEmpty { .. } => RawGlobalNewsTerminalKind::VerifiedEmpty,
            RawGlobalNewsTerminal::Unavailable(_) => RawGlobalNewsTerminalKind::Unavailable,
        }
    }

    pub const fn is_complete(&self) -> bool {
        self.terminal.is_complete()
    }

    pub fn evidence(&self) -> Option<&'a BatchEvidence> {
        self.terminal.evidence()
    }

    pub fn records(&self) -> Option<&'a [GlobalNewsRecord]> {
        match self.terminal {
            RawGlobalNewsTerminal::Available { records, .. } => Some(records),
            RawGlobalNewsTerminal::VerifiedEmpty { .. } | RawGlobalNewsTerminal::Unavailable(_) => {
                None
            }
        }
    }

    pub fn unavailable(&self) -> Option<&'a FeedUnavailable> {
        match self.terminal {
            RawGlobalNewsTerminal::Unavailable(unavailable) => Some(unavailable),
            RawGlobalNewsTerminal::Available { .. }
            | RawGlobalNewsTerminal::VerifiedEmpty { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawGlobalNewsFeedAttempt {
    registration: RegisteredGlobalNewsFeed,
    attempted_at: DateTime<Utc>,
    terminal: RawGlobalNewsTerminal,
}

impl RawGlobalNewsFeedAttempt {
    pub const fn registration(&self) -> RegisteredGlobalNewsFeed {
        self.registration
    }

    pub const fn attempted_at(&self) -> DateTime<Utc> {
        self.attempted_at
    }

    pub const fn terminal(&self) -> RawGlobalNewsTerminalRef<'_> {
        RawGlobalNewsTerminalRef {
            terminal: &self.terminal,
        }
    }
}

/// Opaque, source-admitted result of one real global-news acquisition.
///
/// Its attempts and terminal states are intentionally readable only through
/// borrowed accessors. Production callers cannot construct a batch literal:
///
/// ```compile_fail
/// use chrono::Utc;
/// use stock_analysis::news::aggregator::raw_v2::RawNewsAggregationBatch;
///
/// let _forged = RawNewsAggregationBatch {
///     attempts: Vec::new(),
///     observed_at: Utc::now(),
/// };
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawNewsAggregationBatch {
    attempts: Vec<RawGlobalNewsFeedAttempt>,
    observed_at: DateTime<Utc>,
}

impl RawNewsAggregationBatch {
    pub fn attempts(&self) -> &[RawGlobalNewsFeedAttempt] {
        &self.attempts
    }

    pub const fn observed_at(&self) -> DateTime<Utc> {
        self.observed_at
    }

    pub fn sources_complete(&self) -> bool {
        self.attempts.len() == REGISTERED_PROVIDERS.len()
            && self
                .attempts
                .iter()
                .all(|attempt| attempt.terminal().is_complete())
    }

    pub fn source_record_count(&self) -> usize {
        self.attempts
            .iter()
            .map(|attempt| attempt.terminal().records().map_or(0, <[_]>::len))
            .sum()
    }
}

#[cfg(test)]
#[derive(Debug, Clone)]
pub(crate) enum TestRawGlobalNewsTerminal {
    Available {
        records: Vec<GlobalNewsRecord>,
        evidence: BatchEvidence,
    },
    VerifiedEmpty {
        evidence: BatchEvidence,
    },
    Unavailable {
        failed_stage: &'static str,
        diagnostic_code: &'static str,
        reason_code: &'static str,
        retryable: bool,
        available_evidence: Option<BatchEvidence>,
    },
}

#[cfg(test)]
impl RawGlobalNewsFeedAttempt {
    pub(crate) fn test_fixture(
        test_identity: &str,
        registration: RegisteredGlobalNewsFeed,
        attempted_at: DateTime<Utc>,
        terminal: TestRawGlobalNewsTerminal,
    ) -> Self {
        require_test_identity(test_identity);
        let terminal = match terminal {
            TestRawGlobalNewsTerminal::Available { records, evidence } => {
                assert!(
                    !records.is_empty(),
                    "TEST_CODE Available fixture requires records"
                );
                assert_test_evidence(&evidence);
                for record in &records {
                    require_test_identity(&record.item_id);
                    require_test_identity(record.evidence.batch_id());
                }
                RawGlobalNewsTerminal::Available { records, evidence }
            }
            TestRawGlobalNewsTerminal::VerifiedEmpty { evidence } => {
                assert_test_evidence(&evidence);
                RawGlobalNewsTerminal::VerifiedEmpty { evidence }
            }
            TestRawGlobalNewsTerminal::Unavailable {
                failed_stage,
                diagnostic_code,
                reason_code,
                retryable,
                available_evidence,
            } => {
                if let Some(evidence) = &available_evidence {
                    assert_test_evidence(evidence);
                }
                RawGlobalNewsTerminal::Unavailable(FeedUnavailable {
                    failed_stage,
                    diagnostic_code,
                    reason_code,
                    retryable,
                    available_evidence,
                })
            }
        };
        Self {
            registration,
            attempted_at,
            terminal,
        }
    }
}

#[cfg(test)]
impl RawNewsAggregationBatch {
    pub(crate) fn test_fixture(
        test_identity: &str,
        attempts: Vec<RawGlobalNewsFeedAttempt>,
        observed_at: DateTime<Utc>,
    ) -> Self {
        require_test_identity(test_identity);
        assert!(
            attempts.len() == REGISTERED_PROVIDERS.len(),
            "TEST_CODE raw batch fixture requires every registered provider"
        );
        Self {
            attempts,
            observed_at,
        }
    }
}

#[cfg(test)]
fn require_test_identity(value: &str) {
    assert!(
        value.starts_with("TEST_CODE"),
        "test fixture identity must start with TEST_CODE"
    );
}

#[cfg(test)]
fn assert_test_evidence(evidence: &BatchEvidence) {
    require_test_identity(&evidence.batch_id);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawNewsAcquisitionError {
    InvalidLimit(u32),
}

impl fmt::Display for RawNewsAcquisitionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLimit(limit) => write!(
                formatter,
                "BR-174 raw global-news limit must be within 1..={REGISTERED_GLOBAL_NEWS_LIMIT}, got {limit}"
            ),
        }
    }
}

impl std::error::Error for RawNewsAcquisitionError {}

#[async_trait]
trait RawGlobalNewsPort: Sync {
    async fn fetch(
        &self,
        provider: GlobalNewsProvider,
        limit: u32,
    ) -> Result<GatewayBatch<GlobalNewsRecord>, GatewayError>;
}

#[derive(Debug, Clone, Copy, Default)]
struct ProductionRawGlobalNewsPort;

#[async_trait]
impl RawGlobalNewsPort for ProductionRawGlobalNewsPort {
    async fn fetch(
        &self,
        provider: GlobalNewsProvider,
        limit: u32,
    ) -> Result<GatewayBatch<GlobalNewsRecord>, GatewayError> {
        GlobalNewsGateway::new().global_news(provider, limit).await
    }
}

pub async fn fetch_raw_global_news_batch(
    per_feed_limit: u32,
) -> Result<RawNewsAggregationBatch, RawNewsAcquisitionError> {
    fetch_raw_global_news_batch_with(&ProductionRawGlobalNewsPort, per_feed_limit).await
}

async fn fetch_raw_global_news_batch_with(
    port: &impl RawGlobalNewsPort,
    per_feed_limit: u32,
) -> Result<RawNewsAggregationBatch, RawNewsAcquisitionError> {
    if !(1..=REGISTERED_GLOBAL_NEWS_LIMIT).contains(&per_feed_limit) {
        return Err(RawNewsAcquisitionError::InvalidLimit(per_feed_limit));
    }

    let futures = REGISTERED_PROVIDERS
        .into_iter()
        .map(|provider| fetch_registered_feed(port, provider, per_feed_limit));
    let attempts = join_all(futures).await;
    Ok(RawNewsAggregationBatch {
        attempts,
        observed_at: Utc::now(),
    })
}

async fn fetch_registered_feed(
    port: &impl RawGlobalNewsPort,
    provider: GlobalNewsProvider,
    limit: u32,
) -> RawGlobalNewsFeedAttempt {
    let registration = RegisteredGlobalNewsFeed::for_provider(provider);
    let attempted_at = Utc::now();
    let terminal = match port.fetch(provider, limit).await {
        Ok(GatewayBatch::Available { records, evidence }) if records.is_empty() => {
            RawGlobalNewsTerminal::Unavailable(FeedUnavailable {
                failed_stage: "global_news_gateway_admission",
                diagnostic_code: "available_batch_empty",
                reason_code: "invalid_evidence",
                retryable: false,
                available_evidence: Some(evidence),
            })
        }
        Ok(GatewayBatch::Available { records, evidence }) => {
            RawGlobalNewsTerminal::Available { records, evidence }
        }
        Ok(GatewayBatch::VerifiedEmpty(evidence)) => {
            RawGlobalNewsTerminal::VerifiedEmpty { evidence }
        }
        Err(error) => {
            let (diagnostic_code, reason_code, retryable) = classify_gateway_error(&error);
            RawGlobalNewsTerminal::Unavailable(FeedUnavailable {
                failed_stage: "global_news_gateway",
                diagnostic_code,
                reason_code,
                retryable,
                available_evidence: None,
            })
        }
    };
    RawGlobalNewsFeedAttempt {
        registration,
        attempted_at,
        terminal,
    }
}

fn classify_gateway_error(error: &GatewayError) -> (&'static str, &'static str, bool) {
    match error.reason_code() {
        "no_verified_batch" => (
            "provider_batch_unavailable",
            "no_verified_batch",
            error.retryable(),
        ),
        "invalid_evidence" => (
            "provider_evidence_invalid",
            "invalid_evidence",
            error.retryable(),
        ),
        "invalid_request" => (
            "provider_request_invalid",
            "invalid_request",
            error.retryable(),
        ),
        "acquisition_audit_unavailable" => (
            "provider_audit_unavailable",
            "acquisition_audit_unavailable",
            error.retryable(),
        ),
        _ => (
            "provider_error_mapping_missing",
            "provider_error_mapping_missing",
            false,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::magic_compat::{ProviderId, SourceEvidence};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TypedFixturePort {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl RawGlobalNewsPort for TypedFixturePort {
        async fn fetch(
            &self,
            provider: GlobalNewsProvider,
            _limit: u32,
        ) -> Result<GatewayBatch<GlobalNewsRecord>, GatewayError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let evidence = evidence(provider, provider.feed_name());
            match provider {
                GlobalNewsProvider::Eastmoney => Ok(GatewayBatch::Available {
                    records: vec![record(provider)],
                    evidence,
                }),
                GlobalNewsProvider::Cailianpress => Ok(GatewayBatch::VerifiedEmpty(evidence)),
                GlobalNewsProvider::Jin10 => Err(GatewayError::unavailable(
                    "GlobalNews-Jin10",
                    Some(ProviderId::Jin10),
                    true,
                    "TEST_CODE secret transport diagnostic",
                )),
                GlobalNewsProvider::ThePaper => Ok(GatewayBatch::Available {
                    records: Vec::new(),
                    evidence,
                }),
            }
        }
    }

    fn evidence(provider: GlobalNewsProvider, batch_id: &str) -> BatchEvidence {
        BatchEvidence {
            provider: provider.provider_id(),
            source: provider.source().to_owned(),
            source_at: Some("2026-07-28T01:00:00.000000000Z".to_owned()),
            observed_at: "2026-07-28T01:00:01.000000000Z".to_owned(),
            batch_id: format!("TEST_CODE_{batch_id}"),
        }
    }

    fn record(provider: GlobalNewsProvider) -> GlobalNewsRecord {
        let published_at = DateTime::parse_from_rfc3339("2026-07-28T09:00:00+08:00")
            .expect("TEST_CODE publication")
            .with_timezone(&Utc);
        let observed_at = DateTime::parse_from_rfc3339("2026-07-28T09:00:01+08:00")
            .expect("TEST_CODE observation")
            .with_timezone(&Utc);
        GlobalNewsRecord {
            item_id: "TEST_CODE_item".to_owned(),
            title: "TEST_CODE title".to_owned(),
            summary: Some("TEST_CODE summary".to_owned()),
            content: None,
            publisher: "TEST_CODE publisher".to_owned(),
            canonical_url: "https://example.com/TEST_CODE_item".to_owned(),
            published_at,
            observed_at,
            instruments: vec!["TEST_CODE_600000".to_owned()],
            topics: vec!["TEST_CODE topic".to_owned()],
            language: "zh-CN".to_owned(),
            evidence: SourceEvidence::new(
                provider.provider_id(),
                "2026-07-28T01:00:00.000000000Z",
                "TEST_CODE_record_batch",
            )
            .expect("TEST_CODE source evidence"),
        }
    }

    #[tokio::test]
    async fn retains_all_registered_terminal_states_in_registry_order() {
        let port = TypedFixturePort {
            calls: AtomicUsize::new(0),
        };
        let batch = fetch_raw_global_news_batch_with(&port, 20)
            .await
            .expect("TEST_CODE raw batch");

        assert_eq!(port.calls.load(Ordering::SeqCst), 4);
        assert_eq!(batch.attempts().len(), 4);
        assert_eq!(
            batch
                .attempts()
                .iter()
                .map(|attempt| attempt.registration().feed_name)
                .collect::<Vec<_>>(),
            vec![
                "eastmoney_global_news",
                "cls_global_news",
                "jin10_global_news",
                "thepaper_global_news",
            ]
        );
        assert_eq!(
            batch.attempts()[0].terminal().kind(),
            RawGlobalNewsTerminalKind::Available
        );
        assert_eq!(
            batch.attempts()[1].terminal().kind(),
            RawGlobalNewsTerminalKind::VerifiedEmpty
        );
        let jin10 = batch.attempts()[2]
            .terminal()
            .unavailable()
            .expect("TEST_CODE Jin10 unavailable");
        assert_eq!(jin10.reason_code(), "no_verified_batch");
        let thepaper = batch.attempts()[3]
            .terminal()
            .unavailable()
            .expect("TEST_CODE ThePaper unavailable");
        assert_eq!(thepaper.diagnostic_code(), "available_batch_empty");
        assert!(!thepaper.retryable());
        assert_eq!(batch.source_record_count(), 1);
        assert!(!batch.sources_complete());
        assert!(
            !format!("{batch:?}").contains("secret transport diagnostic"),
            "raw provider message must not cross the typed terminal boundary"
        );
    }

    #[tokio::test]
    async fn invalid_limit_fails_before_any_provider_call() {
        let port = TypedFixturePort {
            calls: AtomicUsize::new(0),
        };
        let error = fetch_raw_global_news_batch_with(&port, 21)
            .await
            .expect_err("TEST_CODE invalid limit");
        assert_eq!(error, RawNewsAcquisitionError::InvalidLimit(21));
        assert_eq!(port.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn registry_is_exact_and_pinned() {
        let feeds = registered_global_news_feeds();
        assert_eq!(feeds.len(), 4);
        assert!(feeds.iter().all(|feed| {
            feed.max_limit == 20 && feed.upstream_revision == MAGIC_MARKET_DATA_REVISION
        }));
    }

    #[test]
    fn unavailable_contract_uses_only_typed_provider_diagnostics() {
        let unavailable = FeedUnavailable {
            failed_stage: "TEST_CODE_stage",
            diagnostic_code: "provider_batch_unavailable",
            reason_code: "no_verified_batch",
            retryable: true,
            available_evidence: None,
        };
        assert_eq!(unavailable.diagnostic_code(), "provider_batch_unavailable");
    }

    #[test]
    #[should_panic(expected = "test fixture identity must start with TEST_CODE")]
    fn fixture_constructor_rejects_non_test_identity() {
        let _ = RawNewsAggregationBatch::test_fixture("production", Vec::new(), Utc::now());
    }
}
