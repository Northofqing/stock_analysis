//! Instance-owned Macro adapter. Construction never performs readiness RPCs or dials.
use super::*;
use crate::grpc_client::client::macro_attempt::{
    MacroQueryIdentity, MacroQuerySession, RestoredMacroRequest,
};
use crate::grpc_client::client::PreparedExternalEndpoint;

#[derive(Clone)]
pub(crate) struct ConnectedMacroQueries {
    client: GrpcMarketClient,
    endpoint: String,
}

pub(crate) enum PreparedMacroQueries {
    Local(ConnectedMacroQueries),
    External(PreparedExternalEndpoint),
}

impl GrpcSource {
    pub(crate) fn local_semantic_search_connection_state(
        &self,
    ) -> LocalSemanticSearchConnectionState {
        match self.client.try_lock() {
            Ok(guard) if guard.is_some() => LocalSemanticSearchConnectionState::Connected {
                endpoint: self.addr.clone(),
            },
            Ok(_) => LocalSemanticSearchConnectionState::Disconnected,
            Err(_) => LocalSemanticSearchConnectionState::Busy,
        }
    }

    #[cfg(test)]
    pub(crate) fn hold_local_connection_state_for_test(
        &self,
    ) -> tokio::sync::MutexGuard<'_, Option<GrpcMarketClient>> {
        self.client
            .try_lock()
            .expect("TEST_CODE Local connection state lock")
    }

    #[cfg(test)]
    pub(crate) fn from_external_macro_bundle_for_test(bundle: PathBuf) -> Self {
        Self {
            addr: "TEST_CODE_EXTERNAL_MACRO_LOCAL_SENTINEL".to_owned(),
            client: AsyncMutex::new(None),
            external_bundle: Some(bundle),
            external_client: AsyncMutex::new(None),
            local_initialization: Arc::new(tokio::sync::Semaphore::new(1)),
            external_initialization: Arc::new(tokio::sync::Semaphore::new(1)),
        }
    }

    pub(crate) fn connected_macro_queries(&self) -> Result<ConnectedMacroQueries, GatewayError> {
        if self.external_bundle.is_some() {
            return Err(GatewayError::unavailable(
                "Macro",
                None,
                false,
                "External Macro requires separately confirmed control effects",
            ));
        }
        self.connected_local_macro_queries()
    }

    /// Independent Local transport, including on an instance with External news.
    /// This observes only the supplied instance; it never initializes a bridge.
    pub(crate) fn connected_local_macro_queries(
        &self,
    ) -> Result<ConnectedMacroQueries, GatewayError> {
        let guard = self.client.try_lock().map_err(|_| {
            GatewayError::unavailable(
                "Macro",
                None,
                false,
                "Macro Local bridge is being prepared by another owner",
            )
        })?;
        let client = guard.as_ref().cloned().ok_or_else(|| {
            GatewayError::unavailable(
                "Macro",
                None,
                false,
                "Macro requires an explicitly connected Local bridge",
            )
        })?;
        Ok(ConnectedMacroQueries {
            client,
            endpoint: self.addr.clone(),
        })
    }

    pub(crate) fn prepare_macro_queries(&self) -> Result<PreparedMacroQueries, GatewayError> {
        let Some(bundle) = self.external_bundle.as_ref() else {
            return self
                .connected_macro_queries()
                .map(PreparedMacroQueries::Local);
        };
        if !bundle.is_absolute() {
            return Err(GatewayError::classified(
                "GrpcExternalV1",
                None,
                "invalid_request",
                "external_bundle_invalid",
                false,
                "ExternalV1 client-bundle 必须是绝对路径",
            ));
        }
        GrpcMarketClient::prepare_client_bundle(bundle)
            .map(PreparedMacroQueries::External)
            .map_err(map_external_connection_error)
    }
}

impl ConnectedMacroQueries {
    pub(super) fn from_legacy(client: GrpcMarketClient, endpoint: String) -> Self {
        Self { client, endpoint }
    }
    pub(crate) fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub(crate) fn session(
        &self,
        identity: MacroQueryIdentity,
    ) -> Result<MacroQuerySession, GrpcError> {
        self.client.macro_query(identity)
    }

    pub(crate) fn resume(
        &self,
        identity: MacroQueryIdentity,
        request: RestoredMacroRequest,
    ) -> Result<MacroQuerySession, GrpcError> {
        self.client.resume_macro_query(identity, request)
    }
}

impl PreparedMacroQueries {
    pub(crate) fn endpoint(&self) -> &str {
        match self {
            Self::Local(queries) => queries.endpoint(),
            Self::External(prepared) => prepared.endpoint_uri(),
        }
    }

    pub(crate) fn profile(&self) -> ContractProfile {
        match self {
            Self::Local(_) => ContractProfile::LocalBridgeV1,
            Self::External(_) => ContractProfile::ExternalV1,
        }
    }
}

pub(crate) fn map_macro_error(error: &GrpcError) -> GatewayError {
    map_query_error(Operation::GlobalNews, error)
}

pub(crate) fn map_macro_query_error(
    operation: Operation,
    profile: ContractProfile,
    error: &GrpcError,
) -> GatewayError {
    match profile {
        ContractProfile::LocalBridgeV1 => map_query_error(operation, error),
        ContractProfile::ExternalV1 => map_external_query_error(operation, error),
    }
}

pub(crate) fn news_outcome(
    provider: GlobalNewsProvider,
    limit: u32,
    profile: ContractProfile,
    processed: &Result<QueryResult, GrpcError>,
) -> Result<GatewayBatch<GlobalNewsRecord>, GatewayError> {
    match processed {
        Ok(query) => GrpcSource::global_news_query_result(
            provider,
            limit,
            profile == ContractProfile::ExternalV1,
            query,
        ),
        Err(error) => Err(map_macro_query_error(Operation::GlobalNews, profile, error)),
    }
}

pub(crate) fn economic_outcome(
    processed: &Result<QueryResult, GrpcError>,
) -> Result<GatewayBatch<EconomicReleaseFact>, GatewayError> {
    match processed {
        Ok(query) => convert::economic_calendar(query),
        Err(error) => Err(map_query_error(Operation::EconomicCalendar, error)),
    }
}

pub(crate) fn web_outcome(
    provider: GeneralWebResearchProvider,
    query: &str,
    limit: usize,
    processed: &Result<QueryResult, GrpcError>,
) -> Result<
    GeneralWebResearchBatch,
    crate::data_gateway::general_web_research::GeneralWebResearchError,
> {
    use crate::data_gateway::general_web_research::{transport_error, validate_request};
    let query = validate_request(provider, query, limit)?;
    match processed {
        Ok(result) => convert::semantic_search(result, query, provider, limit),
        Err(error) => Err(map_query_error(Operation::SemanticSearch, error)),
    }
    .map_err(|error| transport_error(provider, error))
}
