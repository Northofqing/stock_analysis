//! Independent External generated data fixture with a test-only protobuf body mutation.

use super::external_control_loopback_fixture::{
    write_test_code_bundle, TEST_CODE_MTLS_CA_CERT, TEST_CODE_MTLS_SERVER_CERT,
    TEST_CODE_MTLS_SERVER_KEY,
};
use crate::grpc_client::external_pb::magic::market::v1::{
    market_data_service_server::{MarketDataService, MarketDataServiceServer},
    system_service_server::{SystemService, SystemServiceServer},
    AdmissionState, CanonicalPayload, CapabilitiesRequest, CapabilitiesResponse, Capability,
    ErrorDetail, HealthRequest, HealthResponse, Operation, ProviderAttemptDetail, QueryRequest,
    QueryResponse,
};
use prost::bytes::BufMut as _;
use prost::Message as _;
use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;
use tokio_stream::StreamExt as _;
use tonic::codec::{BufferSettings, Codec, EncodeBuf, Encoder};
use tonic::codegen::{http, Body, BoxFuture, Service, StdError};
use tonic::server::{Grpc, NamedService, UnaryService};
use tonic::transport::{Certificate, Identity, ServerTlsConfig};
use tonic::{Request, Response, Status};

const TEST_AUTHORIZATION: &str = "Bearer TEST_CODE_EXTERNAL_CONTROL_TOKEN";
const TEST_TLS_SERVER_NAME: &str = "macro.test.invalid";
const TEST_RECORD_DATA: &[u8] = br#"{"item_id":"TEST_CODE_EXTERNAL_NEWS_001","title":"TEST_CODE external data title","summary":"TEST_CODE external data summary","content":"TEST_CODE external data content","publisher":"TEST_CODE Eastmoney publisher","url":"https://example.com/TEST_CODE_EXTERNAL_NEWS_001","published_at":"2026-09-14T15:30:00+08:00","instruments":[{"exchange":"Shanghai","code":"TEST_CODE_600001","asset_class":"Equity"}],"topics":["TEST_CODE_external_topic"],"language":"zh-CN","evidence":{"provider":"Eastmoney","source_at":"2026-09-14 15:30","observed_at":"2026-09-14T15:31:00+08:00","batch_id":"TEST_CODE_EXTERNAL_DATA_BATCH"}}"#;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ExternalQueryWireObservation {
    pub(crate) tcp_accepts: usize,
    pub(crate) capabilities_calls: usize,
    pub(crate) capabilities_authorized: Vec<bool>,
    pub(crate) capabilities_requests: Vec<Vec<u8>>,
    pub(crate) capabilities_responses: Vec<Vec<u8>>,
    pub(crate) capabilities_status_codes: Vec<i32>,
    pub(crate) calls: usize,
    pub(crate) authorized: Vec<bool>,
    pub(crate) methods: Vec<String>,
    pub(crate) requests: Vec<Vec<u8>>,
    pub(crate) protobuf_payloads: Vec<Vec<u8>>,
    pub(crate) unexpected_methods: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ExternalQueryWireReply {
    #[default]
    Success,
    ProviderAttemptsStatus {
        unpublished_provider: bool,
    },
    CatalogLifecycleStatus,
    CatalogRequestedProviderStatus,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ExternalCapabilitiesBehavior {
    #[default]
    Stable,
    CatalogLifecycle,
    FixedCatalog {
        provider: &'static str,
    },
}

#[derive(Default)]
struct ExternalQueryWireState {
    observation: ExternalQueryWireObservation,
    reply: ExternalQueryWireReply,
    capabilities_behavior: ExternalCapabilitiesBehavior,
}

#[derive(Clone)]
struct ExternalQueryWireService {
    state: Arc<Mutex<ExternalQueryWireState>>,
    release: Arc<tokio::sync::Semaphore>,
    capabilities_release: Arc<tokio::sync::Semaphore>,
}

#[tonic::async_trait]
impl SystemService for ExternalQueryWireService {
    async fn get_health(
        &self,
        _request: Request<HealthRequest>,
    ) -> Result<Response<HealthResponse>, Status> {
        Err(Status::unimplemented(
            "TEST_CODE External query wire Health is out of scope",
        ))
    }

    async fn get_capabilities(
        &self,
        request: Request<CapabilitiesRequest>,
    ) -> Result<Response<CapabilitiesResponse>, Status> {
        let authorized = request
            .metadata()
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            == Some(TEST_AUTHORIZATION);
        let request = request.into_inner();
        let (call_ordinal, capabilities_behavior) = {
            let mut state = self
                .state
                .lock()
                .expect("TEST_CODE External query wire Capabilities capture");
            state.observation.capabilities_calls += 1;
            state.observation.capabilities_authorized.push(authorized);
            state
                .observation
                .capabilities_requests
                .push(request.encode_to_vec());
            (
                state.observation.capabilities_calls,
                state.capabilities_behavior,
            )
        };
        if !authorized {
            self.state
                .lock()
                .expect("TEST_CODE External query wire Capabilities status capture")
                .observation
                .capabilities_status_codes
                .push(tonic::Code::Unauthenticated as i32);
            return Err(Status::unauthenticated(
                "TEST_CODE External query wire bearer required",
            ));
        }
        let permit =
            self.capabilities_release.acquire().await.map_err(|_| {
                Status::cancelled("TEST_CODE External query wire Capabilities closing")
            })?;
        permit.forget();
        let request_id = request
            .context
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("TEST_CODE missing Capabilities context"))?
            .request_id
            .clone();
        let response = match capabilities_behavior {
            ExternalCapabilitiesBehavior::Stable => CapabilitiesResponse {
                request_id,
                capabilities: vec![
                    Capability {
                        operation: Operation::GlobalNews as i32,
                        repository_admission: AdmissionState::Admitted as i32,
                        runtime_available: true,
                        provider: "Eastmoney".to_owned(),
                        exact_scope: "TEST_CODE_GLOBAL_NEWS_EASTMONEY_20".to_owned(),
                        blocker: String::new(),
                        diagnostic_available: true,
                    },
                    Capability {
                        operation: Operation::SemanticSearch as i32,
                        repository_admission: AdmissionState::Unadmitted as i32,
                        runtime_available: false,
                        provider: "Bocha".to_owned(),
                        exact_scope: "TEST_CODE_UNDELIVERED_SEMANTIC_SEARCH".to_owned(),
                        blocker: "TEST_CODE_CAPABILITY_BLOCKED".to_owned(),
                        diagnostic_available: false,
                    },
                ],
            },
            ExternalCapabilitiesBehavior::CatalogLifecycle if call_ordinal == 3 => {
                self.state
                    .lock()
                    .expect("TEST_CODE External query wire refresh status capture")
                    .observation
                    .capabilities_status_codes
                    .push(tonic::Code::Unavailable as i32);
                return Err(Status::unavailable(
                    "TEST_CODE External query wire Capabilities refresh unavailable",
                ));
            }
            ExternalCapabilitiesBehavior::CatalogLifecycle => {
                let (response_id, provider) = match call_ordinal {
                    1 => (
                        "TEST_CODE_WRONG_INITIAL_CAPABILITIES_ID".to_owned(),
                        "Eastmoney",
                    ),
                    2 => (request_id, "Eastmoney"),
                    4 => (
                        "TEST_CODE_WRONG_REFRESH_CAPABILITIES_ID".to_owned(),
                        "Cailianpress",
                    ),
                    _ => (request_id, "Cailianpress"),
                };
                CapabilitiesResponse {
                    request_id: response_id,
                    capabilities: vec![catalog_capability(provider)],
                }
            }
            ExternalCapabilitiesBehavior::FixedCatalog { provider } => CapabilitiesResponse {
                request_id,
                capabilities: vec![catalog_capability(provider)],
            },
        };
        self.state
            .lock()
            .expect("TEST_CODE External query wire Capabilities response capture")
            .observation
            .capabilities_responses
            .push(response.encode_to_vec());
        Ok(Response::new(response))
    }
}

fn catalog_capability(provider: &str) -> Capability {
    Capability {
        operation: Operation::GlobalNews as i32,
        repository_admission: AdmissionState::Admitted as i32,
        runtime_available: true,
        provider: provider.to_owned(),
        exact_scope: format!("TEST_CODE_GLOBAL_NEWS_{provider}"),
        blocker: String::new(),
        diagnostic_available: true,
    }
}

impl ExternalQueryWireService {
    async fn respond(
        &self,
        request: Request<QueryRequest>,
        method: &'static str,
        operation: Operation,
    ) -> Result<Response<QueryResponse>, Status> {
        let authorized = request
            .metadata()
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            == Some(TEST_AUTHORIZATION);
        let request = request.into_inner();
        let requested_provider = request.preferred_provider.clone();
        let call_ordinal = {
            let mut state = self
                .state
                .lock()
                .expect("TEST_CODE External query wire request capture");
            state.observation.calls += 1;
            state.observation.authorized.push(authorized);
            state.observation.methods.push(method.to_owned());
            state.observation.requests.push(request.encode_to_vec());
            state.observation.calls
        };
        if !authorized {
            return Err(Status::unauthenticated(
                "TEST_CODE External query wire bearer required",
            ));
        }
        let permit = self
            .release
            .acquire()
            .await
            .map_err(|_| Status::cancelled("TEST_CODE External query wire closing"))?;
        permit.forget();
        let request_id = request
            .context
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("TEST_CODE missing data context"))?
            .request_id
            .clone();
        let reply = self
            .state
            .lock()
            .expect("TEST_CODE External query wire reply mode")
            .reply;
        let attempt_status = match reply {
            ExternalQueryWireReply::ProviderAttemptsStatus {
                unpublished_provider,
            } => Some((
                vec![
                    ProviderAttemptDetail {
                        ordinal: 1,
                        provider: "Eastmoney".to_owned(),
                        outcome: "rejected".to_owned(),
                        reason_code: "query_rejected".to_owned(),
                        retryable: false,
                        terminal: false,
                    },
                    ProviderAttemptDetail {
                        ordinal: 2,
                        provider: "Eastmoney".to_owned(),
                        outcome: "failed".to_owned(),
                        reason_code: "unavailable".to_owned(),
                        retryable: true,
                        terminal: false,
                    },
                    ProviderAttemptDetail {
                        ordinal: 3,
                        provider: if unpublished_provider {
                            "Cailianpress"
                        } else {
                            "Eastmoney"
                        }
                        .to_owned(),
                        outcome: "selected".to_owned(),
                        reason_code: "selected".to_owned(),
                        retryable: false,
                        terminal: false,
                    },
                ],
                "Eastmoney",
            )),
            ExternalQueryWireReply::CatalogLifecycleStatus => {
                let provider = if call_ordinal == 5 {
                    "Cailianpress"
                } else {
                    "Eastmoney"
                };
                Some((complete_provider_attempts(provider), provider))
            }
            ExternalQueryWireReply::CatalogRequestedProviderStatus => Some((
                complete_provider_attempts(&requested_provider),
                requested_provider.as_str(),
            )),
            ExternalQueryWireReply::Success => None,
        };
        if let Some((provider_attempts, provider)) = attempt_status {
            let details = ErrorDetail {
                request_id,
                operation: Operation::GlobalNews as i32,
                provider: provider.to_owned(),
                reason_code: "invalid_evidence".to_owned(),
                retryable: false,
                admission: AdmissionState::Admitted as i32,
                provider_attempts,
                ..ErrorDetail::default()
            }
            .encode_to_vec();
            return Err(Status::with_details(
                tonic::Code::FailedPrecondition,
                "TEST_CODE External provider attempts",
                details.into(),
            ));
        }
        Ok(Response::new(QueryResponse {
            request_id,
            operation: operation as i32,
            admission: AdmissionState::Admitted as i32,
            selected_provider: "Eastmoney".to_owned(),
            batch_id: "TEST_CODE_EXTERNAL_DATA_BATCH".to_owned(),
            complete: true,
            observed_at: "2026-09-14T15:31:00+08:00".to_owned(),
            source_at: "2026-09-14 15:30".to_owned(),
            records: vec![CanonicalPayload {
                schema: "magic.market.news_item".to_owned(),
                schema_version: 2,
                content_type: "application/json; charset=utf-8".to_owned(),
                data: TEST_RECORD_DATA.to_vec(),
            }],
            diagnostic_blocker: String::new(),
        }))
    }
}

fn complete_provider_attempts(provider: &str) -> Vec<ProviderAttemptDetail> {
    vec![
        ProviderAttemptDetail {
            ordinal: 1,
            provider: provider.to_owned(),
            outcome: "rejected".to_owned(),
            reason_code: "query_rejected".to_owned(),
            retryable: false,
            terminal: false,
        },
        ProviderAttemptDetail {
            ordinal: 2,
            provider: provider.to_owned(),
            outcome: "failed".to_owned(),
            reason_code: "unavailable".to_owned(),
            retryable: true,
            terminal: true,
        },
        ProviderAttemptDetail {
            ordinal: 3,
            provider: provider.to_owned(),
            outcome: "selected".to_owned(),
            reason_code: "selected".to_owned(),
            retryable: false,
            terminal: false,
        },
    ]
}

macro_rules! external_query_wire_service {
    ($($method:ident),* $(,)?) => {
        #[tonic::async_trait]
        impl MarketDataService for ExternalQueryWireService {
            async fn global_news(
                &self,
                request: Request<QueryRequest>,
            ) -> Result<Response<QueryResponse>, Status> {
                self.respond(request, "global_news", Operation::GlobalNews).await
            }

            async fn security_metadata(
                &self,
                request: Request<QueryRequest>,
            ) -> Result<Response<QueryResponse>, Status> {
                self.respond(request, "security_metadata", Operation::SecurityMetadata).await
            }

            async fn instrument_news(
                &self,
                request: Request<QueryRequest>,
            ) -> Result<Response<QueryResponse>, Status> {
                self.respond(request, "instrument_news", Operation::InstrumentNews).await
            }

            $(async fn $method(
                &self,
                request: Request<QueryRequest>,
            ) -> Result<Response<QueryResponse>, Status> {
                let request = request.into_inner();
                let mut state = self
                    .state
                    .lock()
                    .expect("TEST_CODE unexpected External query wire method");
                state.observation.calls += 1;
                state.observation.methods.push(stringify!($method).to_owned());
                state.observation.requests.push(request.encode_to_vec());
                state
                    .observation
                    .unexpected_methods
                    .push(stringify!($method).to_owned());
                Err(Status::failed_precondition(
                    "TEST_CODE unexpected External query wire method",
                ))
            })*
        }
    };
}

external_query_wire_service!(
    historical_bars,
    minute_data,
    realtime_quotes,
    money_flows,
    order_books,
    auctions,
    trades,
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
    intraday_shape,
    t0_evidence,
    outcome_daily_bars,
    upper_limit_pool_review,
    current_auction_observations,
    economic_release_observations,
    economic_release_schedule,
);

#[derive(Clone, Copy)]
enum ExternalResponseSuffix {
    ZeroLengthSource,
    UnknownGroup,
    NonEmptySource,
    WrongWireSource,
    MalformedPayload,
}

#[derive(Clone)]
struct ZeroLengthSourceBodyCodec {
    state: Arc<Mutex<ExternalQueryWireState>>,
    suffix: ExternalResponseSuffix,
}

#[derive(Clone)]
struct ZeroLengthSourceBodyEncoder {
    state: Arc<Mutex<ExternalQueryWireState>>,
    suffix: ExternalResponseSuffix,
}

impl Codec for ZeroLengthSourceBodyCodec {
    type Encode = QueryResponse;
    type Decode = QueryRequest;
    type Encoder = ZeroLengthSourceBodyEncoder;
    type Decoder = tonic_prost::ProstDecoder<QueryRequest>;

    fn encoder(&mut self) -> Self::Encoder {
        ZeroLengthSourceBodyEncoder {
            state: Arc::clone(&self.state),
            suffix: self.suffix,
        }
    }

    fn decoder(&mut self) -> Self::Decoder {
        tonic_prost::ProstDecoder::new(BufferSettings::default())
    }
}

impl Encoder for ZeroLengthSourceBodyEncoder {
    type Item = QueryResponse;
    type Error = Status;

    fn encode(
        &mut self,
        item: Self::Item,
        destination: &mut EncodeBuf<'_>,
    ) -> Result<(), Self::Error> {
        let mut protobuf_payload = item.encode_to_vec();
        protobuf_payload.extend_from_slice(match self.suffix {
            ExternalResponseSuffix::ZeroLengthSource => &[0x5a, 0x00],
            ExternalResponseSuffix::UnknownGroup => &[0x63, 0x68, 0x01, 0x64],
            ExternalResponseSuffix::NonEmptySource => &[0x5a, 0x01, b'x'],
            ExternalResponseSuffix::WrongWireSource => &[0x58, 0x00],
            ExternalResponseSuffix::MalformedPayload => &[0x6a, 0x02, 0x01],
        });
        destination.put_slice(&protobuf_payload);
        self.state
            .lock()
            .expect("TEST_CODE External query wire response capture")
            .observation
            .protobuf_payloads
            .push(protobuf_payload);
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum ExternalQueryWireRoute {
    Generated,
    Mutated(ExternalResponseSuffix),
}

#[derive(Clone)]
struct ExternalQueryWireServer {
    generated: MarketDataServiceServer<ExternalQueryWireService>,
    inner: Arc<ExternalQueryWireService>,
    route: ExternalQueryWireRoute,
}

impl ExternalQueryWireServer {
    fn new(service: ExternalQueryWireService, route: ExternalQueryWireRoute) -> Self {
        let inner = Arc::new(service);
        Self {
            generated: MarketDataServiceServer::from_arc(Arc::clone(&inner)),
            inner,
            route,
        }
    }
}

impl<B> Service<http::Request<B>> for ExternalQueryWireServer
where
    B: Body + Send + 'static,
    B::Error: Into<StdError> + Send + 'static,
{
    type Response = http::Response<tonic::body::Body>;
    type Error = Infallible;
    type Future = BoxFuture<Self::Response, Self::Error>;

    fn poll_ready(&mut self, _context: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: http::Request<B>) -> Self::Future {
        let ExternalQueryWireRoute::Mutated(suffix) = self.route else {
            return self.generated.call(request);
        };
        if request.uri().path() != "/magic.market.v1.MarketDataService/GlobalNews" {
            return self.generated.call(request);
        }

        struct GlobalNewsMutationService(Arc<ExternalQueryWireService>);
        impl UnaryService<QueryRequest> for GlobalNewsMutationService {
            type Response = QueryResponse;
            type Future = BoxFuture<tonic::Response<Self::Response>, tonic::Status>;

            fn call(&mut self, request: Request<QueryRequest>) -> Self::Future {
                let inner = Arc::clone(&self.0);
                Box::pin(async move {
                    <ExternalQueryWireService as MarketDataService>::global_news(
                        inner.as_ref(),
                        request,
                    )
                    .await
                })
            }
        }

        let method = GlobalNewsMutationService(Arc::clone(&self.inner));
        let codec = ZeroLengthSourceBodyCodec {
            state: Arc::clone(&self.inner.state),
            suffix,
        };
        Box::pin(async move {
            let mut grpc = Grpc::new(codec);
            Ok(grpc.unary(method, request).await)
        })
    }
}

impl NamedService for ExternalQueryWireServer {
    const NAME: &'static str = "magic.market.v1.MarketDataService";
}

pub(crate) struct ExternalQueryWireFixture {
    bundle_path: PathBuf,
    endpoint: String,
    state: Arc<Mutex<ExternalQueryWireState>>,
    release: Arc<tokio::sync::Semaphore>,
    capabilities_release: Arc<tokio::sync::Semaphore>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<Result<(), tonic::transport::Error>>>,
    temp_dir: Option<tempfile::TempDir>,
}

impl ExternalQueryWireFixture {
    pub(crate) async fn bind() -> Result<Self, String> {
        Self::bind_with_route(ExternalQueryWireRoute::Mutated(
            ExternalResponseSuffix::ZeroLengthSource,
        ))
        .await
    }

    pub(crate) async fn bind_unknown_group() -> Result<Self, String> {
        Self::bind_with_route(ExternalQueryWireRoute::Mutated(
            ExternalResponseSuffix::UnknownGroup,
        ))
        .await
    }

    pub(crate) async fn bind_nonempty_source() -> Result<Self, String> {
        Self::bind_with_route(ExternalQueryWireRoute::Mutated(
            ExternalResponseSuffix::NonEmptySource,
        ))
        .await
    }

    pub(crate) async fn bind_wrong_wire_source() -> Result<Self, String> {
        Self::bind_with_route(ExternalQueryWireRoute::Mutated(
            ExternalResponseSuffix::WrongWireSource,
        ))
        .await
    }

    pub(crate) async fn bind_malformed_payload() -> Result<Self, String> {
        Self::bind_with_route(ExternalQueryWireRoute::Mutated(
            ExternalResponseSuffix::MalformedPayload,
        ))
        .await
    }

    pub(crate) async fn bind_generated_routes() -> Result<Self, String> {
        Self::bind_with_route(ExternalQueryWireRoute::Generated).await
    }

    pub(crate) async fn bind_provider_attempts(unpublished_provider: bool) -> Result<Self, String> {
        Self::bind_with_route_and_reply(
            ExternalQueryWireRoute::Generated,
            ExternalQueryWireReply::ProviderAttemptsStatus {
                unpublished_provider,
            },
        )
        .await
    }

    pub(crate) async fn bind_catalog_lifecycle() -> Result<Self, String> {
        Self::bind_catalog_scenario(
            ExternalQueryWireReply::CatalogLifecycleStatus,
            ExternalCapabilitiesBehavior::CatalogLifecycle,
        )
        .await
    }

    pub(crate) async fn bind_catalog_endpoint_eastmoney() -> Result<Self, String> {
        Self::bind_catalog_scenario(
            ExternalQueryWireReply::CatalogRequestedProviderStatus,
            ExternalCapabilitiesBehavior::FixedCatalog {
                provider: "Eastmoney",
            },
        )
        .await
    }

    pub(crate) async fn bind_catalog_endpoint_cailianpress() -> Result<Self, String> {
        Self::bind_catalog_scenario(
            ExternalQueryWireReply::CatalogRequestedProviderStatus,
            ExternalCapabilitiesBehavior::FixedCatalog {
                provider: "Cailianpress",
            },
        )
        .await
    }

    async fn bind_with_route(route: ExternalQueryWireRoute) -> Result<Self, String> {
        Self::bind_with_route_and_reply(route, ExternalQueryWireReply::Success).await
    }

    async fn bind_with_route_and_reply(
        route: ExternalQueryWireRoute,
        reply: ExternalQueryWireReply,
    ) -> Result<Self, String> {
        Self::bind_with_route_reply_and_capabilities(
            route,
            reply,
            ExternalCapabilitiesBehavior::Stable,
        )
        .await
    }

    async fn bind_catalog_scenario(
        reply: ExternalQueryWireReply,
        capabilities_behavior: ExternalCapabilitiesBehavior,
    ) -> Result<Self, String> {
        Self::bind_with_route_reply_and_capabilities(
            ExternalQueryWireRoute::Generated,
            reply,
            capabilities_behavior,
        )
        .await
    }

    async fn bind_with_route_reply_and_capabilities(
        route: ExternalQueryWireRoute,
        reply: ExternalQueryWireReply,
        capabilities_behavior: ExternalCapabilitiesBehavior,
    ) -> Result<Self, String> {
        let listener = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::net::TcpListener::bind("127.0.0.1:0"),
        )
        .await
        .map_err(|_| "TEST_CODE External query wire bind deadline".to_owned())?
        .map_err(|error| format!("TEST_CODE External query wire bind: {error}"))?;
        let endpoint = format!(
            "https://{}",
            listener
                .local_addr()
                .map_err(|error| format!("TEST_CODE External query wire address: {error}"))?
        );
        let temp_dir = tempfile::Builder::new()
            .prefix("TEST_CODE-external-query-wire-")
            .tempdir()
            .map_err(|error| format!("TEST_CODE External query wire tempdir: {error}"))?;
        let bundle_path =
            write_test_code_bundle(temp_dir.path(), "valid", &endpoint, TEST_TLS_SERVER_NAME)?;
        let tls = ServerTlsConfig::new()
            .identity(Identity::from_pem(
                TEST_CODE_MTLS_SERVER_CERT,
                TEST_CODE_MTLS_SERVER_KEY,
            ))
            .client_ca_root(Certificate::from_pem(TEST_CODE_MTLS_CA_CERT))
            .client_auth_optional(false)
            .timeout(Duration::from_secs(5));
        let mut builder = tonic::transport::Server::builder()
            .tls_config(tls)
            .map_err(|error| format!("TEST_CODE External query wire TLS config: {error}"))?;
        let state = Arc::new(Mutex::new(ExternalQueryWireState {
            reply,
            capabilities_behavior,
            ..ExternalQueryWireState::default()
        }));
        let release = Arc::new(tokio::sync::Semaphore::new(0));
        let capabilities_release = Arc::new(tokio::sync::Semaphore::new(0));
        let service = ExternalQueryWireService {
            state: Arc::clone(&state),
            release: Arc::clone(&release),
            capabilities_release: Arc::clone(&capabilities_release),
        };
        let accept_state = Arc::clone(&state);
        let incoming =
            tokio_stream::wrappers::TcpListenerStream::new(listener).map(move |connection| {
                if connection.is_ok() {
                    accept_state
                        .lock()
                        .expect("TEST_CODE External query wire TCP capture")
                        .observation
                        .tcp_accepts += 1;
                }
                connection
            });
        let (shutdown, receive) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            builder
                .add_service(SystemServiceServer::new(service.clone()))
                .add_service(ExternalQueryWireServer::new(service, route))
                .serve_with_incoming_shutdown(incoming, async move {
                    let _ = receive.await;
                })
                .await
        });
        Ok(Self {
            bundle_path,
            endpoint,
            state,
            release,
            capabilities_release,
            shutdown: Some(shutdown),
            task: Some(task),
            temp_dir: Some(temp_dir),
        })
    }

    pub(crate) fn bundle_path(&self) -> &Path {
        &self.bundle_path
    }

    pub(crate) fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub(crate) fn snapshot(&self) -> ExternalQueryWireObservation {
        self.state
            .lock()
            .expect("TEST_CODE External query wire snapshot")
            .observation
            .clone()
    }

    pub(crate) fn release(&self) {
        self.release.add_permits(1);
    }

    pub(crate) fn release_capabilities(&self) {
        self.capabilities_release.add_permits(1);
    }

    pub(crate) async fn finish(mut self) -> Result<(), String> {
        self.release.close();
        self.capabilities_release.close();
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        let mut errors = Vec::new();
        if let Some(mut task) = self.task.take() {
            match tokio::time::timeout(Duration::from_secs(5), &mut task).await {
                Ok(Ok(Ok(()))) => {}
                Ok(result) => errors.push(format!(
                    "TEST_CODE External query wire server failed: {result:?}"
                )),
                Err(_) => {
                    task.abort();
                    let _ = task.await;
                    errors.push(
                        "TEST_CODE External query wire shutdown timeout; aborted and joined"
                            .to_owned(),
                    );
                }
            }
        }
        if let Some(temp_dir) = self.temp_dir.take() {
            if let Err(error) = temp_dir.close() {
                errors.push(format!(
                    "TEST_CODE External query wire tempdir close failed: {error}"
                ));
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
}

impl Drop for ExternalQueryWireFixture {
    fn drop(&mut self) {
        self.release.close();
        self.capabilities_release.close();
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}
