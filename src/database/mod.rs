//! Registered business rules: BR-001, BR-016, BR-017, BR-050, BR-066, BR-129.
// -*- coding: utf-8 -*-
//! ===================================
//! A股自选股智能分析系统 - 数据库管理
//! ===================================
//!
//! 职责：
//! 1. 管理 SQLite 数据库连接（单例模式）
//! 2. 提供数据存取接口
//! 3. 实现智能更新逻辑（断点续传）

use chrono::NaiveDate;
use diesel::prelude::*;
use diesel::r2d2::{ConnectionManager, Pool, PooledConnection};
use log::info;
use once_cell::sync::OnceCell;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::models::MaStatus;

pub(super) type DbPool = Pool<ConnectionManager<SqliteConnection>>;
pub(super) type DbConnection = PooledConnection<ConnectionManager<SqliteConnection>>;

// ============================================================================
// 数据库管理器 - 单例模式
// ============================================================================

/// 数据库管理器
///
/// 职责：
/// 1. 管理数据库连接池
/// 2. 提供数据存取操作
/// 3. 实现断点续传逻辑
pub struct DatabaseManager {
    pool: DbPool,
}

static DB_INSTANCE: OnceCell<DatabaseManager> = OnceCell::new();

#[cfg(test)]
fn unit_test_database_path() -> &'static PathBuf {
    use once_cell::sync::Lazy;
    use std::time::{SystemTime, UNIX_EPOCH};

    static PATH: Lazy<PathBuf> = Lazy::new(|| {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock must be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "stock-analysis-unit-{}-{nonce}.db",
            std::process::id()
        ))
    });
    &PATH
}

#[cfg(test)]
fn unit_test_init_lock() -> &'static std::sync::Mutex<()> {
    use once_cell::sync::Lazy;
    static LOCK: Lazy<std::sync::Mutex<()>> = Lazy::new(|| std::sync::Mutex::new(()));
    &LOCK
}

#[derive(QueryableByName)]
struct JournalModeRow {
    #[diesel(sql_type = diesel::sql_types::Text)]
    journal_mode: String,
}

const SQLITE_POOL_SIZE: u32 = 10;

fn validate_required_text(field: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{field} 不能为空"))
    } else {
        Ok(())
    }
}

fn validate_date_text(field: &str, value: &str) -> Result<(), String> {
    validate_required_text(field, value)?;
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map(|_| ())
        .map_err(|error| format!("{field} 不是合法 YYYY-MM-DD 日期: {value}: {error}"))
}

fn validate_evidence_code(code: &str) -> Result<(), String> {
    validate_required_text("stock_code", code)?;
    if !code
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        return Err(format!("stock_code 含非法字符: {code:?}"));
    }
    crate::risk::env_guard::validate_symbol_for_current_env(code)
}

fn invalid_input(error: String) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, error)
}

fn configure_sqlite_connection(
    conn: &mut SqliteConnection,
) -> Result<(), Box<dyn std::error::Error>> {
    for (label, statement) in [
        ("busy_timeout=5000", "PRAGMA busy_timeout = 5000"),
        ("synchronous=NORMAL", "PRAGMA synchronous = NORMAL"),
        (
            "wal_autocheckpoint=1000",
            "PRAGMA wal_autocheckpoint = 1000",
        ),
    ] {
        diesel::sql_query(statement)
            .execute(conn)
            .map_err(|error| {
                std::io::Error::other(format!("SQLite PRAGMA {label} failed: {error}"))
            })?;
    }
    Ok(())
}

pub mod factor_snapshot;
pub mod repository;
// v12 MVP-5 §8.1
pub(crate) mod agent_logs;
pub mod concepts; // v15.1: 公开供 push_templates 集成使用
pub mod execution_tracking;
mod kline;
mod lhb;
pub(crate) use lhb::validate_lhb_records;
pub mod order_audit;
mod positions;
// v12 PR1-1.5 (BR-021)
pub mod account_mode_log;
/// BR-103 real-account evidence boundary; nullable fields stay nullable.
pub mod account_snapshot;
// v12 PR3-3.2/3.3 (BR-023/024)
pub mod position_shares;
pub mod user_position_snapshot;
pub mod closing_valuation;

// ============================================================================
// 数据库管理器 - 单例模式
// ============================================================================

impl DatabaseManager {
    /// 初始化数据库管理器
    ///
    /// # Arguments
    ///
    /// * `db_path` - 数据库文件路径（如果为None，默认使用 "./data/stock.db"）
    pub fn init(db_path: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
        #[cfg(test)]
        let _init_guard = unit_test_init_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        #[cfg(test)]
        if DB_INSTANCE.get().is_some() {
            return Ok(());
        }

        #[cfg(test)]
        let path = {
            let _ = db_path;
            unit_test_database_path().clone()
        };

        #[cfg(not(test))]
        let path = db_path.unwrap_or_else(|| {
            let mut p = PathBuf::from("./data");
            std::fs::create_dir_all(&p).ok();
            p.push("stock.db");
            p
        });

        let database_url = path.to_string_lossy().to_string();
        info!("初始化数据库: {}", database_url);

        // WAL is database-wide and requires a lock. Configure it once before
        // r2d2 opens connections concurrently.
        let mut bootstrap_conn = SqliteConnection::establish(&database_url)?;
        diesel::sql_query("PRAGMA busy_timeout = 5000").execute(&mut bootstrap_conn)?;
        let journal_mode = diesel::sql_query("PRAGMA journal_mode = WAL")
            .get_result::<JournalModeRow>(&mut bootstrap_conn)?
            .journal_mode;
        if !journal_mode.eq_ignore_ascii_case("wal") {
            return Err(
                format!("SQLite journal_mode mismatch: expected WAL, got {journal_mode}").into(),
            );
        }
        drop(bootstrap_conn);

        let manager = ConnectionManager::<SqliteConnection>::new(database_url);
        let pool = Pool::builder().max_size(SQLITE_POOL_SIZE).build(manager)?;

        // r2d2 retries `CustomizeConnection::on_acquire` errors. Configure
        // every initial connection directly instead, keeping all of them
        // checked out so each of the ten distinct connections is verified.
        // Any PRAGMA failure therefore propagates from `init` immediately.
        let mut initial_connections = Vec::with_capacity(SQLITE_POOL_SIZE as usize);
        for _ in 0..SQLITE_POOL_SIZE {
            let mut conn = pool.get()?;
            configure_sqlite_connection(&mut conn)?;
            initial_connections.push(conn);
        }

        // 运行迁移
        let mut conn = initial_connections
            .pop()
            .ok_or_else(|| std::io::Error::other("SQLite pool initialized without connections"))?;
        drop(initial_connections);
        Self::run_migrations(&mut conn)?;

        info!("SQLite PRAGMAs 已设置: WAL + busy_timeout=5000");

        // 创建 agent_scratchpad 表 (Agent 内部思考和工具执行记录)
        diesel::sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS agent_scratchpad (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                step INTEGER NOT NULL,
                log_type TEXT NOT NULL,
                content TEXT,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&mut *conn)?;
        diesel::sql_query(
            r#"
            CREATE TRIGGER IF NOT EXISTS agent_scratchpad_no_update
            BEFORE UPDATE ON agent_scratchpad
            BEGIN
                SELECT RAISE(ABORT, 'agent_scratchpad is append-only');
            END;
            "#,
        )
        .execute(&mut *conn)?;
        diesel::sql_query(
            r#"
            CREATE TRIGGER IF NOT EXISTS agent_scratchpad_no_delete
            BEFORE DELETE ON agent_scratchpad
            BEGIN
                SELECT RAISE(ABORT, 'agent_scratchpad is append-only');
            END;
            "#,
        )
        .execute(&mut *conn)?;

        // BR-126 / v16.x R3: the push pool is a durable audit boundary shared by
        // push_recorder and both intraday/evening consumers. Initialization must
        // fail if any part of the table/index contract cannot be installed.
        diesel::sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS pushed_stocks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                push_time TIMESTAMP NOT NULL,
                push_kind TEXT NOT NULL,
                code TEXT NOT NULL,
                name TEXT NOT NULL,
                push_price REAL NOT NULL,
                metric_json TEXT NOT NULL,
                source TEXT NOT NULL,
                consumed_at TIMESTAMP,
                consumed_by TEXT,
                outcome TEXT,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&mut *conn)?;
        for statement in [
            "CREATE INDEX IF NOT EXISTS idx_pushed_stocks_time ON pushed_stocks (push_time, push_kind)",
            "CREATE INDEX IF NOT EXISTS idx_pushed_stocks_code ON pushed_stocks (code, push_time)",
            "CREATE INDEX IF NOT EXISTS idx_pushed_stocks_uncon ON pushed_stocks (consumed_at) WHERE consumed_at IS NULL",
        ] {
            diesel::sql_query(statement).execute(&mut *conn)?;
        }

        // 创建 stock_concepts 表（概念板块标签缓存，产业链聚类用）
        diesel::sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS stock_concepts (
                code TEXT PRIMARY KEY,
                concepts TEXT NOT NULL,
                updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&mut *conn)?;

        // 创建 chain_daily 表（每日涨停主线簇，供单股分析注入主线上下文 + 主线生命周期追踪）
        diesel::sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS chain_daily (
                date TEXT NOT NULL,
                concept TEXT NOT NULL,
                stocks TEXT NOT NULL,
                continuation_count INTEGER NOT NULL DEFAULT 0,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (date, concept)
            )
            "#,
        )
        .execute(&mut *conn)?;

        // B-002 板块联动归因 (Board hit) 落库表 — 与 chain_daily 并列,
        //       供 NewsCatalyst 推送读取今日 top cluster.
        diesel::sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS board_rotation_daily (
                date TEXT NOT NULL,
                board_code TEXT NOT NULL,
                board_name TEXT NOT NULL,
                news_title TEXT NOT NULL,
                board_change_pct REAL NOT NULL DEFAULT 0,
                board_main_net_pct REAL NOT NULL DEFAULT 0,
                stocks TEXT NOT NULL,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (date, board_code)
            )
            "#,
        )
        .execute(&mut *conn)?;

        // B-003 事件抽取去重 (simhash + LCS) — 跨批次跨日去重,
        //       防「苹果折叠屏」类事件在 3+ 天内重复推送.
        diesel::sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS event_seen_simhash (
                simhash INTEGER NOT NULL,
                title TEXT NOT NULL,
                seen_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (simhash)
            )
            "#,
        )
        .execute(&mut *conn)?;
        // CR-8 (review): get_recent_event_seen 用 `WHERE seen_at >= ?` 全表扫,
        //              表行数 > 5000 时变慢. 加 (seen_at) 索引.
        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS idx_event_seen_simhash_seen_at \
             ON event_seen_simhash (seen_at)",
        )
        .execute(&mut *conn)?;

        // 主题新闻去同质化历史（跨重启持久化）
        diesel::sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS topic_novelty_history (
                signature TEXT PRIMARY KEY,
                created_at INTEGER NOT NULL
            )
            "#,
        )
        .execute(&mut *conn)?;
        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS ix_topic_novelty_created_at ON topic_novelty_history(created_at)",
        )
        .execute(&mut *conn)?;

        drop(conn);
        let db = DatabaseManager { pool };
        DB_INSTANCE.set(db).map_err(|_| "数据库已经初始化")?;
        info!("数据库初始化完成");

        Ok(())
    }

    /// 获取数据库管理器单例
    pub fn get() -> &'static DatabaseManager {
        DB_INSTANCE
            .get()
            .expect("数据库未初始化，请先调用 DatabaseManager::init()")
    }

    /// 尝试获取数据库管理器单例（返回 Option，不 panic）.
    /// review #14: 取代之前各处 `catch_unwind(DatabaseManager::get)` 的反 pattern.
    /// catch_unwind 强制 panic = unwind + 还要 AssertUnwindSafe wrap, 而且静默吞
    /// init 失败, 让 operator 看到「数据全空」但不知道 DB 没起来.
    /// 显式 Option 让调用方必须处理 None 路径 (早返回 / log warn).
    pub fn try_get() -> Option<&'static DatabaseManager> {
        DB_INSTANCE.get()
    }

    /// 在 DB 已初始化前提下执行闭包; 否则记录一次 warn 并返回 None.
    /// review #15: 取代 13+ 处 `let Some(db) = DatabaseManager::try_get() else { return; };`
    /// 重复模板. 调用方写 `DatabaseManager::with_db(|db| { ... })?` 比手写 Option 处理更清晰.
    ///
    /// 闭包返回 `Option<T>` 表示 DB 操作本身的成功/失败 (None = 操作失败/缺数据, 不一定是 DB 不可用).
    /// 用 `Once` 状态确保 DB 未初始化只 warn 一次 (避免每 tick 重复刷屏).
    pub fn with_db<F, T>(caller: &str, f: F) -> Option<T>
    where
        F: FnOnce(&DatabaseManager) -> Option<T>,
    {
        match DB_INSTANCE.get() {
            Some(db) => f(db),
            None => {
                use std::sync::atomic::{AtomicBool, Ordering};
                static WARNED: AtomicBool = AtomicBool::new(false);
                if !WARNED.swap(true, Ordering::Relaxed) {
                    log::warn!(
                        "[{}] DatabaseManager 未初始化, 跳过 (后续同路径 DB 错误不再 warn)",
                        caller
                    );
                }
                None
            }
        }
    }

    /// 获取数据库连接
    pub fn get_conn(&self) -> Result<DbConnection, Box<dyn std::error::Error>> {
        let mut conn = self.pool.get()?;
        configure_sqlite_connection(&mut conn)?;
        Ok(conn)
    }

    /// 给已存在的表增量添加列（如果列不存在）。
    /// SQLite 没有原生的 `ADD COLUMN IF NOT EXISTS`；通过 PRAGMA table_info 读列名判断。
    /// 用于把老库升级到新 schema，不破坏现有数据。
    ///
    /// 修复 (2026-07-05 MVP0-A): 如果表本身不存在 (CREATE 还没跑到), 静默跳过,
    ///   等表建好后再 ALTER. 避免 "no such table: X" 错误导致 init 失败.
    pub fn add_column_if_missing(
        conn: &mut SqliteConnection,
        table: &str,
        column: &str,
        column_def: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if !table_exists(conn, table)? {
            // 表还没建, 跳过. 等 CREATE TABLE 之后再回头补.
            return Ok(());
        }
        if column_exists(conn, table, column)? {
            return Ok(());
        }
        let alter = format!("ALTER TABLE {} ADD COLUMN {} {}", table, column, column_def);
        diesel::sql_query(&alter).execute(conn)?;
        Ok(())
    }

    /// review #16: news_items 详存 (与 news_dedup 5min 去重互补, 永久详存)
    pub const NEWS_ITEMS_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS news_items (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source TEXT NOT NULL,
    external_id TEXT NOT NULL,
    category TEXT NOT NULL,
    code TEXT,
    title TEXT NOT NULL,
    summary TEXT,
    url TEXT NOT NULL,
    source_name TEXT,
    published_at INTEGER NOT NULL,
    fetched_at INTEGER NOT NULL,
    content_hash TEXT NOT NULL,
    UNIQUE(source, external_id)
);
CREATE INDEX IF NOT EXISTS idx_news_items_code_time ON news_items(code, published_at);
CREATE INDEX IF NOT EXISTS idx_news_items_published ON news_items(published_at);
"#;

    /// 运行数据库迁移
    #[cfg(test)]
    pub(crate) fn run_migrations_for_test(
        conn: &mut SqliteConnection,
    ) -> Result<(), Box<dyn std::error::Error>> {
        Self::run_migrations(conn)
    }

    fn run_migrations(conn: &mut SqliteConnection) -> Result<(), Box<dyn std::error::Error>> {
        user_position_snapshot::create_schema(conn).map_err(std::io::Error::other)?;
        closing_valuation::create_schema(conn).map_err(std::io::Error::other)?;
        // 创建 stock_daily 表
        diesel::sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS stock_daily (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                code TEXT NOT NULL,
                date DATE NOT NULL,
                open REAL,
                high REAL,
                low REAL,
                close REAL,
                volume REAL,
                amount REAL,
                pct_chg REAL,
                ma5 REAL,
                ma10 REAL,
                ma20 REAL,
                volume_ratio REAL,
                data_source TEXT,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                is_limit_up TINYINT NOT NULL DEFAULT 0,
                is_limit_down TINYINT NOT NULL DEFAULT 0,
                is_suspended TINYINT NOT NULL DEFAULT 0,
                UNIQUE(code, date)
            )
            "#,
        )
        .execute(&mut *conn)?;

        // 老库升级：增量添加 3 列（QUANT_ANALYST_REVIEW §1.1）
        Self::add_column_if_missing(
            conn,
            "stock_daily",
            "is_limit_up",
            "TINYINT NOT NULL DEFAULT 0",
        )?;
        Self::add_column_if_missing(
            conn,
            "stock_daily",
            "is_limit_down",
            "TINYINT NOT NULL DEFAULT 0",
        )?;
        Self::add_column_if_missing(
            conn,
            "stock_daily",
            "is_suspended",
            "TINYINT NOT NULL DEFAULT 0",
        )?;

        // 老库升级：增量添加 6 列 (修复 P1.3 trades 业绩归因)
        // 量化分析师要求: 必须能算真实 PnL (扣除 commission/stamp_tax/slippage)
        Self::add_column_if_missing(conn, "trades", "commission_amount", "REAL DEFAULT 0")?;
        Self::add_column_if_missing(conn, "trades", "stamp_tax_amount", "REAL DEFAULT 0")?;
        Self::add_column_if_missing(conn, "trades", "slippage_amount", "REAL DEFAULT 0")?;
        Self::add_column_if_missing(conn, "trades", "realized_pnl", "REAL DEFAULT 0")?;
        Self::add_column_if_missing(conn, "trades", "strategy_tag", "TEXT DEFAULT ''")?;
        Self::add_column_if_missing(conn, "trades", "signal_id", "TEXT DEFAULT ''")?;

        // 创建索引
        diesel::sql_query("CREATE INDEX IF NOT EXISTS ix_stock_daily_code ON stock_daily(code)")
            .execute(&mut *conn)?;

        diesel::sql_query("CREATE INDEX IF NOT EXISTS ix_stock_daily_date ON stock_daily(date)")
            .execute(&mut *conn)?;

        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS ix_stock_daily_code_date ON stock_daily(code, date)",
        )
        .execute(&mut *conn)?;

        // 创建 lhb_daily 表
        diesel::sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS lhb_daily (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                code TEXT NOT NULL,
                name TEXT NOT NULL,
                trade_date TEXT NOT NULL,
                reason TEXT NOT NULL,
                pct_change REAL NOT NULL,
                close_price REAL NOT NULL,
                buy_amount REAL NOT NULL,
                sell_amount REAL NOT NULL,
                net_amount REAL NOT NULL,
                total_amount REAL NOT NULL,
                lhb_ratio REAL NOT NULL,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                UNIQUE(code, trade_date)
            )
            "#,
        )
        .execute(&mut *conn)?;

        // 创建龙虎榜索引
        diesel::sql_query("CREATE INDEX IF NOT EXISTS ix_lhb_daily_code ON lhb_daily(code)")
            .execute(&mut *conn)?;

        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS ix_lhb_daily_trade_date ON lhb_daily(trade_date)",
        )
        .execute(&mut *conn)?;

        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS ix_lhb_daily_code_date ON lhb_daily(code, trade_date)",
        )
        .execute(&mut *conn)?;

        // 创建 analysis_result 表
        diesel::sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS analysis_result (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                code TEXT NOT NULL,
                name TEXT NOT NULL,
                date DATE NOT NULL,
                sentiment_score INTEGER NOT NULL,
                operation_advice TEXT NOT NULL,
                trend_prediction TEXT NOT NULL,
                pe_ratio REAL,
                pb_ratio REAL,
                turnover_rate REAL,
                market_cap REAL,
                circulating_cap REAL,
                close_price REAL,
                pct_chg REAL,
                data_source TEXT,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                UNIQUE(code, date)
            )
            "#,
        )
        .execute(&mut *conn)?;

        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS ix_analysis_result_code ON analysis_result(code)",
        )
        .execute(&mut *conn)?;

        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS ix_analysis_result_date ON analysis_result(date)",
        )
        .execute(&mut *conn)?;

        // Phase 1 增量：多维评分 + 风险否决（SQLite 不支持 IF NOT EXISTS，忽略已存在错误）
        for sql in [
            "ALTER TABLE analysis_result ADD COLUMN score_breakdown_json TEXT",
            "ALTER TABLE analysis_result ADD COLUMN original_advice TEXT",
            "ALTER TABLE analysis_result ADD COLUMN veto_flags_json TEXT",
        ] {
            let _ = diesel::sql_query(sql).execute(&mut *conn);
        }

        // 创建 stock_position 表（模拟持仓）
        diesel::sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS stock_position (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                code TEXT NOT NULL,
                name TEXT NOT NULL,
                buy_date TEXT NOT NULL,
                buy_price REAL NOT NULL CHECK(buy_price > 0),
                quantity INTEGER NOT NULL CHECK(quantity > 0 AND quantity % 100 = 0),
                status TEXT NOT NULL DEFAULT 'open',
                sell_date TEXT,
                sell_price REAL CHECK(sell_price IS NULL OR sell_price > 0),
                return_rate REAL,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                st_type TEXT,
                UNIQUE(code, buy_date)
            )
            "#,
        )
        .execute(&mut *conn)?;

        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS ix_stock_position_code ON stock_position(code)",
        )
        .execute(&mut *conn)?;

        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS ix_stock_position_status ON stock_position(status)",
        )
        .execute(&mut *conn)?;
        diesel::sql_query(
            "CREATE TRIGGER IF NOT EXISTS trg_stock_position_order_safety_insert
             BEFORE INSERT ON stock_position
             WHEN NEW.buy_price <= 0 OR NEW.quantity <= 0 OR NEW.quantity % 100 != 0
               OR NEW.buy_price * NEW.quantity > 1000000
               OR (NEW.sell_price IS NOT NULL AND NEW.sell_price <= 0)
             BEGIN SELECT RAISE(ABORT, 'BR-084 invalid stock_position order'); END",
        )
        .execute(&mut *conn)?;
        diesel::sql_query(
            "CREATE TRIGGER IF NOT EXISTS trg_stock_position_order_safety_update
             BEFORE UPDATE OF buy_price, quantity, sell_price ON stock_position
             WHEN NEW.buy_price <= 0 OR NEW.quantity <= 0 OR NEW.quantity % 100 != 0
               OR NEW.buy_price * NEW.quantity > 1000000
               OR (NEW.sell_price IS NOT NULL AND NEW.sell_price <= 0)
             BEGIN SELECT RAISE(ABORT, 'BR-084 invalid stock_position order'); END",
        )
        .execute(&mut *conn)?;

        // BR-123: stock_position.chain_name 缺失值必须保留为 NULL。
        // 旧库可能没有, 用 add_column_if_missing 包一层 (SQLite 1.06 无 ADD COLUMN IF NOT EXISTS)
        Self::add_column_if_missing(conn, "stock_position", "chain_name", "TEXT")?;
        diesel::sql_query(
            "UPDATE stock_position SET chain_name = NULL
             WHERE chain_name IS NOT NULL
               AND (trim(chain_name) = '' OR chain_name = '其他')",
        )
        .execute(&mut *conn)?;
        diesel::sql_query("CREATE INDEX IF NOT EXISTS idx_stock_position_chain_name ON stock_position(chain_name)")
            .execute(&mut *conn)?;

        // v14.1 F7: stock_position 加 st_type 列 (TEXT: 'ST' / '*ST' / NULL)
        // T-16 ST 涨跌幅变更 dispatcher 数据源. 由 --backfill-st-type 从 name 字段回填,
        // 后续 broker/exchange 推送时更新. 无 CHECK 约束 (SQLite ALTER ADD COLUMN 不支持)
        Self::add_column_if_missing(conn, "stock_position", "st_type", "TEXT")?;
        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS idx_stock_position_st_type ON stock_position(st_type)",
        )
        .execute(&mut *conn)
        .ok();

        // trades 表（v3 每笔买卖独立记录，与 stock_position 互补）
        diesel::sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS trades (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                code TEXT NOT NULL,
                name TEXT NOT NULL DEFAULT '',
                direction TEXT NOT NULL CHECK(direction IN ('buy', 'sell')),
                price REAL NOT NULL,
                shares INTEGER NOT NULL,
                amount REAL NOT NULL,
                reason TEXT NOT NULL DEFAULT '',
                traded_at TEXT NOT NULL,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&mut *conn)?;
        diesel::sql_query("CREATE INDEX IF NOT EXISTS ix_trades_code ON trades(code)")
            .execute(&mut *conn)?;

        diesel::sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS order_idempotency (
                business_order_id TEXT PRIMARY KEY NOT NULL,
                reserved_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&mut *conn)?;

        diesel::sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS order_audit (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                business_order_id TEXT NOT NULL,
                source TEXT NOT NULL,
                decision_basis TEXT NOT NULL,
                side TEXT NOT NULL CHECK(side IN ('buy', 'sell', 'cancel')),
                code TEXT NOT NULL,
                requested_price REAL NOT NULL,
                execution_price REAL,
                quantity INTEGER NOT NULL,
                quote_observed_at TEXT,
                outcome TEXT NOT NULL CHECK(outcome IN ('Filled', 'Rejected', 'Canceled')),
                failure_reason TEXT,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&mut *conn)?;
        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS idx_order_audit_business_id
             ON order_audit(business_order_id, created_at)",
        )
        .execute(&mut *conn)?;
        diesel::sql_query(
            "CREATE TRIGGER IF NOT EXISTS trg_order_audit_validate_insert
             BEFORE INSERT ON order_audit
             WHEN trim(NEW.business_order_id) = ''
               OR trim(NEW.source) = ''
               OR trim(NEW.decision_basis) = ''
               OR trim(NEW.code) = ''
               OR (NEW.outcome = 'Filled' AND (
                    NEW.requested_price <= 0
                    OR NEW.execution_price IS NULL
                    OR NEW.execution_price <= 0
                    OR NEW.quantity <= 0
                    OR NEW.quantity % 100 != 0
                    OR NEW.quote_observed_at IS NULL
                    OR trim(NEW.quote_observed_at) = ''
               ))
               OR (NEW.outcome = 'Rejected' AND (
                    NEW.failure_reason IS NULL OR trim(NEW.failure_reason) = ''
               ))
             BEGIN SELECT RAISE(ABORT, 'BR-086 invalid order_audit record'); END",
        )
        .execute(&mut *conn)?;
        diesel::sql_query(
            "CREATE TRIGGER IF NOT EXISTS trg_order_audit_no_update
             BEFORE UPDATE ON order_audit
             BEGIN SELECT RAISE(ABORT, 'BR-086 order_audit is immutable'); END",
        )
        .execute(&mut *conn)?;
        diesel::sql_query(
            "CREATE TRIGGER IF NOT EXISTS trg_order_audit_no_delete
             BEFORE DELETE ON order_audit
             BEGIN SELECT RAISE(ABORT, 'BR-086 order_audit retention is at least five years'); END",
        )
        .execute(&mut *conn)?;
        diesel::sql_query(
            "CREATE TABLE IF NOT EXISTS order_audit_chain (
                order_audit_id INTEGER PRIMARY KEY NOT NULL,
                previous_hash TEXT NOT NULL,
                record_hash TEXT NOT NULL UNIQUE,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY(order_audit_id) REFERENCES order_audit(id)
            )",
        )
        .execute(&mut *conn)?;
        diesel::sql_query(
            "CREATE TRIGGER IF NOT EXISTS trg_order_audit_chain_no_update
             BEFORE UPDATE ON order_audit_chain
             BEGIN SELECT RAISE(ABORT, 'BR-086 order audit hash chain is immutable'); END",
        )
        .execute(&mut *conn)?;
        diesel::sql_query(
            "CREATE TRIGGER IF NOT EXISTS trg_order_audit_chain_no_delete
             BEFORE DELETE ON order_audit_chain
             BEGIN SELECT RAISE(ABORT, 'BR-086 order audit hash chain retention is at least five years'); END",
        )
        .execute(&mut *conn)?;
        order_audit::initialize_order_audit_chain(&mut *conn)?;

        // ledger 表（v3 每日净值快照）
        diesel::sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS ledger (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                date TEXT NOT NULL UNIQUE,
                total_value REAL NOT NULL,
                cash REAL NOT NULL DEFAULT 0,
                market_value REAL NOT NULL DEFAULT 0,
                daily_pnl REAL NOT NULL DEFAULT 0,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&mut *conn)?;

        // BR-103: real-account facts are append-only and preserve nullable P&L.
        account_snapshot::create_schema(conn)?;

        // v5 状态持久化：新闻去重
        diesel::sql_query(
            "CREATE TABLE IF NOT EXISTS news_dedup (key TEXT PRIMARY KEY, created_at TEXT NOT NULL DEFAULT (datetime('now')))",
        ).execute(&mut *conn)?;

        // v5 状态持久化：信号状态
        diesel::sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS signal_state (
                key TEXT PRIMARY KEY,
                state TEXT NOT NULL DEFAULT 'idle',
                last_alert TEXT,
                last_change TEXT,
                daily_important_count INTEGER DEFAULT 0,
                daily_info_count INTEGER DEFAULT 0
            )
            "#,
        )
        .execute(&mut *conn)?;

        // 预测追踪表（Phase 5 预测闭环）
        diesel::sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS prediction_tracker (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                pred_date TEXT NOT NULL,
                target_date TEXT NOT NULL,
                theme_name TEXT,
                stock_code TEXT,
                pred_direction TEXT NOT NULL,
                pred_score REAL,
                pred_detail TEXT,
                actual_change REAL,
                actual_result TEXT,
                hit INTEGER,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&mut *conn)?;
        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS ix_pred_date ON prediction_tracker(pred_date)",
        )
        .execute(&mut *conn)?;

        // v10 P0.1 (G0) — prediction_tracker 加 12 列 (idempotent ALTER, 2026-07-01)
        // 设计: BR-016/017/020 落表; 12 列 = 1+1+3+3+3+1
        // 1+1 = reason / reason_secondary (主/副理由, 枚举, v10 §10.3)
        // 3   = actual_change_t1/t3/t5 (T+1/T+3/T+5 实际涨跌幅, BC-3)
        // 3   = hit_t1/t3/t5 (三窗口命中布尔, BC-3)
        // 3   = market_up_rate_t1/t3/t5 (同日同窗市场基准, BC-1, Q2=B 全市场上涨家数占比)
        // 1   = t1_special_case (停牌/涨停/跌停/正常, BC-3)
        //
        // BR-016/017/020 落表; 幂等: 列已存在时 SQLite 报 "duplicate column name"
        //
        // BUG FIX (codex B1): 之前用 `let _ = ...` 吞错, DB 损坏/权限不足时静默 fail
        // 现在区分: "duplicate column" → 静默 (幂等), 其他错误 → 返回 Err 显式报错
        for col_def in [
            "ALTER TABLE prediction_tracker ADD COLUMN reason TEXT",
            "ALTER TABLE prediction_tracker ADD COLUMN reason_secondary TEXT",
            "ALTER TABLE prediction_tracker ADD COLUMN actual_change_t1 REAL",
            "ALTER TABLE prediction_tracker ADD COLUMN actual_change_t3 REAL",
            "ALTER TABLE prediction_tracker ADD COLUMN actual_change_t5 REAL",
            "ALTER TABLE prediction_tracker ADD COLUMN hit_t1 INTEGER",
            "ALTER TABLE prediction_tracker ADD COLUMN hit_t3 INTEGER",
            "ALTER TABLE prediction_tracker ADD COLUMN hit_t5 INTEGER",
            "ALTER TABLE prediction_tracker ADD COLUMN market_up_rate_t1 REAL",
            "ALTER TABLE prediction_tracker ADD COLUMN market_up_rate_t3 REAL",
            "ALTER TABLE prediction_tracker ADD COLUMN market_up_rate_t5 REAL",
            "ALTER TABLE prediction_tracker ADD COLUMN t1_special_case TEXT",
        ] {
            match diesel::sql_query(col_def).execute(&mut *conn) {
                Ok(_) => {}
                Err(e) => {
                    let err_str = e.to_string();
                    if err_str.contains("duplicate column") {
                        // 幂等: 列已存在, 跳过 (这是 re-run 期望行为)
                    } else {
                        // 真错误: DB 损坏/权限不足/磁盘满, 显式返回 Err
                        eprintln!(
                            "[DatabaseManager::init_schema] ✗ 真错误 (col_def={}): {}",
                            col_def, e
                        );
                        return Err(Box::new(e));
                    }
                }
            }
        }

        // 概念共振表（Phase 4 动态产业链拓扑）
        diesel::sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS concept_cooccurrence (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                stock_code TEXT NOT NULL,
                concept_name TEXT NOT NULL,
                cooccur_weight REAL DEFAULT 0.0,
                evidence_level TEXT DEFAULT 'C',
                last_updated TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                UNIQUE(stock_code, concept_name)
            )
            "#,
        )
        .execute(&mut *conn)?;

        // 创建 factor_snapshot 表（修复 QUANT_ANALYST_REVIEW §1.5）
        diesel::sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS factor_snapshot (
                code TEXT NOT NULL,
                snapshot_date TEXT NOT NULL,
                pe_ttm REAL,
                pb REAL,
                roe REAL,
                market_cap REAL,
                turnover_rate REAL,
                source TEXT,
                created_at TEXT NOT NULL,
                PRIMARY KEY (code, snapshot_date)
            )
            "#,
        )
        .execute(&mut *conn)?;
        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS idx_factor_snapshot_date ON factor_snapshot(snapshot_date)",
        )
        .execute(&mut *conn)?;

        // ===== v12 PR1/PR3 表 (idempotent CREATE IF NOT EXISTS) =====
        // Bug A fix (2026-07-05): 原 run_migrations() 不读 migrations/*.sql,
        // v12 表必须在此手写 CREATE IF NOT EXISTS.

        // account_mode_log
        diesel::sql_query(
            "CREATE TABLE IF NOT EXISTS account_mode_log (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                ts              TIMESTAMP NOT NULL,
                prev_mode       TEXT NOT NULL,
                new_mode        TEXT NOT NULL,
                trigger_reason  TEXT NOT NULL,
                today_pnl_pct   REAL,
                consecutive_n   INTEGER,
                total_pos_cheng INTEGER,
                data_complete   INTEGER NOT NULL DEFAULT 1,
                pushed          INTEGER NOT NULL DEFAULT 0,
                push_attempted_at TIMESTAMP
            )",
        )
        .execute(&mut *conn)?;
        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS idx_account_mode_log_ts ON account_mode_log(ts)",
        )
        .execute(&mut *conn)
        .ok();
        diesel::sql_query("CREATE INDEX IF NOT EXISTS idx_account_mode_log_new_mode ON account_mode_log(new_mode)")
            .execute(&mut *conn).ok();

        // paper_trades (PR3-3.5)
        diesel::sql_query(
            "CREATE TABLE IF NOT EXISTS paper_trades (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                plan_id         TEXT NOT NULL,
                code            TEXT NOT NULL,
                name            TEXT NOT NULL,
                direction       TEXT NOT NULL CHECK(direction IN ('buy','sell')),
                price           REAL NOT NULL CHECK(price > 0),
                quantity        INTEGER NOT NULL CHECK(quantity > 0 AND quantity % 100 = 0),
                status          TEXT NOT NULL CHECK(status IN ('SignalTriggered','Filled','NotFilled','Invalidated')),
                fill_price      REAL CHECK(fill_price IS NULL OR fill_price > 0),
                not_fill_reason TEXT,
                virtual_reason  TEXT NOT NULL,
                account_mode    TEXT NOT NULL,
                data_mode       TEXT NOT NULL,
                ts              TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at      TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
            )",
        )
        .execute(&mut *conn)?;
        diesel::sql_query(
            "CREATE UNIQUE INDEX IF NOT EXISTS uniq_paper_trades_plan_id ON paper_trades(plan_id)",
        )
        .execute(&mut *conn)?;
        diesel::sql_query("CREATE INDEX IF NOT EXISTS idx_paper_trades_code ON paper_trades(code)")
            .execute(&mut *conn)
            .ok();
        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS idx_paper_trades_status ON paper_trades(status)",
        )
        .execute(&mut *conn)
        .ok();
        diesel::sql_query(
            "CREATE TRIGGER IF NOT EXISTS trg_paper_trades_order_safety_insert
             BEFORE INSERT ON paper_trades
             WHEN NEW.price <= 0 OR NEW.quantity <= 0 OR NEW.quantity % 100 != 0
               OR NEW.price * NEW.quantity > 1000000
               OR (NEW.fill_price IS NOT NULL AND NEW.fill_price <= 0)
             BEGIN SELECT RAISE(ABORT, 'BR-084 invalid paper trade order'); END",
        )
        .execute(&mut *conn)?;
        diesel::sql_query(
            "CREATE TRIGGER IF NOT EXISTS trg_paper_trades_order_safety_update
             BEFORE UPDATE OF price, quantity, fill_price ON paper_trades
             WHEN NEW.price <= 0 OR NEW.quantity <= 0 OR NEW.quantity % 100 != 0
               OR NEW.price * NEW.quantity > 1000000
               OR (NEW.fill_price IS NOT NULL AND NEW.fill_price <= 0)
             BEGIN SELECT RAISE(ABORT, 'BR-084 invalid paper trade order'); END",
        )
        .execute(&mut *conn)?;

        // execution_tracking (PR3-3.5)
        diesel::sql_query(
            "CREATE TABLE IF NOT EXISTS execution_tracking (
                id                  INTEGER PRIMARY KEY AUTOINCREMENT,
                paper_trade_id      INTEGER NOT NULL,
                plan_id             TEXT NOT NULL,
                code                TEXT NOT NULL,
                expected_price      REAL NOT NULL,
                actual_change_t1    REAL,
                actual_change_t3    REAL,
                actual_change_t5    REAL,
                mfe                 REAL,
                mae                 REAL,
                t1_special_case     TEXT,
                created_at          TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at          TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
            )",
        )
        .execute(&mut *conn)?;
        diesel::sql_query("CREATE INDEX IF NOT EXISTS idx_execution_tracking_plan_id ON execution_tracking(plan_id)")
            .execute(&mut *conn).ok();
        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS idx_execution_tracking_code ON execution_tracking(code)",
        )
        .execute(&mut *conn)
        .ok();

        // position_adjustments (PR3-3.3)
        diesel::sql_query(
            "CREATE TABLE IF NOT EXISTS position_adjustments (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                code            TEXT NOT NULL,
                delta           INTEGER NOT NULL,
                source          TEXT NOT NULL CHECK(source IN ('manual_confirm','import')),
                reason          TEXT NOT NULL DEFAULT '',
                effective_date  TEXT NOT NULL,
                applied_immediately INTEGER NOT NULL DEFAULT 0,
                operator        TEXT,
                created_at      TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
            )",
        )
        .execute(&mut *conn)?;
        diesel::sql_query("CREATE INDEX IF NOT EXISTS idx_position_adjustments_code ON position_adjustments(code)")
            .execute(&mut *conn).ok();
        diesel::sql_query("CREATE INDEX IF NOT EXISTS idx_position_adjustments_effective ON position_adjustments(effective_date)")
            .execute(&mut *conn).ok();

        // review #16: news_items 详存 schema (idempotent CREATE IF NOT EXISTS)
        diesel::sql_query(Self::NEWS_ITEMS_SCHEMA).execute(&mut *conn)?;

        Ok(())
    }

    /// review #16: 插入单条 NewsItem (INSERT OR IGNORE 走 UNIQUE 约束去重).
    ///
    /// 同 `(source, external_id)` 已存在则跳过 (UNIQUE constraint + INSERT OR IGNORE).
    /// `code` 为 None 时写空串 (schema 列允许 TEXT 无默认值, 实际查询时按 code IS NULL 或 code = '' 过滤).
    /// 时间戳写 unix seconds (i64, 落 INTEGER 列).
    pub fn insert_news_item(
        &self,
        item: &crate::data_provider::news_item::NewsItem,
    ) -> Result<(), String> {
        for (field, value) in [
            ("source", item.source.as_str()),
            ("external_id", item.external_id.as_str()),
            ("category", item.category.as_str()),
            ("title", item.title.as_str()),
            ("url", item.url.as_str()),
            ("source_name", item.source_name.as_str()),
            ("content_hash", item.content_hash.as_str()),
        ] {
            validate_required_text(field, value)?;
        }
        if let Some(code) = item.code.as_deref() {
            validate_evidence_code(code)?;
        }
        if item.fetched_at < item.published_at {
            return Err("fetched_at 不能早于 published_at".to_string());
        }
        let expected_hash =
            crate::data_provider::news_item::content_hash(&item.title, &item.summary);
        if item.content_hash != expected_hash {
            return Err(format!(
                "content_hash 与标题/摘要不一致: expected={expected_hash}, actual={}",
                item.content_hash
            ));
        }

        use diesel::sql_types::{BigInt, Nullable, Text};
        let mut conn = self.get_conn().map_err(|e| e.to_string())?;
        diesel::sql_query(
            "INSERT OR IGNORE INTO news_items (source, external_id, category, code, title, summary, url, source_name, published_at, fetched_at, content_hash) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        )
        .bind::<Text, _>(&item.source)
        .bind::<Text, _>(&item.external_id)
        .bind::<Text, _>(&item.category)
        .bind::<Nullable<Text>, _>(item.code.as_deref())
        .bind::<Text, _>(&item.title)
        .bind::<Text, _>(&item.summary)
        .bind::<Text, _>(&item.url)
        .bind::<Text, _>(&item.source_name)
        .bind::<BigInt, _>(item.published_at.timestamp())
        .bind::<BigInt, _>(item.fetched_at.timestamp())
        .bind::<Text, _>(&item.content_hash)
        .execute(&mut *conn)
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// 保存预测记录（Phase 5 预测闭环）
    ///
    /// v10 P0.2 (BR-016): 加 `reason` + `reason_secondary` 参数, 写盘口时记主/副理由
    /// 向后兼容: reason/reason_secondary 默认为 None (走 v9 旧路径)
    #[allow(
        clippy::too_many_arguments,
        reason = "stable audit persistence boundary mirrors prediction_tracker columns"
    )]
    pub fn save_prediction(
        &self,
        pred_date: &str,
        target_date: &str,
        theme_name: Option<&str>,
        stock_code: Option<&str>,
        direction: &str,
        score: f64,
        detail: Option<&str>,
        reason: Option<&str>,
        reason_secondary: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        validate_date_text("pred_date", pred_date).map_err(invalid_input)?;
        validate_date_text("target_date", target_date).map_err(invalid_input)?;
        validate_required_text("pred_direction", direction).map_err(invalid_input)?;
        if !score.is_finite() || !(0.0..=100.0).contains(&score) {
            return Err(invalid_input(format!("pred_score 超出 0..=100: {score}")).into());
        }
        if theme_name.is_none_or(|theme| theme.trim().is_empty())
            && stock_code.is_none_or(|code| code.trim().is_empty())
        {
            return Err(invalid_input("theme_name 与 stock_code 不能同时缺失".to_string()).into());
        }
        if let Some(code) = stock_code {
            validate_evidence_code(code).map_err(invalid_input)?;
        }
        if let Some(reason) = reason {
            validate_required_text("reason", reason).map_err(invalid_input)?;
        }
        if let Some(reason_secondary) = reason_secondary {
            validate_required_text("reason_secondary", reason_secondary).map_err(invalid_input)?;
        }

        use diesel::sql_types::{Double, Nullable, Text};
        let mut conn = self.get_conn()?;
        diesel::sql_query(
            "INSERT INTO prediction_tracker (pred_date, target_date, theme_name, stock_code, pred_direction, pred_score, pred_detail, reason, reason_secondary) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )
        .bind::<Text, _>(pred_date)
        .bind::<Text, _>(target_date)
        .bind::<Nullable<Text>, _>(theme_name)
        .bind::<Nullable<Text>, _>(stock_code)
        .bind::<Text, _>(direction)
        .bind::<Double, _>(score)
        .bind::<Nullable<Text>, _>(detail)
        .bind::<Nullable<Text>, _>(reason)
        .bind::<Nullable<Text>, _>(reason_secondary)
        .execute(&mut *conn)?;
        Ok(())
    }

    /// v10 P0.2 便捷重载: 不带 reason (旧调用路径, 走 v9 旧行为)
    #[allow(
        clippy::too_many_arguments,
        reason = "legacy compatibility wrapper retains its published scalar call contract"
    )]
    pub fn save_prediction_legacy(
        &self,
        pred_date: &str,
        target_date: &str,
        theme_name: Option<&str>,
        stock_code: Option<&str>,
        direction: &str,
        score: f64,
        detail: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.save_prediction(
            pred_date,
            target_date,
            theme_name,
            stock_code,
            direction,
            score,
            detail,
            None,
            None,
        )
    }

    /// 统计 prediction_tracker 总记录数 (用于 sample_threshold 动态计算)
    pub fn count_predictions(&self) -> Result<i64, Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;
        #[derive(diesel::QueryableByName)]
        struct PredCountRow {
            #[diesel(sql_type = diesel::sql_types::BigInt)]
            cnt: i64,
        }
        let result = diesel::sql_query("SELECT COUNT(*) AS cnt FROM prediction_tracker")
            .get_result::<PredCountRow>(&mut *conn)?;
        Ok(result.cnt)
    }

    /// 统计某 reason 的记录数 (用于 sample_threshold 判断)
    pub fn count_predictions_by_reason(
        &self,
        reason: &str,
    ) -> Result<i64, Box<dyn std::error::Error>> {
        validate_required_text("reason", reason).map_err(invalid_input)?;
        let mut conn = self.get_conn()?;
        #[derive(diesel::QueryableByName)]
        struct PredReasonCountRow {
            #[diesel(sql_type = diesel::sql_types::BigInt)]
            cnt: i64,
        }
        let result =
            diesel::sql_query("SELECT COUNT(*) AS cnt FROM prediction_tracker WHERE reason = ?1")
                .bind::<diesel::sql_types::Text, _>(reason)
                .get_result::<PredReasonCountRow>(&mut *conn)?;
        Ok(result.cnt)
    }

    /// 更新预测结果（次日收盘后回调）
    pub fn update_prediction_result(
        &self,
        pred_date: &str,
        stock_code: Option<&str>,
        actual_change: f64,
        hit: bool,
    ) -> Result<usize, Box<dyn std::error::Error>> {
        validate_date_text("pred_date", pred_date).map_err(invalid_input)?;
        if !actual_change.is_finite() || actual_change.abs() > 20.0 {
            return Err(invalid_input(format!(
                "actual_change 必须有限且绝对值不超过 20%: {actual_change}"
            ))
            .into());
        }
        if let Some(code) = stock_code {
            validate_evidence_code(code).map_err(invalid_input)?;
        }

        use diesel::sql_types::{Double, Integer, Text};
        let mut conn = self.get_conn()?;
        let result_text = if hit { "命中" } else { "未命中" };
        let rows = if let Some(code) = stock_code {
            diesel::sql_query(
                "UPDATE prediction_tracker SET actual_change = ?1, hit = ?2, actual_result = ?3 WHERE pred_date = ?4 AND stock_code = ?5",
            )
            .bind::<Double, _>(actual_change)
            .bind::<Integer, _>(hit as i32)
            .bind::<Text, _>(result_text)
            .bind::<Text, _>(pred_date)
            .bind::<Text, _>(code)
            .execute(&mut *conn)?
        } else {
            diesel::sql_query(
                "UPDATE prediction_tracker SET actual_change = ?1, hit = ?2, actual_result = ?3 WHERE pred_date = ?4 AND theme_name IS NOT NULL AND trim(theme_name) != ''",
            )
            .bind::<Double, _>(actual_change)
            .bind::<Integer, _>(hit as i32)
            .bind::<Text, _>(result_text)
            .bind::<Text, _>(pred_date)
            .execute(&mut *conn)?
        };
        Ok(rows)
    }

    /// 按 stock_code + pred_date 查询 prediction 记录
    ///
    /// 修复 R-1: 用于 verify_predictions 真实回填后, 测试断言 hit/actual_change。
    /// 返回最新的一条 (LIMIT 1) — 同一 (code, pred_date) 只期望一条。
    pub fn get_prediction_by_code_date(
        &self,
        stock_code: &str,
        pred_date: &str,
    ) -> Result<PredictionRow, Box<dyn std::error::Error>> {
        validate_evidence_code(stock_code).map_err(invalid_input)?;
        validate_date_text("pred_date", pred_date).map_err(invalid_input)?;
        let mut conn = self.get_conn()?;
        let row = diesel::sql_query(
            "SELECT id, pred_date, target_date, stock_code, pred_direction, pred_score, actual_change, hit, actual_result FROM prediction_tracker WHERE stock_code = ?1 AND pred_date = ?2 ORDER BY id DESC LIMIT 1",
        )
        .bind::<diesel::sql_types::Text, _>(stock_code)
        .bind::<diesel::sql_types::Text, _>(pred_date)
        .get_result::<PredictionRow>(&mut *conn)?;
        Ok(row)
    }

    /// 查某日所有未 verify 的 prediction（hit IS NULL）
    ///
    /// 修复 R-1: verify_predictions 真实拉取 stock_daily 后,
    /// 用此函数找到待回填的预测记录 (替代之前硬编码 0.0, false 的假实现)。
    pub fn get_pending_predictions(
        &self,
        pred_date: &str,
    ) -> Result<Vec<PredictionRow>, Box<dyn std::error::Error>> {
        validate_date_text("pred_date", pred_date).map_err(invalid_input)?;
        let mut conn = self.get_conn()?;
        let rows = diesel::sql_query(
            "SELECT id, pred_date, target_date, stock_code, pred_direction, pred_score, actual_change, hit, actual_result FROM prediction_tracker WHERE pred_date = ?1 AND hit IS NULL",
        )
        .bind::<diesel::sql_types::Text, _>(pred_date)
        .load::<PredictionRow>(&mut *conn)?;
        Ok(rows)
    }

    /// 获取近 `days` 天已验证预测的真实命中率。
    pub fn get_prediction_hit_rate(&self, days: i32) -> Result<f64, Box<dyn std::error::Error>> {
        if days <= 0 {
            return Err("命中率窗口 days 必须 > 0".into());
        }
        let mut conn = self.get_conn()?;
        #[derive(QueryableByName, Debug)]
        struct HitRate {
            #[diesel(sql_type = diesel::sql_types::BigInt)]
            sample_count: i64,
            #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
            hit_sum: Option<f64>,
        }

        let row = diesel::sql_query(
            "SELECT COUNT(*) AS sample_count, SUM(CAST(hit AS REAL)) AS hit_sum \
             FROM prediction_tracker \
             WHERE hit IS NOT NULL AND date(pred_date) >= date('now', '-' || ? || ' days')",
        )
        .bind::<diesel::sql_types::Integer, _>(days)
        .get_result::<HitRate>(&mut *conn)?;
        if row.sample_count <= 0 {
            return Err(format!("近 {days} 天没有已验证预测样本").into());
        }
        let hit_sum = row.hit_sum.ok_or("命中数聚合结果缺失")?;
        let rate = hit_sum / row.sample_count as f64;
        if !rate.is_finite() || !(0.0..=1.0).contains(&rate) {
            return Err(format!("命中率超出有效域: {rate}").into());
        }
        Ok(rate)
    }

    /// 保存主题签名用于去同质化（重复签名更新 created_at）
    pub fn upsert_topic_history_signatures(
        &self,
        signatures: &[String],
        max_rows: usize,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if signatures.is_empty() {
            return Ok(());
        }
        if signatures
            .iter()
            .any(|signature| signature.trim().is_empty())
        {
            return Err(invalid_input("topic signature 批次包含空值".to_string()).into());
        }

        let mut conn = self.get_conn()?;
        let now_ts = chrono::Local::now().timestamp();

        // 事务内批量写入，避免逐行 fsync
        conn.transaction::<_, Box<dyn std::error::Error>, _>(|conn| {
            for sig in signatures {
                diesel::sql_query(
                    "INSERT INTO topic_novelty_history(signature, created_at) VALUES (?1, ?2) ON CONFLICT(signature) DO UPDATE SET created_at=excluded.created_at",
                )
                .bind::<diesel::sql_types::Text, _>(sig)
                .bind::<diesel::sql_types::BigInt, _>(now_ts)
                .execute(conn)?;
            }
            Ok(())
        })?;

        let keep = max_rows.max(50) as i64;
        diesel::sql_query(
            "DELETE FROM topic_novelty_history WHERE signature NOT IN (SELECT signature FROM topic_novelty_history ORDER BY created_at DESC LIMIT ?1)",
        )
        .bind::<diesel::sql_types::BigInt, _>(keep)
        .execute(&mut *conn)?;

        Ok(())
    }

    /// 修复 v9.2 BR-001: 统计某只票近 N 天被 push 的次数
    pub fn count_recent_pushes(
        &self,
        stock_code: &str,
        days: i64,
    ) -> Result<i64, Box<dyn std::error::Error>> {
        validate_evidence_code(stock_code).map_err(invalid_input)?;
        if days <= 0 {
            return Err(invalid_input("count_recent_pushes days 必须 > 0".to_string()).into());
        }
        let mut conn = self.get_conn()?;
        let cutoff = (chrono::Local::now() - chrono::Duration::days(days))
            .format("%Y-%m-%d")
            .to_string();
        #[derive(serde::Serialize, serde::Deserialize, diesel::QueryableByName)]
        struct CountRow {
            #[diesel(sql_type = diesel::sql_types::BigInt)]
            cnt: i64,
        }
        let row = diesel::sql_query(
            "SELECT COUNT(*) as cnt FROM prediction_tracker WHERE stock_code = ?1 AND pred_date >= ?2",
        )
        .bind::<diesel::sql_types::Text, _>(stock_code)
        .bind::<diesel::sql_types::Text, _>(&cutoff)
        .get_result::<CountRow>(&mut *conn)?;
        Ok(row.cnt)
    }

    /// 修复 v9.2 M1 性能: 批量查询近 N 天被 push 的 stock_code 集合
    /// 一次 SQL 查所有 stock_code, 避免 discover() 内 N×M 次 sync DB round-trip
    /// 阻塞 async runtime. 返回 HashSet 含所有近 N 天内被 push 过的 stock_code.
    pub fn count_recent_pushes_batch(
        &self,
        stock_codes: &[String],
        days: i64,
    ) -> Result<std::collections::HashSet<String>, Box<dyn std::error::Error>> {
        if days <= 0 {
            return Err(
                invalid_input("count_recent_pushes_batch days 必须 > 0".to_string()).into(),
            );
        }
        if stock_codes.is_empty() {
            return Ok(std::collections::HashSet::new());
        }
        // 修复 I-5 (2026-06-29 codex review) + review #14:
        // 1. 防 SQL 注入 — 显式 if 校验 stock_code 是 ASCII alphanumeric + 下划线.
        //    原 assert! 在 release 默认被优化掉 (除非显式 panic=abort + debug-assertions),
        //    防护失效. 改为返回 Result 错误, 调用方决定如何处理.
        // 2. 用 diesel prepared statement + ? bind 走参数化, 彻底消除字符串拼接风险.
        for c in stock_codes {
            if !c
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
            {
                return Err(format!(
                    "count_recent_pushes_batch: stock_code must be alphanumeric/_/-, got {:?}",
                    c
                )
                .into());
            }
        }
        let mut conn = self.get_conn()?;
        let cutoff = (chrono::Local::now() - chrono::Duration::days(days))
            .format("%Y-%m-%d")
            .to_string();
        // 用 IN (?, ?, ...) + bind 走 prepared statement, 字符串拼接为零.
        // SQLite parameter binding 类型安全, 无 escape 风险.
        use diesel::sql_types::Text;
        let placeholders = std::iter::repeat_n("?", stock_codes.len())
            .collect::<Vec<_>>()
            .join(",");
        let raw = format!(
            "SELECT DISTINCT stock_code FROM prediction_tracker WHERE stock_code IN ({}) AND pred_date >= ?",
            placeholders
        );
        let mut q = diesel::sql_query(raw).into_boxed::<diesel::sqlite::Sqlite>();
        for c in stock_codes {
            q = q.bind::<Text, _>(c.clone());
        }
        q = q.bind::<Text, _>(cutoff);
        #[derive(serde::Serialize, serde::Deserialize, diesel::QueryableByName)]
        struct CodeRow {
            #[diesel(sql_type = diesel::sql_types::Text)]
            stock_code: String,
        }
        let rows: Vec<CodeRow> = q.load::<CodeRow>(&mut *conn)?;
        Ok(rows.into_iter().map(|r| r.stock_code).collect())
    }

    /// 读取近窗期主题签名（按最新时间倒序）
    pub fn get_recent_topic_history_signatures(
        &self,
        lookback_hours: u64,
        limit: usize,
    ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        #[derive(QueryableByName, Debug)]
        struct SigRow {
            #[diesel(sql_type = diesel::sql_types::Text)]
            signature: String,
        }

        let mut conn = self.get_conn()?;
        let since_ts = chrono::Local::now().timestamp() - (lookback_hours as i64 * 3600);
        let lim = limit.max(20) as i64;
        let rows = diesel::sql_query(
            "SELECT signature FROM topic_novelty_history WHERE created_at >= ?1 ORDER BY created_at DESC LIMIT ?2",
        )
        .bind::<diesel::sql_types::BigInt, _>(since_ts)
        .bind::<diesel::sql_types::BigInt, _>(lim)
        .load::<SigRow>(&mut *conn)?;

        Ok(rows.into_iter().map(|r| r.signature).collect())
    }
}

// 辅助数据结构

/// 股票日线记录（用于批量插入）
#[derive(Debug, Clone)]
pub struct StockDailyRecord {
    pub code: String,
    pub date: NaiveDate,
    pub open: Option<f64>,
    pub high: Option<f64>,
    pub low: Option<f64>,
    pub close: Option<f64>,
    pub volume: Option<f64>,
    pub amount: Option<f64>,
    pub pct_chg: Option<f64>,
    pub ma5: Option<f64>,
    pub ma10: Option<f64>,
    pub ma20: Option<f64>,
    pub volume_ratio: Option<f64>,
    pub data_source: Option<String>,
}

/// 分析上下文
#[derive(Debug, Clone)]
pub struct AnalysisContext {
    pub code: String,
    pub date: NaiveDate,
    pub today: HashMap<String, serde_json::Value>,
    pub yesterday: Option<HashMap<String, serde_json::Value>>,
    pub volume_change_ratio: Option<f64>,
    pub price_change_ratio: Option<f64>,
    pub ma_status: MaStatus,
}

// ============================================================================
// 便捷函数
// ============================================================================

/// 获取数据库管理器实例的快捷方式
pub fn get_db() -> &'static DatabaseManager {
    DatabaseManager::get()
}

// ============================================================================
// P0-3: FactorIC 因子分析 — 已平仓交易 + 评分 JOIN
// ============================================================================

/// 预测记录查询返回行 (公开类型, monitor 模块 + 测试使用)
///
/// 修复 R-1: verify_predictions 真实拉取 stock_daily 之后回填,
/// 测试和 verify 函数本身都需要读这条记录。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, diesel::QueryableByName)]
pub struct PredictionRow {
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub id: i32,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub pred_date: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub target_date: String,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    pub stock_code: Option<String>,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub pred_direction: String,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    pub pred_score: Option<f64>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    pub actual_change: Option<f64>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Integer>)]
    pub hit: Option<i32>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    pub actual_result: Option<String>,
}

/// 因子 IC 分析的查询结果行 (公开类型, review 模块使用)
#[derive(Debug, Clone)]
pub struct FactorIcRow {
    pub buy_price: f64,
    pub sell_price: f64,
    pub sentiment_score: Option<i32>,
    pub score_breakdown_json: Option<String>,
}

/// Diesel 返回的内部行
#[derive(QueryableByName, Debug)]
struct FactorIcRowDb {
    #[diesel(sql_type = diesel::sql_types::Double)]
    buy_price: f64,
    #[diesel(sql_type = diesel::sql_types::Double)]
    sell_price: f64,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Integer>)]
    sentiment_score: Option<i32>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    score_breakdown_json: Option<String>,
}

/// PRAGMA table_info 返回的列名行（只取 name 列）
#[derive(QueryableByName, Debug)]
struct ColumnNameRow {
    #[diesel(sql_type = diesel::sql_types::Text)]
    name: String,
}

/// 判断表是否存在 (PRAGMA table_info 返回空 = 不存在)
/// 修复 (2026-07-05 MVP0-A): 用于 add_column_if_missing 跳过未建表, 避免 init 失败
fn table_exists(
    conn: &mut SqliteConnection,
    table: &str,
) -> Result<bool, Box<dyn std::error::Error>> {
    use diesel::RunQueryDsl;
    let pragma_sql = format!("PRAGMA table_info({})", table);
    let cols: Vec<ColumnNameRow> = diesel::sql_query(pragma_sql).load(conn)?;
    Ok(!cols.is_empty())
}

/// 判断表中是否存在指定列（用于增量 schema 升级）
fn column_exists(
    conn: &mut SqliteConnection,
    table: &str,
    column: &str,
) -> Result<bool, Box<dyn std::error::Error>> {
    use diesel::RunQueryDsl;
    let pragma_sql = format!("PRAGMA table_info({})", table);
    let cols: Vec<ColumnNameRow> = diesel::sql_query(&pragma_sql).load(conn)?;
    Ok(cols.iter().any(|c| c.name.eq_ignore_ascii_case(column)))
}

impl DatabaseManager {
    /// 获取已平仓交易的因子分析数据。
    /// 最多 500 条, 用于 `--review` 路径的因子 IC 诊断。
    pub fn get_factor_ic_data(&self) -> Result<Vec<FactorIcRow>, Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;
        let rows = diesel::sql_query(
            "SELECT sp.buy_price, sp.sell_price, ar.sentiment_score, ar.score_breakdown_json
             FROM stock_position sp
             LEFT JOIN analysis_result ar ON sp.code = ar.code AND sp.buy_date = ar.date
             WHERE sp.status = 'closed'
               AND sp.buy_price > 0
               AND sp.sell_price IS NOT NULL
             ORDER BY sp.buy_date DESC
             LIMIT 500",
        )
        .load::<FactorIcRowDb>(&mut conn)?;

        Ok(rows
            .into_iter()
            .map(|r| FactorIcRow {
                buy_price: r.buy_price,
                sell_price: r.sell_price,
                sentiment_score: r.sentiment_score,
                score_breakdown_json: r.score_breakdown_json,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::StockPosition;
    use chrono::NaiveDate;

    #[derive(QueryableByName)]
    struct BusyTimeoutValue {
        #[diesel(sql_type = diesel::sql_types::Integer)]
        timeout: i32,
    }

    #[derive(QueryableByName)]
    struct SynchronousValue {
        #[diesel(sql_type = diesel::sql_types::Integer)]
        synchronous: i32,
    }

    #[derive(QueryableByName)]
    struct WalAutocheckpointValue {
        #[diesel(sql_type = diesel::sql_types::Integer)]
        wal_autocheckpoint: i32,
    }

    // v14.1 review fix: RAII test DB guard, panic 时 Drop 兜底清理
    struct TestDbGuard(&'static str);
    impl Drop for TestDbGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(self.0);
        }
    }

    // OnceCell 单例全局共享，测试共用同一路径避免竞态
    static TEST_DB: &str = "./test_data/test.db";

    fn init_db_for_test() {
        std::fs::create_dir_all("./test_data").ok();
        let _ = DatabaseManager::init(Some(PathBuf::from(TEST_DB)));
    }

    #[test]
    fn test_database_init() {
        init_db_for_test();
        let db = DatabaseManager::get();
        assert!(db.get_conn().is_ok());
    }

    #[test]
    fn br126_database_init_creates_complete_pushed_stocks_contract() {
        #[derive(QueryableByName)]
        struct TableInfoRow {
            #[diesel(sql_type = diesel::sql_types::Text)]
            name: String,
        }
        #[derive(QueryableByName)]
        struct IndexRow {
            #[diesel(sql_type = diesel::sql_types::Text)]
            name: String,
        }

        init_db_for_test();
        let mut conn = DatabaseManager::get()
            .get_conn()
            .expect("test database connection");
        let columns = diesel::sql_query("PRAGMA table_info(pushed_stocks)")
            .load::<TableInfoRow>(&mut conn)
            .expect("read pushed_stocks columns")
            .into_iter()
            .map(|row| row.name)
            .collect::<Vec<_>>();
        assert_eq!(
            columns,
            [
                "id",
                "push_time",
                "push_kind",
                "code",
                "name",
                "push_price",
                "metric_json",
                "source",
                "consumed_at",
                "consumed_by",
                "outcome",
                "created_at",
            ]
        );
        let mut indexes = diesel::sql_query("PRAGMA index_list(pushed_stocks)")
            .load::<IndexRow>(&mut conn)
            .expect("read pushed_stocks indexes")
            .into_iter()
            .map(|row| row.name)
            .collect::<Vec<_>>();
        indexes.sort();
        assert_eq!(
            indexes,
            [
                "idx_pushed_stocks_code",
                "idx_pushed_stocks_time",
                "idx_pushed_stocks_uncon",
            ]
        );
    }

    #[test]
    fn checked_out_connections_have_required_sqlite_pragmas() {
        init_db_for_test();
        let mut conn = DatabaseManager::get()
            .get_conn()
            .expect("configured SQLite connection");

        let busy_timeout = diesel::sql_query("PRAGMA busy_timeout")
            .get_result::<BusyTimeoutValue>(&mut conn)
            .expect("read configured busy_timeout")
            .timeout;
        let synchronous = diesel::sql_query("PRAGMA synchronous")
            .get_result::<SynchronousValue>(&mut conn)
            .expect("read configured synchronous")
            .synchronous;
        let wal_autocheckpoint = diesel::sql_query("PRAGMA wal_autocheckpoint")
            .get_result::<WalAutocheckpointValue>(&mut conn)
            .expect("read configured wal_autocheckpoint")
            .wal_autocheckpoint;

        assert_eq!(busy_timeout, 5000);
        assert_eq!(synchronous, 1);
        assert_eq!(wal_autocheckpoint, 1000);
    }

    fn unique_test_label(prefix: &str) -> String {
        format!(
            "{prefix}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )
    }

    #[test]
    fn br129_news_items_preserve_nullable_identity_and_verified_hash() {
        use crate::data_provider::news_item::{content_hash, NewsItem};
        use chrono::{Duration, Utc};

        #[derive(QueryableByName)]
        struct StoredNews {
            #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
            code: Option<String>,
            #[diesel(sql_type = diesel::sql_types::BigInt)]
            count: i64,
        }

        init_db_for_test();
        let db = DatabaseManager::get();
        let suffix = unique_test_label("NEWS");
        let source = format!("TEST_SOURCE_{suffix}");
        let external_id = format!("TEST_EXTERNAL_{suffix}");
        let code = format!("TEST_CODE_{suffix}");
        let title = "测试新闻标题".to_string();
        let summary = "测试新闻摘要".to_string();
        let fetched_at = Utc::now();
        let item = NewsItem {
            source: source.clone(),
            external_id: external_id.clone(),
            category: "测试分类".to_string(),
            code: Some(code.clone()),
            title: title.clone(),
            summary: summary.clone(),
            url: format!("https://example.invalid/{suffix}"),
            source_name: "测试来源".to_string(),
            published_at: fetched_at - Duration::seconds(1),
            fetched_at,
            content_hash: content_hash(&title, &summary),
        };

        db.insert_news_item(&item).unwrap();
        db.insert_news_item(&item).unwrap();
        let mut conn = db.get_conn().unwrap();
        let stored = diesel::sql_query(
            "SELECT code, COUNT(*) AS count FROM news_items WHERE source = ?1 AND external_id = ?2",
        )
        .bind::<diesel::sql_types::Text, _>(&source)
        .bind::<diesel::sql_types::Text, _>(&external_id)
        .get_result::<StoredNews>(&mut conn)
        .unwrap();
        assert_eq!(stored.code.as_deref(), Some(code.as_str()));
        assert_eq!(
            stored.count, 1,
            "source/external_id duplicate is idempotent"
        );

        let mut without_code = item.clone();
        without_code.external_id = format!("{external_id}_NULL");
        without_code.code = None;
        db.insert_news_item(&without_code).unwrap();
        let stored_null = diesel::sql_query(
            "SELECT code, COUNT(*) AS count FROM news_items WHERE source = ?1 AND external_id = ?2",
        )
        .bind::<diesel::sql_types::Text, _>(&source)
        .bind::<diesel::sql_types::Text, _>(&without_code.external_id)
        .get_result::<StoredNews>(&mut conn)
        .unwrap();
        assert_eq!(stored_null.code, None, "missing code must stay SQL NULL");
        assert_eq!(stored_null.count, 1);

        let mut bad_hash = item.clone();
        bad_hash.external_id = format!("{external_id}_BAD_HASH");
        bad_hash.content_hash = "0".repeat(64);
        assert!(db.insert_news_item(&bad_hash).is_err());
        let mut bad_time = item.clone();
        bad_time.external_id = format!("{external_id}_BAD_TIME");
        bad_time.fetched_at = bad_time.published_at - Duration::seconds(1);
        assert!(db.insert_news_item(&bad_time).is_err());
        let mut bad_identity = item.clone();
        bad_identity.external_id.clear();
        assert!(db.insert_news_item(&bad_identity).is_err());

        diesel::sql_query("DELETE FROM news_items WHERE source = ?1")
            .bind::<diesel::sql_types::Text, _>(&source)
            .execute(&mut conn)
            .unwrap();
    }

    #[test]
    fn br129_prediction_round_trip_is_bound_validated_and_traceable() {
        #[derive(QueryableByName)]
        struct ThemeOutcome {
            #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
            actual_change: Option<f64>,
            #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Integer>)]
            hit: Option<i32>,
        }

        init_db_for_test();
        let db = DatabaseManager::get();
        let suffix = unique_test_label("PREDICTION");
        let code = format!("TEST_CODE_{suffix}");
        let reason = format!("TEST_REASON_O'CLOCK_{suffix}");
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let target = (chrono::Local::now() + chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string();

        db.save_prediction(
            &today,
            &target,
            Some("测试主题"),
            Some(&code),
            "看多",
            75.0,
            Some("完整预测证据"),
            Some(&reason),
            None,
        )
        .unwrap();
        assert_eq!(db.count_predictions_by_reason(&reason).unwrap(), 1);
        assert!(db.count_predictions().unwrap() >= 1);
        assert!(db
            .get_pending_predictions(&today)
            .unwrap()
            .iter()
            .any(|row| row.stock_code.as_deref() == Some(code.as_str())));
        assert_eq!(db.count_recent_pushes(&code, 1).unwrap(), 1);
        assert!(db
            .count_recent_pushes_batch(std::slice::from_ref(&code), 1)
            .unwrap()
            .contains(&code));

        assert_eq!(
            db.update_prediction_result(&today, Some(&code), 1.25, true)
                .unwrap(),
            1
        );
        let stored = db.get_prediction_by_code_date(&code, &today).unwrap();
        assert_eq!(stored.actual_change, Some(1.25));
        assert_eq!(stored.hit, Some(1));
        assert_eq!(stored.actual_result.as_deref(), Some("命中"));
        assert!((0.0..=1.0).contains(&db.get_prediction_hit_rate(1).unwrap()));
        assert_eq!(
            db.update_prediction_result(&today, Some("TEST_CODE_MISSING"), 0.5, false)
                .unwrap(),
            0
        );

        let theme = format!("TEST_THEME_{suffix}");
        db.save_prediction(
            "1999-01-04",
            "1999-01-05",
            Some(&theme),
            None,
            "看空",
            60.0,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            db.update_prediction_result("1999-01-04", None, -0.75, false)
                .unwrap(),
            1
        );
        let mut conn = db.get_conn().unwrap();
        let theme_outcome = diesel::sql_query(
            "SELECT actual_change, hit FROM prediction_tracker WHERE pred_date = '1999-01-04' AND theme_name = ?1",
        )
        .bind::<diesel::sql_types::Text, _>(&theme)
        .get_result::<ThemeOutcome>(&mut conn)
        .unwrap();
        assert_eq!(theme_outcome.actual_change, Some(-0.75));
        assert_eq!(theme_outcome.hit, Some(0));

        assert!(db
            .save_prediction(
                "bad-date",
                &target,
                Some("测试"),
                Some(&code),
                "看多",
                75.0,
                None,
                None,
                None,
            )
            .is_err());
        assert!(db
            .save_prediction(&today, &target, None, None, "看多", 75.0, None, None, None,)
            .is_err());
        assert!(db
            .save_prediction(
                &today,
                &target,
                Some("测试"),
                Some(&code),
                "看多",
                f64::NAN,
                None,
                None,
                None,
            )
            .is_err());
        assert!(db
            .update_prediction_result(&today, Some(&code), 20.01, true)
            .is_err());
        assert!(db.get_pending_predictions("x' OR 1=1 --").is_err());
        assert!(db.count_recent_pushes(&code, 0).is_err());
        assert!(db.count_predictions_by_reason(" ").is_err());
        assert!(db.get_prediction_hit_rate(0).is_err());
        assert!(db
            .save_prediction(
                &today,
                &target,
                Some("测试"),
                Some("TEST_CODE_BAD'"),
                "看多",
                75.0,
                None,
                None,
                Some(" "),
            )
            .is_err());
        assert!(db
            .update_prediction_result(&today, Some("TEST_CODE_BAD'"), 1.0, true)
            .is_err());
        assert!(db
            .get_prediction_by_code_date("TEST_CODE_BAD'", &today)
            .is_err());
        assert!(db.count_recent_pushes("TEST_CODE_BAD'", 1).is_err());
        assert!(db
            .count_recent_pushes_batch(std::slice::from_ref(&code), 0)
            .is_err());
        assert!(db.count_recent_pushes_batch(&[], 1).unwrap().is_empty());
        assert!(db
            .count_recent_pushes_batch(&["TEST_CODE_BAD'".to_string()], 1)
            .is_err());

        diesel::sql_query(
            "DELETE FROM prediction_tracker WHERE stock_code = ?1 OR theme_name = ?2",
        )
        .bind::<diesel::sql_types::Text, _>(&code)
        .bind::<diesel::sql_types::Text, _>(&theme)
        .execute(&mut conn)
        .unwrap();
    }

    #[test]
    fn br129_topic_history_rejects_partial_bad_batches_and_is_idempotent() {
        #[derive(QueryableByName)]
        struct Count {
            #[diesel(sql_type = diesel::sql_types::BigInt)]
            count: i64,
        }

        init_db_for_test();
        let db = DatabaseManager::get();
        let suffix = unique_test_label("TOPIC");
        let first = format!("TEST_TOPIC_FIRST_{suffix}");
        let second = format!("TEST_TOPIC_SECOND_{suffix}");
        db.upsert_topic_history_signatures(&[], 0)
            .expect("empty complete batch is a no-op");
        db.upsert_topic_history_signatures(&[first.clone(), second.clone()], 50)
            .unwrap();
        db.upsert_topic_history_signatures(std::slice::from_ref(&first), 50)
            .unwrap();
        let recent = db.get_recent_topic_history_signatures(1, 20).unwrap();
        assert!(recent.contains(&first));
        assert!(recent.contains(&second));

        let rejected = format!("TEST_TOPIC_REJECTED_{suffix}");
        assert!(db
            .upsert_topic_history_signatures(&[rejected.clone(), " ".to_string()], 50)
            .is_err());
        let mut conn = db.get_conn().unwrap();
        let rejected_count = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM topic_novelty_history WHERE signature = ?1",
        )
        .bind::<diesel::sql_types::Text, _>(&rejected)
        .get_result::<Count>(&mut conn)
        .unwrap();
        assert_eq!(
            rejected_count.count, 0,
            "bad batch must not partially persist"
        );

        let persisted = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM topic_novelty_history WHERE signature IN (?1, ?2)",
        )
        .bind::<diesel::sql_types::Text, _>(&first)
        .bind::<diesel::sql_types::Text, _>(&second)
        .get_result::<Count>(&mut conn)
        .unwrap();
        assert_eq!(persisted.count, 2, "duplicate signature remains idempotent");

        diesel::sql_query("DELETE FROM topic_novelty_history WHERE signature IN (?1, ?2, ?3)")
            .bind::<diesel::sql_types::Text, _>(&first)
            .bind::<diesel::sql_types::Text, _>(&second)
            .bind::<diesel::sql_types::Text, _>(&rejected)
            .execute(&mut conn)
            .unwrap();
    }

    #[test]
    #[serial_test::serial]
    fn remaining_root_accessors_and_factor_ic_query_use_real_sql_rows() {
        init_db_for_test();
        let db = DatabaseManager::get();
        assert!(std::ptr::eq(get_db(), db));
        assert_eq!(
            DatabaseManager::with_db("TEST_CODE_ROOT", |_| Some(7)),
            Some(7)
        );

        let suffix = unique_test_label("FACTOR_IC");
        let code = format!("TEST_CODE_{suffix}");
        let buy_price = 1234.567;
        let sell_price = 1300.0;
        let buy_date = "2198-01-02";
        let mut conn = db.get_conn().expect("test database connection");
        diesel::sql_query(
            "INSERT INTO stock_position
             (code, name, buy_date, buy_price, quantity, status, sell_date, sell_price, return_rate)
             VALUES (?, 'TEST_CODE_因子样本', ?, ?, 100, 'closed', '2198-01-03', ?, 5.0)",
        )
        .bind::<diesel::sql_types::Text, _>(&code)
        .bind::<diesel::sql_types::Text, _>(buy_date)
        .bind::<diesel::sql_types::Double, _>(buy_price)
        .bind::<diesel::sql_types::Double, _>(sell_price)
        .execute(&mut conn)
        .expect("insert complete closed position");
        drop(conn);

        let rows = db.get_factor_ic_data().expect("factor IC repository query");
        let row = rows
            .iter()
            .find(|row| (row.buy_price - buy_price).abs() < f64::EPSILON)
            .expect("inserted factor IC row");
        assert_eq!(row.sell_price, sell_price);
        assert_eq!(row.sentiment_score, None);
        assert_eq!(row.score_breakdown_json, None);

        let mut conn = db.get_conn().expect("cleanup database connection");
        diesel::sql_query("DELETE FROM stock_position WHERE code = ?")
            .bind::<diesel::sql_types::Text, _>(&code)
            .execute(&mut conn)
            .expect("cleanup factor IC row");
    }

    #[test]
    fn test_order_tables_reject_invalid_direct_writes() {
        init_db_for_test();
        let mut conn = DatabaseManager::get()
            .get_conn()
            .expect("test DB connection");

        let invalid_position = diesel::sql_query(
            "INSERT INTO stock_position
             (code, name, buy_date, buy_price, quantity, status)
             VALUES ('TEST_CODE_INVALID_LOT', '测试', '2026-07-17', 10.0, 99, 'open')",
        )
        .execute(&mut conn);
        assert!(invalid_position.is_err());

        let invalid_paper = diesel::sql_query(
            "INSERT INTO paper_trades
             (plan_id, code, name, direction, price, quantity, status,
              virtual_reason, account_mode, data_mode)
             VALUES ('TEST_PLAN_INVALID_PRICE', 'TEST_CODE_000001', '测试', 'buy',
                     0.0, 100, 'Invalidated', 'NewsCatalyst', 'Normal', 'Full')",
        )
        .execute(&mut conn);
        assert!(invalid_paper.is_err());

        let rejected_without_reason = diesel::sql_query(
            "INSERT INTO order_audit
             (business_order_id, source, decision_basis, side, code,
              requested_price, execution_price, quantity, quote_observed_at,
              outcome, failure_reason)
             VALUES ('TEST_ORDER_INVALID_REJECT', 'DatabaseTest', 'test', 'buy',
                     'TEST_CODE_INVALID', 0.0, NULL, 0, NULL, 'Rejected', NULL)",
        )
        .execute(&mut conn);
        assert!(rejected_without_reason.is_err());

        let filled_without_quote = diesel::sql_query(
            "INSERT INTO order_audit
             (business_order_id, source, decision_basis, side, code,
              requested_price, execution_price, quantity, quote_observed_at,
              outcome, failure_reason)
             VALUES ('TEST_ORDER_INVALID_FILL', 'DatabaseTest', 'test', 'buy',
                     'TEST_CODE_INVALID', 10.0, 10.0, 100, NULL, 'Filled', NULL)",
        )
        .execute(&mut conn);
        assert!(filled_without_quote.is_err());
    }

    #[test]
    fn br094_agent_decision_audit_is_append_only() {
        init_db_for_test();
        let mut conn = DatabaseManager::get()
            .get_conn()
            .expect("test DB connection");
        let session = format!("TEST_CODE_AGENT_AUDIT_{}", std::process::id());
        diesel::sql_query(
            "INSERT INTO agent_scratchpad (session_id, step, log_type, content) \
             VALUES (?, 1, 'decision', 'TEST_CODE immutable evidence')",
        )
        .bind::<diesel::sql_types::Text, _>(&session)
        .execute(&mut conn)
        .expect("append agent audit row");

        let update = diesel::sql_query(
            "UPDATE agent_scratchpad SET content = 'tampered' WHERE session_id = ?",
        )
        .bind::<diesel::sql_types::Text, _>(&session)
        .execute(&mut conn);
        let delete = diesel::sql_query("DELETE FROM agent_scratchpad WHERE session_id = ?")
            .bind::<diesel::sql_types::Text, _>(&session)
            .execute(&mut conn);

        assert!(update.is_err(), "agent decision audit must reject UPDATE");
        assert!(delete.is_err(), "agent decision audit must reject DELETE");
    }

    #[test]
    fn br084_business_order_reservation_is_persistent_and_atomic() {
        init_db_for_test();
        let id = format!(
            "TEST_ORDER_RESERVATION_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let db = DatabaseManager::get();
        assert!(db
            .reserve_business_order_id(&id)
            .expect("first reservation"));
        assert!(
            !db.reserve_business_order_id(&id)
                .expect("duplicate reservation query"),
            "the same ID must be rejected by shared persistence within 60 seconds"
        );
    }

    #[test]
    fn test_order_audit_is_immutable_and_atomic_with_position_fill() {
        use crate::database::order_audit::OrderAuditRecord;
        use crate::models::NewStockPosition;

        init_db_for_test();
        let db = DatabaseManager::get();
        let position = NewStockPosition {
            code: "TEST_CODE_AUDIT_ATOMIC".to_string(),
            name: "审计测试".to_string(),
            buy_date: "2026-07-17".to_string(),
            buy_price: 10.0,
            quantity: 100,
            status: "open".to_string(),
            st_type: None,
            chain_name: Some("测试产业链".to_string()),
        };
        let audit = OrderAuditRecord {
            business_order_id: "TEST_ORDER_AUDIT_ATOMIC",
            source: "DatabaseTest",
            decision_basis: "test",
            side: "buy",
            code: "TEST_CODE_AUDIT_ATOMIC",
            requested_price: 10.0,
            execution_price: Some(10.0),
            quantity: 100,
            quote_observed_at: Some("2026-07-17T09:30:00+08:00"),
            outcome: "Filled",
            failure_reason: None,
        };
        db.save_position_with_audit(&position, &audit)
            .expect("atomic audited position fill");

        let mut conn = db.get_conn().expect("test DB connection");
        #[derive(diesel::QueryableByName)]
        struct Count {
            #[diesel(sql_type = diesel::sql_types::BigInt)]
            count: i64,
        }
        let count: Count = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM order_audit
             WHERE business_order_id = 'TEST_ORDER_AUDIT_ATOMIC' AND outcome = 'Filled'",
        )
        .get_result(&mut conn)
        .expect("query audit");
        assert_eq!(count.count, 1);
        let chain_count: Count = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM order_audit_chain
             WHERE order_audit_id IN (
                 SELECT id FROM order_audit
                 WHERE business_order_id = 'TEST_ORDER_AUDIT_ATOMIC'
             )",
        )
        .get_result(&mut conn)
        .expect("query audit chain evidence");
        assert_eq!(chain_count.count, 1);

        assert!(diesel::sql_query(
            "UPDATE order_audit SET outcome = 'Rejected'
             WHERE business_order_id = 'TEST_ORDER_AUDIT_ATOMIC'",
        )
        .execute(&mut conn)
        .is_err());
        assert!(diesel::sql_query(
            "UPDATE order_audit_chain SET record_hash = 'tampered'
             WHERE order_audit_id IN (
                 SELECT id FROM order_audit
                 WHERE business_order_id = 'TEST_ORDER_AUDIT_ATOMIC'
             )",
        )
        .execute(&mut conn)
        .is_err());
        assert!(diesel::sql_query(
            "DELETE FROM order_audit WHERE business_order_id = 'TEST_ORDER_AUDIT_ATOMIC'",
        )
        .execute(&mut conn)
        .is_err());

        diesel::sql_query("DELETE FROM stock_position WHERE code = 'TEST_CODE_AUDIT_ATOMIC'")
            .execute(&mut conn)
            .expect("cleanup audited position");
    }

    #[test]
    fn test_save_and_retrieve() {
        init_db_for_test();
        let db = DatabaseManager::get();

        // 保存数据
        let date = NaiveDate::from_ymd_opt(2026, 1, 22).unwrap();
        db.save_daily_record(
            "TEST_CODE_600519",
            date,
            Some(1800.0),
            Some(1850.0),
            Some(1780.0),
            Some(1820.0),
            Some(10000000.0),
            Some(18200000000.0),
            Some(1.5),
            Some(1810.0),
            Some(1800.0),
            Some(1790.0),
            Some(1.2),
            Some("TestSource"),
        )
        .expect("保存数据失败");

        // 检查数据是否存在
        let has_data = db
            .has_data_for_date("TEST_CODE_600519", date)
            .expect("查询失败");
        assert!(has_data);

        // 获取数据
        let data = db
            .get_latest_data("TEST_CODE_600519", 1)
            .expect("获取数据失败");
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].code, "TEST_CODE_600519");
        assert_eq!(data[0].close, Some(1820.0));

        // 清理数据（不删DB文件，并行测试可能还在用）
        db.delete_stock_data("TEST_CODE_600519").ok();
    }

    // v14.1 task #167: stock_position.st_type round-trip DB 集成测试
    // 路径: save_position(NewStockPosition{ st_type: Some("*ST") }) →
    //       get_all_open_positions → StockPosition.st_type 真读出
    // 用独立 DB 文件避免与上面 test_save_and_query_stock_data 竞态.
    #[test]
    fn test_st_type_db_round_trip() {
        use crate::models::{NewStockPosition, StockPosition};
        use crate::schema::stock_position;
        use diesel::prelude::*;

        init_db_for_test();

        let db = DatabaseManager::get();

        // 1. insert 一只 *ST 持仓
        let new_pos = NewStockPosition {
            code: "TEST_CODE_600090".to_string(),
            name: "*ST测试".to_string(),
            buy_date: "2026-07-01".to_string(),
            buy_price: 5.0,
            quantity: 1000,
            status: "open".to_string(),
            st_type: Some("*ST".to_string()),
            chain_name: None,
        };
        db.save_position(&new_pos).expect("save_position 失败");

        // 2. 读回 — 验证 st_type 真写入
        let mut conn = db.get_conn().expect("get_conn 失败");
        let row: StockPosition = stock_position::table
            .filter(stock_position::code.eq("TEST_CODE_600090"))
            .first(&mut conn)
            .expect("query 失败");
        assert_eq!(
            row.st_type.as_deref(),
            Some("*ST"),
            "st_type 写入/读出不一致"
        );
        assert_eq!(row.code, "TEST_CODE_600090");
        assert_eq!(row.name, "*ST测试");
        assert_eq!(row.quantity, 1000);

        // 3. 测试 upsert: 同 (code, buy_date) 再 save 不报错, st_type 应被 excluded 同步
        let update_pos = NewStockPosition {
            code: "TEST_CODE_600090".to_string(),
            name: "*ST测试改名".to_string(),
            buy_date: "2026-07-01".to_string(),
            buy_price: 5.5,
            quantity: 1500,
            status: "open".to_string(),
            st_type: Some("ST".to_string()), // 改 ST
            chain_name: Some("化工".to_string()),
        };
        db.save_position(&update_pos).expect("upsert 失败");

        let row2: StockPosition = stock_position::table
            .filter(stock_position::code.eq("TEST_CODE_600090"))
            .first(&mut conn)
            .expect("re-query 失败");
        assert_eq!(row2.st_type.as_deref(), Some("ST"), "upsert st_type 未同步");
        assert_eq!(
            row2.chain_name.as_deref(),
            Some("化工"),
            "upsert chain_name 未同步"
        );
        assert_eq!(row2.name, "*ST测试改名", "upsert name 未同步");

        diesel::delete(stock_position::table.filter(stock_position::code.eq("TEST_CODE_600090")))
            .execute(&mut conn)
            .expect("cleanup test position");
    }

    // v14.1 review fix: 测试 backfill_st_type 前缀锚定 (LIKE 'ST%' / 'ST*%' 而非 '%ST%')
    // 之前 '%ST%' 子串匹配会把 'BEST' / 'GST' 误判成 ST 类
    #[test]
    fn test_backfill_st_type_prefix_anchored() {
        use crate::models::NewStockPosition;
        use crate::schema::stock_position;
        use diesel::prelude::*;

        let test_db = "./test_data/test_backfill_st_type.db";
        std::fs::create_dir_all("./test_data").ok();
        let _ = std::fs::remove_file(test_db);
        let _ = DatabaseManager::init(Some(PathBuf::from(test_db)));
        // review fix: RAII guard, panic 时 Drop 清理 test_db
        let _guard = TestDbGuard(test_db);

        let db = DatabaseManager::get();

        // Insert 4 测试持仓: 真正 ST 开头 + 子串含 ST (非 ST 类) + 普通 + *ST
        let cases = vec![
            ("TEST_CODE_001", "ST康美", Some("ST")),
            ("TEST_CODE_002", "*ST华微", Some("*ST")),
            ("TEST_CODE_003", "BEST新材", None), // 子串含 ST 但不是 ST 类
            ("TEST_CODE_004", "GST电子", None),  // 子串含 ST 但不是 ST 类
            ("TEST_CODE_005", "浦发银行", None), // 普通
            ("TEST_CODE_006", "SST集成", Some("ST")),
            ("TEST_CODE_007", "S*ST海伦", Some("*ST")),
        ];
        for (code, name, _expected) in &cases {
            db.save_position(&NewStockPosition {
                code: code.to_string(),
                name: name.to_string(),
                buy_date: "2026-07-01".to_string(),
                buy_price: 10.0,
                quantity: 100,
                status: "open".to_string(),
                st_type: None,
                chain_name: None,
            })
            .expect("save 失败");
        }

        // 跑 backfill
        let updated = db.backfill_st_type().expect("backfill 失败");
        assert!(updated > 0, "至少应更新 4 条真 ST 类");

        // 验证每个 case
        let mut conn = db.get_conn().unwrap();
        for (code, name, expected) in &cases {
            let row: StockPosition = stock_position::table
                .filter(stock_position::code.eq(code as &str))
                .first(&mut conn)
                .expect("query 失败");
            assert_eq!(
                row.st_type.as_deref(),
                *expected,
                "code={code} name={name} expected={expected:?} got={:?}",
                row.st_type
            );
        }

        for (code, _, _) in &cases {
            diesel::delete(stock_position::table.filter(stock_position::code.eq(*code)))
                .execute(&mut conn)
                .expect("cleanup backfill test position");
        }
    }

    // v14.1 review fix: 测试 save_position upsert 不覆盖 st_type (COALESCE 行为)
    // 之前 excluded(st_type) 会把 backfill 写好的 *ST 清成 NULL
    #[test]
    fn test_save_position_upsert_preserves_st_type() {
        use crate::models::NewStockPosition;
        use crate::schema::stock_position;
        use diesel::prelude::*;

        let test_db = "./test_data/test_upsert_preserve_st.db";
        std::fs::create_dir_all("./test_data").ok();
        let _ = std::fs::remove_file(test_db);
        let _ = DatabaseManager::init(Some(PathBuf::from(test_db)));
        // review fix: RAII guard
        let _guard = TestDbGuard(test_db);

        let db = DatabaseManager::get();

        // 1. 首次 insert, st_type=None
        db.save_position(&NewStockPosition {
            code: "TEST_CODE_600519".to_string(),
            name: "贵州茅台".to_string(),
            buy_date: "2026-07-01".to_string(),
            buy_price: 1800.0,
            quantity: 100,
            status: "open".to_string(),
            st_type: None,
            chain_name: None,
        })
        .expect("save 1 失败");

        // 2. 模拟 broker 推送 *ST (用 raw SQL 写, 模拟 broker update path)
        let mut conn = db.get_conn().unwrap();
        diesel::sql_query(
            "UPDATE stock_position SET st_type = '*ST' WHERE code = 'TEST_CODE_600519'",
        )
        .execute(&mut conn)
        .expect("st_type set 失败");

        // 3. trading::open_position re-buy 同 (code, buy_date) — 传 None
        db.save_position(&NewStockPosition {
            code: "TEST_CODE_600519".to_string(),
            name: "贵州茅台".to_string(),
            buy_date: "2026-07-01".to_string(),
            buy_price: 1850.0, // 价格变 (新买入)
            quantity: 200,     // 数量变
            status: "open".to_string(),
            st_type: None,    // 重买不带 st_type
            chain_name: None, // 重买不带 chain
        })
        .expect("save 2 失败");

        // 4. 验证: st_type 应保持 '*ST' (COALESCE 保 NULL 时不覆盖), 价格/数量更新
        let row: StockPosition = stock_position::table
            .filter(stock_position::code.eq("TEST_CODE_600519"))
            .first(&mut conn)
            .expect("re-query 失败");
        assert_eq!(
            row.st_type.as_deref(),
            Some("*ST"),
            "st_type 应保持 broker 推送的 *ST, 不应被 re-buy NULL 覆盖"
        );
        assert_eq!(row.buy_price, 1850.0, "价格应更新");
        assert_eq!(row.quantity, 200, "数量应更新");

        diesel::delete(stock_position::table.filter(stock_position::code.eq("TEST_CODE_600519")))
            .execute(&mut conn)
            .expect("cleanup upsert test position");
    }
}
