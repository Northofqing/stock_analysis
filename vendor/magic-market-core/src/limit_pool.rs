use crate::{
    DataBatch, InstrumentId, IsoDate, Money, NonEmptyText, PositiveU32, Price, Quantity, Ratio,
    SourceEvidence, SourcedRecord,
};
use serde::{de, Deserialize, Deserializer, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LimitPoolKind {
    Upper,
    Broken,
    Lower,
    PreviousUpper,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LimitPoolEntry {
    pub kind: LimitPoolKind,
    pub instrument: InstrumentId,
    pub trading_date: IsoDate,
    pub price: Price,
    pub change: Ratio,
    pub volume: Option<Quantity>,
    pub turnover: Option<Ratio>,
    pub sealed_amount: Option<Money>,
    pub first_seal_at: Option<NonEmptyText>,
    pub last_seal_at: Option<NonEmptyText>,
    pub break_count: Option<u32>,
    pub streak: Option<PositiveU32>,
    pub industry: Option<NonEmptyText>,
    pub board_name: Option<NonEmptyText>,
    pub seal_state: Option<NonEmptyText>,
    pub reseal_count: Option<u32>,
    pub reason: Option<NonEmptyText>,
    pub evidence: SourceEvidence,
}

impl SourcedRecord for LimitPoolEntry {
    fn provider_id(&self) -> crate::ProviderId {
        self.evidence.provider()
    }

    fn evidence_batch_id(&self) -> &str {
        self.evidence.batch_id()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LimitPoolRequest {
    kind: LimitPoolKind,
    trading_date: IsoDate,
    limit: PositiveU32,
}

impl LimitPoolRequest {
    pub fn new(
        kind: LimitPoolKind,
        trading_date: IsoDate,
        limit: PositiveU32,
    ) -> Result<Self, crate::CoreError> {
        if limit.get() > 1_000 {
            return Err(crate::CoreError::InvalidRequest(
                "limit-pool limit must be at most 1000".into(),
            ));
        }
        Ok(Self {
            kind,
            trading_date,
            limit,
        })
    }

    pub fn kind(&self) -> LimitPoolKind {
        self.kind
    }

    pub fn trading_date(&self) -> &IsoDate {
        &self.trading_date
    }

    pub fn limit(&self) -> PositiveU32 {
        self.limit
    }
}

#[derive(Deserialize)]
struct LimitPoolRequestWire {
    kind: LimitPoolKind,
    trading_date: IsoDate,
    limit: PositiveU32,
}

impl<'de> Deserialize<'de> for LimitPoolRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = LimitPoolRequestWire::deserialize(deserializer)?;
        Self::new(wire.kind, wire.trading_date, wire.limit).map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LimitPoolCapabilities {
    pub upper: bool,
    pub broken: bool,
    pub lower: bool,
    pub previous_upper: bool,
    pub reasons: bool,
}

pub trait LimitPools {
    type Error: std::error::Error + Send + Sync + 'static;
    fn limit_pool(
        &self,
        request: &LimitPoolRequest,
    ) -> Result<DataBatch<LimitPoolEntry>, Self::Error>;
}
