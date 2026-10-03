//! Closed Catalog8 immutable investment evaluation borrower. Production qualification is absent.
use super::paper_v6;
use crate::database::{DatabaseConnectionAuthority, DatabaseManager};
use diesel::SqliteConnection;

pub(crate) use paper_v6::{
    PaperCatalog6Error as InvestmentCatalog8Error,
    PaperCatalog6ReadbackError as InvestmentCatalog8ReadbackError,
    PaperCatalog6TransactionError as InvestmentCatalog8TransactionError, VerifiedCatalog8,
};

pub(crate) struct InvestmentCatalog8Session<'db>(paper_v6::PaperCatalog6Session<'db>);
pub(crate) fn investment_catalog8_session(
    db: &DatabaseManager,
) -> Result<InvestmentCatalog8Session<'_>, InvestmentCatalog8Error> {
    paper_v6::investment_catalog8_session(db).map(InvestmentCatalog8Session)
}
impl InvestmentCatalog8Session<'_> {
    pub(crate) fn with_immediate_catalog8<T, E>(
        &mut self,
        operation: impl for<'s> FnOnce(
            &mut SqliteConnection,
            &DatabaseConnectionAuthority,
            &VerifiedCatalog8<'s>,
        ) -> Result<T, E>,
        tail: impl for<'s> FnMut(
            &mut SqliteConnection,
            &DatabaseConnectionAuthority,
            &VerifiedCatalog8<'s>,
            &T,
        ) -> Result<(), E>,
    ) -> Result<T, InvestmentCatalog8TransactionError<E>> {
        self.0.with_immediate_catalog8(operation, tail)
    }
    pub(crate) fn with_readonly_catalog8<T, E>(
        &mut self,
        operation: impl for<'s> FnOnce(&mut SqliteConnection, &VerifiedCatalog8<'s>) -> Result<T, E>,
        tail: impl for<'s> FnMut(&mut SqliteConnection, &VerifiedCatalog8<'s>, &T) -> Result<(), E>,
    ) -> Result<T, InvestmentCatalog8ReadbackError<E>> {
        self.0.with_readonly_catalog8(operation, tail)
    }
}
#[cfg(test)]
pub(crate) fn migrate_catalog8_for_isolated_test(
    db: &DatabaseManager,
) -> Result<(), InvestmentCatalog8TransactionError<InvestmentCatalog8Error>> {
    paper_v6::migrate_catalog8_for_isolated_test(db)
}
