//! BR-164/BR-188 evidence-preserving board discovery and flow Gateway runtime.

use super::review::{acquisition_request_hash, audit_gateway_result};
#[cfg(feature = "magic-gateway")]
use super::review::audit_blocking_join_failure;
use super::{BatchEvidence, GatewayBatch, GatewayError};
#[cfg(feature = "magic-gateway")]
use magic_eastmoney_rs::{EastmoneyClient, EastmoneyError};
use crate::magic_compat::{InstrumentId, ProviderId};
#[cfg(feature = "magic-gateway")]
use crate::magic_compat::{DataBatch, FlowInterval, PositiveU32};
#[cfg(feature = "magic-gateway")]
use magic_market_core::{BoardCategory, BoardConstituentProvider, BoardConstituentRequest, BoardDirectoryProvider, BoardDirectoryRequest, BoardFlows, BoardMembership, BoardMembershipProvider};
#[cfg(feature = "magic-gateway")]
use magic_tdx_rs::{TdxBoardProvider, TdxError, TdxHqClient};

const DIRECTORY_CAPABILITY: &str = "board-directory";
const MEMBERSHIP_CAPABILITY: &str = "board-memberships";
const FLOW_CAPABILITY: &str = "board-flows";
const PRODUCTION_TDX_CONNECT_TIMEOUT_SECONDS: f64 = 5.0;
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

    #[cfg(feature = "magic-gateway")]
    pub(super) fn board_constituents_raw(
        &self,
        request: &BoardConstituentRequest,
    ) -> Result<DataBatch<BoardMembership>, GatewayError> {
        self.connected_tdx_board_provider(MEMBERSHIP_CAPABILITY)?
            .board_constituents(request)
            .map_err(|error| tdx_gateway_error(MEMBERSHIP_CAPABILITY, error))
    }

    pub async fn directory(
        &self,
        kind: BoardKind,
        limit: u32,
    ) -> Result<GatewayBatch<BoardDirectoryFact>, GatewayError> {
        let request_hash =
            acquisition_request_hash(DIRECTORY_CAPABILITY, &format!("{kind:?}:{limit}"));
        // P4 M3: gRPC 桥 (DATA_GATEWAY_GRPC=1 时替换 transport; audit 留客户端)。
        match super::grpc_source::bridge_for("BoardDirectory") {
            Ok(Some(bridge)) => {
                let result = bridge.board_directory_async(kind, limit).await;
                let audit_provider = result
                    .as_ref()
                    .map(|b| b.evidence().provider)
                    .unwrap_or(ProviderId::Tdx);
                return audit_gateway_result(DIRECTORY_CAPABILITY, audit_provider, &request_hash, result);
            }
            Ok(None) => {}
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
        #[cfg(not(feature = "magic-gateway"))]
        {
            return Err(GatewayError::classified(
                DIRECTORY_CAPABILITY,
                Some(ProviderId::Tdx),
                "unavailable",
                "provider_transport",
                true,
                "library transport disabled: DATA_GATEWAY_GRPC=1 required",
            ));
        }
        #[cfg(feature = "magic-gateway")]
        {
            let worker_hash = request_hash.clone();
            let gateway = *self;
            let joined = tokio::task::spawn_blocking(move || {
                let result = build_directory_request(kind, limit)
                    .and_then(|request| fetch_directory(gateway, request));
                audit_gateway_result(DIRECTORY_CAPABILITY, ProviderId::Tdx, &worker_hash, result)
            })
            .await;
            match joined {
                Ok(result) => result,
                Err(error) => {
                    audit_blocking_join_failure(
                        DIRECTORY_CAPABILITY,
                        ProviderId::Tdx,
                        request_hash,
                        error.to_string(),
                    )
                    .await
                }
            }
        }
    }

    pub async fn memberships(
        &self,
        code: &str,
    ) -> Result<GatewayBatch<BoardMembershipRecord>, GatewayError> {
        let code = validate_code(code, MEMBERSHIP_CAPABILITY)?.to_owned();
        let request_hash = acquisition_request_hash(MEMBERSHIP_CAPABILITY, &code);
        // P4 M3: gRPC 桥 (DATA_GATEWAY_GRPC=1 时替换 transport; audit 留客户端)。
        match super::grpc_source::bridge_for("BoardConstituents") {
            Ok(Some(bridge)) => {
                let result = bridge.board_constituents_async(&code).await;
                let audit_provider = result
                    .as_ref()
                    .map(|b| b.evidence().provider)
                    .unwrap_or(ProviderId::Tdx);
                return audit_gateway_result(MEMBERSHIP_CAPABILITY, audit_provider, &request_hash, result);
            }
            Ok(None) => {}
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
        #[cfg(not(feature = "magic-gateway"))]
        {
            return Err(GatewayError::classified(
                MEMBERSHIP_CAPABILITY,
                Some(ProviderId::Tdx),
                "unavailable",
                "provider_transport",
                true,
                "library transport disabled: DATA_GATEWAY_GRPC=1 required",
            ));
        }
        #[cfg(feature = "magic-gateway")]
        {
            let gateway = *self;
            let joined =
                tokio::task::spawn_blocking(move || fetch_memberships_audited(gateway, code))
                    .await;
            match joined {
                Ok(result) => result,
                Err(error) => {
                    audit_blocking_join_failure(
                        MEMBERSHIP_CAPABILITY,
                        ProviderId::Tdx,
                        request_hash,
                        error.to_string(),
                    )
                    .await
                }
            }
        }
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
        // BR-231: 同步消费者复用与 async memberships 完全相同的 gRPC
        // acquisition + audit 分支；桥失败显式返回，绝不降级 library。
        match super::grpc_source::bridge_for("BoardConstituents") {
            Ok(Some(bridge)) => {
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
            Ok(None) => {}
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
        #[cfg(not(feature = "magic-gateway"))]
        {
            return Err(GatewayError::classified(
                MEMBERSHIP_CAPABILITY,
                Some(ProviderId::Tdx),
                "unavailable",
                "provider_transport",
                true,
                &format!(
                    "library transport disabled: DATA_GATEWAY_GRPC=1 required (code={code})"
                ),
            ));
        }
        #[cfg(feature = "magic-gateway")]
        {
            fetch_memberships_audited(*self, code)
        }
    }

    pub async fn day1_flows(
        &self,
        kind: BoardKind,
        limit: u32,
    ) -> Result<GatewayBatch<BoardFlowFact>, GatewayError> {
        let request_hash =
            acquisition_request_hash(FLOW_CAPABILITY, &format!("{kind:?}:Day1:{limit}"));
        // P4 M3: gRPC 桥 (DATA_GATEWAY_GRPC=1 时替换 transport; audit 留客户端)。
        match super::grpc_source::bridge_for("BoardFlows") {
            Ok(Some(bridge)) => {
                let result = bridge.board_flows_async(kind, limit).await;
                let audit_provider = result
                    .as_ref()
                    .map(|b| b.evidence().provider)
                    .unwrap_or(ProviderId::Eastmoney);
                return audit_gateway_result(FLOW_CAPABILITY, audit_provider, &request_hash, result);
            }
            Ok(None) => {}
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
        #[cfg(not(feature = "magic-gateway"))]
        {
            return Err(GatewayError::classified(
                FLOW_CAPABILITY,
                Some(ProviderId::Eastmoney),
                "unavailable",
                "provider_transport",
                true,
                "library transport disabled: DATA_GATEWAY_GRPC=1 required",
            ));
        }
        #[cfg(feature = "magic-gateway")]
        {
            let worker_hash = request_hash.clone();
            let joined = tokio::task::spawn_blocking(move || {
                let result = build_flow_request(kind, limit).and_then(fetch_flows);
                audit_gateway_result(FLOW_CAPABILITY, ProviderId::Eastmoney, &worker_hash, result)
            })
            .await;
            match joined {
                Ok(result) => result,
                Err(error) => {
                    audit_blocking_join_failure(
                        FLOW_CAPABILITY,
                        ProviderId::Eastmoney,
                        request_hash,
                        error.to_string(),
                    )
                    .await
                }
            }
        }
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
            acquisition_request_hash(FLOW_CAPABILITY, &format!("{kind:?}:Day1:{limit}"));
        // P4 M3: gRPC 桥 (同步路径, spawn_blocking 内调用 → block_on)。
        match super::grpc_source::bridge_for("BoardFlows") {
            Ok(Some(bridge)) => {
                let result = bridge.board_flows(kind, limit);
                let audit_provider = result
                    .as_ref()
                    .map(|b| b.evidence().provider)
                    .unwrap_or(ProviderId::Eastmoney);
                return audit_gateway_result(FLOW_CAPABILITY, audit_provider, &request_hash, result);
            }
            Ok(None) => {}
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
        #[cfg(not(feature = "magic-gateway"))]
        {
            return Err(GatewayError::classified(
                FLOW_CAPABILITY,
                Some(ProviderId::Eastmoney),
                "unavailable",
                "provider_transport",
                true,
                "library transport disabled: DATA_GATEWAY_GRPC=1 required",
            ));
        }
        #[cfg(feature = "magic-gateway")]
        {
            let result = build_flow_request(kind, limit).and_then(fetch_flows);
            audit_gateway_result(
                FLOW_CAPABILITY,
                ProviderId::Eastmoney,
                &request_hash,
                result,
            )
        }
    }
}

#[cfg(feature = "magic-gateway")]
fn fetch_memberships_audited(
    gateway: BoardDataGateway,
    code: String,
) -> Result<GatewayBatch<BoardMembershipRecord>, GatewayError> {
    let request_hash = acquisition_request_hash(MEMBERSHIP_CAPABILITY, &code);
    let result =
        build_instrument(&code).and_then(|instrument| fetch_memberships(gateway, instrument));
    audit_gateway_result(
        MEMBERSHIP_CAPABILITY,
        ProviderId::Tdx,
        &request_hash,
        result,
    )
}

#[cfg(feature = "magic-gateway")]
fn build_directory_request(
    kind: BoardKind,
    limit: u32,
) -> Result<BoardDirectoryRequest, GatewayError> {
    let limit = PositiveU32::new(limit)
        .map_err(|error| GatewayError::invalid_request(DIRECTORY_CAPABILITY, error.to_string()))?;
    BoardDirectoryRequest::new(category(kind)?, limit)
        .map_err(|error| GatewayError::invalid_request(DIRECTORY_CAPABILITY, error.to_string()))
}

#[cfg(feature = "magic-gateway")]
fn fetch_directory(
    gateway: BoardDataGateway,
    request: BoardDirectoryRequest,
) -> Result<GatewayBatch<BoardDirectoryFact>, GatewayError> {
    let provider = gateway.connected_tdx_board_provider(DIRECTORY_CAPABILITY)?;
    let batch = provider
        .boards(&request)
        .map_err(|error| tdx_gateway_error(DIRECTORY_CAPABILITY, error))?;
    ensure_complete(
        DIRECTORY_CAPABILITY,
        batch.quality().is_complete(),
        batch.quality().issues(),
    )?;
    let evidence = BatchEvidence::from_provenance(ProviderId::Tdx, batch.provenance())?;
    validate_batch_evidence(DIRECTORY_CAPABILITY, &evidence, ProviderId::Tdx)?;
    let records = batch
        .records()
        .iter()
        .map(|record| {
            validate_source_evidence(
                DIRECTORY_CAPABILITY,
                record.evidence(),
                &evidence,
                ProviderId::Tdx,
            )?;
            Ok(BoardDirectoryFact {
                code: record.board_code().as_str().to_owned(),
                name: record.board_name().as_str().to_owned(),
                kind: kind(record.category())?,
                member_count: record.member_count().get(),
                evidence: BoardDirectoryRecordEvidence {
                    provider: record.evidence().provider(),
                    source: evidence.source.clone(),
                    source_at: record.evidence().source_at().map(str::to_owned),
                    observed_at: record.evidence().observed_at().to_owned(),
                    batch_id: record.evidence().batch_id().to_owned(),
                },
            })
        })
        .collect::<Result<Vec<_>, GatewayError>>()?;
    finish_batch(records, evidence)
}

#[cfg(feature = "magic-gateway")]
fn build_instrument(code: &str) -> Result<InstrumentId, GatewayError> {
    a_share_instrument(code, MEMBERSHIP_CAPABILITY)
}

#[cfg(feature = "magic-gateway")]
fn fetch_memberships(
    gateway: BoardDataGateway,
    instrument: InstrumentId,
) -> Result<GatewayBatch<BoardMembershipRecord>, GatewayError> {
    let provider = gateway.connected_tdx_board_provider(MEMBERSHIP_CAPABILITY)?;
    let batch = provider
        .board_memberships(std::slice::from_ref(&instrument))
        .map_err(|error| tdx_gateway_error(MEMBERSHIP_CAPABILITY, error))?;
    ensure_complete(
        MEMBERSHIP_CAPABILITY,
        batch.quality().is_complete(),
        batch.quality().issues(),
    )?;
    let evidence = BatchEvidence::from_provenance(ProviderId::Tdx, batch.provenance())?;
    validate_batch_evidence(MEMBERSHIP_CAPABILITY, &evidence, ProviderId::Tdx)?;
    let records = batch
        .records()
        .iter()
        .map(|record| {
            validate_source_evidence(
                MEMBERSHIP_CAPABILITY,
                &record.evidence,
                &evidence,
                ProviderId::Tdx,
            )?;
            if record.instrument != instrument {
                return Err(GatewayError::invalid_evidence(
                    MEMBERSHIP_CAPABILITY,
                    Some(ProviderId::Tdx),
                    "TDX board membership returned a different instrument",
                ));
            }
            Ok(BoardMembershipRecord {
                instrument_code: record.instrument.code().to_owned(),
                board_code: record.board_code.as_str().to_owned(),
                board_name: record.board_name.as_str().to_owned(),
                kind: kind(record.category)?,
            })
        })
        .collect::<Result<Vec<_>, GatewayError>>()?;
    finish_batch(records, evidence)
}

#[cfg(feature = "magic-gateway")]
fn build_flow_request(
    kind: BoardKind,
    limit: u32,
) -> Result<(BoardCategory, PositiveU32), GatewayError> {
    let limit = PositiveU32::new(limit)
        .map_err(|error| GatewayError::invalid_request(FLOW_CAPABILITY, error.to_string()))?;
    let category = category(kind)?;
    Ok((category, limit))
}

#[cfg(feature = "magic-gateway")]
fn fetch_flows(
    request: (BoardCategory, PositiveU32),
) -> Result<GatewayBatch<BoardFlowFact>, GatewayError> {
    let provider = EastmoneyClient::new().map_err(eastmoney_gateway_error)?;
    let batch = provider
        .board_flows(request.0, FlowInterval::Day1, request.1)
        .map_err(eastmoney_gateway_error)?;
    ensure_complete(
        FLOW_CAPABILITY,
        batch.quality().is_complete(),
        batch.quality().issues(),
    )?;
    let evidence = BatchEvidence::from_provenance(ProviderId::Eastmoney, batch.provenance())?;
    validate_batch_evidence(FLOW_CAPABILITY, &evidence, ProviderId::Eastmoney)?;
    let records = batch
        .records()
        .iter()
        .map(|record| {
            validate_source_evidence(
                FLOW_CAPABILITY,
                &record.evidence,
                &evidence,
                ProviderId::Eastmoney,
            )?;
            Ok(BoardFlowFact {
                code: record.board_code.as_str().to_owned(),
                name: record.board_name.as_str().to_owned(),
                kind: kind(record.category)?,
                rank: record.rank.get(),
                return_pct: record.return_ratio.map(|value| value.get()),
                main_net_yuan: record.main_net.map(|value| value.get()),
                leader_code: record
                    .leader_instrument
                    .as_ref()
                    .map(|value| value.code().to_owned()),
                leader_name: record
                    .leader_name
                    .as_ref()
                    .map(|value| value.as_str().to_owned()),
            })
        })
        .collect::<Result<Vec<_>, GatewayError>>()?;
    finish_batch(records, evidence)
}

impl BoardDataGateway {
    #[cfg(feature = "magic-gateway")]
    fn connected_tdx_board_provider(
        self,
        capability: &'static str,
    ) -> Result<TdxBoardProvider, GatewayError> {
        if self.connection_policy != PRODUCTION_TDX_CONNECTION_POLICY {
            return Err(GatewayError::invalid_request(
                capability,
                "board Gateway does not carry the fixed production TDX connection policy",
            ));
        }
        resolve_production_tdx_board_provider(capability)
    }
}

#[cfg(feature = "magic-gateway")]
fn resolve_production_tdx_board_provider(
    capability: &'static str,
) -> Result<TdxBoardProvider, GatewayError> {
    let client = TdxHqClient::new();
    let connected = client
        .connect_to_any(Some(PRODUCTION_TDX_CONNECT_TIMEOUT_SECONDS))
        .map_err(|error| tdx_gateway_error(capability, error))?;
    if !connected {
        return Err(GatewayError::unavailable(
            capability,
            Some(ProviderId::Tdx),
            true,
            "fixed Magic TDX production resolver did not connect",
        ));
    }
    let (server, port) = client.connected_server().ok_or_else(|| {
        GatewayError::unavailable(
            capability,
            Some(ProviderId::Tdx),
            true,
            "Magic TDX connected without exposing a server identity",
        )
    })?;
    Ok(TdxBoardProvider::new(
        &server,
        port,
        PRODUCTION_TDX_CONNECT_TIMEOUT_SECONDS,
    ))
}

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
    record: &crate::magic_compat::SourceEvidence,
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

#[cfg(feature = "magic-gateway")]
fn category(kind: BoardKind) -> Result<BoardCategory, GatewayError> {
    Ok(match kind {
        BoardKind::Industry => BoardCategory::Industry,
        BoardKind::Concept => BoardCategory::Concept,
        BoardKind::Region => BoardCategory::Region,
    })
}

#[cfg(feature = "magic-gateway")]
fn kind(category: BoardCategory) -> Result<BoardKind, GatewayError> {
    match category {
        BoardCategory::Industry => Ok(BoardKind::Industry),
        BoardCategory::Concept => Ok(BoardKind::Concept),
        BoardCategory::Region => Ok(BoardKind::Region),
        BoardCategory::Unknown => Err(GatewayError::invalid_evidence(
            DIRECTORY_CAPABILITY,
            None,
            "provider returned an unknown board category",
        )),
    }
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

#[cfg(feature = "magic-gateway")]
fn tdx_gateway_error(capability: &'static str, error: TdxError) -> GatewayError {
    let message = error.to_string();
    match error {
        TdxError::Io(_)
        | TdxError::FileNotFound(_)
        | TdxError::Connection(_)
        | TdxError::ConnectionTimeout
        | TdxError::SetupFailed(_)
        | TdxError::Disconnected
        | TdxError::RetryExhausted(_) => GatewayError::classified(
            capability,
            Some(ProviderId::Tdx),
            "unavailable",
            "provider_transport",
            true,
            message,
        ),
        TdxError::Unsupported(_) => GatewayError::classified(
            capability,
            Some(ProviderId::Tdx),
            "unsupported",
            "provider_unsupported",
            false,
            message,
        ),
        TdxError::HistoricalBarCardinality {
            offset,
            actual,
            expected_page,
            requested_total,
        } => GatewayError::classified(
            capability,
            Some(ProviderId::Tdx),
            "unavailable",
            "provider_invalid_batch",
            false,
            format!(
                "Magic TDX historical-bar cardinality mismatch: offset={offset} actual={actual} \
                 expected_page={expected_page} requested_total={requested_total}"
            ),
        ),
        TdxError::Parse(_)
        | TdxError::InvalidData(_)
        | TdxError::ResponseParse(_)
        | TdxError::Core(_)
        | TdxError::Coded(_) => GatewayError::classified(
            capability,
            Some(ProviderId::Tdx),
            "unavailable",
            "provider_invalid_batch",
            false,
            message,
        ),
    }
}

#[cfg(feature = "magic-gateway")]
fn eastmoney_gateway_error(error: EastmoneyError) -> GatewayError {
    let message = error.to_string();
    match error {
        EastmoneyError::InvalidRequest(_) => {
            GatewayError::invalid_request(FLOW_CAPABILITY, message)
        }
        EastmoneyError::Unsupported(_) => GatewayError::classified(
            FLOW_CAPABILITY,
            Some(ProviderId::Eastmoney),
            "unsupported",
            "provider_unsupported",
            false,
            message,
        ),
        EastmoneyError::Transport(_) => GatewayError::classified(
            FLOW_CAPABILITY,
            Some(ProviderId::Eastmoney),
            "unavailable",
            "provider_transport",
            true,
            message,
        ),
        EastmoneyError::VerifiedEmpty(_) => GatewayError::classified(
            FLOW_CAPABILITY,
            Some(ProviderId::Eastmoney),
            "verified_empty",
            "verified_empty",
            false,
            message,
        ),
        EastmoneyError::ResponseTooLarge { .. }
        | EastmoneyError::Decode(_)
        | EastmoneyError::Protocol(_)
        | EastmoneyError::Core(_) => GatewayError::classified(
            FLOW_CAPABILITY,
            Some(ProviderId::Eastmoney),
            "unavailable",
            "provider_invalid_batch",
            false,
            message,
        ),
    }
}

#[cfg(test)]
#[cfg(feature = "magic-gateway")]
mod tests {
    use super::{
        build_directory_request, build_flow_request, build_instrument, category,
        eastmoney_gateway_error, ensure_complete, finish_batch, kind, tdx_gateway_error,
        validate_source_evidence, BatchEvidence, BoardDataGateway, BoardKind, GatewayBatch,
        DIRECTORY_CAPABILITY, FLOW_CAPABILITY, MEMBERSHIP_CAPABILITY,
    };
    #[cfg(feature = "magic-gateway")]
    use magic_eastmoney_rs::EastmoneyError;
    use crate::magic_compat::{Exchange, ProviderId, SourceEvidence};
    #[cfg(feature = "magic-gateway")]
    use magic_market_core::BoardCategory;
    #[cfg(feature = "magic-gateway")]
    use magic_tdx_rs::TdxError;

    fn evidence(provider: ProviderId) -> BatchEvidence {
        BatchEvidence {
            provider,
            source: "TEST_CODE_board".to_owned(),
            source_at: Some("2026-07-25T15:00:00+08:00".to_owned()),
            observed_at: "1784982000.000000000".to_owned(),
            batch_id: "TEST_CODE_board_batch".to_owned(),
        }
    }

    #[test]
    fn production_tdx_exposes_the_frozen_connection_policy_identity() {
        let gateway = BoardDataGateway::production_tdx();
        let identity = gateway.connection_policy_identity();

        assert_eq!(identity.version(), "selection-board-tdx-production-v1");
        assert_eq!(identity.provider(), "tdx");
        assert_eq!(identity.source(), "tdx-block-files");
        assert_eq!(
            identity.gateway_constructor(),
            "BoardDataGateway::production_tdx"
        );
        assert_eq!(
            identity.resolver_policy(),
            "magic_tdx_production_resolver_v1"
        );
        assert_eq!(identity.endpoint_override(), "forbidden");
    }

    #[test]
    fn category_mapping_is_explicit() {
        assert_eq!(kind(BoardCategory::Industry).unwrap(), BoardKind::Industry);
        assert_eq!(kind(BoardCategory::Concept).unwrap(), BoardKind::Concept);
        assert_eq!(kind(BoardCategory::Region).unwrap(), BoardKind::Region);
        assert!(kind(BoardCategory::Unknown).is_err());
        assert_eq!(
            category(BoardKind::Industry).unwrap(),
            BoardCategory::Industry
        );
        assert_eq!(
            category(BoardKind::Concept).unwrap(),
            BoardCategory::Concept
        );
        assert_eq!(category(BoardKind::Region).unwrap(), BoardCategory::Region);
    }

    #[test]
    fn instrument_exchange_is_validated() {
        for (code, exchange) in [
            ("TEST_CODE_600000", Exchange::Shanghai),
            ("TEST_CODE_000001", Exchange::Shenzhen),
            ("TEST_CODE_300001", Exchange::Shenzhen),
            ("TEST_CODE_920001", Exchange::Beijing),
        ] {
            let instrument = build_instrument(code).unwrap();
            assert_eq!(instrument.exchange(), exchange, "{code}");
            assert_eq!(instrument.code(), code, "{code}");
        }
        for code in [
            "TEST_CODE_430047",
            "TEST_CODE_830001",
            "TEST_CODE_200001",
            "TEST_CODE_900901",
            "TEST_CODE_100000",
            "TEST_CODE_60000A",
            "TEST_CODE_60000",
        ] {
            assert!(build_instrument(code).is_err(), "{code}");
        }
    }

    #[test]
    fn request_limits_reject_zero_and_preserve_category() {
        let directory = build_directory_request(BoardKind::Region, 50).unwrap();
        assert_eq!(directory.category(), BoardCategory::Region);
        assert_eq!(directory.limit().get(), 50);
        assert!(build_directory_request(BoardKind::Concept, 0).is_err());

        let flow = build_flow_request(BoardKind::Industry, 20).unwrap();
        assert_eq!(flow.0, BoardCategory::Industry);
        assert_eq!(flow.1.get(), 20);
        assert!(build_flow_request(BoardKind::Industry, 0).is_err());
    }

    #[test]
    fn blocking_membership_entry_rejects_invalid_identity_before_provider_access() {
        let error = BoardDataGateway::new()
            .memberships_blocking("TEST_CODE_NOT_A_SHARE")
            .unwrap_err();
        assert_eq!(error.audit_outcome(), "invalid_request");
        assert_eq!(error.reason_code(), "invalid_request");
        assert!(!error.retryable());
    }

    #[test]
    fn empty_complete_and_partial_batches_are_distinct() {
        let empty = finish_batch::<u8>(Vec::new(), evidence(ProviderId::Tdx)).unwrap();
        assert!(empty.is_verified_empty());

        let available = finish_batch(vec![7_u8], evidence(ProviderId::Tdx)).unwrap();
        assert!(matches!(
            available,
            GatewayBatch::Available { records, .. } if records == vec![7]
        ));

        assert!(ensure_complete(DIRECTORY_CAPABILITY, true, &[]).is_ok());
        let partial = ensure_complete(
            DIRECTORY_CAPABILITY,
            false,
            &["TEST_CODE missing row".to_owned()],
        )
        .unwrap_err();
        assert_eq!(partial.reason_code(), "provider_partial_batch");
        assert!(!partial.retryable());
    }

    #[test]
    fn record_evidence_must_match_provider_batch_and_observation() {
        let batch = BatchEvidence {
            provider: ProviderId::Tdx,
            source: "tdx-block-files".to_owned(),
            source_at: None,
            observed_at: "1784982000.000000000".to_owned(),
            batch_id: "TEST_CODE_board_batch".to_owned(),
        };
        let matching =
            SourceEvidence::new(ProviderId::Tdx, &batch.observed_at, &batch.batch_id).unwrap();
        assert!(
            validate_source_evidence(DIRECTORY_CAPABILITY, &matching, &batch, ProviderId::Tdx,)
                .is_ok()
        );

        for mismatched in [
            SourceEvidence::new(ProviderId::Eastmoney, &batch.observed_at, &batch.batch_id)
                .unwrap(),
            SourceEvidence::new(ProviderId::Tdx, &batch.observed_at, "TEST_CODE_wrong_batch")
                .unwrap(),
            SourceEvidence::new(ProviderId::Tdx, "1784982001.000000000", &batch.batch_id).unwrap(),
            SourceEvidence::new(ProviderId::Tdx, &batch.observed_at, &batch.batch_id)
                .and_then(|value| value.with_source_at("2026-07-28T10:00:00.000000000Z"))
                .unwrap(),
        ] {
            assert_eq!(
                validate_source_evidence(
                    DIRECTORY_CAPABILITY,
                    &mismatched,
                    &batch,
                    ProviderId::Tdx,
                )
                .unwrap_err()
                .reason_code(),
                "invalid_evidence"
            );
        }

        for mismatched_batch in [
            BatchEvidence {
                provider: ProviderId::Eastmoney,
                ..batch.clone()
            },
            BatchEvidence {
                source: "TEST_CODE_dynamic_source".to_owned(),
                ..batch.clone()
            },
            BatchEvidence {
                source_at: Some("2026-07-28T10:00:00.000000000Z".to_owned()),
                ..batch.clone()
            },
        ] {
            assert_eq!(
                validate_source_evidence(
                    DIRECTORY_CAPABILITY,
                    &matching,
                    &mismatched_batch,
                    ProviderId::Tdx,
                )
                .unwrap_err()
                .reason_code(),
                "invalid_evidence"
            );
        }
    }

    #[test]
    fn provider_errors_keep_retry_and_failure_categories() {
        let transport = tdx_gateway_error(DIRECTORY_CAPABILITY, TdxError::ConnectionTimeout);
        assert_eq!(transport.reason_code(), "provider_transport");
        assert!(transport.retryable());
        let unsupported = tdx_gateway_error(
            MEMBERSHIP_CAPABILITY,
            TdxError::Unsupported("TEST_CODE missing".to_owned()),
        );
        assert_eq!(unsupported.reason_code(), "provider_unsupported");
        assert!(!unsupported.retryable());
        let invalid = tdx_gateway_error(
            DIRECTORY_CAPABILITY,
            TdxError::InvalidData("TEST_CODE bad row".to_owned()),
        );
        assert_eq!(invalid.reason_code(), "provider_invalid_batch");
        assert!(!invalid.retryable());
        let cardinality = tdx_gateway_error(
            DIRECTORY_CAPABILITY,
            TdxError::HistoricalBarCardinality {
                offset: 800,
                actual: 99,
                expected_page: 100,
                requested_total: 900,
            },
        );
        assert_eq!(cardinality.audit_outcome(), "unavailable");
        assert_eq!(cardinality.reason_code(), "provider_invalid_batch");
        assert!(!cardinality.retryable());
        let cardinality_message = cardinality.to_string();
        for expected in [
            "offset=800",
            "actual=99",
            "expected_page=100",
            "requested_total=900",
        ] {
            assert!(cardinality_message.contains(expected));
        }

        let invalid_request =
            eastmoney_gateway_error(EastmoneyError::InvalidRequest("TEST_CODE bad".to_owned()));
        assert_eq!(invalid_request.reason_code(), "invalid_request");
        let eastmoney_transport =
            eastmoney_gateway_error(EastmoneyError::Transport("TEST_CODE offline".to_owned()));
        assert_eq!(eastmoney_transport.reason_code(), "provider_transport");
        assert!(eastmoney_transport.retryable());
        let eastmoney_unsupported =
            eastmoney_gateway_error(EastmoneyError::Unsupported("TEST_CODE missing".to_owned()));
        assert_eq!(eastmoney_unsupported.reason_code(), "provider_unsupported");
        let eastmoney_invalid =
            eastmoney_gateway_error(EastmoneyError::Protocol("TEST_CODE bad schema".to_owned()));
        assert_eq!(eastmoney_invalid.reason_code(), "provider_invalid_batch");
        assert!(!eastmoney_invalid.retryable());

        assert_eq!(FLOW_CAPABILITY, "board-flows");
    }
}

#[cfg(all(test, not(feature = "magic-gateway")))]
mod no_magic_bridge_tests {
    use super::BoardDataGateway;
    use crate::database::DatabaseManager;
    use serial_test::serial;

    #[test]
    #[serial]
    fn blocking_membership_uses_grpc_bridge_when_enabled() {
        DatabaseManager::init(None).expect("TEST_CODE audit database init");
        std::env::set_var("DATA_GATEWAY_GRPC", "1");
        std::env::remove_var("DATA_GATEWAY_GRPC_DISABLED");
        std::env::remove_var("GRPC_MARKET_CLIENT_BUNDLE");
        std::env::set_var("GRPC_MARKET_ADDR", "http://127.0.0.1:1");
        super::super::grpc_source::reset_bridge();

        // resolve_test_equity 将该测试命名空间映射为合法上海 A 股 identity；
        // 此调用只读查询 membership，不经过订单或生产写入路径。
        let result = BoardDataGateway::new().memberships_blocking("TEST_CODE_600519");

        std::env::remove_var("DATA_GATEWAY_GRPC");
        std::env::remove_var("GRPC_MARKET_CLIENT_BUNDLE");
        std::env::remove_var("GRPC_MARKET_ADDR");
        super::super::grpc_source::reset_bridge();

        let error = result.expect_err("unreachable gRPC bridge must fail closed");

        assert_eq!(error.capability(), "GrpcBridge");
        assert_eq!(error.reason_code(), "no_verified_batch");
        assert!(error.retryable());
        assert!(
            !error.message().contains("library transport disabled"),
            "blocking entry must try the configured bridge: {error}"
        );
    }
}
