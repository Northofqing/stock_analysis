//! Locally owned ranking, global-index, and FX-pair types.
//! Variant and serde representations are stable transport contracts.

use serde::{Deserialize, Serialize};

use super::evidence::NonEmptyText;

/// Ranking metric identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MarketRankingKind {
    VolumeRatio,
    MainNetInflow,
    Industry,
    Concept,
    Region,
    Popularity,
    Custom(NonEmptyText),
}

/// Unit carried by a ranking metric.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MarketRankingUnit {
    /// Turnover volume divided by the comparable recent average.
    Multiple,
    /// Chinese yuan.
    Yuan,
    /// Percentage points.
    Percent,
    /// Source-specific dimensionless score.
    Score,
    /// Explicit unit for a custom metric.
    Custom(NonEmptyText),
}

/// Side of a dragon-tiger (LHB) disclosure seat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DragonTigerSide {
    Buy,
    Sell,
}

/// Global equity-index identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GlobalIndexCode {
    DowJones,
    NasdaqComposite,
    Sp500,
    Nikkei225,
    HangSeng,
    Ftse100,
}

/// Foreign-exchange currency-pair identity.
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
