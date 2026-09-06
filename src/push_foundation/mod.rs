//! Additive push-foundation persistence. No production database is selected or migrated here.

mod intent_store;
mod migration;

pub use intent_store::{
    BusinessIntentStore, InitialDecisionKind, InitialIntentDraft, InitialIntentIdentity,
    InitialIntentOutcome, IntentSnapshot, IntentState, IntentStoreError,
};
pub use migration::{FoundationMigrationError, FoundationSchemaMigration, MigrationReceipt};

#[cfg(test)]
mod tests;
