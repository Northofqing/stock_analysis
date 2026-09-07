//! Additive push-foundation persistence. No production database is selected or migrated here.

mod business_finalizer;
mod dedicated_transport;
mod generic_transport;
mod intent_store;
mod migration;
mod reconciler;
mod terminal_authority;

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
mod business_finalizer_tests;
#[cfg(test)]
mod dedicated_transport_tests;
#[cfg(test)]
mod generic_transport_tests;
#[cfg(test)]
mod reconciler_tests;
#[cfg(test)]
mod terminal_authority_tests;
#[cfg(test)]
mod tests;
