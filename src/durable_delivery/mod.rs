//! BR-192 durable counted-delivery dark core.
//!
//! This module owns the physically isolated SQLite state machine, reservation
//! ledger, fencing and frozen audit payloads. It deliberately does not wire a
//! production provider, renderer or sink; production activation is an
//! all-or-nothing follow-up after every counted caller has migrated.

mod coordinator;
mod correlation;
mod model;
mod schema;
mod schema_g5b_cohort;
mod schema_p05_unit;
mod schema_p05_unit_runtime;

pub use coordinator::{
    CandidateBoardCardObservationV1, CandidateBoardCardObservationV2, CandidateBoardCardTerminalV1,
    CandidateBoardSourceLinkV1, CandidateBoardSourceRowV2, DurableDeliveryCoordinator,
    G5bCountedDayFactV1, G5bCountedDaySnapshotV1, G5bCountedObservationV1, G5bCountedTerminalV1,
    P05ChildReceiptObservation, P05InvalidationRenderFacts, P05NonAcceptedTerminal,
};
pub(crate) use coordinator::{
    G5bConfiguredAnalysis, G5bDaySession, G5bEmptyDayInspection, G5bEmptyPending, G5bSnapshotKind,
    P05ObservedDraftInput, P05ObservedSourceBytes, P05PredictionStart, P05UnitReceiptObservation,
    PreparedG5bArtifact, StoredP05Draft, StoredP05Intent, VerifiedG5bEmptySeal,
    VerifiedG5bModelBundle, VerifiedStoredG5bCohort,
};
pub use correlation::P01OriginProducer;
pub use model::{
    compiled_policy_catalog, AuthoritativeDeliveryRequest, AuthoritativeSink,
    AuthoritativeSinkPort, AuthoritativeSinkResult, AuthorityWatermark,
    BusinessDateOnceClaimEvidence, CooldownScope, CoordinatorConfig, DecisionState,
    DeliveryEnvelope, DeliverySubKind, DurableDeliveryError, ExactOccurrenceOwner,
    ImmutableAppendPort, ManualDisposition, ManualResolutionCommand, PolicyRow, PrepareOutcome,
    PushKind, ReconcileSummary, Result, ResumeOutcome, ReviewTaskOccurrenceEvidence,
    ReviewTerminalReplayAttempt, ReviewTerminalReplayCompletion,
    ReviewTerminalReplayCompletionCanonical, ReviewTerminalReplayCompletionState,
    ReviewTerminalReplayInput, ReviewTerminalReplayStartCanonical, ScheduleHydration,
    ScheduleHydrationState, StoreEnvironment, TaskBinding, TypedReceipt, TypedRejection,
    TypedUncertainty, WindowMode, DAILY_BUDGET_LIMIT, ENVELOPE_VERSION, POLICY_VERSION,
};
pub(crate) use model::{
    FoundationDeliveryBinding, FoundationTerminalDisposition, FoundationTerminalQuery,
    FoundationTerminalRecord, P01DedicatedTerminalQuery, P01DedicatedTerminalRecord,
};
pub(crate) use schema::SCHEMA_VERSION as DURABLE_SCHEMA_VERSION;

#[cfg(test)]
mod tests;
