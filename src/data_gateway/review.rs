//! Registered business rules: BR-158, BR-159, BR-213.
//! Unified A-01/R-03 provider admission, evidence retention and acquisition audit.

#[cfg(feature = "magic-gateway")]
use crate::magic_compat::IsoDate;
use crate::magic_compat::ProviderId;
use crate::magic_compat::{LimitPoolEntry, LimitPoolKind, PositiveU32};
use chrono::NaiveDate;
#[cfg(feature = "magic-gateway")]
use magic_eastmoney_rs::{EastmoneyClient, EastmoneyError};
#[cfg(feature = "magic-gateway")]
use magic_market_core::{LimitPoolRequest, LimitPools};
#[cfg(feature = "magic-gateway")]
use magic_market_router::{
    AcceptancePolicy, AttemptStatus, FailureKind, LimitPoolRouter, RouterError, SourceError,
    SourceFn,
};
#[cfg(feature = "magic-gateway")]
use magic_ths_rs::{ThsClient, ThsError};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fmt;
use thiserror::Error;

use super::historical_bars::{AdmittedDailyBars, HistoricalBarsGateway};

const WHOLE_LIMIT_POOL_BOUND: u32 = 200;

/// Provider and acquisition facts retained for every accepted or verified-empty batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchEvidence {
    pub provider: ProviderId,
    pub source: String,
    pub source_at: Option<String>,
    pub observed_at: String,
    pub batch_id: String,
}

impl BatchEvidence {
    pub fn from_provenance(
        provider: ProviderId,
        provenance: &crate::magic_compat::Provenance,
    ) -> Result<Self, GatewayError> {
        let batch_id = provenance.batch_id().ok_or_else(|| {
            GatewayError::invalid_evidence(
                "review",
                Some(provider),
                "batch provenance has no batch ID",
            )
        })?;
        Ok(Self {
            provider,
            source: provenance.source().to_string(),
            source_at: provenance.source_at().map(str::to_string),
            observed_at: provenance.fetched_at().to_string(),
            batch_id: batch_id.to_string(),
        })
    }
}

/// A provider result that does not collapse a proven empty response into unavailability.
#[derive(Debug, Clone, PartialEq)]
pub enum GatewayBatch<T> {
    Available {
        records: Vec<T>,
        evidence: BatchEvidence,
    },
    VerifiedEmpty(BatchEvidence),
}

impl<T> GatewayBatch<T> {
    pub fn evidence(&self) -> &BatchEvidence {
        match self {
            Self::Available { evidence, .. } | Self::VerifiedEmpty(evidence) => evidence,
        }
    }

    pub fn records(&self) -> &[T] {
        match self {
            Self::Available { records, .. } => records,
            Self::VerifiedEmpty(_) => &[],
        }
    }

    pub fn is_verified_empty(&self) -> bool {
        matches!(self, Self::VerifiedEmpty(_))
    }
}

impl<T> fmt::Display for GatewayBatch<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let status = if self.is_verified_empty() {
            "verified_empty"
        } else {
            "available"
        };
        let evidence = self.evidence();
        write!(
            formatter,
            "status={status} provider={:?} source={} observed_at={} source_at={} batch_id={} records={}",
            evidence.provider,
            evidence.source,
            evidence.observed_at,
            evidence.source_at.as_deref().unwrap_or("absent"),
            evidence.batch_id,
            self.records().len()
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DailyClose {
    pub date: NaiveDate,
    pub close: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpperLimitRecord {
    pub code: String,
    pub trading_date: NaiveDate,
    pub theme: Option<String>,
    pub streak: Option<u32>,
}

/// Explicit Gateway failure; unavailable is never represented as an empty batch.
#[derive(Debug, Clone, Error)]
#[error(
    "{capability} data gateway failed reason_code={reason_code} provider={provider:?} retryable={retryable}: {message}"
)]
pub struct GatewayError {
    capability: &'static str,
    provider: Option<ProviderId>,
    audit_outcome: &'static str,
    reason_code: &'static str,
    retryable: bool,
    message: String,
}

impl GatewayError {
    pub fn unavailable(
        capability: &'static str,
        provider: Option<ProviderId>,
        retryable: bool,
        message: impl Into<String>,
    ) -> Self {
        Self {
            capability,
            provider,
            audit_outcome: "unavailable",
            reason_code: "no_verified_batch",
            retryable,
            message: message.into(),
        }
    }

    pub(super) fn invalid_evidence(
        capability: &'static str,
        provider: Option<ProviderId>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            capability,
            provider,
            audit_outcome: "partial",
            reason_code: "invalid_evidence",
            retryable: false,
            message: message.into(),
        }
    }

    pub(super) fn invalid_request(capability: &'static str, message: impl Into<String>) -> Self {
        Self {
            capability,
            provider: None,
            audit_outcome: "invalid_request",
            reason_code: "invalid_request",
            retryable: false,
            message: message.into(),
        }
    }

    pub fn retryable(&self) -> bool {
        self.retryable
    }

    pub fn reason_code(&self) -> &'static str {
        self.reason_code
    }

    pub fn audit_outcome(&self) -> &'static str {
        self.audit_outcome
    }

    /// 桥错误路径 wire 序列化需要 (delegate 视图 → convert 重建)。
    pub fn capability(&self) -> &'static str {
        self.capability
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    /// 桥错误路径 wire 序列化需要 (delegate 视图 → convert 重建)。
    pub fn provider(&self) -> Option<ProviderId> {
        self.provider
    }

    pub(super) fn classified(
        capability: &'static str,
        provider: Option<ProviderId>,
        audit_outcome: &'static str,
        reason_code: &'static str,
        retryable: bool,
        message: impl Into<String>,
    ) -> Self {
        Self {
            capability,
            provider,
            audit_outcome,
            reason_code,
            retryable,
            message: message.into(),
        }
    }

    pub(super) fn audit_failure(
        capability: &'static str,
        provider: ProviderId,
        original_reason_code: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self::classified(
            capability,
            Some(provider),
            "unavailable",
            "acquisition_audit_unavailable",
            true,
            format!(
                "BR-159 audit failed after provider outcome {original_reason_code}: {}",
                message.into()
            ),
        )
    }
}

/// Production review-data seam. Blocking provider clients are created, used, and dropped only
/// inside `spawn_blocking` workers.
#[derive(Debug, Clone, Copy, Default)]
pub struct ReviewDataGateway;

impl ReviewDataGateway {
    pub const fn new() -> Self {
        Self
    }

    pub async fn a01_daily_bars(
        &self,
        code: &str,
        limit: u16,
    ) -> Result<GatewayBatch<DailyClose>, GatewayError> {
        let admitted = HistoricalBarsGateway::new()
            .required_daily_bars_async(code, usize::from(limit))
            .await?;
        project_a01_daily_closes(admitted)
    }

    pub async fn r03_upper_limit_pool(
        &self,
        trading_date: NaiveDate,
    ) -> Result<GatewayBatch<UpperLimitRecord>, GatewayError> {
        let request_hash =
            acquisition_request_hash("R-03", &format!("{trading_date}:{WHOLE_LIMIT_POOL_BOUND}"));
        // P4 M3: gRPC 桥 (DATA_GATEWAY_GRPC=1 时替换 transport; audit 留客户端,
        // audit_routed 不校验 provider 一致性 — 与本地 Custom provider 对等)。
        match super::grpc_source::bridge_for("UpperLimitPoolReview") {
            Ok(Some(bridge)) => {
                let result = bridge.upper_limit_pool_review_async(trading_date).await;
                return audit_routed_gateway_result("R-03", &request_hash, result);
            }
            Ok(None) => {}
            Err(error) => {
                return audit_routed_gateway_result("R-03", &request_hash, Err(error));
            }
        }
        // no-feature (monitor 零 magic): library transport 不存在。
        // 无 bridge 时显式失败 (fail-closed), 绝不静默回退。
        #[cfg(not(feature = "magic-gateway"))]
        {
            return Err(GatewayError::classified(
                "R-03",
                Some(ProviderId::Custom),
                "unavailable",
                "provider_transport",
                true,
                "library transport disabled: DATA_GATEWAY_GRPC=1 required",
            ));
        }
        #[cfg(feature = "magic-gateway")]
        {
            let worker_request_hash = request_hash.clone();
            let joined = tokio::task::spawn_blocking(move || {
                let result = route_exact_date_upper_limit_pool("R-03", trading_date)
                    .and_then(|batch| map_r03_upper_limit_batch(batch, trading_date));
                audit_routed_gateway_result("R-03", &worker_request_hash, result)
            })
            .await;
            match joined {
                Ok(result) => result,
                Err(error) => {
                    audit_blocking_join_failure(
                        "R-03",
                        ProviderId::Custom,
                        request_hash,
                        error.to_string(),
                    )
                    .await
                }
            }
        }
    }

    /// Exact-date upper-limit membership batch for the BR-213 market display
    /// projection. The acquisition is durably audited before any consumer may
    /// join display names from realtime quotes.
    pub(crate) fn current_upper_limit_pool(
        &self,
        trading_date: NaiveDate,
    ) -> Result<GatewayBatch<LimitPoolEntry>, GatewayError> {
        const CAPABILITY: &str = "BR-213-UpperLimitPool";
        // no-feature (monitor 零 magic): library transport 不存在。
        // 无 bridge 时显式失败 (fail-closed), 绝不静默回退。
        #[cfg(not(feature = "magic-gateway"))]
        {
            return Err(GatewayError::classified(
                CAPABILITY,
                Some(ProviderId::Custom),
                "unavailable",
                "provider_transport",
                true,
                &format!(
                    "library transport disabled: DATA_GATEWAY_GRPC=1 required (trading_date={trading_date})"
                ),
            ));
        }
        #[cfg(feature = "magic-gateway")]
        {
            let request_hash = acquisition_request_hash(
                CAPABILITY,
                &format!("{trading_date}:{WHOLE_LIMIT_POOL_BOUND}"),
            );
            let result = route_exact_date_upper_limit_pool(CAPABILITY, trading_date);
            audit_routed_gateway_result(CAPABILITY, &request_hash, result)
        }
    }
}

/// Persist the exact BR-213 two-batch join before the plain display projection
/// can escape the analyzer. The hash preimage binds every source identity and
/// timestamp from both admitted batches, the caller-owned trading date, and the
/// projected record count; the BR-159 chain makes the binding tamper-evident.
pub(crate) fn audit_limit_up_projection(
    trading_date: NaiveDate,
    limit_pool: &BatchEvidence,
    names: &[BatchEvidence],
    record_count: usize,
) -> Result<crate::database::data_acquisition_audit::DataAcquisitionAuditReceipt, GatewayError> {
    use crate::database::data_acquisition_audit::DataAcquisitionAuditRecord;
    use crate::database::DatabaseManager;

    const CAPABILITY: &str = "BR-213-UpperLimitProjection";
    let accepted_count = i64::try_from(record_count).map_err(|_| {
        GatewayError::audit_failure(
            CAPABILITY,
            ProviderId::Custom,
            "accepted_count_overflow",
            "projected record count exceeds SQLite INTEGER",
        )
    })?;
    let canonical_evidence =
        canonical_limit_up_projection_evidence(trading_date, limit_pool, names, record_count)?;
    let request_hash = acquisition_request_hash(CAPABILITY, &canonical_evidence);
    let batch_id = format!("BR-213:{request_hash}");
    let source_at = trading_date.format("%Y-%m-%d").to_string();
    let observed_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let record = DataAcquisitionAuditRecord {
        capability: CAPABILITY,
        provider: "Composite",
        // The canonical, versioned component document is itself retained in the
        // immutable BR-159 row, not only represented by its request hash.
        source: &canonical_evidence,
        request_hash: &request_hash,
        source_at: Some(&source_at),
        observed_at: &observed_at,
        batch_id: Some(&batch_id),
        outcome: "available",
        request_count: 1 + i64::try_from(names.len()).unwrap_or(i64::MAX),
        accepted_count,
        rejected_count: 0,
        reason_code: "exact_batch_join_accepted",
        retryable: false,
    };
    let database = DatabaseManager::try_get().ok_or_else(|| {
        GatewayError::audit_failure(
            CAPABILITY,
            ProviderId::Custom,
            "exact_batch_join_accepted",
            "database is not initialized",
        )
    })?;
    database.record_data_acquisition(&record).map_err(|error| {
        GatewayError::audit_failure(
            CAPABILITY,
            ProviderId::Custom,
            "exact_batch_join_accepted",
            error,
        )
    })
}

#[derive(Debug, Serialize)]
struct CanonicalProjectionBatchEvidence<'a> {
    provider: ProviderId,
    source: &'a str,
    source_at: Option<&'a str>,
    observed_at: &'a str,
    batch_id: &'a str,
}

impl<'a> From<&'a BatchEvidence> for CanonicalProjectionBatchEvidence<'a> {
    fn from(evidence: &'a BatchEvidence) -> Self {
        Self {
            provider: evidence.provider,
            source: &evidence.source,
            source_at: evidence.source_at.as_deref(),
            observed_at: &evidence.observed_at,
            batch_id: &evidence.batch_id,
        }
    }
}

#[derive(Debug, Serialize)]
struct CanonicalLimitUpProjectionEvidence<'a> {
    schema: &'static str,
    trading_date: &'a str,
    record_count: usize,
    limit_pool: CanonicalProjectionBatchEvidence<'a>,
    /// BR-221: one entry per acquisition shard, in acquisition order. Shard
    /// evidence is retained separately and never merged into one identity.
    names: Vec<CanonicalProjectionBatchEvidence<'a>>,
}

/// A struct-backed JSON document gives each value an explicit field boundary;
/// JSON escaping prevents provider strings containing delimiters from changing
/// the meaning of the hash preimage. The schema tag makes future encodings
/// distinguishable while struct field order keeps V1 bytes deterministic.
fn canonical_limit_up_projection_evidence(
    trading_date: NaiveDate,
    limit_pool: &BatchEvidence,
    names: &[BatchEvidence],
    record_count: usize,
) -> Result<String, GatewayError> {
    const CAPABILITY: &str = "BR-213-UpperLimitProjection";
    let trading_date = trading_date.format("%Y-%m-%d").to_string();
    serde_json::to_string(&CanonicalLimitUpProjectionEvidence {
        schema: "BR213_LIMIT_UP_PROJECTION_V2",
        trading_date: &trading_date,
        record_count,
        limit_pool: limit_pool.into(),
        names: names.iter().map(Into::into).collect(),
    })
    .map_err(|error| {
        GatewayError::audit_failure(
            CAPABILITY,
            ProviderId::Custom,
            "canonical_evidence_encode_failed",
            error.to_string(),
        )
    })
}

fn project_a01_daily_closes(
    admitted: AdmittedDailyBars,
) -> Result<GatewayBatch<DailyClose>, GatewayError> {
    let (target_code, records, evidence) = admitted.into_bound_parts();
    if records.is_empty() {
        return Err(GatewayError::unavailable(
            "A-01",
            Some(evidence.provider),
            true,
            format!(
                "HistoricalBarsGateway admitted no records for {target_code} \
                 source={} batch_id={}",
                evidence.source, evidence.batch_id
            ),
        ));
    }
    let records = records
        .into_iter()
        .map(|bar| DailyClose {
            date: bar.date,
            close: bar.close,
        })
        .collect();
    Ok(GatewayBatch::Available { records, evidence })
}

#[cfg(feature = "magic-gateway")]
fn build_limit_pool_request(trading_date: NaiveDate) -> Result<LimitPoolRequest, GatewayError> {
    let iso_date = IsoDate::new(trading_date.format("%Y-%m-%d").to_string())
        .map_err(|error| GatewayError::invalid_request("R-03", error.to_string()))?;
    let limit = PositiveU32::new(WHOLE_LIMIT_POOL_BOUND)
        .map_err(|error| GatewayError::invalid_request("R-03", error.to_string()))?;
    LimitPoolRequest::new(LimitPoolKind::Upper, iso_date, limit)
        .map_err(|error| GatewayError::invalid_request("R-03", error.to_string()))
}

#[cfg(feature = "magic-gateway")]
pub(crate) fn route_exact_date_upper_limit_pool(
    capability: &'static str,
    expected_date: NaiveDate,
) -> Result<GatewayBatch<LimitPoolEntry>, GatewayError> {
    let request = build_limit_pool_request(expected_date)?;
    let mut router = LimitPoolRouter::new(
        AcceptancePolicy::new()
            .with_require_complete(true)
            .with_require_source_at(true)
            .with_accept_complete_empty(true),
    );
    router
        .register(SourceFn::new(
            ProviderId::Eastmoney,
            |request: &LimitPoolRequest| {
                EastmoneyClient::new()
                    .map_err(eastmoney_source_error)?
                    .limit_pool(request)
                    .map_err(eastmoney_source_error)
            },
        ))
        .map_err(|error| gateway_router_error(capability, None, error))?;
    router
        .register(SourceFn::new(
            ProviderId::Tonghuashun,
            |request: &LimitPoolRequest| {
                ThsClient::new()
                    .map_err(ths_source_error)?
                    .limit_pool(request)
                    .map_err(ths_source_error)
            },
        ))
        .map_err(|error| gateway_router_error(capability, None, error))?;
    let routed = router
        .route(&request)
        .map_err(|error| gateway_router_error(capability, None, error))?;
    let selected_provider = routed.selected_provider();
    let attempts = routed.attempts().to_vec();
    let batch = routed.into_batch();
    log::info!(
        "[DataGateway][{capability}][BR-159] selected_provider={selected_provider:?} route_attempts={attempts:?}"
    );
    validate_routed_limit_pool_batch(capability, selected_provider, &batch, expected_date)?;
    let evidence = BatchEvidence::from_provenance(selected_provider, batch.provenance())?;
    if batch.records().is_empty() {
        Ok(GatewayBatch::VerifiedEmpty(evidence))
    } else {
        Ok(GatewayBatch::Available {
            records: batch.into_records(),
            evidence,
        })
    }
}

fn validate_routed_limit_pool_batch(
    capability: &'static str,
    provider: ProviderId,
    batch: &crate::magic_compat::DataBatch<LimitPoolEntry>,
    expected_date: NaiveDate,
) -> Result<(), GatewayError> {
    if !matches!(provider, ProviderId::Eastmoney | ProviderId::Tonghuashun) {
        return Err(GatewayError::invalid_evidence(
            capability,
            Some(provider),
            "limit-pool router selected an unregistered provider",
        ));
    }
    let expected_text = expected_date.format("%Y-%m-%d").to_string();
    if batch.provenance().source_at() != Some(expected_text.as_str()) {
        return Err(GatewayError::invalid_evidence(
            capability,
            Some(provider),
            format!(
                "limit-pool batch source_at {:?} differs from requested {expected_date}",
                batch.provenance().source_at()
            ),
        ));
    }
    let batch_id = batch.provenance().batch_id().ok_or_else(|| {
        GatewayError::invalid_evidence(capability, Some(provider), "limit-pool batch ID is absent")
    })?;
    let mut identities = HashSet::with_capacity(batch.records().len());
    for record in batch.records() {
        if record.kind != LimitPoolKind::Upper
            || record.evidence.provider() != provider
            || record.evidence.batch_id() != batch_id
            || record.evidence.source_at() != batch.provenance().source_at()
            || record.evidence.observed_at() != batch.provenance().fetched_at()
        {
            return Err(GatewayError::invalid_evidence(
                capability,
                Some(provider),
                "limit-pool record evidence differs from selected batch provenance",
            ));
        }
        let trading_date = NaiveDate::parse_from_str(record.trading_date.as_str(), "%Y-%m-%d")
            .map_err(|error| {
                GatewayError::invalid_evidence(
                    capability,
                    Some(provider),
                    format!("invalid limit-pool trading date: {error}"),
                )
            })?;
        if trading_date != expected_date {
            return Err(GatewayError::invalid_evidence(
                capability,
                Some(provider),
                format!(
                    "limit-pool trading date {trading_date} differs from requested {expected_date}"
                ),
            ));
        }
        if !identities.insert(record.instrument.code().to_string()) {
            return Err(GatewayError::invalid_evidence(
                capability,
                Some(provider),
                format!(
                    "limit-pool batch contains duplicate security {}",
                    record.instrument.code()
                ),
            ));
        }
    }
    Ok(())
}

fn map_r03_upper_limit_batch(
    batch: GatewayBatch<LimitPoolEntry>,
    expected_date: NaiveDate,
) -> Result<GatewayBatch<UpperLimitRecord>, GatewayError> {
    let evidence = batch.evidence().clone();
    if batch.is_verified_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(evidence));
    }
    let provider = evidence.provider;
    let records = batch
        .records()
        .iter()
        .map(|record| {
            if record.kind != LimitPoolKind::Upper {
                return Err(GatewayError::invalid_evidence(
                    "R-03",
                    Some(provider),
                    "limit-pool record is not an upper-limit entry",
                ));
            }
            let trading_date =
                NaiveDate::parse_from_str(record.trading_date.as_str(), "%Y-%m-%d").map_err(
                    |error| {
                        GatewayError::invalid_evidence(
                            "R-03",
                            Some(provider),
                            format!("invalid limit-pool trading date: {error}"),
                        )
                    },
                )?;
            if trading_date != expected_date {
                return Err(GatewayError::invalid_evidence(
                    "R-03",
                    Some(provider),
                    format!(
                        "limit-pool trading date {trading_date} differs from requested {expected_date}"
                    ),
                ));
            }
            let theme = match provider {
                ProviderId::Eastmoney => record.industry.as_ref(),
                ProviderId::Tonghuashun => record.reason.as_ref(),
                _ => {
                    return Err(GatewayError::invalid_evidence(
                        "R-03",
                        Some(provider),
                        "unsupported selected provider for R-03 theme projection",
                    ))
                }
            };
            Ok(UpperLimitRecord {
                code: record.instrument.code().to_string(),
                trading_date,
                theme: theme.map(|value| value.as_str().to_string()),
                streak: record.streak.map(PositiveU32::get),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(GatewayBatch::Available { records, evidence })
}

pub(super) fn acquisition_request_hash(capability: &str, canonical_request: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"BR159_DATA_GATEWAY_REQUEST_V1\0");
    hasher.update(capability.as_bytes());
    hasher.update(b"\0");
    hasher.update(canonical_request.as_bytes());
    hex::encode(hasher.finalize())
}

pub(super) fn audit_gateway_result<T>(
    capability: &'static str,
    provider: ProviderId,
    request_hash: &str,
    result: Result<GatewayBatch<T>, GatewayError>,
) -> Result<GatewayBatch<T>, GatewayError> {
    use crate::database::data_acquisition_audit::DataAcquisitionAuditRecord;
    use crate::database::DatabaseManager;

    let result = result.and_then(|batch| {
        if batch.evidence().provider != provider {
            return Err(GatewayError::invalid_evidence(
                capability,
                Some(provider),
                "Gateway batch provider differs from the admitted provider",
            ));
        }
        Ok(batch)
    });

    let observed_fallback = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let (
        source,
        source_at,
        observed_at,
        batch_id,
        outcome,
        accepted_count,
        rejected_count,
        reason_code,
        retryable,
    ) = match &result {
        Ok(batch) => {
            let evidence = batch.evidence();
            let accepted_count = i64::try_from(batch.records().len()).map_err(|_| {
                GatewayError::audit_failure(
                    capability,
                    provider,
                    "accepted_count_overflow",
                    "accepted record count exceeds SQLite INTEGER",
                )
            })?;
            (
                evidence.source.as_str(),
                evidence.source_at.as_deref(),
                evidence.observed_at.as_str(),
                Some(evidence.batch_id.as_str()),
                if batch.is_verified_empty() {
                    "verified_empty"
                } else {
                    "available"
                },
                accepted_count,
                0,
                if batch.is_verified_empty() {
                    "verified_empty"
                } else {
                    "accepted"
                },
                false,
            )
        }
        Err(error) => (
            "review-data-gateway",
            None,
            observed_fallback.as_str(),
            None,
            error.audit_outcome,
            0,
            1,
            error.reason_code,
            error.retryable,
        ),
    };
    let provider_label = format!("{provider:?}");
    let record = DataAcquisitionAuditRecord {
        capability,
        provider: &provider_label,
        source,
        request_hash,
        source_at,
        observed_at,
        batch_id,
        outcome,
        request_count: 1,
        accepted_count,
        rejected_count,
        reason_code,
        retryable,
    };
    let original_reason_code = reason_code;
    let database = DatabaseManager::try_get().ok_or_else(|| {
        GatewayError::audit_failure(
            capability,
            provider,
            original_reason_code,
            "database is not initialized",
        )
    })?;
    let receipt = database.record_data_acquisition(&record).map_err(|error| {
        GatewayError::audit_failure(capability, provider, original_reason_code, error)
    })?;

    log::info!(
        "[DataGateway][{capability}][BR-159] outcome={outcome} provider={provider:?} \
         source={source} observed_at={observed_at} source_at={} batch_id={} \
         requested=1 accepted={accepted_count} rejected={rejected_count} \
         reason_code={reason_code} retryable={retryable} audit_id={} record_hash={}",
        source_at.unwrap_or("absent"),
        batch_id.unwrap_or("absent"),
        receipt.audit_id,
        receipt.record_hash
    );
    if receipt.provider_state_changed() {
        log::warn!(
            "[DataGateway][{capability}][BR-159] provider state changed \
             provider={provider:?} previous={} current={}",
            receipt.previous_outcome.as_deref().unwrap_or("absent"),
            receipt.current_outcome
        );
    }
    result
}

pub(super) fn audit_routed_gateway_result<T>(
    capability: &'static str,
    request_hash: &str,
    result: Result<GatewayBatch<T>, GatewayError>,
) -> Result<GatewayBatch<T>, GatewayError> {
    let provider = match &result {
        Ok(batch) => batch.evidence().provider,
        Err(error) => error.provider.unwrap_or(ProviderId::Custom),
    };
    audit_gateway_result(capability, provider, request_hash, result)
}

pub(super) async fn audit_blocking_join_failure<T: Send + 'static>(
    capability: &'static str,
    provider: ProviderId,
    request_hash: String,
    message: String,
) -> Result<GatewayBatch<T>, GatewayError> {
    let failure = GatewayError::classified(
        capability,
        Some(provider),
        "unavailable",
        "blocking_task_failed",
        true,
        format!("blocking provider task failed: {message}"),
    );
    tokio::task::spawn_blocking(move || {
        audit_gateway_result(capability, provider, &request_hash, Err(failure))
    })
    .await
    .unwrap_or_else(|error| {
        Err(GatewayError::audit_failure(
            capability,
            provider,
            "blocking_task_failed",
            format!("blocking audit task failed: {error}"),
        ))
    })
}

#[cfg(feature = "magic-gateway")]
fn eastmoney_source_error(error: EastmoneyError) -> SourceError {
    let message = error.to_string();
    match error {
        EastmoneyError::InvalidRequest(_) => {
            SourceError::stop(FailureKind::InvalidRequest, message)
        }
        EastmoneyError::Transport(_) => SourceError::try_next(FailureKind::Transport, message),
        EastmoneyError::ResponseTooLarge { .. } => {
            SourceError::try_next(FailureKind::Quality, message)
        }
        EastmoneyError::Unsupported(_) => SourceError::try_next(FailureKind::Unsupported, message),
        EastmoneyError::Decode(_) | EastmoneyError::Protocol(_) => {
            SourceError::try_next(FailureKind::Protocol, message)
        }
        EastmoneyError::VerifiedEmpty(_) => SourceError::try_next(FailureKind::NoData, message),
        EastmoneyError::Core(_) => SourceError::try_next(FailureKind::Evidence, message),
    }
}

#[cfg(feature = "magic-gateway")]
fn ths_source_error(error: ThsError) -> SourceError {
    let message = error.to_string();
    match error {
        ThsError::InvalidRequest(_) => SourceError::stop(FailureKind::InvalidRequest, message),
        ThsError::Unsupported(_) => SourceError::try_next(FailureKind::Unsupported, message),
        ThsError::Authentication(_) | ThsError::HttpStatus(_) => {
            SourceError::try_next(FailureKind::Provider, message)
        }
        ThsError::RateLimited => SourceError::try_next(FailureKind::RateLimited, message),
        ThsError::Transport(_) => SourceError::try_next(FailureKind::Transport, message),
        ThsError::Decode(_) | ThsError::Schema(_) => {
            SourceError::try_next(FailureKind::Protocol, message)
        }
        ThsError::Incomplete(_) => SourceError::try_next(FailureKind::Quality, message),
        ThsError::VerifiedEmpty(_) => SourceError::try_next(FailureKind::NoData, message),
        ThsError::ProbeAdmission(_) | ThsError::Core(_) => {
            SourceError::try_next(FailureKind::Evidence, message)
        }
    }
}

#[cfg(feature = "magic-gateway")]
fn gateway_router_error(
    capability: &'static str,
    provider: Option<ProviderId>,
    error: RouterError,
) -> GatewayError {
    let terminal_kind = error
        .attempts()
        .iter()
        .rev()
        .find_map(|attempt| match attempt.status() {
            AttemptStatus::Failed { kind, .. } | AttemptStatus::Rejected { kind, .. } => {
                Some(*kind)
            }
            AttemptStatus::Selected => None,
        });
    let (audit_outcome, reason_code, retryable) = match terminal_kind {
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
        ) => ("unavailable", "router_unavailable", true),
        Some(FailureKind::Protocol | FailureKind::Quality | FailureKind::Evidence) => {
            ("partial", "router_batch_rejected", false)
        }
    };
    GatewayError::classified(
        capability,
        provider,
        audit_outcome,
        reason_code,
        retryable,
        error.to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_provider::{AdjustType, KlineData};
    use crate::database::DatabaseManager;
    use crate::magic_compat::{
        AssetClass, DataBatch, Exchange, InstrumentId, IsoDate, NonEmptyText, Price, Provenance,
        Ratio, RatioUnit, SourceEvidence,
    };
    use diesel::prelude::*;
    use diesel::sql_types::{BigInt, Nullable, Text};
    use serial_test::serial;

    #[derive(Debug, QueryableByName)]
    struct ProjectionAuditRow {
        #[diesel(sql_type = Text)]
        source: String,
        #[diesel(sql_type = Text)]
        request_hash: String,
        #[diesel(sql_type = Nullable<Text>)]
        batch_id: Option<String>,
    }

    #[test]
    fn br158_batch_evidence_preserves_provider_and_batch_times() {
        let provenance = Provenance::new("TEST_CODE_tdx-smart", "2099-01-02T10:00:00+08:00")
            .expect("provenance")
            .with_source_at("2099-01-02")
            .expect("source at")
            .with_batch_id("TEST_CODE_batch_1")
            .expect("batch id");

        let evidence =
            BatchEvidence::from_provenance(crate::magic_compat::ProviderId::Tdx, &provenance)
                .expect("valid evidence");

        assert_eq!(evidence.source, "TEST_CODE_tdx-smart");
        assert_eq!(evidence.source_at.as_deref(), Some("2099-01-02"));
        assert_eq!(evidence.observed_at, "2099-01-02T10:00:00+08:00");
        assert_eq!(evidence.batch_id, "TEST_CODE_batch_1");
    }

    #[test]
    fn br158_verified_empty_is_not_unavailable() {
        let evidence = BatchEvidence {
            provider: crate::magic_compat::ProviderId::Tdx,
            source: "TEST_CODE_tdx-smart".to_string(),
            source_at: Some("2099-01-02".to_string()),
            observed_at: "2099-01-02T10:00:00+08:00".to_string(),
            batch_id: "TEST_CODE_batch_2".to_string(),
        };

        let batch: GatewayBatch<DailyClose> = GatewayBatch::VerifiedEmpty(evidence.clone());
        assert_eq!(batch.evidence(), &evidence);
        assert!(batch.records().is_empty());

        let unavailable = GatewayError::unavailable(
            "A-01",
            Some(crate::magic_compat::ProviderId::Tdx),
            true,
            "TEST_CODE transport unavailable",
        );
        assert!(unavailable.retryable());
        assert_ne!(batch.to_string(), unavailable.to_string());
    }

    fn evidence(provider: ProviderId, batch_id: &str) -> BatchEvidence {
        BatchEvidence {
            provider,
            source: "TEST_CODE_source".to_string(),
            source_at: Some("2099-01-02".to_string()),
            observed_at: "2099-01-02T10:00:00+08:00".to_string(),
            batch_id: batch_id.to_string(),
        }
    }

    fn kline(date: NaiveDate, close: f64) -> KlineData {
        KlineData {
            date,
            open: close,
            high: close,
            low: close,
            close,
            volume: 100.0,
            amount: 1_000.0,
            pct_chg: 0.0,
            intraday_price: None,
            settled: true,
            pe_ratio: None,
            pb_ratio: None,
            turnover_rate: None,
            market_cap: None,
            circulating_cap: None,
            eps: None,
            roe: None,
            revenue_yoy: None,
            net_profit_yoy: None,
            gross_margin: None,
            net_margin: None,
            sharpe_ratio: None,
            financials_history: None,
            valuation_history: None,
            consensus: None,
            industry: None,
            is_limit_up: false,
            is_limit_down: false,
            is_suspended: false,
            adjust: AdjustType::None,
        }
    }

    fn limit_pool_entry(provider: ProviderId, kind: LimitPoolKind, date: &str) -> LimitPoolEntry {
        LimitPoolEntry {
            kind,
            instrument: InstrumentId::new(
                Exchange::Shanghai,
                "TEST_CODE_600001",
                AssetClass::Equity,
            )
            .unwrap(),
            trading_date: IsoDate::new(date).unwrap(),
            price: Price::new(10.0).unwrap(),
            change: Ratio::new(10.0, RatioUnit::Percent).unwrap(),
            volume: None,
            turnover: None,
            sealed_amount: None,
            first_seal_at: None,
            last_seal_at: None,
            break_count: None,
            streak: Some(PositiveU32::new(2).unwrap()),
            industry: (provider == ProviderId::Eastmoney)
                .then(|| NonEmptyText::new("TEST_CODE industry").unwrap()),
            board_name: None,
            seal_state: None,
            reseal_count: None,
            reason: (provider == ProviderId::Tonghuashun)
                .then(|| NonEmptyText::new("TEST_CODE reason").unwrap()),
            evidence: SourceEvidence::new(
                provider,
                "2099-01-02T10:00:00+08:00",
                "TEST_CODE_limit_pool",
            )
            .unwrap()
            .with_source_at(date)
            .unwrap(),
        }
    }

    #[test]
    fn br158_batch_display_error_and_missing_provenance_contracts_are_covered() {
        let available = GatewayBatch::Available {
            records: vec![DailyClose {
                date: NaiveDate::from_ymd_opt(2099, 1, 2).unwrap(),
                close: 10.0,
            }],
            evidence: evidence(ProviderId::Tdx, "TEST_CODE_available"),
        };
        assert!(!available.is_verified_empty());
        assert_eq!(available.records().len(), 1);
        assert!(available.to_string().contains("status=available"));

        let generated_batch =
            Provenance::new("TEST_CODE_source", "2099-01-02T10:00:00+08:00").unwrap();
        assert!(
            BatchEvidence::from_provenance(ProviderId::Tdx, &generated_batch)
                .unwrap()
                .batch_id
                .starts_with("TEST_CODE_source:")
        );

        let invalid_request = GatewayError::invalid_request("TEST_CODE", "bad");
        let invalid_evidence =
            GatewayError::invalid_evidence("TEST_CODE", Some(ProviderId::Tdx), "bad");
        let classified = GatewayError::classified(
            "TEST_CODE",
            Some(ProviderId::Tdx),
            "unsupported",
            "TEST_CODE_unsupported",
            false,
            "bad",
        );
        let audit =
            GatewayError::audit_failure("TEST_CODE", ProviderId::Tdx, "TEST_CODE_original", "bad");
        assert_eq!(invalid_request.audit_outcome(), "invalid_request");
        assert_eq!(invalid_evidence.reason_code(), "invalid_evidence");
        assert_eq!(classified.audit_outcome(), "unsupported");
        assert_eq!(audit.reason_code(), "acquisition_audit_unavailable");
        assert!(audit.retryable());
    }

    #[test]
    fn br158_a01_projection_preserves_gateway_order_and_evidence() {
        let evidence = evidence(ProviderId::Tencent, "TEST_CODE_daily_batch");
        let newest = NaiveDate::from_ymd_opt(2099, 1, 3).unwrap();
        let previous = NaiveDate::from_ymd_opt(2099, 1, 2).unwrap();
        let admitted = AdmittedDailyBars::from_test_fixture(
            "TEST_CODE_600396",
            vec![kline(newest, 11.0), kline(previous, 10.0)],
            evidence.clone(),
        )
        .expect("TEST_CODE admitted daily batch");

        let projected =
            project_a01_daily_closes(admitted).expect("A-01 evidence-preserving projection");

        assert_eq!(projected.evidence(), &evidence);
        assert_eq!(
            projected.records(),
            &[
                DailyClose {
                    date: newest,
                    close: 11.0,
                },
                DailyClose {
                    date: previous,
                    close: 10.0,
                },
            ]
        );
    }

    #[test]
    #[cfg(feature = "magic-gateway")]
    fn br159_limit_pool_requests_are_bounded() {
        let date = NaiveDate::from_ymd_opt(2099, 1, 2).unwrap();
        let request = build_limit_pool_request(date).expect("bounded whole-market request");
        assert_eq!(request.kind(), LimitPoolKind::Upper);
        assert_eq!(request.limit().get(), WHOLE_LIMIT_POOL_BOUND);
        assert_eq!(
            acquisition_request_hash("TEST_CODE", "request"),
            acquisition_request_hash("TEST_CODE", "request")
        );
        assert_ne!(
            acquisition_request_hash("TEST_CODE", "request"),
            acquisition_request_hash("TEST_CODE", "other")
        );
    }

    #[test]
    fn br159_limit_pool_mapping_is_provider_specific_and_rejects_wrong_kind_or_date() {
        let date = NaiveDate::from_ymd_opt(2099, 1, 2).unwrap();
        for (provider, expected_theme) in [
            (ProviderId::Eastmoney, "TEST_CODE industry"),
            (ProviderId::Tonghuashun, "TEST_CODE reason"),
        ] {
            let mapped = map_r03_upper_limit_batch(
                GatewayBatch::Available {
                    records: vec![limit_pool_entry(
                        provider,
                        LimitPoolKind::Upper,
                        "2099-01-02",
                    )],
                    evidence: evidence(provider, "TEST_CODE_limit_pool"),
                },
                date,
            )
            .unwrap();
            assert_eq!(mapped.records()[0].code, "TEST_CODE_600001");
            assert_eq!(mapped.records()[0].theme.as_deref(), Some(expected_theme));
            assert_eq!(mapped.records()[0].streak, Some(2));
            assert_eq!(mapped.evidence().provider, provider);
        }

        for entry in [
            limit_pool_entry(ProviderId::Eastmoney, LimitPoolKind::Broken, "2099-01-02"),
            limit_pool_entry(ProviderId::Eastmoney, LimitPoolKind::Upper, "2099-01-03"),
        ] {
            assert!(map_r03_upper_limit_batch(
                GatewayBatch::Available {
                    records: vec![entry],
                    evidence: evidence(ProviderId::Eastmoney, "TEST_CODE_limit_pool"),
                },
                date,
            )
            .is_err());
        }
    }

    #[test]
    fn br213_limit_pool_admission_rejects_conflicting_record_observed_at() {
        let date = NaiveDate::from_ymd_opt(2099, 1, 2).unwrap();
        let mut entry = limit_pool_entry(ProviderId::Eastmoney, LimitPoolKind::Upper, "2099-01-02");
        entry.evidence = SourceEvidence::new(
            ProviderId::Eastmoney,
            "2099-01-02T10:00:01+08:00",
            "TEST_CODE_limit_pool",
        )
        .unwrap()
        .with_source_at("2099-01-02")
        .unwrap();
        let provenance = Provenance::new("TEST_CODE_source", "2099-01-02T10:00:00+08:00")
            .unwrap()
            .with_source_at("2099-01-02")
            .unwrap()
            .with_batch_id("TEST_CODE_limit_pool")
            .unwrap();
        let batch = DataBatch::strict(vec![entry], provenance);

        let error = validate_routed_limit_pool_batch(
            "TEST_CODE_BR213",
            ProviderId::Eastmoney,
            &batch,
            date,
        )
        .expect_err("record observed_at must equal batch fetched_at");

        assert_eq!(error.reason_code(), "invalid_evidence");
    }

    #[test]
    fn br213_projection_evidence_is_versioned_canonical_and_delimiter_safe() {
        let date = NaiveDate::from_ymd_opt(2099, 1, 2).unwrap();
        let mut limit_pool = evidence(ProviderId::Eastmoney, "TEST_CODE_batch|quote_source=x");
        limit_pool.source = "TEST_CODE_source|quote_batch=y".to_string();
        let mut quotes = evidence(ProviderId::Tencent, "TEST_CODE_quote_batch");
        quotes.source = "TEST_CODE_quote_source".to_string();

        let canonical = canonical_limit_up_projection_evidence(
            date,
            &limit_pool,
            std::slice::from_ref(&quotes),
            1,
        )
        .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&canonical).unwrap();
        assert_eq!(parsed["schema"], "BR213_LIMIT_UP_PROJECTION_V2");
        assert_eq!(
            parsed["limit_pool"]["batch_id"],
            "TEST_CODE_batch|quote_source=x"
        );
        assert_eq!(
            parsed["limit_pool"]["source"],
            "TEST_CODE_source|quote_batch=y"
        );

        let mut different_limit_pool = limit_pool.clone();
        different_limit_pool.source = "TEST_CODE_source".to_string();
        different_limit_pool.batch_id = "TEST_CODE_batch|quote_source=x|quote_batch=y".to_string();
        let different = canonical_limit_up_projection_evidence(
            date,
            &different_limit_pool,
            std::slice::from_ref(&quotes),
            1,
        )
        .unwrap();
        assert_ne!(canonical, different);
        assert_ne!(
            acquisition_request_hash("BR-213-UpperLimitProjection", &canonical),
            acquisition_request_hash("BR-213-UpperLimitProjection", &different)
        );
    }

    #[test]
    #[serial]
    fn br213_projection_audit_persists_canonical_component_evidence() {
        DatabaseManager::init(None).expect("TEST_CODE audit database init");
        let date = NaiveDate::from_ymd_opt(2099, 1, 2).unwrap();
        let limit_pool = evidence(ProviderId::Eastmoney, "TEST_CODE_limit_batch");
        let quotes = evidence(ProviderId::Tencent, "TEST_CODE_quote_batch");
        let canonical = canonical_limit_up_projection_evidence(
            date,
            &limit_pool,
            std::slice::from_ref(&quotes),
            2,
        )
        .unwrap();
        let expected_hash = acquisition_request_hash("BR-213-UpperLimitProjection", &canonical);

        let receipt =
            audit_limit_up_projection(date, &limit_pool, std::slice::from_ref(&quotes), 2)
                .expect("TEST_CODE canonical composition audit");
        let mut connection = DatabaseManager::get().get_conn().unwrap();
        let row = diesel::sql_query(
            "SELECT source, request_hash, batch_id FROM data_acquisition_audit WHERE id = ?",
        )
        .bind::<BigInt, _>(receipt.audit_id)
        .get_result::<ProjectionAuditRow>(&mut *connection)
        .unwrap();

        assert_eq!(row.source, canonical);
        assert_eq!(row.request_hash, expected_hash);
        let expected_batch_id = format!("BR-213:{expected_hash}");
        assert_eq!(row.batch_id.as_deref(), Some(expected_batch_id.as_str()));
    }

    #[test]
    #[cfg(feature = "magic-gateway")]
    fn br159_limit_pool_provider_error_mappers_preserve_retry_policy() {
        let eastmoney_cases = [
            eastmoney_source_error(EastmoneyError::InvalidRequest("TEST_CODE".into())),
            eastmoney_source_error(EastmoneyError::Transport("TEST_CODE".into())),
            eastmoney_source_error(EastmoneyError::ResponseTooLarge { limit: 1 }),
            eastmoney_source_error(EastmoneyError::Unsupported("TEST_CODE".into())),
            eastmoney_source_error(EastmoneyError::Decode("TEST_CODE".into())),
            eastmoney_source_error(EastmoneyError::Protocol("TEST_CODE".into())),
        ];
        assert_eq!(eastmoney_cases[0].kind(), FailureKind::InvalidRequest);
        assert_eq!(
            eastmoney_cases[0].action(),
            magic_market_router::FailureAction::Stop
        );
        assert!(eastmoney_cases[1..]
            .iter()
            .all(|error| error.action() == magic_market_router::FailureAction::TryNext));
        assert_eq!(eastmoney_cases[1].kind(), FailureKind::Transport);
        assert_eq!(eastmoney_cases[2].kind(), FailureKind::Quality);
        assert_eq!(eastmoney_cases[3].kind(), FailureKind::Unsupported);
        assert!(eastmoney_cases[4..]
            .iter()
            .all(|error| error.kind() == FailureKind::Protocol));

        let ths_cases = [
            ths_source_error(ThsError::InvalidRequest("TEST_CODE".into())),
            ths_source_error(ThsError::Transport("TEST_CODE".into())),
            ths_source_error(ThsError::Schema("TEST_CODE".into())),
            ths_source_error(ThsError::Incomplete("TEST_CODE".into())),
        ];
        assert_eq!(
            ths_cases[0].action(),
            magic_market_router::FailureAction::Stop
        );
        assert_eq!(ths_cases[1].kind(), FailureKind::Transport);
        assert_eq!(ths_cases[2].kind(), FailureKind::Protocol);
        assert_eq!(ths_cases[3].kind(), FailureKind::Quality);
    }
}
