//! Strict candidate event reconstruction. Integrity does not authenticate recovery sources.

#![cfg_attr(not(test), allow(dead_code))]

use serde_json::Value;

use crate::monitor::push_job::{raw_digest, Sha256Digest, UtcMicros};

use super::readiness_recovery::{
    CandidateReadinessRecord, CandidateRecoveryClaim, ReadinessRecoveryError,
};
use super::readiness_snapshot::CandidateReadinessSnapshot;
use super::readiness_snapshot_codec::{decode_evidence, ReadinessDecodeError};

const DOMAIN_PREFIX: &[u8] = b"OperationalReadinessRecoveryEvent/v1\0";

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum ReadinessRecordDecodeError {
    #[error("readiness event bytes do not match the expected hash")]
    DigestMismatch,
    #[error("readiness event canonical domain is invalid")]
    InvalidDomain,
    #[error("readiness event JSON is invalid")]
    InvalidJson,
    #[error("readiness event field is invalid: {field}")]
    InvalidField { field: &'static str },
    #[error("readiness event schema version is unsupported")]
    UnsupportedSchemaVersion,
    #[error("readiness event is noncanonical or disagrees with its snapshot joins")]
    InconsistentRecord,
    #[error(transparent)]
    InvalidEvidence(#[from] ReadinessDecodeError),
    #[error(transparent)]
    InvalidRecovery(#[from] ReadinessRecoveryError),
}

/// Rebuild derived identity, changes and both joins from typed snapshots and parsed claims.
/// The caller must separately authenticate evidence and verify the persisted head/chain.
pub(crate) fn decode_readiness_record(
    before: Option<&CandidateReadinessSnapshot>,
    after: &CandidateReadinessSnapshot,
    expected_event_sha: &Sha256Digest,
    bytes: &[u8],
) -> Result<CandidateReadinessRecord, ReadinessRecordDecodeError> {
    if &raw_digest(bytes) != expected_event_sha {
        return Err(ReadinessRecordDecodeError::DigestMismatch);
    }
    let json = bytes
        .strip_prefix(DOMAIN_PREFIX)
        .ok_or(ReadinessRecordDecodeError::InvalidDomain)?;
    let record: Value =
        serde_json::from_slice(json).map_err(|_| ReadinessRecordDecodeError::InvalidJson)?;
    let schema = record.get("schema_version").and_then(Value::as_u64).ok_or(
        ReadinessRecordDecodeError::InvalidField {
            field: "schema_version",
        },
    )?;
    if schema != 1 {
        return Err(ReadinessRecordDecodeError::UnsupportedSchemaVersion);
    }
    let claims = record
        .get("recovery_claims")
        .and_then(Value::as_array)
        .ok_or(ReadinessRecordDecodeError::InvalidField {
            field: "recovery_claims",
        })?
        .iter()
        .map(decode_claim)
        .collect::<Result<Vec<_>, _>>()?;
    let rebuilt = CandidateReadinessRecord::try_new(
        before,
        after.context().clone(),
        after.assessment().clone(),
        after.evidence_refs().to_vec(),
        claims,
    )?;
    // Exact reencoding also rejects ignored/duplicate keys, fabricated dependency deltas,
    // recovery-kind drift and noncanonical encodings, even with a recomputed body hash.
    if rebuilt.snapshot() != after || rebuilt.event_bytes() != bytes {
        return Err(ReadinessRecordDecodeError::InconsistentRecord);
    }
    Ok(rebuilt)
}

fn decode_claim(value: &Value) -> Result<CandidateRecoveryClaim, ReadinessRecordDecodeError> {
    let evidence = value
        .get("evidence")
        .ok_or(ReadinessRecordDecodeError::InvalidField { field: "evidence" })?;
    let observed_at = value
        .get("observed_at")
        .and_then(Value::as_i64)
        .and_then(|time| UtcMicros::try_new(time).ok())
        .ok_or(ReadinessRecordDecodeError::InvalidField {
            field: "observed_at",
        })?;
    Ok(CandidateRecoveryClaim {
        evidence: decode_evidence(evidence)?,
        observed_at,
    })
}
