//! Registered business rules: BR-158, BR-159, BR-213, BR-238.
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
const BENCHMARK_GRPC_TRANSPORT_CAPABILITY: &str = "BenchmarkBarsGrpcTransport";
const BENCHMARK_GRPC_CONSUMER_ADMISSION_CAPABILITY: &str = "BenchmarkBarsGrpcConsumerAdmission";

#[derive(Debug, Clone, Copy, Serialize)]
enum LimitPoolAcquisitionProfile {
    #[serde(rename = "LocalBridgeV1")]
    LocalBridgeV1,
    #[serde(rename = "InProcessRouterV1")]
    InProcessRouterV1,
}

#[derive(Debug, Clone, Copy, Serialize)]
enum LimitPoolAcquisitionOperation {
    #[serde(rename = "LimitPools")]
    LimitPools,
}

#[derive(Debug, Serialize)]
struct CanonicalLimitPoolAcquisitionRequest {
    schema: &'static str,
    profile: LimitPoolAcquisitionProfile,
    operation: LimitPoolAcquisitionOperation,
    request: CanonicalLimitPoolRequest,
}

#[derive(Debug, Serialize)]
struct CanonicalLimitPoolRequest {
    kind: LimitPoolKind,
    trading_date: String,
    limit: u32,
}

/// BR-159 request identity for every BR-213 whole-pool acquisition. A typed
/// document keeps the route/profile and the exact request fields in one stable
/// canonical preimage; callers cannot fall back to delimiter-based summaries.
fn limit_pool_acquisition_request_hash(
    capability: &'static str,
    profile: LimitPoolAcquisitionProfile,
    trading_date: NaiveDate,
) -> Result<String, GatewayError> {
    let canonical = serde_json::to_vec(&CanonicalLimitPoolAcquisitionRequest {
        schema: "BR159_LIMIT_POOLS_ACQUISITION_V1",
        profile,
        operation: LimitPoolAcquisitionOperation::LimitPools,
        request: CanonicalLimitPoolRequest {
            kind: LimitPoolKind::Upper,
            trading_date: trading_date.format("%Y-%m-%d").to_string(),
            limit: WHOLE_LIMIT_POOL_BOUND,
        },
    })
    .map_err(|error| {
        GatewayError::invalid_request(
            capability,
            format!("canonical LimitPools request encoding failed: {error}"),
        )
    })?;
    Ok(acquisition_request_hash(capability, canonical))
}

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

#[derive(Debug, Clone)]
pub struct AuditedBenchmarkBatch {
    pub batch: GatewayBatch<crate::data_gateway::BenchmarkBar>,
    pub receipt: crate::database::data_acquisition_audit::DataAcquisitionAuditReceipt,
    pub request_hash: String,
}

#[derive(Debug)]
pub(crate) struct BenchmarkLibraryFailure {
    error: GatewayError,
    audit_state: super::grpc_source::BenchmarkServerAuditState,
}

impl BenchmarkLibraryFailure {
    pub(crate) fn into_parts(
        self,
    ) -> (GatewayError, super::grpc_source::BenchmarkServerAuditState) {
        (self.error, self.audit_state)
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

fn finish_benchmark_bridge_attempt(
    request: &crate::data_gateway::BenchmarkRequest,
    result: Result<AuditedBenchmarkBatch, super::grpc_source::BenchmarkGrpcFailure>,
) -> Result<AuditedBenchmarkBatch, GatewayError> {
    let failure = match result {
        Ok(audited) => return Ok(audited),
        Err(failure) => failure,
    };
    match failure.ownership() {
        super::grpc_source::BenchmarkGrpcOwnership::ServerHandled => {
            Err(failure.into_error())
        }
        super::grpc_source::BenchmarkGrpcOwnership::ServerAuditAppendFailed => {
            audit_benchmark_transport_failure(request, failure.into_error())
        }
        super::grpc_source::BenchmarkGrpcOwnership::ClientBeforeSend => {
            audit_benchmark_transport_failure(request, failure.into_error())
        }
        super::grpc_source::BenchmarkGrpcOwnership::OutcomeUnknown => {
            audit_benchmark_transport_failure(
                request,
                GatewayError::classified(
                    "GrpcBridge",
                    None,
                    "unavailable",
                    "transport_outcome_unknown",
                    true,
                    "BenchmarkBars request may have reached the server; provider outcome is unknown",
                ),
            )
        }
        super::grpc_source::BenchmarkGrpcOwnership::ConsumerAdmissionRejected => {
            debug_assert!(failure.has_verified_receipt());
            audit_benchmark_consumer_admission_failure(request, failure.into_error())
        }
    }
}

fn finish_benchmark_bridge_attempt_in(
    database: &crate::database::DatabaseManager,
    request: &crate::data_gateway::BenchmarkRequest,
    result: Result<AuditedBenchmarkBatch, super::grpc_source::BenchmarkGrpcFailure>,
) -> Result<AuditedBenchmarkBatch, GatewayError> {
    let failure = match result {
        Ok(audited) => return audit_benchmark_bridge_success_in(database, request, audited),
        Err(failure) => failure,
    };
    match failure.ownership() {
        super::grpc_source::BenchmarkGrpcOwnership::ServerHandled => {
            audit_benchmark_provider_failure_in(database, request, failure.into_error())
        }
        super::grpc_source::BenchmarkGrpcOwnership::ServerAuditAppendFailed
        | super::grpc_source::BenchmarkGrpcOwnership::ClientBeforeSend => {
            audit_benchmark_transport_failure_in(database, request, failure.into_error())
        }
        super::grpc_source::BenchmarkGrpcOwnership::OutcomeUnknown => {
            audit_benchmark_transport_failure_in(
                database,
                request,
                GatewayError::classified(
                    "GrpcBridge",
                    None,
                    "unavailable",
                    "transport_outcome_unknown",
                    true,
                    "BenchmarkBars request may have reached the server; provider outcome is unknown",
                ),
            )
        }
        super::grpc_source::BenchmarkGrpcOwnership::ConsumerAdmissionRejected => {
            debug_assert!(failure.has_verified_receipt());
            audit_benchmark_consumer_admission_failure_in(
                database,
                request,
                failure.into_error(),
            )
        }
    }
}

fn audit_benchmark_bridge_success_in(
    database: &crate::database::DatabaseManager,
    request: &crate::data_gateway::BenchmarkRequest,
    remote: AuditedBenchmarkBatch,
) -> Result<AuditedBenchmarkBatch, GatewayError> {
    let request_hash = super::benchmark::canonical_base_request_hash(request);
    if remote.request_hash != request_hash {
        return audit_benchmark_consumer_admission_failure_in(
            database,
            request,
            GatewayError::invalid_evidence(
                "BenchmarkBars",
                Some(ProviderId::Tdx),
                "gRPC benchmark request hash differs from the caller request",
            ),
        );
    }
    audit_gateway_result_with_receipt_state_in(
        database,
        "BenchmarkBars",
        ProviderId::Tdx,
        &request_hash,
        Ok(remote.batch),
    )
    .map(|(batch, receipt)| AuditedBenchmarkBatch {
        batch,
        receipt,
        request_hash,
    })
    .map_err(GatewayAuditFailure::into_error)
}

fn audit_benchmark_provider_failure_in(
    database: &crate::database::DatabaseManager,
    request: &crate::data_gateway::BenchmarkRequest,
    error: GatewayError,
) -> Result<AuditedBenchmarkBatch, GatewayError> {
    let request_hash = super::benchmark::canonical_base_request_hash(request);
    match audit_gateway_result_with_receipt_state_in::<crate::data_gateway::BenchmarkBar>(
        database,
        "BenchmarkBars",
        ProviderId::Tdx,
        &request_hash,
        Err(error),
    ) {
        Err(error) => Err(error.into_error()),
        Ok(_) => unreachable!("an audited provider failure cannot become a successful batch"),
    }
}

fn audit_benchmark_transport_failure(
    request: &crate::data_gateway::BenchmarkRequest,
    error: GatewayError,
) -> Result<AuditedBenchmarkBatch, GatewayError> {
    let transport_request_hash = benchmark_transport_request_hash(request);
    match audit_gateway_result_with_receipt::<crate::data_gateway::BenchmarkBar>(
        BENCHMARK_GRPC_TRANSPORT_CAPABILITY,
        ProviderId::Custom,
        &transport_request_hash,
        Err(error),
    ) {
        Err(error) => Err(error),
        Ok(_) => unreachable!("an audited transport failure cannot become a successful batch"),
    }
}

fn audit_benchmark_transport_failure_in(
    database: &crate::database::DatabaseManager,
    request: &crate::data_gateway::BenchmarkRequest,
    error: GatewayError,
) -> Result<AuditedBenchmarkBatch, GatewayError> {
    let transport_request_hash = benchmark_transport_request_hash(request);
    match audit_gateway_result_with_receipt_state_in::<crate::data_gateway::BenchmarkBar>(
        database,
        BENCHMARK_GRPC_TRANSPORT_CAPABILITY,
        ProviderId::Custom,
        &transport_request_hash,
        Err(error),
    ) {
        Err(error) => Err(error.into_error()),
        Ok(_) => unreachable!("an audited transport failure cannot become a successful batch"),
    }
}

fn benchmark_transport_request_hash(request: &crate::data_gateway::BenchmarkRequest) -> String {
    let provider_request_hash = super::benchmark::canonical_base_request_hash(request);
    acquisition_request_hash(
        BENCHMARK_GRPC_TRANSPORT_CAPABILITY,
        provider_request_hash.as_bytes(),
    )
}

fn audit_benchmark_consumer_admission_failure(
    request: &crate::data_gateway::BenchmarkRequest,
    error: GatewayError,
) -> Result<AuditedBenchmarkBatch, GatewayError> {
    let request_hash = benchmark_consumer_admission_request_hash(request);
    match audit_gateway_result_with_receipt::<crate::data_gateway::BenchmarkBar>(
        BENCHMARK_GRPC_CONSUMER_ADMISSION_CAPABILITY,
        ProviderId::Custom,
        &request_hash,
        Err(error),
    ) {
        Err(error) => Err(error),
        Ok(_) => unreachable!("an audited consumer rejection cannot become a successful batch"),
    }
}

fn audit_benchmark_consumer_admission_failure_in(
    database: &crate::database::DatabaseManager,
    request: &crate::data_gateway::BenchmarkRequest,
    error: GatewayError,
) -> Result<AuditedBenchmarkBatch, GatewayError> {
    let request_hash = benchmark_consumer_admission_request_hash(request);
    match audit_gateway_result_with_receipt_state_in::<crate::data_gateway::BenchmarkBar>(
        database,
        BENCHMARK_GRPC_CONSUMER_ADMISSION_CAPABILITY,
        ProviderId::Custom,
        &request_hash,
        Err(error),
    ) {
        Err(error) => Err(error.into_error()),
        Ok(_) => unreachable!("an audited consumer rejection cannot become a successful batch"),
    }
}

fn benchmark_consumer_admission_request_hash(
    request: &crate::data_gateway::BenchmarkRequest,
) -> String {
    let provider_request_hash = super::benchmark::canonical_base_request_hash(request);
    acquisition_request_hash(
        BENCHMARK_GRPC_CONSUMER_ADMISSION_CAPABILITY,
        provider_request_hash.as_bytes(),
    )
}

impl ReviewDataGateway {
    pub const fn new() -> Self {
        Self
    }

    #[allow(dead_code)] // Task 25 wires the private gRPC/client consumer.
    pub(crate) async fn benchmark_bars(
        &self,
        request: crate::data_gateway::BenchmarkRequest,
    ) -> Result<AuditedBenchmarkBatch, GatewayError> {
        match super::grpc_source::bridge_for("BenchmarkBars") {
            Ok(Some(source)) => finish_benchmark_bridge_attempt(
                &request,
                source.benchmark_bars_async(&request).await,
            ),
            Ok(None) if std::env::var("DATA_GATEWAY_GRPC").as_deref() != Ok("1") => {
                self.benchmark_bars_library(request).await
            }
            Ok(None) => finish_benchmark_bridge_attempt(
                &request,
                Err(super::grpc_source::BenchmarkGrpcFailure::client_before_send(
                    GatewayError::classified(
                        "GrpcBridge",
                        None,
                        "unavailable",
                        "bridge_disabled",
                        false,
                        "BenchmarkBars gRPC bridge is configured but disabled; library fallback is forbidden",
                    ),
                )),
            ),
            Err(error) => finish_benchmark_bridge_attempt(
                &request,
                Err(super::grpc_source::BenchmarkGrpcFailure::client_before_send(error)),
            ),
        }
    }

    /// Same benchmark acquisition contract as `benchmark_bars`, but every
    /// client-owned BR-159 audit is bound to the caller's explicit database.
    pub(crate) async fn benchmark_bars_into(
        &self,
        database: &crate::database::DatabaseManager,
        request: crate::data_gateway::BenchmarkRequest,
    ) -> Result<AuditedBenchmarkBatch, GatewayError> {
        match super::grpc_source::bridge_for("BenchmarkBars") {
            Ok(Some(source)) => finish_benchmark_bridge_attempt_in(
                database,
                &request,
                source
                    .benchmark_bars_async_for_local_readmission(&request)
                    .await,
            ),
            Ok(None) if std::env::var("DATA_GATEWAY_GRPC").as_deref() != Ok("1") => {
                self.benchmark_bars_library_audited_into(database, request)
                    .await
                    .map_err(GatewayAuditFailure::into_error)
            }
            Ok(None) => finish_benchmark_bridge_attempt_in(
                database,
                &request,
                Err(super::grpc_source::BenchmarkGrpcFailure::client_before_send(
                    GatewayError::classified(
                        "GrpcBridge",
                        None,
                        "unavailable",
                        "bridge_disabled",
                        false,
                        "BenchmarkBars gRPC bridge is configured but disabled; library fallback is forbidden",
                    ),
                )),
            ),
            Err(error) => finish_benchmark_bridge_attempt_in(
                database,
                &request,
                Err(super::grpc_source::BenchmarkGrpcFailure::client_before_send(error)),
            ),
        }
    }

    #[allow(dead_code)] // Task 25 wires the private gRPC server delegate.
    pub(crate) async fn benchmark_bars_library(
        &self,
        request: crate::data_gateway::BenchmarkRequest,
    ) -> Result<AuditedBenchmarkBatch, GatewayError> {
        self.benchmark_bars_library_audited(request)
            .await
            .map_err(GatewayAuditFailure::into_error)
    }

    pub(crate) async fn benchmark_bars_library_for_grpc(
        &self,
        request: crate::data_gateway::BenchmarkRequest,
    ) -> Result<AuditedBenchmarkBatch, BenchmarkLibraryFailure> {
        self.benchmark_bars_library_audited(request)
            .await
            .map_err(|failure| {
                let audit_state = failure.benchmark_audit_state();
                BenchmarkLibraryFailure {
                    error: failure.into_error(),
                    audit_state,
                }
            })
    }

    async fn benchmark_bars_library_audited(
        &self,
        request: crate::data_gateway::BenchmarkRequest,
    ) -> Result<AuditedBenchmarkBatch, GatewayAuditFailure> {
        let outcome = super::benchmark::acquire_production_benchmark_bars(request).await;
        audit_gateway_result_with_receipt_state(
            "BenchmarkBars",
            ProviderId::Tdx,
            &outcome.request_hash,
            outcome.result,
        )
        .map(|(batch, receipt)| AuditedBenchmarkBatch {
            batch,
            receipt,
            request_hash: outcome.request_hash,
        })
    }

    async fn benchmark_bars_library_audited_into(
        &self,
        database: &crate::database::DatabaseManager,
        request: crate::data_gateway::BenchmarkRequest,
    ) -> Result<AuditedBenchmarkBatch, GatewayAuditFailure> {
        let outcome = super::benchmark::acquire_production_benchmark_bars(request).await;
        audit_gateway_result_with_receipt_state_in(
            database,
            "BenchmarkBars",
            ProviderId::Tdx,
            &outcome.request_hash,
            outcome.result,
        )
        .map(|(batch, receipt)| AuditedBenchmarkBatch {
            batch,
            receipt,
            request_hash: outcome.request_hash,
        })
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
            acquisition_request_hash("R-03", format!("{trading_date}:{WHOLE_LIMIT_POOL_BOUND}"));
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
    /// join display names from the separately admitted identity batches.
    pub(crate) fn current_upper_limit_pool(
        &self,
        trading_date: NaiveDate,
    ) -> Result<GatewayBatch<LimitPoolEntry>, GatewayError> {
        const CAPABILITY: &str = "BR-213-UpperLimitPool";
        let request_hash = limit_pool_acquisition_request_hash(
            CAPABILITY,
            LimitPoolAcquisitionProfile::LocalBridgeV1,
            trading_date,
        )?;
        // BR-238: the synchronous P-01 consumer keeps one interface while the
        // bridge owns its runtime-safe blocking implementation. A configured
        // bridge failure is terminal for this attempt; never fall back to a
        // library provider after transport/schema/evidence failure.
        match super::grpc_source::bridge_for("LimitPools") {
            Ok(Some(bridge)) => {
                let result = bridge
                    .limit_pools(trading_date)
                    .and_then(|batch| admit_current_upper_limit_pool(batch, trading_date));
                return audit_routed_gateway_result(CAPABILITY, &request_hash, result);
            }
            Ok(None) => {}
            Err(error) => {
                return audit_routed_gateway_result(CAPABILITY, &request_hash, Err(error));
            }
        }
        self.current_upper_limit_pool_library(trading_date)
    }

    /// Library-only server seam for LocalBridge `LimitPools`.
    ///
    /// This deliberately bypasses [`super::grpc_source::bridge_for`] so the
    /// server delegate cannot recursively call itself when its environment also
    /// enables the consumer bridge.
    pub(crate) fn current_upper_limit_pool_library(
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
            let request_hash = limit_pool_acquisition_request_hash(
                CAPABILITY,
                LimitPoolAcquisitionProfile::InProcessRouterV1,
                trading_date,
            )?;
            let result = route_exact_date_upper_limit_pool(CAPABILITY, trading_date);
            audit_routed_gateway_result(CAPABILITY, &request_hash, result)
        }
    }
}

/// Re-admit the LocalBridge projection at the synchronous consumer seam. The
/// converter validates wire shape; this layer additionally binds the complete
/// records to the exact BR-213 date, bound and immutable batch evidence before
/// the batch can escape into P-01.
fn admit_current_upper_limit_pool(
    batch: GatewayBatch<LimitPoolEntry>,
    expected_date: NaiveDate,
) -> Result<GatewayBatch<LimitPoolEntry>, GatewayError> {
    const CAPABILITY: &str = "BR-213-UpperLimitPool";
    let evidence = batch.evidence();
    let provider = evidence.provider;
    if !matches!(provider, ProviderId::Eastmoney | ProviderId::Tonghuashun) {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            Some(provider),
            "LimitPools selected an unregistered provider",
        ));
    }
    if evidence.source.trim().is_empty()
        || evidence.observed_at.trim().is_empty()
        || evidence.batch_id.trim().is_empty()
    {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            Some(provider),
            "LimitPools batch evidence is incomplete",
        ));
    }
    let expected_text = expected_date.format("%Y-%m-%d").to_string();
    if evidence.source_at.as_deref() != Some(expected_text.as_str()) {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            Some(provider),
            format!(
                "LimitPools batch source_at {:?} differs from requested {expected_date}",
                evidence.source_at
            ),
        ));
    }
    if batch.records().len() > WHOLE_LIMIT_POOL_BOUND as usize {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            Some(provider),
            format!(
                "LimitPools returned {} records, exceeding bound {WHOLE_LIMIT_POOL_BOUND}",
                batch.records().len()
            ),
        ));
    }

    let mut identities = HashSet::with_capacity(batch.records().len());
    for record in batch.records() {
        let record_date = NaiveDate::parse_from_str(record.trading_date.as_str(), "%Y-%m-%d")
            .map_err(|error| {
                GatewayError::invalid_evidence(
                    CAPABILITY,
                    Some(provider),
                    format!("invalid LimitPools trading date: {error}"),
                )
            })?;
        if record.kind != LimitPoolKind::Upper
            || record_date != expected_date
            || record.evidence.provider() != provider
            || record.evidence.batch_id() != evidence.batch_id
            || record.evidence.source_at() != evidence.source_at.as_deref()
            || record.evidence.observed_at() != evidence.observed_at
        {
            return Err(GatewayError::invalid_evidence(
                CAPABILITY,
                Some(provider),
                "LimitPools record differs from requested date or batch evidence",
            ));
        }
        if !identities.insert(record.instrument.code().to_owned()) {
            return Err(GatewayError::invalid_evidence(
                CAPABILITY,
                Some(provider),
                format!(
                    "LimitPools contains duplicate security {}",
                    record.instrument.code()
                ),
            ));
        }
    }
    Ok(batch)
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

pub(super) fn acquisition_request_hash(
    capability: &str,
    canonical_request: impl AsRef<[u8]>,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"BR159_DATA_GATEWAY_REQUEST_V1\0");
    hasher.update(capability.as_bytes());
    hasher.update(b"\0");
    hasher.update(canonical_request.as_ref());
    hex::encode(hasher.finalize())
}

#[derive(Debug)]
enum GatewayAuditFailure {
    Persisted(GatewayError),
    AppendFailed(GatewayError),
}

impl GatewayAuditFailure {
    fn into_error(self) -> GatewayError {
        match self {
            Self::Persisted(error) | Self::AppendFailed(error) => error,
        }
    }

    fn benchmark_audit_state(&self) -> super::grpc_source::BenchmarkServerAuditState {
        match self {
            Self::Persisted(_) => super::grpc_source::BenchmarkServerAuditState::Persisted,
            Self::AppendFailed(_) => super::grpc_source::BenchmarkServerAuditState::AppendFailed,
        }
    }
}

fn audit_gateway_result_with_receipt_state_in<T>(
    database: &crate::database::DatabaseManager,
    capability: &'static str,
    provider: ProviderId,
    request_hash: &str,
    result: Result<GatewayBatch<T>, GatewayError>,
) -> Result<
    (
        GatewayBatch<T>,
        crate::database::data_acquisition_audit::DataAcquisitionAuditReceipt,
    ),
    GatewayAuditFailure,
> {
    use crate::database::data_acquisition_audit::DataAcquisitionAuditRecord;

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
                GatewayAuditFailure::AppendFailed(GatewayError::audit_failure(
                    capability,
                    provider,
                    "accepted_count_overflow",
                    "accepted record count exceeds SQLite INTEGER",
                ))
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
    let receipt = database.record_data_acquisition(&record).map_err(|error| {
        GatewayAuditFailure::AppendFailed(GatewayError::audit_failure(
            capability,
            provider,
            original_reason_code,
            error,
        ))
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
    match result {
        Ok(batch) => Ok((batch, receipt)),
        Err(error) => Err(GatewayAuditFailure::Persisted(error)),
    }
}

fn audit_gateway_result_with_receipt_state<T>(
    capability: &'static str,
    provider: ProviderId,
    request_hash: &str,
    result: Result<GatewayBatch<T>, GatewayError>,
) -> Result<
    (
        GatewayBatch<T>,
        crate::database::data_acquisition_audit::DataAcquisitionAuditReceipt,
    ),
    GatewayAuditFailure,
> {
    let original_reason_code = result
        .as_ref()
        .map_or_else(|error| error.reason_code(), |_| "accepted");
    let database = crate::database::DatabaseManager::try_get().ok_or_else(|| {
        GatewayAuditFailure::AppendFailed(GatewayError::audit_failure(
            capability,
            provider,
            original_reason_code,
            "database is not initialized",
        ))
    })?;
    audit_gateway_result_with_receipt_state_in(database, capability, provider, request_hash, result)
}

pub(super) fn audit_gateway_result_with_receipt<T>(
    capability: &'static str,
    provider: ProviderId,
    request_hash: &str,
    result: Result<GatewayBatch<T>, GatewayError>,
) -> Result<
    (
        GatewayBatch<T>,
        crate::database::data_acquisition_audit::DataAcquisitionAuditReceipt,
    ),
    GatewayError,
> {
    audit_gateway_result_with_receipt_state(capability, provider, request_hash, result)
        .map_err(GatewayAuditFailure::into_error)
}

pub(super) fn audit_gateway_result<T>(
    capability: &'static str,
    provider: ProviderId,
    request_hash: &str,
    result: Result<GatewayBatch<T>, GatewayError>,
) -> Result<GatewayBatch<T>, GatewayError> {
    audit_gateway_result_with_receipt(capability, provider, request_hash, result)
        .map(|(batch, _receipt)| batch)
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
    use crate::database::attribution_reports::{
        AttributionDatabaseAccess, AttributionDatabaseSession,
    };
    use crate::database::DatabaseManager;
    use crate::magic_compat::{
        AssetClass, DataBatch, Exchange, InstrumentId, IsoDate, Money, NonEmptyText, Price,
        Provenance, Quantity, Ratio, RatioUnit, SourceEvidence,
    };
    use diesel::prelude::*;
    use diesel::sql_types::{BigInt, Integer, Nullable, Text};
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

    #[derive(Debug, QueryableByName)]
    struct AcquisitionReceiptRow {
        #[diesel(sql_type = Text)]
        outcome: String,
        #[diesel(sql_type = Text)]
        reason_code: String,
        #[diesel(sql_type = Text)]
        record_hash: String,
    }

    #[derive(Debug, QueryableByName)]
    struct AcquisitionCountRow {
        #[diesel(sql_type = BigInt)]
        count: i64,
    }

    #[derive(Debug, QueryableByName)]
    struct BenchmarkTransportAuditRow {
        #[diesel(sql_type = Text)]
        capability: String,
        #[diesel(sql_type = Text)]
        provider: String,
        #[diesel(sql_type = Text)]
        request_hash: String,
        #[diesel(sql_type = Text)]
        outcome: String,
        #[diesel(sql_type = Text)]
        reason_code: String,
        #[diesel(sql_type = Integer)]
        retryable: i32,
    }

    #[cfg(not(feature = "magic-gateway"))]
    #[derive(Debug, QueryableByName)]
    struct AcquisitionRequestAuditRow {
        #[diesel(sql_type = Text)]
        request_hash: String,
    }

    #[test]
    fn br251_explicit_benchmark_audit_uses_the_supplied_database() {
        let path = std::env::temp_dir().join(format!(
            "TEST_CODE_explicit_benchmark_audit_{}_{}.sqlite3",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("TEST_CODE clock")
                .as_nanos()
        ));
        drop(rusqlite::Connection::open(&path).expect("TEST_CODE create explicit database"));
        let session =
            AttributionDatabaseSession::open(&path, AttributionDatabaseAccess::AppendOnly)
                .expect("TEST_CODE narrow append-only schema");
        let request_hash = "a".repeat(64);
        let provider_error = GatewayError::classified(
            "TEST_CODE_BenchmarkBars",
            Some(ProviderId::Tdx),
            "unavailable",
            "TEST_CODE_provider_unavailable",
            true,
            "TEST_CODE explicit audit binding",
        );

        let result = audit_gateway_result_with_receipt_state_in::<crate::data_gateway::BenchmarkBar>(
            session.database(),
            "TEST_CODE_BenchmarkBars",
            ProviderId::Tdx,
            &request_hash,
            Err(provider_error),
        );
        assert!(matches!(result, Err(GatewayAuditFailure::Persisted(_))));

        let mut connection = session
            .database()
            .get_conn()
            .expect("TEST_CODE explicit audit connection");
        let count = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM data_acquisition_audit
             WHERE capability='TEST_CODE_BenchmarkBars' AND request_hash=?",
        )
        .bind::<Text, _>(&request_hash)
        .get_result::<AcquisitionCountRow>(&mut connection)
        .expect("TEST_CODE explicit audit row")
        .count;
        assert_eq!(count, 1);
        drop(connection);
        drop(session);
        std::fs::remove_file(&path).expect("TEST_CODE remove exact temporary database");
    }

    #[test]
    fn br251_grpc_success_replaces_remote_receipt_with_supplied_database_receipt() {
        fn path(label: &str) -> std::path::PathBuf {
            std::env::temp_dir().join(format!(
                "TEST_CODE_grpc_local_audit_{label}_{}_{}.sqlite3",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("TEST_CODE clock")
                    .as_nanos()
            ))
        }

        let local_path = path("local");
        let remote_path = path("remote");
        drop(rusqlite::Connection::open(&local_path).expect("TEST_CODE local database"));
        drop(rusqlite::Connection::open(&remote_path).expect("TEST_CODE remote database"));
        let local =
            AttributionDatabaseSession::open(&local_path, AttributionDatabaseAccess::AppendOnly)
                .expect("TEST_CODE local append-only schema");
        let remote =
            AttributionDatabaseSession::open(&remote_path, AttributionDatabaseAccess::AppendOnly)
                .expect("TEST_CODE remote append-only schema");

        let seed_hash = "b".repeat(64);
        let seed = audit_gateway_result_with_receipt_state_in::<crate::data_gateway::BenchmarkBar>(
            local.database(),
            "TEST_CODE_seed",
            ProviderId::Tdx,
            &seed_hash,
            Err(GatewayError::classified(
                "TEST_CODE_seed",
                Some(ProviderId::Tdx),
                "unavailable",
                "TEST_CODE_seed",
                false,
                "TEST_CODE establish a distinct local audit chain",
            )),
        );
        assert!(matches!(seed, Err(GatewayAuditFailure::Persisted(_))));

        let day = NaiveDate::from_ymd_opt(2026, 8, 21).expect("TEST_CODE date");
        let request = crate::data_gateway::BenchmarkRequest {
            instrument: "TEST_CODE_000300".to_owned(),
            range: crate::data_gateway::BenchmarkRange::Daily { from: day, to: day },
        };
        let request_hash = super::super::benchmark::canonical_base_request_hash(&request);
        let batch = GatewayBatch::Available {
            records: vec![crate::data_gateway::BenchmarkBar {
                at: crate::data_gateway::BenchmarkBarTime::Daily(day),
                open: 3_500.0,
                high: 3_510.0,
                low: 3_490.0,
                close: 3_505.0,
                volume: None,
                amount: Some(8_000.0),
            }],
            evidence: BatchEvidence {
                provider: ProviderId::Tdx,
                source: "TEST_CODE remote benchmark provider".to_owned(),
                source_at: Some("2026-08-21T15:00:00+08:00".to_owned()),
                observed_at: "2026-08-21T15:01:00+08:00".to_owned(),
                batch_id: "TEST_CODE remote batch".to_owned(),
            },
        };
        let (batch, remote_receipt) = audit_gateway_result_with_receipt_state_in(
            remote.database(),
            "BenchmarkBars",
            ProviderId::Tdx,
            &request_hash,
            Ok(batch),
        )
        .expect("TEST_CODE remote server receipt");

        let returned = finish_benchmark_bridge_attempt_in(
            local.database(),
            &request,
            Ok(AuditedBenchmarkBatch {
                batch,
                receipt: remote_receipt.clone(),
                request_hash: request_hash.clone(),
            }),
        )
        .expect("TEST_CODE caller-local re-admission");
        assert_ne!(returned.receipt.audit_id, remote_receipt.audit_id);

        let mut connection = local
            .database()
            .get_conn()
            .expect("TEST_CODE local audit connection");
        let row = diesel::sql_query(
            "SELECT audit.outcome,audit.reason_code,chain.record_hash
             FROM data_acquisition_audit AS audit
             JOIN data_acquisition_audit_chain AS chain
               ON chain.acquisition_audit_id=audit.id
             WHERE audit.capability='BenchmarkBars' AND audit.request_hash=?",
        )
        .bind::<Text, _>(&request_hash)
        .get_result::<AcquisitionReceiptRow>(&mut connection)
        .expect("TEST_CODE local BenchmarkBars receipt");
        assert_eq!(row.outcome, "available");
        assert_eq!(row.reason_code, "accepted");
        assert_eq!(row.record_hash, returned.receipt.record_hash);
        drop(connection);
        drop(local);
        drop(remote);
        std::fs::remove_file(&local_path).expect("TEST_CODE remove local database");
        std::fs::remove_file(&remote_path).expect("TEST_CODE remove remote database");
    }

    #[test]
    fn br251_server_handled_failure_is_audited_in_supplied_database() {
        let path = std::env::temp_dir().join(format!(
            "TEST_CODE_grpc_failure_local_audit_{}_{}.sqlite3",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("TEST_CODE clock")
                .as_nanos()
        ));
        drop(rusqlite::Connection::open(&path).expect("TEST_CODE explicit database"));
        let session =
            AttributionDatabaseSession::open(&path, AttributionDatabaseAccess::AppendOnly)
                .expect("TEST_CODE append-only schema");
        let day = NaiveDate::from_ymd_opt(2026, 8, 21).expect("TEST_CODE date");
        let request = crate::data_gateway::BenchmarkRequest {
            instrument: "TEST_CODE_000300".to_owned(),
            range: crate::data_gateway::BenchmarkRange::Daily { from: day, to: day },
        };
        let returned = finish_benchmark_bridge_attempt_in(
            session.database(),
            &request,
            Err(super::super::grpc_source::benchmark_typed_failure_for_test(
                GatewayError::classified(
                    "BenchmarkBars",
                    Some(ProviderId::Tdx),
                    "unavailable",
                    "provider_transport",
                    true,
                    "TEST_CODE server-handled provider failure",
                ),
            )),
        )
        .expect_err("TEST_CODE server failure remains terminal");
        assert_eq!(returned.reason_code(), "provider_transport");

        let request_hash = super::super::benchmark::canonical_base_request_hash(&request);
        let mut connection = session
            .database()
            .get_conn()
            .expect("TEST_CODE local audit connection");
        let row = diesel::sql_query(
            "SELECT audit.outcome,audit.reason_code,chain.record_hash
             FROM data_acquisition_audit AS audit
             JOIN data_acquisition_audit_chain AS chain
               ON chain.acquisition_audit_id=audit.id
             WHERE audit.capability='BenchmarkBars' AND audit.request_hash=?",
        )
        .bind::<Text, _>(&request_hash)
        .get_result::<AcquisitionReceiptRow>(&mut connection)
        .expect("TEST_CODE local failure audit");
        assert_eq!(row.outcome, "unavailable");
        assert_eq!(row.reason_code, "provider_transport");
        drop(connection);
        drop(session);
        std::fs::remove_file(&path).expect("TEST_CODE remove explicit database");
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

    #[test]
    #[serial]
    fn br159_success_returns_receipt_only_after_the_row_and_hash_are_persisted() {
        let _env = super::super::grpc_source::test_grpc_env_guard();
        DatabaseManager::init(None).expect("TEST_CODE audit database init");
        let batch = GatewayBatch::Available {
            records: vec![DailyClose {
                date: NaiveDate::from_ymd_opt(2099, 1, 2).unwrap(),
                close: 10.0,
            }],
            evidence: evidence(ProviderId::Tdx, "TEST_CODE_receipt_batch"),
        };

        let request_hash =
            acquisition_request_hash("TEST_CODE-BenchmarkBars", "TEST_CODE_request_hash_preimage");
        let (returned, receipt) = audit_gateway_result_with_receipt(
            "TEST_CODE-BenchmarkBars",
            ProviderId::Tdx,
            &request_hash,
            Ok(batch),
        )
        .expect("accepted batch must carry its durable BR-159 receipt");

        let mut connection = DatabaseManager::get().get_conn().unwrap();
        let row = diesel::sql_query(
            "SELECT a.outcome, a.reason_code, c.record_hash \
             FROM data_acquisition_audit a \
             JOIN data_acquisition_audit_chain c ON c.acquisition_audit_id = a.id \
             WHERE a.id = ?",
        )
        .bind::<BigInt, _>(receipt.audit_id)
        .get_result::<AcquisitionReceiptRow>(&mut *connection)
        .expect("receipt must identify a real audit row and chain hash");

        assert_eq!(returned.records().len(), 1);
        assert_eq!(row.outcome, "available");
        assert_eq!(row.reason_code, "accepted");
        assert_eq!(row.record_hash, receipt.record_hash);
    }

    #[test]
    #[serial]
    fn br159_audit_failure_discards_an_accepted_batch() {
        let _env = super::super::grpc_source::test_grpc_env_guard();
        DatabaseManager::init(None).expect("TEST_CODE audit database init");
        let batch = GatewayBatch::Available {
            records: vec![DailyClose {
                date: NaiveDate::from_ymd_opt(2099, 1, 2).unwrap(),
                close: 10.0,
            }],
            evidence: evidence(ProviderId::Tdx, "TEST_CODE_discarded_batch"),
        };

        let error = audit_gateway_result_with_receipt(
            "TEST_CODE-BenchmarkAuditFailure",
            ProviderId::Tdx,
            "TEST_CODE_invalid_hash_for_forced_audit_failure",
            Ok(batch),
        )
        .expect_err("a batch must not escape when its BR-159 append fails");

        assert_eq!(error.reason_code(), "acquisition_audit_unavailable");
    }

    #[test]
    #[serial]
    fn br159_error_outcome_is_persisted_before_the_original_typed_error_returns() {
        let _env = super::super::grpc_source::test_grpc_env_guard();
        DatabaseManager::init(None).expect("TEST_CODE audit database init");
        let request_hash = acquisition_request_hash(
            "TEST_CODE-BenchmarkError",
            "TEST_CODE_provider_failure_preimage",
        );
        let original = GatewayError::classified(
            "TEST_CODE-BenchmarkError",
            Some(ProviderId::Tdx),
            "unavailable",
            "TEST_CODE_provider_failure",
            true,
            "TEST_CODE provider failed",
        );

        let returned = audit_gateway_result_with_receipt::<DailyClose>(
            "TEST_CODE-BenchmarkError",
            ProviderId::Tdx,
            &request_hash,
            Err(original),
        )
        .expect_err("provider failure remains a typed error after audit");
        assert_eq!(returned.reason_code(), "TEST_CODE_provider_failure");

        let mut connection = DatabaseManager::get().get_conn().unwrap();
        let row = diesel::sql_query(
            "SELECT a.outcome, a.reason_code, c.record_hash \
             FROM data_acquisition_audit a \
             JOIN data_acquisition_audit_chain c ON c.acquisition_audit_id = a.id \
             WHERE a.capability = ? AND a.request_hash = ? ORDER BY a.id DESC LIMIT 1",
        )
        .bind::<Text, _>("TEST_CODE-BenchmarkError")
        .bind::<Text, _>(&request_hash)
        .get_result::<AcquisitionReceiptRow>(&mut *connection)
        .expect("typed provider error must be durably audited");
        assert_eq!(row.outcome, "unavailable");
        assert_eq!(row.reason_code, "TEST_CODE_provider_failure");
        assert_eq!(row.record_hash.len(), 64);
    }

    #[tokio::test]
    #[serial]
    async fn benchmark_entrypoint_delegates_to_library_and_appends_exactly_one_audit_row() {
        let _env = super::super::grpc_source::test_grpc_env_guard();
        DatabaseManager::init(None).expect("TEST_CODE audit database init");
        let mut connection = DatabaseManager::get().get_conn().unwrap();
        let before = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM data_acquisition_audit \
             WHERE capability = 'BenchmarkBars'",
        )
        .get_result::<AcquisitionCountRow>(&mut *connection)
        .unwrap()
        .count;
        drop(connection);

        let day = NaiveDate::from_ymd_opt(2026, 8, 21).unwrap();
        let request = crate::data_gateway::BenchmarkRequest {
            instrument: crate::data_gateway::HS300_CANONICAL.to_owned(),
            range: crate::data_gateway::BenchmarkRange::Daily { from: day, to: day },
        };
        let expected_request_hash = super::super::benchmark::canonical_base_request_hash(&request);
        let error = ReviewDataGateway::new()
            .benchmark_bars(request)
            .await
            .expect_err("production identity attestation is intentionally unavailable");
        assert_eq!(error.reason_code(), "benchmark_identity_unverified");

        let mut connection = DatabaseManager::get().get_conn().unwrap();
        let after = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM data_acquisition_audit \
             WHERE capability = 'BenchmarkBars'",
        )
        .get_result::<AcquisitionCountRow>(&mut *connection)
        .unwrap()
        .count;
        assert_eq!(after - before, 1, "delegation must not audit twice");
        let row = diesel::sql_query(
            "SELECT capability, provider, request_hash, outcome, reason_code, retryable \
             FROM data_acquisition_audit WHERE capability = 'BenchmarkBars' \
             ORDER BY id DESC LIMIT 1",
        )
        .get_result::<BenchmarkTransportAuditRow>(&mut *connection)
        .expect("failed provider acquisition audit");
        assert_eq!(row.request_hash, expected_request_hash);
    }

    #[tokio::test]
    async fn grpc_env_guard_configured_benchmark_connect_failure_has_one_transport_audit() {
        let _env = super::super::grpc_source::test_grpc_env_guard();
        DatabaseManager::init(None).expect("TEST_CODE audit database init");
        let mut connection = DatabaseManager::get().get_conn().unwrap();
        let before = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM data_acquisition_audit \
             WHERE capability = 'BenchmarkBarsGrpcTransport'",
        )
        .get_result::<AcquisitionCountRow>(&mut *connection)
        .unwrap()
        .count;
        drop(connection);

        std::env::set_var("DATA_GATEWAY_GRPC", "1");
        std::env::remove_var("DATA_GATEWAY_GRPC_DISABLED");
        std::env::set_var("GRPC_MARKET_ADDR", "http://127.0.0.1:1");
        super::super::grpc_source::reset_bridge();
        let day = NaiveDate::from_ymd_opt(2026, 8, 21).unwrap();
        let result = ReviewDataGateway::new()
            .benchmark_bars(crate::data_gateway::BenchmarkRequest {
                instrument: crate::data_gateway::HS300_CANONICAL.to_owned(),
                range: crate::data_gateway::BenchmarkRange::Daily { from: day, to: day },
            })
            .await;
        std::env::remove_var("DATA_GATEWAY_GRPC");
        std::env::remove_var("GRPC_MARKET_ADDR");
        super::super::grpc_source::reset_bridge();

        let error = result.expect_err("configured unreachable bridge must be terminal");
        assert_eq!(error.capability(), "GrpcBridge");
        assert_eq!(error.reason_code(), "grpc_connect_failed");
        assert!(error.retryable());

        let mut connection = DatabaseManager::get().get_conn().unwrap();
        let after = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM data_acquisition_audit \
             WHERE capability = 'BenchmarkBarsGrpcTransport'",
        )
        .get_result::<AcquisitionCountRow>(&mut *connection)
        .unwrap()
        .count;
        assert_eq!(after - before, 1, "connect failure has one transport owner");
        let row = diesel::sql_query(
            "SELECT capability, provider, request_hash, outcome, reason_code, retryable \
             FROM data_acquisition_audit WHERE capability = 'BenchmarkBarsGrpcTransport' \
             ORDER BY id DESC LIMIT 1",
        )
        .get_result::<BenchmarkTransportAuditRow>(&mut *connection)
        .expect("connect failure transport audit");
        assert_eq!(row.capability, "BenchmarkBarsGrpcTransport");
        assert_eq!(row.provider, "Custom");
        assert_eq!(row.request_hash.len(), 64);
        assert_eq!(row.outcome, "unavailable");
        assert_eq!(row.reason_code, "grpc_connect_failed");
        assert_eq!(row.retryable, 1);
    }

    #[tokio::test]
    async fn grpc_env_guard_configured_benchmark_disabled_does_not_fall_back() {
        let _env = super::super::grpc_source::test_grpc_env_guard();
        DatabaseManager::init(None).expect("TEST_CODE audit database init");
        let mut connection = DatabaseManager::get().get_conn().unwrap();
        let before = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM data_acquisition_audit \
             WHERE capability = 'BenchmarkBarsGrpcTransport'",
        )
        .get_result::<AcquisitionCountRow>(&mut *connection)
        .unwrap()
        .count;
        drop(connection);

        std::env::set_var("DATA_GATEWAY_GRPC", "1");
        std::env::set_var("DATA_GATEWAY_GRPC_DISABLED", "BenchmarkBars");
        super::super::grpc_source::reset_bridge();
        let day = NaiveDate::from_ymd_opt(2026, 8, 21).unwrap();
        let result = ReviewDataGateway::new()
            .benchmark_bars(crate::data_gateway::BenchmarkRequest {
                instrument: crate::data_gateway::HS300_CANONICAL.to_owned(),
                range: crate::data_gateway::BenchmarkRange::Daily { from: day, to: day },
            })
            .await;
        std::env::remove_var("DATA_GATEWAY_GRPC");
        std::env::remove_var("DATA_GATEWAY_GRPC_DISABLED");
        super::super::grpc_source::reset_bridge();

        let error = result.expect_err("a configured bridge opt-out must remain terminal");
        assert_eq!(error.capability(), "GrpcBridge");
        assert_eq!(error.reason_code(), "bridge_disabled");

        let mut connection = DatabaseManager::get().get_conn().unwrap();
        let after = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM data_acquisition_audit \
             WHERE capability = 'BenchmarkBarsGrpcTransport'",
        )
        .get_result::<AcquisitionCountRow>(&mut *connection)
        .unwrap()
        .count;
        assert_eq!(after - before, 1, "disabled bridge has one transport owner");
        let row = diesel::sql_query(
            "SELECT capability, provider, request_hash, outcome, reason_code, retryable \
             FROM data_acquisition_audit WHERE capability = 'BenchmarkBarsGrpcTransport' \
             ORDER BY id DESC LIMIT 1",
        )
        .get_result::<BenchmarkTransportAuditRow>(&mut *connection)
        .expect("disabled bridge transport audit");
        assert_eq!(row.capability, "BenchmarkBarsGrpcTransport");
        assert_eq!(row.provider, "Custom");
        assert_eq!(row.request_hash.len(), 64);
        assert_eq!(row.outcome, "unavailable");
        assert_eq!(row.reason_code, "bridge_disabled");
        assert_eq!(row.retryable, 0);
    }

    #[test]
    #[serial]
    fn benchmark_server_owned_success_and_typed_failure_do_not_append_transport_audits() {
        let _env = super::super::grpc_source::test_grpc_env_guard();
        DatabaseManager::init(None).expect("TEST_CODE audit database init");
        let day = NaiveDate::from_ymd_opt(2026, 8, 21).expect("TEST_CODE date");
        let request = crate::data_gateway::BenchmarkRequest {
            instrument: crate::data_gateway::HS300_CANONICAL.to_owned(),
            range: crate::data_gateway::BenchmarkRange::Daily { from: day, to: day },
        };
        let request_hash = super::super::benchmark::canonical_base_request_hash(&request);
        let count = |capability: &str| {
            let mut connection = DatabaseManager::get().get_conn().unwrap();
            diesel::sql_query(
                "SELECT COUNT(*) AS count FROM data_acquisition_audit WHERE capability = ?",
            )
            .bind::<Text, _>(capability)
            .get_result::<AcquisitionCountRow>(&mut *connection)
            .unwrap()
            .count
        };
        let provider_before = count("BenchmarkBars");
        let transport_before = count("BenchmarkBarsGrpcTransport");
        let batch = GatewayBatch::Available {
            records: vec![crate::data_gateway::BenchmarkBar {
                at: crate::data_gateway::BenchmarkBarTime::Daily(day),
                open: 3_500.0,
                high: 3_510.0,
                low: 3_490.0,
                close: 3_505.0,
                volume: None,
                amount: Some(8_000.0),
            }],
            evidence: BatchEvidence {
                provider: ProviderId::Tdx,
                source: "TEST_CODE benchmark provider".to_owned(),
                source_at: None,
                observed_at: "2026-08-21T15:01:00+08:00".to_owned(),
                batch_id: "TEST_CODE benchmark success".to_owned(),
            },
        };
        let (batch, receipt) = audit_gateway_result_with_receipt(
            "BenchmarkBars",
            ProviderId::Tdx,
            &request_hash,
            Ok(batch),
        )
        .expect("TEST_CODE server provider success audit");
        let expected_receipt = receipt.clone();
        let admitted = finish_benchmark_bridge_attempt(
            &request,
            Ok(AuditedBenchmarkBatch {
                batch,
                receipt,
                request_hash: request_hash.clone(),
            }),
        )
        .expect("server-owned success reuses receipt");
        assert_eq!(admitted.receipt, expected_receipt);
        assert_eq!(count("BenchmarkBars") - provider_before, 1);
        assert_eq!(count("BenchmarkBarsGrpcTransport") - transport_before, 0);

        let provider_before = count("BenchmarkBars");
        let transport_before = count("BenchmarkBarsGrpcTransport");
        let provider_error = GatewayError::classified(
            "BenchmarkBars",
            Some(ProviderId::Tdx),
            "unsupported",
            "benchmark_instrument_unsupported",
            false,
            "TEST_CODE typed server failure",
        );
        let audited_error = audit_gateway_result_with_receipt::<crate::data_gateway::BenchmarkBar>(
            "BenchmarkBars",
            ProviderId::Tdx,
            &request_hash,
            Err(provider_error),
        )
        .expect_err("TEST_CODE server provider failure audit");
        let returned = finish_benchmark_bridge_attempt(
            &request,
            Err(super::super::grpc_source::benchmark_typed_failure_for_test(
                audited_error,
            )),
        )
        .expect_err("server-owned typed failure remains terminal");
        assert_eq!(returned.audit_outcome(), "unsupported");
        assert_eq!(returned.reason_code(), "benchmark_instrument_unsupported");
        assert!(!returned.retryable());
        assert_eq!(count("BenchmarkBars") - provider_before, 1);
        assert_eq!(count("BenchmarkBarsGrpcTransport") - transport_before, 0);
    }

    #[test]
    #[serial]
    fn benchmark_server_audit_append_failure_has_one_client_transport_audit() {
        let _env = super::super::grpc_source::test_grpc_env_guard();
        DatabaseManager::init(None).expect("TEST_CODE audit database init");
        let day = NaiveDate::from_ymd_opt(2026, 8, 21).expect("TEST_CODE date");
        let request = crate::data_gateway::BenchmarkRequest {
            instrument: crate::data_gateway::HS300_CANONICAL.to_owned(),
            range: crate::data_gateway::BenchmarkRange::Daily { from: day, to: day },
        };
        let count = |capability: &str| {
            let mut connection = DatabaseManager::get().get_conn().unwrap();
            diesel::sql_query(
                "SELECT COUNT(*) AS count FROM data_acquisition_audit WHERE capability = ?",
            )
            .bind::<Text, _>(capability)
            .get_result::<AcquisitionCountRow>(&mut *connection)
            .unwrap()
            .count
        };
        let provider_before = count("BenchmarkBars");
        let transport_before = count("BenchmarkBarsGrpcTransport");
        let consumer_before = count("BenchmarkBarsGrpcConsumerAdmission");
        let provider_error = GatewayError::classified(
            "BenchmarkBars",
            Some(ProviderId::Tdx),
            "unavailable",
            "provider_transport",
            true,
            "TEST_CODE provider failed before audit append",
        );
        let audit_failure =
            audit_gateway_result_with_receipt_state::<crate::data_gateway::BenchmarkBar>(
                "BenchmarkBars",
                ProviderId::Tdx,
                "TEST_CODE_invalid_request_hash",
                Err(provider_error),
            )
            .expect_err("invalid audit request hash must make the server append fail");
        assert_eq!(
            audit_failure.benchmark_audit_state(),
            super::super::grpc_source::BenchmarkServerAuditState::AppendFailed
        );
        let returned = finish_benchmark_bridge_attempt(
            &request,
            Err(
                super::super::grpc_source::benchmark_typed_failure_with_state_for_test(
                    audit_failure.into_error(),
                    super::super::grpc_source::BenchmarkServerAuditState::AppendFailed,
                ),
            ),
        )
        .expect_err("server append failure must be persisted by the client transport owner");

        assert_eq!(count("BenchmarkBars") - provider_before, 0);
        assert_eq!(count("BenchmarkBarsGrpcTransport") - transport_before, 1);
        assert_eq!(
            count("BenchmarkBarsGrpcConsumerAdmission") - consumer_before,
            0
        );
        assert_eq!(returned.audit_outcome(), "unavailable");
        assert_eq!(returned.reason_code(), "acquisition_audit_unavailable");
        assert!(returned.retryable());
        let mut connection = DatabaseManager::get().get_conn().unwrap();
        let row = diesel::sql_query(
            "SELECT capability, provider, request_hash, outcome, reason_code, retryable \
             FROM data_acquisition_audit WHERE capability = 'BenchmarkBarsGrpcTransport' \
             ORDER BY id DESC LIMIT 1",
        )
        .get_result::<BenchmarkTransportAuditRow>(&mut *connection)
        .expect("client-owned server audit failure row");
        assert_eq!(row.outcome, "unavailable");
        assert_eq!(row.reason_code, "acquisition_audit_unavailable");
        assert_eq!(row.retryable, 1);
    }

    #[test]
    #[serial]
    fn benchmark_ambiguous_loss_appends_one_unknown_transport_audit() {
        let _env = super::super::grpc_source::test_grpc_env_guard();
        DatabaseManager::init(None).expect("TEST_CODE audit database init");
        let day = NaiveDate::from_ymd_opt(2026, 8, 21).expect("TEST_CODE date");
        let request = crate::data_gateway::BenchmarkRequest {
            instrument: crate::data_gateway::HS300_CANONICAL.to_owned(),
            range: crate::data_gateway::BenchmarkRange::Daily { from: day, to: day },
        };
        let same_hash = benchmark_transport_request_hash(&request);
        let different_hash =
            benchmark_transport_request_hash(&crate::data_gateway::BenchmarkRequest {
                instrument: crate::data_gateway::HS300_CANONICAL.to_owned(),
                range: crate::data_gateway::BenchmarkRange::Daily {
                    from: day,
                    to: day.succ_opt().expect("TEST_CODE next day"),
                },
            });
        assert_eq!(same_hash, benchmark_transport_request_hash(&request));
        assert_ne!(same_hash, different_hash);
        let mut connection = DatabaseManager::get().get_conn().unwrap();
        let before = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM data_acquisition_audit \
             WHERE capability = 'BenchmarkBarsGrpcTransport'",
        )
        .get_result::<AcquisitionCountRow>(&mut *connection)
        .unwrap()
        .count;
        drop(connection);

        let returned = finish_benchmark_bridge_attempt(
            &request,
            Err(super::super::grpc_source::benchmark_unknown_failure_for_test()),
        )
        .expect_err("ambiguous transport result is terminal");
        assert_eq!(returned.audit_outcome(), "unavailable");
        assert_eq!(returned.reason_code(), "transport_outcome_unknown");
        assert!(returned.retryable());

        let mut connection = DatabaseManager::get().get_conn().unwrap();
        let after = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM data_acquisition_audit \
             WHERE capability = 'BenchmarkBarsGrpcTransport'",
        )
        .get_result::<AcquisitionCountRow>(&mut *connection)
        .unwrap()
        .count;
        assert_eq!(after - before, 1);
        let row = diesel::sql_query(
            "SELECT capability, provider, request_hash, outcome, reason_code, retryable \
             FROM data_acquisition_audit WHERE capability = 'BenchmarkBarsGrpcTransport' \
             ORDER BY id DESC LIMIT 1",
        )
        .get_result::<BenchmarkTransportAuditRow>(&mut *connection)
        .expect("ambiguous transport audit");
        assert_eq!(row.provider, "Custom");
        assert_eq!(row.outcome, "unavailable");
        assert_eq!(row.reason_code, "transport_outcome_unknown");
        assert_eq!(row.retryable, 1);
    }

    #[tokio::test]
    #[serial]
    async fn unverified_benchmark_envelope_has_one_unknown_transport_audit() {
        let _env = super::super::grpc_source::test_grpc_env_guard();
        DatabaseManager::init(None).expect("TEST_CODE audit database init");
        let day = NaiveDate::from_ymd_opt(2026, 8, 21).expect("TEST_CODE date");
        let request = crate::data_gateway::BenchmarkRequest {
            instrument: crate::data_gateway::HS300_CANONICAL.to_owned(),
            range: crate::data_gateway::BenchmarkRange::Daily { from: day, to: day },
        };
        let count = |capability: &str| {
            let mut connection = DatabaseManager::get().get_conn().unwrap();
            diesel::sql_query(
                "SELECT COUNT(*) AS count FROM data_acquisition_audit WHERE capability = ?",
            )
            .bind::<Text, _>(capability)
            .get_result::<AcquisitionCountRow>(&mut *connection)
            .unwrap()
            .count
        };
        let provider_before = count("BenchmarkBars");
        let transport_before = count("BenchmarkBarsGrpcTransport");
        let malformed = crate::grpc_client::envelope::QueryResult {
            admission: crate::grpc_client::pb::magic::market::v1::AdmissionState::Admitted,
            selected_provider: "Tdx".to_owned(),
            batch_id: "TEST_CODE_unverified_envelope".to_owned(),
            complete: true,
            observed_at: "2026-08-21T15:01:00+08:00".to_owned(),
            source_at: String::new(),
            records: Vec::new(),
            source: "TEST_CODE_unverified_provider_response".to_owned(),
            diagnostic_blocker: String::new(),
        };

        let result =
            super::super::grpc_source::benchmark_bars_with_test_query(&request, malformed).await;
        let returned = finish_benchmark_bridge_attempt(&request, result)
            .expect_err("unverified response must be a terminal unknown transport outcome");

        assert_eq!(count("BenchmarkBars") - provider_before, 0);
        assert_eq!(
            count("BenchmarkBarsGrpcTransport") - transport_before,
            1,
            "an unverified response must have exactly one transport audit owner"
        );
        assert_eq!(returned.audit_outcome(), "unavailable");
        assert_eq!(returned.reason_code(), "transport_outcome_unknown");
        assert!(returned.retryable());

        let mut connection = DatabaseManager::get().get_conn().unwrap();
        let row = diesel::sql_query(
            "SELECT capability, provider, request_hash, outcome, reason_code, retryable \
             FROM data_acquisition_audit WHERE capability = 'BenchmarkBarsGrpcTransport' \
             ORDER BY id DESC LIMIT 1",
        )
        .get_result::<BenchmarkTransportAuditRow>(&mut *connection)
        .expect("unknown response transport audit");
        assert_eq!(row.capability, "BenchmarkBarsGrpcTransport");
        assert_eq!(row.provider, "Custom");
        assert_eq!(row.outcome, "unavailable");
        assert_eq!(row.reason_code, "transport_outcome_unknown");
        assert_eq!(row.retryable, 1);
    }

    #[tokio::test]
    #[serial]
    async fn replayed_real_receipt_for_another_request_has_only_unknown_transport_audit() {
        let _env = super::super::grpc_source::test_grpc_env_guard();
        DatabaseManager::init(None).expect("TEST_CODE audit database init");
        let request_a_day = NaiveDate::from_ymd_opt(2026, 8, 21).expect("TEST_CODE request A date");
        let request_b_day = NaiveDate::from_ymd_opt(2026, 8, 24).expect("TEST_CODE request B date");
        let request_a = crate::data_gateway::BenchmarkRequest {
            instrument: crate::data_gateway::HS300_CANONICAL.to_owned(),
            range: crate::data_gateway::BenchmarkRange::Daily {
                from: request_a_day,
                to: request_a_day,
            },
        };
        let request_b = crate::data_gateway::BenchmarkRequest {
            instrument: crate::data_gateway::HS300_CANONICAL.to_owned(),
            range: crate::data_gateway::BenchmarkRange::Daily {
                from: request_b_day,
                to: request_b_day,
            },
        };
        let observed_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let provider_batch = GatewayBatch::Available {
            records: vec![crate::data_gateway::BenchmarkBar {
                at: crate::data_gateway::BenchmarkBarTime::Daily(request_a_day),
                open: 3_500.0,
                high: 3_510.0,
                low: 3_490.0,
                close: 3_505.0,
                volume: None,
                amount: Some(8_000.0),
            }],
            evidence: BatchEvidence {
                provider: ProviderId::Tdx,
                source: "TEST_CODE persisted request A provider".to_owned(),
                source_at: None,
                observed_at: observed_at.clone(),
                batch_id: "TEST_CODE persisted request A batch".to_owned(),
            },
        };
        let request_a_hash = super::super::benchmark::canonical_base_request_hash(&request_a);
        let (batch, receipt) = audit_gateway_result_with_receipt(
            "BenchmarkBars",
            ProviderId::Tdx,
            &request_a_hash,
            Ok(provider_batch),
        )
        .expect("TEST_CODE request A must have a real persisted receipt");
        let audited = AuditedBenchmarkBatch {
            batch,
            receipt,
            request_hash: request_a_hash,
        };
        let mut replayed = super::super::grpc_source::BenchmarkGrpcResponseWire::from_audited(
            &request_a, &audited,
        )
        .expect("TEST_CODE request A wire");
        replayed.request =
            super::super::benchmark::BenchmarkRequestWire::try_from_request(&request_b)
                .expect("TEST_CODE request B wire");
        replayed.bars[0].at = replayed.request.from.clone();

        let count = |capability: &str| {
            let mut connection = DatabaseManager::get().get_conn().unwrap();
            diesel::sql_query(
                "SELECT COUNT(*) AS count FROM data_acquisition_audit WHERE capability = ?",
            )
            .bind::<Text, _>(capability)
            .get_result::<AcquisitionCountRow>(&mut *connection)
            .unwrap()
            .count
        };
        let provider_before = count("BenchmarkBars");
        let transport_before = count("BenchmarkBarsGrpcTransport");
        let consumer_before = count("BenchmarkBarsGrpcConsumerAdmission");
        let response = crate::grpc_client::envelope::QueryResult {
            admission: crate::grpc_client::pb::magic::market::v1::AdmissionState::Admitted,
            selected_provider: "Tdx".to_owned(),
            batch_id: "TEST_CODE persisted request A batch".to_owned(),
            complete: true,
            observed_at,
            source_at: String::new(),
            records: vec![
                crate::grpc_client::pb::magic::market::v1::CanonicalPayload {
                    schema: "market.benchmark_bars".to_owned(),
                    schema_version: 1,
                    content_type: "application/json; charset=utf-8".to_owned(),
                    data: serde_json::to_vec(&replayed).expect("TEST_CODE replay payload"),
                },
            ],
            source: "TEST_CODE persisted request A provider".to_owned(),
            diagnostic_blocker: String::new(),
        };

        let result =
            super::super::grpc_source::benchmark_bars_with_test_query(&request_b, response).await;
        let returned = finish_benchmark_bridge_attempt(&request_b, result)
            .expect_err("a real receipt for request A must not authorize request B");

        assert_eq!(count("BenchmarkBars") - provider_before, 0);
        assert_eq!(count("BenchmarkBarsGrpcTransport") - transport_before, 1);
        assert_eq!(
            count("BenchmarkBarsGrpcConsumerAdmission") - consumer_before,
            0
        );
        assert_eq!(returned.audit_outcome(), "unavailable");
        assert_eq!(returned.reason_code(), "transport_outcome_unknown");
        assert!(returned.retryable());
    }

    #[tokio::test]
    #[serial]
    async fn replayed_real_receipt_precedes_bad_ohlc_for_another_request() {
        let _env = super::super::grpc_source::test_grpc_env_guard();
        DatabaseManager::init(None).expect("TEST_CODE audit database init");
        let request_a_day = NaiveDate::from_ymd_opt(2026, 8, 21).expect("TEST_CODE request A date");
        let request_b_day = NaiveDate::from_ymd_opt(2026, 8, 24).expect("TEST_CODE request B date");
        let request_a = crate::data_gateway::BenchmarkRequest {
            instrument: crate::data_gateway::HS300_CANONICAL.to_owned(),
            range: crate::data_gateway::BenchmarkRange::Daily {
                from: request_a_day,
                to: request_a_day,
            },
        };
        let request_b = crate::data_gateway::BenchmarkRequest {
            instrument: crate::data_gateway::HS300_CANONICAL.to_owned(),
            range: crate::data_gateway::BenchmarkRange::Daily {
                from: request_b_day,
                to: request_b_day,
            },
        };
        let observed_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let provider_batch = GatewayBatch::Available {
            records: vec![crate::data_gateway::BenchmarkBar {
                at: crate::data_gateway::BenchmarkBarTime::Daily(request_a_day),
                open: 3_500.0,
                high: 3_510.0,
                low: 3_490.0,
                close: 3_505.0,
                volume: None,
                amount: Some(8_000.0),
            }],
            evidence: BatchEvidence {
                provider: ProviderId::Tdx,
                source: "TEST_CODE persisted request A bad OHLC replay".to_owned(),
                source_at: None,
                observed_at: observed_at.clone(),
                batch_id: "TEST_CODE persisted request A bad OHLC batch".to_owned(),
            },
        };
        let request_a_hash = super::super::benchmark::canonical_base_request_hash(&request_a);
        let (batch, receipt) = audit_gateway_result_with_receipt(
            "BenchmarkBars",
            ProviderId::Tdx,
            &request_a_hash,
            Ok(provider_batch),
        )
        .expect("TEST_CODE request A must have a real persisted receipt");
        let audited = AuditedBenchmarkBatch {
            batch,
            receipt,
            request_hash: request_a_hash,
        };
        let mut replayed = super::super::grpc_source::BenchmarkGrpcResponseWire::from_audited(
            &request_a, &audited,
        )
        .expect("TEST_CODE request A wire");
        replayed.request =
            super::super::benchmark::BenchmarkRequestWire::try_from_request(&request_b)
                .expect("TEST_CODE request B wire");
        replayed.bars[0].at = replayed.request.from.clone();
        replayed.bars[0].close = 0.0;

        let count = |capability: &str| {
            let mut connection = DatabaseManager::get().get_conn().unwrap();
            diesel::sql_query(
                "SELECT COUNT(*) AS count FROM data_acquisition_audit WHERE capability = ?",
            )
            .bind::<Text, _>(capability)
            .get_result::<AcquisitionCountRow>(&mut *connection)
            .unwrap()
            .count
        };
        let provider_before = count("BenchmarkBars");
        let transport_before = count("BenchmarkBarsGrpcTransport");
        let consumer_before = count("BenchmarkBarsGrpcConsumerAdmission");
        let response = crate::grpc_client::envelope::QueryResult {
            admission: crate::grpc_client::pb::magic::market::v1::AdmissionState::Admitted,
            selected_provider: "Tdx".to_owned(),
            batch_id: "TEST_CODE persisted request A bad OHLC batch".to_owned(),
            complete: true,
            observed_at,
            source_at: String::new(),
            records: vec![
                crate::grpc_client::pb::magic::market::v1::CanonicalPayload {
                    schema: "market.benchmark_bars".to_owned(),
                    schema_version: 1,
                    content_type: "application/json; charset=utf-8".to_owned(),
                    data: serde_json::to_vec(&replayed).expect("TEST_CODE bad OHLC replay payload"),
                },
            ],
            source: "TEST_CODE persisted request A bad OHLC replay".to_owned(),
            diagnostic_blocker: String::new(),
        };

        let result =
            super::super::grpc_source::benchmark_bars_with_test_query(&request_b, response).await;
        let returned = finish_benchmark_bridge_attempt(&request_b, result)
            .expect_err("receipt mismatch must take precedence over request B bad OHLC");

        assert_eq!(count("BenchmarkBars") - provider_before, 0);
        assert_eq!(count("BenchmarkBarsGrpcTransport") - transport_before, 1);
        assert_eq!(
            count("BenchmarkBarsGrpcConsumerAdmission") - consumer_before,
            0
        );
        assert_eq!(returned.audit_outcome(), "unavailable");
        assert_eq!(returned.reason_code(), "transport_outcome_unknown");
        assert!(returned.retryable());
    }

    #[tokio::test]
    #[serial]
    async fn forged_benchmark_receipt_with_valid_bars_has_only_one_unknown_transport_audit() {
        let _env = super::super::grpc_source::test_grpc_env_guard();
        DatabaseManager::init(None).expect("TEST_CODE audit database init");
        let day = NaiveDate::from_ymd_opt(2026, 8, 21).expect("TEST_CODE date");
        let request = crate::data_gateway::BenchmarkRequest {
            instrument: crate::data_gateway::HS300_CANONICAL.to_owned(),
            range: crate::data_gateway::BenchmarkRange::Daily { from: day, to: day },
        };
        let count = |capability: &str| {
            let mut connection = DatabaseManager::get().get_conn().unwrap();
            diesel::sql_query(
                "SELECT COUNT(*) AS count FROM data_acquisition_audit WHERE capability = ?",
            )
            .bind::<Text, _>(capability)
            .get_result::<AcquisitionCountRow>(&mut *connection)
            .unwrap()
            .count
        };
        let provider_before = count("BenchmarkBars");
        let transport_before = count("BenchmarkBarsGrpcTransport");
        let consumer_before = count("BenchmarkBarsGrpcConsumerAdmission");
        let forged = AuditedBenchmarkBatch {
            batch: GatewayBatch::Available {
                records: vec![crate::data_gateway::BenchmarkBar {
                    at: crate::data_gateway::BenchmarkBarTime::Daily(day),
                    open: 3_500.0,
                    high: 3_510.0,
                    low: 3_490.0,
                    close: 3_505.0,
                    volume: None,
                    amount: Some(8_000.0),
                }],
                evidence: BatchEvidence {
                    provider: ProviderId::Tdx,
                    source: "TEST_CODE forged provider response".to_owned(),
                    source_at: None,
                    observed_at: "2026-08-21T15:01:00+08:00".to_owned(),
                    batch_id: "TEST_CODE forged receipt".to_owned(),
                },
            },
            receipt: crate::database::data_acquisition_audit::DataAcquisitionAuditReceipt {
                audit_id: i64::MAX,
                record_hash: "a".repeat(64),
                previous_outcome: None,
                current_outcome: "available".to_owned(),
            },
            request_hash: super::super::benchmark::canonical_base_request_hash(&request),
        };
        let wire =
            super::super::grpc_source::BenchmarkGrpcResponseWire::from_audited(&request, &forged)
                .expect("TEST_CODE structurally valid forged wire");
        let response = crate::grpc_client::envelope::QueryResult {
            admission: crate::grpc_client::pb::magic::market::v1::AdmissionState::Admitted,
            selected_provider: "Tdx".to_owned(),
            batch_id: "TEST_CODE forged receipt".to_owned(),
            complete: true,
            observed_at: "2026-08-21T15:01:00+08:00".to_owned(),
            source_at: String::new(),
            records: vec![
                crate::grpc_client::pb::magic::market::v1::CanonicalPayload {
                    schema: "market.benchmark_bars".to_owned(),
                    schema_version: 1,
                    content_type: "application/json; charset=utf-8".to_owned(),
                    data: serde_json::to_vec(&wire).expect("TEST_CODE benchmark payload"),
                },
            ],
            source: "TEST_CODE forged provider response".to_owned(),
            diagnostic_blocker: String::new(),
        };

        let result =
            super::super::grpc_source::benchmark_bars_with_test_query(&request, response).await;
        let returned = finish_benchmark_bridge_attempt(&request, result)
            .expect_err("a structurally valid but unpersisted receipt must fail closed");

        assert_eq!(count("BenchmarkBars") - provider_before, 0);
        assert_eq!(count("BenchmarkBarsGrpcTransport") - transport_before, 1);
        assert_eq!(
            count("BenchmarkBarsGrpcConsumerAdmission") - consumer_before,
            0
        );
        assert_eq!(returned.audit_outcome(), "unavailable");
        assert_eq!(returned.reason_code(), "transport_outcome_unknown");
        assert!(returned.retryable());
    }

    #[tokio::test]
    #[serial]
    async fn forged_benchmark_receipt_precedes_bad_ohlc_and_has_only_transport_audit() {
        let _env = super::super::grpc_source::test_grpc_env_guard();
        DatabaseManager::init(None).expect("TEST_CODE audit database init");
        let day = NaiveDate::from_ymd_opt(2026, 8, 21).expect("TEST_CODE date");
        let request = crate::data_gateway::BenchmarkRequest {
            instrument: crate::data_gateway::HS300_CANONICAL.to_owned(),
            range: crate::data_gateway::BenchmarkRange::Daily { from: day, to: day },
        };
        let count = |capability: &str| {
            let mut connection = DatabaseManager::get().get_conn().unwrap();
            diesel::sql_query(
                "SELECT COUNT(*) AS count FROM data_acquisition_audit WHERE capability = ?",
            )
            .bind::<Text, _>(capability)
            .get_result::<AcquisitionCountRow>(&mut *connection)
            .unwrap()
            .count
        };
        let provider_before = count("BenchmarkBars");
        let transport_before = count("BenchmarkBarsGrpcTransport");
        let consumer_before = count("BenchmarkBarsGrpcConsumerAdmission");
        let forged = AuditedBenchmarkBatch {
            batch: GatewayBatch::Available {
                records: vec![crate::data_gateway::BenchmarkBar {
                    at: crate::data_gateway::BenchmarkBarTime::Daily(day),
                    open: 3_500.0,
                    high: 3_510.0,
                    low: 3_490.0,
                    close: 0.0,
                    volume: None,
                    amount: Some(8_000.0),
                }],
                evidence: BatchEvidence {
                    provider: ProviderId::Tdx,
                    source: "TEST_CODE forged bad OHLC response".to_owned(),
                    source_at: None,
                    observed_at: "2026-08-21T15:01:00+08:00".to_owned(),
                    batch_id: "TEST_CODE forged bad OHLC receipt".to_owned(),
                },
            },
            receipt: crate::database::data_acquisition_audit::DataAcquisitionAuditReceipt {
                audit_id: i64::MAX - 1,
                record_hash: "b".repeat(64),
                previous_outcome: None,
                current_outcome: "available".to_owned(),
            },
            request_hash: super::super::benchmark::canonical_base_request_hash(&request),
        };
        let wire =
            super::super::grpc_source::BenchmarkGrpcResponseWire::from_audited(&request, &forged)
                .expect("TEST_CODE structurally valid forged wire with bad OHLC");
        let response = crate::grpc_client::envelope::QueryResult {
            admission: crate::grpc_client::pb::magic::market::v1::AdmissionState::Admitted,
            selected_provider: "Tdx".to_owned(),
            batch_id: "TEST_CODE forged bad OHLC receipt".to_owned(),
            complete: true,
            observed_at: "2026-08-21T15:01:00+08:00".to_owned(),
            source_at: String::new(),
            records: vec![
                crate::grpc_client::pb::magic::market::v1::CanonicalPayload {
                    schema: "market.benchmark_bars".to_owned(),
                    schema_version: 1,
                    content_type: "application/json; charset=utf-8".to_owned(),
                    data: serde_json::to_vec(&wire).expect("TEST_CODE benchmark payload"),
                },
            ],
            source: "TEST_CODE forged bad OHLC response".to_owned(),
            diagnostic_blocker: String::new(),
        };

        let result =
            super::super::grpc_source::benchmark_bars_with_test_query(&request, response).await;
        let returned = finish_benchmark_bridge_attempt(&request, result)
            .expect_err("receipt verification must precede client OHLC admission");

        assert_eq!(count("BenchmarkBars") - provider_before, 0);
        assert_eq!(count("BenchmarkBarsGrpcTransport") - transport_before, 1);
        assert_eq!(
            count("BenchmarkBarsGrpcConsumerAdmission") - consumer_before,
            0
        );
        assert_eq!(returned.audit_outcome(), "unavailable");
        assert_eq!(returned.reason_code(), "transport_outcome_unknown");
        assert!(returned.retryable());
    }

    #[tokio::test]
    #[serial]
    async fn verified_provider_receipt_and_client_rejection_have_separate_audits() {
        let _env = super::super::grpc_source::test_grpc_env_guard();
        DatabaseManager::init(None).expect("TEST_CODE audit database init");
        let day = NaiveDate::from_ymd_opt(2026, 8, 21).expect("TEST_CODE date");
        let request = crate::data_gateway::BenchmarkRequest {
            instrument: crate::data_gateway::HS300_CANONICAL.to_owned(),
            range: crate::data_gateway::BenchmarkRange::Daily { from: day, to: day },
        };
        let count = |capability: &str| {
            let mut connection = DatabaseManager::get().get_conn().unwrap();
            diesel::sql_query(
                "SELECT COUNT(*) AS count FROM data_acquisition_audit WHERE capability = ?",
            )
            .bind::<Text, _>(capability)
            .get_result::<AcquisitionCountRow>(&mut *connection)
            .unwrap()
            .count
        };
        let provider_before = count("BenchmarkBars");
        let consumer_before = count("BenchmarkBarsGrpcConsumerAdmission");
        let transport_before = count("BenchmarkBarsGrpcTransport");
        let provider_batch = GatewayBatch::Available {
            records: vec![crate::data_gateway::BenchmarkBar {
                at: crate::data_gateway::BenchmarkBarTime::Daily(day),
                open: 3_500.0,
                high: 3_510.0,
                low: 3_490.0,
                close: 3_505.0,
                volume: None,
                amount: Some(8_000.0),
            }],
            evidence: BatchEvidence {
                provider: ProviderId::Tdx,
                source: "TEST_CODE benchmark provider".to_owned(),
                source_at: None,
                observed_at: "2026-08-21T15:01:00+08:00".to_owned(),
                batch_id: "TEST_CODE verified provider receipt".to_owned(),
            },
        };
        let request_hash = super::super::benchmark::canonical_base_request_hash(&request);
        let (batch, receipt) = audit_gateway_result_with_receipt(
            "BenchmarkBars",
            ProviderId::Tdx,
            &request_hash,
            Ok(provider_batch),
        )
        .expect("TEST_CODE server provider audit");
        let audited = AuditedBenchmarkBatch {
            batch,
            receipt,
            request_hash,
        };
        let wire =
            super::super::grpc_source::BenchmarkGrpcResponseWire::from_audited(&request, &audited)
                .expect("TEST_CODE verified benchmark wire");
        let mut wire_json = serde_json::to_value(wire).expect("TEST_CODE benchmark JSON");
        wire_json["bars"][0]["close"] = serde_json::json!(0.0);
        let response = crate::grpc_client::envelope::QueryResult {
            admission: crate::grpc_client::pb::magic::market::v1::AdmissionState::Admitted,
            selected_provider: "Tdx".to_owned(),
            batch_id: "TEST_CODE verified provider receipt".to_owned(),
            complete: true,
            observed_at: "2026-08-21T15:01:00+08:00".to_owned(),
            source_at: String::new(),
            records: vec![
                crate::grpc_client::pb::magic::market::v1::CanonicalPayload {
                    schema: "market.benchmark_bars".to_owned(),
                    schema_version: 1,
                    content_type: "application/json; charset=utf-8".to_owned(),
                    data: serde_json::to_vec(&wire_json).expect("TEST_CODE benchmark payload"),
                },
            ],
            source: "TEST_CODE benchmark provider".to_owned(),
            diagnostic_blocker: String::new(),
        };

        let result =
            super::super::grpc_source::benchmark_bars_with_test_query(&request, response).await;
        let returned = finish_benchmark_bridge_attempt(&request, result)
            .expect_err("client OHLC admission must remain terminal");

        assert_eq!(count("BenchmarkBars") - provider_before, 1);
        assert_eq!(
            count("BenchmarkBarsGrpcConsumerAdmission") - consumer_before,
            1,
            "verified provider success must not hide the client rejection"
        );
        assert_eq!(count("BenchmarkBarsGrpcTransport") - transport_before, 0);
        assert_eq!(returned.audit_outcome(), "partial");
        assert_eq!(returned.reason_code(), "benchmark_ohlc_not_positive_finite");
        assert!(!returned.retryable());

        let mut connection = DatabaseManager::get().get_conn().unwrap();
        let row = diesel::sql_query(
            "SELECT capability, provider, request_hash, outcome, reason_code, retryable \
             FROM data_acquisition_audit \
             WHERE capability = 'BenchmarkBarsGrpcConsumerAdmission' \
             ORDER BY id DESC LIMIT 1",
        )
        .get_result::<BenchmarkTransportAuditRow>(&mut *connection)
        .expect("consumer admission audit");
        assert_eq!(row.provider, "Custom");
        assert_eq!(row.outcome, "partial");
        assert_eq!(row.reason_code, "benchmark_ohlc_not_positive_finite");
        assert_eq!(row.retryable, 0);
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
    fn br213_localbridge_admission_preserves_the_complete_limit_pool_record() {
        let date = NaiveDate::from_ymd_opt(2099, 1, 2).unwrap();
        let mut record =
            limit_pool_entry(ProviderId::Eastmoney, LimitPoolKind::Upper, "2099-01-02");
        record.volume = Some(Quantity::new(12_300.0).unwrap());
        record.turnover = Some(Ratio::new(7.5, RatioUnit::Percent).unwrap());
        record.sealed_amount = Some(Money::new(8_900_000.0).unwrap());
        record.first_seal_at = Some(NonEmptyText::new("09:31:02").unwrap());
        record.last_seal_at = Some(NonEmptyText::new("14:52:03").unwrap());
        record.break_count = Some(3);
        record.board_name = Some(NonEmptyText::new("TEST_CODE board").unwrap());
        record.seal_state = Some(NonEmptyText::new("TEST_CODE sealed").unwrap());
        record.reseal_count = Some(2);
        record.reason = Some(NonEmptyText::new("TEST_CODE reason").unwrap());
        let expected = record.clone();

        let admitted = admit_current_upper_limit_pool(
            GatewayBatch::Available {
                records: vec![record],
                evidence: evidence(ProviderId::Eastmoney, "TEST_CODE_limit_pool"),
            },
            date,
        )
        .expect("complete LocalBridge limit-pool record must be preserved");

        assert_eq!(admitted.records(), &[expected]);
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
        let _env = super::super::grpc_source::test_grpc_env_guard();
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

    #[cfg(not(feature = "magic-gateway"))]
    #[test]
    #[serial]
    fn br213_no_feature_current_pool_uses_enabled_local_bridge() {
        let _env = super::super::grpc_source::test_grpc_env_guard();
        DatabaseManager::init(None).expect("TEST_CODE audit database init");
        std::env::set_var("DATA_GATEWAY_GRPC", "1");
        std::env::remove_var("DATA_GATEWAY_GRPC_DISABLED");
        std::env::remove_var("GRPC_MARKET_CLIENT_BUNDLE");
        std::env::set_var("GRPC_MARKET_ADDR", "http://127.0.0.1:1");
        super::super::grpc_source::reset_bridge();

        let result = ReviewDataGateway::new()
            .current_upper_limit_pool(NaiveDate::from_ymd_opt(2099, 1, 2).unwrap());

        std::env::remove_var("DATA_GATEWAY_GRPC");
        std::env::remove_var("GRPC_MARKET_CLIENT_BUNDLE");
        std::env::remove_var("GRPC_MARKET_ADDR");
        super::super::grpc_source::reset_bridge();

        let error = result.expect_err("unreachable LocalBridge must fail closed");
        assert_eq!(error.capability(), "GrpcBridge");
        assert_eq!(error.reason_code(), "no_verified_batch");
        assert!(error.retryable());
        assert!(
            !error.message().contains("library transport disabled"),
            "configured bridge must be attempted: {error}"
        );

        // Independently precomputed from the BR-159 domain prefix, capability,
        // and this exact canonical request (field order is schema/profile/
        // operation/request(kind,trading_date,limit)).
        const EXPECTED_REQUEST_HASH: &str =
            "f9f0a9b2ccddff6edcc14cf0773b58f9a7fbb2db73442acb31e1b24229a4b6d7";
        let mut connection = DatabaseManager::get().get_conn().unwrap();
        let row = diesel::sql_query(
            "SELECT request_hash FROM data_acquisition_audit \
             WHERE capability = 'BR-213-UpperLimitPool' ORDER BY id DESC LIMIT 1",
        )
        .get_result::<AcquisitionRequestAuditRow>(&mut *connection)
        .expect("LocalBridge failure must retain its exact request identity in BR-159 audit");
        assert_eq!(row.request_hash, EXPECTED_REQUEST_HASH);
        assert_ne!(
            row.request_hash,
            acquisition_request_hash("BR-213-UpperLimitPool", "2099-01-02:200"),
            "legacy date:limit text does not prove route, profile, kind, or exact request fields"
        );
    }

    #[cfg(not(feature = "magic-gateway"))]
    #[test]
    #[serial]
    fn br213_no_feature_disabled_bridge_fails_without_fallback_data() {
        let _env = super::super::grpc_source::test_grpc_env_guard();
        std::env::set_var("DATA_GATEWAY_GRPC", "1");
        std::env::set_var("DATA_GATEWAY_GRPC_DISABLED", "LimitPools");
        std::env::remove_var("GRPC_MARKET_CLIENT_BUNDLE");
        super::super::grpc_source::reset_bridge();

        let result = ReviewDataGateway::new()
            .current_upper_limit_pool(NaiveDate::from_ymd_opt(2099, 1, 2).unwrap());

        std::env::remove_var("DATA_GATEWAY_GRPC");
        std::env::remove_var("DATA_GATEWAY_GRPC_DISABLED");
        super::super::grpc_source::reset_bridge();

        let error = result.expect_err("disabled bridge has no no-feature provider fallback");
        assert_eq!(error.capability(), "BR-213-UpperLimitPool");
        assert_eq!(error.reason_code(), "provider_transport");
        assert!(error.retryable());
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
