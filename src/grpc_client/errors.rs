//! gRPC status code → 项目错误类型 (合同 §10 错误映射表)。
//! 不依赖自然语言 message 做程序分支; ErrorDetail 从 status details 解码。
//! D2 (错误分类保真): 所有变体携带 details — 服务端 Fetch 失败时
//! handlers.rs 附加 ErrorDetail (provider/reason_code/retryable), 客户端
//! 桥据此重建 GatewayError 分类 (grpc_source.rs query_op), 不再折叠为
//! 默认 unavailable+provider=None (BR-170 生产日志 pre-fix 形态)。
use prost::Message; // ErrorDetail::decode (tonic 0.14 details() 返回 &[u8])
use sha2::{Digest, Sha256};

pub use crate::grpc_client::provider_attempts::{
    ProviderAttempt, ProviderAttemptText, ProviderAttemptValue, ProviderAttempts,
};
use crate::grpc_client::provider_attempts::ExternalProviderCatalog;
use crate::grpc_contract::methods::{ContractProfile, ExternalMethod, LocalMethod, MethodIdentity};

#[derive(Debug, thiserror::Error, Clone, PartialEq)]
pub enum GrpcError {
    #[error("请求参数错误 (不重试)")]
    InvalidArgument { details: Box<ErrorDetail> },
    #[error("认证失败 (刷新凭据)")]
    Unauthenticated { details: Box<ErrorDetail> },
    #[error("无权限调用该能力 (停止调用)")]
    PermissionDenied { details: Box<ErrorDetail> },
    #[error("能力未准入或不支持 (不重试)")]
    Unimplemented { details: Box<ErrorDetail> },
    #[error("资源受限 (退避; 流消费者记录 gap)")]
    ResourceExhausted { details: Box<ErrorDetail> },
    #[error("超时 (有界重试, 保留原 request_id)")]
    DeadlineExceeded { details: Box<ErrorDetail> },
    #[error("服务不可用 (指数退避, 重新检查 health/capabilities)")]
    Unavailable { details: Box<ErrorDetail> },
    #[error("数据完整性/连续性失败 (不能当空成功)")]
    FailedPrecondition { details: Box<ErrorDetail> },
    #[error("服务端内部错误 (记录 request_id, 停止无界重试)")]
    Internal { details: Box<ErrorDetail> },
    #[error("未知错误 (code={code})", code = details.code)]
    Unknown { details: Box<ErrorDetail> },
}

impl GrpcError {
    /// D2: 解码后的 ErrorDetail (所有变体必带; 无 details 的 status → 默认值)。
    /// 桥 (grpc_source.rs) 用 e.details().provider/reason_code/retryable 重建 GatewayError。
    pub fn details(&self) -> &ErrorDetail {
        match self {
            GrpcError::InvalidArgument { details }
            | GrpcError::Unauthenticated { details }
            | GrpcError::PermissionDenied { details }
            | GrpcError::Unimplemented { details }
            | GrpcError::ResourceExhausted { details }
            | GrpcError::DeadlineExceeded { details }
            | GrpcError::Unavailable { details }
            | GrpcError::FailedPrecondition { details }
            | GrpcError::Internal { details }
            | GrpcError::Unknown { details } => details,
        }
    }

    pub(crate) fn safe_diagnostic(&self) -> Option<&str> {
        self.details()
            .diagnostic_message
            .as_deref()
            .map(DiagnosticMessage::as_str)
    }

    pub(crate) fn from_status(status: tonic::Status, context: StatusErrorContext<'_>) -> Self {
        let diagnostic_message = safe_status_message(status.message());
        grpc_error_from_parts(
            status.code(),
            decode_status_error_detail(&status, context),
            diagnostic_message,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ErrorDetail {
    pub code: String,
    pub request_id: Option<String>,
    pub method: Option<MethodIdentity>,
    pub provider: Option<String>,
    pub reason_code: Option<String>,
    pub retryable: Option<bool>,
    pub admission: Option<crate::grpc_client::pb::magic::market::v1::AdmissionState>,
    pub evidence_code: Option<SafeEvidenceIdentifier>,
    pub evidence_field: Option<SafeEvidenceIdentifier>,
    pub record_index: Option<u32>,
    pub provider_attempts: ProviderAttempts,
    /// Bounded, secret-screened server diagnostic for operator evidence only.
    /// Program flow must continue to branch exclusively on typed fields above.
    pub diagnostic_message: Option<Box<DiagnosticMessage>>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct SafeEvidenceIdentifier(String);

impl SafeEvidenceIdentifier {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for SafeEvidenceIdentifier {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SafeEvidenceIdentifier([redacted])")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiagnosticMessage(String);

impl DiagnosticMessage {
    fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

const MAX_DIAGNOSTIC_CHARS: usize = 512;
const REQUEST_ID_HASH_DOMAIN: &[u8] = b"stock_analysis.grpc_error.request_id.v1";
const ERROR_DETAIL_TRAILER: &str = "magic-error-detail-bin";

#[derive(Clone, Copy)]
enum StatusErrorExpectation<'a> {
    Unchecked,
    Data {
        method: MethodIdentity,
        request_id: &'a str,
        provider_catalog: Option<&'a ExternalProviderCatalog>,
    },
    Control {
        request_id: &'a str,
    },
}

#[derive(Clone, Copy)]
pub(crate) struct StatusErrorContext<'a> {
    profile: ContractProfile,
    expectation: StatusErrorExpectation<'a>,
}

impl<'a> StatusErrorContext<'a> {
    pub(crate) const fn unchecked_local() -> Self {
        Self {
            profile: ContractProfile::LocalBridgeV1,
            expectation: StatusErrorExpectation::Unchecked,
        }
    }

    pub(crate) const fn data(method: MethodIdentity, request_id: &'a str) -> Self {
        Self {
            profile: method.profile(),
            expectation: StatusErrorExpectation::Data {
                method,
                request_id,
                provider_catalog: None,
            },
        }
    }

    pub(crate) const fn external_data(
        method: ExternalMethod,
        request_id: &'a str,
        provider_catalog: &'a ExternalProviderCatalog,
    ) -> Self {
        Self {
            profile: ContractProfile::ExternalV1,
            expectation: StatusErrorExpectation::Data {
                method: MethodIdentity::External(method),
                request_id,
                provider_catalog: Some(provider_catalog),
            },
        }
    }

    pub(crate) const fn control(profile: ContractProfile, request_id: &'a str) -> Self {
        Self {
            profile,
            expectation: StatusErrorExpectation::Control { request_id },
        }
    }

    fn accepts(self, wire: &DecodedWireErrorDetail) -> bool {
        match self.expectation {
            StatusErrorExpectation::Unchecked => true,
            StatusErrorExpectation::Data {
                method, request_id, ..
            } => {
                !request_id.is_empty()
                    && wire.request_id == request_id
                    && wire.method == Some(method)
            }
            StatusErrorExpectation::Control { request_id } => {
                !request_id.is_empty() && wire.request_id == request_id && wire.raw_operation == 0
            }
        }
    }

    fn external_provider_catalog(self) -> Option<&'a ExternalProviderCatalog> {
        match self.expectation {
            StatusErrorExpectation::Data {
                provider_catalog, ..
            } if self.profile == ContractProfile::ExternalV1 => provider_catalog,
            _ => None,
        }
    }
}

fn request_id_correlation(value: &str) -> Option<String> {
    if value.is_empty() {
        return None;
    }
    let mut digest = Sha256::new();
    digest.update((REQUEST_ID_HASH_DOMAIN.len() as u64).to_be_bytes());
    digest.update(REQUEST_ID_HASH_DOMAIN);
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value.as_bytes());
    Some(format!("sha256:{}", hex::encode(digest.finalize())))
}

fn known_provider(value: &str) -> Option<crate::market_domain::ProviderId> {
    use crate::market_domain::ProviderId;

    Some(match value {
        "Tdx" => ProviderId::Tdx,
        "Tencent" => ProviderId::Tencent,
        "Eastmoney" => ProviderId::Eastmoney,
        "Sina" => ProviderId::Sina,
        "Baostock" => ProviderId::Baostock,
        "Baidu" => ProviderId::Baidu,
        "Tonghuashun" => ProviderId::Tonghuashun,
        "Iwencai" => ProviderId::Iwencai,
        "Cninfo" => ProviderId::Cninfo,
        "Cailianpress" => ProviderId::Cailianpress,
        "Jin10" => ProviderId::Jin10,
        "ThePaper" => ProviderId::ThePaper,
        "Yonhap" => ProviderId::Yonhap,
        "WallstreetCn" => ProviderId::WallstreetCn,
        "Sse" => ProviderId::Sse,
        "Szse" => ProviderId::Szse,
        "Hkex" => ProviderId::Hkex,
        "Cffex" => ProviderId::Cffex,
        "StateCouncil" => ProviderId::StateCouncil,
        "Nbs" => ProviderId::Nbs,
        "Pbc" => ProviderId::Pbc,
        "Cfets" => ProviderId::Cfets,
        "Fred" => ProviderId::Fred,
        "Imf" => ProviderId::Imf,
        "WorldBank" => ProviderId::WorldBank,
        "SecEdgar" => ProviderId::SecEdgar,
        "XinhuaFinance" => ProviderId::XinhuaFinance,
        "Yicai" => ProviderId::Yicai,
        "SecuritiesTimes" => ProviderId::SecuritiesTimes,
        "LocalAnalysis" => ProviderId::LocalAnalysis,
        "LocalTerminal" => ProviderId::LocalTerminal,
        "Custom" => ProviderId::Custom,
        _ => return None,
    })
}

fn safe_wire_provider(value: &str) -> Option<String> {
    known_provider(value).map(|provider| format!("{provider:?}"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KnownReasonCode {
    /// 业务态: 180 天窗口无研究报告 (server 分类, BR-159 保真还原, 非错误)。
    NoCurrentReports,
    NoVerifiedBatch,
    InvalidRequest,
    InvalidEvidence,
    Unavailable,
    Partial,
    Internal,
    TdxBoardMembershipUnsupported,
    UpperLimitStreakMissing,
    ManualConfirmationContractUnavailable,
    FiveMinuteGap,
    ExactBatchJoinAccepted,
    DatabaseFailure,
    ExternalSourceFieldConflict,
    ExternalAcquisitionAuthorityMissing,
    ProviderAuthenticationRejected,
    ProviderRateLimited,
    ProviderUnavailable,
    ExternalQueryRejected,
    ProviderResponseInvalid,
}

impl KnownReasonCode {
    fn from_wire(value: &str) -> Option<Self> {
        Some(match value {
            "no_current_reports" => Self::NoCurrentReports,
            "no_verified_batch" => Self::NoVerifiedBatch,
            "invalid_request" => Self::InvalidRequest,
            "invalid_evidence" => Self::InvalidEvidence,
            "unavailable" => Self::Unavailable,
            "partial" => Self::Partial,
            "internal" => Self::Internal,
            "tdx_board_membership_unsupported" => Self::TdxBoardMembershipUnsupported,
            "upper_limit_streak_missing" => Self::UpperLimitStreakMissing,
            "manual_confirmation_contract_unavailable" => {
                Self::ManualConfirmationContractUnavailable
            }
            "five_minute_gap" => Self::FiveMinuteGap,
            "exact_batch_join_accepted" => Self::ExactBatchJoinAccepted,
            "database_failure" => Self::DatabaseFailure,
            "external_source_field_conflict" => Self::ExternalSourceFieldConflict,
            "external_acquisition_authority_missing" => Self::ExternalAcquisitionAuthorityMissing,
            "provider_authentication_rejected" => Self::ProviderAuthenticationRejected,
            "provider_rate_limited" => Self::ProviderRateLimited,
            "provider_unavailable" => Self::ProviderUnavailable,
            "external_query_rejected" => Self::ExternalQueryRejected,
            "provider_response_invalid" => Self::ProviderResponseInvalid,
            _ => return None,
        })
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::NoCurrentReports => "no_current_reports",
            Self::NoVerifiedBatch => "no_verified_batch",
            Self::InvalidRequest => "invalid_request",
            Self::InvalidEvidence => "invalid_evidence",
            Self::Unavailable => "unavailable",
            Self::Partial => "partial",
            Self::Internal => "internal",
            Self::TdxBoardMembershipUnsupported => "tdx_board_membership_unsupported",
            Self::UpperLimitStreakMissing => "upper_limit_streak_missing",
            Self::ManualConfirmationContractUnavailable => {
                "manual_confirmation_contract_unavailable"
            }
            Self::FiveMinuteGap => "five_minute_gap",
            Self::ExactBatchJoinAccepted => "exact_batch_join_accepted",
            Self::DatabaseFailure => "database_failure",
            Self::ExternalSourceFieldConflict => "external_source_field_conflict",
            Self::ExternalAcquisitionAuthorityMissing => "external_acquisition_authority_missing",
            Self::ProviderAuthenticationRejected => "provider_authentication_rejected",
            Self::ProviderRateLimited => "provider_rate_limited",
            Self::ProviderUnavailable => "provider_unavailable",
            Self::ExternalQueryRejected => "external_query_rejected",
            Self::ProviderResponseInvalid => "provider_response_invalid",
        }
    }
}

pub(crate) fn known_wire_reason_code(value: &str) -> Option<&'static str> {
    KnownReasonCode::from_wire(value).map(KnownReasonCode::as_str)
}

pub(crate) fn is_canonical_safe_diagnostic(value: Option<&str>) -> bool {
    match value {
        None => true,
        Some(value) => {
            safe_status_message(value).is_some_and(|canonical| canonical.as_str() == value)
        }
    }
}

const MAX_EVIDENCE_IDENTIFIER_CHARS: usize = 160;

fn safe_evidence_identifier(value: &str) -> Option<SafeEvidenceIdentifier> {
    if value.is_empty()
        || value.chars().count() > MAX_EVIDENCE_IDENTIFIER_CHARS
        || !value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.' | '[' | ']')
        })
    {
        return None;
    }
    Some(SafeEvidenceIdentifier(value.to_owned()))
}

fn safe_wire_admission(
    value: i32,
) -> Option<crate::grpc_client::pb::magic::market::v1::AdmissionState> {
    use crate::grpc_client::pb::magic::market::v1::AdmissionState;

    AdmissionState::try_from(value)
        .ok()
        .filter(|admission| *admission != AdmissionState::Unspecified)
}

fn safe_external_wire_admission(
    value: i32,
) -> Option<crate::grpc_client::pb::magic::market::v1::AdmissionState> {
    use crate::grpc_client::external_pb::magic::market::v1::AdmissionState as ExternalAdmission;
    use crate::grpc_client::pb::magic::market::v1::AdmissionState as LocalAdmission;

    match ExternalAdmission::try_from(value).ok()? {
        ExternalAdmission::Unspecified => None,
        ExternalAdmission::Admitted => Some(LocalAdmission::Admitted),
        ExternalAdmission::Unadmitted => Some(LocalAdmission::Unadmitted),
    }
}

fn safe_wire_reason_code(value: &str) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(
            KnownReasonCode::from_wire(value)
                .unwrap_or(KnownReasonCode::Internal)
                .as_str()
                .to_owned(),
        )
    }
}

fn safe_local_wire_method(value: i32) -> Option<MethodIdentity> {
    LocalMethod::try_from_raw(value)
        .ok()
        .map(MethodIdentity::Local)
}

fn safe_external_wire_method(value: i32) -> Option<MethodIdentity> {
    ExternalMethod::try_from_raw(value)
        .ok()
        .map(MethodIdentity::External)
}

fn safe_status_message(message: &str) -> Option<Box<DiagnosticMessage>> {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return None;
    }
    let normalized = trimmed
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .take(MAX_DIAGNOSTIC_CHARS)
        .collect::<String>();
    let lower = normalized.to_ascii_lowercase();
    let safe = if lower.contains("instrumentnews record has conflicting evidence") {
        "InstrumentNews record has conflicting evidence"
    } else if lower.contains("instrument-news html entity is not closed") {
        "instrument-news HTML entity is not closed"
    } else if lower.contains("instrument-news page is not newest-first") {
        "instrument-news page is not newest-first"
    } else if lower.contains("native the paper row unexpectedly has an external link") {
        "native The Paper row unexpectedly has an external link"
    } else if lower.contains("news article host")
        && lower.contains("is not an admitted global-news host")
    {
        "news article host is not an admitted global-news host"
    } else {
        // Upstream status text is an untrusted free-form payload. A blacklist
        // cannot prove that credentials, cookies or request data are absent,
        // so only the closed canonical vocabulary above may reach logs.
        "[redacted-unclassified-status]"
    };
    Some(Box::new(DiagnosticMessage::new(safe)))
}

struct DecodedWireErrorDetail {
    request_id: String,
    raw_operation: i32,
    method: Option<MethodIdentity>,
    provider: String,
    reason_code: String,
    retryable: bool,
    admission: Option<crate::grpc_client::pb::magic::market::v1::AdmissionState>,
    evidence_code: String,
    evidence_field: String,
    record_index: u32,
    has_record_index: bool,
    provider_attempts: ProviderAttempts,
}

fn reconcile_raw_error_detail(
    standard: Result<Option<Vec<u8>>, ()>,
    trailer: Result<Option<Vec<u8>>, ()>,
) -> Option<Vec<u8>> {
    match (standard.ok()?, trailer.ok()?) {
        (Some(standard), Some(trailer)) if standard == trailer => Some(standard),
        (Some(_), Some(_)) => None,
        (Some(detail), None) | (None, Some(detail)) => Some(detail),
        (None, None) => None,
    }
}

fn decode_status_error_detail(
    status: &tonic::Status,
    context: StatusErrorContext<'_>,
) -> Option<DecodedWireErrorDetail> {
    let standard = if status.details().is_empty() {
        Ok(None)
    } else {
        Ok(Some(status.details().to_vec()))
    };
    let trailer = match status.metadata().get_bin(ERROR_DETAIL_TRAILER) {
        None => Ok(None),
        Some(value) => value
            .to_bytes()
            .map(|bytes| Some(bytes.to_vec()))
            .map_err(|_| ()),
    };
    let bytes = reconcile_raw_error_detail(standard, trailer)?;
    decode_error_detail_bytes(&bytes, context)
}

fn decode_error_detail_bytes(
    bytes: &[u8],
    context: StatusErrorContext<'_>,
) -> Option<DecodedWireErrorDetail> {
    let wire = match context.profile {
        ContractProfile::LocalBridgeV1 => {
            let detail =
                crate::grpc_client::pb::magic::market::v1::ErrorDetail::decode(bytes).ok()?;
            DecodedWireErrorDetail {
                request_id: detail.request_id,
                raw_operation: detail.operation,
                method: safe_local_wire_method(detail.operation),
                provider: detail.provider,
                reason_code: detail.reason_code,
                retryable: detail.retryable,
                admission: safe_wire_admission(detail.admission),
                evidence_code: detail.evidence_code,
                evidence_field: detail.evidence_field,
                record_index: detail.record_index,
                has_record_index: detail.has_record_index,
                provider_attempts: ProviderAttempts::default(),
            }
        }
        ContractProfile::ExternalV1 => {
            let detail =
                crate::grpc_client::external_pb::magic::market::v1::ErrorDetail::decode(bytes)
                    .ok()?;
            let provider_attempts = ProviderAttempts::from_external_wire(
                detail.provider_attempts,
                context.external_provider_catalog(),
            );
            DecodedWireErrorDetail {
                request_id: detail.request_id,
                raw_operation: detail.operation,
                method: safe_external_wire_method(detail.operation),
                provider: detail.provider,
                reason_code: detail.reason_code,
                retryable: detail.retryable,
                admission: safe_external_wire_admission(detail.admission),
                evidence_code: detail.evidence_code,
                evidence_field: detail.evidence_field,
                record_index: detail.record_index,
                has_record_index: detail.has_record_index,
                provider_attempts,
            }
        }
    };
    context.accepts(&wire).then_some(wire)
}

fn grpc_error_from_parts(
    code: tonic::Code,
    wire: Option<DecodedWireErrorDetail>,
    diagnostic_message: Option<Box<DiagnosticMessage>>,
) -> GrpcError {
    let detail = if let Some(d) = wire {
        ErrorDetail {
            code: code.to_string(),
            request_id: request_id_correlation(&d.request_id),
            method: d.method,
            provider: safe_wire_provider(&d.provider),
            reason_code: safe_wire_reason_code(&d.reason_code),
            retryable: Some(d.retryable),
            admission: d.admission,
            evidence_code: safe_evidence_identifier(&d.evidence_code),
            evidence_field: safe_evidence_identifier(&d.evidence_field),
            record_index: d.has_record_index.then_some(d.record_index),
            provider_attempts: d.provider_attempts,
            diagnostic_message,
        }
    } else {
        ErrorDetail {
            code: code.to_string(),
            diagnostic_message,
            ..Default::default()
        }
    };
    match code {
        tonic::Code::InvalidArgument => GrpcError::InvalidArgument {
            details: Box::new(detail),
        },
        tonic::Code::Unauthenticated => GrpcError::Unauthenticated {
            details: Box::new(detail),
        },
        tonic::Code::PermissionDenied => GrpcError::PermissionDenied {
            details: Box::new(detail),
        },
        tonic::Code::Unimplemented => GrpcError::Unimplemented {
            details: Box::new(detail),
        },
        tonic::Code::ResourceExhausted => GrpcError::ResourceExhausted {
            details: Box::new(detail),
        },
        tonic::Code::DeadlineExceeded => GrpcError::DeadlineExceeded {
            details: Box::new(detail),
        },
        tonic::Code::Unavailable => GrpcError::Unavailable {
            details: Box::new(detail),
        },
        tonic::Code::FailedPrecondition => GrpcError::FailedPrecondition {
            details: Box::new(detail),
        },
        tonic::Code::Internal => GrpcError::Internal {
            details: Box::new(detail),
        },
        _ => GrpcError::Unknown {
            details: Box::new(detail),
        },
    }
}

pub(crate) enum PersistedErrorDetailTrailer<'a> {
    Absent,
    Bytes(&'a [u8]),
    Malformed,
}

pub(crate) enum PersistedErrorDetailTrailerOwned {
    Absent,
    Bytes(Vec<u8>),
    Malformed,
}

impl PersistedErrorDetailTrailerOwned {
    pub(crate) fn as_ref(&self) -> PersistedErrorDetailTrailer<'_> {
        match self {
            Self::Absent => PersistedErrorDetailTrailer::Absent,
            Self::Bytes(bytes) => PersistedErrorDetailTrailer::Bytes(bytes),
            Self::Malformed => PersistedErrorDetailTrailer::Malformed,
        }
    }
}

pub(crate) fn restore_persisted_status_error(
    code: i32,
    standard: &[u8],
    trailer: PersistedErrorDetailTrailer<'_>,
    diagnostic: Option<&str>,
    context: StatusErrorContext<'_>,
) -> Option<GrpcError> {
    if !is_canonical_safe_diagnostic(diagnostic) {
        return None;
    }
    let code = match code {
        1 => tonic::Code::Cancelled,
        2 => tonic::Code::Unknown,
        3 => tonic::Code::InvalidArgument,
        4 => tonic::Code::DeadlineExceeded,
        5 => tonic::Code::NotFound,
        6 => tonic::Code::AlreadyExists,
        7 => tonic::Code::PermissionDenied,
        8 => tonic::Code::ResourceExhausted,
        9 => tonic::Code::FailedPrecondition,
        10 => tonic::Code::Aborted,
        11 => tonic::Code::OutOfRange,
        12 => tonic::Code::Unimplemented,
        13 => tonic::Code::Internal,
        14 => tonic::Code::Unavailable,
        15 => tonic::Code::DataLoss,
        16 => tonic::Code::Unauthenticated,
        _ => return None,
    };
    let standard = if standard.is_empty() {
        Ok(None)
    } else {
        Ok(Some(standard.to_vec()))
    };
    let trailer = match trailer {
        PersistedErrorDetailTrailer::Absent => Ok(None),
        PersistedErrorDetailTrailer::Bytes(bytes) => Ok(Some(bytes.to_vec())),
        PersistedErrorDetailTrailer::Malformed => Err(()),
    };
    let wire = reconcile_raw_error_detail(standard, trailer)
        .and_then(|bytes| decode_error_detail_bytes(&bytes, context));
    let diagnostic_message = diagnostic.map(|value| Box::new(DiagnosticMessage::new(value)));
    Some(grpc_error_from_parts(code, wire, diagnostic_message))
}

impl From<tonic::Status> for GrpcError {
    fn from(status: tonic::Status) -> Self {
        // Backwards-compatible LocalBridgeV1 parser for callers that have no
        // request context. Real online and recovery consumers use from_status
        // with an explicit profile and identity binding.
        Self::from_status(status, StatusErrorContext::unchecked_local())
    }
}

impl From<crate::grpc_client::auth::AuthError> for GrpcError {
    fn from(_: crate::grpc_client::auth::AuthError) -> Self {
        // token 含非法字符无法注入 metadata → 请求根本到不了服务端, 语义上等同认证失败。
        GrpcError::Unauthenticated {
            details: Box::new(ErrorDetail {
                code: "unauthenticated".to_string(),
                ..Default::default()
            }),
        }
    }
}

impl From<crate::grpc_client::envelope::EnvelopeError> for GrpcError {
    fn from(_: crate::grpc_client::envelope::EnvelopeError) -> Self {
        // 信封构造失败是客户端本地错误 (序列化失败/未冻结 schema), 非服务端状态。
        // 映射 Unknown + code=envelope, 与响应侧信封校验失败同码 (见 client.rs query)。
        GrpcError::Unknown {
            details: Box::new(ErrorDetail {
                code: "envelope".to_string(),
                ..Default::default()
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonic::Code;

    fn detail_for(code: Code) -> ErrorDetail {
        ErrorDetail {
            code: code.to_string(),
            diagnostic_message: Some(Box::new(DiagnosticMessage::new(
                "[redacted-unclassified-status]",
            ))),
            ..Default::default()
        }
    }

    #[test]
    fn maps_all_contract_codes() {
        let cases = [
            (
                Code::InvalidArgument,
                GrpcError::InvalidArgument {
                    details: Box::new(detail_for(Code::InvalidArgument)),
                },
            ),
            (
                Code::Unauthenticated,
                GrpcError::Unauthenticated {
                    details: Box::new(detail_for(Code::Unauthenticated)),
                },
            ),
            (
                Code::PermissionDenied,
                GrpcError::PermissionDenied {
                    details: Box::new(detail_for(Code::PermissionDenied)),
                },
            ),
            (
                Code::Unimplemented,
                GrpcError::Unimplemented {
                    details: Box::new(detail_for(Code::Unimplemented)),
                },
            ),
            (
                Code::ResourceExhausted,
                GrpcError::ResourceExhausted {
                    details: Box::new(detail_for(Code::ResourceExhausted)),
                },
            ),
            (
                Code::DeadlineExceeded,
                GrpcError::DeadlineExceeded {
                    details: Box::new(detail_for(Code::DeadlineExceeded)),
                },
            ),
            (
                Code::Unavailable,
                GrpcError::Unavailable {
                    details: Box::new(detail_for(Code::Unavailable)),
                },
            ),
            (
                Code::FailedPrecondition,
                GrpcError::FailedPrecondition {
                    details: Box::new(detail_for(Code::FailedPrecondition)),
                },
            ),
            (
                Code::Internal,
                GrpcError::Internal {
                    details: Box::new(detail_for(Code::Internal)),
                },
            ),
            // tonic 0.14: Code::Unknown.to_string() = "Unknown error" (grpc 规范英文描述)。
            (
                Code::Unknown,
                GrpcError::Unknown {
                    details: Box::new(ErrorDetail {
                        code: "Unknown error".into(),
                        diagnostic_message: Some(Box::new(DiagnosticMessage::new(
                            "[redacted-unclassified-status]",
                        ))),
                        ..Default::default()
                    }),
                },
            ),
        ];
        for (code, expected) in cases {
            let status = tonic::Status::new(code, "msg");
            assert_eq!(GrpcError::from(status), expected, "code {code:?}");
        }
    }

    /// D2 核心: Fetch 失败 status 携带的 ErrorDetail (provider/reason_code/retryable)
    /// 必须保真解码到 GrpcError.details(), 桥据此重建 GatewayError 分类；
    /// request_id 仅保留不可逆关联 token, 不保留不受信任的 wire 原文。
    #[test]
    fn decodes_fetch_error_detail_into_all_variants() {
        let mut detail = crate::grpc_client::pb::magic::market::v1::ErrorDetail {
            request_id: "req-42".to_string(),
            operation: 8,
            provider: "Tdx".to_string(),
            reason_code: "no_verified_batch".to_string(),
            retryable: true,
            ..Default::default()
        };
        // Internal (Fetch 分支) + Unavailable (服务端不可达时无 detail) 两条路径。
        let encoded = detail.encode_to_vec();
        let status = tonic::Status::with_details(Code::Internal, "取数失败", encoded.into());
        let err = GrpcError::from(status);
        assert!(matches!(err, GrpcError::Internal { .. }));
        assert_eq!(err.details().provider.as_deref(), Some("Tdx"));
        assert_eq!(
            err.details().reason_code.as_deref(),
            Some("no_verified_batch")
        );
        assert_eq!(err.details().retryable, Some(true));
        let request_42 = err
            .details()
            .request_id
            .as_deref()
            .expect("request correlation token")
            .to_owned();
        assert!(request_42.starts_with("sha256:"));
        assert_eq!(request_42.len(), "sha256:".len() + 64);
        assert_ne!(request_42, "req-42");

        // Unavailable 也带 detail (非 Fetch 路径同样保留 request_id 供审计)。
        detail.request_id = "req-43".to_string();
        let status = tonic::Status::with_details(
            Code::Unavailable,
            "连接被拒绝",
            detail.encode_to_vec().into(),
        );
        let err = GrpcError::from(status);
        assert!(matches!(err, GrpcError::Unavailable { .. }));
        let request_43 = err
            .details()
            .request_id
            .as_deref()
            .expect("request correlation token");
        assert!(request_43.starts_with("sha256:"));
        assert_eq!(request_43.len(), "sha256:".len() + 64);
        assert_ne!(request_43, "req-43");
        assert_ne!(request_43, request_42);
    }

    /// BR-159: server 分类 no_current_reports (业务态: 180 天窗口无报告) 必须在
    /// wire 解码层保真 — safe_wire_reason_code 的 KnownReasonCode 白名单不能把它
    /// 折叠成 internal (2026-08-30 诊断: client 侧 13 只股票 audit 仍显示 internal,
    /// 折叠发生在 grpc_client/errors.rs, 早于 grpc_source.rs 的 reason_code_static)。
    #[test]
    fn no_current_reports_survives_wire_detail_decode() {
        let detail = crate::grpc_client::pb::magic::market::v1::ErrorDetail {
            request_id: "req-ncr".to_string(),
            operation: 8,
            provider: "Eastmoney".to_string(),
            reason_code: "no_current_reports".to_string(),
            retryable: false,
            ..Default::default()
        };
        let status =
            tonic::Status::with_details(Code::Internal, "取数失败", detail.encode_to_vec().into());
        let err = GrpcError::from(status);
        assert!(matches!(err, GrpcError::Internal { .. }));
        assert_eq!(err.details().provider.as_deref(), Some("Eastmoney"));
        assert_eq!(
            err.details().reason_code.as_deref(),
            Some("no_current_reports")
        );
        assert_eq!(err.details().retryable, Some(false));
    }

    #[test]
    fn preserves_bounded_safe_status_detail_without_exposing_credentials() {
        let status = tonic::Status::new(
            Code::FailedPrecondition,
            "InstrumentNews record has conflicting evidence",
        );
        let err = GrpcError::from(status);
        assert_eq!(
            err.details()
                .diagnostic_message
                .as_ref()
                .map(|message| message.as_str()),
            Some("InstrumentNews record has conflicting evidence")
        );

        let status = tonic::Status::new(
            Code::Unauthenticated,
            "authorization: Bearer TEST_SECRET_TOKEN",
        );
        let err = GrpcError::from(status);
        assert_eq!(
            err.details()
                .diagnostic_message
                .as_ref()
                .map(|message| message.as_str()),
            Some("[redacted-unclassified-status]")
        );
        assert!(!format!("{err:?}").contains("TEST_SECRET_TOKEN"));

        let long = "x".repeat(600);
        let err = GrpcError::from(tonic::Status::new(Code::Internal, long));
        assert_eq!(
            err.details()
                .diagnostic_message
                .as_deref()
                .expect("bounded diagnostic")
                .as_str(),
            "[redacted-unclassified-status]"
        );
    }

    #[test]
    fn rejects_unclassified_upstream_status_text_instead_of_blacklisting_secrets() {
        for diagnostic in [
            "api_key=TEST_ONLY_VALUE",
            "cookie: session=TEST_ONLY_COOKIE",
            r#"request payload={\"query\":\"TEST_ONLY_PRIVATE_INPUT\"}"#,
            "unlabelled TEST_ONLY_SENSITIVE_VALUE",
        ] {
            let error = GrpcError::from(tonic::Status::new(Code::Internal, diagnostic));
            assert_eq!(
                error
                    .details()
                    .diagnostic_message
                    .as_deref()
                    .map(DiagnosticMessage::as_str),
                Some("[redacted-unclassified-status]")
            );
            assert!(
                !format!("{error:?}").contains("TEST_ONLY"),
                "arbitrary upstream status text must never survive in Debug"
            );
        }
    }

    #[test]
    fn wire_error_detail_is_closed_and_keeps_authoritative_classification() {
        let untrusted = crate::grpc_client::pb::magic::market::v1::ErrorDetail {
            request_id: "TEST_ONLY_SECRET_REQUEST_ID".to_string(),
            operation: i32::MAX,
            provider: "TEST_ONLY_SECRET_PROVIDER".to_string(),
            reason_code: "TEST_ONLY_SECRET_REASON".to_string(),
            retryable: true,
            admission: i32::MAX,
            evidence_code: "TEST_ONLY_SECRET=value".to_owned(),
            evidence_field: "private/request/payload".to_owned(),
            record_index: 42,
            has_record_index: false,
        };
        let error = GrpcError::from(tonic::Status::with_details(
            Code::Internal,
            "",
            untrusted.encode_to_vec().into(),
        ));
        let detail = error.details();
        let request_id = detail
            .request_id
            .as_deref()
            .expect("hashed request identity");
        assert!(request_id.starts_with("sha256:"));
        assert_eq!(request_id.len(), "sha256:".len() + 64);
        assert_eq!(detail.method, None);
        assert_eq!(detail.provider, None);
        assert_eq!(detail.reason_code.as_deref(), Some("internal"));
        assert_eq!(detail.retryable, Some(true));
        assert_eq!(detail.admission, None);
        assert_eq!(detail.evidence_code, None);
        assert_eq!(detail.evidence_field, None);
        assert_eq!(detail.record_index, None);
        assert!(!format!("{error:?}").contains("TEST_ONLY_SECRET"));

        let classified = crate::grpc_client::pb::magic::market::v1::ErrorDetail {
            request_id: "TEST_CODE_CLASSIFIED_REQUEST".to_string(),
            operation: crate::grpc_client::pb::magic::market::v1::Operation::ProviderTopNRankings
                as i32,
            provider: "Tdx".to_string(),
            reason_code: "invalid_evidence".to_string(),
            retryable: false,
            ..Default::default()
        };
        let error = GrpcError::from(tonic::Status::with_details(
            Code::Internal,
            "",
            classified.encode_to_vec().into(),
        ));
        let detail = error.details();
        assert_ne!(
            detail.request_id.as_deref(),
            Some("TEST_CODE_CLASSIFIED_REQUEST")
        );
        assert_eq!(
            detail.method,
            Some(MethodIdentity::Local(
                LocalMethod::try_from_raw(
                    crate::grpc_client::pb::magic::market::v1::Operation::ProviderTopNRankings
                        as i32,
                )
                .expect("known Local method"),
            ))
        );
        assert_eq!(detail.provider.as_deref(), Some("Tdx"));
        assert_eq!(detail.reason_code.as_deref(), Some("invalid_evidence"));
        assert_eq!(detail.retryable, Some(false));
    }

    #[test]
    fn br238_external_v2_error_detail_preserves_structured_authority() {
        use crate::grpc_client::pb::magic::market::v1::AdmissionState;

        let wire = crate::grpc_client::pb::magic::market::v1::ErrorDetail {
            request_id: "TEST_CODE_V2_ERROR_DETAIL".to_owned(),
            operation: crate::grpc_client::pb::magic::market::v1::Operation::GlobalNews as i32,
            provider: "Cailianpress".to_owned(),
            reason_code: "external_query_rejected".to_owned(),
            retryable: false,
            admission: AdmissionState::Unadmitted as i32,
            evidence_code: "record_evidence_conflict".to_owned(),
            evidence_field: "records[2].identity".to_owned(),
            record_index: 2,
            has_record_index: true,
        };
        let error = GrpcError::from(tonic::Status::with_details(
            Code::Internal,
            "",
            wire.encode_to_vec().into(),
        ));
        let detail = error.details();

        assert_eq!(
            detail.reason_code.as_deref(),
            Some("external_query_rejected")
        );
        assert_eq!(detail.retryable, Some(false));
        assert_eq!(detail.admission, Some(AdmissionState::Unadmitted));
        assert_eq!(
            detail
                .evidence_code
                .as_ref()
                .map(SafeEvidenceIdentifier::as_str),
            Some("record_evidence_conflict")
        );
        assert_eq!(
            detail
                .evidence_field
                .as_ref()
                .map(SafeEvidenceIdentifier::as_str),
            Some("records[2].identity")
        );
        assert_eq!(detail.record_index, Some(2));
        assert!(!format!("{error:?}").contains("records[2].identity"));
    }

    #[test]
    fn br238_external_v2_error_detail_decodes_custom_binary_trailer() {
        use crate::grpc_client::pb::magic::market::v1::AdmissionState;
        use tonic::metadata::MetadataValue;

        let wire = crate::grpc_client::pb::magic::market::v1::ErrorDetail {
            request_id: "TEST_CODE_TRAILER_ERROR_DETAIL".to_owned(),
            operation: crate::grpc_client::pb::magic::market::v1::Operation::InstrumentNews as i32,
            provider: "Sina".to_owned(),
            reason_code: "invalid_evidence".to_owned(),
            retryable: false,
            admission: AdmissionState::Unadmitted as i32,
            evidence_code: "record_time_conflict".to_owned(),
            evidence_field: "records[0].published_at".to_owned(),
            record_index: 0,
            has_record_index: true,
        };
        let mut status = tonic::Status::new(Code::FailedPrecondition, "rejected");
        status.metadata_mut().insert_bin(
            "magic-error-detail-bin",
            MetadataValue::from_bytes(&wire.encode_to_vec()),
        );

        let error = GrpcError::from(status);
        let detail = error.details();
        assert_eq!(detail.provider.as_deref(), Some("Sina"));
        assert_eq!(detail.reason_code.as_deref(), Some("invalid_evidence"));
        assert_eq!(detail.admission, Some(AdmissionState::Unadmitted));
        assert_eq!(
            detail
                .evidence_field
                .as_ref()
                .map(SafeEvidenceIdentifier::as_str),
            Some("records[0].published_at")
        );
        assert_eq!(detail.record_index, Some(0));
    }

    #[test]
    fn grpc_dual_contract_error_detail_carriers_reject_new_field_conflict() {
        use crate::grpc_client::external_pb::magic::market::v1::{
            AdmissionState, ErrorDetail as ExternalErrorDetail, Operation,
            ProviderAttemptDetail,
        };
        use tonic::metadata::MetadataValue;

        let standard = ExternalErrorDetail {
            request_id: "TEST_CODE_DUAL_CARRIER_REQUEST".to_owned(),
            operation: Operation::GlobalNews as i32,
            provider: "Cailianpress".to_owned(),
            reason_code: "provider_unavailable".to_owned(),
            retryable: true,
            admission: AdmissionState::Unadmitted as i32,
            evidence_code: "provider_attempt_failed".to_owned(),
            evidence_field: "provider_attempts[0]".to_owned(),
            record_index: 0,
            has_record_index: false,
            provider_attempts: vec![ProviderAttemptDetail {
                ordinal: 1,
                provider: "Cailianpress".to_owned(),
                outcome: "unavailable".to_owned(),
                reason_code: "provider_unavailable".to_owned(),
                retryable: true,
                terminal: false,
            }],
        };
        let trailer = ExternalErrorDetail {
            provider_attempts: vec![ProviderAttemptDetail {
                outcome: "rejected".to_owned(),
                ..standard.provider_attempts[0].clone()
            }],
            ..standard.clone()
        };
        let mut status = tonic::Status::with_details(
            Code::Unavailable,
            "provider unavailable",
            standard.encode_to_vec().into(),
        );
        status.metadata_mut().insert_bin(
            "magic-error-detail-bin",
            MetadataValue::from_bytes(&trailer.encode_to_vec()),
        );

        let error = GrpcError::from(status);

        assert!(matches!(&error, GrpcError::Unavailable { .. }));
        assert_eq!(
            (
                error.details().provider.as_deref(),
                error.details().reason_code.as_deref(),
                error.details().retryable,
            ),
            (None, None, None),
            "conflicting carriers must not authorize provider classification or retry evidence",
        );
    }

    fn local_detail(
        request_id: &str,
        operation: crate::grpc_client::pb::magic::market::v1::Operation,
    ) -> crate::grpc_client::pb::magic::market::v1::ErrorDetail {
        crate::grpc_client::pb::magic::market::v1::ErrorDetail {
            request_id: request_id.to_owned(),
            operation: operation as i32,
            provider: "Tdx".to_owned(),
            reason_code: "provider_unavailable".to_owned(),
            retryable: true,
            ..Default::default()
        }
    }

    fn external_detail(
        request_id: &str,
        operation: crate::grpc_client::external_pb::magic::market::v1::Operation,
        provider_attempts: Vec<
            crate::grpc_client::external_pb::magic::market::v1::ProviderAttemptDetail,
        >,
    ) -> crate::grpc_client::external_pb::magic::market::v1::ErrorDetail {
        crate::grpc_client::external_pb::magic::market::v1::ErrorDetail {
            request_id: request_id.to_owned(),
            operation: operation as i32,
            provider: "Cailianpress".to_owned(),
            reason_code: "provider_unavailable".to_owned(),
            retryable: true,
            provider_attempts,
            ..Default::default()
        }
    }

    fn external_global_news_method() -> MethodIdentity {
        MethodIdentity::from_client_operation(
            ContractProfile::ExternalV1,
            crate::grpc_client::pb::magic::market::v1::Operation::GlobalNews,
        )
        .expect("delivered External GlobalNews method")
    }

    fn validated_attempt_catalog(providers: &[&str]) -> ExternalProviderCatalog {
        use crate::grpc_client::external_pb::magic::market::v1::{
            AdmissionState, CapabilitiesResponse, Capability, Operation,
        };

        let response = CapabilitiesResponse {
            request_id: "TEST_CODE_ATTEMPT_CATALOG".to_owned(),
            capabilities: providers
                .iter()
                .map(|provider| Capability {
                    operation: Operation::GlobalNews as i32,
                    repository_admission: AdmissionState::Admitted as i32,
                    runtime_available: true,
                    provider: (*provider).to_owned(),
                    exact_scope: "TEST_CODE_ATTEMPT_CATALOG_SCOPE".to_owned(),
                    blocker: String::new(),
                    diagnostic_available: true,
                })
                .collect(),
        };
        crate::grpc_client::client::external_control_attempt::validated_external_provider_catalog(
            "TEST_CODE_ATTEMPT_CATALOG",
            &response,
        )
        .expect("request-ID validated provider catalog")
    }

    #[test]
    fn grpc_dual_contract_status_parser_keeps_profile_bound_method_identity() {
        let local_cases = [
            (61, "OPERATION_CHAIN_BATCH"),
            (62, "OPERATION_BENCHMARK_BARS"),
        ];
        for (raw, expected_name) in local_cases {
            let wire = crate::grpc_client::pb::magic::market::v1::ErrorDetail {
                request_id: "TEST_CODE_LOCAL_RAW_METHOD".to_owned(),
                operation: raw,
                provider: "Tdx".to_owned(),
                ..Default::default()
            };
            let method = MethodIdentity::Local(
                LocalMethod::try_from_raw(raw).expect("known Local raw method"),
            );
            let error = GrpcError::from_status(
                tonic::Status::with_details(
                    Code::Unavailable,
                    "",
                    wire.encode_to_vec().into(),
                ),
                StatusErrorContext::data(method, "TEST_CODE_LOCAL_RAW_METHOD"),
            );

            assert_eq!(error.details().method, Some(method));
            assert_eq!(
                error.details().method.map(MethodIdentity::as_str_name),
                Some(expected_name)
            );
        }

        let external_cases = [
            (61, "OPERATION_CURRENT_AUCTION_OBSERVATIONS"),
            (62, "OPERATION_ECONOMIC_RELEASE_OBSERVATIONS"),
            (63, "OPERATION_ECONOMIC_RELEASE_SCHEDULE"),
        ];
        for (raw, expected_name) in external_cases {
            let wire = crate::grpc_client::external_pb::magic::market::v1::ErrorDetail {
                request_id: "TEST_CODE_EXTERNAL_RAW_METHOD".to_owned(),
                operation: raw,
                provider: "Cailianpress".to_owned(),
                ..Default::default()
            };
            let method = MethodIdentity::External(
                ExternalMethod::try_from_raw(raw).expect("known External raw method"),
            );
            let error = GrpcError::from_status(
                tonic::Status::with_details(
                    Code::Unavailable,
                    "",
                    wire.encode_to_vec().into(),
                ),
                StatusErrorContext::data(method, "TEST_CODE_EXTERNAL_RAW_METHOD"),
            );

            assert_eq!(error.details().method, Some(method));
            assert_eq!(
                error.details().method.map(MethodIdentity::as_str_name),
                Some(expected_name)
            );
        }
    }

    #[test]
    fn grpc_dual_contract_error_detail_carriers_enforce_raw_integrity() {
        use tonic::metadata::MetadataValue;

        let wire = local_detail(
            "TEST_CODE_CARRIER_REQUEST",
            crate::grpc_client::pb::magic::market::v1::Operation::GlobalNews,
        );
        let bytes = wire.encode_to_vec();

        let standard_only = GrpcError::from(tonic::Status::with_details(
            Code::Unavailable,
            "",
            bytes.clone().into(),
        ));
        assert_eq!(standard_only.details().provider.as_deref(), Some("Tdx"));

        let mut trailer_only = tonic::Status::new(Code::Unavailable, "");
        trailer_only.metadata_mut().insert_bin(
            ERROR_DETAIL_TRAILER,
            MetadataValue::from_bytes(&bytes),
        );
        assert_eq!(
            GrpcError::from(trailer_only).details().provider.as_deref(),
            Some("Tdx")
        );

        let mut equal = tonic::Status::with_details(
            Code::Unavailable,
            "",
            bytes.clone().into(),
        );
        equal.metadata_mut().insert_bin(
            ERROR_DETAIL_TRAILER,
            MetadataValue::from_bytes(&bytes),
        );
        assert_eq!(
            GrpcError::from(equal).details().provider.as_deref(),
            Some("Tdx")
        );

        let mut conflicting = tonic::Status::with_details(
            Code::Unavailable,
            "",
            bytes.clone().into(),
        );
        let mut conflicting_wire = wire.clone();
        conflicting_wire.retryable = false;
        conflicting.metadata_mut().insert_bin(
            ERROR_DETAIL_TRAILER,
            MetadataValue::from_bytes(&conflicting_wire.encode_to_vec()),
        );
        assert_eq!(
            GrpcError::from(conflicting).details().provider,
            None,
            "different raw carriers reject the complete detail"
        );

        let mut malformed = tonic::Status::with_details(Code::Unavailable, "", bytes.into());
        let mut headers = tonic::codegen::http::HeaderMap::new();
        headers.insert(
            ERROR_DETAIL_TRAILER,
            tonic::codegen::http::HeaderValue::from_static("%%%"),
        );
        *malformed.metadata_mut() = tonic::metadata::MetadataMap::from_headers(headers);
        let malformed = GrpcError::from(malformed);
        assert!(matches!(&malformed, GrpcError::Unavailable { .. }));
        assert_eq!(malformed.details().provider, None);

        let malformed_standard = GrpcError::from(tonic::Status::with_details(
            Code::Unavailable,
            "",
            vec![0xff].into(),
        ));
        assert!(matches!(
            &malformed_standard,
            GrpcError::Unavailable { .. }
        ));
        assert_eq!(malformed_standard.details().provider, None);
    }

    #[test]
    fn grpc_dual_contract_data_context_rejects_missing_or_mismatched_identity() {
        use crate::grpc_client::pb::magic::market::v1::Operation;

        let method = MethodIdentity::from_client_operation(
            ContractProfile::LocalBridgeV1,
            Operation::GlobalNews,
        )
        .expect("Local GlobalNews method");
        let exact = local_detail("TEST_CODE_EXPECTED_REQUEST", Operation::GlobalNews);
        let exact_error = GrpcError::from_status(
            tonic::Status::with_details(
                Code::Internal,
                "",
                exact.encode_to_vec().into(),
            ),
            StatusErrorContext::data(method, "TEST_CODE_EXPECTED_REQUEST"),
        );
        assert_eq!(exact_error.details().provider.as_deref(), Some("Tdx"));

        let mut unknown_method = exact.clone();
        unknown_method.operation = 63;
        let cases = vec![
            local_detail("", Operation::GlobalNews),
            local_detail("TEST_CODE_WRONG_REQUEST", Operation::GlobalNews),
            local_detail("TEST_CODE_EXPECTED_REQUEST", Operation::Unspecified),
            local_detail("TEST_CODE_EXPECTED_REQUEST", Operation::InstrumentNews),
            unknown_method,
        ];
        for wire in cases {
            let error = GrpcError::from_status(
                tonic::Status::with_details(
                    Code::Internal,
                    "",
                    wire.encode_to_vec().into(),
                ),
                StatusErrorContext::data(method, "TEST_CODE_EXPECTED_REQUEST"),
            );
            assert!(matches!(&error, GrpcError::Internal { .. }));
            assert_eq!(
                (
                    error.details().method,
                    error.details().provider.as_deref(),
                    error.details().retryable,
                ),
                (None, None, None)
            );
        }
    }

    #[test]
    fn grpc_dual_contract_control_context_requires_exact_id_and_zero_operation() {
        use crate::grpc_client::external_pb::magic::market::v1::Operation;

        let exact = external_detail("TEST_CODE_CONTROL_REQUEST", Operation::Unspecified, vec![]);
        let exact_error = GrpcError::from_status(
            tonic::Status::with_details(
                Code::Unavailable,
                "",
                exact.encode_to_vec().into(),
            ),
            StatusErrorContext::control(
                ContractProfile::ExternalV1,
                "TEST_CODE_CONTROL_REQUEST",
            ),
        );
        assert_eq!(exact_error.details().method, None);
        assert_eq!(
            exact_error.details().provider.as_deref(),
            Some("Cailianpress")
        );

        let cases = [
            external_detail("", Operation::Unspecified, vec![]),
            external_detail("TEST_CODE_WRONG_CONTROL", Operation::Unspecified, vec![]),
            external_detail("TEST_CODE_CONTROL_REQUEST", Operation::GlobalNews, vec![]),
        ];
        for wire in cases {
            let error = GrpcError::from_status(
                tonic::Status::with_details(
                    Code::Unavailable,
                    "",
                    wire.encode_to_vec().into(),
                ),
                StatusErrorContext::control(
                    ContractProfile::ExternalV1,
                    "TEST_CODE_CONTROL_REQUEST",
                ),
            );
            assert!(matches!(&error, GrpcError::Unavailable { .. }));
            assert_eq!(error.details().provider, None);
        }
    }

    #[test]
    fn grpc_dual_contract_external_attempts_preserve_bounds_fields_and_order() {
        use crate::grpc_client::external_pb::magic::market::v1::{
            Operation, ProviderAttemptDetail,
        };

        let catalog = validated_attempt_catalog(&["Cailianpress"]);
        let MethodIdentity::External(method) = external_global_news_method() else {
            panic!("External GlobalNews method identity");
        };
        let decode = |attempts: Vec<ProviderAttemptDetail>| {
            GrpcError::from_status(
                tonic::Status::with_details(
                    Code::Unavailable,
                    "",
                    external_detail("TEST_CODE_ATTEMPTS", Operation::GlobalNews, attempts)
                        .encode_to_vec()
                        .into(),
                ),
                StatusErrorContext::external_data(method, "TEST_CODE_ATTEMPTS", &catalog),
            )
        };
        let attempt = |ordinal| {
            let (outcome, reason_code, retryable, terminal) = match ordinal % 3 {
                1 => ("selected", "selected", false, false),
                2 => ("rejected", "query_rejected", false, false),
                _ => ("failed", "unavailable", true, ordinal % 2 == 0),
            };
            ProviderAttemptDetail {
                ordinal,
                provider: "Cailianpress".to_owned(),
                outcome: outcome.to_owned(),
                reason_code: reason_code.to_owned(),
                retryable,
                terminal,
            }
        };

        assert_eq!(
            decode(vec![]).details().provider_attempts,
            ProviderAttempts::Rejected { observed_count: 0 }
        );

        let one_error = decode(vec![attempt(1)]);
        assert!(matches!(&one_error, GrpcError::Unavailable { .. }));
        assert_eq!(
            (
                one_error.details().provider.as_deref(),
                one_error.details().reason_code.as_deref(),
                one_error.details().retryable,
            ),
            (Some("Cailianpress"), Some("provider_unavailable"), Some(true)),
        );
        let one = one_error
            .details()
            .provider_attempts
            .accepted()
            .expect("one attempt accepted");
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].ordinal, 1);
        assert_eq!(one[0].provider.as_str(), "Cailianpress");
        assert_eq!(one[0].outcome.as_str(), "selected");
        assert_eq!(one[0].reason_code.as_str(), "selected");
        assert!(!one[0].retryable);
        assert!(!one[0].terminal);
        assert!(one[0].provider.is_supported());
        assert!(one[0].outcome.is_supported());
        assert!(one[0].reason_code.is_supported());

        let sixteen_error = decode((1..=16).map(attempt).collect());
        let sixteen = sixteen_error
            .details()
            .provider_attempts
            .accepted()
            .expect("sixteen attempts accepted");
        assert_eq!(sixteen.len(), 16);
        let expected_sixteen = [
            (1, "Cailianpress", "selected", "selected", false, false),
            (2, "Cailianpress", "rejected", "query_rejected", false, false),
            (3, "Cailianpress", "failed", "unavailable", true, false),
            (4, "Cailianpress", "selected", "selected", false, false),
            (5, "Cailianpress", "rejected", "query_rejected", false, false),
            (6, "Cailianpress", "failed", "unavailable", true, true),
            (7, "Cailianpress", "selected", "selected", false, false),
            (8, "Cailianpress", "rejected", "query_rejected", false, false),
            (9, "Cailianpress", "failed", "unavailable", true, false),
            (10, "Cailianpress", "selected", "selected", false, false),
            (11, "Cailianpress", "rejected", "query_rejected", false, false),
            (12, "Cailianpress", "failed", "unavailable", true, true),
            (13, "Cailianpress", "selected", "selected", false, false),
            (14, "Cailianpress", "rejected", "query_rejected", false, false),
            (15, "Cailianpress", "failed", "unavailable", true, false),
            (16, "Cailianpress", "selected", "selected", false, false),
        ];
        for (attempt, expected) in sixteen.iter().zip(expected_sixteen) {
            assert_eq!(
                (
                    attempt.ordinal,
                    attempt.provider.as_str(),
                    attempt.outcome.as_str(),
                    attempt.reason_code.as_str(),
                    attempt.retryable,
                    attempt.terminal,
                ),
                expected,
            );
            assert!(attempt.provider.is_supported());
            assert!(attempt.outcome.is_supported());
            assert!(attempt.reason_code.is_supported());
        }

        let seventeen = decode((1..=17).map(attempt).collect());
        assert_eq!(
            seventeen.details().provider_attempts,
            ProviderAttempts::Rejected { observed_count: 17 }
        );
    }

    #[test]
    fn grpc_dual_contract_external_attempt_unknown_vocabulary_is_preserved_and_redacted() {
        use crate::grpc_client::external_pb::magic::market::v1::{
            Operation, ProviderAttemptDetail,
        };

        let catalog = validated_attempt_catalog(&["TEST_ONLY_UNKNOWN_PROVIDER"]);
        let MethodIdentity::External(method) = external_global_news_method() else {
            panic!("External GlobalNews method identity");
        };
        let error = GrpcError::from_status(
            tonic::Status::with_details(
                Code::Unavailable,
                "",
                external_detail(
                    "TEST_CODE_UNKNOWN_ATTEMPT_REQUEST",
                    Operation::GlobalNews,
                    vec![ProviderAttemptDetail {
                        ordinal: 1,
                        provider: "TEST_ONLY_UNKNOWN_PROVIDER".to_owned(),
                        outcome: "TEST_ONLY_UNKNOWN_OUTCOME".to_owned(),
                        reason_code: "TEST_ONLY_UNKNOWN_REASON".to_owned(),
                        retryable: false,
                        terminal: false,
                    }],
                )
                .encode_to_vec()
                .into(),
            ),
            StatusErrorContext::external_data(
                method,
                "TEST_CODE_UNKNOWN_ATTEMPT_REQUEST",
                &catalog,
            ),
        );
        assert_eq!(
            error.details().provider_attempts,
            ProviderAttempts::Rejected { observed_count: 1 }
        );
        assert!(matches!(&error, GrpcError::Unavailable { .. }));
        assert_eq!(error.details().provider.as_deref(), Some("Cailianpress"));
        assert_eq!(
            error.details().reason_code.as_deref(),
            Some("provider_unavailable")
        );
        assert_eq!(error.details().retryable, Some(true));
        assert!(!format!("{error:?}").contains("TEST_ONLY_UNKNOWN"));
    }

    fn decode_closed_contract_attempts(
        request_id: &str,
        provider_attempts: Vec<
            crate::grpc_client::external_pb::magic::market::v1::ProviderAttemptDetail,
        >,
    ) -> GrpcError {
        use crate::grpc_client::external_pb::magic::market::v1::Operation;

        GrpcError::from_status(
            tonic::Status::with_details(
                Code::Unavailable,
                "",
                external_detail(request_id, Operation::GlobalNews, provider_attempts)
                    .encode_to_vec()
                    .into(),
            ),
            StatusErrorContext::data(external_global_news_method(), request_id),
        )
    }

    fn decode_validated_closed_contract_attempts(
        providers: &[&str],
        request_id: &str,
        provider_attempts: Vec<
            crate::grpc_client::external_pb::magic::market::v1::ProviderAttemptDetail,
        >,
    ) -> GrpcError {
        use crate::grpc_client::external_pb::magic::market::v1::Operation;

        let catalog = validated_attempt_catalog(providers);
        let MethodIdentity::External(method) = external_global_news_method() else {
            panic!("External GlobalNews method identity");
        };
        GrpcError::from_status(
            tonic::Status::with_details(
                Code::Unavailable,
                "",
                external_detail(request_id, Operation::GlobalNews, provider_attempts)
                    .encode_to_vec()
                    .into(),
            ),
            StatusErrorContext::external_data(method, request_id, &catalog),
        )
    }

    fn closed_contract_attempt(
        ordinal: u32,
        provider: &str,
        outcome: &str,
        reason_code: &str,
        retryable: bool,
        terminal: bool,
    ) -> crate::grpc_client::external_pb::magic::market::v1::ProviderAttemptDetail {
        crate::grpc_client::external_pb::magic::market::v1::ProviderAttemptDetail {
            ordinal,
            provider: provider.to_owned(),
            outcome: outcome.to_owned(),
            reason_code: reason_code.to_owned(),
            retryable,
            terminal,
        }
    }

    fn assert_attempt_trace_unsupported_without_top_level_change(
        error: &GrpcError,
        case: &str,
    ) {
        assert!(matches!(error, GrpcError::Unavailable { .. }), "{case}");
        assert_eq!(
            (
                error.details().provider.as_deref(),
                error.details().reason_code.as_deref(),
                error.details().retryable,
            ),
            (Some("Cailianpress"), Some("provider_unavailable"), Some(true)),
            "{case}: attempts validity must not rewrite the top-level detail",
        );
        assert_eq!(
            crate::grpc_client::retry::retry_decision(error),
            crate::grpc_client::retry::RetryDecision::RetryBackoff,
            "{case}: attempts validity must not enter the top-level retry decision",
        );
        assert!(
            error.details().provider_attempts.accepted().is_none(),
            "{case}: the entire attempts interpretation must be unsupported",
        );
    }

    fn assert_single_supported_attempt_without_top_level_change(
        error: &GrpcError,
        expected: (u32, &str, &str, &str, bool, bool),
        case: &str,
    ) {
        assert!(matches!(error, GrpcError::Unavailable { .. }), "{case}");
        assert_eq!(
            (
                error.details().provider.as_deref(),
                error.details().reason_code.as_deref(),
                error.details().retryable,
            ),
            (Some("Cailianpress"), Some("provider_unavailable"), Some(true)),
            "{case}: attempts interpretation must not rewrite the top-level detail",
        );
        assert_eq!(
            crate::grpc_client::retry::retry_decision(error),
            crate::grpc_client::retry::RetryDecision::RetryBackoff,
            "{case}: attempts interpretation must not enter the top-level retry decision",
        );
        let attempts = error
            .details()
            .provider_attempts
            .accepted()
            .unwrap_or_else(|| panic!("{case}: public contract row must be supported"));
        assert_eq!(attempts.len(), 1, "{case}");
        let attempt = &attempts[0];
        assert_eq!(
            (
                attempt.ordinal,
                attempt.provider.as_str(),
                attempt.outcome.as_str(),
                attempt.reason_code.as_str(),
                attempt.retryable,
                attempt.terminal,
            ),
            expected,
            "{case}",
        );
        assert!(attempt.provider.is_supported(), "{case}");
        assert!(attempt.outcome.is_supported(), "{case}");
        assert!(attempt.reason_code.is_supported(), "{case}");
    }

    #[test]
    fn grpc_dual_contract_external_attempts_reject_empty_and_noncontiguous_ordinals() {
        let attempt = |ordinal| {
            closed_contract_attempt(
                ordinal,
                "Cailianpress",
                "failed",
                "unavailable",
                true,
                false,
            )
        };
        let cases = [
            ("empty", vec![]),
            ("zero", vec![attempt(0)]),
            ("starts-at-two", vec![attempt(2)]),
            ("gap", vec![attempt(1), attempt(3)]),
            ("duplicate", vec![attempt(1), attempt(1)]),
            ("wire-order-is-not-ordinal-order", vec![attempt(2), attempt(1)]),
        ];

        for (case, attempts) in cases {
            let error = decode_closed_contract_attempts("TEST_CODE_ATTEMPT_STRUCTURE", attempts);
            assert_attempt_trace_unsupported_without_top_level_change(&error, case);
        }
    }

    #[test]
    fn grpc_dual_contract_external_attempts_reject_illegal_outcome_reason_boolean_matrix() {
        let cases = [
            ("selected-reason", "selected", "transport", false, false),
            ("selected-retryable", "selected", "selected", true, false),
            ("selected-terminal", "selected", "selected", false, true),
            ("rejected-retryable", "rejected", "invalid_request", true, false),
            ("rejected-terminal", "rejected", "evidence", false, true),
            ("failed-selected-reason", "failed", "selected", false, false),
            ("failed-retry-required", "failed", "unavailable", false, true),
            ("failed-no-retry-required", "failed", "invalid_request", true, false),
        ];

        for (case, outcome, reason, retryable, terminal) in cases {
            let error = decode_closed_contract_attempts(
                "TEST_CODE_ATTEMPT_MATRIX",
                vec![closed_contract_attempt(
                    1,
                    "Cailianpress",
                    outcome,
                    reason,
                    retryable,
                    terminal,
                )],
            );
            assert_attempt_trace_unsupported_without_top_level_change(&error, case);
        }
    }

    #[test]
    fn grpc_dual_contract_external_attempts_without_capability_or_unknown_field_are_uninterpretable(
    ) {
        let cases = [
            ("no-same-endpoint-capability-evidence", "Cailianpress", "failed", "unavailable"),
            ("unknown-provider", "TEST_ONLY_UNKNOWN_PROVIDER", "failed", "unavailable"),
            ("unknown-outcome", "Cailianpress", "TEST_ONLY_UNKNOWN_OUTCOME", "unavailable"),
            ("unknown-reason", "Cailianpress", "failed", "TEST_ONLY_UNKNOWN_REASON"),
        ];

        for (case, provider, outcome, reason) in cases {
            let error = decode_closed_contract_attempts(
                "TEST_CODE_ATTEMPT_AUTHORITY",
                vec![closed_contract_attempt(1, provider, outcome, reason, true, false)],
            );
            assert_attempt_trace_unsupported_without_top_level_change(&error, case);
            assert!(
                !format!("{error:?}").contains("TEST_ONLY_UNKNOWN"),
                "{case}: bounded raw fields must remain redacted from Debug",
            );
        }
    }

    #[test]
    fn grpc_dual_contract_external_attempts_accept_complete_closed_reason_boolean_matrix() {
        const PUBLISHED: &str = "TEST_ONLY_ENDPOINT_PUBLISHED_PROVIDER";
        let selected = decode_validated_closed_contract_attempts(
            &[PUBLISHED],
            "TEST_CODE_ATTEMPT_SELECTED_MATRIX",
            vec![closed_contract_attempt(
                1,
                PUBLISHED,
                "selected",
                "selected",
                false,
                false,
            )],
        );
        assert_single_supported_attempt_without_top_level_change(
            &selected,
            (1, PUBLISHED, "selected", "selected", false, false),
            "selected/selected/false/false",
        );

        let rejected_reasons = [
            "authentication_rejected",
            "query_rejected",
            "response_invalid",
            "invalid_request",
            "unsupported",
            "unauthenticated",
            "permission_denied",
            "provider_route_exhausted",
            "provider_route_stopped",
            "source_precondition",
            "invalid_evidence",
            "internal",
            "transport",
            "timeout",
            "rate_limited",
            "no_data",
            "protocol",
            "quality",
            "evidence",
            "provider",
        ];
        for reason in rejected_reasons {
            let error = decode_validated_closed_contract_attempts(
                &[PUBLISHED],
                "TEST_CODE_ATTEMPT_REJECTED_MATRIX",
                vec![closed_contract_attempt(
                    1,
                    PUBLISHED,
                    "rejected",
                    reason,
                    false,
                    false,
                )],
            );
            assert_single_supported_attempt_without_top_level_change(
                &error,
                (1, PUBLISHED, "rejected", reason, false, false),
                reason,
            );
        }

        let retryable_failed_reasons = [
            "transport",
            "timeout",
            "rate_limited",
            "unavailable",
            "provider_busy",
            "worker_unavailable",
        ];
        for reason in retryable_failed_reasons {
            for terminal in [false, true] {
                let error = decode_validated_closed_contract_attempts(
                    &[PUBLISHED],
                    "TEST_CODE_ATTEMPT_FAILED_RETRYABLE_MATRIX",
                    vec![closed_contract_attempt(
                        1, PUBLISHED, "failed", reason, true, terminal,
                    )],
                );
                assert_single_supported_attempt_without_top_level_change(
                    &error,
                    (1, PUBLISHED, "failed", reason, true, terminal),
                    reason,
                );
            }
        }

        let terminal_failed_reasons = [
            "invalid_request",
            "unsupported",
            "no_data",
            "protocol",
            "quality",
            "evidence",
            "provider",
        ];
        for reason in terminal_failed_reasons {
            for terminal in [false, true] {
                let error = decode_validated_closed_contract_attempts(
                    &[PUBLISHED],
                    "TEST_CODE_ATTEMPT_FAILED_TERMINAL_MATRIX",
                    vec![closed_contract_attempt(
                        1, PUBLISHED, "failed", reason, false, terminal,
                    )],
                );
                assert_single_supported_attempt_without_top_level_change(
                    &error,
                    (1, PUBLISHED, "failed", reason, false, terminal),
                    reason,
                );
            }
        }
    }

    #[test]
    fn grpc_dual_contract_external_attempts_reject_complete_cross_and_boolean_matrix() {
        const PUBLISHED: &str = "TEST_ONLY_ENDPOINT_PUBLISHED_PROVIDER";
        let structure_cases = [
            ("ordinal-zero", vec![closed_contract_attempt(0, PUBLISHED, "selected", "selected", false, false)]),
            ("ordinal-start-two", vec![closed_contract_attempt(2, PUBLISHED, "selected", "selected", false, false)]),
            (
                "ordinal-gap",
                vec![
                    closed_contract_attempt(1, PUBLISHED, "selected", "selected", false, false),
                    closed_contract_attempt(3, PUBLISHED, "selected", "selected", false, false),
                ],
            ),
            (
                "ordinal-duplicate",
                vec![
                    closed_contract_attempt(1, PUBLISHED, "selected", "selected", false, false),
                    closed_contract_attempt(1, PUBLISHED, "selected", "selected", false, false),
                ],
            ),
            (
                "ordinal-reverse-wire-order",
                vec![
                    closed_contract_attempt(2, PUBLISHED, "selected", "selected", false, false),
                    closed_contract_attempt(1, PUBLISHED, "selected", "selected", false, false),
                ],
            ),
        ];
        for (case, attempts) in structure_cases {
            let error = decode_validated_closed_contract_attempts(
                &[PUBLISHED],
                "TEST_CODE_ATTEMPT_VALIDATED_STRUCTURE",
                attempts,
            );
            assert_attempt_trace_unsupported_without_top_level_change(&error, case);
        }

        let selected_invalid = [
            ("selected-wrong-reason", "transport", false, false),
            ("selected-retryable", "selected", true, false),
            ("selected-terminal", "selected", false, true),
            ("selected-both-bools", "selected", true, true),
        ];
        for (case, reason, retryable, terminal) in selected_invalid {
            let error = decode_validated_closed_contract_attempts(
                &[PUBLISHED],
                "TEST_CODE_ATTEMPT_SELECTED_NEGATIVE",
                vec![closed_contract_attempt(
                    1, PUBLISHED, "selected", reason, retryable, terminal,
                )],
            );
            assert_attempt_trace_unsupported_without_top_level_change(&error, case);
        }

        let rejected_reasons = [
            "authentication_rejected",
            "query_rejected",
            "response_invalid",
            "invalid_request",
            "unsupported",
            "unauthenticated",
            "permission_denied",
            "provider_route_exhausted",
            "provider_route_stopped",
            "source_precondition",
            "invalid_evidence",
            "internal",
            "transport",
            "timeout",
            "rate_limited",
            "no_data",
            "protocol",
            "quality",
            "evidence",
            "provider",
        ];
        for reason in rejected_reasons {
            for (retryable, terminal) in [(true, false), (false, true), (true, true)] {
                let error = decode_validated_closed_contract_attempts(
                    &[PUBLISHED],
                    "TEST_CODE_ATTEMPT_REJECTED_NEGATIVE",
                    vec![closed_contract_attempt(
                        1,
                        PUBLISHED,
                        "rejected",
                        reason,
                        retryable,
                        terminal,
                    )],
                );
                assert_attempt_trace_unsupported_without_top_level_change(&error, reason);
            }
        }
        for reason in ["selected", "unavailable", "provider_busy", "worker_unavailable"] {
            let error = decode_validated_closed_contract_attempts(
                &[PUBLISHED],
                "TEST_CODE_ATTEMPT_REJECTED_CROSS",
                vec![closed_contract_attempt(
                    1, PUBLISHED, "rejected", reason, false, false,
                )],
            );
            assert_attempt_trace_unsupported_without_top_level_change(&error, reason);
        }

        for reason in [
            "transport",
            "timeout",
            "rate_limited",
            "unavailable",
            "provider_busy",
            "worker_unavailable",
        ] {
            let error = decode_validated_closed_contract_attempts(
                &[PUBLISHED],
                "TEST_CODE_ATTEMPT_FAILED_RETRY_NEGATIVE",
                vec![closed_contract_attempt(
                    1, PUBLISHED, "failed", reason, false, false,
                )],
            );
            assert_attempt_trace_unsupported_without_top_level_change(&error, reason);
        }
        for reason in [
            "invalid_request",
            "unsupported",
            "no_data",
            "protocol",
            "quality",
            "evidence",
            "provider",
        ] {
            let error = decode_validated_closed_contract_attempts(
                &[PUBLISHED],
                "TEST_CODE_ATTEMPT_FAILED_TERMINAL_NEGATIVE",
                vec![closed_contract_attempt(
                    1, PUBLISHED, "failed", reason, true, false,
                )],
            );
            assert_attempt_trace_unsupported_without_top_level_change(&error, reason);
        }
        for reason in [
            "selected",
            "authentication_rejected",
            "query_rejected",
            "response_invalid",
            "unauthenticated",
            "permission_denied",
            "provider_route_exhausted",
            "provider_route_stopped",
            "source_precondition",
            "invalid_evidence",
            "internal",
        ] {
            let error = decode_validated_closed_contract_attempts(
                &[PUBLISHED],
                "TEST_CODE_ATTEMPT_FAILED_CROSS",
                vec![closed_contract_attempt(
                    1, PUBLISHED, "failed", reason, false, false,
                )],
            );
            assert_attempt_trace_unsupported_without_top_level_change(&error, reason);
        }

        let unknown_cases = [
            ("unknown-outcome", "TEST_ONLY_UNKNOWN_OUTCOME", "selected"),
            ("unknown-reason", "selected", "TEST_ONLY_UNKNOWN_REASON"),
        ];
        for (case, outcome, reason) in unknown_cases {
            let error = decode_validated_closed_contract_attempts(
                &[PUBLISHED],
                "TEST_CODE_ATTEMPT_UNKNOWN_NEGATIVE",
                vec![closed_contract_attempt(
                    1, PUBLISHED, outcome, reason, false, false,
                )],
            );
            assert_attempt_trace_unsupported_without_top_level_change(&error, case);
            assert!(!format!("{error:?}").contains("TEST_ONLY_UNKNOWN"), "{case}");
        }
    }

    #[test]
    fn grpc_dual_contract_external_attempts_bind_exact_published_provider_identity() {
        let provider_64 = "p".repeat(64);
        let provider_65 = "p".repeat(65);
        for provider in [provider_64.as_str(), "TEST_ONLY_ENDPOINT_PUBLISHED_PROVIDER"] {
            let error = decode_validated_closed_contract_attempts(
                &[provider],
                "TEST_CODE_ATTEMPT_PROVIDER_POSITIVE",
                vec![closed_contract_attempt(
                    1, provider, "selected", "selected", false, false,
                )],
            );
            assert_single_supported_attempt_without_top_level_change(
                &error,
                (1, provider, "selected", "selected", false, false),
                provider,
            );
        }

        let provider_cases = [
            ("empty", "", ""),
            ("sixty-five", provider_65.as_str(), provider_65.as_str()),
            ("control", "Eastmoney\n", "Eastmoney\n"),
            ("case-mismatch", "Eastmoney", "eastmoney"),
            ("unpublished-local-known", "Eastmoney", "Cailianpress"),
        ];
        for (case, published, attempted) in provider_cases {
            let error = decode_validated_closed_contract_attempts(
                &[published],
                "TEST_CODE_ATTEMPT_PROVIDER_NEGATIVE",
                vec![closed_contract_attempt(
                    1, attempted, "selected", "selected", false, false,
                )],
            );
            assert_attempt_trace_unsupported_without_top_level_change(&error, case);
        }
    }

    #[test]
    fn grpc_dual_contract_online_and_restored_status_decoders_match_by_profile() {
        use crate::grpc_client::external_pb::magic::market::v1::{
            Operation as ExternalOperation, ProviderAttemptDetail,
        };
        use crate::grpc_client::pb::magic::market::v1::Operation as LocalOperation;

        let local_bytes = local_detail("TEST_CODE_LOCAL_PARITY", LocalOperation::GlobalNews)
            .encode_to_vec();
        let local_method = MethodIdentity::from_client_operation(
            ContractProfile::LocalBridgeV1,
            LocalOperation::GlobalNews,
        )
        .expect("Local method");
        let local_online = GrpcError::from_status(
            tonic::Status::with_details(
                Code::Unavailable,
                "provider unavailable",
                local_bytes.clone().into(),
            ),
            StatusErrorContext::data(local_method, "TEST_CODE_LOCAL_PARITY"),
        );
        let local_restored = restore_persisted_status_error(
            Code::Unavailable as i32,
            &local_bytes,
            PersistedErrorDetailTrailer::Absent,
            Some("[redacted-unclassified-status]"),
            StatusErrorContext::data(local_method, "TEST_CODE_LOCAL_PARITY"),
        )
        .expect("restored Local status");
        assert_eq!(local_online, local_restored);

        let external_bytes = external_detail(
            "TEST_CODE_EXTERNAL_PARITY",
            ExternalOperation::GlobalNews,
            vec![ProviderAttemptDetail {
                ordinal: 1,
                provider: "Jin10".to_owned(),
                outcome: "TEST_CODE_EXTERNAL_OUTCOME".to_owned(),
                reason_code: "provider_rate_limited".to_owned(),
                retryable: true,
                terminal: false,
            }],
        )
        .encode_to_vec();
        let external_method = external_global_news_method();
        let external_online = GrpcError::from_status(
            tonic::Status::with_details(
                Code::Unavailable,
                "provider unavailable",
                external_bytes.clone().into(),
            ),
            StatusErrorContext::data(external_method, "TEST_CODE_EXTERNAL_PARITY"),
        );
        let external_restored = restore_persisted_status_error(
            Code::Unavailable as i32,
            &external_bytes,
            PersistedErrorDetailTrailer::Absent,
            Some("[redacted-unclassified-status]"),
            StatusErrorContext::data(external_method, "TEST_CODE_EXTERNAL_PARITY"),
        )
        .expect("restored External status");
        assert_eq!(external_online, external_restored);
    }

    #[test]
    fn details_getter_covers_all_variants() {
        // 每个变体都可通过 details() 拿到 ErrorDetail (不 panic)。
        let status = tonic::Status::with_details(
            Code::Unimplemented,
            "未实现",
            crate::grpc_client::pb::magic::market::v1::ErrorDetail::default()
                .encode_to_vec()
                .into(),
        );
        let err = GrpcError::from(status);
        assert!(matches!(err, GrpcError::Unimplemented { .. }));
        // tonic: Code::Unimplemented.to_string() = grpc 规范英文描述。
        assert_eq!(err.details().code, Code::Unimplemented.to_string());
    }
}
