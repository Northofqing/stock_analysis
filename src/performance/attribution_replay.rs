//! BR-251 历史归因只读证据装载。

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::Metadata;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use chrono::{DateTime, FixedOffset, NaiveDate};
use rusqlite::{
    params_from_iter, types::Value, Connection, ErrorCode, OpenFlags, Transaction,
    TransactionBehavior,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::economic_position::{
    rebuild_economic_positions, select_economic_rows_through, EconomicFillRow,
};
use crate::database::order_audit::{
    validate_canonical_order_audit_chain, CanonicalOrderAuditChainRow, CanonicalOrderAuditRow,
};
use crate::trading::paper_lot_ledger::parse_paper_fill_timestamp;

const STOCK_CLOSE_HASH_DOMAIN: &[u8] = b"BR251_STOCK_CLOSE_MANIFEST_V1\0";
const FEE_EVIDENCE_HASH_DOMAIN: &[u8] = b"BR251_FILL_FEE_EVIDENCE_V1\0";
const STOCK_CLOSE_KEYS_PER_QUERY: usize = 400;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributionUnavailable {
    SourceUnavailable,
    TradeTimeUnavailable,
    StockCloseUnavailable,
    FeeEvidenceUnavailable,
}

impl AttributionUnavailable {
    pub const fn code(self) -> &'static str {
        match self {
            Self::SourceUnavailable => "replay_source_unavailable",
            Self::TradeTimeUnavailable => "trade_time_unavailable",
            Self::StockCloseUnavailable => "stock_close_unavailable",
            Self::FeeEvidenceUnavailable => "fee_evidence_unavailable",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributionIntegrityFailure {
    InvalidRequest,
    DatabaseIdentity,
    ReadOnlyBoundary,
    SourceRead,
    OrderAuditChain,
    PaperTradeSource,
    TerminalBinding,
    StockCloseSource,
    FeeEvidence,
}

impl AttributionIntegrityFailure {
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_replay_request",
            Self::DatabaseIdentity => "database_identity_failed",
            Self::ReadOnlyBoundary => "read_only_boundary_failed",
            Self::SourceRead => "replay_source_read_failed",
            Self::OrderAuditChain => "order_audit_chain_failed",
            Self::PaperTradeSource => "paper_trade_source_failed",
            Self::TerminalBinding => "terminal_binding_failed",
            Self::StockCloseSource => "stock_close_source_failed",
            Self::FeeEvidence => "fee_evidence_failed",
        }
    }
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum AttributionReplayError {
    #[error("{code:?}: {detail}")]
    Unavailable {
        code: AttributionUnavailable,
        retryable: bool,
        detail: String,
    },
    #[error("{code:?}: {detail}")]
    FailedIntegrity {
        code: AttributionIntegrityFailure,
        detail: String,
    },
}

impl AttributionReplayError {
    fn unavailable(
        code: AttributionUnavailable,
        retryable: bool,
        detail: impl Into<String>,
    ) -> Self {
        Self::Unavailable {
            code,
            retryable,
            detail: detail.into(),
        }
    }

    fn integrity(code: AttributionIntegrityFailure, detail: impl Into<String>) -> Self {
        Self::FailedIntegrity {
            code,
            detail: detail.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FillFeeEvidence {
    pub fill_id: i64,
    pub adverse_cost: f64,
    pub source: String,
    pub authority: String,
    pub evidence_id: String,
    pub evidence_hash: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AuthoritativeFillFeeLedger {
    pub entries: Vec<FillFeeEvidence>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FeeEvidenceAvailability {
    Available(AuthoritativeFillFeeLedger),
    Unavailable {
        code: AttributionUnavailable,
        retryable: bool,
        detail: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct AttributionReplayRequest {
    pub from: NaiveDate,
    pub to: NaiveDate,
    /// 由上层已验证交易日 authority 提供；本装载器绝不猜工作日或节假日。
    pub required_trading_dates: Vec<NaiveDate>,
    pub fee_ledger: Option<AuthoritativeFillFeeLedger>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReplayFillEvidence {
    pub fill: EconomicFillRow,
    pub terminal_audit_id: i64,
    pub terminal_audit_hash: String,
    pub terminal_time: DateTime<FixedOffset>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StockCloseEvidence {
    pub code: String,
    pub date: NaiveDate,
    pub close: f64,
    pub data_source: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StockCloseManifest {
    pub entries: Vec<StockCloseEvidence>,
    pub manifest_hash: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AttributionReplayEvidence {
    pub from: NaiveDate,
    pub to: NaiveDate,
    /// 含范围开始前 FIFO 前史，但不含 `to` 之后的成交。
    pub fills: Vec<ReplayFillEvidence>,
    pub stock_closes: StockCloseManifest,
    pub fees: FeeEvidenceAvailability,
}

#[derive(Debug, Clone)]
pub struct AttributionReplayLoader {
    database: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    device: u64,
    inode: u64,
}

impl FileIdentity {
    fn of(metadata: &Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        }
    }
}

#[derive(Debug, Clone)]
struct PaperTradeSourceRow {
    fill: EconomicFillRow,
    requested_price: f64,
}

#[derive(Debug, Clone)]
struct RawStockCloseRow {
    id: i64,
    code: String,
    date: String,
    close: Option<f64>,
    data_source: Option<String>,
    created_at: String,
    updated_at: String,
}

impl AttributionReplayLoader {
    pub fn new(database: impl AsRef<Path>) -> Self {
        Self {
            database: database.as_ref().to_path_buf(),
        }
    }

    pub fn load(
        &self,
        request: &AttributionReplayRequest,
    ) -> Result<AttributionReplayEvidence, AttributionReplayError> {
        validate_request(request)?;
        let canonical_database = self.database.canonicalize().map_err(|error| {
            AttributionReplayError::integrity(
                AttributionIntegrityFailure::DatabaseIdentity,
                format!("explicit database path cannot be resolved: {error}"),
            )
        })?;
        let before_metadata = canonical_database.metadata().map_err(|error| {
            AttributionReplayError::integrity(
                AttributionIntegrityFailure::DatabaseIdentity,
                format!("explicit database metadata unavailable: {error}"),
            )
        })?;
        if !before_metadata.is_file() {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::DatabaseIdentity,
                "explicit database path is not a regular file",
            ));
        }
        let expected_identity = FileIdentity::of(&before_metadata);
        let mut connection = open_query_only_connection(&canonical_database)?;
        verify_main_database(&connection, &canonical_database, expected_identity)?;

        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|error| source_read_error("begin one read transaction", error))?;
        let all_paper_rows = load_paper_rows(&transaction)?;
        let audit_rows = load_order_audits(&transaction)?;
        let chain_rows = load_order_audit_chain(&transaction)?;
        validate_canonical_order_audit_chain(&audit_rows, &chain_rows).map_err(|detail| {
            AttributionReplayError::integrity(AttributionIntegrityFailure::OrderAuditChain, detail)
        })?;

        let all_economic_rows = all_paper_rows
            .iter()
            .map(|row| row.fill.clone())
            .collect::<Vec<_>>();
        validate_complete_paper_source(&all_economic_rows, request.to)?;
        let all_terminals = bind_all_terminals(&all_paper_rows, &audit_rows, &chain_rows)?;
        let projected_rows =
            select_economic_rows_through(all_economic_rows, request.to).map_err(|detail| {
                AttributionReplayError::integrity(
                    AttributionIntegrityFailure::PaperTradeSource,
                    detail,
                )
            })?;
        let terminal_by_fill = all_terminals
            .into_iter()
            .map(|terminal| (terminal.fill.id, terminal))
            .collect::<HashMap<_, _>>();
        let fills = projected_rows
            .into_iter()
            .map(|row| {
                terminal_by_fill.get(&row.id).cloned().ok_or_else(|| {
                    AttributionReplayError::integrity(
                        AttributionIntegrityFailure::TerminalBinding,
                        format!("validated terminal disappeared for fill id={}", row.id),
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let required_close_keys =
            derive_required_close_keys(&fills, &request.required_trading_dates)?;
        let raw_closes = load_stock_closes(&transaction, &required_close_keys)?;
        let stock_closes = build_stock_close_manifest(raw_closes, &required_close_keys)?;
        verify_transaction_main_database(&transaction, &canonical_database, expected_identity)?;
        let fees = validate_fee_ledger(request.fee_ledger.as_ref(), &fills)?;

        #[cfg(test)]
        run_after_read_test_hook();
        let during_identity = canonical_database
            .metadata()
            .map(|metadata| FileIdentity::of(&metadata))
            .map_err(|error| {
                AttributionReplayError::integrity(
                    AttributionIntegrityFailure::DatabaseIdentity,
                    format!("database identity re-check during read failed: {error}"),
                )
            })?;
        if during_identity != expected_identity {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::DatabaseIdentity,
                "database file identity changed during read",
            ));
        }
        transaction
            .commit()
            .map_err(|error| source_read_error("finish read transaction", error))?;
        verify_main_database(&connection, &canonical_database, expected_identity)?;
        let after_identity = canonical_database
            .metadata()
            .map(|metadata| FileIdentity::of(&metadata))
            .map_err(|error| {
                AttributionReplayError::integrity(
                    AttributionIntegrityFailure::DatabaseIdentity,
                    format!("database identity re-check after read failed: {error}"),
                )
            })?;
        if after_identity != expected_identity {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::DatabaseIdentity,
                "database file identity changed after read",
            ));
        }

        Ok(AttributionReplayEvidence {
            from: request.from,
            to: request.to,
            fills,
            stock_closes,
            fees,
        })
    }
}

fn open_query_only_connection(path: &Path) -> Result<Connection, AttributionReplayError> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| {
        AttributionReplayError::integrity(
            AttributionIntegrityFailure::ReadOnlyBoundary,
            format!("open explicit SQLite database read-only: {error}"),
        )
    })?;
    connection
        .execute_batch("PRAGMA query_only=ON;")
        .map_err(|error| {
            AttributionReplayError::integrity(
                AttributionIntegrityFailure::ReadOnlyBoundary,
                format!("enable SQLite query_only: {error}"),
            )
        })?;
    let query_only: i64 = connection
        .query_row("PRAGMA query_only", [], |row| row.get(0))
        .map_err(|error| {
            AttributionReplayError::integrity(
                AttributionIntegrityFailure::ReadOnlyBoundary,
                format!("verify SQLite query_only: {error}"),
            )
        })?;
    if query_only != 1 {
        return Err(AttributionReplayError::integrity(
            AttributionIntegrityFailure::ReadOnlyBoundary,
            format!("SQLite query_only expected 1, got {query_only}"),
        ));
    }
    Ok(connection)
}

fn validate_request(request: &AttributionReplayRequest) -> Result<(), AttributionReplayError> {
    if request.from > request.to {
        return Err(AttributionReplayError::integrity(
            AttributionIntegrityFailure::InvalidRequest,
            "attribution replay from date is after to date",
        ));
    }
    if request.required_trading_dates.is_empty() {
        return Err(AttributionReplayError::integrity(
            AttributionIntegrityFailure::InvalidRequest,
            "required trading dates authority must not be empty",
        ));
    }
    let mut previous = None;
    for current in &request.required_trading_dates {
        if *current < request.from || *current > request.to {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::InvalidRequest,
                format!("required trading date {current} is outside requested range"),
            ));
        }
        if previous.is_some_and(|date| date >= *current) {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::InvalidRequest,
                "required trading dates must be sorted and unique",
            ));
        }
        previous = Some(*current);
    }
    Ok(())
}

fn verify_main_database(
    connection: &Connection,
    expected_path: &Path,
    expected_identity: FileIdentity,
) -> Result<(), AttributionReplayError> {
    let mut statement = connection
        .prepare("PRAGMA database_list")
        .map_err(|error| {
            AttributionReplayError::integrity(
                AttributionIntegrityFailure::DatabaseIdentity,
                format!("prepare SQLite database_list: {error}"),
            )
        })?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(1)?, row.get::<_, String>(2)?))
        })
        .map_err(|error| source_read_error("read SQLite database_list", error))?;
    let databases = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| source_read_error("decode SQLite database_list", error))?;
    let main_files = databases
        .into_iter()
        .filter_map(|(name, file)| (name == "main").then_some(file))
        .collect::<Vec<_>>();
    if main_files.len() != 1 || main_files[0].trim().is_empty() {
        return Err(AttributionReplayError::integrity(
            AttributionIntegrityFailure::DatabaseIdentity,
            "SQLite database_list does not identify exactly one main file",
        ));
    }
    let main_path = PathBuf::from(&main_files[0])
        .canonicalize()
        .map_err(|error| {
            AttributionReplayError::integrity(
                AttributionIntegrityFailure::DatabaseIdentity,
                format!("resolve SQLite main file: {error}"),
            )
        })?;
    let main_identity = main_path
        .metadata()
        .map(|metadata| FileIdentity::of(&metadata))
        .map_err(|error| {
            AttributionReplayError::integrity(
                AttributionIntegrityFailure::DatabaseIdentity,
                format!("read SQLite main identity: {error}"),
            )
        })?;
    if main_path != expected_path || main_identity != expected_identity {
        return Err(AttributionReplayError::integrity(
            AttributionIntegrityFailure::DatabaseIdentity,
            "SQLite main file does not match pinned explicit database",
        ));
    }
    Ok(())
}

fn verify_transaction_main_database(
    transaction: &Transaction<'_>,
    expected_path: &Path,
    expected_identity: FileIdentity,
) -> Result<(), AttributionReplayError> {
    let file: String = transaction
        .query_row(
            "SELECT file FROM pragma_database_list WHERE name='main'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| source_read_error("read transaction main file", error))?;
    let main = PathBuf::from(file).canonicalize().map_err(|error| {
        AttributionReplayError::integrity(
            AttributionIntegrityFailure::DatabaseIdentity,
            format!("resolve transaction main file: {error}"),
        )
    })?;
    let identity = main
        .metadata()
        .map(|metadata| FileIdentity::of(&metadata))
        .map_err(|error| {
            AttributionReplayError::integrity(
                AttributionIntegrityFailure::DatabaseIdentity,
                format!("read transaction main identity: {error}"),
            )
        })?;
    if main != expected_path || identity != expected_identity {
        return Err(AttributionReplayError::integrity(
            AttributionIntegrityFailure::DatabaseIdentity,
            "transaction main file identity changed",
        ));
    }
    Ok(())
}

fn source_read_error(context: &str, error: rusqlite::Error) -> AttributionReplayError {
    if matches!(
        &error,
        rusqlite::Error::SqliteFailure(sqlite, _)
            if matches!(sqlite.code, ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
    ) {
        return AttributionReplayError::unavailable(
            AttributionUnavailable::SourceUnavailable,
            true,
            format!("{context}: SQLite source is busy or locked"),
        );
    }
    AttributionReplayError::integrity(
        AttributionIntegrityFailure::SourceRead,
        format!("{context}: {error}"),
    )
}

fn load_paper_rows(
    transaction: &Transaction<'_>,
) -> Result<Vec<PaperTradeSourceRow>, AttributionReplayError> {
    let mut statement = transaction
        .prepare(
            "SELECT id, plan_id, code, name, direction, price, fill_price, quantity,
                    CAST(ts AS TEXT), virtual_reason
             FROM paper_trades WHERE status='Filled'
             ORDER BY CAST(ts AS TEXT) ASC, id ASC",
        )
        .map_err(|error| source_read_error("prepare complete Filled paper source", error))?;
    let rows = statement
        .query_map([], |row| {
            Ok(PaperTradeSourceRow {
                fill: EconomicFillRow {
                    id: row.get(0)?,
                    plan_id: row.get(1)?,
                    code: row.get(2)?,
                    name: row.get(3)?,
                    direction: row.get(4)?,
                    fill_price: row.get(6)?,
                    quantity: row.get(7)?,
                    occurred_at: row.get(8)?,
                    virtual_reason: row.get(9)?,
                },
                requested_price: row.get(5)?,
            })
        })
        .map_err(|error| source_read_error("read complete Filled paper source", error))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| source_read_error("decode complete Filled paper source", error))
}

fn load_order_audits(
    transaction: &Transaction<'_>,
) -> Result<Vec<CanonicalOrderAuditRow>, AttributionReplayError> {
    let mut statement = transaction
        .prepare(
            "SELECT id,business_order_id,source,decision_basis,side,code,
                    requested_price,execution_price,quantity,quote_observed_at,
                    outcome,failure_reason,CAST(created_at AS TEXT)
             FROM order_audit ORDER BY id ASC",
        )
        .map_err(|error| source_read_error("prepare complete order audit source", error))?;
    let rows = statement
        .query_map([], |row| {
            Ok(CanonicalOrderAuditRow {
                id: row.get(0)?,
                business_order_id: row.get(1)?,
                source: row.get(2)?,
                decision_basis: row.get(3)?,
                side: row.get(4)?,
                code: row.get(5)?,
                requested_price: row.get(6)?,
                execution_price: row.get(7)?,
                quantity: row.get(8)?,
                quote_observed_at: row.get(9)?,
                outcome: row.get(10)?,
                failure_reason: row.get(11)?,
                created_at: row.get(12)?,
            })
        })
        .map_err(|error| source_read_error("read complete order audit source", error))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| source_read_error("decode complete order audit source", error))
}

fn load_order_audit_chain(
    transaction: &Transaction<'_>,
) -> Result<Vec<CanonicalOrderAuditChainRow>, AttributionReplayError> {
    let mut statement = transaction
        .prepare(
            "SELECT order_audit_id,previous_hash,record_hash
             FROM order_audit_chain ORDER BY order_audit_id ASC",
        )
        .map_err(|error| source_read_error("prepare complete order audit chain", error))?;
    let rows = statement
        .query_map([], |row| {
            Ok(CanonicalOrderAuditChainRow {
                order_audit_id: row.get(0)?,
                previous_hash: row.get(1)?,
                record_hash: row.get(2)?,
            })
        })
        .map_err(|error| source_read_error("read complete order audit chain", error))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| source_read_error("decode complete order audit chain", error))
}

fn load_stock_closes(
    transaction: &Transaction<'_>,
    required_keys: &BTreeSet<(String, NaiveDate)>,
) -> Result<Vec<RawStockCloseRow>, AttributionReplayError> {
    let keys = required_keys.iter().collect::<Vec<_>>();
    let mut result = Vec::new();
    for chunk in keys.chunks(STOCK_CLOSE_KEYS_PER_QUERY) {
        let predicate = std::iter::repeat_n("(code = ? AND date = ?)", chunk.len())
            .collect::<Vec<_>>()
            .join(" OR ");
        let sql = format!(
            "SELECT id,code,date,close,data_source,
                    CAST(created_at AS TEXT),CAST(updated_at AS TEXT)
             FROM stock_daily WHERE {predicate}
             ORDER BY code ASC, date ASC, id ASC"
        );
        let values = chunk
            .iter()
            .flat_map(|(code, date)| {
                [
                    Value::Text(code.clone()),
                    Value::Text(date.format("%Y-%m-%d").to_string()),
                ]
            })
            .collect::<Vec<_>>();
        let mut statement = transaction
            .prepare(&sql)
            .map_err(|error| source_read_error("prepare exact stock close source", error))?;
        let rows = statement
            .query_map(params_from_iter(values), |row| {
                Ok(RawStockCloseRow {
                    id: row.get(0)?,
                    code: row.get(1)?,
                    date: row.get(2)?,
                    close: row.get(3)?,
                    data_source: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            })
            .map_err(|error| source_read_error("read exact stock close source", error))?;
        result.extend(
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|error| source_read_error("decode exact stock close source", error))?,
        );
    }
    Ok(result)
}

fn validate_complete_paper_source(
    rows: &[EconomicFillRow],
    empty_source_date: NaiveDate,
) -> Result<(), AttributionReplayError> {
    let max_date = rows
        .iter()
        .map(|row| {
            parse_paper_fill_timestamp(row.id, &row.occurred_at)
                .map(|timestamp| timestamp.date())
                .map_err(|detail| {
                    AttributionReplayError::integrity(
                        AttributionIntegrityFailure::PaperTradeSource,
                        detail,
                    )
                })
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .max()
        .unwrap_or(empty_source_date);
    select_economic_rows_through(rows.to_vec(), max_date).map_err(|detail| {
        AttributionReplayError::integrity(AttributionIntegrityFailure::PaperTradeSource, detail)
    })?;
    rebuild_economic_positions(rows, max_date, None).map_err(|detail| {
        AttributionReplayError::integrity(AttributionIntegrityFailure::PaperTradeSource, detail)
    })?;
    Ok(())
}

fn bind_all_terminals(
    paper_rows: &[PaperTradeSourceRow],
    audits: &[CanonicalOrderAuditRow],
    chain: &[CanonicalOrderAuditChainRow],
) -> Result<Vec<ReplayFillEvidence>, AttributionReplayError> {
    let hashes = chain
        .iter()
        .map(|row| (row.order_audit_id, row.record_hash.as_str()))
        .collect::<HashMap<_, _>>();
    let paper_plans = paper_rows
        .iter()
        .map(|paper| paper.fill.plan_id.as_str())
        .collect::<HashSet<_>>();
    let mut by_business = HashMap::<&str, Vec<&CanonicalOrderAuditRow>>::new();
    for audit in audits
        .iter()
        .filter(|audit| audit.source == "PaperTrade" && audit.outcome == "Filled")
    {
        if !paper_plans.contains(audit.business_order_id.as_str()) {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::TerminalBinding,
                format!(
                    "PaperTrade Filled audit id={} has no Filled paper plan {}",
                    audit.id, audit.business_order_id
                ),
            ));
        }
        by_business
            .entry(audit.business_order_id.as_str())
            .or_default()
            .push(audit);
    }
    let shanghai = FixedOffset::east_opt(8 * 60 * 60).ok_or_else(|| {
        AttributionReplayError::integrity(
            AttributionIntegrityFailure::TerminalBinding,
            "fixed +08:00 offset is unavailable",
        )
    })?;
    let mut result = Vec::with_capacity(paper_rows.len());
    for paper in paper_rows {
        // BR-251：空集合只表示“零条 Filled 终态”，紧接着转为 typed
        // TradeTimeUnavailable；它绝不作为可计算数据或静默成功返回。
        let terminals = by_business
            .get(paper.fill.plan_id.as_str())
            .map(Vec::as_slice)
            .unwrap_or_default();
        if terminals.is_empty() {
            return Err(AttributionReplayError::unavailable(
                AttributionUnavailable::TradeTimeUnavailable,
                false,
                format!(
                    "Filled paper id={} has no Filled audit terminal",
                    paper.fill.id
                ),
            ));
        }
        if terminals.len() != 1 {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::TerminalBinding,
                format!(
                    "Filled paper id={} has {} Filled audit terminals",
                    paper.fill.id,
                    terminals.len()
                ),
            ));
        }
        let terminal = terminals[0];
        let execution_price = terminal.execution_price.ok_or_else(|| {
            AttributionReplayError::integrity(
                AttributionIntegrityFailure::TerminalBinding,
                format!("Filled audit id={} execution price is absent", terminal.id),
            )
        })?;
        let paper_fill_price = paper.fill.fill_price.ok_or_else(|| {
            AttributionReplayError::integrity(
                AttributionIntegrityFailure::PaperTradeSource,
                format!("Filled paper id={} fill price is absent", paper.fill.id),
            )
        })?;
        let exact = terminal.code == paper.fill.code
            && terminal.side == paper.fill.direction
            && terminal.requested_price.to_bits() == paper.requested_price.to_bits()
            && execution_price.to_bits() == paper_fill_price.to_bits()
            && terminal.quantity == paper.fill.quantity;
        if !exact {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::TerminalBinding,
                format!(
                    "Filled paper id={} does not exactly match audit id={} source/code/side/prices/quantity",
                    paper.fill.id, terminal.id
                ),
            ));
        }
        if !paper.requested_price.is_finite()
            || paper.requested_price <= 0.0
            || !paper_fill_price.is_finite()
            || paper_fill_price <= 0.0
        {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::TerminalBinding,
                format!(
                    "Filled paper id={} contains an invalid price",
                    paper.fill.id
                ),
            ));
        }
        let raw_time = terminal
            .quote_observed_at
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                AttributionReplayError::unavailable(
                    AttributionUnavailable::TradeTimeUnavailable,
                    false,
                    format!("Filled audit id={} has no quote_observed_at", terminal.id),
                )
            })?;
        let terminal_time = DateTime::parse_from_rfc3339(raw_time)
            .map_err(|error| {
                AttributionReplayError::integrity(
                    AttributionIntegrityFailure::TerminalBinding,
                    format!(
                        "Filled audit id={} quote_observed_at is not full RFC3339: {error}",
                        terminal.id
                    ),
                )
            })?
            .with_timezone(&shanghai);
        let terminal_audit_hash = hashes.get(&terminal.id).ok_or_else(|| {
            AttributionReplayError::integrity(
                AttributionIntegrityFailure::OrderAuditChain,
                format!("Filled audit id={} has no chain hash", terminal.id),
            )
        })?;
        result.push(ReplayFillEvidence {
            fill: paper.fill.clone(),
            terminal_audit_id: terminal.id,
            terminal_audit_hash: (*terminal_audit_hash).to_owned(),
            terminal_time,
        });
    }
    Ok(result)
}

fn derive_required_close_keys(
    fills: &[ReplayFillEvidence],
    required_dates: &[NaiveDate],
) -> Result<BTreeSet<(String, NaiveDate)>, AttributionReplayError> {
    let dated_fills = fills
        .iter()
        .map(|evidence| {
            parse_paper_fill_timestamp(evidence.fill.id, &evidence.fill.occurred_at)
                .map(|timestamp| (timestamp.date(), evidence.fill.clone()))
                .map_err(|detail| {
                    AttributionReplayError::integrity(
                        AttributionIntegrityFailure::PaperTradeSource,
                        detail,
                    )
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut keys = BTreeSet::new();
    let mut prefix = Vec::new();
    let mut next_fill = 0;
    for required_date in required_dates {
        while next_fill < dated_fills.len() && dated_fills[next_fill].0 <= *required_date {
            let (fill_date, fill) = &dated_fills[next_fill];
            if fill_date == required_date {
                keys.insert((fill.code.clone(), *required_date));
            }
            prefix.push(fill.clone());
            next_fill += 1;
        }
        let report =
            rebuild_economic_positions(&prefix, *required_date, None).map_err(|detail| {
                AttributionReplayError::integrity(
                    AttributionIntegrityFailure::PaperTradeSource,
                    detail,
                )
            })?;
        keys.extend(
            report
                .open_positions
                .into_iter()
                .map(|position| (position.code, *required_date)),
        );
    }
    Ok(keys)
}

fn build_stock_close_manifest(
    rows: Vec<RawStockCloseRow>,
    required_keys: &BTreeSet<(String, NaiveDate)>,
) -> Result<StockCloseManifest, AttributionReplayError> {
    let mut selected = BTreeMap::<(String, NaiveDate), StockCloseEvidence>::new();
    for row in rows {
        let parsed_date = NaiveDate::parse_from_str(&row.date, "%Y-%m-%d").map_err(|error| {
            AttributionReplayError::integrity(
                AttributionIntegrityFailure::StockCloseSource,
                format!(
                    "stock_daily id={} date is not exact YYYY-MM-DD: {error}",
                    row.id
                ),
            )
        })?;
        if parsed_date.format("%Y-%m-%d").to_string() != row.date {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::StockCloseSource,
                format!("stock_daily id={} date is not canonical YYYY-MM-DD", row.id),
            ));
        }
        let key = (row.code.clone(), parsed_date);
        if !required_keys.contains(&key) {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::StockCloseSource,
                format!(
                    "stock close query returned unexpected key {} {}",
                    row.code, parsed_date
                ),
            ));
        }
        if selected.contains_key(&key) {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::StockCloseSource,
                format!(
                    "duplicate stock close fact for {} {}",
                    row.code, parsed_date
                ),
            ));
        }
        let close = row.close.ok_or_else(|| {
            AttributionReplayError::unavailable(
                AttributionUnavailable::StockCloseUnavailable,
                true,
                format!("stock close is absent for {} {}", row.code, parsed_date),
            )
        })?;
        if !close.is_finite() || close <= 0.0 {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::StockCloseSource,
                format!("stock close is invalid for {} {}", row.code, parsed_date),
            ));
        }
        row.data_source
            .as_deref()
            .filter(|source| !source.trim().is_empty())
            .ok_or_else(|| {
                AttributionReplayError::unavailable(
                    AttributionUnavailable::StockCloseUnavailable,
                    true,
                    format!(
                        "stock close source is absent for {} {}",
                        row.code, parsed_date
                    ),
                )
            })?;
        selected.insert(
            key,
            StockCloseEvidence {
                code: row.code,
                date: parsed_date,
                close,
                data_source: row.data_source,
                created_at: row.created_at,
                updated_at: row.updated_at,
            },
        );
    }
    for (code, date) in required_keys {
        if !selected.contains_key(&(code.clone(), *date)) {
            return Err(AttributionReplayError::unavailable(
                AttributionUnavailable::StockCloseUnavailable,
                true,
                format!("stock close is unavailable for {code} {date}"),
            ));
        }
    }
    let entries = selected.into_values().collect::<Vec<_>>();
    let manifest_hash = canonical_stock_close_manifest_hash(&entries);
    Ok(StockCloseManifest {
        entries,
        manifest_hash,
    })
}

fn update_len_prefixed(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

pub fn canonical_stock_close_manifest_hash(entries: &[StockCloseEvidence]) -> String {
    let mut sorted = entries.to_vec();
    sorted.sort_by(|left, right| (&left.code, left.date).cmp(&(&right.code, right.date)));
    let mut hasher = Sha256::new();
    hasher.update(STOCK_CLOSE_HASH_DOMAIN);
    hasher.update((sorted.len() as u64).to_be_bytes());
    for entry in sorted {
        update_len_prefixed(&mut hasher, entry.code.as_bytes());
        update_len_prefixed(
            &mut hasher,
            entry.date.format("%Y-%m-%d").to_string().as_bytes(),
        );
        hasher.update(entry.close.to_bits().to_be_bytes());
        match entry.data_source {
            Some(source) => {
                hasher.update([1]);
                update_len_prefixed(&mut hasher, source.as_bytes());
            }
            None => hasher.update([0]),
        }
        update_len_prefixed(&mut hasher, entry.created_at.as_bytes());
        update_len_prefixed(&mut hasher, entry.updated_at.as_bytes());
    }
    hex::encode(hasher.finalize())
}

pub fn canonical_fill_fee_evidence_hash(evidence: &FillFeeEvidence) -> String {
    let mut hasher = Sha256::new();
    hasher.update(FEE_EVIDENCE_HASH_DOMAIN);
    hasher.update(evidence.fill_id.to_be_bytes());
    hasher.update(evidence.adverse_cost.to_bits().to_be_bytes());
    update_len_prefixed(&mut hasher, evidence.source.as_bytes());
    update_len_prefixed(&mut hasher, evidence.authority.as_bytes());
    update_len_prefixed(&mut hasher, evidence.evidence_id.as_bytes());
    hex::encode(hasher.finalize())
}

fn validate_fee_ledger(
    ledger: Option<&AuthoritativeFillFeeLedger>,
    fills: &[ReplayFillEvidence],
) -> Result<FeeEvidenceAvailability, AttributionReplayError> {
    let Some(ledger) = ledger else {
        return Ok(FeeEvidenceAvailability::Unavailable {
            code: AttributionUnavailable::FeeEvidenceUnavailable,
            retryable: false,
            detail: "explicit authoritative per-fill fee ledger is unavailable".to_owned(),
        });
    };
    let fill_ids = fills
        .iter()
        .map(|evidence| evidence.fill.id)
        .collect::<HashSet<_>>();
    let mut seen = HashSet::new();
    for entry in &ledger.entries {
        if !fill_ids.contains(&entry.fill_id) {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::FeeEvidence,
                format!("fee evidence references unknown fill id={}", entry.fill_id),
            ));
        }
        if !seen.insert(entry.fill_id) {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::FeeEvidence,
                format!("duplicate fee evidence for fill id={}", entry.fill_id),
            ));
        }
        if !entry.adverse_cost.is_finite()
            || entry.adverse_cost < 0.0
            || entry.source.trim().is_empty()
            || entry.authority.trim().is_empty()
            || entry.evidence_id.trim().is_empty()
            || !is_lowercase_sha256(&entry.evidence_hash)
            || canonical_fill_fee_evidence_hash(entry) != entry.evidence_hash
        {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::FeeEvidence,
                format!(
                    "invalid authoritative fee evidence for fill id={}",
                    entry.fill_id
                ),
            ));
        }
    }
    if seen != fill_ids {
        let missing = fill_ids.difference(&seen).copied().collect::<Vec<_>>();
        return Err(AttributionReplayError::integrity(
            AttributionIntegrityFailure::FeeEvidence,
            format!("fee evidence is missing fill ids {missing:?}"),
        ));
    }
    Ok(FeeEvidenceAvailability::Available(ledger.clone()))
}

fn is_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
type AfterReadTestHook = Box<dyn FnOnce() + Send + 'static>;

#[cfg(test)]
static AFTER_READ_TEST_HOOK: once_cell::sync::Lazy<std::sync::Mutex<Option<AfterReadTestHook>>> =
    once_cell::sync::Lazy::new(|| std::sync::Mutex::new(None));

#[cfg(test)]
fn set_after_read_test_hook(hook: AfterReadTestHook) {
    *AFTER_READ_TEST_HOOK.lock().expect("TEST_CODE hook mutex") = Some(hook);
}

#[cfg(test)]
fn run_after_read_test_hook() {
    let hook = AFTER_READ_TEST_HOOK
        .lock()
        .expect("TEST_CODE hook mutex")
        .take();
    if let Some(hook) = hook {
        hook();
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chrono::NaiveDate;
    use rusqlite::{params, Connection};

    use super::*;
    use crate::database::order_audit::{
        canonical_order_audit_record_hash, CanonicalOrderAuditRow, AUDIT_CHAIN_GENESIS,
    };

    fn date(raw: &str) -> NaiveDate {
        NaiveDate::parse_from_str(raw, "%Y-%m-%d").unwrap()
    }

    fn test_database_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "TEST_CODE_attribution_replay_{label}_{}_{}.sqlite3",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn create_schema(connection: &Connection) {
        connection
            .execute_batch(
                "CREATE TABLE paper_trades (
                    id INTEGER PRIMARY KEY, plan_id TEXT NOT NULL UNIQUE,
                    code TEXT NOT NULL, name TEXT NOT NULL, direction TEXT NOT NULL,
                    price REAL NOT NULL, quantity INTEGER NOT NULL, status TEXT NOT NULL,
                    fill_price REAL, virtual_reason TEXT NOT NULL, ts TEXT NOT NULL
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
                    record_hash TEXT NOT NULL, created_at TEXT NOT NULL
                 );
                 CREATE TABLE stock_daily (
                    id INTEGER PRIMARY KEY, code TEXT NOT NULL, date TEXT NOT NULL,
                    close REAL, data_source TEXT, created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                 );",
            )
            .unwrap();
    }

    fn append_filled_pair(
        connection: &Connection,
        id: i64,
        plan_id: &str,
        side: &str,
        price: f64,
        quote_observed_at: &str,
        paper_ts: &str,
        previous_hash: &str,
    ) -> String {
        connection
            .execute(
                "INSERT INTO paper_trades
                 (id,plan_id,code,name,direction,price,quantity,status,fill_price,virtual_reason,ts)
                 VALUES (?1,?2,'TEST_CODE_600001','TEST_CODE公司',?3,?4,100,'Filled',?4,?5,?6)",
                params![
                    id,
                    plan_id,
                    side,
                    price,
                    if side == "buy" {
                        "Breakout"
                    } else {
                        "ExitByRule"
                    },
                    paper_ts
                ],
            )
            .unwrap();
        let row = CanonicalOrderAuditRow {
            id,
            business_order_id: plan_id.to_owned(),
            source: "PaperTrade".to_owned(),
            decision_basis: "TEST_CODE terminal".to_owned(),
            side: side.to_owned(),
            code: "TEST_CODE_600001".to_owned(),
            requested_price: price,
            execution_price: Some(price),
            quantity: 100,
            quote_observed_at: Some(quote_observed_at.to_owned()),
            outcome: "Filled".to_owned(),
            failure_reason: None,
            created_at: "2026-08-22 00:00:00".to_owned(),
        };
        connection
            .execute(
                "INSERT INTO order_audit VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
                params![
                    row.id,
                    row.business_order_id,
                    row.source,
                    row.decision_basis,
                    row.side,
                    row.code,
                    row.requested_price,
                    row.execution_price,
                    row.quantity,
                    row.quote_observed_at,
                    row.outcome,
                    row.failure_reason,
                    row.created_at,
                ],
            )
            .unwrap();
        let record_hash = canonical_order_audit_record_hash(previous_hash, &row).unwrap();
        connection
            .execute(
                "INSERT INTO order_audit_chain VALUES (?1,?2,?3,'2026-08-22 00:00:01')",
                params![id, previous_hash, record_hash],
            )
            .unwrap();
        record_hash
    }

    fn complete_database(label: &str) -> PathBuf {
        let path = test_database_path(label);
        let connection = Connection::open(&path).unwrap();
        create_schema(&connection);
        let first_hash = append_filled_pair(
            &connection,
            1,
            "TEST_CODE_PLAN_1",
            "buy",
            10.0,
            "2026-08-20T01:31:05Z",
            "2026-08-20 09:31:05",
            AUDIT_CHAIN_GENESIS,
        );
        append_filled_pair(
            &connection,
            2,
            "TEST_CODE_PLAN_2",
            "sell",
            11.0,
            "2026-08-21T14:20:00+08:00",
            "2026-08-21 14:20:00",
            &first_hash,
        );
        for (id, day, close) in [(1, "2026-08-20", 10.2), (2, "2026-08-21", 11.1)] {
            connection
                .execute(
                    "INSERT INTO stock_daily VALUES (?1,'TEST_CODE_600001',?2,?3,'TEST_CODE_SOURCE','2026-08-22','2026-08-22')",
                    params![id, day, close],
                )
                .unwrap();
        }
        drop(connection);
        path
    }

    fn request_with_no_fees() -> AttributionReplayRequest {
        AttributionReplayRequest {
            from: date("2026-08-20"),
            to: date("2026-08-21"),
            required_trading_dates: vec![date("2026-08-20"), date("2026-08-21")],
            fee_ledger: None,
        }
    }

    fn audit_rows(connection: &Connection) -> Vec<CanonicalOrderAuditRow> {
        let mut statement = connection
            .prepare(
                "SELECT id,business_order_id,source,decision_basis,side,code,
                        requested_price,execution_price,quantity,quote_observed_at,
                        outcome,failure_reason,created_at FROM order_audit ORDER BY id",
            )
            .unwrap();
        statement
            .query_map([], |row| {
                Ok(CanonicalOrderAuditRow {
                    id: row.get(0)?,
                    business_order_id: row.get(1)?,
                    source: row.get(2)?,
                    decision_basis: row.get(3)?,
                    side: row.get(4)?,
                    code: row.get(5)?,
                    requested_price: row.get(6)?,
                    execution_price: row.get(7)?,
                    quantity: row.get(8)?,
                    quote_observed_at: row.get(9)?,
                    outcome: row.get(10)?,
                    failure_reason: row.get(11)?,
                    created_at: row.get(12)?,
                })
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    fn rehash_audits(connection: &Connection) {
        let rows = audit_rows(connection);
        connection
            .execute("DELETE FROM order_audit_chain", [])
            .unwrap();
        let mut previous = AUDIT_CHAIN_GENESIS.to_owned();
        for row in rows {
            let hash = canonical_order_audit_record_hash(&previous, &row).unwrap();
            connection
                .execute(
                    "INSERT INTO order_audit_chain VALUES (?1,?2,?3,'2026-08-22 00:00:01')",
                    params![row.id, previous, hash],
                )
                .unwrap();
            previous = hash;
        }
    }

    fn remove_database(path: PathBuf) {
        if std::env::var("TEST_CODE_KEEP_REPLAY_DB").as_deref() == Ok("1") {
            eprintln!("TEST_CODE_REPLAY_DB={}", path.display());
            return;
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn loader_returns_verified_history_and_typed_missing_fee() {
        let path = complete_database("happy");
        let request = request_with_no_fees();
        let before = path.metadata().unwrap();
        let before_count: i64 = Connection::open(&path)
            .unwrap()
            .query_row("SELECT COUNT(*) FROM paper_trades", [], |row| row.get(0))
            .unwrap();

        let evidence = AttributionReplayLoader::new(&path)
            .load(&request)
            .expect("complete read-only evidence");
        assert_eq!(evidence.fills.len(), 2);
        assert_eq!(
            evidence.fills[0].terminal_time.to_rfc3339(),
            "2026-08-20T09:31:05+08:00"
        );
        assert_eq!(evidence.stock_closes.entries.len(), 2);
        assert_eq!(evidence.stock_closes.manifest_hash.len(), 64);
        assert!(matches!(
            evidence.fees,
            FeeEvidenceAvailability::Unavailable {
                code: AttributionUnavailable::FeeEvidenceUnavailable,
                ..
            }
        ));
        let after = path.metadata().unwrap();
        let after_count: i64 = Connection::open(&path)
            .unwrap()
            .query_row("SELECT COUNT(*) FROM paper_trades", [], |row| row.get(0))
            .unwrap();
        assert_eq!(before.dev(), after.dev());
        assert_eq!(before.ino(), after.ino());
        assert_eq!(before_count, after_count);
        let readonly = open_query_only_connection(&path).unwrap();
        assert_eq!(
            readonly
                .query_row("PRAGMA query_only", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert!(readonly
            .execute("CREATE TABLE TEST_CODE_FORBIDDEN_WRITE(id INTEGER)", [])
            .is_err());
        assert!(path.is_file());
        remove_database(path);
    }

    #[test]
    fn loader_requires_an_existing_file_and_never_initializes_schema() {
        let missing = test_database_path("missing_file");
        assert!(matches!(
            AttributionReplayLoader::new(&missing).load(&request_with_no_fees()),
            Err(AttributionReplayError::FailedIntegrity {
                code: AttributionIntegrityFailure::DatabaseIdentity,
                ..
            })
        ));
        assert!(!missing.exists());

        let empty = test_database_path("empty_schema");
        Connection::open(&empty).unwrap();
        assert!(matches!(
            AttributionReplayLoader::new(&empty).load(&request_with_no_fees()),
            Err(AttributionReplayError::FailedIntegrity {
                code: AttributionIntegrityFailure::SourceRead,
                ..
            })
        ));
        let tables: i64 = Connection::open(&empty)
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tables, 0);
        remove_database(empty);

        let path = complete_database("bad_authority_dates");
        let mut request = request_with_no_fees();
        request.required_trading_dates.reverse();
        assert!(matches!(
            AttributionReplayLoader::new(&path).load(&request),
            Err(AttributionReplayError::FailedIntegrity {
                code: AttributionIntegrityFailure::InvalidRequest,
                ..
            })
        ));
        remove_database(path);

        let empty_dates_path = complete_database("empty_authority_dates");
        let mut empty_dates = request_with_no_fees();
        empty_dates.required_trading_dates.clear();
        assert!(matches!(
            AttributionReplayLoader::new(&empty_dates_path).load(&empty_dates),
            Err(AttributionReplayError::FailedIntegrity {
                code: AttributionIntegrityFailure::InvalidRequest,
                ..
            })
        ));
        remove_database(empty_dates_path);
    }

    #[test]
    fn external_sqlite_lock_is_retryable_source_unavailable() {
        let path = complete_database("busy_source");
        let writer = Connection::open(&path).unwrap();
        writer.execute_batch("BEGIN EXCLUSIVE").unwrap();

        assert!(matches!(
            AttributionReplayLoader::new(&path).load(&request_with_no_fees()),
            Err(AttributionReplayError::Unavailable {
                code: AttributionUnavailable::SourceUnavailable,
                retryable: true,
                ..
            })
        ));
        writer.execute_batch("ROLLBACK").unwrap();
        drop(writer);
        remove_database(path);
    }

    #[test]
    fn loader_rejects_main_file_replacement_during_the_read_snapshot() {
        let path = complete_database("replace_main");
        let displaced = path.with_extension("TEST_CODE_displaced.sqlite3");
        let hook_path = path.clone();
        let hook_displaced = displaced.clone();
        set_after_read_test_hook(Box::new(move || {
            std::fs::rename(&hook_path, &hook_displaced).unwrap();
            Connection::open(&hook_path).unwrap();
        }));

        assert!(matches!(
            AttributionReplayLoader::new(&path).load(&request_with_no_fees()),
            Err(AttributionReplayError::FailedIntegrity {
                code: AttributionIntegrityFailure::DatabaseIdentity,
                ..
            })
        ));
        std::fs::remove_file(&path).unwrap();
        std::fs::rename(&displaced, &path).unwrap();
        remove_database(path);
    }

    #[test]
    fn loader_rejects_missing_duplicate_and_mismatched_filled_terminals() {
        let missing_path = complete_database("missing_terminal");
        let missing = Connection::open(&missing_path).unwrap();
        missing
            .execute("DELETE FROM order_audit_chain WHERE order_audit_id=2", [])
            .unwrap();
        missing
            .execute("DELETE FROM order_audit WHERE id=2", [])
            .unwrap();
        drop(missing);
        assert!(matches!(
            AttributionReplayLoader::new(&missing_path).load(&request_with_no_fees()),
            Err(AttributionReplayError::Unavailable {
                code: AttributionUnavailable::TradeTimeUnavailable,
                ..
            })
        ));
        remove_database(missing_path);

        let duplicate_path = complete_database("duplicate_terminal");
        let duplicate = Connection::open(&duplicate_path).unwrap();
        duplicate
            .execute(
                "INSERT INTO order_audit VALUES
                 (3,'TEST_CODE_PLAN_2','PaperTrade','TEST_CODE duplicate','sell',
                  'TEST_CODE_600001',11.0,11.0,100,'2026-08-21T14:20:01+08:00',
                  'Filled',NULL,'2026-08-22 00:00:02')",
                [],
            )
            .unwrap();
        rehash_audits(&duplicate);
        drop(duplicate);
        assert!(matches!(
            AttributionReplayLoader::new(&duplicate_path).load(&request_with_no_fees()),
            Err(AttributionReplayError::FailedIntegrity {
                code: AttributionIntegrityFailure::TerminalBinding,
                ..
            })
        ));
        remove_database(duplicate_path);

        let source_path = complete_database("source_mismatch");
        let source = Connection::open(&source_path).unwrap();
        source
            .execute(
                "UPDATE order_audit SET source='TEST_CODE_OTHER' WHERE id=2",
                [],
            )
            .unwrap();
        rehash_audits(&source);
        drop(source);
        assert!(matches!(
            AttributionReplayLoader::new(&source_path).load(&request_with_no_fees()),
            Err(AttributionReplayError::Unavailable {
                code: AttributionUnavailable::TradeTimeUnavailable,
                ..
            })
        ));
        remove_database(source_path);

        for (label, update) in [
            (
                "code_mismatch",
                "UPDATE order_audit SET code='TEST_CODE_OTHER' WHERE id=2",
            ),
            (
                "side_mismatch",
                "UPDATE order_audit SET side='buy' WHERE id=2",
            ),
            (
                "request_price_mismatch",
                "UPDATE order_audit SET requested_price=11.1 WHERE id=2",
            ),
            (
                "execution_price_mismatch",
                "UPDATE order_audit SET execution_price=11.1 WHERE id=2",
            ),
            (
                "quantity_mismatch",
                "UPDATE order_audit SET quantity=200 WHERE id=2",
            ),
        ] {
            let path = complete_database(label);
            let connection = Connection::open(&path).unwrap();
            connection.execute(update, []).unwrap();
            rehash_audits(&connection);
            drop(connection);
            assert!(matches!(
                AttributionReplayLoader::new(&path).load(&request_with_no_fees()),
                Err(AttributionReplayError::FailedIntegrity {
                    code: AttributionIntegrityFailure::TerminalBinding,
                    ..
                })
            ));
            remove_database(path);
        }
    }

    #[test]
    fn paper_fills_and_paper_terminals_are_a_bidirectional_exact_set() {
        let orphan_path = complete_database("orphan_paper_terminal");
        let orphan = Connection::open(&orphan_path).unwrap();
        orphan
            .execute(
                "INSERT INTO order_audit VALUES
                 (3,'TEST_CODE_ORPHAN_PLAN','PaperTrade','TEST_CODE orphan','buy',
                  'TEST_CODE_600002',20.0,20.0,100,'2026-08-21T14:30:00+08:00',
                  'Filled',NULL,'2026-08-22 00:00:02')",
                [],
            )
            .unwrap();
        rehash_audits(&orphan);
        drop(orphan);
        assert!(matches!(
            AttributionReplayLoader::new(&orphan_path).load(&request_with_no_fees()),
            Err(AttributionReplayError::FailedIntegrity {
                code: AttributionIntegrityFailure::TerminalBinding,
                ..
            })
        ));
        remove_database(orphan_path);

        let other_source_path = complete_database("other_source_filled");
        let other_source = Connection::open(&other_source_path).unwrap();
        other_source
            .execute(
                "INSERT INTO order_audit VALUES
                 (3,'TEST_CODE_PLAN_2','TEST_CODE_BROKER','TEST_CODE unrelated source','sell',
                  'TEST_CODE_600001',11.0,11.0,100,'2026-08-21T14:20:01+08:00',
                  'Filled',NULL,'2026-08-22 00:00:02')",
                [],
            )
            .unwrap();
        rehash_audits(&other_source);
        drop(other_source);
        AttributionReplayLoader::new(&other_source_path)
            .load(&request_with_no_fees())
            .expect("Filled owned by another source is outside the PaperTrade join");
        remove_database(other_source_path);
    }

    #[test]
    fn rejected_retry_never_supplies_terminal_time_and_bad_rfc3339_fails_integrity() {
        let rejected_path = complete_database("rejected_retry");
        let rejected = Connection::open(&rejected_path).unwrap();
        rejected
            .execute(
                "UPDATE order_audit SET outcome='Rejected', failure_reason='TEST_CODE rejected'
                 WHERE id=2",
                [],
            )
            .unwrap();
        rehash_audits(&rejected);
        drop(rejected);
        assert!(matches!(
            AttributionReplayLoader::new(&rejected_path).load(&request_with_no_fees()),
            Err(AttributionReplayError::Unavailable {
                code: AttributionUnavailable::TradeTimeUnavailable,
                ..
            })
        ));
        remove_database(rejected_path);

        let missing_time_path = complete_database("missing_quote_time");
        let missing_time = Connection::open(&missing_time_path).unwrap();
        missing_time
            .execute(
                "UPDATE order_audit SET quote_observed_at=NULL WHERE id=2",
                [],
            )
            .unwrap();
        rehash_audits(&missing_time);
        drop(missing_time);
        assert!(matches!(
            AttributionReplayLoader::new(&missing_time_path).load(&request_with_no_fees()),
            Err(AttributionReplayError::Unavailable {
                code: AttributionUnavailable::TradeTimeUnavailable,
                ..
            })
        ));
        remove_database(missing_time_path);

        let bad_time_path = complete_database("bad_rfc3339");
        let bad_time = Connection::open(&bad_time_path).unwrap();
        bad_time
            .execute(
                "UPDATE order_audit SET quote_observed_at='2026-08-21 14:20:00' WHERE id=2",
                [],
            )
            .unwrap();
        rehash_audits(&bad_time);
        drop(bad_time);
        assert!(matches!(
            AttributionReplayLoader::new(&bad_time_path).load(&request_with_no_fees()),
            Err(AttributionReplayError::FailedIntegrity {
                code: AttributionIntegrityFailure::TerminalBinding,
                ..
            })
        ));
        remove_database(bad_time_path);
    }

    #[test]
    fn loader_validates_future_source_before_range_and_retains_fifo_prehistory() {
        let path = complete_database("source_before_filter");
        let evidence = AttributionReplayLoader::new(&path)
            .load(&AttributionReplayRequest {
                from: date("2026-08-21"),
                to: date("2026-08-21"),
                required_trading_dates: vec![date("2026-08-21")],
                fee_ledger: None,
            })
            .unwrap();
        assert_eq!(
            evidence
                .fills
                .iter()
                .map(|fill| fill.fill.id)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        let connection = Connection::open(&path).unwrap();
        connection
            .execute(
                "INSERT INTO paper_trades VALUES
                 (3,'TEST_CODE_PLAN_3','TEST_CODE_600001','TEST_CODE公司','hold',12.0,
                  100,'Filled',12.0,'ExitByRule','2026-08-25 10:00:00')",
                [],
            )
            .unwrap();
        drop(connection);
        assert!(matches!(
            AttributionReplayLoader::new(&path).load(&request_with_no_fees()),
            Err(AttributionReplayError::FailedIntegrity {
                code: AttributionIntegrityFailure::PaperTradeSource,
                ..
            })
        ));
        remove_database(path);

        let t1_path = complete_database("future_t1");
        let t1 = Connection::open(&t1_path).unwrap();
        t1.execute_batch(
            "INSERT INTO paper_trades VALUES
             (3,'TEST_CODE_PLAN_3','TEST_CODE_600001','TEST_CODE公司','buy',12.0,
              100,'Filled',12.0,'Breakout','2026-08-25 10:00:00');
             INSERT INTO paper_trades VALUES
             (4,'TEST_CODE_PLAN_4','TEST_CODE_600001','TEST_CODE公司','sell',12.1,
              100,'Filled',12.1,'ExitByRule','2026-08-25 14:00:00');",
        )
        .unwrap();
        drop(t1);
        let error = AttributionReplayLoader::new(&t1_path)
            .load(&request_with_no_fees())
            .unwrap_err();
        assert!(matches!(
            error,
            AttributionReplayError::FailedIntegrity {
                code: AttributionIntegrityFailure::PaperTradeSource,
                ..
            }
        ));
        assert!(error.to_string().contains("T+1"));
        remove_database(t1_path);
    }

    #[test]
    fn later_fractional_fills_are_validated_before_an_earlier_projection() {
        let path = complete_database("later_fractional_source");
        let connection = Connection::open(&path).unwrap();
        let previous_hash: String = connection
            .query_row(
                "SELECT record_hash FROM order_audit_chain WHERE order_audit_id=2",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let buy_hash = append_filled_pair(
            &connection,
            3,
            "TEST_CODE_PLAN_3",
            "buy",
            12.0,
            "2026-08-25T10:00:00.123+08:00",
            "2026-08-25 10:00:00.123",
            &previous_hash,
        );
        append_filled_pair(
            &connection,
            4,
            "TEST_CODE_PLAN_4",
            "sell",
            12.1,
            "2026-08-26T10:00:00.456+08:00",
            "2026-08-26 10:00:00.456",
            &buy_hash,
        );
        drop(connection);

        let evidence = AttributionReplayLoader::new(&path)
            .load(&request_with_no_fees())
            .expect("valid later fractional source must not poison earlier projection");
        assert_eq!(
            evidence
                .fills
                .iter()
                .map(|fill| fill.fill.id)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        remove_database(path);
    }

    #[test]
    fn close_manifest_is_order_independent_and_missing_or_bad_close_fails_closed() {
        let entries = vec![
            StockCloseEvidence {
                code: "TEST_CODE_B".to_owned(),
                date: date("2026-08-21"),
                close: 20.0,
                data_source: Some("TEST_CODE_SOURCE".to_owned()),
                created_at: "2026-08-22".to_owned(),
                updated_at: "2026-08-22".to_owned(),
            },
            StockCloseEvidence {
                code: "TEST_CODE_A".to_owned(),
                date: date("2026-08-20"),
                close: 10.0,
                data_source: None,
                created_at: "2026-08-22".to_owned(),
                updated_at: "2026-08-22".to_owned(),
            },
        ];
        let mut reordered = entries.clone();
        reordered.reverse();
        assert_eq!(
            canonical_stock_close_manifest_hash(&entries),
            canonical_stock_close_manifest_hash(&reordered)
        );

        let missing_path = complete_database("missing_close");
        Connection::open(&missing_path)
            .unwrap()
            .execute("DELETE FROM stock_daily WHERE date='2026-08-21'", [])
            .unwrap();
        assert!(matches!(
            AttributionReplayLoader::new(&missing_path).load(&request_with_no_fees()),
            Err(AttributionReplayError::Unavailable {
                code: AttributionUnavailable::StockCloseUnavailable,
                ..
            })
        ));
        remove_database(missing_path);

        let null_path = complete_database("null_close");
        Connection::open(&null_path)
            .unwrap()
            .execute("UPDATE stock_daily SET close=NULL WHERE id=2", [])
            .unwrap();
        assert!(matches!(
            AttributionReplayLoader::new(&null_path).load(&request_with_no_fees()),
            Err(AttributionReplayError::Unavailable {
                code: AttributionUnavailable::StockCloseUnavailable,
                ..
            })
        ));
        remove_database(null_path);

        let duplicate_path = complete_database("duplicate_close");
        Connection::open(&duplicate_path)
            .unwrap()
            .execute(
                "INSERT INTO stock_daily VALUES
                 (3,'TEST_CODE_600001','2026-08-21',11.2,'TEST_CODE_SOURCE',
                  '2026-08-22','2026-08-22')",
                [],
            )
            .unwrap();
        assert!(matches!(
            AttributionReplayLoader::new(&duplicate_path).load(&request_with_no_fees()),
            Err(AttributionReplayError::FailedIntegrity {
                code: AttributionIntegrityFailure::StockCloseSource,
                ..
            })
        ));
        remove_database(duplicate_path);

        let invalid_date_path = complete_database("invalid_close_date");
        Connection::open(&invalid_date_path)
            .unwrap()
            .execute("UPDATE stock_daily SET date='2026-02-30' WHERE id=2", [])
            .unwrap();
        assert!(matches!(
            AttributionReplayLoader::new(&invalid_date_path).load(&request_with_no_fees()),
            Err(AttributionReplayError::Unavailable {
                code: AttributionUnavailable::StockCloseUnavailable,
                ..
            })
        ));
        remove_database(invalid_date_path);

        for (label, value) in [
            ("zero_close", "0.0"),
            ("negative_close", "-1.0"),
            ("infinite_close", "1e999"),
        ] {
            let path = complete_database(label);
            Connection::open(&path)
                .unwrap()
                .execute(
                    &format!("UPDATE stock_daily SET close={value} WHERE id=2"),
                    [],
                )
                .unwrap();
            assert!(matches!(
                AttributionReplayLoader::new(&path).load(&request_with_no_fees()),
                Err(AttributionReplayError::FailedIntegrity {
                    code: AttributionIntegrityFailure::StockCloseSource,
                    ..
                })
            ));
            remove_database(path);
        }
    }

    #[test]
    fn required_close_requires_a_nonblank_source_identity() {
        for (label, source) in [
            ("missing_close_source", "NULL"),
            ("blank_close_source", "'   '"),
        ] {
            let path = complete_database(label);
            Connection::open(&path)
                .unwrap()
                .execute(
                    &format!("UPDATE stock_daily SET data_source={source} WHERE id=2"),
                    [],
                )
                .unwrap();
            assert!(matches!(
                AttributionReplayLoader::new(&path).load(&request_with_no_fees()),
                Err(AttributionReplayError::Unavailable {
                    code: AttributionUnavailable::StockCloseUnavailable,
                    ..
                })
            ));
            remove_database(path);
        }
    }

    #[test]
    fn close_loading_uses_only_range_relevant_exact_keys() {
        let unrelated_path = complete_database("unrelated_bad_stock_row");
        Connection::open(&unrelated_path)
            .unwrap()
            .execute(
                "INSERT INTO stock_daily VALUES
                 (3,'TEST_CODE_OTHER','2026-08-21',X'00','TEST_CODE_SOURCE',
                  '2026-08-22','2026-08-22')",
                [],
            )
            .unwrap();
        AttributionReplayLoader::new(&unrelated_path)
            .load(&request_with_no_fees())
            .expect("unrelated bad stock row must never be decoded");
        remove_database(unrelated_path);

        let preclosed_path = complete_database("fully_closed_before_range");
        let evidence = AttributionReplayLoader::new(&preclosed_path)
            .load(&AttributionReplayRequest {
                from: date("2026-08-25"),
                to: date("2026-08-25"),
                required_trading_dates: vec![date("2026-08-25")],
                fee_ledger: None,
            })
            .expect("fully closed pre-range lifecycle requires no future close");
        assert!(evidence.stock_closes.entries.is_empty());
        remove_database(preclosed_path);

        let required_bad_path = complete_database("required_bad_stock_row");
        Connection::open(&required_bad_path)
            .unwrap()
            .execute("UPDATE stock_daily SET close=X'00' WHERE id=2", [])
            .unwrap();
        assert!(matches!(
            AttributionReplayLoader::new(&required_bad_path).load(&request_with_no_fees()),
            Err(AttributionReplayError::FailedIntegrity {
                code: AttributionIntegrityFailure::SourceRead,
                ..
            }) | Err(AttributionReplayError::FailedIntegrity {
                code: AttributionIntegrityFailure::StockCloseSource,
                ..
            })
        ));
        remove_database(required_bad_path);
    }

    #[test]
    fn exact_close_query_chunks_below_the_sqlite_variable_limit() {
        let path = complete_database("chunked_exact_close_keys");
        let mut connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "DELETE FROM order_audit_chain WHERE order_audit_id=2;
                 DELETE FROM order_audit WHERE id=2;
                 DELETE FROM paper_trades WHERE id=2;
                 DELETE FROM stock_daily;",
            )
            .unwrap();
        let first = date("2026-08-20");
        let required_dates = (0..=STOCK_CLOSE_KEYS_PER_QUERY)
            .map(|offset| first + chrono::Duration::days(offset as i64))
            .collect::<Vec<_>>();
        let transaction = connection.transaction().unwrap();
        for (offset, required_date) in required_dates.iter().enumerate() {
            transaction
                .execute(
                    "INSERT INTO stock_daily VALUES
                     (?1,'TEST_CODE_600001',?2,10.0,'TEST_CODE_SOURCE',
                      '2026-08-22','2026-08-22')",
                    params![
                        offset as i64 + 1,
                        required_date.format("%Y-%m-%d").to_string()
                    ],
                )
                .unwrap();
        }
        transaction.commit().unwrap();
        drop(connection);

        let evidence = AttributionReplayLoader::new(&path)
            .load(&AttributionReplayRequest {
                from: first,
                to: *required_dates.last().unwrap(),
                required_trading_dates: required_dates,
                fee_ledger: None,
            })
            .expect("401 exact keys must cross the fixed 400-key query boundary");
        assert_eq!(
            evidence.stock_closes.entries.len(),
            STOCK_CLOSE_KEYS_PER_QUERY + 1
        );
        remove_database(path);
    }

    fn fee(fill_id: i64, adverse_cost: f64) -> FillFeeEvidence {
        let mut evidence = FillFeeEvidence {
            fill_id,
            adverse_cost,
            source: "TEST_CODE_BROKER_LEDGER".to_owned(),
            authority: "TEST_CODE_SIGNED_EXPORT".to_owned(),
            evidence_id: format!("TEST_CODE_FEE_{fill_id}"),
            evidence_hash: String::new(),
        };
        evidence.evidence_hash = canonical_fill_fee_evidence_hash(&evidence);
        evidence
    }

    #[test]
    fn fee_ledger_requires_exact_authoritative_one_to_one_evidence() {
        let path = complete_database("fees");
        let mut request = request_with_no_fees();
        request.fee_ledger = Some(AuthoritativeFillFeeLedger {
            entries: vec![fee(1, 1.25), fee(2, 1.50)],
        });
        assert!(matches!(
            AttributionReplayLoader::new(&path)
                .load(&request)
                .unwrap()
                .fees,
            FeeEvidenceAvailability::Available(_)
        ));

        let invalid_ledgers = vec![
            vec![fee(1, 1.25)],
            vec![fee(1, 1.25), fee(1, 1.50)],
            vec![fee(1, 1.25), fee(2, 1.50), fee(3, 1.0)],
            {
                let mut entries = vec![fee(1, 1.25), fee(2, 1.50)];
                entries[0].source.clear();
                entries
            },
            {
                let mut entries = vec![fee(1, 1.25), fee(2, 1.50)];
                entries[0].evidence_hash = "A".repeat(64);
                entries
            },
            vec![fee(1, -1.0), fee(2, 1.50)],
            vec![fee(1, f64::NAN), fee(2, 1.50)],
        ];
        for entries in invalid_ledgers {
            request.fee_ledger = Some(AuthoritativeFillFeeLedger { entries });
            assert!(matches!(
                AttributionReplayLoader::new(&path).load(&request),
                Err(AttributionReplayError::FailedIntegrity {
                    code: AttributionIntegrityFailure::FeeEvidence,
                    ..
                })
            ));
        }
        remove_database(path);
    }
}
