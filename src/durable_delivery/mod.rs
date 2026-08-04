//! BR-192 durable counted-delivery dark core.
//!
//! This module owns the physically isolated SQLite state machine, reservation
//! ledger, fencing and frozen audit payloads. It deliberately does not wire a
//! production provider, renderer or sink; production activation is an
//! all-or-nothing follow-up after every counted caller has migrated.

mod coordinator;
mod model;
mod schema;

pub use coordinator::DurableDeliveryCoordinator;
pub use model::{
    compiled_policy_catalog, AuthoritativeDeliveryRequest, AuthoritativeSink,
    AuthoritativeSinkPort, AuthoritativeSinkResult, AuthorityWatermark, CooldownScope,
    CoordinatorConfig, DecisionState, DeliveryEnvelope, DeliverySubKind, DurableDeliveryError,
    ImmutableAppendPort, ManualDisposition, ManualResolutionCommand, PolicyRow, PrepareOutcome,
    PushKind, ReconcileSummary, Result, ResumeOutcome, ReviewTaskOccurrenceEvidence,
    ReviewTerminalReplayAttempt, ReviewTerminalReplayCompletion,
    ReviewTerminalReplayCompletionCanonical, ReviewTerminalReplayCompletionState,
    ReviewTerminalReplayInput, ReviewTerminalReplayStartCanonical, ScheduleHydration,
    ScheduleHydrationState, StoreEnvironment, TaskBinding, TypedReceipt, TypedRejection,
    TypedUncertainty, WindowMode, DAILY_BUDGET_LIMIT, ENVELOPE_VERSION, POLICY_VERSION,
};

#[cfg(test)]
mod tests;
