use serde::{Deserialize, Serialize};

use crate::data_gateway::review::{OwnedGatewayAuditRecord, StoredGatewayError};

use super::ChainPostCloseError;

#[derive(Deserialize, Serialize)]
struct StatusMaterialEnvelope {
    schema_version: u32,
    projection_version: u32,
    safe_diagnostic: Option<String>,
}

#[derive(Deserialize, Serialize)]
struct ErrorMaterialEnvelope {
    schema_version: u32,
    gateway: StoredGatewayError,
    audit: OwnedGatewayAuditRecord,
}

pub(super) fn status_bytes(diagnostic: Option<&str>) -> Result<Vec<u8>, ChainPostCloseError> {
    serde_json::to_vec(&StatusMaterialEnvelope {
        schema_version: 1,
        projection_version: 1,
        safe_diagnostic: diagnostic.map(str::to_owned),
    })
    .map_err(|_| ChainPostCloseError::SchemaRejected)
}

pub(super) fn decode_status(bytes: &[u8]) -> Result<Option<String>, ChainPostCloseError> {
    let value: StatusMaterialEnvelope =
        serde_json::from_slice(bytes).map_err(|_| ChainPostCloseError::SchemaRejected)?;
    if value.schema_version != 1
        || value.projection_version != 1
        || !crate::grpc_client::errors::is_canonical_safe_diagnostic(
            value.safe_diagnostic.as_deref(),
        )
        || status_bytes(value.safe_diagnostic.as_deref())? != bytes
    {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    Ok(value.safe_diagnostic)
}

pub(super) fn error_bytes(
    gateway: StoredGatewayError,
    audit: OwnedGatewayAuditRecord,
) -> Result<Vec<u8>, ChainPostCloseError> {
    serde_json::to_vec(&ErrorMaterialEnvelope {
        schema_version: 1,
        gateway,
        audit,
    })
    .map_err(|_| ChainPostCloseError::SchemaRejected)
}

pub(super) fn decode_error(
    bytes: &[u8],
) -> Result<(StoredGatewayError, OwnedGatewayAuditRecord), ChainPostCloseError> {
    let value: ErrorMaterialEnvelope =
        serde_json::from_slice(bytes).map_err(|_| ChainPostCloseError::SchemaRejected)?;
    if value.schema_version != 1 {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    let encoded = error_bytes(value.gateway.clone(), value.audit.clone())?;
    if encoded != bytes {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    Ok((value.gateway, value.audit))
}
