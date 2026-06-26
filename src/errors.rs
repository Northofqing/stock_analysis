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
