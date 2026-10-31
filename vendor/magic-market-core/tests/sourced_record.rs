use magic_market_core::{
    Announcement, AuctionSnapshot, Bar, BlockTrade, BoardDefinition, BoardFlow, BoardMembership,
    ConceptHit, ConsensusSnapshot, DividendPlan, DragonTigerEntry, DragonTigerSeat,
    FinancialStatement, FundFlowPoint, HolderCount, InvestorQuestion, LimitPoolEntry, LockupEvent,
    MarginBalance, MarketRankingEntry, MarketStatistics, MinutePoint, MoneyFlow, NewsItem,
    NorthboundDailyStat, OptionContract, OptionGreeks, OptionQuote, OrderBook, PopularityRank,
    PostCloseFlow, ProviderId, Quote, ResearchReport, SecurityMetadata, SecurityProfile,
    SemanticSearchDocument, SourcedRecord, StrongStockReason, TechnicalBar, Trade,
};

fn assert_sourced<T: SourcedRecord>() {}

#[test]
fn every_normalized_record_exposes_common_evidence() {
    assert_sourced::<Quote>();
    assert_sourced::<Bar>();
    assert_sourced::<MinutePoint>();
    assert_sourced::<Trade>();
    assert_sourced::<MoneyFlow>();
    assert_sourced::<OrderBook>();
    assert_sourced::<AuctionSnapshot>();
    assert_sourced::<SecurityMetadata>();
    assert_sourced::<MarketStatistics>();
    assert_sourced::<TechnicalBar>();
    assert_sourced::<ResearchReport>();
    assert_sourced::<ConsensusSnapshot>();
    assert_sourced::<SemanticSearchDocument>();
    assert_sourced::<BoardDefinition>();
    assert_sourced::<BoardMembership>();
    assert_sourced::<StrongStockReason>();
    assert_sourced::<DragonTigerEntry>();
    assert_sourced::<DragonTigerSeat>();
    assert_sourced::<MarketRankingEntry>();
    assert_sourced::<PopularityRank>();
    assert_sourced::<ConceptHit>();
    assert_sourced::<FundFlowPoint>();
    assert_sourced::<BoardFlow>();
    assert_sourced::<MarginBalance>();
    assert_sourced::<BlockTrade>();
    assert_sourced::<HolderCount>();
    assert_sourced::<LockupEvent>();
    assert_sourced::<DividendPlan>();
    assert_sourced::<PostCloseFlow>();
    assert_sourced::<NorthboundDailyStat>();
    assert_sourced::<NewsItem>();
    assert_sourced::<Announcement>();
    assert_sourced::<InvestorQuestion>();
    assert_sourced::<SecurityProfile>();
    assert_sourced::<FinancialStatement>();
    assert_sourced::<LimitPoolEntry>();
    assert_sourced::<OptionContract>();
    assert_sourced::<OptionQuote>();
    assert_sourced::<OptionGreeks>();

    let provider: fn(&Quote) -> ProviderId = SourcedRecord::provider_id;
    let batch: fn(&Quote) -> &str = SourcedRecord::evidence_batch_id;
    let _ = (provider, batch);
}
