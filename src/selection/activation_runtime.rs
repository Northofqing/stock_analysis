//! BR-193 closed runtime activation vocabulary.
//!
//! These enums deliberately expose no caller-provided string escape hatch.
//! Storage-backed capability construction remains private to the activation
//! owner added by BR-193.

use serde::{Deserialize, Serialize};

/// Fail-closed reasons that keep generation providers and schedulers
/// unconstructed.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionDisabledReason {
    SchemaNotAmended,
    ProposalMissing,
    BoardArtifactUnverified,
    BoardArtifactExpired,
    ActivationMissing,
    ActivationNotEffective,
    ActivationExpired,
    ActivationUnreceipted,
    ActivationRevoked,
    TradingCalendarMissing,
    TradingCalendarUnverified,
    TradingCalendarCoverageIncomplete,
    IngressContractUnavailable,
}

impl SelectionDisabledReason {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::SchemaNotAmended => "schema_not_amended",
            Self::ProposalMissing => "proposal_missing",
            Self::BoardArtifactUnverified => "board_artifact_unverified",
            Self::BoardArtifactExpired => "board_artifact_expired",
            Self::ActivationMissing => "activation_missing",
            Self::ActivationNotEffective => "activation_not_effective",
            Self::ActivationExpired => "activation_expired",
            Self::ActivationUnreceipted => "activation_unreceipted",
            Self::ActivationRevoked => "activation_revoked",
            Self::TradingCalendarMissing => "trading_calendar_missing",
            Self::TradingCalendarUnverified => "trading_calendar_unverified",
            Self::TradingCalendarCoverageIncomplete => "trading_calendar_coverage_incomplete",
            Self::IngressContractUnavailable => "ingress_contract_unavailable",
        }
    }
}

/// The only outcome capability reason released with generation-only
/// activation.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeDisabledReason {
    OutcomeActivationNotReleased,
}
