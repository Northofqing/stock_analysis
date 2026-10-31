use crate::{NonEmptyText, ProviderId};
use serde::{de, Deserialize, Deserializer, Serialize};

/// Record-level source and observation evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceEvidence {
    provider: ProviderId,
    source_at: Option<NonEmptyText>,
    observed_at: NonEmptyText,
    batch_id: NonEmptyText,
}

impl SourceEvidence {
    pub fn new(
        provider: ProviderId,
        observed_at: impl Into<String>,
        batch_id: impl Into<String>,
    ) -> Result<Self, crate::CoreError> {
        Ok(Self {
            provider,
            source_at: None,
            observed_at: NonEmptyText::new(observed_at)?,
            batch_id: NonEmptyText::new(batch_id)?,
        })
    }

    pub fn with_source_at(
        mut self,
        source_at: impl Into<String>,
    ) -> Result<Self, crate::CoreError> {
        self.source_at = Some(NonEmptyText::new(source_at)?);
        Ok(self)
    }

    pub fn provider(&self) -> ProviderId {
        self.provider
    }

    pub fn source_at(&self) -> Option<&str> {
        self.source_at.as_ref().map(NonEmptyText::as_str)
    }

    pub fn observed_at(&self) -> &str {
        self.observed_at.as_str()
    }

    pub fn batch_id(&self) -> &str {
        self.batch_id.as_str()
    }
}

impl<'de> Deserialize<'de> for SourceEvidence {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            provider: ProviderId,
            source_at: Option<String>,
            observed_at: String,
            batch_id: String,
        }

        let wire = Wire::deserialize(deserializer)?;
        let mut evidence =
            Self::new(wire.provider, wire.observed_at, wire.batch_id).map_err(de::Error::custom)?;
        if let Some(source_at) = wire.source_at {
            evidence = evidence
                .with_source_at(source_at)
                .map_err(de::Error::custom)?;
        }
        Ok(evidence)
    }
}
