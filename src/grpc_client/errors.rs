//! gRPC status code → 项目错误类型 (合同 §10 错误映射表)。
//! 不依赖自然语言 message 做程序分支; ErrorDetail 从 status details 解码。
use prost::Message; // ErrorDetail::decode (tonic 0.14 details() 返回 &[u8])

#[derive(Debug, thiserror::Error, Clone, PartialEq)]
pub enum GrpcError {
    #[error("请求参数错误 (不重试)")]
    InvalidArgument,
    #[error("认证失败 (刷新凭据)")]
    Unauthenticated,
    #[error("无权限调用该能力 (停止调用)")]
    PermissionDenied,
    #[error("能力未准入或不支持 (不重试)")]
    Unimplemented,
    #[error("资源受限 (退避; 流消费者记录 gap)")]
    ResourceExhausted,
    #[error("超时 (有界重试, 保留原 request_id)")]
    DeadlineExceeded,
    #[error("服务不可用 (指数退避, 重新检查 health/capabilities)")]
    Unavailable,
    #[error("数据完整性/连续性失败 (不能当空成功)")]
    FailedPrecondition,
    #[error("服务端内部错误 (记录 request_id, 停止无界重试)")]
    Internal,
    #[error("未知错误 (code={code})", code = details.code)]
    Unknown { details: ErrorDetail },
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
            ErrorDetail { code: status.code().to_string(), ..Default::default() }
        } else {
            crate::grpc_client::pb::magic::market::v1::ErrorDetail::decode(status.details())
                .ok()
                .map(|d| ErrorDetail {
                    code: status.code().to_string(),
                    request_id: if d.request_id.is_empty() { None } else { Some(d.request_id.clone()) },
                    operation: Some(d.operation),
                    provider: if d.provider.is_empty() { None } else { Some(d.provider.clone()) },
                    reason_code: if d.reason_code.is_empty() { None } else { Some(d.reason_code.clone()) },
                    retryable: Some(d.retryable),
                })
                .unwrap_or_else(|| ErrorDetail { code: status.code().to_string(), ..Default::default() })
        };

        match status.code() {
            tonic::Code::InvalidArgument => GrpcError::InvalidArgument,
            tonic::Code::Unauthenticated => GrpcError::Unauthenticated,
            tonic::Code::PermissionDenied => GrpcError::PermissionDenied,
            tonic::Code::Unimplemented => GrpcError::Unimplemented,
            tonic::Code::ResourceExhausted => GrpcError::ResourceExhausted,
            tonic::Code::DeadlineExceeded => GrpcError::DeadlineExceeded,
            tonic::Code::Unavailable => GrpcError::Unavailable,
            tonic::Code::FailedPrecondition => GrpcError::FailedPrecondition,
            tonic::Code::Internal => GrpcError::Internal,
            _ => GrpcError::Unknown { details: detail },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonic::Code;

    #[test]
    fn maps_all_contract_codes() {
        let cases = [
            (Code::InvalidArgument, GrpcError::InvalidArgument),
            (Code::Unauthenticated, GrpcError::Unauthenticated),
            (Code::PermissionDenied, GrpcError::PermissionDenied),
            (Code::Unimplemented, GrpcError::Unimplemented),
            (Code::ResourceExhausted, GrpcError::ResourceExhausted),
            (Code::DeadlineExceeded, GrpcError::DeadlineExceeded),
            (Code::Unavailable, GrpcError::Unavailable),
            (Code::FailedPrecondition, GrpcError::FailedPrecondition),
            (Code::Internal, GrpcError::Internal),
            // tonic 0.14: Code::Unknown.to_string() = "Unknown error" (grpc 规范英文描述)。
            (Code::Unknown, GrpcError::Unknown { details: ErrorDetail { code: "Unknown error".into(), ..Default::default() } }),
        ];
        for (code, expected) in cases {
            let status = tonic::Status::new(code, "msg");
            assert_eq!(GrpcError::from(status), expected, "code {code:?}");
        }
    }
}
