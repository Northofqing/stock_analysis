//! MarketRankingKind / MarketRankingUnit / GlobalIndexCode / FxPair
//! 本地镜像 (M5, Task #76, feature 关时使用)。
//! 与上游 magic_market_core rev 75ee2a2 (signals.rs + global.rs) 同构:
//! 变体集/serde 表示一致 (wire 是 JSON, convert.rs 依赖)。

#[cfg(not(feature = "magic-gateway"))]
use serde::{Deserialize, Serialize};

#[cfg(not(feature = "magic-gateway"))]
use super::evidence::NonEmptyText;

/// Ranking metric identity.
#[cfg(not(feature = "magic-gateway"))]
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
#[cfg(not(feature = "magic-gateway"))]
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

/// Global equity-index identity.
#[cfg(not(feature = "magic-gateway"))]
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
#[cfg(not(feature = "magic-gateway"))]
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
