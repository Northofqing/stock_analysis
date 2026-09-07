//! Pure W15 dependency assessment. An assessment is not an attested snapshot or execution permit.

#![cfg_attr(not(test), allow(dead_code))]

use std::collections::{BTreeMap, BTreeSet};

use crate::monitor::push_job::{
    MachineCatalog, OccurrenceId, ProducerId, ReasonCode, Sha256Digest, SourceContractId,
    SourceContractVersion, UnitId,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ReadinessScope {
    Core,
    Producer {
        unit_id: UnitId,
        producer_id: ProducerId,
    },
    Occurrence {
        unit_id: UnitId,
        producer_id: ProducerId,
        occurrence_id: OccurrenceId,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReadinessStage {
    Startup,
    Running,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReadinessStatus {
    Ready,
    CoreUnready,
    ProducerUnready,
    BlockedOnInput,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReadinessExitDisposition {
    Continue,
    StartupNonzero,
    StopNewAndRecoverIsolateThenNonzero,
    IsolateAffectedContinueOthers,
    ContinueWithoutOccurrenceWork,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) enum DependencyKind {
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReadinessEvidenceKind {
    AuthorityArtifact,
    DataAcquisitionAudit,
}

impl ReadinessEvidenceKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::AuthorityArtifact => "AuthorityArtifact",
            Self::DataAcquisitionAudit => "DataAcquisitionAudit",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DependencyApplicability {
    Required,
    NotRequired { basis_sha256: Sha256Digest },
}

impl DependencyKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Namespace => "Namespace",
            Self::Durable => "Durable",
            Self::Audit => "Audit",
            Self::TypedAuthority => "TypedAuthority",
            Self::Schema => "Schema",
            Self::Manifest => "Manifest",
            Self::ProducerBinding => "ProducerBinding",
            Self::SourceContract => "SourceContract",
            Self::ScheduleOrTrigger => "ScheduleOrTrigger",
            Self::Presentation => "Presentation",
            Self::DurablePolicy => "DurablePolicy",
            Self::ReceiptStrength => "ReceiptStrength",
            Self::FeatureGate => "FeatureGate",
            Self::CompletionPolicy => "CompletionPolicy",
            Self::OccurrenceInput => "OccurrenceInput",
        }
    }
}

const CORE_DEPENDENCIES: &[DependencyKind] = &[
    DependencyKind::Namespace,
    DependencyKind::Durable,
    DependencyKind::Audit,
    DependencyKind::TypedAuthority,
    DependencyKind::Schema,
    DependencyKind::Manifest,
];
const PRODUCER_DEPENDENCIES: &[DependencyKind] = &[
    DependencyKind::ProducerBinding,
    DependencyKind::SourceContract,
    DependencyKind::ScheduleOrTrigger,
    DependencyKind::Presentation,
    DependencyKind::DurablePolicy,
    DependencyKind::ReceiptStrength,
    DependencyKind::FeatureGate,
    DependencyKind::CompletionPolicy,
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DependencyRequirement {
    pub(crate) kind: DependencyKind,
    pub(crate) contract_id: SourceContractId,
    pub(crate) version: SourceContractVersion,
    pub(crate) expected_authority: ReadinessEvidenceKind,
    pub(crate) applicability: DependencyApplicability,
}

/// Candidate observations must be independently attested before any snapshot can be published.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DependencyObservation {
    Available {
        kind: DependencyKind,
        contract_id: SourceContractId,
        version: SourceContractVersion,
        evidence_sha256: Sha256Digest,
    },
    Unavailable {
        kind: DependencyKind,
        contract_id: SourceContractId,
        version: SourceContractVersion,
        evidence_sha256: Sha256Digest,
        reason: ReasonCode,
    },
    NotRequired {
        kind: DependencyKind,
        contract_id: SourceContractId,
        version: SourceContractVersion,
        evidence_sha256: Sha256Digest,
        basis_sha256: Sha256Digest,
    },
}

impl DependencyObservation {
    pub(crate) fn kind(&self) -> DependencyKind {
        match self {
            Self::Available { kind, .. }
            | Self::Unavailable { kind, .. }
            | Self::NotRequired { kind, .. } => *kind,
        }
    }

    pub(crate) fn contract_id(&self) -> &SourceContractId {
        match self {
            Self::Available { contract_id, .. }
            | Self::Unavailable { contract_id, .. }
            | Self::NotRequired { contract_id, .. } => contract_id,
        }
    }

    pub(crate) fn version(&self) -> &SourceContractVersion {
        match self {
            Self::Available { version, .. }
            | Self::Unavailable { version, .. }
            | Self::NotRequired { version, .. } => version,
        }
    }

    pub(crate) fn evidence_sha256(&self) -> &Sha256Digest {
        match self {
            Self::Available {
                evidence_sha256, ..
            }
            | Self::Unavailable {
                evidence_sha256, ..
            }
            | Self::NotRequired {
                evidence_sha256, ..
            } => evidence_sha256,
        }
    }

    fn failure_for(&self, requirement: &DependencyRequirement) -> Option<DependencyFailure> {
        match self {
            Self::Available {
                contract_id,
                version,
                ..
            }
            | Self::Unavailable {
                contract_id,
                version,
                ..
            }
            | Self::NotRequired {
                contract_id,
                version,
                ..
            } => {
                if contract_id != &requirement.contract_id {
                    Some(DependencyFailure::ContractMismatch)
                } else if version != &requirement.version {
                    Some(DependencyFailure::VersionMismatch)
                } else {
                    match (&requirement.applicability, self) {
                        (DependencyApplicability::Required, Self::Available { .. }) => None,
                        (DependencyApplicability::Required, Self::Unavailable { reason, .. }) => {
                            Some(DependencyFailure::Unavailable { reason: *reason })
                        }
                        (
                            DependencyApplicability::NotRequired {
                                basis_sha256: expected_basis,
                            },
                            Self::NotRequired {
                                basis_sha256: actual_basis,
                                ..
                            },
                        ) if expected_basis == actual_basis => None,
                        _ => Some(DependencyFailure::ApplicabilityMismatch),
                    }
                }
            }
        }
    }

    fn validate(&self) -> Result<(), ReadinessError> {
        if let Self::Unavailable { kind, reason, .. } = self {
            match unavailable_reason_allows_kind(*reason, *kind) {
                Some(true) => {}
                Some(false) => {
                    return Err(ReadinessError::InvalidDependencySet {
                        check: "incompatible_unavailable_reason",
                        kind: *kind,
                    });
                }
                None => {
                    return Err(ReadinessError::InvalidDependencySet {
                        check: "invalid_unavailable_reason",
                        kind: *kind,
                    });
                }
            }
        }
        Ok(())
    }
}

fn unavailable_reason_allows_kind(reason: ReasonCode, kind: DependencyKind) -> Option<bool> {
    let allowed = match reason {
        ReasonCode::ActivationCoreUnready => CORE_DEPENDENCIES.contains(&kind),
        ReasonCode::ActivationProducerUnready => PRODUCER_DEPENDENCIES.contains(&kind),
        ReasonCode::ActivationManifestMismatch | ReasonCode::ActivationGenerationConflict => {
            kind == DependencyKind::Manifest
        }
        ReasonCode::ActivationOwnerConflict => kind == DependencyKind::ProducerBinding,
        ReasonCode::PolicyDisabled
        | ReasonCode::PolicyStarved
        | ReasonCode::PolicyOptInDisabled => kind == DependencyKind::FeatureGate,
        ReasonCode::InputSourceUnavailable
        | ReasonCode::InputSourceUnready
        | ReasonCode::InputNoVerifiedBatch
        | ReasonCode::InputAccountSnapshotMissing => matches!(
            kind,
            DependencyKind::SourceContract | DependencyKind::OccurrenceInput
        ),
        ReasonCode::InputNamespaceViolation => matches!(
            kind,
            DependencyKind::Namespace
                | DependencyKind::SourceContract
                | DependencyKind::OccurrenceInput
        ),
        ReasonCode::InputEvidenceInvalid => true,
        _ => return None,
    };
    Some(allowed)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DependencyFailure {
    MissingDeclaration,
    MissingEvidence,
    ContractMismatch,
    VersionMismatch,
    ApplicabilityMismatch,
    Unavailable { reason: ReasonCode },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum ReadinessError {
    #[error("invalid readiness scope: {check}")]
    InvalidScope { check: &'static str },
    #[error("invalid readiness dependency set: {check} ({kind:?})")]
    InvalidDependencySet {
        check: &'static str,
        kind: DependencyKind,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReadinessAssessment {
    scope: ReadinessScope,
    catalog_sha256: Sha256Digest,
    enabled_producers: Vec<ProducerId>,
    requirements: Vec<DependencyRequirement>,
    status: ReadinessStatus,
    stage: ReadinessStage,
    affected_unit_ids: Vec<UnitId>,
    affected_producer_ids: Vec<ProducerId>,
    failures: Vec<(DependencyKind, DependencyFailure)>,
    observations: Vec<DependencyObservation>,
}

impl ReadinessAssessment {
    pub(crate) fn evaluate(
        catalog: &MachineCatalog,
        scope: &ReadinessScope,
        enabled_producers: &[ProducerId],
        stage: ReadinessStage,
        requirements: &[DependencyRequirement],
        observations: &[DependencyObservation],
    ) -> Result<Self, ReadinessError> {
        let (units, producers) = validate_scope(catalog, scope, enabled_producers)?;
        let required = required_kinds(scope);
        let mut declarations = BTreeMap::new();
        for requirement in requirements {
            if !required.contains(&requirement.kind) {
                return Err(ReadinessError::InvalidDependencySet {
                    check: "unexpected_declaration",
                    kind: requirement.kind,
                });
            }
            if declarations.insert(requirement.kind, requirement).is_some() {
                return Err(ReadinessError::InvalidDependencySet {
                    check: "duplicate_declaration",
                    kind: requirement.kind,
                });
            }
            if matches!(
                &requirement.applicability,
                DependencyApplicability::NotRequired { .. }
            ) && requirement.expected_authority != ReadinessEvidenceKind::AuthorityArtifact
            {
                return Err(ReadinessError::InvalidDependencySet {
                    check: "not_required_requires_authority_artifact",
                    kind: requirement.kind,
                });
            }
        }
        let mut evidence = BTreeMap::new();
        for observation in observations {
            observation.validate()?;
            let kind = observation.kind();
            if !declarations.contains_key(&kind) {
                return Err(ReadinessError::InvalidDependencySet {
                    check: "undeclared_observation",
                    kind,
                });
            }
            if evidence.insert(kind, observation).is_some() {
                return Err(ReadinessError::InvalidDependencySet {
                    check: "duplicate_observation",
                    kind,
                });
            }
        }
        let failures: Vec<_> = required
            .into_iter()
            .filter_map(|kind| {
                let failure = match declarations.get(&kind) {
                    None => Some(DependencyFailure::MissingDeclaration),
                    Some(requirement) => match evidence.get(&kind) {
                        None => Some(DependencyFailure::MissingEvidence),
                        Some(observation) => observation.failure_for(requirement),
                    },
                };
                failure.map(|failure| (kind, failure))
            })
            .collect();
        let status = if failures.is_empty() {
            ReadinessStatus::Ready
        } else {
            match scope {
                ReadinessScope::Core => ReadinessStatus::CoreUnready,
                ReadinessScope::Occurrence { .. }
                    if failures.iter().all(|(kind, failure)| {
                        *kind == DependencyKind::OccurrenceInput
                            && *failure != DependencyFailure::MissingDeclaration
                    }) =>
                {
                    ReadinessStatus::BlockedOnInput
                }
                _ => ReadinessStatus::ProducerUnready,
            }
        };
        let (affected_unit_ids, affected_producer_ids) = if status == ReadinessStatus::Ready {
            (Vec::new(), Vec::new())
        } else {
            (units, producers)
        };
        Ok(Self {
            scope: scope.clone(),
            catalog_sha256: catalog.catalog_sha256().clone(),
            enabled_producers: enabled_producers
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            requirements: declarations.into_values().cloned().collect(),
            status,
            stage,
            affected_unit_ids,
            affected_producer_ids,
            failures,
            observations: evidence.into_values().cloned().collect(),
        })
    }

    pub(crate) fn status(&self) -> ReadinessStatus {
        self.status
    }

    pub(crate) fn scope(&self) -> &ReadinessScope {
        &self.scope
    }

    pub(crate) fn catalog_sha256(&self) -> &Sha256Digest {
        &self.catalog_sha256
    }

    pub(crate) fn enabled_producers(&self) -> &[ProducerId] {
        &self.enabled_producers
    }

    pub(crate) fn requirements(&self) -> &[DependencyRequirement] {
        &self.requirements
    }

    pub(crate) fn stage(&self) -> ReadinessStage {
        self.stage
    }

    pub(crate) fn reason(&self) -> ReasonCode {
        match self.status {
            ReadinessStatus::Ready => ReasonCode::ActivationReady,
            ReadinessStatus::CoreUnready => ReasonCode::ActivationCoreUnready,
            ReadinessStatus::ProducerUnready => ReasonCode::ActivationProducerUnready,
            ReadinessStatus::BlockedOnInput => ReasonCode::InputSourceUnavailable,
        }
    }

    pub(crate) fn liveness(&self) -> bool {
        !(self.status == ReadinessStatus::CoreUnready && self.stage == ReadinessStage::Startup)
    }

    pub(crate) fn deployment_ready(&self) -> bool {
        matches!(
            self.status,
            ReadinessStatus::Ready | ReadinessStatus::BlockedOnInput
        )
    }

    pub(crate) fn exit_disposition(&self) -> ReadinessExitDisposition {
        match (self.status, self.stage) {
            (ReadinessStatus::Ready, _) => ReadinessExitDisposition::Continue,
            (ReadinessStatus::CoreUnready, ReadinessStage::Startup) => {
                ReadinessExitDisposition::StartupNonzero
            }
            (ReadinessStatus::CoreUnready, ReadinessStage::Running) => {
                ReadinessExitDisposition::StopNewAndRecoverIsolateThenNonzero
            }
            (ReadinessStatus::ProducerUnready, _) => {
                ReadinessExitDisposition::IsolateAffectedContinueOthers
            }
            (ReadinessStatus::BlockedOnInput, _) => {
                ReadinessExitDisposition::ContinueWithoutOccurrenceWork
            }
        }
    }

    pub(crate) fn affected_unit_ids(&self) -> &[UnitId] {
        &self.affected_unit_ids
    }
    pub(crate) fn affected_producer_ids(&self) -> &[ProducerId] {
        &self.affected_producer_ids
    }
    pub(crate) fn failures(&self) -> &[(DependencyKind, DependencyFailure)] {
        &self.failures
    }
    pub(crate) fn observations(&self) -> &[DependencyObservation] {
        &self.observations
    }
}

fn required_kinds(scope: &ReadinessScope) -> BTreeSet<DependencyKind> {
    match scope {
        ReadinessScope::Core => CORE_DEPENDENCIES.iter().copied().collect(),
        ReadinessScope::Producer { .. } => PRODUCER_DEPENDENCIES.iter().copied().collect(),
        ReadinessScope::Occurrence { .. } => PRODUCER_DEPENDENCIES
            .iter()
            .copied()
            .chain([DependencyKind::OccurrenceInput])
            .collect(),
    }
}

fn validate_scope(
    catalog: &MachineCatalog,
    scope: &ReadinessScope,
    enabled_producers: &[ProducerId],
) -> Result<(Vec<UnitId>, Vec<ProducerId>), ReadinessError> {
    let mut units = BTreeSet::new();
    let mut producers = BTreeSet::new();
    for producer_id in enabled_producers {
        let registration = catalog
            .producer(producer_id)
            .ok_or(ReadinessError::InvalidScope {
                check: "unknown_enabled_producer",
            })?;
        if !producers.insert(producer_id.clone()) {
            return Err(ReadinessError::InvalidScope {
                check: "duplicate_enabled_producer",
            });
        }
        units.insert(registration.unit_id().clone());
    }
    match scope {
        ReadinessScope::Core => Ok((units.into_iter().collect(), producers.into_iter().collect())),
        ReadinessScope::Producer {
            unit_id,
            producer_id,
        }
        | ReadinessScope::Occurrence {
            unit_id,
            producer_id,
            ..
        } => {
            let registration =
                catalog
                    .producer(producer_id)
                    .ok_or(ReadinessError::InvalidScope {
                        check: "unknown_scope_producer",
                    })?;
            if registration.unit_id() != unit_id {
                return Err(ReadinessError::InvalidScope {
                    check: "producer_unit_mismatch",
                });
            }
            if !producers.contains(producer_id) {
                return Err(ReadinessError::InvalidScope {
                    check: "scope_producer_not_enabled",
                });
            }
            Ok((vec![unit_id.clone()], vec![producer_id.clone()]))
        }
    }
}
