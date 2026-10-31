use crate::{DataBatch, FiniteNumber, NonEmptyText, Price, Ratio, SourceEvidence, SourcedRecord};
use serde::{de, Deserialize, Deserializer, Serialize};
use std::collections::HashSet;

/// Global index identities admitted by the verified Sina packet family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GlobalIndexCode {
    DowJones,
    NasdaqComposite,
    Sp500,
    Nikkei225,
    HangSeng,
    Ftse100,
}

/// Bounded, duplicate-free global-index snapshot request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GlobalIndexRequest {
    indices: Vec<GlobalIndexCode>,
}

impl GlobalIndexRequest {
    pub fn new(indices: Vec<GlobalIndexCode>) -> Result<Self, crate::CoreError> {
        validate_identities(&indices, "global index", 20)?;
        Ok(Self { indices })
    }

    pub fn indices(&self) -> &[GlobalIndexCode] {
        &self.indices
    }
}

impl<'de> Deserialize<'de> for GlobalIndexRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            indices: Vec<GlobalIndexCode>,
        }
        Self::new(Wire::deserialize(deserializer)?.indices).map_err(de::Error::custom)
    }
}

/// One normalized global-index snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GlobalIndexQuote {
    pub index: GlobalIndexCode,
    pub name: NonEmptyText,
    pub value: Price,
    pub change: FiniteNumber,
    pub change_percent: Ratio,
    pub evidence: SourceEvidence,
}

impl SourcedRecord for GlobalIndexQuote {
    fn provider_id(&self) -> crate::ProviderId {
        self.evidence.provider()
    }

    fn evidence_batch_id(&self) -> &str {
        self.evidence.batch_id()
    }
}

/// Foreign-exchange pairs admitted by the verified Sina packet family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FxPair {
    UsdCny,
    EurUsd,
    UsdJpy,
    GbpUsd,
    AudUsd,
    UsdChf,
    UsdCad,
    NzdUsd,
}

/// Bounded, duplicate-free foreign-exchange snapshot request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FxRequest {
    pairs: Vec<FxPair>,
}

impl FxRequest {
    pub fn new(pairs: Vec<FxPair>) -> Result<Self, crate::CoreError> {
        validate_identities(&pairs, "FX pair", 20)?;
        Ok(Self { pairs })
    }

    pub fn pairs(&self) -> &[FxPair] {
        &self.pairs
    }
}

impl<'de> Deserialize<'de> for FxRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            pairs: Vec<FxPair>,
        }
        Self::new(Wire::deserialize(deserializer)?.pairs).map_err(de::Error::custom)
    }
}

/// One normalized foreign-exchange snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FxQuote {
    pub pair: FxPair,
    pub name: NonEmptyText,
    pub rate: Price,
    pub change: Option<FiniteNumber>,
    pub change_percent: Option<Ratio>,
    pub evidence: SourceEvidence,
}

impl SourcedRecord for FxQuote {
    fn provider_id(&self) -> crate::ProviderId {
        self.evidence.provider()
    }

    fn evidence_batch_id(&self) -> &str {
        self.evidence.batch_id()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct GlobalMarketCapabilities {
    pub indices: bool,
    pub foreign_exchange: bool,
}

pub trait GlobalIndexProvider {
    type Error: std::error::Error + Send + Sync + 'static;

    fn global_indices(
        &self,
        request: &GlobalIndexRequest,
    ) -> Result<DataBatch<GlobalIndexQuote>, Self::Error>;
}

pub trait ForeignExchangeProvider {
    type Error: std::error::Error + Send + Sync + 'static;

    fn foreign_exchange(&self, request: &FxRequest) -> Result<DataBatch<FxQuote>, Self::Error>;
}

fn validate_identities<T>(
    values: &[T],
    family: &'static str,
    maximum: usize,
) -> Result<(), crate::CoreError>
where
    T: Copy + Eq + std::hash::Hash,
{
    if values.is_empty() {
        return Err(crate::CoreError::InvalidRequest(format!(
            "{family} request must not be empty"
        )));
    }
    if values.len() > maximum {
        return Err(crate::CoreError::InvalidRequest(format!(
            "{family} request accepts at most {maximum} identities"
        )));
    }
    let mut seen = HashSet::with_capacity(values.len());
    if values.iter().copied().any(|value| !seen.insert(value)) {
        return Err(crate::CoreError::InvalidRequest(format!(
            "{family} request contains duplicate identities"
        )));
    }
    Ok(())
}

#[cfg(test)]
#[path = "../tests/internal/global_tests.rs"]
mod tests;
