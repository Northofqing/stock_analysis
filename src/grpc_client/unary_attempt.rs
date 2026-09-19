use prost::Message as _;

use super::{
    apply_acquisition_authority, ContractProfile, DataCallAuthorized, GrpcMarketClient,
    ProfileAuthorizedRequest,
};
use crate::grpc_client::envelope::{
    parse_external_query_response, parse_query_response, QueryResult,
};
use crate::grpc_client::errors::GrpcError;
use crate::grpc_client::external_query_transport::{
    admit_external_payload, ExternalQueryCall, ExternalWireEvidenceV1,
};
use crate::grpc_client::pb::magic::market::v1::{Operation, QueryResponse};
use crate::grpc_client::retry::{retry_decision, RetryDecision, RetryPolicy};

pub(crate) struct UnaryAttemptCompletion {
    pub(crate) response_bytes: Option<Vec<u8>>,
    pub(crate) status_code: Option<i32>,
    pub(crate) status_details: Option<Vec<u8>>,
    pub(crate) status_error_detail_trailer: UnaryTrailerMaterial,
    pub(crate) processed: Result<QueryResult, GrpcError>,
    pub(crate) retry_decision: RetryDecision,
    pub(crate) continuation: UnaryContinuation,
    pub(crate) external_wire: Option<ExternalWireEvidenceV1>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum UnaryTrailerMaterial {
    Absent,
    Bytes(Vec<u8>),
    Malformed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UnaryContinuation {
    Retry { backoff_ms: u64 },
    Terminal,
}

pub(super) fn capture_status_material(
    status: &tonic::Status,
) -> (i32, Vec<u8>, UnaryTrailerMaterial) {
    let trailer = match status.metadata().get_bin("magic-error-detail-bin") {
        None => UnaryTrailerMaterial::Absent,
        Some(value) => match value.to_bytes() {
            Ok(bytes) => UnaryTrailerMaterial::Bytes(bytes.to_vec()),
            Err(_) => UnaryTrailerMaterial::Malformed,
        },
    };
    (status.code() as i32, status.details().to_vec(), trailer)
}

pub(super) fn project_response(
    profile: ContractProfile,
    acquisition_authority: Option<&str>,
    request_id: &str,
    operation: Operation,
    mut response: QueryResponse,
) -> Result<QueryResult, GrpcError> {
    apply_acquisition_authority(profile, acquisition_authority, &mut response)?;
    let mut result = parse_query_response(request_id, operation, response).map_err(GrpcError::from)?;
    if profile == ContractProfile::ExternalV1 {
        result.provenance = crate::grpc_client::envelope::AcquisitionProvenance::ExternalMtlsAuthority(
            acquisition_authority
                .ok_or_else(|| {
                    crate::grpc_client::external_query_transport::wire_error(
                        "external_acquisition_authority_missing",
                    )
                })?
                .to_owned(),
        );
    }
    Ok(result)
}

pub(super) fn failure_retry(
    error: &GrpcError,
    retry: &RetryPolicy,
    attempt_ordinal: u32,
) -> (RetryDecision, UnaryContinuation) {
    let decision = retry_decision(error);
    let continuation = match decision {
        RetryDecision::RetryBackoff | RetryDecision::RetryBounded
            if attempt_ordinal < retry.max_attempts =>
        {
            let millis = retry
                .backoff(attempt_ordinal)
                .as_millis()
                .try_into()
                .unwrap_or(u64::MAX);
            UnaryContinuation::Retry { backoff_ms: millis }
        }
        _ => UnaryContinuation::Terminal,
    };
    (decision, continuation)
}

fn status_completion(
    client: &GrpcMarketClient,
    method: crate::grpc_contract::methods::MethodIdentity,
    request_id: &str,
    attempt_ordinal: u32,
    status: tonic::Status,
    external_wire: Option<ExternalWireEvidenceV1>,
) -> UnaryAttemptCompletion {
    let (status_code, status_details, status_error_detail_trailer) =
        capture_status_material(&status);
    let error = GrpcError::from_status(status, client.data_status_context(method, request_id));
    let (decision, continuation) = failure_retry(&error, &client.retry, attempt_ordinal);
    UnaryAttemptCompletion {
        response_bytes: None,
        status_code: Some(status_code),
        status_details: Some(status_details),
        status_error_detail_trailer,
        processed: Err(error),
        retry_decision: decision,
        continuation,
        external_wire,
    }
}

/// One already-authorized unary call. Session identity and retry admission are
/// owned by the caller; this function never sleeps or initiates another call.
pub(super) async fn execute(
    mut client: GrpcMarketClient,
    operation: Operation,
    request: ProfileAuthorizedRequest,
    request_id: &str,
    attempt_ordinal: u32,
) -> UnaryAttemptCompletion {
    let method = match crate::grpc_contract::methods::MethodIdentity::from_client_operation(
        client.profile,
        operation,
    ) {
        Ok(method) => method,
        Err(_) => {
            let error = GrpcError::Unimplemented {
                details: Box::default(),
            };
            let (decision, continuation) = failure_retry(&error, &client.retry, attempt_ordinal);
            return UnaryAttemptCompletion {
                response_bytes: None,
                status_code: None,
                status_details: None,
                status_error_detail_trailer: UnaryTrailerMaterial::Absent,
                processed: Err(error),
                retry_decision: decision,
                continuation,
                external_wire: None,
            };
        }
    };
    match client.data_call_authorized(operation, request).await {
        DataCallAuthorized::Local(Ok(response)) => {
            let response_bytes = response.encode_to_vec();
            let processed = project_response(
                client.profile,
                client.acquisition_authority.as_deref(),
                request_id,
                operation,
                response,
            );
            UnaryAttemptCompletion {
                response_bytes: Some(response_bytes),
                status_code: None,
                status_details: None,
                status_error_detail_trailer: UnaryTrailerMaterial::Absent,
                processed,
                retry_decision: RetryDecision::NoRetry,
                continuation: UnaryContinuation::Terminal,
                external_wire: None,
            }
        }
        DataCallAuthorized::Local(Err(status)) => {
            status_completion(&client, method, request_id, attempt_ordinal, status, None)
        }
        DataCallAuthorized::External(ExternalQueryCall::UnaryStatus { status, evidence }) => {
            status_completion(
                &client,
                method,
                request_id,
                attempt_ordinal,
                status,
                Some(evidence),
            )
        }
        DataCallAuthorized::External(ExternalQueryCall::Response { message, evidence }) => {
            let response_bytes = evidence.payload().map(ToOwned::to_owned);
            let processed = evidence
                .payload()
                .ok_or_else(|| {
                    crate::grpc_client::external_query_transport::wire_error(
                        "external_response_wire_invalid",
                    )
                })
                .and_then(admit_external_payload)
                .and_then(|()| {
                    let authority = client.acquisition_authority.as_deref().ok_or_else(|| {
                        crate::grpc_client::external_query_transport::wire_error(
                            "external_acquisition_authority_missing",
                        )
                    })?;
                    parse_external_query_response(
                        request_id,
                        operation,
                        authority,
                        message,
                    )
                    .map_err(GrpcError::from)
                });
            UnaryAttemptCompletion {
                response_bytes,
                status_code: None,
                status_details: None,
                status_error_detail_trailer: UnaryTrailerMaterial::Absent,
                processed,
                retry_decision: RetryDecision::NoRetry,
                continuation: UnaryContinuation::Terminal,
                external_wire: Some(evidence),
            }
        }
        DataCallAuthorized::External(ExternalQueryCall::LocalWireFailure { error, evidence }) => {
            UnaryAttemptCompletion {
                response_bytes: None,
                status_code: None,
                status_details: None,
                status_error_detail_trailer: UnaryTrailerMaterial::Absent,
                processed: Err(error),
                retry_decision: RetryDecision::NoRetry,
                continuation: UnaryContinuation::Terminal,
                external_wire: Some(evidence),
            }
        }
        DataCallAuthorized::Rejected(error) => {
            let (retry_decision, continuation) =
                failure_retry(&error, &client.retry, attempt_ordinal);
            UnaryAttemptCompletion {
                response_bytes: None,
                status_code: None,
                status_details: None,
                status_error_detail_trailer: UnaryTrailerMaterial::Absent,
                processed: Err(error),
                retry_decision,
                continuation,
                external_wire: None,
            }
        }
    }
}
