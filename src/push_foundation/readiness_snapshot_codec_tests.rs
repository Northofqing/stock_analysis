use crate::monitor::push_job::{
    derive_occurrence_id, raw_digest, BusinessDate, GitSha40, MachineCatalog, Namespace,
    OccurrenceIdentityMaterial, OccurrenceKey, ProducerId, ProtectedRef, ReasonCode, RunId,
    Sha256Digest, SourceContractId, SourceContractVersion, UnitId, UtcMicros,
};

use super::operational_readiness::{
    DependencyFailure, DependencyKind, DependencyObservation, DependencyRequirement,
    ReadinessAssessment, ReadinessError, ReadinessScope, ReadinessStage, ReadinessStatus,
};
use super::readiness_snapshot::{
    CandidateReadinessSnapshot, ReadinessEvidenceKind, ReadinessEvidenceRef,
    ReadinessRecoveryEventId, ReadinessSnapshotContext,
};
use super::readiness_snapshot_codec::{decode_readiness_snapshot, ReadinessDecodeError};

fn test_namespace() -> Namespace {
    Namespace::test(RunId::try_new("TEST_CODE-w15-reload".to_owned()).expect("TEST_CODE run"))
}

fn snapshot_context(namespace: Namespace) -> ReadinessSnapshotContext {
    ReadinessSnapshotContext {
        namespace,
        business_date: BusinessDate::parse("2026-09-07").expect("TEST_CODE date"),
        build_commit: GitSha40::parse(&"1".repeat(40)).expect("TEST_CODE build"),
        activation_generation: 7,
        manifest_sha256: Sha256Digest::parse("TEST_CODE manifest", &"a".repeat(64))
            .expect("TEST_CODE hash"),
        captured_at: UtcMicros::try_new(100).expect("TEST_CODE time"),
    }
}

fn negative_snapshot(catalog: &MachineCatalog) -> CandidateReadinessSnapshot {
    let assessment = ReadinessAssessment::evaluate(
        catalog,
        &ReadinessScope::Core,
        &[],
        ReadinessStage::Running,
        &[],
        &[],
    )
    .expect("TEST_CODE missing core evidence");
    CandidateReadinessSnapshot::try_new(
        snapshot_context(test_namespace()),
        assessment,
        ReadinessRecoveryEventId::from_digest(raw_digest(b"TEST_CODE pending event")),
        Vec::new(),
    )
    .expect("TEST_CODE negative snapshot")
}

fn rewrite_fields(bytes: &[u8], edit: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
    let mut fields: serde_json::Value = serde_json::from_slice(
        bytes
            .strip_prefix(b"OperationalReadinessSnapshot/v1\0")
            .expect("TEST_CODE domain"),
    )
    .expect("TEST_CODE JSON");
    edit(&mut fields);
    let mut rewritten = b"OperationalReadinessSnapshot/v1\0".to_vec();
    rewritten.extend(serde_json::to_vec(&fields).expect("TEST_CODE encode edited JSON"));
    rewritten
}

#[test]
fn w15_snapshot_reload_rederives_readiness_instead_of_trusting_hash_valid_status() {
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let snapshot = negative_snapshot(&catalog);
    let bytes = snapshot.canonical_bytes();
    let restored = decode_readiness_snapshot(&catalog, snapshot.snapshot_id(), &bytes)
        .expect("TEST_CODE exact canonical reload");
    assert_eq!(restored, snapshot);
    assert_eq!(restored.assessment().status(), ReadinessStatus::CoreUnready);
    assert_eq!(
        decode_readiness_snapshot(&catalog, snapshot.snapshot_id(), b"TEST_CODE altered"),
        Err(ReadinessDecodeError::DigestMismatch)
    );
    let forged_ready = rewrite_fields(&bytes, |fields| {
        fields["status"] = "Ready".into();
        fields["reason"] = "activation.ready".into();
        fields["dependency_failures"] = serde_json::json!([]);
        fields["deployment_ready"] = true.into();
        fields["exit_disposition"] = "Continue".into();
    });
    assert_eq!(
        decode_readiness_snapshot(&catalog, &raw_digest(&forged_ready), &forged_ready),
        Err(ReadinessDecodeError::InconsistentSnapshot)
    );
}

fn assessed_snapshot(
    catalog: &MachineCatalog,
    scope: ReadinessScope,
    stage: ReadinessStage,
    unavailable: bool,
) -> CandidateReadinessSnapshot {
    let (failing, reason) = match scope {
        ReadinessScope::Core => (DependencyKind::Manifest, ReasonCode::ActivationCoreUnready),
        ReadinessScope::Producer { .. } => (
            DependencyKind::SourceContract,
            ReasonCode::InputSourceUnready,
        ),
        ReadinessScope::Occurrence { .. } => (
            DependencyKind::OccurrenceInput,
            ReasonCode::InputSourceUnready,
        ),
    };
    assessed_snapshot_with_override(
        catalog,
        scope,
        stage,
        unavailable.then_some((failing, None, None, Some(reason))),
        test_namespace(),
    )
}

fn assessed_snapshot_with_override(
    catalog: &MachineCatalog,
    scope: ReadinessScope,
    stage: ReadinessStage,
    observation_override: Option<(
        DependencyKind,
        Option<&str>,
        Option<&str>,
        Option<ReasonCode>,
    )>,
    namespace: Namespace,
) -> CandidateReadinessSnapshot {
    use DependencyKind::*;
    let roles = match scope {
        ReadinessScope::Core => vec![Namespace, Durable, Audit, TypedAuthority, Schema, Manifest],
        ReadinessScope::Producer { .. } => vec![
            ProducerBinding,
            SourceContract,
            ScheduleOrTrigger,
            Presentation,
            DurablePolicy,
            ReceiptStrength,
            FeatureGate,
            CompletionPolicy,
        ],
        ReadinessScope::Occurrence { .. } => vec![
            ProducerBinding,
            SourceContract,
            ScheduleOrTrigger,
            Presentation,
            DurablePolicy,
            ReceiptStrength,
            FeatureGate,
            CompletionPolicy,
            OccurrenceInput,
        ],
    };
    let mut requirements = Vec::new();
    let mut observations = Vec::new();
    let mut refs = Vec::new();
    for kind in roles {
        let contract_id = SourceContractId::try_new(format!("TEST_CODE-{}", kind.as_str()))
            .expect("TEST_CODE contract");
        let version = SourceContractVersion::try_new("v1".to_owned()).expect("TEST_CODE version");
        let sha = raw_digest(kind.as_str().as_bytes());
        requirements.push(DependencyRequirement {
            kind,
            contract_id: contract_id.clone(),
            version: version.clone(),
        });
        let (observed_contract_id, observed_version, unavailable_reason) =
            match observation_override {
                Some((target, source, observed_version, reason)) if target == kind => (
                    source
                        .map(|value| {
                            SourceContractId::try_new(value.to_owned())
                                .expect("TEST_CODE observed contract")
                        })
                        .unwrap_or_else(|| contract_id.clone()),
                    observed_version
                        .map(|value| {
                            SourceContractVersion::try_new(value.to_owned())
                                .expect("TEST_CODE observed version")
                        })
                        .unwrap_or_else(|| version.clone()),
                    reason,
                ),
                _ => (contract_id.clone(), version.clone(), None),
            };
        observations.push(if let Some(reason) = unavailable_reason {
            DependencyObservation::Unavailable {
                kind,
                contract_id: observed_contract_id.clone(),
                version: observed_version.clone(),
                evidence_sha256: sha.clone(),
                reason,
            }
        } else {
            DependencyObservation::Available {
                kind,
                contract_id: observed_contract_id.clone(),
                version: observed_version.clone(),
                evidence_sha256: sha.clone(),
            }
        });
        refs.push(ReadinessEvidenceRef::new(
            kind,
            if matches!(kind, SourceContract | OccurrenceInput) {
                ReadinessEvidenceKind::DataAcquisitionAudit
            } else {
                ReadinessEvidenceKind::AuthorityArtifact
            },
            ProtectedRef::try_new(format!("vault://TEST_CODE-SECRET/中文/{}", kind.as_str()))
                .expect("TEST_CODE URI"),
            sha,
            observed_contract_id,
            observed_version,
        ));
    }
    let enabled = [ProducerId::try_new("p01-scheduled".to_owned()).expect("TEST_CODE producer")];
    let assessed = ReadinessAssessment::evaluate(
        catalog,
        &scope,
        &enabled,
        stage,
        &requirements,
        &observations,
    )
    .expect("TEST_CODE typed assessment");
    CandidateReadinessSnapshot::try_new(
        snapshot_context(namespace),
        assessed,
        ReadinessRecoveryEventId::from_digest(raw_digest(b"TEST_CODE event")),
        refs,
    )
    .expect("TEST_CODE full candidate")
}

fn producer_scope() -> ReadinessScope {
    ReadinessScope::Producer {
        unit_id: UnitId::try_new("MU-p01".to_owned()).expect("TEST_CODE unit"),
        producer_id: ProducerId::try_new("p01-scheduled".to_owned()).expect("TEST_CODE producer"),
    }
}

fn occurrence_scope(catalog: &MachineCatalog) -> ReadinessScope {
    let producer_id = ProducerId::try_new("p01-scheduled".to_owned()).expect("TEST_CODE producer");
    ReadinessScope::Occurrence {
        unit_id: UnitId::try_new("MU-p01".to_owned()).expect("TEST_CODE unit"),
        occurrence_id: derive_occurrence_id(&OccurrenceIdentityMaterial::new(
            BusinessDate::parse("2026-09-07").expect("TEST_CODE date"),
            catalog
                .producer(&producer_id)
                .expect("TEST_CODE registration")
                .occurrence_family()
                .clone(),
            OccurrenceKey::try_new("p01:2026-09-07".to_owned()).expect("TEST_CODE key"),
        )),
        producer_id,
    }
}

#[test]
fn w15_snapshot_reload_roundtrips_all_scopes_stages_and_evidence_outcomes() {
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    for scope in [
        ReadinessScope::Core,
        producer_scope(),
        occurrence_scope(&catalog),
    ] {
        for stage in [ReadinessStage::Startup, ReadinessStage::Running] {
            for unavailable in [false, true] {
                let snapshot = assessed_snapshot(&catalog, scope.clone(), stage, unavailable);
                let restored = decode_readiness_snapshot(
                    &catalog,
                    snapshot.snapshot_id(),
                    &snapshot.canonical_bytes(),
                )
                .expect("TEST_CODE replay every scope/stage/outcome");
                assert_eq!(restored, snapshot);
                assert!(!format!("{restored:?}").contains("TEST_CODE-SECRET"));
            }
        }
    }
}

#[test]
fn w15_snapshot_reload_preserves_contract_and_version_mismatch_facts() {
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let cases = [
        (
            "TEST_CODE-other-source",
            "v1",
            None,
            DependencyFailure::ContractMismatch,
        ),
        (
            "TEST_CODE-SourceContract",
            "v2",
            None,
            DependencyFailure::VersionMismatch,
        ),
        (
            "TEST_CODE-other-source",
            "v2",
            None,
            DependencyFailure::ContractMismatch,
        ),
        (
            "TEST_CODE-SourceContract",
            "v2",
            Some(ReasonCode::InputSourceUnready),
            DependencyFailure::VersionMismatch,
        ),
    ];
    for (source, version, unavailable_reason, expected_failure) in cases {
        let snapshot = assessed_snapshot_with_override(
            &catalog,
            producer_scope(),
            ReadinessStage::Running,
            Some((
                DependencyKind::SourceContract,
                Some(source),
                Some(version),
                unavailable_reason,
            )),
            test_namespace(),
        );
        assert_eq!(
            snapshot.assessment().status(),
            ReadinessStatus::ProducerUnready
        );
        assert_eq!(
            snapshot.assessment().failures(),
            &[(DependencyKind::SourceContract, expected_failure)]
        );
        let observed = snapshot
            .assessment()
            .observations()
            .iter()
            .find(|observation| observation.kind() == DependencyKind::SourceContract)
            .expect("TEST_CODE source observation");
        assert_eq!(observed.contract_id().as_str(), source);
        assert_eq!(observed.version().as_str(), version);
        assert_eq!(
            observed.evidence_sha256(),
            &raw_digest(DependencyKind::SourceContract.as_str().as_bytes())
        );
        match (observed, unavailable_reason) {
            (DependencyObservation::Unavailable { reason, .. }, Some(expected)) => {
                assert_eq!(*reason, expected)
            }
            (DependencyObservation::Available { .. }, None) => {}
            _ => panic!("TEST_CODE observation outcome changed"),
        }

        let restored = decode_readiness_snapshot(
            &catalog,
            snapshot.snapshot_id(),
            &snapshot.canonical_bytes(),
        )
        .expect("TEST_CODE mismatch fact roundtrip");
        assert_eq!(
            restored.assessment().status(),
            snapshot.assessment().status()
        );
        assert_eq!(
            restored.assessment().failures(),
            snapshot.assessment().failures()
        );
        assert_eq!(
            restored.assessment().observations(),
            snapshot.assessment().observations()
        );
        assert_eq!(restored, snapshot);
    }
}

#[test]
fn w15_snapshot_reload_roundtrips_closed_allowed_unavailable_reasons() {
    use DependencyKind::*;
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let cases = vec![
        (
            ReadinessScope::Core,
            Namespace,
            ReasonCode::ActivationCoreUnready,
        ),
        (
            producer_scope(),
            ProducerBinding,
            ReasonCode::ActivationProducerUnready,
        ),
        (
            ReadinessScope::Core,
            Manifest,
            ReasonCode::ActivationManifestMismatch,
        ),
        (
            ReadinessScope::Core,
            Manifest,
            ReasonCode::ActivationGenerationConflict,
        ),
        (
            producer_scope(),
            ProducerBinding,
            ReasonCode::ActivationOwnerConflict,
        ),
        (producer_scope(), FeatureGate, ReasonCode::PolicyDisabled),
        (producer_scope(), FeatureGate, ReasonCode::PolicyStarved),
        (
            producer_scope(),
            FeatureGate,
            ReasonCode::PolicyOptInDisabled,
        ),
        (
            producer_scope(),
            SourceContract,
            ReasonCode::InputSourceUnavailable,
        ),
        (
            producer_scope(),
            SourceContract,
            ReasonCode::InputSourceUnready,
        ),
        (
            occurrence_scope(&catalog),
            OccurrenceInput,
            ReasonCode::InputNoVerifiedBatch,
        ),
        (
            occurrence_scope(&catalog),
            OccurrenceInput,
            ReasonCode::InputAccountSnapshotMissing,
        ),
        (
            ReadinessScope::Core,
            Namespace,
            ReasonCode::InputNamespaceViolation,
        ),
        (
            ReadinessScope::Core,
            Audit,
            ReasonCode::InputEvidenceInvalid,
        ),
    ];
    for (scope, kind, reason) in cases {
        let snapshot = assessed_snapshot_with_override(
            &catalog,
            scope,
            ReadinessStage::Running,
            Some((kind, None, None, Some(reason))),
            test_namespace(),
        );
        assert!(snapshot
            .assessment()
            .failures()
            .contains(&(kind, DependencyFailure::Unavailable { reason })));
        let restored = decode_readiness_snapshot(
            &catalog,
            snapshot.snapshot_id(),
            &snapshot.canonical_bytes(),
        )
        .expect("TEST_CODE allowed unavailable roundtrip");
        assert_eq!(
            restored.assessment().status(),
            snapshot.assessment().status()
        );
        assert_eq!(
            restored.assessment().failures(),
            snapshot.assessment().failures()
        );
        assert_eq!(
            restored.assessment().observations(),
            snapshot.assessment().observations()
        );
        assert_eq!(restored, snapshot);
    }
}

#[test]
fn w15_snapshot_reload_roundtrips_valid_production_and_test_namespaces() {
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    for namespace in [Namespace::Production, test_namespace()] {
        let snapshot = assessed_snapshot_with_override(
            &catalog,
            ReadinessScope::Core,
            ReadinessStage::Running,
            None,
            namespace,
        );
        let restored = decode_readiness_snapshot(
            &catalog,
            snapshot.snapshot_id(),
            &snapshot.canonical_bytes(),
        )
        .expect("TEST_CODE namespace roundtrip");
        assert_eq!(restored, snapshot);
    }
}

#[test]
fn w15_snapshot_reload_rejects_noncanonical_bytes_extra_fields_and_invalid_schema() {
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let bytes = negative_snapshot(&catalog).canonical_bytes();
    let mut whitespace = bytes.clone();
    whitespace.push(b'\n');
    let duplicate = String::from_utf8(bytes.clone())
        .expect("TEST_CODE UTF8")
        .replacen(
            "\"activation_generation\":7",
            "\"activation_generation\":7,\"activation_generation\":7",
            1,
        )
        .into_bytes();
    let extra = rewrite_fields(&bytes, |fields| {
        fields["TEST_CODE-SECRET"] = "vault://private".into()
    });
    let foreign_catalog = rewrite_fields(&bytes, |fields| {
        fields["catalog_sha256"] = "f".repeat(64).into()
    });
    for invalid_bytes in [whitespace, duplicate, extra, foreign_catalog] {
        let error =
            decode_readiness_snapshot(&catalog, &raw_digest(&invalid_bytes), &invalid_bytes)
                .expect_err("TEST_CODE invalid canonical record");
        assert_eq!(error, ReadinessDecodeError::InconsistentSnapshot);
        assert!(!format!("{error:?} {error}").contains("TEST_CODE-SECRET"));
    }
    let version = rewrite_fields(&bytes, |fields| fields["schema_version"] = 2.into());
    assert_eq!(
        decode_readiness_snapshot(&catalog, &raw_digest(&version), &version),
        Err(ReadinessDecodeError::UnsupportedSchemaVersion)
    );
    for (bytes, expected) in [
        (
            b"foreign-domain\0{}".to_vec(),
            ReadinessDecodeError::InvalidDomain,
        ),
        (
            b"OperationalReadinessSnapshot/v1\0{".to_vec(),
            ReadinessDecodeError::InvalidJson,
        ),
    ] {
        assert_eq!(
            decode_readiness_snapshot(&catalog, &raw_digest(&bytes), &bytes),
            Err(expected)
        );
    }
}

#[test]
fn w15_snapshot_reload_rejects_invalid_typed_inputs_and_protected_uri_without_echoing_them() {
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let snapshot = assessed_snapshot(
        &catalog,
        occurrence_scope(&catalog),
        ReadinessStage::Running,
        false,
    );
    let bytes = snapshot.canonical_bytes();
    let mutations: Vec<(fn(&mut serde_json::Value), ReadinessDecodeError)> = vec![
        (
            |fields| {
                fields["namespace"] = serde_json::json!({
                    "kind": "Production",
                    "run_id": "TEST_CODE-SECRET-production-run"
                })
            },
            ReadinessDecodeError::InvalidField { field: "namespace" },
        ),
        (
            |fields| {
                fields["namespace"] = serde_json::json!({
                    "kind": "Test",
                    "run_id": null
                })
            },
            ReadinessDecodeError::InvalidField { field: "run_id" },
        ),
        (
            |fields| {
                fields["namespace"] = serde_json::json!({
                    "kind": "TEST_CODE-SECRET-unknown",
                    "run_id": null
                })
            },
            ReadinessDecodeError::InvalidField { field: "namespace" },
        ),
        (
            |fields| fields["build_commit"] = "1".repeat(39).into(),
            ReadinessDecodeError::InvalidField {
                field: "build_commit",
            },
        ),
        (
            |fields| fields["build_commit"] = "g".repeat(40).into(),
            ReadinessDecodeError::InvalidField {
                field: "build_commit",
            },
        ),
        (
            |fields| fields["activation_generation"] = (-1).into(),
            ReadinessDecodeError::InvalidField {
                field: "activation_generation",
            },
        ),
        (
            |fields| fields["activation_generation"] = serde_json::json!(1.5),
            ReadinessDecodeError::InvalidField {
                field: "activation_generation",
            },
        ),
        (
            |fields| {
                fields["activation_generation"] = serde_json::from_str("18446744073709551616")
                    .expect("TEST_CODE oversized integer JSON")
            },
            ReadinessDecodeError::InvalidField {
                field: "activation_generation",
            },
        ),
        (
            |fields| fields["captured_at"] = u64::MAX.into(),
            ReadinessDecodeError::InvalidField {
                field: "captured_at",
            },
        ),
        (
            |fields| fields["business_date"] = "TEST_CODE-SECRET-invalid-date".into(),
            ReadinessDecodeError::InvalidField {
                field: "business_date",
            },
        ),
        (
            |fields| fields["manifest_sha256"] = "TEST_CODE-SECRET-invalid-digest".into(),
            ReadinessDecodeError::InvalidField {
                field: "manifest_sha256",
            },
        ),
        (
            |fields| fields["recovery_event_id"] = "g".repeat(64).into(),
            ReadinessDecodeError::InvalidField {
                field: "recovery_event_id",
            },
        ),
        (
            |fields| fields["scope"]["kind"] = "TEST_CODE-SECRET-unknown".into(),
            ReadinessDecodeError::InvalidField { field: "scope" },
        ),
        (
            |fields| fields["scope"]["unit_id"] = "".into(),
            ReadinessDecodeError::InvalidField { field: "unit_id" },
        ),
        (
            |fields| fields["scope"]["producer_id"] = " TEST_CODE-SECRET ".into(),
            ReadinessDecodeError::InvalidField {
                field: "producer_id",
            },
        ),
        (
            |fields| fields["scope"]["occurrence_id"] = "TEST_CODE-SECRET-invalid".into(),
            ReadinessDecodeError::InvalidField {
                field: "occurrence_id",
            },
        ),
        (
            |fields| fields["scope"]["unit_id"] = "MU-d01".into(),
            ReadinessDecodeError::InvalidAssessment(ReadinessError::InvalidScope {
                check: "producer_unit_mismatch",
            }),
        ),
        (
            |fields| fields["scope"]["producer_id"] = "TEST_CODE-unknown-producer".into(),
            ReadinessDecodeError::InvalidAssessment(ReadinessError::InvalidScope {
                check: "unknown_scope_producer",
            }),
        ),
        (
            |fields| fields["enabled_producer_ids"][0] = "TEST_CODE-unknown-producer".into(),
            ReadinessDecodeError::InvalidAssessment(ReadinessError::InvalidScope {
                check: "unknown_enabled_producer",
            }),
        ),
        (
            |fields| {
                let duplicate = fields["enabled_producer_ids"][0].clone();
                fields["enabled_producer_ids"]
                    .as_array_mut()
                    .expect("TEST_CODE enabled array")
                    .push(duplicate);
            },
            ReadinessDecodeError::InvalidAssessment(ReadinessError::InvalidScope {
                check: "duplicate_enabled_producer",
            }),
        ),
        (
            |fields| fields["dependency_refs"][0]["contract_id"] = "".into(),
            ReadinessDecodeError::InvalidField {
                field: "contract_id",
            },
        ),
        (
            |fields| {
                fields["dependency_refs"][0]["observation"]["contract_id"] =
                    " TEST_CODE-SECRET ".into()
            },
            ReadinessDecodeError::InvalidField {
                field: "contract_id",
            },
        ),
        (
            |fields| fields["dependency_refs"][0]["version"] = "".into(),
            ReadinessDecodeError::InvalidField { field: "version" },
        ),
        (
            |fields| {
                fields["dependency_refs"][0]["observation"]["version"] =
                    "TEST_CODE-SECRET\0version".into()
            },
            ReadinessDecodeError::InvalidField { field: "version" },
        ),
        (
            |fields| fields["evidence_refs"][0]["source_contract_id"] = "".into(),
            ReadinessDecodeError::InvalidField {
                field: "source_contract_id",
            },
        ),
        (
            |fields| {
                fields["evidence_refs"][0]["source_contract_version"] = " TEST_CODE-SECRET ".into()
            },
            ReadinessDecodeError::InvalidField {
                field: "source_contract_version",
            },
        ),
        (
            |fields| fields["stage"] = "Unknown".into(),
            ReadinessDecodeError::InvalidField { field: "stage" },
        ),
        (
            |fields| fields["dependency_refs"][0]["kind"] = "Unknown".into(),
            ReadinessDecodeError::InvalidField {
                field: "dependency_kind",
            },
        ),
        (
            |fields| fields["evidence_refs"][0]["kind"] = "Unknown".into(),
            ReadinessDecodeError::InvalidField {
                field: "evidence_kind",
            },
        ),
        (
            |fields| fields["evidence_refs"][0]["protected_uri"] = " TEST_CODE-SECRET ".into(),
            ReadinessDecodeError::InvalidField {
                field: "protected_uri",
            },
        ),
    ];
    for (mutate, expected) in mutations {
        let invalid_bytes = rewrite_fields(&bytes, mutate);
        let error =
            decode_readiness_snapshot(&catalog, &raw_digest(&invalid_bytes), &invalid_bytes)
                .expect_err("TEST_CODE invalid typed field");
        assert_eq!(error, expected);
        assert!(!format!("{error:?} {error}").contains("TEST_CODE-SECRET"));
    }
}
