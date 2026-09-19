use super::external_control_attempt::ExternalControlResultMaterial;
use super::{ClientAuthorization, ContractProfile, GrpcMarketClient, PreparedExternalEndpoint};
use crate::grpc_client::external_pb::magic::market::v1::{
    system_service_server::{SystemService, SystemServiceServer},
    AdmissionState, BuildIdentity, CapabilitiesRequest, CapabilitiesResponse, Capability,
    HealthRequest, HealthResponse, Operation as ExternalOperation, RuntimeObservability,
};
use crate::grpc_client::pb::magic::market::v1::Operation as LocalOperation;
use crate::grpc_contract::methods::{ExternalMethod, MethodIdentity, UnknownMethod};
use futures::FutureExt as _;
use prost::Message as _;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio_stream::StreamExt as _;
use tonic::transport::Endpoint;
use tonic::{Request, Response, Status};
use zeroize::Zeroizing;

const TEST_CODE_AUTHORITY: &str = "grpc-mtls:TEST_CODE-native-control.external";
const TEST_CODE_BEARER: &str = "TEST_CODE_NATIVE_CONTROL_TOKEN";
const TEST_CODE_AUTHORIZATION: &str = "Bearer TEST_CODE_NATIVE_CONTROL_TOKEN";
const TEST_CODE_HEALTH_STATE: &str = "TEST_CODE_NATIVE_HEALTH_RUNNING";
const TEST_CODE_GLOBAL_NEWS_PROVIDER: &str = "TEST_CODE_NATIVE_GLOBAL_NEWS_PROVIDER";
const TEST_CODE_AUCTION_PROVIDER: &str = "TEST_CODE_NATIVE_AUCTION_PROVIDER";
const TEST_CODE_AUCTION_SCOPE: &str = "TEST_CODE_CURRENT_AUCTION_OBSERVATIONS_SCOPE";
const TEST_CODE_SERVICE_VERSION: &str = "TEST_CODE_SERVICE_VERSION_2026_09_16";
const TEST_CODE_SOURCE_REVISION: &str = "TEST_CODE_SOURCE_REVISION_NATIVE_CONTROL";
const TEST_CODE_CONTRACT_SHA256: &str =
    "TEST_CODE_CONTRACT_SHA256_0123456789abcdef0123456789abcdef0123456789abcdef";
const TEST_CODE_BINARY_SHA256: &str =
    "TEST_CODE_BINARY_SHA256_fedcba9876543210fedcba9876543210fedcba9876543210";
const TEST_CODE_ENDPOINT_TIMEOUT: Duration = Duration::from_secs(35);
const TEST_CODE_RECEIPT_TIMEOUT: Duration = Duration::from_secs(5);

fn test_code_observability() -> RuntimeObservability {
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

fn test_code_build_identity() -> BuildIdentity {
    BuildIdentity {
        service_version: TEST_CODE_SERVICE_VERSION.to_owned(),
        source_revision: TEST_CODE_SOURCE_REVISION.to_owned(),
        contract_sha256: TEST_CODE_CONTRACT_SHA256.to_owned(),
        binary_sha256: TEST_CODE_BINARY_SHA256.to_owned(),
        identity_error: String::new(),
    }
}

#[derive(Clone, Debug, Default)]
struct NativeControlObservation {
    tcp_accepts: usize,
    request_paths: Vec<String>,
    total_rpc_calls: usize,
    health_calls: usize,
    health_authorized: Vec<bool>,
    health_request_ids: Vec<String>,
    capabilities_calls: usize,
    capabilities_authorized: Vec<bool>,
    capabilities_request_ids: Vec<String>,
}

#[derive(Clone)]
struct NativeExternalSystemService {
    observation: Arc<Mutex<NativeControlObservation>>,
    health_release: Arc<tokio::sync::Semaphore>,
    capabilities_release: Arc<tokio::sync::Semaphore>,
}

#[tonic::async_trait]
impl SystemService for NativeExternalSystemService {
    async fn get_capabilities(
        &self,
        request: Request<CapabilitiesRequest>,
    ) -> Result<Response<CapabilitiesResponse>, Status> {
        let authorized = request
            .metadata()
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            == Some(TEST_CODE_AUTHORIZATION);
        let request = request.into_inner();
        let request_id = request
            .context
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("TEST_CODE missing Capabilities context"))?
            .request_id
            .clone();
        {
            let mut observation = self
                .observation
                .lock()
                .expect("TEST_CODE native Capabilities observation");
            observation.capabilities_calls += 1;
            observation
                .request_paths
                .push("/magic.market.v1.SystemService/GetCapabilities".to_owned());
            observation.capabilities_authorized.push(authorized);
            observation
                .capabilities_request_ids
                .push(request_id.clone());
        }
        if !authorized {
            return Err(Status::unauthenticated(
                "TEST_CODE native Capabilities bearer required",
            ));
        }
        let permit = self
            .capabilities_release
            .acquire()
            .await
            .map_err(|_| Status::cancelled("TEST_CODE native fixture closing"))?;
        permit.forget();
        Ok(Response::new(CapabilitiesResponse {
            request_id,
            capabilities: vec![
                Capability {
                    operation: ExternalOperation::GlobalNews as i32,
                    repository_admission: AdmissionState::Admitted as i32,
                    runtime_available: true,
                    provider: TEST_CODE_GLOBAL_NEWS_PROVIDER.to_owned(),
                    exact_scope: "TEST_CODE_GLOBAL_NEWS_SCOPE".to_owned(),
                    blocker: String::new(),
                    diagnostic_available: true,
                },
                Capability {
                    operation: ExternalOperation::CurrentAuctionObservations as i32,
                    repository_admission: AdmissionState::Admitted as i32,
                    runtime_available: true,
                    provider: TEST_CODE_AUCTION_PROVIDER.to_owned(),
                    exact_scope: TEST_CODE_AUCTION_SCOPE.to_owned(),
                    blocker: String::new(),
                    diagnostic_available: false,
                },
            ],
        }))
    }

    async fn get_health(
        &self,
        request: Request<HealthRequest>,
    ) -> Result<Response<HealthResponse>, Status> {
        let authorized = request
            .metadata()
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            == Some(TEST_CODE_AUTHORIZATION);
        let request = request.into_inner();
        let request_id = request
            .context
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("TEST_CODE missing Health context"))?
            .request_id
            .clone();
        {
            let mut observation = self
                .observation
                .lock()
                .expect("TEST_CODE native Health observation");
            observation.health_calls += 1;
            observation
                .request_paths
                .push("/magic.market.v1.SystemService/GetHealth".to_owned());
            observation.health_authorized.push(authorized);
            observation.health_request_ids.push(request_id.clone());
        }
        if !authorized {
            return Err(Status::unauthenticated(
                "TEST_CODE native Health bearer required",
            ));
        }
        let permit = self
            .health_release
            .acquire()
            .await
            .map_err(|_| Status::cancelled("TEST_CODE native fixture closing"))?;
        permit.forget();
        Ok(Response::new(HealthResponse {
            request_id,
            live: true,
            ready: true,
            state: TEST_CODE_HEALTH_STATE.to_owned(),
            observability: Some(test_code_observability()),
            build_identity: Some(test_code_build_identity()),
        }))
    }
}

struct NativeExternalSystemServer {
    endpoint: String,
    observation: Arc<Mutex<NativeControlObservation>>,
    health_release: Arc<tokio::sync::Semaphore>,
    capabilities_release: Arc<tokio::sync::Semaphore>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<Result<(), tonic::transport::Error>>>,
}

impl NativeExternalSystemServer {
    async fn bind() -> Self {
        let listener = tokio::time::timeout(
            TEST_CODE_RECEIPT_TIMEOUT,
            tokio::net::TcpListener::bind("127.0.0.1:0"),
        )
        .await
        .expect("TEST_CODE native fixture bind deadline")
        .expect("TEST_CODE native fixture bind");
        let endpoint = format!(
            "http://{}",
            listener
                .local_addr()
                .expect("TEST_CODE native fixture address")
        );
        let observation = Arc::new(Mutex::new(NativeControlObservation::default()));
        let health_release = Arc::new(tokio::sync::Semaphore::new(0));
        let capabilities_release = Arc::new(tokio::sync::Semaphore::new(0));
        let service = NativeExternalSystemService {
            observation: Arc::clone(&observation),
            health_release: Arc::clone(&health_release),
            capabilities_release: Arc::clone(&capabilities_release),
        };
        let accept_observation = Arc::clone(&observation);
        let incoming =
            tokio_stream::wrappers::TcpListenerStream::new(listener).map(move |connection| {
                if connection.is_ok() {
                    accept_observation
                        .lock()
                        .expect("TEST_CODE native fixture TCP observation")
                        .tcp_accepts += 1;
                }
                connection
            });
        let inbound_observation = Arc::clone(&observation);
        let (shutdown, receive) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .layer(tonic::service::InterceptorLayer::new(
                    move |request: tonic::Request<()>| {
                        inbound_observation
                            .lock()
                            .expect("TEST_CODE native fixture inbound observation")
                            .total_rpc_calls += 1;
                        Ok(request)
                    },
                ))
                .add_service(SystemServiceServer::new(service))
                .serve_with_incoming_shutdown(incoming, async move {
                    let _ = receive.await;
                })
                .await
        });
        Self {
            endpoint,
            observation,
            health_release,
            capabilities_release,
            shutdown: Some(shutdown),
            task: Some(task),
        }
    }

    fn endpoint(&self) -> &str {
        &self.endpoint
    }

    fn snapshot(&self) -> NativeControlObservation {
        self.observation
            .lock()
            .expect("TEST_CODE native fixture snapshot")
            .clone()
    }

    fn release_health(&self) {
        self.health_release.add_permits(1);
    }

    fn release_capabilities(&self) {
        self.capabilities_release.add_permits(1);
    }

    async fn finish(mut self) -> Result<(), String> {
        self.health_release.close();
        self.capabilities_release.close();
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(mut task) = self.task.take() {
            match tokio::time::timeout(TEST_CODE_RECEIPT_TIMEOUT, &mut task).await {
                Ok(Ok(Ok(()))) => {}
                Ok(result) => {
                    return Err(format!("TEST_CODE native fixture failed: {result:?}"));
                }
                Err(_) => {
                    task.abort();
                    let _ = task.await;
                    return Err(
                        "TEST_CODE native fixture shutdown timeout; aborted and joined".to_owned(),
                    );
                }
            }
        }
        Ok(())
    }
}

async fn wait_for_call<F>(
    completion: &mut std::pin::Pin<&mut F>,
    server: &NativeExternalSystemServer,
    calls: impl Fn(&NativeControlObservation) -> usize,
    label: &str,
) where
    F: std::future::Future,
{
    let deadline = Instant::now() + TEST_CODE_RECEIPT_TIMEOUT;
    loop {
        tokio::select! {
            biased;
            _ = completion.as_mut() => {
                panic!("TEST_CODE native {label} completed before fixture release");
            }
            _ = tokio::task::yield_now() => {}
        }
        if calls(&server.snapshot()) == 1 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "TEST_CODE native {label} receipt watchdog"
        );
    }
}

#[tokio::test]
async fn grpc_dual_contract_external_native_controls_preserve_health_identity_and_capability_61() {
    let mut server = None;
    let outcome =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(20), async {
            server = Some(NativeExternalSystemServer::bind().await);
            let server_ref = server.as_ref().expect("TEST_CODE native fixture owner");
            let endpoint_uri = server_ref.endpoint().to_owned();
            let endpoint = Endpoint::from_shared(endpoint_uri.clone())
                .expect("TEST_CODE native endpoint")
                .timeout(TEST_CODE_ENDPOINT_TIMEOUT);
            let prepared = PreparedExternalEndpoint::from_plaintext_for_test(
                endpoint,
                endpoint_uri,
                Zeroizing::new(TEST_CODE_BEARER.to_owned()),
                TEST_CODE_AUTHORITY.to_owned(),
            );

            let health = prepared
                .prepare_health_attempt()
                .expect("TEST_CODE native Health preparation");
            let health_request_id = health.request_id().to_owned();
            let decoded_health_request = HealthRequest::decode(health.request_bytes().as_slice())
                .expect("TEST_CODE native Health request decode");
            let health_context = decoded_health_request
                .context
                .expect("TEST_CODE native Health request context");
            assert_eq!(health_context.protocol_version, 1);
            assert_eq!(health_context.request_id, health_request_id);

            let health_execution = health.execute();
            tokio::pin!(health_execution);
            wait_for_call(
                &mut health_execution,
                server_ref,
                |observation| observation.health_calls,
                "Health",
            )
            .await;
            server_ref.release_health();
            let health_completion =
                tokio::time::timeout(TEST_CODE_RECEIPT_TIMEOUT, &mut health_execution)
                    .await
                    .expect("TEST_CODE native Health completion deadline");
            health_completion
                .processed()
                .expect("TEST_CODE native Health processed response");
            let health_response_bytes = match health_completion.result_material() {
                ExternalControlResultMaterial::Response { bytes, .. } => bytes.to_vec(),
                _ => panic!("TEST_CODE expected native Health response material"),
            };
            let connected = health_completion
                .into_connected_client()
                .expect("TEST_CODE native Health returned connection");

            let capabilities = prepared
                .prepare_capabilities_attempt()
                .expect("TEST_CODE native Capabilities preparation")
                .bind_connected(connected)
                .expect("TEST_CODE native Capabilities bind returned connection");
            let capabilities_request_id = capabilities.request_id().to_owned();
            let decoded_capabilities_request =
                CapabilitiesRequest::decode(capabilities.request_bytes().as_slice())
                    .expect("TEST_CODE native Capabilities request decode");
            let capabilities_context = decoded_capabilities_request
                .context
                .expect("TEST_CODE native Capabilities request context");
            assert_eq!(capabilities_context.protocol_version, 1);
            assert_eq!(capabilities_context.request_id, capabilities_request_id);

            let capabilities_execution = capabilities.execute();
            tokio::pin!(capabilities_execution);
            wait_for_call(
                &mut capabilities_execution,
                server_ref,
                |observation| observation.capabilities_calls,
                "Capabilities",
            )
            .await;
            server_ref.release_capabilities();
            let capabilities_completion =
                tokio::time::timeout(TEST_CODE_RECEIPT_TIMEOUT, &mut capabilities_execution)
                    .await
                    .expect("TEST_CODE native Capabilities completion deadline");
            capabilities_completion
                .processed()
                .expect("TEST_CODE native Capabilities processed response");
            let capabilities_response_bytes = match capabilities_completion.result_material() {
                ExternalControlResultMaterial::Response { bytes, .. } => bytes.to_vec(),
                _ => panic!("TEST_CODE expected native Capabilities response material"),
            };
            drop(
                capabilities_completion
                    .into_connected_client()
                    .expect("TEST_CODE native Capabilities returned connection"),
            );

            let decoded_health = HealthResponse::decode(health_response_bytes.as_slice())
                .expect("TEST_CODE External Health completion decode");
            assert_eq!(decoded_health.request_id, health_request_id);
            assert!(decoded_health.live);
            assert!(decoded_health.ready);
            assert_eq!(decoded_health.state, TEST_CODE_HEALTH_STATE);

            let decoded_capabilities =
                CapabilitiesResponse::decode(capabilities_response_bytes.as_slice())
                    .expect("TEST_CODE External Capabilities completion decode");
            assert_eq!(decoded_capabilities.request_id, capabilities_request_id);
            let auction = decoded_capabilities
                .capabilities
                .iter()
                .find_map(|capability| {
                    let method = ExternalMethod::try_from_raw(capability.operation).ok()?;
                    (method.as_str_name() == "OPERATION_CURRENT_AUCTION_OBSERVATIONS")
                        .then_some((method, capability))
                })
                .expect("TEST_CODE typed External CurrentAuctionObservations capability");
            assert_eq!(
                auction.0.as_str_name(),
                ExternalOperation::CurrentAuctionObservations.as_str_name()
            );
            assert_eq!(
                auction.1.repository_admission,
                AdmissionState::Admitted as i32
            );
            assert!(auction.1.runtime_available);
            assert_eq!(auction.1.provider, TEST_CODE_AUCTION_PROVIDER);
            assert_eq!(auction.1.exact_scope, TEST_CODE_AUCTION_SCOPE);
            assert_eq!(
                MethodIdentity::from_client_operation(
                    ContractProfile::ExternalV1,
                    LocalOperation::ChainBatch,
                ),
                Err(UnknownMethod),
                "TEST_CODE discovered External 61 must not authorize Local ChainBatch sending"
            );

            let observation = server_ref.snapshot();
            assert_eq!(observation.tcp_accepts, 1);
            assert_eq!(observation.health_calls, 1);
            assert_eq!(observation.health_authorized, vec![true]);
            assert_eq!(observation.health_request_ids, vec![health_request_id]);
            assert_eq!(observation.capabilities_calls, 1);
            assert_eq!(observation.capabilities_authorized, vec![true]);
            assert_eq!(
                observation.capabilities_request_ids,
                vec![capabilities_request_id]
            );
            assert_eq!(
                observation.request_paths,
                vec![
                    "/magic.market.v1.SystemService/GetHealth".to_owned(),
                    "/magic.market.v1.SystemService/GetCapabilities".to_owned(),
                ]
            );
            assert_eq!(observation.total_rpc_calls, 2);
            let non_control_calls = observation
                .total_rpc_calls
                .saturating_sub(observation.health_calls + observation.capabilities_calls);
            assert_eq!(non_control_calls, 0, "TEST_CODE native data RPC count");

            let expected_observability = test_code_observability();
            let expected_build_identity = test_code_build_identity();
            assert_eq!(
                (
                    decoded_health.observability.as_ref(),
                    decoded_health.build_identity.as_ref(),
                ),
                (
                    Some(&expected_observability),
                    Some(&expected_build_identity),
                ),
                "TEST_CODE External Health completion must preserve observability/build_identity"
            );
            drop(prepared);
        }))
        .catch_unwind()
        .await;

    let cleanup = match server.take() {
        Some(server) => server.finish().await,
        None => Ok(()),
    };
    if let Err(error) = cleanup {
        panic!("TEST_CODE native control cleanup failed: {error}");
    }
    match outcome {
        Ok(result) => result.expect("TEST_CODE native control body deadline"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

async fn assert_profile_rejection_has_no_tcp_accept(
    listener: &tokio::net::TcpListener,
    error: crate::grpc_client::errors::GrpcError,
) {
    assert!(matches!(
        error,
        crate::grpc_client::errors::GrpcError::FailedPrecondition { .. }
    ));
    assert_eq!(error.details().code, "system_control_profile_mismatch");
    assert_eq!(
        error.details().reason_code.as_deref(),
        Some("system_control_profile_mismatch")
    );
    assert_eq!(error.details().retryable, Some(false));
    assert!(
        tokio::time::timeout(Duration::from_millis(50), listener.accept())
            .await
            .is_err(),
        "TEST_CODE profile rejection must happen before TCP accept"
    );
}

#[tokio::test]
async fn grpc_dual_contract_local_control_entry_rejects_external_profile_before_io() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("TEST_CODE profile refusal listener");
    let endpoint = format!(
        "http://{}",
        listener
            .local_addr()
            .expect("TEST_CODE profile refusal address")
    );
    let channel = Endpoint::from_shared(endpoint)
        .expect("TEST_CODE profile refusal endpoint")
        .connect_lazy();
    let mut client = GrpcMarketClient::from_channel(
        channel,
        ContractProfile::ExternalV1,
        ClientAuthorization::InstanceBearer(Zeroizing::new(TEST_CODE_BEARER.to_owned())),
        Some(TEST_CODE_AUTHORITY.to_owned()),
    );
    let error = client
        .get_health()
        .await
        .expect_err("TEST_CODE Local Health must reject External profile");
    assert_profile_rejection_has_no_tcp_accept(&listener, error).await;
}

#[tokio::test]
async fn grpc_dual_contract_external_control_entry_rejects_local_profile_before_io() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("TEST_CODE reverse profile refusal listener");
    let endpoint = format!(
        "http://{}",
        listener
            .local_addr()
            .expect("TEST_CODE reverse profile refusal address")
    );
    let channel = Endpoint::from_shared(endpoint)
        .expect("TEST_CODE reverse profile refusal endpoint")
        .connect_lazy();
    let mut client = GrpcMarketClient::from_channel(
        channel,
        ContractProfile::LocalBridgeV1,
        ClientAuthorization::Environment,
        None,
    );
    let error = client
        .get_external_health()
        .await
        .expect_err("TEST_CODE External Health must reject Local profile");
    assert_profile_rejection_has_no_tcp_accept(&listener, error).await;
}
