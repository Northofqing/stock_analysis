//! Locally owned limit-pool, financial-statement, and market-statistics types.
//! Field and serde representations are stable transport contracts.

use serde::{de, Deserialize, Deserializer, Serialize};

use super::evidence::{NonEmptyText, SourceEvidence};
use super::instrument::InstrumentId;
use super::record::IsoDate;
use super::value::{FiniteNumber, Money, PositiveU32, Price, Quantity, Ratio};

/// Kind of price-limit pool (board) membership.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LimitPoolKind {
    Upper,
    Broken,
    Lower,
    PreviousUpper,
}

/// A single record from a price-limit pool.
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

/// Financial-statement family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StatementKind {
    Balance,
    Income,
    CashFlow,
}

/// A single line of a financial statement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FinancialLine {
    pub key: NonEmptyText,
    pub source_label: NonEmptyText,
    pub value: Option<FiniteNumber>,
    pub unit: Option<NonEmptyText>,
}

/// One reported financial statement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FinancialStatement {
    pub instrument: InstrumentId,
    pub kind: StatementKind,
    pub report_period: IsoDate,
    pub announced_on: Option<IsoDate>,
    pub currency: Option<NonEmptyText>,
    pub lines: Vec<FinancialLine>,
    pub evidence: SourceEvidence,
}

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
    ) -> Result<Self, super::instrument::CoreError> {
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

fn ensure_nonnegative(
    field: &'static str,
    value: Option<Money>,
) -> Result<(), super::instrument::CoreError> {
    if value.is_some_and(|number| number.get() < 0.0) {
        return Err(super::instrument::CoreError::InvalidValue {
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
