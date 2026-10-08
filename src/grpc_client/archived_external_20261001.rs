//! Read-only decoder for the public 2026-10-01.3 contract. This module has no
//! generated RPC clients and cannot mint a current connection qualification.
use super::errors::GrpcError;
use super::external_pb::magic::market::v1 as current;
use super::external_query_transport::wire_error;
use prost::Message;
use sha2::{Digest, Sha256};

#[allow(dead_code)]
mod frozen {
    include!(concat!(
        env!("OUT_DIR"),
        "/external_history_20261001/magic.market.v1.rs"
    ));
}
const DESCRIPTOR: &[u8] = include_bytes!(concat!(
    env!("OUT_DIR"),
    "/external_history_20261001/descriptor.bin"
));
pub(crate) const DESCRIPTOR_SHA256: &str =
    "41db4b931010d7dfed7240dd1713ad85dac91ddee6338265737fcc6970ace90b";

pub(crate) fn accepts_descriptor(recorded: &str) -> bool {
    recorded == DESCRIPTOR_SHA256 && hex::encode(Sha256::digest(DESCRIPTOR)) == DESCRIPTOR_SHA256
}

fn canonical<T: Message + Default>(bytes: &[u8]) -> Result<T, GrpcError> {
    let message = T::decode(bytes).map_err(|_| wire_error("external_response_wire_invalid"))?;
    if message.encode_to_vec() != bytes {
        return Err(wire_error("external_response_wire_invalid"));
    }
    Ok(message)
}

pub(crate) fn request_context(health: bool, bytes: &[u8]) -> Result<(u32, String), GrpcError> {
    let context = if health {
        canonical::<frozen::HealthRequest>(bytes)?.context
    } else {
        canonical::<frozen::CapabilitiesRequest>(bytes)?.context
    }
    .ok_or_else(|| wire_error("external_request_wire_invalid"))?;
    Ok((context.protocol_version, context.request_id))
}

pub(crate) fn query_request(bytes: &[u8]) -> Result<(), GrpcError> {
    canonical::<frozen::QueryRequest>(bytes)?;
    Ok(())
}

// Projection uses only fields known to this frozen release. Raw bytes remain
// unchanged in durable records, including accepted unknown query/status fields.
pub(crate) fn health(bytes: &[u8]) -> Result<current::HealthResponse, GrpcError> {
    let value = canonical::<frozen::HealthResponse>(bytes)?;
    Ok(current::HealthResponse {
        request_id: value.request_id,
        live: value.live,
        ready: value.ready,
        state: value.state,
        observability: value
            .observability
            .map(|observation| current::RuntimeObservability {
                process_started_at_unix_ms: observation.process_started_at_unix_ms,
                uptime_millis: observation.uptime_millis,
                query_started: observation.query_started,
                query_succeeded: observation.query_succeeded,
                query_failed: observation.query_failed,
                query_cancelled: observation.query_cancelled,
                query_in_flight: observation.query_in_flight,
                query_rejected: observation.query_rejected,
                query_timed_out: observation.query_timed_out,
                query_duration_micros_total: observation.query_duration_micros_total,
                query_duration_micros_max: observation.query_duration_micros_max,
                unary_concurrency_limit: observation.unary_concurrency_limit,
                unary_concurrency_available: observation.unary_concurrency_available,
                blocking_concurrency_limit: observation.blocking_concurrency_limit,
                blocking_concurrency_available: observation.blocking_concurrency_available,
            }),
        build_identity: value.build_identity.map(|build| current::BuildIdentity {
            service_version: build.service_version,
            source_revision: build.source_revision,
            contract_sha256: build.contract_sha256,
            binary_sha256: build.binary_sha256,
            identity_error: build.identity_error,
        }),
    })
}

pub(crate) fn capabilities(bytes: &[u8]) -> Result<current::CapabilitiesResponse, GrpcError> {
    let value = canonical::<frozen::CapabilitiesResponse>(bytes)?;
    Ok(current::CapabilitiesResponse {
        request_id: value.request_id,
        capabilities: value
            .capabilities
            .into_iter()
            .map(|cap| current::Capability {
                operation: cap.operation,
                repository_admission: cap.repository_admission,
                runtime_available: cap.runtime_available,
                provider: cap.provider,
                exact_scope: cap.exact_scope,
                blocker: cap.blocker,
                diagnostic_available: cap.diagnostic_available,
            })
            .collect(),
    })
}

pub(crate) fn error_detail(bytes: &[u8]) -> Option<current::ErrorDetail> {
    // Status details historically accepted protobuf unknown fields. Retain
    // that contract; carrier reconciliation remains the caller's job.
    let value = frozen::ErrorDetail::decode(bytes).ok()?;
    Some(current::ErrorDetail {
        request_id: value.request_id,
        operation: value.operation,
        provider: value.provider,
        reason_code: value.reason_code,
        retryable: value.retryable,
        admission: value.admission,
        evidence_code: value.evidence_code,
        evidence_field: value.evidence_field,
        record_index: value.record_index,
        has_record_index: value.has_record_index,
        provider_attempts: value
            .provider_attempts
            .into_iter()
            .map(|attempt| current::ProviderAttemptDetail {
                ordinal: attempt.ordinal,
                provider: attempt.provider,
                outcome: attempt.outcome,
                reason_code: attempt.reason_code,
                retryable: attempt.retryable,
                terminal: attempt.terminal,
            })
            .collect(),
    })
}

pub(crate) fn query(bytes: &[u8]) -> Result<current::QueryResponse, GrpcError> {
    // Preserve the former current decoder's forward-compatible query fields.
    // The caller checks conflicting fields after retaining exact raw bytes.
    let value = frozen::QueryResponse::decode(bytes)
        .map_err(|_| wire_error("external_response_wire_invalid"))?;
    Ok(current::QueryResponse {
        request_id: value.request_id,
        operation: value.operation,
        admission: value.admission,
        selected_provider: value.selected_provider,
        batch_id: value.batch_id,
        complete: value.complete,
        observed_at: value.observed_at,
        source_at: value.source_at,
        diagnostic_blocker: value.diagnostic_blocker,
        records: value
            .records
            .into_iter()
            .map(|record| current::CanonicalPayload {
                schema: record.schema,
                schema_version: record.schema_version,
                content_type: record.content_type,
                data: record.data,
            })
            .collect(),
    })
}
