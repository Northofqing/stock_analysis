//! RatioUnit / Ratio / Price / Quantity / Money / FiniteNumber / PositiveU32
//! 本地镜像 (M5, Task #76, feature 关时使用)。
//! 与上游 magic_market_core rev 75ee2a2 (value.rs + validated.rs) 同构:
//! derive 集/校验语义/serde 表示一致 (wire 是 JSON, convert.rs 依赖)。

#[cfg(not(feature = "magic-gateway"))]
use serde::{de, Deserialize, Deserializer, Serialize};

/// Unit for a ratio.
#[cfg(not(feature = "magic-gateway"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RatioUnit {
    Decimal,
    Percent,
}

#[cfg(not(feature = "magic-gateway"))]
macro_rules! finite_type {
    ($name:ident, $field:literal, $pred:expr, $reason:literal) => {
        #[derive(Debug, Clone, Copy, PartialEq, Serialize)]
        #[serde(transparent)]
        pub struct $name(f64);

        impl $name {
            pub fn new(value: f64) -> Result<Self, super::instrument::CoreError> {
                if !value.is_finite() || !(($pred)(value)) {
                    Err(super::instrument::CoreError::InvalidValue {
                        field: $field,
                        value: value.to_string(),
                        reason: $reason,
                    })
                } else {
                    Ok(Self(value))
                }
            }
            pub fn get(self) -> f64 {
                self.0
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = f64::deserialize(deserializer)?;
                Self::new(value).map_err(de::Error::custom)
            }
        }
    };
}

#[cfg(not(feature = "magic-gateway"))]
finite_type!(Price, "price", |v: f64| v > 0.0, "must be positive");
#[cfg(not(feature = "magic-gateway"))]
finite_type!(
    Quantity,
    "quantity",
    |v: f64| v >= 0.0,
    "must be non-negative"
);
#[cfg(not(feature = "magic-gateway"))]
finite_type!(Money, "money", |_v: f64| true, "must be finite");

/// Any finite signed scalar supplied by a source or deterministic analysis.
#[cfg(not(feature = "magic-gateway"))]
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct FiniteNumber(f64);

#[cfg(not(feature = "magic-gateway"))]
impl FiniteNumber {
    pub fn new(value: f64) -> Result<Self, super::instrument::CoreError> {
        if !value.is_finite() {
            return Err(super::instrument::CoreError::InvalidValue {
                field: "finite_number",
                value: value.to_string(),
                reason: "must be finite",
            });
        }
        Ok(Self(value))
    }

    pub fn get(self) -> f64 {
        self.0
    }
}

#[cfg(not(feature = "magic-gateway"))]
impl<'de> Deserialize<'de> for FiniteNumber {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(f64::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

/// Positive one-based count or rank.
#[cfg(not(feature = "magic-gateway"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct PositiveU32(u32);

#[cfg(not(feature = "magic-gateway"))]
impl PositiveU32 {
    pub fn new(value: u32) -> Result<Self, super::instrument::CoreError> {
        if value == 0 {
            return Err(super::instrument::CoreError::InvalidValue {
                field: "positive_u32",
                value: value.to_string(),
                reason: "must be positive",
            });
        }
        Ok(Self(value))
    }

    pub fn get(self) -> u32 {
        self.0
    }
}

#[cfg(not(feature = "magic-gateway"))]
impl<'de> Deserialize<'de> for PositiveU32 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(u32::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

/// A finite decimal or percentage ratio.
#[cfg(not(feature = "magic-gateway"))]
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Ratio {
    value: f64,
    unit: RatioUnit,
}

#[cfg(not(feature = "magic-gateway"))]
impl Ratio {
    pub fn decimal(v: f64) -> Result<Self, super::instrument::CoreError> {
        Self::new(v, RatioUnit::Decimal)
    }
    pub fn new(v: f64, unit: RatioUnit) -> Result<Self, super::instrument::CoreError> {
        if v.is_finite() {
            Ok(Self { value: v, unit })
        } else {
            Err(super::instrument::CoreError::InvalidValue {
                field: "ratio",
                value: v.to_string(),
                reason: "must be finite",
            })
        }
    }
    pub fn get(self) -> f64 {
        self.value
    }
    pub fn unit(self) -> RatioUnit {
        self.unit
    }
}

#[cfg(not(feature = "magic-gateway"))]
impl<'de> Deserialize<'de> for Ratio {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Repr {
            value: f64,
            unit: RatioUnit,
        }

        let repr = Repr::deserialize(deserializer)?;
        Self::new(repr.value, repr.unit).map_err(de::Error::custom)
    }
}
