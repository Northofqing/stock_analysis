//! gRPC status code → 项目错误类型 (合同 §10 错误映射表)。
//! 不依赖自然语言 message 做程序分支; ErrorDetail 从 status details 解码。
//! D2 (错误分类保真): 所有变体携带 details — 服务端 Fetch 失败时
//! handlers.rs 附加 ErrorDetail (provider/reason_code/retryable), 客户端
//! 桥据此重建 GatewayError 分类 (grpc_source.rs query_op), 不再折叠为
//! 默认 unavailable+provider=None (BR-170 生产日志 pre-fix 形态)。
use prost::Message; // ErrorDetail::decode (tonic 0.14 details() 返回 &[u8])
use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error, Clone, PartialEq)]
pub enum GrpcError {
    #[error("请求参数错误 (不重试)")]
    InvalidArgument { details: ErrorDetail },
    #[error("认证失败 (刷新凭据)")]
    Unauthenticated { details: ErrorDetail },
    #[error("无权限调用该能力 (停止调用)")]
    PermissionDenied { details: ErrorDetail },
    #[error("能力未准入或不支持 (不重试)")]
    Unimplemented { details: ErrorDetail },
    #[error("资源受限 (退避; 流消费者记录 gap)")]
    ResourceExhausted { details: ErrorDetail },
    #[error("超时 (有界重试, 保留原 request_id)")]
    DeadlineExceeded { details: ErrorDetail },
    #[error("服务不可用 (指数退避, 重新检查 health/capabilities)")]
    Unavailable { details: ErrorDetail },
    #[error("数据完整性/连续性失败 (不能当空成功)")]
    FailedPrecondition { details: ErrorDetail },
    #[error("服务端内部错误 (记录 request_id, 停止无界重试)")]
    Internal { details: ErrorDetail },
    #[error("未知错误 (code={code})", code = details.code)]
    Unknown { details: ErrorDetail },
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
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ErrorDetail {
    pub code: String,
    pub request_id: Option<String>,
    pub operation: Option<i32>,
    pub provider: Option<String>,
    pub reason_code: Option<String>,
    pub retryable: Option<bool>,
    /// Bounded, secret-screened server diagnostic for operator evidence only.
    /// Program flow must continue to branch exclusively on typed fields above.
    pub diagnostic_message: Option<Box<DiagnosticMessage>>,
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

fn known_provider(value: &str) -> Option<crate::magic_compat::ProviderId> {
    use crate::magic_compat::ProviderId;

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
    ProviderUnavailable,
}

impl KnownReasonCode {
    fn from_wire(value: &str) -> Option<Self> {
        Some(match value {
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
            "provider_unavailable" => Self::ProviderUnavailable,
            _ => return None,
        })
    }

    const fn as_str(self) -> &'static str {
        match self {
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
            Self::ProviderUnavailable => "provider_unavailable",
        }
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

fn safe_wire_operation(value: i32) -> Option<i32> {
    use crate::grpc_client::pb::magic::market::v1::Operation;

    Operation::try_from(value)
        .ok()
        .filter(|operation| *operation != Operation::Unspecified)
        .map(|operation| operation as i32)
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

impl From<tonic::Status> for GrpcError {
    fn from(status: tonic::Status) -> Self {
        let diagnostic_message = safe_status_message(status.message());
        // 尝试解码 ErrorDetail (合同 §10: request ID/operation/provider/reason code/retryable)。
        // tonic 0.14 重构: details() 返回原始 &[u8] (不再是 Box<dyn Any>),
        // 用 prost::Message::decode 直接解码; 解码失败则忽略, 用 code 分支即可。
        // 空 details → 纯默认 (prost 会把空 bytes 解码为全默认值, 语义上等于没有 ErrorDetail)。
        let detail = if status.details().is_empty() {
            ErrorDetail {
                code: status.code().to_string(),
                diagnostic_message: diagnostic_message.clone(),
                ..Default::default()
            }
        } else {
            crate::grpc_client::pb::magic::market::v1::ErrorDetail::decode(status.details())
                .ok()
                .map(|d| ErrorDetail {
                    code: status.code().to_string(),
                    request_id: request_id_correlation(&d.request_id),
                    operation: safe_wire_operation(d.operation),
                    provider: safe_wire_provider(&d.provider),
                    reason_code: safe_wire_reason_code(&d.reason_code),
                    retryable: Some(d.retryable),
                    diagnostic_message: diagnostic_message.clone(),
                })
                .unwrap_or_else(|| ErrorDetail {
                    code: status.code().to_string(),
                    diagnostic_message: diagnostic_message.clone(),
                    ..Default::default()
                })
        };

        // D2: 每个变体都携带 detail — 即便非 Fetch 错误码也保留 request_id 供日志/审计。
        match status.code() {
            tonic::Code::InvalidArgument => GrpcError::InvalidArgument { details: detail },
            tonic::Code::Unauthenticated => GrpcError::Unauthenticated { details: detail },
            tonic::Code::PermissionDenied => GrpcError::PermissionDenied { details: detail },
            tonic::Code::Unimplemented => GrpcError::Unimplemented { details: detail },
            tonic::Code::ResourceExhausted => GrpcError::ResourceExhausted { details: detail },
            tonic::Code::DeadlineExceeded => GrpcError::DeadlineExceeded { details: detail },
            tonic::Code::Unavailable => GrpcError::Unavailable { details: detail },
            tonic::Code::FailedPrecondition => GrpcError::FailedPrecondition { details: detail },
            tonic::Code::Internal => GrpcError::Internal { details: detail },
            _ => GrpcError::Unknown { details: detail },
        }
    }
}

impl From<crate::grpc_client::auth::AuthError> for GrpcError {
    fn from(_: crate::grpc_client::auth::AuthError) -> Self {
        // token 含非法字符无法注入 metadata → 请求根本到不了服务端, 语义上等同认证失败。
        GrpcError::Unauthenticated {
            details: ErrorDetail {
                code: "unauthenticated".to_string(),
                ..Default::default()
            },
        }
    }
}

impl From<crate::grpc_client::envelope::EnvelopeError> for GrpcError {
    fn from(_: crate::grpc_client::envelope::EnvelopeError) -> Self {
        // 信封构造失败是客户端本地错误 (序列化失败/未冻结 schema), 非服务端状态。
        // 映射 Unknown + code=envelope, 与响应侧信封校验失败同码 (见 client.rs query)。
        GrpcError::Unknown {
            details: ErrorDetail {
                code: "envelope".to_string(),
                ..Default::default()
            },
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
                    details: detail_for(Code::InvalidArgument),
                },
            ),
            (
                Code::Unauthenticated,
                GrpcError::Unauthenticated {
                    details: detail_for(Code::Unauthenticated),
                },
            ),
            (
                Code::PermissionDenied,
                GrpcError::PermissionDenied {
                    details: detail_for(Code::PermissionDenied),
                },
            ),
            (
                Code::Unimplemented,
                GrpcError::Unimplemented {
                    details: detail_for(Code::Unimplemented),
                },
            ),
            (
                Code::ResourceExhausted,
                GrpcError::ResourceExhausted {
                    details: detail_for(Code::ResourceExhausted),
                },
            ),
            (
                Code::DeadlineExceeded,
                GrpcError::DeadlineExceeded {
                    details: detail_for(Code::DeadlineExceeded),
                },
            ),
            (
                Code::Unavailable,
                GrpcError::Unavailable {
                    details: detail_for(Code::Unavailable),
                },
            ),
            (
                Code::FailedPrecondition,
                GrpcError::FailedPrecondition {
                    details: detail_for(Code::FailedPrecondition),
                },
            ),
            (
                Code::Internal,
                GrpcError::Internal {
                    details: detail_for(Code::Internal),
                },
            ),
            // tonic 0.14: Code::Unknown.to_string() = "Unknown error" (grpc 规范英文描述)。
            (
                Code::Unknown,
                GrpcError::Unknown {
                    details: ErrorDetail {
                        code: "Unknown error".into(),
                        diagnostic_message: Some(Box::new(DiagnosticMessage::new(
                            "[redacted-unclassified-status]",
                        ))),
                        ..Default::default()
                    },
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
        assert_eq!(detail.operation, None);
        assert_eq!(detail.provider, None);
        assert_eq!(detail.reason_code.as_deref(), Some("internal"));
        assert_eq!(detail.retryable, Some(true));
        assert!(!format!("{error:?}").contains("TEST_ONLY_SECRET"));

        let classified = crate::grpc_client::pb::magic::market::v1::ErrorDetail {
            request_id: "TEST_CODE_CLASSIFIED_REQUEST".to_string(),
            operation: crate::grpc_client::pb::magic::market::v1::Operation::ProviderTopNRankings
                as i32,
            provider: "Tdx".to_string(),
            reason_code: "invalid_evidence".to_string(),
            retryable: false,
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
            detail.operation,
            Some(crate::grpc_client::pb::magic::market::v1::Operation::ProviderTopNRankings as i32)
        );
        assert_eq!(detail.provider.as_deref(), Some("Tdx"));
        assert_eq!(detail.reason_code.as_deref(), Some("invalid_evidence"));
        assert_eq!(detail.retryable, Some(false));
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
