//! Application-level push contracts. This module is pure and has no runtime wiring.

mod delivery;
mod identity;
mod policy;

pub use delivery::{
    classify_durable_state, AttemptId, AuthorityClass, ChannelId, CompatId,
    CompatibilityEvidenceRef, CompletionEligibility, DecisionId, DeliveryAuthority, DeliveryResult,
    DeliveryResultView, DurableSchemaVersion, DurableStateProjection, TemplateId, TemplateVersion,
    TerminalDisposition, TerminalRefId, VerifiedTerminalRef, WeakOutcome, WeakOutcomeKind,
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

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum PushJobError {
    #[error("invalid {field}: {reason}")]
    InvalidText {
        field: &'static str,
        reason: &'static str,
    },
    #[error("invalid business date: {0}")]
    InvalidBusinessDate(String),
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
}

pub type Result<T> = std::result::Result<T, PushJobError>;

#[cfg(test)]
mod tests;
