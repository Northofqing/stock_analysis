//! BR-155/BR-157: append-only persistence for event-scoped shadow selection.

use std::collections::BTreeSet;

use chrono::{DateTime, FixedOffset, NaiveDate, SecondsFormat};
use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel::sql_types::{Integer, Nullable, Text};
use thiserror::Error;

const TABLES: [&str; 7] = [
    "selection_event_inbox",
    "selection_event_completions",
    "selection_runs",
    "selection_candidates",
    "selection_feature_snapshots",
    "selection_outcomes",
    "selection_visibility_receipts",
];

#[derive(Debug, Error)]
pub enum SelectionStoreError {
    #[error("selection persistence conflict for {entity} identity={identity}")]
    Conflict {
        entity: &'static str,
        identity: String,
    },
    #[error("invalid selection persistence input: {0}")]
    InvalidInput(String),
    #[error("selection persistence database error: {0}")]
    Database(#[from] diesel::result::Error),
}

pub type SelectionStoreResult<T> = Result<T, SelectionStoreError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsertReceipt {
    pub inserted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageBatchReceipt {
    pub run_inserted: bool,
    pub candidates_inserted: usize,
    pub feature_snapshots_inserted: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboxEvent {
    pub event_id: String,
    pub content_hash: String,
    pub payload_json: String,
    pub provider: String,
    pub provider_published_at: Option<DateTime<FixedOffset>>,
    pub provider_published_on: Option<NaiveDate>,
    pub observed_at: DateTime<FixedOffset>,
    pub source_batch_id: String,
    pub source_batch_hash: String,
    pub evaluation_market_date: NaiveDate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionStatus {
    Completed,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventCompletion {
    pub completion_id: String,
    pub event_id: String,
    pub content_hash: String,
    pub status: CompletionStatus,
    pub reason_code: Option<String>,
    pub completed_at: DateTime<FixedOffset>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionRunInput {
    pub run_id: String,
    pub content_hash: String,
    pub evaluation_market_date: NaiveDate,
    pub config_hash: String,
    pub magic_tdx_batch_id: String,
    pub magic_tdx_batch_hash: String,
    pub created_at: DateTime<FixedOffset>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionCandidateInput {
    pub candidate_id: String,
    pub run_id: String,
    pub event_id: String,
    pub chain_id: String,
    pub stock_code: String,
    pub stock_name: String,
    pub relation_version: String,
    pub feature_version: String,
    pub ordinal: i32,
    pub content_hash: String,
    pub evaluation_market_date: NaiveDate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeatureSnapshotInput {
    pub feature_snapshot_id: String,
    pub candidate_id: String,
    pub content_hash: String,
    pub payload_json: String,
    pub source_batch_id: String,
    pub source_batch_hash: String,
    pub observed_at: DateTime<FixedOffset>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionBatchInput {
    pub run: SelectionRunInput,
    pub candidates: Vec<SelectionCandidateInput>,
    pub feature_snapshots: Vec<FeatureSnapshotInput>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisibilityReceiptInput {
    pub receipt_id: String,
    pub run_id: String,
    pub audit_record_hash: String,
    pub content_hash: String,
    pub published_at: DateTime<FixedOffset>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomePhase {
    T0Close,
    D1Settled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionOutcomeInput {
    pub outcome_id: String,
    pub candidate_id: String,
    pub phase: OutcomePhase,
    pub market_date: NaiveDate,
    pub content_hash: String,
    pub payload_json: String,
    pub observed_at: DateTime<FixedOffset>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueOutcome {
    pub candidate_id: String,
    pub stock_code: String,
    pub evaluation_market_date: NaiveDate,
    pub phase: OutcomePhase,
    pub due_market_date: NaiveDate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisibleSample {
    pub candidate_id: String,
    pub run_id: String,
    pub event_id: String,
    pub chain_id: String,
    pub stock_code: String,
    pub stock_name: String,
    pub evaluation_market_date: NaiveDate,
    pub feature_payload_json: String,
    pub t0_outcome_payload_json: Option<String>,
    pub d1_outcome_payload_json: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportFilter {
    pub from_market_date: Option<NaiveDate>,
    pub to_market_date: Option<NaiveDate>,
    pub stock_code: Option<String>,
    pub limit: usize,
}

impl Default for ReportFilter {
    fn default() -> Self {
        Self {
            from_market_date: None,
            to_market_date: None,
            stock_code: None,
            limit: 10_000,
        }
    }
}

const SELECTION_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS selection_event_inbox (
    event_id TEXT PRIMARY KEY,
    content_hash TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    provider TEXT NOT NULL,
    provider_published_at TEXT,
    provider_published_on TEXT,
    observed_at TEXT NOT NULL,
    source_batch_id TEXT NOT NULL,
    source_batch_hash TEXT NOT NULL,
    evaluation_market_date TEXT NOT NULL,
    ingested_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK (length(trim(event_id)) > 0),
    CHECK (length(trim(content_hash)) > 0),
    CHECK (length(trim(payload_json)) > 0),
    CHECK (length(trim(provider)) > 0),
    CHECK (length(trim(source_batch_id)) > 0),
    CHECK (length(trim(source_batch_hash)) > 0)
);

CREATE TABLE IF NOT EXISTS selection_event_completions (
    completion_id TEXT PRIMARY KEY,
    event_id TEXT NOT NULL UNIQUE REFERENCES selection_event_inbox(event_id),
    content_hash TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('completed', 'rejected')),
    reason_code TEXT,
    completed_at TEXT NOT NULL,
    recorded_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK (length(trim(completion_id)) > 0),
    CHECK (length(trim(content_hash)) > 0),
    CHECK (status = 'completed' OR (reason_code IS NOT NULL AND length(trim(reason_code)) > 0))
);

CREATE TABLE IF NOT EXISTS selection_runs (
    run_id TEXT PRIMARY KEY,
    content_hash TEXT NOT NULL,
    evaluation_market_date TEXT NOT NULL,
    config_hash TEXT NOT NULL,
    magic_tdx_batch_id TEXT NOT NULL,
    magic_tdx_batch_hash TEXT NOT NULL,
    created_at TEXT NOT NULL,
    recorded_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK (length(trim(run_id)) > 0),
    CHECK (length(trim(content_hash)) > 0),
    CHECK (length(trim(config_hash)) > 0),
    CHECK (length(trim(magic_tdx_batch_id)) > 0),
    CHECK (length(trim(magic_tdx_batch_hash)) > 0)
);

CREATE TABLE IF NOT EXISTS selection_candidates (
    candidate_id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES selection_runs(run_id),
    event_id TEXT NOT NULL REFERENCES selection_event_inbox(event_id),
    chain_id TEXT NOT NULL,
    stock_code TEXT NOT NULL,
    stock_name TEXT NOT NULL,
    relation_version TEXT NOT NULL,
    feature_version TEXT NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    content_hash TEXT NOT NULL,
    evaluation_market_date TEXT NOT NULL,
    recorded_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (run_id, event_id, chain_id, stock_code),
    CHECK (length(trim(candidate_id)) > 0),
    CHECK (length(trim(chain_id)) > 0),
    CHECK (__SELECTION_STOCK_CODE_CHECK__),
    CHECK (length(trim(stock_name)) > 0),
    CHECK (length(trim(relation_version)) > 0),
    CHECK (length(trim(feature_version)) > 0),
    CHECK (length(trim(content_hash)) > 0)
);

CREATE TABLE IF NOT EXISTS selection_feature_snapshots (
    feature_snapshot_id TEXT PRIMARY KEY,
    candidate_id TEXT NOT NULL UNIQUE REFERENCES selection_candidates(candidate_id),
    content_hash TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    source_batch_id TEXT NOT NULL,
    source_batch_hash TEXT NOT NULL,
    observed_at TEXT NOT NULL,
    recorded_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK (length(trim(feature_snapshot_id)) > 0),
    CHECK (length(trim(content_hash)) > 0),
    CHECK (length(trim(payload_json)) > 0),
    CHECK (length(trim(source_batch_id)) > 0),
    CHECK (length(trim(source_batch_hash)) > 0)
);

CREATE TABLE IF NOT EXISTS selection_outcomes (
    outcome_id TEXT PRIMARY KEY,
    candidate_id TEXT NOT NULL REFERENCES selection_candidates(candidate_id),
    phase TEXT NOT NULL CHECK (phase IN ('t0_close', 'd1_settled')),
    market_date TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    observed_at TEXT NOT NULL,
    recorded_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (candidate_id, phase),
    CHECK (length(trim(outcome_id)) > 0),
    CHECK (length(trim(content_hash)) > 0),
    CHECK (length(trim(payload_json)) > 0)
);

CREATE TABLE IF NOT EXISTS selection_visibility_receipts (
    receipt_id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL UNIQUE REFERENCES selection_runs(run_id),
    audit_record_hash TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    published_at TEXT NOT NULL,
    recorded_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK (length(trim(receipt_id)) > 0),
    CHECK (length(trim(audit_record_hash)) > 0),
    CHECK (length(trim(content_hash)) > 0)
);

CREATE INDEX IF NOT EXISTS idx_selection_event_pending
    ON selection_event_inbox(observed_at, event_id);
CREATE INDEX IF NOT EXISTS idx_selection_completion_event
    ON selection_event_completions(event_id);
CREATE INDEX IF NOT EXISTS idx_selection_candidate_run_date
    ON selection_candidates(run_id, evaluation_market_date, ordinal, candidate_id);
CREATE INDEX IF NOT EXISTS idx_selection_outcome_due
    ON selection_outcomes(candidate_id, phase, market_date);
CREATE INDEX IF NOT EXISTS idx_selection_visibility_run
    ON selection_visibility_receipts(run_id);
"#;

pub fn create_schema(conn: &mut SqliteConnection) -> Result<(), String> {
    let schema = SELECTION_SCHEMA.replace(
        "__SELECTION_STOCK_CODE_CHECK__",
        selection_stock_code_check(),
    );
    conn.batch_execute(&schema)
        .map_err(|error| error.to_string())?;
    for table in TABLES {
        for action in ["UPDATE", "DELETE"] {
            let suffix = action.to_ascii_lowercase();
            let sql = format!(
                "CREATE TRIGGER IF NOT EXISTS trg_{table}_no_{suffix}
                 BEFORE {action} ON {table}
                 BEGIN
                     SELECT RAISE(ABORT, 'BR-157 {table} is append-only');
                 END;"
            );
            conn.batch_execute(&sql)
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

impl CompletionStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Rejected => "rejected",
        }
    }
}

impl OutcomePhase {
    fn as_str(self) -> &'static str {
        match self {
            Self::T0Close => "t0_close",
            Self::D1Settled => "d1_settled",
        }
    }
}

#[derive(QueryableByName)]
struct HashRow {
    #[diesel(sql_type = Text)]
    content_hash: String,
}

#[derive(QueryableByName)]
struct DateRow {
    #[diesel(sql_type = Text)]
    evaluation_market_date: String,
}

#[derive(QueryableByName)]
struct InboxRow {
    #[diesel(sql_type = Text)]
    event_id: String,
    #[diesel(sql_type = Text)]
    content_hash: String,
    #[diesel(sql_type = Text)]
    payload_json: String,
    #[diesel(sql_type = Text)]
    provider: String,
    #[diesel(sql_type = Nullable<Text>)]
    provider_published_at: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    provider_published_on: Option<String>,
    #[diesel(sql_type = Text)]
    observed_at: String,
    #[diesel(sql_type = Text)]
    source_batch_id: String,
    #[diesel(sql_type = Text)]
    source_batch_hash: String,
    #[diesel(sql_type = Text)]
    evaluation_market_date: String,
}

impl InboxRow {
    fn into_event(self) -> SelectionStoreResult<InboxEvent> {
        Ok(InboxEvent {
            event_id: self.event_id,
            content_hash: self.content_hash,
            payload_json: self.payload_json,
            provider: self.provider,
            provider_published_at: self
                .provider_published_at
                .map(|value| parse_timestamp("provider_published_at", &value))
                .transpose()?,
            provider_published_on: self
                .provider_published_on
                .map(|value| parse_date("provider_published_on", &value))
                .transpose()?,
            observed_at: parse_timestamp("observed_at", &self.observed_at)?,
            source_batch_id: self.source_batch_id,
            source_batch_hash: self.source_batch_hash,
            evaluation_market_date: parse_date(
                "evaluation_market_date",
                &self.evaluation_market_date,
            )?,
        })
    }
}

#[derive(QueryableByName)]
struct DueRow {
    #[diesel(sql_type = Text)]
    candidate_id: String,
    #[diesel(sql_type = Text)]
    stock_code: String,
    #[diesel(sql_type = Text)]
    evaluation_market_date: String,
    #[diesel(sql_type = Integer)]
    has_t0: i32,
    #[diesel(sql_type = Integer)]
    has_d1: i32,
}

#[derive(QueryableByName)]
struct VisibleRow {
    #[diesel(sql_type = Text)]
    candidate_id: String,
    #[diesel(sql_type = Text)]
    run_id: String,
    #[diesel(sql_type = Text)]
    event_id: String,
    #[diesel(sql_type = Text)]
    chain_id: String,
    #[diesel(sql_type = Text)]
    stock_code: String,
    #[diesel(sql_type = Text)]
    stock_name: String,
    #[diesel(sql_type = Text)]
    evaluation_market_date: String,
    #[diesel(sql_type = Text)]
    feature_payload_json: String,
    #[diesel(sql_type = Nullable<Text>)]
    t0_outcome_payload_json: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    d1_outcome_payload_json: Option<String>,
}

impl VisibleRow {
    fn into_sample(self) -> SelectionStoreResult<VisibleSample> {
        Ok(VisibleSample {
            candidate_id: self.candidate_id,
            run_id: self.run_id,
            event_id: self.event_id,
            chain_id: self.chain_id,
            stock_code: self.stock_code,
            stock_name: self.stock_name,
            evaluation_market_date: parse_date(
                "visible evaluation_market_date",
                &self.evaluation_market_date,
            )?,
            feature_payload_json: self.feature_payload_json,
            t0_outcome_payload_json: self.t0_outcome_payload_json,
            d1_outcome_payload_json: self.d1_outcome_payload_json,
        })
    }
}

fn format_timestamp(value: DateTime<FixedOffset>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, false)
}

fn parse_timestamp(
    field: &'static str,
    value: &str,
) -> SelectionStoreResult<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(value).map_err(|error| {
        SelectionStoreError::InvalidInput(format!(
            "persisted {field} is invalid: {value:?}: {error}"
        ))
    })
}

fn parse_date(field: &'static str, value: &str) -> SelectionStoreResult<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|error| {
        SelectionStoreError::InvalidInput(format!(
            "persisted {field} is invalid: {value:?}: {error}"
        ))
    })
}

fn require_non_empty(field: &'static str, value: &str) -> SelectionStoreResult<()> {
    if value.trim().is_empty() {
        return Err(SelectionStoreError::InvalidInput(format!(
            "{field} must not be empty"
        )));
    }
    Ok(())
}

fn require_json(field: &'static str, value: &str) -> SelectionStoreResult<()> {
    require_non_empty(field, value)?;
    serde_json::from_str::<serde_json::Value>(value).map_err(|error| {
        SelectionStoreError::InvalidInput(format!("{field} must be valid JSON: {error}"))
    })?;
    Ok(())
}

#[cfg(not(test))]
fn selection_stock_code_check() -> &'static str {
    "length(stock_code) = 6 AND stock_code NOT GLOB '*[^0-9]*'"
}

#[cfg(test)]
fn selection_stock_code_check() -> &'static str {
    "stock_code GLOB 'TEST_CODE_[0-9][0-9][0-9][0-9][0-9][0-9]'"
}

#[cfg(not(test))]
fn is_valid_selection_stock_code(code: &str) -> bool {
    code.len() == 6 && code.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
fn is_valid_selection_stock_code(code: &str) -> bool {
    code.strip_prefix("TEST_CODE_")
        .is_some_and(|suffix| suffix.len() == 6 && suffix.bytes().all(|byte| byte.is_ascii_digit()))
}

fn validate_event(event: &InboxEvent) -> SelectionStoreResult<()> {
    for (field, value) in [
        ("event_id", event.event_id.as_str()),
        ("event content_hash", event.content_hash.as_str()),
        ("event provider", event.provider.as_str()),
        ("event source_batch_id", event.source_batch_id.as_str()),
        ("event source_batch_hash", event.source_batch_hash.as_str()),
    ] {
        require_non_empty(field, value)?;
    }
    if event.provider_published_at.is_some() && event.provider_published_on.is_some() {
        return Err(SelectionStoreError::InvalidInput(
            "provider publication must be either an exact timestamp or a date, not both".to_owned(),
        ));
    }
    require_json("event payload_json", &event.payload_json)
}

fn validate_completion(completion: &EventCompletion) -> SelectionStoreResult<()> {
    for (field, value) in [
        ("completion_id", completion.completion_id.as_str()),
        ("completion event_id", completion.event_id.as_str()),
        ("completion content_hash", completion.content_hash.as_str()),
    ] {
        require_non_empty(field, value)?;
    }
    if completion.status == CompletionStatus::Rejected {
        let reason = completion.reason_code.as_deref().ok_or_else(|| {
            SelectionStoreError::InvalidInput("rejected completion requires reason_code".to_owned())
        })?;
        require_non_empty("completion reason_code", reason)?;
    }
    Ok(())
}

fn validate_run(run: &SelectionRunInput) -> SelectionStoreResult<()> {
    for (field, value) in [
        ("run_id", run.run_id.as_str()),
        ("run content_hash", run.content_hash.as_str()),
        ("run config_hash", run.config_hash.as_str()),
        ("run magic_tdx_batch_id", run.magic_tdx_batch_id.as_str()),
        (
            "run magic_tdx_batch_hash",
            run.magic_tdx_batch_hash.as_str(),
        ),
    ] {
        require_non_empty(field, value)?;
    }
    Ok(())
}

fn validate_candidate(candidate: &SelectionCandidateInput) -> SelectionStoreResult<()> {
    for (field, value) in [
        ("candidate_id", candidate.candidate_id.as_str()),
        ("candidate run_id", candidate.run_id.as_str()),
        ("candidate event_id", candidate.event_id.as_str()),
        ("candidate chain_id", candidate.chain_id.as_str()),
        ("candidate stock_name", candidate.stock_name.as_str()),
        (
            "candidate relation_version",
            candidate.relation_version.as_str(),
        ),
        (
            "candidate feature_version",
            candidate.feature_version.as_str(),
        ),
        ("candidate content_hash", candidate.content_hash.as_str()),
    ] {
        require_non_empty(field, value)?;
    }
    if !is_valid_selection_stock_code(&candidate.stock_code) {
        return Err(SelectionStoreError::InvalidInput(format!(
            "candidate stock_code is invalid for this environment: {:?}",
            candidate.stock_code
        )));
    }
    if candidate.ordinal < 0 {
        return Err(SelectionStoreError::InvalidInput(
            "candidate ordinal must not be negative".to_owned(),
        ));
    }
    Ok(())
}

fn validate_feature(snapshot: &FeatureSnapshotInput) -> SelectionStoreResult<()> {
    for (field, value) in [
        ("feature_snapshot_id", snapshot.feature_snapshot_id.as_str()),
        ("feature candidate_id", snapshot.candidate_id.as_str()),
        ("feature content_hash", snapshot.content_hash.as_str()),
        ("feature source_batch_id", snapshot.source_batch_id.as_str()),
        (
            "feature source_batch_hash",
            snapshot.source_batch_hash.as_str(),
        ),
    ] {
        require_non_empty(field, value)?;
    }
    require_json("feature payload_json", &snapshot.payload_json)
}

fn validate_batch(batch: &SelectionBatchInput) -> SelectionStoreResult<()> {
    validate_run(&batch.run)?;
    let mut candidate_ids = BTreeSet::new();
    for candidate in &batch.candidates {
        validate_candidate(candidate)?;
        if candidate.run_id != batch.run.run_id {
            return Err(SelectionStoreError::InvalidInput(format!(
                "candidate {} belongs to run {}, expected {}",
                candidate.candidate_id, candidate.run_id, batch.run.run_id
            )));
        }
        if candidate.evaluation_market_date != batch.run.evaluation_market_date {
            return Err(SelectionStoreError::InvalidInput(format!(
                "candidate {} evaluation date {} differs from run date {}",
                candidate.candidate_id,
                candidate.evaluation_market_date,
                batch.run.evaluation_market_date
            )));
        }
        if !candidate_ids.insert(candidate.candidate_id.as_str()) {
            return Err(SelectionStoreError::InvalidInput(format!(
                "duplicate candidate identity in batch: {}",
                candidate.candidate_id
            )));
        }
    }

    let mut feature_candidate_ids = BTreeSet::new();
    for snapshot in &batch.feature_snapshots {
        validate_feature(snapshot)?;
        if snapshot.source_batch_id != batch.run.magic_tdx_batch_id
            || snapshot.source_batch_hash != batch.run.magic_tdx_batch_hash
        {
            return Err(SelectionStoreError::InvalidInput(format!(
                "feature {} source batch does not match run Magic TDX evidence",
                snapshot.feature_snapshot_id
            )));
        }
        if !candidate_ids.contains(snapshot.candidate_id.as_str()) {
            return Err(SelectionStoreError::InvalidInput(format!(
                "feature {} refers to candidate outside batch: {}",
                snapshot.feature_snapshot_id, snapshot.candidate_id
            )));
        }
        if !feature_candidate_ids.insert(snapshot.candidate_id.as_str()) {
            return Err(SelectionStoreError::InvalidInput(format!(
                "candidate {} has more than one feature snapshot",
                snapshot.candidate_id
            )));
        }
    }
    if feature_candidate_ids != candidate_ids {
        let missing = candidate_ids
            .difference(&feature_candidate_ids)
            .copied()
            .collect::<Vec<_>>();
        return Err(SelectionStoreError::InvalidInput(format!(
            "candidates missing feature snapshots: {missing:?}"
        )));
    }
    Ok(())
}

fn validate_visibility(receipt: &VisibilityReceiptInput) -> SelectionStoreResult<()> {
    for (field, value) in [
        ("visibility receipt_id", receipt.receipt_id.as_str()),
        ("visibility run_id", receipt.run_id.as_str()),
        (
            "visibility audit_record_hash",
            receipt.audit_record_hash.as_str(),
        ),
        ("visibility content_hash", receipt.content_hash.as_str()),
    ] {
        require_non_empty(field, value)?;
    }
    Ok(())
}

fn validate_outcome(outcome: &SelectionOutcomeInput) -> SelectionStoreResult<()> {
    for (field, value) in [
        ("outcome_id", outcome.outcome_id.as_str()),
        ("outcome candidate_id", outcome.candidate_id.as_str()),
        ("outcome content_hash", outcome.content_hash.as_str()),
    ] {
        require_non_empty(field, value)?;
    }
    require_json("outcome payload_json", &outcome.payload_json)
}

fn validate_report_filter(filter: &ReportFilter) -> SelectionStoreResult<()> {
    if filter.limit == 0 {
        return Err(SelectionStoreError::InvalidInput(
            "report limit must be greater than zero".to_owned(),
        ));
    }
    if let (Some(from), Some(to)) = (filter.from_market_date, filter.to_market_date) {
        if from > to {
            return Err(SelectionStoreError::InvalidInput(format!(
                "report from_market_date {from} is after to_market_date {to}"
            )));
        }
    }
    if let Some(code) = filter.stock_code.as_deref() {
        if !is_valid_selection_stock_code(code) {
            return Err(SelectionStoreError::InvalidInput(format!(
                "report stock_code is invalid for this environment: {code:?}"
            )));
        }
    }
    Ok(())
}

fn existing_hash(
    conn: &mut SqliteConnection,
    table: &'static str,
    identity_column: &'static str,
    identity: &str,
) -> SelectionStoreResult<Option<String>> {
    let query = format!("SELECT content_hash FROM {table} WHERE {identity_column} = ? LIMIT 1");
    Ok(diesel::sql_query(query)
        .bind::<Text, _>(identity)
        .get_result::<HashRow>(conn)
        .optional()?
        .map(|row| row.content_hash))
}

fn existing_outcome_hash(
    conn: &mut SqliteConnection,
    candidate_id: &str,
    phase: &str,
) -> SelectionStoreResult<Option<String>> {
    Ok(diesel::sql_query(
        "SELECT content_hash
         FROM selection_outcomes
         WHERE candidate_id = ? AND phase = ?
         LIMIT 1",
    )
    .bind::<Text, _>(candidate_id)
    .bind::<Text, _>(phase)
    .get_result::<HashRow>(conn)
    .optional()?
    .map(|row| row.content_hash))
}

fn candidate_evaluation_market_date(
    conn: &mut SqliteConnection,
    candidate_id: &str,
) -> SelectionStoreResult<Option<NaiveDate>> {
    diesel::sql_query(
        "SELECT evaluation_market_date
         FROM selection_candidates
         WHERE candidate_id = ?
         LIMIT 1",
    )
    .bind::<Text, _>(candidate_id)
    .get_result::<DateRow>(conn)
    .optional()?
    .map(|row| {
        parse_date(
            "candidate evaluation_market_date",
            &row.evaluation_market_date,
        )
    })
    .transpose()
}

fn idempotent_receipt(
    entity: &'static str,
    identity: &str,
    existing_hash: &str,
    requested_hash: &str,
) -> SelectionStoreResult<InsertReceipt> {
    if existing_hash == requested_hash {
        return Ok(InsertReceipt { inserted: false });
    }
    Err(SelectionStoreError::Conflict {
        entity,
        identity: identity.to_owned(),
    })
}

fn insert_run(conn: &mut SqliteConnection, run: &SelectionRunInput) -> SelectionStoreResult<bool> {
    if let Some(existing) = existing_hash(conn, "selection_runs", "run_id", &run.run_id)? {
        return idempotent_receipt("run", &run.run_id, &existing, &run.content_hash)
            .map(|receipt| receipt.inserted);
    }
    diesel::sql_query(
        "INSERT INTO selection_runs (
            run_id, content_hash, evaluation_market_date, config_hash,
            magic_tdx_batch_id, magic_tdx_batch_hash, created_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind::<Text, _>(&run.run_id)
    .bind::<Text, _>(&run.content_hash)
    .bind::<Text, _>(run.evaluation_market_date.to_string())
    .bind::<Text, _>(&run.config_hash)
    .bind::<Text, _>(&run.magic_tdx_batch_id)
    .bind::<Text, _>(&run.magic_tdx_batch_hash)
    .bind::<Text, _>(format_timestamp(run.created_at))
    .execute(conn)?;
    Ok(true)
}

fn insert_candidate(
    conn: &mut SqliteConnection,
    candidate: &SelectionCandidateInput,
) -> SelectionStoreResult<bool> {
    if let Some(existing) = existing_hash(
        conn,
        "selection_candidates",
        "candidate_id",
        &candidate.candidate_id,
    )? {
        return idempotent_receipt(
            "candidate",
            &candidate.candidate_id,
            &existing,
            &candidate.content_hash,
        )
        .map(|receipt| receipt.inserted);
    }
    diesel::sql_query(
        "INSERT INTO selection_candidates (
            candidate_id, run_id, event_id, chain_id, stock_code, stock_name,
            relation_version, feature_version, ordinal, content_hash,
            evaluation_market_date
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind::<Text, _>(&candidate.candidate_id)
    .bind::<Text, _>(&candidate.run_id)
    .bind::<Text, _>(&candidate.event_id)
    .bind::<Text, _>(&candidate.chain_id)
    .bind::<Text, _>(&candidate.stock_code)
    .bind::<Text, _>(&candidate.stock_name)
    .bind::<Text, _>(&candidate.relation_version)
    .bind::<Text, _>(&candidate.feature_version)
    .bind::<Integer, _>(candidate.ordinal)
    .bind::<Text, _>(&candidate.content_hash)
    .bind::<Text, _>(candidate.evaluation_market_date.to_string())
    .execute(conn)?;
    Ok(true)
}

fn insert_feature(
    conn: &mut SqliteConnection,
    snapshot: &FeatureSnapshotInput,
) -> SelectionStoreResult<bool> {
    if let Some(existing) = existing_hash(
        conn,
        "selection_feature_snapshots",
        "candidate_id",
        &snapshot.candidate_id,
    )? {
        return idempotent_receipt(
            "feature",
            &snapshot.candidate_id,
            &existing,
            &snapshot.content_hash,
        )
        .map(|receipt| receipt.inserted);
    }
    if let Some(existing) = existing_hash(
        conn,
        "selection_feature_snapshots",
        "feature_snapshot_id",
        &snapshot.feature_snapshot_id,
    )? {
        return idempotent_receipt(
            "feature",
            &snapshot.feature_snapshot_id,
            &existing,
            &snapshot.content_hash,
        )
        .map(|receipt| receipt.inserted);
    }
    diesel::sql_query(
        "INSERT INTO selection_feature_snapshots (
            feature_snapshot_id, candidate_id, content_hash, payload_json,
            source_batch_id, source_batch_hash, observed_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind::<Text, _>(&snapshot.feature_snapshot_id)
    .bind::<Text, _>(&snapshot.candidate_id)
    .bind::<Text, _>(&snapshot.content_hash)
    .bind::<Text, _>(&snapshot.payload_json)
    .bind::<Text, _>(&snapshot.source_batch_id)
    .bind::<Text, _>(&snapshot.source_batch_hash)
    .bind::<Text, _>(format_timestamp(snapshot.observed_at))
    .execute(conn)?;
    Ok(true)
}

pub struct SelectionRepository<'a> {
    conn: &'a mut SqliteConnection,
}

impl<'a> SelectionRepository<'a> {
    pub fn new(conn: &'a mut SqliteConnection) -> Self {
        Self { conn }
    }

    pub fn ingest_event(&mut self, event: &InboxEvent) -> SelectionStoreResult<InsertReceipt> {
        validate_event(event)?;
        if let Some(existing) = existing_hash(
            self.conn,
            "selection_event_inbox",
            "event_id",
            &event.event_id,
        )? {
            return idempotent_receipt("event", &event.event_id, &existing, &event.content_hash);
        }

        diesel::sql_query(
            "INSERT INTO selection_event_inbox (
                event_id, content_hash, payload_json, provider,
                provider_published_at, provider_published_on, observed_at,
                source_batch_id, source_batch_hash, evaluation_market_date
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind::<Text, _>(&event.event_id)
        .bind::<Text, _>(&event.content_hash)
        .bind::<Text, _>(&event.payload_json)
        .bind::<Text, _>(&event.provider)
        .bind::<Nullable<Text>, _>(event.provider_published_at.map(format_timestamp))
        .bind::<Nullable<Text>, _>(event.provider_published_on.map(|date| date.to_string()))
        .bind::<Text, _>(format_timestamp(event.observed_at))
        .bind::<Text, _>(&event.source_batch_id)
        .bind::<Text, _>(&event.source_batch_hash)
        .bind::<Text, _>(event.evaluation_market_date.to_string())
        .execute(self.conn)?;

        Ok(InsertReceipt { inserted: true })
    }

    pub fn pending_events(&mut self, limit: usize) -> SelectionStoreResult<Vec<InboxEvent>> {
        if limit == 0 {
            return Err(SelectionStoreError::InvalidInput(
                "pending event limit must be greater than zero".to_owned(),
            ));
        }
        let limit = i64::try_from(limit).map_err(|_| {
            SelectionStoreError::InvalidInput("pending event limit exceeds i64".to_owned())
        })?;
        diesel::sql_query(
            "SELECT
                inbox.event_id,
                inbox.content_hash,
                inbox.payload_json,
                inbox.provider,
                inbox.provider_published_at,
                inbox.provider_published_on,
                inbox.observed_at,
                inbox.source_batch_id,
                inbox.source_batch_hash,
                inbox.evaluation_market_date
             FROM selection_event_inbox AS inbox
             LEFT JOIN selection_event_completions AS completion
               ON completion.event_id = inbox.event_id
             WHERE completion.event_id IS NULL
             ORDER BY inbox.observed_at ASC, inbox.event_id ASC
             LIMIT ?",
        )
        .bind::<diesel::sql_types::BigInt, _>(limit)
        .load::<InboxRow>(self.conn)?
        .into_iter()
        .map(InboxRow::into_event)
        .collect()
    }

    pub fn stage_batch(
        &mut self,
        batch: &SelectionBatchInput,
    ) -> SelectionStoreResult<StageBatchReceipt> {
        validate_batch(batch)?;
        self.conn.transaction(|conn| {
            let run_inserted = insert_run(conn, &batch.run)?;
            let mut candidates_inserted = 0;
            for candidate in &batch.candidates {
                candidates_inserted += usize::from(insert_candidate(conn, candidate)?);
            }
            let mut feature_snapshots_inserted = 0;
            for snapshot in &batch.feature_snapshots {
                feature_snapshots_inserted += usize::from(insert_feature(conn, snapshot)?);
            }
            Ok(StageBatchReceipt {
                run_inserted,
                candidates_inserted,
                feature_snapshots_inserted,
            })
        })
    }

    pub fn publish_visibility(
        &mut self,
        receipt: &VisibilityReceiptInput,
    ) -> SelectionStoreResult<InsertReceipt> {
        validate_visibility(receipt)?;
        if let Some(existing) = existing_hash(
            self.conn,
            "selection_visibility_receipts",
            "run_id",
            &receipt.run_id,
        )? {
            return idempotent_receipt(
                "visibility",
                &receipt.run_id,
                &existing,
                &receipt.content_hash,
            );
        }
        if let Some(existing) = existing_hash(
            self.conn,
            "selection_visibility_receipts",
            "receipt_id",
            &receipt.receipt_id,
        )? {
            return idempotent_receipt(
                "visibility",
                &receipt.receipt_id,
                &existing,
                &receipt.content_hash,
            );
        }
        diesel::sql_query(
            "INSERT INTO selection_visibility_receipts (
                receipt_id, run_id, audit_record_hash, content_hash, published_at
             ) VALUES (?, ?, ?, ?, ?)",
        )
        .bind::<Text, _>(&receipt.receipt_id)
        .bind::<Text, _>(&receipt.run_id)
        .bind::<Text, _>(&receipt.audit_record_hash)
        .bind::<Text, _>(&receipt.content_hash)
        .bind::<Text, _>(format_timestamp(receipt.published_at))
        .execute(self.conn)?;
        Ok(InsertReceipt { inserted: true })
    }

    pub fn append_completion(
        &mut self,
        completion: &EventCompletion,
    ) -> SelectionStoreResult<InsertReceipt> {
        validate_completion(completion)?;
        if let Some(existing) = existing_hash(
            self.conn,
            "selection_event_completions",
            "event_id",
            &completion.event_id,
        )? {
            return idempotent_receipt(
                "completion",
                &completion.event_id,
                &existing,
                &completion.content_hash,
            );
        }
        if let Some(existing) = existing_hash(
            self.conn,
            "selection_event_completions",
            "completion_id",
            &completion.completion_id,
        )? {
            return idempotent_receipt(
                "completion",
                &completion.completion_id,
                &existing,
                &completion.content_hash,
            );
        }
        diesel::sql_query(
            "INSERT INTO selection_event_completions (
                completion_id, event_id, content_hash, status, reason_code, completed_at
             ) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind::<Text, _>(&completion.completion_id)
        .bind::<Text, _>(&completion.event_id)
        .bind::<Text, _>(&completion.content_hash)
        .bind::<Text, _>(completion.status.as_str())
        .bind::<Nullable<Text>, _>(completion.reason_code.as_deref())
        .bind::<Text, _>(format_timestamp(completion.completed_at))
        .execute(self.conn)?;
        Ok(InsertReceipt { inserted: true })
    }

    pub fn due_outcomes(&mut self, as_of: NaiveDate) -> SelectionStoreResult<Vec<DueOutcome>> {
        let rows = diesel::sql_query(
            "SELECT
                candidate.candidate_id,
                candidate.stock_code,
                candidate.evaluation_market_date,
                EXISTS (
                    SELECT 1 FROM selection_outcomes AS outcome
                    WHERE outcome.candidate_id = candidate.candidate_id
                      AND outcome.phase = 't0_close'
                ) AS has_t0,
                EXISTS (
                    SELECT 1 FROM selection_outcomes AS outcome
                    WHERE outcome.candidate_id = candidate.candidate_id
                      AND outcome.phase = 'd1_settled'
                ) AS has_d1
             FROM selection_candidates AS candidate
             INNER JOIN selection_visibility_receipts AS visibility
               ON visibility.run_id = candidate.run_id
             ORDER BY
                candidate.evaluation_market_date ASC,
                candidate.ordinal ASC,
                candidate.candidate_id ASC",
        )
        .load::<DueRow>(self.conn)?;

        let mut due = Vec::new();
        for row in rows {
            let evaluation_market_date = parse_date(
                "candidate evaluation_market_date",
                &row.evaluation_market_date,
            )?;
            if row.has_t0 == 0 {
                if evaluation_market_date <= as_of {
                    due.push(DueOutcome {
                        candidate_id: row.candidate_id,
                        stock_code: row.stock_code,
                        evaluation_market_date,
                        phase: OutcomePhase::T0Close,
                        due_market_date: evaluation_market_date,
                    });
                }
                continue;
            }
            if row.has_d1 == 0 {
                let due_market_date = crate::calendar::next_trading_day(evaluation_market_date);
                if due_market_date <= as_of {
                    due.push(DueOutcome {
                        candidate_id: row.candidate_id,
                        stock_code: row.stock_code,
                        evaluation_market_date,
                        phase: OutcomePhase::D1Settled,
                        due_market_date,
                    });
                }
            }
        }
        Ok(due)
    }

    pub fn append_outcome(
        &mut self,
        outcome: &SelectionOutcomeInput,
    ) -> SelectionStoreResult<InsertReceipt> {
        validate_outcome(outcome)?;
        let evaluation_market_date =
            candidate_evaluation_market_date(self.conn, &outcome.candidate_id)?.ok_or_else(
                || {
                    SelectionStoreError::InvalidInput(format!(
                        "outcome candidate does not exist: {}",
                        outcome.candidate_id
                    ))
                },
            )?;
        let expected_market_date = match outcome.phase {
            OutcomePhase::T0Close => evaluation_market_date,
            OutcomePhase::D1Settled => crate::calendar::next_trading_day(evaluation_market_date),
        };
        if outcome.market_date != expected_market_date {
            return Err(SelectionStoreError::InvalidInput(format!(
                "outcome {} phase={} market_date={} expected={expected_market_date}",
                outcome.outcome_id,
                outcome.phase.as_str(),
                outcome.market_date
            )));
        }
        if let Some(existing) =
            existing_outcome_hash(self.conn, &outcome.candidate_id, outcome.phase.as_str())?
        {
            return idempotent_receipt(
                "outcome",
                &format!("{}:{}", outcome.candidate_id, outcome.phase.as_str()),
                &existing,
                &outcome.content_hash,
            );
        }
        if let Some(existing) = existing_hash(
            self.conn,
            "selection_outcomes",
            "outcome_id",
            &outcome.outcome_id,
        )? {
            return idempotent_receipt(
                "outcome",
                &outcome.outcome_id,
                &existing,
                &outcome.content_hash,
            );
        }
        diesel::sql_query(
            "INSERT INTO selection_outcomes (
                outcome_id, candidate_id, phase, market_date,
                content_hash, payload_json, observed_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind::<Text, _>(&outcome.outcome_id)
        .bind::<Text, _>(&outcome.candidate_id)
        .bind::<Text, _>(outcome.phase.as_str())
        .bind::<Text, _>(outcome.market_date.to_string())
        .bind::<Text, _>(&outcome.content_hash)
        .bind::<Text, _>(&outcome.payload_json)
        .bind::<Text, _>(format_timestamp(outcome.observed_at))
        .execute(self.conn)?;
        Ok(InsertReceipt { inserted: true })
    }

    pub fn visible_samples(
        &mut self,
        filter: &ReportFilter,
    ) -> SelectionStoreResult<Vec<VisibleSample>> {
        validate_report_filter(filter)?;
        let from_market_date = filter.from_market_date.map(|date| date.to_string());
        let to_market_date = filter.to_market_date.map(|date| date.to_string());
        let stock_code = filter.stock_code.as_deref();
        let limit = i64::try_from(filter.limit).map_err(|_| {
            SelectionStoreError::InvalidInput("report limit exceeds i64".to_owned())
        })?;
        diesel::sql_query(
            "SELECT
                candidate.candidate_id,
                candidate.run_id,
                candidate.event_id,
                candidate.chain_id,
                candidate.stock_code,
                candidate.stock_name,
                candidate.evaluation_market_date,
                feature.payload_json AS feature_payload_json,
                t0.payload_json AS t0_outcome_payload_json,
                d1.payload_json AS d1_outcome_payload_json
             FROM selection_candidates AS candidate
             INNER JOIN selection_visibility_receipts AS visibility
               ON visibility.run_id = candidate.run_id
             INNER JOIN selection_feature_snapshots AS feature
               ON feature.candidate_id = candidate.candidate_id
             LEFT JOIN selection_outcomes AS t0
               ON t0.candidate_id = candidate.candidate_id
              AND t0.phase = 't0_close'
             LEFT JOIN selection_outcomes AS d1
               ON d1.candidate_id = candidate.candidate_id
              AND d1.phase = 'd1_settled'
             WHERE (? IS NULL OR candidate.evaluation_market_date >= ?)
               AND (? IS NULL OR candidate.evaluation_market_date <= ?)
               AND (? IS NULL OR candidate.stock_code = ?)
             ORDER BY
                candidate.evaluation_market_date ASC,
                candidate.ordinal ASC,
                candidate.candidate_id ASC
             LIMIT ?",
        )
        .bind::<Nullable<Text>, _>(from_market_date.as_deref())
        .bind::<Nullable<Text>, _>(from_market_date.as_deref())
        .bind::<Nullable<Text>, _>(to_market_date.as_deref())
        .bind::<Nullable<Text>, _>(to_market_date.as_deref())
        .bind::<Nullable<Text>, _>(stock_code)
        .bind::<Nullable<Text>, _>(stock_code)
        .bind::<diesel::sql_types::BigInt, _>(limit)
        .load::<VisibleRow>(self.conn)?
        .into_iter()
        .map(VisibleRow::into_sample)
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use diesel::connection::SimpleConnection;
    use diesel::sql_types::{BigInt, Text};

    #[derive(QueryableByName)]
    struct CountRow {
        #[diesel(sql_type = BigInt)]
        count: i64,
    }

    #[derive(QueryableByName)]
    struct NameRow {
        #[diesel(sql_type = Text)]
        name: String,
    }

    fn connection() -> SqliteConnection {
        let mut conn = SqliteConnection::establish(":memory:").expect("in-memory sqlite");
        conn.batch_execute("PRAGMA foreign_keys = ON;")
            .expect("foreign keys");
        crate::database::DatabaseManager::run_migrations_for_test(&mut conn)
            .expect("database migrations");
        conn
    }

    fn ts(hour: u32) -> DateTime<FixedOffset> {
        FixedOffset::east_opt(8 * 60 * 60)
            .expect("offset")
            .with_ymd_and_hms(2026, 7, 23, hour, 0, 0)
            .single()
            .expect("timestamp")
    }

    fn event(id: &str, hash: &str) -> InboxEvent {
        InboxEvent {
            event_id: id.to_owned(),
            content_hash: hash.to_owned(),
            payload_json: r#"{"headline":"产业事件"}"#.to_owned(),
            provider: "provider-a".to_owned(),
            provider_published_at: Some(ts(9)),
            provider_published_on: None,
            observed_at: ts(9),
            source_batch_id: "source-batch-1".to_owned(),
            source_batch_hash: "source-batch-hash-1".to_owned(),
            evaluation_market_date: NaiveDate::from_ymd_opt(2026, 7, 23).expect("date"),
        }
    }

    fn batch(run_id: &str, candidate_id: &str) -> SelectionBatchInput {
        let market_date = NaiveDate::from_ymd_opt(2026, 7, 23).expect("date");
        SelectionBatchInput {
            run: SelectionRunInput {
                run_id: run_id.to_owned(),
                content_hash: format!("{run_id}-hash"),
                evaluation_market_date: market_date,
                config_hash: "config-hash-1".to_owned(),
                magic_tdx_batch_id: "tdx-batch-1".to_owned(),
                magic_tdx_batch_hash: "tdx-batch-hash-1".to_owned(),
                created_at: ts(10),
            },
            candidates: vec![SelectionCandidateInput {
                candidate_id: candidate_id.to_owned(),
                run_id: run_id.to_owned(),
                event_id: "event-1".to_owned(),
                chain_id: "chain-power".to_owned(),
                stock_code: "TEST_CODE_600396".to_owned(),
                stock_name: "华电辽能".to_owned(),
                relation_version: "relation-v1".to_owned(),
                feature_version: "feature-v1".to_owned(),
                ordinal: 0,
                content_hash: format!("{candidate_id}-hash"),
                evaluation_market_date: market_date,
            }],
            feature_snapshots: vec![FeatureSnapshotInput {
                feature_snapshot_id: format!("feature-{candidate_id}"),
                candidate_id: candidate_id.to_owned(),
                content_hash: format!("feature-{candidate_id}-hash"),
                payload_json: r#"{"ma5":"12.30","volume_ratio":"1.42"}"#.to_owned(),
                source_batch_id: "tdx-batch-1".to_owned(),
                source_batch_hash: "tdx-batch-hash-1".to_owned(),
                observed_at: ts(10),
            }],
        }
    }

    fn seed_staged(conn: &mut SqliteConnection) {
        let mut repo = SelectionRepository::new(conn);
        repo.ingest_event(&event("event-1", "event-hash-1"))
            .expect("ingest event");
        repo.stage_batch(&batch("run-1", "candidate-1"))
            .expect("stage batch");
    }

    #[test]
    fn schema_contains_seven_append_only_tables() {
        let mut conn = connection();
        let tables = diesel::sql_query(
            "SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'selection_%' ORDER BY name",
        )
        .load::<NameRow>(&mut conn)
        .expect("list tables")
        .into_iter()
        .map(|row| row.name)
        .collect::<Vec<_>>();
        assert_eq!(
            tables,
            [
                "selection_candidates",
                "selection_event_completions",
                "selection_event_inbox",
                "selection_feature_snapshots",
                "selection_outcomes",
                "selection_runs",
                "selection_visibility_receipts",
            ]
        );

        let triggers = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM sqlite_master WHERE type='trigger' AND name LIKE 'trg_selection_%_no_%'",
        )
        .get_result::<CountRow>(&mut conn)
        .expect("count triggers")
        .count;
        assert_eq!(triggers, i64::try_from(TABLES.len() * 2).expect("count"));
    }

    #[test]
    fn event_identity_is_idempotent_but_conflicting_hash_is_rejected() {
        let mut conn = connection();
        let mut repo = SelectionRepository::new(&mut conn);
        assert!(
            repo.ingest_event(&event("event-1", "hash-a"))
                .expect("first insert")
                .inserted
        );
        assert!(
            !repo
                .ingest_event(&event("event-1", "hash-a"))
                .expect("idempotent insert")
                .inserted
        );

        let conflict = repo
            .ingest_event(&event("event-1", "hash-b"))
            .expect_err("conflicting payload must fail");
        assert!(matches!(
            conflict,
            SelectionStoreError::Conflict {
                entity: "event",
                ..
            }
        ));
    }

    #[test]
    fn pending_events_exclude_terminal_completion() {
        let mut conn = connection();
        let mut repo = SelectionRepository::new(&mut conn);
        repo.ingest_event(&event("event-1", "hash-a"))
            .expect("event 1");
        repo.ingest_event(&event("event-2", "hash-b"))
            .expect("event 2");
        repo.append_completion(&EventCompletion {
            completion_id: "completion-1".to_owned(),
            event_id: "event-1".to_owned(),
            content_hash: "completion-hash-1".to_owned(),
            status: CompletionStatus::Completed,
            reason_code: None,
            completed_at: ts(11),
        })
        .expect("completion");
        let duplicate = repo
            .append_completion(&EventCompletion {
                completion_id: "completion-1".to_owned(),
                event_id: "event-1".to_owned(),
                content_hash: "completion-hash-1".to_owned(),
                status: CompletionStatus::Completed,
                reason_code: None,
                completed_at: ts(11),
            })
            .expect("idempotent completion");
        assert!(!duplicate.inserted);

        let pending = repo.pending_events(20).expect("pending events");
        assert_eq!(
            pending
                .into_iter()
                .map(|item| item.event_id)
                .collect::<Vec<_>>(),
            ["event-2"]
        );
    }

    #[test]
    fn staged_candidates_are_hidden_until_authoritative_visibility_receipt() {
        let mut conn = connection();
        seed_staged(&mut conn);

        let mut repo = SelectionRepository::new(&mut conn);
        assert!(repo
            .visible_samples(&ReportFilter::default())
            .expect("hidden samples")
            .is_empty());
        repo.publish_visibility(&VisibilityReceiptInput {
            receipt_id: "visibility-1".to_owned(),
            run_id: "run-1".to_owned(),
            audit_record_hash: "audit-record-hash-1".to_owned(),
            content_hash: "visibility-hash-1".to_owned(),
            published_at: ts(11),
        })
        .expect("visibility");
        let duplicate_visibility = repo
            .publish_visibility(&VisibilityReceiptInput {
                receipt_id: "visibility-1".to_owned(),
                run_id: "run-1".to_owned(),
                audit_record_hash: "audit-record-hash-1".to_owned(),
                content_hash: "visibility-hash-1".to_owned(),
                published_at: ts(11),
            })
            .expect("idempotent visibility");
        assert!(!duplicate_visibility.inserted);

        let visible = repo
            .visible_samples(&ReportFilter::default())
            .expect("visible samples");
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].candidate_id, "candidate-1");
        assert_eq!(visible[0].stock_code, "TEST_CODE_600396");
        assert_eq!(
            visible[0].feature_payload_json,
            r#"{"ma5":"12.30","volume_ratio":"1.42"}"#
        );
        let excluded = repo
            .visible_samples(&ReportFilter {
                stock_code: Some("TEST_CODE_000001".to_owned()),
                ..ReportFilter::default()
            })
            .expect("filtered samples");
        assert!(excluded.is_empty());
    }

    #[test]
    fn stage_batch_rolls_back_run_when_child_identity_conflicts() {
        let mut conn = connection();
        seed_staged(&mut conn);
        let mut conflicting = batch("run-2", "candidate-1");
        conflicting.candidates[0].content_hash = "candidate-conflicting-hash".to_owned();

        let mut repo = SelectionRepository::new(&mut conn);
        assert!(matches!(
            repo.stage_batch(&conflicting),
            Err(SelectionStoreError::Conflict {
                entity: "candidate",
                ..
            })
        ));
        let count = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM selection_runs WHERE run_id = 'run-2'",
        )
        .get_result::<CountRow>(&mut conn)
        .expect("count run")
        .count;
        assert_eq!(count, 0, "parent run must roll back with child failure");
    }

    #[test]
    fn stage_batch_is_idempotent_and_foreign_key_failure_is_atomic() {
        let mut conn = connection();
        seed_staged(&mut conn);
        let mut repo = SelectionRepository::new(&mut conn);
        let duplicate = repo
            .stage_batch(&batch("run-1", "candidate-1"))
            .expect("idempotent stage");
        assert!(!duplicate.run_inserted);
        assert_eq!(duplicate.candidates_inserted, 0);
        assert_eq!(duplicate.feature_snapshots_inserted, 0);

        let mut orphan = batch("run-orphan", "candidate-orphan");
        orphan.candidates[0].event_id = "event-does-not-exist".to_owned();
        assert!(matches!(
            repo.stage_batch(&orphan),
            Err(SelectionStoreError::Database(_))
        ));
        let count = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM selection_runs WHERE run_id = 'run-orphan'",
        )
        .get_result::<CountRow>(&mut conn)
        .expect("count orphan run")
        .count;
        assert_eq!(count, 0, "foreign key failure must roll back run");
    }

    #[test]
    fn outcomes_are_due_only_for_visible_samples_and_advance_by_phase() {
        let mut conn = connection();
        seed_staged(&mut conn);
        let mut repo = SelectionRepository::new(&mut conn);
        assert!(repo
            .due_outcomes(NaiveDate::from_ymd_opt(2026, 7, 23).expect("date"))
            .expect("hidden due outcomes")
            .is_empty());
        repo.publish_visibility(&VisibilityReceiptInput {
            receipt_id: "visibility-1".to_owned(),
            run_id: "run-1".to_owned(),
            audit_record_hash: "audit-record-hash-1".to_owned(),
            content_hash: "visibility-hash-1".to_owned(),
            published_at: ts(11),
        })
        .expect("visibility");

        let t0_due = repo
            .due_outcomes(NaiveDate::from_ymd_opt(2026, 7, 23).expect("date"))
            .expect("t0 due");
        assert_eq!(t0_due.len(), 1);
        assert_eq!(t0_due[0].phase, OutcomePhase::T0Close);

        let invalid_date = repo
            .append_outcome(&SelectionOutcomeInput {
                outcome_id: "outcome-t0-invalid".to_owned(),
                candidate_id: "candidate-1".to_owned(),
                phase: OutcomePhase::T0Close,
                market_date: NaiveDate::from_ymd_opt(2026, 7, 24).expect("date"),
                content_hash: "outcome-t0-invalid-hash".to_owned(),
                payload_json: r#"{"close":"16.12"}"#.to_owned(),
                observed_at: ts(15),
            })
            .expect_err("wrong phase date must fail");
        assert!(matches!(invalid_date, SelectionStoreError::InvalidInput(_)));

        let t0 = SelectionOutcomeInput {
            outcome_id: "outcome-t0-1".to_owned(),
            candidate_id: "candidate-1".to_owned(),
            phase: OutcomePhase::T0Close,
            market_date: NaiveDate::from_ymd_opt(2026, 7, 23).expect("date"),
            content_hash: "outcome-t0-hash-1".to_owned(),
            payload_json: r#"{"close":"16.12"}"#.to_owned(),
            observed_at: ts(15),
        };
        repo.append_outcome(&t0).expect("t0 outcome");
        assert!(
            !repo
                .append_outcome(&t0)
                .expect("idempotent t0 outcome")
                .inserted
        );

        let next_day =
            crate::calendar::next_trading_day(NaiveDate::from_ymd_opt(2026, 7, 23).expect("date"));
        assert!(repo
            .due_outcomes(NaiveDate::from_ymd_opt(2026, 7, 23).expect("date"))
            .expect("not due before d1")
            .is_empty());
        let d1_due = repo.due_outcomes(next_day).expect("d1 due");
        assert_eq!(d1_due.len(), 1);
        assert_eq!(d1_due[0].phase, OutcomePhase::D1Settled);
        assert_eq!(d1_due[0].due_market_date, next_day);
    }

    #[test]
    fn append_only_trigger_rejects_direct_mutation() {
        let mut conn = connection();
        let mut repo = SelectionRepository::new(&mut conn);
        repo.ingest_event(&event("event-1", "hash-a"))
            .expect("event");

        let update = diesel::sql_query(
            "UPDATE selection_event_inbox SET provider = 'tampered' WHERE event_id = 'event-1'",
        )
        .execute(&mut conn)
        .expect_err("append-only update must fail");
        assert!(update.to_string().contains("append-only"));

        let delete =
            diesel::sql_query("DELETE FROM selection_event_inbox WHERE event_id = 'event-1'")
                .execute(&mut conn)
                .expect_err("append-only delete must fail");
        assert!(delete.to_string().contains("append-only"));
    }

    #[test]
    fn provider_publication_may_be_truly_absent_without_substitution() {
        let mut conn = connection();
        let mut missing = event("event-1", "hash-a");
        missing.provider_published_at = None;
        missing.provider_published_on = None;
        let mut repo = SelectionRepository::new(&mut conn);
        repo.ingest_event(&missing).expect("missing provider time");
        let persisted = repo.pending_events(1).expect("pending");
        assert_eq!(persisted[0].provider_published_at, None);
        assert_eq!(persisted[0].provider_published_on, None);
        assert_ne!(
            persisted[0].observed_at.with_timezone(&Utc),
            DateTime::<Utc>::UNIX_EPOCH
        );
    }
}
