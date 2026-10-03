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
#[derive(Clone, Copy)]
enum ValueCopySchema {
    Configuration,
    BuiltinDescriptors,
}
fn value_copy<T: Serialize, D: for<'a> Deserialize<'a>>(
    v: &T,
    schema: ValueCopySchema,
) -> Result<D, CodecError> {
    let bytes = encode(v, MAX_META)?;
    // Internal Task4 ABI copies are closed fragments, not stored Records.
    // Select their exact root explicitly; never auto-detect or fall back.
    let (node, slot) = match schema {
        ValueCopySchema::Configuration => (
            Node::Object(Object::Configuration),
            std::mem::size_of::<Configuration>(),
        ),
        ValueCopySchema::BuiltinDescriptors => (
            Node::List(List::Descriptors),
            std::mem::size_of::<Vec<RuleDescriptor>>(),
        ),
    };
    preflight_root(&bytes, node, slot)?;
    serde_json::from_slice(&bytes).map_err(|_| CodecError::Invalid)
}
fn catalog() -> Result<Vec<RuleDescriptor>, CodecError> {
    value_copy(
        builtin_rule_catalog_v1(),
        ValueCopySchema::BuiltinDescriptors,
    )
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
    let configuration: Configuration = value_copy(&snapshot, ValueCopySchema::Configuration)?;
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
    if let Err(error) = preflight(bytes) {
        #[cfg(test)]
        DECODE_PROBE.with(|p| {
            let (refused, owned) = p.get();
            p.set((refused + 1, owned));
        });
        return Err(error);
    }
    #[cfg(test)]
    DECODE_PROBE.with(|p| {
        let (refused, owned) = p.get();
        p.set((refused, owned + 1));
    });
    let r: Record = serde_json::from_slice(bytes).map_err(|_| CodecError::Invalid)?;
    validate(&r, o)?;
    if encode(&r, MAX_RECORD_BYTES)? != bytes {
        return Err(CodecError::Invalid);
    }
    Ok(r)
}

// This preflight borrows bytes only: serde is not called until all paths, types,
// token lengths and cumulative reservations have been checked. Canonical order
// is mandatory here, just as it is in the final byte-for-byte roundtrip.
#[derive(Clone, Copy)]
enum Node {
    String,
    ScopeCanonical,
    Bool,
    U8,
    U16,
    Usize,
    I64,
    Float,
    Enum(&'static [&'static str]),
    Object(Object),
    List(List),
    Calendar,
    Assessment,
    RuleEvaluation,
    Thresholds,
}
#[derive(Clone, Copy)]
enum Object {
    Root,
    Scope,
    RiskCatalog,
    Configuration,
    RuleDescriptor,
    InputDescriptor,
    SubconditionDescriptor,
    ConfiguredRule,
    RequiredInput,
    ConfiguredSubcondition,
    BiasThresholds,
    FlowThresholds,
    FundamentalThresholds,
    Candidate,
    FieldAssessment,
    RuleAssessment,
    RuleInput,
    RuleSubcondition,
}
#[derive(Clone, Copy)]
enum List {
    RequiredFields,
    Candidates,
    Descriptors,
    ConfiguredRules,
    InputDescriptors,
    SubconditionDescriptors,
    InputNames,
    RequiredInputs,
    ConfiguredSubconditions,
    FieldAssessments,
    RuleAssessments,
    RuleInputs,
    RuleSubconditions,
}
const REQUIRED_FIELD_NAMES: &[&str] = &[
    "NativeIdentity",
    "FactTime",
    "Lifecycle",
    "PriceBand",
    "Suspension",
    "IntegerPrice",
    "Liquidity",
    "Cost",
    "Risk",
    "Funding",
    "ManualApproval",
    "SourceProfile",
];
const REASON_NAMES: &[&str] = &[
    "NativeIdentityUnavailable",
    "SourceProfileNotDelivered",
    "BlockedByNativeIdentity",
    "IntegerPriceContractUnavailable",
    "RiskPrerequisitesUnavailable",
    "FundingAllocationNotDelivered",
    "ManualApprovalNotDelivered",
];
const RULE_REASON_NAMES: &[&str] = &[
    "GloballyDisabled",
    "RuleDisabled",
    "BlockedByNativeIdentity",
];
const DISPOSITIONS: &[&str] = &["DeniedBeforeFacts", "NoCandidates"];
fn object_fields(kind: Object) -> &'static [(&'static str, Node)] {
    use Node::*;
    match kind {
        self::Object::Root => &[
            ("domain", String),
            ("schema", String),
            ("schema_version", U16),
            ("strategy_id", String),
            ("strategy_version", U16),
            ("model_id", String),
            ("model_version", U16),
            ("evaluation_occurrence_id", String),
            ("scope_policy_id", String),
            ("slot_start_unix_ms", I64),
            ("evaluation_revision", U16),
            ("first_cutoff_utc", String),
            ("shanghai_business_date", String),
            ("scope", Object(self::Object::Scope)),
            ("calendar", Calendar),
            ("risk_catalog", Object(self::Object::RiskCatalog)),
            ("required_fields", List(self::List::RequiredFields)),
            ("candidate_evaluations", List(self::List::Candidates)),
            ("disposition", Enum(DISPOSITIONS)),
        ],
        self::Object::Scope => &[
            ("observation_row_id", I64),
            ("observation_scope_id", String),
            ("observation_occurrence_id", String),
            ("observation_revision", U16),
            ("scope_sha256", String),
            ("canonical_utf8", ScopeCanonical),
        ],
        self::Object::RiskCatalog => &[
            ("catalog_version", U16),
            ("config_version", U16),
            ("model_version", String),
            ("descriptors", List(self::List::Descriptors)),
            ("configuration", Object(self::Object::Configuration)),
        ],
        self::Object::Configuration => &[
            ("schema_version", U16),
            ("catalog_version", U16),
            ("config_version", U16),
            ("model_version", String),
            ("enabled", Bool),
            ("raw_mode", String),
            ("effective_mode", String),
            ("bias_rate_enabled", Bool),
            ("bearish_alignment_enabled", Bool),
            ("main_flow_enabled", Bool),
            ("fundamental_enabled", Bool),
            ("rules", List(self::List::ConfiguredRules)),
        ],
        self::Object::RuleDescriptor => &[
            ("id", String),
            ("version", U16),
            ("priority", U8),
            ("required_inputs", List(self::List::InputDescriptors)),
            ("subconditions", List(self::List::SubconditionDescriptors)),
        ],
        self::Object::InputDescriptor => &[("field", String), ("required_when", String)],
        self::Object::SubconditionDescriptor => &[
            ("id", String),
            ("required_inputs", List(self::List::InputNames)),
        ],
        self::Object::ConfiguredRule => &[
            ("id", String),
            ("version", U16),
            ("priority", U8),
            ("enabled", Bool),
            ("required_inputs", List(self::List::RequiredInputs)),
            ("subconditions", List(self::List::ConfiguredSubconditions)),
            ("thresholds", Thresholds),
        ],
        self::Object::RequiredInput => &[("field", String), ("required", Bool)],
        self::Object::ConfiguredSubcondition => &[("id", String), ("enabled", Bool)],
        self::Object::BiasThresholds => &[("bias_threshold", Float)],
        self::Object::FlowThresholds => &[
            ("outflow_threshold", Float),
            ("lure_pct_threshold", Float),
            ("lure_outflow_threshold", Float),
        ],
        self::Object::FundamentalThresholds => {
            &[("pe_upper", Float), ("profit_decline_threshold", Float)]
        }
        self::Object::Candidate => &[
            ("ordinal", Usize),
            ("source_row_id", I64),
            ("raw_row_sha256", String),
            ("required_fields", List(self::List::FieldAssessments)),
            ("rules", List(self::List::RuleAssessments)),
            ("disposition", Enum(DISPOSITIONS)),
        ],
        self::Object::FieldAssessment => &[
            ("field", Enum(REQUIRED_FIELD_NAMES)),
            (
                "action_fact_state",
                Enum(&["Admitted", "Missing", "Stale", "Conflict", "Unqualified"]),
            ),
            ("assessment", Assessment),
        ],
        self::Object::RuleAssessment => &[
            ("rule_id", String),
            ("rule_version", U16),
            ("priority", U8),
            ("enabled", Bool),
            ("required_inputs", List(self::List::RuleInputs)),
            ("subconditions", List(self::List::RuleSubconditions)),
            ("evaluation", RuleEvaluation),
        ],
        self::Object::RuleInput => &[
            ("field", String),
            ("required", Bool),
            (
                "status",
                Enum(&[
                    "NotAcquiredBlockedByNativeIdentity",
                    "NotRequiredByDisabledCondition",
                ]),
            ),
        ],
        self::Object::RuleSubcondition => &[
            ("id", String),
            ("enabled", Bool),
            ("evaluation", RuleEvaluation),
        ],
    }
}
fn list_shape(kind: List) -> (Node, usize, usize, usize) {
    use std::mem::size_of;
    // Header and inline slots already belong to the containing DTO; only the
    // element backing storage is charged in list(). All element types are nonzero.
    match kind {
        List::RequiredFields => (
            Node::Enum(REQUIRED_FIELD_NAMES),
            12,
            12,
            size_of::<RequiredField>(),
        ),
        List::Candidates => (
            Node::Object(Object::Candidate),
            0,
            50,
            size_of::<Candidate>(),
        ),
        List::Descriptors => (
            Node::Object(Object::RuleDescriptor),
            3,
            3,
            size_of::<RuleDescriptor>(),
        ),
        List::ConfiguredRules => (
            Node::Object(Object::ConfiguredRule),
            3,
            3,
            size_of::<ConfiguredRule>(),
        ),
        List::InputDescriptors => (
            Node::Object(Object::InputDescriptor),
            0,
            5,
            size_of::<InputDescriptor>(),
        ),
        List::SubconditionDescriptors => (
            Node::Object(Object::SubconditionDescriptor),
            0,
            2,
            size_of::<SubconditionDescriptor>(),
        ),
        List::InputNames => (Node::String, 0, 5, size_of::<String>()),
        List::RequiredInputs => (
            Node::Object(Object::RequiredInput),
            0,
            5,
            size_of::<RequiredInput>(),
        ),
        List::ConfiguredSubconditions => (
            Node::Object(Object::ConfiguredSubcondition),
            0,
            2,
            size_of::<Subcondition>(),
        ),
        List::FieldAssessments => (
            Node::Object(Object::FieldAssessment),
            12,
            12,
            size_of::<FieldAssessment>(),
        ),
        List::RuleAssessments => (
            Node::Object(Object::RuleAssessment),
            3,
            3,
            size_of::<RuleAssessment>(),
        ),
        List::RuleInputs => (
            Node::Object(Object::RuleInput),
            0,
            5,
            size_of::<RuleInput>(),
        ),
        List::RuleSubconditions => (
            Node::Object(Object::RuleSubcondition),
            0,
            2,
            size_of::<RuleSubcondition>(),
        ),
    }
}
#[derive(Default)]
struct DecodeBudget {
    metadata: usize,
    row: Option<usize>,
    rows: usize,
}
impl DecodeBudget {
    fn charge(&mut self, amount: usize) -> Result<(), CodecError> {
        self.metadata = self
            .metadata
            .checked_add(amount)
            .ok_or(CodecError::Bounds)?;
        if self.metadata > MAX_META {
            return Err(CodecError::Bounds);
        }
        if let Some(row) = &mut self.row {
            *row = row.checked_add(amount).ok_or(CodecError::Bounds)?;
            if *row > MAX_ROW {
                return Err(CodecError::Bounds);
            }
        }
        Ok(())
    }
}
struct Preflight<'a> {
    bytes: &'a [u8],
    at: usize,
    budget: DecodeBudget,
}
impl Preflight<'_> {
    fn byte(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }
    fn take(&mut self, expected: &[u8]) -> Result<(), CodecError> {
        if !self.bytes[self.at..].starts_with(expected) {
            return Err(CodecError::Invalid);
        }
        self.budget.charge(expected.len())?;
        self.at += expected.len();
        Ok(())
    }
    fn key(&mut self, expected: &str) -> Result<(), CodecError> {
        self.take(b"\"")?;
        self.take(expected.as_bytes())?;
        self.take(b"\":")
    }
    fn choice(&mut self, names: &[&str]) -> Result<usize, CodecError> {
        // Closed enum tags/keys are canonical unescaped literals, never owned.
        self.take(b"\"")?;
        for (i, name) in names.iter().enumerate() {
            let rest = &self.bytes[self.at..];
            if rest.starts_with(name.as_bytes()) && rest.get(name.len()) == Some(&b'"') {
                self.take(name.as_bytes())?;
                self.take(b"\"")?;
                return Ok(i);
            }
        }
        Err(CodecError::Invalid)
    }
    fn fields(&mut self, fields: &[(&str, Node)], depth: u8) -> Result<(), CodecError> {
        for (i, (key, node)) in fields.iter().enumerate() {
            if i != 0 {
                self.take(b",")?;
            }
            self.key(key)?;
            self.node(*node, depth + 1)?;
        }
        Ok(())
    }
    fn tagged_reservation(&mut self) -> Result<(), CodecError> {
        // Pinned serde 1.0.228 private/de.rs TaggedContentVisitor collects the
        // NON-tag entries into Vec<(Content,Content)>. All our tagged variants
        // contain <=3 such entries and only scalar values. Reserve four pairs.
        // Content's largest payload is three words (Vec/String), plus a tag;
        // using (u64,[usize;3]) is a conservative portable slot reservation.
        // Vec header is transient, not part of the owned DTO slot.
        let slot = std::mem::size_of::<(u64, [usize; 3])>();
        let amount = slot
            .checked_mul(8)
            .and_then(|n| n.checked_add(std::mem::size_of::<Vec<()>>()))
            .ok_or(CodecError::Bounds)?;
        self.budget.charge(amount)
    }
    fn node(&mut self, node: Node, depth: u8) -> Result<(), CodecError> {
        if depth > 16 {
            return Err(CodecError::Bounds);
        }
        match node {
            Node::String => self.string(false),
            Node::ScopeCanonical => self.string(true),
            Node::Bool => {
                if self.byte() == Some(b't') {
                    self.take(b"true")
                } else {
                    self.take(b"false")
                }
            }
            Node::U8 | Node::U16 | Node::Usize | Node::I64 | Node::Float => self.number(node),
            Node::Enum(names) => self.choice(names).map(|_| ()),
            Node::Object(kind) => {
                self.take(b"{")?;
                self.fields(object_fields(kind), depth)?;
                self.take(b"}")
            }
            Node::List(kind) => self.list(kind, depth),
            Node::Calendar => {
                self.take(b"{")?;
                self.key("state")?;
                let variant = self.choice(&["Available", "Unavailable"])?;
                self.tagged_reservation()?;
                self.take(b",")?;
                if variant == 0 {
                    self.fields(
                        &[
                            ("contract", Node::String),
                            ("authority_sha256", Node::String),
                            ("open", Node::Bool),
                        ],
                        depth,
                    )?;
                } else {
                    self.fields(&[("reason", Node::String)], depth)?;
                }
                self.take(b"}")
            }
            Node::Assessment | Node::RuleEvaluation => {
                self.take(b"{")?;
                self.key("state")?;
                let reasons = if matches!(node, Node::Assessment) {
                    self.choice(&["Unavailable", "NotEvaluated", "AcquisitionNotInvoked"])?;
                    REASON_NAMES
                } else {
                    self.choice(&["NotEvaluated"])?;
                    RULE_REASON_NAMES
                };
                self.tagged_reservation()?;
                self.take(b",")?;
                self.key("reason")?;
                self.choice(reasons)?;
                self.take(b"}")
            }
            Node::Thresholds => {
                self.take(b"{")?;
                let variant = self.choice(&["BiasRate", "MainFlow", "FundamentalDeterioration"])?;
                self.take(b":")?;
                self.node(
                    Node::Object(match variant {
                        0 => Object::BiasThresholds,
                        1 => Object::FlowThresholds,
                        _ => Object::FundamentalThresholds,
                    }),
                    depth + 1,
                )?;
                self.take(b"}")
            }
        }
    }
    fn list(&mut self, kind: List, depth: u8) -> Result<(), CodecError> {
        let (element, min, max, element_size) = list_shape(kind);
        self.take(b"[")?;
        let mut count = 0usize;
        let mut capacity = 0usize;
        while self.byte() != Some(b']') {
            if count == max {
                return Err(CodecError::Bounds);
            }
            if count != 0 {
                self.take(b",")?;
            }
            let candidate = matches!(kind, List::Candidates);
            if candidate {
                self.budget.row = Some(0);
            }
            // serde VecVisitor uses cautious(None)==0 then push. Reserve a
            // power-of-two backing capacity, floor 8 for byte-size T, otherwise
            // 4. This bounds ordinary growth without charging inline slots twice.
            let needed = count.checked_add(1).ok_or(CodecError::Bounds)?;
            let next_capacity = needed
                .checked_next_power_of_two()
                .ok_or(CodecError::Bounds)?
                .max(if element_size == 1 { 8 } else { 4 });
            let growth = next_capacity
                .checked_sub(capacity)
                .ok_or(CodecError::Bounds)?
                .checked_mul(element_size)
                .ok_or(CodecError::Bounds)?;
            self.budget.charge(growth)?;
            capacity = next_capacity;
            self.node(element, depth + 1)?;
            if candidate {
                let row = self.budget.row.take().ok_or(CodecError::Invalid)?;
                self.budget.rows = self
                    .budget
                    .rows
                    .checked_add(row)
                    .ok_or(CodecError::Bounds)?;
                if self.budget.rows > MAX_ROWS {
                    return Err(CodecError::Bounds);
                }
            }
            count = needed;
        }
        if count < min {
            return Err(CodecError::Invalid);
        }
        self.take(b"]")
    }
    fn hex4(&mut self) -> Result<u16, CodecError> {
        let mut n = 0u16;
        for _ in 0..4 {
            let digit = match self.byte().ok_or(CodecError::Invalid)? {
                c @ b'0'..=b'9' => c - b'0',
                c @ b'a'..=b'f' => c - b'a' + 10,
                c @ b'A'..=b'F' => c - b'A' + 10,
                _ => return Err(CodecError::Invalid),
            };
            self.at += 1;
            n = n * 16 + u16::from(digit);
        }
        Ok(n)
    }
    fn string(&mut self, scope: bool) -> Result<(), CodecError> {
        if self.byte() != Some(b'"') {
            return Err(CodecError::Invalid);
        }
        let start = self.at;
        self.at += 1;
        let mut decoded = 0usize;
        let mut saw_escape = false;
        loop {
            let c = self.byte().ok_or(CodecError::Invalid)?;
            self.at += 1;
            let width = match c {
                b'"' => break,
                0..=31 => return Err(CodecError::Invalid),
                b'\\' => {
                    saw_escape = true;
                    let escaped = self.byte().ok_or(CodecError::Invalid)?;
                    self.at += 1;
                    match escaped {
                        b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => 1,
                        b'u' => {
                            let high = self.hex4()?;
                            let code = if (0xd800..=0xdbff).contains(&high) {
                                if !self.bytes[self.at..].starts_with(b"\\u") {
                                    return Err(CodecError::Invalid);
                                }
                                self.at += 2;
                                let low = self.hex4()?;
                                if !(0xdc00..=0xdfff).contains(&low) {
                                    return Err(CodecError::Invalid);
                                }
                                0x10000 + ((u32::from(high) - 0xd800) << 10) + u32::from(low)
                                    - 0xdc00
                            } else if (0xdc00..=0xdfff).contains(&high) {
                                return Err(CodecError::Invalid);
                            } else {
                                u32::from(high)
                            };
                            char::from_u32(code).ok_or(CodecError::Invalid)?.len_utf8()
                        }
                        _ => return Err(CodecError::Invalid),
                    }
                }
                _ => 1, // Raw UTF-8 bytes counted individually, validated below.
            };
            decoded = decoded.checked_add(width).ok_or(CodecError::Bounds)?;
            if decoded > if scope { 8 * 1024 * 1024 } else { 16 * 1024 } {
                return Err(CodecError::Bounds);
            }
        }
        std::str::from_utf8(&self.bytes[start..self.at]).map_err(|_| CodecError::Invalid)?;
        if !scope {
            // Wire span + owned string + escaped-token scratch capacity.
            // SliceRead borrows unescaped tokens. Reserve byte-Vec capacity with
            // floor 8 for escaped tokens, cumulatively rather than reusing it.
            let scratch = if saw_escape && decoded != 0 {
                decoded
                    .checked_next_power_of_two()
                    .ok_or(CodecError::Bounds)?
                    .max(8)
            } else {
                0
            };
            let amount = decoded
                .checked_add(scratch)
                .and_then(|n| n.checked_add(self.at - start))
                .ok_or(CodecError::Bounds)?;
            self.budget.charge(amount)?;
        }
        Ok(())
    }
    fn number(&mut self, node: Node) -> Result<(), CodecError> {
        let start = self.at;
        if self.byte() == Some(b'-') {
            self.at += 1;
        }
        match self.byte() {
            Some(b'0') => self.at += 1,
            Some(b'1'..=b'9') => {
                while matches!(self.byte(), Some(b'0'..=b'9')) {
                    self.at += 1;
                    if self.at - start > 64 {
                        return Err(CodecError::Bounds);
                    }
                }
            }
            _ => return Err(CodecError::Invalid),
        }
        if matches!(node, Node::Float) {
            if self.byte() == Some(b'.') {
                self.at += 1;
                let before = self.at;
                while matches!(self.byte(), Some(b'0'..=b'9')) {
                    self.at += 1;
                    if self.at - start > 64 {
                        return Err(CodecError::Bounds);
                    }
                }
                if self.at == before {
                    return Err(CodecError::Invalid);
                }
            }
            if matches!(self.byte(), Some(b'e' | b'E')) {
                self.at += 1;
                if matches!(self.byte(), Some(b'+' | b'-')) {
                    self.at += 1;
                }
                let before = self.at;
                while matches!(self.byte(), Some(b'0'..=b'9')) {
                    self.at += 1;
                    if self.at - start > 64 {
                        return Err(CodecError::Bounds);
                    }
                }
                if self.at == before {
                    return Err(CodecError::Invalid);
                }
            }
        }
        if self.at - start > 64 {
            return Err(CodecError::Bounds);
        }
        let token =
            std::str::from_utf8(&self.bytes[start..self.at]).map_err(|_| CodecError::Invalid)?;
        let valid = match node {
            Node::U8 => token.parse::<u8>().is_ok(),
            Node::U16 => token.parse::<u16>().is_ok(),
            Node::Usize => token.parse::<usize>().is_ok(),
            Node::I64 => token.parse::<i64>().is_ok(),
            Node::Float => token.parse::<f64>().is_ok_and(f64::is_finite),
            _ => false,
        };
        if !valid {
            return Err(CodecError::Invalid);
        }
        self.budget.charge(self.at - start)
    }
}
fn preflight(bytes: &[u8]) -> Result<(), CodecError> {
    // Every stored record still has exactly this root and this reservation.
    preflight_root(
        bytes,
        Node::Object(Object::Root),
        std::mem::size_of::<Record>(),
    )
}
fn preflight_root(bytes: &[u8], root: Node, inline_slot: usize) -> Result<(), CodecError> {
    if bytes.is_empty() || bytes.len() > MAX_RECORD_BYTES {
        return Err(CodecError::Bounds);
    }
    let mut p = Preflight {
        bytes,
        at: 0,
        budget: DecodeBudget::default(),
    };
    // Covers all root inline slots/headers. Backing Vec slots and string bytes
    // are charged separately, not each nested inline object a second time.
    p.budget.charge(inline_slot)?;
    p.node(root, 0)?;
    if p.at != bytes.len() {
        return Err(CodecError::Invalid);
    }
    Ok(())
}

#[cfg(test)]
std::thread_local! {
    static DECODE_PROBE: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}
#[cfg(test)]
impl super::investment_decision_v1::InvestmentDecisionId {
    // Access through an already visible type keeps the codec module private and
    // adds no production ABI or third-file visibility change.
    pub(crate) fn decode_probe_for_test(reset: bool) -> (usize, usize) {
        DECODE_PROBE.with(|p| {
            let value = p.get();
            if reset {
                p.set((0, 0));
            }
            value
        })
    }
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
    fn node_preflight(bytes: &[u8], node: Node) -> Result<(), CodecError> {
        let mut p = Preflight {
            bytes,
            at: 0,
            budget: DecodeBudget::default(),
        };
        p.node(node, 0)?;
        if p.at != bytes.len() {
            return Err(CodecError::Invalid);
        }
        Ok(())
    }
    #[test]
    fn investment_record_preflight_closed_node_types_and_tagged_shapes() {
        for bad in [
            br#"{"state":"Unavailable","reason":[]}"#.as_slice(),
            br#"{"state":"Unavailable","reason":0}"#,
            br#"{"state":"Unavailable","reason":null}"#,
            br#"{"state":"Unavailable","reason":"ok","canonical_utf8":"x"}"#,
            br#"{"state":"Unavailable","canonical_utf8":"x","reason":"ok"}"#,
            br#"{"state":"Unavailable","reason":"ok","reason":"duplicate"}"#,
            br#"{"state":"Unavailable"}"#,
            br#"{"reason":"ok","state":"Unavailable"}"#,
            br#"{"state":"Future","reason":"ok"}"#,
            br#"{"state":"Available","contract":"x","authority_sha256":"x","open":1}"#,
        ] {
            assert!(node_preflight(bad, Node::Calendar).is_err());
        }
        assert!(
            node_preflight(br#"{"state":"Unavailable","reason":"ok"}"#, Node::Calendar).is_ok()
        );
        assert!(node_preflight(b"[[0]]", Node::List(List::InputNames)).is_err());
        assert!(node_preflight(br#"["a","b","c","d","e"]"#, Node::List(List::InputNames)).is_ok());
        assert!(node_preflight(
            br#"["a","b","c","d","e","f"]"#,
            Node::List(List::InputNames)
        )
        .is_err());
        assert!(node_preflight(b"[]", Node::List(List::RequiredFields)).is_err());
        for (bytes, node) in [
            (b"256".as_slice(), Node::U8),
            (b"65536", Node::U16),
            (b"9223372036854775808", Node::I64),
            (b"01", Node::U16),
            (b"1.0", Node::U16),
            (b"1e999", Node::Float),
            (b"truex", Node::Bool),
        ] {
            assert!(node_preflight(bytes, node).is_err());
        }
    }
    #[test]
    fn investment_record_preflight_escaped_tokens_and_unique_scope_budget() {
        for bad in [
            br#""\x""#.as_slice(),
            br#""\u123""#,
            br#""\ud800""#,
            br#""\udc00""#,
            br#""\ud800\u0041""#,
            b"\"unterminated",
            b"\"\xff\"",
        ] {
            assert!(node_preflight(bad, Node::String).is_err());
        }
        assert!(node_preflight(r#""\ud83d\ude00\"\\\n雪""#.as_bytes(), Node::String).is_ok());
        let exact = format!("\"{}\"", "\\u0061".repeat(16 * 1024));
        assert!(node_preflight(exact.as_bytes(), Node::String).is_ok());
        let excess = format!("\"{}\"", "\\u0061".repeat(16 * 1024 + 1));
        assert!(node_preflight(excess.as_bytes(), Node::String).is_err());
        let scope = format!("\"{}\"", "x".repeat(8 * 1024 * 1024));
        assert!(node_preflight(scope.as_bytes(), Node::ScopeCanonical).is_ok());
        assert!(node_preflight(scope.as_bytes(), Node::String).is_err());
        let excess_scope = format!("\"{}\"", "x".repeat(8 * 1024 * 1024 + 1));
        assert!(node_preflight(excess_scope.as_bytes(), Node::ScopeCanonical).is_err());
    }
    #[test]
    fn investment_record_preflight_checked_cumulative_reservations() {
        let mut b = DecodeBudget::default();
        b.charge(MAX_META).unwrap();
        assert!(b.charge(1).is_err());
        let mut b = DecodeBudget {
            metadata: usize::MAX,
            ..DecodeBudget::default()
        };
        assert!(b.charge(1).is_err());
        let mut b = DecodeBudget {
            row: Some(MAX_ROW),
            ..DecodeBudget::default()
        };
        assert!(b.charge(1).is_err());
        // Each member is a legal short token, but the complete descriptor's
        // encoded/owned/scratch reservation must fit the one metadata budget.
        let token = format!("\"{}\"", "\\u0061".repeat(16 * 1024));
        let value = format!("[{}]", vec![token; 5].join(","));
        let mut p = Preflight {
            bytes: value.as_bytes(),
            at: 0,
            budget: DecodeBudget::default(),
        };
        p.budget.charge(MAX_META / 2).unwrap();
        assert!(p.node(Node::List(List::InputNames), 0).is_err());
        let mut p = Preflight {
            bytes: b"true",
            at: 0,
            budget: DecodeBudget::default(),
        };
        assert!(p.node(Node::Bool, 17).is_err());
    }
    #[test]
    fn investment_record_preflight_explicit_task4_copy_roots_preserve_record_boundary() {
        let snapshot = VetoConfigurationSnapshotV1::from_live_config(
            &crate::config::LiveVetoConfig::default(),
        )
        .unwrap();
        let config: Configuration = value_copy(&snapshot, ValueCopySchema::Configuration).unwrap();
        let descriptors = catalog().unwrap();
        assert_eq!(config.rules.len(), 3);
        assert_eq!(descriptors.len(), 3);
        let config_bytes = encode(&snapshot, MAX_META).unwrap();
        let catalog_bytes = encode(builtin_rule_catalog_v1(), MAX_META).unwrap();
        // No autodetection: the independently bounded fragments must remain
        // invalid as stored Record roots, and cannot use each other's root.
        assert!(preflight(&config_bytes).is_err());
        assert!(preflight(&catalog_bytes).is_err());
        assert!(value_copy::<_, Configuration>(
            builtin_rule_catalog_v1(),
            ValueCopySchema::Configuration
        )
        .is_err());
        assert!(value_copy::<_, Vec<RuleDescriptor>>(
            &snapshot,
            ValueCopySchema::BuiltinDescriptors
        )
        .is_err());
    }
}
