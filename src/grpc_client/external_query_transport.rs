use crate::grpc_client::errors::{ErrorDetail, GrpcError};
use crate::grpc_client::external_pb::magic::market::v1::{
    market_data_service_client::MarketDataServiceClient, QueryRequest, QueryResponse,
};
use http_body::{Frame, SizeHint};
use prost::bytes::Buf as _;
use prost::encoding::{
    decode_key, decode_varint, skip_field, DecodeContext, WireType,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use tonic::body::Body;
use tonic::codegen::{http, Bytes, Service};
use tonic::transport::Channel;

pub(crate) const EXTERNAL_QUERY_DECODE_LIMIT_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const EXTERNAL_QUERY_FRAMED_BODY_LIMIT_BYTES: usize =
    EXTERNAL_QUERY_DECODE_LIMIT_BYTES + 5;
const EXTERNAL_WIRE_MATERIAL: &str = "external-unary-response-evidence-v1";
pub(crate) const EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256: &str =
    "5ba0fa3b2fa450e74bdcc8cb5f163348a6ca90df3f3626d1d8f2ec27137f5edb";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) enum ExternalQueryMethod {
    #[serde(rename = "OPERATION_SECURITY_METADATA")]
    SecurityMetadata,
    #[serde(rename = "OPERATION_GLOBAL_NEWS")]
    GlobalNews,
    #[serde(rename = "OPERATION_INSTRUMENT_NEWS")]
    InstrumentNews,
}

impl ExternalQueryMethod {
    const SERVICE: &'static str = "magic.market.v1.MarketDataService";

    fn generated_method(self) -> &'static str {
        match self {
            Self::SecurityMetadata => "SecurityMetadata",
            Self::GlobalNews => "GlobalNews",
            Self::InstrumentNews => "InstrumentNews",
        }
    }

    fn path(self) -> &'static str {
        match self {
            Self::SecurityMetadata => {
                "/magic.market.v1.MarketDataService/SecurityMetadata"
            }
            Self::GlobalNews => "/magic.market.v1.MarketDataService/GlobalNews",
            Self::InstrumentNews => "/magic.market.v1.MarketDataService/InstrumentNews",
        }
    }

    fn matches_binding(
        self,
        path: &str,
        grpc_method: Option<&tonic::GrpcMethod<'static>>,
    ) -> bool {
        path == self.path()
            && grpc_method.is_some_and(|grpc_method| {
                grpc_method.service() == Self::SERVICE
                    && grpc_method.method() == self.generated_method()
            })
    }

    pub(crate) fn from_local_operation(
        operation: crate::grpc_client::pb::magic::market::v1::Operation,
    ) -> Option<Self> {
        use crate::grpc_client::pb::magic::market::v1::Operation;
        match operation {
            Operation::SecurityMetadata => Some(Self::SecurityMetadata),
            Operation::GlobalNews => Some(Self::GlobalNews),
            Operation::InstrumentNews => Some(Self::InstrumentNews),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExternalWireEvidenceV1 {
    pub(crate) material: String,
    pub(crate) profile: String,
    pub(crate) method: ExternalQueryMethod,
    pub(crate) client_descriptor_sha256: String,
    pub(crate) evidence: ExternalWireMaterialV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) enum ExternalWireMaterialV1 {
    Payload {
        protobuf_payload: Vec<u8>,
        payload_sha256: String,
        decode_limit_bytes: usize,
    },
    Missing {
        framed_body_limit_bytes: usize,
    },
    Overflow {
        framed_body_limit_bytes: usize,
        observed_framed_body_bytes_at_least: usize,
    },
    InvalidFrame {
        failure: ExternalFrameFailureV1,
        grpc_body_bytes: Vec<u8>,
        body_sha256: String,
        framed_body_limit_bytes: usize,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) enum ExternalFrameFailureV1 {
    HeaderTruncated,
    CompressionUnsupported,
    PayloadLengthExceedsLimit,
    PayloadTruncated,
    TrailingData,
}

impl ExternalWireEvidenceV1 {
    fn new(method: ExternalQueryMethod, evidence: ExternalWireMaterialV1) -> Self {
        Self {
            material: EXTERNAL_WIRE_MATERIAL.to_owned(),
            profile: "ExternalV1".to_owned(),
            method,
            client_descriptor_sha256: EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256.to_owned(),
            evidence,
        }
    }

    pub(crate) fn payload(&self) -> Option<&[u8]> {
        match &self.evidence {
            ExternalWireMaterialV1::Payload {
                protobuf_payload, ..
            } => Some(protobuf_payload),
            _ => None,
        }
    }

    pub(crate) fn validate(&self, method: ExternalQueryMethod) -> Result<(), GrpcError> {
        if self.material != EXTERNAL_WIRE_MATERIAL
            || self.profile != "ExternalV1"
            || self.method != method
            || self.client_descriptor_sha256 != EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256
            || compiled_descriptor_sha256() != EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256
        {
            return Err(wire_error("external_response_wire_invalid"));
        }
        match &self.evidence {
            ExternalWireMaterialV1::Payload {
                protobuf_payload,
                payload_sha256,
                decode_limit_bytes,
            } => {
                if *decode_limit_bytes != EXTERNAL_QUERY_DECODE_LIMIT_BYTES
                    || protobuf_payload.len() > *decode_limit_bytes
                    || *payload_sha256 != hex::encode(Sha256::digest(protobuf_payload))
                {
                    return Err(wire_error("external_response_wire_invalid"));
                }
            }
            ExternalWireMaterialV1::Missing {
                framed_body_limit_bytes,
            } => {
                if *framed_body_limit_bytes != EXTERNAL_QUERY_FRAMED_BODY_LIMIT_BYTES {
                    return Err(wire_error("external_response_wire_invalid"));
                }
            }
            ExternalWireMaterialV1::Overflow {
                framed_body_limit_bytes,
                observed_framed_body_bytes_at_least,
            } => {
                if *framed_body_limit_bytes != EXTERNAL_QUERY_FRAMED_BODY_LIMIT_BYTES
                    || *observed_framed_body_bytes_at_least <= *framed_body_limit_bytes
                {
                    return Err(wire_error("external_response_wire_invalid"));
                }
            }
            ExternalWireMaterialV1::InvalidFrame {
                failure,
                grpc_body_bytes,
                body_sha256,
                framed_body_limit_bytes,
            } => {
                if *framed_body_limit_bytes != EXTERNAL_QUERY_FRAMED_BODY_LIMIT_BYTES
                    || grpc_body_bytes.is_empty()
                    || grpc_body_bytes.len() > *framed_body_limit_bytes
                    || *body_sha256 != hex::encode(Sha256::digest(grpc_body_bytes))
                    || parse_uncompressed_unary_frame(grpc_body_bytes).err() != Some(*failure)
                {
                    return Err(wire_error("external_response_wire_invalid"));
                }
            }
        }
        Ok(())
    }
}

pub(crate) enum ExternalQueryCall {
    Response {
        message: QueryResponse,
        evidence: ExternalWireEvidenceV1,
    },
    /// Remote status; `evidence` is whatever body material arrived with it
    /// (normally `Missing` for a trailers-only status).
    UnaryStatus {
        status: tonic::Status,
        evidence: ExternalWireEvidenceV1,
    },
    LocalWireFailure {
        error: GrpcError,
        evidence: ExternalWireEvidenceV1,
    },
}

#[derive(Clone)]
pub(crate) struct ExternalQueryTransport {
    client: MarketDataServiceClient<CapturedExternalChannel>,
}

impl ExternalQueryTransport {
    pub(crate) fn new(channel: Channel) -> Self {
        let client = MarketDataServiceClient::new(CapturedExternalChannel(channel))
            .max_decoding_message_size(EXTERNAL_QUERY_DECODE_LIMIT_BYTES);
        Self { client }
    }

    pub(crate) async fn call(
        &mut self,
        method: ExternalQueryMethod,
        mut request: tonic::Request<QueryRequest>,
    ) -> ExternalQueryCall {
        let capture = CaptureHandle::new(method);
        request.extensions_mut().insert(capture.clone());
        let response = match method {
            ExternalQueryMethod::SecurityMetadata => self.client.security_metadata(request).await,
            ExternalQueryMethod::GlobalNews => self.client.global_news(request).await,
            ExternalQueryMethod::InstrumentNews => self.client.instrument_news(request).await,
        };
        match response {
            Err(status) => ExternalQueryCall::UnaryStatus {
                status,
                evidence: capture.observed(),
            },
            Ok(response) => match capture.evidence() {
                Ok(evidence) => ExternalQueryCall::Response {
                    message: response.into_inner(),
                    evidence,
                },
                Err((error, evidence)) => {
                    ExternalQueryCall::LocalWireFailure { error, evidence }
                }
            },
        }
    }
}

#[derive(Clone)]
struct CapturedExternalChannel(Channel);

#[derive(Debug)]
enum CapturedExternalChannelError {
    Transport(tonic::transport::Error),
    Binding,
}

impl std::fmt::Display for CapturedExternalChannelError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(error) => write!(formatter, "{error}"),
            Self::Binding => formatter.write_str("external query transport binding mismatch"),
        }
    }
}

impl std::error::Error for CapturedExternalChannelError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Transport(error) => Some(error),
            Self::Binding => None,
        }
    }
}

impl Service<http::Request<Body>> for CapturedExternalChannel {
    type Response = http::Response<Body>;
    type Error = CapturedExternalChannelError;
    type Future = Pin<
        Box<
            dyn Future<Output = Result<Self::Response, Self::Error>> + Send + 'static,
        >,
    >;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.0
            .poll_ready(cx)
            .map(|result| result.map_err(CapturedExternalChannelError::Transport))
    }

    fn call(&mut self, mut request: http::Request<Body>) -> Self::Future {
        let capture = request.extensions_mut().remove::<CaptureHandle>();
        let Some(capture) = capture else {
            return Box::pin(async { Err(CapturedExternalChannelError::Binding) });
        };
        if !capture.method.matches_binding(
            request.uri().path(),
            request
                .extensions()
                .get::<tonic::GrpcMethod<'static>>(),
        ) {
            return Box::pin(async { Err(CapturedExternalChannelError::Binding) });
        }
        let future = self.0.call(request);
        Box::pin(async move {
            let response = future
                .await
                .map_err(CapturedExternalChannelError::Transport)?;
            let (parts, body) = response.into_parts();
            Ok(http::Response::from_parts(
                parts,
                Body::new(CapturedBody { body, capture }),
            ))
        })
    }
}

#[derive(Clone)]
struct CaptureHandle {
    state: Arc<Mutex<CaptureState>>,
    method: ExternalQueryMethod,
}

#[derive(Default)]
struct CaptureState {
    bytes: Vec<u8>,
    observed: usize,
    overflow: bool,
    ended: bool,
}

impl CaptureHandle {
    fn new(method: ExternalQueryMethod) -> Self {
        Self {
            state: Arc::new(Mutex::new(CaptureState::default())),
            method,
        }
    }

    /// Body material exactly as captured, without judging whether it is a
    /// usable response.
    fn observed(&self) -> ExternalWireEvidenceV1 {
        let state = self.state.lock().expect("external capture mutex poisoned");
        let material = if state.overflow {
            ExternalWireMaterialV1::Overflow {
                framed_body_limit_bytes: EXTERNAL_QUERY_FRAMED_BODY_LIMIT_BYTES,
                observed_framed_body_bytes_at_least: state.observed,
            }
        } else if !state.ended || state.bytes.is_empty() {
            ExternalWireMaterialV1::Missing {
                framed_body_limit_bytes: EXTERNAL_QUERY_FRAMED_BODY_LIMIT_BYTES,
            }
        } else {
            match parse_uncompressed_unary_frame(&state.bytes) {
                Ok(payload) => ExternalWireMaterialV1::Payload {
                    protobuf_payload: payload.to_vec(),
                    payload_sha256: hex::encode(Sha256::digest(payload)),
                    decode_limit_bytes: EXTERNAL_QUERY_DECODE_LIMIT_BYTES,
                },
                Err(failure) => ExternalWireMaterialV1::InvalidFrame {
                    failure,
                    grpc_body_bytes: state.bytes.clone(),
                    body_sha256: hex::encode(Sha256::digest(&state.bytes)),
                    framed_body_limit_bytes: EXTERNAL_QUERY_FRAMED_BODY_LIMIT_BYTES,
                },
            }
        };
        ExternalWireEvidenceV1::new(self.method, material)
    }

    fn evidence(&self) -> Result<ExternalWireEvidenceV1, (GrpcError, ExternalWireEvidenceV1)> {
        let evidence = self.observed();
        match &evidence.evidence {
            ExternalWireMaterialV1::Payload { .. } => match evidence.validate(self.method) {
                Ok(()) => Ok(evidence),
                Err(error) => Err((error, evidence)),
            },
            _ => Err((wire_error("external_response_wire_invalid"), evidence)),
        }
    }
}

struct CapturedBody {
    body: Body,
    capture: CaptureHandle,
}

impl http_body::Body for CapturedBody {
    type Data = Bytes;
    type Error = tonic::Status;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let result = Pin::new(&mut self.body).poll_frame(cx);
        match &result {
            Poll::Ready(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    let mut state = self
                        .capture
                        .state
                        .lock()
                        .expect("external capture mutex poisoned");
                    state.observed = state.observed.saturating_add(data.len());
                    if state.observed <= EXTERNAL_QUERY_FRAMED_BODY_LIMIT_BYTES {
                        state.bytes.extend_from_slice(data);
                    } else {
                        state.overflow = true;
                        state.bytes.clear();
                    }
                }
                if frame.is_trailers() || self.body.is_end_stream() {
                    self.capture
                        .state
                        .lock()
                        .expect("external capture mutex poisoned")
                        .ended = true;
                }
            }
            Poll::Ready(None) => {
                self.capture
                    .state
                    .lock()
                    .expect("external capture mutex poisoned")
                    .ended = true;
            }
            _ => {}
        }
        result
    }

    fn size_hint(&self) -> SizeHint {
        self.body.size_hint()
    }

    fn is_end_stream(&self) -> bool {
        self.body.is_end_stream()
    }
}

fn parse_uncompressed_unary_frame(
    bytes: &[u8],
) -> Result<&[u8], ExternalFrameFailureV1> {
    if bytes.len() < 5 {
        return Err(ExternalFrameFailureV1::HeaderTruncated);
    }
    if bytes[0] != 0 {
        return Err(ExternalFrameFailureV1::CompressionUnsupported);
    }
    let declared = u32::from_be_bytes(bytes[1..5].try_into().expect("five-byte header")) as usize;
    if declared > EXTERNAL_QUERY_DECODE_LIMIT_BYTES {
        return Err(ExternalFrameFailureV1::PayloadLengthExceedsLimit);
    }
    let expected = declared + 5;
    if bytes.len() < expected {
        return Err(ExternalFrameFailureV1::PayloadTruncated);
    }
    if bytes.len() > expected {
        return Err(ExternalFrameFailureV1::TrailingData);
    }
    Ok(&bytes[5..])
}

pub(crate) fn wire_error(code: &str) -> GrpcError {
    GrpcError::FailedPrecondition {
        details: Box::new(ErrorDetail {
            code: code.to_owned(),
            reason_code: Some(code.to_owned()),
            retryable: Some(false),
            ..ErrorDetail::default()
        }),
    }
}

fn compiled_descriptor_sha256() -> String {
    hex::encode(Sha256::digest(
        crate::grpc_client::external_pb::FILE_DESCRIPTOR_SET,
    ))
}

pub(crate) fn admit_external_payload(payload: &[u8]) -> Result<(), GrpcError> {
    let mut remaining = payload;
    while remaining.has_remaining() {
        let (field, wire) = decode_key(&mut remaining)
            .map_err(|_| wire_error("external_response_wire_invalid"))?;
        if field == 11 {
            if wire != WireType::LengthDelimited {
                return Err(wire_error("external_response_wire_invalid"));
            }
            let length = decode_varint(&mut remaining)
                .ok()
                .and_then(|value| usize::try_from(value).ok())
                .ok_or_else(|| wire_error("external_response_wire_invalid"))?;
            if length > remaining.remaining() {
                return Err(wire_error("external_response_wire_invalid"));
            }
            if length != 0 {
                return Err(wire_error("external_source_field_conflict"));
            }
            remaining.advance(length);
            continue;
        }
        skip_field(wire, field, &mut remaining, DecodeContext::default())
            .map_err(|_| wire_error("external_response_wire_invalid"))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "external_query_transport_tests.rs"]
mod tests;
