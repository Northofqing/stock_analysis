//! Canonical W15 candidate snapshots. These values do not attest evidence or authorize work.

#![cfg_attr(not(test), allow(dead_code))]

use std::collections::BTreeMap;
use std::fmt;

use crate::monitor::push_job::{
    canonical_digest, canonical_preimage, namespace_value, BusinessDate, CanonicalValue, GitSha40,
    Namespace, ProtectedRef, Sha256Digest, SourceContractId, SourceContractVersion, UtcMicros,
};

use super::activation_readiness::ActivationDeploymentSet;
use super::operational_readiness::{
    DependencyApplicability, DependencyFailure, DependencyKind, DependencyObservation,
    ReadinessAssessment, ReadinessExitDisposition, ReadinessScope, ReadinessStage, ReadinessStatus,
};

pub(crate) use super::operational_readiness::ReadinessEvidenceKind;

const SNAPSHOT_V2_DOMAIN: &str = "OperationalReadinessSnapshot/v2";
const SNAPSHOT_V3_DOMAIN: &str = "OperationalReadinessSnapshot/v3";
const MATERIAL_V2_DOMAIN: &str = "OperationalReadinessMaterial/v2";
const MATERIAL_V3_DOMAIN: &str = "OperationalReadinessMaterial/v3";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReadinessSnapshotContext {
    pub(crate) namespace: Namespace,
    pub(crate) business_date: BusinessDate,
    pub(crate) build_commit: GitSha40,
    pub(crate) activation_generation: u64,
    pub(crate) manifest_sha256: Sha256Digest,
    pub(crate) captured_at: UtcMicros,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ReadinessDeploymentSetContext {
    business_date: BusinessDate,
    captured_at: UtcMicros,
    deployment_set: ActivationDeploymentSet,
}

impl ReadinessDeploymentSetContext {
    pub(super) fn new(
        business_date: BusinessDate,
        captured_at: UtcMicros,
        deployment_set: ActivationDeploymentSet,
    ) -> Self {
        Self {
            business_date,
            captured_at,
            deployment_set,
        }
    }

    pub(super) fn deployment_set(&self) -> &ActivationDeploymentSet {
        &self.deployment_set
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ReadinessSnapshotVersionedContext {
    V2(ReadinessSnapshotContext),
    V3(ReadinessDeploymentSetContext),
}

impl ReadinessSnapshotVersionedContext {
    pub(super) fn namespace(&self) -> &Namespace {
        match self {
            Self::V2(context) => &context.namespace,
            Self::V3(context) => context.deployment_set.namespace(),
        }
    }

    pub(super) fn business_date(&self) -> &BusinessDate {
        match self {
            Self::V2(context) => &context.business_date,
            Self::V3(context) => &context.business_date,
        }
    }

    pub(super) fn captured_at(&self) -> UtcMicros {
        match self {
            Self::V2(context) => context.captured_at,
            Self::V3(context) => context.captured_at,
        }
    }
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

    fn matches(
        &self,
        observation: &DependencyObservation,
        expected_authority: ReadinessEvidenceKind,
    ) -> bool {
        self.dependency_kind == observation.kind()
            && self.kind == expected_authority
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
    #[error("readiness assessment does not match its deployment set")]
    DeploymentSetAssessmentMismatch,
    #[error("readiness deployment-set construction requires an exact catalog")]
    DeploymentSetCatalogRequired,
    #[error(transparent)]
    InvalidAssessment(#[from] super::operational_readiness::ReadinessError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CandidateReadinessSnapshot {
    snapshot_id: Sha256Digest,
    context: ReadinessSnapshotVersionedContext,
    assessment: ReadinessAssessment,
    recovery_event_id: ReadinessRecoveryEventId,
    evidence_refs: Vec<ReadinessEvidenceRef>,
}

impl CandidateReadinessSnapshot {
    /// Preserve the frozen scalar v2 construction interface for existing callers and fixtures.
    pub(crate) fn try_new(
        context: ReadinessSnapshotContext,
        assessment: ReadinessAssessment,
        recovery_event_id: ReadinessRecoveryEventId,
        evidence_refs: Vec<ReadinessEvidenceRef>,
    ) -> Result<Self, ReadinessSnapshotError> {
        Self::try_new_versioned(
            None,
            ReadinessSnapshotVersionedContext::V2(context),
            assessment,
            recovery_event_id,
            evidence_refs,
        )
    }

    pub(super) fn try_new_v3(
        catalog: &crate::monitor::push_job::MachineCatalog,
        context: ReadinessDeploymentSetContext,
        assessment: ReadinessAssessment,
        recovery_event_id: ReadinessRecoveryEventId,
        evidence_refs: Vec<ReadinessEvidenceRef>,
    ) -> Result<Self, ReadinessSnapshotError> {
        Self::try_new_versioned(
            Some(catalog),
            ReadinessSnapshotVersionedContext::V3(context),
            assessment,
            recovery_event_id,
            evidence_refs,
        )
    }

    pub(super) fn try_new_versioned(
        catalog: Option<&crate::monitor::push_job::MachineCatalog>,
        context: ReadinessSnapshotVersionedContext,
        assessment: ReadinessAssessment,
        recovery_event_id: ReadinessRecoveryEventId,
        evidence_refs: Vec<ReadinessEvidenceRef>,
    ) -> Result<Self, ReadinessSnapshotError> {
        if let ReadinessSnapshotVersionedContext::V3(context) = &context {
            let catalog = catalog.ok_or(ReadinessSnapshotError::DeploymentSetCatalogRequired)?;
            let reevaluated = ReadinessAssessment::evaluate_for_deployment_set(
                catalog,
                &context.deployment_set,
                assessment.scope(),
                assessment.stage(),
                assessment.requirements(),
                assessment.observations(),
            )?;
            if reevaluated != assessment {
                return Err(ReadinessSnapshotError::DeploymentSetAssessmentMismatch);
            }
        }
        let evidence_refs = normalize_snapshot_evidence(&assessment, evidence_refs)?;
        let fields = snapshot_fields(&context, &assessment, &recovery_event_id, &evidence_refs);
        let snapshot_id = canonical_digest(snapshot_domain(&context), &fields);
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
    pub(super) fn versioned_context(&self) -> &ReadinessSnapshotVersionedContext {
        &self.context
    }
    pub(crate) fn legacy_context(&self) -> Option<&ReadinessSnapshotContext> {
        match &self.context {
            ReadinessSnapshotVersionedContext::V2(context) => Some(context),
            ReadinessSnapshotVersionedContext::V3(_) => None,
        }
    }
    pub(crate) fn namespace(&self) -> &Namespace {
        self.context.namespace()
    }
    pub(crate) fn captured_at(&self) -> UtcMicros {
        self.context.captured_at()
    }
    pub(super) fn deployment_set(&self) -> Option<&ActivationDeploymentSet> {
        match &self.context {
            ReadinessSnapshotVersionedContext::V2(_) => None,
            ReadinessSnapshotVersionedContext::V3(context) => Some(&context.deployment_set),
        }
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
            snapshot_domain(&self.context),
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
        let requirement = assessment
            .requirements()
            .iter()
            .find(|requirement| requirement.kind == kind)
            .ok_or(ReadinessSnapshotError::InvalidEvidenceSet {
                check: "missing_requirement_for_observation",
                kind,
            })?;
        let evidence = by_kind
            .get(&kind)
            .ok_or(ReadinessSnapshotError::InvalidEvidenceSet {
                check: "missing_evidence_ref",
                kind,
            })?;
        if !evidence.matches(observation, requirement.expected_authority) {
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
    context: &ReadinessSnapshotVersionedContext,
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
    context: &ReadinessSnapshotVersionedContext,
    assessment: &ReadinessAssessment,
    evidence: &[ReadinessEvidenceRef],
) -> Sha256Digest {
    canonical_digest(
        material_domain(context),
        &snapshot_material_fields(context, assessment, evidence),
    )
}

fn snapshot_material_fields(
    context: &ReadinessSnapshotVersionedContext,
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
                    "expected_authority",
                    string(requirement.expected_authority.as_str()),
                ),
                (
                    "applicability",
                    applicability_value(&requirement.applicability),
                ),
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
    let mut fields = BTreeMap::from([
        (
            "schema_version",
            CanonicalValue::Unsigned(match context {
                ReadinessSnapshotVersionedContext::V2(_) => 2,
                ReadinessSnapshotVersionedContext::V3(_) => 3,
            }),
        ),
        ("namespace", namespace_value(context.namespace())),
        ("business_date", string(context.business_date().as_str())),
        (
            "catalog_sha256",
            string(assessment.catalog_sha256().as_str()),
        ),
        (
            "captured_at",
            CanonicalValue::Unsigned(context.captured_at().get() as u64),
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
    ]);
    match context {
        ReadinessSnapshotVersionedContext::V2(context) => {
            fields.insert("build_commit", string(context.build_commit.as_str()));
            fields.insert(
                "activation_generation",
                CanonicalValue::Unsigned(context.activation_generation),
            );
            fields.insert("manifest_sha256", string(context.manifest_sha256.as_str()));
        }
        ReadinessSnapshotVersionedContext::V3(context) => {
            fields.insert("deployment_set", context.deployment_set.canonical_value());
            fields.insert(
                "deployment_set_sha256",
                string(context.deployment_set.sha256().as_str()),
            );
        }
    }
    fields
}

fn snapshot_domain(context: &ReadinessSnapshotVersionedContext) -> &'static str {
    match context {
        ReadinessSnapshotVersionedContext::V2(_) => SNAPSHOT_V2_DOMAIN,
        ReadinessSnapshotVersionedContext::V3(_) => SNAPSHOT_V3_DOMAIN,
    }
}

fn material_domain(context: &ReadinessSnapshotVersionedContext) -> &'static str {
    match context {
        ReadinessSnapshotVersionedContext::V2(_) => MATERIAL_V2_DOMAIN,
        ReadinessSnapshotVersionedContext::V3(_) => MATERIAL_V3_DOMAIN,
    }
}

fn string(value: &str) -> CanonicalValue {
    CanonicalValue::String(value.to_owned())
}

pub(super) fn scope_value(scope: &ReadinessScope) -> CanonicalValue {
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
    let (status, reason, basis_sha256) = match observation {
        DependencyObservation::Available { .. } => {
            ("Available", CanonicalValue::Null, CanonicalValue::Null)
        }
        DependencyObservation::Unavailable { reason, .. } => {
            ("Unavailable", string(reason.as_str()), CanonicalValue::Null)
        }
        DependencyObservation::NotRequired { basis_sha256, .. } => (
            "NotRequired",
            CanonicalValue::Null,
            string(basis_sha256.as_str()),
        ),
    };
    CanonicalValue::Object(BTreeMap::from([
        ("status", string(status)),
        ("reason", reason),
        ("basis_sha256", basis_sha256),
        ("contract_id", string(observation.contract_id().as_str())),
        ("version", string(observation.version().as_str())),
        (
            "evidence_sha256",
            string(observation.evidence_sha256().as_str()),
        ),
    ]))
}

fn applicability_value(applicability: &DependencyApplicability) -> CanonicalValue {
    let (mode, basis_sha256) = match applicability {
        DependencyApplicability::Required => ("Required", CanonicalValue::Null),
        DependencyApplicability::NotRequired { basis_sha256 } => {
            ("NotRequired", string(basis_sha256.as_str()))
        }
    };
    CanonicalValue::Object(BTreeMap::from([
        ("mode", string(mode)),
        ("basis_sha256", basis_sha256),
    ]))
}

fn failure_value(kind: DependencyKind, failure: DependencyFailure) -> CanonicalValue {
    let (failure, reason) = match failure {
        DependencyFailure::MissingDeclaration => ("MissingDeclaration", CanonicalValue::Null),
        DependencyFailure::MissingEvidence => ("MissingEvidence", CanonicalValue::Null),
        DependencyFailure::ContractMismatch => ("ContractMismatch", CanonicalValue::Null),
        DependencyFailure::VersionMismatch => ("VersionMismatch", CanonicalValue::Null),
        DependencyFailure::ApplicabilityMismatch => ("ApplicabilityMismatch", CanonicalValue::Null),
        DependencyFailure::Unavailable { reason } => ("Unavailable", string(reason.as_str())),
    };
    CanonicalValue::Object(BTreeMap::from([
        ("kind", string(kind.as_str())),
        ("failure", string(failure)),
        ("reason", reason),
    ]))
}
