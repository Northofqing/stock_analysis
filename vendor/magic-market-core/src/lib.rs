#![forbid(unsafe_code)]
//! Provider-neutral market-data contracts.
mod batch;
mod calendar;
mod capital;
mod company;
mod conformance;
mod content;
mod discovery;
mod enrichment;
mod error;
mod evidence;
mod filings;
mod global;
mod instrument;
mod lifecycle;
mod limit_pool;
mod macro_data;
mod market_announcements;
mod options;
mod policy;
mod probe;
mod provenance;
mod provider;
mod provider_top_n_rankings;
mod reference_data;
mod research;
mod signals;
mod time;
mod validated;
mod value;
pub use batch::{DataBatch, QualityReport};
pub use calendar::{
    CalendarCapabilities, EconomicCalendarProvider, EconomicCalendarRequest, EconomicEvent,
    FuturesDeliveryCalendar, FuturesDeliveryEvent, FuturesDeliveryMethod, FuturesDeliveryRequest,
    FuturesProduct,
};
pub use capital::{
    BlockTrade, BlockTrades, BoardFlow, BoardFlows, CapitalCapabilities, DividendPlan,
    DividendPlans, FlowInterval, FlowScope, FundFlowPoint, FundFlowRequest, FundFlowSeries,
    HolderCount, HolderCounts, InstrumentDateRangeRequest, LockupEvent, LockupEvents,
    MarginBalance, MarginData, NorthboundChannel, NorthboundDailyRequest, NorthboundDailyStat,
    NorthboundDailyStatistics, NorthboundQuotaBalance, NorthboundTopTurnover, PostCloseFlow,
    PostCloseFlowRequest, PostCloseFlows,
};
pub use company::{
    CompanyCapabilities, FinancialLine, FinancialStatement, FinancialStatements, ProfileFact,
    SecurityProfile, SecurityProfiles, StatementKind,
};
pub use conformance::{verify_auction_conformance, AuctionConformancePolicy};
pub use content::{
    Announcement, Announcements, ContentCapabilities, InvestorQuestion, InvestorQuestions,
    NewsItem, NewsProvider,
};
pub use discovery::{
    BoardConstituentProvider, BoardConstituentRequest, BoardDefinition, BoardDirectoryProvider,
    BoardDirectoryRequest, DragonTigerDiscovery, DragonTigerDiscoveryRequest,
    MarketDiscoveryCapabilities,
};
pub use enrichment::{
    MarketStatistics, MarketStatisticsProvider, TechnicalBar, TechnicalBarsProvider,
};
pub use error::CoreError;
pub use evidence::SourceEvidence;
pub use filings::{
    CompanyFiling, CompanyFilingRequest, CompanyFilingsProvider, FilingCapabilities,
    SecAccessionNumber, SecCompanyIdentity, SecPrimaryDocument,
};
pub use global::{
    ForeignExchangeProvider, FxPair, FxQuote, FxRequest, GlobalIndexCode, GlobalIndexProvider,
    GlobalIndexQuote, GlobalIndexRequest, GlobalMarketCapabilities,
};
pub use instrument::{AssetClass, Exchange, InstrumentId};
pub use lifecycle::{
    CorporateAction, CorporateActionCategory, CorporateActionRequest, CorporateActionResponse,
    CorporateActionStatus, CorporateActionTerms, CorporateActions,
    CorporateActions as CorporateActionsProvider, UnverifiedSourceUnit,
};
pub use limit_pool::{
    LimitPoolCapabilities, LimitPoolEntry, LimitPoolKind, LimitPoolRequest, LimitPools,
};
pub use macro_data::{
    EconomicDataCapabilities, EconomicFrequency, EconomicObservation, EconomicObservationStatus,
    EconomicPeriod, EconomicRevision, EconomicRevisionKind, EconomicSeriesKey,
    EconomicSeriesProvider, EconomicSeriesRequest,
};
pub use market_announcements::{MarketAnnouncementRequest, MarketAnnouncements};
pub use options::{
    ContractMonth, OptionCapabilities, OptionContract, OptionData, OptionGreeks, OptionKind,
    OptionQuote,
};
pub use policy::{PolicyCapabilities, PolicyDocument, PolicyDocuments, PolicyRequest};
pub use probe::{
    verify_admitted_batch, verify_serial_load, verify_verified_empty, EvidenceTimestamp,
    LoadProbeError, LoadProbeSnapshot, ProbeAdmissionError, ProbeAdmissionPolicy,
    ProbeRequestTracker, ProbeStatus, VerifiedEmpty,
};
pub use provenance::Provenance;
pub use provider::{
    Adjustment, AsyncHistoricalBars, AsyncMinuteData, AsyncRealtimeQuotes, AsyncTrades,
    AuctionSnapshot, Auctions, Bar, BarInterval, BarsRequest, Board, BookLevel, Capabilities,
    DataStatus, HistoricalBars, MinuteData, MinuteDataRequest, MinutePoint, MoneyFlow, MoneyFlows,
    OrderBook, OrderBooks, PriceLimitRule, ProviderId, Quote, RealtimeQuotes, SecurityMetadata,
    SecurityMetadataProvider, SourcedRecord, Trade, TradeSide, Trades, TradesRequest,
};
pub use provider_top_n_rankings::{
    validate_provider_top_n_ranking_batch, ProviderTopNRankingCapabilities,
    ProviderTopNRankingEntry, ProviderTopNRankingRequest, ProviderTopNRankings,
};
pub use reference_data::{
    CurrencyCode, OfficialFxFixing, OfficialFxFixingIdentity, OfficialFxFixingProvider,
    OfficialFxFixingRequest, ReferenceDataCapabilities, ReferenceRateIdentity, ReferenceRateKind,
    ReferenceRateObservation, ReferenceRateProvider, ReferenceRateRequest, ReferenceTenor,
};
pub use research::{
    ConsensusData, ConsensusSnapshot, EarningsEstimate, ReportScope, ResearchCapabilities,
    ResearchDocument, ResearchDocumentRequest, ResearchDocuments, ResearchReport, ResearchReports,
    ResearchRequest, SemanticChannel, SemanticSearch, SemanticSearchDocument,
    SemanticSearchRequest, TargetPriceConsensus, TargetPriceData, TargetPriceObservation,
    TargetPriceRequest,
};
pub use signals::{
    validate_market_ranking_batch, BoardCategory, BoardMembership, BoardMembershipProvider,
    ConceptHit, ConceptHits, DragonTigerData, DragonTigerDisclosure, DragonTigerEntry,
    DragonTigerSeat, DragonTigerSide, InstrumentSignalRequest, MarketBreadth, MarketBreadthRequest,
    MarketBreadthSnapshot, MarketDragonTigerData, MarketDragonTigerRequest,
    MarketRankingCapabilities, MarketRankingEntry, MarketRankingKind, MarketRankingUnit,
    MarketRankings, MarketSession, PopularityData, PopularityRank, SignalCapabilities,
    StrongStockReason, StrongStockReasons,
};
pub use time::{unix_seconds_to_china_rfc3339, unix_seconds_to_fixed_offset_rfc3339, ClockTime};
pub use validated::{FiniteNumber, HttpsUrl, IsoDate, NonEmptyText, PositiveU32};
pub use value::{Money, NumericTolerance, Price, Quantity, Ratio, RatioUnit};
