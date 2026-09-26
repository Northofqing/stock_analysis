//! Read-only decoder for the public 2026-09-17.1 contract. This module has no
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
        "/external_history_20260917/magic.market.v1.rs"
    ));
}
const DESCRIPTOR: &[u8] = include_bytes!(concat!(
    env!("OUT_DIR"),
    "/external_history_20260917/descriptor.bin"
));
pub(crate) const DESCRIPTOR_SHA256: &str =
    "5ba0fa3b2fa450e74bdcc8cb5f163348a6ca90df3f3626d1d8f2ec27137f5edb";

pub(crate) fn accepts_descriptor(recorded: &str) -> bool {
    recorded == DESCRIPTOR_SHA256 && hex::encode(Sha256::digest(DESCRIPTOR)) == DESCRIPTOR_SHA256
}

fn canonical<T: Message + Default>(bytes: &[u8]) -> Result<T, GrpcError> {
    let message = T::decode(bytes).map_err(|_| wire_error("unsupported_historical_wire"))?;
    if message.encode_to_vec() != bytes {
        return Err(wire_error("unsupported_historical_wire"));
    }
    Ok(message)
}

pub(crate) fn request_context(health: bool, bytes: &[u8]) -> Result<(u32, String), GrpcError> {
    let context = if health {
        canonical::<frozen::HealthRequest>(bytes)?.context
    } else {
        canonical::<frozen::CapabilitiesRequest>(bytes)?.context
    }
    .ok_or_else(|| wire_error("unsupported_historical_wire"))?;
    Ok((context.protocol_version, context.request_id))
}

pub(crate) fn query_request(bytes: &[u8]) -> Result<(), GrpcError> {
    canonical::<frozen::QueryRequest>(bytes)?;
    Ok(())
}

pub(crate) fn global_news_request(
    bytes: &[u8],
    id: &str,
    provider: &str,
    limit: u32,
) -> Result<(), GrpcError> {
    if !matches!(
        provider,
        "Eastmoney" | "Cailianpress" | "Jin10" | "ThePaper"
    ) || !(1..=20).contains(&limit)
    {
        return Err(wire_error("unsupported_historical_wire"));
    }
    let value = canonical::<frozen::QueryRequest>(bytes)?;
    let expected = frozen::QueryRequest {
        context: Some(frozen::RequestContext {
            protocol_version: 1,
            request_id: id.into(),
        }),
        preferred_provider: provider.into(),
        payload: Some(frozen::CanonicalPayload {
            schema: "magic.market.global_news.request".into(),
            schema_version: 2,
            content_type: "application/json; charset=utf-8".into(),
            data: serde_json::to_vec(&serde_json::json!({"limit": limit}))
                .map_err(|_| wire_error("unsupported_historical_wire"))?,
        }),
        allow_unadmitted: false,
    };
    if value != expected {
        return Err(wire_error("unsupported_historical_wire"));
    }
    Ok(())
}

// Only stable fields needed by the read-only business projection cross into
// today's domain adapters. Original full bytes (including observability) stay
// in the immutable record and are checked with the frozen decoder above.
pub(crate) fn health(bytes: &[u8]) -> Result<current::HealthResponse, GrpcError> {
    let value = canonical::<frozen::HealthResponse>(bytes)?;
    Ok(current::HealthResponse {
        request_id: value.request_id,
        live: value.live,
        ready: value.ready,
        state: value.state,
        observability: None,
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
    // Historical data accepted forward-compatible protobuf fields except the
    // explicitly conflicting source field. Do not retroactively canonicalize.
    super::external_query_transport::admit_external_payload(bytes)?;
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
