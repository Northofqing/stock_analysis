//! Independent TEST_CODE fixture for deferred External control attempts.

use crate::grpc_client::external_pb::magic::market::v1::{
    market_data_service_server::{MarketDataService, MarketDataServiceServer},
    market_event_service_server::{MarketEventService, MarketEventServiceServer},
    system_service_server::{SystemService, SystemServiceServer},
    AdmissionState, BuildIdentity, CanonicalPayload, CapabilitiesRequest, CapabilitiesResponse,
    Capability, ErrorDetail, EventCursor, HealthRequest, HealthResponse, ListenerStatusRequest,
    ListenerStatusResponse, MarketEventEnvelope, Operation, ProviderAttemptDetail, QueryRequest,
    QueryResponse, ReplayRequest, RuntimeObservability, SetWatchlistRequest, SetWatchlistResponse,
    SubscribeRequest,
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ExternalControlObservation {
    pub(crate) tcp_accepts: usize,
    pub(crate) health_requests: Vec<Vec<u8>>,
    pub(crate) health_authorized: Vec<bool>,
    pub(crate) health_responses: Vec<Vec<u8>>,
    pub(crate) health_statuses: Vec<ObservedHealthStatus>,
    pub(crate) capabilities_calls: usize,
    pub(crate) capabilities_authorized: Vec<bool>,
    pub(crate) capabilities_requests: Vec<Vec<u8>>,
    pub(crate) capabilities_responses: Vec<Vec<u8>>,
    pub(crate) capabilities_statuses: Vec<ObservedHealthStatus>,
    pub(crate) data_calls: usize,
    pub(crate) data_authorized: Vec<bool>,
    pub(crate) data_methods: Vec<String>,
    pub(crate) data_requests: Vec<Vec<u8>>,
    pub(crate) data_responses: Vec<Vec<u8>>,
    pub(crate) data_statuses: Vec<ObservedHealthStatus>,
    pub(crate) listener_status_requests: Vec<Vec<u8>>,
    pub(crate) listener_status_authorized: Vec<bool>,
    pub(crate) listener_status_responses: Vec<Vec<u8>>,
    pub(crate) subscribe_requests: Vec<Vec<u8>>,
    pub(crate) subscribe_authorized: Vec<bool>,
    pub(crate) subscribe_events: Vec<Vec<u8>>,
    pub(crate) replay_requests: Vec<Vec<u8>>,
    pub(crate) watchlist_requests: Vec<Vec<u8>>,
    pub(crate) watchlist_authorized: Vec<bool>,
    pub(crate) watchlist_responses: Vec<Vec<u8>>,
    pub(crate) watchlist_status_details: Vec<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HealthStatusCase {
    Absent,
    Bytes,
    Malformed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum HealthReply {
    #[default]
    Success,
    NotReady,
    Status(HealthStatusCase),
    MismatchedId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CapabilitiesReply {
    Success,
    Status(HealthStatusCase),
    MismatchedId,
    MissingGlobalNews,
    GlobalNewsUnadmitted,
    GlobalNewsRuntimeUnavailable,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum DataReply {
    #[default]
    Reject,
    Success,
    UnavailableStatus,
    RetryThenSuccess,
    ProviderAttemptsStatus {
        unpublished_provider: bool,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum WatchlistReply {
    #[default]
    Success,
    UnavailableStatus,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ObservedHealthStatus {
    pub(crate) code: i32,
    pub(crate) details: Vec<u8>,
    pub(crate) trailer: ObservedHealthTrailer,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ObservedHealthTrailer {
    Absent,
    Bytes(Vec<u8>),
    Malformed,
}

pub(crate) fn test_external_observability() -> RuntimeObservability {
    RuntimeObservability {
        process_started_at_unix_ms: 1_797_430_001_234,
        uptime_millis: 98_765,
        query_started: 101,
        query_succeeded: 89,
        query_failed: 5,
        query_cancelled: 2,
        query_in_flight: 3,
        query_rejected: 7,
        query_timed_out: 4,
        query_duration_micros_total: 8_765_432,
        query_duration_micros_max: 345_678,
        unary_concurrency_limit: 32,
        unary_concurrency_available: 19,
        blocking_concurrency_limit: 8,
        blocking_concurrency_available: 5,
    }
}

pub(crate) fn test_external_build_identity() -> BuildIdentity {
    BuildIdentity {
        service_version: "TEST_CODE_EXTERNAL_SERVICE_VERSION".to_owned(),
        source_revision: "TEST_CODE_EXTERNAL_SOURCE_REVISION".to_owned(),
        contract_sha256: "TEST_CODE_EXTERNAL_CONTRACT_SHA256".to_owned(),
        binary_sha256: "TEST_CODE_EXTERNAL_BINARY_SHA256".to_owned(),
        identity_error: String::new(),
    }
}

#[derive(Default)]
struct ExternalControlState {
    observation: ExternalControlObservation,
    health_reply: HealthReply,
    capabilities_reply: Option<CapabilitiesReply>,
    data_reply: DataReply,
    watchlist_reply: WatchlistReply,
    reject_new_connections: bool,
    append_zero_length_source: bool,
}

#[derive(Clone)]
struct ExternalControlService {
    state: Arc<Mutex<ExternalControlState>>,
    health_release: Arc<tokio::sync::Semaphore>,
    capabilities_release: Arc<tokio::sync::Semaphore>,
    data_release: Arc<tokio::sync::Semaphore>,
}

#[tonic::async_trait]
impl SystemService for ExternalControlService {
    async fn get_health(
        &self,
        request: Request<HealthRequest>,
    ) -> Result<Response<HealthResponse>, Status> {
        let authorized = request
            .metadata()
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            == Some("Bearer TEST_CODE_EXTERNAL_CONTROL_TOKEN");
        let request = request.into_inner();
        {
            let mut state = self
                .state
                .lock()
                .expect("TEST_CODE External control Health capture");
            state
                .observation
                .health_requests
                .push(request.encode_to_vec());
            state.observation.health_authorized.push(authorized);
        }
        if !authorized {
            return Err(Status::unauthenticated(
                "TEST_CODE External control bearer required",
            ));
        }
        let permit = self
            .health_release
            .acquire()
            .await
            .map_err(|_| Status::cancelled("TEST_CODE External control fixture closing"))?;
        permit.forget();
        let request_id = request
            .context
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("TEST_CODE missing Health context"))?
            .request_id
            .clone();
        let health_reply = self
            .state
            .lock()
            .expect("TEST_CODE External control Health reply mode")
            .health_reply;
        match health_reply {
            HealthReply::Success | HealthReply::NotReady | HealthReply::MismatchedId => {
                let ready = health_reply != HealthReply::NotReady;
                let response = HealthResponse {
                    request_id: if health_reply == HealthReply::MismatchedId {
                        "TEST_CODE_WRONG_HEALTH_REQUEST_ID".to_owned()
                    } else {
                        request_id
                    },
                    live: true,
                    ready,
                    state: if ready {
                        "TEST_CODE_HEALTH_RUNNING"
                    } else {
                        "TEST_CODE_HEALTH_NOT_READY"
                    }
                    .to_owned(),
                    observability: Some(test_external_observability()),
                    build_identity: Some(test_external_build_identity()),
                };
                self.state
                    .lock()
                    .expect("TEST_CODE External control Health response")
                    .observation
                    .health_responses
                    .push(response.encode_to_vec());
                Ok(Response::new(response))
            }
            HealthReply::Status(case) => {
                let detail = ErrorDetail {
                    request_id,
                    provider: "Eastmoney".to_owned(),
                    reason_code: "unavailable".to_owned(),
                    retryable: false,
                    ..ErrorDetail::default()
                }
                .encode_to_vec();
                let mut status = match case {
                    HealthStatusCase::Bytes => {
                        Status::new(tonic::Code::Unavailable, "TEST_CODE Health unavailable")
                    }
                    HealthStatusCase::Absent | HealthStatusCase::Malformed => Status::with_details(
                        tonic::Code::Unavailable,
                        "TEST_CODE Health unavailable",
                        detail.clone().into(),
                    ),
                };
                match case {
                    HealthStatusCase::Absent => {}
                    HealthStatusCase::Bytes => {
                        status.metadata_mut().insert_bin(
                            "magic-error-detail-bin",
                            tonic::metadata::MetadataValue::from_bytes(&detail),
                        );
                    }
                    HealthStatusCase::Malformed => {
                        let mut headers = tonic::codegen::http::HeaderMap::new();
                        headers.insert(
                            "magic-error-detail-bin",
                            tonic::codegen::http::HeaderValue::from_static("%%%"),
                        );
                        *status.metadata_mut() =
                            tonic::metadata::MetadataMap::from_headers(headers);
                    }
                }
                let trailer = match status.metadata().get_bin("magic-error-detail-bin") {
                    None => ObservedHealthTrailer::Absent,
                    Some(value) => match value.to_bytes() {
                        Ok(bytes) => ObservedHealthTrailer::Bytes(bytes.to_vec()),
                        Err(_) => ObservedHealthTrailer::Malformed,
                    },
                };
                let observed = ObservedHealthStatus {
                    code: status.code() as i32,
                    details: status.details().to_vec(),
                    trailer,
                };
                self.state
                    .lock()
                    .expect("TEST_CODE External control Health status")
                    .observation
                    .health_statuses
                    .push(observed);
                Err(status)
            }
        }
    }

    async fn get_capabilities(
        &self,
        request: Request<CapabilitiesRequest>,
    ) -> Result<Response<CapabilitiesResponse>, Status> {
        let authorized = request
            .metadata()
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            == Some("Bearer TEST_CODE_EXTERNAL_CONTROL_TOKEN");
        let request = request.into_inner();
        let capabilities_reply = {
            let mut state = self
                .state
                .lock()
                .expect("TEST_CODE External control Capabilities capture");
            state.observation.capabilities_calls += 1;
            state.observation.capabilities_authorized.push(authorized);
            state
                .observation
                .capabilities_requests
                .push(request.encode_to_vec());
            state.capabilities_reply
        };
        let Some(capabilities_reply) = capabilities_reply else {
            return Err(Status::failed_precondition(
                "TEST_CODE Health test must not call Capabilities",
            ));
        };
        if !authorized {
            return Err(Status::unauthenticated(
                "TEST_CODE External control bearer required",
            ));
        }
        let permit = self
            .capabilities_release
            .acquire()
            .await
            .map_err(|_| Status::cancelled("TEST_CODE External control fixture closing"))?;
        permit.forget();
        let request_id = request
            .context
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("TEST_CODE missing Capabilities context"))?
            .request_id
            .clone();
        if let CapabilitiesReply::Status(case) = capabilities_reply {
            let detail = ErrorDetail {
                request_id,
                provider: "Eastmoney".to_owned(),
                reason_code: "unavailable".to_owned(),
                retryable: false,
                ..ErrorDetail::default()
            }
            .encode_to_vec();
            let mut status = match case {
                HealthStatusCase::Bytes => Status::new(
                    tonic::Code::Unavailable,
                    "TEST_CODE Capabilities unavailable",
                ),
                HealthStatusCase::Absent | HealthStatusCase::Malformed => Status::with_details(
                    tonic::Code::Unavailable,
                    "TEST_CODE Capabilities unavailable",
                    detail.clone().into(),
                ),
            };
            match case {
                HealthStatusCase::Absent => {}
                HealthStatusCase::Bytes => {
                    status.metadata_mut().insert_bin(
                        "magic-error-detail-bin",
                        tonic::metadata::MetadataValue::from_bytes(&detail),
                    );
                }
                HealthStatusCase::Malformed => {
                    let mut headers = tonic::codegen::http::HeaderMap::new();
                    headers.insert(
                        "magic-error-detail-bin",
                        tonic::codegen::http::HeaderValue::from_static("%%%"),
                    );
                    *status.metadata_mut() = tonic::metadata::MetadataMap::from_headers(headers);
                }
            }
            let trailer = match status.metadata().get_bin("magic-error-detail-bin") {
                None => ObservedHealthTrailer::Absent,
                Some(value) => match value.to_bytes() {
                    Ok(bytes) => ObservedHealthTrailer::Bytes(bytes.to_vec()),
                    Err(_) => ObservedHealthTrailer::Malformed,
                },
            };
            let observed = ObservedHealthStatus {
                code: status.code() as i32,
                details: status.details().to_vec(),
                trailer,
            };
            self.state
                .lock()
                .expect("TEST_CODE External control Capabilities status")
                .observation
                .capabilities_statuses
                .push(observed);
            return Err(status);
        }
        let global_news = Capability {
            operation: Operation::GlobalNews as i32,
            repository_admission: AdmissionState::Admitted as i32,
            runtime_available: true,
            provider: "Eastmoney".to_owned(),
            exact_scope: "TEST_CODE_GLOBAL_NEWS_EASTMONEY_20".to_owned(),
            blocker: String::new(),
            diagnostic_available: true,
        };
        let semantic_search = Capability {
            operation: Operation::SemanticSearch as i32,
            repository_admission: AdmissionState::Unadmitted as i32,
            runtime_available: false,
            provider: "Bocha".to_owned(),
            exact_scope: "TEST_CODE_UNDELIVERED_SEMANTIC_SEARCH".to_owned(),
            blocker: "TEST_CODE_CAPABILITY_BLOCKED".to_owned(),
            diagnostic_available: false,
        };
        let capabilities = match capabilities_reply {
            CapabilitiesReply::MissingGlobalNews => vec![semantic_search],
            CapabilitiesReply::GlobalNewsUnadmitted => vec![
                Capability {
                    repository_admission: AdmissionState::Unadmitted as i32,
                    ..global_news
                },
                semantic_search,
            ],
            CapabilitiesReply::GlobalNewsRuntimeUnavailable => vec![
                Capability {
                    runtime_available: false,
                    ..global_news
                },
                semantic_search,
            ],
            CapabilitiesReply::Success | CapabilitiesReply::MismatchedId => {
                vec![global_news, semantic_search]
            }
            CapabilitiesReply::Status(_) => unreachable!("handled above"),
        };
        let response = CapabilitiesResponse {
            request_id: if capabilities_reply == CapabilitiesReply::MismatchedId {
                "TEST_CODE_WRONG_CAPABILITIES_REQUEST_ID".to_owned()
            } else {
                request_id
            },
            capabilities,
        };
        self.state
            .lock()
            .expect("TEST_CODE External control Capabilities response")
            .observation
            .capabilities_responses
            .push(response.encode_to_vec());
        Ok(Response::new(response))
    }
}

#[tonic::async_trait]
impl MarketEventService for ExternalControlService {
    type SubscribeStream =
        tokio_stream::wrappers::ReceiverStream<Result<MarketEventEnvelope, Status>>;
    type ReplayStream = tokio_stream::wrappers::ReceiverStream<Result<MarketEventEnvelope, Status>>;

    async fn subscribe(
        &self,
        request: Request<SubscribeRequest>,
    ) -> Result<Response<Self::SubscribeStream>, Status> {
        let authorized = request
            .metadata()
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            == Some("Bearer TEST_CODE_EXTERNAL_CONTROL_TOKEN");
        let request = request.into_inner();
        {
            let mut state = self
                .state
                .lock()
                .expect("TEST_CODE External Subscribe request capture");
            state
                .observation
                .subscribe_requests
                .push(request.encode_to_vec());
            state.observation.subscribe_authorized.push(authorized);
        }
        if !authorized {
            return Err(Status::unauthenticated(
                "TEST_CODE External Subscribe bearer required",
            ));
        }
        request
            .context
            .as_ref()
            .filter(|context| context.protocol_version == 1 && !context.request_id.is_empty())
            .ok_or_else(|| Status::invalid_argument("TEST_CODE invalid Subscribe context"))?;
        let event = MarketEventEnvelope {
            protocol_version: 1,
            event_id: "TEST_CODE_EXTERNAL_EVENT_41".to_owned(),
            cursor: Some(EventCursor {
                generation: "TEST_CODE_EXTERNAL_GENERATION".to_owned(),
                sequence: 41,
            }),
            event_kind: "price".to_owned(),
            provider: "TDX".to_owned(),
            instrument: "EQUITY:SH:600396".to_owned(),
            observed_at: "2026-09-17T09:31:00+08:00".to_owned(),
            source_at: String::new(),
            admission: AdmissionState::Admitted as i32,
            payload: Some(CanonicalPayload {
                schema: "magic.market.event.price".to_owned(),
                schema_version: 1,
                content_type: "application/json; charset=utf-8".to_owned(),
                data: br#"{"instrument":"EQUITY:SH:600396","price":"17.28"}"#.to_vec(),
            }),
        };
        self.state
            .lock()
            .expect("TEST_CODE External Subscribe event capture")
            .observation
            .subscribe_events
            .push(event.encode_to_vec());
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        sender
            .send(Ok(event))
            .await
            .map_err(|_| Status::cancelled("TEST_CODE External Subscribe receiver dropped"))?;
        drop(sender);
        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
            receiver,
        )))
    }

    async fn replay(
        &self,
        request: Request<ReplayRequest>,
    ) -> Result<Response<Self::ReplayStream>, Status> {
        let request = request.into_inner();
        self.state
            .lock()
            .expect("TEST_CODE External Replay request capture")
            .observation
            .replay_requests
            .push(request.encode_to_vec());
        Err(Status::unimplemented(
            "TEST_CODE External event Replay is outside Listener RED",
        ))
    }

    async fn get_listener_status(
        &self,
        request: Request<ListenerStatusRequest>,
    ) -> Result<Response<ListenerStatusResponse>, Status> {
        let authorized = request
            .metadata()
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            == Some("Bearer TEST_CODE_EXTERNAL_CONTROL_TOKEN");
        let request = request.into_inner();
        {
            let mut state = self
                .state
                .lock()
                .expect("TEST_CODE External Listener request capture");
            state
                .observation
                .listener_status_requests
                .push(request.encode_to_vec());
            state
                .observation
                .listener_status_authorized
                .push(authorized);
        }
        if !authorized {
            return Err(Status::unauthenticated(
                "TEST_CODE External Listener bearer required",
            ));
        }
        let request_id = request
            .context
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("TEST_CODE missing Listener context"))?
            .request_id
            .clone();
        let response = ListenerStatusResponse {
            request_id,
            state: "agent_connected_production".to_owned(),
            terminal_generation: "TEST_CODE_EXTERNAL_GENERATION".to_owned(),
            latest: Some(EventCursor {
                generation: "TEST_CODE_EXTERNAL_GENERATION".to_owned(),
                sequence: 44,
            }),
            capabilities: Vec::new(),
            desired_watchlist_revision: 7,
            desired_instruments: vec!["EQUITY:SH:600396".to_owned()],
            applied_watchlist_revision: 7,
            applied_instruments: vec!["EQUITY:SH:600396".to_owned()],
            maximum_watchlist_instruments: 128,
            admitted_event_families: vec!["price".to_owned(), "analysis".to_owned()],
            replay_oldest: Some(EventCursor {
                generation: "TEST_CODE_EXTERNAL_GENERATION".to_owned(),
                sequence: 4,
            }),
            replay_event_count: 41,
            replay_bytes: 4_096,
            active_subscribers: 3,
            agent_connections_total: 11,
            agent_disconnects_total: 2,
            events_published_total: 44,
            replay_evictions_total: 1,
        };
        self.state
            .lock()
            .expect("TEST_CODE External Listener response capture")
            .observation
            .listener_status_responses
            .push(response.encode_to_vec());
        Ok(Response::new(response))
    }

    async fn set_watchlist(
        &self,
        request: Request<SetWatchlistRequest>,
    ) -> Result<Response<SetWatchlistResponse>, Status> {
        let authorized = request
            .metadata()
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            == Some("Bearer TEST_CODE_EXTERNAL_CONTROL_TOKEN");
        let request = request.into_inner();
        {
            let mut state = self
                .state
                .lock()
                .expect("TEST_CODE External SetWatchlist request capture");
            state
                .observation
                .watchlist_requests
                .push(request.encode_to_vec());
            state.observation.watchlist_authorized.push(authorized);
        }
        if !authorized {
            return Err(Status::unauthenticated(
                "TEST_CODE External SetWatchlist bearer required",
            ));
        }
        let request_id = request
            .context
            .as_ref()
            .filter(|context| context.protocol_version == 1 && !context.request_id.is_empty())
            .ok_or_else(|| Status::invalid_argument("TEST_CODE invalid SetWatchlist context"))?
            .request_id
            .clone();
        let watchlist_reply = self
            .state
            .lock()
            .expect("TEST_CODE External SetWatchlist reply mode")
            .watchlist_reply;
        if watchlist_reply == WatchlistReply::UnavailableStatus {
            let detail = ErrorDetail {
                request_id,
                provider: "Tdx".to_owned(),
                reason_code: "unavailable".to_owned(),
                retryable: true,
                ..ErrorDetail::default()
            }
            .encode_to_vec();
            self.state
                .lock()
                .expect("TEST_CODE External SetWatchlist status capture")
                .observation
                .watchlist_status_details
                .push(detail.clone());
            return Err(Status::with_details(
                tonic::Code::Unavailable,
                "TEST_CODE SetWatchlist agent unavailable",
                detail.into(),
            ));
        }
        let response = SetWatchlistResponse {
            request_id,
            desired_revision: 8,
            state: "restarting".to_owned(),
            instruments: request.instruments,
        };
        self.state
            .lock()
            .expect("TEST_CODE External SetWatchlist response capture")
            .observation
            .watchlist_responses
            .push(response.encode_to_vec());
        Ok(Response::new(response))
    }
}

macro_rules! external_control_data_service {
    ($($method:ident),* $(,)?) => {
        #[tonic::async_trait]
        impl MarketDataService for ExternalControlService {
            async fn global_news(
                &self,
                request: Request<QueryRequest>,
            ) -> Result<Response<QueryResponse>, Status> {
                let authorized = request
                    .metadata()
                    .get("authorization")
                    .and_then(|value| value.to_str().ok())
                    == Some("Bearer TEST_CODE_EXTERNAL_CONTROL_TOKEN");
                let request = request.into_inner();
                let data_reply = {
                    let mut state = self
                        .state
                        .lock()
                        .expect("TEST_CODE External GlobalNews capture");
                    state.observation.data_calls += 1;
                    state.observation.data_authorized.push(authorized);
                    state
                        .observation
                        .data_methods
                        .push("global_news".to_owned());
                    state
                        .observation
                        .data_requests
                        .push(request.encode_to_vec());
                    match (state.data_reply, state.observation.data_calls) {
                        (DataReply::RetryThenSuccess, 1) => DataReply::UnavailableStatus,
                        (DataReply::RetryThenSuccess, _) => DataReply::Success,
                        (reply, _) => reply,
                    }
                };
                if data_reply == DataReply::Reject {
                    return Err(Status::failed_precondition(
                        "TEST_CODE Health test must not call data RPC",
                    ));
                }
                if !authorized {
                    return Err(Status::unauthenticated(
                        "TEST_CODE External control bearer required",
                    ));
                }
                let permit = self
                    .data_release
                    .acquire()
                    .await
                    .map_err(|_| Status::cancelled("TEST_CODE External control fixture closing"))?;
                permit.forget();
                let request_id = request
                    .context
                    .as_ref()
                    .ok_or_else(|| Status::invalid_argument("TEST_CODE missing data context"))?
                    .request_id
                    .clone();
                if let DataReply::ProviderAttemptsStatus {
                    unpublished_provider,
                } = data_reply
                {
                    let details = ErrorDetail {
                        request_id,
                        operation: Operation::GlobalNews as i32,
                        provider: "Eastmoney".to_owned(),
                        reason_code: "invalid_evidence".to_owned(),
                        retryable: false,
                        admission: AdmissionState::Admitted as i32,
                        provider_attempts: vec![
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
                        ..ErrorDetail::default()
                    }
                    .encode_to_vec();
                    let status = Status::with_details(
                        tonic::Code::FailedPrecondition,
                        "TEST_CODE External provider attempts",
                        details.into(),
                    );
                    let trailer = match status.metadata().get_bin("magic-error-detail-bin") {
                        None => ObservedHealthTrailer::Absent,
                        Some(value) => match value.to_bytes() {
                            Ok(bytes) => ObservedHealthTrailer::Bytes(bytes.to_vec()),
                            Err(_) => ObservedHealthTrailer::Malformed,
                        },
                    };
                    self.state
                        .lock()
                        .expect("TEST_CODE External provider attempts status")
                        .observation
                        .data_statuses
                        .push(ObservedHealthStatus {
                            code: status.code() as i32,
                            details: status.details().to_vec(),
                            trailer,
                        });
                    return Err(status);
                }
                if data_reply == DataReply::UnavailableStatus {
                    let details = ErrorDetail {
                        request_id,
                        operation: Operation::GlobalNews as i32,
                        provider: "Eastmoney".to_owned(),
                        reason_code: "no_verified_batch".to_owned(),
                        retryable: true,
                        ..ErrorDetail::default()
                    }
                    .encode_to_vec();
                    let status = Status::with_details(
                        tonic::Code::Unavailable,
                        "TEST_CODE External data unavailable",
                        details.into(),
                    );
                    let trailer = match status.metadata().get_bin("magic-error-detail-bin") {
                        None => ObservedHealthTrailer::Absent,
                        Some(value) => match value.to_bytes() {
                            Ok(bytes) => ObservedHealthTrailer::Bytes(bytes.to_vec()),
                            Err(_) => ObservedHealthTrailer::Malformed,
                        },
                    };
                    self.state
                        .lock()
                        .expect("TEST_CODE External GlobalNews status")
                        .observation
                        .data_statuses
                        .push(ObservedHealthStatus {
                            code: status.code() as i32,
                            details: status.details().to_vec(),
                            trailer,
                        });
                    return Err(status);
                }
                let response = QueryResponse {
                    request_id,
                    operation: Operation::GlobalNews as i32,
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
                        data: br#"{"item_id":"TEST_CODE_EXTERNAL_NEWS_001","title":"TEST_CODE external data title","summary":"TEST_CODE external data summary","content":"TEST_CODE external data content","publisher":"TEST_CODE Eastmoney publisher","url":"https://example.com/TEST_CODE_EXTERNAL_NEWS_001","published_at":"2026-09-14T15:30:00+08:00","instruments":[{"exchange":"Shanghai","code":"TEST_CODE_600001","asset_class":"Equity"}],"topics":["TEST_CODE_external_topic"],"language":"zh-CN","evidence":{"provider":"Eastmoney","source_at":"2026-09-14 15:30","observed_at":"2026-09-14T15:31:00+08:00","batch_id":"TEST_CODE_EXTERNAL_DATA_BATCH"}}"#.to_vec(),
                    }],
                    diagnostic_blocker: String::new(),
                };
                Ok(Response::new(response))
            }

            $(async fn $method(&self, request: Request<QueryRequest>)
                -> Result<Response<QueryResponse>, Status> {
                let authorized = request
                    .metadata()
                    .get("authorization")
                    .and_then(|value| value.to_str().ok())
                    == Some("Bearer TEST_CODE_EXTERNAL_CONTROL_TOKEN");
                let request = request.into_inner();
                let mut state = self.state
                    .lock()
                    .expect("TEST_CODE unexpected External data RPC");
                state.observation.data_calls += 1;
                state.observation.data_authorized.push(authorized);
                state.observation.data_methods.push(stringify!($method).to_owned());
                state.observation.data_requests.push(request.encode_to_vec());
                Err(Status::failed_precondition(
                    "TEST_CODE Health test must not call data RPC",
                ))
            })*
        }
    };
}

external_control_data_service!(
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
    semantic_search,
    current_auction_observations,
    economic_release_observations,
    economic_release_schedule,
);

#[derive(Clone)]
struct ExternalControlDataCodec {
    state: Arc<Mutex<ExternalControlState>>,
}

#[derive(Clone)]
struct ExternalControlDataEncoder {
    state: Arc<Mutex<ExternalControlState>>,
}

impl Codec for ExternalControlDataCodec {
    type Encode = QueryResponse;
    type Decode = QueryRequest;
    type Encoder = ExternalControlDataEncoder;
    type Decoder = tonic_prost::ProstDecoder<QueryRequest>;

    fn encoder(&mut self) -> Self::Encoder {
        ExternalControlDataEncoder {
            state: Arc::clone(&self.state),
        }
    }

    fn decoder(&mut self) -> Self::Decoder {
        tonic_prost::ProstDecoder::new(BufferSettings::default())
    }
}

impl Encoder for ExternalControlDataEncoder {
    type Item = QueryResponse;
    type Error = Status;

    fn encode(
        &mut self,
        item: Self::Item,
        destination: &mut EncodeBuf<'_>,
    ) -> Result<(), Self::Error> {
        let mut payload = item.encode_to_vec();
        let mut state = self
            .state
            .lock()
            .expect("TEST_CODE External GlobalNews response capture");
        if state.append_zero_length_source {
            payload.extend_from_slice(&[0x5a, 0x00]);
        }
        destination.put_slice(&payload);
        state.observation.data_responses.push(payload);
        Ok(())
    }
}

#[derive(Clone)]
struct ExternalControlDataServer {
    generated: MarketDataServiceServer<ExternalControlService>,
    inner: Arc<ExternalControlService>,
}

impl ExternalControlDataServer {
    fn new(service: ExternalControlService) -> Self {
        let inner = Arc::new(service);
        Self {
            generated: MarketDataServiceServer::from_arc(Arc::clone(&inner)),
            inner,
        }
    }
}

impl<B> Service<http::Request<B>> for ExternalControlDataServer
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
        if request.uri().path() != "/magic.market.v1.MarketDataService/GlobalNews" {
            return self.generated.call(request);
        }

        struct GlobalNewsMutationService(Arc<ExternalControlService>);
        impl UnaryService<QueryRequest> for GlobalNewsMutationService {
            type Response = QueryResponse;
            type Future = BoxFuture<tonic::Response<Self::Response>, tonic::Status>;

            fn call(&mut self, request: Request<QueryRequest>) -> Self::Future {
                let inner = Arc::clone(&self.0);
                Box::pin(async move {
                    <ExternalControlService as MarketDataService>::global_news(
                        inner.as_ref(),
                        request,
                    )
                    .await
                })
            }
        }

        let method = GlobalNewsMutationService(Arc::clone(&self.inner));
        let codec = ExternalControlDataCodec {
            state: Arc::clone(&self.inner.state),
        };
        Box::pin(async move {
            let mut grpc = Grpc::new(codec);
            Ok(grpc.unary(method, request).await)
        })
    }
}

impl NamedService for ExternalControlDataServer {
    const NAME: &'static str = "magic.market.v1.MarketDataService";
}

pub(crate) struct ExternalControlLoopbackServer {
    endpoint: String,
    state: Arc<Mutex<ExternalControlState>>,
    health_release: Arc<tokio::sync::Semaphore>,
    capabilities_release: Arc<tokio::sync::Semaphore>,
    data_release: Arc<tokio::sync::Semaphore>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<Result<(), tonic::transport::Error>>>,
}

impl ExternalControlLoopbackServer {
    pub(crate) async fn bind() -> Self {
        Self::bind_with_modes(HealthReply::Success, None, DataReply::Reject).await
    }

    pub(crate) async fn bind_with_health_reply_for_test(health_reply: HealthReply) -> Self {
        Self::bind_with_modes(health_reply, None, DataReply::Reject).await
    }

    pub(crate) async fn bind_with_capabilities_success_for_test() -> Self {
        Self::bind_with_modes(
            HealthReply::Success,
            Some(CapabilitiesReply::Success),
            DataReply::Reject,
        )
        .await
    }

    pub(crate) async fn bind_with_capabilities_reply_for_test(
        capabilities_reply: CapabilitiesReply,
    ) -> Self {
        Self::bind_with_modes(
            HealthReply::Success,
            Some(capabilities_reply),
            DataReply::Reject,
        )
        .await
    }

    pub(crate) async fn bind_with_data_success_for_test() -> Self {
        Self::bind_with_modes(HealthReply::Success, None, DataReply::Success).await
    }

    pub(crate) async fn bind_with_data_status_for_test() -> Self {
        Self::bind_with_modes(HealthReply::Success, None, DataReply::UnavailableStatus).await
    }

    async fn bind_with_modes(
        health_reply: HealthReply,
        capabilities_reply: Option<CapabilitiesReply>,
        data_reply: DataReply,
    ) -> Self {
        let listener = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::net::TcpListener::bind("127.0.0.1:0"),
        )
        .await
        .expect("TEST_CODE External control bind deadline")
        .expect("TEST_CODE External control bind");
        let endpoint = format!(
            "http://{}",
            listener
                .local_addr()
                .expect("TEST_CODE External control address")
        );
        let state = Arc::new(Mutex::new(ExternalControlState {
            health_reply,
            capabilities_reply,
            data_reply,
            ..ExternalControlState::default()
        }));
        let health_release = Arc::new(tokio::sync::Semaphore::new(0));
        let capabilities_release = Arc::new(tokio::sync::Semaphore::new(0));
        let data_release = Arc::new(tokio::sync::Semaphore::new(0));
        let service = ExternalControlService {
            state: Arc::clone(&state),
            health_release: Arc::clone(&health_release),
            capabilities_release: Arc::clone(&capabilities_release),
            data_release: Arc::clone(&data_release),
        };
        let accept_state = Arc::clone(&state);
        let incoming =
            tokio_stream::wrappers::TcpListenerStream::new(listener).map(move |connection| {
                if connection.is_ok() {
                    accept_state
                        .lock()
                        .expect("TEST_CODE External control TCP capture")
                        .observation
                        .tcp_accepts += 1;
                }
                connection
            });
        let (shutdown, receive) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(SystemServiceServer::new(service.clone()))
                .add_service(ExternalControlDataServer::new(service))
                .serve_with_incoming_shutdown(incoming, async move {
                    let _ = receive.await;
                })
                .await
        });
        Self {
            endpoint,
            state,
            health_release,
            capabilities_release,
            data_release,
            shutdown: Some(shutdown),
            task: Some(task),
        }
    }

    pub(crate) fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub(crate) fn snapshot(&self) -> ExternalControlObservation {
        self.state
            .lock()
            .expect("TEST_CODE External control snapshot")
            .observation
            .clone()
    }

    pub(crate) fn release_health(&self) {
        self.health_release.add_permits(1);
    }

    pub(crate) fn release_capabilities(&self) {
        self.capabilities_release.add_permits(1);
    }

    pub(crate) fn release_data(&self) {
        self.data_release.add_permits(1);
    }

    pub(crate) async fn finish(mut self) -> Result<(), String> {
        self.health_release.close();
        self.capabilities_release.close();
        self.data_release.close();
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(mut task) = self.task.take() {
            match tokio::time::timeout(Duration::from_secs(5), &mut task).await {
                Ok(Ok(Ok(()))) => {}
                Ok(result) => {
                    return Err(format!(
                        "TEST_CODE External control server failed: {result:?}"
                    ));
                }
                Err(_) => {
                    task.abort();
                    let _ = task.await;
                    return Err(
                        "TEST_CODE External control shutdown timeout; aborted and joined"
                            .to_owned(),
                    );
                }
            }
        }
        Ok(())
    }
}

impl Drop for ExternalControlLoopbackServer {
    fn drop(&mut self) {
        self.health_release.close();
        self.capabilities_release.close();
        self.data_release.close();
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

pub(crate) const TEST_CODE_MTLS_CA_CERT: &[u8] =
    include_bytes!("testdata/external_mtls/ca-cert.pem");
pub(crate) const TEST_CODE_MTLS_SERVER_CERT: &[u8] =
    include_bytes!("testdata/external_mtls/server-cert.pem");
pub(crate) const TEST_CODE_MTLS_SERVER_KEY: &[u8] =
    include_bytes!("testdata/external_mtls/server-key.pem");
const TEST_CODE_MTLS_CLIENT_CERT: &[u8] = include_bytes!("testdata/external_mtls/client-cert.pem");
const TEST_CODE_MTLS_CLIENT_KEY: &[u8] = include_bytes!("testdata/external_mtls/client-key.pem");

pub(crate) fn write_test_code_bundle(
    parent: &Path,
    directory: &str,
    endpoint: &str,
    tls_server_name: &str,
) -> Result<PathBuf, String> {
    let root = parent.join(directory);
    std::fs::create_dir(&root)
        .map_err(|error| format!("TEST_CODE create {directory} bundle: {error}"))?;
    for (name, bytes) in [
        ("ca.pem", TEST_CODE_MTLS_CA_CERT),
        ("certificate.pem", TEST_CODE_MTLS_CLIENT_CERT),
        ("private-key.pem", TEST_CODE_MTLS_CLIENT_KEY),
        (
            "bearer-token.txt",
            b"TEST_CODE_EXTERNAL_CONTROL_TOKEN\n".as_slice(),
        ),
    ] {
        std::fs::write(root.join(name), bytes)
            .map_err(|error| format!("TEST_CODE write {directory}/{name}: {error}"))?;
    }
    let manifest = format!(
        concat!(
            "{{\n",
            "  \"endpoint\": \"{}\",\n",
            "  \"tls_server_name\": \"{}\",\n",
            "  \"ca\": \"ca.pem\",\n",
            "  \"certificate\": \"certificate.pem\",\n",
            "  \"private_key\": \"private-key.pem\",\n",
            "  \"bearer_token\": \"bearer-token.txt\",\n",
            "  \"protocol_version\": 1\n",
            "}}\n"
        ),
        endpoint, tls_server_name
    );
    std::fs::write(root.join("connection.json"), manifest.as_bytes())
        .map_err(|error| format!("TEST_CODE write {directory}/connection.json: {error}"))?;
    Ok(root)
}

pub(crate) struct ExternalMtlsMacroFixture {
    bundle_path: PathBuf,
    wrong_name_bundle_path: PathBuf,
    temp_dir: Option<tempfile::TempDir>,
    server: Option<ExternalControlLoopbackServer>,
}

impl ExternalMtlsMacroFixture {
    pub(crate) async fn bind_data_success_for_test() -> Result<Self, String> {
        Self::bind_with_modes_for_test(
            HealthReply::Success,
            Some(CapabilitiesReply::Success),
            DataReply::Success,
        )
        .await
    }

    pub(crate) async fn bind_data_provider_attempts_for_test(
        unpublished_provider: bool,
    ) -> Result<Self, String> {
        Self::bind_with_modes_for_test(
            HealthReply::Success,
            Some(CapabilitiesReply::Success),
            DataReply::ProviderAttemptsStatus {
                unpublished_provider,
            },
        )
        .await
    }

    pub(crate) async fn bind_data_retry_then_success_for_test() -> Result<Self, String> {
        Self::bind_with_modes_for_test(
            HealthReply::Success,
            Some(CapabilitiesReply::Success),
            DataReply::RetryThenSuccess,
        )
        .await
    }

    pub(crate) async fn bind_data_retry_then_zero_length_success_for_test() -> Result<Self, String>
    {
        let fixture = Self::bind_with_modes_for_test(
            HealthReply::Success,
            Some(CapabilitiesReply::Success),
            DataReply::RetryThenSuccess,
        )
        .await?;
        fixture
            .server
            .as_ref()
            .expect("TEST_CODE zero-length External server owner")
            .state
            .lock()
            .expect("TEST_CODE zero-length External response mode")
            .append_zero_length_source = true;
        Ok(fixture)
    }

    pub(crate) async fn bind_health_not_ready_for_test() -> Result<Self, String> {
        Self::bind_with_modes_for_test(
            HealthReply::NotReady,
            Some(CapabilitiesReply::Success),
            DataReply::Success,
        )
        .await
    }

    pub(crate) async fn bind_health_reply_for_test(
        health_reply: HealthReply,
    ) -> Result<Self, String> {
        Self::bind_with_modes_for_test(
            health_reply,
            Some(CapabilitiesReply::Success),
            DataReply::Success,
        )
        .await
    }

    pub(crate) async fn bind_capabilities_reply_for_test(
        capabilities_reply: CapabilitiesReply,
    ) -> Result<Self, String> {
        Self::bind_with_modes_for_test(
            HealthReply::Success,
            Some(capabilities_reply),
            DataReply::Success,
        )
        .await
    }

    async fn bind_with_modes_for_test(
        health_reply: HealthReply,
        capabilities_reply: Option<CapabilitiesReply>,
        data_reply: DataReply,
    ) -> Result<Self, String> {
        let listener = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::net::TcpListener::bind("127.0.0.1:0"),
        )
        .await
        .map_err(|_| "TEST_CODE mTLS bind deadline".to_owned())?
        .map_err(|error| format!("TEST_CODE mTLS bind: {error}"))?;
        let endpoint = format!(
            "https://{}",
            listener
                .local_addr()
                .map_err(|error| format!("TEST_CODE mTLS address: {error}"))?
        );
        let temp_dir = tempfile::Builder::new()
            .prefix("TEST_CODE-external-mtls-")
            .tempdir()
            .map_err(|error| format!("TEST_CODE mTLS tempdir: {error}"))?;
        let bundle_path =
            write_test_code_bundle(temp_dir.path(), "valid", &endpoint, "macro.test.invalid")?;
        let wrong_name_bundle_path = write_test_code_bundle(
            temp_dir.path(),
            "wrong-name",
            &endpoint,
            "wrong-name.invalid",
        )?;
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
            .map_err(|error| format!("TEST_CODE mTLS server config: {error}"))?;
        let state = Arc::new(Mutex::new(ExternalControlState {
            health_reply,
            capabilities_reply,
            data_reply,
            ..ExternalControlState::default()
        }));
        let health_release = Arc::new(tokio::sync::Semaphore::new(0));
        let capabilities_release = Arc::new(tokio::sync::Semaphore::new(0));
        let data_release = Arc::new(tokio::sync::Semaphore::new(0));
        let service = ExternalControlService {
            state: Arc::clone(&state),
            health_release: Arc::clone(&health_release),
            capabilities_release: Arc::clone(&capabilities_release),
            data_release: Arc::clone(&data_release),
        };
        let accept_state = Arc::clone(&state);
        let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener).filter_map(
            move |connection| match connection {
                Ok(connection) => {
                    let reject = {
                        let mut state = accept_state.lock().expect("TEST_CODE mTLS TCP capture");
                        state.observation.tcp_accepts += 1;
                        state.reject_new_connections
                    };
                    (!reject).then_some(Ok(connection))
                }
                Err(error) => Some(Err(error)),
            },
        );
        let (shutdown, receive) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            builder
                .add_service(SystemServiceServer::new(service.clone()))
                .add_service(MarketEventServiceServer::new(service.clone()))
                .add_service(ExternalControlDataServer::new(service))
                .serve_with_incoming_shutdown(incoming, async move {
                    let _ = receive.await;
                })
                .await
        });
        let server = ExternalControlLoopbackServer {
            endpoint,
            state,
            health_release,
            capabilities_release,
            data_release,
            shutdown: Some(shutdown),
            task: Some(task),
        };
        Ok(Self {
            bundle_path,
            wrong_name_bundle_path,
            temp_dir: Some(temp_dir),
            server: Some(server),
        })
    }

    pub(crate) fn bundle_path(&self) -> &Path {
        &self.bundle_path
    }

    pub(crate) fn wrong_name_bundle_path(&self) -> &Path {
        &self.wrong_name_bundle_path
    }

    pub(crate) fn endpoint(&self) -> &str {
        self.server
            .as_ref()
            .expect("TEST_CODE mTLS server owner")
            .endpoint()
    }

    pub(crate) fn snapshot(&self) -> ExternalControlObservation {
        self.server
            .as_ref()
            .expect("TEST_CODE mTLS server owner")
            .snapshot()
    }

    pub(crate) fn set_reject_new_connections_for_test(&self, reject: bool) {
        self.server
            .as_ref()
            .expect("TEST_CODE mTLS server owner")
            .state
            .lock()
            .expect("TEST_CODE mTLS reject-new-connections mode")
            .reject_new_connections = reject;
    }

    pub(crate) fn release_health(&self) {
        self.server
            .as_ref()
            .expect("TEST_CODE mTLS server owner")
            .release_health();
    }

    pub(crate) fn release_capabilities(&self) {
        self.server
            .as_ref()
            .expect("TEST_CODE mTLS server owner")
            .release_capabilities();
    }

    pub(crate) fn release_data(&self) {
        self.server
            .as_ref()
            .expect("TEST_CODE mTLS server owner")
            .release_data();
    }

    pub(crate) fn set_watchlist_unavailable_for_test(&self) {
        self.server
            .as_ref()
            .expect("TEST_CODE mTLS server owner")
            .state
            .lock()
            .expect("TEST_CODE mTLS SetWatchlist reply mode")
            .watchlist_reply = WatchlistReply::UnavailableStatus;
    }

    pub(crate) async fn finish(mut self) -> Result<(), String> {
        let mut errors = Vec::new();
        if let Some(server) = self.server.take() {
            if let Err(error) = server.finish().await {
                errors.push(error);
            }
        }
        if let Some(temp_dir) = self.temp_dir.take() {
            if let Err(error) = temp_dir.close() {
                errors.push(format!("TEST_CODE mTLS tempdir close failed: {error}"));
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
}
