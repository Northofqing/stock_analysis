//! Additive push-foundation persistence. No production database is selected or migrated here.

mod intent_store;
mod migration;

pub use intent_store::{
    BusinessIntentStore, InitialDecisionKind, InitialIntentDraft, InitialIntentIdentity,
    InitialIntentOutcome, IntentSnapshot, IntentState, IntentStoreError, IntentTransitionCommand,
    LeaseAction, LeaseOwnerId, TransitionActor, TransitionOutcome, TransitionReceipt,
};
#[cfg(test)]
pub(crate) use intent_store::{InitialCommitFault, TransitionFault};
pub use migration::{FoundationMigrationError, FoundationSchemaMigration, MigrationReceipt};

#[cfg(test)]
mod tests;
