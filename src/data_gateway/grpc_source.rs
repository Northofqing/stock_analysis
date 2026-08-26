//! P4 M2: data_gateway → gRPC 桥 (双进程模式: monitor 连独立 grpc_market_server)。
//! env 契约:
//! - `DATA_GATEWAY_GRPC=1` 启用 (缺省 library 模式, 出声默认 v15.x);
//! - `GRPC_MARKET_ADDR` 服务端地址 (默认 http://127.0.0.1:18082);
//! - `DATA_GATEWAY_GRPC_DISABLED=RealtimeQuotes,HistoricalBars` 按 op 名 opt-out。
//!
//! fail-closed: 服务端不可达 / 证据链缺失 → GatewayError::unavailable
//! (retryable=true) 或 invalid_evidence, 绝不静默回退 library。
//!
//! 同步方法 (realtime_quotes/daily_bars) 在 spawn_blocking 线程里调用 →
//! Handle::block_on; 纯同步线程 → 静态 BRIDGE_RUNTIME。
pub mod convert;

// BR-251 server/client share this exact wire model without exposing the private
// benchmark acquisition module as a general construction API.
#[cfg(any(feature = "magic-gateway", test))]
pub(crate) use super::benchmark::BenchmarkGrpcResponseWire;
pub(crate) use super::benchmark::BenchmarkRequestWire;

use crate::data_gateway::market_capabilities::MarketSecurityIdentity;
use crate::data_gateway::outcome_daily_bars::{OutcomeTransportFailure, RawOutcomeFetch};
use crate::data_gateway::{
    board_ranking::BoardRankingFact, BlockTradeReview, BoardDirectoryFact, BoardFlowFact,
    BoardKind, BoardMembershipRecord, DragonTigerStockReview, EconomicReleaseFact,
    EventAnnouncement, ForeignExchangeFact, FuturesDeliveryFact, GatewayBatch, GatewayError,
    GeneralWebResearchBatch, GeneralWebResearchProvider, GlobalIndexFact, GlobalNewsProvider,
    GlobalNewsRecord, ImplementedCorporateAction, InstrumentFundFlowFact, IntradayShapeFact,
    MagicTdxT0Batch, MarketMinutePoint, MarketMoneyFlow, MarketOrderBook, MarketSecurityMetadata,
    NorthboundDailyFact, ProviderTopNFact, RealtimeIndexQuote, RealtimeMarketQuote,
    ResearchReportFact, SinaInstrumentNewsRecord, UpperLimitRecord,
};
use crate::data_provider::{consensus::ConsensusData, KlineData};
use crate::grpc_client::client::GrpcMarketClient;
use crate::grpc_client::envelope::QueryResult;
use crate::grpc_client::errors::GrpcError;
use crate::grpc_client::pb::magic::market::v1::{AdmissionState, Operation};
use crate::magic_compat::SecurityBar;
use crate::magic_compat::{
    FinancialStatement, FlowInterval, InstrumentId, MarketStatistics, NorthboundChannel,
    StatementKind,
};
use crate::magic_compat::{LimitPoolEntry, LimitPoolKind, ProviderId};
use chrono::{DateTime, FixedOffset, NaiveDate};
use prost::Message;
use serde_json::Value;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::Mutex as AsyncMutex;

/// 桥单例缓存 (l6_sink OnceLock 模式): None = 未初始化 (首次调用时连接)。
static SOURCE: OnceLock<Mutex<Option<Arc<GrpcSource>>>> = OnceLock::new();

/// 纯同步线程 (无 tokio runtime) 的 block_on 载体。
static BRIDGE_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

const DEFAULT_ADDR: &str = "http://127.0.0.1:18082";
/// BR-243: 桥同步查询总期限。原 20s — BoardConstituents 每次下载 TDX 板块文件
/// (13-15s) 加排队/重试后逼近 20s 导致边缘超时 (CANCELLED)。放宽到 35s 与
/// 客户端 tonic deadline (client.rs .timeout(35s)) 对齐; 慢查询失败语义不变
/// (fail-closed, retryable)。
const GRPC_BRIDGE_SYNC_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(35);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BenchmarkGrpcOwnership {
    ServerHandled,
    ServerAuditAppendFailed,
    ClientBeforeSend,
    OutcomeUnknown,
    ConsumerAdmissionRejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BenchmarkServerAuditState {
    Persisted,
    AppendFailed,
}

impl BenchmarkServerAuditState {
    pub(crate) const fn as_proto(self) -> i32 {
        match self {
            Self::Persisted => {
                crate::grpc_client::pb::magic::market::v1::BenchmarkAuditState::Persisted as i32
            }
            Self::AppendFailed => {
                crate::grpc_client::pb::magic::market::v1::BenchmarkAuditState::AppendFailed as i32
            }
        }
    }

    fn from_proto(value: i32) -> Option<Self> {
        match crate::grpc_client::pb::magic::market::v1::BenchmarkAuditState::try_from(value)
            .ok()?
        {
            crate::grpc_client::pb::magic::market::v1::BenchmarkAuditState::Persisted => {
                Some(Self::Persisted)
            }
            crate::grpc_client::pb::magic::market::v1::BenchmarkAuditState::AppendFailed => {
                Some(Self::AppendFailed)
            }
            crate::grpc_client::pb::magic::market::v1::BenchmarkAuditState::Unspecified => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BenchmarkServerFailureSpec {
    audit_outcome: &'static str,
    reason_code: &'static str,
    retryable: bool,
    audit_state: BenchmarkServerAuditState,
}

const fn benchmark_server_failure_spec(
    audit_outcome: &'static str,
    reason_code: &'static str,
    retryable: bool,
    audit_state: BenchmarkServerAuditState,
) -> BenchmarkServerFailureSpec {
    BenchmarkServerFailureSpec {
        audit_outcome,
        reason_code,
        retryable,
        audit_state,
    }
}

const BENCHMARK_SERVER_FAILURES: &[BenchmarkServerFailureSpec] = &[
    benchmark_server_failure_spec(
        "unsupported",
        "benchmark_instrument_unsupported",
        false,
        BenchmarkServerAuditState::Persisted,
    ),
    benchmark_server_failure_spec(
        "unsupported",
        "benchmark_test_identity_rejected",
        false,
        BenchmarkServerAuditState::Persisted,
    ),
    benchmark_server_failure_spec(
        "invalid_request",
        "benchmark_range_reversed",
        false,
        BenchmarkServerAuditState::Persisted,
    ),
    benchmark_server_failure_spec(
        "invalid_request",
        "benchmark_time_zone_invalid",
        false,
        BenchmarkServerAuditState::Persisted,
    ),
    benchmark_server_failure_spec(
        "invalid_request",
        "benchmark_minute_range_crosses_day",
        false,
        BenchmarkServerAuditState::Persisted,
    ),
    benchmark_server_failure_spec(
        "invalid_request",
        "benchmark_minute_range_off_grid",
        false,
        BenchmarkServerAuditState::Persisted,
    ),
    benchmark_server_failure_spec(
        "unavailable",
        "benchmark_batch_empty",
        true,
        BenchmarkServerAuditState::Persisted,
    ),
    benchmark_server_failure_spec(
        "unavailable",
        "benchmark_identity_unverified",
        false,
        BenchmarkServerAuditState::Persisted,
    ),
    benchmark_server_failure_spec(
        "unavailable",
        "benchmark_time_semantics_unavailable",
        false,
        BenchmarkServerAuditState::Persisted,
    ),
    benchmark_server_failure_spec(
        "unavailable",
        "benchmark_trading_calendar_unavailable",
        true,
        BenchmarkServerAuditState::Persisted,
    ),
    benchmark_server_failure_spec(
        "unavailable",
        "benchmark_trading_calendar_unavailable",
        false,
        BenchmarkServerAuditState::Persisted,
    ),
    benchmark_server_failure_spec(
        "unavailable",
        "provider_transport",
        true,
        BenchmarkServerAuditState::Persisted,
    ),
    benchmark_server_failure_spec(
        "unavailable",
        "blocking_task_failed",
        true,
        BenchmarkServerAuditState::Persisted,
    ),
    benchmark_server_failure_spec(
        "partial",
        "provider_transport",
        true,
        BenchmarkServerAuditState::Persisted,
    ),
    benchmark_server_failure_spec(
        "partial",
        "benchmark_ohlc_not_positive_finite",
        false,
        BenchmarkServerAuditState::Persisted,
    ),
    benchmark_server_failure_spec(
        "partial",
        "benchmark_ohlc_inconsistent",
        false,
        BenchmarkServerAuditState::Persisted,
    ),
    benchmark_server_failure_spec(
        "partial",
        "benchmark_turnover_not_finite_nonnegative",
        false,
        BenchmarkServerAuditState::Persisted,
    ),
    benchmark_server_failure_spec(
        "partial",
        "benchmark_bar_granularity_mismatch",
        false,
        BenchmarkServerAuditState::Persisted,
    ),
    benchmark_server_failure_spec(
        "partial",
        "benchmark_bar_outside_range",
        false,
        BenchmarkServerAuditState::Persisted,
    ),
    benchmark_server_failure_spec(
        "partial",
        "benchmark_bar_order_or_duplicate",
        false,
        BenchmarkServerAuditState::Persisted,
    ),
    benchmark_server_failure_spec(
        "partial",
        "benchmark_coverage_kind_mismatch",
        false,
        BenchmarkServerAuditState::Persisted,
    ),
    benchmark_server_failure_spec(
        "partial",
        "benchmark_authoritative_days_invalid",
        false,
        BenchmarkServerAuditState::Persisted,
    ),
    benchmark_server_failure_spec(
        "partial",
        "benchmark_daily_coverage_incomplete",
        false,
        BenchmarkServerAuditState::Persisted,
    ),
    benchmark_server_failure_spec(
        "partial",
        "benchmark_minute_bar_invalid",
        false,
        BenchmarkServerAuditState::Persisted,
    ),
    benchmark_server_failure_spec(
        "partial",
        "benchmark_minute_coverage_incomplete",
        false,
        BenchmarkServerAuditState::Persisted,
    ),
    benchmark_server_failure_spec(
        "partial",
        "benchmark_page_empty_before_range",
        true,
        BenchmarkServerAuditState::Persisted,
    ),
    benchmark_server_failure_spec(
        "partial",
        "benchmark_page_size_invalid",
        false,
        BenchmarkServerAuditState::Persisted,
    ),
    benchmark_server_failure_spec(
        "partial",
        "benchmark_page_order_or_duplicate",
        false,
        BenchmarkServerAuditState::Persisted,
    ),
    benchmark_server_failure_spec(
        "partial",
        "benchmark_page_did_not_advance",
        false,
        BenchmarkServerAuditState::Persisted,
    ),
    benchmark_server_failure_spec(
        "partial",
        "benchmark_page_boundary_did_not_advance",
        false,
        BenchmarkServerAuditState::Persisted,
    ),
    benchmark_server_failure_spec(
        "partial",
        "benchmark_short_page_before_range",
        true,
        BenchmarkServerAuditState::Persisted,
    ),
    benchmark_server_failure_spec(
        "partial",
        "benchmark_page_offset_overflow",
        false,
        BenchmarkServerAuditState::Persisted,
    ),
    benchmark_server_failure_spec(
        "partial",
        "benchmark_raw_time_invalid",
        false,
        BenchmarkServerAuditState::Persisted,
    ),
    benchmark_server_failure_spec(
        "partial",
        "benchmark_datetime_conflict",
        false,
        BenchmarkServerAuditState::Persisted,
    ),
    benchmark_server_failure_spec(
        "partial",
        "benchmark_canonical_identity_unavailable",
        false,
        BenchmarkServerAuditState::Persisted,
    ),
    benchmark_server_failure_spec(
        "unavailable",
        "acquisition_audit_unavailable",
        true,
        BenchmarkServerAuditState::AppendFailed,
    ),
];

fn exact_benchmark_server_failure(
    audit_outcome: &str,
    reason_code: &str,
    retryable: bool,
    audit_state: BenchmarkServerAuditState,
) -> Option<BenchmarkServerFailureSpec> {
    BENCHMARK_SERVER_FAILURES.iter().copied().find(|spec| {
        spec.audit_outcome == audit_outcome
            && spec.reason_code == reason_code
            && spec.retryable == retryable
            && spec.audit_state == audit_state
    })
}

pub(crate) fn benchmark_server_failure_is_exact(
    audit_outcome: &str,
    reason_code: &str,
    retryable: bool,
    audit_state: BenchmarkServerAuditState,
) -> bool {
    exact_benchmark_server_failure(audit_outcome, reason_code, retryable, audit_state).is_some()
}

#[derive(Debug, Clone)]
pub(crate) struct BenchmarkGrpcFailure {
    error: GatewayError,
    ownership: BenchmarkGrpcOwnership,
    verified_receipt:
        Option<Box<crate::database::data_acquisition_audit::DataAcquisitionAuditReceipt>>,
}

impl BenchmarkGrpcFailure {
    pub(crate) const fn client_before_send(error: GatewayError) -> Self {
        Self {
            error,
            ownership: BenchmarkGrpcOwnership::ClientBeforeSend,
            verified_receipt: None,
        }
    }

    fn server_handled(error: GatewayError) -> Self {
        Self {
            error,
            ownership: BenchmarkGrpcOwnership::ServerHandled,
            verified_receipt: None,
        }
    }

    fn server_audit_append_failed(error: GatewayError) -> Self {
        Self {
            error,
            ownership: BenchmarkGrpcOwnership::ServerAuditAppendFailed,
            verified_receipt: None,
        }
    }

    fn outcome_unknown(error: GatewayError) -> Self {
        Self {
            error,
            ownership: BenchmarkGrpcOwnership::OutcomeUnknown,
            verified_receipt: None,
        }
    }

    fn consumer_admission(
        error: GatewayError,
        receipt: crate::database::data_acquisition_audit::DataAcquisitionAuditReceipt,
    ) -> Self {
        Self {
            error,
            ownership: BenchmarkGrpcOwnership::ConsumerAdmissionRejected,
            verified_receipt: Some(Box::new(receipt)),
        }
    }

    pub(crate) const fn ownership(&self) -> BenchmarkGrpcOwnership {
        self.ownership
    }

    pub(crate) fn into_error(self) -> GatewayError {
        self.error
    }

    pub(crate) const fn has_verified_receipt(&self) -> bool {
        self.verified_receipt.is_some()
    }
}

impl std::ops::Deref for BenchmarkGrpcFailure {
    type Target = GatewayError;

    fn deref(&self) -> &Self::Target {
        &self.error
    }
}

fn benchmark_transport_error(
    audit_outcome: &'static str,
    reason_code: &'static str,
    retryable: bool,
    message: impl Into<String>,
) -> GatewayError {
    GatewayError::classified(
        "GrpcBridge",
        None,
        audit_outcome,
        reason_code,
        retryable,
        message,
    )
}

fn local_benchmark_failure(error: GatewayError) -> BenchmarkGrpcFailure {
    BenchmarkGrpcFailure::client_before_send(error)
}

fn unknown_benchmark_transport_failure() -> BenchmarkGrpcFailure {
    BenchmarkGrpcFailure::outcome_unknown(benchmark_transport_error(
        "unavailable",
        "transport_outcome_unknown",
        true,
        "BenchmarkBars RPC outcome cannot be attributed to a verified server audit",
    ))
}

fn classify_benchmark_status(request_id: &str, status: tonic::Status) -> BenchmarkGrpcFailure {
    let Ok(detail) =
        crate::grpc_client::pb::magic::market::v1::BenchmarkErrorDetail::decode(status.details())
    else {
        return unknown_benchmark_transport_failure();
    };
    let Some(error_detail) = detail.error else {
        return unknown_benchmark_transport_failure();
    };
    let Some(audit_state) = BenchmarkServerAuditState::from_proto(detail.audit_state) else {
        return unknown_benchmark_transport_failure();
    };
    let Some(spec) = exact_benchmark_server_failure(
        &detail.audit_outcome,
        &error_detail.reason_code,
        error_detail.retryable,
        audit_state,
    ) else {
        return unknown_benchmark_transport_failure();
    };
    if status.code() != tonic::Code::Internal
        || error_detail.request_id != request_id
        || error_detail.operation != Operation::BenchmarkBars as i32
        || error_detail.provider != "Tdx"
    {
        return unknown_benchmark_transport_failure();
    }
    let error = GatewayError::classified(
        "GrpcBridge",
        Some(ProviderId::Tdx),
        spec.audit_outcome,
        spec.reason_code,
        spec.retryable,
        "BenchmarkBars server returned a verified typed provider failure",
    );
    match audit_state {
        BenchmarkServerAuditState::Persisted => BenchmarkGrpcFailure::server_handled(error),
        BenchmarkServerAuditState::AppendFailed => {
            BenchmarkGrpcFailure::server_audit_append_failed(error)
        }
    }
}

#[cfg(test)]
pub(crate) fn benchmark_typed_failure_for_test(error: GatewayError) -> BenchmarkGrpcFailure {
    benchmark_typed_failure_with_state_for_test(error, BenchmarkServerAuditState::Persisted)
}

#[cfg(test)]
pub(crate) fn benchmark_typed_failure_with_state_for_test(
    error: GatewayError,
    audit_state: BenchmarkServerAuditState,
) -> BenchmarkGrpcFailure {
    let request_id = "TEST_CODE_benchmark_typed_failure";
    let provider = error
        .provider()
        .map(|provider| format!("{provider:?}"))
        // Proto3 scalar strings have no presence bit: empty means absent. This
        // TEST_CODE helper preserves None so the client fails closed to an
        // unknown transport audit; it must never fabricate provider Tdx.
        .unwrap_or_default();
    let detail = crate::grpc_client::pb::magic::market::v1::BenchmarkErrorDetail {
        error: Some(crate::grpc_client::pb::magic::market::v1::ErrorDetail {
            request_id: request_id.to_owned(),
            operation: Operation::BenchmarkBars as i32,
            provider,
            reason_code: error.reason_code().to_owned(),
            retryable: error.retryable(),
            ..Default::default()
        }),
        audit_outcome: error.audit_outcome().to_owned(),
        audit_state: audit_state.as_proto(),
    };
    classify_benchmark_status(
        request_id,
        tonic::Status::with_details(
            tonic::Code::Internal,
            "TEST_CODE typed benchmark failure",
            detail.encode_to_vec().into(),
        ),
    )
}

#[cfg(test)]
pub(crate) fn benchmark_unknown_failure_for_test() -> BenchmarkGrpcFailure {
    unknown_benchmark_transport_failure()
}

fn parse_benchmark_query_response(
    request_id: &str,
    response: crate::grpc_client::pb::magic::market::v1::QueryResponse,
) -> Result<QueryResult, BenchmarkGrpcFailure> {
    crate::grpc_client::envelope::parse_query_response(
        request_id,
        Operation::BenchmarkBars,
        response,
    )
    .map_err(|_| unknown_benchmark_transport_failure())
}

async fn benchmark_bars_with_query<F, Fut>(
    request: &crate::data_gateway::BenchmarkRequest,
    query: F,
) -> Result<crate::data_gateway::review::AuditedBenchmarkBatch, BenchmarkGrpcFailure>
where
    F: FnOnce(Value) -> Fut,
    Fut: std::future::Future<Output = Result<QueryResult, BenchmarkGrpcFailure>>,
{
    benchmark_bars_with_query_and_converter(request, query, convert::benchmark_bars).await
}

async fn benchmark_bars_with_query_for_local_readmission<F, Fut>(
    request: &crate::data_gateway::BenchmarkRequest,
    query: F,
) -> Result<crate::data_gateway::review::AuditedBenchmarkBatch, BenchmarkGrpcFailure>
where
    F: FnOnce(Value) -> Fut,
    Fut: std::future::Future<Output = Result<QueryResult, BenchmarkGrpcFailure>>,
{
    benchmark_bars_with_query_and_converter(
        request,
        query,
        convert::benchmark_bars_for_local_readmission,
    )
    .await
}

async fn benchmark_bars_with_query_and_converter<F, Fut>(
    request: &crate::data_gateway::BenchmarkRequest,
    query: F,
    converter: fn(
        &crate::data_gateway::BenchmarkRequest,
        &QueryResult,
    ) -> Result<
        crate::data_gateway::review::AuditedBenchmarkBatch,
        convert::BenchmarkGrpcConversionFailure,
    >,
) -> Result<crate::data_gateway::review::AuditedBenchmarkBatch, BenchmarkGrpcFailure>
where
    F: FnOnce(Value) -> Fut,
    Fut: std::future::Future<Output = Result<QueryResult, BenchmarkGrpcFailure>>,
{
    super::benchmark::validate_production_benchmark_request(request)
        .map_err(local_benchmark_failure)?;
    let wire = BenchmarkRequestWire::try_from_request(request).map_err(local_benchmark_failure)?;
    let params = serde_json::to_value(wire).map_err(|error| {
        local_benchmark_failure(benchmark_transport_error(
            "invalid_request",
            "benchmark_request_serialize_failed",
            false,
            format!("BenchmarkBars request serialize failed: {error}"),
        ))
    })?;
    let response = query(params).await?;
    converter(request, &response).map_err(|failure| {
        let (error, receipt) = failure.into_parts();
        match receipt {
            Some(receipt) => BenchmarkGrpcFailure::consumer_admission(error, receipt),
            None => unknown_benchmark_transport_failure(),
        }
    })
}

#[cfg(test)]
pub(crate) async fn benchmark_bars_with_test_query(
    request: &crate::data_gateway::BenchmarkRequest,
    response: QueryResult,
) -> Result<crate::data_gateway::review::AuditedBenchmarkBatch, BenchmarkGrpcFailure> {
    benchmark_bars_with_query(request, |_| std::future::ready(Ok(response))).await
}

fn production_equity_instrument(code: &str) -> Result<InstrumentId, GatewayError> {
    crate::data_gateway::instrument_identity::resolve_production_equity(code, None)
        .map(|identity| identity.instrument().clone())
        .map_err(|error| GatewayError::invalid_request("GrpcExternalV1", error.to_string()))
}

fn production_equity_instruments(codes: &[String]) -> Result<Vec<InstrumentId>, GatewayError> {
    codes
        .iter()
        .map(|code| production_equity_instrument(code))
        .collect()
}

fn limit_pools_request(trading_date: NaiveDate) -> Value {
    serde_json::json!({
        "kind": "Upper",
        "trading_date": trading_date.format("%Y-%m-%d").to_string(),
        "limit": 200,
    })
}

fn instrument_news_request_params(
    instrument: &InstrumentId,
    start: NaiveDate,
    end: NaiveDate,
    limit: u32,
    captured_at: DateTime<FixedOffset>,
) -> Value {
    serde_json::json!({
        "instrument": instrument,
        "start": start.format("%Y-%m-%d").to_string(),
        "end": end.format("%Y-%m-%d").to_string(),
        "limit": limit,
        "captured_through": captured_at.to_rfc3339(),
    })
}

/// D2: gRPC 错误 → GatewayError 分类保真映射 (query_op 共用)。
/// 服务端 Fetch 失败 (handlers.rs) 携带 ErrorDetail (provider/reason_code/retryable),
/// 客户端据此重建分类 — 不再折叠为默认 unavailable+provider=None (BR-170 pre-fix 形态)。
/// 明确错误码 (invalid_argument/unimplemented/…): 重试不会变好 → invalid_request 不重试。
fn map_query_error(op: Operation, e: &GrpcError) -> GatewayError {
    let method = crate::grpc_contract::ops::method_name(op);
    let message = format!("gRPC {method} 查询失败: {e}");
    match e {
        // 请求/权限/能力类错误码: 服务端拒绝语义 (参数错/未实现/未认证/无权限/超限/前提失败)。
        GrpcError::InvalidArgument { .. }
        | GrpcError::Unimplemented { .. }
        | GrpcError::PermissionDenied { .. }
        | GrpcError::Unauthenticated { .. }
        | GrpcError::ResourceExhausted { .. }
        | GrpcError::FailedPrecondition { .. } => {
            GatewayError::invalid_request("GrpcBridge", message)
        }
        // Fetch 失败 (Internal + ErrorDetail) 与传输类 (Unavailable/DeadlineExceeded/Unknown):
        // 从 detail 恢复 provider/reason_code/retryable, 保真重建分类。
        _ => {
            let d = e.details();
            let provider = d
                .provider
                .as_deref()
                .and_then(|s| convert::parse_provider(s).ok());
            let reason_code = d.reason_code.as_deref().unwrap_or("no_verified_batch");
            let retryable = d.retryable.unwrap_or(true);
            GatewayError::classified(
                "GrpcBridge",
                provider,
                "unavailable",
                reason_code_static(reason_code),
                retryable,
                message,
            )
        }
    }
}

fn map_external_connection_error(error: GrpcError) -> GatewayError {
    let details = error.details();
    let provider = details
        .provider
        .as_deref()
        .and_then(|value| convert::parse_provider(value).ok());
    let fixed_non_retryable = matches!(
        &error,
        GrpcError::InvalidArgument { .. }
            | GrpcError::Unauthenticated { .. }
            | GrpcError::PermissionDenied { .. }
            | GrpcError::Unimplemented { .. }
            | GrpcError::FailedPrecondition { .. }
    );
    let default_reason = match &error {
        GrpcError::InvalidArgument { .. } => "external_bundle_invalid",
        GrpcError::Unauthenticated { .. } => "external_authentication_failed",
        GrpcError::PermissionDenied { .. } => "external_permission_denied",
        GrpcError::Unimplemented { .. } | GrpcError::FailedPrecondition { .. } => {
            "external_contract_unavailable"
        }
        GrpcError::ResourceExhausted { .. }
        | GrpcError::DeadlineExceeded { .. }
        | GrpcError::Unavailable { .. } => "external_transport_unavailable",
        GrpcError::Internal { .. } | GrpcError::Unknown { .. } => "external_connection_failed",
    };
    let reason_code = details
        .reason_code
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(reason_code_static)
        .unwrap_or(default_reason);
    let retryable = if fixed_non_retryable {
        false
    } else {
        details.retryable.unwrap_or(matches!(
            &error,
            GrpcError::ResourceExhausted { .. }
                | GrpcError::DeadlineExceeded { .. }
                | GrpcError::Unavailable { .. }
        ))
    };
    let message = details
        .diagnostic_message
        .as_ref()
        .map(|diagnostic| diagnostic.as_str())
        .map(|diagnostic| {
            format!("ExternalV1 client-bundle 连接或 readiness 检查失败: {diagnostic}")
        })
        .unwrap_or_else(|| "ExternalV1 client-bundle 连接或 readiness 检查失败".to_owned());
    GatewayError::classified(
        "GrpcExternalV1",
        provider,
        "unavailable",
        reason_code,
        retryable,
        message,
    )
}

fn map_external_query_error(operation: Operation, error: &GrpcError) -> GatewayError {
    let details = error.details();
    let provider = details
        .provider
        .as_deref()
        .and_then(|value| convert::parse_provider(value).ok());
    let fixed_non_retryable = matches!(
        error,
        GrpcError::InvalidArgument { .. }
            | GrpcError::Unauthenticated { .. }
            | GrpcError::PermissionDenied { .. }
            | GrpcError::Unimplemented { .. }
    );
    let retryable = if fixed_non_retryable {
        false
    } else {
        details.retryable.unwrap_or(matches!(
            error,
            GrpcError::ResourceExhausted { .. }
                | GrpcError::DeadlineExceeded { .. }
                | GrpcError::Unavailable { .. }
        ))
    };
    let reason_code = details
        .reason_code
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(reason_code_static)
        .unwrap_or(if fixed_non_retryable {
            "external_query_rejected"
        } else {
            "external_query_unavailable"
        });
    let message = details
        .diagnostic_message
        .as_ref()
        .map(|diagnostic| diagnostic.as_str())
        .map(|diagnostic| format!("ExternalV1 {operation:?} 查询失败: {diagnostic}"))
        .unwrap_or_else(|| format!("ExternalV1 {operation:?} 查询失败"));
    GatewayError::classified(
        "GrpcExternalV1",
        provider,
        if fixed_non_retryable {
            "invalid_request"
        } else if matches!(error, GrpcError::FailedPrecondition { .. }) {
            "partial"
        } else {
            "unavailable"
        },
        reason_code,
        retryable,
        message,
    )
}

fn require_external_capability_family(
    capabilities: &[crate::grpc_client::pb::magic::market::v1::Capability],
    family: &'static str,
    operations: &[Operation],
) -> Result<(), GatewayError> {
    let matching = capabilities
        .iter()
        .filter(|capability| {
            operations
                .iter()
                .any(|operation| capability.operation == *operation as i32)
        })
        .collect::<Vec<_>>();
    if matching.iter().any(|capability| {
        capability.repository_admission == AdmissionState::Admitted as i32
            && capability.runtime_available
    }) {
        return Ok(());
    }

    let (outcome, reason_code, retryable, detail) = if matching.is_empty() {
        (
            "invalid_request",
            "external_capability_missing",
            false,
            "服务端没有发布该语义族合同",
        )
    } else if !matching
        .iter()
        .any(|capability| capability.repository_admission == AdmissionState::Admitted as i32)
    {
        (
            "invalid_request",
            "external_capability_unadmitted",
            false,
            "该语义族只有未准入或诊断能力",
        )
    } else {
        (
            "unavailable",
            "external_capability_runtime_unavailable",
            true,
            "已准入语义族的 runtime provider 暂不可用",
        )
    };
    Err(GatewayError::classified(
        "GrpcExternalV1",
        None,
        outcome,
        reason_code,
        retryable,
        format!("ExternalV1 {family}: {detail}"),
    ))
}

fn require_external_capability(
    capabilities: &[crate::grpc_client::pb::magic::market::v1::Capability],
    operation: Operation,
) -> Result<(), GatewayError> {
    require_external_capability_family(
        capabilities,
        crate::grpc_contract::ops::method_name(operation),
        std::slice::from_ref(&operation),
    )
}

const STATIC_OPENING_CAPABILITY_FAMILIES: &[(&str, &[Operation])] = &[
    ("SecurityMetadata", &[Operation::SecurityMetadata]),
    ("InstrumentNews", &[Operation::InstrumentNews]),
    ("GlobalNews", &[Operation::GlobalNews]),
    (
        "Announcements",
        &[Operation::Announcements, Operation::MarketAnnouncements],
    ),
    (
        "BoardMemberships",
        &[Operation::BoardConstituents, Operation::BoardMemberships],
    ),
    (
        "UpperLimitReview",
        &[Operation::UpperLimitPoolReview, Operation::LimitPools],
    ),
];

const LIVE_OPENING_CAPABILITY_FAMILIES: &[(&str, &[Operation])] = &[
    ("RealtimeQuotes", &[Operation::RealtimeQuotes]),
    ("OrderBooks", &[Operation::OrderBooks]),
    ("T0Evidence", &[Operation::T0Evidence]),
];

fn require_external_capability_families(
    capabilities: &[crate::grpc_client::pb::magic::market::v1::Capability],
    families: &[(&'static str, &[Operation])],
) -> Result<(), GatewayError> {
    for &(family, operations) in families {
        require_external_capability_family(capabilities, family, operations)?;
    }
    Ok(())
}

fn require_external_static_capabilities(
    capabilities: &[crate::grpc_client::pb::magic::market::v1::Capability],
) -> Result<(), GatewayError> {
    require_external_capability_families(capabilities, STATIC_OPENING_CAPABILITY_FAMILIES)
}

fn require_external_live_capabilities(
    capabilities: &[crate::grpc_client::pb::magic::market::v1::Capability],
) -> Result<(), GatewayError> {
    require_external_capability_families(capabilities, LIVE_OPENING_CAPABILITY_FAMILIES)
}

/// reason_code 需要 &'static (GatewayError 字段); wire 值来自服务端。
/// 静态表覆盖已知集合；任何未知 wire 值都归入封闭的 `internal`，既不
/// 泄露上游自由文本，也不通过 Box::leak 建立无界进程内存。
fn reason_code_static(s: &str) -> &'static str {
    const KNOWN: &[&str] = &[
        "no_verified_batch",
        "invalid_request",
        "invalid_evidence",
        "unavailable",
        "partial",
        "internal",
        "tdx_board_membership_unsupported",
        "upper_limit_streak_missing",
        "manual_confirmation_contract_unavailable",
        "five_minute_gap",
        "exact_batch_join_accepted",
        "database_failure",
        "external_source_field_conflict",
        "external_acquisition_authority_missing",
        "provider_authentication_rejected",
        "provider_rate_limited",
        "provider_unavailable",
        "external_query_rejected",
        "provider_response_invalid",
        "provider_transport",
        "blocking_task_failed",
        "acquisition_audit_unavailable",
        "benchmark_authoritative_days_invalid",
        "benchmark_bar_granularity_mismatch",
        "benchmark_bar_order_or_duplicate",
        "benchmark_bar_outside_range",
        "benchmark_batch_empty",
        "benchmark_canonical_identity_unavailable",
        "benchmark_coverage_kind_mismatch",
        "benchmark_daily_coverage_incomplete",
        "benchmark_datetime_conflict",
        "benchmark_identity_unverified",
        "benchmark_instrument_unsupported",
        "benchmark_minute_bar_invalid",
        "benchmark_minute_coverage_incomplete",
        "benchmark_minute_range_crosses_day",
        "benchmark_minute_range_off_grid",
        "benchmark_ohlc_inconsistent",
        "benchmark_ohlc_not_positive_finite",
        "benchmark_page_boundary_did_not_advance",
        "benchmark_page_did_not_advance",
        "benchmark_page_empty_before_range",
        "benchmark_page_offset_overflow",
        "benchmark_page_order_or_duplicate",
        "benchmark_page_size_invalid",
        "benchmark_range_reversed",
        "benchmark_raw_time_invalid",
        "benchmark_short_page_before_range",
        "benchmark_test_identity_rejected",
        "benchmark_time_semantics_unavailable",
        "benchmark_time_zone_invalid",
        "benchmark_trading_calendar_unavailable",
        "benchmark_turnover_not_finite_nonnegative",
        "benchmark_request_envelope_invalid",
        "benchmark_request_identity_missing",
        "benchmark_request_serialize_failed",
        "grpc_address_invalid",
        "grpc_connect_failed",
        "grpc_auth_metadata_invalid",
        "transport_outcome_unknown",
        "bridge_disabled",
    ];
    KNOWN
        .iter()
        .find(|k| **k == s)
        .copied()
        .unwrap_or("internal")
}

/// 已挂桥的 op 清单 (与各网关文件内 `super::grpc_source::bridge_for("X")` 调用
/// 一一对应)。变更时必须同步 — hooked_ops_match_bridge_for_call_sites 单测
/// 直接扫 src/data_gateway 源码断言集合相等, 防 rot (Spec Evidence Rule)。
pub const HOOKED_OPS: &[&str] = &[
    "Announcements",
    "BlockTrades",
    "BoardConstituents",
    "BoardDirectory",
    "BoardFlows",
    "BoardRanking",
    "BenchmarkBars",
    "Consensus",
    "CorporateActions",
    "DragonTiger",
    "EconomicCalendar",
    "FinancialStatements",
    "ForeignExchange",
    "FundFlowSeries",
    "FuturesDelivery",
    "GlobalIndices",
    "GlobalNews",
    "HistoricalBars",
    "IndexQuotes",
    "InstrumentNews",
    "IntradayShape",
    "LimitPools",
    "MarketStatistics",
    "MinuteData",
    "MoneyFlows",
    "NorthboundDaily",
    "OrderBooks",
    "OutcomeDailyBars",
    "ProviderTopNRankings",
    "RealtimeQuotes",
    "ResearchReports",
    "SecurityMetadata",
    "SemanticSearch",
    "T0Evidence",
    "TechnicalBars",
    "UpperLimitPoolReview",
];

/// 保持本地 (library 模式) 的网关能力 — P4 M3 风险条款: 服务端 op 已实现或
/// 半实现, 但桥保真未经验证 → 不静默切换, 出声 banner 列 follow-up。
/// 接桥时从本表删除并移入 HOOKED_OPS。
pub const KEEP_LOCAL_OPS: &[&str] = &["strong_stock_reasons"];

/// 网关钩子入口: DATA_GATEWAY_GRPC=1 且 op 未被 DISABLED → Some(Arc<GrpcSource>)
/// (惰性连接, 失败不缓存); 否则 Ok(None) (library 路径)。
/// 连接失败 → Err(unavailable retryable) (fail-closed)。
pub fn bridge_for(op: &str) -> Result<Option<Arc<GrpcSource>>, GatewayError> {
    if std::env::var("DATA_GATEWAY_GRPC").as_deref() != Ok("1") {
        return Ok(None);
    }
    let disabled = std::env::var("DATA_GATEWAY_GRPC_DISABLED").unwrap_or_default();
    if disabled.split(',').any(|name| name.trim() == op) {
        log::warn!(
            "[data_gateway] gRPC 桥: op {op} 被 DATA_GATEWAY_GRPC_DISABLED 排除, 走 library"
        );
        return Ok(None);
    }
    let cell = SOURCE.get_or_init(|| Mutex::new(None));
    if let Some(source) = cell
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
    {
        return Ok(Some(source.clone()));
    }
    // 连接是惰性的: 此处只注册桥实例 (连接在首个方法调用 ensure_connected 做,
    // 避免 block_on 在 async 上下文 panic; 失败不缓存语义在方法层保持)。
    let arc = Arc::new(GrpcSource {
        addr: std::env::var("GRPC_MARKET_ADDR").unwrap_or_else(|_| DEFAULT_ADDR.to_string()),
        client: AsyncMutex::new(None),
        external_bundle: std::env::var_os("GRPC_MARKET_CLIENT_BUNDLE")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from),
        external_client: AsyncMutex::new(None),
    });
    *cell
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(arc.clone());
    Ok(Some(arc))
}

/// 测试/重置用: 清空桥缓存 (重连)。
pub fn reset_bridge() {
    if let Some(cell) = SOURCE.get() {
        *cell
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }
}

#[cfg(test)]
static TEST_GRPC_ENV_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
pub(crate) struct TestGrpcEnvGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
    snapshot: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

#[cfg(test)]
pub(crate) fn test_grpc_env_guard() -> TestGrpcEnvGuard {
    const KEYS: &[&str] = &[
        "DATA_GATEWAY_GRPC",
        "DATA_GATEWAY_GRPC_DISABLED",
        "GRPC_MARKET_ADDR",
        "GRPC_MARKET_CLIENT_BUNDLE",
    ];
    let lock = TEST_GRPC_ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let snapshot = KEYS
        .iter()
        .map(|key| (*key, std::env::var_os(key)))
        .collect();
    reset_bridge();
    TestGrpcEnvGuard {
        _lock: lock,
        snapshot,
    }
}

#[cfg(test)]
impl Drop for TestGrpcEnvGuard {
    fn drop(&mut self) {
        reset_bridge();
        for (key, value) in &self.snapshot {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
        reset_bridge();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpeningRouteReadiness {
    pub route: &'static str,
    pub profile: &'static str,
    pub provider: ProviderId,
    pub source: String,
    pub source_at: Option<String>,
    pub observed_at: String,
    pub batch_id: String,
    pub records: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpeningRouteFailure {
    pub route: &'static str,
    pub provider: ProviderId,
    pub reason_code: &'static str,
    pub retryable: bool,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpeningReadinessReport {
    pub routes: Vec<OpeningRouteReadiness>,
    pub degraded_routes: Vec<OpeningRouteFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpeningDiagnosticFailure {
    pub route: &'static str,
    pub capability: &'static str,
    pub provider: Option<ProviderId>,
    pub audit_outcome: &'static str,
    pub reason_code: &'static str,
    pub retryable: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OpeningDiagnosticReport {
    attempts: Vec<&'static str>,
    ready_routes: Vec<OpeningRouteReadiness>,
    failures: Vec<OpeningDiagnosticFailure>,
}

impl OpeningDiagnosticReport {
    fn record_result(
        &mut self,
        route: &'static str,
        result: Result<OpeningRouteReadiness, GatewayError>,
    ) {
        match result {
            Ok(readiness) => self.record_ready(readiness),
            Err(error) => self.record_failure(route, error),
        }
    }

    fn record_ready(&mut self, route: OpeningRouteReadiness) {
        self.attempts.push(route.route);
        self.ready_routes.push(route);
    }

    fn record_failure(&mut self, route: &'static str, error: GatewayError) {
        self.attempts.push(route);
        self.failures.push(OpeningDiagnosticFailure {
            route,
            capability: error.capability(),
            provider: error.provider(),
            audit_outcome: error.audit_outcome(),
            reason_code: error.reason_code(),
            retryable: error.retryable(),
        });
    }

    pub fn attempted_route_names(&self) -> String {
        self.attempts.join(",")
    }

    pub fn failed_route_names(&self) -> String {
        if self.failures.is_empty() {
            return "none".to_owned();
        }
        self.failures
            .iter()
            .map(|failure| failure.route)
            .collect::<Vec<_>>()
            .join(",")
    }

    pub fn ready_routes(&self) -> &[OpeningRouteReadiness] {
        &self.ready_routes
    }

    pub fn failures(&self) -> &[OpeningDiagnosticFailure] {
        &self.failures
    }

    pub fn production_ready(&self) -> bool {
        let mut degraded_routes = Vec::with_capacity(self.failures.len());
        for failure in &self.failures {
            let Some(expected_provider) = expected_global_news_provider(failure.route) else {
                return false;
            };
            degraded_routes.push(OpeningRouteFailure {
                route: failure.route,
                provider: expected_provider,
                reason_code: failure.reason_code,
                retryable: failure.retryable,
                message: format!(
                    "diagnostic route failed with reason_code={}",
                    failure.reason_code
                ),
            });
        }
        require_external_static_readiness(&OpeningReadinessReport {
            routes: self.ready_routes.clone(),
            degraded_routes,
        })
        .is_ok()
    }
}

impl OpeningReadinessReport {
    pub fn route_names(&self) -> String {
        self.routes
            .iter()
            .map(|route| route.route)
            .collect::<Vec<_>>()
            .join(",")
    }

    pub fn degraded_route_names(&self) -> String {
        if self.degraded_routes.is_empty() {
            return "none".to_owned();
        }
        self.degraded_routes
            .iter()
            .map(|route| route.route)
            .collect::<Vec<_>>()
            .join(",")
    }

    pub fn global_news_routes(&self) -> usize {
        self.routes
            .iter()
            .filter(|route| route.route.starts_with("GlobalNews-"))
            .count()
    }
}

fn require_global_news_quorum(
    verified: usize,
    failures: &[OpeningRouteFailure],
) -> Result<(), GatewayError> {
    const MIN_VERIFIED_GLOBAL_NEWS_PROVIDERS: usize = 2;
    if verified >= MIN_VERIFIED_GLOBAL_NEWS_PROVIDERS {
        return Ok(());
    }
    Err(GatewayError::classified(
        "OpeningReadiness",
        None,
        "unavailable",
        "global_news_quorum_unavailable",
        failures.iter().any(|failure| failure.retryable),
        format!(
            "verified global-news provider count {verified} is below required quorum {MIN_VERIFIED_GLOBAL_NEWS_PROVIDERS}; failures={}",
            failures
                .iter()
                .map(|failure| format!(
                    "{}/{:?}/{}/{}",
                    failure.route,
                    failure.provider,
                    failure.reason_code,
                    failure.message
                ))
                .collect::<Vec<_>>()
                .join(" | ")
        ),
    ))
}

fn invalid_static_attempt_set(message: impl Into<String>) -> GatewayError {
    GatewayError::classified(
        "OpeningReadiness",
        None,
        "partial",
        "opening_static_attempt_set_invalid",
        false,
        message,
    )
}

/// BR-238 final static-gate proof. Capability aliases are discovery-only; the
/// report itself must contain each exact production route exactly once.
fn require_external_static_readiness(report: &OpeningReadinessReport) -> Result<(), GatewayError> {
    const EXPECTED_ROUTES: [&str; 9] = [
        "SecurityMetadata",
        "InstrumentNews",
        "GlobalNews-Eastmoney",
        "GlobalNews-CLS",
        "GlobalNews-Jin10",
        "GlobalNews-ThePaper",
        "Announcements",
        "BoardConstituents",
        "LimitPools",
    ];

    if report.routes.len() + report.degraded_routes.len() != EXPECTED_ROUTES.len() {
        return Err(invalid_static_attempt_set(format!(
            "opening static gate recorded {} attempts; expected {} exact routes",
            report.routes.len() + report.degraded_routes.len(),
            EXPECTED_ROUTES.len()
        )));
    }

    let mut seen = HashSet::with_capacity(EXPECTED_ROUTES.len());
    for route in &report.routes {
        if !EXPECTED_ROUTES.contains(&route.route) || !seen.insert(route.route) {
            return Err(invalid_static_attempt_set(format!(
                "opening static gate contains unexpected or duplicate route {}",
                route.route
            )));
        }
        let expected_profile = if matches!(route.route, "SecurityMetadata" | "InstrumentNews")
            || route.route.starts_with("GlobalNews-")
        {
            "ExternalV1"
        } else {
            "LocalBridgeV1"
        };
        if route.profile != expected_profile {
            return Err(invalid_static_attempt_set(format!(
                "opening route {} used profile {} instead of {}",
                route.route, route.profile, expected_profile
            )));
        }
        if let Some(expected_provider) = expected_global_news_provider(route.route) {
            if route.provider != expected_provider {
                return Err(invalid_static_attempt_set(format!(
                    "opening route {} used the wrong provider identity",
                    route.route
                )));
            }
        }
    }
    for failure in &report.degraded_routes {
        let Some(expected_provider) = expected_global_news_provider(failure.route) else {
            return Err(invalid_static_attempt_set(format!(
                "mandatory non-news route {} was not verified",
                failure.route
            )));
        };
        if failure.provider != expected_provider || !seen.insert(failure.route) {
            return Err(invalid_static_attempt_set(format!(
                "opening degraded route {} has conflicting identity",
                failure.route
            )));
        }
    }

    if EXPECTED_ROUTES.iter().any(|route| !seen.contains(route))
        || !report
            .routes
            .iter()
            .any(|route| route.route == "LimitPools")
    {
        return Err(invalid_static_attempt_set(
            "opening static gate did not verify every exact mandatory route",
        ));
    }
    require_global_news_quorum(report.global_news_routes(), &report.degraded_routes)
}

fn expected_global_news_provider(route: &str) -> Option<ProviderId> {
    expected_global_news_route_provider(route).map(GlobalNewsProvider::provider_id)
}

fn expected_global_news_route_provider(route: &str) -> Option<GlobalNewsProvider> {
    match route {
        "GlobalNews-Eastmoney" => Some(GlobalNewsProvider::Eastmoney),
        "GlobalNews-CLS" => Some(GlobalNewsProvider::Cailianpress),
        "GlobalNews-Jin10" => Some(GlobalNewsProvider::Jin10),
        "GlobalNews-ThePaper" => Some(GlobalNewsProvider::ThePaper),
        _ => None,
    }
}

/// Persist one BR-159 hash-chained row per admitted readiness route. The
/// monitor calls this before publishing a ready banner; the read-only bundle
/// probe intentionally does not mutate production audit state.
pub fn audit_opening_readiness_report(
    phase: &'static str,
    report: &OpeningReadinessReport,
) -> Result<(), GatewayError> {
    use crate::database::data_acquisition_audit::DataAcquisitionAuditRecord;
    use crate::database::DatabaseManager;

    let database = DatabaseManager::try_get().ok_or_else(|| {
        GatewayError::audit_failure(
            "OpeningReadiness",
            ProviderId::Custom,
            "database_unavailable",
            "opening readiness audit database is not initialized",
        )
    })?;
    for route in &report.routes {
        let capability = format!("{phase}-{}", route.route);
        let request_hash = crate::data_gateway::review::acquisition_request_hash(
            &capability,
            format!("{}:{}", route.profile, route.route),
        );
        let provider = format!("{:?}", route.provider);
        let accepted_count = i64::try_from(route.records).map_err(|_| {
            GatewayError::audit_failure(
                "OpeningReadiness",
                route.provider,
                "accepted_count_overflow",
                "opening readiness record count exceeds SQLite INTEGER",
            )
        })?;
        let outcome = if route.records == 0 {
            "verified_empty"
        } else {
            "available"
        };
        let reason_code = if route.records == 0 {
            "verified_empty"
        } else {
            "accepted"
        };
        let record = DataAcquisitionAuditRecord {
            capability: &capability,
            provider: &provider,
            source: &route.source,
            request_hash: &request_hash,
            source_at: route.source_at.as_deref(),
            observed_at: &route.observed_at,
            batch_id: Some(&route.batch_id),
            outcome,
            request_count: 1,
            accepted_count,
            rejected_count: 0,
            reason_code,
            retryable: false,
        };
        let receipt = database.record_data_acquisition(&record).map_err(|error| {
            GatewayError::audit_failure("OpeningReadiness", route.provider, reason_code, error)
        })?;
        log::info!(
            "[OpeningReadiness][BR-159][BR-238] phase={phase} route={} audit_id={} record_hash={}",
            route.route,
            receipt.audit_id,
            receipt.record_hash
        );
    }
    Ok(())
}

#[derive(Debug)]
struct OpeningDiagnosticAuditRow {
    route: &'static str,
    provider_id: ProviderId,
    capability: String,
    provider: String,
    source: String,
    request_hash: String,
    source_at: Option<String>,
    observed_at: String,
    batch_id: Option<String>,
    outcome: &'static str,
    accepted_count: i64,
    rejected_count: i64,
    reason_code: &'static str,
    retryable: bool,
}

fn invalid_opening_diagnostic_report(message: impl Into<String>) -> GatewayError {
    GatewayError::invalid_evidence("OpeningReadiness", Some(ProviderId::Custom), message.into())
}

fn opening_diagnostic_audit_rows(
    phase: &'static str,
    report: &OpeningDiagnosticReport,
) -> Result<Vec<OpeningDiagnosticAuditRow>, GatewayError> {
    let failure_observed_at =
        chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let mut seen_routes = std::collections::HashSet::with_capacity(report.attempts.len());
    let mut rows = Vec::with_capacity(report.attempts.len());

    for route_name in &report.attempts {
        if !seen_routes.insert(*route_name) {
            return Err(invalid_opening_diagnostic_report(format!(
                "opening diagnostic report repeats route {route_name}"
            )));
        }
        let ready = report
            .ready_routes
            .iter()
            .filter(|route| route.route == *route_name)
            .collect::<Vec<_>>();
        let failed = report
            .failures
            .iter()
            .filter(|failure| failure.route == *route_name)
            .collect::<Vec<_>>();
        match (ready.as_slice(), failed.as_slice()) {
            ([route], []) => {
                let accepted_count = i64::try_from(route.records).map_err(|_| {
                    GatewayError::audit_failure(
                        "OpeningReadiness",
                        route.provider,
                        "accepted_count_overflow",
                        "opening readiness record count exceeds SQLite INTEGER",
                    )
                })?;
                let capability = format!("{phase}-{}", route.route);
                let request_hash = crate::data_gateway::review::acquisition_request_hash(
                    &capability,
                    format!("{}:{}", route.profile, route.route),
                );
                let (outcome, reason_code) = if route.records == 0 {
                    ("verified_empty", "verified_empty")
                } else {
                    ("available", "accepted")
                };
                rows.push(OpeningDiagnosticAuditRow {
                    route: route.route,
                    provider_id: route.provider,
                    capability,
                    provider: format!("{:?}", route.provider),
                    source: route.source.clone(),
                    request_hash,
                    source_at: route.source_at.clone(),
                    observed_at: route.observed_at.clone(),
                    batch_id: Some(route.batch_id.clone()),
                    outcome,
                    accepted_count,
                    rejected_count: 0,
                    reason_code,
                    retryable: false,
                });
            }
            ([], [failure]) => {
                let provider_id = failure.provider.unwrap_or(ProviderId::Custom);
                let capability = format!("{phase}-{}", failure.route);
                let request_hash = crate::data_gateway::review::acquisition_request_hash(
                    &capability,
                    format!(
                        "{}:{}:{}",
                        failure.capability, failure.audit_outcome, failure.reason_code
                    ),
                );
                rows.push(OpeningDiagnosticAuditRow {
                    route: failure.route,
                    provider_id,
                    capability,
                    provider: format!("{provider_id:?}"),
                    source: "opening-readiness".to_owned(),
                    request_hash,
                    source_at: None,
                    observed_at: failure_observed_at.clone(),
                    batch_id: None,
                    outcome: failure.audit_outcome,
                    accepted_count: 0,
                    rejected_count: 1,
                    reason_code: failure.reason_code,
                    retryable: failure.retryable,
                });
            }
            _ => {
                return Err(invalid_opening_diagnostic_report(format!(
                    "opening diagnostic route {route_name} must have exactly one terminal outcome"
                )));
            }
        }
    }

    if rows.len() != report.ready_routes.len() + report.failures.len() {
        return Err(invalid_opening_diagnostic_report(
            "opening diagnostic report contains a terminal outcome without an attempt",
        ));
    }
    Ok(rows)
}

/// Persist one BR-159 hash-chained row for every static diagnostic attempt.
/// Unlike the strict release probe, this records partial readiness so the
/// resident monitor can remain alive without disguising a failed route.
pub fn audit_opening_diagnostic_report(
    phase: &'static str,
    report: &OpeningDiagnosticReport,
) -> Result<(), GatewayError> {
    use crate::database::data_acquisition_audit::DataAcquisitionAuditRecord;
    use crate::database::DatabaseManager;

    let rows = opening_diagnostic_audit_rows(phase, report)?;
    let database = DatabaseManager::try_get().ok_or_else(|| {
        GatewayError::audit_failure(
            "OpeningReadiness",
            ProviderId::Custom,
            "database_unavailable",
            "opening diagnostic audit database is not initialized",
        )
    })?;
    for row in rows {
        let record = DataAcquisitionAuditRecord {
            capability: &row.capability,
            provider: &row.provider,
            source: &row.source,
            request_hash: &row.request_hash,
            source_at: row.source_at.as_deref(),
            observed_at: &row.observed_at,
            batch_id: row.batch_id.as_deref(),
            outcome: row.outcome,
            request_count: 1,
            accepted_count: row.accepted_count,
            rejected_count: row.rejected_count,
            reason_code: row.reason_code,
            retryable: row.retryable,
        };
        let receipt = database.record_data_acquisition(&record).map_err(|error| {
            GatewayError::audit_failure("OpeningReadiness", row.provider_id, row.reason_code, error)
        })?;
        log::info!(
            "[OpeningReadiness][BR-159][BR-238][BR-246] phase={phase} route={} audit_id={} record_hash={}",
            row.route,
            receipt.audit_id,
            receipt.record_hash
        );
    }
    Ok(())
}

pub fn audit_opening_readiness_failure(
    phase: &'static str,
    error: &GatewayError,
) -> Result<(), GatewayError> {
    use crate::database::data_acquisition_audit::DataAcquisitionAuditRecord;
    use crate::database::DatabaseManager;

    let provider_id = error.provider().unwrap_or(ProviderId::Custom);
    let database = DatabaseManager::try_get().ok_or_else(|| {
        GatewayError::audit_failure(
            "OpeningReadiness",
            provider_id,
            error.reason_code(),
            "opening readiness audit database is not initialized",
        )
    })?;
    let capability = format!("{phase}-{}", error.capability());
    let request_hash = crate::data_gateway::review::acquisition_request_hash(
        &capability,
        format!("{}:{}", error.capability(), error.reason_code()),
    );
    let provider = format!("{provider_id:?}");
    let observed_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let record = DataAcquisitionAuditRecord {
        capability: &capability,
        provider: &provider,
        source: "opening-readiness",
        request_hash: &request_hash,
        source_at: None,
        observed_at: &observed_at,
        batch_id: None,
        outcome: error.audit_outcome(),
        request_count: 1,
        accepted_count: 0,
        rejected_count: 1,
        reason_code: error.reason_code(),
        retryable: error.retryable(),
    };
    let receipt = database
        .record_data_acquisition(&record)
        .map_err(|audit_error| {
            GatewayError::audit_failure(
                "OpeningReadiness",
                provider_id,
                error.reason_code(),
                audit_error,
            )
        })?;
    log::info!(
        "[OpeningReadiness][BR-159][BR-238] phase={phase} failure={} audit_id={} record_hash={}",
        error.reason_code(),
        receipt.audit_id,
        receipt.record_hash
    );
    Ok(())
}

fn required_opening_bridge(op_name: &'static str) -> Result<Arc<GrpcSource>, GatewayError> {
    bridge_for(op_name)?.ok_or_else(|| {
        GatewayError::classified(
            "OpeningReadiness",
            None,
            "unavailable",
            "external_bridge_disabled",
            false,
            format!("开盘 readiness 要求 DATA_GATEWAY_GRPC=1 且 {op_name} 未被禁用"),
        )
    })
}

async fn current_external_opening_capabilities(
    source: &GrpcSource,
) -> Result<Vec<crate::grpc_client::pb::magic::market::v1::Capability>, GatewayError> {
    let mut guard = source.external_client.lock().await;
    let state = guard
        .as_mut()
        .expect("ensure_external_connected 后必有 external client");
    let health = state
        .client
        .get_health()
        .await
        .map_err(map_external_connection_error)?;
    if !health.live || !health.ready {
        return Err(GatewayError::classified(
            "GrpcExternalV1",
            None,
            "unavailable",
            "external_health_not_ready",
            true,
            "ExternalV1 current health 未达到 live+ready",
        ));
    }
    state
        .client
        .get_capabilities()
        .await
        .map_err(map_external_connection_error)
}

fn opening_route<T>(
    route: &'static str,
    profile: &'static str,
    batch: &GatewayBatch<T>,
    require_records: bool,
) -> Result<OpeningRouteReadiness, GatewayError> {
    let evidence = batch.evidence();
    if require_records && batch.records().is_empty() {
        return Err(GatewayError::classified(
            "OpeningReadiness",
            Some(evidence.provider),
            "unavailable",
            "opening_canary_empty",
            true,
            format!("{route} canary returned verified-empty"),
        ));
    }
    if evidence.source.trim().is_empty() || evidence.batch_id.trim().is_empty() {
        return Err(GatewayError::classified(
            "OpeningReadiness",
            Some(evidence.provider),
            "partial",
            "invalid_evidence",
            false,
            format!("{route} canary evidence is incomplete"),
        ));
    }
    let global_news_provider = expected_global_news_route_provider(route);
    if global_news_provider.is_some_and(|provider| provider.provider_id() != evidence.provider) {
        return Err(GatewayError::invalid_evidence(
            "OpeningReadiness",
            Some(evidence.provider),
            format!("{route} canary provider differs from its exact route"),
        ));
    }
    let observed_at = if let Some(provider) = global_news_provider {
        crate::data_gateway::global_news::parse_global_news_observed_at(
            provider,
            &evidence.observed_at,
        )?
    } else {
        crate::data_gateway::parse_evidence_instant(
            "OpeningReadiness",
            evidence.provider,
            "observed_at",
            &evidence.observed_at,
        )?
    };
    if let Some(source_at) = evidence.source_at.as_deref() {
        let source_at = if let Some(provider) = global_news_provider {
            crate::data_gateway::global_news::parse_global_news_provider_time(provider, source_at)?
        } else {
            crate::data_gateway::parse_evidence_instant(
                "OpeningReadiness",
                evidence.provider,
                "source_at",
                source_at,
            )?
        };
        if source_at > observed_at {
            return Err(GatewayError::invalid_evidence(
                "OpeningReadiness",
                Some(evidence.provider),
                format!("{route} canary source_at is later than observed_at"),
            ));
        }
    }
    Ok(OpeningRouteReadiness {
        route,
        profile,
        provider: evidence.provider,
        source: evidence.source.clone(),
        source_at: evidence.source_at.clone(),
        observed_at: evidence.observed_at.clone(),
        batch_id: evidence.batch_id.clone(),
        records: batch.records().len(),
    })
}

fn opening_limit_pools_route(
    batch: &GatewayBatch<LimitPoolEntry>,
    requested_date: NaiveDate,
) -> Result<OpeningRouteReadiness, GatewayError> {
    const ROUTE: &str = "LimitPools";
    let evidence = batch.evidence();
    if evidence.source.trim().is_empty() || evidence.batch_id.trim().is_empty() {
        return Err(GatewayError::invalid_evidence(
            "OpeningReadiness",
            Some(evidence.provider),
            format!("{ROUTE} canary evidence is incomplete"),
        ));
    }
    let observed_at = crate::data_gateway::parse_evidence_instant(
        "OpeningReadiness",
        evidence.provider,
        "observed_at",
        &evidence.observed_at,
    )?;
    let source_at = evidence.source_at.as_deref().ok_or_else(|| {
        GatewayError::invalid_evidence(
            "OpeningReadiness",
            Some(evidence.provider),
            format!("{ROUTE} daily source_at is missing"),
        )
    })?;
    let source_date = NaiveDate::parse_from_str(source_at, "%Y-%m-%d").map_err(|error| {
        GatewayError::invalid_evidence(
            "OpeningReadiness",
            Some(evidence.provider),
            format!("{ROUTE} source_at is not YYYY-MM-DD: {error}"),
        )
    })?;
    let shanghai = chrono::FixedOffset::east_opt(8 * 60 * 60).expect("valid Shanghai offset");
    let expected_date = requested_date.format("%Y-%m-%d").to_string();
    if source_date != requested_date
        || source_date > observed_at.with_timezone(&shanghai).date_naive()
        || batch.records().iter().any(|record| {
            record.kind != LimitPoolKind::Upper
                || record.trading_date.as_str() != expected_date.as_str()
        })
    {
        return Err(GatewayError::invalid_evidence(
            "OpeningReadiness",
            Some(evidence.provider),
            format!("{ROUTE} evidence does not match exact Upper pool date {requested_date}"),
        ));
    }
    Ok(OpeningRouteReadiness {
        route: ROUTE,
        profile: "LocalBridgeV1",
        provider: evidence.provider,
        source: evidence.source.clone(),
        source_at: evidence.source_at.clone(),
        observed_at: evidence.observed_at.clone(),
        batch_id: evidence.batch_id.clone(),
        records: batch.records().len(),
    })
}

fn exact_quote_canary(
    requested_code: &str,
    batch: &GatewayBatch<RealtimeMarketQuote>,
) -> Result<(), GatewayError> {
    if batch.records().len() != 1 || batch.records()[0].code != requested_code {
        return Err(GatewayError::classified(
            "OpeningReadiness",
            Some(batch.evidence().provider),
            "partial",
            "opening_canary_identity_mismatch",
            false,
            "RealtimeQuotes canary did not return the exact requested instrument",
        ));
    }
    Ok(())
}

fn exact_book_canary(
    requested_code: &str,
    batch: &GatewayBatch<MarketOrderBook>,
) -> Result<(), GatewayError> {
    if batch.records().len() != 1 || batch.records()[0].code != requested_code {
        return Err(GatewayError::classified(
            "OpeningReadiness",
            Some(batch.evidence().provider),
            "partial",
            "opening_canary_identity_mismatch",
            false,
            "OrderBooks canary did not return the exact requested instrument",
        ));
    }
    Ok(())
}

fn exact_membership_canary(
    requested_code: &str,
    batch: &GatewayBatch<BoardMembershipRecord>,
) -> Result<(), GatewayError> {
    if batch.records().is_empty()
        || batch
            .records()
            .iter()
            .any(|record| record.instrument_code != requested_code)
    {
        return Err(GatewayError::classified(
            "OpeningReadiness",
            Some(batch.evidence().provider),
            "partial",
            "opening_canary_identity_mismatch",
            false,
            "BoardConstituents canary did not return exact requested memberships",
        ));
    }
    Ok(())
}

fn exact_t0_canary(requested_code: &str, batch: &MagicTdxT0Batch) -> Result<(), GatewayError> {
    if !batch.rejections.is_empty()
        || batch.records.len() != 1
        || batch.records[0].code != requested_code
    {
        return Err(GatewayError::classified(
            "OpeningReadiness",
            Some(batch.provider),
            "partial",
            "opening_canary_identity_mismatch",
            false,
            "T0Evidence canary did not return one exact admitted instrument",
        ));
    }
    Ok(())
}

fn require_valid_requested_codes(
    capability: &'static str,
    requested: &[String],
) -> Result<(), GatewayError> {
    let unique = requested.iter().map(String::as_str).collect::<HashSet<_>>();
    if requested.is_empty() || unique.len() != requested.len() {
        return Err(GatewayError::invalid_request(
            capability,
            "live request requires a non-empty unique instrument set",
        ));
    }
    Ok(())
}

fn require_exact_response_codes(
    capability: &'static str,
    requested: &[String],
    returned: &[String],
    provider: ProviderId,
) -> Result<(), GatewayError> {
    let requested_set = requested.iter().map(String::as_str).collect::<HashSet<_>>();
    let returned_set = returned.iter().map(String::as_str).collect::<HashSet<_>>();
    if returned_set.len() != returned.len() || returned_set != requested_set {
        return Err(GatewayError::invalid_evidence(
            capability,
            Some(provider),
            "live response identities do not exactly match the requested instrument set",
        ));
    }
    Ok(())
}

fn opening_t0_route(batch: &MagicTdxT0Batch) -> Result<OpeningRouteReadiness, GatewayError> {
    if batch.source.trim().is_empty() || batch.batch_id.trim().is_empty() {
        return Err(GatewayError::invalid_evidence(
            "OpeningReadiness",
            Some(batch.provider),
            "T0Evidence canary evidence is incomplete",
        ));
    }
    Ok(OpeningRouteReadiness {
        route: "T0Evidence",
        profile: "LocalBridgeV1",
        provider: batch.provider,
        source: batch.source.clone(),
        source_at: Some(batch.source_at.to_rfc3339()),
        observed_at: batch.observed_at.to_rfc3339(),
        batch_id: batch.batch_id.clone(),
        records: batch.records.len(),
    })
}

/// BR-238 static opening gate. Capabilities are discovery evidence only; the
/// nine static/auth/contract routes are exercised before producer loops start.
/// Live quote/book/T0 evidence is deliberately excluded and observed later by
/// the background live gate so it cannot suppress the 09:00--09:15 P-01 window.
pub async fn external_static_opening_diagnostics() -> Result<OpeningDiagnosticReport, GatewayError>
{
    const CANARY_CODE: &str = "600396";
    let source = required_opening_bridge("SecurityMetadata")?;
    source
        .ensure_external_connected(Operation::SecurityMetadata)
        .await?;
    let capabilities = current_external_opening_capabilities(&source).await?;
    require_external_static_capabilities(&capabilities)?;

    let code = CANARY_CODE.to_owned();
    let mut report = OpeningDiagnosticReport::default();

    let identities = source
        .security_identities_async(std::slice::from_ref(&code))
        .await
        .and_then(|batch| opening_route("SecurityMetadata", "ExternalV1", &batch, true));
    report.record_result("SecurityMetadata", identities);

    let instrument_news = match required_opening_bridge("InstrumentNews") {
        Ok(bridge) => bridge
            .instrument_news_canary_async(CANARY_CODE)
            .await
            .and_then(|batch| opening_route("InstrumentNews", "ExternalV1", &batch, false)),
        Err(error) => Err(error),
    };
    report.record_result("InstrumentNews", instrument_news);

    for (route, provider) in [
        ("GlobalNews-Eastmoney", GlobalNewsProvider::Eastmoney),
        ("GlobalNews-CLS", GlobalNewsProvider::Cailianpress),
        ("GlobalNews-Jin10", GlobalNewsProvider::Jin10),
        ("GlobalNews-ThePaper", GlobalNewsProvider::ThePaper),
    ] {
        let result = match required_opening_bridge("GlobalNews") {
            Ok(bridge) => bridge
                .global_news_async(provider, 1)
                .await
                .and_then(|batch| opening_route(route, "ExternalV1", &batch, false)),
            Err(error) => Err(error),
        };
        report.record_result(route, result);
    }

    let announcements = match required_opening_bridge("Announcements") {
        Ok(bridge) => bridge
            .announcements_async()
            .await
            .and_then(|batch| opening_route("Announcements", "LocalBridgeV1", &batch, false)),
        Err(error) => Err(error),
    };
    report.record_result("Announcements", announcements);

    let memberships = match required_opening_bridge("BoardConstituents") {
        Ok(bridge) => match bridge.board_constituents_async(CANARY_CODE).await {
            Ok(batch) => exact_membership_canary(CANARY_CODE, &batch)
                .and_then(|()| opening_route("BoardConstituents", "LocalBridgeV1", &batch, true)),
            Err(error) => Err(error),
        },
        Err(error) => Err(error),
    };
    report.record_result("BoardConstituents", memberships);

    let review_date = crate::calendar::prev_trading_day(chrono::Local::now().date_naive());
    let limit_pools = match required_opening_bridge("LimitPools") {
        Ok(bridge) => bridge
            .limit_pools_async(review_date)
            .await
            .and_then(|batch| opening_limit_pools_route(&batch, review_date)),
        Err(error) => Err(error),
    };
    report.record_result("LimitPools", limit_pools);

    Ok(report)
}

/// BR-238 production static opening gate. Unlike the read-only diagnostics
/// collector above, this remains fail-closed at each mandatory route.
pub async fn external_static_opening_readiness() -> Result<OpeningReadinessReport, GatewayError> {
    const CANARY_CODE: &str = "600396";
    let source = required_opening_bridge("SecurityMetadata")?;
    source
        .ensure_external_connected(Operation::SecurityMetadata)
        .await?;
    let capabilities = current_external_opening_capabilities(&source).await?;
    require_external_static_capabilities(&capabilities)?;

    let code = CANARY_CODE.to_string();
    let mut routes = Vec::with_capacity(9);
    let mut degraded_routes = Vec::with_capacity(2);

    let identities = source
        .security_identities_async(std::slice::from_ref(&code))
        .await?;
    routes.push(opening_route(
        "SecurityMetadata",
        "ExternalV1",
        &identities,
        true,
    )?);

    let instrument_news = required_opening_bridge("InstrumentNews")?
        .instrument_news_canary_async(CANARY_CODE)
        .await?;
    routes.push(opening_route(
        "InstrumentNews",
        "ExternalV1",
        &instrument_news,
        false,
    )?);

    let global_news_bridge = required_opening_bridge("GlobalNews")?;
    for (route, provider) in [
        ("GlobalNews-Eastmoney", GlobalNewsProvider::Eastmoney),
        ("GlobalNews-CLS", GlobalNewsProvider::Cailianpress),
        ("GlobalNews-Jin10", GlobalNewsProvider::Jin10),
        ("GlobalNews-ThePaper", GlobalNewsProvider::ThePaper),
    ] {
        let result = global_news_bridge
            .global_news_async(provider, 1)
            .await
            .and_then(|batch| opening_route(route, "ExternalV1", &batch, false));
        match result {
            Ok(readiness) => routes.push(readiness),
            Err(error) => {
                log::warn!(
                    "[BR-238] opening GlobalNews route degraded route={} provider={:?} reason_code={} retryable={}",
                    route,
                    provider.provider_id(),
                    error.reason_code(),
                    error.retryable()
                );
                degraded_routes.push(OpeningRouteFailure {
                    route,
                    provider: provider.provider_id(),
                    reason_code: error.reason_code(),
                    retryable: error.retryable(),
                    message: error.message().to_owned(),
                });
            }
        }
    }
    let verified_global_news = 4_usize.saturating_sub(degraded_routes.len());
    require_global_news_quorum(verified_global_news, &degraded_routes)?;

    let announcements = required_opening_bridge("Announcements")?
        .announcements_async()
        .await?;
    routes.push(opening_route(
        "Announcements",
        "LocalBridgeV1",
        &announcements,
        false,
    )?);

    let memberships = required_opening_bridge("BoardConstituents")?
        .board_constituents_async(CANARY_CODE)
        .await?;
    exact_membership_canary(CANARY_CODE, &memberships)?;
    routes.push(opening_route(
        "BoardConstituents",
        "LocalBridgeV1",
        &memberships,
        true,
    )?);

    let review_date = crate::calendar::prev_trading_day(chrono::Local::now().date_naive());
    let limit_pools = required_opening_bridge("LimitPools")?
        .limit_pools_async(review_date)
        .await?;
    routes.push(opening_limit_pools_route(&limit_pools, review_date)?);

    let report = OpeningReadinessReport {
        routes,
        degraded_routes,
    };
    require_external_static_readiness(&report)?;
    Ok(report)
}

/// BR-238 live-session observation. This is not a transferable startup permit:
/// each method below repeats the exact consumer-clock gate after its own RPC.
pub async fn external_live_opening_readiness() -> Result<OpeningReadinessReport, GatewayError> {
    const CANARY_CODE: &str = "600396";
    let source = required_opening_bridge("RealtimeQuotes")?;
    source
        .ensure_external_connected(Operation::RealtimeQuotes)
        .await?;
    let capabilities = current_external_opening_capabilities(&source).await?;
    require_external_live_capabilities(&capabilities)?;

    let code = CANARY_CODE.to_string();
    let mut routes = Vec::with_capacity(3);

    let quotes = source
        .realtime_quotes_async(std::slice::from_ref(&code))
        .await?;
    exact_quote_canary(CANARY_CODE, &quotes)?;
    routes.push(opening_route(
        "RealtimeQuotes",
        "LocalBridgeV1",
        &quotes,
        true,
    )?);

    let books = required_opening_bridge("OrderBooks")?
        .order_books_async(std::slice::from_ref(&code))
        .await?;
    exact_book_canary(CANARY_CODE, &books)?;
    routes.push(opening_route("OrderBooks", "LocalBridgeV1", &books, true)?);

    let t0 = required_opening_bridge("T0Evidence")?
        .t0_evidence_batch_async(std::slice::from_ref(&code))
        .await?;
    exact_t0_canary(CANARY_CODE, &t0)?;
    routes.push(opening_t0_route(&t0)?);

    Ok(OpeningReadinessReport {
        routes,
        degraded_routes: Vec::new(),
    })
}

/// M4 启动 banner (v15.x 出声原则): 数据源模式必须打印, 默认 library。
/// 语义与 bridge_for 完全一致 (DATA_GATEWAY_GRPC=1 才走 gRPC)。
/// main.rs [broker] 启动完成后调用。
pub fn startup_banner() -> String {
    let mode = if std::env::var("DATA_GATEWAY_GRPC").as_deref() == Ok("1") {
        "grpc"
    } else {
        "library"
    };
    let server = std::env::var("GRPC_MARKET_ADDR").unwrap_or_else(|_| DEFAULT_ADDR.to_string());
    let disabled = std::env::var("DATA_GATEWAY_GRPC_DISABLED").unwrap_or_default();
    let disabled = if disabled.is_empty() {
        "无".to_string()
    } else {
        disabled
    };
    let external =
        if std::env::var_os("GRPC_MARKET_CLIENT_BUNDLE").is_some_and(|value| !value.is_empty()) {
            "configured"
        } else {
            "unconfigured"
        };
    format!(
        "[data_gateway] 数据源模式 = {mode} | server = {server} | external-v1 = {external} | 桥接 {} ops | \
         禁用 = {disabled} | 保持本地 {} ops: {} \
         (P-01: LimitPools 已桥接完整 LimitPoolEntry; strong_stock_reasons 的 op 45 \
          扁平视图不消费, monitor 复盘经 chain_batch op 61 拿完整 VisibleChainBatch)",
        HOOKED_OPS.len(),
        KEEP_LOCAL_OPS.len(),
        KEEP_LOCAL_OPS.join(",")
    )
}

fn bridge_runtime() -> &'static tokio::runtime::Runtime {
    BRIDGE_RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(2)
            .build()
            .expect("grpc bridge runtime 创建失败")
    })
}

fn grpc_bridge_sync_timeout_error() -> GatewayError {
    GatewayError::classified(
        "GrpcBridge",
        None,
        "unavailable",
        "grpc_bridge_sync_timeout",
        true,
        "gRPC 同步桥完整 future 超过 20 秒总期限",
    )
}

trait GrpcBridgeSyncError {
    fn grpc_bridge_sync_timeout() -> Self;
}

impl GrpcBridgeSyncError for GatewayError {
    fn grpc_bridge_sync_timeout() -> Self {
        grpc_bridge_sync_timeout_error()
    }
}

impl GrpcBridgeSyncError for OutcomeTransportFailure {
    fn grpc_bridge_sync_timeout() -> Self {
        Self::new(grpc_bridge_sync_timeout_error(), Vec::new())
    }
}

fn run_on_bridge_runtime_with_timeout<F, T, E>(fut: F, timeout: std::time::Duration) -> Result<T, E>
where
    F: std::future::Future<Output = Result<T, E>> + Send,
    T: Send,
    E: GrpcBridgeSyncError + Send,
{
    bridge_runtime().block_on(async move {
        let outcome = tokio::time::timeout(timeout, fut).await;
        outcome.unwrap_or_else(|_| Err(E::grpc_bridge_sync_timeout()))
    })
}

fn block_on_with_timeout<F, T, E>(fut: F, timeout: std::time::Duration) -> Result<T, E>
where
    F: std::future::Future<Output = Result<T, E>> + Send,
    T: Send,
    E: GrpcBridgeSyncError + Send,
{
    match tokio::runtime::Handle::try_current() {
        Ok(_) => std::thread::scope(|scope| {
            scope
                .spawn(move || run_on_bridge_runtime_with_timeout(fut, timeout))
                .join()
                .unwrap_or_else(|_| panic!("grpc 桥 blocking 线程 panic (runtime 上下文路径)"))
        }),
        Err(_) => run_on_bridge_runtime_with_timeout(fut, timeout),
    }
}

/// block_on 判别器 (两路, 任何线程上下文都安全 — 生产 monitor 的同步网关调用
/// 直接在 runtime 线程上发生, library 模式就是阻塞该线程, 桥必须保持同语义
/// 而不是 panic):
/// - runtime 上下文 (Handle 命中): 一律走独立 std 线程 + BRIDGE_RUNTIME 并
///   join。不做 Handle::block_on 是因为这些线程里它要么必 panic — worker task
///   (tokio 红线 "Cannot start a runtime from within a runtime", 2026-08-15
///   生产事故根因) 或 block_on 驱动主线程 (#[tokio::main]/#[tokio::test],
///   try_id 无法区分) — 要么只是碰巧合法 (spawn_blocking)。统一独立线程无
///   上下文误判风险。完整 future 在 owning runtime 内受 BR-243 的 20 秒总期限
///   约束；timeout 先 drop future，再 join helper，禁止 detached worker。
/// - 纯同步线程 (Handle 未命中): 静态 BRIDGE_RUNTIME 直接执行同一总期限。
fn block_on<F, T, E>(fut: F) -> Result<T, E>
where
    F: std::future::Future<Output = Result<T, E>> + Send,
    T: Send,
    E: GrpcBridgeSyncError + Send,
{
    block_on_with_timeout(fut, GRPC_BRIDGE_SYNC_TIMEOUT)
}

/// gRPC 客户端桥: 每 op 一个查询方法, 内部 client.query (§10 重试语义) + convert。
/// 连接是惰性 async (`ensure_connected`, 在方法层做) — 同步方法在 blocking 线程
/// 经 block_on 调用, async 方法在 runtime worker 调用; 首连放在方法层避免
/// bridge_for 里 block_on 在 async 上下文 panic (tokio 禁止)。
pub struct GrpcSource {
    addr: String,
    /// 连接态缓存: None = 尚未连接成功 (失败不缓存, 下次调用重试)。
    /// tokio Mutex: 跨 await 持有 (Send) — delegate JoinSet spawn 要求。
    client: AsyncMutex<Option<GrpcMarketClient>>,
    /// Optional authenticated ExternalV1 bundle. The path is never logged.
    external_bundle: Option<PathBuf>,
    /// ExternalV1 has a separate channel/profile from the normalized local bridge.
    external_client: AsyncMutex<Option<ExternalClientState>>,
}

struct ExternalClientState {
    client: GrpcMarketClient,
    ready_operations: HashSet<i32>,
}

impl GrpcSource {
    async fn ensure_connected(&self) -> Result<(), GatewayError> {
        let mut guard = self.client.lock().await;
        if guard.is_some() {
            return Ok(());
        }
        let client = GrpcMarketClient::connect(&self.addr).await.map_err(|e| {
            GatewayError::unavailable(
                "GrpcBridge",
                None,
                true,
                format!("gRPC 服务端 {} 不可达: {e}", self.addr),
            )
        })?;
        log::info!("[data_gateway] gRPC 桥已连接: server={}", self.addr);
        *guard = Some(client);
        Ok(())
    }

    pub fn addr(&self) -> &str {
        &self.addr
    }

    async fn query_op(&self, op: Operation, params: Value) -> Result<QueryResult, GatewayError> {
        self.ensure_connected().await?;
        // BR-243: 锁只保护惰性连接初始化, 查询必须并发。全局锁覆盖整个
        // query await 会让一个慢 op (如 tdx-smart failover 20s+) 阻塞所有
        // 排队请求, 排队者超过 GRPC_BRIDGE_SYNC_TIMEOUT 后全部超时取消。
        let mut client = {
            let guard = self.client.lock().await;
            guard
                .as_ref()
                .expect("ensure_connected 后必有 client")
                .clone()
        };
        client
            .query(op, params)
            .await
            .map_err(|e| map_query_error(op, &e))
    }

    /// BR-251 has a local-only generated RPC that is intentionally absent from
    /// `GrpcMarketClient::data_call`: R-13 keeps that production router frozen.
    /// This narrow client still uses the shared envelope/auth contract and
    /// returns the server's real BR-159 receipt for client-side re-admission.
    async fn query_benchmark_op(&self, params: Value) -> Result<QueryResult, BenchmarkGrpcFailure> {
        let request =
            crate::grpc_client::envelope::build_query_request(Operation::BenchmarkBars, params)
                .map_err(|error| {
                    local_benchmark_failure(benchmark_transport_error(
                        "invalid_request",
                        "benchmark_request_envelope_invalid",
                        false,
                        format!("BenchmarkBars request envelope rejected: {error}"),
                    ))
                })?;
        let request_id = request
            .context
            .as_ref()
            .map(|context| context.request_id.clone())
            .ok_or_else(|| {
                local_benchmark_failure(benchmark_transport_error(
                    "invalid_request",
                    "benchmark_request_identity_missing",
                    false,
                    "BenchmarkBars request envelope has no request_id",
                ))
            })?;
        let endpoint = tonic::transport::Endpoint::from_shared(self.addr.clone())
            .map_err(|error| {
                local_benchmark_failure(benchmark_transport_error(
                    "invalid_request",
                    "grpc_address_invalid",
                    false,
                    format!("BenchmarkBars gRPC address rejected: {error}"),
                ))
            })?
            .timeout(GRPC_BRIDGE_SYNC_TIMEOUT);
        let channel = endpoint.connect().await.map_err(|error| {
            local_benchmark_failure(benchmark_transport_error(
                "unavailable",
                "grpc_connect_failed",
                true,
                format!(
                    "BenchmarkBars gRPC server {} unavailable: {error}",
                    self.addr
                ),
            ))
        })?;
        let mut client = crate::grpc_client::pb::magic::market::v1::market_data_service_client::MarketDataServiceClient::new(channel);
        let mut request = tonic::Request::new(request);
        crate::grpc_client::auth::attach_bearer(&mut request).map_err(|error| {
            local_benchmark_failure(benchmark_transport_error(
                "invalid_request",
                "grpc_auth_metadata_invalid",
                false,
                format!("BenchmarkBars bearer metadata rejected: {error}"),
            ))
        })?;
        let response = client
            .benchmark_bars(request)
            .await
            .map_err(|status| classify_benchmark_status(&request_id, status))?
            .into_inner();
        parse_benchmark_query_response(&request_id, response)
    }

    async fn ensure_external_connected(&self, operation: Operation) -> Result<(), GatewayError> {
        let mut guard = self.external_client.lock().await;
        if let Some(state) = guard.as_mut() {
            if state.ready_operations.contains(&(operation as i32)) {
                return Ok(());
            }
            let capabilities = state
                .client
                .get_capabilities()
                .await
                .map_err(map_external_connection_error)?;
            require_external_capability(&capabilities, operation)?;
            state.ready_operations.insert(operation as i32);
            return Ok(());
        }
        let bundle = self.external_bundle.as_ref().ok_or_else(|| {
            GatewayError::classified(
                "GrpcExternalV1",
                None,
                "unavailable",
                "external_bundle_unconfigured",
                false,
                "ExternalV1 client-bundle 未配置",
            )
        })?;
        if !bundle.is_absolute() {
            return Err(GatewayError::classified(
                "GrpcExternalV1",
                None,
                "invalid_request",
                "external_bundle_invalid",
                false,
                "ExternalV1 client-bundle 必须是绝对路径",
            ));
        }

        let mut client = GrpcMarketClient::connect_client_bundle(bundle)
            .await
            .map_err(map_external_connection_error)?;
        let health = client
            .get_health()
            .await
            .map_err(map_external_connection_error)?;
        if !health.live || !health.ready {
            return Err(GatewayError::classified(
                "GrpcExternalV1",
                None,
                "unavailable",
                "external_health_not_ready",
                true,
                "ExternalV1 health 未达到 live+ready",
            ));
        }
        let capabilities = client
            .get_capabilities()
            .await
            .map_err(map_external_connection_error)?;
        require_external_capability(&capabilities, operation)?;
        log::info!(
            "[data_gateway] ExternalV1 已通过 health/capability gate: operation={operation:?}"
        );
        *guard = Some(ExternalClientState {
            client,
            ready_operations: HashSet::from([operation as i32]),
        });
        Ok(())
    }

    async fn query_external_op(
        &self,
        operation: Operation,
        params: Value,
    ) -> Result<QueryResult, GatewayError> {
        crate::grpc_client::external_v1::build_external_query_request(operation, params.clone())
            .map_err(|_| {
                GatewayError::classified(
                    "GrpcExternalV1",
                    None,
                    "invalid_request",
                    "external_contract_rejected",
                    false,
                    "ExternalV1 operation 或参数未在交付合同中冻结",
                )
            })?;
        self.ensure_external_connected(operation).await?;
        let mut guard = self.external_client.lock().await;
        let state = guard
            .as_mut()
            .expect("ensure_external_connected 后必有 external client");
        state
            .client
            .query(operation, params)
            .await
            .map_err(|error| map_external_query_error(operation, &error))
    }

    // ---------- 6 个首批 op (M2) ----------

    pub async fn realtime_quotes_async(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<RealtimeMarketQuote>, GatewayError> {
        require_valid_requested_codes("RealtimeMarketQuotes", codes)?;
        let q = self
            .query_op(
                Operation::RealtimeQuotes,
                serde_json::json!({ "codes": codes }),
            )
            .await?;
        let batch = convert::realtime_quotes_at(&q, chrono::Utc::now())?;
        let returned = batch
            .records()
            .iter()
            .map(|record| record.code.clone())
            .collect::<Vec<_>>();
        require_exact_response_codes(
            "RealtimeMarketQuotes",
            codes,
            &returned,
            batch.evidence().provider,
        )?;
        Ok(batch)
    }

    /// 同步包装 (spawn_blocking / 纯同步线程)。
    pub fn realtime_quotes(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<RealtimeMarketQuote>, GatewayError> {
        block_on(self.realtime_quotes_async(codes))
    }

    pub async fn minute_data_async(
        &self,
        code: &str,
    ) -> Result<GatewayBatch<MarketMinutePoint>, GatewayError> {
        let q = self
            .query_op(
                Operation::MinuteData,
                serde_json::json!({ "codes": [code] }),
            )
            .await?;
        convert::minute_data(&q)
    }

    pub fn minute_data(&self, code: &str) -> Result<GatewayBatch<MarketMinutePoint>, GatewayError> {
        block_on(self.minute_data_async(code))
    }

    pub async fn order_books_async(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<MarketOrderBook>, GatewayError> {
        require_valid_requested_codes("MarketOrderBooks", codes)?;
        let q = self
            .query_op(Operation::OrderBooks, serde_json::json!({ "codes": codes }))
            .await?;
        let batch = convert::order_books_at(&q, chrono::Utc::now())?;
        let returned = batch
            .records()
            .iter()
            .map(|record| record.code.clone())
            .collect::<Vec<_>>();
        require_exact_response_codes(
            "MarketOrderBooks",
            codes,
            &returned,
            batch.evidence().provider,
        )?;
        Ok(batch)
    }

    pub fn order_books(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<MarketOrderBook>, GatewayError> {
        block_on(self.order_books_async(codes))
    }

    pub async fn money_flows_async(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<MarketMoneyFlow>, GatewayError> {
        let q = self
            .query_op(Operation::MoneyFlows, serde_json::json!({ "codes": codes }))
            .await?;
        convert::money_flows(&q)
    }

    pub fn money_flows(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<MarketMoneyFlow>, GatewayError> {
        block_on(self.money_flows_async(codes))
    }

    pub async fn security_metadata_async(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<MarketSecurityMetadata>, GatewayError> {
        // 文档 §8 契约: instruments 对象数组 (exchange 由 code 前缀推导)。
        let params = crate::grpc_contract::params::instruments_for(codes);
        let q = self.query_op(Operation::SecurityMetadata, params).await?;
        convert::security_metadata(&q)
    }

    pub fn security_metadata(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<MarketSecurityMetadata>, GatewayError> {
        block_on(self.security_metadata_async(codes))
    }

    /// BR-238: the narrow identity projection is the only metadata consumer
    /// permitted to use the authenticated ExternalV1 partial contract.
    pub async fn security_identities_async(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<MarketSecurityIdentity>, GatewayError> {
        let instruments = production_equity_instruments(codes)?;
        let result = self
            .query_external_op(
                Operation::SecurityMetadata,
                serde_json::json!({ "instruments": instruments }),
            )
            .await?;
        convert::security_identities(codes, &result, chrono::Utc::now())
    }

    pub async fn daily_bars_async(
        &self,
        code: &str,
        days: usize,
    ) -> Result<GatewayBatch<KlineData>, GatewayError> {
        let q = self
            .query_op(
                Operation::HistoricalBars,
                serde_json::json!({ "codes": [code], "days": days }),
            )
            .await?;
        convert::historical_bars(code, &q)
    }

    pub(crate) async fn benchmark_bars_async(
        &self,
        request: &crate::data_gateway::BenchmarkRequest,
    ) -> Result<crate::data_gateway::review::AuditedBenchmarkBatch, BenchmarkGrpcFailure> {
        benchmark_bars_with_query(request, |params| self.query_benchmark_op(params)).await
    }

    pub(crate) async fn benchmark_bars_async_for_local_readmission(
        &self,
        request: &crate::data_gateway::BenchmarkRequest,
    ) -> Result<crate::data_gateway::review::AuditedBenchmarkBatch, BenchmarkGrpcFailure> {
        benchmark_bars_with_query_for_local_readmission(request, |params| {
            self.query_benchmark_op(params)
        })
        .await
    }

    /// 同步包装。
    pub fn daily_bars(
        &self,
        code: &str,
        days: usize,
    ) -> Result<GatewayBatch<KlineData>, GatewayError> {
        block_on(self.daily_bars_async(code, days))
    }

    // ---------- M3 批次 1: 全球市场/日历/公告/新闻/交割 ----------

    pub async fn global_indices_async(
        &self,
    ) -> Result<GatewayBatch<GlobalIndexFact>, GatewayError> {
        let q = self
            .query_op(Operation::GlobalIndices, serde_json::json!({}))
            .await?;
        convert::global_indices(&q)
    }

    pub async fn foreign_exchange_async(
        &self,
    ) -> Result<GatewayBatch<ForeignExchangeFact>, GatewayError> {
        let q = self
            .query_op(Operation::ForeignExchange, serde_json::json!({}))
            .await?;
        convert::foreign_exchange(&q)
    }

    pub async fn announcements_async(
        &self,
    ) -> Result<GatewayBatch<EventAnnouncement>, GatewayError> {
        let q = self
            .query_op(Operation::Announcements, serde_json::json!({}))
            .await?;
        convert::announcements(&q)
    }

    pub async fn global_news_async(
        &self,
        provider: GlobalNewsProvider,
        limit: u32,
    ) -> Result<GatewayBatch<GlobalNewsRecord>, GatewayError> {
        let params = serde_json::json!({
            "provider": provider.wire_name(),
            "limit": limit,
        });
        let external = self.external_bundle.is_some();
        let q = if external {
            self.query_external_op(Operation::GlobalNews, params)
                .await?
        } else {
            self.query_op(Operation::GlobalNews, params).await?
        };
        let batch = if external {
            convert::external_global_news(provider, &q)?
        } else {
            convert::global_news(&q)?
        };
        if batch.evidence().provider != provider.provider_id()
            || batch.evidence().source != provider.source()
        {
            return Err(GatewayError::invalid_evidence(
                "GlobalNews",
                Some(provider.provider_id()),
                format!(
                    "global-news response evidence does not match request provider={} source={}",
                    provider.wire_name(),
                    provider.source()
                ),
            ));
        }
        if batch.records().len() > limit as usize {
            return Err(GatewayError::invalid_evidence(
                "GlobalNews",
                Some(provider.provider_id()),
                format!(
                    "global-news response record count {} exceeds requested limit {limit}",
                    batch.records().len()
                ),
            ));
        }
        Ok(batch)
    }

    pub async fn economic_calendar_async(
        &self,
    ) -> Result<GatewayBatch<EconomicReleaseFact>, GatewayError> {
        let q = self
            .query_op(Operation::EconomicCalendar, serde_json::json!({}))
            .await?;
        convert::economic_calendar(&q)
    }

    pub async fn futures_delivery_async(
        &self,
    ) -> Result<GatewayBatch<FuturesDeliveryFact>, GatewayError> {
        let q = self
            .query_op(Operation::FuturesDelivery, serde_json::json!({}))
            .await?;
        convert::futures_delivery(&q)
    }

    // ---------- M3 批次 2: 龙虎榜/大宗/一致预期/板块/研报/北向/财务/技术/资金流/排行/指数/个股新闻/形态/涨停复盘/T0 ----------

    /// 龙虎榜: 参数与本地 DragonTigerGateway::market_review 对齐 (date +
    /// disclosure_limit + stock_limit)。
    pub async fn dragon_tiger_async(
        &self,
        trading_date: NaiveDate,
        disclosure_limit: u32,
        stock_limit: usize,
    ) -> Result<GatewayBatch<DragonTigerStockReview>, GatewayError> {
        let q = self
            .query_op(
                Operation::DragonTiger,
                serde_json::json!({
                    "date": trading_date.format("%Y-%m-%d").to_string(),
                    "disclosure_limit": disclosure_limit,
                    "stock_limit": stock_limit,
                }),
            )
            .await?;
        convert::dragon_tiger(&q)
    }

    pub async fn market_dragon_tiger_async(
        &self,
    ) -> Result<GatewayBatch<DragonTigerStockReview>, GatewayError> {
        let q = self
            .query_op(Operation::MarketDragonTiger, serde_json::json!({}))
            .await?;
        convert::market_dragon_tiger(&q)
    }

    /// 大宗交易: 参数与本地 BlockTradesGateway::market_review 对齐 (codes + date)。
    pub async fn block_trades_async(
        &self,
        codes: &[String],
        trading_date: NaiveDate,
    ) -> Result<GatewayBatch<BlockTradeReview>, GatewayError> {
        let q = self
            .query_op(
                Operation::BlockTrades,
                serde_json::json!({
                    "codes": codes,
                    "date": trading_date.format("%Y-%m-%d").to_string(),
                }),
            )
            .await?;
        convert::block_trades(&q)
    }

    /// 一致预期: 逐代码 (与本地 ConsensusDataGateway::fetch 对齐)。
    pub async fn consensus_async(
        &self,
        code: &str,
    ) -> Result<GatewayBatch<ConsensusData>, GatewayError> {
        let q = self
            .query_op(Operation::Consensus, serde_json::json!({ "codes": [code] }))
            .await?;
        convert::consensus(&q)
    }

    /// 板块目录: kind + limit (与本地 BoardDataGateway::directory 对齐)。
    pub async fn board_directory_async(
        &self,
        kind: BoardKind,
        limit: u32,
    ) -> Result<GatewayBatch<BoardDirectoryFact>, GatewayError> {
        let q = self
            .query_op(
                Operation::BoardDirectory,
                serde_json::json!({ "kind": format!("{kind:?}"), "limit": limit }),
            )
            .await?;
        convert::board_directory(&q)
    }

    /// 板块成分归属: 逐代码 (与本地 BoardDataGateway::memberships 对齐)。
    pub async fn board_constituents_async(
        &self,
        code: &str,
    ) -> Result<GatewayBatch<BoardMembershipRecord>, GatewayError> {
        let q = self
            .query_op(
                Operation::BoardConstituents,
                serde_json::json!({ "codes": [code] }),
            )
            .await?;
        convert::board_constituents(&q)
    }

    /// 同步包装：供已经位于 blocking 调用链的板块归属消费者复用同一桥。
    pub fn board_constituents(
        &self,
        code: &str,
    ) -> Result<GatewayBatch<BoardMembershipRecord>, GatewayError> {
        block_on(self.board_constituents_async(code))
    }

    /// 板块资金流: kind + limit (与本地 BoardDataGateway::day1_flows 对齐)。
    pub async fn board_flows_async(
        &self,
        kind: BoardKind,
        limit: u32,
    ) -> Result<GatewayBatch<BoardFlowFact>, GatewayError> {
        let q = self
            .query_op(
                Operation::BoardFlows,
                serde_json::json!({ "kind": format!("{kind:?}"), "limit": limit }),
            )
            .await?;
        convert::board_flows(&q)
    }

    /// 同步包装 (spawn_blocking / 纯同步线程), 与本地 day1_flows_blocking 对齐。
    pub fn board_flows(
        &self,
        kind: BoardKind,
        limit: u32,
    ) -> Result<GatewayBatch<BoardFlowFact>, GatewayError> {
        block_on(self.board_flows_async(kind, limit))
    }

    /// 板块排行: fid 路由 (f3 → ConceptHits, f62 → MarketRankings) + top_n
    /// (与本地 BoardRankingGateway::fetch_top 对齐; 非法 fid fail-closed)。
    pub async fn board_ranking_async(
        &self,
        fid: &str,
        top_n: usize,
    ) -> Result<GatewayBatch<BoardRankingFact>, GatewayError> {
        let operation = match fid {
            "f3" => Operation::ConceptHits,
            "f62" => Operation::MarketRankings,
            _ => {
                return Err(GatewayError::invalid_request(
                    "GrpcBridge",
                    format!("板块排行 fid 非法: {fid:?} (允许 f3/f62)"),
                ))
            }
        };
        let q = self
            .query_op(operation, serde_json::json!({ "top_n": top_n }))
            .await?;
        convert::board_ranking(&q)
    }

    /// 同步包装 (spawn_blocking / 纯同步线程)。
    pub fn board_ranking(
        &self,
        fid: &str,
        top_n: usize,
    ) -> Result<GatewayBatch<BoardRankingFact>, GatewayError> {
        block_on(self.board_ranking_async(fid, top_n))
    }

    /// 研报: 逐代码 + page_size (与本地 ResearchDataGateway::instrument_reports
    /// 对齐; 服务端 fetch_research_reports 收 codes+page_size)。
    pub async fn research_reports_async(
        &self,
        code: &str,
        page_size: u32,
    ) -> Result<GatewayBatch<ResearchReportFact>, GatewayError> {
        let q = self
            .query_op(
                Operation::ResearchReports,
                serde_json::json!({ "codes": [code], "page_size": page_size }),
            )
            .await?;
        convert::research_reports(&q)
    }

    /// 北向日数据: date + channel (与本地 CapitalDataGateway::northbound_daily
    /// 对齐; 服务端 fetch_northbound_daily 收 date+channel)。
    pub async fn northbound_daily_async(
        &self,
        trading_date: NaiveDate,
        channel: NorthboundChannel,
    ) -> Result<GatewayBatch<NorthboundDailyFact>, GatewayError> {
        let q = self
            .query_op(
                Operation::NorthboundDaily,
                serde_json::json!({
                    "date": trading_date.format("%Y-%m-%d").to_string(),
                    "channel": format!("{channel:?}"),
                }),
            )
            .await?;
        convert::northbound_daily(&q)
    }

    /// 财务报告: codes + kind (与本地 CompanyDataGateway::financial_statements
    /// 对齐; 服务端 fetch_financial_statements 的 kind 是 snake_case 字面量)。
    pub async fn financial_statements_async(
        &self,
        codes: &[String],
        kind: StatementKind,
    ) -> Result<GatewayBatch<FinancialStatement>, GatewayError> {
        let kind = match kind {
            StatementKind::Balance => "balance",
            StatementKind::Income => "income",
            StatementKind::CashFlow => "cash_flow",
        };
        let q = self
            .query_op(
                Operation::FinancialStatements,
                serde_json::json!({ "codes": codes, "kind": kind }),
            )
            .await?;
        convert::financial_statements(&q)
    }

    /// 估值统计: codes (与本地 CompanyDataGateway::market_statistics 对齐)。
    pub async fn market_statistics_async(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<MarketStatistics>, GatewayError> {
        let q = self
            .query_op(
                Operation::MarketStatistics,
                serde_json::json!({ "codes": codes }),
            )
            .await?;
        convert::market_statistics(&q)
    }

    /// 15 分钟线: codes + count (与本地 HistoricalBarsGateway::fifteen_min_bars
    /// 对齐)。
    pub async fn technical_bars_async(
        &self,
        codes: &[String],
        count: u32,
    ) -> Result<GatewayBatch<SecurityBar>, GatewayError> {
        let q = self
            .query_op(
                Operation::TechnicalBars,
                serde_json::json!({ "codes": codes, "count": count }),
            )
            .await?;
        convert::technical_bars(&q)
    }

    /// 同步包装 (spawn_blocking / 纯同步线程)。
    pub fn technical_bars(
        &self,
        codes: &[String],
        count: u32,
    ) -> Result<GatewayBatch<SecurityBar>, GatewayError> {
        block_on(self.technical_bars_async(codes, count))
    }

    /// 资金流序列: 逐代码 + interval + limit (与本地
    /// CapitalDataGateway::instrument_fund_flow 对齐; 服务端
    /// fetch_fund_flow_series 收 codes+interval+limit)。
    pub async fn fund_flow_series_async(
        &self,
        code: &str,
        interval: FlowInterval,
        limit: u32,
    ) -> Result<GatewayBatch<InstrumentFundFlowFact>, GatewayError> {
        let q = self
            .query_op(
                Operation::FundFlowSeries,
                serde_json::json!({
                    "codes": [code],
                    "interval": format!("{interval:?}"),
                    "limit": limit,
                }),
            )
            .await?;
        convert::fund_flow_series(&q)
    }

    /// 头部排行双路 (volume_ratio + main_net_inflow): 与本地
    /// CapitalDataGateway::provider_top_n_pair 对齐 — 客户端 convert 按 metric
    /// 分组重建两个 GatewayBatch (request evidence 由本地方法构造, 桥只换
    /// transport 数据)。
    pub async fn provider_top_n_pair_async(
        &self,
        trading_date: NaiveDate,
    ) -> Result<
        (
            GatewayBatch<ProviderTopNFact>,
            GatewayBatch<ProviderTopNFact>,
        ),
        GatewayError,
    > {
        let q = self
            .query_op(
                Operation::ProviderTopNRankings,
                serde_json::json!({ "date": trading_date.format("%Y-%m-%d").to_string() }),
            )
            .await?;
        convert::provider_top_n_pair(&q)
    }

    /// 指数实时行情: codes (与本地 IndexDataGateway::realtime_quotes 对齐)。
    pub async fn index_quotes_async(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<RealtimeIndexQuote>, GatewayError> {
        let q = self
            .query_op(
                Operation::IndexQuotes,
                serde_json::json!({ "codes": codes }),
            )
            .await?;
        convert::index_quotes(&q)
    }

    /// 同步包装 (spawn_blocking / 纯同步线程)。
    pub fn index_quotes(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<RealtimeIndexQuote>, GatewayError> {
        block_on(self.index_quotes_async(codes))
    }

    /// 个股新闻: codes + from_days (与本地 SinaInstrumentNewsGateway
    /// instrument_news_in_range 对齐; 范围终点=服务端当前时刻, 同机等价)。
    pub async fn instrument_news_async(
        &self,
        codes: &[String],
        from_days: u32,
    ) -> Result<GatewayBatch<SinaInstrumentNewsRecord>, GatewayError> {
        let [code] = codes else {
            return Err(GatewayError::invalid_request(
                "GrpcExternalV1",
                "ExternalV1 InstrumentNews requires exactly one canonical instrument",
            ));
        };
        if !(1..=30).contains(&from_days) {
            return Err(GatewayError::invalid_request(
                "GrpcExternalV1",
                "ExternalV1 InstrumentNews from_days must be within 1..=30",
            ));
        }
        let instrument = production_equity_instrument(code)?;
        let captured_at = chrono::Local::now().fixed_offset();
        let captured_through = captured_at.with_timezone(&chrono::Utc);
        let end = captured_at.date_naive();
        let start = end
            .checked_sub_signed(chrono::Duration::days(i64::from(from_days)))
            .ok_or_else(|| {
                GatewayError::invalid_request(
                    "GrpcExternalV1",
                    "ExternalV1 InstrumentNews date range overflow",
                )
            })?;
        let q = self
            .query_external_op(
                Operation::InstrumentNews,
                instrument_news_request_params(&instrument, start, end, 100, captured_at),
            )
            .await?;
        convert::external_instrument_news_in_range_at(
            code,
            &instrument,
            &q,
            convert::ExternalInstrumentNewsRequestContext::new(
                start,
                end,
                100,
                captured_through,
                chrono::Utc::now(),
            ),
        )
    }

    /// BR-238 startup canary exercises the same start/end contract as the real
    /// post-close consumer. It never persists the returned item and limit=1
    /// bounds startup traffic.
    async fn instrument_news_canary_async(
        &self,
        code: &str,
    ) -> Result<GatewayBatch<SinaInstrumentNewsRecord>, GatewayError> {
        let instrument = production_equity_instrument(code)?;
        let captured_at = chrono::Local::now().fixed_offset();
        let captured_through = captured_at.with_timezone(&chrono::Utc);
        let end = captured_at.date_naive();
        let start = crate::calendar::prev_trading_day(end);
        let q = self
            .query_external_op(
                Operation::InstrumentNews,
                instrument_news_request_params(&instrument, start, end, 1, captured_at),
            )
            .await?;
        convert::external_instrument_news_in_range_at(
            code,
            &instrument,
            &q,
            convert::ExternalInstrumentNewsRequestContext::new(
                start,
                end,
                1,
                captured_through,
                chrono::Utc::now(),
            ),
        )
    }

    /// 日内形态: 逐代码 (与本地 IntradayShapeGateway::current_shape 对齐;
    /// 服务端 fetch_intraday_shape 收 codes)。
    pub async fn intraday_shape_async(
        &self,
        code: &str,
    ) -> Result<GatewayBatch<IntradayShapeFact>, GatewayError> {
        let q = self
            .query_op(
                Operation::IntradayShape,
                serde_json::json!({ "codes": [code] }),
            )
            .await?;
        convert::intraday_shape(&q)
    }

    /// 涨停复盘: date (与本地 ReviewGateway::r03_upper_limit_pool 对齐)。
    pub async fn upper_limit_pool_review_async(
        &self,
        trading_date: NaiveDate,
    ) -> Result<GatewayBatch<UpperLimitRecord>, GatewayError> {
        let q = self
            .query_op(
                Operation::UpperLimitPoolReview,
                serde_json::json!({ "date": trading_date.format("%Y-%m-%d").to_string() }),
            )
            .await?;
        convert::upper_limit_pool_review(&q)
    }

    /// P-01 exact-date complete upper-limit pool. The LocalBridge request is
    /// deliberately fixed to the provider's whole-pool bound; callers own
    /// only the trading date.
    pub async fn limit_pools_async(
        &self,
        trading_date: NaiveDate,
    ) -> Result<GatewayBatch<LimitPoolEntry>, GatewayError> {
        let q = self
            .query_op(Operation::LimitPools, limit_pools_request(trading_date))
            .await?;
        convert::limit_pools(&q, trading_date)
    }

    /// Runtime-safe blocking seam retained by the synchronous P-01 consumer.
    pub fn limit_pools(
        &self,
        trading_date: NaiveDate,
    ) -> Result<GatewayBatch<LimitPoolEntry>, GatewayError> {
        block_on(self.limit_pools_async(trading_date))
    }

    /// T0 证据批: 返回 MagicTdxT0Batch (records + rejections 全量, 与本地
    /// MagicTdxGateway::get_t0_evidence_batch 对齐 — rejections 不能丢)。
    pub async fn t0_evidence_batch_async(
        &self,
        codes: &[String],
    ) -> Result<MagicTdxT0Batch, GatewayError> {
        let q = self
            .query_op(Operation::T0Evidence, serde_json::json!({ "codes": codes }))
            .await?;
        let batch = convert::t0_evidence_batch_at(&q, chrono::Utc::now())?;
        let requested = codes.iter().map(String::as_str).collect::<HashSet<_>>();
        let outcomes = batch
            .records
            .iter()
            .map(|record| record.code.as_str())
            .chain(
                batch
                    .rejections
                    .iter()
                    .map(|rejection| rejection.code.as_str()),
            )
            .collect::<Vec<_>>();
        let outcome_set = outcomes.iter().copied().collect::<HashSet<_>>();
        if requested.is_empty()
            || requested.len() != codes.len()
            || outcome_set.len() != outcomes.len()
            || outcome_set != requested
        {
            return Err(GatewayError::invalid_evidence(
                "T0Evidence",
                Some(batch.provider),
                "T0Evidence outcomes do not exactly match the requested instrument set",
            ));
        }
        Ok(batch)
    }

    /// 同步包装 (spawn_blocking / 纯同步线程)。
    pub fn t0_evidence_batch(&self, codes: &[String]) -> Result<MagicTdxT0Batch, GatewayError> {
        block_on(self.t0_evidence_batch_async(codes))
    }

    /// 联网检索: exact provider + query + limit (BR-242)。
    pub async fn semantic_search_async(
        &self,
        provider: GeneralWebResearchProvider,
        query: &str,
        limit: usize,
    ) -> Result<GeneralWebResearchBatch, GatewayError> {
        let q = self
            .query_op(
                Operation::SemanticSearch,
                serde_json::json!({
                    "provider": provider.wire_name(),
                    "query": query,
                    "limit": limit,
                }),
            )
            .await?;
        convert::semantic_search(&q, query, provider, limit)
    }

    /// 公司行动: code + window (与本地 SecurityLifecycleGateway::acquire 的
    /// corporate_actions 部分对齐; 服务端 fetch_corporate_actions 收
    /// code+window_start+window_end, 已服务端侧完成 Implemented 投影)。
    pub async fn corporate_actions_async(
        &self,
        code: &str,
        window_start: NaiveDate,
        window_end: NaiveDate,
    ) -> Result<GatewayBatch<ImplementedCorporateAction>, GatewayError> {
        let q = self
            .query_op(
                Operation::CorporateActions,
                serde_json::json!({
                    "code": code,
                    "window_start": window_start.format("%Y-%m-%d").to_string(),
                    "window_end": window_end.format("%Y-%m-%d").to_string(),
                }),
            )
            .await?;
        convert::corporate_actions(&q)
    }

    /// outcome 复盘日线 (P4 M3): 服务端执行 adaptive 抓取 (claim 台账留客户端),
    /// 视图重建 RawOutcomeFetch / OutcomeTransportFailure (error+attempts 保真)。
    /// 参数与 fetch_magic_tdx_outcome_adaptive 对齐 (instrument 对象 round-trip)。
    pub async fn outcome_daily_bars_async(
        &self,
        instrument: InstrumentId,
        market: String,
        code: String,
        expected_bar_count: u16,
        maximum_latest_n: u16,
        window_start: NaiveDate,
    ) -> Result<RawOutcomeFetch, OutcomeTransportFailure> {
        let q = self
            .query_op(
                Operation::OutcomeDailyBars,
                serde_json::json!({
                    "instrument": instrument,
                    "market": market,
                    "code": code,
                    "expected_bar_count": expected_bar_count,
                    "maximum_latest_n": maximum_latest_n,
                    "window_start": window_start.format("%Y-%m-%d").to_string(),
                }),
            )
            .await
            .map_err(|e| OutcomeTransportFailure::new(e, Vec::new()))?;
        convert::outcome_daily_bars(&q)
    }

    /// 同步包装 (spawn_blocking / 纯同步线程)。
    pub fn outcome_daily_bars_adaptive(
        &self,
        instrument: InstrumentId,
        market: String,
        code: String,
        expected_bar_count: u16,
        maximum_latest_n: u16,
        window_start: NaiveDate,
    ) -> Result<RawOutcomeFetch, OutcomeTransportFailure> {
        block_on(self.outcome_daily_bars_async(
            instrument,
            market,
            code,
            expected_bar_count,
            maximum_latest_n,
            window_start,
        ))
    }

    /// M4c: A-10 完整 batch (op 61, market.chain_batch)。服务端执行
    /// build_for_date 计算+stage+publish (单写方), 本方法只重建 VisibleChainBatch。
    pub async fn chain_batch_async(
        &self,
        date: &str,
    ) -> Result<crate::database::chain_intelligence::VisibleChainBatch, GatewayError> {
        let q = self
            .query_op(Operation::ChainBatch, serde_json::json!({ "date": date }))
            .await
            .map_err(|e| {
                GatewayError::classified(
                    "A-10",
                    Some(ProviderId::Custom),
                    "unavailable",
                    "chain_batch_fetch",
                    true,
                    format!("A-10 chain_batch op 61 查询失败: {e}"),
                )
            })?;
        let record = q.records.first().ok_or_else(|| {
            GatewayError::classified(
                "A-10",
                Some(ProviderId::Custom),
                "unavailable",
                "empty_chain_batch",
                true,
                "A-10 chain_batch 响应无记录".to_string(),
            )
        })?;
        let batch: crate::database::chain_intelligence::VisibleChainBatch =
            serde_json::from_slice(&record.data).map_err(|e| {
                GatewayError::classified(
                    "A-10",
                    Some(ProviderId::Custom),
                    "unavailable",
                    "chain_batch_parse",
                    true,
                    format!("VisibleChainBatch 反序列化失败: {e}"),
                )
            })?;
        if batch.trading_date.format("%Y-%m-%d").to_string() != date {
            return Err(GatewayError::classified(
                "A-10",
                Some(ProviderId::Custom),
                "unavailable",
                "chain_batch_date_mismatch",
                true,
                format!(
                    "A-10 visible batch as_of={} differs from requested {}",
                    batch.trading_date, date
                ),
            ));
        }
        Ok(batch)
    }
}

/// M4c: A-10 完整 batch 静态入口 (catalyst_review 复盘消费, 无桥实例上下文)。
/// gRPC 模式 (DATA_GATEWAY_GRPC=1) → 服务端计算+写库, 返回完整 batch;
/// library 模式 → Ok(None), 调用方走本地 build_for_date (默认出声, v15.x);
/// gRPC 模式失败 → Err (fail-closed, 绝不静默回退 library 重算)。
pub async fn fetch_chain_batch_grpc(
    date: &str,
) -> Result<Option<crate::database::chain_intelligence::VisibleChainBatch>, GatewayError> {
    if std::env::var("DATA_GATEWAY_GRPC").as_deref() != Ok("1") {
        return Ok(None);
    }
    let source = bridge_for("ChainBatch")?.ok_or_else(|| {
        GatewayError::classified(
            "A-10",
            Some(ProviderId::Custom),
            "unavailable",
            "bridge_disabled",
            true,
            "ChainBatch 被 DATA_GATEWAY_GRPC_DISABLED 排除, 复盘需要完整 batch \
             — 不静默回退 library (fail-closed)"
                .to_string(),
        )
    })?;
    source.chain_batch_async(date).await.map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grpc_client::errors::{ErrorDetail, GrpcError};
    use crate::grpc_client::pb::magic::market::v1 as pb;
    use crate::magic_compat::ProviderId;
    use chrono::TimeZone;
    use prost::Message; // pb::ErrorDetail::encode_to_vec
                        // env 是进程级: 这些测试并行时会互相看到对方的 env (race)。
                        // 共享锁串行化 env 敏感的测试 (M3 全量并行跑时暴露)。

    #[test]
    fn br243_sync_context_returns_typed_retryable_timeout() {
        let started = std::time::Instant::now();
        let error = block_on_with_timeout(
            std::future::pending::<Result<(), GatewayError>>(),
            std::time::Duration::from_millis(25),
        )
        .expect_err("pending gRPC bridge future must hit the complete-future deadline");

        assert_eq!(error.capability(), "GrpcBridge");
        assert_eq!(error.audit_outcome(), "unavailable");
        assert_eq!(error.reason_code(), "grpc_bridge_sync_timeout");
        assert!(error.retryable());
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "timeout must return promptly instead of waiting for the future"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn br243_tokio_runtime_context_returns_typed_retryable_timeout() {
        let error = block_on_with_timeout(
            std::future::pending::<Result<(), GatewayError>>(),
            std::time::Duration::from_millis(25),
        )
        .expect_err("runtime caller must use the joined helper-thread timeout path");

        assert_eq!(error.reason_code(), "grpc_bridge_sync_timeout");
        assert!(error.retryable());
    }

    #[test]
    fn br243_timeout_drops_future_before_sync_helper_returns() {
        struct DropProbeFuture {
            dropped: std::sync::Arc<std::sync::atomic::AtomicBool>,
        }

        impl std::future::Future for DropProbeFuture {
            type Output = Result<(), GatewayError>;

            fn poll(
                self: std::pin::Pin<&mut Self>,
                _context: &mut std::task::Context<'_>,
            ) -> std::task::Poll<Self::Output> {
                std::task::Poll::Pending
            }
        }

        impl Drop for DropProbeFuture {
            fn drop(&mut self) {
                self.dropped
                    .store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }

        let dropped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let error = block_on_with_timeout(
            DropProbeFuture {
                dropped: std::sync::Arc::clone(&dropped),
            },
            std::time::Duration::from_millis(25),
        )
        .expect_err("drop probe must time out");

        assert_eq!(error.reason_code(), "grpc_bridge_sync_timeout");
        assert!(
            dropped.load(std::sync::atomic::Ordering::SeqCst),
            "owning runtime must drop the timed-out future before the joined helper returns"
        );
    }

    #[test]
    fn br243_normal_result_is_preserved_and_production_deadline_is_literal() {
        assert_eq!(
            GRPC_BRIDGE_SYNC_TIMEOUT,
            std::time::Duration::from_secs(35),
            "BR-243 pins one literal 35-second complete-future deadline \
             (20s caused BoardConstituents TDX block-file download edge timeouts)"
        );

        let expected = "TEST_CODE bridge payload".to_string();
        let actual = block_on_with_timeout(
            std::future::ready(Ok::<String, GatewayError>(expected.clone())),
            std::time::Duration::from_secs(1),
        )
        .expect("ready result must pass through unchanged");
        assert_eq!(actual, expected);
    }

    #[test]
    fn br243_completed_error_classification_is_preserved() {
        let original = GatewayError::classified(
            "TEST_CODE_Capability",
            Some(ProviderId::Tdx),
            "partial",
            "TEST_CODE_original_reason",
            false,
            "TEST_CODE original message",
        );
        let actual = block_on_with_timeout(
            std::future::ready(Err::<(), GatewayError>(original)),
            std::time::Duration::from_secs(1),
        )
        .expect_err("completed gateway error must remain an error");

        assert_eq!(actual.capability(), "TEST_CODE_Capability");
        assert_eq!(actual.provider(), Some(ProviderId::Tdx));
        assert_eq!(actual.audit_outcome(), "partial");
        assert_eq!(actual.reason_code(), "TEST_CODE_original_reason");
        assert!(!actual.retryable());
        assert_eq!(actual.message(), "TEST_CODE original message");
    }

    #[test]
    fn br243_outcome_transport_timeout_keeps_typed_failure_envelope() {
        let failure = block_on_with_timeout(
            std::future::pending::<Result<(), OutcomeTransportFailure>>(),
            std::time::Duration::from_millis(25),
        )
        .expect_err("outcome wrapper must expose the same typed sync timeout");

        assert_eq!(failure.error.reason_code(), "grpc_bridge_sync_timeout");
        assert!(failure.error.retryable());
        assert!(
            failure.attempts.is_empty(),
            "a timed-out future cannot fabricate provider attempts"
        );
    }

    /// D2 核心: Fetch 失败 (Internal + ErrorDetail) 按 detail 重建分类 —
    /// provider/reason_code/retryable 保真, 不折叠 (BR-170 pre-fix 形态回归)。
    #[test]
    fn map_query_error_restores_fetch_classification() {
        let err = GrpcError::Internal {
            details: Box::new(ErrorDetail {
                code: "internal".to_string(),
                request_id: Some("req-1".to_string()),
                operation: Some(8),
                provider: Some("Tdx".to_string()),
                reason_code: Some("no_verified_batch".to_string()),
                retryable: Some(true),
                diagnostic_message: None,
                ..Default::default()
            }),
        };
        let g = map_query_error(Operation::BoardConstituents, &err);
        assert_eq!(g.capability(), "GrpcBridge");
        assert_eq!(g.provider(), Some(ProviderId::Tdx));
        assert_eq!(g.reason_code(), "no_verified_batch");
        assert_eq!(g.audit_outcome(), "unavailable");
        assert!(g.retryable());
        assert!(g.message().contains("BoardConstituents"));
    }

    /// D2: 服务端未知 provider 名 (新增 provider 未同步 parse_provider) →
    /// provider=None 但 reason_code/retryable 仍保真 (fail-closed, 不猜默认)。
    #[test]
    fn map_query_error_unknown_provider_keeps_rest() {
        let err = GrpcError::Internal {
            details: Box::new(ErrorDetail {
                code: "internal".to_string(),
                provider: Some("NewProvider".to_string()),
                reason_code: Some("database_failure".to_string()),
                retryable: Some(false),
                ..Default::default()
            }),
        };
        let g = map_query_error(Operation::RealtimeQuotes, &err);
        assert_eq!(g.provider(), None);
        assert_eq!(g.reason_code(), "database_failure");
        assert!(!g.retryable());
    }

    /// D2: 请求/权限类错误码 → invalid_request 不重试 (重试不会变好)。
    #[test]
    fn map_query_error_request_class_codes_no_retry() {
        for code in [
            GrpcError::InvalidArgument {
                details: Box::default(),
            },
            GrpcError::Unimplemented {
                details: Box::default(),
            },
            GrpcError::PermissionDenied {
                details: Box::default(),
            },
            GrpcError::Unauthenticated {
                details: Box::default(),
            },
            GrpcError::ResourceExhausted {
                details: Box::default(),
            },
            GrpcError::FailedPrecondition {
                details: Box::default(),
            },
        ] {
            let g = map_query_error(Operation::RealtimeQuotes, &code);
            assert_eq!(g.audit_outcome(), "invalid_request", "{code:?}");
            assert!(!g.retryable(), "{code:?}");
        }
    }

    /// D2: Unavailable 无 ErrorDetail (服务端不可达, connect 失败) →
    /// 默认 reason_code=no_verified_batch + retryable=true (原有语义不变)。
    #[test]
    fn map_query_error_unavailable_without_detail_keeps_defaults() {
        let err = GrpcError::Unavailable {
            details: Box::default(),
        };
        let g = map_query_error(Operation::HistoricalBars, &err);
        assert_eq!(g.reason_code(), "no_verified_batch");
        assert!(g.retryable());
    }

    #[test]
    fn benchmark_reason_codes_are_closed_without_internal_folding() {
        for reason in [
            "benchmark_instrument_unsupported",
            "benchmark_test_identity_rejected",
            "benchmark_range_reversed",
            "benchmark_identity_unverified",
            "benchmark_time_semantics_unavailable",
            "benchmark_trading_calendar_unavailable",
            "benchmark_page_empty_before_range",
            "benchmark_ohlc_inconsistent",
        ] {
            assert_eq!(reason_code_static(reason), reason, "reason={reason}");
        }
    }

    #[test]
    fn benchmark_transport_ownership_is_explicit_not_reason_derived() {
        let same_error = GatewayError::classified(
            "GrpcBridge",
            None,
            "unavailable",
            "provider_transport",
            true,
            "TEST_CODE identical taxonomy",
        );
        let local = BenchmarkGrpcFailure::client_before_send(same_error.clone());
        assert_eq!(local.ownership(), BenchmarkGrpcOwnership::ClientBeforeSend);
        assert_eq!(local.reason_code(), "provider_transport");

        let server = benchmark_typed_failure_for_test(GatewayError::classified(
            "GrpcBridge",
            Some(ProviderId::Tdx),
            "unavailable",
            "provider_transport",
            true,
            "TEST_CODE identical taxonomy with typed provider marker",
        ));
        assert_eq!(server.ownership(), BenchmarkGrpcOwnership::ServerHandled);
        assert_eq!(server.reason_code(), "provider_transport");

        let unknown = classify_benchmark_status(
            "TEST_CODE_wrong_request",
            tonic::Status::with_details(
                tonic::Code::Internal,
                "TEST_CODE same reason without valid marker",
                pb::BenchmarkErrorDetail {
                    error: Some(pb::ErrorDetail {
                        request_id: "TEST_CODE_different_request".to_owned(),
                        operation: Operation::BenchmarkBars as i32,
                        provider: "Tdx".to_owned(),
                        reason_code: "provider_transport".to_owned(),
                        retryable: true,
                        ..Default::default()
                    }),
                    audit_outcome: "unavailable".to_owned(),
                    audit_state: BenchmarkServerAuditState::Persisted.as_proto(),
                }
                .encode_to_vec()
                .into(),
            ),
        );
        assert_eq!(unknown.ownership(), BenchmarkGrpcOwnership::OutcomeUnknown);
        assert_eq!(unknown.reason_code(), "transport_outcome_unknown");
    }

    #[test]
    fn benchmark_typed_server_failures_preserve_outcome_reason_and_retryability() {
        let request_id = "TEST_CODE_benchmark_request";
        for spec in BENCHMARK_SERVER_FAILURES {
            let outcome = spec.audit_outcome;
            let reason = spec.reason_code;
            let retryable = spec.retryable;
            let audit_state = spec.audit_state;
            let detail = pb::BenchmarkErrorDetail {
                error: Some(pb::ErrorDetail {
                    request_id: request_id.to_owned(),
                    operation: Operation::BenchmarkBars as i32,
                    provider: "Tdx".to_owned(),
                    reason_code: reason.to_owned(),
                    retryable,
                    ..Default::default()
                }),
                audit_outcome: outcome.to_owned(),
                audit_state: audit_state.as_proto(),
            };
            let failure = classify_benchmark_status(
                request_id,
                tonic::Status::with_details(
                    tonic::Code::Internal,
                    "TEST_CODE typed server failure",
                    detail.encode_to_vec().into(),
                ),
            );
            let ownership = match audit_state {
                BenchmarkServerAuditState::Persisted => BenchmarkGrpcOwnership::ServerHandled,
                BenchmarkServerAuditState::AppendFailed => {
                    BenchmarkGrpcOwnership::ServerAuditAppendFailed
                }
            };
            assert_eq!(failure.ownership(), ownership);
            assert_eq!(failure.audit_outcome(), outcome);
            assert_eq!(failure.reason_code(), reason);
            assert_eq!(failure.retryable(), retryable);
        }
    }

    #[test]
    fn benchmark_server_failure_marker_rejects_client_reasons_and_illegal_tuples() {
        let request_id = "TEST_CODE_benchmark_request";
        for (outcome, reason, retryable, audit_state) in [
            (
                "unavailable",
                "bridge_disabled",
                false,
                BenchmarkServerAuditState::Persisted,
            ),
            (
                "partial",
                "provider_transport",
                false,
                BenchmarkServerAuditState::Persisted,
            ),
            (
                "unavailable",
                "acquisition_audit_unavailable",
                true,
                BenchmarkServerAuditState::Persisted,
            ),
            (
                "unavailable",
                "provider_transport",
                true,
                BenchmarkServerAuditState::AppendFailed,
            ),
        ] {
            let detail = pb::BenchmarkErrorDetail {
                error: Some(pb::ErrorDetail {
                    request_id: request_id.to_owned(),
                    operation: Operation::BenchmarkBars as i32,
                    provider: "Tdx".to_owned(),
                    reason_code: reason.to_owned(),
                    retryable,
                    ..Default::default()
                }),
                audit_outcome: outcome.to_owned(),
                audit_state: audit_state.as_proto(),
            };
            let failure = classify_benchmark_status(
                request_id,
                tonic::Status::with_details(
                    tonic::Code::Internal,
                    "TEST_CODE invalid typed server tuple",
                    detail.encode_to_vec().into(),
                ),
            );
            assert_eq!(failure.ownership(), BenchmarkGrpcOwnership::OutcomeUnknown);
            assert_eq!(failure.reason_code(), "transport_outcome_unknown");
        }
    }

    #[test]
    fn benchmark_unverified_status_has_ambiguous_transport_ownership() {
        let failure = classify_benchmark_status(
            "TEST_CODE_benchmark_request",
            tonic::Status::unavailable("TEST_CODE response lost"),
        );
        assert_eq!(failure.ownership(), BenchmarkGrpcOwnership::OutcomeUnknown);
        assert_eq!(failure.audit_outcome(), "unavailable");
        assert_eq!(failure.reason_code(), "transport_outcome_unknown");
        assert!(failure.retryable());
    }

    #[test]
    fn benchmark_typed_failure_without_provider_marker_is_outcome_unknown() {
        let request_id = "TEST_CODE_benchmark_request";
        let detail = pb::BenchmarkErrorDetail {
            error: Some(pb::ErrorDetail {
                request_id: request_id.to_owned(),
                operation: Operation::BenchmarkBars as i32,
                provider: String::new(),
                reason_code: "provider_transport".to_owned(),
                retryable: true,
                ..Default::default()
            }),
            audit_outcome: "unavailable".to_owned(),
            audit_state: BenchmarkServerAuditState::Persisted.as_proto(),
        };

        let failure = classify_benchmark_status(
            request_id,
            tonic::Status::with_details(
                tonic::Code::Internal,
                "TEST_CODE missing provider marker",
                detail.encode_to_vec().into(),
            ),
        );

        assert_eq!(failure.ownership(), BenchmarkGrpcOwnership::OutcomeUnknown);
        assert_eq!(failure.audit_outcome(), "unavailable");
        assert_eq!(failure.reason_code(), "transport_outcome_unknown");
        assert!(failure.retryable());
    }

    #[test]
    fn benchmark_test_helper_roundtrips_absent_provider_without_inventing_tdx() {
        let failure = benchmark_typed_failure_for_test(GatewayError::classified(
            "GrpcBridge",
            None,
            "unavailable",
            "provider_transport",
            true,
            "TEST_CODE absent provider",
        ));
        assert_eq!(failure.ownership(), BenchmarkGrpcOwnership::OutcomeUnknown);
        assert_eq!(failure.reason_code(), "transport_outcome_unknown");
        assert!(failure.retryable());
    }

    #[test]
    fn benchmark_unverified_response_envelope_has_ambiguous_ownership() {
        let response = pb::QueryResponse {
            request_id: "TEST_CODE_wrong_request".to_owned(),
            operation: Operation::BenchmarkBars as i32,
            admission: AdmissionState::Admitted as i32,
            selected_provider: "Tdx".to_owned(),
            batch_id: "TEST_CODE_unverified_envelope".to_owned(),
            complete: true,
            observed_at: "2026-08-21T15:01:00+08:00".to_owned(),
            source_at: String::new(),
            records: Vec::new(),
            source: "TEST_CODE benchmark response".to_owned(),
            diagnostic_blocker: String::new(),
        };
        let failure = parse_benchmark_query_response("TEST_CODE_expected_request", response)
            .expect_err("unverified request echo cannot prove server audit ownership");
        assert_eq!(failure.ownership(), BenchmarkGrpcOwnership::OutcomeUnknown);
        assert_eq!(failure.audit_outcome(), "unavailable");
        assert_eq!(failure.reason_code(), "transport_outcome_unknown");
        assert!(failure.retryable());
    }

    #[tokio::test]
    async fn benchmark_request_validation_precedes_wire_and_transport() {
        let transport_calls = std::sync::atomic::AtomicUsize::new(0);
        let shanghai = chrono::FixedOffset::east_opt(8 * 60 * 60).expect("Shanghai offset");
        let day = NaiveDate::from_ymd_opt(2026, 8, 21).expect("TEST_CODE date");
        let off_grid = crate::data_gateway::BenchmarkRequest {
            instrument: crate::data_gateway::HS300_CANONICAL.to_owned(),
            range: crate::data_gateway::BenchmarkRange::Minute1 {
                from: shanghai
                    .with_ymd_and_hms(2026, 8, 21, 9, 31, 30)
                    .single()
                    .expect("TEST_CODE second"),
                to: shanghai
                    .with_ymd_and_hms(2026, 8, 21, 9, 32, 0)
                    .single()
                    .expect("TEST_CODE minute"),
            },
        };
        let reversed = crate::data_gateway::BenchmarkRequest {
            instrument: crate::data_gateway::HS300_CANONICAL.to_owned(),
            range: crate::data_gateway::BenchmarkRange::Daily {
                from: day,
                to: day.pred_opt().expect("TEST_CODE previous day"),
            },
        };

        for (request, expected_reason) in [
            (off_grid, "benchmark_minute_range_off_grid"),
            (reversed, "benchmark_range_reversed"),
        ] {
            let error = benchmark_bars_with_query(&request, |_| {
                transport_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                std::future::ready(Err(unknown_benchmark_transport_failure()))
            })
            .await
            .expect_err("invalid request must stop before transport construction");
            assert_eq!(error.ownership(), BenchmarkGrpcOwnership::ClientBeforeSend);
            assert_eq!(error.capability(), "BenchmarkBars");
            assert_eq!(error.audit_outcome(), "invalid_request");
            assert_eq!(error.reason_code(), expected_reason);
            assert!(!error.retryable());
        }
        assert_eq!(
            transport_calls.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "invalid requests must not invoke the transport closure"
        );
    }

    #[tokio::test]
    async fn pure_nanosecond_minute_request_fails_before_transport() {
        let transport_calls = std::sync::atomic::AtomicUsize::new(0);
        let request = crate::data_gateway::BenchmarkRequest {
            instrument: crate::data_gateway::HS300_CANONICAL.to_owned(),
            range: crate::data_gateway::BenchmarkRange::Minute1 {
                from: DateTime::parse_from_rfc3339("2026-08-21T09:31:00.000000001+08:00")
                    .expect("TEST_CODE nanosecond minute"),
                to: DateTime::parse_from_rfc3339("2026-08-21T09:32:00+08:00")
                    .expect("TEST_CODE minute"),
            },
        };

        let error = benchmark_bars_with_query(&request, |_| {
            transport_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            std::future::ready(Err(unknown_benchmark_transport_failure()))
        })
        .await
        .expect_err("pure nanosecond off-grid request must stop before transport");

        assert_eq!(error.ownership(), BenchmarkGrpcOwnership::ClientBeforeSend);
        assert_eq!(error.audit_outcome(), "invalid_request");
        assert_eq!(error.reason_code(), "benchmark_minute_range_off_grid");
        assert!(!error.retryable());
        assert_eq!(transport_calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    fn br238_external_errors_preserve_retryability_but_close_unknown_reason_codes() {
        let detail = ErrorDetail {
            provider: Some("Sina".to_string()),
            reason_code: Some("TEST_CODE_upstream_retry".to_string()),
            retryable: Some(true),
            ..Default::default()
        };
        let query = map_external_query_error(
            Operation::SecurityMetadata,
            &GrpcError::Internal {
                details: Box::new(detail.clone()),
            },
        );
        assert_eq!(query.provider(), Some(ProviderId::Sina));
        assert_eq!(query.reason_code(), "internal");
        assert!(query.retryable());
        assert!(!query.message().contains("TEST_CODE_upstream_retry"));

        let connection = map_external_connection_error(GrpcError::Internal {
            details: Box::new(detail),
        });
        assert_eq!(connection.provider(), Some(ProviderId::Sina));
        assert_eq!(connection.reason_code(), "internal");
        assert!(connection.retryable());
        assert!(!connection.message().contains("TEST_CODE_upstream_retry"));
    }

    #[test]
    fn br238_documented_provider_failure_reasons_survive_gateway_mapping() {
        for reason_code in [
            "provider_authentication_rejected",
            "provider_rate_limited",
            "provider_unavailable",
            "external_query_rejected",
            "provider_response_invalid",
        ] {
            let error = map_external_query_error(
                Operation::GlobalNews,
                &GrpcError::Internal {
                    details: Box::new(ErrorDetail {
                        provider: Some("Cailianpress".to_owned()),
                        reason_code: Some(reason_code.to_owned()),
                        retryable: Some(false),
                        ..Default::default()
                    }),
                },
            );
            assert_eq!(error.provider(), Some(ProviderId::Cailianpress));
            assert_eq!(error.reason_code(), reason_code);
            assert!(!error.retryable());
        }
    }

    #[test]
    fn p01_limit_pools_request_is_exact_upper_date_and_whole_pool_bound() {
        let date = NaiveDate::from_ymd_opt(2026, 8, 18).expect("TEST_CODE date");
        assert_eq!(
            limit_pools_request(date),
            serde_json::json!({
                "kind": "Upper",
                "trading_date": "2026-08-18",
                "limit": 200,
            })
        );
    }

    #[test]
    fn br238_static_readiness_accepts_one_admitted_runtime_alias_per_family() {
        let admitted = |operation: Operation| pb::Capability {
            operation: operation as i32,
            repository_admission: AdmissionState::Admitted as i32,
            runtime_available: true,
            provider: "TEST_CODE_provider".to_string(),
            exact_scope: "TEST_CODE_scope".to_string(),
            blocker: String::new(),
            diagnostic_available: false,
        };
        let mut capabilities = [
            Operation::SecurityMetadata,
            Operation::InstrumentNews,
            Operation::GlobalNews,
            Operation::Announcements,
            Operation::BoardMemberships,
            Operation::LimitPools,
        ]
        .into_iter()
        .map(admitted)
        .collect::<Vec<_>>();
        require_external_static_capabilities(&capabilities)
            .expect("one admitted runtime alias per static family is ready");

        capabilities.retain(|row| row.operation != Operation::InstrumentNews as i32);
        let error = require_external_static_capabilities(&capabilities)
            .expect_err("a missing semantic family must fail closed");
        assert_eq!(error.reason_code(), "external_capability_missing");
        assert!(!error.retryable());

        let mut diagnostic_only = admitted(Operation::InstrumentNews);
        diagnostic_only.repository_admission = AdmissionState::Unadmitted as i32;
        diagnostic_only.diagnostic_available = true;
        capabilities.push(diagnostic_only);
        let error = require_external_static_capabilities(&capabilities)
            .expect_err("an unadmitted diagnostic row cannot authorize production");
        assert_eq!(error.reason_code(), "external_capability_unadmitted");
        assert!(!error.retryable());
    }

    #[test]
    fn br238_live_readiness_requires_all_three_live_semantic_families() {
        let admitted = |operation: Operation| pb::Capability {
            operation: operation as i32,
            repository_admission: AdmissionState::Admitted as i32,
            runtime_available: true,
            provider: "TEST_CODE_provider".to_string(),
            exact_scope: "TEST_CODE_scope".to_string(),
            blocker: String::new(),
            diagnostic_available: false,
        };
        let mut capabilities = [
            Operation::RealtimeQuotes,
            Operation::OrderBooks,
            Operation::T0Evidence,
        ]
        .into_iter()
        .map(admitted)
        .collect::<Vec<_>>();
        require_external_live_capabilities(&capabilities)
            .expect("all three admitted runtime live families are ready");

        capabilities.retain(|row| row.operation != Operation::T0Evidence as i32);
        let error = require_external_live_capabilities(&capabilities)
            .expect_err("missing T0 evidence authority must remain fail-closed");
        assert_eq!(error.reason_code(), "external_capability_missing");
        assert!(!error.retryable());
    }

    #[test]
    fn br238_live_wrappers_require_the_exact_requested_instrument_set() {
        let requested = vec!["TEST_CODE_600396".to_string()];
        require_exact_response_codes(
            "RealtimeMarketQuotes",
            &requested,
            &["TEST_CODE_600396".to_string()],
            ProviderId::Tencent,
        )
        .expect("one exact response identity is admitted");

        for returned in [
            vec![],
            vec!["TEST_CODE_000001".to_string()],
            vec![
                "TEST_CODE_600396".to_string(),
                "TEST_CODE_600396".to_string(),
            ],
        ] {
            let error = require_exact_response_codes(
                "RealtimeMarketQuotes",
                &requested,
                &returned,
                ProviderId::Tencent,
            )
            .expect_err("missing, extra or duplicate live identities fail closed");
            assert_eq!(error.reason_code(), "invalid_evidence");
            assert!(!error.retryable());
        }
    }

    #[test]
    fn br238_opening_route_keeps_verified_empty_distinct_from_required_records() {
        let batch = GatewayBatch::<u8>::VerifiedEmpty(crate::data_gateway::BatchEvidence {
            provider: ProviderId::Sina,
            source: "TEST_CODE_source".to_string(),
            source_at: None,
            observed_at: "2026-08-17T09:20:01+08:00".to_string(),
            batch_id: "TEST_CODE_opening_batch".to_string(),
        });

        let optional = opening_route("InstrumentNews", "ExternalV1", &batch, false)
            .expect("verified-empty is valid for bounded news");
        assert_eq!(optional.records, 0);
        assert_eq!(optional.source_at, None);

        let error = opening_route("RealtimeQuotes", "LocalBridgeV1", &batch, true)
            .expect_err("live quote canary requires an exact real record");
        assert_eq!(error.reason_code(), "opening_canary_empty");
        assert!(error.retryable());
    }

    #[test]
    fn br238_diagnostic_report_retains_a_failure_and_later_success() {
        let mut report = OpeningDiagnosticReport::default();
        report.record_failure(
            "InstrumentNews",
            GatewayError::invalid_evidence(
                "InstrumentNews",
                Some(ProviderId::Sina),
                "TEST_CODE cutoff-empty failure",
            ),
        );
        report.record_ready(OpeningRouteReadiness {
            route: "Announcements",
            profile: "LocalBridgeV1",
            provider: ProviderId::Cninfo,
            source: "TEST_CODE_source".to_owned(),
            source_at: None,
            observed_at: "2026-08-20T09:00:00+08:00".to_owned(),
            batch_id: "TEST_CODE_batch".to_owned(),
            records: 0,
        });

        assert_eq!(
            report.attempted_route_names(),
            "InstrumentNews,Announcements"
        );
        assert_eq!(report.failed_route_names(), "InstrumentNews");
        assert_eq!(report.ready_routes().len(), 1);
        assert_eq!(report.failures().len(), 1);
        assert_eq!(report.failures()[0].capability, "InstrumentNews");
        assert!(!report.production_ready());
    }

    #[test]
    fn br246_diagnostic_audit_persists_every_attempt_without_fabricating_failure_evidence() {
        let _env = test_grpc_env_guard();
        use diesel::prelude::*;
        use diesel::sql_types::{BigInt, Integer, Nullable, Text};

        #[derive(QueryableByName)]
        struct PersistedAttempt {
            #[diesel(sql_type = Text)]
            capability: String,
            #[diesel(sql_type = Text)]
            provider: String,
            #[diesel(sql_type = Nullable<Text>)]
            source_at: Option<String>,
            #[diesel(sql_type = Nullable<Text>)]
            batch_id: Option<String>,
            #[diesel(sql_type = BigInt)]
            accepted_count: i64,
            #[diesel(sql_type = BigInt)]
            rejected_count: i64,
            #[diesel(sql_type = Text)]
            reason_code: String,
            #[diesel(sql_type = Integer)]
            retryable: i32,
        }

        crate::database::DatabaseManager::init(None).expect("TEST_CODE audit database");
        let mut report = OpeningDiagnosticReport::default();
        for (route, provider) in [
            ("SecurityMetadata", ProviderId::Sina),
            ("GlobalNews-Eastmoney", ProviderId::Eastmoney),
            ("GlobalNews-CLS", ProviderId::Cailianpress),
            ("GlobalNews-Jin10", ProviderId::Jin10),
            ("GlobalNews-ThePaper", ProviderId::ThePaper),
            ("Announcements", ProviderId::Cninfo),
            ("BoardConstituents", ProviderId::Eastmoney),
            ("LimitPools", ProviderId::Eastmoney),
        ] {
            report.record_ready(br238_ready_route(route, provider));
        }
        report.record_failure(
            "InstrumentNews",
            GatewayError::classified(
                "InstrumentNews",
                Some(ProviderId::Sina),
                "unavailable",
                "instrument_cutoff_empty",
                true,
                "TEST_CODE upstream detail must not become audit evidence",
            ),
        );

        audit_opening_diagnostic_report("TEST_CODE_OpeningStaticResident", &report)
            .expect("every diagnostic attempt must be immutably audited");

        let mut connection = crate::database::DatabaseManager::get()
            .get_conn()
            .expect("TEST_CODE audit connection");
        let rows = diesel::sql_query(
            "SELECT capability, provider, source_at, batch_id, accepted_count, \
                    rejected_count, reason_code, retryable \
             FROM data_acquisition_audit \
             WHERE capability LIKE 'TEST_CODE_OpeningStaticResident-%' \
             ORDER BY id ASC",
        )
        .load::<PersistedAttempt>(&mut connection)
        .expect("TEST_CODE diagnostic audit rows");

        assert_eq!(rows.len(), 9);
        assert_eq!(
            rows.iter()
                .map(|row| row.capability.as_str())
                .collect::<Vec<_>>(),
            vec![
                "TEST_CODE_OpeningStaticResident-SecurityMetadata",
                "TEST_CODE_OpeningStaticResident-GlobalNews-Eastmoney",
                "TEST_CODE_OpeningStaticResident-GlobalNews-CLS",
                "TEST_CODE_OpeningStaticResident-GlobalNews-Jin10",
                "TEST_CODE_OpeningStaticResident-GlobalNews-ThePaper",
                "TEST_CODE_OpeningStaticResident-Announcements",
                "TEST_CODE_OpeningStaticResident-BoardConstituents",
                "TEST_CODE_OpeningStaticResident-LimitPools",
                "TEST_CODE_OpeningStaticResident-InstrumentNews",
            ]
        );
        assert_eq!(rows.iter().filter(|row| row.accepted_count == 1).count(), 8);
        let failed = rows.last().expect("InstrumentNews failure row");
        assert_eq!(failed.provider, "Sina");
        assert_eq!(failed.source_at, None);
        assert_eq!(failed.batch_id, None);
        assert_eq!(failed.accepted_count, 0);
        assert_eq!(failed.rejected_count, 1);
        assert_eq!(failed.reason_code, "instrument_cutoff_empty");
        assert_eq!(failed.retryable, 1);
    }

    #[test]
    fn br246_diagnostic_audit_rejects_record_count_overflow() {
        let mut report = OpeningDiagnosticReport::default();
        let mut route = br238_ready_route("SecurityMetadata", ProviderId::Sina);
        route.records = usize::MAX;
        report.record_ready(route);

        let error = opening_diagnostic_audit_rows("TEST_CODE_OpeningStaticResident", &report)
            .expect_err("SQLite INTEGER overflow must fail instead of truncating");

        assert_eq!(error.audit_outcome(), "unavailable");
        assert_eq!(error.reason_code(), "acquisition_audit_unavailable");
        assert!(error.retryable());
        assert!(error.message().contains("accepted_count_overflow"));
    }

    #[test]
    fn br238_diagnostic_report_applies_the_existing_global_news_quorum() {
        let mut report = OpeningDiagnosticReport::default();
        for (route, provider) in [
            ("SecurityMetadata", ProviderId::Sina),
            ("InstrumentNews", ProviderId::Sina),
            ("GlobalNews-Eastmoney", ProviderId::Eastmoney),
            ("GlobalNews-CLS", ProviderId::Cailianpress),
            ("Announcements", ProviderId::Cninfo),
            ("BoardConstituents", ProviderId::Eastmoney),
            ("LimitPools", ProviderId::Eastmoney),
        ] {
            report.record_ready(br238_ready_route(route, provider));
        }
        for (route, provider) in [
            ("GlobalNews-Jin10", None),
            ("GlobalNews-ThePaper", Some(ProviderId::ThePaper)),
        ] {
            report.record_failure(
                route,
                GatewayError::classified(
                    "GlobalNews",
                    provider,
                    "unavailable",
                    "provider_unavailable",
                    true,
                    "TEST_CODE provider unavailable",
                ),
            );
        }

        assert_eq!(report.ready_routes().len(), 7);
        assert_eq!(report.failures().len(), 2);
        assert!(report.production_ready());
    }

    #[test]
    fn br238_instrument_news_request_reuses_one_captured_upper_bound() {
        let instrument = InstrumentId::new(
            crate::magic_compat::Exchange::Shanghai,
            "TEST_CODE_600396",
            crate::magic_compat::AssetClass::Equity,
        )
        .expect("TEST_CODE canonical instrument");
        let start = NaiveDate::from_ymd_opt(2026, 8, 18).expect("TEST_CODE start");
        let end = NaiveDate::from_ymd_opt(2026, 8, 19).expect("TEST_CODE end");
        let captured = DateTime::parse_from_rfc3339("2026-08-19T16:15:37.125+08:00")
            .expect("TEST_CODE captured clock");

        let params = instrument_news_request_params(&instrument, start, end, 20, captured);

        assert_eq!(params["captured_through"], "2026-08-19T16:15:37.125+08:00");
        assert_eq!(params["start"], "2026-08-18");
        assert_eq!(params["end"], "2026-08-19");
        assert_eq!(params["limit"], 20);
    }

    #[test]
    fn br238_opening_route_accepts_valid_fractional_unix_evidence_time() {
        let batch = GatewayBatch::<u8>::VerifiedEmpty(crate::data_gateway::BatchEvidence {
            provider: ProviderId::Sina,
            source: "TEST_CODE_source".to_string(),
            source_at: Some("1786931999.125000000".to_string()),
            observed_at: "1786932000.250000000".to_string(),
            batch_id: "TEST_CODE_fractional_unix".to_string(),
        });
        opening_route("InstrumentNews", "ExternalV1", &batch, false)
            .expect("fractional Unix evidence accepted by the production converter stays valid");
    }

    #[test]
    fn br238_opening_route_accepts_validated_jin10_provider_source_time() {
        let batch = GatewayBatch::<u8>::VerifiedEmpty(crate::data_gateway::BatchEvidence {
            provider: ProviderId::Jin10,
            source: GlobalNewsProvider::Jin10.source().to_owned(),
            source_at: Some("2026-08-19 22:41:08".to_owned()),
            observed_at: "1787150469.000000000".to_owned(),
            batch_id: "TEST_CODE_jin10_opening_batch".to_owned(),
        });

        opening_route("GlobalNews-Jin10", "ExternalV1", &batch, false)
            .expect("provider-specific source time was already admitted by the converter");
    }

    #[test]
    fn br238_limit_pools_readiness_keeps_the_exact_route_identity() {
        let batch =
            GatewayBatch::<LimitPoolEntry>::VerifiedEmpty(crate::data_gateway::BatchEvidence {
                provider: ProviderId::Tonghuashun,
                source: "TEST_CODE_ths_limit_pool".to_string(),
                source_at: Some("2026-08-14".to_string()),
                observed_at: "1786970635.386291000".to_string(),
                batch_id: "TEST_CODE_limit_pool_batch".to_string(),
            });

        let expected = NaiveDate::from_ymd_opt(2026, 8, 14).expect("TEST_CODE date");
        let route = opening_limit_pools_route(&batch, expected)
            .expect("strict converter evidence remains the exact BR-238 route");
        assert_eq!(route.route, "LimitPools");
        assert_eq!(route.profile, "LocalBridgeV1");
        assert_eq!(route.source_at.as_deref(), Some("2026-08-14"));

        let error = opening_limit_pools_route(
            &batch,
            NaiveDate::from_ymd_opt(2026, 8, 13).expect("TEST_CODE mismatched date"),
        )
        .expect_err("date-only evidence for a different request must fail closed");
        assert_eq!(error.reason_code(), "invalid_evidence");
        assert!(!error.retryable());
    }

    #[test]
    fn br238_opening_report_has_stable_route_order() {
        let report = OpeningReadinessReport {
            routes: vec![
                OpeningRouteReadiness {
                    route: "SecurityMetadata",
                    profile: "ExternalV1",
                    provider: ProviderId::Sina,
                    source: "TEST_CODE_source".to_string(),
                    source_at: None,
                    observed_at: "2026-08-17T09:20:01+08:00".to_string(),
                    batch_id: "TEST_CODE_batch_1".to_string(),
                    records: 1,
                },
                OpeningRouteReadiness {
                    route: "RealtimeQuotes",
                    profile: "LocalBridgeV1",
                    provider: ProviderId::Tencent,
                    source: "TEST_CODE_source".to_string(),
                    source_at: Some("2026-08-17T09:20:00+08:00".to_string()),
                    observed_at: "2026-08-17T09:20:01+08:00".to_string(),
                    batch_id: "TEST_CODE_batch_2".to_string(),
                    records: 1,
                },
            ],
            degraded_routes: vec![OpeningRouteFailure {
                route: "GlobalNews-Jin10",
                provider: ProviderId::Jin10,
                reason_code: "invalid_evidence",
                retryable: false,
                message: "TEST_CODE invalid evidence".to_string(),
            }],
        };
        assert_eq!(report.route_names(), "SecurityMetadata,RealtimeQuotes");
        assert_eq!(report.degraded_route_names(), "GlobalNews-Jin10");
    }

    /// D2 wire round-trip: proto ErrorDetail → GrpcError::details() → map_query_error
    /// 全链路保真 (与 grpc_server::handlers.rs Fetch 分支编码端对应)。
    #[test]
    fn map_query_error_roundtrip_from_status_detail() {
        let pb_detail = pb::ErrorDetail {
            request_id: "req-7".to_string(),
            operation: Operation::OutcomeDailyBars as i32,
            provider: "Tdx".to_string(),
            reason_code: "no_verified_batch".to_string(),
            retryable: true,
            ..Default::default()
        };
        let status = tonic::Status::with_details(
            tonic::Code::Internal,
            "取数失败",
            pb_detail.encode_to_vec().into(),
        );
        let g = map_query_error(Operation::OutcomeDailyBars, &GrpcError::from(status));
        assert_eq!(g.provider(), Some(ProviderId::Tdx));
        assert_eq!(g.reason_code(), "no_verified_batch");
        assert!(g.retryable());
    }

    #[test]
    fn bridge_disabled_without_env() {
        let _env = test_grpc_env_guard();
        std::env::remove_var("DATA_GATEWAY_GRPC");
        std::env::remove_var("DATA_GATEWAY_GRPC_DISABLED");
        std::env::remove_var("GRPC_MARKET_ADDR");
        assert!(bridge_for("RealtimeQuotes").unwrap().is_none());
    }

    #[test]
    fn grpc_env_guard_reset_bridge_recovers_from_poisoned_cache() {
        let guard = test_grpc_env_guard();
        let cell = SOURCE.get_or_init(|| Mutex::new(None));
        let poisoned = std::panic::catch_unwind(|| {
            let _cache = cell
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            panic!("TEST_CODE poison bridge cache");
        });
        assert!(poisoned.is_err());

        reset_bridge();
        assert!(cell
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_none());
        drop(guard);

        let reacquired = test_grpc_env_guard();
        assert!(SOURCE
            .get()
            .expect("TEST_CODE bridge cache initialized")
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_none());
        drop(reacquired);
    }

    #[test]
    fn grpc_env_guard_restores_env_and_cache_after_panic_and_reacquires() {
        const KEYS: &[&str] = &[
            "DATA_GATEWAY_GRPC",
            "DATA_GATEWAY_GRPC_DISABLED",
            "GRPC_MARKET_ADDR",
            "GRPC_MARKET_CLIENT_BUNDLE",
        ];
        let baseline_guard = test_grpc_env_guard();
        let before: Vec<_> = KEYS
            .iter()
            .map(|key| (*key, std::env::var_os(key)))
            .collect();
        drop(baseline_guard);
        let unwound = std::panic::catch_unwind(|| {
            let _env = test_grpc_env_guard();
            std::env::set_var("DATA_GATEWAY_GRPC", "1");
            std::env::set_var("DATA_GATEWAY_GRPC_DISABLED", "TEST_CODE_disabled");
            std::env::set_var("GRPC_MARKET_ADDR", "http://TEST_CODE_changed:1");
            std::env::set_var("GRPC_MARKET_CLIENT_BUNDLE", "/TEST_CODE_changed_bundle");
            let _ = bridge_for("RealtimeQuotes").expect("TEST_CODE configured bridge");
            let cell = SOURCE.get().expect("TEST_CODE bridge cache initialized");
            let _cache = cell
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            panic!("TEST_CODE unwind through env/cache guard");
        });
        assert!(unwound.is_err());

        let reacquired = test_grpc_env_guard();
        for (key, expected) in &before {
            assert_eq!(std::env::var_os(key), expected.clone(), "env key {key}");
        }
        assert!(SOURCE
            .get()
            .expect("TEST_CODE bridge cache initialized")
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_none());
        drop(reacquired);
    }

    #[test]
    fn bridge_disabled_by_op_name() {
        let _env = test_grpc_env_guard();
        std::env::set_var("DATA_GATEWAY_GRPC", "1");
        std::env::set_var("DATA_GATEWAY_GRPC_DISABLED", "RealtimeQuotes");
        std::env::remove_var("GRPC_MARKET_ADDR");
        assert!(
            bridge_for("RealtimeQuotes").unwrap().is_none(),
            "DISABLED 命中 → library"
        );
        std::env::remove_var("DATA_GATEWAY_GRPC");
        std::env::remove_var("DATA_GATEWAY_GRPC_DISABLED");
    }

    #[test]
    fn bridge_enabled_but_unreachable_is_fail_closed() {
        // 连接是惰性的: bridge_for 只注册实例, fail-closed 在方法层
        // (首个查询 ensure_connected 失败 → unavailable retryable)。
        let _env = test_grpc_env_guard();
        std::env::set_var("DATA_GATEWAY_GRPC", "1");
        std::env::set_var("GRPC_MARKET_ADDR", "http://127.0.0.1:1");
        reset_bridge();
        let bridge = bridge_for("RealtimeQuotes")
            .unwrap()
            .expect("bridge 实例存在");
        let err = bridge.realtime_quotes(&["600519".to_string()]).unwrap_err();
        assert!(err.retryable(), "服务端不可达必须 retryable");
        std::env::remove_var("DATA_GATEWAY_GRPC");
        std::env::remove_var("GRPC_MARKET_ADDR");
        reset_bridge();
    }

    #[tokio::test]
    async fn sync_method_from_async_worker_does_not_panic() {
        // 生产事故回归 (2026-08-15 21:07 主线程 panic 杀进程): monitor 同步
        // 网关调用直接在 async worker 上发生, 旧判别器 Handle::block_on 触发
        // tokio "Cannot start a runtime from within a runtime"。修复后 async
        // worker 走独立 std 线程路径 → 服务端不可达返回 Err 而非 panic。
        let _env = test_grpc_env_guard();
        std::env::set_var("DATA_GATEWAY_GRPC", "1");
        std::env::set_var("GRPC_MARKET_ADDR", "http://127.0.0.1:1");
        reset_bridge();
        let bridge = bridge_for("RealtimeQuotes")
            .unwrap()
            .expect("bridge 实例存在");
        let err = bridge.realtime_quotes(&["600519".to_string()]).unwrap_err();
        assert!(
            err.retryable(),
            "async worker 路径也必须 fail-closed retryable"
        );
        std::env::remove_var("DATA_GATEWAY_GRPC");
        std::env::remove_var("GRPC_MARKET_ADDR");
        reset_bridge();
    }

    #[test]
    fn startup_banner_defaults_to_library() {
        let _env = test_grpc_env_guard();
        std::env::remove_var("DATA_GATEWAY_GRPC");
        std::env::remove_var("DATA_GATEWAY_GRPC_DISABLED");
        std::env::remove_var("GRPC_MARKET_ADDR");
        let b = startup_banner();
        assert!(
            b.contains("数据源模式 = library"),
            "默认必须 library (v15.x 出声): {b}"
        );
        assert!(
            b.contains("server = http://127.0.0.1:18082"),
            "默认地址: {b}"
        );
        assert!(b.contains("禁用 = 无"), "无禁用: {b}");
        assert!(b.contains("保持本地 1 ops"), "keep-local 计数: {b}");
    }

    #[test]
    fn startup_banner_grpc_mode_and_disabled() {
        let _env = test_grpc_env_guard();
        std::env::set_var("DATA_GATEWAY_GRPC", "1");
        std::env::set_var("GRPC_MARKET_ADDR", "http://127.0.0.1:19001");
        std::env::set_var("DATA_GATEWAY_GRPC_DISABLED", "T0Evidence,InstrumentNews");
        let b = startup_banner();
        assert!(b.contains("数据源模式 = grpc"), "grpc 模式: {b}");
        assert!(
            b.contains("server = http://127.0.0.1:19001"),
            "显式地址: {b}"
        );
        assert!(
            b.contains("禁用 = T0Evidence,InstrumentNews"),
            "禁用列表: {b}"
        );
        assert!(b.contains("保持本地 1 ops"), "keep-local 计数: {b}");
        assert!(
            b.contains("chain_batch op 61"),
            "M4c keep-local 原因出声 (op 61 消费): {b}"
        );
        std::env::remove_var("DATA_GATEWAY_GRPC");
        std::env::remove_var("DATA_GATEWAY_GRPC_DISABLED");
        std::env::remove_var("GRPC_MARKET_ADDR");
    }

    #[test]
    fn br231_external_bundle_is_captured_but_never_printed() {
        let _env = test_grpc_env_guard();
        let secret_marker = "/TEST_CODE_private_bundle_marker";
        std::env::set_var("DATA_GATEWAY_GRPC", "1");
        std::env::set_var("GRPC_MARKET_CLIENT_BUNDLE", secret_marker);
        reset_bridge();

        let bridge = bridge_for("SecurityMetadata")
            .expect("bridge config")
            .expect("bridge enabled");
        assert_eq!(
            bridge.external_bundle.as_deref(),
            Some(std::path::Path::new(secret_marker))
        );
        let banner = startup_banner();
        assert!(banner.contains("external-v1 = configured"), "{banner}");
        assert!(
            !banner.contains(secret_marker),
            "bundle path must stay secret-safe: {banner}"
        );

        std::env::remove_var("DATA_GATEWAY_GRPC");
        std::env::remove_var("GRPC_MARKET_CLIENT_BUNDLE");
        reset_bridge();
    }

    #[test]
    fn br231_security_identity_requires_external_bundle() {
        let _env = test_grpc_env_guard();
        std::env::set_var("DATA_GATEWAY_GRPC", "1");
        std::env::remove_var("GRPC_MARKET_CLIENT_BUNDLE");
        reset_bridge();

        let bridge = bridge_for("SecurityMetadata")
            .expect("bridge config")
            .expect("bridge enabled");
        let error = block_on(bridge.ensure_external_connected(Operation::SecurityMetadata))
            .expect_err("identity must not fall back to the local bridge contract");
        assert_eq!(error.capability(), "GrpcExternalV1");
        assert_eq!(error.reason_code(), "external_bundle_unconfigured");
        assert!(!error.retryable());

        std::env::remove_var("DATA_GATEWAY_GRPC");
        reset_bridge();
    }

    #[test]
    fn br238_configured_global_news_never_falls_back_to_local_bridge() {
        let _env = test_grpc_env_guard();
        std::env::set_var("DATA_GATEWAY_GRPC", "1");
        std::env::set_var("GRPC_MARKET_ADDR", "http://127.0.0.1:1");
        std::env::set_var(
            "GRPC_MARKET_CLIENT_BUNDLE",
            "/TEST_CODE_missing_global_news_bundle",
        );
        reset_bridge();

        let bridge = bridge_for("GlobalNews")
            .expect("bridge config")
            .expect("bridge enabled");
        let error = block_on(bridge.global_news_async(GlobalNewsProvider::Jin10, 1))
            .expect_err("configured ExternalV1 failure must not fall back to LocalBridgeV1");
        assert_eq!(error.capability(), "GrpcExternalV1");
        assert_eq!(error.reason_code(), "external_bundle_invalid");
        assert!(!error.retryable());

        std::env::remove_var("DATA_GATEWAY_GRPC");
        std::env::remove_var("GRPC_MARKET_ADDR");
        std::env::remove_var("GRPC_MARKET_CLIENT_BUNDLE");
        reset_bridge();
    }

    #[test]
    fn br238_opening_readiness_requires_external_bundle() {
        let _env = test_grpc_env_guard();
        std::env::set_var("DATA_GATEWAY_GRPC", "1");
        std::env::remove_var("GRPC_MARKET_CLIENT_BUNDLE");
        reset_bridge();

        let error = block_on(external_static_opening_readiness())
            .expect_err("opening readiness must fail before producer startup");
        assert_eq!(error.capability(), "GrpcExternalV1");
        assert_eq!(error.reason_code(), "external_bundle_unconfigured");

        std::env::remove_var("DATA_GATEWAY_GRPC");
        reset_bridge();
    }

    #[test]
    fn br238_global_news_quorum_requires_two_independent_verified_providers() {
        assert!(require_global_news_quorum(2, &[]).is_ok());
        assert!(require_global_news_quorum(4, &[]).is_ok());

        let retryable_failure = OpeningRouteFailure {
            route: "GlobalNews-CLS",
            provider: ProviderId::Cailianpress,
            reason_code: "upstream_unavailable",
            retryable: true,
            message: "TEST_CODE provider unavailable".to_string(),
        };

        let retryable = require_global_news_quorum(1, std::slice::from_ref(&retryable_failure))
            .expect_err("one verified provider cannot authorize the news chain");
        assert_eq!(retryable.reason_code(), "global_news_quorum_unavailable");
        assert!(retryable.retryable());
        assert!(retryable.message().contains("GlobalNews-CLS"));

        let deterministic_failure = OpeningRouteFailure {
            route: "GlobalNews-Jin10",
            provider: ProviderId::Jin10,
            reason_code: "invalid_evidence",
            retryable: false,
            message: "TEST_CODE invalid evidence".to_string(),
        };
        let deterministic =
            require_global_news_quorum(1, std::slice::from_ref(&deterministic_failure))
                .expect_err("deterministic provider failures still fail closed below quorum");
        assert_eq!(
            deterministic.reason_code(),
            "global_news_quorum_unavailable"
        );
        assert!(!deterministic.retryable());
    }

    fn br238_ready_route(route: &'static str, provider: ProviderId) -> OpeningRouteReadiness {
        OpeningRouteReadiness {
            route,
            profile: if matches!(route, "SecurityMetadata" | "InstrumentNews")
                || route.starts_with("GlobalNews-")
            {
                "ExternalV1"
            } else {
                "LocalBridgeV1"
            },
            provider,
            source: "TEST_CODE_source".to_owned(),
            source_at: Some("2026-08-18".to_owned()),
            observed_at: "2026-08-19T09:00:00+08:00".to_owned(),
            batch_id: format!("TEST_CODE_{route}"),
            records: 1,
        }
    }

    fn br238_failure(route: &'static str, provider: ProviderId) -> OpeningRouteFailure {
        OpeningRouteFailure {
            route,
            provider,
            reason_code: "invalid_evidence",
            retryable: false,
            message: format!("TEST_CODE {route} rejected"),
        }
    }

    #[test]
    fn br238_static_gate_requires_exact_limit_pools_route_and_all_nine_attempts() {
        let routes = vec![
            br238_ready_route("SecurityMetadata", ProviderId::Tencent),
            br238_ready_route("InstrumentNews", ProviderId::Sina),
            br238_ready_route("GlobalNews-CLS", ProviderId::Cailianpress),
            br238_ready_route("GlobalNews-ThePaper", ProviderId::ThePaper),
            br238_ready_route("Announcements", ProviderId::Cninfo),
            br238_ready_route("BoardConstituents", ProviderId::Tdx),
            br238_ready_route("LimitPools", ProviderId::Tonghuashun),
        ];
        let degraded_routes = vec![
            br238_failure("GlobalNews-Eastmoney", ProviderId::Eastmoney),
            br238_failure("GlobalNews-Jin10", ProviderId::Jin10),
        ];
        let report = OpeningReadinessReport {
            routes,
            degraded_routes,
        };
        require_external_static_readiness(&report)
            .expect("five exact non-news routes plus two news providers must be ready");

        let mut legacy = report.clone();
        legacy
            .routes
            .iter_mut()
            .find(|route| route.route == "LimitPools")
            .expect("LimitPools route")
            .route = "UpperLimitPoolReview";
        let error = require_external_static_readiness(&legacy)
            .expect_err("legacy UpperLimitPoolReview must never authorize BR-238");
        assert_eq!(error.reason_code(), "opening_static_attempt_set_invalid");
        assert!(!error.retryable());
    }

    #[test]
    fn br231_undelivered_external_contract_is_rejected_before_connection() {
        let _env = test_grpc_env_guard();
        std::env::set_var("DATA_GATEWAY_GRPC", "1");
        std::env::remove_var("GRPC_MARKET_CLIENT_BUNDLE");
        reset_bridge();

        let bridge = bridge_for("BoardConstituents")
            .expect("bridge config")
            .expect("bridge enabled");
        let error =
            block_on(bridge.query_external_op(Operation::BoardConstituents, serde_json::json!({})))
                .expect_err("undelivered contract must be rejected before I/O");
        assert_eq!(error.reason_code(), "external_contract_rejected");
        assert!(!error.retryable());

        std::env::remove_var("DATA_GATEWAY_GRPC");
        reset_bridge();
    }

    #[test]
    fn hooked_ops_disjoint_from_keep_local() {
        for op in HOOKED_OPS {
            assert!(
                !KEEP_LOCAL_OPS.contains(op),
                "{op} 同时出现在 HOOKED_OPS 和 KEEP_LOCAL_OPS — 必须只在一处"
            );
        }
    }

    #[test]
    fn hooked_ops_match_bridge_for_call_sites() {
        // Spec Evidence Rule: banner 的 HOOKED_OPS 必须与真实钩子一致, 防 rot。
        // 扫 src/data_gateway/ 下除 grpc_source.rs 外各文件的 bridge_for 调用
        // (grpc_source.rs 自身的 bridge_for 是单测不是钩子), 去重后集合断言相等。
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/data_gateway");
        let mut found: Vec<String> = Vec::new();
        let mut entries: Vec<_> = std::fs::read_dir(&dir)
            .expect("src/data_gateway 可读")
            .filter_map(|e| e.ok())
            .collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            if path.file_name().and_then(|n| n.to_str()) == Some("grpc_source.rs") {
                continue; // 定义模块: 钩子调用在其他网关文件
            }
            let text = std::fs::read_to_string(&path).expect("读网关源文件");
            for (idx, _) in text.match_indices("bridge_for(\"") {
                let rest = &text[idx + "bridge_for(\"".len()..];
                let end = rest
                    .find('"')
                    .unwrap_or_else(|| panic!("bridge_for 名称未闭合: {path:?}"));
                found.push(rest[..end].to_string());
            }
        }
        // 一个 op 可有多个钩子调用点 (day1_flows/day5_flows 等) → 去重。
        found.sort();
        found.dedup();
        let mut expected: Vec<&str> = HOOKED_OPS.to_vec();
        expected.sort();
        assert_eq!(
            found, expected,
            "HOOKED_OPS 与真实 bridge_for 调用不一致 (banner 会撒谎)。\
             改钩子时同步 const, 改 const 时同步钩子。"
        );
    }

    /// M4c wire 契约: canned JSON (与 delegate.rs fetch_chain_batch / fixture.rs
    /// fixture-cb 视图保持一致) → VisibleChainBatch 反序列化 roundtrip。
    /// 服务端 build_for_date 输出经 serde_json::to_vec 直出, 客户端 from_slice 重建 —
    /// 这个测试 pin 住双向 serde 一致 (字段改名会在这里炸)。
    #[test]
    fn chain_batch_wire_roundtrip() {
        use crate::database::chain_intelligence::VisibleChainBatch;
        let canned = r#"{"batch_id":"fixture-cb","content_hash":"h1","trading_date":"2026-08-15","calculation_version":"v1","taxonomy_version":"t1","inputs":[{"input_id":"i1","ordinal":1,"capability":"limit-up","provider":"tdx","source":"tdx","source_at":"2026-08-15T10:00:00+08:00","observed_at":"2026-08-15T10:00:00+08:00","source_batch_id":"b1","source_batch_hash":"h1","content_hash":"h1"}],"chains":[{"chain_id":"c1","canonical_board_id":"BK0475","board_name":"白酒","upper_limit_count":3,"continuous_count":2,"members":[{"instrument_id":"600519","security_name":"贵州茅台","source_event_id":"e1","streak":2}]}],"rejections":[]}"#;
        let batch: VisibleChainBatch = serde_json::from_slice(canned.as_bytes())
            .expect("canned fixture-cb JSON → VisibleChainBatch");
        assert_eq!(batch.batch_id, "fixture-cb");
        assert_eq!(
            batch.trading_date.format("%Y-%m-%d").to_string(),
            "2026-08-15"
        );
        assert_eq!(batch.chains.len(), 1);
        assert_eq!(batch.chains[0].canonical_board_id, "BK0475");
        assert_eq!(batch.chains[0].members.len(), 1);
        assert_eq!(batch.chains[0].members[0].instrument_id, "600519");
        assert_eq!(batch.chains[0].members[0].streak, 2);
        assert!(batch.rejections.is_empty());
        // 双向: 序列化回去仍可重建 (服务端 to_vec → 客户端 from_slice 往返)。
        let reencoded = serde_json::to_vec(&batch).expect("VisibleChainBatch → bytes");
        let round: VisibleChainBatch = serde_json::from_slice(&reencoded).expect("重新反序列化");
        assert_eq!(round.batch_id, batch.batch_id);
        assert_eq!(round.trading_date, batch.trading_date);
    }
}
