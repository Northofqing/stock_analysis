//! BR-251 immutable natural-quarter benchmark segment and manifest store.

use std::collections::{HashMap, HashSet};
use std::io::Cursor;

use chrono::{DateTime, Datelike, Duration, FixedOffset, Months, NaiveDate, Utc};
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Binary, Integer, Nullable, Text};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::data_acquisition_audit::DataAcquisitionAuditReceipt;
use super::DatabaseManager;
use crate::data_gateway::{
    BatchEvidence, BenchmarkBar, BenchmarkBarTime, BenchmarkError, BenchmarkGranularity,
    BenchmarkRange, BenchmarkRequest,
};
use crate::market_domain::ProviderId;

const SEGMENT_CHAIN_GENESIS: &str = "BR251_BENCHMARK_SEGMENT_CHAIN_GENESIS_V1";
const MANIFEST_CHAIN_GENESIS: &str = "BR251_BENCHMARK_MANIFEST_CHAIN_GENESIS_V1";
const PAYLOAD_SCHEMA: &str = "BR251_BENCHMARK_SEGMENT_PAYLOAD_V1";
const MANIFEST_ACQUISITION_SCHEMA: &str = "BR251_BENCHMARK_MANIFEST_ACQUISITION_V1";
const COMPOSED_MANIFEST_ACQUISITION_SCHEMA: &str =
    "BR251_BENCHMARK_COMPOSED_MANIFEST_ACQUISITION_V2";
const CODEC: &str = "zstd";
const CODEC_VERSION: i32 = 1;
const PAYLOAD_VERSION: i32 = 1;
const IMMUTABLE_TABLES: [(&str, &str, &str); 5] = [
    (
        "benchmark_segment_revision",
        "BR-251 benchmark segment revision is immutable",
        "BR-251 benchmark segment retention is at least five years",
    ),
    (
        "benchmark_segment_chain",
        "BR-251 benchmark segment chain is immutable",
        "BR-251 benchmark segment chain retention is at least five years",
    ),
    (
        "benchmark_manifest",
        "BR-251 benchmark manifest is immutable",
        "BR-251 benchmark manifest retention is at least five years",
    ),
    (
        "benchmark_manifest_acquisition",
        "BR-251 benchmark manifest acquisition is immutable",
        "BR-251 benchmark manifest acquisition retention is at least five years",
    ),
    (
        "benchmark_manifest_chain",
        "BR-251 benchmark manifest chain is immutable",
        "BR-251 benchmark manifest chain retention is at least five years",
    ),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentState {
    Provisional,
    Sealed,
}

impl SegmentState {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Provisional => "Provisional",
            Self::Sealed => "Sealed",
        }
    }
}

#[derive(Debug, Clone)]
pub struct BenchmarkSegmentAppend {
    pub request: BenchmarkRequest,
    pub quarter_start: NaiveDate,
    pub state: SegmentState,
    pub bars: Vec<BenchmarkBar>,
    pub evidence: BatchEvidence,
    pub acquisition_receipt: DataAcquisitionAuditReceipt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BenchmarkManifestRef {
    pub manifest_hash: String,
    pub instrument: String,
    pub granularity: BenchmarkGranularity,
    pub from_key: String,
    pub to_key: String,
    pub segment_hashes: Vec<String>,
}

/// One exact retained segment revision and the immutable manifest that proves
/// its original acquisition association. Composition never searches latest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BenchmarkRetainedSegmentRef {
    pub source_manifest_hash: String,
    pub segment_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BenchmarkSegmentStoreError {
    BenchmarkSegmentUnavailable {
        reason_code: &'static str,
        retryable: bool,
        detail: String,
    },
    FailedIntegrity {
        reason_code: &'static str,
        detail: String,
    },
}

impl BenchmarkSegmentStoreError {
    pub fn reason_code(&self) -> &'static str {
        match self {
            Self::BenchmarkSegmentUnavailable { reason_code, .. }
            | Self::FailedIntegrity { reason_code, .. } => reason_code,
        }
    }

    pub fn retryable(&self) -> bool {
        match self {
            Self::BenchmarkSegmentUnavailable { retryable, .. } => *retryable,
            Self::FailedIntegrity { .. } => false,
        }
    }

    pub fn detail(&self) -> &str {
        match self {
            Self::BenchmarkSegmentUnavailable { detail, .. }
            | Self::FailedIntegrity { detail, .. } => detail,
        }
    }

    #[cfg(test)]
    fn contains(&self, pattern: &str) -> bool {
        self.reason_code().contains(pattern) || self.detail().contains(pattern)
    }
}

impl std::fmt::Display for BenchmarkSegmentStoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BenchmarkSegmentUnavailable {
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

impl std::error::Error for BenchmarkSegmentStoreError {}

pub struct BenchmarkSegmentStore<'a> {
    database: &'a DatabaseManager,
}

#[derive(Debug, Serialize, Deserialize)]
struct CanonicalPayloadV1 {
    schema: String,
    instrument: String,
    granularity: String,
    quarter_start: String,
    bars: Vec<CanonicalBarV1>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CanonicalBarV1 {
    time_kind: String,
    at: String,
    open_bits: u64,
    high_bits: u64,
    low_bits: u64,
    close_bits: u64,
    volume_bits: Option<u64>,
    amount_bits: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CanonicalManifestAcquisitionMemberV1 {
    segment_hash: String,
    record_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CanonicalManifestAcquisitionBindingV1 {
    schema: String,
    audit_id: i64,
    acquisition_record_hash: String,
    provider: String,
    source: String,
    request_hash: String,
    source_at: Option<String>,
    observed_at: String,
    batch_id: String,
    accepted_count: i64,
    members: Vec<CanonicalManifestAcquisitionMemberV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_manifest_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_binding_hash: Option<String>,
}

struct PreparedSegment {
    instrument: String,
    granularity: String,
    quarter_start: String,
    state: String,
    first_key: String,
    last_key: String,
    record_count: i64,
    canonical_hash: String,
    compressed_hash: String,
    compressed_payload: Vec<u8>,
    provider: String,
    source: String,
    source_at: Option<String>,
    observed_at: String,
    batch_id: String,
    acquisition_audit_id: i64,
    acquisition_record_hash: String,
    receipt_previous_outcome: Option<String>,
    receipt_current_outcome: String,
    bars: Vec<BenchmarkBar>,
    segment_hash: String,
}

#[derive(Debug, QueryableByName, Serialize)]
struct PersistedSegment {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = Text)]
    segment_hash: String,
    #[diesel(sql_type = Text)]
    instrument: String,
    #[diesel(sql_type = Text)]
    granularity: String,
    #[diesel(sql_type = Text)]
    quarter_start: String,
    #[diesel(sql_type = Text)]
    state: String,
    #[diesel(sql_type = Text)]
    first_key: String,
    #[diesel(sql_type = Text)]
    last_key: String,
    #[diesel(sql_type = BigInt)]
    record_count: i64,
    #[diesel(sql_type = Text)]
    canonical_hash: String,
    #[diesel(sql_type = Text)]
    compressed_hash: String,
    #[diesel(sql_type = Text)]
    codec: String,
    #[diesel(sql_type = Integer)]
    codec_version: i32,
    #[diesel(sql_type = Integer)]
    payload_version: i32,
    #[diesel(sql_type = Binary)]
    compressed_payload: Vec<u8>,
    #[diesel(sql_type = Text)]
    provider: String,
    #[diesel(sql_type = Text)]
    source: String,
    #[diesel(sql_type = Nullable<Text>)]
    source_at: Option<String>,
    #[diesel(sql_type = Text)]
    observed_at: String,
    #[diesel(sql_type = Text)]
    batch_id: String,
    #[diesel(sql_type = BigInt)]
    acquisition_audit_id: i64,
    #[diesel(sql_type = Text)]
    acquisition_record_hash: String,
    #[diesel(sql_type = Nullable<Text>)]
    predecessor_segment_hash: Option<String>,
    #[diesel(sql_type = Text)]
    created_at: String,
    #[diesel(sql_type = Text)]
    retention_deadline: String,
}

#[derive(Debug, QueryableByName)]
struct SegmentChainRow {
    #[diesel(sql_type = BigInt)]
    segment_revision_id: i64,
    #[diesel(sql_type = Text)]
    previous_hash: String,
    #[diesel(sql_type = Text)]
    record_hash: String,
    #[diesel(sql_type = Text)]
    created_at: String,
    #[diesel(sql_type = Text)]
    retention_deadline: String,
}

#[derive(Debug, QueryableByName, Serialize)]
struct PersistedManifest {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = Text)]
    manifest_hash: String,
    #[diesel(sql_type = Text)]
    instrument: String,
    #[diesel(sql_type = Text)]
    granularity: String,
    #[diesel(sql_type = Text)]
    from_key: String,
    #[diesel(sql_type = Text)]
    to_key: String,
    #[diesel(sql_type = Text)]
    segment_hashes_json: String,
    #[diesel(sql_type = Text)]
    created_at: String,
    #[diesel(sql_type = Text)]
    retention_deadline: String,
}

#[derive(Debug, QueryableByName, Serialize)]
struct PersistedManifestAcquisition {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = BigInt)]
    manifest_id: i64,
    #[diesel(sql_type = Integer)]
    ordinal: i32,
    #[diesel(sql_type = Text)]
    binding_hash: String,
    #[diesel(sql_type = Text)]
    canonical_binding_json: String,
    #[diesel(sql_type = Text)]
    created_at: String,
    #[diesel(sql_type = Text)]
    retention_deadline: String,
}

#[derive(Debug, QueryableByName)]
struct ManifestChainRow {
    #[diesel(sql_type = BigInt)]
    manifest_id: i64,
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
struct SequenceRow {
    #[diesel(sql_type = Nullable<BigInt>)]
    seq: Option<i64>,
}

#[derive(Debug, QueryableByName)]
struct IdRow {
    #[diesel(sql_type = BigInt)]
    id: i64,
}

#[derive(Debug, QueryableByName)]
struct TriggerSchemaRow {
    #[diesel(sql_type = Text)]
    name: String,
    #[diesel(sql_type = Text)]
    table_name: String,
    #[diesel(sql_type = Nullable<Text>)]
    sql: Option<String>,
}

#[derive(Debug, QueryableByName)]
struct RetentionWindow {
    #[diesel(sql_type = Text)]
    created_at: String,
    #[diesel(sql_type = Text)]
    retention_deadline: String,
}

#[derive(Debug, QueryableByName)]
struct CountValueRow {
    #[diesel(sql_type = BigInt)]
    count: i64,
}

struct ValidatedBenchmarkState {
    segment_chain_tail: String,
    manifest_chain_tail: String,
}

#[derive(Debug, QueryableByName)]
struct AuditFactsRow {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = Integer)]
    schema_version: i32,
    #[diesel(sql_type = Text)]
    capability: String,
    #[diesel(sql_type = Text)]
    provider: String,
    #[diesel(sql_type = Text)]
    source: String,
    #[diesel(sql_type = Text)]
    request_hash: String,
    #[diesel(sql_type = Nullable<Text>)]
    source_at: Option<String>,
    #[diesel(sql_type = Text)]
    observed_at: String,
    #[diesel(sql_type = Nullable<Text>)]
    batch_id: Option<String>,
    #[diesel(sql_type = Text)]
    outcome: String,
    #[diesel(sql_type = BigInt)]
    request_count: i64,
    #[diesel(sql_type = BigInt)]
    accepted_count: i64,
    #[diesel(sql_type = BigInt)]
    rejected_count: i64,
    #[diesel(sql_type = Text)]
    reason_code: String,
    #[diesel(sql_type = Integer)]
    retryable: i32,
    #[diesel(sql_type = Text)]
    record_hash: String,
}

#[derive(Debug, QueryableByName)]
struct PreviousAuditOutcomeRow {
    #[diesel(sql_type = Text)]
    outcome: String,
}

fn store_error(message: impl Into<String>) -> diesel::result::Error {
    typed_store_error(failed_integrity(
        "benchmark_segment_failed_integrity",
        message,
    ))
}

fn failed_integrity(
    reason_code: &'static str,
    detail: impl Into<String>,
) -> BenchmarkSegmentStoreError {
    BenchmarkSegmentStoreError::FailedIntegrity {
        reason_code,
        detail: detail.into(),
    }
}

fn unavailable(
    reason_code: &'static str,
    retryable: bool,
    detail: impl Into<String>,
) -> BenchmarkSegmentStoreError {
    BenchmarkSegmentStoreError::BenchmarkSegmentUnavailable {
        reason_code,
        retryable,
        detail: detail.into(),
    }
}

fn typed_store_error(error: BenchmarkSegmentStoreError) -> diesel::result::Error {
    diesel::result::Error::QueryBuilderError(Box::new(error))
}

fn integrity_store_error(
    reason_code: &'static str,
    detail: impl Into<String>,
) -> diesel::result::Error {
    typed_store_error(failed_integrity(reason_code, detail))
}

fn unavailable_store_error(
    reason_code: &'static str,
    retryable: bool,
    detail: impl Into<String>,
) -> diesel::result::Error {
    typed_store_error(unavailable(reason_code, retryable, detail))
}

fn map_benchmark_error(
    error: BenchmarkError,
    detail: impl Into<String>,
) -> BenchmarkSegmentStoreError {
    let detail = detail.into();
    match error {
        BenchmarkError::Unavailable { code, retryable } => unavailable(code, retryable, detail),
        BenchmarkError::FailedIntegrity { code } => failed_integrity(code, detail),
        BenchmarkError::Unsupported(unsupported) => failed_integrity(
            "benchmark_request_unsupported",
            format!("{detail}: {unsupported:?}"),
        ),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DieselErrorContext {
    TransactionBody,
    TransactionEnvelope,
}

fn map_diesel_error(
    error: diesel::result::Error,
    operation: &'static str,
    context: DieselErrorContext,
) -> BenchmarkSegmentStoreError {
    match error {
        diesel::result::Error::InvalidCString(source) => failed_integrity(
            "benchmark_segment_query_invalid_cstring",
            format!("{operation}: {source}"),
        ),
        diesel::result::Error::QueryBuilderError(source) => {
            match source.downcast::<BenchmarkSegmentStoreError>() {
                Ok(error) => *error,
                Err(source) => failed_integrity(
                    "benchmark_segment_query_failed_integrity",
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
                "benchmark_segment_storage_constraint",
                format!("{operation}: {}", information.message()),
            ),
            diesel::result::DatabaseErrorKind::ReadOnlyTransaction => unavailable(
                "benchmark_segment_storage_read_only",
                false,
                format!("{operation}: {}", information.message()),
            ),
            diesel::result::DatabaseErrorKind::UnableToSendCommand => failed_integrity(
                "benchmark_segment_storage_protocol",
                format!("{operation}: {}", information.message()),
            ),
            diesel::result::DatabaseErrorKind::SerializationFailure => unavailable(
                "benchmark_segment_transaction_conflict",
                true,
                format!("{operation}: {}", information.message()),
            ),
            diesel::result::DatabaseErrorKind::ClosedConnection => unavailable(
                "benchmark_segment_connection_closed",
                true,
                format!("{operation}: {}", information.message()),
            ),
            diesel::result::DatabaseErrorKind::Unknown => match context {
                DieselErrorContext::TransactionBody => failed_integrity(
                    "benchmark_segment_storage_body_unknown",
                    format!("{operation}: {}", information.message()),
                ),
                DieselErrorContext::TransactionEnvelope => unavailable(
                    "benchmark_segment_storage_unavailable",
                    true,
                    format!("{operation}: {}", information.message()),
                ),
            },
            // Diesel's enums are non-exhaustive. A future unclassified kind is
            // conservatively non-retryable until its semantics are registered.
            _ => failed_integrity(
                "benchmark_segment_storage_unclassified",
                format!("{operation}: {}", information.message()),
            ),
        },
        diesel::result::Error::NotFound => failed_integrity(
            "benchmark_segment_unexpected_not_found",
            format!("{operation}: an expected persistence row was not found"),
        ),
        diesel::result::Error::DeserializationError(source)
        | diesel::result::Error::SerializationError(source) => failed_integrity(
            "benchmark_segment_storage_serialization",
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
            if matches!(rollback, BenchmarkSegmentStoreError::FailedIntegrity { .. })
                || matches!(commit, BenchmarkSegmentStoreError::FailedIntegrity { .. })
            {
                failed_integrity(
                    "benchmark_segment_transaction_rollback_commit_integrity",
                    detail,
                )
            } else {
                let retryable = rollback.retryable() && commit.retryable();
                unavailable(
                    "benchmark_segment_transaction_rollback_commit_unavailable",
                    retryable,
                    detail,
                )
            }
        }
        diesel::result::Error::RollbackTransaction => failed_integrity(
            "benchmark_segment_transaction_rollback_requested",
            format!("{operation}: unexpected explicit rollback request"),
        ),
        diesel::result::Error::AlreadyInTransaction => failed_integrity(
            "benchmark_segment_transaction_already_active",
            format!("{operation}: transaction was already active"),
        ),
        diesel::result::Error::NotInTransaction => failed_integrity(
            "benchmark_segment_transaction_not_active",
            format!("{operation}: transaction was not active"),
        ),
        diesel::result::Error::BrokenTransactionManager => unavailable(
            "benchmark_segment_transaction_manager_broken",
            true,
            format!("{operation}: transaction manager is broken"),
        ),
        // Diesel's top-level error is non-exhaustive. Unknown future variants
        // cannot be assumed transient or safe to retry.
        other => failed_integrity(
            "benchmark_segment_diesel_unclassified",
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

fn granularity_name(granularity: BenchmarkGranularity) -> &'static str {
    match granularity {
        BenchmarkGranularity::Daily => "Daily",
        BenchmarkGranularity::Minute1 => "Minute1",
    }
}

fn parse_granularity(value: &str) -> Result<BenchmarkGranularity, String> {
    match value {
        "Daily" => Ok(BenchmarkGranularity::Daily),
        "Minute1" => Ok(BenchmarkGranularity::Minute1),
        other => Err(format!("BR-251 unknown benchmark granularity {other}")),
    }
}

fn request_parts(
    request: &BenchmarkRequest,
) -> Result<(BenchmarkGranularity, String, String), String> {
    if request.instrument.trim().is_empty() {
        return Err("BR-251 benchmark instrument must not be blank".into());
    }
    match &request.range {
        BenchmarkRange::Daily { from, to } if from <= to => Ok((
            BenchmarkGranularity::Daily,
            from.format("%Y-%m-%d").to_string(),
            to.format("%Y-%m-%d").to_string(),
        )),
        BenchmarkRange::Minute1 { from, to }
            if from <= to
                && from.offset().local_minus_utc() == 8 * 60 * 60
                && to.offset().local_minus_utc() == 8 * 60 * 60
                && from.date_naive() == to.date_naive() =>
        {
            Ok((
                BenchmarkGranularity::Minute1,
                from.to_rfc3339(),
                to.to_rfc3339(),
            ))
        }
        BenchmarkRange::Daily { .. } => Err("BR-251 benchmark request range is reversed".into()),
        BenchmarkRange::Minute1 { from, to } if from > to => {
            Err("BR-251 benchmark request range is reversed".into())
        }
        BenchmarkRange::Minute1 { .. } => {
            Err("BR-251 benchmark minute request must be one Asia/Shanghai business day".into())
        }
    }
}

fn quarter_end(quarter_start: NaiveDate) -> Result<NaiveDate, String> {
    if quarter_start.day() != 1 || !matches!(quarter_start.month(), 1 | 4 | 7 | 10) {
        return Err(format!(
            "BR-251 quarter_start {} is not a natural-quarter boundary",
            quarter_start
        ));
    }
    let next = if quarter_start.month() == 10 {
        NaiveDate::from_ymd_opt(quarter_start.year() + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(quarter_start.year(), quarter_start.month() + 3, 1)
    }
    .ok_or_else(|| "BR-251 natural-quarter boundary overflows".to_string())?;
    Ok(next - Duration::days(1))
}

fn bar_key_and_kind(bar: &BenchmarkBar) -> (String, &'static str, NaiveDate) {
    match &bar.at {
        BenchmarkBarTime::Daily(day) => (day.format("%Y-%m-%d").to_string(), "daily", *day),
        BenchmarkBarTime::MinuteEnd(at) => (at.to_rfc3339(), "minute1", at.date_naive()),
    }
}

fn validate_number(value: f64, field: &str, allow_zero: bool) -> Result<(), String> {
    if !value.is_finite() || (!allow_zero && value <= 0.0) || (allow_zero && value < 0.0) {
        return Err(format!("BR-251 invalid benchmark {field}"));
    }
    Ok(())
}

fn validate_bar(bar: &BenchmarkBar) -> Result<(), String> {
    for (field, value) in [
        ("open", bar.open),
        ("high", bar.high),
        ("low", bar.low),
        ("close", bar.close),
    ] {
        validate_number(value, field, false)?;
    }
    if bar.low > bar.open
        || bar.low > bar.close
        || bar.high < bar.open
        || bar.high < bar.close
        || bar.low > bar.high
    {
        return Err("BR-251 benchmark OHLC relationship is invalid".into());
    }
    if let Some(volume) = bar.volume {
        validate_number(volume, "volume", true)?;
    }
    if let Some(amount) = bar.amount {
        validate_number(amount, "amount", true)?;
    }
    Ok(())
}

fn canonical_bar(bar: &BenchmarkBar) -> CanonicalBarV1 {
    let (at, time_kind, _) = bar_key_and_kind(bar);
    CanonicalBarV1 {
        time_kind: time_kind.into(),
        at,
        open_bits: bar.open.to_bits(),
        high_bits: bar.high.to_bits(),
        low_bits: bar.low.to_bits(),
        close_bits: bar.close.to_bits(),
        volume_bits: bar.volume.map(f64::to_bits),
        amount_bits: bar.amount.map(f64::to_bits),
    }
}

fn validate_sealed_segment(
    input: &BenchmarkSegmentAppend,
    granularity: BenchmarkGranularity,
    natural_quarter_end: NaiveDate,
) -> Result<(), String> {
    if input.state != SegmentState::Sealed {
        return Ok(());
    }
    if granularity == BenchmarkGranularity::Minute1 {
        return Err(
            "BR-251 Minute1 Sealed unavailable without complete-quarter Capture proof".into(),
        );
    }

    let complete_quarter_request = BenchmarkRequest {
        instrument: input.request.instrument.clone(),
        range: BenchmarkRange::Daily {
            from: input.quarter_start,
            to: natural_quarter_end,
        },
    };
    complete_quarter_request
        .validate_persisted_payload(&input.bars)
        .map_err(|_| "BR-251 Daily Sealed requires complete natural-quarter coverage".to_owned())?;

    let observed_at = DateTime::<FixedOffset>::parse_from_rfc3339(&input.evidence.observed_at)
        .map_err(|_| "BR-251 Daily Sealed evidence observed_at must be RFC3339".to_owned())?;
    let shanghai = FixedOffset::east_opt(8 * 60 * 60)
        .ok_or_else(|| "BR-251 Asia/Shanghai offset is unavailable".to_owned())?;
    if observed_at.with_timezone(&shanghai).date_naive() <= natural_quarter_end {
        return Err(
            "BR-251 Daily Sealed evidence observed_at must be after natural-quarter end".into(),
        );
    }
    Ok(())
}

fn prepare_segment(input: BenchmarkSegmentAppend) -> Result<PreparedSegment, String> {
    let (granularity, request_from, request_to) = request_parts(&input.request)?;
    let quarter_end = quarter_end(input.quarter_start)?;
    if input.bars.is_empty() {
        return Err("BR-251 benchmark segment must contain at least one bar".into());
    }
    for (field, value) in [
        ("source", input.evidence.source.as_str()),
        ("observed_at", input.evidence.observed_at.as_str()),
        ("batch_id", input.evidence.batch_id.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(format!(
                "BR-251 benchmark evidence {field} must not be blank"
            ));
        }
    }
    if input.acquisition_receipt.audit_id <= 0
        || !is_lower_hex_hash(&input.acquisition_receipt.record_hash)
    {
        return Err("BR-159 benchmark acquisition receipt is malformed".into());
    }
    let expected_kind = match granularity {
        BenchmarkGranularity::Daily => "daily",
        BenchmarkGranularity::Minute1 => "minute1",
    };
    let mut previous_key: Option<String> = None;
    let mut canonical_bars = Vec::with_capacity(input.bars.len());
    for bar in &input.bars {
        validate_bar(bar)?;
        if let BenchmarkBarTime::MinuteEnd(at) = &bar.at {
            if at.offset().local_minus_utc() != 8 * 60 * 60 {
                return Err("BR-251 benchmark minute bar must use Asia/Shanghai offset".into());
            }
        }
        let (key, kind, day) = bar_key_and_kind(bar);
        if kind != expected_kind {
            return Err("BR-251 Daily and Minute1 bars cannot share a segment".into());
        }
        if day < input.quarter_start || day > quarter_end {
            return Err(format!(
                "BR-251 benchmark bar {key} lies outside natural quarter {}..={quarter_end}",
                input.quarter_start
            ));
        }
        if key < request_from || key > request_to {
            return Err(format!(
                "BR-251 benchmark bar {key} lies outside request range"
            ));
        }
        if previous_key
            .as_ref()
            .is_some_and(|previous| previous >= &key)
        {
            return Err("BR-251 benchmark segment bars must be strictly ordered".into());
        }
        previous_key = Some(key);
        canonical_bars.push(canonical_bar(bar));
    }
    validate_sealed_segment(&input, granularity, quarter_end)?;
    let first_key = canonical_bars
        .first()
        .map(|bar| bar.at.clone())
        .ok_or_else(|| "BR-251 benchmark segment has no first bar".to_string())?;
    let last_key = canonical_bars
        .last()
        .map(|bar| bar.at.clone())
        .ok_or_else(|| "BR-251 benchmark segment has no last bar".to_string())?;
    let granularity = granularity_name(granularity).to_string();
    let quarter_start = input.quarter_start.format("%Y-%m-%d").to_string();
    let canonical = serde_json::to_vec(&CanonicalPayloadV1 {
        schema: PAYLOAD_SCHEMA.into(),
        instrument: input.request.instrument.clone(),
        granularity: granularity.clone(),
        quarter_start: quarter_start.clone(),
        bars: canonical_bars,
    })
    .map_err(|error| format!("BR-251 canonical benchmark payload: {error}"))?;
    let canonical_hash = hash_with_domain(b"BR251_BENCHMARK_CANONICAL_PAYLOAD_V1", &[&canonical]);
    let compressed_payload = zstd::stream::encode_all(Cursor::new(&canonical), 0)
        .map_err(|error| format!("BR-251 zstd encode benchmark payload: {error}"))?;
    let compressed_hash = hash_with_domain(
        b"BR251_BENCHMARK_COMPRESSED_PAYLOAD_V1",
        &[&compressed_payload],
    );
    let state = input.state.as_str().to_string();
    let segment_hash = hash_with_domain(
        b"BR251_BENCHMARK_SEGMENT_ID_V1",
        &[
            input.request.instrument.as_bytes(),
            granularity.as_bytes(),
            quarter_start.as_bytes(),
            state.as_bytes(),
            canonical_hash.as_bytes(),
        ],
    );
    Ok(PreparedSegment {
        instrument: input.request.instrument,
        granularity,
        quarter_start,
        state,
        first_key,
        last_key,
        record_count: input.bars.len() as i64,
        canonical_hash,
        compressed_hash,
        compressed_payload,
        provider: format!("{:?}", input.evidence.provider),
        source: input.evidence.source,
        source_at: input.evidence.source_at,
        observed_at: input.evidence.observed_at,
        batch_id: input.evidence.batch_id,
        acquisition_audit_id: input.acquisition_receipt.audit_id,
        acquisition_record_hash: input.acquisition_receipt.record_hash,
        receipt_previous_outcome: input.acquisition_receipt.previous_outcome,
        receipt_current_outcome: input.acquisition_receipt.current_outcome,
        bars: input.bars,
        segment_hash,
    })
}

fn load_segments(conn: &mut SqliteConnection) -> diesel::QueryResult<Vec<PersistedSegment>> {
    diesel::sql_query(
        "SELECT id, segment_hash, instrument, granularity, quarter_start, state,
                first_key, last_key, record_count, canonical_hash, compressed_hash,
                codec, codec_version, payload_version, compressed_payload, provider,
                source, source_at, observed_at, batch_id, acquisition_audit_id,
                acquisition_record_hash, predecessor_segment_hash, created_at,
                retention_deadline
         FROM benchmark_segment_revision ORDER BY id ASC",
    )
    .load(conn)
}

fn load_segment_chains(conn: &mut SqliteConnection) -> diesel::QueryResult<Vec<SegmentChainRow>> {
    diesel::sql_query(
        "SELECT segment_revision_id, previous_hash, record_hash, created_at, retention_deadline
         FROM benchmark_segment_chain ORDER BY segment_revision_id ASC",
    )
    .load(conn)
}

fn load_manifests(conn: &mut SqliteConnection) -> diesel::QueryResult<Vec<PersistedManifest>> {
    diesel::sql_query(
        "SELECT id, manifest_hash, instrument, granularity, from_key, to_key,
                segment_hashes_json, created_at, retention_deadline
         FROM benchmark_manifest ORDER BY id ASC",
    )
    .load(conn)
}

fn build_manifest_acquisition_bindings(
    prepared: &[PreparedSegment],
    request_hash: &str,
) -> Result<Vec<CanonicalManifestAcquisitionBindingV1>, String> {
    let mut bindings = Vec::<CanonicalManifestAcquisitionBindingV1>::new();
    let mut binding_by_audit = HashMap::<i64, usize>::new();
    for segment in prepared {
        let member = CanonicalManifestAcquisitionMemberV1 {
            segment_hash: segment.segment_hash.clone(),
            record_count: segment.record_count,
        };
        if let Some(index) = binding_by_audit.get(&segment.acquisition_audit_id).copied() {
            let binding = &mut bindings[index];
            if binding.acquisition_record_hash != segment.acquisition_record_hash
                || binding.provider != segment.provider
                || binding.source != segment.source
                || binding.source_at != segment.source_at
                || binding.observed_at != segment.observed_at
                || binding.batch_id != segment.batch_id
            {
                return Err(
                    "BR-251 one acquisition binding has conflicting receipt/evidence facts".into(),
                );
            }
            binding.accepted_count += segment.record_count;
            binding.members.push(member);
        } else {
            binding_by_audit.insert(segment.acquisition_audit_id, bindings.len());
            bindings.push(CanonicalManifestAcquisitionBindingV1 {
                schema: MANIFEST_ACQUISITION_SCHEMA.into(),
                audit_id: segment.acquisition_audit_id,
                acquisition_record_hash: segment.acquisition_record_hash.clone(),
                provider: segment.provider.clone(),
                source: segment.source.clone(),
                request_hash: request_hash.into(),
                source_at: segment.source_at.clone(),
                observed_at: segment.observed_at.clone(),
                batch_id: segment.batch_id.clone(),
                accepted_count: segment.record_count,
                members: vec![member],
                source_manifest_hash: None,
                source_binding_hash: None,
            });
        }
    }
    Ok(bindings)
}

fn canonical_manifest_acquisition_bytes(
    bindings: &[CanonicalManifestAcquisitionBindingV1],
) -> Result<Vec<u8>, String> {
    serde_json::to_vec(bindings)
        .map_err(|error| format!("BR-251 serialize manifest acquisition bindings: {error}"))
}

fn manifest_acquisition_binding_hash(
    binding: &CanonicalManifestAcquisitionBindingV1,
) -> Result<String, String> {
    let bytes = serde_json::to_vec(binding)
        .map_err(|error| format!("BR-251 serialize manifest acquisition binding: {error}"))?;
    Ok(hash_with_domain(
        b"BR251_BENCHMARK_MANIFEST_ACQUISITION_BINDING_V1",
        &[&bytes],
    ))
}

fn parse_persisted_utc_timestamp(value: &str) -> Result<DateTime<Utc>, String> {
    let Some(without_z) = value.strip_suffix('Z') else {
        return Err(format!("persisted timestamp is not canonical UTC: {value}"));
    };
    let Some((whole_seconds, fractional)) = without_z.rsplit_once('.') else {
        return Err(format!(
            "persisted timestamp does not retain fractional seconds: {value}"
        ));
    };
    if !whole_seconds.contains('T')
        || fractional.is_empty()
        || fractional.len() > 9
        || !fractional.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(format!(
            "persisted timestamp has noncanonical fractional UTC bytes: {value}"
        ));
    }
    let parsed = DateTime::parse_from_rfc3339(value)
        .map_err(|error| format!("invalid persisted UTC timestamp {value}: {error}"))?;
    if parsed.offset().local_minus_utc() != 0 {
        return Err(format!("persisted timestamp is not UTC: {value}"));
    }
    Ok(parsed.with_timezone(&Utc))
}

fn validate_retention_window(
    family: &str,
    created_at: &str,
    retention_deadline: &str,
) -> diesel::QueryResult<()> {
    let created_at = parse_persisted_utc_timestamp(created_at).map_err(|detail| {
        integrity_store_error(
            "benchmark_retention_invalid",
            format!("BR-251 {family} created_at is invalid: {detail}"),
        )
    })?;
    let retention_deadline =
        parse_persisted_utc_timestamp(retention_deadline).map_err(|detail| {
            integrity_store_error(
                "benchmark_retention_invalid",
                format!("BR-251 {family} retention_deadline is invalid: {detail}"),
            )
        })?;
    if retention_deadline < created_at {
        return Err(integrity_store_error(
            "benchmark_retention_invalid",
            format!("BR-251 {family} retention_deadline precedes created_at"),
        ));
    }
    let minimum_deadline = created_at
        .checked_add_months(Months::new(60))
        .ok_or_else(|| {
            integrity_store_error(
                "benchmark_retention_invalid",
                format!("BR-251 {family} 60-calendar-month deadline overflows"),
            )
        })?;
    if retention_deadline < minimum_deadline {
        return Err(integrity_store_error(
            "benchmark_retention_invalid",
            format!("BR-251 {family} retention must be at least 60 calendar months"),
        ));
    }
    Ok(())
}

fn validate_manifest_acquisition_retention(
    row: &PersistedManifestAcquisition,
) -> diesel::QueryResult<()> {
    validate_retention_window(
        "benchmark manifest acquisition",
        &row.created_at,
        &row.retention_deadline,
    )
}

fn load_manifest_acquisitions(
    conn: &mut SqliteConnection,
    manifest_id: i64,
) -> diesel::QueryResult<(
    Vec<PersistedManifestAcquisition>,
    Vec<CanonicalManifestAcquisitionBindingV1>,
)> {
    let rows = diesel::sql_query(
        "SELECT id, manifest_id, ordinal, binding_hash, canonical_binding_json,
                created_at, retention_deadline
         FROM benchmark_manifest_acquisition WHERE manifest_id = ? ORDER BY ordinal ASC",
    )
    .bind::<BigInt, _>(manifest_id)
    .load::<PersistedManifestAcquisition>(conn)?;
    if rows.is_empty() {
        return Err(integrity_store_error(
            "benchmark_manifest_acquisition_missing",
            "BR-251 manifest acquisition association is missing",
        ));
    }
    let mut bindings = Vec::with_capacity(rows.len());
    let mut audit_ids = HashSet::new();
    for (expected_ordinal, row) in rows.iter().enumerate() {
        if row.id <= 0 || row.manifest_id != manifest_id || row.ordinal != expected_ordinal as i32 {
            return Err(integrity_store_error(
                "benchmark_manifest_acquisition_ordinal_mismatch",
                "BR-251 manifest acquisition association ordinal mismatch",
            ));
        }
        validate_manifest_acquisition_retention(row)?;
        let binding: CanonicalManifestAcquisitionBindingV1 =
            serde_json::from_str(&row.canonical_binding_json).map_err(|error| {
                integrity_store_error(
                    "benchmark_manifest_acquisition_invalid",
                    format!("BR-251 decode manifest acquisition binding: {error}"),
                )
            })?;
        let canonical = serde_json::to_string(&binding).map_err(|error| {
            integrity_store_error(
                "benchmark_manifest_acquisition_invalid",
                format!("BR-251 serialize retained manifest acquisition binding: {error}"),
            )
        })?;
        if canonical != row.canonical_binding_json {
            return Err(integrity_store_error(
                "benchmark_manifest_acquisition_noncanonical",
                "BR-251 manifest acquisition binding bytes are not canonical",
            ));
        }
        let expected_hash = manifest_acquisition_binding_hash(&binding).map_err(store_error)?;
        if !is_lower_hex_hash(&row.binding_hash) || row.binding_hash != expected_hash {
            return Err(integrity_store_error(
                "benchmark_manifest_acquisition_hash_mismatch",
                "BR-251 manifest acquisition binding hash mismatch",
            ));
        }
        let provenance_shape_valid = match binding.schema.as_str() {
            MANIFEST_ACQUISITION_SCHEMA => {
                binding.source_manifest_hash.is_none() && binding.source_binding_hash.is_none()
            }
            COMPOSED_MANIFEST_ACQUISITION_SCHEMA => {
                binding
                    .source_manifest_hash
                    .as_deref()
                    .is_some_and(is_lower_hex_hash)
                    && binding
                        .source_binding_hash
                        .as_deref()
                        .is_some_and(is_lower_hex_hash)
            }
            _ => false,
        };
        if !provenance_shape_valid
            || binding.audit_id <= 0
            || !is_lower_hex_hash(&binding.acquisition_record_hash)
            || !is_lower_hex_hash(&binding.request_hash)
            || binding.accepted_count <= 0
            || binding.members.is_empty()
            || binding
                .members
                .iter()
                .any(|member| !is_lower_hex_hash(&member.segment_hash) || member.record_count <= 0)
        {
            return Err(integrity_store_error(
                "benchmark_manifest_acquisition_invalid",
                "BR-251 manifest acquisition binding shape is invalid",
            ));
        }
        if !audit_ids.insert(binding.audit_id) {
            return Err(integrity_store_error(
                "benchmark_manifest_acquisition_audit_duplicate",
                "BR-251 manifest acquisition association repeats an audit id",
            ));
        }
        bindings.push(binding);
    }
    Ok((rows, bindings))
}

fn load_manifest_chains(conn: &mut SqliteConnection) -> diesel::QueryResult<Vec<ManifestChainRow>> {
    diesel::sql_query(
        "SELECT manifest_id, previous_hash, record_hash, created_at, retention_deadline
         FROM benchmark_manifest_chain ORDER BY manifest_id ASC",
    )
    .load(conn)
}

fn segment_chain_hash(
    previous_hash: &str,
    row: &PersistedSegment,
    created_at: &str,
    retention_deadline: &str,
) -> diesel::QueryResult<String> {
    let payload = serde_json::to_vec(row)
        .map_err(|error| store_error(format!("BR-251 serialize segment row: {error}")))?;
    Ok(hash_with_domain(
        b"BR251_BENCHMARK_SEGMENT_CHAIN_V2",
        &[
            previous_hash.as_bytes(),
            &payload,
            created_at.as_bytes(),
            retention_deadline.as_bytes(),
        ],
    ))
}

fn manifest_chain_hash(
    previous_hash: &str,
    row: &PersistedManifest,
    associations: &[PersistedManifestAcquisition],
    created_at: &str,
    retention_deadline: &str,
) -> diesel::QueryResult<String> {
    let payload = serde_json::to_vec(row)
        .map_err(|error| store_error(format!("BR-251 serialize manifest row: {error}")))?;
    let acquisition = serde_json::to_vec(associations).map_err(|error| {
        store_error(format!(
            "BR-251 serialize persisted manifest acquisition rows: {error}"
        ))
    })?;
    Ok(hash_with_domain(
        b"BR251_BENCHMARK_MANIFEST_CHAIN_V2",
        &[
            previous_hash.as_bytes(),
            &payload,
            &acquisition,
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

fn immutable_triggers(
    conn: &mut SqliteConnection,
    table: &str,
    update_action: &str,
    delete_action: &str,
) -> diesel::QueryResult<()> {
    diesel::sql_query(format!(
        "CREATE TRIGGER IF NOT EXISTS trg_{table}_no_update
         BEFORE UPDATE ON {table}
         BEGIN SELECT RAISE(ABORT, '{update_action}'); END",
    ))
    .execute(conn)?;
    diesel::sql_query(format!(
        "CREATE TRIGGER IF NOT EXISTS trg_{table}_no_delete
         BEFORE DELETE ON {table}
         BEGIN SELECT RAISE(ABORT, '{delete_action}'); END",
    ))
    .execute(conn)?;
    Ok(())
}

fn expected_trigger_sql(table: &str, event: &str, action: &str) -> String {
    format!(
        "CREATE TRIGGER trg_{table}_no_{} BEFORE {event} ON {table} \
         BEGIN SELECT RAISE(ABORT, '{action}'); END",
        event.to_ascii_lowercase(),
    )
}

fn normalized_sql(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn validate_immutable_triggers(conn: &mut SqliteConnection) -> diesel::QueryResult<()> {
    for (table, update_action, delete_action) in IMMUTABLE_TABLES {
        for (event, action) in [("UPDATE", update_action), ("DELETE", delete_action)] {
            let name = format!("trg_{table}_no_{}", event.to_ascii_lowercase());
            let expected = expected_trigger_sql(table, event, action);
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
                return Err(integrity_store_error(
                    "benchmark_trigger_definition_invalid",
                    format!(
                        "BR-251 immutable trigger {name} does not match its canonical table, timing, event and abort action"
                    ),
                ));
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
    let exact = match sequence {
        None => ids.is_empty(),
        Some(SequenceRow {
            seq: Some(highwater),
        }) if highwater > 0 => ids.iter().copied().eq(1..=highwater),
        Some(_) => false,
    };
    if !exact {
        return Err(integrity_store_error(
            "benchmark_sequence_highwater_invalid",
            format!(
                "BR-251 {table} AUTOINCREMENT high-water does not match its full append-only identity sequence"
            ),
        ));
    }
    Ok(())
}

fn validate_all_state(conn: &mut SqliteConnection) -> diesel::QueryResult<ValidatedBenchmarkState> {
    validate_immutable_triggers(conn)?;
    let segment_ids = load_segments(conn)?
        .into_iter()
        .map(|row| row.id)
        .collect::<Vec<_>>();
    let manifest_ids = load_manifests(conn)?
        .into_iter()
        .map(|row| row.id)
        .collect::<Vec<_>>();
    let association_ids =
        diesel::sql_query("SELECT id FROM benchmark_manifest_acquisition ORDER BY id ASC")
            .load::<IdRow>(conn)?
            .into_iter()
            .map(|row| row.id)
            .collect::<Vec<_>>();
    validate_autoincrement_highwater(conn, "benchmark_segment_revision", &segment_ids)?;
    validate_autoincrement_highwater(conn, "benchmark_manifest", &manifest_ids)?;
    validate_autoincrement_highwater(conn, "benchmark_manifest_acquisition", &association_ids)?;
    let segment_chain_tail = validate_segment_chain(conn)?;
    let manifest_chain_tail = validate_manifest_chain(conn)?;
    Ok(ValidatedBenchmarkState {
        segment_chain_tail,
        manifest_chain_tail,
    })
}

fn validate_segment_chain(conn: &mut SqliteConnection) -> diesel::QueryResult<String> {
    let rows = load_segments(conn)?;
    let chain = load_segment_chains(conn)?;
    if rows.len() != chain.len() {
        return Err(store_error(format!(
            "BR-251 segment chain length mismatch: rows={}, links={}",
            rows.len(),
            chain.len()
        )));
    }
    let mut previous = SEGMENT_CHAIN_GENESIS.to_string();
    let mut identity_tails: HashMap<(&str, &str, &str), &str> = HashMap::new();
    for (row, link) in rows.iter().zip(chain.iter()) {
        validate_retention_window(
            "benchmark segment revision",
            &row.created_at,
            &row.retention_deadline,
        )?;
        validate_retention_window(
            "benchmark segment chain",
            &link.created_at,
            &link.retention_deadline,
        )?;
        if link.segment_revision_id != row.id || link.previous_hash != previous {
            return Err(store_error(format!(
                "BR-251 segment chain linkage mismatch at revision {}",
                row.id
            )));
        }
        let identity = (
            row.instrument.as_str(),
            row.granularity.as_str(),
            row.quarter_start.as_str(),
        );
        let expected_predecessor = identity_tails.get(&identity).copied();
        if row.predecessor_segment_hash.as_deref() != expected_predecessor {
            return Err(store_error(format!(
                "BR-251 segment predecessor mismatch at revision {}",
                row.id
            )));
        }
        decode_segment(row).map_err(store_error)?;
        let expected_hash =
            segment_chain_hash(&previous, row, &link.created_at, &link.retention_deadline)?;
        if link.record_hash != expected_hash {
            return Err(store_error(format!(
                "BR-251 segment chain hash mismatch at revision {}",
                row.id
            )));
        }
        identity_tails.insert(identity, row.segment_hash.as_str());
        previous = link.record_hash.clone();
    }
    Ok(previous)
}

fn validate_manifest_chain(conn: &mut SqliteConnection) -> diesel::QueryResult<String> {
    super::data_acquisition_audit::validate_data_acquisition_audit_chain(conn)?;
    let rows = load_manifests(conn)?;
    let chain = load_manifest_chains(conn)?;
    if rows.len() != chain.len() {
        return Err(store_error(format!(
            "BR-251 manifest chain length mismatch: rows={}, links={}",
            rows.len(),
            chain.len()
        )));
    }
    let mut previous = MANIFEST_CHAIN_GENESIS.to_string();
    for (row, link) in rows.iter().zip(chain.iter()) {
        validate_retention_window(
            "benchmark manifest",
            &row.created_at,
            &row.retention_deadline,
        )?;
        validate_retention_window(
            "benchmark manifest chain",
            &link.created_at,
            &link.retention_deadline,
        )?;
        if link.manifest_id != row.id || link.previous_hash != previous {
            return Err(store_error(format!(
                "BR-251 manifest chain linkage mismatch at manifest {}",
                row.id
            )));
        }
        let (associations, bindings) = load_manifest_acquisitions(conn, row.id)?;
        let manifest = decoded_manifest_ref(row).map_err(store_error)?;
        let segments = manifest
            .segment_hashes
            .iter()
            .map(|segment_hash| {
                load_segment_by_hash(conn, segment_hash)?.ok_or_else(|| {
                    store_error(format!(
                        "BR-251 manifest references missing segment {segment_hash}"
                    ))
                })
            })
            .collect::<diesel::QueryResult<Vec<_>>>()?;
        validate_manifest_acquisition_membership(&manifest, &segments, &bindings)
            .map_err(typed_store_error)?;
        validate_composed_acquisition_provenance(conn, row.id, &bindings)?;
        for binding in &bindings {
            verify_audit_facts(
                conn,
                &ExpectedAuditBinding {
                    audit_id: binding.audit_id,
                    record_hash: &binding.acquisition_record_hash,
                    provider: &binding.provider,
                    source: &binding.source,
                    request_hash: &binding.request_hash,
                    source_at: binding.source_at.as_deref(),
                    observed_at: &binding.observed_at,
                    batch_id: &binding.batch_id,
                    accepted_count: binding.accepted_count,
                    receipt_previous_outcome: None,
                    receipt_current_outcome: None,
                    failure_reason_code: "benchmark_manifest_acquisition_audit_invalid",
                },
            )?;
        }
        stored_manifest_ref(row, &bindings).map_err(store_error)?;
        let expected_hash = manifest_chain_hash(
            &previous,
            row,
            &associations,
            &link.created_at,
            &link.retention_deadline,
        )?;
        if link.record_hash != expected_hash {
            return Err(store_error(format!(
                "BR-251 manifest chain hash mismatch at manifest {}",
                row.id
            )));
        }
        previous = link.record_hash.clone();
    }
    Ok(previous)
}

struct ExpectedAuditBinding<'a> {
    audit_id: i64,
    record_hash: &'a str,
    provider: &'a str,
    source: &'a str,
    request_hash: &'a str,
    source_at: Option<&'a str>,
    observed_at: &'a str,
    batch_id: &'a str,
    accepted_count: i64,
    receipt_previous_outcome: Option<&'a Option<String>>,
    receipt_current_outcome: Option<&'a str>,
    failure_reason_code: &'static str,
}

fn audit_fact_mismatch(
    reason_code: &'static str,
    field: &str,
    audit_id: i64,
) -> diesel::result::Error {
    integrity_store_error(
        reason_code,
        format!("BR-159 acquisition audit {field} mismatch at audit id {audit_id}"),
    )
}

fn verify_audit_facts(
    conn: &mut SqliteConnection,
    expected: &ExpectedAuditBinding<'_>,
) -> diesel::QueryResult<()> {
    let retained = diesel::sql_query(
        "SELECT a.id, a.schema_version, a.capability, a.provider, a.source,
                a.request_hash, a.source_at, a.observed_at, a.batch_id, a.outcome,
                a.request_count, a.accepted_count, a.rejected_count, a.reason_code,
                a.retryable, c.record_hash
         FROM data_acquisition_audit a
         JOIN data_acquisition_audit_chain c ON c.acquisition_audit_id = a.id
         WHERE a.id = ?",
    )
    .bind::<BigInt, _>(expected.audit_id)
    .get_result::<AuditFactsRow>(conn)
    .optional()?
    .ok_or_else(|| {
        integrity_store_error(
            expected.failure_reason_code,
            format!(
                "BR-159 acquisition receipt id/hash mismatch at audit id {}",
                expected.audit_id
            ),
        )
    })?;
    if retained.id != expected.audit_id || retained.record_hash != expected.record_hash {
        return Err(integrity_store_error(
            expected.failure_reason_code,
            format!(
                "BR-159 acquisition receipt id/hash mismatch at audit id {}",
                expected.audit_id
            ),
        ));
    }
    for (field, matches) in [
        ("schema_version", retained.schema_version == 1),
        ("capability", retained.capability == "BenchmarkBars"),
        ("provider", retained.provider == expected.provider),
        ("source", retained.source == expected.source),
        (
            "request_hash",
            retained.request_hash == expected.request_hash,
        ),
        (
            "source_at",
            retained.source_at.as_deref() == expected.source_at,
        ),
        ("observed_at", retained.observed_at == expected.observed_at),
        (
            "batch_id",
            retained.batch_id.as_deref() == Some(expected.batch_id),
        ),
        ("outcome", retained.outcome == "available"),
        ("request_count", retained.request_count == 1),
        ("rejected_count", retained.rejected_count == 0),
        ("reason_code", retained.reason_code == "accepted"),
        ("retryable", retained.retryable == 0),
    ] {
        if !matches {
            return Err(audit_fact_mismatch(
                expected.failure_reason_code,
                field,
                expected.audit_id,
            ));
        }
    }
    if retained.accepted_count != expected.accepted_count {
        return Err(integrity_store_error(
            expected.failure_reason_code,
            format!(
                "BR-159 acquisition audit accepted_count mismatch: expected {}, retained {} at audit id {}",
                expected.accepted_count, retained.accepted_count, expected.audit_id
            ),
        ));
    }
    if let (Some(receipt_previous), Some(receipt_current)) = (
        expected.receipt_previous_outcome,
        expected.receipt_current_outcome,
    ) {
        let previous_outcome = diesel::sql_query(
            "SELECT outcome FROM data_acquisition_audit
             WHERE id < ? AND capability = ? AND provider = ?
             ORDER BY id DESC LIMIT 1",
        )
        .bind::<BigInt, _>(retained.id)
        .bind::<Text, _>(&retained.capability)
        .bind::<Text, _>(&retained.provider)
        .get_result::<PreviousAuditOutcomeRow>(conn)
        .optional()?
        .map(|row| row.outcome);
        if previous_outcome.as_ref() != receipt_previous.as_ref() {
            return Err(audit_fact_mismatch(
                expected.failure_reason_code,
                "receipt previous_outcome",
                expected.audit_id,
            ));
        }
        if retained.outcome != receipt_current {
            return Err(audit_fact_mismatch(
                expected.failure_reason_code,
                "receipt current_outcome",
                expected.audit_id,
            ));
        }
    }
    Ok(())
}

fn load_segment_by_hash(
    conn: &mut SqliteConnection,
    segment_hash: &str,
) -> diesel::QueryResult<Option<PersistedSegment>> {
    diesel::sql_query(
        "SELECT id, segment_hash, instrument, granularity, quarter_start, state,
                first_key, last_key, record_count, canonical_hash, compressed_hash,
                codec, codec_version, payload_version, compressed_payload, provider,
                source, source_at, observed_at, batch_id, acquisition_audit_id,
                acquisition_record_hash, predecessor_segment_hash, created_at,
                retention_deadline
         FROM benchmark_segment_revision WHERE segment_hash = ?",
    )
    .bind::<Text, _>(segment_hash)
    .get_result(conn)
    .optional()
}

fn load_manifest_by_hash(
    conn: &mut SqliteConnection,
    manifest_hash: &str,
) -> diesel::QueryResult<Option<PersistedManifest>> {
    diesel::sql_query(
        "SELECT id, manifest_hash, instrument, granularity, from_key, to_key,
                segment_hashes_json, created_at, retention_deadline
         FROM benchmark_manifest WHERE manifest_hash = ?",
    )
    .bind::<Text, _>(manifest_hash)
    .get_result(conn)
    .optional()
}

fn compute_manifest_hash(
    instrument: &str,
    granularity: &str,
    from_key: &str,
    to_key: &str,
    segment_hashes: &[String],
    acquisition_bindings: &[CanonicalManifestAcquisitionBindingV1],
) -> Result<String, String> {
    let hashes = serde_json::to_vec(segment_hashes)
        .map_err(|error| format!("BR-251 serialize manifest segment hashes: {error}"))?;
    let acquisition = canonical_manifest_acquisition_bytes(acquisition_bindings)?;
    Ok(hash_with_domain(
        b"BR251_BENCHMARK_MANIFEST_ID_V1",
        &[
            instrument.as_bytes(),
            granularity.as_bytes(),
            from_key.as_bytes(),
            to_key.as_bytes(),
            &hashes,
            &acquisition,
        ],
    ))
}

fn decoded_manifest_ref(row: &PersistedManifest) -> Result<BenchmarkManifestRef, String> {
    let segment_hashes: Vec<String> = serde_json::from_str(&row.segment_hashes_json)
        .map_err(|error| format!("BR-251 decode manifest segments: {error}"))?;
    if segment_hashes.is_empty() || segment_hashes.iter().any(|hash| !is_lower_hex_hash(hash)) {
        return Err("BR-251 manifest has invalid segment hashes".into());
    }
    Ok(BenchmarkManifestRef {
        manifest_hash: row.manifest_hash.clone(),
        instrument: row.instrument.clone(),
        granularity: parse_granularity(&row.granularity)?,
        from_key: row.from_key.clone(),
        to_key: row.to_key.clone(),
        segment_hashes,
    })
}

fn stored_manifest_ref(
    row: &PersistedManifest,
    acquisition_bindings: &[CanonicalManifestAcquisitionBindingV1],
) -> Result<BenchmarkManifestRef, String> {
    let manifest = decoded_manifest_ref(row)?;
    let expected = compute_manifest_hash(
        &row.instrument,
        &row.granularity,
        &row.from_key,
        &row.to_key,
        &manifest.segment_hashes,
        acquisition_bindings,
    )?;
    if expected != row.manifest_hash {
        return Err("BR-251 manifest content hash mismatch".into());
    }
    Ok(manifest)
}

fn validate_manifest_acquisition_membership(
    manifest: &BenchmarkManifestRef,
    segments: &[PersistedSegment],
    bindings: &[CanonicalManifestAcquisitionBindingV1],
) -> Result<(), BenchmarkSegmentStoreError> {
    let mut segment_counts = HashMap::new();
    for segment in segments {
        if segment_counts
            .insert(segment.segment_hash.as_str(), segment.record_count)
            .is_some()
        {
            return Err(failed_integrity(
                "benchmark_manifest_segment_duplicate",
                "BR-251 manifest repeats a segment hash",
            ));
        }
    }
    if segment_counts.len() != manifest.segment_hashes.len()
        || manifest
            .segment_hashes
            .iter()
            .any(|hash| !segment_counts.contains_key(hash.as_str()))
    {
        return Err(failed_integrity(
            "benchmark_manifest_acquisition_segment_set_mismatch",
            "BR-251 manifest acquisition segment set mismatch",
        ));
    }

    let mut covered = HashSet::new();
    for binding in bindings {
        let mut represented_count = 0_i64;
        for member in &binding.members {
            let Some(record_count) = segment_counts.get(member.segment_hash.as_str()) else {
                return Err(failed_integrity(
                    "benchmark_manifest_acquisition_member_outside",
                    "BR-251 manifest acquisition binding contains a member outside the manifest",
                ));
            };
            if *record_count != member.record_count {
                return Err(failed_integrity(
                    "benchmark_manifest_acquisition_member_count_mismatch",
                    "BR-251 manifest acquisition member count mismatch",
                ));
            }
            if !covered.insert(member.segment_hash.as_str()) {
                return Err(failed_integrity(
                    "benchmark_manifest_acquisition_member_duplicate",
                    "BR-251 manifest acquisition association repeats a segment member",
                ));
            }
            represented_count = represented_count
                .checked_add(member.record_count)
                .ok_or_else(|| {
                    failed_integrity(
                        "benchmark_manifest_acquisition_count_overflow",
                        "BR-251 manifest acquisition member count overflow",
                    )
                })?;
        }
        let count_matches_scope = match binding.schema.as_str() {
            MANIFEST_ACQUISITION_SCHEMA => represented_count == binding.accepted_count,
            COMPOSED_MANIFEST_ACQUISITION_SCHEMA => {
                represented_count > 0 && represented_count <= binding.accepted_count
            }
            _ => false,
        };
        if !count_matches_scope {
            return Err(failed_integrity(
                "benchmark_manifest_acquisition_count_mismatch",
                "BR-251 manifest acquisition accepted_count membership mismatch",
            ));
        }
    }
    if covered.len() != manifest.segment_hashes.len() {
        return Err(failed_integrity(
            "benchmark_manifest_acquisition_member_uncovered",
            "BR-251 manifest acquisition association leaves a member uncovered",
        ));
    }
    Ok(())
}

fn validate_composed_acquisition_provenance(
    conn: &mut SqliteConnection,
    current_manifest_id: i64,
    bindings: &[CanonicalManifestAcquisitionBindingV1],
) -> diesel::QueryResult<()> {
    for binding in bindings {
        if binding.schema != COMPOSED_MANIFEST_ACQUISITION_SCHEMA {
            continue;
        }
        let source_manifest_hash = binding.source_manifest_hash.as_deref().ok_or_else(|| {
            integrity_store_error(
                "benchmark_composition_provenance_invalid",
                "BR-251 composed acquisition is missing its source manifest identity",
            )
        })?;
        let source_binding_hash = binding.source_binding_hash.as_deref().ok_or_else(|| {
            integrity_store_error(
                "benchmark_composition_provenance_invalid",
                "BR-251 composed acquisition is missing its source binding identity",
            )
        })?;
        let source_manifest = load_manifest_by_hash(conn, source_manifest_hash)?
            .filter(|source| source.id < current_manifest_id)
            .ok_or_else(|| {
                integrity_store_error(
                    "benchmark_composition_source_manifest_invalid",
                    "BR-251 composed acquisition source manifest is missing, cyclic or not retained earlier",
                )
            })?;
        let source_ref = decoded_manifest_ref(&source_manifest).map_err(store_error)?;
        let (source_rows, source_bindings) = load_manifest_acquisitions(conn, source_manifest.id)?;
        let source_binding = source_rows
            .iter()
            .zip(source_bindings.iter())
            .find_map(|(row, candidate)| {
                (row.binding_hash == source_binding_hash).then_some(candidate)
            })
            .ok_or_else(|| {
                integrity_store_error(
                    "benchmark_composition_source_binding_invalid",
                    "BR-251 composed acquisition source binding is unavailable",
                )
            })?;
        let original_facts_match = binding.audit_id == source_binding.audit_id
            && binding.acquisition_record_hash == source_binding.acquisition_record_hash
            && binding.provider == source_binding.provider
            && binding.source == source_binding.source
            && binding.request_hash == source_binding.request_hash
            && binding.source_at == source_binding.source_at
            && binding.observed_at == source_binding.observed_at
            && binding.batch_id == source_binding.batch_id
            && binding.accepted_count == source_binding.accepted_count;
        let members_are_retained = binding.members.iter().all(|member| {
            source_ref.segment_hashes.contains(&member.segment_hash)
                && source_binding.members.contains(member)
        });
        if !original_facts_match || !members_are_retained {
            return Err(integrity_store_error(
                "benchmark_composition_provenance_invalid",
                "BR-251 composed acquisition does not preserve its original manifest, receipt and member binding",
            ));
        }
    }
    Ok(())
}

pub(crate) fn request_from_manifest(
    manifest: &BenchmarkManifestRef,
) -> Result<BenchmarkRequest, String> {
    let range = match manifest.granularity {
        BenchmarkGranularity::Daily => BenchmarkRange::Daily {
            from: NaiveDate::parse_from_str(&manifest.from_key, "%Y-%m-%d")
                .map_err(|error| format!("BR-251 decode manifest daily from_key: {error}"))?,
            to: NaiveDate::parse_from_str(&manifest.to_key, "%Y-%m-%d")
                .map_err(|error| format!("BR-251 decode manifest daily to_key: {error}"))?,
        },
        BenchmarkGranularity::Minute1 => BenchmarkRange::Minute1 {
            from: DateTime::<FixedOffset>::parse_from_rfc3339(&manifest.from_key)
                .map_err(|error| format!("BR-251 decode manifest minute from_key: {error}"))?,
            to: DateTime::<FixedOffset>::parse_from_rfc3339(&manifest.to_key)
                .map_err(|error| format!("BR-251 decode manifest minute to_key: {error}"))?,
        },
    };
    Ok(BenchmarkRequest {
        instrument: manifest.instrument.clone(),
        range,
    })
}

fn decode_segment(row: &PersistedSegment) -> Result<Vec<BenchmarkBar>, String> {
    if row.codec != CODEC
        || row.codec_version != CODEC_VERSION
        || row.payload_version != PAYLOAD_VERSION
    {
        return Err("BR-251 benchmark segment codec/version is unsupported".into());
    }
    let compressed_hash = hash_with_domain(
        b"BR251_BENCHMARK_COMPRESSED_PAYLOAD_V1",
        &[&row.compressed_payload],
    );
    if compressed_hash != row.compressed_hash {
        return Err("BR-251 benchmark compressed payload hash mismatch".into());
    }
    let canonical = zstd::stream::decode_all(Cursor::new(&row.compressed_payload))
        .map_err(|error| format!("BR-251 zstd decode benchmark payload: {error}"))?;
    let canonical_hash = hash_with_domain(b"BR251_BENCHMARK_CANONICAL_PAYLOAD_V1", &[&canonical]);
    if canonical_hash != row.canonical_hash {
        return Err("BR-251 benchmark canonical payload hash mismatch".into());
    }
    let payload: CanonicalPayloadV1 = serde_json::from_slice(&canonical)
        .map_err(|error| format!("BR-251 decode canonical benchmark payload: {error}"))?;
    if payload.schema != PAYLOAD_SCHEMA
        || payload.instrument != row.instrument
        || payload.granularity != row.granularity
        || payload.quarter_start != row.quarter_start
        || payload.bars.len() as i64 != row.record_count
        || payload.bars.first().map(|bar| bar.at.as_str()) != Some(row.first_key.as_str())
        || payload.bars.last().map(|bar| bar.at.as_str()) != Some(row.last_key.as_str())
    {
        return Err("BR-251 benchmark payload metadata/boundary mismatch".into());
    }
    let expected_segment_hash = hash_with_domain(
        b"BR251_BENCHMARK_SEGMENT_ID_V1",
        &[
            row.instrument.as_bytes(),
            row.granularity.as_bytes(),
            row.quarter_start.as_bytes(),
            row.state.as_bytes(),
            row.canonical_hash.as_bytes(),
        ],
    );
    if expected_segment_hash != row.segment_hash {
        return Err("BR-251 benchmark segment identity mismatch".into());
    }
    let granularity = parse_granularity(&row.granularity)?;
    let mut bars = Vec::with_capacity(payload.bars.len());
    let mut previous: Option<String> = None;
    for canonical_bar in payload.bars {
        let at = match (granularity, canonical_bar.time_kind.as_str()) {
            (BenchmarkGranularity::Daily, "daily") => BenchmarkBarTime::Daily(
                NaiveDate::parse_from_str(&canonical_bar.at, "%Y-%m-%d")
                    .map_err(|error| format!("BR-251 decode daily bar time: {error}"))?,
            ),
            (BenchmarkGranularity::Minute1, "minute1") => BenchmarkBarTime::MinuteEnd(
                DateTime::<FixedOffset>::parse_from_rfc3339(&canonical_bar.at)
                    .map_err(|error| format!("BR-251 decode minute bar time: {error}"))?,
            ),
            _ => return Err("BR-251 benchmark payload mixes granularities".into()),
        };
        if previous
            .as_ref()
            .is_some_and(|previous_key| previous_key >= &canonical_bar.at)
        {
            return Err("BR-251 decoded benchmark bars are not strictly ordered".into());
        }
        previous = Some(canonical_bar.at);
        let bar = BenchmarkBar {
            at,
            open: f64::from_bits(canonical_bar.open_bits),
            high: f64::from_bits(canonical_bar.high_bits),
            low: f64::from_bits(canonical_bar.low_bits),
            close: f64::from_bits(canonical_bar.close_bits),
            volume: canonical_bar.volume_bits.map(f64::from_bits),
            amount: canonical_bar.amount_bits.map(f64::from_bits),
        };
        validate_bar(&bar)?;
        bars.push(bar);
    }
    Ok(bars)
}

fn insert_segment(
    conn: &mut SqliteConnection,
    prepared: &PreparedSegment,
    existing: Option<&PersistedSegment>,
    previous_chain_hash: &str,
) -> diesel::QueryResult<(String, String)> {
    if let Some(existing) = existing {
        return Ok((
            existing.segment_hash.clone(),
            previous_chain_hash.to_string(),
        ));
    }
    let predecessor = diesel::sql_query(
        "SELECT segment_hash AS value FROM benchmark_segment_revision
         WHERE instrument = ? AND granularity = ? AND quarter_start = ?
         ORDER BY id DESC LIMIT 1",
    )
    .bind::<Text, _>(&prepared.instrument)
    .bind::<Text, _>(&prepared.granularity)
    .bind::<Text, _>(&prepared.quarter_start)
    .get_result::<SingleTextRow>(conn)
    .optional()?
    .map(|row| row.value);
    let inserted = diesel::sql_query(
        "INSERT INTO benchmark_segment_revision (
            segment_hash, instrument, granularity, quarter_start, state, first_key,
            last_key, record_count, canonical_hash, compressed_hash, codec,
            codec_version, payload_version, compressed_payload, provider, source,
            source_at, observed_at, batch_id, acquisition_audit_id,
            acquisition_record_hash, predecessor_segment_hash
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind::<Text, _>(&prepared.segment_hash)
    .bind::<Text, _>(&prepared.instrument)
    .bind::<Text, _>(&prepared.granularity)
    .bind::<Text, _>(&prepared.quarter_start)
    .bind::<Text, _>(&prepared.state)
    .bind::<Text, _>(&prepared.first_key)
    .bind::<Text, _>(&prepared.last_key)
    .bind::<BigInt, _>(prepared.record_count)
    .bind::<Text, _>(&prepared.canonical_hash)
    .bind::<Text, _>(&prepared.compressed_hash)
    .bind::<Text, _>(CODEC)
    .bind::<Integer, _>(CODEC_VERSION)
    .bind::<Integer, _>(PAYLOAD_VERSION)
    .bind::<Binary, _>(&prepared.compressed_payload)
    .bind::<Text, _>(&prepared.provider)
    .bind::<Text, _>(&prepared.source)
    .bind::<Nullable<Text>, _>(prepared.source_at.as_deref())
    .bind::<Text, _>(&prepared.observed_at)
    .bind::<Text, _>(&prepared.batch_id)
    .bind::<BigInt, _>(prepared.acquisition_audit_id)
    .bind::<Text, _>(&prepared.acquisition_record_hash)
    .bind::<Nullable<Text>, _>(predecessor.as_deref())
    .execute(conn)?;
    if inserted != 1 {
        return Err(store_error(format!(
            "BR-251 segment append affected {inserted} rows"
        )));
    }
    let row = load_segment_by_hash(conn, &prepared.segment_hash)?
        .ok_or_else(|| store_error("BR-251 inserted segment cannot be reloaded"))?;
    let window = new_retention_window(conn)?;
    let record_hash = segment_chain_hash(
        previous_chain_hash,
        &row,
        &window.created_at,
        &window.retention_deadline,
    )?;
    let chain_inserted = diesel::sql_query(
        "INSERT INTO benchmark_segment_chain
         (segment_revision_id, previous_hash, record_hash, created_at, retention_deadline)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind::<BigInt, _>(row.id)
    .bind::<Text, _>(previous_chain_hash)
    .bind::<Text, _>(&record_hash)
    .bind::<Text, _>(&window.created_at)
    .bind::<Text, _>(&window.retention_deadline)
    .execute(conn)?;
    if chain_inserted != 1 {
        return Err(store_error(format!(
            "BR-251 segment chain append affected {chain_inserted} rows"
        )));
    }
    Ok((row.segment_hash, record_hash))
}

#[derive(Debug, QueryableByName)]
struct SingleTextRow {
    #[diesel(sql_type = Text)]
    value: String,
}

fn insert_manifest(
    conn: &mut SqliteConnection,
    manifest: &BenchmarkManifestRef,
    acquisition_bindings: &[CanonicalManifestAcquisitionBindingV1],
    previous_chain_hash: &str,
) -> diesel::QueryResult<()> {
    if let Some(existing) = load_manifest_by_hash(conn, &manifest.manifest_hash)? {
        let (_, retained_bindings) = load_manifest_acquisitions(conn, existing.id)?;
        let retained = stored_manifest_ref(&existing, &retained_bindings).map_err(store_error)?;
        if &retained != manifest || retained_bindings != acquisition_bindings {
            return Err(store_error("BR-251 manifest hash collision"));
        }
        return Ok(());
    }
    let hashes_json = serde_json::to_string(&manifest.segment_hashes)
        .map_err(|error| store_error(format!("BR-251 encode manifest hashes: {error}")))?;
    let inserted = diesel::sql_query(
        "INSERT INTO benchmark_manifest (
            manifest_hash, instrument, granularity, from_key, to_key, segment_hashes_json
         ) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind::<Text, _>(&manifest.manifest_hash)
    .bind::<Text, _>(&manifest.instrument)
    .bind::<Text, _>(granularity_name(manifest.granularity))
    .bind::<Text, _>(&manifest.from_key)
    .bind::<Text, _>(&manifest.to_key)
    .bind::<Text, _>(&hashes_json)
    .execute(conn)?;
    if inserted != 1 {
        return Err(store_error(format!(
            "BR-251 manifest append affected {inserted} rows"
        )));
    }
    let row = load_manifest_by_hash(conn, &manifest.manifest_hash)?
        .ok_or_else(|| store_error("BR-251 inserted manifest cannot be reloaded"))?;
    for (ordinal, binding) in acquisition_bindings.iter().enumerate() {
        let canonical_binding_json = serde_json::to_string(binding).map_err(|error| {
            store_error(format!(
                "BR-251 encode manifest acquisition binding: {error}"
            ))
        })?;
        let binding_hash = manifest_acquisition_binding_hash(binding).map_err(store_error)?;
        let association_inserted = diesel::sql_query(
            "INSERT INTO benchmark_manifest_acquisition (
                manifest_id, ordinal, binding_hash, canonical_binding_json
             ) VALUES (?, ?, ?, ?)",
        )
        .bind::<BigInt, _>(row.id)
        .bind::<Integer, _>(ordinal as i32)
        .bind::<Text, _>(&binding_hash)
        .bind::<Text, _>(&canonical_binding_json)
        .execute(conn)?;
        if association_inserted != 1 {
            return Err(store_error(format!(
                "BR-251 manifest acquisition append affected {association_inserted} rows"
            )));
        }
    }
    let (associations, retained_bindings) = load_manifest_acquisitions(conn, row.id)?;
    if retained_bindings != acquisition_bindings {
        return Err(store_error(
            "BR-251 inserted manifest acquisition associations cannot be reproduced",
        ));
    }
    let window = new_retention_window(conn)?;
    let record_hash = manifest_chain_hash(
        previous_chain_hash,
        &row,
        &associations,
        &window.created_at,
        &window.retention_deadline,
    )?;
    let chain_inserted = diesel::sql_query(
        "INSERT INTO benchmark_manifest_chain (
            manifest_id, previous_hash, record_hash, created_at, retention_deadline
         ) VALUES (?, ?, ?, ?, ?)",
    )
    .bind::<BigInt, _>(row.id)
    .bind::<Text, _>(previous_chain_hash)
    .bind::<Text, _>(&record_hash)
    .bind::<Text, _>(&window.created_at)
    .bind::<Text, _>(&window.retention_deadline)
    .execute(conn)?;
    if chain_inserted != 1 {
        return Err(store_error(format!(
            "BR-251 manifest chain append affected {chain_inserted} rows"
        )));
    }
    Ok(())
}

fn read_exact_on_connection(
    conn: &mut SqliteConnection,
    manifest_hash: &str,
) -> diesel::QueryResult<(BenchmarkManifestRef, Vec<BenchmarkBar>, Vec<BatchEvidence>)> {
    if !is_lower_hex_hash(manifest_hash) {
        return Err(integrity_store_error(
            "benchmark_manifest_hash_invalid",
            "BR-251 exact manifest hash must be 64 lowercase hex characters",
        ));
    }
    validate_all_state(conn)?;
    let row = load_manifest_by_hash(conn, manifest_hash)?.ok_or_else(|| {
        unavailable_store_error(
            "benchmark_manifest_unavailable",
            false,
            "BR-251 exact benchmark manifest is unavailable",
        )
    })?;
    let (_, acquisition_bindings) = load_manifest_acquisitions(conn, row.id)?;
    let manifest = stored_manifest_ref(&row, &acquisition_bindings).map_err(store_error)?;
    let request = request_from_manifest(&manifest).map_err(store_error)?;
    let expected_granularity = granularity_name(manifest.granularity);
    let mut bars = Vec::new();
    let mut retained_segments = Vec::with_capacity(manifest.segment_hashes.len());
    let mut previous_order: Option<(String, String, String)> = None;
    let mut seen = HashSet::new();
    for segment_hash in &manifest.segment_hashes {
        if !seen.insert(segment_hash) {
            return Err(store_error("BR-251 manifest repeats a segment hash"));
        }
        let segment = load_segment_by_hash(conn, segment_hash)?.ok_or_else(|| {
            store_error(format!(
                "BR-251 manifest references missing segment {segment_hash}"
            ))
        })?;
        if segment.instrument != manifest.instrument || segment.granularity != expected_granularity
        {
            return Err(store_error(
                "BR-251 manifest and segment identity/granularity conflict",
            ));
        }
        let order = (
            segment.quarter_start.clone(),
            segment.granularity.clone(),
            segment.segment_hash.clone(),
        );
        if previous_order
            .as_ref()
            .is_some_and(|previous| previous >= &order)
        {
            return Err(store_error(
                "BR-251 manifest segment order is not canonical",
            ));
        }
        previous_order = Some(order);
        bars.extend(decode_segment(&segment).map_err(store_error)?);
        retained_segments.push(segment);
    }
    let request_hash = request
        .validate_persisted_payload(&bars)
        .map_err(|error| store_error(format!("BR-251 benchmark payload validation: {error:?}")))?;
    validate_manifest_acquisition_membership(&manifest, &retained_segments, &acquisition_bindings)
        .map_err(typed_store_error)?;
    let mut evidence = Vec::with_capacity(acquisition_bindings.len());
    for binding in &acquisition_bindings {
        if binding.schema == MANIFEST_ACQUISITION_SCHEMA && binding.request_hash != request_hash {
            return Err(store_error(
                "BR-251 manifest acquisition request_hash mismatch",
            ));
        }
        verify_audit_facts(
            conn,
            &ExpectedAuditBinding {
                audit_id: binding.audit_id,
                record_hash: &binding.acquisition_record_hash,
                provider: &binding.provider,
                source: &binding.source,
                request_hash: &binding.request_hash,
                source_at: binding.source_at.as_deref(),
                observed_at: &binding.observed_at,
                batch_id: &binding.batch_id,
                accepted_count: binding.accepted_count,
                receipt_previous_outcome: None,
                receipt_current_outcome: None,
                failure_reason_code: "benchmark_manifest_acquisition_audit_invalid",
            },
        )?;
        let provider = serde_json::from_value::<ProviderId>(serde_json::Value::String(
            binding.provider.clone(),
        ))
        .map_err(|error| {
            integrity_store_error(
                "benchmark_manifest_acquisition_provider_invalid",
                format!(
                    "BR-251 persisted manifest acquisition provider is invalid at audit id {}: {error}",
                    binding.audit_id
                ),
            )
        })?;
        evidence.push(BatchEvidence {
            provider,
            source: binding.source.clone(),
            source_at: binding.source_at.clone(),
            observed_at: binding.observed_at.clone(),
            batch_id: binding.batch_id.clone(),
        });
    }
    Ok((manifest, bars, evidence))
}

impl<'a> BenchmarkSegmentStore<'a> {
    pub fn new(database: &'a DatabaseManager) -> Self {
        Self { database }
    }

    pub(crate) fn append(
        &self,
        segments: Vec<BenchmarkSegmentAppend>,
    ) -> Result<BenchmarkManifestRef, BenchmarkSegmentStoreError> {
        let Some(first) = segments.first() else {
            return Err(failed_integrity(
                "benchmark_manifest_empty",
                "BR-251 benchmark manifest requires at least one segment",
            ));
        };
        let request = first.request.clone();
        if segments.iter().any(|segment| segment.request != request) {
            return Err(failed_integrity(
                "benchmark_manifest_request_mixed",
                "BR-251 one manifest cannot mix benchmark requests",
            ));
        }
        let (granularity, from_key, to_key) = request_parts(&request)
            .map_err(|detail| failed_integrity("benchmark_manifest_request_invalid", detail))?;
        let mut prepared = segments
            .into_iter()
            .map(prepare_segment)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|detail| failed_integrity("benchmark_segment_invalid", detail))?;
        prepared.sort_by(|left, right| {
            (&left.quarter_start, &left.granularity, &left.segment_hash).cmp(&(
                &right.quarter_start,
                &right.granularity,
                &right.segment_hash,
            ))
        });
        let mut quarters = HashSet::new();
        if prepared
            .iter()
            .any(|segment| !quarters.insert(segment.quarter_start.clone()))
        {
            return Err(failed_integrity(
                "benchmark_manifest_quarter_overlap",
                "BR-251 benchmark manifest has overlapping quarter segments",
            ));
        }
        let payload = prepared
            .iter()
            .flat_map(|segment| segment.bars.iter().cloned())
            .collect::<Vec<_>>();
        let request_hash = request
            .validate_persisted_payload(&payload)
            .map_err(|error| {
                map_benchmark_error(error, "BR-251 benchmark payload validation failed")
            })?;
        let acquisition_bindings = build_manifest_acquisition_bindings(&prepared, &request_hash)
            .map_err(|detail| failed_integrity("benchmark_manifest_acquisition_invalid", detail))?;
        let accepted_by_audit = prepared.iter().fold(HashMap::new(), |mut counts, segment| {
            *counts.entry(segment.acquisition_audit_id).or_insert(0_i64) += segment.record_count;
            counts
        });

        let mut conn = self.database.get_conn().map_err(|error| {
            unavailable(
                "benchmark_segment_storage_unavailable",
                true,
                format!("BR-251 benchmark store DB connection: {error}"),
            )
        })?;
        conn.immediate_transaction::<_, diesel::result::Error, _>(|conn| {
            let body_result = (|| -> diesel::QueryResult<BenchmarkManifestRef> {
                let state = validate_all_state(conn)?;
                let mut segment_chain_tail = state.segment_chain_tail;
                let manifest_chain_tail = state.manifest_chain_tail;
                let existing_segments = prepared
                    .iter()
                    .map(|segment| load_segment_by_hash(conn, &segment.segment_hash))
                    .collect::<diesel::QueryResult<Vec<_>>>()?;
                for segment in &prepared {
                    verify_audit_facts(
                        conn,
                        &ExpectedAuditBinding {
                            audit_id: segment.acquisition_audit_id,
                            record_hash: &segment.acquisition_record_hash,
                            provider: &segment.provider,
                            source: &segment.source,
                            request_hash: &request_hash,
                            source_at: segment.source_at.as_deref(),
                            observed_at: &segment.observed_at,
                            batch_id: &segment.batch_id,
                            accepted_count: accepted_by_audit[&segment.acquisition_audit_id],
                            receipt_previous_outcome: Some(&segment.receipt_previous_outcome),
                            receipt_current_outcome: Some(&segment.receipt_current_outcome),
                            failure_reason_code: "benchmark_acquisition_audit_invalid",
                        },
                    )?;
                }
                for existing in existing_segments.iter().flatten() {
                    decode_segment(existing).map_err(store_error)?;
                    let retained_request_hash = diesel::sql_query(
                        "SELECT request_hash AS value FROM data_acquisition_audit WHERE id = ?",
                    )
                    .bind::<BigInt, _>(existing.acquisition_audit_id)
                    .get_result::<SingleTextRow>(conn)
                    .optional()?
                    .ok_or_else(|| {
                        store_error(format!(
                            "BR-159 acquisition receipt id/hash mismatch at audit id {}",
                            existing.acquisition_audit_id
                        ))
                    })?
                    .value;
                    if retained_request_hash != request_hash {
                        return Err(store_error(
                            "BR-251 existing segment acquisition request conflicts with manifest request",
                        ));
                    }
                }
                let mut segment_hashes = Vec::with_capacity(prepared.len());
                for (segment, existing) in prepared.iter().zip(&existing_segments) {
                    let (segment_hash, new_tail) = insert_segment(
                        conn,
                        segment,
                        existing.as_ref(),
                        &segment_chain_tail,
                    )?;
                    segment_chain_tail = new_tail;
                    segment_hashes.push(segment_hash);
                }
                let granularity_name = granularity_name(granularity);
                let manifest_hash = compute_manifest_hash(
                    &request.instrument,
                    granularity_name,
                    &from_key,
                    &to_key,
                    &segment_hashes,
                    &acquisition_bindings,
                )
                .map_err(store_error)?;
                let manifest = BenchmarkManifestRef {
                    manifest_hash,
                    instrument: request.instrument.clone(),
                    granularity,
                    from_key: from_key.clone(),
                    to_key: to_key.clone(),
                    segment_hashes,
                };
                insert_manifest(
                    conn,
                    &manifest,
                    &acquisition_bindings,
                    &manifest_chain_tail,
                )?;
                let (retained_manifest, retained_payload, _) =
                    read_exact_on_connection(conn, &manifest.manifest_hash)?;
                if retained_manifest != manifest || retained_payload != payload {
                    return Err(store_error(
                        "BR-251 append exact-read proof disagrees with candidate manifest/payload",
                    ));
                }
                Ok(manifest)
            })();
            body_result.map_err(|error| {
                transaction_body_error(error, "BR-251 benchmark append transaction body")
            })
        })
        .map_err(|error| {
            map_diesel_error(
                error,
                "BR-251 benchmark append transaction",
                DieselErrorContext::TransactionEnvelope,
            )
        })
    }

    /// Compose one exact manifest from caller-selected retained revisions.
    ///
    /// Each selection names both the source manifest and segment revision that
    /// preserve its original BR-159 request/receipt binding. This path never
    /// searches latest and never relabels an acquisition for the new request.
    pub fn compose_exact(
        &self,
        request: BenchmarkRequest,
        selections: Vec<BenchmarkRetainedSegmentRef>,
    ) -> Result<BenchmarkManifestRef, BenchmarkSegmentStoreError> {
        if selections.is_empty() {
            return Err(unavailable(
                "benchmark_composition_segment_unavailable",
                false,
                "BR-251 exact composition requires caller-selected retained segments",
            ));
        }
        let (granularity, from_key, to_key) = request_parts(&request)
            .map_err(|detail| failed_integrity("benchmark_composition_request_invalid", detail))?;
        if selections.iter().any(|selection| {
            !is_lower_hex_hash(&selection.source_manifest_hash)
                || !is_lower_hex_hash(&selection.segment_hash)
        }) {
            return Err(failed_integrity(
                "benchmark_composition_identity_invalid",
                "BR-251 exact composition identities must be 64 lowercase hex characters",
            ));
        }

        let mut conn = self.database.get_conn().map_err(|error| {
            unavailable(
                "benchmark_segment_storage_unavailable",
                true,
                format!("BR-251 benchmark composition DB connection: {error}"),
            )
        })?;
        conn.immediate_transaction::<_, diesel::result::Error, _>(|conn| {
            let body_result = (|| -> diesel::QueryResult<BenchmarkManifestRef> {
                let state = validate_all_state(conn)?;
                let expected_granularity = granularity_name(granularity);
                let mut segment_hashes = Vec::with_capacity(selections.len());
                let mut retained_segments = Vec::with_capacity(selections.len());
                let mut payload = Vec::new();
                let mut acquisition_bindings =
                    Vec::<CanonicalManifestAcquisitionBindingV1>::new();
                let mut binding_provenance = HashMap::<(String, String), usize>::new();
                let mut audit_provenance = HashMap::<i64, (String, String)>::new();
                let mut seen_segments = HashSet::new();
                let mut seen_quarters = HashSet::new();
                let mut previous_order: Option<(String, String, String)> = None;
                let mut provider_version: Option<(String, String, String, i32, i32)> = None;

                for selection in &selections {
                    if !seen_segments.insert(selection.segment_hash.clone()) {
                        return Err(integrity_store_error(
                            "benchmark_composition_segment_duplicate",
                            "BR-251 exact composition repeats a retained segment revision",
                        ));
                    }
                    let source_row = load_manifest_by_hash(conn, &selection.source_manifest_hash)?
                        .ok_or_else(|| {
                            unavailable_store_error(
                                "benchmark_composition_source_manifest_unavailable",
                                false,
                                "BR-251 exact composition source manifest is unavailable",
                            )
                        })?;
                    let (source_associations, source_bindings) =
                        load_manifest_acquisitions(conn, source_row.id)?;
                    let source_manifest =
                        stored_manifest_ref(&source_row, &source_bindings).map_err(store_error)?;
                    if !source_manifest
                        .segment_hashes
                        .contains(&selection.segment_hash)
                    {
                        return Err(integrity_store_error(
                            "benchmark_composition_source_member_mismatch",
                            "BR-251 selected segment is not retained by its named source manifest",
                        ));
                    }
                    let segment = load_segment_by_hash(conn, &selection.segment_hash)?
                        .ok_or_else(|| {
                            unavailable_store_error(
                                "benchmark_composition_segment_unavailable",
                                false,
                                "BR-251 selected retained segment is unavailable",
                            )
                        })?;
                    if source_manifest.instrument != segment.instrument
                        || granularity_name(source_manifest.granularity) != segment.granularity
                        || segment.instrument != request.instrument
                        || segment.granularity != expected_granularity
                    {
                        return Err(integrity_store_error(
                            "benchmark_composition_identity_mismatch",
                            "BR-251 exact composition mixes instrument or granularity identities",
                        ));
                    }
                    if !seen_quarters.insert(segment.quarter_start.clone()) {
                        return Err(integrity_store_error(
                            "benchmark_composition_quarter_overlap",
                            "BR-251 exact composition repeats or overlaps a natural quarter",
                        ));
                    }
                    let order = (
                        segment.quarter_start.clone(),
                        segment.granularity.clone(),
                        segment.segment_hash.clone(),
                    );
                    if previous_order
                        .as_ref()
                        .is_some_and(|previous| previous >= &order)
                    {
                        return Err(integrity_store_error(
                            "benchmark_composition_order_invalid",
                            "BR-251 exact composition selections are not in canonical quarter order",
                        ));
                    }
                    previous_order = Some(order);
                    let mut matching = source_bindings
                        .iter()
                        .zip(source_associations.iter())
                        .filter_map(|(binding, association)| {
                            binding
                                .members
                                .iter()
                                .find(|member| member.segment_hash == selection.segment_hash)
                                .map(|member| (binding, association, member))
                        });
                    let (source_binding, source_association, member) =
                        matching.next().ok_or_else(|| {
                            integrity_store_error(
                                "benchmark_composition_source_binding_invalid",
                                "BR-251 source manifest does not bind the selected segment to an acquisition",
                            )
                        })?;
                    if matching.next().is_some() {
                        return Err(integrity_store_error(
                            "benchmark_composition_source_binding_invalid",
                            "BR-251 source manifest binds one segment to multiple acquisitions",
                        ));
                    }
                    let version = (
                        source_binding.provider.clone(),
                        source_binding.source.clone(),
                        segment.codec.clone(),
                        segment.codec_version,
                        segment.payload_version,
                    );
                    if provider_version
                        .as_ref()
                        .is_some_and(|expected| expected != &version)
                    {
                        return Err(integrity_store_error(
                            "benchmark_composition_provider_version_mismatch",
                            "BR-251 exact composition mixes provider source revision or payload version identities",
                        ));
                    }
                    if provider_version.is_none() {
                        provider_version = Some(version);
                    }

                    let decoded = decode_segment(&segment).map_err(store_error)?;
                    payload.extend(decoded);
                    segment_hashes.push(segment.segment_hash.clone());
                    retained_segments.push(segment);

                    let provenance = (
                        selection.source_manifest_hash.clone(),
                        source_association.binding_hash.clone(),
                    );
                    if audit_provenance
                        .insert(source_binding.audit_id, provenance.clone())
                        .is_some_and(|retained| retained != provenance)
                    {
                        return Err(integrity_store_error(
                            "benchmark_composition_audit_provenance_conflict",
                            "BR-251 one acquisition audit is selected through conflicting source bindings",
                        ));
                    }
                    if let Some(index) = binding_provenance.get(&provenance).copied() {
                        acquisition_bindings[index].members.push(member.clone());
                    } else {
                        binding_provenance.insert(provenance.clone(), acquisition_bindings.len());
                        acquisition_bindings.push(CanonicalManifestAcquisitionBindingV1 {
                            schema: COMPOSED_MANIFEST_ACQUISITION_SCHEMA.into(),
                            audit_id: source_binding.audit_id,
                            acquisition_record_hash: source_binding
                                .acquisition_record_hash
                                .clone(),
                            provider: source_binding.provider.clone(),
                            source: source_binding.source.clone(),
                            request_hash: source_binding.request_hash.clone(),
                            source_at: source_binding.source_at.clone(),
                            observed_at: source_binding.observed_at.clone(),
                            batch_id: source_binding.batch_id.clone(),
                            accepted_count: source_binding.accepted_count,
                            members: vec![member.clone()],
                            source_manifest_hash: Some(provenance.0),
                            source_binding_hash: Some(provenance.1),
                        });
                    }
                }

                request
                    .validate_persisted_payload(&payload)
                    .map_err(|error| {
                        typed_store_error(map_benchmark_error(
                            error,
                            "BR-251 exact composition does not cover the requested payload",
                        ))
                    })?;
                let manifest = BenchmarkManifestRef {
                    manifest_hash: compute_manifest_hash(
                        &request.instrument,
                        expected_granularity,
                        &from_key,
                        &to_key,
                        &segment_hashes,
                        &acquisition_bindings,
                    )
                    .map_err(store_error)?,
                    instrument: request.instrument.clone(),
                    granularity,
                    from_key: from_key.clone(),
                    to_key: to_key.clone(),
                    segment_hashes,
                };
                validate_manifest_acquisition_membership(
                    &manifest,
                    &retained_segments,
                    &acquisition_bindings,
                )
                .map_err(typed_store_error)?;
                insert_manifest(
                    conn,
                    &manifest,
                    &acquisition_bindings,
                    &state.manifest_chain_tail,
                )?;
                let (retained_manifest, retained_payload, _) =
                    read_exact_on_connection(conn, &manifest.manifest_hash)?;
                if retained_manifest != manifest || retained_payload != payload {
                    return Err(integrity_store_error(
                        "benchmark_composition_exact_read_mismatch",
                        "BR-251 composed manifest exact-read proof disagrees with its selected payload",
                    ));
                }
                Ok(manifest)
            })();
            body_result.map_err(|error| {
                transaction_body_error(error, "BR-251 exact benchmark composition body")
            })
        })
        .map_err(|error| {
            map_diesel_error(
                error,
                "BR-251 exact benchmark composition",
                DieselErrorContext::TransactionEnvelope,
            )
        })
    }

    pub(crate) fn read_exact(
        &self,
        manifest_hash: &str,
    ) -> Result<
        (BenchmarkManifestRef, Vec<BenchmarkBar>, Vec<BatchEvidence>),
        BenchmarkSegmentStoreError,
    > {
        let mut conn = self.database.get_conn().map_err(|error| {
            unavailable(
                "benchmark_segment_storage_unavailable",
                true,
                format!("BR-251 benchmark reader DB connection: {error}"),
            )
        })?;
        conn.transaction::<_, diesel::result::Error, _>(|conn| {
            read_exact_on_connection(conn, manifest_hash).map_err(|error| {
                transaction_body_error(error, "BR-251 exact benchmark read transaction body")
            })
        })
        .map_err(|error| {
            map_diesel_error(
                error,
                "BR-251 exact benchmark read",
                DieselErrorContext::TransactionEnvelope,
            )
        })
    }
}

pub(super) fn create_schema(conn: &mut SqliteConnection) -> diesel::QueryResult<()> {
    let existing_table_count = diesel::sql_query(
        "SELECT COUNT(*) AS count FROM sqlite_master
         WHERE type = 'table' AND name IN (
            'benchmark_segment_revision', 'benchmark_segment_chain',
            'benchmark_manifest', 'benchmark_manifest_acquisition',
            'benchmark_manifest_chain'
         )",
    )
    .get_result::<CountValueRow>(conn)?
    .count;
    if existing_table_count != 0 {
        validate_immutable_triggers(conn)?;
    }
    diesel::sql_query(
        "CREATE TABLE IF NOT EXISTS benchmark_segment_revision (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            segment_hash TEXT NOT NULL UNIQUE CHECK(length(segment_hash) = 64),
            instrument TEXT NOT NULL,
            granularity TEXT NOT NULL CHECK(granularity IN ('Daily', 'Minute1')),
            quarter_start TEXT NOT NULL,
            state TEXT NOT NULL CHECK(state IN ('Provisional', 'Sealed')),
            first_key TEXT NOT NULL,
            last_key TEXT NOT NULL,
            record_count INTEGER NOT NULL CHECK(record_count > 0),
            canonical_hash TEXT NOT NULL CHECK(length(canonical_hash) = 64),
            compressed_hash TEXT NOT NULL CHECK(length(compressed_hash) = 64),
            codec TEXT NOT NULL CHECK(codec = 'zstd'),
            codec_version INTEGER NOT NULL CHECK(codec_version = 1),
            payload_version INTEGER NOT NULL CHECK(payload_version = 1),
            compressed_payload BLOB NOT NULL CHECK(length(compressed_payload) > 0),
            provider TEXT NOT NULL,
            source TEXT NOT NULL,
            source_at TEXT,
            observed_at TEXT NOT NULL,
            batch_id TEXT NOT NULL,
            acquisition_audit_id INTEGER NOT NULL,
            acquisition_record_hash TEXT NOT NULL CHECK(length(acquisition_record_hash) = 64),
            predecessor_segment_hash TEXT,
            created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
            retention_deadline TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '+5 years')),
            UNIQUE(instrument, granularity, quarter_start, state, canonical_hash),
            FOREIGN KEY(acquisition_audit_id) REFERENCES data_acquisition_audit(id),
            FOREIGN KEY(predecessor_segment_hash) REFERENCES benchmark_segment_revision(segment_hash)
        )",
    )
    .execute(conn)?;
    immutable_triggers(
        conn,
        IMMUTABLE_TABLES[0].0,
        IMMUTABLE_TABLES[0].1,
        IMMUTABLE_TABLES[0].2,
    )?;
    diesel::sql_query(
        "CREATE TABLE IF NOT EXISTS benchmark_segment_chain (
            segment_revision_id INTEGER PRIMARY KEY NOT NULL,
            previous_hash TEXT NOT NULL,
            record_hash TEXT NOT NULL UNIQUE,
            created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
            retention_deadline TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '+5 years')),
            FOREIGN KEY(segment_revision_id) REFERENCES benchmark_segment_revision(id)
        )",
    )
    .execute(conn)?;
    immutable_triggers(
        conn,
        IMMUTABLE_TABLES[1].0,
        IMMUTABLE_TABLES[1].1,
        IMMUTABLE_TABLES[1].2,
    )?;
    diesel::sql_query(
        "CREATE TABLE IF NOT EXISTS benchmark_manifest (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            manifest_hash TEXT NOT NULL UNIQUE CHECK(length(manifest_hash) = 64),
            instrument TEXT NOT NULL,
            granularity TEXT NOT NULL CHECK(granularity IN ('Daily', 'Minute1')),
            from_key TEXT NOT NULL,
            to_key TEXT NOT NULL,
            segment_hashes_json TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
            retention_deadline TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '+5 years'))
        )",
    )
    .execute(conn)?;
    immutable_triggers(
        conn,
        IMMUTABLE_TABLES[2].0,
        IMMUTABLE_TABLES[2].1,
        IMMUTABLE_TABLES[2].2,
    )?;
    diesel::sql_query(
        "CREATE TABLE IF NOT EXISTS benchmark_manifest_acquisition (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            manifest_id INTEGER NOT NULL,
            ordinal INTEGER NOT NULL CHECK(ordinal >= 0),
            binding_hash TEXT NOT NULL CHECK(length(binding_hash) = 64),
            canonical_binding_json TEXT NOT NULL CHECK(length(canonical_binding_json) > 0),
            created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
            retention_deadline TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '+5 years')),
            UNIQUE(manifest_id, ordinal),
            UNIQUE(manifest_id, binding_hash),
            FOREIGN KEY(manifest_id) REFERENCES benchmark_manifest(id)
        )",
    )
    .execute(conn)?;
    immutable_triggers(
        conn,
        IMMUTABLE_TABLES[3].0,
        IMMUTABLE_TABLES[3].1,
        IMMUTABLE_TABLES[3].2,
    )?;
    diesel::sql_query(
        "CREATE TABLE IF NOT EXISTS benchmark_manifest_chain (
            manifest_id INTEGER PRIMARY KEY NOT NULL,
            previous_hash TEXT NOT NULL,
            record_hash TEXT NOT NULL UNIQUE,
            created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
            retention_deadline TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '+5 years')),
            FOREIGN KEY(manifest_id) REFERENCES benchmark_manifest(id)
        )",
    )
    .execute(conn)?;
    immutable_triggers(
        conn,
        IMMUTABLE_TABLES[4].0,
        IMMUTABLE_TABLES[4].1,
        IMMUTABLE_TABLES[4].2,
    )?;
    validate_all_state(conn)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use chrono::{DateTime, NaiveDate};
    use diesel::sql_types::{BigInt, Binary, Nullable, Text};

    use super::*;
    use crate::data_gateway::review::AuditedBenchmarkBatch;
    use crate::data_gateway::{
        BenchmarkBarTime, BenchmarkCapture, BenchmarkRange, BenchmarkReader, GatewayBatch,
        HS300_CANONICAL,
    };
    use crate::database::data_acquisition_audit::DataAcquisitionAuditRecord;
    use crate::market_domain::ProviderId;

    #[derive(QueryableByName)]
    struct CountRow {
        #[diesel(sql_type = BigInt)]
        count: i64,
    }

    #[derive(QueryableByName)]
    struct TextRow {
        #[diesel(sql_type = Text)]
        value: String,
    }

    #[derive(QueryableByName)]
    struct OptionalTextRow {
        #[diesel(sql_type = Nullable<Text>)]
        value: Option<String>,
    }

    #[derive(QueryableByName)]
    struct BlobRow {
        #[diesel(sql_type = Binary)]
        value: Vec<u8>,
    }

    #[derive(QueryableByName)]
    struct TimestampRow {
        #[diesel(sql_type = Text)]
        created_at: String,
        #[diesel(sql_type = Text)]
        retention_deadline: String,
    }

    fn test_database_error(
        kind: diesel::result::DatabaseErrorKind,
        label: &str,
    ) -> diesel::result::Error {
        diesel::result::Error::DatabaseError(kind, Box::new(label.to_string()))
    }

    fn assert_failed_integrity(
        error: BenchmarkSegmentStoreError,
        expected_reason_code: &'static str,
    ) -> String {
        match error {
            BenchmarkSegmentStoreError::FailedIntegrity {
                reason_code,
                detail,
            } => {
                assert_eq!(reason_code, expected_reason_code);
                assert!(!detail.is_empty());
                detail
            }
            other => panic!("TEST_CODE expected FailedIntegrity, got {other:?}"),
        }
    }

    fn assert_unavailable(
        error: BenchmarkSegmentStoreError,
        expected_reason_code: &'static str,
        expected_retryable: bool,
    ) -> String {
        match error {
            BenchmarkSegmentStoreError::BenchmarkSegmentUnavailable {
                reason_code,
                retryable,
                detail,
            } => {
                assert_eq!(reason_code, expected_reason_code);
                assert_eq!(retryable, expected_retryable);
                assert!(!detail.is_empty());
                detail
            }
            other => panic!("TEST_CODE expected BenchmarkSegmentUnavailable, got {other:?}"),
        }
    }

    struct TestDatabase {
        path: PathBuf,
        manager: DatabaseManager,
    }

    impl TestDatabase {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("TEST_CODE clock")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "TEST_CODE_benchmark_segments_{}_{}.sqlite",
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
            let pool = super::super::build_sqlite_pool_with_size(database_url, 1)
                .expect("TEST_CODE SQLite pool");
            {
                let mut conn = pool.get().expect("TEST_CODE schema connection");
                super::super::data_acquisition_audit::create_schema(&mut conn)
                    .expect("TEST_CODE BR-159 schema");
                create_schema(&mut conn).expect("TEST_CODE benchmark schema");
            }
            Self {
                path,
                manager: DatabaseManager {
                    pool,
                    attribution_pool: None,
                    attribution_connection_source: None,
                    readonly_attribution_snapshot: None,
                    allow_unattested_attribution_reads_for_test: true,
                    selection_connection_source: None,
                    selection_schema_authority: None,
                },
            }
        }

        fn conn(&self) -> super::super::DbConnection {
            self.manager.get_conn().expect("TEST_CODE connection")
        }

        fn store(&self) -> BenchmarkSegmentStore<'_> {
            BenchmarkSegmentStore::new(&self.manager)
        }

        fn receipt(
            &self,
            request: &BenchmarkRequest,
            evidence: &BatchEvidence,
            accepted_count: i64,
        ) -> DataAcquisitionAuditReceipt {
            self.receipt_with_outcome(
                request,
                evidence,
                "available",
                accepted_count,
                "accepted",
                false,
            )
        }

        fn receipt_with_outcome(
            &self,
            request: &BenchmarkRequest,
            evidence: &BatchEvidence,
            outcome: &str,
            accepted_count: i64,
            reason_code: &str,
            retryable: bool,
        ) -> DataAcquisitionAuditReceipt {
            let request_hash = request.canonical_request_hash();
            let provider = serde_json::to_value(evidence.provider)
                .expect("TEST_CODE serialize provider")
                .as_str()
                .expect("TEST_CODE provider is a unit-variant string")
                .to_owned();
            self.manager
                .record_data_acquisition(&DataAcquisitionAuditRecord {
                    capability: "BenchmarkBars",
                    provider: &provider,
                    source: &evidence.source,
                    request_hash: &request_hash,
                    source_at: evidence.source_at.as_deref(),
                    observed_at: &evidence.observed_at,
                    batch_id: Some(&evidence.batch_id),
                    outcome,
                    request_count: 1,
                    accepted_count,
                    rejected_count: 0,
                    reason_code,
                    retryable,
                })
                .expect("TEST_CODE real BR-159 receipt")
        }
    }

    impl Drop for TestDatabase {
        fn drop(&mut self) {
            for suffix in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{suffix}", self.path.to_string_lossy()));
            }
        }
    }

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).expect("TEST_CODE date")
    }

    fn evidence(batch_id: &str) -> BatchEvidence {
        BatchEvidence {
            provider: ProviderId::Tdx,
            source: "TEST_CODE_magic_tdx_index".into(),
            source_at: Some("2026-01-05T15:00:00+08:00".into()),
            observed_at: "2026-01-05T15:00:01+08:00".into(),
            batch_id: batch_id.into(),
        }
    }

    fn evidence_observed_at(batch_id: &str, observed_at: &str) -> BatchEvidence {
        BatchEvidence {
            observed_at: observed_at.into(),
            ..evidence(batch_id)
        }
    }

    fn daily_request() -> BenchmarkRequest {
        BenchmarkRequest {
            instrument: "TEST_CODE_sh000300".into(),
            range: BenchmarkRange::Daily {
                from: date(2026, 1, 5),
                to: date(2026, 1, 5),
            },
        }
    }

    fn daily_bar(at: NaiveDate, close: f64) -> BenchmarkBar {
        BenchmarkBar {
            at: BenchmarkBarTime::Daily(at),
            open: close - 1.0,
            high: close + 1.0,
            low: close - 2.0,
            close,
            volume: Some(1000.25),
            amount: None,
        }
    }

    fn minute_bar(at: DateTime<FixedOffset>, close: f64) -> BenchmarkBar {
        BenchmarkBar {
            at: BenchmarkBarTime::MinuteEnd(at),
            open: close - 1.0,
            high: close + 1.0,
            low: close - 2.0,
            close,
            volume: Some(1000.25),
            amount: None,
        }
    }

    fn q1_2026_dates() -> Vec<NaiveDate> {
        "2026-01-05 2026-01-06 2026-01-07 2026-01-08 2026-01-09 \
         2026-01-12 2026-01-13 2026-01-14 2026-01-15 2026-01-16 \
         2026-01-19 2026-01-20 2026-01-21 2026-01-22 2026-01-23 \
         2026-01-26 2026-01-27 2026-01-28 2026-01-29 2026-01-30 \
         2026-02-02 2026-02-03 2026-02-04 2026-02-05 2026-02-06 \
         2026-02-09 2026-02-10 2026-02-11 2026-02-12 2026-02-13 \
         2026-02-24 2026-02-25 2026-02-26 2026-02-27 \
         2026-03-02 2026-03-03 2026-03-04 2026-03-05 2026-03-06 \
         2026-03-09 2026-03-10 2026-03-11 2026-03-12 2026-03-13 \
         2026-03-16 2026-03-17 2026-03-18 2026-03-19 2026-03-20 \
         2026-03-23 2026-03-24 2026-03-25 2026-03-26 2026-03-27 \
         2026-03-30 2026-03-31"
            .split_whitespace()
            .map(|value| {
                NaiveDate::parse_from_str(value, "%Y-%m-%d").expect("TEST_CODE Q1 trading day")
            })
            .collect()
    }

    fn q1_2026_request() -> BenchmarkRequest {
        BenchmarkRequest {
            instrument: "TEST_CODE_sh000300".into(),
            range: BenchmarkRange::Daily {
                from: date(2026, 1, 1),
                to: date(2026, 3, 31),
            },
        }
    }

    fn q1_2026_bars(last_close: f64) -> Vec<BenchmarkBar> {
        let mut bars = q1_2026_dates()
            .into_iter()
            .map(|day| daily_bar(day, 101.0))
            .collect::<Vec<_>>();
        bars.last_mut().expect("TEST_CODE Q1 last bar").close = last_close;
        bars.last_mut().expect("TEST_CODE Q1 last bar").high = last_close + 1.0;
        bars
    }

    fn append_for(
        database: &TestDatabase,
        request: BenchmarkRequest,
        quarter_start: NaiveDate,
        state: SegmentState,
        bars: Vec<BenchmarkBar>,
        batch_id: &str,
    ) -> BenchmarkSegmentAppend {
        let evidence = evidence(batch_id);
        let receipt = database.receipt(&request, &evidence, bars.len() as i64);
        BenchmarkSegmentAppend {
            request,
            quarter_start,
            state,
            bars,
            evidence,
            acquisition_receipt: receipt,
        }
    }

    fn append_with_evidence(
        database: &TestDatabase,
        request: BenchmarkRequest,
        quarter_start: NaiveDate,
        state: SegmentState,
        bars: Vec<BenchmarkBar>,
        evidence: BatchEvidence,
    ) -> BenchmarkSegmentAppend {
        let receipt = database.receipt(&request, &evidence, bars.len() as i64);
        BenchmarkSegmentAppend {
            request,
            quarter_start,
            state,
            bars,
            evidence,
            acquisition_receipt: receipt,
        }
    }

    fn append_readable(
        database: &TestDatabase,
        segments: Vec<BenchmarkSegmentAppend>,
        context: &str,
    ) -> BenchmarkManifestRef {
        let manifest = database
            .store()
            .append(segments)
            .unwrap_or_else(|error| panic!("{context}: append failed: {error}"));
        let (retained, _, _) = database
            .store()
            .read_exact(&manifest.manifest_hash)
            .unwrap_or_else(|error| panic!("{context}: immediate exact read failed: {error}"));
        assert_eq!(retained, manifest, "{context}: manifest changed on read");
        manifest
    }

    fn shared_cross_quarter_manifest(
        database: &TestDatabase,
        batch_id: &str,
    ) -> BenchmarkManifestRef {
        let request = BenchmarkRequest {
            instrument: "TEST_CODE_sh000300".into(),
            range: BenchmarkRange::Daily {
                from: date(2026, 3, 31),
                to: date(2026, 4, 1),
            },
        };
        let evidence = evidence(batch_id);
        let receipt = database.receipt(&request, &evidence, 2);
        append_readable(
            database,
            vec![
                BenchmarkSegmentAppend {
                    request: request.clone(),
                    quarter_start: date(2026, 1, 1),
                    state: SegmentState::Provisional,
                    bars: vec![daily_bar(date(2026, 3, 31), 101.0)],
                    evidence: evidence.clone(),
                    acquisition_receipt: receipt.clone(),
                },
                BenchmarkSegmentAppend {
                    request,
                    quarter_start: date(2026, 4, 1),
                    state: SegmentState::Provisional,
                    bars: vec![daily_bar(date(2026, 4, 1), 102.0)],
                    evidence,
                    acquisition_receipt: receipt,
                },
            ],
            "TEST_CODE shared cross-quarter manifest",
        )
    }

    fn rewrite_only_manifest_binding(
        database: &TestDatabase,
        manifest: &BenchmarkManifestRef,
        mutate: impl FnOnce(&mut CanonicalManifestAcquisitionBindingV1),
    ) {
        let mut conn = database.conn();
        let mut binding: CanonicalManifestAcquisitionBindingV1 = diesel::sql_query(
            "SELECT canonical_binding_json AS value
             FROM benchmark_manifest_acquisition
             WHERE manifest_id = (SELECT id FROM benchmark_manifest WHERE manifest_hash = ?)
               AND ordinal = 0",
        )
        .bind::<Text, _>(&manifest.manifest_hash)
        .get_result::<TextRow>(&mut conn)
        .map(|row| serde_json::from_str(&row.value).expect("TEST_CODE decode binding"))
        .expect("TEST_CODE load binding");
        mutate(&mut binding);
        let canonical = serde_json::to_string(&binding).expect("TEST_CODE canonical binding");
        let binding_hash =
            manifest_acquisition_binding_hash(&binding).expect("TEST_CODE binding hash");
        diesel::sql_query("DROP TRIGGER trg_benchmark_manifest_acquisition_no_update")
            .execute(&mut conn)
            .expect("TEST_CODE drop association update trigger");
        diesel::sql_query(
            "UPDATE benchmark_manifest_acquisition
             SET canonical_binding_json = ?, binding_hash = ?
             WHERE manifest_id = (SELECT id FROM benchmark_manifest WHERE manifest_hash = ?)
               AND ordinal = 0",
        )
        .bind::<Text, _>(&canonical)
        .bind::<Text, _>(&binding_hash)
        .bind::<Text, _>(&manifest.manifest_hash)
        .execute(&mut conn)
        .expect("TEST_CODE rewrite association binding");
        diesel::sql_query(canonical_update_trigger_definition(
            "benchmark_manifest_acquisition",
        ))
        .execute(&mut conn)
        .expect("TEST_CODE restore association update trigger");
    }

    fn count(conn: &mut SqliteConnection, table: &str) -> i64 {
        diesel::sql_query(format!("SELECT COUNT(*) AS count FROM {table}"))
            .get_result::<CountRow>(conn)
            .expect("TEST_CODE count")
            .count
    }

    fn database_with_one_manifest() -> (TestDatabase, BenchmarkManifestRef) {
        let database = TestDatabase::new();
        let manifest = append_readable(
            &database,
            vec![append_for(
                &database,
                daily_request(),
                date(2026, 1, 1),
                SegmentState::Provisional,
                vec![daily_bar(date(2026, 1, 5), 101.0)],
                "TEST_CODE_integrity_seed",
            )],
            "TEST_CODE benchmark integrity seed",
        );
        (database, manifest)
    }

    fn database_with_two_manifests() -> (TestDatabase, BenchmarkManifestRef) {
        let (database, retained) = database_with_one_manifest();
        let request = BenchmarkRequest {
            instrument: "TEST_CODE_sh000300".into(),
            range: BenchmarkRange::Daily {
                from: date(2026, 1, 6),
                to: date(2026, 1, 6),
            },
        };
        append_readable(
            &database,
            vec![append_for(
                &database,
                request,
                date(2026, 1, 1),
                SegmentState::Provisional,
                vec![daily_bar(date(2026, 1, 6), 102.0)],
                "TEST_CODE_integrity_tail",
            )],
            "TEST_CODE benchmark integrity tail",
        );
        (database, retained)
    }

    fn independently_retained_quarters() -> (
        TestDatabase,
        BenchmarkRequest,
        BenchmarkManifestRef,
        BenchmarkManifestRef,
    ) {
        let database = TestDatabase::new();
        let q1_request = BenchmarkRequest {
            instrument: "TEST_CODE_sh000300".into(),
            range: BenchmarkRange::Daily {
                from: date(2026, 3, 31),
                to: date(2026, 3, 31),
            },
        };
        let q2_request = BenchmarkRequest {
            instrument: q1_request.instrument.clone(),
            range: BenchmarkRange::Daily {
                from: date(2026, 4, 1),
                to: date(2026, 4, 1),
            },
        };
        let q1 = append_readable(
            &database,
            vec![append_for(
                &database,
                q1_request.clone(),
                date(2026, 1, 1),
                SegmentState::Provisional,
                vec![daily_bar(date(2026, 3, 31), 101.0)],
                "TEST_CODE_exact_fixture_q1",
            )],
            "TEST_CODE exact fixture Q1",
        );
        let q2 = append_readable(
            &database,
            vec![append_for(
                &database,
                q2_request,
                date(2026, 4, 1),
                SegmentState::Provisional,
                vec![daily_bar(date(2026, 4, 1), 102.0)],
                "TEST_CODE_exact_fixture_q2",
            )],
            "TEST_CODE exact fixture Q2",
        );
        let request = BenchmarkRequest {
            instrument: q1_request.instrument,
            range: BenchmarkRange::Daily {
                from: date(2026, 3, 31),
                to: date(2026, 4, 1),
            },
        };
        (database, request, q1, q2)
    }

    fn retained_segment(
        manifest: &BenchmarkManifestRef,
        index: usize,
    ) -> BenchmarkRetainedSegmentRef {
        BenchmarkRetainedSegmentRef {
            source_manifest_hash: manifest.manifest_hash.clone(),
            segment_hash: manifest.segment_hashes[index].clone(),
        }
    }

    fn replace_benchmark_trigger_with_noop(
        database: &TestDatabase,
        name: &str,
        table: &str,
        event: &str,
    ) {
        let mut conn = database.conn();
        diesel::sql_query(format!("DROP TRIGGER {name}"))
            .execute(&mut conn)
            .expect("TEST_CODE drop canonical benchmark trigger");
        diesel::sql_query(format!(
            "CREATE TRIGGER {name} BEFORE {event} ON {table} BEGIN SELECT 1; END"
        ))
        .execute(&mut conn)
        .expect("TEST_CODE install same-name no-op benchmark trigger");
    }

    fn next_integrity_append(database: &TestDatabase) -> BenchmarkSegmentAppend {
        let request = BenchmarkRequest {
            instrument: "TEST_CODE_sh000300".into(),
            range: BenchmarkRange::Daily {
                from: date(2026, 1, 7),
                to: date(2026, 1, 7),
            },
        };
        append_for(
            database,
            request,
            date(2026, 1, 1),
            SegmentState::Provisional,
            vec![daily_bar(date(2026, 1, 7), 103.0)],
            "TEST_CODE_integrity_preappend",
        )
    }

    fn delete_manifest_tail_and_restore_triggers(database: &TestDatabase) {
        let mut conn = database.conn();
        for trigger in [
            "trg_benchmark_manifest_chain_no_delete",
            "trg_benchmark_manifest_acquisition_no_delete",
            "trg_benchmark_manifest_no_delete",
        ] {
            diesel::sql_query(format!("DROP TRIGGER {trigger}"))
                .execute(&mut conn)
                .expect("TEST_CODE drop benchmark tail delete trigger");
        }
        diesel::sql_query("DELETE FROM benchmark_manifest_chain WHERE manifest_id = 2")
            .execute(&mut conn)
            .expect("TEST_CODE delete manifest-chain tail");
        diesel::sql_query("DELETE FROM benchmark_manifest_acquisition WHERE manifest_id = 2")
            .execute(&mut conn)
            .expect("TEST_CODE delete manifest-association tail");
        diesel::sql_query("DELETE FROM benchmark_manifest WHERE id = 2")
            .execute(&mut conn)
            .expect("TEST_CODE delete manifest tail");
        for definition in [
            "CREATE TRIGGER trg_benchmark_manifest_no_delete
             BEFORE DELETE ON benchmark_manifest
             BEGIN SELECT RAISE(ABORT, 'BR-251 benchmark manifest retention is at least five years'); END",
            "CREATE TRIGGER trg_benchmark_manifest_acquisition_no_delete
             BEFORE DELETE ON benchmark_manifest_acquisition
             BEGIN SELECT RAISE(ABORT, 'BR-251 benchmark manifest acquisition retention is at least five years'); END",
            "CREATE TRIGGER trg_benchmark_manifest_chain_no_delete
             BEFORE DELETE ON benchmark_manifest_chain
             BEGIN SELECT RAISE(ABORT, 'BR-251 benchmark manifest chain retention is at least five years'); END",
        ] {
            diesel::sql_query(definition)
                .execute(&mut conn)
                .expect("TEST_CODE restore canonical benchmark delete trigger");
        }
    }

    fn canonical_update_trigger_definition(table: &str) -> String {
        let action = match table {
            "benchmark_segment_revision" => "BR-251 benchmark segment revision is immutable",
            "benchmark_segment_chain" => "BR-251 benchmark segment chain is immutable",
            "benchmark_manifest" => "BR-251 benchmark manifest is immutable",
            "benchmark_manifest_acquisition" => {
                "BR-251 benchmark manifest acquisition is immutable"
            }
            "benchmark_manifest_chain" => "BR-251 benchmark manifest chain is immutable",
            other => panic!("TEST_CODE unknown benchmark table {other}"),
        };
        format!(
            "CREATE TRIGGER trg_{table}_no_update BEFORE UPDATE ON {table} \
             BEGIN SELECT RAISE(ABORT, '{action}'); END"
        )
    }

    fn tamper_retention(
        database: &TestDatabase,
        table: &str,
        created_at: &str,
        retention_deadline: &str,
    ) {
        let mut conn = database.conn();
        diesel::sql_query(format!("DROP TRIGGER trg_{table}_no_update"))
            .execute(&mut conn)
            .expect("TEST_CODE drop retention update trigger");
        diesel::sql_query(format!(
            "UPDATE {table} SET created_at = ?, retention_deadline = ?"
        ))
        .bind::<Text, _>(created_at)
        .bind::<Text, _>(retention_deadline)
        .execute(&mut conn)
        .expect("TEST_CODE tamper retained benchmark timestamps");
        diesel::sql_query(canonical_update_trigger_definition(table))
            .execute(&mut conn)
            .expect("TEST_CODE restore canonical retention update trigger");
    }

    #[test]
    fn migration_installs_four_immutable_tables_and_startup_rejects_missing_chain() {
        let database = TestDatabase::new();
        let append = append_for(
            &database,
            daily_request(),
            date(2026, 1, 1),
            SegmentState::Provisional,
            vec![daily_bar(date(2026, 1, 5), 101.0)],
            "TEST_CODE_batch_migration",
        );
        append_readable(&database, vec![append], "TEST_CODE migration append");

        let mut conn = database.conn();
        for table in [
            "benchmark_segment_revision",
            "benchmark_segment_chain",
            "benchmark_manifest",
            "benchmark_manifest_chain",
        ] {
            assert_eq!(count(&mut conn, table), 1, "{table}");
            diesel::sql_query(format!("UPDATE {table} SET created_at = created_at"))
                .execute(&mut conn)
                .expect_err("TEST_CODE UPDATE must be rejected");
            diesel::sql_query(format!("DELETE FROM {table}"))
                .execute(&mut conn)
                .expect_err("TEST_CODE DELETE must be rejected");
        }
        assert_eq!(count(&mut conn, "benchmark_manifest_acquisition"), 1);
        diesel::sql_query("UPDATE benchmark_manifest_acquisition SET created_at = created_at")
            .execute(&mut conn)
            .expect_err("TEST_CODE association UPDATE must be rejected");
        diesel::sql_query("DELETE FROM benchmark_manifest_acquisition")
            .execute(&mut conn)
            .expect_err("TEST_CODE association DELETE must be rejected");

        diesel::sql_query("DROP TRIGGER trg_benchmark_segment_chain_no_delete")
            .execute(&mut conn)
            .expect("TEST_CODE drop immutable trigger");
        diesel::sql_query("DELETE FROM benchmark_segment_chain")
            .execute(&mut conn)
            .expect("TEST_CODE remove chain row");
        create_schema(&mut conn).expect_err("TEST_CODE startup must reject missing chain");
    }

    #[test]
    fn startup_read_and_preappend_reject_every_same_name_noop_immutable_trigger() {
        let triggers = [
            (
                "trg_benchmark_segment_revision_no_update",
                "benchmark_segment_revision",
                "UPDATE",
            ),
            (
                "trg_benchmark_segment_revision_no_delete",
                "benchmark_segment_revision",
                "DELETE",
            ),
            (
                "trg_benchmark_segment_chain_no_update",
                "benchmark_segment_chain",
                "UPDATE",
            ),
            (
                "trg_benchmark_segment_chain_no_delete",
                "benchmark_segment_chain",
                "DELETE",
            ),
            (
                "trg_benchmark_manifest_no_update",
                "benchmark_manifest",
                "UPDATE",
            ),
            (
                "trg_benchmark_manifest_no_delete",
                "benchmark_manifest",
                "DELETE",
            ),
            (
                "trg_benchmark_manifest_acquisition_no_update",
                "benchmark_manifest_acquisition",
                "UPDATE",
            ),
            (
                "trg_benchmark_manifest_acquisition_no_delete",
                "benchmark_manifest_acquisition",
                "DELETE",
            ),
            (
                "trg_benchmark_manifest_chain_no_update",
                "benchmark_manifest_chain",
                "UPDATE",
            ),
            (
                "trg_benchmark_manifest_chain_no_delete",
                "benchmark_manifest_chain",
                "DELETE",
            ),
        ];

        for (name, table, event) in triggers {
            let (startup, _) = database_with_one_manifest();
            replace_benchmark_trigger_with_noop(&startup, name, table, event);
            let startup_error = create_schema(&mut startup.conn())
                .expect_err("TEST_CODE startup must reject same-name no-op trigger");
            assert_eq!(
                map_diesel_error(
                    startup_error,
                    "TEST_CODE benchmark startup trigger validation",
                    DieselErrorContext::TransactionEnvelope,
                )
                .reason_code(),
                "benchmark_trigger_definition_invalid",
                "TEST_CODE startup accepted {name}",
            );

            let (reader, manifest) = database_with_one_manifest();
            replace_benchmark_trigger_with_noop(&reader, name, table, event);
            assert_eq!(
                reader
                    .store()
                    .read_exact(&manifest.manifest_hash)
                    .expect_err("TEST_CODE exact read must reject same-name no-op trigger")
                    .reason_code(),
                "benchmark_trigger_definition_invalid",
                "TEST_CODE reader accepted {name}",
            );

            let (preappend, _) = database_with_one_manifest();
            replace_benchmark_trigger_with_noop(&preappend, name, table, event);
            assert_eq!(
                preappend
                    .store()
                    .append(vec![next_integrity_append(&preappend)])
                    .expect_err("TEST_CODE preappend must reject same-name no-op trigger")
                    .reason_code(),
                "benchmark_trigger_definition_invalid",
                "TEST_CODE preappend accepted {name}",
            );
        }
    }

    #[test]
    fn startup_read_and_preappend_reject_coordinated_manifest_tail_deletion() {
        let (startup, _) = database_with_two_manifests();
        delete_manifest_tail_and_restore_triggers(&startup);
        let startup_error = create_schema(&mut startup.conn())
            .expect_err("TEST_CODE startup must reject coordinated benchmark tail deletion");
        assert_eq!(
            map_diesel_error(
                startup_error,
                "TEST_CODE benchmark startup high-water validation",
                DieselErrorContext::TransactionEnvelope,
            )
            .reason_code(),
            "benchmark_sequence_highwater_invalid",
        );

        let (reader, retained) = database_with_two_manifests();
        delete_manifest_tail_and_restore_triggers(&reader);
        assert_eq!(
            reader
                .store()
                .read_exact(&retained.manifest_hash)
                .expect_err("TEST_CODE read must reject coordinated benchmark tail deletion")
                .reason_code(),
            "benchmark_sequence_highwater_invalid",
        );

        let (preappend, _) = database_with_two_manifests();
        delete_manifest_tail_and_restore_triggers(&preappend);
        assert_eq!(
            preappend
                .store()
                .append(vec![next_integrity_append(&preappend)])
                .expect_err("TEST_CODE preappend must reject coordinated benchmark tail deletion")
                .reason_code(),
            "benchmark_sequence_highwater_invalid",
        );
    }

    #[test]
    fn receipt_mismatch_fails_before_any_segment_or_manifest_write() {
        let database = TestDatabase::new();
        let mut append = append_for(
            &database,
            daily_request(),
            date(2026, 1, 1),
            SegmentState::Provisional,
            vec![daily_bar(date(2026, 1, 5), 101.0)],
            "TEST_CODE_batch_receipt",
        );
        append.acquisition_receipt.record_hash = "0".repeat(64);
        let error = database
            .store()
            .append(vec![append])
            .expect_err("TEST_CODE forged receipt must fail");
        assert!(error.contains("BR-159"), "{error}");
        let mut conn = database.conn();
        assert_eq!(count(&mut conn, "benchmark_segment_revision"), 0);
        assert_eq!(count(&mut conn, "benchmark_segment_chain"), 0);
        assert_eq!(count(&mut conn, "benchmark_manifest"), 0);
        assert_eq!(count(&mut conn, "benchmark_manifest_chain"), 0);
    }

    #[test]
    fn existing_segment_reuse_rejects_a_different_manifest_acquisition_request() {
        let database = TestDatabase::new();
        append_readable(
            &database,
            vec![append_for(
                &database,
                daily_request(),
                date(2026, 1, 1),
                SegmentState::Provisional,
                vec![daily_bar(date(2026, 1, 5), 101.0)],
                "TEST_CODE_batch_reuse_original_request",
            )],
            "TEST_CODE original request",
        );

        let expanded = BenchmarkRequest {
            instrument: "TEST_CODE_sh000300".into(),
            range: BenchmarkRange::Daily {
                from: date(2026, 1, 3),
                to: date(2026, 1, 5),
            },
        };
        let error = database
            .store()
            .append(vec![append_for(
                &database,
                expanded,
                date(2026, 1, 1),
                SegmentState::Provisional,
                vec![daily_bar(date(2026, 1, 5), 101.0)],
                "TEST_CODE_batch_reuse_weekend_expanded_request",
            )])
            .expect_err("TEST_CODE incompatible segment reuse must fail before manifest insert");
        assert!(
            error.contains(
                "BR-251 existing segment acquisition request conflicts with manifest request"
            ),
            "{error}"
        );
        let mut conn = database.conn();
        assert_eq!(count(&mut conn, "benchmark_manifest"), 1);
        assert_eq!(count(&mut conn, "benchmark_manifest_chain"), 1);
    }

    #[test]
    fn exact_composition_reuses_independent_quarters_without_rebinding_acquisitions() {
        let database = TestDatabase::new();
        let q1_request = BenchmarkRequest {
            instrument: "TEST_CODE_sh000300".into(),
            range: BenchmarkRange::Daily {
                from: date(2026, 3, 31),
                to: date(2026, 3, 31),
            },
        };
        let q2_request = BenchmarkRequest {
            instrument: q1_request.instrument.clone(),
            range: BenchmarkRange::Daily {
                from: date(2026, 4, 1),
                to: date(2026, 4, 1),
            },
        };
        let q1 = append_readable(
            &database,
            vec![append_for(
                &database,
                q1_request.clone(),
                date(2026, 1, 1),
                SegmentState::Provisional,
                vec![daily_bar(date(2026, 3, 31), 101.0)],
                "TEST_CODE_compose_q1_receipt",
            )],
            "TEST_CODE independently retained Q1",
        );
        let q2 = append_readable(
            &database,
            vec![append_for(
                &database,
                q2_request.clone(),
                date(2026, 4, 1),
                SegmentState::Provisional,
                vec![daily_bar(date(2026, 4, 1), 102.0)],
                "TEST_CODE_compose_q2_receipt",
            )],
            "TEST_CODE independently retained Q2",
        );
        let exact_request = BenchmarkRequest {
            instrument: q1_request.instrument.clone(),
            range: BenchmarkRange::Daily {
                from: date(2026, 3, 31),
                to: date(2026, 4, 1),
            },
        };

        let composed = database
            .store()
            .compose_exact(
                exact_request.clone(),
                vec![
                    BenchmarkRetainedSegmentRef {
                        source_manifest_hash: q1.manifest_hash.clone(),
                        segment_hash: q1.segment_hashes[0].clone(),
                    },
                    BenchmarkRetainedSegmentRef {
                        source_manifest_hash: q2.manifest_hash.clone(),
                        segment_hash: q2.segment_hashes[0].clone(),
                    },
                ],
            )
            .expect("TEST_CODE exact composition must retain independent quarters");

        assert_eq!(
            composed.segment_hashes,
            vec![q1.segment_hashes[0].clone(), q2.segment_hashes[0].clone()]
        );
        let snapshot = BenchmarkReader::new(&database.manager)
            .read_exact(&composed.manifest_hash, &exact_request)
            .expect("TEST_CODE active Reader accepts exact same-revision composition");
        assert_eq!(snapshot.manifest, composed);
        assert_eq!(
            snapshot
                .bars
                .iter()
                .map(|bar| bar.close)
                .collect::<Vec<_>>(),
            vec![101.0, 102.0]
        );
        assert_eq!(
            snapshot
                .evidence
                .iter()
                .map(|item| item.batch_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "TEST_CODE_compose_q1_receipt",
                "TEST_CODE_compose_q2_receipt"
            ]
        );

        let mut conn = database.conn();
        assert_eq!(count(&mut conn, "data_acquisition_audit"), 2);
        let retained_request_hashes = diesel::sql_query(
            "SELECT json_extract(canonical_binding_json, '$.request_hash') AS value
             FROM benchmark_manifest_acquisition
             WHERE manifest_id = (
                SELECT id FROM benchmark_manifest WHERE manifest_hash = ?
             ) ORDER BY ordinal ASC",
        )
        .bind::<Text, _>(&composed.manifest_hash)
        .load::<TextRow>(&mut conn)
        .expect("TEST_CODE composed acquisition request bindings");
        assert_eq!(
            retained_request_hashes
                .into_iter()
                .map(|row| row.value)
                .collect::<Vec<_>>(),
            vec![
                q1_request.canonical_request_hash(),
                q2_request.canonical_request_hash(),
            ]
        );
    }

    #[test]
    fn exact_composition_rejects_missing_duplicate_or_out_of_order_segment_selections() {
        for case in ["missing", "duplicate", "out_of_order"] {
            let (database, request, q1, q2) = independently_retained_quarters();
            let selections = match case {
                "missing" => vec![retained_segment(&q1, 0)],
                "duplicate" => vec![retained_segment(&q1, 0), retained_segment(&q1, 0)],
                "out_of_order" => vec![retained_segment(&q2, 0), retained_segment(&q1, 0)],
                _ => unreachable!("TEST_CODE fixed composition case"),
            };
            let before = count(&mut database.conn(), "benchmark_manifest");
            let error = database
                .store()
                .compose_exact(request, selections)
                .expect_err("TEST_CODE incomplete or noncanonical composition must fail");
            assert!(
                matches!(
                    error,
                    BenchmarkSegmentStoreError::BenchmarkSegmentUnavailable { .. }
                        | BenchmarkSegmentStoreError::FailedIntegrity { .. }
                ),
                "TEST_CODE {case} must remain typed",
            );
            assert_eq!(
                count(&mut database.conn(), "benchmark_manifest"),
                before,
                "TEST_CODE {case} must not publish a partial manifest",
            );
        }
    }

    #[test]
    fn exact_composition_rejects_source_manifest_segment_identity_mismatch() {
        let (database, request, q1, q2) = independently_retained_quarters();
        let error = database
            .store()
            .compose_exact(
                request,
                vec![
                    BenchmarkRetainedSegmentRef {
                        source_manifest_hash: q1.manifest_hash,
                        segment_hash: q2.segment_hashes[0].clone(),
                    },
                    retained_segment(&q2, 0),
                ],
            )
            .expect_err("TEST_CODE segment must be retained by its named source manifest");
        assert!(matches!(
            error,
            BenchmarkSegmentStoreError::FailedIntegrity { .. }
                | BenchmarkSegmentStoreError::BenchmarkSegmentUnavailable { .. }
        ));
    }

    #[test]
    fn exact_composition_rejects_cross_instrument_granularity_and_provider() {
        let (instrument_database, request, q1, _q2) = independently_retained_quarters();
        let other_request = BenchmarkRequest {
            instrument: "TEST_CODE_other_index".into(),
            range: BenchmarkRange::Daily {
                from: date(2026, 4, 1),
                to: date(2026, 4, 1),
            },
        };
        let other = append_readable(
            &instrument_database,
            vec![append_for(
                &instrument_database,
                other_request,
                date(2026, 4, 1),
                SegmentState::Provisional,
                vec![daily_bar(date(2026, 4, 1), 102.0)],
                "TEST_CODE_compose_other_instrument",
            )],
            "TEST_CODE retained other instrument",
        );
        assert!(instrument_database
            .store()
            .compose_exact(
                request.clone(),
                vec![retained_segment(&q1, 0), retained_segment(&other, 0)],
            )
            .is_err());

        let minute_database = TestDatabase::new();
        let minute = DateTime::parse_from_rfc3339("2026-04-01T09:31:00+08:00")
            .expect("TEST_CODE compose minute");
        let minute_request = BenchmarkRequest {
            instrument: request.instrument.clone(),
            range: BenchmarkRange::Minute1 {
                from: minute,
                to: minute,
            },
        };
        let minute_manifest = append_readable(
            &minute_database,
            vec![append_for(
                &minute_database,
                minute_request,
                date(2026, 4, 1),
                SegmentState::Provisional,
                vec![minute_bar(minute, 102.0)],
                "TEST_CODE_compose_minute_granularity",
            )],
            "TEST_CODE retained minute granularity",
        );
        let daily = append_readable(
            &minute_database,
            vec![append_for(
                &minute_database,
                BenchmarkRequest {
                    instrument: request.instrument.clone(),
                    range: BenchmarkRange::Daily {
                        from: date(2026, 3, 31),
                        to: date(2026, 3, 31),
                    },
                },
                date(2026, 1, 1),
                SegmentState::Provisional,
                vec![daily_bar(date(2026, 3, 31), 101.0)],
                "TEST_CODE_compose_daily_granularity",
            )],
            "TEST_CODE retained daily granularity",
        );
        assert!(minute_database
            .store()
            .compose_exact(
                request.clone(),
                vec![
                    retained_segment(&daily, 0),
                    retained_segment(&minute_manifest, 0),
                ],
            )
            .is_err());

        let provider_database = TestDatabase::new();
        let q1_request = BenchmarkRequest {
            instrument: request.instrument.clone(),
            range: BenchmarkRange::Daily {
                from: date(2026, 3, 31),
                to: date(2026, 3, 31),
            },
        };
        let q2_request = BenchmarkRequest {
            instrument: request.instrument.clone(),
            range: BenchmarkRange::Daily {
                from: date(2026, 4, 1),
                to: date(2026, 4, 1),
            },
        };
        let provider_q1 = append_readable(
            &provider_database,
            vec![append_for(
                &provider_database,
                q1_request,
                date(2026, 1, 1),
                SegmentState::Provisional,
                vec![daily_bar(date(2026, 3, 31), 101.0)],
                "TEST_CODE_compose_tdx_provider",
            )],
            "TEST_CODE retained TDX provider",
        );
        let mut other_provider = evidence("TEST_CODE_compose_tencent_provider");
        other_provider.provider = ProviderId::Tencent;
        other_provider.source = "TEST_CODE_tencent_index".into();
        let provider_q2 = append_readable(
            &provider_database,
            vec![append_with_evidence(
                &provider_database,
                q2_request,
                date(2026, 4, 1),
                SegmentState::Provisional,
                vec![daily_bar(date(2026, 4, 1), 102.0)],
                other_provider,
            )],
            "TEST_CODE retained Tencent provider",
        );
        assert!(provider_database
            .store()
            .compose_exact(
                request,
                vec![
                    retained_segment(&provider_q1, 0),
                    retained_segment(&provider_q2, 0),
                ],
            )
            .is_err());
    }

    #[test]
    fn exact_composition_rejects_different_locked_provider_sources_without_writes() {
        let database = TestDatabase::new();
        let q1_request = BenchmarkRequest {
            instrument: "TEST_CODE_sh000300".into(),
            range: BenchmarkRange::Daily {
                from: date(2026, 3, 31),
                to: date(2026, 3, 31),
            },
        };
        let q2_request = BenchmarkRequest {
            instrument: q1_request.instrument.clone(),
            range: BenchmarkRange::Daily {
                from: date(2026, 4, 1),
                to: date(2026, 4, 1),
            },
        };
        let mut q1_evidence = evidence("TEST_CODE_compose_source_revision_q1");
        q1_evidence.source = "TEST_CODE_magic-tdx-index-bars@revision-a".into();
        let q1 = append_readable(
            &database,
            vec![append_with_evidence(
                &database,
                q1_request.clone(),
                date(2026, 1, 1),
                SegmentState::Provisional,
                vec![daily_bar(date(2026, 3, 31), 101.0)],
                q1_evidence,
            )],
            "TEST_CODE retained provider revision A",
        );
        let mut q2_evidence = evidence("TEST_CODE_compose_source_revision_q2");
        q2_evidence.source = "TEST_CODE_magic-tdx-index-bars@revision-b".into();
        let q2 = append_readable(
            &database,
            vec![append_with_evidence(
                &database,
                q2_request,
                date(2026, 4, 1),
                SegmentState::Provisional,
                vec![daily_bar(date(2026, 4, 1), 102.0)],
                q2_evidence,
            )],
            "TEST_CODE retained provider revision B",
        );
        let request = BenchmarkRequest {
            instrument: q1_request.instrument,
            range: BenchmarkRange::Daily {
                from: date(2026, 3, 31),
                to: date(2026, 4, 1),
            },
        };
        let before = {
            let mut conn = database.conn();
            (
                count(&mut conn, "benchmark_manifest"),
                count(&mut conn, "benchmark_manifest_acquisition"),
                count(&mut conn, "benchmark_manifest_chain"),
            )
        };

        let error = database
            .store()
            .compose_exact(
                request,
                vec![retained_segment(&q1, 0), retained_segment(&q2, 0)],
            )
            .expect_err("TEST_CODE mixed locked provider revisions must fail closed");
        assert_failed_integrity(error, "benchmark_composition_provider_version_mismatch");

        let mut conn = database.conn();
        assert_eq!(
            (
                count(&mut conn, "benchmark_manifest"),
                count(&mut conn, "benchmark_manifest_acquisition"),
                count(&mut conn, "benchmark_manifest_chain"),
            ),
            before,
            "TEST_CODE rejected provider revision mix must publish no manifest evidence",
        );
    }

    #[test]
    fn exact_composition_revalidates_original_receipt_request_and_segment_version() {
        let (receipt_database, request, q1, q2) = independently_retained_quarters();
        let mut conn = receipt_database.conn();
        diesel::sql_query("DROP TRIGGER trg_data_acquisition_audit_no_update")
            .execute(&mut conn)
            .expect("TEST_CODE drop receipt update trigger");
        diesel::sql_query("UPDATE data_acquisition_audit SET request_hash = ? WHERE id = 1")
            .bind::<Text, _>("f".repeat(64))
            .execute(&mut conn)
            .expect("TEST_CODE tamper original receipt request binding");
        drop(conn);
        assert!(receipt_database
            .store()
            .compose_exact(
                request.clone(),
                vec![retained_segment(&q1, 0), retained_segment(&q2, 0)],
            )
            .is_err());

        let (version_database, request, q1, q2) = independently_retained_quarters();
        let mut conn = version_database.conn();
        diesel::sql_query("PRAGMA ignore_check_constraints = ON")
            .execute(&mut conn)
            .expect("TEST_CODE allow adversarial version tamper");
        diesel::sql_query("DROP TRIGGER trg_benchmark_segment_revision_no_update")
            .execute(&mut conn)
            .expect("TEST_CODE drop segment update trigger");
        diesel::sql_query("UPDATE benchmark_segment_revision SET payload_version = 2 WHERE id = 2")
            .execute(&mut conn)
            .expect("TEST_CODE tamper retained payload version");
        diesel::sql_query(canonical_update_trigger_definition(
            "benchmark_segment_revision",
        ))
        .execute(&mut conn)
        .expect("TEST_CODE restore segment update trigger");
        drop(conn);
        assert!(version_database
            .store()
            .compose_exact(
                request,
                vec![retained_segment(&q1, 0), retained_segment(&q2, 0)],
            )
            .is_err());
    }

    #[test]
    fn minute_append_rejects_a_complete_saturday_grid() {
        let database = TestDatabase::new();
        let from = DateTime::parse_from_rfc3339("2026-08-22T09:31:00+08:00")
            .expect("TEST_CODE Saturday from");
        let to = DateTime::parse_from_rfc3339("2026-08-22T09:32:00+08:00")
            .expect("TEST_CODE Saturday to");
        let request = BenchmarkRequest {
            instrument: "TEST_CODE_sh000300".into(),
            range: BenchmarkRange::Minute1 { from, to },
        };
        let error = database
            .store()
            .append(vec![append_for(
                &database,
                request,
                date(2026, 7, 1),
                SegmentState::Provisional,
                vec![minute_bar(from, 101.0), minute_bar(to, 102.0)],
                "TEST_CODE_batch_saturday_minute",
            )])
            .expect_err("TEST_CODE Saturday has no authoritative Minute1 session");
        assert!(
            error.contains("benchmark_minute_non_trading_day"),
            "{error}"
        );
    }

    #[test]
    fn minute_append_rejects_a_complete_exchange_holiday_grid() {
        let database = TestDatabase::new();
        let from = DateTime::parse_from_rfc3339("2026-10-01T09:31:00+08:00")
            .expect("TEST_CODE holiday from");
        let to = DateTime::parse_from_rfc3339("2026-10-01T09:32:00+08:00")
            .expect("TEST_CODE holiday to");
        let request = BenchmarkRequest {
            instrument: "TEST_CODE_sh000300".into(),
            range: BenchmarkRange::Minute1 { from, to },
        };
        let error = database
            .store()
            .append(vec![append_for(
                &database,
                request,
                date(2026, 10, 1),
                SegmentState::Provisional,
                vec![minute_bar(from, 101.0), minute_bar(to, 102.0)],
                "TEST_CODE_batch_holiday_minute",
            )])
            .expect_err("TEST_CODE exchange holiday has no authoritative Minute1 session");
        assert!(
            error.contains("benchmark_minute_non_trading_day"),
            "{error}"
        );
    }

    #[test]
    fn sealed_append_rejects_single_day_daily_payload() {
        let database = TestDatabase::new();
        let error = database
            .store()
            .append(vec![append_for(
                &database,
                daily_request(),
                date(2026, 1, 1),
                SegmentState::Sealed,
                vec![daily_bar(date(2026, 1, 5), 101.0)],
                "TEST_CODE_batch_single_day_sealed",
            )])
            .expect_err("TEST_CODE a caller cannot seal one Daily bar");
        assert!(
            error.contains("BR-251 Daily Sealed requires complete natural-quarter coverage"),
            "{error}"
        );
    }

    #[test]
    fn sealed_append_rejects_evidence_observed_before_quarter_end() {
        let database = TestDatabase::new();
        let request = q1_2026_request();
        let bars = q1_2026_bars(101.0);
        let evidence = evidence_observed_at(
            "TEST_CODE_batch_unended_quarter_sealed",
            "2026-03-31T15:00:00+08:00",
        );
        let error = database
            .store()
            .append(vec![append_with_evidence(
                &database,
                request,
                date(2026, 1, 1),
                SegmentState::Sealed,
                bars,
                evidence,
            )])
            .expect_err("TEST_CODE evidence before quarter end cannot seal it");
        assert!(
            error.contains(
                "BR-251 Daily Sealed evidence observed_at must be after natural-quarter end"
            ),
            "{error}"
        );
    }

    #[test]
    fn sealed_append_rejects_malformed_observed_at() {
        let database = TestDatabase::new();
        let request = q1_2026_request();
        let bars = q1_2026_bars(101.0);
        let evidence = evidence_observed_at("TEST_CODE_batch_malformed_sealed_time", "not-rfc3339");
        let error = database
            .store()
            .append(vec![append_with_evidence(
                &database,
                request,
                date(2026, 1, 1),
                SegmentState::Sealed,
                bars,
                evidence,
            )])
            .expect_err("TEST_CODE malformed observed_at cannot seal a quarter");
        assert!(
            error.contains("BR-251 Daily Sealed evidence observed_at must be RFC3339"),
            "{error}"
        );
    }

    #[test]
    fn sealed_append_rejects_minute_without_complete_quarter_capture_proof() {
        let database = TestDatabase::new();
        let from = DateTime::parse_from_rfc3339("2026-01-05T09:31:00+08:00")
            .expect("TEST_CODE Minute1 Sealed from");
        let to = DateTime::parse_from_rfc3339("2026-01-05T09:32:00+08:00")
            .expect("TEST_CODE Minute1 Sealed to");
        let request = BenchmarkRequest {
            instrument: "TEST_CODE_sh000300".into(),
            range: BenchmarkRange::Minute1 { from, to },
        };
        let error = database
            .store()
            .append(vec![append_for(
                &database,
                request,
                date(2026, 1, 1),
                SegmentState::Sealed,
                vec![minute_bar(from, 101.0), minute_bar(to, 102.0)],
                "TEST_CODE_batch_minute_sealed",
            )])
            .expect_err("TEST_CODE one-day request cannot prove a sealed Minute1 quarter");
        assert!(
            error.contains(
                "BR-251 Minute1 Sealed unavailable without complete-quarter Capture proof"
            ),
            "{error}"
        );
    }

    #[test]
    fn append_rejects_caller_relabelled_unavailable_and_cross_request_receipts() {
        let database = TestDatabase::new();
        let request = daily_request();
        let unavailable_evidence = evidence("TEST_CODE_batch_unavailable");
        let mut unavailable = database.receipt_with_outcome(
            &request,
            &unavailable_evidence,
            "unavailable",
            0,
            "source_unavailable",
            true,
        );
        unavailable.current_outcome = "available".into();
        let error = database
            .store()
            .append(vec![BenchmarkSegmentAppend {
                request: request.clone(),
                quarter_start: date(2026, 1, 1),
                state: SegmentState::Provisional,
                bars: vec![daily_bar(date(2026, 1, 5), 101.0)],
                evidence: unavailable_evidence,
                acquisition_receipt: unavailable,
            }])
            .expect_err("TEST_CODE persisted unavailable outcome must authorize nothing");
        assert!(
            error.contains("BR-159 acquisition audit outcome mismatch"),
            "{error}"
        );

        let audited_request = daily_request();
        let replayed_request = BenchmarkRequest {
            instrument: audited_request.instrument.clone(),
            range: BenchmarkRange::Daily {
                from: date(2026, 1, 6),
                to: date(2026, 1, 6),
            },
        };
        let replay_evidence = evidence("TEST_CODE_batch_cross_request");
        let receipt = database.receipt(&audited_request, &replay_evidence, 1);
        let error = database
            .store()
            .append(vec![BenchmarkSegmentAppend {
                request: replayed_request,
                quarter_start: date(2026, 1, 1),
                state: SegmentState::Provisional,
                bars: vec![daily_bar(date(2026, 1, 6), 102.0)],
                evidence: replay_evidence,
                acquisition_receipt: receipt,
            }])
            .expect_err("TEST_CODE receipt must not replay across requests");
        assert!(
            error.contains("BR-159 acquisition audit request_hash mismatch"),
            "{error}"
        );

        let database = TestDatabase::new();
        let request = daily_request();
        let audited_evidence = evidence("TEST_CODE_batch_audited");
        let receipt = database.receipt(&request, &audited_evidence, 1);
        let replayed_evidence = evidence("TEST_CODE_batch_replayed");
        let error = database
            .store()
            .append(vec![BenchmarkSegmentAppend {
                request,
                quarter_start: date(2026, 1, 1),
                state: SegmentState::Provisional,
                bars: vec![daily_bar(date(2026, 1, 5), 103.0)],
                evidence: replayed_evidence,
                acquisition_receipt: receipt,
            }])
            .expect_err("TEST_CODE receipt must not replay across provider batches");
        assert!(
            error.contains("BR-159 acquisition audit batch_id mismatch"),
            "{error}"
        );
    }

    #[test]
    fn shared_receipt_count_is_the_complete_cross_quarter_payload() {
        let database = TestDatabase::new();
        let request = BenchmarkRequest {
            instrument: "TEST_CODE_sh000300".into(),
            range: BenchmarkRange::Daily {
                from: date(2026, 3, 31),
                to: date(2026, 4, 1),
            },
        };
        let bad_count_evidence = evidence("TEST_CODE_batch_shared_receipt_bad_count");
        let receipt = database.receipt(&request, &bad_count_evidence, 1);
        let appends = vec![
            BenchmarkSegmentAppend {
                request: request.clone(),
                quarter_start: date(2026, 1, 1),
                state: SegmentState::Provisional,
                bars: vec![daily_bar(date(2026, 3, 31), 101.0)],
                evidence: bad_count_evidence.clone(),
                acquisition_receipt: receipt.clone(),
            },
            BenchmarkSegmentAppend {
                request,
                quarter_start: date(2026, 4, 1),
                state: SegmentState::Provisional,
                bars: vec![daily_bar(date(2026, 4, 1), 102.0)],
                evidence: bad_count_evidence,
                acquisition_receipt: receipt,
            },
        ];
        let error = database
            .store()
            .append(appends)
            .expect_err("TEST_CODE sub-segment count must not stand in for full payload count");
        assert!(
            error.contains("BR-159 acquisition audit accepted_count mismatch: expected 2"),
            "{error}"
        );

        let database = TestDatabase::new();
        let request = BenchmarkRequest {
            instrument: "TEST_CODE_sh000300".into(),
            range: BenchmarkRange::Daily {
                from: date(2026, 3, 31),
                to: date(2026, 4, 1),
            },
        };
        let evidence = evidence("TEST_CODE_batch_shared_receipt_complete_count");
        let receipt = database.receipt(&request, &evidence, 2);
        let manifest = append_readable(
            &database,
            vec![
                BenchmarkSegmentAppend {
                    request: request.clone(),
                    quarter_start: date(2026, 1, 1),
                    state: SegmentState::Provisional,
                    bars: vec![daily_bar(date(2026, 3, 31), 101.0)],
                    evidence: evidence.clone(),
                    acquisition_receipt: receipt.clone(),
                },
                BenchmarkSegmentAppend {
                    request,
                    quarter_start: date(2026, 4, 1),
                    state: SegmentState::Provisional,
                    bars: vec![daily_bar(date(2026, 4, 1), 102.0)],
                    evidence,
                    acquisition_receipt: receipt,
                },
            ],
            "TEST_CODE complete shared audit payload count",
        );
        assert_eq!(manifest.segment_hashes.len(), 2);
    }

    #[test]
    fn mixed_reuse_with_a_new_shared_receipt_remains_exactly_readable() {
        let database = TestDatabase::new();
        let request = BenchmarkRequest {
            instrument: "TEST_CODE_sh000300".into(),
            range: BenchmarkRange::Daily {
                from: date(2026, 3, 31),
                to: date(2026, 4, 1),
            },
        };
        let original = append_readable(
            &database,
            vec![
                append_for(
                    &database,
                    request.clone(),
                    date(2026, 1, 1),
                    SegmentState::Provisional,
                    vec![daily_bar(date(2026, 3, 31), 101.0)],
                    "TEST_CODE_batch_independent_q1",
                ),
                append_for(
                    &database,
                    request.clone(),
                    date(2026, 4, 1),
                    SegmentState::Provisional,
                    vec![daily_bar(date(2026, 4, 1), 102.0)],
                    "TEST_CODE_batch_independent_q2",
                ),
            ],
            "TEST_CODE original independent cross-quarter receipts",
        );

        let correction_evidence = evidence("TEST_CODE_batch_shared_correction");
        let correction_receipt = database.receipt(&request, &correction_evidence, 2);
        let corrected = append_readable(
            &database,
            vec![
                BenchmarkSegmentAppend {
                    request: request.clone(),
                    quarter_start: date(2026, 1, 1),
                    state: SegmentState::Provisional,
                    bars: vec![daily_bar(date(2026, 3, 31), 101.0)],
                    evidence: correction_evidence.clone(),
                    acquisition_receipt: correction_receipt.clone(),
                },
                BenchmarkSegmentAppend {
                    request,
                    quarter_start: date(2026, 4, 1),
                    state: SegmentState::Provisional,
                    bars: vec![daily_bar(date(2026, 4, 1), 103.0)],
                    evidence: correction_evidence,
                    acquisition_receipt: correction_receipt,
                },
            ],
            "TEST_CODE mixed reused Q1 and corrected Q2 shared receipt",
        );

        assert_eq!(corrected.segment_hashes[0], original.segment_hashes[0]);
        assert_ne!(corrected.segment_hashes[1], original.segment_hashes[1]);
        let (retained_original, original_bars, _) = database
            .store()
            .read_exact(&original.manifest_hash)
            .expect("TEST_CODE old manifest remains exactly readable");
        assert_eq!(retained_original, original);
        assert_eq!(
            original_bars
                .iter()
                .map(|bar| bar.close)
                .collect::<Vec<_>>(),
            vec![101.0, 102.0]
        );

        let mut conn = database.conn();
        assert_eq!(count(&mut conn, "benchmark_segment_revision"), 3);
        assert_eq!(count(&mut conn, "benchmark_segment_chain"), 3);
        assert_eq!(count(&mut conn, "benchmark_manifest"), 2);
        assert_eq!(count(&mut conn, "benchmark_manifest_chain"), 2);
        assert_eq!(count(&mut conn, "benchmark_manifest_acquisition"), 3);
        let predecessor = diesel::sql_query(
            "SELECT predecessor_segment_hash AS value
             FROM benchmark_segment_revision WHERE segment_hash = ?",
        )
        .bind::<Text, _>(&corrected.segment_hashes[1])
        .get_result::<OptionalTextRow>(&mut conn)
        .expect("TEST_CODE corrected Q2 predecessor");
        assert_eq!(
            predecessor.value.as_deref(),
            Some(original.segment_hashes[1].as_str())
        );
    }

    #[test]
    fn correction_after_an_original_shared_receipt_preserves_both_manifests() {
        let database = TestDatabase::new();
        let request = BenchmarkRequest {
            instrument: "TEST_CODE_sh000300".into(),
            range: BenchmarkRange::Daily {
                from: date(2026, 3, 31),
                to: date(2026, 4, 1),
            },
        };
        let original_evidence = evidence("TEST_CODE_batch_original_shared");
        let original_receipt = database.receipt(&request, &original_evidence, 2);
        let original = append_readable(
            &database,
            vec![
                BenchmarkSegmentAppend {
                    request: request.clone(),
                    quarter_start: date(2026, 1, 1),
                    state: SegmentState::Provisional,
                    bars: vec![daily_bar(date(2026, 3, 31), 101.0)],
                    evidence: original_evidence.clone(),
                    acquisition_receipt: original_receipt.clone(),
                },
                BenchmarkSegmentAppend {
                    request: request.clone(),
                    quarter_start: date(2026, 4, 1),
                    state: SegmentState::Provisional,
                    bars: vec![daily_bar(date(2026, 4, 1), 102.0)],
                    evidence: original_evidence,
                    acquisition_receipt: original_receipt,
                },
            ],
            "TEST_CODE original shared cross-quarter receipt",
        );

        let corrected = append_readable(
            &database,
            vec![
                append_for(
                    &database,
                    request.clone(),
                    date(2026, 1, 1),
                    SegmentState::Provisional,
                    vec![daily_bar(date(2026, 3, 31), 101.0)],
                    "TEST_CODE_batch_correction_independent_q1",
                ),
                append_for(
                    &database,
                    request,
                    date(2026, 4, 1),
                    SegmentState::Provisional,
                    vec![daily_bar(date(2026, 4, 1), 103.0)],
                    "TEST_CODE_batch_correction_independent_q2",
                ),
            ],
            "TEST_CODE correction after original shared receipt",
        );

        assert_eq!(corrected.segment_hashes[0], original.segment_hashes[0]);
        assert_ne!(corrected.segment_hashes[1], original.segment_hashes[1]);
        let (retained_original, original_bars, _) = database
            .store()
            .read_exact(&original.manifest_hash)
            .expect("TEST_CODE original shared manifest remains readable");
        assert_eq!(retained_original, original);
        assert_eq!(
            original_bars
                .iter()
                .map(|bar| bar.close)
                .collect::<Vec<_>>(),
            vec![101.0, 102.0]
        );

        let mut conn = database.conn();
        assert_eq!(count(&mut conn, "benchmark_segment_revision"), 3);
        assert_eq!(count(&mut conn, "benchmark_segment_chain"), 3);
        assert_eq!(count(&mut conn, "benchmark_manifest"), 2);
        assert_eq!(count(&mut conn, "benchmark_manifest_chain"), 2);
        assert_eq!(count(&mut conn, "benchmark_manifest_acquisition"), 3);
        let predecessor = diesel::sql_query(
            "SELECT predecessor_segment_hash AS value
             FROM benchmark_segment_revision WHERE segment_hash = ?",
        )
        .bind::<Text, _>(&corrected.segment_hashes[1])
        .get_result::<OptionalTextRow>(&mut conn)
        .expect("TEST_CODE corrected Q2 predecessor");
        assert_eq!(
            predecessor.value.as_deref(),
            Some(original.segment_hashes[1].as_str())
        );
    }

    #[test]
    fn startup_rejects_deleted_association_only_acquisition_audit_tail() {
        let database = TestDatabase::new();
        let request = daily_request();
        let bars = vec![daily_bar(date(2026, 1, 5), 101.0)];
        let original = append_readable(
            &database,
            vec![append_for(
                &database,
                request.clone(),
                date(2026, 1, 1),
                SegmentState::Provisional,
                bars.clone(),
                "TEST_CODE_batch_original_segment_receipt",
            )],
            "TEST_CODE original segment receipt",
        );

        let replacement_evidence = evidence("TEST_CODE_batch_association_only_receipt");
        let replacement_receipt = database.receipt(&request, &replacement_evidence, 1);
        let association_only_audit_id = replacement_receipt.audit_id;
        let replacement = append_readable(
            &database,
            vec![BenchmarkSegmentAppend {
                request,
                quarter_start: date(2026, 1, 1),
                state: SegmentState::Provisional,
                bars,
                evidence: replacement_evidence,
                acquisition_receipt: replacement_receipt,
            }],
            "TEST_CODE full reuse with association-only receipt",
        );
        assert_eq!(replacement.segment_hashes, original.segment_hashes);
        assert_ne!(replacement.manifest_hash, original.manifest_hash);

        let mut conn = database.conn();
        diesel::sql_query("PRAGMA foreign_keys = OFF")
            .execute(&mut conn)
            .expect("TEST_CODE disable foreign keys for tail corruption");
        diesel::sql_query("DROP TRIGGER trg_data_acquisition_audit_chain_no_delete")
            .execute(&mut conn)
            .expect("TEST_CODE drop audit-chain retention trigger");
        diesel::sql_query("DROP TRIGGER trg_data_acquisition_audit_no_delete")
            .execute(&mut conn)
            .expect("TEST_CODE drop audit-row retention trigger");
        diesel::sql_query(
            "DELETE FROM data_acquisition_audit_chain WHERE acquisition_audit_id = ?",
        )
        .bind::<BigInt, _>(association_only_audit_id)
        .execute(&mut conn)
        .expect("TEST_CODE delete association-only audit chain tail");
        diesel::sql_query("DELETE FROM data_acquisition_audit WHERE id = ?")
            .bind::<BigInt, _>(association_only_audit_id)
            .execute(&mut conn)
            .expect("TEST_CODE delete association-only audit row tail");
        super::super::data_acquisition_audit::validate_data_acquisition_audit_chain(&mut conn)
            .expect("TEST_CODE retained BR-159 prefix remains internally valid");

        let error = create_schema(&mut conn)
            .expect_err("TEST_CODE startup rejects missing association-only audit evidence");
        match map_diesel_error(
            error,
            "TEST_CODE startup association audit validation",
            DieselErrorContext::TransactionEnvelope,
        ) {
            BenchmarkSegmentStoreError::FailedIntegrity {
                reason_code,
                detail,
            } => {
                assert_eq!(reason_code, "benchmark_manifest_acquisition_audit_invalid");
                assert!(detail.contains("receipt id/hash mismatch"), "{detail}");
            }
            other => panic!("TEST_CODE unexpected association-audit category: {other:?}"),
        }
    }

    #[test]
    fn append_rejects_incomplete_authoritative_payload_coverage() {
        let database = TestDatabase::new();
        let request = BenchmarkRequest {
            instrument: "TEST_CODE_sh000300".into(),
            range: BenchmarkRange::Daily {
                from: date(2026, 1, 1),
                to: date(2026, 12, 31),
            },
        };
        let evidence = evidence("TEST_CODE_batch_incomplete_year");
        let receipt = database.receipt(&request, &evidence, 1);
        let error = database
            .store()
            .append(vec![BenchmarkSegmentAppend {
                request,
                quarter_start: date(2026, 1, 1),
                state: SegmentState::Provisional,
                bars: vec![daily_bar(date(2026, 1, 5), 101.0)],
                evidence,
                acquisition_receipt: receipt,
            }])
            .expect_err("TEST_CODE one bar cannot cover a full year");
        assert!(
            error.contains("benchmark_daily_coverage_incomplete"),
            "{error}"
        );
    }

    #[test]
    fn append_rejects_gapped_and_off_session_minute_payloads() {
        let database = TestDatabase::new();
        let m0931 =
            DateTime::parse_from_rfc3339("2026-08-21T09:31:00+08:00").expect("TEST_CODE m0931");
        let m0933 =
            DateTime::parse_from_rfc3339("2026-08-21T09:33:00+08:00").expect("TEST_CODE m0933");
        let request = BenchmarkRequest {
            instrument: "TEST_CODE_sh000300".into(),
            range: BenchmarkRange::Minute1 {
                from: m0931,
                to: m0933,
            },
        };
        let gap_evidence = evidence("TEST_CODE_batch_minute_gap");
        let receipt = database.receipt(&request, &gap_evidence, 2);
        let error = database
            .store()
            .append(vec![BenchmarkSegmentAppend {
                request,
                quarter_start: date(2026, 7, 1),
                state: SegmentState::Provisional,
                bars: vec![minute_bar(m0931, 101.0), minute_bar(m0933, 102.0)],
                evidence: gap_evidence,
                acquisition_receipt: receipt,
            }])
            .expect_err("TEST_CODE Minute1 gap must fail before persistence");
        assert!(
            error.contains("benchmark_minute_coverage_incomplete"),
            "{error}"
        );

        let m1130 =
            DateTime::parse_from_rfc3339("2026-08-21T11:30:00+08:00").expect("TEST_CODE m1130");
        let m1131 = DateTime::parse_from_rfc3339("2026-08-21T11:31:00+08:00")
            .expect("TEST_CODE off-session m1131");
        let m1301 =
            DateTime::parse_from_rfc3339("2026-08-21T13:01:00+08:00").expect("TEST_CODE m1301");
        let request = BenchmarkRequest {
            instrument: "TEST_CODE_sh000300".into(),
            range: BenchmarkRange::Minute1 {
                from: m1130,
                to: m1301,
            },
        };
        let evidence = evidence("TEST_CODE_batch_minute_off_session");
        let receipt = database.receipt(&request, &evidence, 3);
        let error = database
            .store()
            .append(vec![BenchmarkSegmentAppend {
                request,
                quarter_start: date(2026, 7, 1),
                state: SegmentState::Provisional,
                bars: vec![
                    minute_bar(m1130, 101.0),
                    minute_bar(m1131, 102.0),
                    minute_bar(m1301, 103.0),
                ],
                evidence,
                acquisition_receipt: receipt,
            }])
            .expect_err("TEST_CODE off-session Minute1 bar must fail before persistence");
        assert!(error.contains("benchmark_minute_bar_invalid"), "{error}");
    }

    #[test]
    fn manifest_chain_insert_failure_rolls_back_segment_and_manifest_transaction() {
        let database = TestDatabase::new();
        let mut conn = database.conn();
        diesel::sql_query(
            "CREATE TRIGGER TEST_CODE_fail_manifest_chain
             BEFORE INSERT ON benchmark_manifest_chain
             BEGIN SELECT RAISE(ABORT, 'TEST_CODE manifest chain insert failed'); END",
        )
        .execute(&mut conn)
        .expect("TEST_CODE failure trigger");
        drop(conn);

        let append = append_for(
            &database,
            daily_request(),
            date(2026, 1, 1),
            SegmentState::Provisional,
            vec![daily_bar(date(2026, 1, 5), 101.0)],
            "TEST_CODE_batch_atomic",
        );
        let error = database
            .store()
            .append(vec![append])
            .expect_err("TEST_CODE manifest-chain failure must fail append");
        let detail = assert_failed_integrity(error, "benchmark_segment_storage_body_unknown");
        assert!(detail.contains("TEST_CODE manifest chain insert failed"));
        let mut conn = database.conn();
        for table in [
            "benchmark_segment_revision",
            "benchmark_segment_chain",
            "benchmark_manifest",
            "benchmark_manifest_chain",
        ] {
            assert_eq!(count(&mut conn, table), 0, "{table}");
        }
    }

    #[test]
    fn append_exactly_rereads_persisted_associations_before_commit() {
        let database = TestDatabase::new();
        let mut conn = database.conn();
        diesel::sql_query("DROP TRIGGER trg_benchmark_manifest_acquisition_no_delete")
            .execute(&mut conn)
            .expect("TEST_CODE drop association retention trigger");
        diesel::sql_query(
            "CREATE TRIGGER TEST_CODE_remove_association_before_manifest_chain
             BEFORE INSERT ON benchmark_manifest_chain
             BEGIN DELETE FROM benchmark_manifest_acquisition; END",
        )
        .execute(&mut conn)
        .expect("TEST_CODE association corruption trigger");
        drop(conn);

        let append = append_for(
            &database,
            daily_request(),
            date(2026, 1, 1),
            SegmentState::Provisional,
            vec![daily_bar(date(2026, 1, 5), 101.0)],
            "TEST_CODE_batch_append_reread",
        );
        let error = database
            .store()
            .append(vec![append])
            .expect_err("TEST_CODE append must prove exact readability before commit");
        match error {
            BenchmarkSegmentStoreError::FailedIntegrity {
                reason_code,
                detail,
            } => {
                assert_eq!(reason_code, "benchmark_trigger_definition_invalid");
                assert!(detail.contains("immutable trigger"));
            }
            other => panic!("TEST_CODE unexpected append-reread category: {other:?}"),
        }

        let mut conn = database.conn();
        for table in [
            "benchmark_segment_revision",
            "benchmark_segment_chain",
            "benchmark_manifest",
            "benchmark_manifest_chain",
            "benchmark_manifest_acquisition",
        ] {
            assert_eq!(count(&mut conn, table), 0, "{table}");
        }
    }

    #[test]
    fn natural_quarters_are_sorted_and_daily_minute_payloads_cannot_mix() {
        let database = TestDatabase::new();
        let request = BenchmarkRequest {
            instrument: "TEST_CODE_sh000300".into(),
            range: BenchmarkRange::Daily {
                from: date(2026, 3, 31),
                to: date(2026, 4, 1),
            },
        };
        let appends = [
            (4, 4, 1, 102.0, "TEST_CODE_batch_q2"),
            (1, 3, 31, 101.0, "TEST_CODE_batch_q1"),
        ]
        .into_iter()
        .map(|(quarter_month, bar_month, bar_day, close, batch)| {
            append_for(
                &database,
                request.clone(),
                date(2026, quarter_month, 1),
                SegmentState::Provisional,
                vec![daily_bar(date(2026, bar_month, bar_day), close)],
                batch,
            )
        })
        .collect();
        let manifest = append_readable(&database, appends, "TEST_CODE natural quarters");
        assert_eq!(manifest.segment_hashes.len(), 2);
        let (_, bars, _) = database
            .store()
            .read_exact(&manifest.manifest_hash)
            .expect("TEST_CODE exact manifest");
        assert_eq!(
            bars.iter().map(|bar| bar.close).collect::<Vec<_>>(),
            vec![101.0, 102.0]
        );

        let invalid_quarter = append_for(
            &database,
            BenchmarkRequest {
                instrument: "TEST_CODE_sh000300".into(),
                range: BenchmarkRange::Daily {
                    from: date(2026, 2, 24),
                    to: date(2026, 2, 24),
                },
            },
            date(2026, 2, 1),
            SegmentState::Provisional,
            vec![daily_bar(date(2026, 2, 24), 105.0)],
            "TEST_CODE_batch_bad_quarter",
        );
        assert!(database.store().append(vec![invalid_quarter]).is_err());

        let from = DateTime::parse_from_rfc3339("2026-01-05T09:31:00+08:00")
            .expect("TEST_CODE minute from");
        let to =
            DateTime::parse_from_rfc3339("2026-01-05T09:32:00+08:00").expect("TEST_CODE minute to");
        let minute_request = BenchmarkRequest {
            instrument: "TEST_CODE_sh000300".into(),
            range: BenchmarkRange::Minute1 { from, to },
        };
        let mixed = append_for(
            &database,
            minute_request,
            date(2026, 1, 1),
            SegmentState::Provisional,
            vec![daily_bar(date(2026, 1, 5), 106.0)],
            "TEST_CODE_batch_mixed",
        );
        assert!(database.store().append(vec![mixed]).is_err());
    }

    #[test]
    fn complete_ended_daily_quarter_transitions_provisional_to_sealed_and_corrected_successor() {
        let database = TestDatabase::new();
        let request = q1_2026_request();
        let first = append_with_evidence(
            &database,
            request.clone(),
            date(2026, 1, 1),
            SegmentState::Provisional,
            q1_2026_bars(101.0),
            evidence_observed_at(
                "TEST_CODE_batch_full_q1_provisional",
                "2026-04-01T00:00:01+08:00",
            ),
        );
        let first_manifest = append_readable(
            &database,
            vec![first.clone()],
            "TEST_CODE complete Q1 provisional",
        );
        let replay_manifest = append_readable(
            &database,
            vec![first],
            "TEST_CODE complete Q1 provisional replay",
        );
        assert_eq!(first_manifest, replay_manifest);

        let sealed = append_with_evidence(
            &database,
            request.clone(),
            date(2026, 1, 1),
            SegmentState::Sealed,
            q1_2026_bars(101.0),
            evidence_observed_at("TEST_CODE_batch_full_q1_sealed", "2026-03-31T16:00:02Z"),
        );
        let sealed_manifest = append_readable(
            &database,
            vec![sealed],
            "TEST_CODE complete ended Q1 sealed successor",
        );
        let corrected = append_with_evidence(
            &database,
            request,
            date(2026, 1, 1),
            SegmentState::Sealed,
            q1_2026_bars(102.0),
            evidence_observed_at("TEST_CODE_batch_full_q1_corrected", "2026-04-01T16:00:00Z"),
        );
        let corrected_manifest = append_readable(
            &database,
            vec![corrected],
            "TEST_CODE complete ended Q1 corrected sealed successor",
        );
        assert_ne!(first_manifest.manifest_hash, sealed_manifest.manifest_hash);
        assert_ne!(
            sealed_manifest.manifest_hash,
            corrected_manifest.manifest_hash
        );

        let mut conn = database.conn();
        assert_eq!(count(&mut conn, "benchmark_segment_revision"), 3);
        assert_eq!(count(&mut conn, "benchmark_segment_chain"), 3);
        assert_eq!(count(&mut conn, "benchmark_manifest"), 3);
        assert_eq!(count(&mut conn, "benchmark_manifest_chain"), 3);
        let predecessors = diesel::sql_query(
            "SELECT predecessor_segment_hash AS value
             FROM benchmark_segment_revision ORDER BY id ASC",
        )
        .load::<OptionalTextRow>(&mut conn)
        .expect("TEST_CODE predecessors");
        assert_eq!(predecessors[0].value, None);
        assert_eq!(
            predecessors[1].value.as_deref(),
            Some(first_manifest.segment_hashes[0].as_str())
        );
        assert_eq!(
            predecessors[2].value.as_deref(),
            Some(sealed_manifest.segment_hashes[0].as_str())
        );
    }

    #[test]
    fn exact_reader_rejects_deleted_manifest_acquisition_association() {
        let database = TestDatabase::new();
        let manifest =
            shared_cross_quarter_manifest(&database, "TEST_CODE_batch_deleted_association");
        let mut conn = database.conn();
        diesel::sql_query("DROP TRIGGER trg_benchmark_manifest_acquisition_no_delete")
            .execute(&mut conn)
            .expect("TEST_CODE drop association delete trigger");
        diesel::sql_query("DELETE FROM benchmark_manifest_acquisition")
            .execute(&mut conn)
            .expect("TEST_CODE delete association");
        diesel::sql_query(
            "CREATE TRIGGER trg_benchmark_manifest_acquisition_no_delete
             BEFORE DELETE ON benchmark_manifest_acquisition
             BEGIN SELECT RAISE(ABORT, 'BR-251 benchmark manifest acquisition retention is at least five years'); END",
        )
        .execute(&mut conn)
        .expect("TEST_CODE restore association delete trigger");
        create_schema(&mut conn).expect_err("TEST_CODE startup rejects missing association");
        drop(conn);

        let error = database
            .store()
            .read_exact(&manifest.manifest_hash)
            .expect_err("TEST_CODE exact read rejects missing association");
        match error {
            BenchmarkSegmentStoreError::FailedIntegrity {
                reason_code,
                detail,
            } => {
                assert_eq!(reason_code, "benchmark_sequence_highwater_invalid");
                assert!(detail.contains("AUTOINCREMENT high-water"));
            }
            other => panic!("TEST_CODE unexpected missing-association category: {other:?}"),
        }
    }

    #[test]
    fn exact_reader_rejects_noncanonical_manifest_acquisition_tamper() {
        let database = TestDatabase::new();
        let manifest =
            shared_cross_quarter_manifest(&database, "TEST_CODE_batch_binding_bytes_tamper");
        let mut conn = database.conn();
        diesel::sql_query("DROP TRIGGER trg_benchmark_manifest_acquisition_no_update")
            .execute(&mut conn)
            .expect("TEST_CODE drop association update trigger");
        diesel::sql_query(
            "UPDATE benchmark_manifest_acquisition
             SET canonical_binding_json = canonical_binding_json || ' '",
        )
        .execute(&mut conn)
        .expect("TEST_CODE tamper canonical association bytes");
        diesel::sql_query(canonical_update_trigger_definition(
            "benchmark_manifest_acquisition",
        ))
        .execute(&mut conn)
        .expect("TEST_CODE restore association update trigger");
        create_schema(&mut conn).expect_err("TEST_CODE startup rejects noncanonical binding bytes");
        drop(conn);

        let error = database
            .store()
            .read_exact(&manifest.manifest_hash)
            .expect_err("TEST_CODE exact read rejects noncanonical binding bytes");
        match error {
            BenchmarkSegmentStoreError::FailedIntegrity {
                reason_code,
                detail,
            } => {
                assert_eq!(reason_code, "benchmark_manifest_acquisition_noncanonical");
                assert!(detail.contains("binding bytes are not canonical"));
            }
            other => panic!("TEST_CODE unexpected noncanonical category: {other:?}"),
        }
    }

    #[test]
    fn exact_reader_rejects_manifest_acquisition_count_mismatch() {
        let database = TestDatabase::new();
        let manifest =
            shared_cross_quarter_manifest(&database, "TEST_CODE_batch_binding_count_tamper");
        rewrite_only_manifest_binding(&database, &manifest, |binding| {
            binding.accepted_count = 1;
        });
        let mut conn = database.conn();
        create_schema(&mut conn).expect_err("TEST_CODE startup rejects association count mismatch");
        drop(conn);

        let error = database
            .store()
            .read_exact(&manifest.manifest_hash)
            .expect_err("TEST_CODE exact read rejects association count mismatch");
        match error {
            BenchmarkSegmentStoreError::FailedIntegrity {
                reason_code,
                detail,
            } => {
                assert_eq!(reason_code, "benchmark_manifest_acquisition_count_mismatch");
                assert!(detail.contains("acquisition accepted_count membership mismatch"));
            }
            other => panic!("TEST_CODE unexpected count-mismatch category: {other:?}"),
        }
    }

    #[test]
    fn exact_reader_rejects_manifest_acquisition_member_outside_manifest() {
        let database = TestDatabase::new();
        let manifest =
            shared_cross_quarter_manifest(&database, "TEST_CODE_batch_binding_outside_member");
        rewrite_only_manifest_binding(&database, &manifest, |binding| {
            binding.members[0].segment_hash = "a".repeat(64);
        });
        let mut conn = database.conn();
        create_schema(&mut conn).expect_err("TEST_CODE startup rejects member outside manifest");
        drop(conn);

        let error = database
            .store()
            .read_exact(&manifest.manifest_hash)
            .expect_err("TEST_CODE exact read rejects member outside manifest");
        match error {
            BenchmarkSegmentStoreError::FailedIntegrity {
                reason_code,
                detail,
            } => {
                assert_eq!(reason_code, "benchmark_manifest_acquisition_member_outside");
                assert!(detail.contains("member outside the manifest"));
            }
            other => panic!("TEST_CODE unexpected outside-member category: {other:?}"),
        }
    }

    #[test]
    fn exact_reader_rejects_uncovered_manifest_member() {
        let database = TestDatabase::new();
        let manifest =
            shared_cross_quarter_manifest(&database, "TEST_CODE_batch_binding_uncovered_member");
        rewrite_only_manifest_binding(&database, &manifest, |binding| {
            binding.members.pop();
            binding.accepted_count = 1;
        });
        let mut conn = database.conn();
        create_schema(&mut conn).expect_err("TEST_CODE startup rejects uncovered member");
        drop(conn);

        let error = database
            .store()
            .read_exact(&manifest.manifest_hash)
            .expect_err("TEST_CODE exact read rejects uncovered manifest member");
        match error {
            BenchmarkSegmentStoreError::FailedIntegrity {
                reason_code,
                detail,
            } => {
                assert_eq!(
                    reason_code,
                    "benchmark_manifest_acquisition_member_uncovered"
                );
                assert!(detail.contains("leaves a member uncovered"));
            }
            other => panic!("TEST_CODE unexpected uncovered-member category: {other:?}"),
        }
    }

    #[test]
    fn exact_reader_rejects_duplicate_manifest_acquisition_audit() {
        let database = TestDatabase::new();
        let manifest =
            shared_cross_quarter_manifest(&database, "TEST_CODE_batch_binding_duplicate_audit");
        let mut conn = database.conn();
        let mut binding: CanonicalManifestAcquisitionBindingV1 = diesel::sql_query(
            "SELECT canonical_binding_json AS value
             FROM benchmark_manifest_acquisition LIMIT 1",
        )
        .get_result::<TextRow>(&mut conn)
        .map(|row| serde_json::from_str(&row.value).expect("TEST_CODE decode binding"))
        .expect("TEST_CODE load binding");
        binding.batch_id.push_str("_duplicate");
        let canonical = serde_json::to_string(&binding).expect("TEST_CODE duplicate binding");
        let binding_hash =
            manifest_acquisition_binding_hash(&binding).expect("TEST_CODE duplicate binding hash");
        diesel::sql_query(
            "INSERT INTO benchmark_manifest_acquisition (
                manifest_id, ordinal, binding_hash, canonical_binding_json
             ) SELECT id, 1, ?, ? FROM benchmark_manifest WHERE manifest_hash = ?",
        )
        .bind::<Text, _>(&binding_hash)
        .bind::<Text, _>(&canonical)
        .bind::<Text, _>(&manifest.manifest_hash)
        .execute(&mut conn)
        .expect("TEST_CODE inject duplicate audit association");
        create_schema(&mut conn)
            .expect_err("TEST_CODE startup rejects duplicate audit association");
        drop(conn);

        let error = database
            .store()
            .read_exact(&manifest.manifest_hash)
            .expect_err("TEST_CODE exact read rejects duplicate audit association");
        match error {
            BenchmarkSegmentStoreError::FailedIntegrity {
                reason_code,
                detail,
            } => {
                assert_eq!(
                    reason_code,
                    "benchmark_manifest_acquisition_audit_duplicate"
                );
                assert!(detail.contains("repeats an audit id"));
            }
            other => panic!("TEST_CODE unexpected duplicate-audit category: {other:?}"),
        }
    }

    #[test]
    fn diesel_mapper_distinguishes_body_envelope_unknown_and_known_database_kinds() {
        use diesel::result::DatabaseErrorKind;

        for label in ["TEST_CODE SQLITE_CORRUPT", "TEST_CODE schema changed"] {
            assert_failed_integrity(
                map_diesel_error(
                    test_database_error(DatabaseErrorKind::Unknown, label),
                    "TEST_CODE transaction body",
                    DieselErrorContext::TransactionBody,
                ),
                "benchmark_segment_storage_body_unknown",
            );
        }
        assert_unavailable(
            map_diesel_error(
                test_database_error(DatabaseErrorKind::Unknown, "TEST_CODE database locked"),
                "TEST_CODE transaction envelope",
                DieselErrorContext::TransactionEnvelope,
            ),
            "benchmark_segment_storage_unavailable",
            true,
        );

        for kind in [
            DatabaseErrorKind::UniqueViolation,
            DatabaseErrorKind::ForeignKeyViolation,
            DatabaseErrorKind::CheckViolation,
        ] {
            assert_failed_integrity(
                map_diesel_error(
                    test_database_error(kind, "TEST_CODE constraint"),
                    "TEST_CODE known constraint",
                    DieselErrorContext::TransactionBody,
                ),
                "benchmark_segment_storage_constraint",
            );
        }
        assert_unavailable(
            map_diesel_error(
                test_database_error(DatabaseErrorKind::ReadOnlyTransaction, "TEST_CODE readonly"),
                "TEST_CODE readonly",
                DieselErrorContext::TransactionBody,
            ),
            "benchmark_segment_storage_read_only",
            false,
        );
        assert_failed_integrity(
            map_diesel_error(
                test_database_error(
                    DatabaseErrorKind::UnableToSendCommand,
                    "TEST_CODE protocol misuse",
                ),
                "TEST_CODE protocol",
                DieselErrorContext::TransactionEnvelope,
            ),
            "benchmark_segment_storage_protocol",
        );
        assert_unavailable(
            map_diesel_error(
                test_database_error(
                    DatabaseErrorKind::SerializationFailure,
                    "TEST_CODE serialization conflict",
                ),
                "TEST_CODE serialization conflict",
                DieselErrorContext::TransactionEnvelope,
            ),
            "benchmark_segment_transaction_conflict",
            true,
        );
        assert_unavailable(
            map_diesel_error(
                test_database_error(DatabaseErrorKind::ClosedConnection, "TEST_CODE closed"),
                "TEST_CODE closed connection",
                DieselErrorContext::TransactionEnvelope,
            ),
            "benchmark_segment_connection_closed",
            true,
        );
    }

    #[test]
    fn diesel_mapper_classifies_programmer_data_and_transaction_state_errors() {
        let invalid_c_string = std::ffi::CString::new(b"TEST_CODE\0invalid".as_slice())
            .expect_err("TEST_CODE embedded nul");
        assert_failed_integrity(
            map_diesel_error(
                diesel::result::Error::InvalidCString(invalid_c_string),
                "TEST_CODE invalid c string",
                DieselErrorContext::TransactionBody,
            ),
            "benchmark_segment_query_invalid_cstring",
        );
        assert_failed_integrity(
            map_diesel_error(
                diesel::result::Error::NotFound,
                "TEST_CODE unexpected not found",
                DieselErrorContext::TransactionBody,
            ),
            "benchmark_segment_unexpected_not_found",
        );
        assert_failed_integrity(
            map_diesel_error(
                diesel::result::Error::QueryBuilderError(Box::new(std::io::Error::other(
                    "TEST_CODE non-store query builder",
                ))),
                "TEST_CODE invalid query",
                DieselErrorContext::TransactionBody,
            ),
            "benchmark_segment_query_failed_integrity",
        );
        for error in [
            diesel::result::Error::SerializationError(Box::new(std::io::Error::other(
                "TEST_CODE serialization",
            ))),
            diesel::result::Error::DeserializationError(Box::new(std::io::Error::other(
                "TEST_CODE deserialization",
            ))),
        ] {
            assert_failed_integrity(
                map_diesel_error(
                    error,
                    "TEST_CODE data representation",
                    DieselErrorContext::TransactionBody,
                ),
                "benchmark_segment_storage_serialization",
            );
        }
        for (error, reason_code) in [
            (
                diesel::result::Error::RollbackTransaction,
                "benchmark_segment_transaction_rollback_requested",
            ),
            (
                diesel::result::Error::AlreadyInTransaction,
                "benchmark_segment_transaction_already_active",
            ),
            (
                diesel::result::Error::NotInTransaction,
                "benchmark_segment_transaction_not_active",
            ),
        ] {
            assert_failed_integrity(
                map_diesel_error(
                    error,
                    "TEST_CODE transaction state",
                    DieselErrorContext::TransactionEnvelope,
                ),
                reason_code,
            );
        }
        assert_unavailable(
            map_diesel_error(
                diesel::result::Error::BrokenTransactionManager,
                "TEST_CODE broken transaction manager",
                DieselErrorContext::TransactionEnvelope,
            ),
            "benchmark_segment_transaction_manager_broken",
            true,
        );
    }

    #[test]
    fn diesel_mapper_recursively_combines_rollback_and_commit_failures() {
        use diesel::result::DatabaseErrorKind;

        let availability = diesel::result::Error::RollbackErrorOnCommit {
            rollback_error: Box::new(test_database_error(
                DatabaseErrorKind::Unknown,
                "TEST_CODE rollback locked",
            )),
            commit_error: Box::new(test_database_error(
                DatabaseErrorKind::ClosedConnection,
                "TEST_CODE commit closed",
            )),
        };
        let detail = assert_unavailable(
            map_diesel_error(
                availability,
                "TEST_CODE rollback availability",
                DieselErrorContext::TransactionEnvelope,
            ),
            "benchmark_segment_transaction_rollback_commit_unavailable",
            true,
        );
        assert!(detail.contains("rollback locked"));
        assert!(detail.contains("commit closed"));

        let nonretryable_availability = diesel::result::Error::RollbackErrorOnCommit {
            rollback_error: Box::new(test_database_error(
                DatabaseErrorKind::ReadOnlyTransaction,
                "TEST_CODE rollback readonly",
            )),
            commit_error: Box::new(test_database_error(
                DatabaseErrorKind::ClosedConnection,
                "TEST_CODE commit closed",
            )),
        };
        assert_unavailable(
            map_diesel_error(
                nonretryable_availability,
                "TEST_CODE rollback nonretryable",
                DieselErrorContext::TransactionEnvelope,
            ),
            "benchmark_segment_transaction_rollback_commit_unavailable",
            false,
        );

        let integrity = diesel::result::Error::RollbackErrorOnCommit {
            rollback_error: Box::new(test_database_error(
                DatabaseErrorKind::CheckViolation,
                "TEST_CODE rollback check",
            )),
            commit_error: Box::new(test_database_error(
                DatabaseErrorKind::ClosedConnection,
                "TEST_CODE commit closed",
            )),
        };
        let detail = assert_failed_integrity(
            map_diesel_error(
                integrity,
                "TEST_CODE rollback integrity",
                DieselErrorContext::TransactionEnvelope,
            ),
            "benchmark_segment_transaction_rollback_commit_integrity",
        );
        assert!(detail.contains("rollback check"));
        assert!(detail.contains("commit closed"));
    }

    #[test]
    fn public_store_errors_preserve_typed_category_reason_and_retryability() {
        let database = TestDatabase::new();
        match database
            .store()
            .read_exact("TEST_CODE_not_a_manifest_hash")
            .expect_err("TEST_CODE malformed exact hash is integrity failure")
        {
            BenchmarkSegmentStoreError::FailedIntegrity {
                reason_code,
                detail,
            } => {
                assert_eq!(reason_code, "benchmark_manifest_hash_invalid");
                assert!(!detail.is_empty());
            }
            other => panic!("TEST_CODE unexpected malformed-hash category: {other:?}"),
        }

        match database
            .store()
            .read_exact(&"f".repeat(64))
            .expect_err("TEST_CODE absent exact hash is unavailable")
        {
            BenchmarkSegmentStoreError::BenchmarkSegmentUnavailable {
                reason_code,
                retryable,
                detail,
            } => {
                assert_eq!(reason_code, "benchmark_manifest_unavailable");
                assert!(!retryable);
                assert!(!detail.is_empty());
            }
            other => panic!("TEST_CODE unexpected missing-manifest category: {other:?}"),
        }

        let minute = DateTime::parse_from_rfc3339("2099-01-05T09:31:00+08:00")
            .expect("TEST_CODE unavailable calendar minute");
        let request = BenchmarkRequest {
            instrument: "TEST_CODE_sh000300".into(),
            range: BenchmarkRange::Minute1 {
                from: minute,
                to: minute,
            },
        };
        match database
            .store()
            .append(vec![append_for(
                &database,
                request,
                date(2099, 1, 1),
                SegmentState::Provisional,
                vec![minute_bar(minute, 101.0)],
                "TEST_CODE_batch_calendar_unavailable",
            )])
            .expect_err("TEST_CODE immutable calendar absence is unavailable")
        {
            BenchmarkSegmentStoreError::BenchmarkSegmentUnavailable {
                reason_code,
                retryable,
                detail,
            } => {
                assert_eq!(reason_code, "benchmark_trading_calendar_unavailable");
                assert!(retryable);
                assert!(!detail.is_empty());
            }
            other => panic!("TEST_CODE unexpected calendar category: {other:?}"),
        }

        let database = TestDatabase::new();
        let manifest =
            shared_cross_quarter_manifest(&database, "TEST_CODE_batch_typed_count_tamper");
        rewrite_only_manifest_binding(&database, &manifest, |binding| {
            binding.accepted_count = 1;
        });
        match database
            .store()
            .read_exact(&manifest.manifest_hash)
            .expect_err("TEST_CODE association count tamper is integrity failure")
        {
            BenchmarkSegmentStoreError::FailedIntegrity {
                reason_code,
                detail,
            } => {
                assert_eq!(reason_code, "benchmark_manifest_acquisition_count_mismatch");
                assert!(!detail.is_empty());
            }
            other => panic!("TEST_CODE unexpected count-tamper category: {other:?}"),
        }
    }

    #[test]
    fn shortened_association_retention_is_rejected_by_startup_and_exact_read() {
        let database = TestDatabase::new();
        let manifest = shared_cross_quarter_manifest(&database, "TEST_CODE_batch_short_retention");
        let mut conn = database.conn();
        diesel::sql_query("DROP TRIGGER trg_benchmark_manifest_acquisition_no_update")
            .execute(&mut conn)
            .expect("TEST_CODE drop association update trigger");
        diesel::sql_query(
            "UPDATE benchmark_manifest_acquisition SET retention_deadline = created_at",
        )
        .execute(&mut conn)
        .expect("TEST_CODE shorten association retention");
        diesel::sql_query(canonical_update_trigger_definition(
            "benchmark_manifest_acquisition",
        ))
        .execute(&mut conn)
        .expect("TEST_CODE restore association update trigger");
        let startup_error =
            create_schema(&mut conn).expect_err("TEST_CODE startup rejects shortened retention");
        match map_diesel_error(
            startup_error,
            "TEST_CODE startup retention validation",
            DieselErrorContext::TransactionEnvelope,
        ) {
            BenchmarkSegmentStoreError::FailedIntegrity {
                reason_code,
                detail,
            } => {
                assert_eq!(reason_code, "benchmark_retention_invalid");
                assert!(detail.contains("60 calendar months"), "{detail}");
            }
            other => panic!("TEST_CODE unexpected startup retention category: {other:?}"),
        }
        drop(conn);

        match database
            .store()
            .read_exact(&manifest.manifest_hash)
            .expect_err("TEST_CODE exact read rejects shortened association retention")
        {
            BenchmarkSegmentStoreError::FailedIntegrity {
                reason_code,
                detail,
            } => {
                assert_eq!(reason_code, "benchmark_retention_invalid");
                assert!(detail.contains("60 calendar months"), "{detail}");
            }
            other => panic!("TEST_CODE unexpected exact-read retention category: {other:?}"),
        }
    }

    #[test]
    fn every_benchmark_row_and_chain_preserves_fractional_utc_for_sixty_calendar_months() {
        let (database, _) = database_with_one_manifest();
        let mut conn = database.conn();
        for table in [
            "benchmark_segment_revision",
            "benchmark_segment_chain",
            "benchmark_manifest",
            "benchmark_manifest_acquisition",
            "benchmark_manifest_chain",
        ] {
            let window = diesel::sql_query(format!(
                "SELECT created_at, retention_deadline FROM {table} LIMIT 1"
            ))
            .get_result::<TimestampRow>(&mut conn)
            .expect("TEST_CODE load canonical benchmark retention window");
            for (field, value) in [
                ("created_at", window.created_at.as_str()),
                ("retention_deadline", window.retention_deadline.as_str()),
            ] {
                assert!(
                    value.contains('T') && value.contains('.') && value.ends_with('Z'),
                    "TEST_CODE {table}.{field} lost fractional UTC: {value}",
                );
            }
            validate_retention_window(table, &window.created_at, &window.retention_deadline)
                .expect("TEST_CODE canonical benchmark retention is at least 60 months");
        }
    }

    #[test]
    fn sixty_calendar_month_validation_handles_leap_day_exactly() {
        validate_retention_window(
            "TEST_CODE leap boundary",
            "2024-02-29T12:34:56.789Z",
            "2029-02-28T12:34:56.789Z",
        )
        .expect("TEST_CODE exact 60-month leap deadline must pass");
        let error = validate_retention_window(
            "TEST_CODE leap boundary",
            "2024-02-29T12:34:56.789Z",
            "2029-02-28T12:34:56.788Z",
        )
        .expect_err("TEST_CODE one millisecond before exact 60 months must fail");
        assert_eq!(
            map_diesel_error(
                error,
                "TEST_CODE leap retention validation",
                DieselErrorContext::TransactionEnvelope,
            )
            .reason_code(),
            "benchmark_retention_invalid",
        );
    }

    #[test]
    fn startup_rejects_missing_unparseable_short_or_reversed_time_on_every_benchmark_table() {
        let cases = [
            ("", "2031-01-05T15:00:01.000Z", "missing created_at"),
            (
                "not-a-timestamp",
                "2031-01-05T15:00:01.000Z",
                "unparseable created_at",
            ),
            (
                "2026-01-05T15:00:01Z",
                "2031-01-05T15:00:01Z",
                "non-fractional timestamps",
            ),
            (
                "2026-01-05T15:00:01.000Z",
                "2031-01-05T15:00:00.999Z",
                "short retention",
            ),
            (
                "2026-01-05T15:00:01.000Z",
                "2025-01-05T15:00:01.000Z",
                "reversed retention",
            ),
        ];
        for table in [
            "benchmark_segment_revision",
            "benchmark_segment_chain",
            "benchmark_manifest",
            "benchmark_manifest_acquisition",
            "benchmark_manifest_chain",
        ] {
            for (created_at, retention_deadline, case) in cases {
                let (database, _) = database_with_one_manifest();
                tamper_retention(&database, table, created_at, retention_deadline);
                let error = create_schema(&mut database.conn())
                    .expect_err("TEST_CODE startup must reject invalid benchmark time");
                assert_eq!(
                    map_diesel_error(
                        error,
                        "TEST_CODE benchmark startup retention validation",
                        DieselErrorContext::TransactionEnvelope,
                    )
                    .reason_code(),
                    "benchmark_retention_invalid",
                    "TEST_CODE startup accepted {case} on {table}",
                );
            }
        }
    }

    #[test]
    fn sqlite_exclusive_lock_is_retryable_store_unavailability() {
        let database = TestDatabase::new();
        let append = append_for(
            &database,
            daily_request(),
            date(2026, 1, 1),
            SegmentState::Provisional,
            vec![daily_bar(date(2026, 1, 5), 101.0)],
            "TEST_CODE_batch_exclusive_lock",
        );
        let database_url = database.path.to_string_lossy().into_owned();
        let mut locker =
            SqliteConnection::establish(&database_url).expect("TEST_CODE independent locker");
        diesel::sql_query("PRAGMA busy_timeout = 0")
            .execute(&mut locker)
            .expect("TEST_CODE locker busy timeout");
        diesel::sql_query("BEGIN EXCLUSIVE")
            .execute(&mut locker)
            .expect("TEST_CODE begin exclusive lock");

        let error = database
            .store()
            .append(vec![append])
            .expect_err("TEST_CODE SQLite lock must reject append");
        diesel::sql_query("ROLLBACK")
            .execute(&mut locker)
            .expect("TEST_CODE release exclusive lock");
        match error {
            BenchmarkSegmentStoreError::BenchmarkSegmentUnavailable {
                reason_code,
                retryable,
                detail,
            } => {
                assert_eq!(reason_code, "benchmark_segment_storage_unavailable");
                assert!(retryable);
                assert!(!detail.is_empty());
            }
            other => panic!("TEST_CODE unexpected SQLite lock category: {other:?}"),
        }
    }

    #[test]
    fn exact_reader_rejects_missing_manifest_and_compressed_payload_tamper() {
        let database = TestDatabase::new();
        let append = append_for(
            &database,
            daily_request(),
            date(2026, 1, 1),
            SegmentState::Provisional,
            vec![daily_bar(date(2026, 1, 5), 101.25)],
            "TEST_CODE_batch_reader",
        );
        let manifest = database
            .store()
            .append(vec![append])
            .expect("TEST_CODE append");
        let (loaded_manifest, loaded_bars, _) = database
            .store()
            .read_exact(&manifest.manifest_hash)
            .expect("TEST_CODE read exact");
        assert_eq!(loaded_manifest, manifest);
        assert_eq!(loaded_bars, vec![daily_bar(date(2026, 1, 5), 101.25)]);
        assert!(database.store().read_exact(&"f".repeat(64)).is_err());

        let mut conn = database.conn();
        let original = diesel::sql_query(
            "SELECT compressed_payload AS value FROM benchmark_segment_revision LIMIT 1",
        )
        .get_result::<BlobRow>(&mut conn)
        .expect("TEST_CODE compressed payload")
        .value;
        assert!(!original.is_empty());
        diesel::sql_query("DROP TRIGGER trg_benchmark_segment_revision_no_update")
            .execute(&mut conn)
            .expect("TEST_CODE drop immutable trigger");
        diesel::sql_query("UPDATE benchmark_segment_revision SET compressed_payload = X'00010203'")
            .execute(&mut conn)
            .expect("TEST_CODE corrupt compressed payload");
        drop(conn);
        assert!(database
            .store()
            .read_exact(&manifest.manifest_hash)
            .is_err());
    }

    #[test]
    fn exact_reader_revalidates_complete_acquisition_audit_snapshot() {
        let database = TestDatabase::new();
        let manifest = append_readable(
            &database,
            vec![append_for(
                &database,
                daily_request(),
                date(2026, 1, 1),
                SegmentState::Provisional,
                vec![daily_bar(date(2026, 1, 5), 101.0)],
                "TEST_CODE_batch_audit_chain_removed",
            )],
            "TEST_CODE append before audit chain removal",
        );
        let mut conn = database.conn();
        diesel::sql_query("DROP TRIGGER trg_data_acquisition_audit_chain_no_delete")
            .execute(&mut conn)
            .expect("TEST_CODE drop audit-chain retention trigger");
        diesel::sql_query("DELETE FROM data_acquisition_audit_chain")
            .execute(&mut conn)
            .expect("TEST_CODE remove audit-chain evidence");
        drop(conn);
        let error = database
            .store()
            .read_exact(&manifest.manifest_hash)
            .expect_err("TEST_CODE read must reject missing audit-chain evidence");
        assert!(
            error.contains("BR-159 acquisition audit hash chain length mismatch"),
            "{error}"
        );

        let database = TestDatabase::new();
        let manifest = append_readable(
            &database,
            vec![append_for(
                &database,
                daily_request(),
                date(2026, 1, 1),
                SegmentState::Provisional,
                vec![daily_bar(date(2026, 1, 5), 101.0)],
                "TEST_CODE_batch_audit_removed",
            )],
            "TEST_CODE append before complete audit removal",
        );
        let mut conn = database.conn();
        diesel::sql_query("PRAGMA foreign_keys = OFF")
            .execute(&mut conn)
            .expect("TEST_CODE disable foreign keys for corruption injection");
        diesel::sql_query("DROP TRIGGER trg_data_acquisition_audit_chain_no_delete")
            .execute(&mut conn)
            .expect("TEST_CODE drop audit-chain retention trigger");
        diesel::sql_query("DROP TRIGGER trg_data_acquisition_audit_no_delete")
            .execute(&mut conn)
            .expect("TEST_CODE drop audit-row retention trigger");
        diesel::sql_query("DELETE FROM data_acquisition_audit_chain")
            .execute(&mut conn)
            .expect("TEST_CODE remove audit chain before audit row");
        diesel::sql_query("DELETE FROM data_acquisition_audit")
            .execute(&mut conn)
            .expect("TEST_CODE remove audit row");
        drop(conn);
        let error = database
            .store()
            .read_exact(&manifest.manifest_hash)
            .expect_err("TEST_CODE read must reject missing complete audit receipt");
        assert!(
            error.contains("BR-159 acquisition receipt id/hash mismatch"),
            "{error}"
        );

        let database = TestDatabase::new();
        let manifest = append_readable(
            &database,
            vec![append_for(
                &database,
                daily_request(),
                date(2026, 1, 1),
                SegmentState::Provisional,
                vec![daily_bar(date(2026, 1, 5), 101.0)],
                "TEST_CODE_batch_audit_row_tampered",
            )],
            "TEST_CODE append before audit row tamper",
        );
        let mut conn = database.conn();
        diesel::sql_query("DROP TRIGGER trg_data_acquisition_audit_no_update")
            .execute(&mut conn)
            .expect("TEST_CODE drop audit-row immutability trigger");
        diesel::sql_query("UPDATE data_acquisition_audit SET source = 'TEST_CODE_tampered_source'")
            .execute(&mut conn)
            .expect("TEST_CODE tamper audit source");
        drop(conn);
        let error = database
            .store()
            .read_exact(&manifest.manifest_hash)
            .expect_err("TEST_CODE read must reject audit-row chain tamper");
        assert!(
            error.contains("BR-159 acquisition audit hash mismatch"),
            "{error}"
        );
    }

    #[test]
    fn startup_rejects_revision_and_manifest_chain_tamper() {
        for target in ["benchmark_segment_chain", "benchmark_manifest_chain"] {
            let database = TestDatabase::new();
            let append = append_for(
                &database,
                daily_request(),
                date(2026, 1, 1),
                SegmentState::Provisional,
                vec![daily_bar(date(2026, 1, 5), 101.0)],
                &format!("TEST_CODE_batch_tamper_{target}"),
            );
            append_readable(
                &database,
                vec![append],
                "TEST_CODE append before benchmark-chain tamper",
            );
            let mut conn = database.conn();
            diesel::sql_query(format!("DROP TRIGGER trg_{target}_no_update"))
                .execute(&mut conn)
                .expect("TEST_CODE drop chain trigger");
            diesel::sql_query(format!(
                "UPDATE {target} SET record_hash = '{}'",
                "0".repeat(64)
            ))
            .execute(&mut conn)
            .expect("TEST_CODE tamper chain");
            create_schema(&mut conn).expect_err("TEST_CODE startup rejects chain tamper");
        }
    }

    #[test]
    fn stored_rows_retain_codec_hash_evidence_and_five_year_deadline() {
        let database = TestDatabase::new();
        let append = append_for(
            &database,
            daily_request(),
            date(2026, 1, 1),
            SegmentState::Provisional,
            vec![daily_bar(date(2026, 1, 5), 101.0)],
            "TEST_CODE_batch_columns",
        );
        append_readable(&database, vec![append], "TEST_CODE retained-column append");
        let mut conn = database.conn();
        for expression in [
            "codec",
            "CAST(codec_version AS TEXT)",
            "CAST(payload_version AS TEXT)",
            "canonical_hash",
            "compressed_hash",
            "provider",
            "source",
            "observed_at",
            "batch_id",
            "acquisition_record_hash",
        ] {
            let value = diesel::sql_query(format!(
                "SELECT {expression} AS value FROM benchmark_segment_revision LIMIT 1"
            ))
            .get_result::<TextRow>(&mut conn)
            .expect("TEST_CODE retained column")
            .value;
            assert!(!value.trim().is_empty(), "{expression}");
        }
        let retained = diesel::sql_query(
            "SELECT CAST(julianday(retention_deadline) - julianday(created_at) AS TEXT) AS value
             FROM benchmark_segment_revision LIMIT 1",
        )
        .get_result::<TextRow>(&mut conn)
        .expect("TEST_CODE retention interval")
        .value
        .parse::<f64>()
        .expect("TEST_CODE retention number");
        assert!(retained >= 1825.0, "retained days={retained}");
    }

    #[test]
    fn capture_reader_and_explicit_composition_are_public_business_seams() {
        let database = TestDatabase::new();
        let _capture = BenchmarkCapture::new(&database.manager);
        let _reader = BenchmarkReader::new(&database.manager);
        let _composer = BenchmarkSegmentStore::new(&database.manager);
    }

    #[test]
    fn capture_preview_is_side_effect_free_and_commit_is_canonical_and_idempotent() {
        let database = TestDatabase::new();
        let request = BenchmarkRequest {
            instrument: "TEST_CODE_sh000300".into(),
            range: BenchmarkRange::Daily {
                from: date(2026, 3, 31),
                to: date(2026, 4, 1),
            },
        };
        let evidence = evidence("TEST_CODE_capture_cross_quarter");
        let bars = vec![
            daily_bar(date(2026, 3, 31), 101.0),
            daily_bar(date(2026, 4, 1), 102.0),
        ];
        let receipt = database.receipt(&request, &evidence, 2);
        let audited = AuditedBenchmarkBatch {
            batch: GatewayBatch::Available {
                records: bars,
                evidence,
            },
            receipt,
            request_hash: request.canonical_request_hash(),
        };
        let tables = [
            "benchmark_segment_revision",
            "benchmark_segment_chain",
            "benchmark_manifest",
            "benchmark_manifest_acquisition",
            "benchmark_manifest_chain",
        ];
        let mut conn = database.conn();
        let before = tables.map(|table| count(&mut conn, table));
        drop(conn);

        let capture = BenchmarkCapture::new(&database.manager);
        let preview = capture
            .preview_audited_for_test(request.clone(), audited.clone())
            .expect("TEST_CODE admitted preview");
        let mut conn = database.conn();
        assert_eq!(tables.map(|table| count(&mut conn, table)), before);
        drop(conn);

        let manifest = capture.commit(preview).expect("TEST_CODE commit");
        assert_eq!(manifest.segment_hashes.len(), 2);
        let replay = capture
            .preview_audited_for_test(request, audited)
            .and_then(|preview| capture.commit(preview))
            .expect("TEST_CODE idempotent replay");
        assert_eq!(replay, manifest);

        let mut conn = database.conn();
        assert_eq!(count(&mut conn, "benchmark_segment_revision"), 2);
        assert_eq!(count(&mut conn, "benchmark_segment_chain"), 2);
        assert_eq!(count(&mut conn, "benchmark_manifest"), 1);
        assert_eq!(count(&mut conn, "benchmark_manifest_chain"), 1);
    }

    #[test]
    fn capture_preview_rejects_relabelled_request_hash_and_real_test_identity() {
        let database = TestDatabase::new();
        let request = daily_request();
        let evidence = evidence("TEST_CODE_capture_identity_rejection");
        let bars = vec![daily_bar(date(2026, 1, 5), 101.0)];
        let mut audited = AuditedBenchmarkBatch {
            batch: GatewayBatch::Available {
                records: bars,
                evidence: evidence.clone(),
            },
            receipt: database.receipt(&request, &evidence, 1),
            request_hash: request.canonical_request_hash(),
        };
        audited.request_hash = "f".repeat(64);
        let capture = BenchmarkCapture::new(&database.manager);
        assert!(matches!(
            capture.preview_audited_for_test(request.clone(), audited.clone()),
            Err(BenchmarkError::FailedIntegrity {
                code: "benchmark_capture_request_hash_mismatch"
            })
        ));

        let production_identity = BenchmarkRequest {
            instrument: HS300_CANONICAL.into(),
            range: request.range,
        };
        assert!(matches!(
            capture.preview_audited_for_test(production_identity, audited),
            Err(BenchmarkError::FailedIntegrity {
                code: "benchmark_test_preview_requires_test_identity"
            })
        ));
    }

    #[test]
    fn reader_requires_exact_request_and_projects_every_daily_close() {
        let database = TestDatabase::new();
        let request = BenchmarkRequest {
            instrument: "TEST_CODE_sh000300".into(),
            range: BenchmarkRange::Daily {
                from: date(2026, 3, 31),
                to: date(2026, 4, 1),
            },
        };
        let evidence = evidence("TEST_CODE_reader_exact_daily");
        let bars = vec![
            daily_bar(date(2026, 3, 31), 101.0),
            daily_bar(date(2026, 4, 1), 102.0),
        ];
        let audited = AuditedBenchmarkBatch {
            batch: GatewayBatch::Available {
                records: bars.clone(),
                evidence: evidence.clone(),
            },
            receipt: database.receipt(&request, &evidence, 2),
            request_hash: request.canonical_request_hash(),
        };
        let capture = BenchmarkCapture::new(&database.manager);
        let manifest = capture
            .preview_audited_for_test(request.clone(), audited)
            .and_then(|preview| capture.commit(preview))
            .expect("TEST_CODE captured daily manifest");
        let reader = BenchmarkReader::new(&database.manager);
        let snapshot = reader
            .read_exact(&manifest.manifest_hash, &request)
            .expect("TEST_CODE exact reader");
        assert_eq!(snapshot.manifest, manifest);
        assert_eq!(snapshot.bars, bars);

        let series = reader
            .to_daily_series(&snapshot, "TEST_CODE HS300")
            .expect("TEST_CODE complete daily projection");
        assert_eq!(series.name, "TEST_CODE HS300");
        assert_eq!(series.closes.len(), 2);
        assert_eq!(series.closes[&date(2026, 3, 31)], 101.0);
        assert_eq!(series.closes[&date(2026, 4, 1)], 102.0);

        let wrong = BenchmarkRequest {
            instrument: "TEST_CODE_other".into(),
            range: request.range.clone(),
        };
        assert_eq!(
            reader.read_exact(&manifest.manifest_hash, &wrong),
            Err(BenchmarkError::FailedIntegrity {
                code: "benchmark_expected_request_mismatch"
            })
        );
        let wrong_range = BenchmarkRequest {
            instrument: request.instrument.clone(),
            range: BenchmarkRange::Daily {
                from: date(2026, 3, 31),
                to: date(2026, 3, 31),
            },
        };
        assert_eq!(
            reader.read_exact(&manifest.manifest_hash, &wrong_range),
            Err(BenchmarkError::FailedIntegrity {
                code: "benchmark_expected_request_mismatch"
            })
        );
        let minute = DateTime::parse_from_rfc3339("2026-03-31T09:31:00+08:00")
            .expect("TEST_CODE wrong expected minute");
        let wrong_granularity = BenchmarkRequest {
            instrument: request.instrument.clone(),
            range: BenchmarkRange::Minute1 {
                from: minute,
                to: minute,
            },
        };
        assert_eq!(
            reader.read_exact(&manifest.manifest_hash, &wrong_granularity),
            Err(BenchmarkError::FailedIntegrity {
                code: "benchmark_expected_request_mismatch"
            })
        );

        let mut tampered = snapshot;
        tampered.bars.pop();
        assert!(matches!(
            reader.to_daily_series(&tampered, "TEST_CODE HS300"),
            Err(BenchmarkError::FailedIntegrity {
                code: "benchmark_snapshot_tampered"
            })
        ));
    }

    #[test]
    fn reader_returns_ordered_persisted_multi_receipt_evidence_and_detects_caller_tamper() {
        let database = TestDatabase::new();
        let request = BenchmarkRequest {
            instrument: "TEST_CODE_sh000300".into(),
            range: BenchmarkRange::Daily {
                from: date(2026, 3, 31),
                to: date(2026, 4, 1),
            },
        };
        let q1_evidence = BatchEvidence {
            source: "TEST_CODE_magic_tdx_index@revision-q1".into(),
            source_at: Some("2026-03-31T15:00:00+08:00".into()),
            observed_at: "2026-03-31T15:00:01+08:00".into(),
            ..evidence("TEST_CODE_reader_evidence_q1")
        };
        let q2_evidence = BatchEvidence {
            provider: ProviderId::Tencent,
            source: "TEST_CODE_tencent_index@revision-q2".into(),
            source_at: Some("2026-04-01T15:00:00+08:00".into()),
            observed_at: "2026-04-01T15:00:01+08:00".into(),
            ..evidence("TEST_CODE_reader_evidence_q2")
        };
        let manifest = database
            .store()
            .append(vec![
                append_with_evidence(
                    &database,
                    request.clone(),
                    date(2026, 1, 1),
                    SegmentState::Provisional,
                    vec![daily_bar(date(2026, 3, 31), 101.0)],
                    q1_evidence.clone(),
                ),
                append_with_evidence(
                    &database,
                    request.clone(),
                    date(2026, 4, 1),
                    SegmentState::Provisional,
                    vec![daily_bar(date(2026, 4, 1), 102.0)],
                    q2_evidence.clone(),
                ),
            ])
            .expect("TEST_CODE multi-receipt manifest");

        let reader = BenchmarkReader::new(&database.manager);
        let snapshot = reader
            .read_exact(&manifest.manifest_hash, &request)
            .expect("TEST_CODE exact snapshot with persisted evidence");
        assert_eq!(snapshot.evidence, vec![q1_evidence, q2_evidence]);

        let mut tampered = snapshot;
        tampered.evidence[0].source = "TEST_CODE_caller_relabelled_revision".into();
        assert!(matches!(
            reader.to_daily_series(&tampered, "TEST_CODE HS300"),
            Err(BenchmarkError::FailedIntegrity {
                code: "benchmark_snapshot_tampered"
            })
        ));
    }

    #[test]
    fn reader_rejects_persisted_manifest_provider_and_source_tamper() {
        let request = BenchmarkRequest {
            instrument: "TEST_CODE_sh000300".into(),
            range: BenchmarkRange::Daily {
                from: date(2026, 3, 31),
                to: date(2026, 4, 1),
            },
        };

        let provider_database = TestDatabase::new();
        let provider_manifest =
            shared_cross_quarter_manifest(&provider_database, "TEST_CODE_reader_provider_tamper");
        rewrite_only_manifest_binding(&provider_database, &provider_manifest, |binding| {
            binding.provider = "TEST_CODE_unknown_provider".into();
        });
        assert!(matches!(
            BenchmarkReader::new(&provider_database.manager)
                .read_exact(&provider_manifest.manifest_hash, &request),
            Err(BenchmarkError::FailedIntegrity {
                code: "benchmark_manifest_acquisition_audit_invalid"
            })
        ));

        let source_database = TestDatabase::new();
        let source_manifest =
            shared_cross_quarter_manifest(&source_database, "TEST_CODE_reader_source_tamper");
        rewrite_only_manifest_binding(&source_database, &source_manifest, |binding| {
            binding.source = "TEST_CODE_relabelled_source@revision".into();
        });
        assert!(matches!(
            BenchmarkReader::new(&source_database.manager)
                .read_exact(&source_manifest.manifest_hash, &request),
            Err(BenchmarkError::FailedIntegrity {
                code: "benchmark_manifest_acquisition_audit_invalid"
            })
        ));
    }

    #[test]
    fn capture_seals_only_complete_ended_daily_quarters_and_never_one_day_minute() {
        let daily_database = TestDatabase::new();
        let daily_request = q1_2026_request();
        let daily_bars = q1_2026_bars(108.0);
        let daily_evidence =
            evidence_observed_at("TEST_CODE_capture_sealed_q1", "2026-04-01T00:00:01+08:00");
        let daily_audited = AuditedBenchmarkBatch {
            batch: GatewayBatch::Available {
                records: daily_bars.clone(),
                evidence: daily_evidence.clone(),
            },
            receipt: daily_database.receipt(
                &daily_request,
                &daily_evidence,
                daily_bars.len() as i64,
            ),
            request_hash: daily_request.canonical_request_hash(),
        };
        let daily_capture = BenchmarkCapture::new(&daily_database.manager);
        daily_capture
            .preview_audited_for_test(daily_request, daily_audited)
            .and_then(|preview| daily_capture.commit(preview))
            .expect("TEST_CODE complete ended quarter seals");
        let daily_state =
            diesel::sql_query("SELECT state AS value FROM benchmark_segment_revision LIMIT 1")
                .get_result::<TextRow>(&mut daily_database.conn())
                .expect("TEST_CODE daily state")
                .value;
        assert_eq!(daily_state, "Sealed");

        let minute_database = TestDatabase::new();
        let from = DateTime::parse_from_rfc3339("2026-01-05T09:31:00+08:00")
            .expect("TEST_CODE minute from");
        let to =
            DateTime::parse_from_rfc3339("2026-01-05T09:32:00+08:00").expect("TEST_CODE minute to");
        let minute_request = BenchmarkRequest {
            instrument: "TEST_CODE_sh000300".into(),
            range: BenchmarkRange::Minute1 { from, to },
        };
        let minute_bars = vec![minute_bar(from, 101.0), minute_bar(to, 102.0)];
        let minute_evidence = evidence_observed_at(
            "TEST_CODE_capture_minute_provisional",
            "2026-04-01T00:00:01+08:00",
        );
        let minute_audited = AuditedBenchmarkBatch {
            batch: GatewayBatch::Available {
                records: minute_bars,
                evidence: minute_evidence.clone(),
            },
            receipt: minute_database.receipt(&minute_request, &minute_evidence, 2),
            request_hash: minute_request.canonical_request_hash(),
        };
        let minute_capture = BenchmarkCapture::new(&minute_database.manager);
        let minute_manifest = minute_capture
            .preview_audited_for_test(minute_request.clone(), minute_audited)
            .and_then(|preview| minute_capture.commit(preview))
            .expect("TEST_CODE one-day minute remains provisional");
        let minute_state =
            diesel::sql_query("SELECT state AS value FROM benchmark_segment_revision LIMIT 1")
                .get_result::<TextRow>(&mut minute_database.conn())
                .expect("TEST_CODE minute state")
                .value;
        assert_eq!(minute_state, "Provisional");
        let minute_snapshot = BenchmarkReader::new(&minute_database.manager)
            .read_exact(&minute_manifest.manifest_hash, &minute_request)
            .expect("TEST_CODE minute exact read");
        assert!(matches!(
            BenchmarkReader::new(&minute_database.manager)
                .to_daily_series(&minute_snapshot, "TEST_CODE minute"),
            Err(BenchmarkError::FailedIntegrity {
                code: "benchmark_daily_projection_granularity_mismatch"
            })
        ));
    }

    #[test]
    fn reader_preserves_typed_missing_and_integrity_store_failures() {
        let database = TestDatabase::new();
        let reader = BenchmarkReader::new(&database.manager);
        let request = daily_request();
        assert_eq!(
            reader.read_exact("TEST_CODE_not_a_hash", &request),
            Err(BenchmarkError::FailedIntegrity {
                code: "benchmark_manifest_hash_invalid"
            })
        );
        assert_eq!(
            reader.read_exact(&"f".repeat(64), &request),
            Err(BenchmarkError::Unavailable {
                code: "benchmark_manifest_unavailable",
                retryable: false
            })
        );
    }
}
