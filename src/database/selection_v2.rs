//! BR-174/BR-176/BR-177/BR-178 schema-v2 append-only persistence.
//!
//! This module is deliberately a deep schema seam.  Callers supply an already
//! isolated SQLite connection and an explicit store mode; the implementation
//! installs and verifies the safety pragmas, schema and direct-SQL guards.

use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Integer, Nullable, Text};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use thiserror::Error;

use crate::selection::audit::{
    LockedSelectionAuditSession, SelectionAuditPhase, SelectionAuditRecord,
};

const V2_TABLES: [&str; 12] = [
    "selection_source_batch_attempts",
    "selection_source_facts_v2",
    "selection_source_fact_attempts",
    "selection_relation_attempts",
    "selection_evaluation_attempts",
    "selection_samples",
    "selection_rejections",
    "selection_sample_outcomes",
    "selection_outcome_attempts",
    "selection_v2_recovery_envelopes",
    "selection_v2_run_stages",
    "selection_v2_commit_receipts",
];

const V2_INDEXES: [&str; 5] = [
    "selection_v2_one_activation_per_config",
    "selection_v2_source_facts_pending",
    "selection_v2_samples_generation",
    "selection_v2_outcome_attempt_run",
    "selection_v2_receipt_subject",
];

pub const STOCK_ANALYSIS_SQLITE_APPLICATION_ID: i32 = 1_398_035_265;
pub const STOCK_ANALYSIS_DB_SCHEMA_GENERATION: i32 = 1;

const V2_COLUMN_REGISTRY: [(&str, &str); 12] = [
    (
        "selection_source_batch_attempts",
        "source_batch_attempt_id,ingress_run_id,config_activation_run_id,config_hash,generation_market_date,registered_feed_identity,registered_feed_snapshot_hash,request_hash,request_evidence_json,request_evidence_hash,feed_attempt_content_hash,status_kind,record_count,provider,source,source_at,observed_at,batch_id,batch_content_hash,failed_stage,reason_code,retryable,available_evidence_json,available_evidence_hash,error_detail_json,error_detail_hash,error_fingerprint,attempted_at,content_hash",
    ),
    (
        "selection_source_facts_v2",
        "source_fact_key,event_id,payload_schema,config_activation_run_id,config_hash,generation_market_date,provider_source,item_id,title,summary,content,publisher,canonical_url,published_at,instruments_json,topics_json,language,record_provider,record_source,record_source_at,record_observed_at,record_batch_id,record_batch_content_hash,provider_content_hash,first_ingress_run_id,ingress_gate_version,ingress_gate_input_json,ingress_gate_input_hash,ingress_decision,ingress_reason_code,ingress_retryable,ingress_gate_receipt_json,ingress_gate_receipt_hash,content_hash",
    ),
    (
        "selection_source_fact_attempts",
        "source_fact_attempt_id,ingress_run_id,source_batch_attempt_id,provider_ordinal,source_fact_key,acquired_record_json,acquired_record_hash,batch_evidence_json,batch_evidence_hash,event_projection_id,attempt_result,conflict_hash,attempted_at,content_hash",
    ),
    (
        "selection_relation_attempts",
        "relation_attempt_id,relation_key,generation_run_id,source_fact_key,event_id,chain_id,config_activation_run_id,config_hash,relation_schema_version,relation_kind,relation_source_identity_json,relation_source_identity_hash,typed_binding_state_json,typed_binding_state_hash,request_hash,request_evidence_json,request_evidence_hash,result_code,failed_stage,retryable,raw_identity_json,raw_identity_hash,canonical_stock_code,canonical_stock_name,canonical_market,artifact_content_hash,binding_audit_hash,provider_board_kind,provider_board_code,provider_board_name,provider_source,provider_source_at,provider_observed_at,provider_batch_id,provider_batch_content_hash,actual_constituent_count,available_evidence_json,available_evidence_hash,error_detail_json,error_detail_hash,error_fingerprint,attempted_at,content_hash",
    ),
    (
        "selection_evaluation_attempts",
        "evaluation_attempt_id,sample_key,generation_run_id,source_fact_key,event_id,chain_id,canonical_stock_code,canonical_stock_name,canonical_market,relation_evidence_set_hash,market_request_hash,request_evidence_json,request_evidence_hash,result_code,failed_stage,retryable,provider,source,source_at,observed_at,batch_id,batch_content_hash,available_evidence_json,available_evidence_hash,terminal_decision_hash,error_detail_json,error_detail_hash,error_fingerprint,attempted_at,content_hash",
    ),
    (
        "selection_samples",
        "sample_key,generation_run_id,source_fact_key,source_fact_content_hash,source_fact_attempt_id,source_batch_attempt_id,event_id,chain_id,config_activation_run_id,config_hash,matched_keyword,canonical_stock_code,canonical_stock_name,canonical_market,relation_schema_version,relation_evidence_json,relation_evidence_set_hash,feature_version,t0_feature_json,t0_feature_hash,market_provider,market_source,market_source_at,market_observed_at,market_batch_id,market_batch_content_hash,admission_version,decision_kind,rejection_count,rejection_row_hashes_in_ordinal_order,evaluation_market_date,t0_due_date,d1_due_date,d2_due_date,d3_due_date,d4_due_date,d5_due_date,calendar_version,calendar_hash,trading_date_vector_json,trading_date_vector_hash,staged_at,content_hash",
    ),
    (
        "selection_rejections",
        "sample_key,ordinal,generation_run_id,reason_code,rule_id,retryable,structured_detail_json,structured_detail_hash,provider,source,source_at,observed_at,batch_id,batch_content_hash,created_at,content_hash",
    ),
    (
        "selection_sample_outcomes",
        "sample_key,phase,outcome_run_id,due_trading_date,open,high,low,close,volume,amount,return_from_t0_close,cumulative_mfe,cumulative_mae,volume_ratio,provider,source,source_at,observed_at,batch_id,batch_content_hash,created_at,content_hash",
    ),
    (
        "selection_outcome_attempts",
        "outcome_attempt_id,sample_key,phase,stored_due_date,outcome_run_id,request_hash,request_evidence_json,request_evidence_hash,result_code,reason_code,retryable,provider,source,source_at,observed_at,batch_id,batch_content_hash,available_evidence_json,available_evidence_hash,error_detail_json,error_detail_hash,error_fingerprint,settled_outcome_content_hash,attempted_at,content_hash",
    ),
    (
        "selection_v2_recovery_envelopes",
        "stage_run_id,subject_kind,logical_subject_key,payload_schema,payload_json,payload_json_hash,in_memory_payload_hash,config_activation_run_id,config_hash,enveloped_at,content_hash",
    ),
    (
        "selection_v2_run_stages",
        "subject_kind,subject_id,in_memory_payload_hash,prepared_record_hash,expected_staged_row_count,staged_db_content_hash,recovery_envelope_content_hash,logical_subject_key,run_status,source_fact_key,config_activation_run_id,config_hash,config_snapshot_json_hash,config_activation_content_hash,config_activation_file_content_hash,config_effective_from,artifact_valid_from,artifact_expires_at,executable_revision,legacy_cutover_snapshot_hash,generation_market_date,aggregator_observed_at,ingress_source_batch_content_hash,outcome_phase,stored_due_date,staged_at,manifest_content_hash",
    ),
    (
        "selection_v2_commit_receipts",
        "subject_kind,subject_id,logical_subject_key,in_memory_payload_hash,recovery_envelope_content_hash,prepared_audit_hash,run_manifest_content_hash,staged_db_content_hash,committed_audit_hash,committed_at,content_hash",
    ),
];

// BR-180 freezes the complete executable pre-amendment SQL, not a subset of
// columns or a handful of discriminator tokens. This digest covers all twelve
// table DDL statements, five explicit indexes and the static trigger bodies
// after the exact D2/D4/vector amendment is reversed. Dynamic stage,
// append-only and symbol-isolation triggers are compared separately.
const PRE_AMENDMENT_STATIC_SCHEMA_SHA256: &str =
    "71115640dd4cd5412497845bd81d63e1e9a175dea06f4a69e0629f41580f99dd";
const PRE_AMENDMENT_SCHEMA: &str = include_str!("fixtures/selection_v2_pre_amendment.sql");

const STATIC_TRIGGER_NAMES: [&str; 17] = [
    "selection_v2_batch_lineage",
    "selection_v2_fact_lineage",
    "selection_v2_fact_attempt_lineage",
    "selection_v2_relation_requires_admitted_source",
    "selection_v2_evaluation_requires_admitted_source",
    "selection_v2_sample_requires_admitted_source",
    "selection_v2_rejection_requires_admitted_source",
    "selection_v2_manifest_envelope_binding",
    "selection_v2_config_manifest_closure",
    "selection_v2_ingress_manifest_closure",
    "selection_v2_generation_manifest_closure",
    "selection_v2_outcome_manifest_closure",
    "selection_v2_receipt_manifest_binding",
    "selection_v2_config_receipt_closure",
    "selection_v2_ingress_receipt_closure",
    "selection_v2_generation_receipt_closure",
    "selection_v2_outcome_receipt_closure",
];

const STAGE_MEMBERSHIPS: [(&str, &str, &str); 9] = [
    (
        "selection_source_batch_attempts",
        "ingress_run_id",
        "ingress_run",
    ),
    (
        "selection_source_facts_v2",
        "first_ingress_run_id",
        "ingress_run",
    ),
    (
        "selection_source_fact_attempts",
        "ingress_run_id",
        "ingress_run",
    ),
    (
        "selection_relation_attempts",
        "generation_run_id",
        "generation_run",
    ),
    (
        "selection_evaluation_attempts",
        "generation_run_id",
        "generation_run",
    ),
    ("selection_samples", "generation_run_id", "generation_run"),
    (
        "selection_rejections",
        "generation_run_id",
        "generation_run",
    ),
    ("selection_sample_outcomes", "outcome_run_id", "outcome_run"),
    (
        "selection_outcome_attempts",
        "outcome_run_id",
        "outcome_run",
    ),
];

/// BR-174/BR-178 physical symbol namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionV2StoreMode {
    Production,
    Test,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionV2AffectedTableCount {
    pub table: &'static str,
    pub rows: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionV2MigrationState {
    /// The database half is absent, but the CLI must validate the fixed audit
    /// chain before it may render the authoritative word `absent`.
    DatabaseAbsentAuditUnverified,
    /// The four-payload schema embedded in this module predates the final
    /// config/ingress/generation/outcome-claim/outcome target contract.
    TransitionalCurrent {
        store_mode: SelectionV2StoreMode,
        nonempty_legacy_v2_outcome_tables: Vec<SelectionV2AffectedTableCount>,
    },
    /// Exact five-payload SQLite catalog and STSA/1 identity were verified.
    /// This remains diagnostic-only until the audit half is validated by the
    /// global owner; it must never be treated as authoritative `Amended`.
    FinalTargetDatabaseHalf { store_mode: SelectionV2StoreMode },
    PreAmendment {
        store_mode: SelectionV2StoreMode,
        nonempty_tables: Vec<SelectionV2AffectedTableCount>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use = "operator migration preflight must be inspected before any apply decision"]
pub struct SelectionV2MigrationPreflight {
    pub state: SelectionV2MigrationState,
    pub integrity_check: &'static str,
    pub foreign_key_violations: i64,
    pub apply_supported: bool,
    pub apply_blocker: &'static str,
}

pub const SELECTION_V2_APPLY_BLOCKER: &str =
    "BR-180 apply disabled: final five-payload target including outcome-claim-stage-v2/outcome-stage-v3 schema, parser, receipt owner, maintenance locks, full-file backup+fsync, and atomic exchange are not all implemented";

#[derive(Debug, Error)]
pub enum SelectionV2SchemaError {
    #[error("selection-v2 database error: {0}")]
    Database(#[from] diesel::result::Error),
    #[error("selection-v2 unsafe SQLite pragma {name}: expected={expected} actual={actual}")]
    UnsafePragma {
        name: &'static str,
        expected: i32,
        actual: i32,
    },
    #[error(
        "selection-v2 schema mismatch for {table}.{column}: expected={expected} actual={actual}"
    )]
    SchemaMismatch {
        table: &'static str,
        column: &'static str,
        expected: &'static str,
        actual: String,
    },
    #[error(
        "selection-v2 store mode conflicts with the symbol-isolation triggers already in the database"
    )]
    StoreModeConflict,
    #[error("selection-v2 foreign-key check failed with {violations} violation(s)")]
    ForeignKeyViolation { violations: i64 },
    #[error("selection-v2 managed trigger set is missing, extra, or non-canonical: {name}")]
    TriggerMismatch { name: String },
    #[error("selection-v2 contains symbols that violate the selected store mode")]
    ExistingSymbolViolation,
    #[error("selection-v2 audit validation failed: {detail}")]
    AuditValidation { detail: String },
    #[error("selection-v2 target schema is incomplete: {detail}")]
    IncompleteTarget { detail: &'static str },
    #[error(
        "selection-v2 database uses unsupported future global schema generation {actual}; maximum supported is {supported}"
    )]
    UnsupportedFutureGeneration { actual: i32, supported: i32 },
}

pub type SelectionV2SchemaResult<T> = Result<T, SelectionV2SchemaError>;

#[derive(QueryableByName)]
struct ForeignKeysPragma {
    #[diesel(sql_type = Integer)]
    foreign_keys: i32,
}

#[derive(QueryableByName)]
struct SynchronousPragma {
    #[diesel(sql_type = Integer)]
    synchronous: i32,
}

#[derive(QueryableByName)]
struct QueryOnlyPragma {
    #[diesel(sql_type = Integer)]
    query_only: i32,
}

#[derive(QueryableByName)]
struct ApplicationIdPragma {
    #[diesel(sql_type = Integer)]
    application_id: i32,
}

#[derive(QueryableByName)]
struct UserVersionPragma {
    #[diesel(sql_type = Integer)]
    user_version: i32,
}

#[derive(QueryableByName)]
struct CountRow {
    #[diesel(sql_type = BigInt)]
    count: i64,
}

#[derive(QueryableByName)]
struct TriggerSqlRow {
    #[diesel(sql_type = Text)]
    sql: String,
}

#[derive(QueryableByName)]
struct SchemaObjectDependencyRow {
    #[diesel(sql_type = Text)]
    object_type: String,
    #[diesel(sql_type = Text)]
    name: String,
    #[diesel(sql_type = Text)]
    table_name: String,
    #[diesel(sql_type = Text)]
    sql: String,
}

#[derive(Debug, Clone, PartialEq, Eq, QueryableByName)]
struct ManagedSchemaCatalogRow {
    #[diesel(sql_type = Text)]
    object_type: String,
    #[diesel(sql_type = Text)]
    name: String,
    #[diesel(sql_type = Text)]
    table_name: String,
    #[diesel(sql_type = Text)]
    sql: String,
}

#[derive(Debug, Clone, PartialEq, Eq, QueryableByName)]
struct IndexListCatalogRow {
    #[diesel(sql_type = Text)]
    name: String,
    #[diesel(sql_type = Integer)]
    unique_flag: i32,
    #[diesel(sql_type = Text)]
    origin: String,
    #[diesel(sql_type = Integer)]
    partial: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, QueryableByName)]
struct IndexXInfoCatalogRow {
    #[diesel(sql_type = Integer)]
    seqno: i32,
    #[diesel(sql_type = Integer)]
    cid: i32,
    #[diesel(sql_type = Nullable<Text>)]
    name: Option<String>,
    #[diesel(sql_type = Integer)]
    desc_flag: i32,
    #[diesel(sql_type = Nullable<Text>)]
    collation: Option<String>,
    #[diesel(sql_type = Integer)]
    key_flag: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ManagedIndexCatalogRow {
    table: &'static str,
    index: IndexListCatalogRow,
    columns: Vec<IndexXInfoCatalogRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SqliteRuntimeIdentity {
    version_number: u32,
    source_id: String,
    compile_options_hash: String,
}

#[derive(QueryableByName)]
struct IntegrityCheckRow {
    #[diesel(sql_type = Text)]
    integrity_check: String,
}

#[derive(QueryableByName)]
struct ColumnNullabilityRow {
    #[diesel(sql_type = Integer)]
    not_null: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectionV2SchemaRevision {
    Absent,
    PreAmendment,
    TransitionalCurrent,
    FinalDatabaseHalf,
}

#[derive(QueryableByName)]
struct TextRow {
    #[diesel(sql_type = Text)]
    value: String,
}

/// Validate the selected schema-v2 namespace without installing the known
/// incomplete four-payload schema. Production additionally requires a locked,
/// fully validated audit-chain capability.
pub fn initialize_selection_v2_schema(
    conn: &mut SqliteConnection,
    mode: SelectionV2StoreMode,
) -> SelectionV2SchemaResult<()> {
    if mode == SelectionV2StoreMode::Production {
        return Err(SelectionV2SchemaError::SchemaMismatch {
            table: "selection_v2_table_set",
            column: "audit_phase",
            expected: "validated selection audit state",
            actual: "unknown".to_owned(),
        });
    }
    initialize_selection_v2_schema_with_audit_state(conn, mode, false)
}

/// Test-only explicit audit-state seam. A raw boolean is not a production
/// capability: production must present an unforgeable locked audit session.
fn initialize_selection_v2_schema_with_audit_state(
    conn: &mut SqliteConnection,
    mode: SelectionV2StoreMode,
    v2_audit_phase_exists: bool,
) -> SelectionV2SchemaResult<()> {
    if mode == SelectionV2StoreMode::Production {
        return Err(SelectionV2SchemaError::SchemaMismatch {
            table: "selection_v2_table_set",
            column: "audit_phase",
            expected: "validated locked selection audit session",
            actual: "raw boolean state".to_owned(),
        });
    }
    initialize_selection_v2_schema_with_validated_audit_state(conn, mode, v2_audit_phase_exists)
}

/// Production-capable initialization requires the exact locked session that
/// performed full on-disk chain validation. Callers cannot manufacture this
/// capability from a boolean or an in-memory record list.
pub(super) fn initialize_selection_v2_schema_with_audit_session(
    conn: &mut SqliteConnection,
    mode: SelectionV2StoreMode,
    audit_session: &mut LockedSelectionAuditSession<'_>,
) -> SelectionV2SchemaResult<()> {
    let snapshot = audit_session.validated_records().map_err(|error| {
        SelectionV2SchemaError::AuditValidation {
            detail: error.to_string(),
        }
    })?;
    let v2_audit_phase_exists = snapshot
        .records()
        .iter()
        .any(selection_v2_audit_record_is_v2);
    initialize_selection_v2_schema_with_validated_audit_state(conn, mode, v2_audit_phase_exists)
}

fn selection_v2_audit_record_is_v2(record: &SelectionAuditRecord) -> bool {
    match record.phase {
        SelectionAuditPhase::Ingested
        | SelectionAuditPhase::Prepared
        | SelectionAuditPhase::Committed
        | SelectionAuditPhase::Rejected
        | SelectionAuditPhase::Completed
        | SelectionAuditPhase::T0Close
        | SelectionAuditPhase::D1Settled => false,
        SelectionAuditPhase::V2ConfigActivationPrepared
        | SelectionAuditPhase::V2ConfigActivationCommitted
        | SelectionAuditPhase::V2IngressPrepared
        | SelectionAuditPhase::V2IngressCommitted
        | SelectionAuditPhase::V2GenerationPrepared
        | SelectionAuditPhase::V2GenerationCommitted
        | SelectionAuditPhase::V2OutcomeClaimPrepared
        | SelectionAuditPhase::V2OutcomeClaimCommitted
        | SelectionAuditPhase::V2OutcomePrepared
        | SelectionAuditPhase::V2OutcomeCommitted
        | SelectionAuditPhase::V2BoardBindingAuditPrepared
        | SelectionAuditPhase::V2BoardBindingAuditCommitted
        | SelectionAuditPhase::V2GateDCanaryVerified => true,
    }
}

/// Initialize schema-v2 only after the caller has validated the fixed
/// selection audit root. An audit-only state must never be repaired by
/// silently creating empty tables because that would sever durable history.
fn initialize_selection_v2_schema_with_validated_audit_state(
    conn: &mut SqliteConnection,
    mode: SelectionV2StoreMode,
    v2_audit_phase_exists: bool,
) -> SelectionV2SchemaResult<()> {
    conn.batch_execute("PRAGMA foreign_keys = ON; PRAGMA synchronous = FULL;")?;
    verify_pragmas(conn)?;
    verify_foreign_keys(conn)?;
    match classify_selection_v2_schema(conn)? {
        SelectionV2SchemaRevision::Absent => {
            if v2_audit_phase_exists {
                return Err(SelectionV2SchemaError::SchemaMismatch {
                    table: "selection_v2_table_set",
                    column: "audit_phase",
                    expected: "no v2 audit phase when all tables are absent",
                    actual: "present".to_owned(),
                });
            }
            verify_integrity(conn)?;
            Err(incomplete_target_error())
        }
        SelectionV2SchemaRevision::TransitionalCurrent => {
            verify_transitional_current_schema_contract(conn)?;
            if detect_store_mode(conn)? != mode {
                return Err(SelectionV2SchemaError::StoreModeConflict);
            }
            // The transitional database is validation-only. Missing safety
            // objects are drift, but a complete four-payload schema still
            // cannot enable new selection work.
            verify_trigger_registry(conn, mode)?;
            Err(incomplete_target_error())
        }
        SelectionV2SchemaRevision::FinalDatabaseHalf => {
            if detect_store_mode(conn)? != mode {
                return Err(SelectionV2SchemaError::StoreModeConflict);
            }
            verify_final_database_half_schema_contract(conn, mode)?;
            verify_integrity(conn)?;
            Err(incomplete_target_error())
        }
        SelectionV2SchemaRevision::PreAmendment => {
            let nonempty = affected_nonempty_table_counts(conn)?;
            Err(SelectionV2SchemaError::SchemaMismatch {
                table: "selection_v2_table_set",
                column: "revision",
                expected: "final five-payload target or explicit operator migration",
                actual: format_pre_amendment_failure(&nonempty),
            })
        }
    }
}

/// Recheck the connection invariant before a stage/receipt transaction.
pub fn verify_selection_v2_connection(conn: &mut SqliteConnection) -> SelectionV2SchemaResult<()> {
    verify_pragmas(conn)?;
    verify_foreign_keys(conn)?;
    match classify_selection_v2_schema(conn)? {
        SelectionV2SchemaRevision::FinalDatabaseHalf => {
            let mode = detect_store_mode(conn)?;
            verify_final_database_half_schema_contract(conn, mode)?;
            Err(incomplete_target_error())
        }
        SelectionV2SchemaRevision::TransitionalCurrent => {
            verify_transitional_current_schema_contract(conn)?;
            let mode = detect_store_mode(conn)?;
            verify_trigger_registry(conn, mode)?;
            Err(incomplete_target_error())
        }
        SelectionV2SchemaRevision::Absent | SelectionV2SchemaRevision::PreAmendment => {
            Err(incomplete_target_error())
        }
    }
}

fn incomplete_target_error() -> SelectionV2SchemaError {
    SelectionV2SchemaError::IncompleteTarget {
        detail: "database-half catalog exists, but authoritative audit classification plus the complete outcome_claim parser/owner/recovery contract is not release-enabled",
    }
}

/// Performs the database half of the BR-180 operator preflight without
/// mutating schema, rows, PRAGMAs, audit files, backups, or lock files.
///
/// The separately gated apply path remains disabled until the complete
/// outcome-claim and full-file exchange contract exists. Returning a typed
/// blocker makes a dry run useful without pretending that migration is safe.
pub fn inspect_selection_v2_migration(
    conn: &mut SqliteConnection,
) -> SelectionV2SchemaResult<SelectionV2MigrationPreflight> {
    conn.transaction::<_, SelectionV2SchemaError, _>(inspect_selection_v2_migration_snapshot)
}

fn inspect_selection_v2_migration_snapshot(
    conn: &mut SqliteConnection,
) -> SelectionV2SchemaResult<SelectionV2MigrationPreflight> {
    let query_only = diesel::sql_query("PRAGMA query_only").get_result::<QueryOnlyPragma>(conn)?;
    if query_only.query_only != 1 {
        return Err(SelectionV2SchemaError::UnsafePragma {
            name: "query_only",
            expected: 1,
            actual: query_only.query_only,
        });
    }
    verify_integrity(conn)?;
    let foreign_key_violations =
        diesel::sql_query("SELECT COUNT(*) AS count FROM pragma_foreign_key_check")
            .get_result::<CountRow>(conn)?
            .count;
    if foreign_key_violations != 0 {
        return Err(SelectionV2SchemaError::ForeignKeyViolation {
            violations: foreign_key_violations,
        });
    }

    let state = match classify_selection_v2_schema(conn)? {
        SelectionV2SchemaRevision::Absent => {
            SelectionV2MigrationState::DatabaseAbsentAuditUnverified
        }
        SelectionV2SchemaRevision::TransitionalCurrent => {
            SelectionV2MigrationState::TransitionalCurrent {
                store_mode: detect_store_mode(conn)?,
                nonempty_legacy_v2_outcome_tables: legacy_v2_outcome_table_counts(conn)?,
            }
        }
        SelectionV2SchemaRevision::FinalDatabaseHalf => {
            SelectionV2MigrationState::FinalTargetDatabaseHalf {
                store_mode: detect_store_mode(conn)?,
            }
        }
        SelectionV2SchemaRevision::PreAmendment => SelectionV2MigrationState::PreAmendment {
            store_mode: detect_store_mode(conn)?,
            nonempty_tables: affected_nonempty_table_counts(conn)?,
        },
    };

    Ok(SelectionV2MigrationPreflight {
        state,
        integrity_check: "ok",
        foreign_key_violations,
        apply_supported: false,
        apply_blocker: SELECTION_V2_APPLY_BLOCKER,
    })
}

fn classify_selection_v2_schema(
    conn: &mut SqliteConnection,
) -> SelectionV2SchemaResult<SelectionV2SchemaRevision> {
    let (application_id, user_version) = read_global_schema_identity(conn)?;
    if application_id == STOCK_ANALYSIS_SQLITE_APPLICATION_ID
        && user_version > STOCK_ANALYSIS_DB_SCHEMA_GENERATION
    {
        return Err(SelectionV2SchemaError::UnsupportedFutureGeneration {
            actual: user_version,
            supported: STOCK_ANALYSIS_DB_SCHEMA_GENERATION,
        });
    }
    let present = diesel::sql_query(
        "SELECT COUNT(*) AS count FROM sqlite_master
         WHERE type='table' AND lower(name) IN (
             'selection_source_batch_attempts',
             'selection_source_facts_v2',
             'selection_source_fact_attempts',
             'selection_relation_attempts',
             'selection_evaluation_attempts',
             'selection_samples',
             'selection_rejections',
             'selection_sample_outcomes',
             'selection_outcome_attempts',
             'selection_v2_recovery_envelopes',
             'selection_v2_run_stages',
             'selection_v2_commit_receipts'
         )",
    )
    .get_result::<CountRow>(conn)?;
    if present.count == 0 {
        verify_no_unexpected_affected_objects(conn, 0)?;
        require_global_schema_identity(application_id, user_version, 0, 0)?;
        return Ok(SelectionV2SchemaRevision::Absent);
    }
    if present.count != V2_TABLES.len() as i64 {
        return Err(SelectionV2SchemaError::SchemaMismatch {
            table: "selection_v2_table_set",
            column: "revision",
            expected: "all absent, exact pre-amendment, or transitional-current",
            actual: format!(
                "classified drift: partial affected table set ({}/{})",
                present.count,
                V2_TABLES.len()
            ),
        });
    }

    if application_id == STOCK_ANALYSIS_SQLITE_APPLICATION_ID
        && user_version == STOCK_ANALYSIS_DB_SCHEMA_GENERATION
    {
        let mode = detect_store_mode(conn)?;
        verify_final_database_half_schema_contract(conn, mode)?;
        return Ok(SelectionV2SchemaRevision::FinalDatabaseHalf);
    }

    require_global_schema_identity(application_id, user_version, 0, 0)?;
    if semantic_schema_match(|| {
        verify_transitional_current_schema_contract(conn)?;
        let mode = detect_store_mode(conn)?;
        verify_trigger_registry(conn, mode)
    })? {
        return Ok(SelectionV2SchemaRevision::TransitionalCurrent);
    }
    if semantic_schema_match(|| {
        verify_pre_amendment_schema_contract(conn)?;
        let mode = detect_store_mode(conn)?;
        verify_trigger_registry(conn, mode)
    })? {
        return Ok(SelectionV2SchemaRevision::PreAmendment);
    }

    Err(SelectionV2SchemaError::SchemaMismatch {
        table: "selection_v2_table_set",
        column: "revision",
        expected:
            "all absent, exact pre-amendment, transitional-current, or exact final database half",
        actual: "classified drift: affected table/index/trigger/FK/CHECK registry mismatch"
            .to_owned(),
    })
}

fn read_global_schema_identity(conn: &mut SqliteConnection) -> SelectionV2SchemaResult<(i32, i32)> {
    let application_id =
        diesel::sql_query("PRAGMA application_id").get_result::<ApplicationIdPragma>(conn)?;
    let user_version =
        diesel::sql_query("PRAGMA user_version").get_result::<UserVersionPragma>(conn)?;
    Ok((application_id.application_id, user_version.user_version))
}

fn require_global_schema_identity(
    actual_application_id: i32,
    actual_user_version: i32,
    expected_application_id: i32,
    expected_user_version: i32,
) -> SelectionV2SchemaResult<()> {
    if actual_application_id == expected_application_id
        && actual_user_version == expected_user_version
    {
        return Ok(());
    }
    Err(SelectionV2SchemaError::SchemaMismatch {
        table: "selection_v2_table_set",
        column: "global_schema_identity",
        expected: if expected_application_id == 0 {
            "application_id=0,user_version=0"
        } else {
            "application_id=1398035265,user_version=1"
        },
        actual: format!(
            "application_id={actual_application_id},user_version={actual_user_version}"
        ),
    })
}

fn semantic_schema_match(
    check: impl FnOnce() -> SelectionV2SchemaResult<()>,
) -> SelectionV2SchemaResult<bool> {
    match check() {
        Ok(()) => Ok(true),
        Err(SelectionV2SchemaError::Database(diesel::result::Error::NotFound)) => Ok(false),
        Err(error @ SelectionV2SchemaError::Database(_)) => Err(error),
        Err(_) => Ok(false),
    }
}

fn verify_pre_amendment_schema_contract(
    conn: &mut SqliteConnection,
) -> SelectionV2SchemaResult<()> {
    let golden = PRE_AMENDMENT_SCHEMA;
    let digest = hex::encode(Sha256::digest(golden.as_bytes()));
    if digest != PRE_AMENDMENT_STATIC_SCHEMA_SHA256 {
        return Err(SelectionV2SchemaError::SchemaMismatch {
            table: "selection_v2_table_set",
            column: "pre_amendment_golden_hash",
            expected: PRE_AMENDMENT_STATIC_SCHEMA_SHA256,
            actual: digest,
        });
    }

    for table in V2_TABLES {
        let actual = table_sql(conn, table)?;
        let expected =
            schema_statement_from(golden, &format!("CREATE TABLE IF NOT EXISTS {table}"))
                .ok_or_else(|| SelectionV2SchemaError::SchemaMismatch {
                    table,
                    column: "pre_amendment_table_ddl_registry",
                    expected: "complete frozen pre-amendment table DDL",
                    actual: "missing from internal golden".to_owned(),
                })?;
        if normalize_schema_object_sql(&actual) != normalize_schema_object_sql(expected) {
            return Err(SelectionV2SchemaError::SchemaMismatch {
                table,
                column: "pre_amendment_table_ddl_registry",
                expected: "exact frozen pre-amendment PK/FK/UNIQUE/CHECK/default table DDL",
                actual: "non-canonical pre-amendment table DDL".to_owned(),
            });
        }
    }

    for (table, transitional_columns) in V2_COLUMN_REGISTRY {
        let expected_columns = if table == "selection_samples" {
            transitional_columns
                .split(',')
                .filter(|column| {
                    !matches!(
                        *column,
                        "d2_due_date"
                            | "d4_due_date"
                            | "trading_date_vector_json"
                            | "trading_date_vector_hash"
                    )
                })
                .collect::<Vec<_>>()
                .join(",")
        } else {
            transitional_columns.to_owned()
        };
        let query = format!(
            "SELECT group_concat(name, ',') AS value
             FROM (SELECT name FROM pragma_table_xinfo('{table}') ORDER BY cid)"
        );
        let actual = diesel::sql_query(query).get_result::<TextRow>(conn)?.value;
        if actual != expected_columns {
            return Err(SelectionV2SchemaError::SchemaMismatch {
                table,
                column: "pre_amendment_field_registry",
                expected: "exact frozen pre-amendment field order",
                actual,
            });
        }
    }

    verify_explicit_index_registry_against(conn, golden)?;
    verify_no_unexpected_affected_objects(conn, 53)
}

fn replace_schema_exact_once(
    input: String,
    from: &str,
    to: &str,
    field: &'static str,
) -> SelectionV2SchemaResult<String> {
    let occurrences = input.match_indices(from).count();
    if occurrences != 1 {
        return Err(SelectionV2SchemaError::SchemaMismatch {
            table: "selection_v2_table_set",
            column: field,
            expected: "exactly one frozen amendment token",
            actual: format!("{occurrences} matching tokens"),
        });
    }
    Ok(input.replacen(from, to, 1))
}

fn schema_statement_from<'a>(schema: &'a str, marker: &str) -> Option<&'a str> {
    let start = schema.find(marker)?;
    let remainder = &schema[start..];
    let end = remainder.find(';')? + 1;
    Some(&remainder[..end])
}

fn verify_explicit_index_registry_against(
    conn: &mut SqliteConnection,
    schema: &str,
) -> SelectionV2SchemaResult<()> {
    let explicit_index_count = diesel::sql_query(
        "SELECT COUNT(*) AS count FROM sqlite_master
         WHERE type='index' AND sql IS NOT NULL
           AND tbl_name IN (
             'selection_source_batch_attempts','selection_source_facts_v2',
             'selection_source_fact_attempts','selection_relation_attempts',
             'selection_evaluation_attempts','selection_samples',
             'selection_rejections','selection_sample_outcomes',
             'selection_outcome_attempts','selection_v2_recovery_envelopes',
             'selection_v2_run_stages','selection_v2_commit_receipts'
           )",
    )
    .get_result::<CountRow>(conn)?;
    if explicit_index_count.count != V2_INDEXES.len() as i64 {
        return Err(SelectionV2SchemaError::SchemaMismatch {
            table: "selection_v2_table_set",
            column: "index_registry",
            expected: "exact five canonical explicit indexes",
            actual: format!("{} explicit affected indexes", explicit_index_count.count),
        });
    }

    for index in V2_INDEXES {
        let actual = schema_object_sql(conn, "index", index)?;
        let expected = [
            format!("CREATE UNIQUE INDEX IF NOT EXISTS {index}"),
            format!("CREATE INDEX IF NOT EXISTS {index}"),
        ]
        .iter()
        .find_map(|marker| schema_statement_from(schema, marker))
        .ok_or_else(|| SelectionV2SchemaError::SchemaMismatch {
            table: "selection_v2_table_set",
            column: "index_registry",
            expected: "canonical index statement",
            actual: format!("{index} missing from internal golden"),
        })?;
        if normalize_schema_object_sql(&actual) != normalize_schema_object_sql(expected) {
            return Err(SelectionV2SchemaError::SchemaMismatch {
                table: "selection_v2_table_set",
                column: "index_registry",
                expected: "exact canonical index DDL",
                actual: format!("non-canonical index {index}"),
            });
        }
    }
    Ok(())
}

fn verify_no_unexpected_affected_objects(
    conn: &mut SqliteConnection,
    expected_managed_trigger_count: i64,
) -> SelectionV2SchemaResult<()> {
    let unexpected_named_objects = diesel::sql_query(
        "SELECT name AS value FROM sqlite_master
         WHERE type IN ('table','index','trigger','view')
           AND substr(lower(name),1,10)='selection_'
         ORDER BY CAST(name AS BLOB)",
    )
    .load::<TextRow>(conn)?
    .into_iter()
    .map(|row| row.value)
    .filter(|name| expected_managed_trigger_count == 0 || !known_selection_v2_object_name(name))
    .collect::<Vec<_>>();
    let unexpected_v2_tables = diesel::sql_query(
        "SELECT COUNT(*) AS count FROM sqlite_master
         WHERE type='table' AND substr(lower(name),1,10)='selection_'
           AND lower(name) NOT IN (
             'selection_source_batch_attempts',
             'selection_source_facts_v2',
             'selection_source_fact_attempts',
             'selection_relation_attempts',
             'selection_evaluation_attempts',
             'selection_samples',
             'selection_rejections',
             'selection_sample_outcomes',
             'selection_outcome_attempts',
             'selection_v2_recovery_envelopes',
             'selection_v2_run_stages',
             'selection_v2_commit_receipts'
           )",
    )
    .get_result::<CountRow>(conn)?;
    let managed_trigger_count = diesel::sql_query(
        "SELECT COUNT(*) AS count FROM sqlite_master
         WHERE type='trigger' AND lower(tbl_name) IN (
             'selection_source_batch_attempts',
             'selection_source_facts_v2',
             'selection_source_fact_attempts',
             'selection_relation_attempts',
             'selection_evaluation_attempts',
             'selection_samples',
             'selection_rejections',
             'selection_sample_outcomes',
             'selection_outcome_attempts',
             'selection_v2_recovery_envelopes',
             'selection_v2_run_stages',
             'selection_v2_commit_receipts'
         )",
    )
    .get_result::<CountRow>(conn)?;
    let external_objects = diesel::sql_query(
        "SELECT type AS object_type, name, tbl_name AS table_name, sql
         FROM sqlite_master
         WHERE type IN ('view','trigger') AND sql IS NOT NULL",
    )
    .load::<SchemaObjectDependencyRow>(conn)?;
    let mut external_sql_dependencies = Vec::new();
    for object in external_objects {
        let external_view = object.object_type == "view";
        let external_trigger =
            object.object_type == "trigger" && !V2_TABLES.contains(&object.table_name.as_str());
        if (external_view || external_trigger) && schema_sql_references_affected_table(&object.sql)?
        {
            external_sql_dependencies.push(format!("{}:{}", object.object_type, object.name));
        }
    }

    let external_fk_dependencies = external_foreign_key_dependencies(conn)?;
    if !unexpected_named_objects.is_empty()
        || unexpected_v2_tables.count != 0
        || managed_trigger_count.count != expected_managed_trigger_count
        || !external_sql_dependencies.is_empty()
        || !external_fk_dependencies.is_empty()
    {
        return Err(SelectionV2SchemaError::SchemaMismatch {
            table: "selection_v2_table_set",
            column: "object_registry",
            expected: "exact expected managed-trigger count and no unexpected selection table or external dependency",
            actual: format!(
                "unexpected_named=[{}] unexpected_tables={} managed_triggers={}/{} external_sql=[{}] external_fk=[{}]",
                unexpected_named_objects.join(","),
                unexpected_v2_tables.count,
                managed_trigger_count.count,
                expected_managed_trigger_count,
                external_sql_dependencies.join(","),
                external_fk_dependencies.join(",")
            ),
        });
    }
    Ok(())
}

fn known_selection_v2_object_name(name: &str) -> bool {
    V2_TABLES
        .iter()
        .chain(V2_INDEXES.iter())
        .chain(STATIC_TRIGGER_NAMES.iter())
        .any(|known| name.eq_ignore_ascii_case(known))
        || STAGE_MEMBERSHIPS.iter().any(|(table, _, _)| {
            name.eq_ignore_ascii_case(&format!("selection_v2_{table}_stage_membership"))
        })
        || V2_TABLES.iter().any(|table| {
            name.eq_ignore_ascii_case(&format!("{table}_deny_update"))
                || name.eq_ignore_ascii_case(&format!("{table}_deny_delete"))
        })
        || [
            "selection_v2_relation_symbol_isolation_production",
            "selection_v2_evaluation_symbol_isolation_production",
            "selection_v2_sample_symbol_isolation_production",
            "selection_v2_relation_symbol_isolation_test",
            "selection_v2_evaluation_symbol_isolation_test",
            "selection_v2_sample_symbol_isolation_test",
        ]
        .iter()
        .any(|known| name.eq_ignore_ascii_case(known))
}

fn schema_sql_references_affected_table(sql: &str) -> SelectionV2SchemaResult<bool> {
    let bytes = sql.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            byte if byte.is_ascii_whitespace() => index += 1,
            b'-' if bytes.get(index + 1) == Some(&b'-') => {
                index += 2;
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index += 2;
                let mut terminated = false;
                while index + 1 < bytes.len() {
                    if bytes[index] == b'/' && bytes[index + 1] == b'*' {
                        return Err(external_sql_scan_error("nested block comment is forbidden"));
                    }
                    if bytes[index] == b'*' && bytes[index + 1] == b'/' {
                        index += 2;
                        terminated = true;
                        break;
                    }
                    index += 1;
                }
                if !terminated {
                    return Err(external_sql_scan_error("unterminated block comment"));
                }
            }
            b'x' | b'X' if bytes.get(index + 1) == Some(&b'\'') => {
                index += 1;
                let value = read_sql_quoted_token(bytes, &mut index, b'\'', b'\'')?;
                if value.len() % 2 != 0 || !value.iter().all(u8::is_ascii_hexdigit) {
                    return Err(external_sql_scan_error("invalid SQLite hex blob literal"));
                }
            }
            b'\'' | b'"' | b'`' => {
                let delimiter = bytes[index];
                let value = read_sql_quoted_token(bytes, &mut index, delimiter, delimiter)?;
                if managed_identifier_bytes(&value) {
                    return Ok(true);
                }
            }
            b'[' => {
                let value = read_sql_quoted_token(bytes, &mut index, b'[', b']')?;
                if managed_identifier_bytes(&value) {
                    return Ok(true);
                }
            }
            byte if byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$') => {
                let start = index;
                index += 1;
                while index < bytes.len()
                    && (bytes[index].is_ascii_alphanumeric() || matches!(bytes[index], b'_' | b'$'))
                {
                    index += 1;
                }
                if managed_identifier_bytes(&bytes[start..index]) {
                    return Ok(true);
                }
            }
            byte if b"(),.;=+-*%<>!|&~?/".contains(&byte) => index += 1,
            _ => {
                return Err(external_sql_scan_error(
                    "unparsed token class in external sqlite_schema SQL",
                ));
            }
        }
    }
    Ok(false)
}

fn read_sql_quoted_token(
    sql: &[u8],
    index: &mut usize,
    opening: u8,
    closing: u8,
) -> SelectionV2SchemaResult<Vec<u8>> {
    debug_assert_eq!(sql.get(*index), Some(&opening));
    *index += 1;
    let mut decoded = Vec::new();
    while *index < sql.len() {
        let byte = sql[*index];
        if byte == closing {
            *index += 1;
            if sql.get(*index) == Some(&closing) {
                decoded.push(closing);
                *index += 1;
                continue;
            }
            return Ok(decoded);
        }
        decoded.push(byte);
        *index += 1;
    }
    Err(external_sql_scan_error(
        "unterminated quoted token in external sqlite_schema SQL",
    ))
}

fn managed_identifier_bytes(identifier: &[u8]) -> bool {
    V2_TABLES.iter().any(|table| {
        identifier.len() == table.len()
            && identifier
                .iter()
                .zip(table.as_bytes())
                .all(|(left, right)| left.eq_ignore_ascii_case(right))
    })
}

fn external_sql_scan_error(actual: impl Into<String>) -> SelectionV2SchemaError {
    SelectionV2SchemaError::SchemaMismatch {
        table: "selection_v2_table_set",
        column: "external_sql_scanner",
        expected: "fully parsed external SQLite SQL with no managed-table identifier",
        actual: actual.into(),
    }
}

fn external_foreign_key_dependencies(
    conn: &mut SqliteConnection,
) -> SelectionV2SchemaResult<Vec<String>> {
    let tables = diesel::sql_query(
        "SELECT name AS value FROM sqlite_master
         WHERE type='table' AND name NOT LIKE 'sqlite_%'
         ORDER BY name",
    )
    .load::<TextRow>(conn)?;
    let mut dependencies = Vec::new();
    for table in tables {
        if V2_TABLES
            .iter()
            .any(|managed| table.value.eq_ignore_ascii_case(managed))
        {
            continue;
        }
        let referenced =
            diesel::sql_query("SELECT [table] AS value FROM pragma_foreign_key_list(?1)")
                .bind::<Text, _>(&table.value)
                .load::<TextRow>(conn)?;
        for referenced in referenced {
            if V2_TABLES
                .iter()
                .any(|managed| referenced.value.eq_ignore_ascii_case(managed))
            {
                dependencies.push(format!("{}->{}", table.value, referenced.value));
            }
        }
    }
    Ok(dependencies)
}

fn affected_nonempty_table_counts(
    conn: &mut SqliteConnection,
) -> SelectionV2SchemaResult<Vec<SelectionV2AffectedTableCount>> {
    let mut nonempty = Vec::new();
    for table in V2_TABLES {
        let count = diesel::sql_query(format!("SELECT COUNT(*) AS count FROM {table}"))
            .get_result::<CountRow>(conn)?
            .count;
        if count != 0 {
            nonempty.push(SelectionV2AffectedTableCount { table, rows: count });
        }
    }
    Ok(nonempty)
}

fn legacy_v2_outcome_table_counts(
    conn: &mut SqliteConnection,
) -> SelectionV2SchemaResult<Vec<SelectionV2AffectedTableCount>> {
    let mut nonempty = Vec::new();
    for (table, predicate) in [
        ("selection_sample_outcomes", None),
        ("selection_outcome_attempts", None),
        (
            "selection_v2_recovery_envelopes",
            Some("subject_kind='outcome_run' AND payload_schema='outcome-stage-v2'"),
        ),
        (
            "selection_v2_run_stages",
            Some("subject_kind='outcome_run'"),
        ),
        (
            "selection_v2_commit_receipts",
            Some("subject_kind='outcome_run'"),
        ),
    ] {
        let query = match predicate {
            Some(predicate) => {
                format!("SELECT COUNT(*) AS count FROM {table} WHERE {predicate}")
            }
            None => format!("SELECT COUNT(*) AS count FROM {table}"),
        };
        let count = diesel::sql_query(query).get_result::<CountRow>(conn)?.count;
        if count != 0 {
            nonempty.push(SelectionV2AffectedTableCount { table, rows: count });
        }
    }
    Ok(nonempty)
}

fn format_pre_amendment_failure(nonempty: &[SelectionV2AffectedTableCount]) -> String {
    let counts = nonempty
        .iter()
        .map(|entry| format!("{}={}", entry.table, entry.rows))
        .collect::<Vec<_>>()
        .join(",");
    format!("pre-amendment; nonempty=[{counts}]")
}

fn verify_transitional_current_schema_contract(
    conn: &mut SqliteConnection,
) -> SelectionV2SchemaResult<()> {
    verify_field_registry(conn)?;
    verify_canonical_table_and_index_registry(conn)?;
    verify_request_evidence_columns(conn)?;
    verify_recovery_payload_contract(conn)?;
    verify_source_batch_record_count_contract(conn)
}

fn verify_final_database_half_schema_contract(
    conn: &mut SqliteConnection,
    mode: SelectionV2StoreMode,
) -> SelectionV2SchemaResult<()> {
    let (application_id, user_version) = read_global_schema_identity(conn)?;
    require_global_schema_identity(
        application_id,
        user_version,
        STOCK_ANALYSIS_SQLITE_APPLICATION_ID,
        STOCK_ANALYSIS_DB_SCHEMA_GENERATION,
    )?;
    let actual_runtime = sqlite_runtime_identity(conn)?;

    let mut reference = SqliteConnection::establish(":memory:").map_err(|error| {
        SelectionV2SchemaError::SchemaMismatch {
            table: "selection_v2_table_set",
            column: "final_catalog_reference",
            expected: "same-runtime in-memory SQLite reference connection",
            actual: error.to_string(),
        }
    })?;
    reference.batch_execute("PRAGMA foreign_keys = ON; PRAGMA synchronous = FULL;")?;
    let reference_runtime = sqlite_runtime_identity(&mut reference)?;
    if actual_runtime != reference_runtime {
        return Err(SelectionV2SchemaError::SchemaMismatch {
            table: "selection_v2_table_set",
            column: "sqlite_runtime_identity",
            expected: "same linked SQLite runtime identity for actual and reference catalogs",
            actual: format!("actual={actual_runtime:?} reference={reference_runtime:?}"),
        });
    }
    reference.batch_execute(&selection_v2_final_schema()?)?;
    install_stage_membership_triggers(&mut reference)?;
    install_append_only_triggers(&mut reference)?;
    install_symbol_triggers(&mut reference, mode)?;

    let expected = managed_schema_catalog_rows(&mut reference)?;
    let actual = managed_schema_catalog_rows(conn)?;
    if expected.len() != 70 {
        return Err(SelectionV2SchemaError::SchemaMismatch {
            table: "selection_v2_table_set",
            column: "final_catalog_reference",
            expected: "exactly 70 registered same-runtime objects",
            actual: format!("reference emitted {} objects", expected.len()),
        });
    }
    if actual != expected {
        let first_difference = actual
            .iter()
            .zip(expected.iter())
            .position(|(actual, expected)| actual != expected)
            .map_or_else(
                || {
                    format!(
                        "cardinality actual={} expected={}",
                        actual.len(),
                        expected.len()
                    )
                },
                |index| {
                    format!(
                        "row {index} actual={:?} expected={:?}",
                        actual[index], expected[index]
                    )
                },
            );
        return Err(SelectionV2SchemaError::SchemaMismatch {
            table: "selection_v2_table_set",
            column: "final_catalog_exact_sql",
            expected: "same-runtime exact production/test 70-object catalog",
            actual: first_difference,
        });
    }
    let expected_indexes = managed_index_catalog_rows(&mut reference)?;
    let actual_indexes = managed_index_catalog_rows(conn)?;
    if actual_indexes != expected_indexes {
        return Err(SelectionV2SchemaError::SchemaMismatch {
            table: "selection_v2_table_set",
            column: "final_catalog_index_geometry",
            expected: "same-runtime exact index_list/index_xinfo catalog",
            actual: format!("actual={actual_indexes:?} expected={expected_indexes:?}"),
        });
    }
    verify_no_unexpected_affected_objects(conn, 53)?;
    verify_final_outcome_attempt_field_order(conn)?;
    verify_source_batch_record_count_contract(conn)
}

fn sqlite_runtime_identity(
    conn: &mut SqliteConnection,
) -> SelectionV2SchemaResult<SqliteRuntimeIdentity> {
    let version = diesel::sql_query("SELECT sqlite_version() AS value")
        .get_result::<TextRow>(conn)?
        .value;
    let mut parts = version.split('.');
    let major = parts.next().and_then(|part| part.parse::<u32>().ok());
    let minor = parts.next().and_then(|part| part.parse::<u32>().ok());
    let patch = parts.next().and_then(|part| part.parse::<u32>().ok());
    if parts.next().is_some() || major.is_none() || minor.is_none() || patch.is_none() {
        return Err(SelectionV2SchemaError::SchemaMismatch {
            table: "selection_v2_table_set",
            column: "sqlite_runtime_identity",
            expected: "SQLite semantic version major.minor.patch",
            actual: version,
        });
    }
    let version_number = major
        .expect("checked")
        .checked_mul(1_000_000)
        .and_then(|value| {
            minor
                .expect("checked")
                .checked_mul(1_000)
                .and_then(|minor| value.checked_add(minor))
        })
        .and_then(|value| value.checked_add(patch.expect("checked")))
        .ok_or_else(|| SelectionV2SchemaError::SchemaMismatch {
            table: "selection_v2_table_set",
            column: "sqlite_runtime_identity",
            expected: "SQLite version number without integer overflow",
            actual: version.clone(),
        })?;
    if !(3_035_000..4_000_000).contains(&version_number) {
        return Err(SelectionV2SchemaError::SchemaMismatch {
            table: "selection_v2_table_set",
            column: "sqlite_runtime_identity",
            expected: "SQLite >=3.35.0,<4.0.0",
            actual: version,
        });
    }
    let source_id = diesel::sql_query("SELECT sqlite_source_id() AS value")
        .get_result::<TextRow>(conn)?
        .value;
    if source_id.is_empty() {
        return Err(SelectionV2SchemaError::SchemaMismatch {
            table: "selection_v2_table_set",
            column: "sqlite_runtime_identity",
            expected: "nonempty sqlite_source_id",
            actual: "empty".to_owned(),
        });
    }
    let options = diesel::sql_query(
        "SELECT compile_options AS value
         FROM pragma_compile_options
         ORDER BY CAST(compile_options AS BLOB)",
    )
    .load::<TextRow>(conn)?;
    let mut digest = Sha256::new();
    for option in options {
        let length = u64::try_from(option.value.len()).map_err(|_| {
            SelectionV2SchemaError::SchemaMismatch {
                table: "selection_v2_table_set",
                column: "sqlite_runtime_identity",
                expected: "compile option length fits u64",
                actual: option.value.clone(),
            }
        })?;
        digest.update(length.to_be_bytes());
        digest.update(option.value.as_bytes());
    }
    Ok(SqliteRuntimeIdentity {
        version_number,
        source_id,
        compile_options_hash: hex::encode(digest.finalize()),
    })
}

fn managed_schema_catalog_rows(
    conn: &mut SqliteConnection,
) -> SelectionV2SchemaResult<Vec<ManagedSchemaCatalogRow>> {
    let rows = diesel::sql_query(
        "SELECT type AS object_type, name, tbl_name AS table_name, sql
         FROM sqlite_schema
         WHERE sql IS NOT NULL AND name LIKE 'selection_%'
         ORDER BY CASE type
             WHEN 'table' THEN 0
             WHEN 'index' THEN 1
             WHEN 'trigger' THEN 2
             ELSE 3
         END, CAST(name AS BLOB), CAST(tbl_name AS BLOB), CAST(sql AS BLOB)",
    )
    .load::<ManagedSchemaCatalogRow>(conn)?;
    Ok(rows)
}

fn managed_index_catalog_rows(
    conn: &mut SqliteConnection,
) -> SelectionV2SchemaResult<Vec<ManagedIndexCatalogRow>> {
    let mut rows = Vec::new();
    for table in V2_TABLES {
        let indexes = diesel::sql_query(format!(
            "SELECT name, \"unique\" AS unique_flag, origin, partial
             FROM pragma_index_list('{table}')
             ORDER BY CAST(name AS BLOB)"
        ))
        .load::<IndexListCatalogRow>(conn)?;
        for index in indexes {
            let escaped_index = index.name.replace('\'', "''");
            let columns = diesel::sql_query(format!(
                "SELECT seqno,cid,name,\"desc\" AS desc_flag,coll AS collation,\"key\" AS key_flag
                 FROM pragma_index_xinfo('{escaped_index}')
                 ORDER BY seqno"
            ))
            .load::<IndexXInfoCatalogRow>(conn)?;
            rows.push(ManagedIndexCatalogRow {
                table,
                index,
                columns,
            });
        }
    }
    rows.sort_by(|left, right| {
        left.table
            .as_bytes()
            .cmp(right.table.as_bytes())
            .then_with(|| left.index.name.as_bytes().cmp(right.index.name.as_bytes()))
    });
    Ok(rows)
}

fn verify_final_outcome_attempt_field_order(
    conn: &mut SqliteConnection,
) -> SelectionV2SchemaResult<()> {
    let actual = diesel::sql_query(
        "SELECT group_concat(name, ',') AS value
         FROM (
             SELECT name
             FROM pragma_table_xinfo('selection_outcome_attempts')
             ORDER BY cid
         )",
    )
    .get_result::<TextRow>(conn)?
    .value;
    let expected = "outcome_attempt_id,sample_key,phase,stored_due_date,outcome_run_id,request_hash,request_evidence_json,request_evidence_hash,transport_attempts_json,transport_attempts_hash,result_code,reason_code,retryable,provider,source,source_at,observed_at,batch_id,batch_content_hash,available_evidence_json,available_evidence_hash,error_detail_json,error_detail_hash,error_fingerprint,settled_outcome_content_hash,attempted_at,content_hash";
    if actual != expected {
        return Err(SelectionV2SchemaError::SchemaMismatch {
            table: "selection_outcome_attempts",
            column: "final_field_registry",
            expected,
            actual,
        });
    }
    Ok(())
}

fn verify_field_registry(conn: &mut SqliteConnection) -> SelectionV2SchemaResult<()> {
    for (table, expected) in V2_COLUMN_REGISTRY {
        let query = format!(
            "SELECT group_concat(name, ',') AS value
             FROM (SELECT name FROM pragma_table_xinfo('{table}') ORDER BY cid)"
        );
        let actual = diesel::sql_query(query).get_result::<TextRow>(conn)?.value;
        if actual != expected {
            return Err(SelectionV2SchemaError::SchemaMismatch {
                table,
                column: "field_registry",
                expected,
                actual,
            });
        }
    }
    Ok(())
}

/// PK/FK/UNIQUE/CHECK constraints are part of the frozen table DDL, not merely
/// column metadata. Comparing every managed table and explicit index against
/// the canonical statement rejects a same-column schema with weakened safety
/// constraints, a missing partial-unique index, or an extra affected index.
fn verify_canonical_table_and_index_registry(
    conn: &mut SqliteConnection,
) -> SelectionV2SchemaResult<()> {
    for table in V2_TABLES {
        let actual = table_sql(conn, table)?;
        let expected = canonical_table_statement(table)?;
        if normalize_schema_object_sql(&actual) != normalize_schema_object_sql(expected) {
            return Err(SelectionV2SchemaError::SchemaMismatch {
                table,
                column: "table_ddl_registry",
                expected: "exact canonical PK/FK/UNIQUE/CHECK table DDL",
                actual: "non-canonical table DDL".to_owned(),
            });
        }
    }

    let explicit_index_count = diesel::sql_query(
        "SELECT COUNT(*) AS count FROM sqlite_master
         WHERE type='index' AND sql IS NOT NULL
           AND tbl_name IN (
             'selection_source_batch_attempts','selection_source_facts_v2',
             'selection_source_fact_attempts','selection_relation_attempts',
             'selection_evaluation_attempts','selection_samples',
             'selection_rejections','selection_sample_outcomes',
             'selection_outcome_attempts','selection_v2_recovery_envelopes',
             'selection_v2_run_stages','selection_v2_commit_receipts'
           )",
    )
    .get_result::<CountRow>(conn)?;
    if explicit_index_count.count != V2_INDEXES.len() as i64 {
        return Err(SelectionV2SchemaError::SchemaMismatch {
            table: "selection_v2_table_set",
            column: "index_registry",
            expected: "exact five canonical explicit indexes",
            actual: format!("{} explicit affected indexes", explicit_index_count.count),
        });
    }
    for index in V2_INDEXES {
        let actual = schema_object_sql(conn, "index", index)?;
        let expected = canonical_index_statement(index)?;
        if normalize_schema_object_sql(&actual) != normalize_schema_object_sql(expected) {
            return Err(SelectionV2SchemaError::SchemaMismatch {
                table: "selection_v2_table_set",
                column: "index_registry",
                expected: "exact canonical index DDL",
                actual: format!("non-canonical index {index}"),
            });
        }
    }

    verify_no_unexpected_affected_objects(conn, 53)
}

fn canonical_table_statement(table: &'static str) -> SelectionV2SchemaResult<&'static str> {
    canonical_schema_statement(&format!("CREATE TABLE IF NOT EXISTS {table}")).ok_or_else(|| {
        SelectionV2SchemaError::SchemaMismatch {
            table,
            column: "table_ddl_registry",
            expected: "canonical table statement",
            actual: "missing from embedded schema".to_owned(),
        }
    })
}

fn canonical_index_statement(index: &'static str) -> SelectionV2SchemaResult<&'static str> {
    for marker in [
        format!("CREATE UNIQUE INDEX IF NOT EXISTS {index}"),
        format!("CREATE INDEX IF NOT EXISTS {index}"),
    ] {
        if let Some(statement) = canonical_schema_statement(&marker) {
            return Ok(statement);
        }
    }
    Err(SelectionV2SchemaError::SchemaMismatch {
        table: "selection_v2_table_set",
        column: "index_registry",
        expected: "canonical index statement",
        actual: format!("{index} missing from embedded schema"),
    })
}

fn canonical_schema_statement(marker: &str) -> Option<&'static str> {
    let start = SELECTION_V2_TRANSITIONAL_SCHEMA.find(marker)?;
    let remainder = &SELECTION_V2_TRANSITIONAL_SCHEMA[start..];
    let end = remainder.find(';')? + 1;
    Some(&remainder[..end])
}

fn schema_object_sql(
    conn: &mut SqliteConnection,
    object_type: &'static str,
    name: &'static str,
) -> SelectionV2SchemaResult<String> {
    diesel::sql_query(
        "SELECT sql FROM sqlite_master
         WHERE type=?1 AND name=?2 AND sql IS NOT NULL",
    )
    .bind::<Text, _>(object_type)
    .bind::<Text, _>(name)
    .get_result::<TriggerSqlRow>(conn)
    .map(|row| row.sql)
    .map_err(Into::into)
}

fn normalize_schema_object_sql(sql: &str) -> String {
    let mut tokens = tokenize_schema_sql(sql);
    while matches!(tokens.last(), Some(token) if token == ";") {
        tokens.pop();
    }
    let mut canonical = Vec::with_capacity(tokens.len());
    let mut index = 0;
    while index < tokens.len() {
        if index + 2 < tokens.len()
            && tokens[index] == "IF"
            && tokens[index + 1] == "NOT"
            && tokens[index + 2] == "EXISTS"
        {
            index += 3;
            continue;
        }
        canonical.push(tokens[index].as_str());
        index += 1;
    }
    canonical.join("\u{1f}")
}

/// Tokenize SQLite schema SQL without ever normalizing bytes inside a quoted
/// string or identifier. SQLite removes `IF NOT EXISTS` from `sqlite_master`
/// but otherwise the BR-180 comparison treats quoted bytes as schema data.
fn tokenize_schema_sql(sql: &str) -> Vec<String> {
    let chars = sql.chars().collect::<Vec<_>>();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index].is_whitespace() {
            index += 1;
            continue;
        }

        let start = index;
        match chars[index] {
            '\'' | '"' | '`' => {
                let delimiter = chars[index];
                index += 1;
                while index < chars.len() {
                    if chars[index] == delimiter {
                        index += 1;
                        if index < chars.len() && chars[index] == delimiter {
                            index += 1;
                            continue;
                        }
                        break;
                    }
                    index += 1;
                }
            }
            '[' => {
                index += 1;
                while index < chars.len() {
                    if chars[index] == ']' {
                        index += 1;
                        if index < chars.len() && chars[index] == ']' {
                            index += 1;
                            continue;
                        }
                        break;
                    }
                    index += 1;
                }
            }
            ch if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '$') => {
                index += 1;
                while index < chars.len()
                    && (chars[index].is_ascii_alphanumeric() || matches!(chars[index], '_' | '$'))
                {
                    index += 1;
                }
            }
            _ => index += 1,
        }
        tokens.push(chars[start..index].iter().collect());
    }
    tokens
}

fn verify_request_evidence_columns(conn: &mut SqliteConnection) -> SelectionV2SchemaResult<()> {
    for (table, request_column, expected_request_not_null, expected_evidence_not_null) in [
        ("selection_source_batch_attempts", "request_hash", 1, 1),
        ("selection_relation_attempts", "request_hash", 0, 0),
        ("selection_evaluation_attempts", "market_request_hash", 1, 1),
        ("selection_outcome_attempts", "request_hash", 0, 0),
    ] {
        verify_column_nullability(conn, table, request_column, expected_request_not_null)?;
        verify_column_nullability(
            conn,
            table,
            "request_evidence_json",
            expected_evidence_not_null,
        )?;
        verify_column_nullability(
            conn,
            table,
            "request_evidence_hash",
            expected_evidence_not_null,
        )?;
    }
    require_table_sql_tokens(
        conn,
        "selection_source_batch_attempts",
        "request evidence",
        &[
            "request_hashTEXTNOTNULL",
            "request_evidence_jsonTEXTNOTNULLCHECK(json_valid(request_evidence_json))",
            "request_evidence_hashTEXTNOTNULL",
        ],
    )?;
    require_table_sql_tokens(
        conn,
        "selection_evaluation_attempts",
        "request evidence",
        &[
            "market_request_hashTEXTNOTNULL",
            "request_evidence_jsonTEXTNOTNULLCHECK(json_valid(request_evidence_json))",
            "request_evidence_hashTEXTNOTNULL",
        ],
    )?;
    require_table_sql_tokens(
        conn,
        "selection_relation_attempts",
        "request evidence",
        &[
            "(request_hashISNULLANDrequest_evidence_jsonISNULLANDrequest_evidence_hashISNULL)",
            "(request_hashISNOTNULLANDrequest_evidence_jsonISNOTNULLANDrequest_evidence_hashISNOTNULLANDjson_valid(request_evidence_json))",
            "(relation_kind='direct_mention'ANDavailable_evidence_jsonISNULLANDavailable_evidence_hashISNULL)",
            "(relation_kind='provider_board_constituent'ANDavailable_evidence_jsonISNOTNULLANDavailable_evidence_hashISNOTNULL)",
            "json_extract(typed_binding_state_json,'$.state')='direct_not_applicable'ANDrequest_hashISNULLANDrequest_evidence_jsonISNULLANDrequest_evidence_hashISNULL",
            "json_extract(typed_binding_state_json,'$.state')='not_configured'ANDresult_codeIN('rejected','unsupported')",
            "request_hashISNOTNULLANDrequest_evidence_jsonISNOTNULLANDrequest_evidence_hashISNOTNULLANDjson_extract(typed_binding_state_json,'$.state')='verified'",
        ],
    )?;
    require_table_sql_tokens(
        conn,
        "selection_outcome_attempts",
        "request evidence",
        &[
            "(request_hashISNULLANDrequest_evidence_jsonISNULLANDrequest_evidence_hashISNULL)",
            "(request_hashISNOTNULLANDrequest_evidence_jsonISNOTNULLANDrequest_evidence_hashISNOTNULLANDjson_valid(request_evidence_json))",
            "(result_code='settled'ANDreason_codeISNULLANDretryableISNULLANDrequest_hashISNOTNULLANDrequest_evidence_jsonISNOTNULLANDrequest_evidence_hashISNOTNULL",
            "(result_code='expected_wait'ANDreason_code='market_session_unsettled'ANDrequest_hashISNULLANDrequest_evidence_jsonISNULLANDrequest_evidence_hashISNULL",
            "(result_code='error'ANDreason_codeISNOTNULLANDlength(reason_code)>0ANDretryableISNOTNULLANDrequest_hashISNOTNULLANDrequest_evidence_jsonISNOTNULLANDrequest_evidence_hashISNOTNULL",
        ],
    )?;
    Ok(())
}

fn require_table_sql_tokens(
    conn: &mut SqliteConnection,
    table: &'static str,
    column: &'static str,
    tokens: &[&str],
) -> SelectionV2SchemaResult<()> {
    let sql = compact_sql(&table_sql(conn, table)?);
    for token in tokens {
        if !sql.contains(token) {
            return Err(SelectionV2SchemaError::SchemaMismatch {
                table,
                column,
                expected: "exact transitional-current CHECK matrix",
                actual: format!("missing canonical DDL token {token}"),
            });
        }
    }
    Ok(())
}

fn verify_column_nullability(
    conn: &mut SqliteConnection,
    table: &'static str,
    column: &'static str,
    expected_not_null: i32,
) -> SelectionV2SchemaResult<()> {
    let query = format!(
        "SELECT [notnull] AS not_null
         FROM pragma_table_xinfo('{table}')
         WHERE name='{column}'"
    );
    let rows = diesel::sql_query(query).load::<ColumnNullabilityRow>(conn)?;
    let Some(actual) = rows.first() else {
        return Err(SelectionV2SchemaError::SchemaMismatch {
            table,
            column,
            expected: if expected_not_null == 0 {
                "nullable"
            } else {
                "NOT NULL"
            },
            actual: "missing".to_owned(),
        });
    };
    if actual.not_null != expected_not_null {
        return Err(SelectionV2SchemaError::SchemaMismatch {
            table,
            column,
            expected: if expected_not_null == 0 {
                "nullable"
            } else {
                "NOT NULL"
            },
            actual: if actual.not_null == 0 {
                "nullable".to_owned()
            } else {
                "NOT NULL".to_owned()
            },
        });
    }
    Ok(())
}

fn verify_recovery_payload_contract(conn: &mut SqliteConnection) -> SelectionV2SchemaResult<()> {
    let sql = compact_sql(&table_sql(conn, "selection_v2_recovery_envelopes")?);
    for token in [
        "COALESCE(json_type(payload_json,'$.domain')='text',0)",
        "payload_schema='config-activation-stage-v1'",
        "payload_schema='source-ingress-stage-v2'",
        "payload_schema='generation-stage-v3'",
        "payload_schema='outcome-stage-v2'",
        "json_extract(payload_json,'$.domain')='stock_analysis.br174.config_activation_stage.v1'",
        "json_extract(payload_json,'$.domain')='stock_analysis.br174.source_ingress_stage.v2'",
        "json_extract(payload_json,'$.domain')='stock_analysis.br174.generation_stage.v3'",
        "json_extract(payload_json,'$.domain')='stock_analysis.br174.outcome_stage.v2'",
    ] {
        if !sql.contains(token) {
            return Err(SelectionV2SchemaError::SchemaMismatch {
                table: "selection_v2_recovery_envelopes",
                column: "payload_schema/payload_json",
                expected: "exact transitional-current payload schema/domain matrix",
                actual: format!("missing canonical DDL token {token}"),
            });
        }
    }
    Ok(())
}

fn table_sql(conn: &mut SqliteConnection, table: &'static str) -> SelectionV2SchemaResult<String> {
    diesel::sql_query(
        "SELECT sql FROM sqlite_master
         WHERE type='table' AND name=?1",
    )
    .bind::<Text, _>(table)
    .get_result::<TriggerSqlRow>(conn)
    .map(|row| row.sql)
    .map_err(Into::into)
}

fn compact_sql(sql: &str) -> String {
    tokenize_schema_sql(sql).join("")
}

fn verify_integrity(conn: &mut SqliteConnection) -> SelectionV2SchemaResult<()> {
    let rows = diesel::sql_query("PRAGMA integrity_check").load::<IntegrityCheckRow>(conn)?;
    if rows.len() != 1 || rows[0].integrity_check != "ok" {
        return Err(SelectionV2SchemaError::SchemaMismatch {
            table: "selection_v2_table_set",
            column: "integrity_check",
            expected: "ok",
            actual: rows
                .iter()
                .map(|row| row.integrity_check.as_str())
                .collect::<Vec<_>>()
                .join(";"),
        });
    }
    Ok(())
}

fn verify_source_batch_record_count_contract(
    conn: &mut SqliteConnection,
) -> SelectionV2SchemaResult<()> {
    const TABLE: &str = "selection_source_batch_attempts";
    const COLUMN: &str = "record_count";
    const EXPECTED: &str = "nullable";

    let rows = diesel::sql_query(
        "SELECT [notnull] AS not_null
         FROM pragma_table_xinfo('selection_source_batch_attempts')
         WHERE name='record_count'",
    )
    .load::<ColumnNullabilityRow>(conn)?;
    let Some(column) = rows.first() else {
        return Err(SelectionV2SchemaError::SchemaMismatch {
            table: TABLE,
            column: COLUMN,
            expected: EXPECTED,
            actual: "missing".to_owned(),
        });
    };
    if column.not_null != 0 {
        return Err(SelectionV2SchemaError::SchemaMismatch {
            table: TABLE,
            column: COLUMN,
            expected: EXPECTED,
            actual: "NOT NULL".to_owned(),
        });
    }
    let sql = compact_sql(&table_sql(conn, TABLE)?);
    for token in [
        "record_countINTEGERCHECK(record_count>=0)",
        "(status_kind='available'ANDrecord_countISNOTNULLANDrecord_count>0",
        "(status_kind='verified_empty'ANDrecord_countISNOTNULLANDrecord_count=0",
        "(status_kind='unavailable'ANDrecord_countISNULL",
    ] {
        if !sql.contains(token) {
            return Err(SelectionV2SchemaError::SchemaMismatch {
                table: TABLE,
                column: COLUMN,
                expected: "exact status CHECK matrix",
                actual: format!("missing canonical DDL token {token}"),
            });
        }
    }
    Ok(())
}

fn verify_pragmas(conn: &mut SqliteConnection) -> SelectionV2SchemaResult<()> {
    let foreign_keys =
        diesel::sql_query("PRAGMA foreign_keys").get_result::<ForeignKeysPragma>(conn)?;
    if foreign_keys.foreign_keys != 1 {
        return Err(SelectionV2SchemaError::UnsafePragma {
            name: "foreign_keys",
            expected: 1,
            actual: foreign_keys.foreign_keys,
        });
    }

    let synchronous =
        diesel::sql_query("PRAGMA synchronous").get_result::<SynchronousPragma>(conn)?;
    if synchronous.synchronous != 2 {
        return Err(SelectionV2SchemaError::UnsafePragma {
            name: "synchronous",
            expected: 2,
            actual: synchronous.synchronous,
        });
    }
    Ok(())
}

fn verify_foreign_keys(conn: &mut SqliteConnection) -> SelectionV2SchemaResult<()> {
    let violations = diesel::sql_query("SELECT COUNT(*) AS count FROM pragma_foreign_key_check")
        .get_result::<CountRow>(conn)?;
    if violations.count != 0 {
        return Err(SelectionV2SchemaError::ForeignKeyViolation {
            violations: violations.count,
        });
    }
    Ok(())
}

const SELECTION_V2_TRANSITIONAL_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS selection_v2_recovery_envelopes (
    stage_run_id TEXT PRIMARY KEY NOT NULL,
    subject_kind TEXT NOT NULL CHECK (
        subject_kind IN ('config_activation','ingress_run','generation_run','outcome_run')
    ),
    logical_subject_key TEXT NOT NULL,
    payload_schema TEXT NOT NULL CHECK (
        (subject_kind='config_activation' AND payload_schema='config-activation-stage-v1')
        OR (subject_kind='ingress_run' AND payload_schema='source-ingress-stage-v2')
        OR (subject_kind='generation_run' AND payload_schema='generation-stage-v3')
        OR (subject_kind='outcome_run' AND payload_schema='outcome-stage-v2')
    ),
    payload_json TEXT NOT NULL CHECK (
        json_valid(payload_json)
        AND COALESCE(json_type(payload_json, '$.domain')='text', 0)
        AND (
            (subject_kind='config_activation'
                AND json_extract(payload_json, '$.domain')
                    ='stock_analysis.br174.config_activation_stage.v1')
            OR
            (subject_kind='ingress_run'
                AND json_extract(payload_json, '$.domain')
                    ='stock_analysis.br174.source_ingress_stage.v2')
            OR
            (subject_kind='generation_run'
                AND json_extract(payload_json, '$.domain')
                    ='stock_analysis.br174.generation_stage.v3')
            OR
            (subject_kind='outcome_run'
                AND json_extract(payload_json, '$.domain')
                    ='stock_analysis.br174.outcome_stage.v2')
        )
    ),
    payload_json_hash TEXT NOT NULL,
    in_memory_payload_hash TEXT NOT NULL,
    config_activation_run_id TEXT NOT NULL,
    config_hash TEXT NOT NULL,
    enveloped_at TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    CHECK (
        subject_kind<>'config_activation'
        OR config_activation_run_id=stage_run_id
    ),
    UNIQUE(stage_run_id, payload_json_hash, in_memory_payload_hash)
);

CREATE TABLE IF NOT EXISTS selection_source_batch_attempts (
    source_batch_attempt_id TEXT PRIMARY KEY NOT NULL,
    ingress_run_id TEXT NOT NULL,
    config_activation_run_id TEXT NOT NULL,
    config_hash TEXT NOT NULL,
    generation_market_date TEXT NOT NULL,
    registered_feed_identity TEXT NOT NULL,
    registered_feed_snapshot_hash TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    request_evidence_json TEXT NOT NULL CHECK (json_valid(request_evidence_json)),
    request_evidence_hash TEXT NOT NULL,
    feed_attempt_content_hash TEXT NOT NULL,
    status_kind TEXT NOT NULL CHECK (
        status_kind IN ('available','verified_empty','unavailable')
    ),
    record_count INTEGER CHECK (record_count >= 0),
    provider TEXT,
    source TEXT,
    source_at TEXT,
    observed_at TEXT,
    batch_id TEXT,
    batch_content_hash TEXT,
    failed_stage TEXT,
    reason_code TEXT,
    retryable INTEGER CHECK (retryable IN (0,1)),
    available_evidence_json TEXT,
    available_evidence_hash TEXT,
    error_detail_json TEXT,
    error_detail_hash TEXT,
    error_fingerprint TEXT,
    attempted_at TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    CHECK (
        (available_evidence_json IS NULL AND available_evidence_hash IS NULL)
        OR
        (available_evidence_json IS NOT NULL AND available_evidence_hash IS NOT NULL
            AND json_valid(available_evidence_json))
    ),
    CHECK (
        (error_detail_json IS NULL AND error_detail_hash IS NULL)
        OR
        (error_detail_json IS NOT NULL AND error_detail_hash IS NOT NULL
            AND json_valid(error_detail_json))
    ),
    CHECK (
        (status_kind='available' AND record_count IS NOT NULL AND record_count>0
            AND provider IS NOT NULL AND source IS NOT NULL
            AND source_at IS NOT NULL AND observed_at IS NOT NULL
            AND batch_id IS NOT NULL AND batch_content_hash IS NOT NULL
            AND available_evidence_json IS NOT NULL
            AND failed_stage IS NULL AND reason_code IS NULL AND retryable IS NULL
            AND error_detail_json IS NULL AND error_fingerprint IS NULL)
        OR
        (status_kind='verified_empty' AND record_count IS NOT NULL AND record_count=0
            AND provider IS NOT NULL AND source IS NOT NULL
            AND source_at IS NOT NULL AND observed_at IS NOT NULL
            AND batch_id IS NOT NULL AND batch_content_hash IS NOT NULL
            AND available_evidence_json IS NOT NULL
            AND failed_stage IS NULL AND reason_code IS NULL AND retryable IS NULL
            AND error_detail_json IS NULL AND error_fingerprint IS NULL)
        OR
        (status_kind='unavailable' AND record_count IS NULL
            AND batch_content_hash IS NULL
            AND failed_stage IS NOT NULL AND length(failed_stage)>0
            AND reason_code IS NOT NULL AND length(reason_code)>0
            AND retryable IS NOT NULL
            AND error_detail_json IS NOT NULL AND error_fingerprint IS NOT NULL
            AND (
                (available_evidence_json IS NULL
                    AND provider IS NULL AND source IS NULL AND source_at IS NULL
                    AND observed_at IS NULL AND batch_id IS NULL)
                OR
                (available_evidence_json IS NOT NULL
                    AND (provider IS NOT NULL OR source IS NOT NULL
                         OR source_at IS NOT NULL OR observed_at IS NOT NULL
                         OR batch_id IS NOT NULL))
            ))
    ),
    FOREIGN KEY(ingress_run_id)
        REFERENCES selection_v2_recovery_envelopes(stage_run_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    UNIQUE(ingress_run_id, registered_feed_identity)
);

CREATE TABLE IF NOT EXISTS selection_source_facts_v2 (
    source_fact_key TEXT PRIMARY KEY NOT NULL,
    event_id TEXT NOT NULL,
    payload_schema TEXT NOT NULL CHECK (payload_schema='global-news-source-fact-v2'),
    config_activation_run_id TEXT NOT NULL,
    config_hash TEXT NOT NULL,
    generation_market_date TEXT NOT NULL,
    provider_source TEXT NOT NULL,
    item_id TEXT NOT NULL,
    title TEXT NOT NULL,
    summary TEXT,
    content TEXT,
    publisher TEXT,
    canonical_url TEXT,
    published_at TEXT,
    instruments_json TEXT NOT NULL CHECK (json_valid(instruments_json)),
    topics_json TEXT NOT NULL CHECK (json_valid(topics_json)),
    language TEXT,
    record_provider TEXT NOT NULL,
    record_source TEXT NOT NULL,
    record_source_at TEXT,
    record_observed_at TEXT NOT NULL,
    record_batch_id TEXT NOT NULL,
    record_batch_content_hash TEXT NOT NULL,
    provider_content_hash TEXT NOT NULL,
    first_ingress_run_id TEXT NOT NULL,
    ingress_gate_version TEXT NOT NULL,
    ingress_gate_input_json TEXT NOT NULL CHECK (json_valid(ingress_gate_input_json)),
    ingress_gate_input_hash TEXT NOT NULL,
    ingress_decision TEXT NOT NULL CHECK (ingress_decision IN ('admitted','rejected')),
    ingress_reason_code TEXT,
    ingress_retryable INTEGER CHECK (ingress_retryable IN (0,1)),
    ingress_gate_receipt_json TEXT NOT NULL CHECK (json_valid(ingress_gate_receipt_json)),
    ingress_gate_receipt_hash TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    CHECK (
        (ingress_decision='admitted' AND ingress_reason_code IS NULL
            AND ingress_retryable IS NULL)
        OR
        (ingress_decision='rejected' AND length(ingress_reason_code)>0
            AND ingress_retryable=0)
    ),
    FOREIGN KEY(first_ingress_run_id)
        REFERENCES selection_v2_recovery_envelopes(stage_run_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED
);

CREATE TABLE IF NOT EXISTS selection_source_fact_attempts (
    source_fact_attempt_id TEXT PRIMARY KEY NOT NULL,
    ingress_run_id TEXT NOT NULL,
    source_batch_attempt_id TEXT NOT NULL,
    provider_ordinal INTEGER NOT NULL CHECK (provider_ordinal >= 0),
    source_fact_key TEXT NOT NULL,
    acquired_record_json TEXT NOT NULL CHECK (json_valid(acquired_record_json)),
    acquired_record_hash TEXT NOT NULL,
    batch_evidence_json TEXT NOT NULL CHECK (json_valid(batch_evidence_json)),
    batch_evidence_hash TEXT NOT NULL,
    event_projection_id TEXT NOT NULL,
    attempt_result TEXT NOT NULL CHECK (
        attempt_result IN ('inserted','exact_replay','conflict')
    ),
    conflict_hash TEXT,
    attempted_at TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    CHECK (
        (attempt_result IN ('inserted','exact_replay') AND conflict_hash IS NULL)
        OR (attempt_result='conflict' AND length(conflict_hash)>0)
    ),
    FOREIGN KEY(ingress_run_id)
        REFERENCES selection_v2_recovery_envelopes(stage_run_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY(source_batch_attempt_id)
        REFERENCES selection_source_batch_attempts(source_batch_attempt_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY(source_fact_key)
        REFERENCES selection_source_facts_v2(source_fact_key)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    UNIQUE(source_batch_attempt_id, provider_ordinal)
);

CREATE TABLE IF NOT EXISTS selection_relation_attempts (
    relation_attempt_id TEXT PRIMARY KEY NOT NULL,
    relation_key TEXT NOT NULL,
    generation_run_id TEXT NOT NULL,
    source_fact_key TEXT NOT NULL,
    event_id TEXT NOT NULL,
    chain_id TEXT NOT NULL,
    config_activation_run_id TEXT NOT NULL,
    config_hash TEXT NOT NULL,
    relation_schema_version TEXT NOT NULL CHECK (relation_schema_version='event-relation-v2'),
    relation_kind TEXT NOT NULL CHECK (
        relation_kind IN ('direct_mention','provider_board_constituent')
    ),
    relation_source_identity_json TEXT NOT NULL CHECK (json_valid(relation_source_identity_json)),
    relation_source_identity_hash TEXT NOT NULL,
    typed_binding_state_json TEXT NOT NULL CHECK (json_valid(typed_binding_state_json)),
    typed_binding_state_hash TEXT NOT NULL,
    request_hash TEXT,
    request_evidence_json TEXT,
    request_evidence_hash TEXT,
    result_code TEXT NOT NULL CHECK (result_code IN ('resolved','rejected','unsupported')),
    failed_stage TEXT,
    retryable INTEGER CHECK (retryable IN (0,1)),
    raw_identity_json TEXT,
    raw_identity_hash TEXT,
    canonical_stock_code TEXT,
    canonical_stock_name TEXT,
    canonical_market TEXT,
    artifact_content_hash TEXT,
    binding_audit_hash TEXT,
    provider_board_kind TEXT,
    provider_board_code TEXT,
    provider_board_name TEXT,
    provider_source TEXT,
    provider_source_at TEXT,
    provider_observed_at TEXT,
    provider_batch_id TEXT,
    provider_batch_content_hash TEXT,
    actual_constituent_count INTEGER CHECK (actual_constituent_count >= 0),
    available_evidence_json TEXT,
    available_evidence_hash TEXT,
    error_detail_json TEXT,
    error_detail_hash TEXT,
    error_fingerprint TEXT,
    attempted_at TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    CHECK (
        (request_hash IS NULL
            AND request_evidence_json IS NULL AND request_evidence_hash IS NULL)
        OR
        (request_hash IS NOT NULL
            AND request_evidence_json IS NOT NULL
            AND request_evidence_hash IS NOT NULL
            AND json_valid(request_evidence_json))
    ),
    CHECK (
        (raw_identity_json IS NULL AND raw_identity_hash IS NULL)
        OR
        (raw_identity_json IS NOT NULL AND raw_identity_hash IS NOT NULL
            AND json_valid(raw_identity_json))
    ),
    CHECK (
        (available_evidence_json IS NULL AND available_evidence_hash IS NULL)
        OR
        (available_evidence_json IS NOT NULL AND available_evidence_hash IS NOT NULL
            AND json_valid(available_evidence_json))
    ),
    CHECK (
        (error_detail_json IS NULL AND error_detail_hash IS NULL)
        OR
        (error_detail_json IS NOT NULL AND error_detail_hash IS NOT NULL
            AND json_valid(error_detail_json))
    ),
    CHECK (
        (result_code='resolved' AND failed_stage IS NULL AND retryable IS NULL
            AND canonical_stock_code IS NOT NULL
            AND canonical_stock_name IS NOT NULL AND canonical_market IS NOT NULL
            AND raw_identity_json IS NOT NULL
            AND (
                (relation_kind='direct_mention'
                    AND available_evidence_json IS NULL
                    AND available_evidence_hash IS NULL)
                OR
                (relation_kind='provider_board_constituent'
                    AND available_evidence_json IS NOT NULL
                    AND available_evidence_hash IS NOT NULL)
            )
            AND error_detail_json IS NULL AND error_fingerprint IS NULL)
        OR
        (result_code IN ('rejected','unsupported')
            AND failed_stage IS NOT NULL AND length(failed_stage)>0
            AND retryable IS NOT NULL
            AND error_detail_json IS NOT NULL AND error_fingerprint IS NOT NULL)
    ),
    CHECK (
        (relation_kind='direct_mention'
            AND json_extract(typed_binding_state_json, '$.state')='direct_not_applicable'
            AND request_hash IS NULL AND request_evidence_json IS NULL
            AND request_evidence_hash IS NULL
            AND artifact_content_hash IS NULL AND binding_audit_hash IS NULL
            AND provider_board_kind IS NULL AND provider_board_code IS NULL
            AND provider_board_name IS NULL AND actual_constituent_count IS NULL
            AND provider_source IS NULL AND provider_source_at IS NULL
            AND provider_observed_at IS NULL AND provider_batch_id IS NULL
            AND provider_batch_content_hash IS NULL
            AND available_evidence_json IS NULL AND available_evidence_hash IS NULL)
        OR
        (relation_kind='provider_board_constituent'
            AND (
                (artifact_content_hash IS NULL AND binding_audit_hash IS NULL
                    AND provider_board_kind IS NULL AND provider_board_code IS NULL
                    AND provider_board_name IS NULL
                    AND request_hash IS NULL AND request_evidence_json IS NULL
                    AND request_evidence_hash IS NULL
                    AND json_extract(typed_binding_state_json, '$.state')='not_configured'
                    AND result_code IN ('rejected','unsupported'))
                OR
                (artifact_content_hash IS NOT NULL AND binding_audit_hash IS NOT NULL
                    AND provider_board_kind IS NOT NULL AND provider_board_code IS NOT NULL
                    AND provider_board_name IS NOT NULL
                    AND request_hash IS NOT NULL AND request_evidence_json IS NOT NULL
                    AND request_evidence_hash IS NOT NULL
                    AND json_extract(typed_binding_state_json, '$.state')='verified')
            ))
    ),
    CHECK (
        relation_kind<>'provider_board_constituent' OR result_code<>'resolved'
        OR (
            provider_source IS NOT NULL AND provider_observed_at IS NOT NULL
            AND provider_batch_id IS NOT NULL AND provider_batch_content_hash IS NOT NULL
            AND actual_constituent_count>0
        )
    ),
    FOREIGN KEY(generation_run_id)
        REFERENCES selection_v2_recovery_envelopes(stage_run_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY(source_fact_key)
        REFERENCES selection_source_facts_v2(source_fact_key)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    UNIQUE(generation_run_id, relation_key)
);

CREATE TABLE IF NOT EXISTS selection_evaluation_attempts (
    evaluation_attempt_id TEXT PRIMARY KEY NOT NULL,
    sample_key TEXT NOT NULL,
    generation_run_id TEXT NOT NULL,
    source_fact_key TEXT NOT NULL,
    event_id TEXT NOT NULL,
    chain_id TEXT NOT NULL,
    canonical_stock_code TEXT NOT NULL,
    canonical_stock_name TEXT NOT NULL,
    canonical_market TEXT NOT NULL,
    relation_evidence_set_hash TEXT NOT NULL,
    market_request_hash TEXT NOT NULL,
    request_evidence_json TEXT NOT NULL CHECK (json_valid(request_evidence_json)),
    request_evidence_hash TEXT NOT NULL,
    result_code TEXT NOT NULL CHECK (result_code IN ('completed','error')),
    failed_stage TEXT,
    retryable INTEGER CHECK (retryable IN (0,1)),
    provider TEXT,
    source TEXT,
    source_at TEXT,
    observed_at TEXT,
    batch_id TEXT,
    batch_content_hash TEXT,
    available_evidence_json TEXT,
    available_evidence_hash TEXT,
    terminal_decision_hash TEXT,
    error_detail_json TEXT,
    error_detail_hash TEXT,
    error_fingerprint TEXT,
    attempted_at TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    CHECK (
        (available_evidence_json IS NULL AND available_evidence_hash IS NULL)
        OR
        (available_evidence_json IS NOT NULL AND available_evidence_hash IS NOT NULL
            AND json_valid(available_evidence_json))
    ),
    CHECK (
        (error_detail_json IS NULL AND error_detail_hash IS NULL)
        OR
        (error_detail_json IS NOT NULL AND error_detail_hash IS NOT NULL
            AND json_valid(error_detail_json))
    ),
    CHECK (
        (result_code='completed' AND failed_stage IS NULL AND retryable IS NULL
            AND provider IS NOT NULL AND source IS NOT NULL
            AND observed_at IS NOT NULL AND batch_id IS NOT NULL
            AND batch_content_hash IS NOT NULL AND available_evidence_json IS NOT NULL
            AND terminal_decision_hash IS NOT NULL
            AND error_detail_json IS NULL AND error_fingerprint IS NULL)
        OR
        (result_code='error' AND length(failed_stage)>0 AND retryable IS NOT NULL
            AND terminal_decision_hash IS NULL
            AND error_detail_json IS NOT NULL AND error_fingerprint IS NOT NULL)
    ),
    FOREIGN KEY(generation_run_id)
        REFERENCES selection_v2_recovery_envelopes(stage_run_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY(source_fact_key)
        REFERENCES selection_source_facts_v2(source_fact_key)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    UNIQUE(generation_run_id, sample_key)
);

CREATE TABLE IF NOT EXISTS selection_samples (
    sample_key TEXT PRIMARY KEY NOT NULL,
    generation_run_id TEXT NOT NULL,
    source_fact_key TEXT NOT NULL,
    source_fact_content_hash TEXT NOT NULL,
    source_fact_attempt_id TEXT NOT NULL,
    source_batch_attempt_id TEXT NOT NULL,
    event_id TEXT NOT NULL,
    chain_id TEXT NOT NULL,
    config_activation_run_id TEXT NOT NULL,
    config_hash TEXT NOT NULL,
    matched_keyword TEXT NOT NULL,
    canonical_stock_code TEXT NOT NULL,
    canonical_stock_name TEXT NOT NULL,
    canonical_market TEXT NOT NULL,
    relation_schema_version TEXT NOT NULL,
    relation_evidence_json TEXT NOT NULL CHECK (json_valid(relation_evidence_json)),
    relation_evidence_set_hash TEXT NOT NULL,
    feature_version TEXT NOT NULL,
    t0_feature_json TEXT NOT NULL CHECK (json_valid(t0_feature_json)),
    t0_feature_hash TEXT NOT NULL,
    market_provider TEXT NOT NULL,
    market_source TEXT NOT NULL,
    market_source_at TEXT,
    market_observed_at TEXT NOT NULL,
    market_batch_id TEXT NOT NULL,
    market_batch_content_hash TEXT NOT NULL,
    admission_version TEXT NOT NULL,
    decision_kind TEXT NOT NULL CHECK (decision_kind IN ('admitted','hard_rejected')),
    rejection_count INTEGER NOT NULL CHECK (
        (decision_kind='admitted' AND rejection_count=0)
        OR (decision_kind='hard_rejected' AND rejection_count>0)
    ),
    rejection_row_hashes_in_ordinal_order TEXT NOT NULL CHECK (
        json_valid(rejection_row_hashes_in_ordinal_order)
        AND json_type(rejection_row_hashes_in_ordinal_order)='array'
    ),
    evaluation_market_date TEXT NOT NULL,
    t0_due_date TEXT NOT NULL,
    d1_due_date TEXT NOT NULL,
    d2_due_date TEXT NOT NULL,
    d3_due_date TEXT NOT NULL,
    d4_due_date TEXT NOT NULL,
    d5_due_date TEXT NOT NULL,
    calendar_version TEXT NOT NULL,
    calendar_hash TEXT NOT NULL,
    trading_date_vector_json TEXT NOT NULL CHECK (
        json_valid(trading_date_vector_json)
        AND json_type(trading_date_vector_json)='object'
        AND json_extract(trading_date_vector_json, '$.domain')
            ='stock_analysis.br178.outcome_trading_dates.v1'
        AND json_extract(trading_date_vector_json, '$.t0')=t0_due_date
        AND json_extract(trading_date_vector_json, '$.d1')=d1_due_date
        AND json_extract(trading_date_vector_json, '$.d2')=d2_due_date
        AND json_extract(trading_date_vector_json, '$.d3')=d3_due_date
        AND json_extract(trading_date_vector_json, '$.d4')=d4_due_date
        AND json_extract(trading_date_vector_json, '$.d5')=d5_due_date
    ),
    trading_date_vector_hash TEXT NOT NULL CHECK (
        length(trading_date_vector_hash)=64
        AND trading_date_vector_hash NOT GLOB '*[^0-9a-f]*'
    ),
    staged_at TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    CHECK (
        evaluation_market_date=t0_due_date
        AND t0_due_date<d1_due_date
        AND d1_due_date<d2_due_date
        AND d2_due_date<d3_due_date
        AND d3_due_date<d4_due_date
        AND d4_due_date<d5_due_date
    ),
    CHECK (
        (decision_kind='admitted'
            AND json_array_length(rejection_row_hashes_in_ordinal_order)=0)
        OR
        (decision_kind='hard_rejected'
            AND json_array_length(rejection_row_hashes_in_ordinal_order)=rejection_count)
    ),
    FOREIGN KEY(generation_run_id)
        REFERENCES selection_v2_recovery_envelopes(stage_run_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY(source_fact_key)
        REFERENCES selection_source_facts_v2(source_fact_key)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY(source_fact_attempt_id)
        REFERENCES selection_source_fact_attempts(source_fact_attempt_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY(source_batch_attempt_id)
        REFERENCES selection_source_batch_attempts(source_batch_attempt_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    UNIQUE(source_fact_key, chain_id, canonical_stock_code, config_hash)
);

CREATE TABLE IF NOT EXISTS selection_rejections (
    sample_key TEXT NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    generation_run_id TEXT NOT NULL,
    reason_code TEXT NOT NULL CHECK (
        reason_code IN (
            'moving_average_nonpositive','trend_alignment_failed','price_below_ma5',
            'price_ma20_distance_out_of_range','five_day_return_out_of_range',
            'settled_volume_confirmation_failed','intraday_volume_confirmation_failed'
        )
    ),
    rule_id TEXT NOT NULL,
    retryable INTEGER NOT NULL CHECK (retryable=0),
    structured_detail_json TEXT NOT NULL CHECK (json_valid(structured_detail_json)),
    structured_detail_hash TEXT NOT NULL,
    provider TEXT,
    source TEXT,
    source_at TEXT,
    observed_at TEXT,
    batch_id TEXT,
    batch_content_hash TEXT,
    created_at TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    CHECK (json_extract(structured_detail_json, '$.kind')=reason_code),
    PRIMARY KEY(sample_key, ordinal),
    FOREIGN KEY(sample_key) REFERENCES selection_samples(sample_key)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY(generation_run_id)
        REFERENCES selection_v2_recovery_envelopes(stage_run_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED
);

CREATE TABLE IF NOT EXISTS selection_sample_outcomes (
    sample_key TEXT NOT NULL,
    phase TEXT NOT NULL CHECK (
        phase IN ('t0_close','d1_settled','d3_settled','d5_settled')
    ),
    outcome_run_id TEXT NOT NULL,
    due_trading_date TEXT NOT NULL,
    open TEXT NOT NULL CHECK (CAST(open AS REAL)>0),
    high TEXT NOT NULL CHECK (CAST(high AS REAL)>0),
    low TEXT NOT NULL CHECK (CAST(low AS REAL)>0),
    close TEXT NOT NULL CHECK (CAST(close AS REAL)>0),
    volume TEXT NOT NULL CHECK (CAST(volume AS REAL)>0),
    amount TEXT NOT NULL CHECK (CAST(amount AS REAL)>=0),
    return_from_t0_close TEXT NOT NULL,
    cumulative_mfe TEXT NOT NULL,
    cumulative_mae TEXT NOT NULL,
    volume_ratio TEXT NOT NULL CHECK (CAST(volume_ratio AS REAL)>0),
    provider TEXT NOT NULL,
    source TEXT NOT NULL,
    source_at TEXT,
    observed_at TEXT NOT NULL,
    batch_id TEXT NOT NULL,
    batch_content_hash TEXT NOT NULL,
    created_at TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    CHECK (CAST(high AS REAL)>=CAST(open AS REAL)),
    CHECK (CAST(high AS REAL)>=CAST(close AS REAL)),
    CHECK (CAST(low AS REAL)<=CAST(open AS REAL)),
    CHECK (CAST(low AS REAL)<=CAST(close AS REAL)),
    CHECK (
        phase<>'t0_close'
        OR (
            CAST(return_from_t0_close AS REAL)=0
            AND CAST(cumulative_mfe AS REAL)=0
            AND CAST(cumulative_mae AS REAL)=0
            AND CAST(volume_ratio AS REAL)=1
        )
    ),
    PRIMARY KEY(sample_key, phase),
    FOREIGN KEY(sample_key) REFERENCES selection_samples(sample_key)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY(outcome_run_id)
        REFERENCES selection_v2_recovery_envelopes(stage_run_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED
);

CREATE TABLE IF NOT EXISTS selection_outcome_attempts (
    outcome_attempt_id TEXT PRIMARY KEY NOT NULL,
    sample_key TEXT NOT NULL,
    phase TEXT NOT NULL CHECK (
        phase IN ('t0_close','d1_settled','d3_settled','d5_settled')
    ),
    stored_due_date TEXT NOT NULL,
    outcome_run_id TEXT NOT NULL,
    request_hash TEXT,
    request_evidence_json TEXT,
    request_evidence_hash TEXT,
    result_code TEXT NOT NULL CHECK (result_code IN ('settled','expected_wait','error')),
    reason_code TEXT,
    retryable INTEGER CHECK (retryable IN (0,1)),
    provider TEXT,
    source TEXT,
    source_at TEXT,
    observed_at TEXT,
    batch_id TEXT,
    batch_content_hash TEXT,
    available_evidence_json TEXT,
    available_evidence_hash TEXT,
    error_detail_json TEXT,
    error_detail_hash TEXT,
    error_fingerprint TEXT,
    settled_outcome_content_hash TEXT,
    attempted_at TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    CHECK (
        (request_hash IS NULL
            AND request_evidence_json IS NULL AND request_evidence_hash IS NULL)
        OR
        (request_hash IS NOT NULL
            AND request_evidence_json IS NOT NULL
            AND request_evidence_hash IS NOT NULL
            AND json_valid(request_evidence_json))
    ),
    CHECK (
        (available_evidence_json IS NULL AND available_evidence_hash IS NULL)
        OR
        (available_evidence_json IS NOT NULL AND available_evidence_hash IS NOT NULL
            AND json_valid(available_evidence_json))
    ),
    CHECK (
        (error_detail_json IS NULL AND error_detail_hash IS NULL)
        OR
        (error_detail_json IS NOT NULL AND error_detail_hash IS NOT NULL
            AND json_valid(error_detail_json))
    ),
    CHECK (
        (result_code='settled' AND reason_code IS NULL AND retryable IS NULL
            AND request_hash IS NOT NULL AND request_evidence_json IS NOT NULL
            AND request_evidence_hash IS NOT NULL
            AND provider IS NOT NULL AND source IS NOT NULL
            AND observed_at IS NOT NULL AND batch_id IS NOT NULL
            AND batch_content_hash IS NOT NULL
            AND available_evidence_json IS NOT NULL
            AND error_detail_json IS NULL AND error_fingerprint IS NULL
            AND settled_outcome_content_hash IS NOT NULL)
        OR
        (result_code='expected_wait' AND reason_code='market_session_unsettled'
            AND request_hash IS NULL AND request_evidence_json IS NULL
            AND request_evidence_hash IS NULL
            AND retryable IS NULL AND provider IS NULL
            AND source IS NULL AND source_at IS NULL AND observed_at IS NULL
            AND batch_id IS NULL AND batch_content_hash IS NULL
            AND available_evidence_json IS NULL
            AND error_detail_json IS NULL AND error_fingerprint IS NULL
            AND settled_outcome_content_hash IS NULL)
        OR
        (result_code='error' AND reason_code IS NOT NULL
            AND length(reason_code)>0 AND retryable IS NOT NULL
            AND request_hash IS NOT NULL AND request_evidence_json IS NOT NULL
            AND request_evidence_hash IS NOT NULL
            AND error_detail_json IS NOT NULL AND error_fingerprint IS NOT NULL
            AND settled_outcome_content_hash IS NULL)
    ),
    FOREIGN KEY(sample_key) REFERENCES selection_samples(sample_key)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY(outcome_run_id)
        REFERENCES selection_v2_recovery_envelopes(stage_run_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    UNIQUE(outcome_run_id, sample_key, phase)
);

CREATE TABLE IF NOT EXISTS selection_v2_run_stages (
    subject_kind TEXT NOT NULL CHECK (
        subject_kind IN ('config_activation','ingress_run','generation_run','outcome_run')
    ),
    subject_id TEXT PRIMARY KEY NOT NULL,
    in_memory_payload_hash TEXT NOT NULL,
    prepared_record_hash TEXT NOT NULL,
    expected_staged_row_count INTEGER NOT NULL CHECK (expected_staged_row_count >= 1),
    staged_db_content_hash TEXT NOT NULL,
    recovery_envelope_content_hash TEXT NOT NULL,
    logical_subject_key TEXT NOT NULL,
    run_status TEXT NOT NULL,
    source_fact_key TEXT,
    config_activation_run_id TEXT NOT NULL,
    config_hash TEXT NOT NULL,
    config_snapshot_json_hash TEXT,
    config_activation_content_hash TEXT,
    config_activation_file_content_hash TEXT,
    config_effective_from TEXT,
    artifact_valid_from TEXT,
    artifact_expires_at TEXT,
    executable_revision TEXT,
    legacy_cutover_snapshot_hash TEXT,
    generation_market_date TEXT,
    aggregator_observed_at TEXT,
    ingress_source_batch_content_hash TEXT,
    outcome_phase TEXT,
    stored_due_date TEXT,
    staged_at TEXT NOT NULL,
    manifest_content_hash TEXT NOT NULL,
    CHECK (
        (subject_kind='config_activation' AND run_status='activated')
        OR
        (subject_kind='ingress_run'
            AND run_status IN ('completed','failed_non_retryable'))
        OR
        (subject_kind='generation_run'
            AND run_status IN (
                'completed','verified_no_relation','pending_dependency',
                'failed_non_retryable'
            ))
        OR
        (subject_kind='outcome_run'
            AND run_status IN (
                'settled','expected_wait','failed_retryable','failed_non_retryable'
            ))
    ),
    CHECK (
        (subject_kind='config_activation'
            AND config_activation_run_id=subject_id
            AND source_fact_key IS NULL
            AND config_snapshot_json_hash IS NOT NULL
            AND config_activation_content_hash IS NOT NULL
            AND config_activation_file_content_hash IS NOT NULL
            AND config_effective_from IS NOT NULL
            AND artifact_valid_from IS NOT NULL AND artifact_expires_at IS NOT NULL
            AND executable_revision IS NOT NULL
            AND legacy_cutover_snapshot_hash IS NOT NULL
            AND generation_market_date IS NULL
            AND aggregator_observed_at IS NULL
            AND ingress_source_batch_content_hash IS NULL
            AND outcome_phase IS NULL AND stored_due_date IS NULL)
        OR
        (subject_kind='ingress_run'
            AND source_fact_key IS NULL
            AND config_snapshot_json_hash IS NULL
            AND config_activation_content_hash IS NULL
            AND config_activation_file_content_hash IS NULL
            AND config_effective_from IS NULL
            AND artifact_valid_from IS NULL AND artifact_expires_at IS NULL
            AND executable_revision IS NULL AND legacy_cutover_snapshot_hash IS NULL
            AND generation_market_date IS NOT NULL
            AND aggregator_observed_at IS NOT NULL
            AND ingress_source_batch_content_hash IS NOT NULL
            AND outcome_phase IS NULL AND stored_due_date IS NULL)
        OR
        (subject_kind='generation_run'
            AND source_fact_key IS NOT NULL
            AND config_snapshot_json_hash IS NULL
            AND config_activation_content_hash IS NULL
            AND config_activation_file_content_hash IS NULL
            AND config_effective_from IS NULL
            AND artifact_valid_from IS NULL AND artifact_expires_at IS NULL
            AND executable_revision IS NULL AND legacy_cutover_snapshot_hash IS NULL
            AND generation_market_date IS NOT NULL
            AND aggregator_observed_at IS NULL
            AND ingress_source_batch_content_hash IS NULL
            AND outcome_phase IS NULL AND stored_due_date IS NULL)
        OR
        (subject_kind='outcome_run'
            AND source_fact_key IS NULL
            AND config_snapshot_json_hash IS NULL
            AND config_activation_content_hash IS NULL
            AND config_activation_file_content_hash IS NULL
            AND config_effective_from IS NULL
            AND artifact_valid_from IS NULL AND artifact_expires_at IS NULL
            AND executable_revision IS NULL AND legacy_cutover_snapshot_hash IS NULL
            AND generation_market_date IS NULL
            AND aggregator_observed_at IS NULL
            AND ingress_source_batch_content_hash IS NULL
            AND outcome_phase IN ('t0_close','d1_settled','d3_settled','d5_settled')
            AND stored_due_date IS NOT NULL)
    ),
    FOREIGN KEY(subject_id)
        REFERENCES selection_v2_recovery_envelopes(stage_run_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    UNIQUE(subject_kind, subject_id)
);

CREATE UNIQUE INDEX IF NOT EXISTS selection_v2_one_activation_per_config
ON selection_v2_run_stages(config_hash)
WHERE subject_kind='config_activation' AND run_status='activated';

CREATE TABLE IF NOT EXISTS selection_v2_commit_receipts (
    subject_kind TEXT NOT NULL,
    subject_id TEXT NOT NULL,
    logical_subject_key TEXT NOT NULL,
    in_memory_payload_hash TEXT NOT NULL,
    recovery_envelope_content_hash TEXT NOT NULL,
    prepared_audit_hash TEXT NOT NULL,
    run_manifest_content_hash TEXT NOT NULL,
    staged_db_content_hash TEXT NOT NULL,
    committed_audit_hash TEXT NOT NULL,
    committed_at TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    PRIMARY KEY(subject_kind, subject_id),
    FOREIGN KEY(subject_kind, subject_id)
        REFERENCES selection_v2_run_stages(subject_kind, subject_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY(subject_id)
        REFERENCES selection_v2_recovery_envelopes(stage_run_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED
);

CREATE INDEX IF NOT EXISTS selection_v2_source_facts_pending
ON selection_source_facts_v2(ingress_decision, first_ingress_run_id, source_fact_key);
CREATE INDEX IF NOT EXISTS selection_v2_samples_generation
ON selection_samples(generation_run_id, sample_key);
CREATE INDEX IF NOT EXISTS selection_v2_outcome_attempt_run
ON selection_outcome_attempts(outcome_run_id, sample_key, phase);
CREATE INDEX IF NOT EXISTS selection_v2_receipt_subject
ON selection_v2_commit_receipts(subject_id, subject_kind);

CREATE TRIGGER IF NOT EXISTS selection_v2_batch_lineage
BEFORE INSERT ON selection_source_batch_attempts
BEGIN
    SELECT RAISE(ABORT, 'BR-174 batch/envelope config lineage mismatch')
    WHERE NOT EXISTS (
        SELECT 1 FROM selection_v2_recovery_envelopes e
        WHERE e.stage_run_id=NEW.ingress_run_id
          AND e.subject_kind='ingress_run'
          AND e.config_activation_run_id=NEW.config_activation_run_id
          AND e.config_hash=NEW.config_hash
    );
END;

CREATE TRIGGER IF NOT EXISTS selection_v2_fact_lineage
BEFORE INSERT ON selection_source_facts_v2
BEGIN
    SELECT RAISE(ABORT, 'BR-174 fact/envelope config lineage mismatch')
    WHERE NOT EXISTS (
        SELECT 1 FROM selection_v2_recovery_envelopes e
        WHERE e.stage_run_id=NEW.first_ingress_run_id
          AND e.subject_kind='ingress_run'
          AND e.config_activation_run_id=NEW.config_activation_run_id
          AND e.config_hash=NEW.config_hash
    );
END;

CREATE TRIGGER IF NOT EXISTS selection_v2_fact_attempt_lineage
BEFORE INSERT ON selection_source_fact_attempts
BEGIN
    SELECT RAISE(ABORT, 'BR-174 fact attempt lineage mismatch')
    WHERE NOT EXISTS (
        SELECT 1
        FROM selection_source_batch_attempts b
        JOIN selection_source_facts_v2 f
          ON f.source_fact_key=NEW.source_fact_key
        WHERE b.source_batch_attempt_id=NEW.source_batch_attempt_id
          AND b.ingress_run_id=NEW.ingress_run_id
          AND b.batch_content_hash=f.record_batch_content_hash
          AND b.provider=f.record_provider
    );
END;

CREATE TRIGGER IF NOT EXISTS selection_v2_relation_requires_admitted_source
BEFORE INSERT ON selection_relation_attempts
BEGIN
    SELECT RAISE(ABORT, 'BR-174 generation requires matching ingress-admitted source fact')
    WHERE NOT EXISTS (
        SELECT 1
        FROM selection_source_facts_v2 f
        JOIN selection_v2_recovery_envelopes e
          ON e.stage_run_id=NEW.generation_run_id
        WHERE f.source_fact_key=NEW.source_fact_key
          AND f.ingress_decision='admitted'
          AND f.event_id=NEW.event_id
          AND f.config_activation_run_id=NEW.config_activation_run_id
          AND f.config_hash=NEW.config_hash
          AND e.subject_kind='generation_run'
          AND e.config_activation_run_id=f.config_activation_run_id
          AND e.config_hash=f.config_hash
    );
END;

CREATE TRIGGER IF NOT EXISTS selection_v2_evaluation_requires_admitted_source
BEFORE INSERT ON selection_evaluation_attempts
BEGIN
    SELECT RAISE(ABORT, 'BR-174 evaluation/source lineage mismatch')
    WHERE NOT EXISTS (
        SELECT 1
        FROM selection_source_facts_v2 f
        JOIN selection_v2_recovery_envelopes e
          ON e.stage_run_id=NEW.generation_run_id
        WHERE f.source_fact_key=NEW.source_fact_key
          AND f.ingress_decision='admitted'
          AND f.event_id=NEW.event_id
          AND e.subject_kind='generation_run'
          AND e.config_activation_run_id=f.config_activation_run_id
          AND e.config_hash=f.config_hash
    );
END;

CREATE TRIGGER IF NOT EXISTS selection_v2_sample_requires_admitted_source
BEFORE INSERT ON selection_samples
BEGIN
    SELECT RAISE(ABORT, 'BR-174 terminal sample lineage mismatch')
    WHERE NOT EXISTS (
        SELECT 1
        FROM selection_source_facts_v2 f
        JOIN selection_source_fact_attempts fa
          ON fa.source_fact_attempt_id=NEW.source_fact_attempt_id
        JOIN selection_source_batch_attempts b
          ON b.source_batch_attempt_id=NEW.source_batch_attempt_id
        JOIN selection_evaluation_attempts ev
          ON ev.generation_run_id=NEW.generation_run_id
         AND ev.sample_key=NEW.sample_key
        JOIN selection_v2_recovery_envelopes e
          ON e.stage_run_id=NEW.generation_run_id
        WHERE f.source_fact_key=NEW.source_fact_key
          AND f.ingress_decision='admitted'
          AND f.content_hash=NEW.source_fact_content_hash
          AND fa.source_fact_key=f.source_fact_key
          AND fa.ingress_run_id=f.first_ingress_run_id
          AND fa.source_batch_attempt_id=b.source_batch_attempt_id
          AND b.ingress_run_id=f.first_ingress_run_id
          AND f.event_id=NEW.event_id
          AND f.config_activation_run_id=NEW.config_activation_run_id
          AND f.config_hash=NEW.config_hash
          AND ev.source_fact_key=f.source_fact_key
          AND ev.event_id=NEW.event_id
          AND ev.chain_id=NEW.chain_id
          AND ev.canonical_stock_code=NEW.canonical_stock_code
          AND ev.relation_evidence_set_hash=NEW.relation_evidence_set_hash
          AND ev.result_code='completed'
          AND ev.terminal_decision_hash=NEW.content_hash
          AND e.config_activation_run_id=NEW.config_activation_run_id
          AND e.config_hash=NEW.config_hash
    );
END;

CREATE TRIGGER IF NOT EXISTS selection_v2_rejection_requires_admitted_source
BEFORE INSERT ON selection_rejections
BEGIN
    SELECT RAISE(ABORT, 'BR-174 rejection parent/matrix mismatch')
    WHERE NOT EXISTS (
        SELECT 1
        FROM selection_samples s
        JOIN selection_source_facts_v2 f ON f.source_fact_key=s.source_fact_key
        WHERE s.sample_key=NEW.sample_key
          AND s.generation_run_id=NEW.generation_run_id
          AND s.decision_kind='hard_rejected'
          AND NEW.ordinal < s.rejection_count
          AND NEW.ordinal=(
              SELECT COUNT(*) FROM selection_rejections r
              WHERE r.sample_key=NEW.sample_key
          )
          AND json_extract(
              s.rejection_row_hashes_in_ordinal_order,
              '$[' || NEW.ordinal || ']'
          )=NEW.content_hash
          AND f.ingress_decision='admitted'
    );
END;

CREATE TRIGGER IF NOT EXISTS selection_v2_manifest_envelope_binding
BEFORE INSERT ON selection_v2_run_stages
BEGIN
    SELECT RAISE(ABORT, 'BR-174 manifest/envelope binding mismatch')
    WHERE NOT EXISTS (
        SELECT 1 FROM selection_v2_recovery_envelopes e
        WHERE e.stage_run_id=NEW.subject_id
          AND e.subject_kind=NEW.subject_kind
          AND e.logical_subject_key=NEW.logical_subject_key
          AND e.in_memory_payload_hash=NEW.in_memory_payload_hash
          AND e.config_activation_run_id=NEW.config_activation_run_id
          AND e.config_hash=NEW.config_hash
          AND e.content_hash=NEW.recovery_envelope_content_hash
    );
END;

CREATE TRIGGER IF NOT EXISTS selection_v2_config_manifest_closure
BEFORE INSERT ON selection_v2_run_stages
WHEN NEW.subject_kind='config_activation'
BEGIN
    SELECT RAISE(ABORT, 'BR-174 config activation manifest closure mismatch')
    WHERE NEW.expected_staged_row_count<>1
       OR NEW.config_activation_run_id<>NEW.subject_id;
END;

CREATE TRIGGER IF NOT EXISTS selection_v2_ingress_manifest_closure
BEFORE INSERT ON selection_v2_run_stages
WHEN NEW.subject_kind='ingress_run'
BEGIN
    SELECT RAISE(ABORT, 'BR-174 ingress requires receipted config activation')
    WHERE NOT EXISTS (
        SELECT 1 FROM selection_v2_commit_receipts r
        WHERE r.subject_kind='config_activation'
          AND r.subject_id=NEW.config_activation_run_id
    );
    SELECT RAISE(ABORT, 'BR-174 ingress domain config mismatch')
    WHERE EXISTS (
        SELECT 1 FROM selection_source_batch_attempts b
        WHERE b.ingress_run_id=NEW.subject_id
          AND (
              b.config_activation_run_id<>NEW.config_activation_run_id
              OR b.config_hash<>NEW.config_hash
              OR b.generation_market_date<>NEW.generation_market_date
          )
    ) OR EXISTS (
        SELECT 1 FROM selection_source_facts_v2 f
        WHERE f.first_ingress_run_id=NEW.subject_id
          AND (
              f.config_activation_run_id<>NEW.config_activation_run_id
              OR f.config_hash<>NEW.config_hash
              OR f.generation_market_date<>NEW.generation_market_date
          )
    );
    SELECT RAISE(ABORT, 'BR-174 ingress feed/fact no-loss matrix mismatch')
    WHERE (SELECT COUNT(*) FROM selection_source_batch_attempts b
           WHERE b.ingress_run_id=NEW.subject_id)<>4
       OR (SELECT COUNT(DISTINCT registered_feed_snapshot_hash)
           FROM selection_source_batch_attempts b
           WHERE b.ingress_run_id=NEW.subject_id)<>1
       OR EXISTS (
        SELECT 1
        FROM selection_source_batch_attempts b
        WHERE b.ingress_run_id=NEW.subject_id
          AND (
            (b.status_kind='available' AND b.record_count<>(
                SELECT COUNT(*) FROM selection_source_fact_attempts fa
                WHERE fa.source_batch_attempt_id=b.source_batch_attempt_id
            ))
            OR (b.status_kind='available' AND (
                (SELECT MIN(provider_ordinal) FROM selection_source_fact_attempts fa
                 WHERE fa.source_batch_attempt_id=b.source_batch_attempt_id)<>0
                OR (SELECT MAX(provider_ordinal) FROM selection_source_fact_attempts fa
                    WHERE fa.source_batch_attempt_id=b.source_batch_attempt_id)
                   <>b.record_count-1
                OR EXISTS (
                    SELECT 1 FROM selection_source_fact_attempts fa
                    WHERE fa.source_batch_attempt_id=b.source_batch_attempt_id
                      AND fa.batch_evidence_hash<>b.available_evidence_hash
                )
            ))
            OR (b.status_kind<>'available' AND EXISTS (
                SELECT 1 FROM selection_source_fact_attempts fa
                WHERE fa.source_batch_attempt_id=b.source_batch_attempt_id
            ))
          )
    ) OR EXISTS (
        SELECT 1 FROM selection_source_fact_attempts fa
        LEFT JOIN selection_source_batch_attempts b
          ON b.source_batch_attempt_id=fa.source_batch_attempt_id
         AND b.ingress_run_id=fa.ingress_run_id
        LEFT JOIN selection_source_facts_v2 f
          ON f.source_fact_key=fa.source_fact_key
        WHERE fa.ingress_run_id=NEW.subject_id
          AND (b.source_batch_attempt_id IS NULL OR f.source_fact_key IS NULL)
    ) OR EXISTS (
        SELECT 1 FROM selection_source_facts_v2 f
        WHERE f.first_ingress_run_id=NEW.subject_id
          AND NOT EXISTS (
              SELECT 1 FROM selection_source_fact_attempts fa
              WHERE fa.ingress_run_id=NEW.subject_id
                AND fa.source_fact_key=f.source_fact_key
          )
    );
    SELECT RAISE(ABORT, 'BR-174 ingress staged row count mismatch')
    WHERE NEW.expected_staged_row_count <> (
        1
        + (SELECT COUNT(*) FROM selection_source_batch_attempts b
           WHERE b.ingress_run_id=NEW.subject_id)
        + (SELECT COUNT(*) FROM selection_source_facts_v2 f
           WHERE f.first_ingress_run_id=NEW.subject_id)
        + (SELECT COUNT(*) FROM selection_source_fact_attempts fa
           WHERE fa.ingress_run_id=NEW.subject_id)
    );
END;

CREATE TRIGGER IF NOT EXISTS selection_v2_generation_manifest_closure
BEFORE INSERT ON selection_v2_run_stages
WHEN NEW.subject_kind='generation_run'
BEGIN
    SELECT RAISE(ABORT, 'BR-174 generation requires receipted activation and ingress')
    WHERE NOT EXISTS (
        SELECT 1
        FROM selection_source_facts_v2 f
        JOIN selection_v2_commit_receipts ar
          ON ar.subject_kind='config_activation'
         AND ar.subject_id=f.config_activation_run_id
        JOIN selection_v2_commit_receipts ir
          ON ir.subject_kind='ingress_run'
         AND ir.subject_id=f.first_ingress_run_id
        WHERE f.source_fact_key=NEW.source_fact_key
          AND f.ingress_decision='admitted'
          AND f.config_activation_run_id=NEW.config_activation_run_id
          AND f.config_hash=NEW.config_hash
          AND f.generation_market_date=NEW.generation_market_date
    );
    SELECT RAISE(ABORT, 'BR-174 generation domain lineage mismatch')
    WHERE EXISTS (
        SELECT source_fact_key FROM selection_relation_attempts
        WHERE generation_run_id=NEW.subject_id AND source_fact_key<>NEW.source_fact_key
        UNION ALL
        SELECT source_fact_key FROM selection_evaluation_attempts
        WHERE generation_run_id=NEW.subject_id AND source_fact_key<>NEW.source_fact_key
        UNION ALL
        SELECT source_fact_key FROM selection_samples
        WHERE generation_run_id=NEW.subject_id AND source_fact_key<>NEW.source_fact_key
    );
    SELECT RAISE(ABORT, 'BR-174 generation terminal/rejection matrix mismatch')
    WHERE EXISTS (
        SELECT 1
        FROM selection_samples s
        WHERE s.generation_run_id=NEW.subject_id
          AND (
            (s.decision_kind='admitted' AND EXISTS (
                SELECT 1 FROM selection_rejections r WHERE r.sample_key=s.sample_key
            ))
            OR
            (s.decision_kind='hard_rejected' AND (
                (SELECT COUNT(*) FROM selection_rejections r
                 WHERE r.sample_key=s.sample_key)<>s.rejection_count
                OR (SELECT MIN(ordinal) FROM selection_rejections r
                    WHERE r.sample_key=s.sample_key)<>0
                OR (SELECT MAX(ordinal) FROM selection_rejections r
                    WHERE r.sample_key=s.sample_key)<>s.rejection_count-1
                OR EXISTS (
                    SELECT 1 FROM selection_rejections r
                    WHERE r.sample_key=s.sample_key
                      AND json_extract(
                          s.rejection_row_hashes_in_ordinal_order,
                          '$[' || r.ordinal || ']'
                      )<>r.content_hash
                )
            ))
          )
    ) OR EXISTS (
        SELECT 1
        FROM selection_evaluation_attempts e
        WHERE e.generation_run_id=NEW.subject_id
          AND (
            (e.result_code='completed' AND NOT EXISTS (
                SELECT 1 FROM selection_samples s
                WHERE s.generation_run_id=e.generation_run_id
                  AND s.sample_key=e.sample_key
                  AND s.source_fact_key=e.source_fact_key
                  AND s.content_hash=e.terminal_decision_hash
                  AND s.relation_evidence_set_hash=e.relation_evidence_set_hash
            ))
            OR
            (e.result_code='error' AND EXISTS (
                SELECT 1 FROM selection_samples s
                WHERE s.generation_run_id=e.generation_run_id
                  AND s.sample_key=e.sample_key
            ))
          )
    );
    SELECT RAISE(ABORT, 'BR-174 generation status matrix mismatch')
    WHERE (NEW.run_status='verified_no_relation' AND (
              EXISTS (SELECT 1 FROM selection_relation_attempts
                      WHERE generation_run_id=NEW.subject_id)
              OR EXISTS (SELECT 1 FROM selection_evaluation_attempts
                         WHERE generation_run_id=NEW.subject_id)
              OR EXISTS (SELECT 1 FROM selection_samples
                         WHERE generation_run_id=NEW.subject_id)
          ))
       OR (NEW.run_status='completed' AND (
              NOT EXISTS (
                  SELECT 1 FROM selection_relation_attempts r
                  WHERE r.generation_run_id=NEW.subject_id
              )
              OR EXISTS (
                  SELECT 1 FROM selection_relation_attempts r
                  WHERE r.generation_run_id=NEW.subject_id
                    AND r.result_code<>'resolved'
                    AND r.retryable<>0
              )
              OR EXISTS (
                  SELECT 1 FROM selection_evaluation_attempts e
                  WHERE e.generation_run_id=NEW.subject_id
                    AND e.result_code<>'completed'
              )
          ));
    SELECT RAISE(ABORT, 'BR-174 generation dependency status mismatch')
    WHERE (NEW.run_status='pending_dependency' AND NOT EXISTS (
              SELECT 1 FROM selection_relation_attempts r
              WHERE r.generation_run_id=NEW.subject_id AND r.retryable=1
              UNION ALL
              SELECT 1 FROM selection_evaluation_attempts e
              WHERE e.generation_run_id=NEW.subject_id AND e.retryable=1
          ))
       OR (NEW.run_status='failed_non_retryable' AND (
              EXISTS (
                  SELECT 1 FROM selection_relation_attempts r
                  WHERE r.generation_run_id=NEW.subject_id AND r.retryable=1
                  UNION ALL
                  SELECT 1 FROM selection_evaluation_attempts e
                  WHERE e.generation_run_id=NEW.subject_id AND e.retryable=1
              )
              OR NOT EXISTS (
                  SELECT 1 FROM selection_relation_attempts r
                  WHERE r.generation_run_id=NEW.subject_id
                    AND r.result_code<>'resolved' AND r.retryable=0
                  UNION ALL
                  SELECT 1 FROM selection_evaluation_attempts e
                  WHERE e.generation_run_id=NEW.subject_id
                    AND e.result_code='error' AND e.retryable=0
              )
          ));
    SELECT RAISE(ABORT, 'BR-174 generation staged row count mismatch')
    WHERE NEW.expected_staged_row_count <> (
        1
        + (SELECT COUNT(*) FROM selection_relation_attempts r
           WHERE r.generation_run_id=NEW.subject_id)
        + (SELECT COUNT(*) FROM selection_evaluation_attempts e
           WHERE e.generation_run_id=NEW.subject_id)
        + (SELECT COUNT(*) FROM selection_samples s
           WHERE s.generation_run_id=NEW.subject_id)
        + (SELECT COUNT(*) FROM selection_rejections x
           WHERE x.generation_run_id=NEW.subject_id)
    );
END;

CREATE TRIGGER IF NOT EXISTS selection_v2_outcome_manifest_closure
BEFORE INSERT ON selection_v2_run_stages
WHEN NEW.subject_kind='outcome_run'
BEGIN
    SELECT RAISE(ABORT, 'BR-178 outcome manifest must be inserted last')
    WHERE NOT EXISTS (
        SELECT 1 FROM selection_v2_recovery_envelopes e
        WHERE e.stage_run_id=NEW.subject_id AND e.subject_kind='outcome_run'
    );
    SELECT RAISE(ABORT, 'BR-178 outcome upstream receipt lineage mismatch')
    WHERE NOT EXISTS (
        SELECT 1
        FROM selection_outcome_attempts a
        JOIN selection_samples s ON s.sample_key=a.sample_key
        JOIN selection_source_facts_v2 f ON f.source_fact_key=s.source_fact_key
        JOIN selection_v2_commit_receipts ar
          ON ar.subject_kind='config_activation'
         AND ar.subject_id=s.config_activation_run_id
        JOIN selection_v2_commit_receipts ir
          ON ir.subject_kind='ingress_run'
         AND ir.subject_id=f.first_ingress_run_id
        JOIN selection_v2_commit_receipts gr
          ON gr.subject_kind='generation_run'
         AND gr.subject_id=s.generation_run_id
        WHERE a.outcome_run_id=NEW.subject_id
          AND s.decision_kind IN ('admitted','hard_rejected')
          AND s.config_activation_run_id=NEW.config_activation_run_id
          AND s.config_hash=NEW.config_hash
          AND a.stored_due_date=CASE a.phase
              WHEN 't0_close' THEN s.t0_due_date
              WHEN 'd1_settled' THEN s.d1_due_date
              WHEN 'd3_settled' THEN s.d3_due_date
              WHEN 'd5_settled' THEN s.d5_due_date
          END
    );
    SELECT RAISE(ABORT, 'BR-178 required preceding settled phase receipt missing')
    WHERE (
        SELECT COUNT(DISTINCT pa.phase)
        FROM selection_outcome_attempts a
        JOIN selection_outcome_attempts pa ON pa.sample_key=a.sample_key
        JOIN selection_v2_run_stages pm
          ON pm.subject_kind='outcome_run'
         AND pm.subject_id=pa.outcome_run_id
         AND pm.run_status='settled'
        JOIN selection_v2_commit_receipts pr
          ON pr.subject_kind='outcome_run'
         AND pr.subject_id=pa.outcome_run_id
        WHERE a.outcome_run_id=NEW.subject_id
          AND pa.result_code='settled'
          AND (
              (a.phase='d1_settled' AND pa.phase='t0_close')
              OR (a.phase='d3_settled' AND pa.phase IN ('t0_close','d1_settled'))
              OR (a.phase='d5_settled'
                  AND pa.phase IN ('t0_close','d1_settled','d3_settled'))
          )
    ) <> CASE NEW.outcome_phase
        WHEN 't0_close' THEN 0
        WHEN 'd1_settled' THEN 1
        WHEN 'd3_settled' THEN 2
        WHEN 'd5_settled' THEN 3
    END;
    SELECT RAISE(ABORT, 'BR-178 outcome requires exactly one attempt')
    WHERE (SELECT COUNT(*) FROM selection_outcome_attempts a
           WHERE a.outcome_run_id=NEW.subject_id) <> 1;
    SELECT RAISE(ABORT, 'BR-178 outcome attempt identity mismatch')
    WHERE EXISTS (
        SELECT 1 FROM selection_outcome_attempts a
        WHERE a.outcome_run_id=NEW.subject_id
          AND (a.phase<>NEW.outcome_phase OR a.stored_due_date<>NEW.stored_due_date)
    );
    SELECT RAISE(ABORT, 'BR-178 outcome status/result mismatch')
    WHERE NOT EXISTS (
        SELECT 1 FROM selection_outcome_attempts a
        WHERE a.outcome_run_id=NEW.subject_id
          AND (
            (NEW.run_status='settled' AND a.result_code='settled')
            OR (NEW.run_status='expected_wait' AND a.result_code='expected_wait')
            OR (NEW.run_status='failed_retryable'
                AND a.result_code='error' AND a.retryable=1)
            OR (NEW.run_status='failed_non_retryable'
                AND a.result_code='error' AND a.retryable=0)
          )
    );
    SELECT RAISE(ABORT, 'BR-178 outcome cardinality mismatch')
    WHERE (
        (NEW.run_status='settled'
         AND (SELECT COUNT(*) FROM selection_sample_outcomes o
              WHERE o.outcome_run_id=NEW.subject_id) <> 1)
        OR
        (NEW.run_status<>'settled'
         AND (SELECT COUNT(*) FROM selection_sample_outcomes o
              WHERE o.outcome_run_id=NEW.subject_id) <> 0)
    );
    SELECT RAISE(ABORT, 'BR-178 outcome identity mismatch')
    WHERE EXISTS (
        SELECT 1
        FROM selection_sample_outcomes o
        JOIN selection_outcome_attempts a ON a.outcome_run_id=o.outcome_run_id
        WHERE o.outcome_run_id=NEW.subject_id
          AND (o.sample_key<>a.sample_key OR o.phase<>a.phase
               OR o.phase<>NEW.outcome_phase
               OR o.due_trading_date<>NEW.stored_due_date)
    );
    SELECT RAISE(ABORT, 'BR-178 settled outcome hash mismatch')
    WHERE NEW.run_status='settled' AND NOT EXISTS (
        SELECT 1
        FROM selection_outcome_attempts a
        JOIN selection_sample_outcomes o
          ON o.outcome_run_id=a.outcome_run_id
         AND o.sample_key=a.sample_key AND o.phase=a.phase
        WHERE a.outcome_run_id=NEW.subject_id
          AND a.settled_outcome_content_hash=o.content_hash
    );
    SELECT RAISE(ABORT, 'BR-178 staged row count mismatch')
    WHERE NEW.expected_staged_row_count
          <> CASE WHEN NEW.run_status='settled' THEN 3 ELSE 2 END
       OR NEW.expected_staged_row_count <> (
            1
            + (SELECT COUNT(*) FROM selection_outcome_attempts a
               WHERE a.outcome_run_id=NEW.subject_id)
            + (SELECT COUNT(*) FROM selection_sample_outcomes o
               WHERE o.outcome_run_id=NEW.subject_id)
       );
END;

CREATE TRIGGER IF NOT EXISTS selection_v2_receipt_manifest_binding
BEFORE INSERT ON selection_v2_commit_receipts
BEGIN
    SELECT RAISE(ABORT, 'BR-174 receipt/manifest/envelope binding mismatch')
    WHERE NOT EXISTS (
        SELECT 1
        FROM selection_v2_run_stages m
        JOIN selection_v2_recovery_envelopes e ON e.stage_run_id=m.subject_id
        WHERE m.subject_kind=NEW.subject_kind
          AND m.subject_id=NEW.subject_id
          AND m.logical_subject_key=NEW.logical_subject_key
          AND m.in_memory_payload_hash=NEW.in_memory_payload_hash
          AND m.prepared_record_hash=NEW.prepared_audit_hash
          AND m.staged_db_content_hash=NEW.staged_db_content_hash
          AND m.manifest_content_hash=NEW.run_manifest_content_hash
          AND m.recovery_envelope_content_hash=NEW.recovery_envelope_content_hash
          AND e.content_hash=NEW.recovery_envelope_content_hash
    );
END;

CREATE TRIGGER IF NOT EXISTS selection_v2_config_receipt_closure
BEFORE INSERT ON selection_v2_commit_receipts
WHEN NEW.subject_kind='config_activation'
BEGIN
    SELECT RAISE(ABORT, 'BR-174 config activation receipt closure mismatch')
    WHERE NOT EXISTS (
        SELECT 1 FROM selection_v2_run_stages m
        WHERE m.subject_kind='config_activation'
          AND m.subject_id=NEW.subject_id
          AND m.run_status='activated'
          AND m.expected_staged_row_count=1
          AND m.config_activation_run_id=m.subject_id
          AND m.config_snapshot_json_hash IS NOT NULL
          AND m.config_activation_content_hash IS NOT NULL
          AND m.config_activation_file_content_hash IS NOT NULL
          AND m.legacy_cutover_snapshot_hash IS NOT NULL
    );
END;

CREATE TRIGGER IF NOT EXISTS selection_v2_ingress_receipt_closure
BEFORE INSERT ON selection_v2_commit_receipts
WHEN NEW.subject_kind='ingress_run'
BEGIN
    SELECT RAISE(ABORT, 'BR-174 ingress receipt missing activation')
    WHERE NOT EXISTS (
        SELECT 1
        FROM selection_v2_run_stages m
        JOIN selection_v2_commit_receipts ar
          ON ar.subject_kind='config_activation'
         AND ar.subject_id=m.config_activation_run_id
        WHERE m.subject_kind='ingress_run' AND m.subject_id=NEW.subject_id
    );
    SELECT RAISE(ABORT, 'BR-174 ingress receipt no-loss/count mismatch')
    WHERE (SELECT COUNT(*) FROM selection_source_batch_attempts b
           WHERE b.ingress_run_id=NEW.subject_id)<>4
       OR (SELECT COUNT(DISTINCT registered_feed_snapshot_hash)
           FROM selection_source_batch_attempts b
           WHERE b.ingress_run_id=NEW.subject_id)<>1
       OR EXISTS (
        SELECT 1
        FROM selection_source_batch_attempts b
        JOIN selection_v2_run_stages m
          ON m.subject_kind='ingress_run' AND m.subject_id=b.ingress_run_id
        WHERE b.ingress_run_id=NEW.subject_id
          AND (
            b.config_activation_run_id<>m.config_activation_run_id
            OR b.config_hash<>m.config_hash
            OR b.generation_market_date<>m.generation_market_date
            OR (b.status_kind='available' AND b.record_count<>(
                SELECT COUNT(*) FROM selection_source_fact_attempts fa
                WHERE fa.source_batch_attempt_id=b.source_batch_attempt_id
            ))
            OR (b.status_kind='available' AND (
                (SELECT MIN(provider_ordinal) FROM selection_source_fact_attempts fa
                 WHERE fa.source_batch_attempt_id=b.source_batch_attempt_id)<>0
                OR (SELECT MAX(provider_ordinal) FROM selection_source_fact_attempts fa
                    WHERE fa.source_batch_attempt_id=b.source_batch_attempt_id)
                   <>b.record_count-1
                OR EXISTS (
                    SELECT 1 FROM selection_source_fact_attempts fa
                    WHERE fa.source_batch_attempt_id=b.source_batch_attempt_id
                      AND fa.batch_evidence_hash<>b.available_evidence_hash
                )
            ))
            OR (b.status_kind<>'available' AND EXISTS (
                SELECT 1 FROM selection_source_fact_attempts fa
                WHERE fa.source_batch_attempt_id=b.source_batch_attempt_id
            ))
          )
    ) OR EXISTS (
        SELECT 1
        FROM selection_source_facts_v2 f
        JOIN selection_v2_run_stages m
          ON m.subject_kind='ingress_run' AND m.subject_id=f.first_ingress_run_id
        WHERE f.first_ingress_run_id=NEW.subject_id
          AND (
            f.config_activation_run_id<>m.config_activation_run_id
            OR f.config_hash<>m.config_hash
            OR f.generation_market_date<>m.generation_market_date
            OR NOT EXISTS (
                SELECT 1 FROM selection_source_fact_attempts fa
                WHERE fa.ingress_run_id=f.first_ingress_run_id
                  AND fa.source_fact_key=f.source_fact_key
            )
          )
    ) OR NOT EXISTS (
        SELECT 1
        FROM selection_v2_run_stages m
        WHERE m.subject_kind='ingress_run' AND m.subject_id=NEW.subject_id
          AND m.expected_staged_row_count=(
            1
            + (SELECT COUNT(*) FROM selection_source_batch_attempts b
               WHERE b.ingress_run_id=NEW.subject_id)
            + (SELECT COUNT(*) FROM selection_source_facts_v2 f
               WHERE f.first_ingress_run_id=NEW.subject_id)
            + (SELECT COUNT(*) FROM selection_source_fact_attempts fa
               WHERE fa.ingress_run_id=NEW.subject_id)
          )
    );
END;

CREATE TRIGGER IF NOT EXISTS selection_v2_generation_receipt_closure
BEFORE INSERT ON selection_v2_commit_receipts
WHEN NEW.subject_kind='generation_run'
BEGIN
    SELECT RAISE(ABORT, 'BR-174 generation receipt missing activation/source ingress')
    WHERE NOT EXISTS (
        SELECT 1
        FROM selection_v2_run_stages m
        JOIN selection_source_facts_v2 f
          ON f.source_fact_key=m.source_fact_key
        JOIN selection_v2_commit_receipts ar
          ON ar.subject_kind='config_activation'
         AND ar.subject_id=m.config_activation_run_id
        JOIN selection_v2_commit_receipts ir
          ON ir.subject_kind='ingress_run'
         AND ir.subject_id=f.first_ingress_run_id
        WHERE m.subject_kind='generation_run'
          AND m.subject_id=NEW.subject_id
          AND f.ingress_decision='admitted'
          AND f.config_activation_run_id=m.config_activation_run_id
          AND f.config_hash=m.config_hash
          AND f.generation_market_date=m.generation_market_date
    );
    SELECT RAISE(ABORT, 'BR-174 generation receipt source fact is not admitted')
    WHERE EXISTS (
        SELECT 1
        FROM (
            SELECT source_fact_key FROM selection_relation_attempts
            WHERE generation_run_id=NEW.subject_id
            UNION ALL
            SELECT source_fact_key FROM selection_evaluation_attempts
            WHERE generation_run_id=NEW.subject_id
            UNION ALL
            SELECT source_fact_key FROM selection_samples
            WHERE generation_run_id=NEW.subject_id
        ) x
        LEFT JOIN selection_source_facts_v2 f ON f.source_fact_key=x.source_fact_key
        WHERE f.source_fact_key IS NULL OR f.ingress_decision<>'admitted'
    );
    SELECT RAISE(ABORT, 'BR-174 generation receipt missing ingress receipt')
    WHERE EXISTS (
        SELECT 1
        FROM (
            SELECT source_fact_key FROM selection_relation_attempts
            WHERE generation_run_id=NEW.subject_id
            UNION ALL
            SELECT source_fact_key FROM selection_evaluation_attempts
            WHERE generation_run_id=NEW.subject_id
            UNION ALL
            SELECT source_fact_key FROM selection_samples
            WHERE generation_run_id=NEW.subject_id
        ) x
        JOIN selection_source_facts_v2 f ON f.source_fact_key=x.source_fact_key
        LEFT JOIN selection_v2_commit_receipts ir
          ON ir.subject_kind='ingress_run' AND ir.subject_id=f.first_ingress_run_id
        WHERE ir.subject_id IS NULL
    );
    SELECT RAISE(ABORT, 'BR-174 generation receipt terminal closure mismatch')
    WHERE EXISTS (
        SELECT 1
        FROM selection_samples s
        LEFT JOIN selection_evaluation_attempts e
          ON e.generation_run_id=s.generation_run_id
         AND e.sample_key=s.sample_key
        WHERE s.generation_run_id=NEW.subject_id
          AND (
            e.evaluation_attempt_id IS NULL
            OR e.result_code<>'completed'
            OR e.terminal_decision_hash<>s.content_hash
            OR e.source_fact_key<>s.source_fact_key
            OR e.relation_evidence_set_hash<>s.relation_evidence_set_hash
            OR (s.decision_kind='admitted' AND EXISTS (
                SELECT 1 FROM selection_rejections r WHERE r.sample_key=s.sample_key
            ))
            OR (s.decision_kind='hard_rejected' AND (
                (SELECT COUNT(*) FROM selection_rejections r
                 WHERE r.sample_key=s.sample_key)<>s.rejection_count
                OR (SELECT MIN(ordinal) FROM selection_rejections r
                    WHERE r.sample_key=s.sample_key)<>0
                OR (SELECT MAX(ordinal) FROM selection_rejections r
                    WHERE r.sample_key=s.sample_key)<>s.rejection_count-1
                OR EXISTS (
                    SELECT 1 FROM selection_rejections r
                    WHERE r.sample_key=s.sample_key
                      AND json_extract(
                          s.rejection_row_hashes_in_ordinal_order,
                          '$[' || r.ordinal || ']'
                      )<>r.content_hash
                )
            ))
          )
    ) OR EXISTS (
        SELECT 1
        FROM selection_evaluation_attempts e
        WHERE e.generation_run_id=NEW.subject_id
          AND (
            (e.result_code='completed' AND NOT EXISTS (
                SELECT 1 FROM selection_samples s
                WHERE s.generation_run_id=e.generation_run_id
                  AND s.sample_key=e.sample_key
                  AND s.content_hash=e.terminal_decision_hash
                  AND s.relation_evidence_set_hash=e.relation_evidence_set_hash
            ))
            OR (e.result_code='error' AND EXISTS (
                SELECT 1 FROM selection_samples s
                WHERE s.generation_run_id=e.generation_run_id
                  AND s.sample_key=e.sample_key
            ))
          )
    ) OR NOT EXISTS (
        SELECT 1 FROM selection_v2_run_stages m
        WHERE m.subject_kind='generation_run' AND m.subject_id=NEW.subject_id
          AND m.expected_staged_row_count=(
            1
            + (SELECT COUNT(*) FROM selection_relation_attempts r
               WHERE r.generation_run_id=NEW.subject_id)
            + (SELECT COUNT(*) FROM selection_evaluation_attempts e
               WHERE e.generation_run_id=NEW.subject_id)
            + (SELECT COUNT(*) FROM selection_samples s
               WHERE s.generation_run_id=NEW.subject_id)
            + (SELECT COUNT(*) FROM selection_rejections x
               WHERE x.generation_run_id=NEW.subject_id)
          )
    );
END;

CREATE TRIGGER IF NOT EXISTS selection_v2_outcome_receipt_closure
BEFORE INSERT ON selection_v2_commit_receipts
WHEN NEW.subject_kind='outcome_run'
BEGIN
    SELECT RAISE(ABORT, 'BR-178 outcome receipt upstream lineage mismatch')
    WHERE NOT EXISTS (
        SELECT 1
        FROM selection_v2_run_stages m
        JOIN selection_outcome_attempts a ON a.outcome_run_id=m.subject_id
        JOIN selection_samples s ON s.sample_key=a.sample_key
        JOIN selection_source_facts_v2 f ON f.source_fact_key=s.source_fact_key
        JOIN selection_v2_commit_receipts ar
          ON ar.subject_kind='config_activation'
         AND ar.subject_id=s.config_activation_run_id
        JOIN selection_v2_commit_receipts ir
          ON ir.subject_kind='ingress_run'
         AND ir.subject_id=f.first_ingress_run_id
        JOIN selection_v2_commit_receipts gr
          ON gr.subject_kind='generation_run'
         AND gr.subject_id=s.generation_run_id
        WHERE m.subject_kind='outcome_run' AND m.subject_id=NEW.subject_id
          AND s.decision_kind IN ('admitted','hard_rejected')
          AND s.config_activation_run_id=m.config_activation_run_id
          AND s.config_hash=m.config_hash
          AND a.phase=m.outcome_phase
          AND a.stored_due_date=m.stored_due_date
          AND a.stored_due_date=CASE a.phase
              WHEN 't0_close' THEN s.t0_due_date
              WHEN 'd1_settled' THEN s.d1_due_date
              WHEN 'd3_settled' THEN s.d3_due_date
              WHEN 'd5_settled' THEN s.d5_due_date
          END
    );
    SELECT RAISE(ABORT, 'BR-178 outcome receipt preceding phase missing')
    WHERE (
        SELECT COUNT(DISTINCT pa.phase)
        FROM selection_outcome_attempts a
        JOIN selection_outcome_attempts pa ON pa.sample_key=a.sample_key
        JOIN selection_v2_run_stages pm
          ON pm.subject_kind='outcome_run'
         AND pm.subject_id=pa.outcome_run_id
         AND pm.run_status='settled'
        JOIN selection_v2_commit_receipts pr
          ON pr.subject_kind='outcome_run'
         AND pr.subject_id=pa.outcome_run_id
        WHERE a.outcome_run_id=NEW.subject_id
          AND pa.result_code='settled'
          AND (
              (a.phase='d1_settled' AND pa.phase='t0_close')
              OR (a.phase='d3_settled' AND pa.phase IN ('t0_close','d1_settled'))
              OR (a.phase='d5_settled'
                  AND pa.phase IN ('t0_close','d1_settled','d3_settled'))
          )
    ) <> (
        SELECT CASE m.outcome_phase
            WHEN 't0_close' THEN 0
            WHEN 'd1_settled' THEN 1
            WHEN 'd3_settled' THEN 2
            WHEN 'd5_settled' THEN 3
        END
        FROM selection_v2_run_stages m
        WHERE m.subject_kind='outcome_run' AND m.subject_id=NEW.subject_id
    );
    SELECT RAISE(ABORT, 'BR-178 outcome receipt requires exactly one attempt')
    WHERE (SELECT COUNT(*) FROM selection_outcome_attempts a
           WHERE a.outcome_run_id=NEW.subject_id) <> 1;
    SELECT RAISE(ABORT, 'BR-178 outcome receipt status/cardinality mismatch')
    WHERE NOT EXISTS (
        SELECT 1
        FROM selection_v2_run_stages m
        JOIN selection_outcome_attempts a ON a.outcome_run_id=m.subject_id
        WHERE m.subject_id=NEW.subject_id
          AND (
            (m.run_status='settled' AND a.result_code='settled'
             AND (SELECT COUNT(*) FROM selection_sample_outcomes o
                  WHERE o.outcome_run_id=m.subject_id)=1)
            OR
            (m.run_status='expected_wait' AND a.result_code='expected_wait'
             AND (SELECT COUNT(*) FROM selection_sample_outcomes o
                  WHERE o.outcome_run_id=m.subject_id)=0)
            OR
            (m.run_status='failed_retryable' AND a.result_code='error'
             AND a.retryable=1
             AND (SELECT COUNT(*) FROM selection_sample_outcomes o
                  WHERE o.outcome_run_id=m.subject_id)=0)
            OR
            (m.run_status='failed_non_retryable' AND a.result_code='error'
             AND a.retryable=0
             AND (SELECT COUNT(*) FROM selection_sample_outcomes o
                  WHERE o.outcome_run_id=m.subject_id)=0)
          )
    );
    SELECT RAISE(ABORT, 'BR-178 settled receipt hash mismatch')
    WHERE EXISTS (
        SELECT 1 FROM selection_v2_run_stages m
        JOIN selection_outcome_attempts a ON a.outcome_run_id=m.subject_id
        WHERE m.subject_id=NEW.subject_id AND m.run_status='settled'
          AND NOT EXISTS (
            SELECT 1 FROM selection_sample_outcomes o
            WHERE o.outcome_run_id=m.subject_id
              AND o.sample_key=a.sample_key AND o.phase=a.phase
              AND o.content_hash=a.settled_outcome_content_hash
          )
    );
END;
"#;

const SELECTION_V2_FINAL_RECOVERY_ENVELOPES_TABLE: &str = r#"CREATE TABLE IF NOT EXISTS selection_v2_recovery_envelopes (
    stage_run_id TEXT PRIMARY KEY NOT NULL,
    subject_kind TEXT NOT NULL CHECK (
        subject_kind IN (
            'config_activation','ingress_run','generation_run','outcome_claim','outcome_run'
        )
    ),
    logical_subject_key TEXT NOT NULL,
    payload_schema TEXT NOT NULL CHECK (
        (subject_kind='config_activation' AND payload_schema='config-activation-stage-v1')
        OR (subject_kind='ingress_run' AND payload_schema='source-ingress-stage-v2')
        OR (subject_kind='generation_run' AND payload_schema='generation-stage-v3')
        OR (subject_kind='outcome_claim' AND payload_schema='outcome-claim-stage-v2')
        OR (subject_kind='outcome_run' AND payload_schema='outcome-stage-v3')
    ),
    payload_json TEXT NOT NULL CHECK (
        json_valid(payload_json)
        AND COALESCE(json_type(payload_json, '$.domain')='text', 0)
        AND (
            (subject_kind='config_activation'
                AND json_extract(payload_json, '$.domain')
                    ='stock_analysis.br174.config_activation_stage.v1')
            OR
            (subject_kind='ingress_run'
                AND json_extract(payload_json, '$.domain')
                    ='stock_analysis.br174.source_ingress_stage.v2')
            OR
            (subject_kind='generation_run'
                AND json_extract(payload_json, '$.domain')
                    ='stock_analysis.br174.generation_stage.v3')
            OR
            (subject_kind='outcome_claim'
                AND json_extract(payload_json, '$.domain')
                    ='stock_analysis.br174.outcome_claim_stage.v2')
            OR
            (subject_kind='outcome_run'
                AND json_extract(payload_json, '$.domain')
                    ='stock_analysis.br174.outcome_stage.v3')
        )
    ),
    payload_json_hash TEXT NOT NULL,
    in_memory_payload_hash TEXT NOT NULL,
    config_activation_run_id TEXT NOT NULL,
    config_hash TEXT NOT NULL,
    enveloped_at TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    CHECK (
        subject_kind<>'config_activation'
        OR config_activation_run_id=stage_run_id
    ),
    UNIQUE(stage_run_id, payload_json_hash, in_memory_payload_hash)
);"#;

const SELECTION_V2_FINAL_OUTCOME_ATTEMPTS_TABLE: &str = r#"CREATE TABLE IF NOT EXISTS selection_outcome_attempts (
    outcome_attempt_id TEXT PRIMARY KEY NOT NULL,
    sample_key TEXT NOT NULL,
    phase TEXT NOT NULL CHECK (
        phase IN ('t0_close','d1_settled','d3_settled','d5_settled')
    ),
    stored_due_date TEXT NOT NULL,
    outcome_run_id TEXT NOT NULL,
    request_hash TEXT,
    request_evidence_json TEXT,
    request_evidence_hash TEXT,
    transport_attempts_json TEXT,
    transport_attempts_hash TEXT,
    result_code TEXT NOT NULL CHECK (result_code IN ('settled','expected_wait','error')),
    reason_code TEXT,
    retryable INTEGER CHECK (retryable IN (0,1)),
    provider TEXT,
    source TEXT,
    source_at TEXT,
    observed_at TEXT,
    batch_id TEXT,
    batch_content_hash TEXT,
    available_evidence_json TEXT,
    available_evidence_hash TEXT,
    error_detail_json TEXT,
    error_detail_hash TEXT,
    error_fingerprint TEXT,
    settled_outcome_content_hash TEXT,
    attempted_at TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    CHECK (
        (request_hash IS NULL
            AND request_evidence_json IS NULL AND request_evidence_hash IS NULL)
        OR
        (request_hash IS NOT NULL
            AND request_evidence_json IS NOT NULL
            AND request_evidence_hash IS NOT NULL
            AND json_valid(request_evidence_json))
    ),
    CHECK (
        (transport_attempts_json IS NULL AND transport_attempts_hash IS NULL)
        OR
        (transport_attempts_json IS NOT NULL AND transport_attempts_hash IS NOT NULL
            AND json_valid(transport_attempts_json))
    ),
    CHECK (
        (available_evidence_json IS NULL AND available_evidence_hash IS NULL)
        OR
        (available_evidence_json IS NOT NULL AND available_evidence_hash IS NOT NULL
            AND json_valid(available_evidence_json))
    ),
    CHECK (
        (error_detail_json IS NULL AND error_detail_hash IS NULL)
        OR
        (error_detail_json IS NOT NULL AND error_detail_hash IS NOT NULL
            AND json_valid(error_detail_json))
    ),
    CHECK (
        (result_code='settled' AND reason_code IS NULL AND retryable IS NULL
            AND request_hash IS NOT NULL AND request_evidence_json IS NOT NULL
            AND request_evidence_hash IS NOT NULL
            AND transport_attempts_json IS NOT NULL AND transport_attempts_hash IS NOT NULL
            AND provider IS NOT NULL AND source IS NOT NULL
            AND observed_at IS NOT NULL AND batch_id IS NOT NULL
            AND batch_content_hash IS NOT NULL
            AND available_evidence_json IS NOT NULL
            AND error_detail_json IS NULL AND error_fingerprint IS NULL
            AND settled_outcome_content_hash IS NOT NULL)
        OR
        (result_code='expected_wait' AND reason_code='market_session_unsettled'
            AND request_hash IS NULL AND request_evidence_json IS NULL
            AND request_evidence_hash IS NULL
            AND transport_attempts_json IS NULL AND transport_attempts_hash IS NULL
            AND retryable IS NULL AND provider IS NULL
            AND source IS NULL AND source_at IS NULL AND observed_at IS NULL
            AND batch_id IS NULL AND batch_content_hash IS NULL
            AND available_evidence_json IS NULL
            AND error_detail_json IS NULL AND error_fingerprint IS NULL
            AND settled_outcome_content_hash IS NULL)
        OR
        (result_code='error' AND reason_code IS NOT NULL
            AND length(reason_code)>0 AND retryable IS NOT NULL
            AND request_hash IS NOT NULL AND request_evidence_json IS NOT NULL
            AND request_evidence_hash IS NOT NULL
            AND transport_attempts_json IS NOT NULL AND transport_attempts_hash IS NOT NULL
            AND error_detail_json IS NOT NULL AND error_fingerprint IS NOT NULL
            AND settled_outcome_content_hash IS NULL)
    ),
    FOREIGN KEY(sample_key) REFERENCES selection_samples(sample_key)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY(outcome_run_id)
        REFERENCES selection_v2_recovery_envelopes(stage_run_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    UNIQUE(outcome_run_id, sample_key, phase)
);"#;

const SELECTION_V2_FINAL_RUN_STAGES_TABLE: &str = r#"CREATE TABLE IF NOT EXISTS selection_v2_run_stages (
    subject_kind TEXT NOT NULL CHECK (
        subject_kind IN (
            'config_activation','ingress_run','generation_run','outcome_claim','outcome_run'
        )
    ),
    subject_id TEXT PRIMARY KEY NOT NULL,
    in_memory_payload_hash TEXT NOT NULL,
    prepared_record_hash TEXT NOT NULL,
    expected_staged_row_count INTEGER NOT NULL CHECK (expected_staged_row_count >= 1),
    staged_db_content_hash TEXT NOT NULL,
    recovery_envelope_content_hash TEXT NOT NULL,
    logical_subject_key TEXT NOT NULL,
    run_status TEXT NOT NULL,
    source_fact_key TEXT,
    config_activation_run_id TEXT NOT NULL,
    config_hash TEXT NOT NULL,
    config_snapshot_json_hash TEXT,
    config_activation_content_hash TEXT,
    config_activation_file_content_hash TEXT,
    config_effective_from TEXT,
    artifact_valid_from TEXT,
    artifact_expires_at TEXT,
    executable_revision TEXT,
    legacy_cutover_snapshot_hash TEXT,
    generation_market_date TEXT,
    aggregator_observed_at TEXT,
    ingress_source_batch_content_hash TEXT,
    outcome_phase TEXT,
    stored_due_date TEXT,
    outcome_claim_id TEXT,
    planned_outcome_run_id TEXT,
    outcome_claim_receipt_content_hash TEXT,
    outcome_claim_due_binding_hash TEXT,
    outcome_claim_provider_request_hash TEXT,
    staged_at TEXT NOT NULL,
    manifest_content_hash TEXT NOT NULL,
    CHECK (
        (subject_kind='config_activation' AND run_status='activated')
        OR
        (subject_kind='ingress_run'
            AND run_status IN ('completed','failed_non_retryable'))
        OR
        (subject_kind='generation_run'
            AND run_status IN (
                'completed','verified_no_relation','pending_dependency',
                'failed_non_retryable'
            ))
        OR
        (subject_kind='outcome_claim' AND run_status='claimed')
        OR
        (subject_kind='outcome_run'
            AND run_status IN (
                'settled','expected_wait','failed_retryable','failed_non_retryable'
            ))
    ),
    CHECK (
        (subject_kind='config_activation'
            AND config_activation_run_id=subject_id
            AND source_fact_key IS NULL
            AND config_snapshot_json_hash IS NOT NULL
            AND config_activation_content_hash IS NOT NULL
            AND config_activation_file_content_hash IS NOT NULL
            AND config_effective_from IS NOT NULL
            AND artifact_valid_from IS NOT NULL AND artifact_expires_at IS NOT NULL
            AND executable_revision IS NOT NULL
            AND legacy_cutover_snapshot_hash IS NOT NULL
            AND generation_market_date IS NULL
            AND aggregator_observed_at IS NULL
            AND ingress_source_batch_content_hash IS NULL
            AND outcome_phase IS NULL AND stored_due_date IS NULL
            AND outcome_claim_id IS NULL AND planned_outcome_run_id IS NULL
            AND outcome_claim_receipt_content_hash IS NULL
            AND outcome_claim_due_binding_hash IS NULL
            AND outcome_claim_provider_request_hash IS NULL)
        OR
        (subject_kind='ingress_run'
            AND source_fact_key IS NULL
            AND config_snapshot_json_hash IS NULL
            AND config_activation_content_hash IS NULL
            AND config_activation_file_content_hash IS NULL
            AND config_effective_from IS NULL
            AND artifact_valid_from IS NULL AND artifact_expires_at IS NULL
            AND executable_revision IS NULL AND legacy_cutover_snapshot_hash IS NULL
            AND generation_market_date IS NOT NULL
            AND aggregator_observed_at IS NOT NULL
            AND ingress_source_batch_content_hash IS NOT NULL
            AND outcome_phase IS NULL AND stored_due_date IS NULL
            AND outcome_claim_id IS NULL AND planned_outcome_run_id IS NULL
            AND outcome_claim_receipt_content_hash IS NULL
            AND outcome_claim_due_binding_hash IS NULL
            AND outcome_claim_provider_request_hash IS NULL)
        OR
        (subject_kind='generation_run'
            AND source_fact_key IS NOT NULL
            AND config_snapshot_json_hash IS NULL
            AND config_activation_content_hash IS NULL
            AND config_activation_file_content_hash IS NULL
            AND config_effective_from IS NULL
            AND artifact_valid_from IS NULL AND artifact_expires_at IS NULL
            AND executable_revision IS NULL AND legacy_cutover_snapshot_hash IS NULL
            AND generation_market_date IS NOT NULL
            AND aggregator_observed_at IS NULL
            AND ingress_source_batch_content_hash IS NULL
            AND outcome_phase IS NULL AND stored_due_date IS NULL
            AND outcome_claim_id IS NULL AND planned_outcome_run_id IS NULL
            AND outcome_claim_receipt_content_hash IS NULL
            AND outcome_claim_due_binding_hash IS NULL
            AND outcome_claim_provider_request_hash IS NULL)
        OR
        (subject_kind='outcome_claim'
            AND source_fact_key IS NULL
            AND config_snapshot_json_hash IS NULL
            AND config_activation_content_hash IS NULL
            AND config_activation_file_content_hash IS NULL
            AND config_effective_from IS NULL
            AND artifact_valid_from IS NULL AND artifact_expires_at IS NULL
            AND executable_revision IS NULL AND legacy_cutover_snapshot_hash IS NULL
            AND generation_market_date IS NULL
            AND aggregator_observed_at IS NULL
            AND ingress_source_batch_content_hash IS NULL
            AND outcome_phase IN ('t0_close','d1_settled','d3_settled','d5_settled')
            AND stored_due_date IS NOT NULL
            AND outcome_claim_id=subject_id
            AND planned_outcome_run_id IS NOT NULL
            AND planned_outcome_run_id<>subject_id
            AND outcome_claim_receipt_content_hash IS NULL
            AND outcome_claim_due_binding_hash IS NOT NULL
            AND outcome_claim_provider_request_hash IS NOT NULL)
        OR
        (subject_kind='outcome_run'
            AND source_fact_key IS NULL
            AND config_snapshot_json_hash IS NULL
            AND config_activation_content_hash IS NULL
            AND config_activation_file_content_hash IS NULL
            AND config_effective_from IS NULL
            AND artifact_valid_from IS NULL AND artifact_expires_at IS NULL
            AND executable_revision IS NULL AND legacy_cutover_snapshot_hash IS NULL
            AND generation_market_date IS NULL
            AND aggregator_observed_at IS NULL
            AND ingress_source_batch_content_hash IS NULL
            AND outcome_phase IN ('t0_close','d1_settled','d3_settled','d5_settled')
            AND stored_due_date IS NOT NULL
            AND outcome_claim_id IS NOT NULL
            AND planned_outcome_run_id IS NULL
            AND outcome_claim_receipt_content_hash IS NOT NULL
            AND outcome_claim_due_binding_hash IS NOT NULL
            AND outcome_claim_provider_request_hash IS NOT NULL)
    ),
    FOREIGN KEY(subject_id)
        REFERENCES selection_v2_recovery_envelopes(stage_run_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
    UNIQUE(subject_kind, subject_id)
);"#;

const SELECTION_V2_FINAL_OUTCOME_MANIFEST_TRIGGER: &str = r#"CREATE TRIGGER IF NOT EXISTS selection_v2_outcome_manifest_closure
BEFORE INSERT ON selection_v2_run_stages
WHEN NEW.subject_kind IN ('outcome_claim','outcome_run')
BEGIN
    SELECT RAISE(ABORT, 'BR-178 outcome claim manifest closure mismatch')
    WHERE NEW.subject_kind='outcome_claim' AND (
        NEW.expected_staged_row_count<>1
        OR NOT EXISTS (
            SELECT 1 FROM selection_v2_commit_receipts ar
            WHERE ar.subject_kind='config_activation'
              AND ar.subject_id=NEW.config_activation_run_id
        )
    );
    SELECT RAISE(ABORT, 'BR-178 outcome run lacks exact receipted claim')
    WHERE NEW.subject_kind='outcome_run' AND NOT EXISTS (
        SELECT 1
        FROM selection_v2_run_stages cm
        JOIN selection_v2_commit_receipts cr
          ON cr.subject_kind='outcome_claim' AND cr.subject_id=cm.subject_id
        WHERE cm.subject_kind='outcome_claim'
          AND cm.subject_id=NEW.outcome_claim_id
          AND cm.planned_outcome_run_id=NEW.subject_id
          AND cm.logical_subject_key=NEW.logical_subject_key
          AND cm.config_activation_run_id=NEW.config_activation_run_id
          AND cm.config_hash=NEW.config_hash
          AND cm.outcome_phase=NEW.outcome_phase
          AND cm.stored_due_date=NEW.stored_due_date
          AND cm.outcome_claim_due_binding_hash=NEW.outcome_claim_due_binding_hash
          AND cm.outcome_claim_provider_request_hash=NEW.outcome_claim_provider_request_hash
          AND cr.content_hash=NEW.outcome_claim_receipt_content_hash
    );
    SELECT RAISE(ABORT, 'BR-182 outcome attempt cardinality mismatch')
    WHERE NEW.subject_kind='outcome_run' AND (
        (NEW.run_status='expected_wait'
         AND (SELECT COUNT(*) FROM selection_outcome_attempts a
              WHERE a.outcome_run_id=NEW.subject_id)<>0)
        OR
        (NEW.run_status<>'expected_wait'
         AND (SELECT COUNT(*) FROM selection_outcome_attempts a
              WHERE a.outcome_run_id=NEW.subject_id)<>1)
    );
    SELECT RAISE(ABORT, 'BR-178 outcome attempt identity/status mismatch')
    WHERE NEW.subject_kind='outcome_run'
      AND NEW.run_status<>'expected_wait'
      AND NOT EXISTS (
        SELECT 1 FROM selection_outcome_attempts a
        WHERE a.outcome_run_id=NEW.subject_id
          AND a.phase=NEW.outcome_phase
          AND a.stored_due_date=NEW.stored_due_date
          AND (
            (NEW.run_status='settled' AND a.result_code='settled')
            OR (NEW.run_status='failed_retryable'
                AND a.result_code='error' AND a.retryable=1)
            OR (NEW.run_status='failed_non_retryable'
                AND a.result_code='error' AND a.retryable=0)
          )
    );
    SELECT RAISE(ABORT, 'BR-178 outcome cardinality mismatch')
    WHERE NEW.subject_kind='outcome_run' AND (
        (NEW.run_status='settled'
         AND (SELECT COUNT(*) FROM selection_sample_outcomes o
              WHERE o.outcome_run_id=NEW.subject_id)<>1)
        OR
        (NEW.run_status<>'settled'
         AND (SELECT COUNT(*) FROM selection_sample_outcomes o
              WHERE o.outcome_run_id=NEW.subject_id)<>0)
        OR
        NEW.expected_staged_row_count
          <> CASE
              WHEN NEW.run_status='settled' THEN 3
              WHEN NEW.run_status='expected_wait' THEN 1
              ELSE 2
             END
    );
END;"#;

const SELECTION_V2_FINAL_OUTCOME_RECEIPT_TRIGGER: &str = r#"CREATE TRIGGER IF NOT EXISTS selection_v2_outcome_receipt_closure
BEFORE INSERT ON selection_v2_commit_receipts
WHEN NEW.subject_kind IN ('outcome_claim','outcome_run')
BEGIN
    SELECT RAISE(ABORT, 'BR-178 outcome claim receipt closure mismatch')
    WHERE NEW.subject_kind='outcome_claim' AND NOT EXISTS (
        SELECT 1 FROM selection_v2_run_stages m
        WHERE m.subject_kind='outcome_claim'
          AND m.subject_id=NEW.subject_id
          AND m.run_status='claimed'
          AND m.expected_staged_row_count=1
          AND m.outcome_claim_id=m.subject_id
          AND m.planned_outcome_run_id IS NOT NULL
          AND m.outcome_claim_receipt_content_hash IS NULL
          AND m.outcome_claim_due_binding_hash IS NOT NULL
          AND m.outcome_claim_provider_request_hash IS NOT NULL
    );
    SELECT RAISE(ABORT, 'BR-178 outcome receipt claim lineage mismatch')
    WHERE NEW.subject_kind='outcome_run' AND NOT EXISTS (
        SELECT 1
        FROM selection_v2_run_stages m
        JOIN selection_v2_run_stages cm
          ON cm.subject_kind='outcome_claim'
         AND cm.subject_id=m.outcome_claim_id
         AND cm.planned_outcome_run_id=m.subject_id
        JOIN selection_v2_commit_receipts cr
          ON cr.subject_kind='outcome_claim'
         AND cr.subject_id=cm.subject_id
         AND cr.content_hash=m.outcome_claim_receipt_content_hash
        WHERE m.subject_kind='outcome_run'
          AND m.subject_id=NEW.subject_id
          AND cm.logical_subject_key=m.logical_subject_key
          AND cm.outcome_claim_due_binding_hash=m.outcome_claim_due_binding_hash
          AND cm.outcome_claim_provider_request_hash=m.outcome_claim_provider_request_hash
    );
    SELECT RAISE(ABORT, 'BR-182 outcome receipt attempt/cardinality mismatch')
    WHERE NEW.subject_kind='outcome_run' AND NOT EXISTS (
        SELECT 1
        FROM selection_v2_run_stages m
        WHERE m.subject_kind='outcome_run' AND m.subject_id=NEW.subject_id
          AND (
            (m.run_status='expected_wait'
             AND (SELECT COUNT(*) FROM selection_outcome_attempts a
                  WHERE a.outcome_run_id=m.subject_id)=0
             AND (SELECT COUNT(*) FROM selection_sample_outcomes o
                  WHERE o.outcome_run_id=m.subject_id)=0)
            OR
            (m.run_status='settled'
             AND (SELECT COUNT(*) FROM selection_outcome_attempts a
                  WHERE a.outcome_run_id=m.subject_id
                    AND a.result_code='settled')=1
             AND (SELECT COUNT(*) FROM selection_sample_outcomes o
                  WHERE o.outcome_run_id=m.subject_id)=1)
            OR
            (m.run_status='failed_retryable'
             AND (SELECT COUNT(*) FROM selection_outcome_attempts a
                  WHERE a.outcome_run_id=m.subject_id
                    AND a.result_code='error' AND a.retryable=1)=1
             AND (SELECT COUNT(*) FROM selection_sample_outcomes o
                  WHERE o.outcome_run_id=m.subject_id)=0)
            OR
            (m.run_status='failed_non_retryable'
             AND (SELECT COUNT(*) FROM selection_outcome_attempts a
                  WHERE a.outcome_run_id=m.subject_id
                    AND a.result_code='error' AND a.retryable=0)=1
             AND (SELECT COUNT(*) FROM selection_sample_outcomes o
                  WHERE o.outcome_run_id=m.subject_id)=0)
          )
    );
END;"#;

fn selection_v2_final_schema() -> SelectionV2SchemaResult<String> {
    let mut schema = SELECTION_V2_TRANSITIONAL_SCHEMA.to_owned();
    for (marker, replacement, field) in [
        (
            "CREATE TABLE IF NOT EXISTS selection_v2_recovery_envelopes",
            SELECTION_V2_FINAL_RECOVERY_ENVELOPES_TABLE,
            "final.recovery_envelopes",
        ),
        (
            "CREATE TABLE IF NOT EXISTS selection_outcome_attempts",
            SELECTION_V2_FINAL_OUTCOME_ATTEMPTS_TABLE,
            "final.outcome_attempts",
        ),
        (
            "CREATE TABLE IF NOT EXISTS selection_v2_run_stages",
            SELECTION_V2_FINAL_RUN_STAGES_TABLE,
            "final.run_stages",
        ),
    ] {
        let original = schema_statement_from(&schema, marker)
            .ok_or_else(|| SelectionV2SchemaError::SchemaMismatch {
                table: "selection_v2_table_set",
                column: field,
                expected: "exactly one transitional DDL statement",
                actual: "statement missing".to_owned(),
            })?
            .to_owned();
        schema = replace_schema_exact_once(schema, &original, replacement, field)?;
    }
    for (name, replacement, field) in [
        (
            "selection_v2_outcome_manifest_closure",
            SELECTION_V2_FINAL_OUTCOME_MANIFEST_TRIGGER,
            "final.outcome_manifest_trigger",
        ),
        (
            "selection_v2_outcome_receipt_closure",
            SELECTION_V2_FINAL_OUTCOME_RECEIPT_TRIGGER,
            "final.outcome_receipt_trigger",
        ),
    ] {
        let original = static_trigger_sql(name)?;
        schema = replace_schema_exact_once(schema, &original, replacement, field)?;
    }
    Ok(schema)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum SelectionV2CatalogObjectKind {
    Table,
    Index,
    Trigger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SelectionV2CatalogDdlPhase {
    Transitional,
    Final,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SelectionV2CatalogDdlStatement {
    pub(super) kind: SelectionV2CatalogObjectKind,
    pub(super) name: String,
    pub(super) exact_sql: String,
}

/// Expose the narrow, read-only checked-in DDL plan to the whole-database
/// BR-180 owner. Every statement is produced by the same schema and trigger
/// generators used by selection migration/verification; this is not a second
/// expected-SQL registry and grants no connection or mutation capability.
pub(super) fn selection_v2_catalog_ddl_plan(
    mode: SelectionV2StoreMode,
    phase: SelectionV2CatalogDdlPhase,
) -> SelectionV2SchemaResult<Vec<SelectionV2CatalogDdlStatement>> {
    let schema = match phase {
        SelectionV2CatalogDdlPhase::Transitional => SELECTION_V2_TRANSITIONAL_SCHEMA.to_owned(),
        SelectionV2CatalogDdlPhase::Final => selection_v2_final_schema()?,
    };
    let mut plan = Vec::with_capacity(70);

    for table in V2_TABLES {
        plan.push(SelectionV2CatalogDdlStatement {
            kind: SelectionV2CatalogObjectKind::Table,
            name: table.to_owned(),
            exact_sql: required_catalog_schema_statement(
                &schema,
                &format!("CREATE TABLE IF NOT EXISTS {table}"),
                table,
            )?,
        });
    }
    for index in V2_INDEXES {
        let statement = [
            format!("CREATE UNIQUE INDEX IF NOT EXISTS {index}"),
            format!("CREATE INDEX IF NOT EXISTS {index}"),
        ]
        .into_iter()
        .find_map(|marker| schema_statement_from(&schema, &marker).map(str::to_owned))
        .ok_or_else(|| SelectionV2SchemaError::SchemaMismatch {
            table: "selection_v2_table_set",
            column: "catalog_ddl_plan",
            expected: "checked-in selection index statement",
            actual: format!("missing index {index}"),
        })?;
        plan.push(SelectionV2CatalogDdlStatement {
            kind: SelectionV2CatalogObjectKind::Index,
            name: index.to_owned(),
            exact_sql: statement,
        });
    }
    for name in STATIC_TRIGGER_NAMES {
        plan.push(SelectionV2CatalogDdlStatement {
            kind: SelectionV2CatalogObjectKind::Trigger,
            name: name.to_owned(),
            exact_sql: static_trigger_sql_from(&schema, name)?,
        });
    }
    for (table, run_column, kind) in STAGE_MEMBERSHIPS {
        plan.push(SelectionV2CatalogDdlStatement {
            kind: SelectionV2CatalogObjectKind::Trigger,
            name: format!("selection_v2_{table}_stage_membership"),
            exact_sql: stage_membership_trigger_sql(table, run_column, kind),
        });
    }
    for table in V2_TABLES {
        let (update, delete) = append_only_trigger_sql(table);
        plan.push(SelectionV2CatalogDdlStatement {
            kind: SelectionV2CatalogObjectKind::Trigger,
            name: format!("{table}_deny_update"),
            exact_sql: update,
        });
        plan.push(SelectionV2CatalogDdlStatement {
            kind: SelectionV2CatalogObjectKind::Trigger,
            name: format!("{table}_deny_delete"),
            exact_sql: delete,
        });
    }
    for (table, base) in [
        (
            "selection_relation_attempts",
            "selection_v2_relation_symbol_isolation",
        ),
        (
            "selection_evaluation_attempts",
            "selection_v2_evaluation_symbol_isolation",
        ),
        ("selection_samples", "selection_v2_sample_symbol_isolation"),
    ] {
        let (mode_name, _) = mode_names(mode);
        plan.push(SelectionV2CatalogDdlStatement {
            kind: SelectionV2CatalogObjectKind::Trigger,
            name: format!("{base}_{mode_name}"),
            exact_sql: symbol_trigger_sql(mode, table, base),
        });
    }

    let identities = plan
        .iter()
        .map(|statement| (statement.kind, statement.name.as_str()))
        .collect::<BTreeSet<_>>();
    if plan.len() != 70 || identities.len() != 70 {
        return Err(SelectionV2SchemaError::SchemaMismatch {
            table: "selection_v2_table_set",
            column: "catalog_ddl_plan",
            expected: "exactly 70 unique checked-in selection objects",
            actual: format!("rows={},unique={}", plan.len(), identities.len()),
        });
    }
    Ok(plan)
}

fn required_catalog_schema_statement(
    schema: &str,
    marker: &str,
    object: &str,
) -> SelectionV2SchemaResult<String> {
    schema_statement_from(schema, marker)
        .map(str::to_owned)
        .ok_or_else(|| SelectionV2SchemaError::SchemaMismatch {
            table: "selection_v2_table_set",
            column: "catalog_ddl_plan",
            expected: "checked-in selection schema statement",
            actual: format!("missing object {object}"),
        })
}

/// Install the frozen final database-half catalog for repository unit tests.
///
/// This deliberately bypasses the production initialization gate: tests need
/// to exercise persistence against the exact target catalog while
/// `initialize_selection_v2_schema` remains fail-closed until the authoritative
/// audit/outcome-claim owner is release-enabled.
#[cfg(test)]
pub(super) fn install_selection_v2_final_database_half_for_test(
    conn: &mut SqliteConnection,
    mode: SelectionV2StoreMode,
) -> SelectionV2SchemaResult<()> {
    conn.batch_execute("PRAGMA foreign_keys = ON; PRAGMA synchronous = FULL;")?;
    conn.batch_execute(&selection_v2_final_schema()?)?;
    install_stage_membership_triggers(conn)?;
    install_append_only_triggers(conn)?;
    install_symbol_triggers(conn, mode)?;
    // Unit tests emulate the whole-database schema-version owner. Selection
    // production code never writes either identity PRAGMA.
    conn.batch_execute(
        "PRAGMA application_id = 1398035265;
         PRAGMA user_version = 1;",
    )?;
    verify_final_database_half_schema_contract(conn, mode)?;
    verify_integrity(conn)
}

/// Recheck the exact final catalog for unit tests without weakening the
/// production `IncompleteTarget` release gate.
#[cfg(test)]
pub(super) fn verify_selection_v2_final_database_half_for_test(
    conn: &mut SqliteConnection,
    mode: SelectionV2StoreMode,
) -> SelectionV2SchemaResult<()> {
    verify_pragmas(conn)?;
    verify_foreign_keys(conn)?;
    if classify_selection_v2_schema(conn)? != SelectionV2SchemaRevision::FinalDatabaseHalf {
        return Err(incomplete_target_error());
    }
    if detect_store_mode(conn)? != mode {
        return Err(SelectionV2SchemaError::StoreModeConflict);
    }
    verify_final_database_half_schema_contract(conn, mode)?;
    verify_integrity(conn)
}

fn install_stage_membership_triggers(conn: &mut SqliteConnection) -> QueryResult<()> {
    for (table, run_column, kind) in STAGE_MEMBERSHIPS {
        let sql = stage_membership_trigger_sql(table, run_column, kind);
        conn.batch_execute(&sql)?;
    }
    Ok(())
}

fn stage_membership_trigger_sql(table: &str, run_column: &str, kind: &str) -> String {
    let trigger = format!("selection_v2_{table}_stage_membership");
    format!(
        "CREATE TRIGGER IF NOT EXISTS {trigger} BEFORE INSERT ON {table} BEGIN \
         SELECT RAISE(ABORT, 'BR-174 domain row lacks matching recovery envelope') \
         WHERE NOT EXISTS (SELECT 1 FROM selection_v2_recovery_envelopes e \
                           WHERE e.stage_run_id=NEW.{run_column} \
                             AND e.subject_kind='{kind}'); \
         SELECT RAISE(ABORT, 'BR-174 manifest must be inserted last') \
         WHERE EXISTS (SELECT 1 FROM selection_v2_run_stages m \
                       WHERE m.subject_kind='{kind}' \
                         AND m.subject_id=NEW.{run_column}); \
         SELECT RAISE(ABORT, 'BR-174 receipted run is immutable') \
         WHERE EXISTS (SELECT 1 FROM selection_v2_commit_receipts r \
                       WHERE r.subject_kind='{kind}' \
                         AND r.subject_id=NEW.{run_column}); END;"
    )
}

fn install_append_only_triggers(conn: &mut SqliteConnection) -> QueryResult<()> {
    for table in V2_TABLES {
        let (update_trigger, delete_trigger) = append_only_trigger_sql(table);
        conn.batch_execute(&update_trigger)?;
        conn.batch_execute(&delete_trigger)?;
    }
    Ok(())
}

fn append_only_trigger_sql(table: &str) -> (String, String) {
    let update_trigger = format!(
        "CREATE TRIGGER IF NOT EXISTS {table}_deny_update \
         BEFORE UPDATE ON {table} BEGIN \
         SELECT RAISE(ABORT, 'BR-174 append-only UPDATE denied'); END;"
    );
    let delete_trigger = format!(
        "CREATE TRIGGER IF NOT EXISTS {table}_deny_delete \
         BEFORE DELETE ON {table} BEGIN \
         SELECT RAISE(ABORT, 'BR-174 append-only DELETE denied'); END;"
    );
    (update_trigger, delete_trigger)
}

fn install_symbol_triggers(
    conn: &mut SqliteConnection,
    mode: SelectionV2StoreMode,
) -> SelectionV2SchemaResult<()> {
    let (_, opposing_mode) = mode_names(mode);

    let opposing = diesel::sql_query(format!(
        "SELECT COUNT(*) AS count FROM sqlite_master \
         WHERE type='trigger' AND name LIKE 'selection_v2_%_symbol_isolation_{opposing_mode}'"
    ))
    .get_result::<CountRow>(conn)?;
    if opposing.count != 0 {
        return Err(SelectionV2SchemaError::StoreModeConflict);
    }

    let existing_predicate = symbol_invalid_predicate(mode, "canonical_stock_code");
    let existing = diesel::sql_query(format!(
        "SELECT COUNT(*) AS count FROM (\
            SELECT canonical_stock_code FROM selection_relation_attempts \
            UNION ALL SELECT canonical_stock_code FROM selection_evaluation_attempts \
            UNION ALL SELECT canonical_stock_code FROM selection_samples\
         ) symbols WHERE canonical_stock_code IS NOT NULL \
           AND ({existing_predicate})"
    ))
    .get_result::<CountRow>(conn)?;
    if existing.count != 0 {
        return Err(SelectionV2SchemaError::ExistingSymbolViolation);
    }

    for (table, trigger) in [
        (
            "selection_relation_attempts",
            "selection_v2_relation_symbol_isolation",
        ),
        (
            "selection_evaluation_attempts",
            "selection_v2_evaluation_symbol_isolation",
        ),
        ("selection_samples", "selection_v2_sample_symbol_isolation"),
    ] {
        let sql = symbol_trigger_sql(mode, table, trigger);
        conn.batch_execute(&sql)?;
    }
    Ok(())
}

fn mode_names(mode: SelectionV2StoreMode) -> (&'static str, &'static str) {
    match mode {
        SelectionV2StoreMode::Production => ("production", "test"),
        SelectionV2StoreMode::Test => ("test", "production"),
    }
}

fn symbol_invalid_predicate(mode: SelectionV2StoreMode, column: &str) -> String {
    match mode {
        SelectionV2StoreMode::Production => format!(
            "{column} LIKE 'TEST_CODE_%' OR length({column})<>6 \
             OR {column} GLOB '*[^0-9]*'"
        ),
        SelectionV2StoreMode::Test => {
            format!("{column} NOT GLOB 'TEST_CODE_[0-9][0-9][0-9][0-9][0-9][0-9]'")
        }
    }
}

fn symbol_trigger_sql(mode: SelectionV2StoreMode, table: &str, base_trigger: &str) -> String {
    let (mode_name, _) = mode_names(mode);
    let trigger = format!("{base_trigger}_{mode_name}");
    let invalid_predicate = symbol_invalid_predicate(mode, "NEW.canonical_stock_code");
    format!(
        "CREATE TRIGGER IF NOT EXISTS {trigger} BEFORE INSERT ON {table} \
         WHEN NEW.canonical_stock_code IS NOT NULL AND ({invalid_predicate}) BEGIN \
         SELECT RAISE(ABORT, 'BR-174 TEST_CODE/production symbol isolation'); END;"
    )
}

fn detect_store_mode(conn: &mut SqliteConnection) -> SelectionV2SchemaResult<SelectionV2StoreMode> {
    let production = diesel::sql_query(
        "SELECT COUNT(*) AS count FROM sqlite_master
         WHERE type='trigger' AND name LIKE 'selection_v2_%_symbol_isolation_production'",
    )
    .get_result::<CountRow>(conn)?;
    let test = diesel::sql_query(
        "SELECT COUNT(*) AS count FROM sqlite_master
         WHERE type='trigger' AND name LIKE 'selection_v2_%_symbol_isolation_test'",
    )
    .get_result::<CountRow>(conn)?;
    match (production.count, test.count) {
        (3, 0) => Ok(SelectionV2StoreMode::Production),
        (0, 3) => Ok(SelectionV2StoreMode::Test),
        _ => Err(SelectionV2SchemaError::TriggerMismatch {
            name: "symbol-isolation-mode-set".to_owned(),
        }),
    }
}

fn verify_trigger_registry(
    conn: &mut SqliteConnection,
    mode: SelectionV2StoreMode,
) -> SelectionV2SchemaResult<()> {
    let mut expected = Vec::new();
    for name in STATIC_TRIGGER_NAMES {
        expected.push((name.to_owned(), static_trigger_sql(name)?));
    }
    for (table, run_column, kind) in STAGE_MEMBERSHIPS {
        expected.push((
            format!("selection_v2_{table}_stage_membership"),
            stage_membership_trigger_sql(table, run_column, kind),
        ));
    }
    for table in V2_TABLES {
        let (update, delete) = append_only_trigger_sql(table);
        expected.push((format!("{table}_deny_update"), update));
        expected.push((format!("{table}_deny_delete"), delete));
    }
    for (table, base) in [
        (
            "selection_relation_attempts",
            "selection_v2_relation_symbol_isolation",
        ),
        (
            "selection_evaluation_attempts",
            "selection_v2_evaluation_symbol_isolation",
        ),
        ("selection_samples", "selection_v2_sample_symbol_isolation"),
    ] {
        let (mode_name, _) = mode_names(mode);
        expected.push((
            format!("{base}_{mode_name}"),
            symbol_trigger_sql(mode, table, base),
        ));
    }

    let actual_count = diesel::sql_query(
        "SELECT COUNT(*) AS count FROM sqlite_master
         WHERE type='trigger' AND tbl_name IN (
            'selection_source_batch_attempts','selection_source_facts_v2',
            'selection_source_fact_attempts','selection_relation_attempts',
            'selection_evaluation_attempts','selection_samples',
            'selection_rejections','selection_sample_outcomes',
            'selection_outcome_attempts','selection_v2_recovery_envelopes',
            'selection_v2_run_stages','selection_v2_commit_receipts'
         )",
    )
    .get_result::<CountRow>(conn)?;
    if actual_count.count != i64::try_from(expected.len()).expect("trigger count fits i64") {
        return Err(SelectionV2SchemaError::TriggerMismatch {
            name: "managed-trigger-count".to_owned(),
        });
    }

    for (name, expected_sql) in expected {
        let actual = diesel::sql_query(
            "SELECT sql AS sql FROM sqlite_master WHERE type='trigger' AND name=?1",
        )
        .bind::<Text, _>(&name)
        .get_result::<TriggerSqlRow>(conn)
        .optional()?
        .ok_or_else(|| SelectionV2SchemaError::TriggerMismatch { name: name.clone() })?;
        if normalize_trigger_sql(&actual.sql) != normalize_trigger_sql(&expected_sql) {
            return Err(SelectionV2SchemaError::TriggerMismatch { name });
        }
    }
    Ok(())
}

fn static_trigger_sql(name: &str) -> SelectionV2SchemaResult<String> {
    static_trigger_sql_from(SELECTION_V2_TRANSITIONAL_SCHEMA, name)
}

fn static_trigger_sql_from(schema: &str, name: &str) -> SelectionV2SchemaResult<String> {
    let marker = format!("CREATE TRIGGER IF NOT EXISTS {name}");
    let start = schema
        .find(&marker)
        .ok_or_else(|| SelectionV2SchemaError::TriggerMismatch {
            name: name.to_owned(),
        })?;
    let remainder = &schema[start..];
    let end = remainder
        .find("\nEND;")
        .ok_or_else(|| SelectionV2SchemaError::TriggerMismatch {
            name: name.to_owned(),
        })?
        + "\nEND;".len();
    Ok(remainder[..end].to_owned())
}

fn normalize_trigger_sql(sql: &str) -> String {
    normalize_schema_object_sql(sql)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_normalization_preserves_quoted_literal_and_identifier_bytes() {
        let canonical = normalize_schema_object_sql(
            r#"CREATE TABLE "quoted name" (
                status TEXT CHECK(status='available value')
            );"#,
        );
        let changed_literal = normalize_schema_object_sql(
            r#"CREATE TABLE "quoted name" (
                status TEXT CHECK(status='availablevalue')
            );"#,
        );
        let changed_identifier = normalize_schema_object_sql(
            r#"CREATE TABLE "quotedname" (
                status TEXT CHECK(status='available value')
            );"#,
        );

        assert_ne!(
            canonical, changed_literal,
            "whitespace inside a SQL string literal is schema data"
        );
        assert_ne!(
            canonical, changed_identifier,
            "whitespace inside a quoted identifier is schema data"
        );
    }

    #[test]
    fn schema_normalization_does_not_strip_if_not_exists_inside_malicious_quotes() {
        let canonical = normalize_schema_object_sql(
            r#"CREATE TABLE quoted_guard (
                literal TEXT CHECK(literal='IF NOT EXISTS'),
                "IF NOT EXISTS" TEXT
            )"#,
        );
        let changed_literal = normalize_schema_object_sql(
            r#"CREATE TABLE quoted_guard (
                literal TEXT CHECK(literal=''),
                "IF NOT EXISTS" TEXT
            )"#,
        );
        let changed_identifier = normalize_schema_object_sql(
            r#"CREATE TABLE quoted_guard (
                literal TEXT CHECK(literal='IF NOT EXISTS'),
                "" TEXT
            )"#,
        );

        assert_ne!(canonical, changed_literal);
        assert_ne!(canonical, changed_identifier);
    }

    #[test]
    fn startup_v2_audit_classifier_covers_every_registered_v2_phase() {
        let recorded_at = "2026-07-28T10:00:00+08:00"
            .parse()
            .expect("fixed audit timestamp");
        for phase in [
            SelectionAuditPhase::V2ConfigActivationPrepared,
            SelectionAuditPhase::V2ConfigActivationCommitted,
            SelectionAuditPhase::V2IngressPrepared,
            SelectionAuditPhase::V2IngressCommitted,
            SelectionAuditPhase::V2GenerationPrepared,
            SelectionAuditPhase::V2GenerationCommitted,
            SelectionAuditPhase::V2OutcomePrepared,
            SelectionAuditPhase::V2OutcomeCommitted,
            SelectionAuditPhase::V2BoardBindingAuditPrepared,
            SelectionAuditPhase::V2BoardBindingAuditCommitted,
            SelectionAuditPhase::V2GateDCanaryVerified,
        ] {
            let record =
                SelectionAuditRecord::new(phase, "TEST_CODE_subject", "content", recorded_at);
            assert!(
                selection_v2_audit_record_is_v2(&record),
                "{phase:?} must block absent startup"
            );
        }
    }

    #[test]
    fn pre_amendment_fixture_is_independent_frozen_bytes() {
        assert_eq!(
            hex::encode(Sha256::digest(PRE_AMENDMENT_SCHEMA.as_bytes())),
            PRE_AMENDMENT_STATIC_SCHEMA_SHA256
        );
        assert!(PRE_AMENDMENT_SCHEMA.contains("payload_schema='generation-stage-v2'"));
        assert!(!PRE_AMENDMENT_SCHEMA.contains("trading_date_vector_json"));
        assert!(!PRE_AMENDMENT_SCHEMA.contains("d2_due_date TEXT NOT NULL"));
        assert!(!PRE_AMENDMENT_SCHEMA.contains("d4_due_date TEXT NOT NULL"));
    }

    #[test]
    fn catalog_ddl_plan_executes_the_same_transitional_and_final_sql_as_installers() {
        for mode in [SelectionV2StoreMode::Production, SelectionV2StoreMode::Test] {
            for phase in [
                SelectionV2CatalogDdlPhase::Transitional,
                SelectionV2CatalogDdlPhase::Final,
            ] {
                let plan =
                    selection_v2_catalog_ddl_plan(mode, phase).expect("build checked-in DDL plan");
                assert_eq!(plan.len(), 70);
                assert_eq!(
                    plan.iter()
                        .filter(|statement| {
                            statement.kind == SelectionV2CatalogObjectKind::Table
                        })
                        .count(),
                    12
                );
                assert_eq!(
                    plan.iter()
                        .filter(|statement| {
                            statement.kind == SelectionV2CatalogObjectKind::Index
                        })
                        .count(),
                    5
                );
                assert_eq!(
                    plan.iter()
                        .filter(|statement| {
                            statement.kind == SelectionV2CatalogObjectKind::Trigger
                        })
                        .count(),
                    53
                );

                let mut planned =
                    SqliteConnection::establish(":memory:").expect("isolated planned sqlite");
                planned
                    .batch_execute("PRAGMA foreign_keys = ON; PRAGMA synchronous = FULL;")
                    .expect("configure planned sqlite");
                for statement in &plan {
                    planned
                        .batch_execute(&statement.exact_sql)
                        .unwrap_or_else(|error| {
                            panic!(
                                "execute planned {:?} {}: {error}",
                                statement.kind, statement.name
                            )
                        });
                }

                let mut installed =
                    SqliteConnection::establish(":memory:").expect("isolated installed sqlite");
                match phase {
                    SelectionV2CatalogDdlPhase::Transitional => {
                        install_transitional_current_table_set(&mut installed, mode);
                    }
                    SelectionV2CatalogDdlPhase::Final => {
                        install_final_database_half(&mut installed, mode);
                    }
                }

                assert_eq!(
                    managed_schema_catalog_rows(&mut planned).expect("capture planned catalog"),
                    managed_schema_catalog_rows(&mut installed).expect("capture installed catalog"),
                    "DDL plan must remain byte-derived from the installer for {mode:?}/{phase:?}"
                );
            }
        }
    }

    #[test]
    fn final_catalog_plan_enforces_expected_wait_without_attempt_or_outcome_rows() {
        let plan = selection_v2_catalog_ddl_plan(
            SelectionV2StoreMode::Test,
            SelectionV2CatalogDdlPhase::Final,
        )
        .expect("build final checked-in DDL plan");
        let trigger = |name: &str| {
            compact_sql(
                &plan
                    .iter()
                    .find(|statement| {
                        statement.kind == SelectionV2CatalogObjectKind::Trigger
                            && statement.name == name
                    })
                    .unwrap_or_else(|| panic!("final plan contains trigger {name}"))
                    .exact_sql,
            )
        };

        let manifest = trigger("selection_v2_outcome_manifest_closure");
        assert!(manifest.contains(
            "NEW.run_status='expected_wait'AND(SELECTCOUNT(*)FROMselection_outcome_attemptsaWHEREa.outcome_run_id=NEW.subject_id)<>0"
        ));
        assert!(manifest.contains(
            "NEW.run_status<>'expected_wait'AND(SELECTCOUNT(*)FROMselection_outcome_attemptsaWHEREa.outcome_run_id=NEW.subject_id)<>1"
        ));
        assert!(manifest.contains("WHENNEW.run_status='expected_wait'THEN1"));
        assert!(
            !manifest.contains("NEW.run_status='expected_wait'ANDa.result_code='expected_wait'")
        );

        let receipt = trigger("selection_v2_outcome_receipt_closure");
        assert!(receipt.contains(
            "m.run_status='expected_wait'AND(SELECTCOUNT(*)FROMselection_outcome_attemptsaWHEREa.outcome_run_id=m.subject_id)=0AND(SELECTCOUNT(*)FROMselection_sample_outcomesoWHEREo.outcome_run_id=m.subject_id)=0"
        ));
        assert!(!receipt.contains("m.run_status='expected_wait'ANDa.result_code='expected_wait'"));
    }

    fn install_pre_amendment_table_set(conn: &mut SqliteConnection, mode: SelectionV2StoreMode) {
        conn.batch_execute("PRAGMA foreign_keys = ON; PRAGMA synchronous = FULL;")
            .unwrap();
        conn.batch_execute(PRE_AMENDMENT_SCHEMA)
            .expect("install all twelve real old tables, indexes and static triggers");
        install_stage_membership_triggers(conn).expect("install old stage-membership triggers");
        install_append_only_triggers(conn).expect("install old append-only triggers");
        install_symbol_triggers(conn, mode)
            .expect("install old physical symbol-isolation triggers");
        verify_pre_amendment_schema_contract(conn).expect("fixture exactly matches old golden");
        verify_trigger_registry(conn, mode).expect("fixture has the complete old trigger registry");
    }

    fn install_transitional_current_table_set(
        conn: &mut SqliteConnection,
        mode: SelectionV2StoreMode,
    ) {
        conn.batch_execute("PRAGMA foreign_keys = ON; PRAGMA synchronous = FULL;")
            .unwrap();
        conn.batch_execute(SELECTION_V2_TRANSITIONAL_SCHEMA)
            .expect("install frozen four-payload transitional schema");
        install_stage_membership_triggers(conn).expect("install stage-membership triggers");
        install_append_only_triggers(conn).expect("install append-only triggers");
        install_symbol_triggers(conn, mode).expect("install symbol-isolation triggers");
        verify_transitional_current_schema_contract(conn)
            .expect("fixture matches the transitional contract");
        verify_trigger_registry(conn, mode).expect("fixture has the complete trigger registry");
    }

    fn install_final_database_half(conn: &mut SqliteConnection, mode: SelectionV2StoreMode) {
        install_unverified_final_database_half(
            conn,
            mode,
            &selection_v2_final_schema().expect("build final checked DDL"),
        );
        verify_final_database_half_schema_contract(conn, mode)
            .expect("fixture matches the exact final database-half catalog");
    }

    fn install_unverified_final_database_half(
        conn: &mut SqliteConnection,
        mode: SelectionV2StoreMode,
        schema: &str,
    ) {
        conn.batch_execute("PRAGMA foreign_keys = ON; PRAGMA synchronous = FULL;")
            .unwrap();
        conn.batch_execute(schema)
            .expect("install final five-payload schema");
        install_stage_membership_triggers(conn).expect("install stage-membership triggers");
        install_append_only_triggers(conn).expect("install append-only triggers");
        install_symbol_triggers(conn, mode).expect("install symbol-isolation triggers");
        // Tests emulate the global schema-version owner. Selection production
        // code never writes either whole-database identity PRAGMA.
        conn.batch_execute(
            "PRAGMA application_id = 1398035265;
             PRAGMA user_version = 1;",
        )
        .expect("emulate generation-1 global owner");
    }

    fn connection(mode: SelectionV2StoreMode) -> SqliteConnection {
        let mut conn = SqliteConnection::establish(":memory:").expect("isolated sqlite");
        install_transitional_current_table_set(&mut conn, mode);
        conn
    }

    fn inspect_read_only(
        conn: &mut SqliteConnection,
    ) -> SelectionV2SchemaResult<SelectionV2MigrationPreflight> {
        conn.batch_execute("PRAGMA query_only = ON;").unwrap();
        inspect_selection_v2_migration(conn)
    }

    fn insert_envelope(
        conn: &mut SqliteConnection,
        run_id: &str,
        kind: &str,
    ) -> QueryResult<usize> {
        let payload_schema = match kind {
            "config_activation" => "config-activation-stage-v1",
            "ingress_run" => "source-ingress-stage-v2",
            "generation_run" => "generation-stage-v3",
            "outcome_run" => "outcome-stage-v2",
            _ => unreachable!("test envelope kind"),
        };
        let payload_json = match kind {
            "config_activation" => {
                r#"{"domain":"stock_analysis.br174.config_activation_stage.v1"}"#
            }
            "ingress_run" => r#"{"domain":"stock_analysis.br174.source_ingress_stage.v2"}"#,
            "generation_run" => r#"{"domain":"stock_analysis.br174.generation_stage.v3"}"#,
            "outcome_run" => r#"{"domain":"stock_analysis.br174.outcome_stage.v2"}"#,
            _ => unreachable!("test envelope kind"),
        };
        let activation_run_id = if kind == "config_activation" {
            run_id
        } else {
            "activation"
        };
        diesel::sql_query(
            "INSERT INTO selection_v2_recovery_envelopes (
                stage_run_id,subject_kind,logical_subject_key,payload_schema,payload_json,
                payload_json_hash,in_memory_payload_hash,config_activation_run_id,config_hash,
                enveloped_at,content_hash
             ) VALUES (?1,?2,?1,?3,?4,'payload-hash','memory-hash',
                       ?5,'config-hash','2026-07-28T00:00:00Z','envelope-hash')",
        )
        .bind::<diesel::sql_types::Text, _>(run_id)
        .bind::<diesel::sql_types::Text, _>(kind)
        .bind::<diesel::sql_types::Text, _>(payload_schema)
        .bind::<diesel::sql_types::Text, _>(payload_json)
        .bind::<diesel::sql_types::Text, _>(activation_run_id)
        .execute(conn)
    }

    fn insert_source_batch_attempt(
        conn: &mut SqliteConnection,
        attempt_id: &str,
        feed_identity: &str,
        status_kind: &str,
        record_count: Option<i32>,
    ) -> QueryResult<usize> {
        diesel::sql_query(
            "INSERT INTO selection_source_batch_attempts (
                source_batch_attempt_id,ingress_run_id,config_activation_run_id,config_hash,
                generation_market_date,registered_feed_identity,registered_feed_snapshot_hash,
                request_hash,request_evidence_json,request_evidence_hash,
                feed_attempt_content_hash,status_kind,record_count,provider,source,
                source_at,observed_at,batch_id,batch_content_hash,failed_stage,reason_code,
                retryable,available_evidence_json,available_evidence_hash,error_detail_json,
                error_detail_hash,error_fingerprint,attempted_at,content_hash
             ) VALUES (
                ?1,'ingress-count-contract','activation','config-hash','2026-07-28',
                ?2,'feed-snapshot','request-hash','{}','request-evidence-hash',
                'feed-attempt-hash',?3,?4,
                CASE WHEN ?3='unavailable' THEN NULL ELSE 'provider' END,
                CASE WHEN ?3='unavailable' THEN NULL ELSE 'source' END,
                CASE WHEN ?3='unavailable' THEN NULL ELSE '2026-07-28T00:00:00Z' END,
                CASE WHEN ?3='unavailable' THEN NULL ELSE '2026-07-28T00:00:01Z' END,
                CASE WHEN ?3='unavailable' THEN NULL ELSE 'batch-id' END,
                CASE WHEN ?3='unavailable' THEN NULL ELSE 'batch-content-hash' END,
                CASE WHEN ?3='unavailable' THEN 'fetch' ELSE NULL END,
                CASE WHEN ?3='unavailable' THEN 'transport_failure' ELSE NULL END,
                CASE WHEN ?3='unavailable' THEN 1 ELSE NULL END,
                CASE WHEN ?3='unavailable' THEN NULL ELSE '{}' END,
                CASE WHEN ?3='unavailable' THEN NULL ELSE 'available-evidence-hash' END,
                CASE WHEN ?3='unavailable' THEN '{}' ELSE NULL END,
                CASE WHEN ?3='unavailable' THEN 'error-detail-hash' ELSE NULL END,
                CASE WHEN ?3='unavailable' THEN 'error-fingerprint' ELSE NULL END,
                '2026-07-28T00:00:02Z','attempt-content-hash'
             )",
        )
        .bind::<Text, _>(attempt_id)
        .bind::<Text, _>(feed_identity)
        .bind::<Text, _>(status_kind)
        .bind::<diesel::sql_types::Nullable<Integer>, _>(record_count)
        .execute(conn)
    }

    fn insert_fact(
        conn: &mut SqliteConnection,
        key: &str,
        ingress_run: &str,
        decision: &str,
    ) -> QueryResult<usize> {
        let (reason, retryable) = if decision == "admitted" {
            (None::<String>, None::<i32>)
        } else {
            (Some("stale".to_owned()), Some(0))
        };
        diesel::sql_query(
            "INSERT INTO selection_source_facts_v2 (
                source_fact_key,event_id,payload_schema,config_activation_run_id,config_hash,
                generation_market_date,provider_source,item_id,title,summary,content,publisher,
                canonical_url,published_at,instruments_json,topics_json,language,
                record_provider,record_source,record_source_at,record_observed_at,
                record_batch_id,record_batch_content_hash,provider_content_hash,
                first_ingress_run_id,ingress_gate_version,ingress_gate_input_json,
                ingress_gate_input_hash,ingress_decision,ingress_reason_code,ingress_retryable,
                ingress_gate_receipt_json,ingress_gate_receipt_hash,content_hash
             ) VALUES (
                ?1,'event','global-news-source-fact-v2','activation','config-hash',
                '2026-07-28','eastmoney','item','title',NULL,NULL,'publisher',
                'https://example.invalid/item','2026-07-28T00:00:00Z','[]','[]','zh',
                'eastmoney','global-news','2026-07-28T00:00:00Z',
                '2026-07-28T00:00:01Z','batch-provider','batch-content','provider-content',
                ?2,'ingress-gate-v1','{}','gate-input',?3,?4,?5,
                '{}','gate-receipt','fact-hash'
             )",
        )
        .bind::<diesel::sql_types::Text, _>(key)
        .bind::<diesel::sql_types::Text, _>(ingress_run)
        .bind::<diesel::sql_types::Text, _>(decision)
        .bind::<diesel::sql_types::Nullable<diesel::sql_types::Text>, _>(reason)
        .bind::<diesel::sql_types::Nullable<Integer>, _>(retryable)
        .execute(conn)
    }

    fn insert_resolved_direct_relation(
        conn: &mut SqliteConnection,
        attempt_id: &str,
        relation_key: &str,
        generation_run_id: &str,
        source_fact_key: &str,
        canonical_stock_code: &str,
    ) -> QueryResult<usize> {
        insert_resolved_direct_relation_with_evidence(
            conn,
            attempt_id,
            relation_key,
            generation_run_id,
            source_fact_key,
            canonical_stock_code,
            None,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_resolved_direct_relation_with_evidence(
        conn: &mut SqliteConnection,
        attempt_id: &str,
        relation_key: &str,
        generation_run_id: &str,
        source_fact_key: &str,
        canonical_stock_code: &str,
        available_evidence_json: Option<&str>,
        available_evidence_hash: Option<&str>,
    ) -> QueryResult<usize> {
        diesel::sql_query(
            "INSERT INTO selection_relation_attempts (
                relation_attempt_id,relation_key,generation_run_id,source_fact_key,event_id,
                chain_id,config_activation_run_id,config_hash,relation_schema_version,
                relation_kind,relation_source_identity_json,relation_source_identity_hash,
                typed_binding_state_json,typed_binding_state_hash,request_hash,result_code,
                request_evidence_json,request_evidence_hash,
                failed_stage,retryable,raw_identity_json,raw_identity_hash,
                canonical_stock_code,canonical_stock_name,canonical_market,
                artifact_content_hash,binding_audit_hash,provider_board_kind,
                provider_board_code,provider_board_name,provider_source,provider_source_at,
                provider_observed_at,provider_batch_id,provider_batch_content_hash,
                actual_constituent_count,available_evidence_json,available_evidence_hash,
                error_detail_json,error_detail_hash,error_fingerprint,attempted_at,content_hash
             ) VALUES (
                ?1,?2,?3,?4,'event','chain','activation','config-hash',
                'event-relation-v2','direct_mention','{}','identity-hash',
                '{\"state\":\"direct_not_applicable\"}',
                'binding-state-hash',NULL,'resolved',NULL,NULL,NULL,NULL,'{}','raw-hash',
                ?5,'Test Security','SZ',NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,
                ?6,?7,NULL,NULL,NULL,'2026-07-28T00:00:03Z','relation-hash'
             )",
        )
        .bind::<Text, _>(attempt_id)
        .bind::<Text, _>(relation_key)
        .bind::<Text, _>(generation_run_id)
        .bind::<Text, _>(source_fact_key)
        .bind::<Text, _>(canonical_stock_code)
        .bind::<diesel::sql_types::Nullable<Text>, _>(available_evidence_json)
        .bind::<diesel::sql_types::Nullable<Text>, _>(available_evidence_hash)
        .execute(conn)
    }

    fn insert_rejection(
        conn: &mut SqliteConnection,
        ordinal: i32,
        content_hash: &str,
    ) -> QueryResult<usize> {
        diesel::sql_query(
            "INSERT INTO selection_rejections (
                sample_key,ordinal,generation_run_id,reason_code,rule_id,retryable,
                structured_detail_json,structured_detail_hash,provider,source,source_at,
                observed_at,batch_id,batch_content_hash,created_at,content_hash
             ) VALUES (
                'TEST_CODE_SAMPLE',?1,'generation-sample','trend_alignment_failed',
                'admission-v1.trend-alignment-failed',0,
                '{\"kind\":\"trend_alignment_failed\"}','detail-hash',
                NULL,NULL,NULL,NULL,NULL,NULL,
                '2026-07-28T00:00:03Z',?2
             )",
        )
        .bind::<Integer, _>(ordinal)
        .bind::<Text, _>(content_hash)
        .execute(conn)
    }

    fn insert_outcome_attempt(
        conn: &mut SqliteConnection,
        run_id: &str,
        result: &str,
    ) -> QueryResult<usize> {
        match result {
            "expected_wait" => conn.batch_execute(&format!(
                "INSERT INTO selection_outcome_attempts (
                    outcome_attempt_id,sample_key,phase,stored_due_date,outcome_run_id,
                    request_hash,request_evidence_json,request_evidence_hash,
                    result_code,reason_code,retryable,provider,source,source_at,
                    observed_at,batch_id,batch_content_hash,available_evidence_json,
                    available_evidence_hash,error_detail_json,error_detail_hash,error_fingerprint,
                    settled_outcome_content_hash,attempted_at,content_hash
                 ) VALUES (
                    'attempt-{run_id}','TEST_CODE_SAMPLE','t0_close','2026-07-28','{run_id}',
                    NULL,NULL,NULL,'expected_wait','market_session_unsettled',NULL,NULL,NULL,NULL,
                    NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,
                    '2026-07-28T00:00:02Z','attempt-hash'
                 );"
            )),
            _ => unreachable!("test fixture result"),
        }?;
        Ok(1)
    }

    fn insert_settled_attempt(conn: &mut SqliteConnection, run_id: &str) -> QueryResult<usize> {
        conn.batch_execute(&format!(
            "INSERT INTO selection_outcome_attempts (
                outcome_attempt_id,sample_key,phase,stored_due_date,outcome_run_id,
                request_hash,request_evidence_json,request_evidence_hash,
                result_code,reason_code,retryable,provider,source,source_at,
                observed_at,batch_id,batch_content_hash,available_evidence_json,
                available_evidence_hash,error_detail_json,error_detail_hash,error_fingerprint,
                settled_outcome_content_hash,attempted_at,content_hash
             ) VALUES (
                'attempt-{run_id}','TEST_CODE_SAMPLE','t0_close','2026-07-28','{run_id}',
                'request-hash','{{}}','request-evidence-hash',
                'settled',NULL,NULL,'magic-tdx','daily-bars',
                '2026-07-28T07:00:00Z','2026-07-28T07:00:01Z','outcome-batch',
                'outcome-batch-hash','{{\"evidence_kind\":\"outcome_daily_bars\"}}',
                'evidence-hash',NULL,NULL,NULL,'outcome-hash',
                '2026-07-28T07:00:02Z','attempt-hash'
             );"
        ))?;
        Ok(1)
    }

    fn insert_final_outcome_envelope(conn: &mut SqliteConnection, run_id: &str) {
        diesel::sql_query(
            "INSERT INTO selection_v2_recovery_envelopes (
                stage_run_id,subject_kind,logical_subject_key,payload_schema,payload_json,
                payload_json_hash,in_memory_payload_hash,config_activation_run_id,config_hash,
                enveloped_at,content_hash
             ) VALUES (
                ?1,'outcome_run',?1,'outcome-stage-v3',
                '{\"domain\":\"stock_analysis.br174.outcome_stage.v3\"}',
                'payload-hash','memory-hash','activation','config-hash',
                '2026-07-28T00:00:00Z','envelope-hash'
             )",
        )
        .bind::<Text, _>(run_id)
        .execute(conn)
        .expect("insert final outcome envelope");
    }

    fn insert_final_outcome_attempt(
        conn: &mut SqliteConnection,
        run_id: &str,
        result_code: &str,
        transport_attempts_json: Option<&str>,
        transport_attempts_hash: Option<&str>,
    ) -> QueryResult<usize> {
        let settled = result_code == "settled";
        diesel::sql_query(
            "INSERT INTO selection_outcome_attempts (
                outcome_attempt_id,sample_key,phase,stored_due_date,outcome_run_id,
                request_hash,request_evidence_json,request_evidence_hash,
                transport_attempts_json,transport_attempts_hash,
                result_code,reason_code,retryable,provider,source,source_at,observed_at,
                batch_id,batch_content_hash,available_evidence_json,available_evidence_hash,
                error_detail_json,error_detail_hash,error_fingerprint,
                settled_outcome_content_hash,attempted_at,content_hash
             ) VALUES (
                'attempt-' || ?1,'TEST_CODE_SAMPLE','t0_close','2026-07-28',?1,
                CASE WHEN ?2 THEN 'request-hash' ELSE NULL END,
                CASE WHEN ?2 THEN '{}' ELSE NULL END,
                CASE WHEN ?2 THEN 'request-evidence-hash' ELSE NULL END,
                ?3,?4,?5,
                CASE WHEN ?2 THEN NULL ELSE 'market_session_unsettled' END,
                NULL,
                CASE WHEN ?2 THEN 'magic-tdx' ELSE NULL END,
                CASE WHEN ?2 THEN 'daily-bars' ELSE NULL END,
                CASE WHEN ?2 THEN '2026-07-28T07:00:00Z' ELSE NULL END,
                CASE WHEN ?2 THEN '2026-07-28T07:00:01Z' ELSE NULL END,
                CASE WHEN ?2 THEN 'outcome-batch' ELSE NULL END,
                CASE WHEN ?2 THEN 'outcome-batch-hash' ELSE NULL END,
                CASE WHEN ?2 THEN '{}' ELSE NULL END,
                CASE WHEN ?2 THEN 'available-evidence-hash' ELSE NULL END,
                NULL,NULL,NULL,
                CASE WHEN ?2 THEN 'outcome-hash' ELSE NULL END,
                '2026-07-28T07:00:02Z','attempt-hash'
             )",
        )
        .bind::<Text, _>(run_id)
        .bind::<diesel::sql_types::Bool, _>(settled)
        .bind::<diesel::sql_types::Nullable<Text>, _>(transport_attempts_json)
        .bind::<diesel::sql_types::Nullable<Text>, _>(transport_attempts_hash)
        .bind::<Text, _>(result_code)
        .execute(conn)
    }

    fn insert_manifest(
        conn: &mut SqliteConnection,
        subject_id: &str,
        kind: &str,
        status: &str,
        source_fact_key: Option<&str>,
        expected_count: i32,
    ) -> QueryResult<usize> {
        let activation_run_id = if kind == "config_activation" {
            subject_id
        } else {
            "activation"
        };
        let (
            snapshot,
            activation_content,
            activation_file,
            effective,
            valid_from,
            expires_at,
            revision,
            legacy,
            market_date,
            aggregator_at,
            ingress_hash,
        ) = if kind == "config_activation" {
            (
                Some("snapshot"),
                Some("activation-content"),
                Some("activation-file"),
                Some("2026-07-28T00:00:00Z"),
                Some("2026-07-28"),
                Some("2027-07-28"),
                Some("revision"),
                Some("legacy"),
                None,
                None,
                None,
            )
        } else if kind == "ingress_run" {
            (
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some("2026-07-28"),
                Some("2026-07-28T00:00:01Z"),
                Some("source-batch-hash"),
            )
        } else if kind == "generation_run" {
            (
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some("2026-07-28"),
                None,
                None,
            )
        } else {
            unreachable!("non-outcome fixture manifest")
        };
        diesel::sql_query(
            "INSERT INTO selection_v2_run_stages (
                subject_id,subject_kind,logical_subject_key,in_memory_payload_hash,
                prepared_record_hash,expected_staged_row_count,staged_db_content_hash,
                recovery_envelope_content_hash,run_status,source_fact_key,
                config_activation_run_id,config_hash,config_snapshot_json_hash,
                config_activation_content_hash,config_activation_file_content_hash,
                config_effective_from,artifact_valid_from,artifact_expires_at,
                executable_revision,legacy_cutover_snapshot_hash,generation_market_date,
                aggregator_observed_at,ingress_source_batch_content_hash,outcome_phase,
                stored_due_date,staged_at,manifest_content_hash
             ) VALUES (
                ?1,?2,?1,'memory-hash','prepared-hash',?3,'db-hash',
                'envelope-hash',?4,?5,?6,'config-hash',?7,?8,?9,?10,?11,?12,
                ?13,?14,?15,?16,?17,NULL,NULL,'2026-07-28T00:00:03Z','manifest-hash'
             )",
        )
        .bind::<Text, _>(subject_id)
        .bind::<Text, _>(kind)
        .bind::<Integer, _>(expected_count)
        .bind::<Text, _>(status)
        .bind::<diesel::sql_types::Nullable<Text>, _>(source_fact_key)
        .bind::<Text, _>(activation_run_id)
        .bind::<diesel::sql_types::Nullable<Text>, _>(snapshot)
        .bind::<diesel::sql_types::Nullable<Text>, _>(activation_content)
        .bind::<diesel::sql_types::Nullable<Text>, _>(activation_file)
        .bind::<diesel::sql_types::Nullable<Text>, _>(effective)
        .bind::<diesel::sql_types::Nullable<Text>, _>(valid_from)
        .bind::<diesel::sql_types::Nullable<Text>, _>(expires_at)
        .bind::<diesel::sql_types::Nullable<Text>, _>(revision)
        .bind::<diesel::sql_types::Nullable<Text>, _>(legacy)
        .bind::<diesel::sql_types::Nullable<Text>, _>(market_date)
        .bind::<diesel::sql_types::Nullable<Text>, _>(aggregator_at)
        .bind::<diesel::sql_types::Nullable<Text>, _>(ingress_hash)
        .execute(conn)
    }

    fn insert_receipt(
        conn: &mut SqliteConnection,
        subject_id: &str,
        kind: &str,
    ) -> QueryResult<usize> {
        diesel::sql_query(
            "INSERT INTO selection_v2_commit_receipts (
                subject_kind,subject_id,logical_subject_key,in_memory_payload_hash,
                recovery_envelope_content_hash,prepared_audit_hash,
                run_manifest_content_hash,staged_db_content_hash,committed_audit_hash,
                committed_at,content_hash
             ) VALUES (
                ?1,?2,?2,'memory-hash','envelope-hash','prepared-hash',
                'manifest-hash','db-hash','audit-hash','2026-07-28T00:00:04Z',
                'receipt-hash'
             )",
        )
        .bind::<Text, _>(kind)
        .bind::<Text, _>(subject_id)
        .execute(conn)
    }

    fn insert_activation_chain(conn: &mut SqliteConnection) {
        insert_envelope(conn, "activation", "config_activation").unwrap();
        insert_manifest(
            conn,
            "activation",
            "config_activation",
            "activated",
            None,
            1,
        )
        .unwrap();
        insert_receipt(conn, "activation", "config_activation").unwrap();
    }

    fn stage_sample(
        conn: &mut SqliteConnection,
        decision: &str,
        rejection_count: i32,
        rejection_hashes_json: &str,
    ) {
        insert_activation_chain(conn);
        insert_envelope(conn, "ingress-sample", "ingress_run").unwrap();
        conn.batch_execute(
            "INSERT INTO selection_source_batch_attempts (
                source_batch_attempt_id,ingress_run_id,config_activation_run_id,config_hash,
                generation_market_date,registered_feed_identity,registered_feed_snapshot_hash,
                request_hash,request_evidence_json,request_evidence_hash,
                feed_attempt_content_hash,status_kind,record_count,provider,source,
                source_at,observed_at,batch_id,batch_content_hash,failed_stage,reason_code,
                retryable,available_evidence_json,available_evidence_hash,error_detail_json,
                error_detail_hash,error_fingerprint,attempted_at,content_hash
             ) VALUES (
                'batch-sample','ingress-sample','activation','config-hash','2026-07-28',
                'feed','feed-snapshot','request','{}','request-evidence',
                'feed-attempt','available',1,
                'eastmoney','global-news','2026-07-28T00:00:00Z',
                '2026-07-28T00:00:01Z','batch-provider','batch-content',
                NULL,NULL,NULL,'{}','available-hash',NULL,NULL,NULL,
                '2026-07-28T00:00:02Z','batch-hash'
             );
             INSERT INTO selection_source_batch_attempts (
                source_batch_attempt_id,ingress_run_id,config_activation_run_id,config_hash,
                generation_market_date,registered_feed_identity,registered_feed_snapshot_hash,
                request_hash,request_evidence_json,request_evidence_hash,
                feed_attempt_content_hash,status_kind,record_count,provider,source,
                source_at,observed_at,batch_id,batch_content_hash,failed_stage,reason_code,
                retryable,available_evidence_json,available_evidence_hash,error_detail_json,
                error_detail_hash,error_fingerprint,attempted_at,content_hash
             ) VALUES
             ('batch-empty-2','ingress-sample','activation','config-hash','2026-07-28',
              'feed-2','feed-snapshot','request-2','{}','request-evidence-2',
              'feed-attempt-2','verified_empty',0,
              'cailianpress','global-news','2026-07-28T00:00:00Z',
              '2026-07-28T00:00:01Z','batch-2','batch-content-2',NULL,NULL,NULL,
              '{}','available-hash-2',NULL,NULL,NULL,'2026-07-28T00:00:02Z','batch-hash-2'),
             ('batch-empty-3','ingress-sample','activation','config-hash','2026-07-28',
              'feed-3','feed-snapshot','request-3','{}','request-evidence-3',
              'feed-attempt-3','verified_empty',0,
              'jin10','global-news','2026-07-28T00:00:00Z',
              '2026-07-28T00:00:01Z','batch-3','batch-content-3',NULL,NULL,NULL,
              '{}','available-hash-3',NULL,NULL,NULL,'2026-07-28T00:00:02Z','batch-hash-3'),
             ('batch-empty-4','ingress-sample','activation','config-hash','2026-07-28',
              'feed-4','feed-snapshot','request-4','{}','request-evidence-4',
              'feed-attempt-4','verified_empty',0,
              'thepaper','global-news','2026-07-28T00:00:00Z',
              '2026-07-28T00:00:01Z','batch-4','batch-content-4',NULL,NULL,NULL,
              '{}','available-hash-4',NULL,NULL,NULL,'2026-07-28T00:00:02Z','batch-hash-4');",
        )
        .unwrap();
        insert_fact(conn, "fact-sample", "ingress-sample", "admitted").unwrap();
        conn.batch_execute(
            "INSERT INTO selection_source_fact_attempts (
                source_fact_attempt_id,ingress_run_id,source_batch_attempt_id,
                provider_ordinal,source_fact_key,acquired_record_json,acquired_record_hash,
                batch_evidence_json,batch_evidence_hash,event_projection_id,attempt_result,
                conflict_hash,attempted_at,content_hash
             ) VALUES (
                'fact-attempt-sample','ingress-sample','batch-sample',0,
                'fact-sample','{}','record-hash','{}','available-hash',
                'event','inserted',NULL,'2026-07-28T00:00:02Z','fact-attempt-hash'
             );",
        )
        .unwrap();
        insert_manifest(conn, "ingress-sample", "ingress_run", "completed", None, 7).unwrap();
        insert_receipt(conn, "ingress-sample", "ingress_run").unwrap();
        insert_envelope(conn, "generation-sample", "generation_run").unwrap();
        insert_resolved_direct_relation(
            conn,
            "relation-sample",
            "relation-key",
            "generation-sample",
            "fact-sample",
            "TEST_CODE_600000",
        )
        .unwrap();
        conn.batch_execute(
            "INSERT INTO selection_evaluation_attempts (
                evaluation_attempt_id,sample_key,generation_run_id,source_fact_key,event_id,
                chain_id,canonical_stock_code,canonical_stock_name,canonical_market,
                relation_evidence_set_hash,market_request_hash,
                request_evidence_json,request_evidence_hash,result_code,failed_stage,retryable,
                provider,source,source_at,observed_at,batch_id,batch_content_hash,
                available_evidence_json,available_evidence_hash,terminal_decision_hash,
                error_detail_json,error_detail_hash,error_fingerprint,attempted_at,content_hash
             ) VALUES (
                'evaluation-sample','TEST_CODE_SAMPLE','generation-sample','fact-sample','event',
                'chain','TEST_CODE_600000','Test Security','SZ','relation-set','market-request',
                '{}','request-evidence',
                'completed',NULL,NULL,'magic-tdx','t0-bundle',NULL,
                '2026-07-28T00:00:03Z','market-batch','market-batch-hash','{}',
                'available-hash','sample-hash',NULL,NULL,NULL,
                '2026-07-28T00:00:03Z','evaluation-hash'
             );",
        )
        .unwrap();
        diesel::sql_query(
            "INSERT INTO selection_samples (
                sample_key,generation_run_id,source_fact_key,source_fact_content_hash,
                source_fact_attempt_id,source_batch_attempt_id,event_id,chain_id,
                config_activation_run_id,config_hash,matched_keyword,canonical_stock_code,
                canonical_stock_name,canonical_market,relation_schema_version,
                relation_evidence_json,relation_evidence_set_hash,feature_version,
                t0_feature_json,t0_feature_hash,market_provider,market_source,market_source_at,
                market_observed_at,market_batch_id,market_batch_content_hash,admission_version,
                decision_kind,rejection_count,rejection_row_hashes_in_ordinal_order,
                evaluation_market_date,t0_due_date,d1_due_date,d2_due_date,d3_due_date,
                d4_due_date,d5_due_date,calendar_version,calendar_hash,
                trading_date_vector_json,trading_date_vector_hash,staged_at,content_hash
             ) VALUES (
                'TEST_CODE_SAMPLE','generation-sample','fact-sample','fact-hash',
                'fact-attempt-sample','batch-sample','event','chain','activation','config-hash',
                'keyword','TEST_CODE_600000','Test Security','SZ','event-relation-v2','{}',
                'relation-set','t0-feature-v1','{}','feature-hash','magic-tdx','t0-bundle',NULL,
                '2026-07-28T00:00:03Z','market-batch','market-batch-hash','admission-v1',
                ?1,?2,?3,'2026-07-28','2026-07-28','2026-07-29',
                '2026-07-30','2026-07-31','2026-08-03','2026-08-04',
                'calendar-v1','calendar-hash',
                '{\"domain\":\"stock_analysis.br178.outcome_trading_dates.v1\",\"t0\":\"2026-07-28\",\"d1\":\"2026-07-29\",\"d2\":\"2026-07-30\",\"d3\":\"2026-07-31\",\"d4\":\"2026-08-03\",\"d5\":\"2026-08-04\"}',
                'eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee',
                '2026-07-28T00:00:03Z','sample-hash'
             )",
        )
        .bind::<Text, _>(decision)
        .bind::<Integer, _>(rejection_count)
        .bind::<Text, _>(rejection_hashes_json)
        .execute(conn)
        .unwrap();
    }

    fn insert_admitted_sample(conn: &mut SqliteConnection) {
        stage_sample(conn, "admitted", 0, "[]");
        insert_manifest(
            conn,
            "generation-sample",
            "generation_run",
            "completed",
            Some("fact-sample"),
            4,
        )
        .unwrap();
        insert_receipt(conn, "generation-sample", "generation_run").unwrap();
    }

    fn insert_outcome_manifest(
        conn: &mut SqliteConnection,
        run_id: &str,
        status: &str,
        expected_count: i32,
    ) -> QueryResult<usize> {
        diesel::sql_query(
            "INSERT INTO selection_v2_run_stages (
                subject_id,subject_kind,logical_subject_key,in_memory_payload_hash,
                prepared_record_hash,expected_staged_row_count,staged_db_content_hash,
                recovery_envelope_content_hash,run_status,source_fact_key,
                config_activation_run_id,config_hash,config_snapshot_json_hash,
                config_activation_content_hash,config_activation_file_content_hash,
                config_effective_from,artifact_valid_from,artifact_expires_at,
                executable_revision,legacy_cutover_snapshot_hash,generation_market_date,
                aggregator_observed_at,ingress_source_batch_content_hash,outcome_phase,
                stored_due_date,staged_at,manifest_content_hash
             ) VALUES (
                ?1,'outcome_run',?1,'memory-hash','prepared-hash',?3,'db-hash',
                'envelope-hash',?2,NULL,'activation','config-hash',NULL,NULL,NULL,
                NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,'t0_close','2026-07-28',
                '2026-07-28T00:00:03Z','manifest-hash'
             )",
        )
        .bind::<diesel::sql_types::Text, _>(run_id)
        .bind::<diesel::sql_types::Text, _>(status)
        .bind::<Integer, _>(expected_count)
        .execute(conn)
    }

    #[test]
    fn transitional_fixture_has_twelve_tables_but_runtime_verification_is_disabled() {
        let mut conn = connection(SelectionV2StoreMode::Test);
        assert!(matches!(
            verify_selection_v2_connection(&mut conn),
            Err(SelectionV2SchemaError::IncompleteTarget { .. })
        ));

        let row = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM sqlite_master
             WHERE type='table' AND name LIKE 'selection_%'
               AND name IN (
                 'selection_source_batch_attempts','selection_source_facts_v2',
                 'selection_source_fact_attempts','selection_relation_attempts',
                 'selection_evaluation_attempts','selection_samples',
                 'selection_rejections','selection_sample_outcomes',
                 'selection_outcome_attempts','selection_v2_recovery_envelopes',
                 'selection_v2_run_stages','selection_v2_commit_receipts'
               )",
        )
        .get_result::<CountRow>(&mut conn)
        .unwrap();
        assert_eq!(row.count, 12);
    }

    #[test]
    fn startup_rejects_complete_transitional_schema_as_incomplete_target() {
        let mut conn = connection(SelectionV2StoreMode::Test);
        assert!(matches!(
            initialize_selection_v2_schema(&mut conn, SelectionV2StoreMode::Test),
            Err(SelectionV2SchemaError::IncompleteTarget { .. })
        ));
    }

    #[test]
    fn startup_does_not_create_transitional_schema_for_absent_database() {
        let mut conn = SqliteConnection::establish(":memory:").expect("isolated sqlite");
        let error = initialize_selection_v2_schema_with_audit_state(
            &mut conn,
            SelectionV2StoreMode::Test,
            false,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            SelectionV2SchemaError::IncompleteTarget { .. }
        ));
        let present = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM sqlite_master
             WHERE type='table' AND name LIKE 'selection_%'",
        )
        .get_result::<CountRow>(&mut conn)
        .unwrap();
        assert_eq!(
            present.count, 0,
            "startup must not install incomplete target"
        );
    }

    #[test]
    fn production_startup_requires_validated_audit_state() {
        let mut conn = SqliteConnection::establish(":memory:").expect("isolated sqlite");
        assert!(matches!(
            initialize_selection_v2_schema(&mut conn, SelectionV2StoreMode::Production),
            Err(SelectionV2SchemaError::SchemaMismatch {
                table: "selection_v2_table_set",
                column: "audit_phase",
                expected: "validated selection audit state",
                actual,
            }) if actual == "unknown"
        ));
    }

    #[test]
    fn production_startup_rejects_raw_boolean_audit_state_bypass() {
        let mut conn = SqliteConnection::establish(":memory:").expect("isolated sqlite");
        assert!(matches!(
            initialize_selection_v2_schema_with_audit_state(
                &mut conn,
                SelectionV2StoreMode::Production,
                false,
            ),
            Err(SelectionV2SchemaError::SchemaMismatch {
                table: "selection_v2_table_set",
                column: "audit_phase",
                expected: "validated locked selection audit session",
                actual,
            }) if actual == "raw boolean state"
        ));
        let present = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM sqlite_master
             WHERE type='table' AND name LIKE 'selection_%'",
        )
        .get_result::<CountRow>(&mut conn)
        .unwrap();
        assert_eq!(
            present.count, 0,
            "raw boolean bypass must not mutate schema"
        );
    }

    #[test]
    fn startup_rejects_unexpected_transitional_column() {
        let mut conn = connection(SelectionV2StoreMode::Test);
        conn.batch_execute(
            "ALTER TABLE selection_outcome_attempts
             ADD COLUMN unexpected_semantic_field TEXT;",
        )
        .unwrap();

        assert!(matches!(
            initialize_selection_v2_schema(&mut conn, SelectionV2StoreMode::Test),
            Err(SelectionV2SchemaError::SchemaMismatch {
                table: "selection_v2_table_set",
                column: "revision",
                actual,
                ..
            }) if actual.starts_with("classified drift:")
        ));
    }

    #[test]
    fn transitional_startup_rejects_and_does_not_repair_missing_safety_trigger() {
        let mut conn = connection(SelectionV2StoreMode::Test);
        conn.batch_execute("DROP TRIGGER selection_v2_batch_lineage;")
            .unwrap();

        assert!(matches!(
            initialize_selection_v2_schema(&mut conn, SelectionV2StoreMode::Test),
            Err(SelectionV2SchemaError::SchemaMismatch {
                table: "selection_v2_table_set",
                column: "revision",
                actual,
                ..
            }) if actual.starts_with("classified drift:")
        ));
        let trigger = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM sqlite_master
             WHERE type='trigger' AND name='selection_v2_batch_lineage'",
        )
        .get_result::<CountRow>(&mut conn)
        .unwrap();
        assert_eq!(trigger.count, 0, "startup must not silently repair trigger");
    }

    #[test]
    fn transitional_startup_rejects_and_does_not_repair_missing_unique_index() {
        let mut conn = connection(SelectionV2StoreMode::Test);
        conn.batch_execute("DROP INDEX selection_v2_one_activation_per_config;")
            .unwrap();

        assert!(matches!(
            initialize_selection_v2_schema(&mut conn, SelectionV2StoreMode::Test),
            Err(SelectionV2SchemaError::SchemaMismatch {
                table: "selection_v2_table_set",
                column: "revision",
                actual,
                ..
            }) if actual.starts_with("classified drift:")
        ));
        let index = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM sqlite_master
             WHERE type='index' AND name='selection_v2_one_activation_per_config'",
        )
        .get_result::<CountRow>(&mut conn)
        .unwrap();
        assert_eq!(index.count, 0, "startup must not silently repair index");
    }

    #[test]
    fn transitional_startup_rejects_unexpected_affected_object() {
        let mut conn = connection(SelectionV2StoreMode::Test);
        conn.batch_execute(
            "CREATE VIEW selection_v2_unexpected_view AS
             SELECT sample_key FROM selection_samples;",
        )
        .unwrap();

        assert!(matches!(
            initialize_selection_v2_schema(&mut conn, SelectionV2StoreMode::Test),
            Err(SelectionV2SchemaError::SchemaMismatch {
                table: "selection_v2_table_set",
                column: "revision",
                actual,
                ..
            }) if actual.starts_with("classified drift:")
        ));
    }

    #[test]
    fn connection_verification_rejects_downgraded_synchronous_pragma() {
        let mut conn = connection(SelectionV2StoreMode::Test);
        conn.batch_execute("PRAGMA synchronous = NORMAL;").unwrap();

        assert!(matches!(
            verify_selection_v2_connection(&mut conn),
            Err(SelectionV2SchemaError::UnsafePragma {
                name: "synchronous",
                expected: 2,
                actual: 1,
            })
        ));
    }

    #[test]
    fn connection_verification_rejects_foreign_key_corruption() {
        let mut conn = connection(SelectionV2StoreMode::Test);
        conn.batch_execute(
            "PRAGMA foreign_keys=OFF;
             CREATE TABLE TEST_CODE_fk_parent(id INTEGER PRIMARY KEY);
             CREATE TABLE TEST_CODE_fk_child(
                parent_id INTEGER NOT NULL REFERENCES TEST_CODE_fk_parent(id)
             );
             INSERT INTO TEST_CODE_fk_child(parent_id) VALUES (7);
             PRAGMA foreign_keys=ON;",
        )
        .unwrap();

        assert!(matches!(
            verify_selection_v2_connection(&mut conn),
            Err(SelectionV2SchemaError::ForeignKeyViolation { violations: 1 })
        ));
    }

    #[test]
    fn connection_verification_rejects_same_name_noop_trigger() {
        let mut conn = connection(SelectionV2StoreMode::Test);
        conn.batch_execute(
            "DROP TRIGGER selection_samples_deny_update;
             CREATE TRIGGER selection_samples_deny_update
             BEFORE UPDATE ON selection_samples BEGIN SELECT 1; END;",
        )
        .unwrap();

        assert!(matches!(
            verify_selection_v2_connection(&mut conn),
            Err(SelectionV2SchemaError::SchemaMismatch {
                table: "selection_v2_table_set",
                column: "revision",
                actual,
                ..
            }) if actual.starts_with("classified drift:")
        ));
    }

    #[test]
    fn one_database_cannot_be_reopened_in_the_opposite_store_mode() {
        let mut conn = connection(SelectionV2StoreMode::Test);

        assert!(matches!(
            initialize_selection_v2_schema_with_validated_audit_state(
                &mut conn,
                SelectionV2StoreMode::Production,
                false,
            ),
            Err(SelectionV2SchemaError::StoreModeConflict)
        ));
    }

    #[test]
    fn config_activation_envelope_rejects_wrong_schema_and_self_reference() {
        let mut conn = connection(SelectionV2StoreMode::Test);

        let result = conn.batch_execute(
            "INSERT INTO selection_v2_recovery_envelopes (
                stage_run_id,subject_kind,logical_subject_key,payload_schema,payload_json,
                payload_json_hash,in_memory_payload_hash,config_activation_run_id,config_hash,
                enveloped_at,content_hash
             ) VALUES (
                'config-invalid','config_activation','config-hash','wrong-schema','{}',
                'payload-hash','memory-hash','different-run','config-hash',
                '2026-07-28T00:00:00Z','envelope-hash'
             );",
        );

        assert!(result.is_err());
    }

    #[test]
    fn config_activation_manifest_rejects_unregistered_status() {
        let mut conn = connection(SelectionV2StoreMode::Test);
        insert_envelope(&mut conn, "config-invalid-status", "config_activation").unwrap();

        let result = conn.batch_execute(
            "INSERT INTO selection_v2_run_stages (
                subject_id,subject_kind,logical_subject_key,run_status,
                config_activation_run_id,config_hash,source_fact_key,sample_key,outcome_phase,
                stored_due_date,expected_staged_row_count,in_memory_payload_hash,
                staged_db_content_hash,recovery_envelope_content_hash,manifest_content_hash
             ) VALUES (
                'config-invalid-status','config_activation','config-invalid-status','garbage',
                'config-invalid-status','config-hash',NULL,NULL,NULL,NULL,1,'memory-hash',
                'db-hash','envelope-hash','manifest-hash'
             );",
        );

        assert!(result.is_err());
    }

    #[test]
    fn every_v2_table_denies_update_and_delete() {
        let mut conn = connection(SelectionV2StoreMode::Test);
        let triggers = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM sqlite_master
             WHERE type='trigger'
               AND (name LIKE '%_deny_update' OR name LIKE '%_deny_delete')",
        )
        .get_result::<CountRow>(&mut conn)
        .unwrap();
        assert_eq!(triggers.count, 24);

        insert_envelope(&mut conn, "ingress-1", "ingress_run").unwrap();

        assert!(conn
            .batch_execute(
                "UPDATE selection_v2_recovery_envelopes
                 SET logical_subject_key='changed' WHERE stage_run_id='ingress-1';"
            )
            .is_err());
        assert!(conn
            .batch_execute(
                "DELETE FROM selection_v2_recovery_envelopes WHERE stage_run_id='ingress-1';"
            )
            .is_err());
    }

    #[test]
    fn generation_domain_rejects_ingress_rejected_fact() {
        let mut conn = connection(SelectionV2StoreMode::Test);
        insert_envelope(&mut conn, "ingress-1", "ingress_run").unwrap();
        insert_fact(&mut conn, "fact-rejected", "ingress-1", "rejected").unwrap();
        insert_envelope(&mut conn, "generation-1", "generation_run").unwrap();

        let result = insert_resolved_direct_relation(
            &mut conn,
            "relation-1",
            "relation-key",
            "generation-1",
            "fact-rejected",
            "TEST_CODE_600000",
        )
        .unwrap_err();
        assert!(result
            .to_string()
            .contains("matching ingress-admitted source fact"));
    }

    #[test]
    fn resolved_direct_relation_requires_no_provider_evidence() {
        let mut conn = connection(SelectionV2StoreMode::Test);
        insert_envelope(&mut conn, "ingress-direct", "ingress_run").unwrap();
        insert_fact(&mut conn, "fact-direct", "ingress-direct", "admitted").unwrap();
        insert_envelope(&mut conn, "generation-direct", "generation_run").unwrap();

        assert!(insert_resolved_direct_relation(
            &mut conn,
            "relation-direct",
            "relation-key-direct",
            "generation-direct",
            "fact-direct",
            "TEST_CODE_600000",
        )
        .is_ok());
        assert!(insert_resolved_direct_relation_with_evidence(
            &mut conn,
            "relation-direct-with-provider",
            "relation-key-direct-with-provider",
            "generation-direct",
            "fact-direct",
            "TEST_CODE_600001",
            Some("{}"),
            Some("provider-evidence-hash"),
        )
        .is_err());
    }

    #[test]
    fn outcome_manifest_is_immediate_and_must_be_last() {
        let mut conn = connection(SelectionV2StoreMode::Test);
        insert_admitted_sample(&mut conn);
        insert_envelope(&mut conn, "outcome-empty", "outcome_run").unwrap();

        assert!(insert_outcome_manifest(&mut conn, "outcome-empty", "expected_wait", 1).is_err());

        insert_outcome_attempt(&mut conn, "outcome-empty", "expected_wait").unwrap();
        assert!(insert_outcome_manifest(&mut conn, "outcome-empty", "expected_wait", 2).is_ok());
    }

    #[test]
    fn expected_wait_attempt_requires_the_registered_reason() {
        let mut conn = connection(SelectionV2StoreMode::Test);
        insert_admitted_sample(&mut conn);
        insert_envelope(&mut conn, "outcome-invalid-wait", "outcome_run").unwrap();

        assert!(conn
            .batch_execute(
                "INSERT INTO selection_outcome_attempts (
                    outcome_attempt_id,sample_key,phase,stored_due_date,outcome_run_id,
                    result_code,reason_code,retryable,provider,available_evidence_json,
                    available_evidence_hash,error_detail_json,error_detail_hash,error_fingerprint,
                    settled_outcome_content_hash,content_hash
                 ) VALUES (
                    'attempt-invalid-wait','TEST_CODE_SAMPLE','t0_close','2026-07-28',
                    'outcome-invalid-wait','expected_wait','settled_bar_missing',
                    NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,'attempt-hash'
                 );"
            )
            .is_err());
    }

    #[test]
    fn settled_manifest_requires_exactly_one_matching_outcome() {
        let mut conn = connection(SelectionV2StoreMode::Test);
        insert_admitted_sample(&mut conn);
        insert_envelope(&mut conn, "outcome-settled", "outcome_run").unwrap();
        insert_settled_attempt(&mut conn, "outcome-settled").unwrap();

        assert!(insert_outcome_manifest(&mut conn, "outcome-settled", "settled", 2).is_err());
        conn.batch_execute(
            "INSERT INTO selection_sample_outcomes (
                sample_key,phase,outcome_run_id,due_trading_date,open,high,low,close,
                volume,amount,return_from_t0_close,cumulative_mfe,cumulative_mae,
                volume_ratio,provider,source,source_at,observed_at,batch_id,
                batch_content_hash,created_at,content_hash
             ) VALUES (
                'TEST_CODE_SAMPLE','t0_close','outcome-settled','2026-07-28',
                '10','11','9','10','1000','10000','0','0','0','1',
                'magic-tdx','daily-bars','2026-07-28T07:00:00Z',
                '2026-07-28T07:00:01Z','outcome-batch','outcome-batch-hash',
                '2026-07-28T07:00:02Z','outcome-hash'
             );",
        )
        .unwrap();
        assert!(insert_outcome_manifest(&mut conn, "outcome-settled", "settled", 3).is_ok());
    }

    #[test]
    fn outcome_manifest_prevents_post_manifest_domain_insertion() {
        let mut conn = connection(SelectionV2StoreMode::Test);
        insert_admitted_sample(&mut conn);
        insert_envelope(&mut conn, "outcome-wait", "outcome_run").unwrap();
        insert_outcome_attempt(&mut conn, "outcome-wait", "expected_wait").unwrap();
        insert_outcome_manifest(&mut conn, "outcome-wait", "expected_wait", 2).unwrap();

        // A direct writer tries to mutate the staged set after the manifest.
        let late_insert = conn.batch_execute(
            "INSERT INTO selection_sample_outcomes (
                sample_key,phase,outcome_run_id,due_trading_date,open,high,low,close,
                volume,amount,return_from_t0_close,cumulative_mfe,cumulative_mae,
                volume_ratio,provider,source,source_at,observed_at,batch_id,
                batch_content_hash,created_at,content_hash
             ) VALUES (
                'TEST_CODE_SAMPLE','t0_close','outcome-wait','2026-07-28',
                '10','11','9','10','1000','10000','0','0','0','1',
                'magic-tdx','daily-bars','2026-07-28T07:00:00Z',
                '2026-07-28T07:00:01Z','outcome-batch','outcome-batch-hash',
                '2026-07-28T07:00:02Z','outcome-hash'
             );",
        );
        assert!(late_insert.is_err());
        let receipt = conn.batch_execute(
            "INSERT INTO selection_v2_commit_receipts (
                subject_kind,subject_id,logical_subject_key,in_memory_payload_hash,
                recovery_envelope_content_hash,prepared_audit_hash,run_manifest_content_hash,
                staged_db_content_hash,committed_audit_hash,committed_at,content_hash
             ) VALUES (
                'outcome_run','outcome-wait','outcome-wait','memory-hash',
                'envelope-hash','prepared-hash','manifest-hash','db-hash','audit-hash',
                '2026-07-28T00:01:00Z','receipt-hash'
             );",
        );
        assert!(receipt.is_ok());
    }

    #[test]
    fn production_and_test_symbol_namespaces_are_physically_isolated() {
        let mut production = connection(SelectionV2StoreMode::Production);
        insert_envelope(&mut production, "ingress-prod", "ingress_run").unwrap();
        insert_fact(&mut production, "fact-prod", "ingress-prod", "admitted").unwrap();
        insert_envelope(&mut production, "generation-prod", "generation_run").unwrap();
        let production_error = insert_resolved_direct_relation(
            &mut production,
            "relation-prod",
            "key",
            "generation-prod",
            "fact-prod",
            "TEST_CODE_600000",
        )
        .unwrap_err();
        assert!(production_error.to_string().contains("symbol isolation"));

        let mut test = connection(SelectionV2StoreMode::Test);
        insert_envelope(&mut test, "ingress-test", "ingress_run").unwrap();
        insert_fact(&mut test, "fact-test", "ingress-test", "admitted").unwrap();
        insert_envelope(&mut test, "generation-test", "generation_run").unwrap();
        let test_error = insert_resolved_direct_relation(
            &mut test,
            "relation-test",
            "key",
            "generation-test",
            "fact-test",
            "600000",
        )
        .unwrap_err();
        assert!(test_error.to_string().contains("symbol isolation"));
    }

    #[test]
    fn source_batch_unavailable_requires_null_record_count() {
        let mut conn = connection(SelectionV2StoreMode::Test);
        insert_envelope(&mut conn, "ingress-count-contract", "ingress_run").unwrap();

        assert!(insert_source_batch_attempt(
            &mut conn,
            "attempt-unavailable-null",
            "feed-unavailable-null",
            "unavailable",
            None,
        )
        .is_ok());
        assert!(insert_source_batch_attempt(
            &mut conn,
            "attempt-unavailable-zero",
            "feed-unavailable-zero",
            "unavailable",
            Some(0),
        )
        .is_err());
    }

    #[test]
    fn source_batch_available_requires_positive_record_count() {
        let mut conn = connection(SelectionV2StoreMode::Test);
        insert_envelope(&mut conn, "ingress-count-contract", "ingress_run").unwrap();

        assert!(insert_source_batch_attempt(
            &mut conn,
            "attempt-available-positive",
            "feed-available-positive",
            "available",
            Some(1),
        )
        .is_ok());
        assert!(insert_source_batch_attempt(
            &mut conn,
            "attempt-available-zero",
            "feed-available-zero",
            "available",
            Some(0),
        )
        .is_err());
        assert!(insert_source_batch_attempt(
            &mut conn,
            "attempt-available-null",
            "feed-available-null",
            "available",
            None,
        )
        .is_err());
    }

    #[test]
    fn source_batch_verified_empty_requires_zero_record_count() {
        let mut conn = connection(SelectionV2StoreMode::Test);
        insert_envelope(&mut conn, "ingress-count-contract", "ingress_run").unwrap();

        assert!(insert_source_batch_attempt(
            &mut conn,
            "attempt-empty-zero",
            "feed-empty-zero",
            "verified_empty",
            Some(0),
        )
        .is_ok());
        assert!(insert_source_batch_attempt(
            &mut conn,
            "attempt-empty-positive",
            "feed-empty-positive",
            "verified_empty",
            Some(1),
        )
        .is_err());
        assert!(insert_source_batch_attempt(
            &mut conn,
            "attempt-empty-null",
            "feed-empty-null",
            "verified_empty",
            None,
        )
        .is_err());
    }

    #[test]
    fn source_batch_record_count_column_is_nullable_after_startup() {
        let mut conn = connection(SelectionV2StoreMode::Test);
        let nullability = diesel::sql_query(
            "SELECT CAST([notnull] AS BIGINT) AS count
             FROM pragma_table_info('selection_source_batch_attempts')
             WHERE name='record_count'",
        )
        .get_result::<CountRow>(&mut conn)
        .unwrap();

        assert_eq!(nullability.count, 0);
    }

    #[test]
    fn record_count_verifier_rejects_not_null_contract() {
        let mut conn = SqliteConnection::establish(":memory:").expect("isolated sqlite");
        conn.batch_execute(
            "CREATE TABLE selection_source_batch_attempts (
                record_count INTEGER NOT NULL
             );",
        )
        .unwrap();

        let error = verify_source_batch_record_count_contract(&mut conn).unwrap_err();
        assert!(matches!(
            error,
            SelectionV2SchemaError::SchemaMismatch {
                table: "selection_source_batch_attempts",
                column: "record_count",
                expected: "nullable",
                ..
            }
        ));
    }

    #[test]
    fn transitional_schema_has_request_evidence_columns_and_payload_contracts() {
        let mut conn = connection(SelectionV2StoreMode::Test);
        for (table, request_column) in [
            ("selection_source_batch_attempts", "request_hash"),
            ("selection_relation_attempts", "request_hash"),
            ("selection_evaluation_attempts", "market_request_hash"),
            ("selection_outcome_attempts", "request_hash"),
        ] {
            let columns = diesel::sql_query(format!(
                "SELECT COUNT(*) AS count
                 FROM pragma_table_info('{table}')
                 WHERE name IN ('{request_column}','request_evidence_json',
                                'request_evidence_hash')"
            ))
            .get_result::<CountRow>(&mut conn)
            .unwrap();
            assert_eq!(columns.count, 3, "transitional request tuple for {table}");
        }
        let schedule_columns = diesel::sql_query(
            "SELECT COUNT(*) AS count
             FROM pragma_table_info('selection_samples')
             WHERE name IN (
                 'd2_due_date','d4_due_date',
                 'trading_date_vector_json','trading_date_vector_hash'
             )",
        )
        .get_result::<CountRow>(&mut conn)
        .unwrap();
        assert_eq!(
            schedule_columns.count, 4,
            "transitional sample schedule persists the full canonical date vector"
        );

        for (kind, schema, domain) in [
            (
                "ingress_run",
                "source-ingress-stage-v2",
                "stock_analysis.br174.source_ingress_stage.v2",
            ),
            (
                "generation_run",
                "generation-stage-v3",
                "stock_analysis.br174.generation_stage.v3",
            ),
            (
                "outcome_run",
                "outcome-stage-v2",
                "stock_analysis.br174.outcome_stage.v2",
            ),
        ] {
            let sql = format!(
                "INSERT INTO selection_v2_recovery_envelopes (
                    stage_run_id,subject_kind,logical_subject_key,payload_schema,payload_json,
                    payload_json_hash,in_memory_payload_hash,config_activation_run_id,config_hash,
                    enveloped_at,content_hash
                 ) VALUES (
                    'stage-{kind}','{kind}','logical','{schema}',
                    '{{\"domain\":\"{domain}\"}}','payload','memory','activation',
                    'config-hash','2026-07-28T00:00:00Z','content'
                 )"
            );
            assert!(
                conn.batch_execute(&sql).is_ok(),
                "{kind} transitional payload"
            );
        }

        assert!(conn
            .batch_execute(
                "INSERT INTO selection_v2_recovery_envelopes (
                    stage_run_id,subject_kind,logical_subject_key,payload_schema,payload_json,
                    payload_json_hash,in_memory_payload_hash,config_activation_run_id,config_hash,
                    enveloped_at,content_hash
                 ) VALUES (
                    'stage-mixed-domain','generation_run','logical',
                    'generation-stage-v3',
                    '{\"domain\":\"stock_analysis.br174.generation_stage.v2\"}',
                    'payload','memory','activation','config-hash',
                    '2026-07-28T00:00:00Z','content'
                 );"
            )
            .is_err());
        assert!(conn
            .batch_execute(
                "INSERT INTO selection_v2_recovery_envelopes (
                    stage_run_id,subject_kind,logical_subject_key,payload_schema,payload_json,
                    payload_json_hash,in_memory_payload_hash,config_activation_run_id,config_hash,
                    enveloped_at,content_hash
                 ) VALUES (
                    'stage-missing-domain','outcome_run','logical',
                    'outcome-stage-v2','{}','payload','memory','activation',
                    'config-hash','2026-07-28T00:00:00Z','content'
                 );"
            )
            .is_err());
    }

    #[test]
    fn outcome_request_tuple_is_null_only_for_expected_wait() {
        let mut conn = connection(SelectionV2StoreMode::Test);
        insert_admitted_sample(&mut conn);
        insert_envelope(&mut conn, "outcome-null-request", "outcome_run").unwrap();

        assert!(
            insert_outcome_attempt(&mut conn, "outcome-null-request", "expected_wait",).is_ok()
        );

        insert_envelope(&mut conn, "outcome-half-request", "outcome_run").unwrap();
        assert!(conn
            .batch_execute(
                "INSERT INTO selection_outcome_attempts (
                    outcome_attempt_id,sample_key,phase,stored_due_date,outcome_run_id,
                    request_hash,request_evidence_json,request_evidence_hash,
                    result_code,reason_code,retryable,provider,source,source_at,observed_at,
                    batch_id,batch_content_hash,available_evidence_json,available_evidence_hash,
                    error_detail_json,error_detail_hash,error_fingerprint,
                    settled_outcome_content_hash,attempted_at,content_hash
                 ) VALUES (
                    'attempt-half-request','TEST_CODE_SAMPLE','t0_close','2026-07-28',
                    'outcome-half-request','request-hash',NULL,NULL,
                    'expected_wait','market_session_unsettled',NULL,NULL,NULL,NULL,NULL,
                    NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,
                    '2026-07-28T00:00:02Z','attempt-hash'
                 );"
            )
            .is_err());
    }

    #[test]
    fn startup_rejects_partial_selection_v2_table_set() {
        let mut conn = SqliteConnection::establish(":memory:").expect("isolated sqlite");
        conn.batch_execute(
            "CREATE TABLE selection_v2_recovery_envelopes (
                stage_run_id TEXT PRIMARY KEY
             );",
        )
        .unwrap();

        let error =
            initialize_selection_v2_schema(&mut conn, SelectionV2StoreMode::Test).unwrap_err();
        assert!(matches!(
            error,
            SelectionV2SchemaError::SchemaMismatch {
                table: "selection_v2_table_set",
                column: "revision",
                expected: "all absent, exact pre-amendment, or transitional-current",
                ..
            }
        ));
        let present = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM sqlite_master
             WHERE type='table' AND name LIKE 'selection_%'",
        )
        .get_result::<CountRow>(&mut conn)
        .unwrap();
        assert_eq!(present.count, 1, "partial startup must not mutate schema");
    }

    #[test]
    fn startup_rejects_mixed_selection_v2_schema_revision() {
        let mut conn = SqliteConnection::establish(":memory:").expect("isolated sqlite");
        install_pre_amendment_table_set(&mut conn, SelectionV2StoreMode::Test);
        conn.batch_execute("DROP INDEX selection_v2_samples_generation;")
            .unwrap();

        let error =
            initialize_selection_v2_schema(&mut conn, SelectionV2StoreMode::Test).unwrap_err();
        assert!(matches!(
            error,
            SelectionV2SchemaError::SchemaMismatch {
                table: "selection_v2_table_set",
                column: "revision",
                actual,
                ..
            } if actual.starts_with("classified drift:")
        ));
    }

    #[test]
    fn startup_disables_complete_pre_amendment_schema() {
        let mut conn = SqliteConnection::establish(":memory:").expect("isolated sqlite");
        install_pre_amendment_table_set(&mut conn, SelectionV2StoreMode::Test);

        let error =
            initialize_selection_v2_schema(&mut conn, SelectionV2StoreMode::Test).unwrap_err();
        assert!(matches!(
            error,
            SelectionV2SchemaError::SchemaMismatch {
                table: "selection_v2_table_set",
                column: "revision",
                expected: "final five-payload target or explicit operator migration",
                actual,
            } if actual == "pre-amendment; nonempty=[]"
        ));
    }

    #[test]
    fn migration_preflight_recognizes_complete_old_golden_in_both_namespaces() {
        for mode in [SelectionV2StoreMode::Test, SelectionV2StoreMode::Production] {
            let mut conn = SqliteConnection::establish(":memory:").expect("isolated sqlite");
            install_pre_amendment_table_set(&mut conn, mode);

            let preflight = inspect_read_only(&mut conn).expect("exact old golden is recognized");
            assert_eq!(preflight.integrity_check, "ok");
            assert_eq!(preflight.foreign_key_violations, 0);
            assert!(!preflight.apply_supported);
            assert_eq!(preflight.apply_blocker, SELECTION_V2_APPLY_BLOCKER);
            assert_eq!(
                preflight.state,
                SelectionV2MigrationState::PreAmendment {
                    store_mode: mode,
                    nonempty_tables: Vec::new(),
                }
            );
        }
    }

    #[test]
    fn migration_preflight_requires_query_only_snapshot() {
        let mut conn = SqliteConnection::establish(":memory:").expect("isolated sqlite");
        install_pre_amendment_table_set(&mut conn, SelectionV2StoreMode::Test);

        assert!(matches!(
            inspect_selection_v2_migration(&mut conn),
            Err(SelectionV2SchemaError::UnsafePragma {
                name: "query_only",
                expected: 1,
                actual: 0,
            })
        ));
    }

    #[test]
    fn migration_preflight_reports_empty_database_half_without_requiring_managed_triggers() {
        let mut conn = SqliteConnection::establish(":memory:").expect("isolated sqlite");

        let preflight = inspect_read_only(&mut conn).expect("inspect empty database half");
        assert_eq!(
            preflight.state,
            SelectionV2MigrationState::DatabaseAbsentAuditUnverified
        );
        assert!(!preflight.apply_supported);
    }

    #[test]
    fn absent_database_half_rejects_known_selection_trigger_name_on_external_table() {
        let mut conn = SqliteConnection::establish(":memory:").expect("isolated sqlite");
        conn.batch_execute(
            "CREATE TABLE external_events(id INTEGER PRIMARY KEY);
             CREATE TRIGGER selection_v2_batch_lineage
             AFTER INSERT ON external_events BEGIN SELECT 1; END;",
        )
        .unwrap();

        assert!(matches!(
            inspect_read_only(&mut conn),
            Err(SelectionV2SchemaError::SchemaMismatch {
                table: "selection_v2_table_set",
                column: "object_registry",
                ..
            })
        ));
    }

    #[test]
    fn migration_preflight_names_four_payload_schema_transitional_current() {
        let mut conn = connection(SelectionV2StoreMode::Test);
        let preflight = inspect_read_only(&mut conn).expect("inspect transitional schema");
        assert_eq!(
            preflight.state,
            SelectionV2MigrationState::TransitionalCurrent {
                store_mode: SelectionV2StoreMode::Test,
                nonempty_legacy_v2_outcome_tables: Vec::new(),
            }
        );
    }

    #[test]
    fn migration_preflight_classifies_transitional_and_final_database_halves_separately() {
        let mut transitional = connection(SelectionV2StoreMode::Test);
        assert!(matches!(
            inspect_read_only(&mut transitional)
                .expect("inspect exact transitional database half")
                .state,
            SelectionV2MigrationState::TransitionalCurrent {
                store_mode: SelectionV2StoreMode::Test,
                ..
            }
        ));

        let mut final_half =
            SqliteConnection::establish(":memory:").expect("isolated final sqlite");
        install_final_database_half(&mut final_half, SelectionV2StoreMode::Test);
        assert_eq!(
            inspect_read_only(&mut final_half)
                .expect("inspect exact final database half")
                .state,
            SelectionV2MigrationState::FinalTargetDatabaseHalf {
                store_mode: SelectionV2StoreMode::Test,
            }
        );
    }

    #[test]
    fn final_database_half_rejects_every_illegal_global_identity_matrix() {
        for (application_id, user_version, future) in [
            (0, 1, false),
            (STOCK_ANALYSIS_SQLITE_APPLICATION_ID, 0, false),
            (1_234_567, STOCK_ANALYSIS_DB_SCHEMA_GENERATION, false),
            (
                STOCK_ANALYSIS_SQLITE_APPLICATION_ID,
                STOCK_ANALYSIS_DB_SCHEMA_GENERATION + 1,
                true,
            ),
        ] {
            let mut conn = SqliteConnection::establish(":memory:").expect("isolated final sqlite");
            install_final_database_half(&mut conn, SelectionV2StoreMode::Test);
            conn.batch_execute("PRAGMA query_only = OFF;")
                .expect("leave read-only test seam");
            conn.batch_execute(&format!(
                "PRAGMA application_id = {application_id};
                 PRAGMA user_version = {user_version};"
            ))
            .expect("mutate test-only identity");
            let error =
                inspect_read_only(&mut conn).expect_err("illegal identity must fail closed");
            if future {
                assert!(matches!(
                    error,
                    SelectionV2SchemaError::UnsupportedFutureGeneration {
                        actual,
                        supported: STOCK_ANALYSIS_DB_SCHEMA_GENERATION,
                    } if actual == STOCK_ANALYSIS_DB_SCHEMA_GENERATION + 1
                ));
            } else {
                assert!(matches!(
                    error,
                    SelectionV2SchemaError::SchemaMismatch {
                        table: "selection_v2_table_set",
                        column: "global_schema_identity",
                        ..
                    }
                ));
            }
        }
    }

    #[test]
    fn future_global_generation_is_never_reclassified_by_selection_table_presence() {
        let mut conn = SqliteConnection::establish(":memory:").expect("isolated empty sqlite");
        conn.batch_execute(
            "PRAGMA application_id = 1398035265;
             PRAGMA user_version = 2;",
        )
        .expect("emulate future global owner");

        assert!(matches!(
            inspect_read_only(&mut conn),
            Err(SelectionV2SchemaError::UnsupportedFutureGeneration {
                actual: 2,
                supported: STOCK_ANALYSIS_DB_SCHEMA_GENERATION,
            })
        ));
    }

    #[test]
    fn final_catalog_rejects_exact_table_index_and_trigger_mutations() {
        let final_schema = selection_v2_final_schema().expect("build final checked DDL");

        let mutated_table = replace_schema_exact_once(
            final_schema.clone(),
            "    transport_attempts_hash TEXT,\n",
            "    transport_attempts_hash TEXT COLLATE BINARY,\n",
            "test.final_table_mutation",
        )
        .expect("one transport field");
        let mut table_conn =
            SqliteConnection::establish(":memory:").expect("isolated table mutation sqlite");
        install_unverified_final_database_half(
            &mut table_conn,
            SelectionV2StoreMode::Test,
            &mutated_table,
        );
        assert!(matches!(
            inspect_read_only(&mut table_conn),
            Err(SelectionV2SchemaError::SchemaMismatch {
                column: "final_catalog_exact_sql",
                ..
            })
        ));

        let mut index_conn =
            SqliteConnection::establish(":memory:").expect("isolated index mutation sqlite");
        install_final_database_half(&mut index_conn, SelectionV2StoreMode::Test);
        index_conn
            .batch_execute(
                "DROP INDEX selection_v2_outcome_attempt_run;
                 CREATE INDEX selection_v2_outcome_attempt_run
                 ON selection_outcome_attempts(outcome_run_id DESC, sample_key, phase);",
            )
            .expect("mutate explicit index");
        assert!(matches!(
            inspect_read_only(&mut index_conn),
            Err(SelectionV2SchemaError::SchemaMismatch {
                column: "final_catalog_exact_sql" | "final_catalog_index_geometry",
                ..
            })
        ));

        let mut trigger_conn =
            SqliteConnection::establish(":memory:").expect("isolated trigger mutation sqlite");
        install_final_database_half(&mut trigger_conn, SelectionV2StoreMode::Test);
        trigger_conn
            .batch_execute(
                "DROP TRIGGER selection_outcome_attempts_deny_update;
                 CREATE TRIGGER selection_outcome_attempts_deny_update
                 BEFORE UPDATE ON selection_outcome_attempts BEGIN SELECT 1; END;",
            )
            .expect("mutate append-only trigger");
        assert!(matches!(
            inspect_read_only(&mut trigger_conn),
            Err(SelectionV2SchemaError::SchemaMismatch {
                column: "final_catalog_exact_sql",
                ..
            })
        ));
    }

    #[test]
    fn final_catalog_rejects_non_catalog_trigger_on_managed_table() {
        let mut conn = SqliteConnection::establish(":memory:").expect("isolated final sqlite");
        install_final_database_half(&mut conn, SelectionV2StoreMode::Test);
        conn.batch_execute(
            "CREATE TRIGGER external_named_selection_dependency
             BEFORE INSERT ON selection_samples
             BEGIN SELECT 1; END;",
        )
        .expect("install non-catalog trigger on a managed table");

        assert!(matches!(
            inspect_read_only(&mut conn),
            Err(SelectionV2SchemaError::SchemaMismatch {
                column: "object_registry",
                ..
            })
        ));
    }

    #[test]
    fn external_schema_scanner_is_quote_and_comment_fail_closed() {
        assert!(!schema_sql_references_affected_table(
            "CREATE VIEW external_v AS SELECT 1 -- selection_samples\n;"
        )
        .expect("line comments are removed"));
        assert!(schema_sql_references_affected_table(
            "CREATE VIEW external_v AS SELECT * FROM 'Selection_Samples';"
        )
        .expect("single-quoted SQLite identifiers are decoded"));
        assert!(matches!(
            schema_sql_references_affected_table(
                "CREATE VIEW external_v AS SELECT 1 /* outer /* nested */;"
            ),
            Err(SelectionV2SchemaError::SchemaMismatch {
                column: "external_sql_scanner",
                ..
            })
        ));
        assert!(matches!(
            schema_sql_references_affected_table(
                "CREATE VIEW external_v AS SELECT * FROM \"unterminated;"
            ),
            Err(SelectionV2SchemaError::SchemaMismatch {
                column: "external_sql_scanner",
                ..
            })
        ));
    }

    #[test]
    fn final_outcome_transport_pair_is_null_only_for_expected_wait() {
        let mut conn = SqliteConnection::establish(":memory:").expect("isolated final sqlite");
        install_final_database_half(&mut conn, SelectionV2StoreMode::Test);
        insert_admitted_sample(&mut conn);

        insert_final_outcome_envelope(&mut conn, "outcome-final-wait");
        assert!(insert_final_outcome_attempt(
            &mut conn,
            "outcome-final-wait",
            "expected_wait",
            None,
            None,
        )
        .is_ok());

        insert_final_outcome_envelope(&mut conn, "outcome-final-half-pair");
        assert!(insert_final_outcome_attempt(
            &mut conn,
            "outcome-final-half-pair",
            "expected_wait",
            Some("{}"),
            None,
        )
        .is_err());

        insert_final_outcome_envelope(&mut conn, "outcome-final-settled-null");
        assert!(insert_final_outcome_attempt(
            &mut conn,
            "outcome-final-settled-null",
            "settled",
            None,
            None,
        )
        .is_err());

        insert_final_outcome_envelope(&mut conn, "outcome-final-settled-pair");
        assert!(insert_final_outcome_attempt(
            &mut conn,
            "outcome-final-settled-pair",
            "settled",
            Some("{}"),
            Some("transport-attempts-hash"),
        )
        .is_ok());
    }

    #[test]
    fn nonempty_legacy_v2_outcome_rows_are_an_explicit_migration_blocker() {
        let mut conn = connection(SelectionV2StoreMode::Test);
        insert_admitted_sample(&mut conn);
        insert_envelope(&mut conn, "legacy-v2-outcome", "outcome_run").unwrap();
        insert_outcome_attempt(&mut conn, "legacy-v2-outcome", "expected_wait").unwrap();

        let preflight = inspect_read_only(&mut conn).expect("inspect transitional legacy rows");
        assert_eq!(
            preflight.state,
            SelectionV2MigrationState::TransitionalCurrent {
                store_mode: SelectionV2StoreMode::Test,
                nonempty_legacy_v2_outcome_tables: vec![
                    SelectionV2AffectedTableCount {
                        table: "selection_outcome_attempts",
                        rows: 1,
                    },
                    SelectionV2AffectedTableCount {
                        table: "selection_v2_recovery_envelopes",
                        rows: 1,
                    },
                ],
            }
        );
        assert!(!preflight.apply_supported);
        assert_eq!(preflight.apply_blocker, SELECTION_V2_APPLY_BLOCKER);
    }

    #[test]
    fn old_schema_with_same_name_noop_trigger_is_classified_drift() {
        let mut conn = SqliteConnection::establish(":memory:").expect("isolated sqlite");
        install_pre_amendment_table_set(&mut conn, SelectionV2StoreMode::Test);
        conn.batch_execute(
            "DROP TRIGGER selection_v2_batch_lineage;
             CREATE TRIGGER selection_v2_batch_lineage
             BEFORE INSERT ON selection_source_batch_attempts
             BEGIN SELECT 1; END;",
        )
        .unwrap();

        let error = inspect_read_only(&mut conn).unwrap_err();
        assert!(matches!(
            error,
            SelectionV2SchemaError::SchemaMismatch {
                table: "selection_v2_table_set",
                column: "revision",
                actual,
                ..
            } if actual.starts_with("classified drift:")
        ));
    }

    #[test]
    fn old_schema_with_weakened_check_is_classified_drift() {
        let mut conn = SqliteConnection::establish(":memory:").expect("isolated sqlite");
        conn.batch_execute("PRAGMA foreign_keys = ON; PRAGMA synchronous = FULL;")
            .unwrap();
        let weakened = replace_schema_exact_once(
            PRE_AMENDMENT_SCHEMA.to_owned(),
            "    record_count INTEGER CHECK (record_count >= 0),\n",
            "    record_count INTEGER,\n",
            "test.selection_source_batch_attempts.record_count",
        )
        .expect("weaken one real old CHECK");
        conn.batch_execute(&weakened).unwrap();
        install_stage_membership_triggers(&mut conn).unwrap();
        install_append_only_triggers(&mut conn).unwrap();
        install_symbol_triggers(&mut conn, SelectionV2StoreMode::Test).unwrap();

        let error = inspect_read_only(&mut conn).unwrap_err();
        assert!(matches!(
            error,
            SelectionV2SchemaError::SchemaMismatch {
                table: "selection_v2_table_set",
                column: "revision",
                actual,
                ..
            } if actual.starts_with("classified drift:")
        ));
    }

    #[test]
    fn pre_amendment_startup_lists_every_nonempty_table_and_exact_count() {
        let mut conn = SqliteConnection::establish(":memory:").expect("isolated sqlite");
        install_pre_amendment_table_set(&mut conn, SelectionV2StoreMode::Test);
        insert_envelope(&mut conn, "ingress-count-contract", "ingress_run")
            .expect("insert real old ingress envelope");
        insert_source_batch_attempt(&mut conn, "old-batch", "old-feed", "unavailable", None)
            .expect("insert real old batch attempt");

        let error =
            initialize_selection_v2_schema(&mut conn, SelectionV2StoreMode::Test).unwrap_err();
        assert!(matches!(
            error,
            SelectionV2SchemaError::SchemaMismatch {
                table: "selection_v2_table_set",
                column: "revision",
                actual,
                ..
            } if actual
                == "pre-amendment; nonempty=[selection_source_batch_attempts=1,selection_v2_recovery_envelopes=1]"
        ));
    }

    #[test]
    fn old_schema_with_affected_view_is_classified_drift() {
        let mut conn = SqliteConnection::establish(":memory:").expect("isolated sqlite");
        install_pre_amendment_table_set(&mut conn, SelectionV2StoreMode::Test);
        conn.batch_execute(
            "CREATE VIEW selection_v2_legacy_view
             AS SELECT sample_key FROM selection_samples;",
        )
        .unwrap();

        let error = inspect_read_only(&mut conn).unwrap_err();
        assert!(matches!(
            error,
            SelectionV2SchemaError::SchemaMismatch {
                table: "selection_v2_table_set",
                column: "revision",
                actual,
                ..
            } if actual.starts_with("classified drift:")
        ));
    }

    #[test]
    fn old_schema_with_external_trigger_dependency_is_classified_drift() {
        let mut conn = SqliteConnection::establish(":memory:").expect("isolated sqlite");
        install_pre_amendment_table_set(&mut conn, SelectionV2StoreMode::Test);
        conn.batch_execute(
            "CREATE TABLE external_events(id INTEGER PRIMARY KEY);
             CREATE TRIGGER external_events_selection_dependency
             AFTER INSERT ON external_events
             BEGIN
               SELECT sample_key FROM selection_samples LIMIT 1;
             END;",
        )
        .unwrap();

        let error = inspect_read_only(&mut conn).unwrap_err();
        assert!(matches!(
            error,
            SelectionV2SchemaError::SchemaMismatch {
                table: "selection_v2_table_set",
                column: "revision",
                actual,
                ..
            } if actual.starts_with("classified drift:")
        ));
    }

    #[test]
    fn old_schema_rejects_extra_selection_named_index_on_external_table() {
        let mut conn = SqliteConnection::establish(":memory:").expect("isolated sqlite");
        install_pre_amendment_table_set(&mut conn, SelectionV2StoreMode::Test);
        conn.batch_execute(
            "CREATE TABLE external_events(id INTEGER PRIMARY KEY);
             CREATE INDEX selection_unregistered_external_index ON external_events(id);",
        )
        .unwrap();

        let error = inspect_read_only(&mut conn).unwrap_err();
        assert!(matches!(
            error,
            SelectionV2SchemaError::SchemaMismatch {
                table: "selection_v2_table_set",
                column: "revision",
                actual,
                ..
            } if actual.starts_with("classified drift:")
        ));
    }

    #[test]
    fn case_variant_affected_table_cannot_be_reported_absent() {
        let mut conn = SqliteConnection::establish(":memory:").expect("isolated sqlite");
        conn.batch_execute("CREATE TABLE \"Selection_Samples\"(sample_key TEXT PRIMARY KEY);")
            .unwrap();

        let error = inspect_read_only(&mut conn).unwrap_err();
        assert!(matches!(
            error,
            SelectionV2SchemaError::SchemaMismatch {
                table: "selection_v2_table_set",
                column: "revision",
                actual,
                ..
            } if actual.starts_with("classified drift:")
        ));
    }

    #[test]
    fn single_quoted_case_variant_external_dependency_is_classified_drift() {
        let mut conn = SqliteConnection::establish(":memory:").expect("isolated sqlite");
        install_pre_amendment_table_set(&mut conn, SelectionV2StoreMode::Test);
        conn.batch_execute(
            "CREATE VIEW external_selection_view
             AS SELECT sample_key FROM 'Selection_Samples';",
        )
        .unwrap();

        let error = inspect_read_only(&mut conn).unwrap_err();
        assert!(matches!(
            error,
            SelectionV2SchemaError::SchemaMismatch {
                table: "selection_v2_table_set",
                column: "revision",
                actual,
                ..
            } if actual.starts_with("classified drift:")
        ));
    }

    #[test]
    fn old_schema_with_external_foreign_key_dependency_is_classified_drift() {
        let mut conn = SqliteConnection::establish(":memory:").expect("isolated sqlite");
        install_pre_amendment_table_set(&mut conn, SelectionV2StoreMode::Test);
        conn.batch_execute(
            "CREATE TABLE external_selection_reference (
               id INTEGER PRIMARY KEY,
               sample_key TEXT NOT NULL,
               FOREIGN KEY(sample_key) REFERENCES selection_samples(sample_key)
             );",
        )
        .unwrap();

        let error = inspect_read_only(&mut conn).unwrap_err();
        assert!(matches!(
            error,
            SelectionV2SchemaError::SchemaMismatch {
                table: "selection_v2_table_set",
                column: "revision",
                actual,
                ..
            } if actual.starts_with("classified drift:")
        ));
    }

    #[test]
    fn case_variant_external_foreign_key_dependency_is_classified_drift() {
        let mut conn = SqliteConnection::establish(":memory:").expect("isolated sqlite");
        install_pre_amendment_table_set(&mut conn, SelectionV2StoreMode::Test);
        conn.batch_execute(
            "CREATE TABLE external_selection_case_reference (
               id INTEGER PRIMARY KEY,
               sample_key TEXT NOT NULL,
               FOREIGN KEY(sample_key) REFERENCES 'Selection_Samples'(sample_key)
             );",
        )
        .unwrap();

        let error = inspect_read_only(&mut conn).unwrap_err();
        assert!(matches!(
            error,
            SelectionV2SchemaError::SchemaMismatch {
                table: "selection_v2_table_set",
                column: "revision",
                actual,
                ..
            } if actual.starts_with("classified drift:")
        ));
    }

    #[test]
    fn startup_rejects_all_absent_tables_when_v2_audit_exists() {
        let mut conn = SqliteConnection::establish(":memory:").expect("isolated sqlite");

        let error = initialize_selection_v2_schema_with_audit_state(
            &mut conn,
            SelectionV2StoreMode::Test,
            true,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            SelectionV2SchemaError::SchemaMismatch {
                table: "selection_v2_table_set",
                column: "audit_phase",
                expected: "no v2 audit phase when all tables are absent",
                actual,
            } if actual == "present"
        ));
        let present = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM sqlite_master
             WHERE type='table' AND name LIKE 'selection_%'",
        )
        .get_result::<CountRow>(&mut conn)
        .unwrap();
        assert_eq!(
            present.count, 0,
            "audit-only startup must not create tables"
        );
    }

    #[test]
    fn frozen_field_registry_column_counts_are_exact() {
        let mut conn = connection(SelectionV2StoreMode::Test);
        for (table, expected) in V2_COLUMN_REGISTRY {
            let row = diesel::sql_query(format!(
                "SELECT group_concat(name, ',') AS value
                 FROM (SELECT name FROM pragma_table_xinfo('{table}') ORDER BY cid)"
            ))
            .get_result::<TextRow>(&mut conn)
            .unwrap();
            assert_eq!(row.value, expected, "frozen field registry for {table}");
        }
    }

    #[test]
    fn config_receipt_rejects_prepared_audit_hash_mismatch() {
        let mut conn = connection(SelectionV2StoreMode::Test);
        insert_envelope(&mut conn, "activation", "config_activation").unwrap();
        insert_manifest(
            &mut conn,
            "activation",
            "config_activation",
            "activated",
            None,
            1,
        )
        .unwrap();

        let result = conn.batch_execute(
            "INSERT INTO selection_v2_commit_receipts (
                subject_kind,subject_id,logical_subject_key,in_memory_payload_hash,
                recovery_envelope_content_hash,prepared_audit_hash,
                run_manifest_content_hash,staged_db_content_hash,committed_audit_hash,
                committed_at,content_hash
             ) VALUES (
                'config_activation','activation','activation','memory-hash',
                'envelope-hash','wrong-prepared-hash','manifest-hash','db-hash',
                'audit-hash','2026-07-28T00:00:04Z','receipt-hash'
             );",
        );
        assert!(result.is_err());
    }

    #[test]
    fn ingress_manifest_rejects_missing_activation_receipt() {
        let mut conn = connection(SelectionV2StoreMode::Test);
        insert_envelope(&mut conn, "ingress-no-activation", "ingress_run").unwrap();

        assert!(insert_manifest(
            &mut conn,
            "ingress-no-activation",
            "ingress_run",
            "completed",
            None,
            1,
        )
        .is_err());
    }

    #[test]
    fn ingress_manifest_rejects_available_batch_with_missing_fact_children() {
        let mut conn = connection(SelectionV2StoreMode::Test);
        insert_activation_chain(&mut conn);
        insert_envelope(&mut conn, "ingress-loss", "ingress_run").unwrap();
        conn.batch_execute(
            "INSERT INTO selection_source_batch_attempts (
                source_batch_attempt_id,ingress_run_id,config_activation_run_id,config_hash,
                generation_market_date,registered_feed_identity,registered_feed_snapshot_hash,
                request_hash,request_evidence_json,request_evidence_hash,
                feed_attempt_content_hash,status_kind,record_count,provider,source,
                source_at,observed_at,batch_id,batch_content_hash,failed_stage,reason_code,
                retryable,available_evidence_json,available_evidence_hash,error_detail_json,
                error_detail_hash,error_fingerprint,attempted_at,content_hash
             ) VALUES (
                'batch-loss','ingress-loss','activation','config-hash','2026-07-28',
                'feed','feed-snapshot','request','{}','request-evidence',
                'feed-attempt','available',1,
                'eastmoney','global-news','2026-07-28T00:00:00Z',
                '2026-07-28T00:00:01Z','batch-provider','batch-content',
                NULL,NULL,NULL,'{}','available-hash',NULL,NULL,NULL,
                '2026-07-28T00:00:02Z','batch-hash'
             );",
        )
        .unwrap();

        assert!(insert_manifest(
            &mut conn,
            "ingress-loss",
            "ingress_run",
            "completed",
            None,
            2,
        )
        .is_err());
    }

    #[test]
    fn generation_manifest_rejects_unreceipted_ingress_lineage() {
        let mut conn = connection(SelectionV2StoreMode::Test);
        insert_activation_chain(&mut conn);
        insert_envelope(&mut conn, "ingress-unreceipted", "ingress_run").unwrap();
        insert_fact(
            &mut conn,
            "fact-unreceipted",
            "ingress-unreceipted",
            "admitted",
        )
        .unwrap();
        insert_envelope(&mut conn, "generation-unreceipted", "generation_run").unwrap();

        assert!(insert_manifest(
            &mut conn,
            "generation-unreceipted",
            "generation_run",
            "verified_no_relation",
            Some("fact-unreceipted"),
            1,
        )
        .is_err());
    }

    #[test]
    fn generation_manifest_rejects_hard_rejection_without_exact_children() {
        let mut conn = connection(SelectionV2StoreMode::Test);
        stage_sample(&mut conn, "hard_rejected", 1, "[\"reject-hash\"]");

        assert!(insert_manifest(
            &mut conn,
            "generation-sample",
            "generation_run",
            "completed",
            Some("fact-sample"),
            4,
        )
        .is_err());
    }

    #[test]
    fn admitted_sample_rejects_direct_rejection_child() {
        let mut conn = connection(SelectionV2StoreMode::Test);
        stage_sample(&mut conn, "admitted", 0, "[]");

        let result = insert_rejection(&mut conn, 0, "reject-hash");
        assert!(result.is_err());
    }

    #[test]
    fn hard_rejection_requires_contiguous_ordinals_and_committed_hash_order() {
        let mut conn = connection(SelectionV2StoreMode::Test);
        stage_sample(&mut conn, "hard_rejected", 2, "[\"reject-0\",\"reject-1\"]");

        let out_of_order = insert_rejection(&mut conn, 1, "reject-1").unwrap_err();
        assert!(out_of_order
            .to_string()
            .contains("rejection parent/matrix mismatch"));

        insert_rejection(&mut conn, 0, "reject-0").unwrap();
        let wrong_hash = insert_rejection(&mut conn, 1, "wrong-hash").unwrap_err();
        assert!(wrong_hash
            .to_string()
            .contains("rejection parent/matrix mismatch"));
    }

    #[test]
    fn generation_manifest_is_last_for_every_generation_domain_table() {
        let mut conn = connection(SelectionV2StoreMode::Test);
        stage_sample(&mut conn, "admitted", 0, "[]");
        insert_manifest(
            &mut conn,
            "generation-sample",
            "generation_run",
            "completed",
            Some("fact-sample"),
            4,
        )
        .unwrap();

        let late = insert_resolved_direct_relation(
            &mut conn,
            "relation-late",
            "relation-key-late",
            "generation-sample",
            "fact-sample",
            "TEST_CODE_600001",
        )
        .unwrap_err();
        assert!(late.to_string().contains("manifest must be inserted last"));
    }

    #[test]
    fn outcome_manifest_rejects_missing_generation_receipt() {
        let mut conn = connection(SelectionV2StoreMode::Test);
        stage_sample(&mut conn, "admitted", 0, "[]");
        insert_envelope(&mut conn, "outcome-no-generation", "outcome_run").unwrap();
        insert_outcome_attempt(&mut conn, "outcome-no-generation", "expected_wait").unwrap();

        assert!(
            insert_outcome_manifest(&mut conn, "outcome-no-generation", "expected_wait", 2,)
                .is_err()
        );
    }

    #[test]
    fn d3_outcome_manifest_rejects_missing_preceding_phase_receipts() {
        let mut conn = connection(SelectionV2StoreMode::Test);
        insert_admitted_sample(&mut conn);
        insert_envelope(&mut conn, "outcome-d3", "outcome_run").unwrap();
        conn.batch_execute(
            "INSERT INTO selection_outcome_attempts (
                outcome_attempt_id,sample_key,phase,stored_due_date,outcome_run_id,
                request_hash,request_evidence_json,request_evidence_hash,
                result_code,reason_code,retryable,provider,source,source_at,
                observed_at,batch_id,batch_content_hash,available_evidence_json,
                available_evidence_hash,error_detail_json,error_detail_hash,error_fingerprint,
                settled_outcome_content_hash,attempted_at,content_hash
             ) VALUES (
                'attempt-outcome-d3','TEST_CODE_SAMPLE','d3_settled','2026-07-31',
                'outcome-d3',NULL,NULL,NULL,'expected_wait','market_session_unsettled',
                NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,
                '2026-07-31T00:00:02Z','attempt-hash'
             );",
        )
        .unwrap();

        let result = diesel::sql_query(
            "INSERT INTO selection_v2_run_stages (
                subject_id,subject_kind,logical_subject_key,in_memory_payload_hash,
                prepared_record_hash,expected_staged_row_count,staged_db_content_hash,
                recovery_envelope_content_hash,run_status,source_fact_key,
                config_activation_run_id,config_hash,config_snapshot_json_hash,
                config_activation_content_hash,config_activation_file_content_hash,
                config_effective_from,artifact_valid_from,artifact_expires_at,
                executable_revision,legacy_cutover_snapshot_hash,generation_market_date,
                aggregator_observed_at,ingress_source_batch_content_hash,outcome_phase,
                stored_due_date,staged_at,manifest_content_hash
             ) VALUES (
                'outcome-d3','outcome_run','outcome-d3','memory-hash','prepared-hash',2,
                'db-hash','envelope-hash','expected_wait',NULL,'activation','config-hash',
                NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,'d3_settled',
                '2026-07-31','2026-07-31T00:00:03Z','manifest-hash'
             )",
        )
        .execute(&mut conn);
        assert!(result.is_err());
    }
}
