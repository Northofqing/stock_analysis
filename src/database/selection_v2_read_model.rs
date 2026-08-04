//! BR-176/BR-178 receipt-verified schema-v2 authoritative reads.
//!
//! A SQLite receipt row is not authority by itself. This module keeps the
//! audit OS lock and a pinned SQLite read transaction together, validates the
//! complete receipt/audit closure, and materializes query results before
//! either boundary can be released.

use std::collections::{BTreeMap, HashMap, HashSet};
#[cfg(test)]
use std::fs;
#[cfg(all(test, unix))]
use std::os::unix::fs::MetadataExt;
#[cfg(test)]
use std::path::Path;

use chrono::{DateTime, FixedOffset, NaiveDate};
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Integer, Nullable, Text};
use serde::Serialize;
use thiserror::Error;

use super::selection_v2_repository::{
    classify_outcome_claim_lifecycle, latest_receipted_status_in_verified_snapshot,
    outcome_claim_lifecycles_in_verified_snapshot,
    verify_database_and_audit_for_recovery_in_current_snapshot,
    verify_database_and_audit_in_current_snapshot, verify_envelope_only_recovery_row,
    NonOutcomeRecoveryStageRequest, OutcomeClaimLifecycleClass, OutcomeClaimRecoveryMaterial,
    SelectionV2RepositoryError,
};
use super::DatabaseManager;
use crate::selection::audit::{
    LockedSelectionAuditSession, SelectionAuditError, SelectionAuditWriter,
};
use crate::selection::outcome_session_gate::{
    expected_wait_is_suppressed, validate_shanghai_tick_instant,
};
#[cfg(test)]
use crate::selection::schema_v2::DOMAIN_OUTCOME_DUE_DATABASE_OBJECT;
use crate::selection::schema_v2::{
    build_request_evidence, canonical_json, run_logical_subject_key, sha256_bytes, sha256_json,
    AdjustmentKind, DailyIntervalKind, OutcomeClaimDueBindingPreimage,
    OutcomeClaimStageInputPreimage, OutcomeMarketRequestParametersPreimage, OutcomePhase,
    OutcomeProviderRequestPreimage, OutcomeTradingDateVectorPreimage,
    ProviderCapabilityHashPreimage, RequestEvidencePreimage, RequestParametersPreimage,
    RunLogicalSubjectPreimage, RunStatus, SampleKeyPreimage,
    SelectionRecoveryEnvelopeRowContentPreimage, SubjectKind, VerifiedOutcomeAuditPrefixPreimage,
    VerifiedOutcomeDueDatabaseBindingPreimage, VerifiedOutcomeDueDatabaseObjectBindingPreimage,
    VerifiedOutcomeDueSnapshotPreimage, VerifiedOutcomeReceiptTuplePreimage,
    DOMAIN_OUTCOME_AUDIT_PREFIX, DOMAIN_OUTCOME_CLAIM_DUE_BINDING,
    DOMAIN_OUTCOME_DUE_DATABASE_BINDING, DOMAIN_OUTCOME_DUE_RECEIPT_TUPLE,
    DOMAIN_OUTCOME_MARKET_REQUEST, DOMAIN_PROVIDER_CAPABILITY, DOMAIN_RUN_LOGICAL_SUBJECT,
    DOMAIN_SAMPLE_KEY, DOMAIN_VERIFIED_OUTCOME_DUE_SNAPSHOT, OUTCOME_CLAIM_STAGE_PAYLOAD_SCHEMA,
    UPSTREAM_REVISION,
};

const MAX_TICK_LIMIT: i64 = 200;
const READ_MODEL_BINDING_DOMAIN: &str = "stock_analysis.br174.verified_selection_read_model.v1";
#[cfg(not(test))]
const PRODUCTION_DATABASE_RELATIVE_PATH: &str = "data/stock_analysis.db";
const OUTCOME_REQUEST_PROVIDER: &str = "magic-tdx";
const OUTCOME_REQUEST_CAPABILITY: &str = "MagicTdx-UnadjustedDailyBars";
const OUTCOME_REQUEST_CONTRACT: &str = "magic-market-core.MarketDataProvider.bars.v0.2.0";
const SELECTION_V2_SCHEMA_OBJECTS: [&str; 12] = [
    "selection_v2_recovery_envelopes",
    "selection_source_batch_attempts",
    "selection_source_facts_v2",
    "selection_source_fact_attempts",
    "selection_relation_attempts",
    "selection_evaluation_attempts",
    "selection_samples",
    "selection_rejections",
    "selection_sample_outcomes",
    "selection_outcome_attempts",
    "selection_v2_run_stages",
    "selection_v2_commit_receipts",
];

#[derive(Debug, Error)]
pub enum SelectionV2ReadModelError {
    #[error("selection-v2 read-model database error: {0}")]
    Database(#[from] diesel::result::Error),
    #[error("selection-v2 read-model repository verification failed: {0}")]
    Repository(#[from] SelectionV2RepositoryError),
    #[error("selection-v2 read-model audit verification failed: {0}")]
    Audit(#[from] SelectionAuditError),
    #[error("selection-v2 read-model database manager failed: {0}")]
    DatabaseManager(String),
    #[error("selection-v2 read-model integrity failure [{code}]: {detail}")]
    Integrity { code: &'static str, detail: String },
}

pub type SelectionV2ReadModelResult<T> = Result<T, SelectionV2ReadModelError>;

fn integrity(code: &'static str, detail: impl Into<String>) -> SelectionV2ReadModelError {
    SelectionV2ReadModelError::Integrity {
        code,
        detail: detail.into(),
    }
}

/// Runs one authoritative query against a receipt snapshot pinned before the
/// complete audit/DB reconciliation.
///
/// The higher-ranked closure cannot retain the read model (which borrows the
/// transaction connection), while its owned materialized result may leave the
/// transaction. The caller continues to own the audit session, so the same OS
/// lock is retained for the full closure.
fn with_verified_selection_read_model_snapshot<T>(
    conn: &mut SqliteConnection,
    audit_session: &mut LockedSelectionAuditSession<'_>,
    database_binding: VerifiedOutcomeDueDatabaseBindingPreimage,
    purpose: VerifiedReadPurpose,
    query: impl for<'snapshot> FnOnce(
        &mut VerifiedSelectionReadModel<'snapshot>,
    ) -> SelectionV2ReadModelResult<T>,
) -> SelectionV2ReadModelResult<T> {
    let database_binding_hash = sha256_json(&database_binding).map_err(|error| {
        integrity(
            "database_binding_hash_failed",
            format!("cannot hash verified outcome database binding: {error}"),
        )
    })?;
    if database_binding_hash.trim().is_empty() {
        return Err(integrity(
            "database_binding_hash_missing",
            "verified read model requires a fixed database namespace binding",
        ));
    }
    conn.transaction::<T, SelectionV2ReadModelError, _>(|conn| {
        let receipt_high_water = load_receipt_high_water(conn)?;
        if receipt_high_water != database_binding.receipt_snapshot_high_water_rowid {
            return Err(integrity(
                "database_binding_receipt_high_water_changed",
                "receipt high-water advanced before the verified read transaction was pinned",
            ));
        }
        let high_water_hash = if receipt_high_water == 0 {
            None
        } else {
            Some(load_receipt_hash_at_rowid(conn, receipt_high_water)?)
        };
        if high_water_hash != database_binding.receipt_snapshot_high_water_content_hash {
            return Err(integrity(
                "database_binding_receipt_high_water_hash_mismatch",
                "receipt high-water hash differs from the pinned database binding",
            ));
        }
        let audit_snapshot = match purpose {
            VerifiedReadPurpose::Authoritative => {
                verify_database_and_audit_in_current_snapshot(conn, audit_session)?
            }
            VerifiedReadPurpose::PersistenceRecovery => {
                verify_database_and_audit_for_recovery_in_current_snapshot(conn, audit_session)?
            }
        };
        let receipts = load_verified_receipts(conn, receipt_high_water)?;
        let audit_record_count = audit_snapshot.validation().record_count;
        let audit_tail_hash = audit_snapshot.validation().tail_hash.clone();
        let binding_hash = sha256_json(&ReadModelBindingPreimage {
            domain: READ_MODEL_BINDING_DOMAIN,
            database_binding_hash: &database_binding_hash,
            receipt_high_water,
            receipt_subject_ids_sorted: receipts.keys().cloned().collect(),
            audit_record_count,
            audit_tail_hash: audit_tail_hash.as_deref(),
        })
        .map_err(|error| {
            integrity(
                "read_model_binding_hash_failed",
                format!("cannot bind verified receipt/audit high-water: {error}"),
            )
        })?;

        let mut model = VerifiedSelectionReadModel {
            conn,
            receipt_high_water,
            receipts,
            audit_record_count,
            audit_tail_hash,
            binding_hash,
            database_binding,
            database_binding_hash,
            audit_record_hashes: audit_snapshot
                .records()
                .iter()
                .map(|record| record.record_hash.clone())
                .collect(),
        };
        let result = query(&mut model)?;

        let ending_receipt_high_water = load_receipt_high_water(model.conn)?;
        if ending_receipt_high_water != model.receipt_high_water {
            return Err(integrity(
                "receipt_snapshot_high_water_changed",
                format!(
                    "pinned receipt high-water changed from {} to {}",
                    model.receipt_high_water, ending_receipt_high_water
                ),
            ));
        }
        let ending_audit = audit_session.validated_records().map_err(|error| {
            integrity(
                "audit_snapshot_revalidation_failed",
                format!("cannot revalidate locked audit chain: {error}"),
            )
        })?;
        if ending_audit.validation().record_count != model.audit_record_count
            || ending_audit.validation().tail_hash != model.audit_tail_hash
        {
            return Err(integrity(
                "audit_snapshot_high_water_changed",
                "locked audit record count/tail changed during authoritative read",
            ));
        }
        Ok(result)
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VerifiedReadPurpose {
    Authoritative,
    PersistenceRecovery,
}

impl DatabaseManager {
    /// Executes one schema-v2 authoritative read against the process-owned
    /// production database and the fixed production selection-audit root.
    ///
    /// No production caller can supply a SQLite connection, database path or
    /// audit namespace. Tests remain physically isolated by `DatabaseManager`
    /// and the audit writer's cfg(test)-only namespace.
    pub(crate) fn with_verified_selection_v2_read_model<T>(
        &self,
        query: impl for<'snapshot> FnOnce(
            &mut VerifiedSelectionReadModel<'snapshot>,
        ) -> SelectionV2ReadModelResult<T>,
    ) -> SelectionV2ReadModelResult<T> {
        let audit_writer = SelectionAuditWriter::production()?;
        let mut audit_session = audit_writer.locked_session()?;
        let mut conn = self
            .get_conn()
            .map_err(|error| SelectionV2ReadModelError::DatabaseManager(error.to_string()))?;
        let database_binding = self.selection_v2_database_binding(&mut conn)?;
        with_verified_selection_read_model_snapshot(
            &mut conn,
            &mut audit_session,
            database_binding,
            VerifiedReadPurpose::Authoritative,
            query,
        )
    }

    pub(crate) fn with_verified_selection_v2_recovery_model<T>(
        &self,
        query: impl for<'snapshot> FnOnce(
            &mut VerifiedSelectionReadModel<'snapshot>,
        ) -> SelectionV2ReadModelResult<T>,
    ) -> SelectionV2ReadModelResult<T> {
        let audit_writer = SelectionAuditWriter::production()?;
        let mut audit_session = audit_writer.locked_session()?;
        let mut conn = self
            .get_conn()
            .map_err(|error| SelectionV2ReadModelError::DatabaseManager(error.to_string()))?;
        let database_binding = self.selection_v2_database_binding(&mut conn)?;
        with_verified_selection_read_model_snapshot(
            &mut conn,
            &mut audit_session,
            database_binding,
            VerifiedReadPurpose::PersistenceRecovery,
            query,
        )
    }

    /// Re-reads the due set after the caller has acquired the fixed
    /// logical-subject lock. Only the exact previously verified snapshot may
    /// advance to claim creation; any receipt/audit/database high-water change
    /// makes the capability superseded and leaves claim creation to a later
    /// scheduler tick.
    pub(crate) fn revalidate_outcome_due_for_claim(
        &self,
        expected: &VerifiedOutcomeDue,
        tick_at: DateTime<FixedOffset>,
    ) -> SelectionV2ReadModelResult<Option<VerifiedOutcomeDue>> {
        let expected_logical_subject_key = expected.logical_subject_key.clone();
        let expected_snapshot_hash = expected.verified_due_snapshot_hash.clone();
        self.with_verified_selection_v2_read_model(|model| {
            Ok(model
                .due_v2_outcomes(tick_at, MAX_TICK_LIMIT)?
                .into_iter()
                .find(|candidate| {
                    candidate.logical_subject_key == expected_logical_subject_key
                        && candidate.verified_due_snapshot_hash == expected_snapshot_hash
                }))
        })
    }

    pub(crate) fn revalidate_outcome_settlement_recovery(
        &self,
        logical_subject_key: &str,
        claim_id: &str,
        planned_outcome_run_id: &str,
        expected_class: OutcomeClaimLifecycleClass,
    ) -> SelectionV2ReadModelResult<Option<VerifiedOutcomeSettlementRecovery>> {
        self.with_verified_selection_v2_recovery_model(|model| {
            Ok(model
                .outcome_settlement_recovery()?
                .into_iter()
                .find(|candidate| {
                    let (candidate_claim_id, candidate_planned_id, candidate_class) =
                        candidate.stable_identity();
                    candidate.logical_subject_key() == logical_subject_key
                        && candidate_claim_id == claim_id
                        && candidate_planned_id == planned_outcome_run_id
                        && candidate_class == expected_class
                }))
        })
    }

    fn selection_v2_database_binding(
        &self,
        conn: &mut super::DbConnection,
    ) -> SelectionV2ReadModelResult<VerifiedOutcomeDueDatabaseBindingPreimage> {
        #[cfg(not(test))]
        {
            let proof = self
                .selection_connection_bound_proof(conn)
                .map_err(|error| {
                    integrity(
                        "production_database_connection_unverified",
                        format!(
                            "actual SQLite checkout is not bound to GlobalSchema authority: {error}"
                        ),
                    )
                })?;
            let object_binding = proof.into_preimage();
            if object_binding.database_relative_path != PRODUCTION_DATABASE_RELATIVE_PATH {
                return Err(integrity(
                    "production_database_relative_identity_mismatch",
                    format!(
                        "owner-fixed database identity is {}, expected {}",
                        object_binding.database_relative_path, PRODUCTION_DATABASE_RELATIVE_PATH
                    ),
                ));
            }
            finish_database_binding(
                conn,
                object_binding,
                PRODUCTION_DATABASE_RELATIVE_PATH.into(),
                "production",
            )
        }

        #[cfg(test)]
        {
            build_test_database_binding(conn, super::unit_test_database_path())
        }
    }
}

#[cfg(test)]
fn build_test_database_binding(
    conn: &mut SqliteConnection,
    database_path: &Path,
) -> SelectionV2ReadModelResult<VerifiedOutcomeDueDatabaseBindingPreimage> {
    #[cfg(not(unix))]
    {
        let _ = (conn, database_path);
        return Err(integrity(
            "outcome_database_binding_platform_unsupported",
            "verified outcome database object identity requires Unix metadata",
        ));
    }

    #[cfg(unix)]
    {
        let parent = database_path.parent().ok_or_else(|| {
            integrity(
                "test_database_parent_missing",
                "test database path has no parent directory",
            )
        })?;
        let file_name = database_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                integrity(
                    "test_database_filename_invalid",
                    "test database filename is not canonical UTF-8",
                )
            })?;
        let manifest_root = fs::canonicalize(parent).map_err(|error| {
            integrity(
                "test_manifest_root_unavailable",
                format!("cannot canonicalize test database parent: {error}"),
            )
        })?;
        let database_relative_path = file_name.to_owned();
        let canonical_database = fs::canonicalize(database_path).map_err(|error| {
            integrity(
                "database_object_unavailable",
                format!("cannot canonicalize database object: {error}"),
            )
        })?;
        if canonical_database != manifest_root.join(&database_relative_path) {
            return Err(integrity(
                "database_relative_path_mismatch",
                "database object does not equal the pinned root plus canonical relative path",
            ));
        }
        let root_metadata = fs::metadata(&manifest_root).map_err(|error| {
            integrity(
                "manifest_root_metadata_unavailable",
                format!("cannot stat pinned manifest root: {error}"),
            )
        })?;
        let database_metadata = fs::metadata(&canonical_database).map_err(|error| {
            integrity(
                "database_metadata_unavailable",
                format!("cannot stat pinned database object: {error}"),
            )
        })?;
        if !root_metadata.is_dir() || !database_metadata.is_file() {
            return Err(integrity(
                "database_object_type_invalid",
                "pinned manifest root must be a directory and database must be a regular file",
            ));
        }
        let root_path = manifest_root.to_str().ok_or_else(|| {
            integrity(
                "manifest_root_not_utf8",
                "pinned manifest root cannot be represented as UTF-8",
            )
        })?;
        let object_binding = VerifiedOutcomeDueDatabaseObjectBindingPreimage {
            domain: DOMAIN_OUTCOME_DUE_DATABASE_OBJECT.into(),
            manifest_root_canonical_path: root_path.into(),
            manifest_root_device: root_metadata.dev(),
            manifest_root_inode: root_metadata.ino(),
            manifest_root_mode: root_metadata.mode(),
            database_relative_path: database_relative_path.clone(),
            database_device: database_metadata.dev(),
            database_inode: database_metadata.ino(),
            database_mode: database_metadata.mode(),
        };
        finish_database_binding(conn, object_binding, database_relative_path, "test")
    }
}

fn finish_database_binding(
    conn: &mut SqliteConnection,
    object_binding: VerifiedOutcomeDueDatabaseObjectBindingPreimage,
    database_relative_path: String,
    scope: &str,
) -> SelectionV2ReadModelResult<VerifiedOutcomeDueDatabaseBindingPreimage> {
    let object_binding_hash = sha256_json(&object_binding).map_err(|error| {
        integrity(
            "database_object_binding_hash_failed",
            format!("cannot hash pinned database object binding: {error}"),
        )
    })?;
    let application_id = pragma_u32(conn, "PRAGMA application_id", "sqlite_application_id")?;
    let user_version = pragma_u32(conn, "PRAGMA user_version", "sqlite_user_version")?;
    let sqlite_schema_hash = selection_v2_schema_hash(conn)?;
    let receipt_snapshot_high_water_rowid = load_receipt_high_water(conn)?;
    let receipt_snapshot_high_water_content_hash = if receipt_snapshot_high_water_rowid == 0 {
        None
    } else {
        Some(load_receipt_hash_at_rowid(
            conn,
            receipt_snapshot_high_water_rowid,
        )?)
    };
    Ok(VerifiedOutcomeDueDatabaseBindingPreimage {
        domain: DOMAIN_OUTCOME_DUE_DATABASE_BINDING.into(),
        scope: scope.into(),
        object_binding,
        object_binding_hash,
        database_relative_path,
        sqlite_application_id: application_id,
        sqlite_user_version: user_version,
        sqlite_schema_hash,
        receipt_snapshot_high_water_rowid,
        receipt_snapshot_high_water_content_hash,
    })
}

fn pragma_u32(
    conn: &mut SqliteConnection,
    sql: &'static str,
    field: &'static str,
) -> SelectionV2ReadModelResult<u32> {
    let value = diesel::sql_query(sql)
        .get_result::<IntegerValueRow>(conn)?
        .value;
    u32::try_from(value).map_err(|_| {
        integrity(
            "sqlite_pragma_out_of_range",
            format!("{field}={value} is outside u32"),
        )
    })
}

fn selection_v2_schema_hash(conn: &mut SqliteConnection) -> SelectionV2ReadModelResult<String> {
    let rows = diesel::sql_query(
        "SELECT type AS object_type, name, tbl_name, sql
         FROM sqlite_schema
         WHERE type='table' AND name IN (
             'selection_v2_recovery_envelopes',
             'selection_source_batch_attempts',
             'selection_source_facts_v2',
             'selection_source_fact_attempts',
             'selection_relation_attempts',
             'selection_evaluation_attempts',
             'selection_samples',
             'selection_rejections',
             'selection_sample_outcomes',
             'selection_outcome_attempts',
             'selection_v2_run_stages',
             'selection_v2_commit_receipts'
         )
         ORDER BY type ASC, name ASC, tbl_name ASC, sql ASC",
    )
    .load::<SchemaObjectRow>(conn)?;
    if rows.len() != SELECTION_V2_SCHEMA_OBJECTS.len() {
        return Err(integrity(
            "selection_v2_schema_object_count_invalid",
            format!(
                "expected {} v2 tables, found {}",
                SELECTION_V2_SCHEMA_OBJECTS.len(),
                rows.len()
            ),
        ));
    }
    let names = rows
        .iter()
        .map(|row| row.name.as_str())
        .collect::<HashSet<_>>();
    if SELECTION_V2_SCHEMA_OBJECTS
        .iter()
        .any(|expected| !names.contains(expected))
        || rows.iter().any(|row| {
            row.object_type != "table"
                || row.tbl_name != row.name
                || row.sql.as_deref().is_none_or(str::is_empty)
        })
    {
        return Err(integrity(
            "selection_v2_schema_object_set_invalid",
            "v2 sqlite_schema rows do not equal the frozen twelve-table set",
        ));
    }
    sha256_json(&rows).map_err(|error| {
        integrity(
            "selection_v2_schema_hash_failed",
            format!("cannot hash ordered sqlite_schema rows: {error}"),
        )
    })
}

fn load_receipt_hash_at_rowid(
    conn: &mut SqliteConnection,
    rowid: i64,
) -> SelectionV2ReadModelResult<String> {
    Ok(diesel::sql_query(
        "SELECT content_hash AS value
         FROM selection_v2_commit_receipts
         WHERE rowid=?",
    )
    .bind::<BigInt, _>(rowid)
    .get_result::<TextValueRow>(conn)?
    .value)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LatestOutcomeDueDisposition {
    Eligible,
    SuppressedExpectedWait,
    Terminal,
}

fn classify_latest_outcome_due_status(
    latest_status: Option<RunStatus>,
    stored_due_date: NaiveDate,
    tick_at: DateTime<FixedOffset>,
) -> SelectionV2ReadModelResult<LatestOutcomeDueDisposition> {
    validate_shanghai_tick_instant(&tick_at)
        .map_err(|error| integrity(error.reason_code(), error.to_string()))?;
    match latest_status {
        Some(RunStatus::Settled | RunStatus::FailedNonRetryable) => {
            Ok(LatestOutcomeDueDisposition::Terminal)
        }
        Some(RunStatus::ExpectedWait)
            if expected_wait_is_suppressed(stored_due_date, tick_at)
                .map_err(|error| integrity(error.reason_code(), error.to_string()))? =>
        {
            Ok(LatestOutcomeDueDisposition::SuppressedExpectedWait)
        }
        _ => Ok(LatestOutcomeDueDisposition::Eligible),
    }
}

/// Non-forgeable receipt/audit verified read capability.
///
/// Fields and construction are private, and the capability borrows the pinned
/// transaction connection. It intentionally does not implement Clone,
/// Serialize, Deserialize or Default.
#[must_use = "authoritative schema-v2 reads must execute through this verified snapshot"]
pub struct VerifiedSelectionReadModel<'snapshot> {
    conn: &'snapshot mut SqliteConnection,
    receipt_high_water: i64,
    receipts: BTreeMap<String, VerifiedReceipt>,
    audit_record_count: usize,
    audit_tail_hash: Option<String>,
    binding_hash: String,
    database_binding: VerifiedOutcomeDueDatabaseBindingPreimage,
    database_binding_hash: String,
    audit_record_hashes: Vec<String>,
}

impl VerifiedSelectionReadModel<'_> {
    pub fn recovery_queues(&mut self) -> SelectionV2ReadModelResult<VerifiedRecoveryQueues> {
        let envelope_rows = diesel::sql_query(
            "SELECT e.subject_kind, e.stage_run_id AS subject_id,
                    e.logical_subject_key, e.enveloped_at AS order_at,
                    e.payload_schema, e.payload_json, e.payload_json_hash,
                    e.in_memory_payload_hash, e.config_activation_run_id, e.config_hash,
                    e.enveloped_at
             FROM selection_v2_recovery_envelopes e
             WHERE NOT EXISTS (
                 SELECT 1 FROM selection_v2_run_stages s
                 WHERE s.subject_id=e.stage_run_id
             )
             ORDER BY e.enveloped_at ASC, e.stage_run_id ASC",
        )
        .load::<RecoveryRow>(self.conn)?;
        let manifested_rows = diesel::sql_query(
            "SELECT s.subject_kind, s.subject_id, s.logical_subject_key,
                    s.staged_at AS order_at,
                    e.payload_schema, e.payload_json, e.payload_json_hash,
                    e.in_memory_payload_hash, e.config_activation_run_id, e.config_hash,
                    e.enveloped_at
             FROM selection_v2_run_stages s
             INNER JOIN selection_v2_recovery_envelopes e
                     ON e.subject_kind=s.subject_kind
                    AND e.stage_run_id=s.subject_id
             WHERE NOT EXISTS (
                 SELECT 1 FROM selection_v2_commit_receipts r
                 WHERE r.subject_kind=s.subject_kind
                   AND r.subject_id=s.subject_id
             )
             ORDER BY s.staged_at ASC, s.subject_id ASC",
        )
        .load::<RecoveryRow>(self.conn)?;

        let mut seen = HashSet::new();
        let mut envelope_only = Vec::with_capacity(envelope_rows.len());
        for row in envelope_rows {
            verify_envelope_only_recovery_row(self.conn, &row.subject_id)?;
            if !seen.insert(row.subject_id.clone()) {
                return Err(integrity(
                    "recovery_queue_duplicate_subject",
                    format!("envelope-only queue repeats {}", row.subject_id),
                ));
            }
            envelope_only.push(VerifiedRecoveryRun::try_from_row(row)?);
        }
        let mut manifested_unreceipted = Vec::with_capacity(manifested_rows.len());
        for row in manifested_rows {
            if !seen.insert(row.subject_id.clone()) {
                return Err(integrity(
                    "recovery_queue_overlap",
                    format!(
                        "run {} entered both envelope-only and manifested queues",
                        row.subject_id
                    ),
                ));
            }
            manifested_unreceipted.push(VerifiedRecoveryRun::try_from_row(row)?);
        }

        Ok(VerifiedRecoveryQueues {
            envelope_only,
            manifested_unreceipted,
            read_model_binding_hash: self.binding_hash.clone(),
        })
    }

    /// Returns only exact, unclosed BR-178 claim lifecycles. `ClaimActive`
    /// carries the original receipted claim identity; `OutcomeRecovery`
    /// carries the already persisted outcome payload and therefore cannot
    /// trigger a provider refetch.
    pub(crate) fn outcome_settlement_recovery(
        &mut self,
    ) -> SelectionV2ReadModelResult<Vec<VerifiedOutcomeSettlementRecovery>> {
        let mut recovery = Vec::new();
        for lifecycle in outcome_claim_lifecycles_in_verified_snapshot(self.conn)? {
            let class = lifecycle.class();
            let Some(material) = lifecycle.into_recovery_material() else {
                continue;
            };
            recovery.push(VerifiedOutcomeSettlementRecovery::try_from_material(
                class, material,
            )?);
        }
        Ok(recovery)
    }

    /// Materializes the BR-178 due set at one exact Shanghai tick instant.
    ///
    /// Only the earliest missing phase per sample is eligible. A latest
    /// receipted `ExpectedWait` remains suppressed until the stored due date's
    /// `15:00:00.000000001 +08:00` deadline, after which its receipt is bound
    /// into the next attempt lineage together with the verified
    /// activation/ingress/generation and preceding outcome receipts.
    pub fn due_v2_outcomes(
        &mut self,
        tick_at: DateTime<FixedOffset>,
        limit: i64,
    ) -> SelectionV2ReadModelResult<Vec<VerifiedOutcomeDue>> {
        validate_tick_limit(limit)?;
        validate_shanghai_tick_instant(&tick_at)
            .map_err(|error| integrity(error.reason_code(), error.to_string()))?;
        let as_of = tick_at.date_naive();

        let samples = load_receipted_samples(self.conn)?;
        let mut due = Vec::new();
        for sample in samples {
            require_exact_receipt(
                &self.receipts,
                &sample.activation_receipt_subject_id,
                SubjectKind::ConfigActivation,
                &sample.activation_receipt_content_hash,
            )?;
            require_exact_receipt(
                &self.receipts,
                &sample.ingress_receipt_subject_id,
                SubjectKind::IngressRun,
                &sample.ingress_receipt_content_hash,
            )?;
            require_exact_receipt(
                &self.receipts,
                &sample.generation_receipt_subject_id,
                SubjectKind::GenerationRun,
                &sample.generation_receipt_content_hash,
            )?;

            let schedule = VerifiedSchedule::from_sample(&sample)?;
            let settled = load_settled_phases(self.conn, &self.receipts, &sample, &schedule)?;
            let Some(phase) = OutcomePhaseOrder::ALL
                .into_iter()
                .find(|phase| !settled.contains_key(phase))
            else {
                continue;
            };
            let stored_due_date = schedule.due_date(phase);
            if stored_due_date > as_of {
                continue;
            }
            let sample_key_preimage = verified_sample_key_preimage(&sample)?;
            let logical_subject_key = outcome_logical_subject_key(&sample, phase, stored_due_date)?;
            if classify_outcome_claim_lifecycle(self.conn, &logical_subject_key)?
                .is_some_and(|lifecycle| lifecycle.blocks_new_due())
            {
                continue;
            }
            if has_unreceipted_logical_subject(self.conn, &logical_subject_key)? {
                continue;
            }
            let latest_status = latest_receipted_status_in_verified_snapshot(
                self.conn,
                SubjectKind::OutcomeRun,
                &logical_subject_key,
            )?;
            if classify_latest_outcome_due_status(latest_status, stored_due_date, tick_at)?
                != LatestOutcomeDueDisposition::Eligible
            {
                continue;
            }

            let preceding = preceding_settled(phase, &settled)?;
            let t0 = if phase == OutcomePhase::T0Close {
                None
            } else {
                Some(
                    settled
                        .get(&OutcomePhase::T0Close)
                        .ok_or_else(|| {
                            integrity(
                                "outcome_t0_baseline_missing",
                                format!(
                                    "sample {} phase {} lacks receipted T0 baseline",
                                    sample.sample_key,
                                    phase.as_str()
                                ),
                            )
                        })?
                        .clone(),
                )
            };
            let window = schedule.window(phase);
            let claim_material = build_outcome_claim_material(
                &sample,
                &sample_key_preimage,
                &logical_subject_key,
                phase,
                stored_due_date,
                &schedule,
                &window,
                &preceding,
                t0.as_ref(),
                &self.receipts,
                &self.database_binding,
                &self.database_binding_hash,
                &self.audit_record_hashes,
            )?;
            let t0_baseline = t0.as_ref().map(|phase| VerifiedT0Baseline {
                close: phase.close.clone(),
                volume: phase.volume.clone(),
            });
            due.push(VerifiedOutcomeDue {
                sample_key: sample.sample_key,
                canonical_stock_code: sample.canonical_stock_code,
                canonical_market: sample.canonical_market,
                sample_key_preimage,
                config_activation_run_id: sample.config_activation_run_id,
                config_hash: sample.config_hash,
                phase,
                stored_due_date,
                calendar_version: sample.calendar_version,
                calendar_hash: sample.calendar_hash,
                trading_date_vector: schedule.vector,
                trading_date_vector_hash: schedule.vector_hash,
                applicable_trading_dates: window.applicable_dates,
                t0: t0_baseline,
                logical_subject_key,
                verified_due_snapshot_hash: claim_material.verified_due_snapshot_hash,
                claim_due_binding: claim_material.due_binding,
                claim_due_binding_hash: claim_material.due_binding_hash,
                provider_request_evidence: claim_material.provider_request_evidence,
                provider_request_hash: claim_material.provider_request_hash,
                provider_transport_request: None,
                provider_transport_request_hash: None,
            });
        }
        due.sort_by(|left, right| {
            (
                left.stored_due_date,
                left.sample_key.as_str(),
                OutcomePhaseOrder::ordinal(left.phase),
            )
                .cmp(&(
                    right.stored_due_date,
                    right.sample_key.as_str(),
                    OutcomePhaseOrder::ordinal(right.phase),
                ))
        });
        due.truncate(limit as usize);
        Ok(due)
    }
}

/// Two disjoint, fully ordered recovery inventories for the stage-specific
/// recovery owner. Outcome due reads use exact receipt dependencies and
/// per-logical-subject anti-joins instead of treating unrelated partial work
/// as a process-wide provider mutex.
#[must_use = "recovery inventories must be handed to their stage-specific owner"]
pub struct VerifiedRecoveryQueues {
    envelope_only: Vec<VerifiedRecoveryRun>,
    manifested_unreceipted: Vec<VerifiedRecoveryRun>,
    read_model_binding_hash: String,
}

impl VerifiedRecoveryQueues {
    pub fn is_empty(&self) -> bool {
        self.envelope_only.is_empty() && self.manifested_unreceipted.is_empty()
    }

    pub fn len(&self) -> usize {
        self.envelope_only.len() + self.manifested_unreceipted.len()
    }

    pub fn envelope_only(&self) -> &[VerifiedRecoveryRun] {
        &self.envelope_only
    }

    pub fn manifested_unreceipted(&self) -> &[VerifiedRecoveryRun] {
        &self.manifested_unreceipted
    }

    pub fn read_model_binding_hash(&self) -> &str {
        &self.read_model_binding_hash
    }

    /// Consumes the two BR-176 queue partitions in their registered order:
    /// envelope-only by `(enveloped_at, stage_run_id)`, followed by manifested
    /// unreceipted by `(staged_at, stage_run_id)`. Outcome claim/run work
    /// remains in the BR-178 lifecycle owner.
    pub(crate) fn into_ordered_non_outcome(self) -> Vec<NonOutcomeRecoveryStageRequest> {
        self.envelope_only
            .into_iter()
            .chain(self.manifested_unreceipted)
            .filter_map(VerifiedRecoveryRun::into_non_outcome)
            .collect()
    }
}

#[must_use = "a verified recovery run must be recovered before new provider acquisition"]
pub struct VerifiedRecoveryRun {
    subject_kind: SubjectKind,
    subject_id: String,
    logical_subject_key: String,
    order_at: String,
    #[allow(
        dead_code,
        reason = "BR-183 deliberately keeps outcome-claim recovery dormant until selection-v2 activation"
    )]
    outcome_claim: Option<VerifiedOutcomeClaimRecovery>,
    non_outcome: Option<NonOutcomeRecoveryStageRequest>,
}

impl VerifiedRecoveryRun {
    fn try_from_row(row: RecoveryRow) -> SelectionV2ReadModelResult<Self> {
        let subject_kind = parse_subject_kind(&row.subject_kind)?;
        let outcome_claim = if subject_kind == SubjectKind::OutcomeClaim {
            Some(VerifiedOutcomeClaimRecovery::try_from_row(&row)?)
        } else {
            None
        };
        let recovery_envelope = SelectionRecoveryEnvelopeRowContentPreimage {
            domain: crate::selection::schema_v2::DOMAIN_RECOVERY_ENVELOPE_ROW.into(),
            stage_run_id: row.subject_id.clone(),
            subject_kind,
            logical_subject_key: row.logical_subject_key.clone(),
            payload_schema: row.payload_schema.clone(),
            payload_json: row.payload_json.clone(),
            payload_json_hash: row.payload_json_hash.clone(),
            in_memory_payload_hash: row.in_memory_payload_hash.clone(),
            config_activation_run_id: row.config_activation_run_id.clone(),
            config_hash: row.config_hash.clone(),
            enveloped_at: row.enveloped_at.clone(),
        };
        let non_outcome = NonOutcomeRecoveryStageRequest::try_from_envelope(recovery_envelope)?;
        Ok(Self {
            subject_kind,
            subject_id: row.subject_id,
            logical_subject_key: row.logical_subject_key,
            order_at: row.order_at,
            outcome_claim,
            non_outcome,
        })
    }

    pub fn subject_kind(&self) -> SubjectKind {
        self.subject_kind
    }

    pub fn subject_id(&self) -> &str {
        &self.subject_id
    }

    pub fn logical_subject_key(&self) -> &str {
        &self.logical_subject_key
    }

    pub fn order_at(&self) -> &str {
        &self.order_at
    }

    /// Consumes the queue row and yields the exact typed claim-recovery
    /// capability. A caller cannot retain the generic row and replay it twice.
    #[allow(
        dead_code,
        reason = "BR-183 deliberately keeps outcome-claim recovery dormant until selection-v2 activation"
    )]
    pub(crate) fn into_outcome_claim(self) -> Option<VerifiedOutcomeClaimRecovery> {
        self.outcome_claim
    }

    /// Consumes the queue row and yields the exact stage-specific recovery
    /// capability. Outcome claim/run rows cannot cross this boundary.
    pub(crate) fn into_non_outcome(self) -> Option<NonOutcomeRecoveryStageRequest> {
        self.non_outcome
    }
}

/// Opaque exact claim recovery capability.
///
/// It retains the strict typed payload bytes from the durable envelope. The
/// recovery owner can only replay that exact claim/run/request lineage.
#[must_use = "an outcome claim recovery capability must be replayed under its subject lock"]
#[allow(
    dead_code,
    reason = "BR-183 preserves this fail-closed recovery capability while selection-v2 is disabled"
)]
pub struct VerifiedOutcomeClaimRecovery {
    stage_input: OutcomeClaimStageInputPreimage,
}

#[allow(
    dead_code,
    reason = "BR-183 preserves this fail-closed recovery capability while selection-v2 is disabled"
)]
impl VerifiedOutcomeClaimRecovery {
    fn try_from_row(row: &RecoveryRow) -> SelectionV2ReadModelResult<Self> {
        if row.payload_schema != OUTCOME_CLAIM_STAGE_PAYLOAD_SCHEMA
            || sha256_bytes(row.payload_json.as_bytes()) != row.payload_json_hash
        {
            return Err(integrity(
                "outcome_claim_recovery_payload_binding_invalid",
                "claim recovery envelope schema/hash does not bind its exact payload",
            ));
        }
        let stage_input: OutcomeClaimStageInputPreimage = serde_json::from_str(&row.payload_json)
            .map_err(|error| {
            integrity(
                "outcome_claim_recovery_payload_invalid",
                format!("strict claim recovery payload parse failed: {error}"),
            )
        })?;
        stage_input.validate().map_err(|error| {
            integrity(
                "outcome_claim_recovery_stage_invalid",
                format!("claim recovery stage validation failed: {error}"),
            )
        })?;
        if canonical_json(&stage_input).map_err(|error| {
            integrity(
                "outcome_claim_recovery_canonicalize_failed",
                error.to_string(),
            )
        })? != row.payload_json
            || stage_input.stage_run_id != row.subject_id
            || stage_input.logical_subject_key != row.logical_subject_key
        {
            return Err(integrity(
                "outcome_claim_recovery_identity_mismatch",
                "claim recovery payload is noncanonical or differs from envelope identity",
            ));
        }
        Ok(Self { stage_input })
    }

    pub(crate) fn claim_lock_key(&self) -> &str {
        &self.stage_input.claim_lock_key
    }

    pub(crate) fn into_prepared(
        self,
    ) -> Result<crate::selection::outcome_v2::PreparedOutcomeClaimStage, SelectionV2ReadModelError>
    {
        crate::selection::outcome_v2::PreparedOutcomeClaimStage::validated(self.stage_input)
            .map_err(|error| {
                integrity(
                    "outcome_claim_recovery_capability_invalid",
                    error.to_string(),
                )
            })
    }
}

/// Exact recovery algebra for one logical outcome claim.
///
/// The variants deliberately separate the only state that may call the
/// provider (`ClaimActive`) from `OutcomeRecovery`, which can only replay the
/// exact durable outcome payload.
#[must_use = "outcome settlement recovery must be drained before new due work"]
pub(crate) enum VerifiedOutcomeSettlementRecovery {
    ClaimPartial {
        due: VerifiedOutcomeDue,
        claim: crate::selection::outcome_v2::PreparedOutcomeClaimStage,
    },
    ClaimActive {
        due: VerifiedOutcomeDue,
        claim: crate::selection::outcome_v2::PreparedOutcomeClaimStage,
        claim_receipt_content_hash: String,
    },
    OutcomeRecovery {
        logical_subject_key: String,
        verified_due_snapshot_hash: String,
        claim_id: String,
        planned_outcome_run_id: String,
        outcome: crate::selection::outcome_v2::PreparedOutcomeStage,
    },
}

impl VerifiedOutcomeSettlementRecovery {
    fn try_from_material(
        class: OutcomeClaimLifecycleClass,
        material: OutcomeClaimRecoveryMaterial,
    ) -> SelectionV2ReadModelResult<Self> {
        let logical_subject_key = material.claim_stage.logical_subject_key.clone();
        let verified_due_snapshot_hash = material
            .claim_stage
            .due_binding
            .verified_due_snapshot_hash
            .clone();
        let claim_id = material.claim_stage.stage_run_id.clone();
        let planned_outcome_run_id = material.claim_stage.planned_outcome_run_id.clone();
        match class {
            OutcomeClaimLifecycleClass::ClaimPartial => {
                if material.claim_receipt_content_hash.is_some() || material.outcome_stage.is_some()
                {
                    return Err(integrity(
                        "outcome_claim_partial_material_mixed",
                        "ClaimPartial must not carry a claim receipt or outcome payload",
                    ));
                }
                Ok(Self::ClaimPartial {
                    due: VerifiedOutcomeDue::from_claim_stage(&material.claim_stage)?,
                    claim: crate::selection::outcome_v2::PreparedOutcomeClaimStage::validated(
                        material.claim_stage,
                    )
                    .map_err(|error| {
                        integrity(
                            "outcome_claim_partial_capability_invalid",
                            error.to_string(),
                        )
                    })?,
                })
            }
            OutcomeClaimLifecycleClass::ClaimActive => {
                if material.outcome_stage.is_some() {
                    return Err(integrity(
                        "outcome_claim_active_material_mixed",
                        "ClaimActive must not carry an outcome payload",
                    ));
                }
                let claim_receipt_content_hash =
                    material.claim_receipt_content_hash.ok_or_else(|| {
                        integrity(
                            "outcome_claim_active_receipt_missing",
                            "ClaimActive requires the exact claim receipt hash",
                        )
                    })?;
                Ok(Self::ClaimActive {
                    due: VerifiedOutcomeDue::from_claim_stage(&material.claim_stage)?,
                    claim: crate::selection::outcome_v2::PreparedOutcomeClaimStage::validated(
                        material.claim_stage,
                    )
                    .map_err(|error| {
                        integrity("outcome_claim_active_capability_invalid", error.to_string())
                    })?,
                    claim_receipt_content_hash,
                })
            }
            OutcomeClaimLifecycleClass::OutcomeRecovery => {
                let outcome_stage = material.outcome_stage.ok_or_else(|| {
                    integrity(
                        "outcome_recovery_payload_missing",
                        "OutcomeRecovery requires the exact persisted outcome payload",
                    )
                })?;
                if material.claim_receipt_content_hash.is_none() {
                    return Err(integrity(
                        "outcome_recovery_claim_receipt_missing",
                        "OutcomeRecovery requires the exact receipted claim lineage",
                    ));
                }
                Ok(Self::OutcomeRecovery {
                    logical_subject_key,
                    verified_due_snapshot_hash,
                    claim_id,
                    planned_outcome_run_id,
                    outcome: crate::selection::outcome_v2::PreparedOutcomeStage::validated(
                        outcome_stage,
                    )
                    .map_err(|error| {
                        integrity("outcome_recovery_capability_invalid", error.to_string())
                    })?,
                })
            }
            OutcomeClaimLifecycleClass::Closed => Err(integrity(
                "closed_outcome_claim_entered_recovery",
                "Closed claim lifecycle must not enter recovery",
            )),
        }
    }

    pub(crate) fn logical_subject_key(&self) -> &str {
        match self {
            Self::ClaimPartial { due, .. } | Self::ClaimActive { due, .. } => {
                due.logical_subject_key()
            }
            Self::OutcomeRecovery {
                logical_subject_key,
                ..
            } => logical_subject_key,
        }
    }

    pub(crate) fn stable_identity(&self) -> (&str, &str, OutcomeClaimLifecycleClass) {
        match self {
            Self::ClaimPartial { claim, .. } => (
                claim.claim_id(),
                claim.planned_outcome_run_id(),
                OutcomeClaimLifecycleClass::ClaimPartial,
            ),
            Self::ClaimActive { claim, .. } => (
                claim.claim_id(),
                claim.planned_outcome_run_id(),
                OutcomeClaimLifecycleClass::ClaimActive,
            ),
            Self::OutcomeRecovery {
                claim_id,
                planned_outcome_run_id,
                ..
            } => (
                claim_id,
                planned_outcome_run_id,
                OutcomeClaimLifecycleClass::OutcomeRecovery,
            ),
        }
    }

    pub(crate) fn verified_due_snapshot_hash(&self) -> &str {
        match self {
            Self::ClaimPartial { due, .. } | Self::ClaimActive { due, .. } => {
                due.verified_due_snapshot_hash()
            }
            Self::OutcomeRecovery {
                verified_due_snapshot_hash,
                ..
            } => verified_due_snapshot_hash,
        }
    }
}

/// Opaque due capability consumed by the Magic-TDX-only outcome Gateway.
///
/// The baseline and request binding can only originate from a receipt/audit
/// verified, pinned read snapshot.
#[must_use = "a due capability must be consumed or explicitly deferred"]
pub struct VerifiedOutcomeDue {
    sample_key: String,
    canonical_stock_code: String,
    canonical_market: String,
    sample_key_preimage: SampleKeyPreimage,
    config_activation_run_id: String,
    config_hash: String,
    phase: OutcomePhase,
    stored_due_date: NaiveDate,
    calendar_version: String,
    calendar_hash: String,
    trading_date_vector: OutcomeTradingDateVectorPreimage,
    trading_date_vector_hash: String,
    applicable_trading_dates: Vec<NaiveDate>,
    t0: Option<VerifiedT0Baseline>,
    logical_subject_key: String,
    verified_due_snapshot_hash: String,
    claim_due_binding: OutcomeClaimDueBindingPreimage,
    claim_due_binding_hash: String,
    provider_request_evidence: RequestEvidencePreimage,
    provider_request_hash: String,
    provider_transport_request: Option<OutcomeProviderRequestPreimage>,
    provider_transport_request_hash: Option<String>,
}

#[derive(Debug, Clone)]
struct VerifiedT0Baseline {
    close: String,
    volume: String,
}

impl VerifiedOutcomeDue {
    fn from_claim_stage(
        stage: &OutcomeClaimStageInputPreimage,
    ) -> SelectionV2ReadModelResult<Self> {
        stage.validate().map_err(|error| {
            integrity(
                "outcome_claim_due_recovery_stage_invalid",
                error.to_string(),
            )
        })?;
        if sha256_json(&stage.due_binding).map_err(|error| {
            integrity(
                "outcome_claim_due_recovery_binding_hash_failed",
                error.to_string(),
            )
        })? != stage.due_binding_hash
            || sha256_json(&stage.due_binding.verified_due_snapshot).map_err(|error| {
                integrity(
                    "outcome_claim_due_recovery_snapshot_hash_failed",
                    error.to_string(),
                )
            })? != stage.due_binding.verified_due_snapshot_hash
        {
            return Err(integrity(
                "outcome_claim_due_recovery_hash_mismatch",
                "claim does not bind its exact verified due snapshot",
            ));
        }
        let binding = &stage.due_binding;
        let snapshot = &binding.verified_due_snapshot;
        if snapshot.logical_subject_key != stage.logical_subject_key
            || snapshot.sample_key != binding.sample_key
            || snapshot.sample_key_preimage != binding.sample_key_preimage
            || snapshot.config_activation_run_id != stage.config_activation_run_id
            || snapshot.config_hash != stage.config_hash
            || snapshot.outcome_phase != binding.outcome_phase
            || snapshot.stored_due_date != binding.stored_due_date
            || snapshot.provider_request_hash != stage.provider_request_hash
        {
            return Err(integrity(
                "outcome_claim_due_recovery_identity_mismatch",
                "claim, due binding and verified due snapshot identities differ",
            ));
        }
        let applicable_trading_dates = snapshot
            .applicable_trading_dates
            .iter()
            .map(|value| parse_date(value, "applicable_trading_date"))
            .collect::<SelectionV2ReadModelResult<Vec<_>>>()?;
        if applicable_trading_dates.is_empty() {
            return Err(integrity(
                "outcome_claim_due_recovery_window_empty",
                "verified due snapshot must retain a non-empty provider window",
            ));
        }
        let t0 = match (snapshot.t0_close.as_ref(), snapshot.t0_volume.as_ref()) {
            (None, None) => None,
            (Some(close), Some(volume)) => Some(VerifiedT0Baseline {
                close: close.clone(),
                volume: volume.clone(),
            }),
            _ => {
                return Err(integrity(
                    "outcome_claim_due_recovery_t0_mixed",
                    "T0 close and volume must both be present or both be absent",
                ));
            }
        };
        Ok(Self {
            sample_key: snapshot.sample_key.clone(),
            canonical_stock_code: snapshot.canonical_stock_code.clone(),
            canonical_market: snapshot.canonical_market.clone(),
            sample_key_preimage: snapshot.sample_key_preimage.clone(),
            config_activation_run_id: snapshot.config_activation_run_id.clone(),
            config_hash: snapshot.config_hash.clone(),
            phase: snapshot.outcome_phase,
            stored_due_date: parse_date(&snapshot.stored_due_date, "stored_due_date")?,
            calendar_version: snapshot.calendar_version.clone(),
            calendar_hash: snapshot.calendar_hash.clone(),
            trading_date_vector: snapshot.trading_date_vector.clone(),
            trading_date_vector_hash: snapshot.trading_date_vector_hash.clone(),
            applicable_trading_dates,
            t0,
            logical_subject_key: snapshot.logical_subject_key.clone(),
            verified_due_snapshot_hash: stage.due_binding.verified_due_snapshot_hash.clone(),
            claim_due_binding: stage.due_binding.clone(),
            claim_due_binding_hash: stage.due_binding_hash.clone(),
            provider_request_evidence: stage.provider_request_evidence.clone(),
            provider_request_hash: stage.provider_request_hash.clone(),
            provider_transport_request: Some(stage.provider_transport_request.clone()),
            provider_transport_request_hash: Some(stage.provider_transport_request_hash.clone()),
        })
    }

    pub(crate) fn sample_key(&self) -> &str {
        &self.sample_key
    }

    pub(crate) fn canonical_stock_code(&self) -> &str {
        &self.canonical_stock_code
    }

    pub(crate) fn canonical_market(&self) -> &str {
        &self.canonical_market
    }

    pub(crate) fn sample_key_preimage(&self) -> &SampleKeyPreimage {
        &self.sample_key_preimage
    }

    pub(crate) fn config_activation_run_id(&self) -> &str {
        &self.config_activation_run_id
    }

    pub(crate) fn phase(&self) -> OutcomePhase {
        self.phase
    }

    pub(crate) fn stored_due_date(&self) -> NaiveDate {
        self.stored_due_date
    }

    pub(crate) fn window_start(&self) -> NaiveDate {
        self.applicable_trading_dates[0]
    }

    pub(crate) fn window_end(&self) -> NaiveDate {
        *self
            .applicable_trading_dates
            .last()
            .expect("verified prefix is non-empty")
    }

    pub(crate) fn expected_bar_count(&self) -> u16 {
        self.applicable_trading_dates.len() as u16
    }

    pub(crate) fn calendar_version(&self) -> &str {
        &self.calendar_version
    }

    pub(crate) fn calendar_hash(&self) -> &str {
        &self.calendar_hash
    }

    pub(crate) fn trading_date_vector(&self) -> &OutcomeTradingDateVectorPreimage {
        &self.trading_date_vector
    }

    pub(crate) fn trading_date_vector_hash(&self) -> &str {
        &self.trading_date_vector_hash
    }

    pub(crate) fn applicable_trading_dates(&self) -> &[NaiveDate] {
        &self.applicable_trading_dates
    }

    pub(crate) fn config_hash(&self) -> &str {
        &self.config_hash
    }

    pub(crate) fn request_binding_hash(&self) -> &str {
        &self.verified_due_snapshot_hash
    }

    pub(crate) fn provider_request_hash(&self) -> &str {
        &self.provider_request_hash
    }

    pub(crate) fn provider_request_evidence(&self) -> &RequestEvidencePreimage {
        &self.provider_request_evidence
    }

    pub(crate) fn bind_provider_transport_request(
        mut self,
        request: OutcomeProviderRequestPreimage,
        request_hash: String,
    ) -> SelectionV2ReadModelResult<Self> {
        let parameters: OutcomeMarketRequestParametersPreimage = serde_json::from_str(
            &self.provider_request_evidence.parameters_json,
        )
        .map_err(|error| {
            integrity(
                "outcome_claim_transport_parameters_decode_failed",
                error.to_string(),
            )
        })?;
        request
            .validate(&self.provider_request_evidence, &parameters)
            .map_err(|error| {
                integrity("outcome_claim_transport_request_invalid", error.to_string())
            })?;
        if sha256_json(&request).map_err(|error| {
            integrity(
                "outcome_claim_transport_request_hash_failed",
                error.to_string(),
            )
        })? != request_hash
            || request.semantic_request_hash != self.provider_request_hash
            || request.verified_due_binding_hash != self.verified_due_snapshot_hash
        {
            return Err(integrity(
                "outcome_claim_transport_request_binding_mismatch",
                "provider transport request does not bind this exact verified due",
            ));
        }
        self.provider_transport_request = Some(request);
        self.provider_transport_request_hash = Some(request_hash);
        Ok(self)
    }

    pub(crate) fn provider_transport_request(
        &self,
    ) -> SelectionV2ReadModelResult<(&OutcomeProviderRequestPreimage, &str)> {
        match (
            self.provider_transport_request.as_ref(),
            self.provider_transport_request_hash.as_deref(),
        ) {
            (Some(request), Some(hash)) => Ok((request, hash)),
            _ => Err(integrity(
                "outcome_claim_transport_request_unbound",
                "provider I/O requires an exact transport request persisted by the claim owner",
            )),
        }
    }

    pub(crate) fn logical_subject_key(&self) -> &str {
        &self.logical_subject_key
    }

    pub(crate) fn verified_due_snapshot_hash(&self) -> &str {
        &self.verified_due_snapshot_hash
    }

    pub(crate) fn claim_due_binding_hash(&self) -> &str {
        &self.claim_due_binding_hash
    }

    pub(crate) fn prepare_outcome_claim(
        &self,
        claim_id: String,
        planned_outcome_run_id: String,
    ) -> SelectionV2ReadModelResult<crate::selection::outcome_v2::PreparedOutcomeClaimStage> {
        let (provider_transport_request, provider_transport_request_hash) =
            self.provider_transport_request()?;
        let stage = OutcomeClaimStageInputPreimage {
            domain: crate::selection::schema_v2::DOMAIN_OUTCOME_CLAIM_STAGE.into(),
            stage_run_id: claim_id,
            logical_subject_key: self.logical_subject_key.clone(),
            config_activation_run_id: self.config_activation_run_id.clone(),
            config_hash: self.config_hash.clone(),
            planned_outcome_run_id,
            due_binding: self.claim_due_binding.clone(),
            due_binding_hash: sha256_json(&self.claim_due_binding).map_err(|error| {
                integrity("outcome_claim_due_binding_hash_failed", error.to_string())
            })?,
            provider_request_evidence: self.provider_request_evidence.clone(),
            provider_request_hash: self.provider_request_hash.clone(),
            provider_transport_request: provider_transport_request.clone(),
            provider_transport_request_hash: provider_transport_request_hash.to_owned(),
            claim_lock_key: self.logical_subject_key.clone(),
            planned_run_status: RunStatus::Claimed,
        };
        crate::selection::outcome_v2::PreparedOutcomeClaimStage::validated(stage)
            .map_err(|error| integrity("outcome_claim_stage_builder_invalid", error.to_string()))
    }

    pub(crate) fn t0_close(&self) -> Option<&str> {
        self.t0.as_ref().map(|phase| phase.close.as_str())
    }

    pub(crate) fn t0_volume(&self) -> Option<&str> {
        self.t0.as_ref().map(|phase| phase.volume.as_str())
    }
}

#[derive(Debug, Clone)]
struct SettledPhase {
    outcome_run_id: String,
    close: String,
    volume: String,
    outcome_content_hash: String,
    receipt_content_hash: String,
}

struct OutcomePhaseOrder;

impl OutcomePhaseOrder {
    const ALL: [OutcomePhase; 4] = [
        OutcomePhase::T0Close,
        OutcomePhase::D1Settled,
        OutcomePhase::D3Settled,
        OutcomePhase::D5Settled,
    ];

    const fn ordinal(phase: OutcomePhase) -> u8 {
        match phase {
            OutcomePhase::T0Close => 0,
            OutcomePhase::D1Settled => 1,
            OutcomePhase::D3Settled => 2,
            OutcomePhase::D5Settled => 3,
        }
    }
}

#[derive(Serialize)]
struct ReadModelBindingPreimage<'a> {
    domain: &'static str,
    database_binding_hash: &'a str,
    receipt_high_water: i64,
    receipt_subject_ids_sorted: Vec<String>,
    audit_record_count: usize,
    audit_tail_hash: Option<&'a str>,
}

#[derive(Debug)]
struct VerifiedReceipt {
    subject_kind: SubjectKind,
    logical_subject_key: String,
    run_status: RunStatus,
    outcome_phase: Option<OutcomePhase>,
    committed_at_rfc3339_nanos_utc: String,
    content_hash: String,
    run_manifest_content_hash: String,
    committed_audit_hash: String,
}

#[derive(QueryableByName)]
struct HighWaterRow {
    #[diesel(sql_type = BigInt)]
    value: i64,
}

#[derive(QueryableByName)]
struct IntegerValueRow {
    #[diesel(sql_type = Integer)]
    value: i32,
}

#[derive(QueryableByName)]
struct TextValueRow {
    #[diesel(sql_type = Text)]
    value: String,
}

#[derive(Debug, Serialize, QueryableByName)]
struct SchemaObjectRow {
    #[diesel(sql_type = Text)]
    object_type: String,
    #[diesel(sql_type = Text)]
    name: String,
    #[diesel(sql_type = Text)]
    tbl_name: String,
    #[diesel(sql_type = Nullable<Text>)]
    sql: Option<String>,
}

#[derive(QueryableByName)]
struct ReceiptRow {
    #[diesel(sql_type = BigInt)]
    rowid: i64,
    #[diesel(sql_type = Text)]
    subject_kind: String,
    #[diesel(sql_type = Text)]
    subject_id: String,
    #[diesel(sql_type = Text)]
    logical_subject_key: String,
    #[diesel(sql_type = Text)]
    run_status: String,
    #[diesel(sql_type = Nullable<Text>)]
    outcome_phase: Option<String>,
    #[diesel(sql_type = Text)]
    committed_at: String,
    #[diesel(sql_type = Text)]
    content_hash: String,
    #[diesel(sql_type = Text)]
    run_manifest_content_hash: String,
    #[diesel(sql_type = Text)]
    committed_audit_hash: String,
}

#[derive(QueryableByName)]
struct RecoveryRow {
    #[diesel(sql_type = Text)]
    subject_kind: String,
    #[diesel(sql_type = Text)]
    subject_id: String,
    #[diesel(sql_type = Text)]
    logical_subject_key: String,
    #[diesel(sql_type = Text)]
    order_at: String,
    #[diesel(sql_type = Text)]
    payload_schema: String,
    #[diesel(sql_type = Text)]
    payload_json: String,
    #[diesel(sql_type = Text)]
    payload_json_hash: String,
    #[diesel(sql_type = Text)]
    in_memory_payload_hash: String,
    #[diesel(sql_type = Text)]
    config_activation_run_id: String,
    #[diesel(sql_type = Text)]
    config_hash: String,
    #[diesel(sql_type = Text)]
    enveloped_at: String,
}

#[derive(QueryableByName)]
struct ReceiptedSampleRow {
    #[diesel(sql_type = Text)]
    sample_key: String,
    #[diesel(sql_type = Text)]
    canonical_stock_code: String,
    #[diesel(sql_type = Text)]
    canonical_market: String,
    #[diesel(sql_type = Text)]
    event_id: String,
    #[diesel(sql_type = Text)]
    chain_id: String,
    #[diesel(sql_type = Text)]
    relation_schema_version: String,
    #[diesel(sql_type = Text)]
    feature_version: String,
    #[diesel(sql_type = Text)]
    config_activation_run_id: String,
    #[diesel(sql_type = Text)]
    config_hash: String,
    #[diesel(sql_type = Text)]
    evaluation_market_date: String,
    #[diesel(sql_type = Text)]
    t0_due_date: String,
    #[diesel(sql_type = Text)]
    d1_due_date: String,
    #[diesel(sql_type = Text)]
    d2_due_date: String,
    #[diesel(sql_type = Text)]
    d3_due_date: String,
    #[diesel(sql_type = Text)]
    d4_due_date: String,
    #[diesel(sql_type = Text)]
    d5_due_date: String,
    #[diesel(sql_type = Text)]
    calendar_version: String,
    #[diesel(sql_type = Text)]
    calendar_hash: String,
    #[diesel(sql_type = Text)]
    trading_date_vector_json: String,
    #[diesel(sql_type = Text)]
    trading_date_vector_hash: String,
    #[diesel(sql_type = Text)]
    activation_receipt_subject_id: String,
    #[diesel(sql_type = Text)]
    activation_receipt_content_hash: String,
    #[diesel(sql_type = Text)]
    ingress_receipt_subject_id: String,
    #[diesel(sql_type = Text)]
    ingress_receipt_content_hash: String,
    #[diesel(sql_type = Text)]
    generation_receipt_subject_id: String,
    #[diesel(sql_type = Text)]
    generation_receipt_content_hash: String,
}

#[derive(QueryableByName)]
struct SettledPhaseRow {
    #[diesel(sql_type = Text)]
    phase: String,
    #[diesel(sql_type = Text)]
    outcome_run_id: String,
    #[diesel(sql_type = Text)]
    due_trading_date: String,
    #[diesel(sql_type = Text)]
    close: String,
    #[diesel(sql_type = Text)]
    volume: String,
    #[diesel(sql_type = Text)]
    outcome_content_hash: String,
    #[diesel(sql_type = Text)]
    receipt_content_hash: String,
}

fn load_receipt_high_water(conn: &mut SqliteConnection) -> SelectionV2ReadModelResult<i64> {
    Ok(diesel::sql_query(
        "SELECT COALESCE(MAX(rowid), 0) AS value
         FROM selection_v2_commit_receipts",
    )
    .get_result::<HighWaterRow>(conn)?
    .value)
}

fn load_verified_receipts(
    conn: &mut SqliteConnection,
    high_water: i64,
) -> SelectionV2ReadModelResult<BTreeMap<String, VerifiedReceipt>> {
    let rows = diesel::sql_query(
        "SELECT r.rowid, r.subject_kind, r.subject_id,
                r.logical_subject_key, m.run_status, m.outcome_phase,
                r.committed_at, r.content_hash,
                r.run_manifest_content_hash, r.committed_audit_hash
         FROM selection_v2_commit_receipts r
         INNER JOIN selection_v2_run_stages m
                 ON m.subject_kind=r.subject_kind
                AND m.subject_id=r.subject_id
                AND m.manifest_content_hash=r.run_manifest_content_hash
         WHERE r.rowid <= ?
         ORDER BY r.rowid ASC",
    )
    .bind::<BigInt, _>(high_water)
    .load::<ReceiptRow>(conn)?;
    let mut receipts = BTreeMap::new();
    let mut previous_rowid = 0;
    for row in rows {
        if row.rowid <= previous_rowid {
            return Err(integrity(
                "receipt_rowid_order_invalid",
                "receipt rowids are not strictly ascending",
            ));
        }
        previous_rowid = row.rowid;
        let receipt = VerifiedReceipt {
            subject_kind: parse_subject_kind(&row.subject_kind)?,
            logical_subject_key: row.logical_subject_key,
            run_status: parse_run_status(&row.run_status)?,
            outcome_phase: row
                .outcome_phase
                .as_deref()
                .map(parse_outcome_phase)
                .transpose()?,
            committed_at_rfc3339_nanos_utc: row.committed_at,
            content_hash: row.content_hash,
            run_manifest_content_hash: row.run_manifest_content_hash,
            committed_audit_hash: row.committed_audit_hash,
        };
        if receipts.insert(row.subject_id.clone(), receipt).is_some() {
            return Err(integrity(
                "receipt_subject_duplicate",
                format!("receipt subject {} is duplicated", row.subject_id),
            ));
        }
    }
    Ok(receipts)
}

fn require_exact_receipt<'a>(
    receipts: &'a BTreeMap<String, VerifiedReceipt>,
    subject_id: &str,
    expected_kind: SubjectKind,
    expected_content_hash: &str,
) -> SelectionV2ReadModelResult<&'a VerifiedReceipt> {
    let receipt = receipts.get(subject_id).ok_or_else(|| {
        integrity(
            "verified_receipt_key_missing",
            format!("receipt {subject_id} is outside the verified snapshot"),
        )
    })?;
    if receipt.subject_kind != expected_kind {
        return Err(integrity(
            "verified_receipt_kind_mismatch",
            format!(
                "receipt {subject_id} expected {}, got {}",
                expected_kind.as_str(),
                receipt.subject_kind.as_str()
            ),
        ));
    }
    if receipt.content_hash != expected_content_hash {
        return Err(integrity(
            "verified_receipt_content_hash_mismatch",
            format!("receipt {subject_id} projection differs from the verified receipt set"),
        ));
    }
    Ok(receipt)
}

fn load_receipted_samples(
    conn: &mut SqliteConnection,
) -> SelectionV2ReadModelResult<Vec<ReceiptedSampleRow>> {
    Ok(diesel::sql_query(
        "SELECT s.sample_key, s.canonical_stock_code, s.canonical_market,
                s.event_id, s.chain_id, s.relation_schema_version, s.feature_version,
                s.config_activation_run_id, s.config_hash,
                s.evaluation_market_date, s.t0_due_date, s.d1_due_date, s.d2_due_date,
                s.d3_due_date, s.d4_due_date, s.d5_due_date,
                s.calendar_version, s.calendar_hash,
                s.trading_date_vector_json, s.trading_date_vector_hash,
                ar.subject_id AS activation_receipt_subject_id,
                ar.content_hash AS activation_receipt_content_hash,
                ir.subject_id AS ingress_receipt_subject_id,
                ir.content_hash AS ingress_receipt_content_hash,
                gr.subject_id AS generation_receipt_subject_id,
                gr.content_hash AS generation_receipt_content_hash
         FROM selection_samples s
         INNER JOIN selection_source_facts_v2 f
                 ON f.source_fact_key=s.source_fact_key
                AND f.content_hash=s.source_fact_content_hash
         INNER JOIN selection_v2_commit_receipts ir
                 ON ir.subject_kind='ingress_run'
                AND ir.subject_id=f.first_ingress_run_id
         INNER JOIN selection_v2_commit_receipts gr
                 ON gr.subject_kind='generation_run'
                AND gr.subject_id=s.generation_run_id
         INNER JOIN selection_v2_commit_receipts ar
                 ON ar.subject_kind='config_activation'
                AND ar.subject_id=s.config_activation_run_id
         ORDER BY s.sample_key ASC",
    )
    .load::<ReceiptedSampleRow>(conn)?)
}

fn load_settled_phases(
    conn: &mut SqliteConnection,
    receipts: &BTreeMap<String, VerifiedReceipt>,
    sample: &ReceiptedSampleRow,
    schedule: &VerifiedSchedule,
) -> SelectionV2ReadModelResult<HashMap<OutcomePhase, SettledPhase>> {
    let rows = diesel::sql_query(
        "SELECT o.phase, o.outcome_run_id, o.due_trading_date, o.close, o.volume,
                o.content_hash AS outcome_content_hash,
                r.content_hash AS receipt_content_hash
         FROM selection_sample_outcomes o
         INNER JOIN selection_v2_run_stages m
                 ON m.subject_kind='outcome_run'
                AND m.subject_id=o.outcome_run_id
                AND m.run_status='settled'
         INNER JOIN selection_v2_commit_receipts r
                 ON r.subject_kind='outcome_run'
                AND r.subject_id=o.outcome_run_id
                AND r.run_manifest_content_hash=m.manifest_content_hash
         WHERE o.sample_key=?
         ORDER BY CASE o.phase
             WHEN 't0_close' THEN 0
             WHEN 'd1_settled' THEN 1
             WHEN 'd3_settled' THEN 2
             WHEN 'd5_settled' THEN 3
             ELSE 99 END ASC",
    )
    .bind::<Text, _>(&sample.sample_key)
    .load::<SettledPhaseRow>(conn)?;
    let mut phases = HashMap::new();
    for row in rows {
        require_exact_receipt(
            receipts,
            &row.outcome_run_id,
            SubjectKind::OutcomeRun,
            &row.receipt_content_hash,
        )?;
        let phase = parse_outcome_phase(&row.phase)?;
        let due_trading_date = parse_date(&row.due_trading_date, "outcome.due_trading_date")?;
        if due_trading_date != schedule.due_date(phase) {
            return Err(integrity(
                "settled_phase_due_date_mismatch",
                format!(
                    "sample {} phase {} has {}, schedule requires {}",
                    sample.sample_key,
                    phase.as_str(),
                    due_trading_date,
                    schedule.due_date(phase)
                ),
            ));
        }
        let settled = SettledPhase {
            outcome_run_id: row.outcome_run_id,
            close: row.close,
            volume: row.volume,
            outcome_content_hash: row.outcome_content_hash,
            receipt_content_hash: row.receipt_content_hash,
        };
        if phases.insert(phase, settled).is_some() {
            return Err(integrity(
                "settled_phase_duplicate",
                format!(
                    "sample {} repeats phase {}",
                    sample.sample_key,
                    phase.as_str()
                ),
            ));
        }
    }
    Ok(phases)
}

fn preceding_settled(
    phase: OutcomePhase,
    settled: &HashMap<OutcomePhase, SettledPhase>,
) -> SelectionV2ReadModelResult<Vec<SettledPhase>> {
    let mut preceding = Vec::new();
    for candidate in OutcomePhaseOrder::ALL {
        if candidate == phase {
            break;
        }
        preceding.push(
            settled
                .get(&candidate)
                .ok_or_else(|| {
                    integrity(
                        "preceding_outcome_receipt_missing",
                        format!(
                            "phase {} requires preceding {}",
                            phase.as_str(),
                            candidate.as_str()
                        ),
                    )
                })?
                .clone(),
        );
    }
    Ok(preceding)
}

fn verified_sample_key_preimage(
    sample: &ReceiptedSampleRow,
) -> SelectionV2ReadModelResult<SampleKeyPreimage> {
    let preimage = SampleKeyPreimage {
        domain: DOMAIN_SAMPLE_KEY.into(),
        event_id: sample.event_id.clone(),
        chain_id: sample.chain_id.clone(),
        stock_code: sample.canonical_stock_code.clone(),
        relation_schema_version: sample.relation_schema_version.clone(),
        feature_version: sample.feature_version.clone(),
        evaluation_market_date: sample.evaluation_market_date.clone(),
    };
    let recomputed = sha256_json(&preimage).map_err(|error| {
        integrity(
            "sample_key_recompute_failed",
            format!(
                "cannot recompute sample key for {}: {error}",
                sample.sample_key
            ),
        )
    })?;
    if recomputed != sample.sample_key {
        return Err(integrity(
            "sample_key_preimage_mismatch",
            format!(
                "receipted sample key {} does not match canonical preimage {}",
                sample.sample_key, recomputed
            ),
        ));
    }
    Ok(preimage)
}

fn outcome_logical_subject_key(
    sample: &ReceiptedSampleRow,
    phase: OutcomePhase,
    due: NaiveDate,
) -> SelectionV2ReadModelResult<String> {
    run_logical_subject_key(&RunLogicalSubjectPreimage {
        domain: DOMAIN_RUN_LOGICAL_SUBJECT.into(),
        subject_kind: SubjectKind::OutcomeRun,
        source_fact_key: None,
        config_hash: Some(sample.config_hash.clone()),
        sample_key: Some(sample.sample_key.clone()),
        outcome_phase: Some(phase),
        stored_due_date: Some(due.format("%Y-%m-%d").to_string()),
        ingress_source_batch_hash: None,
    })
    .map_err(|error| {
        integrity(
            "outcome_logical_subject_key_failed",
            format!("cannot bind due outcome subject: {error}"),
        )
    })
}

fn has_unreceipted_logical_subject(
    conn: &mut SqliteConnection,
    logical_subject_key: &str,
) -> SelectionV2ReadModelResult<bool> {
    Ok(diesel::sql_query(
        "SELECT COUNT(*) AS value
         FROM selection_v2_recovery_envelopes e
         WHERE e.subject_kind IN ('outcome_claim','outcome_run')
           AND e.logical_subject_key=?
           AND NOT EXISTS (
               SELECT 1 FROM selection_v2_commit_receipts r
               WHERE r.subject_kind=e.subject_kind
                 AND r.subject_id=e.stage_run_id
           )",
    )
    .bind::<Text, _>(logical_subject_key)
    .get_result::<HighWaterRow>(conn)?
    .value
        > 0)
}

fn sorted_same_subject_attempts<'a>(
    receipts: &'a BTreeMap<String, VerifiedReceipt>,
    logical_subject_key: &str,
    phase: OutcomePhase,
) -> Vec<(&'a String, &'a VerifiedReceipt)> {
    let mut same_subject_attempts = receipts
        .iter()
        .filter(|(_, receipt)| {
            receipt.subject_kind == SubjectKind::OutcomeRun
                && receipt.logical_subject_key == logical_subject_key
                && receipt.outcome_phase == Some(phase)
        })
        .collect::<Vec<_>>();
    same_subject_attempts.sort_by(|(left_id, left), (right_id, right)| {
        (
            left.committed_at_rfc3339_nanos_utc.as_str(),
            left_id.as_str(),
            left.content_hash.as_str(),
        )
            .cmp(&(
                right.committed_at_rfc3339_nanos_utc.as_str(),
                right_id.as_str(),
                right.content_hash.as_str(),
            ))
    });
    same_subject_attempts
}

struct OutcomeClaimMaterial {
    verified_due_snapshot_hash: String,
    due_binding: OutcomeClaimDueBindingPreimage,
    due_binding_hash: String,
    provider_request_evidence: RequestEvidencePreimage,
    provider_request_hash: String,
}

#[allow(clippy::too_many_arguments)]
fn build_outcome_claim_material(
    sample: &ReceiptedSampleRow,
    sample_key_preimage: &SampleKeyPreimage,
    logical_subject_key: &str,
    phase: OutcomePhase,
    stored_due_date: NaiveDate,
    schedule: &VerifiedSchedule,
    window: &OutcomeWindow,
    preceding: &[SettledPhase],
    t0: Option<&SettledPhase>,
    receipts: &BTreeMap<String, VerifiedReceipt>,
    database_binding: &VerifiedOutcomeDueDatabaseBindingPreimage,
    database_binding_hash: &str,
    audit_record_hashes: &[String],
) -> SelectionV2ReadModelResult<OutcomeClaimMaterial> {
    if audit_record_hashes.is_empty() {
        return Err(integrity(
            "outcome_due_audit_prefix_empty",
            "receipted outcome due work requires a non-empty verified audit prefix",
        ));
    }
    let applicable_trading_dates = window
        .applicable_dates
        .iter()
        .map(|date| date.format("%Y-%m-%d").to_string())
        .collect::<Vec<_>>();
    let request_columns = build_request_evidence(
        RequestParametersPreimage::OutcomeMarketEvidence(OutcomeMarketRequestParametersPreimage {
            domain: DOMAIN_OUTCOME_MARKET_REQUEST.into(),
            sample_key: sample.sample_key.clone(),
            canonical_stock_code: sample.canonical_stock_code.clone(),
            canonical_market: sample.canonical_market.clone(),
            phase,
            stored_due_date: stored_due_date.format("%Y-%m-%d").to_string(),
            calendar_version: sample.calendar_version.clone(),
            calendar_hash: sample.calendar_hash.clone(),
            trading_date_vector: schedule.vector.clone(),
            trading_date_vector_hash: schedule.vector_hash.clone(),
            applicable_trading_dates: applicable_trading_dates.clone(),
            window_start: applicable_trading_dates[0].clone(),
            window_end: applicable_trading_dates
                .last()
                .expect("verified prefix is non-empty")
                .clone(),
            interval: DailyIntervalKind::Day,
            adjustment: AdjustmentKind::None,
        }),
        ProviderCapabilityHashPreimage {
            domain: DOMAIN_PROVIDER_CAPABILITY.into(),
            provider: OUTCOME_REQUEST_PROVIDER.into(),
            capability_name: OUTCOME_REQUEST_CAPABILITY.into(),
            contract_version: OUTCOME_REQUEST_CONTRACT.into(),
            upstream_revision: UPSTREAM_REVISION.into(),
        },
    )
    .map_err(|error| {
        integrity(
            "outcome_due_request_evidence_failed",
            format!("cannot build exact outcome market request evidence: {error}"),
        )
    })?;
    let provider_request_evidence = request_columns
        .validate(Some(
            crate::selection::schema_v2::RequestKind::OutcomeMarketEvidence,
        ))
        .map_err(|error| {
            integrity(
                "outcome_due_request_evidence_invalid",
                format!("exact outcome request evidence failed validation: {error}"),
            )
        })?;
    let provider_request_hash = request_columns.request_hash;

    let activation = require_exact_receipt(
        receipts,
        &sample.activation_receipt_subject_id,
        SubjectKind::ConfigActivation,
        &sample.activation_receipt_content_hash,
    )?;
    let ingress = require_exact_receipt(
        receipts,
        &sample.ingress_receipt_subject_id,
        SubjectKind::IngressRun,
        &sample.ingress_receipt_content_hash,
    )?;
    let generation = require_exact_receipt(
        receipts,
        &sample.generation_receipt_subject_id,
        SubjectKind::GenerationRun,
        &sample.generation_receipt_content_hash,
    )?;

    let mut receipt_tuples_sorted = vec![
        verified_receipt_tuple(
            "config_activation",
            None,
            &sample.activation_receipt_subject_id,
            activation,
        ),
        verified_receipt_tuple(
            "source_ingress",
            None,
            &sample.ingress_receipt_subject_id,
            ingress,
        ),
        verified_receipt_tuple(
            "generation",
            None,
            &sample.generation_receipt_subject_id,
            generation,
        ),
    ];
    for item in preceding {
        let receipt = receipts.get(&item.outcome_run_id).ok_or_else(|| {
            integrity(
                "preceding_outcome_receipt_outside_snapshot",
                format!(
                    "preceding outcome {} has no verified receipt",
                    item.outcome_run_id
                ),
            )
        })?;
        receipt_tuples_sorted.push(verified_receipt_tuple(
            "preceding_outcome",
            receipt.outcome_phase,
            &item.outcome_run_id,
            receipt,
        ));
    }
    let same_subject_attempts = sorted_same_subject_attempts(receipts, logical_subject_key, phase);
    for (subject_id, receipt) in &same_subject_attempts {
        receipt_tuples_sorted.push(verified_receipt_tuple(
            "same_subject_attempt",
            Some(phase),
            subject_id,
            receipt,
        ));
    }
    for tuple in &receipt_tuples_sorted {
        let occurrences = audit_record_hashes
            .iter()
            .filter(|hash| *hash == &tuple.committed_audit_record_hash)
            .count();
        if occurrences != 1 {
            return Err(integrity(
                "outcome_due_receipt_audit_membership_invalid",
                format!(
                    "receipt {} committed audit hash occurs {occurrences} times in prefix",
                    tuple.receipt_content_hash
                ),
            ));
        }
    }
    let selection_audit_high_water_record_hash = audit_record_hashes
        .last()
        .expect("non-empty audit prefix checked above")
        .clone();
    let selection_audit_prefix_hash = sha256_json(&VerifiedOutcomeAuditPrefixPreimage {
        domain: DOMAIN_OUTCOME_AUDIT_PREFIX.into(),
        record_hashes_in_file_order: audit_record_hashes.to_vec(),
    })
    .map_err(|error| {
        integrity(
            "outcome_due_audit_prefix_hash_failed",
            format!("cannot hash verified audit prefix: {error}"),
        )
    })?;
    let snapshot = VerifiedOutcomeDueSnapshotPreimage {
        domain: DOMAIN_VERIFIED_OUTCOME_DUE_SNAPSHOT.into(),
        database_binding: database_binding.clone(),
        database_binding_hash: database_binding_hash.into(),
        selection_audit_high_water_record_ordinal: u64::try_from(audit_record_hashes.len() - 1)
            .map_err(|_| {
                integrity(
                    "outcome_due_audit_ordinal_overflow",
                    "audit prefix length cannot fit u64 ordinal",
                )
            })?,
        selection_audit_high_water_record_hash: selection_audit_high_water_record_hash.clone(),
        selection_audit_prefix_hash,
        receipt_tuples_sorted,
        sample_key_preimage: sample_key_preimage.clone(),
        sample_key: sample.sample_key.clone(),
        logical_subject_key: logical_subject_key.into(),
        canonical_stock_code: sample.canonical_stock_code.clone(),
        canonical_market: sample.canonical_market.clone(),
        config_activation_run_id: sample.config_activation_run_id.clone(),
        config_hash: sample.config_hash.clone(),
        outcome_phase: phase,
        stored_due_date: stored_due_date.format("%Y-%m-%d").to_string(),
        calendar_version: sample.calendar_version.clone(),
        calendar_hash: sample.calendar_hash.clone(),
        trading_date_vector: schedule.vector.clone(),
        trading_date_vector_hash: schedule.vector_hash.clone(),
        applicable_trading_dates: applicable_trading_dates.clone(),
        expected_provider_bar_count: u32::try_from(applicable_trading_dates.len()).map_err(
            |_| {
                integrity(
                    "outcome_due_expected_bar_count_overflow",
                    "applicable trading date count exceeds u32",
                )
            },
        )?,
        provider_request_hash: provider_request_hash.clone(),
        t0_outcome_content_hash: t0.map(|item| item.outcome_content_hash.clone()),
        t0_close: t0.map(|item| item.close.clone()),
        t0_volume: t0.map(|item| item.volume.clone()),
    };
    snapshot.validate().map_err(|error| {
        integrity(
            "verified_outcome_due_snapshot_invalid",
            format!("constructed verified due snapshot is invalid: {error}"),
        )
    })?;
    let verified_due_snapshot_hash = sha256_json(&snapshot).map_err(|error| {
        integrity(
            "verified_outcome_due_snapshot_hash_failed",
            error.to_string(),
        )
    })?;
    let previous_same_subject_attempt_receipt_hashes = same_subject_attempts
        .iter()
        .map(|(_, receipt)| receipt.content_hash.clone())
        .collect::<Vec<_>>();
    let outcome_attempt_ordinal = u32::try_from(previous_same_subject_attempt_receipt_hashes.len())
        .ok()
        .and_then(|count| count.checked_add(1))
        .ok_or_else(|| {
            integrity(
                "outcome_claim_attempt_ordinal_overflow",
                "same-subject attempt count cannot produce a u32 ordinal",
            )
        })?;
    let due_binding = OutcomeClaimDueBindingPreimage {
        domain: DOMAIN_OUTCOME_CLAIM_DUE_BINDING.into(),
        verified_due_snapshot: snapshot,
        verified_due_snapshot_hash: verified_due_snapshot_hash.clone(),
        same_subject_high_water_receipt_hash: previous_same_subject_attempt_receipt_hashes
            .last()
            .cloned(),
        outcome_attempt_ordinal,
        previous_same_subject_attempt_receipt_hashes,
        selection_audit_high_water_record_hash,
        sample_key_preimage: sample_key_preimage.clone(),
        sample_key: sample.sample_key.clone(),
        canonical_stock_code: sample.canonical_stock_code.clone(),
        canonical_market: sample.canonical_market.clone(),
        config_activation_run_id: sample.config_activation_run_id.clone(),
        config_hash: sample.config_hash.clone(),
        config_activation_receipt_hash: sample.activation_receipt_content_hash.clone(),
        source_ingress_run_id: sample.ingress_receipt_subject_id.clone(),
        source_ingress_receipt_hash: sample.ingress_receipt_content_hash.clone(),
        generation_run_id: sample.generation_receipt_subject_id.clone(),
        generation_receipt_hash: sample.generation_receipt_content_hash.clone(),
        outcome_phase: phase,
        t0_market_date: schedule.vector.t0.clone(),
        stored_due_date: stored_due_date.format("%Y-%m-%d").to_string(),
        calendar_version: sample.calendar_version.clone(),
        calendar_hash: sample.calendar_hash.clone(),
        trading_date_vector: schedule.vector.clone(),
        trading_date_vector_hash: schedule.vector_hash.clone(),
        applicable_trading_dates,
        expected_provider_bar_count: u32::try_from(window.applicable_dates.len()).map_err(
            |_| {
                integrity(
                    "outcome_claim_expected_bar_count_overflow",
                    "applicable date count exceeds u32",
                )
            },
        )?,
        preceding_outcome_receipt_hashes: preceding
            .iter()
            .map(|item| item.receipt_content_hash.clone())
            .collect(),
        t0_outcome_content_hash: t0.map(|item| item.outcome_content_hash.clone()),
        t0_close: t0.map(|item| item.close.clone()),
        t0_volume: t0.map(|item| item.volume.clone()),
    };
    due_binding.validate().map_err(|error| {
        integrity(
            "outcome_claim_due_binding_invalid",
            format!("constructed claim due binding is invalid: {error}"),
        )
    })?;
    let due_binding_hash = sha256_json(&due_binding)
        .map_err(|error| integrity("outcome_claim_due_binding_hash_failed", error.to_string()))?;
    Ok(OutcomeClaimMaterial {
        verified_due_snapshot_hash,
        due_binding,
        due_binding_hash,
        provider_request_evidence,
        provider_request_hash,
    })
}

fn verified_receipt_tuple(
    receipt_role: &str,
    outcome_phase: Option<OutcomePhase>,
    subject_id: &str,
    receipt: &VerifiedReceipt,
) -> VerifiedOutcomeReceiptTuplePreimage {
    VerifiedOutcomeReceiptTuplePreimage {
        domain: DOMAIN_OUTCOME_DUE_RECEIPT_TUPLE.into(),
        receipt_role: receipt_role.into(),
        outcome_phase,
        subject_kind: receipt.subject_kind,
        subject_id: subject_id.into(),
        logical_subject_key: receipt.logical_subject_key.clone(),
        run_status: receipt.run_status,
        committed_at_rfc3339_nanos_utc: receipt.committed_at_rfc3339_nanos_utc.clone(),
        receipt_content_hash: receipt.content_hash.clone(),
        run_manifest_content_hash: receipt.run_manifest_content_hash.clone(),
        committed_audit_record_hash: receipt.committed_audit_hash.clone(),
    }
}

#[derive(Debug)]
struct VerifiedSchedule {
    vector: OutcomeTradingDateVectorPreimage,
    dates: [NaiveDate; 6],
    vector_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OutcomeWindow {
    applicable_dates: Vec<NaiveDate>,
}

impl VerifiedSchedule {
    fn from_sample(sample: &ReceiptedSampleRow) -> SelectionV2ReadModelResult<Self> {
        let evaluation = parse_date(
            &sample.evaluation_market_date,
            "sample.evaluation_market_date",
        )?;
        if !is_lower_hex_hash(&sample.trading_date_vector_hash)
            || sha256_bytes(sample.trading_date_vector_json.as_bytes())
                != sample.trading_date_vector_hash
        {
            return Err(integrity(
                "sample_trading_date_vector_hash_invalid",
                format!("sample {} has invalid vector hash", sample.sample_key),
            ));
        }
        let vector: OutcomeTradingDateVectorPreimage =
            serde_json::from_str(&sample.trading_date_vector_json).map_err(|error| {
                integrity(
                    "sample_trading_date_vector_invalid",
                    format!("sample {} vector JSON: {error}", sample.sample_key),
                )
            })?;
        if canonical_json(&vector).map_err(|error| {
            integrity(
                "sample_trading_date_vector_canonicalize_failed",
                error.to_string(),
            )
        })? != sample.trading_date_vector_json
        {
            return Err(integrity(
                "sample_trading_date_vector_noncanonical",
                format!("sample {} vector JSON is noncanonical", sample.sample_key),
            ));
        }
        let dates = vector.validate().map_err(|error| {
            integrity(
                error.code,
                format!("sample {} vector: {}", sample.sample_key, error.detail),
            )
        })?;
        if dates[0] != evaluation {
            return Err(integrity(
                "sample_t0_schedule_mismatch",
                format!(
                    "sample {} evaluation date {} differs from T0 {}",
                    sample.sample_key, evaluation, dates[0]
                ),
            ));
        }
        if sample.calendar_version.trim().is_empty() || !is_lower_hex_hash(&sample.calendar_hash) {
            return Err(integrity(
                "sample_calendar_identity_invalid",
                format!(
                    "sample {} has invalid immutable calendar version/hash",
                    sample.sample_key
                ),
            ));
        }
        let projected = [
            sample.t0_due_date.as_str(),
            sample.d1_due_date.as_str(),
            sample.d2_due_date.as_str(),
            sample.d3_due_date.as_str(),
            sample.d4_due_date.as_str(),
            sample.d5_due_date.as_str(),
        ];
        let vector_values = [
            vector.t0.as_str(),
            vector.d1.as_str(),
            vector.d2.as_str(),
            vector.d3.as_str(),
            vector.d4.as_str(),
            vector.d5.as_str(),
        ];
        if projected != vector_values {
            return Err(integrity(
                "sample_schedule_vector_projection_mismatch",
                format!(
                    "sample {} date columns differ from canonical vector",
                    sample.sample_key
                ),
            ));
        }
        Ok(Self {
            vector,
            dates,
            vector_hash: sample.trading_date_vector_hash.clone(),
        })
    }

    fn due_date(&self, phase: OutcomePhase) -> NaiveDate {
        match phase {
            OutcomePhase::T0Close => self.dates[0],
            OutcomePhase::D1Settled => self.dates[1],
            OutcomePhase::D3Settled => self.dates[3],
            OutcomePhase::D5Settled => self.dates[5],
        }
    }

    fn window(&self, phase: OutcomePhase) -> OutcomeWindow {
        let count = match phase {
            OutcomePhase::T0Close => 1,
            OutcomePhase::D1Settled => 2,
            OutcomePhase::D3Settled => 4,
            OutcomePhase::D5Settled => 6,
        };
        OutcomeWindow {
            applicable_dates: self.dates[..count].to_vec(),
        }
    }
}

fn is_lower_hex_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_tick_limit(limit: i64) -> SelectionV2ReadModelResult<()> {
    if (1..=MAX_TICK_LIMIT).contains(&limit) {
        Ok(())
    } else {
        Err(integrity(
            "tick_limit_invalid",
            format!("BR-176/BR-178 tick limit must be in 1..={MAX_TICK_LIMIT}"),
        ))
    }
}

fn parse_date(value: &str, field: &'static str) -> SelectionV2ReadModelResult<NaiveDate> {
    let parsed = NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|error| {
        integrity(
            "canonical_date_invalid",
            format!("{field}={value:?}: {error}"),
        )
    })?;
    if parsed.format("%Y-%m-%d").to_string() != value {
        return Err(integrity(
            "canonical_date_noncanonical",
            format!("{field} must use exact YYYY-MM-DD"),
        ));
    }
    Ok(parsed)
}

fn parse_subject_kind(value: &str) -> SelectionV2ReadModelResult<SubjectKind> {
    match value {
        "config_activation" => Ok(SubjectKind::ConfigActivation),
        "ingress_run" => Ok(SubjectKind::IngressRun),
        "generation_run" => Ok(SubjectKind::GenerationRun),
        "outcome_claim" => Ok(SubjectKind::OutcomeClaim),
        "outcome_run" => Ok(SubjectKind::OutcomeRun),
        _ => Err(integrity(
            "subject_kind_invalid",
            format!("unknown subject kind {value:?}"),
        )),
    }
}

fn parse_run_status(value: &str) -> SelectionV2ReadModelResult<RunStatus> {
    serde_json::from_str(&format!("\"{value}\"")).map_err(|error| {
        integrity(
            "run_status_invalid",
            format!("unsupported run status {value:?}: {error}"),
        )
    })
}

fn parse_outcome_phase(value: &str) -> SelectionV2ReadModelResult<OutcomePhase> {
    match value {
        "t0_close" => Ok(OutcomePhase::T0Close),
        "d1_settled" => Ok(OutcomePhase::D1Settled),
        "d3_settled" => Ok(OutcomePhase::D3Settled),
        "d5_settled" => Ok(OutcomePhase::D5Settled),
        _ => Err(integrity(
            "outcome_phase_invalid",
            format!("unknown outcome phase {value:?}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, TimeZone, Timelike};

    fn date(value: &str) -> NaiveDate {
        NaiveDate::parse_from_str(value, "%Y-%m-%d").expect("valid test date")
    }

    fn receipted_sample_row() -> ReceiptedSampleRow {
        let preimage = SampleKeyPreimage {
            domain: DOMAIN_SAMPLE_KEY.into(),
            event_id: "event-1".into(),
            chain_id: "chain-1".into(),
            stock_code: "600000".into(),
            relation_schema_version: "relation-v1".into(),
            feature_version: "feature-v1".into(),
            evaluation_market_date: "2026-07-24".into(),
        };
        let vector = OutcomeTradingDateVectorPreimage {
            domain: crate::selection::schema_v2::DOMAIN_OUTCOME_TRADING_DATE_VECTOR.into(),
            t0: "2026-07-24".into(),
            d1: "2026-07-27".into(),
            d2: "2026-07-28".into(),
            d3: "2026-07-29".into(),
            d4: "2026-07-30".into(),
            d5: "2026-07-31".into(),
        };
        ReceiptedSampleRow {
            sample_key: sha256_json(&preimage).expect("canonical sample key"),
            canonical_stock_code: preimage.stock_code.clone(),
            canonical_market: "SH".into(),
            event_id: preimage.event_id,
            chain_id: preimage.chain_id,
            relation_schema_version: preimage.relation_schema_version,
            feature_version: preimage.feature_version,
            config_activation_run_id: "019849b1-e800-7000-8000-000000000001".into(),
            config_hash: "b".repeat(64),
            evaluation_market_date: preimage.evaluation_market_date,
            t0_due_date: "2026-07-24".into(),
            d1_due_date: "2026-07-27".into(),
            d2_due_date: "2026-07-28".into(),
            d3_due_date: "2026-07-29".into(),
            d4_due_date: "2026-07-30".into(),
            d5_due_date: "2026-07-31".into(),
            calendar_version: "calendar-v1".into(),
            calendar_hash: "c".repeat(64),
            trading_date_vector_json: canonical_json(&vector).expect("canonical vector"),
            trading_date_vector_hash: sha256_json(&vector).expect("vector hash"),
            activation_receipt_subject_id: "activation-1".into(),
            activation_receipt_content_hash: "d".repeat(64),
            ingress_receipt_subject_id: "ingress-1".into(),
            ingress_receipt_content_hash: "e".repeat(64),
            generation_receipt_subject_id: "generation-1".into(),
            generation_receipt_content_hash: "f".repeat(64),
        }
    }

    #[test]
    fn tick_limit_is_exactly_one_through_two_hundred() {
        assert!(validate_tick_limit(1).is_ok());
        assert!(validate_tick_limit(200).is_ok());
        assert!(validate_tick_limit(0).is_err());
        assert!(validate_tick_limit(201).is_err());
    }

    #[test]
    fn outcome_window_uses_only_immutable_stored_schedule_boundaries() {
        let schedule =
            VerifiedSchedule::from_sample(&receipted_sample_row()).expect("verified schedule");
        assert_eq!(
            schedule.window(OutcomePhase::T0Close),
            OutcomeWindow {
                applicable_dates: vec![schedule.dates[0]],
            }
        );
        assert_eq!(
            schedule.window(OutcomePhase::D1Settled),
            OutcomeWindow {
                applicable_dates: schedule.dates[..2].to_vec(),
            }
        );
        assert_eq!(
            schedule.window(OutcomePhase::D3Settled),
            OutcomeWindow {
                applicable_dates: schedule.dates[..4].to_vec(),
            }
        );
        assert_eq!(
            schedule.window(OutcomePhase::D5Settled),
            OutcomeWindow {
                applicable_dates: schedule.dates.to_vec(),
            }
        );
    }

    #[test]
    fn calendar_hash_must_be_canonical_lower_hex() {
        assert!(is_lower_hex_hash(&"a".repeat(64)));
        assert!(!is_lower_hex_hash(&"A".repeat(64)));
        assert!(!is_lower_hex_hash("abc"));
    }

    #[test]
    fn stored_schedule_rejects_equal_endpoint_count_with_wrong_middle_date() {
        let mut sample = receipted_sample_row();
        let mut vector: OutcomeTradingDateVectorPreimage =
            serde_json::from_str(&sample.trading_date_vector_json).expect("typed vector");
        vector.d2 = "2026-07-26".into();
        sample.trading_date_vector_json = canonical_json(&vector).expect("canonical vector");
        sample.trading_date_vector_hash = sha256_json(&vector).expect("vector hash");
        let error = VerifiedSchedule::from_sample(&sample)
            .expect_err("wrong middle date must fail despite unchanged phase endpoints/counts");
        assert!(matches!(
            error,
            SelectionV2ReadModelError::Integrity {
                code: "outcome_trading_date_vector_not_strict",
                ..
            }
        ));
    }

    #[test]
    fn due_read_model_never_calls_the_mutable_runtime_calendar() {
        let source = include_str!("selection_v2_read_model.rs");
        let forbidden = ["crate::", "calendar"].concat();
        assert!(
            !source.contains(&forbidden),
            "verified due reads must use the sample's stored immutable schedule"
        );
    }

    #[test]
    fn fixed_phase_order_is_stable() {
        assert_eq!(OutcomePhaseOrder::ordinal(OutcomePhase::T0Close), 0);
        assert_eq!(OutcomePhaseOrder::ordinal(OutcomePhase::D1Settled), 1);
        assert_eq!(OutcomePhaseOrder::ordinal(OutcomePhase::D3Settled), 2);
        assert_eq!(OutcomePhaseOrder::ordinal(OutcomePhase::D5Settled), 3);
    }

    #[test]
    fn due_capability_recomputes_and_binds_the_full_sample_identity() {
        let mut sample = receipted_sample_row();
        let verified =
            verified_sample_key_preimage(&sample).expect("full canonical identity must verify");
        assert_eq!(verified.event_id, "event-1");
        assert_eq!(verified.chain_id, "chain-1");
        assert_eq!(verified.stock_code, "600000");

        sample.event_id = "event-tampered".into();
        let error = verified_sample_key_preimage(&sample)
            .expect_err("a changed identity preimage must invalidate the due capability");
        assert!(matches!(
            error,
            SelectionV2ReadModelError::Integrity {
                code: "sample_key_preimage_mismatch",
                ..
            }
        ));
    }

    fn shanghai_tick(hour: u32, minute: u32, second: u32, nanos: u32) -> DateTime<FixedOffset> {
        FixedOffset::east_opt(8 * 60 * 60)
            .expect("+08:00")
            .with_ymd_and_hms(2026, 7, 24, hour, minute, second)
            .single()
            .expect("test Shanghai instant")
            .with_nanosecond(nanos)
            .expect("test nanoseconds")
    }

    #[test]
    fn latest_expected_wait_is_suppressed_until_one_nanosecond_after_close() {
        let due = date("2026-07-24");
        for tick in [
            shanghai_tick(14, 59, 59, 999_999_999),
            shanghai_tick(15, 0, 0, 0),
        ] {
            assert_eq!(
                classify_latest_outcome_due_status(Some(RunStatus::ExpectedWait), due, tick)
                    .expect("valid Shanghai tick"),
                LatestOutcomeDueDisposition::SuppressedExpectedWait
            );
        }
        assert_eq!(
            classify_latest_outcome_due_status(
                Some(RunStatus::ExpectedWait),
                due,
                shanghai_tick(15, 0, 0, 1)
            )
            .expect("valid Shanghai tick"),
            LatestOutcomeDueDisposition::Eligible
        );
    }

    #[test]
    fn receipted_expected_wait_suppression_is_restart_stable() {
        let due = date("2026-07-24");
        let tick = shanghai_tick(14, 45, 0, 0);
        let before_restart =
            classify_latest_outcome_due_status(Some(RunStatus::ExpectedWait), due, tick)
                .expect("first verified read");
        let after_restart =
            classify_latest_outcome_due_status(Some(RunStatus::ExpectedWait), due, tick)
                .expect("restarted verified read");

        assert_eq!(
            before_restart,
            LatestOutcomeDueDisposition::SuppressedExpectedWait
        );
        assert_eq!(after_restart, before_restart);
    }

    #[test]
    fn serial_owners_share_one_preclose_expected_wait_budget() {
        let due = date("2026-07-24");
        let tick = shanghai_tick(14, 59, 0, 0);
        let first =
            classify_latest_outcome_due_status(None, due, tick).expect("first owner verified read");
        let second = classify_latest_outcome_due_status(Some(RunStatus::ExpectedWait), due, tick)
            .expect("second owner verified read after first receipt");

        assert_eq!(
            first,
            LatestOutcomeDueDisposition::Eligible,
            "the first owner may create the single pre-close wait"
        );
        assert_eq!(
            second,
            LatestOutcomeDueDisposition::SuppressedExpectedWait,
            "the second serial owner must observe the first wait receipt and stop"
        );
    }

    #[test]
    fn non_shanghai_due_tick_fails_closed() {
        let utc = FixedOffset::east_opt(0)
            .expect("UTC offset")
            .with_ymd_and_hms(2026, 7, 24, 15, 0, 0)
            .single()
            .expect("test UTC instant");
        let error = classify_latest_outcome_due_status(
            Some(RunStatus::ExpectedWait),
            date("2026-07-24"),
            utc,
        )
        .expect_err("non-Shanghai tick must fail");
        assert!(matches!(
            error,
            SelectionV2ReadModelError::Integrity {
                code: "outcome_tick_instant_not_shanghai",
                ..
            }
        ));
    }

    #[test]
    fn eligible_after_deadline_carries_prior_wait_receipt_lineage() {
        let logical_subject_key = "a".repeat(64);
        let mut receipts = BTreeMap::new();
        receipts.insert(
            "01900000-0000-7000-8000-000000000001".into(),
            VerifiedReceipt {
                subject_kind: SubjectKind::OutcomeRun,
                logical_subject_key: logical_subject_key.clone(),
                run_status: RunStatus::ExpectedWait,
                outcome_phase: Some(OutcomePhase::T0Close),
                committed_at_rfc3339_nanos_utc: "2026-07-24T06:30:00.000000000Z".into(),
                content_hash: "1".repeat(64),
                run_manifest_content_hash: "2".repeat(64),
                committed_audit_hash: "3".repeat(64),
            },
        );

        assert_eq!(
            classify_latest_outcome_due_status(
                Some(RunStatus::ExpectedWait),
                date("2026-07-24"),
                shanghai_tick(15, 0, 0, 1)
            )
            .expect("deadline reached"),
            LatestOutcomeDueDisposition::Eligible
        );
        let lineage =
            sorted_same_subject_attempts(&receipts, &logical_subject_key, OutcomePhase::T0Close);
        assert_eq!(lineage.len(), 1);
        assert_eq!(lineage[0].1.content_hash, "1".repeat(64));
    }

    #[test]
    fn unrelated_recovery_rows_do_not_globally_block_due_outcome_subjects() {
        let source = include_str!("selection_v2_read_model.rs");
        let due = source
            .split("pub fn due_v2_outcomes(")
            .nth(1)
            .and_then(|tail| tail.split("/// Two disjoint").next())
            .expect("due outcome read");
        assert!(
            !due.contains("self.recovery_queues()"),
            "generic partial work must be owned separately, not permanently block every due subject"
        );
        assert!(due.contains("classify_outcome_claim_lifecycle"));
        assert!(due.contains("has_unreceipted_logical_subject"));
    }

    #[test]
    fn production_database_binding_consumes_checkout_scoped_descriptor_proof() {
        let source = include_str!("selection_v2_read_model.rs");
        let binding = source
            .split("fn selection_v2_database_binding(")
            .nth(1)
            .and_then(|tail| {
                tail.split("\n#[cfg(test)]\nfn build_test_database_binding(")
                    .next()
            })
            .expect("production database binding");
        assert!(binding.contains("selection_connection_bound_proof(conn)"));
        assert!(binding.contains("proof.into_preimage()"));
        assert!(
            !binding.contains("self.database_path"),
            "a path field cannot prove the actual checked-out SQLite connection"
        );
        assert!(
            !binding.contains("canonicalize"),
            "production binding must not substitute pathname canonicalization for descriptor proof"
        );
    }
}
