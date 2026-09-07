//! Canonical W15 candidate snapshots. These values do not attest evidence or authorize work.

#![cfg_attr(not(test), allow(dead_code))]

use std::collections::BTreeMap;
use std::fmt;

use crate::monitor::push_job::{
    canonical_digest, canonical_preimage, namespace_value, BusinessDate, CanonicalValue, GitSha40,
    Namespace, ProtectedRef, Sha256Digest, SourceContractId, SourceContractVersion, UtcMicros,
};

use super::operational_readiness::{
    DependencyFailure, DependencyKind, DependencyObservation, ReadinessAssessment,
    ReadinessExitDisposition, ReadinessScope, ReadinessStage, ReadinessStatus,
};

const SNAPSHOT_DOMAIN: &str = "OperationalReadinessSnapshot/v1";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReadinessSnapshotContext {
    pub(crate) namespace: Namespace,
    pub(crate) business_date: BusinessDate,
    pub(crate) build_commit: GitSha40,
    pub(crate) activation_generation: u64,
    pub(crate) manifest_sha256: Sha256Digest,
    pub(crate) captured_at: UtcMicros,
}

/// A candidate reference only. The store must verify the event and both snapshot joins.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReadinessRecoveryEventId(Sha256Digest);

impl ReadinessRecoveryEventId {
    pub(crate) fn from_digest(digest: Sha256Digest) -> Self {
        Self(digest)
    }

    pub(crate) fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReadinessEvidenceKind {
    AuthorityArtifact,
    DataAcquisitionAudit,
}

impl ReadinessEvidenceKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::AuthorityArtifact => "AuthorityArtifact",
            Self::DataAcquisitionAudit => "DataAcquisitionAudit",
        }
    }
}

/// Evidence location is persisted for exact re-query, but never exposed through Debug.
#[derive(Clone, Eq, PartialEq)]
pub(crate) struct ReadinessEvidenceRef {
    dependency_kind: DependencyKind,
    kind: ReadinessEvidenceKind,
    protected_uri: ProtectedRef,
    sha256: Sha256Digest,
    source_contract_id: SourceContractId,
    source_contract_version: SourceContractVersion,
}

impl fmt::Debug for ReadinessEvidenceRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReadinessEvidenceRef")
            .field("dependency_kind", &self.dependency_kind)
            .field("kind", &self.kind)
            .field("sha256", &self.sha256)
            .field("source_contract_id", &self.source_contract_id)
            .field("source_contract_version", &self.source_contract_version)
            .finish_non_exhaustive()
    }
}

impl ReadinessEvidenceRef {
    pub(super) fn dependency_kind(&self) -> DependencyKind {
        self.dependency_kind
    }

    pub(crate) fn new(
        dependency_kind: DependencyKind,
        kind: ReadinessEvidenceKind,
        protected_uri: ProtectedRef,
        sha256: Sha256Digest,
        source_contract_id: SourceContractId,
        source_contract_version: SourceContractVersion,
    ) -> Self {
        Self {
            dependency_kind,
            kind,
            protected_uri,
            sha256,
            source_contract_id,
            source_contract_version,
        }
    }

    fn matches(&self, observation: &DependencyObservation) -> bool {
        self.dependency_kind == observation.kind()
            && &self.sha256 == observation.evidence_sha256()
            && &self.source_contract_id == observation.contract_id()
            && &self.source_contract_version == observation.version()
    }

    pub(super) fn canonical_value(&self) -> CanonicalValue {
        CanonicalValue::Object(BTreeMap::from([
            ("dependency_kind", string(self.dependency_kind.as_str())),
            ("kind", string(self.kind.as_str())),
            ("protected_uri", string(self.protected_uri.as_str())),
            ("sha256", string(self.sha256.as_str())),
            (
                "source_contract_id",
                string(self.source_contract_id.as_str()),
            ),
            (
                "source_contract_version",
                string(self.source_contract_version.as_str()),
            ),
        ]))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum ReadinessSnapshotError {
    #[error("invalid readiness snapshot evidence: {check} ({kind:?})")]
    InvalidEvidenceSet {
        check: &'static str,
        kind: DependencyKind,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CandidateReadinessSnapshot {
    snapshot_id: Sha256Digest,
    context: ReadinessSnapshotContext,
    assessment: ReadinessAssessment,
    recovery_event_id: ReadinessRecoveryEventId,
    evidence_refs: Vec<ReadinessEvidenceRef>,
}

impl CandidateReadinessSnapshot {
    pub(crate) fn try_new(
        context: ReadinessSnapshotContext,
        assessment: ReadinessAssessment,
        recovery_event_id: ReadinessRecoveryEventId,
        evidence_refs: Vec<ReadinessEvidenceRef>,
    ) -> Result<Self, ReadinessSnapshotError> {
        let evidence_refs = normalize_snapshot_evidence(&assessment, evidence_refs)?;
        let fields = snapshot_fields(&context, &assessment, &recovery_event_id, &evidence_refs);
        let snapshot_id = canonical_digest(SNAPSHOT_DOMAIN, &fields);
        Ok(Self {
            snapshot_id,
            context,
            assessment,
            recovery_event_id,
            evidence_refs,
        })
    }

    pub(crate) fn snapshot_id(&self) -> &Sha256Digest {
        &self.snapshot_id
    }
    pub(crate) fn context(&self) -> &ReadinessSnapshotContext {
        &self.context
    }
    pub(crate) fn assessment(&self) -> &ReadinessAssessment {
        &self.assessment
    }
    pub(crate) fn recovery_event_id(&self) -> &ReadinessRecoveryEventId {
        &self.recovery_event_id
    }
    pub(super) fn evidence_refs(&self) -> &[ReadinessEvidenceRef] {
        &self.evidence_refs
    }

    /// Protected persistence bytes; never include these in errors or probe output.
    pub(crate) fn canonical_bytes(&self) -> Vec<u8> {
        canonical_preimage(
            SNAPSHOT_DOMAIN,
            &snapshot_fields(
                &self.context,
                &self.assessment,
                &self.recovery_event_id,
                &self.evidence_refs,
            ),
        )
    }
}

pub(super) fn normalize_snapshot_evidence(
    assessment: &ReadinessAssessment,
    evidence_refs: Vec<ReadinessEvidenceRef>,
) -> Result<Vec<ReadinessEvidenceRef>, ReadinessSnapshotError> {
    let mut by_kind = BTreeMap::new();
    for evidence in evidence_refs {
        let kind = evidence.dependency_kind;
        if by_kind.insert(kind, evidence).is_some() {
            return Err(ReadinessSnapshotError::InvalidEvidenceSet {
                check: "duplicate_evidence",
                kind,
            });
        }
    }
    for observation in assessment.observations() {
        let kind = observation.kind();
        let evidence = by_kind
            .get(&kind)
            .ok_or(ReadinessSnapshotError::InvalidEvidenceSet {
                check: "missing_evidence_ref",
                kind,
            })?;
        if !evidence.matches(observation) {
            return Err(ReadinessSnapshotError::InvalidEvidenceSet {
                check: "observation_evidence_mismatch",
                kind,
            });
        }
    }
    for kind in by_kind.keys() {
        if !assessment
            .observations()
            .iter()
            .any(|observation| observation.kind() == *kind)
        {
            return Err(ReadinessSnapshotError::InvalidEvidenceSet {
                check: "unobserved_evidence",
                kind: *kind,
            });
        }
    }
    Ok(by_kind.into_values().collect())
}

fn snapshot_fields(
    context: &ReadinessSnapshotContext,
    assessment: &ReadinessAssessment,
    event: &ReadinessRecoveryEventId,
    evidence: &[ReadinessEvidenceRef],
) -> BTreeMap<&'static str, CanonicalValue> {
    let mut fields = snapshot_material_fields(context, assessment, evidence);
    fields.insert("recovery_event_id", string(event.as_str()));
    fields
}

/// Hash the full after-material before an event ID exists, without a circular reference.
pub(super) fn snapshot_material_digest(
    context: &ReadinessSnapshotContext,
    assessment: &ReadinessAssessment,
    evidence: &[ReadinessEvidenceRef],
) -> Sha256Digest {
    canonical_digest(
        "OperationalReadinessMaterial/v1",
        &snapshot_material_fields(context, assessment, evidence),
    )
}

fn snapshot_material_fields(
    context: &ReadinessSnapshotContext,
    assessment: &ReadinessAssessment,
    evidence: &[ReadinessEvidenceRef],
) -> BTreeMap<&'static str, CanonicalValue> {
    let observations: BTreeMap<_, _> = assessment
        .observations()
        .iter()
        .map(|observation| (observation.kind(), observation))
        .collect();
    let dependency_refs = assessment
        .requirements()
        .iter()
        .map(|requirement| {
            CanonicalValue::Object(BTreeMap::from([
                ("kind", string(requirement.kind.as_str())),
                ("contract_id", string(requirement.contract_id.as_str())),
                ("version", string(requirement.version.as_str())),
                (
                    "observation",
                    observations
                        .get(&requirement.kind)
                        .map(|observation| observation_value(observation))
                        .unwrap_or(CanonicalValue::Null),
                ),
            ]))
        })
        .collect();
    BTreeMap::from([
        ("schema_version", CanonicalValue::Unsigned(1)),
        ("namespace", namespace_value(&context.namespace)),
        ("business_date", string(context.business_date.as_str())),
        ("build_commit", string(context.build_commit.as_str())),
        (
            "activation_generation",
            CanonicalValue::Unsigned(context.activation_generation),
        ),
        ("manifest_sha256", string(context.manifest_sha256.as_str())),
        (
            "catalog_sha256",
            string(assessment.catalog_sha256().as_str()),
        ),
        (
            "captured_at",
            CanonicalValue::Unsigned(context.captured_at.get() as u64),
        ),
        ("scope", scope_value(assessment.scope())),
        (
            "stage",
            string(match assessment.stage() {
                ReadinessStage::Startup => "Startup",
                ReadinessStage::Running => "Running",
            }),
        ),
        (
            "status",
            string(match assessment.status() {
                ReadinessStatus::Ready => "Ready",
                ReadinessStatus::CoreUnready => "CoreUnready",
                ReadinessStatus::ProducerUnready => "ProducerUnready",
                ReadinessStatus::BlockedOnInput => "BlockedOnInput",
            }),
        ),
        ("reason", string(assessment.reason().as_str())),
        ("dependency_refs", CanonicalValue::Array(dependency_refs)),
        (
            "dependency_failures",
            CanonicalValue::Array(
                assessment
                    .failures()
                    .iter()
                    .map(|(kind, failure)| failure_value(*kind, *failure))
                    .collect(),
            ),
        ),
        (
            "enabled_producer_ids",
            CanonicalValue::Array(
                assessment
                    .enabled_producers()
                    .iter()
                    .map(|producer| string(producer.as_str()))
                    .collect(),
            ),
        ),
        (
            "affected_unit_ids",
            CanonicalValue::Array(
                assessment
                    .affected_unit_ids()
                    .iter()
                    .map(|unit| string(unit.as_str()))
                    .collect(),
            ),
        ),
        (
            "affected_producer_ids",
            CanonicalValue::Array(
                assessment
                    .affected_producer_ids()
                    .iter()
                    .map(|producer| string(producer.as_str()))
                    .collect(),
            ),
        ),
        (
            "evidence_refs",
            CanonicalValue::Array(
                evidence
                    .iter()
                    .map(ReadinessEvidenceRef::canonical_value)
                    .collect(),
            ),
        ),
        ("liveness", CanonicalValue::Bool(assessment.liveness())),
        (
            "deployment_ready",
            CanonicalValue::Bool(assessment.deployment_ready()),
        ),
        (
            "exit_disposition",
            string(match assessment.exit_disposition() {
                ReadinessExitDisposition::Continue => "Continue",
                ReadinessExitDisposition::StartupNonzero => "StartupNonzero",
                ReadinessExitDisposition::StopNewAndRecoverIsolateThenNonzero => {
                    "StopNewAndRecoverIsolateThenNonzero"
                }
                ReadinessExitDisposition::IsolateAffectedContinueOthers => {
                    "IsolateAffectedContinueOthers"
                }
                ReadinessExitDisposition::ContinueWithoutOccurrenceWork => {
                    "ContinueWithoutOccurrenceWork"
                }
            }),
        ),
    ])
}

fn string(value: &str) -> CanonicalValue {
    CanonicalValue::String(value.to_owned())
}

fn scope_value(scope: &ReadinessScope) -> CanonicalValue {
    let mut fields = BTreeMap::new();
    match scope {
        ReadinessScope::Core => {
            fields.insert("kind", string("Core"));
        }
        ReadinessScope::Producer {
            unit_id,
            producer_id,
        } => {
            fields.insert("kind", string("Producer"));
            fields.insert("unit_id", string(unit_id.as_str()));
            fields.insert("producer_id", string(producer_id.as_str()));
        }
        ReadinessScope::Occurrence {
            unit_id,
            producer_id,
            occurrence_id,
        } => {
            fields.insert("kind", string("Occurrence"));
            fields.insert("unit_id", string(unit_id.as_str()));
            fields.insert("producer_id", string(producer_id.as_str()));
            fields.insert("occurrence_id", string(occurrence_id.as_str()));
        }
    }
    CanonicalValue::Object(fields)
}

pub(super) fn observation_value(observation: &DependencyObservation) -> CanonicalValue {
    let (status, reason) = match observation {
        DependencyObservation::Available { .. } => ("Available", CanonicalValue::Null),
        DependencyObservation::Unavailable { reason, .. } => {
            ("Unavailable", string(reason.as_str()))
        }
    };
    CanonicalValue::Object(BTreeMap::from([
        ("status", string(status)),
        ("reason", reason),
        ("contract_id", string(observation.contract_id().as_str())),
        ("version", string(observation.version().as_str())),
        (
            "evidence_sha256",
            string(observation.evidence_sha256().as_str()),
        ),
    ]))
}

fn failure_value(kind: DependencyKind, failure: DependencyFailure) -> CanonicalValue {
    let (failure, reason) = match failure {
        DependencyFailure::MissingDeclaration => ("MissingDeclaration", CanonicalValue::Null),
        DependencyFailure::MissingEvidence => ("MissingEvidence", CanonicalValue::Null),
        DependencyFailure::ContractMismatch => ("ContractMismatch", CanonicalValue::Null),
        DependencyFailure::VersionMismatch => ("VersionMismatch", CanonicalValue::Null),
        DependencyFailure::Unavailable { reason } => ("Unavailable", string(reason.as_str())),
    };
    CanonicalValue::Object(BTreeMap::from([
        ("kind", string(kind.as_str())),
        ("failure", string(failure)),
        ("reason", reason),
    ]))
}
