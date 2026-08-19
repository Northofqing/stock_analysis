//! BR-164 evidence-preserving company and quote-statistics gateway.
//!
//! Financial statements retain the upstream normalized facts without flattening
//! line-item keys, units or explicit missing values. Market statistics retain
//! every optional field as `Option`; absence is never converted to zero.

use crate::magic_compat::ProviderId;
#[cfg(feature = "magic-gateway")]
use crate::magic_compat::{DataBatch, Exchange, InstrumentId, RatioUnit};
pub use crate::magic_compat::{FinancialLine, FinancialStatement, MarketStatistics, StatementKind};
#[cfg(feature = "magic-gateway")]
use chrono::{DateTime, FixedOffset, NaiveDate, TimeZone, Utc};
#[cfg(feature = "magic-gateway")]
use magic_market_router::{
    financial_statement_source, market_statistics_source, AcceptancePolicy, AttemptStatus,
    FailureKind, FinancialStatementRequest, FinancialStatementRouter, MarketStatisticsRouter,
    RouterError, SourceError,
};
#[cfg(feature = "magic-gateway")]
use magic_sina_rs::{SinaClient, SinaError};
#[cfg(feature = "magic-gateway")]
use magic_tencent_rs::{TencentClient, TencentError};
#[cfg(feature = "magic-gateway")]
use std::collections::HashSet;
#[cfg(feature = "magic-gateway")]
use std::sync::Arc;

#[cfg(feature = "magic-gateway")]
use super::review::audit_blocking_join_failure;
#[cfg(feature = "magic-gateway")]
use super::review::BatchEvidence;
use super::review::{acquisition_request_hash, audit_gateway_result, GatewayBatch, GatewayError};

const FINANCIAL_CAPABILITY: &str = "CompanyFinancialStatements";
const STATISTICS_CAPABILITY: &str = "CompanyMarketStatistics";
const REALTIME_MAX_AGE_MILLIS: i64 = 5_000;
const ACQUISITION_MAX_AGE_MILLIS: i64 = 30_000;
const SHANGHAI_OFFSET_SECONDS: i32 = 8 * 60 * 60;

/// Pinned upstream provider order for all three normalized statements.
pub const FINANCIAL_STATEMENT_PROVIDER_ORDER: &[ProviderId] = &[ProviderId::Sina];
/// Pinned upstream provider order for PE/PB/capitalization/trading statistics.
pub const MARKET_STATISTICS_PROVIDER_ORDER: &[ProviderId] = &[ProviderId::Tencent];

/// BR-164 gateway for normalized company facts.
#[derive(Debug, Clone, Copy, Default)]
pub struct CompanyDataGateway;

impl CompanyDataGateway {
    pub const fn new() -> Self {
        Self
    }

    /// Fetches one statement family for every requested Shanghai/Shenzhen
    /// equity. Returned records are the original upstream strong types.
    pub async fn financial_statements(
        &self,
        codes: &[String],
        kind: StatementKind,
    ) -> Result<GatewayBatch<FinancialStatement>, GatewayError> {
        let storage_codes = codes.to_vec();
        let request_hash = acquisition_request_hash(
            FINANCIAL_CAPABILITY,
            format!("{kind:?}:{}", storage_codes.join(",")),
        );
        // P4 M4b: gRPC 桥 (DATA_GATEWAY_GRPC=1 时替换 transport; audit 留客户端)。
        match super::grpc_source::bridge_for("FinancialStatements") {
            Ok(Some(bridge)) => {
                let result = bridge
                    .financial_statements_async(&storage_codes, kind)
                    .await;
                let audit_provider = result
                    .as_ref()
                    .map(|b| b.evidence().provider)
                    .unwrap_or(ProviderId::Sina);
                return audit_gateway_result(
                    FINANCIAL_CAPABILITY,
                    audit_provider,
                    &request_hash,
                    result,
                );
            }
            Ok(None) => {}
            Err(error) => {
                return audit_gateway_result(
                    FINANCIAL_CAPABILITY,
                    ProviderId::Sina,
                    &request_hash,
                    Err(error),
                );
            }
        }
        // no-feature (monitor 零 magic): library transport 不存在。
        // 无 bridge 时显式失败 (fail-closed), 绝不静默回退。
        #[cfg(not(feature = "magic-gateway"))]
        {
            return Err(GatewayError::classified(
                FINANCIAL_CAPABILITY,
                Some(ProviderId::Sina),
                "unavailable",
                "provider_transport",
                true,
                "library transport disabled: DATA_GATEWAY_GRPC=1 required",
            ));
        }
        #[cfg(feature = "magic-gateway")]
        {
            let worker_hash = request_hash.clone();
            let joined = tokio::task::spawn_blocking(move || {
                let result = build_instruments(&storage_codes, FINANCIAL_CAPABILITY)
                    .map(|instruments| route_financial_statements(&instruments, kind, Utc::now()));
                let (provider, result) = match result {
                    Ok(routed) => routed,
                    Err(error) => (ProviderId::Sina, Err(error)),
                };
                audit_gateway_result(FINANCIAL_CAPABILITY, provider, &worker_hash, result)
            })
            .await;
            match joined {
                Ok(result) => result,
                Err(error) => {
                    audit_blocking_join_failure(
                        FINANCIAL_CAPABILITY,
                        ProviderId::Sina,
                        request_hash,
                        error.to_string(),
                    )
                    .await
                }
            }
        }
    }

    pub async fn balance_sheets(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<FinancialStatement>, GatewayError> {
        self.financial_statements(codes, StatementKind::Balance)
            .await
    }

    pub async fn income_statements(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<FinancialStatement>, GatewayError> {
        self.financial_statements(codes, StatementKind::Income)
            .await
    }

    pub async fn cash_flow_statements(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<FinancialStatement>, GatewayError> {
        self.financial_statements(codes, StatementKind::CashFlow)
            .await
    }

    /// Fetches Tencent quote-adjacent statistics. Optional source fields remain
    /// optional in the returned `MarketStatistics`.
    ///
    /// BR-205 is Gate-A evidence only: this request has no exact trading-session
    /// input, so its output is not yet authority for dynamic order-price limits.
    pub async fn market_statistics(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<MarketStatistics>, GatewayError> {
        let storage_codes = codes.to_vec();
        let request_hash = acquisition_request_hash(STATISTICS_CAPABILITY, storage_codes.join(","));
        // P4 M3: gRPC 桥 (DATA_GATEWAY_GRPC=1 时替换 transport; audit 留客户端)。
        match super::grpc_source::bridge_for("MarketStatistics") {
            Ok(Some(bridge)) => {
                let result = bridge.market_statistics_async(&storage_codes).await;
                let audit_provider = result
                    .as_ref()
                    .map(|b| b.evidence().provider)
                    .unwrap_or(ProviderId::Tencent);
                return audit_gateway_result(
                    STATISTICS_CAPABILITY,
                    audit_provider,
                    &request_hash,
                    result,
                );
            }
            Ok(None) => {}
            Err(error) => {
                return audit_gateway_result(
                    STATISTICS_CAPABILITY,
                    ProviderId::Tencent,
                    &request_hash,
                    Err(error),
                );
            }
        }
        // no-feature (monitor 零 magic): library transport 不存在。
        // 无 bridge 时显式失败 (fail-closed), 绝不静默回退。
        #[cfg(not(feature = "magic-gateway"))]
        {
            return Err(GatewayError::classified(
                STATISTICS_CAPABILITY,
                Some(ProviderId::Tencent),
                "unavailable",
                "provider_transport",
                true,
                "library transport disabled: DATA_GATEWAY_GRPC=1 required",
            ));
        }
        #[cfg(feature = "magic-gateway")]
        {
            let worker_hash = request_hash.clone();
            let joined = tokio::task::spawn_blocking(move || {
                let result = build_instruments(&storage_codes, STATISTICS_CAPABILITY)
                    .map(|instruments| route_market_statistics(&instruments, Utc::now()));
                let (provider, result) = match result {
                    Ok(routed) => routed,
                    Err(error) => (ProviderId::Tencent, Err(error)),
                };
                audit_gateway_result(STATISTICS_CAPABILITY, provider, &worker_hash, result)
            })
            .await;
            match joined {
                Ok(result) => result,
                Err(error) => {
                    audit_blocking_join_failure(
                        STATISTICS_CAPABILITY,
                        ProviderId::Tencent,
                        request_hash,
                        error.to_string(),
                    )
                    .await
                }
            }
        }
    }
}

#[cfg(feature = "magic-gateway")]
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
        .map(|storage_code| {
            if !seen.insert(storage_code.as_str()) {
                return Err(GatewayError::invalid_request(
                    capability,
                    format!("duplicate A-share code {storage_code:?}"),
                ));
            }
            build_instrument(storage_code, capability)
        })
        .collect()
}

#[cfg(feature = "magic-gateway")]
fn build_instrument(
    storage_code: &str,
    capability: &'static str,
) -> Result<InstrumentId, GatewayError> {
    #[cfg(test)]
    let resolved = super::instrument_identity::resolve_test_equity(storage_code, None);
    #[cfg(not(test))]
    let resolved = super::instrument_identity::resolve_production_equity(storage_code, None);
    let identity =
        resolved.map_err(|error| GatewayError::invalid_request(capability, error.to_string()))?;
    identity
        .require_a_share()
        .map_err(|error| GatewayError::invalid_request(capability, error.to_string()))?;
    if capability == FINANCIAL_CAPABILITY && identity.instrument().exchange() == Exchange::Beijing {
        return Err(GatewayError::invalid_request(
            capability,
            "Sina financial statements support only Shanghai and Shenzhen A-shares",
        ));
    }
    Ok(identity.instrument().clone())
}

#[cfg(feature = "magic-gateway")]
fn route_financial_statements(
    instruments: &[InstrumentId],
    kind: StatementKind,
    now: DateTime<Utc>,
) -> (
    ProviderId,
    Result<GatewayBatch<FinancialStatement>, GatewayError>,
) {
    let mut router =
        FinancialStatementRouter::new(AcceptancePolicy::new().with_require_complete(true));
    let registration = SinaClient::new()
        .map_err(|error| {
            RouterError::InvalidConfiguration(format!(
                "Magic Sina financial client initialization failed: {error}"
            ))
        })
        .and_then(|client| {
            router.register(financial_statement_source(
                ProviderId::Sina,
                Arc::new(client),
                classify_sina_error,
            ))?;
            Ok(())
        });
    if let Err(error) = registration {
        return (
            ProviderId::Sina,
            Err(router_gateway_error(
                FINANCIAL_CAPABILITY,
                error,
                ProviderId::Sina,
            )),
        );
    }
    let request: FinancialStatementRequest = (instruments.to_vec(), kind);
    match router.route(&request) {
        Ok(outcome) => {
            let provider = outcome.selected_provider();
            (
                provider,
                admit_financial_batch(instruments, kind, provider, outcome.into_batch(), now),
            )
        }
        Err(error) => {
            let provider = terminal_provider(&error, ProviderId::Sina);
            (
                provider,
                Err(router_gateway_error(FINANCIAL_CAPABILITY, error, provider)),
            )
        }
    }
}

#[cfg(feature = "magic-gateway")]
fn route_market_statistics(
    instruments: &[InstrumentId],
    now: DateTime<Utc>,
) -> (
    ProviderId,
    Result<GatewayBatch<MarketStatistics>, GatewayError>,
) {
    let mut router = MarketStatisticsRouter::new(
        AcceptancePolicy::new()
            .with_require_complete(true)
            .with_require_source_at(true),
    );
    let registration = TencentClient::new()
        .map_err(|error| {
            RouterError::InvalidConfiguration(format!(
                "Magic Tencent statistics client initialization failed: {error}"
            ))
        })
        .and_then(|client| {
            router.register(market_statistics_source(
                ProviderId::Tencent,
                Arc::new(client),
                classify_tencent_error,
            ))?;
            Ok(())
        });
    if let Err(error) = registration {
        return (
            ProviderId::Tencent,
            Err(router_gateway_error(
                STATISTICS_CAPABILITY,
                error,
                ProviderId::Tencent,
            )),
        );
    }
    match router.route(instruments) {
        Ok(outcome) => {
            let provider = outcome.selected_provider();
            (
                provider,
                admit_statistics_batch(instruments, provider, outcome.into_batch(), now),
            )
        }
        Err(error) => {
            let provider = terminal_provider(&error, ProviderId::Tencent);
            (
                provider,
                Err(router_gateway_error(STATISTICS_CAPABILITY, error, provider)),
            )
        }
    }
}

#[cfg(feature = "magic-gateway")]
fn admit_financial_batch(
    instruments: &[InstrumentId],
    kind: StatementKind,
    provider: ProviderId,
    batch: DataBatch<FinancialStatement>,
    now: DateTime<Utc>,
) -> Result<GatewayBatch<FinancialStatement>, GatewayError> {
    require_complete_batch(FINANCIAL_CAPABILITY, provider, &batch)?;
    let mut evidence = BatchEvidence::from_provenance(provider, batch.provenance())?;
    let observed_at = parse_provider_timestamp(
        FINANCIAL_CAPABILITY,
        provider,
        &evidence.observed_at,
        "observed_at",
    )?;
    validate_observation_time(
        FINANCIAL_CAPABILITY,
        provider,
        observed_at,
        now,
        ACQUISITION_MAX_AGE_MILLIS,
    )?;
    if batch.records().is_empty() {
        return Err(GatewayError::classified(
            FINANCIAL_CAPABILITY,
            Some(provider),
            "unavailable",
            "verified_financial_batch_empty",
            true,
            "financial provider returned no statement periods",
        ));
    }

    let mut requested_index = 0_usize;
    let mut seen_instruments = HashSet::with_capacity(instruments.len());
    let mut previous_period: Option<NaiveDate> = None;
    let mut latest_source_date: Option<NaiveDate> = None;
    for statement in batch.records() {
        let position = instruments
            .iter()
            .position(|instrument| instrument == &statement.instrument)
            .ok_or_else(|| {
                GatewayError::invalid_evidence(
                    FINANCIAL_CAPABILITY,
                    Some(provider),
                    format!(
                        "financial batch contains unrequested instrument {}",
                        statement.instrument.code()
                    ),
                )
            })?;
        if position < requested_index || position > requested_index + 1 {
            return Err(GatewayError::invalid_evidence(
                FINANCIAL_CAPABILITY,
                Some(provider),
                "financial statements do not preserve requested instrument order",
            ));
        }
        if position == requested_index + 1 {
            requested_index = position;
            previous_period = None;
        }
        seen_instruments.insert(position);

        if statement.kind != kind
            || statement.evidence.provider() != provider
            || statement.evidence.batch_id() != evidence.batch_id
            || statement.evidence.observed_at() != evidence.observed_at
        {
            return Err(GatewayError::invalid_evidence(
                FINANCIAL_CAPABILITY,
                Some(provider),
                format!(
                    "financial identity/evidence mismatch for {}",
                    statement.instrument.code()
                ),
            ));
        }
        let report_period =
            parse_source_date(statement.report_period.as_str(), "report_period", provider)?;
        if previous_period.is_some_and(|previous| report_period >= previous) {
            return Err(GatewayError::invalid_evidence(
                FINANCIAL_CAPABILITY,
                Some(provider),
                format!(
                    "{} financial report periods are duplicated or not newest-first",
                    statement.instrument.code()
                ),
            ));
        }
        previous_period = Some(report_period);

        let announced = statement.announced_on.as_ref().ok_or_else(|| {
            GatewayError::invalid_evidence(
                FINANCIAL_CAPABILITY,
                Some(provider),
                format!(
                    "{} {} has no announcement/source date",
                    statement.instrument.code(),
                    statement.report_period
                ),
            )
        })?;
        let announced_on = parse_source_date(announced.as_str(), "announced_on", provider)?;
        if announced_on < report_period
            || announced_on > now.with_timezone(&shanghai_offset()).date_naive()
            || statement.evidence.source_at() != Some(announced.as_str())
        {
            return Err(GatewayError::invalid_evidence(
                FINANCIAL_CAPABILITY,
                Some(provider),
                format!(
                    "{} {} has contradictory report/source dates",
                    statement.instrument.code(),
                    statement.report_period
                ),
            ));
        }
        latest_source_date = Some(
            latest_source_date
                .map(|current| current.max(announced_on))
                .unwrap_or(announced_on),
        );
        validate_financial_lines(statement, provider)?;
    }
    if seen_instruments.len() != instruments.len() {
        return Err(GatewayError::invalid_evidence(
            FINANCIAL_CAPABILITY,
            Some(provider),
            format!(
                "financial instrument cardinality mismatch requested={} represented={}",
                instruments.len(),
                seen_instruments.len()
            ),
        ));
    }

    // Sina exposes publication dates per statement rather than at the outer
    // HTTP batch. The newest validated record publication date is retained as
    // the batch source date for BR-159 audit; no time-of-day is invented.
    evidence.source_at = Some(
        latest_source_date
            .ok_or_else(|| {
                GatewayError::invalid_evidence(
                    FINANCIAL_CAPABILITY,
                    Some(provider),
                    "financial batch has no validated source date",
                )
            })?
            .format("%Y-%m-%d")
            .to_string(),
    );
    Ok(GatewayBatch::Available {
        records: batch.into_records(),
        evidence,
    })
}

#[cfg(feature = "magic-gateway")]
fn validate_financial_lines(
    statement: &FinancialStatement,
    provider: ProviderId,
) -> Result<(), GatewayError> {
    if statement.lines.is_empty() {
        return Err(GatewayError::invalid_evidence(
            FINANCIAL_CAPABILITY,
            Some(provider),
            format!(
                "{} {} has no financial line facts",
                statement.instrument.code(),
                statement.report_period
            ),
        ));
    }
    let mut keys = HashSet::with_capacity(statement.lines.len());
    for line in &statement.lines {
        if !keys.insert(line.key.as_str()) {
            return Err(GatewayError::invalid_evidence(
                FINANCIAL_CAPABILITY,
                Some(provider),
                format!(
                    "{} {} duplicated financial key {:?}",
                    statement.instrument.code(),
                    statement.report_period,
                    line.key.as_str()
                ),
            ));
        }
    }
    Ok(())
}

#[cfg(feature = "magic-gateway")]
fn admit_statistics_batch(
    instruments: &[InstrumentId],
    provider: ProviderId,
    batch: DataBatch<MarketStatistics>,
    now: DateTime<Utc>,
) -> Result<GatewayBatch<MarketStatistics>, GatewayError> {
    require_complete_batch(STATISTICS_CAPABILITY, provider, &batch)?;
    let evidence = BatchEvidence::from_provenance(provider, batch.provenance())?;
    let observed_at = parse_provider_timestamp(
        STATISTICS_CAPABILITY,
        provider,
        &evidence.observed_at,
        "observed_at",
    )?;
    validate_observation_time(
        STATISTICS_CAPABILITY,
        provider,
        observed_at,
        now,
        REALTIME_MAX_AGE_MILLIS,
    )?;
    let batch_source_text = evidence.source_at.as_deref().ok_or_else(|| {
        GatewayError::invalid_evidence(
            STATISTICS_CAPABILITY,
            Some(provider),
            "statistics batch has no source timestamp",
        )
    })?;
    let batch_source_at = parse_provider_timestamp(
        STATISTICS_CAPABILITY,
        provider,
        batch_source_text,
        "batch_source_at",
    )?;
    validate_realtime_freshness(provider, batch_source_at, now, "statistics batch")?;
    if batch.records().len() != instruments.len() {
        return Err(GatewayError::invalid_evidence(
            STATISTICS_CAPABILITY,
            Some(provider),
            format!(
                "statistics cardinality mismatch requested={} actual={}",
                instruments.len(),
                batch.records().len()
            ),
        ));
    }

    let mut oldest_record_source: Option<DateTime<Utc>> = None;
    for (instrument, statistics) in instruments.iter().zip(batch.records()) {
        let record_evidence = statistics.evidence();
        if statistics.instrument() != instrument
            || record_evidence.provider() != provider
            || record_evidence.batch_id() != evidence.batch_id
            || record_evidence.observed_at() != evidence.observed_at
        {
            return Err(GatewayError::invalid_evidence(
                STATISTICS_CAPABILITY,
                Some(provider),
                format!(
                    "statistics identity/evidence mismatch for {}",
                    instrument.code()
                ),
            ));
        }
        let source_at_text = record_evidence.source_at().ok_or_else(|| {
            GatewayError::invalid_evidence(
                STATISTICS_CAPABILITY,
                Some(provider),
                format!(
                    "statistics source time unavailable for {}",
                    instrument.code()
                ),
            )
        })?;
        let source_at = parse_provider_timestamp(
            STATISTICS_CAPABILITY,
            provider,
            source_at_text,
            "record_source_at",
        )?;
        validate_realtime_freshness(provider, source_at, now, instrument.code())?;
        oldest_record_source = Some(
            oldest_record_source
                .map(|current| current.min(source_at))
                .unwrap_or(source_at),
        );
        validate_statistics_values(statistics, provider)?;
    }
    if oldest_record_source != Some(batch_source_at) {
        return Err(GatewayError::invalid_evidence(
            STATISTICS_CAPABILITY,
            Some(provider),
            "statistics batch source time is not the oldest record source time",
        ));
    }
    Ok(GatewayBatch::Available {
        records: batch.into_records(),
        evidence,
    })
}

#[cfg(feature = "magic-gateway")]
fn validate_statistics_values(
    statistics: &MarketStatistics,
    provider: ProviderId,
) -> Result<(), GatewayError> {
    if statistics
        .turnover_rate()
        .is_some_and(|ratio| ratio.unit() != RatioUnit::Percent || ratio.get() < 0.0)
    {
        return Err(GatewayError::invalid_evidence(
            STATISTICS_CAPABILITY,
            Some(provider),
            format!(
                "{} turnover rate has invalid unit/value",
                statistics.instrument().code()
            ),
        ));
    }
    if let (Some(total), Some(floating)) = (
        statistics.total_market_cap(),
        statistics.floating_market_cap(),
    ) {
        if floating.get() > total.get() {
            return Err(GatewayError::invalid_evidence(
                STATISTICS_CAPABILITY,
                Some(provider),
                format!(
                    "{} floating market cap exceeds total market cap",
                    statistics.instrument().code()
                ),
            ));
        }
    }
    if let (Some(upper), Some(lower)) = (statistics.upper_limit(), statistics.lower_limit()) {
        if upper.get() < lower.get() {
            return Err(GatewayError::invalid_evidence(
                STATISTICS_CAPABILITY,
                Some(provider),
                format!(
                    "{} upper limit is below lower limit",
                    statistics.instrument().code()
                ),
            ));
        }
    }
    let any_available = statistics.turnover_rate().is_some()
        || statistics.trailing_pe().is_some()
        || statistics.static_pe().is_some()
        || statistics.pb().is_some()
        || statistics.total_market_cap().is_some()
        || statistics.floating_market_cap().is_some()
        || statistics.upper_limit().is_some()
        || statistics.lower_limit().is_some()
        || statistics.volume_ratio().is_some();
    if !any_available {
        return Err(GatewayError::invalid_evidence(
            STATISTICS_CAPABILITY,
            Some(provider),
            format!(
                "{} statistics has no available source field",
                statistics.instrument().code()
            ),
        ));
    }
    Ok(())
}

#[cfg(feature = "magic-gateway")]
fn require_complete_batch<T>(
    capability: &'static str,
    provider: ProviderId,
    batch: &DataBatch<T>,
) -> Result<(), GatewayError> {
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
    Ok(())
}

#[cfg(feature = "magic-gateway")]
fn parse_source_date(
    value: &str,
    field: &'static str,
    provider: ProviderId,
) -> Result<NaiveDate, GatewayError> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|error| {
        GatewayError::invalid_evidence(
            FINANCIAL_CAPABILITY,
            Some(provider),
            format!("invalid {field} date {value:?}: {error}"),
        )
    })
}

#[cfg(feature = "magic-gateway")]
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

#[cfg(feature = "magic-gateway")]
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

#[cfg(feature = "magic-gateway")]
fn shanghai_offset() -> FixedOffset {
    FixedOffset::east_opt(SHANGHAI_OFFSET_SECONDS)
        .expect("Shanghai UTC offset is a compile-time valid constant")
}

#[cfg(feature = "magic-gateway")]
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

#[cfg(feature = "magic-gateway")]
fn validate_realtime_freshness(
    provider: ProviderId,
    source_at: DateTime<Utc>,
    now: DateTime<Utc>,
    identity: &str,
) -> Result<(), GatewayError> {
    let age = now.signed_duration_since(source_at).num_milliseconds();
    if !(0..=REALTIME_MAX_AGE_MILLIS).contains(&age) {
        return Err(GatewayError::classified(
            STATISTICS_CAPABILITY,
            Some(provider),
            "stale",
            "statistics_source_stale",
            true,
            format!("{identity} failed five-second source freshness gate age_ms={age}"),
        ));
    }
    Ok(())
}

#[cfg(feature = "magic-gateway")]
fn terminal_provider(error: &RouterError, default: ProviderId) -> ProviderId {
    error
        .attempts()
        .last()
        .map(|attempt| attempt.provider_id())
        .unwrap_or(default)
}

#[cfg(feature = "magic-gateway")]
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

#[cfg(feature = "magic-gateway")]
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

#[cfg(feature = "magic-gateway")]
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

#[cfg(test)]
#[cfg(feature = "magic-gateway")]
mod tests {
    use super::*;
    use crate::magic_compat::{
        Exchange, FiniteNumber, IsoDate, Money, NonEmptyText, Price, Provenance, Ratio,
        SourceEvidence,
    };

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-07-24T09:30:02+08:00")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn observed_epoch() -> String {
        now().timestamp().to_string()
    }

    fn source_at() -> String {
        "2026-07-24T09:30:00+08:00".to_owned()
    }

    fn sh() -> InstrumentId {
        build_instrument("TEST_CODE_600396", FINANCIAL_CAPABILITY).unwrap()
    }

    fn sz() -> InstrumentId {
        build_instrument("TEST_CODE_002421", FINANCIAL_CAPABILITY).unwrap()
    }

    fn financial_statement(
        instrument: InstrumentId,
        period: &str,
        announced_on: &str,
        kind: StatementKind,
        batch_id: &str,
    ) -> FinancialStatement {
        FinancialStatement {
            instrument,
            kind,
            report_period: IsoDate::new(period).unwrap(),
            announced_on: Some(IsoDate::new(announced_on).unwrap()),
            currency: Some(NonEmptyText::new("CNY").unwrap()),
            lines: vec![
                FinancialLine {
                    key: NonEmptyText::new("revenue").unwrap(),
                    source_label: NonEmptyText::new("营业收入").unwrap(),
                    value: Some(FiniteNumber::new(100.0).unwrap()),
                    unit: None,
                },
                FinancialLine {
                    key: NonEmptyText::new("optional_fact").unwrap(),
                    source_label: NonEmptyText::new("可选事实").unwrap(),
                    value: None,
                    unit: None,
                },
            ],
            evidence: SourceEvidence::new(ProviderId::Sina, observed_epoch(), batch_id)
                .unwrap()
                .with_source_at(announced_on)
                .unwrap(),
        }
    }

    #[test]
    fn br164_company_provider_orders_are_explicit() {
        assert_eq!(FINANCIAL_STATEMENT_PROVIDER_ORDER, &[ProviderId::Sina]);
        assert_eq!(MARKET_STATISTICS_PROVIDER_ORDER, &[ProviderId::Tencent]);
    }

    #[test]
    fn br164_financial_seam_preserves_raw_lines_missing_values_and_order() {
        let batch_id = "TEST_CODE_financial_batch";
        let records = vec![
            financial_statement(
                sh(),
                "2026-03-31",
                "2026-04-30",
                StatementKind::Income,
                batch_id,
            ),
            financial_statement(
                sh(),
                "2025-12-31",
                "2026-03-30",
                StatementKind::Income,
                batch_id,
            ),
            financial_statement(
                sz(),
                "2026-03-31",
                "2026-04-29",
                StatementKind::Income,
                batch_id,
            ),
        ];
        let provenance = Provenance::new("TEST_CODE_sina", observed_epoch())
            .unwrap()
            .with_batch_id(batch_id)
            .unwrap();
        let batch = DataBatch::strict(records, provenance);
        let admitted = admit_financial_batch(
            &[sh(), sz()],
            StatementKind::Income,
            ProviderId::Sina,
            batch,
            now(),
        )
        .unwrap();
        assert_eq!(admitted.records()[0].instrument, sh());
        assert_eq!(admitted.records()[1].report_period.as_str(), "2025-12-31");
        assert!(admitted.records()[0].lines[1].value.is_none());
        assert_eq!(admitted.evidence().source_at.as_deref(), Some("2026-04-30"));
    }

    #[test]
    fn br164_financial_seam_rejects_period_reordering_and_missing_source_date() {
        let batch_id = "TEST_CODE_financial_bad";
        let reordered = vec![
            financial_statement(
                sh(),
                "2025-12-31",
                "2026-03-30",
                StatementKind::Balance,
                batch_id,
            ),
            financial_statement(
                sh(),
                "2026-03-31",
                "2026-04-30",
                StatementKind::Balance,
                batch_id,
            ),
        ];
        let provenance = Provenance::new("TEST_CODE_sina", observed_epoch())
            .unwrap()
            .with_batch_id(batch_id)
            .unwrap();
        assert!(admit_financial_batch(
            &[sh()],
            StatementKind::Balance,
            ProviderId::Sina,
            DataBatch::strict(reordered, provenance),
            now(),
        )
        .is_err());

        let mut missing = financial_statement(
            sh(),
            "2026-03-31",
            "2026-04-30",
            StatementKind::CashFlow,
            batch_id,
        );
        missing.announced_on = None;
        let provenance = Provenance::new("TEST_CODE_sina", observed_epoch())
            .unwrap()
            .with_batch_id(batch_id)
            .unwrap();
        assert!(admit_financial_batch(
            &[sh()],
            StatementKind::CashFlow,
            ProviderId::Sina,
            DataBatch::strict(vec![missing], provenance),
            now(),
        )
        .is_err());
    }

    fn statistics_evidence(batch_id: &str) -> SourceEvidence {
        SourceEvidence::new(ProviderId::Tencent, observed_epoch(), batch_id)
            .unwrap()
            .with_source_at(source_at())
            .unwrap()
    }

    fn financial_provenance(batch_id: &str) -> Provenance {
        Provenance::new("TEST_CODE_sina", observed_epoch())
            .unwrap()
            .with_batch_id(batch_id)
            .unwrap()
    }

    fn statistics_record(
        instrument: InstrumentId,
        batch_id: &str,
        observed_at: &str,
        source_at: Option<&str>,
    ) -> MarketStatistics {
        let mut evidence = SourceEvidence::new(ProviderId::Tencent, observed_at, batch_id).unwrap();
        if let Some(source_at) = source_at {
            evidence = evidence.with_source_at(source_at).unwrap();
        }
        MarketStatistics::new(
            instrument,
            Some(Ratio::new(2.5, RatioUnit::Percent).unwrap()),
            Some(FiniteNumber::new(12.0).unwrap()),
            Some(FiniteNumber::new(10.0).unwrap()),
            Some(FiniteNumber::new(1.5).unwrap()),
            Some(Money::new(10_000_000_000.0).unwrap()),
            Some(Money::new(8_000_000_000.0).unwrap()),
            Some(Price::new(11.0).unwrap()),
            Some(Price::new(9.0).unwrap()),
            Some(FiniteNumber::new(1.2).unwrap()),
            evidence,
        )
        .unwrap()
    }

    fn statistics_provenance(
        batch_id: &str,
        observed_at: &str,
        source_at: Option<&str>,
    ) -> Provenance {
        let mut provenance = Provenance::new("TEST_CODE_tencent", observed_at)
            .unwrap()
            .with_batch_id(batch_id)
            .unwrap();
        if let Some(source_at) = source_at {
            provenance = provenance.with_source_at(source_at).unwrap();
        }
        provenance
    }

    #[test]
    fn br164_statistics_seam_preserves_optional_missing_fields() {
        let batch_id = "TEST_CODE_statistics_batch";
        let statistics = MarketStatistics::new(
            sh(),
            Some(Ratio::new(2.5, RatioUnit::Percent).unwrap()),
            None,
            Some(FiniteNumber::new(20.0).unwrap()),
            Some(FiniteNumber::new(1.5).unwrap()),
            Some(Money::new(10_000_000_000.0).unwrap()),
            Some(Money::new(8_000_000_000.0).unwrap()),
            Some(Price::new(11.0).unwrap()),
            Some(Price::new(9.0).unwrap()),
            None,
            statistics_evidence(batch_id),
        )
        .unwrap();
        let provenance = Provenance::new("TEST_CODE_tencent", observed_epoch())
            .unwrap()
            .with_source_at(source_at())
            .unwrap()
            .with_batch_id(batch_id)
            .unwrap();
        let admitted = admit_statistics_batch(
            &[sh()],
            ProviderId::Tencent,
            DataBatch::strict(vec![statistics], provenance),
            now(),
        )
        .unwrap();
        assert!(admitted.records()[0].trailing_pe().is_none());
        assert!(admitted.records()[0].volume_ratio().is_none());
        assert_eq!(admitted.records()[0].pb().map(FiniteNumber::get), Some(1.5));
    }

    #[test]
    fn br164_statistics_seam_rejects_stale_source_and_bad_identity() {
        let batch_id = "TEST_CODE_statistics_stale";
        let stale_source = "2026-07-24T09:29:50+08:00";
        let statistics = MarketStatistics::new(
            sz(),
            None,
            Some(FiniteNumber::new(10.0).unwrap()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            SourceEvidence::new(ProviderId::Tencent, observed_epoch(), batch_id)
                .unwrap()
                .with_source_at(stale_source)
                .unwrap(),
        )
        .unwrap();
        let provenance = Provenance::new("TEST_CODE_tencent", observed_epoch())
            .unwrap()
            .with_source_at(stale_source)
            .unwrap()
            .with_batch_id(batch_id)
            .unwrap();
        assert!(admit_statistics_batch(
            &[sh()],
            ProviderId::Tencent,
            DataBatch::strict(vec![statistics], provenance),
            now(),
        )
        .is_err());
    }

    #[test]
    fn br164_company_requests_reject_empty_duplicate_and_invalid_codes() {
        assert!(build_instruments(&[], FINANCIAL_CAPABILITY).is_err());
        assert!(build_instruments(
            &["TEST_CODE_600396".to_owned(), "TEST_CODE_600396".to_owned()],
            STATISTICS_CAPABILITY,
        )
        .is_err());
        assert!(build_instrument("TEST_CODE_BAD", FINANCIAL_CAPABILITY).is_err());
    }

    #[test]
    fn br164_company_instruments_enforce_provider_exchange_capabilities() {
        assert_eq!(
            build_instrument("TEST_CODE_600396", FINANCIAL_CAPABILITY)
                .unwrap()
                .exchange(),
            Exchange::Shanghai
        );
        assert_eq!(
            build_instrument("TEST_CODE_000001", FINANCIAL_CAPABILITY)
                .unwrap()
                .exchange(),
            Exchange::Shenzhen
        );
        assert!(build_instrument("TEST_CODE_920047", FINANCIAL_CAPABILITY).is_err());
        assert_eq!(
            build_instrument("TEST_CODE_920047", STATISTICS_CAPABILITY)
                .unwrap()
                .exchange(),
            Exchange::Beijing
        );
        for code in [
            "TEST_CODE_430047",
            "TEST_CODE_830047",
            "TEST_CODE_200001",
            "TEST_CODE_900901",
        ] {
            assert!(
                build_instrument(code, FINANCIAL_CAPABILITY).is_err(),
                "{code}"
            );
        }
        assert!(build_instrument("TEST_CODE_100001", FINANCIAL_CAPABILITY).is_err());
        assert!(build_instrument("TEST_CODE_60039A", FINANCIAL_CAPABILITY).is_err());
    }

    #[test]
    fn br164_company_provider_timestamp_parser_accepts_declared_formats_only() {
        let rfc3339 = parse_provider_timestamp(
            STATISTICS_CAPABILITY,
            ProviderId::Tencent,
            "2026-07-24T09:30:00+08:00",
            "source_at",
        )
        .unwrap();
        assert_eq!(rfc3339.timestamp(), 1_784_856_600);

        let seconds = parse_provider_timestamp(
            STATISTICS_CAPABILITY,
            ProviderId::Tencent,
            "1784856600",
            "observed_at",
        )
        .unwrap();
        assert_eq!(seconds.timestamp(), 1_784_856_600);
        let fractional = parse_provider_timestamp(
            STATISTICS_CAPABILITY,
            ProviderId::Tencent,
            "1784856600.25",
            "observed_at",
        )
        .unwrap();
        assert_eq!(fractional.timestamp_subsec_nanos(), 250_000_000);

        for invalid in [
            "TEST_CODE_invalid",
            "1784856600.",
            "1784856600.1234567890",
            "1784856600.TEST",
        ] {
            let error = parse_provider_timestamp(
                STATISTICS_CAPABILITY,
                ProviderId::Tencent,
                invalid,
                "observed_at",
            )
            .unwrap_err();
            assert_eq!(error.reason_code(), "invalid_evidence");
        }
        assert!(parse_source_date("2026-07-24", "announced_on", ProviderId::Sina).is_ok());
        assert!(parse_source_date("2026/07/24", "announced_on", ProviderId::Sina).is_err());
    }

    #[test]
    fn br164_company_freshness_rejects_future_and_expired_observations() {
        let current = now();
        assert!(validate_observation_time(
            STATISTICS_CAPABILITY,
            ProviderId::Tencent,
            current - chrono::Duration::seconds(2),
            current,
            3_000,
        )
        .is_ok());
        let stale = validate_observation_time(
            STATISTICS_CAPABILITY,
            ProviderId::Tencent,
            current - chrono::Duration::seconds(4),
            current,
            3_000,
        )
        .unwrap_err();
        assert_eq!(stale.reason_code(), "observation_stale");
        assert!(stale.retryable());
        assert!(validate_observation_time(
            STATISTICS_CAPABILITY,
            ProviderId::Tencent,
            current + chrono::Duration::milliseconds(1),
            current,
            3_000,
        )
        .is_err());

        assert!(validate_realtime_freshness(
            ProviderId::Tencent,
            current - chrono::Duration::seconds(5),
            current,
            "TEST_CODE_600396",
        )
        .is_ok());
        let stale_source = validate_realtime_freshness(
            ProviderId::Tencent,
            current - chrono::Duration::milliseconds(5_001),
            current,
            "TEST_CODE_600396",
        )
        .unwrap_err();
        assert_eq!(stale_source.reason_code(), "statistics_source_stale");
    }

    #[test]
    fn br164_company_partial_batches_and_provider_failures_remain_explicit() {
        let provenance = Provenance::new("TEST_CODE_sina", observed_epoch())
            .unwrap()
            .with_batch_id("TEST_CODE_partial_company")
            .unwrap();
        let partial = DataBatch::<FinancialStatement>::best_effort(
            Vec::new(),
            provenance,
            vec!["TEST_CODE missing page".to_owned()],
        )
        .unwrap();
        let error =
            require_complete_batch(FINANCIAL_CAPABILITY, ProviderId::Sina, &partial).unwrap_err();
        assert_eq!(error.reason_code(), "source_batch_incomplete");
        assert!(!error.retryable());

        assert_eq!(
            classify_sina_error(SinaError::InvalidRequest("TEST_CODE".to_owned())).kind(),
            FailureKind::InvalidRequest
        );
        assert_eq!(
            classify_sina_error(SinaError::Transport("TEST_CODE".to_owned())).kind(),
            FailureKind::Transport
        );
        assert_eq!(
            classify_tencent_error(TencentError::Protocol("TEST_CODE".to_owned())).kind(),
            FailureKind::Protocol
        );
        assert_eq!(
            classify_tencent_error(TencentError::Unsupported("TEST_CODE".to_owned())).kind(),
            FailureKind::Unsupported
        );
    }

    #[test]
    fn br164_financial_empty_batch_is_retryable_unavailable_not_verified_empty() {
        let error = admit_financial_batch(
            &[sh()],
            StatementKind::Income,
            ProviderId::Sina,
            DataBatch::strict(
                Vec::new(),
                financial_provenance("TEST_CODE_financial_empty"),
            ),
            now(),
        )
        .unwrap_err();

        assert_eq!(error.audit_outcome(), "unavailable");
        assert_eq!(error.reason_code(), "verified_financial_batch_empty");
        assert!(error.retryable());
    }

    #[test]
    fn br164_financial_admission_rejects_unrequested_reordered_and_missing_instruments() {
        let unrequested_batch_id = "TEST_CODE_financial_unrequested";
        let unrequested = admit_financial_batch(
            &[sh()],
            StatementKind::Income,
            ProviderId::Sina,
            DataBatch::strict(
                vec![financial_statement(
                    sz(),
                    "2026-03-31",
                    "2026-04-29",
                    StatementKind::Income,
                    unrequested_batch_id,
                )],
                financial_provenance(unrequested_batch_id),
            ),
            now(),
        )
        .unwrap_err();
        assert_eq!(unrequested.reason_code(), "invalid_evidence");

        let reordered_batch_id = "TEST_CODE_financial_instrument_order";
        let reordered = admit_financial_batch(
            &[sh(), sz()],
            StatementKind::Income,
            ProviderId::Sina,
            DataBatch::strict(
                vec![
                    financial_statement(
                        sz(),
                        "2026-03-31",
                        "2026-04-29",
                        StatementKind::Income,
                        reordered_batch_id,
                    ),
                    financial_statement(
                        sh(),
                        "2026-03-31",
                        "2026-04-30",
                        StatementKind::Income,
                        reordered_batch_id,
                    ),
                ],
                financial_provenance(reordered_batch_id),
            ),
            now(),
        )
        .unwrap_err();
        assert_eq!(reordered.reason_code(), "invalid_evidence");

        let missing_batch_id = "TEST_CODE_financial_cardinality";
        let missing = admit_financial_batch(
            &[sh(), sz()],
            StatementKind::Income,
            ProviderId::Sina,
            DataBatch::strict(
                vec![financial_statement(
                    sh(),
                    "2026-03-31",
                    "2026-04-30",
                    StatementKind::Income,
                    missing_batch_id,
                )],
                financial_provenance(missing_batch_id),
            ),
            now(),
        )
        .unwrap_err();
        assert_eq!(missing.reason_code(), "invalid_evidence");
    }

    #[test]
    fn br164_financial_admission_rejects_record_evidence_and_line_fact_conflicts() {
        let mismatched_batch_id = "TEST_CODE_financial_evidence";
        let mut wrong_kind = financial_statement(
            sh(),
            "2026-03-31",
            "2026-04-30",
            StatementKind::Balance,
            mismatched_batch_id,
        );
        let error = admit_financial_batch(
            &[sh()],
            StatementKind::Income,
            ProviderId::Sina,
            DataBatch::strict(
                vec![wrong_kind.clone()],
                financial_provenance(mismatched_batch_id),
            ),
            now(),
        )
        .unwrap_err();
        assert_eq!(error.reason_code(), "invalid_evidence");

        wrong_kind.kind = StatementKind::Income;
        wrong_kind.evidence =
            SourceEvidence::new(ProviderId::Tencent, observed_epoch(), mismatched_batch_id)
                .unwrap()
                .with_source_at("2026-04-30")
                .unwrap();
        let error = admit_financial_batch(
            &[sh()],
            StatementKind::Income,
            ProviderId::Sina,
            DataBatch::strict(vec![wrong_kind], financial_provenance(mismatched_batch_id)),
            now(),
        )
        .unwrap_err();
        assert_eq!(error.reason_code(), "invalid_evidence");

        let empty_lines_batch_id = "TEST_CODE_financial_empty_lines";
        let mut empty_lines = financial_statement(
            sh(),
            "2026-03-31",
            "2026-04-30",
            StatementKind::Income,
            empty_lines_batch_id,
        );
        empty_lines.lines.clear();
        let error = admit_financial_batch(
            &[sh()],
            StatementKind::Income,
            ProviderId::Sina,
            DataBatch::strict(
                vec![empty_lines],
                financial_provenance(empty_lines_batch_id),
            ),
            now(),
        )
        .unwrap_err();
        assert_eq!(error.reason_code(), "invalid_evidence");

        let duplicate_lines_batch_id = "TEST_CODE_financial_duplicate_lines";
        let mut duplicate_lines = financial_statement(
            sh(),
            "2026-03-31",
            "2026-04-30",
            StatementKind::Income,
            duplicate_lines_batch_id,
        );
        duplicate_lines.lines.push(duplicate_lines.lines[0].clone());
        let error = admit_financial_batch(
            &[sh()],
            StatementKind::Income,
            ProviderId::Sina,
            DataBatch::strict(
                vec![duplicate_lines],
                financial_provenance(duplicate_lines_batch_id),
            ),
            now(),
        )
        .unwrap_err();
        assert_eq!(error.reason_code(), "invalid_evidence");
    }

    #[test]
    fn br164_financial_admission_rejects_contradictory_and_future_source_dates() {
        let before_period_batch_id = "TEST_CODE_financial_before_period";
        let before_period = admit_financial_batch(
            &[sh()],
            StatementKind::Income,
            ProviderId::Sina,
            DataBatch::strict(
                vec![financial_statement(
                    sh(),
                    "2026-03-31",
                    "2026-03-30",
                    StatementKind::Income,
                    before_period_batch_id,
                )],
                financial_provenance(before_period_batch_id),
            ),
            now(),
        )
        .unwrap_err();
        assert_eq!(before_period.reason_code(), "invalid_evidence");

        let future_batch_id = "TEST_CODE_financial_future";
        let future = admit_financial_batch(
            &[sh()],
            StatementKind::Income,
            ProviderId::Sina,
            DataBatch::strict(
                vec![financial_statement(
                    sh(),
                    "2026-03-31",
                    "2026-07-25",
                    StatementKind::Income,
                    future_batch_id,
                )],
                financial_provenance(future_batch_id),
            ),
            now(),
        )
        .unwrap_err();
        assert_eq!(future.reason_code(), "invalid_evidence");
    }

    #[test]
    fn br164_statistics_admission_rejects_missing_and_conflicting_evidence() {
        let missing_batch_source_id = "TEST_CODE_statistics_no_batch_source";
        let missing_batch_source = admit_statistics_batch(
            &[sh()],
            ProviderId::Tencent,
            DataBatch::strict(
                vec![statistics_record(
                    sh(),
                    missing_batch_source_id,
                    &observed_epoch(),
                    Some(&source_at()),
                )],
                statistics_provenance(missing_batch_source_id, &observed_epoch(), None),
            ),
            now(),
        )
        .unwrap_err();
        assert_eq!(missing_batch_source.reason_code(), "invalid_evidence");

        let missing_record_source_id = "TEST_CODE_statistics_no_record_source";
        let missing_record_source = admit_statistics_batch(
            &[sh()],
            ProviderId::Tencent,
            DataBatch::strict(
                vec![statistics_record(
                    sh(),
                    missing_record_source_id,
                    &observed_epoch(),
                    None,
                )],
                statistics_provenance(
                    missing_record_source_id,
                    &observed_epoch(),
                    Some(&source_at()),
                ),
            ),
            now(),
        )
        .unwrap_err();
        assert_eq!(missing_record_source.reason_code(), "invalid_evidence");

        let wrong_identity_id = "TEST_CODE_statistics_wrong_identity";
        let wrong_identity = admit_statistics_batch(
            &[sh()],
            ProviderId::Tencent,
            DataBatch::strict(
                vec![statistics_record(
                    sz(),
                    wrong_identity_id,
                    &observed_epoch(),
                    Some(&source_at()),
                )],
                statistics_provenance(wrong_identity_id, &observed_epoch(), Some(&source_at())),
            ),
            now(),
        )
        .unwrap_err();
        assert_eq!(wrong_identity.reason_code(), "invalid_evidence");
    }

    #[test]
    fn br164_statistics_admission_enforces_cardinality_and_oldest_source_time() {
        let cardinality_id = "TEST_CODE_statistics_cardinality";
        let cardinality = admit_statistics_batch(
            &[sh(), sz()],
            ProviderId::Tencent,
            DataBatch::strict(
                vec![statistics_record(
                    sh(),
                    cardinality_id,
                    &observed_epoch(),
                    Some(&source_at()),
                )],
                statistics_provenance(cardinality_id, &observed_epoch(), Some(&source_at())),
            ),
            now(),
        )
        .unwrap_err();
        assert_eq!(cardinality.reason_code(), "invalid_evidence");

        let oldest_id = "TEST_CODE_statistics_oldest_source";
        let newer_source = "2026-07-24T09:30:01+08:00";
        let oldest_mismatch = admit_statistics_batch(
            &[sh(), sz()],
            ProviderId::Tencent,
            DataBatch::strict(
                vec![
                    statistics_record(sh(), oldest_id, &observed_epoch(), Some(&source_at())),
                    statistics_record(sz(), oldest_id, &observed_epoch(), Some(newer_source)),
                ],
                statistics_provenance(oldest_id, &observed_epoch(), Some(newer_source)),
            ),
            now(),
        )
        .unwrap_err();
        assert_eq!(oldest_mismatch.reason_code(), "invalid_evidence");
    }

    #[test]
    fn br164_statistics_value_admission_rejects_semantic_conflicts_and_empty_facts() {
        let evidence = statistics_evidence("TEST_CODE_statistics_value");
        let wrong_turnover_unit = MarketStatistics::new(
            sh(),
            Some(Ratio::new(0.025, RatioUnit::Decimal).unwrap()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            evidence.clone(),
        )
        .unwrap();
        assert_eq!(
            validate_statistics_values(&wrong_turnover_unit, ProviderId::Tencent)
                .unwrap_err()
                .reason_code(),
            "invalid_evidence"
        );

        let cap_conflict = MarketStatistics::new(
            sh(),
            None,
            None,
            None,
            None,
            Some(Money::new(8_000_000_000.0).unwrap()),
            Some(Money::new(10_000_000_000.0).unwrap()),
            None,
            None,
            None,
            evidence.clone(),
        )
        .unwrap();
        assert_eq!(
            validate_statistics_values(&cap_conflict, ProviderId::Tencent)
                .unwrap_err()
                .reason_code(),
            "invalid_evidence"
        );

        let price_limit_conflict = MarketStatistics::new(
            sh(),
            None,
            None,
            None,
            None,
            None,
            None,
            Some(Price::new(9.0).unwrap()),
            Some(Price::new(11.0).unwrap()),
            None,
            evidence.clone(),
        )
        .unwrap();
        assert_eq!(
            validate_statistics_values(&price_limit_conflict, ProviderId::Tencent)
                .unwrap_err()
                .reason_code(),
            "invalid_evidence"
        );

        let empty = MarketStatistics::new(
            sh(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            evidence,
        )
        .unwrap();
        assert_eq!(
            validate_statistics_values(&empty, ProviderId::Tencent)
                .unwrap_err()
                .reason_code(),
            "invalid_evidence"
        );
    }

    #[test]
    fn br164_company_error_classifiers_cover_all_provider_variants_and_actions() {
        use magic_market_router::FailureAction;

        assert_eq!(
            classify_sina_error(SinaError::InvalidRequest("TEST_CODE".to_owned())).action(),
            FailureAction::Stop
        );
        for error in [
            SinaError::Transport("TEST_CODE".to_owned()),
            SinaError::Decode("TEST_CODE".to_owned()),
            SinaError::Protocol("TEST_CODE".to_owned()),
            SinaError::Unsupported("TEST_CODE".to_owned()),
            SinaError::Core(crate::magic_compat::CoreError::InvalidRequest(
                "TEST_CODE".to_owned(),
            )),
        ] {
            assert_eq!(classify_sina_error(error).action(), FailureAction::TryNext);
        }

        assert_eq!(
            classify_tencent_error(TencentError::InvalidRequest("TEST_CODE".to_owned())).action(),
            FailureAction::Stop
        );
        for error in [
            TencentError::Transport("TEST_CODE".to_owned()),
            TencentError::Decode("TEST_CODE".to_owned()),
            TencentError::Protocol("TEST_CODE".to_owned()),
            TencentError::Unsupported("TEST_CODE".to_owned()),
            TencentError::Core(crate::magic_compat::CoreError::InvalidRequest(
                "TEST_CODE".to_owned(),
            )),
        ] {
            assert_eq!(
                classify_tencent_error(error).action(),
                FailureAction::TryNext
            );
        }
    }

    #[test]
    fn br164_router_failures_keep_auditable_outcome_reason_and_retryability() {
        use magic_market_router::{FailoverChain, FailureAction, SourceFn};

        fn routed_error(kind: FailureKind, action: FailureAction) -> RouterError {
            let mut router = FailoverChain::<(), FinancialStatement>::new(AcceptancePolicy::new());
            router
                .register(SourceFn::new(ProviderId::Sina, move |_: &()| {
                    Err(SourceError::new(kind, action, "TEST_CODE route failure"))
                }))
                .unwrap();
            router.route(&()).unwrap_err()
        }

        let invalid_configuration =
            RouterError::InvalidConfiguration("TEST_CODE no source".to_owned());
        assert_eq!(
            terminal_provider(&invalid_configuration, ProviderId::Tencent),
            ProviderId::Tencent
        );
        let invalid = router_gateway_error(
            FINANCIAL_CAPABILITY,
            invalid_configuration,
            ProviderId::Sina,
        );
        assert_eq!(invalid.audit_outcome(), "invalid_request");
        assert_eq!(invalid.reason_code(), "router_invalid_request");
        assert!(!invalid.retryable());

        let unsupported_router = routed_error(FailureKind::Unsupported, FailureAction::TryNext);
        assert_eq!(
            terminal_provider(&unsupported_router, ProviderId::Tencent),
            ProviderId::Sina
        );
        let unsupported =
            router_gateway_error(FINANCIAL_CAPABILITY, unsupported_router, ProviderId::Sina);
        assert_eq!(unsupported.audit_outcome(), "unsupported");
        assert_eq!(unsupported.reason_code(), "router_unsupported");
        assert!(!unsupported.retryable());

        for kind in [
            FailureKind::Transport,
            FailureKind::Timeout,
            FailureKind::RateLimited,
            FailureKind::Provider,
            FailureKind::NoData,
        ] {
            let error = router_gateway_error(
                FINANCIAL_CAPABILITY,
                routed_error(kind, FailureAction::TryNext),
                ProviderId::Sina,
            );
            assert_eq!(error.audit_outcome(), "unavailable");
            assert_eq!(error.reason_code(), "router_sources_exhausted");
            assert!(error.retryable());
        }

        for kind in [
            FailureKind::Protocol,
            FailureKind::Quality,
            FailureKind::Evidence,
        ] {
            let error = router_gateway_error(
                FINANCIAL_CAPABILITY,
                routed_error(kind, FailureAction::TryNext),
                ProviderId::Sina,
            );
            assert_eq!(error.audit_outcome(), "partial");
            assert_eq!(error.reason_code(), "router_batch_rejected");
            assert!(!error.retryable());
        }
    }
}
