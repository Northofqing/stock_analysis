use chrono::{DateTime, Local, NaiveDate};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CandidateIdentity(String);

impl CandidateIdentity {
    pub fn new(
        event_id: &str,
        chain_id: &str,
        stock_code: &str,
        relation_version: &str,
        feature_version: &str,
        evaluation_market_date: NaiveDate,
    ) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"stock_analysis.selection_candidate.v1\0");
        for value in [
            event_id,
            chain_id,
            stock_code,
            relation_version,
            feature_version,
        ] {
            hasher.update(value.as_bytes());
            hasher.update(b"\0");
        }
        hasher.update(evaluation_market_date.to_string().as_bytes());
        Self(format!(
            "selection_candidate_v1_{}",
            hex::encode(hasher.finalize())
        ))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SecurityMarket {
    Shanghai,
    Shenzhen,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SecurityIdentity {
    pub code: String,
    pub name: String,
    pub market: SecurityMarket,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityMasterSnapshot {
    identities: Vec<SecurityIdentity>,
    pub batch_id: String,
    pub observed_at: DateTime<Local>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityMasterError {
    reason_code: &'static str,
    message: String,
}

impl SecurityMasterError {
    pub fn reason_code(&self) -> &'static str {
        self.reason_code
    }
}

impl std::fmt::Display for SecurityMasterError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for SecurityMasterError {}

impl SecurityMasterSnapshot {
    pub fn new(
        mut identities: Vec<SecurityIdentity>,
        batch_id: String,
        observed_at: DateTime<Local>,
    ) -> Result<Self, SecurityMasterError> {
        if batch_id.trim().is_empty() {
            return Err(master_error(
                "security_master_batch_id_empty",
                "security master batch ID is empty",
            ));
        }
        if identities.is_empty() {
            return Err(master_error(
                "security_master_empty",
                "security master contains no identities",
            ));
        }

        let mut codes = HashSet::new();
        for identity in &mut identities {
            identity.code = identity.code.trim().to_string();
            identity.name = identity.name.trim().to_string();
            if identity.code.is_empty() || identity.name.is_empty() {
                return Err(master_error(
                    "security_identity_empty",
                    "security code or name is empty",
                ));
            }
            if !codes.insert(identity.code.clone()) {
                return Err(master_error(
                    "duplicate_security_code",
                    format!("duplicate security code: {}", identity.code),
                ));
            }
        }
        identities.sort_by(|left, right| left.code.cmp(&right.code));

        Ok(Self {
            identities,
            batch_id,
            observed_at,
        })
    }

    pub fn identities(&self) -> &[SecurityIdentity] {
        &self.identities
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DirectMentionKind {
    ExactSecurityCode,
    ExactSecurityName,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectMentionEvidence {
    pub security: SecurityIdentity,
    pub matched_by: DirectMentionKind,
    pub master_batch_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardMembershipEvidence {
    pub security: SecurityIdentity,
    pub board_name: String,
    pub master_batch_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiProposedEvidence {
    pub proposed_code: Option<String>,
    pub rationale_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RelationEvidence {
    DirectMention(DirectMentionEvidence),
    BoardMembership(BoardMembershipEvidence),
    AiProposed(AiProposedEvidence),
}

impl RelationEvidence {
    pub fn formal_candidate_allowed(&self) -> bool {
        matches!(self, Self::DirectMention(_))
    }
}

fn master_error(reason_code: &'static str, message: impl Into<String>) -> SecurityMasterError {
    SecurityMasterError {
        reason_code,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_identity_changes_when_event_identity_changes() {
        let market_date = NaiveDate::from_ymd_opt(2026, 7, 23).expect("valid test date");
        let first = CandidateIdentity::new(
            "event-a",
            "chain-semiconductor",
            "TEST_CODE_000001",
            "direct-v1",
            "feature-v1",
            market_date,
        );
        let second = CandidateIdentity::new(
            "event-b",
            "chain-semiconductor",
            "TEST_CODE_000001",
            "direct-v1",
            "feature-v1",
            market_date,
        );

        assert_ne!(first, second);
        assert!(first.as_str().starts_with("selection_candidate_v1_"));
    }
}
