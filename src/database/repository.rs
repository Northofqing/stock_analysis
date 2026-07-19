//! Registered business rules: BR-092.
//! Repository pattern — 解耦数据访问与业务逻辑。
//!
//! 当前 `DatabaseManager` 是全局单例，所有模块直接调用 `DatabaseManager::get()`。
//! Repository trait 允许通过注入 mock 实现进行单元测试。

use crate::data_provider::KlineData;
use crate::errors::DbError;
use async_trait::async_trait;
use chrono::NaiveDate;

/// 股票数据仓库（K 线存取）
#[async_trait]
pub trait StockRepository: Send + Sync {
    /// 获取最近 N 条 K 线
    async fn find_kline(&self, code: &str, limit: usize) -> Result<Vec<KlineData>, DbError>;
    /// 保存 K 线
    async fn save_kline(&self, code: &str, data: &[KlineData]) -> Result<usize, DbError>;
    /// 获取最新数据日期
    async fn get_latest_date(&self, code: &str) -> Result<Option<NaiveDate>, DbError>;
}

/// 交易记录仓库
#[async_trait]
pub trait TradeRepository: Send + Sync {
    /// 记录买入
    async fn record_buy(
        &self,
        code: &str,
        name: &str,
        price: f64,
        shares: i64,
        date: NaiveDate,
    ) -> Result<(), DbError>;
    /// 记录卖出
    async fn record_sell(
        &self,
        code: &str,
        price: f64,
        shares: i64,
        date: NaiveDate,
    ) -> Result<(), DbError>;
    /// 获取当前持仓
    async fn get_positions(&self) -> Result<Vec<(String, String, f64, i64)>, DbError>;
}

// ============================================================================
// DatabaseManager 的具体实现
//
// 注：DatabaseManager 底层是同步 diesel（阻塞）。这里在 async fn 内直接调用同步
// 方法——DatabaseManager 是进程级单例，调用开销低，无需 spawn_blocking 包裹。
// 该实现使现有数据访问可通过 trait 注入 mock 进行单元测试（满足 Repository 模式目标）。
// ============================================================================

use crate::database::DatabaseManager;
use crate::models::StockDaily;

fn required_daily_value(date: NaiveDate, field: &str, value: Option<f64>) -> Result<f64, DbError> {
    value.ok_or_else(|| DbError::QueryFailed {
        sql: format!("stock_daily {date} missing required field {field}"),
    })
}

fn stock_daily_to_kline(r: StockDaily) -> Result<KlineData, DbError> {
    Ok(KlineData {
        date: r.date,
        open: required_daily_value(r.date, "open", r.open)?,
        high: required_daily_value(r.date, "high", r.high)?,
        low: required_daily_value(r.date, "low", r.low)?,
        close: required_daily_value(r.date, "close", r.close)?,
        volume: required_daily_value(r.date, "volume", r.volume)?,
        amount: required_daily_value(r.date, "amount", r.amount)?,
        pct_chg: required_daily_value(r.date, "pct_chg", r.pct_chg)?,
        intraday_price: None,
        settled: true,
        pe_ratio: None,
        pb_ratio: None,
        turnover_rate: None,
        market_cap: None,
        circulating_cap: None,
        eps: None,
        roe: None,
        revenue_yoy: None,
        net_profit_yoy: None,
        gross_margin: None,
        net_margin: None,
        sharpe_ratio: None,
        financials_history: None,
        valuation_history: None,
        consensus: None,
        industry: None,
        is_limit_up: false,
        is_limit_down: false,
        is_suspended: false,
        // DB 反序列化: schema 不存 adjust, 字段值不可知. 语义为"上游假定 Qfq".
        adjust: crate::data_provider::AdjustType::None,
    })
}

#[async_trait]
impl StockRepository for DatabaseManager {
    async fn find_kline(&self, code: &str, limit: usize) -> Result<Vec<KlineData>, DbError> {
        let rows = self
            .get_latest_data(code, limit as i64)
            .map_err(|e| DbError::QueryFailed {
                sql: format!("get_latest_data({code}, {limit}): {e}"),
            })?;
        let mut data: Vec<KlineData> = rows
            .into_iter()
            .map(stock_daily_to_kline)
            .collect::<Result<_, _>>()?;
        crate::data_provider::validate_kline_series_strict(&mut data, code).map_err(|error| {
            DbError::QueryFailed {
                sql: format!("validate stock_daily({code}): {error}"),
            }
        })?;
        Ok(data)
    }

    async fn save_kline(&self, code: &str, data: &[KlineData]) -> Result<usize, DbError> {
        self.save_kline_data(code, data, "repository")
            .map_err(|e| DbError::QueryFailed {
                sql: format!("save_kline_data({code}): {e}"),
            })
    }

    async fn get_latest_date(&self, code: &str) -> Result<Option<NaiveDate>, DbError> {
        let rows = self
            .get_latest_data(code, 1)
            .map_err(|e| DbError::QueryFailed {
                sql: format!("get_latest_data({code}, 1): {e}"),
            })?;
        Ok(rows.first().map(|r| r.date))
    }
}

#[async_trait]
impl TradeRepository for DatabaseManager {
    async fn record_buy(
        &self,
        code: &str,
        name: &str,
        price: f64,
        shares: i64,
        date: NaiveDate,
    ) -> Result<(), DbError> {
        insert_trade(self, code, name, "buy", price, shares, date)
    }

    async fn record_sell(
        &self,
        code: &str,
        price: f64,
        shares: i64,
        date: NaiveDate,
    ) -> Result<(), DbError> {
        insert_trade(self, code, "", "sell", price, shares, date)
    }

    async fn get_positions(&self) -> Result<Vec<(String, String, f64, i64)>, DbError> {
        let rows = self
            .get_all_open_positions()
            .map_err(|e| DbError::QueryFailed {
                sql: format!("get_all_open_positions: {e}"),
            })?;
        Ok(rows
            .into_iter()
            .map(|p| (p.code, p.name, p.buy_price, p.quantity as i64))
            .collect())
    }
}

/// 向 trades 表插入一条交易流水（持久化层，业务边界校验由上层完成）。
fn insert_trade(
    db: &DatabaseManager,
    code: &str,
    name: &str,
    direction: &str,
    price: f64,
    shares: i64,
    date: NaiveDate,
) -> Result<(), DbError> {
    use diesel::prelude::*;
    use diesel::sql_types::{BigInt, Double, Text};

    let amount = price * shares as f64;
    let mut conn = db.get_conn().map_err(|e| DbError::QueryFailed {
        sql: format!("get_conn: {e}"),
    })?;
    diesel::sql_query(
        "INSERT INTO trades (code, name, direction, price, shares, amount, reason, traded_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind::<Text, _>(code)
    .bind::<Text, _>(name)
    .bind::<Text, _>(direction)
    .bind::<Double, _>(price)
    .bind::<BigInt, _>(shares)
    .bind::<Double, _>(amount)
    .bind::<Text, _>("repository")
    .bind::<Text, _>(date.format("%Y-%m-%d").to_string())
    .execute(&mut *conn)
    .map_err(|e| DbError::QueryFailed {
        sql: format!("insert trade {code}: {e}"),
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::NewStockPosition;
    use diesel::prelude::*;
    use std::path::PathBuf;

    fn unique_code(label: &str) -> String {
        format!(
            "TEST_CODE_REPOSITORY_{label}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        )
    }

    fn make_kline(date: NaiveDate, close: f64) -> KlineData {
        KlineData {
            date,
            open: close,
            high: close,
            low: close,
            close,
            volume: 1000.0,
            amount: close * 1000.0,
            pct_chg: 0.0,
            intraday_price: None,
            settled: true,
            pe_ratio: None,
            pb_ratio: None,
            turnover_rate: None,
            market_cap: None,
            circulating_cap: None,
            eps: None,
            roe: None,
            revenue_yoy: None,
            net_profit_yoy: None,
            gross_margin: None,
            net_margin: None,
            sharpe_ratio: None,
            financials_history: None,
            valuation_history: None,
            consensus: None,
            industry: None,
            is_limit_up: false,
            is_limit_down: false,
            is_suspended: false,
            adjust: crate::data_provider::AdjustType::None,
        }
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn test_stock_repository_roundtrip() {
        std::fs::create_dir_all("./test_data").ok();
        let _ = DatabaseManager::init(Some(PathBuf::from("./test_data/test.db")));
        let db = DatabaseManager::get();

        // 使用 TEST_ 前缀代码与真实标的硬隔离（AGENTS.md 2.5）
        let code = unique_code("KLINE");
        let d = NaiveDate::from_ymd_opt(2026, 1, 2).unwrap();
        let saved = StockRepository::save_kline(db, &code, &[make_kline(d, 12.34)])
            .await
            .expect("save_kline 应成功");
        assert_eq!(saved, 1);

        let got = StockRepository::find_kline(db, &code, 5)
            .await
            .expect("find_kline 应成功");
        assert!(!got.is_empty());
        assert_eq!(got[0].close, 12.34);

        let latest = StockRepository::get_latest_date(db, &code)
            .await
            .expect("get_latest_date 应成功");
        assert_eq!(latest, Some(d));
        assert_eq!(
            StockRepository::save_kline(db, &code, &[]).await.unwrap(),
            0
        );
        assert!(
            StockRepository::save_kline(db, &code, &[make_kline(d, -1.0)])
                .await
                .is_err()
        );
        assert_eq!(
            StockRepository::get_latest_date(db, &unique_code("MISSING"))
                .await
                .unwrap(),
            None
        );
        db.delete_stock_data(&code).expect("clean kline fixture");
    }

    #[test]
    fn br092_stock_daily_projection_requires_all_source_values() {
        let date = NaiveDate::from_ymd_opt(2026, 7, 18).unwrap();
        let timestamp = date.and_hms_opt(12, 0, 0).unwrap();
        let complete = StockDaily {
            id: 1,
            code: "TEST_CODE_PROJECTION".to_string(),
            date,
            open: Some(10.0),
            high: Some(11.0),
            low: Some(9.0),
            close: Some(10.5),
            volume: Some(1_000.0),
            amount: Some(10_500.0),
            pct_chg: Some(5.0),
            ma5: None,
            ma10: None,
            ma20: None,
            volume_ratio: None,
            data_source: Some("TEST_CODE_SOURCE".to_string()),
            created_at: timestamp,
            updated_at: timestamp,
        };
        let projected = stock_daily_to_kline(complete.clone()).expect("complete row projects");
        assert_eq!(projected.open, 10.0);
        assert_eq!(projected.high, 11.0);
        assert_eq!(projected.low, 9.0);
        assert_eq!(projected.close, 10.5);
        assert_eq!(projected.volume, 1_000.0);
        assert_eq!(projected.amount, 10_500.0);
        assert_eq!(projected.pct_chg, 5.0);
        assert!(projected.settled);
        assert_eq!(projected.adjust, crate::data_provider::AdjustType::None);

        let mut missing = complete;
        missing.amount = None;
        let error = stock_daily_to_kline(missing).expect_err("missing amount must reject");
        assert!(error.to_string().contains("missing required field amount"));
        assert_eq!(required_daily_value(date, "close", Some(9.5)).unwrap(), 9.5);
        assert!(required_daily_value(date, "close", None).is_err());
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn trade_repository_persists_buy_sell_and_projects_open_positions() {
        #[derive(QueryableByName)]
        struct StoredTrade {
            #[diesel(sql_type = diesel::sql_types::Text)]
            direction: String,
            #[diesel(sql_type = diesel::sql_types::Text)]
            name: String,
            #[diesel(sql_type = diesel::sql_types::Double)]
            amount: f64,
        }

        DatabaseManager::init(None).expect("test database init");
        let db = DatabaseManager::get();
        let code = unique_code("TRADE");
        let date = NaiveDate::from_ymd_opt(2026, 7, 18).unwrap();
        TradeRepository::record_buy(db, &code, "测试交易", 10.0, 200, date)
            .await
            .expect("record buy");
        TradeRepository::record_sell(db, &code, 12.0, 100, date)
            .await
            .expect("record sell");

        db.save_position(&NewStockPosition {
            code: code.clone(),
            name: "测试交易".to_string(),
            buy_date: "2026-07-01".to_string(),
            buy_price: 10.0,
            quantity: 200,
            status: "open".to_string(),
            st_type: None,
            chain_name: None,
        })
        .expect("save open position");
        let positions = TradeRepository::get_positions(db)
            .await
            .expect("project open positions");
        assert!(positions.iter().any(|position| {
            position.0 == code
                && position.1 == "测试交易"
                && position.2 == 10.0
                && position.3 == 200
        }));

        let mut conn = db.get_conn().expect("test database connection");
        let trades = diesel::sql_query(
            "SELECT direction, name, amount FROM trades WHERE code = ? ORDER BY id",
        )
        .bind::<diesel::sql_types::Text, _>(&code)
        .load::<StoredTrade>(&mut conn)
        .expect("load stored trades");
        assert_eq!(trades.len(), 2);
        assert_eq!(trades[0].direction, "buy");
        assert_eq!(trades[0].name, "测试交易");
        assert_eq!(trades[0].amount, 2_000.0);
        assert_eq!(trades[1].direction, "sell");
        assert!(trades[1].name.is_empty());
        assert_eq!(trades[1].amount, 1_200.0);

        diesel::sql_query("DELETE FROM trades WHERE code = ?")
            .bind::<diesel::sql_types::Text, _>(&code)
            .execute(&mut conn)
            .expect("clean trade fixtures");
        diesel::delete(
            crate::schema::stock_position::table
                .filter(crate::schema::stock_position::code.eq(&code)),
        )
        .execute(&mut conn)
        .expect("clean position fixture");
    }
}
