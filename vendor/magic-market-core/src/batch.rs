use crate::{CoreError, Provenance};
use serde::{de, Deserialize, Deserializer, Serialize};
/// Quality state attached to returned records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QualityReport {
    complete: bool,
    issues: Vec<String>,
}
impl QualityReport {
    fn new(issues: Vec<String>) -> Result<Self, CoreError> {
        let mut checked = Vec::with_capacity(issues.len());
        for issue in issues {
            let trimmed = issue.trim();
            if trimmed.is_empty() {
                return Err(CoreError::InvalidValue {
                    field: "quality_issue",
                    value: issue,
                    reason: "must not be empty",
                });
            }
            if trimmed.chars().any(char::is_control) {
                return Err(CoreError::InvalidValue {
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
    ) -> Result<Self, CoreError> {
        Ok(Self {
            records,
            provenance,
            quality: QualityReport::new(issues)?,
        })
    }
}
