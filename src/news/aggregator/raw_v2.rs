//! BR-174/BR-176 source-bound global-news acquisition.
//!
//! This module deliberately stops before selection notification projection and
//! simhash mutation. Selection callers must durably receipt the raw batch.
//! BR-244 exposes one narrower SourceOnly NewsFlash projection that consumes
//! the same opaque tick, never mints a receipt and never changes impact facts.

use crate::data_gateway::{
    BatchEvidence, GatewayBatch, GatewayError, GlobalNewsGateway, GlobalNewsProvider,
    GlobalNewsRecord,
};
use crate::magic_compat::ProviderId;
use crate::signal::market_event::MarketEvent;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::future::join_all;
use sha2::{Digest, Sha256};
use std::fmt;

pub const REGISTERED_GLOBAL_NEWS_LIMIT: u32 = 20;
pub const MAGIC_MARKET_DATA_REVISION: &str = "75ee2a2bdd3b1ca2b01ce3afbb04aec416e7000e";

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
                source_contract: provider.source(),
                capability_name: "GlobalNews-Eastmoney",
                max_limit: REGISTERED_GLOBAL_NEWS_LIMIT,
                upstream_revision: MAGIC_MARKET_DATA_REVISION,
            },
            GlobalNewsProvider::Cailianpress => Self {
                provider,
                feed_name: "cls_global_news",
                gateway_provider: "cailianpress",
                provider_id: "cailianpress",
                source_contract: provider.source(),
                capability_name: "GlobalNews-CLS",
                max_limit: REGISTERED_GLOBAL_NEWS_LIMIT,
                upstream_revision: MAGIC_MARKET_DATA_REVISION,
            },
            GlobalNewsProvider::Jin10 => Self {
                provider,
                feed_name: "jin10_global_news",
                gateway_provider: "jin10",
                provider_id: "jin10",
                source_contract: provider.source(),
                capability_name: "GlobalNews-Jin10",
                max_limit: REGISTERED_GLOBAL_NEWS_LIMIT,
                upstream_revision: MAGIC_MARKET_DATA_REVISION,
            },
            GlobalNewsProvider::ThePaper => Self {
                provider,
                feed_name: "thepaper_global_news",
                gateway_provider: "thepaper",
                provider_id: "thepaper",
                source_contract: provider.source(),
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
    source_record_count: usize,
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

    pub const fn source_record_count(&self) -> usize {
        self.source_record_count
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

/// One provider-local reason why an admitted raw terminal could not contribute
/// to the public SourceOnly NewsFlash view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewsFlashSourceFailure {
    provider: GlobalNewsProvider,
    available_provider: Option<ProviderId>,
    failed_stage: &'static str,
    diagnostic_code: &'static str,
    reason_code: &'static str,
    retryable: bool,
    observed_at: DateTime<Utc>,
    source_record_count: usize,
    diagnostic: String,
    batch_id: Option<String>,
    record_id: Option<String>,
}

impl NewsFlashSourceFailure {
    pub const fn provider(&self) -> GlobalNewsProvider {
        self.provider
    }

    pub const fn available_provider(&self) -> Option<ProviderId> {
        self.available_provider
    }

    pub fn available_provider_wire(&self) -> Option<&'static str> {
        self.available_provider.map(provider_id_wire_name)
    }

    pub const fn failed_stage(&self) -> &'static str {
        self.failed_stage
    }

    pub const fn diagnostic_code(&self) -> &'static str {
        self.diagnostic_code
    }

    pub const fn reason_code(&self) -> &'static str {
        self.reason_code
    }

    pub const fn retryable(&self) -> bool {
        self.retryable
    }

    pub const fn observed_at(&self) -> DateTime<Utc> {
        self.observed_at
    }

    pub const fn source_record_count(&self) -> usize {
        self.source_record_count
    }

    pub fn diagnostic(&self) -> &str {
        &self.diagnostic
    }

    pub fn batch_id(&self) -> Option<&str> {
        self.batch_id.as_deref()
    }

    pub fn record_id(&self) -> Option<&str> {
        self.record_id.as_deref()
    }

    pub fn identity_sha256(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(crate::event::envelope::NEWS_FLASH_FAILURE_IDENTITY_HASH_DOMAIN.as_bytes());
        let provider = self.provider.wire_name();
        let available_provider = self.available_provider_wire().unwrap_or("<absent>");
        let observed_at = self.observed_at.to_rfc3339();
        let source_record_count = self.source_record_count.to_string();
        for value in [
            provider,
            available_provider,
            self.failed_stage,
            self.reason_code,
            self.diagnostic_code,
            self.diagnostic.as_str(),
            if self.retryable { "true" } else { "false" },
            observed_at.as_str(),
            source_record_count.as_str(),
            self.batch_id.as_deref().unwrap_or("<absent>"),
            self.record_id.as_deref().unwrap_or("<absent>"),
        ] {
            hasher.update((value.len() as u64).to_be_bytes());
            hasher.update(value.as_bytes());
        }
        format!("{:x}", hasher.finalize())
    }

    pub fn audit_message(&self) -> String {
        format!(
            "BR-244 provider={} available_provider={} stage={} diagnostic_code={} diagnostic={} reason={} retryable={} records={} batch_id={} record_id={}",
            self.provider.wire_name(),
            self.available_provider_wire().unwrap_or("<absent>"),
            self.failed_stage,
            self.diagnostic_code,
            self.diagnostic,
            self.reason_code,
            self.retryable,
            self.source_record_count,
            self.batch_id.as_deref().unwrap_or("<absent>"),
            self.record_id.as_deref().unwrap_or("<absent>"),
        )
    }
}

const NEWS_FLASH_DIAGNOSTIC_MAX_BYTES: usize = 512;
const NEWS_FLASH_DIAGNOSTIC_TRUNCATED_SUFFIX: &str = " [truncated]";

fn bounded_news_flash_diagnostic(value: &str) -> String {
    if value.len() <= NEWS_FLASH_DIAGNOSTIC_MAX_BYTES {
        return value.to_owned();
    }
    let mut end = NEWS_FLASH_DIAGNOSTIC_MAX_BYTES
        .saturating_sub(NEWS_FLASH_DIAGNOSTIC_TRUNCATED_SUFFIX.len());
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    format!(
        "{}{}",
        &value[..end],
        NEWS_FLASH_DIAGNOSTIC_TRUNCATED_SUFFIX
    )
}

/// Canonical source identity carried with a public SourceOnly NewsFlash event.
/// Fields are private so callers cannot substitute evidence after admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewsFlashSourceIdentity {
    event_id: String,
    provider: String,
    source: String,
    published_at: DateTime<Utc>,
    observed_at: DateTime<Utc>,
    batch_id: String,
}

impl NewsFlashSourceIdentity {
    pub fn event_id(&self) -> &str {
        &self.event_id
    }

    pub fn provider(&self) -> &str {
        &self.provider
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub const fn published_at(&self) -> DateTime<Utc> {
        self.published_at
    }

    pub const fn observed_at(&self) -> DateTime<Utc> {
        self.observed_at
    }

    pub fn batch_id(&self) -> &str {
        &self.batch_id
    }

    fn update_canonical_hasher(&self, hasher: &mut Sha256) {
        let published_at = self.published_at.to_rfc3339();
        let observed_at = self.observed_at.to_rfc3339();
        for value in [
            self.event_id.as_str(),
            self.provider.as_str(),
            self.source.as_str(),
            published_at.as_str(),
            observed_at.as_str(),
            self.batch_id.as_str(),
        ] {
            hasher.update((value.len() as u64).to_be_bytes());
            hasher.update(value.as_bytes());
        }
    }
}

/// Evidence and semantic projection remain inseparable until NewsFlash audit.
#[derive(Debug, Clone)]
pub struct NewsFlashProjectedEvent {
    event: MarketEvent,
    source: NewsFlashSourceIdentity,
}

/// Opaque capability for constructing source-bound projection fixtures from
/// monitor-binary tests, where this library is compiled without `cfg(test)`.
/// Production processes cannot mint the capability.
#[derive(Debug)]
pub struct NewsFlashProjectionTestCapability {
    _private: (),
}

impl NewsFlashProjectionTestCapability {
    pub fn bind() -> Result<Self, &'static str> {
        if crate::risk::env_guard::runtime_is_test_process()
            && crate::risk::env_guard::current_env() == crate::risk::env_guard::TradingEnv::Test
        {
            Ok(Self { _private: () })
        } else {
            Err("NewsFlash projection test capability requires a test process and namespace")
        }
    }
}

impl NewsFlashProjectedEvent {
    pub fn event(&self) -> &MarketEvent {
        &self.event
    }

    pub fn source(&self) -> &NewsFlashSourceIdentity {
        &self.source
    }

    #[doc(hidden)]
    pub fn test_fixture(
        _capability: &NewsFlashProjectionTestCapability,
        event: MarketEvent,
        provider: &str,
        source: &str,
        published_at: DateTime<Utc>,
        observed_at: DateTime<Utc>,
        batch_id: &str,
    ) -> Self {
        assert!(
            event.event_id.is_empty() || event.event_id.starts_with("TEST_CODE"),
            "test fixture identity must start with TEST_CODE"
        );
        assert!(
            batch_id.starts_with("TEST_CODE"),
            "test fixture identity must start with TEST_CODE"
        );
        assert!(
            provider.starts_with("TEST_CODE") && source.starts_with("TEST_CODE"),
            "test fixture provider and source must start with TEST_CODE"
        );
        Self {
            source: NewsFlashSourceIdentity {
                event_id: event.event_id.clone(),
                provider: provider.to_owned(),
                source: source.to_owned(),
                published_at,
                observed_at,
                batch_id: batch_id.to_owned(),
            },
            event,
        }
    }
}

/// Hash the exact evidence sequence in display order.
pub fn ordered_news_flash_evidence_sha256(events: &[NewsFlashProjectedEvent]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"stock_analysis.news_flash_ordered_source_evidence.v1");
    hasher.update((events.len() as u64).to_be_bytes());
    for projected in events {
        projected.source.update_canonical_hasher(&mut hasher);
    }
    format!("{:x}", hasher.finalize())
}

fn news_flash_identity_allowed_for_env(
    env: crate::risk::env_guard::TradingEnv,
    values: &[&str],
) -> bool {
    env != crate::risk::env_guard::TradingEnv::Prod
        || values
            .iter()
            .all(|value| !crate::risk::env_guard::is_test_code(value))
}

const fn provider_id_wire_name(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Tdx => "Tdx",
        ProviderId::Tencent => "Tencent",
        ProviderId::Eastmoney => "Eastmoney",
        ProviderId::Sina => "Sina",
        ProviderId::Baostock => "Baostock",
        ProviderId::Baidu => "Baidu",
        ProviderId::Tonghuashun => "Tonghuashun",
        ProviderId::Iwencai => "Iwencai",
        ProviderId::Cninfo => "Cninfo",
        ProviderId::Cailianpress => "Cailianpress",
        ProviderId::Jin10 => "Jin10",
        ProviderId::ThePaper => "ThePaper",
        ProviderId::Yonhap => "Yonhap",
        ProviderId::WallstreetCn => "WallstreetCn",
        ProviderId::Sse => "Sse",
        ProviderId::Szse => "Szse",
        ProviderId::Hkex => "Hkex",
        ProviderId::Cffex => "Cffex",
        ProviderId::StateCouncil => "StateCouncil",
        ProviderId::Nbs => "Nbs",
        ProviderId::Pbc => "Pbc",
        ProviderId::Cfets => "Cfets",
        ProviderId::Fred => "Fred",
        ProviderId::Imf => "Imf",
        ProviderId::WorldBank => "WorldBank",
        ProviderId::SecEdgar => "SecEdgar",
        ProviderId::XinhuaFinance => "XinhuaFinance",
        ProviderId::Yicai => "Yicai",
        ProviderId::SecuritiesTimes => "SecuritiesTimes",
        ProviderId::LocalAnalysis => "LocalAnalysis",
        ProviderId::LocalTerminal => "LocalTerminal",
        ProviderId::Custom => "Custom",
    }
}

fn validate_news_flash_record(
    record: &GlobalNewsRecord,
    evidence: &BatchEvidence,
    env: crate::risk::env_guard::TradingEnv,
) -> Result<(), &'static str> {
    let source_time_matches = record
        .evidence
        .source_at()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .is_some_and(|value| value.with_timezone(&Utc) == record.published_at);
    let observation_time_matches = DateTime::parse_from_rfc3339(record.evidence.observed_at())
        .is_ok_and(|value| value.with_timezone(&Utc) == record.observed_at);
    if record.item_id.trim().is_empty() {
        Err("missing_record_id")
    } else if record.title.trim().is_empty() {
        Err("missing_title")
    } else if record.publisher.trim().is_empty() {
        Err("missing_publisher")
    } else if record.canonical_url.trim().is_empty() {
        Err("missing_canonical_url")
    } else if record.evidence.provider() != evidence.provider {
        Err("record_provider_mismatch")
    } else if record.evidence.batch_id() != evidence.batch_id {
        Err("record_batch_mismatch")
    } else if record.evidence.observed_at() != evidence.observed_at {
        Err("record_observed_at_mismatch")
    } else if !source_time_matches {
        Err("record_published_at_mismatch")
    } else if !observation_time_matches {
        Err("record_observation_time_mismatch")
    } else if !news_flash_identity_allowed_for_env(
        env,
        &[
            record.item_id.as_str(),
            evidence.source.as_str(),
            record.evidence.batch_id(),
        ],
    ) {
        Err("test_identity_rejected_in_production")
    } else {
        Ok(())
    }
}

/// Evidence-preserving SourceOnly notification view of one opaque raw tick.
///
/// The raw batch remains the source authority. This view projects only its
/// `Available` terminals, retains every provider-local failure for dispatcher
/// audit, and deliberately carries no selection-ingress capability.
#[derive(Debug, Clone, Default)]
pub struct NewsFlashSourceProjection {
    events: Vec<NewsFlashProjectedEvent>,
    failures: Vec<NewsFlashSourceFailure>,
    available_feed_count: usize,
    verified_empty_feed_count: usize,
}

impl NewsFlashSourceProjection {
    pub fn events(&self) -> &[NewsFlashProjectedEvent] {
        &self.events
    }

    pub fn failures(&self) -> &[NewsFlashSourceFailure] {
        &self.failures
    }

    pub const fn available_feed_count(&self) -> usize {
        self.available_feed_count
    }

    pub const fn verified_empty_feed_count(&self) -> usize {
        self.verified_empty_feed_count
    }

    pub fn into_parts(self) -> (Vec<NewsFlashProjectedEvent>, Vec<NewsFlashSourceFailure>) {
        (self.events, self.failures)
    }
}

/// BR-244 public SourceOnly projection from the same-tick opaque raw batch.
///
/// This is intentionally separate from BR-174 selection ingress: it neither
/// mints a receipt nor advances the selection notification simhash. Projection
/// is provider-atomic, so one malformed provider produces an explicit failure
/// without discarding events from the other admitted providers.
pub fn project_news_flash_events(batch: &RawNewsAggregationBatch) -> NewsFlashSourceProjection {
    let mut projection = NewsFlashSourceProjection::default();
    for attempt in batch.attempts() {
        let registration = attempt.registration();
        let terminal = attempt.terminal();
        match terminal.kind() {
            RawGlobalNewsTerminalKind::Available => {
                let Some(records) = terminal.records() else {
                    unreachable!("Available raw terminal always owns records")
                };
                let Some(evidence) = terminal.evidence() else {
                    unreachable!("Available raw terminal always owns batch evidence")
                };
                let batch_times_valid = evidence
                    .source_at
                    .as_deref()
                    .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                    .zip(DateTime::parse_from_rfc3339(&evidence.observed_at).ok())
                    .is_some_and(|(source_at, observed_at)| source_at <= observed_at);
                if registration.source_contract != registration.provider.source()
                    || evidence.provider != registration.provider.provider_id()
                    || evidence.source != registration.source_contract
                    || evidence
                        .source_at
                        .as_deref()
                        .is_none_or(|value| value.trim().is_empty())
                    || evidence.observed_at.trim().is_empty()
                    || evidence.batch_id.trim().is_empty()
                    || !batch_times_valid
                    || !news_flash_identity_allowed_for_env(
                        crate::risk::env_guard::current_env(),
                        &[
                            registration.provider.wire_name(),
                            evidence.source.as_str(),
                            evidence.batch_id.as_str(),
                        ],
                    )
                {
                    projection.failures.push(NewsFlashSourceFailure {
                        provider: registration.provider,
                        available_provider: Some(evidence.provider),
                        failed_stage: "news_flash_source_projection",
                        diagnostic_code: "batch_evidence_incomplete",
                        reason_code: "invalid_evidence",
                        retryable: false,
                        observed_at: batch.observed_at(),
                        source_record_count: records.len(),
                        diagnostic:
                            "batch evidence does not match the registered provider/source contract"
                                .to_owned(),
                        batch_id: (!evidence.batch_id.trim().is_empty())
                            .then(|| evidence.batch_id.clone()),
                        record_id: None,
                    });
                    continue;
                }
                if let Some((record, diagnostic)) = records.iter().find_map(|record| {
                    validate_news_flash_record(
                        record,
                        evidence,
                        crate::risk::env_guard::current_env(),
                    )
                    .err()
                    .map(|diagnostic| (record, diagnostic))
                }) {
                    projection.failures.push(NewsFlashSourceFailure {
                        provider: registration.provider,
                        available_provider: Some(evidence.provider),
                        failed_stage: "news_flash_source_projection",
                        diagnostic_code: "record_evidence_incomplete",
                        reason_code: "invalid_evidence",
                        retryable: false,
                        observed_at: batch.observed_at(),
                        source_record_count: records.len(),
                        diagnostic: diagnostic.to_owned(),
                        batch_id: Some(evidence.batch_id.clone()),
                        record_id: (!record.item_id.trim().is_empty())
                            .then(|| record.item_id.clone()),
                    });
                    continue;
                }

                let mut provider_events = Vec::with_capacity(records.len());
                let mut provider_failure = None;
                for record in records {
                    match super::feed::record_to_market_event(registration.provider, record) {
                        Ok(event) => provider_events.push(NewsFlashProjectedEvent {
                            source: NewsFlashSourceIdentity {
                                event_id: event.event_id.clone(),
                                provider: registration.provider.wire_name().to_owned(),
                                source: evidence.source.clone(),
                                published_at: record.published_at,
                                observed_at: record.observed_at,
                                batch_id: evidence.batch_id.clone(),
                            },
                            event,
                        }),
                        Err(error) => {
                            provider_failure = Some(NewsFlashSourceFailure {
                                provider: registration.provider,
                                available_provider: Some(evidence.provider),
                                failed_stage: "news_flash_source_projection",
                                diagnostic_code: "market_event_projection_failed",
                                reason_code: "invalid_evidence",
                                retryable: false,
                                observed_at: batch.observed_at(),
                                source_record_count: records.len(),
                                diagnostic: bounded_news_flash_diagnostic(&format!(
                                    "item_id={} error={error}",
                                    record.item_id
                                )),
                                batch_id: Some(evidence.batch_id.clone()),
                                record_id: Some(record.item_id.clone()),
                            });
                            break;
                        }
                    }
                }
                if let Some(failure) = provider_failure {
                    projection.failures.push(failure);
                } else {
                    projection.available_feed_count += 1;
                    projection.events.extend(provider_events);
                }
            }
            RawGlobalNewsTerminalKind::VerifiedEmpty => {
                projection.verified_empty_feed_count += 1;
            }
            RawGlobalNewsTerminalKind::Unavailable => {
                let unavailable = terminal
                    .unavailable()
                    .expect("Unavailable raw terminal owns its typed failure");
                projection.failures.push(NewsFlashSourceFailure {
                    provider: registration.provider,
                    available_provider: unavailable
                        .available_evidence()
                        .map(|evidence| evidence.provider),
                    failed_stage: unavailable.failed_stage(),
                    diagnostic_code: unavailable.diagnostic_code(),
                    reason_code: unavailable.reason_code(),
                    retryable: unavailable.retryable(),
                    observed_at: batch.observed_at(),
                    source_record_count: unavailable.source_record_count(),
                    diagnostic: unavailable.diagnostic_code().to_owned(),
                    batch_id: unavailable
                        .available_evidence()
                        .map(|evidence| evidence.batch_id.clone()),
                    record_id: None,
                });
            }
        }
    }
    projection
        .events
        .sort_by_key(|event| std::cmp::Reverse(event.event.occurred_at));
    projection
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
                    source_record_count: 0,
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
        Ok(batch)
            if batch.evidence().provider != registration.provider.provider_id()
                || batch.evidence().source != registration.source_contract =>
        {
            RawGlobalNewsTerminal::Unavailable(FeedUnavailable {
                failed_stage: "global_news_gateway_admission",
                diagnostic_code: "provider_evidence_mismatch",
                reason_code: "invalid_evidence",
                retryable: false,
                available_evidence: Some(batch.evidence().clone()),
                source_record_count: batch.records().len(),
            })
        }
        Ok(GatewayBatch::Available { records, evidence }) if records.len() > limit as usize => {
            RawGlobalNewsTerminal::Unavailable(FeedUnavailable {
                failed_stage: "global_news_gateway_admission",
                diagnostic_code: "provider_limit_exceeded",
                reason_code: "invalid_evidence",
                retryable: false,
                available_evidence: Some(evidence),
                source_record_count: records.len(),
            })
        }
        Ok(GatewayBatch::Available { records, evidence }) if records.is_empty() => {
            RawGlobalNewsTerminal::Unavailable(FeedUnavailable {
                failed_stage: "global_news_gateway_admission",
                diagnostic_code: "available_batch_empty",
                reason_code: "invalid_evidence",
                retryable: false,
                available_evidence: Some(evidence),
                source_record_count: 0,
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
                source_record_count: 0,
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

    struct WrongProviderFixturePort;

    struct EastmoneyRequestJin10EvidencePort;

    struct EastmoneyRequestSinaEvidencePort;

    struct OverLimitFixturePort;

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

    #[async_trait]
    impl RawGlobalNewsPort for WrongProviderFixturePort {
        async fn fetch(
            &self,
            provider: GlobalNewsProvider,
            _limit: u32,
        ) -> Result<GatewayBatch<GlobalNewsRecord>, GatewayError> {
            Ok(GatewayBatch::Available {
                records: vec![record(provider)],
                evidence: evidence(GlobalNewsProvider::Eastmoney, "wrong_provider_batch"),
            })
        }
    }

    #[async_trait]
    impl RawGlobalNewsPort for EastmoneyRequestJin10EvidencePort {
        async fn fetch(
            &self,
            provider: GlobalNewsProvider,
            _limit: u32,
        ) -> Result<GatewayBatch<GlobalNewsRecord>, GatewayError> {
            if provider == GlobalNewsProvider::Eastmoney {
                return Ok(GatewayBatch::Available {
                    records: vec![record(provider)],
                    evidence: evidence(
                        GlobalNewsProvider::Jin10,
                        "eastmoney_request_jin10_evidence",
                    ),
                });
            }
            Ok(GatewayBatch::VerifiedEmpty(evidence(
                provider,
                provider.feed_name(),
            )))
        }
    }

    #[async_trait]
    impl RawGlobalNewsPort for EastmoneyRequestSinaEvidencePort {
        async fn fetch(
            &self,
            provider: GlobalNewsProvider,
            _limit: u32,
        ) -> Result<GatewayBatch<GlobalNewsRecord>, GatewayError> {
            if provider == GlobalNewsProvider::Eastmoney {
                let mut available_evidence = evidence(provider, "eastmoney_request_sina_evidence");
                available_evidence.provider = ProviderId::Sina;
                available_evidence.source = "TEST_CODE_SINA_SOURCE".to_owned();
                return Ok(GatewayBatch::Available {
                    records: vec![record(provider)],
                    evidence: available_evidence,
                });
            }
            Ok(GatewayBatch::VerifiedEmpty(evidence(
                provider,
                provider.feed_name(),
            )))
        }
    }

    #[async_trait]
    impl RawGlobalNewsPort for OverLimitFixturePort {
        async fn fetch(
            &self,
            provider: GlobalNewsProvider,
            _limit: u32,
        ) -> Result<GatewayBatch<GlobalNewsRecord>, GatewayError> {
            Ok(GatewayBatch::Available {
                records: vec![record(provider), record(provider)],
                evidence: evidence(provider, provider.feed_name()),
            })
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
                "2026-07-28T01:00:01.000000000Z",
                format!("TEST_CODE_{}", provider.feed_name()),
            )
            .expect("TEST_CODE source evidence")
            .with_source_at("2026-07-28T01:00:00.000000000Z")
            .expect("TEST_CODE source publication evidence"),
        }
    }

    #[tokio::test]
    async fn br238_rejects_batch_evidence_from_a_different_registered_provider() {
        let batch = fetch_raw_global_news_batch_with(&WrongProviderFixturePort, 1)
            .await
            .expect("typed acquisition returns all terminal attempts");
        assert_eq!(
            batch
                .attempts()
                .iter()
                .filter(|attempt| attempt.terminal().kind() == RawGlobalNewsTerminalKind::Available)
                .count(),
            1,
            "only the actual Eastmoney batch may satisfy the Eastmoney registration"
        );
        for attempt in &batch.attempts()[1..] {
            let unavailable = attempt
                .terminal()
                .unavailable()
                .expect("mismatched provider evidence must fail closed");
            assert_eq!(unavailable.reason_code(), "invalid_evidence");
            assert!(!unavailable.retryable());
        }
    }

    #[tokio::test]
    async fn br244_failure_retains_requested_and_available_provider_wires() {
        let batch = fetch_raw_global_news_batch_with(&EastmoneyRequestJin10EvidencePort, 1)
            .await
            .expect("typed acquisition returns all terminal attempts");
        let projection = project_news_flash_events(&batch);
        let failure = projection
            .failures()
            .iter()
            .find(|failure| failure.provider() == GlobalNewsProvider::Eastmoney)
            .expect("Eastmoney request with Jin10 evidence must fail closed");

        assert_eq!(failure.available_provider(), Some(ProviderId::Jin10));
        assert!(failure.audit_message().contains("provider=Eastmoney"));
        assert!(failure.audit_message().contains("available_provider=Jin10"));
        assert_eq!(failure.diagnostic_code(), "provider_evidence_mismatch");
    }

    #[tokio::test]
    async fn br244_failure_retains_non_global_news_available_provider_wire() {
        let batch = fetch_raw_global_news_batch_with(&EastmoneyRequestSinaEvidencePort, 1)
            .await
            .expect("typed acquisition returns all terminal attempts");
        let projection = project_news_flash_events(&batch);
        let failure = projection
            .failures()
            .iter()
            .find(|failure| failure.provider() == GlobalNewsProvider::Eastmoney)
            .expect("Eastmoney request with Sina evidence must fail closed");

        assert_eq!(failure.available_provider(), Some(ProviderId::Sina));
        assert_eq!(failure.available_provider_wire(), Some("Sina"));
        assert!(failure.audit_message().contains("provider=Eastmoney"));
        assert!(failure.audit_message().contains("available_provider=Sina"));
        assert_eq!(failure.diagnostic_code(), "provider_evidence_mismatch");
    }

    #[tokio::test]
    async fn br238_rejects_provider_batches_that_exceed_the_requested_limit() {
        let batch = fetch_raw_global_news_batch_with(&OverLimitFixturePort, 1)
            .await
            .expect("typed acquisition returns all terminal attempts");
        for attempt in batch.attempts() {
            let unavailable = attempt
                .terminal()
                .unavailable()
                .expect("over-limit provider response must fail closed");
            assert_eq!(unavailable.diagnostic_code(), "provider_limit_exceeded");
            assert_eq!(unavailable.reason_code(), "invalid_evidence");
            assert!(!unavailable.retryable());
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
    async fn br244_projects_only_available_admitted_records_and_retains_failures() {
        let port = TypedFixturePort {
            calls: AtomicUsize::new(0),
        };
        let batch = fetch_raw_global_news_batch_with(&port, 20)
            .await
            .expect("TEST_CODE raw batch");

        let projection = project_news_flash_events(&batch);

        assert_eq!(projection.available_feed_count(), 1);
        assert_eq!(projection.verified_empty_feed_count(), 1);
        assert_eq!(projection.events().len(), 1);
        assert_eq!(projection.failures().len(), 2);
        let projected = &projection.events()[0];
        let event = projected.event();
        assert_eq!(event.full_title, "TEST_CODE title");
        assert_eq!(
            event.direction,
            crate::signal::market_event::Direction::Neutral
        );
        assert_eq!(
            event.strength, 0,
            "source projection must not invent impact"
        );
        assert_eq!(event.certainty, 100);
        assert_eq!(event.provenance[0].provider, "eastmoney-web");
        assert_eq!(projected.source().event_id(), event.event_id);
        assert_eq!(projected.source().provider(), "Eastmoney");
        assert_eq!(projected.source().source(), "eastmoney-web");
        assert_eq!(
            projected.source().batch_id(),
            "TEST_CODE_eastmoney_global_news"
        );
        assert_eq!(projected.source().published_at(), event.occurred_at);
        assert!(projection.failures().iter().any(|failure| {
            failure.provider() == GlobalNewsProvider::Jin10
                && failure.available_provider().is_none()
                && failure.reason_code() == "no_verified_batch"
                && failure.retryable()
        }));
        assert!(projection.failures().iter().any(|failure| {
            failure.provider() == GlobalNewsProvider::ThePaper
                && failure.diagnostic_code() == "available_batch_empty"
                && !failure.retryable()
        }));
    }

    #[test]
    fn br244_provider_contract_is_closed_and_test_identity_is_capability_bound() {
        assert_eq!(GlobalNewsProvider::Eastmoney.wire_name(), "Eastmoney");
        assert_eq!(GlobalNewsProvider::Cailianpress.wire_name(), "Cailianpress");
        assert_eq!(GlobalNewsProvider::Jin10.wire_name(), "Jin10");
        assert_eq!(GlobalNewsProvider::ThePaper.wire_name(), "ThePaper");
        assert_eq!(provider_id_wire_name(ProviderId::Sina), "Sina");
        assert_eq!(provider_id_wire_name(ProviderId::Custom), "Custom");
        for registration in registered_global_news_feeds() {
            assert_eq!(
                registration.source_contract,
                registration.provider.source(),
                "closed provider and source contract must remain an exact pair"
            );
            assert_eq!(
                GlobalNewsProvider::from_wire_name(registration.provider.wire_name()),
                Some(registration.provider),
                "wire name must round-trip without Debug formatting"
            );
        }
        assert!(news_flash_identity_allowed_for_env(
            crate::risk::env_guard::TradingEnv::Prod,
            &["Eastmoney", "eastmoney-web", "real-batch"],
        ));
        assert!(!news_flash_identity_allowed_for_env(
            crate::risk::env_guard::TradingEnv::Prod,
            &["Eastmoney", "eastmoney-web", "TEST_CODE_BATCH"],
        ));
        assert!(news_flash_identity_allowed_for_env(
            crate::risk::env_guard::TradingEnv::Test,
            &["TEST_CODE_PROVIDER", "TEST_CODE_SOURCE", "TEST_CODE_BATCH"],
        ));
        NewsFlashProjectionTestCapability::bind()
            .expect("unit test process owns the opaque projection capability");
    }

    #[test]
    fn br244_projection_failure_is_provider_atomic_and_does_not_hide_other_events() {
        let attempted_at = DateTime::parse_from_rfc3339("2026-07-28T09:00:02+08:00")
            .expect("TEST_CODE attempted time")
            .with_timezone(&Utc);
        let mut invalid_cls_record = record(GlobalNewsProvider::Cailianpress);
        invalid_cls_record.observed_at =
            invalid_cls_record.published_at - chrono::Duration::seconds(1);
        let attempts = vec![
            RawGlobalNewsFeedAttempt::test_fixture(
                "TEST_CODE raw attempt",
                RegisteredGlobalNewsFeed::for_provider(GlobalNewsProvider::Eastmoney),
                attempted_at,
                TestRawGlobalNewsTerminal::Available {
                    records: vec![record(GlobalNewsProvider::Eastmoney)],
                    evidence: evidence(GlobalNewsProvider::Eastmoney, "eastmoney_global_news"),
                },
            ),
            RawGlobalNewsFeedAttempt::test_fixture(
                "TEST_CODE raw attempt",
                RegisteredGlobalNewsFeed::for_provider(GlobalNewsProvider::Cailianpress),
                attempted_at,
                TestRawGlobalNewsTerminal::Available {
                    records: vec![invalid_cls_record],
                    evidence: evidence(GlobalNewsProvider::Cailianpress, "cls_global_news"),
                },
            ),
            RawGlobalNewsFeedAttempt::test_fixture(
                "TEST_CODE raw attempt",
                RegisteredGlobalNewsFeed::for_provider(GlobalNewsProvider::Jin10),
                attempted_at,
                TestRawGlobalNewsTerminal::VerifiedEmpty {
                    evidence: evidence(GlobalNewsProvider::Jin10, "jin10_global_news"),
                },
            ),
            RawGlobalNewsFeedAttempt::test_fixture(
                "TEST_CODE raw attempt",
                RegisteredGlobalNewsFeed::for_provider(GlobalNewsProvider::ThePaper),
                attempted_at,
                TestRawGlobalNewsTerminal::VerifiedEmpty {
                    evidence: evidence(GlobalNewsProvider::ThePaper, "thepaper_global_news"),
                },
            ),
        ];
        let batch =
            RawNewsAggregationBatch::test_fixture("TEST_CODE raw batch", attempts, attempted_at);

        let projection = project_news_flash_events(&batch);

        assert_eq!(projection.events().len(), 1);
        assert_eq!(projection.events()[0].event().full_title, "TEST_CODE title");
        assert_eq!(projection.failures().len(), 1);
        let failure = &projection.failures()[0];
        assert_eq!(failure.provider(), GlobalNewsProvider::Cailianpress);
        assert_eq!(failure.diagnostic_code(), "record_evidence_incomplete");
        assert_eq!(failure.reason_code(), "invalid_evidence");
        assert!(!failure.retryable());
        assert_eq!(failure.source_record_count(), 1);
        assert_eq!(failure.batch_id(), Some("TEST_CODE_cls_global_news"));
        assert_eq!(failure.record_id(), Some("TEST_CODE_item"));
        assert_eq!(failure.diagnostic(), "record_observation_time_mismatch");
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
            source_record_count: 0,
        };
        assert_eq!(unavailable.diagnostic_code(), "provider_batch_unavailable");
    }

    #[test]
    fn br244_news_flash_projection_binding_is_field_and_order_sensitive() {
        let attempted_at = DateTime::parse_from_rfc3339("2026-07-28T09:00:02+08:00")
            .expect("TEST_CODE attempted time")
            .with_timezone(&Utc);
        let attempts = REGISTERED_PROVIDERS
            .into_iter()
            .map(|provider| {
                let terminal = if provider == GlobalNewsProvider::Eastmoney {
                    TestRawGlobalNewsTerminal::Available {
                        records: vec![record(provider)],
                        evidence: evidence(provider, provider.feed_name()),
                    }
                } else {
                    TestRawGlobalNewsTerminal::VerifiedEmpty {
                        evidence: evidence(provider, provider.feed_name()),
                    }
                };
                RawGlobalNewsFeedAttempt::test_fixture(
                    "TEST_CODE raw attempt",
                    RegisteredGlobalNewsFeed::for_provider(provider),
                    attempted_at,
                    terminal,
                )
            })
            .collect();
        let batch =
            RawNewsAggregationBatch::test_fixture("TEST_CODE raw batch", attempts, attempted_at);
        let projection = project_news_flash_events(&batch);
        let first = projection.events()[0].clone();
        let mut changed = first.clone();
        changed.source.batch_id = "TEST_CODE_changed_batch".to_owned();

        assert_ne!(
            ordered_news_flash_evidence_sha256(std::slice::from_ref(&first)),
            ordered_news_flash_evidence_sha256(std::slice::from_ref(&changed))
        );
        assert_ne!(
            ordered_news_flash_evidence_sha256(&[first.clone(), changed.clone()]),
            ordered_news_flash_evidence_sha256(&[changed, first])
        );
    }

    #[test]
    fn br244_news_flash_failure_identity_binds_required_fields() {
        let failure = NewsFlashSourceFailure {
            provider: GlobalNewsProvider::Jin10,
            available_provider: Some(ProviderId::Eastmoney),
            failed_stage: "TEST_CODE_stage",
            diagnostic_code: "provider_batch_unavailable",
            reason_code: "no_verified_batch",
            retryable: true,
            observed_at: DateTime::parse_from_rfc3339("2026-07-28T09:00:02+08:00")
                .expect("TEST_CODE observed")
                .with_timezone(&Utc),
            source_record_count: 0,
            diagnostic: bounded_news_flash_diagnostic(&"诊".repeat(600)),
            batch_id: Some("TEST_CODE_batch".to_owned()),
            record_id: Some("TEST_CODE_record".to_owned()),
        };
        let mut changed = failure.clone();
        changed.retryable = false;
        assert_eq!(failure.identity_sha256().len(), 64);
        assert_ne!(failure.identity_sha256(), changed.identity_sha256());
        let observed_at = failure.observed_at().fixed_offset();
        assert_eq!(
            failure.identity_sha256(),
            crate::event::envelope::news_flash_failure_identity_hash(
                crate::event::envelope::NewsFlashFailureIdentityFields {
                    provider: Some(failure.provider().wire_name()),
                    available_provider: failure.available_provider_wire(),
                    stage: failure.failed_stage(),
                    reason_code: failure.reason_code(),
                    diagnostic_code: failure.diagnostic_code(),
                    diagnostic: failure.diagnostic(),
                    retryable: failure.retryable(),
                    observed_at: &observed_at,
                    source_record_count: u32::try_from(failure.source_record_count()).unwrap(),
                    batch_id: failure.batch_id(),
                    record_id: failure.record_id(),
                }
            ),
            "raw failure identity must match the typed v6 audit authority"
        );
        assert!(failure.diagnostic().len() <= NEWS_FLASH_DIAGNOSTIC_MAX_BYTES);
        assert!(failure
            .diagnostic()
            .ends_with(NEWS_FLASH_DIAGNOSTIC_TRUNCATED_SUFFIX));
        assert_eq!(failure.batch_id(), Some("TEST_CODE_batch"));
        assert_eq!(failure.record_id(), Some("TEST_CODE_record"));
        assert_eq!(failure.available_provider(), Some(ProviderId::Eastmoney));
        assert!(failure
            .audit_message()
            .contains("diagnostic_code=provider_batch_unavailable"));

        let mut changed = failure.clone();
        changed.available_provider = None;
        assert_ne!(failure.identity_sha256(), changed.identity_sha256());
        let mut changed = failure.clone();
        changed.diagnostic_code = "TEST_CODE_changed_diagnostic_code";
        assert_ne!(failure.identity_sha256(), changed.identity_sha256());
        let mut changed = failure.clone();
        changed.source_record_count = 1;
        assert_ne!(failure.identity_sha256(), changed.identity_sha256());
        let mut changed = failure.clone();
        changed.batch_id = None;
        assert_ne!(failure.identity_sha256(), changed.identity_sha256());
        let mut changed = failure.clone();
        changed.record_id = None;
        assert_ne!(failure.identity_sha256(), changed.identity_sha256());
    }

    #[test]
    #[should_panic(expected = "test fixture identity must start with TEST_CODE")]
    fn fixture_constructor_rejects_non_test_identity() {
        let _ = RawNewsAggregationBatch::test_fixture("production", Vec::new(), Utc::now());
    }
}
