//! Legacy initialization qualification shared with the ordinary bridge cache.
//! A semaphore permit coordinates initialization; no cache mutex spans I/O.
use super::macro_queries::{ConnectedMacroQueries, PreparedMacroQueries};
use super::*;
use crate::grpc_client::client::PreparedExternalEndpoint;
use crate::grpc_contract::methods::ExternalMethod;

fn external_global_news() -> ExternalMethod {
    ExternalMethod::try_from_operation(
        crate::grpc_client::external_pb::magic::market::v1::Operation::GlobalNews,
    )
    .expect("External GlobalNews is a nonzero generated operation")
}

pub(crate) struct LegacyExternalRoute {
    pub(crate) prepared: PreparedExternalEndpoint,
    pub(crate) client: Option<GrpcMarketClient>,
    pub(crate) health_ready: bool,
    pub(crate) macro_ready: bool,
    pub(crate) qualification: Option<LegacyQualification>,
}

pub(crate) struct LegacyQualification {
    source: Arc<GrpcSource>,
    prepared: PreparedExternalEndpoint,
    _permit: tokio::sync::OwnedSemaphorePermit,
}

impl LegacyQualification {
    pub(crate) fn publish(self, client: GrpcMarketClient) -> Result<(), GatewayError> {
        let mut cache = self.source.external_client.try_lock().map_err(|_| {
            GatewayError::unavailable(
                "GrpcExternalV1",
                None,
                false,
                "External cache busy at Macro initialization confirmation",
            )
        })?;
        if let Some(state) = cache.as_mut() {
            if state.prepared.endpoint_uri() != self.prepared.endpoint_uri() {
                return Err(GatewayError::unavailable(
                    "GrpcExternalV1",
                    None,
                    false,
                    "External initialization identity changed",
                ));
            }
            state.ready_operations.insert(external_global_news());
        } else {
            *cache = Some(ExternalClientState {
                client,
                prepared: self.prepared.clone(),
                ready_operations: HashSet::from([external_global_news()]),
            });
        }
        Ok(())
    }
}

impl GrpcSource {
    pub(crate) fn legacy_macro_external_news(&self) -> bool {
        self.external_bundle.is_some()
    }

    pub(crate) async fn legacy_macro_local(
        self: Arc<Self>,
    ) -> Result<ConnectedMacroQueries, GatewayError> {
        self.ensure_connected().await?;
        let client = self.client.lock().await.as_ref().cloned().ok_or_else(|| {
            GatewayError::unavailable(
                "GrpcBridge",
                None,
                false,
                "Local cache disappeared after initialization",
            )
        })?;
        Ok(ConnectedMacroQueries::from_legacy(
            client,
            self.addr.clone(),
        ))
    }

    pub(crate) async fn legacy_macro_external(
        self: Arc<Self>,
    ) -> Result<LegacyExternalRoute, GatewayError> {
        let permit = Arc::clone(&self.external_initialization)
            .acquire_owned()
            .await
            .map_err(|_| {
                GatewayError::unavailable(
                    "GrpcExternalV1",
                    None,
                    false,
                    "External initialization qualification closed",
                )
            })?;
        let cached = self.external_client.lock().await.as_ref().map(|state| {
            (
                state.prepared.clone(),
                state.client.clone(),
                state.ready_operations.contains(&external_global_news()),
            )
        });
        if let Some((prepared, client, true)) = &cached {
            return Ok(LegacyExternalRoute {
                prepared: prepared.clone(),
                client: Some(client.clone()),
                health_ready: true,
                macro_ready: true,
                qualification: None,
            });
        }
        let (prepared, client, health_ready) = if let Some((prepared, client, false)) = cached {
            (prepared, Some(client), true)
        } else {
            let PreparedMacroQueries::External(prepared) = self.prepare_macro_queries()? else {
                return Err(GatewayError::unavailable(
                    "GrpcExternalV1",
                    None,
                    false,
                    "Legacy Macro has no External route",
                ));
            };
            (prepared, None, false)
        };
        Ok(LegacyExternalRoute {
            prepared: prepared.clone(),
            client,
            health_ready,
            macro_ready: false,
            qualification: Some(LegacyQualification {
                source: self,
                prepared,
                _permit: permit,
            }),
        })
    }
}

pub(crate) fn health_outcome(
    completion: &crate::grpc_client::client::external_control_attempt::ExternalControlCompletion<
        crate::grpc_client::external_pb::magic::market::v1::HealthResponse,
    >,
) -> Result<(), GatewayError> {
    completion
        .processed()
        .map_err(map_external_connection_error_ref)?;
    match completion.result_material() {
        crate::grpc_client::client::external_control_attempt::ExternalControlResultMaterial::Response { response,.. } => require_external_health_ready(response),
        _=>Err(GatewayError::unavailable("GrpcExternalV1",None,false,"External Health lacks a response")),
    }
}

pub(crate) fn capabilities_outcome(
    completion: &crate::grpc_client::client::external_control_attempt::ExternalControlCompletion<
        crate::grpc_client::external_pb::magic::market::v1::CapabilitiesResponse,
    >,
) -> Result<(), GatewayError> {
    completion
        .processed()
        .map_err(map_external_connection_error_ref)?;
    match completion.result_material() {
        crate::grpc_client::client::external_control_attempt::ExternalControlResultMaterial::Response { response,.. } => require_external_capability(&response.capabilities,external_global_news()),
        _=>Err(GatewayError::unavailable("GrpcExternalV1",None,false,"External Capabilities lacks a response")),
    }
}
