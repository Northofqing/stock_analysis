//! Locally owned date, quality, batch, provenance, and capital-flow record types.
//! Field, validation, and serde representations are stable transport contracts.

use serde::{de, Deserialize, Deserializer, Serialize};
use std::fmt;

/// Valid Gregorian calendar date encoded as `YYYY-MM-DD`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct IsoDate(String);

impl IsoDate {
    pub fn new(value: impl Into<String>) -> Result<Self, super::instrument::CoreError> {
        let value = value.into();
        if !is_valid_iso_date(&value) {
            return Err(super::instrument::CoreError::InvalidValue {
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
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    (1..=days).contains(&day)
}

/// Quality state attached to returned records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QualityReport {
    complete: bool,
    issues: Vec<String>,
}

impl QualityReport {
    fn new(issues: Vec<String>) -> Result<Self, super::instrument::CoreError> {
        let mut checked = Vec::with_capacity(issues.len());
        for issue in issues {
            let trimmed = issue.trim();
            if trimmed.is_empty() {
                return Err(super::instrument::CoreError::InvalidValue {
                    field: "quality_issue",
                    value: issue,
                    reason: "must not be empty",
                });
            }
            if trimmed.chars().any(char::is_control) {
                return Err(super::instrument::CoreError::InvalidValue {
                    field: "quality_issue",
                    value: issue,
                    reason: "must not contain control characters",
                });
            }
            checked.push(trimmed.to_owned());
        }
        Ok(Self {
            complete: checked.is_empty(),
            issues: checked,
        })
    }
    pub fn is_complete(&self) -> bool {
        self.complete
    }
    pub fn issues(&self) -> &[String] {
        &self.issues
    }
}

impl Default for QualityReport {
    fn default() -> Self {
        Self {
            complete: true,
            issues: Vec::new(),
        }
    }
}

impl<'de> Deserialize<'de> for QualityReport {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Repr {
            complete: bool,
            issues: Vec<String>,
        }
        let repr = Repr::deserialize(deserializer)?;
        let value = Self::new(repr.issues).map_err(de::Error::custom)?;
        if value.complete != repr.complete {
            return Err(de::Error::custom(
                "quality complete flag contradicts issue list",
            ));
        }
        Ok(value)
    }
}

/// Source and retrieval timestamps for a batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Provenance {
    source: String,
    source_at: Option<String>,
    fetched_at: String,
    /// Stable per-batch evidence identifier supplied by the provider facade.
    batch_id: Option<String>,
}

impl Provenance {
    pub fn new(
        source: impl Into<String>,
        fetched_at: impl Into<String>,
    ) -> Result<Self, super::instrument::CoreError> {
        let source = checked_text("source", source)?;
        let fetched_at = checked_text("fetched_at", fetched_at)?;
        Ok(Self {
            batch_id: Some(format!("{source}:{fetched_at}")),
            source,
            source_at: None,
            fetched_at,
        })
    }
    pub fn with_source_at(
        mut self,
        v: impl Into<String>,
    ) -> Result<Self, super::instrument::CoreError> {
        self.source_at = Some(checked_text("source_at", v)?);
        Ok(self)
    }
    /// Overrides the generated batch identifier with a provider-issued one.
    pub fn with_batch_id(
        mut self,
        v: impl Into<String>,
    ) -> Result<Self, super::instrument::CoreError> {
        self.batch_id = Some(checked_text("batch_id", v)?);
        Ok(self)
    }
    pub fn source(&self) -> &str {
        &self.source
    }
    pub fn source_at(&self) -> Option<&str> {
        self.source_at.as_deref()
    }
    pub fn fetched_at(&self) -> &str {
        &self.fetched_at
    }
    pub fn batch_id(&self) -> Option<&str> {
        self.batch_id.as_deref()
    }
}

fn checked_text(
    field: &'static str,
    value: impl Into<String>,
) -> Result<String, super::instrument::CoreError> {
    let value = value.into();
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(super::instrument::CoreError::InvalidValue {
            field,
            value,
            reason: "must not be empty",
        });
    }
    if trimmed.chars().any(char::is_control) {
        return Err(super::instrument::CoreError::InvalidValue {
            field,
            value,
            reason: "must not contain control characters",
        });
    }
    Ok(trimmed.to_owned())
}

impl<'de> Deserialize<'de> for Provenance {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Repr {
            source: String,
            source_at: Option<String>,
            fetched_at: String,
            batch_id: Option<String>,
        }

        let repr = Repr::deserialize(deserializer)?;
        let mut value = Self::new(repr.source, repr.fetched_at).map_err(de::Error::custom)?;
        if let Some(source_at) = repr.source_at {
            value = value.with_source_at(source_at).map_err(de::Error::custom)?;
        }
        match repr.batch_id {
            Some(batch_id) => {
                value = value.with_batch_id(batch_id).map_err(de::Error::custom)?;
            }
            None => value.batch_id = None,
        }
        Ok(value)
    }
}

/// Records plus provenance and quality metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataBatch<T> {
    records: Vec<T>,
    provenance: Provenance,
    quality: QualityReport,
}

impl<T> DataBatch<T> {
    pub fn strict(records: Vec<T>, provenance: Provenance) -> Self {
        Self {
            records,
            provenance,
            quality: QualityReport::default(),
        }
    }
    pub fn records(&self) -> &[T] {
        &self.records
    }
    pub fn provenance(&self) -> &Provenance {
        &self.provenance
    }
    pub fn quality(&self) -> &QualityReport {
        &self.quality
    }
    pub fn into_records(self) -> Vec<T> {
        self.records
    }

    /// Constructs a batch whose completeness is explicitly reported.
    pub fn best_effort(
        records: Vec<T>,
        provenance: Provenance,
        issues: Vec<String>,
    ) -> Result<Self, super::instrument::CoreError> {
        Ok(Self {
            records,
            provenance,
            quality: QualityReport::new(issues)?,
        })
    }
}

/// Fund-flow reporting interval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FlowInterval {
    Minute1,
    Day1,
    Day5,
    Day10,
    Day120,
}

/// Stock Connect northbound venue whose daily statistics are being requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NorthboundChannel {
    Shanghai,
    Shenzhen,
}
