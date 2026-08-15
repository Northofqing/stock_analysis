//! GrpcMarketClient: 24 个已实现 op 的 gRPC 查询客户端 (合同 §5-§7)。
//! 启动后应先调 GetCapabilities (合同 §7: RPC 存在 ≠ 能力准入); 未实现 op 在客户端拦截, 不发起调用。
use crate::grpc_client::auth::attach_bearer;
use crate::grpc_client::envelope::{build_query_request, parse_query_response, QueryResult};
use crate::grpc_client::errors::GrpcError;
use crate::grpc_client::pb::magic::market::v1::{
    market_data_service_client::MarketDataServiceClient, market_event_service_client::MarketEventServiceClient,
    system_service_client::SystemServiceClient, CapabilitiesRequest, EventCursor, EventFilter,
    HealthRequest, Operation, SubscribeRequest,
};
use crate::grpc_client::retry::{retry_decision, RetryDecision, RetryPolicy};
use std::time::Duration;
use tonic::transport::Channel;

pub struct GrpcMarketClient {
    data: MarketDataServiceClient<Channel>,
    system: SystemServiceClient<Channel>,
    events: MarketEventServiceClient<Channel>,
    retry: RetryPolicy,
}

impl GrpcMarketClient {
    pub async fn connect(addr: &str) -> Result<Self, GrpcError> {
        let channel = Channel::from_shared(addr.to_string())
            .map_err(|_| GrpcError::InvalidArgument)?
            // 合同 §12: 为 unary 和 stream 分别设置 deadline/keepalive。
            .timeout(Duration::from_secs(15))
            .connect()
            .await
            .map_err(|_| GrpcError::Unavailable)?;
        Ok(Self {
            data: MarketDataServiceClient::new(channel.clone()),
            system: SystemServiceClient::new(channel.clone()),
            events: MarketEventServiceClient::new(channel),
            retry: RetryPolicy::default(),
        })
    }

    pub async fn get_health(
        &mut self,
    ) -> Result<crate::grpc_client::pb::magic::market::v1::HealthResponse, GrpcError> {
        let mut req = tonic::Request::new(HealthRequest {
            context: Some(crate::grpc_client::pb::magic::market::v1::RequestContext {
                protocol_version: 1,
                request_id: crate::grpc_client::envelope::new_request_id(),
            }),
        });
        attach_bearer(&mut req)?;
        let resp = self.system.get_health(req).await.map_err(GrpcError::from)?;
        Ok(resp.into_inner())
    }

    pub async fn get_capabilities(
        &mut self,
    ) -> Result<Vec<crate::grpc_client::pb::magic::market::v1::Capability>, GrpcError> {
        let mut req = tonic::Request::new(CapabilitiesRequest {
            context: Some(crate::grpc_client::pb::magic::market::v1::RequestContext {
                protocol_version: 1,
                request_id: crate::grpc_client::envelope::new_request_id(),
            }),
        });
        attach_bearer(&mut req)?;
        let resp = self.system.get_capabilities(req).await.map_err(GrpcError::from)?;
        Ok(resp.into_inner().capabilities)
    }

    /// 按 §10 重试语义执行一次查询 (UNAVAILABLE 指数退避 / DEADLINE_EXCEEDED 有界重试,
    /// 同一业务重试保留原 request_id)。未实现 op 在客户端拦截, 不发起调用。
    pub async fn query(
        &mut self,
        op: Operation,
        payload: serde_json::Value,
    ) -> Result<QueryResult, GrpcError> {
        if !crate::grpc_contract::ops::is_implemented(op) {
            return Err(GrpcError::Unimplemented);
        }
        let request = build_query_request(op, payload)?;
        let request_id = request
            .context
            .as_ref()
            .map(|c| c.request_id.clone())
            .unwrap_or_default();

        let mut attempt: u32 = 1;
        loop {
            let outcome = self.data_call(op, request.clone()).await;
            match outcome {
                Ok(resp) => {
                    // 信封错误 (request_id 失配等) 由 From<EnvelopeError> 映射 Unknown + code=envelope。
                    return parse_query_response(&request_id, resp).map_err(GrpcError::from)
                }
                Err(err) => match retry_decision(&err) {
                    RetryDecision::RetryBackoff | RetryDecision::RetryBounded
                        if attempt < self.retry.max_attempts =>
                    {
                        tokio::time::sleep(self.retry.backoff(attempt)).await;
                        attempt += 1;
                    }
                    _ => return Err(err),
                },
            }
        }
    }

    /// Operation → MarketDataService 方法调用 (实现 op 的 match; 其余已由 is_implemented 拦截)。
    async fn data_call(
        &mut self,
        op: Operation,
        request: crate::grpc_client::pb::magic::market::v1::QueryRequest,
    ) -> Result<crate::grpc_client::pb::magic::market::v1::QueryResponse, GrpcError> {
        let mut req = tonic::Request::new(request);
        attach_bearer(&mut req)?;
        let resp = match op {
            Operation::RealtimeQuotes => self.data.realtime_quotes(req).await,
            Operation::HistoricalBars => self.data.historical_bars(req).await,
            Operation::MinuteData => self.data.minute_data(req).await,
            Operation::OrderBooks => self.data.order_books(req).await,
            Operation::MoneyFlows => self.data.money_flows(req).await,
            Operation::SecurityMetadata => self.data.security_metadata(req).await,
            Operation::Announcements => self.data.announcements(req).await,
            Operation::GlobalNews => self.data.global_news(req).await,
            Operation::EconomicCalendar => self.data.economic_calendar(req).await,
            Operation::FuturesDelivery => self.data.futures_delivery(req).await,
            Operation::GlobalIndices => self.data.global_indices(req).await,
            Operation::BoardDirectory => self.data.board_directory(req).await,
            Operation::BoardConstituents => self.data.board_constituents(req).await,
            Operation::BoardFlows => self.data.board_flows(req).await,
            Operation::LimitPools => self.data.limit_pools(req).await,
            Operation::StrongStockReasons => self.data.strong_stock_reasons(req).await,
            Operation::DragonTiger => self.data.dragon_tiger(req).await,
            Operation::MarketDragonTiger => self.data.market_dragon_tiger(req).await,
            Operation::MarketRankings => self.data.market_rankings(req).await,
            Operation::ConceptHits => self.data.concept_hits(req).await,
            Operation::Consensus => self.data.consensus(req).await,
            Operation::ResearchReports => self.data.research_reports(req).await,
            Operation::BlockTrades => self.data.block_trades(req).await,
            Operation::NorthboundDaily => self.data.northbound_daily(req).await,
            // M1 扩展 (P4): 8 个 proto 已有 op。
            Operation::ForeignExchange => self.data.foreign_exchange(req).await,
            Operation::FinancialStatements => self.data.financial_statements(req).await,
            Operation::MarketStatistics => self.data.market_statistics(req).await,
            Operation::TechnicalBars => self.data.technical_bars(req).await,
            Operation::CorporateActions => self.data.corporate_actions(req).await,
            Operation::SemanticSearch => self.data.semantic_search(req).await,
            Operation::FundFlowSeries => self.data.fund_flow_series(req).await,
            Operation::ProviderTopNRankings => self.data.provider_top_n_rankings(req).await,
            // M1 扩展 (P4): 6 个新 op (proto 编号 55-60)。
            Operation::IndexQuotes => self.data.index_quotes(req).await,
            Operation::InstrumentNews => self.data.instrument_news(req).await,
            Operation::IntradayShape => self.data.intraday_shape(req).await,
            Operation::T0Evidence => self.data.t0_evidence(req).await,
            Operation::OutcomeDailyBars => self.data.outcome_daily_bars(req).await,
            Operation::UpperLimitPoolReview => self.data.upper_limit_pool_review(req).await,
            _ => return Err(GrpcError::Unimplemented), // 防御: is_implemented 已拦截
        };
        match resp {
            Ok(r) => Ok(r.into_inner()),
            Err(status) => Err(GrpcError::from(status)),
        }
    }

    pub async fn subscribe(
        &mut self,
        filter: EventFilter,
        after: Option<EventCursor>,
    ) -> Result<
        tonic::Streaming<crate::grpc_client::pb::magic::market::v1::MarketEventEnvelope>,
        GrpcError,
    > {
        let mut req = tonic::Request::new(SubscribeRequest {
            context: Some(crate::grpc_client::pb::magic::market::v1::RequestContext {
                protocol_version: 1,
                request_id: crate::grpc_client::envelope::new_request_id(),
            }),
            filter: Some(filter),
            after,
        });
        attach_bearer(&mut req)?;
        let resp = self.events.subscribe(req).await.map_err(GrpcError::from)?;
        Ok(resp.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grpc_client::pb::magic::market::v1::{
        market_data_service_server::{MarketDataService, MarketDataServiceServer},
        system_service_server::{SystemService, SystemServiceServer},
        AdmissionState, CanonicalPayload, CapabilitiesResponse, HealthResponse, QueryRequest,
        QueryResponse,
    };
    use tonic::{Request, Response, Status};

    struct MockSystem;
    #[tonic::async_trait]
    impl SystemService for MockSystem {
        async fn get_health(&self, _req: Request<HealthRequest>) -> Result<Response<HealthResponse>, Status> {
            Ok(Response::new(HealthResponse {
                request_id: "h-1".into(),
                live: true,
                ready: true,
                state: "RUNNING".into(),
            }))
        }
        async fn get_capabilities(&self, _req: Request<CapabilitiesRequest>) -> Result<Response<CapabilitiesResponse>, Status> {
            Ok(Response::new(CapabilitiesResponse {
                request_id: "c-1".into(),
                capabilities: vec![],
            }))
        }
    }

    struct MockData;

    // macro_rules! 不能在 impl 块内定义 → 模块级宏生成整个 trait impl。
    macro_rules! impl_mock_market_data {
        ($($stub:ident),* $(,)?) => {
            #[tonic::async_trait]
            impl MarketDataService for MockData {
                async fn realtime_quotes(&self, req: Request<QueryRequest>) -> Result<Response<QueryResponse>, Status> {
            let inner = req.into_inner();
            let request_id = inner.context.unwrap().request_id;
            Ok(Response::new(QueryResponse {
                request_id,
                operation: Operation::RealtimeQuotes as i32,
                admission: AdmissionState::Admitted as i32,
                selected_provider: "mock".into(),
                batch_id: "mock-b1".into(),
                complete: true,
                observed_at: "2026-08-13T10:00:00+08:00".into(),
                source_at: "2026-08-13T10:00:00+08:00".into(),
                records: vec![CanonicalPayload {
                    schema: "market.realtime_quotes".into(),
                    schema_version: 1,
                    content_type: "application/json; charset=utf-8".into(),
                    // raw byte string (br#) 不允许非 ASCII; 用普通字符串再转 bytes。
                    data: r#"[{"code":"600519","name":"贵州茅台"}]"#.as_bytes().to_vec(),
                }],
            }))
        }

                // tonic 生成的 MarketDataService trait 共 54 个方法, 全部必须实现。
                // 这里只有 realtime_quotes 是真实桩; 其余 53 个 (proto RPC 名 camelCase) 全部 unimplemented。
                $(
                    async fn $stub(&self, _req: Request<QueryRequest>) -> Result<Response<QueryResponse>, Status> {
                        Err(Status::unimplemented(stringify!($stub)))
                    }
                )*
            }
        };
    }

    impl_mock_market_data!(
        historical_bars, minute_data, money_flows, order_books, auctions, trades,
        security_metadata, global_indices, foreign_exchange, economic_calendar,
        futures_delivery, reference_rates, official_fx_fixings, economic_series,
        company_filings, global_news, announcements, market_announcements,
        investor_questions, policy_documents, security_profiles, financial_statements,
        market_statistics, technical_bars, corporate_actions, board_directory,
        board_constituents, board_memberships, research_reports, research_documents,
        consensus, target_prices, semantic_search, fund_flow_series, board_flows,
        margin_data, block_trades, holder_counts, lockup_events, dividend_plans,
        post_close_flows, northbound_daily, limit_pools, strong_stock_reasons,
        dragon_tiger, market_dragon_tiger, dragon_tiger_discovery, market_rankings,
        market_breadth, popularity, concept_hits, option_data, provider_top_n_rankings,
        index_quotes, instrument_news, intraday_shape, t0_evidence,
        outcome_daily_bars, upper_limit_pool_review,
    );

    async fn spawn_mock() -> String {
        let addr = "127.0.0.1:0";
        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
        let local = listener.local_addr().unwrap();
        tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(SystemServiceServer::new(MockSystem))
                .add_service(MarketDataServiceServer::new(MockData))
                .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
                .await
                .unwrap();
        });
        format!("http://{local}")
    }

    #[tokio::test]
    async fn query_realtime_quotes_roundtrip() {
        let addr = spawn_mock().await;
        let mut client = GrpcMarketClient::connect(&addr).await.unwrap();
        let result = client
            .query(Operation::RealtimeQuotes, serde_json::json!({"codes": ["600519"]}))
            .await
            .unwrap();
        // 用 PartialEq 断言 (envelope.rs 已验证 AdmissionState 可比较), 不依赖 prost 是否生成 Display。
        assert_eq!(result.admission, AdmissionState::Admitted);
        assert!(result.complete);
        assert_eq!(result.records.len(), 1);
        let payload = &result.records[0];
        assert_eq!(payload.schema, "market.realtime_quotes");
        let parsed: serde_json::Value = serde_json::from_slice(&payload.data).unwrap();
        assert_eq!(parsed[0]["code"], "600519");
    }

    #[tokio::test]
    async fn query_unimplemented_op_returns_unimplemented() {
        let addr = spawn_mock().await;
        let mut client = GrpcMarketClient::connect(&addr).await.unwrap();
        // OptionData 不在 implemented 集合 → 客户端直接拦截, 不发起调用。
        let err = client.query(Operation::OptionData, serde_json::json!({})).await.unwrap_err();
        assert!(matches!(err, GrpcError::Unimplemented));
    }

    #[tokio::test]
    async fn get_health_and_capabilities_roundtrip() {
        let addr = spawn_mock().await;
        let mut client = GrpcMarketClient::connect(&addr).await.unwrap();
        let health = client.get_health().await.unwrap();
        assert!(health.live && health.ready);
        assert_eq!(health.state, "RUNNING");
        let caps = client.get_capabilities().await.unwrap();
        assert!(caps.is_empty());
    }
}
