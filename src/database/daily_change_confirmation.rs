//! BR-171 immutable manual-confirmation ledger for adjacent daily-close changes.
//!
//! A confirmation is valid only for the exact objective evidence in
//! [`DailyChangeConfirmationQuery`]. Facts and their SHA-256 chain are appended
//! in one SQLite `IMMEDIATE` transaction. There is intentionally no update,
//! delete, fuzzy lookup, or retention-shortening API.

use chrono::{DateTime, FixedOffset, NaiveDate, SecondsFormat};
use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Integer, Nullable, Text};
use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

use super::DatabaseManager;

const SCHEMA_VERSION: i32 = 1;
const SCHEMA_VERSION_V2: i32 = 2;
const CHAIN_GENESIS: &str = "BR171_DAILY_CHANGE_CONFIRMATION_GENESIS_V1";
const CHAIN_GENESIS_V2: &str = "BR171_DAILY_CHANGE_CONFIRMATION_GENESIS_V2";
const QUERY_HASH_DOMAIN: &[u8] = b"BR171_DAILY_CHANGE_QUERY_V1\0";
const CONTENT_HASH_DOMAIN: &[u8] = b"BR171_DAILY_CHANGE_CONFIRMATION_CONTENT_V1\0";
const CHAIN_HASH_DOMAIN: &[u8] = b"BR171_DAILY_CHANGE_CONFIRMATION_CHAIN_V1\0";
const STABLE_FACT_HASH_DOMAIN_V2: &[u8] = b"BR171_DAILY_CHANGE_STABLE_FACT_V2\0";
const REVIEW_TOKEN_HASH_DOMAIN_V2: &[u8] = b"BR171_OPERATOR_REVIEW_FACT_V2\0";
const ALIAS_CONTENT_HASH_DOMAIN_V2: &[u8] = b"BR171_DAILY_CHANGE_CONFIRMATION_ALIAS_V2\0";
const CHAIN_HASH_DOMAIN_V2: &[u8] = b"BR171_DAILY_CHANGE_CONFIRMATION_CHAIN_V2\0";

pub const DAILY_CHANGE_CONFIRMATION_MIN_RETENTION_YEARS: u32 = 5;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS daily_change_confirmation (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    schema_version INTEGER NOT NULL CHECK (schema_version = 1),
    confirmation_id TEXT NOT NULL UNIQUE,
    query_identity_hash TEXT NOT NULL UNIQUE CHECK (length(query_identity_hash) = 64),
    content_hash TEXT NOT NULL UNIQUE CHECK (length(content_hash) = 64),
    code TEXT NOT NULL CHECK (length(trim(code)) > 0),
    previous_date TEXT NOT NULL,
    "current_date" TEXT NOT NULL,
    previous_close TEXT NOT NULL,
    current_close TEXT NOT NULL,
    calculated_pct TEXT NOT NULL,
    daily_provider TEXT NOT NULL CHECK (length(trim(daily_provider)) > 0),
    daily_source TEXT NOT NULL CHECK (length(trim(daily_source)) > 0),
    daily_batch_id TEXT NOT NULL CHECK (length(trim(daily_batch_id)) > 0),
    lifecycle_provider TEXT NOT NULL CHECK (length(trim(lifecycle_provider)) > 0),
    lifecycle_batch_id TEXT NOT NULL CHECK (length(trim(lifecycle_batch_id)) > 0),
    listing_date TEXT,
    corporate_action_identity TEXT,
    operator_identity TEXT NOT NULL CHECK (length(trim(operator_identity)) > 0),
    reason TEXT NOT NULL CHECK (length(trim(reason)) > 0),
    confirmed_at TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX IF NOT EXISTS idx_daily_change_confirmation_exact
    ON daily_change_confirmation(
        code, previous_date, "current_date", daily_batch_id, lifecycle_batch_id
    );

CREATE TABLE IF NOT EXISTS daily_change_confirmation_chain (
    confirmation_row_id INTEGER PRIMARY KEY NOT NULL,
    previous_hash TEXT NOT NULL,
    record_hash TEXT NOT NULL UNIQUE CHECK (length(record_hash) = 64),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    FOREIGN KEY(confirmation_row_id) REFERENCES daily_change_confirmation(id)
);

CREATE TRIGGER IF NOT EXISTS trg_daily_change_confirmation_no_update
BEFORE UPDATE ON daily_change_confirmation
BEGIN
    SELECT RAISE(
        ABORT,
        'BR-171 daily-change confirmation is immutable and retained for at least five years'
    );
END;
CREATE TRIGGER IF NOT EXISTS trg_daily_change_confirmation_no_delete
BEFORE DELETE ON daily_change_confirmation
BEGIN
    SELECT RAISE(
        ABORT,
        'BR-171 daily-change confirmation is immutable and retained for at least five years'
    );
END;
CREATE TRIGGER IF NOT EXISTS trg_daily_change_confirmation_chain_no_update
BEFORE UPDATE ON daily_change_confirmation_chain
BEGIN
    SELECT RAISE(
        ABORT,
        'BR-171 daily-change confirmation hash chain is immutable and retained for at least five years'
    );
END;
CREATE TRIGGER IF NOT EXISTS trg_daily_change_confirmation_chain_no_delete
BEFORE DELETE ON daily_change_confirmation_chain
BEGIN
    SELECT RAISE(
        ABORT,
        'BR-171 daily-change confirmation hash chain is immutable and retained for at least five years'
    );
END;

CREATE TABLE IF NOT EXISTS daily_change_confirmation_v2 (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    schema_version INTEGER NOT NULL CHECK (schema_version = 2),
    stable_fact_identity_hash TEXT NOT NULL UNIQUE
        CHECK (length(stable_fact_identity_hash) = 64),
    v1_confirmation_row_id INTEGER NOT NULL UNIQUE,
    v1_confirmation_id TEXT NOT NULL UNIQUE,
    reviewed_daily_batch_id TEXT NOT NULL
        CHECK (length(trim(reviewed_daily_batch_id)) > 0),
    reviewed_lifecycle_batch_id TEXT NOT NULL
        CHECK (length(trim(reviewed_lifecycle_batch_id)) > 0),
    content_hash TEXT NOT NULL UNIQUE CHECK (length(content_hash) = 64),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    FOREIGN KEY(v1_confirmation_row_id) REFERENCES daily_change_confirmation(id)
);

CREATE TABLE IF NOT EXISTS daily_change_confirmation_chain_v2 (
    confirmation_row_id INTEGER PRIMARY KEY NOT NULL,
    previous_hash TEXT NOT NULL,
    record_hash TEXT NOT NULL UNIQUE CHECK (length(record_hash) = 64),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    FOREIGN KEY(confirmation_row_id) REFERENCES daily_change_confirmation_v2(id)
);

CREATE TRIGGER IF NOT EXISTS trg_daily_change_confirmation_v2_no_update
BEFORE UPDATE ON daily_change_confirmation_v2
BEGIN
    SELECT RAISE(
        ABORT,
        'BR-171 v2 stable confirmation is immutable and retained for at least five years'
    );
END;
CREATE TRIGGER IF NOT EXISTS trg_daily_change_confirmation_v2_no_delete
BEFORE DELETE ON daily_change_confirmation_v2
BEGIN
    SELECT RAISE(
        ABORT,
        'BR-171 v2 stable confirmation is immutable and retained for at least five years'
    );
END;
CREATE TRIGGER IF NOT EXISTS trg_daily_change_confirmation_chain_v2_no_update
BEFORE UPDATE ON daily_change_confirmation_chain_v2
BEGIN
    SELECT RAISE(
        ABORT,
        'BR-171 v2 stable confirmation hash chain is immutable and retained for at least five years'
    );
END;
CREATE TRIGGER IF NOT EXISTS trg_daily_change_confirmation_chain_v2_no_delete
BEFORE DELETE ON daily_change_confirmation_chain_v2
BEGIN
    SELECT RAISE(
        ABORT,
        'BR-171 v2 stable confirmation hash chain is immutable and retained for at least five years'
    );
END;
"#;

/// Objective evidence that must match exactly before a large daily move is admitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DailyChangeConfirmationQuery {
    pub code: String,
    pub previous_date: NaiveDate,
    pub current_date: NaiveDate,
    pub previous_close: String,
    pub current_close: String,
    pub calculated_pct: String,
    pub daily_provider: String,
    pub daily_source: String,
    pub daily_batch_id: String,
    pub lifecycle_provider: String,
    pub lifecycle_batch_id: String,
    pub listing_date: Option<NaiveDate>,
    pub corporate_action_identity: Option<String>,
}

/// Append input. Operator decision fields are part of the immutable content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DailyChangeConfirmationInput {
    pub query: DailyChangeConfirmationQuery,
    pub operator_identity: String,
    pub reason: String,
    pub confirmed_at: DateTime<FixedOffset>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DailyChangeConfirmationReceipt {
    pub confirmation_id: String,
    pub query_identity_hash: String,
    pub record_hash: String,
    pub inserted: bool,
}

#[derive(Debug, Error)]
pub enum DailyChangeConfirmationError {
    #[error("BR-171 confirmation conflict for query identity {query_identity_hash}")]
    Conflict { query_identity_hash: String },
    #[error("BR-171 invalid confirmation input: {0}")]
    InvalidInput(String),
    #[error("BR-171 confirmation audit failure: {0}")]
    Audit(String),
    #[error("BR-171 confirmation database error: {0}")]
    Database(#[from] diesel::result::Error),
}

pub type DailyChangeConfirmationResult<T> = Result<T, DailyChangeConfirmationError>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CanonicalQuery {
    code: String,
    previous_date: String,
    current_date: String,
    previous_close: String,
    current_close: String,
    calculated_pct: String,
    daily_provider: String,
    daily_source: String,
    daily_batch_id: String,
    lifecycle_provider: String,
    lifecycle_batch_id: String,
    listing_date: Option<String>,
    corporate_action_identity: Option<String>,
}

/// Objective provider facts that survive a fresh acquisition of the same data.
/// Concrete batch IDs remain mandatory in `CanonicalQuery` and in the audit
/// rows, but are not part of this v2 identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CanonicalStableFactV2 {
    code: String,
    previous_date: String,
    current_date: String,
    previous_close: String,
    current_close: String,
    calculated_pct: String,
    daily_provider: String,
    daily_source: String,
    lifecycle_provider: String,
    listing_date: Option<String>,
    corporate_action_identity: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CanonicalConfirmation {
    query: CanonicalQuery,
    operator_identity: String,
    reason: String,
    confirmed_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CanonicalStableAliasV2 {
    stable_fact_identity_hash: String,
    v1_confirmation_id: String,
    reviewed_daily_batch_id: String,
    reviewed_lifecycle_batch_id: String,
}

#[derive(Debug, QueryableByName, Serialize)]
struct PersistedConfirmationRow {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = Integer)]
    schema_version: i32,
    #[diesel(sql_type = Text)]
    confirmation_id: String,
    #[diesel(sql_type = Text)]
    query_identity_hash: String,
    #[diesel(sql_type = Text)]
    content_hash: String,
    #[diesel(sql_type = Text)]
    code: String,
    #[diesel(sql_type = Text)]
    previous_date: String,
    #[diesel(sql_type = Text)]
    current_date: String,
    #[diesel(sql_type = Text)]
    previous_close: String,
    #[diesel(sql_type = Text)]
    current_close: String,
    #[diesel(sql_type = Text)]
    calculated_pct: String,
    #[diesel(sql_type = Text)]
    daily_provider: String,
    #[diesel(sql_type = Text)]
    daily_source: String,
    #[diesel(sql_type = Text)]
    daily_batch_id: String,
    #[diesel(sql_type = Text)]
    lifecycle_provider: String,
    #[diesel(sql_type = Text)]
    lifecycle_batch_id: String,
    #[diesel(sql_type = Nullable<Text>)]
    listing_date: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    corporate_action_identity: Option<String>,
    #[diesel(sql_type = Text)]
    operator_identity: String,
    #[diesel(sql_type = Text)]
    reason: String,
    #[diesel(sql_type = Text)]
    confirmed_at: String,
    #[diesel(sql_type = Text)]
    created_at: String,
}

#[derive(Debug, QueryableByName)]
struct ChainRow {
    #[diesel(sql_type = BigInt)]
    confirmation_row_id: i64,
    #[diesel(sql_type = Text)]
    previous_hash: String,
    #[diesel(sql_type = Text)]
    record_hash: String,
}

#[derive(Debug, QueryableByName, Serialize)]
struct PersistedStableAliasV2Row {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = Integer)]
    schema_version: i32,
    #[diesel(sql_type = Text)]
    stable_fact_identity_hash: String,
    #[diesel(sql_type = BigInt)]
    v1_confirmation_row_id: i64,
    #[diesel(sql_type = Text)]
    v1_confirmation_id: String,
    #[diesel(sql_type = Text)]
    reviewed_daily_batch_id: String,
    #[diesel(sql_type = Text)]
    reviewed_lifecycle_batch_id: String,
    #[diesel(sql_type = Text)]
    content_hash: String,
    #[diesel(sql_type = Text)]
    created_at: String,
}

fn invalid(message: impl Into<String>) -> DailyChangeConfirmationError {
    DailyChangeConfirmationError::InvalidInput(message.into())
}

fn audit(message: impl Into<String>) -> DailyChangeConfirmationError {
    DailyChangeConfirmationError::Audit(message.into())
}

fn validate_exact_text(field: &str, value: &str) -> DailyChangeConfirmationResult<()> {
    if value.is_empty() || value.trim() != value {
        return Err(invalid(format!(
            "{field} must be non-empty and contain no surrounding whitespace"
        )));
    }
    Ok(())
}

#[cfg(not(test))]
fn validate_code(code: &str) -> DailyChangeConfirmationResult<()> {
    if code.len() != 6 || !code.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid(format!(
            "code must be one canonical six-digit security code, got {code:?}"
        )));
    }
    Ok(())
}

#[cfg(test)]
fn validate_code(code: &str) -> DailyChangeConfirmationResult<()> {
    let suffix = code.strip_prefix("TEST_CODE_").ok_or_else(|| {
        invalid(format!(
            "test confirmation code must use TEST_CODE_: {code:?}"
        ))
    })?;
    if suffix.len() != 6 || !suffix.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid(format!(
            "test confirmation code must end with six digits: {code:?}"
        )));
    }
    Ok(())
}

fn canonical_decimal(
    field: &str,
    value: &str,
    require_positive: bool,
) -> DailyChangeConfirmationResult<(String, f64)> {
    validate_exact_text(field, value)?;
    let (negative, unsigned) = match value.strip_prefix('-') {
        Some(unsigned) => (true, unsigned),
        None => {
            if value.starts_with('+') {
                return Err(invalid(format!("{field} must not contain a plus sign")));
            }
            (false, value)
        }
    };
    let mut parts = unsigned.split('.');
    let integer = parts.next().unwrap_or_default();
    let fraction = parts.next();
    if parts.next().is_some()
        || integer.is_empty()
        || !integer.bytes().all(|byte| byte.is_ascii_digit())
        || fraction.is_some_and(|digits| {
            digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit())
        })
    {
        return Err(invalid(format!(
            "{field} must be a plain decimal without exponent notation: {value:?}"
        )));
    }

    let normalized_integer = integer.trim_start_matches('0');
    let normalized_integer = if normalized_integer.is_empty() {
        "0"
    } else {
        normalized_integer
    };
    let normalized_fraction = fraction.unwrap_or_default().trim_end_matches('0');
    let magnitude = if normalized_fraction.is_empty() {
        normalized_integer.to_string()
    } else {
        format!("{normalized_integer}.{normalized_fraction}")
    };
    let canonical = if negative && magnitude != "0" {
        format!("-{magnitude}")
    } else {
        magnitude
    };
    let numeric = canonical
        .parse::<f64>()
        .map_err(|error| invalid(format!("{field} is not numeric: {error}")))?;
    if !numeric.is_finite() || (require_positive && numeric <= 0.0) {
        return Err(invalid(format!(
            "{field} must be {}finite, got {value:?}",
            if require_positive {
                "positive and "
            } else {
                ""
            }
        )));
    }
    Ok((canonical, numeric))
}

fn canonical_query(
    query: &DailyChangeConfirmationQuery,
) -> DailyChangeConfirmationResult<CanonicalQuery> {
    validate_exact_text("code", &query.code)?;
    validate_code(&query.code)?;
    if query.previous_date >= query.current_date {
        return Err(invalid(format!(
            "previous_date {} must precede current_date {}",
            query.previous_date, query.current_date
        )));
    }
    for (field, value) in [
        ("daily_provider", query.daily_provider.as_str()),
        ("daily_source", query.daily_source.as_str()),
        ("daily_batch_id", query.daily_batch_id.as_str()),
        ("lifecycle_provider", query.lifecycle_provider.as_str()),
        ("lifecycle_batch_id", query.lifecycle_batch_id.as_str()),
    ] {
        validate_exact_text(field, value)?;
    }
    if let Some(listing_date) = query.listing_date {
        if listing_date > query.current_date {
            return Err(invalid(format!(
                "listing_date {listing_date} must not follow current_date {}",
                query.current_date
            )));
        }
    }
    if let Some(identity) = &query.corporate_action_identity {
        validate_exact_text("corporate_action_identity", identity)?;
    }

    let (previous_close, previous_numeric) =
        canonical_decimal("previous_close", &query.previous_close, true)?;
    let (current_close, current_numeric) =
        canonical_decimal("current_close", &query.current_close, true)?;
    let (calculated_pct, pct_numeric) =
        canonical_decimal("calculated_pct", &query.calculated_pct, false)?;
    let expected_pct = (current_numeric / previous_numeric - 1.0) * 100.0;
    if (pct_numeric - expected_pct).abs() > 0.0001 {
        return Err(invalid(format!(
            "calculated_pct {calculated_pct} disagrees with closes; expected {expected_pct:.6}"
        )));
    }
    if pct_numeric.abs() <= 20.0 {
        return Err(invalid(format!(
            "confirmation is only valid beyond the BR-171 ±20% gate, got {calculated_pct}%"
        )));
    }

    Ok(CanonicalQuery {
        code: query.code.clone(),
        previous_date: query.previous_date.to_string(),
        current_date: query.current_date.to_string(),
        previous_close,
        current_close,
        calculated_pct,
        daily_provider: query.daily_provider.clone(),
        daily_source: query.daily_source.clone(),
        daily_batch_id: query.daily_batch_id.clone(),
        lifecycle_provider: query.lifecycle_provider.clone(),
        lifecycle_batch_id: query.lifecycle_batch_id.clone(),
        listing_date: query.listing_date.map(|date| date.to_string()),
        corporate_action_identity: query.corporate_action_identity.clone(),
    })
}

fn canonical_confirmation(
    input: &DailyChangeConfirmationInput,
) -> DailyChangeConfirmationResult<CanonicalConfirmation> {
    validate_exact_text("operator_identity", &input.operator_identity)?;
    validate_exact_text("reason", &input.reason)?;
    Ok(CanonicalConfirmation {
        query: canonical_query(&input.query)?,
        operator_identity: input.operator_identity.clone(),
        reason: input.reason.clone(),
        confirmed_at: input
            .confirmed_at
            .to_rfc3339_opts(SecondsFormat::Nanos, true),
    })
}

fn canonical_stable_fact_v2(query: &CanonicalQuery) -> CanonicalStableFactV2 {
    CanonicalStableFactV2 {
        code: query.code.clone(),
        previous_date: query.previous_date.clone(),
        current_date: query.current_date.clone(),
        previous_close: query.previous_close.clone(),
        current_close: query.current_close.clone(),
        calculated_pct: query.calculated_pct.clone(),
        daily_provider: query.daily_provider.clone(),
        daily_source: query.daily_source.clone(),
        lifecycle_provider: query.lifecycle_provider.clone(),
        listing_date: query.listing_date.clone(),
        corporate_action_identity: query.corporate_action_identity.clone(),
    }
}

fn hash_serializable<T: Serialize>(
    domain: &[u8],
    payload: &T,
) -> DailyChangeConfirmationResult<String> {
    let encoded = serde_json::to_vec(payload).map_err(|error| {
        audit(format!(
            "cannot serialize confirmation hash payload: {error}"
        ))
    })?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(encoded);
    Ok(hex::encode(hasher.finalize()))
}

fn query_identity_hash(query: &CanonicalQuery) -> DailyChangeConfirmationResult<String> {
    hash_serializable(QUERY_HASH_DOMAIN, query)
}

fn stable_fact_identity_hash_v2(
    fact: &CanonicalStableFactV2,
) -> DailyChangeConfirmationResult<String> {
    hash_serializable(STABLE_FACT_HASH_DOMAIN_V2, fact)
}

/// Stable operator-review token for one objective daily-change fact. The
/// concrete acquisition batch IDs are still validated and audited, but a fresh
/// acquisition of byte-identical facts intentionally produces the same token.
pub fn daily_change_review_token_v2(
    query: &DailyChangeConfirmationQuery,
) -> Result<String, String> {
    let canonical = canonical_query(query).map_err(|error| error.to_string())?;
    hash_serializable(
        REVIEW_TOKEN_HASH_DOMAIN_V2,
        &canonical_stable_fact_v2(&canonical),
    )
    .map_err(|error| error.to_string())
}

fn confirmation_content_hash(
    confirmation: &CanonicalConfirmation,
) -> DailyChangeConfirmationResult<String> {
    hash_serializable(CONTENT_HASH_DOMAIN, confirmation)
}

fn confirmation_id(content_hash: &str) -> String {
    format!("br171_{content_hash}")
}

fn load_rows(
    conn: &mut SqliteConnection,
) -> DailyChangeConfirmationResult<Vec<PersistedConfirmationRow>> {
    diesel::sql_query(
        "SELECT id, schema_version, confirmation_id, query_identity_hash, content_hash,
                code, previous_date, \"current_date\", previous_close, current_close,
                calculated_pct, daily_provider, daily_source, daily_batch_id,
                lifecycle_provider, lifecycle_batch_id, listing_date,
                corporate_action_identity, operator_identity, reason, confirmed_at,
                created_at
           FROM daily_change_confirmation
          ORDER BY id ASC",
    )
    .load(conn)
    .map_err(DailyChangeConfirmationError::from)
}

fn load_chain(conn: &mut SqliteConnection) -> DailyChangeConfirmationResult<Vec<ChainRow>> {
    diesel::sql_query(
        "SELECT confirmation_row_id, previous_hash, record_hash
           FROM daily_change_confirmation_chain
          ORDER BY confirmation_row_id ASC",
    )
    .load(conn)
    .map_err(DailyChangeConfirmationError::from)
}

fn load_by_query_hash(
    conn: &mut SqliteConnection,
    identity_hash: &str,
) -> DailyChangeConfirmationResult<Option<PersistedConfirmationRow>> {
    diesel::sql_query(
        "SELECT id, schema_version, confirmation_id, query_identity_hash, content_hash,
                code, previous_date, \"current_date\", previous_close, current_close,
                calculated_pct, daily_provider, daily_source, daily_batch_id,
                lifecycle_provider, lifecycle_batch_id, listing_date,
                corporate_action_identity, operator_identity, reason, confirmed_at,
                created_at
           FROM daily_change_confirmation
          WHERE query_identity_hash = ?
          LIMIT 1",
    )
    .bind::<Text, _>(identity_hash)
    .get_result(conn)
    .optional()
    .map_err(DailyChangeConfirmationError::from)
}

fn load_v1_by_id(
    conn: &mut SqliteConnection,
    id: i64,
) -> DailyChangeConfirmationResult<PersistedConfirmationRow> {
    diesel::sql_query(
        "SELECT id, schema_version, confirmation_id, query_identity_hash, content_hash,
                code, previous_date, \"current_date\", previous_close, current_close,
                calculated_pct, daily_provider, daily_source, daily_batch_id,
                lifecycle_provider, lifecycle_batch_id, listing_date,
                corporate_action_identity, operator_identity, reason, confirmed_at,
                created_at
           FROM daily_change_confirmation
          WHERE id = ?",
    )
    .bind::<BigInt, _>(id)
    .get_result(conn)
    .map_err(DailyChangeConfirmationError::from)
}

fn load_v2_rows(
    conn: &mut SqliteConnection,
) -> DailyChangeConfirmationResult<Vec<PersistedStableAliasV2Row>> {
    diesel::sql_query(
        "SELECT id, schema_version, stable_fact_identity_hash,
                v1_confirmation_row_id, v1_confirmation_id,
                reviewed_daily_batch_id, reviewed_lifecycle_batch_id,
                content_hash, created_at
           FROM daily_change_confirmation_v2
          ORDER BY id ASC",
    )
    .load(conn)
    .map_err(DailyChangeConfirmationError::from)
}

fn load_v2_by_fact_hash(
    conn: &mut SqliteConnection,
    fact_hash: &str,
) -> DailyChangeConfirmationResult<Option<PersistedStableAliasV2Row>> {
    diesel::sql_query(
        "SELECT id, schema_version, stable_fact_identity_hash,
                v1_confirmation_row_id, v1_confirmation_id,
                reviewed_daily_batch_id, reviewed_lifecycle_batch_id,
                content_hash, created_at
           FROM daily_change_confirmation_v2
          WHERE stable_fact_identity_hash = ?
          LIMIT 1",
    )
    .bind::<Text, _>(fact_hash)
    .get_result(conn)
    .optional()
    .map_err(DailyChangeConfirmationError::from)
}

fn load_v2_chain(conn: &mut SqliteConnection) -> DailyChangeConfirmationResult<Vec<ChainRow>> {
    diesel::sql_query(
        "SELECT confirmation_row_id, previous_hash, record_hash
           FROM daily_change_confirmation_chain_v2
          ORDER BY confirmation_row_id ASC",
    )
    .load(conn)
    .map_err(DailyChangeConfirmationError::from)
}

fn load_chain_for_row(
    conn: &mut SqliteConnection,
    confirmation_row_id: i64,
) -> DailyChangeConfirmationResult<ChainRow> {
    diesel::sql_query(
        "SELECT confirmation_row_id, previous_hash, record_hash
           FROM daily_change_confirmation_chain
          WHERE confirmation_row_id = ?",
    )
    .bind::<BigInt, _>(confirmation_row_id)
    .get_result(conn)
    .map_err(DailyChangeConfirmationError::from)
}

fn parse_date(field: &str, value: &str) -> DailyChangeConfirmationResult<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|error| audit(format!("persisted {field} is invalid: {value:?}: {error}")))
}

fn canonical_confirmation_from_row(
    row: &PersistedConfirmationRow,
) -> DailyChangeConfirmationResult<CanonicalConfirmation> {
    let confirmed_at = DateTime::parse_from_rfc3339(&row.confirmed_at).map_err(|error| {
        audit(format!(
            "persisted confirmed_at is invalid: {:?}: {error}",
            row.confirmed_at
        ))
    })?;
    let input = DailyChangeConfirmationInput {
        query: DailyChangeConfirmationQuery {
            code: row.code.clone(),
            previous_date: parse_date("previous_date", &row.previous_date)?,
            current_date: parse_date("current_date", &row.current_date)?,
            previous_close: row.previous_close.clone(),
            current_close: row.current_close.clone(),
            calculated_pct: row.calculated_pct.clone(),
            daily_provider: row.daily_provider.clone(),
            daily_source: row.daily_source.clone(),
            daily_batch_id: row.daily_batch_id.clone(),
            lifecycle_provider: row.lifecycle_provider.clone(),
            lifecycle_batch_id: row.lifecycle_batch_id.clone(),
            listing_date: row
                .listing_date
                .as_deref()
                .map(|value| parse_date("listing_date", value))
                .transpose()?,
            corporate_action_identity: row.corporate_action_identity.clone(),
        },
        operator_identity: row.operator_identity.clone(),
        reason: row.reason.clone(),
        confirmed_at,
    };
    let canonical = canonical_confirmation(&input).map_err(|error| {
        audit(format!(
            "persisted confirmation row {} is not canonical: {error}",
            row.id
        ))
    })?;
    if canonical.query.previous_close != row.previous_close
        || canonical.query.current_close != row.current_close
        || canonical.query.calculated_pct != row.calculated_pct
        || canonical.confirmed_at != row.confirmed_at
    {
        return Err(audit(format!(
            "persisted confirmation row {} contains non-canonical decimals or timestamp",
            row.id
        )));
    }
    Ok(canonical)
}

fn validate_persisted_row(
    row: &PersistedConfirmationRow,
) -> DailyChangeConfirmationResult<CanonicalConfirmation> {
    if row.schema_version != SCHEMA_VERSION {
        return Err(audit(format!(
            "unsupported schema version {} at confirmation row {}",
            row.schema_version, row.id
        )));
    }
    let canonical = canonical_confirmation_from_row(row)?;
    let expected_query_hash = query_identity_hash(&canonical.query)?;
    if expected_query_hash != row.query_identity_hash {
        return Err(audit(format!(
            "query identity hash mismatch at confirmation row {}",
            row.id
        )));
    }
    let expected_content_hash = confirmation_content_hash(&canonical)?;
    if expected_content_hash != row.content_hash
        || confirmation_id(&expected_content_hash) != row.confirmation_id
    {
        return Err(audit(format!(
            "content identity mismatch at confirmation row {}",
            row.id
        )));
    }
    Ok(canonical)
}

fn calculate_chain_hash(
    previous_hash: &str,
    row: &PersistedConfirmationRow,
) -> DailyChangeConfirmationResult<String> {
    let encoded = serde_json::to_vec(row)
        .map_err(|error| audit(format!("cannot serialize persisted confirmation: {error}")))?;
    let mut hasher = Sha256::new();
    hasher.update(CHAIN_HASH_DOMAIN);
    hasher.update(previous_hash.as_bytes());
    hasher.update(b"\0");
    hasher.update(encoded);
    Ok(hex::encode(hasher.finalize()))
}

fn canonical_alias_v2(
    fact_hash: String,
    v1_confirmation: &PersistedConfirmationRow,
) -> CanonicalStableAliasV2 {
    CanonicalStableAliasV2 {
        stable_fact_identity_hash: fact_hash,
        v1_confirmation_id: v1_confirmation.confirmation_id.clone(),
        reviewed_daily_batch_id: v1_confirmation.daily_batch_id.clone(),
        reviewed_lifecycle_batch_id: v1_confirmation.lifecycle_batch_id.clone(),
    }
}

fn alias_content_hash_v2(alias: &CanonicalStableAliasV2) -> DailyChangeConfirmationResult<String> {
    hash_serializable(ALIAS_CONTENT_HASH_DOMAIN_V2, alias)
}

fn validate_v2_row(
    conn: &mut SqliteConnection,
    row: &PersistedStableAliasV2Row,
) -> DailyChangeConfirmationResult<CanonicalStableFactV2> {
    if row.schema_version != SCHEMA_VERSION_V2 {
        return Err(audit(format!(
            "unsupported v2 schema version {} at stable confirmation row {}",
            row.schema_version, row.id
        )));
    }
    let v1 = load_v1_by_id(conn, row.v1_confirmation_row_id)?;
    let canonical_v1 = validate_persisted_row(&v1)?;
    if row.v1_confirmation_id != v1.confirmation_id
        || row.reviewed_daily_batch_id != v1.daily_batch_id
        || row.reviewed_lifecycle_batch_id != v1.lifecycle_batch_id
    {
        return Err(audit(format!(
            "v2 stable confirmation reference mismatch at row {}",
            row.id
        )));
    }
    let fact = canonical_stable_fact_v2(&canonical_v1.query);
    let fact_hash = stable_fact_identity_hash_v2(&fact)?;
    if fact_hash != row.stable_fact_identity_hash {
        return Err(audit(format!(
            "v2 stable fact identity mismatch at row {}",
            row.id
        )));
    }
    let alias = canonical_alias_v2(fact_hash, &v1);
    if alias_content_hash_v2(&alias)? != row.content_hash {
        return Err(audit(format!(
            "v2 stable confirmation content mismatch at row {}",
            row.id
        )));
    }
    Ok(fact)
}

fn calculate_chain_hash_v2(
    previous_hash: &str,
    row: &PersistedStableAliasV2Row,
) -> DailyChangeConfirmationResult<String> {
    let encoded = serde_json::to_vec(row)
        .map_err(|error| audit(format!("cannot serialize v2 confirmation: {error}")))?;
    let mut hasher = Sha256::new();
    hasher.update(CHAIN_HASH_DOMAIN_V2);
    hasher.update(previous_hash.as_bytes());
    hasher.update(b"\0");
    hasher.update(encoded);
    Ok(hex::encode(hasher.finalize()))
}

fn validate_daily_change_confirmation_chain_v2(
    conn: &mut SqliteConnection,
) -> DailyChangeConfirmationResult<String> {
    let rows = load_v2_rows(conn)?;
    let chain = load_v2_chain(conn)?;
    if rows.len() != chain.len() {
        return Err(audit(format!(
            "v2 hash-chain length mismatch: confirmation_rows={}, chain_rows={}",
            rows.len(),
            chain.len()
        )));
    }
    let mut previous_hash = CHAIN_GENESIS_V2.to_string();
    for (row, link) in rows.iter().zip(chain.iter()) {
        validate_v2_row(conn, row)?;
        if link.confirmation_row_id != row.id || link.previous_hash != previous_hash {
            return Err(audit(format!(
                "v2 hash-chain linkage mismatch at confirmation row {}",
                row.id
            )));
        }
        let expected = calculate_chain_hash_v2(&previous_hash, row)?;
        if expected != link.record_hash {
            return Err(audit(format!(
                "v2 hash-chain record mismatch at confirmation row {}",
                row.id
            )));
        }
        previous_hash = link.record_hash.clone();
    }
    Ok(previous_hash)
}

/// Validate every retained fact and chain link. Any failure is admission-blocking.
pub(crate) fn validate_daily_change_confirmation_chain(
    conn: &mut SqliteConnection,
) -> DailyChangeConfirmationResult<String> {
    let rows = load_rows(conn)?;
    let chain = load_chain(conn)?;
    if rows.len() != chain.len() {
        return Err(audit(format!(
            "hash-chain length mismatch: confirmation_rows={}, chain_rows={}",
            rows.len(),
            chain.len()
        )));
    }

    let mut previous_hash = CHAIN_GENESIS.to_string();
    for (row, link) in rows.iter().zip(chain.iter()) {
        validate_persisted_row(row)?;
        if link.confirmation_row_id != row.id || link.previous_hash != previous_hash {
            return Err(audit(format!(
                "hash-chain linkage mismatch at confirmation row {}",
                row.id
            )));
        }
        let expected_hash = calculate_chain_hash(&previous_hash, row)?;
        if expected_hash != link.record_hash {
            return Err(audit(format!(
                "hash-chain record mismatch at confirmation row {}",
                row.id
            )));
        }
        previous_hash = link.record_hash.clone();
    }
    Ok(previous_hash)
}

pub(super) fn create_schema(conn: &mut SqliteConnection) -> Result<(), String> {
    conn.batch_execute(SCHEMA)
        .map_err(|error| format!("BR-171 create confirmation schema: {error}"))?;
    validate_daily_change_confirmation_chain(conn).map_err(|error| error.to_string())?;
    validate_daily_change_confirmation_chain_v2(conn)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn insert_confirmation_in_transaction(
    conn: &mut SqliteConnection,
    input: &DailyChangeConfirmationInput,
) -> DailyChangeConfirmationResult<DailyChangeConfirmationReceipt> {
    let canonical = canonical_confirmation(input)?;
    let query_hash = query_identity_hash(&canonical.query)?;
    let content_hash = confirmation_content_hash(&canonical)?;
    let requested_confirmation_id = confirmation_id(&content_hash);

    // A complete validation precedes every append. The ledger is intentionally
    // low-volume, so fail-closed historical verification is preferred over a
    // tail-only optimization.
    let previous_hash = validate_daily_change_confirmation_chain(conn)?;
    if let Some(existing) = load_by_query_hash(conn, &query_hash)? {
        if existing.content_hash != content_hash
            || existing.confirmation_id != requested_confirmation_id
        {
            return Err(DailyChangeConfirmationError::Conflict {
                query_identity_hash: query_hash,
            });
        }
        let link = load_chain_for_row(conn, existing.id)?;
        return Ok(DailyChangeConfirmationReceipt {
            confirmation_id: existing.confirmation_id,
            query_identity_hash: existing.query_identity_hash,
            record_hash: link.record_hash,
            inserted: false,
        });
    }

    let inserted = diesel::sql_query(
        "INSERT INTO daily_change_confirmation (
            schema_version, confirmation_id, query_identity_hash, content_hash,
            code, previous_date, \"current_date\", previous_close, current_close,
            calculated_pct, daily_provider, daily_source, daily_batch_id,
            lifecycle_provider, lifecycle_batch_id, listing_date,
            corporate_action_identity, operator_identity, reason, confirmed_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind::<Integer, _>(SCHEMA_VERSION)
    .bind::<Text, _>(&requested_confirmation_id)
    .bind::<Text, _>(&query_hash)
    .bind::<Text, _>(&content_hash)
    .bind::<Text, _>(&canonical.query.code)
    .bind::<Text, _>(&canonical.query.previous_date)
    .bind::<Text, _>(&canonical.query.current_date)
    .bind::<Text, _>(&canonical.query.previous_close)
    .bind::<Text, _>(&canonical.query.current_close)
    .bind::<Text, _>(&canonical.query.calculated_pct)
    .bind::<Text, _>(&canonical.query.daily_provider)
    .bind::<Text, _>(&canonical.query.daily_source)
    .bind::<Text, _>(&canonical.query.daily_batch_id)
    .bind::<Text, _>(&canonical.query.lifecycle_provider)
    .bind::<Text, _>(&canonical.query.lifecycle_batch_id)
    .bind::<Nullable<Text>, _>(canonical.query.listing_date.as_deref())
    .bind::<Nullable<Text>, _>(canonical.query.corporate_action_identity.as_deref())
    .bind::<Text, _>(&canonical.operator_identity)
    .bind::<Text, _>(&canonical.reason)
    .bind::<Text, _>(&canonical.confirmed_at)
    .execute(conn)?;
    if inserted != 1 {
        return Err(audit(format!(
            "confirmation append affected {inserted} fact rows"
        )));
    }

    let row = diesel::sql_query(
        "SELECT id, schema_version, confirmation_id, query_identity_hash, content_hash,
                code, previous_date, \"current_date\", previous_close, current_close,
                calculated_pct, daily_provider, daily_source, daily_batch_id,
                lifecycle_provider, lifecycle_batch_id, listing_date,
                corporate_action_identity, operator_identity, reason, confirmed_at,
                created_at
           FROM daily_change_confirmation
          WHERE id = last_insert_rowid()",
    )
    .get_result::<PersistedConfirmationRow>(conn)?;
    validate_persisted_row(&row)?;
    let record_hash = calculate_chain_hash(&previous_hash, &row)?;
    let chain_inserted = diesel::sql_query(
        "INSERT INTO daily_change_confirmation_chain (
            confirmation_row_id, previous_hash, record_hash
         ) VALUES (?, ?, ?)",
    )
    .bind::<BigInt, _>(row.id)
    .bind::<Text, _>(&previous_hash)
    .bind::<Text, _>(&record_hash)
    .execute(conn)?;
    if chain_inserted != 1 {
        return Err(audit(format!(
            "confirmation append affected {chain_inserted} chain rows"
        )));
    }
    Ok(DailyChangeConfirmationReceipt {
        confirmation_id: requested_confirmation_id,
        query_identity_hash: query_hash,
        record_hash,
        inserted: true,
    })
}

fn receipt_for_v2_alias(
    conn: &mut SqliteConnection,
    alias: &PersistedStableAliasV2Row,
) -> DailyChangeConfirmationResult<DailyChangeConfirmationReceipt> {
    validate_v2_row(conn, alias)?;
    let v1 = load_v1_by_id(conn, alias.v1_confirmation_row_id)?;
    let link = load_chain_for_row(conn, v1.id)?;
    Ok(DailyChangeConfirmationReceipt {
        confirmation_id: v1.confirmation_id,
        query_identity_hash: alias.stable_fact_identity_hash.clone(),
        record_hash: link.record_hash,
        inserted: false,
    })
}

fn insert_v2_alias(
    conn: &mut SqliteConnection,
    previous_hash: &str,
    stable_fact_identity_hash: &str,
    v1: &PersistedConfirmationRow,
) -> DailyChangeConfirmationResult<()> {
    let alias = canonical_alias_v2(stable_fact_identity_hash.to_string(), v1);
    let content_hash = alias_content_hash_v2(&alias)?;
    let inserted = diesel::sql_query(
        "INSERT INTO daily_change_confirmation_v2 (
            schema_version, stable_fact_identity_hash, v1_confirmation_row_id,
            v1_confirmation_id, reviewed_daily_batch_id,
            reviewed_lifecycle_batch_id, content_hash
         ) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind::<Integer, _>(SCHEMA_VERSION_V2)
    .bind::<Text, _>(&alias.stable_fact_identity_hash)
    .bind::<BigInt, _>(v1.id)
    .bind::<Text, _>(&alias.v1_confirmation_id)
    .bind::<Text, _>(&alias.reviewed_daily_batch_id)
    .bind::<Text, _>(&alias.reviewed_lifecycle_batch_id)
    .bind::<Text, _>(&content_hash)
    .execute(conn)?;
    if inserted != 1 {
        return Err(audit(format!(
            "v2 confirmation append affected {inserted} fact rows"
        )));
    }
    let row = diesel::sql_query(
        "SELECT id, schema_version, stable_fact_identity_hash,
                v1_confirmation_row_id, v1_confirmation_id,
                reviewed_daily_batch_id, reviewed_lifecycle_batch_id,
                content_hash, created_at
           FROM daily_change_confirmation_v2
          WHERE id = last_insert_rowid()",
    )
    .get_result::<PersistedStableAliasV2Row>(conn)?;
    validate_v2_row(conn, &row)?;
    let record_hash = calculate_chain_hash_v2(previous_hash, &row)?;
    let chain_inserted = diesel::sql_query(
        "INSERT INTO daily_change_confirmation_chain_v2 (
            confirmation_row_id, previous_hash, record_hash
         ) VALUES (?, ?, ?)",
    )
    .bind::<BigInt, _>(row.id)
    .bind::<Text, _>(previous_hash)
    .bind::<Text, _>(&record_hash)
    .execute(conn)?;
    if chain_inserted != 1 {
        return Err(audit(format!(
            "v2 confirmation append affected {chain_inserted} chain rows"
        )));
    }
    Ok(())
}

pub(crate) fn append_daily_change_confirmation_on_conn(
    conn: &mut SqliteConnection,
    input: &DailyChangeConfirmationInput,
) -> DailyChangeConfirmationResult<DailyChangeConfirmationReceipt> {
    conn.immediate_transaction::<_, DailyChangeConfirmationError, _>(|conn| {
        let canonical = canonical_confirmation(input)?;
        validate_daily_change_confirmation_chain(conn)?;
        let previous_v2_hash = validate_daily_change_confirmation_chain_v2(conn)?;
        let stable_fact = canonical_stable_fact_v2(&canonical.query);
        let stable_hash = stable_fact_identity_hash_v2(&stable_fact)?;
        if let Some(existing) = load_v2_by_fact_hash(conn, &stable_hash)? {
            let persisted = validate_v2_row(conn, &existing)?;
            if persisted != stable_fact {
                return Err(audit(format!(
                    "SHA-256 stable fact identity collision at v2 row {}",
                    existing.id
                )));
            }
            let retained_v1 = load_v1_by_id(conn, existing.v1_confirmation_row_id)?;
            let retained_decision = validate_persisted_row(&retained_v1)?;
            if retained_decision.operator_identity != canonical.operator_identity
                || retained_decision.reason != canonical.reason
            {
                return Err(DailyChangeConfirmationError::Conflict {
                    query_identity_hash: stable_hash,
                });
            }
            return receipt_for_v2_alias(conn, &existing);
        }

        let receipt = insert_confirmation_in_transaction(conn, input)?;
        let v1_query_hash = query_identity_hash(&canonical.query)?;
        let v1 = load_by_query_hash(conn, &v1_query_hash)?
            .ok_or_else(|| audit("v1 confirmation disappeared before v2 alias append"))?;
        insert_v2_alias(conn, &previous_v2_hash, &stable_hash, &v1)?;
        Ok(receipt)
    })
}

pub(crate) fn has_exact_daily_change_confirmation_on_conn(
    conn: &mut SqliteConnection,
    query: &DailyChangeConfirmationQuery,
) -> DailyChangeConfirmationResult<bool> {
    validate_daily_change_confirmation_chain(conn)?;
    validate_daily_change_confirmation_chain_v2(conn)?;
    let canonical = canonical_query(query)?;
    let identity_hash = query_identity_hash(&canonical)?;
    if let Some(row) = load_by_query_hash(conn, &identity_hash)? {
        let persisted = validate_persisted_row(&row)?;
        if persisted.query != canonical {
            return Err(audit(format!(
                "SHA-256 query identity collision at confirmation row {}",
                row.id
            )));
        }
        return Ok(true);
    }
    let stable_fact = canonical_stable_fact_v2(&canonical);
    let stable_hash = stable_fact_identity_hash_v2(&stable_fact)?;
    let Some(alias) = load_v2_by_fact_hash(conn, &stable_hash)? else {
        log::warn!(
            "[BR-171] stable confirmation miss hash={} code={} dates={}→{} closes={}→{} pct={} daily={}/{} lifecycle={} listing={} action={}",
            stable_hash,
            canonical.code,
            canonical.previous_date,
            canonical.current_date,
            canonical.previous_close,
            canonical.current_close,
            canonical.calculated_pct,
            canonical.daily_provider,
            canonical.daily_source,
            canonical.lifecycle_provider,
            canonical.listing_date.as_deref().unwrap_or("missing"),
            canonical
                .corporate_action_identity
                .as_deref()
                .unwrap_or("missing"),
        );
        return Ok(false);
    };
    let persisted = validate_v2_row(conn, &alias)?;
    if persisted != stable_fact {
        return Err(audit(format!(
            "SHA-256 stable fact identity collision at v2 row {}",
            alias.id
        )));
    }
    Ok(true)
}

impl DatabaseManager {
    pub fn append_daily_change_confirmation(
        &self,
        input: &DailyChangeConfirmationInput,
    ) -> Result<DailyChangeConfirmationReceipt, String> {
        let mut conn = self
            .get_conn()
            .map_err(|error| format!("BR-171 confirmation DB connection: {error}"))?;
        append_daily_change_confirmation_on_conn(&mut conn, input)
            .map_err(|error| error.to_string())
    }

    pub fn has_exact_daily_change_confirmation(
        &self,
        query: &DailyChangeConfirmationQuery,
    ) -> Result<bool, String> {
        let mut conn = self
            .get_conn()
            .map_err(|error| format!("BR-171 confirmation DB connection: {error}"))?;
        has_exact_daily_change_confirmation_on_conn(&mut conn, query)
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[derive(Debug, QueryableByName)]
    struct CountRow {
        #[diesel(sql_type = BigInt)]
        count: i64,
    }

    fn connection() -> SqliteConnection {
        let mut conn = SqliteConnection::establish(":memory:").expect("in-memory SQLite");
        conn.batch_execute("PRAGMA foreign_keys = ON;")
            .expect("foreign keys");
        create_schema(&mut conn).expect("confirmation schema");
        conn
    }

    fn query() -> DailyChangeConfirmationQuery {
        DailyChangeConfirmationQuery {
            code: "TEST_CODE_000001".to_string(),
            previous_date: NaiveDate::from_ymd_opt(2026, 7, 23).expect("previous date"),
            current_date: NaiveDate::from_ymd_opt(2026, 7, 24).expect("current date"),
            previous_close: "10.00".to_string(),
            current_close: "12.50".to_string(),
            calculated_pct: "25.00".to_string(),
            daily_provider: "TEST_CODE_MAGIC_TDX".to_string(),
            daily_source: "TEST_CODE_tdx-smart".to_string(),
            daily_batch_id: "TEST_CODE_daily_batch_1".to_string(),
            lifecycle_provider: "TEST_CODE_MAGIC_TDX".to_string(),
            lifecycle_batch_id: "TEST_CODE_lifecycle_batch_1".to_string(),
            listing_date: Some(NaiveDate::from_ymd_opt(2020, 1, 2).expect("listing date")),
            corporate_action_identity: None,
        }
    }

    fn input() -> DailyChangeConfirmationInput {
        DailyChangeConfirmationInput {
            query: query(),
            operator_identity: "TEST_CODE_OPERATOR".to_string(),
            reason: "TEST_CODE reviewed provider evidence".to_string(),
            confirmed_at: FixedOffset::east_opt(8 * 3600)
                .expect("offset")
                .with_ymd_and_hms(2026, 7, 24, 18, 30, 0)
                .single()
                .expect("confirmation timestamp"),
        }
    }

    fn count(conn: &mut SqliteConnection, table: &str) -> i64 {
        diesel::sql_query(format!("SELECT COUNT(*) AS count FROM {table}"))
            .get_result::<CountRow>(conn)
            .expect("count")
            .count
    }

    #[test]
    fn exact_confirmation_round_trips_through_public_connection_api() {
        let mut conn = connection();
        assert!(
            !has_exact_daily_change_confirmation_on_conn(&mut conn, &query())
                .expect("empty exact lookup")
        );

        let receipt = append_daily_change_confirmation_on_conn(&mut conn, &input())
            .expect("append confirmation");
        assert!(receipt.inserted);
        assert_eq!(receipt.query_identity_hash.len(), 64);
        assert_eq!(receipt.record_hash.len(), 64);
        assert!(
            has_exact_daily_change_confirmation_on_conn(&mut conn, &query()).expect("exact lookup")
        );
        validate_daily_change_confirmation_chain(&mut conn).expect("valid hash chain");
    }

    #[test]
    fn identical_append_is_idempotent_and_different_decision_conflicts() {
        let mut conn = connection();
        let first =
            append_daily_change_confirmation_on_conn(&mut conn, &input()).expect("first append");
        let replay =
            append_daily_change_confirmation_on_conn(&mut conn, &input()).expect("exact replay");
        assert!(!replay.inserted);
        assert_eq!(replay.confirmation_id, first.confirmation_id);
        assert_eq!(replay.record_hash, first.record_hash);
        assert_eq!(count(&mut conn, "daily_change_confirmation"), 1);
        assert_eq!(count(&mut conn, "daily_change_confirmation_chain"), 1);

        let mut conflicting = input();
        conflicting.reason = "TEST_CODE different operator decision".to_string();
        assert!(matches!(
            append_daily_change_confirmation_on_conn(&mut conn, &conflicting),
            Err(DailyChangeConfirmationError::Conflict { .. })
        ));
        let mut conflicting_operator = input();
        conflicting_operator.operator_identity = "TEST_CODE_OTHER_OPERATOR".to_string();
        assert!(matches!(
            append_daily_change_confirmation_on_conn(&mut conn, &conflicting_operator),
            Err(DailyChangeConfirmationError::Conflict { .. })
        ));
        assert_eq!(count(&mut conn, "daily_change_confirmation"), 1);
    }

    #[test]
    fn v2_stable_fact_reuses_confirmation_across_acquisition_batch_rotation() {
        let mut conn = connection();
        append_daily_change_confirmation_on_conn(&mut conn, &input()).expect("first append");

        let mut changed_query = query();
        changed_query.daily_batch_id = "TEST_CODE_daily_batch_2".to_string();
        changed_query.lifecycle_batch_id = "TEST_CODE_lifecycle_batch_2".to_string();
        assert!(
            has_exact_daily_change_confirmation_on_conn(&mut conn, &changed_query)
                .expect("same stable fact lookup")
        );
        let mut changed_input = input();
        changed_input.query = changed_query.clone();
        let replay = append_daily_change_confirmation_on_conn(&mut conn, &changed_input)
            .expect("stable fact replay");
        assert!(!replay.inserted);
        assert!(
            has_exact_daily_change_confirmation_on_conn(&mut conn, &changed_query)
                .expect("replayed stable lookup")
        );
        assert_eq!(count(&mut conn, "daily_change_confirmation"), 1);
        assert_eq!(count(&mut conn, "daily_change_confirmation_v2"), 1);
        assert_eq!(count(&mut conn, "daily_change_confirmation_chain_v2"), 1);
        validate_daily_change_confirmation_chain(&mut conn).expect("v1 chain remains valid");
        validate_daily_change_confirmation_chain_v2(&mut conn).expect("v2 chain remains valid");

        let mut changed_fact = changed_query;
        changed_fact.daily_source = "TEST_CODE_different_source".to_string();
        assert!(
            !has_exact_daily_change_confirmation_on_conn(&mut conn, &changed_fact)
                .expect("different source requires a new confirmation")
        );
    }

    #[test]
    fn invalid_input_is_atomic_and_small_changes_cannot_be_confirmed() {
        let mut conn = connection();
        let mut invalid = input();
        invalid.query.current_close = "11".to_string();
        invalid.query.calculated_pct = "10".to_string();
        assert!(matches!(
            append_daily_change_confirmation_on_conn(&mut conn, &invalid),
            Err(DailyChangeConfirmationError::InvalidInput(_))
        ));
        assert_eq!(count(&mut conn, "daily_change_confirmation"), 0);
        assert_eq!(count(&mut conn, "daily_change_confirmation_chain"), 0);

        let mut blank_reason = input();
        blank_reason.reason = " ".to_string();
        assert!(matches!(
            append_daily_change_confirmation_on_conn(&mut conn, &blank_reason),
            Err(DailyChangeConfirmationError::InvalidInput(_))
        ));
        assert_eq!(count(&mut conn, "daily_change_confirmation"), 0);
    }

    #[test]
    fn facts_and_chain_are_immutable_with_five_year_retention() {
        let mut conn = connection();
        append_daily_change_confirmation_on_conn(&mut conn, &input()).expect("append");

        let fact_update =
            diesel::sql_query("UPDATE daily_change_confirmation SET reason = 'TEST_CODE_TAMPER'")
                .execute(&mut conn)
                .expect_err("fact update must fail");
        assert!(fact_update.to_string().contains("at least five years"));
        let fact_delete = diesel::sql_query("DELETE FROM daily_change_confirmation")
            .execute(&mut conn)
            .expect_err("fact delete must fail");
        assert!(fact_delete.to_string().contains("at least five years"));
        let chain_update = diesel::sql_query(
            "UPDATE daily_change_confirmation_chain SET previous_hash = 'TEST_CODE_TAMPER'",
        )
        .execute(&mut conn)
        .expect_err("chain update must fail");
        assert!(chain_update.to_string().contains("at least five years"));
        let chain_delete = diesel::sql_query("DELETE FROM daily_change_confirmation_chain")
            .execute(&mut conn)
            .expect_err("chain delete must fail");
        assert!(chain_delete.to_string().contains("at least five years"));
        assert_eq!(DAILY_CHANGE_CONFIRMATION_MIN_RETENTION_YEARS, 5);
    }

    #[test]
    fn fact_tamper_is_detected_and_blocks_lookup_and_append() {
        let mut conn = connection();
        append_daily_change_confirmation_on_conn(&mut conn, &input()).expect("append");
        diesel::sql_query("DROP TRIGGER trg_daily_change_confirmation_no_update")
            .execute(&mut conn)
            .expect("test-only tamper setup");
        diesel::sql_query(
            "UPDATE daily_change_confirmation SET reason = 'TEST_CODE_TAMPERED_CONTENT'",
        )
        .execute(&mut conn)
        .expect("test-only fact tamper");

        assert!(matches!(
            validate_daily_change_confirmation_chain(&mut conn),
            Err(DailyChangeConfirmationError::Audit(_))
        ));
        assert!(matches!(
            has_exact_daily_change_confirmation_on_conn(&mut conn, &query()),
            Err(DailyChangeConfirmationError::Audit(_))
        ));
        let mut second = input();
        second.query.daily_batch_id = "TEST_CODE_daily_batch_2".to_string();
        assert!(matches!(
            append_daily_change_confirmation_on_conn(&mut conn, &second),
            Err(DailyChangeConfirmationError::Audit(_))
        ));
        assert_eq!(count(&mut conn, "daily_change_confirmation"), 1);
    }

    #[test]
    fn chain_tamper_is_detected() {
        let mut conn = connection();
        append_daily_change_confirmation_on_conn(&mut conn, &input()).expect("append");
        diesel::sql_query("DROP TRIGGER trg_daily_change_confirmation_chain_no_update")
            .execute(&mut conn)
            .expect("test-only tamper setup");
        diesel::sql_query(
            "UPDATE daily_change_confirmation_chain
                SET record_hash = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'",
        )
        .execute(&mut conn)
        .expect("test-only chain tamper");
        assert!(matches!(
            validate_daily_change_confirmation_chain(&mut conn),
            Err(DailyChangeConfirmationError::Audit(_))
        ));
    }
}
