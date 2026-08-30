//! Fixture-only MarketDataService handler for client integration tests.

use super::data;
use stock_analysis::grpc_client::pb::magic::market::v1::{
    market_data_service_server::MarketDataService, Operation, QueryRequest, QueryResponse,
};
use stock_analysis::grpc_contract::schema::schema_for;
use tonic::{Request, Response, Status};

pub use stock_analysis::grpc_client::pb::magic::market::v1::market_data_service_server;

pub struct DataService;

impl DataService {
    async fn serve_query(
        &self,
        operation: Operation,
        request: QueryRequest,
    ) -> Result<Response<QueryResponse>, Status> {
        let payload = request
            .payload
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("QueryRequest missing payload"))?;
        let frozen = schema_for(operation).ok_or_else(|| {
            Status::unimplemented(format!(
                "{} is not implemented by the fixture",
                stock_analysis::grpc_contract::ops::method_name(operation)
            ))
        })?;
        if payload.schema != frozen.schema_name {
            return Err(Status::invalid_argument(format!(
                "schema mismatch: expected {}, got {}",
                frozen.schema_name, payload.schema
            )));
        }
        if payload.schema_version != frozen.schema_version {
            return Err(Status::invalid_argument(format!(
                "unsupported schema version: expected {}, got {}",
                frozen.schema_version, payload.schema_version
            )));
        }

        let mut response =
            data::fixture_response(operation, frozen.schema_name, frozen.schema_version)
                .ok_or_else(|| {
                    Status::unimplemented(format!("{} has no fixture", frozen.schema_name))
                })?;
        response.request_id = request
            .context
            .as_ref()
            .map(|context| context.request_id.clone())
            .unwrap_or_default();
        Ok(Response::new(response))
    }
}

macro_rules! impl_market_data_service {
    ($( $method:ident => $operation:ident ),* $(,)?) => {
        #[tonic::async_trait]
        impl MarketDataService for DataService {
            $(
                async fn $method(
                    &self,
                    request: Request<QueryRequest>,
                ) -> Result<Response<QueryResponse>, Status> {
                    self.serve_query(Operation::$operation, request.into_inner()).await
                }
            )*
        }
    };
}

impl_market_data_service!(
    historical_bars => HistoricalBars,
    minute_data => MinuteData,
    realtime_quotes => RealtimeQuotes,
    money_flows => MoneyFlows,
    order_books => OrderBooks,
    auctions => Auctions,
    trades => Trades,
    security_metadata => SecurityMetadata,
    global_indices => GlobalIndices,
    foreign_exchange => ForeignExchange,
    economic_calendar => EconomicCalendar,
    futures_delivery => FuturesDelivery,
    reference_rates => ReferenceRates,
    official_fx_fixings => OfficialFxFixings,
    economic_series => EconomicSeries,
    company_filings => CompanyFilings,
    global_news => GlobalNews,
    announcements => Announcements,
    market_announcements => MarketAnnouncements,
    investor_questions => InvestorQuestions,
    policy_documents => PolicyDocuments,
    security_profiles => SecurityProfiles,
    financial_statements => FinancialStatements,
    market_statistics => MarketStatistics,
    technical_bars => TechnicalBars,
    corporate_actions => CorporateActions,
    board_directory => BoardDirectory,
    board_constituents => BoardConstituents,
    board_memberships => BoardMemberships,
    research_reports => ResearchReports,
    research_documents => ResearchDocuments,
    consensus => Consensus,
    target_prices => TargetPrices,
    semantic_search => SemanticSearch,
    fund_flow_series => FundFlowSeries,
    board_flows => BoardFlows,
    margin_data => MarginData,
    block_trades => BlockTrades,
    holder_counts => HolderCounts,
    lockup_events => LockupEvents,
    dividend_plans => DividendPlans,
    post_close_flows => PostCloseFlows,
    northbound_daily => NorthboundDaily,
    limit_pools => LimitPools,
    strong_stock_reasons => StrongStockReasons,
    dragon_tiger => DragonTiger,
    market_dragon_tiger => MarketDragonTiger,
    dragon_tiger_discovery => DragonTigerDiscovery,
    market_rankings => MarketRankings,
    market_breadth => MarketBreadth,
    popularity => Popularity,
    concept_hits => ConceptHits,
    option_data => OptionData,
    provider_top_n_rankings => ProviderTopNRankings,
    index_quotes => IndexQuotes,
    instrument_news => InstrumentNews,
    intraday_shape => IntradayShape,
    t0_evidence => T0Evidence,
    outcome_daily_bars => OutcomeDailyBars,
    upper_limit_pool_review => UpperLimitPoolReview,
    chain_batch => ChainBatch,
    benchmark_bars => BenchmarkBars,
);
