use thiserror::Error;

use crate::error_codes::{CodedError, ErrorCode};

#[derive(Error, Debug)]
pub enum TdxError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Invalid data: {0}")]
    InvalidData(String),

    #[error(
        "TDX historical bar cardinality mismatch at offset {offset}: returned {actual} rows for \
         exact page limit {expected_page} within total request {requested_total}"
    )]
    HistoricalBarCardinality {
        offset: u32,
        actual: usize,
        expected_page: u16,
        requested_total: u16,
    },

    #[error("File not found: {0}")]
    FileNotFound(String),

    #[error("Connection error: {0}")]
    Connection(String),

    #[error("Connection timeout")]
    ConnectionTimeout,

    #[error("Setup handshake failed: {0}")]
    SetupFailed(String),

    #[error("Response parse error: {0}")]
    ResponseParse(String),

    #[error("Server disconnected")]
    Disconnected,

    #[error("Retry exhausted after {0} attempts")]
    RetryExhausted(usize),

    #[error("Unsupported capability: {0}")]
    Unsupported(String),

    #[error("Core contract error: {0}")]
    Core(#[from] magic_market_core::CoreError),

    #[error("{0}")]
    Coded(CodedError),
}

impl TdxError {
    /// 创建带错误码的错误
    pub fn coded(code: ErrorCode, message: impl Into<String>) -> Self {
        TdxError::Coded(CodedError::new(code, message))
    }

    /// 获取错误码 (如果有)
    pub fn error_code(&self) -> Option<ErrorCode> {
        match self {
            TdxError::Coded(e) => Some(e.code),
            _ => None,
        }
    }

    /// 格式化为用户友好的错误信息 (包含错误码)
    pub fn format_coded(&self) -> String {
        match self {
            TdxError::Coded(e) => e.format(),
            _ => self.to_string(),
        }
    }

    /// 格式化为中文错误信息 (包含错误码)
    pub fn format_coded_zh(&self) -> String {
        match self {
            TdxError::Coded(e) => e.format_zh(),
            _ => self.to_string(),
        }
    }
}

pub type Result<T> = std::result::Result<T, TdxError>;
