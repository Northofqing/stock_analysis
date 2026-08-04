//! BR-193 durable generation-acquisition journal.
//!
//! This module is intentionally not a provider or scheduler API. It persists
//! already validated acquisition carriers under one SQLite transaction, then
//! closes the immutable row through the sole selection audit chain and an
//! append-only audit-closure row. A crash between those steps is recoverable:
//! replay finds the exact SQLite row and either reuses or appends the exact
//! audit record before inserting the closure.

#![allow(
    dead_code,
    reason = "BR-193 Gate B builds the durable journal before the opaque scheduler owner is released"
)]

use chrono::{DateTime, FixedOffset};
use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel::sql_types::{Binary, Nullable, Text};
use thiserror::Error;

use crate::selection::acquisition_v2::{
    parse_generation_acquisition_cadence_receipt, VerifiedGenerationAcquisitionCadenceReceipt,
};
use crate::selection::audit::{
    AuditExactLookup, LockedSelectionAuditSession, SelectionAuditError, SelectionAuditPhase,
    SelectionAuditRecord,
};

const CADENCE_TABLE: &str = "selection_v2_generation_acquisition_cadence_receipts";
const AUDIT_CLOSURE_TABLE: &str = "selection_v2_generation_acquisition_audit_closures";
const CADENCE_AUDIT_SUBJECT_PREFIX: &str = "generation_acquisition_cadence:";

#[derive(Debug, Error)]
pub(crate) enum GenerationJournalError {
    #[error("BR-193 generation journal database error: {0}")]
    Database(#[from] diesel::result::Error),
    #[error("BR-193 generation journal audit error: {0}")]
    Audit(#[from] SelectionAuditError),
    #[error("BR-193 generation journal invariant [{code}]: {detail}")]
    Invariant { code: &'static str, detail: String },
}

type JournalResult<T> = Result<T, GenerationJournalError>;

fn invariant(code: &'static str, detail: impl Into<String>) -> GenerationJournalError {
    GenerationJournalError::Invariant {
        code,
        detail: detail.into(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PersistedCadenceReceipt {
    receipt: VerifiedGenerationAcquisitionCadenceReceipt,
    audit_record_hash: String,
}

impl PersistedCadenceReceipt {
    pub(crate) const fn receipt(&self) -> &VerifiedGenerationAcquisitionCadenceReceipt {
        &self.receipt
    }

    pub(crate) fn audit_record_hash(&self) -> &str {
        &self.audit_record_hash
    }
}

#[derive(Debug, QueryableByName)]
struct CadenceRow {
    #[diesel(sql_type = Text)]
    cadence_receipt_id: String,
    #[diesel(sql_type = Text)]
    mode_namespace: String,
    #[diesel(sql_type = Text)]
    scheduler_cycle_id: String,
    #[diesel(sql_type = Binary)]
    canonical_bytes: Vec<u8>,
    #[diesel(sql_type = Text)]
    content_hash: String,
    #[diesel(sql_type = Text)]
    acquisition_started_at: String,
    #[diesel(sql_type = Text)]
    next_acquisition_eligible_at: String,
    #[diesel(sql_type = Nullable<Text>)]
    prior_cadence_receipt_hash: Option<String>,
    #[diesel(sql_type = Text)]
    boot_instance_id: String,
    #[diesel(sql_type = Text)]
    committed_at: String,
}

#[derive(Debug, QueryableByName)]
struct AuditClosureRow {
    #[diesel(sql_type = Text)]
    artifact_content_hash: String,
    #[diesel(sql_type = Text)]
    audit_subject_id: String,
    #[diesel(sql_type = Text)]
    audit_record_hash: String,
}

#[derive(Debug, QueryableByName)]
struct CountRow {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    count: i64,
}

pub(crate) struct GenerationAcquisitionJournalRepository;

impl GenerationAcquisitionJournalRepository {
    pub(crate) fn initialize(conn: &mut SqliteConnection) -> JournalResult<Self> {
        conn.batch_execute(
            "
            PRAGMA foreign_keys = ON;
            PRAGMA synchronous = FULL;

            CREATE TABLE IF NOT EXISTS selection_v2_generation_acquisition_cadence_receipts (
                cadence_receipt_id TEXT PRIMARY KEY NOT NULL,
                mode_namespace TEXT NOT NULL,
                scheduler_cycle_id TEXT NOT NULL UNIQUE,
                canonical_bytes BLOB NOT NULL,
                content_hash TEXT NOT NULL UNIQUE,
                acquisition_started_at TEXT NOT NULL,
                next_acquisition_eligible_at TEXT NOT NULL,
                prior_cadence_receipt_hash TEXT,
                boot_instance_id TEXT NOT NULL,
                committed_at TEXT NOT NULL,
                UNIQUE(mode_namespace, committed_at, cadence_receipt_id)
            ) STRICT;

            CREATE TABLE IF NOT EXISTS selection_v2_generation_acquisition_audit_closures (
                artifact_content_hash TEXT PRIMARY KEY NOT NULL
                    REFERENCES selection_v2_generation_acquisition_cadence_receipts(content_hash),
                audit_subject_id TEXT NOT NULL UNIQUE,
                audit_record_hash TEXT NOT NULL UNIQUE
            ) STRICT;

            CREATE TRIGGER IF NOT EXISTS selection_v2_generation_cadence_no_update
            BEFORE UPDATE ON selection_v2_generation_acquisition_cadence_receipts
            BEGIN
                SELECT RAISE(ABORT, 'selection-v2 generation cadence rows are append-only');
            END;

            CREATE TRIGGER IF NOT EXISTS selection_v2_generation_cadence_no_delete
            BEFORE DELETE ON selection_v2_generation_acquisition_cadence_receipts
            BEGIN
                SELECT RAISE(ABORT, 'selection-v2 generation cadence rows are append-only');
            END;

            CREATE TRIGGER IF NOT EXISTS selection_v2_generation_audit_closure_no_update
            BEFORE UPDATE ON selection_v2_generation_acquisition_audit_closures
            BEGIN
                SELECT RAISE(ABORT, 'selection-v2 generation audit closures are append-only');
            END;

            CREATE TRIGGER IF NOT EXISTS selection_v2_generation_audit_closure_no_delete
            BEFORE DELETE ON selection_v2_generation_acquisition_audit_closures
            BEGIN
                SELECT RAISE(ABORT, 'selection-v2 generation audit closures are append-only');
            END;
            ",
        )?;
        Ok(Self)
    }

    pub(crate) fn append_sync_read_back_cadence(
        &self,
        conn: &mut SqliteConnection,
        audit_session: &mut LockedSelectionAuditSession<'_>,
        receipt: &VerifiedGenerationAcquisitionCadenceReceipt,
        recorded_at: DateTime<FixedOffset>,
    ) -> JournalResult<PersistedCadenceReceipt> {
        let reparsed = parse_generation_acquisition_cadence_receipt(
            receipt.canonical_bytes(),
            receipt.content_hash(),
        )
        .map_err(|error| {
            invariant(
                "generation_state_ambiguous",
                format!("cadence capability failed exact revalidation: {error}"),
            )
        })?;
        if &reparsed != receipt {
            return Err(invariant(
                "generation_state_ambiguous",
                "cadence capability changed during exact revalidation",
            ));
        }

        immediate_transaction(conn, |conn| {
            validate_cadence_insert_order(conn, receipt)?;
            match load_cadence_by_id(conn, receipt.cadence_receipt_id())? {
                Some(existing) => verify_exact_cadence_row(&existing, receipt),
                None => {
                    if let Some(existing) =
                        load_cadence_by_cycle(conn, receipt.scheduler_cycle_id())?
                    {
                        return Err(invariant(
                            "generation_state_ambiguous",
                            format!(
                                "scheduler cycle {} already belongs to cadence {}",
                                receipt.scheduler_cycle_id(),
                                existing.cadence_receipt_id
                            ),
                        ));
                    }
                    insert_cadence_row(conn, receipt)?;
                    let inserted = load_cadence_by_id(conn, receipt.cadence_receipt_id())?
                        .ok_or_else(|| {
                            invariant(
                                "generation_state_ambiguous",
                                "inserted cadence receipt was not readable in its transaction",
                            )
                        })?;
                    verify_exact_cadence_row(&inserted, receipt)
                }
            }
        })?;

        let audit_subject_id = format!(
            "{CADENCE_AUDIT_SUBJECT_PREFIX}{}",
            receipt.cadence_receipt_id()
        );
        let audit_record_hash = match audit_session.lookup_exact(
            SelectionAuditPhase::V2GenerationPrepared,
            &audit_subject_id,
            receipt.content_hash(),
        )? {
            AuditExactLookup::Missing => {
                audit_session
                    .append(SelectionAuditRecord::new(
                        SelectionAuditPhase::V2GenerationPrepared,
                        audit_subject_id.clone(),
                        receipt.content_hash().to_owned(),
                        recorded_at,
                    ))?
                    .record_hash
            }
            AuditExactLookup::Exact(record) => record.record_hash,
            AuditExactLookup::ContentConflict { existing_record } => {
                return Err(invariant(
                    "generation_state_ambiguous",
                    format!(
                        "cadence audit subject conflicts with existing content {}",
                        existing_record.content_hash
                    ),
                ));
            }
        };

        immediate_transaction(conn, |conn| {
            match load_audit_closure(conn, receipt.content_hash())? {
                Some(existing) => {
                    if existing.audit_subject_id != audit_subject_id
                        || existing.audit_record_hash != audit_record_hash
                    {
                        return Err(invariant(
                            "generation_state_ambiguous",
                            "cadence audit closure conflicts with synced audit readback",
                        ));
                    }
                }
                None => {
                    diesel::sql_query(format!(
                        "INSERT INTO {AUDIT_CLOSURE_TABLE} (
                            artifact_content_hash,audit_subject_id,audit_record_hash
                         ) VALUES (?1,?2,?3)"
                    ))
                    .bind::<Text, _>(receipt.content_hash())
                    .bind::<Text, _>(&audit_subject_id)
                    .bind::<Text, _>(&audit_record_hash)
                    .execute(conn)?;
                }
            }
            let readback = load_audit_closure(conn, receipt.content_hash())?.ok_or_else(|| {
                invariant(
                    "generation_state_ambiguous",
                    "cadence audit closure was not readable in its transaction",
                )
            })?;
            if readback.audit_subject_id != audit_subject_id
                || readback.audit_record_hash != audit_record_hash
            {
                return Err(invariant(
                    "generation_state_ambiguous",
                    "cadence audit closure readback mismatch",
                ));
            }
            Ok(())
        })?;

        let durable = self
            .latest_cadence_for_namespace(conn, receipt.mode_namespace())?
            .ok_or_else(|| {
                invariant(
                    "generation_state_ambiguous",
                    "closed cadence receipt disappeared before final readback",
                )
            })?;
        if durable.receipt != *receipt || durable.audit_record_hash != audit_record_hash {
            return Err(invariant(
                "generation_state_ambiguous",
                "final cadence receipt/audit readback differs from requested closure",
            ));
        }
        Ok(durable)
    }

    /// Returns the current durable cadence authority after exact byte/hash and
    /// audit-closure readback. Recovery must resume this scheduler cycle; it
    /// must not allocate a replacement cadence receipt.
    pub(crate) fn latest_cadence_for_namespace(
        &self,
        conn: &mut SqliteConnection,
        mode_namespace: &str,
    ) -> JournalResult<Option<PersistedCadenceReceipt>> {
        let Some(row) = load_latest_cadence(conn, mode_namespace)? else {
            return Ok(None);
        };
        let receipt = receipt_from_row(&row)?;
        let closure = load_audit_closure(conn, receipt.content_hash())?.ok_or_else(|| {
            invariant(
                "generation_state_ambiguous",
                "latest cadence receipt has no durable audit closure",
            )
        })?;
        let expected_subject = format!(
            "{CADENCE_AUDIT_SUBJECT_PREFIX}{}",
            receipt.cadence_receipt_id()
        );
        if closure.artifact_content_hash != receipt.content_hash()
            || closure.audit_subject_id != expected_subject
        {
            return Err(invariant(
                "generation_state_ambiguous",
                "latest cadence audit closure does not bind the exact receipt",
            ));
        }
        Ok(Some(PersistedCadenceReceipt {
            receipt,
            audit_record_hash: closure.audit_record_hash,
        }))
    }
}

fn validate_cadence_insert_order(
    conn: &mut SqliteConnection,
    receipt: &VerifiedGenerationAcquisitionCadenceReceipt,
) -> JournalResult<()> {
    let Some(latest) = load_latest_cadence(conn, receipt.mode_namespace())? else {
        if receipt.prior_cadence_receipt_hash().is_some() {
            return Err(invariant(
                "generation_state_ambiguous",
                "first cadence receipt cannot name a prior receipt",
            ));
        }
        return Ok(());
    };
    if latest.cadence_receipt_id == receipt.cadence_receipt_id() {
        return Ok(());
    }
    let latest_receipt = receipt_from_row(&latest)?;
    if load_audit_closure(conn, latest_receipt.content_hash())?.is_none() {
        return Err(invariant(
            "generation_state_ambiguous",
            "prior cadence receipt is not audit closed",
        ));
    }
    if receipt.prior_cadence_receipt_hash() != Some(latest_receipt.content_hash()) {
        return Err(invariant(
            "generation_state_ambiguous",
            "new cadence receipt does not bind the exact prior receipt hash",
        ));
    }
    if receipt.acquisition_started_at() < latest_receipt.committed_at()
        || receipt.acquisition_started_at() < latest_receipt.next_acquisition_eligible_at()
    {
        return Err(invariant(
            "generation_state_ambiguous",
            "new cadence receipt regresses the clock or violates the durable eligibility window",
        ));
    }
    Err(invariant(
        "generation_cycle_not_terminal",
        "the prior cadence cycle has no exact terminal receipt",
    ))
}

fn insert_cadence_row(
    conn: &mut SqliteConnection,
    receipt: &VerifiedGenerationAcquisitionCadenceReceipt,
) -> JournalResult<()> {
    diesel::sql_query(format!(
        "INSERT INTO {CADENCE_TABLE} (
            cadence_receipt_id,mode_namespace,scheduler_cycle_id,
            canonical_bytes,content_hash,acquisition_started_at,
            next_acquisition_eligible_at,prior_cadence_receipt_hash,
            boot_instance_id,committed_at
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)"
    ))
    .bind::<Text, _>(receipt.cadence_receipt_id())
    .bind::<Text, _>(receipt.mode_namespace())
    .bind::<Text, _>(receipt.scheduler_cycle_id())
    .bind::<Binary, _>(receipt.canonical_bytes())
    .bind::<Text, _>(receipt.content_hash())
    .bind::<Text, _>(receipt.acquisition_started_at())
    .bind::<Text, _>(receipt.next_acquisition_eligible_at())
    .bind::<Nullable<Text>, _>(receipt.prior_cadence_receipt_hash())
    .bind::<Text, _>(receipt.boot_instance_id())
    .bind::<Text, _>(receipt.committed_at())
    .execute(conn)?;
    Ok(())
}

fn load_cadence_by_id(
    conn: &mut SqliteConnection,
    cadence_receipt_id: &str,
) -> JournalResult<Option<CadenceRow>> {
    diesel::sql_query(format!(
        "SELECT cadence_receipt_id,mode_namespace,scheduler_cycle_id,
                canonical_bytes,content_hash,acquisition_started_at,
                next_acquisition_eligible_at,prior_cadence_receipt_hash,
                boot_instance_id,committed_at
         FROM {CADENCE_TABLE}
         WHERE cadence_receipt_id=?1"
    ))
    .bind::<Text, _>(cadence_receipt_id)
    .get_result::<CadenceRow>(conn)
    .optional()
    .map_err(Into::into)
}

fn load_cadence_by_cycle(
    conn: &mut SqliteConnection,
    scheduler_cycle_id: &str,
) -> JournalResult<Option<CadenceRow>> {
    diesel::sql_query(format!(
        "SELECT cadence_receipt_id,mode_namespace,scheduler_cycle_id,
                canonical_bytes,content_hash,acquisition_started_at,
                next_acquisition_eligible_at,prior_cadence_receipt_hash,
                boot_instance_id,committed_at
         FROM {CADENCE_TABLE}
         WHERE scheduler_cycle_id=?1"
    ))
    .bind::<Text, _>(scheduler_cycle_id)
    .get_result::<CadenceRow>(conn)
    .optional()
    .map_err(Into::into)
}

fn load_latest_cadence(
    conn: &mut SqliteConnection,
    mode_namespace: &str,
) -> JournalResult<Option<CadenceRow>> {
    diesel::sql_query(format!(
        "SELECT cadence_receipt_id,mode_namespace,scheduler_cycle_id,
                canonical_bytes,content_hash,acquisition_started_at,
                next_acquisition_eligible_at,prior_cadence_receipt_hash,
                boot_instance_id,committed_at
         FROM {CADENCE_TABLE}
         WHERE mode_namespace=?1
         ORDER BY CAST(committed_at AS BLOB) DESC,
                  CAST(cadence_receipt_id AS BLOB) DESC
         LIMIT 1"
    ))
    .bind::<Text, _>(mode_namespace)
    .get_result::<CadenceRow>(conn)
    .optional()
    .map_err(Into::into)
}

fn load_audit_closure(
    conn: &mut SqliteConnection,
    content_hash: &str,
) -> JournalResult<Option<AuditClosureRow>> {
    diesel::sql_query(format!(
        "SELECT artifact_content_hash,audit_subject_id,audit_record_hash
         FROM {AUDIT_CLOSURE_TABLE}
         WHERE artifact_content_hash=?1"
    ))
    .bind::<Text, _>(content_hash)
    .get_result::<AuditClosureRow>(conn)
    .optional()
    .map_err(Into::into)
}

fn verify_exact_cadence_row(
    row: &CadenceRow,
    receipt: &VerifiedGenerationAcquisitionCadenceReceipt,
) -> JournalResult<()> {
    let readback = receipt_from_row(row)?;
    if readback != *receipt
        || row.mode_namespace != receipt.mode_namespace()
        || row.scheduler_cycle_id != receipt.scheduler_cycle_id()
        || row.acquisition_started_at != receipt.acquisition_started_at()
        || row.next_acquisition_eligible_at != receipt.next_acquisition_eligible_at()
        || row.prior_cadence_receipt_hash.as_deref() != receipt.prior_cadence_receipt_hash()
        || row.boot_instance_id != receipt.boot_instance_id()
        || row.committed_at != receipt.committed_at()
    {
        return Err(invariant(
            "generation_state_ambiguous",
            "cadence row differs from its exact canonical carrier",
        ));
    }
    Ok(())
}

fn receipt_from_row(
    row: &CadenceRow,
) -> JournalResult<VerifiedGenerationAcquisitionCadenceReceipt> {
    if row.cadence_receipt_id.trim().is_empty() {
        return Err(invariant(
            "generation_state_ambiguous",
            "cadence row identity is blank",
        ));
    }
    parse_generation_acquisition_cadence_receipt(&row.canonical_bytes, &row.content_hash).map_err(
        |error| {
            invariant(
                "generation_state_ambiguous",
                format!("persisted cadence bytes/hash are invalid: {error}"),
            )
        },
    )
}

enum ImmediateTransactionError {
    Primary(GenerationJournalError),
    Diesel(diesel::result::Error),
}

impl From<diesel::result::Error> for ImmediateTransactionError {
    fn from(error: diesel::result::Error) -> Self {
        Self::Diesel(error)
    }
}

fn immediate_transaction<T>(
    conn: &mut SqliteConnection,
    operation: impl FnOnce(&mut SqliteConnection) -> JournalResult<T>,
) -> JournalResult<T> {
    match conn.immediate_transaction::<T, ImmediateTransactionError, _>(|conn| {
        operation(conn).map_err(ImmediateTransactionError::Primary)
    }) {
        Ok(value) => Ok(value),
        Err(ImmediateTransactionError::Primary(error)) => Err(error),
        Err(ImmediateTransactionError::Diesel(error)) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::acquisition_v2::{
        build_generation_acquisition_cadence_receipt, AcquisitionModeNamespace,
    };
    use crate::selection::audit::SelectionAuditWriter;
    use chrono::DateTime;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TestRoot(PathBuf);

    impl TestRoot {
        fn new(label: &str) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root =
                std::fs::canonicalize(std::env::temp_dir()).expect("canonical isolated temp root");
            Self(root.join(format!(
                "stock-analysis-br193-generation-journal-{label}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            )))
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn receipt() -> VerifiedGenerationAcquisitionCadenceReceipt {
        let namespace =
            AcquisitionModeNamespace::for_test_code("TEST_CODE_selection_v2_scheduler").unwrap();
        build_generation_acquisition_cadence_receipt(
            "018f8f3e-7b2a-7abc-8def-1234567890a1",
            &namespace,
            "TEST_CODE_activation_run",
            "a".repeat(64),
            "018f8f3e-7b2a-7abc-8def-1234567890a2",
            "2026-07-31T01:02:03.123456789Z",
            None,
            "018f8f3e-7b2a-7abc-8def-1234567890a3",
            "2026-07-31T01:02:03.223456789Z",
        )
        .unwrap()
    }

    fn recorded_at() -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339("2026-07-31T01:02:03.323456789Z").unwrap()
    }

    fn audit_writer(root: &TestRoot) -> SelectionAuditWriter {
        let audit_root = root.path().join("audit");
        std::fs::create_dir_all(&audit_root).unwrap();
        SelectionAuditWriter::for_test_code_root(audit_root).unwrap()
    }

    #[test]
    fn cadence_is_durable_idempotent_and_audit_closed_after_restart() {
        let root = TestRoot::new("durable-restart");
        std::fs::create_dir_all(root.path()).unwrap();
        let database_path = root.path().join("TEST_CODE_selection_v2.db");
        let database_url = database_path.to_str().unwrap();
        let expected = receipt();

        {
            let mut conn = SqliteConnection::establish(database_url).unwrap();
            let repository = GenerationAcquisitionJournalRepository::initialize(&mut conn).unwrap();
            let writer = audit_writer(&root);
            let mut session = writer.locked_session().unwrap();
            let inserted = repository
                .append_sync_read_back_cadence(&mut conn, &mut session, &expected, recorded_at())
                .unwrap();
            assert_eq!(inserted.receipt(), &expected);
            assert!(!inserted.audit_record_hash().is_empty());

            let replayed = repository
                .append_sync_read_back_cadence(&mut conn, &mut session, &expected, recorded_at())
                .unwrap();
            assert_eq!(replayed, inserted);
            assert_eq!(session.validate().unwrap().record_count, 1);
            session.finish().unwrap();
        }

        let mut reopened = SqliteConnection::establish(database_url).unwrap();
        let repository = GenerationAcquisitionJournalRepository::initialize(&mut reopened).unwrap();
        let readback = repository
            .latest_cadence_for_namespace(&mut reopened, expected.mode_namespace())
            .unwrap()
            .expect("current durable cadence");
        assert_eq!(readback.receipt(), &expected);
    }

    #[test]
    fn cadence_recovers_sqlite_row_left_before_audit_append() {
        let root = TestRoot::new("recover-before-audit");
        std::fs::create_dir_all(root.path()).unwrap();
        let mut conn = SqliteConnection::establish(":memory:").unwrap();
        let repository = GenerationAcquisitionJournalRepository::initialize(&mut conn).unwrap();
        let writer = audit_writer(&root);
        let expected = receipt();
        immediate_transaction(&mut conn, |conn| insert_cadence_row(conn, &expected)).unwrap();

        let mut session = writer.locked_session().unwrap();
        let recovered = repository
            .append_sync_read_back_cadence(&mut conn, &mut session, &expected, recorded_at())
            .unwrap();
        assert_eq!(recovered.receipt(), &expected);
        assert_eq!(session.validate().unwrap().record_count, 1);
        session.finish().unwrap();
    }

    #[test]
    fn cadence_transaction_failure_writes_neither_row_nor_audit() {
        let root = TestRoot::new("transaction-failure");
        std::fs::create_dir_all(root.path()).unwrap();
        let mut conn = SqliteConnection::establish(":memory:").unwrap();
        let repository = GenerationAcquisitionJournalRepository::initialize(&mut conn).unwrap();
        let writer = audit_writer(&root);
        conn.batch_execute(
            "CREATE TRIGGER TEST_CODE_fail_cadence_insert
             BEFORE INSERT ON selection_v2_generation_acquisition_cadence_receipts
             BEGIN SELECT RAISE(ABORT, 'TEST_CODE injected transaction failure'); END;",
        )
        .unwrap();
        let expected = receipt();
        let mut session = writer.locked_session().unwrap();

        let error = repository
            .append_sync_read_back_cadence(&mut conn, &mut session, &expected, recorded_at())
            .expect_err("injected SQLite failure must abort before audit");
        assert!(matches!(error, GenerationJournalError::Database(_)));
        let rows = diesel::sql_query(format!("SELECT COUNT(*) AS count FROM {CADENCE_TABLE}"))
            .get_result::<CountRow>(&mut conn)
            .unwrap();
        assert_eq!(rows.count, 0);
        assert_eq!(session.validate().unwrap().record_count, 0);
        session.finish().unwrap();
    }
}
