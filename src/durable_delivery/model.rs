use chrono::{DateTime, NaiveDate, Utc};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

pub const ENVELOPE_VERSION: i64 = 1;
pub const POLICY_VERSION: i64 = 1;
pub const DAILY_BUDGET_LIMIT: i64 = 30;
pub(crate) const MANUAL_ACCEPTED_DELIVERY_AUDIT_DOMAIN: &str = "manual-delivery-accepted-audit-v1";

#[derive(Debug, thiserror::Error)]
pub enum DurableDeliveryError {
    #[error("invalid durable-delivery configuration: {0}")]
    InvalidConfiguration(String),
    #[error("test/live durable-delivery isolation violation: {0}")]
    IsolationViolation(String),
    #[error("invalid delivery envelope: {0}")]
    InvalidEnvelope(String),
    #[error("delivery policy mismatch: {0}")]
    PolicyMismatch(String),
    #[error(
        "decision identity conflict for {decision_identity}; immutable conflict audit is pending"
    )]
    DecisionIdentityConflict { decision_identity: String },
    #[error("decision {0} is not found")]
    DecisionNotFound(String),
    #[error("illegal delivery state transition: {from} -> {to}")]
    IllegalTransition { from: String, to: String },
    #[error("manual resolution rejected: {0}")]
    InvalidManualResolution(String),
    #[error("immutable append conflict for identity {0}")]
    ImmutableAppendConflict(String),
    #[error("immutable audit predecessor is missing or cyclic")]
    AuditPredecessorBlocked,
    #[error("sqlite durable-delivery failure: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("serialization failure: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("io failure: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, DurableDeliveryError>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StoreEnvironment {
    Production,
    Test { test_code: String },
}

#[derive(Clone, Debug)]
pub struct CoordinatorConfig {
    pub database_path: PathBuf,
    pub environment: StoreEnvironment,
    pub attempt_lease_secs: i64,
    pub owner_instance_identity: String,
}

impl CoordinatorConfig {
    pub fn production(owner_instance_identity: impl Into<String>) -> Self {
        Self {
            database_path: PathBuf::from("data/durable_delivery.sqlite3"),
            environment: StoreEnvironment::Production,
            attempt_lease_secs: 120,
            owner_instance_identity: owner_instance_identity.into(),
        }
    }

    pub fn test(
        database_path: impl Into<PathBuf>,
        test_code: impl Into<String>,
        owner_instance_identity: impl Into<String>,
    ) -> Self {
        Self {
            database_path: database_path.into(),
            environment: StoreEnvironment::Test {
                test_code: test_code.into(),
            },
            attempt_lease_secs: 120,
            owner_instance_identity: owner_instance_identity.into(),
        }
    }

    pub(crate) fn validate(&self) -> Result<()> {
        self.repository_relative_database_path().map(|_| ())
    }

    pub(crate) fn repository_relative_database_path(&self) -> Result<PathBuf> {
        if !(30..=900).contains(&self.attempt_lease_secs) {
            return Err(DurableDeliveryError::InvalidConfiguration(format!(
                "attempt_lease_secs={} is outside [30,900]",
                self.attempt_lease_secs
            )));
        }
        if self.owner_instance_identity.trim().len() < 16 {
            return Err(DurableDeliveryError::InvalidConfiguration(
                "owner_instance_identity must be process-unique and at least 128 bits encoded"
                    .to_owned(),
            ));
        }
        let rendered = self.database_path.to_string_lossy();
        match &self.environment {
            StoreEnvironment::Production => {
                if rendered.contains("TEST_CODE") {
                    return Err(DurableDeliveryError::IsolationViolation(
                        "production cannot open a TEST_CODE database path".to_owned(),
                    ));
                }
                if self.database_path != Path::new("data/durable_delivery.sqlite3") {
                    return Err(DurableDeliveryError::IsolationViolation(format!(
                        "production database path must be data/durable_delivery.sqlite3, got {rendered}"
                    )));
                }
                Ok(self.database_path.clone())
            }
            StoreEnvironment::Test { test_code } => {
                if !test_code.starts_with("TEST_CODE") {
                    return Err(DurableDeliveryError::IsolationViolation(
                        "test identity must start with TEST_CODE".to_owned(),
                    ));
                }
                let mut test_code_components = Path::new(test_code).components();
                if !matches!(
                    (test_code_components.next(), test_code_components.next()),
                    (Some(Component::Normal(component)), None)
                        if component == std::ffi::OsStr::new(test_code)
                ) {
                    return Err(DurableDeliveryError::IsolationViolation(format!(
                        "test identity must be one lexical path component: {test_code}"
                    )));
                }
                let expected_database_path = PathBuf::from("data")
                    .join("test")
                    .join(test_code)
                    .join("durable_delivery.sqlite3");
                let manifest_absolute =
                    Path::new(env!("CARGO_MANIFEST_DIR")).join(&expected_database_path);
                if self.database_path != expected_database_path
                    && self.database_path != manifest_absolute
                {
                    return Err(DurableDeliveryError::IsolationViolation(format!(
                        "test database path must be exactly {} or {}, got {rendered}",
                        expected_database_path.display(),
                        manifest_absolute.display()
                    )));
                }
                Ok(expected_database_path)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum PushKind {
    HoldingPlan,
    HoldingEvent,
    T0Advice,
    CandidateTriggered,
    CloseCall,
    ForbiddenOps,
    PaperTrade,
    ReviewMarket,
    ReviewLhb,
    ReviewSignal,
    ReviewFailure,
    TomorrowWatch,
    EventCalendar,
    DailyReport,
    ReviewProviderTopN,
}

impl PushKind {
    pub const ALL: [Self; 15] = [
        Self::HoldingPlan,
        Self::HoldingEvent,
        Self::T0Advice,
        Self::CandidateTriggered,
        Self::CloseCall,
        Self::ForbiddenOps,
        Self::PaperTrade,
        Self::ReviewMarket,
        Self::ReviewLhb,
        Self::ReviewSignal,
        Self::ReviewFailure,
        Self::TomorrowWatch,
        Self::EventCalendar,
        Self::DailyReport,
        Self::ReviewProviderTopN,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::HoldingPlan => "HoldingPlan",
            Self::HoldingEvent => "HoldingEvent",
            Self::T0Advice => "T0Advice",
            Self::CandidateTriggered => "CandidateTriggered",
            Self::CloseCall => "CloseCall",
            Self::ForbiddenOps => "ForbiddenOps",
            Self::PaperTrade => "PaperTrade",
            Self::ReviewMarket => "ReviewMarket",
            Self::ReviewLhb => "ReviewLhb",
            Self::ReviewSignal => "ReviewSignal",
            Self::ReviewFailure => "ReviewFailure",
            Self::TomorrowWatch => "TomorrowWatch",
            Self::EventCalendar => "EventCalendar",
            Self::DailyReport => "DailyReport",
            Self::ReviewProviderTopN => "ReviewProviderTopN",
        }
    }

    pub const fn stable_template_id(self) -> &'static str {
        match self {
            Self::HoldingPlan => "holding_plan_v1",
            Self::HoldingEvent => "holding_event_v1",
            Self::T0Advice => "t0_advice_v1",
            Self::CandidateTriggered => "candidate_triggered_v1",
            Self::CloseCall => "close_call_v1",
            Self::ForbiddenOps => "forbidden_ops_v1",
            Self::PaperTrade => "paper_trade_v1",
            Self::ReviewMarket => "review_market_v1",
            Self::ReviewLhb => "review_lhb_v1",
            Self::ReviewSignal => "review_signal_v1",
            Self::ReviewFailure => "review_failure_v1",
            Self::TomorrowWatch => "tomorrow_watch_v1",
            Self::EventCalendar => "event_calendar_v1",
            Self::DailyReport => "daily_report_v1",
            Self::ReviewProviderTopN => "review_provider_top_n_v1",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|candidate| candidate.as_str() == value)
            .ok_or_else(|| {
                DurableDeliveryError::InvalidEnvelope(format!("unknown push kind {value}"))
            })
    }
}

impl fmt::Display for PushKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub enum DeliverySubKind {
    #[serde(rename = "NONE")]
    None,
    FactorIC,
    SectorTier,
    CapitalVerify,
}

impl DeliverySubKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "NONE",
            Self::FactorIC => "FactorIC",
            Self::SectorTier => "SectorTier",
            Self::CapitalVerify => "CapitalVerify",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self> {
        match value {
            "NONE" => Ok(Self::None),
            "FactorIC" => Ok(Self::FactorIC),
            "SectorTier" => Ok(Self::SectorTier),
            "CapitalVerify" => Ok(Self::CapitalVerify),
            other => Err(DurableDeliveryError::InvalidEnvelope(format!(
                "unknown sub-kind {other}"
            ))),
        }
    }
}

impl fmt::Display for DeliverySubKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CooldownScope {
    Global,
    PerTicket,
}

impl CooldownScope {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Global => "Global",
            Self::PerTicket => "PerTicket",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self> {
        match value {
            "Global" => Ok(Self::Global),
            "PerTicket" => Ok(Self::PerTicket),
            other => Err(DurableDeliveryError::InvalidEnvelope(format!(
                "unknown cooldown scope {other}"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowMode {
    None,
    Rolling,
    BusinessDateOnce,
}

impl WindowMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Rolling => "Rolling",
            Self::BusinessDateOnce => "BusinessDateOnce",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self> {
        match value {
            "None" => Ok(Self::None),
            "Rolling" => Ok(Self::Rolling),
            "BusinessDateOnce" => Ok(Self::BusinessDateOnce),
            other => Err(DurableDeliveryError::PolicyMismatch(format!(
                "unknown window mode {other}"
            ))),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyRow {
    pub push_kind: PushKind,
    pub sub_kind: DeliverySubKind,
    pub cooldown_scope: CooldownScope,
    pub base_cooldown_secs: Option<i64>,
    pub override_cooldown_secs: Option<i64>,
    pub window_mode: WindowMode,
    pub counts_against_daily_budget: bool,
    pub policy_version: i64,
}

impl PolicyRow {
    pub fn effective_cooldown_secs(&self) -> Option<i64> {
        self.override_cooldown_secs.or(self.base_cooldown_secs)
    }
}

pub fn compiled_policy_catalog() -> Vec<PolicyRow> {
    use CooldownScope::{Global, PerTicket};
    use DeliverySubKind::{CapitalVerify, FactorIC, None, SectorTier};
    use PushKind::*;
    use WindowMode::{BusinessDateOnce, Rolling};

    let mut rows = vec![
        (HoldingPlan, PerTicket, Some(1_800), Rolling),
        (
            HoldingEvent,
            Global,
            std::option::Option::None,
            WindowMode::None,
        ),
        (T0Advice, PerTicket, Some(1_800), Rolling),
        (CandidateTriggered, PerTicket, Some(86_400), Rolling),
        (CloseCall, Global, Some(86_400), Rolling),
        (ForbiddenOps, PerTicket, Some(3_600), Rolling),
        (PaperTrade, PerTicket, Some(300), Rolling),
        (ReviewMarket, Global, Some(86_400), Rolling),
        (ReviewLhb, Global, Some(86_400), Rolling),
        (ReviewSignal, Global, Some(86_400), Rolling),
        (ReviewFailure, Global, Some(86_400), Rolling),
        (TomorrowWatch, Global, Some(86_400), Rolling),
        (EventCalendar, Global, Some(86_400), Rolling),
    ]
    .into_iter()
    .map(
        |(push_kind, cooldown_scope, base_cooldown_secs, window_mode)| PolicyRow {
            push_kind,
            sub_kind: None,
            cooldown_scope,
            base_cooldown_secs,
            override_cooldown_secs: std::option::Option::None,
            window_mode,
            counts_against_daily_budget: true,
            policy_version: POLICY_VERSION,
        },
    )
    .collect::<Vec<_>>();

    rows.extend([
        PolicyRow {
            push_kind: DailyReport,
            sub_kind: None,
            cooldown_scope: Global,
            base_cooldown_secs: Some(86_400),
            override_cooldown_secs: std::option::Option::None,
            window_mode: Rolling,
            counts_against_daily_budget: true,
            policy_version: POLICY_VERSION,
        },
        PolicyRow {
            push_kind: DailyReport,
            sub_kind: FactorIC,
            cooldown_scope: Global,
            base_cooldown_secs: Some(86_400),
            override_cooldown_secs: std::option::Option::None,
            window_mode: Rolling,
            counts_against_daily_budget: true,
            policy_version: POLICY_VERSION,
        },
        PolicyRow {
            push_kind: DailyReport,
            sub_kind: SectorTier,
            cooldown_scope: Global,
            base_cooldown_secs: Some(86_400),
            override_cooldown_secs: Some(1_800),
            window_mode: Rolling,
            counts_against_daily_budget: true,
            policy_version: POLICY_VERSION,
        },
        PolicyRow {
            push_kind: DailyReport,
            sub_kind: CapitalVerify,
            cooldown_scope: Global,
            base_cooldown_secs: Some(86_400),
            override_cooldown_secs: Some(1_800),
            window_mode: Rolling,
            counts_against_daily_budget: true,
            policy_version: POLICY_VERSION,
        },
        PolicyRow {
            push_kind: ReviewProviderTopN,
            sub_kind: None,
            cooldown_scope: Global,
            base_cooldown_secs: Some(86_400),
            override_cooldown_secs: std::option::Option::None,
            window_mode: BusinessDateOnce,
            counts_against_daily_budget: true,
            policy_version: POLICY_VERSION,
        },
    ]);
    rows
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TaskBinding {
    pub task_identity: String,
    pub transition_basis_canonical: Vec<u8>,
    pub transition_basis_sha256: String,
}

impl TaskBinding {
    pub fn new(
        task_identity: impl Into<String>,
        transition_basis_canonical: Vec<u8>,
    ) -> Result<Self> {
        let task_identity = task_identity.into();
        if task_identity.trim().is_empty() || transition_basis_canonical.is_empty() {
            return Err(DurableDeliveryError::InvalidEnvelope(
                "task binding identity and transition basis must be non-empty".to_owned(),
            ));
        }
        let transition_basis_sha256 = sha256_hex(&transition_basis_canonical);
        Ok(Self {
            task_identity,
            transition_basis_canonical,
            transition_basis_sha256,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeliveryEnvelope {
    pub envelope_version: i64,
    pub decision_identity: String,
    pub business_date: String,
    pub push_kind: PushKind,
    pub sub_kind: DeliverySubKind,
    pub cooldown_scope: CooldownScope,
    pub scope_key: String,
    pub schedule_occurrence_identity: String,
    pub source_evidence_fingerprint: String,
    pub source_binding_canonical: Vec<u8>,
    pub source_binding_sha256: String,
    pub delivery_subject_hash: String,
    pub rendered_content: Vec<u8>,
    pub rendered_content_sha256: String,
    pub policy_version: i64,
    pub retry_authorized: bool,
    pub provider_observed_at: Option<String>,
    pub provider_as_of: Option<String>,
    pub original_batch_ids: Vec<String>,
    pub task_binding: Option<TaskBinding>,
}

#[derive(Serialize)]
struct DecisionIdentityMaterial<'a> {
    domain: &'static str,
    policy_version: i64,
    business_date: &'a str,
    push_kind: PushKind,
    sub_kind: DeliverySubKind,
    cooldown_scope: CooldownScope,
    scope_key: &'a str,
    schedule_occurrence_identity: &'a str,
    source_evidence_fingerprint: &'a str,
    delivery_subject_hash: &'a str,
    rendered_content_sha256: &'a str,
}

impl DeliveryEnvelope {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        business_date: impl Into<String>,
        push_kind: PushKind,
        sub_kind: DeliverySubKind,
        scope_key: impl Into<String>,
        schedule_occurrence_identity: impl Into<String>,
        source_evidence_fingerprint: impl Into<String>,
        source_binding_canonical: Vec<u8>,
        delivery_subject_hash: impl Into<String>,
        rendered_content: Vec<u8>,
        retry_authorized: bool,
        task_binding: Option<TaskBinding>,
    ) -> Result<Self> {
        let business_date = business_date.into();
        validate_business_date(&business_date)?;
        let policy = compiled_policy_catalog()
            .into_iter()
            .find(|row| row.push_kind == push_kind && row.sub_kind == sub_kind)
            .ok_or_else(|| {
                DurableDeliveryError::PolicyMismatch(format!(
                    "no registered policy for {push_kind}/{sub_kind}"
                ))
            })?;
        let scope_key = scope_key.into();
        validate_scope_key(policy.cooldown_scope, &scope_key)?;
        let schedule_occurrence_identity = schedule_occurrence_identity.into();
        let source_evidence_fingerprint = source_evidence_fingerprint.into();
        let delivery_subject_hash = delivery_subject_hash.into();
        if schedule_occurrence_identity.trim().is_empty()
            || source_evidence_fingerprint.trim().is_empty()
            || source_binding_canonical.is_empty()
            || delivery_subject_hash.trim().is_empty()
            || rendered_content.is_empty()
        {
            return Err(DurableDeliveryError::InvalidEnvelope(
                "occurrence, evidence, source binding, subject and rendered content must be non-empty"
                    .to_owned(),
            ));
        }
        let source_binding_sha256 = sha256_hex(&source_binding_canonical);
        let rendered_content_sha256 = sha256_hex(&rendered_content);
        let material = DecisionIdentityMaterial {
            domain: "durable-delivery-decision-v1",
            policy_version: policy.policy_version,
            business_date: &business_date,
            push_kind,
            sub_kind,
            cooldown_scope: policy.cooldown_scope,
            scope_key: &scope_key,
            schedule_occurrence_identity: &schedule_occurrence_identity,
            source_evidence_fingerprint: &source_evidence_fingerprint,
            delivery_subject_hash: &delivery_subject_hash,
            rendered_content_sha256: &rendered_content_sha256,
        };
        let decision_identity = sha256_hex(&serde_json::to_vec(&material)?);
        Ok(Self {
            envelope_version: ENVELOPE_VERSION,
            decision_identity,
            business_date,
            push_kind,
            sub_kind,
            cooldown_scope: policy.cooldown_scope,
            scope_key,
            schedule_occurrence_identity,
            source_evidence_fingerprint,
            source_binding_canonical,
            source_binding_sha256,
            delivery_subject_hash,
            rendered_content,
            rendered_content_sha256,
            policy_version: policy.policy_version,
            retry_authorized,
            provider_observed_at: None,
            provider_as_of: None,
            original_batch_ids: Vec::new(),
            task_binding,
        })
    }

    pub fn with_provider_evidence(
        mut self,
        provider_observed_at: Option<String>,
        provider_as_of: Option<String>,
        original_batch_ids: Vec<String>,
    ) -> Result<Self> {
        self.provider_observed_at = provider_observed_at;
        self.provider_as_of = provider_as_of;
        self.original_batch_ids = original_batch_ids;
        self.validate()?;
        Ok(self)
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>> {
        self.validate()?;
        Ok(serde_json::to_vec(self)?)
    }

    pub fn canonical_sha256(&self) -> Result<String> {
        Ok(sha256_hex(&self.canonical_bytes()?))
    }

    pub fn validate(&self) -> Result<()> {
        validate_business_date(&self.business_date)?;
        validate_scope_key(self.cooldown_scope, &self.scope_key)?;
        if self.envelope_version != ENVELOPE_VERSION {
            return Err(DurableDeliveryError::InvalidEnvelope(format!(
                "unsupported envelope version {}",
                self.envelope_version
            )));
        }
        if self.rendered_content.is_empty()
            || sha256_hex(&self.rendered_content) != self.rendered_content_sha256
        {
            return Err(DurableDeliveryError::InvalidEnvelope(
                "rendered content hash mismatch".to_owned(),
            ));
        }
        if self.source_binding_canonical.is_empty()
            || sha256_hex(&self.source_binding_canonical) != self.source_binding_sha256
        {
            return Err(DurableDeliveryError::InvalidEnvelope(
                "source binding hash mismatch".to_owned(),
            ));
        }
        if self
            .provider_observed_at
            .as_ref()
            .is_some_and(|value| value.trim().is_empty())
            || self
                .provider_as_of
                .as_ref()
                .is_some_and(|value| value.trim().is_empty())
            || self
                .original_batch_ids
                .iter()
                .any(|value| value.trim().is_empty())
        {
            return Err(DurableDeliveryError::InvalidEnvelope(
                "provider evidence metadata must not contain empty values".to_owned(),
            ));
        }
        if let Some(binding) = &self.task_binding {
            if binding.transition_basis_canonical.is_empty()
                || sha256_hex(&binding.transition_basis_canonical)
                    != binding.transition_basis_sha256
            {
                return Err(DurableDeliveryError::InvalidEnvelope(
                    "task transition basis hash mismatch".to_owned(),
                ));
            }
        }
        let material = DecisionIdentityMaterial {
            domain: "durable-delivery-decision-v1",
            policy_version: self.policy_version,
            business_date: &self.business_date,
            push_kind: self.push_kind,
            sub_kind: self.sub_kind,
            cooldown_scope: self.cooldown_scope,
            scope_key: &self.scope_key,
            schedule_occurrence_identity: &self.schedule_occurrence_identity,
            source_evidence_fingerprint: &self.source_evidence_fingerprint,
            delivery_subject_hash: &self.delivery_subject_hash,
            rendered_content_sha256: &self.rendered_content_sha256,
        };
        let expected = sha256_hex(&serde_json::to_vec(&material)?);
        if expected != self.decision_identity {
            return Err(DurableDeliveryError::InvalidEnvelope(
                "decision identity does not match canonical evidence".to_owned(),
            ));
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn replace_content_preserving_identity(&mut self, value: Vec<u8>) {
        self.rendered_content = value;
        self.rendered_content_sha256 = sha256_hex(&self.rendered_content);
    }

    #[cfg(test)]
    pub(crate) fn replace_source_binding_preserving_identity(&mut self, value: Vec<u8>) {
        self.source_binding_canonical = value;
        self.source_binding_sha256 = sha256_hex(&self.source_binding_canonical);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DecisionState {
    Reserved,
    AttemptInFlight,
    AcceptedAuditPending,
    AcceptedTaskTransitionPending,
    Delivered,
    RejectedAuditPending,
    RejectedTaskTransitionPending,
    RejectedDurable,
    UncertainAuditPending,
    UncertainTaskTransitionPending,
    UncertainManualReview,
    ManualRejectedAuditPending,
    ManualRejectedTaskTransitionPending,
    ManualResolvedRejected,
}

impl DecisionState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Reserved => "Reserved",
            Self::AttemptInFlight => "AttemptInFlight",
            Self::AcceptedAuditPending => "AcceptedAuditPending",
            Self::AcceptedTaskTransitionPending => "AcceptedTaskTransitionPending",
            Self::Delivered => "Delivered",
            Self::RejectedAuditPending => "RejectedAuditPending",
            Self::RejectedTaskTransitionPending => "RejectedTaskTransitionPending",
            Self::RejectedDurable => "RejectedDurable",
            Self::UncertainAuditPending => "UncertainAuditPending",
            Self::UncertainTaskTransitionPending => "UncertainTaskTransitionPending",
            Self::UncertainManualReview => "UncertainManualReview",
            Self::ManualRejectedAuditPending => "ManualRejectedAuditPending",
            Self::ManualRejectedTaskTransitionPending => "ManualRejectedTaskTransitionPending",
            Self::ManualResolvedRejected => "ManualResolvedRejected",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self> {
        use DecisionState::*;
        match value {
            "Reserved" => Ok(Reserved),
            "AttemptInFlight" => Ok(AttemptInFlight),
            "AcceptedAuditPending" => Ok(AcceptedAuditPending),
            "AcceptedTaskTransitionPending" => Ok(AcceptedTaskTransitionPending),
            "Delivered" => Ok(Delivered),
            "RejectedAuditPending" => Ok(RejectedAuditPending),
            "RejectedTaskTransitionPending" => Ok(RejectedTaskTransitionPending),
            "RejectedDurable" => Ok(RejectedDurable),
            "UncertainAuditPending" => Ok(UncertainAuditPending),
            "UncertainTaskTransitionPending" => Ok(UncertainTaskTransitionPending),
            "UncertainManualReview" => Ok(UncertainManualReview),
            "ManualRejectedAuditPending" => Ok(ManualRejectedAuditPending),
            "ManualRejectedTaskTransitionPending" => Ok(ManualRejectedTaskTransitionPending),
            "ManualResolvedRejected" => Ok(ManualResolvedRejected),
            other => Err(DurableDeliveryError::InvalidEnvelope(format!(
                "unknown decision state {other}"
            ))),
        }
    }
}

impl fmt::Display for DecisionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TypedReceipt {
    pub channel: String,
    pub provider: String,
    pub message_id: String,
    pub platform_message_id: Option<String>,
    pub accepted_at: DateTime<Utc>,
    pub latency_ms: Option<i64>,
}

impl TypedReceipt {
    pub fn validate(&self) -> Result<()> {
        if self.channel.trim().is_empty()
            || self.provider.trim().is_empty()
            || self.message_id.trim().is_empty()
            || self
                .platform_message_id
                .as_deref()
                .is_some_and(|value| value.trim().is_empty())
            || self.latency_ms.is_some_and(|value| value < 0)
        {
            return Err(DurableDeliveryError::InvalidEnvelope(
                "accepted receipt requires non-blank identities and non-negative latency"
                    .to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeliveryDispositionCanonical {
    pub(crate) schema_version: i64,
    pub(crate) disposition_identity: String,
    pub(crate) decision_identity: String,
    pub(crate) envelope_sha256: String,
    pub(crate) attempt_identity: Option<String>,
    pub(crate) denial_identity: Option<String>,
    pub(crate) resolution_identity: Option<String>,
    pub(crate) disposition: String,
    pub(crate) evidence_sha256: String,
    pub(crate) retry_authorized: bool,
    pub(crate) manual_action_required: bool,
    pub(crate) created_at: String,
}

impl DeliveryDispositionCanonical {
    pub(crate) const FIELDS: [&'static str; 12] = [
        "schema_version",
        "disposition_identity",
        "decision_identity",
        "envelope_sha256",
        "attempt_identity",
        "denial_identity",
        "resolution_identity",
        "disposition",
        "evidence_sha256",
        "retry_authorized",
        "manual_action_required",
        "created_at",
    ];

    pub(crate) fn parse_exact(canonical: &[u8], role: &str) -> Result<Self> {
        parse_exact_canonical_object(canonical, role, &Self::FIELDS)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TaskTransitionCanonical {
    pub(crate) schema_version: i64,
    pub(crate) transition_identity: String,
    pub(crate) task_identity: String,
    pub(crate) decision_identity: String,
    pub(crate) source_identity: String,
    pub(crate) task_disposition: String,
    pub(crate) task_binding_sha256: String,
    pub(crate) generic_disposition_identity: String,
    pub(crate) generic_disposition_sha256: String,
}

impl TaskTransitionCanonical {
    pub(crate) const FIELDS: [&'static str; 9] = [
        "schema_version",
        "transition_identity",
        "task_identity",
        "decision_identity",
        "source_identity",
        "task_disposition",
        "task_binding_sha256",
        "generic_disposition_identity",
        "generic_disposition_sha256",
    ];

    pub(crate) fn parse_exact(canonical: &[u8]) -> Result<Self> {
        parse_exact_canonical_object(canonical, "task transition", &Self::FIELDS)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AcceptedSinkResultCanonical {
    pub(crate) kind: String,
    pub(crate) receipt: TypedReceipt,
}

impl AcceptedSinkResultCanonical {
    pub(crate) fn parse_exact(canonical: &[u8]) -> Result<Self> {
        let value = parse_exact_canonical_value(canonical, "authoritative accepted sink result")?;
        require_exact_object_fields(
            &value,
            "authoritative accepted sink result",
            &["kind", "receipt"],
        )?;
        let receipt = value.get("receipt").ok_or_else(|| {
            DurableDeliveryError::PolicyMismatch(
                "authoritative accepted sink result receipt is missing".to_owned(),
            )
        })?;
        require_exact_object_fields(
            receipt,
            "authoritative accepted receipt",
            &[
                "channel",
                "provider",
                "message_id",
                "platform_message_id",
                "accepted_at",
                "latency_ms",
            ],
        )?;
        let payload: Self = serde_json::from_value(value).map_err(|error| {
            DurableDeliveryError::PolicyMismatch(format!(
                "authoritative accepted sink result canonical payload is invalid: {error}"
            ))
        })?;
        validate_typed_canonical_reencode(
            canonical,
            "authoritative accepted sink result",
            &payload,
        )?;
        Ok(payload)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ManualResolutionAuthorizationCanonical {
    pub(crate) resolution_identity: String,
    pub(crate) decision_identity: String,
    pub(crate) disposition: String,
    pub(crate) operator_identity: String,
    pub(crate) reason: String,
    pub(crate) evidence_sha256: String,
    pub(crate) resolved_at: String,
}

impl ManualResolutionAuthorizationCanonical {
    pub(crate) fn canonical_bytes(&self) -> Result<Vec<u8>> {
        let value = serde_json::to_value(self)?;
        Ok(serde_json::to_vec(&value)?)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManualAcceptedDeliveryAuditCanonical {
    acceptance_evidence_sha256: String,
    attempt_identity: String,
    authorization_ref: String,
    authorization_sha256: String,
    decision_identity: String,
    operator_identity: String,
    reason: String,
    receipt_sha256: Option<String>,
    resolution_identity: String,
    resolved_at: String,
}

impl ManualAcceptedDeliveryAuditCanonical {
    const FIELDS: [&'static str; 10] = [
        "acceptance_evidence_sha256",
        "attempt_identity",
        "authorization_ref",
        "authorization_sha256",
        "decision_identity",
        "operator_identity",
        "reason",
        "receipt_sha256",
        "resolution_identity",
        "resolved_at",
    ];

    fn parse_exact(canonical: &[u8]) -> Result<Self> {
        parse_exact_canonical_object(canonical, "manual accepted delivery audit", &Self::FIELDS)
    }
}

fn parse_exact_canonical_object<T: DeserializeOwned + Serialize>(
    canonical: &[u8],
    role: &str,
    expected_fields: &[&str],
) -> Result<T> {
    let value = parse_exact_canonical_value(canonical, role)?;
    require_exact_object_fields(&value, role, expected_fields)?;
    let payload: T = serde_json::from_value(value).map_err(|error| {
        DurableDeliveryError::PolicyMismatch(format!(
            "{role} canonical payload is invalid: {error}"
        ))
    })?;
    validate_typed_canonical_reencode(canonical, role, &payload)?;
    Ok(payload)
}

fn parse_exact_canonical_value(canonical: &[u8], role: &str) -> Result<serde_json::Value> {
    let value: serde_json::Value = serde_json::from_slice(canonical).map_err(|error| {
        DurableDeliveryError::PolicyMismatch(format!("{role} canonical JSON is invalid: {error}"))
    })?;
    let reencoded = serde_json::to_vec(&value).map_err(|error| {
        DurableDeliveryError::PolicyMismatch(format!(
            "{role} canonical JSON cannot be re-encoded: {error}"
        ))
    })?;
    if reencoded != canonical {
        return Err(DurableDeliveryError::PolicyMismatch(format!(
            "{role} bytes are not canonical JSON"
        )));
    }
    Ok(value)
}

fn validate_typed_canonical_reencode<T: Serialize>(
    canonical: &[u8],
    role: &str,
    payload: &T,
) -> Result<()> {
    let typed_value = serde_json::to_value(payload).map_err(|error| {
        DurableDeliveryError::PolicyMismatch(format!(
            "{role} typed payload cannot be re-encoded: {error}"
        ))
    })?;
    let typed_reencoded = serde_json::to_vec(&typed_value).map_err(|error| {
        DurableDeliveryError::PolicyMismatch(format!(
            "{role} typed payload cannot be canonicalized: {error}"
        ))
    })?;
    if typed_reencoded != canonical {
        return Err(DurableDeliveryError::PolicyMismatch(format!(
            "{role} typed payload does not round-trip to the exact canonical bytes"
        )));
    }
    Ok(())
}

fn require_exact_object_fields(
    value: &serde_json::Value,
    role: &str,
    expected_fields: &[&str],
) -> Result<()> {
    let object = value.as_object().ok_or_else(|| {
        DurableDeliveryError::PolicyMismatch(format!("{role} canonical payload is not an object"))
    })?;
    if object.len() != expected_fields.len()
        || expected_fields
            .iter()
            .any(|field| !object.contains_key(*field))
    {
        return Err(DurableDeliveryError::PolicyMismatch(format!(
            "{role} canonical payload has an invalid field set"
        )));
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TypedRejection {
    pub reason_code: String,
    pub evidence: Vec<u8>,
    pub retry_authorized: bool,
    pub observed_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TypedUncertainty {
    pub reason_code: String,
    pub evidence: Vec<u8>,
    pub observed_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AuthoritativeSinkResult {
    Accepted(TypedReceipt),
    Rejected(TypedRejection),
    Uncertain(TypedUncertainty),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthoritativeDeliveryRequest {
    pub decision_identity: String,
    pub attempt_identity: String,
    pub fence_token: i64,
    pub push_kind: PushKind,
    pub stable_template_id: String,
    pub rendered_content: Vec<u8>,
    pub rendered_content_sha256: String,
}

pub trait AuthoritativeSinkPort: Send + Sync {
    fn sink_identity(&self) -> &str;
    fn deliver(&self, request: &AuthoritativeDeliveryRequest) -> AuthoritativeSinkResult;
}

pub type AuthoritativeSink = Arc<dyn AuthoritativeSinkPort>;

pub trait ImmutableAppendPort: Send + Sync {
    fn append_exact(
        &self,
        record_kind: &str,
        identity: &str,
        canonical_bytes: &[u8],
        sha256: &str,
    ) -> Result<String>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScheduleHydrationState {
    Pending,
    Applied,
}

impl ScheduleHydrationState {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::Applied => "Applied",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self> {
        match value {
            "Pending" => Ok(Self::Pending),
            "Applied" => Ok(Self::Applied),
            other => Err(DurableDeliveryError::InvalidEnvelope(format!(
                "unknown schedule hydration state {other}"
            ))),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduleHydration {
    pub decision_identity: String,
    pub task_identity: String,
    pub transition_identity: String,
    pub transition_canonical: Vec<u8>,
    pub transition_sha256: String,
    pub transition_basis_canonical: Vec<u8>,
    pub transition_basis_sha256: String,
    pub immutable_audit_ref: String,
    pub hydration_state: ScheduleHydrationState,
}

/// Read-only evidence for an already-owned review-task occurrence.
///
/// BR-200 consumers use this before provider acquisition. Returning this type
/// never reserves budget/cooldown, creates a decision, or calls a sink.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewTaskOccurrenceEvidence {
    pub decision_identity: String,
    pub state: DecisionState,
    pub schedule_hydration: Option<ScheduleHydration>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrepareOutcome {
    pub decision_identity: String,
    pub state: DecisionState,
    pub sink_calls: usize,
    pub reservation_generation: i64,
    pub budget_reservation_identity: Option<String>,
    pub cooldown_reservation_identity: Option<String>,
    pub schedule_hydration: Option<ScheduleHydration>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResumeOutcome {
    pub decision_identity: String,
    pub state: DecisionState,
    pub sink_calls: usize,
    pub persisted_receipt: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReconcileSummary {
    pub provider_calls: usize,
    pub sink_calls: usize,
    pub progress_count: usize,
    pub locally_pending_decisions: Vec<String>,
    pub deliverable_decisions: Vec<String>,
    pub non_progressable_foreign_attempts: Vec<String>,
    pub non_progressable_manual_reviews: Vec<String>,
    pub schedule_hydrations: Vec<ScheduleHydration>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorityWatermark {
    pub count: i64,
    pub ordered_identity_set_sha256: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ReviewTerminalReplayCompletionState {
    Passed,
    Failed,
}

impl ReviewTerminalReplayCompletionState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "Passed",
            Self::Failed => "Failed",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewTerminalReplayStartCanonical {
    pub schema_version: i64,
    pub attempt_identity: String,
    pub business_date: String,
    pub review_task: String,
    pub task_identity: String,
    pub decision_identity: String,
    pub replay_ordinal: i64,
    pub started_at: String,
    pub pre_sink_watermark: AuthorityWatermark,
    pub pre_delivery_audit_watermark: AuthorityWatermark,
    pub provider_calls: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewTerminalReplayCompletionCanonical {
    pub schema_version: i64,
    pub attempt_identity: String,
    pub decision_identity: String,
    pub state: ReviewTerminalReplayCompletionState,
    pub completed_at: String,
    pub post_sink_watermark: AuthorityWatermark,
    pub post_delivery_audit_watermark: AuthorityWatermark,
    pub provider_calls: i64,
    pub resume_calls: i64,
    pub sink_calls: i64,
    pub delivery_audit_appends: i64,
    pub reason_code: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewTerminalReplayInput {
    pub business_date: String,
    pub review_task: String,
    pub task_identity: String,
    pub decision_identity: String,
    pub envelope: DeliveryEnvelope,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewTerminalReplayAttempt {
    pub attempt_identity: String,
    pub decision_identity: String,
    pub replay_ordinal: i64,
    pub start_audit_identity: String,
    pub pre_sink_watermark: AuthorityWatermark,
    pub pre_delivery_audit_watermark: AuthorityWatermark,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewTerminalReplayCompletion {
    pub attempt_identity: String,
    pub decision_identity: String,
    pub state: ReviewTerminalReplayCompletionState,
    pub completion_audit_identity: String,
    pub post_sink_watermark: AuthorityWatermark,
    pub post_delivery_audit_watermark: AuthorityWatermark,
    pub reason_code: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ManualDisposition {
    Accepted { receipt: Option<TypedReceipt> },
    Rejected,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManualResolutionCommand {
    pub decision_identity: String,
    pub disposition: ManualDisposition,
    pub operator_identity: String,
    pub reason: String,
    pub external_evidence: Vec<u8>,
    pub resolved_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ManualAcceptedDeliveryAuditEvidence {
    pub(crate) decision_identity: String,
    pub(crate) resolution_identity: String,
    pub(crate) attempt_identity: String,
    pub(crate) decision_current_attempt_identity: String,
    pub(crate) attempt_state: String,
    pub(crate) operator_identity: String,
    pub(crate) reason: String,
    pub(crate) authorization_immutable_audit_ref: String,
    pub(crate) envelope_sha256: String,
    pub(crate) current_disposition_identity: String,
    pub(crate) disposition_identity: String,
    pub(crate) disposition_canonical: Vec<u8>,
    pub(crate) disposition_sha256: String,
    pub(crate) disposition_append_state: String,
    pub(crate) disposition_immutable_audit_ref: Option<String>,
    pub(crate) acceptance_evidence_canonical: Vec<u8>,
    pub(crate) acceptance_evidence_sha256: String,
    pub(crate) receipt_canonical: Option<Vec<u8>>,
    pub(crate) resolved_at: String,
    pub(crate) audit_identity: String,
    pub(crate) canonical: Vec<u8>,
    pub(crate) sha256: String,
    pub(crate) append_state: String,
    pub(crate) accepted_audit_immutable_ref: Option<String>,
}

impl ManualAcceptedDeliveryAuditEvidence {
    pub(crate) fn authorization_canonical(&self) -> Result<Vec<u8>> {
        ManualResolutionAuthorizationCanonical {
            resolution_identity: self.resolution_identity.clone(),
            decision_identity: self.decision_identity.clone(),
            disposition: "Accepted".to_owned(),
            operator_identity: self.operator_identity.clone(),
            reason: self.reason.clone(),
            evidence_sha256: self.acceptance_evidence_sha256.clone(),
            resolved_at: self.resolved_at.clone(),
        }
        .canonical_bytes()
    }

    pub(crate) fn validate(&self) -> Result<()> {
        self.validate_semantic_binding()?;
        if self.disposition_append_state != "Appended"
            || self
                .disposition_immutable_audit_ref
                .as_deref()
                .is_none_or(|value| !has_non_ascii_whitespace(value))
        {
            return Err(DurableDeliveryError::PolicyMismatch(
                "manual accepted current disposition is not durably acknowledged".to_owned(),
            ));
        }
        if self.append_state != "Appended"
            || self
                .accepted_audit_immutable_ref
                .as_deref()
                .is_none_or(|value| !has_non_ascii_whitespace(value))
        {
            return Err(DurableDeliveryError::PolicyMismatch(
                "manual accepted delivery audit is not durably acknowledged".to_owned(),
            ));
        }
        Ok(())
    }

    pub(crate) fn validate_for_migration(&self) -> Result<()> {
        self.validate_semantic_binding()?;
        validate_append_state_and_ref(
            "manual accepted current disposition",
            &self.disposition_append_state,
            self.disposition_immutable_audit_ref.as_deref(),
        )?;
        validate_append_state_and_ref(
            "manual accepted delivery audit",
            &self.append_state,
            self.accepted_audit_immutable_ref.as_deref(),
        )
    }

    fn validate_semantic_binding(&self) -> Result<()> {
        if self.decision_identity.trim().is_empty()
            || self.resolution_identity.trim().is_empty()
            || self.attempt_identity.trim().is_empty()
            || self.decision_current_attempt_identity.trim().is_empty()
            || self.operator_identity.trim().is_empty()
            || self.reason.trim().is_empty()
            || !has_non_ascii_whitespace(&self.authorization_immutable_audit_ref)
            || self.envelope_sha256.trim().is_empty()
            || self.current_disposition_identity.trim().is_empty()
            || self.disposition_identity.trim().is_empty()
            || self.disposition_canonical.is_empty()
            || self.disposition_sha256.trim().is_empty()
            || self.acceptance_evidence_canonical.is_empty()
            || self.acceptance_evidence_sha256.trim().is_empty()
            || self.resolved_at.trim().is_empty()
            || self.audit_identity.trim().is_empty()
            || self.canonical.is_empty()
            || self.sha256.trim().is_empty()
        {
            return Err(DurableDeliveryError::PolicyMismatch(
                "manual accepted delivery audit evidence is incomplete".to_owned(),
            ));
        }
        if self.attempt_identity != self.decision_current_attempt_identity
            || self.attempt_state != "Uncertain"
        {
            return Err(DurableDeliveryError::PolicyMismatch(
                "manual accepted resolution is not bound to the decision's original uncertain attempt"
                    .to_owned(),
            ));
        }
        if self.current_disposition_identity != self.disposition_identity {
            return Err(DurableDeliveryError::PolicyMismatch(
                "manual accepted disposition is not the decision's current disposition".to_owned(),
            ));
        }
        if sha256_hex(&self.acceptance_evidence_canonical) != self.acceptance_evidence_sha256 {
            return Err(DurableDeliveryError::PolicyMismatch(
                "manual accepted external evidence hash mismatch".to_owned(),
            ));
        }
        let expected_resolution_identity = stable_identity(
            "delivery-manual-resolution-v1",
            &[
                &self.decision_identity,
                "Accepted",
                &self.operator_identity,
                &self.acceptance_evidence_sha256,
            ],
        );
        if self.resolution_identity != expected_resolution_identity {
            return Err(DurableDeliveryError::PolicyMismatch(
                "manual accepted resolution identity binding mismatch".to_owned(),
            ));
        }
        let authorization_canonical = self.authorization_canonical()?;
        let authorization_sha256 = sha256_hex(&authorization_canonical);
        if let Some(receipt_canonical) = &self.receipt_canonical {
            let receipt: TypedReceipt =
                serde_json::from_slice(receipt_canonical).map_err(|error| {
                    DurableDeliveryError::PolicyMismatch(format!(
                        "manual accepted receipt canonical payload is invalid: {error}"
                    ))
                })?;
            let reencoded = serde_json::to_vec(&receipt).map_err(|error| {
                DurableDeliveryError::PolicyMismatch(format!(
                    "manual accepted receipt cannot be re-encoded: {error}"
                ))
            })?;
            if reencoded.as_slice() != receipt_canonical.as_slice() {
                return Err(DurableDeliveryError::PolicyMismatch(
                    "manual accepted receipt bytes are not exact typed canonical JSON".to_owned(),
                ));
            }
            receipt.validate().map_err(|error| {
                DurableDeliveryError::PolicyMismatch(format!(
                    "manual accepted receipt validation failed: {error}"
                ))
            })?;
        }
        if sha256_hex(&self.disposition_canonical) != self.disposition_sha256 {
            return Err(DurableDeliveryError::PolicyMismatch(
                "manual accepted disposition canonical hash mismatch".to_owned(),
            ));
        }
        let disposition_payload = DeliveryDispositionCanonical::parse_exact(
            &self.disposition_canonical,
            "manual accepted disposition",
        )?;
        let disposition_matches = disposition_payload.schema_version == 1
            && disposition_payload.disposition_identity == self.disposition_identity
            && disposition_payload.decision_identity == self.decision_identity
            && disposition_payload.envelope_sha256 == self.envelope_sha256
            && disposition_payload.attempt_identity.is_none()
            && disposition_payload.denial_identity.is_none()
            && disposition_payload.resolution_identity.as_deref()
                == Some(self.resolution_identity.as_str())
            && disposition_payload.disposition == "ManualAccepted"
            && disposition_payload.evidence_sha256 == self.acceptance_evidence_sha256
            && !disposition_payload.retry_authorized
            && !disposition_payload.manual_action_required
            && disposition_payload.created_at == self.resolved_at;
        if !disposition_matches {
            return Err(DurableDeliveryError::PolicyMismatch(
                "manual accepted disposition canonical binding mismatch".to_owned(),
            ));
        }
        if sha256_hex(&self.canonical) != self.sha256 {
            return Err(DurableDeliveryError::PolicyMismatch(
                "manual accepted delivery audit canonical hash mismatch".to_owned(),
            ));
        }
        let payload = ManualAcceptedDeliveryAuditCanonical::parse_exact(&self.canonical)?;
        if !is_sha256_hex(&payload.acceptance_evidence_sha256)
            || !is_sha256_hex(&payload.authorization_sha256)
        {
            return Err(DurableDeliveryError::PolicyMismatch(
                "manual accepted delivery audit evidence hashes are invalid".to_owned(),
            ));
        }
        let expected_receipt_sha256 = self
            .receipt_canonical
            .as_ref()
            .map(|receipt| sha256_hex(receipt));
        if payload
            .receipt_sha256
            .as_deref()
            .is_some_and(|value| !is_sha256_hex(value))
            || payload.receipt_sha256 != expected_receipt_sha256
        {
            return Err(DurableDeliveryError::PolicyMismatch(
                "manual accepted delivery audit receipt binding mismatch".to_owned(),
            ));
        }
        DateTime::parse_from_rfc3339(&payload.resolved_at).map_err(|error| {
            DurableDeliveryError::PolicyMismatch(format!(
                "manual accepted delivery audit resolved_at is invalid: {error}"
            ))
        })?;
        let exact_binding = payload.acceptance_evidence_sha256 == self.acceptance_evidence_sha256
            && payload.attempt_identity == self.attempt_identity
            && payload.authorization_ref == self.authorization_immutable_audit_ref
            && payload.authorization_sha256 == authorization_sha256
            && payload.decision_identity == self.decision_identity
            && payload.operator_identity == self.operator_identity
            && payload.reason == self.reason
            && payload.resolution_identity == self.resolution_identity
            && payload.resolved_at == self.resolved_at;
        if !exact_binding {
            return Err(DurableDeliveryError::PolicyMismatch(
                "manual accepted delivery audit exact semantic binding mismatch".to_owned(),
            ));
        }
        let expected_identity = stable_identity(
            MANUAL_ACCEPTED_DELIVERY_AUDIT_DOMAIN,
            &[&self.resolution_identity, &self.sha256],
        );
        if self.audit_identity != expected_identity {
            return Err(DurableDeliveryError::PolicyMismatch(
                "manual accepted delivery audit identity mismatch".to_owned(),
            ));
        }
        Ok(())
    }
}

fn validate_append_state_and_ref(
    role: &str,
    append_state: &str,
    immutable_ref: Option<&str>,
) -> Result<()> {
    match (append_state, immutable_ref) {
        ("Pending", None) => Ok(()),
        ("Appended", Some(value)) if has_non_ascii_whitespace(value) => Ok(()),
        _ => Err(DurableDeliveryError::PolicyMismatch(format!(
            "{role} append state/reference binding is invalid"
        ))),
    }
}

pub(crate) fn has_non_ascii_whitespace(value: &str) -> bool {
    value
        .as_bytes()
        .iter()
        .any(|byte| !matches!(byte, b' ' | b'\t' | b'\n' | b'\r'))
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(crate) fn sha256_hex(value: &[u8]) -> String {
    hex::encode(Sha256::digest(value))
}

pub(crate) fn stable_identity(domain: &str, parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    hasher.update((domain.len() as u64).to_be_bytes());
    hasher.update(domain.as_bytes());
    for part in parts {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    hex::encode(hasher.finalize())
}

pub(crate) fn validate_business_date(value: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|error| {
        DurableDeliveryError::InvalidEnvelope(format!("business_date must be YYYY-MM-DD: {error}"))
    })
}

fn validate_scope_key(scope: CooldownScope, scope_key: &str) -> Result<()> {
    match scope {
        CooldownScope::Global if scope_key == "GLOBAL" => Ok(()),
        CooldownScope::Global => Err(DurableDeliveryError::InvalidEnvelope(
            "Global cooldown scope requires scope_key=GLOBAL".to_owned(),
        )),
        CooldownScope::PerTicket if !scope_key.trim().is_empty() && scope_key != "GLOBAL" => Ok(()),
        CooldownScope::PerTicket => Err(DurableDeliveryError::InvalidEnvelope(
            "PerTicket cooldown scope requires a canonical typed instrument key".to_owned(),
        )),
    }
}
