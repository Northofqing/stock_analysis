//! Bounded, log-safe ExternalV1 provider-attempt evidence.

use std::collections::BTreeSet;

use crate::grpc_client::external_pb::magic::market::v1::{
    CapabilitiesResponse, ProviderAttemptDetail,
};

pub(crate) const MAX_PROVIDER_ATTEMPTS: usize = 16;
const MAX_PROVIDER_IDENTITY_CHARS: usize = 64;

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ExternalProviderCatalog {
    providers: BTreeSet<String>,
}

impl ExternalProviderCatalog {
    pub(crate) fn from_request_id_validated_capabilities(
        response: &CapabilitiesResponse,
    ) -> Self {
        Self {
            providers: response
                .capabilities
                .iter()
                .map(|capability| capability.provider.clone())
                .collect(),
        }
    }

    fn contains(&self, provider: &str) -> bool {
        self.providers.contains(provider)
    }
}

impl std::fmt::Debug for ExternalProviderCatalog {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ExternalProviderCatalog")
            .field("provider_count", &self.providers.len())
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ProviderAttemptText(String);

impl ProviderAttemptText {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn new(value: String) -> Self {
        Self(value)
    }
}

impl std::fmt::Debug for ProviderAttemptText {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ProviderAttemptText([redacted])")
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum ProviderAttemptValue {
    Supported(ProviderAttemptText),
    Unsupported(ProviderAttemptText),
}

impl ProviderAttemptValue {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Supported(value) | Self::Unsupported(value) => value.as_str(),
        }
    }

    pub const fn is_supported(&self) -> bool {
        matches!(self, Self::Supported(_))
    }

    pub(crate) fn new(value: String, supported: bool) -> Self {
        let value = ProviderAttemptText::new(value);
        if supported {
            Self::Supported(value)
        } else {
            Self::Unsupported(value)
        }
    }
}

impl std::fmt::Debug for ProviderAttemptValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Supported(value) => formatter.debug_tuple("Supported").field(value).finish(),
            Self::Unsupported(value) => formatter.debug_tuple("Unsupported").field(value).finish(),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ProviderAttempt {
    pub ordinal: u32,
    pub provider: ProviderAttemptValue,
    pub outcome: ProviderAttemptValue,
    pub reason_code: ProviderAttemptValue,
    pub retryable: bool,
    pub terminal: bool,
}

impl std::fmt::Debug for ProviderAttempt {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProviderAttempt")
            .field("ordinal", &self.ordinal)
            .field("provider", &self.provider)
            .field("outcome", &self.outcome)
            .field("reason_code", &self.reason_code)
            .field("retryable", &self.retryable)
            .field("terminal", &self.terminal)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum ProviderAttempts {
    Accepted(Vec<ProviderAttempt>),
    Rejected { observed_count: usize },
}

impl ProviderAttempts {
    pub fn accepted(&self) -> Option<&[ProviderAttempt]> {
        match self {
            Self::Accepted(attempts) => Some(attempts),
            Self::Rejected { .. } => None,
        }
    }

    pub const fn is_rejected(&self) -> bool {
        matches!(self, Self::Rejected { .. })
    }

    pub(crate) fn from_external_wire(
        attempts: Vec<ProviderAttemptDetail>,
        catalog: Option<&ExternalProviderCatalog>,
    ) -> Self {
        let observed_count = attempts.len();
        let Some(catalog) = catalog else {
            return Self::Rejected { observed_count };
        };
        if !(1..=MAX_PROVIDER_ATTEMPTS).contains(&observed_count) {
            return Self::Rejected { observed_count };
        }

        let mut projected = Vec::with_capacity(observed_count);
        for (index, attempt) in attempts.into_iter().enumerate() {
            let expected_ordinal = u32::try_from(index + 1)
                .expect("provider-attempt count is bounded at sixteen");
            if attempt.ordinal != expected_ordinal
                || !valid_provider_identity(&attempt.provider)
                || !catalog.contains(&attempt.provider)
                || !valid_outcome_contract(
                    &attempt.outcome,
                    &attempt.reason_code,
                    attempt.retryable,
                    attempt.terminal,
                )
            {
                return Self::Rejected { observed_count };
            }
            projected.push(ProviderAttempt {
                ordinal: attempt.ordinal,
                provider: ProviderAttemptValue::new(attempt.provider, true),
                outcome: ProviderAttemptValue::new(attempt.outcome, true),
                reason_code: ProviderAttemptValue::new(attempt.reason_code, true),
                retryable: attempt.retryable,
                terminal: attempt.terminal,
            });
        }
        Self::Accepted(projected)
    }
}

fn valid_provider_identity(provider: &str) -> bool {
    !provider.is_empty()
        && provider.chars().count() <= MAX_PROVIDER_IDENTITY_CHARS
        && !provider.chars().any(char::is_control)
}

fn valid_outcome_contract(
    outcome: &str,
    reason_code: &str,
    retryable: bool,
    terminal: bool,
) -> bool {
    match outcome {
        "selected" => reason_code == "selected" && !retryable && !terminal,
        "rejected" => {
            !retryable
                && !terminal
                && matches!(
                    reason_code,
                    "authentication_rejected"
                        | "query_rejected"
                        | "response_invalid"
                        | "invalid_request"
                        | "unsupported"
                        | "unauthenticated"
                        | "permission_denied"
                        | "provider_route_exhausted"
                        | "provider_route_stopped"
                        | "source_precondition"
                        | "invalid_evidence"
                        | "internal"
                        | "transport"
                        | "timeout"
                        | "rate_limited"
                        | "no_data"
                        | "protocol"
                        | "quality"
                        | "evidence"
                        | "provider"
                )
        }
        "failed" => match reason_code {
            "transport" | "timeout" | "rate_limited" | "unavailable" | "provider_busy"
            | "worker_unavailable" => retryable,
            "invalid_request" | "unsupported" | "no_data" | "protocol" | "quality"
            | "evidence" | "provider" => !retryable,
            _ => false,
        },
        _ => false,
    }
}

impl Default for ProviderAttempts {
    fn default() -> Self {
        Self::Accepted(Vec::new())
    }
}

impl std::fmt::Debug for ProviderAttempts {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Accepted(attempts) => formatter.debug_tuple("Accepted").field(attempts).finish(),
            Self::Rejected { observed_count } => formatter
                .debug_struct("Rejected")
                .field("observed_count", observed_count)
                .finish(),
        }
    }
}
