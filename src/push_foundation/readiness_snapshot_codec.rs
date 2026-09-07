//! Strict reconstruction of candidate snapshots; canonical integrity is not evidence authority.

#![cfg_attr(not(test), allow(dead_code))]

use serde_json::Value;

use crate::monitor::push_job::{
    raw_digest, BusinessDate, GitSha40, MachineCatalog, Namespace, OccurrenceId, ProducerId,
    ProtectedRef, ReasonCode, RunId, Sha256Digest, SourceContractId, SourceContractVersion, UnitId,
    UtcMicros,
};

use super::operational_readiness::{
    DependencyApplicability, DependencyKind, DependencyObservation, DependencyRequirement,
    ReadinessAssessment, ReadinessError, ReadinessScope, ReadinessStage,
};
use super::readiness_snapshot::{
    CandidateReadinessSnapshot, ReadinessEvidenceKind, ReadinessEvidenceRef,
    ReadinessRecoveryEventId, ReadinessSnapshotContext, ReadinessSnapshotError,
};

const DOMAIN_PREFIX: &[u8] = b"OperationalReadinessSnapshot/v2\0";

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum ReadinessDecodeError {
    #[error("readiness snapshot bytes do not match the expected hash")]
    DigestMismatch,
    #[error("readiness snapshot canonical domain is invalid")]
    InvalidDomain,
    #[error("readiness snapshot JSON is invalid")]
    InvalidJson,
    #[error("readiness snapshot field is invalid: {field}")]
    InvalidField { field: &'static str },
    #[error("readiness snapshot schema version is unsupported")]
    UnsupportedSchemaVersion,
    #[error("readiness snapshot bytes are noncanonical or disagree with reevaluation")]
    InconsistentSnapshot,
    #[error(transparent)]
    InvalidAssessment(#[from] ReadinessError),
    #[error(transparent)]
    InvalidEvidence(#[from] ReadinessSnapshotError),
}

/// Recompute all derived fields. Never hydrate persisted Ready/affected sets as trusted facts.
pub(crate) fn decode_readiness_snapshot(
    catalog: &MachineCatalog,
    expected_id: &Sha256Digest,
    bytes: &[u8],
) -> Result<CandidateReadinessSnapshot, ReadinessDecodeError> {
    if &raw_digest(bytes) != expected_id {
        return Err(ReadinessDecodeError::DigestMismatch);
    }
    let json = bytes
        .strip_prefix(DOMAIN_PREFIX)
        .ok_or(ReadinessDecodeError::InvalidDomain)?;
    let record: Value =
        serde_json::from_slice(json).map_err(|_| ReadinessDecodeError::InvalidJson)?;
    if unsigned_field(&record, "schema_version")? != 2 {
        return Err(ReadinessDecodeError::UnsupportedSchemaVersion);
    }
    let captured_at = i64::try_from(unsigned_field(&record, "captured_at")?)
        .map_err(|_| invalid("captured_at"))?;
    let context = ReadinessSnapshotContext {
        namespace: decode_namespace(field(&record, "namespace")?)?,
        business_date: BusinessDate::parse(string_field(&record, "business_date")?)
            .map_err(|_| invalid("business_date"))?,
        build_commit: GitSha40::parse(string_field(&record, "build_commit")?)
            .map_err(|_| invalid("build_commit"))?,
        activation_generation: unsigned_field(&record, "activation_generation")?,
        manifest_sha256: digest_field(&record, "manifest_sha256")?,
        captured_at: UtcMicros::try_new(captured_at).map_err(|_| invalid("captured_at"))?,
    };
    let scope = decode_scope(field(&record, "scope")?)?;
    let stage = match string_field(&record, "stage")? {
        "Startup" => ReadinessStage::Startup,
        "Running" => ReadinessStage::Running,
        _ => return Err(invalid("stage")),
    };
    let enabled: Vec<_> = array_field(&record, "enabled_producer_ids")?
        .iter()
        .map(|value| {
            let id = value.as_str().ok_or(invalid("enabled_producer_ids"))?;
            ProducerId::try_new(id.to_owned()).map_err(|_| invalid("enabled_producer_ids"))
        })
        .collect::<Result<_, _>>()?;
    let (requirements, observations) =
        decode_dependencies(array_field(&record, "dependency_refs")?)?;
    let assessment = ReadinessAssessment::evaluate(
        catalog,
        &scope,
        &enabled,
        stage,
        &requirements,
        &observations,
    )?;
    let evidence = array_field(&record, "evidence_refs")?
        .iter()
        .map(decode_evidence)
        .collect::<Result<Vec<_>, _>>()?;
    let event_id =
        ReadinessRecoveryEventId::from_digest(digest_field(&record, "recovery_event_id")?);
    let rebuilt = CandidateReadinessSnapshot::try_new(context, assessment, event_id, evidence)?;

    // This also rejects ignored/duplicate keys, order/format drift, foreign catalog bindings,
    // tampered derived fields, and source material the parser would otherwise discard.
    if rebuilt.canonical_bytes() != bytes {
        return Err(ReadinessDecodeError::InconsistentSnapshot);
    }
    Ok(rebuilt)
}

fn decode_namespace(value: &Value) -> Result<Namespace, ReadinessDecodeError> {
    match string_field(value, "kind")? {
        "Production" if field(value, "run_id")?.is_null() => Ok(Namespace::Production),
        "Test" => {
            let run = RunId::try_new(string_field(value, "run_id")?.to_owned())
                .map_err(|_| invalid("run_id"))?;
            Ok(Namespace::test(run))
        }
        _ => Err(invalid("namespace")),
    }
}

fn decode_scope(value: &Value) -> Result<ReadinessScope, ReadinessDecodeError> {
    let kind = string_field(value, "kind")?;
    if kind == "Core" {
        return Ok(ReadinessScope::Core);
    }
    let unit_id = UnitId::try_new(string_field(value, "unit_id")?.to_owned())
        .map_err(|_| invalid("unit_id"))?;
    let producer_id = ProducerId::try_new(string_field(value, "producer_id")?.to_owned())
        .map_err(|_| invalid("producer_id"))?;
    match kind {
        "Producer" => Ok(ReadinessScope::Producer {
            unit_id,
            producer_id,
        }),
        "Occurrence" => Ok(ReadinessScope::Occurrence {
            unit_id,
            producer_id,
            occurrence_id: OccurrenceId::from_digest(&digest_field(value, "occurrence_id")?),
        }),
        _ => Err(invalid("scope")),
    }
}

fn decode_dependencies(
    values: &[Value],
) -> Result<(Vec<DependencyRequirement>, Vec<DependencyObservation>), ReadinessDecodeError> {
    let mut requirements = Vec::new();
    let mut observations = Vec::new();
    for value in values {
        let kind = dependency_kind(string_field(value, "kind")?)?;
        requirements.push(DependencyRequirement {
            kind,
            contract_id: contract_id(value, "contract_id")?,
            version: contract_version(value, "version")?,
            expected_authority: evidence_kind(value, "expected_authority")?,
            applicability: decode_applicability(field(value, "applicability")?)?,
        });
        let observed = field(value, "observation")?;
        if observed.is_null() {
            continue;
        }
        let contract_id = contract_id(observed, "contract_id")?;
        let version = contract_version(observed, "version")?;
        let evidence_sha256 = digest_field(observed, "evidence_sha256")?;
        observations.push(match string_field(observed, "status")? {
            "Available" => DependencyObservation::Available {
                kind,
                contract_id,
                version,
                evidence_sha256,
            },
            "Unavailable" => DependencyObservation::Unavailable {
                kind,
                contract_id,
                version,
                evidence_sha256,
                reason: ReasonCode::try_from(string_field(observed, "reason")?)
                    .map_err(|_| invalid("reason"))?,
            },
            "NotRequired" => DependencyObservation::NotRequired {
                kind,
                contract_id,
                version,
                evidence_sha256,
                basis_sha256: digest_field(observed, "basis_sha256")?,
            },
            _ => return Err(invalid("observation_status")),
        });
    }
    Ok((requirements, observations))
}

pub(super) fn decode_evidence(value: &Value) -> Result<ReadinessEvidenceRef, ReadinessDecodeError> {
    let dependency_kind = dependency_kind(string_field(value, "dependency_kind")?)?;
    let kind = evidence_kind(value, "kind")?;
    let protected_uri = ProtectedRef::try_new(string_field(value, "protected_uri")?.to_owned())
        .map_err(|_| invalid("protected_uri"))?;
    Ok(ReadinessEvidenceRef::new(
        dependency_kind,
        kind,
        protected_uri,
        digest_field(value, "sha256")?,
        contract_id(value, "source_contract_id")?,
        contract_version(value, "source_contract_version")?,
    ))
}

fn decode_applicability(value: &Value) -> Result<DependencyApplicability, ReadinessDecodeError> {
    match string_field(value, "mode")? {
        "Required" if field(value, "basis_sha256")?.is_null() => {
            Ok(DependencyApplicability::Required)
        }
        "NotRequired" => Ok(DependencyApplicability::NotRequired {
            basis_sha256: digest_field(value, "basis_sha256")?,
        }),
        _ => Err(invalid("applicability")),
    }
}

fn evidence_kind(
    value: &Value,
    key: &'static str,
) -> Result<ReadinessEvidenceKind, ReadinessDecodeError> {
    match string_field(value, key)? {
        "AuthorityArtifact" => Ok(ReadinessEvidenceKind::AuthorityArtifact),
        "DataAcquisitionAudit" => Ok(ReadinessEvidenceKind::DataAcquisitionAudit),
        _ => Err(invalid(if key == "kind" { "evidence_kind" } else { key })),
    }
}

fn dependency_kind(value: &str) -> Result<DependencyKind, ReadinessDecodeError> {
    use DependencyKind::*;
    // Frozen v2 vocabulary: unknown future roles require an explicit schema/codec change.
    [
        Namespace,
        Durable,
        Audit,
        TypedAuthority,
        Schema,
        Manifest,
        ProducerBinding,
        SourceContract,
        ScheduleOrTrigger,
        Presentation,
        DurablePolicy,
        ReceiptStrength,
        FeatureGate,
        CompletionPolicy,
        OccurrenceInput,
    ]
    .into_iter()
    .find(|kind| kind.as_str() == value)
    .ok_or(invalid("dependency_kind"))
}

fn field<'a>(value: &'a Value, key: &'static str) -> Result<&'a Value, ReadinessDecodeError> {
    value.get(key).ok_or(invalid(key))
}

fn string_field<'a>(value: &'a Value, key: &'static str) -> Result<&'a str, ReadinessDecodeError> {
    field(value, key)?.as_str().ok_or(invalid(key))
}

fn unsigned_field(value: &Value, key: &'static str) -> Result<u64, ReadinessDecodeError> {
    field(value, key)?.as_u64().ok_or(invalid(key))
}

fn array_field<'a>(
    value: &'a Value,
    key: &'static str,
) -> Result<&'a [Value], ReadinessDecodeError> {
    field(value, key)?
        .as_array()
        .map(Vec::as_slice)
        .ok_or(invalid(key))
}

fn digest_field(value: &Value, key: &'static str) -> Result<Sha256Digest, ReadinessDecodeError> {
    Sha256Digest::parse(key, string_field(value, key)?).map_err(|_| invalid(key))
}

fn contract_id(value: &Value, key: &'static str) -> Result<SourceContractId, ReadinessDecodeError> {
    SourceContractId::try_new(string_field(value, key)?.to_owned()).map_err(|_| invalid(key))
}

fn contract_version(
    value: &Value,
    key: &'static str,
) -> Result<SourceContractVersion, ReadinessDecodeError> {
    SourceContractVersion::try_new(string_field(value, key)?.to_owned()).map_err(|_| invalid(key))
}

fn invalid(field: &'static str) -> ReadinessDecodeError {
    ReadinessDecodeError::InvalidField { field }
}
