//! Independent External generated data fixture with a test-only protobuf body mutation.

use super::external_control_loopback_fixture::{
    test_external_build_identity, test_external_observability, write_test_code_bundle,
    TEST_CODE_MTLS_CA_CERT, TEST_CODE_MTLS_SERVER_CERT, TEST_CODE_MTLS_SERVER_KEY,
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
const TEST_AUCTION_RECORD_DATA: &[u8] = r#"{"instrument":{"exchange":"Shanghai","code":"600519","asset_class":"Equity"},"name":"贵州茅台","requested_stage":"live","auction_phase":"matching","data_status":"live","auction_price":null,"pre_close_price":1316.01,"auction_pct":null,"auction_volume_shares":0.0,"auction_amount":0.0,"auction_unmatched":-321.0,"auction_turnover_pct":null,"auction_volume_ratio":null,"auction_yesterday_ratio_pct":null,"float_market_cap":1653000000000.0,"last_price":null,"open_price":null,"evidence":{"provider":"Tonghuashun","source_at":null,"observed_at":"unix-ms:1788956044416","batch_id":"TEST_CODE_HITHINK_AUCTION_BATCH"}}"#.as_bytes();
const TEST_RELEASE_RECORD_DATA: &[u8] = r#"{"event_id":"202607250001","indicator_id":950,"country":"中国","name":"规模以上工业企业利润","period":"6月","scheduled_at":"2026-07-25T09:30:00+08:00","released_at":"2026-07-25T09:30:01+08:00","previous":"-9.1","consensus":null,"actual":"0","revised":null,"unit":"%","importance":3,"impact":"1","evidence":{"provider":"Jin10","source_at":"2026-07-25 09:30:01","observed_at":"1784943002.000000000","batch_id":"TEST_CODE_JIN10_RELEASE_BATCH"}}"#.as_bytes();
const TEST_CFFEX_DELIVERY_BATCH: &str = "cffex-equity-index-planned-delivery-2026-v2:09";
const TEST_CFFEX_DELIVERY_OBSERVED_AT: &str = "1790478510.469882800";
const TEST_CFFEX_HOLIDAY_URL: &str =
    "https://www.gov.cn/gongbao/2025/issue_12406/material/gwygb202532.pdf";

fn test_futures_delivery_record_data(product: &str, contract_code: &str) -> Vec<u8> {
    let rule_url = match product {
        "If" => "https://www.cffex.com.cn/cn/hs300.html",
        "Ih" => "https://www.cffex.com.cn/cn/sz50gzqh.html",
        "Ic" => "https://www.cffex.com.cn/cn/zz500.html",
        "Im" => "https://www.cffex.com.cn/zz1000/",
        _ => unreachable!("closed CFFEX fixture products"),
    };
    serde_json::to_vec(&serde_json::json!({
        "product": product,
        "contract_code": contract_code,
        "last_trading_date": "2026-09-18",
        "delivery_date": "2026-09-18",
        "method": "Cash",
        "schedule_status": "Planned",
        "date_basis": "CffexRuleAndPublishedHolidays",
        "rule_url": rule_url,
        "holiday_calendar_url": TEST_CFFEX_HOLIDAY_URL,
        "evidence": {
            "provider": "Cffex",
            "source_at": null,
            "observed_at": TEST_CFFEX_DELIVERY_OBSERVED_AT,
            "batch_id": TEST_CFFEX_DELIVERY_BATCH
        }
    }))
    .expect("TEST_CODE CFFEX delivery record JSON")
}

fn test_schedule_record_data(release_date: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "release_id":10,
        "release_name":"Consumer Price Index",
        "release_date":release_date,
        "release_last_updated":"2026-08-01 09:30:00-05",
        "evidence":{
            "provider":"Fred", "source_at":null,
            "observed_at":"1789257600.000000000",
            "batch_id":"TEST_CODE_FRED_SCHEDULE_BATCH"
        }
    }))
    .expect("TEST_CODE schedule record JSON")
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ExternalQueryWireObservation {
    pub(crate) tcp_accepts: usize,
    pub(crate) health_calls: usize,
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
    FlowUnavailableStatus,
    FlowIncomplete,
    FlowRecordUnavailable,
    Historical(HistoricalQueryReply),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HistoricalQueryReply {
    Success,
    Incomplete,
    Unadmitted,
    ProviderMismatch,
    OperationMismatch,
    RequestIdMismatch,
    StatusWithTrailer,
    StatusMalformedTrailer(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HistoricalCapabilityBehavior {
    Ready,
    Missing,
    Duplicate,
    Unadmitted,
    Unavailable,
    Blocked,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ExternalCapabilitiesBehavior {
    #[default]
    Stable,
    CatalogLifecycle,
    FixedCatalog {
        provider: &'static str,
    },
    Auction,
    ReleaseObservations,
    ReleaseSchedule,
    FuturesDelivery,
    Flows,
    Historical(HistoricalCapabilityBehavior),
}

#[derive(Default)]
struct ExternalQueryWireState {
    observation: ExternalQueryWireObservation,
    reply: ExternalQueryWireReply,
    capabilities_behavior: ExternalCapabilitiesBehavior,
    invalid_health_identity: bool,
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
        request: Request<HealthRequest>,
    ) -> Result<Response<HealthResponse>, Status> {
        let authorized = request
            .metadata()
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            == Some(TEST_AUTHORIZATION);
        let request = request.into_inner();
        let invalid_health_identity = {
            let mut state = self.state.lock().expect("TEST_CODE auction Health state");
            state.observation.health_calls += 1;
            state.invalid_health_identity
        };
        if !authorized {
            return Err(Status::unauthenticated(
                "TEST_CODE auction Health bearer required",
            ));
        }
        let request_id = request
            .context
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("TEST_CODE auction Health context missing"))?
            .request_id
            .clone();
        let mut build_identity = test_external_build_identity();
        if invalid_health_identity {
            build_identity.binary_sha256 = "f".repeat(64);
        }
        Ok(Response::new(HealthResponse {
            request_id,
            live: true,
            ready: true,
            state: "TEST_CODE_AUCTION_READY".to_owned(),
            observability: Some(test_external_observability()),
            build_identity: Some(build_identity),
        }))
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
            ExternalCapabilitiesBehavior::Auction => CapabilitiesResponse {
                request_id,
                capabilities: vec![Capability {
                    operation: Operation::CurrentAuctionObservations as i32,
                    repository_admission: AdmissionState::Admitted as i32,
                    runtime_available: true,
                    provider: "HithinkFinance".to_owned(),
                    exact_scope: "TEST_CODE_LIVE_AUCTION".to_owned(),
                    blocker: String::new(),
                    diagnostic_available: true,
                }],
            },
            ExternalCapabilitiesBehavior::ReleaseObservations => CapabilitiesResponse {
                request_id,
                capabilities: vec![Capability {
                    operation: Operation::EconomicReleaseObservations as i32,
                    repository_admission: AdmissionState::Admitted as i32,
                    runtime_available: true,
                    provider: "Jin10".to_owned(),
                    exact_scope: "TEST_CODE_ROLLING_RELEASE_WINDOW".to_owned(),
                    blocker: String::new(),
                    diagnostic_available: true,
                }],
            },
            ExternalCapabilitiesBehavior::ReleaseSchedule => CapabilitiesResponse {
                request_id,
                capabilities: vec![Capability {
                    operation: Operation::EconomicReleaseSchedule as i32,
                    repository_admission: AdmissionState::Admitted as i32,
                    runtime_available: true,
                    provider: "Fred".to_owned(),
                    exact_scope: "TEST_CODE_FRED_DATE_RANGE".to_owned(),
                    blocker: String::new(),
                    diagnostic_available: true,
                }],
            },
            ExternalCapabilitiesBehavior::FuturesDelivery => CapabilitiesResponse {
                request_id,
                capabilities: vec![Capability {
                    operation: Operation::FuturesDelivery as i32,
                    repository_admission: AdmissionState::Admitted as i32,
                    runtime_available: true,
                    provider: "Cffex".to_owned(),
                    exact_scope: "TEST_CODE_CFFEX_DELIVERY_2026_09".to_owned(),
                    blocker: String::new(),
                    diagnostic_available: true,
                }],
            },
            ExternalCapabilitiesBehavior::Flows => CapabilitiesResponse {
                request_id,
                capabilities: [Operation::MoneyFlows, Operation::BoardFlows]
                    .into_iter()
                    .map(|operation| Capability {
                        operation: operation as i32,
                        repository_admission: AdmissionState::Admitted as i32,
                        runtime_available: true,
                        provider: "Eastmoney".to_owned(),
                        exact_scope: "TEST_CODE_FLOW_READ".to_owned(),
                        blocker: String::new(),
                        diagnostic_available: false,
                    })
                    .collect(),
            },
            ExternalCapabilitiesBehavior::Historical(behavior) => CapabilitiesResponse {
                request_id,
                capabilities: historical_capabilities(behavior),
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

fn historical_capabilities(behavior: HistoricalCapabilityBehavior) -> Vec<Capability> {
    let mut capability = Capability {
        operation: Operation::HistoricalBars as i32,
        repository_admission: AdmissionState::Admitted as i32,
        runtime_available: true,
        provider: "HithinkFinance".to_owned(),
        exact_scope: "TEST_CODE_HISTORICAL_EXPLICIT_DAY_WINDOW".to_owned(),
        blocker: String::new(),
        diagnostic_available: false,
    };
    match behavior {
        HistoricalCapabilityBehavior::Ready => vec![capability],
        HistoricalCapabilityBehavior::Missing => vec![],
        HistoricalCapabilityBehavior::Duplicate => vec![capability.clone(), capability],
        HistoricalCapabilityBehavior::Unadmitted => {
            capability.repository_admission = AdmissionState::Unadmitted as i32;
            vec![capability]
        }
        HistoricalCapabilityBehavior::Unavailable => {
            capability.runtime_available = false;
            vec![capability]
        }
        HistoricalCapabilityBehavior::Blocked => {
            capability.blocker = "TEST_CODE contradictory HistoricalBars capability".to_owned();
            vec![capability]
        }
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
        if let ExternalQueryWireReply::Historical(reply) = reply {
            let payload = request
                .payload
                .as_ref()
                .ok_or_else(|| Status::invalid_argument("TEST_CODE historical payload missing"))?;
            let body: serde_json::Value = serde_json::from_slice(&payload.data)
                .map_err(|_| Status::invalid_argument("TEST_CODE historical JSON invalid"))?;
            if operation != Operation::HistoricalBars
                || payload.schema != "magic.market.historical_bars.request"
                || payload.schema_version != 1
                || payload.content_type != "application/json; charset=utf-8"
                || requested_provider != "HithinkFinance"
                || request.allow_unadmitted
                || body
                    != serde_json::json!({
                        "instrument":{"exchange":"Shanghai","code":"600519","asset_class":"Equity"},
                        "interval":"Day","start":"2026-09-11","end":"2026-09-15","limit":3
                    })
            {
                return Err(Status::invalid_argument(
                    "TEST_CODE historical request contract",
                ));
            }
            if matches!(
                reply,
                HistoricalQueryReply::StatusWithTrailer
                    | HistoricalQueryReply::StatusMalformedTrailer(_)
            ) {
                let details = ErrorDetail {
                    request_id,
                    operation: Operation::HistoricalBars as i32,
                    provider: "HithinkFinance".to_owned(),
                    reason_code: "unavailable".to_owned(),
                    retryable: true,
                    admission: AdmissionState::Admitted as i32,
                    ..ErrorDetail::default()
                }
                .encode_to_vec();
                let mut status = Status::with_details(
                    tonic::Code::Unavailable,
                    "TEST_CODE historical provider unavailable",
                    details.clone().into(),
                );
                if let HistoricalQueryReply::StatusMalformedTrailer(encoded) = reply {
                    let mut headers = http::HeaderMap::new();
                    headers.insert(
                        "magic-error-detail-bin",
                        http::HeaderValue::from_static(encoded),
                    );
                    *status.metadata_mut() = tonic::metadata::MetadataMap::from_headers(headers);
                } else {
                    status.metadata_mut().insert_bin(
                        "magic-error-detail-bin",
                        tonic::metadata::MetadataValue::from_bytes(&details),
                    );
                }
                return Err(status);
            }
            return Ok(Response::new(QueryResponse {
                request_id: if reply == HistoricalQueryReply::RequestIdMismatch {
                    "TEST_CODE_DIFFERENT_HISTORICAL_REQUEST".to_owned()
                } else {
                    request_id
                },
                operation: if reply == HistoricalQueryReply::OperationMismatch {
                    Operation::MoneyFlows as i32
                } else {
                    Operation::HistoricalBars as i32
                },
                admission: if reply == HistoricalQueryReply::Unadmitted {
                    AdmissionState::Unadmitted as i32
                } else {
                    AdmissionState::Admitted as i32
                },
                selected_provider: if reply == HistoricalQueryReply::ProviderMismatch {
                    "EmQuant"
                } else {
                    "HithinkFinance"
                }
                .to_owned(),
                batch_id: "TEST_CODE_HISTORICAL_BATCH".to_owned(),
                complete: reply != HistoricalQueryReply::Incomplete,
                observed_at: "2026-09-16T15:31:00+08:00".to_owned(),
                source_at: String::new(),
                // Transport-only opaque bytes intentionally do not invent a
                // provider HistoricalBars record schema or date coverage.
                records: vec![CanonicalPayload {
                    schema: "TEST_CODE_OPAQUE_HISTORICAL_RECORD".to_owned(),
                    schema_version: 1,
                    content_type: "application/octet-stream".to_owned(),
                    data: b"TEST_CODE_RAW_HISTORICAL_RECORD".to_vec(),
                }],
                diagnostic_blocker: String::new(),
            }));
        }
        if reply == ExternalQueryWireReply::FlowUnavailableStatus {
            let details = ErrorDetail {
                request_id,
                operation: operation as i32,
                provider: "Eastmoney".to_owned(),
                reason_code: "unavailable".to_owned(),
                retryable: true,
                admission: AdmissionState::Admitted as i32,
                ..ErrorDetail::default()
            }
            .encode_to_vec();
            return Err(Status::with_details(
                tonic::Code::Unavailable,
                "TEST_CODE flow provider unavailable",
                details.into(),
            ));
        }
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
            ExternalQueryWireReply::FlowUnavailableStatus => unreachable!(),
            ExternalQueryWireReply::FlowIncomplete => None,
            ExternalQueryWireReply::FlowRecordUnavailable => None,
            ExternalQueryWireReply::Historical(_) => unreachable!(),
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
        if matches!(operation, Operation::MoneyFlows | Operation::BoardFlows) {
            let payload = request
                .payload
                .as_ref()
                .ok_or_else(|| Status::invalid_argument("TEST_CODE flow payload missing"))?;
            let body: serde_json::Value = serde_json::from_slice(&payload.data)
                .map_err(|_| Status::invalid_argument("TEST_CODE flow JSON invalid"))?;
            let expected = match operation {
                Operation::MoneyFlows => (
                    "magic.market.money_flows.request",
                    serde_json::json!({"instruments":[{"exchange":"Shanghai","code":"600519","asset_class":"Equity"}]}),
                    "magic.market.money_flow",
                    serde_json::json!({"instrument":{"exchange":"Shanghai","code":"600519","asset_class":"Equity"},"main_net":1.0,"super_large_net":2.0,"large_net":3.0,"medium_net":4.0,"small_net":5.0,"status":"Available","source_at":"2026-09-14","observed_at":"2026-09-14T15:31:00+08:00","provider":"Eastmoney","batch_id":"TEST_CODE_EXTERNAL_DATA_BATCH"}),
                ),
                Operation::BoardFlows => (
                    "magic.market.board_flows.request",
                    serde_json::json!({"category":"Industry","interval":"Day1","limit":2}),
                    "magic.market.board_flow",
                    serde_json::json!({"board_code":"BK0001","board_name":"TEST_CODE board","category":"Industry","interval":"Day1","rank":1,"return_ratio":{"value":1.0,"unit":"Percent"},"main_net":1.0,"super_large_net":2.0,"large_net":3.0,"medium_net":4.0,"small_net":5.0,"leader_instrument":null,"leader_name":null,"leader_return_ratio":null,"evidence":{"provider":"Eastmoney","source_at":"1789371000","observed_at":"2026-09-14T15:31:00+08:00","batch_id":"TEST_CODE_EXTERNAL_DATA_BATCH"}}),
                ),
                _ => unreachable!(),
            };
            if payload.schema != expected.0
                || payload.schema_version != 1
                || requested_provider != "Eastmoney"
                || request.allow_unadmitted
                || body != expected.1
            {
                return Err(Status::invalid_argument("TEST_CODE flow request contract"));
            }
            return Ok(Response::new(QueryResponse {
                request_id,
                operation: operation as i32,
                admission: AdmissionState::Admitted as i32,
                selected_provider: "Eastmoney".to_owned(),
                batch_id: "TEST_CODE_EXTERNAL_DATA_BATCH".to_owned(),
                complete: reply != ExternalQueryWireReply::FlowIncomplete,
                observed_at: "2026-09-14T15:31:00+08:00".to_owned(),
                source_at: if operation == Operation::MoneyFlows {
                    "2026-09-14"
                } else {
                    "1789371000"
                }
                .to_owned(),
                records: vec![CanonicalPayload {
                    schema: expected.2.to_owned(),
                    schema_version: 1,
                    content_type: "application/json; charset=utf-8".to_owned(),
                    data: serde_json::to_vec(&if reply
                        == ExternalQueryWireReply::FlowRecordUnavailable
                        && operation == Operation::MoneyFlows
                    {
                        let mut record = expected.3;
                        record["status"] = serde_json::json!("Unavailable");
                        record
                    } else {
                        expected.3
                    })
                    .expect("TEST_CODE flow record JSON"),
                }],
                diagnostic_blocker: String::new(),
            }));
        }
        if operation == Operation::CurrentAuctionObservations {
            let payload = request
                .payload
                .as_ref()
                .ok_or_else(|| Status::invalid_argument("TEST_CODE auction payload missing"))?;
            if payload.schema != "magic.market.current_auction_observations.request"
                || payload.schema_version != 1
                || requested_provider != "HithinkFinance"
            {
                return Err(Status::invalid_argument(
                    "TEST_CODE auction request contract",
                ));
            }
            return Ok(Response::new(QueryResponse {
                request_id,
                operation: operation as i32,
                admission: AdmissionState::Admitted as i32,
                selected_provider: "HithinkFinance".to_owned(),
                batch_id: "TEST_CODE_HITHINK_AUCTION_BATCH".to_owned(),
                complete: true,
                observed_at: "unix-ms:1788956044416".to_owned(),
                source_at: String::new(),
                records: vec![CanonicalPayload {
                    schema: "magic.market.current_auction_observation".to_owned(),
                    schema_version: 1,
                    content_type: "application/json; charset=utf-8".to_owned(),
                    data: TEST_AUCTION_RECORD_DATA.to_vec(),
                }],
                diagnostic_blocker: String::new(),
            }));
        }
        if operation == Operation::EconomicReleaseObservations {
            let payload = request
                .payload
                .as_ref()
                .ok_or_else(|| Status::invalid_argument("TEST_CODE release payload missing"))?;
            let body: serde_json::Value = serde_json::from_slice(&payload.data)
                .map_err(|_| Status::invalid_argument("TEST_CODE release JSON invalid"))?;
            if payload.schema != "magic.market.economic_release_observations.request"
                || payload.schema_version != 1
                || requested_provider != "Jin10"
                || body != serde_json::json!({"limit":20,"country":"中国"})
            {
                return Err(Status::invalid_argument(
                    "TEST_CODE release request contract",
                ));
            }
            return Ok(Response::new(QueryResponse {
                request_id,
                operation: operation as i32,
                admission: AdmissionState::Admitted as i32,
                selected_provider: "Jin10".to_owned(),
                batch_id: "TEST_CODE_JIN10_RELEASE_BATCH".to_owned(),
                complete: true,
                observed_at: "1784943002.000000000".to_owned(),
                source_at: "2026-07-25 09:30:01".to_owned(),
                records: vec![CanonicalPayload {
                    schema: "magic.market.economic_release_observation".to_owned(),
                    schema_version: 1,
                    content_type: "application/json; charset=utf-8".to_owned(),
                    data: TEST_RELEASE_RECORD_DATA.to_vec(),
                }],
                diagnostic_blocker: String::new(),
            }));
        }
        if operation == Operation::EconomicReleaseSchedule {
            let payload = request
                .payload
                .as_ref()
                .ok_or_else(|| Status::invalid_argument("TEST_CODE schedule payload missing"))?;
            let body: serde_json::Value = serde_json::from_slice(&payload.data)
                .map_err(|_| Status::invalid_argument("TEST_CODE schedule JSON invalid"))?;
            if payload.schema != "magic.market.economic_release_schedule.request"
                || payload.schema_version != 1
                || requested_provider != "Fred"
                || body != serde_json::json!({"start":"2026-09-13","end":"2026-10-13","limit":20})
            {
                return Err(Status::invalid_argument(
                    "TEST_CODE schedule request contract",
                ));
            }
            return Ok(Response::new(QueryResponse {
                request_id,
                operation: operation as i32,
                admission: AdmissionState::Admitted as i32,
                selected_provider: "Fred".to_owned(),
                batch_id: "TEST_CODE_FRED_SCHEDULE_BATCH".to_owned(),
                complete: true,
                observed_at: "1789257600.000000000".to_owned(),
                source_at: String::new(),
                records: ["2026-09-15", "2026-09-16"]
                    .into_iter()
                    .map(|date| CanonicalPayload {
                        schema: "magic.market.economic_release_schedule_entry".to_owned(),
                        schema_version: 1,
                        content_type: "application/json; charset=utf-8".to_owned(),
                        data: test_schedule_record_data(date),
                    })
                    .collect(),
                diagnostic_blocker: String::new(),
            }));
        }
        if operation == Operation::FuturesDelivery {
            let payload = request
                .payload
                .as_ref()
                .ok_or_else(|| Status::invalid_argument("TEST_CODE CFFEX payload missing"))?;
            let body: serde_json::Value = serde_json::from_slice(&payload.data)
                .map_err(|_| Status::invalid_argument("TEST_CODE CFFEX request JSON invalid"))?;
            if payload.schema != "magic.market.futures_delivery.request"
                || payload.schema_version != 2
                || requested_provider != "Cffex"
                || body != serde_json::json!({"year": 2026, "month": 9})
            {
                return Err(Status::invalid_argument("TEST_CODE CFFEX request contract"));
            }
            return Ok(Response::new(QueryResponse {
                request_id,
                operation: operation as i32,
                admission: AdmissionState::Admitted as i32,
                selected_provider: "Cffex".to_owned(),
                batch_id: TEST_CFFEX_DELIVERY_BATCH.to_owned(),
                complete: true,
                observed_at: TEST_CFFEX_DELIVERY_OBSERVED_AT.to_owned(),
                source_at: String::new(),
                records: [
                    ("If", "IF2609"),
                    ("Ih", "IH2609"),
                    ("Ic", "IC2609"),
                    ("Im", "IM2609"),
                ]
                .into_iter()
                .map(|(product, contract_code)| CanonicalPayload {
                    schema: "magic.market.futures_delivery_event".to_owned(),
                    schema_version: 2,
                    content_type: "application/json; charset=utf-8".to_owned(),
                    data: test_futures_delivery_record_data(product, contract_code),
                })
                .collect(),
                diagnostic_blocker: String::new(),
            }));
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
            async fn official_publications(
                &self,
                _request: Request<QueryRequest>,
            ) -> Result<Response<QueryResponse>, Status> {
                Err(Status::unimplemented("TEST_CODE official publications unavailable"))
            }

            async fn official_publication(
                &self,
                _request: Request<QueryRequest>,
            ) -> Result<Response<QueryResponse>, Status> {
                Err(Status::unimplemented("TEST_CODE official publication unavailable"))
            }

            async fn money_flows(
                &self,
                request: Request<QueryRequest>,
            ) -> Result<Response<QueryResponse>, Status> {
                self.respond(request, "money_flows", Operation::MoneyFlows).await
            }

            async fn historical_bars(
                &self,
                request: Request<QueryRequest>,
            ) -> Result<Response<QueryResponse>, Status> {
                self.respond(request, "historical_bars", Operation::HistoricalBars).await
            }

            async fn board_flows(
                &self,
                request: Request<QueryRequest>,
            ) -> Result<Response<QueryResponse>, Status> {
                self.respond(request, "board_flows", Operation::BoardFlows).await
            }

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

            async fn current_auction_observations(
                &self,
                request: Request<QueryRequest>,
            ) -> Result<Response<QueryResponse>, Status> {
                self.respond(request, "current_auction_observations", Operation::CurrentAuctionObservations).await
            }

            async fn economic_release_observations(
                &self,
                request: Request<QueryRequest>,
            ) -> Result<Response<QueryResponse>, Status> {
                self.respond(request, "economic_release_observations", Operation::EconomicReleaseObservations).await
            }

            async fn economic_release_schedule(
                &self,
                request: Request<QueryRequest>,
            ) -> Result<Response<QueryResponse>, Status> {
                self.respond(request, "economic_release_schedule", Operation::EconomicReleaseSchedule).await
            }

            async fn futures_delivery(
                &self,
                request: Request<QueryRequest>,
            ) -> Result<Response<QueryResponse>, Status> {
                self.respond(request, "futures_delivery", Operation::FuturesDelivery).await
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
    minute_data,
    realtime_quotes,
    order_books,
    auctions,
    trades,
    global_indices,
    foreign_exchange,
    economic_calendar,
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
    route: ExternalQueryWireRoute,
}

impl ExternalQueryWireFixture {
    pub(crate) async fn bind_historical(
        reply: HistoricalQueryReply,
        capabilities: HistoricalCapabilityBehavior,
    ) -> Result<Self, String> {
        Self::bind_with_route_reply_and_capabilities(
            ExternalQueryWireRoute::Generated,
            ExternalQueryWireReply::Historical(reply),
            ExternalCapabilitiesBehavior::Historical(capabilities),
        )
        .await
    }

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

    pub(crate) async fn bind_qualified_current_auction() -> Result<Self, String> {
        Self::bind_with_route_reply_and_capabilities(
            ExternalQueryWireRoute::Generated,
            ExternalQueryWireReply::Success,
            ExternalCapabilitiesBehavior::Auction,
        )
        .await
    }

    pub(crate) async fn bind_qualified_release_observations() -> Result<Self, String> {
        Self::bind_with_route_reply_and_capabilities(
            ExternalQueryWireRoute::Generated,
            ExternalQueryWireReply::Success,
            ExternalCapabilitiesBehavior::ReleaseObservations,
        )
        .await
    }

    pub(crate) async fn bind_qualified_release_schedule() -> Result<Self, String> {
        Self::bind_with_route_reply_and_capabilities(
            ExternalQueryWireRoute::Generated,
            ExternalQueryWireReply::Success,
            ExternalCapabilitiesBehavior::ReleaseSchedule,
        )
        .await
    }

    pub(crate) async fn bind_qualified_futures_delivery() -> Result<Self, String> {
        Self::bind_with_route_reply_and_capabilities(
            ExternalQueryWireRoute::Generated,
            ExternalQueryWireReply::Success,
            ExternalCapabilitiesBehavior::FuturesDelivery,
        )
        .await
    }

    pub(crate) async fn bind_qualified_flows() -> Result<Self, String> {
        Self::bind_with_route_reply_and_capabilities(
            ExternalQueryWireRoute::Generated,
            ExternalQueryWireReply::Success,
            ExternalCapabilitiesBehavior::Flows,
        )
        .await
    }

    pub(crate) async fn bind_flow_status() -> Result<Self, String> {
        Self::bind_with_route_reply_and_capabilities(
            ExternalQueryWireRoute::Generated,
            ExternalQueryWireReply::FlowUnavailableStatus,
            ExternalCapabilitiesBehavior::Flows,
        )
        .await
    }

    pub(crate) async fn bind_flow_incomplete() -> Result<Self, String> {
        Self::bind_with_route_reply_and_capabilities(
            ExternalQueryWireRoute::Generated,
            ExternalQueryWireReply::FlowIncomplete,
            ExternalCapabilitiesBehavior::Flows,
        )
        .await
    }

    pub(crate) async fn bind_flow_record_unavailable() -> Result<Self, String> {
        Self::bind_with_route_reply_and_capabilities(
            ExternalQueryWireRoute::Generated,
            ExternalQueryWireReply::FlowRecordUnavailable,
            ExternalCapabilitiesBehavior::Flows,
        )
        .await
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

    pub(crate) async fn bind_catalog_lifecycle_requested_provider() -> Result<Self, String> {
        Self::bind_catalog_scenario(
            ExternalQueryWireReply::CatalogRequestedProviderStatus,
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
        let state = Arc::new(Mutex::new(ExternalQueryWireState {
            reply,
            capabilities_behavior,
            ..ExternalQueryWireState::default()
        }));
        let release = Arc::new(tokio::sync::Semaphore::new(0));
        let capabilities_release = Arc::new(tokio::sync::Semaphore::new(0));
        let (shutdown, task) =
            Self::spawn_server(listener, &state, &release, &capabilities_release, route)?;
        Ok(Self {
            bundle_path,
            endpoint,
            state,
            release,
            capabilities_release,
            shutdown: Some(shutdown),
            task: Some(task),
            temp_dir: Some(temp_dir),
            route,
        })
    }

    fn spawn_server(
        listener: tokio::net::TcpListener,
        state: &Arc<Mutex<ExternalQueryWireState>>,
        release: &Arc<tokio::sync::Semaphore>,
        capabilities_release: &Arc<tokio::sync::Semaphore>,
        route: ExternalQueryWireRoute,
    ) -> Result<
        (
            tokio::sync::oneshot::Sender<()>,
            tokio::task::JoinHandle<Result<(), tonic::transport::Error>>,
        ),
        String,
    > {
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
        let service = ExternalQueryWireService {
            state: Arc::clone(state),
            release: Arc::clone(release),
            capabilities_release: Arc::clone(capabilities_release),
        };
        let accept_state = Arc::clone(state);
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
        Ok((shutdown, task))
    }

    /// Force a real server/TCP shutdown, then reopen the same mTLS endpoint.
    /// Existing clients retain their spent one-dial generation.
    pub(crate) async fn restart_transport(&mut self) -> Result<(), String> {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(mut task) = self.task.take() {
            match tokio::time::timeout(Duration::from_secs(5), &mut task).await {
                Ok(Ok(Ok(()))) => {}
                Ok(result) => return Err(format!("TEST_CODE restart shutdown: {result:?}")),
                Err(_) => {
                    task.abort();
                    let _ = task.await;
                    return Err("TEST_CODE restart TCP shutdown deadline".to_owned());
                }
            }
        }
        let listener = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::net::TcpListener::bind(self.endpoint.trim_start_matches("https://")),
        )
        .await
        .map_err(|_| "TEST_CODE restart bind deadline".to_owned())?
        .map_err(|error| format!("TEST_CODE restart bind: {error}"))?;
        let (shutdown, task) = Self::spawn_server(
            listener,
            &self.state,
            &self.release,
            &self.capabilities_release,
            self.route,
        )?;
        self.shutdown = Some(shutdown);
        self.task = Some(task);
        Ok(())
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

    pub(crate) fn set_invalid_health_identity(&self, invalid: bool) {
        self.state
            .lock()
            .expect("TEST_CODE Health identity control")
            .invalid_health_identity = invalid;
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
