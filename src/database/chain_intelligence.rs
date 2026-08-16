//! BR-160 append-only persistence for same-date chain-intelligence batches.

use std::collections::BTreeSet;

use chrono::{DateTime, FixedOffset, NaiveDate, SecondsFormat};
use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Integer, Nullable, Text};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::DatabaseManager;

const TABLES: [&str; 6] = [
    "chain_intelligence_batches",
    "chain_intelligence_input_evidence",
    "chain_intelligence_chains",
    "chain_intelligence_members",
    "chain_intelligence_rejections",
    "chain_intelligence_visibility_receipts",
];

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS chain_intelligence_batches (
    batch_id TEXT PRIMARY KEY,
    content_hash TEXT NOT NULL,
    trading_date TEXT NOT NULL,
    calculation_version TEXT NOT NULL,
    taxonomy_version TEXT NOT NULL,
    created_at TEXT NOT NULL,
    recorded_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK (length(trim(batch_id)) > 0),
    CHECK (length(content_hash) = 64),
    CHECK (length(trim(calculation_version)) > 0),
    CHECK (length(trim(taxonomy_version)) > 0)
);

CREATE TABLE IF NOT EXISTS chain_intelligence_input_evidence (
    input_id TEXT PRIMARY KEY,
    batch_id TEXT NOT NULL REFERENCES chain_intelligence_batches(batch_id),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    capability TEXT NOT NULL,
    provider TEXT NOT NULL,
    source TEXT NOT NULL,
    source_at TEXT,
    observed_at TEXT NOT NULL,
    source_batch_id TEXT NOT NULL,
    source_batch_hash TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    UNIQUE (batch_id, ordinal),
    UNIQUE (batch_id, source_batch_id),
    CHECK (length(trim(capability)) > 0),
    CHECK (length(trim(provider)) > 0),
    CHECK (length(trim(source)) > 0),
    CHECK (length(trim(source_batch_id)) > 0),
    CHECK (length(source_batch_hash) = 64),
    CHECK (length(content_hash) = 64)
);

CREATE TABLE IF NOT EXISTS chain_intelligence_chains (
    chain_row_id TEXT PRIMARY KEY,
    batch_id TEXT NOT NULL REFERENCES chain_intelligence_batches(batch_id),
    chain_id TEXT NOT NULL,
    canonical_board_id TEXT NOT NULL,
    board_name TEXT NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    upper_limit_count INTEGER NOT NULL CHECK (upper_limit_count >= 3),
    continuous_count INTEGER NOT NULL CHECK (
        continuous_count >= 0 AND continuous_count <= upper_limit_count
    ),
    content_hash TEXT NOT NULL,
    UNIQUE (batch_id, chain_id),
    UNIQUE (batch_id, canonical_board_id),
    UNIQUE (batch_id, ordinal),
    CHECK (length(trim(chain_id)) > 0),
    CHECK (length(trim(canonical_board_id)) > 0),
    CHECK (length(trim(board_name)) > 0),
    CHECK (length(content_hash) = 64)
);

CREATE TABLE IF NOT EXISTS chain_intelligence_members (
    member_id TEXT PRIMARY KEY,
    chain_row_id TEXT NOT NULL REFERENCES chain_intelligence_chains(chain_row_id),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    instrument_id TEXT NOT NULL,
    security_name TEXT NOT NULL,
    source_event_id TEXT NOT NULL,
    streak INTEGER NOT NULL CHECK (streak > 0),
    content_hash TEXT NOT NULL,
    UNIQUE (chain_row_id, ordinal),
    UNIQUE (chain_row_id, instrument_id),
    CHECK (__CHAIN_STOCK_CODE_CHECK__),
    CHECK (length(trim(security_name)) > 0),
    CHECK (length(trim(source_event_id)) > 0),
    CHECK (length(content_hash) = 64)
);

CREATE TABLE IF NOT EXISTS chain_intelligence_rejections (
    rejection_id TEXT PRIMARY KEY,
    batch_id TEXT NOT NULL REFERENCES chain_intelligence_batches(batch_id),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    identity_hash TEXT NOT NULL,
    reason_code TEXT NOT NULL,
    retryable INTEGER NOT NULL CHECK (retryable IN (0, 1)),
    content_hash TEXT NOT NULL,
    UNIQUE (batch_id, ordinal),
    CHECK (length(identity_hash) = 64),
    CHECK (length(trim(reason_code)) > 0),
    CHECK (length(content_hash) = 64)
);

CREATE TABLE IF NOT EXISTS chain_intelligence_visibility_receipts (
    receipt_id TEXT PRIMARY KEY,
    batch_id TEXT NOT NULL UNIQUE REFERENCES chain_intelligence_batches(batch_id),
    audit_record_hash TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    published_at TEXT NOT NULL,
    recorded_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK (length(trim(receipt_id)) > 0),
    CHECK (length(audit_record_hash) = 64),
    CHECK (length(content_hash) = 64)
);

CREATE INDEX IF NOT EXISTS idx_chain_intelligence_visible_date
    ON chain_intelligence_batches(trading_date, batch_id);
CREATE INDEX IF NOT EXISTS idx_chain_intelligence_chain_order
    ON chain_intelligence_chains(batch_id, ordinal, chain_id);
CREATE INDEX IF NOT EXISTS idx_chain_intelligence_member_order
    ON chain_intelligence_members(chain_row_id, ordinal, instrument_id);
"#;

#[derive(Debug, Error)]
pub enum ChainStoreError {
    #[error("chain intelligence conflict for {entity} identity={identity}")]
    Conflict {
        entity: &'static str,
        identity: String,
    },
    #[error("invalid chain intelligence input: {0}")]
    InvalidInput(String),
    #[error("chain intelligence database error: {0}")]
    Database(#[from] diesel::result::Error),
}

pub type ChainStoreResult<T> = Result<T, ChainStoreError>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainInputEvidenceInput {
    pub input_id: String,
    pub ordinal: i32,
    pub capability: String,
    pub provider: String,
    pub source: String,
    pub source_at: Option<String>,
    /// Provider observation text preserved exactly as emitted by the upstream
    /// contract.  Providers currently use Unix-second/nanosecond text, so
    /// coercing this field to RFC3339 would either reject real evidence or
    /// tempt callers to substitute process time.
    pub observed_at: String,
    pub source_batch_id: String,
    pub source_batch_hash: String,
    pub content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainMemberInput {
    pub member_id: String,
    pub ordinal: i32,
    pub instrument_id: String,
    pub security_name: String,
    pub source_event_id: String,
    pub streak: i32,
    pub content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainInput {
    pub chain_row_id: String,
    pub chain_id: String,
    pub canonical_board_id: String,
    pub board_name: String,
    pub ordinal: i32,
    pub upper_limit_count: i32,
    pub continuous_count: i32,
    pub content_hash: String,
    pub members: Vec<ChainMemberInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainRejectionInput {
    pub rejection_id: String,
    pub ordinal: i32,
    pub identity_hash: String,
    pub reason_code: String,
    pub retryable: bool,
    pub content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainBatchInput {
    pub batch_id: String,
    pub content_hash: String,
    pub trading_date: NaiveDate,
    pub calculation_version: String,
    pub taxonomy_version: String,
    pub created_at: DateTime<FixedOffset>,
    pub inputs: Vec<ChainInputEvidenceInput>,
    pub chains: Vec<ChainInput>,
    pub rejections: Vec<ChainRejectionInput>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainVisibilityReceiptInput {
    pub receipt_id: String,
    pub batch_id: String,
    pub audit_record_hash: String,
    pub content_hash: String,
    pub published_at: DateTime<FixedOffset>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainStageReceipt {
    pub inserted: bool,
    pub chains_inserted: usize,
    pub members_inserted: usize,
}

// M4c: 以下三个 visible 类型补 Serialize+Deserialize — 服务端 op 61
// (market.chain_batch) 直出 to_value, 客户端 converter 重建 VisibleChainBatch。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibleChainMember {
    pub instrument_id: String,
    pub security_name: String,
    pub source_event_id: String,
    pub streak: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibleChain {
    pub chain_id: String,
    pub canonical_board_id: String,
    pub board_name: String,
    pub upper_limit_count: i32,
    pub continuous_count: i32,
    pub members: Vec<VisibleChainMember>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibleChainBatch {
    pub batch_id: String,
    pub content_hash: String,
    pub trading_date: NaiveDate,
    pub calculation_version: String,
    pub taxonomy_version: String,
    pub inputs: Vec<ChainInputEvidenceInput>,
    pub chains: Vec<VisibleChain>,
    pub rejections: Vec<ChainRejectionInput>,
}

#[derive(QueryableByName)]
struct ExistingHashRow {
    #[diesel(sql_type = Text)]
    content_hash: String,
}

#[derive(QueryableByName)]
struct VisibilityRow {
    #[diesel(sql_type = Text)]
    content_hash: String,
    #[diesel(sql_type = Text)]
    audit_record_hash: String,
}

#[derive(QueryableByName)]
struct CountRow {
    #[diesel(sql_type = BigInt)]
    count: i64,
}

#[derive(QueryableByName)]
struct VisibleBatchRow {
    #[diesel(sql_type = Text)]
    batch_id: String,
    #[diesel(sql_type = Text)]
    content_hash: String,
    #[diesel(sql_type = Text)]
    trading_date: String,
    #[diesel(sql_type = Text)]
    calculation_version: String,
    #[diesel(sql_type = Text)]
    taxonomy_version: String,
}

#[derive(QueryableByName)]
struct InputRow {
    #[diesel(sql_type = Text)]
    input_id: String,
    #[diesel(sql_type = Integer)]
    ordinal: i32,
    #[diesel(sql_type = Text)]
    capability: String,
    #[diesel(sql_type = Text)]
    provider: String,
    #[diesel(sql_type = Text)]
    source: String,
    #[diesel(sql_type = Nullable<Text>)]
    source_at: Option<String>,
    #[diesel(sql_type = Text)]
    observed_at: String,
    #[diesel(sql_type = Text)]
    source_batch_id: String,
    #[diesel(sql_type = Text)]
    source_batch_hash: String,
    #[diesel(sql_type = Text)]
    content_hash: String,
}

#[derive(QueryableByName)]
struct ChainRow {
    #[diesel(sql_type = Text)]
    chain_row_id: String,
    #[diesel(sql_type = Text)]
    chain_id: String,
    #[diesel(sql_type = Text)]
    canonical_board_id: String,
    #[diesel(sql_type = Text)]
    board_name: String,
    #[diesel(sql_type = Integer)]
    upper_limit_count: i32,
    #[diesel(sql_type = Integer)]
    continuous_count: i32,
}

#[derive(QueryableByName)]
struct MemberRow {
    #[diesel(sql_type = Text)]
    instrument_id: String,
    #[diesel(sql_type = Text)]
    security_name: String,
    #[diesel(sql_type = Text)]
    source_event_id: String,
    #[diesel(sql_type = Integer)]
    streak: i32,
}

#[derive(QueryableByName)]
struct RejectionRow {
    #[diesel(sql_type = Text)]
    rejection_id: String,
    #[diesel(sql_type = Integer)]
    ordinal: i32,
    #[diesel(sql_type = Text)]
    identity_hash: String,
    #[diesel(sql_type = Text)]
    reason_code: String,
    #[diesel(sql_type = Integer)]
    retryable: i32,
    #[diesel(sql_type = Text)]
    content_hash: String,
}

pub fn create_schema(conn: &mut SqliteConnection) -> Result<(), String> {
    let schema = SCHEMA.replace("__CHAIN_STOCK_CODE_CHECK__", stock_code_check());
    conn.batch_execute(&schema)
        .map_err(|error| error.to_string())?;
    for table in TABLES {
        for action in ["UPDATE", "DELETE"] {
            let suffix = action.to_ascii_lowercase();
            conn.batch_execute(&format!(
                "CREATE TRIGGER IF NOT EXISTS trg_{table}_no_{suffix}
                 BEFORE {action} ON {table}
                 BEGIN
                     SELECT RAISE(ABORT, 'BR-160 {table} is append-only');
                 END;"
            ))
            .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

#[cfg(not(test))]
fn stock_code_check() -> &'static str {
    "length(instrument_id) = 6 AND instrument_id NOT GLOB '*[^0-9]*'"
}

#[cfg(test)]
fn stock_code_check() -> &'static str {
    "instrument_id GLOB 'TEST_CODE_[0-9][0-9][0-9][0-9][0-9][0-9]'"
}

#[cfg(not(test))]
fn valid_stock_code(code: &str) -> bool {
    code.len() == 6 && code.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
fn valid_stock_code(code: &str) -> bool {
    code.strip_prefix("TEST_CODE_")
        .is_some_and(|suffix| suffix.len() == 6 && suffix.bytes().all(|byte| byte.is_ascii_digit()))
}

fn require_non_empty(field: &'static str, value: &str) -> ChainStoreResult<()> {
    if value.trim().is_empty() {
        return Err(ChainStoreError::InvalidInput(format!(
            "{field} must not be empty"
        )));
    }
    Ok(())
}

fn require_hash(field: &'static str, value: &str) -> ChainStoreResult<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ChainStoreError::InvalidInput(format!(
            "{field} must be 64 lowercase hex characters"
        )));
    }
    Ok(())
}

fn validate_batch(batch: &ChainBatchInput) -> ChainStoreResult<()> {
    for (field, value) in [
        ("batch_id", batch.batch_id.as_str()),
        ("calculation_version", batch.calculation_version.as_str()),
        ("taxonomy_version", batch.taxonomy_version.as_str()),
    ] {
        require_non_empty(field, value)?;
    }
    require_hash("batch content_hash", &batch.content_hash)?;
    if batch.inputs.is_empty() {
        return Err(ChainStoreError::InvalidInput(
            "a chain batch requires input evidence".to_owned(),
        ));
    }

    let mut input_ids = BTreeSet::new();
    let mut input_batch_ids = BTreeSet::new();
    for (ordinal, input) in batch.inputs.iter().enumerate() {
        let expected = i32::try_from(ordinal).map_err(|_| {
            ChainStoreError::InvalidInput("input evidence ordinal overflow".to_owned())
        })?;
        if input.ordinal != expected {
            return Err(ChainStoreError::InvalidInput(format!(
                "input evidence ordinal {} is not stable expected {expected}",
                input.ordinal
            )));
        }
        for (field, value) in [
            ("input_id", input.input_id.as_str()),
            ("input capability", input.capability.as_str()),
            ("input provider", input.provider.as_str()),
            ("input source", input.source.as_str()),
            ("input observed_at", input.observed_at.as_str()),
            ("input source_batch_id", input.source_batch_id.as_str()),
        ] {
            require_non_empty(field, value)?;
        }
        if input
            .source_at
            .as_deref()
            .is_some_and(|source_at| source_at.trim().is_empty())
        {
            return Err(ChainStoreError::InvalidInput(
                "input source_at must be absent or non-empty".to_owned(),
            ));
        }
        require_hash("input source_batch_hash", &input.source_batch_hash)?;
        require_hash("input content_hash", &input.content_hash)?;
        if !input_ids.insert(input.input_id.as_str())
            || !input_batch_ids.insert(input.source_batch_id.as_str())
        {
            return Err(ChainStoreError::InvalidInput(
                "duplicate input evidence identity".to_owned(),
            ));
        }
    }

    let mut chain_ids = BTreeSet::new();
    let mut board_ids = BTreeSet::new();
    for (ordinal, chain) in batch.chains.iter().enumerate() {
        let expected = i32::try_from(ordinal)
            .map_err(|_| ChainStoreError::InvalidInput("chain ordinal overflow".to_owned()))?;
        if chain.ordinal != expected {
            return Err(ChainStoreError::InvalidInput(format!(
                "chain ordinal {} is not stable expected {expected}",
                chain.ordinal
            )));
        }
        for (field, value) in [
            ("chain_row_id", chain.chain_row_id.as_str()),
            ("chain_id", chain.chain_id.as_str()),
            ("canonical_board_id", chain.canonical_board_id.as_str()),
            ("board_name", chain.board_name.as_str()),
        ] {
            require_non_empty(field, value)?;
        }
        require_hash("chain content_hash", &chain.content_hash)?;
        if !chain_ids.insert(chain.chain_id.as_str())
            || !board_ids.insert(chain.canonical_board_id.as_str())
        {
            return Err(ChainStoreError::InvalidInput(
                "duplicate chain or board identity".to_owned(),
            ));
        }
        let member_count = i32::try_from(chain.members.len())
            .map_err(|_| ChainStoreError::InvalidInput("chain member count overflow".to_owned()))?;
        if member_count < 3
            || chain.upper_limit_count != member_count
            || chain.continuous_count < 0
            || chain.continuous_count > member_count
        {
            return Err(ChainStoreError::InvalidInput(format!(
                "chain {} count contract mismatch",
                chain.chain_id
            )));
        }
        if let Some(previous) = ordinal
            .checked_sub(1)
            .and_then(|index| batch.chains.get(index))
        {
            let correct = previous.upper_limit_count > chain.upper_limit_count
                || (previous.upper_limit_count == chain.upper_limit_count
                    && (previous.continuous_count > chain.continuous_count
                        || (previous.continuous_count == chain.continuous_count
                            && previous.canonical_board_id <= chain.canonical_board_id)));
            if !correct {
                return Err(ChainStoreError::InvalidInput(
                    "chains are not in BR-160 deterministic order".to_owned(),
                ));
            }
        }
        validate_members(chain)?;
    }

    for (ordinal, rejection) in batch.rejections.iter().enumerate() {
        let expected = i32::try_from(ordinal)
            .map_err(|_| ChainStoreError::InvalidInput("rejection ordinal overflow".to_owned()))?;
        if rejection.ordinal != expected {
            return Err(ChainStoreError::InvalidInput(
                "rejection ordinals are not stable".to_owned(),
            ));
        }
        require_non_empty("rejection_id", &rejection.rejection_id)?;
        require_non_empty("rejection reason_code", &rejection.reason_code)?;
        require_hash("rejection identity_hash", &rejection.identity_hash)?;
        require_hash("rejection content_hash", &rejection.content_hash)?;
    }
    Ok(())
}

fn validate_members(chain: &ChainInput) -> ChainStoreResult<()> {
    let mut instruments = BTreeSet::new();
    for (ordinal, member) in chain.members.iter().enumerate() {
        let expected = i32::try_from(ordinal)
            .map_err(|_| ChainStoreError::InvalidInput("member ordinal overflow".to_owned()))?;
        if member.ordinal != expected {
            return Err(ChainStoreError::InvalidInput(
                "member ordinals are not stable".to_owned(),
            ));
        }
        require_non_empty("member_id", &member.member_id)?;
        require_non_empty("member security_name", &member.security_name)?;
        require_non_empty("member source_event_id", &member.source_event_id)?;
        require_hash("member content_hash", &member.content_hash)?;
        if !valid_stock_code(&member.instrument_id) || member.streak <= 0 {
            return Err(ChainStoreError::InvalidInput(format!(
                "member {} instrument/streak is invalid",
                member.member_id
            )));
        }
        if !instruments.insert(member.instrument_id.as_str()) {
            return Err(ChainStoreError::InvalidInput(format!(
                "duplicate member instrument {}",
                member.instrument_id
            )));
        }
        if let Some(previous) = ordinal
            .checked_sub(1)
            .and_then(|index| chain.members.get(index))
        {
            let correct = previous.streak > member.streak
                || (previous.streak == member.streak
                    && (previous.source_event_id < member.source_event_id
                        || (previous.source_event_id == member.source_event_id
                            && previous.instrument_id <= member.instrument_id)));
            if !correct {
                return Err(ChainStoreError::InvalidInput(
                    "members are not in BR-160 deterministic order".to_owned(),
                ));
            }
        }
    }
    Ok(())
}

fn format_timestamp(value: DateTime<FixedOffset>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, false)
}

pub struct ChainIntelligenceStore<'a> {
    conn: &'a mut SqliteConnection,
}

impl<'a> ChainIntelligenceStore<'a> {
    pub fn new(conn: &'a mut SqliteConnection) -> Self {
        Self { conn }
    }

    pub fn stage_batch(&mut self, batch: &ChainBatchInput) -> ChainStoreResult<ChainStageReceipt> {
        validate_batch(batch)?;
        self.conn
            .immediate_transaction::<_, ChainStoreError, _>(|conn| {
                let existing = diesel::sql_query(
                    "SELECT content_hash FROM chain_intelligence_batches WHERE batch_id = ?",
                )
                .bind::<Text, _>(&batch.batch_id)
                .get_result::<ExistingHashRow>(conn)
                .optional()?;
                if let Some(existing) = existing {
                    if existing.content_hash == batch.content_hash {
                        return Ok(ChainStageReceipt {
                            inserted: false,
                            chains_inserted: 0,
                            members_inserted: 0,
                        });
                    }
                    return Err(ChainStoreError::Conflict {
                        entity: "batch",
                        identity: batch.batch_id.clone(),
                    });
                }

                diesel::sql_query(
                    "INSERT INTO chain_intelligence_batches (
                        batch_id, content_hash, trading_date, calculation_version,
                        taxonomy_version, created_at
                     ) VALUES (?, ?, ?, ?, ?, ?)",
                )
                .bind::<Text, _>(&batch.batch_id)
                .bind::<Text, _>(&batch.content_hash)
                .bind::<Text, _>(batch.trading_date.format("%Y-%m-%d").to_string())
                .bind::<Text, _>(&batch.calculation_version)
                .bind::<Text, _>(&batch.taxonomy_version)
                .bind::<Text, _>(format_timestamp(batch.created_at))
                .execute(conn)?;

                for input in &batch.inputs {
                    diesel::sql_query(
                        "INSERT INTO chain_intelligence_input_evidence (
                            input_id, batch_id, ordinal, capability, provider, source,
                            source_at, observed_at, source_batch_id, source_batch_hash,
                            content_hash
                         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    )
                    .bind::<Text, _>(&input.input_id)
                    .bind::<Text, _>(&batch.batch_id)
                    .bind::<Integer, _>(input.ordinal)
                    .bind::<Text, _>(&input.capability)
                    .bind::<Text, _>(&input.provider)
                    .bind::<Text, _>(&input.source)
                    .bind::<Nullable<Text>, _>(input.source_at.as_deref())
                    .bind::<Text, _>(&input.observed_at)
                    .bind::<Text, _>(&input.source_batch_id)
                    .bind::<Text, _>(&input.source_batch_hash)
                    .bind::<Text, _>(&input.content_hash)
                    .execute(conn)?;
                }

                let mut members_inserted = 0usize;
                for chain in &batch.chains {
                    diesel::sql_query(
                        "INSERT INTO chain_intelligence_chains (
                            chain_row_id, batch_id, chain_id, canonical_board_id,
                            board_name, ordinal, upper_limit_count, continuous_count,
                            content_hash
                         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    )
                    .bind::<Text, _>(&chain.chain_row_id)
                    .bind::<Text, _>(&batch.batch_id)
                    .bind::<Text, _>(&chain.chain_id)
                    .bind::<Text, _>(&chain.canonical_board_id)
                    .bind::<Text, _>(&chain.board_name)
                    .bind::<Integer, _>(chain.ordinal)
                    .bind::<Integer, _>(chain.upper_limit_count)
                    .bind::<Integer, _>(chain.continuous_count)
                    .bind::<Text, _>(&chain.content_hash)
                    .execute(conn)?;
                    for member in &chain.members {
                        diesel::sql_query(
                            "INSERT INTO chain_intelligence_members (
                                member_id, chain_row_id, ordinal, instrument_id,
                                security_name, source_event_id, streak, content_hash
                             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                        )
                        .bind::<Text, _>(&member.member_id)
                        .bind::<Text, _>(&chain.chain_row_id)
                        .bind::<Integer, _>(member.ordinal)
                        .bind::<Text, _>(&member.instrument_id)
                        .bind::<Text, _>(&member.security_name)
                        .bind::<Text, _>(&member.source_event_id)
                        .bind::<Integer, _>(member.streak)
                        .bind::<Text, _>(&member.content_hash)
                        .execute(conn)?;
                        members_inserted += 1;
                    }
                }

                for rejection in &batch.rejections {
                    diesel::sql_query(
                        "INSERT INTO chain_intelligence_rejections (
                            rejection_id, batch_id, ordinal, identity_hash,
                            reason_code, retryable, content_hash
                         ) VALUES (?, ?, ?, ?, ?, ?, ?)",
                    )
                    .bind::<Text, _>(&rejection.rejection_id)
                    .bind::<Text, _>(&batch.batch_id)
                    .bind::<Integer, _>(rejection.ordinal)
                    .bind::<Text, _>(&rejection.identity_hash)
                    .bind::<Text, _>(&rejection.reason_code)
                    .bind::<Integer, _>(i32::from(rejection.retryable))
                    .bind::<Text, _>(&rejection.content_hash)
                    .execute(conn)?;
                }

                Ok(ChainStageReceipt {
                    inserted: true,
                    chains_inserted: batch.chains.len(),
                    members_inserted,
                })
            })
    }

    pub fn publish(&mut self, receipt: &ChainVisibilityReceiptInput) -> ChainStoreResult<bool> {
        for (field, value) in [
            ("visibility receipt_id", receipt.receipt_id.as_str()),
            ("visibility batch_id", receipt.batch_id.as_str()),
        ] {
            require_non_empty(field, value)?;
        }
        require_hash("visibility audit_record_hash", &receipt.audit_record_hash)?;
        require_hash("visibility content_hash", &receipt.content_hash)?;

        self.conn
            .immediate_transaction::<_, ChainStoreError, _>(|conn| {
                let staged = diesel::sql_query(
                    "SELECT content_hash FROM chain_intelligence_batches WHERE batch_id = ?",
                )
                .bind::<Text, _>(&receipt.batch_id)
                .get_result::<ExistingHashRow>(conn)
                .optional()?
                .ok_or_else(|| {
                    ChainStoreError::InvalidInput(format!(
                        "visibility refers to unstaged batch {}",
                        receipt.batch_id
                    ))
                })?;
                if staged.content_hash != receipt.content_hash {
                    return Err(ChainStoreError::Conflict {
                        entity: "visibility-content",
                        identity: receipt.batch_id.clone(),
                    });
                }
                let existing = diesel::sql_query(
                    "SELECT content_hash, audit_record_hash
                     FROM chain_intelligence_visibility_receipts WHERE batch_id = ?",
                )
                .bind::<Text, _>(&receipt.batch_id)
                .get_result::<VisibilityRow>(conn)
                .optional()?;
                if let Some(existing) = existing {
                    if existing.content_hash == receipt.content_hash
                        && existing.audit_record_hash == receipt.audit_record_hash
                    {
                        return Ok(false);
                    }
                    return Err(ChainStoreError::Conflict {
                        entity: "visibility",
                        identity: receipt.batch_id.clone(),
                    });
                }
                diesel::sql_query(
                    "INSERT INTO chain_intelligence_visibility_receipts (
                        receipt_id, batch_id, audit_record_hash, content_hash, published_at
                     ) VALUES (?, ?, ?, ?, ?)",
                )
                .bind::<Text, _>(&receipt.receipt_id)
                .bind::<Text, _>(&receipt.batch_id)
                .bind::<Text, _>(&receipt.audit_record_hash)
                .bind::<Text, _>(&receipt.content_hash)
                .bind::<Text, _>(format_timestamp(receipt.published_at))
                .execute(conn)?;
                Ok(true)
            })
    }

    pub fn visible_batch_count(&mut self, trading_date: NaiveDate) -> ChainStoreResult<i64> {
        diesel::sql_query(
            "SELECT COUNT(*) AS count
             FROM chain_intelligence_batches AS batch
             INNER JOIN chain_intelligence_visibility_receipts AS visibility
                ON visibility.batch_id = batch.batch_id
               AND visibility.content_hash = batch.content_hash
             WHERE batch.trading_date = ?",
        )
        .bind::<Text, _>(trading_date.format("%Y-%m-%d").to_string())
        .get_result::<CountRow>(self.conn)
        .map(|row| row.count)
        .map_err(ChainStoreError::from)
    }

    /// Reads one exact visible batch. Staged rows without a matching immutable
    /// visibility receipt are deliberately indistinguishable from absence.
    pub fn load_visible_batch(
        &mut self,
        batch_id: &str,
    ) -> ChainStoreResult<Option<VisibleChainBatch>> {
        require_non_empty("visible batch_id", batch_id)?;
        let Some(batch) = diesel::sql_query(
            "SELECT batch.batch_id, batch.content_hash, batch.trading_date,
                    batch.calculation_version, batch.taxonomy_version
             FROM chain_intelligence_batches AS batch
             INNER JOIN chain_intelligence_visibility_receipts AS visibility
                ON visibility.batch_id = batch.batch_id
               AND visibility.content_hash = batch.content_hash
             WHERE batch.batch_id = ?",
        )
        .bind::<Text, _>(batch_id)
        .get_result::<VisibleBatchRow>(self.conn)
        .optional()?
        else {
            return Ok(None);
        };
        let trading_date =
            NaiveDate::parse_from_str(&batch.trading_date, "%Y-%m-%d").map_err(|error| {
                ChainStoreError::InvalidInput(format!(
                    "stored chain trading_date {:?} is invalid: {error}",
                    batch.trading_date
                ))
            })?;
        let inputs = diesel::sql_query(
            "SELECT input_id, ordinal, capability, provider, source, source_at,
                    observed_at, source_batch_id, source_batch_hash, content_hash
             FROM chain_intelligence_input_evidence
             WHERE batch_id = ? ORDER BY ordinal ASC",
        )
        .bind::<Text, _>(batch_id)
        .load::<InputRow>(self.conn)?
        .into_iter()
        .map(|row| ChainInputEvidenceInput {
            input_id: row.input_id,
            ordinal: row.ordinal,
            capability: row.capability,
            provider: row.provider,
            source: row.source,
            source_at: row.source_at,
            observed_at: row.observed_at,
            source_batch_id: row.source_batch_id,
            source_batch_hash: row.source_batch_hash,
            content_hash: row.content_hash,
        })
        .collect();
        let chain_rows = diesel::sql_query(
            "SELECT chain_row_id, chain_id, canonical_board_id, board_name,
                    upper_limit_count, continuous_count
             FROM chain_intelligence_chains
             WHERE batch_id = ? ORDER BY ordinal ASC",
        )
        .bind::<Text, _>(batch_id)
        .load::<ChainRow>(self.conn)?;
        let mut chains = Vec::with_capacity(chain_rows.len());
        for row in chain_rows {
            let members = diesel::sql_query(
                "SELECT instrument_id, security_name, source_event_id, streak
                 FROM chain_intelligence_members
                 WHERE chain_row_id = ? ORDER BY ordinal ASC",
            )
            .bind::<Text, _>(&row.chain_row_id)
            .load::<MemberRow>(self.conn)?
            .into_iter()
            .map(|member| VisibleChainMember {
                instrument_id: member.instrument_id,
                security_name: member.security_name,
                source_event_id: member.source_event_id,
                streak: member.streak,
            })
            .collect();
            chains.push(VisibleChain {
                chain_id: row.chain_id,
                canonical_board_id: row.canonical_board_id,
                board_name: row.board_name,
                upper_limit_count: row.upper_limit_count,
                continuous_count: row.continuous_count,
                members,
            });
        }
        let rejections = diesel::sql_query(
            "SELECT rejection_id, ordinal, identity_hash, reason_code,
                    retryable, content_hash
             FROM chain_intelligence_rejections
             WHERE batch_id = ? ORDER BY ordinal ASC",
        )
        .bind::<Text, _>(batch_id)
        .load::<RejectionRow>(self.conn)?
        .into_iter()
        .map(|row| ChainRejectionInput {
            rejection_id: row.rejection_id,
            ordinal: row.ordinal,
            identity_hash: row.identity_hash,
            reason_code: row.reason_code,
            retryable: row.retryable != 0,
            content_hash: row.content_hash,
        })
        .collect();
        Ok(Some(VisibleChainBatch {
            batch_id: batch.batch_id,
            content_hash: batch.content_hash,
            trading_date,
            calculation_version: batch.calculation_version,
            taxonomy_version: batch.taxonomy_version,
            inputs,
            chains,
            rejections,
        }))
    }
}

impl DatabaseManager {
    pub fn stage_chain_intelligence_batch(
        &self,
        batch: &ChainBatchInput,
    ) -> Result<ChainStageReceipt, String> {
        let mut conn = self
            .get_conn()
            .map_err(|error| format!("BR-160 chain batch connection failed: {error}"))?;
        ChainIntelligenceStore::new(&mut conn)
            .stage_batch(batch)
            .map_err(|error| error.to_string())
    }

    pub fn publish_chain_intelligence_batch(
        &self,
        receipt: &ChainVisibilityReceiptInput,
    ) -> Result<bool, String> {
        let mut conn = self
            .get_conn()
            .map_err(|error| format!("BR-160 chain visibility connection failed: {error}"))?;
        ChainIntelligenceStore::new(&mut conn)
            .publish(receipt)
            .map_err(|error| error.to_string())
    }

    pub fn load_visible_chain_intelligence_batch(
        &self,
        batch_id: &str,
    ) -> Result<Option<VisibleChainBatch>, String> {
        let mut conn = self
            .get_conn()
            .map_err(|error| format!("BR-160 chain reader connection failed: {error}"))?;
        ChainIntelligenceStore::new(&mut conn)
            .load_visible_batch(batch_id)
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(QueryableByName)]
    struct ObservedAtRow {
        #[diesel(sql_type = Text)]
        observed_at: String,
    }

    fn hash(character: char) -> String {
        std::iter::repeat_n(character, 64).collect()
    }

    fn connection() -> SqliteConnection {
        let mut conn = SqliteConnection::establish(":memory:").expect("memory sqlite");
        conn.batch_execute("PRAGMA foreign_keys = ON;")
            .expect("foreign keys");
        create_schema(&mut conn).expect("chain schema");
        conn
    }

    fn batch() -> ChainBatchInput {
        let timestamp =
            DateTime::parse_from_rfc3339("2099-01-02T16:00:00+08:00").expect("timestamp");
        let members = [
            ("000001", "event-1", 3),
            ("000002", "event-2", 2),
            ("000003", "event-3", 1),
        ]
        .into_iter()
        .enumerate()
        .map(|(ordinal, (code, event, streak))| ChainMemberInput {
            member_id: format!("TEST_CODE_member_{code}"),
            ordinal: i32::try_from(ordinal).expect("ordinal"),
            instrument_id: format!("TEST_CODE_{code}"),
            security_name: format!("TEST_CODE_name_{code}"),
            source_event_id: format!("TEST_CODE_{event}"),
            streak,
            content_hash: hash('c'),
        })
        .collect();
        ChainBatchInput {
            batch_id: "TEST_CODE_chain_batch_1".to_owned(),
            content_hash: hash('a'),
            trading_date: NaiveDate::from_ymd_opt(2099, 1, 2).expect("date"),
            calculation_version: "TEST_CODE_chain-v1".to_owned(),
            taxonomy_version: "TEST_CODE_taxonomy-v1".to_owned(),
            created_at: timestamp,
            inputs: vec![ChainInputEvidenceInput {
                input_id: "TEST_CODE_input_1".to_owned(),
                ordinal: 0,
                capability: "TEST_CODE_limit-pool".to_owned(),
                provider: "TEST_CODE_eastmoney".to_owned(),
                source: "TEST_CODE_source".to_owned(),
                source_at: Some("2099-01-02".to_owned()),
                observed_at: "TEST_CODE_observed_at_1".to_owned(),
                source_batch_id: "TEST_CODE_source_batch_1".to_owned(),
                source_batch_hash: hash('b'),
                content_hash: hash('c'),
            }],
            chains: vec![ChainInput {
                chain_row_id: "TEST_CODE_chain_row_1".to_owned(),
                chain_id: "TEST_CODE_chain_1".to_owned(),
                canonical_board_id: "TEST_CODE_tdx:gn:test".to_owned(),
                board_name: "测试主线".to_owned(),
                ordinal: 0,
                upper_limit_count: 3,
                continuous_count: 2,
                content_hash: hash('d'),
                members,
            }],
            rejections: vec![],
        }
    }

    fn visibility(batch: &ChainBatchInput) -> ChainVisibilityReceiptInput {
        ChainVisibilityReceiptInput {
            receipt_id: "TEST_CODE_visibility_1".to_owned(),
            batch_id: batch.batch_id.clone(),
            audit_record_hash: hash('e'),
            content_hash: batch.content_hash.clone(),
            published_at: batch.created_at,
        }
    }

    #[test]
    fn br160_staged_batch_is_hidden_until_authoritative_visibility_receipt() {
        let mut conn = connection();
        let batch = batch();
        let mut store = ChainIntelligenceStore::new(&mut conn);
        let staged = store.stage_batch(&batch).expect("stage batch");
        assert!(staged.inserted);
        assert_eq!(staged.chains_inserted, 1);
        assert_eq!(staged.members_inserted, 3);
        assert_eq!(store.visible_batch_count(batch.trading_date).unwrap(), 0);
        assert!(store
            .load_visible_batch(&batch.batch_id)
            .expect("hidden read")
            .is_none());

        assert!(store.publish(&visibility(&batch)).expect("publish"));
        assert_eq!(store.visible_batch_count(batch.trading_date).unwrap(), 1);
        let visible = store
            .load_visible_batch(&batch.batch_id)
            .expect("visible read")
            .expect("visible batch");
        assert_eq!(visible.batch_id, batch.batch_id);
        assert_eq!(visible.inputs, batch.inputs);
        assert_eq!(visible.chains.len(), 1);
        assert_eq!(visible.chains[0].members.len(), 3);
        assert_eq!(
            visible.chains[0].members[0].security_name,
            "TEST_CODE_name_000001"
        );
    }

    #[test]
    fn br160_same_identity_is_idempotent_and_different_content_conflicts() {
        let mut conn = connection();
        let batch = batch();
        let mut store = ChainIntelligenceStore::new(&mut conn);
        store.stage_batch(&batch).expect("first stage");
        assert!(
            !store
                .stage_batch(&batch)
                .expect("idempotent stage")
                .inserted
        );

        let mut conflict = batch;
        conflict.content_hash = hash('f');
        assert!(matches!(
            store.stage_batch(&conflict),
            Err(ChainStoreError::Conflict {
                entity: "batch",
                ..
            })
        ));
    }

    #[test]
    fn br160_rejects_non_deterministic_member_order_before_writing() {
        let mut conn = connection();
        let mut batch = batch();
        batch.chains[0].members.swap(0, 1);
        let mut store = ChainIntelligenceStore::new(&mut conn);
        store
            .stage_batch(&batch)
            .expect_err("wrong member order must fail");
        assert_eq!(store.visible_batch_count(batch.trading_date).unwrap(), 0);
    }

    #[test]
    fn br160_preserves_provider_observation_text_and_rejects_blank_evidence() {
        let mut conn = connection();
        let mut original = batch();
        original.inputs[0].observed_at = "1784937600123456789".to_owned();
        ChainIntelligenceStore::new(&mut conn)
            .stage_batch(&original)
            .expect("stage provider-native timestamp");
        let row = diesel::sql_query(
            "SELECT observed_at FROM chain_intelligence_input_evidence WHERE input_id = ?",
        )
        .bind::<Text, _>(&original.inputs[0].input_id)
        .get_result::<ObservedAtRow>(&mut conn)
        .expect("stored input evidence");
        assert_eq!(row.observed_at, "1784937600123456789");

        let mut blank = batch();
        blank.batch_id = "TEST_CODE_chain_batch_blank_observed".to_owned();
        blank.inputs[0].input_id = "TEST_CODE_input_blank_observed".to_owned();
        blank.inputs[0].observed_at = "  ".to_owned();
        ChainIntelligenceStore::new(&mut conn)
            .stage_batch(&blank)
            .expect_err("blank provider observation must fail");
    }

    #[test]
    fn br160_tables_are_append_only() {
        let mut conn = connection();
        let batch = batch();
        ChainIntelligenceStore::new(&mut conn)
            .stage_batch(&batch)
            .expect("stage");
        diesel::sql_query(
            "UPDATE chain_intelligence_batches SET taxonomy_version = 'TEST_CODE_changed'",
        )
        .execute(&mut conn)
        .expect_err("update must be blocked");
        diesel::sql_query("DELETE FROM chain_intelligence_batches")
            .execute(&mut conn)
            .expect_err("delete must be blocked");
    }

    #[test]
    fn br160_visibility_rejects_unstaged_and_conflicting_receipts() {
        let mut conn = connection();
        let batch = batch();
        let mut store = ChainIntelligenceStore::new(&mut conn);
        let unstaged = visibility(&batch);
        assert!(matches!(
            store.publish(&unstaged),
            Err(ChainStoreError::InvalidInput(_))
        ));
        store.stage_batch(&batch).expect("stage");

        let mut wrong_content = visibility(&batch);
        wrong_content.content_hash = hash('f');
        assert!(matches!(
            store.publish(&wrong_content),
            Err(ChainStoreError::Conflict {
                entity: "visibility-content",
                ..
            })
        ));

        let receipt = visibility(&batch);
        assert!(store.publish(&receipt).expect("publish"));
        assert!(!store.publish(&receipt).expect("idempotent publish"));
        let mut conflicting_audit = receipt;
        conflicting_audit.audit_record_hash = hash('a');
        assert!(matches!(
            store.publish(&conflicting_audit),
            Err(ChainStoreError::Conflict {
                entity: "visibility",
                ..
            })
        ));
        assert!(matches!(
            store.load_visible_batch("  "),
            Err(ChainStoreError::InvalidInput(_))
        ));
    }

    #[test]
    fn br160_invalid_counts_duplicate_inputs_and_bad_hashes_fail_before_write() {
        for mutate in 0..3 {
            let mut conn = connection();
            let mut invalid = batch();
            match mutate {
                0 => invalid.chains[0].upper_limit_count = 4,
                1 => invalid.inputs.push(invalid.inputs[0].clone()),
                2 => invalid.rejections.push(ChainRejectionInput {
                    rejection_id: "TEST_CODE_REJECTION".to_owned(),
                    ordinal: 0,
                    identity_hash: "not-a-hash".to_owned(),
                    reason_code: "TEST_CODE_REJECTED".to_owned(),
                    retryable: false,
                    content_hash: hash('f'),
                }),
                _ => unreachable!(),
            }
            let error = ChainIntelligenceStore::new(&mut conn)
                .stage_batch(&invalid)
                .expect_err("invalid batch");
            assert!(matches!(error, ChainStoreError::InvalidInput(_)));
            let count = ChainIntelligenceStore::new(&mut conn)
                .visible_batch_count(invalid.trading_date)
                .expect("visible count");
            assert_eq!(count, 0);
        }
    }
}
