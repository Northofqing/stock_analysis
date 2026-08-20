//! GrpcMarketClient: 24 个已实现 op 的 gRPC 查询客户端 (合同 §5-§7)。
//! 启动后应先调 GetCapabilities (合同 §7: RPC 存在 ≠ 能力准入); 未实现 op 在客户端拦截, 不发起调用。
use crate::grpc_client::auth::{attach_bearer, attach_bearer_value};
use crate::grpc_client::bundle::ClientBundleConfig;
use crate::grpc_client::envelope::{build_query_request, parse_query_response, QueryResult};
use crate::grpc_client::errors::{ErrorDetail, GrpcError};
use crate::grpc_client::pb::magic::market::v1::{
    market_data_service_client::MarketDataServiceClient,
    market_event_service_client::MarketEventServiceClient,
    system_service_client::SystemServiceClient, CapabilitiesRequest, EventCursor, EventFilter,
    HealthRequest, ListenerStatusRequest, Operation, SetWatchlistRequest, SubscribeRequest,
};
use crate::grpc_client::retry::{retry_decision, RetryDecision, RetryPolicy};
use std::path::Path;
use std::time::Duration;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Identity};
use zeroize::Zeroizing;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractProfile {
    LocalBridgeV1,
    ExternalV1,
}

enum ClientAuthorization {
    Environment,
    InstanceBearer(Zeroizing<String>),
}

pub struct GrpcMarketClient {
    data: MarketDataServiceClient<Channel>,
    system: SystemServiceClient<Channel>,
    events: MarketEventServiceClient<Channel>,
    retry: RetryPolicy,
    profile: ContractProfile,
    authorization: ClientAuthorization,
    acquisition_authority: Option<String>,
}

impl GrpcMarketClient {
    pub async fn connect(addr: &str) -> Result<Self, GrpcError> {
        let channel = Channel::from_shared(addr.to_string())
            // D2: 本地构造错误无服务端 status → details 全默认 (桥只看码 + 远端 detail)。
            .map_err(|_| GrpcError::InvalidArgument {
                details: Box::default(),
            })?
            // 合同 §12: 为 unary 和 stream 分别设置 deadline/keepalive。
            .timeout(Duration::from_secs(15))
            .connect()
            .await
            .map_err(|_| GrpcError::Unavailable {
                details: Box::default(),
            })?;
        Ok(Self::from_channel(
            channel,
            ContractProfile::LocalBridgeV1,
            ClientAuthorization::Environment,
            None,
        ))
    }

    /// Loads a validated client bundle and opens an ExternalV1 connection using
    /// its private mTLS identity and instance-owned bearer credential.
    pub async fn connect_client_bundle(path: &Path) -> Result<Self, GrpcError> {
        let ClientBundleConfig {
            endpoint_uri,
            tls_server_name,
            ca_pem,
            certificate_pem,
            private_key_pem,
            bearer_token,
        } = crate::grpc_client::bundle::load(path).map_err(|_| GrpcError::InvalidArgument {
            details: Box::default(),
        })?;

        let acquisition_authority = format!("grpc-mtls:{tls_server_name}");
        let tls = ClientTlsConfig::new()
            .domain_name(tls_server_name)
            .ca_certificate(Certificate::from_pem(ca_pem))
            .identity(Identity::from_pem(
                certificate_pem,
                private_key_pem.as_slice(),
            ));
        let channel = Channel::from_shared(endpoint_uri)
            .map_err(|_| GrpcError::InvalidArgument {
                details: Box::default(),
            })?
            .timeout(Duration::from_secs(15))
            .tls_config(tls)
            .map_err(|_| GrpcError::InvalidArgument {
                details: Box::default(),
            })?
            .connect()
            .await
            .map_err(|_| GrpcError::Unavailable {
                details: Box::default(),
            })?;

        Ok(Self::from_channel(
            channel,
            ContractProfile::ExternalV1,
            ClientAuthorization::InstanceBearer(bearer_token),
            Some(acquisition_authority),
        ))
    }

    fn from_channel(
        channel: Channel,
        profile: ContractProfile,
        authorization: ClientAuthorization,
        acquisition_authority: Option<String>,
    ) -> Self {
        Self {
            data: MarketDataServiceClient::new(channel.clone()),
            system: SystemServiceClient::new(channel.clone()),
            events: MarketEventServiceClient::new(channel),
            retry: RetryPolicy::default(),
            profile,
            authorization,
            acquisition_authority,
        }
    }

    fn attach_request_auth<T>(&self, request: &mut tonic::Request<T>) -> Result<(), GrpcError> {
        match &self.authorization {
            ClientAuthorization::Environment => attach_bearer(request)?,
            ClientAuthorization::InstanceBearer(token) => {
                attach_bearer_value(request, token.as_str())?
            }
        }
        Ok(())
    }

    fn build_profile_query_request(
        &self,
        operation: Operation,
        payload: serde_json::Value,
    ) -> Result<crate::grpc_client::pb::magic::market::v1::QueryRequest, GrpcError> {
        match self.profile {
            ContractProfile::LocalBridgeV1 => {
                build_query_request(operation, payload).map_err(GrpcError::from)
            }
            ContractProfile::ExternalV1 => {
                crate::grpc_client::external_v1::build_external_query_request(operation, payload)
                    .map_err(map_external_contract_error)
            }
        }
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
        self.attach_request_auth(&mut req)?;
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
        self.attach_request_auth(&mut req)?;
        let resp = self
            .system
            .get_capabilities(req)
            .await
            .map_err(GrpcError::from)?;
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
            return Err(GrpcError::Unimplemented {
                details: Box::default(),
            });
        }
        let request = self.build_profile_query_request(op, payload)?;
        let request_id = request
            .context
            .as_ref()
            .map(|c| c.request_id.clone())
            .unwrap_or_default();

        let mut attempt: u32 = 1;
        loop {
            let outcome = self.data_call(op, request.clone()).await;
            match outcome {
                Ok(mut resp) => {
                    apply_acquisition_authority(
                        self.profile,
                        self.acquisition_authority.as_deref(),
                        &mut resp,
                    )?;
                    // 信封错误 (request_id 失配等) 由 From<EnvelopeError> 映射 Unknown + code=envelope。
                    return parse_query_response(&request_id, op, resp).map_err(GrpcError::from);
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
        self.attach_request_auth(&mut req)?;
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
            Operation::ChainBatch => self.data.chain_batch(req).await,
            _ => {
                return Err(GrpcError::Unimplemented {
                    details: Box::default(),
                })
            } // 防御: is_implemented 已拦截
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
        self.attach_request_auth(&mut req)?;
        let resp = self.events.subscribe(req).await.map_err(GrpcError::from)?;
        Ok(resp.into_inner())
    }

    /// 合同 §8 GetListenerStatus: 读取服务端 listener 状态 (generation/cursor/
    /// watchlist 版本)。上游直连排期后用于核对服务端 watchlist 与本地 STOCK_LIST。
    pub async fn get_listener_status(
        &mut self,
    ) -> Result<crate::grpc_client::pb::magic::market::v1::ListenerStatusResponse, GrpcError> {
        let mut req = tonic::Request::new(ListenerStatusRequest {
            context: Some(crate::grpc_client::pb::magic::market::v1::RequestContext {
                protocol_version: 1,
                request_id: crate::grpc_client::envelope::new_request_id(),
            }),
        });
        self.attach_request_auth(&mut req)?;
        let resp = self
            .events
            .get_listener_status(req)
            .await
            .map_err(GrpcError::from)?;
        Ok(resp.into_inner())
    }

    /// 合同 §8 SetWatchlist: 请求覆盖服务端 watchlist (上游: 终端申请 desired,
    /// 服务端决定应用; 本地 server: 立即应用 desired==applied)。返回服务端确认。
    pub async fn set_watchlist(
        &mut self,
        instruments: Vec<String>,
    ) -> Result<crate::grpc_client::pb::magic::market::v1::SetWatchlistResponse, GrpcError> {
        let mut req = tonic::Request::new(SetWatchlistRequest {
            context: Some(crate::grpc_client::pb::magic::market::v1::RequestContext {
                protocol_version: 1,
                request_id: crate::grpc_client::envelope::new_request_id(),
            }),
            instruments,
        });
        self.attach_request_auth(&mut req)?;
        let resp = self
            .events
            .set_watchlist(req)
            .await
            .map_err(GrpcError::from)?;
        Ok(resp.into_inner())
    }
}

fn map_external_contract_error(
    error: crate::grpc_client::external_v1::ExternalContractError,
) -> GrpcError {
    use crate::grpc_client::external_v1::ExternalContractError;

    match error {
        ExternalContractError::UndeliveredOperation => GrpcError::Unimplemented {
            details: Box::default(),
        },
        ExternalContractError::InvalidParameters => GrpcError::InvalidArgument {
            details: Box::default(),
        },
        ExternalContractError::Serialize => GrpcError::Unknown {
            details: Box::new(ErrorDetail {
                code: "envelope".to_string(),
                ..ErrorDetail::default()
            }),
        },
    }
}

fn apply_acquisition_authority(
    profile: ContractProfile,
    acquisition_authority: Option<&str>,
    response: &mut crate::grpc_client::pb::magic::market::v1::QueryResponse,
) -> Result<(), GrpcError> {
    if profile != ContractProfile::ExternalV1 {
        return Ok(());
    }

    if !response.source.is_empty() {
        return Err(GrpcError::FailedPrecondition {
            details: Box::new(ErrorDetail {
                code: "external_source_field_conflict".to_string(),
                reason_code: Some("external_source_field_conflict".to_string()),
                retryable: Some(false),
                ..ErrorDetail::default()
            }),
        });
    }
    let authority = acquisition_authority.ok_or_else(|| GrpcError::FailedPrecondition {
        details: Box::new(ErrorDetail {
            code: "external_acquisition_authority_missing".to_string(),
            reason_code: Some("external_acquisition_authority_missing".to_string()),
            retryable: Some(false),
            ..ErrorDetail::default()
        }),
    })?;
    response.source = authority.to_owned();
    Ok(())
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
    use zeroize::Zeroizing;

    struct MockSystem;
    #[tonic::async_trait]
    impl SystemService for MockSystem {
        async fn get_health(
            &self,
            _req: Request<HealthRequest>,
        ) -> Result<Response<HealthResponse>, Status> {
            Ok(Response::new(HealthResponse {
                request_id: "h-1".into(),
                live: true,
                ready: true,
                state: "RUNNING".into(),
            }))
        }
        async fn get_capabilities(
            &self,
            _req: Request<CapabilitiesRequest>,
        ) -> Result<Response<CapabilitiesResponse>, Status> {
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
                source: "mock".into(),
                diagnostic_blocker: String::new(),
                records: vec![CanonicalPayload {
                    schema: "market.realtime_quotes".into(),
                    schema_version: 1,
                    content_type: "application/json; charset=utf-8".into(),
                    // raw byte string (br#) 不允许非 ASCII; 用普通字符串再转 bytes。
                    data: r#"[{"code":"600519","name":"贵州茅台"}]"#.as_bytes().to_vec(),
                }],
            }))
        }

                // tonic 生成的 MarketDataService trait 共 60 个方法 (上游 55 + 本地扩展 5),
                // 全部必须实现。这里只有 realtime_quotes 是真实桩; 其余 59 个
                // (proto RPC 名 camelCase) 全部 unimplemented。
                $(
                    async fn $stub(&self, _req: Request<QueryRequest>) -> Result<Response<QueryResponse>, Status> {
                        Err(Status::unimplemented(stringify!($stub)))
                    }
                )*
            }
        };
    }

    impl_mock_market_data!(
        historical_bars,
        minute_data,
        money_flows,
        order_books,
        auctions,
        trades,
        security_metadata,
        global_indices,
        foreign_exchange,
        economic_calendar,
        futures_delivery,
        reference_rates,
        official_fx_fixings,
        economic_series,
        company_filings,
        global_news,
        announcements,
        market_announcements,
        investor_questions,
        policy_documents,
        security_profiles,
        financial_statements,
        market_statistics,
        technical_bars,
        corporate_actions,
        board_directory,
        board_constituents,
        board_memberships,
        research_reports,
        research_documents,
        consensus,
        target_prices,
        semantic_search,
        fund_flow_series,
        board_flows,
        margin_data,
        block_trades,
        holder_counts,
        lockup_events,
        dividend_plans,
        post_close_flows,
        northbound_daily,
        limit_pools,
        strong_stock_reasons,
        dragon_tiger,
        market_dragon_tiger,
        dragon_tiger_discovery,
        market_rankings,
        market_breadth,
        popularity,
        concept_hits,
        option_data,
        provider_top_n_rankings,
        index_quotes,
        instrument_news,
        intraday_shape,
        t0_evidence,
        outcome_daily_bars,
        upper_limit_pool_review,
        chain_batch,
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
            .query(
                Operation::RealtimeQuotes,
                serde_json::json!({"codes": ["600519"]}),
            )
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
        let err = client
            .query(Operation::OptionData, serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, GrpcError::Unimplemented { .. }));
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

    fn lazy_test_channel() -> Channel {
        Channel::from_static("http://127.0.0.1:1").connect_lazy()
    }

    #[tokio::test]
    async fn contract_profiles_select_distinct_request_contracts() {
        let local = GrpcMarketClient::from_channel(
            lazy_test_channel(),
            ContractProfile::LocalBridgeV1,
            ClientAuthorization::Environment,
            None,
        );
        let local_request = local
            .build_profile_query_request(
                Operation::RealtimeQuotes,
                serde_json::json!({"codes": ["600396"]}),
            )
            .expect("local bridge request");
        assert_eq!(
            local_request.payload.expect("local payload").schema,
            "market.realtime_quotes"
        );

        let external = GrpcMarketClient::from_channel(
            lazy_test_channel(),
            ContractProfile::ExternalV1,
            ClientAuthorization::InstanceBearer(Zeroizing::new(
                "TEST_CODE_bundle_token".to_string(),
            )),
            Some("grpc-mtls:magic-market.local".to_string()),
        );
        let external_request = external
            .build_profile_query_request(
                Operation::SecurityMetadata,
                serde_json::json!({
                    "instruments": [{
                        "exchange": "Shanghai",
                        "code": "600396",
                        "asset_class": "Equity"
                    }]
                }),
            )
            .expect("delivered external request");
        assert_eq!(
            external_request.payload.expect("external payload").schema,
            "magic.market.security_metadata.request"
        );

        let invalid = external
            .build_profile_query_request(
                Operation::SecurityMetadata,
                serde_json::json!({"instruments": []}),
            )
            .expect_err("invalid external parameters must fail closed");
        assert!(matches!(invalid, GrpcError::InvalidArgument { .. }));

        let undelivered = external
            .build_profile_query_request(Operation::RealtimeQuotes, serde_json::json!({}))
            .expect_err("undelivered external contract must not reach I/O");
        assert!(matches!(undelivered, GrpcError::Unimplemented { .. }));
    }

    #[test]
    fn external_contract_error_mapping_is_non_retryable_and_specific() {
        use crate::grpc_client::external_v1::ExternalContractError;

        assert!(matches!(
            map_external_contract_error(ExternalContractError::UndeliveredOperation),
            GrpcError::Unimplemented { .. }
        ));
        assert!(matches!(
            map_external_contract_error(ExternalContractError::InvalidParameters),
            GrpcError::InvalidArgument { .. }
        ));
        let serialization = map_external_contract_error(ExternalContractError::Serialize);
        assert!(matches!(serialization, GrpcError::Unknown { .. }));
        assert_eq!(serialization.details().code, "envelope");
    }

    #[tokio::test]
    async fn instance_owned_bearer_is_attached_without_environment_fallback() {
        let client = GrpcMarketClient::from_channel(
            lazy_test_channel(),
            ContractProfile::ExternalV1,
            ClientAuthorization::InstanceBearer(Zeroizing::new(
                "TEST_CODE_bundle_token".to_string(),
            )),
            Some("grpc-mtls:magic-market.local".to_string()),
        );
        let mut request = Request::new(());
        client
            .attach_request_auth(&mut request)
            .expect("instance bearer metadata");
        assert_eq!(
            request
                .metadata()
                .get("authorization")
                .expect("authorization metadata")
                .to_str()
                .expect("ASCII authorization"),
            "Bearer TEST_CODE_bundle_token"
        );
        assert_eq!(client.profile, ContractProfile::ExternalV1);
        assert_eq!(
            client.acquisition_authority.as_deref(),
            Some("grpc-mtls:magic-market.local")
        );
    }

    fn response_with_source(source: &str) -> QueryResponse {
        QueryResponse {
            request_id: "TEST_CODE-request".to_string(),
            operation: Operation::SecurityMetadata as i32,
            admission: AdmissionState::Admitted as i32,
            selected_provider: "Tencent".to_string(),
            batch_id: "TEST_CODE-batch".to_string(),
            complete: true,
            observed_at: "2026-08-17T08:00:00+08:00".to_string(),
            source_at: "2026-08-17T07:59:59+08:00".to_string(),
            records: vec![],
            diagnostic_blocker: String::new(),
            source: source.to_string(),
        }
    }

    #[test]
    fn external_acquisition_authority_rejects_remote_field_eleven_and_uses_local_mtls() {
        let authority = "grpc-mtls:magic-market.local";
        let mut missing = response_with_source("");
        apply_acquisition_authority(ContractProfile::ExternalV1, Some(authority), &mut missing)
            .expect("local mTLS authority");
        assert_eq!(missing.source, authority);
        assert_eq!(missing.selected_provider, "Tencent");
        assert_eq!(missing.batch_id, "TEST_CODE-batch");
        assert_eq!(missing.observed_at, "2026-08-17T08:00:00+08:00");
        assert_eq!(missing.source_at, "2026-08-17T07:59:59+08:00");

        let mut upstream = response_with_source("upstream-source");
        let error = apply_acquisition_authority(
            ContractProfile::ExternalV1,
            Some(authority),
            &mut upstream,
        )
        .expect_err("ExternalV1 field 11 is not an upstream contract field");
        assert!(matches!(error, GrpcError::FailedPrecondition { .. }));

        let mut local = response_with_source("");
        apply_acquisition_authority(ContractProfile::LocalBridgeV1, Some(authority), &mut local)
            .expect("local bridge keeps its own source");
        assert!(local.source.is_empty());
    }

    #[test]
    fn client_bundle_constructor_is_exposed_without_reading_a_real_bundle() {
        let _constructor = GrpcMarketClient::connect_client_bundle;
    }
}
