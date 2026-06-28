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


pub mod repository;
pub mod factor_snapshot;
mod concepts;
mod kline;
mod lhb;
mod positions;
pub(crate) mod agent_logs;

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
        let path = db_path.unwrap_or_else(|| {
            let mut p = PathBuf::from("./data");
            std::fs::create_dir_all(&p).ok();
            p.push("stock.db");
            p
        });

        let database_url = path.to_string_lossy().to_string();
        info!("初始化数据库: {}", database_url);

        let manager = ConnectionManager::<SqliteConnection>::new(database_url);
        let pool = Pool::builder()
            .max_size(10)
            .build(manager)?;

        // 运行迁移
        let mut conn = pool.get()?;
        Self::run_migrations(&mut conn)?;

        // 修复 并行测试隔离: SQLite WAL 模式 + busy_timeout
        // 之前: SQLite 默认 DELETE journal mode → 写锁整库, 并行测试同时写同一个 ./test_data/test.db → "database is locked"
        // 现在: WAL 模式让读写不互斥, busy_timeout 让等待锁的连接最多等 5s
        // 收益: (a) cargo test 默认并行度不再 flake, (b) 生产路径并发写也安全
        diesel::sql_query("PRAGMA journal_mode = WAL").execute(&mut *conn)?;
        diesel::sql_query("PRAGMA synchronous = NORMAL").execute(&mut *conn)?;
        diesel::sql_query("PRAGMA busy_timeout = 5000").execute(&mut *conn)?;
        diesel::sql_query("PRAGMA wal_autocheckpoint = 1000").execute(&mut *conn)?;
        info!("SQLite PRAGMAs 已设置: WAL + busy_timeout=5000");

        let db = DatabaseManager { pool };

        DB_INSTANCE
            .set(db)
            .map_err(|_| "数据库已经初始化")?;

        info!("数据库初始化完成");
        
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
            "#
        )
        .execute(&mut *conn)?;

        // 创建 stock_concepts 表（概念板块标签缓存，产业链聚类用）
        diesel::sql_query(
            r#"
            CREATE TABLE IF NOT EXISTS stock_concepts (
                code TEXT PRIMARY KEY,
                concepts TEXT NOT NULL,
                updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#
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
            "#
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

        Ok(())
    }

    /// 获取数据库管理器单例
    pub fn get() -> &'static DatabaseManager {
        DB_INSTANCE
            .get()
            .expect("数据库未初始化，请先调用 DatabaseManager::init()")
    }

    /// 获取数据库连接
    pub fn get_conn(&self) -> Result<DbConnection, Box<dyn std::error::Error>> {
        Ok(self.pool.get()?)
    }

    /// 给已存在的表增量添加列（如果列不存在）。
    /// SQLite 没有原生的 `ADD COLUMN IF NOT EXISTS`；通过 PRAGMA table_info 读列名判断。
    /// 用于把老库升级到新 schema，不破坏现有数据。
    pub fn add_column_if_missing(
        conn: &mut SqliteConnection,
        table: &str,
        column: &str,
        column_def: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if column_exists(conn, table, column)? {
            return Ok(());
        }
        let alter = format!("ALTER TABLE {} ADD COLUMN {} {}", table, column, column_def);
        diesel::sql_query(&alter).execute(conn)?;
        Ok(())
    }

    /// 运行数据库迁移
    fn run_migrations(conn: &mut SqliteConnection) -> Result<(), Box<dyn std::error::Error>> {
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
        Self::add_column_if_missing(conn, "stock_daily", "is_limit_up", "TINYINT NOT NULL DEFAULT 0")?;
        Self::add_column_if_missing(conn, "stock_daily", "is_limit_down", "TINYINT NOT NULL DEFAULT 0")?;
        Self::add_column_if_missing(conn, "stock_daily", "is_suspended", "TINYINT NOT NULL DEFAULT 0")?;

        // 老库升级：增量添加 6 列 (修复 P1.3 trades 业绩归因)
        // 量化分析师要求: 必须能算真实 PnL (扣除 commission/stamp_tax/slippage)
        Self::add_column_if_missing(conn, "trades", "commission_amount", "REAL DEFAULT 0")?;
        Self::add_column_if_missing(conn, "trades", "stamp_tax_amount", "REAL DEFAULT 0")?;
        Self::add_column_if_missing(conn, "trades", "slippage_amount", "REAL DEFAULT 0")?;
        Self::add_column_if_missing(conn, "trades", "realized_pnl", "REAL DEFAULT 0")?;
        Self::add_column_if_missing(conn, "trades", "strategy_tag", "TEXT DEFAULT ''")?;
        Self::add_column_if_missing(conn, "trades", "signal_id", "TEXT DEFAULT ''")?;

        // 创建索引
        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS ix_stock_daily_code ON stock_daily(code)",
        )
        .execute(&mut *conn)?;

        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS ix_stock_daily_date ON stock_daily(date)",
        )
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
        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS ix_lhb_daily_code ON lhb_daily(code)",
        )
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
                buy_price REAL NOT NULL,
                quantity INTEGER NOT NULL,
                status TEXT NOT NULL DEFAULT 'open',
                sell_date TEXT,
                sell_price REAL,
                return_rate REAL,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
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
        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS ix_trades_code ON trades(code)",
        )
        .execute(&mut *conn)?;

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
        ).execute(&mut *conn)?;

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

        Ok(())
    }

    /// 保存预测记录（Phase 5 预测闭环）
    pub fn save_prediction(
        &self,
        pred_date: &str,
        target_date: &str,
        theme_name: Option<&str>,
        stock_code: Option<&str>,
        direction: &str,
        score: f64,
        detail: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;
        let tn = theme_name.unwrap_or("");
        let sc = stock_code.unwrap_or("");
        let det = detail.unwrap_or("");
        diesel::sql_query(format!(
            "INSERT INTO prediction_tracker (pred_date, target_date, theme_name, stock_code, pred_direction, pred_score, pred_detail) VALUES ('{}', '{}', '{}', '{}', '{}', {}, '{}')",
            pred_date, target_date, tn, sc, direction, score, det
        ))
        .execute(&mut *conn)?;
        Ok(())
    }

    /// 更新预测结果（次日收盘后回调）
    pub fn update_prediction_result(
        &self,
        pred_date: &str,
        stock_code: Option<&str>,
        actual_change: f64,
        hit: bool,
    ) -> Result<usize, Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;
        let result_text = if hit { "命中" } else { "未命中" };
        let rows = if let Some(code) = stock_code {
            diesel::sql_query(format!(
                "UPDATE prediction_tracker SET actual_change = {}, hit = {}, actual_result = '{}' WHERE pred_date = '{}' AND stock_code = '{}'",
                actual_change, hit as i32, result_text, pred_date, code
            ))
            .execute(&mut *conn)?
        } else {
            diesel::sql_query(format!(
                "UPDATE prediction_tracker SET actual_change = {}, hit = {}, actual_result = '{}' WHERE pred_date = '{}' AND theme_name != ''",
                actual_change, hit as i32, result_text, pred_date
            ))
            .execute(&mut *conn)?
        };
        Ok(rows)
    }

    /// 获取预测命中率（简化实现，直接执行 SQL 返回 f64）
    pub fn get_prediction_hit_rate(&self, _days: i32) -> Result<f64, Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;
        // 使用 Diesel 的 sql_query + get_result 返回单值
        #[derive(QueryableByName, Debug)]
        struct HitRate {
            #[diesel(sql_type = diesel::sql_types::Text)]
            rate_text: String,
        }
        let raw = "SELECT CAST(COALESCE(SUM(CAST(hit AS REAL)), 0) / CASE WHEN COUNT(*) = 0 THEN 1 ELSE COUNT(*) END AS TEXT) as rate_text FROM prediction_tracker";
        let result = diesel::sql_query(raw).get_result::<HitRate>(&mut *conn);
        match result {
            Ok(r) => Ok(r.rate_text.parse::<f64>().unwrap_or(0.0)),
            Err(_) => Ok(0.0),
        }
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

        let mut conn = self.get_conn()?;
        let now_ts = chrono::Local::now().timestamp();

        // 事务内批量写入，避免逐行 fsync
        conn.transaction::<_, Box<dyn std::error::Error>, _>(|conn| {
            for sig in signatures {
                if sig.is_empty() {
                    continue;
                }
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
        let mut conn = self.pool.get()?;
        let rows = diesel::sql_query(
            "SELECT sp.buy_price, sp.sell_price, ar.sentiment_score, ar.score_breakdown_json
             FROM stock_position sp
             LEFT JOIN analysis_result ar ON sp.code = ar.code AND sp.buy_date = ar.date
             WHERE sp.status = 'closed'
               AND sp.buy_price > 0
               AND sp.sell_price IS NOT NULL
             ORDER BY sp.buy_date DESC
             LIMIT 500"
        )
        .load::<FactorIcRowDb>(&mut conn)?;

        Ok(rows.into_iter().map(|r| FactorIcRow {
            buy_price: r.buy_price,
            sell_price: r.sell_price,
            sentiment_score: r.sentiment_score,
            score_breakdown_json: r.score_breakdown_json,
        }).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

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
        assert!(db.pool.get().is_ok());
    }

    #[test]
    fn test_save_and_retrieve() {
        init_db_for_test();
        let db = DatabaseManager::get();

        // 保存数据
        let date = NaiveDate::from_ymd_opt(2026, 1, 22).unwrap();
        db.save_daily_record(
            "600519",
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
        let has_data = db.has_data_for_date("600519", date).expect("查询失败");
        assert!(has_data);

        // 获取数据
        let data = db.get_latest_data("600519", 1).expect("获取数据失败");
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].code, "600519");
        assert_eq!(data[0].close, Some(1820.0));

        // 清理数据（不删DB文件，并行测试可能还在用）
        db.delete_stock_data("600519").ok();
    }
}
