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
