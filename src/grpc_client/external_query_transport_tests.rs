use super::*;
use crate::grpc_client::bundle::ClientBundleConfig;
use crate::grpc_client::client::external_query_wire_fixture::ExternalQueryWireFixture;
use futures::future::poll_fn;
use futures::FutureExt as _;
use http_body::Body as _;
use std::collections::VecDeque;
use std::time::Duration;
use tonic::transport::{Certificate, ClientTlsConfig, Identity};

struct TestBody {
    frames: VecDeque<Result<Frame<Bytes>, tonic::Status>>,
    done: bool,
}

impl TestBody {
    fn new(frames: impl IntoIterator<Item = Result<Frame<Bytes>, tonic::Status>>) -> Self {
        Self {
            frames: frames.into_iter().collect(),
            done: false,
        }
    }

    fn data(chunks: impl IntoIterator<Item = Vec<u8>>) -> Self {
        Self::new(
            chunks
                .into_iter()
                .map(|chunk| Ok(Frame::data(Bytes::from(chunk)))),
        )
    }
}

impl http_body::Body for TestBody {
    type Data = Bytes;
    type Error = tonic::Status;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        match self.frames.pop_front() {
            Some(frame) => {
                self.done = self.frames.is_empty();
                Poll::Ready(Some(frame))
            }
            None => {
                self.done = true;
                Poll::Ready(None)
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        self.done
    }
}

fn capture_body(
    method: ExternalQueryMethod,
    body: TestBody,
) -> (CapturedBody, CaptureHandle) {
    let capture = CaptureHandle::new(method);
    (
        CapturedBody {
            body: Body::new(body),
            capture: capture.clone(),
        },
        capture,
    )
}

#[derive(Debug, Eq, PartialEq)]
enum ForwardedFrame {
    Data(Vec<u8>),
    Trailers(http::HeaderMap),
}

async fn drain(mut body: CapturedBody) -> Result<Vec<ForwardedFrame>, tonic::Status> {
    let mut forwarded = Vec::new();
    while let Some(frame) = poll_fn(|cx| Pin::new(&mut body).poll_frame(cx)).await {
        let frame = frame?;
        match frame.into_data() {
            Ok(data) => forwarded.push(ForwardedFrame::Data(data.to_vec())),
            Err(frame) => forwarded.push(ForwardedFrame::Trailers(
                frame
                    .into_trailers()
                    .expect("TEST_CODE body frame must be data or trailers"),
            )),
        }
    }
    Ok(forwarded)
}

fn framed(payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(payload.len() + 5);
    bytes.push(0);
    bytes.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    bytes.extend_from_slice(payload);
    bytes
}

async fn invalid_evidence(body: Vec<u8>) -> (GrpcError, ExternalWireEvidenceV1) {
    let (captured, handle) = capture_body(
        ExternalQueryMethod::GlobalNews,
        TestBody::data([body]),
    );
    drain(captured)
        .await
        .expect("TEST_CODE drain invalid body");
    handle
        .evidence()
        .expect_err("TEST_CODE invalid frame evidence")
}

async fn controlled_channel(
    fixture: &ExternalQueryWireFixture,
) -> CapturedExternalChannel {
    let ClientBundleConfig {
        endpoint_uri,
        tls_server_name,
        ca_pem,
        certificate_pem,
        private_key_pem,
        ..
    } = crate::grpc_client::bundle::load(fixture.bundle_path())
        .expect("TEST_CODE binding fixture bundle");
    let tls = ClientTlsConfig::new()
        .domain_name(tls_server_name)
        .ca_certificate(Certificate::from_pem(ca_pem))
        .identity(Identity::from_pem(
            certificate_pem,
            private_key_pem.as_slice(),
        ));
    let endpoint = Channel::from_shared(endpoint_uri)
        .expect("TEST_CODE binding endpoint")
        .tls_config(tls)
        .expect("TEST_CODE binding TLS");
    let channel = tokio::time::timeout(Duration::from_secs(5), endpoint.connect())
        .await
        .expect("TEST_CODE binding connect deadline")
        .expect("TEST_CODE binding fixture connect");
    CapturedExternalChannel(channel)
}

async fn binding_error(
    service: &mut CapturedExternalChannel,
    handle: Option<ExternalQueryMethod>,
    path: &'static str,
    grpc_method: Option<tonic::GrpcMethod<'static>>,
) -> CapturedExternalChannelError {
    tokio::time::timeout(
        Duration::from_secs(5),
        poll_fn(|cx| service.poll_ready(cx)),
    )
    .await
    .expect("TEST_CODE controlled channel readiness deadline")
    .expect("TEST_CODE controlled channel ready");
    let mut request = http::Request::builder()
        .uri(path)
        .body(Body::empty())
        .expect("TEST_CODE binding request");
    if let Some(method) = handle {
        request.extensions_mut().insert(CaptureHandle::new(method));
    }
    if let Some(grpc_method) = grpc_method {
        request.extensions_mut().insert(grpc_method);
    }
    tokio::time::timeout(
        Duration::from_secs(5),
        Service::call(service, request),
    )
    .await
    .expect("TEST_CODE binding rejection deadline")
    .expect_err("TEST_CODE binding must fail before inner call")
}

#[tokio::test]
async fn captured_channel_rejects_missing_or_mismatched_closed_binding_before_inner_call() {
    let mut fixture = None;
    let outcome = std::panic::AssertUnwindSafe(tokio::time::timeout(
        Duration::from_secs(30),
        async {
            fixture = Some(
                ExternalQueryWireFixture::bind()
                    .await
                    .expect("TEST_CODE binding fixture"),
            );
            let fixture = fixture.as_ref().expect("TEST_CODE binding fixture owner");
            let mut channel = controlled_channel(fixture).await;
            let service = ExternalQueryMethod::SERVICE;
            let global_path = ExternalQueryMethod::GlobalNews.path();
            assert!(matches!(
                binding_error(
                    &mut channel,
                    None,
                    global_path,
                    Some(tonic::GrpcMethod::new(service, "GlobalNews")),
                )
                .await,
                CapturedExternalChannelError::Binding
            ));
            assert!(matches!(
                binding_error(
                    &mut channel,
                    Some(ExternalQueryMethod::GlobalNews),
                    global_path,
                    None,
                )
                .await,
                CapturedExternalChannelError::Binding
            ));
            for (path, grpc_service, grpc_method) in [
                (
                    ExternalQueryMethod::SecurityMetadata.path(),
                    service,
                    "GlobalNews",
                ),
                (global_path, "TEST_CODE.WrongService", "GlobalNews"),
                (global_path, service, "InstrumentNews"),
            ] {
                assert!(matches!(
                    binding_error(
                        &mut channel,
                        Some(ExternalQueryMethod::GlobalNews),
                        path,
                        Some(tonic::GrpcMethod::new(grpc_service, grpc_method)),
                    )
                    .await,
                    CapturedExternalChannelError::Binding
                ));
            }
            let observation = fixture.snapshot();
            assert_eq!(observation.tcp_accepts, 1);
            assert_eq!(observation.calls, 0);
            assert!(observation.methods.is_empty());
            assert!(observation.requests.is_empty());
        },
    ))
    .catch_unwind()
    .await;
    let cleanup = match fixture.take() {
        Some(fixture) => fixture.finish().await,
        None => Ok(()),
    };
    if let Err(error) = cleanup {
        panic!("TEST_CODE binding cleanup failed: {error}");
    }
    match outcome {
        Ok(result) => result.expect("TEST_CODE binding test body deadline"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test]
async fn captured_body_forwards_chunks_and_trailers_and_accepts_exact_limit() {
    let payload = b"TEST_CODE_chunked_external_payload";
    let body = framed(payload);
    let split = 3;
    let mut trailers = http::HeaderMap::new();
    trailers.insert(
        "x-test-code-trailer",
        http::HeaderValue::from_static("TEST_CODE_present"),
    );
    let frames = vec![
        Ok(Frame::data(Bytes::copy_from_slice(&body[..split]))),
        Ok(Frame::data(Bytes::copy_from_slice(&body[split..]))),
        Ok(Frame::trailers(trailers.clone())),
    ];
    let (captured, handle) = capture_body(ExternalQueryMethod::GlobalNews, TestBody::new(frames));
    assert_eq!(
        drain(captured).await.unwrap(),
        vec![
            ForwardedFrame::Data(body[..split].to_vec()),
            ForwardedFrame::Data(body[split..].to_vec()),
            ForwardedFrame::Trailers(trailers),
        ]
    );
    let evidence = handle.evidence().expect("TEST_CODE chunked payload evidence");
    assert_eq!(evidence.payload(), Some(&payload[..]));
    evidence
        .validate(ExternalQueryMethod::GlobalNews)
        .expect("TEST_CODE valid chunked evidence");

    let payload = vec![b'x'; EXTERNAL_QUERY_DECODE_LIMIT_BYTES];
    let body = framed(&payload);
    assert_eq!(body.len(), EXTERNAL_QUERY_FRAMED_BODY_LIMIT_BYTES);
    let (captured, handle) = capture_body(
        ExternalQueryMethod::SecurityMetadata,
        TestBody::data([body.clone()]),
    );
    assert_eq!(
        drain(captured).await.unwrap(),
        vec![ForwardedFrame::Data(body)]
    );
    assert_eq!(
        handle.evidence().unwrap().payload(),
        Some(payload.as_slice())
    );
}

#[tokio::test]
async fn captured_body_closes_zero_payload_second_frame_and_body_error_boundaries() {
    let zero = framed(&[]);
    let (captured, handle) = capture_body(
        ExternalQueryMethod::GlobalNews,
        TestBody::data([zero.clone()]),
    );
    assert_eq!(
        drain(captured).await.unwrap(),
        vec![ForwardedFrame::Data(zero)]
    );
    let evidence = handle.evidence().expect("TEST_CODE zero payload evidence");
    assert_eq!(evidence.payload(), Some(&[][..]));
    evidence.validate(ExternalQueryMethod::GlobalNews).unwrap();

    let two_frames = [framed(b"first"), framed(b"second")].concat();
    let (error, evidence) = invalid_evidence(two_frames).await;
    assert_eq!(error.details().code, "external_response_wire_invalid");
    assert!(matches!(
        evidence.evidence,
        ExternalWireMaterialV1::InvalidFrame {
            failure: ExternalFrameFailureV1::TrailingData,
            ..
        }
    ));

    let expected = tonic::Status::data_loss("TEST_CODE body failure");
    let expected_code = expected.code();
    let expected_message = expected.message().to_owned();
    let (captured, _) = capture_body(
        ExternalQueryMethod::InstrumentNews,
        TestBody::new([Err(expected)]),
    );
    let actual = drain(captured)
        .await
        .expect_err("TEST_CODE body failure must be forwarded");
    assert_eq!(actual.code(), expected_code);
    assert_eq!(actual.message(), expected_message);
}

#[tokio::test]
async fn captured_body_closes_missing_overflow_and_each_framing_subkind() {
    let (captured, handle) = capture_body(
        ExternalQueryMethod::InstrumentNews,
        TestBody::data(Vec::<Vec<u8>>::new()),
    );
    assert!(drain(captured).await.unwrap().is_empty());
    let (_, missing) = handle.evidence().expect_err("TEST_CODE missing evidence");
    assert!(matches!(missing.evidence, ExternalWireMaterialV1::Missing { .. }));
    missing.validate(ExternalQueryMethod::InstrumentNews).unwrap();

    let over = vec![0; EXTERNAL_QUERY_FRAMED_BODY_LIMIT_BYTES + 1];
    let (captured, handle) = capture_body(
        ExternalQueryMethod::SecurityMetadata,
        TestBody::data([over.clone()]),
    );
    assert_eq!(
        drain(captured).await.unwrap(),
        vec![ForwardedFrame::Data(over)]
    );
    let (_, overflow) = handle.evidence().expect_err("TEST_CODE overflow evidence");
    assert!(matches!(
        overflow.evidence,
        ExternalWireMaterialV1::Overflow {
            observed_framed_body_bytes_at_least,
            ..
        } if observed_framed_body_bytes_at_least == EXTERNAL_QUERY_FRAMED_BODY_LIMIT_BYTES + 1
    ));
    overflow.validate(ExternalQueryMethod::SecurityMetadata).unwrap();

    let declared_over = (EXTERNAL_QUERY_DECODE_LIMIT_BYTES as u32 + 1).to_be_bytes();
    let cases = [
        (vec![0, 0, 0, 0], ExternalFrameFailureV1::HeaderTruncated),
        (
            vec![1, 0, 0, 0, 0],
            ExternalFrameFailureV1::CompressionUnsupported,
        ),
        (
            [vec![0], declared_over.to_vec()].concat(),
            ExternalFrameFailureV1::PayloadLengthExceedsLimit,
        ),
        (
            vec![0, 0, 0, 0, 2, b'x'],
            ExternalFrameFailureV1::PayloadTruncated,
        ),
        (
            vec![0, 0, 0, 0, 1, b'x', 0],
            ExternalFrameFailureV1::TrailingData,
        ),
    ];
    for (body, expected_failure) in cases {
        let (error, evidence) = invalid_evidence(body).await;
        assert_eq!(error.details().code, "external_response_wire_invalid");
        assert!(matches!(
            evidence.evidence,
            ExternalWireMaterialV1::InvalidFrame { failure, .. }
                if failure == expected_failure
        ));
        evidence.validate(ExternalQueryMethod::GlobalNews).unwrap();
    }
}

#[tokio::test]
async fn invalid_frame_evidence_rejects_empty_body_or_changed_subkind() {
    let empty = ExternalWireEvidenceV1::new(
        ExternalQueryMethod::GlobalNews,
        ExternalWireMaterialV1::InvalidFrame {
            failure: ExternalFrameFailureV1::HeaderTruncated,
            grpc_body_bytes: Vec::new(),
            body_sha256: hex::encode(Sha256::digest(b"")),
            framed_body_limit_bytes: EXTERNAL_QUERY_FRAMED_BODY_LIMIT_BYTES,
        },
    );
    assert!(empty.validate(ExternalQueryMethod::GlobalNews).is_err());

    let (_, mut evidence) = invalid_evidence(vec![1, 0, 0, 0, 0]).await;
    let ExternalWireMaterialV1::InvalidFrame { failure, .. } = &mut evidence.evidence else {
        panic!("TEST_CODE expected invalid frame evidence");
    };
    *failure = ExternalFrameFailureV1::PayloadTruncated;
    assert!(evidence.validate(ExternalQueryMethod::GlobalNews).is_err());
}

#[test]
fn evidence_serialization_uses_fixed_tag_and_proto_method_identity() {
    let evidence = ExternalWireEvidenceV1::new(
        ExternalQueryMethod::GlobalNews,
        ExternalWireMaterialV1::Missing {
            framed_body_limit_bytes: EXTERNAL_QUERY_FRAMED_BODY_LIMIT_BYTES,
        },
    );
    evidence.validate(ExternalQueryMethod::GlobalNews).unwrap();
    let value = serde_json::to_value(evidence).unwrap();
    assert_eq!(value["material"], "external-unary-response-evidence-v1");
    assert_eq!(value["profile"], "ExternalV1");
    assert_eq!(value["method"], "OPERATION_GLOBAL_NEWS");
    assert_eq!(
        value["client_descriptor_sha256"],
        EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256
    );
}
