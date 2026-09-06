//! W05 deterministic semantic projection contracts. Runtime catalog wiring starts in W06.

use std::collections::BTreeMap;

use super::canonical::{canonical_digest, canonical_preimage, CanonicalValue};
use super::facts::{model_output_ref_value, source_ref_value};
use super::identity::{subject_value, validate_text};
use super::{
    AudienceId, CompletionOwnerId, CompletionPolicyId, CompletionPolicyVersion, ExactBytes,
    Namespace, OccurrenceId, PreparedFacts, PreparedFactsSnapshot, PushJobError, ReasonCode,
    Result, RunContext, Sha256Digest, SubjectId, TemplateId, TemplateVersion, UnitId, UtcMicros,
};

macro_rules! monitor_kinds {
    ($($variant:ident),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
        pub enum MonitorKind {
            $($variant),+
        }

        impl MonitorKind {
            pub const ALL: [Self; 65] = [$(Self::$variant),+];

            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => stringify!($variant)),+
                }
            }
        }

        impl TryFrom<&str> for MonitorKind {
            type Error = PushJobError;

            fn try_from(value: &str) -> Result<Self> {
                match value {
                    $(stringify!($variant) => Ok(Self::$variant)),+,
                    other => Err(PushJobError::InvalidMonitorKind(other.to_owned())),
                }
            }
        }
    };
}

monitor_kinds! {
    HoldingEvent,
    DailyReport,
    Announcement,
    AuctionVolume,
    VirtualWatch,
    LimitBoards,
    SectorTop,
    FundInflow,
    AuctionRepush,
    FactorIC,
    SectorTier,
    CapitalVerify,
    WeeklySOP,
    StockPick,
    IndustryChain,
    TurnoverTop,
    CandidateBoard,
    NewsRanked,
    AccountMode,
    DataMode,
    HoldingPlan,
    T0Advice,
    CandidateTriggered,
    ForbiddenOps,
    PaperTrade,
    PaperSell,
    SnapshotStale,
    AttributionDaily,
    G5bAttribution,
    CloseCall,
    ReviewMarket,
    ReviewLhb,
    ReviewSignal,
    ReviewFailure,
    TomorrowWatch,
    EventCalendar,
    ReviewProviderTopN,
    PositionReview,
    ReviewBacktest,
    WatchlistTracking,
    PreopenNewsHot,
    IntradayMarket,
    NewsCatalyst,
    SectorAnomaly,
    NewsToIdea,
    CatalystReview,
    IndustryChainIntraday,
    PostFixedPriceOrder,
    PostFixedPriceFill,
    StPriceLimitChanged,
    EtfClosingCallAuction,
    BlockTradeIntradayConfirm,
    BlockTradePriceRange,
    PaperReview,
    CandidateInvalidated,
    IpoListingApproval,
    IpoProspectus,
    IpoCatalyst,
    PolicyHit,
    EarningsBeat,
    EarningsMiss,
    AnalystUpgrade,
    MarketActionAlert,
    NewsFlashCritical,
    NewsFlashAggregated,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct SubKindValue(String);

impl SubKindValue {
    fn try_new(value: String) -> Result<Self> {
        validate_text("sub_kind", value).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub enum SubKind {
    None,
    Registered(SubKindValue),
}

impl SubKind {
    pub const fn none() -> Self {
        Self::None
    }

    pub fn try_registered(value: String) -> Result<Self> {
        SubKindValue::try_new(value).map(Self::Registered)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub enum Severity {
    Emergency,
    Important,
    Info,
    Research,
}

impl Severity {
    pub const ALL: [Self; 4] = [Self::Emergency, Self::Important, Self::Info, Self::Research];

    const fn as_str(self) -> &'static str {
        match self {
            Self::Emergency => "Emergency",
            Self::Important => "Important",
            Self::Info => "Info",
            Self::Research => "Research",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Suppression {
    Eligible,
    Suppressed {
        reason: ReasonCode,
        eligible_after: Option<UtcMicros>,
    },
}

impl Suppression {
    pub const fn eligible() -> Self {
        Self::Eligible
    }

    pub const fn suppressed(reason: ReasonCode, eligible_after: Option<UtcMicros>) -> Self {
        Self::Suppressed {
            reason,
            eligible_after,
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct SemanticInput {
    business_subject: SubjectId,
    severity: Severity,
    suppression: Suppression,
}

impl SemanticInput {
    pub fn new(business_subject: SubjectId, severity: Severity, suppression: Suppression) -> Self {
        Self {
            business_subject,
            severity,
            suppression,
        }
    }
}

// W06 machine-catalog registration is the first non-test constructor.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug)]
pub(super) struct ProjectionBinding {
    unit_id: UnitId,
    audience: AudienceId,
    monitor_kind: Option<MonitorKind>,
    sub_kind: SubKind,
    completion_owner: CompletionOwnerId,
    completion_policy_id: CompletionPolicyId,
    completion_policy_version: CompletionPolicyVersion,
    template_id: TemplateId,
}

impl ProjectionBinding {
    #[cfg_attr(not(test), allow(dead_code))]
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        unit_id: UnitId,
        audience: AudienceId,
        monitor_kind: Option<MonitorKind>,
        sub_kind: SubKind,
        completion_owner: CompletionOwnerId,
        completion_policy_id: CompletionPolicyId,
        completion_policy_version: CompletionPolicyVersion,
        template_id: TemplateId,
    ) -> Self {
        Self {
            unit_id,
            audience,
            monitor_kind,
            sub_kind,
            completion_owner,
            completion_policy_id,
            completion_policy_version,
            template_id,
        }
    }
}

/// Projection failures preserve binding failures without exposing fact or rendered bytes.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ProjectionError {
    #[error("projection binding unit does not match run context")]
    UnitBindingMismatch,
    #[error("prepared facts do not belong to the projector run context")]
    ContextFactsMismatch,
}

/// A catalog-bound pure projector. It has no provider, model, clock, storage, or sink capability.
#[derive(Debug)]
pub struct DecisionProjector {
    namespace: Namespace,
    unit_id: UnitId,
    occurrence: OccurrenceId,
    run_context_sha256: Sha256Digest,
    template_version: TemplateVersion,
    binding: ProjectionBinding,
}

impl DecisionProjector {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn try_new(
        context: &RunContext,
        binding: ProjectionBinding,
    ) -> std::result::Result<Self, ProjectionError> {
        if &binding.unit_id != context.unit_id() {
            return Err(ProjectionError::UnitBindingMismatch);
        }
        Ok(Self {
            namespace: context.namespace().clone(),
            unit_id: context.unit_id().clone(),
            occurrence: context.occurrence().clone(),
            run_context_sha256: context.canonical_sha256(),
            template_version: context.template_version().clone(),
            binding,
        })
    }

    pub fn occurrence(&self) -> &OccurrenceId {
        &self.occurrence
    }

    pub fn project_semantics(
        &self,
        facts: &PreparedFactsSnapshot,
        input: SemanticInput,
    ) -> std::result::Result<SemanticProjection, ProjectionError> {
        if facts.facts().run_context_sha256() != &self.run_context_sha256 {
            return Err(ProjectionError::ContextFactsMismatch);
        }
        Ok(SemanticProjection::new(self, facts.facts(), input))
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct SemanticProjection {
    audience: AudienceId,
    monitor_kind: Option<MonitorKind>,
    sub_kind: SubKind,
    occurrence: OccurrenceId,
    business_subject: SubjectId,
    severity: Severity,
    suppression: Suppression,
    completion_policy_id: CompletionPolicyId,
    completion_policy_version: CompletionPolicyVersion,
    evidence_fingerprint: Sha256Digest,
    template_id: TemplateId,
    template_version: TemplateVersion,
    canonical_bytes: ExactBytes,
    sha256: Sha256Digest,
}

impl SemanticProjection {
    fn new(projector: &DecisionProjector, facts: &PreparedFacts, input: SemanticInput) -> Self {
        let core = SemanticProjectionCore {
            audience: projector.binding.audience.clone(),
            monitor_kind: projector.binding.monitor_kind,
            sub_kind: projector.binding.sub_kind.clone(),
            occurrence: projector.occurrence.clone(),
            business_subject: input.business_subject,
            severity: input.severity,
            suppression: input.suppression,
            completion_policy_id: projector.binding.completion_policy_id.clone(),
            completion_policy_version: projector.binding.completion_policy_version.clone(),
            evidence_fingerprint: evidence_fingerprint(facts),
            template_id: projector.binding.template_id.clone(),
            template_version: projector.template_version.clone(),
        };
        let canonical_bytes = ExactBytes::new(canonical_preimage(
            "SemanticProjection/v1",
            &semantic_projection_fields(&core),
        ));
        let sha256 = canonical_bytes.sha256().clone();
        Self {
            audience: core.audience,
            monitor_kind: core.monitor_kind,
            sub_kind: core.sub_kind,
            occurrence: core.occurrence,
            business_subject: core.business_subject,
            severity: core.severity,
            suppression: core.suppression,
            completion_policy_id: core.completion_policy_id,
            completion_policy_version: core.completion_policy_version,
            evidence_fingerprint: core.evidence_fingerprint,
            template_id: core.template_id,
            template_version: core.template_version,
            canonical_bytes,
            sha256,
        }
    }

    pub fn audience(&self) -> &AudienceId {
        &self.audience
    }

    pub fn monitor_kind(&self) -> Option<MonitorKind> {
        self.monitor_kind
    }

    pub fn sub_kind(&self) -> &SubKind {
        &self.sub_kind
    }

    pub fn occurrence(&self) -> &OccurrenceId {
        &self.occurrence
    }

    pub fn business_subject(&self) -> &SubjectId {
        &self.business_subject
    }

    pub fn severity(&self) -> Severity {
        self.severity
    }

    pub fn suppression(&self) -> &Suppression {
        &self.suppression
    }

    pub fn completion_policy_id(&self) -> &CompletionPolicyId {
        &self.completion_policy_id
    }

    pub fn completion_policy_version(&self) -> &CompletionPolicyVersion {
        &self.completion_policy_version
    }

    pub fn evidence_fingerprint(&self) -> &Sha256Digest {
        &self.evidence_fingerprint
    }

    pub fn template_id(&self) -> &TemplateId {
        &self.template_id
    }

    pub fn template_version(&self) -> &TemplateVersion {
        &self.template_version
    }

    pub fn canonical_bytes(&self) -> &ExactBytes {
        &self.canonical_bytes
    }

    pub fn sha256(&self) -> &Sha256Digest {
        &self.sha256
    }
}

struct SemanticProjectionCore {
    audience: AudienceId,
    monitor_kind: Option<MonitorKind>,
    sub_kind: SubKind,
    occurrence: OccurrenceId,
    business_subject: SubjectId,
    severity: Severity,
    suppression: Suppression,
    completion_policy_id: CompletionPolicyId,
    completion_policy_version: CompletionPolicyVersion,
    evidence_fingerprint: Sha256Digest,
    template_id: TemplateId,
    template_version: TemplateVersion,
}

fn evidence_fingerprint(facts: &PreparedFacts) -> Sha256Digest {
    canonical_digest(
        "EvidenceFingerprint/v1",
        &BTreeMap::from([
            (
                "model_output_refs",
                CanonicalValue::Array(
                    facts
                        .model_output_refs()
                        .iter()
                        .map(model_output_ref_value)
                        .collect(),
                ),
            ),
            (
                "source_refs",
                CanonicalValue::Array(facts.source_refs().iter().map(source_ref_value).collect()),
            ),
        ]),
    )
}

fn semantic_projection_fields(
    core: &SemanticProjectionCore,
) -> BTreeMap<&'static str, CanonicalValue> {
    BTreeMap::from([
        (
            "audience",
            CanonicalValue::String(core.audience.as_str().to_owned()),
        ),
        ("business_subject", subject_value(&core.business_subject)),
        (
            "completion_policy_id",
            CanonicalValue::String(core.completion_policy_id.as_str().to_owned()),
        ),
        (
            "completion_policy_version",
            CanonicalValue::String(core.completion_policy_version.as_str().to_owned()),
        ),
        (
            "evidence_fingerprint",
            CanonicalValue::String(core.evidence_fingerprint.as_str().to_owned()),
        ),
        (
            "monitor_kind",
            core.monitor_kind.map_or(CanonicalValue::Null, |kind| {
                CanonicalValue::String(kind.as_str().to_owned())
            }),
        ),
        (
            "occurrence",
            CanonicalValue::String(core.occurrence.as_str().to_owned()),
        ),
        (
            "severity",
            CanonicalValue::String(core.severity.as_str().to_owned()),
        ),
        ("sub_kind", sub_kind_value(&core.sub_kind)),
        ("suppression", suppression_value(&core.suppression)),
        (
            "template_id",
            CanonicalValue::String(core.template_id.as_str().to_owned()),
        ),
        (
            "template_version",
            CanonicalValue::String(core.template_version.as_str().to_owned()),
        ),
    ])
}

fn sub_kind_value(sub_kind: &SubKind) -> CanonicalValue {
    match sub_kind {
        SubKind::None => CanonicalValue::Object(BTreeMap::from([
            ("kind", CanonicalValue::String("None".to_owned())),
            ("value", CanonicalValue::Null),
        ])),
        SubKind::Registered(value) => CanonicalValue::Object(BTreeMap::from([
            ("kind", CanonicalValue::String("Registered".to_owned())),
            ("value", CanonicalValue::String(value.as_str().to_owned())),
        ])),
    }
}

fn suppression_value(suppression: &Suppression) -> CanonicalValue {
    match suppression {
        Suppression::Eligible => CanonicalValue::Object(BTreeMap::from([
            ("eligible_after", CanonicalValue::Null),
            ("kind", CanonicalValue::String("Eligible".to_owned())),
            ("reason", CanonicalValue::Null),
        ])),
        Suppression::Suppressed {
            reason,
            eligible_after,
        } => CanonicalValue::Object(BTreeMap::from([
            (
                "eligible_after",
                eligible_after.map_or(CanonicalValue::Null, |value| {
                    CanonicalValue::Unsigned(value.get() as u64)
                }),
            ),
            ("kind", CanonicalValue::String("Suppressed".to_owned())),
            ("reason", CanonicalValue::String(reason.as_str().to_owned())),
        ])),
    }
}

#[cfg(test)]
pub(super) fn projector_fixture(context: &RunContext) -> Result<DecisionProjector> {
    DecisionProjector::try_new(
        context,
        ProjectionBinding::new(
            UnitId::try_new("MU-auction".to_owned())?,
            AudienceId::try_new("portfolio-owner".to_owned())?,
            Some(MonitorKind::AuctionVolume),
            SubKind::None,
            CompletionOwnerId::try_new("owner-auction".to_owned())?,
            CompletionPolicyId::try_new("auction-notification".to_owned())?,
            CompletionPolicyVersion::try_new("policy-v1".to_owned())?,
            TemplateId::try_new("auction-card".to_owned())?,
        ),
    )
    .map_err(|error| match error {
        ProjectionError::UnitBindingMismatch => {
            PushJobError::InvalidRunContext("projection fixture unit mismatch")
        }
        ProjectionError::ContextFactsMismatch => {
            PushJobError::InvalidPreparedFacts("projection fixture context mismatch")
        }
    })
}
