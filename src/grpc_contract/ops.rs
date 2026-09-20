//! Operation ↔ proto 方法名映射 + 已实现集合。
//! 全部 op 列出 (client-bundle/market.proto 上游 0-55 + build.rs 本地扩展 56-62,
//! 合并后共 63 个); 生产未用到的 op 不进 implemented。
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
        IndexQuotes => "IndexQuotes",
        InstrumentNews => "InstrumentNews",
        IntradayShape => "IntradayShape",
        T0Evidence => "T0Evidence",
        OutcomeDailyBars => "OutcomeDailyBars",
        UpperLimitPoolReview => "UpperLimitPoolReview",
        ChainBatch => "ChainBatch",
        BenchmarkBars => "BenchmarkBars",
        Unspecified => "OPERATION_UNSPECIFIED",
    }
}

/// 生产实际用到的 40 个 op（含 BR-251 历史基准入口）。
pub fn implemented_operations() -> Vec<Operation> {
    use Operation::*;
    vec![
        RealtimeQuotes,
        HistoricalBars,
        MinuteData,
        OrderBooks,
        MoneyFlows,
        SecurityMetadata,
        Announcements,
        GlobalNews,
        EconomicCalendar,
        FuturesDelivery,
        GlobalIndices,
        BoardDirectory,
        BoardConstituents,
        BoardFlows,
        LimitPools,
        StrongStockReasons,
        DragonTiger,
        MarketDragonTiger,
        MarketRankings,
        ConceptHits,
        Consensus,
        ResearchReports,
        BlockTrades,
        NorthboundDaily,
        // M1 扩展: 8 个 proto 已有 op 补齐 delegate 实现。
        ForeignExchange,
        FinancialStatements,
        MarketStatistics,
        TechnicalBars,
        CorporateActions,
        SemanticSearch,
        FundFlowSeries,
        ProviderTopNRankings,
        // M1 扩展: InstrumentNews 是上游合同 op (=55), 其余 5 个是本地扩展
        // (56-60, 用户决策 2026-08-16 「保留本地 server 扩展」; 上游直连后
        // 这些 op 由本地 server 继续提供, monitor 桥不感知差异)。
        IndexQuotes,
        InstrumentNews,
        IntradayShape,
        T0Evidence,
        OutcomeDailyBars,
        UpperLimitPoolReview,
        // M4c 扩展: A-10 完整 batch (=61, 本地扩展)。limit_pools/strong_stock_reasons
        // (44/45) 视图不可重建 VisibleChainBatch → monitor 复盘经此 op 消费。
        ChainBatch,
        // BR-251: 指数专用历史基准合同 (=62, 本地扩展)。
        BenchmarkBars,
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
    fn method_name_covers_all_62_operations() {
        // 从 proto 的 Operation 枚举全量遍历 (0..=62), 每个都映射到非空方法名。
        // prost 0.14 标记 from_i32 deprecated → 用 TryFrom<i32> (语义等价)。
        for value in 0..=62 {
            let op =
                Operation::try_from(value).unwrap_or_else(|_| panic!("op {value} 缺少冻结枚举值"));
            assert!(!method_name(op).is_empty(), "op {value} 缺少方法名映射");
        }
    }

    #[test]
    fn implemented_set_is_40_and_within_62() {
        assert_eq!(implemented_operations().len(), 40);
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
