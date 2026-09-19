use chrono::{DateTime, FixedOffset};
use prost::Message as _;
use serde::{Deserialize, Serialize};

use super::{dragon_tiger::MacroParent, ChainPostCloseError};
use crate::data_gateway::grpc_source::LocalSemanticSearchConnectionState;
use crate::data_gateway::grpc_source::{
    macro_queries::map_macro_error, map_external_query_error, GrpcSource,
};
use crate::data_gateway::review::store_gateway_error;
use crate::data_gateway::{
    GatewayBatch, GatewayError, GeneralWebResearchProvider, GlobalNewsProvider, GlobalNewsRecord,
};
use crate::grpc_client::client::external_control_attempt::{
    validate_capabilities_response_id, validate_health_response_id,
    validated_external_provider_catalog, ExternalControlCompletion, ExternalControlKind,
    ExternalControlRequestMaterial, ExternalControlResultMaterial,
};
use crate::grpc_client::client::macro_attempt::{
    project_macro_response, AuthorizedMacroAttempt, AuthorizedPreparedMacroRequest,
    ExternalMacroAttemptCompletion, MacroAttemptCompletion, MacroContinuation, MacroQueryIdentity,
    MacroTrailerMaterial, RestoredExternalMacroRequest, RestoredMacroRequest,
};
use crate::grpc_client::client::ContractProfile;
use crate::grpc_client::errors::{
    restore_persisted_status_error, GrpcError, PersistedErrorDetailTrailer, StatusErrorContext,
};
use crate::grpc_client::external_pb::magic::market::v1::{
    CapabilitiesRequest, CapabilitiesResponse, HealthRequest, HealthResponse,
    Operation as ExternalOperation, QueryRequest as ExternalQueryRequest,
    QueryResponse as ExternalQueryResponse,
};
use crate::grpc_client::external_query_transport::{
    admit_external_payload, wire_error, ExternalQueryMethod, ExternalWireEvidenceV1,
    ExternalWireMaterialV1,
};
use crate::grpc_client::pb::magic::market::v1::{Operation, QueryRequest, QueryResponse};
use crate::grpc_client::provider_attempts::{ExternalProviderCatalog, ProviderAttempts};
use crate::grpc_client::retry::{retry_decision, RetryDecision, RetryPolicy};
use crate::grpc_contract::methods::{ExternalMethod, MethodIdentity};
use crate::monitor::push_job::{raw_digest, UtcMicros};
use crate::search_service::service::MacroWebSnapshot;

pub(super) type Result<T> = std::result::Result<T, ChainPostCloseError>;
pub(super) type NewsResult = std::result::Result<GatewayBatch<GlobalNewsRecord>, GatewayError>;

fn external_global_news() -> ExternalMethod {
    ExternalMethod::try_from_operation(ExternalOperation::GlobalNews)
        .expect("External GlobalNews is a nonzero generated operation")
}

pub(super) fn require(value: bool) -> Result<()> {
    if value {
        Ok(())
    } else {
        Err(ChainPostCloseError::SchemaRejected)
    }
}

pub(super) fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    serde_json::to_vec(value).map_err(|_| ChainPostCloseError::SchemaRejected)
}

pub(super) fn decode<T: for<'a> Deserialize<'a> + Serialize>(bytes: &[u8]) -> Result<T> {
    let value: T =
        serde_json::from_slice(bytes).map_err(|_| ChainPostCloseError::SchemaRejected)?;
    require(encode(&value)? == bytes)?;
    Ok(value)
}

pub(super) fn first_identity() -> MacroQueryIdentity {
    MacroQueryIdentity::GlobalNews {
        provider: GlobalNewsProvider::Eastmoney,
        limit: 20,
    }
}

fn source_identities() -> Vec<MacroQueryIdentity> {
    [
        GlobalNewsProvider::Eastmoney,
        GlobalNewsProvider::Cailianpress,
        GlobalNewsProvider::Jin10,
        GlobalNewsProvider::ThePaper,
    ]
    .into_iter()
    .map(|provider| MacroQueryIdentity::GlobalNews {
        provider,
        limit: 20,
    })
    .chain(std::iter::once(MacroQueryIdentity::EconomicCalendar))
    .collect()
}

fn queries(observed: DateTime<FixedOffset>) -> Vec<String> {
    let date = observed.format("%Y年%m月%d日");
    [
        "A股 大盘 股市 最新动态",
        "国际财经 地缘政治 最新消息",
        "美股 美联储 大宗商品 今日",
        "中国 央行 财政 产业政策 重要新闻",
        "高盛 摩根 大摩 美银 JPMorgan 中国A股 市场观点 研报",
        "证券时报 第一财经 21世纪经济报道 重要财经",
    ]
    .into_iter()
    .map(|suffix| format!("{date}{suffix}"))
    .collect()
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResearchDecision {
    provider: String,
    supported: bool,
    available: bool,
    availability_source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    registration_ordinal: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    local_transport_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    remote_health: Option<String>,
}

impl ResearchDecision {
    pub(crate) fn supports_general_web_search(&self) -> bool {
        self.supported
    }
    /// For v2 this means only that this explicit `GrpcSource` already held a
    /// connected Local SemanticSearch transport when the plan was fixed. It
    /// does not imply remote provider health or legacy/global bridge availability.
    pub(crate) fn is_available(&self) -> bool {
        self.available
    }
    pub(crate) fn registration_ordinal(&self) -> Option<u32> {
        self.registration_ordinal
    }
    pub(crate) fn availability_source(&self) -> &str {
        &self.availability_source
    }
    pub(crate) fn local_transport_endpoint(&self) -> Option<&str> {
        self.local_transport_endpoint.as_deref()
    }
    pub(crate) fn remote_health(&self) -> Option<&str> {
        self.remote_health.as_deref()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResearchDecisionProvenance {
    LegacySuppliedConnectedLocalBridgeV1,
    ExplicitRegistryV2,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    pub(super) bytes: Vec<u8>,
    pub(super) id: String,
    pub(super) policy: (u32, u64, u64, u64),
    pub(super) profile: String,
    pub(super) authority: Option<String>,
}

impl Request {
    pub(super) fn capture_for(
        identity: &MacroQueryIdentity,
        attempt: &AuthorizedMacroAttempt,
    ) -> Result<Self> {
        let request = Self {
            bytes: attempt.request_bytes(),
            id: attempt.request_id().to_owned(),
            policy: attempt.retry_policy(),
            profile: attempt.profile().to_owned(),
            authority: attempt.acquisition_authority().map(str::to_owned),
        };
        request.validate_for(identity)?;
        Ok(request)
    }

    pub(super) fn validate_for(&self, identity: &MacroQueryIdentity) -> Result<()> {
        let external = match self.profile.as_str() {
            "LocalBridgeV1" => { require(self.authority.is_none())?; false },
            "ExternalV1" => {
                require(self.authority.as_ref().is_some_and(|value| value.starts_with("grpc-mtls:") && value.len() <= 512))?;
                require(matches!(identity, MacroQueryIdentity::GlobalNews { .. }))?;
                true
            },
            _ => return Err(ChainPostCloseError::SchemaRejected),
        };
        require(!self.id.is_empty() && self.id.len() <= 512 && self.policy.0 > 0)?;
        let params = match identity {
            MacroQueryIdentity::GlobalNews { provider, limit } => serde_json::json!({"provider":provider.wire_name(),"limit":limit}),
            MacroQueryIdentity::EconomicCalendar => serde_json::json!({}),
            MacroQueryIdentity::SemanticSearch { provider, query, limit } => serde_json::json!({"provider":provider.wire_name(),"query":query,"limit":limit}),
        };
        if external {
            let request = ExternalQueryRequest::decode(self.bytes.as_slice())
                .map_err(|_| ChainPostCloseError::SchemaRejected)?;
            require(request.encode_to_vec() == self.bytes)?;
            let mut expected = crate::grpc_client::external_v1::build_external_query_request(
                identity.operation(),
                params,
            )
            .map_err(|_| ChainPostCloseError::SchemaRejected)?;
            expected
                .context
                .as_mut()
                .ok_or(ChainPostCloseError::SchemaRejected)?
                .request_id = self.id.clone();
            require(request == expected)
        } else {
            let request = QueryRequest::decode(self.bytes.as_slice())
                .map_err(|_| ChainPostCloseError::SchemaRejected)?;
            require(request.encode_to_vec() == self.bytes)?;
            let mut expected =
                crate::grpc_client::envelope::build_query_request(identity.operation(), params)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?;
            expected
                .context
                .as_mut()
                .ok_or(ChainPostCloseError::SchemaRejected)?
                .request_id = self.id.clone();
            require(request == expected)
        }
    }

    pub(super) fn capture(attempt: &AuthorizedMacroAttempt) -> Result<Self> {
        let value = Self {
            bytes: attempt.request_bytes(),
            id: attempt.request_id().to_owned(),
            policy: attempt.retry_policy(),
            profile: attempt.profile().to_owned(),
            authority: attempt.acquisition_authority().map(str::to_owned),
        };
        value.validate()?;
        Ok(value)
    }

    pub(super) fn capture_prepared(attempt: &AuthorizedPreparedMacroRequest) -> Result<Self> {
        let value = Self {
            bytes: attempt.request_bytes(),
            id: attempt.request_id().to_owned(),
            policy: attempt.retry_policy(),
            profile: match attempt.profile() {
                ContractProfile::LocalBridgeV1 => "LocalBridgeV1",
                ContractProfile::ExternalV1 => "ExternalV1",
            }
            .to_owned(),
            authority: Some(attempt.acquisition_authority().to_owned()),
        };
        value.validate()?;
        Ok(value)
    }

    pub(super) fn capture_prepared_for(identity: &MacroQueryIdentity, attempt: &AuthorizedPreparedMacroRequest) -> Result<Self> {
        let value = Self { bytes:attempt.request_bytes(),id:attempt.request_id().to_owned(),policy:attempt.retry_policy(),
            profile:"ExternalV1".to_owned(),authority:Some(attempt.acquisition_authority().to_owned()) };
        require(attempt.profile() == ContractProfile::ExternalV1)?;
        value.validate_for(identity)?;
        Ok(value)
    }

    pub(super) fn validate(&self) -> Result<()> {
        self.validate_for(&first_identity())
    }

    pub(super) fn restored(&self, ordinal: u32) -> RestoredMacroRequest {
        RestoredMacroRequest {
            request_bytes: self.bytes.clone(),
            request_id: self.id.clone(),
            profile: self.contract_profile(),
            acquisition_authority: self.authority.clone(),
            retry_policy: self.policy,
            next_attempt: ordinal,
        }
    }

    pub(super) fn restored_external(
        &self,
        endpoint: &str,
        ordinal: u32,
    ) -> RestoredExternalMacroRequest {
        RestoredExternalMacroRequest {
            endpoint_uri: endpoint.to_owned(),
            request: self.restored(ordinal),
        }
    }

    pub(crate) fn request_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(crate) fn request_id(&self) -> &str {
        &self.id
    }

    pub(crate) fn retry_policy(&self) -> (u32, u64, u64, u64) {
        self.policy
    }

    pub(super) fn contract_profile(&self) -> ContractProfile {
        self.checked_contract_profile()
            .expect("validated Macro request contract profile")
    }

    fn checked_contract_profile(&self) -> Result<ContractProfile> {
        match self.profile.as_str() {
            "LocalBridgeV1" => Ok(ContractProfile::LocalBridgeV1),
            "ExternalV1" => Ok(ContractProfile::ExternalV1),
            _ => Err(ChainPostCloseError::SchemaRejected),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Plan {
    version: u32,
    pub(super) parent_bytes: Vec<u8>,
    pub(super) parent_digest: String,
    pub(super) parent_version: u64,
    parent_owner: String,
    parent_generation: u64,
    parent_time: i64,
    pub(super) started: i64,
    pub(super) deadline: i64,
    observation: String,
    local_offset_seconds: i32,
    source_order: Vec<String>,
    news_limit: u32,
    economic_limit: u32,
    economic_country: Option<String>,
    pub(super) endpoint: String,
    pub(super) request: Request,
    queries: Vec<String>,
    decisions: Vec<ResearchDecision>,
    research_limit: usize,
    gateway_pace_ms: u64,
    query_pace_ms: u64,
    #[serde(skip)]
    identities: Vec<MacroQueryIdentity>,
    #[serde(skip)]
    providers: Vec<GeneralWebResearchProvider>,
}

impl Plan {
    pub(super) fn format_version(&self) -> u32 { self.version }

    pub(super) fn new(
        parent: &MacroParent,
        now: UtcMicros,
        observation: DateTime<FixedOffset>,
        endpoint: &str,
        request: Request,
        web: &MacroWebSnapshot,
    ) -> Result<Self> {
        let mut plan = Self {
            version: 2,
            parent_bytes: parent.bytes.clone(),
            parent_digest: parent.digest.clone(),
            parent_version: parent.version,
            parent_owner: parent.owner.clone(),
            parent_generation: parent.generation,
            parent_time: parent.applied_at,
            started: now.get(),
            deadline: now
                .get()
                .checked_add(15_000_000)
                .ok_or(ChainPostCloseError::SchemaRejected)?,
            observation: observation.to_rfc3339(),
            local_offset_seconds: observation.offset().local_minus_utc(),
            source_order: [
                "Eastmoney",
                "Cailianpress",
                "Jin10",
                "ThePaper",
                "EconomicCalendar",
            ]
            .map(str::to_owned)
            .to_vec(),
            news_limit: 20,
            economic_limit: 20,
            economic_country: None,
            endpoint: endpoint.to_owned(),
            request,
            queries: queries(observation),
            decisions: web
                .decisions()
                .iter()
                .map(|decision| {
                    let (available, availability_source, local_transport_endpoint) =
                        match decision.local_transport() {
                            LocalSemanticSearchConnectionState::Connected { endpoint } => (
                                true,
                                "explicit-registry-local-semantic-search-connected".to_owned(),
                                Some(endpoint.clone()),
                            ),
                            LocalSemanticSearchConnectionState::Disconnected => (
                                false,
                                "explicit-registry-local-semantic-search-disconnected".to_owned(),
                                None,
                            ),
                            LocalSemanticSearchConnectionState::Busy => unreachable!(
                                "busy Local SemanticSearch state is rejected before planning"
                            ),
                        };
                    ResearchDecision {
                        provider: decision.provider().wire_name().to_owned(),
                        supported: decision.supported(),
                        available,
                        availability_source,
                        registration_ordinal: Some(decision.registration_ordinal()),
                        local_transport_endpoint,
                        remote_health: Some("Unknown".to_owned()),
                    }
                })
                .collect(),
            research_limit: 3,
            gateway_pace_ms: 200,
            query_pace_ms: 300,
            identities: Vec::new(),
            providers: Vec::new(),
        };
        plan.validate(parent)?;
        Ok(plan)
    }

    pub(super) fn validate(&mut self, parent: &MacroParent) -> Result<()> {
        self.request.validate()?;
        let observed = DateTime::parse_from_rfc3339(&self.observation)
            .map_err(|_| ChainPostCloseError::SchemaRejected)?;
        require(
            matches!(self.version, 1 | 2)
                && self.parent_bytes == parent.bytes
                && self.parent_digest == parent.digest
                && raw_digest(&self.parent_bytes).as_str() == self.parent_digest
                && self.parent_version == parent.version
                && self.parent_owner == parent.owner
                && self.parent_generation == parent.generation
                && self.parent_time == parent.applied_at
                && self.started >= self.parent_time
                && self.deadline.checked_sub(self.started) == Some(15_000_000)
                && observed.to_rfc3339() == self.observation
                && observed.offset().local_minus_utc() == self.local_offset_seconds
                && self.queries == queries(observed)
                && self.news_limit == 20
                && self.economic_limit == 20
                && self.economic_country.is_none()
                && self.research_limit == 3
                && self.gateway_pace_ms == 200
                && self.query_pace_ms == 300
                && self.source_order
                    == [
                        "Eastmoney",
                        "Cailianpress",
                        "Jin10",
                        "ThePaper",
                        "EconomicCalendar",
                    ]
                && self
                    .endpoint
                    .starts_with(if self.request.profile == "ExternalV1" {
                        "https://"
                    } else {
                        "http://"
                    })
                && !self.endpoint.contains(['@', '?', '#'])
                && self.endpoint.parse::<tonic::codegen::http::Uri>().is_ok(),
        )?;
        self.validate_research_decisions()?;
        self.identities = source_identities();
        Ok(())
    }

    fn validate_research_decisions(&mut self) -> Result<()> {
        self.providers.clear();
        let mut last_registration_ordinal = 0;
        let mut explicit_local_transport = None;
        for decision in &self.decisions {
            let provider = GeneralWebResearchProvider::from_wire_name(&decision.provider)
                .ok_or(ChainPostCloseError::SchemaRejected)?;
            require(!self.providers.contains(&provider))?;
            match self.version {
                1 => require(
                    decision.supported
                        && decision.available
                        && decision.availability_source == "instance-connected-local-bridge"
                        && decision.registration_ordinal.is_none()
                        && decision.local_transport_endpoint.is_none()
                        && decision.remote_health.is_none(),
                )?,
                2 => {
                    let ordinal = decision
                        .registration_ordinal
                        .ok_or(ChainPostCloseError::SchemaRejected)?;
                    require(
                        ordinal > last_registration_ordinal
                            && decision.remote_health.as_deref() == Some("Unknown"),
                    )?;
                    let local_transport = (
                        decision.availability_source.as_str(),
                        decision.local_transport_endpoint.as_deref(),
                    );
                    if let Some(expected) = explicit_local_transport {
                        require(local_transport == expected)?;
                    } else {
                        explicit_local_transport = Some(local_transport);
                    }
                    match decision.availability_source.as_str() {
                        "explicit-registry-local-semantic-search-connected" => {
                            let endpoint = decision
                                .local_transport_endpoint
                                .as_deref()
                                .ok_or(ChainPostCloseError::SchemaRejected)?;
                            require(
                                decision.available
                                    && match self.request.contract_profile() {
                                        ContractProfile::LocalBridgeV1 => endpoint == self.endpoint,
                                        ContractProfile::ExternalV1 => {
                                            endpoint.starts_with("http://")
                                        }
                                    }
                                    && !endpoint.contains(['@', '?', '#'])
                                    && endpoint.parse::<tonic::codegen::http::Uri>().is_ok(),
                            )?;
                        }
                        "explicit-registry-local-semantic-search-disconnected" => require(
                            !decision.available && decision.local_transport_endpoint.is_none(),
                        )?,
                        _ => return Err(ChainPostCloseError::SchemaRejected),
                    }
                    last_registration_ordinal = ordinal;
                }
                _ => return Err(ChainPostCloseError::SchemaRejected),
            }
            self.providers.push(provider);
        }
        Ok(())
    }

    pub(crate) fn started_at(&self) -> UtcMicros {
        UtcMicros::try_new(self.started).expect("validated Macro timestamp")
    }
    pub(crate) fn deadline_at(&self) -> UtcMicros {
        UtcMicros::try_new(self.deadline).expect("validated Macro deadline")
    }
    pub(crate) fn observed_local(&self) -> &str {
        &self.observation
    }
    pub(crate) fn profile(&self) -> ContractProfile {
        self.request.contract_profile()
    }
    pub(crate) fn acquisition_authority(&self) -> Option<&str> {
        self.request.authority.as_deref()
    }
    pub(crate) fn endpoint(&self) -> &str {
        &self.endpoint
    }
    pub(crate) fn first_source_request(&self) -> &Request {
        &self.request
    }
    pub(crate) fn source_identities(&self) -> &[MacroQueryIdentity] {
        &self.identities
    }
    pub(crate) fn economic_intent(&self) -> (u32, Option<&str>) {
        (self.economic_limit, self.economic_country.as_deref())
    }
    pub(crate) fn research_queries(&self) -> &[String] {
        &self.queries
    }
    pub(crate) fn research_providers(&self) -> &[GeneralWebResearchProvider] {
        &self.providers
    }
    pub(crate) fn research_decisions(&self) -> &[ResearchDecision] {
        &self.decisions
    }
    pub(crate) fn research_decision_provenance(&self) -> ResearchDecisionProvenance {
        if self.version == 1 {
            ResearchDecisionProvenance::LegacySuppliedConnectedLocalBridgeV1
        } else {
            ResearchDecisionProvenance::ExplicitRegistryV2
        }
    }
    pub(crate) fn research_limit(&self) -> usize {
        self.research_limit
    }
    pub(crate) fn gateway_pace_ms(&self) -> u64 {
        self.gateway_pace_ms
    }
    pub(crate) fn query_pace_ms(&self) -> u64 {
        self.query_pace_ms
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ControlRequest {
    version: u32,
    kind: String,
    bytes: Vec<u8>,
    id: String,
    profile: String,
    endpoint: String,
    authority: String,
}

impl ControlRequest {
    pub(super) fn capture(material: ExternalControlRequestMaterial) -> Result<Self> {
        let value = Self {
            version: 1,
            kind: match material.kind {
                ExternalControlKind::Health => "Health",
                ExternalControlKind::Capabilities => "Capabilities",
            }
            .to_owned(),
            bytes: material.request_bytes,
            id: material.request_id,
            profile: match material.profile {
                ContractProfile::LocalBridgeV1 => "LocalBridgeV1",
                ContractProfile::ExternalV1 => "ExternalV1",
            }
            .to_owned(),
            endpoint: material.endpoint_uri,
            authority: material.acquisition_authority,
        };
        value.validate()?;
        Ok(value)
    }

    pub(super) fn validate(&self) -> Result<()> {
        require(
            self.version == 1
                && self.profile == "ExternalV1"
                && self.endpoint.starts_with("https://")
                && self.endpoint.parse::<tonic::codegen::http::Uri>().is_ok()
                && !self.endpoint.contains(['@', '?', '#'])
                && self.authority.starts_with("grpc-mtls:")
                && !self.id.is_empty()
                && self.id.len() <= 512,
        )?;
        let context = match self.kind.as_str() {
            "Health" => {
                let request = HealthRequest::decode(self.bytes.as_slice())
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?;
                require(request.encode_to_vec() == self.bytes)?;
                request.context
            }
            "Capabilities" => {
                let request = CapabilitiesRequest::decode(self.bytes.as_slice())
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?;
                require(request.encode_to_vec() == self.bytes)?;
                request.context
            }
            _ => return Err(ChainPostCloseError::SchemaRejected),
        }
        .ok_or(ChainPostCloseError::SchemaRejected)?;
        require(context.protocol_version == 1 && context.request_id == self.id)
    }

    pub(super) fn material(&self) -> ExternalControlRequestMaterial {
        ExternalControlRequestMaterial {
            kind: self.kind(),
            request_bytes: self.bytes.clone(),
            request_id: self.id.clone(),
            profile: ContractProfile::ExternalV1,
            endpoint_uri: self.endpoint.clone(),
            acquisition_authority: self.authority.clone(),
        }
    }

    pub(crate) fn kind(&self) -> ExternalControlKind {
        if self.kind == "Health" {
            ExternalControlKind::Health
        } else {
            ExternalControlKind::Capabilities
        }
    }

    pub(crate) fn request_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(crate) fn request_id(&self) -> &str {
        &self.id
    }

    pub(super) fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub(super) fn authority(&self) -> &str {
        &self.authority
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReadinessEpisodePlan {
    version: u32,
    episode_ordinal: u32,
    phase: String,
    item_ordinal: u32,
    candidate_ordinal: u32,
    required_operation: i32,
    health: ControlRequest,
    capabilities: ControlRequest,
}

impl ReadinessEpisodePlan {
    pub(super) fn new(
        health: ExternalControlRequestMaterial,
        capabilities: ExternalControlRequestMaterial,
    ) -> Result<Self> {
        let value = Self {
            version: 1,
            episode_ordinal: 1,
            phase: "Gateway".to_owned(),
            item_ordinal: 1,
            candidate_ordinal: 1,
            required_operation: ExternalOperation::GlobalNews as i32,
            health: ControlRequest::capture(health)?,
            capabilities: ControlRequest::capture(capabilities)?,
        };
        value.validate()?;
        Ok(value)
    }

    pub(super) fn validate(&self) -> Result<()> {
        self.health.validate()?;
        self.capabilities.validate()?;
        require(
            self.version == 1
                && self.episode_ordinal == 1
                && self.phase == "Gateway"
                && self.item_ordinal == 1
                && self.candidate_ordinal == 1
                && self.required_operation == ExternalOperation::GlobalNews as i32
                && self.health.kind() == ExternalControlKind::Health
                && self.capabilities.kind() == ExternalControlKind::Capabilities
                && self.health.endpoint == self.capabilities.endpoint
                && self.health.authority == self.capabilities.authority
                && self.health.id != self.capabilities.id,
        )
    }

    pub(super) fn controls(&self) -> [&ControlRequest; 2] {
        [&self.health, &self.capabilities]
    }

    pub(crate) fn episode_ordinal(&self) -> u32 {
        self.episode_ordinal
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ControlRawResult {
    version: u32,
    connect_unavailable: bool,
    response: Option<Vec<u8>>,
    code: Option<i32>,
    details: Option<Vec<u8>>,
    trailer: Trailer,
    diagnostic: Option<String>,
}

impl ControlRawResult {
    pub(super) fn capture_health(completion: &ExternalControlCompletion<HealthResponse>) -> Self {
        Self::capture(completion.result_material())
    }

    pub(super) fn capture_capabilities(
        completion: &ExternalControlCompletion<CapabilitiesResponse>,
    ) -> Self {
        Self::capture(completion.result_material())
    }

    fn capture<T>(material: ExternalControlResultMaterial<'_, T>) -> Self {
        match material {
            ExternalControlResultMaterial::ConnectUnavailable { error } => Self {
                version: 1,
                connect_unavailable: true,
                response: None,
                code: None,
                details: None,
                trailer: Trailer::Absent,
                diagnostic: error.safe_diagnostic().map(str::to_owned),
            },
            ExternalControlResultMaterial::Response { bytes, .. } => Self {
                version: 1,
                connect_unavailable: false,
                response: Some(bytes.to_vec()),
                code: None,
                details: None,
                trailer: Trailer::Absent,
                diagnostic: None,
            },
            ExternalControlResultMaterial::Status {
                code,
                details,
                error_detail_trailer,
                error,
            } => Self {
                version: 1,
                connect_unavailable: false,
                response: None,
                code: Some(code),
                details: Some(details.to_vec()),
                trailer: match error_detail_trailer {
                    MacroTrailerMaterial::Absent => Trailer::Absent,
                    MacroTrailerMaterial::Bytes(bytes) => Trailer::Bytes(bytes.clone()),
                    MacroTrailerMaterial::Malformed => Trailer::Malformed,
                },
                diagnostic: error.safe_diagnostic().map(str::to_owned),
            },
        }
    }

    pub(super) fn project(
        &self,
        request: &ControlRequest,
    ) -> Result<std::result::Result<(), GatewayError>> {
        require(self.version == 1)?;
        request.validate()?;
        match (
            self.connect_unavailable,
            &self.response,
            self.code,
            &self.details,
        ) {
            (true, None, None, None) => {
                require(matches!(self.trailer, Trailer::Absent))?;
                let error = GrpcError::Unavailable {
                    details: Box::default(),
                };
                require(self.diagnostic.as_deref() == error.safe_diagnostic())?;
                Ok(Err(
                    crate::data_gateway::grpc_source::map_external_connection_error(error),
                ))
            }
            (false, Some(bytes), None, None) => {
                require(matches!(self.trailer, Trailer::Absent))?;
                match request.kind() {
                    ExternalControlKind::Health => {
                        let response = HealthResponse::decode(bytes.as_slice())
                            .map_err(|_| ChainPostCloseError::SchemaRejected)?;
                        require(response.encode_to_vec() == *bytes)?;
                        if let Err(error) =
                            validate_health_response_id(request.request_id(), &response)
                        {
                            require(self.diagnostic.as_deref() == error.safe_diagnostic())?;
                            return Ok(Err(
                                crate::data_gateway::grpc_source::map_external_connection_error(
                                    error,
                                ),
                            ));
                        }
                        require(self.diagnostic.is_none())?;
                        Ok(
                            crate::data_gateway::grpc_source::require_external_health_ready(
                                &response,
                            ),
                        )
                    }
                    ExternalControlKind::Capabilities => {
                        let response = CapabilitiesResponse::decode(bytes.as_slice())
                            .map_err(|_| ChainPostCloseError::SchemaRejected)?;
                        require(response.encode_to_vec() == *bytes)?;
                        if let Err(error) =
                            validate_capabilities_response_id(request.request_id(), &response)
                        {
                            require(self.diagnostic.as_deref() == error.safe_diagnostic())?;
                            return Ok(Err(
                                crate::data_gateway::grpc_source::map_external_connection_error(
                                    error,
                                ),
                            ));
                        }
                        require(self.diagnostic.is_none())?;
                        Ok(
                            crate::data_gateway::grpc_source::require_external_capability(
                                &response.capabilities,
                                external_global_news(),
                            ),
                        )
                    }
                }
            }
            (false, None, Some(code), Some(details)) => {
                let trailer = match &self.trailer {
                    Trailer::Absent => PersistedErrorDetailTrailer::Absent,
                    Trailer::Bytes(bytes) => PersistedErrorDetailTrailer::Bytes(bytes),
                    Trailer::Malformed => PersistedErrorDetailTrailer::Malformed,
                };
                let error = restore_persisted_status_error(
                    code,
                    details,
                    trailer,
                    self.diagnostic.as_deref(),
                    StatusErrorContext::control(
                        ContractProfile::ExternalV1,
                        request.request_id(),
                    ),
                )
                .ok_or(ChainPostCloseError::SchemaRejected)?;
                require(self.diagnostic.as_deref() == error.safe_diagnostic())?;
                Ok(Err(
                    crate::data_gateway::grpc_source::map_external_connection_error(error),
                ))
            }
            _ => return Err(ChainPostCloseError::SchemaRejected),
        }
    }

    pub(super) fn validated_provider_catalog(
        &self,
        request: &ControlRequest,
    ) -> Result<ExternalProviderCatalog> {
        require(matches!(
            request.kind(),
            ExternalControlKind::Capabilities
        ))?;
        require(self.project(request)?.is_ok())?;
        let bytes = self
            .response
            .as_deref()
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        let response = CapabilitiesResponse::decode(bytes)
            .map_err(|_| ChainPostCloseError::SchemaRejected)?;
        require(response.encode_to_vec().as_slice() == bytes)?;
        validated_external_provider_catalog(request.request_id(), &response)
            .map_err(|_| ChainPostCloseError::SchemaRejected)
    }

    pub(super) fn response_bytes(&self) -> Option<&[u8]> {
        self.response.as_deref()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) enum Trailer {
    Absent,
    Bytes(Vec<u8>),
    Malformed,
}

pub(super) enum RecoveredWire {
    ConnectUnavailable,
    Response(Vec<u8>),
    Status {
        code: i32,
        details: Vec<u8>,
        trailer: Trailer,
    },
    LocalWireFailure,
}

pub(super) struct RecoveredResult {
    pub(super) wire: RecoveredWire,
    pub(super) diagnostic: Option<String>,
    pub(super) retry_decision: RetryDecision,
    pub(super) continuation: MacroContinuation,
    pub(super) provider_attempts: Option<ProviderAttempts>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawResult {
    version: u32,
    connect_unavailable: bool,
    pub(super) response: Option<Vec<u8>>,
    code: Option<i32>,
    details: Option<Vec<u8>>,
    trailer: Trailer,
    diagnostic: Option<String>,
    decision: String,
    backoff_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    external_wire: Option<ExternalWireEvidenceV1>,
}

impl RawResult {
    pub(super) fn capture(completion: &MacroAttemptCompletion) -> Self {
        Self {
            version: 1,
            connect_unavailable: false,
            response: completion.response_bytes.clone(),
            code: completion.status_code,
            details: completion.status_details.clone(),
            trailer: match &completion.status_error_detail_trailer {
                MacroTrailerMaterial::Absent => Trailer::Absent,
                MacroTrailerMaterial::Bytes(bytes) => Trailer::Bytes(bytes.clone()),
                MacroTrailerMaterial::Malformed => Trailer::Malformed,
            },
            diagnostic: completion
                .processed
                .as_ref()
                .err()
                .and_then(|error| error.safe_diagnostic())
                .map(str::to_owned),
            decision: format!("{:?}", completion.retry_decision),
            backoff_ms: match completion.continuation {
                MacroContinuation::Retry { backoff_ms } => Some(backoff_ms),
                MacroContinuation::Terminal => None,
            },
            external_wire: completion.external_wire.clone(),
        }
    }

    pub(super) fn capture_external(completion: &ExternalMacroAttemptCompletion) -> Self {
        match completion {
            ExternalMacroAttemptCompletion::Unary(completion) => {
                let mut captured = Self::capture(completion);
                if captured.external_wire.is_some() {
                    captured.version = 2;
                }
                captured
            }
            ExternalMacroAttemptCompletion::ConnectUnavailable {
                error,
                retry_decision,
                continuation,
            } => Self {
                version: 1,
                connect_unavailable: true,
                response: None,
                code: None,
                details: None,
                trailer: Trailer::Absent,
                diagnostic: error.safe_diagnostic().map(str::to_owned),
                decision: format!("{retry_decision:?}"),
                backoff_ms: match continuation {
                    MacroContinuation::Retry { backoff_ms } => Some(*backoff_ms),
                    MacroContinuation::Terminal => None,
                },
                external_wire: None,
            },
        }
    }

    pub(super) fn continuation(&self) -> MacroContinuation {
        match self.backoff_ms {
            Some(backoff_ms) => MacroContinuation::Retry { backoff_ms },
            None => MacroContinuation::Terminal,
        }
    }

    pub(super) fn project(
        &self,
        request: &Request,
        ordinal: u32,
        provider_catalog: Option<&ExternalProviderCatalog>,
    ) -> Result<(NewsResult, RetryDecision, Option<ProviderAttempts>)> {
        let (processed, decision, provider_attempts) =
            self.project_for(&first_identity(), request, ordinal, provider_catalog)?;
        Ok((
            gateway_for(request.checked_contract_profile()?, &processed),
            decision,
            provider_attempts,
        ))
    }

    pub(super) fn project_for(
        &self,
        identity: &MacroQueryIdentity,
        request: &Request,
        ordinal: u32,
        provider_catalog: Option<&ExternalProviderCatalog>,
    ) -> Result<(
        std::result::Result<crate::grpc_client::envelope::QueryResult, GrpcError>,
        RetryDecision,
        Option<ProviderAttempts>,
    )> {
        require(matches!(self.version, 1 | 2) && ordinal > 0 && ordinal <= request.policy.0)?;
        request.validate_for(identity)?;
        let profile = request.checked_contract_profile()?;
        let method = identity
            .method(profile)
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        if self.version == 2 {
            if self.code.is_none() {
                let (processed, decision) = self.project_external_v2(identity, request, ordinal)?;
                return Ok((processed, decision, None));
            }
            // A remote status on the External wire keeps the captured body
            // material alongside the status; the status itself is projected
            // exactly like a v1 status below.
            self.checked_external_evidence(request, identity)?;
            require(
                !self.connect_unavailable && self.response.is_none() && self.details.is_some(),
            )?;
        } else {
            require(self.external_wire.is_none())?;
        }
        let (processed, decision) = match (
            self.connect_unavailable,
            &self.response,
            self.code,
            &self.details,
        ) {
            (false, Some(bytes), None, None) => {
                require(
                    matches!(self.trailer, Trailer::Absent)
                        && self.backoff_ms.is_none()
                        && self.decision == "NoRetry",
                )?;
                let response = QueryResponse::decode(bytes.as_slice())
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?;
                require(response.encode_to_vec() == *bytes)?;
                (
                    project_macro_response(
                        identity,
                        profile,
                        request.authority.as_deref(),
                        &request.id,
                        response,
                    ),
                    RetryDecision::NoRetry,
                )
            }
            (false, None, Some(code), Some(details)) => {
                let trailer = match &self.trailer {
                    Trailer::Absent => PersistedErrorDetailTrailer::Absent,
                    Trailer::Bytes(bytes) => PersistedErrorDetailTrailer::Bytes(bytes),
                    Trailer::Malformed => PersistedErrorDetailTrailer::Malformed,
                };
                let error = restore_persisted_status_error(
                    code,
                    details,
                    trailer,
                    self.diagnostic.as_deref(),
                    match (profile, method, provider_catalog) {
                        (
                            ContractProfile::ExternalV1,
                            MethodIdentity::External(external_method),
                            Some(catalog),
                        ) => StatusErrorContext::external_data(
                            external_method,
                            &request.id,
                            catalog,
                        ),
                        (_, method, _) => StatusErrorContext::data(method, &request.id),
                    },
                )
                .ok_or(ChainPostCloseError::SchemaRejected)?;
                let decision = retry_decision(&error);
                require(self.decision == format!("{decision:?}"))?;
                validate_retry_backoff(request, ordinal, decision, self.backoff_ms)?;
                (Err(error), decision)
            }
            (true, None, None, None) => {
                require(matches!(self.trailer, Trailer::Absent))?;
                let error = GrpcError::Unavailable {
                    details: Box::default(),
                };
                let decision = retry_decision(&error);
                require(self.decision == format!("{decision:?}"))?;
                validate_retry_backoff(request, ordinal, decision, self.backoff_ms)?;
                (Err(error), decision)
            }
            _ => return Err(ChainPostCloseError::SchemaRejected),
        };
        require(
            self.diagnostic.as_deref()
                == processed
                    .as_ref()
                    .err()
                    .and_then(|error| error.safe_diagnostic()),
        )?;
        let provider_attempts = if profile == ContractProfile::ExternalV1 && self.code.is_some() {
            processed.as_ref().err().and_then(|error| {
                let attempts = &error.details().provider_attempts;
                (!attempts
                    .accepted()
                    .is_some_and(|accepted| accepted.is_empty()))
                .then(|| attempts.clone())
            })
        } else {
            None
        };
        Ok((processed, decision, provider_attempts))
    }

    fn checked_external_evidence(
        &self,
        request: &Request,
        identity: &MacroQueryIdentity,
    ) -> Result<&ExternalWireEvidenceV1> {
        require(
            request.checked_contract_profile()? == ContractProfile::ExternalV1
                && matches!(identity, MacroQueryIdentity::GlobalNews { .. }),
        )?;
        let evidence = self
            .external_wire
            .as_ref()
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        evidence
            .validate(ExternalQueryMethod::GlobalNews)
            .map_err(|_| ChainPostCloseError::SchemaRejected)?;
        Ok(evidence)
    }

    fn project_external_v2(
        &self,
        identity: &MacroQueryIdentity,
        request: &Request,
        _ordinal: u32,
    ) -> Result<(
        std::result::Result<crate::grpc_client::envelope::QueryResult, GrpcError>,
        RetryDecision,
    )> {
        require(
            !self.connect_unavailable
                && self.code.is_none()
                && self.details.is_none()
                && matches!(self.trailer, Trailer::Absent)
                && self.decision == "NoRetry"
                && self.backoff_ms.is_none(),
        )?;
        let evidence = self.checked_external_evidence(request, identity)?;
        let processed = match &evidence.evidence {
            ExternalWireMaterialV1::Payload {
                protobuf_payload, ..
            } => {
                require(self.response.as_deref() == Some(protobuf_payload.as_slice()))?;
                admit_external_payload(protobuf_payload).and_then(|()| {
                    let response = ExternalQueryResponse::decode(protobuf_payload.as_slice())
                        .map_err(|_| wire_error("external_response_wire_invalid"))?;
                    crate::grpc_client::envelope::parse_external_query_response(
                        &request.id,
                        Operation::GlobalNews,
                        request.authority.as_deref().ok_or_else(|| {
                            wire_error("external_acquisition_authority_missing")
                        })?,
                        response,
                    )
                    .map_err(GrpcError::from)
                })
            }
            ExternalWireMaterialV1::Missing { .. }
            | ExternalWireMaterialV1::Overflow { .. }
            | ExternalWireMaterialV1::InvalidFrame { .. } => {
                require(self.response.is_none())?;
                Err(wire_error("external_response_wire_invalid"))
            }
        };
        require(
            self.diagnostic.as_deref()
                == processed
                    .as_ref()
                    .err()
                    .and_then(|error| error.safe_diagnostic()),
        )?;
        Ok((processed, RetryDecision::NoRetry))
    }

    pub(super) fn into_recovered(
        self,
        retry_decision: RetryDecision,
        provider_attempts: Option<ProviderAttempts>,
    ) -> Result<RecoveredResult> {
        let continuation = self.continuation();
        let wire = match (
            self.connect_unavailable,
            self.response,
            self.code,
            self.details,
        ) {
            (false, Some(response), None, None) => RecoveredWire::Response(response),
            (false, None, Some(code), Some(details)) => RecoveredWire::Status {
                code,
                details,
                trailer: self.trailer,
            },
            (true, None, None, None) => RecoveredWire::ConnectUnavailable,
            (false, None, None, None) if self.version == 2 && self.external_wire.is_some() => {
                RecoveredWire::LocalWireFailure
            }
            _ => return Err(ChainPostCloseError::SchemaRejected),
        };
        Ok(RecoveredResult {
            wire,
            diagnostic: self.diagnostic,
            retry_decision,
            continuation,
            provider_attempts,
        })
    }
}

fn validate_retry_backoff(
    request: &Request,
    ordinal: u32,
    decision: RetryDecision,
    actual: Option<u64>,
) -> Result<()> {
    let expected = if matches!(
        decision,
        RetryDecision::RetryBackoff | RetryDecision::RetryBounded
    ) && ordinal < request.policy.0
    {
        let policy = RetryPolicy {
            max_attempts: request.policy.0,
            base_delay_ms: request.policy.1,
            max_delay_ms: request.policy.2,
            jitter_ms: request.policy.3,
        };
        Some(
            u64::try_from(policy.backoff(ordinal).as_millis())
                .map_err(|_| ChainPostCloseError::SchemaRejected)?,
        )
    } else {
        None
    };
    require(actual == expected)
}

pub(super) fn gateway_for(
    profile: ContractProfile,
    processed: &std::result::Result<crate::grpc_client::envelope::QueryResult, GrpcError>,
) -> NewsResult {
    match processed {
        Ok(query) => GrpcSource::global_news_query_result(
            GlobalNewsProvider::Eastmoney,
            20,
            profile == ContractProfile::ExternalV1,
            query,
        ),
        Err(error) => Err(match profile {
            ContractProfile::LocalBridgeV1 => map_macro_error(error),
            ContractProfile::ExternalV1 => map_external_query_error(Operation::GlobalNews, error),
        }),
    }
}

/// All native fields are saved before any rendering. Reader reprojects the raw
/// response through the real adapter and compares these exact canonical bytes.
pub(super) fn native_bytes(result: &NewsResult) -> Result<Vec<u8>> {
    let value = match result {
        Err(error) => {
            serde_json::json!({"version":1,"kind":"Error","error":store_gateway_error(error)})
        }
        Ok(batch) => {
            let ev = batch.evidence();
            let records: Vec<_> = batch.records().iter().map(|record| serde_json::json!({
                "item_id":record.item_id,"title":record.title,"summary":record.summary,"content":record.content,
                "publisher":record.publisher,"canonical_url":record.canonical_url,
                "published_at":record.published_at.to_rfc3339(),"observed_at":record.observed_at.to_rfc3339(),
                "instruments":record.instruments,"topics":record.topics,"language":record.language,"evidence":record.evidence,
            })).collect();
            serde_json::json!({"version":1,"kind":if batch.is_verified_empty(){"VerifiedEmpty"}else{"Available"},
                "evidence":{"provider":ev.provider,"source":ev.source,"source_at":ev.source_at,
                    "observed_at":ev.observed_at,"batch_id":ev.batch_id},"records":records})
        }
    };
    encode(&value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grpc_client::external_query_transport::ExternalFrameFailureV1;
    use sha2::{Digest as _, Sha256};

    fn external_v2_material(
        source11: &[u8],
    ) -> (
        MacroQueryIdentity,
        Request,
        ExternalMacroAttemptCompletion,
    ) {
        let identity = MacroQueryIdentity::GlobalNews {
            provider: GlobalNewsProvider::Eastmoney,
            limit: 20,
        };
        let mut native_request = crate::grpc_client::external_v1::build_external_query_request(
            Operation::GlobalNews,
            serde_json::json!({"provider":"Eastmoney","limit":20}),
        )
        .unwrap();
        native_request.context.as_mut().unwrap().request_id = "TEST_CODE_S2_V2".to_owned();
        let request = Request {
            bytes: native_request.encode_to_vec(),
            id: "TEST_CODE_S2_V2".to_owned(),
            policy: (3, 1, 2, 0),
            profile: "ExternalV1".to_owned(),
            authority: Some("grpc-mtls:TEST_CODE_s2.invalid".to_owned()),
        };
        let response = ExternalQueryResponse {
            request_id: request.id.clone(),
            operation: ExternalOperation::GlobalNews as i32,
            admission:
                crate::grpc_client::external_pb::magic::market::v1::AdmissionState::Admitted
                    as i32,
            selected_provider: "Eastmoney".to_owned(),
            batch_id: "TEST_CODE_S2_BATCH".to_owned(),
            complete: true,
            observed_at: "2026-09-17T01:00:00Z".to_owned(),
            source_at: "2026-09-17T00:59:00Z".to_owned(),
            records: Vec::new(),
            diagnostic_blocker: String::new(),
        };
        let mut payload = response.encode_to_vec();
        payload.extend_from_slice(source11);
        let evidence = ExternalWireEvidenceV1 {
            material: "external-unary-response-evidence-v1".to_owned(),
            profile: "ExternalV1".to_owned(),
            method: ExternalQueryMethod::GlobalNews,
            client_descriptor_sha256:
                crate::grpc_client::external_query_transport::EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256
                    .to_owned(),
            evidence: ExternalWireMaterialV1::Payload {
                protobuf_payload: payload.clone(),
                payload_sha256: hex::encode(Sha256::digest(&payload)),
                decode_limit_bytes: crate::grpc_client::external_query_transport::EXTERNAL_QUERY_DECODE_LIMIT_BYTES,
            },
        };
        let processed = admit_external_payload(&payload).and_then(|()| {
            crate::grpc_client::envelope::parse_external_query_response(
                &request.id,
                Operation::GlobalNews,
                request.authority.as_deref().unwrap(),
                response,
            )
            .map_err(GrpcError::from)
        });
        let completion = ExternalMacroAttemptCompletion::Unary(MacroAttemptCompletion {
            response_bytes: Some(payload),
            status_code: None,
            status_details: None,
            status_error_detail_trailer: MacroTrailerMaterial::Absent,
            processed,
            retry_decision: RetryDecision::NoRetry,
            continuation: MacroContinuation::Terminal,
            external_wire: Some(evidence),
        });
        (identity, request, completion)
    }

    #[test]
    fn raw_result_v2_restores_exact_external_payload_with_zero_length_source11() {
        let (identity, request, completion) = external_v2_material(&[0x5a, 0x00]);
        let raw = RawResult::capture_external(&completion);
        assert_eq!(raw.version, 2);
        assert!(raw.response.as_ref().unwrap().ends_with(&[0x5a, 0x00]));
        let (processed, decision, _) = raw.project_for(&identity, &request, 1, None).unwrap();
        assert_eq!(decision, RetryDecision::NoRetry);
        assert_eq!(
            processed.unwrap().source(),
            "grpc-mtls:TEST_CODE_s2.invalid"
        );
    }

    #[test]
    fn raw_result_v2_replays_nonempty_source11_as_local_conflict_without_status() {
        let (identity, request, completion) = external_v2_material(&[0x5a, 0x01, b'x']);
        let raw = RawResult::capture_external(&completion);
        assert_eq!(raw.version, 2);
        assert!(raw.code.is_none() && raw.details.is_none());
        let (processed, decision, _) = raw.project_for(&identity, &request, 1, None).unwrap();
        assert_eq!(decision, RetryDecision::NoRetry);
        assert_eq!(
            processed.unwrap_err().details().code,
            "external_source_field_conflict"
        );
    }

    #[test]
    fn raw_result_v2_rejects_descriptor_contract_drift_before_projection() {
        let (identity, request, completion) = external_v2_material(&[0x5a, 0x00]);
        let mut raw = RawResult::capture_external(&completion);
        raw.external_wire
            .as_mut()
            .unwrap()
            .client_descriptor_sha256 = "0".repeat(64);
        assert!(matches!(
            raw.project_for(&identity, &request, 1, None),
            Err(ChainPostCloseError::SchemaRejected)
        ));
    }

    #[test]
    fn raw_result_v2_rejects_each_closed_payload_binding_mutation() {
        fn assert_rejected(raw: &RawResult, identity: &MacroQueryIdentity, request: &Request) {
            assert!(matches!(
                raw.project_for(identity, request, 1, None),
                Err(ChainPostCloseError::SchemaRejected)
            ));
        }

        let fresh = || {
            let (identity, request, completion) =
                external_v2_material(&[0x5a, 0x00]);
            (identity, request, RawResult::capture_external(&completion))
        };

        let (identity, request, mut raw) = fresh();
        raw.external_wire.as_mut().unwrap().material = "TEST_CODE_wrong".to_owned();
        assert_rejected(&raw, &identity, &request);

        let (identity, request, mut raw) = fresh();
        raw.external_wire.as_mut().unwrap().profile = "LocalBridgeV1".to_owned();
        assert_rejected(&raw, &identity, &request);

        let (identity, request, mut raw) = fresh();
        raw.external_wire.as_mut().unwrap().method = ExternalQueryMethod::SecurityMetadata;
        assert_rejected(&raw, &identity, &request);

        let (identity, request, mut raw) = fresh();
        raw.external_wire
            .as_mut()
            .unwrap()
            .client_descriptor_sha256 = "0".repeat(64);
        assert_rejected(&raw, &identity, &request);

        let (identity, request, mut raw) = fresh();
        let ExternalWireMaterialV1::Payload { payload_sha256, .. } =
            &mut raw.external_wire.as_mut().unwrap().evidence
        else {
            panic!("TEST_CODE expected payload evidence");
        };
        *payload_sha256 = "0".repeat(64);
        assert_rejected(&raw, &identity, &request);

        let (identity, request, mut raw) = fresh();
        let ExternalWireMaterialV1::Payload { protobuf_payload, .. } =
            &mut raw.external_wire.as_mut().unwrap().evidence
        else {
            panic!("TEST_CODE expected payload evidence");
        };
        protobuf_payload.push(0);
        assert_rejected(&raw, &identity, &request);

        let (identity, request, mut raw) = fresh();
        let ExternalWireMaterialV1::Payload {
            decode_limit_bytes, ..
        } = &mut raw.external_wire.as_mut().unwrap().evidence
        else {
            panic!("TEST_CODE expected payload evidence");
        };
        *decode_limit_bytes -= 1;
        assert_rejected(&raw, &identity, &request);

        let (identity, request, mut raw) = fresh();
        raw.response.as_mut().unwrap().push(0);
        assert_rejected(&raw, &identity, &request);
    }

    #[test]
    fn raw_result_v2_restores_capture_failure_without_response_or_status() {
        let (identity, request, _) = external_v2_material(&[0x5a, 0x00]);
        let evidence = ExternalWireEvidenceV1 {
            material: "external-unary-response-evidence-v1".to_owned(),
            profile: "ExternalV1".to_owned(),
            method: ExternalQueryMethod::GlobalNews,
            client_descriptor_sha256:
                crate::grpc_client::external_query_transport::EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256
                    .to_owned(),
            evidence: ExternalWireMaterialV1::Missing {
                framed_body_limit_bytes: crate::grpc_client::external_query_transport::EXTERNAL_QUERY_FRAMED_BODY_LIMIT_BYTES,
            },
        };
        let completion = ExternalMacroAttemptCompletion::Unary(MacroAttemptCompletion {
            response_bytes: None,
            status_code: None,
            status_details: None,
            status_error_detail_trailer: MacroTrailerMaterial::Absent,
            processed: Err(wire_error("external_response_wire_invalid")),
            retry_decision: RetryDecision::NoRetry,
            continuation: MacroContinuation::Terminal,
            external_wire: Some(evidence),
        });
        let raw = RawResult::capture_external(&completion);
        assert_eq!(raw.version, 2);
        assert!(raw.response.is_none() && raw.code.is_none() && raw.details.is_none());
        let (processed, decision, _) = raw.project_for(&identity, &request, 1, None).unwrap();
        assert_eq!(decision, RetryDecision::NoRetry);
        assert_eq!(
            processed.unwrap_err().details().code,
            "external_response_wire_invalid"
        );
        assert!(matches!(
            raw.into_recovered(RetryDecision::NoRetry, None).unwrap().wire,
            RecoveredWire::LocalWireFailure
        ));
    }

    fn local_v1_material() -> (MacroQueryIdentity, Request, RawResult) {
        let identity = MacroQueryIdentity::GlobalNews {
            provider: GlobalNewsProvider::Eastmoney,
            limit: 20,
        };
        let mut native_request = crate::grpc_client::envelope::build_query_request(
            Operation::GlobalNews,
            serde_json::json!({"provider":"Eastmoney","limit":20}),
        )
        .unwrap();
        native_request.context.as_mut().unwrap().request_id =
            "TEST_CODE_S2_V1_STRICT".to_owned();
        let request = Request {
            bytes: native_request.encode_to_vec(),
            id: "TEST_CODE_S2_V1_STRICT".to_owned(),
            policy: (3, 1, 2, 0),
            profile: "LocalBridgeV1".to_owned(),
            authority: None,
        };
        let response = QueryResponse {
            request_id: request.id.clone(),
            operation: Operation::GlobalNews as i32,
            admission:
                crate::grpc_client::pb::magic::market::v1::AdmissionState::Admitted as i32,
            selected_provider: "Eastmoney".to_owned(),
            batch_id: "TEST_CODE_S2_V1_BATCH".to_owned(),
            complete: true,
            observed_at: "2026-09-17T01:00:00Z".to_owned(),
            source_at: "2026-09-17T00:59:00Z".to_owned(),
            records: Vec::new(),
            source: "eastmoney-web".to_owned(),
            diagnostic_blocker: String::new(),
        };
        let completion = MacroAttemptCompletion {
            response_bytes: Some(response.encode_to_vec()),
            status_code: None,
            status_details: None,
            status_error_detail_trailer: MacroTrailerMaterial::Absent,
            processed: project_macro_response(
                &identity,
                ContractProfile::LocalBridgeV1,
                None,
                &request.id,
                response,
            ),
            retry_decision: RetryDecision::NoRetry,
            continuation: MacroContinuation::Terminal,
            external_wire: None,
        };
        (identity, request, RawResult::capture(&completion))
    }

    fn assert_v1_response_schema_rejected_without_id_drift(
        label: &str,
        raw: &RawResult,
        identity: &MacroQueryIdentity,
        request: &Request,
    ) {
        let bytes = raw.response.as_deref().expect("TEST_CODE V1 response");
        let decoded = QueryResponse::decode(bytes)
            .unwrap_or_else(|error| panic!("{label}: mutation must remain protobuf: {error}"));
        assert_eq!(decoded.request_id, request.id, "{label}: request ID drift");
        assert_ne!(
            decoded.encode_to_vec(),
            bytes,
            "{label}: mutation unexpectedly remained canonical"
        );
        assert!(
            matches!(
                raw.project_for(identity, request, 1, None),
                Err(ChainPostCloseError::SchemaRejected)
            ),
            "{label}: noncanonical V1 response was accepted"
        );
    }

    #[test]
    fn raw_result_v1_canonical_snapshot_rejects_unknown_duplicate_and_noncanonical_protobuf_without_id_drift(
    ) {
        let (identity, request, canonical) = local_v1_material();
        let (processed, decision, _) = canonical.project_for(&identity, &request, 1, None).unwrap();
        assert!(processed.is_ok());
        assert_eq!(decision, RetryDecision::NoRetry);

        let mut unknown = canonical.clone();
        unknown
            .response
            .as_mut()
            .unwrap()
            .extend_from_slice(&[0xf8, 0x07, 0x01]);
        assert_v1_response_schema_rejected_without_id_drift(
            "unknown response field",
            &unknown,
            &identity,
            &request,
        );

        let mut duplicate = canonical.clone();
        let duplicate_id = request.id.as_bytes();
        assert!(duplicate_id.len() < 128);
        duplicate.response.as_mut().unwrap().push(0x0a);
        duplicate
            .response
            .as_mut()
            .unwrap()
            .push(duplicate_id.len() as u8);
        duplicate
            .response
            .as_mut()
            .unwrap()
            .extend_from_slice(duplicate_id);
        assert_v1_response_schema_rejected_without_id_drift(
            "duplicate request_id field",
            &duplicate,
            &identity,
            &request,
        );

        let mut reordered = canonical.clone();
        let canonical_response = reordered.response.as_ref().unwrap();
        assert_eq!(canonical_response[0], 0x0a);
        assert!(canonical_response[1] < 0x80);
        let first_field_end = 2 + usize::from(canonical_response[1]);
        assert!(first_field_end < canonical_response.len());
        let mut noncanonical = canonical_response[first_field_end..].to_vec();
        noncanonical.extend_from_slice(&canonical_response[..first_field_end]);
        reordered.response = Some(noncanonical);
        assert_v1_response_schema_rejected_without_id_drift(
            "noncanonical field order",
            &reordered,
            &identity,
            &request,
        );
    }

    #[test]
    fn raw_result_v1_outer_json_reader_rejects_noncanonical_and_unknown_material() {
        let (identity, request, raw) = local_v1_material();
        let canonical = encode(&raw).unwrap();
        let restored: RawResult = decode(&canonical).unwrap();
        assert!(restored.project_for(&identity, &request, 1, None).is_ok());

        let mut whitespace = canonical.clone();
        whitespace.push(b'\n');
        assert!(matches!(
            decode::<RawResult>(&whitespace),
            Err(ChainPostCloseError::SchemaRejected)
        ));

        assert_eq!(canonical.last(), Some(&b'}'));
        let mut unknown = canonical[..canonical.len() - 1].to_vec();
        unknown.extend_from_slice(br#","TEST_CODE_unknown":true}"#);
        assert!(matches!(
            decode::<RawResult>(&unknown),
            Err(ChainPostCloseError::SchemaRejected)
        ));
    }

    fn external_v2_failure_raw(
        material: ExternalWireMaterialV1,
    ) -> (MacroQueryIdentity, Request, RawResult) {
        let (identity, request, _) = external_v2_material(&[0x5a, 0x00]);
        let evidence = ExternalWireEvidenceV1 {
            material: "external-unary-response-evidence-v1".to_owned(),
            profile: "ExternalV1".to_owned(),
            method: ExternalQueryMethod::GlobalNews,
            client_descriptor_sha256:
                crate::grpc_client::external_query_transport::EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256
                    .to_owned(),
            evidence: material,
        };
        let completion = ExternalMacroAttemptCompletion::Unary(MacroAttemptCompletion {
            response_bytes: None,
            status_code: None,
            status_details: None,
            status_error_detail_trailer: MacroTrailerMaterial::Absent,
            processed: Err(wire_error("external_response_wire_invalid")),
            retry_decision: RetryDecision::NoRetry,
            continuation: MacroContinuation::Terminal,
            external_wire: Some(evidence),
        });
        (identity, request, RawResult::capture_external(&completion))
    }

    fn invalid_frame_material(
        failure: ExternalFrameFailureV1,
        grpc_body_bytes: Vec<u8>,
    ) -> ExternalWireMaterialV1 {
        ExternalWireMaterialV1::InvalidFrame {
            body_sha256: hex::encode(Sha256::digest(&grpc_body_bytes)),
            grpc_body_bytes,
            failure,
            framed_body_limit_bytes:
                crate::grpc_client::external_query_transport::EXTERNAL_QUERY_FRAMED_BODY_LIMIT_BYTES,
        }
    }

    fn assert_v2_failure_schema_rejected(
        label: &str,
        raw: &RawResult,
        identity: &MacroQueryIdentity,
        request: &Request,
    ) {
        assert!(
            matches!(
                raw.project_for(identity, request, 1, None),
                Err(ChainPostCloseError::SchemaRejected)
            ),
            "{label}: invalid V2 capture material was accepted"
        );
    }

    #[test]
    fn raw_result_v2_closed_capture_failures_round_trip_each_supported_variant() {
        let body_limit =
            crate::grpc_client::external_query_transport::EXTERNAL_QUERY_FRAMED_BODY_LIMIT_BYTES;
        let decode_limit =
            crate::grpc_client::external_query_transport::EXTERNAL_QUERY_DECODE_LIMIT_BYTES;
        let mut cases = vec![
            (
                "missing",
                ExternalWireMaterialV1::Missing {
                    framed_body_limit_bytes: body_limit,
                },
            ),
            (
                "overflow",
                ExternalWireMaterialV1::Overflow {
                    framed_body_limit_bytes: body_limit,
                    observed_framed_body_bytes_at_least: body_limit + 1,
                },
            ),
        ];
        cases.extend([
            (
                "header truncated",
                invalid_frame_material(
                    ExternalFrameFailureV1::HeaderTruncated,
                    vec![0, 0, 0, 0],
                ),
            ),
            (
                "compression unsupported",
                invalid_frame_material(
                    ExternalFrameFailureV1::CompressionUnsupported,
                    vec![1, 0, 0, 0, 0],
                ),
            ),
            (
                "payload length exceeds limit",
                invalid_frame_material(
                    ExternalFrameFailureV1::PayloadLengthExceedsLimit,
                    [vec![0], (decode_limit as u32 + 1).to_be_bytes().to_vec()].concat(),
                ),
            ),
            (
                "payload truncated",
                invalid_frame_material(
                    ExternalFrameFailureV1::PayloadTruncated,
                    vec![0, 0, 0, 0, 2, b'x'],
                ),
            ),
            (
                "trailing data",
                invalid_frame_material(
                    ExternalFrameFailureV1::TrailingData,
                    vec![0, 0, 0, 0, 1, b'x', 0],
                ),
            ),
        ]);

        for (label, material) in cases {
            let (identity, request, raw) = external_v2_failure_raw(material.clone());
            let encoded = encode(&raw).unwrap();
            let restored: RawResult = decode(&encoded).unwrap();
            assert_eq!(
                restored.external_wire.as_ref().unwrap().evidence,
                material,
                "{label}: durable material changed"
            );
            let (processed, decision, _) = restored
                .project_for(&identity, &request, 1, None)
                .unwrap_or_else(|error| panic!("{label}: valid material rejected: {error:?}"));
            assert_eq!(decision, RetryDecision::NoRetry, "{label}");
            assert_eq!(
                processed.unwrap_err().details().code,
                "external_response_wire_invalid",
                "{label}"
            );
            assert!(matches!(
                restored.into_recovered(RetryDecision::NoRetry, None).unwrap().wire,
                RecoveredWire::LocalWireFailure
            ));
        }
    }

    #[test]
    fn raw_result_v2_closed_capture_failures_reject_each_independent_binding_mismatch() {
        let body_limit =
            crate::grpc_client::external_query_transport::EXTERNAL_QUERY_FRAMED_BODY_LIMIT_BYTES;

        let (identity, request, mut raw) = external_v2_failure_raw(
            ExternalWireMaterialV1::Missing {
                framed_body_limit_bytes: body_limit,
            },
        );
        let ExternalWireMaterialV1::Missing { framed_body_limit_bytes } =
            &mut raw.external_wire.as_mut().unwrap().evidence
        else {
            panic!("TEST_CODE expected missing evidence");
        };
        *framed_body_limit_bytes -= 1;
        assert_v2_failure_schema_rejected("missing bound", &raw, &identity, &request);

        let (identity, request, mut raw) = external_v2_failure_raw(
            ExternalWireMaterialV1::Overflow {
                framed_body_limit_bytes: body_limit,
                observed_framed_body_bytes_at_least: body_limit + 1,
            },
        );
        let ExternalWireMaterialV1::Overflow {
            observed_framed_body_bytes_at_least,
            ..
        } = &mut raw.external_wire.as_mut().unwrap().evidence
        else {
            panic!("TEST_CODE expected overflow evidence");
        };
        *observed_framed_body_bytes_at_least = body_limit;
        assert_v2_failure_schema_rejected("overflow lower bound", &raw, &identity, &request);

        let fresh_invalid = || {
            external_v2_failure_raw(invalid_frame_material(
                ExternalFrameFailureV1::CompressionUnsupported,
                vec![1, 0, 0, 0, 0],
            ))
        };

        let (identity, request, mut raw) = fresh_invalid();
        let ExternalWireMaterialV1::InvalidFrame { body_sha256, .. } =
            &mut raw.external_wire.as_mut().unwrap().evidence
        else {
            panic!("TEST_CODE expected invalid-frame evidence");
        };
        *body_sha256 = "0".repeat(64);
        assert_v2_failure_schema_rejected("body hash", &raw, &identity, &request);

        let (identity, request, mut raw) = fresh_invalid();
        let ExternalWireMaterialV1::InvalidFrame {
            grpc_body_bytes,
            body_sha256,
            ..
        } = &mut raw.external_wire.as_mut().unwrap().evidence
        else {
            panic!("TEST_CODE expected invalid-frame evidence");
        };
        grpc_body_bytes[0] = 0;
        *body_sha256 = hex::encode(Sha256::digest(grpc_body_bytes.as_slice()));
        assert_v2_failure_schema_rejected("body semantics", &raw, &identity, &request);

        let (identity, request, mut raw) = fresh_invalid();
        let ExternalWireMaterialV1::InvalidFrame { failure, .. } =
            &mut raw.external_wire.as_mut().unwrap().evidence
        else {
            panic!("TEST_CODE expected invalid-frame evidence");
        };
        *failure = ExternalFrameFailureV1::HeaderTruncated;
        assert_v2_failure_schema_rejected("framing subkind", &raw, &identity, &request);

        let (identity, request, mut raw) = external_v2_failure_raw(
            ExternalWireMaterialV1::Missing {
                framed_body_limit_bytes: body_limit,
            },
        );
        raw.response = Some(vec![0]);
        assert_v2_failure_schema_rejected("unexpected response", &raw, &identity, &request);

        // A remote status may legitimately ride with captured material, but
        // only as a complete status: code without details, or a status that
        // also claims a response, is refused.
        let (identity, request, mut raw) = external_v2_failure_raw(
            ExternalWireMaterialV1::Missing {
                framed_body_limit_bytes: body_limit,
            },
        );
        raw.code = Some(13);
        assert_v2_failure_schema_rejected("status without details", &raw, &identity, &request);

        let (identity, request, mut raw) = external_v2_failure_raw(
            ExternalWireMaterialV1::Missing {
                framed_body_limit_bytes: body_limit,
            },
        );
        raw.code = Some(13);
        raw.details = Some(Vec::new());
        raw.response = Some(vec![0]);
        assert_v2_failure_schema_rejected("status with response", &raw, &identity, &request);
    }

    fn decode_hex(raw: &str) -> Vec<u8> {
        let raw = raw.trim_end();
        assert_eq!(raw.len() % 2, 0);
        raw.as_bytes()
            .chunks_exact(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect()
    }

    fn matching_parent(plan: &Plan) -> MacroParent {
        let final_ = super::super::dragon_tiger_codec::decode_final(&plan.parent_bytes).unwrap();
        MacroParent {
            bytes: plan.parent_bytes.clone(),
            digest: plan.parent_digest.clone(),
            version: plan.parent_version,
            owner: plan.parent_owner.clone(),
            generation: plan.parent_generation,
            applied_at: plan.parent_time,
            projection: final_.projection,
        }
    }

    #[test]
    fn legacy_v1_plan_roundtrips_and_cannot_mix_explicit_registry_fields() {
        let bytes = decode_hex(include_str!("testdata/legacy_macro_plan_v1.hex"));
        assert_eq!(
            raw_digest(&bytes).as_str(),
            "bd3a3e93949b3c4568af35c3306ce904e512768b5345b7029a8a94b5056fbe7b"
        );
        let mut plan: Plan = decode(&bytes).unwrap();
        assert_eq!(encode(&plan).unwrap(), bytes);
        assert_eq!(
            plan.research_decision_provenance(),
            ResearchDecisionProvenance::LegacySuppliedConnectedLocalBridgeV1
        );
        let parent = matching_parent(&plan);
        plan.validate(&parent).unwrap();

        plan.decisions[0].registration_ordinal = Some(1);
        assert!(matches!(
            plan.validate(&parent),
            Err(ChainPostCloseError::SchemaRejected)
        ));
    }

    #[test]
    fn v2_registry_decisions_require_ordinal_provenance_remote_health_and_valid_endpoint() {
        let bytes = decode_hex(include_str!("testdata/legacy_macro_plan_v1.hex"));
        let mut plan: Plan = decode(&bytes).unwrap();
        let parent = matching_parent(&plan);
        plan.version = 2;
        for (index, decision) in plan.decisions.iter_mut().enumerate() {
            decision.available = false;
            decision.availability_source =
                "explicit-registry-local-semantic-search-disconnected".to_owned();
            decision.registration_ordinal = Some(u32::try_from(index + 1).unwrap());
            decision.local_transport_endpoint = None;
            decision.remote_health = Some("Unknown".to_owned());
        }
        plan.validate(&parent).unwrap();

        plan.decisions[0].registration_ordinal = None;
        assert!(plan.validate(&parent).is_err());
        plan.decisions[0].registration_ordinal = Some(1);
        plan.decisions[0].remote_health = None;
        assert!(plan.validate(&parent).is_err());
        plan.decisions[0].remote_health = Some("Unknown".to_owned());
        plan.decisions[0].availability_source.clear();
        assert!(plan.validate(&parent).is_err());
        for decision in &mut plan.decisions {
            decision.availability_source =
                "explicit-registry-local-semantic-search-connected".to_owned();
            decision.available = true;
            decision.local_transport_endpoint = Some("not a URI".to_owned());
        }
        assert!(plan.validate(&parent).is_err());
        for decision in &mut plan.decisions {
            decision.local_transport_endpoint = Some("http://127.0.0.1:50051".to_owned());
        }
        assert_eq!(plan.profile(), ContractProfile::ExternalV1);
        assert_ne!(
            plan.decisions[0].local_transport_endpoint(),
            Some(plan.endpoint())
        );
        plan.validate(&parent).unwrap();
    }

    #[test]
    fn external_non_global_news_remains_rejected_even_with_a_provider_catalog() {
        let (_, request, _) = external_v2_material(&[0x5a, 0x00]);
        let catalog = ExternalProviderCatalog::from_request_id_validated_capabilities(
            &CapabilitiesResponse::default(),
        );
        let raw = RawResult {
            version: 1,
            connect_unavailable: false,
            response: None,
            code: Some(tonic::Code::FailedPrecondition as i32),
            details: Some(Vec::new()),
            trailer: Trailer::Absent,
            diagnostic: Some("[redacted-unclassified-status]".to_owned()),
            decision: "NoRetry".to_owned(),
            backoff_ms: None,
            external_wire: None,
        };

        assert!(matches!(
            raw.project_for(
                &MacroQueryIdentity::EconomicCalendar,
                &request,
                1,
                Some(&catalog),
            ),
            Err(ChainPostCloseError::SchemaRejected)
        ));
    }

    #[test]
    fn external_status_without_validated_detail_has_no_attempt_observation() {
        let (identity, request, _) = external_v2_material(&[0x5a, 0x00]);
        let raw = RawResult {
            version: 1,
            connect_unavailable: false,
            response: None,
            code: Some(tonic::Code::FailedPrecondition as i32),
            details: Some(Vec::new()),
            trailer: Trailer::Absent,
            diagnostic: Some("[redacted-unclassified-status]".to_owned()),
            decision: "NoRetry".to_owned(),
            backoff_ms: None,
            external_wire: None,
        };
        let (processed, decision, provider_attempts) =
            raw.project_for(&identity, &request, 1, None).unwrap();
        let error = processed.expect_err("TEST_CODE missing detail remains a typed status error");
        assert!(matches!(error, GrpcError::FailedPrecondition { .. }));
        assert_eq!(decision, RetryDecision::NoRetry);
        assert_eq!(
            error
                .details()
                .provider_attempts
                .accepted()
                .map(|attempts| attempts.len()),
            Some(0),
            "the existing default is not validated External attempt evidence",
        );
        assert!(
            provider_attempts.is_none(),
            "missing or unvalidated External detail must not become supported empty attempts",
        );
        let recovered = raw
            .into_recovered(decision, provider_attempts)
            .expect("TEST_CODE missing-detail status recovery");
        assert!(recovered.provider_attempts.is_none());
        assert!(matches!(
            recovered.wire,
            RecoveredWire::Status {
                code,
                details,
                trailer: Trailer::Absent,
            } if code == tonic::Code::FailedPrecondition as i32 && details.is_empty()
        ));
        assert_eq!(
            recovered.diagnostic.as_deref(),
            Some("[redacted-unclassified-status]")
        );
        assert_eq!(recovered.retry_decision, RetryDecision::NoRetry);
        assert_eq!(recovered.continuation, MacroContinuation::Terminal);
    }

}
