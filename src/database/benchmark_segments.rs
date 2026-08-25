//! BR-251 immutable natural-quarter benchmark segment and manifest store.

use std::collections::{HashMap, HashSet};
use std::io::Cursor;

use chrono::{DateTime, Datelike, Duration, FixedOffset, Months, NaiveDate, NaiveDateTime, Utc};
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

const SEGMENT_CHAIN_GENESIS: &str = "BR251_BENCHMARK_SEGMENT_CHAIN_GENESIS_V1";
const MANIFEST_CHAIN_GENESIS: &str = "BR251_BENCHMARK_MANIFEST_CHAIN_GENESIS_V1";
const PAYLOAD_SCHEMA: &str = "BR251_BENCHMARK_SEGMENT_PAYLOAD_V1";
const MANIFEST_ACQUISITION_SCHEMA: &str = "BR251_BENCHMARK_MANIFEST_ACQUISITION_V1";
const CODEC: &str = "zstd";
const CODEC_VERSION: i32 = 1;
const PAYLOAD_VERSION: i32 = 1;

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
        "SELECT segment_revision_id, previous_hash, record_hash
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
    if let Ok(value) = DateTime::parse_from_rfc3339(value) {
        return Ok(value.with_timezone(&Utc));
    }
    for format in ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S%.fZ"] {
        if let Ok(value) = NaiveDateTime::parse_from_str(value, format) {
            return Ok(value.and_utc());
        }
    }
    Err(format!("invalid persisted UTC timestamp: {value}"))
}

fn validate_manifest_acquisition_retention(
    row: &PersistedManifestAcquisition,
) -> diesel::QueryResult<()> {
    let created_at = parse_persisted_utc_timestamp(&row.created_at).map_err(|detail| {
        integrity_store_error(
            "benchmark_manifest_acquisition_retention_invalid",
            format!("BR-251 manifest acquisition created_at is invalid: {detail}"),
        )
    })?;
    let retention_deadline =
        parse_persisted_utc_timestamp(&row.retention_deadline).map_err(|detail| {
            integrity_store_error(
                "benchmark_manifest_acquisition_retention_invalid",
                format!("BR-251 manifest acquisition retention_deadline is invalid: {detail}"),
            )
        })?;
    let minimum_deadline = created_at
        .checked_add_months(Months::new(60))
        .ok_or_else(|| {
            integrity_store_error(
                "benchmark_manifest_acquisition_retention_invalid",
                "BR-251 manifest acquisition five-year deadline overflows",
            )
        })?;
    if retention_deadline < minimum_deadline {
        return Err(integrity_store_error(
            "benchmark_manifest_acquisition_retention_invalid",
            "BR-251 manifest acquisition retention must be at least five years",
        ));
    }
    Ok(())
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
        if binding.schema != MANIFEST_ACQUISITION_SCHEMA
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
        "SELECT manifest_id, previous_hash, record_hash
         FROM benchmark_manifest_chain ORDER BY manifest_id ASC",
    )
    .load(conn)
}

fn segment_chain_hash(previous_hash: &str, row: &PersistedSegment) -> diesel::QueryResult<String> {
    let payload = serde_json::to_vec(row)
        .map_err(|error| store_error(format!("BR-251 serialize segment row: {error}")))?;
    Ok(hash_with_domain(
        b"BR251_BENCHMARK_SEGMENT_CHAIN_V1",
        &[previous_hash.as_bytes(), &payload],
    ))
}

fn manifest_chain_hash(
    previous_hash: &str,
    row: &PersistedManifest,
    associations: &[PersistedManifestAcquisition],
) -> diesel::QueryResult<String> {
    let payload = serde_json::to_vec(row)
        .map_err(|error| store_error(format!("BR-251 serialize manifest row: {error}")))?;
    let acquisition = serde_json::to_vec(associations).map_err(|error| {
        store_error(format!(
            "BR-251 serialize persisted manifest acquisition rows: {error}"
        ))
    })?;
    Ok(hash_with_domain(
        b"BR251_BENCHMARK_MANIFEST_CHAIN_V1",
        &[previous_hash.as_bytes(), &payload, &acquisition],
    ))
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
        let expected_hash = segment_chain_hash(&previous, row)?;
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
        let expected_hash = manifest_chain_hash(&previous, row, &associations)?;
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
        if represented_count != binding.accepted_count {
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

fn request_from_manifest(manifest: &BenchmarkManifestRef) -> Result<BenchmarkRequest, String> {
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
    let record_hash = segment_chain_hash(previous_chain_hash, &row)?;
    let chain_inserted = diesel::sql_query(
        "INSERT INTO benchmark_segment_chain
         (segment_revision_id, previous_hash, record_hash) VALUES (?, ?, ?)",
    )
    .bind::<BigInt, _>(row.id)
    .bind::<Text, _>(previous_chain_hash)
    .bind::<Text, _>(&record_hash)
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
    let record_hash = manifest_chain_hash(previous_chain_hash, &row, &associations)?;
    let chain_inserted = diesel::sql_query(
        "INSERT INTO benchmark_manifest_chain (manifest_id, previous_hash, record_hash)
         VALUES (?, ?, ?)",
    )
    .bind::<BigInt, _>(row.id)
    .bind::<Text, _>(previous_chain_hash)
    .bind::<Text, _>(&record_hash)
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
) -> diesel::QueryResult<(BenchmarkManifestRef, Vec<BenchmarkBar>)> {
    if !is_lower_hex_hash(manifest_hash) {
        return Err(integrity_store_error(
            "benchmark_manifest_hash_invalid",
            "BR-251 exact manifest hash must be 64 lowercase hex characters",
        ));
    }
    super::data_acquisition_audit::validate_data_acquisition_audit_chain(conn)?;
    validate_segment_chain(conn)?;
    validate_manifest_chain(conn)?;
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
    for binding in &acquisition_bindings {
        if binding.request_hash != request_hash {
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
    }
    Ok((manifest, bars))
}

impl<'a> BenchmarkSegmentStore<'a> {
    pub fn new(database: &'a DatabaseManager) -> Self {
        Self { database }
    }

    pub fn append(
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
                super::data_acquisition_audit::validate_data_acquisition_audit_chain(conn)?;
                let mut segment_chain_tail = validate_segment_chain(conn)?;
                let manifest_chain_tail = validate_manifest_chain(conn)?;
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
                let (retained_manifest, retained_payload) =
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

    pub fn read_exact(
        &self,
        manifest_hash: &str,
    ) -> Result<(BenchmarkManifestRef, Vec<BenchmarkBar>), BenchmarkSegmentStoreError> {
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
            retention_deadline TEXT NOT NULL DEFAULT (datetime('now', '+5 years')),
            UNIQUE(instrument, granularity, quarter_start, state, canonical_hash),
            FOREIGN KEY(acquisition_audit_id) REFERENCES data_acquisition_audit(id),
            FOREIGN KEY(predecessor_segment_hash) REFERENCES benchmark_segment_revision(segment_hash)
        )",
    )
    .execute(conn)?;
    diesel::sql_query(
        "CREATE TRIGGER IF NOT EXISTS trg_benchmark_segment_revision_no_update
         BEFORE UPDATE ON benchmark_segment_revision
         BEGIN SELECT RAISE(ABORT, 'BR-251 benchmark segment revision is immutable'); END",
    )
    .execute(conn)?;
    diesel::sql_query(
        "CREATE TRIGGER IF NOT EXISTS trg_benchmark_segment_revision_no_delete
         BEFORE DELETE ON benchmark_segment_revision
         BEGIN SELECT RAISE(ABORT, 'BR-251 benchmark segment retention is at least five years'); END",
    )
    .execute(conn)?;
    diesel::sql_query(
        "CREATE TABLE IF NOT EXISTS benchmark_segment_chain (
            segment_revision_id INTEGER PRIMARY KEY NOT NULL,
            previous_hash TEXT NOT NULL,
            record_hash TEXT NOT NULL UNIQUE,
            created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
            retention_deadline TEXT NOT NULL DEFAULT (datetime('now', '+5 years')),
            FOREIGN KEY(segment_revision_id) REFERENCES benchmark_segment_revision(id)
        )",
    )
    .execute(conn)?;
    diesel::sql_query(
        "CREATE TRIGGER IF NOT EXISTS trg_benchmark_segment_chain_no_update
         BEFORE UPDATE ON benchmark_segment_chain
         BEGIN SELECT RAISE(ABORT, 'BR-251 benchmark segment chain is immutable'); END",
    )
    .execute(conn)?;
    diesel::sql_query(
        "CREATE TRIGGER IF NOT EXISTS trg_benchmark_segment_chain_no_delete
         BEFORE DELETE ON benchmark_segment_chain
         BEGIN SELECT RAISE(ABORT, 'BR-251 benchmark segment chain retention is at least five years'); END",
    )
    .execute(conn)?;
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
            retention_deadline TEXT NOT NULL DEFAULT (datetime('now', '+5 years'))
        )",
    )
    .execute(conn)?;
    diesel::sql_query(
        "CREATE TRIGGER IF NOT EXISTS trg_benchmark_manifest_no_update
         BEFORE UPDATE ON benchmark_manifest
         BEGIN SELECT RAISE(ABORT, 'BR-251 benchmark manifest is immutable'); END",
    )
    .execute(conn)?;
    diesel::sql_query(
        "CREATE TRIGGER IF NOT EXISTS trg_benchmark_manifest_no_delete
         BEFORE DELETE ON benchmark_manifest
         BEGIN SELECT RAISE(ABORT, 'BR-251 benchmark manifest retention is at least five years'); END",
    )
    .execute(conn)?;
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
    diesel::sql_query(
        "CREATE TRIGGER IF NOT EXISTS trg_benchmark_manifest_acquisition_no_update
         BEFORE UPDATE ON benchmark_manifest_acquisition
         BEGIN SELECT RAISE(ABORT, 'BR-251 benchmark manifest acquisition is immutable'); END",
    )
    .execute(conn)?;
    diesel::sql_query(
        "CREATE TRIGGER IF NOT EXISTS trg_benchmark_manifest_acquisition_no_delete
         BEFORE DELETE ON benchmark_manifest_acquisition
         BEGIN SELECT RAISE(ABORT, 'BR-251 benchmark manifest acquisition retention is at least five years'); END",
    )
    .execute(conn)?;
    diesel::sql_query(
        "CREATE TABLE IF NOT EXISTS benchmark_manifest_chain (
            manifest_id INTEGER PRIMARY KEY NOT NULL,
            previous_hash TEXT NOT NULL,
            record_hash TEXT NOT NULL UNIQUE,
            created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
            retention_deadline TEXT NOT NULL DEFAULT (datetime('now', '+5 years')),
            FOREIGN KEY(manifest_id) REFERENCES benchmark_manifest(id)
        )",
    )
    .execute(conn)?;
    diesel::sql_query(
        "CREATE TRIGGER IF NOT EXISTS trg_benchmark_manifest_chain_no_update
         BEFORE UPDATE ON benchmark_manifest_chain
         BEGIN SELECT RAISE(ABORT, 'BR-251 benchmark manifest chain is immutable'); END",
    )
    .execute(conn)?;
    diesel::sql_query(
        "CREATE TRIGGER IF NOT EXISTS trg_benchmark_manifest_chain_no_delete
         BEFORE DELETE ON benchmark_manifest_chain
         BEGIN SELECT RAISE(ABORT, 'BR-251 benchmark manifest chain retention is at least five years'); END",
    )
    .execute(conn)?;
    validate_segment_chain(conn)?;
    validate_manifest_chain(conn)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use chrono::{DateTime, NaiveDate};
    use diesel::sql_types::{BigInt, Binary, Nullable, Text};

    use super::*;
    use crate::data_gateway::{BenchmarkBarTime, BenchmarkRange};
    use crate::database::data_acquisition_audit::DataAcquisitionAuditRecord;
    use crate::magic_compat::ProviderId;

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
            self.manager
                .record_data_acquisition(&DataAcquisitionAuditRecord {
                    capability: "BenchmarkBars",
                    provider: "Tdx",
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
        let (retained, _) = database
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
    }

    fn count(conn: &mut SqliteConnection, table: &str) -> i64 {
        diesel::sql_query(format!("SELECT COUNT(*) AS count FROM {table}"))
            .get_result::<CountRow>(conn)
            .expect("TEST_CODE count")
            .count
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
        let (retained_original, original_bars) = database
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
        let (retained_original, original_bars) = database
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
                assert_eq!(reason_code, "benchmark_manifest_acquisition_missing");
                assert!(detail.contains("manifest acquisition association is missing"));
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
        let (_, bars) = database
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
                assert_eq!(reason_code, "benchmark_manifest_acquisition_missing");
                assert!(detail.contains("manifest acquisition association is missing"));
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
                assert_eq!(
                    reason_code,
                    "benchmark_manifest_acquisition_retention_invalid"
                );
                assert!(detail.contains("at least five years"), "{detail}");
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
                assert_eq!(
                    reason_code,
                    "benchmark_manifest_acquisition_retention_invalid"
                );
                assert!(detail.contains("at least five years"), "{detail}");
            }
            other => panic!("TEST_CODE unexpected exact-read retention category: {other:?}"),
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
        let (loaded_manifest, loaded_bars) = database
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
}
