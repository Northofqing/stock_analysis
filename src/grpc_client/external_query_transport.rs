use crate::grpc_client::errors::{ErrorDetail, GrpcError};
use crate::grpc_client::external_pb::magic::market::v1::{
    market_data_service_client::MarketDataServiceClient, QueryRequest, QueryResponse,
};
use http_body::{Frame, SizeHint};
use prost::bytes::Buf as _;
use prost::encoding::{decode_key, decode_varint, skip_field, DecodeContext, WireType};
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
#[derive(Clone, Copy)]
pub(crate) enum ExternalQueryLimit {
    Existing,
    OrdinaryWindow,
}
impl ExternalQueryLimit {
    fn bytes(self) -> usize {
        match self {
            Self::Existing => EXTERNAL_QUERY_DECODE_LIMIT_BYTES,
            Self::OrdinaryWindow => 8 * 1024 * 1024,
        }
    }
}
const EXTERNAL_WIRE_MATERIAL: &str = "external-unary-response-evidence-v1";
// The 2026-10-01.3 public bundle compiles to this client descriptor. The VM
// build identity's server contract digest is a distinct release identity.
pub(crate) const EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256: &str =
    "41db4b931010d7dfed7240dd1713ad85dac91ddee6338265737fcc6970ace90b";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) enum ExternalQueryMethod {
    #[serde(rename = "OPERATION_HISTORICAL_BARS")]
    HistoricalBars,
    #[serde(rename = "OPERATION_MONEY_FLOWS")]
    MoneyFlows,
    #[serde(rename = "OPERATION_BOARD_FLOWS")]
    BoardFlows,
    #[serde(rename = "OPERATION_FUTURES_DELIVERY")]
    FuturesDelivery,
    #[serde(rename = "OPERATION_SECURITY_METADATA")]
    SecurityMetadata,
    #[serde(rename = "OPERATION_MARKET_ANNOUNCEMENTS")]
    MarketAnnouncements,
    #[serde(rename = "OPERATION_GLOBAL_NEWS")]
    GlobalNews,
    #[serde(rename = "OPERATION_INSTRUMENT_NEWS")]
    InstrumentNews,
    #[serde(rename = "OPERATION_CURRENT_AUCTION_OBSERVATIONS")]
    CurrentAuctionObservations,
    #[serde(rename = "OPERATION_ECONOMIC_RELEASE_OBSERVATIONS")]
    EconomicReleaseObservations,
    #[serde(rename = "OPERATION_ECONOMIC_RELEASE_SCHEDULE")]
    EconomicReleaseSchedule,
}

impl ExternalQueryMethod {
    const SERVICE: &'static str = "magic.market.v1.MarketDataService";

    fn generated_method(self) -> &'static str {
        match self {
            Self::HistoricalBars => "HistoricalBars",
            Self::MoneyFlows => "MoneyFlows",
            Self::BoardFlows => "BoardFlows",
            Self::FuturesDelivery => "FuturesDelivery",
            Self::SecurityMetadata => "SecurityMetadata",
            Self::MarketAnnouncements => "MarketAnnouncements",
            Self::GlobalNews => "GlobalNews",
            Self::InstrumentNews => "InstrumentNews",
            Self::CurrentAuctionObservations => "CurrentAuctionObservations",
            Self::EconomicReleaseObservations => "EconomicReleaseObservations",
            Self::EconomicReleaseSchedule => "EconomicReleaseSchedule",
        }
    }

    fn path(self) -> &'static str {
        match self {
            Self::HistoricalBars => "/magic.market.v1.MarketDataService/HistoricalBars",
            Self::MoneyFlows => "/magic.market.v1.MarketDataService/MoneyFlows",
            Self::BoardFlows => "/magic.market.v1.MarketDataService/BoardFlows",
            Self::FuturesDelivery => "/magic.market.v1.MarketDataService/FuturesDelivery",
            Self::SecurityMetadata => "/magic.market.v1.MarketDataService/SecurityMetadata",
            Self::MarketAnnouncements => "/magic.market.v1.MarketDataService/MarketAnnouncements",
            Self::GlobalNews => "/magic.market.v1.MarketDataService/GlobalNews",
            Self::InstrumentNews => "/magic.market.v1.MarketDataService/InstrumentNews",
            Self::CurrentAuctionObservations => {
                "/magic.market.v1.MarketDataService/CurrentAuctionObservations"
            }
            Self::EconomicReleaseObservations => {
                "/magic.market.v1.MarketDataService/EconomicReleaseObservations"
            }
            Self::EconomicReleaseSchedule => {
                "/magic.market.v1.MarketDataService/EconomicReleaseSchedule"
            }
        }
    }

    fn matches_binding(self, path: &str, grpc_method: Option<&tonic::GrpcMethod<'static>>) -> bool {
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
            Operation::MarketAnnouncements => Some(Self::MarketAnnouncements),
            Operation::GlobalNews => Some(Self::GlobalNews),
            Operation::InstrumentNews => Some(Self::InstrumentNews),
            _ => None,
        }
    }

    pub(crate) fn from_external_operation(
        operation: crate::grpc_client::external_pb::magic::market::v1::Operation,
    ) -> Option<Self> {
        use crate::grpc_client::external_pb::magic::market::v1::Operation;
        match operation {
            Operation::HistoricalBars => Some(Self::HistoricalBars),
            Operation::MoneyFlows => Some(Self::MoneyFlows),
            Operation::BoardFlows => Some(Self::BoardFlows),
            Operation::FuturesDelivery => Some(Self::FuturesDelivery),
            Operation::CurrentAuctionObservations => Some(Self::CurrentAuctionObservations),
            Operation::EconomicReleaseObservations => Some(Self::EconomicReleaseObservations),
            Operation::EconomicReleaseSchedule => Some(Self::EconomicReleaseSchedule),
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
        self.validate_bound(
            method,
            self.client_descriptor_sha256 == EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256
                && compiled_descriptor_sha256() == EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256,
        )
    }

    pub(crate) fn validate_historical(&self, method: ExternalQueryMethod) -> Result<(), GrpcError> {
        self.validate_bound(
            method,
            crate::grpc_client::historical_external::accepts_descriptor(
                &self.client_descriptor_sha256,
            ),
        )
    }

    pub(crate) fn validate_descriptor(
        &self,
        method: ExternalQueryMethod,
        descriptor: &str,
    ) -> Result<(), GrpcError> {
        self.validate_bound(
            method,
            self.client_descriptor_sha256 == descriptor
                && super::external_decoder::ExternalDecoder::for_descriptor(descriptor).is_ok(),
        )
    }

    pub(crate) fn validate_window(&self, descriptor: &str) -> Result<(), GrpcError> {
        self.validate_bound_limit(
            ExternalQueryMethod::HistoricalBars,
            self.client_descriptor_sha256 == descriptor
                && super::external_decoder::ExternalDecoder::for_descriptor(descriptor).is_ok(),
            ExternalQueryLimit::OrdinaryWindow,
        )
    }
    fn validate_bound(
        &self,
        method: ExternalQueryMethod,
        descriptor_valid: bool,
    ) -> Result<(), GrpcError> {
        self.validate_bound_limit(method, descriptor_valid, ExternalQueryLimit::Existing)
    }
    fn validate_bound_limit(
        &self,
        method: ExternalQueryMethod,
        descriptor_valid: bool,
        limit: ExternalQueryLimit,
    ) -> Result<(), GrpcError> {
        if self.material != EXTERNAL_WIRE_MATERIAL
            || self.profile != "ExternalV1"
            || self.method != method
            || !descriptor_valid
        {
            return Err(wire_error("external_response_wire_invalid"));
        }
        match &self.evidence {
            ExternalWireMaterialV1::Payload {
                protobuf_payload,
                payload_sha256,
                decode_limit_bytes,
            } => {
                if *decode_limit_bytes != limit.bytes()
                    || protobuf_payload.len() > *decode_limit_bytes
                    || *payload_sha256 != hex::encode(Sha256::digest(protobuf_payload))
                {
                    return Err(wire_error("external_response_wire_invalid"));
                }
            }
            ExternalWireMaterialV1::Missing {
                framed_body_limit_bytes,
            } => {
                if *framed_body_limit_bytes != limit.bytes() + 5 {
                    return Err(wire_error("external_response_wire_invalid"));
                }
            }
            ExternalWireMaterialV1::Overflow {
                framed_body_limit_bytes,
                observed_framed_body_bytes_at_least,
            } => {
                if *framed_body_limit_bytes != limit.bytes() + 5
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
                if *framed_body_limit_bytes != limit.bytes() + 5
                    || grpc_body_bytes.is_empty()
                    || grpc_body_bytes.len() > *framed_body_limit_bytes
                    || *body_sha256 != hex::encode(Sha256::digest(grpc_body_bytes))
                    || parse_uncompressed_unary_frame_limit(grpc_body_bytes, limit.bytes()).err()
                        != Some(*failure)
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
    channel: Channel,
}

impl ExternalQueryTransport {
    pub(crate) fn new(channel: Channel) -> Self {
        let client = MarketDataServiceClient::new(CapturedExternalChannel(channel.clone()))
            .max_decoding_message_size(EXTERNAL_QUERY_DECODE_LIMIT_BYTES);
        Self { client, channel }
    }

    pub(crate) async fn call_with_descriptor(
        &mut self,
        method: ExternalQueryMethod,
        request: tonic::Request<QueryRequest>,
        descriptor: &str,
    ) -> ExternalQueryCall {
        self.call_with_limit(method, request, descriptor, ExternalQueryLimit::Existing)
            .await
    }
    pub(crate) async fn call_window(
        &mut self,
        request: tonic::Request<QueryRequest>,
        descriptor: &str,
    ) -> ExternalQueryCall {
        self.call_with_limit(
            ExternalQueryMethod::HistoricalBars,
            request,
            descriptor,
            ExternalQueryLimit::OrdinaryWindow,
        )
        .await
    }
    async fn call_with_limit(
        &mut self,
        method: ExternalQueryMethod,
        mut request: tonic::Request<QueryRequest>,
        descriptor: &str,
        limit: ExternalQueryLimit,
    ) -> ExternalQueryCall {
        if let Err(error) = super::external_decoder::ExternalDecoder::for_descriptor(descriptor)
            .and_then(|decoder| {
                decoder.query_request(&prost::Message::encode_to_vec(request.get_ref()))
            })
        {
            let mut evidence = ExternalWireEvidenceV1::new(
                method,
                ExternalWireMaterialV1::Missing {
                    framed_body_limit_bytes: limit.bytes() + 5,
                },
            );
            evidence.client_descriptor_sha256 = descriptor.to_owned();
            return ExternalQueryCall::LocalWireFailure { error, evidence };
        }
        let capture = CaptureHandle::with_limit(method, limit);
        request.extensions_mut().insert(capture.clone());
        let mut client = self.client.clone().max_decoding_message_size(limit.bytes());
        let response = if matches!(limit, ExternalQueryLimit::OrdinaryWindow) {
            let mut grpc = tonic::client::Grpc::new(CapturedExternalChannel(self.channel.clone()))
                .max_decoding_message_size(limit.bytes());
            match grpc.ready().await {
                Err(e) => Err(tonic::Status::unavailable(e.to_string())),
                Ok(()) => {
                    request.extensions_mut().insert(tonic::GrpcMethod::new(
                        ExternalQueryMethod::SERVICE,
                        method.generated_method(),
                    ));
                    grpc.unary(
                        request,
                        http::uri::PathAndQuery::from_static(method.path()),
                        WindowCodec,
                    )
                    .await
                }
            }
        } else {
            match method {
                ExternalQueryMethod::HistoricalBars => client.historical_bars(request).await,
                ExternalQueryMethod::MoneyFlows => client.money_flows(request).await,
                ExternalQueryMethod::BoardFlows => client.board_flows(request).await,
                ExternalQueryMethod::FuturesDelivery => client.futures_delivery(request).await,
                ExternalQueryMethod::SecurityMetadata => client.security_metadata(request).await,
                ExternalQueryMethod::MarketAnnouncements => {
                    client.market_announcements(request).await
                }
                ExternalQueryMethod::GlobalNews => client.global_news(request).await,
                ExternalQueryMethod::InstrumentNews => client.instrument_news(request).await,
                ExternalQueryMethod::CurrentAuctionObservations => {
                    client.current_auction_observations(request).await
                }
                ExternalQueryMethod::EconomicReleaseObservations => {
                    client.economic_release_observations(request).await
                }
                ExternalQueryMethod::EconomicReleaseSchedule => {
                    client.economic_release_schedule(request).await
                }
            }
        };
        let bind = |mut evidence: ExternalWireEvidenceV1| {
            evidence.client_descriptor_sha256 = descriptor.to_owned();
            evidence
        };
        match response {
            Err(status) => ExternalQueryCall::UnaryStatus {
                status,
                evidence: bind(capture.observed()),
            },
            Ok(response) => match capture.evidence() {
                Ok(evidence) => {
                    let evidence = bind(evidence);
                    let _ = response;
                    match super::external_decoder::ExternalDecoder::for_descriptor(descriptor)
                        .and_then(|decoder| {
                            decoder.query(
                                evidence
                                    .payload()
                                    .ok_or_else(|| wire_error("external_response_wire_invalid"))?,
                            )
                        }) {
                        Ok(message) => ExternalQueryCall::Response { message, evidence },
                        Err(error) => ExternalQueryCall::LocalWireFailure { error, evidence },
                    }
                }
                Err((error, evidence)) => ExternalQueryCall::LocalWireFailure {
                    error,
                    evidence: bind(evidence),
                },
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
    type Future =
        Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send + 'static>>;

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
            request.extensions().get::<tonic::GrpcMethod<'static>>(),
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
    limit: ExternalQueryLimit,
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
        Self::with_limit(method, ExternalQueryLimit::Existing)
    }
    fn with_limit(method: ExternalQueryMethod, limit: ExternalQueryLimit) -> Self {
        Self {
            state: Arc::new(Mutex::new(CaptureState::default())),
            method,
            limit,
        }
    }

    /// Body material exactly as captured, without judging whether it is a
    /// usable response.
    fn observed(&self) -> ExternalWireEvidenceV1 {
        let state = self.state.lock().expect("external capture mutex poisoned");
        let material = if state.overflow {
            ExternalWireMaterialV1::Overflow {
                framed_body_limit_bytes: self.limit.bytes() + 5,
                observed_framed_body_bytes_at_least: state.observed,
            }
        } else if !state.ended || state.bytes.is_empty() {
            ExternalWireMaterialV1::Missing {
                framed_body_limit_bytes: self.limit.bytes() + 5,
            }
        } else {
            match parse_uncompressed_unary_frame_limit(&state.bytes, self.limit.bytes()) {
                Ok(payload) => ExternalWireMaterialV1::Payload {
                    protobuf_payload: payload.to_vec(),
                    payload_sha256: hex::encode(Sha256::digest(payload)),
                    decode_limit_bytes: self.limit.bytes(),
                },
                Err(failure) => ExternalWireMaterialV1::InvalidFrame {
                    failure,
                    grpc_body_bytes: state.bytes.clone(),
                    body_sha256: hex::encode(Sha256::digest(&state.bytes)),
                    framed_body_limit_bytes: self.limit.bytes() + 5,
                },
            }
        };
        ExternalWireEvidenceV1::new(self.method, material)
    }

    fn evidence(&self) -> Result<ExternalWireEvidenceV1, (GrpcError, ExternalWireEvidenceV1)> {
        let evidence = self.observed();
        match &evidence.evidence {
            ExternalWireMaterialV1::Payload { .. } => match evidence.validate_bound_limit(
                self.method,
                compiled_descriptor_sha256() == EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256,
                self.limit,
            ) {
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
                    if state.observed <= self.capture.limit.bytes() + 5 {
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

fn parse_uncompressed_unary_frame(bytes: &[u8]) -> Result<&[u8], ExternalFrameFailureV1> {
    parse_uncompressed_unary_frame_limit(bytes, EXTERNAL_QUERY_DECODE_LIMIT_BYTES)
}
fn parse_uncompressed_unary_frame_limit(
    bytes: &[u8],
    limit: usize,
) -> Result<&[u8], ExternalFrameFailureV1> {
    if bytes.len() < 5 {
        return Err(ExternalFrameFailureV1::HeaderTruncated);
    }
    if bytes[0] != 0 {
        return Err(ExternalFrameFailureV1::CompressionUnsupported);
    }
    let declared = u32::from_be_bytes(bytes[1..5].try_into().expect("five-byte header")) as usize;
    if declared > limit {
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

pub(crate) fn compiled_descriptor_sha256() -> String {
    hex::encode(Sha256::digest(
        crate::grpc_client::external_pb::FILE_DESCRIPTOR_SET,
    ))
}

pub(crate) fn admit_external_payload(payload: &[u8]) -> Result<(), GrpcError> {
    let mut remaining = payload;
    while remaining.has_remaining() {
        let (field, wire) =
            decode_key(&mut remaining).map_err(|_| wire_error("external_response_wire_invalid"))?;
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

/// WG07 checks protobuf scalar and canonical JSON limits before Prost creates
/// owned strings/vectors. The old generated codec and its wire bytes are unchanged.
struct WindowCodec;
struct WindowDecoder;
impl tonic::codec::Codec for WindowCodec {
    type Encode = QueryRequest;
    type Decode = QueryResponse;
    type Encoder = tonic_prost::ProstEncoder<QueryRequest>;
    type Decoder = WindowDecoder;
    fn encoder(&mut self) -> Self::Encoder {
        tonic_prost::ProstEncoder::new(tonic::codec::BufferSettings::default())
    }
    fn decoder(&mut self) -> Self::Decoder {
        WindowDecoder
    }
}
impl tonic::codec::Decoder for WindowDecoder {
    type Item = QueryResponse;
    type Error = tonic::Status;
    fn decode(
        &mut self,
        buf: &mut tonic::codec::DecodeBuf<'_>,
    ) -> Result<Option<QueryResponse>, tonic::Status> {
        use prost::Message;
        let remaining = buf.remaining();
        if remaining > 8 * 1024 * 1024 || buf.chunk().len() != remaining {
            return Err(tonic::Status::resource_exhausted("WG07 decode buffer"));
        }
        preflight_window_protobuf(buf.chunk(), false)
            .map_err(|_| tonic::Status::resource_exhausted("WG07 response resource limit"))?;
        QueryResponse::decode(buf)
            .map(Some)
            .map_err(|_| tonic::Status::data_loss("WG07 protobuf"))
    }
}
fn preflight_window_protobuf(mut bytes: &[u8], record: bool) -> Result<(), GrpcError> {
    let mut records = 0;
    while !bytes.is_empty() {
        let (tag, wire) =
            decode_key(&mut bytes).map_err(|_| wire_error("ordinary_window_protobuf"))?;
        if wire == WireType::LengthDelimited {
            let len = usize::try_from(
                decode_varint(&mut bytes).map_err(|_| wire_error("ordinary_window_protobuf"))?,
            )
            .map_err(|_| wire_error("ordinary_window_protobuf"))?;
            if len > bytes.len() {
                return Err(wire_error("ordinary_window_protobuf"));
            }
            let value = &bytes[..len];
            bytes = &bytes[len..];
            if !record && tag == 9 {
                records += 1;
                if records > 1 {
                    return Err(wire_error("ordinary_window_record_count"));
                }
                preflight_window_protobuf(value, true)?;
            } else if record && tag == 4 {
                crate::data_gateway::ordinary_daily_change_window_contract::preflight(
                    value,
                    8 * 1024 * 1024,
                )
                .map_err(|_| wire_error("ordinary_window_json_limit"))?;
            } else if len > 16 * 1024 {
                return Err(wire_error("ordinary_window_scalar_limit"));
            }
        } else {
            skip_field(wire, tag, &mut bytes, DecodeContext::default())
                .map_err(|_| wire_error("ordinary_window_protobuf"))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod window_limit_tests {
    use super::*;
    #[test]
    fn wg07_closed_eight_mib_limit_preserves_old_wire_bytes_and_default() {
        let old = ExternalWireEvidenceV1::new(
            ExternalQueryMethod::HistoricalBars,
            ExternalWireMaterialV1::Missing {
                framed_body_limit_bytes: 4194309,
            },
        );
        assert_eq!(
            serde_json::to_string(&old).unwrap(),
            r#"{"material":"external-unary-response-evidence-v1","profile":"ExternalV1","method":"OPERATION_HISTORICAL_BARS","client_descriptor_sha256":"41db4b931010d7dfed7240dd1713ad85dac91ddee6338265737fcc6970ace90b","evidence":{"Missing":{"framed_body_limit_bytes":4194309}}}"#
        );
        assert!(old.validate(ExternalQueryMethod::HistoricalBars).is_ok());
        assert!(old
            .validate_window(EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256)
            .is_err());
        let window = ExternalWireEvidenceV1::new(
            ExternalQueryMethod::HistoricalBars,
            ExternalWireMaterialV1::Missing {
                framed_body_limit_bytes: 8388613,
            },
        );
        assert!(window
            .validate_window(EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256)
            .is_ok());
        assert!(window
            .validate(ExternalQueryMethod::HistoricalBars)
            .is_err());
        let n = EXTERNAL_QUERY_DECODE_LIMIT_BYTES + 1;
        let mut frame = vec![0];
        frame.extend_from_slice(&(n as u32).to_be_bytes());
        frame.resize(n + 5, 0);
        assert!(parse_uncompressed_unary_frame(&frame).is_err());
        assert_eq!(
            parse_uncompressed_unary_frame_limit(&frame, 8 * 1024 * 1024)
                .unwrap()
                .len(),
            n
        );
    }
    #[test]
    fn wg07_protobuf_preflight_rejects_large_scalars_before_owned_decode() {
        use prost::Message;
        let response = QueryResponse {
            request_id: "x".repeat(16385),
            ..Default::default()
        };
        assert!(preflight_window_protobuf(&response.encode_to_vec(), false).is_err());
        let record = crate::grpc_client::external_pb::magic::market::v1::CanonicalPayload {
            schema: "x".into(),
            schema_version: 1,
            content_type: "application/json; charset=utf-8".into(),
            data: format!("{{\"sessions\":[{}]}}", vec!["{}"; 261].join(",")).into_bytes(),
        };
        let response = QueryResponse {
            records: vec![record],
            ..Default::default()
        };
        assert!(preflight_window_protobuf(&response.encode_to_vec(), false).is_err());
    }
}
