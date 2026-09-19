use serde::{Deserialize, Serialize};

use crate::data_gateway::grpc_source::BoardAttemptCompletion;
use crate::grpc_client::client::ContractProfile;
use crate::grpc_client::pb::magic::market::v1::QueryRequest;

use super::ChainPostCloseError;

#[derive(Clone, Eq, PartialEq, Deserialize, Serialize)]
struct RequestEnvelope {
    schema_version: u32,
    code: String,
    operation: String,
    request_id: String,
    request_wire: Vec<u8>,
    profile: String,
    acquisition_authority: Option<String>,
    retry_max_attempts: u32,
    retry_base_delay_ms: u64,
    retry_max_delay_ms: u64,
    retry_jitter_ms: u64,
}

pub(super) struct RestoredRequest {
    pub(super) code: String,
    pub(super) request_id: String,
    pub(super) request: QueryRequest,
    pub(super) profile: ContractProfile,
    pub(super) acquisition_authority: Option<String>,
    pub(super) retry_policy: (u32, u64, u64, u64),
}

#[derive(Clone, Eq, PartialEq, Deserialize, Serialize)]
struct FinalEnvelope {
    schema_version: u32,
    outcome: String,
    raw: String,
}

pub(super) fn request_bytes(
    code: &str,
    request_id: &str,
    request_wire: Vec<u8>,
    profile: &str,
    acquisition_authority: Option<&str>,
    retry_policy: (u32, u64, u64, u64),
) -> Result<Vec<u8>, ChainPostCloseError> {
    encode(&RequestEnvelope {
        schema_version: 1,
        code: code.to_owned(),
        operation: "BoardConstituents".to_owned(),
        request_id: request_id.to_owned(),
        request_wire,
        profile: profile.to_owned(),
        acquisition_authority: acquisition_authority.map(str::to_owned),
        retry_max_attempts: retry_policy.0,
        retry_base_delay_ms: retry_policy.1,
        retry_max_delay_ms: retry_policy.2,
        retry_jitter_ms: retry_policy.3,
    })
}

pub(super) fn decode_request(bytes: &[u8]) -> Result<RestoredRequest, ChainPostCloseError> {
    let envelope: RequestEnvelope = decode(bytes)?;
    if envelope.schema_version != 1
        || envelope.operation != "BoardConstituents"
        || envelope.code.is_empty()
        || envelope.code.len() > 512
        || envelope.request_id.is_empty()
        || envelope.request_id.len() > 512
        || envelope.retry_max_attempts == 0
    {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    let request = <QueryRequest as prost::Message>::decode(envelope.request_wire.as_slice())
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    if <QueryRequest as prost::Message>::encode_to_vec(&request) != envelope.request_wire {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    let context = request
        .context
        .as_ref()
        .ok_or(ChainPostCloseError::SchemaRejected)?;
    let payload = request
        .payload
        .as_ref()
        .ok_or(ChainPostCloseError::SchemaRejected)?;
    let expected_body = serde_json::to_vec(&serde_json::json!({ "codes": [&envelope.code] }))
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    if context.protocol_version != 1
        || context.request_id != envelope.request_id
        || payload.schema != "board.constituents"
        || payload.schema_version != 1
        || payload.content_type != "application/json; charset=utf-8"
        || payload.data != expected_body
        || !request.preferred_provider.is_empty()
        || request.allow_unadmitted
    {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    let profile = match envelope.profile.as_str() {
        "LocalBridgeV1" => ContractProfile::LocalBridgeV1,
        // The delivered ExternalV1 builder has no BoardConstituents contract.
        _ => return Err(ChainPostCloseError::SchemaRejected),
    };
    Ok(RestoredRequest {
        code: envelope.code,
        request_id: envelope.request_id,
        request,
        profile,
        acquisition_authority: envelope.acquisition_authority,
        retry_policy: (
            envelope.retry_max_attempts,
            envelope.retry_base_delay_ms,
            envelope.retry_max_delay_ms,
            envelope.retry_jitter_ms,
        ),
    })
}

pub(super) fn result_bytes(
    completion: &BoardAttemptCompletion,
) -> Result<Vec<u8>, ChainPostCloseError> {
    super::board_codec::result_bytes(completion)
}

pub(super) fn decode_result(
    bytes: &[u8],
) -> Result<super::board_codec::ResultEnvelope, ChainPostCloseError> {
    super::board_codec::decode_result(bytes)
}

pub(super) fn final_bytes(outcome: &str, raw: String) -> Result<Vec<u8>, ChainPostCloseError> {
    if !matches!(outcome, "Available" | "VerifiedEmpty" | "Error") {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    encode(&FinalEnvelope {
        schema_version: 1,
        outcome: outcome.to_owned(),
        raw,
    })
}

pub(super) fn decode_final(bytes: &[u8]) -> Result<(String, String), ChainPostCloseError> {
    let envelope: FinalEnvelope = decode(bytes)?;
    if envelope.schema_version != 1
        || !matches!(
            envelope.outcome.as_str(),
            "Available" | "VerifiedEmpty" | "Error"
        )
    {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    Ok((envelope.outcome, envelope.raw))
}

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, ChainPostCloseError> {
    serde_json::to_vec(value).map_err(|_| ChainPostCloseError::SchemaRejected)
}

fn decode<T>(bytes: &[u8]) -> Result<T, ChainPostCloseError>
where
    T: serde::de::DeserializeOwned + Serialize,
{
    let value = serde_json::from_slice(bytes).map_err(|_| ChainPostCloseError::SchemaRejected)?;
    if encode(&value)? != bytes {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    Ok(value)
}
