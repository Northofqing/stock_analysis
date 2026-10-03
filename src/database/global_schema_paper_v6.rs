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
}
impl ClosedGeneration {
    fn number(self) -> i64 {
        match self {
            Self::Six => 6,
            Self::Seven => 7,
        }
    }
    fn error(self) -> PaperCatalog6Error {
        match self {
            Self::Six => PaperCatalog6Error::Catalog6RequalificationRequired,
            Self::Seven => PaperCatalog6Error::Catalog7RequalificationRequired,
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
                if generation == ClosedGeneration::Seven {
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
#[path = "global_schema_paper_v6_tests.rs"]
mod tests;
