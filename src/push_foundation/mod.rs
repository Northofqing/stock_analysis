//! Additive push-foundation persistence. No production database is selected or migrated here.

mod migration;

pub use migration::{FoundationMigrationError, FoundationSchemaMigration, MigrationReceipt};

#[cfg(test)]
mod tests;
