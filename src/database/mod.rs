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

pub mod factor_snapshot;
pub mod repository;
// v12 MVP-5 §8.1
pub(crate) mod agent_logs;
pub mod concepts;  // v15.1: 公开供 push_templates 集成使用
pub mod execution_tracking;
mod kline;
mod lhb;
mod positions;
// v12 PR1-1.5 (BR-021)
pub mod account_mode_log;
// v12 PR3-3.2/3.3 (BR-023/024)
pub mod position_shares;

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
        let pool = Pool::builder().max_size(10).build(manager)?;

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

        DB_INSTANCE.set(db).map_err(|_| "数据库已经初始化")?;

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
            "#,
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

    /// 尝试获取数据库管理器单例（返回 Option，不 panic）.
    /// review #14: 取代之前各处 `catch_unwind(DatabaseManager::get)` 的反 pattern.
    /// catch_unwind 强制 panic = unwind + 还要 AssertUnwindSafe wrap, 而且静默吞
    /// init 失败, 让 operator 看到「数据全空」但不知道 DB 没起来.
    /// 显式 Option 让调用方必须处理 None 路径 (早返回 / log warn).
    pub fn try_get() -> Option<&'static DatabaseManager> {
        DB_INSTANCE.get()
    }

    /// 获取数据库连接
    pub fn get_conn(&self) -> Result<DbConnection, Box<dyn std::error::Error>> {
        Ok(self.pool.get()?)
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
                buy_price REAL NOT NULL,
                quantity INTEGER NOT NULL,
                status TEXT NOT NULL DEFAULT 'open',
                sell_date TEXT,
                sell_price REAL,
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

        // v12 PR3-3.6 (BR-015 偿还): stock_position 加 chain_name 列
        // 旧库可能没有, 用 add_column_if_missing 包一层 (SQLite 1.06 无 ADD COLUMN IF NOT EXISTS)
        Self::add_column_if_missing(conn, "stock_position", "chain_name", "TEXT DEFAULT '其他'")?;
        diesel::sql_query("CREATE INDEX IF NOT EXISTS idx_stock_position_chain_name ON stock_position(chain_name)")
            .execute(&mut *conn).ok();

        // v14.1 F7: stock_position 加 st_type 列 (TEXT: 'ST' / '*ST' / NULL)
        // T-16 ST 涨跌幅变更 dispatcher 数据源. 由 --backfill-st-type 从 name 字段回填,
        // 后续 broker/exchange 推送时更新. 无 CHECK 约束 (SQLite ALTER ADD COLUMN 不支持)
        Self::add_column_if_missing(conn, "stock_position", "st_type", "TEXT")?;
        diesel::sql_query("CREATE INDEX IF NOT EXISTS idx_stock_position_st_type ON stock_position(st_type)")
            .execute(&mut *conn).ok();

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
                price           REAL NOT NULL,
                quantity        INTEGER NOT NULL,
                status          TEXT NOT NULL CHECK(status IN ('SignalTriggered','Filled','NotFilled','Invalidated')),
                fill_price      REAL,
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
        .execute(&mut *conn)
        .ok();
        diesel::sql_query("CREATE INDEX IF NOT EXISTS idx_paper_trades_code ON paper_trades(code)")
            .execute(&mut *conn)
            .ok();
        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS idx_paper_trades_status ON paper_trades(status)",
        )
        .execute(&mut *conn)
        .ok();

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

        Ok(())
    }

    /// 保存预测记录（Phase 5 预测闭环）
    ///
    /// v10 P0.2 (BR-016): 加 `reason` + `reason_secondary` 参数, 写盘口时记主/副理由
    /// 向后兼容: reason/reason_secondary 默认为 None (走 v9 旧路径)
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
        let mut conn = self.get_conn()?;
        // codex P0#3: escape 单引号 (`'` → `''`) 防 SQL injection
        // 项目惯例: raw SQL 嵌入字符串, 标准 SQL escape 用双单引号
        let esc = |s: &str| s.replace('\'', "''");
        let tn = esc(theme_name.unwrap_or(""));
        let sc = esc(stock_code.unwrap_or(""));
        let det = esc(detail.unwrap_or(""));
        let rsn = esc(reason.unwrap_or(""));
        let rsn2 = esc(reason_secondary.unwrap_or(""));
        let pd = esc(pred_date);
        let td = esc(target_date);
        let dir = esc(direction);
        diesel::sql_query(format!(
            "INSERT INTO prediction_tracker (pred_date, target_date, theme_name, stock_code, pred_direction, pred_score, pred_detail, reason, reason_secondary) VALUES ('{}', '{}', '{}', '{}', '{}', {}, '{}', '{}', '{}')",
            pd, td, tn, sc, dir, score, det, rsn, rsn2
        ))
        .execute(&mut *conn)?;
        Ok(())
    }

    /// v10 P0.2 便捷重载: 不带 reason (旧调用路径, 走 v9 旧行为)
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
        let mut conn = self.get_conn()?;
        #[derive(diesel::QueryableByName)]
        struct PredReasonCountRow {
            #[diesel(sql_type = diesel::sql_types::BigInt)]
            cnt: i64,
        }
        // codex P0#3: escape 单引号防 SQL injection
        let rsn = reason.replace('\'', "''");
        let result = diesel::sql_query(format!(
            "SELECT COUNT(*) AS cnt FROM prediction_tracker WHERE reason = '{}'",
            rsn
        ))
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

    /// 按 stock_code + pred_date 查询 prediction 记录
    ///
    /// 修复 R-1: 用于 verify_predictions 真实回填后, 测试断言 hit/actual_change。
    /// 返回最新的一条 (LIMIT 1) — 同一 (code, pred_date) 只期望一条。
    pub fn get_prediction_by_code_date(
        &self,
        stock_code: &str,
        pred_date: &str,
    ) -> Result<PredictionRow, Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;
        let row = diesel::sql_query(format!(
            "SELECT id, pred_date, target_date, stock_code, pred_direction, pred_score, actual_change, hit, actual_result FROM prediction_tracker WHERE stock_code = '{}' AND pred_date = '{}' ORDER BY id DESC LIMIT 1",
            stock_code, pred_date
        ))
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
        let mut conn = self.get_conn()?;
        let rows = diesel::sql_query(format!(
            "SELECT id, pred_date, target_date, stock_code, pred_direction, pred_score, actual_change, hit, actual_result FROM prediction_tracker WHERE pred_date = '{}' AND hit IS NULL",
            pred_date
        ))
        .load::<PredictionRow>(&mut *conn)?;
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

    /// 修复 v9.2 BR-001: 统计某只票近 N 天被 push 的次数
    pub fn count_recent_pushes(
        &self,
        stock_code: &str,
        days: i64,
    ) -> Result<i64, Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;
        let cutoff = (chrono::Local::now() - chrono::Duration::days(days))
            .format("%Y-%m-%d")
            .to_string();
        #[derive(serde::Serialize, serde::Deserialize, diesel::QueryableByName)]
        struct CountRow {
            #[diesel(sql_type = diesel::sql_types::BigInt)]
            cnt: i64,
        }
        let raw = format!(
            "SELECT COUNT(*) as cnt FROM prediction_tracker WHERE stock_code = '{}' AND pred_date >= '{}'",
            stock_code, cutoff
        );
        let row = diesel::sql_query(raw).get_result::<CountRow>(&mut *conn)?;
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
        if stock_codes.is_empty() {
            return Ok(std::collections::HashSet::new());
        }
        // 修复 I-5 (2026-06-29 codex review) + review #14:
        // 1. 防 SQL 注入 — 显式 if 校验 stock_code 是 ASCII alphanumeric + 下划线.
        //    原 assert! 在 release 默认被优化掉 (除非显式 panic=abort + debug-assertions),
        //    防护失效. 改为返回 Result 错误, 调用方决定如何处理.
        // 2. 用 diesel prepared statement + ? bind 走参数化, 彻底消除字符串拼接风险.
        for c in stock_codes {
            if !c.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-') {
                return Err(format!(
                    "count_recent_pushes_batch: stock_code must be alphanumeric/_/-, got {:?}",
                    c
                ).into());
            }
        }
        let mut conn = self.get_conn()?;
        let cutoff = (chrono::Local::now() - chrono::Duration::days(days))
            .format("%Y-%m-%d")
            .to_string();
        // 用 IN (?, ?, ...) + bind 走 prepared statement, 字符串拼接为零.
        // SQLite parameter binding 类型安全, 无 escape 风险.
        use diesel::sql_types::Text;
        let placeholders = std::iter::repeat("?")
            .take(stock_codes.len())
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
        let mut conn = self.pool.get()?;
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

    // v14.1 task #167: stock_position.st_type round-trip DB 集成测试
    // 路径: save_position(NewStockPosition{ st_type: Some("*ST") }) →
    //       get_all_open_positions → StockPosition.st_type 真读出
    // 用独立 DB 文件避免与上面 test_save_and_query_stock_data 竞态.
    #[test]
    fn test_st_type_db_round_trip() {
        use crate::models::{NewStockPosition, StockPosition};
        use crate::schema::stock_position;
        use diesel::prelude::*;

        let test_db = "./test_data/test_st_type_round_trip.db";
        std::fs::create_dir_all("./test_data").ok();
        // 删旧文件保证干净
        let _ = std::fs::remove_file(test_db);
        let _ = DatabaseManager::init(Some(PathBuf::from(test_db)));

        let db = DatabaseManager::get();

        // 1. insert 一只 *ST 持仓
        let new_pos = NewStockPosition {
            code: "600090".to_string(),
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
            .filter(stock_position::code.eq("600090"))
            .first(&mut conn)
            .expect("query 失败");
        assert_eq!(row.st_type.as_deref(), Some("*ST"), "st_type 写入/读出不一致");
        assert_eq!(row.code, "600090");
        assert_eq!(row.name, "*ST测试");
        assert_eq!(row.quantity, 1000);

        // 3. 测试 upsert: 同 (code, buy_date) 再 save 不报错, st_type 应被 excluded 同步
        let update_pos = NewStockPosition {
            code: "600090".to_string(),
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
            .filter(stock_position::code.eq("600090"))
            .first(&mut conn)
            .expect("re-query 失败");
        assert_eq!(row2.st_type.as_deref(), Some("ST"), "upsert st_type 未同步");
        assert_eq!(row2.chain_name.as_deref(), Some("化工"), "upsert chain_name 未同步");
        assert_eq!(row2.name, "*ST测试改名", "upsert name 未同步");

        // 4. 清理
        let _ = std::fs::remove_file(test_db);
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
            ("TEST001", "ST康美", Some("ST")),
            ("TEST002", "*ST华微", Some("*ST")),
            ("TEST003", "BEST新材", None),    // 子串含 ST 但不是 ST 类
            ("TEST004", "GST电子", None),     // 子串含 ST 但不是 ST 类
            ("TEST005", "浦发银行", None),    // 普通
            ("TEST006", "SST集成", Some("ST")),
            ("TEST007", "S*ST海伦", Some("*ST")),
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
            }).expect("save 失败");
        }

        // 跑 backfill
        let updated = db.backfill_st_type().expect("backfill 失败");
        assert!(updated > 0, "至少应更新 4 条真 ST 类");

        // 验证每个 case
        let mut conn = db.get_conn().unwrap();
        for (code, name, expected) in &cases {
            let row: StockPosition = stock_position::table
                .filter(stock_position::code.eq(code.as_ref() as &str))
                .first(&mut conn)
                .expect("query 失败");
            assert_eq!(
                row.st_type.as_deref(), *expected,
                "code={code} name={name} expected={expected:?} got={:?}",
                row.st_type
            );
        }

        // 清理
        let _ = std::fs::remove_file(test_db);
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
            code: "600519".to_string(),
            name: "贵州茅台".to_string(),
            buy_date: "2026-07-01".to_string(),
            buy_price: 1800.0,
            quantity: 100,
            status: "open".to_string(),
            st_type: None,
            chain_name: None,
        }).expect("save 1 失败");

        // 2. 模拟 broker 推送 *ST (用 raw SQL 写, 模拟 broker update path)
        let mut conn = db.get_conn().unwrap();
        diesel::sql_query("UPDATE stock_position SET st_type = '*ST' WHERE code = '600519'")
            .execute(&mut conn).expect("st_type set 失败");

        // 3. trading::open_position re-buy 同 (code, buy_date) — 传 None
        db.save_position(&NewStockPosition {
            code: "600519".to_string(),
            name: "贵州茅台".to_string(),
            buy_date: "2026-07-01".to_string(),
            buy_price: 1850.0,  // 价格变 (新买入)
            quantity: 200,       // 数量变
            status: "open".to_string(),
            st_type: None,        // 重买不带 st_type
            chain_name: None,     // 重买不带 chain
        }).expect("save 2 失败");

        // 4. 验证: st_type 应保持 '*ST' (COALESCE 保 NULL 时不覆盖), 价格/数量更新
        let row: StockPosition = stock_position::table
            .filter(stock_position::code.eq("600519"))
            .first(&mut conn)
            .expect("re-query 失败");
        assert_eq!(row.st_type.as_deref(), Some("*ST"),
            "st_type 应保持 broker 推送的 *ST, 不应被 re-buy NULL 覆盖");
        assert_eq!(row.buy_price, 1850.0, "价格应更新");
        assert_eq!(row.quantity, 200, "数量应更新");

        // 清理
        let _ = std::fs::remove_file(test_db);
    }
}
