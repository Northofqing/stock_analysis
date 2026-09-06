//! Application-level push contracts. This module is pure and has no runtime wiring.

mod delivery;
mod identity;

pub use delivery::{
    classify_durable_state, AttemptId, AuthorityClass, ChannelId, CompatId,
    CompatibilityEvidenceRef, CompletionEligibility, DecisionId, DeliveryAuthority, DeliveryResult,
    DeliveryResultView, DurableSchemaVersion, DurableStateProjection, ReasonCode, TemplateId,
    TemplateVersion, TerminalDisposition, TerminalRefId, VerifiedTerminalRef, WeakOutcome,
    WeakOutcomeKind,
};
pub use identity::{
    derive_intent_id, derive_occurrence_id, derive_schedule_occurrence_id, AudienceId,
    BusinessDate, CalendarId, CompletionOwnerId, IntentId, IntentIdentityMaterial, Namespace,
    OccurrenceFamily, OccurrenceId, OccurrenceIdentityMaterial, OccurrenceKey, ProducerId, RunId,
    ScheduleOccurrenceId, ScheduleOccurrenceIdentityMaterial, ScheduleOrTriggerId, Sha256Digest,
    SourceContractId, SourceContractVersion, SubjectId, SubjectValue, UnitId, UtcMicros,
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
}

pub type Result<T> = std::result::Result<T, PushJobError>;

#[cfg(test)]
mod tests;
