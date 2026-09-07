use crate::monitor::push_job::{
    raw_digest, BusinessDate, GitSha40, MachineCatalog, Namespace, OccurrenceId, ProducerId,
    ProtectedRef, ReasonCode, RunId, Sha256Digest, SourceContractId, SourceContractVersion, UnitId,
    UtcMicros,
};

use super::operational_readiness::{
    DependencyApplicability, DependencyKind, DependencyObservation, DependencyRequirement,
    ReadinessAssessment, ReadinessScope, ReadinessStage,
};
use super::readiness_recovery::{
    CandidateReadinessRecord, CandidateRecoveryClaim, ReadinessRecoveryError, ReadinessRecoveryKind,
};
use super::readiness_snapshot::{
    ReadinessEvidenceKind, ReadinessEvidenceRef, ReadinessSnapshotContext,
};

pub(super) fn assessed(
    scope: &ReadinessScope,
    blocked: Option<DependencyKind>,
) -> (ReadinessAssessment, Vec<ReadinessEvidenceRef>) {
    use DependencyKind::*;
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let mut roles = match scope {
        ReadinessScope::Core => vec![Namespace, Durable, Audit, TypedAuthority, Schema, Manifest],
        _ => vec![
            ProducerBinding,
            SourceContract,
            ScheduleOrTrigger,
            Presentation,
            DurablePolicy,
            ReceiptStrength,
            FeatureGate,
            CompletionPolicy,
        ],
    };
    if matches!(scope, ReadinessScope::Occurrence { .. }) {
        roles.push(OccurrenceInput);
    }
    let mut requirements = vec![];
    let mut observations = vec![];
    let mut evidence = vec![];
    for kind in roles {
        let contract_id =
            SourceContractId::try_new(format!("TEST_CODE-{kind:?}")).expect("TEST_CODE source");
        let version = SourceContractVersion::try_new("v1".to_owned()).expect("TEST_CODE version");
        let is_blocked = Some(kind) == blocked;
        let hash = raw_digest(if is_blocked {
            b"unavailable"
        } else {
            b"available"
        });
        requirements.push(DependencyRequirement {
            kind,
            contract_id: contract_id.clone(),
            version: version.clone(),
            expected_authority: ReadinessEvidenceKind::AuthorityArtifact,
            applicability: DependencyApplicability::Required,
        });
        observations.push(if is_blocked {
            DependencyObservation::Unavailable {
                kind,
                contract_id: contract_id.clone(),
                version: version.clone(),
                evidence_sha256: hash.clone(),
                reason: if matches!(scope, ReadinessScope::Core) {
                    ReasonCode::ActivationCoreUnready
                } else {
                    ReasonCode::InputSourceUnready
                },
            }
        } else {
            DependencyObservation::Available {
                kind,
                contract_id: contract_id.clone(),
                version: version.clone(),
                evidence_sha256: hash.clone(),
            }
        });
        evidence.push(ReadinessEvidenceRef::new(
            kind,
            ReadinessEvidenceKind::AuthorityArtifact,
            ProtectedRef::try_new(format!("vault://TEST_CODE-SECRET/{kind:?}"))
                .expect("TEST_CODE ref"),
            hash,
            contract_id,
            version,
        ));
    }
    let enabled = [ProducerId::try_new("p01-scheduled".to_owned()).expect("TEST_CODE producer")];
    let assessment = ReadinessAssessment::evaluate(
        &catalog,
        scope,
        &enabled,
        ReadinessStage::Running,
        &requirements,
        &observations,
    )
    .expect("TEST_CODE assessed");
    (assessment, evidence)
}

pub(super) fn context(captured_at: i64) -> ReadinessSnapshotContext {
    ReadinessSnapshotContext {
        namespace: Namespace::test(
            RunId::try_new("TEST_CODE-w15-recovery".to_owned()).expect("TEST_CODE namespace"),
        ),
        business_date: BusinessDate::parse("2026-09-07").expect("TEST_CODE date"),
        build_commit: GitSha40::parse(&"a".repeat(40)).expect("TEST_CODE build"),
        activation_generation: 1,
        manifest_sha256: Sha256Digest::parse("TEST_CODE manifest", &"b".repeat(64))
            .expect("TEST_CODE manifest"),
        captured_at: UtcMicros::try_new(captured_at).expect("TEST_CODE time"),
    }
}

#[test]
fn w15_recovery_requires_explicit_changed_evidence_and_binds_each_restored_scope() {
    let producer_id = ProducerId::try_new("p01-scheduled".to_owned()).expect("TEST_CODE producer");
    let unit_id = UnitId::try_new("MU-p01".to_owned()).expect("TEST_CODE unit");
    for (scope, failed_role, expected_kind) in [
        (
            ReadinessScope::Core,
            DependencyKind::Schema,
            "CoreDependenciesRestored",
        ),
        (
            ReadinessScope::Producer {
                unit_id: unit_id.clone(),
                producer_id: producer_id.clone(),
            },
            DependencyKind::SourceContract,
            "ProducerContractRestored",
        ),
        (
            ReadinessScope::Occurrence {
                unit_id,
                producer_id,
                occurrence_id: OccurrenceId::from_digest(&raw_digest(b"TEST_CODE occurrence")),
            },
            DependencyKind::OccurrenceInput,
            "InputEvidenceRestored",
        ),
    ] {
        let (negative, refs) = assessed(&scope, Some(failed_role));
        let before = CandidateReadinessRecord::try_new(None, context(100), negative, refs, vec![])
            .expect("TEST_CODE pending");
        let (ready, refs) = assessed(&scope, None);
        assert_eq!(
            CandidateReadinessRecord::try_new(
                Some(before.snapshot()),
                context(200),
                ready.clone(),
                refs.clone(),
                vec![]
            ),
            Err(ReadinessRecoveryError::ExplicitRecoveryRequired)
        );
        let role_index = ready
            .observations()
            .iter()
            .position(|observation| observation.kind() == failed_role)
            .expect("TEST_CODE failed role");
        let restored = CandidateReadinessRecord::try_new(
            Some(before.snapshot()),
            context(200),
            ready,
            refs.clone(),
            vec![CandidateRecoveryClaim {
                evidence: refs[role_index].clone(),
                observed_at: UtcMicros::try_new(150).expect("TEST_CODE source time"),
            }],
        )
        .expect("TEST_CODE explicit recovery");
        let body_bytes = restored.event_bytes();
        let body: serde_json::Value = serde_json::from_slice(
            body_bytes
                .strip_prefix(b"OperationalReadinessRecoveryEvent/v1\0")
                .expect("TEST_CODE domain"),
        )
        .expect("TEST_CODE JSON");
        assert_eq!(body["kind"], expected_kind);
        assert_eq!(
            body["dependency_changes"]
                .as_array()
                .expect("TEST_CODE delta")
                .len(),
            1
        );
        assert_eq!(body["dependency_changes"][0]["kind"], failed_role.as_str());
        assert_eq!(
            body["recovery_claims"]
                .as_array()
                .expect("TEST_CODE claims")
                .len(),
            1
        );
        assert_eq!(
            body["before_snapshot_id"],
            before.snapshot().snapshot_id().as_str()
        );
        assert_eq!(
            body["after_snapshot_id"],
            restored.snapshot().snapshot_id().as_str()
        );
        assert!(!format!("{restored:?}").contains("TEST_CODE-SECRET"));
    }
}

#[test]
fn w15_recovery_first_pending_event_has_acyclic_identity_and_exact_snapshot_join() {
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let assessment = ReadinessAssessment::evaluate(
        &catalog,
        &ReadinessScope::Core,
        &[],
        ReadinessStage::Startup,
        &[],
        &[],
    )
    .expect("TEST_CODE assessment");
    let record =
        CandidateReadinessRecord::try_new(None, context(100), assessment.clone(), vec![], vec![])
            .expect("TEST_CODE initial record");
    assert_eq!(record.kind(), ReadinessRecoveryKind::Pending);
    assert_eq!(record.before_snapshot_id(), None);
    assert_eq!(raw_digest(&record.event_bytes()), *record.event_sha256());
    let bytes = record.event_bytes();
    let body: serde_json::Value = serde_json::from_slice(
        bytes
            .strip_prefix(b"OperationalReadinessRecoveryEvent/v1\0")
            .expect("TEST_CODE event domain"),
    )
    .expect("TEST_CODE event JSON");
    assert_eq!(
        body["after_snapshot_id"],
        record.snapshot().snapshot_id().as_str()
    );
    assert_eq!(
        body["event_id"],
        record.snapshot().recovery_event_id().as_str()
    );
    assert!(body["before_snapshot_id"].is_null());
    assert_eq!(body["kind"], "Pending");
    assert_eq!(body["dependency_changes"], serde_json::json!([]));
    assert_eq!(body["recovery_claims"], serde_json::json!([]));
    let replay =
        CandidateReadinessRecord::try_new(None, context(100), assessment.clone(), vec![], vec![])
            .expect("TEST_CODE replay");
    assert_eq!(record, replay);
    let tick = CandidateReadinessRecord::try_new(
        Some(record.snapshot()),
        context(200),
        assessment,
        vec![],
        vec![],
    )
    .expect("TEST_CODE tick remains pending");
    assert_eq!(tick.kind(), ReadinessRecoveryKind::Pending);
    assert_eq!(
        tick.before_snapshot_id(),
        Some(record.snapshot().snapshot_id())
    );
    assert_ne!(
        record.snapshot().recovery_event_id(),
        tick.snapshot().recovery_event_id()
    );
}

#[test]
fn w15_recovery_claims_reject_cross_binding_duplicates_missing_roles_and_bad_times() {
    let (negative, negative_refs) = assessed(&ReadinessScope::Core, Some(DependencyKind::Schema));
    let before = CandidateReadinessRecord::try_new(
        None,
        context(100),
        negative,
        negative_refs.clone(),
        vec![],
    )
    .expect("TEST_CODE pending");
    let (ready, refs) = assessed(&ReadinessScope::Core, None);
    let claim = CandidateRecoveryClaim {
        evidence: refs[4].clone(),
        observed_at: UtcMicros::try_new(150).expect("TEST_CODE time"),
    };
    let build = |claims| {
        CandidateReadinessRecord::try_new(
            Some(before.snapshot()),
            context(200),
            ready.clone(),
            refs.clone(),
            claims,
        )
    };
    let invalid = |check, kind| ReadinessRecoveryError::InvalidClaims { check, kind };
    assert_eq!(
        build(vec![claim.clone(), claim.clone()]),
        Err(invalid("duplicate_dependency", DependencyKind::Schema))
    );
    let mut bad = claim.clone();
    bad.evidence = negative_refs[4].clone();
    assert_eq!(
        build(vec![bad]),
        Err(invalid("after_evidence_mismatch", DependencyKind::Schema))
    );
    let mut bad = claim.clone();
    bad.evidence = refs[5].clone();
    assert_eq!(
        build(vec![bad]),
        Err(invalid("unexpected_dependency", DependencyKind::Manifest))
    );
    for time in [99, 201] {
        let mut bad = claim.clone();
        bad.observed_at = UtcMicros::try_new(time).expect("TEST_CODE time");
        let error = build(vec![bad]).expect_err("TEST_CODE out-of-window claim");
        assert_eq!(error, invalid("observation_time", DependencyKind::Schema));
        assert!(!format!("{error:?} {error}").contains("TEST_CODE-SECRET"));
    }
    for time in [100, 200] {
        let mut boundary = claim.clone();
        boundary.observed_at = UtcMicros::try_new(time).expect("TEST_CODE boundary");
        assert!(build(vec![boundary]).is_ok());
    }
    let accepted = build(vec![claim.clone()]).expect("TEST_CODE explicit recovery");
    assert_eq!(accepted.snapshot().evidence_refs(), refs.as_slice());
    let mut later_claim = claim.clone();
    later_claim.observed_at = UtcMicros::try_new(151).expect("TEST_CODE later source observation");
    let later = build(vec![later_claim]).expect("TEST_CODE changed claim");
    assert_ne!(
        accepted.snapshot().recovery_event_id(),
        later.snapshot().recovery_event_id()
    );

    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let all_missing = ReadinessAssessment::evaluate(
        &catalog,
        &ReadinessScope::Core,
        ready.enabled_producers(),
        ReadinessStage::Running,
        &[],
        &[],
    )
    .expect("TEST_CODE missing");
    let missing =
        CandidateReadinessRecord::try_new(None, context(100), all_missing, vec![], vec![])
            .expect("TEST_CODE missing record");
    assert_eq!(
        CandidateReadinessRecord::try_new(
            Some(missing.snapshot()),
            context(200),
            ready.clone(),
            refs.clone(),
            vec![claim.clone()]
        ),
        Err(invalid("missing_dependency", DependencyKind::Namespace))
    );
    let claims: Vec<_> = refs
        .iter()
        .map(|evidence| CandidateRecoveryClaim {
            evidence: evidence.clone(),
            observed_at: claim.observed_at,
        })
        .collect();
    let ordered = CandidateReadinessRecord::try_new(
        Some(missing.snapshot()),
        context(200),
        ready.clone(),
        refs.clone(),
        claims.clone(),
    )
    .expect("TEST_CODE all recovered");
    let mut reversed = claims;
    reversed.reverse();
    let mut reversed_refs = refs.clone();
    reversed_refs.reverse();
    assert_eq!(
        ordered,
        CandidateReadinessRecord::try_new(
            Some(missing.snapshot()),
            context(200),
            ready.clone(),
            reversed_refs,
            reversed
        )
        .expect("TEST_CODE canonical replay")
    );
    assert_eq!(
        CandidateReadinessRecord::try_new(
            None,
            context(200),
            ready.clone(),
            refs.clone(),
            vec![claim.clone()]
        ),
        Err(ReadinessRecoveryError::UnexpectedClaims)
    );
    assert_eq!(
        CandidateReadinessRecord::try_new(
            Some(accepted.snapshot()),
            context(300),
            ready,
            refs,
            vec![claim]
        ),
        Err(ReadinessRecoveryError::UnexpectedClaims)
    );
}

#[test]
fn w15_recovery_rejects_cross_context_scope_enabled_set_and_backwards_capture() {
    let (assessment, refs) = assessed(&ReadinessScope::Core, Some(DependencyKind::Schema));
    let before = CandidateReadinessRecord::try_new(
        None,
        context(100),
        assessment.clone(),
        refs.clone(),
        vec![],
    )
    .expect("TEST_CODE before");
    let mut contexts = vec![];
    let mut changed = context(200);
    changed.namespace = Namespace::Production;
    contexts.push(changed);
    let mut changed = context(200);
    changed.business_date = BusinessDate::parse("2026-09-08").expect("TEST_CODE date");
    contexts.push(changed);
    let mut changed = context(200);
    changed.build_commit = GitSha40::parse(&"c".repeat(40)).expect("TEST_CODE build");
    contexts.push(changed);
    let mut changed = context(200);
    changed.activation_generation = 2;
    contexts.push(changed);
    let mut changed = context(200);
    changed.manifest_sha256 = raw_digest(b"different manifest");
    contexts.push(changed);
    for changed in contexts {
        assert_eq!(
            CandidateReadinessRecord::try_new(
                Some(before.snapshot()),
                changed,
                assessment.clone(),
                refs.clone(),
                vec![]
            ),
            Err(ReadinessRecoveryError::ContextMismatch)
        );
    }
    assert_eq!(
        CandidateReadinessRecord::try_new(
            Some(before.snapshot()),
            context(99),
            assessment.clone(),
            refs.clone(),
            vec![]
        ),
        Err(ReadinessRecoveryError::TimeRegression)
    );
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let changed = ReadinessAssessment::evaluate(
        &catalog,
        &ReadinessScope::Core,
        &[],
        ReadinessStage::Running,
        assessment.requirements(),
        assessment.observations(),
    )
    .expect("TEST_CODE changed coverage");
    assert_eq!(
        CandidateReadinessRecord::try_new(
            Some(before.snapshot()),
            context(200),
            changed,
            refs,
            vec![]
        ),
        Err(ReadinessRecoveryError::ContextMismatch)
    );
    let (changed, refs) = assessed(
        &ReadinessScope::Producer {
            unit_id: UnitId::try_new("MU-p01".to_owned()).expect("TEST_CODE unit"),
            producer_id: ProducerId::try_new("p01-scheduled".to_owned())
                .expect("TEST_CODE producer"),
        },
        Some(DependencyKind::SourceContract),
    );
    assert_eq!(
        CandidateReadinessRecord::try_new(
            Some(before.snapshot()),
            context(200),
            changed,
            refs,
            vec![]
        ),
        Err(ReadinessRecoveryError::ContextMismatch)
    );
}

#[test]
fn w15_recovery_event_retains_actual_old_and_new_contract_versions() {
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let (ready, refs) = assessed(&ReadinessScope::Core, None);
    let version = SourceContractVersion::try_new("v2".to_owned()).expect("TEST_CODE version");
    let mut requirements = ready.requirements().to_vec();
    requirements[4].version = version.clone();
    let negative = ReadinessAssessment::evaluate(
        &catalog,
        &ReadinessScope::Core,
        ready.enabled_producers(),
        ReadinessStage::Running,
        &requirements,
        ready.observations(),
    )
    .expect("TEST_CODE v1 does not fulfill v2");
    let before =
        CandidateReadinessRecord::try_new(None, context(100), negative, refs.clone(), vec![])
            .expect("TEST_CODE before");
    let mut observations = ready.observations().to_vec();
    let hash = raw_digest(b"TEST_CODE capability v2");
    observations[4] = DependencyObservation::Available {
        kind: DependencyKind::Schema,
        contract_id: requirements[4].contract_id.clone(),
        version: version.clone(),
        evidence_sha256: hash.clone(),
    };
    let mut evidence = refs;
    evidence[4] = ReadinessEvidenceRef::new(
        DependencyKind::Schema,
        ReadinessEvidenceKind::AuthorityArtifact,
        ProtectedRef::try_new("vault://TEST_CODE-SECRET/schema-v2".to_owned())
            .expect("TEST_CODE ref"),
        hash,
        requirements[4].contract_id.clone(),
        version,
    );
    let after = ReadinessAssessment::evaluate(
        &catalog,
        &ReadinessScope::Core,
        ready.enabled_producers(),
        ReadinessStage::Running,
        &requirements,
        &observations,
    )
    .expect("TEST_CODE v2 ready");
    let claim = CandidateRecoveryClaim {
        evidence: evidence[4].clone(),
        observed_at: UtcMicros::try_new(150).expect("TEST_CODE time"),
    };
    let record = CandidateReadinessRecord::try_new(
        Some(before.snapshot()),
        context(200),
        after,
        evidence,
        vec![claim],
    )
    .expect("TEST_CODE explicit version recovery");
    let bytes = record.event_bytes();
    let event: serde_json::Value = serde_json::from_slice(
        bytes
            .strip_prefix(b"OperationalReadinessRecoveryEvent/v1\0")
            .expect("TEST_CODE domain"),
    )
    .expect("TEST_CODE JSON");
    let change = &event["dependency_changes"][0];
    assert_eq!(change["kind"], "Schema");
    assert_eq!(change["before_requirement"]["version"], "v2");
    assert_eq!(change["after_requirement"]["version"], "v2");
    assert_eq!(change["before_observation"]["version"], "v1");
    assert_eq!(change["after_observation"]["version"], "v2");
    assert_eq!(event["recovery_claims"][0]["observed_at"], 150);
}
