//! Application-level push contracts. This module is pure and has no runtime wiring.

mod canonical;
mod catalog;
mod context;
mod delivery;
mod facts;
mod identity;
mod policy;
mod projection;

pub(crate) use canonical::{canonical_preimage, raw_digest, CanonicalValue};

pub use context::{
    AuthenticatedOperatorRef, CalendarDate, CommandId, GitSha40, PhaseEpic, RunContext, ScheduleId,
    Trigger, TriggerView,
};

pub use catalog::{
    CatalogEntity, CatalogKindRegistration, CatalogProducerRegistration, CatalogRelation,
    CatalogStatus, CatalogUnitRegistration, MachineCatalog, MachineCatalogError,
    MachineCatalogStatus,
};

pub use delivery::{
    classify_durable_state, AttemptId, AuthorityClass, ChannelId, CompatId,
    CompatibilityEvidenceRef, CompletionEligibility, DecisionId, DeliveryAuthority, DeliveryResult,
    DeliveryResultView, DurableSchemaVersion, DurableStateProjection, TemplateId, TemplateVersion,
    TerminalDisposition, TerminalRefId, VerifiedTerminalRef, WeakOutcome, WeakOutcomeKind,
};
pub use facts::{
    CaptureStateView, CapturedFacts, ExactBytes, ExternalId, FactsPresence, ModelId,
    ModelOutputRef, ModelVersion, PreparationCapture, PreparationError, PreparedFacts,
    PreparedFactsSnapshot, ProtectedRef, SourceProvider, SourceRef, SourceRefId, SourceTime,
    SourceTimeKind,
};
pub use identity::{
    derive_intent_id, derive_occurrence_id, derive_schedule_occurrence_id, AudienceId,
    BusinessDate, CalendarId, CompletionOwnerId, IntentId, IntentIdentityMaterial, Namespace,
    OccurrenceFamily, OccurrenceId, OccurrenceIdentityMaterial, OccurrenceKey, ProducerId, RunId,
    ScheduleOccurrenceId, ScheduleOccurrenceIdentityMaterial, ScheduleOrTriggerId, Sha256Digest,
    SourceContractId, SourceContractVersion, SubjectId, SubjectValue, UnitId, UtcMicros,
};
pub use policy::{
    evaluate_completion, AdvanceEvent, AlreadyTerminalPolicy, CatalogOwnerRef, CompletionDirective,
    CompletionFact, CompletionPolicy, CompletionPolicyId, CompletionPolicyVersion, CursorDirective,
    CursorPolicy, DisabledEvidenceRef, DisabledPolicy, FinalizerKind, ManualDirective,
    NoDataPolicy, ReasonCode, RetentionClass, RetryDirective, RetryEligibility, RetryPolicy,
    ScheduleCloseBranch, ScheduleClosePolicy, ScheduleDirective, UncertainPolicy,
    VerifiedEmptyEvidenceRef,
};
pub use projection::{
    DecisionProjector, JobDecision, JobDecisionView, MonitorKind, PreparedPush,
    PreparedPushComparison, ProjectionError, ReadyPreparation, RenderStateView, SemanticInput,
    SemanticProjection, Severity, SourceBinding, SubKind, SubKindValue, Suppression,
};

pub(crate) use projection::derive_decision_id;
#[cfg(test)]
pub(crate) use projection::w08_prepared_push_fixture;

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum PushJobError {
    #[error("invalid {field}: {reason}")]
    InvalidText {
        field: &'static str,
        reason: &'static str,
    },
    #[error("invalid business date: {0}")]
    InvalidBusinessDate(String),
    #[error("invalid calendar date: {0}")]
    InvalidCalendarDate(String),
    #[error("invalid Git SHA-40")]
    InvalidGitSha40,
    #[error("invalid sha256 for {field}")]
    InvalidSha256 { field: &'static str },
    #[error("UTC microseconds must be non-negative")]
    InvalidUtcMicros,
    #[error("invalid compatibility evidence: {0}")]
    InvalidCompatibilityEvidence(&'static str),
    #[error("invalid delivery result: {0}")]
    InvalidDeliveryResult(&'static str),
    #[error("invalid reason code: {0}")]
    InvalidReasonCode(String),
    #[error("invalid completion policy: {0}")]
    InvalidCompletionPolicy(&'static str),
    #[error("completion policy violation: {0}")]
    PolicyViolation(&'static str),
    #[error("invalid run context: {0}")]
    InvalidRunContext(&'static str),
    #[error("invalid source references: {0}")]
    InvalidSourceReferences(&'static str),
    #[error("invalid model output references: {0}")]
    InvalidModelOutputReferences(&'static str),
    #[error("invalid verified-empty evidence: {0}")]
    InvalidVerifiedEmptyEvidence(&'static str),
    #[error("invalid prepared facts: {0}")]
    InvalidPreparedFacts(&'static str),
    #[error("unknown monitor kind: {0}")]
    InvalidMonitorKind(String),
}

pub type Result<T> = std::result::Result<T, PushJobError>;

#[cfg(test)]
mod tests;
