//! gRPC 服务端库模块 (mock 服务端, 方案 A: handler 委托 data_gateway)。
//! 薄二进制 src/bin/grpc_market_server.rs 只负责读配置 + start()。
//! fixture_mode=true 时 handler 返回 fixture 数据 (离线确定性测试);
//! 生产/手工运行 fixture_mode=false 走真实 data_gateway → magic-* crates。
pub mod delegate;
pub mod events;
pub mod fixture;
pub mod handlers;

use crate::grpc_client::pb::magic::market::v1::{
    system_service_server::{SystemService, SystemServiceServer},
    AdmissionState, CapabilitiesRequest, CapabilitiesResponse, Capability, HealthRequest,
    HealthResponse, ListenerStatusRequest, ListenerStatusResponse,
};
use crate::grpc_contract::ops::implemented_operations;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tonic::{Request, Response, Status};

pub struct ServerState {
    /// 服务端进程代次 (合同 §8: generation 改变 = 连续性重建, 不可跨代拼接)。
    pub generation: String,
    pub sequence: AtomicU64,
    pub shadow_events: bool,
    /// SetWatchlist 状态 (合同 §8): 本地 server 无异步应用流程, desired==applied,
    /// 初始 = STOCK_LIST (ServerConfig.instruments)。上游直连后该语义仍一致。
    pub watchlist: Mutex<Vec<String>>,
    /// 已应用 watchlist 版本 (每次 SetWatchlist 成功 +1)。
    pub watchlist_revision: AtomicU64,
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// fixture 模式: handler 返回 fixture 数据, 不连真实 provider。
    pub fixture_mode: bool,
    /// 事件标 UNADMITTED (影子模式, 测试影子隔离用)。
    pub shadow_events: bool,
    /// 监听端口; 0 = 随机端口 (集成测试用)。
    pub port: u16,
    /// 事件轮询的标的 (空 = 服务端 watchlist)。
    pub instruments: Vec<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            fixture_mode: std::env::var("GRPC_GATEWAY_TEST_FIXTURE").as_deref() == Ok("1"),
            shadow_events: std::env::var("GRPC_EVENTS_SHADOW").as_deref() == Ok("1"),
            port: std::env::var("GRPC_MARKET_PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(18082),
            instruments: std::env::var("STOCK_LIST")
                .map(|s| {
                    s.split(',')
                        .map(|c| c.trim().to_string())
                        .filter(|c| !c.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
        }
    }
}

/// 启动 gRPC 服务端。返回实际绑定地址 (port=0 时随机)、serve task 与事件 hub。
pub async fn start(
    config: ServerConfig,
) -> anyhow::Result<(
    std::net::SocketAddr,
    tokio::task::JoinHandle<Result<(), tonic::transport::Error>>,
    std::sync::Arc<events::EventHub>,
)> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], config.port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let bound = listener.local_addr()?;
    log::info!(
        "[grpc_server] 监听 {bound} (fixture_mode={}, shadow_events={})",
        config.fixture_mode,
        config.shadow_events
    );

    let state = std::sync::Arc::new(ServerState {
        generation: format!("dev-{}", std::process::id()),
        sequence: AtomicU64::new(0),
        shadow_events: config.shadow_events,
        watchlist: Mutex::new(config.instruments.clone()),
        watchlist_revision: AtomicU64::new(1),
    });

    // Task 11: 事件服务注册。fixture 模式 hub 只接受注入, 不启动轮询。
    let event_svc = events::EventService::new(state.clone(), config.fixture_mode);
    let hub = event_svc.hub.clone();

    let handle = tokio::spawn(async move {
        let health_svc = HealthService { state: state.clone() };
        let data_svc = handlers::DataService::new(state.clone(), config.fixture_mode);
        tonic::transport::Server::builder()
            .add_service(SystemServiceServer::new(health_svc))
            .add_service(handlers::market_data_service_server::MarketDataServiceServer::new(data_svc))
            .add_service(events::market_event_service_server::MarketEventServiceServer::new(event_svc))
            // 注: TdxAgentService 未注册 → tonic 对未注册服务返回 UNIMPLEMENTED (合同 §2 不做项)。
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
    });
    Ok((bound, handle, hub))
}

struct HealthService {
    state: std::sync::Arc<ServerState>,
}

#[tonic::async_trait]
impl SystemService for HealthService {
    async fn get_health(&self, _req: Request<HealthRequest>) -> Result<Response<HealthResponse>, Status> {
        Ok(Response::new(HealthResponse {
            request_id: "health".to_string(),
            live: true,
            ready: true,
            state: "RUNNING".to_string(),
        }))
    }

    async fn get_capabilities(&self, _req: Request<CapabilitiesRequest>) -> Result<Response<CapabilitiesResponse>, Status> {
        let capabilities = implemented_operations()
            .into_iter()
            .map(|op| Capability {
                operation: op as i32,
                repository_admission: AdmissionState::Admitted as i32,
                runtime_available: true,
                provider: "tdx-dev".to_string(),
                exact_scope: "watchlist + explicit instruments".to_string(),
                blocker: String::new(),
                // 上游合同字段 7: 本地 server 无诊断 handler, 永远 false。
                diagnostic_available: false,
            })
            .collect();
        Ok(Response::new(CapabilitiesResponse {
            request_id: "capabilities".to_string(),
            capabilities,
        }))
    }
}

// ListenerStatus 占位 (Task 11 实现): 当前只编译, 不注册方法。
#[allow(dead_code)]
pub(crate) fn listener_status_placeholder(
    _req: ListenerStatusRequest,
) -> Result<ListenerStatusResponse, Status> {
    Err(Status::unimplemented("Task 11 实现"))
}
