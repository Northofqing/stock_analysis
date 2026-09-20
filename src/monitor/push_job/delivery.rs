//! W02 delivery evidence and result contracts.

use std::collections::BTreeSet;

use crate::durable_delivery::DecisionState;

use super::identity::validate_text;
use super::policy::ReasonCode;
use super::{
    AudienceId, BusinessDate, IntentId, Namespace, OccurrenceId, PushJobError, Result,
    Sha256Digest, SubjectId, UnitId, UtcMicros,
};

macro_rules! delivery_text_id {
    ($name:ident, $field:literal) => {
        #[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
        pub struct $name(String);

        impl $name {
            pub fn try_new(value: String) -> Result<Self> {
                validate_text($field, value).map(Self)
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

delivery_text_id!(ChannelId, "channel_id");
delivery_text_id!(CompatId, "compat_id");
delivery_text_id!(AttemptId, "attempt_id");
delivery_text_id!(DecisionId, "decision_id");
delivery_text_id!(DurableSchemaVersion, "durable_schema_version");
delivery_text_id!(TemplateId, "template_id");
delivery_text_id!(TemplateVersion, "template_version");
delivery_text_id!(TerminalRefId, "terminal_ref_id");

impl DecisionId {
    pub(super) fn from_digest(digest: &Sha256Digest) -> Self {
        Self(digest.as_str().to_owned())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum AuthorityClass {
    GenericCounted,
    P01Dedicated,
    N02Dedicated,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum TerminalDisposition {
    Accepted,
    Rejected,
    Uncertain,
    ManualConfirmedAccepted,
    ManualConfirmedNotDelivered,
}

/// A compatibility observation cannot be upgraded into an authoritative terminal.
///
/// ```compile_fail
/// use stock_analysis::monitor::push_job::{
///     CompatibilityEvidenceRef, VerifiedTerminalRef,
/// };
/// fn forbidden(weak: CompatibilityEvidenceRef) -> VerifiedTerminalRef {
///     weak.into()
/// }
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedTerminalRef {
    ref_id: TerminalRefId,
    authority_class: AuthorityClass,
    namespace: Namespace,
    decision_id: DecisionId,
    attempt_id: Option<AttemptId>,
    intent_id: IntentId,
    unit_id: UnitId,
    occurrence: OccurrenceId,
    business_date: BusinessDate,
    subject: SubjectId,
    audience: AudienceId,
    template_id: TemplateId,
    template_version: TemplateVersion,
    rendered_sha256: Sha256Digest,
    terminal_disposition: TerminalDisposition,
    evidence_sha256: Sha256Digest,
    durable_schema_version: DurableSchemaVersion,
    verified_at: UtcMicros,
    binding_sha256: Sha256Digest,
}

pub(crate) struct VerifiedTerminalParts {
    pub(crate) ref_id: TerminalRefId,
    pub(crate) authority_class: AuthorityClass,
    pub(crate) namespace: Namespace,
    pub(crate) decision_id: DecisionId,
    pub(crate) attempt_id: Option<AttemptId>,
    pub(crate) intent_id: IntentId,
    pub(crate) unit_id: UnitId,
    pub(crate) occurrence: OccurrenceId,
    pub(crate) business_date: BusinessDate,
    pub(crate) subject: SubjectId,
    pub(crate) audience: AudienceId,
    pub(crate) template_id: TemplateId,
    pub(crate) template_version: TemplateVersion,
    pub(crate) rendered_sha256: Sha256Digest,
    pub(crate) terminal_disposition: TerminalDisposition,
    pub(crate) evidence_sha256: Sha256Digest,
    pub(crate) durable_schema_version: DurableSchemaVersion,
    pub(crate) verified_at: UtcMicros,
    pub(crate) binding_sha256: Sha256Digest,
}

impl VerifiedTerminalRef {
    pub(crate) fn from_verified_parts(parts: VerifiedTerminalParts) -> Self {
        Self {
            ref_id: parts.ref_id,
            authority_class: parts.authority_class,
            namespace: parts.namespace,
            decision_id: parts.decision_id,
            attempt_id: parts.attempt_id,
            intent_id: parts.intent_id,
            unit_id: parts.unit_id,
            occurrence: parts.occurrence,
            business_date: parts.business_date,
            subject: parts.subject,
            audience: parts.audience,
            template_id: parts.template_id,
            template_version: parts.template_version,
            rendered_sha256: parts.rendered_sha256,
            terminal_disposition: parts.terminal_disposition,
            evidence_sha256: parts.evidence_sha256,
            durable_schema_version: parts.durable_schema_version,
            verified_at: parts.verified_at,
            binding_sha256: parts.binding_sha256,
        }
    }

    pub fn ref_id(&self) -> &TerminalRefId {
        &self.ref_id
    }

    pub fn authority_class(&self) -> AuthorityClass {
        self.authority_class
    }

    pub fn namespace(&self) -> &Namespace {
        &self.namespace
    }

    pub fn decision_id(&self) -> &DecisionId {
        &self.decision_id
    }

    pub fn attempt_id(&self) -> Option<&AttemptId> {
        self.attempt_id.as_ref()
    }

    pub fn intent_id(&self) -> &IntentId {
        &self.intent_id
    }

    pub fn unit_id(&self) -> &UnitId {
        &self.unit_id
    }

    pub fn occurrence(&self) -> &OccurrenceId {
        &self.occurrence
    }

    pub fn business_date(&self) -> &BusinessDate {
        &self.business_date
    }

    pub fn subject(&self) -> &SubjectId {
        &self.subject
    }

    pub fn audience(&self) -> &AudienceId {
        &self.audience
    }

    pub fn template_id(&self) -> &TemplateId {
        &self.template_id
    }

    pub fn template_version(&self) -> &TemplateVersion {
        &self.template_version
    }

    pub fn rendered_sha256(&self) -> &Sha256Digest {
        &self.rendered_sha256
    }

    pub fn terminal_disposition(&self) -> TerminalDisposition {
        self.terminal_disposition
    }

    pub fn evidence_sha256(&self) -> &Sha256Digest {
        &self.evidence_sha256
    }

    pub fn durable_schema_version(&self) -> &DurableSchemaVersion {
        &self.durable_schema_version
    }

    pub fn verified_at(&self) -> UtcMicros {
        self.verified_at
    }

    pub fn binding_sha256(&self) -> &Sha256Digest {
        &self.binding_sha256
    }

    pub fn into_delivery_result(self) -> DeliveryResult {
        DeliveryResult::from_verified_terminal(self)
    }

    pub(crate) fn same_stable_binding(&self, other: &Self) -> bool {
        self.ref_id == other.ref_id
            && self.authority_class == other.authority_class
            && self.namespace == other.namespace
            && self.decision_id == other.decision_id
            && self.attempt_id == other.attempt_id
            && self.intent_id == other.intent_id
            && self.unit_id == other.unit_id
            && self.occurrence == other.occurrence
            && self.business_date == other.business_date
            && self.subject == other.subject
            && self.audience == other.audience
            && self.template_id == other.template_id
            && self.template_version == other.template_version
            && self.rendered_sha256 == other.rendered_sha256
            && self.terminal_disposition == other.terminal_disposition
            && self.evidence_sha256 == other.evidence_sha256
            && self.durable_schema_version == other.durable_schema_version
            && self.binding_sha256 == other.binding_sha256
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum WeakOutcomeKind {
    Accepted,
    Rejected,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WeakOutcome {
    channel: ChannelId,
    kind: WeakOutcomeKind,
    local_evidence_sha256: Sha256Digest,
}

impl WeakOutcome {
    pub fn new(
        channel: ChannelId,
        kind: WeakOutcomeKind,
        local_evidence_sha256: Sha256Digest,
    ) -> Self {
        Self {
            channel,
            kind,
            local_evidence_sha256,
        }
    }

    pub fn channel(&self) -> &ChannelId {
        &self.channel
    }

    pub fn kind(&self) -> WeakOutcomeKind {
        self.kind
    }

    pub fn local_evidence_sha256(&self) -> &Sha256Digest {
        &self.local_evidence_sha256
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompatibilityEvidenceRef {
    compat_id: CompatId,
    intent_id: IntentId,
    unit_id: UnitId,
    occurrence: OccurrenceId,
    configured_channels: Vec<ChannelId>,
    attempted_channels: Vec<ChannelId>,
    weak_outcomes: Vec<WeakOutcome>,
    local_evidence_sha256: Sha256Digest,
    captured_at: UtcMicros,
}

impl CompatibilityEvidenceRef {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        compat_id: CompatId,
        intent_id: IntentId,
        unit_id: UnitId,
        occurrence: OccurrenceId,
        configured_channels: Vec<ChannelId>,
        attempted_channels: Vec<ChannelId>,
        weak_outcomes: Vec<WeakOutcome>,
        local_evidence_sha256: Sha256Digest,
        captured_at: UtcMicros,
    ) -> Result<Self> {
        let configured = unique_channels(&configured_channels).ok_or(
            PushJobError::InvalidCompatibilityEvidence("configured channels must be unique"),
        )?;
        let attempted = unique_channels(&attempted_channels).ok_or(
            PushJobError::InvalidCompatibilityEvidence("attempted channels must be unique"),
        )?;
        if !attempted.is_subset(&configured) {
            return Err(PushJobError::InvalidCompatibilityEvidence(
                "attempted channels must be a subset of configured channels",
            ));
        }

        let outcome_channels = weak_outcomes
            .iter()
            .map(|outcome| outcome.channel.as_str())
            .collect::<BTreeSet<_>>();
        if outcome_channels.len() != weak_outcomes.len() || outcome_channels != attempted {
            return Err(PushJobError::InvalidCompatibilityEvidence(
                "each attempted channel must have exactly one weak outcome",
            ));
        }

        Ok(Self {
            compat_id,
            intent_id,
            unit_id,
            occurrence,
            configured_channels,
            attempted_channels,
            weak_outcomes,
            local_evidence_sha256,
            captured_at,
        })
    }

    pub fn compat_id(&self) -> &CompatId {
        &self.compat_id
    }

    pub fn intent_id(&self) -> &IntentId {
        &self.intent_id
    }

    pub fn unit_id(&self) -> &UnitId {
        &self.unit_id
    }

    pub fn occurrence(&self) -> &OccurrenceId {
        &self.occurrence
    }

    pub fn configured_channels(&self) -> &[ChannelId] {
        &self.configured_channels
    }

    pub fn attempted_channels(&self) -> &[ChannelId] {
        &self.attempted_channels
    }

    pub fn weak_outcomes(&self) -> &[WeakOutcome] {
        &self.weak_outcomes
    }

    pub fn local_evidence_sha256(&self) -> &Sha256Digest {
        &self.local_evidence_sha256
    }

    pub fn captured_at(&self) -> UtcMicros {
        self.captured_at
    }
}

fn unique_channels(channels: &[ChannelId]) -> Option<BTreeSet<&str>> {
    let unique = channels
        .iter()
        .map(ChannelId::as_str)
        .collect::<BTreeSet<_>>();
    (unique.len() == channels.len()).then_some(unique)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum DeliveryAuthority {
    Strong,
    Compat,
    None,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum CompletionEligibility {
    PolicyBound,
    Never,
}

#[derive(Clone, Debug, Eq, PartialEq)]
// W09 will be the first production authority adapter allowed to construct these branches.
#[allow(dead_code)]
enum DeliveryResultKind {
    TransportAccepted(VerifiedTerminalRef),
    TransportRejected(VerifiedTerminalRef),
    TransportUncertain(VerifiedTerminalRef),
    AlreadyTerminal(VerifiedTerminalRef),
    BestEffortAccepted(CompatibilityEvidenceRef),
    PartiallyAccepted(CompatibilityEvidenceRef),
    NoChannelConfigured(ReasonCode),
    AllChannelsFailed(CompatibilityEvidenceRef),
    Blocked(ReasonCode),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliveryResult(DeliveryResultKind);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryResultView<'a> {
    TransportAccepted(&'a VerifiedTerminalRef),
    TransportRejected(&'a VerifiedTerminalRef),
    TransportUncertain(&'a VerifiedTerminalRef),
    AlreadyTerminal(&'a VerifiedTerminalRef),
    BestEffortAccepted(&'a CompatibilityEvidenceRef),
    PartiallyAccepted(&'a CompatibilityEvidenceRef),
    NoChannelConfigured(ReasonCode),
    AllChannelsFailed(&'a CompatibilityEvidenceRef),
    Blocked(ReasonCode),
}

impl DeliveryResult {
    pub(super) fn from_verified_terminal(terminal: VerifiedTerminalRef) -> Self {
        let kind = match terminal.terminal_disposition {
            TerminalDisposition::Accepted => DeliveryResultKind::TransportAccepted(terminal),
            TerminalDisposition::Rejected => DeliveryResultKind::TransportRejected(terminal),
            TerminalDisposition::Uncertain => DeliveryResultKind::TransportUncertain(terminal),
            TerminalDisposition::ManualConfirmedAccepted
            | TerminalDisposition::ManualConfirmedNotDelivered => {
                DeliveryResultKind::AlreadyTerminal(terminal)
            }
        };
        Self(kind)
    }

    pub fn best_effort_accepted(evidence: CompatibilityEvidenceRef) -> Result<Self> {
        let accepted = accepted_count(&evidence);
        let all_configured_attempted =
            evidence.attempted_channels.len() == evidence.configured_channels.len();
        if evidence.configured_channels.is_empty()
            || !all_configured_attempted
            || accepted != evidence.configured_channels.len()
        {
            return Err(PushJobError::InvalidDeliveryResult(
                "best-effort accepted requires every configured channel to accept",
            ));
        }
        Ok(Self(DeliveryResultKind::BestEffortAccepted(evidence)))
    }

    pub fn partially_accepted(evidence: CompatibilityEvidenceRef) -> Result<Self> {
        let accepted = accepted_count(&evidence);
        if accepted == 0 || accepted >= evidence.configured_channels.len() {
            return Err(PushJobError::InvalidDeliveryResult(
                "partial acceptance requires accepted and non-accepted configured channels",
            ));
        }
        Ok(Self(DeliveryResultKind::PartiallyAccepted(evidence)))
    }

    pub fn no_channel_configured() -> Self {
        Self(DeliveryResultKind::NoChannelConfigured(
            ReasonCode::TransportNoChannelConfigured,
        ))
    }

    pub fn all_channels_failed(evidence: CompatibilityEvidenceRef) -> Result<Self> {
        if evidence.configured_channels.is_empty() || accepted_count(&evidence) != 0 {
            return Err(PushJobError::InvalidDeliveryResult(
                "all channels failed requires configured channels and zero accepted outcomes",
            ));
        }
        Ok(Self(DeliveryResultKind::AllChannelsFailed(evidence)))
    }

    pub fn blocked(reason: ReasonCode) -> Self {
        Self(DeliveryResultKind::Blocked(reason))
    }

    pub fn view(&self) -> DeliveryResultView<'_> {
        match &self.0 {
            DeliveryResultKind::TransportAccepted(terminal) => {
                DeliveryResultView::TransportAccepted(terminal)
            }
            DeliveryResultKind::TransportRejected(terminal) => {
                DeliveryResultView::TransportRejected(terminal)
            }
            DeliveryResultKind::TransportUncertain(terminal) => {
                DeliveryResultView::TransportUncertain(terminal)
            }
            DeliveryResultKind::AlreadyTerminal(terminal) => {
                DeliveryResultView::AlreadyTerminal(terminal)
            }
            DeliveryResultKind::BestEffortAccepted(evidence) => {
                DeliveryResultView::BestEffortAccepted(evidence)
            }
            DeliveryResultKind::PartiallyAccepted(evidence) => {
                DeliveryResultView::PartiallyAccepted(evidence)
            }
            DeliveryResultKind::NoChannelConfigured(reason) => {
                DeliveryResultView::NoChannelConfigured(*reason)
            }
            DeliveryResultKind::AllChannelsFailed(evidence) => {
                DeliveryResultView::AllChannelsFailed(evidence)
            }
            DeliveryResultKind::Blocked(reason) => DeliveryResultView::Blocked(*reason),
        }
    }

    pub fn authority_class(&self) -> DeliveryAuthority {
        match &self.0 {
            DeliveryResultKind::TransportAccepted(_)
            | DeliveryResultKind::TransportRejected(_)
            | DeliveryResultKind::TransportUncertain(_)
            | DeliveryResultKind::AlreadyTerminal(_) => DeliveryAuthority::Strong,
            DeliveryResultKind::BestEffortAccepted(_)
            | DeliveryResultKind::PartiallyAccepted(_)
            | DeliveryResultKind::NoChannelConfigured(_)
            | DeliveryResultKind::AllChannelsFailed(_) => DeliveryAuthority::Compat,
            DeliveryResultKind::Blocked(_) => DeliveryAuthority::None,
        }
    }

    pub fn completion_eligibility(&self) -> CompletionEligibility {
        match &self.0 {
            DeliveryResultKind::TransportAccepted(_) | DeliveryResultKind::AlreadyTerminal(_) => {
                CompletionEligibility::PolicyBound
            }
            DeliveryResultKind::TransportRejected(_)
            | DeliveryResultKind::TransportUncertain(_)
            | DeliveryResultKind::BestEffortAccepted(_)
            | DeliveryResultKind::PartiallyAccepted(_)
            | DeliveryResultKind::NoChannelConfigured(_)
            | DeliveryResultKind::AllChannelsFailed(_)
            | DeliveryResultKind::Blocked(_) => CompletionEligibility::Never,
        }
    }

    pub fn requires_manual_quarantine(&self) -> bool {
        matches!(&self.0, DeliveryResultKind::TransportUncertain(_))
    }

    pub fn reason_code(&self) -> Option<ReasonCode> {
        match &self.0 {
            DeliveryResultKind::TransportAccepted(_)
            | DeliveryResultKind::AlreadyTerminal(_)
            | DeliveryResultKind::BestEffortAccepted(_) => None,
            DeliveryResultKind::TransportRejected(_) => Some(ReasonCode::TransportRejected),
            DeliveryResultKind::TransportUncertain(_) => Some(ReasonCode::TransportUncertain),
            DeliveryResultKind::PartiallyAccepted(_) => {
                Some(ReasonCode::TransportPartiallyAccepted)
            }
            DeliveryResultKind::NoChannelConfigured(reason)
            | DeliveryResultKind::Blocked(reason) => Some(*reason),
            DeliveryResultKind::AllChannelsFailed(_) => {
                Some(ReasonCode::TransportAllChannelsFailed)
            }
        }
    }
}

fn accepted_count(evidence: &CompatibilityEvidenceRef) -> usize {
    evidence
        .weak_outcomes
        .iter()
        .filter(|outcome| outcome.kind == WeakOutcomeKind::Accepted)
        .count()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum DurableStateProjection {
    BlockedBeforeAttempt,
    BlockedAwaitingReconciliation,
    BlockedAwaitingAuthoritySeal,
    RequiresVerifiedAcceptedOrAlreadyTerminal,
    RequiresVerifiedRejectedOrAlreadyTerminal,
    RequiresVerifiedUncertainOrAlreadyTerminal,
    RequiresVerifiedNotDeliveredTerminal,
}

pub fn classify_durable_state(state: DecisionState) -> DurableStateProjection {
    match state {
        DecisionState::Reserved => DurableStateProjection::BlockedBeforeAttempt,
        DecisionState::AttemptInFlight => DurableStateProjection::BlockedAwaitingReconciliation,
        DecisionState::AcceptedAuditPending | DecisionState::AcceptedTaskTransitionPending => {
            DurableStateProjection::BlockedAwaitingAuthoritySeal
        }
        DecisionState::Delivered => {
            DurableStateProjection::RequiresVerifiedAcceptedOrAlreadyTerminal
        }
        DecisionState::RejectedAuditPending | DecisionState::RejectedTaskTransitionPending => {
            DurableStateProjection::BlockedAwaitingReconciliation
        }
        DecisionState::RejectedDurable => {
            DurableStateProjection::RequiresVerifiedRejectedOrAlreadyTerminal
        }
        DecisionState::UncertainAuditPending
        | DecisionState::UncertainTaskTransitionPending
        | DecisionState::ManualRejectedAuditPending
        | DecisionState::ManualRejectedTaskTransitionPending => {
            DurableStateProjection::BlockedAwaitingAuthoritySeal
        }
        DecisionState::UncertainManualReview => {
            DurableStateProjection::RequiresVerifiedUncertainOrAlreadyTerminal
        }
        DecisionState::ManualResolvedRejected => {
            DurableStateProjection::RequiresVerifiedNotDeliveredTerminal
        }
    }
}

#[cfg(test)]
pub(super) fn verified_terminal_fixture(
    terminal_disposition: TerminalDisposition,
) -> VerifiedTerminalRef {
    use super::{
        derive_intent_id, derive_occurrence_id, CompletionOwnerId, IntentIdentityMaterial,
        OccurrenceFamily, OccurrenceIdentityMaterial, OccurrenceKey, SourceContractId,
    };

    let business_date = BusinessDate::parse("2026-09-06").expect("fixture business date");
    let occurrence = derive_occurrence_id(&OccurrenceIdentityMaterial::new(
        business_date.clone(),
        OccurrenceFamily::try_new("daily".to_owned()).expect("fixture occurrence family"),
        OccurrenceKey::try_new("close".to_owned()).expect("fixture occurrence key"),
    ));
    let unit_id = UnitId::try_new("MU-fixture".to_owned()).expect("fixture unit");
    let subject = SubjectId::Global;
    let audience = AudienceId::try_new("fixture-audience".to_owned()).expect("fixture audience");
    let intent_id = derive_intent_id(&IntentIdentityMaterial::new(
        Namespace::Production,
        unit_id.clone(),
        CompletionOwnerId::try_new("fixture-owner".to_owned()).expect("fixture owner"),
        SourceContractId::try_new("fixture-source-v1".to_owned()).expect("fixture source"),
        occurrence.clone(),
        subject.clone(),
        audience.clone(),
    ));

    VerifiedTerminalRef {
        ref_id: TerminalRefId::try_new("fixture-terminal-ref".to_owned())
            .expect("fixture terminal ref"),
        authority_class: AuthorityClass::GenericCounted,
        namespace: Namespace::Production,
        decision_id: DecisionId::try_new("fixture-decision".to_owned()).expect("fixture decision"),
        attempt_id: Some(
            AttemptId::try_new("fixture-attempt".to_owned()).expect("fixture attempt"),
        ),
        intent_id,
        unit_id,
        occurrence,
        business_date,
        subject,
        audience,
        template_id: TemplateId::try_new("fixture-template".to_owned()).expect("fixture template"),
        template_version: TemplateVersion::try_new("v1".to_owned())
            .expect("fixture template version"),
        rendered_sha256: Sha256Digest::parse("fixture rendered", &"a".repeat(64))
            .expect("fixture rendered digest"),
        terminal_disposition,
        evidence_sha256: Sha256Digest::parse("fixture evidence", &"b".repeat(64))
            .expect("fixture evidence digest"),
        durable_schema_version: DurableSchemaVersion::try_new("v1".to_owned())
            .expect("fixture durable schema version"),
        verified_at: UtcMicros::try_new(1_788_705_600_000_000).expect("fixture verified time"),
        binding_sha256: Sha256Digest::parse("fixture binding", &"c".repeat(64))
            .expect("fixture binding digest"),
    }
}
