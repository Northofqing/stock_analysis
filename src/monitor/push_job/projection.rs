//! W05 deterministic semantic projection contracts. Runtime catalog wiring starts in W06.

use std::collections::BTreeMap;

use super::canonical::{canonical_digest, canonical_preimage, CanonicalValue};
use super::facts::{model_output_ref_value, source_ref_value};
use super::identity::{subject_value, validate_text};
use super::{
    derive_intent_id, AudienceId, CompletionOwnerId, CompletionPolicyId, CompletionPolicyVersion,
    DecisionId, ExactBytes, IntentId, IntentIdentityMaterial, Namespace, OccurrenceId,
    PreparedFacts, PreparedFactsSnapshot, PushJobError, ReasonCode, Result, RunContext,
    Sha256Digest, SourceContractId, SourceContractVersion, SourceRef, SubjectId, TemplateId,
    TemplateVersion, UnitId, UtcMicros,
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
    #[error("verified-empty facts cannot produce a Ready decision")]
    VerifiedEmptyCannotBeReady,
    #[error("a suppressed semantic projection cannot produce a Ready decision")]
    SuppressedCannotBeReady,
    #[error("NoData requires verified-empty facts")]
    NoDataRequiresVerifiedEmpty,
    #[error("reason is not allowed for {branch}")]
    ReasonNotAllowed { branch: &'static str },
    #[error("rendered bytes are not valid UTF-8")]
    RenderedBytesNotUtf8,
    #[error("render was already attempted while preparation was {state:?}")]
    RenderAlreadyAttempted { state: RenderStateView },
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
        if let Suppression::Suppressed { reason, .. } = &input.suppression {
            if !reason.is_suppression_reason() {
                return Err(ProjectionError::ReasonNotAllowed {
                    branch: "Suppression",
                });
            }
        }
        Ok(SemanticProjection::new(self, facts.facts(), input))
    }

    pub fn prepare_ready(
        self,
        facts: PreparedFactsSnapshot,
        input: SemanticInput,
    ) -> std::result::Result<ReadyPreparation, ProjectionError> {
        ReadyPreparation::new(self, facts, input)
    }

    pub fn decide_no_data(
        self,
        facts: &PreparedFactsSnapshot,
        reason: ReasonCode,
    ) -> std::result::Result<JobDecision, ProjectionError> {
        self.verify_facts_context(facts)?;
        if !facts.facts().verified_empty() {
            return Err(ProjectionError::NoDataRequiresVerifiedEmpty);
        }
        if reason != ReasonCode::IntentNoData {
            return Err(ProjectionError::ReasonNotAllowed { branch: "NoData" });
        }
        Ok(JobDecision::new(JobDecisionKind::NoData {
            reason,
            evidence_sha256: facts.facts().canonical_sha256(),
        }))
    }

    pub fn decide_disabled(
        self,
        reason: ReasonCode,
    ) -> std::result::Result<JobDecision, ProjectionError> {
        if !matches!(
            reason,
            ReasonCode::PolicyDisabled | ReasonCode::PolicyOptInDisabled
        ) {
            return Err(ProjectionError::ReasonNotAllowed { branch: "Disabled" });
        }
        Ok(JobDecision::new(JobDecisionKind::Disabled { reason }))
    }

    pub fn decide_blocked_on_input(
        self,
        reason: ReasonCode,
        retry_after: Option<UtcMicros>,
    ) -> std::result::Result<JobDecision, ProjectionError> {
        if !reason.is_input_blocker() {
            return Err(ProjectionError::ReasonNotAllowed {
                branch: "BlockedOnInput",
            });
        }
        Ok(JobDecision::new(JobDecisionKind::BlockedOnInput {
            reason,
            retry_after,
        }))
    }

    pub fn decide_suppressed(
        self,
        facts: &PreparedFactsSnapshot,
        input: SemanticInput,
    ) -> std::result::Result<JobDecision, ProjectionError> {
        let projection = self.project_semantics(facts, input)?;
        match projection.suppression {
            Suppression::Eligible => Err(ProjectionError::ReasonNotAllowed {
                branch: "Suppressed",
            }),
            Suppression::Suppressed {
                reason,
                eligible_after,
            } => Ok(JobDecision::new(JobDecisionKind::Suppressed {
                reason,
                eligible_after,
            })),
        }
    }

    pub fn decide_retryable_failure(
        self,
        reason: ReasonCode,
        retry_after: Option<UtcMicros>,
    ) -> std::result::Result<JobDecision, ProjectionError> {
        if !reason.allows_input_backoff() {
            return Err(ProjectionError::ReasonNotAllowed {
                branch: "RetryableFailure",
            });
        }
        Ok(JobDecision::new(JobDecisionKind::RetryableFailure {
            reason,
            retry_after,
        }))
    }

    pub fn decide_permanent_failure(
        self,
        reason: ReasonCode,
    ) -> std::result::Result<JobDecision, ProjectionError> {
        if !reason.is_permanent_preparation_failure() {
            return Err(ProjectionError::ReasonNotAllowed {
                branch: "PermanentFailure",
            });
        }
        Ok(JobDecision::new(JobDecisionKind::PermanentFailure {
            reason,
        }))
    }

    fn verify_facts_context(
        &self,
        facts: &PreparedFactsSnapshot,
    ) -> std::result::Result<(), ProjectionError> {
        if facts.facts().run_context_sha256() != &self.run_context_sha256 {
            return Err(ProjectionError::ContextFactsMismatch);
        }
        Ok(())
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceBinding {
    source_contract_id: SourceContractId,
    source_contract_version: SourceContractVersion,
    source_refs: Vec<SourceRef>,
    evidence_fingerprint: Sha256Digest,
}

impl SourceBinding {
    fn from_facts(facts: &PreparedFacts, evidence_fingerprint: Sha256Digest) -> Self {
        Self {
            source_contract_id: facts.source_contract_id().clone(),
            source_contract_version: facts.source_contract_version().clone(),
            source_refs: facts.source_refs().to_vec(),
            evidence_fingerprint,
        }
    }

    pub fn source_contract_id(&self) -> &SourceContractId {
        &self.source_contract_id
    }

    pub fn source_contract_version(&self) -> &SourceContractVersion {
        &self.source_contract_version
    }

    pub fn source_refs(&self) -> &[SourceRef] {
        &self.source_refs
    }

    pub fn evidence_fingerprint(&self) -> &Sha256Digest {
        &self.evidence_fingerprint
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct PreparedPush {
    intent_id: IntentId,
    decision_id: DecisionId,
    unit_id: UnitId,
    occurrence: OccurrenceId,
    subject: SubjectId,
    run_context_sha256: Sha256Digest,
    prepared_facts_sha256: Sha256Digest,
    semantic_projection_sha256: Sha256Digest,
    source_binding: SourceBinding,
    rendered_bytes: ExactBytes,
    rendered_sha256: Sha256Digest,
}

impl PreparedPush {
    fn from_first_render(
        projector: &DecisionProjector,
        facts: &PreparedFacts,
        projection: &SemanticProjection,
        rendered_bytes: ExactBytes,
    ) -> Self {
        let intent_id = derive_intent_id(&IntentIdentityMaterial::new(
            projector.namespace.clone(),
            projector.unit_id.clone(),
            projector.binding.completion_owner.clone(),
            facts.source_contract_id().clone(),
            projector.occurrence.clone(),
            projection.business_subject.clone(),
            projection.audience.clone(),
        ));
        let decision_id = derive_decision_id(&intent_id);
        let rendered_sha256 = rendered_bytes.sha256().clone();
        Self {
            intent_id,
            decision_id,
            unit_id: projector.unit_id.clone(),
            occurrence: projector.occurrence.clone(),
            subject: projection.business_subject.clone(),
            run_context_sha256: projector.run_context_sha256.clone(),
            prepared_facts_sha256: facts.canonical_sha256(),
            semantic_projection_sha256: projection.sha256.clone(),
            source_binding: SourceBinding::from_facts(
                facts,
                projection.evidence_fingerprint.clone(),
            ),
            rendered_bytes,
            rendered_sha256,
        }
    }

    pub fn intent_id(&self) -> &IntentId {
        &self.intent_id
    }

    pub fn decision_id(&self) -> &DecisionId {
        &self.decision_id
    }

    pub fn unit_id(&self) -> &UnitId {
        &self.unit_id
    }

    pub fn occurrence(&self) -> &OccurrenceId {
        &self.occurrence
    }

    pub fn subject(&self) -> &SubjectId {
        &self.subject
    }

    pub fn run_context_sha256(&self) -> &Sha256Digest {
        &self.run_context_sha256
    }

    pub fn prepared_facts_sha256(&self) -> &Sha256Digest {
        &self.prepared_facts_sha256
    }

    pub fn semantic_projection_sha256(&self) -> &Sha256Digest {
        &self.semantic_projection_sha256
    }

    pub fn source_binding(&self) -> &SourceBinding {
        &self.source_binding
    }

    pub fn rendered_bytes(&self) -> &ExactBytes {
        &self.rendered_bytes
    }

    pub fn rendered_sha256(&self) -> &Sha256Digest {
        &self.rendered_sha256
    }

    pub fn replay_rendered_bytes(&self) -> &[u8] {
        self.rendered_bytes.as_bytes()
    }

    /// Exact restart snapshot. Raw rendered bytes remain a separate outbox column; this snapshot
    /// binds them by length and SHA-256 and never embeds their contents.
    pub fn canonical_snapshot_bytes(&self) -> ExactBytes {
        ExactBytes::new(canonical_preimage(
            "PreparedPush/v1",
            &prepared_push_fields(self),
        ))
    }

    pub fn compare_immutable(&self, other: &Self) -> PreparedPushComparison {
        if self.intent_id != other.intent_id {
            PreparedPushComparison::DifferentIntent
        } else if self == other {
            PreparedPushComparison::Identical
        } else {
            PreparedPushComparison::ResolutionRequired {
                reason: ReasonCode::IntentPayloadConflict,
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreparedPushComparison {
    DifferentIntent,
    Identical,
    ResolutionRequired { reason: ReasonCode },
}

pub(crate) fn derive_decision_id(intent_id: &IntentId) -> DecisionId {
    DecisionId::from_digest(&canonical_digest(
        "PreparedPushDecision/v1",
        &BTreeMap::from([(
            "intent_id",
            CanonicalValue::String(intent_id.as_str().to_owned()),
        )]),
    ))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RenderStateView {
    Open,
    Rendering,
    Sealed,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RenderState {
    Open,
    Rendering,
    Sealed,
    Failed,
}

/// A Ready preparation cannot be copied into a second renderer path.
///
/// ```compile_fail
/// use stock_analysis::monitor::push_job::ReadyPreparation;
/// fn requires_clone<T: Clone>() {}
/// requires_clone::<ReadyPreparation>();
/// ```
#[derive(Debug)]
pub struct ReadyPreparation {
    projector: DecisionProjector,
    facts: PreparedFactsSnapshot,
    projection: SemanticProjection,
    state: RenderState,
    attempt_count: u64,
    rejected_count: u64,
}

impl ReadyPreparation {
    fn new(
        projector: DecisionProjector,
        facts: PreparedFactsSnapshot,
        input: SemanticInput,
    ) -> std::result::Result<Self, ProjectionError> {
        let projection = projector.project_semantics(&facts, input)?;
        if facts.facts().verified_empty() {
            return Err(ProjectionError::VerifiedEmptyCannotBeReady);
        }
        if matches!(projection.suppression, Suppression::Suppressed { .. }) {
            return Err(ProjectionError::SuppressedCannotBeReady);
        }
        Ok(Self {
            projector,
            facts,
            projection,
            state: RenderState::Open,
            attempt_count: 0,
            rejected_count: 0,
        })
    }

    pub fn projection(&self) -> &SemanticProjection {
        &self.projection
    }

    pub fn run_context_sha256(&self) -> &Sha256Digest {
        &self.projector.run_context_sha256
    }

    pub fn prepared_facts_sha256(&self) -> Sha256Digest {
        self.facts.facts().canonical_sha256()
    }

    pub fn render_once<F>(&mut self, render: F) -> std::result::Result<JobDecision, ProjectionError>
    where
        F: FnOnce(&SemanticProjection) -> Vec<u8>,
    {
        let current = self.state();
        if current != RenderStateView::Open {
            self.rejected_count = self.rejected_count.saturating_add(1);
            return Err(ProjectionError::RenderAlreadyAttempted { state: current });
        }
        self.state = RenderState::Rendering;
        self.attempt_count = self.attempt_count.saturating_add(1);
        let bytes = render(&self.projection);
        if std::str::from_utf8(&bytes).is_err() {
            self.state = RenderState::Failed;
            return Err(ProjectionError::RenderedBytesNotUtf8);
        }
        let rendered_bytes = ExactBytes::new(bytes);
        let prepared_push = PreparedPush::from_first_render(
            &self.projector,
            self.facts.facts(),
            &self.projection,
            rendered_bytes,
        );
        self.state = RenderState::Sealed;
        Ok(JobDecision::new(JobDecisionKind::Ready(Box::new(
            prepared_push,
        ))))
    }

    pub fn state(&self) -> RenderStateView {
        match self.state {
            RenderState::Open => RenderStateView::Open,
            RenderState::Rendering => RenderStateView::Rendering,
            RenderState::Sealed => RenderStateView::Sealed,
            RenderState::Failed => RenderStateView::Failed,
        }
    }

    pub fn attempt_count(&self) -> u64 {
        self.attempt_count
    }

    pub fn rejected_count(&self) -> u64 {
        self.rejected_count
    }
}

#[derive(Debug, Eq, PartialEq)]
enum JobDecisionKind {
    Ready(Box<PreparedPush>),
    NoData {
        reason: ReasonCode,
        evidence_sha256: Sha256Digest,
    },
    Disabled {
        reason: ReasonCode,
    },
    BlockedOnInput {
        reason: ReasonCode,
        retry_after: Option<UtcMicros>,
    },
    Suppressed {
        reason: ReasonCode,
        eligible_after: Option<UtcMicros>,
    },
    RetryableFailure {
        reason: ReasonCode,
        retry_after: Option<UtcMicros>,
    },
    PermanentFailure {
        reason: ReasonCode,
    },
}

#[derive(Debug, Eq, PartialEq)]
pub struct JobDecision {
    kind: JobDecisionKind,
}

impl JobDecision {
    fn new(kind: JobDecisionKind) -> Self {
        Self { kind }
    }

    pub fn view(&self) -> JobDecisionView<'_> {
        match &self.kind {
            JobDecisionKind::Ready(push) => JobDecisionView::Ready(push),
            JobDecisionKind::NoData {
                reason,
                evidence_sha256,
            } => JobDecisionView::NoData {
                reason: *reason,
                evidence_sha256,
            },
            JobDecisionKind::Disabled { reason } => JobDecisionView::Disabled { reason: *reason },
            JobDecisionKind::BlockedOnInput {
                reason,
                retry_after,
            } => JobDecisionView::BlockedOnInput {
                reason: *reason,
                retry_after: *retry_after,
            },
            JobDecisionKind::Suppressed {
                reason,
                eligible_after,
            } => JobDecisionView::Suppressed {
                reason: *reason,
                eligible_after: *eligible_after,
            },
            JobDecisionKind::RetryableFailure {
                reason,
                retry_after,
            } => JobDecisionView::RetryableFailure {
                reason: *reason,
                retry_after: *retry_after,
            },
            JobDecisionKind::PermanentFailure { reason } => {
                JobDecisionView::PermanentFailure { reason: *reason }
            }
        }
    }

    pub fn canonical_sha256(&self) -> Sha256Digest {
        canonical_digest("JobDecision/v1", &job_decision_fields(&self.kind))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobDecisionView<'a> {
    Ready(&'a PreparedPush),
    NoData {
        reason: ReasonCode,
        evidence_sha256: &'a Sha256Digest,
    },
    Disabled {
        reason: ReasonCode,
    },
    BlockedOnInput {
        reason: ReasonCode,
        retry_after: Option<UtcMicros>,
    },
    Suppressed {
        reason: ReasonCode,
        eligible_after: Option<UtcMicros>,
    },
    RetryableFailure {
        reason: ReasonCode,
        retry_after: Option<UtcMicros>,
    },
    PermanentFailure {
        reason: ReasonCode,
    },
}

fn job_decision_fields(kind: &JobDecisionKind) -> BTreeMap<&'static str, CanonicalValue> {
    let (variant, payload) = match kind {
        JobDecisionKind::Ready(push) => ("Ready", prepared_push_value(push)),
        JobDecisionKind::NoData {
            reason,
            evidence_sha256,
        } => (
            "NoData",
            CanonicalValue::Object(BTreeMap::from([
                (
                    "evidence_sha256",
                    CanonicalValue::String(evidence_sha256.as_str().to_owned()),
                ),
                ("reason", CanonicalValue::String(reason.as_str().to_owned())),
            ])),
        ),
        JobDecisionKind::Disabled { reason } => ("Disabled", reason_payload(*reason)),
        JobDecisionKind::BlockedOnInput {
            reason,
            retry_after,
        } => (
            "BlockedOnInput",
            reason_time_payload(*reason, "retry_after", *retry_after),
        ),
        JobDecisionKind::Suppressed {
            reason,
            eligible_after,
        } => (
            "Suppressed",
            reason_time_payload(*reason, "eligible_after", *eligible_after),
        ),
        JobDecisionKind::RetryableFailure {
            reason,
            retry_after,
        } => (
            "RetryableFailure",
            reason_time_payload(*reason, "retry_after", *retry_after),
        ),
        JobDecisionKind::PermanentFailure { reason } => {
            ("PermanentFailure", reason_payload(*reason))
        }
    };
    BTreeMap::from([
        ("payload", payload),
        ("variant", CanonicalValue::String(variant.to_owned())),
    ])
}

fn reason_payload(reason: ReasonCode) -> CanonicalValue {
    CanonicalValue::Object(BTreeMap::from([(
        "reason",
        CanonicalValue::String(reason.as_str().to_owned()),
    )]))
}

fn reason_time_payload(
    reason: ReasonCode,
    time_field: &'static str,
    time: Option<UtcMicros>,
) -> CanonicalValue {
    CanonicalValue::Object(BTreeMap::from([
        ("reason", CanonicalValue::String(reason.as_str().to_owned())),
        (
            time_field,
            time.map_or(CanonicalValue::Null, |value| {
                CanonicalValue::Unsigned(value.get() as u64)
            }),
        ),
    ]))
}

fn prepared_push_value(push: &PreparedPush) -> CanonicalValue {
    CanonicalValue::Object(prepared_push_fields(push))
}

fn prepared_push_fields(push: &PreparedPush) -> BTreeMap<&'static str, CanonicalValue> {
    BTreeMap::from([
        (
            "decision_id",
            CanonicalValue::String(push.decision_id.as_str().to_owned()),
        ),
        (
            "intent_id",
            CanonicalValue::String(push.intent_id.as_str().to_owned()),
        ),
        (
            "occurrence",
            CanonicalValue::String(push.occurrence.as_str().to_owned()),
        ),
        (
            "prepared_facts_sha256",
            CanonicalValue::String(push.prepared_facts_sha256.as_str().to_owned()),
        ),
        ("rendered_bytes", exact_bytes_value(&push.rendered_bytes)),
        (
            "rendered_sha256",
            CanonicalValue::String(push.rendered_sha256.as_str().to_owned()),
        ),
        (
            "run_context_sha256",
            CanonicalValue::String(push.run_context_sha256.as_str().to_owned()),
        ),
        (
            "semantic_projection_sha256",
            CanonicalValue::String(push.semantic_projection_sha256.as_str().to_owned()),
        ),
        ("source_binding", source_binding_value(&push.source_binding)),
        ("subject", subject_value(&push.subject)),
        (
            "unit_id",
            CanonicalValue::String(push.unit_id.as_str().to_owned()),
        ),
    ])
}

fn exact_bytes_value(bytes: &ExactBytes) -> CanonicalValue {
    CanonicalValue::Object(BTreeMap::from([
        ("length", CanonicalValue::Unsigned(bytes.len() as u64)),
        (
            "sha256",
            CanonicalValue::String(bytes.sha256().as_str().to_owned()),
        ),
    ]))
}

fn source_binding_value(binding: &SourceBinding) -> CanonicalValue {
    CanonicalValue::Object(BTreeMap::from([
        (
            "evidence_fingerprint",
            CanonicalValue::String(binding.evidence_fingerprint.as_str().to_owned()),
        ),
        (
            "source_contract_id",
            CanonicalValue::String(binding.source_contract_id.as_str().to_owned()),
        ),
        (
            "source_contract_version",
            CanonicalValue::String(binding.source_contract_version.as_str().to_owned()),
        ),
        (
            "source_refs",
            CanonicalValue::Array(binding.source_refs.iter().map(source_ref_value).collect()),
        ),
    ]))
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
    .map_err(|_| PushJobError::InvalidRunContext("projection fixture binding mismatch"))
}

#[cfg(test)]
pub(crate) fn w08_prepared_push_fixture() -> PreparedPush {
    let source_contract_id = SourceContractId::try_new("auction-source".to_owned()).unwrap();
    let occurrence = super::derive_occurrence_id(&super::OccurrenceIdentityMaterial::new(
        super::BusinessDate::parse("2026-09-07").unwrap(),
        super::OccurrenceFamily::try_new("auction-session".to_owned()).unwrap(),
        super::OccurrenceKey::try_new("main".to_owned()).unwrap(),
    ));
    let subject = SubjectId::entity("000001.SZ".to_owned()).unwrap();
    let intent_id = derive_intent_id(&IntentIdentityMaterial::new(
        Namespace::Production,
        UnitId::try_new("MU-auction".to_owned()).unwrap(),
        CompletionOwnerId::try_new("owner-auction".to_owned()).unwrap(),
        source_contract_id.clone(),
        occurrence.clone(),
        subject.clone(),
        AudienceId::try_new("portfolio-owner".to_owned()).unwrap(),
    ));
    let rendered_bytes = ExactBytes::new(b"first render  \nline two!".to_vec());
    let rendered_sha256 = rendered_bytes.sha256().clone();
    PreparedPush {
        decision_id: derive_decision_id(&intent_id),
        intent_id,
        unit_id: UnitId::try_new("MU-auction".to_owned()).unwrap(),
        occurrence,
        subject,
        run_context_sha256: Sha256Digest::parse("fixture", &"c".repeat(64)).unwrap(),
        prepared_facts_sha256: Sha256Digest::parse("fixture", &"d".repeat(64)).unwrap(),
        semantic_projection_sha256: Sha256Digest::parse("fixture", &"e".repeat(64)).unwrap(),
        source_binding: SourceBinding {
            source_contract_id: source_contract_id.clone(),
            source_contract_version: SourceContractVersion::try_new("auction-source-v2".to_owned())
                .unwrap(),
            source_refs: vec![SourceRef::new(
                super::SourceRefId::try_new("source-1".to_owned()).unwrap(),
                super::SourceProvider::try_new("fixture-provider".to_owned()).unwrap(),
                super::ExternalId::try_new("external-1".to_owned()).unwrap(),
                source_contract_id,
                Sha256Digest::parse("fixture", &"a".repeat(64)).unwrap(),
            )],
            evidence_fingerprint: Sha256Digest::parse("fixture", &"b".repeat(64)).unwrap(),
        },
        rendered_bytes,
        rendered_sha256,
    }
}
