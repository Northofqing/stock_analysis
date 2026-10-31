use crate::CoreError;
use serde::{de, Deserialize, Deserializer, Serialize};

pub(crate) fn checked_text(
    field: &'static str,
    value: impl Into<String>,
) -> Result<String, CoreError> {
    let value = value.into();
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(CoreError::InvalidValue {
            field,
            value,
            reason: "must not be empty",
        });
    }
    if trimmed.chars().any(char::is_control) {
        return Err(CoreError::InvalidValue {
            field,
            value,
            reason: "must not contain control characters",
        });
    }
    Ok(trimmed.to_owned())
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
    ) -> Result<Self, CoreError> {
        let source = checked_text("source", source)?;
        let fetched_at = checked_text("fetched_at", fetched_at)?;
        Ok(Self {
            batch_id: Some(format!("{source}:{fetched_at}")),
            source,
            source_at: None,
            fetched_at,
        })
    }
    pub fn with_source_at(mut self, v: impl Into<String>) -> Result<Self, CoreError> {
        self.source_at = Some(checked_text("source_at", v)?);
        Ok(self)
    }
    /// Overrides the generated batch identifier with a provider-issued one.
    pub fn with_batch_id(mut self, v: impl Into<String>) -> Result<Self, CoreError> {
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
