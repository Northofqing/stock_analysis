//! BR-164 evidence-preserving market capability gateways.
//!
//! Each route is ordered only by capabilities implemented at the pinned
//! `magic-market-data-rs` revision. A source can win only with an
//! identity-consistent batch that carries all evidence and fields required by
//! that consumer contract. Missing fields never become zeroes.

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, TimeZone, Timelike, Utc};
use magic_market_core::{
    AssetClass, BookLevel, DataBatch, DataStatus, Exchange, InstrumentId, MinuteDataRequest,
    MinutePoint, OrderBook, ProviderId, SecurityMetadata,
};
#[cfg(test)]
use magic_market_core::{Board, RatioUnit};
use magic_market_router::{
    minute_source, order_book_source, security_metadata_source, AcceptancePolicy, AttemptStatus,
    FailureKind, MinuteRouter, OrderBookRouter, RouterError, SecurityMetadataRouter, SourceError,
};
use magic_sina_rs::{SinaClient, SinaError};
use magic_tdx_rs::{TdxError, TdxSmartClient};
use magic_tencent_rs::{TencentClient, TencentError};
use std::collections::HashSet;
use std::sync::Arc;

use super::instrument_identity::{resolve_production_equity, EquitySegment};
use super::review::{
    acquisition_request_hash, audit_blocking_join_failure, audit_gateway_result, BatchEvidence,
    GatewayBatch, GatewayError,
};

const MINUTE_CAPABILITY: &str = "MarketMinuteData";
const ORDER_BOOK_CAPABILITY: &str = "MarketOrderBooks";
const MONEY_FLOW_CAPABILITY: &str = "MarketMoneyFlows";
const METADATA_CAPABILITY: &str = "SecurityMetadata";
const SECURITY_IDENTITY_CAPABILITY: &str = "SecurityIdentity";
const REALTIME_MAX_AGE_MILLIS: i64 = 5_000;
const ACQUISITION_MAX_AGE_MILLIS: i64 = 30_000;
const SHANGHAI_OFFSET_SECONDS: i32 = 8 * 60 * 60;

/// Actual upstream source order for current and historical minute data.
pub const MINUTE_PROVIDER_ORDER: &[ProviderId] =
    &[ProviderId::Tdx, ProviderId::Tencent, ProviderId::Sina];
/// Actual upstream source order for five-level order books.
///
/// Pinned TDX currently lacks an auditable source timestamp, so strict routing
/// rejects its batch and continues instead of pretending it is current.
pub const ORDER_BOOK_PROVIDER_ORDER: &[ProviderId] =
    &[ProviderId::Tdx, ProviderId::Tencent, ProviderId::Sina];
/// The only implemented normalized money-flow provider in the upstream
/// workspace is the separately licensed EMQuant adapter (Eastmoney identity).
pub const MONEY_FLOW_PROVIDER_ORDER: &[ProviderId] = &[ProviderId::Eastmoney];
/// Actual upstream source order for the source-backed security identity
/// subset (name, ST label and source evidence).
///
/// TDX is intentionally absent because its list packet has no source
/// timestamp. None of these providers is advertised as complete
/// security-master data.
pub const METADATA_PROVIDER_ORDER: &[ProviderId] = &[ProviderId::Tencent, ProviderId::Sina];

/// One admitted minute point. `cumulative_amount` remains optional because the
/// TDX protocol does not provide it; absence is preserved rather than filled.
#[derive(Debug, Clone, PartialEq)]
pub struct MarketMinutePoint {
    pub code: String,
    pub minute_at: DateTime<Utc>,
    pub price: f64,
    pub cumulative_quantity: f64,
    pub cumulative_amount: Option<f64>,
    pub source_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    pub provider: ProviderId,
    pub batch_id: String,
}

/// One complete order-book level.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarketBookLevel {
    pub price: f64,
    pub quantity: f64,
}

/// One admitted five-level order book.
#[derive(Debug, Clone, PartialEq)]
pub struct MarketOrderBook {
    pub code: String,
    pub bids: [MarketBookLevel; 5],
    pub asks: [MarketBookLevel; 5],
    pub total_bid_quantity: f64,
    pub total_ask_quantity: f64,
    pub source_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    pub provider: ProviderId,
    pub batch_id: String,
}

/// One admitted normalized money-flow snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct MarketMoneyFlow {
    pub code: String,
    pub main_net: f64,
    pub super_large_net: f64,
    pub large_net: f64,
    pub medium_net: f64,
    pub small_net: f64,
    pub source_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    pub provider: ProviderId,
    pub batch_id: String,
}

/// Stable consumer-side board vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityBoard {
    Main,
    Star,
    ChiNext,
    Beijing,
}

/// One admitted complete security-master record.
#[derive(Debug, Clone, PartialEq)]
pub struct MarketSecurityMetadata {
    pub code: String,
    pub name: String,
    pub board: SecurityBoard,
    pub is_st: bool,
    pub listed_on: NaiveDate,
    pub price_limit_percent: f64,
    pub price_limit_version: String,
    pub source_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    pub provider: ProviderId,
    pub batch_id: String,
}

/// Source-backed identity subset used by watchlist admission and delisting
/// name checks. This intentionally does not claim that listing date, board or
/// price-limit metadata is available.
#[derive(Debug, Clone, PartialEq)]
pub struct MarketSecurityIdentity {
    pub code: String,
    pub name: String,
    pub is_st: bool,
    pub source_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    pub provider: ProviderId,
    pub batch_id: String,
}

/// Unified entry point for BR-164 market capability acquisition.
#[derive(Debug, Clone, Copy, Default)]
pub struct MarketCapabilitiesGateway;

impl MarketCapabilitiesGateway {
    pub const fn new() -> Self {
        Self
    }

    /// Fetches the current session (`date=None`) or one explicit historical
    /// session (`date=Some`) without blocking an async runtime worker.
    pub async fn minute_data(
        &self,
        code: &str,
        date: Option<NaiveDate>,
    ) -> Result<GatewayBatch<MarketMinutePoint>, GatewayError> {
        let storage_code = code.to_owned();
        let canonical = format!(
            "{storage_code}:{}",
            date.map(|value| value.to_string())
                .unwrap_or_else(|| "current".to_owned())
        );
        let request_hash = acquisition_request_hash(MINUTE_CAPABILITY, &canonical);
        let worker_hash = request_hash.clone();
        let joined = tokio::task::spawn_blocking(move || {
            let result = build_instrument(&storage_code, MINUTE_CAPABILITY)
                .and_then(|instrument| build_minute_request(instrument, date))
                .map(|request| route_minute(&storage_code, &request));
            let (provider, result) = match result {
                Ok(routed) => routed,
                Err(error) => (ProviderId::Tdx, Err(error)),
            };
            audit_gateway_result(MINUTE_CAPABILITY, provider, &worker_hash, result)
        })
        .await;
        match joined {
            Ok(result) => result,
            Err(error) => {
                audit_blocking_join_failure(
                    MINUTE_CAPABILITY,
                    ProviderId::Tdx,
                    request_hash,
                    error.to_string(),
                )
                .await
            }
        }
    }

    /// Fetches complete, current five-level books for every requested code.
    pub async fn order_books(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<MarketOrderBook>, GatewayError> {
        let storage_codes = codes.to_vec();
        let request_hash =
            acquisition_request_hash(ORDER_BOOK_CAPABILITY, &storage_codes.join(","));
        let worker_hash = request_hash.clone();
        let joined = tokio::task::spawn_blocking(move || {
            let result = build_instruments(&storage_codes, ORDER_BOOK_CAPABILITY)
                .map(|instruments| route_order_books(&storage_codes, &instruments));
            let (provider, result) = match result {
                Ok(routed) => routed,
                Err(error) => (ProviderId::Tdx, Err(error)),
            };
            audit_gateway_result(ORDER_BOOK_CAPABILITY, provider, &worker_hash, result)
        })
        .await;
        match joined {
            Ok(result) => result,
            Err(error) => {
                audit_blocking_join_failure(
                    ORDER_BOOK_CAPABILITY,
                    ProviderId::Tdx,
                    request_hash,
                    error.to_string(),
                )
                .await
            }
        }
    }

    /// Returns an explicit contract error until the separately licensed
    /// `magic-emquant-rs` provider is wired at the dependency boundary.
    ///
    /// TDX is deliberately not treated as a money-flow source: upstream marks
    /// this capability unsupported because its packets do not prove the
    /// standardized main/net-flow methodology.
    pub async fn money_flows(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<MarketMoneyFlow>, GatewayError> {
        let storage_codes = codes.to_vec();
        let request_hash =
            acquisition_request_hash(MONEY_FLOW_CAPABILITY, &storage_codes.join(","));
        let worker_hash = request_hash.clone();
        let joined = tokio::task::spawn_blocking(move || {
            let result = build_instruments(&storage_codes, MONEY_FLOW_CAPABILITY).and_then(|_| {
                Err(GatewayError::classified(
                    MONEY_FLOW_CAPABILITY,
                    Some(ProviderId::Eastmoney),
                    "unsupported",
                    "unsupported_contract",
                    false,
                    "pinned upstream implements normalized MoneyFlows only through \
                     magic-emquant-rs; that licensed bridge is not linked into this binary",
                ))
            });
            audit_gateway_result(
                MONEY_FLOW_CAPABILITY,
                ProviderId::Eastmoney,
                &worker_hash,
                result,
            )
        })
        .await;
        match joined {
            Ok(result) => result,
            Err(error) => {
                audit_blocking_join_failure(
                    MONEY_FLOW_CAPABILITY,
                    ProviderId::Eastmoney,
                    request_hash,
                    error.to_string(),
                )
                .await
            }
        }
    }

    /// Returns a typed non-retryable unsupported error because the pinned
    /// providers do not prove the complete security-master contract.
    pub async fn security_metadata(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<MarketSecurityMetadata>, GatewayError> {
        let storage_codes = codes.to_vec();
        let request_hash = acquisition_request_hash(METADATA_CAPABILITY, &storage_codes.join(","));
        let worker_hash = request_hash.clone();
        let joined = tokio::task::spawn_blocking(move || {
            let result = unsupported_security_metadata(&storage_codes);
            audit_gateway_result(METADATA_CAPABILITY, ProviderId::Tdx, &worker_hash, result)
        })
        .await;
        match joined {
            Ok(result) => result,
            Err(error) => {
                audit_blocking_join_failure(
                    METADATA_CAPABILITY,
                    ProviderId::Tdx,
                    request_hash,
                    error.to_string(),
                )
                .await
            }
        }
    }

    /// Fetches only the source-backed security identity subset needed by
    /// watchlist admission and delisting-name checks.
    pub async fn security_identities(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<MarketSecurityIdentity>, GatewayError> {
        let storage_codes = codes.to_vec();
        let request_hash =
            acquisition_request_hash(SECURITY_IDENTITY_CAPABILITY, &storage_codes.join(","));
        let worker_hash = request_hash.clone();
        let joined = tokio::task::spawn_blocking(move || {
            let result = build_instruments(&storage_codes, SECURITY_IDENTITY_CAPABILITY)
                .map(|instruments| route_security_identities(&storage_codes, &instruments));
            let (provider, result) = match result {
                Ok(routed) => routed,
                Err(error) => (ProviderId::Tencent, Err(error)),
            };
            audit_gateway_result(SECURITY_IDENTITY_CAPABILITY, provider, &worker_hash, result)
        })
        .await;
        match joined {
            Ok(result) => result,
            Err(error) => {
                audit_blocking_join_failure(
                    SECURITY_IDENTITY_CAPABILITY,
                    ProviderId::Tencent,
                    request_hash,
                    error.to_string(),
                )
                .await
            }
        }
    }
}

fn unsupported_security_metadata(
    storage_codes: &[String],
) -> Result<GatewayBatch<MarketSecurityMetadata>, GatewayError> {
    build_instruments(storage_codes, METADATA_CAPABILITY).and_then(|_| {
        Err(GatewayError::classified(
            METADATA_CAPABILITY,
            Some(ProviderId::Tdx),
            "unsupported",
            "unsupported_contract",
            false,
            "pinned TDX/Tencent/Sina providers do not prove listing date and \
             versioned price-limit fields required by complete security metadata",
        ))
    })
}

fn build_minute_request(
    instrument: InstrumentId,
    date: Option<NaiveDate>,
) -> Result<MinuteDataRequest, GatewayError> {
    let request = MinuteDataRequest::new(instrument);
    match date {
        Some(date) => request
            .with_date(date.format("%Y-%m-%d").to_string())
            .map_err(|error| GatewayError::invalid_request(MINUTE_CAPABILITY, error.to_string())),
        None => Ok(request),
    }
}

fn build_instruments(
    codes: &[String],
    capability: &'static str,
) -> Result<Vec<InstrumentId>, GatewayError> {
    if codes.is_empty() {
        return Err(GatewayError::invalid_request(
            capability,
            "request must contain at least one A-share code",
        ));
    }
    let mut seen = HashSet::with_capacity(codes.len());
    codes
        .iter()
        .map(|code| {
            if !seen.insert(code.as_str()) {
                return Err(GatewayError::invalid_request(
                    capability,
                    format!("duplicate A-share code {code:?}"),
                ));
            }
            build_instrument(code, capability)
        })
        .collect()
}

fn build_instrument(
    storage_code: &str,
    capability: &'static str,
) -> Result<InstrumentId, GatewayError> {
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
                capability,
                format!("invalid market-capability equity identity {storage_code:?}: {error}"),
            )
        })?;
    if identity.segment() == EquitySegment::BeijingA
        && !identity.canonical_code().starts_with("920")
    {
        return Err(GatewayError::invalid_request(
            capability,
            format!(
                "market providers have no verified {capability} capability for {storage_code:?}"
            ),
        ));
    }
    InstrumentId::new(
        identity.exchange(),
        identity.canonical_code(),
        AssetClass::Equity,
    )
    .map_err(|error| {
        GatewayError::invalid_request(
            capability,
            format!("validated instrument {storage_code:?} failed core invariant: {error}"),
        )
    })
}

fn route_minute(
    storage_code: &str,
    request: &MinuteDataRequest,
) -> (
    ProviderId,
    Result<GatewayBatch<MarketMinutePoint>, GatewayError>,
) {
    let mut router = MinuteRouter::new(strict_policy());
    let registration = router
        .register(minute_source(
            ProviderId::Tdx,
            Arc::new(TdxSmartClient::new()),
            classify_tdx_error,
        ))
        .and_then(|router| {
            let client = TencentClient::new().map_err(|error| {
                RouterError::InvalidConfiguration(format!(
                    "Magic Tencent minute client initialization failed: {error}"
                ))
            })?;
            router.register(minute_source(
                ProviderId::Tencent,
                Arc::new(client),
                classify_tencent_error,
            ))
        })
        .and_then(|router| {
            let client = SinaClient::new().map_err(|error| {
                RouterError::InvalidConfiguration(format!(
                    "Magic Sina minute client initialization failed: {error}"
                ))
            })?;
            router.register(minute_source(
                ProviderId::Sina,
                Arc::new(client),
                classify_sina_error,
            ))
        });
    if let Err(error) = registration {
        return (
            ProviderId::Tdx,
            Err(router_gateway_error(
                MINUTE_CAPABILITY,
                error,
                ProviderId::Tdx,
            )),
        );
    }
    match router.route(request) {
        Ok(outcome) => {
            let provider = outcome.selected_provider();
            let batch = outcome.into_batch();
            // BR-164: validation time is captured after the blocking provider
            // route completes. Capturing it before network acquisition makes
            // the provider's real observed_at appear to be in the future.
            let now = Utc::now();
            (
                provider,
                admit_minute_batch(storage_code, request, provider, batch, now),
            )
        }
        Err(error) => {
            let provider = terminal_provider(&error, ProviderId::Tdx);
            (
                provider,
                Err(router_gateway_error(MINUTE_CAPABILITY, error, provider)),
            )
        }
    }
}

fn route_order_books(
    storage_codes: &[String],
    instruments: &[InstrumentId],
) -> (
    ProviderId,
    Result<GatewayBatch<MarketOrderBook>, GatewayError>,
) {
    let mut router = OrderBookRouter::new(strict_policy());
    let registration = router
        .register(order_book_source(
            ProviderId::Tdx,
            Arc::new(TdxSmartClient::new()),
            classify_tdx_error,
        ))
        .and_then(|router| {
            let client = TencentClient::new().map_err(|error| {
                RouterError::InvalidConfiguration(format!(
                    "Magic Tencent order-book client initialization failed: {error}"
                ))
            })?;
            router.register(order_book_source(
                ProviderId::Tencent,
                Arc::new(client),
                classify_tencent_error,
            ))
        })
        .and_then(|router| {
            let client = SinaClient::new().map_err(|error| {
                RouterError::InvalidConfiguration(format!(
                    "Magic Sina order-book client initialization failed: {error}"
                ))
            })?;
            router.register(order_book_source(
                ProviderId::Sina,
                Arc::new(client),
                classify_sina_error,
            ))
        });
    if let Err(error) = registration {
        return (
            ProviderId::Tdx,
            Err(router_gateway_error(
                ORDER_BOOK_CAPABILITY,
                error,
                ProviderId::Tdx,
            )),
        );
    }
    match router.route(instruments) {
        Ok(outcome) => {
            let provider = outcome.selected_provider();
            let batch = outcome.into_batch();
            let now = Utc::now();
            (
                provider,
                admit_order_book_batch(storage_codes, instruments, provider, batch, now),
            )
        }
        Err(error) => {
            let provider = terminal_provider(&error, ProviderId::Tdx);
            (
                provider,
                Err(router_gateway_error(ORDER_BOOK_CAPABILITY, error, provider)),
            )
        }
    }
}

fn route_security_identities(
    storage_codes: &[String],
    instruments: &[InstrumentId],
) -> (
    ProviderId,
    Result<GatewayBatch<MarketSecurityIdentity>, GatewayError>,
) {
    let mut router = SecurityMetadataRouter::new(security_identity_policy());
    let registration = TencentClient::new()
        .map_err(|error| {
            RouterError::InvalidConfiguration(format!(
                "Magic Tencent identity client initialization failed: {error}"
            ))
        })
        .and_then(|client| {
            router.register(security_metadata_source(
                ProviderId::Tencent,
                Arc::new(client),
                classify_tencent_error,
            ))
        })
        .and_then(|router| {
            let client = SinaClient::new().map_err(|error| {
                RouterError::InvalidConfiguration(format!(
                    "Magic Sina identity client initialization failed: {error}"
                ))
            })?;
            router.register(security_metadata_source(
                ProviderId::Sina,
                Arc::new(client),
                classify_sina_error,
            ))
        });
    if let Err(error) = registration {
        return (
            ProviderId::Tencent,
            Err(router_gateway_error(
                SECURITY_IDENTITY_CAPABILITY,
                error,
                ProviderId::Tencent,
            )),
        );
    }
    match router.route(instruments) {
        Ok(outcome) => {
            let provider = outcome.selected_provider();
            let batch = outcome.into_batch();
            let now = Utc::now();
            (
                provider,
                admit_security_identity_batch(storage_codes, instruments, provider, batch, now),
            )
        }
        Err(error) => {
            let provider = terminal_provider(&error, ProviderId::Tencent);
            (
                provider,
                Err(router_gateway_error(
                    SECURITY_IDENTITY_CAPABILITY,
                    error,
                    provider,
                )),
            )
        }
    }
}

fn strict_policy() -> AcceptancePolicy {
    AcceptancePolicy::new()
        .with_require_complete(true)
        .with_require_source_at(true)
}

fn security_identity_policy() -> AcceptancePolicy {
    AcceptancePolicy::new()
        .with_require_complete(false)
        .with_require_source_at(true)
}

fn admit_minute_batch(
    storage_code: &str,
    request: &MinuteDataRequest,
    provider: ProviderId,
    batch: DataBatch<MinutePoint>,
    now: DateTime<Utc>,
) -> Result<GatewayBatch<MarketMinutePoint>, GatewayError> {
    let (evidence, observed_at, batch_source_at) =
        validate_batch_evidence(MINUTE_CAPABILITY, provider, &batch)?;
    validate_observation_time(
        MINUTE_CAPABILITY,
        provider,
        observed_at,
        now,
        ACQUISITION_MAX_AGE_MILLIS,
    )?;
    if batch.records().is_empty() {
        return Err(GatewayError::classified(
            MINUTE_CAPABILITY,
            Some(provider),
            "unavailable",
            "verified_minute_batch_empty",
            true,
            "minute provider returned no source points",
        ));
    }

    let mut projected = Vec::with_capacity(batch.records().len());
    let mut previous: Option<(DateTime<Utc>, f64, f64)> = None;
    for point in batch.records() {
        if point.instrument() != request.instrument()
            || point.provider() != provider
            || point.batch_id() != evidence.batch_id
            || point.observed_at() != evidence.observed_at
            || point.status() != DataStatus::Available
        {
            return Err(GatewayError::invalid_evidence(
                MINUTE_CAPABILITY,
                Some(provider),
                format!("minute identity/evidence mismatch for {storage_code}"),
            ));
        }
        let minute_at = parse_minute_at(point.minute_at(), provider)?;
        let source_at = parse_required_record_time(
            MINUTE_CAPABILITY,
            provider,
            point.source_at(),
            storage_code,
        )?;
        if minute_at != source_at {
            return Err(GatewayError::invalid_evidence(
                MINUTE_CAPABILITY,
                Some(provider),
                format!(
                    "minute domain/source timestamps differ for {storage_code}: \
                     minute_at={} source_at={}",
                    minute_at.to_rfc3339(),
                    source_at.to_rfc3339()
                ),
            ));
        }
        if let Some(requested_date) = request.date() {
            let actual_date = point.minute_at().get(..10).ok_or_else(|| {
                GatewayError::invalid_evidence(
                    MINUTE_CAPABILITY,
                    Some(provider),
                    "minute timestamp has no source date",
                )
            })?;
            if actual_date != requested_date {
                return Err(GatewayError::invalid_evidence(
                    MINUTE_CAPABILITY,
                    Some(provider),
                    format!(
                        "minute source date {actual_date} differs from requested {requested_date}"
                    ),
                ));
            }
        }

        let price = point.price().get();
        let quantity = point.cumulative_quantity().get();
        if let Some((previous_at, previous_price, previous_quantity)) = previous {
            validate_minute_continuity(request.instrument(), provider, previous_at, minute_at)?;
            if quantity < previous_quantity {
                return Err(GatewayError::invalid_evidence(
                    MINUTE_CAPABILITY,
                    Some(provider),
                    format!("minute cumulative quantity regressed for {storage_code}"),
                ));
            }
            let change_percent = (price / previous_price - 1.0) * 100.0;
            if change_percent.abs() > 20.0 {
                return Err(GatewayError::classified(
                    MINUTE_CAPABILITY,
                    Some(provider),
                    "partial",
                    "adjacent_price_change_requires_confirmation",
                    false,
                    format!(
                        "minute adjacent price change for {storage_code} is \
                         {change_percent:.4}%, exceeding ±20%"
                    ),
                ));
            }
        }
        previous = Some((minute_at, price, quantity));
        projected.push(MarketMinutePoint {
            code: storage_code.to_owned(),
            minute_at,
            price,
            cumulative_quantity: quantity,
            cumulative_amount: point.cumulative_amount().map(|value| value.get()),
            source_at,
            observed_at,
            provider,
            batch_id: point.batch_id().to_owned(),
        });
    }

    let latest_source_at = projected
        .last()
        .map(|point| point.source_at)
        .ok_or_else(|| {
            GatewayError::invalid_evidence(
                MINUTE_CAPABILITY,
                Some(provider),
                "minute batch unexpectedly became empty",
            )
        })?;
    if latest_source_at != batch_source_at {
        return Err(GatewayError::invalid_evidence(
            MINUTE_CAPABILITY,
            Some(provider),
            "minute batch source time differs from the latest record",
        ));
    }
    if request.date().is_none() {
        validate_realtime_freshness(MINUTE_CAPABILITY, provider, latest_source_at, now, "minute")?;
    }
    Ok(GatewayBatch::Available {
        records: projected,
        evidence,
    })
}

fn admit_order_book_batch(
    storage_codes: &[String],
    instruments: &[InstrumentId],
    provider: ProviderId,
    batch: DataBatch<OrderBook>,
    now: DateTime<Utc>,
) -> Result<GatewayBatch<MarketOrderBook>, GatewayError> {
    let (evidence, observed_at, batch_source_at) =
        validate_batch_evidence(ORDER_BOOK_CAPABILITY, provider, &batch)?;
    validate_observation_time(
        ORDER_BOOK_CAPABILITY,
        provider,
        observed_at,
        now,
        REALTIME_MAX_AGE_MILLIS,
    )?;
    if batch.records().len() != instruments.len() {
        return Err(GatewayError::invalid_evidence(
            ORDER_BOOK_CAPABILITY,
            Some(provider),
            format!(
                "order-book cardinality mismatch requested={} actual={}",
                instruments.len(),
                batch.records().len()
            ),
        ));
    }

    let mut projected = Vec::with_capacity(batch.records().len());
    for ((storage_code, instrument), book) in
        storage_codes.iter().zip(instruments).zip(batch.records())
    {
        if book.instrument() != instrument
            || book.provider() != provider
            || book.batch_id() != evidence.batch_id
            || book.observed_at() != evidence.observed_at
            || book.status() != DataStatus::Available
        {
            return Err(GatewayError::invalid_evidence(
                ORDER_BOOK_CAPABILITY,
                Some(provider),
                format!("order-book identity/evidence mismatch for {storage_code}"),
            ));
        }
        let source_at = parse_required_record_time(
            ORDER_BOOK_CAPABILITY,
            provider,
            book.source_at(),
            storage_code,
        )?;
        if source_at != batch_source_at {
            return Err(GatewayError::invalid_evidence(
                ORDER_BOOK_CAPABILITY,
                Some(provider),
                format!("order-book source time differs within batch for {storage_code}"),
            ));
        }
        validate_realtime_freshness(
            ORDER_BOOK_CAPABILITY,
            provider,
            source_at,
            now,
            storage_code,
        )?;
        let bids = project_book_levels(book.bids(), provider, storage_code, "bid")?;
        let asks = project_book_levels(book.asks(), provider, storage_code, "ask")?;
        validate_book_order(provider, storage_code, &bids, &asks)?;
        let total_bid_quantity = book
            .total_bid_quantity()
            .ok_or_else(|| {
                GatewayError::invalid_evidence(
                    ORDER_BOOK_CAPABILITY,
                    Some(provider),
                    format!("order-book total bid quantity unavailable for {storage_code}"),
                )
            })?
            .get();
        let total_ask_quantity = book
            .total_ask_quantity()
            .ok_or_else(|| {
                GatewayError::invalid_evidence(
                    ORDER_BOOK_CAPABILITY,
                    Some(provider),
                    format!("order-book total ask quantity unavailable for {storage_code}"),
                )
            })?
            .get();
        projected.push(MarketOrderBook {
            code: storage_code.clone(),
            bids,
            asks,
            total_bid_quantity,
            total_ask_quantity,
            source_at,
            observed_at,
            provider,
            batch_id: book.batch_id().to_owned(),
        });
    }
    Ok(GatewayBatch::Available {
        records: projected,
        evidence,
    })
}

#[cfg(test)]
fn admit_money_flow_batch(
    storage_codes: &[String],
    instruments: &[InstrumentId],
    provider: ProviderId,
    batch: DataBatch<magic_market_core::MoneyFlow>,
    now: DateTime<Utc>,
) -> Result<GatewayBatch<MarketMoneyFlow>, GatewayError> {
    let (evidence, observed_at, batch_source_at) =
        validate_batch_evidence(MONEY_FLOW_CAPABILITY, provider, &batch)?;
    validate_observation_time(
        MONEY_FLOW_CAPABILITY,
        provider,
        observed_at,
        now,
        ACQUISITION_MAX_AGE_MILLIS,
    )?;
    validate_daily_freshness(MONEY_FLOW_CAPABILITY, provider, batch_source_at, now)?;
    if batch.records().len() != instruments.len() {
        return Err(GatewayError::invalid_evidence(
            MONEY_FLOW_CAPABILITY,
            Some(provider),
            format!(
                "money-flow cardinality mismatch requested={} actual={}",
                instruments.len(),
                batch.records().len()
            ),
        ));
    }

    let mut projected = Vec::with_capacity(batch.records().len());
    for ((storage_code, instrument), flow) in
        storage_codes.iter().zip(instruments).zip(batch.records())
    {
        if flow.instrument() != instrument
            || flow.provider() != provider
            || flow.batch_id() != evidence.batch_id
            || flow.observed_at() != evidence.observed_at
            || flow.status() != DataStatus::Available
        {
            return Err(GatewayError::invalid_evidence(
                MONEY_FLOW_CAPABILITY,
                Some(provider),
                format!("money-flow identity/evidence mismatch for {storage_code}"),
            ));
        }
        let source_at = parse_required_record_time(
            MONEY_FLOW_CAPABILITY,
            provider,
            flow.source_at(),
            storage_code,
        )?;
        if source_at != batch_source_at {
            return Err(GatewayError::invalid_evidence(
                MONEY_FLOW_CAPABILITY,
                Some(provider),
                format!("money-flow source time differs within batch for {storage_code}"),
            ));
        }
        let main_net = required_money(flow.main_net(), provider, storage_code, "main_net")?;
        let super_large_net = required_money(
            flow.super_large_net(),
            provider,
            storage_code,
            "super_large_net",
        )?;
        let large_net = required_money(flow.large_net(), provider, storage_code, "large_net")?;
        let medium_net = required_money(flow.medium_net(), provider, storage_code, "medium_net")?;
        let small_net = required_money(flow.small_net(), provider, storage_code, "small_net")?;
        let composed_main = super_large_net + large_net;
        let tolerance = composed_main.abs().max(1.0) * f64::EPSILON * 32.0;
        if (main_net - composed_main).abs() > tolerance {
            return Err(GatewayError::invalid_evidence(
                MONEY_FLOW_CAPABILITY,
                Some(provider),
                format!(
                    "money-flow main_net differs from super_large_net + large_net \
                     for {storage_code}"
                ),
            ));
        }
        projected.push(MarketMoneyFlow {
            code: storage_code.clone(),
            main_net,
            super_large_net,
            large_net,
            medium_net,
            small_net,
            source_at,
            observed_at,
            provider,
            batch_id: flow.batch_id().to_owned(),
        });
    }
    Ok(GatewayBatch::Available {
        records: projected,
        evidence,
    })
}

fn admit_security_identity_batch(
    storage_codes: &[String],
    instruments: &[InstrumentId],
    provider: ProviderId,
    batch: DataBatch<SecurityMetadata>,
    now: DateTime<Utc>,
) -> Result<GatewayBatch<MarketSecurityIdentity>, GatewayError> {
    let evidence = BatchEvidence::from_provenance(provider, batch.provenance())?;
    let observed_at = parse_provider_timestamp(
        SECURITY_IDENTITY_CAPABILITY,
        provider,
        &evidence.observed_at,
        "observed_at",
    )?;
    let batch_source_at = evidence.source_at.as_deref().ok_or_else(|| {
        GatewayError::invalid_evidence(
            SECURITY_IDENTITY_CAPABILITY,
            Some(provider),
            "security identity batch provenance has no source timestamp",
        )
    })?;
    let batch_source_at = parse_provider_timestamp(
        SECURITY_IDENTITY_CAPABILITY,
        provider,
        batch_source_at,
        "source_at",
    )?;
    if batch_source_at > observed_at {
        return Err(GatewayError::invalid_evidence(
            SECURITY_IDENTITY_CAPABILITY,
            Some(provider),
            "security identity source timestamp is after observation timestamp",
        ));
    }
    validate_observation_time(
        SECURITY_IDENTITY_CAPABILITY,
        provider,
        observed_at,
        now,
        ACQUISITION_MAX_AGE_MILLIS,
    )?;
    validate_daily_freshness(SECURITY_IDENTITY_CAPABILITY, provider, batch_source_at, now)?;
    if batch.records().len() != instruments.len() {
        return Err(GatewayError::invalid_evidence(
            SECURITY_IDENTITY_CAPABILITY,
            Some(provider),
            format!(
                "security identity cardinality mismatch requested={} actual={}",
                instruments.len(),
                batch.records().len()
            ),
        ));
    }

    let mut projected = Vec::with_capacity(batch.records().len());
    for ((storage_code, instrument), metadata) in
        storage_codes.iter().zip(instruments).zip(batch.records())
    {
        if metadata.instrument() != instrument
            || metadata.provider() != provider
            || metadata.batch_id() != evidence.batch_id
            || metadata.observed_at() != evidence.observed_at
            || matches!(
                metadata.status(),
                DataStatus::Stale | DataStatus::Conflicted | DataStatus::Unsupported
            )
        {
            return Err(GatewayError::invalid_evidence(
                SECURITY_IDENTITY_CAPABILITY,
                Some(provider),
                format!("security identity/evidence mismatch for {storage_code}"),
            ));
        }
        let source_at = parse_required_record_time(
            SECURITY_IDENTITY_CAPABILITY,
            provider,
            metadata.source_at(),
            storage_code,
        )?;
        if source_at < batch_source_at || source_at > observed_at {
            return Err(GatewayError::invalid_evidence(
                SECURITY_IDENTITY_CAPABILITY,
                Some(provider),
                format!(
                    "security identity record time is outside batch evidence for {storage_code}"
                ),
            ));
        }
        validate_daily_freshness(SECURITY_IDENTITY_CAPABILITY, provider, source_at, now)?;
        let name = metadata
            .name()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                GatewayError::invalid_evidence(
                    SECURITY_IDENTITY_CAPABILITY,
                    Some(provider),
                    format!("security identity name unavailable for {storage_code}"),
                )
            })?
            .to_owned();
        let is_st = metadata.is_st().ok_or_else(|| {
            GatewayError::invalid_evidence(
                SECURITY_IDENTITY_CAPABILITY,
                Some(provider),
                format!("security identity ST label unavailable for {storage_code}"),
            )
        })?;
        projected.push(MarketSecurityIdentity {
            code: storage_code.clone(),
            name,
            is_st,
            source_at,
            observed_at,
            provider,
            batch_id: metadata.batch_id().to_owned(),
        });
    }
    Ok(GatewayBatch::Available {
        records: projected,
        evidence,
    })
}

#[cfg(test)]
fn admit_metadata_batch(
    storage_codes: &[String],
    instruments: &[InstrumentId],
    provider: ProviderId,
    batch: DataBatch<SecurityMetadata>,
    now: DateTime<Utc>,
) -> Result<GatewayBatch<MarketSecurityMetadata>, GatewayError> {
    let (evidence, observed_at, batch_source_at) =
        validate_batch_evidence(METADATA_CAPABILITY, provider, &batch)?;
    validate_observation_time(
        METADATA_CAPABILITY,
        provider,
        observed_at,
        now,
        ACQUISITION_MAX_AGE_MILLIS,
    )?;
    validate_daily_freshness(METADATA_CAPABILITY, provider, batch_source_at, now)?;
    if batch.records().len() != instruments.len() {
        return Err(GatewayError::invalid_evidence(
            METADATA_CAPABILITY,
            Some(provider),
            format!(
                "metadata cardinality mismatch requested={} actual={}",
                instruments.len(),
                batch.records().len()
            ),
        ));
    }

    let mut projected = Vec::with_capacity(batch.records().len());
    for ((storage_code, instrument), metadata) in
        storage_codes.iter().zip(instruments).zip(batch.records())
    {
        if metadata.instrument() != instrument
            || metadata.provider() != provider
            || metadata.batch_id() != evidence.batch_id
            || metadata.observed_at() != evidence.observed_at
            || metadata.status() != DataStatus::Available
        {
            return Err(GatewayError::invalid_evidence(
                METADATA_CAPABILITY,
                Some(provider),
                format!("metadata identity/evidence mismatch for {storage_code}"),
            ));
        }
        let source_at = parse_required_record_time(
            METADATA_CAPABILITY,
            provider,
            metadata.source_at(),
            storage_code,
        )?;
        if source_at != batch_source_at {
            return Err(GatewayError::invalid_evidence(
                METADATA_CAPABILITY,
                Some(provider),
                format!("metadata source time differs within batch for {storage_code}"),
            ));
        }
        let name = metadata
            .name()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                GatewayError::invalid_evidence(
                    METADATA_CAPABILITY,
                    Some(provider),
                    format!("security name unavailable for {storage_code}"),
                )
            })?
            .to_owned();
        let board = metadata.board().and_then(project_board).ok_or_else(|| {
            GatewayError::invalid_evidence(
                METADATA_CAPABILITY,
                Some(provider),
                format!("security board unavailable for {storage_code}"),
            )
        })?;
        let is_st = metadata.is_st().ok_or_else(|| {
            GatewayError::invalid_evidence(
                METADATA_CAPABILITY,
                Some(provider),
                format!("security ST flag unavailable for {storage_code}"),
            )
        })?;
        let listed_on_text = metadata.listed_on().ok_or_else(|| {
            GatewayError::invalid_evidence(
                METADATA_CAPABILITY,
                Some(provider),
                format!("security listing date unavailable for {storage_code}"),
            )
        })?;
        let listed_on = NaiveDate::parse_from_str(listed_on_text, "%Y-%m-%d").map_err(|error| {
            GatewayError::invalid_evidence(
                METADATA_CAPABILITY,
                Some(provider),
                format!("invalid listing date {listed_on_text:?}: {error}"),
            )
        })?;
        if listed_on > source_at.with_timezone(&shanghai_offset()).date_naive() {
            return Err(GatewayError::invalid_evidence(
                METADATA_CAPABILITY,
                Some(provider),
                format!("listing date is after source time for {storage_code}"),
            ));
        }
        let percent = metadata.price_limit().percent().ok_or_else(|| {
            GatewayError::invalid_evidence(
                METADATA_CAPABILITY,
                Some(provider),
                format!("price-limit percent unavailable for {storage_code}"),
            )
        })?;
        if percent.unit() != RatioUnit::Percent {
            return Err(GatewayError::invalid_evidence(
                METADATA_CAPABILITY,
                Some(provider),
                format!("price-limit unit mismatch for {storage_code}"),
            ));
        }
        let version = metadata
            .price_limit()
            .version()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                GatewayError::invalid_evidence(
                    METADATA_CAPABILITY,
                    Some(provider),
                    format!("price-limit rule version unavailable for {storage_code}"),
                )
            })?
            .to_owned();
        projected.push(MarketSecurityMetadata {
            code: storage_code.clone(),
            name,
            board,
            is_st,
            listed_on,
            price_limit_percent: percent.get(),
            price_limit_version: version,
            source_at,
            observed_at,
            provider,
            batch_id: metadata.batch_id().to_owned(),
        });
    }
    Ok(GatewayBatch::Available {
        records: projected,
        evidence,
    })
}

fn validate_batch_evidence<T>(
    capability: &'static str,
    provider: ProviderId,
    batch: &DataBatch<T>,
) -> Result<(BatchEvidence, DateTime<Utc>, DateTime<Utc>), GatewayError> {
    if !batch.quality().is_complete() {
        return Err(GatewayError::classified(
            capability,
            Some(provider),
            "partial",
            "source_batch_incomplete",
            false,
            format!(
                "source batch is incomplete: {}",
                batch.quality().issues().join("; ")
            ),
        ));
    }
    let evidence = BatchEvidence::from_provenance(provider, batch.provenance())?;
    let observed_at =
        parse_provider_timestamp(capability, provider, &evidence.observed_at, "observed_at")?;
    let source_text = evidence.source_at.as_deref().ok_or_else(|| {
        GatewayError::invalid_evidence(
            capability,
            Some(provider),
            "batch provenance has no source timestamp",
        )
    })?;
    let source_at = parse_provider_timestamp(capability, provider, source_text, "source_at")?;
    if source_at > observed_at {
        return Err(GatewayError::invalid_evidence(
            capability,
            Some(provider),
            "batch source timestamp is after its observation timestamp",
        ));
    }
    Ok((evidence, observed_at, source_at))
}

fn parse_required_record_time(
    capability: &'static str,
    provider: ProviderId,
    value: Option<&str>,
    identity: &str,
) -> Result<DateTime<Utc>, GatewayError> {
    let value = value.ok_or_else(|| {
        GatewayError::invalid_evidence(
            capability,
            Some(provider),
            format!("record source timestamp unavailable for {identity}"),
        )
    })?;
    parse_provider_timestamp(capability, provider, value, "record_source_at")
}

fn parse_provider_timestamp(
    capability: &'static str,
    provider: ProviderId,
    value: &str,
    field: &'static str,
) -> Result<DateTime<Utc>, GatewayError> {
    if let Ok(timestamp) = DateTime::parse_from_rfc3339(value) {
        return Ok(timestamp.with_timezone(&Utc));
    }
    let (seconds, nanos) = match value.split_once('.') {
        Some((seconds, fraction)) => {
            if fraction.is_empty()
                || fraction.len() > 9
                || !fraction.bytes().all(|byte| byte.is_ascii_digit())
            {
                return Err(invalid_timestamp(capability, provider, field, value));
            }
            let seconds = seconds
                .parse::<i64>()
                .map_err(|_| invalid_timestamp(capability, provider, field, value))?;
            let padded = format!("{fraction:0<9}");
            let nanos = padded
                .parse::<u32>()
                .map_err(|_| invalid_timestamp(capability, provider, field, value))?;
            (seconds, nanos)
        }
        None => (
            value
                .parse::<i64>()
                .map_err(|_| invalid_timestamp(capability, provider, field, value))?,
            0,
        ),
    };
    Utc.timestamp_opt(seconds, nanos)
        .single()
        .ok_or_else(|| invalid_timestamp(capability, provider, field, value))
}

fn invalid_timestamp(
    capability: &'static str,
    provider: ProviderId,
    field: &'static str,
    value: &str,
) -> GatewayError {
    GatewayError::invalid_evidence(
        capability,
        Some(provider),
        format!("invalid {field} timestamp {value:?}"),
    )
}

fn parse_minute_at(value: &str, provider: ProviderId) -> Result<DateTime<Utc>, GatewayError> {
    let naive = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M").map_err(|error| {
        GatewayError::invalid_evidence(
            MINUTE_CAPABILITY,
            Some(provider),
            format!("invalid minute timestamp {value:?}: {error}"),
        )
    })?;
    shanghai_offset()
        .from_local_datetime(&naive)
        .single()
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .ok_or_else(|| {
            GatewayError::invalid_evidence(
                MINUTE_CAPABILITY,
                Some(provider),
                format!("ambiguous minute timestamp {value:?}"),
            )
        })
}

fn shanghai_offset() -> FixedOffset {
    FixedOffset::east_opt(SHANGHAI_OFFSET_SECONDS)
        .expect("Shanghai UTC offset is a compile-time valid constant")
}

fn validate_observation_time(
    capability: &'static str,
    provider: ProviderId,
    observed_at: DateTime<Utc>,
    now: DateTime<Utc>,
    max_age_millis: i64,
) -> Result<(), GatewayError> {
    let age = now.signed_duration_since(observed_at).num_milliseconds();
    if !(0..=max_age_millis).contains(&age) {
        return Err(GatewayError::classified(
            capability,
            Some(provider),
            "stale",
            "observation_stale",
            true,
            format!(
                "provider observation failed freshness gate age_ms={age} \
                 max_ms={max_age_millis}"
            ),
        ));
    }
    Ok(())
}

fn validate_realtime_freshness(
    capability: &'static str,
    provider: ProviderId,
    source_at: DateTime<Utc>,
    now: DateTime<Utc>,
    identity: &str,
) -> Result<(), GatewayError> {
    let age = now.signed_duration_since(source_at).num_milliseconds();
    if !(0..=REALTIME_MAX_AGE_MILLIS).contains(&age) {
        return Err(GatewayError::classified(
            capability,
            Some(provider),
            "stale",
            "realtime_source_stale",
            true,
            format!("{identity} failed five-second source freshness gate age_ms={age}"),
        ));
    }
    Ok(())
}

fn validate_daily_freshness(
    capability: &'static str,
    provider: ProviderId,
    source_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<(), GatewayError> {
    if source_at > now {
        return Err(GatewayError::invalid_evidence(
            capability,
            Some(provider),
            "daily source timestamp is in the future",
        ));
    }
    let offset = shanghai_offset();
    let source_date = source_at.with_timezone(&offset).date_naive();
    let today = now.with_timezone(&offset).date_naive();
    let oldest_allowed = crate::calendar::prev_trading_day(today);
    if source_date < oldest_allowed {
        return Err(GatewayError::classified(
            capability,
            Some(provider),
            "stale",
            "daily_source_stale",
            true,
            format!(
                "daily source date {source_date} is older than one trading day; \
                 oldest_allowed={oldest_allowed}"
            ),
        ));
    }
    Ok(())
}

fn validate_minute_continuity(
    instrument: &InstrumentId,
    provider: ProviderId,
    previous: DateTime<Utc>,
    current: DateTime<Utc>,
) -> Result<(), GatewayError> {
    let delta = current.signed_duration_since(previous).num_minutes();
    let offset = shanghai_offset();
    let previous_local = previous.with_timezone(&offset);
    let current_local = current.with_timezone(&offset);
    let lunch_transition = previous_local.hour() == 11
        && previous_local.minute() == 30
        && current_local.hour() == 13
        && matches!(current_local.minute(), 0 | 1);
    let beijing_after_hours = instrument.exchange() == Exchange::Beijing
        && previous_local.hour() == 15
        && previous_local.minute() == 0
        && current_local.hour() == 15
        && current_local.minute() == 6;
    if delta != 1 && !lunch_transition && !beijing_after_hours {
        return Err(GatewayError::classified(
            MINUTE_CAPABILITY,
            Some(provider),
            "partial",
            "minute_time_discontinuity",
            false,
            format!(
                "minute timestamps have a gap/duplicate: previous={} current={}",
                previous_local.format("%Y-%m-%d %H:%M"),
                current_local.format("%Y-%m-%d %H:%M")
            ),
        ));
    }
    Ok(())
}

fn project_book_levels(
    levels: &[BookLevel; 5],
    provider: ProviderId,
    code: &str,
    side: &str,
) -> Result<[MarketBookLevel; 5], GatewayError> {
    Ok([
        project_book_level(levels[0], provider, code, side, 1)?,
        project_book_level(levels[1], provider, code, side, 2)?,
        project_book_level(levels[2], provider, code, side, 3)?,
        project_book_level(levels[3], provider, code, side, 4)?,
        project_book_level(levels[4], provider, code, side, 5)?,
    ])
}

fn project_book_level(
    level: BookLevel,
    provider: ProviderId,
    code: &str,
    side: &str,
    position: usize,
) -> Result<MarketBookLevel, GatewayError> {
    let price = level.price().ok_or_else(|| {
        GatewayError::invalid_evidence(
            ORDER_BOOK_CAPABILITY,
            Some(provider),
            format!("{code} {side}{position} price unavailable"),
        )
    })?;
    let quantity = level.quantity().ok_or_else(|| {
        GatewayError::invalid_evidence(
            ORDER_BOOK_CAPABILITY,
            Some(provider),
            format!("{code} {side}{position} quantity unavailable"),
        )
    })?;
    if quantity.get() <= 0.0 {
        return Err(GatewayError::invalid_evidence(
            ORDER_BOOK_CAPABILITY,
            Some(provider),
            format!("{code} {side}{position} quantity must be positive"),
        ));
    }
    Ok(MarketBookLevel {
        price: price.get(),
        quantity: quantity.get(),
    })
}

fn validate_book_order(
    provider: ProviderId,
    code: &str,
    bids: &[MarketBookLevel; 5],
    asks: &[MarketBookLevel; 5],
) -> Result<(), GatewayError> {
    if bids.windows(2).any(|pair| pair[0].price <= pair[1].price) {
        return Err(GatewayError::invalid_evidence(
            ORDER_BOOK_CAPABILITY,
            Some(provider),
            format!("{code} bid prices are not strictly descending"),
        ));
    }
    if asks.windows(2).any(|pair| pair[0].price >= pair[1].price) {
        return Err(GatewayError::invalid_evidence(
            ORDER_BOOK_CAPABILITY,
            Some(provider),
            format!("{code} ask prices are not strictly ascending"),
        ));
    }
    if bids[0].price > asks[0].price {
        return Err(GatewayError::invalid_evidence(
            ORDER_BOOK_CAPABILITY,
            Some(provider),
            format!("{code} order book is crossed"),
        ));
    }
    Ok(())
}

#[cfg(test)]
fn required_money(
    value: Option<magic_market_core::Money>,
    provider: ProviderId,
    code: &str,
    field: &str,
) -> Result<f64, GatewayError> {
    value.map(|money| money.get()).ok_or_else(|| {
        GatewayError::invalid_evidence(
            MONEY_FLOW_CAPABILITY,
            Some(provider),
            format!("{code} {field} unavailable"),
        )
    })
}

#[cfg(test)]
fn project_board(board: Board) -> Option<SecurityBoard> {
    match board {
        Board::Main => Some(SecurityBoard::Main),
        Board::Star => Some(SecurityBoard::Star),
        Board::ChiNext => Some(SecurityBoard::ChiNext),
        Board::Beijing => Some(SecurityBoard::Beijing),
        Board::Unknown => None,
    }
}

fn terminal_provider(error: &RouterError, default: ProviderId) -> ProviderId {
    error
        .attempts()
        .last()
        .map(|attempt| attempt.provider_id())
        .unwrap_or(default)
}

fn router_gateway_error(
    capability: &'static str,
    error: RouterError,
    provider: ProviderId,
) -> GatewayError {
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
        capability,
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
    use magic_market_core::{
        BookLevel, DataBatch, Money, MoneyFlow, Price, PriceLimitRule, Provenance, Quantity, Ratio,
    };

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-07-24T09:30:02+08:00")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn source_at() -> String {
        "2026-07-24T09:30:00+08:00".to_owned()
    }

    fn observed_epoch() -> String {
        now().timestamp().to_string()
    }

    fn instrument() -> InstrumentId {
        build_instrument("TEST_CODE_600396", MINUTE_CAPABILITY).unwrap()
    }

    fn provenance(source: &str, batch_id: &str) -> Provenance {
        Provenance::new(source, observed_epoch())
            .unwrap()
            .with_source_at(source_at())
            .unwrap()
            .with_batch_id(batch_id)
            .unwrap()
    }

    #[test]
    fn br164_provider_orders_match_implemented_upstream_contracts() {
        assert_eq!(
            MINUTE_PROVIDER_ORDER,
            &[ProviderId::Tdx, ProviderId::Tencent, ProviderId::Sina]
        );
        assert_eq!(
            ORDER_BOOK_PROVIDER_ORDER,
            &[ProviderId::Tdx, ProviderId::Tencent, ProviderId::Sina]
        );
        assert_eq!(MONEY_FLOW_PROVIDER_ORDER, &[ProviderId::Eastmoney]);
        assert_eq!(
            METADATA_PROVIDER_ORDER,
            &[ProviderId::Tencent, ProviderId::Sina]
        );
    }

    #[test]
    fn br164_complete_metadata_is_typed_unsupported_without_network() {
        let error = unsupported_security_metadata(&["TEST_CODE_600396".to_owned()]).unwrap_err();
        assert_eq!(error.reason_code(), "unsupported_contract");
        assert_eq!(error.audit_outcome(), "unsupported");
        assert!(!error.retryable());
    }

    #[test]
    fn br164_observation_age_requires_a_post_route_validation_clock() {
        let observed_at = now();
        assert!(validate_observation_time(
            SECURITY_IDENTITY_CAPABILITY,
            ProviderId::Tencent,
            observed_at,
            observed_at + chrono::Duration::milliseconds(1),
            ACQUISITION_MAX_AGE_MILLIS,
        )
        .is_ok());

        let error = validate_observation_time(
            SECURITY_IDENTITY_CAPABILITY,
            ProviderId::Tencent,
            observed_at,
            observed_at - chrono::Duration::milliseconds(1),
            ACQUISITION_MAX_AGE_MILLIS,
        )
        .unwrap_err();
        assert_eq!(error.reason_code(), "observation_stale");
    }

    #[test]
    fn br164_minute_fixture_preserves_optional_amount_and_evidence() {
        let batch_id = "TEST_CODE_minute_batch";
        let point = MinutePoint::new(
            instrument(),
            "2026-07-24 09:30",
            Price::new(10.0).unwrap(),
            Quantity::new(100.0).unwrap(),
            None,
            DataStatus::Available,
            Some(source_at()),
            observed_epoch(),
            ProviderId::Tdx,
            batch_id,
        )
        .unwrap();
        let batch = DataBatch::strict(vec![point], provenance("TEST_CODE_tdx", batch_id));
        let request = MinuteDataRequest::new(instrument());
        let admitted =
            admit_minute_batch("TEST_CODE_600396", &request, ProviderId::Tdx, batch, now())
                .unwrap();
        assert_eq!(admitted.records()[0].code, "TEST_CODE_600396");
        assert_eq!(admitted.records()[0].cumulative_amount, None);
        assert_eq!(admitted.evidence().batch_id, batch_id);
    }

    #[test]
    fn br164_minute_fixture_rejects_gaps_and_stale_current_data() {
        let batch_id = "TEST_CODE_minute_gap";
        let points = ["09:30", "09:32"]
            .into_iter()
            .enumerate()
            .map(|(index, time)| {
                MinutePoint::new(
                    instrument(),
                    format!("2026-07-24 {time}"),
                    Price::new(10.0 + index as f64).unwrap(),
                    Quantity::new(100.0 + index as f64).unwrap(),
                    None,
                    DataStatus::Available,
                    Some(format!("2026-07-24T{time}:00+08:00")),
                    observed_epoch(),
                    ProviderId::Tdx,
                    batch_id,
                )
                .unwrap()
            })
            .collect();
        let gap_provenance = Provenance::new("TEST_CODE_tdx", observed_epoch())
            .unwrap()
            .with_source_at("2026-07-24T09:32:00+08:00")
            .unwrap()
            .with_batch_id(batch_id)
            .unwrap();
        let gap = DataBatch::strict(points, gap_provenance);
        let request = MinuteDataRequest::new(instrument());
        assert!(admit_minute_batch(
            "TEST_CODE_600396",
            &request,
            ProviderId::Tdx,
            gap,
            DateTime::parse_from_rfc3339("2026-07-24T09:32:02+08:00")
                .unwrap()
                .with_timezone(&Utc),
        )
        .is_err());

        let stale_point = MinutePoint::new(
            instrument(),
            "2026-07-24 09:30",
            Price::new(10.0).unwrap(),
            Quantity::new(100.0).unwrap(),
            None,
            DataStatus::Available,
            Some(source_at()),
            observed_epoch(),
            ProviderId::Tdx,
            batch_id,
        )
        .unwrap();
        let stale = DataBatch::strict(vec![stale_point], provenance("TEST_CODE_tdx", batch_id));
        let stale_now = DateTime::parse_from_rfc3339("2026-07-24T09:30:06+08:00")
            .unwrap()
            .with_timezone(&Utc);
        assert!(admit_minute_batch(
            "TEST_CODE_600396",
            &request,
            ProviderId::Tdx,
            stale,
            stale_now,
        )
        .is_err());
    }

    fn book_levels(prices: [f64; 5]) -> [BookLevel; 5] {
        prices.map(|price| {
            BookLevel::new(
                Some(Price::new(price).unwrap()),
                Some(Quantity::new(100.0).unwrap()),
            )
            .unwrap()
        })
    }

    #[test]
    fn br164_order_book_fixture_requires_complete_ordered_depth() {
        let batch_id = "TEST_CODE_order_book";
        let bids = book_levels([9.99, 9.98, 9.97, 9.96, 9.95]);
        let asks = book_levels([10.01, 10.02, 10.03, 10.04, 10.05]);
        let book = OrderBook::new(
            instrument(),
            bids,
            asks,
            Some(Quantity::new(500.0).unwrap()),
            Some(Quantity::new(500.0).unwrap()),
            DataStatus::Available,
            Some(source_at()),
            observed_epoch(),
            ProviderId::Tencent,
            batch_id,
        )
        .unwrap();
        let batch = DataBatch::strict(vec![book], provenance("TEST_CODE_tencent", batch_id));
        let codes = vec!["TEST_CODE_600396".to_owned()];
        let instruments = vec![instrument()];
        let admitted =
            admit_order_book_batch(&codes, &instruments, ProviderId::Tencent, batch, now())
                .unwrap();
        assert_eq!(admitted.records()[0].bids[0].price, 9.99);
        assert_eq!(admitted.records()[0].asks[4].price, 10.05);
    }

    #[test]
    fn br164_money_flow_fixture_validates_composition_and_daily_evidence() {
        let batch_id = "TEST_CODE_money_flow";
        let flow = MoneyFlow::new(
            instrument(),
            Some(Money::new(30.0).unwrap()),
            Some(Money::new(10.0).unwrap()),
            Some(Money::new(20.0).unwrap()),
            Some(Money::new(-5.0).unwrap()),
            Some(Money::new(-25.0).unwrap()),
            DataStatus::Available,
            Some(source_at()),
            observed_epoch(),
            ProviderId::Eastmoney,
            batch_id,
        )
        .unwrap();
        let batch = DataBatch::strict(vec![flow], provenance("TEST_CODE_emquant", batch_id));
        let codes = vec!["TEST_CODE_600396".to_owned()];
        let instruments = vec![instrument()];
        let admitted =
            admit_money_flow_batch(&codes, &instruments, ProviderId::Eastmoney, batch, now())
                .unwrap();
        assert_eq!(admitted.records()[0].main_net, 30.0);
        assert_eq!(admitted.records()[0].small_net, -25.0);
    }

    #[test]
    fn br164_security_metadata_fixture_requires_all_source_fields() {
        let batch_id = "TEST_CODE_metadata";
        let metadata = SecurityMetadata::new(
            instrument(),
            Some("协议测试股票".to_owned()),
            Some(Board::Main),
            Some(false),
            Some("1999-01-01".to_owned()),
            PriceLimitRule::new(
                Some(Ratio::new(10.0, RatioUnit::Percent).unwrap()),
                Some("TEST_CODE_rule_v1".to_owned()),
            )
            .unwrap(),
            DataStatus::Available,
            Some(source_at()),
            observed_epoch(),
            ProviderId::Tencent,
            batch_id,
        )
        .unwrap();
        let batch = DataBatch::strict(vec![metadata], provenance("TEST_CODE_tencent", batch_id));
        let codes = vec!["TEST_CODE_600396".to_owned()];
        let instruments = vec![instrument()];
        let admitted =
            admit_metadata_batch(&codes, &instruments, ProviderId::Tencent, batch, now()).unwrap();
        assert_eq!(admitted.records()[0].board, SecurityBoard::Main);
        assert_eq!(admitted.records()[0].price_limit_percent, 10.0);
        assert_eq!(
            admitted.records()[0].price_limit_version,
            "TEST_CODE_rule_v1"
        );
    }

    #[test]
    fn br164_security_identity_admits_only_real_subset_without_network() {
        let batch_id = "TEST_CODE_security_identity";
        let metadata = SecurityMetadata::new(
            instrument(),
            Some("协议测试股票".to_owned()),
            Some(Board::Main),
            Some(false),
            None,
            PriceLimitRule::new(None, None).unwrap(),
            DataStatus::Unavailable,
            Some(source_at()),
            observed_epoch(),
            ProviderId::Tencent,
            batch_id,
        )
        .unwrap();
        let batch = DataBatch::best_effort(
            vec![metadata],
            provenance("TEST_CODE_tencent", batch_id),
            vec![
                "TEST_CODE listing date unavailable".to_owned(),
                "TEST_CODE price-limit rule unavailable".to_owned(),
            ],
        )
        .unwrap();
        let codes = vec!["TEST_CODE_600396".to_owned()];
        let instruments = vec![instrument()];
        let admitted =
            admit_security_identity_batch(&codes, &instruments, ProviderId::Tencent, batch, now())
                .unwrap();

        assert_eq!(admitted.records()[0].code, "TEST_CODE_600396");
        assert_eq!(admitted.records()[0].name, "协议测试股票");
        assert!(!admitted.records()[0].is_st);
        assert_eq!(admitted.evidence().batch_id, batch_id);
        assert!(!security_identity_policy().require_complete());
        assert!(security_identity_policy().require_source_at());
    }

    #[test]
    fn br164_rejects_empty_duplicate_and_invalid_requests() {
        assert!(build_instruments(&[], ORDER_BOOK_CAPABILITY).is_err());
        assert!(build_instruments(
            &["TEST_CODE_600396".to_owned(), "TEST_CODE_600396".to_owned()],
            ORDER_BOOK_CAPABILITY,
        )
        .is_err());
        assert!(build_instrument("TEST_CODE_BAD", METADATA_CAPABILITY).is_err());
    }

    #[test]
    fn br173_market_capability_requests_use_canonical_a_share_identity() {
        let instruments = build_instruments(
            &[
                "TEST_CODE_600396".to_owned(),
                "TEST_CODE_000001".to_owned(),
                "TEST_CODE_920118".to_owned(),
            ],
            METADATA_CAPABILITY,
        )
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
            assert!(build_instrument(code, METADATA_CAPABILITY).is_err());
        }
        assert!(build_instrument("TEST_CODE_100001", METADATA_CAPABILITY).is_err());
        assert!(build_instrument("TEST_CODE_60039A", METADATA_CAPABILITY).is_err());

        let date = NaiveDate::from_ymd_opt(2026, 7, 24).unwrap();
        let dated = build_minute_request(instrument(), Some(date)).unwrap();
        assert_eq!(dated.date(), Some("2026-07-24"));
        let current = build_minute_request(instrument(), None).unwrap();
        assert_eq!(current.date(), None);
    }

    #[test]
    fn br164_market_capability_timestamp_parsers_reject_ambiguous_input() {
        let rfc3339 = parse_provider_timestamp(
            MINUTE_CAPABILITY,
            ProviderId::Tencent,
            "2026-07-24T09:30:00+08:00",
            "source_at",
        )
        .unwrap();
        let epoch = parse_provider_timestamp(
            MINUTE_CAPABILITY,
            ProviderId::Tencent,
            "1784856600.25",
            "source_at",
        )
        .unwrap();
        assert_eq!(rfc3339.timestamp(), epoch.timestamp());
        assert_eq!(epoch.timestamp_subsec_nanos(), 250_000_000);
        for invalid in [
            "TEST_CODE_invalid",
            "1784856600.",
            "1784856600.TEST",
            "1784856600.1234567890",
        ] {
            assert!(parse_provider_timestamp(
                MINUTE_CAPABILITY,
                ProviderId::Tencent,
                invalid,
                "source_at",
            )
            .is_err());
        }

        assert_eq!(
            parse_minute_at("2026-07-24 09:30", ProviderId::Tencent)
                .unwrap()
                .timestamp(),
            rfc3339.timestamp()
        );
        assert!(parse_minute_at("2026/07/24 09:30", ProviderId::Tencent).is_err());
        assert!(parse_required_record_time(
            ORDER_BOOK_CAPABILITY,
            ProviderId::Tencent,
            None,
            "TEST_CODE_600396",
        )
        .is_err());
    }

    #[test]
    fn br164_minute_continuity_allows_market_sessions_but_rejects_duplicates() {
        let instant = |value: &str| {
            DateTime::parse_from_rfc3339(value)
                .unwrap()
                .with_timezone(&Utc)
        };
        assert!(validate_minute_continuity(
            &instrument(),
            ProviderId::Tencent,
            instant("2026-07-24T09:30:00+08:00"),
            instant("2026-07-24T09:31:00+08:00"),
        )
        .is_ok());
        assert!(validate_minute_continuity(
            &instrument(),
            ProviderId::Tencent,
            instant("2026-07-24T11:30:00+08:00"),
            instant("2026-07-24T13:00:00+08:00"),
        )
        .is_ok());
        let beijing =
            build_instrument("TEST_CODE_920118", MINUTE_CAPABILITY).expect("Beijing instrument");
        assert!(validate_minute_continuity(
            &beijing,
            ProviderId::Tdx,
            instant("2026-07-24T15:00:00+08:00"),
            instant("2026-07-24T15:06:00+08:00"),
        )
        .is_ok());

        let duplicate = validate_minute_continuity(
            &instrument(),
            ProviderId::Tencent,
            instant("2026-07-24T09:30:00+08:00"),
            instant("2026-07-24T09:30:00+08:00"),
        )
        .unwrap_err();
        assert_eq!(duplicate.reason_code(), "minute_time_discontinuity");
        assert!(!duplicate.retryable());
    }

    #[test]
    fn br164_order_book_projection_rejects_missing_zero_unordered_and_crossed_levels() {
        assert!(project_book_level(
            BookLevel::unavailable(),
            ProviderId::Tencent,
            "TEST_CODE_600396",
            "bid",
            1,
        )
        .is_err());
        let zero = BookLevel::new(
            Some(Price::new(10.0).unwrap()),
            Some(Quantity::new(0.0).unwrap()),
        )
        .unwrap();
        assert!(
            project_book_level(zero, ProviderId::Tencent, "TEST_CODE_600396", "bid", 1,).is_err()
        );

        let levels = |prices: [f64; 5]| {
            prices.map(|price| MarketBookLevel {
                price,
                quantity: 100.0,
            })
        };
        let bids = levels([9.99, 9.98, 9.97, 9.96, 9.95]);
        let asks = levels([10.01, 10.02, 10.03, 10.04, 10.05]);
        assert!(
            validate_book_order(ProviderId::Tencent, "TEST_CODE_600396", &bids, &asks,).is_ok()
        );
        assert!(validate_book_order(
            ProviderId::Tencent,
            "TEST_CODE_600396",
            &levels([9.99, 9.99, 9.97, 9.96, 9.95]),
            &asks,
        )
        .is_err());
        assert!(validate_book_order(
            ProviderId::Tencent,
            "TEST_CODE_600396",
            &bids,
            &levels([10.01, 10.01, 10.03, 10.04, 10.05]),
        )
        .is_err());
        assert!(validate_book_order(
            ProviderId::Tencent,
            "TEST_CODE_600396",
            &levels([10.02, 9.98, 9.97, 9.96, 9.95]),
            &asks,
        )
        .is_err());
    }

    #[test]
    fn br164_board_projection_and_provider_classifiers_are_exhaustive() {
        assert_eq!(project_board(Board::Main), Some(SecurityBoard::Main));
        assert_eq!(project_board(Board::Star), Some(SecurityBoard::Star));
        assert_eq!(project_board(Board::ChiNext), Some(SecurityBoard::ChiNext));
        assert_eq!(project_board(Board::Beijing), Some(SecurityBoard::Beijing));
        assert_eq!(project_board(Board::Unknown), None);

        assert_eq!(
            classify_tdx_error(TdxError::ConnectionTimeout).kind(),
            FailureKind::Transport
        );
        assert_eq!(
            classify_tdx_error(TdxError::InvalidData("TEST_CODE".to_owned())).kind(),
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
            classify_tencent_error(TencentError::InvalidRequest("TEST_CODE".to_owned())).kind(),
            FailureKind::InvalidRequest
        );
        assert_eq!(
            classify_tencent_error(TencentError::Core(
                magic_market_core::CoreError::InvalidRequest("TEST_CODE".to_owned())
            ))
            .kind(),
            FailureKind::Evidence
        );
        assert_eq!(
            classify_sina_error(SinaError::Unsupported("TEST_CODE".to_owned())).kind(),
            FailureKind::Unsupported
        );
    }

    #[test]
    fn br164_minute_admission_rejects_every_unproved_transition() {
        let make_point = |minute: &str,
                          price: f64,
                          quantity: f64,
                          source: Option<String>,
                          provider: ProviderId,
                          batch_id: &str| {
            MinutePoint::new(
                instrument(),
                minute,
                Price::new(price).unwrap(),
                Quantity::new(quantity).unwrap(),
                None,
                DataStatus::Available,
                source,
                observed_epoch(),
                provider,
                batch_id,
            )
            .unwrap()
        };
        let request = MinuteDataRequest::new(instrument());
        let empty = DataBatch::strict(
            Vec::<MinutePoint>::new(),
            provenance("TEST_CODE_tdx", "TEST_CODE_empty"),
        );
        assert_eq!(
            admit_minute_batch("TEST_CODE_600396", &request, ProviderId::Tdx, empty, now(),)
                .unwrap_err()
                .reason_code(),
            "verified_minute_batch_empty"
        );

        let cases = [
            (
                vec![make_point(
                    "2026-07-24 09:30",
                    10.0,
                    100.0,
                    Some(source_at()),
                    ProviderId::Sina,
                    "TEST_CODE_minute",
                )],
                source_at(),
                now(),
            ),
            (
                vec![make_point(
                    "2026-07-24 09:30",
                    10.0,
                    100.0,
                    Some("2026-07-24T09:31:00+08:00".to_owned()),
                    ProviderId::Tdx,
                    "TEST_CODE_minute",
                )],
                "2026-07-24T09:31:00+08:00".to_owned(),
                now(),
            ),
            (
                vec![
                    make_point(
                        "2026-07-24 09:30",
                        10.0,
                        200.0,
                        Some(source_at()),
                        ProviderId::Tdx,
                        "TEST_CODE_minute",
                    ),
                    make_point(
                        "2026-07-24 09:31",
                        10.1,
                        100.0,
                        Some("2026-07-24T09:31:00+08:00".to_owned()),
                        ProviderId::Tdx,
                        "TEST_CODE_minute",
                    ),
                ],
                "2026-07-24T09:31:00+08:00".to_owned(),
                DateTime::parse_from_rfc3339("2026-07-24T09:31:02+08:00")
                    .unwrap()
                    .with_timezone(&Utc),
            ),
            (
                vec![
                    make_point(
                        "2026-07-24 09:30",
                        10.0,
                        100.0,
                        Some(source_at()),
                        ProviderId::Tdx,
                        "TEST_CODE_minute",
                    ),
                    make_point(
                        "2026-07-24 09:31",
                        13.0,
                        200.0,
                        Some("2026-07-24T09:31:00+08:00".to_owned()),
                        ProviderId::Tdx,
                        "TEST_CODE_minute",
                    ),
                ],
                "2026-07-24T09:31:00+08:00".to_owned(),
                DateTime::parse_from_rfc3339("2026-07-24T09:31:02+08:00")
                    .unwrap()
                    .with_timezone(&Utc),
            ),
        ];
        for (points, batch_source_at, test_now) in cases {
            let batch = DataBatch::strict(
                points,
                Provenance::new("TEST_CODE_tdx", observed_epoch())
                    .unwrap()
                    .with_source_at(batch_source_at)
                    .unwrap()
                    .with_batch_id("TEST_CODE_minute")
                    .unwrap(),
            );
            assert!(admit_minute_batch(
                "TEST_CODE_600396",
                &request,
                ProviderId::Tdx,
                batch,
                test_now,
            )
            .is_err());
        }

        let dated_request = build_minute_request(
            instrument(),
            Some(NaiveDate::from_ymd_opt(2026, 7, 23).unwrap()),
        )
        .unwrap();
        let dated = DataBatch::strict(
            vec![make_point(
                "2026-07-24 09:30",
                10.0,
                100.0,
                Some(source_at()),
                ProviderId::Tdx,
                "TEST_CODE_minute",
            )],
            provenance("TEST_CODE_tdx", "TEST_CODE_minute"),
        );
        assert!(admit_minute_batch(
            "TEST_CODE_600396",
            &dated_request,
            ProviderId::Tdx,
            dated,
            now(),
        )
        .is_err());

        let batch_source_mismatch = DataBatch::strict(
            vec![make_point(
                "2026-07-24 09:30",
                10.0,
                100.0,
                Some(source_at()),
                ProviderId::Tdx,
                "TEST_CODE_minute",
            )],
            Provenance::new("TEST_CODE_tdx", observed_epoch())
                .unwrap()
                .with_source_at("2026-07-24T09:31:00+08:00")
                .unwrap()
                .with_batch_id("TEST_CODE_minute")
                .unwrap(),
        );
        assert!(admit_minute_batch(
            "TEST_CODE_600396",
            &request,
            ProviderId::Tdx,
            batch_source_mismatch,
            now(),
        )
        .is_err());
    }

    #[test]
    fn br164_order_book_and_batch_envelopes_reject_missing_evidence() {
        let codes = vec!["TEST_CODE_600396".to_owned()];
        let instruments = vec![instrument()];
        let empty = DataBatch::strict(
            Vec::<OrderBook>::new(),
            provenance("TEST_CODE_tencent", "TEST_CODE_empty_book"),
        );
        assert!(
            admit_order_book_batch(&codes, &instruments, ProviderId::Tencent, empty, now(),)
                .is_err()
        );

        let build_book = |provider: ProviderId,
                          source: Option<String>,
                          total_bid: Option<Quantity>,
                          total_ask: Option<Quantity>,
                          batch_id: &str| {
            let bids = total_bid.map_or_else(
                || [BookLevel::unavailable(); 5],
                |_| book_levels([9.99, 9.98, 9.97, 9.96, 9.95]),
            );
            let asks = total_ask.map_or_else(
                || [BookLevel::unavailable(); 5],
                |_| book_levels([10.01, 10.02, 10.03, 10.04, 10.05]),
            );
            let status = if total_bid.is_some() && total_ask.is_some() && source.is_some() {
                DataStatus::Available
            } else {
                DataStatus::Unavailable
            };
            OrderBook::new(
                instrument(),
                bids,
                asks,
                total_bid,
                total_ask,
                status,
                source,
                observed_epoch(),
                provider,
                batch_id,
            )
            .unwrap()
        };
        for book in [
            build_book(
                ProviderId::Sina,
                Some(source_at()),
                Some(Quantity::new(500.0).unwrap()),
                Some(Quantity::new(500.0).unwrap()),
                "TEST_CODE_book",
            ),
            build_book(
                ProviderId::Tencent,
                Some("2026-07-24T09:29:00+08:00".to_owned()),
                Some(Quantity::new(500.0).unwrap()),
                Some(Quantity::new(500.0).unwrap()),
                "TEST_CODE_book",
            ),
            build_book(
                ProviderId::Tencent,
                Some(source_at()),
                None,
                Some(Quantity::new(500.0).unwrap()),
                "TEST_CODE_book",
            ),
            build_book(
                ProviderId::Tencent,
                Some(source_at()),
                Some(Quantity::new(500.0).unwrap()),
                None,
                "TEST_CODE_book",
            ),
        ] {
            let batch = DataBatch::strict(
                vec![book],
                provenance("TEST_CODE_tencent", "TEST_CODE_book"),
            );
            assert!(admit_order_book_batch(
                &codes,
                &instruments,
                ProviderId::Tencent,
                batch,
                now(),
            )
            .is_err());
        }

        let no_source = Provenance::new("TEST_CODE_tencent", observed_epoch())
            .unwrap()
            .with_batch_id("TEST_CODE_no_source")
            .unwrap();
        assert!(validate_batch_evidence::<()>(
            ORDER_BOOK_CAPABILITY,
            ProviderId::Tencent,
            &DataBatch::strict(Vec::new(), no_source),
        )
        .is_err());
        let partial = DataBatch::best_effort(
            Vec::<OrderBook>::new(),
            provenance("TEST_CODE_tencent", "TEST_CODE_partial"),
            vec!["TEST_CODE incomplete".to_owned()],
        )
        .unwrap();
        assert!(
            validate_batch_evidence(ORDER_BOOK_CAPABILITY, ProviderId::Tencent, &partial).is_err()
        );
    }

    #[test]
    fn br164_money_flow_and_metadata_missing_fields_never_get_filled() {
        let codes = vec!["TEST_CODE_600396".to_owned()];
        let instruments = vec![instrument()];
        let flow = |main: Option<f64>,
                    super_large: Option<f64>,
                    large: Option<f64>,
                    medium: Option<f64>,
                    small: Option<f64>,
                    provider: ProviderId,
                    source: Option<String>| {
            let status = if main.is_some()
                && super_large.is_some()
                && large.is_some()
                && medium.is_some()
                && small.is_some()
                && source.is_some()
            {
                DataStatus::Available
            } else {
                DataStatus::Unavailable
            };
            MoneyFlow::new(
                instrument(),
                main.map(|value| Money::new(value).unwrap()),
                super_large.map(|value| Money::new(value).unwrap()),
                large.map(|value| Money::new(value).unwrap()),
                medium.map(|value| Money::new(value).unwrap()),
                small.map(|value| Money::new(value).unwrap()),
                status,
                source,
                observed_epoch(),
                provider,
                "TEST_CODE_flow",
            )
            .unwrap()
        };
        for record in [
            flow(
                None,
                Some(10.0),
                Some(20.0),
                Some(-5.0),
                Some(-25.0),
                ProviderId::Eastmoney,
                Some(source_at()),
            ),
            flow(
                Some(30.0),
                None,
                Some(20.0),
                Some(-5.0),
                Some(-25.0),
                ProviderId::Eastmoney,
                Some(source_at()),
            ),
            flow(
                Some(30.0),
                Some(10.0),
                None,
                Some(-5.0),
                Some(-25.0),
                ProviderId::Eastmoney,
                Some(source_at()),
            ),
            flow(
                Some(30.0),
                Some(10.0),
                Some(20.0),
                None,
                Some(-25.0),
                ProviderId::Eastmoney,
                Some(source_at()),
            ),
            flow(
                Some(30.0),
                Some(10.0),
                Some(20.0),
                Some(-5.0),
                None,
                ProviderId::Eastmoney,
                Some(source_at()),
            ),
            flow(
                Some(31.0),
                Some(10.0),
                Some(20.0),
                Some(-5.0),
                Some(-25.0),
                ProviderId::Eastmoney,
                Some(source_at()),
            ),
            flow(
                Some(30.0),
                Some(10.0),
                Some(20.0),
                Some(-5.0),
                Some(-25.0),
                ProviderId::Custom,
                Some(source_at()),
            ),
            flow(
                Some(30.0),
                Some(10.0),
                Some(20.0),
                Some(-5.0),
                Some(-25.0),
                ProviderId::Eastmoney,
                None,
            ),
        ] {
            assert!(admit_money_flow_batch(
                &codes,
                &instruments,
                ProviderId::Eastmoney,
                DataBatch::strict(
                    vec![record],
                    provenance("TEST_CODE_emquant", "TEST_CODE_flow"),
                ),
                now(),
            )
            .is_err());
        }
        assert!(admit_money_flow_batch(
            &codes,
            &instruments,
            ProviderId::Eastmoney,
            DataBatch::strict(
                Vec::<MoneyFlow>::new(),
                provenance("TEST_CODE_emquant", "TEST_CODE_empty_flow"),
            ),
            now(),
        )
        .is_err());

        let metadata = |name: Option<&str>,
                        board: Option<Board>,
                        is_st: Option<bool>,
                        listed_on: Option<&str>,
                        percent: Option<f64>,
                        version: Option<&str>,
                        provider: ProviderId,
                        source: Option<String>| {
            let status = if name.is_some()
                && board.is_some()
                && is_st.is_some()
                && listed_on.is_some()
                && percent.is_some()
                && version.is_some()
                && source.is_some()
            {
                DataStatus::Available
            } else {
                DataStatus::Unavailable
            };
            SecurityMetadata::new(
                instrument(),
                name.map(str::to_owned),
                board,
                is_st,
                listed_on.map(str::to_owned),
                PriceLimitRule::new(
                    percent.map(|value| Ratio::new(value, RatioUnit::Percent).unwrap()),
                    version.map(str::to_owned),
                )
                .unwrap(),
                status,
                source,
                observed_epoch(),
                provider,
                "TEST_CODE_metadata",
            )
            .unwrap()
        };
        for record in [
            metadata(
                None,
                Some(Board::Main),
                Some(false),
                Some("1999-01-01"),
                Some(10.0),
                Some("TEST_CODE_v1"),
                ProviderId::Tencent,
                Some(source_at()),
            ),
            metadata(
                Some("测试"),
                None,
                Some(false),
                Some("1999-01-01"),
                Some(10.0),
                Some("TEST_CODE_v1"),
                ProviderId::Tencent,
                Some(source_at()),
            ),
            metadata(
                Some("测试"),
                Some(Board::Unknown),
                Some(false),
                Some("1999-01-01"),
                Some(10.0),
                Some("TEST_CODE_v1"),
                ProviderId::Tencent,
                Some(source_at()),
            ),
            metadata(
                Some("测试"),
                Some(Board::Main),
                None,
                Some("1999-01-01"),
                Some(10.0),
                Some("TEST_CODE_v1"),
                ProviderId::Tencent,
                Some(source_at()),
            ),
            metadata(
                Some("测试"),
                Some(Board::Main),
                Some(false),
                None,
                Some(10.0),
                Some("TEST_CODE_v1"),
                ProviderId::Tencent,
                Some(source_at()),
            ),
            metadata(
                Some("测试"),
                Some(Board::Main),
                Some(false),
                Some("2099-01-01"),
                Some(10.0),
                Some("TEST_CODE_v1"),
                ProviderId::Tencent,
                Some(source_at()),
            ),
            metadata(
                Some("测试"),
                Some(Board::Main),
                Some(false),
                Some("1999-01-01"),
                None,
                Some("TEST_CODE_v1"),
                ProviderId::Tencent,
                Some(source_at()),
            ),
            metadata(
                Some("测试"),
                Some(Board::Main),
                Some(false),
                Some("1999-01-01"),
                Some(10.0),
                None,
                ProviderId::Tencent,
                Some(source_at()),
            ),
            metadata(
                Some("测试"),
                Some(Board::Main),
                Some(false),
                Some("1999-01-01"),
                Some(10.0),
                Some("TEST_CODE_v1"),
                ProviderId::Sina,
                Some(source_at()),
            ),
            metadata(
                Some("测试"),
                Some(Board::Main),
                Some(false),
                Some("1999-01-01"),
                Some(10.0),
                Some("TEST_CODE_v1"),
                ProviderId::Tencent,
                None,
            ),
        ] {
            assert!(admit_metadata_batch(
                &codes,
                &instruments,
                ProviderId::Tencent,
                DataBatch::strict(
                    vec![record],
                    provenance("TEST_CODE_tencent", "TEST_CODE_metadata"),
                ),
                now(),
            )
            .is_err());
        }
        assert!(admit_metadata_batch(
            &codes,
            &instruments,
            ProviderId::Tencent,
            DataBatch::strict(
                Vec::<SecurityMetadata>::new(),
                provenance("TEST_CODE_tencent", "TEST_CODE_empty_metadata"),
            ),
            now(),
        )
        .is_err());
    }
}
