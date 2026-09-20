//! W15 event/snapshot material with acyclic identity. No source authentication or work permit.

#![cfg_attr(not(test), allow(dead_code))]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::monitor::push_job::{
    canonical_digest, canonical_preimage, raw_digest, CanonicalValue, MachineCatalog, Sha256Digest,
    UtcMicros,
};

use super::operational_readiness::{
    DependencyKind, DependencyRequirement, ReadinessAssessment, ReadinessStatus,
};
use super::readiness_snapshot::{
    normalize_snapshot_evidence, observation_value, snapshot_material_digest,
    CandidateReadinessSnapshot, ReadinessDeploymentSetContext, ReadinessEvidenceRef,
    ReadinessRecoveryEventId, ReadinessSnapshotContext, ReadinessSnapshotError,
    ReadinessSnapshotVersionedContext,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReadinessRecoveryKind {
    Pending,
    ReadyObserved,
    CoreDependenciesRestored,
    ProducerContractRestored,
    InputEvidenceRestored,
}

impl ReadinessRecoveryKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::ReadyObserved => "ReadyObserved",
            Self::CoreDependenciesRestored => "CoreDependenciesRestored",
            Self::ProducerContractRestored => "ProducerContractRestored",
            Self::InputEvidenceRestored => "InputEvidenceRestored",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum ReadinessRecoveryError {
    #[error("readiness recovery context does not match the previous snapshot")]
    ContextMismatch,
    #[error("readiness recovery capture time moved backwards")]
    TimeRegression,
    #[error("readiness recovery needs an explicit capability or version claim")]
    ExplicitRecoveryRequired,
    #[error("readiness observation contains unsupported recovery claims")]
    UnexpectedClaims,
    #[error("invalid readiness recovery claim: {check} ({kind:?})")]
    InvalidClaims {
        check: &'static str,
        kind: DependencyKind,
    },
    #[error(transparent)]
    InvalidSnapshot(#[from] ReadinessSnapshotError),
}

/// Caller-supplied material, not an authenticated capability.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CandidateRecoveryClaim {
    pub(crate) evidence: ReadinessEvidenceRef,
    pub(crate) observed_at: UtcMicros,
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct CandidateReadinessRecord {
    snapshot: CandidateReadinessSnapshot,
    kind: ReadinessRecoveryKind,
    before_snapshot_id: Option<Sha256Digest>,
    event_sha256: Sha256Digest,
    event_bytes: Vec<u8>,
}

impl fmt::Debug for CandidateReadinessRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CandidateReadinessRecord")
            .field("snapshot", &self.snapshot)
            .field("kind", &self.kind)
            .field("before_snapshot_id", &self.before_snapshot_id)
            .field("event_sha256", &self.event_sha256)
            .finish_non_exhaustive()
    }
}

impl CandidateReadinessRecord {
    /// Preserve the frozen scalar v2 construction interface for existing callers and fixtures.
    pub(crate) fn try_new(
        before: Option<&CandidateReadinessSnapshot>,
        context: ReadinessSnapshotContext,
        assessment: ReadinessAssessment,
        evidence: Vec<ReadinessEvidenceRef>,
        claims: Vec<CandidateRecoveryClaim>,
    ) -> Result<Self, ReadinessRecoveryError> {
        Self::try_new_versioned(
            None,
            before,
            ReadinessSnapshotVersionedContext::V2(context),
            assessment,
            evidence,
            claims,
        )
    }

    pub(super) fn try_new_v3(
        catalog: &MachineCatalog,
        before: Option<&CandidateReadinessSnapshot>,
        context: ReadinessDeploymentSetContext,
        assessment: ReadinessAssessment,
        evidence: Vec<ReadinessEvidenceRef>,
        claims: Vec<CandidateRecoveryClaim>,
    ) -> Result<Self, ReadinessRecoveryError> {
        Self::try_new_versioned(
            Some(catalog),
            before,
            ReadinessSnapshotVersionedContext::V3(context),
            assessment,
            evidence,
            claims,
        )
    }

    pub(super) fn try_new_versioned(
        catalog: Option<&MachineCatalog>,
        before: Option<&CandidateReadinessSnapshot>,
        context: ReadinessSnapshotVersionedContext,
        assessment: ReadinessAssessment,
        evidence: Vec<ReadinessEvidenceRef>,
        claims: Vec<CandidateRecoveryClaim>,
    ) -> Result<Self, ReadinessRecoveryError> {
        if let Some(before) = before {
            validate_continuity(before, &context, &assessment)?;
        }
        let evidence = normalize_snapshot_evidence(&assessment, evidence)?;
        let (kind, claims) = classify_claims(before, &context, &assessment, &evidence, claims)?;
        let before_snapshot_id = before.map(|value| value.snapshot_id().clone());
        let mut fields = BTreeMap::from([
            ("schema_version", CanonicalValue::Unsigned(1)),
            ("kind", string(kind.as_str())),
            (
                "before_snapshot_id",
                before_snapshot_id
                    .as_ref()
                    .map(|id| string(id.as_str()))
                    .unwrap_or(CanonicalValue::Null),
            ),
            (
                "after_material_sha256",
                string(snapshot_material_digest(&context, &assessment, &evidence).as_str()),
            ),
            (
                "dependency_changes",
                CanonicalValue::Array(dependency_changes(before, &assessment)),
            ),
            ("recovery_claims", CanonicalValue::Array(claims)),
        ]);
        let event_id = ReadinessRecoveryEventId::from_digest(canonical_digest(
            "OperationalReadinessRecoveryIdentity/v1",
            &fields,
        ));
        let snapshot = CandidateReadinessSnapshot::try_new_versioned(
            catalog,
            context,
            assessment,
            event_id.clone(),
            evidence,
        )?;
        fields.insert("event_id", string(event_id.as_str()));
        fields.insert("after_snapshot_id", string(snapshot.snapshot_id().as_str()));
        let event_bytes = canonical_preimage("OperationalReadinessRecoveryEvent/v1", &fields);
        Ok(Self {
            snapshot,
            kind,
            before_snapshot_id,
            event_sha256: raw_digest(&event_bytes),
            event_bytes,
        })
    }

    pub(crate) fn snapshot(&self) -> &CandidateReadinessSnapshot {
        &self.snapshot
    }
    pub(crate) fn kind(&self) -> ReadinessRecoveryKind {
        self.kind
    }
    pub(crate) fn before_snapshot_id(&self) -> Option<&Sha256Digest> {
        self.before_snapshot_id.as_ref()
    }
    pub(crate) fn event_sha256(&self) -> &Sha256Digest {
        &self.event_sha256
    }
    /// Protected persistence bytes; may contain protected URIs and must never enter errors or probes.
    pub(crate) fn event_bytes(&self) -> Vec<u8> {
        self.event_bytes.clone()
    }
}

fn classify_claims(
    before: Option<&CandidateReadinessSnapshot>,
    context: &ReadinessSnapshotVersionedContext,
    assessment: &ReadinessAssessment,
    evidence: &[ReadinessEvidenceRef],
    claims: Vec<CandidateRecoveryClaim>,
) -> Result<(ReadinessRecoveryKind, Vec<CanonicalValue>), ReadinessRecoveryError> {
    let recovering = before.filter(|previous| {
        previous.assessment().status() != ReadinessStatus::Ready
            && assessment.status() == ReadinessStatus::Ready
    });
    let Some(before) = recovering else {
        if !claims.is_empty() {
            return Err(ReadinessRecoveryError::UnexpectedClaims);
        }
        return Ok((
            if assessment.status() == ReadinessStatus::Ready {
                ReadinessRecoveryKind::ReadyObserved
            } else {
                ReadinessRecoveryKind::Pending
            },
            vec![],
        ));
    };
    if claims.is_empty() {
        return Err(ReadinessRecoveryError::ExplicitRecoveryRequired);
    }
    let required: BTreeSet<_> = before
        .assessment()
        .failures()
        .iter()
        .map(|(kind, _)| *kind)
        .collect();
    let mut by_kind = BTreeMap::new();
    for claim in claims {
        let kind = claim.evidence.dependency_kind();
        let invalid = |check| ReadinessRecoveryError::InvalidClaims { check, kind };
        if !required.contains(&kind) {
            return Err(invalid("unexpected_dependency"));
        }
        if !evidence.contains(&claim.evidence) {
            return Err(invalid("after_evidence_mismatch"));
        }
        if claim.observed_at < before.captured_at() || claim.observed_at > context.captured_at() {
            return Err(invalid("observation_time"));
        }
        if by_kind.insert(kind, claim).is_some() {
            return Err(invalid("duplicate_dependency"));
        }
    }
    for kind in required {
        if !by_kind.contains_key(&kind) {
            return Err(ReadinessRecoveryError::InvalidClaims {
                check: "missing_dependency",
                kind,
            });
        }
    }
    let kind = match before.assessment().status() {
        ReadinessStatus::CoreUnready => ReadinessRecoveryKind::CoreDependenciesRestored,
        ReadinessStatus::ProducerUnready => ReadinessRecoveryKind::ProducerContractRestored,
        ReadinessStatus::BlockedOnInput => ReadinessRecoveryKind::InputEvidenceRestored,
        ReadinessStatus::Ready => return Err(ReadinessRecoveryError::UnexpectedClaims),
    };
    Ok((
        kind,
        by_kind
            .into_values()
            .map(|claim| {
                CanonicalValue::Object(BTreeMap::from([
                    ("evidence", claim.evidence.canonical_value()),
                    (
                        "observed_at",
                        CanonicalValue::Unsigned(claim.observed_at.get() as u64),
                    ),
                ]))
            })
            .collect(),
    ))
}

fn dependency_changes(
    before: Option<&CandidateReadinessSnapshot>,
    after: &ReadinessAssessment,
) -> Vec<CanonicalValue> {
    let Some(before) = before else {
        return vec![];
    };
    let before = before.assessment();
    let roles: BTreeSet<_> = before
        .requirements()
        .iter()
        .chain(after.requirements())
        .map(|requirement| requirement.kind)
        .collect();
    roles
        .into_iter()
        .filter_map(|kind| {
            let old_requirement = before
                .requirements()
                .iter()
                .find(|value| value.kind == kind);
            let new_requirement = after.requirements().iter().find(|value| value.kind == kind);
            let old_observation = before
                .observations()
                .iter()
                .find(|value| value.kind() == kind);
            let new_observation = after
                .observations()
                .iter()
                .find(|value| value.kind() == kind);
            if old_requirement == new_requirement && old_observation == new_observation {
                return None;
            }
            Some(CanonicalValue::Object(BTreeMap::from([
                ("kind", string(kind.as_str())),
                ("before_requirement", requirement_value(old_requirement)),
                ("after_requirement", requirement_value(new_requirement)),
                (
                    "before_observation",
                    old_observation
                        .map(observation_value)
                        .unwrap_or(CanonicalValue::Null),
                ),
                (
                    "after_observation",
                    new_observation
                        .map(observation_value)
                        .unwrap_or(CanonicalValue::Null),
                ),
            ])))
        })
        .collect()
}

fn requirement_value(value: Option<&DependencyRequirement>) -> CanonicalValue {
    value
        .map(|value| {
            CanonicalValue::Object(BTreeMap::from([
                ("contract_id", string(value.contract_id.as_str())),
                ("version", string(value.version.as_str())),
            ]))
        })
        .unwrap_or(CanonicalValue::Null)
}

fn validate_continuity(
    before: &CandidateReadinessSnapshot,
    after: &ReadinessSnapshotVersionedContext,
    assessment: &ReadinessAssessment,
) -> Result<(), ReadinessRecoveryError> {
    let prior = before.versioned_context();
    let version_identity_matches = match (prior, after) {
        (
            ReadinessSnapshotVersionedContext::V2(prior),
            ReadinessSnapshotVersionedContext::V2(after),
        ) => {
            prior.build_commit == after.build_commit
                && prior.activation_generation == after.activation_generation
                && prior.manifest_sha256 == after.manifest_sha256
        }
        (
            ReadinessSnapshotVersionedContext::V3(prior),
            ReadinessSnapshotVersionedContext::V3(after),
        ) => prior.deployment_set() == after.deployment_set(),
        _ => false,
    };
    if !version_identity_matches
        || prior.namespace() != after.namespace()
        || prior.business_date() != after.business_date()
        || before.assessment().catalog_sha256() != assessment.catalog_sha256()
        || before.assessment().scope() != assessment.scope()
        || before.assessment().enabled_producers() != assessment.enabled_producers()
    {
        return Err(ReadinessRecoveryError::ContextMismatch);
    }
    if after.captured_at() < prior.captured_at() {
        return Err(ReadinessRecoveryError::TimeRegression);
    }
    Ok(())
}

fn string(value: &str) -> CanonicalValue {
    CanonicalValue::String(value.to_owned())
}
