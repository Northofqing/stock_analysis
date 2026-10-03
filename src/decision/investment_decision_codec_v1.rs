//! Closed historical values only. No input DTO here is a write or live-fact capability.
use super::candidate_scope_observation_v1::StoredCandidateScopeObservation;
use super::pushed_candidate_scope_v1::{self as source, CalendarObservation};
use crate::database::candidate_scope_observation_schema_v1::POLICY;
use crate::database::investment_decision_schema_v1::{MAX_RECORD_BYTES, STRATEGY};
use crate::risk::veto_execution_report_v1::{builtin_rule_catalog_v1, VetoConfigurationSnapshotV1};
use chrono::{FixedOffset, SecondsFormat};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Write;

pub(super) const DOMAIN: &[u8] = b"stock_analysis.investment_decision.record.v1\0";
const ROOT_DOMAIN: &str = "stock_analysis.investment_decision.record.v1";
const SCHEMA: &str = "investment-decision-record-v1";
const MAX_META: usize = 1024 * 1024;
const MAX_ROW: usize = 128 * 1024;
const MAX_ROWS: usize = 8 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub(super) enum CodecError {
    #[error("investment record exceeds its fixed budget")]
    Bounds,
    #[error("investment record is not the closed canonical schema")]
    Invalid,
    #[error("original scope differs")]
    Scope,
    #[error("risk configuration cannot be frozen")]
    Configuration,
}
pub(super) fn digest(bytes: &[u8]) -> Vec<u8> {
    let mut h = Sha256::new();
    h.update(DOMAIN);
    h.update(bytes);
    h.finalize().to_vec()
}
pub(super) fn id(bytes: &[u8]) -> String {
    format!("investment-decision-v1:{}", hex::encode(digest(bytes)))
}
fn occurrence(slot: i64) -> String {
    let mut h = Sha256::new();
    h.update(b"stock_analysis.investment_decision.occurrence.v1\0");
    for s in [STRATEGY, POLICY] {
        h.update((s.len() as u64).to_be_bytes());
        h.update(s.as_bytes());
    }
    h.update(slot.to_be_bytes());
    h.update(1i64.to_be_bytes());
    format!(
        "investment-evaluation-occurrence-v1:{}",
        hex::encode(h.finalize())
    )
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum RequiredField {
    NativeIdentity,
    FactTime,
    Lifecycle,
    PriceBand,
    Suspension,
    IntegerPrice,
    Liquidity,
    Cost,
    Risk,
    Funding,
    ManualApproval,
    SourceProfile,
}
const FIELDS: [RequiredField; 12] = [
    RequiredField::NativeIdentity,
    RequiredField::FactTime,
    RequiredField::Lifecycle,
    RequiredField::PriceBand,
    RequiredField::Suspension,
    RequiredField::IntegerPrice,
    RequiredField::Liquidity,
    RequiredField::Cost,
    RequiredField::Risk,
    RequiredField::Funding,
    RequiredField::ManualApproval,
    RequiredField::SourceProfile,
];
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum FactState {
    Admitted,
    Missing,
    Stale,
    Conflict,
    Unqualified,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Reason {
    NativeIdentityUnavailable,
    SourceProfileNotDelivered,
    BlockedByNativeIdentity,
    IntegerPriceContractUnavailable,
    RiskPrerequisitesUnavailable,
    FundingAllocationNotDelivered,
    ManualApprovalNotDelivered,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", deny_unknown_fields)]
enum Assessment {
    Unavailable { reason: Reason },
    NotEvaluated { reason: Reason },
    AcquisitionNotInvoked { reason: Reason },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FieldAssessment {
    field: RequiredField,
    action_fact_state: FactState,
    assessment: Assessment,
}
fn matrix() -> Vec<FieldAssessment> {
    FIELDS
        .into_iter()
        .map(|field| {
            use RequiredField::*;
            let assessment = match field {
                NativeIdentity => Assessment::Unavailable {
                    reason: Reason::NativeIdentityUnavailable,
                },
                Lifecycle | PriceBand | Suspension => Assessment::AcquisitionNotInvoked {
                    reason: Reason::BlockedByNativeIdentity,
                },
                FactTime | Liquidity | Cost => Assessment::NotEvaluated {
                    reason: Reason::BlockedByNativeIdentity,
                },
                IntegerPrice => Assessment::Unavailable {
                    reason: Reason::IntegerPriceContractUnavailable,
                },
                Risk => Assessment::NotEvaluated {
                    reason: Reason::RiskPrerequisitesUnavailable,
                },
                Funding => Assessment::Unavailable {
                    reason: Reason::FundingAllocationNotDelivered,
                },
                ManualApproval => Assessment::Unavailable {
                    reason: Reason::ManualApprovalNotDelivered,
                },
                SourceProfile => Assessment::Unavailable {
                    reason: Reason::SourceProfileNotDelivered,
                },
            };
            FieldAssessment {
                field,
                action_fact_state: FactState::Unqualified,
                assessment,
            }
        })
        .collect()
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum Disposition {
    DeniedBeforeFacts,
    NoCandidates,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", deny_unknown_fields)]
enum Calendar {
    Available {
        contract: String,
        authority_sha256: String,
        open: bool,
    },
    Unavailable {
        reason: String,
    },
}
fn frozen_calendar(c: &CalendarObservation) -> Calendar {
    match c {
        CalendarObservation::Covered {
            contract,
            authority_sha256,
            open,
        } => Calendar::Available {
            contract: contract.clone(),
            authority_sha256: authority_sha256.clone(),
            open: *open,
        },
        CalendarObservation::Unavailable { .. } => Calendar::Unavailable {
            reason: "ImmutableCalendarCoverageUnavailable".into(),
        },
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RequiredInput {
    field: String,
    required: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Subcondition {
    id: String,
    enabled: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum Thresholds {
    BiasRate {
        bias_threshold: f64,
    },
    MainFlow {
        outflow_threshold: f64,
        lure_pct_threshold: f64,
        lure_outflow_threshold: f64,
    },
    FundamentalDeterioration {
        pe_upper: f64,
        profit_decline_threshold: f64,
    },
}
impl Thresholds {
    fn finite(&self) -> bool {
        match self {
            Self::BiasRate { bias_threshold } => bias_threshold.is_finite(),
            Self::MainFlow {
                outflow_threshold,
                lure_pct_threshold,
                lure_outflow_threshold,
            } => [
                outflow_threshold,
                lure_pct_threshold,
                lure_outflow_threshold,
            ]
            .iter()
            .all(|v| v.is_finite()),
            Self::FundamentalDeterioration {
                pe_upper,
                profit_decline_threshold,
            } => pe_upper.is_finite() && profit_decline_threshold.is_finite(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfiguredRule {
    id: String,
    version: u16,
    priority: u8,
    enabled: bool,
    required_inputs: Vec<RequiredInput>,
    subconditions: Vec<Subcondition>,
    thresholds: Thresholds,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Configuration {
    schema_version: u16,
    catalog_version: u16,
    config_version: u16,
    model_version: String,
    enabled: bool,
    raw_mode: String,
    effective_mode: String,
    bias_rate_enabled: bool,
    bearish_alignment_enabled: bool,
    main_flow_enabled: bool,
    fundamental_enabled: bool,
    rules: Vec<ConfiguredRule>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InputDescriptor {
    field: String,
    required_when: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SubconditionDescriptor {
    id: String,
    required_inputs: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleDescriptor {
    id: String,
    version: u16,
    priority: u8,
    required_inputs: Vec<InputDescriptor>,
    subconditions: Vec<SubconditionDescriptor>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RiskCatalog {
    catalog_version: u16,
    config_version: u16,
    model_version: String,
    descriptors: Vec<RuleDescriptor>,
    configuration: Configuration,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum RuleReason {
    GloballyDisabled,
    RuleDisabled,
    BlockedByNativeIdentity,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", deny_unknown_fields)]
enum RuleEvaluation {
    NotEvaluated { reason: RuleReason },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum InputStatus {
    NotAcquiredBlockedByNativeIdentity,
    NotRequiredByDisabledCondition,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleInput {
    field: String,
    required: bool,
    status: InputStatus,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleSubcondition {
    id: String,
    enabled: bool,
    evaluation: RuleEvaluation,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleAssessment {
    rule_id: String,
    rule_version: u16,
    priority: u8,
    enabled: bool,
    required_inputs: Vec<RuleInput>,
    subconditions: Vec<RuleSubcondition>,
    evaluation: RuleEvaluation,
}
fn rule_reason(global: bool, enabled: bool) -> RuleEvaluation {
    RuleEvaluation::NotEvaluated {
        reason: if !global {
            RuleReason::GloballyDisabled
        } else if !enabled {
            RuleReason::RuleDisabled
        } else {
            RuleReason::BlockedByNativeIdentity
        },
    }
}
fn rules(c: &Configuration) -> Vec<RuleAssessment> {
    c.rules
        .iter()
        .map(|r| RuleAssessment {
            rule_id: r.id.clone(),
            rule_version: r.version,
            priority: r.priority,
            enabled: r.enabled,
            required_inputs: r
                .required_inputs
                .iter()
                .map(|i| RuleInput {
                    field: i.field.clone(),
                    required: i.required,
                    status: if c.enabled && i.required {
                        InputStatus::NotAcquiredBlockedByNativeIdentity
                    } else {
                        InputStatus::NotRequiredByDisabledCondition
                    },
                })
                .collect(),
            subconditions: r
                .subconditions
                .iter()
                .map(|i| RuleSubcondition {
                    id: i.id.clone(),
                    enabled: i.enabled,
                    evaluation: rule_reason(c.enabled, i.enabled),
                })
                .collect(),
            evaluation: rule_reason(c.enabled, r.enabled),
        })
        .collect()
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Candidate {
    ordinal: usize,
    source_row_id: i64,
    raw_row_sha256: String,
    required_fields: Vec<FieldAssessment>,
    rules: Vec<RuleAssessment>,
    disposition: Disposition,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Scope {
    observation_row_id: i64,
    observation_scope_id: String,
    observation_occurrence_id: String,
    observation_revision: u16,
    scope_sha256: String,
    canonical_utf8: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Record {
    domain: String,
    schema: String,
    schema_version: u16,
    strategy_id: String,
    strategy_version: u16,
    model_id: String,
    model_version: u16,
    evaluation_occurrence_id: String,
    scope_policy_id: String,
    slot_start_unix_ms: i64,
    evaluation_revision: u16,
    first_cutoff_utc: String,
    shanghai_business_date: String,
    scope: Scope,
    calendar: Calendar,
    risk_catalog: RiskCatalog,
    required_fields: Vec<RequiredField>,
    candidate_evaluations: Vec<Candidate>,
    disposition: Disposition,
}
impl Record {
    pub(super) fn observation_row_id(&self) -> i64 {
        self.scope.observation_row_id
    }
    pub(super) fn disposition(&self) -> &Disposition {
        &self.disposition
    }
    pub(super) fn candidate_count(&self) -> usize {
        self.candidate_evaluations.len()
    }
}
struct BoundedWriter {
    bytes: Vec<u8>,
    limit: usize,
}
impl Write for BoundedWriter {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        if b.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(std::io::Error::other("investment codec bound"));
        }
        self.bytes.extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn encode<T: Serialize>(v: &T, limit: usize) -> Result<Vec<u8>, CodecError> {
    let mut w = BoundedWriter {
        bytes: Vec::new(),
        limit,
    };
    serde_json::to_writer(&mut w, v).map_err(|_| CodecError::Bounds)?;
    Ok(w.bytes)
}
fn value_copy<T: Serialize, D: for<'a> Deserialize<'a>>(v: &T) -> Result<D, CodecError> {
    let bytes = encode(v, MAX_META)?;
    preflight(&bytes)?;
    serde_json::from_slice(&bytes).map_err(|_| CodecError::Invalid)
}
fn catalog() -> Result<Vec<RuleDescriptor>, CodecError> {
    value_copy(builtin_rule_catalog_v1())
}
fn validate_risk(r: &RiskCatalog) -> Result<(), CodecError> {
    let c = &r.configuration;
    if r.catalog_version != 1
        || r.config_version != 1
        || r.model_version != "veto-builtin-model-v1"
        || c.schema_version != 1
        || c.catalog_version != 1
        || c.config_version != 1
        || c.model_version != r.model_version
        || c.raw_mode.len() > 64
        || c.effective_mode
            != if c.raw_mode == "live" {
                "live"
            } else {
                "dry_run"
            }
        || r.descriptors != catalog()?
        || c.rules.len() != 3
    {
        return Err(CodecError::Invalid);
    }
    // Values only: enforce the frozen V1 descriptor relationships. Never compare
    // original finite thresholds with today's Default or execute a risk rule.
    for (rule, d) in c.rules.iter().zip(&r.descriptors) {
        if rule.id != d.id
            || rule.version != d.version
            || rule.priority != d.priority
            || !rule.thresholds.finite()
            || rule.required_inputs.len() != d.required_inputs.len()
            || rule.subconditions.len() != d.subconditions.len()
        {
            return Err(CodecError::Invalid);
        }
        let expected_enabled = match rule.id.as_str() {
            "BiasRateRule" => c.bias_rate_enabled || c.bearish_alignment_enabled,
            "MainFlowRule" => c.main_flow_enabled,
            "FundamentalDeteriorationRule" => c.fundamental_enabled,
            _ => return Err(CodecError::Invalid),
        };
        if rule.enabled != expected_enabled
            || !matches!(
                (&*rule.id, &rule.thresholds),
                ("BiasRateRule", Thresholds::BiasRate { .. })
                    | ("MainFlowRule", Thresholds::MainFlow { .. })
                    | (
                        "FundamentalDeteriorationRule",
                        Thresholds::FundamentalDeterioration { .. }
                    )
            )
        {
            return Err(CodecError::Invalid);
        }
        for (i, d) in rule.required_inputs.iter().zip(&d.required_inputs) {
            let required = rule.enabled
                && match d.required_when.as_str() {
                    "rule_enabled" => true,
                    "bias_rate_enabled" => c.bias_rate_enabled,
                    "bearish_alignment_enabled" => c.bearish_alignment_enabled,
                    _ => return Err(CodecError::Invalid),
                };
            if i.field != d.field || i.required != required {
                return Err(CodecError::Invalid);
            }
        }
        for (i, d) in rule.subconditions.iter().zip(&d.subconditions) {
            let enabled = rule.enabled
                && match d.id.as_str() {
                    "high_bias" => c.bias_rate_enabled,
                    "bearish_alignment" => c.bearish_alignment_enabled,
                    _ => true,
                };
            if i.id != d.id || i.enabled != enabled {
                return Err(CodecError::Invalid);
            }
        }
    }
    Ok(())
}
fn raw_hash(row: &source::FrozenPushRow) -> Result<String, CodecError> {
    let mut h = Sha256::new();
    h.update(b"stock_analysis.investment_decision.raw_candidate.v1\0");
    h.update(encode(row, 1024 * 1024)?);
    Ok(hex::encode(h.finalize()))
}
fn expected_scope(o: &StoredCandidateScopeObservation) -> Result<Scope, CodecError> {
    Ok(Scope {
        observation_row_id: o.row_id(),
        observation_scope_id: o.scope_id().into(),
        observation_occurrence_id: o.occurrence_id().into(),
        observation_revision: 1,
        scope_sha256: o
            .scope_id()
            .strip_prefix("candidate-observation-scope-v1:")
            .unwrap_or("")
            .into(),
        canonical_utf8: std::str::from_utf8(o.canonical_bytes())
            .map_err(|_| CodecError::Scope)?
            .to_owned(),
    })
}
fn date(o: &StoredCandidateScopeObservation) -> String {
    o.cutoff()
        .with_timezone(&FixedOffset::east_opt(8 * 3600).expect("fixed offset"))
        .format("%Y-%m-%d")
        .to_string()
}
pub(super) fn build(
    o: &StoredCandidateScopeObservation,
    config: &crate::config::LiveVetoConfig,
) -> Result<Vec<u8>, CodecError> {
    let snapshot = VetoConfigurationSnapshotV1::from_live_config(config)
        .map_err(|_| CodecError::Configuration)?;
    let configuration: Configuration = value_copy(&snapshot)?;
    let risk_catalog = RiskCatalog {
        catalog_version: 1,
        config_version: 1,
        model_version: "veto-builtin-model-v1".into(),
        descriptors: catalog()?,
        configuration,
    };
    validate_risk(&risk_catalog)?;
    let source = source::decode_historical_scope(o.canonical_bytes(), o.cutoff())
        .map_err(|_| CodecError::Scope)?;
    let candidate_evaluations = source
        .rows
        .iter()
        .enumerate()
        .map(|(ordinal, row)| {
            Ok(Candidate {
                ordinal,
                source_row_id: row.id,
                raw_row_sha256: raw_hash(row)?,
                required_fields: matrix(),
                rules: rules(&risk_catalog.configuration),
                disposition: Disposition::DeniedBeforeFacts,
            })
        })
        .collect::<Result<Vec<_>, CodecError>>()?;
    let disposition = if candidate_evaluations.is_empty() {
        Disposition::NoCandidates
    } else {
        Disposition::DeniedBeforeFacts
    };
    let r = Record {
        domain: ROOT_DOMAIN.into(),
        schema: SCHEMA.into(),
        schema_version: 1,
        strategy_id: STRATEGY.into(),
        strategy_version: 1,
        model_id: "native-identity-required-negative-evaluation-v1".into(),
        model_version: 1,
        evaluation_occurrence_id: occurrence(o.slot_start_unix_ms()),
        scope_policy_id: POLICY.into(),
        slot_start_unix_ms: o.slot_start_unix_ms(),
        evaluation_revision: 1,
        first_cutoff_utc: o.cutoff().to_rfc3339_opts(SecondsFormat::Nanos, true),
        shanghai_business_date: date(o),
        scope: expected_scope(o)?,
        calendar: frozen_calendar(&source.calendar),
        risk_catalog,
        required_fields: FIELDS.to_vec(),
        candidate_evaluations,
        disposition,
    };
    validate(&r, o)?;
    let bytes = encode(&r, MAX_RECORD_BYTES)?;
    preflight(&bytes)?;
    Ok(bytes)
}
fn validate(r: &Record, o: &StoredCandidateScopeObservation) -> Result<(), CodecError> {
    if r.domain != ROOT_DOMAIN
        || r.schema != SCHEMA
        || r.schema_version != 1
        || r.strategy_id != STRATEGY
        || r.strategy_version != 1
        || r.model_id != "native-identity-required-negative-evaluation-v1"
        || r.model_version != 1
        || r.evaluation_occurrence_id != occurrence(o.slot_start_unix_ms())
        || r.scope_policy_id != POLICY
        || r.slot_start_unix_ms != o.slot_start_unix_ms()
        || r.evaluation_revision != 1
        || r.first_cutoff_utc != o.cutoff().to_rfc3339_opts(SecondsFormat::Nanos, true)
        || r.shanghai_business_date != date(o)
        || r.scope != expected_scope(o)?
        || r.required_fields != FIELDS
    {
        return Err(CodecError::Invalid);
    }
    validate_risk(&r.risk_catalog)?;
    let source = source::decode_historical_scope(o.canonical_bytes(), o.cutoff())
        .map_err(|_| CodecError::Scope)?;
    if r.calendar != frozen_calendar(&source.calendar)
        || r.candidate_evaluations.len() != source.rows.len()
        || source.rows.len() > 50
        || r.disposition
            != if source.rows.is_empty() {
                Disposition::NoCandidates
            } else {
                Disposition::DeniedBeforeFacts
            }
    {
        return Err(CodecError::Invalid);
    }
    let mut total = 0usize;
    let expected_rules = rules(&r.risk_catalog.configuration);
    for (ordinal, (c, row)) in r.candidate_evaluations.iter().zip(&source.rows).enumerate() {
        if c.ordinal != ordinal
            || c.source_row_id != row.id
            || c.raw_row_sha256 != raw_hash(row)?
            || c.required_fields != matrix()
            || c.rules != expected_rules
            || c.rules.len() > 64
            || c.disposition != Disposition::DeniedBeforeFacts
        {
            return Err(CodecError::Invalid);
        }
        total = total
            .checked_add(encode(c, MAX_ROW)?.len())
            .ok_or(CodecError::Bounds)?;
        if total > MAX_ROWS {
            return Err(CodecError::Bounds);
        }
    }
    Ok(())
}
pub(super) fn decode(
    bytes: &[u8],
    o: &StoredCandidateScopeObservation,
) -> Result<Record, CodecError> {
    if bytes.is_empty() || bytes.len() > MAX_RECORD_BYTES {
        return Err(CodecError::Bounds);
    }
    preflight(bytes)?;
    let r: Record = serde_json::from_slice(bytes).map_err(|_| CodecError::Invalid)?;
    validate(&r, o)?;
    if encode(&r, MAX_RECORD_BYTES)? != bytes {
        return Err(CodecError::Invalid);
    }
    Ok(r)
}

// Streaming limits precede owned Deserialize. Borrowed unescaped map keys have
// a fixed whitelist; duplicates are refused while traversing, before any Vec.
#[derive(Default)]
struct DecodeBudget {
    metadata: usize,
    row: usize,
    rows: usize,
}
impl DecodeBudget {
    fn charge<E: serde::de::Error>(&mut self, amount: usize, in_row: bool) -> Result<(), E> {
        self.metadata = self
            .metadata
            .checked_add(amount)
            .ok_or_else(|| E::custom("metadata overflow"))?;
        if self.metadata > MAX_META {
            return Err(E::custom("metadata bound"));
        }
        if in_row {
            self.row = self
                .row
                .checked_add(amount)
                .ok_or_else(|| E::custom("row overflow"))?;
            if self.row > MAX_ROW {
                return Err(E::custom("row bound"));
            }
        }
        Ok(())
    }
}
struct Seed<'a> {
    budget: &'a mut DecodeBudget,
    depth: u8,
    scope_string: bool,
    max_items: usize,
    candidate_list: bool,
    in_row: bool,
}
impl<'de> serde::de::DeserializeSeed<'de> for Seed<'_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        if self.depth > 16 {
            return Err(serde::de::Error::custom("depth"));
        }
        d.deserialize_any(self)
    }
}
const KEYS: &[&str] = &[
    "domain",
    "schema",
    "schema_version",
    "strategy_id",
    "strategy_version",
    "model_id",
    "model_version",
    "evaluation_occurrence_id",
    "scope_policy_id",
    "slot_start_unix_ms",
    "evaluation_revision",
    "first_cutoff_utc",
    "shanghai_business_date",
    "scope",
    "calendar",
    "risk_catalog",
    "required_fields",
    "candidate_evaluations",
    "disposition",
    "observation_row_id",
    "observation_scope_id",
    "observation_occurrence_id",
    "observation_revision",
    "scope_sha256",
    "canonical_utf8",
    "state",
    "contract",
    "authority_sha256",
    "open",
    "reason",
    "catalog_version",
    "config_version",
    "descriptors",
    "configuration",
    "enabled",
    "raw_mode",
    "effective_mode",
    "bias_rate_enabled",
    "bearish_alignment_enabled",
    "main_flow_enabled",
    "fundamental_enabled",
    "rules",
    "id",
    "version",
    "priority",
    "required_inputs",
    "subconditions",
    "thresholds",
    "field",
    "required",
    "required_when",
    "BiasRate",
    "bias_threshold",
    "MainFlow",
    "outflow_threshold",
    "lure_pct_threshold",
    "lure_outflow_threshold",
    "FundamentalDeterioration",
    "pe_upper",
    "profit_decline_threshold",
    "ordinal",
    "source_row_id",
    "raw_row_sha256",
    "action_fact_state",
    "assessment",
    "rule_id",
    "rule_version",
    "status",
    "evaluation",
];
impl<'de> serde::de::Visitor<'de> for Seed<'_> {
    type Value = ();
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("bounded investment canonical")
    }
    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<(), E> {
        let limit = if self.scope_string {
            8 * 1024 * 1024
        } else {
            16 * 1024
        };
        if v.len() > limit {
            return Err(E::custom("scalar"));
        }
        if !self.scope_string {
            // Sixfold JSON escaping bound reserves row output before owned strings.
            self.budget.charge::<E>(
                v.len()
                    .checked_mul(6)
                    .and_then(|n| n.checked_add(16))
                    .ok_or_else(|| E::custom("scalar overflow"))?,
                self.in_row,
            )?;
        }
        Ok(())
    }
    fn visit_borrowed_str<E: serde::de::Error>(self, v: &'de str) -> Result<(), E> {
        self.visit_str(v)
    }
    fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<(), E> {
        Ok(())
    }
    fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<(), E> {
        Ok(())
    }
    fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<(), E> {
        Ok(())
    }
    fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<(), E> {
        if !v.is_finite() {
            Err(E::custom("nonfinite"))
        } else {
            Ok(())
        }
    }
    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        let mut n = 0;
        loop {
            if self.candidate_list {
                self.budget.row = 0;
            }
            let exists = seq
                .next_element_seed(Seed {
                    budget: self.budget,
                    depth: self.depth + 1,
                    scope_string: false,
                    max_items: 64,
                    candidate_list: false,
                    in_row: self.in_row || self.candidate_list,
                })?
                .is_some();
            if !exists {
                break;
            }
            n += 1;
            if n > self.max_items {
                return Err(serde::de::Error::custom("sequence bound"));
            }
            if self.candidate_list {
                self.budget.rows = self
                    .budget
                    .rows
                    .checked_add(self.budget.row)
                    .ok_or_else(|| <A::Error as serde::de::Error>::custom("rows overflow"))?;
                if self.budget.rows > MAX_ROWS {
                    return Err(serde::de::Error::custom("rows bound"));
                }
            }
        }
        Ok(())
    }
    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        let mut seen = 0u128;
        let mut n = 0;
        while let Some(k) = map.next_key::<&str>()? {
            let i = KEYS
                .iter()
                .position(|x| *x == k)
                .ok_or_else(|| <A::Error as serde::de::Error>::custom("unknown key"))?;
            if seen & (1u128 << i) != 0 {
                return Err(serde::de::Error::custom("duplicate key"));
            }
            seen |= 1u128 << i;
            n += 1;
            if n > 32 {
                return Err(serde::de::Error::custom("object"));
            }
            self.budget.charge::<A::Error>(k.len() + 32, self.in_row)?;
            map.next_value_seed(Seed {
                budget: self.budget,
                depth: self.depth + 1,
                scope_string: k == "canonical_utf8",
                max_items: if k == "candidate_evaluations" { 50 } else { 64 },
                candidate_list: k == "candidate_evaluations",
                in_row: self.in_row,
            })?;
        }
        Ok(())
    }
}
fn preflight(bytes: &[u8]) -> Result<(), CodecError> {
    use serde::de::DeserializeSeed;
    let mut d = serde_json::Deserializer::from_slice(bytes);
    let mut budget = DecodeBudget::default();
    Seed {
        budget: &mut budget,
        depth: 0,
        scope_string: false,
        max_items: 64,
        candidate_list: false,
        in_row: false,
    }
    .deserialize(&mut d)
    .map_err(|_| CodecError::Bounds)?;
    d.end().map_err(|_| CodecError::Invalid)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn investment_record_identity_domain_and_occurrence_literal_golden() {
        assert_eq!(id(b"{}"), "investment-decision-v1:19e1e8ddbafd11f58a804408fd16d0f74ae1dac09ffdccd7b4c0ebd9bb5a47ac");
        assert_eq!(occurrence(0), "investment-evaluation-occurrence-v1:92384be62a6211cdec1bf4891553df985b73a3b0af38ff20346f667e1a498d61");
        assert_ne!(occurrence(0), occurrence(30_000));
        assert_ne!(digest(b"{}"), Sha256::digest(b"{}").to_vec());
    }
    #[test]
    fn investment_record_streaming_bounds_and_closed_keys_before_owned_decode() {
        assert!(preflight(br#"{"schema":"a","schema":"b"}"#).is_err());
        assert!(preflight(br#"{"unknown":true}"#).is_err());
        assert!(
            preflight(format!("{{\"model_id\":\"{}\"}}", "x".repeat(16_385)).as_bytes()).is_err()
        );
        assert!(preflight(
            format!(
                "{{\"candidate_evaluations\":[{}]}}",
                vec!["{}"; 51].join(",")
            )
            .as_bytes()
        )
        .is_err());
        assert!(preflight(
            format!(
                "{{\"canonical_utf8\":\"{}\"}}",
                "x".repeat(8 * 1024 * 1024 + 1)
            )
            .as_bytes()
        )
        .is_err());
        let mut writer = BoundedWriter {
            bytes: Vec::new(),
            limit: 3,
        };
        assert_eq!(writer.write(b"abc").unwrap(), 3);
        assert!(writer.write(b"d").is_err());
        assert_eq!(writer.bytes, b"abc");
    }
}
