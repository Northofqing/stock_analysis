//! A production-gateway-authored denial for the trading facts that have no
//! authority contract yet. This is a read-only result, never an admission,
//! `InvestmentDecisionId`, or paper-order capability.
//!
//! F0 claims supply evaluation scope and a complete universe only. Claimed
//! states, evidence digests, and proposed dispositions are deliberately ignored.

use crate::data_gateway::qualified_trading_facts::{
    QualifiedFact, QualifiedTradingFactsGateway, QualifiedTradingFactsRequest, TradingFactField,
    TradingFactUnavailableReason, QUALIFIED_TRADING_FACTS_CONTRACT_V1,
};
use crate::market_domain::InstrumentId;
use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

use super::action_fact_snapshot::{
    SnapshotError, UnverifiedActionFactSnapshot, UnverifiedActionFactSnapshotInput,
};

const SCHEMA: &str = "ACTION_FACT_DENIAL_V1";
const ID_PREFIX: &str = "action-fact-denial-v1:";
const FACT_GATE_DENIED: &str = "fact_gate_denied";

/// The fixed trading requirements checked by this denial-only gate.
pub const TRADING_FACT_REQUIREMENTS_V1: [&str; 3] = ["lifecycle", "price_regime", "suspension"];

/// A negative fact state. There is intentionally no `Admitted` variant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeniedFactState {
    Unqualified,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeniedTradingFact {
    field: TradingFactField,
    state: DeniedFactState,
    source_reason: TradingFactUnavailableReason,
}

impl DeniedTradingFact {
    pub const fn field(&self) -> TradingFactField {
        self.field
    }

    pub const fn state(&self) -> DeniedFactState {
        self.state
    }

    pub const fn source_reason(&self) -> TradingFactUnavailableReason {
        self.source_reason
    }
}

/// The only disposition this module can issue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeniedDisposition {
    FactGateDenied,
}

impl DeniedDisposition {
    pub const fn reason_code(self) -> &'static str {
        FACT_GATE_DENIED
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeniedCandidate {
    instrument: InstrumentId,
    facts: [DeniedTradingFact; 3],
    disposition: DeniedDisposition,
}

impl DeniedCandidate {
    pub fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }

    pub fn facts(&self) -> &[DeniedTradingFact; 3] {
        &self.facts
    }

    pub const fn disposition(&self) -> DeniedDisposition {
        self.disposition
    }
}

/// Namespace for a replayable denial result, distinct from investment and F0 IDs.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct DeniedEvaluationId(String);

impl DeniedEvaluationId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Complete, read-only evaluation denial. Its private constructor prevents
/// caller-written assessments from masquerading as gateway results.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeniedEvaluationReceipt {
    id: DeniedEvaluationId,
    business_date: NaiveDate,
    as_of: DateTime<Utc>,
    candidates: Vec<DeniedCandidate>,
    canonical: Vec<u8>,
}

impl DeniedEvaluationReceipt {
    pub fn id(&self) -> &DeniedEvaluationId {
        &self.id
    }

    pub const fn business_date(&self) -> NaiveDate {
        self.business_date
    }

    pub const fn as_of(&self) -> DateTime<Utc> {
        self.as_of
    }

    pub fn candidates(&self) -> &[DeniedCandidate] {
        &self.candidates
    }

    pub fn canonical_bytes(&self) -> &[u8] {
        &self.canonical
    }
}

#[derive(Debug, Error, PartialEq)]
pub enum DenialError {
    #[error(transparent)]
    InvalidClaims(#[from] SnapshotError),
    #[error("F2a requires exactly the three trading fact fields")]
    WrongRequirements,
    #[error("gateway no longer reports an evidence-free, undelivered trading contract for {instrument:?} {field:?}")]
    GatewayChanged {
        instrument: InstrumentId,
        field: TradingFactField,
    },
    #[error("denial encoding failed: {0}")]
    Encoding(String),
}

/// Evaluate a caller's complete F0 scope against the concrete production
/// gateway. No caller-supplied fact claim participates in the outcome or ID.
/// If an authority contract arrives, this denial-only version fails closed
/// until a separate as-of/versioned admission witness is implemented.
pub fn evaluate_production_trading_fact_denial(
    input: UnverifiedActionFactSnapshotInput,
) -> Result<DeniedEvaluationReceipt, DenialError> {
    let frozen = UnverifiedActionFactSnapshot::freeze(input)?;
    let scope = frozen.frozen_claims();
    if !scope
        .required_facts
        .iter()
        .map(String::as_str)
        .eq(TRADING_FACT_REQUIREMENTS_V1)
    {
        return Err(DenialError::WrongRequirements);
    }

    let gateway = QualifiedTradingFactsGateway::new();
    let mut candidates = Vec::with_capacity(scope.universe.len());
    for instrument in &scope.universe {
        let request = QualifiedTradingFactsRequest::new(instrument.clone(), scope.business_date);
        let facts = gateway.acquire(request.clone());
        if facts.request() != &request
            || facts.evidence().is_some()
            || facts.contract_version() != QUALIFIED_TRADING_FACTS_CONTRACT_V1
        {
            return Err(DenialError::GatewayChanged {
                instrument: instrument.clone(),
                field: TradingFactField::Lifecycle,
            });
        }
        candidates.push(DeniedCandidate {
            instrument: instrument.clone(),
            facts: [
                require_undelivered(facts.lifecycle(), TradingFactField::Lifecycle, instrument)?,
                require_undelivered(
                    facts.price_regime(),
                    TradingFactField::PriceRegime,
                    instrument,
                )?,
                require_undelivered(facts.suspension(), TradingFactField::Suspension, instrument)?,
            ],
            disposition: DeniedDisposition::FactGateDenied,
        });
    }

    let canonical_candidates = candidates
        .iter()
        .map(|candidate| CanonicalCandidate {
            instrument: &candidate.instrument,
            facts: candidate
                .facts
                .iter()
                .map(|fact| CanonicalFact {
                    field: field_code(fact.field),
                    state: "unqualified",
                    source_reason: fact.source_reason.reason_code(),
                })
                .collect(),
            disposition: "rejected",
            reason_codes: [FACT_GATE_DENIED],
        })
        .collect::<Vec<_>>();
    let canonical = serde_json::to_vec(&CanonicalReceipt {
        schema: SCHEMA,
        strategy_version: &scope.strategy_version,
        model_version: &scope.model_version,
        config_version: &scope.config_version,
        evaluation_key: &scope.evaluation_key,
        business_date: scope.business_date.format("%Y-%m-%d").to_string(),
        as_of_utc: scope.as_of.to_rfc3339_opts(SecondsFormat::Nanos, true),
        calendar_version: &scope.calendar_version,
        required_facts: TRADING_FACT_REQUIREMENTS_V1,
        gateway_contract_version: QUALIFIED_TRADING_FACTS_CONTRACT_V1,
        candidates: &canonical_candidates,
    })
    .map_err(|error| DenialError::Encoding(error.to_string()))?;
    let id = DeniedEvaluationId(format!(
        "{ID_PREFIX}{}",
        hex::encode(Sha256::digest(&canonical))
    ));
    Ok(DeniedEvaluationReceipt {
        id,
        business_date: scope.business_date,
        as_of: scope.as_of,
        candidates,
        canonical,
    })
}

fn require_undelivered<T>(
    fact: &QualifiedFact<T>,
    field: TradingFactField,
    instrument: &InstrumentId,
) -> Result<DeniedTradingFact, DenialError> {
    match fact {
        QualifiedFact::Unavailable(unavailable)
            if unavailable.field() == field
                && unavailable.reason() == TradingFactUnavailableReason::ContractNotDelivered =>
        {
            Ok(DeniedTradingFact {
                field,
                state: DeniedFactState::Unqualified,
                source_reason: unavailable.reason(),
            })
        }
        _ => Err(DenialError::GatewayChanged {
            instrument: instrument.clone(),
            field,
        }),
    }
}

const fn field_code(field: TradingFactField) -> &'static str {
    match field {
        TradingFactField::Lifecycle => "lifecycle",
        TradingFactField::PriceRegime => "price_regime",
        TradingFactField::Suspension => "suspension",
    }
}

#[derive(Serialize)]
struct CanonicalReceipt<'a> {
    schema: &'static str,
    strategy_version: &'a str,
    model_version: &'a str,
    config_version: &'a str,
    evaluation_key: &'a str,
    business_date: String,
    as_of_utc: String,
    calendar_version: &'a str,
    required_facts: [&'static str; 3],
    gateway_contract_version: &'static str,
    candidates: &'a [CanonicalCandidate<'a>],
}

#[derive(Serialize)]
struct CanonicalCandidate<'a> {
    instrument: &'a InstrumentId,
    facts: Vec<CanonicalFact>,
    disposition: &'static str,
    reason_codes: [&'static str; 1],
}

#[derive(Serialize)]
struct CanonicalFact {
    field: &'static str,
    state: &'static str,
    source_reason: &'static str,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision::action_fact_snapshot::{
        FactState, ProposedDisposition, UnverifiedCandidateAssessment, UnverifiedFactAssessment,
        UnverifiedFactReference,
    };
    use crate::market_domain::{AssetClass, Exchange};
    use chrono::{Duration, TimeZone};

    fn instrument(code: &str) -> InstrumentId {
        InstrumentId::new(Exchange::Shanghai, code, AssetClass::Equity).unwrap()
    }

    fn invented_admission(requirement: &str) -> UnverifiedFactAssessment {
        UnverifiedFactAssessment {
            requirement: requirement.to_owned(),
            claimed_state: FactState::Admitted,
            evidence: vec![UnverifiedFactReference {
                dataset_version: "caller-asserted-v1".to_owned(),
                evidence_sha256: "a".repeat(64),
            }],
            reason_codes: vec![],
        }
    }

    fn input() -> UnverifiedActionFactSnapshotInput {
        let first = instrument("600001");
        let second = instrument("600002");
        UnverifiedActionFactSnapshotInput {
            strategy_version: "strategy-v1".to_owned(),
            model_version: "model-v1".to_owned(),
            config_version: "config-v1".to_owned(),
            evaluation_key: "eval-42".to_owned(),
            business_date: NaiveDate::from_ymd_opt(2026, 9, 29).unwrap(),
            as_of: Utc.with_ymd_and_hms(2026, 9, 29, 6, 30, 0).unwrap(),
            calendar_version: "calendar-v1".to_owned(),
            universe: vec![second.clone(), first.clone()],
            required_facts: TRADING_FACT_REQUIREMENTS_V1.map(str::to_owned).to_vec(),
            candidates: vec![second, first]
                .into_iter()
                .map(|instrument| UnverifiedCandidateAssessment {
                    instrument,
                    facts: TRADING_FACT_REQUIREMENTS_V1
                        .iter()
                        .rev()
                        .map(|required| invented_admission(required))
                        .collect(),
                    proposed_disposition: ProposedDisposition::WouldPass,
                })
                .collect(),
        }
    }

    #[test]
    fn fabricated_f0_admissions_still_yield_a_complete_gateway_denial() {
        let receipt = evaluate_production_trading_fact_denial(input()).unwrap();
        assert!(receipt.id().as_str().starts_with(ID_PREFIX));
        assert_eq!(receipt.candidates().len(), 2);
        assert_eq!(receipt.candidates()[0].instrument().code(), "600001");
        assert_eq!(receipt.candidates()[1].instrument().code(), "600002");
        for candidate in receipt.candidates() {
            assert_eq!(candidate.disposition(), DeniedDisposition::FactGateDenied);
            assert_eq!(candidate.disposition().reason_code(), FACT_GATE_DENIED);
            assert_eq!(
                candidate
                    .facts()
                    .iter()
                    .map(DeniedTradingFact::field)
                    .collect::<Vec<_>>(),
                vec![
                    TradingFactField::Lifecycle,
                    TradingFactField::PriceRegime,
                    TradingFactField::Suspension,
                ]
            );
            for fact in candidate.facts() {
                assert_eq!(fact.state(), DeniedFactState::Unqualified);
                assert_eq!(
                    fact.source_reason(),
                    TradingFactUnavailableReason::ContractNotDelivered
                );
            }
        }
        let encoded = String::from_utf8(receipt.canonical_bytes().to_vec()).unwrap();
        assert!(!encoded.contains("caller-asserted-v1"));
        assert!(!encoded.contains(&"a".repeat(64)));
        assert!(encoded.contains("trading_fact_contract_not_delivered"));
    }

    #[test]
    fn same_evaluation_has_stable_denial_identity_independent_of_claims_and_order() {
        let original = evaluate_production_trading_fact_denial(input()).unwrap();
        let mut reordered = input();
        reordered.universe.reverse();
        reordered.required_facts.reverse();
        reordered.candidates.reverse();
        for candidate in &mut reordered.candidates {
            candidate.facts.reverse();
            candidate.facts[0].evidence[0].dataset_version = "invented-v999".to_owned();
            candidate.facts[0].evidence[0].evidence_sha256 = "b".repeat(64);
            candidate.proposed_disposition = ProposedDisposition::WouldReject {
                reason_codes: vec!["risk_veto".to_owned()],
            };
        }
        let changed_claims = evaluate_production_trading_fact_denial(reordered).unwrap();
        assert_eq!(changed_claims.id(), original.id());
        assert_eq!(changed_claims.canonical_bytes(), original.canonical_bytes());

        let mut later = input();
        later.as_of += Duration::seconds(1);
        assert_ne!(
            evaluate_production_trading_fact_denial(later).unwrap().id(),
            original.id()
        );
    }

    #[test]
    fn incomplete_or_ambiguous_universe_cannot_get_a_receipt() {
        let mut missing = input();
        missing.candidates.pop();
        assert_eq!(
            evaluate_production_trading_fact_denial(missing),
            Err(DenialError::InvalidClaims(SnapshotError::Missing(
                "candidate disposition"
            )))
        );

        let mut duplicate = input();
        duplicate.candidates.push(duplicate.candidates[0].clone());
        assert_eq!(
            evaluate_production_trading_fact_denial(duplicate),
            Err(DenialError::InvalidClaims(SnapshotError::Duplicate(
                "candidate disposition"
            )))
        );

        let mut missing_requirement = input();
        missing_requirement.required_facts.pop();
        for candidate in &mut missing_requirement.candidates {
            candidate
                .facts
                .retain(|fact| fact.requirement != "suspension");
        }
        assert_eq!(
            evaluate_production_trading_fact_denial(missing_requirement),
            Err(DenialError::WrongRequirements)
        );
    }

    #[test]
    fn a_future_available_fact_cannot_turn_the_denial_api_into_an_admission() {
        let instrument = instrument("600001");
        assert_eq!(
            require_undelivered(
                &QualifiedFact::Available(()),
                TradingFactField::Lifecycle,
                &instrument,
            ),
            Err(DenialError::GatewayChanged {
                instrument,
                field: TradingFactField::Lifecycle,
            })
        );
    }
}
