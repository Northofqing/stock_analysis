use crate::CoreError;
use serde::{de, Deserialize, Deserializer, Serialize};
use std::fmt;

const MAX_TEXT_CHARS: usize = 16_384;
const MAX_URL_CHARS: usize = 4_096;

/// Trimmed, non-empty, control-free text bounded for untrusted source payloads.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct NonEmptyText(String);

impl NonEmptyText {
    pub fn new(value: impl Into<String>) -> Result<Self, CoreError> {
        let value = value.into();
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(CoreError::InvalidValue {
                field: "text",
                value,
                reason: "must not be empty",
            });
        }
        if trimmed.chars().any(char::is_control) {
            return Err(CoreError::InvalidValue {
                field: "text",
                value,
                reason: "must not contain control characters",
            });
        }
        if trimmed.chars().count() > MAX_TEXT_CHARS {
            return Err(CoreError::InvalidValue {
                field: "text",
                value: format!("{} characters", trimmed.chars().count()),
                reason: "exceeds maximum length",
            });
        }
        Ok(Self(trimmed.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for NonEmptyText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for NonEmptyText {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

/// Bounded canonical HTTPS URL.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct HttpsUrl(String);

impl HttpsUrl {
    pub fn new(value: impl Into<String>) -> Result<Self, CoreError> {
        let value = value.into();
        let trimmed = value.trim();
        if trimmed.chars().count() > MAX_URL_CHARS {
            return Err(CoreError::InvalidValue {
                field: "https_url",
                value: format!("{} characters", trimmed.chars().count()),
                reason: "exceeds maximum length",
            });
        }
        if trimmed.chars().any(|character| {
            character.is_control() || character.is_whitespace() || character == '\\'
        }) {
            return Err(CoreError::InvalidValue {
                field: "https_url",
                value,
                reason: "must not contain whitespace, controls or backslashes",
            });
        }
        let remainder =
            trimmed
                .strip_prefix("https://")
                .ok_or_else(|| CoreError::InvalidValue {
                    field: "https_url",
                    value: value.clone(),
                    reason: "must use https",
                })?;
        let authority = remainder.split(['/', '?', '#']).next().unwrap_or_default();
        if authority.is_empty() || authority.contains('@') || authority.starts_with('.') {
            return Err(CoreError::InvalidValue {
                field: "https_url",
                value,
                reason: "must contain a valid host without credentials",
            });
        }
        Ok(Self(trimmed.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for HttpsUrl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for HttpsUrl {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

/// Valid Gregorian calendar date encoded as `YYYY-MM-DD`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct IsoDate(String);

impl IsoDate {
    pub fn new(value: impl Into<String>) -> Result<Self, CoreError> {
        let value = value.into();
        if !is_valid_iso_date(&value) {
            return Err(CoreError::InvalidValue {
                field: "iso_date",
                value,
                reason: "must be a valid YYYY-MM-DD Gregorian date",
            });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for IsoDate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for IsoDate {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

fn is_valid_iso_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit())
    {
        return false;
    }
    let year = value[0..4].parse::<u32>().unwrap_or(0);
    let month = value[5..7].parse::<u32>().unwrap_or(0);
    let day = value[8..10].parse::<u32>().unwrap_or(0);
    if !(1900..=9999).contains(&year) || !(1..=12).contains(&month) {
        return false;
    }
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    (1..=days).contains(&day)
}

/// Any finite signed scalar supplied by a source or deterministic analysis.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct FiniteNumber(f64);

impl FiniteNumber {
    pub fn new(value: f64) -> Result<Self, CoreError> {
        if !value.is_finite() {
            return Err(CoreError::InvalidValue {
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

impl<'de> Deserialize<'de> for FiniteNumber {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(f64::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

/// Positive one-based count or rank.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct PositiveU32(u32);

impl PositiveU32 {
    pub fn new(value: u32) -> Result<Self, CoreError> {
        if value == 0 {
            return Err(CoreError::InvalidValue {
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

impl<'de> Deserialize<'de> for PositiveU32 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(u32::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}
