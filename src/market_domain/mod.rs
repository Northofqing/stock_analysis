#[cfg(feature = "magic-gateway")]
pub mod bars { pub use magic_market_core::{Adjustment, Bar, BarInterval}; }
#[cfg(feature = "magic-gateway")]
pub mod evidence { pub use magic_market_core::{EvidenceTimestamp, NonEmptyText, SourceEvidence}; }
#[cfg(feature = "magic-gateway")]
pub mod instrument { pub use magic_market_core::{AssetClass, CoreError, Exchange, InstrumentId}; }
#[cfg(feature = "magic-gateway")]
pub mod lifecycle { pub use magic_market_core::{CorporateActionCategory, CorporateActionStatus, CorporateActionTerms, UnverifiedSourceUnit}; }
#[cfg(feature = "magic-gateway")]
pub mod market { pub use magic_market_core::{FinancialLine, FinancialStatement, LimitPoolEntry, LimitPoolKind, MarketStatistics, StatementKind}; }
#[cfg(feature = "magic-gateway")]
pub mod provider_id { pub use magic_market_core::ProviderId; }
#[cfg(feature = "magic-gateway")]
pub mod ranking { pub use magic_market_core::{DragonTigerSide, FxPair, GlobalIndexCode, MarketRankingKind, MarketRankingUnit}; }
#[cfg(feature = "magic-gateway")]
pub mod record { pub use magic_market_core::{DataBatch, FlowInterval, IsoDate, NorthboundChannel, Provenance, QualityReport}; }
#[cfg(feature = "magic-gateway")]
pub mod tdx { pub use magic_tdx_rs::protocol::types::SecurityBar; }
#[cfg(feature = "magic-gateway")]
pub mod value { pub use magic_market_core::{FiniteNumber, Money, PositiveU32, Price, Quantity, Ratio, RatioUnit}; }

#[cfg(not(feature = "magic-gateway"))]
pub mod bars;
#[cfg(not(feature = "magic-gateway"))]
pub mod evidence;
#[cfg(not(feature = "magic-gateway"))]
pub mod instrument;
#[cfg(not(feature = "magic-gateway"))]
pub mod lifecycle;
#[cfg(not(feature = "magic-gateway"))]
pub mod market;
#[cfg(not(feature = "magic-gateway"))]
pub mod provider_id;
#[cfg(not(feature = "magic-gateway"))]
pub mod ranking;
#[cfg(not(feature = "magic-gateway"))]
pub mod record;
#[cfg(not(feature = "magic-gateway"))]
pub mod tdx;
#[cfg(not(feature = "magic-gateway"))]
pub mod value;

pub use bars::{Adjustment, Bar, BarInterval};
pub use evidence::{EvidenceTimestamp, NonEmptyText, SourceEvidence};
pub use instrument::{AssetClass, CoreError, Exchange, InstrumentId};
pub use lifecycle::{
    CorporateActionCategory, CorporateActionStatus, CorporateActionTerms, UnverifiedSourceUnit,
};
pub use market::{
    FinancialLine, FinancialStatement, LimitPoolEntry, LimitPoolKind, MarketStatistics,
    StatementKind,
};
pub use provider_id::ProviderId;
pub use ranking::{DragonTigerSide, FxPair, GlobalIndexCode, MarketRankingKind, MarketRankingUnit};
pub use record::{DataBatch, FlowInterval, IsoDate, NorthboundChannel, Provenance, QualityReport};
pub use tdx::SecurityBar;
pub use value::{FiniteNumber, Money, PositiveU32, Price, Quantity, Ratio, RatioUnit};

#[cfg(test)]
mod tests {
    use super::ProviderId;

    #[test]
    fn provider_id_wire_names_are_stable() {
        let cases = [
            ("Tdx", ProviderId::Tdx),
            ("Tencent", ProviderId::Tencent),
            ("Eastmoney", ProviderId::Eastmoney),
            ("Sina", ProviderId::Sina),
            ("Custom", ProviderId::Custom),
        ];
        for (expected, provider) in cases {
            assert_eq!(format!("{provider:?}"), expected);
            assert_eq!(
                serde_json::to_string(&provider).unwrap(),
                format!("\"{expected}\"")
            );
        }
    }
}
