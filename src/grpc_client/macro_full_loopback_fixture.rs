//! Owned TEST_CODE Local Macro listener: five gated financial lanes and real web envelopes.
use super::{ClientAuthorization, ContractProfile, GrpcMarketClient};
use crate::data_gateway::{GeneralWebResearchProvider, GlobalNewsProvider};
use crate::grpc_client::pb::magic::market::v1::{
    market_data_service_server::{MarketDataService, MarketDataServiceServer},
    system_service_server::{SystemService, SystemServiceServer},
    AdmissionState, CanonicalPayload, CapabilitiesRequest, CapabilitiesResponse, HealthRequest,
    HealthResponse, Operation, QueryRequest, QueryResponse,
};
use prost::Message as _;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;
use tonic::{Request, Response, Status};
use zeroize::Zeroizing;

pub(crate) const QUERIES: [&str; 6] = [
    "2026年09月14日A股 大盘 股市 最新动态",
    "2026年09月14日国际财经 地缘政治 最新消息",
    "2026年09月14日美股 美联储 大宗商品 今日",
    "2026年09月14日中国 央行 财政 产业政策 重要新闻",
    "2026年09月14日高盛 摩根 大摩 美银 JPMorgan 中国A股 市场观点 研报",
    "2026年09月14日证券时报 第一财经 21世纪经济报道 重要财经",
];
pub(crate) const ECONOMIC_RECORDS: &str = r#"[{
 "event_id":"TEST_CODE_RELEASE_一","indicator_id":123,"country":"TEST_CODE 中国",
 "name":"TEST_CODE 实际发布🌏","period":"2026-08",
 "scheduled_at":"2026-09-14T07:29:00.123456789Z",
 "released_at":"2026-09-14T07:30:00.987654321Z",
 "previous":"-0.0000000000000001","consensus":null,"actual":"3.141592653589793",
 "revised":"","unit":"%","importance":3,"impact":"TEST_CODE 原始影响  "
}]"#;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lane {
    A,
    B,
    C,
    D,
    E,
}
impl Lane {
    pub(crate) fn index(self) -> usize {
        match self {
            Self::A => 0,
            Self::B => 1,
            Self::C => 2,
            Self::D => 3,
            Self::E => 4,
        }
    }
    pub(crate) fn news(self) -> Option<GlobalNewsProvider> {
        match self {
            Self::A => Some(GlobalNewsProvider::Eastmoney),
            Self::B => Some(GlobalNewsProvider::Cailianpress),
            Self::C => Some(GlobalNewsProvider::Jin10),
            Self::D => Some(GlobalNewsProvider::ThePaper),
            Self::E => None,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Call {
    Gateway(Lane),
    Web {
        dimension: usize,
        provider: GeneralWebResearchProvider,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CapturedCall {
    pub(crate) call: Call,
    pub(crate) request: Vec<u8>,
    pub(crate) authorized: bool,
    pub(crate) response: Option<Vec<u8>>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Observation {
    pub(crate) calls: Vec<CapturedCall>,
    pub(crate) controls: usize,
    pub(crate) unexpected: Vec<&'static str>,
}
#[derive(Clone)]
struct Service {
    observation: Arc<Mutex<Observation>>,
    gates: Arc<[tokio::sync::Semaphore; 5]>,
    web_gate_enabled: Arc<AtomicBool>,
    web_gate: Arc<tokio::sync::Semaphore>,
    seen: Arc<tokio::sync::Notify>,
}
impl Service {
    async fn query(
        &self,
        request: Request<QueryRequest>,
        operation: Operation,
    ) -> Result<Response<QueryResponse>, Status> {
        let authorized = request
            .metadata()
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            == Some("Bearer TEST_CODE_MACRO_FULL_TOKEN");
        let request = request.into_inner();
        let payload = request
            .payload
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("TEST_CODE missing payload"))?;
        let schema = match operation {
            Operation::GlobalNews => "news.global_news",
            Operation::EconomicCalendar => "market.economic_calendar",
            Operation::SemanticSearch => "market.semantic_search",
            _ => unreachable!(),
        };
        if payload.schema != schema || payload.schema_version != 1 {
            return Err(Status::invalid_argument(
                "TEST_CODE original operation schema",
            ));
        }
        let input: serde_json::Value = serde_json::from_slice(&payload.data)
            .map_err(|_| Status::invalid_argument("TEST_CODE invalid request JSON"))?;
        let provider = input["provider"].as_str().unwrap_or("");
        let call = match operation {
            Operation::GlobalNews => {
                let lane = match provider {
                    "Eastmoney" => Lane::A,
                    "Cailianpress" => Lane::B,
                    "Jin10" => Lane::C,
                    "ThePaper" => Lane::D,
                    _ => return Err(Status::invalid_argument("TEST_CODE unknown news provider")),
                };
                if input["limit"] != 20 {
                    return Err(Status::invalid_argument("TEST_CODE news limit"));
                }
                Call::Gateway(lane)
            }
            Operation::EconomicCalendar => {
                if payload.data != b"{}" {
                    return Err(Status::invalid_argument(
                        "TEST_CODE economic wire must be {}",
                    ));
                }
                Call::Gateway(Lane::E)
            }
            Operation::SemanticSearch => {
                let provider = GeneralWebResearchProvider::from_wire_name(provider)
                    .ok_or_else(|| Status::invalid_argument("TEST_CODE unknown web provider"))?;
                let query = input["query"].as_str().unwrap_or("");
                let dimension = QUERIES
                    .iter()
                    .position(|expected| *expected == query)
                    .ok_or_else(|| Status::invalid_argument("TEST_CODE changed original query"))?;
                if input["limit"] != 3 {
                    return Err(Status::invalid_argument("TEST_CODE web limit"));
                }
                Call::Web {
                    dimension,
                    provider,
                }
            }
            _ => unreachable!(),
        };
        let index = {
            let mut observation = self.observation.lock().expect("TEST_CODE capture");
            let index = observation.calls.len();
            observation.calls.push(CapturedCall {
                call: call.clone(),
                request: request.encode_to_vec(),
                authorized,
                response: None,
            });
            index
        };
        self.seen.notify_one();
        if !authorized {
            return Err(Status::unauthenticated("TEST_CODE instance bearer"));
        }
        if let Call::Gateway(lane) = &call {
            self.gates[lane.index()]
                .acquire()
                .await
                .map_err(|_| Status::cancelled("TEST_CODE fixture closing"))?
                .forget();
        } else if self.web_gate_enabled.load(Ordering::Acquire) {
            self.web_gate
                .acquire()
                .await
                .map_err(|_| Status::cancelled("TEST_CODE fixture closing"))?
                .forget();
        }
        let (selected_provider, source, batch_id, records) = match &call {
            Call::Gateway(Lane::E) => (
                "Jin10".to_owned(),
                "jin10-flash-v1".to_owned(),
                "TEST_CODE_FULL_E".to_owned(),
                ECONOMIC_RECORDS.as_bytes().to_vec(),
            ),
            Call::Gateway(lane) => {
                let provider = lane.news().expect("TEST_CODE news lane");
                let label = provider.wire_name();
                let records = serde_json::json!([{
                    "item_id": format!("TEST_CODE_{label}_一"),
                    "title": format!("TEST_CODE {label} 财经🌏"),
                    "summary": "TEST_CODE 摘要  ", "content": null,
                    "publisher": "TEST_CODE 发布者", "url": "https://example.invalid/TEST_CODE/一",
                    "published_at": "2026-09-14T07:29:59.123456789Z",
                    "instruments": ["TEST_CODE_600001.SH"], "topics": ["TEST_CODE 政策"],
                    "language": "zh-CN"
                }]);
                (
                    label.to_owned(),
                    provider.source().to_owned(),
                    format!("TEST_CODE_FULL_{label}"),
                    serde_json::to_vec(&records).unwrap(),
                )
            }
            Call::Web {
                dimension,
                provider,
            } => {
                let batch_id = format!("TEST_CODE_WEB_{}_{}", dimension + 1, provider.wire_name());
                // SerpApi is VerifiedEmpty. Bocha succeeds in dimensions 1..5.
                // Dimension 6 exhausts all three eligible providers.
                let records = if *provider == GeneralWebResearchProvider::Bocha && *dimension < 5 {
                    serde_json::json!([{
                        "title": format!("TEST_CODE 网页维度{}", dimension + 1),
                        "snippet": "TEST_CODE 研究摘要🌏  ", "url": "https://example.invalid/TEST_CODE/web?q=一",
                        "publisher": "TEST_CODE 网页发布者",
                        "published_at_raw": "2026-09-14T15:29:00.123456789+08:00",
                        "published_at": "2026-09-14T07:29:00.123456789Z",
                        "evidence": {
                            "provider": "bocha", "observed_at": "2026-09-14T07:30:00.987654321Z",
                            "batch_id": batch_id, "item_id": format!("TEST_CODE_WEB_ITEM_{}", dimension + 1),
                            "publication_quality": "exact_provider_time", "use_scope": "research_only"
                        }
                    }])
                } else {
                    serde_json::json!([])
                };
                (
                    provider.wire_name().to_owned(),
                    provider.source().to_owned(),
                    batch_id,
                    serde_json::to_vec(&records).unwrap(),
                )
            }
        };
        let response = QueryResponse {
            request_id: request
                .context
                .as_ref()
                .ok_or_else(|| Status::invalid_argument("TEST_CODE missing context"))?
                .request_id
                .clone(),
            operation: operation as i32,
            admission: AdmissionState::Admitted as i32,
            selected_provider,
            batch_id,
            complete: true,
            observed_at: "2026-09-14T15:30:00.987654321+08:00".to_owned(),
            source_at: "2026-09-14T15:30:00.123456789+08:00".to_owned(),
            source,
            diagnostic_blocker: String::new(),
            records: vec![CanonicalPayload {
                schema: schema.to_owned(),
                schema_version: 1,
                content_type: "application/json; charset=utf-8".to_owned(),
                data: records,
            }],
        };
        self.observation.lock().expect("TEST_CODE response").calls[index].response =
            Some(response.encode_to_vec());
        Ok(Response::new(response))
    }
}
macro_rules! service {
    ($($method:ident),* $(,)?) => {
        #[tonic::async_trait]
        impl MarketDataService for Service {
            async fn global_news(&self, request: Request<QueryRequest>) -> Result<Response<QueryResponse>, Status> {
                self.query(request, Operation::GlobalNews).await
            }
            async fn economic_calendar(&self, request: Request<QueryRequest>) -> Result<Response<QueryResponse>, Status> {
                self.query(request, Operation::EconomicCalendar).await
            }
            async fn semantic_search(&self, request: Request<QueryRequest>) -> Result<Response<QueryResponse>, Status> {
                self.query(request, Operation::SemanticSearch).await
            }
            $(async fn $method(&self, _: Request<QueryRequest>) -> Result<Response<QueryResponse>, Status> {
                self.observation.lock().expect("TEST_CODE unexpected").unexpected.push(stringify!($method));
                Err(Status::unimplemented("TEST_CODE outside Macro"))
            })*
        }
    };
}
service!(
    realtime_quotes,
    historical_bars,
    minute_data,
    money_flows,
    order_books,
    auctions,
    trades,
    security_metadata,
    global_indices,
    foreign_exchange,
    futures_delivery,
    reference_rates,
    official_fx_fixings,
    economic_series,
    company_filings,
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
    benchmark_bars,
);
#[tonic::async_trait]
impl SystemService for Service {
    async fn get_health(
        &self,
        _: Request<HealthRequest>,
    ) -> Result<Response<HealthResponse>, Status> {
        self.observation
            .lock()
            .expect("TEST_CODE controls")
            .controls += 1;
        Err(Status::failed_precondition(
            "TEST_CODE Local has no Health effect",
        ))
    }
    async fn get_capabilities(
        &self,
        _: Request<CapabilitiesRequest>,
    ) -> Result<Response<CapabilitiesResponse>, Status> {
        self.observation
            .lock()
            .expect("TEST_CODE controls")
            .controls += 1;
        Err(Status::failed_precondition(
            "TEST_CODE Local has no Capabilities effect",
        ))
    }
}
pub(crate) struct MacroFullLoopbackServer {
    endpoint: String,
    service: Service,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<Result<(), tonic::transport::Error>>>,
}
impl MacroFullLoopbackServer {
    pub(crate) async fn bind() -> Self {
        let listener = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::net::TcpListener::bind("127.0.0.1:0"),
        )
        .await
        .expect("TEST_CODE full Macro bind deadline")
        .expect("TEST_CODE full Macro bind");
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let service = Service {
            observation: Arc::new(Mutex::new(Observation::default())),
            gates: Arc::new(std::array::from_fn(|_| tokio::sync::Semaphore::new(0))),
            web_gate_enabled: Arc::new(AtomicBool::new(false)),
            web_gate: Arc::new(tokio::sync::Semaphore::new(0)),
            seen: Arc::new(tokio::sync::Notify::new()),
        };
        let serving = service.clone();
        let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
        let (shutdown, receive) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(MarketDataServiceServer::new(serving.clone()))
                .add_service(SystemServiceServer::new(serving))
                .serve_with_incoming_shutdown(incoming, async move {
                    let _ = receive.await;
                })
                .await
        });
        Self {
            endpoint,
            service,
            shutdown: Some(shutdown),
            task: Some(task),
        }
    }
    pub(crate) fn endpoint(&self) -> &str {
        &self.endpoint
    }
    pub(crate) async fn connect(&self) -> GrpcMarketClient {
        let channel = tokio::time::timeout(
            Duration::from_secs(5),
            tonic::transport::Channel::from_shared(self.endpoint.clone())
                .unwrap()
                .connect(),
        )
        .await
        .expect("TEST_CODE full connect deadline")
        .expect("TEST_CODE full connect");
        GrpcMarketClient::from_channel(
            channel,
            ContractProfile::LocalBridgeV1,
            ClientAuthorization::InstanceBearer(Zeroizing::new(
                "TEST_CODE_MACRO_FULL_TOKEN".to_owned(),
            )),
            None,
        )
    }
    pub(crate) fn snapshot(&self) -> Observation {
        self.service
            .observation
            .lock()
            .expect("TEST_CODE snapshot")
            .clone()
    }
    pub(crate) async fn wait_for_gateway_count(&self, count: usize) {
        loop {
            let seen = self.service.seen.notified();
            if self
                .snapshot()
                .calls
                .iter()
                .filter(|call| matches!(call.call, Call::Gateway(_)))
                .count()
                >= count
            {
                return;
            }
            seen.await;
        }
    }
    pub(crate) fn enable_web_response_gate_for_test(&self) {
        assert!(
            self.snapshot()
                .calls
                .iter()
                .all(|call| !matches!(call.call, Call::Web { .. })),
            "TEST_CODE Web response gate must be configured before the first Web request"
        );
        self.service.web_gate_enabled.store(true, Ordering::Release);
    }
    pub(crate) async fn wait_for_web_count(&self, count: usize) {
        loop {
            let seen = self.service.seen.notified();
            if self
                .snapshot()
                .calls
                .iter()
                .filter(|call| matches!(call.call, Call::Web { .. }))
                .count()
                >= count
            {
                return;
            }
            seen.await;
        }
    }
    pub(crate) fn release_web_response_for_test(&self) {
        assert!(
            self.service.web_gate_enabled.load(Ordering::Acquire),
            "TEST_CODE Web response gate is disabled"
        );
        self.service.web_gate.add_permits(1);
    }
    pub(crate) fn release(&self, lane: Lane) {
        self.service.gates[lane.index()].add_permits(1);
    }
    pub(crate) fn release_all(&self) {
        for gate in self.service.gates.iter() {
            gate.add_permits(8);
        }
    }
    pub(crate) async fn finish(mut self) -> Result<(), String> {
        for gate in self.service.gates.iter() {
            gate.close();
        }
        self.service.web_gate.close();
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(mut task) = self.task.take() {
            match tokio::time::timeout(Duration::from_secs(5), &mut task).await {
                Ok(Ok(Ok(()))) => {}
                Ok(result) => return Err(format!("TEST_CODE full server failed: {result:?}")),
                Err(_) => {
                    task.abort();
                    let _ = task.await;
                    return Err("TEST_CODE full shutdown timeout; aborted and joined".to_owned());
                }
            }
        }
        Ok(())
    }
}
impl Drop for MacroFullLoopbackServer {
    fn drop(&mut self) {
        for gate in self.service.gates.iter() {
            gate.close();
        }
        self.service.web_gate.close();
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}
