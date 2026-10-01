//! Isolated, one-shot ExternalV1 flow reader. No production source uses it.

use super::*;
use crate::grpc_client::connection_qualification::ConnectionIdentity;
use crate::grpc_client::envelope::QueryAdmission;
use crate::grpc_client::external_pb::magic::market::v1::{
    AdmissionState, BuildIdentity, Capability,
};
use crate::grpc_client::external_query_transport::{wire_error, ExternalWireEvidenceV1};
use crate::grpc_contract::methods::{ExternalMethod, MethodIdentity};
use prost::Message as _;

pub(crate) struct ExternalFlowReadClient {
    client: GrpcMarketClient,
}

pub(crate) struct ExternalFlowStatusEvidence {
    pub(crate) code: i32,
    pub(crate) details: Vec<u8>,
    pub(crate) error_detail_trailer: super::unary_attempt::UnaryTrailerMaterial,
}

/// One data RPC, with the exact request, response wire, qualification receipt,
/// and classified result. A failed admission remains available only as wire
/// evidence; it is never returned as a successful QueryResult.
pub(crate) struct ExternalFlowObservation {
    pub(crate) operation: ExternalOperation,
    pub(crate) connection_identity: ConnectionIdentity,
    pub(crate) server_build_identity: BuildIdentity,
    pub(crate) capability: Capability,
    pub(crate) request_id_correlation: String,
    pub(crate) request_bytes: Vec<u8>,
    pub(crate) wire: ExternalWireEvidenceV1,
    pub(crate) status: Option<ExternalFlowStatusEvidence>,
    pub(crate) result: Result<QueryResult, GrpcError>,
}

impl ExternalFlowReadClient {
    pub(crate) async fn connect_client_bundle(path: &Path) -> Result<Self, GrpcError> {
        Ok(Self {
            client: GrpcMarketClient::connect_client_bundle(path).await?,
        })
    }

    /// Revalidates Health and Capabilities immediately before each data call.
    /// This is deliberately separate from GrpcSource and performs no retry.
    pub(crate) async fn query_once(
        &mut self,
        operation: ExternalOperation,
        params: serde_json::Value,
    ) -> Result<ExternalFlowObservation, GrpcError> {
        let method = ExternalQueryMethod::from_external_operation(operation)
            .filter(|method| {
                matches!(
                    method,
                    ExternalQueryMethod::MoneyFlows | ExternalQueryMethod::BoardFlows
                )
            })
            .ok_or_else(unsupported_flow)?;
        let request =
            crate::grpc_client::external_v1::build_external_flow_query_request(operation, params)
                .map_err(super::map_external_contract_error)?;
        if self.client.profile != ContractProfile::ExternalV1 {
            return Err(unsupported_flow());
        }
        let health = self.client.get_external_health().await?;
        let connection_identity = self.client.external_connection_identity()?;
        let server_build_identity = health
            .build_identity
            .ok_or_else(|| crate::grpc_client::connection_qualification::unqualified())?;
        let capabilities = self.client.get_external_capabilities().await?;
        let capability = require_flow_capability(&capabilities, operation)?.clone();

        let request_id = request
            .context
            .as_ref()
            .map(|context| context.request_id.as_str())
            .filter(|id| !id.is_empty())
            .ok_or_else(|| wire_error("external_request_context_missing"))?
            .to_owned();
        let request_id_correlation =
            crate::grpc_client::errors::request_id_correlation(&request_id)
                .ok_or_else(|| wire_error("external_request_context_missing"))?;
        let request_bytes = request.encode_to_vec();
        let authority = self
            .client
            .acquisition_authority
            .as_deref()
            .ok_or_else(|| wire_error("external_acquisition_authority_missing"))?;
        let mut authorized = tonic::Request::new(request);
        self.client.attach_request_auth(&mut authorized)?;
        self.client.require_external_qualification()?;
        let outcome = match &mut self.client.data {
            DataTransport::External(data) => {
                data.call_with_descriptor(
                    method,
                    authorized,
                    &connection_identity.descriptor_sha256,
                )
                .await
            }
            DataTransport::Local(_) => return Err(unsupported_flow()),
        };
        let method_identity = MethodIdentity::External(
            ExternalMethod::try_from_operation(operation).map_err(|_| unsupported_flow())?,
        );
        let (wire, status, result) = match outcome {
            ExternalQueryCall::Response { message, evidence } => {
                let result = evidence
                    .validate_descriptor(method, &connection_identity.descriptor_sha256)
                    .and_then(|()| {
                        admit_external_payload(
                            evidence
                                .payload()
                                .ok_or_else(|| wire_error("external_response_wire_invalid"))?,
                        )
                    })
                    .and_then(|()| {
                        crate::grpc_client::envelope::parse_external_native_query_response(
                            &request_id,
                            operation,
                            authority,
                            message,
                        )
                        .map_err(GrpcError::from)
                    })
                    .and_then(admit_flow_envelope);
                (evidence, None, result)
            }
            ExternalQueryCall::UnaryStatus { status, evidence } => {
                let (code, details, error_detail_trailer) =
                    super::unary_attempt::capture_status_material(&status);
                let result = Err(self.client.external_status_error(
                    status,
                    self.client
                        .data_status_context(method_identity, &request_id),
                ));
                (
                    evidence,
                    Some(ExternalFlowStatusEvidence {
                        code,
                        details,
                        error_detail_trailer,
                    }),
                    result,
                )
            }
            ExternalQueryCall::LocalWireFailure { error, evidence } => (evidence, None, Err(error)),
        };
        Ok(ExternalFlowObservation {
            operation,
            connection_identity,
            server_build_identity,
            capability,
            request_id_correlation,
            request_bytes,
            wire,
            status,
            result,
        })
    }
}

fn unsupported_flow() -> GrpcError {
    GrpcError::Unimplemented {
        details: Box::default(),
    }
}

fn flow_gate_error(code: &str, unavailable: bool) -> GrpcError {
    let details = Box::new(ErrorDetail {
        code: code.to_owned(),
        reason_code: Some(code.to_owned()),
        retryable: Some(unavailable),
        ..ErrorDetail::default()
    });
    if unavailable {
        GrpcError::Unavailable { details }
    } else {
        GrpcError::FailedPrecondition { details }
    }
}

fn require_flow_capability(
    capabilities: &[Capability],
    operation: ExternalOperation,
) -> Result<&Capability, GrpcError> {
    let matching = capabilities
        .iter()
        .filter(|capability| {
            capability.operation == operation as i32 && capability.provider == "Eastmoney"
        })
        .collect::<Vec<_>>();
    // The public repeated field has no uniqueness rule for operation/provider.
    // Multiple rows may describe incompatible scopes or admission decisions.
    if matching.len() > 1 {
        return Err(flow_gate_error("external_flow_capability_ambiguous", false));
    }
    if let Some(ready) = matching.first().filter(|capability| {
        capability.repository_admission == AdmissionState::Admitted as i32
            && capability.runtime_available
            && capability.blocker.is_empty()
    }) {
        return Ok(ready);
    }
    if matching.is_empty() {
        Err(flow_gate_error("external_flow_capability_missing", false))
    } else if !matching
        .iter()
        .any(|capability| capability.repository_admission == AdmissionState::Admitted as i32)
    {
        Err(flow_gate_error(
            "external_flow_capability_unadmitted",
            false,
        ))
    } else if matching
        .iter()
        .any(|capability| !capability.blocker.is_empty())
    {
        Err(flow_gate_error("external_flow_capability_conflict", false))
    } else {
        Err(flow_gate_error("external_flow_runtime_unavailable", true))
    }
}

fn admit_flow_envelope(result: QueryResult) -> Result<QueryResult, GrpcError> {
    if result.admission != QueryAdmission::Admitted || !result.diagnostic_blocker.is_empty() {
        return Err(flow_gate_error("external_flow_response_unadmitted", false));
    }
    if !result.complete {
        return Err(flow_gate_error("external_flow_response_incomplete", false));
    }
    if result.selected_provider != "Eastmoney" {
        return Err(flow_gate_error("external_flow_provider_mismatch", false));
    }
    Ok(result)
}

#[cfg(test)]
#[path = "external_flow_read_tests.rs"]
mod tests;
