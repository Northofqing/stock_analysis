//! CatalogV6 scoped borrower. Production qualification is deliberately absent.
//! A copied registration token is object evidence, never connection identity.
use super::*;
use crate::database::global_schema_catalog_v1::{
    diesel_capture::{self as capture, CopyWork},
    SameRuntimeCatalogReferences,
};
use crate::database::{AttestedAttributionCheckout, DatabaseConnectionAuthority, DatabaseManager};
use diesel::{Connection as DieselConnection, RunQueryDsl, SqliteConnection};
use std::{
    cell::RefCell,
    marker::PhantomData,
    sync::{Arc, OnceLock},
};

#[derive(Debug, thiserror::Error)]
pub(crate) enum PaperCatalog6Error {
    #[error("Catalog6RequalificationRequired")]
    Catalog6RequalificationRequired,
    #[error("Catalog7RequalificationRequired")]
    Catalog7RequalificationRequired,
    #[error("Catalog8RequalificationRequired")]
    Catalog8RequalificationRequired,
    #[error("borrowed SQLite connection instance differs")]
    ConnectionInstanceMismatch,
    #[error("catalog copy-work budget exceeded")]
    CopyBudgetExceeded,
    #[error("SQLite snapshot failed")]
    Sql,
    #[error("catalog differs: {0}")]
    Catalog(String),
    #[error("retained database authority differs")]
    Authority,
    #[error("retained namespace differs")]
    Namespace,
    #[cfg(test)]
    #[error("TEST_CODE interrupted catalog boundary")]
    InjectedFailure,
}
impl From<diesel::result::Error> for PaperCatalog6Error {
    fn from(_: diesel::result::Error) -> Self {
        Self::Sql
    }
}
#[derive(Debug)]
pub(crate) enum PaperCatalog6TransactionError<E> {
    BeforeCommit(PaperCatalog6Error),
    Consumer(E),
    /// The original transaction may have committed. No automatic replay.
    CommitOutcomeUnknown(PaperCatalog6Error),
    CommittedConsumerOutcomeUnknown(E),
}
impl<E> From<diesel::result::Error> for PaperCatalog6TransactionError<E> {
    fn from(_: diesel::result::Error) -> Self {
        Self::BeforeCommit(PaperCatalog6Error::Sql)
    }
}
#[derive(Debug)]
pub(crate) enum PaperCatalog6ReadbackError<E> {
    ObservationUnavailable(PaperCatalog6Error),
    Consumer(E),
}
impl<E> From<diesel::result::Error> for PaperCatalog6ReadbackError<E> {
    fn from(_: diesel::result::Error) -> Self {
        Self::ObservationUnavailable(PaperCatalog6Error::Sql)
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum ClosedGeneration {
    Six,
    Seven,
    Eight,
}
impl ClosedGeneration {
    fn number(self) -> i64 {
        match self {
            Self::Six => 6,
            Self::Seven => 7,
            Self::Eight => 8,
        }
    }
    fn error(self) -> PaperCatalog6Error {
        match self {
            Self::Six => PaperCatalog6Error::Catalog6RequalificationRequired,
            Self::Seven => PaperCatalog6Error::Catalog7RequalificationRequired,
            Self::Eight => PaperCatalog6Error::Catalog8RequalificationRequired,
        }
    }
}

/// Distinct witnesses prevent a Catalog6 consumer from borrowing Catalog7.
pub(crate) struct VerifiedCatalog6<'s>(&'s ClosedCatalogProof<'s>);
pub(crate) struct VerifiedCatalog7<'s>(&'s ClosedCatalogProof<'s>);
impl<'s> std::ops::Deref for VerifiedCatalog6<'s> {
    type Target = ClosedCatalogProof<'s>;
    fn deref(&self) -> &Self::Target {
        self.0
    }
}
impl<'s> std::ops::Deref for VerifiedCatalog7<'s> {
    type Target = ClosedCatalogProof<'s>;
    fn deref(&self) -> &Self::Target {
        self.0
    }
}

pub(crate) struct VerifiedCatalog8<'s>(&'s ClosedCatalogProof<'s>);
impl<'s> std::ops::Deref for VerifiedCatalog8<'s> {
    type Target = ClosedCatalogProof<'s>;
    fn deref(&self) -> &Self::Target {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Purpose {
    Writer,
    Reader,
}
struct LoanScope;
/// Issued from the actual callback-local borrowed instance, never a path/token.
pub(in crate::database) struct CatalogLoan<'s> {
    instance: usize,
    _scope: PhantomData<&'s LoanScope>,
}
impl<'s> CatalogLoan<'s> {
    fn new(conn: &mut SqliteConnection, _scope: &'s LoanScope) -> Self {
        Self {
            instance: conn as *mut SqliteConnection as usize,
            _scope: PhantomData,
        }
    }
    pub(in crate::database) fn require_instance(
        &self,
        conn: &mut SqliteConnection,
    ) -> Result<(), PaperCatalog6Error> {
        if self.instance != conn as *mut SqliteConnection as usize {
            Err(PaperCatalog6Error::ConnectionInstanceMismatch)
        } else {
            Ok(())
        }
    }
}
/// Non-Clone, non-deserializable callback-local witness. Writer and retained
/// reader receive separately issued loans. It cannot escape the HRTB callback.
pub(crate) struct ClosedCatalogProof<'s> {
    loan: CatalogLoan<'s>,
    authority: &'s DatabaseConnectionAuthority,
    source: &'s Arc<crate::database::DescriptorSqliteSource>,
    namespace: &'s PinnedNamespace,
    references: &'static SameRuntimeCatalogReferences,
    work: &'s RefCell<CopyWork>,
    purpose: Purpose,
    generation: ClosedGeneration,
}
impl VerifiedCatalog7<'_> {
    pub(crate) fn require_observation_instance(
        &self,
        conn: &mut SqliteConnection,
    ) -> Result<(), PaperCatalog6Error> {
        self.0.loan.require_instance(conn)
    }
}
impl VerifiedCatalog8<'_> {
    pub(crate) fn require_decision_instance(
        &self,
        conn: &mut SqliteConnection,
    ) -> Result<(), PaperCatalog6Error> {
        self.0.loan.require_instance(conn)
    }
}
impl ClosedCatalogProof<'_> {
    /// Original opaque object authority of this actual borrowed snapshot.
    pub(crate) fn connection_authority(&self) -> &DatabaseConnectionAuthority {
        self.authority
    }
    pub(crate) fn validate_on(
        &self,
        conn: &mut SqliteConnection,
    ) -> Result<(), PaperCatalog6Error> {
        self.loan.require_instance(conn)?; // zero SQL / copying on wrong instance
        let actual = crate::database::registered_descriptor_connection_authority(self.source, conn)
            .map_err(|_| PaperCatalog6Error::Authority)?;
        if &actual != self.authority {
            return Err(PaperCatalog6Error::Authority);
        }
        self.namespace
            .validate_unchanged()
            .map_err(|_| PaperCatalog6Error::Namespace)?;
        controls(conn, self.purpose)?;
        let mut work = self
            .work
            .try_borrow_mut()
            .map_err(|_| PaperCatalog6Error::CopyBudgetExceeded)?;
        capture::preflight_pair(conn, &mut work)?;
        let tmp = capture::temporary_catalog(&self.loan, conn, &mut work)?;
        if &tmp != temp_reference()? {
            return Err(PaperCatalog6Error::Catalog(
                "unexpected TEMP catalog".into(),
            ));
        }
        let snapshot = capture::capture(&self.loan, conn, self.references.mode(), &mut work)?;
        match capture::classify_bounded(&snapshot, self.references, &mut work)? {
            DatabaseHalfDiagnostic::AmendedDatabaseHalf(e)
                if e.identity.user_version == self.generation.number() => {}
            _ => return Err(self.generation.error()),
        }
        // The original financial validator reads this same SQL snapshot. Its
        // independent history limits are not charged as catalog-copy bytes.
        match self.generation {
            ClosedGeneration::Six => crate::trading::paper_book_v2_execution::verify_rows_on(conn),
            ClosedGeneration::Seven => {
                crate::trading::paper_book_v2_execution::verify_rows_on_catalog7(conn)
            }
            ClosedGeneration::Eight => {
                crate::trading::paper_book_v2_execution::verify_rows_on_catalog8(conn)
            }
        }
        .map_err(|_| PaperCatalog6Error::Catalog("financial replay failed".into()))?;
        self.loan.require_instance(conn)?;
        self.namespace
            .validate_unchanged()
            .map_err(|_| PaperCatalog6Error::Namespace)?;
        let terminal =
            crate::database::registered_descriptor_connection_authority(self.source, conn)
                .map_err(|_| PaperCatalog6Error::Authority)?;
        if terminal != actual {
            return Err(PaperCatalog6Error::Authority);
        }
        Ok(())
    }
    fn hook_free_check(&self, conn: &mut SqliteConnection) -> Result<(), PaperCatalog6Error> {
        self.loan.require_instance(conn)?;
        self.namespace
            .validate_unchanged()
            .map_err(|_| PaperCatalog6Error::Namespace)?;
        if crate::database::registered_descriptor_connection_authority(self.source, conn)
            .map_err(|_| PaperCatalog6Error::Authority)?
            != *self.authority
        {
            return Err(PaperCatalog6Error::Authority);
        }
        controls(conn, self.purpose)?;
        self.loan.require_instance(conn)
    }
}
fn controls(conn: &mut SqliteConnection, purpose: Purpose) -> Result<(), PaperCatalog6Error> {
    for (sql, expected) in [
        ("SELECT foreign_keys AS value FROM pragma_foreign_keys", 1),
        ("SELECT synchronous AS value FROM pragma_synchronous", 2),
        (
            "SELECT query_only AS value FROM pragma_query_only",
            if purpose == Purpose::Reader { 1 } else { 0 },
        ),
    ] {
        if capture::int(conn, sql)? != expected {
            return Err(PaperCatalog6Error::Authority);
        }
    }
    #[derive(diesel::QueryableByName)]
    struct Mode {
        #[diesel(sql_type=diesel::sql_types::Text)]
        value: String,
    }
    if diesel::sql_query("SELECT journal_mode AS value FROM pragma_journal_mode")
        .get_result::<Mode>(conn)?
        .value
        != "wal"
    {
        return Err(PaperCatalog6Error::Authority);
    }
    if capture::int(
        conn,
        "SELECT COUNT(*) AS value FROM pragma_foreign_key_check",
    )? != 0
        || capture::int(
            conn,
            "SELECT COUNT(*) AS value FROM pragma_integrity_check WHERE integrity_check!='ok'",
        )? != 0
    {
        return Err(PaperCatalog6Error::Authority);
    }
    Ok(())
}
fn refs() -> Result<&'static SameRuntimeCatalogReferences, PaperCatalog6Error> {
    static REFERENCES: OnceLock<Result<SameRuntimeCatalogReferences, String>> = OnceLock::new();
    #[cfg(test)]
    let mode = GlobalSchemaCatalogMode::Test;
    #[cfg(not(test))]
    let mode = GlobalSchemaCatalogMode::Production;
    REFERENCES
        .get_or_init(|| build_same_runtime_catalog_references(mode).map_err(|e| e.to_string()))
        .as_ref()
        .map_err(|e| PaperCatalog6Error::Catalog(e.clone()))
}
fn temp_reference() -> Result<&'static capture::TempCatalog, PaperCatalog6Error> {
    static TEMP: OnceLock<Result<capture::TempCatalog, String>> = OnceLock::new();
    TEMP.get_or_init(|| {
        let mut conn = SqliteConnection::establish(":memory:").map_err(|e| e.to_string())?;
        crate::database::install_connection_attestation_token(
            &mut conn,
            "reference-only-not-registered",
        )
        .map_err(|e| e.to_string())?;
        capture::temporary_reference(&mut conn, &mut CopyWork::new()).map_err(|e| e.to_string())
    })
    .as_ref()
    .map_err(|e| PaperCatalog6Error::Catalog(e.clone()))
}
fn paths(db: &DatabaseManager) -> Result<ModeBoundPaths, PaperCatalog6Error> {
    #[cfg(not(test))]
    {
        let _ = db;
        Err(PaperCatalog6Error::Catalog6RequalificationRequired)
    }
    #[cfg(test)]
    {
        let origin = db
            .isolated_p05_consumer_origin
            .as_ref()
            .ok_or(PaperCatalog6Error::Catalog6RequalificationRequired)?;
        origin
            .validate()
            .map_err(|_| PaperCatalog6Error::Namespace)?;
        let root = origin
            .path
            .parent()
            .ok_or(PaperCatalog6Error::Namespace)?
            .to_owned();
        let mut paths =
            ModeBoundPaths::isolated_test(&root).map_err(|_| PaperCatalog6Error::Namespace)?;
        paths.database = origin.path.clone();
        paths.wal = sidecar_path(&paths.database, "-wal");
        paths.shm = sidecar_path(&paths.database, "-shm");
        paths
            .validate_mode_binding()
            .map_err(|_| PaperCatalog6Error::Namespace)?;
        Ok(paths)
    }
}
/// Field order releases the checkout before the shared maintenance lease.
pub(crate) struct PaperCatalog6Session<'db> {
    checkout: AttestedAttributionCheckout,
    namespace: PinnedNamespace,
    _lease: GlobalSchemaMaintenanceLease,
    _db: &'db DatabaseManager,
    references: &'static SameRuntimeCatalogReferences,
    work: RefCell<CopyWork>,
    generation: ClosedGeneration,
}
pub(crate) fn paper_catalog6_session(
    db: &DatabaseManager,
) -> Result<PaperCatalog6Session<'_>, PaperCatalog6Error> {
    let paths = paths(db)?; // Production refuses before any lock / checkout / SQL.
    let namespace = PinnedNamespace::open(&paths).map_err(|_| PaperCatalog6Error::Namespace)?;
    let lease = GlobalSchemaMaintenanceLease::acquire_shared(&paths, &namespace)
        .map_err(|_| PaperCatalog6Error::Namespace)?;
    let checkout = db
        .attribution_checkout()
        .map_err(|_| PaperCatalog6Error::Authority)?;
    Ok(PaperCatalog6Session {
        checkout,
        namespace,
        _lease: lease,
        _db: db,
        references: refs()?,
        work: RefCell::new(CopyWork::new()),
        generation: ClosedGeneration::Six,
    })
}
pub(super) fn candidate_catalog7_session(
    db: &DatabaseManager,
) -> Result<PaperCatalog6Session<'_>, PaperCatalog6Error> {
    let mut session = paper_catalog6_session(db).map_err(|e| match e {
        PaperCatalog6Error::Catalog6RequalificationRequired => {
            PaperCatalog6Error::Catalog7RequalificationRequired
        }
        e => e,
    })?;
    session.generation = ClosedGeneration::Seven;
    Ok(session)
}
pub(super) fn investment_catalog8_session(
    db: &DatabaseManager,
) -> Result<PaperCatalog6Session<'_>, PaperCatalog6Error> {
    let mut session = paper_catalog6_session(db).map_err(|e| match e {
        PaperCatalog6Error::Catalog6RequalificationRequired => {
            PaperCatalog6Error::Catalog8RequalificationRequired
        }
        e => e,
    })?;
    session.generation = ClosedGeneration::Eight;
    Ok(session)
}

impl PaperCatalog6Session<'_> {
    pub(crate) fn with_immediate_catalog6<T, E>(
        &mut self,
        operation: impl for<'s> FnOnce(
            &mut SqliteConnection,
            &DatabaseConnectionAuthority,
            &VerifiedCatalog6<'s>,
        ) -> Result<T, E>,
        mut tail: impl for<'s> FnMut(
            &mut SqliteConnection,
            &DatabaseConnectionAuthority,
            &VerifiedCatalog6<'s>,
            &T,
        ) -> Result<(), E>,
    ) -> Result<T, PaperCatalog6TransactionError<E>> {
        if self.generation != ClosedGeneration::Six {
            return Err(PaperCatalog6TransactionError::BeforeCommit(
                ClosedGeneration::Six.error(),
            ));
        }
        self.with_immediate_closed(
            |c, a, p| operation(c, a, &VerifiedCatalog6(p)),
            |c, a, p, v| tail(c, a, &VerifiedCatalog6(p), v),
        )
    }
    pub(crate) fn with_readonly_catalog6<T, E>(
        &mut self,
        operation: impl for<'s> FnOnce(&mut SqliteConnection, &VerifiedCatalog6<'s>) -> Result<T, E>,
        mut tail: impl for<'s> FnMut(&mut SqliteConnection, &VerifiedCatalog6<'s>, &T) -> Result<(), E>,
    ) -> Result<T, PaperCatalog6ReadbackError<E>> {
        if self.generation != ClosedGeneration::Six {
            return Err(PaperCatalog6ReadbackError::ObservationUnavailable(
                ClosedGeneration::Six.error(),
            ));
        }
        self.with_readonly_closed(
            |c, p| operation(c, &VerifiedCatalog6(p)),
            |c, p, v| tail(c, &VerifiedCatalog6(p), v),
        )
    }
    pub(super) fn with_immediate_catalog7<T, E>(
        &mut self,
        operation: impl for<'s> FnOnce(
            &mut SqliteConnection,
            &DatabaseConnectionAuthority,
            &VerifiedCatalog7<'s>,
        ) -> Result<T, E>,
        mut tail: impl for<'s> FnMut(
            &mut SqliteConnection,
            &DatabaseConnectionAuthority,
            &VerifiedCatalog7<'s>,
            &T,
        ) -> Result<(), E>,
    ) -> Result<T, PaperCatalog6TransactionError<E>> {
        if self.generation != ClosedGeneration::Seven {
            return Err(PaperCatalog6TransactionError::BeforeCommit(
                ClosedGeneration::Seven.error(),
            ));
        }
        self.with_immediate_closed(
            |c, a, p| operation(c, a, &VerifiedCatalog7(p)),
            |c, a, p, v| tail(c, a, &VerifiedCatalog7(p), v),
        )
    }
    pub(super) fn with_readonly_catalog7<T, E>(
        &mut self,
        operation: impl for<'s> FnOnce(&mut SqliteConnection, &VerifiedCatalog7<'s>) -> Result<T, E>,
        mut tail: impl for<'s> FnMut(&mut SqliteConnection, &VerifiedCatalog7<'s>, &T) -> Result<(), E>,
    ) -> Result<T, PaperCatalog6ReadbackError<E>> {
        if self.generation != ClosedGeneration::Seven {
            return Err(PaperCatalog6ReadbackError::ObservationUnavailable(
                ClosedGeneration::Seven.error(),
            ));
        }
        self.with_readonly_closed(
            |c, p| operation(c, &VerifiedCatalog7(p)),
            |c, p, v| tail(c, &VerifiedCatalog7(p), v),
        )
    }
    pub(super) fn with_immediate_catalog8<T, E>(
        &mut self,
        operation: impl for<'s> FnOnce(
            &mut SqliteConnection,
            &DatabaseConnectionAuthority,
            &VerifiedCatalog8<'s>,
        ) -> Result<T, E>,
        mut tail: impl for<'s> FnMut(
            &mut SqliteConnection,
            &DatabaseConnectionAuthority,
            &VerifiedCatalog8<'s>,
            &T,
        ) -> Result<(), E>,
    ) -> Result<T, PaperCatalog6TransactionError<E>> {
        if self.generation != ClosedGeneration::Eight {
            return Err(PaperCatalog6TransactionError::BeforeCommit(
                ClosedGeneration::Eight.error(),
            ));
        }
        self.with_immediate_closed(
            |c, a, p| operation(c, a, &VerifiedCatalog8(p)),
            |c, a, p, v| tail(c, a, &VerifiedCatalog8(p), v),
        )
    }
    pub(super) fn with_readonly_catalog8<T, E>(
        &mut self,
        operation: impl for<'s> FnOnce(&mut SqliteConnection, &VerifiedCatalog8<'s>) -> Result<T, E>,
        mut tail: impl for<'s> FnMut(&mut SqliteConnection, &VerifiedCatalog8<'s>, &T) -> Result<(), E>,
    ) -> Result<T, PaperCatalog6ReadbackError<E>> {
        if self.generation != ClosedGeneration::Eight {
            return Err(PaperCatalog6ReadbackError::ObservationUnavailable(
                ClosedGeneration::Eight.error(),
            ));
        }
        self.with_readonly_closed(
            |c, p| operation(c, &VerifiedCatalog8(p)),
            |c, p, v| tail(c, &VerifiedCatalog8(p), v),
        )
    }
    pub(crate) fn with_committed_readback<T, E>(
        &mut self,
        expected: &DatabaseConnectionAuthority,
        operation: impl for<'s> FnOnce(&mut SqliteConnection, &VerifiedCatalog6<'s>) -> Result<T, E>,
        mut tail: impl for<'s> FnMut(&mut SqliteConnection, &VerifiedCatalog6<'s>, &T) -> Result<(), E>,
    ) -> Result<T, PaperCatalog6ReadbackError<E>> {
        if self.generation != ClosedGeneration::Six {
            return Err(PaperCatalog6ReadbackError::ObservationUnavailable(
                ClosedGeneration::Six.error(),
            ));
        }
        self.with_committed_readback_closed(
            expected,
            |c, p| operation(c, &VerifiedCatalog6(p)),
            |c, p, v| tail(c, &VerifiedCatalog6(p), v),
        )
    }
    /// The mandatory tail is called after every wrapper hook and full gate,
    /// and again in the independent post-COMMIT reader. It must check the
    /// consumer's exact binding and (for new commands) real transaction time.
    fn with_immediate_closed<T, E>(
        &mut self,
        operation: impl for<'s> FnOnce(
            &mut SqliteConnection,
            &DatabaseConnectionAuthority,
            &ClosedCatalogProof<'s>,
        ) -> Result<T, E>,
        mut tail: impl for<'s> FnMut(
            &mut SqliteConnection,
            &DatabaseConnectionAuthority,
            &ClosedCatalogProof<'s>,
            &T,
        ) -> Result<(), E>,
    ) -> Result<T, PaperCatalog6TransactionError<E>> {
        let source = Arc::clone(&self.checkout.source);
        let namespace = &self.namespace;
        let references = self.references;
        let work = &self.work;
        let generation = self.generation;
        let mut ready = false;
        let result = self.checkout.immediate_transaction_with_authority(
            |_| PaperCatalog6TransactionError::BeforeCommit(PaperCatalog6Error::Authority),
            |conn, authority| {
                let scope = LoanScope;
                let witness = ClosedCatalogProof {
                    loan: CatalogLoan::new(conn, &scope),
                    authority,
                    source: &source,
                    namespace,
                    references,
                    work,
                    purpose: Purpose::Writer,
                    generation,
                };
                witness
                    .validate_on(conn)
                    .map_err(PaperCatalog6TransactionError::BeforeCommit)?;
                let value = operation(conn, authority, &witness)
                    .map_err(PaperCatalog6TransactionError::Consumer)?;
                hook(TestPhase::AfterOperation, conn)
                    .map_err(PaperCatalog6TransactionError::BeforeCommit)?;
                hook(TestPhase::BeforeTail, conn)
                    .map_err(PaperCatalog6TransactionError::BeforeCommit)?;
                witness
                    .validate_on(conn)
                    .map_err(PaperCatalog6TransactionError::BeforeCommit)?;
                tail(conn, authority, &witness, &value)
                    .map_err(PaperCatalog6TransactionError::Consumer)?;
                witness
                    .hook_free_check(conn)
                    .map_err(PaperCatalog6TransactionError::BeforeCommit)?;
                ready = true;
                Ok((value, authority.clone()))
            },
        );
        let (value, authority) = match result {
            Ok(value) => value,
            Err(PaperCatalog6TransactionError::BeforeCommit(e)) if ready => {
                return Err(PaperCatalog6TransactionError::CommitOutcomeUnknown(e))
            }
            Err(e) => return Err(e),
        };
        self.with_committed_readback_closed(
            &authority,
            |_conn, _proof| Ok::<_, E>(()),
            |conn, proof, _| tail(conn, &authority, proof, &value),
        )
        .map_err(|e| match e {
            PaperCatalog6ReadbackError::ObservationUnavailable(e) => {
                PaperCatalog6TransactionError::CommitOutcomeUnknown(e)
            }
            PaperCatalog6ReadbackError::Consumer(e) => {
                PaperCatalog6TransactionError::CommittedConsumerOutcomeUnknown(e)
            }
        })?;
        Ok(value)
    }
    fn with_readonly_closed<T, E>(
        &mut self,
        operation: impl for<'s> FnOnce(&mut SqliteConnection, &ClosedCatalogProof<'s>) -> Result<T, E>,
        tail: impl for<'s> FnMut(&mut SqliteConnection, &ClosedCatalogProof<'s>, &T) -> Result<(), E>,
    ) -> Result<T, PaperCatalog6ReadbackError<E>> {
        let authority = self.checkout.authority().map_err(|_| {
            PaperCatalog6ReadbackError::ObservationUnavailable(PaperCatalog6Error::Authority)
        })?;
        self.with_committed_readback_closed(&authority, operation, tail)
    }

    /// Fresh reader has its own callback-local connection loan. No writer
    /// witness is reused, even when both connections see the same file set.
    fn with_committed_readback_closed<T, E>(
        &mut self,
        expected: &DatabaseConnectionAuthority,
        operation: impl for<'s> FnOnce(&mut SqliteConnection, &ClosedCatalogProof<'s>) -> Result<T, E>,
        mut tail: impl for<'s> FnMut(
            &mut SqliteConnection,
            &ClosedCatalogProof<'s>,
            &T,
        ) -> Result<(), E>,
    ) -> Result<T, PaperCatalog6ReadbackError<E>> {
        let source = Arc::clone(&self.checkout.source);
        let namespace = &self.namespace;
        let references = self.references;
        let work = &self.work;
        let generation = self.generation;
        self.checkout.authority_bound_readonly_snapshot(
            expected,
            |_| PaperCatalog6ReadbackError::ObservationUnavailable(PaperCatalog6Error::Authority),
            |conn| {
                let scope = LoanScope;
                let loan = CatalogLoan::new(conn, &scope);
                loan.require_instance(conn)
                    .map_err(PaperCatalog6ReadbackError::ObservationUnavailable)?;
                let authority =
                    crate::database::registered_descriptor_connection_authority(&source, conn)
                        .map_err(|_| {
                            PaperCatalog6ReadbackError::ObservationUnavailable(
                                PaperCatalog6Error::Authority,
                            )
                        })?;
                if &authority != expected {
                    return Err(PaperCatalog6ReadbackError::ObservationUnavailable(
                        PaperCatalog6Error::Authority,
                    ));
                }
                let witness = ClosedCatalogProof {
                    loan,
                    authority: &authority,
                    source: &source,
                    namespace,
                    references,
                    work,
                    purpose: Purpose::Reader,
                    generation,
                };
                hook(TestPhase::AfterCommit, conn)
                    .map_err(PaperCatalog6ReadbackError::ObservationUnavailable)?;
                if matches!(
                    generation,
                    ClosedGeneration::Seven | ClosedGeneration::Eight
                ) {
                    // First main read fixes the snapshot before catalog/source validation.
                    capture::int(conn, "SELECT COUNT(*) AS value FROM main.sqlite_schema")
                        .map_err(PaperCatalog6ReadbackError::ObservationUnavailable)?;
                }
                witness
                    .validate_on(conn)
                    .map_err(PaperCatalog6ReadbackError::ObservationUnavailable)?;
                let value =
                    operation(conn, &witness).map_err(PaperCatalog6ReadbackError::Consumer)?;
                hook(TestPhase::AfterRead, conn)
                    .map_err(PaperCatalog6ReadbackError::ObservationUnavailable)?;
                witness
                    .validate_on(conn)
                    .map_err(PaperCatalog6ReadbackError::ObservationUnavailable)?;
                tail(conn, &witness, &value).map_err(PaperCatalog6ReadbackError::Consumer)?;
                witness
                    .hook_free_check(conn)
                    .map_err(PaperCatalog6ReadbackError::ObservationUnavailable)?;
                Ok(value)
            },
        )
    }
}
/// Ordinary retained transport. These phases confer no catalog/financial
/// authority and never authorize a retry of the original operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RetainedPaperWritePhase {
    Ready,
    Unopened,
    WriterStopped,
    BeforeCommitReturned,
    CommitUnknown,
    CommittedReadbackPending,
    Complete,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RetainedPaperFirstFault { Callback, Driver, Readback }
#[derive(Debug)]
enum RetainedPaperCallbackFault<E> { Catalog(PaperCatalog6Error), Consumer(E) }
#[derive(Debug)]
enum RetainedPaperBoundary {
    Authority(crate::database::DatabaseAuthorityError),
    Driver(diesel::result::Error),
    CallbackStopped,
}
impl From<diesel::result::Error> for RetainedPaperBoundary {
    fn from(error: diesel::result::Error) -> Self { Self::Driver(error) }
}

// Field order preserves the original checkout-before-maintenance-lease release.
struct RetainedPaperOpening {
    checkout: Option<AttestedAttributionCheckout>,
    namespace: Option<PinnedNamespace>,
    lease: Option<GlobalSchemaMaintenanceLease>,
    paths: Option<ModeBoundPaths>,
}
/// The same actual session, cumulative work, original input and acquired
/// return remain together on every failure. Driver-internal rollback remains
/// unobserved; a returned boundary error is not a rollback-success witness.
#[must_use = "retain the whole owner; an error label cannot recover its resources"]
pub(crate) struct RetainedPaperWrite<'db, I, T, E> {
    session: Option<PaperCatalog6Session<'db>>,
    opening: RetainedPaperOpening,
    input: I,
    value: Option<T>,
    authority: Option<DatabaseConnectionAuthority>,
    callback_fault: Option<RetainedPaperCallbackFault<E>>,
    driver_boundary: Option<RetainedPaperBoundary>,
    readback_boundary: Option<PaperCatalog6ReadbackError<RetainedPaperBoundary>>,
    first_fault: Option<RetainedPaperFirstFault>,
    phase: RetainedPaperWritePhase,
}
#[must_use = "Held and Pending contain the original resource owner"]
pub(crate) enum RetainedPaperWriteOutcome<'db, I, T, E> {
    Complete(RetainedPaperWrite<'db, I, T, E>),
    Held(RetainedPaperWrite<'db, I, T, E>),
    Pending(RetainedPaperWrite<'db, I, T, E>),
}

/// Used at both actual callback-return boundaries, before the next fallible
/// operation. No clone, summary or later database read reconstructs T.
fn retain_paper_return<T, E>(slot: &mut Option<T>, result: Result<T, E>) -> Result<(), E> {
    match result {
        Ok(value) => { *slot = Some(value); Ok(()) }
        Err(error) => Err(error),
    }
}
impl<'db, I, T, E> RetainedPaperWrite<'db, I, T, E> {
    pub(crate) fn open(db: &'db DatabaseManager, input: I) -> Self {
        // Input is owned before the first path/namespace/lease/checkout action.
        let mut frame = Self::unopened(input, PaperCatalog6Error::Authority);
        frame.callback_fault = None;
        frame.first_fault = None;
        match frame.open_acquire(db) {
            Ok(()) => frame.phase = RetainedPaperWritePhase::Ready,
            Err(error) => { frame.callback_fault = Some(RetainedPaperCallbackFault::Catalog(error)); frame.first_fault = Some(RetainedPaperFirstFault::Callback); }
        }
        frame
    }
    fn open_acquire(&mut self, db: &'db DatabaseManager) -> Result<(), PaperCatalog6Error> {
        self.opening.paths = Some(paths(db)?); // Original production refusal is first.
        self.opening.namespace = Some(PinnedNamespace::open(self.opening.paths.as_ref().expect("actual paths retained"))
            .map_err(|_| PaperCatalog6Error::Namespace)?);
        self.opening.lease = Some(GlobalSchemaMaintenanceLease::acquire_shared(
            self.opening.paths.as_ref().expect("actual paths retained"),
            self.opening.namespace.as_ref().expect("actual namespace retained"))
            .map_err(|_| PaperCatalog6Error::Namespace)?);
        self.opening.checkout = Some(db.attribution_checkout().map_err(|_| PaperCatalog6Error::Authority)?);
        let references = refs()?;
        self.session = Some(PaperCatalog6Session {
            checkout: self.opening.checkout.take().expect("actual checkout retained"),
            namespace: self.opening.namespace.take().expect("actual namespace retained"),
            _lease: self.opening.lease.take().expect("actual maintenance lease retained"),
            _db: db, references, work: RefCell::new(CopyWork::new()), generation: ClosedGeneration::Six,
        });
        Ok(())
    }
    pub(crate) fn unopened(input: I, error: PaperCatalog6Error) -> Self {
        Self { session: None,
            opening: RetainedPaperOpening { checkout: None, namespace: None, lease: None, paths: None },
            input, value: None, authority: None,
            callback_fault: Some(RetainedPaperCallbackFault::Catalog(error)),
            driver_boundary: None, readback_boundary: None,
            first_fault: Some(RetainedPaperFirstFault::Callback),
            phase: RetainedPaperWritePhase::Unopened }
    }
    pub(crate) fn phase(&self) -> RetainedPaperWritePhase { self.phase }
    fn outcome(self) -> RetainedPaperWriteOutcome<'db, I, T, E> {
        match self.phase {
            RetainedPaperWritePhase::Complete => RetainedPaperWriteOutcome::Complete(self),
            RetainedPaperWritePhase::CommitUnknown | RetainedPaperWritePhase::CommittedReadbackPending => RetainedPaperWriteOutcome::Pending(self),
            _ => RetainedPaperWriteOutcome::Held(self),
        }
    }
    /// Explicit known-success release of this pooled checkout, not an observed
    /// SQLite consuming-close result. The original input is also returned.
    pub(crate) fn finish_complete(mut self) -> Result<(I, T), Self> {
        if self.phase != RetainedPaperWritePhase::Complete || self.value.is_none() { return Err(self); }
        let value = self.value.take().expect("complete requires the actual return");
        drop(self.session.take());
        Ok((self.input, value))
    }
    pub(crate) fn run_once(
        mut self,
        operation: impl for<'s> FnOnce(&mut SqliteConnection, &DatabaseConnectionAuthority, &VerifiedCatalog6<'s>, &mut I) -> Result<T, E>,
        mut tail: impl for<'s> FnMut(&mut SqliteConnection, &DatabaseConnectionAuthority, &VerifiedCatalog6<'s>, &I, &T) -> Result<(), E>,
    ) -> RetainedPaperWriteOutcome<'db, I, T, E> {
        if self.phase != RetainedPaperWritePhase::Ready { return self.outcome(); }
        let Self { session, input, value, authority, callback_fault, driver_boundary,
            readback_boundary, first_fault, phase, .. } = &mut self;
        let session = session.as_mut().expect("Ready requires the acquired session");
        let source = Arc::clone(&session.checkout.source);
        let namespace = &session.namespace;
        let references = session.references;
        let work = &session.work;
        let generation = session.generation;
        // Lower authority checks and the true driver transaction own only ().
        let writer = session.checkout.immediate_transaction_with_authority(
            RetainedPaperBoundary::Authority,
            |conn, actual| {
                *authority = Some(actual.clone());
                let scope = LoanScope;
                let proof = ClosedCatalogProof { loan: CatalogLoan::new(conn, &scope),
                    authority: actual, source: &source, namespace, references, work,
                    purpose: Purpose::Writer, generation };
                if let Err(error) = proof.validate_on(conn) {
                    *callback_fault = Some(RetainedPaperCallbackFault::Catalog(error));
                    *first_fault = Some(RetainedPaperFirstFault::Callback);
                    return Err(RetainedPaperBoundary::CallbackStopped);
                }
                if let Err(error) = retain_paper_return(value, operation(conn, actual, &VerifiedCatalog6(&proof), input)) {
                    *callback_fault = Some(RetainedPaperCallbackFault::Consumer(error));
                    *first_fault = Some(RetainedPaperFirstFault::Callback);
                    return Err(RetainedPaperBoundary::CallbackStopped);
                }
                // T already belongs to the outer frame at all these cuts.
                for next in [TestPhase::AfterOperation, TestPhase::BeforeTail] {
                    if let Err(error) = hook(next, conn) {
                        *callback_fault = Some(RetainedPaperCallbackFault::Catalog(error));
                        *first_fault = Some(RetainedPaperFirstFault::Callback);
                        return Err(RetainedPaperBoundary::CallbackStopped);
                    }
                }
                if let Err(error) = proof.validate_on(conn) {
                    *callback_fault = Some(RetainedPaperCallbackFault::Catalog(error));
                    *first_fault = Some(RetainedPaperFirstFault::Callback);
                    return Err(RetainedPaperBoundary::CallbackStopped);
                }
                if let Err(error) = tail(conn, actual, &VerifiedCatalog6(&proof), input, value.as_ref().expect("actual return retained")) {
                    *callback_fault = Some(RetainedPaperCallbackFault::Consumer(error));
                    *first_fault = Some(RetainedPaperFirstFault::Callback);
                    return Err(RetainedPaperBoundary::CallbackStopped);
                }
                if let Err(error) = proof.hook_free_check(conn) {
                    *callback_fault = Some(RetainedPaperCallbackFault::Catalog(error));
                    *first_fault = Some(RetainedPaperFirstFault::Callback);
                    return Err(RetainedPaperBoundary::CallbackStopped);
                }
                *phase = RetainedPaperWritePhase::BeforeCommitReturned;
                Ok(())
            },
        );
        if let Err(error) = writer {
            *driver_boundary = Some(error);
            if first_fault.is_none() { *first_fault = Some(RetainedPaperFirstFault::Driver); }
            *phase = if *phase == RetainedPaperWritePhase::BeforeCommitReturned {
                RetainedPaperWritePhase::CommitUnknown
            } else { RetainedPaperWritePhase::WriterStopped };
            return self.outcome();
        }
        // This phase follows the real lower transaction's successful return,
        // not merely the callback's BeforeCommitReturned marker.
        *phase = RetainedPaperWritePhase::CommittedReadbackPending;
        let expected = authority.as_ref().expect("successful writer acquired actual authority");
        let reader = session.with_committed_readback_closed(
            expected,
            |_conn, _proof| Ok::<(), RetainedPaperBoundary>(()),
            |conn, proof, _| {
                if let Err(error) = tail(conn, expected, &VerifiedCatalog6(proof), input,
                    value.as_ref().expect("actual return retained")) {
                    *callback_fault = Some(RetainedPaperCallbackFault::Consumer(error));
                    *first_fault = Some(RetainedPaperFirstFault::Callback);
                    return Err(RetainedPaperBoundary::CallbackStopped);
                }
                Ok(())
            },
        );
        match reader {
            Ok(()) => *phase = RetainedPaperWritePhase::Complete,
            Err(error) => {
                *readback_boundary = Some(error);
                if first_fault.is_none() { *first_fault = Some(RetainedPaperFirstFault::Readback); }
            }
        }
        self.outcome()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TestPhase {
    AfterOperation,
    BeforeTail,
    AfterCommit,
    AfterRead,
    DuringMigration,
}
#[cfg(test)]
thread_local! { static HOOK:RefCell<Option<Box<dyn FnMut(TestPhase,&mut SqliteConnection)->Result<(),PaperCatalog6Error>>>>=RefCell::new(None); }
#[cfg(test)]
fn hook(phase: TestPhase, conn: &mut SqliteConnection) -> Result<(), PaperCatalog6Error> {
    HOOK.with(|h| match h.borrow_mut().as_mut() {
        Some(h) => h(phase, conn),
        None => Ok(()),
    })
}
#[cfg(not(test))]
fn hook(_: TestPhase, _: &mut SqliteConnection) -> Result<(), PaperCatalog6Error> {
    Ok(())
}

/// Prepare only the original complete V5 fixture for its exact selection
/// catalog transition. It never qualifies Production or accepts extra objects.
#[cfg(test)]
pub(crate) fn prepare_final_selection_for_isolated_v5_test(
    db: &DatabaseManager,
) -> Result<(), PaperCatalog6Error> {
    use diesel::connection::SimpleConnection;
    let mut session = paper_catalog6_session(db)?;
    let references = session.references;
    let work = &session.work;
    session.checkout.immediate_transaction_with_authority(
        |_| PaperCatalog6Error::Authority,
        |conn, _| {
            let scope = LoanScope;
            let loan = CatalogLoan::new(conn, &scope);
            let snapshot = capture::capture(
                &loan,
                conn,
                GlobalSchemaCatalogMode::Test,
                &mut work.borrow_mut(),
            )?;
            match capture::classify_bounded(&snapshot, references, &mut work.borrow_mut())? {
                DatabaseHalfDiagnostic::PreAmendment(e) if e.identity.user_version == 5 => {}
                _ => {
                    return Err(PaperCatalog6Error::Catalog(
                        "not an exact original V5 family".into(),
                    ))
                }
            }
            for statement in crate::database::selection_v2::selection_v2_catalog_ddl_plan(
                crate::database::selection_v2::SelectionV2StoreMode::Test,
                crate::database::selection_v2::SelectionV2CatalogDdlPhase::Final,
            )
            .map_err(|e| PaperCatalog6Error::Catalog(e.to_string()))?
            {
                conn.batch_execute(&statement.exact_sql)?;
            }
            let snapshot = capture::capture(
                &loan,
                conn,
                GlobalSchemaCatalogMode::Test,
                &mut work.borrow_mut(),
            )?;
            match capture::classify_bounded(&snapshot, references, &mut work.borrow_mut())? {
                DatabaseHalfDiagnostic::AmendedDatabaseHalf(e) if e.identity.user_version == 5 => {
                    Ok(())
                }
                _ => Err(PaperCatalog6Error::Catalog(
                    "final V5 family differs".into(),
                )),
            }
        },
    )
}

/// Test-only real migration from the complete V5 catalog and original replay.
/// No arbitrary connection or bool can issue the constructor-origin witness.
#[cfg(test)]
pub(crate) fn migrate_catalog6_for_isolated_test(
    db: &DatabaseManager,
) -> Result<(), PaperCatalog6TransactionError<PaperCatalog6Error>> {
    let mut session =
        paper_catalog6_session(db).map_err(PaperCatalog6TransactionError::BeforeCommit)?;
    let source = Arc::clone(&session.checkout.source);
    let ns = &session.namespace;
    let references = session.references;
    let work = &session.work;
    let mut ready = false;
    let result = session.checkout.immediate_transaction_with_authority(
        |_| PaperCatalog6TransactionError::BeforeCommit(PaperCatalog6Error::Authority),
        |conn, authority| {
            let result = (|| -> Result<_, PaperCatalog6Error> {
                let scope = LoanScope;
                let loan = CatalogLoan::new(conn, &scope);
                loan.require_instance(conn)?;
                controls(conn, Purpose::Writer)?;
                capture::preflight_pair(conn, &mut work.borrow_mut())?;
                if &capture::temporary_catalog(&loan, conn, &mut work.borrow_mut())?
                    != temp_reference()?
                {
                    return Err(PaperCatalog6Error::Catalog(
                        "unexpected TEMP catalog".into(),
                    ));
                }
                ns.validate_unchanged()
                    .map_err(|_| PaperCatalog6Error::Namespace)?;
                let actual = capture::capture(
                    &loan,
                    conn,
                    GlobalSchemaCatalogMode::Test,
                    &mut work.borrow_mut(),
                )?;
                match capture::classify_bounded(&actual, references, &mut work.borrow_mut())? {
                    DatabaseHalfDiagnostic::AmendedDatabaseHalf(e)
                        if e.identity.user_version == 5 => {}
                    _ => return Err(PaperCatalog6Error::Catalog6RequalificationRequired),
                }
                crate::trading::paper_book_v2::verify_owner_rows_on(conn)
                    .map_err(|_| PaperCatalog6Error::Catalog("original owner replay".into()))?;
                crate::database::paper_book_v2_schema::verify_v5_manifest_on(conn)
                    .map_err(|_| PaperCatalog6Error::Catalog("original fee manifest".into()))?;
                crate::database::paper_book_v2_execution_schema_v1::create_schema(conn)
                    .map_err(|_| PaperCatalog6Error::Sql)?;
                diesel::sql_query("PRAGMA user_version=6").execute(conn)?;
                hook(TestPhase::DuringMigration, conn)?;
                let proof = ClosedCatalogProof {
                    loan,
                    authority,
                    source: &source,
                    namespace: ns,
                    references,
                    work,
                    purpose: Purpose::Writer,
                    generation: ClosedGeneration::Six,
                };
                proof.validate_on(conn)?;
                proof.hook_free_check(conn)?;
                Ok(authority.clone())
            })();
            if result.is_ok() {
                ready = true;
            }
            result.map_err(PaperCatalog6TransactionError::BeforeCommit)
        },
    );
    let authority = match result {
        Ok(a) => a,
        Err(PaperCatalog6TransactionError::BeforeCommit(e)) if ready => {
            return Err(PaperCatalog6TransactionError::CommitOutcomeUnknown(e))
        }
        Err(e) => return Err(e),
    };
    session
        .with_committed_readback_closed(
            &authority,
            |_, _| Ok::<_, PaperCatalog6Error>(()),
            |_, _, _| Ok(()),
        )
        .map_err(|e| match e {
            PaperCatalog6ReadbackError::ObservationUnavailable(e) => {
                PaperCatalog6TransactionError::CommitOutcomeUnknown(e)
            }
            PaperCatalog6ReadbackError::Consumer(e) => {
                PaperCatalog6TransactionError::CommittedConsumerOutcomeUnknown(e)
            }
        })
}

#[cfg(test)]
pub(super) fn migrate_catalog7_for_isolated_test(
    db: &DatabaseManager,
) -> Result<(), PaperCatalog6TransactionError<PaperCatalog6Error>> {
    let mut session =
        candidate_catalog7_session(db).map_err(PaperCatalog6TransactionError::BeforeCommit)?;
    let source = Arc::clone(&session.checkout.source);
    let namespace = &session.namespace;
    let references = session.references;
    let work = &session.work;
    let mut ready = false;
    let result = session.checkout.immediate_transaction_with_authority(
        |_| PaperCatalog6TransactionError::BeforeCommit(PaperCatalog6Error::Authority),
        |conn, authority| {
            let scope = LoanScope;
            let before = ClosedCatalogProof {
                loan: CatalogLoan::new(conn, &scope),
                authority,
                source: &source,
                namespace,
                references,
                work,
                purpose: Purpose::Writer,
                generation: ClosedGeneration::Six,
            };
            before
                .validate_on(conn)
                .map_err(PaperCatalog6TransactionError::BeforeCommit)?;
            crate::database::candidate_scope_observation_schema_v1::create_schema(conn)?;
            diesel::sql_query("PRAGMA user_version=7").execute(conn)?;
            hook(TestPhase::DuringMigration, conn)
                .map_err(PaperCatalog6TransactionError::BeforeCommit)?;
            let after = ClosedCatalogProof {
                generation: ClosedGeneration::Seven,
                ..before
            };
            after
                .validate_on(conn)
                .map_err(PaperCatalog6TransactionError::BeforeCommit)?;
            after
                .hook_free_check(conn)
                .map_err(PaperCatalog6TransactionError::BeforeCommit)?;
            ready = true;
            Ok(authority.clone())
        },
    );
    let authority = match result {
        Ok(a) => a,
        Err(PaperCatalog6TransactionError::BeforeCommit(e)) if ready => {
            return Err(PaperCatalog6TransactionError::CommitOutcomeUnknown(e))
        }
        Err(e) => return Err(e),
    };
    session
        .with_committed_readback_closed(
            &authority,
            |_, _| Ok::<_, PaperCatalog6Error>(()),
            |_, _, _| Ok(()),
        )
        .map_err(|e| match e {
            PaperCatalog6ReadbackError::ObservationUnavailable(e) => {
                PaperCatalog6TransactionError::CommitOutcomeUnknown(e)
            }
            PaperCatalog6ReadbackError::Consumer(e) => {
                PaperCatalog6TransactionError::CommittedConsumerOutcomeUnknown(e)
            }
        })
}

#[cfg(test)]
pub(super) fn migrate_catalog8_for_isolated_test(
    db: &DatabaseManager,
) -> Result<(), PaperCatalog6TransactionError<PaperCatalog6Error>> {
    let mut session =
        investment_catalog8_session(db).map_err(PaperCatalog6TransactionError::BeforeCommit)?;
    let source = Arc::clone(&session.checkout.source);
    let namespace = &session.namespace;
    let references = session.references;
    let work = &session.work;
    let mut ready = false;
    let result = session.checkout.immediate_transaction_with_authority(
        |_| PaperCatalog6TransactionError::BeforeCommit(PaperCatalog6Error::Authority),
        |conn, authority| {
            let scope = LoanScope;
            let before = ClosedCatalogProof {
                loan: CatalogLoan::new(conn, &scope),
                authority,
                source: &source,
                namespace,
                references,
                work,
                purpose: Purpose::Writer,
                generation: ClosedGeneration::Seven,
            };
            before
                .validate_on(conn)
                .map_err(PaperCatalog6TransactionError::BeforeCommit)?;
            crate::database::investment_decision_schema_v1::create_schema(conn)?;
            diesel::sql_query("PRAGMA user_version=8").execute(conn)?;
            hook(TestPhase::DuringMigration, conn)
                .map_err(PaperCatalog6TransactionError::BeforeCommit)?;
            let after = ClosedCatalogProof {
                generation: ClosedGeneration::Eight,
                ..before
            };
            after
                .validate_on(conn)
                .map_err(PaperCatalog6TransactionError::BeforeCommit)?;
            after
                .hook_free_check(conn)
                .map_err(PaperCatalog6TransactionError::BeforeCommit)?;
            ready = true;
            Ok(authority.clone())
        },
    );
    let authority = match result {
        Ok(a) => a,
        Err(PaperCatalog6TransactionError::BeforeCommit(e)) if ready => {
            return Err(PaperCatalog6TransactionError::CommitOutcomeUnknown(e))
        }
        Err(e) => return Err(e),
    };
    session
        .with_committed_readback_closed(
            &authority,
            |_, _| Ok::<_, PaperCatalog6Error>(()),
            |_, _, _| Ok(()),
        )
        .map_err(|e| match e {
            PaperCatalog6ReadbackError::ObservationUnavailable(e) => {
                PaperCatalog6TransactionError::CommitOutcomeUnknown(e)
            }
            PaperCatalog6ReadbackError::Consumer(e) => {
                PaperCatalog6TransactionError::CommittedConsumerOutcomeUnknown(e)
            }
        })
}

#[cfg(test)]
mod tests {
    include!("global_schema_paper_v6_tests.rs");
    include!("global_schema_investment_v8_tests.rs");

    struct RetainedTestReturn {
        bytes: Box<String>,
        dropped: std::rc::Rc<std::cell::Cell<usize>>,
    }
    impl Drop for RetainedTestReturn {
        fn drop(&mut self) { self.dropped.set(self.dropped.get() + 1); }
    }
    fn retained_test_command() -> crate::trading::paper_book_v2_execution::PaperV2Command {
        crate::trading::paper_book_v2_execution::PaperV2Command::Cancel {
            command_id: "TEST_CODE_RETAINED_ORIGINAL_COMMAND".into(),
            expected: crate::trading::paper_book_v2_execution::HeadIdentity {
                version: 1, event_hash: "a".repeat(64) },
            parent_id: "TEST_CODE_RETAINED_ORIGINAL_PARENT".into(),
        }
    }
    fn retained_command_pointer(command: &crate::trading::paper_book_v2_execution::PaperV2Command) -> *const u8 {
        match command {
            crate::trading::paper_book_v2_execution::PaperV2Command::Cancel { command_id, .. } => command_id.as_ptr(),
            _ => panic!("fixed Cancel fixture only"),
        }
    }
    #[test]
    fn paper_retained_write_actual_commit_reader_cuts_keep_original_owners() {
        let _serial = super::super::tests::PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        // Successful transport, actual post-COMMIT gate failure and actual
        // post-COMMIT consumer failure are separate reached cuts.
        for cut in 0..3 {
            let f = Fixture::v6();
            let command = retained_test_command();
            let command_pointer = retained_command_pointer(&command);
            let drops = std::rc::Rc::new(std::cell::Cell::new(0));
            let owned = RetainedTestReturn { bytes: Box::new("TEST_CODE_actual_owned_return".into()), dropped: drops.clone() };
            let value_pointer = owned.bytes.as_ptr();
            let calls = std::rc::Rc::new(std::cell::Cell::new(0));
            let counted = calls.clone();
            let guard = set_hook(move |phase, _| {
                if cut == 1 && phase == TestPhase::AfterCommit { Err(PaperCatalog6Error::InjectedFailure) } else { Ok(()) }
            });
            let outcome = RetainedPaperWrite::open(&f.db, command).run_once(
                move |conn, _, _, _| {
                    counted.set(counted.get() + 1);
                    insert(conn, "2026-09-28")?;
                    Ok::<_, PaperCatalog6Error>(owned)
                },
                |conn, _, proof, _, _| {
                    tail_count(conn, 1)?;
                    if cut == 2 && proof.purpose == Purpose::Reader { Err(PaperCatalog6Error::InjectedFailure) } else { Ok(()) }
                },
            );
            let frame = match outcome {
                RetainedPaperWriteOutcome::Complete(frame) if cut == 0 => frame,
                RetainedPaperWriteOutcome::Pending(frame) if cut != 0 => frame,
                _ => panic!("actual COMMIT/readback cut must have exact outcome"),
            };
            assert_eq!(f.daily_count(), 1);
            assert_eq!(calls.get(), 1);
            assert_eq!(retained_command_pointer(&frame.input), command_pointer);
            assert_eq!(frame.value.as_ref().unwrap().bytes.as_ptr(), value_pointer);
            assert_eq!(drops.get(), 0);
            assert!(frame.session.is_some() && frame.authority.is_some());
            let work_before = frame.session.as_ref().unwrap().work.borrow().remaining_for_test();
            let repeated = frame.run_once(|_, _, _, _| panic!("operation must never replay"),
                |_, _, _, _, _| panic!("tail must never replay"));
            let frame = match repeated {
                RetainedPaperWriteOutcome::Complete(frame) if cut == 0 => frame,
                RetainedPaperWriteOutcome::Pending(frame) if cut != 0 => frame,
                _ => panic!("repeat must retain the same terminal phase"),
            };
            assert_eq!(frame.session.as_ref().unwrap().work.borrow().remaining_for_test(), work_before);
            assert_eq!(frame.value.as_ref().unwrap().bytes.as_ptr(), value_pointer);
            if cut == 0 {
                let (input, value) = match frame.finish_complete() { Ok(done) => done, Err(_) => panic!("complete must release explicitly") };
                assert_eq!(retained_command_pointer(&input), command_pointer);
                assert_eq!(value.bytes.as_ptr(), value_pointer);
                assert_eq!(drops.get(), 0);
                drop(value);
            } else {
                assert_eq!(frame.phase(), RetainedPaperWritePhase::CommittedReadbackPending);
                assert!(frame.readback_boundary.is_some());
                if cut == 2 {
                    assert!(matches!(frame.callback_fault, Some(RetainedPaperCallbackFault::Consumer(PaperCatalog6Error::InjectedFailure))));
                    assert_eq!(frame.first_fault, Some(RetainedPaperFirstFault::Callback));
                } else { assert_eq!(frame.first_fault, Some(RetainedPaperFirstFault::Readback)); }
                let frame = match frame.finish_complete() { Err(retained) => retained, Ok(_) => panic!("Pending cannot release as success") };
                assert_eq!(drops.get(), 0);
                drop(frame); // Explicit fixture teardown, not production recovery.
            }
            assert_eq!(drops.get(), 1);
            drop(guard);
        }
    }
    #[test]
    fn paper_retained_write_precommit_first_fault_and_once_work_hold() {
        let _serial = super::super::tests::PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        let f = Fixture::v6();
        let command = retained_test_command();
        let command_pointer = retained_command_pointer(&command);
        let drops = std::rc::Rc::new(std::cell::Cell::new(0));
        let owned = RetainedTestReturn { bytes: Box::new("TEST_CODE_retained_before_tail".into()), dropped: drops.clone() };
        let value_pointer = owned.bytes.as_ptr();
        let outcome = RetainedPaperWrite::open(&f.db, command).run_once(
            move |conn, _, _, _| { insert(conn, "2026-09-28")?; Ok::<_, PaperCatalog6Error>(owned) },
            |conn, _, _, _, _| { tail_count(conn, 1)?; Err(PaperCatalog6Error::InjectedFailure) },
        );
        let frame = match outcome { RetainedPaperWriteOutcome::Held(frame) => frame, _ => panic!("writer tail refusal must hold") };
        assert_eq!(f.daily_count(), 0); // Independent actual DB observation after real driver rollback path.
        assert_eq!(frame.phase(), RetainedPaperWritePhase::WriterStopped);
        assert_eq!(frame.first_fault, Some(RetainedPaperFirstFault::Callback));
        assert!(matches!(frame.callback_fault, Some(RetainedPaperCallbackFault::Consumer(PaperCatalog6Error::InjectedFailure))));
        assert!(matches!(frame.driver_boundary, Some(RetainedPaperBoundary::CallbackStopped)));
        assert_eq!(frame.value.as_ref().unwrap().bytes.as_ptr(), value_pointer);
        assert_eq!(retained_command_pointer(&frame.input), command_pointer);
        assert_eq!(drops.get(), 0);
        let work_before = frame.session.as_ref().unwrap().work.borrow().remaining_for_test();
        let held = frame.run_once(|_, _, _, _| panic!("no second SQL operation"), |_, _, _, _, _| panic!("no second tail"));
        let frame = match held { RetainedPaperWriteOutcome::Held(frame) => frame, _ => panic!("same Held owner required") };
        assert_eq!(frame.session.as_ref().unwrap().work.borrow().remaining_for_test(), work_before);
        assert_eq!(frame.first_fault, Some(RetainedPaperFirstFault::Callback));
        assert_eq!(drops.get(), 0);
        drop(frame); // Only explicit fixture cleanup consumes the retained T.
        assert_eq!(drops.get(), 1);
        // A separate genuinely new invocation has a short pool before any
        // capture/operation; it does not reset the preceding owner's work.
        let mut short = RetainedPaperWrite::open(&f.db, retained_test_command());
        short.session.as_mut().unwrap().work = RefCell::new(CopyWork::limited(0));
        let calls = std::rc::Rc::new(std::cell::Cell::new(0));
        let counted = calls.clone();
        let short = short.run_once(move |_, _, _, _| {
            counted.set(counted.get() + 1);
            Ok::<_, PaperCatalog6Error>(1u8)
        }, |_, _, _, _, _| panic!("short-before-owned cannot reach the tail"));
        let short = match short { RetainedPaperWriteOutcome::Held(frame) => frame, _ => panic!("short gate must retain") };
        assert_eq!(calls.get(), 0);
        assert!(short.value.is_none());
        assert!(matches!(short.callback_fault, Some(RetainedPaperCallbackFault::Catalog(PaperCatalog6Error::CopyBudgetExceeded))));
        assert_eq!(short.session.as_ref().unwrap().work.borrow().remaining_for_test(), 0);
        drop(short);
    }
    #[test]
    fn paper_retained_write_genuine_deferred_fk_commit_error_keeps_outer_return() {
        // Callee-only control: this connection does not claim a Catalog6 loan.
        let mut conn = SqliteConnection::establish(":memory:").unwrap();
        conn.batch_execute("PRAGMA foreign_keys=ON; CREATE TABLE retained_parent(id INTEGER PRIMARY KEY); CREATE TABLE retained_child(parent_id INTEGER REFERENCES retained_parent(id) DEFERRABLE INITIALLY DEFERRED);").unwrap();
        let drops = std::rc::Rc::new(std::cell::Cell::new(0));
        let owned = RetainedTestReturn { bytes: Box::new("TEST_CODE_commit_error_owned".into()), dropped: drops.clone() };
        let value_pointer = owned.bytes.as_ptr();
        let mut outer = None;
        let callback_returned = std::rc::Rc::new(std::cell::Cell::new(false));
        let reached = callback_returned.clone();
        let actual = conn.immediate_transaction::<(), diesel::result::Error, _>(|conn| {
            conn.batch_execute("INSERT INTO retained_child(parent_id) VALUES(41)")?;
            retain_paper_return(&mut outer, Ok::<_, diesel::result::Error>(owned))?;
            reached.set(true);
            Ok(())
        });
        assert!(callback_returned.get());
        assert!(matches!(actual, Err(diesel::result::Error::DatabaseError(diesel::result::DatabaseErrorKind::ForeignKeyViolation, _))));
        assert_eq!(outer.as_ref().unwrap().bytes.as_ptr(), value_pointer);
        assert_eq!(drops.get(), 0);
        // Explicit fixture cleanup; no inference about an opaque driver-internal rollback.
        let cleanup_return = conn.batch_execute("ROLLBACK");
        #[derive(diesel::QueryableByName)]
        struct Count { #[diesel(sql_type=diesel::sql_types::BigInt)] value: i64 }
        let actual_rows = diesel::sql_query("SELECT COUNT(*) AS value FROM retained_child").get_result::<Count>(&mut conn).unwrap().value;
        assert_eq!(actual_rows, 0);
        assert_eq!(drops.get(), 0);
        drop(outer.take());
        assert_eq!(drops.get(), 1);
        drop(cleanup_return); // Preserve the actual cleanup return until explicit fixture teardown.
    }
    #[test]
    fn paper_retained_write_fixed_execution_entry_refusal_keeps_original_command() {
        use crate::trading::paper_book_v2_execution::{
            apply_retained_actual, retained_execution_command_for_test, PaperV2Command,
        };
        let _serial = super::super::tests::PROSPECTIVE_TEST_SERIAL.lock().unwrap();
        // The ordinary unit-test singleton is a real manager without an isolated
        // constructor-origin witness. Do not replace its OnceCell with a V6 fixture.
        DatabaseManager::init(None).unwrap();
        assert!(!DatabaseManager::get().has_isolated_p05_consumer_origin());
        let observe = |command: &PaperV2Command| {
            match command {
                PaperV2Command::Cancel { command_id, expected, parent_id } => {
                    assert_eq!(command_id, "TEST_CODE_RETAINED_ORIGINAL_COMMAND");
                    assert_eq!(parent_id, "TEST_CODE_RETAINED_ORIGINAL_PARENT");
                    assert_eq!(expected.version, 1);
                    assert_eq!(expected.event_hash, "a".repeat(64));
                    (command_id.as_ptr(), parent_id.as_ptr(), expected.event_hash.as_ptr())
                }
                _ => panic!("the genuine fixed Cancel command must remain owned"),
            }
        };
        let command = retained_test_command();
        let original_buffers = observe(&command);
        // This calls the fixed execution API, not the generic retained core.
        let outcome = apply_retained_actual("TEST_CODE_RETAINED_ENTRY_ACCOUNT", command);
        let frame = match outcome {
            RetainedPaperWriteOutcome::Held(frame) => frame,
            _ => panic!("ordinary global manager must keep the original opening refusal"),
        };
        assert_eq!(frame.phase(), RetainedPaperWritePhase::Unopened);
        assert!(matches!(frame.callback_fault.as_ref(), Some(RetainedPaperCallbackFault::Catalog(
            PaperCatalog6Error::Catalog6RequalificationRequired))));
        assert_eq!(frame.first_fault, Some(RetainedPaperFirstFault::Callback));
        assert_eq!(observe(retained_execution_command_for_test(&frame.input)), original_buffers);
        // paths() refused before a session/Work existed. This proves no pool
        // was created or reset, not a successful allocation/consumption witness.
        assert!(frame.session.is_none());
        assert!(frame.opening.paths.is_none() && frame.opening.namespace.is_none());
        assert!(frame.opening.checkout.is_none() && frame.opening.lease.is_none());
        assert!(frame.value.is_none() && frame.authority.is_none());
        assert!(frame.driver_boundary.is_none() && frame.readback_boundary.is_none());
        let repeated = frame.run_once(
            |_, _, _, _| panic!("opening refusal cannot run the actual execution callback"),
            |_, _, _, _, _| panic!("opening refusal cannot run any execution tail"),
        );
        let frame = match repeated {
            RetainedPaperWriteOutcome::Held(frame) => frame,
            _ => panic!("repeat must keep the same unopened owner"),
        };
        assert_eq!(frame.phase(), RetainedPaperWritePhase::Unopened);
        assert_eq!(frame.first_fault, Some(RetainedPaperFirstFault::Callback));
        assert!(matches!(frame.callback_fault.as_ref(), Some(RetainedPaperCallbackFault::Catalog(
            PaperCatalog6Error::Catalog6RequalificationRequired))));
        assert!(frame.session.is_none() && frame.value.is_none());
        assert!(frame.driver_boundary.is_none() && frame.readback_boundary.is_none());
        assert_eq!(observe(retained_execution_command_for_test(&frame.input)), original_buffers);
        let frame = match frame.finish_complete() {
            Err(held) => held,
            Ok(_) => panic!("unopened failure must never release a successful command/return"),
        };
        assert_eq!(frame.phase(), RetainedPaperWritePhase::Unopened);
        assert_eq!(frame.first_fault, Some(RetainedPaperFirstFault::Callback));
        assert!(frame.session.is_none() && frame.value.is_none());
        assert_eq!(observe(retained_execution_command_for_test(&frame.input)), original_buffers);
        drop(frame); // Explicit fixture teardown, not production recovery/retry.
    }
}


// A specialized cfg-only short loan of the fixed execution's actual owner.
// Work's address identifies two observations in the same stable test placement;
// it is not a persistent origin/connection witness across moves.
#[cfg(test)]
impl<'db, 'a> RetainedPaperWrite<
    'db,
    crate::trading::paper_book_v2_execution::RetainedExecutionInput<'a>,
    crate::trading::paper_book_v2_execution::RetainedExecutionAcquired,
    crate::trading::paper_ledger::LedgerError,
> {
    pub(crate) fn observe_fixed_execution_for_test(&self) -> (
        &crate::trading::paper_book_v2_execution::RetainedExecutionInput<'a>,
        Option<&crate::trading::paper_book_v2_execution::RetainedExecutionAcquired>,
        Option<(usize, usize)>,
        Option<&crate::trading::paper_ledger::LedgerError>,
        (bool, bool, bool),
    ) {
        let work = self.session.as_ref().map(|session| {
            (&session.work as *const RefCell<CopyWork> as usize,
                session.work.borrow().remaining_for_test())
        });
        let consumer = match self.callback_fault.as_ref() {
            Some(RetainedPaperCallbackFault::Consumer(error)) => Some(error),
            _ => None,
        };
        (&self.input, self.value.as_ref(), work, consumer,
            (self.first_fault == Some(RetainedPaperFirstFault::Callback),
                self.driver_boundary.is_some(), self.readback_boundary.is_some()))
    }
}
