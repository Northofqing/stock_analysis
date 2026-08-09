//! BR-164 evidence-preserving production financial/news data gateway.

pub mod board;
pub mod board_ranking;
mod board_runtime;
pub mod capital;
pub mod chain_intelligence;
pub mod company;
pub mod consensus;
pub mod dragon_tiger;
pub mod economic_calendar;
pub mod event_calendar;
pub mod evidence_time;
pub mod exchange_calendar_authority;
pub mod futures_delivery;
pub mod general_web_research;
pub mod global_market;
pub mod global_news;
pub mod historical_bars;
pub mod index;
pub mod instrument_identity;
pub mod intraday_shape;
pub mod magic_tdx;
pub mod magic_tdx_selection;
pub mod magic_tdx_t0;
pub mod market_capabilities;
pub mod block_trade;
pub mod market_data;
pub mod outcome_daily_bars;
pub mod position_chain;
pub mod research;
pub mod review;
pub mod security_lifecycle;
pub mod sina_instrument_news;

pub use board::{
    load_verified_board_artifact_default, BoardBindingRegistry, BoardDataGateway,
    BoardDirectoryFact, BoardDirectoryRecordEvidence, BoardFlowFact, BoardKind,
    BoardMembershipRecord, BoardSelectionError, SelectionBoardConfiguration,
    ValidatedSelectionBoardBatch, VerifiedBoardArtifact, VerifiedBoardBinding,
    BOARD_CONSTITUENT_REQUEST_LIMIT,
};
pub use capital::{
    CapitalDataGateway, InstrumentFundFlowFact, NorthboundDailyFact, NorthboundQuotaFact,
    NorthboundTopTurnoverFact, ProviderTopNFact, ProviderTopNPair, ProviderTopNRequestEvidence,
};
pub use chain_intelligence::{
    build_chain_intelligence_batch, BoardMembershipFact, ChainIntelligenceGateway,
    ChainIntelligencePolicy, ChainSourceEvidence, ChainSourceRejection, UpperLimitFact,
};
pub use company::CompanyDataGateway;
pub use consensus::ConsensusDataGateway;
pub use dragon_tiger::{
    DragonTigerGateway, DragonTigerSeatReview, DragonTigerSourceDisclosure, DragonTigerStockReview,
};
pub use economic_calendar::{EconomicCalendarGateway, EconomicReleaseFact};
pub use event_calendar::{EventAnnouncement, EventCalendarGateway};
pub use evidence_time::parse_evidence_instant;
pub use exchange_calendar_authority::{
    validate_canonical_sse_announcement_url, validate_official_exchange_notice_url,
    OfficialAshareExchange, OfficialExchangeUrlError, OFFICIAL_SSE_AUTHORITY_ROOT,
};
pub use futures_delivery::{
    cffex_futures_delivery_live_supported, FuturesDeliveryFact, FuturesDeliveryGateway,
};
pub use general_web_research::{
    GeneralWebResearchBatch, GeneralWebResearchBatchEvidence, GeneralWebResearchError,
    GeneralWebResearchGateway, GeneralWebResearchProvider, GeneralWebResearchRecord,
    GeneralWebResearchRecordEvidence, GeneralWebResearchStage, PublicationTimeQuality,
    ResearchUseScope,
};
pub use global_market::{ForeignExchangeFact, GlobalIndexFact, GlobalMarketGateway};
pub use global_news::{GlobalNewsGateway, GlobalNewsProvider, GlobalNewsRecord};
pub use historical_bars::{daily_bar_provider_label, AdmittedDailyBars, HistoricalBarsGateway};
pub use index::{IndexDataGateway, RealtimeIndexQuote};
pub use intraday_shape::{IntradayShapeFact, IntradayShapeGateway};
pub use magic_tdx::MagicTdxGateway;
pub use magic_tdx_t0::{
    MagicTdxT0Batch, MagicTdxT0DailyBar, MagicTdxT0Evidence, MagicTdxT0FiveMinuteBar,
    MagicTdxT0Quote, MagicTdxT0Rejection, T0BookLevel,
};
pub use market_capabilities::{
    MarketBookLevel, MarketCapabilitiesGateway, MarketMinutePoint, MarketMoneyFlow,
    MarketOrderBook, MarketSecurityMetadata, SecurityBoard, METADATA_PROVIDER_ORDER,
    MINUTE_PROVIDER_ORDER, MONEY_FLOW_PROVIDER_ORDER, ORDER_BOOK_PROVIDER_ORDER,
};
pub use block_trade::{BlockTradeReview, BlockTradesGateway};
pub use market_data::{MarketDataGateway, RealtimeMarketQuote};
pub use outcome_daily_bars::{AdmittedOutcomeDailyBars, OutcomeDailyBarsGateway};
pub use position_chain::{
    acquire_candidate_position_chain, derive_position_chain, refresh_position_chains,
    CanonicalPositionMembership, PositionChainAssignment, PositionChainRefreshOutcome,
    PositionChainRefreshReport, PositionChainRefreshStatus,
};
pub use research::{ResearchDataGateway, ResearchReportFact};
pub use review::{
    BatchEvidence, DailyClose, GatewayBatch, GatewayError, ReviewDataGateway, UpperLimitRecord,
};
pub use security_lifecycle::{
    AdmittedListingDate, CorporateActionState, ImplementedCorporateAction,
    LifecycleConfirmationEvidence, ListingDateState, SecurityLifecycleContext,
    SecurityLifecycleGateway,
};
pub use sina_instrument_news::{SinaInstrumentNewsGateway, SinaInstrumentNewsRecord};
