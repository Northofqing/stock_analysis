use crate::{
    Bar, BarsRequest, DataBatch, FiniteNumber, InstrumentId, Money, Price, Ratio, SourceEvidence,
    SourcedRecord,
};
use serde::{de, Deserialize, Deserializer, Serialize};

/// Valuation, capitalization and trading statistics adjacent to a quote.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MarketStatistics {
    instrument: InstrumentId,
    turnover_rate: Option<Ratio>,
    trailing_pe: Option<FiniteNumber>,
    static_pe: Option<FiniteNumber>,
    pb: Option<FiniteNumber>,
    total_market_cap: Option<Money>,
    floating_market_cap: Option<Money>,
    upper_limit: Option<Price>,
    lower_limit: Option<Price>,
    volume_ratio: Option<FiniteNumber>,
    evidence: SourceEvidence,
}

impl MarketStatistics {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        instrument: InstrumentId,
        turnover_rate: Option<Ratio>,
        trailing_pe: Option<FiniteNumber>,
        static_pe: Option<FiniteNumber>,
        pb: Option<FiniteNumber>,
        total_market_cap: Option<Money>,
        floating_market_cap: Option<Money>,
        upper_limit: Option<Price>,
        lower_limit: Option<Price>,
        volume_ratio: Option<FiniteNumber>,
        evidence: SourceEvidence,
    ) -> Result<Self, crate::CoreError> {
        ensure_nonnegative("total_market_cap", total_market_cap)?;
        ensure_nonnegative("floating_market_cap", floating_market_cap)?;
        Ok(Self {
            instrument,
            turnover_rate,
            trailing_pe,
            static_pe,
            pb,
            total_market_cap,
            floating_market_cap,
            upper_limit,
            lower_limit,
            volume_ratio,
            evidence,
        })
    }

    pub fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }

    pub fn turnover_rate(&self) -> Option<Ratio> {
        self.turnover_rate
    }

    pub fn trailing_pe(&self) -> Option<FiniteNumber> {
        self.trailing_pe
    }

    pub fn static_pe(&self) -> Option<FiniteNumber> {
        self.static_pe
    }

    pub fn pb(&self) -> Option<FiniteNumber> {
        self.pb
    }

    pub fn total_market_cap(&self) -> Option<Money> {
        self.total_market_cap
    }

    pub fn floating_market_cap(&self) -> Option<Money> {
        self.floating_market_cap
    }

    pub fn upper_limit(&self) -> Option<Price> {
        self.upper_limit
    }

    pub fn lower_limit(&self) -> Option<Price> {
        self.lower_limit
    }

    pub fn volume_ratio(&self) -> Option<FiniteNumber> {
        self.volume_ratio
    }

    pub fn evidence(&self) -> &SourceEvidence {
        &self.evidence
    }
}

fn ensure_nonnegative(field: &'static str, value: Option<Money>) -> Result<(), crate::CoreError> {
    if value.is_some_and(|number| number.get() < 0.0) {
        return Err(crate::CoreError::InvalidValue {
            field,
            value: value
                .map(|number| number.get().to_string())
                .unwrap_or_default(),
            reason: "must be non-negative",
        });
    }
    Ok(())
}

#[derive(Deserialize)]
struct MarketStatisticsWire {
    instrument: InstrumentId,
    turnover_rate: Option<Ratio>,
    trailing_pe: Option<FiniteNumber>,
    static_pe: Option<FiniteNumber>,
    pb: Option<FiniteNumber>,
    total_market_cap: Option<Money>,
    floating_market_cap: Option<Money>,
    upper_limit: Option<Price>,
    lower_limit: Option<Price>,
    volume_ratio: Option<FiniteNumber>,
    evidence: SourceEvidence,
}

impl<'de> Deserialize<'de> for MarketStatistics {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = MarketStatisticsWire::deserialize(deserializer)?;
        Self::new(
            value.instrument,
            value.turnover_rate,
            value.trailing_pe,
            value.static_pe,
            value.pb,
            value.total_market_cap,
            value.floating_market_cap,
            value.upper_limit,
            value.lower_limit,
            value.volume_ratio,
            value.evidence,
        )
        .map_err(de::Error::custom)
    }
}

impl SourcedRecord for MarketStatistics {
    fn provider_id(&self) -> crate::ProviderId {
        self.evidence.provider()
    }

    fn evidence_batch_id(&self) -> &str {
        self.evidence.batch_id()
    }
}

/// Bar plus moving averages explicitly supplied by the same source response.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TechnicalBar {
    bar: Bar,
    ma5: Option<Price>,
    ma10: Option<Price>,
    ma20: Option<Price>,
    evidence: SourceEvidence,
}

impl TechnicalBar {
    pub fn new(
        bar: Bar,
        ma5: Option<Price>,
        ma10: Option<Price>,
        ma20: Option<Price>,
        evidence: SourceEvidence,
    ) -> Result<Self, crate::CoreError> {
        if bar.provider() != evidence.provider() {
            return Err(crate::CoreError::InvalidRequest(
                "technical bar provider does not match source bar".into(),
            ));
        }
        if bar.batch_id() != evidence.batch_id() {
            return Err(crate::CoreError::InvalidRequest(
                "technical bar batch does not match source bar".into(),
            ));
        }
        Ok(Self {
            bar,
            ma5,
            ma10,
            ma20,
            evidence,
        })
    }

    pub fn bar(&self) -> &Bar {
        &self.bar
    }

    pub fn ma5(&self) -> Option<Price> {
        self.ma5
    }

    pub fn ma10(&self) -> Option<Price> {
        self.ma10
    }

    pub fn ma20(&self) -> Option<Price> {
        self.ma20
    }

    pub fn evidence(&self) -> &SourceEvidence {
        &self.evidence
    }
}

#[derive(Deserialize)]
struct TechnicalBarWire {
    bar: Bar,
    ma5: Option<Price>,
    ma10: Option<Price>,
    ma20: Option<Price>,
    evidence: SourceEvidence,
}

impl<'de> Deserialize<'de> for TechnicalBar {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = TechnicalBarWire::deserialize(deserializer)?;
        Self::new(wire.bar, wire.ma5, wire.ma10, wire.ma20, wire.evidence)
            .map_err(de::Error::custom)
    }
}

impl SourcedRecord for TechnicalBar {
    fn provider_id(&self) -> crate::ProviderId {
        self.evidence.provider()
    }

    fn evidence_batch_id(&self) -> &str {
        self.evidence.batch_id()
    }
}

/// Provider capability for quote-adjacent market statistics.
pub trait MarketStatisticsProvider {
    type Error: std::error::Error + Send + Sync + 'static;

    fn market_statistics(
        &self,
        instruments: &[InstrumentId],
    ) -> Result<DataBatch<MarketStatistics>, Self::Error>;
}

/// Provider capability for bars that include source-supplied indicators.
pub trait TechnicalBarsProvider {
    type Error: std::error::Error + Send + Sync + 'static;

    fn technical_bars(&self, request: &BarsRequest)
        -> Result<DataBatch<TechnicalBar>, Self::Error>;
}
