//! MarketDataService handler: 校验请求 → fixture 或 data_gateway 委托 → QueryResponse。
use crate::grpc_client::pb::magic::market::v1::{
    market_data_service_server::MarketDataService, AdmissionState, CanonicalPayload,
    Operation, QueryRequest, QueryResponse,
};
use crate::grpc_contract::schema::schema_for;
use crate::grpc_server::{delegate, fixture, ServerState};
use std::sync::Arc;
use tonic::{Request, Response, Status};

pub use crate::grpc_client::pb::magic::market::v1::market_data_service_server;

pub struct DataService {
    state: Arc<ServerState>,
    fixture_mode: bool,
}

impl DataService {
    pub fn new(state: Arc<ServerState>, fixture_mode: bool) -> Self {
        Self { state, fixture_mode }
    }

    /// 统一查询入口: 校验 → 取数 → 包装 QueryResponse。
    async fn serve_query(&self, op: Operation, req: QueryRequest) -> Result<Response<QueryResponse>, Status> {
        let payload = req
            .payload
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("QueryRequest 缺 payload"))?;
        let request_schema = payload.schema.clone();
        let request_version = payload.schema_version;

        // 合同 §5: 未知 schema/version 必须拒绝。
        let frozen = schema_for(op).ok_or_else(|| {
            Status::unimplemented(format!(
                "{} 未实现",
                crate::grpc_contract::ops::method_name(op)
            ))
        })?;
        if frozen.schema_name != request_schema {
            return Err(Status::invalid_argument(format!(
                "schema 不匹配: op 期望 {} 实际 {request_schema}",
                frozen.schema_name
            )));
        }
        if frozen.schema_version != request_version {
            return Err(Status::invalid_argument(format!(
                "schema 版本不支持: {} v{request_version} (冻结 v{})",
                frozen.schema_name, frozen.schema_version
            )));
        }

        // fixture 模式 (离线确定性测试) 优先。
        if self.fixture_mode {
            if let Some(mut resp) = fixture::fixture_response(op, &request_schema, request_version) {
                // fixture 硬编码 request_id ("fixture-xxx"); 客户端 §6 严格匹配 request_id,
                // 必须回显请求的 request_id (与真实路径行为一致)。
                resp.request_id = req
                    .context
                    .as_ref()
                    .map(|c| c.request_id.clone())
                    .unwrap_or_default();
                return Ok(Response::new(resp));
            }
            return Err(Status::unimplemented(format!(
                "{} 无 fixture",
                frozen.schema_name
            )));
        }

        // 真实路径: 委托 data_gateway。gateway 全部是 async fn 且内部自行
        // spawn_blocking (见 delegate.rs 头注释), 直接 await, 不套 spawn_blocking。
        let result = delegate::fetch(op, &request_schema)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let request_id = req
            .context
            .as_ref()
            .map(|c| c.request_id.clone())
            .unwrap_or_else(|| "unknown".to_string());
        Ok(Response::new(QueryResponse {
            request_id,
            operation: op as i32,
            admission: AdmissionState::Admitted as i32,
            selected_provider: "tdx-dev".to_string(),
            batch_id: format!("{}-{}", frozen.schema_name, crate::grpc_client::envelope::new_request_id()),
            complete: true,
            observed_at: chrono::Local::now().to_rfc3339(),
            source_at: result.source_at,
            records: vec![CanonicalPayload {
                schema: frozen.schema_name.to_string(),
                schema_version: frozen.schema_version,
                content_type: "application/json; charset=utf-8".to_string(),
                data: result.data,
            }],
        }))
    }
}

// 54 个 RPC 的统一实现 (全部委托 serve_query; 未实现 op 返回 UNIMPLEMENTED)。
// 宏展开必须包含 #[tonic::async_trait] 属性本身: 属性宏在宏展开前运行,
// 若属性在 impl 外面而方法由宏生成, async_trait 看不到生成的方法 → E0195
// (方法与 trait 声明签名不匹配)。客户端测试 MockData 已验证此模式可编译。
macro_rules! impl_market_data_service {
    ($( $method:ident => $op:ident ),* $(,)?) => {
        #[tonic::async_trait]
        impl MarketDataService for DataService {
            $(
                async fn $method(&self, req: Request<QueryRequest>) -> Result<Response<QueryResponse>, Status> {
                    self.serve_query(Operation::$op, req.into_inner()).await
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
);
