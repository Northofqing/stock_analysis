use thiserror::Error;
/// Errors raised while constructing core values.
#[derive(Debug, Error, PartialEq)]
pub enum CoreError {
    #[error("invalid {field}: {value} ({reason})")]
    InvalidValue {
        field: &'static str,
        value: String,
        reason: &'static str,
    },
    #[error("invalid instrument: {0}")]
    InvalidInstrument(String),
    #[error("invalid request: {0}")]
    InvalidRequest(String),
}
