use crate::{
    DataBatch, HttpsUrl, IsoDate, NonEmptyText, PositiveU32, SourceEvidence, SourcedRecord,
};
use serde::{de, Deserialize, Deserializer, Serialize};

/// Bounded official-policy-library page request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PolicyRequest {
    query: Option<NonEmptyText>,
    start: Option<IsoDate>,
    end: Option<IsoDate>,
    page: PositiveU32,
    page_size: PositiveU32,
}

impl PolicyRequest {
    pub fn new(page: PositiveU32, page_size: PositiveU32) -> Result<Self, crate::CoreError> {
        if page_size.get() > 50 {
            return Err(crate::CoreError::InvalidRequest(
                "policy page_size must be at most 50".into(),
            ));
        }
        Ok(Self {
            query: None,
            start: None,
            end: None,
            page,
            page_size,
        })
    }

    pub fn with_query(mut self, query: impl Into<String>) -> Result<Self, crate::CoreError> {
        self.query = Some(NonEmptyText::new(query)?);
        Ok(self)
    }

    pub fn with_range(mut self, start: IsoDate, end: IsoDate) -> Result<Self, crate::CoreError> {
        if start > end {
            return Err(crate::CoreError::InvalidRequest(
                "policy start must not exceed end".into(),
            ));
        }
        self.start = Some(start);
        self.end = Some(end);
        Ok(self)
    }

    pub fn query(&self) -> Option<&NonEmptyText> {
        self.query.as_ref()
    }

    pub fn start(&self) -> Option<&IsoDate> {
        self.start.as_ref()
    }

    pub fn end(&self) -> Option<&IsoDate> {
        self.end.as_ref()
    }

    pub fn page(&self) -> PositiveU32 {
        self.page
    }

    pub fn page_size(&self) -> PositiveU32 {
        self.page_size
    }
}

impl<'de> Deserialize<'de> for PolicyRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            query: Option<String>,
            start: Option<IsoDate>,
            end: Option<IsoDate>,
            page: PositiveU32,
            page_size: PositiveU32,
        }
        let wire = Wire::deserialize(deserializer)?;
        let mut request = Self::new(wire.page, wire.page_size).map_err(de::Error::custom)?;
        if let Some(query) = wire.query {
            request = request.with_query(query).map_err(de::Error::custom)?;
        }
        match (wire.start, wire.end) {
            (Some(start), Some(end)) => {
                request = request.with_range(start, end).map_err(de::Error::custom)?;
            }
            (None, None) => {}
            _ => return Err(de::Error::custom("policy range requires start and end")),
        }
        Ok(request)
    }
}

/// One policy document from the official China Government policy library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyDocument {
    pub document_id: NonEmptyText,
    pub title: NonEmptyText,
    pub summary: Option<NonEmptyText>,
    pub organization: NonEmptyText,
    pub document_number: Option<NonEmptyText>,
    pub category: Option<NonEmptyText>,
    pub published_date: IsoDate,
    pub canonical_url: HttpsUrl,
    pub evidence: SourceEvidence,
}

impl SourcedRecord for PolicyDocument {
    fn provider_id(&self) -> crate::ProviderId {
        self.evidence.provider()
    }

    fn evidence_batch_id(&self) -> &str {
        self.evidence.batch_id()
    }
}

pub trait PolicyDocuments {
    type Error: std::error::Error + Send + Sync + 'static;

    fn policy_documents(
        &self,
        request: &PolicyRequest,
    ) -> Result<DataBatch<PolicyDocument>, Self::Error>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PolicyCapabilities {
    pub official_documents: bool,
}

#[cfg(test)]
#[path = "../tests/internal/policy_tests.rs"]
mod tests;
