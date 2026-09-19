use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::data_gateway::grpc_source::{
    BoardAttemptCompletion, BoardContinuation, BoardTrailerMaterial,
};
use crate::data_gateway::{
    BatchEvidence, BoardDirectoryFact, BoardDirectoryRecordEvidence, BoardKind, GatewayBatch,
    GatewayError,
};
use crate::grpc_client::client::ContractProfile;
use crate::grpc_client::retry::RetryDecision;
use crate::market_domain::ProviderId;

use super::ChainPostCloseError;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(super) struct RequestEnvelope {
    schema_version: u32,
    kind: String,
    request_id: String,
    request_wire: Vec<u8>,
    profile: String,
    acquisition_authority: Option<String>,
    retry_max_attempts: u32,
    retry_base_delay_ms: u64,
    retry_max_delay_ms: u64,
    retry_jitter_ms: u64,
}

pub(super) struct RestoredBoardRequest {
    pub(super) profile: ContractProfile,
    pub(super) acquisition_authority: Option<String>,
    pub(super) request_id: String,
    pub(super) request: crate::grpc_client::pb::magic::market::v1::QueryRequest,
    pub(super) retry_policy: (u32, u64, u64, u64),
}

pub(super) struct RestoredStatusWire {
    pub(super) code: i32,
    pub(super) details: Vec<u8>,
    pub(super) trailer: crate::grpc_client::errors::PersistedErrorDetailTrailerOwned,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(super) struct ResultEnvelope {
    schema_version: u32,
    response_wire: Option<Vec<u8>>,
    status_code: Option<i32>,
    status_details: Option<Vec<u8>>,
    status_error_detail_trailer: TrailerEnvelope,
    retry_decision: String,
    continuation: String,
    backoff_ms: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum TrailerEnvelope {
    Absent,
    Bytes(Vec<u8>),
    Malformed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(super) struct BatchEnvelope {
    schema_version: u32,
    outcome: BatchOutcome,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum BatchOutcome {
    Available {
        records: Vec<BoardRecordEnvelope>,
        evidence: EvidenceEnvelope,
    },
    VerifiedEmpty(EvidenceEnvelope),
    Error {
        provider: Option<ProviderId>,
        audit_outcome: String,
        reason_code: String,
        retryable: bool,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct BoardRecordEnvelope {
    code: String,
    name: String,
    kind: String,
    member_count: u32,
    evidence: EvidenceEnvelope,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(super) struct EvidenceEnvelope {
    provider: ProviderId,
    source: String,
    source_at: Option<String>,
    observed_at: String,
    batch_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(super) struct DirectoryEnvelope {
    schema_version: u32,
    pub(super) codes: BTreeMap<String, String>,
    pub(super) evidence: Vec<EvidenceEnvelope>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(super) struct SelectionEnvelope {
    schema_version: u32,
    pub(super) cluster_ordinal: usize,
    pub(super) cluster_concept: String,
    pub(super) selected_code: Option<String>,
}

pub(super) fn request_bytes(
    kind: BoardKind,
    request_id: &str,
    request_wire: Vec<u8>,
    profile: &str,
    acquisition_authority: Option<&str>,
    retry_policy: (u32, u64, u64, u64),
) -> Result<Vec<u8>, ChainPostCloseError> {
    encode(&RequestEnvelope {
        schema_version: 1,
        kind: kind_name(kind).to_owned(),
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

pub(super) fn decode_request(bytes: &[u8]) -> Result<RequestEnvelope, ChainPostCloseError> {
    decode(bytes, |value: &RequestEnvelope| value.schema_version == 1)
}

impl RequestEnvelope {
    pub(super) fn kind(&self) -> Result<BoardKind, ChainPostCloseError> {
        parse_kind(&self.kind)
    }
    pub(super) fn request_id(&self) -> &str {
        &self.request_id
    }
    pub(super) fn request_wire(&self) -> &[u8] {
        &self.request_wire
    }

    pub(super) fn validate_for(
        &self,
        expected_kind: BoardKind,
        expected_limit: u32,
    ) -> Result<RestoredBoardRequest, ChainPostCloseError> {
        use crate::grpc_client::pb::magic::market::v1::QueryRequest;

        if self.kind()? != expected_kind || self.request_id.is_empty() {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        let request = <QueryRequest as prost::Message>::decode(self.request_wire.as_slice())
            .map_err(|_| ChainPostCloseError::SchemaRejected)?;
        if <QueryRequest as prost::Message>::encode_to_vec(&request) != self.request_wire {
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
        let expected_payload = serde_json::json!({
            "kind": format!("{expected_kind:?}"),
            "limit": expected_limit,
        });
        let actual_payload: serde_json::Value = serde_json::from_slice(&payload.data)
            .map_err(|_| ChainPostCloseError::SchemaRejected)?;
        if context.protocol_version != 1
            || context.request_id != self.request_id
            || !request.preferred_provider.is_empty()
            || request.allow_unadmitted
            || payload.schema != "board.directory"
            || payload.schema_version != 1
            || payload.content_type != "application/json; charset=utf-8"
            || actual_payload != expected_payload
        {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        let profile = match self.profile.as_str() {
            "LocalBridgeV1" => ContractProfile::LocalBridgeV1,
            // ExternalV1's delivered request builder has no BoardDirectory contract.
            "ExternalV1" => return Err(ChainPostCloseError::SchemaRejected),
            _ => return Err(ChainPostCloseError::SchemaRejected),
        };
        Ok(RestoredBoardRequest {
            profile,
            acquisition_authority: self.acquisition_authority.clone(),
            request_id: self.request_id.clone(),
            request,
            retry_policy: (
                self.retry_max_attempts,
                self.retry_base_delay_ms,
                self.retry_max_delay_ms,
                self.retry_jitter_ms,
            ),
        })
    }
}

pub(super) fn result_bytes(
    completion: &BoardAttemptCompletion,
) -> Result<Vec<u8>, ChainPostCloseError> {
    let (continuation, backoff_ms) = match completion.continuation {
        BoardContinuation::Retry { backoff_ms } => ("Retry", Some(backoff_ms)),
        BoardContinuation::Terminal => ("Terminal", None),
    };
    let retry_decision = match completion.retry_decision {
        RetryDecision::RetryBackoff => "RetryBackoff",
        RetryDecision::RetryBounded => "RetryBounded",
        RetryDecision::NoRetry => "NoRetry",
    };
    let trailer = match &completion.status_error_detail_trailer {
        BoardTrailerMaterial::Absent => TrailerEnvelope::Absent,
        BoardTrailerMaterial::Bytes(bytes) => TrailerEnvelope::Bytes(bytes.clone()),
        BoardTrailerMaterial::Malformed => TrailerEnvelope::Malformed,
    };
    encode(&ResultEnvelope {
        schema_version: 1,
        response_wire: completion.response_bytes.clone(),
        status_code: completion.status_code,
        status_details: completion.status_details.clone(),
        status_error_detail_trailer: trailer,
        retry_decision: retry_decision.to_owned(),
        continuation: continuation.to_owned(),
        backoff_ms,
    })
}

pub(super) fn decode_result(bytes: &[u8]) -> Result<ResultEnvelope, ChainPostCloseError> {
    decode(bytes, |value: &ResultEnvelope| {
        value.schema_version == 1
            && matches!(value.continuation.as_str(), "Retry" | "Terminal")
            && matches!(
                value.retry_decision.as_str(),
                "RetryBackoff" | "RetryBounded" | "NoRetry"
            )
    })
}

impl ResultEnvelope {
    pub(super) fn confirmed_retry_backoff(&self) -> Result<Option<u64>, ChainPostCloseError> {
        if self.continuation != "Retry" {
            return Ok(None);
        }
        let status_code = self
            .status_code
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        if self.response_wire.is_some()
            || !(1..=16).contains(&status_code)
            || self.status_details.is_none()
            || !matches!(
                self.retry_decision.as_str(),
                "RetryBackoff" | "RetryBounded"
            )
            || self.backoff_ms.is_none()
        {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        Ok(self.backoff_ms)
    }

    pub(super) fn terminal_response(
        &self,
    ) -> Result<Option<crate::grpc_client::pb::magic::market::v1::QueryResponse>, ChainPostCloseError>
    {
        use crate::grpc_client::pb::magic::market::v1::QueryResponse;

        let Some(bytes) = &self.response_wire else {
            return Ok(None);
        };
        if self.status_code.is_some()
            || self.status_details.is_some()
            || self.status_error_detail_trailer != TrailerEnvelope::Absent
            || self.retry_decision != "NoRetry"
            || self.continuation != "Terminal"
            || self.backoff_ms.is_some()
        {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        let response = <QueryResponse as prost::Message>::decode(bytes.as_slice())
            .map_err(|_| ChainPostCloseError::SchemaRejected)?;
        if <QueryResponse as prost::Message>::encode_to_vec(&response) != *bytes {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        Ok(Some(response))
    }

    pub(super) fn terminal_status(
        &self,
    ) -> Result<Option<RestoredStatusWire>, ChainPostCloseError> {
        let Some(wire) = self.status_wire()? else {
            return Ok(None);
        };
        if self.continuation != "Terminal"
            || !matches!(
                self.retry_decision.as_str(),
                "RetryBackoff" | "RetryBounded" | "NoRetry"
            )
            || self.backoff_ms.is_some()
        {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        Ok(Some(wire))
    }

    /// Read the original Status transport fields independently of continuation.
    /// The journal validates the frozen retry policy and saved Status A together.
    pub(super) fn status_wire(&self) -> Result<Option<RestoredStatusWire>, ChainPostCloseError> {
        let Some(code) = self.status_code else {
            return Ok(None);
        };
        let details = self
            .status_details
            .clone()
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        if self.response_wire.is_some() || !(1..=16).contains(&code) {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        let trailer = match &self.status_error_detail_trailer {
            TrailerEnvelope::Absent => {
                crate::grpc_client::errors::PersistedErrorDetailTrailerOwned::Absent
            }
            TrailerEnvelope::Bytes(bytes) => {
                crate::grpc_client::errors::PersistedErrorDetailTrailerOwned::Bytes(bytes.clone())
            }
            TrailerEnvelope::Malformed => {
                crate::grpc_client::errors::PersistedErrorDetailTrailerOwned::Malformed
            }
        };
        Ok(Some(RestoredStatusWire {
            code,
            details,
            trailer,
        }))
    }

    pub(super) fn payload_bytes(&self) -> Result<Option<Vec<u8>>, ChainPostCloseError> {
        let Some(bytes) = &self.response_wire else {
            return Ok(None);
        };
        let response =
            <crate::grpc_client::pb::magic::market::v1::QueryResponse as prost::Message>::decode(
                bytes.as_slice(),
            )
            .map_err(|_| ChainPostCloseError::SchemaRejected)?;
        Ok(response.records.first().map(|payload| payload.data.clone()))
    }
    pub(super) fn error_detail_bytes(&self) -> Option<&[u8]> {
        self.status_details
            .as_deref()
            .filter(|bytes| !bytes.is_empty())
    }
    pub(super) fn continuation(&self) -> &str {
        &self.continuation
    }

    pub(super) fn retry_decision(&self) -> &str {
        &self.retry_decision
    }

    pub(super) fn backoff_ms(&self) -> Option<u64> {
        self.backoff_ms
    }
}

pub(super) fn batch_bytes(
    result: &Result<GatewayBatch<BoardDirectoryFact>, GatewayError>,
) -> Result<Vec<u8>, ChainPostCloseError> {
    let outcome = match result {
        Ok(GatewayBatch::Available { records, evidence }) => BatchOutcome::Available {
            records: records.iter().map(BoardRecordEnvelope::from).collect(),
            evidence: EvidenceEnvelope::from(evidence),
        },
        Ok(GatewayBatch::VerifiedEmpty(evidence)) => {
            BatchOutcome::VerifiedEmpty(EvidenceEnvelope::from(evidence))
        }
        Err(error) => BatchOutcome::Error {
            provider: error.provider(),
            audit_outcome: error.audit_outcome().to_owned(),
            reason_code: error.reason_code().to_owned(),
            retryable: error.retryable(),
        },
    };
    encode(&BatchEnvelope {
        schema_version: 1,
        outcome,
    })
}

pub(super) fn decode_batch(bytes: &[u8]) -> Result<BatchEnvelope, ChainPostCloseError> {
    decode(bytes, |value: &BatchEnvelope| value.schema_version == 1)
}

impl BatchEnvelope {
    pub(super) fn outcome_name(&self) -> &'static str {
        match self.outcome {
            BatchOutcome::Available { .. } => "Available",
            BatchOutcome::VerifiedEmpty(_) => "VerifiedEmpty",
            BatchOutcome::Error { .. } => "Error",
        }
    }
    pub(super) fn into_result(self) -> Result<GatewayBatch<BoardDirectoryFact>, GatewayError> {
        match self.outcome {
            BatchOutcome::Available { records, evidence } => Ok(GatewayBatch::Available {
                records: records
                    .into_iter()
                    .map(BoardRecordEnvelope::into_fact)
                    .collect::<Result<_, _>>()
                    .map_err(|_| {
                        GatewayError::unavailable(
                            "BoardDirectory",
                            None,
                            false,
                            "stored board kind is invalid",
                        )
                    })?,
                evidence: evidence.into_batch(),
            }),
            BatchOutcome::VerifiedEmpty(evidence) => {
                Ok(GatewayBatch::VerifiedEmpty(evidence.into_batch()))
            }
            BatchOutcome::Error {
                provider,
                retryable,
                reason_code,
                ..
            } => Err(GatewayError::unavailable(
                "BoardDirectory",
                provider,
                retryable,
                format!("stored board directory failure reason_code={reason_code}"),
            )),
        }
    }
    pub(super) fn evidence(&self) -> Option<EvidenceEnvelope> {
        match &self.outcome {
            BatchOutcome::Available { evidence, .. } | BatchOutcome::VerifiedEmpty(evidence) => {
                Some(evidence.clone())
            }
            BatchOutcome::Error { .. } => None,
        }
    }
}

pub(super) fn directory_bytes(
    codes: &BTreeMap<String, String>,
    evidence: &[BatchEvidence],
) -> Result<Vec<u8>, ChainPostCloseError> {
    encode(&DirectoryEnvelope {
        schema_version: 1,
        codes: codes.clone(),
        evidence: evidence.iter().map(EvidenceEnvelope::from).collect(),
    })
}

pub(super) fn decode_directory(bytes: &[u8]) -> Result<DirectoryEnvelope, ChainPostCloseError> {
    decode(bytes, |value: &DirectoryEnvelope| value.schema_version == 1)
}

pub(super) fn selection_bytes(
    ordinal: usize,
    concept: &str,
    selected_code: Option<&str>,
) -> Result<Vec<u8>, ChainPostCloseError> {
    encode(&SelectionEnvelope {
        schema_version: 1,
        cluster_ordinal: ordinal,
        cluster_concept: concept.to_owned(),
        selected_code: selected_code.map(str::to_owned),
    })
}

pub(super) fn decode_selection(bytes: &[u8]) -> Result<SelectionEnvelope, ChainPostCloseError> {
    decode(bytes, |value: &SelectionEnvelope| value.schema_version == 1)
}

impl From<&BatchEvidence> for EvidenceEnvelope {
    fn from(value: &BatchEvidence) -> Self {
        Self {
            provider: value.provider,
            source: value.source.clone(),
            source_at: value.source_at.clone(),
            observed_at: value.observed_at.clone(),
            batch_id: value.batch_id.clone(),
        }
    }
}

impl EvidenceEnvelope {
    pub(super) fn into_batch(self) -> BatchEvidence {
        BatchEvidence {
            provider: self.provider,
            source: self.source,
            source_at: self.source_at,
            observed_at: self.observed_at,
            batch_id: self.batch_id,
        }
    }
}

impl From<&BoardDirectoryRecordEvidence> for EvidenceEnvelope {
    fn from(value: &BoardDirectoryRecordEvidence) -> Self {
        Self {
            provider: value.provider,
            source: value.source.clone(),
            source_at: value.source_at.clone(),
            observed_at: value.observed_at.clone(),
            batch_id: value.batch_id.clone(),
        }
    }
}

impl From<&BoardDirectoryFact> for BoardRecordEnvelope {
    fn from(value: &BoardDirectoryFact) -> Self {
        Self {
            code: value.code.clone(),
            name: value.name.clone(),
            kind: kind_name(value.kind).to_owned(),
            member_count: value.member_count,
            evidence: EvidenceEnvelope::from(&value.evidence),
        }
    }
}

impl BoardRecordEnvelope {
    fn into_fact(self) -> Result<BoardDirectoryFact, ChainPostCloseError> {
        let evidence = self.evidence.into_batch();
        Ok(BoardDirectoryFact {
            code: self.code,
            name: self.name,
            kind: parse_kind(&self.kind)?,
            member_count: self.member_count,
            evidence: BoardDirectoryRecordEvidence {
                provider: evidence.provider,
                source: evidence.source,
                source_at: evidence.source_at,
                observed_at: evidence.observed_at,
                batch_id: evidence.batch_id,
            },
        })
    }
}

pub(super) fn kind_name(kind: BoardKind) -> &'static str {
    match kind {
        BoardKind::Industry => "Industry",
        BoardKind::Concept => "Concept",
        BoardKind::Region => "Region",
    }
}

pub(super) fn parse_kind(value: &str) -> Result<BoardKind, ChainPostCloseError> {
    match value {
        "Industry" => Ok(BoardKind::Industry),
        "Concept" => Ok(BoardKind::Concept),
        _ => Err(ChainPostCloseError::SchemaRejected),
    }
}

fn encode<T>(value: &T) -> Result<Vec<u8>, ChainPostCloseError>
where
    T: Serialize + for<'de> Deserialize<'de> + PartialEq,
{
    let bytes = serde_json::to_vec(value).map_err(|_| ChainPostCloseError::SchemaRejected)?;
    let decoded: T =
        serde_json::from_slice(&bytes).map_err(|_| ChainPostCloseError::SchemaRejected)?;
    if decoded != *value {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    Ok(bytes)
}

fn decode<T>(bytes: &[u8], valid: impl FnOnce(&T) -> bool) -> Result<T, ChainPostCloseError>
where
    T: for<'de> Deserialize<'de> + Serialize + PartialEq,
{
    let value: T =
        serde_json::from_slice(bytes).map_err(|_| ChainPostCloseError::SchemaRejected)?;
    if !valid(&value)
        || serde_json::to_vec(&value).map_err(|_| ChainPostCloseError::SchemaRejected)? != bytes
    {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    Ok(value)
}
