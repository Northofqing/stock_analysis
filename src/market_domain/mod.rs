pub mod bars;
pub mod evidence;
pub mod instrument;
pub mod lifecycle;
pub mod market;
pub mod provider_id;
pub mod ranking;
pub mod record;
pub mod tdx;
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
