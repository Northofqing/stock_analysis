//! 领域错误类型
//!
//! 渐进从 anyhow::Error 迁移到此模块的具名错误枚举。
//! 二进制入口层（main.rs, monitor.rs）仍使用 anyhow 做胶水层。

use thiserror::Error;

#[derive(Error, Debug)]
pub enum ProviderError {
    #[error("API key exhausted for {provider}")]
    KeyExhausted { provider: String },

    #[error("HTTP timeout after {seconds}s for {url}")]
    Timeout { seconds: u64, url: String },

    #[error("Rate limited for {provider}, retry after {retry_after}s")]
    RateLimited { provider: String, retry_after: u64 },

    #[error("Data not found: {code}")]
    NotFound { code: String },

    #[error("Parse error: {detail}")]
    ParseError { detail: String },

    #[error("{provider}: {detail}")]
    Other { provider: String, detail: String },
}

impl ProviderError {
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::Timeout { .. } | Self::RateLimited { .. })
    }
}

#[derive(Error, Debug)]
pub enum DbError {
    #[error("Connection pool exhausted")]
    PoolExhausted,

    #[error("Query failed: {sql}")]
    QueryFailed { sql: String },

    #[error("Migration failed: {detail}")]
    MigrationFailed { detail: String },

    #[error("Record not found: {table} where {condition}")]
    NotFound { table: String, condition: String },
}

#[derive(Error, Debug)]
pub enum SearchError {
    #[error("Search timeout: {query}")]
    Timeout { query: String },

    #[error("All providers exhausted for query: {query}")]
    AllProvidersExhausted { query: String },

    #[error("Invalid query: {reason}")]
    InvalidQuery { reason: String },
}

/// 交易/下单领域错误（对齐 AGENTS.md 2.6 写入侧防护红线）
#[derive(Error, Debug)]
pub enum TradeError {
    #[error("资金不足：需 {needed:.2} 元，可用 {available:.2} 元")]
    InsufficientFunds { needed: f64, available: f64 },

    #[error("数量非法：{shares} 股，必须为正且为 100 股整数倍")]
    InvalidQuantity { shares: i64 },

    #[error("委托价 {price:.2} 超出涨跌停区间 [{low:.2}, {high:.2}]")]
    PriceOutOfLimit { price: f64, low: f64, high: f64 },

    #[error("单笔金额 {amount:.2} 元超过上限 {limit:.2} 元")]
    AmountExceedsLimit { amount: f64, limit: f64 },

    #[error("持仓不存在：{code}")]
    PositionNotFound { code: String },

    #[error("重复订单：业务号 {order_id} 在去重窗口内重复提交")]
    DuplicateOrder { order_id: String },
}
