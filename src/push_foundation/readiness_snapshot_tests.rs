use crate::monitor::push_job::{
    BusinessDate, GitSha40, MachineCatalog, Namespace, ProducerId, ProtectedRef, RunId,
    Sha256Digest, SourceContractId, SourceContractVersion, UtcMicros,
};

use super::operational_readiness::{
    DependencyApplicability, DependencyKind, DependencyObservation, DependencyRequirement,
    ReadinessAssessment, ReadinessScope, ReadinessStage, ReadinessStatus,
};
use super::readiness_snapshot::{
    CandidateReadinessSnapshot, ReadinessEvidenceKind, ReadinessEvidenceRef,
    ReadinessRecoveryEventId, ReadinessSnapshotContext, ReadinessSnapshotError,
};

fn digest(hex: char) -> Sha256Digest {
    Sha256Digest::parse("TEST_CODE digest", &hex.to_string().repeat(64)).expect("TEST_CODE digest")
}

fn context() -> ReadinessSnapshotContext {
    ReadinessSnapshotContext {
        namespace: Namespace::test(
            RunId::try_new("TEST_CODE-w15-snapshot".to_owned()).expect("TEST_CODE run"),
        ),
        business_date: BusinessDate::parse("2026-09-07").expect("TEST_CODE date"),
        build_commit: GitSha40::parse(&"1".repeat(40)).expect("TEST_CODE build"),
        activation_generation: 7,
        manifest_sha256: digest('d'),
        captured_at: UtcMicros::try_new(100).expect("TEST_CODE time"),
    }
}

fn core_facts() -> (
    Vec<DependencyRequirement>,
    Vec<DependencyObservation>,
    Vec<ReadinessEvidenceRef>,
) {
    let mut requirements = Vec::new();
    let mut observations = Vec::new();
    let mut evidence = Vec::new();
    for kind in [
        DependencyKind::Namespace,
        DependencyKind::Durable,
        DependencyKind::Audit,
        DependencyKind::TypedAuthority,
        DependencyKind::Schema,
        DependencyKind::Manifest,
    ] {
        let contract_id =
            SourceContractId::try_new(format!("TEST_CODE-{kind:?}")).expect("TEST_CODE source");
        let version = SourceContractVersion::try_new("v1".to_owned()).expect("TEST_CODE version");
        requirements.push(DependencyRequirement {
            kind,
            contract_id: contract_id.clone(),
            version: version.clone(),
            expected_authority: ReadinessEvidenceKind::AuthorityArtifact,
            applicability: DependencyApplicability::Required,
        });
        observations.push(DependencyObservation::Available {
            kind,
            contract_id: contract_id.clone(),
            version: version.clone(),
            evidence_sha256: digest('a'),
        });
        evidence.push(ReadinessEvidenceRef::new(
            kind,
            ReadinessEvidenceKind::AuthorityArtifact,
            ProtectedRef::try_new(format!("vault://TEST_CODE-SECRET/{kind:?}"))
                .expect("TEST_CODE protected ref"),
            digest('a'),
            contract_id,
            version,
        ));
    }
    (requirements, observations, evidence)
}

#[test]
fn w15_snapshot_binds_context_dependency_evidence_and_recovery_without_debug_leakage() {
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let enabled = [ProducerId::try_new("p01-scheduled".to_owned()).expect("TEST_CODE producer")];
    let (mut requirements, mut observations, mut evidence) = core_facts();
    let assessed = ReadinessAssessment::evaluate(
        &catalog,
        &ReadinessScope::Core,
        &enabled,
        ReadinessStage::Running,
        &requirements,
        &observations,
    )
    .expect("TEST_CODE core assessment");
    let event = ReadinessRecoveryEventId::from_digest(digest('b'));
    let snapshot = CandidateReadinessSnapshot::try_new(
        context(),
        assessed.clone(),
        event.clone(),
        evidence.clone(),
    )
    .expect("TEST_CODE candidate snapshot");
    assert_eq!(snapshot.assessment().status(), ReadinessStatus::Ready);
    assert_eq!(snapshot.context(), &context());
    assert_eq!(snapshot.recovery_event_id(), &event);
    assert!(!format!("{snapshot:?}").contains("TEST_CODE-SECRET"));
    assert!(!format!("{evidence:?}").contains("TEST_CODE-SECRET"));
    let bytes = snapshot.canonical_bytes();
    let fields: serde_json::Value = serde_json::from_slice(
        bytes
            .strip_prefix(b"OperationalReadinessSnapshot/v2\0")
            .expect("TEST_CODE canonical domain"),
    )
    .expect("TEST_CODE canonical JSON");
    assert!(fields.get("snapshot_id").is_none());
    assert_eq!(fields["business_date"], "2026-09-07");
    assert_eq!(fields["activation_generation"], 7);
    assert_eq!(
        fields["build_commit"],
        "1111111111111111111111111111111111111111"
    );
    assert_eq!(fields["catalog_sha256"], catalog.catalog_sha256().as_str());
    assert_eq!(fields["status"], "Ready");
    assert_eq!(fields["reason"], "activation.ready");
    assert_eq!(
        fields["dependency_refs"]
            .as_array()
            .expect("TEST_CODE deps")
            .len(),
        6
    );
    assert_eq!(
        fields["evidence_refs"]
            .as_array()
            .expect("TEST_CODE refs")
            .len(),
        6
    );
    assert_eq!(fields["liveness"], true);
    assert_eq!(fields["deployment_ready"], true);
    assert_eq!(fields["exit_disposition"], "Continue");

    requirements.reverse();
    observations.reverse();
    evidence.reverse();
    let reordered = ReadinessAssessment::evaluate(
        &catalog,
        &ReadinessScope::Core,
        &enabled,
        ReadinessStage::Running,
        &requirements,
        &observations,
    )
    .expect("TEST_CODE reordered");
    assert_eq!(
        snapshot,
        CandidateReadinessSnapshot::try_new(context(), reordered, event.clone(), evidence.clone())
            .expect("TEST_CODE replay")
    );

    let mut later = context();
    later.activation_generation = 8;
    let changed =
        CandidateReadinessSnapshot::try_new(later, assessed.clone(), event, evidence.clone())
            .expect("TEST_CODE changed generation");
    assert_ne!(snapshot.snapshot_id(), changed.snapshot_id());
    let changed = CandidateReadinessSnapshot::try_new(
        context(),
        assessed,
        ReadinessRecoveryEventId::from_digest(digest('c')),
        evidence,
    )
    .expect("TEST_CODE changed recovery");
    assert_ne!(snapshot.snapshot_id(), changed.snapshot_id());
}

#[test]
fn w15_snapshot_rejects_missing_duplicate_extra_and_cross_bound_evidence_refs() {
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let (requirements, observations, evidence) = core_facts();
    let assessed = ReadinessAssessment::evaluate(
        &catalog,
        &ReadinessScope::Core,
        &[],
        ReadinessStage::Running,
        &requirements,
        &observations,
    )
    .expect("TEST_CODE assessed");
    let build = |refs| {
        CandidateReadinessSnapshot::try_new(
            context(),
            assessed.clone(),
            ReadinessRecoveryEventId::from_digest(digest('b')),
            refs,
        )
    };
    assert_eq!(
        build(Vec::new()),
        Err(ReadinessSnapshotError::InvalidEvidenceSet {
            check: "missing_evidence_ref",
            kind: DependencyKind::Namespace,
        })
    );
    let mut duplicated = evidence.clone();
    duplicated.push(evidence[0].clone());
    assert_eq!(
        build(duplicated),
        Err(ReadinessSnapshotError::InvalidEvidenceSet {
            check: "duplicate_evidence",
            kind: DependencyKind::Namespace,
        })
    );
    let mut extra = evidence.clone();
    extra.push(ReadinessEvidenceRef::new(
        DependencyKind::SourceContract,
        ReadinessEvidenceKind::DataAcquisitionAudit,
        ProtectedRef::try_new("vault://TEST_CODE-SECRET/extra".to_owned()).expect("TEST_CODE uri"),
        digest('a'),
        requirements[0].contract_id.clone(),
        requirements[0].version.clone(),
    ));
    assert_eq!(
        build(extra),
        Err(ReadinessSnapshotError::InvalidEvidenceSet {
            check: "unobserved_evidence",
            kind: DependencyKind::SourceContract,
        })
    );
    for (sha, source, version) in [
        (
            digest('f'),
            requirements[0].contract_id.clone(),
            requirements[0].version.clone(),
        ),
        (
            digest('a'),
            SourceContractId::try_new("TEST_CODE-foreign".to_owned()).expect("TEST_CODE source"),
            requirements[0].version.clone(),
        ),
        (
            digest('a'),
            requirements[0].contract_id.clone(),
            SourceContractVersion::try_new("v2".to_owned()).expect("TEST_CODE version"),
        ),
    ] {
        let mut mismatched = evidence.clone();
        mismatched[0] = ReadinessEvidenceRef::new(
            DependencyKind::Namespace,
            ReadinessEvidenceKind::AuthorityArtifact,
            ProtectedRef::try_new("vault://TEST_CODE-SECRET/wrong".to_owned())
                .expect("TEST_CODE uri"),
            sha,
            source,
            version,
        );
        let error = build(mismatched).expect_err("TEST_CODE mismatched evidence");
        assert_eq!(
            error,
            ReadinessSnapshotError::InvalidEvidenceSet {
                check: "observation_evidence_mismatch",
                kind: DependencyKind::Namespace,
            }
        );
        assert!(!format!("{error:?} {error}").contains("TEST_CODE-SECRET"));
    }
}

#[test]
fn w15_snapshot_rejects_wrong_authority_for_available_unavailable_and_not_required() {
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let build = |requirements: Vec<DependencyRequirement>,
                 observations: Vec<DependencyObservation>,
                 evidence: Vec<ReadinessEvidenceRef>| {
        let assessment = ReadinessAssessment::evaluate(
            &catalog,
            &ReadinessScope::Core,
            &[],
            ReadinessStage::Running,
            &requirements,
            &observations,
        )
        .expect("TEST_CODE assessed evidence kind mismatch");
        CandidateReadinessSnapshot::try_new(
            context(),
            assessment,
            ReadinessRecoveryEventId::from_digest(digest('b')),
            evidence,
        )
    };

    let (requirements, observations, mut evidence) = core_facts();
    evidence[0] = ReadinessEvidenceRef::new(
        DependencyKind::Namespace,
        ReadinessEvidenceKind::DataAcquisitionAudit,
        ProtectedRef::try_new("vault://TEST_CODE-SECRET/wrong-available".to_owned())
            .expect("TEST_CODE URI"),
        digest('a'),
        requirements[0].contract_id.clone(),
        requirements[0].version.clone(),
    );
    assert_eq!(
        build(requirements, observations, evidence),
        Err(ReadinessSnapshotError::InvalidEvidenceSet {
            check: "observation_evidence_mismatch",
            kind: DependencyKind::Namespace,
        })
    );

    let (requirements, mut observations, mut evidence) = core_facts();
    observations[5] = DependencyObservation::Unavailable {
        kind: DependencyKind::Manifest,
        contract_id: requirements[5].contract_id.clone(),
        version: requirements[5].version.clone(),
        evidence_sha256: digest('a'),
        reason: crate::monitor::push_job::ReasonCode::ActivationCoreUnready,
    };
    evidence[5] = ReadinessEvidenceRef::new(
        DependencyKind::Manifest,
        ReadinessEvidenceKind::DataAcquisitionAudit,
        ProtectedRef::try_new("vault://TEST_CODE-SECRET/wrong-unavailable".to_owned())
            .expect("TEST_CODE URI"),
        digest('a'),
        requirements[5].contract_id.clone(),
        requirements[5].version.clone(),
    );
    assert_eq!(
        build(requirements, observations, evidence),
        Err(ReadinessSnapshotError::InvalidEvidenceSet {
            check: "observation_evidence_mismatch",
            kind: DependencyKind::Manifest,
        })
    );

    let (mut requirements, mut observations, mut evidence) = core_facts();
    let basis_sha256 = digest('f');
    requirements[4].applicability = DependencyApplicability::NotRequired {
        basis_sha256: basis_sha256.clone(),
    };
    observations[4] = DependencyObservation::NotRequired {
        kind: DependencyKind::Schema,
        contract_id: requirements[4].contract_id.clone(),
        version: requirements[4].version.clone(),
        evidence_sha256: digest('a'),
        basis_sha256,
    };
    evidence[4] = ReadinessEvidenceRef::new(
        DependencyKind::Schema,
        ReadinessEvidenceKind::DataAcquisitionAudit,
        ProtectedRef::try_new("vault://TEST_CODE-SECRET/wrong-not-required".to_owned())
            .expect("TEST_CODE URI"),
        digest('a'),
        requirements[4].contract_id.clone(),
        requirements[4].version.clone(),
    );
    assert_eq!(
        build(requirements, observations, evidence),
        Err(ReadinessSnapshotError::InvalidEvidenceSet {
            check: "observation_evidence_mismatch",
            kind: DependencyKind::Schema,
        })
    );
}

#[test]
fn w15_snapshot_identity_binds_expected_authority_applicability_and_basis() {
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let build = |requirements: Vec<DependencyRequirement>,
                 observations: Vec<DependencyObservation>,
                 evidence: Vec<ReadinessEvidenceRef>| {
        let assessment = ReadinessAssessment::evaluate(
            &catalog,
            &ReadinessScope::Core,
            &[],
            ReadinessStage::Running,
            &requirements,
            &observations,
        )
        .expect("TEST_CODE identity assessment");
        CandidateReadinessSnapshot::try_new(
            context(),
            assessment,
            ReadinessRecoveryEventId::from_digest(digest('b')),
            evidence,
        )
        .expect("TEST_CODE identity candidate")
    };
    let (requirements, observations, evidence) = core_facts();
    let original = build(requirements.clone(), observations.clone(), evidence.clone());

    let mut authority_requirements = requirements.clone();
    authority_requirements[0].expected_authority = ReadinessEvidenceKind::DataAcquisitionAudit;
    let mut authority_evidence = evidence.clone();
    authority_evidence[0] = ReadinessEvidenceRef::new(
        DependencyKind::Namespace,
        ReadinessEvidenceKind::DataAcquisitionAudit,
        ProtectedRef::try_new("vault://TEST_CODE-SECRET/Namespace".to_owned())
            .expect("TEST_CODE URI"),
        digest('a'),
        authority_requirements[0].contract_id.clone(),
        authority_requirements[0].version.clone(),
    );
    let changed_authority = build(
        authority_requirements,
        observations.clone(),
        authority_evidence,
    );
    assert_ne!(original.snapshot_id(), changed_authority.snapshot_id());

    let not_required = |basis_sha256: Sha256Digest| {
        let mut changed_requirements = requirements.clone();
        changed_requirements[0].applicability = DependencyApplicability::NotRequired {
            basis_sha256: basis_sha256.clone(),
        };
        let mut changed_observations = observations.clone();
        changed_observations[0] = DependencyObservation::NotRequired {
            kind: DependencyKind::Namespace,
            contract_id: changed_requirements[0].contract_id.clone(),
            version: changed_requirements[0].version.clone(),
            evidence_sha256: digest('a'),
            basis_sha256,
        };
        build(changed_requirements, changed_observations, evidence.clone())
    };
    let first_basis = not_required(digest('e'));
    let second_basis = not_required(digest('f'));
    assert_ne!(original.snapshot_id(), first_basis.snapshot_id());
    assert_ne!(first_basis.snapshot_id(), second_basis.snapshot_id());
}

#[test]
fn w15_snapshot_preserves_non_ready_gaps_and_actual_mismatched_versions() {
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let (requirements, mut observations, mut evidence) = core_facts();
    observations.remove(0);
    evidence.remove(0);
    let missing = ReadinessAssessment::evaluate(
        &catalog,
        &ReadinessScope::Core,
        &[],
        ReadinessStage::Startup,
        &requirements,
        &observations,
    )
    .expect("TEST_CODE missing observation");
    let snapshot = CandidateReadinessSnapshot::try_new(
        context(),
        missing,
        ReadinessRecoveryEventId::from_digest(digest('b')),
        evidence,
    )
    .expect("TEST_CODE negative snapshot");
    assert_eq!(snapshot.assessment().status(), ReadinessStatus::CoreUnready);
    assert!(!snapshot.assessment().liveness());
    assert!(!snapshot.assessment().deployment_ready());

    let (requirements, mut observations, mut evidence) = core_facts();
    let v2 = SourceContractVersion::try_new("v2".to_owned()).expect("TEST_CODE actual version");
    observations[0] = DependencyObservation::Available {
        kind: DependencyKind::Namespace,
        contract_id: requirements[0].contract_id.clone(),
        version: v2.clone(),
        evidence_sha256: digest('a'),
    };
    evidence[0] = ReadinessEvidenceRef::new(
        DependencyKind::Namespace,
        ReadinessEvidenceKind::AuthorityArtifact,
        ProtectedRef::try_new("vault://TEST_CODE-SECRET/version".to_owned())
            .expect("TEST_CODE uri"),
        digest('a'),
        requirements[0].contract_id.clone(),
        v2,
    );
    let mismatched = ReadinessAssessment::evaluate(
        &catalog,
        &ReadinessScope::Core,
        &[],
        ReadinessStage::Running,
        &requirements,
        &observations,
    )
    .expect("TEST_CODE mismatched version");
    let snapshot = CandidateReadinessSnapshot::try_new(
        context(),
        mismatched,
        ReadinessRecoveryEventId::from_digest(digest('b')),
        evidence,
    )
    .expect("TEST_CODE actual negative evidence is retained");
    assert_eq!(snapshot.assessment().status(), ReadinessStatus::CoreUnready);
    let bytes = snapshot.canonical_bytes();
    let fields: serde_json::Value = serde_json::from_slice(
        bytes
            .strip_prefix(b"OperationalReadinessSnapshot/v2\0")
            .expect("TEST_CODE domain"),
    )
    .expect("TEST_CODE JSON");
    assert_eq!(fields["dependency_refs"][0]["version"], "v1");
    assert_eq!(fields["dependency_refs"][0]["observation"]["version"], "v2");
    assert_eq!(
        fields["dependency_failures"][0]["failure"],
        "VersionMismatch"
    );
}

#[test]
fn w15_snapshot_identity_changes_when_publication_context_or_evaluated_set_changes() {
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let (requirements, observations, evidence) = core_facts();
    let assess = |enabled: &[ProducerId]| {
        ReadinessAssessment::evaluate(
            &catalog,
            &ReadinessScope::Core,
            enabled,
            ReadinessStage::Running,
            &requirements,
            &observations,
        )
        .expect("TEST_CODE assessment")
    };
    let build = |ctx, assessed| {
        CandidateReadinessSnapshot::try_new(
            ctx,
            assessed,
            ReadinessRecoveryEventId::from_digest(digest('b')),
            evidence.clone(),
        )
        .expect("TEST_CODE candidate")
    };
    let original = build(context(), assess(&[]));
    let mut variants = Vec::new();
    let mut changed = context();
    changed.namespace = Namespace::Production;
    variants.push(changed);
    let mut changed = context();
    changed.business_date = BusinessDate::parse("2026-09-08").expect("TEST_CODE other date");
    variants.push(changed);
    let mut changed = context();
    changed.build_commit = GitSha40::parse(&"2".repeat(40)).expect("TEST_CODE other build");
    variants.push(changed);
    let mut changed = context();
    changed.manifest_sha256 = digest('e');
    variants.push(changed);
    let mut changed = context();
    changed.captured_at = UtcMicros::try_new(101).expect("TEST_CODE later observation");
    variants.push(changed);
    for changed in variants {
        assert_ne!(
            original.snapshot_id(),
            build(changed, assess(&[])).snapshot_id()
        );
    }
    let enabled = [ProducerId::try_new("p01-scheduled".to_owned()).expect("TEST_CODE producer")];
    let changed = build(context(), assess(&enabled));
    assert!(original.assessment().affected_producer_ids().is_empty());
    assert!(changed.assessment().affected_producer_ids().is_empty());
    assert_ne!(original.snapshot_id(), changed.snapshot_id());
}
