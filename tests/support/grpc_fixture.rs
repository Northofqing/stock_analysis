//! Test-local gRPC fixture server; never linked into production targets.

mod data;
mod events;
mod handlers;

pub use events::{DetectedEvent, EventHub, EventKind};

use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use stock_analysis::grpc_client::pb::magic::market::v1::{
    system_service_server::{SystemService, SystemServiceServer},
    AdmissionState, CapabilitiesRequest, CapabilitiesResponse, Capability, HealthRequest,
    HealthResponse,
};
use stock_analysis::grpc_contract::ops::implemented_operations;
use tonic::{Request, Response, Status};

pub(super) struct ServerState {
    pub generation: String,
    pub shadow_events: bool,
    pub watchlist: Mutex<Vec<String>>,
    pub watchlist_revision: AtomicU64,
}

pub struct FixtureServerGuard {
    addr: std::net::SocketAddr,
    handle: Option<tokio::task::JoinHandle<Result<(), tonic::transport::Error>>>,
    hub: Arc<EventHub>,
}

impl FixtureServerGuard {
    pub fn addr(&self) -> std::net::SocketAddr {
        self.addr
    }

    pub fn hub(&self) -> Arc<EventHub> {
        self.hub.clone()
    }

    pub async fn stop(mut self) {
        if let Some(handle) = self.handle.take() {
            handle.abort();
            let _ = handle.await;
        }
    }
}

impl Drop for FixtureServerGuard {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

pub async fn start_fixture_server(port: u16) -> anyhow::Result<FixtureServerGuard> {
    let listener =
        tokio::net::TcpListener::bind(std::net::SocketAddr::from(([127, 0, 0, 1], port))).await?;
    let addr = listener.local_addr()?;
    let state = Arc::new(ServerState {
        generation: format!(
            "fixture-{}",
            stock_analysis::grpc_client::envelope::new_request_id()
        ),
        shadow_events: false,
        watchlist: Mutex::new(Vec::new()),
        watchlist_revision: AtomicU64::new(1),
    });
    let event_service = events::EventService::new(state.clone());
    let hub = event_service.hub.clone();
    let handle = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(SystemServiceServer::new(HealthService))
            .add_service(
                handlers::market_data_service_server::MarketDataServiceServer::new(
                    handlers::DataService,
                ),
            )
            .add_service(
                events::market_event_service_server::MarketEventServiceServer::new(event_service),
            )
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
    });
    Ok(FixtureServerGuard {
        addr,
        handle: Some(handle),
        hub,
    })
}

struct HealthService;

#[tonic::async_trait]
impl SystemService for HealthService {
    async fn get_health(
        &self,
        _request: Request<HealthRequest>,
    ) -> Result<Response<HealthResponse>, Status> {
        Ok(Response::new(HealthResponse {
            request_id: "fixture-health".to_owned(),
            live: true,
            ready: true,
            state: "RUNNING".to_owned(),
        }))
    }

    async fn get_capabilities(
        &self,
        _request: Request<CapabilitiesRequest>,
    ) -> Result<Response<CapabilitiesResponse>, Status> {
        let capabilities = implemented_operations()
            .into_iter()
            .map(|operation| Capability {
                operation: operation as i32,
                repository_admission: AdmissionState::Admitted as i32,
                runtime_available: true,
                provider: "fixture".to_owned(),
                exact_scope: "integration-test fixture".to_owned(),
                blocker: String::new(),
                diagnostic_available: false,
            })
            .collect();
        Ok(Response::new(CapabilitiesResponse {
            request_id: "fixture-capabilities".to_owned(),
            capabilities,
        }))
    }
}
