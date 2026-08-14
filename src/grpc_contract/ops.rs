//! Operation ↔ proto 方法名映射 + 已实现集合。
//! 54 个 op 全部列出 (合同 market.proto 冻结); 生产未用到的 op 不进 implemented。
use crate::grpc_client::pb::magic::market::v1::Operation;

/// proto 方法名 (MarketDataService 的 RPC 名, 与 market.proto 一一对应)。
pub fn method_name(op: Operation) -> &'static str {
    use Operation::*;
    match op {
        HistoricalBars => "HistoricalBars",
        MinuteData => "MinuteData",
        RealtimeQuotes => "RealtimeQuotes",
        MoneyFlows => "MoneyFlows",
        OrderBooks => "OrderBooks",
        Auctions => "Auctions",
        Trades => "Trades",
        SecurityMetadata => "SecurityMetadata",
        GlobalIndices => "GlobalIndices",
        ForeignExchange => "ForeignExchange",
        EconomicCalendar => "EconomicCalendar",
        FuturesDelivery => "FuturesDelivery",
        ReferenceRates => "ReferenceRates",
        OfficialFxFixings => "OfficialFxFixings",
        EconomicSeries => "EconomicSeries",
        CompanyFilings => "CompanyFilings",
        GlobalNews => "GlobalNews",
        Announcements => "Announcements",
        MarketAnnouncements => "MarketAnnouncements",
        InvestorQuestions => "InvestorQuestions",
        PolicyDocuments => "PolicyDocuments",
        SecurityProfiles => "SecurityProfiles",
        FinancialStatements => "FinancialStatements",
        MarketStatistics => "MarketStatistics",
        TechnicalBars => "TechnicalBars",
        CorporateActions => "CorporateActions",
        BoardDirectory => "BoardDirectory",
        BoardConstituents => "BoardConstituents",
        BoardMemberships => "BoardMemberships",
        ResearchReports => "ResearchReports",
        ResearchDocuments => "ResearchDocuments",
        Consensus => "Consensus",
        TargetPrices => "TargetPrices",
        SemanticSearch => "SemanticSearch",
        FundFlowSeries => "FundFlowSeries",
        BoardFlows => "BoardFlows",
        MarginData => "MarginData",
        BlockTrades => "BlockTrades",
        HolderCounts => "HolderCounts",
        LockupEvents => "LockupEvents",
        DividendPlans => "DividendPlans",
        PostCloseFlows => "PostCloseFlows",
        NorthboundDaily => "NorthboundDaily",
        LimitPools => "LimitPools",
        StrongStockReasons => "StrongStockReasons",
        DragonTiger => "DragonTiger",
        MarketDragonTiger => "MarketDragonTiger",
        DragonTigerDiscovery => "DragonTigerDiscovery",
        MarketRankings => "MarketRankings",
        MarketBreadth => "MarketBreadth",
        Popularity => "Popularity",
        ConceptHits => "ConceptHits",
        OptionData => "OptionData",
        ProviderTopNRankings => "ProviderTopNRankings",
        Unspecified => "OPERATION_UNSPECIFIED",
    }
}

/// 生产实际用到的 24 个 op (spec §4.2 清单, P2 冻结)。
pub fn implemented_operations() -> Vec<Operation> {
    use Operation::*;
    vec![
        RealtimeQuotes, HistoricalBars, MinuteData, OrderBooks, MoneyFlows,
        SecurityMetadata, Announcements, GlobalNews, EconomicCalendar,
        FuturesDelivery, GlobalIndices, BoardDirectory, BoardConstituents,
        BoardFlows, LimitPools, StrongStockReasons, DragonTiger,
        MarketDragonTiger, MarketRankings, ConceptHits, Consensus,
        ResearchReports, BlockTrades, NorthboundDaily,
    ]
}

pub fn is_implemented(op: Operation) -> bool {
    implemented_operations().contains(&op)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grpc_client::pb::magic::market::v1::Operation;

    #[test]
    fn method_name_covers_all_54_operations() {
        // 从 proto 的 Operation 枚举全量遍历 (0..=54), 每个都映射到非空方法名。
        // prost 0.14 标记 from_i32 deprecated → 用 TryFrom<i32> (语义等价)。
        for value in 0..=54 {
            if let Ok(op) = Operation::try_from(value) {
                assert!(!method_name(op).is_empty(), "op {value} 缺少方法名映射");
            }
        }
    }

    #[test]
    fn implemented_set_is_24_and_within_54() {
        assert_eq!(implemented_operations().len(), 24);
        assert!(implemented_operations()
            .iter()
            .all(|op| !matches!(op, Operation::Unspecified)));
        assert!(implemented_operations()
            .iter()
            .all(|op| !method_name(*op).is_empty()));
    }

    #[test]
    fn realtime_quotes_is_implemented() {
        assert!(is_implemented(Operation::RealtimeQuotes));
        assert!(!is_implemented(Operation::OptionData));
    }
}
