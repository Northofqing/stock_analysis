//! GrpcMarketClient: 24 个已实现 op 的 gRPC 查询客户端 (合同 §5-§7)。
//! 启动后应先调 GetCapabilities (合同 §7: RPC 存在 ≠ 能力准入); 未实现 op 在客户端拦截, 不发起调用。
use crate::grpc_client::auth::{attach_bearer, attach_bearer_value};
use crate::grpc_client::bundle::ClientBundleConfig;
use crate::grpc_client::envelope::{build_query_request, parse_query_response, QueryResult};
use crate::grpc_client::errors::{ErrorDetail, GrpcError, StatusErrorContext};
use crate::grpc_client::external_pb::magic::market::v1::{
    market_event_service_client::MarketEventServiceClient as ExternalMarketEventServiceClient,
    system_service_client::SystemServiceClient as ExternalSystemServiceClient,
    CapabilitiesRequest as ExternalCapabilitiesRequest,
    CapabilitiesResponse as ExternalCapabilitiesResponse, HealthRequest as ExternalHealthRequest,
    HealthResponse as ExternalHealthResponse,
    EventCursor as ExternalEventCursor, EventFilter as ExternalEventFilter,
    ListenerStatusRequest as ExternalListenerStatusRequest,
    ListenerStatusResponse as ExternalListenerStatusResponse,
    MarketEventEnvelope as ExternalMarketEventEnvelope,
    RequestContext as ExternalRequestContext,
    SetWatchlistRequest as ExternalSetWatchlistRequest,
    SetWatchlistResponse as ExternalSetWatchlistResponse,
    SubscribeRequest as ExternalSubscribeRequest,
};
use crate::grpc_client::external_query_transport::{
    admit_external_payload, ExternalQueryCall, ExternalQueryMethod, ExternalQueryTransport,
};
use crate::grpc_client::pb::magic::market::v1::{
    market_data_service_client::MarketDataServiceClient,
    market_event_service_client::MarketEventServiceClient as LocalMarketEventServiceClient,
    system_service_client::SystemServiceClient as LocalSystemServiceClient, CapabilitiesRequest,
    EventCursor, EventFilter, HealthRequest, ListenerStatusRequest, Operation, SetWatchlistRequest,
    SubscribeRequest,
};
use crate::grpc_client::provider_attempts::ExternalProviderCatalog;
use crate::grpc_client::retry::{retry_decision, RetryDecision, RetryPolicy};
use std::path::Path;
use std::time::Duration;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Identity};
use zeroize::Zeroizing;

pub use crate::grpc_contract::methods::ContractProfile;

#[path = "board_attempt.rs"]
pub(crate) mod board_attempt;
#[cfg(test)]
#[path = "board_loopback_fixture.rs"]
pub(crate) mod board_loopback_fixture;
#[cfg(test)]
#[path = "dragon_tiger_attempt_tests.rs"]
mod dragon_tiger_attempt_tests;
#[path = "external_control_attempt.rs"]
pub(crate) mod external_control_attempt;
#[cfg(test)]
#[path = "external_control_attempt_tests.rs"]
mod external_control_attempt_tests;
#[cfg(test)]
#[path = "external_control_loopback_fixture.rs"]
pub(crate) mod external_control_loopback_fixture;
#[cfg(test)]
#[path = "external_query_wire_fixture.rs"]
pub(crate) mod external_query_wire_fixture;
#[cfg(test)]
#[path = "external_mtls_attempt_tests.rs"]
mod external_mtls_attempt_tests;
#[cfg(test)]
#[path = "external_native_control_tests.rs"]
mod external_native_control_tests;
#[path = "macro_attempt.rs"]
pub(crate) mod macro_attempt;
#[cfg(test)]
#[path = "macro_attempt_tests.rs"]
mod macro_attempt_tests;
#[cfg(test)]
#[path = "macro_full_loopback_fixture.rs"]
pub(crate) mod macro_full_loopback_fixture;
#[cfg(test)]
#[path = "macro_loopback_fixture.rs"]
pub(crate) mod macro_loopback_fixture;
#[path = "unary_attempt.rs"]
mod unary_attempt;

#[derive(Clone)]
enum ClientAuthorization {
    Environment,
    InstanceBearer(Zeroizing<String>),
}

#[derive(Clone)]
pub(crate) struct PreparedExternalEndpoint {
    endpoint: tonic::transport::Endpoint,
    endpoint_uri: String,
    authorization: ClientAuthorization,
    acquisition_authority: String,
}

#[derive(Clone)]
enum SystemTransport {
    Local(LocalSystemServiceClient<Channel>),
    External(ExternalSystemServiceClient<Channel>),
}

#[derive(Clone)]
enum DataTransport {
    Local(MarketDataServiceClient<Channel>),
    External(ExternalQueryTransport),
}

#[derive(Clone)]
enum EventTransport {
    Local(LocalMarketEventServiceClient<Channel>),
    External(ExternalMarketEventServiceClient<Channel>),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ProfileQueryRequest {
    Local(crate::grpc_client::pb::magic::market::v1::QueryRequest),
    External(crate::grpc_client::external_pb::magic::market::v1::QueryRequest),
}

impl ProfileQueryRequest {
    pub(crate) fn request_id(&self) -> &str {
        match self {
            Self::Local(request) => request
                .context
                .as_ref()
                .map(|context| context.request_id.as_str())
                .unwrap_or_default(),
            Self::External(request) => request
                .context
                .as_ref()
                .map(|context| context.request_id.as_str())
                .unwrap_or_default(),
        }
    }

    pub(crate) fn encode_to_vec(&self) -> Vec<u8> {
        use prost::Message as _;
        match self {
            Self::Local(request) => request.encode_to_vec(),
            Self::External(request) => request.encode_to_vec(),
        }
    }
}

pub(crate) enum DataCallAuthorized {
    Local(Result<crate::grpc_client::pb::magic::market::v1::QueryResponse, tonic::Status>),
    External(ExternalQueryCall),
    Rejected(GrpcError),
}

pub(crate) enum ProfileAuthorizedRequest {
    Local(tonic::Request<crate::grpc_client::pb::magic::market::v1::QueryRequest>),
    External(tonic::Request<crate::grpc_client::external_pb::magic::market::v1::QueryRequest>),
}

enum ExternalSystemCall<T> {
    Response(T),
    UnaryStatus(tonic::Status),
}

#[derive(Clone)]
pub struct GrpcMarketClient {
    data: DataTransport,
    system: SystemTransport,
    events: EventTransport,
    retry: RetryPolicy,
    profile: ContractProfile,
    authorization: ClientAuthorization,
    acquisition_authority: Option<String>,
    endpoint_uri: Option<String>,
    external_provider_catalog: Option<ExternalProviderCatalog>,
}

impl GrpcMarketClient {
    pub(crate) fn macro_query(
        &self,
        identity: macro_attempt::MacroQueryIdentity,
    ) -> Result<macro_attempt::MacroQuerySession, GrpcError> {
        macro_attempt::MacroQuerySession::new(self.clone(), identity)
    }

    pub(crate) fn resume_macro_query(
        &self,
        identity: macro_attempt::MacroQueryIdentity,
        restored: macro_attempt::RestoredMacroRequest,
    ) -> Result<macro_attempt::MacroQuerySession, GrpcError> {
        macro_attempt::MacroQuerySession::resume(self.clone(), identity, restored)
    }

    pub(crate) fn board_directory_query(
        &self,
        payload: serde_json::Value,
    ) -> Result<board_attempt::BoardQuerySession, GrpcError> {
        board_attempt::BoardQuerySession::new(self.clone(), payload)
    }

    pub(crate) fn resume_board_directory_query(
        &self,
        request: crate::grpc_client::pb::magic::market::v1::QueryRequest,
        profile: ContractProfile,
        acquisition_authority: Option<&str>,
        retry_policy: (u32, u64, u64, u64),
        next_attempt: u32,
    ) -> Result<board_attempt::BoardQuerySession, GrpcError> {
        board_attempt::BoardQuerySession::resume(
            self.clone(),
            request,
            profile,
            acquisition_authority,
            retry_policy,
            next_attempt,
        )
    }

    pub(crate) fn board_memberships_query(
        &self,
        code: String,
    ) -> Result<board_attempt::BoardQuerySession, GrpcError> {
        board_attempt::BoardQuerySession::new_memberships(self.clone(), code)
    }

    pub(crate) fn resume_board_memberships_query(
        &self,
        code: String,
        expected_request_id: &str,
        request: crate::grpc_client::pb::magic::market::v1::QueryRequest,
        profile: ContractProfile,
        acquisition_authority: Option<&str>,
        retry_policy: (u32, u64, u64, u64),
        next_attempt: u32,
    ) -> Result<board_attempt::BoardQuerySession, GrpcError> {
        board_attempt::BoardQuerySession::resume_memberships(
            self.clone(),
            code,
            expected_request_id,
            request,
            profile,
            acquisition_authority,
            retry_policy,
            next_attempt,
        )
    }

    pub(crate) fn dragon_tiger_query(
        &self,
        date: String,
        disclosure_limit: u32,
        stock_limit: usize,
    ) -> Result<board_attempt::BoardQuerySession, GrpcError> {
        board_attempt::BoardQuerySession::new_dragon_tiger(
            self.clone(),
            date,
            disclosure_limit,
            stock_limit,
        )
    }

    pub(crate) fn resume_dragon_tiger_query(
        &self,
        date: &str,
        disclosure_limit: u32,
        stock_limit: usize,
        expected_request_id: &str,
        request: crate::grpc_client::pb::magic::market::v1::QueryRequest,
        profile: ContractProfile,
        acquisition_authority: Option<&str>,
        retry_policy: (u32, u64, u64, u64),
        next_attempt: u32,
    ) -> Result<board_attempt::BoardQuerySession, GrpcError> {
        board_attempt::BoardQuerySession::resume_dragon_tiger(
            self.clone(),
            date,
            disclosure_limit,
            stock_limit,
            expected_request_id,
            request,
            profile,
            acquisition_authority,
            retry_policy,
            next_attempt,
        )
    }

    pub async fn connect(addr: &str) -> Result<Self, GrpcError> {
        let channel = Channel::from_shared(addr.to_string())
            // D2: 本地构造错误无服务端 status → details 全默认 (桥只看码 + 远端 detail)。
            .map_err(|_| GrpcError::InvalidArgument {
                details: Box::default(),
            })?
            // 合同 §12: 为 unary 和 stream 分别设置 deadline/keepalive。
            // BR-243 follow-up: BoardConstituents 每次从 TDX 服务器下载完整板块
            // 文件 (实测 13-15s, 无本地缓存) — 15s deadline 边缘超时导致
            // position-chain/做T 证据批 CANCELLED。放宽到 35s 容纳下载耗时;
            // 慢查询仍受桥侧 GRPC_BRIDGE_SYNC_TIMEOUT (35s) 兜底, fail-closed 语义不变。
            .timeout(Duration::from_secs(35))
            .connect()
            .await
            .map_err(|_| GrpcError::Unavailable {
                details: Box::default(),
            })?;
        Ok(Self::from_channel(
            channel,
            ContractProfile::LocalBridgeV1,
            ClientAuthorization::Environment,
            None,
        ))
    }

    /// Loads a validated client bundle and opens an ExternalV1 connection using
    /// its private mTLS identity and instance-owned bearer credential.
    pub async fn connect_client_bundle(path: &Path) -> Result<Self, GrpcError> {
        Self::prepare_client_bundle(path)?.connect_once().await
    }

    pub(crate) fn prepare_client_bundle(
        path: &Path,
    ) -> Result<PreparedExternalEndpoint, GrpcError> {
        let ClientBundleConfig {
            endpoint_uri,
            tls_server_name,
            ca_pem,
            certificate_pem,
            private_key_pem,
            bearer_token,
        } = crate::grpc_client::bundle::load(path).map_err(|_| GrpcError::InvalidArgument {
            details: Box::default(),
        })?;

        let acquisition_authority = format!("grpc-mtls:{tls_server_name}");
        let tls = ClientTlsConfig::new()
            .domain_name(tls_server_name)
            .ca_certificate(Certificate::from_pem(ca_pem))
            .identity(Identity::from_pem(
                certificate_pem,
                private_key_pem.as_slice(),
            ));
        let endpoint = Channel::from_shared(endpoint_uri.clone())
            .map_err(|_| GrpcError::InvalidArgument {
                details: Box::default(),
            })?
            // BR-243 follow-up: 与本地桥同 deadline (BoardConstituents 板块文件下载 13-15s)。
            .timeout(Duration::from_secs(35))
            .tls_config(tls)
            .map_err(|_| GrpcError::InvalidArgument {
                details: Box::default(),
            })?;

        Ok(PreparedExternalEndpoint {
            endpoint,
            endpoint_uri,
            authorization: ClientAuthorization::InstanceBearer(bearer_token),
            acquisition_authority,
        })
    }

    fn from_channel(
        channel: Channel,
        profile: ContractProfile,
        authorization: ClientAuthorization,
        acquisition_authority: Option<String>,
    ) -> Self {
        let system = match profile {
            ContractProfile::LocalBridgeV1 => {
                SystemTransport::Local(LocalSystemServiceClient::new(channel.clone()))
            }
            ContractProfile::ExternalV1 => {
                SystemTransport::External(ExternalSystemServiceClient::new(channel.clone()))
            }
        };
        let data = match profile {
            ContractProfile::LocalBridgeV1 => {
                DataTransport::Local(MarketDataServiceClient::new(channel.clone()))
            }
            ContractProfile::ExternalV1 => {
                DataTransport::External(ExternalQueryTransport::new(channel.clone()))
            }
        };
        let events = match profile {
            ContractProfile::LocalBridgeV1 => {
                EventTransport::Local(LocalMarketEventServiceClient::new(channel))
            }
            ContractProfile::ExternalV1 => {
                EventTransport::External(ExternalMarketEventServiceClient::new(channel))
            }
        };
        Self {
            data,
            system,
            events,
            retry: RetryPolicy::default(),
            profile,
            authorization,
            acquisition_authority,
            endpoint_uri: None,
            external_provider_catalog: None,
        }
    }

    pub(super) fn accept_external_capabilities(
        &mut self,
        request_id: &str,
        response: &ExternalCapabilitiesResponse,
    ) -> Result<(), GrpcError> {
        let catalog = external_control_attempt::validated_external_provider_catalog(
            request_id,
            response,
        )?;
        self.external_provider_catalog = Some(catalog);
        Ok(())
    }

    pub(super) fn data_status_context<'a>(
        &'a self,
        method: crate::grpc_contract::methods::MethodIdentity,
        request_id: &'a str,
    ) -> StatusErrorContext<'a> {
        match (method, self.external_provider_catalog.as_ref()) {
            (
                crate::grpc_contract::methods::MethodIdentity::External(method),
                Some(catalog),
            ) => StatusErrorContext::external_data(method, request_id, catalog),
            _ => StatusErrorContext::data(method, request_id),
        }
    }

    fn attach_request_auth<T>(&self, request: &mut tonic::Request<T>) -> Result<(), GrpcError> {
        attach_authorization(&self.authorization, request)
    }

    fn build_profile_query_request(
        &self,
        operation: Operation,
        payload: serde_json::Value,
    ) -> Result<crate::grpc_client::pb::magic::market::v1::QueryRequest, GrpcError> {
        match build_native_profile_query_request(self.profile, operation, payload)? {
            ProfileQueryRequest::Local(request) => Ok(request),
            ProfileQueryRequest::External(_) => Err(GrpcError::Unimplemented {
                details: Box::default(),
            }),
        }
    }

    pub async fn get_health(
        &mut self,
    ) -> Result<crate::grpc_client::pb::magic::market::v1::HealthResponse, GrpcError> {
        if !matches!(&self.system, SystemTransport::Local(_)) {
            return Err(system_profile_mismatch());
        }
        let request_id = crate::grpc_client::envelope::new_request_id();
        let mut req = tonic::Request::new(HealthRequest {
            context: Some(crate::grpc_client::pb::magic::market::v1::RequestContext {
                protocol_version: 1,
                request_id: request_id.clone(),
            }),
        });
        self.attach_request_auth(&mut req)?;
        let system = match &mut self.system {
            SystemTransport::Local(system) => system,
            SystemTransport::External(_) => return Err(system_profile_mismatch()),
        };
        let resp = system.get_health(req).await.map_err(|status| {
            GrpcError::from_status(
                status,
                StatusErrorContext::control(ContractProfile::LocalBridgeV1, &request_id),
            )
        })?;
        Ok(resp.into_inner())
    }

    pub async fn get_capabilities(
        &mut self,
    ) -> Result<Vec<crate::grpc_client::pb::magic::market::v1::Capability>, GrpcError> {
        if !matches!(&self.system, SystemTransport::Local(_)) {
            return Err(system_profile_mismatch());
        }
        let request_id = crate::grpc_client::envelope::new_request_id();
        let mut req = tonic::Request::new(CapabilitiesRequest {
            context: Some(crate::grpc_client::pb::magic::market::v1::RequestContext {
                protocol_version: 1,
                request_id: request_id.clone(),
            }),
        });
        self.attach_request_auth(&mut req)?;
        let system = match &mut self.system {
            SystemTransport::Local(system) => system,
            SystemTransport::External(_) => return Err(system_profile_mismatch()),
        };
        let resp = system.get_capabilities(req).await.map_err(|status| {
            GrpcError::from_status(
                status,
                StatusErrorContext::control(ContractProfile::LocalBridgeV1, &request_id),
            )
        })?;
        Ok(resp.into_inner().capabilities)
    }

    pub async fn get_external_health(&mut self) -> Result<ExternalHealthResponse, GrpcError> {
        if !matches!(&self.system, SystemTransport::External(_)) {
            return Err(system_profile_mismatch());
        }
        let request_id = crate::grpc_client::envelope::new_request_id();
        let mut request = tonic::Request::new(ExternalHealthRequest {
            context: Some(ExternalRequestContext {
                protocol_version: 1,
                request_id: request_id.clone(),
            }),
        });
        self.attach_request_auth(&mut request)?;
        match self.execute_external_health(request).await {
            ExternalSystemCall::Response(response) => Ok(response),
            ExternalSystemCall::UnaryStatus(status) => Err(GrpcError::from_status(
                status,
                StatusErrorContext::control(ContractProfile::ExternalV1, &request_id),
            )),
        }
    }

    pub async fn get_external_capabilities(
        &mut self,
    ) -> Result<Vec<crate::grpc_client::external_pb::magic::market::v1::Capability>, GrpcError>
    {
        if !matches!(&self.system, SystemTransport::External(_)) {
            return Err(system_profile_mismatch());
        }
        let request_id = crate::grpc_client::envelope::new_request_id();
        let mut request = tonic::Request::new(ExternalCapabilitiesRequest {
            context: Some(ExternalRequestContext {
                protocol_version: 1,
                request_id: request_id.clone(),
            }),
        });
        self.attach_request_auth(&mut request)?;
        match self.execute_external_capabilities(request).await {
            ExternalSystemCall::Response(response) => {
                self.accept_external_capabilities(&request_id, &response)?;
                Ok(response.capabilities)
            }
            ExternalSystemCall::UnaryStatus(status) => Err(GrpcError::from_status(
                status,
                StatusErrorContext::control(ContractProfile::ExternalV1, &request_id),
            )),
        }
    }

    async fn execute_external_health(
        &mut self,
        request: tonic::Request<ExternalHealthRequest>,
    ) -> ExternalSystemCall<ExternalHealthResponse> {
        let system = match &mut self.system {
            SystemTransport::External(system) => system,
            SystemTransport::Local(_) => {
                unreachable!("GrpcMarketClient profile/SystemTransport invariant")
            }
        };
        match system.get_health(request).await {
            Ok(response) => ExternalSystemCall::Response(response.into_inner()),
            Err(status) => ExternalSystemCall::UnaryStatus(status),
        }
    }

    async fn execute_external_capabilities(
        &mut self,
        request: tonic::Request<ExternalCapabilitiesRequest>,
    ) -> ExternalSystemCall<ExternalCapabilitiesResponse> {
        let system = match &mut self.system {
            SystemTransport::External(system) => system,
            SystemTransport::Local(_) => {
                unreachable!("GrpcMarketClient profile/SystemTransport invariant")
            }
        };
        match system.get_capabilities(request).await {
            Ok(response) => ExternalSystemCall::Response(response.into_inner()),
            Err(status) => ExternalSystemCall::UnaryStatus(status),
        }
    }

    /// 按 §10 重试语义执行一次查询 (UNAVAILABLE 指数退避 / DEADLINE_EXCEEDED 有界重试,
    /// 同一业务重试保留原 request_id)。未实现 op 在客户端拦截, 不发起调用。
    pub async fn query(
        &mut self,
        op: Operation,
        payload: serde_json::Value,
    ) -> Result<QueryResult, GrpcError> {
        if !crate::grpc_contract::ops::is_implemented(op) {
            return Err(GrpcError::Unimplemented {
                details: Box::default(),
            });
        }
        let request = build_native_profile_query_request(self.profile, op, payload)?;

        let mut attempt: u32 = 1;
        loop {
            let outcome = self.data_call(op, request.clone()).await;
            match outcome {
                Ok(result) => return Ok(result),
                Err(err) => match retry_decision(&err) {
                    RetryDecision::RetryBackoff | RetryDecision::RetryBounded
                        if attempt < self.retry.max_attempts =>
                    {
                        tokio::time::sleep(self.retry.backoff(attempt)).await;
                        attempt += 1;
                    }
                    _ => return Err(err),
                },
            }
        }
    }

    /// Operation → MarketDataService 方法调用 (实现 op 的 match; 其余已由 is_implemented 拦截)。
    async fn data_call(
        &mut self,
        op: Operation,
        request: ProfileQueryRequest,
    ) -> Result<QueryResult, GrpcError> {
        let method =
            crate::grpc_contract::methods::MethodIdentity::from_client_operation(self.profile, op)
                .map_err(|_| GrpcError::Unimplemented {
                    details: Box::default(),
                })?;
        let request_id = request.request_id().to_owned();
        let outcome = match request {
            ProfileQueryRequest::Local(request) => {
                let mut request = tonic::Request::new(request);
                self.attach_request_auth(&mut request)?;
                self.data_call_authorized(op, ProfileAuthorizedRequest::Local(request)).await
            }
            ProfileQueryRequest::External(request) => {
                let mut request = tonic::Request::new(request);
                self.attach_request_auth(&mut request)?;
                self.data_call_authorized(op, ProfileAuthorizedRequest::External(request)).await
            }
        };
        match outcome {
            DataCallAuthorized::Local(result) => {
                let mut response = result.map_err(|status| {
                    GrpcError::from_status(status, StatusErrorContext::data(method, &request_id))
                })?;
                apply_acquisition_authority(
                    self.profile,
                    self.acquisition_authority.as_deref(),
                    &mut response,
                )?;
                parse_query_response(&request_id, op, response).map_err(GrpcError::from)
            }
            DataCallAuthorized::External(ExternalQueryCall::Response { message, evidence }) => {
                let payload = evidence
                    .payload()
                    .ok_or_else(|| crate::grpc_client::external_query_transport::wire_error("external_response_wire_invalid"))?;
                admit_external_payload(payload)?;
                crate::grpc_client::envelope::parse_external_query_response(
                    &request_id,
                    op,
                    self.acquisition_authority.as_deref().ok_or_else(|| {
                        crate::grpc_client::external_query_transport::wire_error(
                            "external_acquisition_authority_missing",
                        )
                    })?,
                    message,
                )
                .map_err(GrpcError::from)
            }
            DataCallAuthorized::External(ExternalQueryCall::UnaryStatus { status, .. }) => Err(
                GrpcError::from_status(status, self.data_status_context(method, &request_id)),
            ),
            DataCallAuthorized::External(ExternalQueryCall::LocalWireFailure { error, .. }) => {
                Err(error)
            }
            DataCallAuthorized::Rejected(error) => Err(error),
        }
    }

    /// Shared authenticated router used by the ordinary query loop and the
    /// durable board-attempt loop. The caller must attach authorization first.
    async fn data_call_authorized(
        &mut self,
        op: Operation,
        req: ProfileAuthorizedRequest,
    ) -> DataCallAuthorized {
        let (data, req) = match (&mut self.data, req) {
            (DataTransport::Local(data), ProfileAuthorizedRequest::Local(req)) => (data, req),
            (DataTransport::External(data), ProfileAuthorizedRequest::External(req)) => {
                let Some(method) = ExternalQueryMethod::from_local_operation(op) else {
                    return DataCallAuthorized::Rejected(GrpcError::Unimplemented {
                        details: Box::default(),
                    });
                };
                return DataCallAuthorized::External(data.call(method, req).await);
            }
            _ => {
                return DataCallAuthorized::Rejected(GrpcError::FailedPrecondition {
                    details: Box::default(),
                })
            }
        };
        let resp = match op {
            Operation::RealtimeQuotes => data.realtime_quotes(req).await,
            Operation::HistoricalBars => data.historical_bars(req).await,
            Operation::MinuteData => data.minute_data(req).await,
            Operation::OrderBooks => data.order_books(req).await,
            Operation::MoneyFlows => data.money_flows(req).await,
            Operation::SecurityMetadata => data.security_metadata(req).await,
            Operation::Announcements => data.announcements(req).await,
            Operation::GlobalNews => data.global_news(req).await,
            Operation::EconomicCalendar => data.economic_calendar(req).await,
            Operation::FuturesDelivery => data.futures_delivery(req).await,
            Operation::GlobalIndices => data.global_indices(req).await,
            Operation::BoardDirectory => data.board_directory(req).await,
            Operation::BoardConstituents => data.board_constituents(req).await,
            Operation::BoardFlows => data.board_flows(req).await,
            Operation::LimitPools => data.limit_pools(req).await,
            Operation::StrongStockReasons => data.strong_stock_reasons(req).await,
            Operation::DragonTiger => data.dragon_tiger(req).await,
            Operation::MarketDragonTiger => data.market_dragon_tiger(req).await,
            Operation::MarketRankings => data.market_rankings(req).await,
            Operation::ConceptHits => data.concept_hits(req).await,
            Operation::Consensus => data.consensus(req).await,
            Operation::ResearchReports => data.research_reports(req).await,
            Operation::BlockTrades => data.block_trades(req).await,
            Operation::NorthboundDaily => data.northbound_daily(req).await,
            // M1 扩展 (P4): 8 个 proto 已有 op。
            Operation::ForeignExchange => data.foreign_exchange(req).await,
            Operation::FinancialStatements => data.financial_statements(req).await,
            Operation::MarketStatistics => data.market_statistics(req).await,
            Operation::TechnicalBars => data.technical_bars(req).await,
            Operation::CorporateActions => data.corporate_actions(req).await,
            Operation::SemanticSearch => data.semantic_search(req).await,
            Operation::FundFlowSeries => data.fund_flow_series(req).await,
            Operation::ProviderTopNRankings => data.provider_top_n_rankings(req).await,
            // M1 扩展 (P4): 6 个新 op (proto 编号 55-60)。
            Operation::IndexQuotes => data.index_quotes(req).await,
            Operation::InstrumentNews => data.instrument_news(req).await,
            Operation::IntradayShape => data.intraday_shape(req).await,
            Operation::T0Evidence => data.t0_evidence(req).await,
            Operation::OutcomeDailyBars => data.outcome_daily_bars(req).await,
            Operation::UpperLimitPoolReview => data.upper_limit_pool_review(req).await,
            Operation::ChainBatch => data.chain_batch(req).await,
            _ => {
                return DataCallAuthorized::Local(Err(tonic::Status::unimplemented(
                    "operation is not implemented",
                )))
            } // 防御: is_implemented 已拦截
        };
        DataCallAuthorized::Local(resp.map(tonic::Response::into_inner))
    }

    pub async fn subscribe(
        &mut self,
        filter: EventFilter,
        after: Option<EventCursor>,
    ) -> Result<
        tonic::Streaming<crate::grpc_client::pb::magic::market::v1::MarketEventEnvelope>,
        GrpcError,
    > {
        if !matches!(&self.events, EventTransport::Local(_)) {
            return Err(event_profile_mismatch());
        }
        let request_id = crate::grpc_client::envelope::new_request_id();
        let mut req = tonic::Request::new(SubscribeRequest {
            context: Some(crate::grpc_client::pb::magic::market::v1::RequestContext {
                protocol_version: 1,
                request_id: request_id.clone(),
            }),
            filter: Some(filter),
            after,
        });
        self.attach_request_auth(&mut req)?;
        let EventTransport::Local(events) = &mut self.events else {
            unreachable!("event transport/profile precondition checked above")
        };
        let resp = events.subscribe(req).await.map_err(|status| {
            GrpcError::from_status(
                status,
                StatusErrorContext::control(ContractProfile::LocalBridgeV1, &request_id),
            )
        })?;
        Ok(resp.into_inner())
    }

    /// ExternalV1 native Subscribe using generated External request and stream types.
    /// Non-External profiles are rejected before bearer lookup, connect, or RPC.
    pub async fn subscribe_external(
        &mut self,
        filter: ExternalEventFilter,
        after: Option<ExternalEventCursor>,
    ) -> Result<tonic::Streaming<ExternalMarketEventEnvelope>, GrpcError> {
        if !matches!(&self.events, EventTransport::External(_)) {
            return Err(event_profile_mismatch());
        }
        let request_id = crate::grpc_client::envelope::new_request_id();
        let mut req = tonic::Request::new(ExternalSubscribeRequest {
            context: Some(ExternalRequestContext {
                protocol_version: 1,
                request_id: request_id.clone(),
            }),
            filter: Some(filter),
            after,
        });
        self.attach_request_auth(&mut req)?;
        let EventTransport::External(events) = &mut self.events else {
            unreachable!("event transport/profile precondition checked above")
        };
        let resp = events.subscribe(req).await.map_err(|status| {
            GrpcError::from_status(
                status,
                StatusErrorContext::control(ContractProfile::ExternalV1, &request_id),
            )
        })?;
        Ok(resp.into_inner())
    }

    /// 合同 §8 GetListenerStatus: 读取服务端 listener 状态 (generation/cursor/
    /// watchlist 版本)。上游直连排期后用于核对服务端 watchlist 与本地 STOCK_LIST。
    pub async fn get_listener_status(
        &mut self,
    ) -> Result<crate::grpc_client::pb::magic::market::v1::ListenerStatusResponse, GrpcError> {
        if !matches!(&self.events, EventTransport::Local(_)) {
            return Err(event_profile_mismatch());
        }
        let request_id = crate::grpc_client::envelope::new_request_id();
        let mut req = tonic::Request::new(ListenerStatusRequest {
            context: Some(crate::grpc_client::pb::magic::market::v1::RequestContext {
                protocol_version: 1,
                request_id: request_id.clone(),
            }),
        });
        self.attach_request_auth(&mut req)?;
        let EventTransport::Local(events) = &mut self.events else {
            unreachable!("event transport/profile precondition checked above")
        };
        let resp = events.get_listener_status(req).await.map_err(|status| {
            GrpcError::from_status(
                status,
                StatusErrorContext::control(ContractProfile::LocalBridgeV1, &request_id),
            )
        })?;
        Ok(resp.into_inner())
    }

    /// Native ExternalV1 Listener status. The External generated response is returned without
    /// projection so fields 12..19 remain available to the caller.
    pub async fn get_external_listener_status(
        &mut self,
    ) -> Result<ExternalListenerStatusResponse, GrpcError> {
        if !matches!(&self.events, EventTransport::External(_)) {
            return Err(event_profile_mismatch());
        }
        let request_id = crate::grpc_client::envelope::new_request_id();
        let mut req = tonic::Request::new(ExternalListenerStatusRequest {
            context: Some(ExternalRequestContext {
                protocol_version: 1,
                request_id: request_id.clone(),
            }),
        });
        self.attach_request_auth(&mut req)?;
        let EventTransport::External(events) = &mut self.events else {
            unreachable!("event transport/profile precondition checked above")
        };
        let resp = events.get_listener_status(req).await.map_err(|status| {
            GrpcError::from_status(
                status,
                StatusErrorContext::control(ContractProfile::ExternalV1, &request_id),
            )
        })?;
        Ok(resp.into_inner())
    }

    /// 合同 §8 SetWatchlist: 请求覆盖服务端 watchlist (上游: 终端申请 desired,
    /// 服务端决定应用; 本地 server: 立即应用 desired==applied)。返回服务端确认。
    pub async fn set_watchlist(
        &mut self,
        instruments: Vec<String>,
    ) -> Result<crate::grpc_client::pb::magic::market::v1::SetWatchlistResponse, GrpcError> {
        if !matches!(&self.events, EventTransport::Local(_)) {
            return Err(event_profile_mismatch());
        }
        let request_id = crate::grpc_client::envelope::new_request_id();
        let mut req = tonic::Request::new(SetWatchlistRequest {
            context: Some(crate::grpc_client::pb::magic::market::v1::RequestContext {
                protocol_version: 1,
                request_id: request_id.clone(),
            }),
            instruments,
        });
        self.attach_request_auth(&mut req)?;
        let EventTransport::Local(events) = &mut self.events else {
            unreachable!("event transport/profile precondition checked above")
        };
        let resp = events.set_watchlist(req).await.map_err(|status| {
            GrpcError::from_status(
                status,
                StatusErrorContext::control(ContractProfile::LocalBridgeV1, &request_id),
            )
        })?;
        Ok(resp.into_inner())
    }

    /// ExternalV1 native SetWatchlist using generated External request and response types.
    /// Non-External profiles are rejected before bearer lookup, connect, or RPC.
    pub async fn set_external_watchlist(
        &mut self,
        instruments: Vec<String>,
    ) -> Result<ExternalSetWatchlistResponse, GrpcError> {
        if !matches!(&self.events, EventTransport::External(_)) {
            return Err(event_profile_mismatch());
        }
        let request_id = crate::grpc_client::envelope::new_request_id();
        let mut req = tonic::Request::new(ExternalSetWatchlistRequest {
            context: Some(ExternalRequestContext {
                protocol_version: 1,
                request_id: request_id.clone(),
            }),
            instruments,
        });
        self.attach_request_auth(&mut req)?;
        let EventTransport::External(events) = &mut self.events else {
            unreachable!("event transport/profile precondition checked above")
        };
        let resp = events.set_watchlist(req).await.map_err(|status| {
            GrpcError::from_status(
                status,
                StatusErrorContext::control(ContractProfile::ExternalV1, &request_id),
            )
        })?;
        Ok(resp.into_inner())
    }
}

impl PreparedExternalEndpoint {
    #[cfg(test)]
    pub(crate) fn from_plaintext_for_test(
        endpoint: tonic::transport::Endpoint,
        endpoint_uri: String,
        bearer_token: Zeroizing<String>,
        acquisition_authority: String,
    ) -> Self {
        Self {
            endpoint,
            endpoint_uri,
            authorization: ClientAuthorization::InstanceBearer(bearer_token),
            acquisition_authority,
        }
    }

    pub(crate) fn endpoint_uri(&self) -> &str {
        &self.endpoint_uri
    }

    pub(crate) fn prepare_macro_query(
        &self,
        identity: macro_attempt::MacroQueryIdentity,
    ) -> Result<macro_attempt::AuthorizedPreparedMacroRequest, GrpcError> {
        macro_attempt::AuthorizedPreparedMacroRequest::new(self.clone(), identity)
    }

    pub(crate) fn resume_macro_query(
        &self,
        identity: macro_attempt::MacroQueryIdentity,
        restored: macro_attempt::RestoredExternalMacroRequest,
    ) -> Result<macro_attempt::AuthorizedPreparedMacroRequest, GrpcError> {
        macro_attempt::AuthorizedPreparedMacroRequest::resume(self.clone(), identity, restored)
    }

    pub(crate) fn prepare_health_attempt(
        &self,
    ) -> Result<external_control_attempt::AuthorizedHealthAttempt, GrpcError> {
        external_control_attempt::AuthorizedHealthAttempt::new(self.clone())
    }

    pub(crate) fn resume_health_attempt(
        &self,
        material: external_control_attempt::ExternalControlRequestMaterial,
    ) -> Result<external_control_attempt::AuthorizedHealthAttempt, GrpcError> {
        external_control_attempt::AuthorizedHealthAttempt::resume(self.clone(), material)
    }

    pub(crate) fn prepare_capabilities_attempt(
        &self,
    ) -> Result<external_control_attempt::AuthorizedCapabilitiesAttempt, GrpcError> {
        external_control_attempt::AuthorizedCapabilitiesAttempt::new(self.clone())
    }

    pub(crate) fn resume_capabilities_attempt(
        &self,
        material: external_control_attempt::ExternalControlRequestMaterial,
    ) -> Result<external_control_attempt::AuthorizedCapabilitiesAttempt, GrpcError> {
        external_control_attempt::AuthorizedCapabilitiesAttempt::resume(self.clone(), material)
    }

    pub(crate) async fn connect_once(&self) -> Result<GrpcMarketClient, GrpcError> {
        let channel =
            self.endpoint
                .clone()
                .connect()
                .await
                .map_err(|_| GrpcError::Unavailable {
                    details: Box::default(),
                })?;
        let mut client = GrpcMarketClient::from_channel(
            channel,
            ContractProfile::ExternalV1,
            self.authorization.clone(),
            Some(self.acquisition_authority.clone()),
        );
        client.endpoint_uri = Some(self.endpoint_uri.clone());
        Ok(client)
    }

    fn attach_request_auth<T>(&self, request: &mut tonic::Request<T>) -> Result<(), GrpcError> {
        attach_authorization(&self.authorization, request)
    }
}

fn attach_authorization<T>(
    authorization: &ClientAuthorization,
    request: &mut tonic::Request<T>,
) -> Result<(), GrpcError> {
    match authorization {
        ClientAuthorization::Environment => attach_bearer(request)?,
        ClientAuthorization::InstanceBearer(token) => attach_bearer_value(request, token.as_str())?,
    }
    Ok(())
}

fn system_profile_mismatch() -> GrpcError {
    GrpcError::FailedPrecondition {
        details: Box::new(ErrorDetail {
            code: "system_control_profile_mismatch".to_owned(),
            reason_code: Some("system_control_profile_mismatch".to_owned()),
            retryable: Some(false),
            ..ErrorDetail::default()
        }),
    }
}

fn event_profile_mismatch() -> GrpcError {
    GrpcError::FailedPrecondition {
        details: Box::new(ErrorDetail {
            code: "event_profile_mismatch".to_owned(),
            reason_code: Some("event_profile_mismatch".to_owned()),
            retryable: Some(false),
            ..ErrorDetail::default()
        }),
    }
}

pub(crate) fn build_native_profile_query_request(
    profile: ContractProfile,
    operation: Operation,
    payload: serde_json::Value,
) -> Result<ProfileQueryRequest, GrpcError> {
    match profile {
        ContractProfile::LocalBridgeV1 => build_query_request(operation, payload)
            .map(ProfileQueryRequest::Local)
            .map_err(GrpcError::from),
        ContractProfile::ExternalV1 =>
            crate::grpc_client::external_v1::build_external_query_request(operation, payload)
                .map(ProfileQueryRequest::External)
                .map_err(map_external_contract_error),
    }
}

fn map_external_contract_error(
    error: crate::grpc_client::external_v1::ExternalContractError,
) -> GrpcError {
    use crate::grpc_client::external_v1::ExternalContractError;

    match error {
        ExternalContractError::UndeliveredOperation => GrpcError::Unimplemented {
            details: Box::default(),
        },
        ExternalContractError::InvalidParameters => GrpcError::InvalidArgument {
            details: Box::default(),
        },
        ExternalContractError::Serialize => GrpcError::Unknown {
            details: Box::new(ErrorDetail {
                code: "envelope".to_string(),
                ..ErrorDetail::default()
            }),
        },
    }
}

fn apply_acquisition_authority(
    profile: ContractProfile,
    acquisition_authority: Option<&str>,
    response: &mut crate::grpc_client::pb::magic::market::v1::QueryResponse,
) -> Result<(), GrpcError> {
    if profile != ContractProfile::ExternalV1 {
        return Ok(());
    }

    if !response.source.is_empty() {
        return Err(GrpcError::FailedPrecondition {
            details: Box::new(ErrorDetail {
                code: "external_source_field_conflict".to_string(),
                reason_code: Some("external_source_field_conflict".to_string()),
                retryable: Some(false),
                ..ErrorDetail::default()
            }),
        });
    }
    let authority = acquisition_authority.ok_or_else(|| GrpcError::FailedPrecondition {
        details: Box::new(ErrorDetail {
            code: "external_acquisition_authority_missing".to_string(),
            reason_code: Some("external_acquisition_authority_missing".to_string()),
            retryable: Some(false),
            ..ErrorDetail::default()
        }),
    })?;
    response.source = authority.to_owned();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grpc_client::pb::magic::market::v1::{
        market_data_service_server::{MarketDataService, MarketDataServiceServer},
        system_service_server::{SystemService, SystemServiceServer},
        AdmissionState, CanonicalPayload, CapabilitiesResponse, HealthResponse, QueryRequest,
        QueryResponse,
    };
    use tonic::{Request, Response, Status};
    use zeroize::Zeroizing;

    struct MockSystem;
    #[tonic::async_trait]
    impl SystemService for MockSystem {
        async fn get_health(
            &self,
            _req: Request<HealthRequest>,
        ) -> Result<Response<HealthResponse>, Status> {
            Ok(Response::new(HealthResponse {
                request_id: "h-1".into(),
                live: true,
                ready: true,
                state: "RUNNING".into(),
            }))
        }
        async fn get_capabilities(
            &self,
            _req: Request<CapabilitiesRequest>,
        ) -> Result<Response<CapabilitiesResponse>, Status> {
            Ok(Response::new(CapabilitiesResponse {
                request_id: "c-1".into(),
                capabilities: vec![],
            }))
        }
    }

    struct MockData;

    // macro_rules! 不能在 impl 块内定义 → 模块级宏生成整个 trait impl。
    macro_rules! impl_mock_market_data {
        ($($stub:ident),* $(,)?) => {
            #[tonic::async_trait]
            impl MarketDataService for MockData {
                async fn realtime_quotes(&self, req: Request<QueryRequest>) -> Result<Response<QueryResponse>, Status> {
            let inner = req.into_inner();
            let request_id = inner.context.unwrap().request_id;
            Ok(Response::new(QueryResponse {
                request_id,
                operation: Operation::RealtimeQuotes as i32,
                admission: AdmissionState::Admitted as i32,
                selected_provider: "mock".into(),
                batch_id: "mock-b1".into(),
                complete: true,
                observed_at: "2026-08-13T10:00:00+08:00".into(),
                source_at: "2026-08-13T10:00:00+08:00".into(),
                source: "mock".into(),
                diagnostic_blocker: String::new(),
                records: vec![CanonicalPayload {
                    schema: "market.realtime_quotes".into(),
                    schema_version: 1,
                    content_type: "application/json; charset=utf-8".into(),
                    // raw byte string (br#) 不允许非 ASCII; 用普通字符串再转 bytes。
                    data: r#"[{"code":"600519","name":"贵州茅台"}]"#.as_bytes().to_vec(),
                }],
            }))
        }

                // tonic 生成的 MarketDataService trait 共 60 个方法 (上游 55 + 本地扩展 5),
                // 全部必须实现。这里只有 realtime_quotes 是真实桩; 其余 59 个
                // (proto RPC 名 camelCase) 全部 unimplemented。
                $(
                    async fn $stub(&self, _req: Request<QueryRequest>) -> Result<Response<QueryResponse>, Status> {
                        Err(Status::unimplemented(stringify!($stub)))
                    }
                )*
            }
        };
    }

    impl_mock_market_data!(
        historical_bars,
        minute_data,
        money_flows,
        order_books,
        auctions,
        trades,
        security_metadata,
        global_indices,
        foreign_exchange,
        economic_calendar,
        futures_delivery,
        reference_rates,
        official_fx_fixings,
        economic_series,
        company_filings,
        global_news,
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
        instrument_news,
        intraday_shape,
        t0_evidence,
        outcome_daily_bars,
        upper_limit_pool_review,
        chain_batch,
        benchmark_bars,
    );

    async fn spawn_mock() -> String {
        let addr = "127.0.0.1:0";
        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
        let local = listener.local_addr().unwrap();
        tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(SystemServiceServer::new(MockSystem))
                .add_service(MarketDataServiceServer::new(MockData))
                .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
                .await
                .unwrap();
        });
        format!("http://{local}")
    }

    #[tokio::test]
    async fn query_realtime_quotes_roundtrip() {
        let addr = spawn_mock().await;
        let mut client = GrpcMarketClient::connect(&addr).await.unwrap();
        let result = client
            .query(
                Operation::RealtimeQuotes,
                serde_json::json!({"codes": ["600519"]}),
            )
            .await
            .unwrap();
        // 用 PartialEq 断言 (envelope.rs 已验证 AdmissionState 可比较), 不依赖 prost 是否生成 Display。
        assert_eq!(result.admission, AdmissionState::Admitted);
        assert!(result.complete);
        assert_eq!(result.records.len(), 1);
        let payload = &result.records[0];
        assert_eq!(payload.schema, "market.realtime_quotes");
        let parsed: serde_json::Value = serde_json::from_slice(&payload.data).unwrap();
        assert_eq!(parsed[0]["code"], "600519");
    }

    #[tokio::test]
    async fn query_unimplemented_op_returns_unimplemented() {
        let addr = spawn_mock().await;
        let mut client = GrpcMarketClient::connect(&addr).await.unwrap();
        // OptionData 不在 implemented 集合 → 客户端直接拦截, 不发起调用。
        let err = client
            .query(Operation::OptionData, serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, GrpcError::Unimplemented { .. }));
    }

    #[tokio::test]
    async fn get_health_and_capabilities_roundtrip() {
        let addr = spawn_mock().await;
        let mut client = GrpcMarketClient::connect(&addr).await.unwrap();
        let health = client.get_health().await.unwrap();
        assert!(health.live && health.ready);
        assert_eq!(health.state, "RUNNING");
        let caps = client.get_capabilities().await.unwrap();
        assert!(caps.is_empty());
    }

    fn lazy_test_channel() -> Channel {
        Channel::from_static("http://127.0.0.1:1").connect_lazy()
    }

    #[tokio::test]
    async fn event_methods_reject_wrong_profile_before_network_io() {
        let bearer = || {
            ClientAuthorization::InstanceBearer(Zeroizing::new(
                "TEST_CODE_event_profile_token".to_owned(),
            ))
        };
        let mut local = GrpcMarketClient::from_channel(
            lazy_test_channel(),
            ContractProfile::LocalBridgeV1,
            bearer(),
            None,
        );
        assert!(matches!(&local.events, EventTransport::Local(_)));
        let local_subscribe_error = match local
            .subscribe_external(ExternalEventFilter::default(), None)
            .await
        {
            Err(error) => error,
            Ok(_) => panic!("Local profile must reject External Subscribe before I/O"),
        };
        let local_error = match local.get_external_listener_status().await {
            Err(error) => error,
            Ok(_) => panic!("Local profile must reject External Listener before I/O"),
        };
        let local_watchlist_error = match local
            .set_external_watchlist(vec!["EQUITY:SH:TEST_CODE_600396".to_owned()])
            .await
        {
            Err(error) => error,
            Ok(_) => panic!("Local profile must reject External SetWatchlist before I/O"),
        };

        let mut external = GrpcMarketClient::from_channel(
            lazy_test_channel(),
            ContractProfile::ExternalV1,
            bearer(),
            Some("grpc-mtls:TEST_CODE-event-profile".to_owned()),
        );
        assert!(matches!(&external.events, EventTransport::External(_)));
        let subscribe_error = match external.subscribe(EventFilter::default(), None).await {
            Err(error) => error,
            Ok(_) => panic!("External profile must reject Local Subscribe before I/O"),
        };
        let listener_error = match external.get_listener_status().await {
            Err(error) => error,
            Ok(_) => panic!("External profile must reject Local Listener before I/O"),
        };
        let watchlist_error = match external
            .set_watchlist(vec!["EQUITY:SH:TEST_CODE_600396".to_owned()])
            .await
        {
            Err(error) => error,
            Ok(_) => panic!("External profile must reject Local SetWatchlist before I/O"),
        };

        for error in [
            local_subscribe_error,
            local_error,
            local_watchlist_error,
            subscribe_error,
            listener_error,
            watchlist_error,
        ] {
            assert!(matches!(&error, GrpcError::FailedPrecondition { .. }));
            assert_eq!(error.details().code, "event_profile_mismatch");
            assert_eq!(
                error.details().reason_code.as_deref(),
                Some("event_profile_mismatch")
            );
            assert_eq!(error.details().retryable, Some(false));
        }
    }

    #[tokio::test]
    async fn contract_profiles_select_distinct_request_contracts() {
        let local = GrpcMarketClient::from_channel(
            lazy_test_channel(),
            ContractProfile::LocalBridgeV1,
            ClientAuthorization::Environment,
            None,
        );
        let local_request = local
            .build_profile_query_request(
                Operation::RealtimeQuotes,
                serde_json::json!({"codes": ["600396"]}),
            )
            .expect("local bridge request");
        assert_eq!(
            local_request.payload.expect("local payload").schema,
            "market.realtime_quotes"
        );

        let external = GrpcMarketClient::from_channel(
            lazy_test_channel(),
            ContractProfile::ExternalV1,
            ClientAuthorization::InstanceBearer(Zeroizing::new(
                "TEST_CODE_bundle_token".to_string(),
            )),
            Some("grpc-mtls:magic-market.local".to_string()),
        );
        let external_request = build_native_profile_query_request(
                external.profile,
                Operation::SecurityMetadata,
                serde_json::json!({
                    "instruments": [{
                        "exchange": "Shanghai",
                        "code": "600396",
                        "asset_class": "Equity"
                    }]
                }),
            )
            .expect("delivered external request");
        let ProfileQueryRequest::External(external_request) = external_request else {
            panic!("external profile must build native external request")
        };
        assert_eq!(
            external_request.payload.expect("external payload").schema,
            "magic.market.security_metadata.request"
        );

        let invalid = build_native_profile_query_request(
                external.profile,
                Operation::SecurityMetadata,
                serde_json::json!({"instruments": []}),
            )
            .expect_err("invalid external parameters must fail closed");
        assert!(matches!(invalid, GrpcError::InvalidArgument { .. }));

        let undelivered = build_native_profile_query_request(
            external.profile,
            Operation::RealtimeQuotes,
            serde_json::json!({}),
        )
            .expect_err("undelivered external contract must not reach I/O");
        assert!(matches!(undelivered, GrpcError::Unimplemented { .. }));
    }

    #[test]
    fn external_contract_error_mapping_is_non_retryable_and_specific() {
        use crate::grpc_client::external_v1::ExternalContractError;

        assert!(matches!(
            map_external_contract_error(ExternalContractError::UndeliveredOperation),
            GrpcError::Unimplemented { .. }
        ));
        assert!(matches!(
            map_external_contract_error(ExternalContractError::InvalidParameters),
            GrpcError::InvalidArgument { .. }
        ));
        let serialization = map_external_contract_error(ExternalContractError::Serialize);
        assert!(matches!(serialization, GrpcError::Unknown { .. }));
        assert_eq!(serialization.details().code, "envelope");
    }

    #[tokio::test]
    async fn instance_owned_bearer_is_attached_without_environment_fallback() {
        let client = GrpcMarketClient::from_channel(
            lazy_test_channel(),
            ContractProfile::ExternalV1,
            ClientAuthorization::InstanceBearer(Zeroizing::new(
                "TEST_CODE_bundle_token".to_string(),
            )),
            Some("grpc-mtls:magic-market.local".to_string()),
        );
        let mut request = Request::new(());
        client
            .attach_request_auth(&mut request)
            .expect("instance bearer metadata");
        assert_eq!(
            request
                .metadata()
                .get("authorization")
                .expect("authorization metadata")
                .to_str()
                .expect("ASCII authorization"),
            "Bearer TEST_CODE_bundle_token"
        );
        assert_eq!(client.profile, ContractProfile::ExternalV1);
        assert_eq!(
            client.acquisition_authority.as_deref(),
            Some("grpc-mtls:magic-market.local")
        );
    }

    fn response_with_source(source: &str) -> QueryResponse {
        QueryResponse {
            request_id: "TEST_CODE-request".to_string(),
            operation: Operation::SecurityMetadata as i32,
            admission: AdmissionState::Admitted as i32,
            selected_provider: "Tencent".to_string(),
            batch_id: "TEST_CODE-batch".to_string(),
            complete: true,
            observed_at: "2026-08-17T08:00:00+08:00".to_string(),
            source_at: "2026-08-17T07:59:59+08:00".to_string(),
            records: vec![],
            diagnostic_blocker: String::new(),
            source: source.to_string(),
        }
    }

    #[test]
    fn external_acquisition_authority_rejects_remote_field_eleven_and_uses_local_mtls() {
        let authority = "grpc-mtls:magic-market.local";
        let mut missing = response_with_source("");
        apply_acquisition_authority(ContractProfile::ExternalV1, Some(authority), &mut missing)
            .expect("local mTLS authority");
        assert_eq!(missing.source, authority);
        assert_eq!(missing.selected_provider, "Tencent");
        assert_eq!(missing.batch_id, "TEST_CODE-batch");
        assert_eq!(missing.observed_at, "2026-08-17T08:00:00+08:00");
        assert_eq!(missing.source_at, "2026-08-17T07:59:59+08:00");

        let mut upstream = response_with_source("upstream-source");
        let error = apply_acquisition_authority(
            ContractProfile::ExternalV1,
            Some(authority),
            &mut upstream,
        )
        .expect_err("ExternalV1 field 11 is not an upstream contract field");
        assert!(matches!(error, GrpcError::FailedPrecondition { .. }));

        let mut local = response_with_source("");
        apply_acquisition_authority(ContractProfile::LocalBridgeV1, Some(authority), &mut local)
            .expect("local bridge keeps its own source");
        assert!(local.source.is_empty());
    }

    #[test]
    fn client_bundle_constructor_is_exposed_without_reading_a_real_bundle() {
        let _constructor = GrpcMarketClient::connect_client_bundle;
    }
}
