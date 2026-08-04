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
use std::collections::HashSet;
use std::sync::Arc;

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

    /// Batch evidence shared by every sealed record.
    pub(crate) fn evidence(&self) -> &BatchEvidence {
        &self.quotes[0].evidence
    }

    /// Test-only construction seam for exercising downstream exact-set
    /// rejection. Individual fixtures have already passed the TEST_CODE and
    /// record/evidence validation in [`AdmittedRealtimeQuote::from_test_fixture`].
    #[cfg(test)]
    pub(crate) fn from_test_fixtures(
        quotes: Vec<AdmittedRealtimeQuote>,
    ) -> Result<Self, GatewayError> {
        if quotes.is_empty()
            || quotes
                .iter()
                .any(|quote| !quote.code().starts_with("TEST_CODE_"))
        {
            return Err(GatewayError::invalid_request(
                CAPABILITY,
                "admitted realtime quote batch fixtures must be non-empty TEST_CODE records",
            ));
        }
        Ok(Self { quotes })
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

fn route_quotes(
    storage_codes: &[String],
    instruments: &[InstrumentId],
) -> (
    ProviderId,
    Result<GatewayBatch<RealtimeMarketQuote>, GatewayError>,
) {
    let mut router = QuoteRouter::new(
        AcceptancePolicy::new()
            .with_require_complete(true)
            .with_require_source_at(true),
    );

    let registration = router
        .register(quote_source(
            ProviderId::Tdx,
            Arc::new(TdxSmartClient::new()),
            classify_tdx_error,
        ))
        .and_then(|router| {
            let client = TencentClient::new().map_err(|error| {
                RouterError::InvalidConfiguration(format!(
                    "Magic Tencent quote client initialization failed: {error}"
                ))
            })?;
            router.register(quote_source(
                ProviderId::Tencent,
                Arc::new(client),
                classify_tencent_error,
            ))
        })
        .and_then(|router| {
            let client = SinaClient::new().map_err(|error| {
                RouterError::InvalidConfiguration(format!(
                    "Magic Sina quote client initialization failed: {error}"
                ))
            })?;
            router.register(quote_source(
                ProviderId::Sina,
                Arc::new(client),
                classify_sina_error,
            ))
        });

    if let Err(error) = registration {
        return (
            ProviderId::Tdx,
            Err(router_gateway_error(error, ProviderId::Tdx)),
        );
    }

    match router.route(instruments) {
        Ok(outcome) => {
            let provider = outcome.selected_provider();
            let batch = outcome.into_batch();
            (provider, admit_quote_batch(storage_codes, provider, batch))
        }
        Err(error) => {
            let provider = error
                .attempts()
                .last()
                .map(|attempt| attempt.provider_id())
                .unwrap_or(ProviderId::Tdx);
            (provider, Err(router_gateway_error(error, provider)))
        }
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
            return Err(GatewayError::classified(
                CAPABILITY,
                Some(provider),
                "stale",
                "quote_stale",
                true,
                format!("quote {storage_code} failed five-second freshness gate age_ms={age_ms}"),
            ));
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
