// -*- coding: utf-8 -*-
//! ===================================
//! A股自选股智能分析系统 - 数据库管理
//! ===================================
//!
//! 职责：
//! 1. 管理 SQLite 数据库连接（单例模式）
//! 2. 提供数据存取接口
//! 3. 实现智能更新逻辑（断点续传）

use chrono::{Local, NaiveDate};
use diesel::prelude::*;
use diesel::r2d2::{ConnectionManager, Pool, PooledConnection};
use log::{info, warn};
use once_cell::sync::OnceCell;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::models::{MaStatus, NewStockDaily, StockDaily, NewLhbDaily, LhbDaily, NewAnalysisResult, AnalysisResultRecord};
use crate::schema::{stock_daily, lhb_daily, analysis_result};

type DbPool = Pool<ConnectionManager<SqliteConnection>>;
type DbConnection = PooledConnection<ConnectionManager<SqliteConnection>>;

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

        Ok(())
    }

    /// 检查是否已有指定日期的数据
    ///
    /// 用于断点续传逻辑：如果已有数据则跳过网络请求
    pub fn has_data_for_date(
        &self,
        code: &str,
        target_date: NaiveDate,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;

        let count: i64 = stock_daily::table
            .filter(stock_daily::code.eq(code))
            .filter(stock_daily::date.eq(target_date))
            .count()
            .get_result(&mut conn)?;

        Ok(count > 0)
    }

    /// 检查是否有今天的数据
    pub fn has_today_data(&self, code: &str) -> Result<bool, Box<dyn std::error::Error>> {
        let today = Local::now().date_naive();
        self.has_data_for_date(code, today)
    }

    /// 获取最近 N 天的数据
    ///
    /// 用于计算"相比昨日"的变化
    pub fn get_latest_data(
        &self,
        code: &str,
        days: i64,
    ) -> Result<Vec<StockDaily>, Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;

        let results = stock_daily::table
            .filter(stock_daily::code.eq(code))
            .order(stock_daily::date.desc())
            .limit(days)
            .load::<StockDaily>(&mut conn)?;

        Ok(results)
    }

    /// 获取指定日期范围的数据
    pub fn get_data_range(
        &self,
        code: &str,
        start_date: NaiveDate,
        end_date: NaiveDate,
    ) -> Result<Vec<StockDaily>, Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;

        let results = stock_daily::table
            .filter(stock_daily::code.eq(code))
            .filter(stock_daily::date.ge(start_date))
            .filter(stock_daily::date.le(end_date))
            .order(stock_daily::date.asc())
            .load::<StockDaily>(&mut conn)?;

        Ok(results)
    }

    /// 保存单条日线数据
    ///
    /// 策略：使用 ON CONFLICT DO UPDATE（单条 SQL 完成 UPSERT）
    pub fn save_daily_record(
        &self,
        code: &str,
        date: NaiveDate,
        open: Option<f64>,
        high: Option<f64>,
        low: Option<f64>,
        close: Option<f64>,
        volume: Option<f64>,
        amount: Option<f64>,
        pct_chg: Option<f64>,
        ma5: Option<f64>,
        ma10: Option<f64>,
        ma20: Option<f64>,
        volume_ratio: Option<f64>,
        data_source: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;
        Self::upsert_daily_record(&mut conn, code, date, open, high, low, close, volume, amount, pct_chg, ma5, ma10, ma20, volume_ratio, data_source)
    }

    /// 内部 UPSERT 方法，接受已有连接（避免批量操作时重复获取连接）
    fn upsert_daily_record(
        conn: &mut DbConnection,
        code: &str,
        date: NaiveDate,
        open: Option<f64>,
        high: Option<f64>,
        low: Option<f64>,
        close: Option<f64>,
        volume: Option<f64>,
        amount: Option<f64>,
        pct_chg: Option<f64>,
        ma5: Option<f64>,
        ma10: Option<f64>,
        ma20: Option<f64>,
        volume_ratio: Option<f64>,
        data_source: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        use diesel::upsert::excluded;

        let new_record = NewStockDaily {
            code: code.to_string(),
            date,
            open,
            high,
            low,
            close,
            volume,
            amount,
            pct_chg,
            ma5,
            ma10,
            ma20,
            volume_ratio,
            data_source: data_source.map(|s| s.to_string()),
        };

        diesel::insert_into(stock_daily::table)
            .values(&new_record)
            .on_conflict((stock_daily::code, stock_daily::date))
            .do_update()
            .set((
                stock_daily::open.eq(excluded(stock_daily::open)),
                stock_daily::high.eq(excluded(stock_daily::high)),
                stock_daily::low.eq(excluded(stock_daily::low)),
                stock_daily::close.eq(excluded(stock_daily::close)),
                stock_daily::volume.eq(excluded(stock_daily::volume)),
                stock_daily::amount.eq(excluded(stock_daily::amount)),
                stock_daily::pct_chg.eq(excluded(stock_daily::pct_chg)),
                stock_daily::ma5.eq(excluded(stock_daily::ma5)),
                stock_daily::ma10.eq(excluded(stock_daily::ma10)),
                stock_daily::ma20.eq(excluded(stock_daily::ma20)),
                stock_daily::volume_ratio.eq(excluded(stock_daily::volume_ratio)),
                stock_daily::data_source.eq(excluded(stock_daily::data_source)),
                stock_daily::updated_at.eq(Local::now().naive_local()),
            ))
            .execute(conn)?;

        Ok(())
    }

    /// 批量保存日线数据
    ///
    /// 使用单连接 + 事务，返回新增/更新的记录数
    pub fn save_daily_batch(
        &self,
        records: &[StockDailyRecord],
    ) -> Result<usize, Box<dyn std::error::Error>> {
        if records.is_empty() {
            return Ok(0);
        }

        let mut conn = self.get_conn()?;
        let saved_count = conn.transaction::<usize, Box<dyn std::error::Error>, _>(|conn| {
            for record in records {
                Self::upsert_daily_record(
                    conn,
                    &record.code,
                    record.date,
                    record.open,
                    record.high,
                    record.low,
                    record.close,
                    record.volume,
                    record.amount,
                    record.pct_chg,
                    record.ma5,
                    record.ma10,
                    record.ma20,
                    record.volume_ratio,
                    record.data_source.as_deref(),
                )?;
            }
            Ok(records.len())
        })?;

        info!("批量保存完成，新增/更新 {} 条记录", saved_count);
        Ok(saved_count)
    }

    /// 获取分析所需的上下文数据
    ///
    /// 返回今日数据 + 昨日数据的对比信息
    pub fn get_analysis_context(
        &self,
        code: &str,
        target_date: Option<NaiveDate>,
    ) -> Result<Option<AnalysisContext>, Box<dyn std::error::Error>> {
        let _target = target_date.unwrap_or_else(|| Local::now().date_naive());

        // 获取最近2天数据
        let recent_data = self.get_latest_data(code, 2)?;

        if recent_data.is_empty() {
            warn!("未找到 {} 的数据", code);
            return Ok(None);
        }

        let today_data = &recent_data[0];
        let yesterday_data = recent_data.get(1);

        let mut context = AnalysisContext {
            code: code.to_string(),
            date: today_data.date,
            today: today_data.to_dict(),
            yesterday: None,
            volume_change_ratio: None,
            price_change_ratio: None,
            ma_status: today_data.analyze_ma_status(),
        };

        if let Some(yesterday) = yesterday_data {
            context.yesterday = Some(yesterday.to_dict());

            // 计算成交量变化
            if let (Some(today_vol), Some(yesterday_vol)) = (today_data.volume, yesterday.volume) {
                if yesterday_vol > 0.0 {
                    context.volume_change_ratio = Some((today_vol / yesterday_vol * 100.0).round() / 100.0);
                }
            }

            // 计算价格变化
            if let (Some(today_close), Some(yesterday_close)) =
                (today_data.close, yesterday.close)
            {
                if yesterday_close > 0.0 {
                    context.price_change_ratio = Some(
                        ((today_close - yesterday_close) / yesterday_close * 100.0 * 100.0).round()
                            / 100.0,
                    );
                }
            }
        }

        Ok(Some(context))
    }

    /// 保存 KlineData 列表到数据库
    ///
    /// 使用单连接 + 事务批量 UPSERT
    pub fn save_kline_data(
        &self,
        code: &str,
        data: &[crate::data_provider::KlineData],
        source: &str,
    ) -> Result<usize, Box<dyn std::error::Error>> {
        if data.is_empty() {
            return Ok(0);
        }

        let mut conn = self.get_conn()?;
        let saved = conn.transaction::<usize, Box<dyn std::error::Error>, _>(|conn| {
            for kline in data {
                Self::upsert_daily_record(
                    conn,
                    code,
                    kline.date,
                    Some(kline.open),
                    Some(kline.high),
                    Some(kline.low),
                    Some(kline.close),
                    Some(kline.volume),
                    Some(kline.amount),
                    Some(kline.pct_chg),
                    None, // ma5 由趋势分析模块计算
                    None, // ma10
                    None, // ma20
                    None, // volume_ratio
                    Some(source),
                )?;
            }
            Ok(data.len())
        })?;

        info!("[{}] 已保存 {} 条K线数据到数据库（数据源: {}）", code, saved, source);
        Ok(saved)
    }

    /// 保存分析结果到数据库（使用 ON CONFLICT DO UPDATE，单条 SQL）
    pub fn save_analysis_result(
        &self,
        result: &NewAnalysisResult,
    ) -> Result<(), Box<dyn std::error::Error>> {
        use diesel::upsert::excluded;

        let mut conn = self.get_conn()?;

        diesel::insert_into(analysis_result::table)
            .values(result)
            .on_conflict((analysis_result::code, analysis_result::date))
            .do_update()
            .set((
                analysis_result::name.eq(excluded(analysis_result::name)),
                analysis_result::sentiment_score.eq(excluded(analysis_result::sentiment_score)),
                analysis_result::operation_advice.eq(excluded(analysis_result::operation_advice)),
                analysis_result::trend_prediction.eq(excluded(analysis_result::trend_prediction)),
                analysis_result::pe_ratio.eq(excluded(analysis_result::pe_ratio)),
                analysis_result::pb_ratio.eq(excluded(analysis_result::pb_ratio)),
                analysis_result::turnover_rate.eq(excluded(analysis_result::turnover_rate)),
                analysis_result::market_cap.eq(excluded(analysis_result::market_cap)),
                analysis_result::circulating_cap.eq(excluded(analysis_result::circulating_cap)),
                analysis_result::close_price.eq(excluded(analysis_result::close_price)),
                analysis_result::pct_chg.eq(excluded(analysis_result::pct_chg)),
                analysis_result::data_source.eq(excluded(analysis_result::data_source)),
            ))
            .execute(&mut conn)?;

        info!("[{}] 保存/更新分析结果（评分: {}）", result.code, result.sentiment_score);
        Ok(())
    }

    /// 获取指定日期的所有分析结果
    #[allow(dead_code)]
    pub fn get_analysis_results_by_date(
        &self,
        date: NaiveDate,
    ) -> Result<Vec<AnalysisResultRecord>, Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;

        let results = analysis_result::table
            .filter(analysis_result::date.eq(date))
            .order(analysis_result::sentiment_score.desc())
            .load::<AnalysisResultRecord>(&mut conn)?;

        Ok(results)
    }

    /// 获取指定股票最近N次分析结果
    #[allow(dead_code)]
    pub fn get_latest_analysis_results(
        &self,
        code: &str,
        limit: i64,
    ) -> Result<Vec<AnalysisResultRecord>, Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;

        let results = analysis_result::table
            .filter(analysis_result::code.eq(code))
            .order(analysis_result::date.desc())
            .limit(limit)
            .load::<AnalysisResultRecord>(&mut conn)?;

        Ok(results)
    }

    /// 删除指定股票的所有数据（用于测试）
    #[allow(dead_code)]
    pub fn delete_stock_data(&self, code: &str) -> Result<usize, Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;

        let deleted = diesel::delete(stock_daily::table.filter(stock_daily::code.eq(code)))
            .execute(&mut conn)?;

        Ok(deleted)
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
// 龙虎榜数据操作
// ============================================================================

impl DatabaseManager {
    /// 保存龙虎榜数据到数据库（事务 + ON CONFLICT DO NOTHING）
    pub fn save_lhb_records(&self, records: &[NewLhbDaily]) -> Result<usize, Box<dyn std::error::Error>> {
        if records.is_empty() {
            return Ok(0);
        }

        let mut conn = self.get_conn()?;
        let saved = conn.transaction::<usize, Box<dyn std::error::Error>, _>(|conn| {
            let mut count = 0;
            for record in records {
                let result = diesel::insert_into(lhb_daily::table)
                    .values(record)
                    .on_conflict((lhb_daily::code, lhb_daily::trade_date))
                    .do_nothing()
                    .execute(conn);

                match result {
                    Ok(n) => count += n,
                    Err(e) => return Err(Box::new(e) as Box<dyn std::error::Error>),
                }
            }
            Ok(count)
        })?;

        Ok(saved)
    }

    /// 检查指定日期的龙虎榜数据是否已缓存
    pub fn has_lhb_data_for_date(&self, trade_date: &str) -> Result<bool, Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;

        let count: i64 = lhb_daily::table
            .filter(lhb_daily::trade_date.eq(trade_date))
            .count()
            .get_result(&mut conn)?;

        Ok(count > 0)
    }

    /// 从数据库获取指定日期的龙虎榜数据（支持模糊匹配）
    pub fn get_lhb_by_date(&self, trade_date: &str) -> Result<Vec<LhbDaily>, Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;

        // 支持日期模糊匹配：2026-01-29 可以匹配 2026-01-29%
        let date_pattern = format!("{}%", trade_date);
        
        let records = lhb_daily::table
            .filter(lhb_daily::trade_date.like(date_pattern))
            .order(lhb_daily::net_amount.desc())
            .load::<LhbDaily>(&mut conn)?;

        Ok(records)
    }

    /// 获取指定股票在某段时间内的龙虎榜上榜次数
    pub fn get_lhb_count_by_code(
        &self,
        code: &str,
        start_date: &str,
        end_date: &str,
    ) -> Result<i64, Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;

        let count = lhb_daily::table
            .filter(lhb_daily::code.eq(code))
            .filter(lhb_daily::trade_date.ge(start_date))
            .filter(lhb_daily::trade_date.le(end_date))
            .count()
            .get_result(&mut conn)?;

        Ok(count)
    }

    /// 清除过期的龙虎榜缓存数据（保留最近N天）
    pub fn clean_old_lhb_data(&self, keep_days: i64) -> Result<usize, Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;
        
        let cutoff_date = Local::now()
            .date_naive()
            .checked_sub_signed(chrono::Duration::days(keep_days))
            .unwrap()
            .format("%Y%m%d")
            .to_string();

        let deleted = diesel::delete(
            lhb_daily::table.filter(lhb_daily::trade_date.lt(cutoff_date))
        )
        .execute(&mut conn)?;

        Ok(deleted)
    }

    /// 去重龙虎榜缓存（同一股票同一日期仅保留最新一条）
    pub fn dedupe_lhb_data(&self) -> Result<usize, Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;

        let deleted = diesel::sql_query(
            r#"
            DELETE FROM lhb_daily
            WHERE id NOT IN (
                SELECT MAX(id)
                FROM lhb_daily
                GROUP BY code, trade_date
            )
            "#,
        )
        .execute(&mut conn)?;

        Ok(deleted)
    }
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
