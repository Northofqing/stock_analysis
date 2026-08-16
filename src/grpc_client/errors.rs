//! gRPC status code → 项目错误类型 (合同 §10 错误映射表)。
//! 不依赖自然语言 message 做程序分支; ErrorDetail 从 status details 解码。
//! D2 (错误分类保真): 所有变体携带 details — 服务端 Fetch 失败时
//! handlers.rs 附加 ErrorDetail (provider/reason_code/retryable), 客户端
//! 桥据此重建 GatewayError 分类 (grpc_source.rs query_op), 不再折叠为
//! 默认 unavailable+provider=None (BR-170 生产日志 pre-fix 形态)。
use prost::Message; // ErrorDetail::decode (tonic 0.14 details() 返回 &[u8])

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
}

impl From<tonic::Status> for GrpcError {
    fn from(status: tonic::Status) -> Self {
        // 尝试解码 ErrorDetail (合同 §10: request ID/operation/provider/reason code/retryable)。
        // tonic 0.14 重构: details() 返回原始 &[u8] (不再是 Box<dyn Any>),
        // 用 prost::Message::decode 直接解码; 解码失败则忽略, 用 code 分支即可。
        // 空 details → 纯默认 (prost 会把空 bytes 解码为全默认值, 语义上等于没有 ErrorDetail)。
        let detail = if status.details().is_empty() {
            ErrorDetail {
                code: status.code().to_string(),
                ..Default::default()
            }
        } else {
            crate::grpc_client::pb::magic::market::v1::ErrorDetail::decode(status.details())
                .ok()
                .map(|d| ErrorDetail {
                    code: status.code().to_string(),
                    request_id: if d.request_id.is_empty() {
                        None
                    } else {
                        Some(d.request_id.clone())
                    },
                    operation: Some(d.operation),
                    provider: if d.provider.is_empty() {
                        None
                    } else {
                        Some(d.provider.clone())
                    },
                    reason_code: if d.reason_code.is_empty() {
                        None
                    } else {
                        Some(d.reason_code.clone())
                    },
                    retryable: Some(d.retryable),
                })
                .unwrap_or_else(|| ErrorDetail {
                    code: status.code().to_string(),
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
    /// 必须保真解码到 GrpcError.details(), 桥据此重建 GatewayError 分类。
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
        assert_eq!(err.details().request_id.as_deref(), Some("req-42"));

        // Unavailable 也带 detail (非 Fetch 路径同样保留 request_id 供审计)。
        detail.request_id = "req-43".to_string();
        let status = tonic::Status::with_details(
            Code::Unavailable,
            "连接被拒绝",
            detail.encode_to_vec().into(),
        );
        let err = GrpcError::from(status);
        assert!(matches!(err, GrpcError::Unavailable { .. }));
        assert_eq!(err.details().request_id.as_deref(), Some("req-43"));
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
