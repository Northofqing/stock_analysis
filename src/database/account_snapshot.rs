//! BR-103 real-account snapshot persistence.

use chrono::{DateTime, FixedOffset, NaiveDate};
use diesel::prelude::*;
use serde::Deserialize;

use super::DatabaseManager;

const ACCOUNT_SNAPSHOT_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS real_account_snapshot (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    snapshot_date TEXT NOT NULL,
    evidence_class TEXT NOT NULL,
    environment TEXT NOT NULL,
    total_assets REAL NOT NULL,
    securities_market_value REAL NOT NULL,
    available_cash REAL NOT NULL,
    withdrawable_cash REAL,
    holding_pnl REAL,
    daily_pnl REAL,
    daily_pnl_status TEXT NOT NULL,
    position_ratio_pct REAL,
    source_provider TEXT NOT NULL,
    source_account_type TEXT NOT NULL,
    ownership_attestation TEXT NOT NULL,
    currency TEXT NOT NULL,
    source_captured_at TEXT NOT NULL,
    observed_at TEXT NOT NULL,
    account_mode TEXT,
    account_ref TEXT,
    account_ref_status TEXT NOT NULL,
    evidence_sha256 TEXT NOT NULL UNIQUE,
    recorded_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK (total_assets >= 0),
    CHECK (securities_market_value >= 0),
    CHECK (available_cash >= 0),
    CHECK (withdrawable_cash IS NULL OR withdrawable_cash >= 0),
    CHECK (position_ratio_pct IS NULL OR (position_ratio_pct >= 0 AND position_ratio_pct <= 100))
);
CREATE INDEX IF NOT EXISTS idx_real_account_snapshot_observed
    ON real_account_snapshot(observed_at DESC, id DESC);
CREATE TRIGGER IF NOT EXISTS trg_real_account_snapshot_no_update
BEFORE UPDATE ON real_account_snapshot
BEGIN SELECT RAISE(ABORT, 'BR-103 real_account_snapshot is immutable'); END;
CREATE TRIGGER IF NOT EXISTS trg_real_account_snapshot_no_delete
BEFORE DELETE ON real_account_snapshot
BEGIN SELECT RAISE(ABORT, 'BR-103 real_account_snapshot retention is at least five years'); END;
"#;

#[derive(Debug, Clone, PartialEq)]
pub struct AccountSnapshotInput {
    pub snapshot_date: NaiveDate,
    pub evidence_class: String,
    pub environment: String,
    pub total_assets: f64,
    pub securities_market_value: f64,
    pub available_cash: f64,
    pub withdrawable_cash: Option<f64>,
    pub holding_pnl: Option<f64>,
    pub daily_pnl: Option<f64>,
    pub daily_pnl_status: String,
    pub position_ratio_pct: Option<f64>,
    pub source_provider: String,
    pub source_account_type: String,
    pub ownership_attestation: String,
    pub currency: String,
    pub source_captured_at: DateTime<FixedOffset>,
    pub observed_at: DateTime<FixedOffset>,
    pub account_mode: Option<String>,
    pub account_ref: Option<String>,
    pub account_ref_status: String,
    pub evidence_sha256: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AccountSnapshot {
    pub id: i64,
    pub snapshot_date: NaiveDate,
    pub evidence_class: String,
    pub environment: String,
    pub total_assets: f64,
    pub securities_market_value: f64,
    pub available_cash: f64,
    pub withdrawable_cash: Option<f64>,
    pub holding_pnl: Option<f64>,
    pub daily_pnl: Option<f64>,
    pub daily_pnl_status: String,
    pub position_ratio_pct: Option<f64>,
    pub source_provider: String,
    pub source_account_type: String,
    pub ownership_attestation: String,
    pub currency: String,
    pub source_captured_at: DateTime<FixedOffset>,
    pub observed_at: DateTime<FixedOffset>,
    pub account_mode: Option<String>,
    pub account_ref: Option<String>,
    pub account_ref_status: String,
    pub evidence_sha256: String,
    pub recorded_at: String,
}

impl AccountSnapshot {
    /// AGENTS 2.4 / BR-103: account facts authorize actions for at most 30 seconds.
    pub fn validate_fresh_for_action(&self, now: DateTime<FixedOffset>) -> Result<(), String> {
        let age_ms = now
            .signed_duration_since(self.observed_at)
            .num_milliseconds();
        if age_ms < 0 {
            return Err(format!(
                "BR-103 account snapshot is from the future: age_ms={age_ms}"
            ));
        }
        if age_ms > 30_000 {
            return Err(format!(
                "BR-103 account snapshot is stale: age_ms={age_ms} max_ms=30000"
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SaveAccountSnapshotReceipt {
    pub id: i64,
    pub inserted: bool,
}

#[derive(Deserialize)]
struct AccountSnapshotEvidenceFile {
    schema_version: u32,
    evidence_class: String,
    environment: String,
    source: AccountSnapshotEvidenceSource,
    recorded_at: String,
    account: AccountSnapshotEvidenceAccount,
}

#[derive(Deserialize)]
struct AccountSnapshotEvidenceSource {
    provider: String,
    account_type: String,
    capture_time: String,
    ownership_attestation: String,
    original_image_sha256: String,
}

#[derive(Deserialize)]
struct AccountSnapshotEvidenceAccount {
    account_ref_status: String,
    #[serde(default)]
    account_ref: Option<String>,
    currency: String,
    total_assets: f64,
    securities_market_value: f64,
    available_cash: f64,
    #[serde(default)]
    withdrawable_cash: Option<f64>,
    #[serde(default)]
    holding_pnl: Option<f64>,
    daily_pnl_status: String,
    #[serde(default)]
    daily_pnl: Option<f64>,
    #[serde(default)]
    position_ratio_pct: Option<f64>,
    #[serde(default)]
    account_mode: Option<String>,
}

/// Parse the ignored local evidence manifest without copying image bytes or
/// inventing fields. The caller still chooses the destination database.
pub fn account_snapshot_input_from_json(json: &str) -> Result<AccountSnapshotInput, String> {
    let evidence: AccountSnapshotEvidenceFile = serde_json::from_str(json)
        .map_err(|error| format!("BR-103 account evidence JSON is invalid: {error}"))?;
    if evidence.schema_version == 0 {
        return Err("BR-103 account evidence schema_version must be positive".to_string());
    }
    let source_captured_at = DateTime::parse_from_rfc3339(&evidence.source.capture_time)
        .map_err(|error| format!("BR-103 source capture_time is invalid: {error}"))?;
    let observed_at = DateTime::parse_from_rfc3339(&evidence.recorded_at)
        .map_err(|error| format!("BR-103 evidence recorded_at is invalid: {error}"))?;
    let input = AccountSnapshotInput {
        snapshot_date: source_captured_at.date_naive(),
        evidence_class: evidence.evidence_class,
        environment: evidence.environment,
        total_assets: evidence.account.total_assets,
        securities_market_value: evidence.account.securities_market_value,
        available_cash: evidence.account.available_cash,
        withdrawable_cash: evidence.account.withdrawable_cash,
        holding_pnl: evidence.account.holding_pnl,
        daily_pnl: evidence.account.daily_pnl,
        daily_pnl_status: evidence.account.daily_pnl_status,
        position_ratio_pct: evidence.account.position_ratio_pct,
        source_provider: evidence.source.provider,
        source_account_type: evidence.source.account_type,
        ownership_attestation: evidence.source.ownership_attestation,
        currency: evidence.account.currency,
        source_captured_at,
        observed_at,
        account_mode: evidence.account.account_mode,
        account_ref: evidence.account.account_ref,
        account_ref_status: evidence.account.account_ref_status,
        evidence_sha256: evidence.source.original_image_sha256,
    };
    validate_input(&input)?;
    Ok(input)
}

#[derive(QueryableByName)]
struct AccountSnapshotRow {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    id: i64,
    #[diesel(sql_type = diesel::sql_types::Text)]
    snapshot_date: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    evidence_class: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    environment: String,
    #[diesel(sql_type = diesel::sql_types::Double)]
    total_assets: f64,
    #[diesel(sql_type = diesel::sql_types::Double)]
    securities_market_value: f64,
    #[diesel(sql_type = diesel::sql_types::Double)]
    available_cash: f64,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    withdrawable_cash: Option<f64>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    holding_pnl: Option<f64>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    daily_pnl: Option<f64>,
    #[diesel(sql_type = diesel::sql_types::Text)]
    daily_pnl_status: String,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    position_ratio_pct: Option<f64>,
    #[diesel(sql_type = diesel::sql_types::Text)]
    source_provider: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    source_account_type: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    ownership_attestation: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    currency: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    source_captured_at: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    observed_at: String,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    account_mode: Option<String>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    account_ref: Option<String>,
    #[diesel(sql_type = diesel::sql_types::Text)]
    account_ref_status: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    evidence_sha256: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    recorded_at: String,
}

pub(crate) fn create_schema(conn: &mut SqliteConnection) -> Result<(), Box<dyn std::error::Error>> {
    use diesel::connection::SimpleConnection;
    conn.batch_execute(ACCOUNT_SNAPSHOT_SCHEMA)?;
    Ok(())
}

fn validate_input(input: &AccountSnapshotInput) -> Result<(), String> {
    for (field, value) in [
        ("evidence_class", input.evidence_class.as_str()),
        ("source_provider", input.source_provider.as_str()),
        ("source_account_type", input.source_account_type.as_str()),
        (
            "ownership_attestation",
            input.ownership_attestation.as_str(),
        ),
        ("daily_pnl_status", input.daily_pnl_status.as_str()),
        ("account_ref_status", input.account_ref_status.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(format!("BR-103 {field} is required"));
        }
    }
    if input.environment != "live" {
        return Err("BR-103 real account snapshot environment must be live".to_string());
    }
    if input.currency != "CNY" {
        return Err(format!(
            "BR-103 unsupported account snapshot currency: {}",
            input.currency
        ));
    }
    for (field, value) in [
        ("total_assets", input.total_assets),
        ("securities_market_value", input.securities_market_value),
        ("available_cash", input.available_cash),
    ] {
        if !value.is_finite() || value < 0.0 {
            return Err(format!("BR-103 {field} must be finite and non-negative"));
        }
    }
    for (field, value) in [
        ("withdrawable_cash", input.withdrawable_cash),
        ("holding_pnl", input.holding_pnl),
        ("daily_pnl", input.daily_pnl),
        ("position_ratio_pct", input.position_ratio_pct),
    ] {
        if value.is_some_and(|value| !value.is_finite()) {
            return Err(format!("BR-103 {field} must be finite when present"));
        }
    }
    let accounted = input.securities_market_value + input.available_cash;
    if (input.total_assets - accounted).abs() > 0.01 {
        return Err(format!(
            "BR-103 account total mismatch: total={} market={} cash={}",
            input.total_assets, input.securities_market_value, input.available_cash
        ));
    }
    if input
        .withdrawable_cash
        .is_some_and(|value| value < 0.0 || value - input.available_cash > 0.01)
    {
        return Err("BR-103 withdrawable_cash exceeds available_cash".to_string());
    }
    if let Some(ratio) = input.position_ratio_pct {
        if !(0.0..=100.0).contains(&ratio) {
            return Err("BR-103 position_ratio_pct is outside 0..=100".to_string());
        }
        if input.total_assets <= 0.0 {
            return Err("BR-103 position ratio requires positive total assets".to_string());
        }
        let computed = input.securities_market_value / input.total_assets * 100.0;
        if (ratio - computed).abs() > 0.1 {
            return Err(format!(
                "BR-103 position ratio mismatch: supplied={ratio} computed={computed}"
            ));
        }
    }
    if input.observed_at < input.source_captured_at {
        return Err("BR-103 observed_at precedes source_captured_at".to_string());
    }
    if input.snapshot_date != input.source_captured_at.date_naive() {
        return Err("BR-103 snapshot_date differs from source date".to_string());
    }
    if input.evidence_sha256.len() != 64
        || !input
            .evidence_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("BR-103 evidence_sha256 must be 64 lowercase hex characters".to_string());
    }
    if let Some(mode) = input.account_mode.as_deref() {
        if !matches!(mode, "Normal" | "ReduceOnly" | "Frozen") {
            return Err(format!("BR-103 account_mode is invalid: {mode}"));
        }
    }
    if input
        .account_ref
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        return Err("BR-103 account_ref is blank when present".to_string());
    }
    Ok(())
}

const SELECT_COLUMNS: &str = "id, snapshot_date, evidence_class, environment, total_assets, \
     securities_market_value, available_cash, withdrawable_cash, holding_pnl, daily_pnl, \
     daily_pnl_status, position_ratio_pct, source_provider, source_account_type, \
     ownership_attestation, currency, source_captured_at, observed_at, account_mode, \
     account_ref, account_ref_status, evidence_sha256, recorded_at";

fn convert_row(row: AccountSnapshotRow) -> Result<AccountSnapshot, String> {
    let snapshot_date = NaiveDate::parse_from_str(&row.snapshot_date, "%Y-%m-%d")
        .map_err(|error| format!("BR-103 stored snapshot_date is invalid: {error}"))?;
    let source_captured_at = DateTime::parse_from_rfc3339(&row.source_captured_at)
        .map_err(|error| format!("BR-103 stored source_captured_at is invalid: {error}"))?;
    let observed_at = DateTime::parse_from_rfc3339(&row.observed_at)
        .map_err(|error| format!("BR-103 stored observed_at is invalid: {error}"))?;
    let snapshot = AccountSnapshot {
        id: row.id,
        snapshot_date,
        evidence_class: row.evidence_class,
        environment: row.environment,
        total_assets: row.total_assets,
        securities_market_value: row.securities_market_value,
        available_cash: row.available_cash,
        withdrawable_cash: row.withdrawable_cash,
        holding_pnl: row.holding_pnl,
        daily_pnl: row.daily_pnl,
        daily_pnl_status: row.daily_pnl_status,
        position_ratio_pct: row.position_ratio_pct,
        source_provider: row.source_provider,
        source_account_type: row.source_account_type,
        ownership_attestation: row.ownership_attestation,
        currency: row.currency,
        source_captured_at,
        observed_at,
        account_mode: row.account_mode,
        account_ref: row.account_ref,
        account_ref_status: row.account_ref_status,
        evidence_sha256: row.evidence_sha256,
        recorded_at: row.recorded_at,
    };
    validate_input(&AccountSnapshotInput::from(&snapshot))?;
    Ok(snapshot)
}

impl From<&AccountSnapshot> for AccountSnapshotInput {
    fn from(snapshot: &AccountSnapshot) -> Self {
        Self {
            snapshot_date: snapshot.snapshot_date,
            evidence_class: snapshot.evidence_class.clone(),
            environment: snapshot.environment.clone(),
            total_assets: snapshot.total_assets,
            securities_market_value: snapshot.securities_market_value,
            available_cash: snapshot.available_cash,
            withdrawable_cash: snapshot.withdrawable_cash,
            holding_pnl: snapshot.holding_pnl,
            daily_pnl: snapshot.daily_pnl,
            daily_pnl_status: snapshot.daily_pnl_status.clone(),
            position_ratio_pct: snapshot.position_ratio_pct,
            source_provider: snapshot.source_provider.clone(),
            source_account_type: snapshot.source_account_type.clone(),
            ownership_attestation: snapshot.ownership_attestation.clone(),
            currency: snapshot.currency.clone(),
            source_captured_at: snapshot.source_captured_at,
            observed_at: snapshot.observed_at,
            account_mode: snapshot.account_mode.clone(),
            account_ref: snapshot.account_ref.clone(),
            account_ref_status: snapshot.account_ref_status.clone(),
            evidence_sha256: snapshot.evidence_sha256.clone(),
        }
    }
}

fn snapshot_matches_input(snapshot: &AccountSnapshot, input: &AccountSnapshotInput) -> bool {
    AccountSnapshotInput::from(snapshot) == *input
}

pub(crate) fn save_account_snapshot_with_conn(
    conn: &mut SqliteConnection,
    input: &AccountSnapshotInput,
) -> Result<SaveAccountSnapshotReceipt, String> {
    validate_input(input)?;
    let snapshot_date = input.snapshot_date.to_string();
    let source_captured_at = input.source_captured_at.to_rfc3339();
    let observed_at = input.observed_at.to_rfc3339();
    let inserted = diesel::sql_query(
        "INSERT OR IGNORE INTO real_account_snapshot \
         (snapshot_date, evidence_class, environment, total_assets, securities_market_value, \
          available_cash, withdrawable_cash, holding_pnl, daily_pnl, daily_pnl_status, \
          position_ratio_pct, source_provider, source_account_type, ownership_attestation, \
          currency, source_captured_at, observed_at, account_mode, account_ref, \
          account_ref_status, evidence_sha256) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind::<diesel::sql_types::Text, _>(&snapshot_date)
    .bind::<diesel::sql_types::Text, _>(&input.evidence_class)
    .bind::<diesel::sql_types::Text, _>(&input.environment)
    .bind::<diesel::sql_types::Double, _>(input.total_assets)
    .bind::<diesel::sql_types::Double, _>(input.securities_market_value)
    .bind::<diesel::sql_types::Double, _>(input.available_cash)
    .bind::<diesel::sql_types::Nullable<diesel::sql_types::Double>, _>(input.withdrawable_cash)
    .bind::<diesel::sql_types::Nullable<diesel::sql_types::Double>, _>(input.holding_pnl)
    .bind::<diesel::sql_types::Nullable<diesel::sql_types::Double>, _>(input.daily_pnl)
    .bind::<diesel::sql_types::Text, _>(&input.daily_pnl_status)
    .bind::<diesel::sql_types::Nullable<diesel::sql_types::Double>, _>(input.position_ratio_pct)
    .bind::<diesel::sql_types::Text, _>(&input.source_provider)
    .bind::<diesel::sql_types::Text, _>(&input.source_account_type)
    .bind::<diesel::sql_types::Text, _>(&input.ownership_attestation)
    .bind::<diesel::sql_types::Text, _>(&input.currency)
    .bind::<diesel::sql_types::Text, _>(&source_captured_at)
    .bind::<diesel::sql_types::Text, _>(&observed_at)
    .bind::<diesel::sql_types::Nullable<diesel::sql_types::Text>, _>(input.account_mode.as_deref())
    .bind::<diesel::sql_types::Nullable<diesel::sql_types::Text>, _>(input.account_ref.as_deref())
    .bind::<diesel::sql_types::Text, _>(&input.account_ref_status)
    .bind::<diesel::sql_types::Text, _>(&input.evidence_sha256)
    .execute(conn)
    .map_err(|error| format!("BR-103 save account snapshot: {error}"))?
        == 1;

    let snapshot = get_account_snapshot_by_evidence_with_conn(conn, &input.evidence_sha256)?
        .ok_or_else(|| "BR-103 inserted snapshot cannot be reloaded".to_string())?;
    if !snapshot_matches_input(&snapshot, input) {
        return Err("BR-103 evidence hash conflicts with different account facts".to_string());
    }
    Ok(SaveAccountSnapshotReceipt {
        id: snapshot.id,
        inserted,
    })
}

fn get_account_snapshot_by_evidence_with_conn(
    conn: &mut SqliteConnection,
    evidence_sha256: &str,
) -> Result<Option<AccountSnapshot>, String> {
    let query = format!(
        "SELECT {SELECT_COLUMNS} FROM real_account_snapshot WHERE evidence_sha256 = ? LIMIT 1"
    );
    let row = diesel::sql_query(query)
        .bind::<diesel::sql_types::Text, _>(evidence_sha256)
        .get_result::<AccountSnapshotRow>(conn)
        .optional()
        .map_err(|error| format!("BR-103 query account snapshot evidence: {error}"))?;
    row.map(convert_row).transpose()
}

pub(crate) fn get_account_snapshot_with_conn(
    conn: &mut SqliteConnection,
    id: i64,
) -> Result<Option<AccountSnapshot>, String> {
    let query = format!("SELECT {SELECT_COLUMNS} FROM real_account_snapshot WHERE id = ? LIMIT 1");
    let row = diesel::sql_query(query)
        .bind::<diesel::sql_types::BigInt, _>(id)
        .get_result::<AccountSnapshotRow>(conn)
        .optional()
        .map_err(|error| format!("BR-103 query account snapshot: {error}"))?;
    row.map(convert_row).transpose()
}

pub fn save_account_snapshot(
    input: &AccountSnapshotInput,
) -> Result<SaveAccountSnapshotReceipt, String> {
    let db = DatabaseManager::try_get()
        .ok_or_else(|| "BR-103 DB is not initialized for account snapshot".to_string())?;
    let mut conn = db
        .get_conn()
        .map_err(|error| format!("BR-103 account snapshot DB connection: {error}"))?;
    save_account_snapshot_with_conn(&mut conn, input)
}

pub fn get_account_snapshot(id: i64) -> Result<Option<AccountSnapshot>, String> {
    let db = DatabaseManager::try_get()
        .ok_or_else(|| "BR-103 DB is not initialized for account snapshot".to_string())?;
    let mut conn = db
        .get_conn()
        .map_err(|error| format!("BR-103 account snapshot DB connection: {error}"))?;
    get_account_snapshot_with_conn(&mut conn, id)
}

pub fn latest_account_snapshot() -> Result<Option<AccountSnapshot>, String> {
    let db = DatabaseManager::try_get()
        .ok_or_else(|| "BR-103 DB is not initialized for account snapshot".to_string())?;
    let mut conn = db
        .get_conn()
        .map_err(|error| format!("BR-103 account snapshot DB connection: {error}"))?;
    let query = format!(
        "SELECT {SELECT_COLUMNS} FROM real_account_snapshot ORDER BY observed_at DESC, id DESC LIMIT 1"
    );
    let row = diesel::sql_query(query)
        .get_result::<AccountSnapshotRow>(&mut conn)
        .optional()
        .map_err(|error| format!("BR-103 query latest account snapshot: {error}"))?;
    row.map(convert_row).transpose()
}

#[cfg(test)]
mod tests {
    use super::{
        account_snapshot_input_from_json, create_schema, get_account_snapshot,
        get_account_snapshot_with_conn, latest_account_snapshot, save_account_snapshot,
        save_account_snapshot_with_conn, AccountSnapshotInput,
    };
    use chrono::{DateTime, Duration, FixedOffset};
    use diesel::connection::SimpleConnection;
    use diesel::{Connection, RunQueryDsl, SqliteConnection};

    fn ts(value: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(value).expect("valid RFC3339 fixture")
    }

    fn valid_input() -> AccountSnapshotInput {
        AccountSnapshotInput {
            snapshot_date: chrono::NaiveDate::from_ymd_opt(2026, 7, 18).unwrap(),
            evidence_class: "TEST_CODE_live_account_snapshot".to_string(),
            environment: "live".to_string(),
            total_assets: 100_000.0,
            securities_market_value: 80_000.0,
            available_cash: 20_000.0,
            withdrawable_cash: Some(19_000.0),
            holding_pnl: Some(-5_000.0),
            daily_pnl: None,
            daily_pnl_status: "unavailable".to_string(),
            position_ratio_pct: Some(80.0),
            source_provider: "TEST_CODE_broker_screenshot".to_string(),
            source_account_type: "TEST_CODE_cash_equity".to_string(),
            ownership_attestation: "TEST_CODE_user_attested".to_string(),
            currency: "CNY".to_string(),
            source_captured_at: ts("2026-07-18T17:38:00+08:00"),
            observed_at: ts("2026-07-18T17:38:01+08:00"),
            account_mode: None,
            account_ref: None,
            account_ref_status: "not_provided".to_string(),
            evidence_sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                .to_string(),
        }
    }

    fn connection() -> SqliteConnection {
        let mut conn = SqliteConnection::establish(":memory:").expect("in-memory sqlite");
        conn.batch_execute("PRAGMA foreign_keys = ON")
            .expect("foreign keys");
        create_schema(&mut conn).expect("snapshot schema");
        conn
    }

    #[test]
    fn nullable_same_day_snapshot_round_trips_and_duplicate_is_idempotent() {
        let mut conn = connection();
        let input = valid_input();
        let first = save_account_snapshot_with_conn(&mut conn, &input).expect("first insert");
        let duplicate =
            save_account_snapshot_with_conn(&mut conn, &input).expect("idempotent duplicate");
        assert_eq!(duplicate.id, first.id);
        assert!(first.inserted);
        assert!(!duplicate.inserted);

        let saved = get_account_snapshot_with_conn(&mut conn, first.id)
            .expect("query succeeds")
            .expect("row exists");
        assert_eq!(saved.snapshot_date, input.snapshot_date);
        assert_eq!(saved.total_assets, 100_000.0);
        assert_eq!(saved.securities_market_value, 80_000.0);
        assert_eq!(saved.available_cash, 20_000.0);
        assert_eq!(saved.daily_pnl, None);
        assert_eq!(saved.account_mode, None);
        assert_eq!(saved.source_provider, input.source_provider);
        assert_eq!(saved.source_captured_at, input.source_captured_at);
        assert_eq!(saved.observed_at, input.observed_at);
    }

    #[test]
    fn validation_rejects_bad_numbers_totals_provenance_and_time() {
        let cases = [
            ("non-finite", {
                let mut value = valid_input();
                value.total_assets = f64::NAN;
                value
            }),
            ("negative", {
                let mut value = valid_input();
                value.available_cash = -1.0;
                value
            }),
            ("accounting", {
                let mut value = valid_input();
                value.total_assets += 0.02;
                value
            }),
            ("withdrawable", {
                let mut value = valid_input();
                value.withdrawable_cash = Some(20_000.02);
                value
            }),
            ("ratio", {
                let mut value = valid_input();
                value.position_ratio_pct = Some(79.8);
                value
            }),
            ("source", {
                let mut value = valid_input();
                value.source_provider = " ".to_string();
                value
            }),
            ("environment", {
                let mut value = valid_input();
                value.environment = "test".to_string();
                value
            }),
            ("currency", {
                let mut value = valid_input();
                value.currency = "USD".to_string();
                value
            }),
            ("optional non-finite", {
                let mut value = valid_input();
                value.daily_pnl = Some(f64::INFINITY);
                value
            }),
            ("ratio range", {
                let mut value = valid_input();
                value.position_ratio_pct = Some(100.1);
                value
            }),
            ("ratio without assets", {
                let mut value = valid_input();
                value.total_assets = 0.0;
                value.securities_market_value = 0.0;
                value.available_cash = 0.0;
                value.withdrawable_cash = None;
                value.position_ratio_pct = Some(0.0);
                value
            }),
            ("snapshot date", {
                let mut value = valid_input();
                value.snapshot_date = chrono::NaiveDate::from_ymd_opt(2026, 7, 17).unwrap();
                value
            }),
            ("account ref", {
                let mut value = valid_input();
                value.account_ref = Some(" ".to_string());
                value
            }),
            ("hash", {
                let mut value = valid_input();
                value.evidence_sha256 = "not-a-sha".to_string();
                value
            }),
            ("time order", {
                let mut value = valid_input();
                value.observed_at = value.source_captured_at - Duration::seconds(1);
                value
            }),
            ("mode", {
                let mut value = valid_input();
                value.account_mode = Some("Unknown".to_string());
                value
            }),
        ];
        for (label, input) in cases {
            let mut conn = connection();
            let error = save_account_snapshot_with_conn(&mut conn, &input)
                .expect_err("invalid snapshot must fail");
            assert!(error.contains("BR-103"), "{label}: {error}");
        }
    }

    #[test]
    fn freshness_gate_rejects_stale_or_future_but_keeps_archival_row() {
        let mut conn = connection();
        let input = valid_input();
        let receipt = save_account_snapshot_with_conn(&mut conn, &input).expect("archival save");
        let saved = get_account_snapshot_with_conn(&mut conn, receipt.id)
            .unwrap()
            .unwrap();
        assert!(saved
            .validate_fresh_for_action(input.observed_at + Duration::seconds(30))
            .is_ok());
        assert!(saved
            .validate_fresh_for_action(input.observed_at + Duration::milliseconds(30_001))
            .unwrap_err()
            .contains("stale"));
        assert!(saved
            .validate_fresh_for_action(input.observed_at - Duration::milliseconds(1))
            .unwrap_err()
            .contains("future"));
    }

    #[test]
    fn persisted_snapshot_is_immutable() {
        let mut conn = connection();
        let receipt = save_account_snapshot_with_conn(&mut conn, &valid_input()).unwrap();
        let update =
            diesel::sql_query("UPDATE real_account_snapshot SET daily_pnl = 0 WHERE id = ?")
                .bind::<diesel::sql_types::BigInt, _>(receipt.id)
                .execute(&mut conn);
        assert!(update.unwrap_err().to_string().contains("immutable"));
        let delete = diesel::sql_query("DELETE FROM real_account_snapshot WHERE id = ?")
            .bind::<diesel::sql_types::BigInt, _>(receipt.id)
            .execute(&mut conn);
        assert!(delete.unwrap_err().to_string().contains("retention"));
    }

    #[test]
    fn evidence_hash_conflict_and_missing_id_are_explicit() {
        let mut conn = connection();
        let original = valid_input();
        let receipt = save_account_snapshot_with_conn(&mut conn, &original).unwrap();
        assert!(get_account_snapshot_with_conn(&mut conn, receipt.id + 1)
            .unwrap()
            .is_none());
        let mut conflict = original;
        conflict.total_assets += 100.0;
        conflict.securities_market_value += 100.0;
        assert!(save_account_snapshot_with_conn(&mut conn, &conflict)
            .unwrap_err()
            .contains("conflicts"));
    }

    #[test]
    fn malformed_persisted_dates_are_rejected_on_read() {
        for (snapshot_date, source_time, observed_time) in [
            (
                "bad-date",
                "2026-07-18T17:38:00+08:00",
                "2026-07-18T17:38:01+08:00",
            ),
            ("2026-07-18", "bad-source", "2026-07-18T17:38:01+08:00"),
            ("2026-07-18", "2026-07-18T17:38:00+08:00", "bad-observed"),
        ] {
            let mut conn = connection();
            diesel::sql_query(
                "INSERT INTO real_account_snapshot \
                 (snapshot_date, evidence_class, environment, total_assets, \
                  securities_market_value, available_cash, daily_pnl_status, \
                  source_provider, source_account_type, ownership_attestation, currency, \
                  source_captured_at, observed_at, account_ref_status, evidence_sha256) \
                 VALUES (?, 'TEST_CODE_live', 'live', 100000, 80000, 20000, \
                         'unavailable', 'TEST_CODE_source', 'TEST_CODE_type', \
                         'TEST_CODE_attested', 'CNY', ?, ?, 'not_provided', \
                         'abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd')",
            )
            .bind::<diesel::sql_types::Text, _>(snapshot_date)
            .bind::<diesel::sql_types::Text, _>(source_time)
            .bind::<diesel::sql_types::Text, _>(observed_time)
            .execute(&mut conn)
            .unwrap();
            assert!(get_account_snapshot_with_conn(&mut conn, 1)
                .unwrap_err()
                .contains("stored"));
        }
    }

    #[test]
    fn public_repository_wrappers_round_trip_and_return_latest() {
        crate::database::DatabaseManager::init(None).expect("shared isolated test database");
        let mut input = valid_input();
        input.evidence_sha256 = "f".repeat(64);
        let receipt = save_account_snapshot(&input).expect("public save");
        let by_id = get_account_snapshot(receipt.id)
            .expect("public get")
            .expect("saved row");
        assert_eq!(by_id.daily_pnl, None);
        let latest = latest_account_snapshot()
            .expect("public latest")
            .expect("at least the saved row");
        assert!(latest.observed_at >= input.observed_at);
    }

    #[test]
    fn evidence_json_preserves_missing_daily_pnl_and_provenance() {
        let json = serde_json::json!({
            "schema_version": 1,
            "evidence_class": "TEST_CODE_live_account_snapshot",
            "environment": "live",
            "source": {
                "provider": "TEST_CODE_broker_screenshot",
                "account_type": "TEST_CODE_cash_equity",
                "capture_time": "2026-07-18T17:38:00+08:00",
                "ownership_attestation": "TEST_CODE_user_attested",
                "original_image_sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            },
            "recorded_at": "2026-07-18T17:38:01+08:00",
            "account": {
                "account_ref_status": "not_provided",
                "currency": "CNY",
                "total_assets": 100000.0,
                "securities_market_value": 80000.0,
                "available_cash": 20000.0,
                "withdrawable_cash": 19000.0,
                "holding_pnl": -5000.0,
                "daily_pnl_status": "unavailable",
                "position_ratio_pct": 80.0
            }
        });
        let input = account_snapshot_input_from_json(&json.to_string()).expect("valid evidence");
        assert_eq!(input.daily_pnl, None);
        assert_eq!(input.account_mode, None);
        assert_eq!(input.account_ref, None);
        assert_eq!(input.daily_pnl_status, "unavailable");
        assert_eq!(input.account_ref_status, "not_provided");
    }

    #[test]
    fn evidence_json_rejects_bad_schema_time_or_missing_provenance() {
        for json in [
            "{}".to_string(),
            serde_json::json!({
                "schema_version": 0,
                "evidence_class": "TEST_CODE_live_account_snapshot",
                "environment": "live",
                "source": {
                    "provider": "TEST_CODE_broker_screenshot",
                    "account_type": "TEST_CODE_cash_equity",
                    "capture_time": "bad-time",
                    "ownership_attestation": "TEST_CODE_user_attested",
                    "original_image_sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                },
                "recorded_at": "bad-time",
                "account": {
                    "account_ref_status": "not_provided",
                    "currency": "CNY",
                    "total_assets": 100000.0,
                    "securities_market_value": 80000.0,
                    "available_cash": 20000.0,
                    "daily_pnl_status": "unavailable"
                }
            })
            .to_string(),
        ] {
            assert!(account_snapshot_input_from_json(&json)
                .unwrap_err()
                .contains("BR-103"));
        }
    }
}
