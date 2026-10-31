use crate::CoreError;
use serde::{de, Deserialize, Deserializer, Serialize};
/// Unit for a ratio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RatioUnit {
    Decimal,
    Percent,
}
macro_rules! finite_type {
    ($name:ident,$field:literal,$pred:expr,$reason:literal) => {
        #[derive(Debug, Clone, Copy, PartialEq, Serialize)]
        pub struct $name(f64);
        impl $name {
            pub fn new(value: f64) -> Result<Self, CoreError> {
                if !value.is_finite() || !(($pred)(value)) {
                    Err(CoreError::InvalidValue {
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
finite_type!(Price, "price", |v: f64| v > 0.0, "must be positive");
finite_type!(
    Quantity,
    "quantity",
    |v: f64| v >= 0.0,
    "must be non-negative"
);
finite_type!(Money, "money", |_v: f64| true, "must be finite");

/// A checked absolute-plus-relative tolerance for comparing finite numbers.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct NumericTolerance {
    absolute: f64,
    relative: f64,
}

impl NumericTolerance {
    /// Creates a tolerance whose components must both be finite and non-negative.
    pub fn new(absolute: f64, relative: f64) -> Result<Self, CoreError> {
        for (field, value) in [
            ("numeric tolerance absolute", absolute),
            ("numeric tolerance relative", relative),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(CoreError::InvalidValue {
                    field,
                    value: value.to_string(),
                    reason: "must be finite and non-negative",
                });
            }
        }
        Ok(Self { absolute, relative })
    }

    /// Returns the absolute component.
    pub fn absolute(self) -> f64 {
        self.absolute
    }

    /// Returns the relative component.
    pub fn relative(self) -> f64 {
        self.relative
    }

    /// Compares two finite numbers with `absolute + relative * max(|a|, |b|)`.
    pub fn matches(self, left: f64, right: f64) -> bool {
        if !left.is_finite() || !right.is_finite() {
            return false;
        }
        let difference = (left - right).abs();
        let threshold = self
            .relative
            .mul_add(left.abs().max(right.abs()), self.absolute);
        difference.is_finite() && threshold.is_finite() && difference <= threshold
    }
}

impl<'de> Deserialize<'de> for NumericTolerance {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Repr {
            absolute: f64,
            relative: f64,
        }

        let repr = Repr::deserialize(deserializer)?;
        Self::new(repr.absolute, repr.relative).map_err(de::Error::custom)
    }
}

/// A finite decimal or percentage ratio.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Ratio {
    value: f64,
    unit: RatioUnit,
}

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
impl Ratio {
    pub fn decimal(v: f64) -> Result<Self, CoreError> {
        Self::new(v, RatioUnit::Decimal)
    }
    pub fn new(v: f64, unit: RatioUnit) -> Result<Self, CoreError> {
        if v.is_finite() {
            Ok(Self { value: v, unit })
        } else {
            Err(CoreError::InvalidValue {
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
