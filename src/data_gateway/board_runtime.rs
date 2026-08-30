//! BR-164/BR-188 evidence-preserving board discovery and flow Gateway runtime.

use super::review::{acquisition_request_hash, audit_gateway_result};
use super::{BatchEvidence, GatewayBatch, GatewayError};

use crate::market_domain::{InstrumentId, ProviderId};

const DIRECTORY_CAPABILITY: &str = "board-directory";
const MEMBERSHIP_CAPABILITY: &str = "board-memberships";
const FLOW_CAPABILITY: &str = "board-flows";
const PRODUCTION_TDX_CONNECT_TIMEOUT_SECONDS: f64 = 5.0;
/// BR-243: connect_to_any 遍历 10 台 PRIMARY + 101 台 ALL_KNOWN (每台 5s 超时)
/// 实测 9-15s — 超过 gRPC 桥客户端 15s deadline 导致 CANCELLED。缓存已验证
/// 可达的 (server, port), 缓存命中直接复用, 不再逐调用遍历。
const TDX_BOARD_SERVER_CACHE_TTL_SECS: u64 = 60;
const BOARD_CONNECTION_POLICY_VERSION: &str = "selection-board-tdx-production-v1";
const BOARD_DIRECTORY_PROVIDER: &str = "tdx";
const BOARD_DIRECTORY_SOURCE: &str = "tdx-block-files";
const BOARD_GATEWAY_CONSTRUCTOR: &str = "BoardDataGateway::production_tdx";
const BOARD_RESOLVER_POLICY: &str = "magic_tdx_production_resolver_v1";
const BOARD_ENDPOINT_OVERRIDE: &str = "forbidden";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoardConnectionPolicyIdentity {
    version: &'static str,
    provider: &'static str,
    source: &'static str,
    gateway_constructor: &'static str,
    resolver_policy: &'static str,
    endpoint_override: &'static str,
}

impl BoardConnectionPolicyIdentity {
    pub const fn version(self) -> &'static str {
        self.version
    }

    pub const fn provider(self) -> &'static str {
        self.provider
    }

    pub const fn source(self) -> &'static str {
        self.source
    }

    pub const fn gateway_constructor(self) -> &'static str {
        self.gateway_constructor
    }

    pub const fn resolver_policy(self) -> &'static str {
        self.resolver_policy
    }

    pub const fn endpoint_override(self) -> &'static str {
        self.endpoint_override
    }
}

const PRODUCTION_TDX_CONNECTION_POLICY: BoardConnectionPolicyIdentity =
    BoardConnectionPolicyIdentity {
        version: BOARD_CONNECTION_POLICY_VERSION,
        provider: BOARD_DIRECTORY_PROVIDER,
        source: BOARD_DIRECTORY_SOURCE,
        gateway_constructor: BOARD_GATEWAY_CONSTRUCTOR,
        resolver_policy: BOARD_RESOLVER_POLICY,
        endpoint_override: BOARD_ENDPOINT_OVERRIDE,
    };

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoardKind {
    Industry,
    Concept,
    Region,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardDirectoryRecordEvidence {
    pub provider: ProviderId,
    pub source: String,
    pub source_at: Option<String>,
    pub observed_at: String,
    pub batch_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardDirectoryFact {
    pub code: String,
    pub name: String,
    pub kind: BoardKind,
    pub member_count: u32,
    pub evidence: BoardDirectoryRecordEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardMembershipRecord {
    pub instrument_code: String,
    pub board_code: String,
    pub board_name: String,
    pub kind: BoardKind,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BoardFlowFact {
    pub code: String,
    pub name: String,
    pub kind: BoardKind,
    pub rank: u32,
    pub return_pct: Option<f64>,
    pub main_net_yuan: Option<f64>,
    pub leader_code: Option<String>,
    pub leader_name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoardDataGateway {
    connection_policy: BoardConnectionPolicyIdentity,
}

impl Default for BoardDataGateway {
    fn default() -> Self {
        Self::production_tdx()
    }
}

impl BoardDataGateway {
    pub const fn new() -> Self {
        Self::production_tdx()
    }

    pub const fn production_tdx() -> Self {
        Self {
            connection_policy: PRODUCTION_TDX_CONNECTION_POLICY,
        }
    }

    pub const fn connection_policy_identity(self) -> BoardConnectionPolicyIdentity {
        self.connection_policy
    }

    pub async fn directory(
        &self,
        kind: BoardKind,
        limit: u32,
    ) -> Result<GatewayBatch<BoardDirectoryFact>, GatewayError> {
        let request_hash =
            acquisition_request_hash(DIRECTORY_CAPABILITY, format!("{kind:?}:{limit}"));
        // P4 M3: gRPC 桥 (remote gRPC 时替换 transport; audit 留客户端)。
        match super::grpc_source::bridge_for("BoardDirectory") {
            Ok(bridge) => {
                let result = bridge.board_directory_async(kind, limit).await;
                let audit_provider = result
                    .as_ref()
                    .map(|b| b.evidence().provider)
                    .unwrap_or(ProviderId::Tdx);
                return audit_gateway_result(
                    DIRECTORY_CAPABILITY,
                    audit_provider,
                    &request_hash,
                    result,
                );
            }
            Err(error) => {
                return audit_gateway_result(
                    DIRECTORY_CAPABILITY,
                    ProviderId::Tdx,
                    &request_hash,
                    Err(error),
                );
            }
        }
        // no-feature (monitor 零 magic): library transport 不存在。
        // 无 bridge 时显式失败 (fail-closed), 绝不静默回退。
    }

    pub async fn memberships(
        &self,
        code: &str,
    ) -> Result<GatewayBatch<BoardMembershipRecord>, GatewayError> {
        let code = validate_code(code, MEMBERSHIP_CAPABILITY)?.to_owned();
        let request_hash = acquisition_request_hash(MEMBERSHIP_CAPABILITY, &code);
        // P4 M3: gRPC 桥 (remote gRPC 时替换 transport; audit 留客户端)。
        match super::grpc_source::bridge_for("BoardConstituents") {
            Ok(bridge) => {
                let result = bridge.board_constituents_async(&code).await;
                let audit_provider = result
                    .as_ref()
                    .map(|b| b.evidence().provider)
                    .unwrap_or(ProviderId::Tdx);
                return audit_gateway_result(
                    MEMBERSHIP_CAPABILITY,
                    audit_provider,
                    &request_hash,
                    result,
                );
            }
            Err(error) => {
                return audit_gateway_result(
                    MEMBERSHIP_CAPABILITY,
                    ProviderId::Tdx,
                    &request_hash,
                    Err(error),
                );
            }
        }
        // no-feature (monitor 零 magic): library transport 不存在。
        // 无 bridge 时显式失败 (fail-closed), 绝不静默回退。
    }

    /// Blocking entry for existing synchronous target-symbol consumers.
    ///
    /// This uses the same configured transport selection and evidence/audit
    /// admission as [`Self::memberships`]. It exists so a synchronous caller
    /// can reuse the gRPC bridge's runtime-safe blocking wrapper.
    pub fn memberships_blocking(
        &self,
        code: &str,
    ) -> Result<GatewayBatch<BoardMembershipRecord>, GatewayError> {
        let code = validate_code(code, MEMBERSHIP_CAPABILITY)?.to_owned();
        let request_hash = acquisition_request_hash(MEMBERSHIP_CAPABILITY, &code);
        // BR-238: 同步消费者复用与 async memberships 完全相同的 gRPC
        // acquisition + audit 分支；桥失败显式返回，绝不降级 library。
        match super::grpc_source::bridge_for("BoardConstituents") {
            Ok(bridge) => {
                let result = bridge.board_constituents(&code);
                let audit_provider = result
                    .as_ref()
                    .map(|batch| batch.evidence().provider)
                    .unwrap_or(ProviderId::Tdx);
                return audit_gateway_result(
                    MEMBERSHIP_CAPABILITY,
                    audit_provider,
                    &request_hash,
                    result,
                );
            }
            Err(error) => {
                return audit_gateway_result(
                    MEMBERSHIP_CAPABILITY,
                    ProviderId::Tdx,
                    &request_hash,
                    Err(error),
                );
            }
        }
        // no-feature (monitor 零 magic): library transport 不存在。
        // 无 bridge 时显式失败 (fail-closed), 绝不静默回退。
    }

    pub async fn day1_flows(
        &self,
        kind: BoardKind,
        limit: u32,
    ) -> Result<GatewayBatch<BoardFlowFact>, GatewayError> {
        let request_hash =
            acquisition_request_hash(FLOW_CAPABILITY, format!("{kind:?}:Day1:{limit}"));
        // P4 M3: gRPC 桥 (remote gRPC 时替换 transport; audit 留客户端)。
        match super::grpc_source::bridge_for("BoardFlows") {
            Ok(bridge) => {
                let result = bridge.board_flows_async(kind, limit).await;
                let audit_provider = result
                    .as_ref()
                    .map(|b| b.evidence().provider)
                    .unwrap_or(ProviderId::Eastmoney);
                return audit_gateway_result(
                    FLOW_CAPABILITY,
                    audit_provider,
                    &request_hash,
                    result,
                );
            }
            Err(error) => {
                return audit_gateway_result(
                    FLOW_CAPABILITY,
                    ProviderId::Eastmoney,
                    &request_hash,
                    Err(error),
                );
            }
        }
        // no-feature (monitor 零 magic): library transport 不存在。
        // 无 bridge 时显式失败 (fail-closed), 绝不静默回退。
    }

    /// Blocking entry for existing synchronous review consumers.
    ///
    /// Provider construction, request validation, evidence admission and audit
    /// remain owned by this Gateway; callers only receive normalized facts.
    pub fn day1_flows_blocking(
        &self,
        kind: BoardKind,
        limit: u32,
    ) -> Result<GatewayBatch<BoardFlowFact>, GatewayError> {
        let request_hash =
            acquisition_request_hash(FLOW_CAPABILITY, format!("{kind:?}:Day1:{limit}"));
        // P4 M3: gRPC 桥 (同步路径, spawn_blocking 内调用 → block_on)。
        match super::grpc_source::bridge_for("BoardFlows") {
            Ok(bridge) => {
                let result = bridge.board_flows(kind, limit);
                let audit_provider = result
                    .as_ref()
                    .map(|b| b.evidence().provider)
                    .unwrap_or(ProviderId::Eastmoney);
                return audit_gateway_result(
                    FLOW_CAPABILITY,
                    audit_provider,
                    &request_hash,
                    result,
                );
            }
            Err(error) => {
                return audit_gateway_result(
                    FLOW_CAPABILITY,
                    ProviderId::Eastmoney,
                    &request_hash,
                    Err(error),
                );
            }
        }
        // no-feature (monitor 零 magic): library transport 不存在。
        // 无 bridge 时显式失败 (fail-closed), 绝不静默回退。
    }
}

impl BoardDataGateway {}

/// 已验证可达的 TDX (server, port) 缓存。connect_to_any 每次遍历 111 台
/// 服务器 (每台 5s 超时, 实测 9-15s) — 缓存命中后直接复用已验证端点,
/// TdxBlockClient 惰性连接该服务器 (毫秒级)。缓存仅作端点记忆:
/// 端点失联时 TdxBlockClient 查询失败 → fail-closed 返回 + 缓存过期后重连。
static TDX_BOARD_SERVER_CACHE: std::sync::OnceLock<
    std::sync::Mutex<Option<(String, u16, std::time::Instant)>>,
> = std::sync::OnceLock::new();

fn finish_batch<T>(
    records: Vec<T>,
    evidence: BatchEvidence,
) -> Result<GatewayBatch<T>, GatewayError> {
    if records.is_empty() {
        Ok(GatewayBatch::VerifiedEmpty(evidence))
    } else {
        Ok(GatewayBatch::Available { records, evidence })
    }
}

fn ensure_complete(
    capability: &'static str,
    complete: bool,
    issues: &[String],
) -> Result<(), GatewayError> {
    if complete {
        Ok(())
    } else {
        Err(GatewayError::classified(
            capability,
            None,
            "partial",
            "provider_partial_batch",
            false,
            format!("quality issues: {issues:?}"),
        ))
    }
}

fn validate_source_evidence(
    capability: &'static str,
    record: &crate::market_domain::SourceEvidence,
    batch: &BatchEvidence,
    provider: ProviderId,
) -> Result<(), GatewayError> {
    validate_batch_evidence(capability, batch, provider)?;
    if record.provider() != provider
        || record.source_at() != batch.source_at.as_deref()
        || record.batch_id() != batch.batch_id
        || record.observed_at() != batch.observed_at
    {
        return Err(GatewayError::invalid_evidence(
            capability,
            Some(provider),
            "record evidence differs from batch evidence",
        ));
    }
    Ok(())
}

fn validate_batch_evidence(
    capability: &'static str,
    batch: &BatchEvidence,
    provider: ProviderId,
) -> Result<(), GatewayError> {
    if batch.provider != provider {
        return Err(GatewayError::invalid_evidence(
            capability,
            Some(provider),
            "batch provider differs from the fixed Gateway provider",
        ));
    }
    if provider == ProviderId::Tdx
        && (batch.source != BOARD_DIRECTORY_SOURCE || batch.source_at.is_some())
    {
        return Err(GatewayError::invalid_evidence(
            capability,
            Some(provider),
            "TDX board evidence must use tdx-block-files with no provider publication time",
        ));
    }
    Ok(())
}

fn validate_code<'a>(code: &'a str, capability: &'static str) -> Result<&'a str, GatewayError> {
    a_share_instrument(code, capability)?;
    Ok(code)
}

fn a_share_instrument(code: &str, capability: &'static str) -> Result<InstrumentId, GatewayError> {
    #[cfg(test)]
    let resolved = super::instrument_identity::resolve_test_equity(code, None);
    #[cfg(not(test))]
    let resolved = super::instrument_identity::resolve_production_equity(code, None);
    let identity =
        resolved.map_err(|error| GatewayError::invalid_request(capability, error.to_string()))?;
    identity
        .require_a_share()
        .map_err(|error| GatewayError::invalid_request(capability, error.to_string()))?;
    Ok(identity.instrument().clone())
}

#[cfg(test)]
mod no_magic_bridge_tests {
    use super::BoardDataGateway;
    use crate::database::DatabaseManager;

    #[test]
    fn grpc_env_guard_blocking_membership_uses_bridge_when_enabled() {
        let _env = super::super::grpc_source::test_grpc_env_guard();
        DatabaseManager::init(None).expect("TEST_CODE audit database init");
        std::env::remove_var("GRPC_MARKET_CLIENT_BUNDLE");
        std::env::set_var("GRPC_MARKET_ADDR", "http://127.0.0.1:1");
        super::super::grpc_source::reset_bridge();

        // resolve_test_equity 将该测试命名空间映射为合法上海 A 股 identity；
        // 此调用只读查询 membership，不经过订单或生产写入路径。
        let result = BoardDataGateway::new().memberships_blocking("TEST_CODE_600519");

        let error = result.expect_err("unreachable gRPC bridge must fail closed");

        assert_eq!(error.capability(), "GrpcBridge");
        assert_eq!(error.reason_code(), "no_verified_batch");
        assert!(error.retryable());
        assert!(
            !error.message().contains("legacy local transport fallback"),
            "blocking entry must try the configured bridge: {error}"
        );
    }
}
