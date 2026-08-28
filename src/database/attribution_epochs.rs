//! BR-255 immutable attribution sample epoch storage.
//!
//! This is the only module allowed to know the epoch/carry/attempt/daily SQL.
//! Every read and append verifies the complete retained state before returning.

use chrono::{DateTime, Months, NaiveDate, Utc};
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Integer, Nullable, Text};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::DatabaseManager;
use crate::performance::attribution_epoch::{
    canonical_legacy_carry_manifest_hash, AttributionEpochSelector, LegacyCarryPosition,
};

const RECEIPT_GENESIS: &str = "BR255_ATTRIBUTION_EPOCH_RECEIPT_GENESIS_V1";
const CARRY_GENESIS: &str = "BR255_ATTRIBUTION_LEGACY_CARRY_GENESIS_V1";
const ATTEMPT_GENESIS: &str = "BR255_ATTRIBUTION_EPOCH_ATTEMPT_GENESIS_V1";
const DAILY_GENESIS: &str = "BR255_ATTRIBUTION_EPOCH_DAILY_GENESIS_V1";

const TABLES: [(&str, &str); 7] = [
    ("attribution_sample_epoch_receipt", RECEIPT_TABLE),
    (
        "attribution_sample_epoch_receipt_chain",
        RECEIPT_CHAIN_TABLE,
    ),
    ("attribution_legacy_carry_item", CARRY_TABLE),
    ("attribution_epoch_attempt_audit", ATTEMPT_TABLE),
    ("attribution_epoch_attempt_chain", ATTEMPT_CHAIN_TABLE),
    ("paper_attribution_epoch_daily", DAILY_TABLE),
    ("paper_attribution_epoch_daily_chain", DAILY_CHAIN_TABLE),
];

const RECEIPT_TABLE: &str = r#"CREATE TABLE attribution_sample_epoch_receipt (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    epoch_id TEXT NOT NULL UNIQUE CHECK(length(epoch_id) = 64),
    cutover_completed_trading_date TEXT NOT NULL,
    effective_trading_date TEXT NOT NULL,
    paper_trade_high_water INTEGER NOT NULL CHECK(paper_trade_high_water >= 0),
    legacy_filled_manifest_hash TEXT NOT NULL CHECK(length(legacy_filled_manifest_hash) = 64),
    terminal_binding_manifest_hash TEXT NOT NULL CHECK(length(terminal_binding_manifest_hash) = 64),
    order_audit_high_water INTEGER NOT NULL CHECK(order_audit_high_water >= 0),
    order_audit_tip_hash TEXT NOT NULL CHECK(length(order_audit_tip_hash) = 64),
    calendar_authority_hash TEXT NOT NULL CHECK(length(calendar_authority_hash) = 64),
    legacy_carry_manifest_hash TEXT NOT NULL CHECK(length(legacy_carry_manifest_hash) = 64),
    carry_item_count INTEGER NOT NULL CHECK(carry_item_count >= 0),
    carry_total_quantity INTEGER NOT NULL CHECK(carry_total_quantity >= 0),
    position_projection_hash TEXT NOT NULL CHECK(length(position_projection_hash) = 64),
    previous_epoch_receipt_hash TEXT CHECK(previous_epoch_receipt_hash IS NULL OR length(previous_epoch_receipt_hash) = 64),
    decision_basis TEXT NOT NULL CHECK(decision_basis = 'BR-255'),
    receipt_hash TEXT NOT NULL UNIQUE CHECK(length(receipt_hash) = 64),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    retention_deadline TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '+60 months'))
)"#;

const RECEIPT_CHAIN_TABLE: &str = r#"CREATE TABLE attribution_sample_epoch_receipt_chain (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    epoch_receipt_id INTEGER NOT NULL UNIQUE,
    previous_hash TEXT NOT NULL,
    record_hash TEXT NOT NULL UNIQUE CHECK(length(record_hash) = 64),
    created_at TEXT NOT NULL,
    retention_deadline TEXT NOT NULL,
    FOREIGN KEY(epoch_receipt_id) REFERENCES attribution_sample_epoch_receipt(id)
)"#;

const CARRY_TABLE: &str = r#"CREATE TABLE attribution_legacy_carry_item (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    epoch_receipt_id INTEGER NOT NULL,
    code TEXT NOT NULL CHECK(length(trim(code)) > 0),
    quantity INTEGER NOT NULL CHECK(quantity > 0),
    item_index INTEGER NOT NULL CHECK(item_index >= 0),
    predecessor_item_hash TEXT NOT NULL,
    item_hash TEXT NOT NULL UNIQUE CHECK(length(item_hash) = 64),
    created_at TEXT NOT NULL,
    retention_deadline TEXT NOT NULL,
    UNIQUE(epoch_receipt_id, item_index),
    FOREIGN KEY(epoch_receipt_id) REFERENCES attribution_sample_epoch_receipt(id)
)"#;

const ATTEMPT_TABLE: &str = r#"CREATE TABLE attribution_epoch_attempt_audit (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source TEXT NOT NULL CHECK(length(trim(source)) > 0),
    invoked_at TEXT NOT NULL,
    completed_session_date TEXT,
    effective_date TEXT,
    outcome TEXT NOT NULL CHECK(length(trim(outcome)) > 0),
    reason_code TEXT NOT NULL CHECK(length(trim(reason_code)) > 0),
    retryable INTEGER NOT NULL CHECK(retryable IN (0, 1)),
    source_summary_hash TEXT NOT NULL CHECK(length(source_summary_hash) = 64),
    epoch_id TEXT CHECK(epoch_id IS NULL OR length(epoch_id) = 64),
    success_receipt_hash TEXT CHECK(success_receipt_hash IS NULL OR length(success_receipt_hash) = 64),
    predecessor_attempt_hash TEXT NOT NULL,
    record_hash TEXT NOT NULL UNIQUE CHECK(length(record_hash) = 64),
    created_at TEXT NOT NULL,
    retention_deadline TEXT NOT NULL,
    CHECK((epoch_id IS NULL) = (success_receipt_hash IS NULL))
)"#;

const ATTEMPT_CHAIN_TABLE: &str = r#"CREATE TABLE attribution_epoch_attempt_chain (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    attempt_audit_id INTEGER NOT NULL UNIQUE,
    previous_hash TEXT NOT NULL,
    record_hash TEXT NOT NULL UNIQUE CHECK(length(record_hash) = 64),
    created_at TEXT NOT NULL,
    retention_deadline TEXT NOT NULL,
    FOREIGN KEY(attempt_audit_id) REFERENCES attribution_epoch_attempt_audit(id)
)"#;

const DAILY_TABLE: &str = r#"CREATE TABLE paper_attribution_epoch_daily (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    epoch_id TEXT NOT NULL CHECK(length(epoch_id) = 64),
    date TEXT NOT NULL,
    signal_family TEXT NOT NULL CHECK(length(trim(signal_family)) > 0),
    payload_json TEXT NOT NULL CHECK(length(trim(payload_json)) > 0),
    payload_hash TEXT NOT NULL CHECK(length(payload_hash) = 64),
    predecessor_daily_hash TEXT NOT NULL,
    record_hash TEXT NOT NULL UNIQUE CHECK(length(record_hash) = 64),
    created_at TEXT NOT NULL,
    retention_deadline TEXT NOT NULL,
    UNIQUE(epoch_id, date, signal_family, payload_hash),
    FOREIGN KEY(epoch_id) REFERENCES attribution_sample_epoch_receipt(epoch_id)
)"#;

const DAILY_CHAIN_TABLE: &str = r#"CREATE TABLE paper_attribution_epoch_daily_chain (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    epoch_daily_id INTEGER NOT NULL UNIQUE,
    previous_hash TEXT NOT NULL,
    record_hash TEXT NOT NULL UNIQUE CHECK(length(record_hash) = 64),
    created_at TEXT NOT NULL,
    retention_deadline TEXT NOT NULL,
    FOREIGN KEY(epoch_daily_id) REFERENCES paper_attribution_epoch_daily(id)
)"#;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AttributionEpochReceipt {
    pub epoch_id: String,
    pub cutover_completed_trading_date: NaiveDate,
    pub effective_trading_date: NaiveDate,
    pub paper_trade_high_water: i64,
    pub legacy_filled_manifest_hash: String,
    pub terminal_binding_manifest_hash: String,
    pub order_audit_high_water: i64,
    pub order_audit_tip_hash: String,
    pub calendar_authority_hash: String,
    pub legacy_carry_manifest_hash: String,
    pub carry_item_count: u64,
    pub carry_total_quantity: u64,
    pub position_projection_hash: String,
    pub previous_epoch_receipt_hash: Option<String>,
    pub receipt_hash: String,
    pub created_at: String,
    pub retention_deadline: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedAttributionEpoch {
    Legacy,
    Epoch(AttributionEpochReceipt),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttributionEpochStoreError {
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

impl AttributionEpochStoreError {
    pub fn reason_code(&self) -> &'static str {
        match self {
            Self::Unavailable { reason_code, .. } | Self::FailedIntegrity { reason_code, .. } => {
                reason_code
            }
        }
    }

    pub fn retryable(&self) -> bool {
        matches!(
            self,
            Self::Unavailable {
                retryable: true,
                ..
            }
        )
    }
}

impl std::fmt::Display for AttributionEpochStoreError {
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

impl std::error::Error for AttributionEpochStoreError {}

#[derive(Debug, Clone)]
#[allow(dead_code)] // Narrow Task 4 activation write seam; not public authority.
pub(crate) struct AttributionEpochAttemptAppend {
    pub(crate) source: String,
    pub(crate) invoked_at: String,
    pub(crate) completed_session_date: Option<NaiveDate>,
    pub(crate) effective_date: Option<NaiveDate>,
    pub(crate) outcome: String,
    pub(crate) reason_code: String,
    pub(crate) retryable: bool,
    pub(crate) source_summary_hash: String,
    pub(crate) epoch_id: Option<String>,
    pub(crate) success_receipt_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // Returned by the Task 4 activation write seam.
pub(crate) struct AttributionEpochAttemptReceipt {
    pub(crate) attempt_audit_id: i64,
    pub(crate) record_hash: String,
    pub(crate) created_at: String,
    pub(crate) retention_deadline: String,
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // Narrow Task 7 daily persistence seam; not public authority.
pub(crate) struct AttributionEpochDailyAppend {
    pub(crate) epoch_id: String,
    pub(crate) date: NaiveDate,
    pub(crate) signal_family: String,
    pub(crate) payload: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // Returned by the Task 7 daily persistence seam.
pub(crate) struct AttributionEpochDailyReceipt {
    pub(crate) epoch_daily_id: i64,
    pub(crate) payload_hash: String,
    pub(crate) record_hash: String,
    pub(crate) created_at: String,
    pub(crate) retention_deadline: String,
}

pub struct AttributionEpochStore<'a> {
    database: &'a DatabaseManager,
}

#[derive(QueryableByName)]
struct CountRow {
    #[diesel(sql_type = BigInt)]
    count: i64,
}

#[derive(QueryableByName)]
struct SchemaRow {
    #[diesel(sql_type = Text)]
    name: String,
    #[diesel(sql_type = Nullable<Text>)]
    sql: Option<String>,
}

#[derive(QueryableByName)]
struct SequenceRow {
    #[diesel(sql_type = Nullable<BigInt>)]
    seq: Option<i64>,
}

#[derive(QueryableByName)]
struct IdRow {
    #[diesel(sql_type = BigInt)]
    id: i64,
}

#[derive(Debug, Clone, QueryableByName, Serialize)]
struct PersistedReceipt {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = Text)]
    epoch_id: String,
    #[diesel(sql_type = Text)]
    cutover_completed_trading_date: String,
    #[diesel(sql_type = Text)]
    effective_trading_date: String,
    #[diesel(sql_type = BigInt)]
    paper_trade_high_water: i64,
    #[diesel(sql_type = Text)]
    legacy_filled_manifest_hash: String,
    #[diesel(sql_type = Text)]
    terminal_binding_manifest_hash: String,
    #[diesel(sql_type = BigInt)]
    order_audit_high_water: i64,
    #[diesel(sql_type = Text)]
    order_audit_tip_hash: String,
    #[diesel(sql_type = Text)]
    calendar_authority_hash: String,
    #[diesel(sql_type = Text)]
    legacy_carry_manifest_hash: String,
    #[diesel(sql_type = BigInt)]
    carry_item_count: i64,
    #[diesel(sql_type = BigInt)]
    carry_total_quantity: i64,
    #[diesel(sql_type = Text)]
    position_projection_hash: String,
    #[diesel(sql_type = Nullable<Text>)]
    previous_epoch_receipt_hash: Option<String>,
    #[diesel(sql_type = Text)]
    decision_basis: String,
    #[diesel(sql_type = Text)]
    receipt_hash: String,
    #[diesel(sql_type = Text)]
    created_at: String,
    #[diesel(sql_type = Text)]
    retention_deadline: String,
}

#[derive(Debug, Clone, QueryableByName, Serialize)]
struct PersistedCarry {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = BigInt)]
    epoch_receipt_id: i64,
    #[diesel(sql_type = Text)]
    code: String,
    #[diesel(sql_type = BigInt)]
    quantity: i64,
    #[diesel(sql_type = BigInt)]
    item_index: i64,
    #[diesel(sql_type = Text)]
    predecessor_item_hash: String,
    #[diesel(sql_type = Text)]
    item_hash: String,
    #[diesel(sql_type = Text)]
    created_at: String,
    #[diesel(sql_type = Text)]
    retention_deadline: String,
}

#[derive(Debug, Clone, QueryableByName, Serialize)]
struct PersistedAttempt {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = Text)]
    source: String,
    #[diesel(sql_type = Text)]
    invoked_at: String,
    #[diesel(sql_type = Nullable<Text>)]
    completed_session_date: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    effective_date: Option<String>,
    #[diesel(sql_type = Text)]
    outcome: String,
    #[diesel(sql_type = Text)]
    reason_code: String,
    #[diesel(sql_type = Integer)]
    retryable: i32,
    #[diesel(sql_type = Text)]
    source_summary_hash: String,
    #[diesel(sql_type = Nullable<Text>)]
    epoch_id: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    success_receipt_hash: Option<String>,
    #[diesel(sql_type = Text)]
    predecessor_attempt_hash: String,
    #[diesel(sql_type = Text)]
    record_hash: String,
    #[diesel(sql_type = Text)]
    created_at: String,
    #[diesel(sql_type = Text)]
    retention_deadline: String,
}

#[derive(Debug, Clone, QueryableByName, Serialize)]
struct PersistedDaily {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = Text)]
    epoch_id: String,
    #[diesel(sql_type = Text)]
    date: String,
    #[diesel(sql_type = Text)]
    signal_family: String,
    #[diesel(sql_type = Text)]
    payload_json: String,
    #[diesel(sql_type = Text)]
    payload_hash: String,
    #[diesel(sql_type = Text)]
    predecessor_daily_hash: String,
    #[diesel(sql_type = Text)]
    record_hash: String,
    #[diesel(sql_type = Text)]
    created_at: String,
    #[diesel(sql_type = Text)]
    retention_deadline: String,
}

#[derive(Debug, Clone, QueryableByName)]
struct PersistedChain {
    #[diesel(sql_type = BigInt)]
    id: i64,
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

#[derive(QueryableByName)]
#[allow(dead_code)]
struct RetentionWindow {
    #[diesel(sql_type = Text)]
    created_at: String,
    #[diesel(sql_type = Text)]
    retention_deadline: String,
}

fn integrity(detail: impl Into<String>) -> diesel::result::Error {
    diesel::result::Error::QueryBuilderError(Box::new(std::io::Error::other(detail.into())))
}

fn failed_integrity(detail: impl Into<String>) -> AttributionEpochStoreError {
    AttributionEpochStoreError::FailedIntegrity {
        reason_code: "attribution_epoch_integrity_failed",
        detail: detail.into(),
    }
}

fn unavailable(detail: impl Into<String>) -> AttributionEpochStoreError {
    AttributionEpochStoreError::Unavailable {
        reason_code: "attribution_epoch_unavailable",
        retryable: false,
        detail: detail.into(),
    }
}

fn map_integrity(error: diesel::result::Error) -> AttributionEpochStoreError {
    failed_integrity(format!("BR-255 attribution epoch retained state: {error}"))
}

fn normalized_sql(sql: &str) -> String {
    sql.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace(" IF NOT EXISTS", "")
}

fn trigger_sql(table: &str, event: &str) -> String {
    let suffix = event.to_ascii_lowercase();
    let action = if event == "UPDATE" {
        format!("BR-255 {table} is immutable")
    } else {
        format!("BR-255 {table} retention is at least 60 natural months")
    };
    format!(
        "CREATE TRIGGER trg_{table}_no_{suffix} BEFORE {event} ON {table} \
         BEGIN SELECT RAISE(ABORT, '{action}'); END"
    )
}

fn install_triggers(conn: &mut SqliteConnection, table: &str) -> diesel::QueryResult<()> {
    for event in ["UPDATE", "DELETE"] {
        diesel::sql_query(trigger_sql(table, event).replacen(
            "CREATE TRIGGER",
            "CREATE TRIGGER IF NOT EXISTS",
            1,
        ))
        .execute(conn)?;
    }
    Ok(())
}

fn validate_schema(conn: &mut SqliteConnection) -> diesel::QueryResult<()> {
    for (name, expected) in TABLES {
        let row = diesel::sql_query(
            "SELECT name, sql FROM sqlite_master WHERE type = 'table' AND name = ?",
        )
        .bind::<Text, _>(name)
        .get_result::<SchemaRow>(conn)
        .optional()?;
        if !row.is_some_and(|row| {
            row.name == name
                && row
                    .sql
                    .is_some_and(|sql| normalized_sql(&sql) == normalized_sql(expected))
        }) {
            return Err(integrity(format!(
                "BR-255 canonical table definition is missing or changed: {name}"
            )));
        }
        for event in ["UPDATE", "DELETE"] {
            let trigger_name = format!("trg_{name}_no_{}", event.to_ascii_lowercase());
            let trigger = diesel::sql_query(
                "SELECT name, sql FROM sqlite_master WHERE type = 'trigger' AND name = ?",
            )
            .bind::<Text, _>(&trigger_name)
            .get_result::<SchemaRow>(conn)
            .optional()?;
            let expected = trigger_sql(name, event);
            if !trigger.is_some_and(|row| {
                row.name == trigger_name
                    && row
                        .sql
                        .is_some_and(|sql| normalized_sql(&sql) == normalized_sql(&expected))
            }) {
                return Err(integrity(format!(
                    "BR-255 canonical trigger definition is missing or changed: {trigger_name}"
                )));
            }
        }
    }
    Ok(())
}

fn parse_date(value: &str, field: &str) -> diesel::QueryResult<NaiveDate> {
    let date = NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|error| integrity(format!("BR-255 invalid {field}: {error}")))?;
    if date.format("%Y-%m-%d").to_string() != value {
        return Err(integrity(format!("BR-255 noncanonical {field}: {value}")));
    }
    Ok(date)
}

fn parse_utc(value: &str, field: &str) -> diesel::QueryResult<DateTime<Utc>> {
    let Some(without_z) = value.strip_suffix('Z') else {
        return Err(integrity(format!("BR-255 {field} is not canonical UTC")));
    };
    let Some((whole, fraction)) = without_z.rsplit_once('.') else {
        return Err(integrity(format!(
            "BR-255 {field} has no fractional seconds"
        )));
    };
    if !whole.contains('T')
        || fraction.is_empty()
        || fraction.len() > 9
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(integrity(format!("BR-255 {field} has noncanonical bytes")));
    }
    let parsed = DateTime::parse_from_rfc3339(value)
        .map_err(|error| integrity(format!("BR-255 invalid {field}: {error}")))?;
    if parsed.offset().local_minus_utc() != 0 {
        return Err(integrity(format!("BR-255 {field} is not UTC")));
    }
    Ok(parsed.with_timezone(&Utc))
}

fn validate_window(created_at: &str, deadline: &str, family: &str) -> diesel::QueryResult<()> {
    let created = parse_utc(created_at, &format!("{family} created_at"))?;
    let retained = parse_utc(deadline, &format!("{family} retention_deadline"))?;
    let minimum = created
        .checked_add_months(Months::new(60))
        .ok_or_else(|| integrity(format!("BR-255 {family} retention overflow")))?;
    if retained < minimum {
        return Err(integrity(format!(
            "BR-255 {family} retention is shorter than 60 natural months"
        )));
    }
    Ok(())
}

fn lower_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn hash_json<T: Serialize>(domain: &[u8], value: &T) -> diesel::QueryResult<String> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| integrity(format!("BR-255 canonical serialization failed: {error}")))?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
    Ok(hex::encode(hasher.finalize()))
}

#[derive(Serialize)]
struct ReceiptPreimage<'a> {
    epoch_id: &'a str,
    cutover_completed_trading_date: &'a str,
    effective_trading_date: &'a str,
    paper_trade_high_water: i64,
    legacy_filled_manifest_hash: &'a str,
    terminal_binding_manifest_hash: &'a str,
    order_audit_high_water: i64,
    order_audit_tip_hash: &'a str,
    calendar_authority_hash: &'a str,
    legacy_carry_manifest_hash: &'a str,
    carry_item_count: i64,
    carry_total_quantity: i64,
    position_projection_hash: &'a str,
    previous_epoch_receipt_hash: Option<&'a str>,
    decision_basis: &'a str,
    created_at: &'a str,
    retention_deadline: &'a str,
}

fn receipt_hash(row: &PersistedReceipt) -> diesel::QueryResult<String> {
    hash_json(
        b"BR255_ATTRIBUTION_EPOCH_RECEIPT_V1\0",
        &ReceiptPreimage {
            epoch_id: &row.epoch_id,
            cutover_completed_trading_date: &row.cutover_completed_trading_date,
            effective_trading_date: &row.effective_trading_date,
            paper_trade_high_water: row.paper_trade_high_water,
            legacy_filled_manifest_hash: &row.legacy_filled_manifest_hash,
            terminal_binding_manifest_hash: &row.terminal_binding_manifest_hash,
            order_audit_high_water: row.order_audit_high_water,
            order_audit_tip_hash: &row.order_audit_tip_hash,
            calendar_authority_hash: &row.calendar_authority_hash,
            legacy_carry_manifest_hash: &row.legacy_carry_manifest_hash,
            carry_item_count: row.carry_item_count,
            carry_total_quantity: row.carry_total_quantity,
            position_projection_hash: &row.position_projection_hash,
            previous_epoch_receipt_hash: row.previous_epoch_receipt_hash.as_deref(),
            decision_basis: &row.decision_basis,
            created_at: &row.created_at,
            retention_deadline: &row.retention_deadline,
        },
    )
}

#[derive(Serialize)]
struct CarryPreimage<'a> {
    epoch_receipt_id: i64,
    code: &'a str,
    quantity: i64,
    item_index: i64,
    predecessor_item_hash: &'a str,
    created_at: &'a str,
    retention_deadline: &'a str,
}

fn carry_hash(row: &PersistedCarry) -> diesel::QueryResult<String> {
    hash_json(
        b"BR255_ATTRIBUTION_LEGACY_CARRY_ITEM_V1\0",
        &CarryPreimage {
            epoch_receipt_id: row.epoch_receipt_id,
            code: &row.code,
            quantity: row.quantity,
            item_index: row.item_index,
            predecessor_item_hash: &row.predecessor_item_hash,
            created_at: &row.created_at,
            retention_deadline: &row.retention_deadline,
        },
    )
}

#[derive(Serialize)]
struct AttemptPreimage<'a> {
    source: &'a str,
    invoked_at: &'a str,
    completed_session_date: Option<&'a str>,
    effective_date: Option<&'a str>,
    outcome: &'a str,
    reason_code: &'a str,
    retryable: i32,
    source_summary_hash: &'a str,
    epoch_id: Option<&'a str>,
    success_receipt_hash: Option<&'a str>,
    predecessor_attempt_hash: &'a str,
    created_at: &'a str,
    retention_deadline: &'a str,
}

fn attempt_hash(row: &PersistedAttempt) -> diesel::QueryResult<String> {
    hash_json(
        b"BR255_ATTRIBUTION_EPOCH_ATTEMPT_V1\0",
        &AttemptPreimage {
            source: &row.source,
            invoked_at: &row.invoked_at,
            completed_session_date: row.completed_session_date.as_deref(),
            effective_date: row.effective_date.as_deref(),
            outcome: &row.outcome,
            reason_code: &row.reason_code,
            retryable: row.retryable,
            source_summary_hash: &row.source_summary_hash,
            epoch_id: row.epoch_id.as_deref(),
            success_receipt_hash: row.success_receipt_hash.as_deref(),
            predecessor_attempt_hash: &row.predecessor_attempt_hash,
            created_at: &row.created_at,
            retention_deadline: &row.retention_deadline,
        },
    )
}

#[derive(Serialize)]
struct DailyPreimage<'a> {
    epoch_id: &'a str,
    date: &'a str,
    signal_family: &'a str,
    payload_json: &'a str,
    payload_hash: &'a str,
    predecessor_daily_hash: &'a str,
    created_at: &'a str,
    retention_deadline: &'a str,
}

fn daily_hash(row: &PersistedDaily) -> diesel::QueryResult<String> {
    hash_json(
        b"BR255_ATTRIBUTION_EPOCH_DAILY_RECORD_V1\0",
        &DailyPreimage {
            epoch_id: &row.epoch_id,
            date: &row.date,
            signal_family: &row.signal_family,
            payload_json: &row.payload_json,
            payload_hash: &row.payload_hash,
            predecessor_daily_hash: &row.predecessor_daily_hash,
            created_at: &row.created_at,
            retention_deadline: &row.retention_deadline,
        },
    )
}

fn load_receipts(conn: &mut SqliteConnection) -> diesel::QueryResult<Vec<PersistedReceipt>> {
    diesel::sql_query(
        "SELECT id, epoch_id, cutover_completed_trading_date, effective_trading_date,
                paper_trade_high_water, legacy_filled_manifest_hash,
                terminal_binding_manifest_hash, order_audit_high_water, order_audit_tip_hash,
                calendar_authority_hash, legacy_carry_manifest_hash, carry_item_count,
                carry_total_quantity, position_projection_hash, previous_epoch_receipt_hash,
                decision_basis, receipt_hash, created_at, retention_deadline
         FROM attribution_sample_epoch_receipt ORDER BY id ASC",
    )
    .load(conn)
}

fn load_carry(conn: &mut SqliteConnection) -> diesel::QueryResult<Vec<PersistedCarry>> {
    diesel::sql_query(
        "SELECT id, epoch_receipt_id, code, quantity, item_index, predecessor_item_hash,
                item_hash, created_at, retention_deadline
         FROM attribution_legacy_carry_item ORDER BY id ASC",
    )
    .load(conn)
}

fn load_attempts(conn: &mut SqliteConnection) -> diesel::QueryResult<Vec<PersistedAttempt>> {
    diesel::sql_query(
        "SELECT id, source, invoked_at, completed_session_date, effective_date, outcome,
                reason_code, retryable, source_summary_hash, epoch_id, success_receipt_hash,
                predecessor_attempt_hash, record_hash, created_at, retention_deadline
         FROM attribution_epoch_attempt_audit ORDER BY id ASC",
    )
    .load(conn)
}

fn load_daily(conn: &mut SqliteConnection) -> diesel::QueryResult<Vec<PersistedDaily>> {
    diesel::sql_query(
        "SELECT id, epoch_id, date, signal_family, payload_json, payload_hash,
                predecessor_daily_hash, record_hash, created_at, retention_deadline
         FROM paper_attribution_epoch_daily ORDER BY id ASC",
    )
    .load(conn)
}

fn load_chain(
    conn: &mut SqliteConnection,
    table: &str,
    fk: &str,
) -> diesel::QueryResult<Vec<PersistedChain>> {
    diesel::sql_query(format!(
        "SELECT id, {fk} AS row_id, previous_hash, record_hash, created_at, retention_deadline
         FROM {table} ORDER BY id ASC"
    ))
    .load(conn)
}

fn validate_sequence(conn: &mut SqliteConnection, table: &str) -> diesel::QueryResult<()> {
    let ids = diesel::sql_query(format!("SELECT id FROM {table} ORDER BY id ASC"))
        .load::<IdRow>(conn)?
        .into_iter()
        .map(|row| row.id)
        .collect::<Vec<_>>();
    let sequence = diesel::sql_query("SELECT seq FROM sqlite_sequence WHERE name = ?")
        .bind::<Text, _>(table)
        .get_result::<SequenceRow>(conn)
        .optional()?;
    let exact = match sequence {
        None => ids.is_empty(),
        Some(SequenceRow { seq: Some(seq) }) if seq > 0 => ids.iter().copied().eq(1..=seq),
        Some(_) => false,
    };
    if !exact {
        return Err(integrity(format!(
            "BR-255 {table} AUTOINCREMENT high-water is inconsistent"
        )));
    }
    Ok(())
}

fn validate_companion<T>(
    rows: &[T],
    row_ids: impl Iterator<Item = i64>,
    row_previous: impl Iterator<Item = String>,
    row_hashes: impl Iterator<Item = String>,
    row_windows: impl Iterator<Item = (String, String)>,
    chain: &[PersistedChain],
    family: &str,
) -> diesel::QueryResult<()> {
    let ids = row_ids.collect::<Vec<_>>();
    let previous = row_previous.collect::<Vec<_>>();
    let hashes = row_hashes.collect::<Vec<_>>();
    let windows = row_windows.collect::<Vec<_>>();
    if rows.len() != chain.len()
        || rows.len() != ids.len()
        || rows.len() != previous.len()
        || rows.len() != hashes.len()
        || rows.len() != windows.len()
    {
        return Err(integrity(format!(
            "BR-255 {family} companion length mismatch"
        )));
    }
    for index in 0..rows.len() {
        let link = &chain[index];
        if link.id
            != i64::try_from(index + 1).map_err(|_| integrity("BR-255 chain index overflow"))?
            || link.row_id != ids[index]
            || link.previous_hash != previous[index]
            || link.record_hash != hashes[index]
            || link.created_at != windows[index].0
            || link.retention_deadline != windows[index].1
        {
            return Err(integrity(format!(
                "BR-255 {family} companion mismatch at row {}",
                ids[index]
            )));
        }
        validate_window(&link.created_at, &link.retention_deadline, family)?;
    }
    Ok(())
}

fn validate_receipts(conn: &mut SqliteConnection) -> diesel::QueryResult<Vec<PersistedReceipt>> {
    let rows = load_receipts(conn)?;
    let chain = load_chain(
        conn,
        "attribution_sample_epoch_receipt_chain",
        "epoch_receipt_id",
    )?;
    let mut expected_previous = RECEIPT_GENESIS.to_owned();
    for row in &rows {
        let cutover = parse_date(
            &row.cutover_completed_trading_date,
            "cutover_completed_trading_date",
        )?;
        let effective = parse_date(&row.effective_trading_date, "effective_trading_date")?;
        validate_window(&row.created_at, &row.retention_deadline, "epoch receipt")?;
        for hash in [
            &row.epoch_id,
            &row.legacy_filled_manifest_hash,
            &row.terminal_binding_manifest_hash,
            &row.order_audit_tip_hash,
            &row.calendar_authority_hash,
            &row.legacy_carry_manifest_hash,
            &row.position_projection_hash,
            &row.receipt_hash,
        ] {
            if !lower_hash(hash) {
                return Err(integrity(format!(
                    "BR-255 invalid receipt hash at row {}",
                    row.id
                )));
            }
        }
        if effective <= cutover
            || row.paper_trade_high_water < 0
            || row.order_audit_high_water < 0
            || row.decision_basis != "BR-255"
            || row.carry_item_count < 0
            || row.carry_total_quantity < 0
            || row.previous_epoch_receipt_hash.as_deref()
                != (row.id > 1).then_some(expected_previous.as_str())
            || row.receipt_hash != receipt_hash(row)?
        {
            return Err(integrity(format!(
                "BR-255 receipt content invalid at row {}",
                row.id
            )));
        }
        expected_previous.clone_from(&row.receipt_hash);
    }
    validate_companion(
        &rows,
        rows.iter().map(|row| row.id),
        rows.iter().map(|row| {
            row.previous_epoch_receipt_hash
                .clone()
                .unwrap_or_else(|| RECEIPT_GENESIS.to_owned())
        }),
        rows.iter().map(|row| row.receipt_hash.clone()),
        rows.iter()
            .map(|row| (row.created_at.clone(), row.retention_deadline.clone())),
        &chain,
        "epoch receipt",
    )?;
    Ok(rows)
}

fn validate_carry(
    conn: &mut SqliteConnection,
    receipts: &[PersistedReceipt],
) -> diesel::QueryResult<()> {
    let rows = load_carry(conn)?;
    let receipt_ids = receipts
        .iter()
        .map(|row| row.id)
        .collect::<std::collections::HashSet<_>>();
    let mut state = std::collections::HashMap::<i64, (i64, String)>::new();
    for row in &rows {
        validate_window(
            &row.created_at,
            &row.retention_deadline,
            "legacy carry item",
        )?;
        let entry = state
            .entry(row.epoch_receipt_id)
            .or_insert_with(|| (0, CARRY_GENESIS.to_owned()));
        if !receipt_ids.contains(&row.epoch_receipt_id)
            || row.item_index != entry.0
            || row.predecessor_item_hash != entry.1
            || row.quantity <= 0
            || row.code.trim().is_empty()
            || row.item_hash != carry_hash(row)?
        {
            return Err(integrity(format!(
                "BR-255 carry chain invalid at row {}",
                row.id
            )));
        }
        entry.0 += 1;
        entry.1.clone_from(&row.item_hash);
    }
    for receipt in receipts {
        let matching = rows
            .iter()
            .filter(|row| row.epoch_receipt_id == receipt.id)
            .collect::<Vec<_>>();
        let count =
            i64::try_from(matching.len()).map_err(|_| integrity("BR-255 carry count overflow"))?;
        let total = matching.iter().try_fold(0_i64, |total, row| {
            total
                .checked_add(row.quantity)
                .ok_or_else(|| integrity("BR-255 carry quantity overflow"))
        })?;
        let positions = matching
            .iter()
            .map(|row| {
                Ok(LegacyCarryPosition {
                    code: row.code.clone(),
                    quantity: u64::try_from(row.quantity)
                        .map_err(|_| integrity("BR-255 carry quantity is not positive"))?,
                })
            })
            .collect::<diesel::QueryResult<Vec<_>>>()?;
        let manifest = canonical_legacy_carry_manifest_hash(&positions);
        if matching.windows(2).any(|pair| pair[0].code >= pair[1].code)
            || count != receipt.carry_item_count
            || total != receipt.carry_total_quantity
            || manifest != receipt.legacy_carry_manifest_hash
        {
            return Err(integrity(format!(
                "BR-255 carry summary mismatch for receipt {}",
                receipt.id
            )));
        }
    }
    Ok(())
}

fn validate_attempts(conn: &mut SqliteConnection) -> diesel::QueryResult<Vec<PersistedAttempt>> {
    let rows = load_attempts(conn)?;
    let chain = load_chain(conn, "attribution_epoch_attempt_chain", "attempt_audit_id")?;
    let mut previous = ATTEMPT_GENESIS.to_owned();
    for row in &rows {
        parse_utc(&row.invoked_at, "attempt invoked_at")?;
        if let Some(date) = row.completed_session_date.as_deref() {
            parse_date(date, "attempt completed_session_date")?;
        }
        if let Some(date) = row.effective_date.as_deref() {
            parse_date(date, "attempt effective_date")?;
        }
        validate_window(&row.created_at, &row.retention_deadline, "epoch attempt")?;
        if row.predecessor_attempt_hash != previous
            || row.record_hash != attempt_hash(row)?
            || !lower_hash(&row.source_summary_hash)
            || !(0..=1).contains(&row.retryable)
            || row.source.trim().is_empty()
            || row.outcome.trim().is_empty()
            || row.reason_code.trim().is_empty()
            || (row.epoch_id.is_some() != row.success_receipt_hash.is_some())
            || row
                .epoch_id
                .as_deref()
                .is_some_and(|value| !lower_hash(value))
            || row
                .success_receipt_hash
                .as_deref()
                .is_some_and(|value| !lower_hash(value))
        {
            return Err(integrity(format!(
                "BR-255 attempt chain invalid at row {}",
                row.id
            )));
        }
        previous.clone_from(&row.record_hash);
    }
    validate_companion(
        &rows,
        rows.iter().map(|row| row.id),
        rows.iter().map(|row| row.predecessor_attempt_hash.clone()),
        rows.iter().map(|row| row.record_hash.clone()),
        rows.iter()
            .map(|row| (row.created_at.clone(), row.retention_deadline.clone())),
        &chain,
        "epoch attempt",
    )?;
    Ok(rows)
}

fn validate_daily(conn: &mut SqliteConnection) -> diesel::QueryResult<Vec<PersistedDaily>> {
    let rows = load_daily(conn)?;
    let chain = load_chain(
        conn,
        "paper_attribution_epoch_daily_chain",
        "epoch_daily_id",
    )?;
    let mut previous = DAILY_GENESIS.to_owned();
    for row in &rows {
        parse_date(&row.date, "epoch daily date")?;
        validate_window(&row.created_at, &row.retention_deadline, "epoch daily")?;
        let parsed: serde_json::Value = serde_json::from_str(&row.payload_json)
            .map_err(|error| integrity(format!("BR-255 invalid daily payload: {error}")))?;
        let canonical = serde_json::to_string(&parsed)
            .map_err(|error| integrity(format!("BR-255 daily payload serialization: {error}")))?;
        let payload_hash = hash_json(b"BR255_ATTRIBUTION_EPOCH_DAILY_PAYLOAD_V1\0", &canonical)?;
        if row.predecessor_daily_hash != previous
            || row.payload_json != canonical
            || row.payload_hash != payload_hash
            || row.record_hash != daily_hash(row)?
            || !lower_hash(&row.epoch_id)
            || row.signal_family.trim().is_empty()
        {
            return Err(integrity(format!(
                "BR-255 daily chain invalid at row {}",
                row.id
            )));
        }
        previous.clone_from(&row.record_hash);
    }
    validate_companion(
        &rows,
        rows.iter().map(|row| row.id),
        rows.iter().map(|row| row.predecessor_daily_hash.clone()),
        rows.iter().map(|row| row.record_hash.clone()),
        rows.iter()
            .map(|row| (row.created_at.clone(), row.retention_deadline.clone())),
        &chain,
        "epoch daily",
    )?;
    Ok(rows)
}

fn validate_all(conn: &mut SqliteConnection) -> diesel::QueryResult<Vec<PersistedReceipt>> {
    validate_schema(conn)?;
    for (table, _) in TABLES {
        validate_sequence(conn, table)?;
    }
    let receipts = validate_receipts(conn)?;
    validate_carry(conn, &receipts)?;
    let attempts = validate_attempts(conn)?;
    for attempt in attempts {
        if let (Some(epoch_id), Some(receipt_hash)) = (
            attempt.epoch_id.as_deref(),
            attempt.success_receipt_hash.as_deref(),
        ) {
            if !receipts
                .iter()
                .any(|receipt| receipt.epoch_id == epoch_id && receipt.receipt_hash == receipt_hash)
            {
                return Err(integrity(format!(
                    "BR-255 attempt {} references an unknown success receipt",
                    attempt.id
                )));
            }
        }
    }
    let daily = validate_daily(conn)?;
    for row in daily {
        if !receipts
            .iter()
            .any(|receipt| receipt.epoch_id == row.epoch_id)
        {
            return Err(integrity(format!(
                "BR-255 daily row {} references an unknown epoch",
                row.id
            )));
        }
    }
    Ok(receipts)
}

pub(super) fn create_schema(conn: &mut SqliteConnection) -> diesel::QueryResult<()> {
    conn.immediate_transaction::<_, diesel::result::Error, _>(|conn| {
        let existing = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM sqlite_master
             WHERE type = 'table' AND name IN (
                'attribution_sample_epoch_receipt',
                'attribution_sample_epoch_receipt_chain',
                'attribution_legacy_carry_item',
                'attribution_epoch_attempt_audit',
                'attribution_epoch_attempt_chain',
                'paper_attribution_epoch_daily',
                'paper_attribution_epoch_daily_chain'
             )",
        )
        .get_result::<CountRow>(conn)?
        .count;
        if existing != 0 {
            validate_schema(conn)?;
        }
        for (table, ddl) in TABLES {
            diesel::sql_query(ddl.replacen("CREATE TABLE", "CREATE TABLE IF NOT EXISTS", 1))
                .execute(conn)?;
            install_triggers(conn, table)?;
        }
        validate_all(conn).map(|_| ())
    })
}

fn receipt_value(row: &PersistedReceipt) -> diesel::QueryResult<AttributionEpochReceipt> {
    Ok(AttributionEpochReceipt {
        epoch_id: row.epoch_id.clone(),
        cutover_completed_trading_date: parse_date(
            &row.cutover_completed_trading_date,
            "cutover_completed_trading_date",
        )?,
        effective_trading_date: parse_date(&row.effective_trading_date, "effective_trading_date")?,
        paper_trade_high_water: row.paper_trade_high_water,
        legacy_filled_manifest_hash: row.legacy_filled_manifest_hash.clone(),
        terminal_binding_manifest_hash: row.terminal_binding_manifest_hash.clone(),
        order_audit_high_water: row.order_audit_high_water,
        order_audit_tip_hash: row.order_audit_tip_hash.clone(),
        calendar_authority_hash: row.calendar_authority_hash.clone(),
        legacy_carry_manifest_hash: row.legacy_carry_manifest_hash.clone(),
        carry_item_count: u64::try_from(row.carry_item_count)
            .map_err(|_| integrity("BR-255 carry item count is negative"))?,
        carry_total_quantity: u64::try_from(row.carry_total_quantity)
            .map_err(|_| integrity("BR-255 carry total quantity is negative"))?,
        position_projection_hash: row.position_projection_hash.clone(),
        previous_epoch_receipt_hash: row.previous_epoch_receipt_hash.clone(),
        receipt_hash: row.receipt_hash.clone(),
        created_at: row.created_at.clone(),
        retention_deadline: row.retention_deadline.clone(),
    })
}

#[allow(dead_code)]
fn new_window(conn: &mut SqliteConnection) -> diesel::QueryResult<RetentionWindow> {
    diesel::sql_query(
        "SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now') AS created_at,
                strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '+60 months') AS retention_deadline",
    )
    .get_result(conn)
}

impl<'a> AttributionEpochStore<'a> {
    pub fn new(database: &'a DatabaseManager) -> Self {
        Self { database }
    }

    pub fn load_selector(
        &self,
        selector: &AttributionEpochSelector,
    ) -> Result<ResolvedAttributionEpoch, AttributionEpochStoreError> {
        if matches!(selector, AttributionEpochSelector::Legacy) {
            return Ok(ResolvedAttributionEpoch::Legacy);
        }
        let mut conn =
            self.database
                .get_conn()
                .map_err(|error| AttributionEpochStoreError::Unavailable {
                    reason_code: "attribution_epoch_storage_unavailable",
                    retryable: true,
                    detail: format!("BR-255 epoch database connection unavailable: {error}"),
                })?;
        let rows = validate_all(&mut conn).map_err(map_integrity)?;
        match selector {
            AttributionEpochSelector::Legacy => unreachable!(),
            AttributionEpochSelector::Active => {
                if rows.is_empty() {
                    return Err(unavailable(
                        "BR-255 active attribution epoch has not been established",
                    ));
                }
                if rows.len() != 1 {
                    return Err(failed_integrity(
                        "BR-255 v1 requires exactly one success receipt",
                    ));
                }
                receipt_value(&rows[0])
                    .map(ResolvedAttributionEpoch::Epoch)
                    .map_err(map_integrity)
            }
            AttributionEpochSelector::Exact(epoch_id) => rows
                .iter()
                .find(|row| row.epoch_id == *epoch_id)
                .ok_or_else(|| {
                    unavailable(format!(
                        "BR-255 exact attribution epoch is unavailable: {epoch_id}"
                    ))
                })
                .and_then(|row| {
                    receipt_value(row)
                        .map(ResolvedAttributionEpoch::Epoch)
                        .map_err(map_integrity)
                }),
        }
    }

    pub fn verify_active(&self) -> Result<AttributionEpochReceipt, AttributionEpochStoreError> {
        match self.load_selector(&AttributionEpochSelector::Active)? {
            ResolvedAttributionEpoch::Epoch(receipt) => Ok(receipt),
            ResolvedAttributionEpoch::Legacy => unreachable!(),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn append_attempt(
        &self,
        input: AttributionEpochAttemptAppend,
    ) -> Result<AttributionEpochAttemptReceipt, AttributionEpochStoreError> {
        if input.source.trim().is_empty()
            || input.outcome.trim().is_empty()
            || input.reason_code.trim().is_empty()
            || !lower_hash(&input.source_summary_hash)
            || input.epoch_id.is_some() != input.success_receipt_hash.is_some()
            || input
                .epoch_id
                .as_deref()
                .is_some_and(|value| !lower_hash(value))
            || input
                .success_receipt_hash
                .as_deref()
                .is_some_and(|value| !lower_hash(value))
        {
            return Err(failed_integrity("BR-255 invalid attempt append input"));
        }
        parse_utc(&input.invoked_at, "attempt invoked_at").map_err(map_integrity)?;
        let mut conn =
            self.database
                .get_conn()
                .map_err(|error| AttributionEpochStoreError::Unavailable {
                    reason_code: "attribution_epoch_storage_unavailable",
                    retryable: true,
                    detail: format!("BR-255 epoch database connection unavailable: {error}"),
                })?;
        conn.immediate_transaction::<_, diesel::result::Error, _>(|conn| {
            let state = validate_attempts(conn)?;
            validate_all(conn)?;
            let previous = state
                .last()
                .map_or(ATTEMPT_GENESIS, |row| row.record_hash.as_str());
            let window = new_window(conn)?;
            let mut row = PersistedAttempt {
                id: 0,
                source: input.source.clone(),
                invoked_at: input.invoked_at.clone(),
                completed_session_date: input.completed_session_date.map(|date| date.to_string()),
                effective_date: input.effective_date.map(|date| date.to_string()),
                outcome: input.outcome.clone(),
                reason_code: input.reason_code.clone(),
                retryable: i32::from(input.retryable),
                source_summary_hash: input.source_summary_hash.clone(),
                epoch_id: input.epoch_id.clone(),
                success_receipt_hash: input.success_receipt_hash.clone(),
                predecessor_attempt_hash: previous.to_owned(),
                record_hash: String::new(),
                created_at: window.created_at.clone(),
                retention_deadline: window.retention_deadline.clone(),
            };
            row.record_hash = attempt_hash(&row)?;
            diesel::sql_query(
                "INSERT INTO attribution_epoch_attempt_audit
                 (source, invoked_at, completed_session_date, effective_date, outcome, reason_code,
                  retryable, source_summary_hash, epoch_id, success_receipt_hash,
                  predecessor_attempt_hash, record_hash, created_at, retention_deadline)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind::<Text, _>(&row.source)
            .bind::<Text, _>(&row.invoked_at)
            .bind::<Nullable<Text>, _>(&row.completed_session_date)
            .bind::<Nullable<Text>, _>(&row.effective_date)
            .bind::<Text, _>(&row.outcome)
            .bind::<Text, _>(&row.reason_code)
            .bind::<Integer, _>(row.retryable)
            .bind::<Text, _>(&row.source_summary_hash)
            .bind::<Nullable<Text>, _>(&row.epoch_id)
            .bind::<Nullable<Text>, _>(&row.success_receipt_hash)
            .bind::<Text, _>(&row.predecessor_attempt_hash)
            .bind::<Text, _>(&row.record_hash)
            .bind::<Text, _>(&row.created_at)
            .bind::<Text, _>(&row.retention_deadline)
            .execute(conn)?;
            row.id = diesel::select(diesel::dsl::sql::<BigInt>("last_insert_rowid()"))
                .get_result(conn)?;
            diesel::sql_query(
                "INSERT INTO attribution_epoch_attempt_chain
                 (attempt_audit_id, previous_hash, record_hash, created_at, retention_deadline)
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind::<BigInt, _>(row.id)
            .bind::<Text, _>(&row.predecessor_attempt_hash)
            .bind::<Text, _>(&row.record_hash)
            .bind::<Text, _>(&row.created_at)
            .bind::<Text, _>(&row.retention_deadline)
            .execute(conn)?;
            validate_all(conn)?;
            Ok(AttributionEpochAttemptReceipt {
                attempt_audit_id: row.id,
                record_hash: row.record_hash,
                created_at: row.created_at,
                retention_deadline: row.retention_deadline,
            })
        })
        .map_err(map_integrity)
    }

    #[allow(dead_code)]
    pub(crate) fn append_daily(
        &self,
        input: AttributionEpochDailyAppend,
    ) -> Result<AttributionEpochDailyReceipt, AttributionEpochStoreError> {
        if !lower_hash(&input.epoch_id) || input.signal_family.trim().is_empty() {
            return Err(failed_integrity("BR-255 invalid epoch daily append input"));
        }
        let payload_json = serde_json::to_string(&input.payload).map_err(|error| {
            failed_integrity(format!("BR-255 serialize daily payload: {error}"))
        })?;
        let payload_hash = hash_json(b"BR255_ATTRIBUTION_EPOCH_DAILY_PAYLOAD_V1\0", &payload_json)
            .map_err(map_integrity)?;
        let mut conn =
            self.database
                .get_conn()
                .map_err(|error| AttributionEpochStoreError::Unavailable {
                    reason_code: "attribution_epoch_storage_unavailable",
                    retryable: true,
                    detail: format!("BR-255 epoch database connection unavailable: {error}"),
                })?;
        conn.immediate_transaction::<_, diesel::result::Error, _>(|conn| {
            let state = validate_daily(conn)?;
            let receipts = validate_all(conn)?;
            if !receipts
                .iter()
                .any(|receipt| receipt.epoch_id == input.epoch_id)
            {
                return Err(integrity(
                    "BR-255 epoch daily append references an unknown epoch",
                ));
            }
            if let Some(existing) = state.iter().find(|row| {
                row.epoch_id == input.epoch_id
                    && row.date == input.date.to_string()
                    && row.signal_family == input.signal_family
                    && row.payload_hash == payload_hash
            }) {
                return Ok(AttributionEpochDailyReceipt {
                    epoch_daily_id: existing.id,
                    payload_hash: existing.payload_hash.clone(),
                    record_hash: existing.record_hash.clone(),
                    created_at: existing.created_at.clone(),
                    retention_deadline: existing.retention_deadline.clone(),
                });
            }
            let previous = state
                .last()
                .map_or(DAILY_GENESIS, |row| row.record_hash.as_str());
            let window = new_window(conn)?;
            let mut row = PersistedDaily {
                id: 0,
                epoch_id: input.epoch_id.clone(),
                date: input.date.to_string(),
                signal_family: input.signal_family.clone(),
                payload_json,
                payload_hash,
                predecessor_daily_hash: previous.to_owned(),
                record_hash: String::new(),
                created_at: window.created_at,
                retention_deadline: window.retention_deadline,
            };
            row.record_hash = daily_hash(&row)?;
            diesel::sql_query(
                "INSERT INTO paper_attribution_epoch_daily
                 (epoch_id, date, signal_family, payload_json, payload_hash,
                  predecessor_daily_hash, record_hash, created_at, retention_deadline)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind::<Text, _>(&row.epoch_id)
            .bind::<Text, _>(&row.date)
            .bind::<Text, _>(&row.signal_family)
            .bind::<Text, _>(&row.payload_json)
            .bind::<Text, _>(&row.payload_hash)
            .bind::<Text, _>(&row.predecessor_daily_hash)
            .bind::<Text, _>(&row.record_hash)
            .bind::<Text, _>(&row.created_at)
            .bind::<Text, _>(&row.retention_deadline)
            .execute(conn)?;
            row.id = diesel::select(diesel::dsl::sql::<BigInt>("last_insert_rowid()"))
                .get_result(conn)?;
            diesel::sql_query(
                "INSERT INTO paper_attribution_epoch_daily_chain
                 (epoch_daily_id, previous_hash, record_hash, created_at, retention_deadline)
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind::<BigInt, _>(row.id)
            .bind::<Text, _>(&row.predecessor_daily_hash)
            .bind::<Text, _>(&row.record_hash)
            .bind::<Text, _>(&row.created_at)
            .bind::<Text, _>(&row.retention_deadline)
            .execute(conn)?;
            validate_all(conn)?;
            Ok(AttributionEpochDailyReceipt {
                epoch_daily_id: row.id,
                payload_hash: row.payload_hash,
                record_hash: row.record_hash,
                created_at: row.created_at,
                retention_deadline: row.retention_deadline,
            })
        })
        .map_err(map_integrity)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use diesel::connection::SimpleConnection;
    use diesel::prelude::*;
    use diesel::sql_types::Text;

    use super::*;

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
                "TEST_CODE_attribution_epochs_{}_{}.sqlite",
                std::process::id(),
                nonce
            ));
            let database_url = path.to_string_lossy().into_owned();
            let pool = super::super::build_sqlite_pool_with_size(database_url, 1)
                .expect("TEST_CODE isolated SQLite pool");
            {
                let mut conn = pool.get().expect("TEST_CODE schema connection");
                create_schema(&mut conn).expect("TEST_CODE epoch schema");
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
    }

    impl Drop for TestDatabase {
        fn drop(&mut self) {
            for suffix in ["", "-wal", "-shm"] {
                let candidate = PathBuf::from(format!("{}{}", self.path.display(), suffix));
                if let Err(error) = std::fs::remove_file(&candidate) {
                    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
                }
            }
        }
    }

    #[test]
    fn schema_installs_every_epoch_fact_and_chain_without_a_success_fact() {
        let mut conn = SqliteConnection::establish(":memory:")
            .expect("TEST_CODE establish isolated epoch schema database");
        create_schema(&mut conn).expect("TEST_CODE install epoch schema");
        validate_all(&mut conn).expect("TEST_CODE validate empty epoch state");
        for (table, _) in TABLES {
            let count = diesel::sql_query(
                "SELECT COUNT(*) AS count FROM sqlite_master WHERE type='table' AND name=?",
            )
            .bind::<Text, _>(table)
            .get_result::<CountRow>(&mut conn)
            .expect("TEST_CODE table count")
            .count;
            assert_eq!(count, 1, "TEST_CODE missing {table}");
        }
        assert!(load_receipts(&mut conn).unwrap().is_empty());
    }

    #[test]
    fn legacy_only_database_installs_complete_schema_without_success() {
        let mut conn = SqliteConnection::establish(":memory:").unwrap();
        diesel::sql_query("CREATE TABLE paper_attribution_daily(date TEXT PRIMARY KEY)")
            .execute(&mut conn)
            .unwrap();
        create_schema(&mut conn).expect("TEST_CODE install alongside legacy table");
        assert!(validate_all(&mut conn).unwrap().is_empty());
    }

    #[test]
    fn partial_epoch_schema_is_integrity_failure_instead_of_silent_completion() {
        let mut conn = SqliteConnection::establish(":memory:").unwrap();
        diesel::sql_query(RECEIPT_TABLE).execute(&mut conn).unwrap();
        assert!(create_schema(&mut conn).is_err());
    }

    #[test]
    fn canonical_triggers_block_mutation_and_noop_replacement_fails_closed() {
        let database = TestDatabase::new();
        let store = AttributionEpochStore::new(&database.manager);
        store
            .append_attempt(sample_attempt())
            .expect("TEST_CODE append attempt");
        let mut conn = database.manager.get_conn().unwrap();
        assert!(diesel::sql_query(
            "UPDATE attribution_epoch_attempt_audit SET source=source WHERE id=1"
        )
        .execute(&mut conn)
        .is_err());
        assert!(
            diesel::sql_query("DELETE FROM attribution_epoch_attempt_audit WHERE id=1")
                .execute(&mut conn)
                .is_err()
        );
        conn.batch_execute(
            "DROP TRIGGER trg_attribution_epoch_attempt_audit_no_update;
             CREATE TRIGGER trg_attribution_epoch_attempt_audit_no_update
             BEFORE UPDATE ON attribution_epoch_attempt_audit BEGIN SELECT 1; END;",
        )
        .unwrap();
        drop(conn);
        let error = store
            .load_selector(&AttributionEpochSelector::Active)
            .expect_err("TEST_CODE no-op trigger must fail closed");
        assert_eq!(error.reason_code(), "attribution_epoch_integrity_failed");
    }

    #[test]
    fn sequence_retention_and_canonical_timestamp_tamper_fail_closed() {
        for statement in [
            "UPDATE sqlite_sequence SET seq=0 WHERE name='attribution_epoch_attempt_audit'",
            "UPDATE attribution_epoch_attempt_audit SET retention_deadline=created_at WHERE id=1",
            "UPDATE attribution_epoch_attempt_audit SET created_at='2026-08-28 00:00:00' WHERE id=1",
        ] {
            let database = TestDatabase::new();
            let store = AttributionEpochStore::new(&database.manager);
            store.append_attempt(sample_attempt()).unwrap();
            let mut conn = database.manager.get_conn().unwrap();
            if statement.contains("attribution_epoch_attempt_audit SET") {
                diesel::sql_query("DROP TRIGGER trg_attribution_epoch_attempt_audit_no_update")
                    .execute(&mut conn)
                    .unwrap();
            }
            diesel::sql_query(statement).execute(&mut conn).unwrap();
            if statement.contains("attribution_epoch_attempt_audit SET") {
                install_triggers(&mut conn, "attribution_epoch_attempt_audit").unwrap();
            }
            drop(conn);
            assert_eq!(
                store.verify_active().unwrap_err().reason_code(),
                "attribution_epoch_integrity_failed"
            );
        }
    }

    #[test]
    fn attempt_and_daily_append_write_target_and_one_to_one_chain_tables() {
        let database = TestDatabase::new();
        let store = AttributionEpochStore::new(&database.manager);
        let attempt = store.append_attempt(sample_attempt()).unwrap();
        assert_eq!(attempt.attempt_audit_id, 1);
        insert_success(&database.manager, None);
        let epoch_id = "a".repeat(64);
        let daily = store
            .append_daily(AttributionEpochDailyAppend {
                epoch_id: epoch_id.clone(),
                date: NaiveDate::from_ymd_opt(2026, 8, 28).unwrap(),
                signal_family: "all".to_owned(),
                payload: serde_json::json!({"closed": 1}),
            })
            .unwrap();
        let reused = store
            .append_daily(AttributionEpochDailyAppend {
                epoch_id,
                date: NaiveDate::from_ymd_opt(2026, 8, 28).unwrap(),
                signal_family: "all".to_owned(),
                payload: serde_json::json!({"closed": 1}),
            })
            .unwrap();
        assert_eq!(daily, reused);
        let revised = store
            .append_daily(AttributionEpochDailyAppend {
                epoch_id: "a".repeat(64),
                date: NaiveDate::from_ymd_opt(2026, 8, 28).unwrap(),
                signal_family: "all".to_owned(),
                payload: serde_json::json!({"closed": 2}),
            })
            .unwrap();
        assert_eq!(revised.epoch_daily_id, 2);
        let mut conn = database.manager.get_conn().unwrap();
        for (table, expected) in [
            ("attribution_epoch_attempt_audit", 1),
            ("attribution_epoch_attempt_chain", 1),
            ("paper_attribution_epoch_daily", 2),
            ("paper_attribution_epoch_daily_chain", 2),
        ] {
            let count = diesel::sql_query(format!("SELECT COUNT(*) AS count FROM {table}"))
                .get_result::<CountRow>(&mut conn)
                .unwrap()
                .count;
            assert_eq!(count, expected, "TEST_CODE one-to-one append for {table}");
        }
    }

    #[test]
    fn companion_chain_break_fails_closed() {
        let database = TestDatabase::new();
        let store = AttributionEpochStore::new(&database.manager);
        store.append_attempt(sample_attempt()).unwrap();
        let mut conn = database.manager.get_conn().unwrap();
        diesel::sql_query("DROP TRIGGER trg_attribution_epoch_attempt_chain_no_delete")
            .execute(&mut conn)
            .unwrap();
        diesel::sql_query("DELETE FROM attribution_epoch_attempt_chain")
            .execute(&mut conn)
            .unwrap();
        install_triggers(&mut conn, "attribution_epoch_attempt_chain").unwrap();
        drop(conn);
        assert_eq!(
            store.verify_active().unwrap_err().reason_code(),
            "attribution_epoch_integrity_failed"
        );
    }

    fn sample_attempt() -> AttributionEpochAttemptAppend {
        AttributionEpochAttemptAppend {
            source: "monitor".to_owned(),
            invoked_at: "2026-08-28T07:40:00.000Z".to_owned(),
            completed_session_date: Some(NaiveDate::from_ymd_opt(2026, 8, 28).unwrap()),
            effective_date: Some(NaiveDate::from_ymd_opt(2026, 8, 31).unwrap()),
            outcome: "unavailable".to_owned(),
            reason_code: "TEST_CODE_window_closed".to_owned(),
            retryable: true,
            source_summary_hash: "1".repeat(64),
            epoch_id: None,
            success_receipt_hash: None,
        }
    }

    fn insert_success(manager: &DatabaseManager, previous: Option<String>) -> String {
        let mut conn = manager.get_conn().unwrap();
        let epoch_id = if previous.is_some() {
            "b".repeat(64)
        } else {
            "a".repeat(64)
        };
        let mut row = sample_persisted_receipt(
            epoch_id,
            previous,
            0,
            0,
            canonical_legacy_carry_manifest_hash(&[]),
        );
        insert_persisted_receipt(&mut conn, &mut row);
        row.receipt_hash
    }

    fn sample_persisted_receipt(
        epoch_id: String,
        previous: Option<String>,
        carry_item_count: i64,
        carry_total_quantity: i64,
        legacy_carry_manifest_hash: String,
    ) -> PersistedReceipt {
        PersistedReceipt {
            id: 0,
            epoch_id,
            cutover_completed_trading_date: "2026-08-28".to_owned(),
            effective_trading_date: "2026-08-31".to_owned(),
            paper_trade_high_water: 12,
            legacy_filled_manifest_hash: "1".repeat(64),
            terminal_binding_manifest_hash: "2".repeat(64),
            order_audit_high_water: 14,
            order_audit_tip_hash: "3".repeat(64),
            calendar_authority_hash: "4".repeat(64),
            legacy_carry_manifest_hash,
            carry_item_count,
            carry_total_quantity,
            position_projection_hash: "6".repeat(64),
            previous_epoch_receipt_hash: previous,
            decision_basis: "BR-255".to_owned(),
            receipt_hash: String::new(),
            created_at: "2026-08-28T08:00:00.000Z".to_owned(),
            retention_deadline: "2031-08-28T08:00:00.000Z".to_owned(),
        }
    }

    fn insert_persisted_receipt(conn: &mut SqliteConnection, row: &mut PersistedReceipt) {
        row.receipt_hash = receipt_hash(&row).unwrap();
        diesel::sql_query(
            "INSERT INTO attribution_sample_epoch_receipt
             (epoch_id, cutover_completed_trading_date, effective_trading_date,
              paper_trade_high_water, legacy_filled_manifest_hash,
              terminal_binding_manifest_hash, order_audit_high_water, order_audit_tip_hash,
              calendar_authority_hash, legacy_carry_manifest_hash, carry_item_count,
              carry_total_quantity, position_projection_hash, previous_epoch_receipt_hash,
              decision_basis, receipt_hash, created_at, retention_deadline)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind::<Text, _>(&row.epoch_id)
        .bind::<Text, _>(&row.cutover_completed_trading_date)
        .bind::<Text, _>(&row.effective_trading_date)
        .bind::<BigInt, _>(row.paper_trade_high_water)
        .bind::<Text, _>(&row.legacy_filled_manifest_hash)
        .bind::<Text, _>(&row.terminal_binding_manifest_hash)
        .bind::<BigInt, _>(row.order_audit_high_water)
        .bind::<Text, _>(&row.order_audit_tip_hash)
        .bind::<Text, _>(&row.calendar_authority_hash)
        .bind::<Text, _>(&row.legacy_carry_manifest_hash)
        .bind::<BigInt, _>(row.carry_item_count)
        .bind::<BigInt, _>(row.carry_total_quantity)
        .bind::<Text, _>(&row.position_projection_hash)
        .bind::<Nullable<Text>, _>(&row.previous_epoch_receipt_hash)
        .bind::<Text, _>(&row.decision_basis)
        .bind::<Text, _>(&row.receipt_hash)
        .bind::<Text, _>(&row.created_at)
        .bind::<Text, _>(&row.retention_deadline)
        .execute(&mut *conn)
        .unwrap();
        row.id = diesel::select(diesel::dsl::sql::<BigInt>("last_insert_rowid()"))
            .get_result(&mut *conn)
            .unwrap();
        diesel::sql_query(
            "INSERT INTO attribution_sample_epoch_receipt_chain
             (epoch_receipt_id, previous_hash, record_hash, created_at, retention_deadline)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind::<BigInt, _>(row.id)
        .bind::<Text, _>(
            row.previous_epoch_receipt_hash
                .as_deref()
                .unwrap_or(RECEIPT_GENESIS),
        )
        .bind::<Text, _>(&row.receipt_hash)
        .bind::<Text, _>(&row.created_at)
        .bind::<Text, _>(&row.retention_deadline)
        .execute(conn)
        .unwrap();
    }

    #[test]
    fn active_rejects_multiple_successes_and_bad_tail_never_falls_back() {
        let database = TestDatabase::new();
        let first_hash = insert_success(&database.manager, None);
        let first_epoch = "a".repeat(64);
        assert!(matches!(
            AttributionEpochStore::new(&database.manager)
                .verify_active()
                .unwrap(),
            AttributionEpochReceipt { epoch_id, .. } if epoch_id == first_epoch
        ));
        let second_hash = insert_success(&database.manager, Some(first_hash));
        let store = AttributionEpochStore::new(&database.manager);
        assert_eq!(
            store.verify_active().unwrap_err().reason_code(),
            "attribution_epoch_integrity_failed"
        );
        assert!(matches!(
            store
                .load_selector(&AttributionEpochSelector::Exact(first_epoch))
                .unwrap(),
            ResolvedAttributionEpoch::Epoch(_)
        ));

        let mut conn = database.manager.get_conn().unwrap();
        diesel::sql_query("DROP TRIGGER trg_attribution_sample_epoch_receipt_chain_no_update")
            .execute(&mut conn)
            .unwrap();
        diesel::sql_query(
            "UPDATE attribution_sample_epoch_receipt_chain SET record_hash=? WHERE id=2",
        )
        .bind::<Text, _>("f".repeat(64))
        .execute(&mut conn)
        .unwrap();
        install_triggers(&mut conn, "attribution_sample_epoch_receipt_chain").unwrap();
        drop(conn);
        assert_ne!(second_hash, "f".repeat(64));
        assert_eq!(
            store.verify_active().unwrap_err().reason_code(),
            "attribution_epoch_integrity_failed"
        );
    }

    #[test]
    fn active_without_success_is_typed_unavailable_but_legacy_is_explicit() {
        let database = TestDatabase::new();
        let store = AttributionEpochStore::new(&database.manager);
        assert_eq!(
            store.verify_active().unwrap_err().reason_code(),
            "attribution_epoch_unavailable"
        );
        assert_eq!(
            store
                .load_selector(&AttributionEpochSelector::Legacy)
                .unwrap(),
            ResolvedAttributionEpoch::Legacy
        );
    }

    #[test]
    fn carry_manifest_and_item_hash_chain_are_fully_verified() {
        let database = TestDatabase::new();
        let mut conn = database.manager.get_conn().unwrap();
        let carry = vec![LegacyCarryPosition {
            code: "600000".to_owned(),
            quantity: 100,
        }];
        let mut receipt = sample_persisted_receipt(
            "a".repeat(64),
            None,
            1,
            100,
            canonical_legacy_carry_manifest_hash(&carry),
        );
        insert_persisted_receipt(&mut conn, &mut receipt);
        let mut item = PersistedCarry {
            id: 0,
            epoch_receipt_id: receipt.id,
            code: carry[0].code.clone(),
            quantity: 100,
            item_index: 0,
            predecessor_item_hash: CARRY_GENESIS.to_owned(),
            item_hash: String::new(),
            created_at: receipt.created_at.clone(),
            retention_deadline: receipt.retention_deadline.clone(),
        };
        item.item_hash = carry_hash(&item).unwrap();
        diesel::sql_query(
            "INSERT INTO attribution_legacy_carry_item
             (epoch_receipt_id, code, quantity, item_index, predecessor_item_hash,
              item_hash, created_at, retention_deadline)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind::<BigInt, _>(item.epoch_receipt_id)
        .bind::<Text, _>(&item.code)
        .bind::<BigInt, _>(item.quantity)
        .bind::<BigInt, _>(item.item_index)
        .bind::<Text, _>(&item.predecessor_item_hash)
        .bind::<Text, _>(&item.item_hash)
        .bind::<Text, _>(&item.created_at)
        .bind::<Text, _>(&item.retention_deadline)
        .execute(&mut conn)
        .unwrap();
        validate_all(&mut conn).unwrap();
        diesel::sql_query("DROP TRIGGER trg_attribution_legacy_carry_item_no_update")
            .execute(&mut conn)
            .unwrap();
        diesel::sql_query("UPDATE attribution_legacy_carry_item SET item_hash=? WHERE id=1")
            .bind::<Text, _>("f".repeat(64))
            .execute(&mut conn)
            .unwrap();
        install_triggers(&mut conn, "attribution_legacy_carry_item").unwrap();
        drop(conn);
        assert_eq!(
            AttributionEpochStore::new(&database.manager)
                .verify_active()
                .unwrap_err()
                .reason_code(),
            "attribution_epoch_integrity_failed"
        );
    }
}
