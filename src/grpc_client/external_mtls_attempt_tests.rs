use super::external_control_attempt::{ExternalControlCompletion, ExternalControlResultMaterial};
use super::external_control_loopback_fixture::{
    test_external_build_identity, test_external_observability, ExternalControlObservation,
    ExternalMtlsMacroFixture,
};
use super::external_query_wire_fixture::ExternalQueryWireFixture;
use super::macro_attempt::{
    ExternalMacroAttemptCompletion, MacroContinuation, MacroQueryIdentity, MacroTrailerMaterial,
    RestoredExternalMacroRequest, RestoredMacroRequest,
};
use super::{ContractProfile, GrpcMarketClient};
use crate::data_gateway::{GatewayBatch, GlobalNewsProvider};
use crate::grpc_client::errors::{ErrorDetail as ClientErrorDetail, GrpcError};
use crate::grpc_client::external_pb::magic::market::v1::{
    system_service_client::SystemServiceClient, AdmissionState,
    CanonicalPayload as ExternalCanonicalPayload, CapabilitiesRequest, CapabilitiesResponse,
    Capability, ErrorDetail as ExternalErrorDetail, EventCursor as ExternalEventCursor,
    EventFilter as ExternalEventFilter,
    HealthRequest, HealthResponse, Operation,
    ListenerStatusRequest as ExternalListenerStatusRequest,
    ListenerStatusResponse as ExternalListenerStatusResponse,
    MarketEventEnvelope as ExternalMarketEventEnvelope, QueryRequest as ExternalQueryRequest,
    QueryResponse as ExternalQueryResponse, RequestContext,
    SetWatchlistRequest as ExternalSetWatchlistRequest,
    SetWatchlistResponse as ExternalSetWatchlistResponse, SubscribeRequest as ExternalSubscribeRequest,
};
use crate::grpc_client::pb::magic::market::v1::{
    AdmissionState as LocalAdmissionState, CanonicalPayload, Operation as LocalOperation,
    QueryRequest, QueryResponse,
};
use crate::grpc_client::retry::RetryDecision;
use futures::FutureExt as _;
use prost::Message as _;
use std::time::{Duration, Instant};
use tonic::metadata::MetadataValue;
use tonic::transport::{Certificate, ClientTlsConfig, Endpoint};

const TEST_BEARER: &str = "TEST_CODE_EXTERNAL_CONTROL_TOKEN";
const TEST_AUTHORITY: &str = "grpc-mtls:macro.test.invalid";
const TEST_BATCH: &str = "TEST_CODE_EXTERNAL_DATA_BATCH";
const TEST_OBSERVED_AT: &str = "2026-09-14T15:31:00+08:00";
const TEST_SOURCE_AT: &str = "2026-09-14 15:30";
const TEST_RECORD_DATA: &[u8] = br#"{"item_id":"TEST_CODE_EXTERNAL_NEWS_001","title":"TEST_CODE external data title","summary":"TEST_CODE external data summary","content":"TEST_CODE external data content","publisher":"TEST_CODE Eastmoney publisher","url":"https://example.com/TEST_CODE_EXTERNAL_NEWS_001","published_at":"2026-09-14T15:30:00+08:00","instruments":[{"exchange":"Shanghai","code":"TEST_CODE_600001","asset_class":"Equity"}],"topics":["TEST_CODE_external_topic"],"language":"zh-CN","evidence":{"provider":"Eastmoney","source_at":"2026-09-14 15:30","observed_at":"2026-09-14T15:31:00+08:00","batch_id":"TEST_CODE_EXTERNAL_DATA_BATCH"}}"#;

fn expected_capabilities(request_id: &str) -> CapabilitiesResponse {
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

fn expected_data_response(request_id: &str) -> QueryResponse {
    QueryResponse {
        request_id: request_id.to_owned(),
        operation: LocalOperation::GlobalNews as i32,
        admission: LocalAdmissionState::Admitted as i32,
        selected_provider: "Eastmoney".to_owned(),
        batch_id: TEST_BATCH.to_owned(),
        complete: true,
        observed_at: TEST_OBSERVED_AT.to_owned(),
        source_at: TEST_SOURCE_AT.to_owned(),
        records: vec![CanonicalPayload {
            schema: "magic.market.news_item".to_owned(),
            schema_version: 2,
            content_type: "application/json; charset=utf-8".to_owned(),
            data: TEST_RECORD_DATA.to_vec(),
        }],
        source: String::new(),
        diagnostic_blocker: String::new(),
    }
}

fn assert_no_rpc(observed: &ExternalControlObservation, expected_tcp: usize) {
    assert_eq!(observed.tcp_accepts, expected_tcp);
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

async fn execute_health(
    fixture: &ExternalMtlsMacroFixture,
    health: super::external_control_attempt::AuthorizedHealthAttempt,
) -> ExternalControlCompletion<HealthResponse> {
    let execution = health.execute();
    tokio::pin!(execution);
    let receipt_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        tokio::select! {
            biased;
            _completion = &mut execution => {
                panic!("TEST_CODE mTLS Health completed before release");
            }
            _ = tokio::task::yield_now() => {}
        }
        if !fixture.snapshot().health_requests.is_empty() {
            break;
        }
        assert!(
            Instant::now() < receipt_deadline,
            "TEST_CODE mTLS Health receipt watchdog"
        );
    }
    fixture.release_health();
    tokio::time::timeout(Duration::from_secs(5), &mut execution)
        .await
        .expect("TEST_CODE mTLS Health completion deadline")
}

async fn execute_capabilities(
    fixture: &ExternalMtlsMacroFixture,
    capabilities: super::external_control_attempt::AuthorizedCapabilitiesAttempt,
) -> ExternalControlCompletion<CapabilitiesResponse> {
    let execution = capabilities.execute();
    tokio::pin!(execution);
    let receipt_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        tokio::select! {
            biased;
            _completion = &mut execution => {
                panic!("TEST_CODE mTLS Capabilities completed before release");
            }
            _ = tokio::task::yield_now() => {}
        }
        if fixture.snapshot().capabilities_calls > 0 {
            break;
        }
        assert!(
            Instant::now() < receipt_deadline,
            "TEST_CODE mTLS Capabilities receipt watchdog"
        );
    }
    fixture.release_capabilities();
    tokio::time::timeout(Duration::from_secs(5), &mut execution)
        .await
        .expect("TEST_CODE mTLS Capabilities completion deadline")
}

#[tokio::test]
async fn external_mtls_bundle_controls_then_restored_cold_data_use_original_requests() {
    let mut fixture = None;
    let outcome =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(30), async {
            fixture = Some(
                ExternalMtlsMacroFixture::bind_data_success_for_test()
                    .await
                    .expect("TEST_CODE mTLS fixture setup"),
            );
            let fixture = fixture.as_ref().expect("TEST_CODE mTLS fixture owner");
            assert!(fixture.bundle_path().is_absolute());
            assert!(fixture.endpoint().starts_with("https://127.0.0.1:"));
            let prepared = GrpcMarketClient::prepare_client_bundle(fixture.bundle_path())
                .expect("TEST_CODE production bundle preparation");
            assert_eq!(prepared.endpoint_uri(), fixture.endpoint());

            let data = prepared
                .prepare_macro_query(MacroQueryIdentity::GlobalNews {
                    provider: GlobalNewsProvider::Eastmoney,
                    limit: 20,
                })
                .expect("TEST_CODE mTLS original data request");
            assert_eq!(data.endpoint_uri(), fixture.endpoint());
            assert_eq!(data.profile(), ContractProfile::ExternalV1);
            assert_eq!(data.acquisition_authority(), TEST_AUTHORITY);
            assert_eq!(data.retry_policy(), (4, 1000, 60_000, 200));
            assert_eq!(data.attempt_ordinal(), 1);
            let data_id = data.request_id().to_owned();
            let data_bytes = data.request_bytes();
            let decoded_data = QueryRequest::decode(data_bytes.as_slice())
                .expect("TEST_CODE decode mTLS data request");
            assert_eq!(decoded_data.encode_to_vec(), data_bytes);
            let data_context = decoded_data
                .context
                .as_ref()
                .expect("TEST_CODE mTLS data context");
            assert_eq!(data_context.protocol_version, 1);
            assert_eq!(data_context.request_id, data_id);
            assert_eq!(decoded_data.preferred_provider, "Eastmoney");
            assert!(!decoded_data.allow_unadmitted);
            let payload = decoded_data
                .payload
                .as_ref()
                .expect("TEST_CODE mTLS data payload");
            assert_eq!(payload.schema, "magic.market.global_news.request");
            assert_eq!(payload.schema_version, 2);
            assert_eq!(payload.content_type, "application/json; charset=utf-8");
            assert_eq!(payload.data, br#"{"limit":20}"#);

            let health = prepared
                .prepare_health_attempt()
                .expect("TEST_CODE mTLS Health request");
            let health_id = health.request_id().to_owned();
            let health_bytes = health.request_bytes();
            let decoded_health = HealthRequest::decode(health_bytes.as_slice())
                .expect("TEST_CODE decode mTLS Health request");
            assert_eq!(decoded_health.encode_to_vec(), health_bytes);
            let health_context = decoded_health
                .context
                .expect("TEST_CODE mTLS Health context");
            assert_eq!(health_context.protocol_version, 1);
            assert_eq!(health_context.request_id, health_id);

            let capabilities = prepared
                .prepare_capabilities_attempt()
                .expect("TEST_CODE mTLS Capabilities request");
            let capabilities_id = capabilities.request_id().to_owned();
            let capabilities_bytes = capabilities.request_bytes();
            let decoded_capabilities = CapabilitiesRequest::decode(capabilities_bytes.as_slice())
                .expect("TEST_CODE decode mTLS Capabilities request");
            assert_eq!(decoded_capabilities.encode_to_vec(), capabilities_bytes);
            let capabilities_context = decoded_capabilities
                .context
                .expect("TEST_CODE mTLS Capabilities context");
            assert_eq!(capabilities_context.protocol_version, 1);
            assert_eq!(capabilities_context.request_id, capabilities_id);
            assert_ne!(health_id, capabilities_id);
            assert_ne!(data_id, health_id);
            assert_ne!(data_id, capabilities_id);
            for bytes in [&data_bytes, &health_bytes, &capabilities_bytes] {
                assert!(!bytes
                    .windows(TEST_BEARER.len())
                    .any(|window| window == TEST_BEARER.as_bytes()));
                assert!(!bytes
                    .windows(b"authorization".len())
                    .any(|window| window == b"authorization"));
            }
            tokio::task::yield_now().await;
            tokio::task::yield_now().await;
            assert_no_rpc(&fixture.snapshot(), 0);

            let health_completion = execute_health(fixture, health).await;
            health_completion
                .processed()
                .expect("TEST_CODE mTLS Health processed");
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
                _ => panic!("TEST_CODE expected mTLS Health response"),
            }
            let after_health = fixture.snapshot();
            assert_eq!(after_health.tcp_accepts, 1);
            assert_eq!(after_health.health_requests, vec![health_bytes.clone()]);
            assert_eq!(after_health.health_authorized, vec![true]);
            assert_eq!(
                after_health.health_responses,
                vec![expected_health.encode_to_vec()]
            );
            assert_eq!(after_health.capabilities_calls, 0);
            assert_eq!(after_health.data_calls, 0);
            let connected = health_completion
                .into_connected_client()
                .expect("TEST_CODE mTLS Health connected client");

            let capabilities = capabilities
                .bind_connected(connected)
                .expect("TEST_CODE bind mTLS Capabilities to Health connection");
            let capabilities_completion = execute_capabilities(fixture, capabilities).await;
            capabilities_completion
                .processed()
                .expect("TEST_CODE mTLS Capabilities processed");
            let expected_capabilities = expected_capabilities(&capabilities_id);
            match capabilities_completion.result_material() {
                ExternalControlResultMaterial::Response { bytes, response } => {
                    assert_eq!(bytes, expected_capabilities.encode_to_vec());
                    assert_eq!(response, &expected_capabilities);
                }
                _ => panic!("TEST_CODE expected mTLS Capabilities response"),
            }
            let after_capabilities = fixture.snapshot();
            assert_eq!(after_capabilities.tcp_accepts, 1);
            assert_eq!(after_capabilities.health_requests, vec![health_bytes]);
            assert_eq!(after_capabilities.capabilities_calls, 1);
            assert_eq!(
                after_capabilities.capabilities_requests,
                vec![capabilities_bytes]
            );
            assert_eq!(after_capabilities.capabilities_authorized, vec![true]);
            assert_eq!(
                after_capabilities.capabilities_responses,
                vec![expected_capabilities.encode_to_vec()]
            );
            assert_eq!(after_capabilities.data_calls, 0);
            let connected = capabilities_completion
                .into_connected_client()
                .expect("TEST_CODE mTLS Capabilities connected client");
            drop(connected);

            let restored = RestoredExternalMacroRequest {
                endpoint_uri: data.endpoint_uri().to_owned(),
                request: RestoredMacroRequest {
                    request_bytes: data_bytes.clone(),
                    request_id: data_id.clone(),
                    profile: data.profile(),
                    acquisition_authority: Some(data.acquisition_authority().to_owned()),
                    retry_policy: data.retry_policy(),
                    next_attempt: data.attempt_ordinal(),
                },
            };
            drop(data);
            drop(prepared);
            let reopened = GrpcMarketClient::prepare_client_bundle(fixture.bundle_path())
                .expect("TEST_CODE reopen production bundle");
            let resumed = reopened
                .resume_macro_query(
                    MacroQueryIdentity::GlobalNews {
                        provider: GlobalNewsProvider::Eastmoney,
                        limit: 20,
                    },
                    restored,
                )
                .expect("TEST_CODE restore mTLS data request");
            assert_eq!(resumed.endpoint_uri(), fixture.endpoint());
            assert_eq!(resumed.profile(), ContractProfile::ExternalV1);
            assert_eq!(resumed.acquisition_authority(), TEST_AUTHORITY);
            assert_eq!(resumed.retry_policy(), (4, 1000, 60_000, 200));
            assert_eq!(resumed.attempt_ordinal(), 1);
            assert_eq!(resumed.request_id(), data_id);
            assert_eq!(resumed.request_bytes(), data_bytes);
            tokio::task::yield_now().await;
            tokio::task::yield_now().await;
            let before_data = fixture.snapshot();
            assert_eq!(before_data.tcp_accepts, 1);
            assert_eq!(before_data.health_requests.len(), 1);
            assert_eq!(before_data.capabilities_calls, 1);
            assert_eq!(before_data.data_calls, 0);

            let execution = resumed.execute();
            tokio::pin!(execution);
            let receipt_deadline = Instant::now() + Duration::from_secs(5);
            loop {
                tokio::select! {
                    biased;
                    _completion = &mut execution => {
                        panic!("TEST_CODE mTLS data completed before release");
                    }
                    _ = tokio::task::yield_now() => {}
                }
                if fixture.snapshot().data_calls > 0 {
                    break;
                }
                assert!(
                    Instant::now() < receipt_deadline,
                    "TEST_CODE mTLS data receipt watchdog"
                );
            }
            let received_data = fixture.snapshot();
            assert_eq!(received_data.tcp_accepts, 2);
            assert_eq!(received_data.health_requests.len(), 1);
            assert_eq!(received_data.capabilities_calls, 1);
            assert_eq!(received_data.data_calls, 1);
            assert_eq!(received_data.data_methods, vec!["global_news"]);
            assert_eq!(received_data.data_requests, vec![data_bytes.clone()]);
            assert_eq!(received_data.data_authorized, vec![true]);
            assert!(received_data.data_responses.is_empty());
            fixture.release_data();
            let completion = tokio::time::timeout(Duration::from_secs(5), &mut execution)
                .await
                .expect("TEST_CODE mTLS data completion deadline")
                .expect("TEST_CODE mTLS data outer result");
            let expected_response = expected_data_response(&data_id);
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
                        .expect("TEST_CODE mTLS data native projection");
                    assert_eq!(processed.admission, LocalAdmissionState::Admitted);
                    assert_eq!(processed.selected_provider, "Eastmoney");
                    assert_eq!(processed.batch_id, TEST_BATCH);
                    assert!(processed.complete);
                    assert_eq!(processed.observed_at, TEST_OBSERVED_AT);
                    assert_eq!(processed.source_at, TEST_SOURCE_AT);
                    assert_eq!(processed.records, expected_response.records);
                    assert_eq!(processed.source(), TEST_AUTHORITY);
                    assert!(processed.diagnostic_blocker.is_empty());

                    let batch = crate::data_gateway::grpc_source::convert::external_global_news(
                        GlobalNewsProvider::Eastmoney,
                        processed,
                    )
                    .expect("TEST_CODE mTLS GlobalNews converter");
                    assert!(matches!(&batch, GatewayBatch::Available { .. }));
                    assert_eq!(batch.records().len(), 1);
                    let evidence = batch.evidence();
                    assert_eq!(
                        evidence.provider,
                        crate::market_domain::ProviderId::Eastmoney
                    );
                    assert_eq!(evidence.source, "eastmoney-web");
                    assert_eq!(evidence.source_at.as_deref(), Some(TEST_SOURCE_AT));
                    assert_eq!(evidence.observed_at, TEST_OBSERVED_AT);
                    assert_eq!(evidence.batch_id, TEST_BATCH);
                    let record = &batch.records()[0];
                    assert_eq!(record.item_id, "TEST_CODE_EXTERNAL_NEWS_001");
                    assert_eq!(record.title, "TEST_CODE external data title");
                    assert_eq!(
                        record.summary.as_deref(),
                        Some("TEST_CODE external data summary")
                    );
                    assert_eq!(
                        record.content.as_deref(),
                        Some("TEST_CODE external data content")
                    );
                    assert_eq!(record.publisher, "TEST_CODE Eastmoney publisher");
                    assert_eq!(
                        record.canonical_url,
                        "https://example.com/TEST_CODE_EXTERNAL_NEWS_001"
                    );
                    assert_eq!(
                        record.published_at,
                        chrono::DateTime::parse_from_rfc3339("2026-09-14T15:30:00+08:00")
                            .expect("TEST_CODE mTLS published_at literal")
                            .with_timezone(&chrono::Utc)
                    );
                    assert_eq!(
                        record.observed_at,
                        chrono::DateTime::parse_from_rfc3339("2026-09-14T15:31:00+08:00")
                            .expect("TEST_CODE mTLS observed_at literal")
                            .with_timezone(&chrono::Utc)
                    );
                    assert_eq!(record.instruments, vec!["TEST_CODE_600001"]);
                    assert_eq!(record.topics, vec!["TEST_CODE_external_topic"]);
                    assert_eq!(record.language, "zh-CN");
                    assert_eq!(
                        record.evidence.provider(),
                        crate::market_domain::ProviderId::Eastmoney
                    );
                    assert_eq!(record.evidence.source_at(), Some(TEST_SOURCE_AT));
                    assert_eq!(record.evidence.observed_at(), TEST_OBSERVED_AT);
                    assert_eq!(record.evidence.batch_id(), TEST_BATCH);
                }
                ExternalMacroAttemptCompletion::ConnectUnavailable { .. } => {
                    panic!("TEST_CODE mTLS data unexpectedly failed to connect");
                }
            }
            let final_observed = fixture.snapshot();
            assert_eq!(final_observed.tcp_accepts, 2);
            assert_eq!(final_observed.health_requests.len(), 1);
            assert_eq!(final_observed.capabilities_calls, 1);
            assert_eq!(final_observed.data_calls, 1);
            assert_eq!(final_observed.data_requests, vec![data_bytes]);
            assert_eq!(final_observed.data_authorized, vec![true]);
            assert_eq!(final_observed.data_responses, vec![expected_response_bytes]);
            assert!(final_observed.data_statuses.is_empty());
            drop(completion);
            drop(reopened);
        }))
        .catch_unwind()
        .await;
    let cleanup = match fixture.take() {
        Some(fixture) => fixture.finish().await,
        None => Ok(()),
    };
    if let Err(error) = cleanup {
        panic!("TEST_CODE mTLS controls/data cleanup failed: {error}");
    }
    match outcome {
        Ok(result) => result.expect("TEST_CODE mTLS controls/data body timeout"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test]
async fn external_mtls_server_requires_client_certificate_and_exact_server_name() {
    let mut fixture = None;
    let outcome =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(30), async {
            fixture = Some(
                ExternalMtlsMacroFixture::bind_data_success_for_test()
                    .await
                    .expect("TEST_CODE mTLS rejection fixture setup"),
            );
            let fixture = fixture
                .as_ref()
                .expect("TEST_CODE mTLS rejection fixture owner");

            let valid = GrpcMarketClient::prepare_client_bundle(fixture.bundle_path())
                .expect("TEST_CODE valid mTLS production bundle");
            let health = valid
                .prepare_health_attempt()
                .expect("TEST_CODE valid mTLS Health");
            let valid_health_bytes = health.request_bytes();
            let valid_completion = execute_health(fixture, health).await;
            valid_completion
                .processed()
                .expect("TEST_CODE valid mTLS Health processed");
            assert!(matches!(
                valid_completion.result_material(),
                ExternalControlResultMaterial::Response { .. }
            ));
            let valid_client = valid_completion
                .into_connected_client()
                .expect("TEST_CODE valid mTLS connected client");
            drop(valid_client);
            drop(valid);
            let positive = fixture.snapshot();
            assert_eq!(positive.tcp_accepts, 1);
            assert_eq!(positive.health_requests, vec![valid_health_bytes]);
            assert_eq!(positive.health_authorized, vec![true]);
            assert_eq!(positive.health_responses.len(), 1);
            assert_eq!(positive.capabilities_calls, 0);
            assert_eq!(positive.data_calls, 0);

            let wrong_name =
                GrpcMarketClient::prepare_client_bundle(fixture.wrong_name_bundle_path())
                    .expect("TEST_CODE wrong-name bundle parses");
            let wrong_health = wrong_name
                .prepare_health_attempt()
                .expect("TEST_CODE wrong-name Health preparation");
            let wrong_completion =
                tokio::time::timeout(Duration::from_secs(5), wrong_health.execute())
                    .await
                    .expect("TEST_CODE wrong-name hard watchdog");
            let wrong_error = wrong_completion
                .processed()
                .expect_err("TEST_CODE wrong server name must fail");
            assert!(matches!(wrong_error, GrpcError::Unavailable { .. }));
            assert_eq!(wrong_error.details(), &ClientErrorDetail::default());
            match wrong_completion.result_material() {
                ExternalControlResultMaterial::ConnectUnavailable { error } => {
                    assert!(std::ptr::eq(error, wrong_error));
                }
                _ => panic!("TEST_CODE wrong server name became RPC material"),
            }
            assert!(wrong_completion.into_connected_client().is_none());
            drop(wrong_name);
            let wrong_tcp_deadline = Instant::now() + Duration::from_secs(5);
            while fixture.snapshot().tcp_accepts < 2 {
                tokio::task::yield_now().await;
                assert!(
                    Instant::now() < wrong_tcp_deadline,
                    "TEST_CODE wrong-name TCP receipt watchdog"
                );
            }
            let after_wrong = fixture.snapshot();
            assert_eq!(after_wrong.health_requests.len(), 1);
            assert_eq!(after_wrong.capabilities_calls, 0);
            assert_eq!(after_wrong.data_calls, 0);

            let no_identity_tls = ClientTlsConfig::new()
                .domain_name("macro.test.invalid")
                .ca_certificate(Certificate::from_pem(include_bytes!(
                    "testdata/external_mtls/ca-cert.pem"
                )));
            let no_identity_endpoint = Endpoint::from_shared(fixture.endpoint().to_owned())
                .expect("TEST_CODE no-client-cert endpoint")
                .timeout(Duration::from_secs(35))
                .connect_timeout(Duration::from_secs(5))
                .tls_config(no_identity_tls)
                .expect("TEST_CODE no-client-cert TLS config");
            let no_identity_result =
                tokio::time::timeout(Duration::from_secs(5), no_identity_endpoint.connect())
                    .await
                    .expect("TEST_CODE no-client-cert connect watchdog");
            if let Ok(channel) = no_identity_result {
                let mut client = SystemServiceClient::new(channel);
                let mut request = tonic::Request::new(HealthRequest {
                    context: Some(RequestContext {
                        protocol_version: 1,
                        request_id: "TEST_CODE_NO_CLIENT_CERT_HEALTH".to_owned(),
                    }),
                });
                request.metadata_mut().insert(
                    "authorization",
                    MetadataValue::try_from("Bearer TEST_CODE_EXTERNAL_CONTROL_TOKEN")
                        .expect("TEST_CODE no-client-cert bearer metadata"),
                );
                match tokio::time::timeout(Duration::from_secs(5), client.get_health(request)).await
                {
                    Ok(Err(_)) => {}
                    Ok(Ok(_)) => panic!("TEST_CODE server accepted missing client certificate"),
                    Err(_) => panic!("TEST_CODE no-client-cert RPC watchdog elapsed"),
                }
                drop(client);
            }
            let no_identity_tcp_deadline = Instant::now() + Duration::from_secs(5);
            while fixture.snapshot().tcp_accepts <= after_wrong.tcp_accepts {
                tokio::task::yield_now().await;
                assert!(
                    Instant::now() < no_identity_tcp_deadline,
                    "TEST_CODE no-client-cert TCP receipt watchdog"
                );
            }
            let final_observed = fixture.snapshot();
            assert_eq!(final_observed.health_requests.len(), 1);
            assert_eq!(final_observed.health_authorized, vec![true]);
            assert_eq!(final_observed.health_responses.len(), 1);
            assert_eq!(final_observed.capabilities_calls, 0);
            assert_eq!(final_observed.data_calls, 0);
        }))
        .catch_unwind()
        .await;
    let cleanup = match fixture.take() {
        Some(fixture) => fixture.finish().await,
        None => Ok(()),
    };
    if let Err(error) = cleanup {
        panic!("TEST_CODE mTLS rejection cleanup failed: {error}");
    }
    match outcome {
        Ok(result) => result.expect("TEST_CODE mTLS rejection body timeout"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test]
async fn grpc_dual_contract_external_three_query_routes_use_generated_client_and_server_methods() {
    let mut fixture = None;
    let outcome =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(30), async {
            fixture = Some(
                ExternalQueryWireFixture::bind_generated_routes()
                    .await
                    .expect("TEST_CODE External generated routes fixture setup"),
            );
            let fixture = fixture
                .as_ref()
                .expect("TEST_CODE External generated routes fixture owner");
            let prepared = GrpcMarketClient::prepare_client_bundle(fixture.bundle_path())
                .expect("TEST_CODE production External generated routes bundle");
            assert_eq!(prepared.endpoint_uri(), fixture.endpoint());
            let mut client = prepared
                .connect_once()
                .await
                .expect("TEST_CODE External generated routes connection");

            let cases = [
                (
                    LocalOperation::SecurityMetadata,
                    serde_json::json!({
                        "instruments": [{
                            "exchange": "Shanghai",
                            "code": "600001",
                            "asset_class": "Equity"
                        }]
                    }),
                    serde_json::json!({
                        "instruments": [{
                            "exchange": "Shanghai",
                            "code": "600001",
                            "asset_class": "Equity"
                        }]
                    }),
                    "magic.market.security_metadata.request",
                    1_u32,
                    "",
                ),
                (
                    LocalOperation::GlobalNews,
                    serde_json::json!({"provider": "Eastmoney", "limit": 20}),
                    serde_json::json!({"limit": 20}),
                    "magic.market.global_news.request",
                    2_u32,
                    "Eastmoney",
                ),
                (
                    LocalOperation::InstrumentNews,
                    serde_json::json!({
                        "instrument": {
                            "exchange": "Shenzhen",
                            "code": "000001",
                            "asset_class": "Equity"
                        },
                        "start": "2026-09-14",
                        "end": "2026-09-14",
                        "limit": 100,
                        "captured_through": "2026-09-14T15:31:00+08:00"
                    }),
                    serde_json::json!({
                        "instrument": {
                            "exchange": "Shenzhen",
                            "code": "000001",
                            "asset_class": "Equity"
                        },
                        "start": "2026-09-14",
                        "end": "2026-09-14",
                        "limit": 100,
                        "captured_through": "2026-09-14T15:31:00+08:00"
                    }),
                    "magic.market.instrument_news.request",
                    2_u32,
                    "",
                ),
            ];

            for (operation, payload, _, _, _, _) in &cases {
                fixture.release();
                let result = client
                    .query(*operation, payload.clone())
                    .await
                    .expect("TEST_CODE External generated route result");
                assert_eq!(result.admission, LocalAdmissionState::Admitted);
                assert_eq!(result.batch_id, TEST_BATCH);
                assert!(result.complete);
                assert_eq!(result.source(), TEST_AUTHORITY);
                assert!(matches!(
                    result.provenance,
                    crate::grpc_client::envelope::AcquisitionProvenance::ExternalMtlsAuthority(
                        ref authority
                    ) if authority == TEST_AUTHORITY
                ));
            }

            let observed = fixture.snapshot();
            assert_eq!(observed.tcp_accepts, 1);
            assert_eq!(observed.calls, 3);
            assert_eq!(observed.authorized, vec![true, true, true]);
            assert_eq!(
                observed.methods,
                vec!["security_metadata", "global_news", "instrument_news"]
            );
            assert!(observed.protobuf_payloads.is_empty());
            assert!(observed.unexpected_methods.is_empty());
            assert_eq!(observed.requests.len(), cases.len());
            for (bytes, (_, _, expected_json, schema, schema_version, provider)) in
                observed.requests.iter().zip(cases.iter())
            {
                let request = ExternalQueryRequest::decode(bytes.as_slice())
                    .expect("TEST_CODE External generated routed request");
                assert_eq!(request.encode_to_vec(), *bytes);
                let context = request
                    .context
                    .as_ref()
                    .expect("TEST_CODE External generated routed context");
                assert_eq!(context.protocol_version, 1);
                assert!(!context.request_id.is_empty());
                assert_eq!(request.preferred_provider, *provider);
                assert!(!request.allow_unadmitted);
                let payload = request
                    .payload
                    .as_ref()
                    .expect("TEST_CODE External generated routed payload");
                assert_eq!(payload.schema, *schema);
                assert_eq!(payload.schema_version, *schema_version);
                assert_eq!(payload.content_type, "application/json; charset=utf-8");
                let actual_json: serde_json::Value = serde_json::from_slice(&payload.data)
                    .expect("TEST_CODE External generated routed payload JSON");
                assert_eq!(actual_json, *expected_json);
            }
        }))
        .catch_unwind()
        .await;
    let cleanup = match fixture.take() {
        Some(fixture) => fixture.finish().await,
        None => Ok(()),
    };
    if let Err(error) = cleanup {
        panic!("TEST_CODE External generated routes cleanup failed: {error}");
    }
    match outcome {
        Ok(result) => result.expect("TEST_CODE External generated routes body timeout"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[derive(Clone, Copy)]
enum ExternalWireBoundaryCase {
    NonEmptySource,
    WrongWireSource,
    MalformedPayload,
}

async fn run_external_wire_boundary_case(case: ExternalWireBoundaryCase) {
    let mut fixture = None;
    let outcome =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(30), async {
            fixture = Some(
                match case {
                    ExternalWireBoundaryCase::NonEmptySource => {
                        ExternalQueryWireFixture::bind_nonempty_source().await
                    }
                    ExternalWireBoundaryCase::WrongWireSource => {
                        ExternalQueryWireFixture::bind_wrong_wire_source().await
                    }
                    ExternalWireBoundaryCase::MalformedPayload => {
                        ExternalQueryWireFixture::bind_malformed_payload().await
                    }
                }
                .expect("TEST_CODE External wire boundary fixture setup"),
            );
            let fixture = fixture
                .as_ref()
                .expect("TEST_CODE External wire boundary fixture owner");
            let prepared = GrpcMarketClient::prepare_client_bundle(fixture.bundle_path())
                .expect("TEST_CODE production External wire boundary bundle");
            let attempt = prepared
                .prepare_macro_query(MacroQueryIdentity::GlobalNews {
                    provider: GlobalNewsProvider::Eastmoney,
                    limit: 20,
                })
                .expect("TEST_CODE production External wire boundary attempt");
            assert_eq!(attempt.profile(), ContractProfile::ExternalV1);
            assert_eq!(attempt.acquisition_authority(), TEST_AUTHORITY);
            assert_eq!(attempt.retry_policy(), (4, 1000, 60_000, 200));
            assert_eq!(attempt.attempt_ordinal(), 1);
            let request_id = attempt.request_id().to_owned();
            let request_bytes = attempt.request_bytes();
            let request = ExternalQueryRequest::decode(request_bytes.as_slice())
                .expect("TEST_CODE native External wire boundary request");
            assert_eq!(request.encode_to_vec(), request_bytes);
            let context = request
                .context
                .as_ref()
                .expect("TEST_CODE native External wire boundary context");
            assert_eq!(context.protocol_version, 1);
            assert_eq!(context.request_id, request_id);
            assert_eq!(request.preferred_provider, "Eastmoney");
            assert!(!request.allow_unadmitted);
            let payload = request
                .payload
                .as_ref()
                .expect("TEST_CODE native External wire boundary payload");
            assert_eq!(payload.schema, "magic.market.global_news.request");
            assert_eq!(payload.schema_version, 2);
            assert_eq!(payload.content_type, "application/json; charset=utf-8");
            assert_eq!(payload.data, br#"{"limit":20}"#);
            assert_eq!(fixture.snapshot(), Default::default());

            let execution = attempt.execute();
            tokio::pin!(execution);
            let receipt_deadline = Instant::now() + Duration::from_secs(5);
            loop {
                tokio::select! {
                    biased;
                    _completion = &mut execution => {
                        panic!("TEST_CODE External wire boundary completed before release");
                    }
                    _ = tokio::task::yield_now() => {}
                }
                if fixture.snapshot().calls > 0 {
                    break;
                }
                assert!(
                    Instant::now() < receipt_deadline,
                    "TEST_CODE External wire boundary receipt watchdog"
                );
            }
            let received = fixture.snapshot();
            assert_eq!(received.tcp_accepts, 1);
            assert_eq!(received.calls, 1);
            assert_eq!(received.authorized, vec![true]);
            assert_eq!(received.methods, vec!["global_news"]);
            assert_eq!(received.requests, vec![request_bytes]);
            assert!(received.protobuf_payloads.is_empty());
            assert!(received.unexpected_methods.is_empty());

            fixture.release();
            let completion = tokio::time::timeout(Duration::from_secs(5), &mut execution)
                .await
                .expect("TEST_CODE External wire boundary completion deadline")
                .expect("TEST_CODE External wire boundary outer result");
            let observed = fixture.snapshot();
            assert_eq!(observed.tcp_accepts, 1);
            assert_eq!(observed.calls, 1);
            assert_eq!(observed.authorized, vec![true]);
            assert_eq!(observed.methods, vec!["global_news"]);
            assert!(observed.unexpected_methods.is_empty());
            assert_eq!(observed.protobuf_payloads.len(), 1);
            let expected_payload = &observed.protobuf_payloads[0];

            let ExternalMacroAttemptCompletion::Unary(inner) = completion else {
                panic!("TEST_CODE expected External wire boundary unary completion");
            };
            assert_eq!(inner.retry_decision, RetryDecision::NoRetry);
            assert_eq!(inner.continuation, MacroContinuation::Terminal);
            match case {
                ExternalWireBoundaryCase::NonEmptySource
                | ExternalWireBoundaryCase::WrongWireSource => {
                    let (suffix, code) = match case {
                        ExternalWireBoundaryCase::NonEmptySource => {
                            (&[0x5a, 0x01, b'x'][..], "external_source_field_conflict")
                        }
                        ExternalWireBoundaryCase::WrongWireSource => {
                            (&[0x58, 0x00][..], "external_response_wire_invalid")
                        }
                        ExternalWireBoundaryCase::MalformedPayload => unreachable!(),
                    };
                    assert!(expected_payload.ends_with(suffix));
                    let native = ExternalQueryResponse::decode(expected_payload.as_slice())
                        .expect("TEST_CODE generated decoder accepts field11 boundary");
                    assert_eq!(native.request_id, request_id);
                    assert_eq!(native.operation, Operation::GlobalNews as i32);
                    assert_eq!(
                        inner.response_bytes.as_deref(),
                        Some(expected_payload.as_slice())
                    );
                    assert_eq!(inner.status_code, None);
                    assert_eq!(inner.status_details, None);
                    assert_eq!(
                        inner.status_error_detail_trailer,
                        MacroTrailerMaterial::Absent
                    );
                    let evidence = inner
                        .external_wire
                        .as_ref()
                        .expect("TEST_CODE field11 boundary capture evidence");
                    assert_eq!(evidence.payload(), Some(expected_payload.as_slice()));
                    let error = inner
                        .processed
                        .as_ref()
                        .expect_err("TEST_CODE field11 boundary must reject locally");
                    assert!(matches!(error, GrpcError::FailedPrecondition { .. }));
                    assert_eq!(error.details().code, code);
                    assert_eq!(error.details().reason_code.as_deref(), Some(code));
                    assert_eq!(error.details().retryable, Some(false));
                }
                ExternalWireBoundaryCase::MalformedPayload => {
                    assert!(expected_payload.ends_with(&[0x6a, 0x02, 0x01]));
                    assert!(ExternalQueryResponse::decode(expected_payload.as_slice()).is_err());
                    assert_eq!(inner.response_bytes, None);
                    assert_eq!(inner.status_code, Some(tonic::Code::Internal as i32));
                    assert_eq!(inner.status_details, Some(Vec::new()));
                    assert_eq!(
                        inner.status_error_detail_trailer,
                        MacroTrailerMaterial::Absent
                    );
                    assert!(inner.external_wire.is_none());
                    let error = inner
                        .processed
                        .as_ref()
                        .expect_err("TEST_CODE malformed generated payload must be status error");
                    assert!(matches!(error, GrpcError::Internal { .. }));
                    assert_eq!(error.details().code, "Internal error");
                    assert_eq!(
                        error.safe_diagnostic(),
                        Some("[redacted-unclassified-status]")
                    );
                }
            }
        }))
        .catch_unwind()
        .await;
    let cleanup = match fixture.take() {
        Some(fixture) => fixture.finish().await,
        None => Ok(()),
    };
    if let Err(error) = cleanup {
        panic!("TEST_CODE External wire boundary cleanup failed: {error}");
    }
    match outcome {
        Ok(result) => result.expect("TEST_CODE External wire boundary body timeout"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test]
async fn grpc_dual_contract_external_global_news_rejects_nonempty_source11_without_status() {
    run_external_wire_boundary_case(ExternalWireBoundaryCase::NonEmptySource).await;
}

#[tokio::test]
async fn grpc_dual_contract_external_global_news_rejects_wrong_wire_source11_without_status() {
    run_external_wire_boundary_case(ExternalWireBoundaryCase::WrongWireSource).await;
}

#[tokio::test]
async fn grpc_dual_contract_external_global_news_classifies_malformed_payload_as_generated_status()
{
    run_external_wire_boundary_case(ExternalWireBoundaryCase::MalformedPayload).await;
}

#[tokio::test]
async fn grpc_dual_contract_external_global_news_accepts_generated_unknown_group_and_preserves_payload(
) {
    let mut fixture = None;
    let outcome =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(30), async {
            fixture = Some(
                ExternalQueryWireFixture::bind_unknown_group()
                    .await
                    .expect("TEST_CODE External unknown-group fixture setup"),
            );
            let fixture = fixture
                .as_ref()
                .expect("TEST_CODE External unknown-group fixture owner");
            let prepared = GrpcMarketClient::prepare_client_bundle(fixture.bundle_path())
                .expect("TEST_CODE production External unknown-group bundle");
            assert_eq!(prepared.endpoint_uri(), fixture.endpoint());
            let attempt = prepared
                .prepare_macro_query(MacroQueryIdentity::GlobalNews {
                    provider: GlobalNewsProvider::Eastmoney,
                    limit: 20,
                })
                .expect("TEST_CODE production External unknown-group attempt");
            assert_eq!(attempt.profile(), ContractProfile::ExternalV1);
            assert_eq!(attempt.acquisition_authority(), TEST_AUTHORITY);
            let request_id = attempt.request_id().to_owned();
            let request_bytes = attempt.request_bytes();
            let request = ExternalQueryRequest::decode(request_bytes.as_slice())
                .expect("TEST_CODE native External unknown-group request");
            assert_eq!(request.encode_to_vec(), request_bytes);
            assert_eq!(fixture.snapshot(), Default::default());

            let execution = attempt.execute();
            tokio::pin!(execution);
            let receipt_deadline = Instant::now() + Duration::from_secs(5);
            loop {
                tokio::select! {
                    biased;
                    _completion = &mut execution => {
                        panic!("TEST_CODE External unknown-group completed before release");
                    }
                    _ = tokio::task::yield_now() => {}
                }
                if fixture.snapshot().calls > 0 {
                    break;
                }
                assert!(
                    Instant::now() < receipt_deadline,
                    "TEST_CODE External unknown-group receipt watchdog"
                );
            }
            let received = fixture.snapshot();
            assert_eq!(received.tcp_accepts, 1);
            assert_eq!(received.calls, 1);
            assert_eq!(received.authorized, vec![true]);
            assert_eq!(received.methods, vec!["global_news"]);
            assert_eq!(received.requests, vec![request_bytes]);
            assert!(received.protobuf_payloads.is_empty());
            assert!(received.unexpected_methods.is_empty());

            fixture.release();
            let completion = tokio::time::timeout(Duration::from_secs(5), &mut execution)
                .await
                .expect("TEST_CODE External unknown-group completion deadline")
                .expect("TEST_CODE External unknown-group outer result");
            let observed = fixture.snapshot();
            assert_eq!(observed.tcp_accepts, 1);
            assert_eq!(observed.calls, 1);
            assert_eq!(observed.authorized, vec![true]);
            assert_eq!(observed.methods, vec!["global_news"]);
            assert!(observed.unexpected_methods.is_empty());
            assert_eq!(observed.protobuf_payloads.len(), 1);
            let expected_payload = &observed.protobuf_payloads[0];
            assert!(expected_payload.ends_with(&[0x63, 0x68, 0x01, 0x64]));
            let native = ExternalQueryResponse::decode(expected_payload.as_slice())
                .expect("TEST_CODE External generated decoder accepts unknown group");
            assert_eq!(native.request_id, request_id);
            assert_eq!(native.operation, Operation::GlobalNews as i32);
            assert_eq!(native.admission, AdmissionState::Admitted as i32);
            assert_eq!(native.selected_provider, "Eastmoney");
            assert_eq!(native.batch_id, TEST_BATCH);
            assert!(native.complete);
            assert_eq!(native.observed_at, TEST_OBSERVED_AT);
            assert_eq!(native.source_at, TEST_SOURCE_AT);
            assert_eq!(native.records.len(), 1);
            assert!(native.diagnostic_blocker.is_empty());
            assert_ne!(native.encode_to_vec().as_slice(), expected_payload.as_slice());

            let ExternalMacroAttemptCompletion::Unary(inner) = completion else {
                panic!("TEST_CODE expected External unknown-group unary completion");
            };
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
                .expect("TEST_CODE unknown non-field11 group must remain admitted");
            assert_eq!(processed.admission, LocalAdmissionState::Admitted);
            assert_eq!(processed.selected_provider, "Eastmoney");
            assert_eq!(processed.batch_id, TEST_BATCH);
            assert!(processed.complete);
            assert_eq!(processed.observed_at, TEST_OBSERVED_AT);
            assert_eq!(processed.source_at, TEST_SOURCE_AT);
            assert_eq!(processed.records.len(), 1);
            assert_eq!(processed.source(), TEST_AUTHORITY);
            assert!(matches!(
                &processed.provenance,
                crate::grpc_client::envelope::AcquisitionProvenance::ExternalMtlsAuthority(
                    authority
                ) if authority == TEST_AUTHORITY
            ));
            assert!(processed.diagnostic_blocker.is_empty());
            assert_eq!(
                inner.response_bytes.as_deref(),
                Some(expected_payload.as_slice()),
                "unknown group bytes must remain the exact captured payload"
            );
        }))
        .catch_unwind()
        .await;
    let cleanup = match fixture.take() {
        Some(fixture) => fixture.finish().await,
        None => Ok(()),
    };
    if let Err(error) = cleanup {
        panic!("TEST_CODE External unknown-group cleanup failed: {error}");
    }
    match outcome {
        Ok(result) => result.expect("TEST_CODE External unknown-group body timeout"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

async fn assert_capability_authorized_attempt_trace(unpublished_provider: bool) {
    let mut fixture = None;
    let outcome =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(30), async {
            fixture = Some(
                ExternalQueryWireFixture::bind_provider_attempts(unpublished_provider)
                    .await
                    .expect("TEST_CODE provider attempts fixture setup"),
            );
            let fixture = fixture
                .as_ref()
                .expect("TEST_CODE provider attempts fixture owner");
            assert!(fixture.endpoint().starts_with("https://127.0.0.1:"));
            let mut client = GrpcMarketClient::connect_client_bundle(fixture.bundle_path())
                .await
                .expect("TEST_CODE provider attempts client connection");

            fixture.release_capabilities();
            let capabilities = client
                .get_external_capabilities()
                .await
                .expect("TEST_CODE same-endpoint Capabilities response");
            assert_eq!(
                capabilities
                    .iter()
                    .map(|capability| capability.provider.as_str())
                    .collect::<Vec<_>>(),
                ["Eastmoney", "Bocha"],
            );
            let after_capabilities = fixture.snapshot();
            assert_eq!(after_capabilities.capabilities_calls, 1);
            assert_eq!(after_capabilities.capabilities_authorized, vec![true]);
            assert_eq!(after_capabilities.calls, 0);

            fixture.release();
            let error = client
                .query(
                    LocalOperation::GlobalNews,
                    serde_json::json!({"provider":"Eastmoney","limit":20}),
                )
                .await
                .expect_err("TEST_CODE provider attempts status");
            assert!(matches!(&error, GrpcError::FailedPrecondition { .. }));
            assert_eq!(error.details().provider.as_deref(), Some("Eastmoney"));
            assert_eq!(
                error.details().reason_code.as_deref(),
                Some("invalid_evidence")
            );
            assert_eq!(error.details().retryable, Some(false));
            assert_eq!(
                crate::grpc_client::retry::retry_decision(&error),
                RetryDecision::NoRetry,
            );

            if unpublished_provider {
                assert!(
                    error.details().provider_attempts.accepted().is_none(),
                    "a locally known provider absent from same-endpoint Capabilities must reject the whole trace",
                );
            } else {
                let attempts = error
                    .details()
                    .provider_attempts
                    .accepted()
                    .expect("same-endpoint published providers and closed trace");
                let expected = [
                    (1, "Eastmoney", "rejected", "query_rejected", false, false),
                    (2, "Eastmoney", "failed", "unavailable", true, false),
                    (3, "Eastmoney", "selected", "selected", false, false),
                ];
                assert_eq!(attempts.len(), expected.len());
                for (actual, expected) in attempts.iter().zip(expected) {
                    assert_eq!(
                        (
                            actual.ordinal,
                            actual.provider.as_str(),
                            actual.outcome.as_str(),
                            actual.reason_code.as_str(),
                            actual.retryable,
                            actual.terminal,
                        ),
                        expected,
                    );
                    assert!(
                        actual.provider.is_supported(),
                        "provider requires same-endpoint Capabilities authority",
                    );
                    assert!(
                        actual.outcome.is_supported(),
                        "closed outcome must be fully interpreted",
                    );
                    assert!(
                        actual.reason_code.is_supported(),
                        "outcome-specific closed reason must be fully interpreted",
                    );
                }
            }

            let observed = fixture.snapshot();
            assert_eq!(observed.capabilities_calls, 1);
            assert_eq!(observed.calls, 1);
            assert_eq!(observed.authorized, vec![true]);
            assert_eq!(observed.methods, vec!["global_news"]);
            assert!(observed.unexpected_methods.is_empty());
        }))
        .catch_unwind()
        .await;
    let cleanup = match fixture.take() {
        Some(fixture) => fixture.finish().await,
        None => Ok(()),
    };
    if let Err(error) = cleanup {
        panic!("TEST_CODE provider attempts cleanup failed: {error}");
    }
    match outcome {
        Ok(result) => result.expect("TEST_CODE provider attempts body timeout"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test]
async fn external_capabilities_authorize_complete_attempt_trace_only_for_published_providers() {
    assert_capability_authorized_attempt_trace(false).await;
    assert_capability_authorized_attempt_trace(true).await;
}

#[tokio::test]
async fn grpc_dual_contract_external_global_news_attempt_preserves_zero_length_source11_wire_payload(
) {
    let mut fixture = None;
    let outcome =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(30), async {
            fixture = Some(
                ExternalQueryWireFixture::bind()
                    .await
                    .expect("TEST_CODE External query wire fixture setup"),
            );
            let fixture = fixture
                .as_ref()
                .expect("TEST_CODE External query wire fixture owner");
            assert!(fixture.bundle_path().is_absolute());
            assert!(fixture.endpoint().starts_with("https://127.0.0.1:"));
            let prepared = GrpcMarketClient::prepare_client_bundle(fixture.bundle_path())
                .expect("TEST_CODE production External query wire bundle");
            assert_eq!(prepared.endpoint_uri(), fixture.endpoint());
            let attempt = prepared
                .prepare_macro_query(MacroQueryIdentity::GlobalNews {
                    provider: GlobalNewsProvider::Eastmoney,
                    limit: 20,
                })
                .expect("TEST_CODE production External GlobalNews attempt");
            assert_eq!(attempt.endpoint_uri(), fixture.endpoint());
            assert_eq!(attempt.profile(), ContractProfile::ExternalV1);
            assert_eq!(attempt.acquisition_authority(), TEST_AUTHORITY);
            assert_eq!(attempt.retry_policy(), (4, 1000, 60_000, 200));
            assert_eq!(attempt.attempt_ordinal(), 1);
            let request_id = attempt.request_id().to_owned();
            let request_bytes = attempt.request_bytes();
            let request = ExternalQueryRequest::decode(request_bytes.as_slice())
                .expect("TEST_CODE native External GlobalNews request");
            assert_eq!(request.encode_to_vec(), request_bytes);
            let context = request
                .context
                .as_ref()
                .expect("TEST_CODE native External GlobalNews context");
            assert_eq!(context.protocol_version, 1);
            assert_eq!(context.request_id, request_id);
            assert_eq!(request.preferred_provider, "Eastmoney");
            assert!(!request.allow_unadmitted);
            let payload = request
                .payload
                .as_ref()
                .expect("TEST_CODE native External GlobalNews payload");
            assert_eq!(payload.schema, "magic.market.global_news.request");
            assert_eq!(payload.schema_version, 2);
            assert_eq!(payload.content_type, "application/json; charset=utf-8");
            assert_eq!(payload.data, br#"{"limit":20}"#);
            assert_eq!(fixture.snapshot(), Default::default());

            let execution = attempt.execute();
            tokio::pin!(execution);
            let receipt_deadline = Instant::now() + Duration::from_secs(5);
            loop {
                tokio::select! {
                    biased;
                    _completion = &mut execution => {
                        panic!("TEST_CODE External query wire completed before release");
                    }
                    _ = tokio::task::yield_now() => {}
                }
                if fixture.snapshot().calls > 0 {
                    break;
                }
                assert!(
                    Instant::now() < receipt_deadline,
                    "TEST_CODE External query wire receipt watchdog"
                );
            }
            let received = fixture.snapshot();
            assert_eq!(received.tcp_accepts, 1);
            assert_eq!(received.calls, 1);
            assert_eq!(received.authorized, vec![true]);
            assert_eq!(received.methods, vec!["global_news"]);
            assert_eq!(received.requests, vec![request_bytes]);
            assert!(received.protobuf_payloads.is_empty());
            assert!(received.unexpected_methods.is_empty());

            fixture.release();
            let completion = tokio::time::timeout(Duration::from_secs(5), &mut execution)
                .await
                .expect("TEST_CODE External query wire completion deadline")
                .expect("TEST_CODE External query wire outer result");
            let observed = fixture.snapshot();
            assert_eq!(observed.tcp_accepts, 1);
            assert_eq!(observed.calls, 1);
            assert_eq!(observed.authorized, vec![true]);
            assert_eq!(observed.methods, vec!["global_news"]);
            assert!(observed.unexpected_methods.is_empty());
            assert_eq!(observed.protobuf_payloads.len(), 1);
            let expected_payload = &observed.protobuf_payloads[0];
            assert!(expected_payload.ends_with(&[0x5a, 0x00]));
            let native = ExternalQueryResponse::decode(expected_payload.as_slice())
                .expect("TEST_CODE External generated response accepts unknown source11");
            assert_eq!(native.request_id, request_id);
            assert_eq!(native.operation, Operation::GlobalNews as i32);
            assert_eq!(native.admission, AdmissionState::Admitted as i32);
            assert_eq!(native.selected_provider, "Eastmoney");
            assert_eq!(native.batch_id, TEST_BATCH);
            assert!(native.complete);
            assert_eq!(native.observed_at, TEST_OBSERVED_AT);
            assert_eq!(native.source_at, TEST_SOURCE_AT);
            assert_eq!(native.records.len(), 1);
            assert!(native.diagnostic_blocker.is_empty());
            assert_ne!(
                native.encode_to_vec().as_slice(),
                expected_payload.as_slice()
            );

            let ExternalMacroAttemptCompletion::Unary(inner) = completion else {
                panic!("TEST_CODE expected External query wire unary completion");
            };
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
                .expect("TEST_CODE External query wire successful projection");
            assert_eq!(processed.admission, LocalAdmissionState::Admitted);
            assert_eq!(processed.selected_provider, "Eastmoney");
            assert_eq!(processed.batch_id, TEST_BATCH);
            assert!(processed.complete);
            assert_eq!(processed.observed_at, TEST_OBSERVED_AT);
            assert_eq!(processed.source_at, TEST_SOURCE_AT);
            assert_eq!(processed.records.len(), 1);
            assert_eq!(processed.source(), TEST_AUTHORITY);
            assert!(processed.diagnostic_blocker.is_empty());

            assert_eq!(
                inner.response_bytes.as_deref(),
                Some(expected_payload.as_slice()),
                "decoded-message re-encoding must not replace the received protobuf payload"
            );
        }))
        .catch_unwind()
        .await;
    let cleanup = match fixture.take() {
        Some(fixture) => fixture.finish().await,
        None => Ok(()),
    };
    if let Err(error) = cleanup {
        panic!("TEST_CODE External query wire cleanup failed: {error}");
    }
    match outcome {
        Ok(result) => result.expect("TEST_CODE External query wire body timeout"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test]
async fn external_mtls_listener_status_preserves_replay_subscriber_and_agent_counters() {
    let mut fixture = None;
    let outcome =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(30), async {
            fixture = Some(
                ExternalMtlsMacroFixture::bind_data_success_for_test()
                    .await
                    .expect("TEST_CODE External Listener fixture setup"),
            );
            let fixture = fixture
                .as_ref()
                .expect("TEST_CODE External Listener fixture owner");
            let mut client = GrpcMarketClient::connect_client_bundle(fixture.bundle_path())
                .await
                .expect("TEST_CODE External Listener mTLS client");
            let before_wrong_profile = fixture.snapshot();
            let wrong_profile_error = match tokio::time::timeout(
                Duration::from_secs(1),
                client.get_listener_status(),
            )
            .await
            .expect("TEST_CODE Local Listener wrong-profile deadline")
            {
                Err(error) => error,
                Ok(_) => panic!("External profile must reject Local Listener before RPC"),
            };
            assert!(matches!(&wrong_profile_error, GrpcError::FailedPrecondition { .. }));
            assert_eq!(wrong_profile_error.details().code, "event_profile_mismatch");
            assert_eq!(fixture.snapshot(), before_wrong_profile);

            let external_status = tokio::time::timeout(
                Duration::from_secs(5),
                client.get_external_listener_status(),
            )
            .await
            .expect("TEST_CODE External Listener call deadline")
            .expect("TEST_CODE External Listener call");

            let observed = fixture.snapshot();
            assert_eq!(observed.listener_status_authorized, vec![true]);
            assert_eq!(observed.listener_status_requests.len(), 1);
            assert_eq!(observed.listener_status_responses.len(), 1);
            assert_eq!(observed.health_requests.len(), 0);
            assert_eq!(observed.capabilities_calls, 0);
            assert_eq!(observed.data_calls, 0);
            let request = ExternalListenerStatusRequest::decode(
                observed.listener_status_requests[0].as_slice(),
            )
            .expect("TEST_CODE External generated Listener request");
            assert_eq!(request.encode_to_vec(), observed.listener_status_requests[0]);
            let context = request
                .context
                .expect("TEST_CODE External Listener request context");
            assert_eq!(context.protocol_version, 1);
            assert!(!context.request_id.is_empty());
            assert_eq!(external_status.request_id, context.request_id);
            assert_eq!(external_status.state, "agent_connected_production");
            assert_eq!(
                external_status.terminal_generation,
                "TEST_CODE_EXTERNAL_GENERATION"
            );
            assert_eq!(
                external_status.latest.as_ref().map(|cursor| (
                    cursor.generation.as_str(),
                    cursor.sequence,
                )),
                Some(("TEST_CODE_EXTERNAL_GENERATION", 44)),
            );
            assert!(external_status.capabilities.is_empty());
            assert_eq!(external_status.desired_watchlist_revision, 7);
            assert_eq!(external_status.applied_watchlist_revision, 7);
            assert_eq!(
                external_status.desired_instruments,
                vec!["EQUITY:SH:600396".to_owned()]
            );
            assert_eq!(
                external_status.applied_instruments,
                vec!["EQUITY:SH:600396".to_owned()]
            );
            assert_eq!(external_status.maximum_watchlist_instruments, 128);
            assert_eq!(
                external_status.admitted_event_families,
                vec!["price".to_owned(), "analysis".to_owned()]
            );

            let external_wire = ExternalListenerStatusResponse::decode(
                observed.listener_status_responses[0].as_slice(),
            )
            .expect("TEST_CODE External generated Listener response");
            assert_eq!(external_wire.encode_to_vec(), observed.listener_status_responses[0]);
            assert_eq!(external_status, external_wire);
            assert_eq!(
                (
                    external_status.replay_oldest.as_ref().map(|cursor| cursor.sequence),
                    external_status.replay_event_count,
                    external_status.replay_bytes,
                    external_status.active_subscribers,
                    external_status.agent_connections_total,
                    external_status.agent_disconnects_total,
                    external_status.events_published_total,
                    external_status.replay_evictions_total,
                ),
                (Some(4), 41, 4_096, 3, 11, 2, 44, 1),
                "TEST_CODE External Listener fields 12..19 must survive the public client boundary",
            );
            drop(client);
        }))
        .catch_unwind()
        .await;
    let cleanup = match fixture.take() {
        Some(fixture) => fixture.finish().await,
        None => Ok(()),
    };
    if let Err(error) = cleanup {
        panic!("TEST_CODE External Listener cleanup failed: {error}");
    }
    match outcome {
        Ok(result) => result.expect("TEST_CODE External Listener body timeout"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test]
async fn external_mtls_native_subscribe_and_watchlist_preserve_wire_contract_without_retry() {
    let mut fixture = None;
    let outcome =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(30), async {
            fixture = Some(
                ExternalMtlsMacroFixture::bind_data_success_for_test()
                    .await
                    .expect("TEST_CODE External event fixture setup"),
            );
            let fixture = fixture
                .as_ref()
                .expect("TEST_CODE External event fixture owner");
            let mut client = GrpcMarketClient::connect_client_bundle(fixture.bundle_path())
                .await
                .expect("TEST_CODE External event mTLS client");

            let filter = ExternalEventFilter {
                instruments: vec!["EQUITY:SH:600396".to_owned()],
                event_kinds: vec!["price".to_owned()],
            };
            let after = ExternalEventCursor {
                generation: "TEST_CODE_EXTERNAL_GENERATION".to_owned(),
                sequence: 40,
            };
            let subscribe_result = tokio::time::timeout(
                Duration::from_secs(5),
                client.subscribe_external(filter.clone(), Some(after.clone())),
            )
            .await
            .expect("TEST_CODE External Subscribe call deadline");
            let instruments = vec![
                "EQUITY:SH:600396".to_owned(),
                "EQUITY:SZ:000001".to_owned(),
            ];
            let watchlist_result = tokio::time::timeout(
                Duration::from_secs(5),
                client.set_external_watchlist(instruments.clone()),
            )
            .await
            .expect("TEST_CODE External SetWatchlist call deadline");

            let subscribe_not_migrated = subscribe_result.as_ref().err().is_some_and(|error| {
                matches!(error, GrpcError::Unimplemented { .. })
                    && error.details().code == "external_event_transport_not_migrated"
            });
            let watchlist_not_migrated = watchlist_result.as_ref().err().is_some_and(|error| {
                matches!(error, GrpcError::Unimplemented { .. })
                    && error.details().code == "external_event_transport_not_migrated"
            });
            assert!(
                !subscribe_not_migrated && !watchlist_not_migrated,
                "TEST_CODE External typed event methods remain unsupported: subscribe={subscribe_not_migrated} watchlist={watchlist_not_migrated}"
            );

            let mut stream = subscribe_result.expect("TEST_CODE External Subscribe call");
            let event = tokio::time::timeout(Duration::from_secs(5), stream.message())
                .await
                .expect("TEST_CODE External Subscribe item deadline")
                .expect("TEST_CODE External Subscribe stream status")
                .expect("TEST_CODE External Subscribe first item");
            let end = tokio::time::timeout(Duration::from_secs(5), stream.message())
                .await
                .expect("TEST_CODE External Subscribe end deadline")
                .expect("TEST_CODE External Subscribe final status");
            assert!(end.is_none(), "TEST_CODE fixture stream must close after one item");
            let watchlist = watchlist_result.expect("TEST_CODE External SetWatchlist call");

            let observed = fixture.snapshot();
            assert_eq!(observed.tcp_accepts, 1,
                "TEST_CODE External event client must reuse its single mTLS connection");
            assert_eq!(observed.subscribe_authorized, vec![true]);
            assert_eq!(observed.subscribe_requests.len(), 1);
            assert_eq!(observed.subscribe_events.len(), 1);
            assert_eq!(observed.watchlist_authorized, vec![true]);
            assert_eq!(observed.watchlist_requests.len(), 1);
            assert_eq!(observed.watchlist_responses.len(), 1);
            assert_eq!(observed.listener_status_requests.len(), 0);
            assert!(observed.replay_requests.is_empty());
            assert_eq!(observed.health_requests.len(), 0);
            assert_eq!(observed.capabilities_calls, 0);
            assert_eq!(observed.data_calls, 0);

            let subscribe = ExternalSubscribeRequest::decode(
                observed.subscribe_requests[0].as_slice(),
            )
            .expect("TEST_CODE External generated Subscribe request");
            assert_eq!(subscribe.encode_to_vec(), observed.subscribe_requests[0]);
            let subscribe_context = subscribe
                .context
                .expect("TEST_CODE External Subscribe request context");
            assert_eq!(subscribe_context.protocol_version, 1);
            assert!(!subscribe_context.request_id.is_empty());
            assert_eq!(subscribe.filter, Some(filter));
            assert_eq!(subscribe.after, Some(after));

            let expected_event = ExternalMarketEventEnvelope {
                protocol_version: 1,
                event_id: "TEST_CODE_EXTERNAL_EVENT_41".to_owned(),
                cursor: Some(ExternalEventCursor {
                    generation: "TEST_CODE_EXTERNAL_GENERATION".to_owned(),
                    sequence: 41,
                }),
                event_kind: "price".to_owned(),
                provider: "TDX".to_owned(),
                instrument: "EQUITY:SH:600396".to_owned(),
                observed_at: "2026-09-17T09:31:00+08:00".to_owned(),
                source_at: String::new(),
                admission: AdmissionState::Admitted as i32,
                payload: Some(ExternalCanonicalPayload {
                    schema: "magic.market.event.price".to_owned(),
                    schema_version: 1,
                    content_type: "application/json; charset=utf-8".to_owned(),
                    data: br#"{"instrument":"EQUITY:SH:600396","price":"17.28"}"#.to_vec(),
                }),
            };
            assert_eq!(event, expected_event);
            assert!(event.source_at.is_empty(), "source_at must not be synthesized");
            assert_eq!(event.encode_to_vec(), observed.subscribe_events[0]);

            let watchlist_request = ExternalSetWatchlistRequest::decode(
                observed.watchlist_requests[0].as_slice(),
            )
            .expect("TEST_CODE External generated SetWatchlist request");
            assert_eq!(
                watchlist_request.encode_to_vec(),
                observed.watchlist_requests[0]
            );
            let watchlist_context = watchlist_request
                .context
                .expect("TEST_CODE External SetWatchlist request context");
            assert_eq!(watchlist_context.protocol_version, 1);
            assert!(!watchlist_context.request_id.is_empty());
            assert_ne!(watchlist_context.request_id, subscribe_context.request_id);
            assert_eq!(watchlist_request.instruments, instruments);
            let watchlist_wire = ExternalSetWatchlistResponse::decode(
                observed.watchlist_responses[0].as_slice(),
            )
            .expect("TEST_CODE External generated SetWatchlist response");
            assert_eq!(watchlist_wire.encode_to_vec(), observed.watchlist_responses[0]);
            assert_eq!(watchlist, watchlist_wire);
            assert_eq!(watchlist.request_id, watchlist_context.request_id);
            assert_eq!(watchlist.desired_revision, 8);
            assert_eq!(watchlist.state, "restarting");
            assert_eq!(watchlist.instruments, instruments);
            drop(stream);
            drop(client);
        }))
        .catch_unwind()
        .await;
    let cleanup = match fixture.take() {
        Some(fixture) => fixture.finish().await,
        None => Ok(()),
    };
    if let Err(error) = cleanup {
        panic!("TEST_CODE External event cleanup failed: {error}");
    }
    match outcome {
        Ok(result) => result.expect("TEST_CODE External event body timeout"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test]
async fn external_mtls_set_watchlist_status_decodes_external_detail_without_retry() {
    let mut fixture = None;
    let outcome =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(30), async {
            fixture = Some(
                ExternalMtlsMacroFixture::bind_data_success_for_test()
                    .await
                    .expect("TEST_CODE External SetWatchlist status fixture setup"),
            );
            let fixture = fixture
                .as_ref()
                .expect("TEST_CODE External SetWatchlist status fixture owner");
            fixture.set_watchlist_unavailable_for_test();
            let mut client = GrpcMarketClient::connect_client_bundle(fixture.bundle_path())
                .await
                .expect("TEST_CODE External SetWatchlist status mTLS client");
            let instruments = vec![
                "EQUITY:SH:600396".to_owned(),
                "EQUITY:SZ:000001".to_owned(),
            ];
            let error = tokio::time::timeout(
                Duration::from_secs(5),
                client.set_external_watchlist(instruments.clone()),
            )
            .await
            .expect("TEST_CODE External SetWatchlist status deadline")
            .expect_err("TEST_CODE External SetWatchlist status must reject");

            let observed = fixture.snapshot();
            assert_eq!(observed.watchlist_authorized, vec![true]);
            assert_eq!(observed.watchlist_requests.len(), 1,
                "TEST_CODE mutating SetWatchlist status must not be retried");
            assert!(observed.watchlist_responses.is_empty());
            assert_eq!(observed.watchlist_status_details.len(), 1);
            assert!(observed.subscribe_requests.is_empty());
            assert!(observed.listener_status_requests.is_empty());
            assert!(observed.health_requests.is_empty());
            assert_eq!(observed.capabilities_calls, 0);
            assert_eq!(observed.data_calls, 0);

            let request = ExternalSetWatchlistRequest::decode(
                observed.watchlist_requests[0].as_slice(),
            )
            .expect("TEST_CODE External generated SetWatchlist status request");
            assert_eq!(request.encode_to_vec(), observed.watchlist_requests[0]);
            let context = request
                .context
                .expect("TEST_CODE External SetWatchlist status request context");
            assert_eq!(context.protocol_version, 1);
            assert!(!context.request_id.is_empty());
            assert_eq!(request.instruments, instruments);
            let wire_detail = ExternalErrorDetail::decode(
                observed.watchlist_status_details[0].as_slice(),
            )
            .expect("TEST_CODE External SetWatchlist status detail");
            assert_eq!(wire_detail.encode_to_vec(), observed.watchlist_status_details[0]);
            assert_eq!(wire_detail.request_id, context.request_id);
            assert_eq!(wire_detail.provider, "Tdx");
            assert_eq!(wire_detail.reason_code, "unavailable");
            assert!(wire_detail.retryable);

            assert!(matches!(&error, GrpcError::Unavailable { .. }));
            let correlation = error
                .details()
                .request_id
                .as_deref()
                .expect("TEST_CODE public error must retain a safe request correlation");
            let correlation_digest = correlation
                .strip_prefix("sha256:")
                .expect("TEST_CODE public request correlation prefix");
            assert_eq!(correlation_digest.len(), 64);
            assert!(correlation_digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
            assert_ne!(correlation, context.request_id);
            assert!(!correlation.contains(context.request_id.as_str()));
            assert_eq!(error.details().provider.as_deref(), Some("Tdx"));
            assert_eq!(
                error.details().reason_code.as_deref(),
                Some("unavailable")
            );
            assert_eq!(error.details().retryable, Some(true));
            drop(client);
        }))
        .catch_unwind()
        .await;
    let cleanup = match fixture.take() {
        Some(fixture) => fixture.finish().await,
        None => Ok(()),
    };
    if let Err(error) = cleanup {
        panic!("TEST_CODE External SetWatchlist status cleanup failed: {error}");
    }
    match outcome {
        Ok(result) => result.expect("TEST_CODE External SetWatchlist status body timeout"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}
async fn query_catalog_attempt_status(
    fixture: &ExternalQueryWireFixture,
    client: &mut GrpcMarketClient,
    requested_provider: &str,
) -> GrpcError {
    fixture.release();
    client
        .query(
            LocalOperation::GlobalNews,
            serde_json::json!({"provider": requested_provider, "limit": 20}),
        )
        .await
        .expect_err("TEST_CODE catalog attempt status")
}

fn assert_complete_catalog_attempts(error: &GrpcError, provider: &str, accepted: bool) {
    assert!(matches!(error, GrpcError::FailedPrecondition { .. }));
    assert_eq!(error.details().provider.as_deref(), Some(provider));
    assert_eq!(
        error.details().reason_code.as_deref(),
        Some("invalid_evidence")
    );
    assert_eq!(error.details().retryable, Some(false));
    assert_eq!(
        crate::grpc_client::retry::retry_decision(error),
        RetryDecision::NoRetry,
    );
    if !accepted {
        assert!(error.details().provider_attempts.accepted().is_none());
        return;
    }
    let attempts = error
        .details()
        .provider_attempts
        .accepted()
        .expect("TEST_CODE complete catalog attempt trace");
    let expected = [
        (1, provider, "rejected", "query_rejected", false, false),
        (2, provider, "failed", "unavailable", true, true),
        (3, provider, "selected", "selected", false, false),
    ];
    assert_eq!(attempts.len(), expected.len());
    for (actual, expected) in attempts.iter().zip(expected) {
        assert_eq!(
            (
                actual.ordinal,
                actual.provider.as_str(),
                actual.outcome.as_str(),
                actual.reason_code.as_str(),
                actual.retryable,
                actual.terminal,
            ),
            expected,
        );
        assert!(actual.provider.is_supported());
        assert!(actual.outcome.is_supported());
        assert!(actual.reason_code.is_supported());
    }
}

#[tokio::test]
async fn external_capabilities_catalog_lifecycle_is_atomic_across_refresh_outcomes() {
    let mut fixture = None;
    let outcome =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(45), async {
            fixture = Some(
                ExternalQueryWireFixture::bind_catalog_lifecycle()
                    .await
                    .expect("TEST_CODE catalog lifecycle fixture"),
            );
            let fixture = fixture
                .as_ref()
                .expect("TEST_CODE catalog lifecycle fixture owner");
            let mut client = GrpcMarketClient::connect_client_bundle(fixture.bundle_path())
                .await
                .expect("TEST_CODE catalog lifecycle client");

            fixture.release_capabilities();
            let wrong_initial = client
                .get_external_capabilities()
                .await
                .expect_err("TEST_CODE wrong initial Capabilities ID");
            assert_eq!(
                wrong_initial.details().code,
                "capabilities_request_id_mismatch"
            );
            let no_catalog = query_catalog_attempt_status(fixture, &mut client, "Eastmoney").await;
            assert_complete_catalog_attempts(&no_catalog, "Eastmoney", false);

            fixture.release_capabilities();
            let first = client
                .get_external_capabilities()
                .await
                .expect("TEST_CODE first valid Capabilities catalog");
            assert_eq!(first.len(), 1);
            assert_eq!(first[0].provider, "Eastmoney");
            let first_authorized =
                query_catalog_attempt_status(fixture, &mut client, "Eastmoney").await;
            assert_complete_catalog_attempts(&first_authorized, "Eastmoney", true);

            fixture.release_capabilities();
            let failed_refresh = client
                .get_external_capabilities()
                .await
                .expect_err("TEST_CODE unavailable Capabilities refresh");
            assert!(matches!(&failed_refresh, GrpcError::Unavailable { .. }));
            let after_failure =
                query_catalog_attempt_status(fixture, &mut client, "Eastmoney").await;
            assert_complete_catalog_attempts(&after_failure, "Eastmoney", true);

            fixture.release_capabilities();
            let wrong_refresh = client
                .get_external_capabilities()
                .await
                .expect_err("TEST_CODE wrong refresh Capabilities ID");
            assert_eq!(
                wrong_refresh.details().code,
                "capabilities_request_id_mismatch"
            );
            let after_wrong_id =
                query_catalog_attempt_status(fixture, &mut client, "Eastmoney").await;
            assert_complete_catalog_attempts(&after_wrong_id, "Eastmoney", true);

            fixture.release_capabilities();
            let replacement = client
                .get_external_capabilities()
                .await
                .expect("TEST_CODE replacement Capabilities catalog");
            assert_eq!(replacement.len(), 1);
            assert_eq!(replacement[0].provider, "Cailianpress");
            let replacement_authorized =
                query_catalog_attempt_status(fixture, &mut client, "Cailianpress").await;
            assert_complete_catalog_attempts(&replacement_authorized, "Cailianpress", true);
            let removed = query_catalog_attempt_status(fixture, &mut client, "Eastmoney").await;
            assert_complete_catalog_attempts(&removed, "Eastmoney", false);

            let observed = fixture.snapshot();
            assert_eq!(observed.tcp_accepts, 1);
            assert_eq!(observed.capabilities_calls, 5);
            assert_eq!(observed.capabilities_authorized, vec![true; 5]);
            assert_eq!(observed.capabilities_requests.len(), 5);
            let requests = observed
                .capabilities_requests
                .iter()
                .map(|bytes| {
                    let request = CapabilitiesRequest::decode(bytes.as_slice())
                        .expect("TEST_CODE generated Capabilities request");
                    assert_eq!(request.encode_to_vec(), *bytes);
                    request
                        .context
                        .expect("TEST_CODE Capabilities request context")
                        .request_id
                })
                .collect::<Vec<_>>();
            assert!(requests.iter().all(|request_id| !request_id.is_empty()));
            assert_eq!(
                requests
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len(),
                5
            );

            assert_eq!(observed.capabilities_responses.len(), 4);
            let responses = observed
                .capabilities_responses
                .iter()
                .map(|bytes| {
                    let response = CapabilitiesResponse::decode(bytes.as_slice())
                        .expect("TEST_CODE generated Capabilities response");
                    assert_eq!(response.encode_to_vec(), *bytes);
                    response
                })
                .collect::<Vec<_>>();
            assert_eq!(
                responses[0].request_id,
                "TEST_CODE_WRONG_INITIAL_CAPABILITIES_ID"
            );
            assert_eq!(responses[1].request_id, requests[1]);
            assert_eq!(
                responses[2].request_id,
                "TEST_CODE_WRONG_REFRESH_CAPABILITIES_ID"
            );
            assert_eq!(responses[3].request_id, requests[4]);
            assert_eq!(responses[0].capabilities[0].provider, "Eastmoney");
            assert_eq!(responses[1].capabilities[0].provider, "Eastmoney");
            assert_eq!(responses[2].capabilities[0].provider, "Cailianpress");
            assert_eq!(responses[3].capabilities[0].provider, "Cailianpress");
            assert_eq!(
                observed.capabilities_status_codes,
                vec![tonic::Code::Unavailable as i32]
            );

            assert_eq!(observed.calls, 6);
            assert_eq!(observed.authorized, vec![true; 6]);
            assert_eq!(observed.methods, vec!["global_news"; 6]);
            let expected_providers = [
                "Eastmoney",
                "Eastmoney",
                "Eastmoney",
                "Eastmoney",
                "Cailianpress",
                "Eastmoney",
            ];
            for (bytes, expected_provider) in observed.requests.iter().zip(expected_providers) {
                let request = ExternalQueryRequest::decode(bytes.as_slice())
                    .expect("TEST_CODE generated data request");
                assert_eq!(request.encode_to_vec(), *bytes);
                assert_eq!(request.preferred_provider, expected_provider);
                assert!(!request
                    .context
                    .expect("TEST_CODE data request context")
                    .request_id
                    .is_empty());
            }
            assert!(observed.unexpected_methods.is_empty());
            drop(client);
        }))
        .catch_unwind()
        .await;
    let cleanup = match fixture.take() {
        Some(fixture) => fixture.finish().await,
        None => Ok(()),
    };
    if let Err(error) = cleanup {
        panic!("TEST_CODE catalog lifecycle cleanup failed: {error}");
    }
    match outcome {
        Ok(result) => result.expect("TEST_CODE catalog lifecycle body timeout"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test]
async fn external_capabilities_catalog_is_owned_and_isolated_per_endpoint_client() {
    let mut eastmoney_fixture = None;
    let mut cailianpress_fixture = None;
    let outcome =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(45), async {
            eastmoney_fixture = Some(
                ExternalQueryWireFixture::bind_catalog_endpoint_eastmoney()
                    .await
                    .expect("TEST_CODE Eastmoney endpoint fixture"),
            );
            cailianpress_fixture = Some(
                ExternalQueryWireFixture::bind_catalog_endpoint_cailianpress()
                    .await
                    .expect("TEST_CODE Cailianpress endpoint fixture"),
            );
            let eastmoney = eastmoney_fixture
                .as_ref()
                .expect("TEST_CODE Eastmoney endpoint owner");
            let cailianpress = cailianpress_fixture
                .as_ref()
                .expect("TEST_CODE Cailianpress endpoint owner");
            assert_ne!(eastmoney.endpoint(), cailianpress.endpoint());

            let mut eastmoney_client =
                GrpcMarketClient::connect_client_bundle(eastmoney.bundle_path())
                    .await
                    .expect("TEST_CODE Eastmoney endpoint client");
            let mut cailianpress_client =
                GrpcMarketClient::connect_client_bundle(cailianpress.bundle_path())
                    .await
                    .expect("TEST_CODE Cailianpress endpoint client");

            eastmoney.release_capabilities();
            let eastmoney_capabilities = eastmoney_client
                .get_external_capabilities()
                .await
                .expect("TEST_CODE Eastmoney endpoint Capabilities");
            assert_eq!(eastmoney_capabilities.len(), 1);
            assert_eq!(eastmoney_capabilities[0].provider, "Eastmoney");

            let eastmoney_qualified =
                query_catalog_attempt_status(eastmoney, &mut eastmoney_client, "Eastmoney").await;
            assert_complete_catalog_attempts(&eastmoney_qualified, "Eastmoney", true);

            let zero_catalog_not_borrowed =
                query_catalog_attempt_status(cailianpress, &mut cailianpress_client, "Eastmoney")
                    .await;
            assert_complete_catalog_attempts(&zero_catalog_not_borrowed, "Eastmoney", false);
            let before_own_catalog = cailianpress.snapshot();
            assert_eq!(before_own_catalog.capabilities_calls, 0);
            assert!(before_own_catalog.capabilities_requests.is_empty());
            assert!(before_own_catalog.capabilities_responses.is_empty());
            assert_eq!(before_own_catalog.calls, 1);

            cailianpress.release_capabilities();
            let cailianpress_capabilities = cailianpress_client
                .get_external_capabilities()
                .await
                .expect("TEST_CODE Cailianpress endpoint Capabilities");
            assert_eq!(cailianpress_capabilities.len(), 1);
            assert_eq!(cailianpress_capabilities[0].provider, "Cailianpress");

            let old_provider_still_rejected =
                query_catalog_attempt_status(cailianpress, &mut cailianpress_client, "Eastmoney")
                    .await;
            assert_complete_catalog_attempts(&old_provider_still_rejected, "Eastmoney", false);
            let cailianpress_qualified = query_catalog_attempt_status(
                cailianpress,
                &mut cailianpress_client,
                "Cailianpress",
            )
            .await;
            assert_complete_catalog_attempts(&cailianpress_qualified, "Cailianpress", true);
            let eastmoney_still_qualified =
                query_catalog_attempt_status(eastmoney, &mut eastmoney_client, "Eastmoney").await;
            assert_complete_catalog_attempts(&eastmoney_still_qualified, "Eastmoney", true);

            let eastmoney_observed = eastmoney.snapshot();
            assert_eq!(eastmoney_observed.tcp_accepts, 1);
            assert_eq!(eastmoney_observed.capabilities_calls, 1);
            assert_eq!(eastmoney_observed.capabilities_authorized, vec![true]);
            assert_eq!(eastmoney_observed.capabilities_requests.len(), 1);
            assert_eq!(eastmoney_observed.capabilities_responses.len(), 1);
            assert!(eastmoney_observed.capabilities_status_codes.is_empty());
            assert_eq!(eastmoney_observed.calls, 2);
            assert_eq!(eastmoney_observed.authorized, vec![true, true]);

            let cailianpress_observed = cailianpress.snapshot();
            assert_eq!(cailianpress_observed.tcp_accepts, 1);
            assert_eq!(cailianpress_observed.capabilities_calls, 1);
            assert_eq!(cailianpress_observed.capabilities_authorized, vec![true]);
            assert_eq!(cailianpress_observed.capabilities_requests.len(), 1);
            assert_eq!(cailianpress_observed.capabilities_responses.len(), 1);
            assert!(cailianpress_observed.capabilities_status_codes.is_empty());
            assert_eq!(cailianpress_observed.calls, 3);
            assert_eq!(cailianpress_observed.authorized, vec![true, true, true]);

            for (observed, expected_provider) in [
                (&eastmoney_observed, "Eastmoney"),
                (&cailianpress_observed, "Cailianpress"),
            ] {
                let request =
                    CapabilitiesRequest::decode(observed.capabilities_requests[0].as_slice())
                        .expect("TEST_CODE endpoint Capabilities request");
                assert_eq!(request.encode_to_vec(), observed.capabilities_requests[0]);
                let response =
                    CapabilitiesResponse::decode(observed.capabilities_responses[0].as_slice())
                        .expect("TEST_CODE endpoint Capabilities response");
                assert_eq!(response.encode_to_vec(), observed.capabilities_responses[0]);
                assert_eq!(
                    response.request_id,
                    request
                        .context
                        .expect("TEST_CODE endpoint Capabilities context")
                        .request_id
                );
                assert_eq!(response.capabilities.len(), 1);
                assert_eq!(response.capabilities[0].provider, expected_provider);
            }
            for (observed, expected_providers) in [
                (&eastmoney_observed, &["Eastmoney", "Eastmoney"][..]),
                (
                    &cailianpress_observed,
                    &["Eastmoney", "Eastmoney", "Cailianpress"][..],
                ),
            ] {
                assert_eq!(observed.requests.len(), expected_providers.len());
                for (bytes, expected_provider) in observed.requests.iter().zip(expected_providers) {
                    let request = ExternalQueryRequest::decode(bytes.as_slice())
                        .expect("TEST_CODE endpoint generated data request");
                    assert_eq!(request.encode_to_vec(), *bytes);
                    assert_eq!(request.preferred_provider, *expected_provider);
                    assert!(!request
                        .context
                        .expect("TEST_CODE endpoint data request context")
                        .request_id
                        .is_empty());
                }
            }
            assert!(eastmoney_observed.unexpected_methods.is_empty());
            assert!(cailianpress_observed.unexpected_methods.is_empty());
            drop(eastmoney_client);
            drop(cailianpress_client);
        }))
        .catch_unwind()
        .await;
    let eastmoney_cleanup = match eastmoney_fixture.take() {
        Some(fixture) => fixture.finish().await,
        None => Ok(()),
    };
    let cailianpress_cleanup = match cailianpress_fixture.take() {
        Some(fixture) => fixture.finish().await,
        None => Ok(()),
    };
    if let Err(error) = eastmoney_cleanup {
        panic!("TEST_CODE Eastmoney endpoint cleanup failed: {error}");
    }
    if let Err(error) = cailianpress_cleanup {
        panic!("TEST_CODE Cailianpress endpoint cleanup failed: {error}");
    }
    match outcome {
        Ok(result) => result.expect("TEST_CODE endpoint catalog isolation body timeout"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}
