//! BR-042 immutable, caller-declared model change draft.
//!
//! References and registration time are declarations, not verified runtime,
//! PIT, human approval or same-input evidence. Recovering these ordinary bytes
//! cannot advance governance, activate a strategy, or approve paper execution.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const SCHEMA: &str = "model-change-draft-v1";
const DOMAIN: &[u8] = b"stock_analysis.model-change-draft/v1\0";
const MAX_BYTES: usize = 64 * 1024;
const MAX_REGIMES: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeclaredStrategyVersionV1 {
    pub strategy_id: String,
    pub strategy_version: String,
    pub model_id: String,
    pub model_version: String,
    pub declared_git_commit: String,
    pub config_sha256: String,
}

/// Half-open UTC window [from, to). No calendar or data coverage is attested.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeclaredEvaluationWindowV1 {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
}

/// Versioned rules both books must use, rather than a claim they did use them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedComparisonContractV1 {
    pub universe_manifest_sha256: String,
    pub benchmark_policy_version: String,
    pub input_alignment_policy_version: String,
    pub data_health_policy_version: String,
    pub account_snapshot_policy_version: String,
    pub fill_model_version: String,
    pub cost_policy_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequiredRegimeSamplesV1 {
    pub regime_id: String,
    pub minimum_decisions: u32,
}

/// Explicit preregistered requirements; no policy defaults or computed returns.
/// All rates are integer basis points and capacity is integer micro-CNY.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeclaredSampleSufficiencyPolicyV1 {
    pub policy_version: String,
    pub minimum_decisions: u32,
    pub minimum_mature_t1: u32,
    pub minimum_mature_t5: u32,
    pub minimum_mature_t20: u32,
    /// Strictly sorted unique IDs; ordering cannot change content identity.
    pub required_regimes: Vec<RequiredRegimeSamplesV1>,
    pub minimum_net_benchmark_excess_bps: i32,
    pub maximum_drawdown_bps: u32,
    pub maximum_tail_loss_bps: u32,
    pub minimum_data_availability_bps: u32,
    pub maximum_independent_trials: u32,
    pub multiple_testing_policy_version: String,
    pub minimum_capacity_micro_cny: i64,
    pub maximum_participation_bps: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeclaredChangeRationaleV1 {
    pub hypothesis: String,
    pub proposed_change: String,
    pub expected_benefit: String,
    pub risk_impact: String,
    pub counterfactual_design: String,
}

/// Mutable input material becomes immutable only inside ModelChangeDraftV1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelChangeDraftRequestV1 {
    pub champion: DeclaredStrategyVersionV1,
    pub challenger: DeclaredStrategyVersionV1,
    pub rollback: DeclaredStrategyVersionV1,
    pub champion_paper_book_id: String,
    pub challenger_paper_book_id: String,
    pub declared_registered_at: DateTime<Utc>,
    pub training: DeclaredEvaluationWindowV1,
    pub validation: DeclaredEvaluationWindowV1,
    pub prospective: DeclaredEvaluationWindowV1,
    pub review_deadline: DateTime<Utc>,
    pub comparison: SharedComparisonContractV1,
    pub sample_policy: DeclaredSampleSufficiencyPolicyV1,
    pub rationale: DeclaredChangeRationaleV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum DraftAssurance {
    DeclaredReferencesOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum DraftPhase {
    Draft,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftWire {
    schema: String,
    assurance: DraftAssurance,
    phase: DraftPhase,
    request: ModelChangeDraftRequestV1,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ModelChangeDraftV1 {
    draft_id: String,
    canonical: Vec<u8>,
    wire: DraftWire,
}

/// Time classification of ordinary draft material, never governance approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum DraftReviewTimeV1 {
    FutureDeclaration,
    BeforeObservation,
    ObservationWindow,
    AwaitingReview,
    Expired,
}

impl ModelChangeDraftV1 {
    pub fn draft_id(&self) -> &str {
        &self.draft_id
    }

    pub fn canonical_bytes(&self) -> &[u8] {
        &self.canonical
    }

    pub fn request(&self) -> &ModelChangeDraftRequestV1 {
        &self.wire.request
    }

    pub fn review_time_at(
        &self,
        now: DateTime<Utc>,
    ) -> Result<DraftReviewTimeV1, ModelChangeDraftErrorV1> {
        if !valid_time(now) {
            return Err(ModelChangeDraftErrorV1::InvalidWindow);
        }
        let r = self.request();
        Ok(if now < r.declared_registered_at {
            DraftReviewTimeV1::FutureDeclaration
        } else if now >= r.review_deadline {
            DraftReviewTimeV1::Expired
        } else if now < r.prospective.from {
            DraftReviewTimeV1::BeforeObservation
        } else if now < r.prospective.to {
            DraftReviewTimeV1::ObservationWindow
        } else {
            DraftReviewTimeV1::AwaitingReview
        })
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ModelChangeDraftErrorV1 {
    #[error("draft reference is not a bounded explicit token or content digest")]
    InvalidReference,
    #[error("champion, challenger or rollback version has an ambiguous identity")]
    AmbiguousVersion,
    #[error("champion and challenger must use distinct declared paper books")]
    SharedPaperBook,
    #[error("draft windows overlap, registration is late, or review deadline is invalid")]
    InvalidWindow,
    #[error("sample sufficiency policy is incomplete, ambiguous or out of range")]
    InvalidSamplePolicy,
    #[error("change rationale is empty, contains controls or exceeds bounds")]
    InvalidRationale,
    #[error("draft bytes exceed the fixed material budget")]
    TooLarge,
    #[error("draft is not the closed canonical Draft schema")]
    InvalidCanonical,
    #[error("original draft content identity differs")]
    IdentityMismatch,
}

fn token(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 256
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-.:/".contains(&c))
}

fn digest(s: &str, len: usize) -> bool {
    s.len() == len
        && s.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        && s.bytes().any(|c| c != b'0')
}

fn valid_time(t: DateTime<Utc>) -> bool {
    (0..=253_402_300_799).contains(&t.timestamp()) && t.timestamp_subsec_nanos() < 1_000_000_000
}

fn version(v: &DeclaredStrategyVersionV1) -> bool {
    [
        &v.strategy_id,
        &v.strategy_version,
        &v.model_id,
        &v.model_version,
    ]
    .into_iter()
    .all(|s| token(s))
        && digest(&v.declared_git_commit, 40)
        && digest(&v.config_sha256, 64)
}

fn same_strategy_version(a: &DeclaredStrategyVersionV1, b: &DeclaredStrategyVersionV1) -> bool {
    a.strategy_id == b.strategy_id && a.strategy_version == b.strategy_version
}

fn validate(r: &ModelChangeDraftRequestV1) -> Result<(), ModelChangeDraftErrorV1> {
    use ModelChangeDraftErrorV1 as E;
    if ![&r.champion, &r.challenger, &r.rollback]
        .into_iter()
        .all(version)
        || !token(&r.champion_paper_book_id)
        || !token(&r.challenger_paper_book_id)
        || !digest(&r.comparison.universe_manifest_sha256, 64)
        || !digest(&r.comparison.cost_policy_sha256, 64)
        || ![
            &r.comparison.benchmark_policy_version,
            &r.comparison.input_alignment_policy_version,
            &r.comparison.data_health_policy_version,
            &r.comparison.account_snapshot_policy_version,
            &r.comparison.fill_model_version,
        ]
        .into_iter()
        .all(|s| token(s))
    {
        return Err(E::InvalidReference);
    }
    // A changed config/model/code requires a new strategy version. Rollback
    // can be the champion, but cannot refer to the proposed challenger.
    if same_strategy_version(&r.champion, &r.challenger)
        || same_strategy_version(&r.rollback, &r.challenger)
        || (same_strategy_version(&r.rollback, &r.champion) && r.rollback != r.champion)
    {
        return Err(E::AmbiguousVersion);
    }
    if r.champion_paper_book_id == r.challenger_paper_book_id {
        return Err(E::SharedPaperBook);
    }
    if ![&r.training, &r.validation, &r.prospective]
        .into_iter()
        .all(|w| valid_time(w.from) && valid_time(w.to) && w.from < w.to)
        || !valid_time(r.declared_registered_at)
        || !valid_time(r.review_deadline)
        || r.training.to > r.validation.from
        || r.validation.to > r.prospective.from
        || r.declared_registered_at > r.validation.from
        || r.review_deadline <= r.prospective.to
    {
        return Err(E::InvalidWindow);
    }
    let p = &r.sample_policy;
    if !token(&p.policy_version)
        || !token(&p.multiple_testing_policy_version)
        || p.minimum_decisions == 0
        || [
            p.minimum_mature_t1,
            p.minimum_mature_t5,
            p.minimum_mature_t20,
        ]
        .into_iter()
        .any(|n| n == 0 || n > p.minimum_decisions)
        || p.required_regimes.is_empty()
        || p.required_regimes.len() > MAX_REGIMES
        || p.required_regimes.iter().any(|v| {
            !token(&v.regime_id)
                || v.minimum_decisions == 0
                || v.minimum_decisions > p.minimum_decisions
        })
        || p.required_regimes
            .windows(2)
            .any(|w| w[0].regime_id >= w[1].regime_id)
        || p.maximum_drawdown_bps > 10_000
        || p.maximum_tail_loss_bps > 10_000
        || !(1..=10_000).contains(&p.minimum_data_availability_bps)
        || p.maximum_independent_trials == 0
        || p.minimum_capacity_micro_cny <= 0
        || !(1..=10_000).contains(&p.maximum_participation_bps)
    {
        return Err(E::InvalidSamplePolicy);
    }
    if ![
        &r.rationale.hypothesis,
        &r.rationale.proposed_change,
        &r.rationale.expected_benefit,
        &r.rationale.risk_impact,
        &r.rationale.counterfactual_design,
    ]
    .into_iter()
    .all(|s| !s.is_empty() && s.len() <= 2048 && s.trim() == s && !s.chars().any(char::is_control))
    {
        return Err(E::InvalidRationale);
    }
    Ok(())
}

fn content_id(bytes: &[u8]) -> String {
    let mut hash = Sha256::new();
    hash.update(DOMAIN);
    hash.update(bytes);
    format!(
        "model-change-draft-v1:sha256:{}",
        hex::encode(hash.finalize())
    )
}

pub fn build_model_change_draft_v1(
    request: &ModelChangeDraftRequestV1,
) -> Result<ModelChangeDraftV1, ModelChangeDraftErrorV1> {
    validate(request)?; // Check all text/vector bounds before cloning material.
    let wire = DraftWire {
        schema: SCHEMA.to_owned(),
        assurance: DraftAssurance::DeclaredReferencesOnly,
        phase: DraftPhase::Draft,
        request: request.clone(),
    };
    let canonical =
        serde_json::to_vec(&wire).map_err(|_| ModelChangeDraftErrorV1::InvalidCanonical)?;
    if canonical.len() > MAX_BYTES {
        return Err(ModelChangeDraftErrorV1::TooLarge);
    }
    Ok(ModelChangeDraftV1 {
        draft_id: content_id(&canonical),
        canonical,
        wire,
    })
}

/// Recover original draft material only. The expected ID must be kept by the
/// caller; even a matching content hash does not attest registration or review.
pub fn recover_model_change_draft_v1(
    bytes: &[u8],
    expected_draft_id: &str,
) -> Result<ModelChangeDraftV1, ModelChangeDraftErrorV1> {
    use ModelChangeDraftErrorV1 as E;
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        return Err(E::TooLarge);
    }
    let wire: DraftWire = serde_json::from_slice(bytes).map_err(|_| E::InvalidCanonical)?;
    if wire.schema != SCHEMA {
        return Err(E::InvalidCanonical);
    }
    validate(&wire.request)?;
    let canonical = serde_json::to_vec(&wire).map_err(|_| E::InvalidCanonical)?;
    if canonical != bytes {
        return Err(E::InvalidCanonical);
    }
    let draft_id = content_id(bytes);
    if draft_id != expected_draft_id {
        return Err(E::IdentityMismatch);
    }
    Ok(ModelChangeDraftV1 {
        draft_id,
        canonical,
        wire,
    })
}

#[cfg(test)]
#[path = "model_change_draft_v1_tests.rs"]
mod tests;
