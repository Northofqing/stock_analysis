//! BR-255 immutable attribution sample epoch storage.
//!
//! This is the only module allowed to know the epoch/carry/attempt/daily SQL.
//! Every read and append verifies the complete retained state before returning.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use chrono::{DateTime, FixedOffset, Months, NaiveDate, NaiveTime, SecondsFormat, Utc};
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Double, Integer, Nullable, Text};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::{DatabaseAuthorityError, DatabaseConnectionAuthority, DatabaseManager};
use crate::database::order_audit::{
    validate_canonical_order_audit_chain, CanonicalOrderAuditChainRow, CanonicalOrderAuditRow,
    AUDIT_CHAIN_GENESIS,
};
use crate::performance::attribution_epoch::{
    build_legacy_carry, canonical_exclusion_manifest_hash, canonical_legacy_carry_manifest_hash,
    canonical_scoped_fill_manifest_hash, scope_epoch_fills, AttributionEpochSelector,
    EpochActivationSource, LegacyCarryPosition,
};
use crate::performance::economic_position::EconomicFillRow;
use crate::trading::paper_lot_ledger::parse_paper_fill_timestamp;

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
    CHECK(
        (outcome = 'success' AND epoch_id IS NOT NULL AND success_receipt_hash IS NOT NULL)
        OR
        (outcome <> 'success' AND epoch_id IS NULL AND success_receipt_hash IS NULL)
    )
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

#[derive(Debug, Clone)]
pub struct EpochActivationRequest {
    pub source: EpochActivationSource,
    pub invoked_at: DateTime<FixedOffset>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EpochActivationPreview {
    /// True only when this preview describes the already-frozen first epoch.
    pub activated: bool,
    pub epoch_id: String,
    pub completed_session_date: NaiveDate,
    pub effective_date: NaiveDate,
    pub paper_trade_high_water: i64,
    pub order_audit_high_water: i64,
    pub carry: Vec<LegacyCarryPosition>,
    pub legacy_filled_manifest_hash: String,
    pub terminal_binding_manifest_hash: String,
    pub order_audit_tip_hash: String,
    pub position_projection_hash: String,
    pub calendar_authority_hash: String,
    pub receipt_hash: Option<String>,
}

/// Domain-verified activation result whose receipt and render projection were
/// frozen from the same activation transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpochActivationOutcome {
    receipt: AttributionEpochReceipt,
    projection: EpochActivationPreview,
}

impl EpochActivationOutcome {
    pub fn receipt(&self) -> &AttributionEpochReceipt {
        &self.receipt
    }

    pub fn projection(&self) -> &EpochActivationPreview {
        &self.projection
    }

    fn into_receipt(self) -> AttributionEpochReceipt {
        self.receipt
    }
}

#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)] // Task 5 consumes the verified deep-module source capability.
pub(crate) struct VerifiedEpochFill {
    fill: EconomicFillRow,
    terminal_audit_id: i64,
    terminal_audit_hash: String,
    terminal_time: DateTime<FixedOffset>,
}

#[allow(dead_code)] // Task 5 consumes these immutable source accessors.
impl VerifiedEpochFill {
    pub(crate) fn fill(&self) -> &EconomicFillRow {
        &self.fill
    }

    pub(crate) fn terminal_audit_id(&self) -> i64 {
        self.terminal_audit_id
    }

    pub(crate) fn terminal_audit_hash(&self) -> &str {
        &self.terminal_audit_hash
    }

    pub(crate) fn terminal_time(&self) -> DateTime<FixedOffset> {
        self.terminal_time
    }
}

#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)] // Task 5 consumes the verified deep-module source capability.
pub(crate) struct VerifiedEpochFillSet {
    fills: Vec<VerifiedEpochFill>,
    carry: Vec<LegacyCarryPosition>,
    current_paper_trade_high_water: i64,
    current_order_audit_high_water: i64,
    all_status_paper_manifest_hash: String,
    filled_manifest_hash: String,
    terminal_binding_manifest_hash: String,
    order_audit_tip_hash: String,
}

#[allow(dead_code)] // Task 5 consumes these immutable source accessors.
impl VerifiedEpochFillSet {
    pub(crate) fn fills(&self) -> &[VerifiedEpochFill] {
        &self.fills
    }

    pub(crate) fn carry(&self) -> &[LegacyCarryPosition] {
        &self.carry
    }

    pub(crate) fn current_paper_trade_high_water(&self) -> i64 {
        self.current_paper_trade_high_water
    }

    pub(crate) fn current_order_audit_high_water(&self) -> i64 {
        self.current_order_audit_high_water
    }

    pub(crate) fn all_status_paper_manifest_hash(&self) -> &str {
        &self.all_status_paper_manifest_hash
    }

    pub(crate) fn filled_manifest_hash(&self) -> &str {
        &self.filled_manifest_hash
    }

    pub(crate) fn terminal_binding_manifest_hash(&self) -> &str {
        &self.terminal_binding_manifest_hash
    }

    pub(crate) fn order_audit_tip_hash(&self) -> &str {
        &self.order_audit_tip_hash
    }
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

impl From<diesel::result::Error> for AttributionEpochStoreError {
    fn from(error: diesel::result::Error) -> Self {
        let detail = error.to_string();
        let lowercase = detail.to_ascii_lowercase();
        if lowercase.contains("database is locked")
            || lowercase.contains("database is busy")
            || lowercase.contains("sqlite_busy")
            || lowercase.contains("sqlite_locked")
        {
            Self::Unavailable {
                reason_code: "attribution_epoch_storage_busy",
                retryable: true,
                detail: format!("BR-255 SQLite activation store is busy: {detail}"),
            }
        } else {
            failed_integrity(format!("BR-255 attribution activation source: {detail}"))
        }
    }
}

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

#[derive(Debug, Clone)]
pub(crate) struct AttributionEpochDailyFamilyAppend {
    pub(crate) signal_family: String,
    pub(crate) payload: serde_json::Value,
}

#[derive(Debug, Clone)]
pub(crate) struct AttributionEpochDailyBatchAppend {
    pub(crate) epoch_id: String,
    pub(crate) date: NaiveDate,
    pub(crate) families: Vec<AttributionEpochDailyFamilyAppend>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AttributionEpochDailySourceBinding {
    pub(crate) database_authority: DatabaseConnectionAuthority,
    pub(crate) epoch_id: String,
    pub(crate) receipt_hash: String,
    pub(crate) effective_date: NaiveDate,
    pub(crate) cutoff_date: NaiveDate,
    pub(crate) frozen_paper_trade_high_water: i64,
    pub(crate) frozen_order_audit_high_water: i64,
    pub(crate) source_paper_trade_high_water: i64,
    pub(crate) source_order_audit_high_water: i64,
    pub(crate) all_status_paper_manifest_hash: String,
    pub(crate) legacy_carry_manifest_hash: String,
    pub(crate) verified_filled_manifest_hash: String,
    pub(crate) verified_terminal_binding_manifest_hash: String,
    pub(crate) verified_order_audit_tip_hash: String,
    pub(crate) exclusion_manifest_hash: String,
    pub(crate) scoped_fill_manifest_hash: String,
    pub(crate) remaining_quarantine_manifest_hash: String,
    pub(crate) released_codes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // Returned by the Task 7 daily persistence seam.
pub(crate) struct AttributionEpochDailyReceipt {
    pub(crate) epoch_daily_id: i64,
    pub(crate) signal_family: String,
    pub(crate) revision: u64,
    pub(crate) payload_hash: String,
    pub(crate) record_hash: String,
    pub(crate) created_at: String,
    pub(crate) retention_deadline: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AttributionEpochDailyBatchReceipt {
    pub(crate) epoch_id: String,
    pub(crate) date: NaiveDate,
    pub(crate) receipts: Vec<AttributionEpochDailyReceipt>,
}

pub struct AttributionEpochStore<'a> {
    database: &'a DatabaseManager,
    read_only_preview_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PreviewFileSnapshot {
    byte_len: u64,
    byte_hash: String,
    modified: std::time::SystemTime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PreviewSqliteSnapshot {
    main: PreviewFileSnapshot,
    wal: Option<PreviewFileSnapshot>,
    shm: Option<PreviewFileSnapshot>,
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

#[derive(Debug, Clone, QueryableByName, Serialize)]
struct FrozenPaperFill {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = Text)]
    plan_id: String,
    #[diesel(sql_type = Text)]
    code: String,
    #[diesel(sql_type = Text)]
    name: String,
    #[diesel(sql_type = Text)]
    direction: String,
    #[diesel(sql_type = Double)]
    requested_price: f64,
    #[diesel(sql_type = Text)]
    status: String,
    #[diesel(sql_type = Nullable<Double>)]
    fill_price: Option<f64>,
    #[diesel(sql_type = Nullable<Text>)]
    not_fill_reason: Option<String>,
    #[diesel(sql_type = BigInt)]
    quantity: i64,
    #[diesel(sql_type = Text)]
    occurred_at: String,
    #[diesel(sql_type = Text)]
    virtual_reason: String,
    #[diesel(sql_type = Text)]
    account_mode: String,
    #[diesel(sql_type = Text)]
    data_mode: String,
    #[diesel(sql_type = Text)]
    updated_at: String,
}

impl FrozenPaperFill {
    fn economic(&self) -> EconomicFillRow {
        EconomicFillRow {
            id: self.id,
            plan_id: self.plan_id.clone(),
            code: self.code.clone(),
            name: self.name.clone(),
            direction: self.direction.clone(),
            fill_price: self.fill_price,
            quantity: self.quantity,
            occurred_at: self.occurred_at.clone(),
            virtual_reason: self.virtual_reason.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct TerminalBindingManifestItem {
    paper_trade_id: i64,
    terminal_audit_id: i64,
    terminal_audit_hash: String,
    terminal_time: String,
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // Fill/binding rows are consumed by the Task 5 loader seam.
struct FrozenSourceProjection {
    paper_trade_high_water: i64,
    order_audit_high_water: i64,
    all_status_paper_manifest_hash: String,
    fills: Vec<FrozenPaperFill>,
    bindings: Vec<TerminalBindingManifestItem>,
    carry: Vec<LegacyCarryPosition>,
    legacy_filled_manifest_hash: String,
    terminal_binding_manifest_hash: String,
    order_audit_tip_hash: String,
    position_projection_hash: String,
}

#[derive(QueryableByName)]
struct DatabaseFileRow {
    #[diesel(sql_type = Text)]
    file: String,
}

fn sqlite_sidecar_path(database_path: &Path, suffix: &str) -> PathBuf {
    let mut leaf = database_path.as_os_str().to_os_string();
    leaf.push(suffix);
    PathBuf::from(leaf)
}

fn preview_file_snapshot(path: &Path) -> Result<PreviewFileSnapshot, AttributionEpochStoreError> {
    let metadata = std::fs::metadata(path).map_err(|error| {
        activation_unavailable(
            "attribution_epoch_preview_storage_unavailable",
            true,
            format!("BR-255 cannot stat read-only preview file {path:?}: {error}"),
        )
    })?;
    if !metadata.is_file() {
        return Err(activation_unavailable(
            "attribution_epoch_preview_storage_unavailable",
            false,
            format!("BR-255 read-only preview target is not a regular file: {path:?}"),
        ));
    }
    let modified = metadata.modified().map_err(|error| {
        activation_unavailable(
            "attribution_epoch_preview_storage_unavailable",
            true,
            format!("BR-255 cannot read preview mtime for {path:?}: {error}"),
        )
    })?;
    let mut file = File::open(path).map_err(|error| {
        activation_unavailable(
            "attribution_epoch_preview_storage_unavailable",
            true,
            format!("BR-255 cannot open preview file {path:?}: {error}"),
        )
    })?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| {
            activation_unavailable(
                "attribution_epoch_preview_storage_unavailable",
                true,
                format!("BR-255 cannot hash preview file {path:?}: {error}"),
            )
        })?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(PreviewFileSnapshot {
        byte_len: metadata.len(),
        byte_hash: hex::encode(digest.finalize()),
        modified,
    })
}

fn optional_preview_file_snapshot(
    path: &Path,
) -> Result<Option<PreviewFileSnapshot>, AttributionEpochStoreError> {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => preview_file_snapshot(path).map(Some),
        Ok(_) => Err(activation_unavailable(
            "attribution_epoch_preview_storage_unavailable",
            false,
            format!("BR-255 SQLite preview sidecar is not a regular file: {path:?}"),
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(activation_unavailable(
            "attribution_epoch_preview_storage_unavailable",
            true,
            format!("BR-255 cannot stat SQLite preview sidecar {path:?}: {error}"),
        )),
    }
}

fn preview_sqlite_snapshot(
    database_path: &Path,
) -> Result<PreviewSqliteSnapshot, AttributionEpochStoreError> {
    Ok(PreviewSqliteSnapshot {
        main: preview_file_snapshot(database_path)?,
        wal: optional_preview_file_snapshot(&sqlite_sidecar_path(database_path, "-wal"))?,
        shm: optional_preview_file_snapshot(&sqlite_sidecar_path(database_path, "-shm"))?,
    })
}

fn reject_preview_wal_state(
    _database_path: &Path,
    snapshot: &PreviewSqliteSnapshot,
) -> Result<(), AttributionEpochStoreError> {
    if snapshot.wal.is_some() || snapshot.shm.is_some() {
        return Err(activation_unavailable(
            "attribution_epoch_preview_live_wal",
            true,
            "BR-255 read-only preview refuses existing WAL/SHM state because it cannot both consume live WAL facts and guarantee sidecar immutability",
        ));
    }
    Ok(())
}

fn sqlite_header_uses_wal(database_path: &Path) -> Result<bool, AttributionEpochStoreError> {
    let mut header = [0_u8; 20];
    File::open(database_path)
        .and_then(|mut file| file.read_exact(&mut header))
        .map_err(|error| {
            activation_unavailable(
                "attribution_epoch_preview_storage_unavailable",
                true,
                format!("BR-255 cannot inspect SQLite preview header: {error}"),
            )
        })?;
    Ok(header[18] == 2 || header[19] == 2)
}

fn read_only_sqlite_uri(
    database_path: &Path,
    clean_wal_header: bool,
) -> Result<String, AttributionEpochStoreError> {
    let raw = database_path.to_str().ok_or_else(|| {
        activation_unavailable(
            "attribution_epoch_preview_storage_unavailable",
            false,
            "BR-255 read-only preview path is not valid UTF-8",
        )
    })?;
    let mut encoded = String::with_capacity(raw.len());
    for byte in raw.as_bytes() {
        match *byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b':' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(char::from(*byte))
            }
            other => encoded.push_str(&format!("%{other:02X}")),
        }
    }
    if clean_wal_header {
        // `immutable=1` is safe only after the caller proved WAL/SHM absent.
        // The after-snapshot prevents returning a stale preview if a writer
        // creates either sidecar while this immutable snapshot is open.
        Ok(format!("file:{encoded}?mode=ro&immutable=1"))
    } else {
        Ok(format!("file:{encoded}?mode=ro"))
    }
}

#[derive(Serialize)]
struct EpochIdentityPreimage<'a> {
    completed_session_date: NaiveDate,
    effective_date: NaiveDate,
    paper_trade_high_water: i64,
    order_audit_high_water: i64,
    legacy_filled_manifest_hash: &'a str,
    terminal_binding_manifest_hash: &'a str,
    order_audit_tip_hash: &'a str,
    calendar_authority_hash: &'a str,
    legacy_carry_manifest_hash: &'a str,
    carry_item_count: u64,
    carry_total_quantity: u64,
    position_projection_hash: &'a str,
    previous_epoch_receipt_hash: Option<&'a str>,
    decision_basis: &'static str,
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
    let protected_trigger_count = diesel::sql_query(
        "SELECT COUNT(*) AS count FROM sqlite_master
         WHERE type = 'trigger' AND tbl_name IN (
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
    if protected_trigger_count != 14 {
        return Err(integrity(format!(
            "BR-255 protected trigger registry has {protected_trigger_count} entries instead of 14"
        )));
    }
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
    let utc = parsed.with_timezone(&Utc);
    if utc.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string() != value {
        return Err(integrity(format!(
            "BR-255 {field} does not use canonical millisecond UTC bytes"
        )));
    }
    Ok(utc)
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
    let sequences = diesel::sql_query("SELECT seq FROM sqlite_sequence WHERE name = ?")
        .bind::<Text, _>(table)
        .load::<SequenceRow>(conn)?;
    let exact = match sequences.as_slice() {
        [SequenceRow { seq: Some(0) }] => ids.is_empty(),
        [SequenceRow { seq: Some(seq) }] if *seq > 0 => ids.iter().copied().eq(1..=*seq),
        _ => false,
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
            || ((row.outcome == "success")
                != (row.epoch_id.is_some() && row.success_receipt_hash.is_some()))
            || (row.outcome != "success"
                && (row.epoch_id.is_some() || row.success_receipt_hash.is_some()))
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

fn daily_receipt(
    state: &[PersistedDaily],
    row: &PersistedDaily,
) -> diesel::QueryResult<AttributionEpochDailyReceipt> {
    let revision = state
        .iter()
        .filter(|candidate| {
            candidate.id <= row.id
                && candidate.epoch_id == row.epoch_id
                && candidate.date == row.date
                && candidate.signal_family == row.signal_family
        })
        .count();
    Ok(AttributionEpochDailyReceipt {
        epoch_daily_id: row.id,
        signal_family: row.signal_family.clone(),
        revision: u64::try_from(revision)
            .map_err(|_| integrity("BR-255 epoch daily revision overflow"))?,
        payload_hash: row.payload_hash.clone(),
        record_hash: row.record_hash.clone(),
        created_at: row.created_at.clone(),
        retention_deadline: row.retention_deadline.clone(),
    })
}

#[cfg(test)]
thread_local! {
    static DAILY_BATCH_FAILURE_AFTER: std::cell::Cell<Option<usize>> = const {
        std::cell::Cell::new(None)
    };
}

#[cfg(test)]
struct DailyBatchFailureInjection;

#[cfg(test)]
impl Drop for DailyBatchFailureInjection {
    fn drop(&mut self) {
        DAILY_BATCH_FAILURE_AFTER.set(None);
    }
}

#[cfg(test)]
fn inject_daily_batch_failure_after(writes: usize) -> DailyBatchFailureInjection {
    DAILY_BATCH_FAILURE_AFTER.set(Some(writes));
    DailyBatchFailureInjection
}

#[cfg(test)]
fn maybe_inject_daily_batch_failure(completed_writes: usize) -> diesel::QueryResult<()> {
    if DAILY_BATCH_FAILURE_AFTER.get() == Some(completed_writes) {
        return Err(integrity(
            "TEST_CODE injected epoch daily batch failure after completed family write",
        ));
    }
    Ok(())
}

#[cfg(test)]
thread_local! {
    static ACTIVE_VERIFIED_SOURCE_DRIFT: std::cell::Cell<bool> = const {
        std::cell::Cell::new(false)
    };
}

#[cfg(test)]
struct ActiveVerifiedSourceDriftInjection;

#[cfg(test)]
impl Drop for ActiveVerifiedSourceDriftInjection {
    fn drop(&mut self) {
        ACTIVE_VERIFIED_SOURCE_DRIFT.set(false);
    }
}

#[cfg(test)]
fn inject_active_verified_source_drift() -> ActiveVerifiedSourceDriftInjection {
    ACTIVE_VERIFIED_SOURCE_DRIFT.set(true);
    ActiveVerifiedSourceDriftInjection
}

#[cfg(test)]
fn maybe_inject_active_verified_source_drift(
    conn: &mut SqliteConnection,
) -> Result<(), AttributionEpochStoreError> {
    if ACTIVE_VERIFIED_SOURCE_DRIFT.replace(false) {
        diesel::sql_query(
            "UPDATE paper_trades SET fill_price=fill_price+1.0
             WHERE id=(SELECT MIN(id) FROM paper_trades)",
        )
        .execute(conn)
        .map_err(AttributionEpochStoreError::from)?;
    }
    Ok(())
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
        if existing == 0 {
            for (table, _) in TABLES {
                let registered =
                    diesel::sql_query("SELECT COUNT(*) AS count FROM sqlite_sequence WHERE name=?")
                        .bind::<Text, _>(table)
                        .get_result::<CountRow>(conn)?
                        .count;
                if registered != 0 {
                    return Err(integrity(format!(
                        "BR-255 fresh sequence registry already contains {table}"
                    )));
                }
                diesel::sql_query("INSERT INTO sqlite_sequence(name, seq) VALUES (?, 0)")
                    .bind::<Text, _>(table)
                    .execute(conn)?;
            }
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

fn source_name(source: EpochActivationSource) -> &'static str {
    match source {
        EpochActivationSource::Monitor => "monitor",
        EpochActivationSource::Cli => "cli",
    }
}

fn canonical_invoked_at(invoked_at: DateTime<FixedOffset>) -> String {
    invoked_at
        .with_timezone(&Utc)
        .to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn activation_unavailable(
    reason_code: &'static str,
    retryable: bool,
    detail: impl Into<String>,
) -> AttributionEpochStoreError {
    AttributionEpochStoreError::Unavailable {
        reason_code,
        retryable,
        detail: detail.into(),
    }
}

fn activation_error_context(
    stage: &'static str,
    error: AttributionEpochStoreError,
) -> AttributionEpochStoreError {
    match error {
        AttributionEpochStoreError::Unavailable {
            reason_code,
            retryable,
            detail,
        } => AttributionEpochStoreError::Unavailable {
            reason_code,
            retryable,
            detail: format!("BR-255 stage={stage}: {detail}"),
        },
        AttributionEpochStoreError::FailedIntegrity {
            reason_code,
            detail,
        } => AttributionEpochStoreError::FailedIntegrity {
            reason_code,
            detail: format!("BR-255 stage={stage}: {detail}"),
        },
    }
}

fn resolve_activation_calendar(
    invoked_at: DateTime<FixedOffset>,
) -> Result<(NaiveDate, NaiveDate, String), AttributionEpochStoreError> {
    if invoked_at.offset().local_minus_utc() != 8 * 60 * 60 {
        return Err(activation_unavailable(
            "attribution_epoch_invalid_timezone",
            false,
            "BR-255 activation requires the exact +08:00 offset",
        ));
    }
    let completed = invoked_at.date_naive();
    let trading_day =
        crate::calendar::verified_a_share_trading_day(completed).map_err(|detail| {
            activation_unavailable(
                "attribution_epoch_calendar_coverage_unavailable",
                false,
                format!("BR-255 checked-in calendar cannot verify {completed}: {detail}"),
            )
        })?;
    if !trading_day {
        return Err(activation_unavailable(
            "attribution_epoch_non_trading_day",
            false,
            format!("BR-255 {completed} is not a verified A-share trading day"),
        ));
    }
    let start = NaiveTime::from_hms_opt(15, 35, 0)
        .ok_or_else(|| failed_integrity("BR-255 activation start time is invalid"))?;
    let end = NaiveTime::from_hms_opt(15, 50, 0)
        .ok_or_else(|| failed_integrity("BR-255 activation end time is invalid"))?;
    if invoked_at.time() < start {
        return Err(activation_unavailable(
            "attribution_epoch_window_not_open",
            true,
            "BR-255 activation is not eligible before 15:35:00 +08:00",
        ));
    }
    if invoked_at.time() > end {
        return Err(activation_unavailable(
            "attribution_epoch_window_closed",
            false,
            "BR-255 activation is not eligible after 15:50:00 +08:00",
        ));
    }
    let effective =
        crate::calendar::verified_next_a_share_trading_day(completed).map_err(|detail| {
            activation_unavailable(
                "attribution_epoch_calendar_coverage_unavailable",
                false,
                format!("BR-255 next verified trading day is unavailable: {detail}"),
            )
        })?;
    let calendar =
        crate::calendar::resolve_verified_replay_range(completed, effective).map_err(|error| {
            activation_unavailable(
                "attribution_epoch_calendar_unavailable",
                error.retryable(),
                format!("BR-255 calendar authority is unavailable: {error}"),
            )
        })?;
    Ok((completed, effective, calendar.authority_hash().to_owned()))
}

fn load_paper_high_water(conn: &mut SqliteConnection) -> diesel::QueryResult<i64> {
    diesel::sql_query("SELECT COALESCE(MAX(id), 0) AS id FROM paper_trades")
        .get_result::<IdRow>(conn)
        .map(|row| row.id)
}

fn load_frozen_paper_rows(
    conn: &mut SqliteConnection,
    high_water: i64,
) -> diesel::QueryResult<Vec<FrozenPaperFill>> {
    diesel::sql_query(
        "SELECT id,plan_id,code,name,direction,price AS requested_price,status,fill_price,
                not_fill_reason,quantity,CAST(ts AS TEXT) AS occurred_at,virtual_reason,
                account_mode,data_mode,CAST(updated_at AS TEXT) AS updated_at
         FROM paper_trades
         WHERE id <= ?
         ORDER BY id ASC",
    )
    .bind::<BigInt, _>(high_water)
    .load(conn)
}

fn validate_paper_identity(code: &str) -> Result<(), AttributionEpochStoreError> {
    crate::risk::env_guard::validate_symbol_for_current_env(code).map_err(|detail| {
        failed_integrity(format!(
            "BR-255 paper identity environment mismatch: {detail}"
        ))
    })?;
    if let Some(canonical) = code.strip_prefix("TEST_CODE_") {
        if canonical.len() != 6 || !canonical.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(failed_integrity(format!(
                "BR-255 TEST_CODE paper identity is invalid: {code:?}"
            )));
        }
        return Ok(());
    }
    let identity = crate::data_gateway::instrument_identity::resolve_production_equity(code, None)
        .map_err(|error| {
            failed_integrity(format!("BR-255 paper equity identity is invalid: {error}"))
        })?;
    identity.require_a_share().map_err(|error| {
        failed_integrity(format!("BR-255 paper equity identity is invalid: {error}"))
    })?;
    Ok(())
}

fn validate_frozen_paper_row(
    row: &FrozenPaperFill,
) -> Result<chrono::NaiveDateTime, AttributionEpochStoreError> {
    if row.id <= 0 {
        return Err(failed_integrity(format!(
            "BR-255 paper source contains non-positive id={}",
            row.id
        )));
    }
    if row.plan_id.trim().is_empty() || row.name.trim().is_empty() {
        return Err(failed_integrity(format!(
            "BR-255 paper id={} has an empty plan or name identity",
            row.id
        )));
    }
    validate_paper_identity(&row.code)?;
    if !matches!(row.direction.as_str(), "buy" | "sell") {
        return Err(failed_integrity(format!(
            "BR-255 paper id={} direction is invalid: {:?}",
            row.id, row.direction
        )));
    }
    if !row.requested_price.is_finite() || row.requested_price <= 0.0 {
        return Err(failed_integrity(format!(
            "BR-255 paper id={} requested price is invalid",
            row.id
        )));
    }
    u32::try_from(row.quantity)
        .ok()
        .filter(|quantity| *quantity > 0 && quantity.is_multiple_of(100))
        .ok_or_else(|| {
            failed_integrity(format!(
                "BR-255 paper id={} quantity is invalid: {}",
                row.id, row.quantity
            ))
        })?;
    let normalized_reason = row
        .not_fill_reason
        .as_deref()
        .map(str::trim)
        .filter(|reason| !reason.is_empty());
    match row.status.as_str() {
        "Filled"
            if row
                .fill_price
                .is_none_or(|price| !price.is_finite() || price <= 0.0)
                || normalized_reason.is_some() =>
        {
            return Err(failed_integrity(format!(
                "BR-255 Filled paper id={} has incomplete terminal facts",
                row.id
            )));
        }
        "NotFilled" | "Invalidated" if row.fill_price.is_some() || normalized_reason.is_none() => {
            return Err(failed_integrity(format!(
                "BR-255 {} paper id={} has incomplete terminal facts",
                row.status, row.id
            )));
        }
        "Filled" | "NotFilled" | "Invalidated" => {}
        other => {
            return Err(failed_integrity(format!(
                "BR-255 paper id={} status is invalid: {other:?}",
                row.id
            )));
        }
    }
    if row.virtual_reason.trim().is_empty()
        || !matches!(
            row.account_mode.as_str(),
            "Normal" | "ReduceOnly" | "Frozen"
        )
        || !matches!(row.data_mode.as_str(), "Full" | "Degraded" | "Unsafe")
    {
        return Err(failed_integrity(format!(
            "BR-255 paper id={} decision/risk context is incomplete",
            row.id
        )));
    }
    let occurred_at =
        parse_paper_fill_timestamp(row.id, &row.occurred_at).map_err(failed_integrity)?;
    let updated_at = parse_paper_fill_timestamp(row.id, &row.updated_at).map_err(|detail| {
        failed_integrity(format!("BR-255 paper id={} updated_at: {detail}", row.id))
    })?;
    if updated_at < occurred_at {
        return Err(failed_integrity(format!(
            "BR-255 paper id={} updated_at precedes its persisted timestamp",
            row.id
        )));
    }
    Ok(occurred_at)
}

fn load_order_audit_rows(
    conn: &mut SqliteConnection,
) -> diesel::QueryResult<Vec<CanonicalOrderAuditRow>> {
    diesel::sql_query(
        "SELECT id,business_order_id,source,decision_basis,side,code,requested_price,
                execution_price,quantity,quote_observed_at,outcome,failure_reason,
                CAST(created_at AS TEXT) AS created_at
         FROM order_audit ORDER BY id ASC",
    )
    .load(conn)
}

fn load_order_audit_chain_rows(
    conn: &mut SqliteConnection,
) -> diesel::QueryResult<Vec<CanonicalOrderAuditChainRow>> {
    diesel::sql_query(
        "SELECT order_audit_id,previous_hash,record_hash
         FROM order_audit_chain ORDER BY order_audit_id ASC",
    )
    .load(conn)
}

fn analyze_source_projection(
    conn: &mut SqliteConnection,
    completed_session: NaiveDate,
    frozen_limits: Option<(i64, i64)>,
    reject_after_completed: bool,
) -> Result<FrozenSourceProjection, AttributionEpochStoreError> {
    let invalid_paper_ids =
        diesel::sql_query("SELECT COUNT(*) AS count FROM paper_trades WHERE id <= 0")
            .get_result::<CountRow>(conn)?
            .count;
    if invalid_paper_ids != 0 {
        return Err(failed_integrity(
            "BR-255 paper source contains a non-positive identity",
        ));
    }
    let current_paper_high_water = load_paper_high_water(conn)?;
    let paper_trade_high_water = frozen_limits
        .map(|limits| limits.0)
        .unwrap_or(current_paper_high_water);
    if current_paper_high_water < paper_trade_high_water {
        return Err(failed_integrity(format!(
            "BR-255 paper high-water regressed current={current_paper_high_water} frozen={paper_trade_high_water}"
        )));
    }
    let paper_rows = load_frozen_paper_rows(conn, paper_trade_high_water)?;
    let mut paper_ids = HashSet::with_capacity(paper_rows.len());
    let mut paper_plans = HashSet::with_capacity(paper_rows.len());
    for (index, row) in paper_rows.iter().enumerate() {
        if index > 0 && paper_rows[index - 1].id >= row.id {
            return Err(failed_integrity(
                "BR-255 paper source identities are not strictly increasing",
            ));
        }
        if !paper_ids.insert(row.id) {
            return Err(failed_integrity(format!(
                "BR-255 paper source contains duplicate id={}",
                row.id
            )));
        }
        if !paper_plans.insert(row.plan_id.as_str()) {
            return Err(failed_integrity(format!(
                "BR-255 paper source contains duplicate plan_id={:?}",
                row.plan_id
            )));
        }
        let occurred_at = validate_frozen_paper_row(row)?;
        if reject_after_completed && occurred_at.date() > completed_session {
            return Err(failed_integrity(format!(
                "BR-255 paper id={} is dated after completed session {completed_session}",
                row.id
            )));
        }
    }
    let all_status_paper_manifest_hash = hash_json(
        b"BR255_ATTRIBUTION_ALL_STATUS_PAPER_MANIFEST_V1\0",
        &paper_rows,
    )?;
    let mut fills = paper_rows
        .into_iter()
        .filter(|row| row.status == "Filled")
        .collect::<Vec<_>>();
    fills.sort_by(|left, right| {
        left.occurred_at
            .cmp(&right.occurred_at)
            .then(left.id.cmp(&right.id))
    });
    let economic_fills = fills
        .iter()
        .map(FrozenPaperFill::economic)
        .collect::<Vec<_>>();
    let carry = build_legacy_carry(&economic_fills, completed_session)
        .map_err(|detail| failed_integrity(format!("BR-255 legacy carry: {detail}")))?;
    let legacy_filled_manifest_hash =
        hash_json(b"BR255_ATTRIBUTION_LEGACY_FILLED_MANIFEST_V1\0", &fills)?;
    let position_projection_hash =
        hash_json(b"BR255_ATTRIBUTION_POSITION_PROJECTION_V1\0", &carry)?;

    let audits = load_order_audit_rows(conn)?;
    let chain = load_order_audit_chain_rows(conn)?;
    if audits
        .iter()
        .enumerate()
        .any(|(index, row)| row.id <= 0 || index > 0 && audits[index - 1].id >= row.id)
    {
        return Err(failed_integrity(
            "BR-255 order audit identities are not positive and strictly increasing",
        ));
    }
    validate_canonical_order_audit_chain(&audits, &chain)
        .map_err(|detail| failed_integrity(format!("BR-255 order audit chain: {detail}")))?;
    let current_order_audit_high_water = audits.last().map_or(0, |row| row.id);
    let order_audit_high_water = frozen_limits
        .map(|limits| limits.1)
        .unwrap_or(current_order_audit_high_water);
    if current_order_audit_high_water < order_audit_high_water {
        return Err(failed_integrity(format!(
            "BR-255 order-audit high-water regressed current={current_order_audit_high_water} frozen={order_audit_high_water}"
        )));
    }
    let prefix_audits = audits
        .iter()
        .filter(|row| row.id <= order_audit_high_water)
        .cloned()
        .collect::<Vec<_>>();
    let prefix_chain = chain
        .iter()
        .filter(|row| row.order_audit_id <= order_audit_high_water)
        .cloned()
        .collect::<Vec<_>>();
    let order_audit_tip_hash = validate_canonical_order_audit_chain(&prefix_audits, &prefix_chain)
        .map_err(|detail| failed_integrity(format!("BR-255 frozen audit prefix: {detail}")))?;
    if order_audit_high_water == 0 && order_audit_tip_hash != AUDIT_CHAIN_GENESIS {
        return Err(failed_integrity("BR-255 empty audit prefix has a bad tip"));
    }

    let chain_hashes = prefix_chain
        .iter()
        .map(|row| (row.order_audit_id, row.record_hash.as_str()))
        .collect::<HashMap<_, _>>();
    let paper_plans = fills
        .iter()
        .map(|row| row.plan_id.as_str())
        .collect::<HashSet<_>>();
    let mut terminals = HashMap::<&str, Vec<&CanonicalOrderAuditRow>>::new();
    for audit in prefix_audits
        .iter()
        .filter(|row| row.source == "PaperTrade" && row.outcome == "Filled")
    {
        if !paper_plans.contains(audit.business_order_id.as_str()) {
            return Err(failed_integrity(format!(
                "BR-255 Filled PaperTrade audit id={} has no frozen paper plan {}",
                audit.id, audit.business_order_id
            )));
        }
        terminals
            .entry(audit.business_order_id.as_str())
            .or_default()
            .push(audit);
    }
    let shanghai = FixedOffset::east_opt(8 * 60 * 60)
        .ok_or_else(|| failed_integrity("BR-255 Shanghai fixed offset is unavailable"))?;
    let mut bindings = Vec::with_capacity(fills.len());
    for paper in &fills {
        let candidates = terminals
            .get(paper.plan_id.as_str())
            .into_iter()
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        if candidates.len() != 1 {
            return Err(failed_integrity(format!(
                "BR-255 Filled paper id={} has {} terminal audit candidates",
                paper.id,
                candidates.len()
            )));
        }
        let terminal = candidates[0];
        let exact = terminal.source == "PaperTrade"
            && terminal.outcome == "Filled"
            && terminal.code == paper.code
            && terminal.decision_basis == paper.virtual_reason
            && terminal.side == paper.direction
            && terminal.requested_price.to_bits() == paper.requested_price.to_bits()
            && terminal.execution_price.map(f64::to_bits) == paper.fill_price.map(f64::to_bits)
            && terminal.quantity == paper.quantity
            && terminal.failure_reason.is_none();
        if !exact {
            return Err(failed_integrity(format!(
                "BR-255 Filled paper id={} and terminal audit id={} do not exactly bind source/code/side/prices/quantity/outcome/decision/failure",
                paper.id, terminal.id
            )));
        }
        let execution_price = terminal.execution_price.ok_or_else(|| {
            failed_integrity(format!(
                "BR-255 Filled audit id={} has no execution price",
                terminal.id
            ))
        })?;
        if !terminal.requested_price.is_finite()
            || terminal.requested_price <= 0.0
            || !execution_price.is_finite()
            || execution_price <= 0.0
        {
            return Err(failed_integrity(format!(
                "BR-255 paper id={} and terminal audit id={} do not exactly bind",
                paper.id, terminal.id
            )));
        }
        let quote_observed_at = terminal
            .quote_observed_at
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                failed_integrity(format!(
                    "BR-255 terminal audit id={} has no quote time",
                    terminal.id
                ))
            })
            .and_then(|value| {
                DateTime::parse_from_rfc3339(value).map_err(|error| {
                    failed_integrity(format!(
                        "BR-255 terminal audit id={} quote time is invalid: {error}",
                        terminal.id
                    ))
                })
            })?;
        let terminal_at = parse_paper_fill_timestamp(terminal.id, &terminal.created_at)
            .map_err(|detail| {
                failed_integrity(format!(
                    "BR-255 terminal audit id={} created_at is invalid: {detail}",
                    terminal.id
                ))
            })?
            .and_utc();
        let quote_observed_at_utc = quote_observed_at.with_timezone(&Utc);
        if quote_observed_at_utc > terminal_at {
            return Err(failed_integrity(format!(
                "BR-255 terminal audit id={} quote time is in the future",
                terminal.id
            )));
        }
        let quote_age = terminal_at
            .signed_duration_since(quote_observed_at_utc)
            .num_milliseconds();
        if quote_age > 5_000 {
            return Err(failed_integrity(format!(
                "BR-255 terminal audit id={} quote is stale by {quote_age}ms",
                terminal.id
            )));
        }
        let paper_created_at = parse_paper_fill_timestamp(paper.id, &paper.occurred_at)
            .map_err(failed_integrity)?
            .and_utc();
        let paper_business_date = paper_created_at.date_naive();
        let quote_business_date = quote_observed_at.with_timezone(&shanghai).date_naive();
        if paper_business_date != quote_business_date {
            return Err(failed_integrity(format!(
                "BR-255 paper id={} business date {paper_business_date} differs from terminal audit id={} quote business date {quote_business_date}",
                paper.id, terminal.id
            )));
        }
        if paper_created_at + chrono::Duration::seconds(1) < quote_observed_at_utc {
            return Err(failed_integrity(format!(
                "BR-255 paper id={} persistence time precedes terminal quote evidence",
                paper.id
            )));
        }
        let terminal_audit_hash = chain_hashes.get(&terminal.id).ok_or_else(|| {
            failed_integrity(format!(
                "BR-255 terminal audit id={} has no canonical chain hash",
                terminal.id
            ))
        })?;
        bindings.push(TerminalBindingManifestItem {
            paper_trade_id: paper.id,
            terminal_audit_id: terminal.id,
            terminal_audit_hash: (*terminal_audit_hash).to_owned(),
            terminal_time: terminal_at.to_rfc3339_opts(SecondsFormat::Millis, true),
        });
    }
    let terminal_binding_manifest_hash = hash_json(
        b"BR255_ATTRIBUTION_TERMINAL_BINDING_MANIFEST_V1\0",
        &bindings,
    )?;
    Ok(FrozenSourceProjection {
        paper_trade_high_water,
        order_audit_high_water,
        all_status_paper_manifest_hash,
        fills,
        bindings,
        carry,
        legacy_filled_manifest_hash,
        terminal_binding_manifest_hash,
        order_audit_tip_hash,
        position_projection_hash,
    })
}

fn carry_summary(carry: &[LegacyCarryPosition]) -> Result<(u64, u64), AttributionEpochStoreError> {
    let count = u64::try_from(carry.len())
        .map_err(|_| failed_integrity("BR-255 carry item count overflow"))?;
    let total = carry.iter().try_fold(0_u64, |total, item| {
        total
            .checked_add(item.quantity)
            .ok_or_else(|| failed_integrity("BR-255 carry total quantity overflow"))
    })?;
    Ok((count, total))
}

fn epoch_identity(
    completed: NaiveDate,
    effective: NaiveDate,
    source: &FrozenSourceProjection,
    calendar_authority_hash: &str,
) -> Result<String, AttributionEpochStoreError> {
    let legacy_carry_manifest_hash = canonical_legacy_carry_manifest_hash(&source.carry);
    let (carry_item_count, carry_total_quantity) = carry_summary(&source.carry)?;
    hash_json(
        b"BR255_ATTRIBUTION_EPOCH_ID_V1\0",
        &EpochIdentityPreimage {
            completed_session_date: completed,
            effective_date: effective,
            paper_trade_high_water: source.paper_trade_high_water,
            order_audit_high_water: source.order_audit_high_water,
            legacy_filled_manifest_hash: &source.legacy_filled_manifest_hash,
            terminal_binding_manifest_hash: &source.terminal_binding_manifest_hash,
            order_audit_tip_hash: &source.order_audit_tip_hash,
            calendar_authority_hash,
            legacy_carry_manifest_hash: &legacy_carry_manifest_hash,
            carry_item_count,
            carry_total_quantity,
            position_projection_hash: &source.position_projection_hash,
            previous_epoch_receipt_hash: None,
            decision_basis: "BR-255",
        },
    )
    .map_err(AttributionEpochStoreError::from)
}

fn activation_preview(
    activated: bool,
    completed: NaiveDate,
    effective: NaiveDate,
    source: FrozenSourceProjection,
    calendar_authority_hash: &str,
    receipt_hash: Option<String>,
) -> Result<EpochActivationPreview, AttributionEpochStoreError> {
    let epoch_id = epoch_identity(completed, effective, &source, calendar_authority_hash)?;
    Ok(EpochActivationPreview {
        activated,
        epoch_id,
        completed_session_date: completed,
        effective_date: effective,
        paper_trade_high_water: source.paper_trade_high_water,
        order_audit_high_water: source.order_audit_high_water,
        carry: source.carry,
        legacy_filled_manifest_hash: source.legacy_filled_manifest_hash,
        terminal_binding_manifest_hash: source.terminal_binding_manifest_hash,
        order_audit_tip_hash: source.order_audit_tip_hash,
        position_projection_hash: source.position_projection_hash,
        calendar_authority_hash: calendar_authority_hash.to_owned(),
        receipt_hash,
    })
}

fn verified_activation_outcome(
    receipt: AttributionEpochReceipt,
    source: FrozenSourceProjection,
) -> Result<EpochActivationOutcome, AttributionEpochStoreError> {
    let projection = activation_preview(
        true,
        receipt.cutover_completed_trading_date,
        receipt.effective_trading_date,
        source,
        &receipt.calendar_authority_hash,
        Some(receipt.receipt_hash.clone()),
    )?;
    let (carry_item_count, carry_total_quantity) = carry_summary(&projection.carry)?;
    if projection.epoch_id != receipt.epoch_id
        || projection.paper_trade_high_water != receipt.paper_trade_high_water
        || projection.order_audit_high_water != receipt.order_audit_high_water
        || projection.legacy_filled_manifest_hash != receipt.legacy_filled_manifest_hash
        || projection.terminal_binding_manifest_hash != receipt.terminal_binding_manifest_hash
        || projection.order_audit_tip_hash != receipt.order_audit_tip_hash
        || projection.position_projection_hash != receipt.position_projection_hash
        || projection.calendar_authority_hash != receipt.calendar_authority_hash
        || projection.receipt_hash.as_deref() != Some(receipt.receipt_hash.as_str())
        || canonical_legacy_carry_manifest_hash(&projection.carry)
            != receipt.legacy_carry_manifest_hash
        || carry_item_count != receipt.carry_item_count
        || carry_total_quantity != receipt.carry_total_quantity
    {
        return Err(failed_integrity(
            "BR-255 activation receipt and frozen render projection differ",
        ));
    }
    Ok(EpochActivationOutcome {
        receipt,
        projection,
    })
}

fn insert_attempt_on_conn(
    conn: &mut SqliteConnection,
    input: &AttributionEpochAttemptAppend,
) -> Result<AttributionEpochAttemptReceipt, AttributionEpochStoreError> {
    let attempts = validate_attempts(conn)?;
    let previous = attempts
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
        created_at: window.created_at,
        retention_deadline: window.retention_deadline,
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
    row.id = diesel::select(diesel::dsl::sql::<BigInt>("last_insert_rowid()")).get_result(conn)?;
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
    Ok(AttributionEpochAttemptReceipt {
        attempt_audit_id: row.id,
        record_hash: row.record_hash,
        created_at: row.created_at,
        retention_deadline: row.retention_deadline,
    })
}

fn insert_activation_receipt(
    conn: &mut SqliteConnection,
    preview: &EpochActivationPreview,
    calendar_authority_hash: &str,
) -> Result<AttributionEpochReceipt, AttributionEpochStoreError> {
    let window = new_window(conn)?;
    let legacy_carry_manifest_hash = canonical_legacy_carry_manifest_hash(&preview.carry);
    let (carry_item_count, carry_total_quantity) = carry_summary(&preview.carry)?;
    let mut row = PersistedReceipt {
        id: 0,
        epoch_id: preview.epoch_id.clone(),
        cutover_completed_trading_date: preview.completed_session_date.to_string(),
        effective_trading_date: preview.effective_date.to_string(),
        paper_trade_high_water: preview.paper_trade_high_water,
        legacy_filled_manifest_hash: preview.legacy_filled_manifest_hash.clone(),
        terminal_binding_manifest_hash: preview.terminal_binding_manifest_hash.clone(),
        order_audit_high_water: preview.order_audit_high_water,
        order_audit_tip_hash: preview.order_audit_tip_hash.clone(),
        calendar_authority_hash: calendar_authority_hash.to_owned(),
        legacy_carry_manifest_hash,
        carry_item_count: i64::try_from(carry_item_count)
            .map_err(|_| failed_integrity("BR-255 carry item count exceeds SQLite INTEGER"))?,
        carry_total_quantity: i64::try_from(carry_total_quantity)
            .map_err(|_| failed_integrity("BR-255 carry quantity exceeds SQLite INTEGER"))?,
        position_projection_hash: preview.position_projection_hash.clone(),
        previous_epoch_receipt_hash: None,
        decision_basis: "BR-255".to_owned(),
        receipt_hash: String::new(),
        created_at: window.created_at,
        retention_deadline: window.retention_deadline,
    };
    row.receipt_hash = receipt_hash(&row)?;
    diesel::sql_query(
        "INSERT INTO attribution_sample_epoch_receipt
         (epoch_id,cutover_completed_trading_date,effective_trading_date,
          paper_trade_high_water,legacy_filled_manifest_hash,terminal_binding_manifest_hash,
          order_audit_high_water,order_audit_tip_hash,calendar_authority_hash,
          legacy_carry_manifest_hash,carry_item_count,carry_total_quantity,
          position_projection_hash,previous_epoch_receipt_hash,decision_basis,receipt_hash,
          created_at,retention_deadline)
         VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
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
    .execute(conn)?;
    row.id = diesel::select(diesel::dsl::sql::<BigInt>("last_insert_rowid()")).get_result(conn)?;
    diesel::sql_query(
        "INSERT INTO attribution_sample_epoch_receipt_chain
         (epoch_receipt_id,previous_hash,record_hash,created_at,retention_deadline)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind::<BigInt, _>(row.id)
    .bind::<Text, _>(RECEIPT_GENESIS)
    .bind::<Text, _>(&row.receipt_hash)
    .bind::<Text, _>(&row.created_at)
    .bind::<Text, _>(&row.retention_deadline)
    .execute(conn)?;

    let mut previous = CARRY_GENESIS.to_owned();
    for (index, position) in preview.carry.iter().enumerate() {
        let mut item = PersistedCarry {
            id: 0,
            epoch_receipt_id: row.id,
            code: position.code.clone(),
            quantity: i64::try_from(position.quantity).map_err(|_| {
                failed_integrity("BR-255 carry item quantity exceeds SQLite INTEGER")
            })?,
            item_index: i64::try_from(index)
                .map_err(|_| failed_integrity("BR-255 carry item index overflow"))?,
            predecessor_item_hash: previous,
            item_hash: String::new(),
            created_at: row.created_at.clone(),
            retention_deadline: row.retention_deadline.clone(),
        };
        item.item_hash = carry_hash(&item)?;
        diesel::sql_query(
            "INSERT INTO attribution_legacy_carry_item
             (epoch_receipt_id,code,quantity,item_index,predecessor_item_hash,item_hash,
              created_at,retention_deadline) VALUES (?,?,?,?,?,?,?,?)",
        )
        .bind::<BigInt, _>(item.epoch_receipt_id)
        .bind::<Text, _>(&item.code)
        .bind::<BigInt, _>(item.quantity)
        .bind::<BigInt, _>(item.item_index)
        .bind::<Text, _>(&item.predecessor_item_hash)
        .bind::<Text, _>(&item.item_hash)
        .bind::<Text, _>(&item.created_at)
        .bind::<Text, _>(&item.retention_deadline)
        .execute(conn)?;
        previous = item.item_hash;
    }
    receipt_value(&row).map_err(AttributionEpochStoreError::from)
}

fn ensure_source_matches_receipt(
    conn: &mut SqliteConnection,
    epoch: &AttributionEpochReceipt,
) -> Result<FrozenSourceProjection, AttributionEpochStoreError> {
    let verified_day =
        crate::calendar::verified_a_share_trading_day(epoch.cutover_completed_trading_date)
            .map_err(|detail| {
                failed_integrity(format!(
                    "BR-255 stored completed date lost calendar authority: {detail}"
                ))
            })?;
    let expected_effective =
        crate::calendar::verified_next_a_share_trading_day(epoch.cutover_completed_trading_date)
            .map_err(|detail| {
                failed_integrity(format!(
                    "BR-255 stored effective date lost calendar authority: {detail}"
                ))
            })?;
    let calendar = crate::calendar::resolve_verified_replay_range(
        epoch.cutover_completed_trading_date,
        epoch.effective_trading_date,
    )
    .map_err(|error| failed_integrity(format!("BR-255 stored calendar is invalid: {error}")))?;
    if !verified_day
        || expected_effective != epoch.effective_trading_date
        || calendar.authority_hash() != epoch.calendar_authority_hash
    {
        return Err(failed_integrity(
            "BR-255 stored completed/effective/calendar authority binding changed",
        ));
    }
    let source = analyze_source_projection(
        conn,
        epoch.cutover_completed_trading_date,
        Some((epoch.paper_trade_high_water, epoch.order_audit_high_water)),
        true,
    )?;
    let legacy_carry_manifest_hash = canonical_legacy_carry_manifest_hash(&source.carry);
    let (carry_item_count, carry_total_quantity) = carry_summary(&source.carry)?;
    let expected_epoch_id = epoch_identity(
        epoch.cutover_completed_trading_date,
        epoch.effective_trading_date,
        &source,
        &epoch.calendar_authority_hash,
    )?;
    if source.legacy_filled_manifest_hash != epoch.legacy_filled_manifest_hash
        || source.terminal_binding_manifest_hash != epoch.terminal_binding_manifest_hash
        || source.order_audit_tip_hash != epoch.order_audit_tip_hash
        || source.position_projection_hash != epoch.position_projection_hash
        || legacy_carry_manifest_hash != epoch.legacy_carry_manifest_hash
        || carry_item_count != epoch.carry_item_count
        || carry_total_quantity != epoch.carry_total_quantity
        || expected_epoch_id != epoch.epoch_id
    {
        return Err(failed_integrity(
            "BR-255 frozen source prefix no longer matches the epoch receipt",
        ));
    }
    Ok(source)
}

pub(crate) fn verify_epoch_source_prefix(
    conn: &mut SqliteConnection,
    epoch: &AttributionEpochReceipt,
) -> Result<(), AttributionEpochStoreError> {
    verified_epoch_retained_carry(conn, epoch).map(|_| ())
}

fn verified_epoch_retained_carry(
    conn: &mut SqliteConnection,
    epoch: &AttributionEpochReceipt,
) -> Result<Vec<LegacyCarryPosition>, AttributionEpochStoreError> {
    let rows = validate_all(conn)?;
    let persisted = rows
        .iter()
        .find(|row| row.epoch_id == epoch.epoch_id)
        .ok_or_else(|| failed_integrity("BR-255 epoch receipt is absent during prefix verify"))?;
    let persisted_value = receipt_value(persisted)?;
    if &persisted_value != epoch {
        return Err(failed_integrity(
            "BR-255 caller receipt differs from canonical retained receipt",
        ));
    }
    ensure_source_matches_receipt(conn, epoch)?;
    let carry = load_carry(conn)?
        .into_iter()
        .filter(|row| row.epoch_receipt_id == persisted.id)
        .map(|row| {
            Ok(LegacyCarryPosition {
                code: row.code,
                quantity: u64::try_from(row.quantity)
                    .map_err(|_| failed_integrity("BR-255 retained carry quantity is invalid"))?,
            })
        })
        .collect::<Result<Vec<_>, AttributionEpochStoreError>>()?;
    let (count, total) = carry_summary(&carry)?;
    if count != epoch.carry_item_count
        || total != epoch.carry_total_quantity
        || canonical_legacy_carry_manifest_hash(&carry) != epoch.legacy_carry_manifest_hash
    {
        return Err(failed_integrity(
            "BR-255 retained carry changed after complete epoch validation",
        ));
    }
    Ok(carry)
}

#[allow(dead_code)] // Task 5 wires this deep verified source capability into replay.
pub(crate) fn load_verified_epoch_fills_until(
    conn: &mut SqliteConnection,
    epoch: &ResolvedAttributionEpoch,
    to: NaiveDate,
) -> Result<VerifiedEpochFillSet, AttributionEpochStoreError> {
    let (completed, paper_high_water, audit_high_water, effective, carry) = match epoch {
        ResolvedAttributionEpoch::Legacy => (to, 0, 0, None, Vec::new()),
        ResolvedAttributionEpoch::Epoch(receipt) => {
            let carry = verified_epoch_retained_carry(conn, receipt)?;
            (
                receipt.cutover_completed_trading_date,
                receipt.paper_trade_high_water,
                receipt.order_audit_high_water,
                Some(receipt.effective_trading_date),
                carry,
            )
        }
    };
    let source = analyze_source_projection(conn, completed, None, false)?;
    let bindings = source
        .bindings
        .iter()
        .map(|binding| (binding.paper_trade_id, binding))
        .collect::<HashMap<_, _>>();
    let mut fills = Vec::new();
    for paper in source.fills {
        let occurred_at =
            parse_paper_fill_timestamp(paper.id, &paper.occurred_at).map_err(failed_integrity)?;
        if let Some(effective_date) = effective {
            if paper.id <= paper_high_water {
                continue;
            }
            if occurred_at.date() < effective_date {
                return Err(failed_integrity(format!(
                    "BR-255 post-high-water fill id={} is dated before effective date {effective_date}",
                    paper.id
                )));
            }
        }
        if occurred_at.date() > to {
            continue;
        }
        let binding = bindings.get(&paper.id).ok_or_else(|| {
            failed_integrity(format!("BR-255 fill id={} lost terminal binding", paper.id))
        })?;
        if effective.is_some() && binding.terminal_audit_id <= audit_high_water {
            return Err(failed_integrity(format!(
                "BR-255 post-epoch fill id={} binds audit id={} at/below frozen high-water",
                paper.id, binding.terminal_audit_id
            )));
        }
        let terminal_time =
            DateTime::parse_from_rfc3339(&binding.terminal_time).map_err(|error| {
                failed_integrity(format!(
                    "BR-255 canonical terminal time disappeared for fill id={}: {error}",
                    paper.id
                ))
            })?;
        fills.push(VerifiedEpochFill {
            fill: paper.economic(),
            terminal_audit_id: binding.terminal_audit_id,
            terminal_audit_hash: binding.terminal_audit_hash.clone(),
            terminal_time,
        });
    }
    Ok(VerifiedEpochFillSet {
        fills,
        carry,
        current_paper_trade_high_water: source.paper_trade_high_water,
        current_order_audit_high_water: source.order_audit_high_water,
        all_status_paper_manifest_hash: source.all_status_paper_manifest_hash,
        filled_manifest_hash: source.legacy_filled_manifest_hash,
        terminal_binding_manifest_hash: source.terminal_binding_manifest_hash,
        order_audit_tip_hash: source.order_audit_tip_hash,
    })
}

fn validate_daily_source_binding(
    conn: &mut SqliteConnection,
    authority: &DatabaseConnectionAuthority,
    binding: &AttributionEpochDailySourceBinding,
) -> Result<(), AttributionEpochStoreError> {
    if authority != &binding.database_authority {
        return Err(failed_integrity(
            "BR-255 epoch daily binding belongs to a different database authority",
        ));
    }
    let active = match load_selector_with_connection(conn, &AttributionEpochSelector::Active)? {
        ResolvedAttributionEpoch::Epoch(receipt) => receipt,
        ResolvedAttributionEpoch::Legacy => unreachable!("active selector cannot resolve Legacy"),
    };
    if binding.cutoff_date < binding.effective_date
        || active.epoch_id != binding.epoch_id
        || active.receipt_hash != binding.receipt_hash
        || active.effective_trading_date != binding.effective_date
        || active.paper_trade_high_water != binding.frozen_paper_trade_high_water
        || active.order_audit_high_water != binding.frozen_order_audit_high_water
        || active.legacy_carry_manifest_hash != binding.legacy_carry_manifest_hash
    {
        return Err(failed_integrity(
            "BR-255 epoch daily binding differs from the active retained receipt",
        ));
    }
    let resolved = ResolvedAttributionEpoch::Epoch(active.clone());
    let verified = load_verified_epoch_fills_until(conn, &resolved, binding.cutoff_date)?;
    if verified.current_paper_trade_high_water() != binding.source_paper_trade_high_water
        || verified.current_order_audit_high_water() != binding.source_order_audit_high_water
        || verified.all_status_paper_manifest_hash() != binding.all_status_paper_manifest_hash
        || verified.filled_manifest_hash() != binding.verified_filled_manifest_hash
        || verified.terminal_binding_manifest_hash()
            != binding.verified_terminal_binding_manifest_hash
        || verified.order_audit_tip_hash() != binding.verified_order_audit_tip_hash
    {
        return Err(failed_integrity(
            "BR-255 epoch daily verified source changed after computation",
        ));
    }
    let source_rows = verified
        .fills()
        .iter()
        .map(|fill| fill.fill().clone())
        .collect::<Vec<_>>();
    let scoped = scope_epoch_fills(
        &source_rows,
        active.effective_trading_date,
        verified.carry(),
    )
    .map_err(|detail| {
        failed_integrity(format!(
            "BR-255 epoch daily source rescoping failed: {detail}"
        ))
    })?;
    let exclusion_manifest_hash =
        canonical_exclusion_manifest_hash(&scoped.exclusions, &source_rows).map_err(|detail| {
            failed_integrity(format!(
                "BR-255 epoch daily exclusion revalidation failed: {detail}"
            ))
        })?;
    let scoped_fill_manifest_hash = canonical_scoped_fill_manifest_hash(&scoped.attributable)
        .map_err(|detail| {
            failed_integrity(format!(
                "BR-255 epoch daily scoped fill revalidation failed: {detail}"
            ))
        })?;
    if exclusion_manifest_hash != binding.exclusion_manifest_hash
        || scoped_fill_manifest_hash != binding.scoped_fill_manifest_hash
        || canonical_legacy_carry_manifest_hash(&scoped.remaining_quarantine)
            != binding.remaining_quarantine_manifest_hash
        || scoped.released_codes != binding.released_codes
    {
        return Err(failed_integrity(
            "BR-255 epoch daily scoped evidence changed after computation",
        ));
    }
    Ok(())
}

fn map_database_authority_error(error: DatabaseAuthorityError) -> AttributionEpochStoreError {
    match error {
        DatabaseAuthorityError::DescriptorAttestationUnavailable { detail } => {
            AttributionEpochStoreError::Unavailable {
                reason_code: "attribution_database_authority_unavailable",
                retryable: false,
                detail: format!("BR-255 attribution checkout authority unavailable: {detail}"),
            }
        }
        DatabaseAuthorityError::DescriptorIntegrityFailed { detail } => failed_integrity(format!(
            "BR-255 attribution checkout descriptor integrity: {detail}"
        )),
    }
}

fn load_selector_with_connection(
    conn: &mut SqliteConnection,
    selector: &AttributionEpochSelector,
) -> Result<ResolvedAttributionEpoch, AttributionEpochStoreError> {
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
    .get_result::<CountRow>(conn)
    .map_err(map_integrity)?
    .count;
    if existing == 0 {
        return match selector {
            AttributionEpochSelector::Legacy => Ok(ResolvedAttributionEpoch::Legacy),
            _ => Err(unavailable(
                "BR-255 attribution epoch storage has not been installed",
            )),
        };
    }
    if existing != 7 {
        return Err(failed_integrity(format!(
            "BR-255 attribution epoch schema is partial: {existing} of 7 tables"
        )));
    }
    let rows = validate_all(conn).map_err(map_integrity)?;
    match selector {
        AttributionEpochSelector::Legacy => Ok(ResolvedAttributionEpoch::Legacy),
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

/// Fully validates and resolves one exact epoch using the caller's existing
/// SQLite transaction connection. This read-only crate seam does not acquire a
/// second connection or begin a nested transaction.
pub(crate) fn load_exact_on_connection(
    conn: &mut SqliteConnection,
    epoch_id: &str,
) -> Result<AttributionEpochReceipt, AttributionEpochStoreError> {
    match load_selector_with_connection(
        conn,
        &AttributionEpochSelector::Exact(epoch_id.to_owned()),
    )? {
        ResolvedAttributionEpoch::Epoch(receipt) => Ok(receipt),
        ResolvedAttributionEpoch::Legacy => Err(failed_integrity(
            "BR-255 exact attribution epoch unexpectedly resolved as Legacy",
        )),
    }
}

fn resolve_explicit_preview_path(supplied: &Path) -> Result<PathBuf, AttributionEpochStoreError> {
    let supplied_metadata = std::fs::symlink_metadata(supplied).map_err(|error| {
        activation_unavailable(
            "attribution_epoch_preview_storage_unavailable",
            false,
            format!("BR-255 explicit preview database is unavailable: {error}"),
        )
    })?;
    if supplied_metadata.file_type().is_symlink() || !supplied_metadata.is_file() {
        return Err(failed_integrity(
            "BR-255 explicit preview database must be a non-symlink regular file",
        ));
    }
    let resolved = std::fs::canonicalize(supplied).map_err(|error| {
        activation_unavailable(
            "attribution_epoch_preview_storage_unavailable",
            false,
            format!("BR-255 explicit preview database cannot be resolved: {error}"),
        )
    })?;
    let resolved_metadata = std::fs::symlink_metadata(&resolved).map_err(|error| {
        activation_unavailable(
            "attribution_epoch_preview_storage_unavailable",
            false,
            format!("BR-255 resolved preview database is unavailable: {error}"),
        )
    })?;
    if resolved_metadata.file_type().is_symlink() || !resolved_metadata.is_file() {
        return Err(failed_integrity(
            "BR-255 resolved preview database is not a regular file",
        ));
    }
    Ok(resolved)
}

fn preview_activation_at_resolved_path(
    database_path: &Path,
    request: &EpochActivationRequest,
) -> Result<EpochActivationPreview, AttributionEpochStoreError> {
    let (completed, effective, calendar_hash) = resolve_activation_calendar(request.invoked_at)?;
    let before = preview_sqlite_snapshot(database_path)?;
    if let Err(error) = reject_preview_wal_state(database_path, &before) {
        let after = preview_sqlite_snapshot(database_path)?;
        if after != before {
            return Err(failed_integrity(
                "BR-255 SQLite files changed while refusing unsafe preview WAL state",
            ));
        }
        return Err(error);
    }
    let clean_wal_header = sqlite_header_uses_wal(database_path)?;
    let primary = (|| {
        let read_only_url = read_only_sqlite_uri(database_path, clean_wal_header)?;
        let mut conn = SqliteConnection::establish(&read_only_url).map_err(|error| {
            activation_unavailable(
                "attribution_epoch_preview_storage_unavailable",
                true,
                format!("BR-255 cold read-only preview connection unavailable: {error}"),
            )
        })?;
        diesel::sql_query("PRAGMA query_only=ON")
            .execute(&mut conn)
            .map_err(|error| {
                activation_unavailable(
                    "attribution_epoch_preview_storage_unavailable",
                    true,
                    format!("BR-255 preview connection is not query-only: {error}"),
                )
            })?;
        conn.transaction::<_, AttributionEpochStoreError, _>(|conn| {
            let existing_tables = diesel::sql_query(
                "SELECT COUNT(*) AS count FROM sqlite_master
                 WHERE type='table' AND name IN (
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
            if existing_tables != 0 && existing_tables != 7 {
                return Err(failed_integrity(format!(
                    "BR-255 preview found partial epoch schema: {existing_tables} of 7 tables"
                )));
            }
            if existing_tables == 7 {
                let receipts = validate_all(conn)?;
                if receipts.len() > 1 {
                    return Err(failed_integrity(
                        "BR-255 v1 preview found multiple activation receipts",
                    ));
                }
                if let Some(row) = receipts.first() {
                    let receipt = receipt_value(row)?;
                    let source = ensure_source_matches_receipt(conn, &receipt)?;
                    return activation_preview(
                        true,
                        receipt.cutover_completed_trading_date,
                        receipt.effective_trading_date,
                        source,
                        &receipt.calendar_authority_hash,
                        Some(receipt.receipt_hash),
                    );
                }
            }
            let source = analyze_source_projection(conn, completed, None, true)?;
            activation_preview(false, completed, effective, source, &calendar_hash, None)
        })
    })();
    let after = preview_sqlite_snapshot(database_path)?;
    if after != before {
        return Err(failed_integrity(
            "BR-255 read-only preview changed main/WAL/SHM bytes, existence, or mtime",
        ));
    }
    primary
}

impl<'a> AttributionEpochStore<'a> {
    pub fn new(database: &'a DatabaseManager) -> Self {
        let read_only_preview_path = database
            .attribution_connection_source
            .as_ref()
            .or(database.selection_connection_source.as_ref())
            .and_then(|source| {
                super::sqlite_open_route_from_retained_parent(&source.parent, &source.leaf).ok()
            });
        Self {
            database,
            read_only_preview_path,
        }
    }

    /// Supplies the filesystem route used only to establish a cold `mode=ro`
    /// preview connection. Activation continues to use the manager's verified
    /// write connection and never trusts this route for mutation authority.
    pub fn with_read_only_preview_path(
        database: &'a DatabaseManager,
        database_path: PathBuf,
    ) -> Self {
        Self {
            database,
            read_only_preview_path: Some(database_path),
        }
    }

    pub fn preview_activation(
        &self,
        request: &EpochActivationRequest,
    ) -> Result<EpochActivationPreview, AttributionEpochStoreError> {
        let supplied_path = self.read_only_preview_path.as_ref().ok_or_else(|| {
            activation_unavailable(
                "attribution_epoch_preview_read_only_path_unavailable",
                false,
                "BR-255 preview requires a database-owned cold read-only route",
            )
        })?;
        let database_path = resolve_explicit_preview_path(supplied_path)?;
        preview_activation_at_resolved_path(&database_path, request)
    }

    /// Runs the same cold, immutable preview directly against an explicit path
    /// without constructing a regular SQLite pool or installing any schema.
    pub fn preview_activation_at_path(
        database_path: impl AsRef<Path>,
        request: &EpochActivationRequest,
    ) -> Result<EpochActivationPreview, AttributionEpochStoreError> {
        let database_path = resolve_explicit_preview_path(database_path.as_ref())?;
        preview_activation_at_resolved_path(&database_path, request)
    }

    pub fn activate_once(
        &self,
        request: EpochActivationRequest,
    ) -> Result<AttributionEpochReceipt, AttributionEpochStoreError> {
        self.activate_once_with_outcome(request)
            .map(EpochActivationOutcome::into_receipt)
    }

    pub fn activate_once_with_outcome(
        &self,
        request: EpochActivationRequest,
    ) -> Result<EpochActivationOutcome, AttributionEpochStoreError> {
        let (completed, effective, calendar_hash) =
            match resolve_activation_calendar(request.invoked_at) {
                Ok(resolved) => resolved,
                Err(error) => return self.return_audited_failure(&request, error),
            };
        let mut conn = match self.database.get_conn() {
            Ok(conn) => conn,
            Err(error) => {
                let primary = activation_unavailable(
                    "attribution_epoch_storage_unavailable",
                    true,
                    format!("BR-255 activation database connection unavailable: {error}"),
                );
                return self.return_audited_failure(&request, primary);
            }
        };
        let database_file =
            match diesel::sql_query("SELECT file FROM pragma_database_list WHERE name='main'")
                .get_result::<DatabaseFileRow>(&mut conn)
            {
                Ok(row) => row.file,
                Err(error) => {
                    drop(conn);
                    return self.return_audited_failure(&request, error.into());
                }
            };
        let invoked_at = canonical_invoked_at(request.invoked_at);
        let primary = conn.immediate_transaction::<_, AttributionEpochStoreError, _>(|conn| {
            let receipts = validate_all(conn).map_err(|error| {
                activation_error_context("activation_transaction_validate_initial", error.into())
            })?;
            if receipts.len() > 1 {
                return Err(failed_integrity(
                    "BR-255 v1 activation found multiple success receipts",
                ));
            }
            if let Some(row) = receipts.first() {
                let receipt = receipt_value(row)?;
                let source = ensure_source_matches_receipt(conn, &receipt).map_err(|error| {
                    activation_error_context("activation_idempotent_verify_source", error)
                })?;
                insert_attempt_on_conn(
                    conn,
                    &AttributionEpochAttemptAppend {
                        source: source_name(request.source).to_owned(),
                        invoked_at: invoked_at.clone(),
                        completed_session_date: Some(receipt.cutover_completed_trading_date),
                        effective_date: Some(receipt.effective_trading_date),
                        outcome: "success".to_owned(),
                        reason_code: "attribution_epoch_idempotent_success".to_owned(),
                        retryable: false,
                        source_summary_hash: receipt.receipt_hash.clone(),
                        epoch_id: Some(receipt.epoch_id.clone()),
                        success_receipt_hash: Some(receipt.receipt_hash.clone()),
                    },
                )
                .map_err(|error| {
                    activation_error_context("activation_idempotent_append_attempt", error)
                })?;
                validate_all(conn).map_err(|error| {
                    activation_error_context(
                        "activation_idempotent_validate_committed_state",
                        error.into(),
                    )
                })?;
                return verified_activation_outcome(receipt, source);
            }
            let source_before =
                analyze_source_projection(conn, completed, None, true).map_err(|error| {
                    activation_error_context("activation_first_analyze_source", error)
                })?;
            let preview = activation_preview(
                false,
                completed,
                effective,
                source_before,
                &calendar_hash,
                None,
            )?;
            let receipt =
                insert_activation_receipt(conn, &preview, &calendar_hash).map_err(|error| {
                    activation_error_context("activation_first_insert_receipt", error)
                })?;
            insert_attempt_on_conn(
                conn,
                &AttributionEpochAttemptAppend {
                    source: source_name(request.source).to_owned(),
                    invoked_at: invoked_at.clone(),
                    completed_session_date: Some(completed),
                    effective_date: Some(effective),
                    outcome: "success".to_owned(),
                    reason_code: "attribution_epoch_activated".to_owned(),
                    retryable: false,
                    source_summary_hash: receipt.receipt_hash.clone(),
                    epoch_id: Some(receipt.epoch_id.clone()),
                    success_receipt_hash: Some(receipt.receipt_hash.clone()),
                },
            )
            .map_err(|error| activation_error_context("activation_first_append_attempt", error))?;
            let source_after =
                analyze_source_projection(conn, completed, None, true).map_err(|error| {
                    activation_error_context("activation_first_reanalyze_source", error)
                })?;
            let after = activation_preview(
                false,
                completed,
                effective,
                source_after,
                &calendar_hash,
                None,
            )?;
            if after != preview {
                return Err(failed_integrity(
                    "BR-255 source position projection changed during activation",
                ));
            }
            validate_all(conn).map_err(|error| {
                activation_error_context("activation_first_validate_written_state", error.into())
            })?;
            let source = ensure_source_matches_receipt(conn, &receipt).map_err(|error| {
                activation_error_context("activation_first_verify_written_source", error)
            })?;
            verified_activation_outcome(receipt, source)
        });
        drop(conn);

        let outcome = match primary {
            Ok(outcome) => outcome,
            Err(error) => return self.return_audited_failure(&request, error),
        };
        let receipt = outcome.receipt();
        let read_back = (|| {
            if database_file.trim().is_empty() {
                return Err(failed_integrity(
                    "BR-255 committed activation cannot be reopened read-only",
                ));
            }
            let read_only_url = format!("file:{database_file}?mode=ro");
            let mut read_only = SqliteConnection::establish(&read_only_url).map_err(|error| {
                failed_integrity(format!(
                    "BR-255 committed activation read-back connection failed: {error}"
                ))
            })?;
            diesel::sql_query("PRAGMA query_only=ON")
                .execute(&mut read_only)
                .map_err(|error| {
                    failed_integrity(format!(
                        "BR-255 committed activation read-back is not query-only: {error}"
                    ))
                })?;
            let rows = validate_all(&mut read_only).map_err(|error| {
                activation_error_context(
                    "post_commit_read_back_validate_retained_state",
                    error.into(),
                )
            })?;
            if rows.len() != 1 || &receipt_value(&rows[0])? != receipt {
                return Err(failed_integrity(
                    "BR-255 committed activation read-back receipt differs",
                ));
            }
            verify_epoch_source_prefix(&mut read_only, receipt).map_err(|error| {
                activation_error_context("post_commit_read_back_verify_source_prefix", error)
            })?;
            Ok(())
        })();
        if let Err(error) = read_back {
            return self.return_audited_failure(&request, error);
        }
        Ok(outcome)
    }

    fn return_audited_failure<T>(
        &self,
        request: &EpochActivationRequest,
        primary: AttributionEpochStoreError,
    ) -> Result<T, AttributionEpochStoreError> {
        let (outcome, retryable) = match &primary {
            AttributionEpochStoreError::Unavailable { retryable, .. } => {
                ("unavailable", *retryable)
            }
            AttributionEpochStoreError::FailedIntegrity { .. } => ("failed_integrity", false),
        };
        let dates = resolve_activation_calendar(request.invoked_at).ok();
        let source_summary_hash = match hash_json(
            b"BR255_ATTRIBUTION_ACTIVATION_FAILURE_SUMMARY_V1\0",
            &(
                source_name(request.source),
                canonical_invoked_at(request.invoked_at),
                primary.reason_code(),
                primary.to_string(),
            ),
        ) {
            Ok(hash) => hash,
            Err(error) => {
                return Err(AttributionEpochStoreError::Unavailable {
                    reason_code: "epoch_attempt_audit_unavailable",
                    retryable: primary.retryable(),
                    detail: format!(
                        "BR-255 primary activation failure summary cannot be audited: primary={primary}; hash={error}"
                    ),
                });
            }
        };
        let append = self.append_attempt(AttributionEpochAttemptAppend {
            source: source_name(request.source).to_owned(),
            invoked_at: canonical_invoked_at(request.invoked_at),
            completed_session_date: dates.as_ref().map(|value| value.0),
            effective_date: dates.as_ref().map(|value| value.1),
            outcome: outcome.to_owned(),
            reason_code: primary.reason_code().to_owned(),
            retryable,
            source_summary_hash,
            epoch_id: None,
            success_receipt_hash: None,
        });
        match append {
            Ok(_) => Err(primary),
            Err(audit_error) => Err(AttributionEpochStoreError::Unavailable {
                reason_code: "epoch_attempt_audit_unavailable",
                retryable: primary.retryable() || audit_error.retryable(),
                detail: format!(
                    "BR-255 primary activation failure could not be audited: primary={primary}; audit={audit_error}"
                ),
            }),
        }
    }

    pub fn load_selector(
        &self,
        selector: &AttributionEpochSelector,
    ) -> Result<ResolvedAttributionEpoch, AttributionEpochStoreError> {
        let mut conn =
            self.database
                .get_conn()
                .map_err(|error| AttributionEpochStoreError::Unavailable {
                    reason_code: "attribution_epoch_storage_unavailable",
                    retryable: true,
                    detail: format!("BR-255 epoch database connection unavailable: {error}"),
                })?;
        load_selector_with_connection(&mut conn, selector)
    }

    pub(crate) fn load_active_verified_fills_until(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<
        (
            AttributionEpochReceipt,
            VerifiedEpochFillSet,
            DatabaseConnectionAuthority,
        ),
        AttributionEpochStoreError,
    > {
        if from > to {
            return Err(failed_integrity(
                "BR-255 epoch attribution source range is reversed",
            ));
        }
        let mut checkout = self
            .database
            .attribution_checkout()
            .map_err(map_database_authority_error)?;
        checkout.transaction_with_authority(map_database_authority_error, |conn, authority| {
            let resolved = load_selector_with_connection(conn, &AttributionEpochSelector::Active)?;
            let receipt = match &resolved {
                ResolvedAttributionEpoch::Epoch(receipt) => receipt.clone(),
                ResolvedAttributionEpoch::Legacy => {
                    unreachable!("active selector cannot resolve Legacy")
                }
            };
            if from < receipt.effective_trading_date {
                return Err(AttributionEpochStoreError::FailedIntegrity {
                    reason_code: "attribution_epoch_range_before_effective",
                    detail: format!(
                        "BR-255 attribution range {from}..={to} precedes effective date {}",
                        receipt.effective_trading_date
                    ),
                });
            }
            #[cfg(test)]
            maybe_inject_active_verified_source_drift(conn)?;
            let verified = load_verified_epoch_fills_until(conn, &resolved, to)?;
            Ok((receipt, verified, authority.clone()))
        })
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
            || ((input.outcome == "success")
                != (input.epoch_id.is_some() && input.success_receipt_hash.is_some()))
            || (input.outcome != "success"
                && (input.epoch_id.is_some() || input.success_receipt_hash.is_some()))
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
        let mut batch = self.append_daily_batch_core(
            AttributionEpochDailyBatchAppend {
                epoch_id: input.epoch_id,
                date: input.date,
                families: vec![AttributionEpochDailyFamilyAppend {
                    signal_family: input.signal_family,
                    payload: input.payload,
                }],
            },
            None,
        )?;
        Ok(batch
            .receipts
            .pop()
            .expect("validated batch-of-one returns exactly one receipt"))
    }

    #[cfg(test)]
    pub(crate) fn append_daily_batch(
        &self,
        input: AttributionEpochDailyBatchAppend,
    ) -> Result<AttributionEpochDailyBatchReceipt, AttributionEpochStoreError> {
        self.append_daily_batch_core(input, None)
    }

    pub(crate) fn append_verified_daily_batch(
        &self,
        input: AttributionEpochDailyBatchAppend,
        source: AttributionEpochDailySourceBinding,
    ) -> Result<AttributionEpochDailyBatchReceipt, AttributionEpochStoreError> {
        self.append_daily_batch_core(input, Some(source))
    }

    fn append_daily_batch_core(
        &self,
        input: AttributionEpochDailyBatchAppend,
        source: Option<AttributionEpochDailySourceBinding>,
    ) -> Result<AttributionEpochDailyBatchReceipt, AttributionEpochStoreError> {
        if !lower_hash(&input.epoch_id) || input.families.is_empty() {
            return Err(failed_integrity("BR-255 invalid epoch daily batch input"));
        }
        let mut seen = HashSet::with_capacity(input.families.len());
        let mut prepared = Vec::with_capacity(input.families.len());
        for family in input.families {
            if family.signal_family.trim().is_empty()
                || family.signal_family.trim() != family.signal_family
                || !seen.insert(family.signal_family.clone())
            {
                return Err(failed_integrity(
                    "BR-255 invalid or duplicate epoch daily signal family",
                ));
            }
            let payload_json = serde_json::to_string(&family.payload).map_err(|error| {
                failed_integrity(format!("BR-255 serialize daily payload: {error}"))
            })?;
            let payload_hash =
                hash_json(b"BR255_ATTRIBUTION_EPOCH_DAILY_PAYLOAD_V1\0", &payload_json)
                    .map_err(map_integrity)?;
            prepared.push((family.signal_family, payload_json, payload_hash));
        }
        prepared.sort_by(|left, right| left.0.cmp(&right.0));
        let epoch_id = input.epoch_id;
        let date = input.date;
        let date_string = date.to_string();
        if let Some(source) = source.as_ref() {
            let mut checkout = self
                .database
                .attribution_checkout()
                .map_err(map_database_authority_error)?;
            return checkout.immediate_transaction_with_authority(
                map_database_authority_error,
                |conn, authority| {
                    append_daily_batch_on_connection(
                        conn,
                        &epoch_id,
                        date,
                        &date_string,
                        &prepared,
                        Some((source, authority)),
                    )
                },
            );
        }
        let mut conn =
            self.database
                .get_conn()
                .map_err(|error| AttributionEpochStoreError::Unavailable {
                    reason_code: "attribution_epoch_storage_unavailable",
                    retryable: true,
                    detail: format!("BR-255 epoch database connection unavailable: {error}"),
                })?;
        conn.immediate_transaction::<_, AttributionEpochStoreError, _>(|conn| {
            append_daily_batch_on_connection(conn, &epoch_id, date, &date_string, &prepared, None)
        })
    }
}

fn append_daily_batch_on_connection(
    conn: &mut SqliteConnection,
    epoch_id: &str,
    date: NaiveDate,
    date_string: &str,
    prepared: &[(String, String, String)],
    source: Option<(
        &AttributionEpochDailySourceBinding,
        &DatabaseConnectionAuthority,
    )>,
) -> Result<AttributionEpochDailyBatchReceipt, AttributionEpochStoreError> {
    let mut state = validate_daily(conn).map_err(map_integrity)?;
    let epochs = validate_all(conn).map_err(map_integrity)?;
    if !epochs.iter().any(|receipt| receipt.epoch_id == epoch_id) {
        return Err(failed_integrity(
            "BR-255 epoch daily append references an unknown epoch",
        ));
    }
    if let Some((source, authority)) = source {
        if source.epoch_id != epoch_id || source.cutoff_date != date {
            return Err(failed_integrity(
                "BR-255 verified epoch daily batch identity is inconsistent",
            ));
        }
        validate_daily_source_binding(conn, authority, source)?;
    }
    let mut batch_receipts = Vec::with_capacity(prepared.len());
    #[cfg(test)]
    let mut completed_writes = 0_usize;
    for (signal_family, payload_json, payload_hash) in prepared {
        let existing = state
            .iter()
            .find(|row| {
                row.epoch_id == epoch_id
                    && row.date == date_string
                    && row.signal_family == *signal_family
                    && row.payload_hash == *payload_hash
            })
            .cloned();
        if let Some(existing) = existing {
            batch_receipts.push(daily_receipt(&state, &existing).map_err(map_integrity)?);
            continue;
        }
        #[cfg(test)]
        maybe_inject_daily_batch_failure(completed_writes).map_err(map_integrity)?;
        let previous = state
            .last()
            .map_or(DAILY_GENESIS, |row| row.record_hash.as_str());
        let window = new_window(conn).map_err(map_integrity)?;
        let mut row = PersistedDaily {
            id: 0,
            epoch_id: epoch_id.to_owned(),
            date: date_string.to_owned(),
            signal_family: signal_family.clone(),
            payload_json: payload_json.clone(),
            payload_hash: payload_hash.clone(),
            predecessor_daily_hash: previous.to_owned(),
            record_hash: String::new(),
            created_at: window.created_at,
            retention_deadline: window.retention_deadline,
        };
        row.record_hash = daily_hash(&row).map_err(map_integrity)?;
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
        .execute(conn)
        .map_err(AttributionEpochStoreError::from)?;
        row.id = diesel::select(diesel::dsl::sql::<BigInt>("last_insert_rowid()"))
            .get_result(conn)
            .map_err(AttributionEpochStoreError::from)?;
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
        .execute(conn)
        .map_err(AttributionEpochStoreError::from)?;
        state.push(row.clone());
        batch_receipts.push(daily_receipt(&state, &row).map_err(map_integrity)?);
        #[cfg(test)]
        {
            completed_writes += 1;
        }
    }
    validate_all(conn).map_err(map_integrity)?;
    Ok(AttributionEpochDailyBatchReceipt {
        epoch_id: epoch_id.to_owned(),
        date,
        receipts: batch_receipts,
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use diesel::connection::SimpleConnection;
    use diesel::prelude::*;
    use diesel::sql_types::Text;

    use super::*;

    fn activation_request(raw: &str) -> EpochActivationRequest {
        EpochActivationRequest {
            source: crate::performance::attribution_epoch::EpochActivationSource::Monitor,
            invoked_at: DateTime::parse_from_rfc3339(raw).expect("TEST_CODE fixed invocation"),
        }
    }

    fn activated_test_database() -> (TestDatabase, AttributionEpochReceipt) {
        let database = TestDatabase::attested();
        install_activation_source(&database.manager);
        let receipt = AttributionEpochStore::new(&database.manager)
            .activate_once(activation_request("2026-08-28T15:40:00+08:00"))
            .expect("TEST_CODE active epoch");
        (database, receipt)
    }

    fn assert_daily_storage_pristine(database: &DatabaseManager) {
        let mut conn = database.get_conn().unwrap();
        assert!(load_daily(&mut conn).unwrap().is_empty());
        for table in [
            "paper_attribution_epoch_daily",
            "paper_attribution_epoch_daily_chain",
        ] {
            let count = diesel::sql_query(format!("SELECT COUNT(*) AS count FROM {table}"))
                .get_result::<CountRow>(&mut conn)
                .unwrap()
                .count;
            assert_eq!(count, 0, "TEST_CODE pristine row count for {table}");
            let seq = diesel::sql_query("SELECT seq FROM sqlite_sequence WHERE name=?")
                .bind::<Text, _>(table)
                .get_result::<SequenceRow>(&mut conn)
                .unwrap()
                .seq;
            assert_eq!(seq, Some(0));
        }
    }

    fn daily_test_batch(
        date: NaiveDate,
        families: &[(&str, i64)],
    ) -> AttributionEpochDailyBatchAppend {
        AttributionEpochDailyBatchAppend {
            epoch_id: "a".repeat(64),
            date,
            families: families
                .iter()
                .map(
                    |(signal_family, closed)| AttributionEpochDailyFamilyAppend {
                        signal_family: (*signal_family).to_owned(),
                        payload: serde_json::json!({"closed": closed}),
                    },
                )
                .collect(),
        }
    }

    fn daily_test_append() -> AttributionEpochDailyAppend {
        AttributionEpochDailyAppend {
            epoch_id: "a".repeat(64),
            date: NaiveDate::from_ymd_opt(2026, 8, 28).unwrap(),
            signal_family: "NewsCatalyst".to_owned(),
            payload: serde_json::json!({"closed": 1}),
        }
    }

    type TestFill<'a> = (i64, &'a str, &'a str, i64, &'a str);

    fn append_test_fills(manager: &DatabaseManager, fills: &[TestFill<'_>]) {
        for &(id, plan_id, direction, quantity, occurred_at) in fills {
            append_activation_fill(manager, id, plan_id, direction, quantity, occurred_at);
        }
    }

    #[test]
    fn daily_compute_without_active_epoch_fails_before_legacy_source_read() {
        let database = TestDatabase::attested();
        let error = crate::performance::attribution::compute_epoch_daily(
            &database.manager,
            NaiveDate::from_ymd_opt(2026, 8, 31).unwrap(),
            &HashMap::new(),
        )
        .expect_err("TEST_CODE missing active epoch must fail closed");

        assert_eq!(error.reason_code(), "attribution_epoch_unavailable");
        assert!(!error.retryable());
    }

    #[test]
    fn unattested_manager_keeps_operational_queries_but_daily_authority_is_unavailable() {
        let database = TestDatabase::new();
        let mut operational = database
            .manager
            .get_conn()
            .expect("TEST_CODE operational legacy checkout remains available");
        let one = diesel::select(diesel::dsl::sql::<BigInt>("1"))
            .get_result::<i64>(&mut operational)
            .expect("TEST_CODE unrelated operational query succeeds");
        assert_eq!(one, 1);
        drop(operational);

        let error = crate::performance::attribution::compute_epoch_daily(
            &database.manager,
            NaiveDate::from_ymd_opt(2026, 8, 31).unwrap(),
            &HashMap::new(),
        )
        .expect_err("TEST_CODE unattested attribution authority must be unavailable");
        assert_eq!(
            error.reason_code(),
            "attribution_database_authority_unavailable"
        );
    }

    #[test]
    fn daily_compute_maps_prefix_drift_and_terminal_mismatch_to_typed_integrity() {
        let prefix = activated_test_database().0;
        let mut conn = prefix.manager.get_conn().unwrap();
        diesel::sql_query("UPDATE paper_trades SET fill_price=11.0 WHERE id=1")
            .execute(&mut conn)
            .unwrap();
        drop(conn);
        let prefix_error = crate::performance::attribution::compute_epoch_daily(
            &prefix.manager,
            NaiveDate::from_ymd_opt(2026, 8, 31).unwrap(),
            &HashMap::new(),
        )
        .expect_err("TEST_CODE frozen paper prefix drift must fail compute");
        assert_eq!(
            prefix_error.reason_code(),
            "attribution_epoch_integrity_failed"
        );

        let terminal = activated_test_database().0;
        append_duplicate_terminal(&terminal.manager);
        let terminal_error = crate::performance::attribution::compute_epoch_daily(
            &terminal.manager,
            NaiveDate::from_ymd_opt(2026, 8, 31).unwrap(),
            &HashMap::new(),
        )
        .expect_err("TEST_CODE duplicate terminal binding must fail compute");
        assert_eq!(
            terminal_error.reason_code(),
            "attribution_epoch_integrity_failed"
        );
    }

    #[test]
    fn active_selector_and_verified_source_share_one_rollback_safe_snapshot() {
        let database = activated_test_database().0;
        let _drift = inject_active_verified_source_drift();

        let error = crate::performance::attribution::compute_epoch_daily(
            &database.manager,
            NaiveDate::from_ymd_opt(2026, 8, 31).unwrap(),
            &HashMap::new(),
        )
        .expect_err("TEST_CODE source drift inside active read snapshot must fail wholly");
        assert_eq!(error.reason_code(), "attribution_epoch_integrity_failed");
        crate::performance::attribution::compute_epoch_daily(
            &database.manager,
            NaiveDate::from_ymd_opt(2026, 8, 31).unwrap(),
            &HashMap::new(),
        )
        .expect("TEST_CODE injected source mutation rolls back with failed read snapshot");
    }

    #[test]
    fn daily_compute_rejects_post_highwater_fill_before_effective_date() {
        let database = activated_test_database().0;
        append_activation_fill(
            &database.manager,
            3,
            "TEST_CODE_PLAN_LATE",
            "buy",
            100,
            "2026-08-28 07:45:00",
        );

        let error = crate::performance::attribution::compute_epoch_daily(
            &database.manager,
            NaiveDate::from_ymd_opt(2026, 8, 31).unwrap(),
            &HashMap::new(),
        )
        .expect_err("TEST_CODE late source must not produce daily output");
        assert_eq!(error.reason_code(), "attribution_epoch_integrity_failed");
    }

    #[test]
    fn daily_compute_quarantines_carry_through_flat_then_attributes_new_cycle() {
        let (database, receipt) = activated_test_database();
        append_test_fills(
            &database.manager,
            &[
                (
                    3,
                    "TEST_CODE_PLAN_OVERLAP_BUY",
                    "buy",
                    100,
                    "2026-08-31 02:00:00",
                ),
                (
                    4,
                    "TEST_CODE_PLAN_FLAT_SELL",
                    "sell",
                    200,
                    "2026-08-31 03:00:00",
                ),
                (
                    5,
                    "TEST_CODE_PLAN_FRESH_BUY",
                    "buy",
                    100,
                    "2026-08-31 04:00:00",
                ),
                (
                    6,
                    "TEST_CODE_PLAN_FRESH_SELL",
                    "sell",
                    100,
                    "2026-09-01 02:00:00",
                ),
            ],
        );

        let result = crate::performance::attribution::compute_epoch_daily(
            &database.manager,
            NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(),
            &HashMap::new(),
        )
        .expect("TEST_CODE active epoch daily attribution");

        assert_eq!(result.epoch().epoch_id, receipt.epoch_id);
        assert_eq!(result.epoch().receipt_hash, receipt.receipt_hash);
        assert_eq!(result.epoch().effective_date.to_string(), "2026-08-31");
        assert_eq!(result.epoch().exclusions.len(), 3);
        assert_eq!(
            result
                .epoch()
                .exclusions
                .iter()
                .map(|item| item.fill_id)
                .collect::<Vec<_>>(),
            vec![3, 4, 4]
        );
        assert!(result.epoch().remaining_quarantine.is_empty());
        assert_eq!(result.epoch().released_codes, 1);
        assert_eq!(result.daily().date.to_string(), "2026-09-01");
        assert_eq!(
            result
                .daily()
                .families
                .iter()
                .map(|family| family.realized_trades)
                .sum::<i64>(),
            1,
            "TEST_CODE only the post-flat lifecycle is attributable"
        );
    }

    #[test]
    fn epoch_window_rejects_a_range_before_the_effective_date() {
        let database = activated_test_database().0;

        let error = crate::performance::attribution::compute_epoch_window(
            &database.manager,
            NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(),
            3,
            &HashMap::new(),
        )
        .expect_err("TEST_CODE window start before effective must not be clipped");

        assert_eq!(
            error.reason_code(),
            "attribution_epoch_range_before_effective"
        );
    }

    #[test]
    fn epoch_window_uses_scoped_rows_from_effective_date_through_end() {
        let database = activated_test_database().0;
        append_test_fills(
            &database.manager,
            &[
                (
                    3,
                    "TEST_CODE_PLAN_CARRY_EXIT",
                    "sell",
                    100,
                    "2026-08-31 02:00:00",
                ),
                (
                    4,
                    "TEST_CODE_PLAN_WINDOW_BUY",
                    "buy",
                    100,
                    "2026-08-31 03:00:00",
                ),
                (
                    5,
                    "TEST_CODE_PLAN_WINDOW_SELL",
                    "sell",
                    100,
                    "2026-09-01 02:00:00",
                ),
            ],
        );

        let result = crate::performance::attribution::compute_epoch_window(
            &database.manager,
            NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(),
            2,
            &HashMap::new(),
        )
        .expect("TEST_CODE epoch-scoped two-day window");

        assert_eq!(result.window().days, 2);
        assert_eq!(result.window().end.to_string(), "2026-09-01");
        assert_eq!(result.epoch().exclusions.len(), 1);
        assert_eq!(result.epoch().exclusions[0].fill_id, 3);
        assert_eq!(result.epoch().released_codes, 1);
        assert_eq!(
            result
                .window()
                .families
                .iter()
                .map(|family| family.realized_trades)
                .sum::<i64>(),
            1
        );
    }

    #[test]
    fn daily_persist_revalidates_active_source_and_writes_nothing_after_late_fill() {
        let database = activated_test_database().0;
        append_test_fills(
            &database.manager,
            &[
                (
                    3,
                    "TEST_CODE_PLAN_CARRY_EXIT",
                    "sell",
                    100,
                    "2026-08-31 02:00:00",
                ),
                (
                    4,
                    "TEST_CODE_PLAN_DAILY_BUY",
                    "buy",
                    100,
                    "2026-08-31 03:00:00",
                ),
                (
                    5,
                    "TEST_CODE_PLAN_DAILY_SELL",
                    "sell",
                    100,
                    "2026-09-01 02:00:00",
                ),
            ],
        );
        let daily = crate::performance::attribution::compute_epoch_daily(
            &database.manager,
            NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(),
            &HashMap::new(),
        )
        .expect("TEST_CODE computed daily evidence");
        append_activation_fill(
            &database.manager,
            6,
            "TEST_CODE_PLAN_LATE_AFTER_COMPUTE",
            "buy",
            100,
            "2026-08-28 07:45:00",
        );

        let error = crate::performance::attribution::persist_epoch_daily(&database.manager, &daily)
            .expect_err("TEST_CODE late source invalidates the computed evidence");
        assert_eq!(error.reason_code(), "attribution_epoch_integrity_failed");
        assert_daily_storage_pristine(&database.manager);
    }

    #[test]
    fn daily_persist_writes_nothing_after_audit_only_terminal_drift() {
        let database = activated_test_database().0;
        let daily = crate::performance::attribution::compute_epoch_daily(
            &database.manager,
            NaiveDate::from_ymd_opt(2026, 8, 31).unwrap(),
            &HashMap::new(),
        )
        .expect("TEST_CODE computed before audit-only drift");
        append_duplicate_terminal(&database.manager);

        let error = crate::performance::attribution::persist_epoch_daily(&database.manager, &daily)
            .expect_err("TEST_CODE duplicate terminal audit invalidates daily evidence");
        assert_eq!(error.reason_code(), "attribution_epoch_integrity_failed");
        assert_daily_storage_pristine(&database.manager);
    }

    #[test]
    fn daily_persist_writes_nothing_if_active_epoch_changes_after_compute() {
        let (database, active) = activated_test_database();
        append_test_fills(
            &database.manager,
            &[
                (
                    3,
                    "TEST_CODE_PLAN_CARRY_EXIT",
                    "sell",
                    100,
                    "2026-08-31 02:00:00",
                ),
                (
                    4,
                    "TEST_CODE_PLAN_DAILY_BUY",
                    "buy",
                    100,
                    "2026-08-31 03:00:00",
                ),
            ],
        );
        let daily = crate::performance::attribution::compute_epoch_daily(
            &database.manager,
            NaiveDate::from_ymd_opt(2026, 8, 31).unwrap(),
            &HashMap::new(),
        )
        .expect("TEST_CODE computed first-epoch daily evidence");
        insert_success(&database.manager, Some(active.receipt_hash));

        let error = crate::performance::attribution::persist_epoch_daily(&database.manager, &daily)
            .expect_err("TEST_CODE changed active receipt set must fail closed");
        assert_eq!(error.reason_code(), "attribution_epoch_integrity_failed");
        assert_daily_storage_pristine(&database.manager);
    }

    #[test]
    fn daily_persist_rejects_late_non_filled_paper_highwater_without_audit_change() {
        for status in ["NotFilled", "Invalidated"] {
            let database = activated_test_database().0;
            let daily = crate::performance::attribution::compute_epoch_daily(
                &database.manager,
                NaiveDate::from_ymd_opt(2026, 8, 31).unwrap(),
                &HashMap::new(),
            )
            .expect("TEST_CODE computed before all-status source drift");
            append_non_filled_paper(&database.manager, 3, status);

            let error =
                crate::performance::attribution::persist_epoch_daily(&database.manager, &daily)
                    .expect_err("TEST_CODE all-status paper highwater drift must reject persist");
            assert_eq!(error.reason_code(), "attribution_epoch_integrity_failed");
            assert_daily_storage_pristine(&database.manager);
        }
    }

    #[test]
    fn daily_persist_rejects_valid_non_filled_content_mutation_without_highwater_change() {
        for status in ["NotFilled", "Invalidated"] {
            let database = activated_test_database().0;
            append_non_filled_paper(&database.manager, 3, status);
            let daily = crate::performance::attribution::compute_epoch_daily(
                &database.manager,
                NaiveDate::from_ymd_opt(2026, 8, 31).unwrap(),
                &HashMap::new(),
            )
            .expect("TEST_CODE computed after valid non-filled row");
            let mut conn = database.manager.get_conn().unwrap();
            diesel::sql_query(
                "UPDATE paper_trades
                 SET not_fill_reason='TEST_CODE another valid rejection reason',
                     updated_at='2026-08-31 04:00:02'
                 WHERE id=3",
            )
            .execute(&mut conn)
            .unwrap();
            drop(conn);

            let error =
                crate::performance::attribution::persist_epoch_daily(&database.manager, &daily)
                    .expect_err("TEST_CODE all-status content drift must reject persist");
            assert_eq!(error.reason_code(), "attribution_epoch_integrity_failed");
            assert_daily_storage_pristine(&database.manager);
        }
    }

    #[test]
    fn daily_persist_rejects_valid_non_filled_gap_insert_below_existing_highwater() {
        let database = activated_test_database().0;
        append_non_filled_paper(&database.manager, 4, "NotFilled");
        let daily = crate::performance::attribution::compute_epoch_daily(
            &database.manager,
            NaiveDate::from_ymd_opt(2026, 8, 31).unwrap(),
            &HashMap::new(),
        )
        .expect("TEST_CODE computed with an unused paper identity below MAX(id)");
        append_non_filled_paper(&database.manager, 3, "Invalidated");

        let error = crate::performance::attribution::persist_epoch_daily(&database.manager, &daily)
            .expect_err("TEST_CODE below-highwater insertion must reject persist");
        assert_eq!(error.reason_code(), "attribution_epoch_integrity_failed");
        assert_daily_storage_pristine(&database.manager);
    }

    #[test]
    fn daily_persist_rejects_an_identical_but_distinct_database_authority() {
        let database_a = activated_test_database().0;
        let daily = crate::performance::attribution::compute_epoch_daily(
            &database_a.manager,
            NaiveDate::from_ymd_opt(2026, 8, 31).unwrap(),
            &HashMap::new(),
        )
        .expect("TEST_CODE computed from database A");
        let database_b = TestDatabase::snapshot_of(&database_a);

        let error =
            crate::performance::attribution::persist_epoch_daily(&database_b.manager, &daily)
                .expect_err("TEST_CODE database B must not accept database A evidence");
        assert_eq!(error.reason_code(), "attribution_epoch_integrity_failed");
        assert_daily_storage_pristine(&database_b.manager);

        crate::performance::attribution::persist_epoch_daily(&database_a.manager, &daily)
            .expect("TEST_CODE originating database authority remains accepted");
    }

    #[test]
    fn daily_persist_latches_idle_registered_checkout_drift_instead_of_pool_self_healing() {
        let database = activated_test_database().0;
        let daily = crate::performance::attribution::compute_epoch_daily(
            &database.manager,
            NaiveDate::from_ymd_opt(2026, 8, 31).unwrap(),
            &HashMap::new(),
        )
        .expect("TEST_CODE computed before idle checkout drift");
        let mut checkout = database.manager.attribution_checkout().unwrap();
        diesel::sql_query(format!(
            "DROP TRIGGER {}",
            super::super::DESCRIPTOR_ATTESTATION_NO_UPDATE_TRIGGER
        ))
        .execute(checkout.connection_for_test())
        .expect("TEST_CODE remove one live registration protection trigger");
        drop(checkout);

        let error = crate::performance::attribution::persist_epoch_daily(&database.manager, &daily)
            .expect_err("TEST_CODE registered checkout drift must poison attribution source");
        assert_eq!(error.reason_code(), "attribution_epoch_integrity_failed");
        assert_daily_storage_pristine(&database.manager);
    }

    #[test]
    fn transaction_exit_integrity_overrides_business_error_and_rolls_back() {
        fn drift_then_fail(
            connection: &mut SqliteConnection,
            _authority: &DatabaseConnectionAuthority,
        ) -> Result<(), AttributionEpochStoreError> {
            diesel::sql_query("INSERT INTO TEST_CODE_authority_exit_probe(marker) VALUES (1)")
                .execute(connection)?;
            diesel::sql_query(format!(
                "DROP TRIGGER {}",
                super::super::DESCRIPTOR_ATTESTATION_NO_UPDATE_TRIGGER
            ))
            .execute(connection)?;
            Err(AttributionEpochStoreError::Unavailable {
                reason_code: "TEST_CODE_business_error",
                retryable: false,
                detail: "TEST_CODE operation failed after authority drift".into(),
            })
        }

        for immediate in [false, true] {
            let database = TestDatabase::attested();
            let mut operational = database.manager.get_conn().unwrap();
            diesel::sql_query(
                "CREATE TABLE TEST_CODE_authority_exit_probe (marker INTEGER NOT NULL)",
            )
            .execute(&mut operational)
            .unwrap();
            drop(operational);
            let mut checkout = database.manager.attribution_checkout().unwrap();

            let result = if immediate {
                checkout.immediate_transaction_with_authority(
                    map_database_authority_error,
                    drift_then_fail,
                )
            } else {
                checkout.transaction_with_authority(map_database_authority_error, drift_then_fail)
            };
            let error =
                result.expect_err("TEST_CODE exit authority must override the operation error");
            assert_eq!(error.reason_code(), "attribution_epoch_integrity_failed");

            let mut operational = database.manager.get_conn().unwrap();
            let count =
                diesel::sql_query("SELECT COUNT(*) AS count FROM TEST_CODE_authority_exit_probe")
                    .get_result::<CountRow>(&mut operational)
                    .unwrap()
                    .count;
            assert_eq!(
                count, 0,
                "TEST_CODE authority failure rolls back operation, immediate={immediate}"
            );
            drop(operational);
            let followup = match database.manager.attribution_checkout() {
                Ok(_) => panic!("TEST_CODE first integrity failure must remain latched"),
                Err(error) => error,
            };
            assert!(matches!(
                followup,
                DatabaseAuthorityError::DescriptorIntegrityFailed { .. }
            ));
        }
    }

    #[test]
    fn daily_authority_is_checkout_attested_and_rejects_byte_identical_path_replacement() {
        let database_a = TestDatabase::with_wal_options(2, true);
        install_activation_source(&database_a.manager);
        AttributionEpochStore::new(&database_a.manager)
            .activate_once(activation_request("2026-08-28T15:40:00+08:00"))
            .expect("TEST_CODE active epoch in database A");

        let mut first = database_a.manager.attribution_checkout().unwrap();
        let first_authority = first
            .authority()
            .expect("TEST_CODE first checkout authority");
        let mut second = database_a.manager.attribution_checkout().unwrap();
        let second_authority = second
            .authority()
            .expect("TEST_CODE second checkout authority");
        assert_eq!(first_authority, second_authority);
        drop(second);
        diesel::sql_query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(first.connection_for_test())
            .expect("TEST_CODE checkpoint database A before byte copy");
        drop(first);

        let replacement_copy = database_a.path.with_extension("replacement-copy.sqlite");
        std::fs::copy(&database_a.path, &replacement_copy)
            .expect("TEST_CODE copy byte-identical database B");
        assert_eq!(
            std::fs::read(&database_a.path).unwrap(),
            std::fs::read(&replacement_copy).unwrap()
        );
        let daily = crate::performance::attribution::compute_epoch_daily(
            &database_a.manager,
            NaiveDate::from_ymd_opt(2026, 8, 31).unwrap(),
            &HashMap::new(),
        )
        .expect("TEST_CODE computed from attested database A");
        let mut detached_checkout = database_a.manager.attribution_checkout().unwrap();

        let detached_main = database_a.path.with_extension("detached-a.sqlite");
        std::fs::rename(&database_a.path, &detached_main)
            .expect("TEST_CODE detach open database A main");
        for suffix in ["-wal", "-shm"] {
            let source = sqlite_sidecar_path(&database_a.path, suffix);
            if source.exists() {
                std::fs::rename(&source, sqlite_sidecar_path(&detached_main, suffix))
                    .expect("TEST_CODE detach open database A sidecar");
            }
        }
        std::fs::copy(&replacement_copy, &database_a.path)
            .expect("TEST_CODE install byte-identical database B at original pathname");
        assert_eq!(
            std::fs::read(&database_a.path).unwrap(),
            std::fs::read(&replacement_copy).unwrap()
        );

        let mut bootstrap_b = SqliteConnection::establish(&database_a.path.to_string_lossy())
            .expect("TEST_CODE open replacement database B");
        let journal_mode = diesel::sql_query("PRAGMA journal_mode = WAL")
            .get_result::<super::super::JournalModeRow>(&mut bootstrap_b)
            .unwrap()
            .journal_mode;
        assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
        drop(bootstrap_b);
        let (attribution_pool_b, source_b) =
            super::super::build_attested_sqlite_pool_with_size(&database_a.path, 1)
                .expect("TEST_CODE attested replacement database B pool");
        let pool_b = super::super::build_sqlite_pool_with_size(
            database_a.path.to_string_lossy().into_owned(),
            1,
        )
        .expect("TEST_CODE replacement database B operational pool");
        let manager_b = DatabaseManager {
            pool: pool_b,
            attribution_pool: Some(attribution_pool_b),
            attribution_connection_source: Some(source_b),
            selection_connection_source: None,
            selection_schema_authority: None,
        };

        let mut genuine_b_checkout = manager_b.attribution_checkout().unwrap();
        let manager_b_token =
            super::super::connection_attestation_token(genuine_b_checkout.connection_for_test())
                .expect("TEST_CODE read manager B registration token");
        let manager_a_token =
            super::super::connection_attestation_token(detached_checkout.connection_for_test())
                .expect("TEST_CODE read detached manager A registration token");
        assert_ne!(manager_a_token, manager_b_token);
        diesel::sql_query(format!(
            "UPDATE {} SET token=? WHERE slot=1",
            super::super::DESCRIPTOR_ATTESTATION_TEMP_TABLE
        ))
        .bind::<Text, _>(&manager_b_token)
        .execute(detached_checkout.connection_for_test())
        .expect_err("TEST_CODE attestation token update must be trigger-protected");
        diesel::sql_query(format!(
            "DELETE FROM {} WHERE slot=1",
            super::super::DESCRIPTOR_ATTESTATION_TEMP_TABLE
        ))
        .execute(detached_checkout.connection_for_test())
        .expect_err("TEST_CODE attestation token delete must be trigger-protected");
        assert_ne!(genuine_b_checkout.authority().unwrap(), first_authority);
        drop(genuine_b_checkout);

        let drift = detached_checkout
            .authority()
            .expect_err("TEST_CODE detached A checkout must fail closed after namespace drift");
        assert!(matches!(
            drift,
            DatabaseAuthorityError::DescriptorIntegrityFailed { .. }
        ));
        let same_manager_error =
            crate::performance::attribution::persist_epoch_daily(&database_a.manager, &daily)
                .expect_err("TEST_CODE original manager must reject its drifted namespace");
        assert_eq!(
            same_manager_error.reason_code(),
            "attribution_epoch_integrity_failed"
        );
        let error = crate::performance::attribution::persist_epoch_daily(&manager_b, &daily)
            .expect_err("TEST_CODE replacement B must reject detached A evidence");
        assert_eq!(error.reason_code(), "attribution_epoch_integrity_failed");
        assert_daily_storage_pristine(&manager_b);

        drop(detached_checkout);
        drop(manager_b);
        drop(database_a);
        for path in [
            replacement_copy,
            detached_main.clone(),
            sqlite_sidecar_path(&detached_main, "-wal"),
            sqlite_sidecar_path(&detached_main, "-shm"),
        ] {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn daily_persist_reuses_identical_payload_appends_revision_and_preserves_legacy_rows() {
        let database = TestDatabase::attested();
        install_activation_source(&database.manager);
        let mut conn = database.manager.get_conn().unwrap();
        conn.batch_execute(
            "CREATE TABLE paper_attribution_daily(date TEXT PRIMARY KEY, marker TEXT NOT NULL);
             INSERT INTO paper_attribution_daily(date,marker)
             VALUES ('2026-08-28','TEST_CODE legacy bytes');",
        )
        .unwrap();
        drop(conn);
        AttributionEpochStore::new(&database.manager)
            .activate_once(activation_request("2026-08-28T15:40:00+08:00"))
            .expect("TEST_CODE active epoch");
        append_test_fills(
            &database.manager,
            &[
                (
                    3,
                    "TEST_CODE_PLAN_CARRY_EXIT",
                    "sell",
                    100,
                    "2026-08-31 02:00:00",
                ),
                (
                    4,
                    "TEST_CODE_PLAN_OPEN_BUY",
                    "buy",
                    100,
                    "2026-08-31 03:00:00",
                ),
            ],
        );

        let mut first_prices = HashMap::new();
        first_prices.insert("TEST_CODE_600001".to_owned(), 11.0);
        let first_daily = crate::performance::attribution::compute_epoch_daily(
            &database.manager,
            NaiveDate::from_ymd_opt(2026, 8, 31).unwrap(),
            &first_prices,
        )
        .unwrap();
        let first =
            crate::performance::attribution::persist_epoch_daily(&database.manager, &first_daily)
                .unwrap();
        let reused =
            crate::performance::attribution::persist_epoch_daily(&database.manager, &first_daily)
                .unwrap();
        assert_eq!(first, reused);
        assert_eq!(first.receipts.len(), 1);
        assert_eq!(first.receipts[0].revision, 1);

        let mut revised_prices = HashMap::new();
        revised_prices.insert("TEST_CODE_600001".to_owned(), 12.0);
        let revised_daily = crate::performance::attribution::compute_epoch_daily(
            &database.manager,
            NaiveDate::from_ymd_opt(2026, 8, 31).unwrap(),
            &revised_prices,
        )
        .unwrap();
        let revised =
            crate::performance::attribution::persist_epoch_daily(&database.manager, &revised_daily)
                .unwrap();
        assert_eq!(revised.receipts.len(), 1);
        assert_eq!(revised.receipts[0].revision, 2);
        assert_ne!(
            revised.receipts[0].epoch_daily_id,
            first.receipts[0].epoch_daily_id
        );

        let mut conn = database.manager.get_conn().unwrap();
        assert_eq!(validate_daily(&mut conn).unwrap().len(), 2);
        let legacy_count =
            diesel::sql_query("SELECT COUNT(*) AS count FROM paper_attribution_daily")
                .get_result::<CountRow>(&mut conn)
                .unwrap()
                .count;
        let marker = diesel::sql_query(
            "SELECT marker AS file FROM paper_attribution_daily WHERE date='2026-08-28'",
        )
        .get_result::<DatabaseFileRow>(&mut conn)
        .unwrap()
        .file;
        assert_eq!(legacy_count, 1);
        assert_eq!(marker, "TEST_CODE legacy bytes");
    }

    struct TestDatabaseCleanup {
        path: PathBuf,
    }

    impl Drop for TestDatabaseCleanup {
        fn drop(&mut self) {
            if self.path == PathBuf::from(":memory:") {
                return;
            }
            for suffix in ["", "-wal", "-shm"] {
                let candidate = PathBuf::from(format!("{}{}", self.path.display(), suffix));
                if let Err(error) = std::fs::remove_file(&candidate) {
                    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
                }
            }
        }
    }

    struct TestDatabase {
        // Fields drop in declaration order. Close the pool before unlinking its
        // WAL files so fixture cleanup cannot race live SQLite handles.
        manager: DatabaseManager,
        path: PathBuf,
        _cleanup: TestDatabaseCleanup,
    }

    impl TestDatabase {
        fn new() -> Self {
            Self::with_options(1, true)
        }

        fn attested() -> Self {
            Self::with_wal_options(1, true)
        }

        fn with_options(pool_size: u32, install_epoch_schema: bool) -> Self {
            Self::with_journal_options(pool_size, install_epoch_schema, false)
        }

        fn with_wal_options(pool_size: u32, install_epoch_schema: bool) -> Self {
            Self::with_journal_options(pool_size, install_epoch_schema, true)
        }

        fn with_journal_options(
            pool_size: u32,
            install_epoch_schema: bool,
            production_wal: bool,
        ) -> Self {
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
            if production_wal {
                let mut bootstrap = SqliteConnection::establish(&database_url)
                    .expect("TEST_CODE SQLite bootstrap connection");
                diesel::sql_query("PRAGMA busy_timeout = 5000")
                    .execute(&mut bootstrap)
                    .expect("TEST_CODE bootstrap busy timeout");
                let journal_mode = diesel::sql_query("PRAGMA journal_mode = WAL")
                    .get_result::<super::super::JournalModeRow>(&mut bootstrap)
                    .expect("TEST_CODE bootstrap WAL mode")
                    .journal_mode;
                assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
                super::super::configure_sqlite_connection(&mut bootstrap)
                    .expect("TEST_CODE bootstrap SQLite configuration");
                drop(bootstrap);
            }
            let (attribution_pool, attribution_connection_source) = if production_wal {
                let (pool, source) =
                    super::super::build_attested_sqlite_pool_with_size(&path, pool_size)
                        .expect("TEST_CODE descriptor-attested attribution pool");
                (Some(pool), Some(source))
            } else {
                (None, None)
            };
            let pool = super::super::build_sqlite_pool_with_size(database_url, pool_size)
                .expect("TEST_CODE isolated operational SQLite pool");
            if install_epoch_schema {
                let mut conn = pool.get().expect("TEST_CODE schema connection");
                create_schema(&mut conn).expect("TEST_CODE epoch schema");
            }
            Self {
                manager: DatabaseManager {
                    pool,
                    attribution_pool,
                    attribution_connection_source,
                    selection_connection_source: None,
                    selection_schema_authority: None,
                },
                path: path.clone(),
                _cleanup: TestDatabaseCleanup { path },
            }
        }

        fn in_memory() -> Self {
            let pool = super::super::build_sqlite_pool_with_size(":memory:".to_owned(), 1)
                .expect("TEST_CODE isolated in-memory SQLite pool");
            {
                let mut conn = pool.get().expect("TEST_CODE in-memory schema connection");
                create_schema(&mut conn).expect("TEST_CODE in-memory epoch schema");
            }
            Self {
                manager: DatabaseManager {
                    pool,
                    attribution_pool: None,
                    attribution_connection_source: None,
                    selection_connection_source: None,
                    selection_schema_authority: None,
                },
                path: PathBuf::from(":memory:"),
                _cleanup: TestDatabaseCleanup {
                    path: PathBuf::from(":memory:"),
                },
            }
        }

        fn snapshot_of(source: &Self) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("TEST_CODE clock")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "TEST_CODE_attribution_epochs_snapshot_{}_{}.sqlite",
                std::process::id(),
                nonce
            ));
            let database_url = path.to_string_lossy().into_owned();
            let mut source_conn = source
                .manager
                .get_conn()
                .expect("TEST_CODE source snapshot");
            diesel::sql_query("VACUUM INTO ?")
                .bind::<Text, _>(&database_url)
                .execute(&mut source_conn)
                .expect("TEST_CODE exact SQLite content snapshot");
            drop(source_conn);
            let mut bootstrap = SqliteConnection::establish(&path.to_string_lossy())
                .expect("TEST_CODE snapshot WAL bootstrap");
            let journal_mode = diesel::sql_query("PRAGMA journal_mode = WAL")
                .get_result::<super::super::JournalModeRow>(&mut bootstrap)
                .expect("TEST_CODE snapshot WAL mode")
                .journal_mode;
            assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
            drop(bootstrap);
            let (attribution_pool, source) =
                super::super::build_attested_sqlite_pool_with_size(&path, 1)
                    .expect("TEST_CODE snapshot descriptor-attested SQLite pool");
            let pool = super::super::build_sqlite_pool_with_size(database_url, 1)
                .expect("TEST_CODE snapshot operational SQLite pool");
            Self {
                manager: DatabaseManager {
                    pool,
                    attribution_pool: Some(attribution_pool),
                    attribution_connection_source: Some(source),
                    selection_connection_source: None,
                    selection_schema_authority: None,
                },
                path: path.clone(),
                _cleanup: TestDatabaseCleanup { path },
            }
        }
    }

    fn install_activation_source(manager: &DatabaseManager) {
        use crate::database::order_audit::{
            canonical_order_audit_record_hash, CanonicalOrderAuditRow, AUDIT_CHAIN_GENESIS,
        };

        let mut conn = manager.get_conn().expect("TEST_CODE source connection");
        conn.batch_execute(
            "CREATE TABLE paper_trades (
                id INTEGER PRIMARY KEY, plan_id TEXT NOT NULL UNIQUE,
                code TEXT NOT NULL, name TEXT NOT NULL, direction TEXT NOT NULL,
                price REAL NOT NULL, quantity INTEGER NOT NULL, status TEXT NOT NULL,
                fill_price REAL, not_fill_reason TEXT, virtual_reason TEXT NOT NULL,
                account_mode TEXT NOT NULL, data_mode TEXT NOT NULL,
                ts TEXT NOT NULL, updated_at TEXT NOT NULL
             );
             CREATE TABLE order_audit (
                id INTEGER PRIMARY KEY, business_order_id TEXT NOT NULL,
                source TEXT NOT NULL, decision_basis TEXT NOT NULL, side TEXT NOT NULL,
                code TEXT NOT NULL, requested_price REAL NOT NULL, execution_price REAL,
                quantity INTEGER NOT NULL, quote_observed_at TEXT, outcome TEXT NOT NULL,
                failure_reason TEXT, created_at TEXT NOT NULL
             );
             CREATE TABLE order_audit_chain (
                order_audit_id INTEGER PRIMARY KEY, previous_hash TEXT NOT NULL,
                record_hash TEXT NOT NULL UNIQUE, created_at TEXT NOT NULL
             );",
        )
        .expect("TEST_CODE source schema");
        let mut previous = AUDIT_CHAIN_GENESIS.to_owned();
        for (id, plan, direction, quantity, timestamp) in [
            (
                1_i64,
                "TEST_CODE_PLAN_BUY",
                "buy",
                200_i64,
                "2026-08-27 02:00:00",
            ),
            (
                2_i64,
                "TEST_CODE_PLAN_SELL",
                "sell",
                100_i64,
                "2026-08-28 02:00:00",
            ),
        ] {
            diesel::sql_query(
                "INSERT INTO paper_trades
                 (id,plan_id,code,name,direction,price,quantity,status,fill_price,not_fill_reason,
                  virtual_reason,account_mode,data_mode,ts,updated_at)
                 VALUES (?,?,'TEST_CODE_600001','TEST_CODE company',?,10.0,?,'Filled',10.0,NULL,
                         'TEST_CODE activation','Normal','Full',?,?)",
            )
            .bind::<BigInt, _>(id)
            .bind::<Text, _>(plan)
            .bind::<Text, _>(direction)
            .bind::<BigInt, _>(quantity)
            .bind::<Text, _>(timestamp)
            .bind::<Text, _>(timestamp)
            .execute(&mut conn)
            .expect("TEST_CODE paper fill");
            let audit = CanonicalOrderAuditRow {
                id,
                business_order_id: plan.to_owned(),
                source: "PaperTrade".to_owned(),
                decision_basis: "TEST_CODE activation".to_owned(),
                side: direction.to_owned(),
                code: "TEST_CODE_600001".to_owned(),
                requested_price: 10.0,
                execution_price: Some(10.0),
                quantity,
                quote_observed_at: Some(format!(
                    "{}T{}+08:00",
                    &timestamp[..10],
                    match &timestamp[11..] {
                        "02:00:00" => "10:00:00",
                        other => other,
                    }
                )),
                outcome: "Filled".to_owned(),
                failure_reason: None,
                created_at: format!("{} 02:00:01", &timestamp[..10]),
            };
            diesel::sql_query(
                "INSERT INTO order_audit
                 (id,business_order_id,source,decision_basis,side,code,requested_price,
                  execution_price,quantity,quote_observed_at,outcome,failure_reason,created_at)
                 VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)",
            )
            .bind::<BigInt, _>(audit.id)
            .bind::<Text, _>(&audit.business_order_id)
            .bind::<Text, _>(&audit.source)
            .bind::<Text, _>(&audit.decision_basis)
            .bind::<Text, _>(&audit.side)
            .bind::<Text, _>(&audit.code)
            .bind::<diesel::sql_types::Double, _>(audit.requested_price)
            .bind::<Nullable<diesel::sql_types::Double>, _>(audit.execution_price)
            .bind::<BigInt, _>(audit.quantity)
            .bind::<Nullable<Text>, _>(&audit.quote_observed_at)
            .bind::<Text, _>(&audit.outcome)
            .bind::<Nullable<Text>, _>(&audit.failure_reason)
            .bind::<Text, _>(&audit.created_at)
            .execute(&mut conn)
            .expect("TEST_CODE audit row");
            let record_hash = canonical_order_audit_record_hash(&previous, &audit)
                .expect("TEST_CODE canonical audit hash");
            diesel::sql_query(
                "INSERT INTO order_audit_chain
                 (order_audit_id,previous_hash,record_hash,created_at) VALUES (?,?,?,?)",
            )
            .bind::<BigInt, _>(id)
            .bind::<Text, _>(&previous)
            .bind::<Text, _>(&record_hash)
            .bind::<Text, _>(&audit.created_at)
            .execute(&mut conn)
            .expect("TEST_CODE audit chain row");
            previous = record_hash;
        }
    }

    fn append_activation_fill(
        manager: &DatabaseManager,
        id: i64,
        plan: &str,
        direction: &str,
        quantity: i64,
        timestamp: &str,
    ) {
        use crate::database::order_audit::{
            canonical_order_audit_record_hash, CanonicalOrderAuditRow,
        };

        let mut conn = manager.get_conn().unwrap();
        let previous = load_order_audit_chain_rows(&mut conn)
            .unwrap()
            .last()
            .map_or(AUDIT_CHAIN_GENESIS.to_owned(), |row| {
                row.record_hash.clone()
            });
        diesel::sql_query(
            "INSERT INTO paper_trades
             (id,plan_id,code,name,direction,price,quantity,status,fill_price,not_fill_reason,
              virtual_reason,account_mode,data_mode,ts,updated_at)
             VALUES (?,?,'TEST_CODE_600001','TEST_CODE company',?,10.0,?,'Filled',10.0,NULL,
                     'TEST_CODE activation','Normal','Full',?,?)",
        )
        .bind::<BigInt, _>(id)
        .bind::<Text, _>(plan)
        .bind::<Text, _>(direction)
        .bind::<BigInt, _>(quantity)
        .bind::<Text, _>(timestamp)
        .bind::<Text, _>(timestamp)
        .execute(&mut conn)
        .unwrap();
        let audit = CanonicalOrderAuditRow {
            id,
            business_order_id: plan.to_owned(),
            source: "PaperTrade".to_owned(),
            decision_basis: "TEST_CODE activation".to_owned(),
            side: direction.to_owned(),
            code: "TEST_CODE_600001".to_owned(),
            requested_price: 10.0,
            execution_price: Some(10.0),
            quantity,
            quote_observed_at: Some(
                chrono::NaiveDateTime::parse_from_str(timestamp, "%Y-%m-%d %H:%M:%S")
                    .unwrap()
                    .and_utc()
                    .with_timezone(&FixedOffset::east_opt(8 * 60 * 60).unwrap())
                    .to_rfc3339(),
            ),
            outcome: "Filled".to_owned(),
            failure_reason: None,
            created_at: (chrono::NaiveDateTime::parse_from_str(timestamp, "%Y-%m-%d %H:%M:%S")
                .unwrap()
                + chrono::Duration::seconds(1))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string(),
        };
        diesel::sql_query(
            "INSERT INTO order_audit
             (id,business_order_id,source,decision_basis,side,code,requested_price,
              execution_price,quantity,quote_observed_at,outcome,failure_reason,created_at)
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind::<BigInt, _>(audit.id)
        .bind::<Text, _>(&audit.business_order_id)
        .bind::<Text, _>(&audit.source)
        .bind::<Text, _>(&audit.decision_basis)
        .bind::<Text, _>(&audit.side)
        .bind::<Text, _>(&audit.code)
        .bind::<Double, _>(audit.requested_price)
        .bind::<Nullable<Double>, _>(audit.execution_price)
        .bind::<BigInt, _>(audit.quantity)
        .bind::<Nullable<Text>, _>(&audit.quote_observed_at)
        .bind::<Text, _>(&audit.outcome)
        .bind::<Nullable<Text>, _>(&audit.failure_reason)
        .bind::<Text, _>(&audit.created_at)
        .execute(&mut conn)
        .unwrap();
        let record_hash = canonical_order_audit_record_hash(&previous, &audit).unwrap();
        diesel::sql_query(
            "INSERT INTO order_audit_chain
             (order_audit_id,previous_hash,record_hash,created_at) VALUES (?,?,?,?)",
        )
        .bind::<BigInt, _>(id)
        .bind::<Text, _>(&previous)
        .bind::<Text, _>(&record_hash)
        .bind::<Text, _>(&audit.created_at)
        .execute(&mut conn)
        .unwrap();
    }

    fn append_non_filled_paper(manager: &DatabaseManager, id: i64, status: &str) {
        let mut conn = manager.get_conn().unwrap();
        diesel::sql_query(
            "INSERT INTO paper_trades
             (id,plan_id,code,name,direction,price,quantity,status,fill_price,not_fill_reason,
              virtual_reason,account_mode,data_mode,ts,updated_at)
             VALUES (?,?,'TEST_CODE_600001','TEST_CODE company',
                     'buy',10.0,100,?,NULL,'TEST_CODE rejected','TEST_CODE late status',
                     'Normal','Full','2026-08-31 04:00:00','2026-08-31 04:00:01')",
        )
        .bind::<BigInt, _>(id)
        .bind::<Text, _>(format!("TEST_CODE_PLAN_NON_FILLED_{id}"))
        .bind::<Text, _>(status)
        .execute(&mut conn)
        .unwrap();
    }

    fn rewrite_test_audit_chain(manager: &DatabaseManager) {
        use crate::database::order_audit::canonical_order_audit_record_hash;

        let mut conn = manager.get_conn().unwrap();
        let audits = load_order_audit_rows(&mut conn).unwrap();
        let mut previous = AUDIT_CHAIN_GENESIS.to_owned();
        for audit in audits {
            let record_hash = canonical_order_audit_record_hash(&previous, &audit).unwrap();
            diesel::sql_query(
                "UPDATE order_audit_chain SET previous_hash=?,record_hash=?
                 WHERE order_audit_id=?",
            )
            .bind::<Text, _>(&previous)
            .bind::<Text, _>(&record_hash)
            .bind::<BigInt, _>(audit.id)
            .execute(&mut conn)
            .unwrap();
            previous = record_hash;
        }
    }

    fn append_duplicate_terminal(manager: &DatabaseManager) {
        use crate::database::order_audit::{
            canonical_order_audit_record_hash, CanonicalOrderAuditRow,
        };

        let mut conn = manager.get_conn().unwrap();
        let previous = load_order_audit_chain_rows(&mut conn)
            .unwrap()
            .last()
            .unwrap()
            .record_hash
            .clone();
        let audit = CanonicalOrderAuditRow {
            id: 3,
            business_order_id: "TEST_CODE_PLAN_BUY".to_owned(),
            source: "PaperTrade".to_owned(),
            decision_basis: "TEST_CODE activation".to_owned(),
            side: "buy".to_owned(),
            code: "TEST_CODE_600001".to_owned(),
            requested_price: 10.0,
            execution_price: Some(10.0),
            quantity: 200,
            quote_observed_at: Some("2026-08-27T10:00:00+08:00".to_owned()),
            outcome: "Filled".to_owned(),
            failure_reason: None,
            created_at: "2026-08-27 02:00:01".to_owned(),
        };
        diesel::sql_query(
            "INSERT INTO order_audit
             (id,business_order_id,source,decision_basis,side,code,requested_price,
              execution_price,quantity,quote_observed_at,outcome,failure_reason,created_at)
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind::<BigInt, _>(audit.id)
        .bind::<Text, _>(&audit.business_order_id)
        .bind::<Text, _>(&audit.source)
        .bind::<Text, _>(&audit.decision_basis)
        .bind::<Text, _>(&audit.side)
        .bind::<Text, _>(&audit.code)
        .bind::<Double, _>(audit.requested_price)
        .bind::<Nullable<Double>, _>(audit.execution_price)
        .bind::<BigInt, _>(audit.quantity)
        .bind::<Nullable<Text>, _>(&audit.quote_observed_at)
        .bind::<Text, _>(&audit.outcome)
        .bind::<Nullable<Text>, _>(&audit.failure_reason)
        .bind::<Text, _>(&audit.created_at)
        .execute(&mut conn)
        .unwrap();
        let record_hash = canonical_order_audit_record_hash(&previous, &audit).unwrap();
        diesel::sql_query(
            "INSERT INTO order_audit_chain
             (order_audit_id,previous_hash,record_hash,created_at) VALUES (?,?,?,?)",
        )
        .bind::<BigInt, _>(audit.id)
        .bind::<Text, _>(&previous)
        .bind::<Text, _>(&record_hash)
        .bind::<Text, _>(&audit.created_at)
        .execute(&mut conn)
        .unwrap();
    }

    fn append_semantically_mismatched_duplicate_terminal(manager: &DatabaseManager) {
        append_duplicate_terminal(manager);
        let mut conn = manager.get_conn().unwrap();
        diesel::sql_query(
            "UPDATE order_audit
             SET decision_basis='TEST_CODE mismatched decision',
                 failure_reason='TEST_CODE mismatched failure',
                 quote_observed_at='2026-08-27T10:00:09+08:00',
                 created_at='2026-08-27 02:00:10'
             WHERE id=3",
        )
        .execute(&mut conn)
        .unwrap();
        drop(conn);
        rewrite_test_audit_chain(manager);
    }

    fn activation_error_detail(error: AttributionEpochStoreError) -> String {
        match error {
            AttributionEpochStoreError::FailedIntegrity { detail, .. }
            | AttributionEpochStoreError::Unavailable { detail, .. } => detail,
        }
    }

    #[test]
    fn activation_preview_and_commit_freeze_one_verified_epoch() {
        let database = TestDatabase::new();
        install_activation_source(&database.manager);
        let store = AttributionEpochStore::with_read_only_preview_path(
            &database.manager,
            database.path.clone(),
        );
        let request = activation_request("2026-08-28T15:40:00+08:00");

        let preview = store
            .preview_activation(&request)
            .expect("TEST_CODE activation preview");
        assert!(!preview.activated);
        assert_eq!(preview.receipt_hash, None);
        assert_eq!(preview.calendar_authority_hash.len(), 64);
        assert_eq!(preview.completed_session_date.to_string(), "2026-08-28");
        assert_eq!(preview.effective_date.to_string(), "2026-08-31");
        assert_eq!(preview.paper_trade_high_water, 2);
        assert_eq!(preview.order_audit_high_water, 2);
        assert_eq!(preview.carry[0].code, "TEST_CODE_600001");
        assert_eq!(preview.carry[0].quantity, 100);

        let outcome = store
            .activate_once_with_outcome(request.clone())
            .expect("TEST_CODE atomic activation with verified render projection");
        let receipt = outcome.receipt();
        let committed = outcome.projection();
        assert_eq!(receipt.epoch_id, preview.epoch_id);
        assert_eq!(receipt.paper_trade_high_water, 2);
        assert!(committed.activated);
        assert_eq!(committed.epoch_id, receipt.epoch_id);
        assert_eq!(
            committed.calendar_authority_hash,
            receipt.calendar_authority_hash
        );
        assert_eq!(
            committed.receipt_hash.as_deref(),
            Some(receipt.receipt_hash.as_str())
        );
        assert_eq!(&store.verify_active().unwrap(), receipt);

        let retained = store
            .preview_activation(&request)
            .expect("TEST_CODE retained activation preview");
        assert!(retained.activated);
        assert_eq!(
            retained.calendar_authority_hash,
            receipt.calendar_authority_hash
        );
        assert_eq!(
            retained.receipt_hash.as_deref(),
            Some(receipt.receipt_hash.as_str())
        );

        let retried = store
            .activate_once_with_outcome(request)
            .expect("TEST_CODE idempotent rich activation retry");
        assert_eq!(retried.receipt(), receipt);
        assert_eq!(retried.projection(), committed);
    }

    #[test]
    fn exact_loader_fully_validates_on_callers_transaction_connection() {
        let database = TestDatabase::new();
        install_activation_source(&database.manager);
        let receipt = AttributionEpochStore::new(&database.manager)
            .activate_once(activation_request("2026-08-28T15:40:00+08:00"))
            .expect("TEST_CODE activate exact-loader epoch");
        let mut conn = database
            .manager
            .get_conn()
            .expect("TEST_CODE caller-owned transaction connection");

        conn.immediate_transaction::<_, diesel::result::Error, _>(|conn| {
            let loaded = load_exact_on_connection(conn, &receipt.epoch_id)
                .expect("TEST_CODE exact loader inside existing write transaction");
            assert_eq!(loaded, receipt);

            diesel::sql_query("DROP TRIGGER trg_attribution_sample_epoch_receipt_no_update")
                .execute(conn)?;
            let error = load_exact_on_connection(conn, &receipt.epoch_id)
                .expect_err("TEST_CODE same-connection trigger drift must fail full validation");
            assert!(matches!(
                error,
                AttributionEpochStoreError::FailedIntegrity { .. }
            ));
            Ok(())
        })
        .expect("TEST_CODE caller transaction remains usable");
    }

    #[test]
    fn activation_retry_keeps_frozen_high_water_and_loader_rejects_late_rows() {
        let database = TestDatabase::new();
        install_activation_source(&database.manager);
        let store = AttributionEpochStore::new(&database.manager);
        let receipt = store
            .activate_once(activation_request("2026-08-28T15:40:00+08:00"))
            .unwrap();
        append_activation_fill(
            &database.manager,
            3,
            "TEST_CODE_PLAN_NEW",
            "buy",
            100,
            "2026-08-31 02:00:00",
        );
        let retried = store
            .activate_once(activation_request("2026-09-01T15:40:00+08:00"))
            .unwrap();
        assert_eq!(retried, receipt);
        assert_eq!(retried.paper_trade_high_water, 2);
        let mut conn = database.manager.get_conn().unwrap();
        let verified = load_verified_epoch_fills_until(
            &mut conn,
            &ResolvedAttributionEpoch::Epoch(receipt.clone()),
            NaiveDate::from_ymd_opt(2026, 8, 31).unwrap(),
        )
        .unwrap();
        assert_eq!(verified.fills.len(), 1);
        assert_eq!(verified.fills[0].fill.id, 3);
        drop(conn);

        append_activation_fill(
            &database.manager,
            4,
            "TEST_CODE_PLAN_LATE",
            "buy",
            100,
            "2026-08-28 07:45:00",
        );
        let mut conn = database.manager.get_conn().unwrap();
        let error = load_verified_epoch_fills_until(
            &mut conn,
            &ResolvedAttributionEpoch::Epoch(receipt),
            NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(),
        )
        .unwrap_err();
        assert_eq!(error.reason_code(), "attribution_epoch_integrity_failed");
    }

    #[test]
    fn verified_epoch_fill_set_exposes_fully_validated_retained_carry() {
        let database = TestDatabase::new();
        install_activation_source(&database.manager);
        let receipt = AttributionEpochStore::new(&database.manager)
            .activate_once(activation_request("2026-08-28T15:40:00+08:00"))
            .unwrap();
        append_activation_fill(
            &database.manager,
            3,
            "TEST_CODE_PLAN_AFTER_EPOCH",
            "buy",
            100,
            "2026-08-31 02:00:00",
        );

        let mut conn = database.manager.get_conn().unwrap();
        let verified = load_verified_epoch_fills_until(
            &mut conn,
            &ResolvedAttributionEpoch::Epoch(receipt),
            NaiveDate::from_ymd_opt(2026, 8, 31).unwrap(),
        )
        .unwrap();

        assert_eq!(
            verified.carry(),
            &[LegacyCarryPosition {
                code: "TEST_CODE_600001".to_owned(),
                quantity: 100,
            }]
        );
        assert_eq!(verified.fills().len(), 1);
    }

    #[test]
    fn activation_retry_revalidates_every_invocation_before_idempotent_success() {
        let database = TestDatabase::new();
        install_activation_source(&database.manager);
        let store = AttributionEpochStore::new(&database.manager);
        let receipt = store
            .activate_once(activation_request("2026-08-28T15:40:00+08:00"))
            .unwrap();

        for (raw, expected) in [
            (
                "2026-08-28T15:40:00+07:00",
                "attribution_epoch_invalid_timezone",
            ),
            (
                "2026-08-29T15:40:00+08:00",
                "attribution_epoch_non_trading_day",
            ),
            (
                "2026-08-28T15:34:59+08:00",
                "attribution_epoch_window_not_open",
            ),
            (
                "2026-08-28T15:50:01+08:00",
                "attribution_epoch_window_closed",
            ),
            (
                "2027-01-04T15:40:00+08:00",
                "attribution_epoch_calendar_coverage_unavailable",
            ),
        ] {
            let error = store.activate_once(activation_request(raw)).unwrap_err();
            assert_eq!(error.reason_code(), expected, "TEST_CODE {raw}");
        }

        let mut conn = database.manager.get_conn().unwrap();
        assert_eq!(
            receipt_value(&validate_all(&mut conn).unwrap()[0]).unwrap(),
            receipt
        );
        let attempts = validate_attempts(&mut conn).unwrap();
        assert_eq!(
            attempts
                .iter()
                .filter(|attempt| attempt.outcome == "success")
                .count(),
            1
        );
        assert_eq!(attempts.len(), 6);
    }

    #[test]
    fn activation_high_water_includes_valid_non_filled_paper_rows() {
        let database = TestDatabase::new();
        install_activation_source(&database.manager);
        let mut conn = database.manager.get_conn().unwrap();
        diesel::sql_query(
            "INSERT INTO paper_trades
             (id,plan_id,code,name,direction,price,quantity,status,fill_price,not_fill_reason,
              virtual_reason,account_mode,data_mode,ts,updated_at)
             VALUES (3,'TEST_CODE_NOT_FILLED','TEST_CODE_600001','TEST_CODE company',
                     'buy',10.0,100,'NotFilled',NULL,'TEST_CODE limit up',
                     'TEST_CODE activation','ReduceOnly','Degraded',
                     '2026-08-28 03:00:00','2026-08-28 03:00:01')",
        )
        .execute(&mut conn)
        .unwrap();
        drop(conn);

        let receipt = AttributionEpochStore::new(&database.manager)
            .activate_once(activation_request("2026-08-28T15:40:00+08:00"))
            .unwrap();
        assert_eq!(receipt.paper_trade_high_water, 3);
        assert_eq!(receipt.carry_total_quantity, 100);
    }

    #[test]
    fn activation_window_failure_is_audited_without_a_success_receipt() {
        let database = TestDatabase::new();
        install_activation_source(&database.manager);
        let error = AttributionEpochStore::new(&database.manager)
            .activate_once(activation_request("2026-08-28T15:34:59+08:00"))
            .unwrap_err();
        assert_eq!(error.reason_code(), "attribution_epoch_window_not_open");
        assert!(error.retryable());
        let mut conn = database.manager.get_conn().unwrap();
        assert_eq!(load_receipts(&mut conn).unwrap().len(), 0);
        let attempts = validate_attempts(&mut conn).unwrap();
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].outcome, "unavailable");
        assert_eq!(attempts[0].reason_code, "attribution_epoch_window_not_open");
    }

    #[test]
    fn activation_preview_does_not_install_absent_epoch_storage_or_change_file_bytes() {
        let database = TestDatabase::with_options(1, false);
        install_activation_source(&database.manager);
        let snapshot = |suffix: &str| {
            let path = PathBuf::from(format!("{}{}", database.path.display(), suffix));
            std::fs::read(&path).ok().map(|bytes| {
                let modified = std::fs::metadata(path).unwrap().modified().unwrap();
                (bytes, modified)
            })
        };
        let before = [snapshot(""), snapshot("-wal"), snapshot("-shm")];
        let held_pool_connection = database.manager.get_conn().unwrap();
        let preview = AttributionEpochStore::with_read_only_preview_path(
            &database.manager,
            database.path.clone(),
        )
        .preview_activation(&activation_request("2026-08-28T15:40:00+08:00"))
        .unwrap();
        assert!(!preview.activated);
        drop(held_pool_connection);
        assert_eq!(
            [snapshot(""), snapshot("-wal"), snapshot("-shm")],
            before,
            "TEST_CODE preview changed DB/WAL/SHM bytes, existence, or mtime"
        );
        let mut conn = database.manager.get_conn().unwrap();
        let tables = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM sqlite_master
             WHERE name LIKE 'attribution_%' OR name LIKE 'paper_attribution_epoch_%'",
        )
        .get_result::<CountRow>(&mut conn)
        .unwrap()
        .count;
        assert_eq!(tables, 0);
    }

    #[test]
    fn activation_preview_fails_before_reading_a_live_wal_and_preserves_sidecars() {
        let database = TestDatabase::with_options(1, false);
        install_activation_source(&database.manager);
        let mut writer = database.manager.get_conn().unwrap();
        writer
            .batch_execute(
                "PRAGMA journal_mode=WAL;
                 PRAGMA wal_autocheckpoint=0;
                 CREATE TABLE TEST_CODE_live_wal_fact(id INTEGER PRIMARY KEY);
                 INSERT INTO TEST_CODE_live_wal_fact(id) VALUES (1);",
            )
            .unwrap();
        let snapshot = |suffix: &str| {
            let path = PathBuf::from(format!("{}{}", database.path.display(), suffix));
            std::fs::read(&path).ok().map(|bytes| {
                let modified = std::fs::metadata(path).unwrap().modified().unwrap();
                (bytes, modified)
            })
        };
        let before = [snapshot(""), snapshot("-wal"), snapshot("-shm")];
        assert!(before[1].is_some(), "TEST_CODE WAL sidecar must be live");
        assert!(before[2].is_some(), "TEST_CODE SHM sidecar must be live");

        let error = AttributionEpochStore::with_read_only_preview_path(
            &database.manager,
            database.path.clone(),
        )
        .preview_activation(&activation_request("2026-08-28T15:40:00+08:00"))
        .unwrap_err();
        assert_eq!(error.reason_code(), "attribution_epoch_preview_live_wal");
        assert_eq!([snapshot(""), snapshot("-wal"), snapshot("-shm")], before);
        drop(writer);
    }

    #[test]
    fn activation_preview_reads_a_clean_wal_mode_header_without_creating_sidecars() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "TEST_CODE_clean_wal_preview_{}_{}.sqlite",
            std::process::id(),
            nonce
        ));
        let mut source = SqliteConnection::establish(path.to_str().unwrap()).unwrap();
        source
            .batch_execute(
                "PRAGMA journal_mode=WAL;
                 CREATE TABLE paper_trades (
                    id INTEGER PRIMARY KEY, plan_id TEXT NOT NULL UNIQUE,
                    code TEXT NOT NULL, name TEXT NOT NULL, direction TEXT NOT NULL,
                    price REAL NOT NULL, quantity INTEGER NOT NULL, status TEXT NOT NULL,
                    fill_price REAL, not_fill_reason TEXT, virtual_reason TEXT NOT NULL,
                    account_mode TEXT NOT NULL, data_mode TEXT NOT NULL,
                    ts TEXT NOT NULL, updated_at TEXT NOT NULL
                 );
                 CREATE TABLE order_audit (
                    id INTEGER PRIMARY KEY, business_order_id TEXT NOT NULL,
                    source TEXT NOT NULL, decision_basis TEXT NOT NULL, side TEXT NOT NULL,
                    code TEXT NOT NULL, requested_price REAL NOT NULL, execution_price REAL,
                    quantity INTEGER NOT NULL, quote_observed_at TEXT, outcome TEXT NOT NULL,
                    failure_reason TEXT, created_at TEXT NOT NULL
                 );
                 CREATE TABLE order_audit_chain (
                    order_audit_id INTEGER PRIMARY KEY, previous_hash TEXT NOT NULL,
                    record_hash TEXT NOT NULL UNIQUE, created_at TEXT NOT NULL
                 );
                 PRAGMA wal_checkpoint(TRUNCATE);",
            )
            .unwrap();
        drop(source);
        for sidecar in [
            sqlite_sidecar_path(&path, "-wal"),
            sqlite_sidecar_path(&path, "-shm"),
        ] {
            if let Err(error) = std::fs::remove_file(&sidecar) {
                assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
            }
        }
        assert!(!sqlite_sidecar_path(&path, "-wal").exists());
        assert!(!sqlite_sidecar_path(&path, "-shm").exists());
        let main_bytes = std::fs::read(&path).unwrap();
        assert_eq!(main_bytes[18], 2, "TEST_CODE header read version");
        assert_eq!(main_bytes[19], 2, "TEST_CODE header write version");
        let before = preview_sqlite_snapshot(&path).unwrap();

        let authority = TestDatabase::in_memory();
        let preview =
            AttributionEpochStore::with_read_only_preview_path(&authority.manager, path.clone())
                .preview_activation(&activation_request("2026-08-28T15:40:00+08:00"))
                .unwrap();
        assert!(!preview.activated);
        assert_eq!(preview.paper_trade_high_water, 0);
        assert_eq!(preview.order_audit_high_water, 0);
        assert_eq!(preview_sqlite_snapshot(&path).unwrap(), before);
        std::fs::remove_file(path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn path_only_activation_preview_rejects_symlink_and_non_regular_targets() {
        let database = TestDatabase::with_options(1, false);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("TEST_CODE clock")
            .as_nanos();
        let link = std::env::temp_dir().join(format!(
            "TEST_CODE_attribution_epoch_preview_symlink_{}_{}",
            std::process::id(),
            nonce
        ));
        std::os::unix::fs::symlink(&database.path, &link)
            .expect("TEST_CODE create exact preview symlink");

        let symlink_error = AttributionEpochStore::preview_activation_at_path(
            &link,
            &activation_request("2026-08-28T15:40:00+08:00"),
        )
        .expect_err("TEST_CODE path-only preview rejects symlink leaf");
        assert_eq!(
            symlink_error.reason_code(),
            "attribution_epoch_integrity_failed"
        );

        let directory_error = AttributionEpochStore::preview_activation_at_path(
            std::env::temp_dir(),
            &activation_request("2026-08-28T15:40:00+08:00"),
        )
        .expect_err("TEST_CODE path-only preview rejects directory");
        assert_eq!(
            directory_error.reason_code(),
            "attribution_epoch_integrity_failed"
        );

        std::fs::remove_file(link).expect("TEST_CODE remove exact preview symlink");
    }

    #[test]
    fn activation_prefix_drift_and_untrustworthy_attempt_store_fail_closed() {
        let database = TestDatabase::new();
        install_activation_source(&database.manager);
        let store = AttributionEpochStore::new(&database.manager);
        let receipt = store
            .activate_once(activation_request("2026-08-28T15:40:00+08:00"))
            .unwrap();
        let mut conn = database.manager.get_conn().unwrap();
        diesel::sql_query("UPDATE paper_trades SET fill_price=11.0 WHERE id=1")
            .execute(&mut conn)
            .unwrap();
        assert_eq!(
            verify_epoch_source_prefix(&mut conn, &receipt)
                .unwrap_err()
                .reason_code(),
            "attribution_epoch_integrity_failed"
        );
        drop(conn);

        let unavailable_audit = TestDatabase::new();
        let mut conn = unavailable_audit.manager.get_conn().unwrap();
        diesel::sql_query("DROP TRIGGER trg_attribution_epoch_attempt_audit_no_update")
            .execute(&mut conn)
            .unwrap();
        drop(conn);
        let error = AttributionEpochStore::new(&unavailable_audit.manager)
            .activate_once(activation_request("2026-08-28T15:34:59+08:00"))
            .unwrap_err();
        assert_eq!(error.reason_code(), "epoch_attempt_audit_unavailable");
    }

    #[test]
    fn activation_concurrent_callers_share_one_receipt() {
        let database = TestDatabase::with_wal_options(4, true);
        install_activation_source(&database.manager);
        let mut conn = database.manager.get_conn().unwrap();
        let journal_mode = diesel::sql_query("PRAGMA journal_mode")
            .get_result::<super::super::JournalModeRow>(&mut conn)
            .unwrap()
            .journal_mode;
        assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
        drop(conn);
        let barrier = std::sync::Barrier::new(2);
        let receipts = std::thread::scope(|scope| {
            let handles = (0..2)
                .map(|_| {
                    scope.spawn(|| {
                        barrier.wait();
                        AttributionEpochStore::new(&database.manager)
                            .activate_once(activation_request("2026-08-28T15:40:00+08:00"))
                            .unwrap()
                    })
                })
                .collect::<Vec<_>>();
            handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert_eq!(receipts[0], receipts[1]);
        let mut conn = database.manager.get_conn().unwrap();
        assert_eq!(validate_all(&mut conn).unwrap().len(), 1);
        assert_eq!(validate_attempts(&mut conn).unwrap().len(), 2);
    }

    #[test]
    fn activation_rejects_wrong_timezone_closed_dates_and_window_edges() {
        for (raw, expected, retryable) in [
            (
                "2026-08-28T15:40:00+07:00",
                "attribution_epoch_invalid_timezone",
                false,
            ),
            (
                "2026-08-29T15:40:00+08:00",
                "attribution_epoch_non_trading_day",
                false,
            ),
            (
                "2026-08-28T15:34:59+08:00",
                "attribution_epoch_window_not_open",
                true,
            ),
            (
                "2026-08-28T15:50:01+08:00",
                "attribution_epoch_window_closed",
                false,
            ),
            (
                "2027-01-04T15:40:00+08:00",
                "attribution_epoch_calendar_coverage_unavailable",
                false,
            ),
        ] {
            let database = TestDatabase::new();
            install_activation_source(&database.manager);
            let error = AttributionEpochStore::new(&database.manager)
                .activate_once(activation_request(raw))
                .unwrap_err();
            assert_eq!(error.reason_code(), expected, "TEST_CODE {raw}");
            assert_eq!(error.retryable(), retryable, "TEST_CODE {raw}");
            let mut conn = database.manager.get_conn().unwrap();
            assert!(load_receipts(&mut conn).unwrap().is_empty());
            assert_eq!(validate_attempts(&mut conn).unwrap().len(), 1);
        }
    }

    #[test]
    fn activation_safe_window_is_inclusive_at_both_seconds() {
        for raw in ["2026-08-28T15:35:00+08:00", "2026-08-28T15:50:00+08:00"] {
            let database = TestDatabase::new();
            install_activation_source(&database.manager);
            let receipt = AttributionEpochStore::new(&database.manager)
                .activate_once(activation_request(raw))
                .unwrap();
            assert_eq!(
                receipt.cutover_completed_trading_date.to_string(),
                "2026-08-28"
            );
        }
    }

    #[test]
    fn activation_source_failure_rolls_back_receipt_then_appends_failure_attempt() {
        let database = TestDatabase::new();
        install_activation_source(&database.manager);
        let mut conn = database.manager.get_conn().unwrap();
        diesel::sql_query(
            "INSERT INTO paper_trades
             (id,plan_id,code,name,direction,price,quantity,status,fill_price,not_fill_reason,
              virtual_reason,account_mode,data_mode,ts,updated_at)
             VALUES (3,'TEST_CODE_MISSING_TERMINAL','TEST_CODE_600001','TEST_CODE company',
                     'buy',10.0,100,'Filled',10.0,NULL,'TEST_CODE missing terminal',
                     'Normal','Full','2026-08-28 03:00:00','2026-08-28 03:00:00')",
        )
        .execute(&mut conn)
        .unwrap();
        drop(conn);
        let error = AttributionEpochStore::new(&database.manager)
            .activate_once(activation_request("2026-08-28T15:40:00+08:00"))
            .unwrap_err();
        assert_eq!(error.reason_code(), "attribution_epoch_integrity_failed");
        let mut conn = database.manager.get_conn().unwrap();
        assert!(load_receipts(&mut conn).unwrap().is_empty());
        let attempts = validate_attempts(&mut conn).unwrap();
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].outcome, "failed_integrity");
    }

    #[test]
    fn activation_rejects_every_malformed_paper_row_before_filled_projection() {
        for (label, mutation, expected_detail) in [
            (
                "unknown status",
                "UPDATE paper_trades SET status='TEST_CODE_UNKNOWN' WHERE id=1",
                "status",
            ),
            (
                "non-terminal status",
                "UPDATE paper_trades SET status='SignalTriggered' WHERE id=1",
                "status",
            ),
            (
                "direction",
                "UPDATE paper_trades SET direction='BUY' WHERE id=1",
                "direction",
            ),
            (
                "requested price",
                "UPDATE paper_trades SET price=0.0 WHERE id=1",
                "requested price",
            ),
            (
                "quantity",
                "UPDATE paper_trades SET quantity=50 WHERE id=1",
                "quantity",
            ),
            (
                "filled reason",
                "UPDATE paper_trades SET not_fill_reason='TEST_CODE impossible' WHERE id=1",
                "Filled",
            ),
            (
                "risk mode",
                "UPDATE paper_trades SET account_mode='TEST_CODE_UNKNOWN' WHERE id=1",
                "risk context",
            ),
        ] {
            let database = TestDatabase::new();
            install_activation_source(&database.manager);
            let mut conn = database.manager.get_conn().unwrap();
            diesel::sql_query(mutation).execute(&mut conn).unwrap();
            drop(conn);

            let error = AttributionEpochStore::new(&database.manager)
                .activate_once(activation_request("2026-08-28T15:40:00+08:00"))
                .unwrap_err();
            assert_eq!(
                error.reason_code(),
                "attribution_epoch_integrity_failed",
                "TEST_CODE {label}"
            );
            assert!(
                activation_error_detail(error).contains(expected_detail),
                "TEST_CODE {label} did not name {expected_detail}"
            );
            let mut conn = database.manager.get_conn().unwrap();
            assert!(load_receipts(&mut conn).unwrap().is_empty());
            assert_eq!(validate_attempts(&mut conn).unwrap().len(), 1);
        }
    }

    #[test]
    fn activation_rejects_oversell_duplicate_terminal_and_audit_chain_corruption() {
        for label in ["oversell", "duplicate terminal", "audit chain"] {
            let database = TestDatabase::new();
            install_activation_source(&database.manager);
            match label {
                "oversell" => {
                    let mut conn = database.manager.get_conn().unwrap();
                    diesel::sql_query("UPDATE paper_trades SET quantity=300 WHERE id=2")
                        .execute(&mut conn)
                        .unwrap();
                    diesel::sql_query("UPDATE order_audit SET quantity=300 WHERE id=2")
                        .execute(&mut conn)
                        .unwrap();
                    drop(conn);
                    rewrite_test_audit_chain(&database.manager);
                }
                "duplicate terminal" => append_duplicate_terminal(&database.manager),
                "audit chain" => {
                    let mut conn = database.manager.get_conn().unwrap();
                    diesel::sql_query(
                        "UPDATE order_audit_chain SET record_hash=? WHERE order_audit_id=2",
                    )
                    .bind::<Text, _>("0".repeat(64))
                    .execute(&mut conn)
                    .unwrap();
                }
                _ => unreachable!(),
            }
            let error = AttributionEpochStore::new(&database.manager)
                .activate_once(activation_request("2026-08-28T15:40:00+08:00"))
                .unwrap_err();
            assert_eq!(
                error.reason_code(),
                "attribution_epoch_integrity_failed",
                "TEST_CODE {label}"
            );
            let mut conn = database.manager.get_conn().unwrap();
            assert!(load_receipts(&mut conn).unwrap().is_empty());
            assert_eq!(validate_attempts(&mut conn).unwrap().len(), 1);
        }
    }

    #[test]
    fn mismatched_duplicate_terminal_fails_activation_and_verified_loading() {
        let activation_database = TestDatabase::new();
        install_activation_source(&activation_database.manager);
        append_semantically_mismatched_duplicate_terminal(&activation_database.manager);

        let activation_error = AttributionEpochStore::new(&activation_database.manager)
            .activate_once(activation_request("2026-08-28T15:40:00+08:00"))
            .unwrap_err();
        assert_eq!(
            activation_error.reason_code(),
            "attribution_epoch_integrity_failed"
        );
        assert!(activation_error_detail(activation_error).contains("2 terminal audit candidates"));

        let loading_database = TestDatabase::new();
        install_activation_source(&loading_database.manager);
        let receipt = AttributionEpochStore::new(&loading_database.manager)
            .activate_once(activation_request("2026-08-28T15:40:00+08:00"))
            .unwrap();
        append_semantically_mismatched_duplicate_terminal(&loading_database.manager);
        let mut conn = loading_database.manager.get_conn().unwrap();
        let loading_error = load_verified_epoch_fills_until(
            &mut conn,
            &ResolvedAttributionEpoch::Epoch(receipt),
            NaiveDate::from_ymd_opt(2026, 8, 31).unwrap(),
        )
        .unwrap_err();
        assert_eq!(
            loading_error.reason_code(),
            "attribution_epoch_integrity_failed"
        );
        assert!(activation_error_detail(loading_error).contains("2 terminal audit candidates"));
    }

    #[test]
    fn quote_business_date_mismatch_fails_activation_and_verified_loading() {
        let activation_database = TestDatabase::new();
        install_activation_source(&activation_database.manager);
        let mut conn = activation_database.manager.get_conn().unwrap();
        diesel::sql_query(
            "UPDATE paper_trades
             SET ts='2026-08-27 23:59:59',updated_at='2026-08-27 23:59:59'
             WHERE id=1",
        )
        .execute(&mut conn)
        .unwrap();
        diesel::sql_query(
            "UPDATE order_audit
             SET quote_observed_at='2026-08-28T08:00:00+08:00',
                 created_at='2026-08-28 00:00:00'
             WHERE id=1",
        )
        .execute(&mut conn)
        .unwrap();
        drop(conn);
        rewrite_test_audit_chain(&activation_database.manager);

        let activation_error = AttributionEpochStore::new(&activation_database.manager)
            .activate_once(activation_request("2026-08-28T15:40:00+08:00"))
            .unwrap_err();
        assert_eq!(
            activation_error.reason_code(),
            "attribution_epoch_integrity_failed"
        );
        assert!(activation_error_detail(activation_error).contains("business date"));

        let loading_database = TestDatabase::new();
        install_activation_source(&loading_database.manager);
        let receipt = AttributionEpochStore::new(&loading_database.manager)
            .activate_once(activation_request("2026-08-28T15:40:00+08:00"))
            .unwrap();
        append_activation_fill(
            &loading_database.manager,
            3,
            "TEST_CODE_PLAN_DATE_MISMATCH",
            "buy",
            100,
            "2026-08-31 23:59:59",
        );
        let mut conn = loading_database.manager.get_conn().unwrap();
        let loading_error = load_verified_epoch_fills_until(
            &mut conn,
            &ResolvedAttributionEpoch::Epoch(receipt),
            NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(),
        )
        .unwrap_err();
        assert_eq!(
            loading_error.reason_code(),
            "attribution_epoch_integrity_failed"
        );
        assert!(activation_error_detail(loading_error).contains("business date"));
    }

    #[test]
    fn activation_rejects_terminal_time_mismatch_and_future_quote() {
        for (label, quote, terminal_at, expected_detail) in [
            (
                "stale terminal",
                "2026-08-27T10:00:00+08:00",
                "2026-08-27 02:00:10",
                "stale",
            ),
            (
                "future quote",
                "2026-08-27T10:00:02+08:00",
                "2026-08-27 02:00:01",
                "future",
            ),
            (
                "quote after paper persistence",
                "2026-08-27T10:00:02+08:00",
                "2026-08-27 02:00:03",
                "persistence time",
            ),
        ] {
            let database = TestDatabase::new();
            install_activation_source(&database.manager);
            let mut conn = database.manager.get_conn().unwrap();
            diesel::sql_query("UPDATE order_audit SET quote_observed_at=?,created_at=? WHERE id=1")
                .bind::<Text, _>(quote)
                .bind::<Text, _>(terminal_at)
                .execute(&mut conn)
                .unwrap();
            drop(conn);
            rewrite_test_audit_chain(&database.manager);

            let error = AttributionEpochStore::new(&database.manager)
                .activate_once(activation_request("2026-08-28T15:40:00+08:00"))
                .unwrap_err();
            assert_eq!(error.reason_code(), "attribution_epoch_integrity_failed");
            assert!(
                activation_error_detail(error).contains(expected_detail),
                "TEST_CODE {label} did not name {expected_detail}"
            );
            let mut conn = database.manager.get_conn().unwrap();
            assert!(load_receipts(&mut conn).unwrap().is_empty());
            assert_eq!(validate_attempts(&mut conn).unwrap().len(), 1);
        }
    }

    #[test]
    fn activation_preview_rejects_partial_epoch_schema_without_healing_it() {
        let database = TestDatabase::with_options(1, false);
        install_activation_source(&database.manager);
        let mut conn = database.manager.get_conn().unwrap();
        diesel::sql_query(RECEIPT_TABLE).execute(&mut conn).unwrap();
        drop(conn);
        let error = AttributionEpochStore::with_read_only_preview_path(
            &database.manager,
            database.path.clone(),
        )
        .preview_activation(&activation_request("2026-08-28T15:40:00+08:00"))
        .unwrap_err();
        assert_eq!(error.reason_code(), "attribution_epoch_integrity_failed");
        let mut conn = database.manager.get_conn().unwrap();
        let count = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM sqlite_master
             WHERE type='table' AND name LIKE 'attribution_%'",
        )
        .get_result::<CountRow>(&mut conn)
        .unwrap()
        .count;
        assert_eq!(count, 1);
    }

    #[test]
    fn activation_busy_is_retryable_and_never_claims_success_when_audit_is_locked() {
        let database = TestDatabase::with_options(2, true);
        install_activation_source(&database.manager);
        let mut locker = database.manager.get_conn().unwrap();
        let mut contender = database.manager.get_conn().unwrap();
        contender.batch_execute("PRAGMA busy_timeout=1").unwrap();
        drop(contender);
        locker.batch_execute("BEGIN IMMEDIATE").unwrap();
        let barrier = std::sync::Barrier::new(2);
        let error = std::thread::scope(|scope| {
            let handle = scope.spawn(|| {
                barrier.wait();
                AttributionEpochStore::new(&database.manager)
                    .activate_once(activation_request("2026-08-28T15:40:00+08:00"))
                    .unwrap_err()
            });
            barrier.wait();
            let error = handle.join().unwrap();
            locker.batch_execute("COMMIT").unwrap();
            error
        });
        assert_eq!(error.reason_code(), "epoch_attempt_audit_unavailable");
        assert!(error.retryable());
        let mut conn = database.manager.get_conn().unwrap();
        assert!(load_receipts(&mut conn).unwrap().is_empty());
        assert!(validate_attempts(&mut conn).unwrap().is_empty());
    }

    #[test]
    fn activation_read_back_failure_never_returns_committed_success() {
        let database = TestDatabase::in_memory();
        install_activation_source(&database.manager);
        let error = AttributionEpochStore::new(&database.manager)
            .activate_once(activation_request("2026-08-28T15:40:00+08:00"))
            .unwrap_err();
        assert_eq!(error.reason_code(), "attribution_epoch_integrity_failed");
        let mut conn = database.manager.get_conn().unwrap();
        assert_eq!(validate_all(&mut conn).unwrap().len(), 1);
        let attempts = validate_attempts(&mut conn).unwrap();
        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[0].outcome, "success");
        assert_eq!(attempts[1].outcome, "failed_integrity");
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
    fn fresh_schema_registers_every_empty_autoincrement_sequence() {
        let database = TestDatabase::new();
        let mut conn = database.manager.get_conn().unwrap();
        for (table, _) in TABLES {
            let row_count = diesel::sql_query(format!("SELECT COUNT(*) AS count FROM {table}"))
                .get_result::<CountRow>(&mut conn)
                .unwrap()
                .count;
            assert_eq!(
                row_count, 0,
                "TEST_CODE schema init wrote a fact to {table}"
            );
            let sequence = diesel::sql_query("SELECT seq FROM sqlite_sequence WHERE name=?")
                .bind::<Text, _>(table)
                .get_result::<SequenceRow>(&mut conn)
                .optional()
                .unwrap();
            assert_eq!(sequence.and_then(|row| row.seq), Some(0), "{table}");
        }
        diesel::sql_query(
            "DELETE FROM sqlite_sequence WHERE name='attribution_sample_epoch_receipt'",
        )
        .execute(&mut conn)
        .unwrap();
        drop(conn);
        assert_eq!(
            AttributionEpochStore::new(&database.manager)
                .verify_active()
                .unwrap_err()
                .reason_code(),
            "attribution_epoch_integrity_failed"
        );
    }

    #[test]
    fn coordinated_history_and_sequence_deletion_cannot_look_pristine() {
        let database = TestDatabase::new();
        AttributionEpochStore::new(&database.manager)
            .append_attempt(sample_attempt())
            .unwrap();
        let mut conn = database.manager.get_conn().unwrap();
        for table in [
            "attribution_epoch_attempt_chain",
            "attribution_epoch_attempt_audit",
        ] {
            diesel::sql_query(format!("DROP TRIGGER trg_{table}_no_delete"))
                .execute(&mut conn)
                .unwrap();
            diesel::sql_query(format!("DELETE FROM {table}"))
                .execute(&mut conn)
                .unwrap();
            install_triggers(&mut conn, table).unwrap();
            diesel::sql_query("DELETE FROM sqlite_sequence WHERE name=?")
                .bind::<Text, _>(table)
                .execute(&mut conn)
                .unwrap();
        }
        drop(conn);
        assert_eq!(
            AttributionEpochStore::new(&database.manager)
                .load_selector(&AttributionEpochSelector::Legacy)
                .unwrap_err()
                .reason_code(),
            "attribution_epoch_integrity_failed"
        );
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
    fn every_protected_table_blocks_both_update_and_delete() {
        let database = TestDatabase::new();
        populate_all_protected_tables(&database.manager);
        let mut conn = database.manager.get_conn().unwrap();
        for (table, _) in TABLES {
            assert!(
                diesel::sql_query(format!("UPDATE {table} SET id=id WHERE id=1"))
                    .execute(&mut conn)
                    .is_err(),
                "TEST_CODE {table} UPDATE protection"
            );
            assert!(
                diesel::sql_query(format!("DELETE FROM {table} WHERE id=1"))
                    .execute(&mut conn)
                    .is_err(),
                "TEST_CODE {table} DELETE protection"
            );
        }
    }

    #[test]
    fn legacy_selector_validates_present_epoch_storage_and_extra_triggers_fail_closed() {
        for extra_trigger in [false, true] {
            let database = TestDatabase::new();
            let mut conn = database.manager.get_conn().unwrap();
            if extra_trigger {
                diesel::sql_query(
                    "CREATE TRIGGER TEST_CODE_unexpected_epoch_insert
                     BEFORE INSERT ON attribution_sample_epoch_receipt BEGIN SELECT 1; END",
                )
                .execute(&mut conn)
                .unwrap();
            } else {
                diesel::sql_query("DROP TRIGGER trg_attribution_sample_epoch_receipt_no_delete")
                    .execute(&mut conn)
                    .unwrap();
            }
            drop(conn);
            let error = AttributionEpochStore::new(&database.manager)
                .load_selector(&AttributionEpochSelector::Legacy)
                .expect_err("TEST_CODE Legacy must validate retained epoch storage");
            assert_eq!(error.reason_code(), "attribution_epoch_integrity_failed");
        }
    }

    #[test]
    fn canonical_timestamp_fraction_and_month_end_retention_are_exact() {
        assert!(parse_utc("2026-08-28T08:00:00.0Z", "TEST_CODE timestamp").is_err());
        assert!(parse_utc("2026-08-28T08:00:00.000000000Z", "TEST_CODE timestamp").is_err());
        assert!(parse_utc("2026-08-28T08:00:00.000Z", "TEST_CODE timestamp").is_ok());
        assert!(validate_window(
            "2024-02-29T08:00:00.000Z",
            "2029-02-28T08:00:00.000Z",
            "TEST_CODE leap retention"
        )
        .is_ok());
        assert!(validate_window(
            "2024-02-29T08:00:00.000Z",
            "2029-02-28T07:59:59.999Z",
            "TEST_CODE leap retention"
        )
        .is_err());
        assert!(validate_window(
            "2024-01-31T08:00:00.000Z",
            "2029-01-31T08:00:00.000Z",
            "TEST_CODE month-end retention"
        )
        .is_ok());
    }

    #[test]
    fn success_attempt_requires_exact_receipt_and_non_success_forbids_one() {
        let database = TestDatabase::new();
        let store = AttributionEpochStore::new(&database.manager);
        let mut success_without_receipt = sample_attempt();
        success_without_receipt.outcome = "success".to_owned();
        assert_eq!(
            store
                .append_attempt(success_without_receipt)
                .unwrap_err()
                .reason_code(),
            "attribution_epoch_integrity_failed"
        );

        let receipt_hash = insert_success(&database.manager, None);
        let mut failure_with_receipt = sample_attempt();
        failure_with_receipt.epoch_id = Some("a".repeat(64));
        failure_with_receipt.success_receipt_hash = Some(receipt_hash.clone());
        assert_eq!(
            store
                .append_attempt(failure_with_receipt)
                .unwrap_err()
                .reason_code(),
            "attribution_epoch_integrity_failed"
        );

        let mut exact_success = sample_attempt();
        exact_success.outcome = "success".to_owned();
        exact_success.epoch_id = Some("a".repeat(64));
        exact_success.success_receipt_hash = Some("f".repeat(64));
        assert_eq!(
            store
                .append_attempt(exact_success.clone())
                .unwrap_err()
                .reason_code(),
            "attribution_epoch_integrity_failed"
        );
        exact_success.success_receipt_hash = Some(receipt_hash);
        assert_eq!(
            store
                .append_attempt(exact_success)
                .unwrap()
                .attempt_audit_id,
            1
        );
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
    fn daily_batch_sorts_families_and_atomically_reuses_or_revises_each_family() {
        let database = TestDatabase::new();
        insert_success(&database.manager, None);
        let store = AttributionEpochStore::new(&database.manager);
        let date = NaiveDate::from_ymd_opt(2026, 8, 28).unwrap();

        let first = store
            .append_daily_batch(daily_test_batch(
                date,
                &[("VolumeSurge", 2), ("NewsCatalyst", 1)],
            ))
            .expect("TEST_CODE first canonical daily batch");
        assert_eq!(
            first
                .receipts
                .iter()
                .map(|receipt| (receipt.signal_family.as_str(), receipt.revision))
                .collect::<Vec<_>>(),
            vec![("NewsCatalyst", 1), ("VolumeSurge", 1)]
        );

        let mixed = store
            .append_daily_batch(daily_test_batch(
                date,
                &[("VolumeSurge", 3), ("NewsCatalyst", 1)],
            ))
            .expect("TEST_CODE mixed reuse and revision batch");
        assert_eq!(mixed.receipts[0], first.receipts[0]);
        assert_eq!(mixed.receipts[1].signal_family, "VolumeSurge");
        assert_eq!(mixed.receipts[1].revision, 2);
        assert_ne!(
            mixed.receipts[1].epoch_daily_id,
            first.receipts[1].epoch_daily_id
        );

        let mut conn = database.manager.get_conn().unwrap();
        assert_eq!(validate_daily(&mut conn).unwrap().len(), 3);
        assert_eq!(
            load_chain(
                &mut conn,
                "paper_attribution_epoch_daily_chain",
                "epoch_daily_id"
            )
            .unwrap()
            .len(),
            3
        );
    }

    #[test]
    fn daily_batch_rejects_duplicate_families_without_writes_and_order_is_deterministic() {
        let database = TestDatabase::new();
        insert_success(&database.manager, None);
        let store = AttributionEpochStore::new(&database.manager);
        let date = NaiveDate::from_ymd_opt(2026, 8, 28).unwrap();
        let duplicate = store
            .append_daily_batch(daily_test_batch(
                date,
                &[("NewsCatalyst", 1), ("NewsCatalyst", 2)],
            ))
            .expect_err("TEST_CODE duplicate families invalidate the whole batch");
        assert_eq!(
            duplicate.reason_code(),
            "attribution_epoch_integrity_failed"
        );
        let mut conn = database.manager.get_conn().unwrap();
        assert!(validate_daily(&mut conn).unwrap().is_empty());
        let seq = diesel::sql_query(
            "SELECT seq FROM sqlite_sequence WHERE name='paper_attribution_epoch_daily'",
        )
        .get_result::<SequenceRow>(&mut conn)
        .unwrap()
        .seq;
        assert_eq!(seq, Some(0));
        drop(conn);

        let reversed = store
            .append_daily_batch(daily_test_batch(
                date,
                &[("VolumeSurge", 2), ("NewsCatalyst", 1)],
            ))
            .unwrap();
        let canonical = store
            .append_daily_batch(daily_test_batch(
                date,
                &[("NewsCatalyst", 1), ("VolumeSurge", 2)],
            ))
            .unwrap();
        assert_eq!(reversed, canonical);
    }

    #[test]
    fn daily_batch_mid_write_failure_rolls_back_facts_chains_and_sequences() {
        let database = TestDatabase::new();
        insert_success(&database.manager, None);
        let store = AttributionEpochStore::new(&database.manager);
        let _injection = inject_daily_batch_failure_after(1);

        let error = store
            .append_daily_batch(daily_test_batch(
                NaiveDate::from_ymd_opt(2026, 8, 28).unwrap(),
                &[("NewsCatalyst", 1), ("VolumeSurge", 2)],
            ))
            .expect_err("TEST_CODE injected second-family failure rolls back first family");
        assert_eq!(error.reason_code(), "attribution_epoch_integrity_failed");
        assert_daily_storage_pristine(&database.manager);
    }

    #[test]
    fn daily_append_fails_closed_on_trigger_sequence_retention_payload_and_chain_tamper() {
        for case in ["trigger", "sequence", "retention", "payload", "chain"] {
            let database = TestDatabase::new();
            insert_success(&database.manager, None);
            let store = AttributionEpochStore::new(&database.manager);
            store.append_daily(daily_test_append()).unwrap();
            let mut conn = database.manager.get_conn().unwrap();
            match case {
                "trigger" => {
                    diesel::sql_query("DROP TRIGGER trg_paper_attribution_epoch_daily_no_update")
                        .execute(&mut conn)
                        .unwrap();
                }
                "sequence" => {
                    diesel::sql_query(
                        "UPDATE sqlite_sequence SET seq=0
                         WHERE name='paper_attribution_epoch_daily'",
                    )
                    .execute(&mut conn)
                    .unwrap();
                }
                "retention" => {
                    diesel::sql_query("DROP TRIGGER trg_paper_attribution_epoch_daily_no_update")
                        .execute(&mut conn)
                        .unwrap();
                    diesel::sql_query(
                        "UPDATE paper_attribution_epoch_daily
                         SET retention_deadline=created_at WHERE id=1",
                    )
                    .execute(&mut conn)
                    .unwrap();
                    install_triggers(&mut conn, "paper_attribution_epoch_daily").unwrap();
                }
                "payload" => {
                    diesel::sql_query("DROP TRIGGER trg_paper_attribution_epoch_daily_no_update")
                        .execute(&mut conn)
                        .unwrap();
                    diesel::sql_query(
                        "UPDATE paper_attribution_epoch_daily
                         SET payload_json='{\"closed\":2}' WHERE id=1",
                    )
                    .execute(&mut conn)
                    .unwrap();
                    install_triggers(&mut conn, "paper_attribution_epoch_daily").unwrap();
                }
                "chain" => {
                    diesel::sql_query(
                        "DROP TRIGGER trg_paper_attribution_epoch_daily_chain_no_delete",
                    )
                    .execute(&mut conn)
                    .unwrap();
                    diesel::sql_query("DELETE FROM paper_attribution_epoch_daily_chain")
                        .execute(&mut conn)
                        .unwrap();
                    install_triggers(&mut conn, "paper_attribution_epoch_daily_chain").unwrap();
                }
                _ => unreachable!(),
            }
            drop(conn);

            let error = store
                .append_daily(daily_test_append())
                .expect_err("TEST_CODE corrupted daily state must never be reused");
            assert_eq!(
                error.reason_code(),
                "attribution_epoch_integrity_failed",
                "TEST_CODE daily corruption case {case}"
            );
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

    fn insert_success_with_single_carry(manager: &DatabaseManager) -> String {
        let mut conn = manager.get_conn().unwrap();
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
            created_at: receipt.created_at,
            retention_deadline: receipt.retention_deadline,
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
        item.item_hash
    }

    fn populate_all_protected_tables(manager: &DatabaseManager) {
        insert_success_with_single_carry(manager);
        let store = AttributionEpochStore::new(manager);
        store.append_attempt(sample_attempt()).unwrap();
        store
            .append_daily(AttributionEpochDailyAppend {
                epoch_id: "a".repeat(64),
                date: NaiveDate::from_ymd_opt(2026, 8, 28).unwrap(),
                signal_family: "all".to_owned(),
                payload: serde_json::json!({"closed": 1}),
            })
            .unwrap();
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

    #[test]
    fn semantic_fact_tamper_is_detected_for_every_hash_family() {
        for (table, statement) in [
            (
                "attribution_sample_epoch_receipt",
                "UPDATE attribution_sample_epoch_receipt SET paper_trade_high_water=13 WHERE id=1",
            ),
            (
                "attribution_legacy_carry_item",
                "UPDATE attribution_legacy_carry_item SET quantity=200 WHERE id=1",
            ),
            (
                "attribution_epoch_attempt_audit",
                "UPDATE attribution_epoch_attempt_audit SET reason_code='TEST_CODE_changed' WHERE id=1",
            ),
            (
                "paper_attribution_epoch_daily",
                "UPDATE paper_attribution_epoch_daily SET signal_family='changed' WHERE id=1",
            ),
        ] {
            let database = TestDatabase::new();
            populate_all_protected_tables(&database.manager);
            let mut conn = database.manager.get_conn().unwrap();
            diesel::sql_query(format!("DROP TRIGGER trg_{table}_no_update"))
                .execute(&mut conn)
                .unwrap();
            diesel::sql_query(statement).execute(&mut conn).unwrap();
            install_triggers(&mut conn, table).unwrap();
            drop(conn);
            assert_eq!(
                AttributionEpochStore::new(&database.manager)
                    .load_selector(&AttributionEpochSelector::Legacy)
                    .unwrap_err()
                    .reason_code(),
                "attribution_epoch_integrity_failed",
                "TEST_CODE semantic tamper in {table}"
            );
        }
    }

    #[test]
    fn every_semantic_preimage_field_changes_its_family_hash() {
        macro_rules! changed {
            ($hash:ident, $row:ident, $field:ident, $value:expr) => {{
                let mut mutation = $row.clone();
                mutation.$field = $value;
                assert_ne!(
                    $hash(&$row).unwrap(),
                    $hash(&mutation).unwrap(),
                    "TEST_CODE {} must be hash-bound",
                    stringify!($field)
                );
            }};
        }

        let receipt = sample_persisted_receipt(
            "a".repeat(64),
            None,
            0,
            0,
            canonical_legacy_carry_manifest_hash(&[]),
        );
        changed!(receipt_hash, receipt, epoch_id, "b".repeat(64));
        changed!(
            receipt_hash,
            receipt,
            cutover_completed_trading_date,
            "2026-08-27".to_owned()
        );
        changed!(
            receipt_hash,
            receipt,
            effective_trading_date,
            "2026-09-01".to_owned()
        );
        changed!(receipt_hash, receipt, paper_trade_high_water, 13);
        changed!(
            receipt_hash,
            receipt,
            legacy_filled_manifest_hash,
            "7".repeat(64)
        );
        changed!(
            receipt_hash,
            receipt,
            terminal_binding_manifest_hash,
            "7".repeat(64)
        );
        changed!(receipt_hash, receipt, order_audit_high_water, 15);
        changed!(receipt_hash, receipt, order_audit_tip_hash, "7".repeat(64));
        changed!(
            receipt_hash,
            receipt,
            calendar_authority_hash,
            "7".repeat(64)
        );
        changed!(
            receipt_hash,
            receipt,
            legacy_carry_manifest_hash,
            "7".repeat(64)
        );
        changed!(receipt_hash, receipt, carry_item_count, 1);
        changed!(receipt_hash, receipt, carry_total_quantity, 100);
        changed!(
            receipt_hash,
            receipt,
            position_projection_hash,
            "7".repeat(64)
        );
        changed!(
            receipt_hash,
            receipt,
            previous_epoch_receipt_hash,
            Some("7".repeat(64))
        );
        changed!(receipt_hash, receipt, decision_basis, "changed".to_owned());
        changed!(
            receipt_hash,
            receipt,
            created_at,
            "2026-08-28T08:00:00.001Z".to_owned()
        );
        changed!(
            receipt_hash,
            receipt,
            retention_deadline,
            "2031-08-28T08:00:00.001Z".to_owned()
        );

        let carry = PersistedCarry {
            id: 1,
            epoch_receipt_id: 1,
            code: "600000".to_owned(),
            quantity: 100,
            item_index: 0,
            predecessor_item_hash: CARRY_GENESIS.to_owned(),
            item_hash: String::new(),
            created_at: "2026-08-28T08:00:00.000Z".to_owned(),
            retention_deadline: "2031-08-28T08:00:00.000Z".to_owned(),
        };
        changed!(carry_hash, carry, epoch_receipt_id, 2);
        changed!(carry_hash, carry, code, "600001".to_owned());
        changed!(carry_hash, carry, quantity, 200);
        changed!(carry_hash, carry, item_index, 1);
        changed!(carry_hash, carry, predecessor_item_hash, "7".repeat(64));
        changed!(
            carry_hash,
            carry,
            created_at,
            "2026-08-28T08:00:00.001Z".to_owned()
        );
        changed!(
            carry_hash,
            carry,
            retention_deadline,
            "2031-08-28T08:00:00.001Z".to_owned()
        );

        let attempt = PersistedAttempt {
            id: 1,
            source: "monitor".to_owned(),
            invoked_at: "2026-08-28T07:40:00.000Z".to_owned(),
            completed_session_date: Some("2026-08-28".to_owned()),
            effective_date: Some("2026-08-31".to_owned()),
            outcome: "unavailable".to_owned(),
            reason_code: "window_closed".to_owned(),
            retryable: 1,
            source_summary_hash: "1".repeat(64),
            epoch_id: None,
            success_receipt_hash: None,
            predecessor_attempt_hash: ATTEMPT_GENESIS.to_owned(),
            record_hash: String::new(),
            created_at: "2026-08-28T08:00:00.000Z".to_owned(),
            retention_deadline: "2031-08-28T08:00:00.000Z".to_owned(),
        };
        changed!(attempt_hash, attempt, source, "cli".to_owned());
        changed!(
            attempt_hash,
            attempt,
            invoked_at,
            "2026-08-28T07:40:00.001Z".to_owned()
        );
        changed!(
            attempt_hash,
            attempt,
            completed_session_date,
            Some("2026-08-27".to_owned())
        );
        changed!(
            attempt_hash,
            attempt,
            effective_date,
            Some("2026-09-01".to_owned())
        );
        changed!(attempt_hash, attempt, outcome, "failed".to_owned());
        changed!(attempt_hash, attempt, reason_code, "changed".to_owned());
        changed!(attempt_hash, attempt, retryable, 0);
        changed!(attempt_hash, attempt, source_summary_hash, "7".repeat(64));
        changed!(attempt_hash, attempt, epoch_id, Some("7".repeat(64)));
        changed!(
            attempt_hash,
            attempt,
            success_receipt_hash,
            Some("7".repeat(64))
        );
        changed!(
            attempt_hash,
            attempt,
            predecessor_attempt_hash,
            "7".repeat(64)
        );
        changed!(
            attempt_hash,
            attempt,
            created_at,
            "2026-08-28T08:00:00.001Z".to_owned()
        );
        changed!(
            attempt_hash,
            attempt,
            retention_deadline,
            "2031-08-28T08:00:00.001Z".to_owned()
        );

        let daily = PersistedDaily {
            id: 1,
            epoch_id: "a".repeat(64),
            date: "2026-08-28".to_owned(),
            signal_family: "all".to_owned(),
            payload_json: "{\"closed\":1}".to_owned(),
            payload_hash: "1".repeat(64),
            predecessor_daily_hash: DAILY_GENESIS.to_owned(),
            record_hash: String::new(),
            created_at: "2026-08-28T08:00:00.000Z".to_owned(),
            retention_deadline: "2031-08-28T08:00:00.000Z".to_owned(),
        };
        changed!(daily_hash, daily, epoch_id, "b".repeat(64));
        changed!(daily_hash, daily, date, "2026-08-29".to_owned());
        changed!(daily_hash, daily, signal_family, "other".to_owned());
        changed!(daily_hash, daily, payload_json, "{\"closed\":2}".to_owned());
        changed!(daily_hash, daily, payload_hash, "7".repeat(64));
        changed!(daily_hash, daily, predecessor_daily_hash, "7".repeat(64));
        changed!(
            daily_hash,
            daily,
            created_at,
            "2026-08-28T08:00:00.001Z".to_owned()
        );
        changed!(
            daily_hash,
            daily,
            retention_deadline,
            "2031-08-28T08:00:00.001Z".to_owned()
        );
    }
}
