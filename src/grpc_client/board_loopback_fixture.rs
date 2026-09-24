use super::{ClientAuthorization, ContractProfile, GrpcMarketClient};
use crate::grpc_client::pb::magic::market::v1::{
    market_data_service_server::{MarketDataService, MarketDataServiceServer},
    AdmissionState, CanonicalPayload, ErrorDetail, Operation, QueryRequest, QueryResponse,
};
use prost::Message;
use std::sync::{Arc, Mutex};
use tokio_stream::StreamExt as _;
use tonic::{Code, Request, Response, Status};
use zeroize::Zeroizing;

pub(crate) const INDUSTRY_DIRECTORY_BYTES: &[u8] = br#"[
 {"code":"TEST_CODE_BOARD_MAIN","name":"TEST_CODE_CLUSTER_A_MAIN","kind":"Industry","member_count":2},
 {"code":"TEST_CODE_BOARD_INDUSTRY_ONLY","name":"TEST_CODE_INDUSTRY_ONLY","kind":"Industry","member_count":3}
]"#;
pub(crate) const CONCEPT_DIRECTORY_BYTES: &[u8] = br#"[
 {"code":"TEST_CODE_BOARD_MAIN","name":"TEST_CODE_CLUSTER_A_MAIN","kind":"Concept","member_count":2},
 {"code":"TEST_CODE_BOARD_CONCEPT_ONLY","name":"TEST_CODE_CONCEPT_ONLY","kind":"Concept","member_count":4}
]"#;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BoardLoopbackRequest {
    pub(crate) kind: String,
    pub(crate) limit: u64,
    pub(crate) request_id: String,
    pub(crate) protocol_version: u32,
    pub(crate) payload_schema: String,
    pub(crate) payload_schema_version: u32,
    pub(crate) payload_content_type: String,
    pub(crate) allow_unadmitted: bool,
    pub(crate) authorized: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BoardMembershipLoopbackRequest {
    pub(crate) codes: Vec<String>,
    pub(crate) request_id: String,
    pub(crate) protocol_version: u32,
    pub(crate) payload_schema: String,
    pub(crate) payload_schema_version: u32,
    pub(crate) payload_content_type: String,
    pub(crate) preferred_provider: String,
    pub(crate) allow_unadmitted: bool,
    pub(crate) authorized: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DragonTigerLoopbackRequest {
    pub(crate) date: String,
    pub(crate) disclosure_limit: u64,
    pub(crate) stock_limit: u64,
    pub(crate) request_id: String,
    pub(crate) request_bytes: Vec<u8>,
    pub(crate) payload_schema: String,
    pub(crate) payload_schema_version: u32,
    pub(crate) payload_content_type: String,
    pub(crate) preferred_provider: String,
    pub(crate) allow_unadmitted: bool,
    pub(crate) authorized: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BoardLoopbackObservation {
    pub(crate) requests: Vec<BoardLoopbackRequest>,
    pub(crate) dragon_tiger_requests: Vec<DragonTigerLoopbackRequest>,
    pub(crate) retry_error_detail: Vec<u8>,
    pub(crate) non_board_requests: usize,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct MacroLoopbackObservation {
    pub(crate) requests: Vec<(Vec<u8>, bool)>,
    pub(crate) routed_operations: Vec<Operation>,
    pub(crate) retry_error_details: Vec<Vec<u8>>,
    pub(crate) response_bytes: Vec<Vec<u8>>,
    pub(crate) raw_statuses: Vec<MacroLoopbackStatus>,
}

#[derive(Clone, Debug)]
pub(crate) struct MacroLoopbackStatus {
    pub(crate) code: i32,
    pub(crate) details: Vec<u8>,
    pub(crate) trailer_header: Option<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MacroRawStatusCase {
    Absent,
    TrailerOnly,
    DistinctDetailsAndTrailer,
    MalformedBase64,
    InvalidProtobuf,
    DeadlineExceeded,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MacroEnvelopeCase {
    WrongRequestId,
    MissingRequestId,
    WrongOperation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MacroExternalSourceCase {
    Empty,
    Conflict,
}

#[derive(Default)]
struct LoopbackState {
    tcp_accepts: usize,
    macro_observation: MacroLoopbackObservation,
    requests: Vec<BoardLoopbackRequest>,
    membership_requests: Vec<BoardMembershipLoopbackRequest>,
    dragon_tiger_requests: Vec<DragonTigerLoopbackRequest>,
    retry_error_detail: Vec<u8>,
    industry_calls: usize,
    non_board_requests: usize,
    terminal_error_detail: Vec<u8>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum MembershipMode {
    Unsupported,
    CommitFailure,
    Success,
}

#[derive(Clone)]
enum DragonTigerMode {
    Unsupported,
    Success {
        records: Vec<u8>,
        batch_id: String,
        source: String,
    },
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum MacroMode {
    Unsupported,
    RetryThenSuccess,
    RepeatRetry,
    FixedStatus { code: Code, retryable: bool },
    RawStatus(MacroRawStatusCase),
    EnvelopeFailure(MacroEnvelopeCase),
    LocalShape,
    ExternalShape(MacroExternalSourceCase),
}

#[derive(Clone)]
struct BoardService {
    state: Arc<Mutex<LoopbackState>>,
    concept_terminal_error: bool,
    success_provider: &'static str,
    membership_mode: MembershipMode,
    dragon_tiger_mode: DragonTigerMode,
    macro_mode: MacroMode,
    membership_seen: Arc<tokio::sync::Notify>,
    membership_release: Arc<tokio::sync::Semaphore>,
}

impl BoardService {
    fn external_global_news_response(
        &self,
        source_case: MacroExternalSourceCase,
        request: Request<QueryRequest>,
    ) -> Result<Response<QueryResponse>, Status> {
        let authorized = request
            .metadata()
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            == Some("Bearer TEST_CODE_BOARD_LOOPBACK_TOKEN");
        let inner = request.into_inner();
        let request_id = inner
            .context
            .as_ref()
            .expect("TEST_CODE External Macro context")
            .request_id
            .clone();
        let mut state = self.state.lock().expect("TEST_CODE External Macro state");
        state
            .macro_observation
            .requests
            .push((inner.encode_to_vec(), authorized));
        state
            .macro_observation
            .routed_operations
            .push(Operation::GlobalNews);
        if state.macro_observation.requests.len() == 1 {
            let encoded = ErrorDetail {
                request_id,
                operation: Operation::GlobalNews as i32,
                provider: inner.preferred_provider,
                reason_code: "no_verified_batch".to_owned(),
                retryable: true,
                ..ErrorDetail::default()
            }
            .encode_to_vec();
            state
                .macro_observation
                .retry_error_details
                .push(encoded.clone());
            let mut status = Status::with_details(
                Code::Unavailable,
                "TEST_CODE External Macro first retry",
                encoded.clone().into(),
            );
            status.metadata_mut().insert_bin(
                "magic-error-detail-bin",
                tonic::metadata::MetadataValue::from_bytes(&encoded),
            );
            return Err(status);
        }
        let response = QueryResponse {
            request_id,
            operation: Operation::GlobalNews as i32,
            admission: AdmissionState::Admitted as i32,
            selected_provider: inner.preferred_provider,
            batch_id: "TEST_CODE_EXTERNAL_MACRO_BATCH".to_owned(),
            complete: true,
            observed_at: "2026-09-14T15:31:00+08:00".to_owned(),
            source_at: "2026-09-14T15:30:00+08:00".to_owned(),
            source: match source_case {
                MacroExternalSourceCase::Empty => "",
                MacroExternalSourceCase::Conflict => "TEST_CODE_FORBIDDEN_REMOTE_SOURCE",
            }
            .to_owned(),
            diagnostic_blocker: String::new(),
            records: Vec::new(),
        };
        state
            .macro_observation
            .response_bytes
            .push(response.encode_to_vec());
        Ok(Response::new(response))
    }

    fn local_macro_response(
        &self,
        operation: Operation,
        request: Request<QueryRequest>,
    ) -> Result<Response<QueryResponse>, Status> {
        let (schema, method) = match operation {
            Operation::GlobalNews => ("news.global_news", "global_news"),
            Operation::EconomicCalendar => ("market.economic_calendar", "economic_calendar"),
            Operation::SemanticSearch => ("market.semantic_search", "semantic_search"),
            _ => unreachable!("TEST_CODE closed Local Macro routes"),
        };
        let mut state = self.state.lock().expect("TEST_CODE Local Macro state");
        if self.macro_mode != MacroMode::LocalShape {
            state.non_board_requests += 1;
            return Err(Status::unimplemented(method));
        }
        let authorized = request
            .metadata()
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            == Some("Bearer TEST_CODE_BOARD_LOOPBACK_TOKEN");
        let inner = request.into_inner();
        let request_id = inner
            .context
            .as_ref()
            .expect("TEST_CODE Local Macro context")
            .request_id
            .clone();
        let payload: serde_json::Value = serde_json::from_slice(
            &inner
                .payload
                .as_ref()
                .expect("TEST_CODE Local Macro payload")
                .data,
        )
        .expect("TEST_CODE Local Macro JSON");
        let provider = if operation == Operation::EconomicCalendar {
            "Jin10"
        } else {
            payload["provider"]
                .as_str()
                .expect("TEST_CODE Local Macro provider")
        };
        state
            .macro_observation
            .requests
            .push((inner.encode_to_vec(), authorized));
        state.macro_observation.routed_operations.push(operation);
        if state.macro_observation.requests.len() == 1 {
            let encoded = ErrorDetail {
                request_id,
                operation: operation as i32,
                provider: provider.to_owned(),
                reason_code: "no_verified_batch".to_owned(),
                retryable: true,
                ..ErrorDetail::default()
            }
            .encode_to_vec();
            state
                .macro_observation
                .retry_error_details
                .push(encoded.clone());
            let mut status = Status::with_details(
                Code::Unavailable,
                "TEST_CODE Local Macro first retry",
                encoded.clone().into(),
            );
            status.metadata_mut().insert_bin(
                "magic-error-detail-bin",
                tonic::metadata::MetadataValue::from_bytes(&encoded),
            );
            return Err(status);
        }
        let response = QueryResponse {
            request_id,
            operation: operation as i32,
            admission: AdmissionState::Admitted as i32,
            selected_provider: provider.to_owned(),
            batch_id: "TEST_CODE_MACRO_SHAPE_BATCH".to_owned(),
            complete: true,
            observed_at: "2026-09-14T15:31:00+08:00".to_owned(),
            source_at: "2026-09-14T15:30:00+08:00".to_owned(),
            source: "TEST_CODE_MACRO_LOCAL_SOURCE".to_owned(),
            diagnostic_blocker: String::new(),
            records: vec![CanonicalPayload {
                schema: schema.to_owned(),
                schema_version: 1,
                content_type: "application/json; charset=utf-8".to_owned(),
                data: br#"[{"value":"TEST_CODE_MACRO_SHAPE"}]"#.to_vec(),
            }],
        };
        state
            .macro_observation
            .response_bytes
            .push(response.encode_to_vec());
        Ok(Response::new(response))
    }

    fn raw_macro_status(case: MacroRawStatusCase, request_id: String) -> Status {
        let detail = ErrorDetail {
            request_id,
            operation: Operation::GlobalNews as i32,
            provider: "Jin10".to_owned(),
            reason_code: "no_verified_batch".to_owned(),
            retryable: true,
            ..ErrorDetail::default()
        };
        let code = if case == MacroRawStatusCase::DeadlineExceeded {
            Code::DeadlineExceeded
        } else {
            Code::Unavailable
        };
        let details = if case == MacroRawStatusCase::DistinctDetailsAndTrailer {
            let mut standard = detail.clone();
            standard.retryable = false;
            standard.encode_to_vec()
        } else {
            Vec::new()
        };
        let mut status = Status::with_details(code, "TEST_CODE macro raw status", details.into());
        match case {
            MacroRawStatusCase::TrailerOnly | MacroRawStatusCase::DistinctDetailsAndTrailer => {
                status.metadata_mut().insert_bin(
                    "magic-error-detail-bin",
                    tonic::metadata::MetadataValue::from_bytes(&detail.encode_to_vec()),
                );
            }
            MacroRawStatusCase::MalformedBase64 => {
                // Public HeaderMap conversion preserves valid HTTP ASCII that
                // cannot decode as binary metadata. Never corrupt reserved
                // grpc-status-details-bin, which tonic decodes eagerly.
                let mut headers = tonic::codegen::http::HeaderMap::new();
                headers.insert(
                    "magic-error-detail-bin",
                    tonic::codegen::http::HeaderValue::from_static("%%%"),
                );
                *status.metadata_mut() = tonic::metadata::MetadataMap::from_headers(headers);
            }
            MacroRawStatusCase::InvalidProtobuf => {
                status.metadata_mut().insert_bin(
                    "magic-error-detail-bin",
                    tonic::metadata::MetadataValue::from_bytes(&[0xff]),
                );
            }
            MacroRawStatusCase::Absent | MacroRawStatusCase::DeadlineExceeded => {}
        }
        status
    }

    fn response(&self, request_id: String, kind: &str) -> QueryResponse {
        let (batch_id, bytes) = match kind {
            "Industry" => ("TEST_CODE_INDUSTRY_BATCH", INDUSTRY_DIRECTORY_BYTES),
            "Concept" => ("TEST_CODE_CONCEPT_BATCH", CONCEPT_DIRECTORY_BYTES),
            _ => unreachable!("TEST_CODE loopback only accepts the two board kinds"),
        };
        QueryResponse {
            request_id,
            operation: Operation::BoardDirectory as i32,
            admission: AdmissionState::Admitted as i32,
            selected_provider: self.success_provider.to_owned(),
            batch_id: batch_id.to_owned(),
            complete: true,
            observed_at: "2026-07-21T15:31:00+08:00".to_owned(),
            source_at: "2026-07-21T15:30:00+08:00".to_owned(),
            source: "TEST_CODE_LOOPBACK_BOARD_SOURCE".to_owned(),
            diagnostic_blocker: String::new(),
            records: vec![CanonicalPayload {
                schema: "board.directory".to_owned(),
                schema_version: 1,
                content_type: "application/json; charset=utf-8".to_owned(),
                data: bytes.to_vec(),
            }],
        }
    }
}

macro_rules! impl_board_service {
    ($($stub:ident),* $(,)?) => {
        #[tonic::async_trait]
        impl MarketDataService for BoardService {
            async fn global_news(
                &self,
                request: Request<QueryRequest>,
            ) -> Result<Response<QueryResponse>, Status> {
                if self.macro_mode == MacroMode::LocalShape {
                    return self.local_macro_response(Operation::GlobalNews, request);
                }
                if let MacroMode::ExternalShape(case) = self.macro_mode {
                    return self.external_global_news_response(case, request);
                }
                let mut state = self.state.lock().expect("TEST_CODE macro state");
                if self.macro_mode == MacroMode::Unsupported {
                    state.non_board_requests += 1;
                    return Err(Status::unimplemented("global_news"));
                }
                let authorized = request.metadata().get("authorization")
                    .and_then(|value| value.to_str().ok())
                    == Some("Bearer TEST_CODE_BOARD_LOOPBACK_TOKEN");
                let inner = request.into_inner();
                let request_id = inner.context.as_ref()
                    .expect("TEST_CODE macro request context").request_id.clone();
                state.macro_observation.requests.push((inner.encode_to_vec(), authorized));
                if let MacroMode::RawStatus(case) = self.macro_mode {
                    let status = Self::raw_macro_status(case, request_id);
                    state.macro_observation.raw_statuses.push(MacroLoopbackStatus {
                        code: status.code() as i32,
                        details: status.details().to_vec(),
                        trailer_header: status.metadata().get_bin("magic-error-detail-bin")
                            .map(|value| value.as_encoded_bytes().to_vec()),
                    });
                    return Err(status);
                }
                if self.macro_mode == MacroMode::RepeatRetry
                    || matches!(self.macro_mode, MacroMode::FixedStatus { .. })
                    || (self.macro_mode == MacroMode::RetryThenSuccess
                        && state.macro_observation.requests.len() == 1)
                {
                    let (code, retryable) = match self.macro_mode {
                        MacroMode::FixedStatus { code, retryable } => (code, retryable),
                        _ => (Code::Unavailable, true),
                    };
                    let detail = ErrorDetail {
                        request_id,
                        operation: Operation::GlobalNews as i32,
                        provider: "Jin10".to_owned(),
                        reason_code: if code == Code::InvalidArgument {
                            "invalid_request"
                        } else {
                            "no_verified_batch"
                        }.to_owned(),
                        retryable,
                        ..ErrorDetail::default()
                    };
                    let encoded = detail.encode_to_vec();
                    state.macro_observation.retry_error_details.push(encoded.clone());
                    let message = if matches!(self.macro_mode, MacroMode::FixedStatus { .. }) {
                        "TEST_CODE macro fixed failure"
                    } else {
                        "TEST_CODE macro retryable failure"
                    };
                    let mut status = Status::with_details(
                        code, message, encoded.clone().into(),
                    );
                    status.metadata_mut().insert_bin(
                        "magic-error-detail-bin", tonic::metadata::MetadataValue::from_bytes(&encoded),
                    );
                    return Err(status);
                }
                let mut response = QueryResponse {
                    request_id,
                    operation: Operation::GlobalNews as i32,
                    admission: AdmissionState::Admitted as i32,
                    selected_provider: "Jin10".to_owned(),
                    batch_id: "TEST_CODE_MACRO_BATCH".to_owned(),
                    complete: true,
                    observed_at: "2026-09-14T15:31:00+08:00".to_owned(),
                    source_at: "2026-09-14T15:30:00+08:00".to_owned(),
                    source: "jin10-flash-v1".to_owned(),
                    diagnostic_blocker: String::new(),
                    records: vec![CanonicalPayload {
                        schema: "news.global_news".to_owned(),
                        schema_version: 1,
                        content_type: "application/json; charset=utf-8".to_owned(),
                        data: br#"[{"title":"TEST_CODE_MACRO_NEWS"}]"#.to_vec(),
                    }],
                };
                match self.macro_mode {
                    MacroMode::EnvelopeFailure(MacroEnvelopeCase::WrongRequestId) => {
                        response.request_id = "TEST_CODE_WRONG_RESPONSE_ID".to_owned();
                    }
                    MacroMode::EnvelopeFailure(MacroEnvelopeCase::MissingRequestId) => {
                        response.request_id.clear();
                    }
                    MacroMode::EnvelopeFailure(MacroEnvelopeCase::WrongOperation) => {
                        response.operation = Operation::EconomicCalendar as i32;
                    }
                    _ => {}
                }
                state.macro_observation.response_bytes.push(response.encode_to_vec());
                Ok(Response::new(response))
            }

            async fn economic_calendar(
                &self, request: Request<QueryRequest>,
            ) -> Result<Response<QueryResponse>, Status> {
                self.local_macro_response(Operation::EconomicCalendar, request)
            }

            async fn semantic_search(
                &self, request: Request<QueryRequest>,
            ) -> Result<Response<QueryResponse>, Status> {
                self.local_macro_response(Operation::SemanticSearch, request)
            }

            async fn board_directory(
                &self,
                request: Request<QueryRequest>,
            ) -> Result<Response<QueryResponse>, Status> {
                let authorized = request
                    .metadata()
                    .get("authorization")
                    .and_then(|value| value.to_str().ok())
                    == Some("Bearer TEST_CODE_BOARD_LOOPBACK_TOKEN");
                let inner = request.into_inner();
                let context = inner
                    .context
                    .as_ref()
                    .expect("TEST_CODE board request context");
                let request_id = context.request_id.clone();
                let protocol_version = context.protocol_version;
                let payload = inner.payload.expect("TEST_CODE board request payload");
                let parameters: serde_json::Value = serde_json::from_slice(&payload.data)
                    .expect("TEST_CODE board request JSON");
                let kind = parameters["kind"]
                    .as_str()
                    .expect("TEST_CODE board request kind")
                    .to_owned();
                let limit = parameters["limit"]
                    .as_u64()
                    .expect("TEST_CODE board request limit");
                let mut state = self.state.lock().expect("TEST_CODE loopback state");
                state.requests.push(BoardLoopbackRequest {
                    kind: kind.clone(),
                    limit,
                    request_id: request_id.clone(),
                    protocol_version,
                    payload_schema: payload.schema,
                    payload_schema_version: payload.schema_version,
                    payload_content_type: payload.content_type,
                    allow_unadmitted: inner.allow_unadmitted,
                    authorized,
                });
                if kind == "Industry" && state.industry_calls == 0 {
                    state.industry_calls += 1;
                    let detail = ErrorDetail {
                        request_id,
                        operation: Operation::BoardDirectory as i32,
                        provider: "Tdx".to_owned(),
                        reason_code: "no_verified_batch".to_owned(),
                        retryable: true,
                        ..ErrorDetail::default()
                    };
                    let encoded = detail.encode_to_vec();
                    state.retry_error_detail = encoded.clone();
                    drop(state);
                    let mut status = Status::with_details(
                        Code::Unavailable,
                        "TEST_CODE synthetic retryable board failure",
                        encoded.clone().into(),
                    );
                    status.metadata_mut().insert_bin(
                        "magic-error-detail-bin",
                        tonic::metadata::MetadataValue::from_bytes(&encoded),
                    );
                    return Err(status);
                }
                if kind == "Concept" && self.concept_terminal_error {
                    let detail = ErrorDetail {
                        request_id,
                        operation: Operation::BoardDirectory as i32,
                        provider: "Tdx".to_owned(),
                        reason_code: "invalid_evidence".to_owned(),
                        retryable: false,
                        ..ErrorDetail::default()
                    };
                    let encoded = detail.encode_to_vec();
                    state.terminal_error_detail = encoded.clone();
                    drop(state);
                    let mut status = Status::with_details(
                        Code::FailedPrecondition,
                        "TEST_CODE synthetic terminal board failure",
                        encoded.clone().into(),
                    );
                    status.metadata_mut().insert_bin(
                        "magic-error-detail-bin",
                        tonic::metadata::MetadataValue::from_bytes(&encoded),
                    );
                    return Err(status);
                }
                drop(state);
                Ok(Response::new(self.response(request_id, &kind)))
            }

            async fn board_constituents(
                &self,
                request: Request<QueryRequest>,
            ) -> Result<Response<QueryResponse>, Status> {
                if self.membership_mode == MembershipMode::Unsupported {
                    self.state
                        .lock()
                        .expect("TEST_CODE loopback state")
                        .non_board_requests += 1;
                    return Err(Status::unimplemented("board_constituents"));
                }
                let authorized = request
                    .metadata()
                    .get("authorization")
                    .and_then(|value| value.to_str().ok())
                    == Some("Bearer TEST_CODE_BOARD_LOOPBACK_TOKEN");
                let inner = request.into_inner();
                let context = inner
                    .context
                    .as_ref()
                    .expect("TEST_CODE membership request context");
                let request_id = context.request_id.clone();
                let protocol_version = context.protocol_version;
                let payload = inner
                    .payload
                    .expect("TEST_CODE membership request payload");
                let parameters: serde_json::Value = serde_json::from_slice(&payload.data)
                    .expect("TEST_CODE membership request JSON");
                let codes = parameters["codes"]
                    .as_array()
                    .expect("TEST_CODE membership request codes")
                    .iter()
                    .map(|code| {
                        code.as_str()
                            .expect("TEST_CODE membership request code")
                            .to_owned()
                    })
                    .collect();
                self.state
                    .lock()
                    .expect("TEST_CODE loopback state")
                    .membership_requests
                    .push(BoardMembershipLoopbackRequest {
                        codes,
                        request_id: request_id.clone(),
                        protocol_version,
                        payload_schema: payload.schema,
                        payload_schema_version: payload.schema_version,
                        payload_content_type: payload.content_type,
                        preferred_provider: inner.preferred_provider,
                        allow_unadmitted: inner.allow_unadmitted,
                        authorized,
                    });
                self.membership_seen.notify_one();
                if self.membership_mode == MembershipMode::Success {
                    return Ok(Response::new(QueryResponse {
                        request_id,
                        operation: Operation::BoardConstituents as i32,
                        selected_provider: "Tdx".to_owned(),
                        admission: AdmissionState::Admitted as i32,
                        batch_id: "TEST_CODE_MEMBERSHIP_BATCH".to_owned(),
                        complete: true,
                        observed_at: "2026-07-21T15:31:00+08:00".to_owned(),
                        source_at: "2026-07-21T15:30:00+08:00".to_owned(),
                        source: "TEST_CODE_LOOPBACK_MEMBERSHIP_SOURCE".to_owned(),
                        diagnostic_blocker: String::new(),
                        records: vec![CanonicalPayload {
                            schema: "board.constituents".to_owned(),
                            schema_version: 1,
                            content_type: "application/json; charset=utf-8".to_owned(),
                            data: br#"[{"instrument_code":"TEST_CODE_600001","board_code":"TEST_CODE_BOARD_MAIN","board_name":"TEST_CODE_CLUSTER_A_MAIN","kind":"Industry"},{"instrument_code":"TEST_CODE_600001","board_code":"TEST_CODE_BOARD_ALIAS","board_name":"TEST_CODE_CLUSTER_B_ALIAS","kind":"Concept"}]"#.to_vec(),
                        }],
                    }));
                }
                let permit = self
                    .membership_release
                    .acquire()
                    .await
                    .expect("TEST_CODE membership release semaphore");
                permit.forget();
                let detail = ErrorDetail {
                    request_id,
                    operation: Operation::BoardConstituents as i32,
                    provider: "Tdx".to_owned(),
                    reason_code: "no_verified_batch".to_owned(),
                    retryable: true,
                    ..ErrorDetail::default()
                };
                let encoded = detail.encode_to_vec();
                let mut status = Status::with_details(
                    Code::Unavailable,
                    "TEST_CODE synthetic retryable membership failure",
                    encoded.clone().into(),
                );
                status.metadata_mut().insert_bin(
                    "magic-error-detail-bin",
                    tonic::metadata::MetadataValue::from_bytes(&encoded),
                );
                Err(status)
            }

            async fn dragon_tiger(
                &self,
                request: Request<QueryRequest>,
            ) -> Result<Response<QueryResponse>, Status> {
                let DragonTigerMode::Success { records, batch_id, source } =
                    &self.dragon_tiger_mode
                else {
                    self.state
                        .lock()
                        .expect("TEST_CODE loopback state")
                        .non_board_requests += 1;
                    return Err(Status::unimplemented("dragon_tiger"));
                };
                let authorized = request
                    .metadata()
                    .get("authorization")
                    .and_then(|value| value.to_str().ok())
                    == Some("Bearer TEST_CODE_BOARD_LOOPBACK_TOKEN");
                let inner = request.into_inner();
                let request_bytes = inner.encode_to_vec();
                let context = inner
                    .context
                    .as_ref()
                    .expect("TEST_CODE dragon-tiger request context");
                let request_id = context.request_id.clone();
                let payload = inner
                    .payload
                    .as_ref()
                    .expect("TEST_CODE dragon-tiger request payload");
                let parameters: serde_json::Value = serde_json::from_slice(&payload.data)
                    .expect("TEST_CODE dragon-tiger request JSON");
                self.state
                    .lock()
                    .expect("TEST_CODE loopback state")
                    .dragon_tiger_requests
                    .push(DragonTigerLoopbackRequest {
                        date: parameters["date"].as_str().unwrap().to_owned(),
                        disclosure_limit: parameters["disclosure_limit"].as_u64().unwrap(),
                        stock_limit: parameters["stock_limit"].as_u64().unwrap(),
                        request_id: request_id.clone(),
                        request_bytes,
                        payload_schema: payload.schema.clone(),
                        payload_schema_version: payload.schema_version,
                        payload_content_type: payload.content_type.clone(),
                        preferred_provider: inner.preferred_provider,
                        allow_unadmitted: inner.allow_unadmitted,
                        authorized,
                    });
                Ok(Response::new(QueryResponse {
                    request_id,
                    operation: Operation::DragonTiger as i32,
                    admission: AdmissionState::Admitted as i32,
                    selected_provider: "Eastmoney".to_owned(),
                    batch_id: batch_id.clone(),
                    complete: true,
                    observed_at: "2026-07-22T15:31:02+08:00".to_owned(),
                    source_at: "2026-07-22T15:30:00+08:00".to_owned(),
                    source: source.clone(),
                    diagnostic_blocker: String::new(),
                    records: vec![CanonicalPayload {
                        schema: "market.dragon_tiger".to_owned(),
                        schema_version: 1,
                        content_type: "application/json; charset=utf-8".to_owned(),
                        data: records.clone(),
                    }],
                }))
            }

            $(
                async fn $stub(
                    &self,
                    _request: Request<QueryRequest>,
                ) -> Result<Response<QueryResponse>, Status> {
                    self.state
                        .lock()
                        .expect("TEST_CODE loopback state")
                        .non_board_requests += 1;
                    Err(Status::unimplemented(stringify!($stub)))
                }
            )*
        }
    };
}

impl_board_service!(
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

pub(crate) struct BoardLoopbackServer {
    state: Arc<Mutex<LoopbackState>>,
    membership_seen: Arc<tokio::sync::Notify>,
    membership_release: Arc<tokio::sync::Semaphore>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl BoardLoopbackServer {
    pub(crate) fn macro_snapshot(&self) -> MacroLoopbackObservation {
        self.state
            .lock()
            .expect("TEST_CODE macro snapshot")
            .macro_observation
            .clone()
    }

    pub(crate) fn membership_snapshot(&self) -> Vec<BoardMembershipLoopbackRequest> {
        self.state
            .lock()
            .expect("TEST_CODE membership snapshot")
            .membership_requests
            .clone()
    }

    pub(crate) fn dragon_tiger_snapshot(&self) -> Vec<DragonTigerLoopbackRequest> {
        self.state
            .lock()
            .expect("TEST_CODE dragon-tiger snapshot")
            .dragon_tiger_requests
            .clone()
    }

    pub(crate) async fn wait_for_membership_request(&self) -> BoardMembershipLoopbackRequest {
        loop {
            let notified = self.membership_seen.notified();
            if let Some(request) = self.membership_snapshot().last().cloned() {
                return request;
            }
            notified.await;
        }
    }

    pub(crate) fn release_membership_unavailable(&self) {
        self.membership_release.add_permits(1);
    }

    pub(crate) fn terminal_error_detail(&self) -> Vec<u8> {
        self.state
            .lock()
            .expect("TEST_CODE terminal error snapshot")
            .terminal_error_detail
            .clone()
    }

    pub(crate) fn snapshot(&self) -> BoardLoopbackObservation {
        self.snapshot_with_tcp_for_test().1
    }

    pub(crate) fn snapshot_with_tcp_for_test(&self) -> (usize, BoardLoopbackObservation) {
        let state = self.state.lock().expect("TEST_CODE loopback snapshot");
        (
            state.tcp_accepts,
            BoardLoopbackObservation {
                requests: state.requests.clone(),
                dragon_tiger_requests: state.dragon_tiger_requests.clone(),
                retry_error_detail: state.retry_error_detail.clone(),
                non_board_requests: state.non_board_requests,
            },
        )
    }

    pub(crate) async fn finish(mut self) -> BoardLoopbackObservation {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(mut task) = self.task.take() {
            match tokio::time::timeout(std::time::Duration::from_secs(5), &mut task).await {
                Ok(result) => result.expect("TEST_CODE loopback server task"),
                Err(_) => {
                    task.abort();
                    let _ = task.await;
                    panic!("TEST_CODE loopback shutdown timeout");
                }
            }
        }
        self.snapshot()
    }
}

impl Drop for BoardLoopbackServer {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

pub(crate) async fn spawn_board_loopback() -> (GrpcMarketClient, BoardLoopbackServer) {
    spawn_board_loopback_mode(
        false,
        "Tdx",
        MembershipMode::Unsupported,
        DragonTigerMode::Unsupported,
    )
    .await
}

pub(crate) async fn spawn_board_terminal_error_loopback() -> (GrpcMarketClient, BoardLoopbackServer)
{
    spawn_board_loopback_mode(
        true,
        "Tdx",
        MembershipMode::Unsupported,
        DragonTigerMode::Unsupported,
    )
    .await
}

pub(crate) async fn spawn_custom_provider_board_loopback() -> (GrpcMarketClient, BoardLoopbackServer)
{
    spawn_board_loopback_mode(
        false,
        "Custom",
        MembershipMode::Unsupported,
        DragonTigerMode::Unsupported,
    )
    .await
}

pub(crate) async fn spawn_membership_commit_failure_loopback(
) -> (GrpcMarketClient, BoardLoopbackServer) {
    spawn_board_loopback_mode(
        false,
        "Tdx",
        MembershipMode::CommitFailure,
        DragonTigerMode::Unsupported,
    )
    .await
}

pub(crate) async fn spawn_membership_success_loopback() -> (GrpcMarketClient, BoardLoopbackServer) {
    // This deadline also covers bind; the shared RAII server owns shutdown after spawn.
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        spawn_board_loopback_mode(
            false,
            "Tdx",
            MembershipMode::Success,
            DragonTigerMode::Unsupported,
        ),
    )
    .await
    .expect("TEST_CODE membership success listener/connect deadline")
}

pub(crate) async fn spawn_dragon_tiger_success_loopback(
    records: &[u8],
    batch_id: &str,
    source: &str,
) -> (GrpcMarketClient, BoardLoopbackServer) {
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        spawn_board_loopback_mode(
            false,
            "Tdx",
            MembershipMode::Success,
            DragonTigerMode::Success {
                records: records.to_vec(),
                batch_id: batch_id.to_owned(),
                source: source.to_owned(),
            },
        ),
    )
    .await
    .expect("TEST_CODE dragon-tiger listener/connect deadline")
}

/// Macro integration owns this handle before making the first connection, so
/// setup failures can explicitly join the parent server as well as its own.
pub(crate) async fn spawn_macro_parent_listener(records: &[u8]) -> (String, BoardLoopbackServer) {
    spawn_board_loopback_service(
        false,
        "Tdx",
        MembershipMode::Success,
        DragonTigerMode::Success {
            records: records.to_vec(),
            batch_id: "TEST_CODE_LHB_BATCH_FIRST".to_owned(),
            source: "TEST_CODE_LHB_SOURCE_FIRST".to_owned(),
        },
        MacroMode::Unsupported,
    )
    .await
}

async fn spawn_board_loopback_mode(
    concept_terminal_error: bool,
    success_provider: &'static str,
    membership_mode: MembershipMode,
    dragon_tiger_mode: DragonTigerMode,
) -> (GrpcMarketClient, BoardLoopbackServer) {
    let (endpoint, server) = spawn_board_loopback_service(
        concept_terminal_error,
        success_provider,
        membership_mode,
        dragon_tiger_mode,
        MacroMode::Unsupported,
    )
    .await;
    let channel = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tonic::transport::Channel::from_shared(endpoint)
            .expect("TEST_CODE board loopback endpoint")
            .connect(),
    )
    .await
    .expect("TEST_CODE board loopback connect timeout")
    .expect("TEST_CODE board loopback connect");
    let client = GrpcMarketClient::from_channel(
        channel,
        ContractProfile::LocalBridgeV1,
        ClientAuthorization::InstanceBearer(Zeroizing::new(
            "TEST_CODE_BOARD_LOOPBACK_TOKEN".to_owned(),
        )),
        None,
    );
    (client, server)
}

/// Macro setup owns the server before connecting, so every connection failure
/// explicitly shuts down and joins it. Bind failure creates no server task.
pub(crate) async fn spawn_macro_loopback() -> (GrpcMarketClient, BoardLoopbackServer) {
    spawn_macro_loopback_mode(MacroMode::RetryThenSuccess).await
}

pub(crate) async fn spawn_macro_repeat_retry_loopback() -> (GrpcMarketClient, BoardLoopbackServer) {
    spawn_macro_loopback_mode(MacroMode::RepeatRetry).await
}

pub(crate) async fn spawn_macro_fixed_status_loopback(
    code: Code,
    retryable: bool,
) -> (GrpcMarketClient, BoardLoopbackServer) {
    spawn_macro_loopback_mode(MacroMode::FixedStatus { code, retryable }).await
}

pub(crate) async fn spawn_macro_raw_status_loopback(
    case: MacroRawStatusCase,
) -> (GrpcMarketClient, BoardLoopbackServer) {
    spawn_macro_loopback_mode(MacroMode::RawStatus(case)).await
}

pub(crate) async fn spawn_macro_envelope_failure_loopback(
    case: MacroEnvelopeCase,
) -> (GrpcMarketClient, BoardLoopbackServer) {
    spawn_macro_loopback_mode(MacroMode::EnvelopeFailure(case)).await
}

pub(crate) async fn spawn_macro_local_shape_loopback() -> (GrpcMarketClient, BoardLoopbackServer) {
    spawn_macro_loopback_mode(MacroMode::LocalShape).await
}

pub(crate) async fn spawn_macro_external_shape_loopback(
    source_case: MacroExternalSourceCase,
    authority: &str,
) -> (GrpcMarketClient, BoardLoopbackServer) {
    spawn_macro_loopback_mode_for_profile(
        MacroMode::ExternalShape(source_case),
        ContractProfile::ExternalV1,
        Some(authority),
    )
    .await
}

async fn spawn_macro_loopback_mode(mode: MacroMode) -> (GrpcMarketClient, BoardLoopbackServer) {
    spawn_macro_loopback_mode_for_profile(mode, ContractProfile::LocalBridgeV1, None).await
}

async fn spawn_macro_loopback_mode_for_profile(
    mode: MacroMode,
    profile: ContractProfile,
    acquisition_authority: Option<&str>,
) -> (GrpcMarketClient, BoardLoopbackServer) {
    use futures::FutureExt as _;
    let (endpoint, server) = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        spawn_board_loopback_service(
            false,
            "Tdx",
            MembershipMode::Unsupported,
            DragonTigerMode::Unsupported,
            mode,
        ),
    )
    .await
    .expect("TEST_CODE macro bind timeout");
    let connected = std::panic::AssertUnwindSafe(tokio::time::timeout(
        std::time::Duration::from_secs(5),
        async {
            let channel = tonic::transport::Channel::from_shared(endpoint)
                .expect("TEST_CODE macro endpoint")
                .connect()
                .await
                .expect("TEST_CODE macro connect");
            GrpcMarketClient::from_channel(
                channel,
                profile,
                ClientAuthorization::InstanceBearer(Zeroizing::new(
                    "TEST_CODE_BOARD_LOOPBACK_TOKEN".to_owned(),
                )),
                acquisition_authority.map(str::to_owned),
            )
        },
    ))
    .catch_unwind()
    .await;
    match connected {
        Ok(Ok(client)) => (client, server),
        failure => {
            server.finish().await;
            match failure {
                Err(panic) => std::panic::resume_unwind(panic),
                _ => panic!("TEST_CODE macro connect timeout"),
            }
        }
    }
}

async fn spawn_board_loopback_service(
    concept_terminal_error: bool,
    success_provider: &'static str,
    membership_mode: MembershipMode,
    dragon_tiger_mode: DragonTigerMode,
    macro_mode: MacroMode,
) -> (String, BoardLoopbackServer) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("TEST_CODE bind board loopback");
    let address = listener
        .local_addr()
        .expect("TEST_CODE board loopback address");
    let state = Arc::new(Mutex::new(LoopbackState::default()));
    let membership_seen = Arc::new(tokio::sync::Notify::new());
    let membership_release = Arc::new(tokio::sync::Semaphore::new(0));
    let service = BoardService {
        state: Arc::clone(&state),
        concept_terminal_error,
        success_provider,
        membership_mode,
        dragon_tiger_mode,
        macro_mode,
        membership_seen: Arc::clone(&membership_seen),
        membership_release: Arc::clone(&membership_release),
    };
    let (shutdown, shutdown_rx) = tokio::sync::oneshot::channel();
    let incoming_state = Arc::clone(&state);
    let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener).map(move |accepted| {
        if accepted.is_ok() {
            incoming_state
                .lock()
                .expect("TEST_CODE board TCP accept")
                .tcp_accepts += 1;
        }
        accepted
    });
    let task = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(MarketDataServiceServer::new(service))
            .serve_with_incoming_shutdown(incoming, async move {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("TEST_CODE board loopback server");
    });
    let server = BoardLoopbackServer {
        state,
        membership_seen,
        membership_release,
        shutdown: Some(shutdown),
        task: Some(task),
    };
    let endpoint = format!("http://{address}");
    (endpoint, server)
}

pub(crate) fn clone_with_invalid_instance_bearer(client: &GrpcMarketClient) -> GrpcMarketClient {
    let mut invalid = client.clone();
    invalid.authorization =
        ClientAuthorization::InstanceBearer(Zeroizing::new("TEST_CODE_INVALID\nTOKEN".to_owned()));
    invalid
}

pub(crate) fn clone_with_retry_policy(
    client: &GrpcMarketClient,
    policy: (u32, u64, u64, u64),
) -> GrpcMarketClient {
    let mut changed = client.clone();
    changed.retry = crate::grpc_client::retry::RetryPolicy {
        max_attempts: policy.0,
        base_delay_ms: policy.1,
        max_delay_ms: policy.2,
        jitter_ms: policy.3,
    };
    changed
}
