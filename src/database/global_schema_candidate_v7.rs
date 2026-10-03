//! Closed Catalog7 observation borrower. Production qualification is absent.
use super::paper_v6;
use crate::database::{DatabaseConnectionAuthority, DatabaseManager};
use diesel::SqliteConnection;

pub(crate) use paper_v6::{
    PaperCatalog6Error as CandidateCatalog7Error,
    PaperCatalog6ReadbackError as CandidateCatalog7ReadbackError,
    PaperCatalog6TransactionError as CandidateCatalog7TransactionError, VerifiedCatalog7,
};

pub(crate) struct CandidateCatalog7Session<'db>(paper_v6::PaperCatalog6Session<'db>);
pub(crate) fn candidate_catalog7_session(
    db: &DatabaseManager,
) -> Result<CandidateCatalog7Session<'_>, CandidateCatalog7Error> {
    paper_v6::candidate_catalog7_session(db).map(CandidateCatalog7Session)
}
impl CandidateCatalog7Session<'_> {
    pub(crate) fn with_immediate_catalog7<T, E>(
        &mut self,
        operation: impl for<'s> FnOnce(
            &mut SqliteConnection,
            &DatabaseConnectionAuthority,
            &VerifiedCatalog7<'s>,
        ) -> Result<T, E>,
        tail: impl for<'s> FnMut(
            &mut SqliteConnection,
            &DatabaseConnectionAuthority,
            &VerifiedCatalog7<'s>,
            &T,
        ) -> Result<(), E>,
    ) -> Result<T, CandidateCatalog7TransactionError<E>> {
        self.0.with_immediate_catalog7(operation, tail)
    }
    pub(crate) fn with_readonly_catalog7<T, E>(
        &mut self,
        operation: impl for<'s> FnOnce(&mut SqliteConnection, &VerifiedCatalog7<'s>) -> Result<T, E>,
        tail: impl for<'s> FnMut(&mut SqliteConnection, &VerifiedCatalog7<'s>, &T) -> Result<(), E>,
    ) -> Result<T, CandidateCatalog7ReadbackError<E>> {
        self.0.with_readonly_catalog7(operation, tail)
    }
}
#[cfg(test)]
pub(crate) fn migrate_catalog7_for_isolated_test(
    db: &DatabaseManager,
) -> Result<(), CandidateCatalog7TransactionError<CandidateCatalog7Error>> {
    paper_v6::migrate_catalog7_for_isolated_test(db)
}
