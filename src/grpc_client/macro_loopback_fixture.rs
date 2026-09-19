//! TEST_CODE first-source integration fixture; no process configuration or auth.

use super::{ClientAuthorization, ContractProfile, GrpcMarketClient};
use crate::grpc_client::pb::magic::market::v1::{
    market_data_service_server::{MarketDataService, MarketDataServiceServer},
    system_service_server::{SystemService, SystemServiceServer},
    AdmissionState, CanonicalPayload, CapabilitiesRequest, CapabilitiesResponse, ErrorDetail,
    HealthRequest, HealthResponse, Operation, QueryRequest, QueryResponse,
};
use prost::Message as _;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_stream::StreamExt as _;
use tonic::{Code, Request, Response, Status};
use zeroize::Zeroizing;

pub(crate) const NEWS_RECORDS: &str = r#"[
 {"item_id":"TEST_CODE_NEWS_一","title":"TEST_CODE 财经🌏  ",
  "summary":"TEST_CODE 摘要\n第二行  ","content":null,
  "publisher":"TEST_CODE 发布者甲","url":"https://example.invalid/TEST_CODE/news?x=1&y=二",
  "published_at":"2026-09-14T15:29:59.123456789+08:00",
  "instruments":["TEST_CODE_600001.SH","TEST_CODE_000001.SZ"],
  "topics":["TEST_CODE 产业","TEST_CODE 政策"],"language":"zh-CN"},
 {"item_id":"TEST_CODE_NEWS_二","title":"TEST_CODE second headline",
  "summary":"","content":"TEST_CODE 正文\t末尾  ",
  "publisher":"TEST_CODE publisher B","url":"https://example.invalid/TEST_CODE/second",
  "published_at":"2026-09-14T07:29:58.987654321Z",
  "instruments":[],"topics":[],"language":"en"}
]"#;

const CHANGED_RECORDS: &str = r#"[
 {"item_id":"TEST_CODE_CHANGED_NEWS","title":"TEST_CODE changed on reopen",
  "summary":null,"content":"TEST_CODE changed content","publisher":"TEST_CODE changed",
  "url":"https://example.invalid/TEST_CODE/changed",
  "published_at":"2026-09-15T15:29:00+08:00",
  "instruments":[],"topics":[],"language":"zh-CN"}
]"#;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct MacroObservation {
    pub(crate) requests: Vec<Vec<u8>>,
    pub(crate) authorized: Vec<bool>,
    pub(crate) responses: Vec<Vec<u8>>,
    pub(crate) statuses: Vec<MacroObservedStatus>,
    pub(crate) health_calls: usize,
    pub(crate) capabilities_calls: usize,
    pub(crate) unexpected_data_calls: Vec<&'static str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MacroObservedStatus {
    pub(crate) code: i32,
    pub(crate) details: Vec<u8>,
    pub(crate) trailer: Vec<u8>,
}

#[derive(Default)]
struct State {
    observation: MacroObservation,
    tcp_accepts: usize,
    changed: bool,
    retry_then_success: bool,
    terminal_status: bool,
    verified_empty: bool,
}

#[derive(Clone)]
struct MacroService {
    state: Arc<Mutex<State>>,
    seen: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Semaphore>,
}

impl MacroService {
    async fn news(
        &self,
        request: Request<QueryRequest>,
    ) -> Result<Response<QueryResponse>, Status> {
        let authorized = request
            .metadata()
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            == Some("Bearer TEST_CODE_MACRO_SOURCE_TOKEN");
        let request = request.into_inner();
        {
            let mut state = self.state.lock().expect("TEST_CODE Macro capture");
            state.observation.requests.push(request.encode_to_vec());
            state.observation.authorized.push(authorized);
        }
        self.seen.notify_one();
        if !authorized {
            return Err(Status::unauthenticated(
                "TEST_CODE instance bearer required",
            ));
        }
        let permit = self
            .release
            .acquire()
            .await
            .map_err(|_| Status::cancelled("TEST_CODE fixture closing"))?;
        permit.forget();
        let mut state = self.state.lock().expect("TEST_CODE Macro response");
        let request_id = request
            .context
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("TEST_CODE missing context"))?
            .request_id
            .clone();
        if !state.changed
            && (state.terminal_status
                || (state.retry_then_success && state.observation.statuses.is_empty()))
        {
            let retryable = !state.terminal_status && state.retry_then_success;
            let details = ErrorDetail {
                request_id: request_id.clone(),
                operation: Operation::GlobalNews as i32,
                provider: "Eastmoney".to_owned(),
                reason_code: "no_verified_batch".to_owned(),
                retryable,
                ..ErrorDetail::default()
            }
            .encode_to_vec();
            let mut status = Status::with_details(
                Code::Unavailable,
                "TEST_CODE terminal Macro status",
                details.into(),
            );
            let trailer = status.details().to_vec();
            status.metadata_mut().insert_bin(
                "magic-error-detail-bin",
                tonic::metadata::MetadataValue::from_bytes(&trailer),
            );
            let observed = MacroObservedStatus {
                code: status.code() as i32,
                details: status.details().to_vec(),
                trailer: status
                    .metadata()
                    .get_bin("magic-error-detail-bin")
                    .expect("TEST_CODE terminal Macro trailer")
                    .to_bytes()
                    .expect("TEST_CODE terminal Macro trailer bytes")
                    .to_vec(),
            };
            state.observation.statuses.push(observed);
            return Err(status);
        }
        let response = QueryResponse {
            request_id,
            operation: Operation::GlobalNews as i32,
            admission: AdmissionState::Admitted as i32,
            selected_provider: "Eastmoney".to_owned(),
            batch_id: if state.changed {
                "TEST_CODE_MACRO_BATCH_CHANGED"
            } else {
                "TEST_CODE_MACRO_BATCH_FIRST"
            }
            .to_owned(),
            complete: true,
            observed_at: "2026-09-14T15:30:00.987654321+08:00".to_owned(),
            source_at: "2026-09-14T15:30:00.123456789+08:00".to_owned(),
            source: "eastmoney-web".to_owned(),
            diagnostic_blocker: String::new(),
            records: vec![CanonicalPayload {
                schema: "news.global_news".to_owned(),
                schema_version: 1,
                content_type: "application/json; charset=utf-8".to_owned(),
                data: if state.changed {
                    CHANGED_RECORDS
                } else if state.verified_empty {
                    "[]"
                } else {
                    NEWS_RECORDS
                }
                .as_bytes()
                .to_vec(),
            }],
        };
        state.observation.responses.push(response.encode_to_vec());
        Ok(Response::new(response))
    }
}

macro_rules! macro_data_service {
    ($($method:ident),* $(,)?) => {
        #[tonic::async_trait]
        impl MarketDataService for MacroService {
            async fn global_news(&self, request: Request<QueryRequest>)
                -> Result<Response<QueryResponse>, Status> {
                self.news(request).await
            }
            $(async fn $method(&self, _: Request<QueryRequest>)
                -> Result<Response<QueryResponse>, Status> {
                self.state.lock().expect("TEST_CODE unexpected Macro RPC")
                    .observation.unexpected_data_calls.push(stringify!($method));
                Err(Status::unimplemented("TEST_CODE only first GlobalNews is released"))
            })*
        }
    };
}

macro_data_service!(
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
    economic_calendar,
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
    semantic_search,
);

#[tonic::async_trait]
impl SystemService for MacroService {
    async fn get_health(
        &self,
        _: Request<HealthRequest>,
    ) -> Result<Response<HealthResponse>, Status> {
        self.state
            .lock()
            .expect("TEST_CODE Health capture")
            .observation
            .health_calls += 1;
        Err(Status::failed_precondition(
            "TEST_CODE Local route has no Health effect",
        ))
    }

    async fn get_capabilities(
        &self,
        _: Request<CapabilitiesRequest>,
    ) -> Result<Response<CapabilitiesResponse>, Status> {
        self.state
            .lock()
            .expect("TEST_CODE Capabilities capture")
            .observation
            .capabilities_calls += 1;
        Err(Status::failed_precondition(
            "TEST_CODE Local route has no Capabilities effect",
        ))
    }
}

pub(crate) struct MacroLoopbackServer {
    endpoint: String,
    service: MacroService,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<Result<(), tonic::transport::Error>>>,
}

impl MacroLoopbackServer {
    /// Starts only the listener. The caller owns this handle before connecting.
    pub(crate) async fn bind() -> Self {
        let listener = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::net::TcpListener::bind("127.0.0.1:0"),
        )
        .await
        .expect("TEST_CODE Macro bind deadline")
        .expect("TEST_CODE Macro bind");
        let endpoint = format!(
            "http://{}",
            listener.local_addr().expect("TEST_CODE Macro address")
        );
        let service = MacroService {
            state: Arc::new(Mutex::new(State::default())),
            seen: Arc::new(tokio::sync::Notify::new()),
            release: Arc::new(tokio::sync::Semaphore::new(0)),
        };
        let serving = service.clone();
        let accept_state = Arc::clone(&service.state);
        let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener).map(
            move |connection| {
                if connection.is_ok() {
                    accept_state
                        .lock()
                        .expect("TEST_CODE Macro TCP capture")
                        .tcp_accepts += 1;
                }
                connection
            },
        );
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

    pub(crate) async fn connect(&self) -> GrpcMarketClient {
        connect_instance(&self.endpoint, "TEST_CODE_MACRO_SOURCE_TOKEN").await
    }

    pub(crate) async fn connect_with_invalid_instance_bearer_for_test(&self) -> GrpcMarketClient {
        connect_instance(&self.endpoint, "TEST_CODE_INVALID\nTOKEN").await
    }

    pub(crate) async fn connect_with_retry_policy_for_test(
        &self,
        policy: (u32, u64, u64, u64),
    ) -> GrpcMarketClient {
        let mut client = self.connect().await;
        client.retry = crate::grpc_client::retry::RetryPolicy {
            max_attempts: policy.0,
            base_delay_ms: policy.1,
            max_delay_ms: policy.2,
            jitter_ms: policy.3,
        };
        client
    }

    pub(crate) fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub(crate) fn snapshot(&self) -> MacroObservation {
        self.snapshot_with_tcp_for_test().1
    }

    pub(crate) fn snapshot_with_tcp_for_test(&self) -> (usize, MacroObservation) {
        let state = self
            .service
            .state
            .lock()
            .expect("TEST_CODE Macro snapshot");
        (state.tcp_accepts, state.observation.clone())
    }

    pub(crate) async fn wait_for_first_request(&self) {
        loop {
            let seen = self.service.seen.notified();
            if !self.snapshot().requests.is_empty() {
                return;
            }
            seen.await;
        }
    }

    pub(crate) fn release_response(&self) {
        self.service.release.add_permits(1);
    }

    pub(crate) fn change_response_for_reopen(&self) {
        self.service
            .state
            .lock()
            .expect("TEST_CODE changed Macro data")
            .changed = true;
        // Any erroneous replay returns promptly and is caught by the RPC count.
        self.service.release.add_permits(8);
    }

    pub(crate) fn respond_with_verified_empty(&self) {
        self.service
            .state
            .lock()
            .expect("TEST_CODE verified-empty Macro data")
            .verified_empty = true;
    }

    pub(crate) fn respond_with_terminal_status(&self) {
        self.service
            .state
            .lock()
            .expect("TEST_CODE terminal Macro status")
            .terminal_status = true;
    }

    pub(crate) fn respond_with_retry_then_success(&self) {
        self.service
            .state
            .lock()
            .expect("TEST_CODE retry-then-success Macro response")
            .retry_then_success = true;
    }

    /// Returns cleanup failure after join, never panicking before other owners
    /// can also be joined. Drop alone is only a last-resort cancellation guard.
    pub(crate) async fn finish(mut self) -> Result<(), String> {
        self.service.release.close();
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(mut task) = self.task.take() {
            match tokio::time::timeout(Duration::from_secs(5), &mut task).await {
                Ok(Ok(Ok(()))) => {}
                Ok(result) => return Err(format!("TEST_CODE Macro server failed: {result:?}")),
                Err(_) => {
                    task.abort();
                    let _ = task.await;
                    return Err("TEST_CODE Macro shutdown timeout; aborted and joined".to_owned());
                }
            }
        }
        Ok(())
    }
}

impl Drop for MacroLoopbackServer {
    fn drop(&mut self) {
        self.service.release.close();
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

pub(crate) async fn connect_parent_instance(endpoint: &str) -> GrpcMarketClient {
    connect_instance(endpoint, "TEST_CODE_BOARD_LOOPBACK_TOKEN").await
}

async fn connect_instance(endpoint: &str, bearer: &str) -> GrpcMarketClient {
    let channel = tokio::time::timeout(
        Duration::from_secs(5),
        tonic::transport::Channel::from_shared(endpoint.to_owned())
            .expect("TEST_CODE instance endpoint")
            .connect(),
    )
    .await
    .expect("TEST_CODE instance connect deadline")
    .expect("TEST_CODE instance connect");
    GrpcMarketClient::from_channel(
        channel,
        ContractProfile::LocalBridgeV1,
        ClientAuthorization::InstanceBearer(Zeroizing::new(bearer.to_owned())),
        None,
    )
}
