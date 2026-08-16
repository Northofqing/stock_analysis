//! Exchange / AssetClass / InstrumentId / CoreError 本地镜像 (M5, Task #76,
//! feature 关时使用)。与上游 magic_market_core (pin rev 75ee2a2,
//! crates/magic-market-core/src/instrument.rs + error.rs) 同构:
//! 变体集/字段/serde 表示/校验语义/Display 字符串一致 (wire 是 JSON)。

#[cfg(not(feature = "magic-gateway"))]
use serde::{de, Deserialize, Deserializer, Serialize};
#[cfg(not(feature = "magic-gateway"))]
use std::fmt;

/// Errors raised while constructing core values.
/// 与上游同 derive 集 (Debug, PartialEq) + thiserror Display 字符串。
#[cfg(not(feature = "magic-gateway"))]
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

#[cfg(not(feature = "magic-gateway"))]
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

#[cfg(not(feature = "magic-gateway"))]
impl std::error::Error for CoreError {}

/// Trading venue.
#[cfg(not(feature = "magic-gateway"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Exchange {
    Shanghai,
    Shenzhen,
    Beijing,
}

/// Instrument category.
#[cfg(not(feature = "magic-gateway"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AssetClass {
    Equity,
    Index,
    Fund,
    Bond,
    Option,
}

/// Validated exchange instrument identifier.
#[cfg(not(feature = "magic-gateway"))]
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct InstrumentId {
    exchange: Exchange,
    code: String,
    asset_class: AssetClass,
}

#[cfg(not(feature = "magic-gateway"))]
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

#[cfg(not(feature = "magic-gateway"))]
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
