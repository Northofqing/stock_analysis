//! Exact-date External HistoricalBars observations. This isolated boundary does
//! not admit record JSON, persisted coverage, or a point-in-time data set.

use crate::calendar::{resolve_verified_replay_range, VerifiedReplayCalendar};
use crate::grpc_client::client::external_historical_read::{
    ExternalHistoricalObservation, ExternalHistoricalReadClient,
};
use crate::grpc_client::errors::{ErrorDetail, GrpcError, ProviderAttempts};
use crate::grpc_client::external_v1::build_external_historical_bars_query_request;
use crate::market_domain::{AssetClass, Exchange, InstrumentId};
use chrono::{DateTime, FixedOffset, NaiveDate, NaiveTime, Utc};
use prost::Message as _;
use sha2::{Digest, Sha256};
use std::path::Path;

const CAPTURE_MATERIAL: &str = "gateway-observed-external-historical-window-v1";

/// Callers cannot replace the verified date vector or lower its wire limit.
#[derive(Debug, Clone)]
pub(crate) struct HistoricalWindowRequest {
    instrument: InstrumentId,
    calendar: VerifiedReplayCalendar,
    invoked_at: DateTime<Utc>,
}

impl HistoricalWindowRequest {
    pub(crate) fn new(
        instrument: InstrumentId,
        from: NaiveDate,
        to: NaiveDate,
        invoked_at: DateTime<Utc>,
    ) -> Result<Self, GrpcError> {
        if instrument.asset_class() != AssetClass::Equity
            || !matches!(
                instrument.exchange(),
                Exchange::Shanghai | Exchange::Shenzhen
            )
            || instrument.code().len() != 6
            || !instrument.code().bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(request_error(
                "external_historical_instrument_unsupported",
                false,
            ));
        }
        let calendar = resolve_verified_replay_range(from, to)
            .map_err(|error| request_error(error.code(), error.retryable()))?;
        let shanghai = FixedOffset::east_opt(8 * 60 * 60).expect("fixed Shanghai offset is valid");
        let local = invoked_at.with_timezone(&shanghai);
        if to > local.date_naive()
            || (calendar.required_trading_dates().last() == Some(&local.date_naive())
                && local.time()
                    < NaiveTime::from_hms_opt(15, 0, 0).expect("session close time is valid"))
        {
            return Err(request_error(
                "external_historical_window_incomplete",
                false,
            ));
        }
        Ok(Self {
            instrument,
            calendar,
            invoked_at,
        })
    }

    pub(crate) fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }

    pub(crate) fn from(&self) -> NaiveDate {
        self.calendar.target_from()
    }

    pub(crate) fn to(&self) -> NaiveDate {
        self.calendar.target_to()
    }

    pub(crate) fn required_trading_dates(&self) -> &[NaiveDate] {
        self.calendar.required_trading_dates()
    }

    pub(crate) fn calendar_authority_hash(&self) -> &str {
        self.calendar.authority_hash()
    }

    fn query(
        &self,
    ) -> Result<crate::grpc_client::external_v1::ExternalHistoricalBarsQuery, GrpcError> {
        let limit = u32::try_from(self.required_trading_dates().len())
            .map_err(|_| request_error("external_historical_limit_invalid", false))?;
        build_external_historical_bars_query_request(
            self.instrument(),
            self.from(),
            self.to(),
            limit,
        )
        .map_err(|_| request_error("external_historical_request_invalid", false))
    }
}

pub(crate) struct ExternalHistoricalBarsGateway {
    reader: ExternalHistoricalReadClient,
}

impl ExternalHistoricalBarsGateway {
    pub(crate) async fn connect_client_bundle(path: &Path) -> Result<Self, GrpcError> {
        Ok(Self {
            reader: ExternalHistoricalReadClient::connect_client_bundle(path).await?,
        })
    }

    /// Pre-call qualification failures return an error. Every post-call outcome
    /// retains an observation, including envelope rejection and remote status.
    pub(crate) async fn observe_once(
        &mut self,
        request: HistoricalWindowRequest,
    ) -> Result<GatewayObservedHistoricalWindowCapture, GrpcError> {
        let query = request.query()?;
        let observation = self.reader.query_once(query).await?;
        GatewayObservedHistoricalWindowCapture::from_observed(request, observation)
    }
}

/// Immutable observation identity. A successful envelope remains an observed
/// envelope: neither `complete=true` nor date-count limit proves row coverage.
#[derive(Debug)]
pub(crate) struct GatewayObservedHistoricalWindowCapture {
    request: HistoricalWindowRequest,
    observation: ExternalHistoricalObservation,
    capture_hash: String,
}

impl GatewayObservedHistoricalWindowCapture {
    pub(crate) fn request(&self) -> &HistoricalWindowRequest {
        &self.request
    }

    pub(crate) fn observation(&self) -> &ExternalHistoricalObservation {
        &self.observation
    }

    pub(crate) fn capture_hash(&self) -> &str {
        &self.capture_hash
    }

    fn from_observed(
        request: HistoricalWindowRequest,
        observation: ExternalHistoricalObservation,
    ) -> Result<Self, GrpcError> {
        let capture_hash = observed_capture_hash(&request, &observation)?;
        Ok(Self {
            request,
            observation,
            capture_hash,
        })
    }
}

fn request_error(code: &str, retryable: bool) -> GrpcError {
    let details = Box::new(ErrorDetail {
        code: code.to_owned(),
        reason_code: Some(code.to_owned()),
        retryable: Some(retryable),
        ..ErrorDetail::default()
    });
    if retryable {
        GrpcError::Unavailable { details }
    } else {
        GrpcError::FailedPrecondition { details }
    }
}

fn hash_bytes(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
}

fn observed_capture_hash(
    request: &HistoricalWindowRequest,
    observation: &ExternalHistoricalObservation,
) -> Result<String, GrpcError> {
    let serialize = |value: &serde_json::Value| {
        serde_json::to_vec(value)
            .map_err(|_| request_error("external_historical_capture_serialization", false))
    };
    let request_material = serialize(&serde_json::json!({
        "instrument":request.instrument(),
        "from":request.from(),
        "to":request.to(),
        "required_trading_dates":request.required_trading_dates(),
        "calendar_authority_sha256":request.calendar_authority_hash(),
        "invoked_at":request.invoked_at,
        "wire_limit":request.required_trading_dates().len()
    }))?;
    let connection_material = serde_json::to_vec(&observation.connection_identity)
        .map_err(|_| request_error("external_historical_capture_serialization", false))?;
    let wire_material = serde_json::to_vec(&observation.wire)
        .map_err(|_| request_error("external_historical_capture_serialization", false))?;
    let status_material = match &observation.status {
        None => serialize(&serde_json::json!({"status":"Absent"}))?,
        Some(status) => serialize(&serde_json::json!({
            "status":"Observed",
            "code":status.code,
            "message":status.raw_status.message(),
            "details":status.details,
            "error_detail_trailer":status.error_detail_trailer
        }))?,
    };
    let mut hasher = Sha256::new();
    for bytes in [
        CAPTURE_MATERIAL.as_bytes(),
        &request_material,
        &connection_material,
        &observation.health_wire,
        &observation.health.encode_to_vec(),
        &observation.server_build_identity.encode_to_vec(),
        &observation.capabilities_wire,
        &observation.capabilities_response.encode_to_vec(),
        &observation.capability.encode_to_vec(),
        observation.request_id_correlation.as_bytes(),
        &observation.request_bytes,
        &wire_material,
        &status_material,
        &serialize(&observed_result_material(&observation.result))?,
    ] {
        hash_bytes(&mut hasher, bytes);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn observed_result_material(
    result: &Result<crate::grpc_client::envelope::QueryResult, GrpcError>,
) -> serde_json::Value {
    match result {
        Ok(result) => serde_json::json!({
            "outcome":"EnvelopeObserved",
            "admission":match result.admission {
                crate::grpc_client::envelope::QueryAdmission::Unspecified => "Unspecified",
                crate::grpc_client::envelope::QueryAdmission::Admitted => "Admitted",
                crate::grpc_client::envelope::QueryAdmission::Unadmitted => "Unadmitted",
            },
            "selected_provider":result.selected_provider,
            "batch_id":result.batch_id,
            "complete":result.complete,
            "observed_at":result.observed_at,
            "source_at":result.source_at,
            "source":result.source(),
            "provenance":match &result.provenance {
                crate::grpc_client::envelope::AcquisitionProvenance::Missing => "Missing",
                crate::grpc_client::envelope::AcquisitionProvenance::LocalWireSource(_) => "LocalWireSource",
                crate::grpc_client::envelope::AcquisitionProvenance::ExternalMtlsAuthority(_) => "ExternalMtlsAuthority",
            },
            "diagnostic_blocker":result.diagnostic_blocker,
            "records":result.records.iter().map(|record| serde_json::json!({
                "schema":record.schema,
                "schema_version":record.schema_version,
                "content_type":record.content_type,
                "data":record.data
            })).collect::<Vec<_>>()
        }),
        Err(error) => {
            let kind = match error {
                GrpcError::InvalidArgument { .. } => "InvalidArgument",
                GrpcError::Unauthenticated { .. } => "Unauthenticated",
                GrpcError::PermissionDenied { .. } => "PermissionDenied",
                GrpcError::Unimplemented { .. } => "Unimplemented",
                GrpcError::ResourceExhausted { .. } => "ResourceExhausted",
                GrpcError::DeadlineExceeded { .. } => "DeadlineExceeded",
                GrpcError::Unavailable { .. } => "Unavailable",
                GrpcError::FailedPrecondition { .. } => "FailedPrecondition",
                GrpcError::Internal { .. } => "Internal",
                GrpcError::Unknown { .. } => "Unknown",
            };
            let details = error.details();
            let attempts = match &details.provider_attempts {
                ProviderAttempts::Rejected { observed_count } => {
                    serde_json::json!({"rejected_observed_count":observed_count})
                }
                ProviderAttempts::Accepted(attempts) => {
                    serde_json::json!(attempts
                        .iter()
                        .map(|attempt| serde_json::json!({
                            "ordinal":attempt.ordinal,
                        "provider":attempt.provider.as_str(),
                        "provider_supported":attempt.provider.is_supported(),
                        "outcome":attempt.outcome.as_str(),
                        "outcome_supported":attempt.outcome.is_supported(),
                        "reason_code":attempt.reason_code.as_str(),
                        "reason_code_supported":attempt.reason_code.is_supported(),
                            "retryable":attempt.retryable,
                            "terminal":attempt.terminal
                        }))
                        .collect::<Vec<_>>())
                }
            };
            serde_json::json!({
                "outcome":"QueryRejected",
                "kind":kind,
                "code":details.code,
                "request_id":details.request_id,
                "method":details.method.map(|method| serde_json::json!({
                    "profile":match method.profile() {
                        crate::grpc_contract::methods::ContractProfile::LocalBridgeV1 => "LocalBridgeV1",
                        crate::grpc_contract::methods::ContractProfile::ExternalV1 => "ExternalV1",
                    },
                    "operation":method.as_str_name()
                })),
                "provider":details.provider,
                "reason_code":details.reason_code,
                "retryable":details.retryable,
                "admission":details.admission.map(|admission| admission as i32),
                "evidence_code":details.evidence_code.as_ref().map(|code|code.as_str()),
                "evidence_field":details.evidence_field.as_ref().map(|field|field.as_str()),
                "record_index":details.record_index,
                "provider_attempts":attempts,
                "diagnostic_message":details.diagnostic_message.as_ref().map(|message|message.as_str())
            })
        }
    }
}

#[cfg(test)]
#[path = "external_historical_bars_tests.rs"]
mod tests;
