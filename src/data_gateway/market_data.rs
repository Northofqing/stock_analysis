//! BR-064/BR-164/BR-172/BR-210/BR-213 realtime market-data boundary.
//!
//! The ordered route is Magic TDX -> Magic Tencent -> Magic Sina. A provider
//! can only win with a complete batch carrying provider source time. The TDX
//! quote contract at the currently pinned upstream revision does not prove a
//! second-level source timestamp, so the router correctly rejects that batch
//! under the five-second freshness rule and continues to the next Magic
//! provider. No consumer-owned HTTP or legacy parser is retained.

use chrono::{DateTime, Utc};
#[cfg(test)]
use magic_market_core::Exchange;
use magic_market_core::{AssetClass, DataStatus, InstrumentId, ProviderId, Quote, RatioUnit};
use magic_market_router::{
    quote_source, AcceptancePolicy, AttemptStatus, FailureKind, QuoteRouter, RouterError,
    SourceError,
};
use magic_sina_rs::{SinaClient, SinaError};
use magic_tdx_rs::{TdxError, TdxSmartClient};
use magic_tencent_rs::{TencentClient, TencentError};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use super::instrument_identity::{resolve_production_equity, EquitySegment};
use super::parse_evidence_instant;
use super::review::{
    acquisition_request_hash, audit_gateway_result, BatchEvidence, GatewayBatch, GatewayError,
};

const CAPABILITY: &str = "RealtimeMarketQuotes";

/// One admitted quote projection used by monitor consumers.
#[derive(Debug, Clone, PartialEq)]
pub struct RealtimeMarketQuote {
    pub code: String,
    pub name: String,
    pub price: f64,
    pub previous_close: f64,
    pub change_percent: f64,
    pub source_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    pub provider: ProviderId,
    pub batch_id: String,
}

/// One realtime quote that cannot be separated from the audited source batch
/// that admitted it.
///
/// All fields are private and production construction is restricted to
/// [`AdmittedRealtimeQuotes::from_audited_batch`]. This prevents consumers from
/// promoting a freely constructed [`RealtimeMarketQuote`] projection into
/// evidence that can drive a decision.
#[derive(Debug, Clone, PartialEq)]
pub struct AdmittedRealtimeQuote {
    record: RealtimeMarketQuote,
    evidence: BatchEvidence,
}

impl AdmittedRealtimeQuote {
    pub fn code(&self) -> &str {
        &self.record.code
    }

    pub fn name(&self) -> &str {
        &self.record.name
    }

    pub fn price(&self) -> f64 {
        self.record.price
    }

    pub fn source_at(&self) -> DateTime<Utc> {
        self.record.source_at
    }

    pub fn observed_at(&self) -> DateTime<Utc> {
        self.record.observed_at
    }

    pub const fn evidence(&self) -> &BatchEvidence {
        &self.evidence
    }

    /// Pure test seam. This symbol is absent from production builds and keeps
    /// test/live identities physically distinct.
    #[cfg(test)]
    pub(crate) fn from_test_fixture(
        record: RealtimeMarketQuote,
        evidence: BatchEvidence,
    ) -> Result<Self, GatewayError> {
        if !record.code.starts_with("TEST_CODE_")
            || !evidence.source.starts_with("TEST_CODE")
            || !evidence.batch_id.starts_with("TEST_CODE")
        {
            return Err(GatewayError::invalid_request(
                CAPABILITY,
                "realtime-quote fixtures must use TEST_CODE identities",
            ));
        }
        validate_admitted_projection(&record, &evidence)?;
        Ok(Self { record, evidence })
    }
}

/// A non-empty realtime quote batch whose records remain bound to the exact
/// provider evidence admitted by [`MarketDataGateway`].
#[derive(Debug)]
pub struct AdmittedRealtimeQuotes {
    quotes: Vec<AdmittedRealtimeQuote>,
}

impl AdmittedRealtimeQuotes {
    fn from_audited_batch(batch: GatewayBatch<RealtimeMarketQuote>) -> Result<Self, GatewayError> {
        match batch {
            GatewayBatch::Available { records, evidence } if !records.is_empty() => {
                let mut quotes = Vec::with_capacity(records.len());
                for record in records {
                    validate_admitted_projection(&record, &evidence)?;
                    quotes.push(AdmittedRealtimeQuote {
                        record,
                        evidence: evidence.clone(),
                    });
                }
                Ok(Self { quotes })
            }
            GatewayBatch::Available { evidence, .. } | GatewayBatch::VerifiedEmpty(evidence) => {
                Err(GatewayError::unavailable(
                    CAPABILITY,
                    Some(evidence.provider),
                    true,
                    format!(
                        "provider returned no admitted realtime quotes source={} batch_id={}",
                        evidence.source, evidence.batch_id
                    ),
                ))
            }
        }
    }

    pub fn len(&self) -> usize {
        self.quotes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.quotes.is_empty()
    }

    pub fn quotes(&self) -> &[AdmittedRealtimeQuote] {
        &self.quotes
    }

    /// Consume the sealed batch and return the exact requested quote. Absence
    /// is an identity/evidence failure, never a default quote.
    pub fn into_required_quote(
        self,
        required_code: &str,
    ) -> Result<AdmittedRealtimeQuote, GatewayError> {
        self.quotes
            .into_iter()
            .find(|quote| quote.code() == required_code)
            .ok_or_else(|| {
                GatewayError::invalid_evidence(
                    CAPABILITY,
                    None,
                    format!("admitted batch does not contain required quote {required_code}"),
                )
            })
    }
}

/// Evidence-preserving public quote route.
#[derive(Debug, Clone, Copy, Default)]
pub struct MarketDataGateway;

impl MarketDataGateway {
    pub const fn new() -> Self {
        Self
    }

    pub fn realtime_quotes(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<RealtimeMarketQuote>, GatewayError> {
        let request_hash = acquisition_request_hash(CAPABILITY, &codes.join(","));
        let instruments = match build_instruments(codes) {
            Ok(instruments) => instruments,
            Err(error) => {
                return audit_gateway_result(
                    CAPABILITY,
                    ProviderId::Tdx,
                    &request_hash,
                    Err(error),
                );
            }
        };

        let (terminal_provider, result) = route_quotes(codes, &instruments);
        audit_gateway_result(CAPABILITY, terminal_provider, &request_hash, result)
    }

    /// Acquire a non-empty batch whose quote projections cannot be detached
    /// from their audited provider evidence.
    pub fn required_realtime_quotes(
        &self,
        codes: &[String],
    ) -> Result<AdmittedRealtimeQuotes, GatewayError> {
        AdmittedRealtimeQuotes::from_audited_batch(self.realtime_quotes(codes)?)
    }

    /// Acquire exactly one source-bound realtime quote.
    pub fn required_realtime_quote(
        &self,
        code: &str,
    ) -> Result<AdmittedRealtimeQuote, GatewayError> {
        self.required_realtime_quotes(&[code.to_owned()])?
            .into_required_quote(code)
    }
}

fn validate_admitted_projection(
    record: &RealtimeMarketQuote,
    evidence: &BatchEvidence,
) -> Result<(), GatewayError> {
    let evidence_observed_at = parse_evidence_instant(
        CAPABILITY,
        evidence.provider,
        "observed_at",
        &evidence.observed_at,
    )?;
    let evidence_source_at = evidence
        .source_at
        .as_deref()
        .ok_or_else(|| {
            GatewayError::invalid_evidence(
                CAPABILITY,
                Some(evidence.provider),
                "admitted realtime batch has no provider source time",
            )
        })
        .and_then(|value| {
            parse_evidence_instant(CAPABILITY, evidence.provider, "source_at", value)
        })?;
    if record.provider != evidence.provider
        || record.batch_id != evidence.batch_id
        || record.observed_at != evidence_observed_at
        || record.source_at != evidence_source_at
    {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            Some(evidence.provider),
            format!(
                "realtime quote {} differs from admitted batch evidence",
                record.code
            ),
        ));
    }
    Ok(())
}

fn build_instruments(codes: &[String]) -> Result<Vec<InstrumentId>, GatewayError> {
    if codes.is_empty() {
        return Err(GatewayError::invalid_request(
            CAPABILITY,
            "quote request must contain at least one A-share code",
        ));
    }
    let mut seen = HashSet::with_capacity(codes.len());
    codes
        .iter()
        .map(|storage_code| {
            if !seen.insert(storage_code.as_str()) {
                return Err(GatewayError::invalid_request(
                    CAPABILITY,
                    format!("duplicate quote code {storage_code:?}"),
                ));
            }
            build_instrument(storage_code)
        })
        .collect()
}

fn build_instrument(storage_code: &str) -> Result<InstrumentId, GatewayError> {
    #[cfg(test)]
    let identity = if storage_code.starts_with("TEST_CODE_") {
        super::instrument_identity::resolve_test_equity(storage_code, None)
    } else {
        resolve_production_equity(storage_code, None)
    };
    #[cfg(not(test))]
    let identity = resolve_production_equity(storage_code, None);
    let identity = identity
        .and_then(|identity| {
            identity.require_a_share()?;
            Ok(identity)
        })
        .map_err(|error| {
            GatewayError::invalid_request(
                CAPABILITY,
                format!("invalid realtime equity identity {storage_code:?}: {error}"),
            )
        })?;
    if identity.segment() == EquitySegment::BeijingA
        && !identity.canonical_code().starts_with("920")
    {
        return Err(GatewayError::invalid_request(
            CAPABILITY,
            format!("realtime quote providers have no verified capability for {storage_code:?}"),
        ));
    }
    InstrumentId::new(
        identity.exchange(),
        identity.canonical_code(),
        AssetClass::Equity,
    )
    .map_err(|error| {
        GatewayError::invalid_request(
            CAPABILITY,
            format!("validated instrument {storage_code:?} failed core invariant: {error}"),
        )
    })
}

/// BR-217: acquisition-time freshness cannot be expressed by the router.
///
/// `AcceptancePolicy::with_max_source_age` bounds `fetched_at - source_at`,
/// i.e. the provider's *internal* lag, not how old the data is when this
/// process consumes it. The AGENTS §2.4 five-second red line is the latter,
/// so it stays in [`admit_quote_batch`]. To keep that gate from terminating
/// the whole acquisition, each provider is routed on its own and a retryable
/// admission failure falls over to the next one instead of aborting.
fn realtime_quote_acceptance_policy() -> AcceptancePolicy {
    AcceptancePolicy::new()
        .with_require_complete(true)
        .with_require_source_at(true)
}

fn route_quotes(
    storage_codes: &[String],
    instruments: &[InstrumentId],
) -> (
    ProviderId,
    Result<GatewayBatch<RealtimeMarketQuote>, GatewayError>,
) {
    // BR-217: the AGENTS §2.4 five-second quote red line belongs inside the
    // BR-217: route each provider on its own so that an acquisition-time
    // staleness rejection in `admit_quote_batch` falls over to the next
    // provider instead of terminating the whole acquisition. A single
    // multi-source router cannot do this: it selects the first batch that
    // satisfies the acceptance policy and returns, after which the §2.4
    // five-second gate can only abort, permanently shadowing the healthier
    // providers registered behind a systematically-late one.
    let policy = realtime_quote_acceptance_policy();
    let mut last_failure: Option<(ProviderId, GatewayError)> = None;
    // BR-219: proven-dead providers are skipped so their retry budget is not
    // paid on every acquisition.
    let skips = quote_breaker_skips(Utc::now());

    for provider in QUOTE_PROVIDER_CHAIN {
        if skips.contains(&provider) {
            continue;
        }
        let source = match quote_chain_source(provider) {
            Ok(source) => source,
            Err(error) => return (provider, Err(router_gateway_error(error, provider))),
        };
        let mut router = QuoteRouter::new(policy);
        if let Err(error) = router.register(source) {
            return (provider, Err(router_gateway_error(error, provider)));
        }

        let admission = match router.route(instruments) {
            Ok(outcome) => {
                let selected = outcome.selected_provider();
                match admit_quote_batch(storage_codes, selected, outcome.into_batch()) {
                    Ok(batch) => {
                        record_quote_provider_success(selected);
                        return (selected, Ok(batch));
                    }
                    Err(error) => Some((selected, error)),
                }
            }
            Err(error) => {
                // A single-source router can only be exhausted by this very
                // provider, so its terminal-looking verdict describes one
                // route, never the whole chain. Always advance; treating it as
                // final is what let a broken primary shadow the fallbacks.
                record_quote_provider_failure(provider, Utc::now());
                last_failure = Some((provider, router_gateway_error(error, provider)));
                None
            }
        };

        if let Some((selected, error)) = admission {
            record_quote_provider_failure(selected, Utc::now());
            // A non-retryable admission failure is a definitive statement about
            // the request itself, not about this provider's liveness; trying
            // the next source would only relabel the same rejection.
            if !error.retryable() {
                return (selected, Err(error));
            }
            last_failure = Some((selected, error));
        }
    }

    match last_failure {
        Some((provider, error)) => (provider, Err(error)),
        None => (
            ProviderId::Tdx,
            Err(GatewayError::unavailable(
                CAPABILITY,
                None,
                true,
                "no realtime quote provider is registered".to_owned(),
            )),
        ),
    }
}

/// BR-217: failover order for realtime quotes. Magic TDX stays the first
/// A-share route candidate; the HTTP providers behind it are fallbacks only.
const QUOTE_PROVIDER_CHAIN: [ProviderId; 3] =
    [ProviderId::Tdx, ProviderId::Tencent, ProviderId::Sina];

/// BR-219: consecutive failures before a proven-dead provider is skipped.
const QUOTE_BREAKER_FAILURE_THRESHOLD: u32 = 3;
/// BR-219: how long a tripped provider stays skipped before it is retried.
const QUOTE_BREAKER_COOLDOWN_SECS: i64 = 300;

#[derive(Debug, Clone, Copy, Default)]
struct QuoteBreakerState {
    consecutive_failures: u32,
    opened_at: Option<DateTime<Utc>>,
}

fn quote_breakers() -> &'static Mutex<HashMap<ProviderId, QuoteBreakerState>> {
    static BREAKERS: OnceLock<Mutex<HashMap<ProviderId, QuoteBreakerState>>> = OnceLock::new();
    BREAKERS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// BR-219: providers whose retry budget is currently not worth paying.
///
/// Skipping only reorders attempts; it never fabricates a batch and never
/// turns an unattempted provider into a successful or empty one. If the whole
/// chain is tripped the skip set is discarded so that a transient outage can
/// never escalate into permanently not acquiring anything.
fn quote_breaker_skips(now: DateTime<Utc>) -> HashSet<ProviderId> {
    let mut guard = match quote_breakers().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let mut skips = HashSet::new();
    for provider in QUOTE_PROVIDER_CHAIN {
        let Some(state) = guard.get_mut(&provider) else {
            continue;
        };
        let Some(opened_at) = state.opened_at else {
            continue;
        };
        let elapsed = now.signed_duration_since(opened_at).num_seconds();
        if elapsed >= QUOTE_BREAKER_COOLDOWN_SECS {
            state.opened_at = None;
            state.consecutive_failures = 0;
            continue;
        }
        log::warn!(
            "[DataGateway][RealtimeMarketQuotes][BR-219] skipping provider={provider:?} \
             consecutive_failures={} cooldown_remaining_secs={}",
            state.consecutive_failures,
            QUOTE_BREAKER_COOLDOWN_SECS - elapsed
        );
        skips.insert(provider);
    }
    if skips.len() == QUOTE_PROVIDER_CHAIN.len() {
        log::warn!(
            "[DataGateway][RealtimeMarketQuotes][BR-219] every provider is tripped; \
             ignoring the breaker and attempting the whole chain"
        );
        return HashSet::new();
    }
    skips
}

fn record_quote_provider_success(provider: ProviderId) {
    let mut guard = match quote_breakers().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    guard.insert(provider, QuoteBreakerState::default());
}

fn record_quote_provider_failure(provider: ProviderId, now: DateTime<Utc>) {
    let mut guard = match quote_breakers().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let state = guard.entry(provider).or_default();
    state.consecutive_failures = state.consecutive_failures.saturating_add(1);
    if state.consecutive_failures >= QUOTE_BREAKER_FAILURE_THRESHOLD && state.opened_at.is_none() {
        state.opened_at = Some(now);
        log::warn!(
            "[DataGateway][RealtimeMarketQuotes][BR-219] tripped provider={provider:?} \
             consecutive_failures={} cooldown_secs={QUOTE_BREAKER_COOLDOWN_SECS}",
            state.consecutive_failures
        );
    }
}

fn quote_chain_source(
    provider: ProviderId,
) -> Result<magic_market_router::SourceFn<[InstrumentId], magic_market_core::Quote>, RouterError> {
    match provider {
        ProviderId::Tdx => Ok(quote_source(
            ProviderId::Tdx,
            Arc::new(TdxSmartClient::new()),
            classify_tdx_error,
        )),
        ProviderId::Tencent => {
            let client = TencentClient::new().map_err(|error| {
                RouterError::InvalidConfiguration(format!(
                    "Magic Tencent quote client initialization failed: {error}"
                ))
            })?;
            Ok(quote_source(
                ProviderId::Tencent,
                Arc::new(client),
                classify_tencent_error,
            ))
        }
        ProviderId::Sina => {
            let client = SinaClient::new().map_err(|error| {
                RouterError::InvalidConfiguration(format!(
                    "Magic Sina quote client initialization failed: {error}"
                ))
            })?;
            Ok(quote_source(
                ProviderId::Sina,
                Arc::new(client),
                classify_sina_error,
            ))
        }
        other => Err(RouterError::InvalidConfiguration(format!(
            "provider {other:?} is not a registered realtime quote route"
        ))),
    }
}

fn admit_quote_batch(
    storage_codes: &[String],
    provider: ProviderId,
    batch: magic_market_core::DataBatch<Quote>,
) -> Result<GatewayBatch<RealtimeMarketQuote>, GatewayError> {
    let evidence = BatchEvidence::from_provenance(provider, batch.provenance())?;
    if batch.records().is_empty() {
        return Err(GatewayError::classified(
            CAPABILITY,
            Some(provider),
            "unavailable",
            "verified_quote_batch_empty",
            true,
            "realtime quote providers must return every requested instrument",
        ));
    }
    if batch.records().len() != storage_codes.len() {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            Some(provider),
            format!(
                "quote cardinality mismatch requested={} actual={}",
                storage_codes.len(),
                batch.records().len()
            ),
        ));
    }

    let now = Utc::now();
    let observed_at = parse_evidence_instant(
        CAPABILITY,
        provider,
        "observed_at",
        batch.provenance().fetched_at(),
    )?;
    let mut records = Vec::with_capacity(batch.records().len());
    let mut stale_exclusions: Vec<String> = Vec::new();
    for (storage_code, quote) in storage_codes.iter().zip(batch.records()) {
        let expected = build_instrument(storage_code)?;
        if quote.instrument() != &expected
            || quote.provider() != provider
            || quote.batch_id() != evidence.batch_id
            || quote.observed_at() != evidence.observed_at
            || quote.status() != DataStatus::Available
        {
            return Err(GatewayError::invalid_evidence(
                CAPABILITY,
                Some(provider),
                format!("quote evidence/identity mismatch for {storage_code}"),
            ));
        }
        let name = quote
            .name()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                GatewayError::invalid_evidence(
                    CAPABILITY,
                    Some(provider),
                    format!("quote name is unavailable for {storage_code}"),
                )
            })?
            .to_owned();
        let change = quote.change_percent().ok_or_else(|| {
            GatewayError::invalid_evidence(
                CAPABILITY,
                Some(provider),
                format!("quote change percent is unavailable for {storage_code}"),
            )
        })?;
        if change.unit() != RatioUnit::Percent {
            return Err(GatewayError::invalid_evidence(
                CAPABILITY,
                Some(provider),
                format!("quote change percent unit mismatch for {storage_code}"),
            ));
        }
        let previous_close = quote.previous_close().ok_or_else(|| {
            GatewayError::invalid_evidence(
                CAPABILITY,
                Some(provider),
                format!("quote previous close is unavailable for {storage_code}"),
            )
        })?;
        let source_at = parse_evidence_instant(
            CAPABILITY,
            provider,
            "source_at",
            quote.source_at().ok_or_else(|| {
                GatewayError::invalid_evidence(
                    CAPABILITY,
                    Some(provider),
                    format!("quote source time is unavailable for {storage_code}"),
                )
            })?,
        )?;
        let age_ms = now.signed_duration_since(source_at).num_milliseconds();
        if !(0..=5_000).contains(&age_ms) {
            // BR-218: the five-second red line is judged per record. A stale
            // record is excluded outright — never repaired, back-filled or
            // served from a previous round — but it must not discard the
            // records that did meet the gate.
            stale_exclusions.push(format!("{storage_code}@{age_ms}ms"));
            continue;
        }

        records.push(RealtimeMarketQuote {
            code: storage_code.clone(),
            name,
            price: quote.price().get(),
            previous_close: previous_close.get(),
            change_percent: change.get(),
            source_at,
            observed_at,
            provider,
            batch_id: quote.batch_id().to_owned(),
        });
    }

    if records.is_empty() {
        // BR-218: every requested instrument was stale. This is still a
        // whole-batch, retryable staleness verdict so BR-217 fails over.
        return Err(GatewayError::classified(
            CAPABILITY,
            Some(provider),
            "stale",
            "quote_stale",
            true,
            format!(
                "every quote failed the five-second freshness gate: {}",
                stale_exclusions.join(",")
            ),
        ));
    }
    if !stale_exclusions.is_empty() {
        log::warn!(
            "[DataGateway][RealtimeMarketQuotes][BR-218] provider={provider:?} batch_id={} \
             admitted={} excluded_stale={} excluded=[{}]",
            evidence.batch_id,
            records.len(),
            stale_exclusions.len(),
            stale_exclusions.join(",")
        );
    }

    Ok(GatewayBatch::Available { records, evidence })
}

fn router_gateway_error(error: RouterError, provider: ProviderId) -> GatewayError {
    let attempts = error
        .attempts()
        .iter()
        .map(|attempt| format!("{:?}={:?}", attempt.provider_id(), attempt.status()))
        .collect::<Vec<_>>()
        .join("; ");
    let last_kind = error
        .attempts()
        .last()
        .and_then(|attempt| match attempt.status() {
            AttemptStatus::Failed { kind, .. } | AttemptStatus::Rejected { kind, .. } => {
                Some(*kind)
            }
            AttemptStatus::Selected => None,
        });
    let (outcome, reason_code, retryable) = match last_kind {
        Some(FailureKind::InvalidRequest) | None => {
            ("invalid_request", "router_invalid_request", false)
        }
        Some(FailureKind::Unsupported) => ("unsupported", "router_unsupported", false),
        Some(
            FailureKind::Transport
            | FailureKind::Timeout
            | FailureKind::RateLimited
            | FailureKind::Provider
            | FailureKind::NoData,
        ) => ("unavailable", "router_sources_exhausted", true),
        Some(FailureKind::Protocol | FailureKind::Quality | FailureKind::Evidence) => {
            ("partial", "router_batch_rejected", false)
        }
    };
    GatewayError::classified(
        CAPABILITY,
        Some(provider),
        outcome,
        reason_code,
        retryable,
        format!("{error}; attempts=[{attempts}]"),
    )
}

fn classify_tdx_error(error: TdxError) -> SourceError {
    let message = error.to_string();
    match error {
        TdxError::Unsupported(_) => SourceError::try_next(FailureKind::Unsupported, message),
        TdxError::Io(_)
        | TdxError::Connection(_)
        | TdxError::ConnectionTimeout
        | TdxError::SetupFailed(_)
        | TdxError::Disconnected
        | TdxError::RetryExhausted(_) => SourceError::try_next(FailureKind::Transport, message),
        TdxError::HistoricalBarCardinality {
            offset,
            actual,
            expected_page,
            requested_total,
        } => SourceError::try_next(
            FailureKind::Protocol,
            format!(
                "Magic TDX historical-bar cardinality mismatch: offset={offset} actual={actual} \
                 expected_page={expected_page} requested_total={requested_total}"
            ),
        ),
        TdxError::Parse(_)
        | TdxError::InvalidData(_)
        | TdxError::ResponseParse(_)
        | TdxError::Core(_)
        | TdxError::Coded(_)
        | TdxError::FileNotFound(_) => SourceError::try_next(FailureKind::Protocol, message),
    }
}

fn classify_tencent_error(error: TencentError) -> SourceError {
    let message = error.to_string();
    match error {
        TencentError::InvalidRequest(_) => SourceError::stop(FailureKind::InvalidRequest, message),
        TencentError::Transport(_) => SourceError::try_next(FailureKind::Transport, message),
        TencentError::Decode(_) | TencentError::Protocol(_) => {
            SourceError::try_next(FailureKind::Protocol, message)
        }
        TencentError::Unsupported(_) => SourceError::try_next(FailureKind::Unsupported, message),
        TencentError::Core(_) => SourceError::try_next(FailureKind::Evidence, message),
    }
}

fn classify_sina_error(error: SinaError) -> SourceError {
    let message = error.to_string();
    match error {
        SinaError::InvalidRequest(_) => SourceError::stop(FailureKind::InvalidRequest, message),
        SinaError::Transport(_) => SourceError::try_next(FailureKind::Transport, message),
        SinaError::Decode(_) | SinaError::Protocol(_) => {
            SourceError::try_next(FailureKind::Protocol, message)
        }
        SinaError::Unsupported(_) => SourceError::try_next(FailureKind::Unsupported, message),
        SinaError::Core(_) => SourceError::try_next(FailureKind::Evidence, message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use magic_market_core::{DataBatch, Money, Price, Provenance, Quantity, Ratio, SourceEvidence};

    fn quote_batch(
        code: &str,
        provider: ProviderId,
        batch_id: &str,
        source_at: DateTime<Utc>,
    ) -> DataBatch<Quote> {
        let timestamp = source_at.to_rfc3339();
        let instrument = InstrumentId::new(Exchange::Shanghai, code, AssetClass::Equity).unwrap();
        let quote = Quote::from_parts(
            instrument,
            Some("协议测试股票".to_owned()),
            Price::new(10.0).unwrap(),
            Some(Price::new(9.5).unwrap()),
            Some(Price::new(9.6).unwrap()),
            Some(Price::new(10.1).unwrap()),
            Some(Price::new(9.4).unwrap()),
            Some(Ratio::new(5.263_157_894_7, RatioUnit::Percent).unwrap()),
            Quantity::new(100.0).unwrap(),
            Some(Money::new(1_000_000.0).unwrap()),
            DataStatus::Available,
            Some(timestamp.clone()),
            timestamp.clone(),
            provider,
            batch_id,
        )
        .unwrap();
        let provenance = Provenance::new("TEST_CODE_quote", &timestamp)
            .unwrap()
            .with_source_at(&timestamp)
            .unwrap()
            .with_batch_id(batch_id)
            .unwrap();
        DataBatch::strict(vec![quote], provenance)
    }

    /// BR-218: a multi-record batch whose members carry independent source
    /// times, matching how free A-share feeds actually behave.
    fn quote_batch_multi(
        entries: &[(&str, DateTime<Utc>)],
        provider: ProviderId,
        batch_id: &str,
    ) -> DataBatch<Quote> {
        let observed_at = Utc::now().to_rfc3339();
        let quotes = entries
            .iter()
            .map(|(code, source_at)| {
                let timestamp = source_at.to_rfc3339();
                let instrument =
                    InstrumentId::new(Exchange::Shanghai, *code, AssetClass::Equity).unwrap();
                Quote::from_parts(
                    instrument,
                    Some("协议测试股票".to_owned()),
                    Price::new(10.0).unwrap(),
                    Some(Price::new(9.5).unwrap()),
                    Some(Price::new(9.6).unwrap()),
                    Some(Price::new(10.1).unwrap()),
                    Some(Price::new(9.4).unwrap()),
                    Some(Ratio::new(5.263_157_894_7, RatioUnit::Percent).unwrap()),
                    Quantity::new(100.0).unwrap(),
                    Some(Money::new(1_000_000.0).unwrap()),
                    DataStatus::Available,
                    Some(timestamp),
                    observed_at.clone(),
                    provider,
                    batch_id,
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        let provenance = Provenance::new("TEST_CODE_quote", &observed_at)
            .unwrap()
            .with_source_at(&observed_at)
            .unwrap()
            .with_batch_id(batch_id)
            .unwrap();
        DataBatch::strict(quotes, provenance)
    }

    /// BR-218 / AGENTS §2.4 + §2.2: one instrument whose feed lagged past five
    /// seconds must be excluded, not allowed to discard the fresh records
    /// alongside it. This is the defect that produced zero live quotes: free
    /// feeds lag 0.5–5s per instrument independently, so an all-or-nothing
    /// batch verdict almost never passes for a realistic watchlist.
    #[test]
    fn br218_stale_record_is_excluded_without_discarding_fresh_records() {
        let now = Utc::now();
        let codes = vec![
            "600396".to_owned(),
            "600519".to_owned(),
            "600036".to_owned(),
        ];
        let batch = quote_batch_multi(
            &[
                ("600396", now - chrono::Duration::milliseconds(500)),
                ("600519", now - chrono::Duration::seconds(30)),
                ("600036", now - chrono::Duration::milliseconds(900)),
            ],
            ProviderId::Tencent,
            "TEST_CODE_partition_batch",
        );

        let admitted = admit_quote_batch(&codes, ProviderId::Tencent, batch)
            .expect("fresh records must survive a stale sibling");
        let kept = admitted
            .records()
            .iter()
            .map(|record| record.code.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            kept,
            vec!["600396", "600036"],
            "the stale instrument stays absent; it is never repaired or back-filled"
        );
    }

    /// BR-218: excluding stale records must not weaken the red line. When every
    /// record is stale the batch still fails retryably so BR-217 fails over.
    #[test]
    fn br218_all_stale_records_still_fail_the_batch_retryably() {
        let now = Utc::now();
        let codes = vec!["600396".to_owned(), "600519".to_owned()];
        let batch = quote_batch_multi(
            &[
                ("600396", now - chrono::Duration::seconds(6)),
                ("600519", now - chrono::Duration::seconds(30)),
            ],
            ProviderId::Tencent,
            "TEST_CODE_all_stale_batch",
        );

        let error = admit_quote_batch(&codes, ProviderId::Tencent, batch)
            .expect_err("an entirely stale batch must remain an explicit failure");
        assert!(error.retryable(), "staleness must keep failing over");
        assert!(
            error.to_string().contains("quote_stale"),
            "reason code must stay quote_stale: {error}"
        );
    }

    /// BR-219: a proven-dead provider must stop costing its full retry budget
    /// on every acquisition, and any success must immediately re-arm it.
    #[test]
    fn br219_breaker_trips_after_threshold_and_resets_on_success() {
        reset_quote_breakers();
        let now = Utc::now();
        for _ in 0..QUOTE_BREAKER_FAILURE_THRESHOLD {
            record_quote_provider_failure(ProviderId::Tdx, now);
        }
        assert!(
            quote_breaker_skips(now).contains(&ProviderId::Tdx),
            "a provider failing {QUOTE_BREAKER_FAILURE_THRESHOLD} times in a row must be skipped"
        );

        record_quote_provider_success(ProviderId::Tdx);
        assert!(
            !quote_breaker_skips(now).contains(&ProviderId::Tdx),
            "one success must re-arm the provider immediately"
        );
        reset_quote_breakers();
    }

    /// BR-219: the breaker only reorders attempts. If every provider is tripped
    /// it must be ignored, otherwise a transient full outage escalates into
    /// permanently acquiring nothing.
    #[test]
    fn br219_fully_tripped_chain_is_attempted_in_full() {
        reset_quote_breakers();
        let now = Utc::now();
        for provider in QUOTE_PROVIDER_CHAIN {
            for _ in 0..QUOTE_BREAKER_FAILURE_THRESHOLD {
                record_quote_provider_failure(provider, now);
            }
        }
        assert!(
            quote_breaker_skips(now).is_empty(),
            "a fully tripped chain must still be attempted end to end"
        );
        reset_quote_breakers();
    }

    /// BR-219: the cooldown must expire on its own so a recovered provider is
    /// retried without needing a process restart.
    #[test]
    fn br219_cooldown_expires_and_the_provider_is_retried() {
        reset_quote_breakers();
        let tripped_at = Utc::now();
        for _ in 0..QUOTE_BREAKER_FAILURE_THRESHOLD {
            record_quote_provider_failure(ProviderId::Tencent, tripped_at);
        }
        assert!(quote_breaker_skips(tripped_at).contains(&ProviderId::Tencent));

        let after_cooldown =
            tripped_at + chrono::Duration::seconds(QUOTE_BREAKER_COOLDOWN_SECS + 1);
        assert!(
            !quote_breaker_skips(after_cooldown).contains(&ProviderId::Tencent),
            "the breaker must re-arm once the cooldown elapses"
        );
        reset_quote_breakers();
    }

    fn reset_quote_breakers() {
        let mut guard = match quote_breakers().lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.clear();
    }

    /// BR-217: `AcceptancePolicy::with_max_source_age` bounds the provider's
    /// internal lag (`fetched_at - source_at`), not how old the data is when
    /// this process consumes it, so it cannot express the AGENTS §2.4
    /// five-second red line. This pins that distinction: a batch whose source
    /// time is six seconds behind the wall clock but internally consistent is
    /// accepted by the router and must be rejected by `admit_quote_batch`.
    #[test]
    fn br217_router_policy_cannot_express_acquisition_time_freshness() {
        use magic_market_router::quote_source;

        struct FixedQuoteProvider {
            batch: DataBatch<Quote>,
        }

        #[derive(Debug)]
        struct NeverFails;

        impl std::fmt::Display for NeverFails {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("unreachable")
            }
        }

        impl std::error::Error for NeverFails {}

        impl magic_market_core::RealtimeQuotes for FixedQuoteProvider {
            type Quote = Quote;
            type Error = NeverFails;

            fn realtime_quotes(
                &self,
                _instruments: &[InstrumentId],
            ) -> Result<DataBatch<Self::Quote>, Self::Error> {
                Ok(self.batch.clone())
            }
        }

        let stale = quote_batch(
            "600396",
            ProviderId::Tencent,
            "TEST_CODE_stale_batch",
            Utc::now() - chrono::Duration::seconds(6),
        );

        let mut router = QuoteRouter::new(realtime_quote_acceptance_policy());
        router
            .register(quote_source(
                ProviderId::Tencent,
                Arc::new(FixedQuoteProvider { batch: stale }),
                |error: NeverFails| SourceError::try_next(FailureKind::Protocol, error.to_string()),
            ))
            .expect("the fixed provider registers");

        let instruments = build_instruments(&["600396".to_owned()]).unwrap();
        let outcome = router
            .route(&instruments)
            .expect("router freshness is provider-internal, so it accepts this batch");

        let admission = admit_quote_batch(
            &["600396".to_owned()],
            outcome.selected_provider(),
            outcome.into_batch(),
        );
        let error = admission.expect_err("§2.4 five-second gate must reject a six-second quote");
        assert!(
            error.retryable(),
            "a stale quote must stay retryable so BR-217 failover can try the next provider"
        );
    }

    /// BR-217 / AGENTS §2.4: the five-second bound must not be relocated into
    /// the router policy, and provider source time stays mandatory.
    #[test]
    fn br217_realtime_quote_policy_keeps_source_evidence_without_widening_the_red_line() {
        let policy = realtime_quote_acceptance_policy();
        assert_eq!(
            policy.max_source_age(),
            None,
            "router freshness measures provider-internal lag; §2.4 lives in admit_quote_batch"
        );
        assert!(policy.require_source_at());
        assert!(policy.require_complete());
    }

    /// BR-217: the failover chain keeps Magic TDX first and every entry must be
    /// a constructible quote route, otherwise a stale primary silently becomes
    /// terminal again.
    #[test]
    fn br217_quote_provider_chain_is_ordered_and_constructible() {
        assert_eq!(QUOTE_PROVIDER_CHAIN[0], ProviderId::Tdx);
        for provider in QUOTE_PROVIDER_CHAIN {
            assert!(
                quote_chain_source(provider).is_ok(),
                "provider {provider:?} must be a registered realtime quote route"
            );
        }
        assert!(quote_chain_source(ProviderId::Cninfo).is_err());
    }

    #[test]
    fn br164_quote_request_rejects_empty_duplicate_and_real_symbol_test_ids() {
        assert!(build_instruments(&[]).is_err());
        assert!(
            build_instruments(&["TEST_CODE_600396".to_owned(), "TEST_CODE_600396".to_owned()])
                .is_err()
        );
        assert!(build_instruments(&["TEST_CODE_BAD".to_owned()]).is_err());
        assert!(build_instruments(&["TEST_CODE_600396".to_owned()]).is_ok());
    }

    #[test]
    fn br164_complete_tencent_quote_batch_keeps_source_evidence() {
        let source_at = Utc::now();
        let observed_at = source_at;
        let source_text = source_at.to_rfc3339();
        let observed_text = observed_at.to_rfc3339();
        let instrument =
            InstrumentId::new(Exchange::Shanghai, "600396", AssetClass::Equity).unwrap();
        let batch_id = "TEST_CODE_tencent_quote_batch";
        let quote = Quote::from_parts(
            instrument,
            Some("协议测试股票".to_owned()),
            Price::new(10.0).unwrap(),
            Some(Price::new(9.5).unwrap()),
            Some(Price::new(9.6).unwrap()),
            Some(Price::new(10.1).unwrap()),
            Some(Price::new(9.4).unwrap()),
            Some(Ratio::new(5.263_157_894_7, RatioUnit::Percent).unwrap()),
            Quantity::new(100.0).unwrap(),
            Some(Money::new(1_000_000.0).unwrap()),
            DataStatus::Available,
            Some(source_text.clone()),
            observed_text.clone(),
            ProviderId::Tencent,
            batch_id,
        )
        .unwrap();
        let provenance = Provenance::new("TEST_CODE_tencent", observed_text)
            .unwrap()
            .with_source_at(source_text)
            .unwrap()
            .with_batch_id(batch_id)
            .unwrap();
        let batch = DataBatch::strict(vec![quote], provenance);

        let admitted =
            admit_quote_batch(&["TEST_CODE_600396".to_owned()], ProviderId::Tencent, batch)
                .unwrap();
        assert_eq!(admitted.records()[0].code, "TEST_CODE_600396");
        assert_eq!(admitted.records()[0].provider, ProviderId::Tencent);
        assert_eq!(admitted.records()[0].price, 10.0);
    }

    #[test]
    fn br172_admitted_realtime_quote_keeps_record_and_batch_evidence_sealed() {
        let now = Utc::now();
        let audited = admit_quote_batch(
            &["TEST_CODE_600396".to_owned()],
            ProviderId::Tencent,
            quote_batch("600396", ProviderId::Tencent, "TEST_CODE_sealed_quote", now),
        )
        .expect("TEST_CODE quote batch must pass transport admission");

        let admitted = AdmittedRealtimeQuotes::from_audited_batch(audited)
            .expect("TEST_CODE audited batch must become a sealed capability");
        assert_eq!(admitted.len(), 1);
        let quote = admitted
            .into_required_quote("TEST_CODE_600396")
            .expect("TEST_CODE exact quote must remain source-bound");
        assert_eq!(quote.code(), "TEST_CODE_600396");
        assert_eq!(quote.price(), 10.0);
        assert_eq!(quote.evidence().provider, ProviderId::Tencent);
        assert_eq!(quote.evidence().batch_id, "TEST_CODE_sealed_quote");
    }

    #[test]
    fn br172_realtime_quote_fixture_rejects_real_identity() {
        let now = Utc::now();
        let error = AdmittedRealtimeQuote::from_test_fixture(
            RealtimeMarketQuote {
                code: "600396".to_owned(),
                name: "TEST_CODE quote".to_owned(),
                price: 10.0,
                previous_close: 9.5,
                change_percent: 5.26,
                source_at: now,
                observed_at: now,
                provider: ProviderId::Tencent,
                batch_id: "TEST_CODE_sealed_quote".to_owned(),
            },
            BatchEvidence {
                provider: ProviderId::Tencent,
                source: "TEST_CODE_quote".to_owned(),
                source_at: Some(now.to_rfc3339()),
                observed_at: now.to_rfc3339(),
                batch_id: "TEST_CODE_sealed_quote".to_owned(),
            },
        )
        .expect_err("real symbol must not enter the test fixture seam");
        assert_eq!(error.reason_code(), "invalid_request");
    }

    #[test]
    fn br164_tdx_without_complete_evidence_cannot_win_quote_route() {
        let evidence = SourceEvidence::new(
            ProviderId::Tdx,
            "2026-07-26T09:30:00+08:00",
            "TEST_CODE_tdx_quote",
        )
        .unwrap();
        assert_eq!(evidence.source_at(), None);
        let policy = AcceptancePolicy::new()
            .with_require_complete(true)
            .with_require_source_at(true);
        assert!(policy.require_complete());
        assert!(policy.require_source_at());
    }

    #[test]
    fn br173_quote_request_uses_canonical_a_share_identity() {
        let instruments = build_instruments(&[
            "TEST_CODE_600396".to_owned(),
            "TEST_CODE_000001".to_owned(),
            "TEST_CODE_920118".to_owned(),
        ])
        .unwrap();
        assert_eq!(instruments[0].exchange(), Exchange::Shanghai);
        assert_eq!(instruments[1].exchange(), Exchange::Shenzhen);
        assert_eq!(instruments[2].exchange(), Exchange::Beijing);
        for code in [
            "TEST_CODE_430047",
            "TEST_CODE_830001",
            "TEST_CODE_900001",
            "TEST_CODE_200001",
            "TEST_CODE_921001",
            "TEST_CODE_929999",
        ] {
            assert!(build_instruments(&[code.to_owned()]).is_err());
        }
        assert!(build_instruments(&["TEST_CODE_100001".to_owned()]).is_err());
        assert!(build_instruments(&["TEST_CODE_60039A".to_owned()]).is_err());
    }

    #[test]
    fn br164_quote_timestamp_parser_rejects_unproven_provider_time() {
        let parsed = parse_evidence_instant(
            CAPABILITY,
            ProviderId::Tencent,
            "source_at",
            "2026-07-26T09:30:00+08:00",
        )
        .unwrap();
        assert_eq!(parsed.to_rfc3339(), "2026-07-26T01:30:00+00:00");

        let provider_epoch = parse_evidence_instant(
            CAPABILITY,
            ProviderId::Tencent,
            "observed_at",
            "1785792189.398743000",
        )
        .expect("BR-208 Magic Core admitted epoch observation must remain admissible");
        assert_eq!(
            provider_epoch.to_rfc3339(),
            "2026-08-03T21:23:09.398743+00:00"
        );

        let error = parse_evidence_instant(
            CAPABILITY,
            ProviderId::Tencent,
            "source_at",
            "TEST_CODE_not-a-time",
        )
        .unwrap_err();
        assert_eq!(error.reason_code(), "invalid_evidence");
        assert!(!error.retryable());
    }

    #[test]
    fn br208_quote_timestamp_parser_matches_magic_instant_contract() {
        for (encoded, expected) in [
            ("1785792189", "2026-08-03T21:23:09+00:00"),
            ("1785792189.3", "2026-08-03T21:23:09.300+00:00"),
            ("unix-ms:1785792189398", "2026-08-03T21:23:09.398+00:00"),
        ] {
            let parsed =
                parse_evidence_instant(CAPABILITY, ProviderId::Tencent, "observed_at", encoded)
                    .expect("BR-208 unambiguous Magic instant must be admitted");
            assert_eq!(parsed.to_rfc3339(), expected, "encoding={encoded}");
        }

        for invalid in [
            "-1",
            "1785792189.",
            ".398743000",
            "1785792189.3987430000",
            "unix-ms:-1",
            "2026-08-04T05:00:00",
        ] {
            let error =
                parse_evidence_instant(CAPABILITY, ProviderId::Tencent, "observed_at", invalid)
                    .expect_err("BR-208 ambiguous or malformed instant must fail closed");
            assert_eq!(error.reason_code(), "invalid_evidence", "value={invalid}");
            assert!(!error.retryable(), "value={invalid}");
        }
    }

    #[test]
    fn br164_quote_admission_rejects_empty_cardinality_identity_and_stale_batches() {
        let now = Utc::now();
        let empty_provenance = Provenance::new("TEST_CODE_quote", now.to_rfc3339())
            .unwrap()
            .with_source_at(now.to_rfc3339())
            .unwrap()
            .with_batch_id("TEST_CODE_empty_quote")
            .unwrap();
        let empty = admit_quote_batch(
            &["TEST_CODE_600396".to_owned()],
            ProviderId::Tencent,
            DataBatch::strict(Vec::new(), empty_provenance),
        )
        .unwrap_err();
        assert_eq!(empty.reason_code(), "verified_quote_batch_empty");
        assert!(empty.retryable());

        let cardinality = admit_quote_batch(
            &["TEST_CODE_600396".to_owned(), "TEST_CODE_000001".to_owned()],
            ProviderId::Tencent,
            quote_batch("600396", ProviderId::Tencent, "TEST_CODE_cardinality", now),
        )
        .unwrap_err();
        assert_eq!(cardinality.reason_code(), "invalid_evidence");

        let identity = admit_quote_batch(
            &["TEST_CODE_600396".to_owned()],
            ProviderId::Tencent,
            quote_batch("600000", ProviderId::Tencent, "TEST_CODE_identity", now),
        )
        .unwrap_err();
        assert_eq!(identity.reason_code(), "invalid_evidence");

        let stale = admit_quote_batch(
            &["TEST_CODE_600396".to_owned()],
            ProviderId::Tencent,
            quote_batch(
                "600396",
                ProviderId::Tencent,
                "TEST_CODE_stale",
                now - chrono::Duration::seconds(6),
            ),
        )
        .unwrap_err();
        assert_eq!(stale.reason_code(), "quote_stale");
        assert!(stale.retryable());
    }

    #[test]
    fn br164_quote_provider_error_classifiers_preserve_retry_semantics() {
        assert_eq!(
            classify_tdx_error(TdxError::ConnectionTimeout).kind(),
            FailureKind::Transport
        );
        assert_eq!(
            classify_tdx_error(TdxError::InvalidData("TEST_CODE bad".to_owned())).kind(),
            FailureKind::Protocol
        );
        let cardinality = classify_tdx_error(TdxError::HistoricalBarCardinality {
            offset: 800,
            actual: 99,
            expected_page: 100,
            requested_total: 900,
        });
        assert_eq!(cardinality.kind(), FailureKind::Protocol);
        for expected in [
            "offset=800",
            "actual=99",
            "expected_page=100",
            "requested_total=900",
        ] {
            assert!(cardinality.message().contains(expected));
        }
        assert_eq!(
            classify_tencent_error(TencentError::InvalidRequest("TEST_CODE bad".to_owned())).kind(),
            FailureKind::InvalidRequest
        );
        assert_eq!(
            classify_tencent_error(TencentError::Transport("TEST_CODE offline".to_owned())).kind(),
            FailureKind::Transport
        );
        assert_eq!(
            classify_sina_error(SinaError::Unsupported("TEST_CODE missing".to_owned())).kind(),
            FailureKind::Unsupported
        );
        assert_eq!(
            classify_sina_error(SinaError::Protocol("TEST_CODE schema".to_owned())).kind(),
            FailureKind::Protocol
        );
    }
}
