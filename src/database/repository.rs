//! Repository pattern — 解耦数据访问与业务逻辑。
//!
//! 当前 `DatabaseManager` 是全局单例，所有模块直接调用 `DatabaseManager::get()`。
//! Repository trait 允许通过注入 mock 实现进行单元测试。

use async_trait::async_trait;
use chrono::NaiveDate;
use crate::errors::DbError;
use crate::data_provider::KlineData;

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
        &self, code: &str, name: &str, price: f64, shares: i64, date: NaiveDate,
    ) -> Result<(), DbError>;
    /// 记录卖出
    async fn record_sell(
        &self, code: &str, price: f64, shares: i64, date: NaiveDate,
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

fn stock_daily_to_kline(r: StockDaily) -> KlineData {
    KlineData {
        date: r.date,
        open: r.open.unwrap_or(0.0),
        high: r.high.unwrap_or(0.0),
        low: r.low.unwrap_or(0.0),
        close: r.close.unwrap_or(0.0),
        volume: r.volume.unwrap_or(0.0),
        amount: r.amount.unwrap_or(0.0),
        pct_chg: r.pct_chg.unwrap_or(0.0),
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
    }
}

#[async_trait]
impl StockRepository for DatabaseManager {
    async fn find_kline(&self, code: &str, limit: usize) -> Result<Vec<KlineData>, DbError> {
        let rows = self.get_latest_data(code, limit as i64).map_err(|e| {
            DbError::QueryFailed { sql: format!("get_latest_data({code}, {limit}): {e}") }
        })?;
        Ok(rows.into_iter().map(stock_daily_to_kline).collect())
    }

    async fn save_kline(&self, code: &str, data: &[KlineData]) -> Result<usize, DbError> {
        self.save_kline_data(code, data, "repository").map_err(|e| {
            DbError::QueryFailed { sql: format!("save_kline_data({code}): {e}") }
        })
    }

    async fn get_latest_date(&self, code: &str) -> Result<Option<NaiveDate>, DbError> {
        let rows = self.get_latest_data(code, 1).map_err(|e| {
            DbError::QueryFailed { sql: format!("get_latest_data({code}, 1): {e}") }
        })?;
        Ok(rows.first().map(|r| r.date))
    }
}

#[async_trait]
impl TradeRepository for DatabaseManager {
    async fn record_buy(
        &self, code: &str, name: &str, price: f64, shares: i64, date: NaiveDate,
    ) -> Result<(), DbError> {
        insert_trade(self, code, name, "buy", price, shares, date)
    }

    async fn record_sell(
        &self, code: &str, price: f64, shares: i64, date: NaiveDate,
    ) -> Result<(), DbError> {
        insert_trade(self, code, "", "sell", price, shares, date)
    }

    async fn get_positions(&self) -> Result<Vec<(String, String, f64, i64)>, DbError> {
        let rows = self.get_all_open_positions().map_err(|e| {
            DbError::QueryFailed { sql: format!("get_all_open_positions: {e}") }
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
    let mut conn = db
        .get_conn()
        .map_err(|e| DbError::QueryFailed { sql: format!("get_conn: {e}") })?;
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
    .map_err(|e| DbError::QueryFailed { sql: format!("insert trade {code}: {e}") })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

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
        }
    }

    #[tokio::test]
    async fn test_stock_repository_roundtrip() {
        std::fs::create_dir_all("./test_data").ok();
        let _ = DatabaseManager::init(Some(PathBuf::from("./test_data/test.db")));
        let db = DatabaseManager::get();

        // 使用 TEST_ 前缀代码与真实标的硬隔离（AGENTS.md 2.5）
        let code = "TEST_REPO";
        let d = NaiveDate::from_ymd_opt(2026, 1, 2).unwrap();
        let saved = StockRepository::save_kline(db, code, &[make_kline(d, 12.34)])
            .await
            .expect("save_kline 应成功");
        assert_eq!(saved, 1);

        let got = StockRepository::find_kline(db, code, 5)
            .await
            .expect("find_kline 应成功");
        assert!(!got.is_empty());
        assert_eq!(got[0].close, 12.34);

        let latest = StockRepository::get_latest_date(db, code)
            .await
            .expect("get_latest_date 应成功");
        assert_eq!(latest, Some(d));
    }
}
