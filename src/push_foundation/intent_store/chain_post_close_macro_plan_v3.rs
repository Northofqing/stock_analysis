//! Plan v3 adds an independent explicit Local observation without rewriting v1/v2.
use super::dragon_tiger::MacroParent;
use super::macro_codec::{self, require, Plan, Request, Result};
use super::ChainPostCloseError;
use crate::data_gateway::grpc_source::{GrpcSource, LocalSemanticSearchConnectionState};
use crate::monitor::push_job::UtcMicros;
use crate::search_service::macro_news::runner::{Candidate, Definition, QueryKey};
use crate::search_service::service::MacroWebSnapshot;
use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LocalRoute {
    pub(super) state: LocalRouteState,
    observed_via: String,
    profile: String,
    operations: Vec<String>,
    pub(super) endpoint: Option<String>,
    pub(super) reason: Option<LocalUnavailableReason>,
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum LocalRouteState {
    ObservedConnected,
    ObservedUnavailable,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) enum LocalUnavailableReason {
    NotConnectedObserved,
    OrdinaryPreparationFailure(crate::data_gateway::review::StoredGatewayError),
}

impl LocalRoute {
    pub(super) fn observe(source: &GrpcSource) -> Result<Self> {
        Self::from_observation(&source.local_semantic_search_connection_state())
    }

    fn from_observation(observation: &LocalSemanticSearchConnectionState) -> Result<Self> {
        match observation {
            LocalSemanticSearchConnectionState::Connected { endpoint } => {
                Self::connected(endpoint.clone())
            }
            LocalSemanticSearchConnectionState::Disconnected => Ok(Self::unavailable()),
            LocalSemanticSearchConnectionState::Busy => {
                Err(ChainPostCloseError::InvalidConfiguration {
                    check: "Macro Local observation is busy",
                })
            }
        }
    }

    fn connected(endpoint: String) -> Result<Self> {
        let route = Self {
            state: LocalRouteState::ObservedConnected,
            observed_via: "ExplicitInstanceObservation".to_owned(),
            profile: "LocalBridgeV1".to_owned(),
            operations: vec!["EconomicCalendar".to_owned(), "SemanticSearch".to_owned()],
            endpoint: Some(endpoint),
            reason: None,
        };
        route.validate()?;
        Ok(route)
    }

    fn unavailable() -> Self {
        Self {
            state: LocalRouteState::ObservedUnavailable,
            observed_via: "ExplicitInstanceObservation".to_owned(),
            profile: "LocalBridgeV1".to_owned(),
            operations: vec!["EconomicCalendar".to_owned(), "SemanticSearch".to_owned()],
            // None is the actual observation: there is no connected endpoint.
            // Never substitute a configured default or an External news endpoint.
            endpoint: None,
            reason: Some(LocalUnavailableReason::NotConnectedObserved),
        }
    }

    pub(super) fn validate(&self) -> Result<()> {
        require(
            self.observed_via == "ExplicitInstanceObservation"
                && self.profile == "LocalBridgeV1"
                && self.operations == ["EconomicCalendar", "SemanticSearch"],
        )?;
        if let Some(endpoint) = &self.endpoint {
            require(
                endpoint.starts_with("http://")
                    && !endpoint.contains(['@', '?', '#'])
                    && endpoint.len() <= 2048
                    && endpoint.parse::<tonic::codegen::http::Uri>().is_ok(),
            )?;
        }
        match self.state {
            LocalRouteState::ObservedConnected => {
                require(self.endpoint.is_some() && self.reason.is_none())
            }
            LocalRouteState::ObservedUnavailable => require(self.reason.is_some()),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PlanV3 {
    version: u32,
    pub(super) core: Plan,
    pub(super) local_route: LocalRoute,
}

impl PlanV3 {
    pub(super) fn new(
        parent: &MacroParent,
        started: UtcMicros,
        observation: DateTime<FixedOffset>,
        endpoint: &str,
        request: Request,
        web: &MacroWebSnapshot,
    ) -> Result<Self> {
        let mut plan = Self {
            version: 3,
            core: Plan::new(parent, started, observation, endpoint, request, web)?,
            local_route: LocalRoute::from_observation(web.local_transport())?,
        };
        plan.validate(parent)?;
        Ok(plan)
    }

    pub(super) fn validate(&mut self, parent: &MacroParent) -> Result<()> {
        require(self.version == 3 && self.core.format_version() == 2)?;
        // Original v2 validator is deliberately reused unchanged for the core.
        self.core.validate(parent)?;
        self.local_route.validate()?;
        for decision in self.core.research_decisions() {
            match self.local_route.state {
                LocalRouteState::ObservedConnected => require(
                    decision.is_available()
                        && decision.local_transport_endpoint()
                            == self.local_route.endpoint.as_deref(),
                )?,
                LocalRouteState::ObservedUnavailable => require(
                    !decision.is_available()
                        && decision.local_transport_endpoint()
                            == self.local_route.endpoint.as_deref(),
                )?,
            }
        }
        if self.core.profile() == crate::grpc_client::client::ContractProfile::LocalBridgeV1 {
            require(
                self.local_route.state == LocalRouteState::ObservedConnected
                    && self.local_route.endpoint.as_deref() == Some(self.core.endpoint()),
            )?;
        }
        Ok(())
    }

    pub(super) fn decode(bytes: &[u8], parent: &MacroParent) -> Result<Self> {
        let mut value: Self = macro_codec::decode(bytes)?;
        value.validate(parent)?;
        Ok(value)
    }
}

pub(super) fn legacy_local_route(plan: &Plan) -> Result<LocalRoute> {
    if plan.format_version() == 1 {
        return Err(ChainPostCloseError::LegacyPlanExecutionUnsupported);
    }
    require(plan.format_version() == 2)?;
    if plan.profile() == crate::grpc_client::client::ContractProfile::LocalBridgeV1 {
        return LocalRoute::connected(plan.endpoint().to_owned());
    }
    for decision in plan.research_decisions() {
        if decision.availability_source() == "explicit-registry-local-semantic-search-connected" {
            return LocalRoute::connected(
                decision
                    .local_transport_endpoint()
                    .ok_or(ChainPostCloseError::MissingPersistedLocalRoute)?
                    .to_owned(),
            );
        }
        if decision.availability_source() == "explicit-registry-local-semantic-search-disconnected"
        {
            return Ok(LocalRoute::unavailable());
        }
    }
    Err(ChainPostCloseError::MissingPersistedLocalRoute)
}

pub(super) fn definition(core: &Plan) -> Result<Definition> {
    let observation = DateTime::parse_from_rfc3339(core.observed_local())
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    let candidates = core
        .research_decisions()
        .iter()
        .zip(core.research_providers())
        .map(|(decision, provider)| {
            Ok(Candidate {
                ordinal: decision
                    .registration_ordinal()
                    .ok_or(ChainPostCloseError::LegacyPlanExecutionUnsupported)?,
                provider: Some(*provider),
                eligible: decision.supports_general_web_search() && decision.is_available(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Definition {
        observed_date: observation.format("%Y年%m月%d日").to_string(),
        research_limit: core.research_limit(),
        candidates,
        external_news: core.profile() == crate::grpc_client::client::ContractProfile::ExternalV1,
    })
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RequestPlan {
    pub(super) version: u32,
    pub(super) query: QueryKey,
    pub(super) request: Request,
}
impl RequestPlan {
    pub(super) fn validate(
        &self,
        definition: &Definition,
        local: &LocalRoute,
        news_endpoint: &str,
        endpoint: &str,
    ) -> Result<()> {
        require(self.version == 2)?;
        let identity = definition
            .identity(self.query)
            .map_err(|_| ChainPostCloseError::SchemaRejected)?;
        self.request.validate_for(&identity)?;
        let external = matches!(self.query, QueryKey::Gateway(1..=4)) && definition.external_news;
        require((self.request.profile == "ExternalV1") == external)?;
        if external {
            require(endpoint == news_endpoint)
        } else {
            require(
                local.state == LocalRouteState::ObservedConnected
                    && local.endpoint.as_deref() == Some(endpoint),
            )
        }
    }
}
