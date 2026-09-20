use super::external_control_attempt::ExternalControlResultMaterial;
use super::macro_attempt::MacroQueryIdentity;
use super::{ContractProfile, PreparedExternalEndpoint};
use crate::data_gateway::GlobalNewsProvider;
use crate::grpc_client::external_pb::magic::market::v1::{HealthRequest, HealthResponse};
use crate::grpc_client::pb::magic::market::v1::QueryRequest;
use futures::FutureExt as _;
use prost::Message as _;
use std::time::{Duration, Instant};
use tonic::transport::Endpoint;
use zeroize::Zeroizing;

use super::external_control_loopback_fixture::{
    test_external_build_identity, test_external_observability, ExternalControlLoopbackServer,
};

const TEST_AUTHORITY: &str = "grpc-mtls:TEST_CODE-macro.external";
const TEST_BEARER: &str = "TEST_CODE_EXTERNAL_CONTROL_TOKEN";

#[tokio::test]
async fn external_health_freezes_requests_before_connect_and_preserves_native_response() {
    let mut server = None;
    let outcome =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(20), async {
            server = Some(ExternalControlLoopbackServer::bind().await);
            let server = server
                .as_ref()
                .expect("TEST_CODE External control server owner");
            let endpoint_uri = server.endpoint().to_owned();
            let endpoint = Endpoint::from_shared(endpoint_uri.clone())
                .expect("TEST_CODE External control endpoint")
                .timeout(Duration::from_secs(35));
            let prepared = PreparedExternalEndpoint::from_plaintext_for_test(
                endpoint,
                endpoint_uri.clone(),
                Zeroizing::new(TEST_BEARER.to_owned()),
                TEST_AUTHORITY.to_owned(),
            );
            assert_eq!(prepared.endpoint_uri(), endpoint_uri);

            let data = prepared
                .prepare_macro_query(MacroQueryIdentity::GlobalNews {
                    provider: GlobalNewsProvider::Eastmoney,
                    limit: 20,
                })
                .expect("TEST_CODE frozen External data request");
            assert_eq!(data.profile(), ContractProfile::ExternalV1);
            assert_eq!(data.retry_policy(), (4, 1000, 60_000, 200));
            assert_eq!(data.acquisition_authority(), TEST_AUTHORITY);
            assert_eq!(data.attempt_ordinal(), 1);
            let data_request_id = data.request_id().to_owned();
            let data_bytes = data.request_bytes();
            let decoded_data = QueryRequest::decode(data_bytes.as_slice())
                .expect("TEST_CODE decode frozen External data request");
            let data_context = decoded_data
                .context
                .expect("TEST_CODE frozen External data context");
            assert_eq!(data_context.protocol_version, 1);
            assert_eq!(data_context.request_id, data_request_id);
            assert!(!data_request_id.is_empty());
            assert_eq!(decoded_data.preferred_provider, "Eastmoney");
            assert!(!decoded_data.allow_unadmitted);
            let data_payload = decoded_data
                .payload
                .expect("TEST_CODE frozen External data payload");
            assert_eq!(data_payload.schema, "magic.market.global_news.request");
            assert_eq!(data_payload.schema_version, 2);
            assert_eq!(data_payload.content_type, "application/json; charset=utf-8");
            assert_eq!(data_payload.data, br#"{"limit":20}"#);
            assert!(!data_bytes
                .windows(TEST_BEARER.len())
                .any(|window| window == TEST_BEARER.as_bytes()));

            let health = prepared
                .prepare_health_attempt()
                .expect("TEST_CODE frozen authorized Health request");
            let health_request_id = health.request_id().to_owned();
            let health_bytes = health.request_bytes();
            let decoded_health = HealthRequest::decode(health_bytes.as_slice())
                .expect("TEST_CODE decode frozen Health request");
            assert_eq!(decoded_health.encode_to_vec(), health_bytes);
            let health_context = decoded_health
                .context
                .expect("TEST_CODE frozen Health context");
            assert_eq!(health_context.protocol_version, 1);
            assert_eq!(health_context.request_id, health_request_id);
            assert!(!health_request_id.is_empty());
            assert_ne!(health_request_id, data_request_id);
            assert!(!health_bytes
                .windows(TEST_BEARER.len())
                .any(|window| window == TEST_BEARER.as_bytes()));
            assert!(!health_bytes
                .windows(b"authorization".len())
                .any(|window| window == b"authorization"));

            tokio::task::yield_now().await;
            tokio::task::yield_now().await;
            let before_execute = server.snapshot();
            assert_eq!(before_execute.tcp_accepts, 0);
            assert!(before_execute.health_requests.is_empty());
            assert_eq!(before_execute.capabilities_calls, 0);
            assert!(before_execute.capabilities_authorized.is_empty());
            assert_eq!(before_execute.data_calls, 0);
            assert!(before_execute.data_authorized.is_empty());

            let execution = health.execute();
            tokio::pin!(execution);
            let receipt_deadline = Instant::now() + Duration::from_secs(5);
            loop {
                tokio::select! {
                    biased;
                    _completion = &mut execution => {
                        panic!("TEST_CODE Health completed before fixture release");
                    }
                    _ = tokio::task::yield_now() => {}
                }
                if !server.snapshot().health_requests.is_empty() {
                    break;
                }
                assert!(
                    Instant::now() < receipt_deadline,
                    "TEST_CODE Health receipt watchdog"
                );
            }
            let received = server.snapshot();
            assert!(received.tcp_accepts > 0);
            assert_eq!(received.health_requests, vec![health_bytes.clone()]);
            assert_eq!(received.health_authorized, vec![true]);
            assert_eq!(received.capabilities_calls, 0);
            assert!(received.capabilities_authorized.is_empty());
            assert_eq!(received.data_calls, 0);
            assert!(received.data_authorized.is_empty());

            server.release_health();
            let completion = tokio::time::timeout(Duration::from_secs(5), &mut execution)
                .await
                .expect("TEST_CODE Health completion deadline");
            let expected_response = HealthResponse {
                request_id: health_request_id.clone(),
                live: true,
                ready: true,
                state: "TEST_CODE_HEALTH_RUNNING".to_owned(),
                observability: Some(test_external_observability()),
                build_identity: Some(test_external_build_identity()),
            };
            match completion.result_material() {
                ExternalControlResultMaterial::Response { bytes, response } => {
                    assert_eq!(bytes, &expected_response.encode_to_vec());
                    assert_eq!(response.request_id, health_request_id);
                    assert!(response.live);
                    assert!(response.ready);
                    assert_eq!(response.state, "TEST_CODE_HEALTH_RUNNING");
                }
                _ => panic!("TEST_CODE expected native Health response material"),
            }
            let observed = server.snapshot();
            assert_eq!(
                observed.health_responses,
                vec![expected_response.encode_to_vec()]
            );
            assert_eq!(observed.health_requests, vec![health_bytes]);
            assert_eq!(observed.health_authorized, vec![true]);
            assert_eq!(observed.capabilities_calls, 0);
            assert!(observed.capabilities_authorized.is_empty());
            assert_eq!(observed.data_calls, 0);
            assert!(observed.data_authorized.is_empty());

            let connected = completion
                .into_connected_client()
                .expect("TEST_CODE Health response returns connected client");
            let bound = data
                .bind_connected(connected)
                .expect("TEST_CODE bind frozen data after Health");
            assert_eq!(bound.profile(), "ExternalV1");
            assert_eq!(bound.retry_policy(), (4, 1000, 60_000, 200));
            assert_eq!(bound.acquisition_authority(), Some(TEST_AUTHORITY));
            assert_eq!(bound.attempt_ordinal(), 1);
            assert_eq!(bound.request_id(), data_request_id);
            assert_eq!(bound.request_bytes(), data_bytes);
            let after_bind = server.snapshot();
            assert_eq!(after_bind.health_requests.len(), 1);
            assert_eq!(after_bind.capabilities_calls, 0);
            assert!(after_bind.capabilities_authorized.is_empty());
            assert_eq!(after_bind.data_calls, 0);
            assert!(after_bind.data_authorized.is_empty());
            drop(bound);
            drop(prepared);
        }))
        .catch_unwind()
        .await;

    let cleanup = match server.take() {
        Some(server) => server.finish().await,
        None => Ok(()),
    };
    if let Err(error) = cleanup {
        panic!("TEST_CODE External control cleanup failed: {error}");
    }
    match outcome {
        Ok(result) => result.expect("TEST_CODE External control body timeout"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

fn prepared_external_health(endpoint_uri: &str) -> PreparedExternalEndpoint {
    let endpoint = Endpoint::from_shared(endpoint_uri.to_owned())
        .expect("TEST_CODE External failure endpoint")
        .timeout(Duration::from_secs(35));
    PreparedExternalEndpoint::from_plaintext_for_test(
        endpoint,
        endpoint_uri.to_owned(),
        Zeroizing::new(TEST_BEARER.to_owned()),
        TEST_AUTHORITY.to_owned(),
    )
}

async fn execute_loopback_health(
    server: &ExternalControlLoopbackServer,
) -> (
    String,
    Vec<u8>,
    super::external_control_attempt::ExternalControlCompletion<HealthResponse>,
) {
    let prepared = prepared_external_health(server.endpoint());
    let health = prepared
        .prepare_health_attempt()
        .expect("TEST_CODE External failure Health request");
    let request_id = health.request_id().to_owned();
    let request_bytes = health.request_bytes();
    let execution = health.execute();
    tokio::pin!(execution);
    let receipt_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        tokio::select! {
            biased;
            _completion = &mut execution => {
                panic!("TEST_CODE External failure Health completed before release");
            }
            _ = tokio::task::yield_now() => {}
        }
        if !server.snapshot().health_requests.is_empty() {
            break;
        }
        assert!(
            Instant::now() < receipt_deadline,
            "TEST_CODE External failure Health receipt watchdog"
        );
    }
    let received = server.snapshot();
    assert!(received.tcp_accepts > 0);
    assert_eq!(received.health_requests, vec![request_bytes.clone()]);
    assert_eq!(received.health_authorized, vec![true]);
    assert_eq!(received.capabilities_calls, 0);
    assert!(received.capabilities_authorized.is_empty());
    assert_eq!(received.data_calls, 0);
    assert!(received.data_authorized.is_empty());
    server.release_health();
    let completion = tokio::time::timeout(Duration::from_secs(5), &mut execution)
        .await
        .expect("TEST_CODE External failure Health completion deadline");
    drop(prepared);
    (request_id, request_bytes, completion)
}

#[tokio::test]
async fn external_health_connect_failure_is_local_unavailable_without_status_or_client() {
    use crate::grpc_client::errors::{ErrorDetail as ClientErrorDetail, GrpcError};
    use tokio::net::TcpSocket;

    let mut socket = None;
    let outcome =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(20), async {
            socket = Some(TcpSocket::new_v4().expect("TEST_CODE External closed socket"));
            socket
                .as_ref()
                .expect("TEST_CODE External closed socket owner")
                .bind(
                    "127.0.0.1:0"
                        .parse()
                        .expect("TEST_CODE External closed socket address"),
                )
                .expect("TEST_CODE External closed socket bind");
            let address = socket
                .as_ref()
                .expect("TEST_CODE External held closed socket")
                .local_addr()
                .expect("TEST_CODE External closed socket local address");
            let endpoint_uri = format!("http://{address}");
            let endpoint = Endpoint::from_shared(endpoint_uri.clone())
                .expect("TEST_CODE External closed socket endpoint")
                .timeout(Duration::from_secs(35))
                .connect_timeout(Duration::from_secs(1));
            let prepared = PreparedExternalEndpoint::from_plaintext_for_test(
                endpoint,
                endpoint_uri,
                Zeroizing::new(TEST_BEARER.to_owned()),
                TEST_AUTHORITY.to_owned(),
            );
            let health = prepared
                .prepare_health_attempt()
                .expect("TEST_CODE External closed socket Health");
            let completion = tokio::time::timeout(Duration::from_secs(5), health.execute())
                .await
                .expect("TEST_CODE External connect failure deadline");
            let processed = completion
                .processed()
                .expect_err("TEST_CODE External connect must fail");
            match completion.result_material() {
                ExternalControlResultMaterial::ConnectUnavailable { error } => {
                    assert!(matches!(error, GrpcError::Unavailable { .. }));
                    assert_eq!(error.details(), &ClientErrorDetail::default());
                    assert!(std::ptr::eq(error, processed));
                }
                _ => panic!("TEST_CODE expected local ConnectUnavailable material"),
            }
            assert!(matches!(processed, GrpcError::Unavailable { .. }));
            assert_eq!(processed.details(), &ClientErrorDetail::default());
            assert!(completion.into_connected_client().is_none());
            drop(prepared);
        }))
        .catch_unwind()
        .await;
    drop(socket.take());
    match outcome {
        Ok(result) => result.expect("TEST_CODE External connect failure body timeout"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test]
async fn external_health_status_preserves_code_details_trailer_and_processed_error() {
    use super::external_control_loopback_fixture::{
        HealthReply, HealthStatusCase, ObservedHealthTrailer,
    };
    use super::macro_attempt::MacroTrailerMaterial;
    use crate::grpc_client::errors::GrpcError;
    use crate::grpc_client::external_pb::magic::market::v1::ErrorDetail as WireErrorDetail;

    for case in [
        HealthStatusCase::Absent,
        HealthStatusCase::Bytes,
        HealthStatusCase::Malformed,
    ] {
        let mut server = None;
        let outcome =
            std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(20), async {
                server = Some(
                    ExternalControlLoopbackServer::bind_with_health_reply_for_test(
                        HealthReply::Status(case),
                    )
                    .await,
                );
                let server = server
                    .as_ref()
                    .expect("TEST_CODE External Status server owner");
                let (request_id, request_bytes, completion) = execute_loopback_health(server).await;
                let wire_detail = WireErrorDetail {
                    request_id: request_id.clone(),
                    provider: "Eastmoney".to_owned(),
                    reason_code: "unavailable".to_owned(),
                    retryable: false,
                    ..WireErrorDetail::default()
                }
                .encode_to_vec();
                let expected_details = if case == HealthStatusCase::Bytes {
                    Vec::new()
                } else {
                    wire_detail.clone()
                };
                let expected_trailer = match case {
                    HealthStatusCase::Absent => ObservedHealthTrailer::Absent,
                    HealthStatusCase::Bytes => ObservedHealthTrailer::Bytes(wire_detail.clone()),
                    HealthStatusCase::Malformed => ObservedHealthTrailer::Malformed,
                };
                let observed = server.snapshot();
                assert_eq!(observed.health_requests, vec![request_bytes]);
                assert!(observed.health_responses.is_empty());
                assert_eq!(observed.health_statuses.len(), 1);
                let actual_status = &observed.health_statuses[0];
                assert_eq!(actual_status.code, tonic::Code::Unavailable as i32);
                assert_eq!(actual_status.details, expected_details);
                assert_eq!(actual_status.trailer, expected_trailer);
                assert_eq!(observed.capabilities_calls, 0);
                assert_eq!(observed.data_calls, 0);

                let processed = completion
                    .processed()
                    .expect_err("TEST_CODE External Health Status processed error");
                match completion.result_material() {
                    ExternalControlResultMaterial::Status {
                        code,
                        details,
                        error_detail_trailer,
                        error,
                    } => {
                        assert_eq!(code, actual_status.code);
                        assert_eq!(details, actual_status.details.as_slice());
                        match (error_detail_trailer, &actual_status.trailer) {
                            (MacroTrailerMaterial::Absent, ObservedHealthTrailer::Absent)
                            | (MacroTrailerMaterial::Malformed, ObservedHealthTrailer::Malformed) =>
                                {}
                            (
                                MacroTrailerMaterial::Bytes(actual),
                                ObservedHealthTrailer::Bytes(expected),
                            ) => assert_eq!(actual, expected),
                            _ => panic!("TEST_CODE Health Status trailer material mismatch"),
                        }
                        assert!(matches!(error, GrpcError::Unavailable { .. }));
                        assert!(std::ptr::eq(error, processed));
                    }
                    _ => panic!("TEST_CODE expected native Health Status material"),
                }
                assert!(matches!(processed, GrpcError::Unavailable { .. }));
                assert_eq!(
                    processed.details().code,
                    tonic::Code::Unavailable.to_string()
                );
                if case == HealthStatusCase::Malformed {
                    assert_eq!(processed.details().provider, None);
                    assert_eq!(processed.details().reason_code, None);
                    assert_eq!(processed.details().retryable, None);
                    assert_eq!(processed.details().request_id, None);
                } else {
                    assert_eq!(processed.details().provider.as_deref(), Some("Eastmoney"));
                    assert_eq!(
                        processed.details().reason_code.as_deref(),
                        Some("unavailable")
                    );
                    assert_eq!(processed.details().retryable, Some(false));
                    let correlation = processed
                        .details()
                        .request_id
                        .as_deref()
                        .expect("TEST_CODE safe Health request correlation");
                    assert!(correlation.starts_with("sha256:"));
                    assert_ne!(correlation, request_id);
                }
                assert!(completion.into_connected_client().is_none());
            }))
            .catch_unwind()
            .await;
        let cleanup = match server.take() {
            Some(server) => server.finish().await,
            None => Ok(()),
        };
        if let Err(error) = cleanup {
            panic!("TEST_CODE External Status cleanup failed: {error}");
        }
        match outcome {
            Ok(result) => result.expect("TEST_CODE External Status body timeout"),
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }
}

#[tokio::test]
async fn external_health_mismatched_request_id_preserves_response_without_client() {
    use super::external_control_loopback_fixture::HealthReply;
    use crate::grpc_client::errors::GrpcError;

    const WRONG_ID: &str = "TEST_CODE_WRONG_HEALTH_REQUEST_ID";
    let mut server = None;
    let outcome =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(20), async {
            server = Some(
                ExternalControlLoopbackServer::bind_with_health_reply_for_test(
                    HealthReply::MismatchedId,
                )
                .await,
            );
            let server = server
                .as_ref()
                .expect("TEST_CODE External mismatch server owner");
            let (request_id, request_bytes, completion) = execute_loopback_health(server).await;
            assert_ne!(request_id, WRONG_ID);
            let expected_response = HealthResponse {
                request_id: WRONG_ID.to_owned(),
                live: true,
                ready: true,
                state: "TEST_CODE_HEALTH_RUNNING".to_owned(),
                observability: Some(test_external_observability()),
                build_identity: Some(test_external_build_identity()),
            };
            let observed = server.snapshot();
            assert_eq!(observed.health_requests, vec![request_bytes]);
            assert_eq!(
                observed.health_responses,
                vec![expected_response.encode_to_vec()]
            );
            assert!(observed.health_statuses.is_empty());
            assert_eq!(observed.capabilities_calls, 0);
            assert_eq!(observed.data_calls, 0);
            match completion.result_material() {
                ExternalControlResultMaterial::Response { bytes, response } => {
                    assert_eq!(bytes, expected_response.encode_to_vec());
                    assert_eq!(response.request_id, WRONG_ID);
                    assert!(response.live);
                    assert!(response.ready);
                    assert_eq!(response.state, "TEST_CODE_HEALTH_RUNNING");
                }
                _ => panic!("TEST_CODE expected mismatched Health Response material"),
            }
            let processed = completion
                .processed()
                .expect_err("TEST_CODE mismatched Health must be rejected");
            assert!(matches!(processed, GrpcError::FailedPrecondition { .. }));
            assert_eq!(processed.details().code, "health_request_id_mismatch");
            assert_eq!(
                processed.details().reason_code.as_deref(),
                Some("health_request_id_mismatch")
            );
            assert_eq!(processed.details().retryable, Some(false));
            assert_eq!(processed.details().request_id, None);
            let diagnostic = format!("{processed:?}");
            assert!(!diagnostic.contains(WRONG_ID));
            assert!(completion.into_connected_client().is_none());
        }))
        .catch_unwind()
        .await;
    let cleanup = match server.take() {
        Some(server) => server.finish().await,
        None => Ok(()),
    };
    if let Err(error) = cleanup {
        panic!("TEST_CODE External mismatch cleanup failed: {error}");
    }
    match outcome {
        Ok(result) => result.expect("TEST_CODE External mismatch body timeout"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test]
async fn external_control_restore_reauthorizes_then_reuses_health_connection_for_capabilities() {
    use super::external_control_attempt::{ExternalControlKind, ExternalControlRequestMaterial};
    use crate::grpc_client::external_pb::magic::market::v1::{
        AdmissionState, CapabilitiesRequest, CapabilitiesResponse, Capability, Operation,
    };

    const OLD_BEARER: &str = "TEST_CODE_EXTERNAL_CONTROL_OLD_TOKEN";

    fn prepared_with_bearer(endpoint_uri: &str, bearer: &str) -> PreparedExternalEndpoint {
        let endpoint = Endpoint::from_shared(endpoint_uri.to_owned())
            .expect("TEST_CODE External restored control endpoint")
            .timeout(Duration::from_secs(35));
        PreparedExternalEndpoint::from_plaintext_for_test(
            endpoint,
            endpoint_uri.to_owned(),
            Zeroizing::new(bearer.to_owned()),
            TEST_AUTHORITY.to_owned(),
        )
    }

    fn assert_material(
        material: &ExternalControlRequestMaterial,
        kind: ExternalControlKind,
        endpoint_uri: &str,
        request_id: &str,
        request_bytes: &[u8],
    ) {
        assert_eq!(material.kind(), kind);
        assert_eq!(material.profile(), ContractProfile::ExternalV1);
        assert_eq!(material.endpoint_uri(), endpoint_uri);
        assert_eq!(material.acquisition_authority(), TEST_AUTHORITY);
        assert_eq!(material.request_id(), request_id);
        assert_eq!(material.request_bytes(), request_bytes);
    }

    let mut server = None;
    let outcome =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(30), async {
            server = Some(
                ExternalControlLoopbackServer::bind_with_capabilities_success_for_test().await,
            );
            let server = server
                .as_ref()
                .expect("TEST_CODE restored control server owner");
            let endpoint_uri = server.endpoint().to_owned();

            let old_prepared = prepared_with_bearer(&endpoint_uri, OLD_BEARER);
            let old_health = old_prepared
                .prepare_health_attempt()
                .expect("TEST_CODE old Health preparation");
            let old_capabilities = old_prepared
                .prepare_capabilities_attempt()
                .expect("TEST_CODE old Capabilities preparation");
            let health_id = old_health.request_id().to_owned();
            let health_bytes = old_health.request_bytes();
            let capabilities_id = old_capabilities.request_id().to_owned();
            let capabilities_bytes = old_capabilities.request_bytes();
            assert!(!health_id.is_empty());
            assert!(!capabilities_id.is_empty());
            assert_ne!(health_id, capabilities_id);

            let decoded_health = HealthRequest::decode(health_bytes.as_slice())
                .expect("TEST_CODE old Health decode");
            assert_eq!(decoded_health.encode_to_vec(), health_bytes);
            let health_context = decoded_health
                .context
                .expect("TEST_CODE old Health context");
            assert_eq!(health_context.protocol_version, 1);
            assert_eq!(health_context.request_id, health_id);
            let decoded_capabilities = CapabilitiesRequest::decode(capabilities_bytes.as_slice())
                .expect("TEST_CODE old Capabilities decode");
            assert_eq!(decoded_capabilities.encode_to_vec(), capabilities_bytes);
            let capabilities_context = decoded_capabilities
                .context
                .expect("TEST_CODE old Capabilities context");
            assert_eq!(capabilities_context.protocol_version, 1);
            assert_eq!(capabilities_context.request_id, capabilities_id);

            for bytes in [&health_bytes, &capabilities_bytes] {
                for forbidden in [
                    TEST_BEARER.as_bytes(),
                    OLD_BEARER.as_bytes(),
                    b"authorization".as_slice(),
                ] {
                    assert!(!bytes
                        .windows(forbidden.len())
                        .any(|window| window == forbidden));
                }
            }

            let health_material = old_health.request_material();
            let capabilities_material = old_capabilities.request_material();
            assert_material(
                &health_material,
                ExternalControlKind::Health,
                &endpoint_uri,
                &health_id,
                &health_bytes,
            );
            assert_material(
                &capabilities_material,
                ExternalControlKind::Capabilities,
                &endpoint_uri,
                &capabilities_id,
                &capabilities_bytes,
            );
            drop(old_health);
            drop(old_capabilities);
            drop(old_prepared);
            tokio::task::yield_now().await;
            tokio::task::yield_now().await;
            let before_restore = server.snapshot();
            assert_eq!(before_restore.tcp_accepts, 0);
            assert!(before_restore.health_requests.is_empty());
            assert!(before_restore.capabilities_requests.is_empty());
            assert_eq!(before_restore.capabilities_calls, 0);
            assert_eq!(before_restore.data_calls, 0);

            let current_prepared = prepared_with_bearer(&endpoint_uri, TEST_BEARER);
            let restored_health = current_prepared
                .resume_health_attempt(health_material.clone())
                .expect("TEST_CODE restore original Health");
            let restored_capabilities = current_prepared
                .resume_capabilities_attempt(capabilities_material.clone())
                .expect("TEST_CODE restore original Capabilities");
            assert_eq!(restored_health.request_id(), health_id);
            assert_eq!(restored_health.request_bytes(), health_bytes);
            assert_material(
                &restored_health.request_material(),
                ExternalControlKind::Health,
                &endpoint_uri,
                &health_id,
                &health_bytes,
            );
            assert_eq!(restored_capabilities.request_id(), capabilities_id);
            assert_eq!(restored_capabilities.request_bytes(), capabilities_bytes);
            assert_material(
                &restored_capabilities.request_material(),
                ExternalControlKind::Capabilities,
                &endpoint_uri,
                &capabilities_id,
                &capabilities_bytes,
            );
            tokio::task::yield_now().await;
            tokio::task::yield_now().await;
            let after_restore = server.snapshot();
            assert_eq!(after_restore.tcp_accepts, 0);
            assert!(after_restore.health_requests.is_empty());
            assert!(after_restore.capabilities_requests.is_empty());
            assert_eq!(after_restore.capabilities_calls, 0);
            assert_eq!(after_restore.data_calls, 0);

            let health_execution = restored_health.execute();
            tokio::pin!(health_execution);
            let health_receipt_deadline = Instant::now() + Duration::from_secs(5);
            loop {
                tokio::select! {
                    biased;
                    _completion = &mut health_execution => {
                        panic!("TEST_CODE restored Health completed before release");
                    }
                    _ = tokio::task::yield_now() => {}
                }
                if !server.snapshot().health_requests.is_empty() {
                    break;
                }
                assert!(
                    Instant::now() < health_receipt_deadline,
                    "TEST_CODE restored Health receipt watchdog"
                );
            }
            let health_received = server.snapshot();
            assert_eq!(health_received.tcp_accepts, 1);
            assert_eq!(health_received.health_requests, vec![health_bytes.clone()]);
            assert_eq!(health_received.health_authorized, vec![true]);
            assert!(health_received.capabilities_requests.is_empty());
            assert_eq!(health_received.capabilities_calls, 0);
            assert_eq!(health_received.data_calls, 0);
            server.release_health();
            let health_completion =
                tokio::time::timeout(Duration::from_secs(5), &mut health_execution)
                    .await
                    .expect("TEST_CODE restored Health completion deadline");
            health_completion
                .processed()
                .expect("TEST_CODE restored Health processed response");
            let expected_health = HealthResponse {
                request_id: health_id.clone(),
                live: true,
                ready: true,
                state: "TEST_CODE_HEALTH_RUNNING".to_owned(),
                observability: Some(test_external_observability()),
                build_identity: Some(test_external_build_identity()),
            };
            match health_completion.result_material() {
                ExternalControlResultMaterial::Response { bytes, response } => {
                    assert_eq!(bytes, expected_health.encode_to_vec());
                    assert_eq!(response, &expected_health);
                }
                _ => panic!("TEST_CODE expected restored Health response material"),
            }
            let connected = health_completion
                .into_connected_client()
                .expect("TEST_CODE restored Health connected client");

            let bound_capabilities = restored_capabilities
                .bind_connected(connected)
                .expect("TEST_CODE bind Capabilities to Health connection");
            assert_eq!(bound_capabilities.request_id(), capabilities_id);
            assert_eq!(bound_capabilities.request_bytes(), capabilities_bytes);
            assert_material(
                &bound_capabilities.request_material(),
                ExternalControlKind::Capabilities,
                &endpoint_uri,
                &capabilities_id,
                &capabilities_bytes,
            );
            let after_bind = server.snapshot();
            assert_eq!(after_bind.tcp_accepts, 1);
            assert_eq!(after_bind.health_requests.len(), 1);
            assert!(after_bind.capabilities_requests.is_empty());
            assert_eq!(after_bind.capabilities_calls, 0);
            assert_eq!(after_bind.data_calls, 0);

            let capabilities_execution = bound_capabilities.execute();
            tokio::pin!(capabilities_execution);
            let capabilities_receipt_deadline = Instant::now() + Duration::from_secs(5);
            loop {
                tokio::select! {
                    biased;
                    _completion = &mut capabilities_execution => {
                        panic!("TEST_CODE Capabilities completed before release");
                    }
                    _ = tokio::task::yield_now() => {}
                }
                if !server.snapshot().capabilities_requests.is_empty() {
                    break;
                }
                assert!(
                    Instant::now() < capabilities_receipt_deadline,
                    "TEST_CODE Capabilities receipt watchdog"
                );
            }
            let capabilities_received = server.snapshot();
            assert_eq!(capabilities_received.tcp_accepts, 1);
            assert_eq!(capabilities_received.health_requests.len(), 1);
            assert_eq!(
                capabilities_received.capabilities_requests,
                vec![capabilities_bytes.clone()]
            );
            assert_eq!(capabilities_received.capabilities_authorized, vec![true]);
            assert_eq!(capabilities_received.capabilities_calls, 1);
            assert_eq!(capabilities_received.data_calls, 0);
            server.release_capabilities();
            let capabilities_completion =
                tokio::time::timeout(Duration::from_secs(5), &mut capabilities_execution)
                    .await
                    .expect("TEST_CODE Capabilities completion deadline");
            capabilities_completion
                .processed()
                .expect("TEST_CODE Capabilities processed response");

            let expected_capabilities = CapabilitiesResponse {
                request_id: capabilities_id.clone(),
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
            };
            match capabilities_completion.result_material() {
                ExternalControlResultMaterial::Response { bytes, response } => {
                    assert_eq!(bytes, expected_capabilities.encode_to_vec());
                    assert_eq!(response.request_id, capabilities_id);
                    assert_eq!(response.capabilities.len(), 2);
                    for (actual, expected) in response
                        .capabilities
                        .iter()
                        .zip(expected_capabilities.capabilities.iter())
                    {
                        assert_eq!(actual.operation, expected.operation);
                        assert_eq!(actual.repository_admission, expected.repository_admission);
                        assert_eq!(actual.runtime_available, expected.runtime_available);
                        assert_eq!(actual.provider, expected.provider);
                        assert_eq!(actual.exact_scope, expected.exact_scope);
                        assert_eq!(actual.blocker, expected.blocker);
                        assert_eq!(actual.diagnostic_available, expected.diagnostic_available);
                    }
                }
                _ => panic!("TEST_CODE expected native Capabilities response material"),
            }
            let completed = server.snapshot();
            assert_eq!(completed.tcp_accepts, 1);
            assert_eq!(completed.health_requests, vec![health_bytes]);
            assert_eq!(completed.health_authorized, vec![true]);
            assert_eq!(completed.capabilities_requests, vec![capabilities_bytes]);
            assert_eq!(completed.capabilities_authorized, vec![true]);
            assert_eq!(
                completed.capabilities_responses,
                vec![expected_capabilities.encode_to_vec()]
            );
            assert_eq!(completed.capabilities_calls, 1);
            assert_eq!(completed.data_calls, 0);
            let connected = capabilities_completion
                .into_connected_client()
                .expect("TEST_CODE Capabilities connected client");
            drop(connected);
            drop(current_prepared);
        }))
        .catch_unwind()
        .await;
    let cleanup = match server.take() {
        Some(server) => server.finish().await,
        None => Ok(()),
    };
    if let Err(error) = cleanup {
        panic!("TEST_CODE External control restore cleanup failed: {error}");
    }
    match outcome {
        Ok(result) => result.expect("TEST_CODE External control restore body timeout"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

fn prepared_external_control_with_bearer(
    endpoint_uri: &str,
    bearer: &str,
) -> PreparedExternalEndpoint {
    let endpoint = Endpoint::from_shared(endpoint_uri.to_owned())
        .expect("TEST_CODE External control batch endpoint")
        .timeout(Duration::from_secs(35));
    PreparedExternalEndpoint::from_plaintext_for_test(
        endpoint,
        endpoint_uri.to_owned(),
        Zeroizing::new(bearer.to_owned()),
        TEST_AUTHORITY.to_owned(),
    )
}

fn rehydrate_external_control_material(
    material: &super::external_control_attempt::ExternalControlRequestMaterial,
) -> super::external_control_attempt::ExternalControlRequestMaterial {
    super::external_control_attempt::ExternalControlRequestMaterial {
        kind: material.kind(),
        request_bytes: material.request_bytes().to_vec(),
        request_id: material.request_id().to_owned(),
        profile: material.profile(),
        endpoint_uri: material.endpoint_uri().to_owned(),
        acquisition_authority: material.acquisition_authority().to_owned(),
    }
}

fn expected_external_capabilities(
    request_id: &str,
) -> crate::grpc_client::external_pb::magic::market::v1::CapabilitiesResponse {
    use crate::grpc_client::external_pb::magic::market::v1::{
        AdmissionState, CapabilitiesResponse, Capability, Operation,
    };

    CapabilitiesResponse {
        request_id: request_id.to_owned(),
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
    }
}

fn assert_external_capabilities(
    actual: &crate::grpc_client::external_pb::magic::market::v1::CapabilitiesResponse,
    expected: &crate::grpc_client::external_pb::magic::market::v1::CapabilitiesResponse,
) {
    assert_eq!(actual.request_id, expected.request_id);
    assert_eq!(actual.capabilities.len(), 2);
    for (actual, expected) in actual.capabilities.iter().zip(expected.capabilities.iter()) {
        assert_eq!(actual.operation, expected.operation);
        assert_eq!(actual.repository_admission, expected.repository_admission);
        assert_eq!(actual.runtime_available, expected.runtime_available);
        assert_eq!(actual.provider, expected.provider);
        assert_eq!(actual.exact_scope, expected.exact_scope);
        assert_eq!(actual.blocker, expected.blocker);
        assert_eq!(actual.diagnostic_available, expected.diagnostic_available);
    }
}

fn assert_no_external_control_effects(server: &ExternalControlLoopbackServer) {
    let observed = server.snapshot();
    assert_eq!(observed.tcp_accepts, 0);
    assert!(observed.health_requests.is_empty());
    assert!(observed.health_authorized.is_empty());
    assert!(observed.health_responses.is_empty());
    assert!(observed.health_statuses.is_empty());
    assert_eq!(observed.capabilities_calls, 0);
    assert!(observed.capabilities_authorized.is_empty());
    assert!(observed.capabilities_requests.is_empty());
    assert!(observed.capabilities_responses.is_empty());
    assert!(observed.capabilities_statuses.is_empty());
    assert_eq!(observed.data_calls, 0);
    assert!(observed.data_authorized.is_empty());
}

async fn assert_external_restore_mismatch<T>(
    case: &str,
    result: Result<T, crate::grpc_client::errors::GrpcError>,
    servers: &[&ExternalControlLoopbackServer],
) {
    use crate::grpc_client::errors::GrpcError;

    tokio::task::yield_now().await;
    for server in servers {
        assert_no_external_control_effects(server);
    }
    let error = match result {
        Ok(_) => panic!("TEST_CODE malformed External control material restored: {case}"),
        Err(error) => error,
    };
    assert!(
        matches!(&error, GrpcError::FailedPrecondition { .. }),
        "TEST_CODE restore error type: {case}"
    );
    assert_eq!(
        error.details().code,
        "external_control_request_mismatch",
        "TEST_CODE restore error code: {case}"
    );
    assert_eq!(
        error.details().reason_code.as_deref(),
        Some("external_control_request_mismatch"),
        "TEST_CODE restore reason: {case}"
    );
    assert_eq!(
        error.details().retryable,
        Some(false),
        "TEST_CODE restore retry: {case}"
    );
    assert_eq!(
        error.details().request_id,
        None,
        "TEST_CODE restore request ID: {case}"
    );
}

async fn assert_external_restore_unauthenticated<T>(
    case: &str,
    result: Result<T, crate::grpc_client::errors::GrpcError>,
    servers: &[&ExternalControlLoopbackServer],
) {
    use crate::grpc_client::errors::GrpcError;

    tokio::task::yield_now().await;
    for server in servers {
        assert_no_external_control_effects(server);
    }
    let error = match result {
        Ok(_) => panic!("TEST_CODE invalid current External auth accepted: {case}"),
        Err(error) => error,
    };
    assert!(
        matches!(&error, GrpcError::Unauthenticated { .. }),
        "TEST_CODE current auth error type: {case}"
    );
    assert_eq!(
        error.details().code,
        "unauthenticated",
        "TEST_CODE current auth error code: {case}"
    );
}

fn overlong_external_context_length(bytes: &[u8]) -> Vec<u8> {
    assert_eq!(bytes.first().copied(), Some(0x0a));
    let length = *bytes
        .get(1)
        .expect("TEST_CODE External canonical context length");
    assert_eq!(length & 0x80, 0);
    let mut overlong = Vec::with_capacity(bytes.len() + 1);
    overlong.push(0x0a);
    overlong.push(length | 0x80);
    overlong.push(0x00);
    overlong.extend_from_slice(&bytes[2..]);
    overlong
}

async fn execute_loopback_capabilities(
    server: &ExternalControlLoopbackServer,
    capabilities: super::external_control_attempt::AuthorizedCapabilitiesAttempt,
) -> (
    String,
    Vec<u8>,
    super::external_control_attempt::ExternalControlCompletion<
        crate::grpc_client::external_pb::magic::market::v1::CapabilitiesResponse,
    >,
) {
    let request_id = capabilities.request_id().to_owned();
    let request_bytes = capabilities.request_bytes();
    let execution = capabilities.execute();
    tokio::pin!(execution);
    let receipt_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        tokio::select! {
            biased;
            _completion = &mut execution => {
                panic!("TEST_CODE External Capabilities completed before release");
            }
            _ = tokio::task::yield_now() => {}
        }
        if !server.snapshot().capabilities_requests.is_empty() {
            break;
        }
        assert!(
            Instant::now() < receipt_deadline,
            "TEST_CODE External Capabilities receipt watchdog"
        );
    }
    let received = server.snapshot();
    assert_eq!(received.tcp_accepts, 1);
    assert!(received.health_requests.is_empty());
    assert!(received.health_authorized.is_empty());
    assert_eq!(received.capabilities_requests, vec![request_bytes.clone()]);
    assert_eq!(received.capabilities_authorized, vec![true]);
    assert_eq!(received.capabilities_calls, 1);
    assert_eq!(received.data_calls, 0);
    assert!(received.data_authorized.is_empty());
    server.release_capabilities();
    let completion = tokio::time::timeout(Duration::from_secs(5), &mut execution)
        .await
        .expect("TEST_CODE External Capabilities completion deadline");
    (request_id, request_bytes, completion)
}

#[tokio::test]
async fn external_capabilities_restored_pending_cold_execute_sends_capabilities_without_health() {
    use super::external_control_attempt::{ExternalControlKind, ExternalControlResultMaterial};

    const OLD_BEARER: &str = "TEST_CODE_EXTERNAL_CONTROL_OLD_TOKEN";
    let mut server = None;
    let outcome =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(20), async {
            server = Some(
                ExternalControlLoopbackServer::bind_with_capabilities_success_for_test().await,
            );
            let server = server
                .as_ref()
                .expect("TEST_CODE cold Capabilities server owner");
            let old_prepared = prepared_external_control_with_bearer(server.endpoint(), OLD_BEARER);
            let old_capabilities = old_prepared
                .prepare_capabilities_attempt()
                .expect("TEST_CODE cold old Capabilities preparation");
            let original = old_capabilities.request_material();
            let restored_fields = rehydrate_external_control_material(&original);
            assert_eq!(restored_fields.kind(), ExternalControlKind::Capabilities);
            assert_eq!(restored_fields.profile(), ContractProfile::ExternalV1);
            assert_eq!(restored_fields.endpoint_uri(), server.endpoint());
            assert_eq!(restored_fields.acquisition_authority(), TEST_AUTHORITY);
            assert!(!restored_fields.request_id().is_empty());
            let original_id = restored_fields.request_id().to_owned();
            let original_bytes = restored_fields.request_bytes().to_vec();
            let decoded =
                crate::grpc_client::external_pb::magic::market::v1::CapabilitiesRequest::decode(
                    original_bytes.as_slice(),
                )
                .expect("TEST_CODE cold Capabilities decode");
            assert_eq!(decoded.encode_to_vec(), original_bytes);
            let context = decoded
                .context
                .expect("TEST_CODE cold Capabilities context");
            assert_eq!(context.protocol_version, 1);
            assert_eq!(context.request_id, original_id);
            for forbidden in [
                TEST_BEARER.as_bytes(),
                OLD_BEARER.as_bytes(),
                b"authorization".as_slice(),
            ] {
                assert!(!original_bytes
                    .windows(forbidden.len())
                    .any(|window| window == forbidden));
            }
            drop(old_capabilities);
            drop(old_prepared);
            tokio::task::yield_now().await;
            assert_no_external_control_effects(server);

            let current = prepared_external_control_with_bearer(server.endpoint(), TEST_BEARER);
            let capabilities = current
                .resume_capabilities_attempt(restored_fields)
                .expect("TEST_CODE cold Capabilities resume");
            assert_eq!(capabilities.request_id(), original_id);
            assert_eq!(capabilities.request_bytes(), original_bytes);
            tokio::task::yield_now().await;
            assert_no_external_control_effects(server);

            let (request_id, request_bytes, completion) =
                execute_loopback_capabilities(server, capabilities).await;
            assert_eq!(request_id, original_id);
            assert_eq!(request_bytes, original_bytes);
            completion
                .processed()
                .expect("TEST_CODE cold Capabilities processed response");
            let expected = expected_external_capabilities(&original_id);
            match completion.result_material() {
                ExternalControlResultMaterial::Response { bytes, response } => {
                    assert_eq!(bytes, expected.encode_to_vec());
                    assert_external_capabilities(response, &expected);
                }
                _ => panic!("TEST_CODE expected cold Capabilities Response"),
            }
            let observed = server.snapshot();
            assert_eq!(observed.tcp_accepts, 1);
            assert!(observed.health_requests.is_empty());
            assert_eq!(observed.capabilities_requests, vec![original_bytes]);
            assert_eq!(
                observed.capabilities_responses,
                vec![expected.encode_to_vec()]
            );
            assert!(observed.capabilities_statuses.is_empty());
            assert_eq!(observed.data_calls, 0);
            let connected = completion
                .into_connected_client()
                .expect("TEST_CODE cold Capabilities connected client");
            drop(connected);
            drop(current);
        }))
        .catch_unwind()
        .await;
    let cleanup = match server.take() {
        Some(server) => server.finish().await,
        None => Ok(()),
    };
    if let Err(error) = cleanup {
        panic!("TEST_CODE cold Capabilities cleanup failed: {error}");
    }
    match outcome {
        Ok(result) => result.expect("TEST_CODE cold Capabilities body timeout"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test]
async fn external_control_restore_rejects_kind_route_identity_and_noncanonical_material_before_connect(
) {
    use super::external_control_attempt::{ExternalControlKind, ExternalControlRequestMaterial};
    use crate::grpc_client::external_pb::magic::market::v1::{
        CapabilitiesRequest, HealthRequest, RequestContext,
    };

    fn pair(
        health: &ExternalControlRequestMaterial,
        capabilities: &ExternalControlRequestMaterial,
    ) -> (
        ExternalControlRequestMaterial,
        ExternalControlRequestMaterial,
    ) {
        (
            rehydrate_external_control_material(health),
            rehydrate_external_control_material(capabilities),
        )
    }

    let mut server_a = None;
    let mut server_b = None;
    let outcome =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(20), async {
            server_a = Some(ExternalControlLoopbackServer::bind().await);
            server_b = Some(ExternalControlLoopbackServer::bind().await);
            let a = server_a
                .as_ref()
                .expect("TEST_CODE restore rejection server A");
            let b = server_b
                .as_ref()
                .expect("TEST_CODE restore rejection server B");
            let prepared = prepared_external_control_with_bearer(a.endpoint(), TEST_BEARER);
            let health = prepared
                .prepare_health_attempt()
                .expect("TEST_CODE restore rejection Health baseline");
            let capabilities = prepared
                .prepare_capabilities_attempt()
                .expect("TEST_CODE restore rejection Capabilities baseline");
            let health_material = health.request_material();
            let capabilities_material = capabilities.request_material();
            drop(health);
            drop(capabilities);
            assert_no_external_control_effects(a);
            assert_no_external_control_effects(b);
            let servers = [a, b];

            assert_external_restore_mismatch(
                "Health material at Capabilities entry",
                prepared.resume_capabilities_attempt(rehydrate_external_control_material(
                    &health_material,
                )),
                &servers,
            )
            .await;
            assert_external_restore_mismatch(
                "Capabilities material at Health entry",
                prepared.resume_health_attempt(rehydrate_external_control_material(
                    &capabilities_material,
                )),
                &servers,
            )
            .await;

            let mut malformed = Vec::new();

            let (mut health_bad, mut capabilities_bad) =
                pair(&health_material, &capabilities_material);
            health_bad.profile = ContractProfile::LocalBridgeV1;
            capabilities_bad.profile = ContractProfile::LocalBridgeV1;
            malformed.push(("profile", health_bad, capabilities_bad));

            let (mut health_bad, mut capabilities_bad) =
                pair(&health_material, &capabilities_material);
            health_bad.endpoint_uri = b.endpoint().to_owned();
            capabilities_bad.endpoint_uri = b.endpoint().to_owned();
            malformed.push(("endpoint", health_bad, capabilities_bad));

            let (mut health_bad, mut capabilities_bad) =
                pair(&health_material, &capabilities_material);
            health_bad.acquisition_authority = "grpc-mtls:TEST_CODE-other.external".to_owned();
            capabilities_bad.acquisition_authority =
                "grpc-mtls:TEST_CODE-other.external".to_owned();
            malformed.push(("authority", health_bad, capabilities_bad));

            let (mut health_bad, mut capabilities_bad) =
                pair(&health_material, &capabilities_material);
            health_bad.request_id.clear();
            capabilities_bad.request_id.clear();
            malformed.push(("empty material ID", health_bad, capabilities_bad));

            let (mut health_bad, mut capabilities_bad) =
                pair(&health_material, &capabilities_material);
            health_bad.request_id = "TEST_CODE_OTHER_ID".to_owned();
            capabilities_bad.request_id = "TEST_CODE_OTHER_ID".to_owned();
            malformed.push(("mismatched material ID", health_bad, capabilities_bad));

            let (mut health_bad, mut capabilities_bad) =
                pair(&health_material, &capabilities_material);
            health_bad.request_bytes = HealthRequest { context: None }.encode_to_vec();
            capabilities_bad.request_bytes = CapabilitiesRequest { context: None }.encode_to_vec();
            malformed.push(("missing context", health_bad, capabilities_bad));

            let (mut health_bad, mut capabilities_bad) =
                pair(&health_material, &capabilities_material);
            health_bad.request_bytes = HealthRequest {
                context: Some(RequestContext {
                    protocol_version: 2,
                    request_id: health_bad.request_id.clone(),
                }),
            }
            .encode_to_vec();
            capabilities_bad.request_bytes = CapabilitiesRequest {
                context: Some(RequestContext {
                    protocol_version: 2,
                    request_id: capabilities_bad.request_id.clone(),
                }),
            }
            .encode_to_vec();
            malformed.push(("protocol", health_bad, capabilities_bad));

            let (mut health_bad, mut capabilities_bad) =
                pair(&health_material, &capabilities_material);
            health_bad.request_bytes = HealthRequest {
                context: Some(RequestContext {
                    protocol_version: 1,
                    request_id: "TEST_CODE_OTHER_ID".to_owned(),
                }),
            }
            .encode_to_vec();
            capabilities_bad.request_bytes = CapabilitiesRequest {
                context: Some(RequestContext {
                    protocol_version: 1,
                    request_id: "TEST_CODE_OTHER_ID".to_owned(),
                }),
            }
            .encode_to_vec();
            malformed.push(("context ID", health_bad, capabilities_bad));

            let (mut health_bad, mut capabilities_bad) =
                pair(&health_material, &capabilities_material);
            health_bad
                .request_bytes
                .extend_from_slice(&[0xf8, 0x07, 0x01]);
            capabilities_bad
                .request_bytes
                .extend_from_slice(&[0xf8, 0x07, 0x01]);
            malformed.push(("unknown field", health_bad, capabilities_bad));

            let (mut health_bad, mut capabilities_bad) =
                pair(&health_material, &capabilities_material);
            health_bad
                .request_bytes
                .extend_from_slice(health_material.request_bytes());
            capabilities_bad
                .request_bytes
                .extend_from_slice(capabilities_material.request_bytes());
            malformed.push(("duplicate context", health_bad, capabilities_bad));

            let (mut health_bad, mut capabilities_bad) =
                pair(&health_material, &capabilities_material);
            health_bad.request_bytes =
                overlong_external_context_length(health_material.request_bytes());
            capabilities_bad.request_bytes =
                overlong_external_context_length(capabilities_material.request_bytes());
            let decoded_health = HealthRequest::decode(health_bad.request_bytes.as_slice())
                .expect("TEST_CODE overlong Health decode");
            let decoded_capabilities =
                CapabilitiesRequest::decode(capabilities_bad.request_bytes.as_slice())
                    .expect("TEST_CODE overlong Capabilities decode");
            assert_eq!(
                decoded_health.encode_to_vec(),
                health_material.request_bytes()
            );
            assert_ne!(decoded_health.encode_to_vec(), health_bad.request_bytes);
            assert_eq!(
                decoded_capabilities.encode_to_vec(),
                capabilities_material.request_bytes()
            );
            assert_ne!(
                decoded_capabilities.encode_to_vec(),
                capabilities_bad.request_bytes
            );
            malformed.push(("noncanonical length", health_bad, capabilities_bad));

            for (case, health_bad, capabilities_bad) in malformed {
                assert_eq!(health_bad.kind, ExternalControlKind::Health);
                assert_eq!(capabilities_bad.kind, ExternalControlKind::Capabilities);
                assert_external_restore_mismatch(
                    case,
                    prepared.resume_health_attempt(health_bad),
                    &servers,
                )
                .await;
                assert_external_restore_mismatch(
                    case,
                    prepared.resume_capabilities_attempt(capabilities_bad),
                    &servers,
                )
                .await;
            }

            let invalid =
                prepared_external_control_with_bearer(a.endpoint(), "TEST_CODE_INVALID\nTOKEN");
            assert_external_restore_unauthenticated(
                "Health current auth",
                invalid
                    .resume_health_attempt(rehydrate_external_control_material(&health_material)),
                &servers,
            )
            .await;
            assert_external_restore_unauthenticated(
                "Capabilities current auth",
                invalid.resume_capabilities_attempt(rehydrate_external_control_material(
                    &capabilities_material,
                )),
                &servers,
            )
            .await;
            drop(invalid);
            drop(prepared);
        }))
        .catch_unwind()
        .await;
    let cleanup_b = match server_b.take() {
        Some(server) => server.finish().await,
        None => Ok(()),
    };
    let cleanup_a = match server_a.take() {
        Some(server) => server.finish().await,
        None => Ok(()),
    };
    if let Err(error) = cleanup_b {
        panic!("TEST_CODE restore rejection server B cleanup failed: {error}");
    }
    if let Err(error) = cleanup_a {
        panic!("TEST_CODE restore rejection server A cleanup failed: {error}");
    }
    match outcome {
        Ok(result) => result.expect("TEST_CODE restore rejection body timeout"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test]
async fn external_capabilities_bind_rejects_same_authority_client_from_different_endpoint() {
    use crate::grpc_client::errors::GrpcError;

    let mut server_a = None;
    let mut server_b = None;
    let outcome =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(30), async {
            server_a = Some(ExternalControlLoopbackServer::bind().await);
            server_b = Some(ExternalControlLoopbackServer::bind().await);
            let a = server_a
                .as_ref()
                .expect("TEST_CODE Capabilities bind server A");
            let b = server_b
                .as_ref()
                .expect("TEST_CODE Capabilities bind server B");
            assert_ne!(a.endpoint(), b.endpoint());
            let prepared_a = prepared_external_control_with_bearer(a.endpoint(), TEST_BEARER);
            let capabilities = prepared_a
                .prepare_capabilities_attempt()
                .expect("TEST_CODE cross-endpoint Capabilities preparation");
            let capabilities_id = capabilities.request_id().to_owned();
            let capabilities_bytes = capabilities.request_bytes();
            assert_no_external_control_effects(a);

            let (_health_id, _health_bytes, health_completion) = execute_loopback_health(b).await;
            health_completion
                .processed()
                .expect("TEST_CODE server B Health processed response");
            let client_b = health_completion
                .into_connected_client()
                .expect("TEST_CODE server B connected client");
            let error = match capabilities.bind_connected(client_b) {
                Ok(_) => panic!("TEST_CODE cross-endpoint client bound to Capabilities"),
                Err(error) => error,
            };
            assert!(matches!(&error, GrpcError::FailedPrecondition { .. }));
            assert_eq!(error.details().code, "external_control_request_mismatch");
            assert_eq!(error.details().retryable, Some(false));
            assert_no_external_control_effects(a);
            let observed_b = b.snapshot();
            assert_eq!(observed_b.tcp_accepts, 1);
            assert_eq!(observed_b.health_requests.len(), 1);
            assert_eq!(observed_b.health_authorized, vec![true]);
            assert_eq!(observed_b.capabilities_calls, 0);
            assert!(observed_b.capabilities_requests.is_empty());
            assert_eq!(observed_b.data_calls, 0);
            assert!(!capabilities_id.is_empty());
            let decoded =
                crate::grpc_client::external_pb::magic::market::v1::CapabilitiesRequest::decode(
                    capabilities_bytes.as_slice(),
                )
                .expect("TEST_CODE cross-endpoint Capabilities bytes");
            assert_eq!(
                decoded
                    .context
                    .expect("TEST_CODE cross-endpoint Capabilities context")
                    .request_id,
                capabilities_id
            );
            drop(prepared_a);
        }))
        .catch_unwind()
        .await;
    let cleanup_b = match server_b.take() {
        Some(server) => server.finish().await,
        None => Ok(()),
    };
    let cleanup_a = match server_a.take() {
        Some(server) => server.finish().await,
        None => Ok(()),
    };
    if let Err(error) = cleanup_b {
        panic!("TEST_CODE Capabilities bind server B cleanup failed: {error}");
    }
    if let Err(error) = cleanup_a {
        panic!("TEST_CODE Capabilities bind server A cleanup failed: {error}");
    }
    match outcome {
        Ok(result) => result.expect("TEST_CODE Capabilities bind body timeout"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test]
async fn external_capabilities_status_preserves_code_details_and_trailer_without_client() {
    use super::external_control_loopback_fixture::{
        CapabilitiesReply, HealthStatusCase, ObservedHealthTrailer,
    };
    use super::macro_attempt::MacroTrailerMaterial;
    use crate::grpc_client::errors::GrpcError;
    use crate::grpc_client::external_pb::magic::market::v1::ErrorDetail as WireErrorDetail;

    for case in [
        HealthStatusCase::Absent,
        HealthStatusCase::Bytes,
        HealthStatusCase::Malformed,
    ] {
        let mut server = None;
        let outcome =
            std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(20), async {
                server = Some(
                    ExternalControlLoopbackServer::bind_with_capabilities_reply_for_test(
                        CapabilitiesReply::Status(case),
                    )
                    .await,
                );
                let server = server
                    .as_ref()
                    .expect("TEST_CODE Capabilities Status server owner");
                let prepared =
                    prepared_external_control_with_bearer(server.endpoint(), TEST_BEARER);
                let capabilities = prepared
                    .prepare_capabilities_attempt()
                    .expect("TEST_CODE Capabilities Status preparation");
                let (request_id, request_bytes, completion) =
                    execute_loopback_capabilities(server, capabilities).await;
                let wire_detail = WireErrorDetail {
                    request_id: request_id.clone(),
                    provider: "Eastmoney".to_owned(),
                    reason_code: "unavailable".to_owned(),
                    retryable: false,
                    ..WireErrorDetail::default()
                }
                .encode_to_vec();
                let expected_details = if case == HealthStatusCase::Bytes {
                    Vec::new()
                } else {
                    wire_detail.clone()
                };
                let expected_trailer = match case {
                    HealthStatusCase::Absent => ObservedHealthTrailer::Absent,
                    HealthStatusCase::Bytes => ObservedHealthTrailer::Bytes(wire_detail),
                    HealthStatusCase::Malformed => ObservedHealthTrailer::Malformed,
                };
                let observed = server.snapshot();
                assert_eq!(observed.capabilities_requests, vec![request_bytes]);
                assert!(observed.capabilities_responses.is_empty());
                assert_eq!(observed.capabilities_statuses.len(), 1);
                let actual_status = &observed.capabilities_statuses[0];
                assert_eq!(actual_status.code, tonic::Code::Unavailable as i32);
                assert_eq!(actual_status.details, expected_details);
                assert_eq!(actual_status.trailer, expected_trailer);
                assert!(observed.health_requests.is_empty());
                assert_eq!(observed.data_calls, 0);

                let processed = completion
                    .processed()
                    .expect_err("TEST_CODE Capabilities Status processed error");
                match completion.result_material() {
                    ExternalControlResultMaterial::Status {
                        code,
                        details,
                        error_detail_trailer,
                        error,
                    } => {
                        assert_eq!(code, actual_status.code);
                        assert_eq!(details, actual_status.details.as_slice());
                        match (error_detail_trailer, &actual_status.trailer) {
                            (MacroTrailerMaterial::Absent, ObservedHealthTrailer::Absent)
                            | (MacroTrailerMaterial::Malformed, ObservedHealthTrailer::Malformed) =>
                                {}
                            (
                                MacroTrailerMaterial::Bytes(actual),
                                ObservedHealthTrailer::Bytes(expected),
                            ) => assert_eq!(actual, expected),
                            _ => panic!("TEST_CODE Capabilities Status trailer mismatch"),
                        }
                        assert!(matches!(error, GrpcError::Unavailable { .. }));
                        assert!(std::ptr::eq(error, processed));
                    }
                    _ => panic!("TEST_CODE expected native Capabilities Status"),
                }
                assert!(matches!(processed, GrpcError::Unavailable { .. }));
                assert_eq!(
                    processed.details().code,
                    tonic::Code::Unavailable.to_string()
                );
                if case == HealthStatusCase::Malformed {
                    assert_eq!(processed.details().provider, None);
                    assert_eq!(processed.details().reason_code, None);
                    assert_eq!(processed.details().retryable, None);
                    assert_eq!(processed.details().request_id, None);
                } else {
                    assert_eq!(processed.details().provider.as_deref(), Some("Eastmoney"));
                    assert_eq!(
                        processed.details().reason_code.as_deref(),
                        Some("unavailable")
                    );
                    assert_eq!(processed.details().retryable, Some(false));
                    let correlation = processed
                        .details()
                        .request_id
                        .as_deref()
                        .expect("TEST_CODE safe Capabilities request correlation");
                    assert!(correlation.starts_with("sha256:"));
                    assert_ne!(correlation, request_id);
                }
                assert!(completion.into_connected_client().is_none());
                drop(prepared);
            }))
            .catch_unwind()
            .await;
        let cleanup = match server.take() {
            Some(server) => server.finish().await,
            None => Ok(()),
        };
        if let Err(error) = cleanup {
            panic!("TEST_CODE Capabilities Status cleanup failed: {error}");
        }
        match outcome {
            Ok(result) => result.expect("TEST_CODE Capabilities Status body timeout"),
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }
}

#[tokio::test]
async fn external_capabilities_mismatched_request_id_preserves_response_without_client() {
    use super::external_control_loopback_fixture::CapabilitiesReply;
    use crate::grpc_client::errors::GrpcError;

    const WRONG_ID: &str = "TEST_CODE_WRONG_CAPABILITIES_REQUEST_ID";
    let mut server = None;
    let outcome =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(20), async {
            server = Some(
                ExternalControlLoopbackServer::bind_with_capabilities_reply_for_test(
                    CapabilitiesReply::MismatchedId,
                )
                .await,
            );
            let server = server
                .as_ref()
                .expect("TEST_CODE Capabilities mismatch server owner");
            let prepared = prepared_external_control_with_bearer(server.endpoint(), TEST_BEARER);
            let capabilities = prepared
                .prepare_capabilities_attempt()
                .expect("TEST_CODE Capabilities mismatch preparation");
            let (request_id, request_bytes, completion) =
                execute_loopback_capabilities(server, capabilities).await;
            assert_ne!(request_id, WRONG_ID);
            let expected = expected_external_capabilities(WRONG_ID);
            let observed = server.snapshot();
            assert_eq!(observed.capabilities_requests, vec![request_bytes]);
            assert_eq!(
                observed.capabilities_responses,
                vec![expected.encode_to_vec()]
            );
            assert!(observed.capabilities_statuses.is_empty());
            assert!(observed.health_requests.is_empty());
            assert_eq!(observed.data_calls, 0);
            match completion.result_material() {
                ExternalControlResultMaterial::Response { bytes, response } => {
                    assert_eq!(bytes, expected.encode_to_vec());
                    assert_external_capabilities(response, &expected);
                }
                _ => panic!("TEST_CODE expected mismatched Capabilities Response"),
            }
            let processed = completion
                .processed()
                .expect_err("TEST_CODE mismatched Capabilities must be rejected");
            assert!(matches!(processed, GrpcError::FailedPrecondition { .. }));
            assert_eq!(processed.details().code, "capabilities_request_id_mismatch");
            assert_eq!(
                processed.details().reason_code.as_deref(),
                Some("capabilities_request_id_mismatch")
            );
            assert_eq!(processed.details().retryable, Some(false));
            assert_eq!(processed.details().request_id, None);
            let diagnostic = format!("{processed:?}");
            assert!(!diagnostic.contains(WRONG_ID));
            assert!(completion.into_connected_client().is_none());
            drop(prepared);
        }))
        .catch_unwind()
        .await;
    let cleanup = match server.take() {
        Some(server) => server.finish().await,
        None => Ok(()),
    };
    if let Err(error) = cleanup {
        panic!("TEST_CODE Capabilities mismatch cleanup failed: {error}");
    }
    match outcome {
        Ok(result) => result.expect("TEST_CODE Capabilities mismatch body timeout"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test]
async fn external_capabilities_connect_failure_is_local_unavailable_without_status_or_client() {
    use crate::grpc_client::errors::{ErrorDetail as ClientErrorDetail, GrpcError};
    use tokio::net::TcpSocket;

    let mut socket = None;
    let outcome =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(20), async {
            socket = Some(TcpSocket::new_v4().expect("TEST_CODE Capabilities closed socket"));
            socket
                .as_ref()
                .expect("TEST_CODE Capabilities closed socket owner")
                .bind(
                    "127.0.0.1:0"
                        .parse()
                        .expect("TEST_CODE Capabilities closed socket address"),
                )
                .expect("TEST_CODE Capabilities closed socket bind");
            let address = socket
                .as_ref()
                .expect("TEST_CODE Capabilities held closed socket")
                .local_addr()
                .expect("TEST_CODE Capabilities closed socket local address");
            let endpoint_uri = format!("http://{address}");
            let endpoint = Endpoint::from_shared(endpoint_uri.clone())
                .expect("TEST_CODE Capabilities closed socket endpoint")
                .timeout(Duration::from_secs(35))
                .connect_timeout(Duration::from_secs(1));
            let prepared = PreparedExternalEndpoint::from_plaintext_for_test(
                endpoint,
                endpoint_uri,
                Zeroizing::new(TEST_BEARER.to_owned()),
                TEST_AUTHORITY.to_owned(),
            );
            let capabilities = prepared
                .prepare_capabilities_attempt()
                .expect("TEST_CODE closed socket Capabilities");
            let completion = tokio::time::timeout(Duration::from_secs(5), capabilities.execute())
                .await
                .expect("TEST_CODE Capabilities connect failure deadline");
            let processed = completion
                .processed()
                .expect_err("TEST_CODE Capabilities connect must fail");
            match completion.result_material() {
                ExternalControlResultMaterial::ConnectUnavailable { error } => {
                    assert!(matches!(error, GrpcError::Unavailable { .. }));
                    assert_eq!(error.details(), &ClientErrorDetail::default());
                    assert!(std::ptr::eq(error, processed));
                }
                _ => panic!("TEST_CODE expected Capabilities ConnectUnavailable"),
            }
            assert!(matches!(processed, GrpcError::Unavailable { .. }));
            assert_eq!(processed.details(), &ClientErrorDetail::default());
            assert!(completion.into_connected_client().is_none());
            drop(prepared);
        }))
        .catch_unwind()
        .await;
    drop(socket.take());
    match outcome {
        Ok(result) => result.expect("TEST_CODE Capabilities connect failure body timeout"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test]
async fn external_macro_restored_pending_cold_execute_sends_only_original_data() {
    use super::macro_attempt::{
        ExternalMacroAttemptCompletion, MacroContinuation, MacroTrailerMaterial,
        RestoredExternalMacroRequest, RestoredMacroRequest,
    };
    use crate::grpc_client::pb::magic::market::v1::{
        AdmissionState, CanonicalPayload, Operation, QueryResponse,
    };
    use crate::grpc_client::retry::RetryDecision;

    const OLD_BEARER: &str = "TEST_CODE_EXTERNAL_CONTROL_OLD_TOKEN";
    const RESPONSE_BATCH: &str = "TEST_CODE_EXTERNAL_DATA_BATCH";
    const RESPONSE_OBSERVED_AT: &str = "2026-09-14T15:31:00+08:00";
    const RESPONSE_SOURCE_AT: &str = "2026-09-14 15:30";
    const RESPONSE_DATA: &[u8] = br#"{"item_id":"TEST_CODE_EXTERNAL_NEWS_001","title":"TEST_CODE external data title","summary":"TEST_CODE external data summary","content":"TEST_CODE external data content","publisher":"TEST_CODE Eastmoney publisher","url":"https://example.com/TEST_CODE_EXTERNAL_NEWS_001","published_at":"2026-09-14T15:30:00+08:00","instruments":[{"exchange":"Shanghai","code":"TEST_CODE_600001","asset_class":"Equity"}],"topics":["TEST_CODE_external_topic"],"language":"zh-CN","evidence":{"provider":"Eastmoney","source_at":"2026-09-14 15:30","observed_at":"2026-09-14T15:31:00+08:00","batch_id":"TEST_CODE_EXTERNAL_DATA_BATCH"}}"#;

    let mut server = None;
    let outcome =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(20), async {
            server = Some(ExternalControlLoopbackServer::bind_with_data_success_for_test().await);
            let server = server
                .as_ref()
                .expect("TEST_CODE External cold data server owner");
            let endpoint_uri = server.endpoint().to_owned();
            let old_endpoint = Endpoint::from_shared(endpoint_uri.clone())
                .expect("TEST_CODE old External data endpoint")
                .timeout(Duration::from_secs(35));
            let old_prepared = PreparedExternalEndpoint::from_plaintext_for_test(
                old_endpoint,
                endpoint_uri.clone(),
                Zeroizing::new(OLD_BEARER.to_owned()),
                TEST_AUTHORITY.to_owned(),
            );
            let old = old_prepared
                .prepare_macro_query(MacroQueryIdentity::GlobalNews {
                    provider: GlobalNewsProvider::Eastmoney,
                    limit: 20,
                })
                .expect("TEST_CODE original deferred External data request");
            assert_eq!(old.endpoint_uri(), endpoint_uri);
            assert_eq!(old.profile(), ContractProfile::ExternalV1);
            assert_eq!(old.acquisition_authority(), TEST_AUTHORITY);
            assert_eq!(old.retry_policy(), (4, 1000, 60_000, 200));
            assert_eq!(old.attempt_ordinal(), 1);
            let request_id = old.request_id().to_owned();
            let request_bytes = old.request_bytes();
            let decoded = QueryRequest::decode(request_bytes.as_slice())
                .expect("TEST_CODE decode original deferred External data request");
            assert_eq!(decoded.encode_to_vec(), request_bytes);
            let context = decoded
                .context
                .as_ref()
                .expect("TEST_CODE deferred External data context");
            assert_eq!(context.protocol_version, 1);
            assert_eq!(context.request_id, request_id);
            assert!(!request_id.is_empty());
            assert_eq!(decoded.preferred_provider, "Eastmoney");
            assert!(!decoded.allow_unadmitted);
            let payload = decoded
                .payload
                .as_ref()
                .expect("TEST_CODE deferred External data payload");
            assert_eq!(payload.schema, "magic.market.global_news.request");
            assert_eq!(payload.schema_version, 2);
            assert_eq!(payload.content_type, "application/json; charset=utf-8");
            assert_eq!(payload.data, br#"{"limit":20}"#);
            for forbidden in [OLD_BEARER.as_bytes(), TEST_BEARER.as_bytes()] {
                assert!(!request_bytes
                    .windows(forbidden.len())
                    .any(|window| window == forbidden));
            }
            assert!(!request_bytes
                .windows(b"authorization".len())
                .any(|window| window == b"authorization"));

            let restored = RestoredExternalMacroRequest {
                endpoint_uri: old.endpoint_uri().to_owned(),
                request: RestoredMacroRequest {
                    request_bytes: request_bytes.clone(),
                    request_id: request_id.clone(),
                    profile: old.profile(),
                    acquisition_authority: Some(old.acquisition_authority().to_owned()),
                    retry_policy: old.retry_policy(),
                    next_attempt: old.attempt_ordinal(),
                },
            };
            drop(old);
            drop(old_prepared);
            tokio::task::yield_now().await;
            tokio::task::yield_now().await;
            let after_old_drop = server.snapshot();
            assert_eq!(after_old_drop.tcp_accepts, 0);
            assert!(after_old_drop.health_requests.is_empty());
            assert_eq!(after_old_drop.capabilities_calls, 0);
            assert!(after_old_drop.capabilities_requests.is_empty());
            assert_eq!(after_old_drop.data_calls, 0);
            assert!(after_old_drop.data_requests.is_empty());

            let current_endpoint = Endpoint::from_shared(endpoint_uri.clone())
                .expect("TEST_CODE current External data endpoint")
                .timeout(Duration::from_secs(35));
            let current = PreparedExternalEndpoint::from_plaintext_for_test(
                current_endpoint,
                endpoint_uri.clone(),
                Zeroizing::new(TEST_BEARER.to_owned()),
                TEST_AUTHORITY.to_owned(),
            );
            let resumed = current
                .resume_macro_query(
                    MacroQueryIdentity::GlobalNews {
                        provider: GlobalNewsProvider::Eastmoney,
                        limit: 20,
                    },
                    restored,
                )
                .expect("TEST_CODE restore deferred External data request");
            assert_eq!(resumed.endpoint_uri(), endpoint_uri);
            assert_eq!(resumed.profile(), ContractProfile::ExternalV1);
            assert_eq!(resumed.acquisition_authority(), TEST_AUTHORITY);
            assert_eq!(resumed.retry_policy(), (4, 1000, 60_000, 200));
            assert_eq!(resumed.attempt_ordinal(), 1);
            assert_eq!(resumed.request_id(), request_id);
            assert_eq!(resumed.request_bytes(), request_bytes);
            tokio::task::yield_now().await;
            tokio::task::yield_now().await;
            let before_execute = server.snapshot();
            assert_eq!(before_execute.tcp_accepts, 0);
            assert!(before_execute.health_requests.is_empty());
            assert_eq!(before_execute.capabilities_calls, 0);
            assert!(before_execute.capabilities_requests.is_empty());
            assert_eq!(before_execute.data_calls, 0);
            assert!(before_execute.data_requests.is_empty());

            let execution = resumed.execute();
            tokio::pin!(execution);
            let receipt_deadline = Instant::now() + Duration::from_secs(5);
            loop {
                tokio::select! {
                    biased;
                    _completion = &mut execution => {
                        panic!("TEST_CODE External cold data completed before fixture release");
                    }
                    _ = tokio::task::yield_now() => {}
                }
                if server.snapshot().data_calls > 0 {
                    break;
                }
                assert!(
                    Instant::now() < receipt_deadline,
                    "TEST_CODE External cold data receipt watchdog"
                );
            }
            let received = server.snapshot();
            assert_eq!(received.tcp_accepts, 1);
            assert!(received.health_requests.is_empty());
            assert!(received.health_responses.is_empty());
            assert_eq!(received.capabilities_calls, 0);
            assert!(received.capabilities_requests.is_empty());
            assert!(received.capabilities_responses.is_empty());
            assert_eq!(received.data_calls, 1);
            assert_eq!(received.data_methods, vec!["global_news"]);
            assert_eq!(received.data_requests, vec![request_bytes.clone()]);
            assert_eq!(received.data_authorized, vec![true]);
            assert!(received.data_responses.is_empty());

            server.release_data();
            let completion = tokio::time::timeout(Duration::from_secs(5), &mut execution)
                .await
                .expect("TEST_CODE External cold data completion deadline")
                .expect("TEST_CODE External cold data execution");
            let expected_response = QueryResponse {
                request_id: request_id.clone(),
                operation: Operation::GlobalNews as i32,
                admission: AdmissionState::Admitted as i32,
                selected_provider: "Eastmoney".to_owned(),
                batch_id: RESPONSE_BATCH.to_owned(),
                complete: true,
                observed_at: RESPONSE_OBSERVED_AT.to_owned(),
                source_at: RESPONSE_SOURCE_AT.to_owned(),
                records: vec![CanonicalPayload {
                    schema: "magic.market.news_item".to_owned(),
                    schema_version: 2,
                    content_type: "application/json; charset=utf-8".to_owned(),
                    data: RESPONSE_DATA.to_vec(),
                }],
                source: String::new(),
                diagnostic_blocker: String::new(),
            };
            let expected_response_bytes = expected_response.encode_to_vec();
            match &completion {
                ExternalMacroAttemptCompletion::Unary(inner) => {
                    assert_eq!(
                        inner.response_bytes.as_deref(),
                        Some(expected_response_bytes.as_slice())
                    );
                    assert_eq!(inner.status_code, None);
                    assert_eq!(inner.status_details, None);
                    assert_eq!(
                        inner.status_error_detail_trailer,
                        MacroTrailerMaterial::Absent
                    );
                    assert_eq!(inner.retry_decision, RetryDecision::NoRetry);
                    assert_eq!(inner.continuation, MacroContinuation::Terminal);
                    let processed = inner
                        .processed
                        .as_ref()
                        .expect("TEST_CODE External cold data native projection");
                    assert_eq!(processed.admission, AdmissionState::Admitted);
                    assert_eq!(processed.selected_provider, "Eastmoney");
                    assert_eq!(processed.batch_id, RESPONSE_BATCH);
                    assert!(processed.complete);
                    assert_eq!(processed.observed_at, RESPONSE_OBSERVED_AT);
                    assert_eq!(processed.source_at, RESPONSE_SOURCE_AT);
                    assert_eq!(processed.records, expected_response.records);
                    assert_eq!(processed.source(), TEST_AUTHORITY);
                    assert!(processed.diagnostic_blocker.is_empty());
                }
                ExternalMacroAttemptCompletion::ConnectUnavailable { .. } => {
                    panic!("TEST_CODE expected External cold data unary completion");
                }
            }
            let observed = server.snapshot();
            assert_eq!(observed.data_responses, vec![expected_response_bytes]);
            assert_eq!(observed.tcp_accepts, 1);
            assert!(observed.health_requests.is_empty());
            assert_eq!(observed.capabilities_calls, 0);
            assert!(observed.capabilities_requests.is_empty());
            assert_eq!(observed.data_calls, 1);
            assert_eq!(observed.data_methods, vec!["global_news"]);
            assert_eq!(observed.data_requests, vec![request_bytes]);
            assert_eq!(observed.data_authorized, vec![true]);
            drop(completion);
            drop(current);
        }))
        .catch_unwind()
        .await;
    let cleanup = match server.take() {
        Some(server) => server.finish().await,
        None => Ok(()),
    };
    if let Err(error) = cleanup {
        panic!("TEST_CODE External cold data cleanup failed: {error}");
    }
    match outcome {
        Ok(result) => result.expect("TEST_CODE External cold data body timeout"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

fn fresh_external_macro_material(
    endpoint_uri: &str,
    request_bytes: &[u8],
    request_id: &str,
    profile: ContractProfile,
    acquisition_authority: &str,
    retry_policy: (u32, u64, u64, u64),
    next_attempt: u32,
) -> super::macro_attempt::RestoredExternalMacroRequest {
    super::macro_attempt::RestoredExternalMacroRequest {
        endpoint_uri: endpoint_uri.to_owned(),
        request: super::macro_attempt::RestoredMacroRequest {
            request_bytes: request_bytes.to_vec(),
            request_id: request_id.to_owned(),
            profile,
            acquisition_authority: Some(acquisition_authority.to_owned()),
            retry_policy,
            next_attempt,
        },
    }
}

fn assert_no_external_macro_rpcs(
    server: &ExternalControlLoopbackServer,
    expected_tcp_accepts: usize,
) {
    let observed = server.snapshot();
    assert_eq!(observed.tcp_accepts, expected_tcp_accepts);
    assert!(observed.health_requests.is_empty());
    assert!(observed.health_authorized.is_empty());
    assert!(observed.health_responses.is_empty());
    assert!(observed.health_statuses.is_empty());
    assert_eq!(observed.capabilities_calls, 0);
    assert!(observed.capabilities_authorized.is_empty());
    assert!(observed.capabilities_requests.is_empty());
    assert!(observed.capabilities_responses.is_empty());
    assert!(observed.capabilities_statuses.is_empty());
    assert_eq!(observed.data_calls, 0);
    assert!(observed.data_authorized.is_empty());
    assert!(observed.data_methods.is_empty());
    assert!(observed.data_requests.is_empty());
    assert!(observed.data_responses.is_empty());
    assert!(observed.data_statuses.is_empty());
}

async fn assert_external_macro_restore_failed<T>(
    case: &str,
    result: Result<T, crate::grpc_client::errors::GrpcError>,
    servers: &[&ExternalControlLoopbackServer],
) {
    use crate::grpc_client::errors::{ErrorDetail as ClientErrorDetail, GrpcError};

    tokio::task::yield_now().await;
    for server in servers {
        assert_no_external_macro_rpcs(server, 0);
    }
    let error = match result {
        Ok(_) => panic!("TEST_CODE External Macro drift restored: {case}"),
        Err(error) => error,
    };
    assert!(
        matches!(&error, GrpcError::FailedPrecondition { .. }),
        "TEST_CODE External Macro drift type: {case}"
    );
    assert_eq!(
        error.details(),
        &ClientErrorDetail::default(),
        "TEST_CODE External Macro drift detail: {case}"
    );
}

#[tokio::test]
async fn external_macro_bind_rejects_same_authority_client_from_different_endpoint() {
    use crate::grpc_client::errors::{ErrorDetail as ClientErrorDetail, GrpcError};

    let mut server_a = None;
    let mut server_b = None;
    let outcome =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(30), async {
            server_a = Some(ExternalControlLoopbackServer::bind().await);
            server_b = Some(ExternalControlLoopbackServer::bind().await);
            let a = server_a
                .as_ref()
                .expect("TEST_CODE External Macro bind server A");
            let b = server_b
                .as_ref()
                .expect("TEST_CODE External Macro bind server B");
            assert_ne!(a.endpoint(), b.endpoint());
            let prepared_a = prepared_external_control_with_bearer(a.endpoint(), TEST_BEARER);
            let request = prepared_a
                .prepare_macro_query(MacroQueryIdentity::GlobalNews {
                    provider: GlobalNewsProvider::Eastmoney,
                    limit: 20,
                })
                .expect("TEST_CODE cross-endpoint Macro request");
            assert_eq!(request.endpoint_uri(), a.endpoint());
            assert_no_external_macro_rpcs(a, 0);
            assert_no_external_macro_rpcs(b, 0);

            let prepared_b = prepared_external_control_with_bearer(b.endpoint(), TEST_BEARER);
            let client_b = tokio::time::timeout(Duration::from_secs(5), prepared_b.connect_once())
                .await
                .expect("TEST_CODE server B connect deadline")
                .expect("TEST_CODE server B connect");
            let receipt_deadline = Instant::now() + Duration::from_secs(5);
            while b.snapshot().tcp_accepts == 0 {
                tokio::task::yield_now().await;
                assert!(
                    Instant::now() < receipt_deadline,
                    "TEST_CODE server B TCP receipt watchdog"
                );
            }
            let error = match request.bind_connected(client_b) {
                Ok(_) => panic!("TEST_CODE cross-endpoint client bound to Macro request"),
                Err(error) => error,
            };
            assert!(matches!(&error, GrpcError::FailedPrecondition { .. }));
            assert_eq!(error.details(), &ClientErrorDetail::default());
            assert_no_external_macro_rpcs(a, 0);
            assert_no_external_macro_rpcs(b, 1);
            drop(prepared_b);
            drop(prepared_a);
        }))
        .catch_unwind()
        .await;
    let cleanup_b = match server_b.take() {
        Some(server) => server.finish().await,
        None => Ok(()),
    };
    let cleanup_a = match server_a.take() {
        Some(server) => server.finish().await,
        None => Ok(()),
    };
    let mut cleanup_errors = Vec::new();
    if let Err(error) = cleanup_b {
        cleanup_errors.push(format!("server B: {error}"));
    }
    if let Err(error) = cleanup_a {
        cleanup_errors.push(format!("server A: {error}"));
    }
    if !cleanup_errors.is_empty() {
        panic!(
            "TEST_CODE External Macro bind cleanup failed: {}",
            cleanup_errors.join("; ")
        );
    }
    match outcome {
        Ok(result) => result.expect("TEST_CODE External Macro bind body timeout"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test]
async fn external_macro_restore_rejects_route_and_core_drift_before_connect() {
    use crate::grpc_client::errors::GrpcError;

    let mut server_a = None;
    let mut server_b = None;
    let outcome =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(30), async {
            server_a = Some(ExternalControlLoopbackServer::bind().await);
            server_b = Some(ExternalControlLoopbackServer::bind().await);
            let a = server_a
                .as_ref()
                .expect("TEST_CODE External Macro restore server A");
            let b = server_b
                .as_ref()
                .expect("TEST_CODE External Macro restore server B");
            assert_ne!(a.endpoint(), b.endpoint());
            let prepared = prepared_external_control_with_bearer(a.endpoint(), TEST_BEARER);
            let original = prepared
                .prepare_macro_query(MacroQueryIdentity::GlobalNews {
                    provider: GlobalNewsProvider::Eastmoney,
                    limit: 20,
                })
                .expect("TEST_CODE canonical External Macro request");
            let request_bytes = original.request_bytes();
            let request_id = original.request_id().to_owned();
            let profile = original.profile();
            let authority = original.acquisition_authority().to_owned();
            let retry_policy = original.retry_policy();
            assert_eq!(original.endpoint_uri(), a.endpoint());
            drop(original);
            let servers = [a, b];

            let mut route = fresh_external_macro_material(
                a.endpoint(),
                &request_bytes,
                &request_id,
                profile,
                &authority,
                retry_policy,
                1,
            );
            route.endpoint_uri = b.endpoint().to_owned();
            assert_external_macro_restore_failed(
                "endpoint",
                prepared.resume_macro_query(
                    MacroQueryIdentity::GlobalNews {
                        provider: GlobalNewsProvider::Eastmoney,
                        limit: 20,
                    },
                    route,
                ),
                &servers,
            )
            .await;

            let mut local_profile = fresh_external_macro_material(
                a.endpoint(),
                &request_bytes,
                &request_id,
                profile,
                &authority,
                retry_policy,
                1,
            );
            local_profile.request.profile = ContractProfile::LocalBridgeV1;
            assert_external_macro_restore_failed(
                "profile",
                prepared.resume_macro_query(
                    MacroQueryIdentity::GlobalNews {
                        provider: GlobalNewsProvider::Eastmoney,
                        limit: 20,
                    },
                    local_profile,
                ),
                &servers,
            )
            .await;

            let mut no_authority = fresh_external_macro_material(
                a.endpoint(),
                &request_bytes,
                &request_id,
                profile,
                &authority,
                retry_policy,
                1,
            );
            no_authority.request.acquisition_authority = None;
            assert_external_macro_restore_failed(
                "authority",
                prepared.resume_macro_query(
                    MacroQueryIdentity::GlobalNews {
                        provider: GlobalNewsProvider::Eastmoney,
                        limit: 20,
                    },
                    no_authority,
                ),
                &servers,
            )
            .await;

            let mut changed_id = fresh_external_macro_material(
                a.endpoint(),
                &request_bytes,
                &request_id,
                profile,
                &authority,
                retry_policy,
                1,
            );
            changed_id.request.request_id = "TEST_CODE_CHANGED_MACRO_REQUEST_ID".to_owned();
            assert_external_macro_restore_failed(
                "saved request ID",
                prepared.resume_macro_query(
                    MacroQueryIdentity::GlobalNews {
                        provider: GlobalNewsProvider::Eastmoney,
                        limit: 20,
                    },
                    changed_id,
                ),
                &servers,
            )
            .await;

            for ordinal in [0, 5] {
                let ordinal_material = fresh_external_macro_material(
                    a.endpoint(),
                    &request_bytes,
                    &request_id,
                    profile,
                    &authority,
                    retry_policy,
                    ordinal,
                );
                assert_external_macro_restore_failed(
                    &format!("ordinal {ordinal}"),
                    prepared.resume_macro_query(
                        MacroQueryIdentity::GlobalNews {
                            provider: GlobalNewsProvider::Eastmoney,
                            limit: 20,
                        },
                        ordinal_material,
                    ),
                    &servers,
                )
                .await;
            }

            assert_external_macro_restore_failed(
                "identity",
                prepared.resume_macro_query(
                    MacroQueryIdentity::GlobalNews {
                        provider: GlobalNewsProvider::Cailianpress,
                        limit: 20,
                    },
                    fresh_external_macro_material(
                        a.endpoint(),
                        &request_bytes,
                        &request_id,
                        profile,
                        &authority,
                        retry_policy,
                        1,
                    ),
                ),
                &servers,
            )
            .await;

            let mut unknown_field = fresh_external_macro_material(
                a.endpoint(),
                &request_bytes,
                &request_id,
                profile,
                &authority,
                retry_policy,
                1,
            );
            unknown_field
                .request
                .request_bytes
                .extend_from_slice(&[0xa0, 0x06, 0x01]);
            assert_external_macro_restore_failed(
                "unknown protobuf field",
                prepared.resume_macro_query(
                    MacroQueryIdentity::GlobalNews {
                        provider: GlobalNewsProvider::Eastmoney,
                        limit: 20,
                    },
                    unknown_field,
                ),
                &servers,
            )
            .await;

            let mut duplicate = fresh_external_macro_material(
                a.endpoint(),
                &request_bytes,
                &request_id,
                profile,
                &authority,
                retry_policy,
                1,
            );
            duplicate
                .request
                .request_bytes
                .extend_from_slice(&request_bytes);
            assert_external_macro_restore_failed(
                "duplicate protobuf message",
                prepared.resume_macro_query(
                    MacroQueryIdentity::GlobalNews {
                        provider: GlobalNewsProvider::Eastmoney,
                        limit: 20,
                    },
                    duplicate,
                ),
                &servers,
            )
            .await;

            let invalid = prepared_external_control_with_bearer(
                a.endpoint(),
                "TEST_CODE_INVALID\nMACRO_TOKEN",
            );
            let auth_error = match invalid.resume_macro_query(
                MacroQueryIdentity::GlobalNews {
                    provider: GlobalNewsProvider::Eastmoney,
                    limit: 20,
                },
                fresh_external_macro_material(
                    a.endpoint(),
                    &request_bytes,
                    &request_id,
                    profile,
                    &authority,
                    retry_policy,
                    1,
                ),
            ) {
                Ok(_) => panic!("TEST_CODE invalid current Macro auth accepted"),
                Err(error) => error,
            };
            tokio::task::yield_now().await;
            assert_no_external_macro_rpcs(a, 0);
            assert_no_external_macro_rpcs(b, 0);
            assert!(matches!(&auth_error, GrpcError::Unauthenticated { .. }));
            assert_eq!(auth_error.details().code, "unauthenticated");
            assert_eq!(auth_error.details().request_id, None);
            drop(invalid);
            drop(prepared);
        }))
        .catch_unwind()
        .await;
    let cleanup_b = match server_b.take() {
        Some(server) => server.finish().await,
        None => Ok(()),
    };
    let cleanup_a = match server_a.take() {
        Some(server) => server.finish().await,
        None => Ok(()),
    };
    let mut cleanup_errors = Vec::new();
    if let Err(error) = cleanup_b {
        cleanup_errors.push(format!("server B: {error}"));
    }
    if let Err(error) = cleanup_a {
        cleanup_errors.push(format!("server A: {error}"));
    }
    if !cleanup_errors.is_empty() {
        panic!(
            "TEST_CODE External Macro restore cleanup failed: {}",
            cleanup_errors.join("; ")
        );
    }
    match outcome {
        Ok(result) => result.expect("TEST_CODE External Macro restore body timeout"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test]
async fn external_macro_connect_unavailable_uses_original_policy_and_ordinal() {
    use super::macro_attempt::{ExternalMacroAttemptCompletion, MacroContinuation};
    use crate::grpc_client::errors::{ErrorDetail as ClientErrorDetail, GrpcError};
    use crate::grpc_client::retry::RetryDecision;
    use tokio::net::TcpSocket;

    let mut socket = None;
    let outcome =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(20), async {
            socket = Some(TcpSocket::new_v4().expect("TEST_CODE External Macro closed socket"));
            socket
                .as_ref()
                .expect("TEST_CODE External Macro closed socket owner")
                .bind(
                    "127.0.0.1:0"
                        .parse()
                        .expect("TEST_CODE External Macro closed socket address"),
                )
                .expect("TEST_CODE External Macro closed socket bind");
            let address = socket
                .as_ref()
                .expect("TEST_CODE External Macro held socket")
                .local_addr()
                .expect("TEST_CODE External Macro socket local address");
            let endpoint_uri = format!("http://{address}");
            let endpoint = Endpoint::from_shared(endpoint_uri.clone())
                .expect("TEST_CODE External Macro closed endpoint")
                .timeout(Duration::from_secs(35))
                .connect_timeout(Duration::from_secs(1));
            let prepared = PreparedExternalEndpoint::from_plaintext_for_test(
                endpoint,
                endpoint_uri.clone(),
                Zeroizing::new(TEST_BEARER.to_owned()),
                TEST_AUTHORITY.to_owned(),
            );
            let original = prepared
                .prepare_macro_query(MacroQueryIdentity::GlobalNews {
                    provider: GlobalNewsProvider::Eastmoney,
                    limit: 20,
                })
                .expect("TEST_CODE External Macro connect request");
            let request_bytes = original.request_bytes();
            let request_id = original.request_id().to_owned();
            let profile = original.profile();
            let authority = original.acquisition_authority().to_owned();
            assert_eq!(original.endpoint_uri(), endpoint_uri);
            assert_eq!(original.retry_policy(), (4, 1000, 60_000, 200));
            assert_eq!(original.attempt_ordinal(), 1);
            drop(original);

            let cases = [
                (
                    (4, 1000, 60_000, 200),
                    1,
                    MacroContinuation::Retry { backoff_ms: 1000 },
                ),
                (
                    (4, 1000, 60_000, 200),
                    2,
                    MacroContinuation::Retry { backoff_ms: 2000 },
                ),
                ((4, 1000, 60_000, 200), 4, MacroContinuation::Terminal),
                (
                    (3, 7, 10, 999),
                    2,
                    MacroContinuation::Retry { backoff_ms: 10 },
                ),
            ];
            for (retry_policy, ordinal, expected_continuation) in cases {
                let request = prepared
                    .resume_macro_query(
                        MacroQueryIdentity::GlobalNews {
                            provider: GlobalNewsProvider::Eastmoney,
                            limit: 20,
                        },
                        fresh_external_macro_material(
                            &endpoint_uri,
                            &request_bytes,
                            &request_id,
                            profile,
                            &authority,
                            retry_policy,
                            ordinal,
                        ),
                    )
                    .expect("TEST_CODE restore External Macro connect request");
                assert_eq!(request.endpoint_uri(), endpoint_uri);
                assert_eq!(request.request_bytes(), request_bytes);
                assert_eq!(request.request_id(), request_id);
                assert_eq!(request.retry_policy(), retry_policy);
                assert_eq!(request.attempt_ordinal(), ordinal);
                let completion = tokio::time::timeout(Duration::from_secs(5), request.execute())
                    .await
                    .expect("TEST_CODE External Macro connect failure deadline")
                    .expect("TEST_CODE External Macro connect failure material");
                match completion {
                    ExternalMacroAttemptCompletion::ConnectUnavailable {
                        error,
                        retry_decision,
                        continuation,
                    } => {
                        assert!(matches!(&error, GrpcError::Unavailable { .. }));
                        assert_eq!(error.details(), &ClientErrorDetail::default());
                        assert_eq!(retry_decision, RetryDecision::RetryBackoff);
                        assert_eq!(continuation, expected_continuation);
                    }
                    ExternalMacroAttemptCompletion::Unary(_) => {
                        panic!("TEST_CODE connect failure became unary material");
                    }
                }
            }
            drop(prepared);
        }))
        .catch_unwind()
        .await;
    drop(socket.take());
    match outcome {
        Ok(result) => result.expect("TEST_CODE External Macro connect failure body timeout"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test]
async fn external_macro_unary_status_remains_distinct_from_connect_unavailable() {
    use super::external_control_loopback_fixture::ObservedHealthTrailer;
    use super::macro_attempt::{
        ExternalMacroAttemptCompletion, MacroContinuation, MacroTrailerMaterial,
    };
    use crate::grpc_client::errors::GrpcError;
    use crate::grpc_client::pb::magic::market::v1::ErrorDetail as WireErrorDetail;
    use crate::grpc_client::retry::RetryDecision;

    let mut server = None;
    let outcome =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(20), async {
            server = Some(ExternalControlLoopbackServer::bind_with_data_status_for_test().await);
            let server = server
                .as_ref()
                .expect("TEST_CODE External Macro Status server owner");
            let prepared = prepared_external_control_with_bearer(server.endpoint(), TEST_BEARER);
            let request = prepared
                .prepare_macro_query(MacroQueryIdentity::GlobalNews {
                    provider: GlobalNewsProvider::Eastmoney,
                    limit: 20,
                })
                .expect("TEST_CODE External Macro Status request");
            let request_id = request.request_id().to_owned();
            let request_bytes = request.request_bytes();
            let execution = request.execute();
            tokio::pin!(execution);
            let receipt_deadline = Instant::now() + Duration::from_secs(5);
            loop {
                tokio::select! {
                    biased;
                    _completion = &mut execution => {
                        panic!("TEST_CODE External Macro Status completed before release");
                    }
                    _ = tokio::task::yield_now() => {}
                }
                if server.snapshot().data_calls > 0 {
                    break;
                }
                assert!(
                    Instant::now() < receipt_deadline,
                    "TEST_CODE External Macro Status receipt watchdog"
                );
            }
            let received = server.snapshot();
            assert_eq!(received.tcp_accepts, 1);
            assert!(received.health_requests.is_empty());
            assert_eq!(received.capabilities_calls, 0);
            assert_eq!(received.data_calls, 1);
            assert_eq!(received.data_methods, vec!["global_news"]);
            assert_eq!(received.data_requests, vec![request_bytes.clone()]);
            assert_eq!(received.data_authorized, vec![true]);
            assert!(received.data_responses.is_empty());
            assert!(received.data_statuses.is_empty());

            server.release_data();
            let completion = tokio::time::timeout(Duration::from_secs(5), &mut execution)
                .await
                .expect("TEST_CODE External Macro Status completion deadline")
                .expect("TEST_CODE External Macro Status outer result");
            let expected_detail = WireErrorDetail {
                request_id: request_id.clone(),
                operation: crate::grpc_client::pb::magic::market::v1::Operation::GlobalNews as i32,
                provider: "Eastmoney".to_owned(),
                reason_code: "no_verified_batch".to_owned(),
                retryable: true,
                ..WireErrorDetail::default()
            };
            let expected_details = expected_detail.encode_to_vec();
            let observed = server.snapshot();
            assert_eq!(observed.data_statuses.len(), 1);
            let status = &observed.data_statuses[0];
            assert_eq!(status.code, tonic::Code::Unavailable as i32);
            assert_eq!(status.details, expected_details);
            assert_eq!(status.trailer, ObservedHealthTrailer::Absent);
            match &completion {
                ExternalMacroAttemptCompletion::Unary(inner) => {
                    assert_eq!(inner.response_bytes, None);
                    assert_eq!(inner.status_code, Some(tonic::Code::Unavailable as i32));
                    assert_eq!(
                        inner.status_details.as_deref(),
                        Some(status.details.as_slice())
                    );
                    assert_eq!(
                        inner.status_error_detail_trailer,
                        MacroTrailerMaterial::Absent
                    );
                    assert_eq!(inner.retry_decision, RetryDecision::RetryBackoff);
                    assert_eq!(
                        inner.continuation,
                        MacroContinuation::Retry { backoff_ms: 1000 }
                    );
                    let error = inner
                        .processed
                        .as_ref()
                        .expect_err("TEST_CODE External Macro Status processed error");
                    assert!(matches!(error, GrpcError::Unavailable { .. }));
                    assert_eq!(error.details().code, tonic::Code::Unavailable.to_string());
                    assert_eq!(
                        error.details().method.map(|method| method.as_str_name()),
                        Some("OPERATION_GLOBAL_NEWS")
                    );
                    assert_eq!(
                        error.details().method.map(|method| method.profile()),
                        Some(ContractProfile::ExternalV1)
                    );
                    assert_eq!(error.details().provider.as_deref(), Some("Eastmoney"));
                    assert_eq!(
                        error.details().reason_code.as_deref(),
                        Some("no_verified_batch")
                    );
                    assert_eq!(error.details().retryable, Some(true));
                    let correlation = error
                        .details()
                        .request_id
                        .as_deref()
                        .expect("TEST_CODE External Macro safe request correlation");
                    assert!(correlation.starts_with("sha256:"));
                    assert_ne!(correlation, request_id);
                    assert!(!format!("{error:?}").contains(&request_id));
                }
                ExternalMacroAttemptCompletion::ConnectUnavailable { .. } => {
                    panic!("TEST_CODE unary Status became ConnectUnavailable");
                }
            }
            assert_eq!(
                WireErrorDetail::decode(status.details.as_slice()).unwrap(),
                expected_detail
            );
            assert_eq!(observed.tcp_accepts, 1);
            assert!(observed.health_requests.is_empty());
            assert_eq!(observed.capabilities_calls, 0);
            assert_eq!(observed.data_calls, 1);
            assert_eq!(observed.data_requests, vec![request_bytes]);
            assert_eq!(observed.data_authorized, vec![true]);
            assert!(observed.data_responses.is_empty());
            drop(completion);
            drop(prepared);
        }))
        .catch_unwind()
        .await;
    let cleanup = match server.take() {
        Some(server) => server.finish().await,
        None => Ok(()),
    };
    if let Err(error) = cleanup {
        panic!("TEST_CODE External Macro Status cleanup failed: {error}");
    }
    match outcome {
        Ok(result) => result.expect("TEST_CODE External Macro Status body timeout"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}
