use rusqlite::{params, Connection};

use crate::monitor::push_job::{
    raw_digest, BusinessDate, GitSha40, MachineCatalog, Namespace, ProtectedRef, RunId,
    Sha256Digest, UtcMicros,
};

use super::activation::{DesiredActivationState, PromotionAction};
use super::activation_readiness::{
    full_deployment_set_codec_fixture, read_activation_deployment_set, ActivationDeploymentSet,
};
use super::activation_readiness_tests::{
    append_generation, first_producer, request, two_unit_database,
};
use super::operational_readiness::{
    DependencyApplicability, DependencyKind, DependencyObservation, DependencyRequirement,
    ReadinessAssessment, ReadinessScope, ReadinessStage, ReadinessStatus,
};
use super::readiness_recovery::{
    CandidateReadinessRecord, CandidateRecoveryClaim, ReadinessRecoveryError, ReadinessRecoveryKind,
};
use super::readiness_snapshot::{
    snapshot_material_digest, CandidateReadinessSnapshot, ReadinessDeploymentSetContext,
    ReadinessEvidenceKind, ReadinessEvidenceRef, ReadinessRecoveryEventId,
    ReadinessSnapshotContext,
};
use super::readiness_snapshot_codec::{decode_readiness_snapshot, ReadinessDecodeError};
use super::readiness_store::{
    ReadinessAppendFault, ReadinessRecordStore, ReadinessStoreError, ReadinessStreamId,
};
use super::readiness_store_schema::initialize_database;

fn digest(value: char) -> Sha256Digest {
    Sha256Digest::parse("TEST_CODE digest", &value.to_string().repeat(64))
        .expect("TEST_CODE digest")
}

pub(super) fn deployment_assessment(
    catalog: &MachineCatalog,
    set: &ActivationDeploymentSet,
    missing: Option<DependencyKind>,
    stage: ReadinessStage,
) -> (ReadinessAssessment, Vec<ReadinessEvidenceRef>) {
    let requirements = set
        .shared_dependencies()
        .iter()
        .map(|dependency| DependencyRequirement {
            kind: dependency.kind(),
            contract_id: dependency.contract_id().clone(),
            version: dependency.contract_version().clone(),
            expected_authority: ReadinessEvidenceKind::AuthorityArtifact,
            applicability: DependencyApplicability::Required,
        })
        .collect::<Vec<_>>();
    let observations = set
        .shared_dependencies()
        .iter()
        .filter(|dependency| Some(dependency.kind()) != missing)
        .map(|dependency| DependencyObservation::Available {
            kind: dependency.kind(),
            contract_id: dependency.contract_id().clone(),
            version: dependency.contract_version().clone(),
            evidence_sha256: dependency.sha256().clone(),
        })
        .collect::<Vec<_>>();
    let evidence = set
        .shared_dependencies()
        .iter()
        .filter(|dependency| Some(dependency.kind()) != missing)
        .map(|dependency| {
            ReadinessEvidenceRef::new(
                dependency.kind(),
                ReadinessEvidenceKind::AuthorityArtifact,
                ProtectedRef::try_new(format!(
                    "vault://TEST_CODE-SECRET/v3/{}",
                    dependency.kind().as_str()
                ))
                .expect("TEST_CODE protected ref"),
                dependency.sha256().clone(),
                dependency.contract_id().clone(),
                dependency.contract_version().clone(),
            )
        })
        .collect::<Vec<_>>();
    let assessment = ReadinessAssessment::evaluate_for_deployment_set(
        catalog,
        set,
        &ReadinessScope::Core,
        stage,
        &requirements,
        &observations,
    )
    .expect("TEST_CODE deployment assessment");
    (assessment, evidence)
}

pub(super) fn v3_context(
    set: ActivationDeploymentSet,
    captured_at: i64,
) -> ReadinessDeploymentSetContext {
    ReadinessDeploymentSetContext::new(
        BusinessDate::parse("2026-09-08").expect("TEST_CODE date"),
        UtcMicros::try_new(captured_at).expect("TEST_CODE time"),
        set,
    )
}

fn rewrite_v3(bytes: &[u8], edit: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
    let mut value: serde_json::Value = serde_json::from_slice(
        bytes
            .strip_prefix(b"OperationalReadinessSnapshot/v3\0")
            .expect("TEST_CODE v3 domain"),
    )
    .expect("TEST_CODE v3 JSON");
    edit(&mut value);
    let mut rewritten = b"OperationalReadinessSnapshot/v3\0".to_vec();
    rewritten.extend(serde_json::to_vec(&value).expect("TEST_CODE rewritten JSON"));
    rewritten
}

fn rehash_embedded_set(value: &mut serde_json::Value) {
    let mut bytes = b"ActivationDeploymentSet/v1\0".to_vec();
    bytes.extend(
        serde_json::to_vec(&value["deployment_set"]).expect("TEST_CODE deployment set JSON"),
    );
    value["deployment_set_sha256"] = raw_digest(&bytes).as_str().into();
}

#[test]
fn readiness_v2_and_v3_have_independent_fixed_snapshot_material_and_stream_golden_hashes() {
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    assert_eq!(
        catalog.catalog_sha256().as_str(),
        "0aa6a2fd87ee9c235073cad3beef44229437f3fe62987b0db510ad36a93aace3"
    );
    let v2_assessment = ReadinessAssessment::evaluate(
        &catalog,
        &ReadinessScope::Core,
        &[],
        ReadinessStage::Startup,
        &[],
        &[],
    )
    .expect("TEST_CODE v2 assessment");
    let v2 = CandidateReadinessSnapshot::try_new(
        ReadinessSnapshotContext {
            namespace: Namespace::test(
                RunId::try_new("TEST_CODE-w15-v2-golden".to_owned()).expect("TEST_CODE run"),
            ),
            business_date: BusinessDate::parse("2026-09-07").expect("TEST_CODE date"),
            build_commit: GitSha40::parse(&"1".repeat(40)).expect("TEST_CODE commit"),
            activation_generation: 7,
            manifest_sha256: digest('a'),
            captured_at: UtcMicros::try_new(100).expect("TEST_CODE time"),
        },
        v2_assessment,
        ReadinessRecoveryEventId::from_digest(digest('b')),
        vec![],
    )
    .expect("TEST_CODE v2 snapshot");
    assert_eq!(v2.canonical_bytes().len(), 1_211);
    assert_eq!(
        v2.snapshot_id().as_str(),
        "2df91aa3a62fa3b555953124759a556a97fd484f9997ebf4a1136f88f1b4c681"
    );
    assert_eq!(
        snapshot_material_digest(v2.versioned_context(), v2.assessment(), v2.evidence_refs())
            .as_str(),
        "77f09d1d64980cd6e664dab84bc91c4f73be821ed5762fc853b88a2392aac1c6"
    );
    assert_eq!(
        ReadinessStreamId::for_snapshot(&v2).as_str(),
        "545888c4c852226914383a5041519b18925d40ea718e52675089d952c4bfb426"
    );

    let set = full_deployment_set_codec_fixture();
    assert_eq!(
        set.sha256().as_str(),
        "1033a23359f8c8a4b3581d3512462f850bb13fed29e2a169b62a62721d8eb179"
    );
    let v3_assessment = ReadinessAssessment::evaluate_for_deployment_set(
        &catalog,
        &set,
        &ReadinessScope::Core,
        ReadinessStage::Startup,
        &[],
        &[],
    )
    .expect("TEST_CODE v3 assessment");
    let v3 = CandidateReadinessSnapshot::try_new_v3(
        &catalog,
        ReadinessDeploymentSetContext::new(
            BusinessDate::parse("2026-09-07").expect("TEST_CODE date"),
            UtcMicros::try_new(100).expect("TEST_CODE time"),
            set,
        ),
        v3_assessment,
        ReadinessRecoveryEventId::from_digest(digest('b')),
        vec![],
    )
    .expect("TEST_CODE v3 snapshot");
    assert_eq!(v3.canonical_bytes().len(), 16_337);
    assert_eq!(
        v3.snapshot_id().as_str(),
        "719974470c9b4cb6ec7dc8d457bb304d432a39b347eb30827c5fecac0e8d6a39"
    );
    assert_eq!(
        snapshot_material_digest(v3.versioned_context(), v3.assessment(), v3.evidence_refs())
            .as_str(),
        "fdb017f7190f7d700ce499a5af8494994cd8b72afab3ca794a2fad80a0c5deef"
    );
    assert_eq!(
        ReadinessStreamId::for_snapshot(&v3).as_str(),
        "f30068beef005595098935d48161b19f330bb1e4b2d47212900acfd977715c4e"
    );
}

#[test]
fn readiness_v3_codec_rebuilds_the_full_set_and_rejects_mixed_or_malformed_versions() {
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let set = full_deployment_set_codec_fixture();
    let assessment = ReadinessAssessment::evaluate_for_deployment_set(
        &catalog,
        &set,
        &ReadinessScope::Core,
        ReadinessStage::Startup,
        &[],
        &[],
    )
    .expect("TEST_CODE assessment");
    let snapshot = CandidateReadinessSnapshot::try_new_v3(
        &catalog,
        ReadinessDeploymentSetContext::new(
            BusinessDate::parse("2026-09-07").expect("TEST_CODE date"),
            UtcMicros::try_new(100).expect("TEST_CODE time"),
            set,
        ),
        assessment,
        ReadinessRecoveryEventId::from_digest(digest('b')),
        vec![],
    )
    .expect("TEST_CODE snapshot");
    let bytes = snapshot.canonical_bytes();
    let restored = decode_readiness_snapshot(&catalog, snapshot.snapshot_id(), &bytes)
        .expect("TEST_CODE v3 roundtrip");
    assert_eq!(restored, snapshot);
    assert_eq!(
        restored
            .deployment_set()
            .expect("TEST_CODE set")
            .unit_generations()
            .len(),
        52
    );

    let old_scalar = rewrite_v3(&bytes, |value| value["activation_generation"] = 7.into());
    assert_eq!(
        decode_readiness_snapshot(&catalog, &raw_digest(&old_scalar), &old_scalar),
        Err(ReadinessDecodeError::InconsistentSnapshot)
    );
    let string_set = rewrite_v3(&bytes, |value| value["deployment_set"] = "{}".into());
    assert_eq!(
        decode_readiness_snapshot(&catalog, &raw_digest(&string_set), &string_set),
        Err(ReadinessDecodeError::InvalidField {
            field: "deployment_set"
        })
    );
    let missing_unit = rewrite_v3(&bytes, |value| {
        value["deployment_set"]["units"]
            .as_array_mut()
            .expect("TEST_CODE units")
            .pop();
    });
    assert_eq!(
        decode_readiness_snapshot(&catalog, &raw_digest(&missing_unit), &missing_unit),
        Err(ReadinessDecodeError::InvalidDeploymentSet {
            check: "unit_coverage"
        })
    );
    for changed_units in [
        rewrite_v3(&bytes, |value| {
            let duplicate = value["deployment_set"]["units"][0].clone();
            value["deployment_set"]["units"]
                .as_array_mut()
                .expect("TEST_CODE units")
                .push(duplicate);
        }),
        rewrite_v3(&bytes, |value| {
            value["deployment_set"]["units"][0]["unit_id"] = "MU-unknown".into();
        }),
    ] {
        assert_eq!(
            decode_readiness_snapshot(&catalog, &raw_digest(&changed_units), &changed_units),
            Err(ReadinessDecodeError::InvalidDeploymentSet {
                check: "unit_coverage"
            })
        );
    }
    let invalid_recovery = rewrite_v3(&bytes, |value| {
        let unit = value["deployment_set"]["units"][0]["unit_id"].clone();
        value["deployment_set"]["recovery_units"] = serde_json::json!([unit]);
        rehash_embedded_set(value);
    });
    assert_eq!(
        decode_readiness_snapshot(&catalog, &raw_digest(&invalid_recovery), &invalid_recovery,),
        Err(ReadinessDecodeError::InvalidDeploymentSet {
            check: "configuration"
        })
    );
    let duplicate_dependency = rewrite_v3(&bytes, |value| {
        let dependency = value["deployment_set"]["shared_dependencies"][0].clone();
        value["deployment_set"]["shared_dependencies"]
            .as_array_mut()
            .expect("TEST_CODE dependencies")
            .push(dependency);
        rehash_embedded_set(value);
    });
    assert_eq!(
        decode_readiness_snapshot(
            &catalog,
            &raw_digest(&duplicate_dependency),
            &duplicate_dependency,
        ),
        Err(ReadinessDecodeError::InvalidDeploymentSet {
            check: "shared_dependencies"
        })
    );
    let enabled_unregistered = rewrite_v3(&bytes, |value| {
        value["deployment_set"]["enabled_producers"] = serde_json::json!(["p01-scheduled"]);
        rehash_embedded_set(value);
    });
    assert_eq!(
        decode_readiness_snapshot(
            &catalog,
            &raw_digest(&enabled_unregistered),
            &enabled_unregistered,
        ),
        Err(ReadinessDecodeError::InvalidDeploymentSet {
            check: "configuration"
        })
    );
    let wrong_hash = rewrite_v3(&bytes, |value| {
        value["deployment_set_sha256"] = "f".repeat(64).into()
    });
    assert_eq!(
        decode_readiness_snapshot(&catalog, &raw_digest(&wrong_hash), &wrong_hash),
        Err(ReadinessDecodeError::InvalidDeploymentSet {
            check: "digest_or_canonical"
        })
    );
    let duplicate = String::from_utf8(bytes.clone())
        .expect("TEST_CODE UTF-8")
        .replacen(
            "\"schema_version\":3",
            "\"schema_version\":3,\"schema_version\":3",
            1,
        )
        .into_bytes();
    assert_eq!(
        decode_readiness_snapshot(&catalog, &raw_digest(&duplicate), &duplicate),
        Err(ReadinessDecodeError::InconsistentSnapshot)
    );
    let wrong_schema = rewrite_v3(&bytes, |value| value["schema_version"] = 2.into());
    assert_eq!(
        decode_readiness_snapshot(&catalog, &raw_digest(&wrong_schema), &wrong_schema),
        Err(ReadinessDecodeError::UnsupportedSchemaVersion)
    );
    let changed_date = rewrite_v3(&bytes, |value| value["business_date"] = "2026-09-08".into());
    let changed = decode_readiness_snapshot(&catalog, &raw_digest(&changed_date), &changed_date)
        .expect("TEST_CODE valid date creates distinct identity");
    assert_ne!(changed.snapshot_id(), snapshot.snapshot_id());
    let changed_dependency = rewrite_v3(&bytes, |value| {
        value["deployment_set"]["shared_dependencies"][0]["sha256"] = "f".repeat(64).into();
        rehash_embedded_set(value);
    });
    let changed = decode_readiness_snapshot(
        &catalog,
        &raw_digest(&changed_dependency),
        &changed_dependency,
    )
    .expect("TEST_CODE self-consistent declaration is a distinct candidate identity");
    assert_ne!(changed.snapshot_id(), snapshot.snapshot_id());
    let namespace_mismatch = rewrite_v3(&bytes, |value| {
        value["namespace"]["run_id"] = "TEST_CODE-w16-v3-foreign".into()
    });
    assert_eq!(
        decode_readiness_snapshot(
            &catalog,
            &raw_digest(&namespace_mismatch),
            &namespace_mismatch,
        ),
        Err(ReadinessDecodeError::InconsistentSnapshot)
    );
}

#[test]
fn readiness_v3_assessment_rejects_shared_dependency_contract_version_and_hash_conflicts() {
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let set = full_deployment_set_codec_fixture();
    let (baseline, _) = deployment_assessment(&catalog, &set, None, ReadinessStage::Running);
    let mut requirements = baseline.requirements().to_vec();
    requirements[0].contract_id =
        crate::monitor::push_job::SourceContractId::try_new("TEST_CODE-foreign".to_owned())
            .expect("TEST_CODE contract");
    assert!(matches!(
        ReadinessAssessment::evaluate_for_deployment_set(
            &catalog,
            &set,
            &ReadinessScope::Core,
            ReadinessStage::Running,
            &requirements,
            baseline.observations(),
        ),
        Err(
            super::operational_readiness::ReadinessError::InvalidDependencySet {
                check: "deployment_set_contract_mismatch",
                ..
            }
        )
    ));

    let requirements = baseline.requirements().to_vec();
    let mut observations = baseline.observations().to_vec();
    let original = observations[0].clone();
    observations[0] = match original {
        DependencyObservation::Available {
            kind,
            contract_id,
            version,
            ..
        } => DependencyObservation::Available {
            kind,
            contract_id,
            version,
            evidence_sha256: digest('f'),
        },
        _ => unreachable!("TEST_CODE baseline observations are available"),
    };
    assert!(matches!(
        ReadinessAssessment::evaluate_for_deployment_set(
            &catalog,
            &set,
            &ReadinessScope::Core,
            ReadinessStage::Running,
            &requirements,
            &observations,
        ),
        Err(
            super::operational_readiness::ReadinessError::InvalidDependencySet {
                check: "deployment_set_observation_hash_mismatch",
                ..
            }
        )
    ));
}

#[test]
fn readiness_v3_real_set_flows_through_recovery_and_the_existing_store() {
    let (_activation_root, activation_database, bindings) =
        two_unit_database("TEST_CODE-v3-activation.sqlite3");
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let enabled = first_producer(&bindings[0].unit_id);
    let set = read_activation_deployment_set(
        &activation_database,
        &bindings[0].unit_id,
        request(
            &bindings,
            vec![enabled.clone()],
            vec![bindings[1].unit_id.clone()],
        ),
    )
    .expect("TEST_CODE actual deployment set");
    let generations = set.unit_generations();
    assert_eq!(generations.len(), 52);
    assert_eq!(
        generations
            .iter()
            .find(|(unit, _)| *unit == &bindings[0].unit_id)
            .map(|(_, generation)| *generation),
        Some(Some(2))
    );
    assert_eq!(
        generations
            .iter()
            .find(|(unit, _)| *unit == &bindings[1].unit_id)
            .map(|(_, generation)| *generation),
        Some(Some(1))
    );
    assert_eq!(set.recovery_units().len(), 1);
    assert_eq!(&set.recovery_units()[0], &bindings[1].unit_id);
    let (pending_assessment, pending_evidence) = deployment_assessment(
        &catalog,
        &set,
        Some(DependencyKind::Schema),
        ReadinessStage::Running,
    );
    assert_eq!(pending_assessment.status(), ReadinessStatus::CoreUnready);
    assert!(pending_assessment
        .affected_unit_ids()
        .contains(&bindings[1].unit_id));
    assert_eq!(pending_assessment.enabled_producers(), &[enabled]);
    let pending = CandidateReadinessRecord::try_new_v3(
        &catalog,
        None,
        v3_context(set.clone(), 100),
        pending_assessment,
        pending_evidence,
        vec![],
    )
    .expect("TEST_CODE pending v3 record");

    let readiness_root = tempfile::tempdir().expect("TEST_CODE readiness root");
    let readiness_database = readiness_root
        .path()
        .canonicalize()
        .expect("TEST_CODE canonical readiness root")
        .join("readiness.sqlite3");
    let namespace = set.namespace().clone();
    initialize_database(&readiness_database, &namespace).expect("TEST_CODE readiness database");
    let store = ReadinessRecordStore::at(&readiness_database, &namespace, &catalog);
    let foreign_namespace = Namespace::test(
        RunId::try_new("TEST_CODE-foreign-v3-store".to_owned()).expect("TEST_CODE run"),
    );
    assert_eq!(
        ReadinessRecordStore::at(&readiness_database, &foreign_namespace, &catalog)
            .append(None, &pending),
        Err(ReadinessStoreError::NamespaceMismatch)
    );
    let first = store
        .append(None, &pending)
        .expect("TEST_CODE append pending");
    assert_eq!(
        store
            .append(None, &pending)
            .expect("TEST_CODE idempotent replay"),
        first
    );
    let stream = ReadinessStreamId::for_snapshot(pending.snapshot());
    assert_eq!(
        ReadinessRecordStore::at(&readiness_database, &namespace, &catalog)
            .load_head(&stream)
            .expect("TEST_CODE reopen v3"),
        Some(first.clone())
    );

    let (ready_assessment, ready_evidence) =
        deployment_assessment(&catalog, &set, None, ReadinessStage::Running);
    assert_eq!(ready_assessment.status(), ReadinessStatus::Ready);
    assert!(ready_assessment.affected_unit_ids().is_empty());
    assert_eq!(
        CandidateReadinessRecord::try_new_v3(
            &catalog,
            Some(pending.snapshot()),
            v3_context(set.clone(), 200),
            ready_assessment.clone(),
            ready_evidence.clone(),
            vec![],
        ),
        Err(ReadinessRecoveryError::ExplicitRecoveryRequired)
    );
    let recovered_evidence = ready_evidence
        .iter()
        .find(|evidence| evidence.dependency_kind() == DependencyKind::Schema)
        .expect("TEST_CODE restored evidence")
        .clone();
    let recovered = CandidateReadinessRecord::try_new_v3(
        &catalog,
        Some(pending.snapshot()),
        v3_context(set, 200),
        ready_assessment,
        ready_evidence,
        vec![CandidateRecoveryClaim {
            evidence: recovered_evidence,
            observed_at: UtcMicros::try_new(150).expect("TEST_CODE observation"),
        }],
    )
    .expect("TEST_CODE explicit v3 recovery");
    assert_eq!(
        recovered.kind(),
        ReadinessRecoveryKind::CoreDependenciesRestored
    );
    let second = store
        .append(Some(&first), &recovered)
        .expect("TEST_CODE append recovery");
    let reopened = ReadinessRecordStore::at(&readiness_database, &namespace, &catalog);
    assert_eq!(
        reopened
            .load_record(recovered.snapshot().snapshot_id())
            .expect("TEST_CODE load recovery"),
        second
    );
    assert!(!format!("{recovered:?}").contains("TEST_CODE-SECRET"));
}

#[test]
fn readiness_v3_changes_start_distinct_streams_without_reusing_old_pending_history() {
    let (_activation_root, activation_database, mut bindings) =
        two_unit_database("TEST_CODE-v3-streams.sqlite3");
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let enabled = first_producer(&bindings[0].unit_id);
    let set_one = read_activation_deployment_set(
        &activation_database,
        &bindings[0].unit_id,
        request(
            &bindings,
            vec![enabled.clone()],
            vec![bindings[1].unit_id.clone()],
        ),
    )
    .expect("TEST_CODE first set");
    let (assessment_one, evidence_one) = deployment_assessment(
        &catalog,
        &set_one,
        Some(DependencyKind::Schema),
        ReadinessStage::Running,
    );
    let pending_one = CandidateReadinessRecord::try_new_v3(
        &catalog,
        None,
        v3_context(set_one.clone(), 100),
        assessment_one,
        evidence_one,
        vec![],
    )
    .expect("TEST_CODE first pending");
    let changed = append_generation(
        &activation_database,
        &bindings[1].unit_id,
        2,
        Some(&bindings[1]),
        DesiredActivationState::Shadow,
        PromotionAction::EnterShadow,
        'd',
        '4',
    );
    bindings[1] = changed;
    let set_two = read_activation_deployment_set(
        &activation_database,
        &bindings[0].unit_id,
        request(&bindings, vec![enabled], vec![bindings[1].unit_id.clone()]),
    )
    .expect("TEST_CODE changed set");
    let (assessment_two, evidence_two) = deployment_assessment(
        &catalog,
        &set_two,
        Some(DependencyKind::Schema),
        ReadinessStage::Running,
    );
    assert_eq!(
        CandidateReadinessRecord::try_new_v3(
            &catalog,
            Some(pending_one.snapshot()),
            v3_context(set_two.clone(), 200),
            assessment_two.clone(),
            evidence_two.clone(),
            vec![],
        ),
        Err(ReadinessRecoveryError::ContextMismatch)
    );
    let pending_two = CandidateReadinessRecord::try_new_v3(
        &catalog,
        None,
        v3_context(set_two, 200),
        assessment_two,
        evidence_two,
        vec![],
    )
    .expect("TEST_CODE explicit new genesis");
    let stream_one = ReadinessStreamId::for_snapshot(pending_one.snapshot());
    let stream_two = ReadinessStreamId::for_snapshot(pending_two.snapshot());
    assert_ne!(stream_one, stream_two);

    let root = tempfile::tempdir().expect("TEST_CODE readiness root");
    let database = root
        .path()
        .canonicalize()
        .expect("TEST_CODE canonical readiness root")
        .join("readiness.sqlite3");
    let namespace = set_one.namespace().clone();
    initialize_database(&database, &namespace).expect("TEST_CODE readiness database");
    let store = ReadinessRecordStore::at(&database, &namespace, &catalog);
    let first = store
        .append(None, &pending_one)
        .expect("TEST_CODE first stream");
    assert_eq!(
        store.append(Some(&first), &pending_two),
        Err(ReadinessStoreError::HeadConflict)
    );
    let second = store
        .append(None, &pending_two)
        .expect("TEST_CODE new stream");
    assert_eq!(
        store.load_head(&stream_one).expect("TEST_CODE old head"),
        Some(first.clone())
    );
    assert_eq!(
        store.load_head(&stream_two).expect("TEST_CODE new head"),
        Some(second.clone())
    );
    let legacy_assessment = ReadinessAssessment::evaluate(
        &catalog,
        &ReadinessScope::Core,
        &[],
        ReadinessStage::Running,
        &[],
        &[],
    )
    .expect("TEST_CODE legacy assessment");
    let legacy = CandidateReadinessRecord::try_new(
        None,
        ReadinessSnapshotContext {
            namespace: namespace.clone(),
            business_date: BusinessDate::parse("2026-09-08").expect("TEST_CODE date"),
            build_commit: GitSha40::parse(&"a".repeat(40)).expect("TEST_CODE commit"),
            activation_generation: 1,
            manifest_sha256: digest('a'),
            captured_at: UtcMicros::try_new(200).expect("TEST_CODE time"),
        },
        legacy_assessment,
        vec![],
        vec![],
    )
    .expect("TEST_CODE legacy record");
    assert_eq!(
        CandidateReadinessRecord::try_new_v3(
            &catalog,
            Some(legacy.snapshot()),
            v3_context(
                pending_two
                    .snapshot()
                    .deployment_set()
                    .expect("TEST_CODE set")
                    .clone(),
                300,
            ),
            pending_two.snapshot().assessment().clone(),
            pending_two.snapshot().evidence_refs().to_vec(),
            vec![],
        ),
        Err(ReadinessRecoveryError::ContextMismatch)
    );
    let legacy_stream = ReadinessStreamId::for_snapshot(legacy.snapshot());
    assert_ne!(legacy_stream, stream_one);
    assert_ne!(legacy_stream, stream_two);
    let legacy_stored = store
        .append(None, &legacy)
        .expect("TEST_CODE legacy coexists");
    assert_eq!(
        store
            .load_head(&legacy_stream)
            .expect("TEST_CODE legacy head"),
        Some(legacy_stored)
    );

    let pending_three = CandidateReadinessRecord::try_new_v3(
        &catalog,
        Some(pending_two.snapshot()),
        v3_context(
            pending_two
                .snapshot()
                .deployment_set()
                .expect("TEST_CODE set")
                .clone(),
            300,
        ),
        pending_two.snapshot().assessment().clone(),
        pending_two.snapshot().evidence_refs().to_vec(),
        vec![],
    )
    .expect("TEST_CODE same-stream successor");
    let third = store
        .append_with_fault(
            Some(&second),
            &pending_three,
            ReadinessAppendFault::CommitConfirmationLost,
        )
        .expect("TEST_CODE confirmation loss resolved by exact reload");
    assert_eq!(third.version(), 2);

    let connection = Connection::open(&database).expect("TEST_CODE tamper connection");
    let trigger_sql: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='trigger' AND name='operational_readiness_recovery_event_no_update'",
            [],
            |row| row.get(0),
        )
        .expect("TEST_CODE event trigger");
    connection
        .execute_batch("DROP TRIGGER operational_readiness_recovery_event_no_update;")
        .expect("TEST_CODE drop event trigger");
    connection
        .execute(
            "UPDATE operational_readiness_recovery_event SET before_snapshot_id=?1 WHERE event_id=?2",
            params![
                pending_one.snapshot().snapshot_id().as_str(),
                pending_three.snapshot().recovery_event_id().as_str()
            ],
        )
        .expect("TEST_CODE forge cross-stream predecessor");
    connection
        .execute_batch(&trigger_sql)
        .expect("TEST_CODE restore event trigger");
    drop(connection);
    assert_eq!(
        store.load_head(&stream_two),
        Err(ReadinessStoreError::Corrupt {
            check: "chain_stream"
        })
    );
}
