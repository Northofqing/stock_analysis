//! BR-251 immutable attribution invocation, report, and failure store.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Datelike, Duration, FixedOffset, NaiveDate};
use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Integer, Nullable, Text};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::DatabaseManager;

const SCHEMA_VERSION: i32 = 1;
const RUN_CHAIN_GENESIS: &str = "BR251_ATTRIBUTION_RUN_CHAIN_GENESIS_V1";
const REPORT_CHAIN_GENESIS: &str = "BR251_ATTRIBUTION_REPORT_CHAIN_GENESIS_V1";
const FAILURE_CHAIN_GENESIS: &str = "BR251_ATTRIBUTION_FAILURE_CHAIN_GENESIS_V1";
const IMMUTABLE_TABLES: [(&str, &str); 6] = [
    ("attribution_run_audit", "run audit"),
    ("attribution_run_chain", "run chain"),
    ("attribution_report_revision", "report revision"),
    ("attribution_report_chain", "report chain"),
    ("attribution_failure_audit", "failure audit"),
    ("attribution_failure_chain", "failure chain"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributionRunMode {
    Scheduled,
    Range,
    Quarter,
}

impl AttributionRunMode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Scheduled => "scheduled",
            Self::Range => "range",
            Self::Quarter => "quarter",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "scheduled" => Ok(Self::Scheduled),
            "range" => Ok(Self::Range),
            "quarter" => Ok(Self::Quarter),
            _ => Err(format!("BR-251 unknown attribution run mode {value}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributionInvocation {
    pub mode: AttributionRunMode,
    pub target_from: NaiveDate,
    pub target_to: NaiveDate,
    pub rule_version: String,
    pub invoked_at: DateTime<FixedOffset>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttributionEvidenceHash {
    Available(String),
    Unavailable(String),
}

#[derive(Debug, Clone)]
pub struct AttributionReportAppend {
    pub invocation: AttributionInvocation,
    pub trade_hash: String,
    pub fee: AttributionEvidenceHash,
    pub stock_close_hash: String,
    pub benchmark_manifest_hash: String,
    pub calendar_authority_hash: String,
    pub regime: AttributionEvidenceHash,
    pub result_payload: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct AttributionFailureAppend {
    pub invocation: AttributionInvocation,
    pub stage: String,
    pub code: String,
    pub retryable: bool,
    pub source_summary_hash: String,
    pub redacted_message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributionRunReceipt {
    pub run_audit_id: i64,
    pub record_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributionReportReceipt {
    pub run: AttributionRunReceipt,
    pub report_revision_id: i64,
    pub report_identity: String,
    pub evidence_identity: String,
    pub series_identity: String,
    pub result_payload_hash: String,
    pub report_revision: i32,
    pub predecessor_report_id: Option<i64>,
    pub report_record_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributionFailureReceipt {
    pub run: AttributionRunReceipt,
    pub failure_audit_id: i64,
    pub failure_identity: String,
    pub failure_record_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttributionReportStoreError {
    Unavailable {
        reason_code: &'static str,
        retryable: bool,
        detail: String,
    },
    FailedIntegrity {
        reason_code: &'static str,
        detail: String,
    },
}

impl AttributionReportStoreError {
    pub fn reason_code(&self) -> &'static str {
        match self {
            Self::Unavailable { reason_code, .. } | Self::FailedIntegrity { reason_code, .. } => {
                reason_code
            }
        }
    }

    pub fn retryable(&self) -> bool {
        match self {
            Self::Unavailable { retryable, .. } => *retryable,
            Self::FailedIntegrity { .. } => false,
        }
    }

    pub fn detail(&self) -> &str {
        match self {
            Self::Unavailable { detail, .. } | Self::FailedIntegrity { detail, .. } => detail,
        }
    }
}

impl std::fmt::Display for AttributionReportStoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable {
                reason_code,
                retryable,
                detail,
            } => write!(
                formatter,
                "{reason_code} (unavailable, retryable={retryable}): {detail}"
            ),
            Self::FailedIntegrity {
                reason_code,
                detail,
            } => write!(formatter, "{reason_code} (failed_integrity): {detail}"),
        }
    }
}

impl std::error::Error for AttributionReportStoreError {}

pub struct AttributionReportStore<'a> {
    database: &'a DatabaseManager,
}

/// Explicit BR-251 database access selected by the standalone attribution CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributionDatabaseAccess {
    ReadOnly,
    AppendOnly,
}

/// Opaque, path-bound database session for the standalone attribution CLI.
///
/// Read-only sessions never migrate. Append-only sessions install only the
/// BR-159 acquisition, benchmark-segment, and attribution-report schemas.
pub struct AttributionDatabaseSession {
    database: DatabaseManager,
    database_path: PathBuf,
}

#[derive(QueryableByName)]
struct AttributionQueryOnlyRow {
    #[diesel(sql_type = Integer)]
    query_only: i32,
}

impl AttributionDatabaseSession {
    pub fn open(
        path: impl AsRef<Path>,
        access: AttributionDatabaseAccess,
    ) -> Result<Self, AttributionReportStoreError> {
        let path = path.as_ref();
        let supplied_metadata = std::fs::symlink_metadata(path).map_err(|_| {
            unavailable(
                "attribution_database_unavailable",
                false,
                "BR-251 explicit attribution database is unavailable",
            )
        })?;
        if supplied_metadata.file_type().is_symlink() || !supplied_metadata.is_file() {
            return Err(failed_integrity(
                "attribution_database_identity_invalid",
                "BR-251 explicit attribution database must be an existing regular file",
            ));
        }
        let database_path = std::fs::canonicalize(path).map_err(|_| {
            unavailable(
                "attribution_database_unavailable",
                false,
                "BR-251 explicit attribution database cannot be resolved",
            )
        })?;
        if !std::fs::metadata(&database_path)
            .map(|metadata| metadata.is_file())
            .unwrap_or(false)
        {
            return Err(failed_integrity(
                "attribution_database_identity_invalid",
                "BR-251 resolved attribution database is not a regular file",
            ));
        }

        let database_url = match access {
            AttributionDatabaseAccess::ReadOnly => {
                let mut url = url::Url::from_file_path(&database_path).map_err(|_| {
                    failed_integrity(
                        "attribution_database_identity_invalid",
                        "BR-251 attribution database path cannot form a SQLite URI",
                    )
                })?;
                url.query_pairs_mut().append_pair("mode", "ro");
                url.to_string()
            }
            AttributionDatabaseAccess::AppendOnly => database_path
                .to_str()
                .ok_or_else(|| {
                    failed_integrity(
                        "attribution_database_identity_invalid",
                        "BR-251 attribution database path must be valid UTF-8",
                    )
                })?
                .to_owned(),
        };
        let pool = super::build_sqlite_pool_with_size(database_url, 1).map_err(|_| {
            unavailable(
                "attribution_database_unavailable",
                true,
                "BR-251 attribution database pool is unavailable",
            )
        })?;
        let database = DatabaseManager {
            pool,
            selection_connection_source: None,
            selection_schema_authority: None,
        };
        let mut connection = database.get_conn().map_err(|_| {
            unavailable(
                "attribution_database_unavailable",
                true,
                "BR-251 attribution database connection is unavailable",
            )
        })?;
        match access {
            AttributionDatabaseAccess::ReadOnly => {
                connection
                    .batch_execute("PRAGMA query_only = ON;")
                    .map_err(|_| {
                        failed_integrity(
                            "attribution_read_only_boundary_failed",
                            "BR-251 attribution preview cannot enforce query_only",
                        )
                    })?;
                let query_only = diesel::sql_query("PRAGMA query_only")
                    .get_result::<AttributionQueryOnlyRow>(&mut connection)
                    .map_err(|_| {
                        failed_integrity(
                            "attribution_read_only_boundary_failed",
                            "BR-251 attribution preview cannot verify query_only",
                        )
                    })?
                    .query_only;
                if query_only != 1 {
                    return Err(failed_integrity(
                        "attribution_read_only_boundary_failed",
                        "BR-251 attribution preview query_only verification failed",
                    ));
                }
            }
            AttributionDatabaseAccess::AppendOnly => {
                super::data_acquisition_audit::create_schema(&mut connection).map_err(|_| {
                    unavailable(
                        "attribution_schema_unavailable",
                        true,
                        "BR-251 acquisition audit schema initialization failed",
                    )
                })?;
                super::benchmark_segments::create_schema(&mut connection).map_err(|_| {
                    unavailable(
                        "attribution_schema_unavailable",
                        true,
                        "BR-251 benchmark schema initialization failed",
                    )
                })?;
                create_schema(&mut connection).map_err(|_| {
                    unavailable(
                        "attribution_schema_unavailable",
                        true,
                        "BR-251 attribution schema initialization failed",
                    )
                })?;
            }
        }
        drop(connection);
        Ok(Self {
            database,
            database_path,
        })
    }

    pub fn database(&self) -> &DatabaseManager {
        &self.database
    }

    pub fn database_path(&self) -> &Path {
        &self.database_path
    }
}

#[cfg(test)]
pub(crate) fn test_runner_database_manager(path: &std::path::Path) -> DatabaseManager {
    let database_url = path.to_string_lossy().into_owned();
    let mut bootstrap = SqliteConnection::establish(&database_url)
        .expect("TEST_CODE establish replay runner database");
    diesel::sql_query("PRAGMA journal_mode = WAL")
        .execute(&mut bootstrap)
        .expect("TEST_CODE replay runner WAL");
    drop(bootstrap);
    let pool =
        super::build_sqlite_pool_with_size(database_url, 1).expect("TEST_CODE replay runner pool");
    {
        let mut connection = pool
            .get()
            .expect("TEST_CODE replay runner schema connection");
        super::data_acquisition_audit::create_schema(&mut connection)
            .expect("TEST_CODE acquisition schema");
        super::benchmark_segments::create_schema(&mut connection)
            .expect("TEST_CODE benchmark schema");
        create_schema(&mut connection).expect("TEST_CODE attribution report schema");
    }
    DatabaseManager {
        pool,
        selection_connection_source: None,
        selection_schema_authority: None,
    }
}

#[derive(Debug, Clone)]
struct PreparedInvocation {
    mode: String,
    target_from: String,
    target_to: String,
    rule_version: String,
    invoked_at: String,
    series_identity: String,
}

#[derive(Debug, Clone)]
struct PreparedReport {
    invocation: PreparedInvocation,
    trade_hash: String,
    fee_status: String,
    fee_value: String,
    stock_close_hash: String,
    benchmark_manifest_hash: String,
    calendar_authority_hash: String,
    regime_status: String,
    regime_value: String,
    result_payload_json: String,
    result_payload_hash: String,
    evidence_identity: String,
    report_identity: String,
}

#[derive(Debug, Clone)]
struct PreparedFailure {
    invocation: PreparedInvocation,
    stage: String,
    code: String,
    retryable: bool,
    source_summary_hash: String,
    redacted_message: String,
    failure_content_hash: String,
}

fn failed_integrity(
    reason_code: &'static str,
    detail: impl Into<String>,
) -> AttributionReportStoreError {
    AttributionReportStoreError::FailedIntegrity {
        reason_code,
        detail: detail.into(),
    }
}

fn unavailable(
    reason_code: &'static str,
    retryable: bool,
    detail: impl Into<String>,
) -> AttributionReportStoreError {
    AttributionReportStoreError::Unavailable {
        reason_code,
        retryable,
        detail: detail.into(),
    }
}

fn typed_store_error(error: AttributionReportStoreError) -> diesel::result::Error {
    diesel::result::Error::QueryBuilderError(Box::new(error))
}

fn integrity_query(reason_code: &'static str, detail: impl Into<String>) -> diesel::result::Error {
    typed_store_error(failed_integrity(reason_code, detail))
}

fn hash_with_domain(domain: &[u8], parts: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    for part in parts {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part);
    }
    hex::encode(hasher.finalize())
}

fn is_lower_hex_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_hash(value: &str, field: &str) -> Result<(), AttributionReportStoreError> {
    if !is_lower_hex_hash(value) {
        return Err(failed_integrity(
            "attribution_input_hash_invalid",
            format!("BR-251 {field} must be exactly 64 lowercase hex characters"),
        ));
    }
    Ok(())
}

fn validate_stable_code(value: &str, field: &str) -> Result<(), AttributionReportStoreError> {
    if value.is_empty()
        || value.len() > 128
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'_' | b'-' | b'.' | b':')
        })
    {
        return Err(failed_integrity(
            "attribution_input_code_invalid",
            format!("BR-251 {field} must be a nonblank stable redacted code"),
        ));
    }
    Ok(())
}

fn natural_quarter_end(start: NaiveDate) -> Option<NaiveDate> {
    if start.day() != 1 || !matches!(start.month(), 1 | 4 | 7 | 10) {
        return None;
    }
    let next = if start.month() == 10 {
        NaiveDate::from_ymd_opt(start.year() + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(start.year(), start.month() + 3, 1)
    }?;
    Some(next - Duration::days(1))
}

fn prepare_invocation(
    invocation: AttributionInvocation,
) -> Result<PreparedInvocation, AttributionReportStoreError> {
    if invocation.rule_version.trim().is_empty()
        || invocation.rule_version.len() > 128
        || invocation.rule_version != invocation.rule_version.trim()
    {
        return Err(failed_integrity(
            "attribution_invocation_rule_invalid",
            "BR-251 attribution rule version must be nonblank, bounded and canonical",
        ));
    }
    if invocation.invoked_at.offset().local_minus_utc() != 8 * 60 * 60 {
        return Err(failed_integrity(
            "attribution_invocation_timezone_invalid",
            "BR-251 attribution invocation time must use explicit +08:00",
        ));
    }
    match invocation.mode {
        AttributionRunMode::Scheduled if invocation.target_from != invocation.target_to => {
            return Err(failed_integrity(
                "attribution_invocation_range_invalid",
                "BR-251 scheduled attribution requires exactly one target day",
            ));
        }
        AttributionRunMode::Range if invocation.target_from > invocation.target_to => {
            return Err(failed_integrity(
                "attribution_invocation_range_invalid",
                "BR-251 attribution range is reversed",
            ));
        }
        AttributionRunMode::Quarter
            if natural_quarter_end(invocation.target_from) != Some(invocation.target_to) =>
        {
            return Err(failed_integrity(
                "attribution_invocation_quarter_invalid",
                "BR-251 quarter attribution requires exact natural-quarter bounds",
            ));
        }
        _ => {}
    }
    let mode = invocation.mode.as_str().to_string();
    let target_from = invocation.target_from.format("%Y-%m-%d").to_string();
    let target_to = invocation.target_to.format("%Y-%m-%d").to_string();
    let series_identity = hash_with_domain(
        b"BR251_ATTRIBUTION_SERIES_IDENTITY_V1",
        &[
            mode.as_bytes(),
            target_from.as_bytes(),
            target_to.as_bytes(),
            invocation.rule_version.as_bytes(),
        ],
    );
    Ok(PreparedInvocation {
        mode,
        target_from,
        target_to,
        rule_version: invocation.rule_version,
        invoked_at: invocation.invoked_at.to_rfc3339(),
        series_identity,
    })
}

fn prepare_evidence(
    evidence: AttributionEvidenceHash,
    field: &str,
) -> Result<(String, String), AttributionReportStoreError> {
    match evidence {
        AttributionEvidenceHash::Available(hash) => {
            validate_hash(&hash, field)?;
            Ok(("available".to_string(), hash))
        }
        AttributionEvidenceHash::Unavailable(code) => {
            validate_stable_code(&code, field)?;
            Ok(("unavailable".to_string(), code))
        }
    }
}

fn write_canonical_json(
    value: &serde_json::Value,
    output: &mut String,
) -> Result<(), AttributionReportStoreError> {
    match value {
        serde_json::Value::Null => output.push_str("null"),
        serde_json::Value::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        serde_json::Value::Number(value) => output.push_str(&value.to_string()),
        serde_json::Value::String(value) => {
            output.push_str(&serde_json::to_string(value).map_err(|error| {
                failed_integrity(
                    "attribution_result_json_invalid",
                    format!("BR-251 serialize result string: {error}"),
                )
            })?)
        }
        serde_json::Value::Array(values) => {
            output.push('[');
            for (index, value) in values.iter().enumerate() {
                if index != 0 {
                    output.push(',');
                }
                write_canonical_json(value, output)?;
            }
            output.push(']');
        }
        serde_json::Value::Object(values) => {
            output.push('{');
            let mut keys = values.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            for (index, key) in keys.into_iter().enumerate() {
                if index != 0 {
                    output.push(',');
                }
                output.push_str(&serde_json::to_string(key).map_err(|error| {
                    failed_integrity(
                        "attribution_result_json_invalid",
                        format!("BR-251 serialize result key: {error}"),
                    )
                })?);
                output.push(':');
                write_canonical_json(&values[key], output)?;
            }
            output.push('}');
        }
    }
    Ok(())
}

fn canonical_result_json(value: &serde_json::Value) -> Result<String, AttributionReportStoreError> {
    if !value.is_object() {
        return Err(failed_integrity(
            "attribution_result_json_invalid",
            "BR-251 attribution result payload must be a structured JSON object",
        ));
    }
    let mut output = String::new();
    write_canonical_json(value, &mut output)?;
    if output.len() > 4 * 1024 * 1024 {
        return Err(failed_integrity(
            "attribution_result_json_oversized",
            "BR-251 attribution result payload exceeds 4 MiB",
        ));
    }
    Ok(output)
}

fn prepare_report(
    input: AttributionReportAppend,
) -> Result<PreparedReport, AttributionReportStoreError> {
    validate_hash(&input.trade_hash, "trade_hash")?;
    validate_hash(&input.stock_close_hash, "stock_close_hash")?;
    validate_hash(&input.benchmark_manifest_hash, "benchmark_manifest_hash")?;
    validate_hash(&input.calendar_authority_hash, "calendar_authority_hash")?;
    let invocation = prepare_invocation(input.invocation)?;
    let (fee_status, fee_value) = prepare_evidence(input.fee, "fee evidence")?;
    let (regime_status, regime_value) = prepare_evidence(input.regime, "regime evidence")?;
    let result_payload_json = canonical_result_json(&input.result_payload)?;
    let result_payload_hash = hash_with_domain(
        b"BR251_ATTRIBUTION_RESULT_PAYLOAD_V1",
        &[result_payload_json.as_bytes()],
    );
    let evidence_identity = hash_with_domain(
        b"BR251_ATTRIBUTION_EVIDENCE_IDENTITY_V1",
        &[
            invocation.series_identity.as_bytes(),
            input.trade_hash.as_bytes(),
            fee_status.as_bytes(),
            fee_value.as_bytes(),
            input.stock_close_hash.as_bytes(),
            input.benchmark_manifest_hash.as_bytes(),
            input.calendar_authority_hash.as_bytes(),
            regime_status.as_bytes(),
            regime_value.as_bytes(),
        ],
    );
    let report_identity = hash_with_domain(
        b"BR251_ATTRIBUTION_REPORT_IDENTITY_V1",
        &[evidence_identity.as_bytes(), result_payload_hash.as_bytes()],
    );
    Ok(PreparedReport {
        invocation,
        trade_hash: input.trade_hash,
        fee_status,
        fee_value,
        stock_close_hash: input.stock_close_hash,
        benchmark_manifest_hash: input.benchmark_manifest_hash,
        calendar_authority_hash: input.calendar_authority_hash,
        regime_status,
        regime_value,
        result_payload_json,
        result_payload_hash,
        evidence_identity,
        report_identity,
    })
}

fn prepare_failure(
    input: AttributionFailureAppend,
) -> Result<PreparedFailure, AttributionReportStoreError> {
    let invocation = prepare_invocation(input.invocation)?;
    validate_stable_code(&input.stage, "failure stage")?;
    validate_stable_code(&input.code, "failure code")?;
    validate_hash(&input.source_summary_hash, "source_summary_hash")?;
    if input.redacted_message.trim().is_empty()
        || input.redacted_message != input.redacted_message.trim()
        || input.redacted_message.len() > 4096
        || input.redacted_message.chars().any(char::is_control)
    {
        return Err(failed_integrity(
            "attribution_failure_message_invalid",
            "BR-251 redacted failure message must be nonblank, canonical and at most 4096 bytes",
        ));
    }
    let failure_content_hash = hash_with_domain(
        b"BR251_ATTRIBUTION_FAILURE_CONTENT_V1",
        &[
            invocation.series_identity.as_bytes(),
            input.stage.as_bytes(),
            input.code.as_bytes(),
            if input.retryable { b"1" } else { b"0" },
            input.source_summary_hash.as_bytes(),
            input.redacted_message.as_bytes(),
        ],
    );
    Ok(PreparedFailure {
        invocation,
        stage: input.stage,
        code: input.code,
        retryable: input.retryable,
        source_summary_hash: input.source_summary_hash,
        redacted_message: input.redacted_message,
        failure_content_hash,
    })
}

#[derive(QueryableByName)]
struct CountRow {
    #[diesel(sql_type = BigInt)]
    count: i64,
}

#[derive(QueryableByName)]
struct SequenceRow {
    #[diesel(sql_type = Nullable<BigInt>)]
    seq: Option<i64>,
}

#[derive(QueryableByName)]
struct TriggerSchemaRow {
    #[diesel(sql_type = Text)]
    name: String,
    #[diesel(sql_type = Text)]
    table_name: String,
    #[diesel(sql_type = Nullable<Text>)]
    sql: Option<String>,
}

#[derive(Debug, Clone, QueryableByName, Serialize)]
struct PersistedRun {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = Integer)]
    schema_version: i32,
    #[diesel(sql_type = Text)]
    mode: String,
    #[diesel(sql_type = Text)]
    target_from: String,
    #[diesel(sql_type = Text)]
    target_to: String,
    #[diesel(sql_type = Text)]
    rule_version: String,
    #[diesel(sql_type = Text)]
    invoked_at: String,
    #[diesel(sql_type = Text)]
    series_identity: String,
    #[diesel(sql_type = Text)]
    outcome: String,
    #[diesel(sql_type = Text)]
    outcome_identity: String,
    #[diesel(sql_type = Text)]
    created_at: String,
    #[diesel(sql_type = Text)]
    retention_deadline: String,
}

#[derive(Debug, Clone, QueryableByName)]
struct PersistedChain {
    #[diesel(sql_type = BigInt)]
    row_id: i64,
    #[diesel(sql_type = Text)]
    previous_hash: String,
    #[diesel(sql_type = Text)]
    record_hash: String,
    #[diesel(sql_type = Text)]
    created_at: String,
    #[diesel(sql_type = Text)]
    retention_deadline: String,
}

#[derive(Debug, QueryableByName)]
struct RetentionWindow {
    #[diesel(sql_type = Text)]
    created_at: String,
    #[diesel(sql_type = Text)]
    retention_deadline: String,
}

#[derive(Debug, Clone, QueryableByName, Serialize)]
struct PersistedReport {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = Integer)]
    schema_version: i32,
    #[diesel(sql_type = Text)]
    report_identity: String,
    #[diesel(sql_type = Text)]
    series_identity: String,
    #[diesel(sql_type = Text)]
    evidence_identity: String,
    #[diesel(sql_type = BigInt)]
    source_run_id: i64,
    #[diesel(sql_type = Text)]
    mode: String,
    #[diesel(sql_type = Text)]
    target_from: String,
    #[diesel(sql_type = Text)]
    target_to: String,
    #[diesel(sql_type = Text)]
    rule_version: String,
    #[diesel(sql_type = Text)]
    trade_hash: String,
    #[diesel(sql_type = Text)]
    fee_status: String,
    #[diesel(sql_type = Text)]
    fee_value: String,
    #[diesel(sql_type = Text)]
    stock_close_hash: String,
    #[diesel(sql_type = Text)]
    benchmark_manifest_hash: String,
    #[diesel(sql_type = Text)]
    calendar_authority_hash: String,
    #[diesel(sql_type = Text)]
    regime_status: String,
    #[diesel(sql_type = Text)]
    regime_value: String,
    #[diesel(sql_type = Text)]
    result_payload_json: String,
    #[diesel(sql_type = Text)]
    result_payload_hash: String,
    #[diesel(sql_type = Integer)]
    revision: i32,
    #[diesel(sql_type = Nullable<BigInt>)]
    predecessor_report_id: Option<i64>,
    #[diesel(sql_type = Text)]
    created_at: String,
    #[diesel(sql_type = Text)]
    retention_deadline: String,
}

#[derive(Debug, Clone, QueryableByName, Serialize)]
struct PersistedFailure {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = Integer)]
    schema_version: i32,
    #[diesel(sql_type = BigInt)]
    source_run_id: i64,
    #[diesel(sql_type = Text)]
    failure_identity: String,
    #[diesel(sql_type = Text)]
    failure_content_hash: String,
    #[diesel(sql_type = Text)]
    stage: String,
    #[diesel(sql_type = Text)]
    code: String,
    #[diesel(sql_type = Integer)]
    retryable: i32,
    #[diesel(sql_type = Text)]
    source_summary_hash: String,
    #[diesel(sql_type = Text)]
    redacted_message: String,
    #[diesel(sql_type = Text)]
    created_at: String,
    #[diesel(sql_type = Text)]
    retention_deadline: String,
}

#[derive(Debug)]
struct ValidatedState {
    runs: Vec<PersistedRun>,
    run_chains: Vec<PersistedChain>,
    reports: Vec<PersistedReport>,
    report_chains: Vec<PersistedChain>,
    failures: Vec<PersistedFailure>,
    failure_chains: Vec<PersistedChain>,
}

pub(super) fn create_schema(conn: &mut SqliteConnection) -> diesel::QueryResult<()> {
    let existing_table_count = diesel::sql_query(
        "SELECT COUNT(*) AS count FROM sqlite_master
         WHERE type = 'table' AND name IN (
            'attribution_run_audit', 'attribution_run_chain',
            'attribution_report_revision', 'attribution_report_chain',
            'attribution_failure_audit', 'attribution_failure_chain'
         )",
    )
    .get_result::<CountRow>(conn)?
    .count;
    if existing_table_count != 0 {
        validate_immutable_triggers(conn)?;
    }
    diesel::sql_query(
        "CREATE TABLE IF NOT EXISTS attribution_run_audit (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            schema_version INTEGER NOT NULL CHECK(schema_version = 1),
            mode TEXT NOT NULL CHECK(mode IN ('scheduled', 'range', 'quarter')),
            target_from TEXT NOT NULL,
            target_to TEXT NOT NULL,
            rule_version TEXT NOT NULL CHECK(length(trim(rule_version)) > 0),
            invoked_at TEXT NOT NULL,
            series_identity TEXT NOT NULL CHECK(length(series_identity) = 64),
            outcome TEXT NOT NULL CHECK(outcome IN (
                'report_appended', 'report_reused', 'failure'
            )),
            outcome_identity TEXT NOT NULL CHECK(length(outcome_identity) = 64),
            created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
            retention_deadline TEXT NOT NULL DEFAULT (
                strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '+5 years')
            )
        )",
    )
    .execute(conn)?;
    immutable_triggers(conn, "attribution_run_audit", "run audit")?;

    diesel::sql_query(
        "CREATE TABLE IF NOT EXISTS attribution_run_chain (
            run_audit_id INTEGER PRIMARY KEY NOT NULL,
            previous_hash TEXT NOT NULL,
            record_hash TEXT NOT NULL UNIQUE CHECK(length(record_hash) = 64),
            created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
            retention_deadline TEXT NOT NULL DEFAULT (
                strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '+5 years')
            ),
            FOREIGN KEY(run_audit_id) REFERENCES attribution_run_audit(id)
        )",
    )
    .execute(conn)?;
    immutable_triggers(conn, "attribution_run_chain", "run chain")?;

    diesel::sql_query(
        "CREATE TABLE IF NOT EXISTS attribution_report_revision (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            schema_version INTEGER NOT NULL CHECK(schema_version = 1),
            report_identity TEXT NOT NULL UNIQUE CHECK(length(report_identity) = 64),
            series_identity TEXT NOT NULL CHECK(length(series_identity) = 64),
            evidence_identity TEXT NOT NULL UNIQUE CHECK(length(evidence_identity) = 64),
            source_run_id INTEGER NOT NULL,
            mode TEXT NOT NULL CHECK(mode IN ('scheduled', 'range', 'quarter')),
            target_from TEXT NOT NULL,
            target_to TEXT NOT NULL,
            rule_version TEXT NOT NULL CHECK(length(trim(rule_version)) > 0),
            trade_hash TEXT NOT NULL CHECK(length(trade_hash) = 64),
            fee_status TEXT NOT NULL CHECK(fee_status IN ('available', 'unavailable')),
            fee_value TEXT NOT NULL CHECK(length(trim(fee_value)) > 0),
            stock_close_hash TEXT NOT NULL CHECK(length(stock_close_hash) = 64),
            benchmark_manifest_hash TEXT NOT NULL CHECK(length(benchmark_manifest_hash) = 64),
            calendar_authority_hash TEXT NOT NULL CHECK(length(calendar_authority_hash) = 64),
            regime_status TEXT NOT NULL CHECK(regime_status IN ('available', 'unavailable')),
            regime_value TEXT NOT NULL CHECK(length(trim(regime_value)) > 0),
            result_payload_json TEXT NOT NULL CHECK(length(trim(result_payload_json)) > 0),
            result_payload_hash TEXT NOT NULL CHECK(length(result_payload_hash) = 64),
            revision INTEGER NOT NULL CHECK(revision > 0),
            predecessor_report_id INTEGER,
            created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
            retention_deadline TEXT NOT NULL DEFAULT (
                strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '+5 years')
            ),
            UNIQUE(series_identity, revision),
            FOREIGN KEY(source_run_id) REFERENCES attribution_run_audit(id),
            FOREIGN KEY(predecessor_report_id) REFERENCES attribution_report_revision(id)
        )",
    )
    .execute(conn)?;
    immutable_triggers(conn, "attribution_report_revision", "report revision")?;

    diesel::sql_query(
        "CREATE TABLE IF NOT EXISTS attribution_report_chain (
            report_revision_id INTEGER PRIMARY KEY NOT NULL,
            previous_hash TEXT NOT NULL,
            record_hash TEXT NOT NULL UNIQUE CHECK(length(record_hash) = 64),
            created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
            retention_deadline TEXT NOT NULL DEFAULT (
                strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '+5 years')
            ),
            FOREIGN KEY(report_revision_id) REFERENCES attribution_report_revision(id)
        )",
    )
    .execute(conn)?;
    immutable_triggers(conn, "attribution_report_chain", "report chain")?;

    diesel::sql_query(
        "CREATE TABLE IF NOT EXISTS attribution_failure_audit (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            schema_version INTEGER NOT NULL CHECK(schema_version = 1),
            source_run_id INTEGER NOT NULL UNIQUE,
            failure_identity TEXT NOT NULL UNIQUE CHECK(length(failure_identity) = 64),
            failure_content_hash TEXT NOT NULL CHECK(length(failure_content_hash) = 64),
            stage TEXT NOT NULL CHECK(length(trim(stage)) > 0),
            code TEXT NOT NULL CHECK(length(trim(code)) > 0),
            retryable INTEGER NOT NULL CHECK(retryable IN (0, 1)),
            source_summary_hash TEXT NOT NULL CHECK(length(source_summary_hash) = 64),
            redacted_message TEXT NOT NULL CHECK(length(trim(redacted_message)) > 0),
            created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
            retention_deadline TEXT NOT NULL DEFAULT (
                strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '+5 years')
            ),
            FOREIGN KEY(source_run_id) REFERENCES attribution_run_audit(id)
        )",
    )
    .execute(conn)?;
    immutable_triggers(conn, "attribution_failure_audit", "failure audit")?;

    diesel::sql_query(
        "CREATE TABLE IF NOT EXISTS attribution_failure_chain (
            failure_audit_id INTEGER PRIMARY KEY NOT NULL,
            previous_hash TEXT NOT NULL,
            record_hash TEXT NOT NULL UNIQUE CHECK(length(record_hash) = 64),
            created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
            retention_deadline TEXT NOT NULL DEFAULT (
                strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '+5 years')
            ),
            FOREIGN KEY(failure_audit_id) REFERENCES attribution_failure_audit(id)
        )",
    )
    .execute(conn)?;
    immutable_triggers(conn, "attribution_failure_chain", "failure chain")?;
    validate_all_state(conn).map(|_| ())
}

fn validate_chain_cardinalities(conn: &mut SqliteConnection) -> diesel::QueryResult<()> {
    for (rows, chain) in [
        ("attribution_run_audit", "attribution_run_chain"),
        ("attribution_report_revision", "attribution_report_chain"),
        ("attribution_failure_audit", "attribution_failure_chain"),
    ] {
        let row_count = diesel::sql_query(format!("SELECT COUNT(*) AS count FROM {rows}"))
            .get_result::<CountRow>(conn)?
            .count;
        let chain_count = diesel::sql_query(format!("SELECT COUNT(*) AS count FROM {chain}"))
            .get_result::<CountRow>(conn)?
            .count;
        if row_count != chain_count {
            return Err(diesel::result::Error::QueryBuilderError(Box::new(
                std::io::Error::other(format!(
                    "BR-251 attribution chain length mismatch: {rows}={row_count}, {chain}={chain_count}"
                )),
            )));
        }
        for table in [rows, chain] {
            let invalid_retention = diesel::sql_query(format!(
                "SELECT COUNT(*) AS count FROM {table}
                 WHERE julianday(created_at) IS NULL
                    OR julianday(retention_deadline) IS NULL
                    OR julianday(retention_deadline) < julianday(created_at, '+5 years')"
            ))
            .get_result::<CountRow>(conn)?
            .count;
            if invalid_retention != 0 {
                return Err(diesel::result::Error::QueryBuilderError(Box::new(
                    std::io::Error::other(format!(
                        "BR-251 attribution retention is shorter than five years in {table}"
                    )),
                )));
            }
        }
    }
    Ok(())
}

fn validate_autoincrement_highwater(
    conn: &mut SqliteConnection,
    table: &str,
    ids: &[i64],
) -> diesel::QueryResult<()> {
    let sequence = diesel::sql_query("SELECT seq FROM sqlite_sequence WHERE name = ?")
        .bind::<Text, _>(table)
        .get_result::<SequenceRow>(conn)
        .optional()?;
    let exact_sequence = match sequence {
        None => ids.is_empty(),
        Some(SequenceRow {
            seq: Some(highwater),
        }) if highwater > 0 => ids.iter().copied().eq(1..=highwater),
        Some(_) => false,
    };
    if !exact_sequence {
        return Err(integrity_query(
            "attribution_sequence_highwater_invalid",
            format!(
                "BR-251 {table} AUTOINCREMENT high-water does not match its full append-only identity sequence"
            ),
        ));
    }
    Ok(())
}

fn immutable_triggers(
    conn: &mut SqliteConnection,
    table: &str,
    label: &str,
) -> diesel::QueryResult<()> {
    diesel::sql_query(format!(
        "CREATE TRIGGER IF NOT EXISTS trg_{table}_no_update
         BEFORE UPDATE ON {table}
         BEGIN SELECT RAISE(ABORT, 'BR-251 attribution {label} is immutable'); END",
    ))
    .execute(conn)?;
    diesel::sql_query(format!(
        "CREATE TRIGGER IF NOT EXISTS trg_{table}_no_delete
         BEFORE DELETE ON {table}
         BEGIN SELECT RAISE(ABORT, 'BR-251 attribution {label} retention is at least five years'); END",
    ))
    .execute(conn)?;
    Ok(())
}

fn expected_trigger_sql(table: &str, label: &str, event: &str) -> String {
    let action = match event {
        "UPDATE" => format!("BR-251 attribution {label} is immutable"),
        "DELETE" => format!("BR-251 attribution {label} retention is at least five years"),
        _ => unreachable!("immutable trigger event is fixed by the module"),
    };
    format!(
        "CREATE TRIGGER trg_{table}_no_{} BEFORE {event} ON {table} \
         BEGIN SELECT RAISE(ABORT, '{action}'); END",
        event.to_ascii_lowercase()
    )
}

fn normalized_sql(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn validate_immutable_triggers(conn: &mut SqliteConnection) -> diesel::QueryResult<()> {
    for (table, label) in IMMUTABLE_TABLES {
        for event in ["UPDATE", "DELETE"] {
            let expected = expected_trigger_sql(table, label, event);
            let name = format!("trg_{table}_no_{}", event.to_ascii_lowercase());
            let row = diesel::sql_query(
                "SELECT name, tbl_name AS table_name, sql
                 FROM sqlite_master WHERE type = 'trigger' AND name = ?",
            )
            .bind::<Text, _>(&name)
            .get_result::<TriggerSchemaRow>(conn)
            .optional()?;
            let exact = row.is_some_and(|row| {
                row.name == name
                    && row.table_name == table
                    && row
                        .sql
                        .is_some_and(|sql| normalized_sql(&sql) == normalized_sql(&expected))
            });
            if !exact {
                return Err(integrity_query(
                    "attribution_trigger_definition_invalid",
                    format!(
                        "BR-251 immutable trigger {name} does not match its canonical table, timing, event and abort action"
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn load_runs(conn: &mut SqliteConnection) -> diesel::QueryResult<Vec<PersistedRun>> {
    diesel::sql_query(
        "SELECT id, schema_version, mode, target_from, target_to, rule_version,
                invoked_at, series_identity, outcome, outcome_identity, created_at,
                retention_deadline
         FROM attribution_run_audit ORDER BY id ASC",
    )
    .load(conn)
}

fn load_run_chains(conn: &mut SqliteConnection) -> diesel::QueryResult<Vec<PersistedChain>> {
    diesel::sql_query(
        "SELECT run_audit_id AS row_id, previous_hash, record_hash, created_at,
                retention_deadline
         FROM attribution_run_chain ORDER BY run_audit_id ASC",
    )
    .load(conn)
}

fn load_reports(conn: &mut SqliteConnection) -> diesel::QueryResult<Vec<PersistedReport>> {
    diesel::sql_query(
        "SELECT id, schema_version, report_identity, series_identity, evidence_identity,
                source_run_id, mode, target_from, target_to, rule_version, trade_hash,
                fee_status, fee_value, stock_close_hash, benchmark_manifest_hash,
                calendar_authority_hash, regime_status, regime_value, result_payload_json,
                result_payload_hash, revision, predecessor_report_id, created_at,
                retention_deadline
         FROM attribution_report_revision ORDER BY id ASC",
    )
    .load(conn)
}

fn load_report_chains(conn: &mut SqliteConnection) -> diesel::QueryResult<Vec<PersistedChain>> {
    diesel::sql_query(
        "SELECT report_revision_id AS row_id, previous_hash, record_hash, created_at,
                retention_deadline
         FROM attribution_report_chain ORDER BY report_revision_id ASC",
    )
    .load(conn)
}

fn load_failures(conn: &mut SqliteConnection) -> diesel::QueryResult<Vec<PersistedFailure>> {
    diesel::sql_query(
        "SELECT id, schema_version, source_run_id, failure_identity,
                failure_content_hash, stage, code, retryable, source_summary_hash,
                redacted_message, created_at, retention_deadline
         FROM attribution_failure_audit ORDER BY id ASC",
    )
    .load(conn)
}

fn load_failure_chains(conn: &mut SqliteConnection) -> diesel::QueryResult<Vec<PersistedChain>> {
    diesel::sql_query(
        "SELECT failure_audit_id AS row_id, previous_hash, record_hash, created_at,
                retention_deadline
         FROM attribution_failure_chain ORDER BY failure_audit_id ASC",
    )
    .load(conn)
}

fn chain_record_hash<T: Serialize>(
    domain: &[u8],
    previous_hash: &str,
    row: &T,
    created_at: &str,
    retention_deadline: &str,
) -> diesel::QueryResult<String> {
    let bytes = serde_json::to_vec(row).map_err(|error| {
        integrity_query(
            "attribution_chain_serialization_failed",
            format!("BR-251 serialize attribution chain row: {error}"),
        )
    })?;
    Ok(hash_with_domain(
        domain,
        &[
            previous_hash.as_bytes(),
            &bytes,
            created_at.as_bytes(),
            retention_deadline.as_bytes(),
        ],
    ))
}

fn new_retention_window(conn: &mut SqliteConnection) -> diesel::QueryResult<RetentionWindow> {
    diesel::sql_query(
        "SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now') AS created_at,
                strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '+5 years') AS retention_deadline",
    )
    .get_result(conn)
}

fn validate_chain<T: Serialize>(
    rows: &[T],
    ids: impl Iterator<Item = i64>,
    chains: &[PersistedChain],
    genesis: &str,
    domain: &[u8],
    family: &str,
) -> diesel::QueryResult<()> {
    let ids = ids.collect::<Vec<_>>();
    if rows.len() != chains.len() || rows.len() != ids.len() {
        return Err(integrity_query(
            "attribution_chain_length_mismatch",
            format!(
                "BR-251 {family} chain length mismatch: rows={}, chains={}",
                rows.len(),
                chains.len()
            ),
        ));
    }
    let mut previous = genesis.to_string();
    for ((row, id), chain) in rows.iter().zip(ids).zip(chains) {
        if chain.row_id != id
            || chain.previous_hash != previous
            || !is_lower_hex_hash(&chain.record_hash)
        {
            return Err(integrity_query(
                "attribution_chain_linkage_invalid",
                format!("BR-251 {family} chain linkage mismatch at row {id}"),
            ));
        }
        let expected = chain_record_hash(
            domain,
            &previous,
            row,
            &chain.created_at,
            &chain.retention_deadline,
        )?;
        if chain.record_hash != expected {
            return Err(integrity_query(
                "attribution_chain_hash_mismatch",
                format!("BR-251 {family} chain hash mismatch at row {id}"),
            ));
        }
        previous = chain.record_hash.clone();
    }
    Ok(())
}

fn parse_persisted_invocation(
    mode: &str,
    target_from: &str,
    target_to: &str,
    rule_version: &str,
    invoked_at: &str,
) -> Result<PreparedInvocation, AttributionReportStoreError> {
    let invocation = AttributionInvocation {
        mode: AttributionRunMode::parse(mode).map_err(|detail| {
            failed_integrity("attribution_persisted_invocation_invalid", detail)
        })?,
        target_from: NaiveDate::parse_from_str(target_from, "%Y-%m-%d").map_err(|error| {
            failed_integrity(
                "attribution_persisted_invocation_invalid",
                format!("BR-251 invalid persisted target_from: {error}"),
            )
        })?,
        target_to: NaiveDate::parse_from_str(target_to, "%Y-%m-%d").map_err(|error| {
            failed_integrity(
                "attribution_persisted_invocation_invalid",
                format!("BR-251 invalid persisted target_to: {error}"),
            )
        })?,
        rule_version: rule_version.to_string(),
        invoked_at: DateTime::parse_from_rfc3339(invoked_at).map_err(|error| {
            failed_integrity(
                "attribution_persisted_invocation_invalid",
                format!("BR-251 invalid persisted invoked_at: {error}"),
            )
        })?,
    };
    prepare_invocation(invocation)
}

fn validate_available_or_unavailable(
    status: &str,
    value: &str,
    field: &str,
) -> Result<(), AttributionReportStoreError> {
    match status {
        "available" => validate_hash(value, field),
        "unavailable" => validate_stable_code(value, field),
        _ => Err(failed_integrity(
            "attribution_persisted_evidence_invalid",
            format!("BR-251 persisted {field} has unknown status {status}"),
        )),
    }
}

fn validate_report_row(report: &PersistedReport) -> diesel::QueryResult<()> {
    if report.schema_version != SCHEMA_VERSION || report.id <= 0 || report.revision <= 0 {
        return Err(integrity_query(
            "attribution_report_schema_invalid",
            format!("BR-251 report row {} violates schema invariants", report.id),
        ));
    }
    let synthetic_invoked_at = "2000-01-01T00:00:00+08:00";
    let invocation = parse_persisted_invocation(
        &report.mode,
        &report.target_from,
        &report.target_to,
        &report.rule_version,
        synthetic_invoked_at,
    )
    .map_err(typed_store_error)?;
    for (value, field) in [
        (&report.trade_hash, "trade_hash"),
        (&report.stock_close_hash, "stock_close_hash"),
        (&report.benchmark_manifest_hash, "benchmark_manifest_hash"),
        (&report.calendar_authority_hash, "calendar_authority_hash"),
        (&report.result_payload_hash, "result_payload_hash"),
        (&report.evidence_identity, "evidence_identity"),
        (&report.report_identity, "report_identity"),
    ] {
        validate_hash(value, field).map_err(typed_store_error)?;
    }
    validate_available_or_unavailable(&report.fee_status, &report.fee_value, "fee evidence")
        .map_err(typed_store_error)?;
    validate_available_or_unavailable(
        &report.regime_status,
        &report.regime_value,
        "regime evidence",
    )
    .map_err(typed_store_error)?;
    let parsed: serde_json::Value =
        serde_json::from_str(&report.result_payload_json).map_err(|error| {
            integrity_query(
                "attribution_result_json_invalid",
                format!("BR-251 decode retained result payload: {error}"),
            )
        })?;
    let canonical = canonical_result_json(&parsed).map_err(typed_store_error)?;
    let result_hash = hash_with_domain(
        b"BR251_ATTRIBUTION_RESULT_PAYLOAD_V1",
        &[canonical.as_bytes()],
    );
    let evidence_identity = hash_with_domain(
        b"BR251_ATTRIBUTION_EVIDENCE_IDENTITY_V1",
        &[
            invocation.series_identity.as_bytes(),
            report.trade_hash.as_bytes(),
            report.fee_status.as_bytes(),
            report.fee_value.as_bytes(),
            report.stock_close_hash.as_bytes(),
            report.benchmark_manifest_hash.as_bytes(),
            report.calendar_authority_hash.as_bytes(),
            report.regime_status.as_bytes(),
            report.regime_value.as_bytes(),
        ],
    );
    let report_identity = hash_with_domain(
        b"BR251_ATTRIBUTION_REPORT_IDENTITY_V1",
        &[evidence_identity.as_bytes(), result_hash.as_bytes()],
    );
    if report.series_identity != invocation.series_identity
        || report.result_payload_json != canonical
        || report.result_payload_hash != result_hash
        || report.evidence_identity != evidence_identity
        || report.report_identity != report_identity
    {
        return Err(integrity_query(
            "attribution_report_identity_mismatch",
            format!("BR-251 report row {} identity/content mismatch", report.id),
        ));
    }
    Ok(())
}

fn report_invocation_matches_run(report: &PersistedReport, run: &PersistedRun) -> bool {
    report.mode == run.mode
        && report.target_from == run.target_from
        && report.target_to == run.target_to
        && report.rule_version == run.rule_version
        && report.series_identity == run.series_identity
}

fn validate_all_state(conn: &mut SqliteConnection) -> diesel::QueryResult<ValidatedState> {
    validate_immutable_triggers(conn)?;
    validate_chain_cardinalities(conn)?;
    let state = ValidatedState {
        runs: load_runs(conn)?,
        run_chains: load_run_chains(conn)?,
        reports: load_reports(conn)?,
        report_chains: load_report_chains(conn)?,
        failures: load_failures(conn)?,
        failure_chains: load_failure_chains(conn)?,
    };
    validate_autoincrement_highwater(
        conn,
        "attribution_run_audit",
        &state.runs.iter().map(|row| row.id).collect::<Vec<_>>(),
    )?;
    validate_autoincrement_highwater(
        conn,
        "attribution_report_revision",
        &state.reports.iter().map(|row| row.id).collect::<Vec<_>>(),
    )?;
    validate_autoincrement_highwater(
        conn,
        "attribution_failure_audit",
        &state.failures.iter().map(|row| row.id).collect::<Vec<_>>(),
    )?;
    let mut report_ids = HashSet::new();
    let mut evidence_ids = HashSet::new();
    let mut report_by_series = HashMap::<String, (i64, i32)>::new();
    for report in &state.reports {
        validate_report_row(report)?;
        if !report_ids.insert(report.report_identity.clone())
            || !evidence_ids.insert(report.evidence_identity.clone())
        {
            return Err(integrity_query(
                "attribution_report_identity_duplicate",
                "BR-251 duplicate retained report/evidence identity",
            ));
        }
        match report_by_series.get(&report.series_identity) {
            None if report.revision == 1 && report.predecessor_report_id.is_none() => {}
            Some((previous_id, previous_revision))
                if report.revision == previous_revision + 1
                    && report.predecessor_report_id == Some(*previous_id) => {}
            _ => {
                return Err(integrity_query(
                    "attribution_report_predecessor_invalid",
                    format!(
                        "BR-251 report row {} has broken revision lineage",
                        report.id
                    ),
                ));
            }
        }
        report_by_series.insert(report.series_identity.clone(), (report.id, report.revision));
        let Some(source_run) = state.runs.iter().find(|run| run.id == report.source_run_id) else {
            return Err(integrity_query(
                "attribution_report_source_run_missing",
                format!("BR-251 report row {} has no source run", report.id),
            ));
        };
        if source_run.outcome != "report_appended"
            || source_run.outcome_identity != report.report_identity
            || !report_invocation_matches_run(report, source_run)
        {
            return Err(integrity_query(
                "attribution_report_source_run_invalid",
                format!("BR-251 report row {} has an invalid source run", report.id),
            ));
        }
    }

    let reports_by_identity = state
        .reports
        .iter()
        .map(|report| (report.report_identity.as_str(), report))
        .collect::<HashMap<_, _>>();
    let report_source_runs = state
        .reports
        .iter()
        .map(|report| (report.source_run_id, report.report_identity.as_str()))
        .collect::<HashMap<_, _>>();
    let failure_source_runs = state
        .failures
        .iter()
        .map(|failure| (failure.source_run_id, failure.failure_content_hash.as_str()))
        .collect::<HashMap<_, _>>();
    for run in &state.runs {
        if run.schema_version != SCHEMA_VERSION || run.id <= 0 {
            return Err(integrity_query(
                "attribution_run_schema_invalid",
                format!("BR-251 run row {} violates schema invariants", run.id),
            ));
        }
        let invocation = parse_persisted_invocation(
            &run.mode,
            &run.target_from,
            &run.target_to,
            &run.rule_version,
            &run.invoked_at,
        )
        .map_err(typed_store_error)?;
        if run.series_identity != invocation.series_identity
            || !is_lower_hex_hash(&run.outcome_identity)
        {
            return Err(integrity_query(
                "attribution_run_identity_mismatch",
                format!("BR-251 run row {} identity mismatch", run.id),
            ));
        }
        let cross_link_valid = match run.outcome.as_str() {
            "report_appended" => {
                report_source_runs.get(&run.id).copied() == Some(run.outcome_identity.as_str())
                    && !failure_source_runs.contains_key(&run.id)
            }
            "report_reused" => {
                reports_by_identity
                    .get(run.outcome_identity.as_str())
                    .is_some_and(|report| report_invocation_matches_run(report, run))
                    && !report_source_runs.contains_key(&run.id)
                    && !failure_source_runs.contains_key(&run.id)
            }
            "failure" => {
                failure_source_runs.get(&run.id).copied() == Some(run.outcome_identity.as_str())
                    && !report_source_runs.contains_key(&run.id)
            }
            _ => false,
        };
        if !cross_link_valid {
            return Err(integrity_query(
                "attribution_run_cross_link_invalid",
                format!("BR-251 run row {} has invalid outcome cross-link", run.id),
            ));
        }
    }
    for failure in &state.failures {
        if failure.schema_version != SCHEMA_VERSION
            || failure.id <= 0
            || !is_lower_hex_hash(&failure.failure_identity)
            || !is_lower_hex_hash(&failure.failure_content_hash)
            || !is_lower_hex_hash(&failure.source_summary_hash)
            || !matches!(failure.retryable, 0 | 1)
        {
            return Err(integrity_query(
                "attribution_failure_schema_invalid",
                format!(
                    "BR-251 failure row {} violates schema invariants",
                    failure.id
                ),
            ));
        }
        validate_stable_code(&failure.stage, "failure stage").map_err(typed_store_error)?;
        validate_stable_code(&failure.code, "failure code").map_err(typed_store_error)?;
        if failure.redacted_message.trim().is_empty()
            || failure.redacted_message.len() > 4096
            || failure.redacted_message != failure.redacted_message.trim()
            || failure.redacted_message.chars().any(char::is_control)
        {
            return Err(integrity_query(
                "attribution_failure_message_invalid",
                format!("BR-251 failure row {} message is invalid", failure.id),
            ));
        }
        let run = state
            .runs
            .iter()
            .find(|run| run.id == failure.source_run_id)
            .ok_or_else(|| {
                integrity_query(
                    "attribution_failure_run_missing",
                    format!("BR-251 failure row {} has no source run", failure.id),
                )
            })?;
        let expected_content = hash_with_domain(
            b"BR251_ATTRIBUTION_FAILURE_CONTENT_V1",
            &[
                run.series_identity.as_bytes(),
                failure.stage.as_bytes(),
                failure.code.as_bytes(),
                if failure.retryable == 1 { b"1" } else { b"0" },
                failure.source_summary_hash.as_bytes(),
                failure.redacted_message.as_bytes(),
            ],
        );
        let run_id = failure.source_run_id.to_string();
        let expected_identity = hash_with_domain(
            b"BR251_ATTRIBUTION_FAILURE_IDENTITY_V1",
            &[run_id.as_bytes(), expected_content.as_bytes()],
        );
        if failure.failure_content_hash != expected_content
            || failure.failure_identity != expected_identity
        {
            return Err(integrity_query(
                "attribution_failure_identity_mismatch",
                format!("BR-251 failure row {} identity mismatch", failure.id),
            ));
        }
    }
    validate_chain(
        &state.runs,
        state.runs.iter().map(|row| row.id),
        &state.run_chains,
        RUN_CHAIN_GENESIS,
        b"BR251_ATTRIBUTION_RUN_RECORD_V1",
        "run",
    )?;
    validate_chain(
        &state.reports,
        state.reports.iter().map(|row| row.id),
        &state.report_chains,
        REPORT_CHAIN_GENESIS,
        b"BR251_ATTRIBUTION_REPORT_RECORD_V1",
        "report",
    )?;
    validate_chain(
        &state.failures,
        state.failures.iter().map(|row| row.id),
        &state.failure_chains,
        FAILURE_CHAIN_GENESIS,
        b"BR251_ATTRIBUTION_FAILURE_RECORD_V1",
        "failure",
    )?;
    Ok(state)
}

#[derive(Debug, Clone, Copy)]
enum DieselErrorContext {
    TransactionBody,
    TransactionEnvelope,
}

fn map_diesel_error(
    error: diesel::result::Error,
    operation: &'static str,
    context: DieselErrorContext,
) -> AttributionReportStoreError {
    match error {
        diesel::result::Error::InvalidCString(source) => failed_integrity(
            "attribution_query_invalid_cstring",
            format!("{operation}: {source}"),
        ),
        diesel::result::Error::QueryBuilderError(source) => {
            match source.downcast::<AttributionReportStoreError>() {
                Ok(error) => *error,
                Err(source) => failed_integrity(
                    "attribution_query_failed_integrity",
                    format!("{operation}: {source}"),
                ),
            }
        }
        diesel::result::Error::DatabaseError(kind, information) => match kind {
            diesel::result::DatabaseErrorKind::UniqueViolation
            | diesel::result::DatabaseErrorKind::ForeignKeyViolation
            | diesel::result::DatabaseErrorKind::NotNullViolation
            | diesel::result::DatabaseErrorKind::CheckViolation
            | diesel::result::DatabaseErrorKind::RestrictViolation
            | diesel::result::DatabaseErrorKind::ExclusionViolation => failed_integrity(
                "attribution_storage_constraint",
                format!("{operation}: {}", information.message()),
            ),
            diesel::result::DatabaseErrorKind::ReadOnlyTransaction => unavailable(
                "attribution_storage_read_only",
                false,
                format!("{operation}: {}", information.message()),
            ),
            diesel::result::DatabaseErrorKind::SerializationFailure => unavailable(
                "attribution_transaction_conflict",
                true,
                format!("{operation}: {}", information.message()),
            ),
            diesel::result::DatabaseErrorKind::ClosedConnection => unavailable(
                "attribution_connection_closed",
                true,
                format!("{operation}: {}", information.message()),
            ),
            diesel::result::DatabaseErrorKind::UnableToSendCommand => failed_integrity(
                "attribution_storage_protocol",
                format!("{operation}: {}", information.message()),
            ),
            diesel::result::DatabaseErrorKind::Unknown => match context {
                DieselErrorContext::TransactionBody => failed_integrity(
                    "attribution_storage_body_integrity",
                    format!("{operation}: {}", information.message()),
                ),
                DieselErrorContext::TransactionEnvelope => unavailable(
                    "attribution_storage_unavailable",
                    true,
                    format!("{operation}: {}", information.message()),
                ),
            },
            _ => failed_integrity(
                "attribution_storage_unclassified",
                format!("{operation}: {}", information.message()),
            ),
        },
        diesel::result::Error::NotFound => failed_integrity(
            "attribution_storage_not_found",
            format!("{operation}: expected persistence row not found"),
        ),
        diesel::result::Error::DeserializationError(source)
        | diesel::result::Error::SerializationError(source) => failed_integrity(
            "attribution_storage_serialization",
            format!("{operation}: {source}"),
        ),
        diesel::result::Error::RollbackErrorOnCommit {
            rollback_error,
            commit_error,
        } => {
            let rollback = map_diesel_error(*rollback_error, operation, context);
            let commit = map_diesel_error(*commit_error, operation, context);
            let detail =
                format!("{operation}: rollback failure=[{rollback}]; commit failure=[{commit}]");
            if matches!(
                rollback,
                AttributionReportStoreError::FailedIntegrity { .. }
            ) || matches!(commit, AttributionReportStoreError::FailedIntegrity { .. })
            {
                failed_integrity("attribution_rollback_commit_integrity", detail)
            } else {
                let retryable = rollback.retryable() && commit.retryable();
                unavailable("attribution_rollback_commit_unavailable", retryable, detail)
            }
        }
        diesel::result::Error::RollbackTransaction => failed_integrity(
            "attribution_transaction_rollback_requested",
            format!("{operation}: unexpected explicit rollback"),
        ),
        diesel::result::Error::AlreadyInTransaction => failed_integrity(
            "attribution_transaction_already_active",
            format!("{operation}: transaction already active"),
        ),
        diesel::result::Error::NotInTransaction => failed_integrity(
            "attribution_transaction_not_active",
            format!("{operation}: transaction not active"),
        ),
        diesel::result::Error::BrokenTransactionManager => unavailable(
            "attribution_transaction_manager_broken",
            true,
            format!("{operation}: transaction manager broken"),
        ),
        other => failed_integrity(
            "attribution_diesel_unclassified",
            format!("{operation}: {other}"),
        ),
    }
}

fn transaction_body_error(
    error: diesel::result::Error,
    operation: &'static str,
) -> diesel::result::Error {
    typed_store_error(map_diesel_error(
        error,
        operation,
        DieselErrorContext::TransactionBody,
    ))
}

fn insert_run(
    conn: &mut SqliteConnection,
    invocation: &PreparedInvocation,
    outcome: &str,
    outcome_identity: &str,
    previous_hash: &str,
) -> diesel::QueryResult<AttributionRunReceipt> {
    let affected = diesel::sql_query(
        "INSERT INTO attribution_run_audit (
            schema_version, mode, target_from, target_to, rule_version, invoked_at,
            series_identity, outcome, outcome_identity
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind::<Integer, _>(SCHEMA_VERSION)
    .bind::<Text, _>(&invocation.mode)
    .bind::<Text, _>(&invocation.target_from)
    .bind::<Text, _>(&invocation.target_to)
    .bind::<Text, _>(&invocation.rule_version)
    .bind::<Text, _>(&invocation.invoked_at)
    .bind::<Text, _>(&invocation.series_identity)
    .bind::<Text, _>(outcome)
    .bind::<Text, _>(outcome_identity)
    .execute(conn)?;
    if affected != 1 {
        return Err(integrity_query(
            "attribution_run_insert_count_invalid",
            format!("BR-251 run insert affected {affected} rows"),
        ));
    }
    let row = diesel::sql_query(
        "SELECT id, schema_version, mode, target_from, target_to, rule_version,
                invoked_at, series_identity, outcome, outcome_identity, created_at,
                retention_deadline
         FROM attribution_run_audit WHERE id = last_insert_rowid()",
    )
    .get_result::<PersistedRun>(conn)?;
    let window = new_retention_window(conn)?;
    let record_hash = chain_record_hash(
        b"BR251_ATTRIBUTION_RUN_RECORD_V1",
        previous_hash,
        &row,
        &window.created_at,
        &window.retention_deadline,
    )?;
    let chain_affected = diesel::sql_query(
        "INSERT INTO attribution_run_chain (
            run_audit_id, previous_hash, record_hash, created_at, retention_deadline
         ) VALUES (?, ?, ?, ?, ?)",
    )
    .bind::<BigInt, _>(row.id)
    .bind::<Text, _>(previous_hash)
    .bind::<Text, _>(&record_hash)
    .bind::<Text, _>(&window.created_at)
    .bind::<Text, _>(&window.retention_deadline)
    .execute(conn)?;
    if chain_affected != 1 {
        return Err(integrity_query(
            "attribution_run_chain_insert_count_invalid",
            format!("BR-251 run chain insert affected {chain_affected} rows"),
        ));
    }
    Ok(AttributionRunReceipt {
        run_audit_id: row.id,
        record_hash,
    })
}

fn report_receipt(
    run: AttributionRunReceipt,
    report: &PersistedReport,
    chain: &PersistedChain,
) -> AttributionReportReceipt {
    AttributionReportReceipt {
        run,
        report_revision_id: report.id,
        report_identity: report.report_identity.clone(),
        evidence_identity: report.evidence_identity.clone(),
        series_identity: report.series_identity.clone(),
        result_payload_hash: report.result_payload_hash.clone(),
        report_revision: report.revision,
        predecessor_report_id: report.predecessor_report_id,
        report_record_hash: chain.record_hash.clone(),
    }
}

fn insert_report(
    conn: &mut SqliteConnection,
    prepared: &PreparedReport,
    source_run_id: i64,
    revision: i32,
    predecessor_report_id: Option<i64>,
    previous_hash: &str,
) -> diesel::QueryResult<(PersistedReport, PersistedChain)> {
    let affected = diesel::sql_query(
        "INSERT INTO attribution_report_revision (
            schema_version, report_identity, series_identity, evidence_identity,
            source_run_id, mode, target_from, target_to, rule_version, trade_hash,
            fee_status, fee_value, stock_close_hash, benchmark_manifest_hash,
            calendar_authority_hash, regime_status, regime_value, result_payload_json,
            result_payload_hash, revision, predecessor_report_id
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind::<Integer, _>(SCHEMA_VERSION)
    .bind::<Text, _>(&prepared.report_identity)
    .bind::<Text, _>(&prepared.invocation.series_identity)
    .bind::<Text, _>(&prepared.evidence_identity)
    .bind::<BigInt, _>(source_run_id)
    .bind::<Text, _>(&prepared.invocation.mode)
    .bind::<Text, _>(&prepared.invocation.target_from)
    .bind::<Text, _>(&prepared.invocation.target_to)
    .bind::<Text, _>(&prepared.invocation.rule_version)
    .bind::<Text, _>(&prepared.trade_hash)
    .bind::<Text, _>(&prepared.fee_status)
    .bind::<Text, _>(&prepared.fee_value)
    .bind::<Text, _>(&prepared.stock_close_hash)
    .bind::<Text, _>(&prepared.benchmark_manifest_hash)
    .bind::<Text, _>(&prepared.calendar_authority_hash)
    .bind::<Text, _>(&prepared.regime_status)
    .bind::<Text, _>(&prepared.regime_value)
    .bind::<Text, _>(&prepared.result_payload_json)
    .bind::<Text, _>(&prepared.result_payload_hash)
    .bind::<Integer, _>(revision)
    .bind::<Nullable<BigInt>, _>(predecessor_report_id)
    .execute(conn)?;
    if affected != 1 {
        return Err(integrity_query(
            "attribution_report_insert_count_invalid",
            format!("BR-251 report insert affected {affected} rows"),
        ));
    }
    let report = diesel::sql_query(
        "SELECT id, schema_version, report_identity, series_identity, evidence_identity,
                source_run_id, mode, target_from, target_to, rule_version, trade_hash,
                fee_status, fee_value, stock_close_hash, benchmark_manifest_hash,
                calendar_authority_hash, regime_status, regime_value, result_payload_json,
                result_payload_hash, revision, predecessor_report_id, created_at,
                retention_deadline
         FROM attribution_report_revision WHERE id = last_insert_rowid()",
    )
    .get_result::<PersistedReport>(conn)?;
    let window = new_retention_window(conn)?;
    let record_hash = chain_record_hash(
        b"BR251_ATTRIBUTION_REPORT_RECORD_V1",
        previous_hash,
        &report,
        &window.created_at,
        &window.retention_deadline,
    )?;
    let chain_affected = diesel::sql_query(
        "INSERT INTO attribution_report_chain
         (report_revision_id, previous_hash, record_hash, created_at, retention_deadline)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind::<BigInt, _>(report.id)
    .bind::<Text, _>(previous_hash)
    .bind::<Text, _>(&record_hash)
    .bind::<Text, _>(&window.created_at)
    .bind::<Text, _>(&window.retention_deadline)
    .execute(conn)?;
    if chain_affected != 1 {
        return Err(integrity_query(
            "attribution_report_chain_insert_count_invalid",
            format!("BR-251 report chain insert affected {chain_affected} rows"),
        ));
    }
    Ok((
        report.clone(),
        PersistedChain {
            row_id: report.id,
            previous_hash: previous_hash.to_string(),
            record_hash,
            created_at: window.created_at,
            retention_deadline: window.retention_deadline,
        },
    ))
}

fn insert_failure(
    conn: &mut SqliteConnection,
    prepared: &PreparedFailure,
    source_run_id: i64,
    previous_hash: &str,
) -> diesel::QueryResult<(PersistedFailure, PersistedChain)> {
    let source_run_id_text = source_run_id.to_string();
    let failure_identity = hash_with_domain(
        b"BR251_ATTRIBUTION_FAILURE_IDENTITY_V1",
        &[
            source_run_id_text.as_bytes(),
            prepared.failure_content_hash.as_bytes(),
        ],
    );
    let affected = diesel::sql_query(
        "INSERT INTO attribution_failure_audit (
            schema_version, source_run_id, failure_identity, failure_content_hash,
            stage, code, retryable, source_summary_hash, redacted_message
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind::<Integer, _>(SCHEMA_VERSION)
    .bind::<BigInt, _>(source_run_id)
    .bind::<Text, _>(&failure_identity)
    .bind::<Text, _>(&prepared.failure_content_hash)
    .bind::<Text, _>(&prepared.stage)
    .bind::<Text, _>(&prepared.code)
    .bind::<Integer, _>(i32::from(prepared.retryable))
    .bind::<Text, _>(&prepared.source_summary_hash)
    .bind::<Text, _>(&prepared.redacted_message)
    .execute(conn)?;
    if affected != 1 {
        return Err(integrity_query(
            "attribution_failure_insert_count_invalid",
            format!("BR-251 failure insert affected {affected} rows"),
        ));
    }
    let failure = diesel::sql_query(
        "SELECT id, schema_version, source_run_id, failure_identity,
                failure_content_hash, stage, code, retryable, source_summary_hash,
                redacted_message, created_at, retention_deadline
         FROM attribution_failure_audit WHERE id = last_insert_rowid()",
    )
    .get_result::<PersistedFailure>(conn)?;
    let window = new_retention_window(conn)?;
    let record_hash = chain_record_hash(
        b"BR251_ATTRIBUTION_FAILURE_RECORD_V1",
        previous_hash,
        &failure,
        &window.created_at,
        &window.retention_deadline,
    )?;
    let chain_affected = diesel::sql_query(
        "INSERT INTO attribution_failure_chain
         (failure_audit_id, previous_hash, record_hash, created_at, retention_deadline)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind::<BigInt, _>(failure.id)
    .bind::<Text, _>(previous_hash)
    .bind::<Text, _>(&record_hash)
    .bind::<Text, _>(&window.created_at)
    .bind::<Text, _>(&window.retention_deadline)
    .execute(conn)?;
    if chain_affected != 1 {
        return Err(integrity_query(
            "attribution_failure_chain_insert_count_invalid",
            format!("BR-251 failure chain insert affected {chain_affected} rows"),
        ));
    }
    Ok((
        failure.clone(),
        PersistedChain {
            row_id: failure.id,
            previous_hash: previous_hash.to_string(),
            record_hash,
            created_at: window.created_at,
            retention_deadline: window.retention_deadline,
        },
    ))
}

impl<'a> AttributionReportStore<'a> {
    pub fn new(database: &'a DatabaseManager) -> Self {
        Self { database }
    }

    pub fn commit_report(
        &self,
        input: AttributionReportAppend,
    ) -> Result<AttributionReportReceipt, AttributionReportStoreError> {
        let prepared = prepare_report(input)?;
        let mut conn = self.database.get_conn().map_err(|error| {
            unavailable(
                "attribution_storage_unavailable",
                true,
                format!("BR-251 attribution store DB connection: {error}"),
            )
        })?;
        conn.immediate_transaction::<_, diesel::result::Error, _>(|conn| {
            let body = (|| -> diesel::QueryResult<AttributionReportReceipt> {
                let state = validate_all_state(conn)?;
                let run_tail = state
                    .run_chains
                    .last()
                    .map_or(RUN_CHAIN_GENESIS, |chain| chain.record_hash.as_str());
                if let Some(existing) = state
                    .reports
                    .iter()
                    .find(|report| report.evidence_identity == prepared.evidence_identity)
                {
                    if existing.result_payload_hash != prepared.result_payload_hash
                        || existing.report_identity != prepared.report_identity
                    {
                        return Err(integrity_query(
                            "attribution_nondeterministic_result",
                            "BR-251 same evidence identity produced a different result payload",
                        ));
                    }
                    let run = insert_run(
                        conn,
                        &prepared.invocation,
                        "report_reused",
                        &existing.report_identity,
                        run_tail,
                    )?;
                    let retained = validate_all_state(conn)?;
                    let report = retained
                        .reports
                        .iter()
                        .find(|report| report.id == existing.id)
                        .ok_or_else(|| {
                            integrity_query(
                                "attribution_report_reuse_missing",
                                "BR-251 reused report disappeared inside transaction",
                            )
                        })?;
                    let chain = retained
                        .report_chains
                        .iter()
                        .find(|chain| chain.row_id == report.id)
                        .ok_or_else(|| {
                            integrity_query(
                                "attribution_report_chain_missing",
                                "BR-251 reused report chain disappeared inside transaction",
                            )
                        })?;
                    return Ok(report_receipt(run, report, chain));
                }
                if state
                    .reports
                    .iter()
                    .any(|report| report.report_identity == prepared.report_identity)
                {
                    return Err(integrity_query(
                        "attribution_report_identity_collision",
                        "BR-251 report identity collides with different evidence",
                    ));
                }
                let predecessor =
                    state.reports.iter().rev().find(|report| {
                        report.series_identity == prepared.invocation.series_identity
                    });
                let revision = predecessor
                    .map(|report| {
                        report.revision.checked_add(1).ok_or_else(|| {
                            integrity_query(
                                "attribution_report_revision_overflow",
                                "BR-251 report revision exceeds i32",
                            )
                        })
                    })
                    .transpose()?
                    .unwrap_or(1);
                let predecessor_report_id = predecessor.map(|report| report.id);
                let run = insert_run(
                    conn,
                    &prepared.invocation,
                    "report_appended",
                    &prepared.report_identity,
                    run_tail,
                )?;
                let report_tail = state
                    .report_chains
                    .last()
                    .map_or(REPORT_CHAIN_GENESIS, |chain| chain.record_hash.as_str());
                let (report, _) = insert_report(
                    conn,
                    &prepared,
                    run.run_audit_id,
                    revision,
                    predecessor_report_id,
                    report_tail,
                )?;
                let retained = validate_all_state(conn)?;
                let retained_report = retained
                    .reports
                    .iter()
                    .find(|candidate| candidate.id == report.id)
                    .ok_or_else(|| {
                        integrity_query(
                            "attribution_report_insert_missing",
                            "BR-251 inserted report disappeared inside transaction",
                        )
                    })?;
                let chain = retained
                    .report_chains
                    .iter()
                    .find(|chain| chain.row_id == report.id)
                    .ok_or_else(|| {
                        integrity_query(
                            "attribution_report_chain_missing",
                            "BR-251 inserted report chain disappeared inside transaction",
                        )
                    })?;
                Ok(report_receipt(run, retained_report, chain))
            })();
            body.map_err(|error| {
                transaction_body_error(error, "BR-251 attribution report transaction body")
            })
        })
        .map_err(|error| {
            map_diesel_error(
                error,
                "BR-251 attribution report transaction",
                DieselErrorContext::TransactionEnvelope,
            )
        })
    }

    pub fn commit_failure(
        &self,
        input: AttributionFailureAppend,
    ) -> Result<AttributionFailureReceipt, AttributionReportStoreError> {
        let prepared = prepare_failure(input)?;
        let mut conn = self.database.get_conn().map_err(|error| {
            unavailable(
                "attribution_storage_unavailable",
                true,
                format!("BR-251 attribution store DB connection: {error}"),
            )
        })?;
        conn.immediate_transaction::<_, diesel::result::Error, _>(|conn| {
            let body = (|| -> diesel::QueryResult<AttributionFailureReceipt> {
                let state = validate_all_state(conn)?;
                let run_tail = state
                    .run_chains
                    .last()
                    .map_or(RUN_CHAIN_GENESIS, |chain| chain.record_hash.as_str());
                let run = insert_run(
                    conn,
                    &prepared.invocation,
                    "failure",
                    &prepared.failure_content_hash,
                    run_tail,
                )?;
                let failure_tail = state
                    .failure_chains
                    .last()
                    .map_or(FAILURE_CHAIN_GENESIS, |chain| chain.record_hash.as_str());
                let (failure, _) = insert_failure(conn, &prepared, run.run_audit_id, failure_tail)?;
                let retained = validate_all_state(conn)?;
                let retained_failure = retained
                    .failures
                    .iter()
                    .find(|candidate| candidate.id == failure.id)
                    .ok_or_else(|| {
                        integrity_query(
                            "attribution_failure_insert_missing",
                            "BR-251 inserted failure disappeared inside transaction",
                        )
                    })?;
                let chain = retained
                    .failure_chains
                    .iter()
                    .find(|chain| chain.row_id == failure.id)
                    .ok_or_else(|| {
                        integrity_query(
                            "attribution_failure_chain_missing",
                            "BR-251 inserted failure chain disappeared inside transaction",
                        )
                    })?;
                Ok(AttributionFailureReceipt {
                    run,
                    failure_audit_id: retained_failure.id,
                    failure_identity: retained_failure.failure_identity.clone(),
                    failure_record_hash: chain.record_hash.clone(),
                })
            })();
            body.map_err(|error| {
                transaction_body_error(error, "BR-251 attribution failure transaction body")
            })
        })
        .map_err(|error| {
            map_diesel_error(
                error,
                "BR-251 attribution failure transaction",
                DieselErrorContext::TransactionEnvelope,
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Barrier;
    use std::time::{SystemTime, UNIX_EPOCH};

    use chrono::{DateTime, NaiveDate};
    use diesel::connection::SimpleConnection;
    use diesel::prelude::*;
    use diesel::sql_types::{BigInt, Integer, Text};

    use super::*;
    use crate::database::DatabaseManager;

    #[derive(QueryableByName)]
    struct NameRow {
        #[diesel(sql_type = Text)]
        name: String,
    }

    #[derive(QueryableByName)]
    struct IntegerRow {
        #[diesel(sql_type = BigInt)]
        value: i64,
    }

    #[derive(QueryableByName)]
    struct FailureFactsRow {
        #[diesel(sql_type = Text)]
        stage: String,
        #[diesel(sql_type = Text)]
        code: String,
        #[diesel(sql_type = Integer)]
        retryable: i32,
        #[diesel(sql_type = Text)]
        source_summary_hash: String,
        #[diesel(sql_type = Text)]
        redacted_message: String,
    }

    #[derive(QueryableByName)]
    struct ReportFactsRow {
        #[diesel(sql_type = Text)]
        fee_status: String,
        #[diesel(sql_type = Text)]
        fee_value: String,
        #[diesel(sql_type = Text)]
        regime_status: String,
        #[diesel(sql_type = Text)]
        regime_value: String,
        #[diesel(sql_type = Text)]
        result_payload_json: String,
        #[diesel(sql_type = Text)]
        result_payload_hash: String,
    }

    fn schema_connection() -> SqliteConnection {
        let mut conn = SqliteConnection::establish(":memory:")
            .expect("TEST_CODE establish attribution schema database");
        super::create_schema(&mut conn).expect("TEST_CODE create attribution schema");
        conn
    }

    #[test]
    fn explicit_attribution_database_session_is_read_only_or_narrow_append_only() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("TEST_CODE clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "TEST_CODE_attribution_session_{}_{}.sqlite",
            std::process::id(),
            nonce
        ));
        let missing = path.with_extension("missing.sqlite");
        let mut bootstrap = SqliteConnection::establish(&path.to_string_lossy())
            .expect("TEST_CODE explicit session bootstrap");
        bootstrap
            .batch_execute(
                "CREATE TABLE legacy_guard(value TEXT NOT NULL);\
                 INSERT INTO legacy_guard(value) VALUES ('unchanged');",
            )
            .expect("TEST_CODE legacy sentinel");
        drop(bootstrap);

        let before_bytes = std::fs::read(&path).expect("TEST_CODE read database before preview");
        let before_modified = std::fs::metadata(&path)
            .and_then(|metadata| metadata.modified())
            .expect("TEST_CODE database mtime before preview");
        let read_only =
            AttributionDatabaseSession::open(&path, AttributionDatabaseAccess::ReadOnly)
                .expect("TEST_CODE open read-only attribution session");
        assert_eq!(
            read_only.database_path(),
            std::fs::canonicalize(&path).unwrap()
        );
        let mut connection = read_only
            .database()
            .get_conn()
            .expect("TEST_CODE read-only connection");
        let sentinel = diesel::sql_query("SELECT value AS name FROM legacy_guard LIMIT 1")
            .get_result::<NameRow>(&mut connection)
            .expect("TEST_CODE read legacy sentinel")
            .name;
        assert_eq!(sentinel, "unchanged");
        assert!(
            diesel::sql_query("CREATE TABLE forbidden_write(id INTEGER)")
                .execute(&mut connection)
                .is_err()
        );
        drop(connection);
        drop(read_only);
        assert_eq!(
            std::fs::read(&path).expect("TEST_CODE read database after preview"),
            before_bytes
        );
        assert_eq!(
            std::fs::metadata(&path)
                .and_then(|metadata| metadata.modified())
                .expect("TEST_CODE database mtime after preview"),
            before_modified
        );
        assert!(!PathBuf::from(format!("{}-wal", path.display())).exists());
        assert!(!PathBuf::from(format!("{}-shm", path.display())).exists());

        let missing_error =
            match AttributionDatabaseSession::open(&missing, AttributionDatabaseAccess::ReadOnly) {
                Ok(_) => panic!("TEST_CODE read-only session must not create a missing database"),
                Err(error) => error,
            };
        assert_eq!(
            missing_error.reason_code(),
            "attribution_database_unavailable"
        );
        assert!(!missing.exists());

        let append_only =
            AttributionDatabaseSession::open(&path, AttributionDatabaseAccess::AppendOnly)
                .expect("TEST_CODE open narrow append-only attribution session");
        let mut connection = append_only
            .database()
            .get_conn()
            .expect("TEST_CODE append-only connection");
        let sentinel = diesel::sql_query("SELECT value AS name FROM legacy_guard LIMIT 1")
            .get_result::<NameRow>(&mut connection)
            .expect("TEST_CODE legacy sentinel after narrow migrations")
            .name;
        assert_eq!(sentinel, "unchanged");
        for forbidden in ["paper_trades", "stock_daily", "stock_position", "trades"] {
            let count = diesel::sql_query(
                "SELECT COUNT(*) AS value FROM sqlite_master WHERE type='table' AND name=?",
            )
            .bind::<Text, _>(forbidden)
            .get_result::<IntegerRow>(&mut connection)
            .expect("TEST_CODE forbidden legacy table count")
            .value;
            assert_eq!(
                count, 0,
                "TEST_CODE must not create legacy table {forbidden}"
            );
        }
        for required in [
            "data_acquisition_audit",
            "benchmark_segment_revision",
            "benchmark_manifest",
            "attribution_run_audit",
            "attribution_report_revision",
            "attribution_failure_audit",
        ] {
            let count = diesel::sql_query(
                "SELECT COUNT(*) AS value FROM sqlite_master WHERE type='table' AND name=?",
            )
            .bind::<Text, _>(required)
            .get_result::<IntegerRow>(&mut connection)
            .expect("TEST_CODE required attribution table count")
            .value;
            assert_eq!(count, 1, "TEST_CODE missing narrow table {required}");
        }
        drop(connection);
        drop(append_only);
        for suffix in ["", "-wal", "-shm"] {
            let candidate = PathBuf::from(format!("{}{}", path.display(), suffix));
            if let Err(error) = std::fs::remove_file(&candidate) {
                assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
            }
        }
    }

    struct TestDatabase {
        path: PathBuf,
        manager: DatabaseManager,
    }

    impl TestDatabase {
        fn new() -> Self {
            Self::with_pool_size(1)
        }

        fn with_pool_size(max_size: u32) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("TEST_CODE clock")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "TEST_CODE_attribution_reports_{}_{}.sqlite",
                std::process::id(),
                nonce
            ));
            let database_url = path.to_string_lossy().into_owned();
            let mut bootstrap =
                SqliteConnection::establish(&database_url).expect("TEST_CODE SQLite bootstrap");
            diesel::sql_query("PRAGMA journal_mode = WAL")
                .execute(&mut bootstrap)
                .expect("TEST_CODE WAL");
            drop(bootstrap);
            let pool = super::super::build_sqlite_pool_with_size(database_url, max_size)
                .expect("TEST_CODE SQLite pool");
            {
                let mut conn = pool.get().expect("TEST_CODE schema connection");
                super::create_schema(&mut conn).expect("TEST_CODE attribution schema");
            }
            Self {
                path,
                manager: DatabaseManager {
                    pool,
                    selection_connection_source: None,
                    selection_schema_authority: None,
                },
            }
        }

        fn count(&self, table: &str) -> i64 {
            let mut conn = self.manager.get_conn().expect("TEST_CODE DB connection");
            diesel::sql_query(format!("SELECT COUNT(*) AS value FROM {table}"))
                .get_result::<IntegerRow>(&mut conn)
                .expect("TEST_CODE row count")
                .value
        }
    }

    impl Drop for TestDatabase {
        fn drop(&mut self) {
            for suffix in ["", "-wal", "-shm"] {
                let candidate = PathBuf::from(format!("{}{}", self.path.display(), suffix));
                if let Err(error) = std::fs::remove_file(&candidate) {
                    if error.kind() != std::io::ErrorKind::NotFound {
                        panic!("TEST_CODE remove {}: {error}", candidate.display());
                    }
                }
            }
        }
    }

    fn lower_hash(byte: char) -> String {
        byte.to_string().repeat(64)
    }

    fn scheduled_invocation(invoked_at: &str) -> AttributionInvocation {
        AttributionInvocation {
            mode: AttributionRunMode::Scheduled,
            target_from: NaiveDate::from_ymd_opt(2026, 8, 21).expect("TEST_CODE target date"),
            target_to: NaiveDate::from_ymd_opt(2026, 8, 21).expect("TEST_CODE target date"),
            rule_version: "BR-251-v1".to_string(),
            invoked_at: DateTime::parse_from_rfc3339(invoked_at)
                .expect("TEST_CODE invocation time"),
        }
    }

    fn report_append(
        invoked_at: &str,
        result_payload: serde_json::Value,
    ) -> AttributionReportAppend {
        AttributionReportAppend {
            invocation: scheduled_invocation(invoked_at),
            trade_hash: lower_hash('a'),
            fee: AttributionEvidenceHash::Available(lower_hash('b')),
            stock_close_hash: lower_hash('c'),
            benchmark_manifest_hash: lower_hash('d'),
            calendar_authority_hash: lower_hash('e'),
            regime: AttributionEvidenceHash::Unavailable("market_regime_unavailable".to_string()),
            result_payload,
        }
    }

    fn failure_append() -> AttributionFailureAppend {
        AttributionFailureAppend {
            invocation: scheduled_invocation("2026-08-21T15:30:00+08:00"),
            stage: "load_trades".to_string(),
            code: "trade_chain_invalid".to_string(),
            retryable: false,
            source_summary_hash: lower_hash('f'),
            redacted_message: "source facts failed immutable-chain validation".to_string(),
        }
    }

    fn assert_all_attribution_tables_empty(database: &TestDatabase) {
        for table in [
            "attribution_run_audit",
            "attribution_run_chain",
            "attribution_report_revision",
            "attribution_report_chain",
            "attribution_failure_audit",
            "attribution_failure_chain",
        ] {
            assert_eq!(
                database.count(table),
                0,
                "TEST_CODE {table} must stay empty"
            );
        }
    }

    fn sequence_highwater(database: &TestDatabase, table: &str) -> Option<i64> {
        let mut conn = database
            .manager
            .get_conn()
            .expect("TEST_CODE sequence connection");
        diesel::sql_query("SELECT seq FROM sqlite_sequence WHERE name = ?")
            .bind::<Text, _>(table)
            .get_result::<SequenceRow>(&mut conn)
            .optional()
            .expect("TEST_CODE query sequence highwater")
            .and_then(|row| row.seq)
    }

    fn database_with_report() -> TestDatabase {
        let database = TestDatabase::new();
        AttributionReportStore::new(&database.manager)
            .commit_report(report_append(
                "2026-08-21T15:30:00+08:00",
                serde_json::json!({"status": "ResearchOnly"}),
            ))
            .expect("TEST_CODE seed report database");
        database
    }

    fn database_with_failure() -> TestDatabase {
        let database = TestDatabase::new();
        AttributionReportStore::new(&database.manager)
            .commit_failure(failure_append())
            .expect("TEST_CODE seed failure database");
        database
    }

    fn delete_unique_report_invocation(database: &TestDatabase) {
        let mut conn = database
            .manager
            .get_conn()
            .expect("TEST_CODE delete report invocation connection");
        for trigger in [
            "trg_attribution_report_chain_no_delete",
            "trg_attribution_report_revision_no_delete",
            "trg_attribution_run_chain_no_delete",
            "trg_attribution_run_audit_no_delete",
        ] {
            diesel::sql_query(format!("DROP TRIGGER {trigger}"))
                .execute(&mut conn)
                .expect("TEST_CODE drop report invocation delete trigger");
        }
        for table in [
            "attribution_report_chain",
            "attribution_report_revision",
            "attribution_run_chain",
            "attribution_run_audit",
        ] {
            diesel::sql_query(format!("DELETE FROM {table}"))
                .execute(&mut conn)
                .expect("TEST_CODE delete unique report invocation row");
        }
        for (table, label) in [
            ("attribution_report_chain", "report chain"),
            ("attribution_report_revision", "report revision"),
            ("attribution_run_chain", "run chain"),
            ("attribution_run_audit", "run audit"),
        ] {
            immutable_triggers(&mut conn, table, label)
                .expect("TEST_CODE restore report invocation triggers");
        }
    }

    fn delete_unique_failure_invocation(database: &TestDatabase) {
        let mut conn = database
            .manager
            .get_conn()
            .expect("TEST_CODE delete failure invocation connection");
        for trigger in [
            "trg_attribution_failure_chain_no_delete",
            "trg_attribution_failure_audit_no_delete",
            "trg_attribution_run_chain_no_delete",
            "trg_attribution_run_audit_no_delete",
        ] {
            diesel::sql_query(format!("DROP TRIGGER {trigger}"))
                .execute(&mut conn)
                .expect("TEST_CODE drop failure invocation delete trigger");
        }
        for table in [
            "attribution_failure_chain",
            "attribution_failure_audit",
            "attribution_run_chain",
            "attribution_run_audit",
        ] {
            diesel::sql_query(format!("DELETE FROM {table}"))
                .execute(&mut conn)
                .expect("TEST_CODE delete unique failure invocation row");
        }
        for (table, label) in [
            ("attribution_failure_chain", "failure chain"),
            ("attribution_failure_audit", "failure audit"),
            ("attribution_run_chain", "run chain"),
            ("attribution_run_audit", "run audit"),
        ] {
            immutable_triggers(&mut conn, table, label)
                .expect("TEST_CODE restore failure invocation triggers");
        }
    }

    fn database_with_report_invocations(count: u32) -> TestDatabase {
        let database = TestDatabase::new();
        let store = AttributionReportStore::new(&database.manager);
        for offset in 0..count {
            let day = 21 + offset;
            let target =
                NaiveDate::from_ymd_opt(2026, 8, day).expect("TEST_CODE report target date");
            let mut input = report_append(
                &format!("2026-08-{day:02}T15:30:00+08:00"),
                serde_json::json!({"status": "ResearchOnly", "day": day}),
            );
            input.invocation.target_from = target;
            input.invocation.target_to = target;
            store
                .commit_report(input)
                .expect("TEST_CODE seed report invocation");
        }
        database
    }

    fn database_with_failure_invocations(count: u32) -> TestDatabase {
        let database = TestDatabase::new();
        let store = AttributionReportStore::new(&database.manager);
        for offset in 0..count {
            let day = 21 + offset;
            let target =
                NaiveDate::from_ymd_opt(2026, 8, day).expect("TEST_CODE failure target date");
            let mut input = failure_append();
            input.invocation.target_from = target;
            input.invocation.target_to = target;
            input.invocation.invoked_at =
                DateTime::parse_from_rfc3339(&format!("2026-08-{day:02}T15:30:00+08:00"))
                    .expect("TEST_CODE failure invocation time");
            store
                .commit_failure(input)
                .expect("TEST_CODE seed failure invocation");
        }
        database
    }

    fn rehash_chain_rows<T: Serialize>(
        conn: &mut SqliteConnection,
        rows: &[T],
        ids: impl Iterator<Item = i64>,
        chains: &[PersistedChain],
        genesis: &str,
        domain: &[u8],
        table_key: (&str, &str),
    ) {
        let (table, id_column) = table_key;
        let mut previous = genesis.to_string();
        for ((row, id), chain) in rows.iter().zip(ids).zip(chains) {
            let record_hash = chain_record_hash(
                domain,
                &previous,
                row,
                &chain.created_at,
                &chain.retention_deadline,
            )
            .expect("TEST_CODE rehash retained chain after paired deletion");
            diesel::sql_query(format!(
                "UPDATE {table} SET previous_hash = ?, record_hash = ? WHERE {id_column} = ?"
            ))
            .bind::<Text, _>(&previous)
            .bind::<Text, _>(&record_hash)
            .bind::<BigInt, _>(id)
            .execute(&mut *conn)
            .expect("TEST_CODE persist retained chain after paired deletion");
            previous = record_hash;
        }
    }

    fn delete_report_invocation_at(database: &TestDatabase, id: i64) {
        let mut conn = database
            .manager
            .get_conn()
            .expect("TEST_CODE report paired deletion connection");
        for trigger in [
            "trg_attribution_report_chain_no_delete",
            "trg_attribution_report_revision_no_delete",
            "trg_attribution_run_chain_no_delete",
            "trg_attribution_run_audit_no_delete",
            "trg_attribution_report_chain_no_update",
            "trg_attribution_run_chain_no_update",
        ] {
            diesel::sql_query(format!("DROP TRIGGER {trigger}"))
                .execute(&mut conn)
                .expect("TEST_CODE drop report paired deletion trigger");
        }
        diesel::sql_query("DELETE FROM attribution_report_chain WHERE report_revision_id = ?")
            .bind::<BigInt, _>(id)
            .execute(&mut conn)
            .expect("TEST_CODE delete report chain pair");
        diesel::sql_query("DELETE FROM attribution_report_revision WHERE id = ?")
            .bind::<BigInt, _>(id)
            .execute(&mut conn)
            .expect("TEST_CODE delete report row pair");
        diesel::sql_query("DELETE FROM attribution_run_chain WHERE run_audit_id = ?")
            .bind::<BigInt, _>(id)
            .execute(&mut conn)
            .expect("TEST_CODE delete run chain pair");
        diesel::sql_query("DELETE FROM attribution_run_audit WHERE id = ?")
            .bind::<BigInt, _>(id)
            .execute(&mut conn)
            .expect("TEST_CODE delete run row pair");

        let runs = load_runs(&mut conn).expect("TEST_CODE load retained runs");
        let run_chains = load_run_chains(&mut conn).expect("TEST_CODE load retained run chains");
        rehash_chain_rows(
            &mut conn,
            &runs,
            runs.iter().map(|row| row.id),
            &run_chains,
            RUN_CHAIN_GENESIS,
            b"BR251_ATTRIBUTION_RUN_RECORD_V1",
            ("attribution_run_chain", "run_audit_id"),
        );
        let reports = load_reports(&mut conn).expect("TEST_CODE load retained reports");
        let report_chains =
            load_report_chains(&mut conn).expect("TEST_CODE load retained report chains");
        rehash_chain_rows(
            &mut conn,
            &reports,
            reports.iter().map(|row| row.id),
            &report_chains,
            REPORT_CHAIN_GENESIS,
            b"BR251_ATTRIBUTION_REPORT_RECORD_V1",
            ("attribution_report_chain", "report_revision_id"),
        );
        for (table, label) in [
            ("attribution_report_chain", "report chain"),
            ("attribution_report_revision", "report revision"),
            ("attribution_run_chain", "run chain"),
            ("attribution_run_audit", "run audit"),
        ] {
            immutable_triggers(&mut conn, table, label)
                .expect("TEST_CODE restore report paired deletion triggers");
        }
    }

    fn delete_failure_invocation_at(database: &TestDatabase, id: i64) {
        let mut conn = database
            .manager
            .get_conn()
            .expect("TEST_CODE failure paired deletion connection");
        for trigger in [
            "trg_attribution_failure_chain_no_delete",
            "trg_attribution_failure_audit_no_delete",
            "trg_attribution_run_chain_no_delete",
            "trg_attribution_run_audit_no_delete",
            "trg_attribution_failure_chain_no_update",
            "trg_attribution_run_chain_no_update",
        ] {
            diesel::sql_query(format!("DROP TRIGGER {trigger}"))
                .execute(&mut conn)
                .expect("TEST_CODE drop failure paired deletion trigger");
        }
        diesel::sql_query("DELETE FROM attribution_failure_chain WHERE failure_audit_id = ?")
            .bind::<BigInt, _>(id)
            .execute(&mut conn)
            .expect("TEST_CODE delete failure chain pair");
        diesel::sql_query("DELETE FROM attribution_failure_audit WHERE id = ?")
            .bind::<BigInt, _>(id)
            .execute(&mut conn)
            .expect("TEST_CODE delete failure row pair");
        diesel::sql_query("DELETE FROM attribution_run_chain WHERE run_audit_id = ?")
            .bind::<BigInt, _>(id)
            .execute(&mut conn)
            .expect("TEST_CODE delete failure run chain pair");
        diesel::sql_query("DELETE FROM attribution_run_audit WHERE id = ?")
            .bind::<BigInt, _>(id)
            .execute(&mut conn)
            .expect("TEST_CODE delete failure run row pair");

        let runs = load_runs(&mut conn).expect("TEST_CODE load retained failure runs");
        let run_chains =
            load_run_chains(&mut conn).expect("TEST_CODE load retained failure run chains");
        rehash_chain_rows(
            &mut conn,
            &runs,
            runs.iter().map(|row| row.id),
            &run_chains,
            RUN_CHAIN_GENESIS,
            b"BR251_ATTRIBUTION_RUN_RECORD_V1",
            ("attribution_run_chain", "run_audit_id"),
        );
        let failures = load_failures(&mut conn).expect("TEST_CODE load retained failures");
        let failure_chains =
            load_failure_chains(&mut conn).expect("TEST_CODE load retained failure chains");
        rehash_chain_rows(
            &mut conn,
            &failures,
            failures.iter().map(|row| row.id),
            &failure_chains,
            FAILURE_CHAIN_GENESIS,
            b"BR251_ATTRIBUTION_FAILURE_RECORD_V1",
            ("attribution_failure_chain", "failure_audit_id"),
        );
        for (table, label) in [
            ("attribution_failure_chain", "failure chain"),
            ("attribution_failure_audit", "failure audit"),
            ("attribution_run_chain", "run chain"),
            ("attribution_run_audit", "run audit"),
        ] {
            immutable_triggers(&mut conn, table, label)
                .expect("TEST_CODE restore failure paired deletion triggers");
        }
    }

    fn startup_fails(database: &TestDatabase) -> bool {
        let mut conn = database
            .manager
            .get_conn()
            .expect("TEST_CODE paired deletion startup connection");
        super::create_schema(&mut conn).is_err()
    }

    fn report_preappend_fails(database: &TestDatabase) -> bool {
        AttributionReportStore::new(&database.manager)
            .commit_report(report_append(
                "2026-08-24T15:30:00+08:00",
                serde_json::json!({"status": "ResearchOnly"}),
            ))
            .is_err()
    }

    fn failure_preappend_fails(database: &TestDatabase) -> bool {
        AttributionReportStore::new(&database.manager)
            .commit_failure(failure_append())
            .is_err()
    }

    fn replace_trigger(database: &TestDatabase, name: &str, replacement: Option<&str>) {
        let mut conn = database
            .manager
            .get_conn()
            .expect("TEST_CODE trigger tamper connection");
        diesel::sql_query(format!("DROP TRIGGER {name}"))
            .execute(&mut conn)
            .expect("TEST_CODE drop canonical trigger");
        if let Some(definition) = replacement {
            diesel::sql_query(definition)
                .execute(&mut conn)
                .expect("TEST_CODE install altered trigger");
        }
    }

    fn restore_table_triggers(conn: &mut SqliteConnection, table: &str) {
        let label = IMMUTABLE_TABLES
            .iter()
            .find_map(|(candidate, label)| (*candidate == table).then_some(*label))
            .expect("TEST_CODE immutable table label");
        immutable_triggers(conn, table, label).expect("TEST_CODE restore canonical triggers");
    }

    fn trigger_preappend_fails(database: &TestDatabase) -> bool {
        matches!(
            AttributionReportStore::new(&database.manager).commit_report(report_append(
                "2026-08-23T15:30:00+08:00",
                serde_json::json!({"status": "ResearchOnly"}),
            )),
            Err(AttributionReportStoreError::FailedIntegrity { .. })
        )
    }

    fn assert_startup_rejects(database: &TestDatabase) {
        let mut conn = database
            .manager
            .get_conn()
            .expect("TEST_CODE DB connection");
        assert!(
            super::create_schema(&mut conn).is_err(),
            "TEST_CODE startup must reject retained tamper"
        );
    }

    fn database_with_cross_series_report_run_swap() -> TestDatabase {
        let database = TestDatabase::new();
        let store = AttributionReportStore::new(&database.manager);
        store
            .commit_report(report_append(
                "2026-08-21T15:30:00+08:00",
                serde_json::json!({"status": "ResearchOnly", "target": "first"}),
            ))
            .expect("TEST_CODE seed first report series");
        let mut second = report_append(
            "2026-08-22T15:30:00+08:00",
            serde_json::json!({"status": "ResearchOnly", "target": "second"}),
        );
        second.invocation.target_from =
            NaiveDate::from_ymd_opt(2026, 8, 22).expect("TEST_CODE second target date");
        second.invocation.target_to = second.invocation.target_from;
        store
            .commit_report(second)
            .expect("TEST_CODE seed second report series");

        let mut conn = database
            .manager
            .get_conn()
            .expect("TEST_CODE coordinated swap connection");
        for trigger in [
            "trg_attribution_run_audit_no_update",
            "trg_attribution_run_chain_no_update",
            "trg_attribution_report_revision_no_update",
            "trg_attribution_report_chain_no_update",
        ] {
            diesel::sql_query(format!("DROP TRIGGER {trigger}"))
                .execute(&mut conn)
                .expect("TEST_CODE drop coordinated swap trigger");
        }
        diesel::sql_query(
            "UPDATE attribution_report_revision
             SET source_run_id = CASE id WHEN 1 THEN 2 WHEN 2 THEN 1 END
             WHERE id IN (1, 2)",
        )
        .execute(&mut conn)
        .expect("TEST_CODE swap report source runs");
        diesel::sql_query(
            "UPDATE attribution_run_audit
             SET outcome_identity = CASE id
                 WHEN 1 THEN (SELECT report_identity FROM attribution_report_revision WHERE id = 2)
                 WHEN 2 THEN (SELECT report_identity FROM attribution_report_revision WHERE id = 1)
             END
             WHERE id IN (1, 2)",
        )
        .execute(&mut conn)
        .expect("TEST_CODE swap run outcomes");

        let runs = load_runs(&mut conn).expect("TEST_CODE load swapped runs");
        let run_chains = load_run_chains(&mut conn).expect("TEST_CODE load run chains");
        let mut previous = RUN_CHAIN_GENESIS.to_string();
        for (run, chain) in runs.iter().zip(&run_chains) {
            let record_hash = chain_record_hash(
                b"BR251_ATTRIBUTION_RUN_RECORD_V1",
                &previous,
                run,
                &chain.created_at,
                &chain.retention_deadline,
            )
            .expect("TEST_CODE rehash swapped run chain");
            diesel::sql_query(
                "UPDATE attribution_run_chain SET previous_hash = ?, record_hash = ?
                 WHERE run_audit_id = ?",
            )
            .bind::<Text, _>(&previous)
            .bind::<Text, _>(&record_hash)
            .bind::<BigInt, _>(run.id)
            .execute(&mut conn)
            .expect("TEST_CODE persist rehashed run chain");
            previous = record_hash;
        }

        let reports = load_reports(&mut conn).expect("TEST_CODE load swapped reports");
        let report_chains = load_report_chains(&mut conn).expect("TEST_CODE load report chains");
        previous = REPORT_CHAIN_GENESIS.to_string();
        for (report, chain) in reports.iter().zip(&report_chains) {
            let record_hash = chain_record_hash(
                b"BR251_ATTRIBUTION_REPORT_RECORD_V1",
                &previous,
                report,
                &chain.created_at,
                &chain.retention_deadline,
            )
            .expect("TEST_CODE rehash swapped report chain");
            diesel::sql_query(
                "UPDATE attribution_report_chain SET previous_hash = ?, record_hash = ?
                 WHERE report_revision_id = ?",
            )
            .bind::<Text, _>(&previous)
            .bind::<Text, _>(&record_hash)
            .bind::<BigInt, _>(report.id)
            .execute(&mut conn)
            .expect("TEST_CODE persist rehashed report chain");
            previous = record_hash;
        }
        for table in [
            "attribution_run_audit",
            "attribution_run_chain",
            "attribution_report_revision",
            "attribution_report_chain",
        ] {
            restore_table_triggers(&mut conn, table);
        }
        drop(conn);
        database
    }

    fn reject_inserts(database: &TestDatabase, table: &str) {
        let mut conn = database
            .manager
            .get_conn()
            .expect("TEST_CODE DB connection");
        diesel::sql_query(format!(
            "CREATE TRIGGER TEST_CODE_reject_{table}_insert
             BEFORE INSERT ON {table}
             BEGIN SELECT RAISE(ABORT, 'TEST_CODE chain insert failure'); END"
        ))
        .execute(&mut conn)
        .expect("TEST_CODE install insert rejection trigger");
    }

    fn database_error(
        kind: diesel::result::DatabaseErrorKind,
        label: &str,
    ) -> diesel::result::Error {
        diesel::result::Error::DatabaseError(kind, Box::new(label.to_string()))
    }

    fn seed_one_row_in_each_table(conn: &mut SqliteConnection) {
        let hash_a = "a".repeat(64);
        let hash_b = "b".repeat(64);
        let hash_c = "c".repeat(64);
        let hash_d = "d".repeat(64);
        diesel::sql_query(
            "INSERT INTO attribution_run_audit (
                schema_version, mode, target_from, target_to, rule_version, invoked_at,
                series_identity, outcome, outcome_identity
             ) VALUES (1, 'scheduled', '2026-08-21', '2026-08-21', 'BR-251-v1',
                       '2026-08-21T15:30:00+08:00', ?, 'report_appended', ?)",
        )
        .bind::<Text, _>(&hash_a)
        .bind::<Text, _>(&hash_b)
        .execute(&mut *conn)
        .expect("TEST_CODE seed run audit");
        diesel::sql_query(
            "INSERT INTO attribution_run_chain (run_audit_id, previous_hash, record_hash)
             VALUES (1, 'BR251_TEST_RUN_GENESIS', ?)",
        )
        .bind::<Text, _>(&hash_c)
        .execute(&mut *conn)
        .expect("TEST_CODE seed run chain");
        diesel::sql_query(
            "INSERT INTO attribution_report_revision (
                schema_version, report_identity, series_identity, evidence_identity,
                source_run_id, mode, target_from, target_to, rule_version, trade_hash,
                fee_status, fee_value, stock_close_hash, benchmark_manifest_hash,
                calendar_authority_hash, regime_status, regime_value,
                result_payload_json, result_payload_hash, revision, predecessor_report_id
             ) VALUES (
                1, ?, ?, ?, 1, 'scheduled', '2026-08-21', '2026-08-21', 'BR-251-v1', ?,
                'available', ?, ?, ?, ?, 'unavailable', 'regime_unavailable',
                '{\"status\":\"ResearchOnly\"}', ?, 1, NULL
             )",
        )
        .bind::<Text, _>(&hash_a)
        .bind::<Text, _>(&hash_b)
        .bind::<Text, _>(&hash_c)
        .bind::<Text, _>(&hash_d)
        .bind::<Text, _>(&hash_a)
        .bind::<Text, _>(&hash_b)
        .bind::<Text, _>(&hash_c)
        .bind::<Text, _>(&hash_d)
        .bind::<Text, _>(&hash_a)
        .execute(&mut *conn)
        .expect("TEST_CODE seed report revision");
        diesel::sql_query(
            "INSERT INTO attribution_report_chain
             (report_revision_id, previous_hash, record_hash)
             VALUES (1, 'BR251_TEST_REPORT_GENESIS', ?)",
        )
        .bind::<Text, _>(&hash_b)
        .execute(&mut *conn)
        .expect("TEST_CODE seed report chain");
        diesel::sql_query(
            "INSERT INTO attribution_failure_audit (
                schema_version, source_run_id, failure_identity, failure_content_hash,
                stage, code, retryable, source_summary_hash, redacted_message
             ) VALUES (1, 2, ?, ?, 'load_trades', 'trade_chain_invalid', 0, ?, 'redacted')",
        )
        .bind::<Text, _>(&hash_c)
        .bind::<Text, _>(&hash_d)
        .bind::<Text, _>(&hash_a)
        .execute(&mut *conn)
        .expect("TEST_CODE seed failure audit");
        diesel::sql_query(
            "INSERT INTO attribution_failure_chain
             (failure_audit_id, previous_hash, record_hash)
             VALUES (1, 'BR251_TEST_FAILURE_GENESIS', ?)",
        )
        .bind::<Text, _>(&hash_c)
        .execute(&mut *conn)
        .expect("TEST_CODE seed failure chain");
    }

    #[test]
    fn migrations_create_all_six_immutable_attribution_tables() {
        let mut conn = SqliteConnection::establish(":memory:")
            .expect("TEST_CODE establish migration database");
        DatabaseManager::run_migrations_for_test(&mut conn)
            .expect("TEST_CODE run complete migrations");

        let names = diesel::sql_query(
            "SELECT name FROM sqlite_master
             WHERE type = 'table' AND name LIKE 'attribution_%'
             ORDER BY name ASC",
        )
        .load::<NameRow>(&mut conn)
        .expect("TEST_CODE list attribution tables")
        .into_iter()
        .map(|row| row.name)
        .collect::<Vec<_>>();

        assert_eq!(
            names,
            vec![
                "attribution_failure_audit",
                "attribution_failure_chain",
                "attribution_report_chain",
                "attribution_report_revision",
                "attribution_run_audit",
                "attribution_run_chain",
            ]
        );

        let trigger_names = diesel::sql_query(
            "SELECT name FROM sqlite_master
             WHERE type = 'trigger' AND name LIKE 'trg_attribution_%'
             ORDER BY name ASC",
        )
        .load::<NameRow>(&mut conn)
        .expect("TEST_CODE list immutable attribution triggers")
        .into_iter()
        .map(|row| row.name)
        .collect::<Vec<_>>();
        assert_eq!(
            trigger_names,
            vec![
                "trg_attribution_failure_audit_no_delete",
                "trg_attribution_failure_audit_no_update",
                "trg_attribution_failure_chain_no_delete",
                "trg_attribution_failure_chain_no_update",
                "trg_attribution_report_chain_no_delete",
                "trg_attribution_report_chain_no_update",
                "trg_attribution_report_revision_no_delete",
                "trg_attribution_report_revision_no_update",
                "trg_attribution_run_audit_no_delete",
                "trg_attribution_run_audit_no_update",
                "trg_attribution_run_chain_no_delete",
                "trg_attribution_run_chain_no_update",
            ]
        );

        super::create_schema(&mut conn).expect("TEST_CODE empty chains validate at startup");
    }

    #[test]
    fn startup_rejects_partial_six_table_schema_without_repairing_it() {
        let mut conn = SqliteConnection::establish(":memory:")
            .expect("TEST_CODE establish partial schema database");
        diesel::sql_query(
            "CREATE TABLE attribution_run_audit (id INTEGER PRIMARY KEY AUTOINCREMENT)",
        )
        .execute(&mut conn)
        .expect("TEST_CODE create one partial attribution table");

        assert!(
            super::create_schema(&mut conn).is_err(),
            "TEST_CODE partial attribution schema must fail closed before repair"
        );
        let table_count = diesel::sql_query(
            "SELECT COUNT(*) AS value FROM sqlite_master
             WHERE type = 'table' AND name LIKE 'attribution_%'",
        )
        .get_result::<IntegerRow>(&mut conn)
        .expect("TEST_CODE count partial attribution tables")
        .value;
        assert_eq!(table_count, 1);
    }

    #[test]
    fn all_six_tables_reject_update_delete_and_retain_at_least_five_years() {
        let mut conn = schema_connection();
        seed_one_row_in_each_table(&mut conn);

        for table in [
            "attribution_run_audit",
            "attribution_run_chain",
            "attribution_report_revision",
            "attribution_report_chain",
            "attribution_failure_audit",
            "attribution_failure_chain",
        ] {
            assert!(
                diesel::sql_query(format!("UPDATE {table} SET created_at = created_at"))
                    .execute(&mut conn)
                    .is_err(),
                "TEST_CODE {table} UPDATE must fail"
            );
            assert!(
                diesel::sql_query(format!("DELETE FROM {table}"))
                    .execute(&mut conn)
                    .is_err(),
                "TEST_CODE {table} DELETE must fail"
            );
            let retained = diesel::sql_query(format!(
                "SELECT COUNT(*) AS value FROM {table}
                 WHERE julianday(retention_deadline) >= julianday(created_at, '+5 years')"
            ))
            .get_result::<IntegerRow>(&mut conn)
            .expect("TEST_CODE query retention")
            .value;
            assert!(retained > 0, "TEST_CODE {table} retention is short");
        }
    }

    #[test]
    fn startup_rejects_shortened_retention() {
        let mut conn = schema_connection();
        seed_one_row_in_each_table(&mut conn);
        diesel::sql_query("DROP TRIGGER trg_attribution_report_revision_no_update")
            .execute(&mut conn)
            .expect("TEST_CODE drop retention protection");
        diesel::sql_query(
            "UPDATE attribution_report_revision SET retention_deadline = created_at WHERE id = 1",
        )
        .execute(&mut conn)
        .expect("TEST_CODE shorten report retention");
        restore_table_triggers(&mut conn, "attribution_report_revision");

        assert!(
            super::create_schema(&mut conn).is_err(),
            "TEST_CODE startup must reject shortened retention"
        );
    }

    #[test]
    fn first_report_commit_appends_one_run_and_one_report() {
        let database = TestDatabase::new();
        let receipt = AttributionReportStore::new(&database.manager)
            .commit_report(report_append(
                "2026-08-21T15:30:00+08:00",
                serde_json::json!({"status": "ResearchOnly", "closed": 12}),
            ))
            .expect("TEST_CODE first report commit");

        assert_eq!(receipt.report_revision, 1);
        assert_eq!(receipt.predecessor_report_id, None);
        assert_eq!(database.count("attribution_run_audit"), 1);
        assert_eq!(database.count("attribution_run_chain"), 1);
        assert_eq!(database.count("attribution_report_revision"), 1);
        assert_eq!(database.count("attribution_report_chain"), 1);
        assert_eq!(database.count("attribution_failure_audit"), 0);
        assert_eq!(database.count("attribution_failure_chain"), 0);
    }

    #[test]
    fn exact_report_replay_appends_new_run_and_reuses_old_report() {
        let database = TestDatabase::new();
        let store = AttributionReportStore::new(&database.manager);
        let first = store
            .commit_report(report_append(
                "2026-08-21T15:30:00+08:00",
                serde_json::json!({
                    "nested": {"z": 1, "a": 2},
                    "closed": 12,
                    "status": "ResearchOnly"
                }),
            ))
            .expect("TEST_CODE first report");
        let replay = store
            .commit_report(report_append(
                "2026-08-23T12:00:00+08:00",
                serde_json::json!({
                    "status": "ResearchOnly",
                    "closed": 12,
                    "nested": {"a": 2, "z": 1}
                }),
            ))
            .expect("TEST_CODE exact report replay");

        assert_ne!(first.run, replay.run);
        assert_eq!(first.report_revision_id, replay.report_revision_id);
        assert_eq!(first.report_identity, replay.report_identity);
        assert_eq!(first.evidence_identity, replay.evidence_identity);
        assert_eq!(first.result_payload_hash, replay.result_payload_hash);
        assert_eq!(first.report_record_hash, replay.report_record_hash);
        assert_eq!(database.count("attribution_run_audit"), 2);
        assert_eq!(database.count("attribution_run_chain"), 2);
        assert_eq!(database.count("attribution_report_revision"), 1);
        assert_eq!(database.count("attribution_report_chain"), 1);
    }

    #[test]
    fn changed_source_evidence_appends_successor_revision() {
        let database = TestDatabase::new();
        let store = AttributionReportStore::new(&database.manager);
        let first = store
            .commit_report(report_append(
                "2026-08-21T15:30:00+08:00",
                serde_json::json!({"status": "ResearchOnly"}),
            ))
            .expect("TEST_CODE first source revision");
        let mut revised = report_append(
            "2026-08-22T12:00:00+08:00",
            serde_json::json!({"status": "ResearchOnly"}),
        );
        revised.stock_close_hash = lower_hash('f');
        let second = store
            .commit_report(revised)
            .expect("TEST_CODE revised source evidence");

        assert_eq!(second.report_revision, 2);
        assert_eq!(second.predecessor_report_id, Some(first.report_revision_id));
        assert_eq!(second.series_identity, first.series_identity);
        assert_ne!(second.evidence_identity, first.evidence_identity);
        assert_ne!(second.report_identity, first.report_identity);
        assert_eq!(database.count("attribution_run_audit"), 2);
        assert_eq!(database.count("attribution_report_revision"), 2);
        assert_eq!(database.count("attribution_report_chain"), 2);
    }

    #[test]
    fn same_evidence_with_different_result_is_integrity_and_writes_nothing() {
        let database = TestDatabase::new();
        let store = AttributionReportStore::new(&database.manager);
        store
            .commit_report(report_append(
                "2026-08-21T15:30:00+08:00",
                serde_json::json!({"status": "ResearchOnly", "closed": 12}),
            ))
            .expect("TEST_CODE first deterministic result");
        let error = store
            .commit_report(report_append(
                "2026-08-22T12:00:00+08:00",
                serde_json::json!({"status": "ResearchOnly", "closed": 13}),
            ))
            .expect_err("TEST_CODE nondeterministic result must fail");

        assert!(matches!(
            error,
            AttributionReportStoreError::FailedIntegrity {
                reason_code: "attribution_nondeterministic_result",
                ..
            }
        ));
        assert_eq!(database.count("attribution_run_audit"), 1);
        assert_eq!(database.count("attribution_run_chain"), 1);
        assert_eq!(database.count("attribution_report_revision"), 1);
        assert_eq!(database.count("attribution_report_chain"), 1);
    }

    #[test]
    fn failure_commit_appends_only_run_and_failure_with_exact_redacted_facts() {
        let database = TestDatabase::new();
        let receipt = AttributionReportStore::new(&database.manager)
            .commit_failure(failure_append())
            .expect("TEST_CODE failure commit");

        assert_eq!(receipt.run.run_audit_id, 1);
        assert_eq!(receipt.failure_audit_id, 1);
        assert_eq!(database.count("attribution_run_audit"), 1);
        assert_eq!(database.count("attribution_run_chain"), 1);
        assert_eq!(database.count("attribution_failure_audit"), 1);
        assert_eq!(database.count("attribution_failure_chain"), 1);
        assert_eq!(database.count("attribution_report_revision"), 0);
        assert_eq!(database.count("attribution_report_chain"), 0);

        let mut conn = database
            .manager
            .get_conn()
            .expect("TEST_CODE DB connection");
        let retained = diesel::sql_query(
            "SELECT stage, code, retryable, source_summary_hash, redacted_message
             FROM attribution_failure_audit WHERE id = 1",
        )
        .get_result::<FailureFactsRow>(&mut conn)
        .expect("TEST_CODE retained failure facts");
        assert_eq!(retained.stage, "load_trades");
        assert_eq!(retained.code, "trade_chain_invalid");
        assert_eq!(retained.retryable, 0);
        assert_eq!(retained.source_summary_hash, lower_hash('f'));
        assert_eq!(
            retained.redacted_message,
            "source facts failed immutable-chain validation"
        );
    }

    #[test]
    fn repeated_same_failure_appends_distinct_run_and_failure_audits() {
        let database = TestDatabase::new();
        let store = AttributionReportStore::new(&database.manager);
        let first = store
            .commit_failure(failure_append())
            .expect("TEST_CODE first failure invocation");
        let second = store
            .commit_failure(failure_append())
            .expect("TEST_CODE repeated failure invocation");

        assert_ne!(first.run, second.run);
        assert_ne!(first.failure_audit_id, second.failure_audit_id);
        assert_ne!(first.failure_identity, second.failure_identity);
        assert_eq!(database.count("attribution_run_audit"), 2);
        assert_eq!(database.count("attribution_run_chain"), 2);
        assert_eq!(database.count("attribution_failure_audit"), 2);
        assert_eq!(database.count("attribution_failure_chain"), 2);
        assert_eq!(database.count("attribution_report_revision"), 0);
    }

    #[test]
    fn noncanonical_failure_input_is_rejected_before_any_write() {
        let database = TestDatabase::new();
        let mut input = failure_append();
        input.redacted_message = "redacted\nmessage".to_string();
        let error = AttributionReportStore::new(&database.manager)
            .commit_failure(input)
            .expect_err("TEST_CODE control characters must be rejected");

        assert!(matches!(
            error,
            AttributionReportStoreError::FailedIntegrity {
                reason_code: "attribution_failure_message_invalid",
                ..
            }
        ));
        assert_all_attribution_tables_empty(&database);
    }

    #[test]
    fn invalid_invocation_evidence_and_payload_matrix_writes_nothing() {
        let database = TestDatabase::new();
        let store = AttributionReportStore::new(&database.manager);

        let mut invalid_failures = Vec::new();
        let mut wrong_offset = failure_append();
        wrong_offset.invocation.invoked_at =
            DateTime::parse_from_rfc3339("2026-08-21T07:30:00+00:00")
                .expect("TEST_CODE wrong offset");
        invalid_failures.push(wrong_offset);
        let mut scheduled_range = failure_append();
        scheduled_range.invocation.target_to += chrono::Duration::days(1);
        invalid_failures.push(scheduled_range);
        let mut reversed_range = failure_append();
        reversed_range.invocation.mode = AttributionRunMode::Range;
        reversed_range.invocation.target_from =
            NaiveDate::from_ymd_opt(2026, 8, 22).expect("TEST_CODE range from");
        invalid_failures.push(reversed_range);
        let mut invalid_quarter = failure_append();
        invalid_quarter.invocation.mode = AttributionRunMode::Quarter;
        invalid_quarter.invocation.target_from =
            NaiveDate::from_ymd_opt(2026, 4, 1).expect("TEST_CODE quarter from");
        invalid_quarter.invocation.target_to =
            NaiveDate::from_ymd_opt(2026, 6, 29).expect("TEST_CODE quarter to");
        invalid_failures.push(invalid_quarter);
        let mut bad_hash = failure_append();
        bad_hash.source_summary_hash = "F".repeat(64);
        invalid_failures.push(bad_hash);
        let mut blank_stage = failure_append();
        blank_stage.stage = String::new();
        invalid_failures.push(blank_stage);
        let mut oversized_stage = failure_append();
        oversized_stage.stage = "a".repeat(129);
        invalid_failures.push(oversized_stage);
        let mut blank_code = failure_append();
        blank_code.code = String::new();
        invalid_failures.push(blank_code);
        let mut oversized_code = failure_append();
        oversized_code.code = "b".repeat(129);
        invalid_failures.push(oversized_code);
        let mut blank_message = failure_append();
        blank_message.redacted_message = " ".to_string();
        invalid_failures.push(blank_message);
        let mut oversized_message = failure_append();
        oversized_message.redacted_message = "x".repeat(4097);
        invalid_failures.push(oversized_message);

        for input in invalid_failures {
            assert!(
                matches!(
                    store.commit_failure(input),
                    Err(AttributionReportStoreError::FailedIntegrity { .. })
                ),
                "TEST_CODE invalid failure input must fail integrity"
            );
            assert_all_attribution_tables_empty(&database);
        }

        let mut bad_required_hash = report_append(
            "2026-08-21T15:30:00+08:00",
            serde_json::json!({"status": "ResearchOnly"}),
        );
        bad_required_hash.trade_hash = "A".repeat(64);
        let mut bad_unavailable_code = report_append(
            "2026-08-21T15:30:00+08:00",
            serde_json::json!({"status": "ResearchOnly"}),
        );
        bad_unavailable_code.fee =
            AttributionEvidenceHash::Unavailable("FeeUnavailable".to_string());
        let non_object_payload = report_append(
            "2026-08-21T15:30:00+08:00",
            serde_json::json!(["ResearchOnly"]),
        );
        for input in [bad_required_hash, bad_unavailable_code, non_object_payload] {
            assert!(matches!(
                store.commit_report(input),
                Err(AttributionReportStoreError::FailedIntegrity { .. })
            ));
            assert_all_attribution_tables_empty(&database);
        }
    }

    #[test]
    fn every_chain_insert_failure_rolls_back_the_whole_invocation() {
        let run_chain_database = TestDatabase::new();
        reject_inserts(&run_chain_database, "attribution_run_chain");
        let run_error = AttributionReportStore::new(&run_chain_database.manager)
            .commit_report(report_append(
                "2026-08-21T15:30:00+08:00",
                serde_json::json!({"status": "ResearchOnly"}),
            ))
            .expect_err("TEST_CODE run chain insert failure");
        assert!(matches!(
            run_error,
            AttributionReportStoreError::FailedIntegrity { .. }
        ));
        assert_all_attribution_tables_empty(&run_chain_database);

        let report_chain_database = TestDatabase::new();
        reject_inserts(&report_chain_database, "attribution_report_chain");
        let report_error = AttributionReportStore::new(&report_chain_database.manager)
            .commit_report(report_append(
                "2026-08-21T15:30:00+08:00",
                serde_json::json!({"status": "ResearchOnly"}),
            ))
            .expect_err("TEST_CODE report chain insert failure");
        assert!(matches!(
            report_error,
            AttributionReportStoreError::FailedIntegrity { .. }
        ));
        assert_all_attribution_tables_empty(&report_chain_database);

        let failure_chain_database = TestDatabase::new();
        reject_inserts(&failure_chain_database, "attribution_failure_chain");
        let failure_error = AttributionReportStore::new(&failure_chain_database.manager)
            .commit_failure(failure_append())
            .expect_err("TEST_CODE failure chain insert failure");
        assert!(matches!(
            failure_error,
            AttributionReportStoreError::FailedIntegrity { .. }
        ));
        assert_all_attribution_tables_empty(&failure_chain_database);
    }

    #[test]
    fn failed_insert_and_transaction_rollback_do_not_create_sequence_gaps() {
        let failed_statement = TestDatabase::new();
        {
            let mut conn = failed_statement
                .manager
                .get_conn()
                .expect("TEST_CODE failed statement connection");
            assert!(
                diesel::sql_query(
                    "INSERT INTO attribution_run_audit (
                        schema_version, mode, target_from, target_to, rule_version,
                        invoked_at, series_identity, outcome, outcome_identity
                     ) VALUES (
                        1, 'invalid', '2026-08-21', '2026-08-21', 'BR-251-v1',
                        '2026-08-21T15:30:00+08:00', ?, 'failure', ?
                     )",
                )
                .bind::<Text, _>(lower_hash('a'))
                .bind::<Text, _>(lower_hash('b'))
                .execute(&mut conn)
                .is_err(),
                "TEST_CODE rejected statement must fail"
            );
        }
        assert_eq!(
            sequence_highwater(&failed_statement, "attribution_run_audit"),
            None
        );

        let report_rollback = TestDatabase::new();
        reject_inserts(&report_rollback, "attribution_report_chain");
        assert!(
            AttributionReportStore::new(&report_rollback.manager)
                .commit_report(report_append(
                    "2026-08-21T15:30:00+08:00",
                    serde_json::json!({"status": "ResearchOnly"}),
                ))
                .is_err(),
            "TEST_CODE report transaction must roll back"
        );
        assert_eq!(
            sequence_highwater(&report_rollback, "attribution_run_audit"),
            None
        );
        assert_eq!(
            sequence_highwater(&report_rollback, "attribution_report_revision"),
            None
        );
        {
            let mut conn = report_rollback
                .manager
                .get_conn()
                .expect("TEST_CODE report retry connection");
            diesel::sql_query("DROP TRIGGER TEST_CODE_reject_attribution_report_chain_insert")
                .execute(&mut conn)
                .expect("TEST_CODE remove report-chain rejection");
        }
        let report_receipt = AttributionReportStore::new(&report_rollback.manager)
            .commit_report(report_append(
                "2026-08-21T15:30:00+08:00",
                serde_json::json!({"status": "ResearchOnly"}),
            ))
            .expect("TEST_CODE report retry without identity gap");
        assert_eq!(report_receipt.run.run_audit_id, 1);
        assert_eq!(report_receipt.report_revision_id, 1);

        let failure_rollback = TestDatabase::new();
        reject_inserts(&failure_rollback, "attribution_failure_chain");
        assert!(
            AttributionReportStore::new(&failure_rollback.manager)
                .commit_failure(failure_append())
                .is_err(),
            "TEST_CODE failure transaction must roll back"
        );
        assert_eq!(
            sequence_highwater(&failure_rollback, "attribution_run_audit"),
            None
        );
        assert_eq!(
            sequence_highwater(&failure_rollback, "attribution_failure_audit"),
            None
        );
        {
            let mut conn = failure_rollback
                .manager
                .get_conn()
                .expect("TEST_CODE failure retry connection");
            diesel::sql_query("DROP TRIGGER TEST_CODE_reject_attribution_failure_chain_insert")
                .execute(&mut conn)
                .expect("TEST_CODE remove failure-chain rejection");
        }
        let failure_receipt = AttributionReportStore::new(&failure_rollback.manager)
            .commit_failure(failure_append())
            .expect("TEST_CODE failure retry without identity gap");
        assert_eq!(failure_receipt.run.run_audit_id, 1);
        assert_eq!(failure_receipt.failure_audit_id, 1);
    }

    #[test]
    fn startup_rejects_chain_metadata_tamper_even_when_retention_is_extended() {
        let database = TestDatabase::new();
        AttributionReportStore::new(&database.manager)
            .commit_report(report_append(
                "2026-08-21T15:30:00+08:00",
                serde_json::json!({"status": "ResearchOnly"}),
            ))
            .expect("TEST_CODE seed immutable report");
        let mut conn = database
            .manager
            .get_conn()
            .expect("TEST_CODE DB connection");
        diesel::sql_query("DROP TRIGGER trg_attribution_run_chain_no_update")
            .execute(&mut conn)
            .expect("TEST_CODE drop run-chain update trigger");
        diesel::sql_query(
            "UPDATE attribution_run_chain
             SET retention_deadline = datetime(retention_deadline, '+1 year')
             WHERE run_audit_id = 1",
        )
        .execute(&mut conn)
        .expect("TEST_CODE extend retained chain deadline");
        restore_table_triggers(&mut conn, "attribution_run_chain");

        assert!(
            super::create_schema(&mut conn).is_err(),
            "TEST_CODE chain metadata tamper must fail startup"
        );
    }

    #[test]
    fn startup_rejects_row_and_chain_tamper_for_every_family() {
        let run_row = database_with_report();
        {
            let mut conn = run_row.manager.get_conn().expect("TEST_CODE run row conn");
            diesel::sql_query("DROP TRIGGER trg_attribution_run_audit_no_update")
                .execute(&mut conn)
                .expect("TEST_CODE drop run row trigger");
            diesel::sql_query(
                "UPDATE attribution_run_audit
                 SET invoked_at = '2026-08-21T15:31:00+08:00' WHERE id = 1",
            )
            .execute(&mut conn)
            .expect("TEST_CODE tamper run row");
            restore_table_triggers(&mut conn, "attribution_run_audit");
        }
        assert_startup_rejects(&run_row);

        let report_row = database_with_report();
        {
            let mut conn = report_row
                .manager
                .get_conn()
                .expect("TEST_CODE report row conn");
            diesel::sql_query("DROP TRIGGER trg_attribution_report_revision_no_update")
                .execute(&mut conn)
                .expect("TEST_CODE drop report row trigger");
            diesel::sql_query(
                "UPDATE attribution_report_revision
                 SET result_payload_json = '{\"status\":\"Tampered\"}' WHERE id = 1",
            )
            .execute(&mut conn)
            .expect("TEST_CODE tamper report row");
            restore_table_triggers(&mut conn, "attribution_report_revision");
        }
        assert_startup_rejects(&report_row);

        let failure_row = database_with_failure();
        {
            let mut conn = failure_row
                .manager
                .get_conn()
                .expect("TEST_CODE failure row conn");
            diesel::sql_query("DROP TRIGGER trg_attribution_failure_audit_no_update")
                .execute(&mut conn)
                .expect("TEST_CODE drop failure row trigger");
            diesel::sql_query(
                "UPDATE attribution_failure_audit
                 SET redacted_message = 'tampered redacted message' WHERE id = 1",
            )
            .execute(&mut conn)
            .expect("TEST_CODE tamper failure row");
            restore_table_triggers(&mut conn, "attribution_failure_audit");
        }
        assert_startup_rejects(&failure_row);

        for (database, table, trigger, id_column) in [
            (
                database_with_report(),
                "attribution_run_chain",
                "trg_attribution_run_chain_no_update",
                "run_audit_id",
            ),
            (
                database_with_report(),
                "attribution_report_chain",
                "trg_attribution_report_chain_no_update",
                "report_revision_id",
            ),
            (
                database_with_failure(),
                "attribution_failure_chain",
                "trg_attribution_failure_chain_no_update",
                "failure_audit_id",
            ),
        ] {
            {
                let mut conn = database.manager.get_conn().expect("TEST_CODE chain conn");
                diesel::sql_query(format!("DROP TRIGGER {trigger}"))
                    .execute(&mut conn)
                    .expect("TEST_CODE drop chain trigger");
                diesel::sql_query(format!(
                    "UPDATE {table} SET previous_hash = 'TEST_CODE_tampered_previous'
                     WHERE {id_column} = 1"
                ))
                .execute(&mut conn)
                .expect("TEST_CODE tamper chain previous");
                restore_table_triggers(&mut conn, table);
            }
            assert_startup_rejects(&database);
        }
    }

    #[test]
    fn startup_rejects_missing_and_extra_chain_rows_for_every_family() {
        for (database, table, trigger, id_column) in [
            (
                database_with_report(),
                "attribution_run_chain",
                "trg_attribution_run_chain_no_delete",
                "run_audit_id",
            ),
            (
                database_with_report(),
                "attribution_report_chain",
                "trg_attribution_report_chain_no_delete",
                "report_revision_id",
            ),
            (
                database_with_failure(),
                "attribution_failure_chain",
                "trg_attribution_failure_chain_no_delete",
                "failure_audit_id",
            ),
        ] {
            {
                let mut conn = database.manager.get_conn().expect("TEST_CODE missing conn");
                diesel::sql_query(format!("DROP TRIGGER {trigger}"))
                    .execute(&mut conn)
                    .expect("TEST_CODE drop chain delete trigger");
                diesel::sql_query(format!("DELETE FROM {table} WHERE {id_column} = 1"))
                    .execute(&mut conn)
                    .expect("TEST_CODE delete chain row");
                restore_table_triggers(&mut conn, table);
            }
            assert_startup_rejects(&database);
        }

        for (database, table, id_column) in [
            (
                database_with_report(),
                "attribution_run_chain",
                "run_audit_id",
            ),
            (
                database_with_report(),
                "attribution_report_chain",
                "report_revision_id",
            ),
            (
                database_with_failure(),
                "attribution_failure_chain",
                "failure_audit_id",
            ),
        ] {
            {
                let mut conn = database.manager.get_conn().expect("TEST_CODE extra conn");
                diesel::sql_query("PRAGMA foreign_keys = OFF")
                    .execute(&mut conn)
                    .expect("TEST_CODE disable FK for adversarial tamper");
                diesel::sql_query(format!(
                    "INSERT INTO {table} (
                        {id_column}, previous_hash, record_hash, created_at, retention_deadline
                     ) VALUES (
                        999, 'TEST_CODE_extra_previous', ?,
                        '2026-08-21T00:00:00.000Z', '2031-08-21T00:00:00.000Z'
                     )"
                ))
                .bind::<Text, _>(lower_hash('9'))
                .execute(&mut conn)
                .expect("TEST_CODE insert extra chain row");
            }
            assert_startup_rejects(&database);
        }
    }

    #[test]
    fn startup_and_preappend_reject_paired_deletion_of_unique_formal_invocation() {
        let report_startup = database_with_report();
        delete_unique_report_invocation(&report_startup);
        let report_startup_result = {
            let mut conn = report_startup
                .manager
                .get_conn()
                .expect("TEST_CODE report startup connection");
            super::create_schema(&mut conn)
        };

        let report_preappend = database_with_report();
        delete_unique_report_invocation(&report_preappend);
        let report_preappend_result = AttributionReportStore::new(&report_preappend.manager)
            .commit_report(report_append(
                "2026-08-22T15:30:00+08:00",
                serde_json::json!({"status": "ResearchOnly"}),
            ));

        let failure_startup = database_with_failure();
        delete_unique_failure_invocation(&failure_startup);
        let failure_startup_result = {
            let mut conn = failure_startup
                .manager
                .get_conn()
                .expect("TEST_CODE failure startup connection");
            super::create_schema(&mut conn)
        };

        let failure_preappend = database_with_failure();
        delete_unique_failure_invocation(&failure_preappend);
        let failure_preappend_result = AttributionReportStore::new(&failure_preappend.manager)
            .commit_failure(failure_append());

        assert!(
            report_startup_result.is_err()
                && report_preappend_result.is_err()
                && failure_startup_result.is_err()
                && failure_preappend_result.is_err(),
            "TEST_CODE paired deletion of unique report/failure invocation must fail startup and preappend"
        );
    }

    #[test]
    fn startup_and_preappend_reject_paired_tail_deletion_for_all_data_families() {
        let report_startup = database_with_report_invocations(3);
        delete_report_invocation_at(&report_startup, 3);
        let report_preappend = database_with_report_invocations(3);
        delete_report_invocation_at(&report_preappend, 3);
        let failure_startup = database_with_failure_invocations(3);
        delete_failure_invocation_at(&failure_startup, 3);
        let failure_preappend = database_with_failure_invocations(3);
        delete_failure_invocation_at(&failure_preappend, 3);

        assert!(
            startup_fails(&report_startup)
                && report_preappend_fails(&report_preappend)
                && startup_fails(&failure_startup)
                && failure_preappend_fails(&failure_preappend),
            "TEST_CODE paired tail deletion must fail startup and preappend for run/report/failure"
        );
    }

    #[test]
    fn startup_and_preappend_reject_rehashed_paired_middle_deletion_for_all_data_families() {
        let report_startup = database_with_report_invocations(3);
        delete_report_invocation_at(&report_startup, 2);
        let report_preappend = database_with_report_invocations(3);
        delete_report_invocation_at(&report_preappend, 2);
        let failure_startup = database_with_failure_invocations(3);
        delete_failure_invocation_at(&failure_startup, 2);
        let failure_preappend = database_with_failure_invocations(3);
        delete_failure_invocation_at(&failure_preappend, 2);

        assert!(
            startup_fails(&report_startup)
                && report_preappend_fails(&report_preappend)
                && startup_fails(&failure_startup)
                && failure_preappend_fails(&failure_preappend),
            "TEST_CODE rehashed paired middle deletion must fail startup and preappend for run/report/failure"
        );
    }

    #[test]
    fn startup_and_preappend_reject_missing_or_altered_immutable_trigger_definitions() {
        let trigger_names = [
            "trg_attribution_run_audit_no_update",
            "trg_attribution_run_audit_no_delete",
            "trg_attribution_run_chain_no_update",
            "trg_attribution_run_chain_no_delete",
            "trg_attribution_report_revision_no_update",
            "trg_attribution_report_revision_no_delete",
            "trg_attribution_report_chain_no_update",
            "trg_attribution_report_chain_no_delete",
            "trg_attribution_failure_audit_no_update",
            "trg_attribution_failure_audit_no_delete",
            "trg_attribution_failure_chain_no_update",
            "trg_attribution_failure_chain_no_delete",
        ];
        let mut accepted = Vec::new();
        for name in trigger_names {
            let startup = database_with_report();
            replace_trigger(&startup, name, None);
            if !startup_fails(&startup) {
                accepted.push(format!("missing {name} at startup"));
            }
            let preappend = database_with_report();
            replace_trigger(&preappend, name, None);
            if !trigger_preappend_fails(&preappend) {
                accepted.push(format!("missing {name} at preappend"));
            }
        }

        for (case, definition) in [
            (
                "same-name no-op",
                "CREATE TRIGGER trg_attribution_run_audit_no_update
                 BEFORE UPDATE ON attribution_run_audit BEGIN SELECT 1; END",
            ),
            (
                "wrong target",
                "CREATE TRIGGER trg_attribution_run_audit_no_update
                 BEFORE UPDATE ON attribution_run_chain
                 BEGIN SELECT RAISE(ABORT, 'BR-251 attribution run audit is immutable'); END",
            ),
            (
                "wrong timing",
                "CREATE TRIGGER trg_attribution_run_audit_no_update
                 AFTER UPDATE ON attribution_run_audit
                 BEGIN SELECT RAISE(ABORT, 'BR-251 attribution run audit is immutable'); END",
            ),
            (
                "wrong event",
                "CREATE TRIGGER trg_attribution_run_audit_no_update
                 BEFORE DELETE ON attribution_run_audit
                 BEGIN SELECT RAISE(ABORT, 'BR-251 attribution run audit is immutable'); END",
            ),
            (
                "wrong action",
                "CREATE TRIGGER trg_attribution_run_audit_no_update
                 BEFORE UPDATE ON attribution_run_audit
                 BEGIN SELECT RAISE(FAIL, 'BR-251 attribution run audit is immutable'); END",
            ),
        ] {
            let startup = database_with_report();
            replace_trigger(
                &startup,
                "trg_attribution_run_audit_no_update",
                Some(definition),
            );
            if !startup_fails(&startup) {
                accepted.push(format!("{case} at startup"));
            }
            let preappend = database_with_report();
            replace_trigger(
                &preappend,
                "trg_attribution_run_audit_no_update",
                Some(definition),
            );
            if !trigger_preappend_fails(&preappend) {
                accepted.push(format!("{case} at preappend"));
            }
        }

        assert!(
            accepted.is_empty(),
            "TEST_CODE trigger tamper was accepted: {}",
            accepted.join(", ")
        );
    }

    #[test]
    fn startup_rejects_rehashed_predecessor_and_outcome_crosslink_tamper() {
        let run_crosslink = database_with_report();
        {
            let mut conn = run_crosslink
                .manager
                .get_conn()
                .expect("TEST_CODE run crosslink conn");
            diesel::sql_query("DROP TRIGGER trg_attribution_run_audit_no_update")
                .execute(&mut conn)
                .expect("TEST_CODE drop run update trigger");
            diesel::sql_query("DROP TRIGGER trg_attribution_run_chain_no_update")
                .execute(&mut conn)
                .expect("TEST_CODE drop run-chain update trigger");
            diesel::sql_query("UPDATE attribution_run_audit SET outcome_identity = ? WHERE id = 1")
                .bind::<Text, _>(lower_hash('f'))
                .execute(&mut conn)
                .expect("TEST_CODE tamper run outcome identity");
            let run = load_runs(&mut conn)
                .expect("TEST_CODE load tampered run")
                .remove(0);
            let chain = load_run_chains(&mut conn)
                .expect("TEST_CODE load run chain")
                .remove(0);
            let rehashed = chain_record_hash(
                b"BR251_ATTRIBUTION_RUN_RECORD_V1",
                RUN_CHAIN_GENESIS,
                &run,
                &chain.created_at,
                &chain.retention_deadline,
            )
            .expect("TEST_CODE rehash run chain");
            diesel::sql_query(
                "UPDATE attribution_run_chain SET record_hash = ? WHERE run_audit_id = 1",
            )
            .bind::<Text, _>(&rehashed)
            .execute(&mut conn)
            .expect("TEST_CODE retain valid run chain after crosslink tamper");
            restore_table_triggers(&mut conn, "attribution_run_audit");
            restore_table_triggers(&mut conn, "attribution_run_chain");
        }
        assert_startup_rejects(&run_crosslink);

        let report_predecessor = TestDatabase::new();
        let store = AttributionReportStore::new(&report_predecessor.manager);
        store
            .commit_report(report_append(
                "2026-08-21T15:30:00+08:00",
                serde_json::json!({"status": "ResearchOnly"}),
            ))
            .expect("TEST_CODE first report revision");
        let mut revised = report_append(
            "2026-08-22T15:30:00+08:00",
            serde_json::json!({"status": "ResearchOnly"}),
        );
        revised.trade_hash = lower_hash('f');
        store
            .commit_report(revised)
            .expect("TEST_CODE second report revision");
        {
            let mut conn = report_predecessor
                .manager
                .get_conn()
                .expect("TEST_CODE predecessor conn");
            diesel::sql_query("DROP TRIGGER trg_attribution_report_revision_no_update")
                .execute(&mut conn)
                .expect("TEST_CODE drop report update trigger");
            diesel::sql_query("DROP TRIGGER trg_attribution_report_chain_no_update")
                .execute(&mut conn)
                .expect("TEST_CODE drop report-chain update trigger");
            diesel::sql_query(
                "UPDATE attribution_report_revision
                 SET predecessor_report_id = NULL WHERE id = 2",
            )
            .execute(&mut conn)
            .expect("TEST_CODE tamper predecessor");
            let reports = load_reports(&mut conn).expect("TEST_CODE load reports");
            let chains = load_report_chains(&mut conn).expect("TEST_CODE load report chains");
            let rehashed = chain_record_hash(
                b"BR251_ATTRIBUTION_REPORT_RECORD_V1",
                &chains[0].record_hash,
                &reports[1],
                &chains[1].created_at,
                &chains[1].retention_deadline,
            )
            .expect("TEST_CODE rehash predecessor report");
            diesel::sql_query(
                "UPDATE attribution_report_chain
                 SET record_hash = ? WHERE report_revision_id = 2",
            )
            .bind::<Text, _>(&rehashed)
            .execute(&mut conn)
            .expect("TEST_CODE retain valid report chain after predecessor tamper");
            restore_table_triggers(&mut conn, "attribution_report_revision");
            restore_table_triggers(&mut conn, "attribution_report_chain");
        }
        assert_startup_rejects(&report_predecessor);

        let failure_crosslink = TestDatabase::new();
        let store = AttributionReportStore::new(&failure_crosslink.manager);
        store
            .commit_report(report_append(
                "2026-08-21T15:30:00+08:00",
                serde_json::json!({"status": "ResearchOnly"}),
            ))
            .expect("TEST_CODE crosslink report run");
        store
            .commit_failure(failure_append())
            .expect("TEST_CODE crosslink failure run");
        {
            let mut conn = failure_crosslink
                .manager
                .get_conn()
                .expect("TEST_CODE failure crosslink conn");
            diesel::sql_query("DROP TRIGGER trg_attribution_failure_audit_no_update")
                .execute(&mut conn)
                .expect("TEST_CODE drop failure update trigger");
            diesel::sql_query("DROP TRIGGER trg_attribution_failure_chain_no_update")
                .execute(&mut conn)
                .expect("TEST_CODE drop failure-chain update trigger");
            let failures = load_failures(&mut conn).expect("TEST_CODE load failure");
            let run_id = 1_i64;
            let run_id_text = run_id.to_string();
            let identity = hash_with_domain(
                b"BR251_ATTRIBUTION_FAILURE_IDENTITY_V1",
                &[
                    run_id_text.as_bytes(),
                    failures[0].failure_content_hash.as_bytes(),
                ],
            );
            diesel::sql_query(
                "UPDATE attribution_failure_audit
                 SET source_run_id = ?, failure_identity = ? WHERE id = 1",
            )
            .bind::<BigInt, _>(run_id)
            .bind::<Text, _>(&identity)
            .execute(&mut conn)
            .expect("TEST_CODE tamper failure run crosslink");
            let failure = load_failures(&mut conn)
                .expect("TEST_CODE reload failure")
                .remove(0);
            let chain = load_failure_chains(&mut conn)
                .expect("TEST_CODE load failure chain")
                .remove(0);
            let rehashed = chain_record_hash(
                b"BR251_ATTRIBUTION_FAILURE_RECORD_V1",
                FAILURE_CHAIN_GENESIS,
                &failure,
                &chain.created_at,
                &chain.retention_deadline,
            )
            .expect("TEST_CODE rehash failure chain");
            diesel::sql_query(
                "UPDATE attribution_failure_chain
                 SET record_hash = ? WHERE failure_audit_id = 1",
            )
            .bind::<Text, _>(&rehashed)
            .execute(&mut conn)
            .expect("TEST_CODE retain valid failure chain after crosslink tamper");
            restore_table_triggers(&mut conn, "attribution_failure_audit");
            restore_table_triggers(&mut conn, "attribution_failure_chain");
        }
        assert_startup_rejects(&failure_crosslink);
    }

    #[test]
    fn startup_and_preappend_reject_cross_series_report_run_swap_after_full_rehash() {
        let startup_database = database_with_cross_series_report_run_swap();
        let startup_result = {
            let mut conn = startup_database
                .manager
                .get_conn()
                .expect("TEST_CODE startup swap connection");
            super::create_schema(&mut conn)
        };

        let preappend_database = database_with_cross_series_report_run_swap();
        let preappend_result = AttributionReportStore::new(&preappend_database.manager)
            .commit_report(report_append(
                "2026-08-23T15:30:00+08:00",
                serde_json::json!({"status": "ResearchOnly", "target": "first"}),
            ));

        assert!(
            startup_result.is_err() && preappend_result.is_err(),
            "TEST_CODE coordinated cross-series swap must fail startup and preappend"
        );
    }

    #[test]
    fn external_exclusive_lock_is_retryable_unavailable_and_writes_nothing() {
        let database = TestDatabase::new();
        let mut locker = SqliteConnection::establish(&database.path.to_string_lossy())
            .expect("TEST_CODE external SQLite connection");
        locker
            .batch_execute("PRAGMA busy_timeout = 0; BEGIN EXCLUSIVE")
            .expect("TEST_CODE acquire external EXCLUSIVE lock");

        let error = AttributionReportStore::new(&database.manager)
            .commit_report(report_append(
                "2026-08-21T15:30:00+08:00",
                serde_json::json!({"status": "ResearchOnly"}),
            ))
            .expect_err("TEST_CODE external lock must fail closed");
        assert!(matches!(
            error,
            AttributionReportStoreError::Unavailable {
                retryable: true,
                ..
            }
        ));
        locker
            .batch_execute("ROLLBACK")
            .expect("TEST_CODE release external lock");
        assert_all_attribution_tables_empty(&database);
    }

    #[test]
    fn body_trigger_is_integrity_not_retryable_unavailable() {
        let database = TestDatabase::new();
        reject_inserts(&database, "attribution_run_audit");
        let error = AttributionReportStore::new(&database.manager)
            .commit_failure(failure_append())
            .expect_err("TEST_CODE body trigger must fail integrity");
        assert!(matches!(
            error,
            AttributionReportStoreError::FailedIntegrity {
                reason_code: "attribution_storage_body_integrity",
                ..
            }
        ));
        assert_all_attribution_tables_empty(&database);
    }

    #[test]
    fn typed_read_only_and_rollback_commit_taxonomy_never_parses_messages() {
        let read_only = map_diesel_error(
            database_error(
                diesel::result::DatabaseErrorKind::ReadOnlyTransaction,
                "TEST_CODE arbitrary localized read-only detail",
            ),
            "TEST_CODE read-only operation",
            DieselErrorContext::TransactionBody,
        );
        assert!(matches!(
            read_only,
            AttributionReportStoreError::Unavailable {
                reason_code: "attribution_storage_read_only",
                retryable: false,
                ..
            }
        ));

        let rollback_commit = map_diesel_error(
            diesel::result::Error::RollbackErrorOnCommit {
                rollback_error: Box::new(database_error(
                    diesel::result::DatabaseErrorKind::SerializationFailure,
                    "TEST_CODE rollback conflict",
                )),
                commit_error: Box::new(database_error(
                    diesel::result::DatabaseErrorKind::ReadOnlyTransaction,
                    "TEST_CODE commit read only",
                )),
            },
            "TEST_CODE rollback-on-commit",
            DieselErrorContext::TransactionEnvelope,
        );
        assert!(matches!(
            rollback_commit,
            AttributionReportStoreError::Unavailable {
                reason_code: "attribution_rollback_commit_unavailable",
                retryable: false,
                ..
            }
        ));
    }

    #[test]
    fn unavailable_evidence_stays_typed_and_result_json_is_store_canonicalized() {
        let database = TestDatabase::new();
        let mut input = report_append(
            "2026-08-21T15:30:00+08:00",
            serde_json::json!({"status": "ResearchOnly", "closed": 12}),
        );
        input.fee = AttributionEvidenceHash::Unavailable("fee_evidence_unavailable".to_string());
        input.regime = AttributionEvidenceHash::Available(lower_hash('f'));
        let receipt = AttributionReportStore::new(&database.manager)
            .commit_report(input)
            .expect("TEST_CODE typed unavailable report");

        let mut conn = database
            .manager
            .get_conn()
            .expect("TEST_CODE DB connection");
        let retained = diesel::sql_query(
            "SELECT fee_status, fee_value, regime_status, regime_value,
                    result_payload_json, result_payload_hash
             FROM attribution_report_revision WHERE id = 1",
        )
        .get_result::<ReportFactsRow>(&mut conn)
        .expect("TEST_CODE retained report facts");
        assert_eq!(retained.fee_status, "unavailable");
        assert_eq!(retained.fee_value, "fee_evidence_unavailable");
        assert!(!is_lower_hex_hash(&retained.fee_value));
        assert_eq!(retained.regime_status, "available");
        assert_eq!(retained.regime_value, lower_hash('f'));
        assert_eq!(
            retained.result_payload_json,
            "{\"closed\":12,\"status\":\"ResearchOnly\"}"
        );
        assert_eq!(retained.result_payload_hash, receipt.result_payload_hash);
        assert!(is_lower_hex_hash(&retained.result_payload_hash));
    }

    #[test]
    fn concurrent_exact_reports_serialize_to_two_runs_and_one_report() {
        let database = TestDatabase::with_pool_size(2);
        let barrier = Barrier::new(2);
        let manager = &database.manager;
        let (first, second) = std::thread::scope(|scope| {
            let first = scope.spawn(|| {
                barrier.wait();
                AttributionReportStore::new(manager).commit_report(report_append(
                    "2026-08-21T15:30:00+08:00",
                    serde_json::json!({"status": "ResearchOnly"}),
                ))
            });
            let second = scope.spawn(|| {
                barrier.wait();
                AttributionReportStore::new(manager).commit_report(report_append(
                    "2026-08-23T12:00:00+08:00",
                    serde_json::json!({"status": "ResearchOnly"}),
                ))
            });
            (
                first.join().expect("TEST_CODE first writer thread"),
                second.join().expect("TEST_CODE second writer thread"),
            )
        });
        let first = first.expect("TEST_CODE first concurrent report");
        let second = second.expect("TEST_CODE second concurrent report");

        assert_ne!(first.run, second.run);
        assert_eq!(first.report_revision_id, second.report_revision_id);
        assert_eq!(first.report_identity, second.report_identity);
        assert_eq!(database.count("attribution_run_audit"), 2);
        assert_eq!(database.count("attribution_run_chain"), 2);
        assert_eq!(database.count("attribution_report_revision"), 1);
        assert_eq!(database.count("attribution_report_chain"), 1);
    }
}
