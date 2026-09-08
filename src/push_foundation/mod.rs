//! Additive push-foundation persistence. No production database is selected or migrated here.

mod activation;
mod activation_authorization;
mod activation_codec;
mod activation_deployment;
mod activation_facts;
#[cfg(unix)]
mod activation_fence;
#[cfg(unix)]
mod activation_fence_ipc;
#[cfg(unix)]
mod activation_fence_store;
#[cfg(unix)]
mod activation_generic_effect;
mod activation_owner;
mod activation_readiness;
mod activation_store;
mod activation_transaction;
mod business_finalizer;
mod dedicated_transport;
mod generic_transport;
mod intent_store;
mod migration;
mod operational_readiness;
mod phase_scheduler;
mod readiness_probe;
mod readiness_recovery;
mod readiness_recovery_codec;
mod readiness_snapshot;
mod readiness_snapshot_codec;
mod readiness_sqlite_io;
mod readiness_store;
mod readiness_store_schema;
mod reconciler;
mod terminal_authority;

pub use activation::{
    ActivationManifest, DesiredActivationState, PromotionAction, PromotionJournalEntry,
};
pub use activation_facts::{ActivationReconciliation, RawActivationFacts, UnitActivationFacts};
pub use activation_store::{inspect_raw_activation_facts, ActivationInspectError};
pub use intent_store::{
    BusinessIntentStore, InitialDecisionKind, InitialIntentDraft, InitialIntentIdentity,
    InitialIntentOutcome, IntentSnapshot, IntentState, IntentStoreError, IntentTransitionCommand,
    LeaseAction, LeaseOwnerId, TransitionActor, TransitionOutcome, TransitionReceipt,
};
#[cfg(test)]
pub(crate) use intent_store::{InitialCommitFault, TransitionFault};
pub use migration::{FoundationMigrationError, FoundationSchemaMigration, MigrationReceipt};
pub use terminal_authority::TerminalTemplateBinding;

#[cfg(test)]
mod activation_authorization_tests;
#[cfg(test)]
mod activation_deployment_tests;
#[cfg(test)]
mod activation_facts_tests;
#[cfg(all(test, unix))]
mod activation_fence_process_tests;
#[cfg(all(test, unix))]
mod activation_fence_tests;
#[cfg(all(test, unix))]
mod activation_generic_effect_tests;
#[cfg(all(test, unix))]
mod activation_generic_process_tests;
#[cfg(test)]
mod activation_owner_tests;
#[cfg(test)]
mod activation_readiness_tests;
#[cfg(test)]
mod activation_transaction_tests;
#[cfg(test)]
mod business_finalizer_tests;
#[cfg(test)]
mod dedicated_transport_tests;
#[cfg(test)]
mod generic_transport_tests;
#[cfg(test)]
mod operational_readiness_tests;
#[cfg(test)]
mod phase_scheduler_tests;
#[cfg(test)]
mod readiness_probe_tests;
#[cfg(test)]
mod readiness_recovery_codec_tests;
#[cfg(test)]
mod readiness_recovery_tests;
#[cfg(test)]
mod readiness_snapshot_codec_tests;
#[cfg(test)]
mod readiness_snapshot_tests;
#[cfg(test)]
mod readiness_store_schema_tests;
#[cfg(test)]
mod readiness_store_tests;
#[cfg(test)]
mod reconciler_tests;
#[cfg(test)]
mod terminal_authority_tests;
#[cfg(test)]
mod tests;
