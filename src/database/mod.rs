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


mod kline;
mod lhb;
mod positions;

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

        let db = DatabaseManager { pool };

        DB_INSTANCE
            .set(db)
            .map_err(|_| "数据库已经初始化")?;

        info!("数据库初始化完成");
        Ok(())
    }

    /// 获取数据库管理器单例
    pub fn get() -> &'static DatabaseManager {
        DB_INSTANCE
            .get()
            .expect("数据库未初始化，请先调用 DatabaseManager::init()")
    }

    /// 获取数据库连接
    fn get_conn(&self) -> Result<DbConnection, Box<dyn std::error::Error>> {
        Ok(self.pool.get()?)
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
                UNIQUE(code, date)
            )
            "#,
        )
        .execute(conn)?;

        // 创建索引
        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS ix_stock_daily_code ON stock_daily(code)",
        )
        .execute(conn)?;

        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS ix_stock_daily_date ON stock_daily(date)",
        )
        .execute(conn)?;

        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS ix_stock_daily_code_date ON stock_daily(code, date)",
        )
        .execute(conn)?;

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
        .execute(conn)?;

        // 创建龙虎榜索引
        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS ix_lhb_daily_code ON lhb_daily(code)",
        )
        .execute(conn)?;

        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS ix_lhb_daily_trade_date ON lhb_daily(trade_date)",
        )
        .execute(conn)?;

        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS ix_lhb_daily_code_date ON lhb_daily(code, trade_date)",
        )
        .execute(conn)?;

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
        .execute(conn)?;

        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS ix_analysis_result_code ON analysis_result(code)",
        )
        .execute(conn)?;

        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS ix_analysis_result_date ON analysis_result(date)",
        )
        .execute(conn)?;

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
        .execute(conn)?;

        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS ix_stock_position_code ON stock_position(code)",
        )
        .execute(conn)?;

        diesel::sql_query(
            "CREATE INDEX IF NOT EXISTS ix_stock_position_status ON stock_position(status)",
        )
        .execute(conn)?;

        Ok(())
    }
}

// ============================================================================
// 辅助数据结构
// ============================================================================

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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn test_database_init() {
        let db_path = PathBuf::from("./test_data/test.db");
        std::fs::create_dir_all("./test_data").ok();

        DatabaseManager::init(Some(db_path)).expect("数据库初始化失败");

        let db = DatabaseManager::get();
        assert!(db.pool.get().is_ok());

        // 清理
        std::fs::remove_dir_all("./test_data").ok();
    }

    #[test]
    fn test_save_and_retrieve() {
        let db_path = PathBuf::from("./test_data/test2.db");
        std::fs::create_dir_all("./test_data").ok();

        DatabaseManager::init(Some(db_path)).expect("数据库初始化失败");
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

        // 清理
        db.delete_stock_data("600519").ok();
        std::fs::remove_dir_all("./test_data").ok();
    }
}
