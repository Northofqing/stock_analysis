//! One-shot ExternalV1 HistoricalBars observation; no production route uses it.

use super::*;
use crate::grpc_client::connection_qualification::ConnectionIdentity;
use crate::grpc_client::envelope::QueryAdmission;
use crate::grpc_client::external_pb::magic::market::v1::{
    AdmissionState, BuildIdentity, Capability,
};
use crate::grpc_client::external_query_transport::{wire_error, ExternalWireEvidenceV1};
use crate::grpc_client::external_v1::ExternalHistoricalBarsQuery;
use crate::grpc_contract::methods::{ExternalMethod, MethodIdentity};
use prost::Message as _;

pub(crate) struct ExternalHistoricalReadClient {
    client: GrpcMarketClient,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(crate) enum ExternalHistoricalTrailerMaterial {
    Absent,
    Bytes(Vec<u8>),
    Malformed,
}

#[derive(Debug)]
pub(crate) struct ExternalHistoricalStatusEvidence {
    pub(crate) raw_status: tonic::Status,
    pub(crate) code: i32,
    pub(crate) details: Vec<u8>,
    pub(crate) error_detail_trailer: ExternalHistoricalTrailerMaterial,
}

/// A complete transport observation, including rejected query envelopes and
/// remote status material. Successful parsing does not certify record coverage.
#[derive(Debug)]
pub(crate) struct ExternalHistoricalObservation {
    pub(crate) connection_identity: ConnectionIdentity,
    pub(crate) health: ExternalHealthResponse,
    pub(crate) health_wire: Vec<u8>,
    pub(crate) server_build_identity: BuildIdentity,
    pub(crate) capabilities_response: ExternalCapabilitiesResponse,
    pub(crate) capabilities_wire: Vec<u8>,
    pub(crate) capability: Capability,
    pub(crate) request_id_correlation: String,
    pub(crate) request_bytes: Vec<u8>,
    pub(crate) wire: ExternalWireEvidenceV1,
    pub(crate) status: Option<ExternalHistoricalStatusEvidence>,
    pub(crate) result: Result<QueryResult, GrpcError>,
}

impl ExternalHistoricalReadClient {
    pub(crate) async fn connect_client_bundle(path: &Path) -> Result<Self, GrpcError> {
        Ok(Self {
            client: GrpcMarketClient::connect_client_bundle(path).await?,
        })
    }

    pub(crate) async fn query_once(
        &mut self,
        query: ExternalHistoricalBarsQuery,
    ) -> Result<ExternalHistoricalObservation, GrpcError> {
        let operation = ExternalOperation::HistoricalBars;
        let method = ExternalQueryMethod::HistoricalBars;
        if self.client.profile != ContractProfile::ExternalV1 {
            return Err(GrpcError::Unimplemented {
                details: Box::default(),
            });
        }
        let request = query.into_request();
        let (health, health_wire) = self.client.get_external_health_observed().await?;
        let connection_identity = self.client.external_connection_identity()?;
        let server_build_identity = health
            .build_identity
            .clone()
            .ok_or_else(|| crate::grpc_client::connection_qualification::unqualified())?;
        let (capabilities_response, capabilities_wire) =
            self.client.get_external_capabilities_observed().await?;
        let capability =
            require_historical_capability(&capabilities_response.capabilities)?.clone();

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
            DataTransport::Local(_) => {
                return Err(GrpcError::Unimplemented {
                    details: Box::default(),
                });
            }
        };
        let method_identity = MethodIdentity::External(
            ExternalMethod::try_from_operation(operation)
                .map_err(|_| wire_error("external_historical_method_invalid"))?,
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
                    .and_then(observe_historical_envelope);
                (evidence, None, result)
            }
            ExternalQueryCall::UnaryStatus { status, evidence } => {
                let raw_status = status.clone();
                let (code, details, trailer) =
                    super::unary_attempt::capture_status_material(&status);
                let error_detail_trailer = match trailer {
                    super::unary_attempt::UnaryTrailerMaterial::Absent => {
                        ExternalHistoricalTrailerMaterial::Absent
                    }
                    super::unary_attempt::UnaryTrailerMaterial::Bytes(bytes) => {
                        ExternalHistoricalTrailerMaterial::Bytes(bytes)
                    }
                    super::unary_attempt::UnaryTrailerMaterial::Malformed => {
                        ExternalHistoricalTrailerMaterial::Malformed
                    }
                };
                let result = Err(self.client.external_status_error(
                    status,
                    self.client
                        .data_status_context(method_identity, &request_id),
                ));
                (
                    evidence,
                    Some(ExternalHistoricalStatusEvidence {
                        raw_status,
                        code,
                        details,
                        error_detail_trailer,
                    }),
                    result,
                )
            }
            ExternalQueryCall::LocalWireFailure { error, evidence } => (evidence, None, Err(error)),
        };
        Ok(ExternalHistoricalObservation {
            connection_identity,
            health,
            health_wire,
            server_build_identity,
            capabilities_response,
            capabilities_wire,
            capability,
            request_id_correlation,
            request_bytes,
            wire,
            status,
            result,
        })
    }
}

fn historical_gate_error(code: &str, unavailable: bool) -> GrpcError {
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

fn require_historical_capability(capabilities: &[Capability]) -> Result<&Capability, GrpcError> {
    let mut matching = capabilities.iter().filter(|capability| {
        capability.operation == ExternalOperation::HistoricalBars as i32
            && capability.provider == "HithinkFinance"
    });
    let capability = matching
        .next()
        .ok_or_else(|| historical_gate_error("external_historical_capability_missing", false))?;
    if matching.next().is_some() {
        return Err(historical_gate_error(
            "external_historical_capability_ambiguous",
            false,
        ));
    }
    if capability.repository_admission != AdmissionState::Admitted as i32 {
        return Err(historical_gate_error(
            "external_historical_capability_unadmitted",
            false,
        ));
    }
    if !capability.blocker.is_empty() {
        return Err(historical_gate_error(
            "external_historical_capability_conflict",
            false,
        ));
    }
    if !capability.runtime_available {
        return Err(historical_gate_error(
            "external_historical_runtime_unavailable",
            true,
        ));
    }
    Ok(capability)
}

fn observe_historical_envelope(result: QueryResult) -> Result<QueryResult, GrpcError> {
    if result.admission != QueryAdmission::Admitted || !result.diagnostic_blocker.is_empty() {
        return Err(historical_gate_error(
            "external_historical_response_unadmitted",
            false,
        ));
    }
    if !result.complete {
        return Err(historical_gate_error(
            "external_historical_response_incomplete",
            false,
        ));
    }
    if result.selected_provider != "HithinkFinance" {
        return Err(historical_gate_error(
            "external_historical_provider_mismatch",
            false,
        ));
    }
    Ok(result)
}

#[cfg(test)]
#[path = "external_historical_read_tests.rs"]
mod tests;
