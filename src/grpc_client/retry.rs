//! 有界重试与指数退避 (合同 §10)。
//! UNAVAILABLE → 指数退避 + 重查 health; DEADLINE_EXCEEDED → 有界重试保留原 request_id。
use crate::grpc_client::errors::GrpcError;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RetryDecision {
    RetryBackoff, // UNAVAILABLE: 指数退避
    RetryBounded, // DEADLINE_EXCEEDED: 固定次数
    NoRetry,
}

/// §10 表: 每个错误码的重试决策。
pub fn retry_decision(err: &GrpcError) -> RetryDecision {
    if err.details().retryable == Some(false) {
        return RetryDecision::NoRetry;
    }
    if err.details().retryable == Some(true) {
        return match err {
            // Request/auth/permission/contract errors remain non-retryable even if a
            // remote endpoint sends contradictory metadata.
            GrpcError::InvalidArgument { .. }
            | GrpcError::Unauthenticated { .. }
            | GrpcError::PermissionDenied { .. }
            | GrpcError::Unimplemented { .. } => RetryDecision::NoRetry,
            GrpcError::DeadlineExceeded { .. } => RetryDecision::RetryBounded,
            _ => RetryDecision::RetryBackoff,
        };
    }
    match err {
        // No explicit detail: fall back to the frozen transport-code table.
        GrpcError::Unavailable { .. } => RetryDecision::RetryBackoff,
        GrpcError::DeadlineExceeded { .. } => RetryDecision::RetryBounded,
        _ => RetryDecision::NoRetry,
    }
}

#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_attempts: u32, // 总尝试次数 (含首次)
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
    pub jitter_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 4, // 首次 + 3 次退避
            base_delay_ms: 1000,
            max_delay_ms: 60_000,
            jitter_ms: 200,
        }
    }
}

impl RetryPolicy {
    /// 第 attempt 次 (1-based) 重试前的等待时长, 指数退避 + jitter。
    pub fn backoff(&self, attempt: u32) -> std::time::Duration {
        let exponent = attempt.saturating_sub(1).min(6);
        let base = self.base_delay_ms << exponent;
        let capped = base.min(self.max_delay_ms);
        std::time::Duration::from_millis(capped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grpc_client::errors::ErrorDetail;

    fn unavail() -> GrpcError {
        GrpcError::Unavailable {
            details: Box::default(),
        }
    }

    #[test]
    fn decision_follows_contract_table() {
        assert_eq!(retry_decision(&unavail()), RetryDecision::RetryBackoff);
        assert_eq!(
            retry_decision(&GrpcError::DeadlineExceeded {
                details: Box::default()
            }),
            RetryDecision::RetryBounded
        );
        assert_eq!(
            retry_decision(&GrpcError::InvalidArgument {
                details: Box::default()
            }),
            RetryDecision::NoRetry
        );
        assert_eq!(
            retry_decision(&GrpcError::Unauthenticated {
                details: Box::default()
            }),
            RetryDecision::NoRetry
        );
        assert_eq!(
            retry_decision(&GrpcError::PermissionDenied {
                details: Box::default()
            }),
            RetryDecision::NoRetry
        );
        assert_eq!(
            retry_decision(&GrpcError::Unimplemented {
                details: Box::default()
            }),
            RetryDecision::NoRetry
        );
        assert_eq!(
            retry_decision(&GrpcError::ResourceExhausted {
                details: Box::default()
            }),
            RetryDecision::NoRetry
        );
        assert_eq!(
            retry_decision(&GrpcError::FailedPrecondition {
                details: Box::default()
            }),
            RetryDecision::NoRetry
        );
        assert_eq!(
            retry_decision(&GrpcError::Internal {
                details: Box::default()
            }),
            RetryDecision::NoRetry
        );
    }

    #[test]
    fn explicit_retryable_detail_overrides_transport_default_safely() {
        let non_retryable = ErrorDetail {
            retryable: Some(false),
            ..Default::default()
        };
        assert_eq!(
            retry_decision(&GrpcError::Unavailable {
                details: Box::new(non_retryable)
            }),
            RetryDecision::NoRetry
        );

        let retryable = ErrorDetail {
            retryable: Some(true),
            ..Default::default()
        };
        assert_eq!(
            retry_decision(&GrpcError::Internal {
                details: Box::new(retryable),
            }),
            RetryDecision::RetryBackoff,
            "delegate failures arrive as Internal with authoritative retryability"
        );

        let invalid = ErrorDetail {
            retryable: Some(true),
            ..Default::default()
        };
        assert_eq!(
            retry_decision(&GrpcError::InvalidArgument {
                details: Box::new(invalid),
            }),
            RetryDecision::NoRetry,
            "an invalid request is never made retryable by remote metadata"
        );
    }

    #[test]
    fn backoff_is_exponential_and_capped() {
        let p = RetryPolicy::default();
        assert!(p.backoff(1) < p.backoff(2));
        assert!(p.backoff(3) < p.backoff(4));
        assert!(p.backoff(10) <= std::time::Duration::from_millis(60_000));
    }
}
