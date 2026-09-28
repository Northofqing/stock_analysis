//! Immutable, per-candidate action-fact assessments and investment identity.
//!
//! This module only checks that a supplied assessment is complete and internally
//! fail-closed. It does not qualify a source, verify evidence or authorize an
//! order. A source adapter must establish those properties before asserting
//! `FactState::Admitted`; no production action path consumes this type yet.

use crate::market_domain::{AssetClass, Exchange, InstrumentId};
use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

const SCHEMA: &str = "INVESTMENT_DECISION_V1";
const ID_PREFIX: &str = "investment-decision-v1:";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FactState {
    Admitted,
    Missing,
    Stale,
    Conflict,
    Unqualified,
}

impl FactState {
    /// Stable state code; detailed machine reason codes are retained separately.
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::Admitted => "fact_admitted",
            Self::Missing => "fact_missing",
            Self::Stale => "fact_stale",
            Self::Conflict => "fact_conflict",
            Self::Unqualified => "fact_unqualified",
        }
    }
}

/// Content-bound reference supplied by a future source-admission adapter.
/// A hash and version here are identifiers, not proof of source qualification.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct FactReference {
    pub dataset_version: String,
    pub evidence_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct FactAssessment {
    pub requirement: String,
    pub state: FactState,
    pub evidence: Vec<FactReference>,
    /// Stable machine codes; conflicting and unqualified causes can coexist.
    pub reason_codes: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CandidateDisposition {
    Pass,
    Reject { reason_codes: Vec<String> },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CandidateAssessment {
    pub instrument: InstrumentId,
    pub facts: Vec<FactAssessment>,
    pub disposition: CandidateDisposition,
}

/// Separate universe and assessments make missing candidate dispositions
/// detectable. Required batch-wide facts must be represented for each affected
/// candidate in this first contract version.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionFactSnapshotInput {
    pub strategy_version: String,
    pub model_version: String,
    pub config_version: String,
    pub evaluation_key: String,
    pub business_date: NaiveDate,
    pub as_of: DateTime<Utc>,
    pub calendar_version: String,
    pub universe: Vec<InstrumentId>,
    pub required_facts: Vec<String>,
    pub candidates: Vec<CandidateAssessment>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct InvestmentDecisionId(String);

impl InvestmentDecisionId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A validated, immutable assessment with one versioned canonical preimage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionFactSnapshot {
    id: InvestmentDecisionId,
    input: ActionFactSnapshotInput,
    canonical: Vec<u8>,
}

impl ActionFactSnapshot {
    pub fn freeze(mut input: ActionFactSnapshotInput) -> Result<Self, SnapshotError> {
        for (name, value) in [
            ("strategy_version", &input.strategy_version),
            ("model_version", &input.model_version),
            ("config_version", &input.config_version),
            ("evaluation_key", &input.evaluation_key),
            ("calendar_version", &input.calendar_version),
        ] {
            validate_token(value).map_err(|_| SnapshotError::Invalid(name))?;
        }

        if input.universe.is_empty() {
            return Err(SnapshotError::Missing("universe"));
        }
        input
            .universe
            .sort_by(|a, b| instrument_key(a).cmp(&instrument_key(b)));
        if input
            .universe
            .windows(2)
            .any(|pair| instrument_key(&pair[0]) == instrument_key(&pair[1]))
        {
            return Err(SnapshotError::Duplicate("universe instrument"));
        }

        if input.required_facts.is_empty() {
            return Err(SnapshotError::Missing("required_facts"));
        }
        for requirement in &input.required_facts {
            validate_token(requirement).map_err(|_| SnapshotError::Invalid("required fact"))?;
        }
        input.required_facts.sort();
        if input
            .required_facts
            .windows(2)
            .any(|pair| pair[0] == pair[1])
        {
            return Err(SnapshotError::Duplicate("required fact"));
        }

        input
            .candidates
            .sort_by(|a, b| instrument_key(&a.instrument).cmp(&instrument_key(&b.instrument)));
        if input
            .candidates
            .windows(2)
            .any(|pair| instrument_key(&pair[0].instrument) == instrument_key(&pair[1].instrument))
        {
            return Err(SnapshotError::Duplicate("candidate disposition"));
        }
        if input.candidates.len() != input.universe.len() {
            return Err(SnapshotError::Missing("candidate disposition"));
        }
        for (instrument, candidate) in input.universe.iter().zip(&mut input.candidates) {
            if instrument_key(instrument) != instrument_key(&candidate.instrument) {
                return Err(SnapshotError::Invalid("candidate universe coverage"));
            }
            candidate
                .facts
                .sort_by(|a, b| a.requirement.cmp(&b.requirement));
            if candidate.facts.len() != input.required_facts.len() {
                return Err(SnapshotError::Missing("candidate required fact"));
            }
            for (required, fact) in input.required_facts.iter().zip(&mut candidate.facts) {
                if &fact.requirement != required {
                    return Err(SnapshotError::Invalid("candidate required fact coverage"));
                }
                validate_fact(fact)?;
            }
            match &mut candidate.disposition {
                CandidateDisposition::Pass
                    if candidate
                        .facts
                        .iter()
                        .any(|fact| fact.state != FactState::Admitted) =>
                {
                    return Err(SnapshotError::NonAdmittedPass);
                }
                CandidateDisposition::Pass => {}
                CandidateDisposition::Reject { reason_codes } => {
                    normalize_reasons(reason_codes)?;
                    if reason_codes.is_empty() {
                        return Err(SnapshotError::Missing("rejection reason"));
                    }
                }
            }
        }

        let canonical = serde_json::to_vec(&CanonicalPreimage {
            schema: SCHEMA,
            strategy_version: &input.strategy_version,
            model_version: &input.model_version,
            config_version: &input.config_version,
            evaluation_key: &input.evaluation_key,
            business_date: input.business_date.format("%Y-%m-%d").to_string(),
            as_of_utc: input.as_of.to_rfc3339_opts(SecondsFormat::Nanos, true),
            calendar_version: &input.calendar_version,
            universe: &input.universe,
            required_facts: &input.required_facts,
            candidates: &input.candidates,
        })
        .map_err(|error| SnapshotError::Encoding(error.to_string()))?;
        let id = InvestmentDecisionId(format!(
            "{ID_PREFIX}{}",
            hex::encode(Sha256::digest(&canonical))
        ));
        Ok(Self {
            id,
            input,
            canonical,
        })
    }

    pub fn decision_id(&self) -> &InvestmentDecisionId {
        &self.id
    }

    pub fn frozen_input(&self) -> &ActionFactSnapshotInput {
        &self.input
    }

    pub fn canonical_bytes(&self) -> &[u8] {
        &self.canonical
    }
}

#[derive(Debug, Eq, Error, PartialEq)]
pub enum SnapshotError {
    #[error("invalid {0}")]
    Invalid(&'static str),
    #[error("missing {0}")]
    Missing(&'static str),
    #[error("duplicate {0}")]
    Duplicate(&'static str),
    #[error("a candidate with a non-admitted required fact cannot pass")]
    NonAdmittedPass,
    #[error("canonical encoding failed: {0}")]
    Encoding(String),
}

#[derive(Serialize)]
struct CanonicalPreimage<'a> {
    schema: &'static str,
    strategy_version: &'a str,
    model_version: &'a str,
    config_version: &'a str,
    evaluation_key: &'a str,
    business_date: String,
    as_of_utc: String,
    calendar_version: &'a str,
    universe: &'a [InstrumentId],
    required_facts: &'a [String],
    candidates: &'a [CandidateAssessment],
}

fn validate_token(value: &str) -> Result<(), ()> {
    if value.is_empty() || value.trim() != value || value.chars().any(char::is_control) {
        return Err(());
    }
    Ok(())
}

fn normalize_reasons(reasons: &mut Vec<String>) -> Result<(), SnapshotError> {
    if reasons.iter().any(|reason| {
        reason.is_empty()
            || !reason
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    }) {
        return Err(SnapshotError::Invalid("reason code"));
    }
    reasons.sort();
    if reasons.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(SnapshotError::Duplicate("reason code"));
    }
    Ok(())
}

fn validate_fact(fact: &mut FactAssessment) -> Result<(), SnapshotError> {
    normalize_reasons(&mut fact.reason_codes)?;
    for reference in &fact.evidence {
        validate_token(&reference.dataset_version)
            .map_err(|_| SnapshotError::Invalid("dataset version"))?;
        if reference.evidence_sha256.len() != 64
            || !reference
                .evidence_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(SnapshotError::Invalid("evidence sha256"));
        }
    }
    fact.evidence.sort_by(|a, b| {
        (&a.dataset_version, &a.evidence_sha256).cmp(&(&b.dataset_version, &b.evidence_sha256))
    });
    if fact.evidence.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(SnapshotError::Duplicate("fact evidence"));
    }
    match fact.state {
        FactState::Admitted if fact.evidence.len() != 1 || !fact.reason_codes.is_empty() => {
            Err(SnapshotError::Invalid("admitted fact evidence"))
        }
        FactState::Missing if !fact.evidence.is_empty() || fact.reason_codes.is_empty() => {
            Err(SnapshotError::Invalid("missing fact assessment"))
        }
        FactState::Stale | FactState::Conflict
            if fact.evidence.is_empty() || fact.reason_codes.is_empty() =>
        {
            Err(SnapshotError::Invalid("non-admitted fact assessment"))
        }
        FactState::Unqualified if fact.reason_codes.is_empty() => {
            Err(SnapshotError::Invalid("unqualified fact assessment"))
        }
        _ => Ok(()),
    }
}

fn instrument_key(instrument: &InstrumentId) -> (u8, &str, u8) {
    let exchange = match instrument.exchange() {
        Exchange::Shanghai => 0,
        Exchange::Shenzhen => 1,
        Exchange::Beijing => 2,
    };
    let asset_class = match instrument.asset_class() {
        AssetClass::Equity => 0,
        AssetClass::Index => 1,
        AssetClass::Fund => 2,
        AssetClass::Bond => 3,
        AssetClass::Option => 4,
    };
    (exchange, instrument.code(), asset_class)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn instrument(code: &str) -> InstrumentId {
        InstrumentId::new(Exchange::Shanghai, code, AssetClass::Equity).unwrap()
    }

    fn admitted(requirement: &str, version: &str) -> FactAssessment {
        FactAssessment {
            requirement: requirement.to_owned(),
            state: FactState::Admitted,
            evidence: vec![FactReference {
                dataset_version: version.to_owned(),
                evidence_sha256: "a".repeat(64),
            }],
            reason_codes: vec![],
        }
    }

    fn input() -> ActionFactSnapshotInput {
        let first = instrument("600001");
        let second = instrument("600002");
        ActionFactSnapshotInput {
            strategy_version: "strategy-v1".to_owned(),
            model_version: "model-v1".to_owned(),
            config_version: "config-v1".to_owned(),
            evaluation_key: "research-run-42".to_owned(),
            business_date: NaiveDate::from_ymd_opt(2026, 9, 29).unwrap(),
            as_of: Utc.with_ymd_and_hms(2026, 9, 29, 6, 30, 0).unwrap(),
            calendar_version: "calendar-v1".to_owned(),
            universe: vec![second.clone(), first.clone()],
            required_facts: vec!["price_band".to_owned(), "lifecycle".to_owned()],
            candidates: vec![
                CandidateAssessment {
                    instrument: second,
                    facts: vec![
                        admitted("price_band", "band-v1"),
                        admitted("lifecycle", "life-v1"),
                    ],
                    disposition: CandidateDisposition::Reject {
                        reason_codes: vec!["risk_veto".to_owned(), "liquidity_low".to_owned()],
                    },
                },
                CandidateAssessment {
                    instrument: first,
                    facts: vec![
                        admitted("price_band", "band-v1"),
                        admitted("lifecycle", "life-v1"),
                    ],
                    disposition: CandidateDisposition::Pass,
                },
            ],
        }
    }

    #[test]
    fn investment_id_golden_is_order_independent_and_namespaced() {
        let snapshot = ActionFactSnapshot::freeze(input()).unwrap();
        assert_eq!(
            snapshot.decision_id().as_str(),
            "investment-decision-v1:71559f0ebb5e5a79c3ded96ae3d32c8a0441017fc280f5a3d6c7a79abd8a9316"
        );
        let mut reordered = input();
        reordered.universe.reverse();
        reordered.required_facts.reverse();
        reordered.candidates.reverse();
        for candidate in &mut reordered.candidates {
            candidate.facts.reverse();
            if let CandidateDisposition::Reject { reason_codes } = &mut candidate.disposition {
                reason_codes.reverse();
            }
        }
        let same = ActionFactSnapshot::freeze(reordered).unwrap();
        assert_eq!(same.decision_id(), snapshot.decision_id());
        assert_eq!(same.canonical_bytes(), snapshot.canonical_bytes());
        assert_eq!(snapshot.frozen_input().universe[0].code(), "600001");
    }

    #[test]
    fn fact_revision_cutoff_and_disposition_each_change_identity() {
        let original = ActionFactSnapshot::freeze(input()).unwrap();
        let mut revision = input();
        revision.candidates[0].facts[0].evidence[0].dataset_version = "band-v2".to_owned();
        assert_ne!(
            ActionFactSnapshot::freeze(revision).unwrap().decision_id(),
            original.decision_id()
        );
        let mut cutoff = input();
        cutoff.as_of += Duration::seconds(1);
        assert_ne!(
            ActionFactSnapshot::freeze(cutoff).unwrap().decision_id(),
            original.decision_id()
        );
        let mut disposition = input();
        disposition.candidates[0].disposition = CandidateDisposition::Pass;
        assert_ne!(
            ActionFactSnapshot::freeze(disposition)
                .unwrap()
                .decision_id(),
            original.decision_id()
        );
    }

    #[test]
    fn frozen_universe_and_fact_matrix_require_exactly_one_entry_each() {
        let mut missing_candidate = input();
        missing_candidate.candidates.pop();
        assert_eq!(
            ActionFactSnapshot::freeze(missing_candidate),
            Err(SnapshotError::Missing("candidate disposition"))
        );
        let mut duplicate_candidate = input();
        duplicate_candidate
            .candidates
            .push(duplicate_candidate.candidates[0].clone());
        assert_eq!(
            ActionFactSnapshot::freeze(duplicate_candidate),
            Err(SnapshotError::Duplicate("candidate disposition"))
        );
        let mut duplicate_universe = input();
        duplicate_universe
            .universe
            .push(duplicate_universe.universe[0].clone());
        assert_eq!(
            ActionFactSnapshot::freeze(duplicate_universe),
            Err(SnapshotError::Duplicate("universe instrument"))
        );
        let mut missing_fact = input();
        missing_fact.candidates[0].facts.pop();
        assert_eq!(
            ActionFactSnapshot::freeze(missing_fact),
            Err(SnapshotError::Missing("candidate required fact"))
        );
        let mut duplicate_fact = input();
        duplicate_fact.candidates[0].facts[0] = duplicate_fact.candidates[0].facts[1].clone();
        assert_eq!(
            ActionFactSnapshot::freeze(duplicate_fact),
            Err(SnapshotError::Invalid("candidate required fact coverage"))
        );
    }

    #[test]
    fn every_non_admitted_state_denies_pass_and_preserves_rejection_causes() {
        for state in [
            FactState::Missing,
            FactState::Stale,
            FactState::Conflict,
            FactState::Unqualified,
        ] {
            let mut assessed = input();
            let fact = &mut assessed.candidates[1].facts[0];
            fact.state = state;
            fact.reason_codes = vec![
                "source_unqualified".to_owned(),
                "identity_conflict".to_owned(),
            ];
            if matches!(state, FactState::Missing | FactState::Unqualified) {
                fact.evidence.clear();
            }
            assert_eq!(
                ActionFactSnapshot::freeze(assessed.clone()),
                Err(SnapshotError::NonAdmittedPass)
            );
            assessed.candidates[1].disposition = CandidateDisposition::Reject {
                reason_codes: vec!["fact_gate_denied".to_owned()],
            };
            let frozen = ActionFactSnapshot::freeze(assessed).unwrap();
            let frozen_fact = frozen.frozen_input().candidates[0]
                .facts
                .iter()
                .find(|fact| fact.requirement == "price_band")
                .unwrap();
            assert_eq!(frozen_fact.state, state);
            assert_eq!(
                frozen_fact.reason_codes,
                ["identity_conflict", "source_unqualified"]
            );
        }
        let mut false_admission = input();
        false_admission.candidates[0].facts[0].evidence.clear();
        assert_eq!(
            ActionFactSnapshot::freeze(false_admission),
            Err(SnapshotError::Invalid("admitted fact evidence"))
        );
        let mut false_missing = input();
        false_missing.candidates[0].facts[0].state = FactState::Missing;
        false_missing.candidates[0].facts[0].reason_codes = vec!["no_value".to_owned()];
        assert_eq!(
            ActionFactSnapshot::freeze(false_missing),
            Err(SnapshotError::Invalid("missing fact assessment"))
        );
    }
}
