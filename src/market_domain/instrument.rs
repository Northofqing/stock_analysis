//! Locally owned exchange, asset-class, instrument, and core error types.
//! Variant, field, validation, and serde representations are stable contracts.

use serde::{de, Deserialize, Deserializer, Serialize};
use std::fmt;

/// Errors raised while constructing core values.
/// 与上游同 derive 集 (Debug, PartialEq) + thiserror Display 字符串。
#[derive(Debug, PartialEq)]
pub enum CoreError {
    InvalidValue {
        field: &'static str,
        value: String,
        reason: &'static str,
    },
    InvalidInstrument(String),
    InvalidRequest(String),
}

impl fmt::Display for CoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidValue {
                field,
                value,
                reason,
            } => write!(formatter, "invalid {field}: {value} ({reason})"),
            Self::InvalidInstrument(message) => {
                write!(formatter, "invalid instrument: {message}")
            }
            Self::InvalidRequest(message) => write!(formatter, "invalid request: {message}"),
        }
    }
}

impl std::error::Error for CoreError {}

/// Trading venue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Exchange {
    Shanghai,
    Shenzhen,
    Beijing,
}

/// Instrument category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AssetClass {
    Equity,
    Index,
    Fund,
    Bond,
    Option,
}

/// Validated exchange instrument identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct InstrumentId {
    exchange: Exchange,
    code: String,
    asset_class: AssetClass,
}

impl InstrumentId {
    /// Constructs an identifier.
    pub fn new(
        exchange: Exchange,
        code: impl Into<String>,
        asset_class: AssetClass,
    ) -> Result<Self, CoreError> {
        let code = code.into().trim().to_owned();
        if code.is_empty() {
            return Err(CoreError::InvalidInstrument("empty code".into()));
        }
        if code.chars().any(char::is_control) {
            return Err(CoreError::InvalidInstrument(
                "code contains control characters".into(),
            ));
        }
        Ok(Self {
            exchange,
            code,
            asset_class,
        })
    }
    /// Venue.
    pub fn exchange(&self) -> Exchange {
        self.exchange
    }
    /// Code.
    pub fn code(&self) -> &str {
        &self.code
    }
    /// Category.
    pub fn asset_class(&self) -> AssetClass {
        self.asset_class
    }
}

impl<'de> Deserialize<'de> for InstrumentId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Repr {
            exchange: Exchange,
            code: String,
            asset_class: AssetClass,
        }

        let repr = Repr::deserialize(deserializer)?;
        Self::new(repr.exchange, repr.code, repr.asset_class).map_err(de::Error::custom)
    }
}
