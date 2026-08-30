//! BR-159/BR-160 deterministic A-10 chain-intelligence derivation.
//!
//! Transport stays outside this pure seam.  Provider adapters normalize their
//! admitted batches into these facts, then this module performs only validated
//! identity joins, filtering, ordering and immutable batch construction.

use crate::data_gateway::review::GatewayError;

use serde::Serialize;

use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainIntelligencePolicy {
    pub calculation_version: String,
    pub taxonomy_version: String,
    pub min_members: usize,
    pub excluded_board_names: BTreeSet<String>,
}

impl TryFrom<&crate::config::ChainIntelligenceConfig> for ChainIntelligencePolicy {
    type Error = GatewayError;

    fn try_from(config: &crate::config::ChainIntelligenceConfig) -> Result<Self, Self::Error> {
        config.validate().map_err(|error| {
            GatewayError::classified(
                "A-10",
                None,
                "invalid_request",
                "chain_policy_invalid",
                false,
                error,
            )
        })?;
        Ok(Self {
            calculation_version: config.calculation_version.clone(),
            taxonomy_version: config.taxonomy_version.clone(),
            min_members: config.min_members,
            excluded_board_names: config
                .excluded_board_names
                .iter()
                .map(|name| name.trim().to_owned())
                .collect(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChainSourceEvidence {
    pub capability: String,
    pub provider: String,
    pub source: String,
    pub source_at: Option<String>,
    pub observed_at: String,
    pub batch_id: String,
    /// SHA-256 of the canonical admitted source records, not merely the batch
    /// identifier.
    pub records_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UpperLimitFact {
    pub instrument_id: String,
    pub security_name: String,
    pub streak: u32,
    pub source_event_id: String,
}

/// Source-backed item that cannot enter the derivation seam.  Its raw identity
/// is retained only through a one-way hash in immutable rejection evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChainSourceRejection {
    pub identity: String,
    pub reason_code: String,
    pub retryable: bool,
}
