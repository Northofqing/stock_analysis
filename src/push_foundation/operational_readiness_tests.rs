use crate::monitor::push_job::{
    derive_occurrence_id, BusinessDate, MachineCatalog, OccurrenceFamily,
    OccurrenceIdentityMaterial, OccurrenceKey, ProducerId, ReasonCode, Sha256Digest,
    SourceContractId, SourceContractVersion, UnitId,
};

use super::operational_readiness::{
    DependencyFailure, DependencyKind, DependencyObservation, DependencyRequirement,
    ReadinessAssessment, ReadinessError, ReadinessExitDisposition, ReadinessScope, ReadinessStage,
    ReadinessStatus,
};

fn producer(value: &str) -> ProducerId {
    ProducerId::try_new(value.to_owned()).expect("TEST_CODE producer ID")
}

fn unit(value: &str) -> UnitId {
    UnitId::try_new(value.to_owned()).expect("TEST_CODE Unit ID")
}

fn requirements(kinds: &[DependencyKind]) -> Vec<DependencyRequirement> {
    kinds
        .iter()
        .map(|kind| DependencyRequirement {
            kind: *kind,
            contract_id: SourceContractId::try_new(format!("TEST_CODE-{kind:?}"))
                .expect("TEST_CODE contract ID"),
            version: SourceContractVersion::try_new("v1".to_owned())
                .expect("TEST_CODE contract version"),
        })
        .collect()
}

fn core_requirements() -> Vec<DependencyRequirement> {
    requirements(&[
        DependencyKind::Namespace,
        DependencyKind::Durable,
        DependencyKind::Audit,
        DependencyKind::TypedAuthority,
        DependencyKind::Schema,
        DependencyKind::Manifest,
    ])
}

fn producer_requirements() -> Vec<DependencyRequirement> {
    requirements(&[
        DependencyKind::ProducerBinding,
        DependencyKind::SourceContract,
        DependencyKind::ScheduleOrTrigger,
        DependencyKind::Presentation,
        DependencyKind::DurablePolicy,
        DependencyKind::ReceiptStrength,
        DependencyKind::FeatureGate,
        DependencyKind::CompletionPolicy,
    ])
}

fn available(requirements: &[DependencyRequirement]) -> Vec<DependencyObservation> {
    requirements
        .iter()
        .map(|required| DependencyObservation::Available {
            kind: required.kind,
            contract_id: required.contract_id.clone(),
            version: required.version.clone(),
            evidence_sha256: Sha256Digest::parse("TEST_CODE evidence", &"a".repeat(64))
                .expect("TEST_CODE digest"),
        })
        .collect()
}

fn p01_scope() -> ReadinessScope {
    ReadinessScope::Producer {
        unit_id: unit("MU-p01"),
        producer_id: producer("p01-scheduled"),
    }
}

fn occurrence_scope() -> ReadinessScope {
    ReadinessScope::Occurrence {
        unit_id: unit("MU-p01"),
        producer_id: producer("p01-scheduled"),
        occurrence_id: derive_occurrence_id(&OccurrenceIdentityMaterial::new(
            BusinessDate::parse("2026-09-07").expect("TEST_CODE date"),
            OccurrenceFamily::try_new("p01:{business_date}".to_owned()).expect("TEST_CODE family"),
            OccurrenceKey::try_new("p01:2026-09-07".to_owned()).expect("TEST_CODE occurrence key"),
        )),
    }
}

#[test]
fn w15_missing_evidence_is_classified_by_scope_and_preserves_independent_producers() {
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let enabled = [producer("p01-scheduled"), producer("news-announcement")];
    let core = core_requirements();
    let core_observations = available(&core);
    let ready = ReadinessAssessment::evaluate(
        &catalog,
        &ReadinessScope::Core,
        &enabled,
        ReadinessStage::Startup,
        &core,
        &core_observations,
    )
    .expect("TEST_CODE complete core");
    assert_eq!(ready.status(), ReadinessStatus::Ready);
    assert_eq!(ready.reason(), ReasonCode::ActivationReady);
    assert!(ready.liveness());
    assert!(ready.deployment_ready());
    assert_eq!(ready.exit_disposition(), ReadinessExitDisposition::Continue);
    assert!(ready.affected_unit_ids().is_empty());
    assert!(ready.affected_producer_ids().is_empty());
    assert!(ready.failures().is_empty());

    let core_missing = &core_observations[..5];
    for (stage, live, exit) in [
        (
            ReadinessStage::Startup,
            false,
            ReadinessExitDisposition::StartupNonzero,
        ),
        (
            ReadinessStage::Running,
            true,
            ReadinessExitDisposition::StopNewAndRecoverIsolateThenNonzero,
        ),
    ] {
        let failure = ReadinessAssessment::evaluate(
            &catalog,
            &ReadinessScope::Core,
            &enabled,
            stage,
            &core,
            core_missing,
        )
        .expect("TEST_CODE missing manifest classification");
        assert_eq!(failure.status(), ReadinessStatus::CoreUnready);
        assert_eq!(failure.reason(), ReasonCode::ActivationCoreUnready);
        assert_eq!(failure.liveness(), live);
        assert!(!failure.deployment_ready());
        assert_eq!(failure.exit_disposition(), exit);
        assert_eq!(
            failure.affected_unit_ids(),
            &[unit("MU-announcement"), unit("MU-p01")]
        );
        assert_eq!(
            failure.affected_producer_ids(),
            &[producer("news-announcement"), producer("p01-scheduled")]
        );
        assert_eq!(
            failure.failures(),
            &[(DependencyKind::Manifest, DependencyFailure::MissingEvidence)]
        );
    }

    let producer_contract = producer_requirements();
    let producer_observations = available(&producer_contract);
    let source_missing: Vec<_> = producer_observations
        .iter()
        .filter(|item| item.kind() != DependencyKind::SourceContract)
        .cloned()
        .collect();
    let failure = ReadinessAssessment::evaluate(
        &catalog,
        &p01_scope(),
        &enabled,
        ReadinessStage::Running,
        &producer_contract,
        &source_missing,
    )
    .expect("TEST_CODE missing producer source contract");
    assert_eq!(failure.status(), ReadinessStatus::ProducerUnready);
    assert_eq!(failure.reason(), ReasonCode::ActivationProducerUnready);
    assert!(failure.liveness());
    assert!(!failure.deployment_ready());
    assert_eq!(
        failure.exit_disposition(),
        ReadinessExitDisposition::IsolateAffectedContinueOthers
    );
    assert_eq!(failure.affected_unit_ids(), &[unit("MU-p01")]);
    assert_eq!(
        failure.affected_producer_ids(),
        &[producer("p01-scheduled")]
    );

    let independent = ReadinessScope::Producer {
        unit_id: unit("MU-announcement"),
        producer_id: producer("news-announcement"),
    };
    assert_eq!(
        ReadinessAssessment::evaluate(
            &catalog,
            &independent,
            &enabled,
            ReadinessStage::Running,
            &producer_contract,
            &producer_observations
        )
        .expect("TEST_CODE independent producer remains ready")
        .status(),
        ReadinessStatus::Ready
    );

    let mut occurrence_contract = producer_contract.clone();
    occurrence_contract.extend(requirements(&[DependencyKind::OccurrenceInput]));
    let blocked = ReadinessAssessment::evaluate(
        &catalog,
        &occurrence_scope(),
        &enabled,
        ReadinessStage::Running,
        &occurrence_contract,
        &producer_observations,
    )
    .expect("TEST_CODE only this occurrence input is missing");
    assert_eq!(blocked.status(), ReadinessStatus::BlockedOnInput);
    assert_eq!(blocked.reason(), ReasonCode::InputSourceUnavailable);
    assert!(blocked.liveness());
    assert!(blocked.deployment_ready());
    assert_eq!(
        blocked.exit_disposition(),
        ReadinessExitDisposition::ContinueWithoutOccurrenceWork
    );
    assert_eq!(
        blocked.affected_producer_ids(),
        &[producer("p01-scheduled")]
    );

    let missing_contract = ReadinessAssessment::evaluate(
        &catalog,
        &occurrence_scope(),
        &enabled,
        ReadinessStage::Running,
        &occurrence_contract,
        &source_missing,
    )
    .expect("TEST_CODE occurrence cannot hide its missing producer contract");
    assert_eq!(missing_contract.status(), ReadinessStatus::ProducerUnready);
    assert!(!missing_contract.deployment_ready());
}

#[test]
fn w15_missing_occurrence_input_declaration_is_a_contract_gap_not_a_transient_blocker() {
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let declared = producer_requirements();
    let assessment = ReadinessAssessment::evaluate(
        &catalog,
        &occurrence_scope(),
        &[producer("p01-scheduled")],
        ReadinessStage::Running,
        &declared,
        &available(&declared),
    )
    .expect("TEST_CODE missing input declaration is observable");
    assert_eq!(assessment.status(), ReadinessStatus::ProducerUnready);
    assert!(!assessment.deployment_ready());
    assert_eq!(
        assessment.failures(),
        &[(
            DependencyKind::OccurrenceInput,
            DependencyFailure::MissingDeclaration
        )]
    );
}

#[test]
fn w15_dependency_source_version_and_declaration_mismatches_never_report_ready() {
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let enabled = [producer("p01-scheduled")];
    let declared = producer_requirements();
    for (replacement_id, replacement_version, expected_failure) in [
        (
            declared[1].contract_id.clone(),
            SourceContractVersion::try_new("v2".into()).expect("TEST_CODE version"),
            DependencyFailure::VersionMismatch,
        ),
        (
            SourceContractId::try_new("TEST_CODE-foreign-source".into()).expect("TEST_CODE source"),
            declared[1].version.clone(),
            DependencyFailure::ContractMismatch,
        ),
    ] {
        let mut observed = available(&declared);
        observed[1] = DependencyObservation::Available {
            kind: DependencyKind::SourceContract,
            contract_id: replacement_id,
            version: replacement_version,
            evidence_sha256: Sha256Digest::parse("TEST_CODE evidence", &"a".repeat(64))
                .expect("TEST_CODE digest"),
        };
        let assessment = ReadinessAssessment::evaluate(
            &catalog,
            &p01_scope(),
            &enabled,
            ReadinessStage::Running,
            &declared,
            &observed,
        )
        .expect("TEST_CODE invalid dependency assessment");
        assert_eq!(assessment.status(), ReadinessStatus::ProducerUnready);
        assert_eq!(
            assessment.failures(),
            &[(DependencyKind::SourceContract, expected_failure)]
        );
    }
    let empty = ReadinessAssessment::evaluate(
        &catalog,
        &ReadinessScope::Core,
        &enabled,
        ReadinessStage::Startup,
        &[],
        &[],
    )
    .expect("TEST_CODE no declarations is not vacuous readiness");
    assert_eq!(empty.status(), ReadinessStatus::CoreUnready);
    assert_eq!(empty.failures().len(), 6);

    let mut missing = declared.clone();
    missing.remove(1);
    let assessment = ReadinessAssessment::evaluate(
        &catalog,
        &p01_scope(),
        &enabled,
        ReadinessStage::Running,
        &missing,
        &available(&missing),
    )
    .expect("TEST_CODE missing source declaration");
    assert_eq!(assessment.status(), ReadinessStatus::ProducerUnready);
    assert_eq!(
        assessment.failures(),
        &[(
            DependencyKind::SourceContract,
            DependencyFailure::MissingDeclaration
        )]
    );
}

#[test]
fn w15_invalid_scope_and_ambiguous_dependency_sets_are_rejected_before_assessment() {
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let p01 = producer("p01-scheduled");
    let declared = producer_requirements();
    let observed = available(&declared);
    for (scope, enabled, check) in [
        (
            p01_scope(),
            vec![p01.clone(), p01.clone()],
            "duplicate_enabled_producer",
        ),
        (
            p01_scope(),
            vec![producer("TEST_CODE-unknown")],
            "unknown_enabled_producer",
        ),
        (
            p01_scope(),
            vec![producer("news-announcement")],
            "scope_producer_not_enabled",
        ),
        (
            ReadinessScope::Producer {
                unit_id: unit("MU-announcement"),
                producer_id: p01.clone(),
            },
            vec![p01.clone()],
            "producer_unit_mismatch",
        ),
        (
            ReadinessScope::Producer {
                unit_id: unit("MU-p01"),
                producer_id: producer("TEST_CODE-unknown"),
            },
            vec![p01.clone()],
            "unknown_scope_producer",
        ),
    ] {
        assert_eq!(
            ReadinessAssessment::evaluate(
                &catalog,
                &scope,
                &enabled,
                ReadinessStage::Running,
                &declared,
                &observed
            ),
            Err(ReadinessError::InvalidScope { check })
        );
    }
    let mut duplicated = declared.clone();
    duplicated.push(declared[0].clone());
    assert_eq!(
        ReadinessAssessment::evaluate(
            &catalog,
            &p01_scope(),
            &[p01.clone()],
            ReadinessStage::Running,
            &duplicated,
            &observed
        ),
        Err(ReadinessError::InvalidDependencySet {
            check: "duplicate_declaration",
            kind: DependencyKind::ProducerBinding,
        })
    );
    let mut duplicated = observed.clone();
    duplicated.push(observed[0].clone());
    assert_eq!(
        ReadinessAssessment::evaluate(
            &catalog,
            &p01_scope(),
            &[p01.clone()],
            ReadinessStage::Running,
            &declared,
            &duplicated
        ),
        Err(ReadinessError::InvalidDependencySet {
            check: "duplicate_observation",
            kind: DependencyKind::ProducerBinding,
        })
    );
    assert_eq!(
        ReadinessAssessment::evaluate(
            &catalog,
            &ReadinessScope::Core,
            &[p01.clone()],
            ReadinessStage::Running,
            &declared,
            &observed
        ),
        Err(ReadinessError::InvalidDependencySet {
            check: "unexpected_declaration",
            kind: DependencyKind::ProducerBinding,
        })
    );
    assert_eq!(
        ReadinessAssessment::evaluate(
            &catalog,
            &p01_scope(),
            &[p01],
            ReadinessStage::Running,
            &[],
            &observed
        ),
        Err(ReadinessError::InvalidDependencySet {
            check: "undeclared_observation",
            kind: DependencyKind::ProducerBinding,
        })
    );
}

#[test]
fn w15_assessment_is_replayable_under_dependency_and_producer_reordering() {
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let mut enabled = vec![producer("p01-scheduled"), producer("news-announcement")];
    let mut declared = core_requirements();
    let mut observed = available(&declared);
    observed.pop();
    let original = ReadinessAssessment::evaluate(
        &catalog,
        &ReadinessScope::Core,
        &enabled,
        ReadinessStage::Running,
        &declared,
        &observed,
    )
    .expect("TEST_CODE original assessment");
    enabled.reverse();
    declared.reverse();
    observed.reverse();
    let reordered = ReadinessAssessment::evaluate(
        &catalog,
        &ReadinessScope::Core,
        &enabled,
        ReadinessStage::Running,
        &declared,
        &observed,
    )
    .expect("TEST_CODE same logical observations");
    assert_eq!(original, reordered);
}

#[test]
fn w15_explicit_unavailable_evidence_retains_failure_reason_without_becoming_no_data() {
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let mut declared = producer_requirements();
    let input = requirements(&[DependencyKind::OccurrenceInput]).remove(0);
    declared.push(input.clone());
    let mut observed = available(&declared);
    observed.pop();
    let failed = DependencyObservation::Unavailable {
        kind: input.kind,
        contract_id: input.contract_id.clone(),
        version: input.version.clone(),
        evidence_sha256: Sha256Digest::parse("TEST_CODE failed evidence", &"b".repeat(64))
            .expect("TEST_CODE digest"),
        reason: ReasonCode::InputSourceUnready,
    };
    observed.push(failed.clone());
    let blocked = ReadinessAssessment::evaluate(
        &catalog,
        &occurrence_scope(),
        &[producer("p01-scheduled")],
        ReadinessStage::Running,
        &declared,
        &observed,
    )
    .expect("TEST_CODE explicit input failure");
    assert_eq!(blocked.status(), ReadinessStatus::BlockedOnInput);
    assert!(blocked.deployment_ready());
    assert_eq!(
        blocked.failures(),
        &[(
            DependencyKind::OccurrenceInput,
            DependencyFailure::Unavailable {
                reason: ReasonCode::InputSourceUnready
            }
        )]
    );
    assert!(blocked.observations().contains(&failed));

    for reason in [ReasonCode::ActivationReady, ReasonCode::IntentNoData] {
        let mut invalid = observed.clone();
        invalid.pop();
        invalid.push(DependencyObservation::Unavailable {
            kind: input.kind,
            contract_id: input.contract_id.clone(),
            version: input.version.clone(),
            evidence_sha256: Sha256Digest::parse("TEST_CODE invalid reason", &"c".repeat(64))
                .expect("TEST_CODE digest"),
            reason,
        });
        assert_eq!(
            ReadinessAssessment::evaluate(
                &catalog,
                &occurrence_scope(),
                &[producer("p01-scheduled")],
                ReadinessStage::Running,
                &declared,
                &invalid,
            ),
            Err(ReadinessError::InvalidDependencySet {
                check: "invalid_unavailable_reason",
                kind: DependencyKind::OccurrenceInput
            })
        );
    }
}
