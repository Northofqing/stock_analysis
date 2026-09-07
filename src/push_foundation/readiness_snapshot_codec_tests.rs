use crate::monitor::push_job::{
    derive_occurrence_id, raw_digest, BusinessDate, GitSha40, MachineCatalog, Namespace,
    OccurrenceIdentityMaterial, OccurrenceKey, ProducerId, ProtectedRef, ReasonCode, RunId,
    Sha256Digest, SourceContractId, SourceContractVersion, UnitId, UtcMicros,
};

use super::operational_readiness::{
    DependencyKind, DependencyObservation, DependencyRequirement, ReadinessAssessment,
    ReadinessScope, ReadinessStage, ReadinessStatus,
};
use super::readiness_snapshot::{
    CandidateReadinessSnapshot, ReadinessEvidenceKind, ReadinessEvidenceRef,
    ReadinessRecoveryEventId, ReadinessSnapshotContext,
};
use super::readiness_snapshot_codec::{decode_readiness_snapshot, ReadinessDecodeError};

fn negative_snapshot(catalog: &MachineCatalog) -> CandidateReadinessSnapshot {
    let context = ReadinessSnapshotContext {
        namespace: Namespace::test(
            RunId::try_new("TEST_CODE-w15-reload".to_owned()).expect("TEST_CODE run"),
        ),
        business_date: BusinessDate::parse("2026-09-07").expect("TEST_CODE date"),
        build_commit: GitSha40::parse(&"1".repeat(40)).expect("TEST_CODE build"),
        activation_generation: 7,
        manifest_sha256: Sha256Digest::parse("TEST_CODE manifest", &"a".repeat(64))
            .expect("TEST_CODE hash"),
        captured_at: UtcMicros::try_new(100).expect("TEST_CODE time"),
    };
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
        context,
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
    use DependencyKind::*;
    let (roles, failing, reason) = match scope {
        ReadinessScope::Core => (
            vec![Namespace, Durable, Audit, TypedAuthority, Schema, Manifest],
            Manifest,
            ReasonCode::ActivationCoreUnready,
        ),
        ReadinessScope::Producer { .. } => (
            vec![
                ProducerBinding,
                SourceContract,
                ScheduleOrTrigger,
                Presentation,
                DurablePolicy,
                ReceiptStrength,
                FeatureGate,
                CompletionPolicy,
            ],
            SourceContract,
            ReasonCode::InputSourceUnready,
        ),
        ReadinessScope::Occurrence { .. } => (
            vec![
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
            OccurrenceInput,
            ReasonCode::InputSourceUnready,
        ),
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
        observations.push(if unavailable && kind == failing {
            DependencyObservation::Unavailable {
                kind,
                contract_id: contract_id.clone(),
                version: version.clone(),
                evidence_sha256: sha.clone(),
                reason,
            }
        } else {
            DependencyObservation::Available {
                kind,
                contract_id: contract_id.clone(),
                version: version.clone(),
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
            contract_id,
            version,
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
        negative_snapshot(catalog).context().clone(),
        assessed,
        ReadinessRecoveryEventId::from_digest(raw_digest(b"TEST_CODE event")),
        refs,
    )
    .expect("TEST_CODE full candidate")
}

#[test]
fn w15_snapshot_reload_roundtrips_all_scopes_stages_and_evidence_outcomes() {
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let producer = ProducerId::try_new("p01-scheduled".to_owned()).expect("TEST_CODE producer");
    let unit = UnitId::try_new("MU-p01".to_owned()).expect("TEST_CODE unit");
    let occurrence_id = derive_occurrence_id(&OccurrenceIdentityMaterial::new(
        BusinessDate::parse("2026-09-07").expect("TEST_CODE date"),
        catalog
            .producer(&producer)
            .expect("TEST_CODE registration")
            .occurrence_family()
            .clone(),
        OccurrenceKey::try_new("p01:2026-09-07".to_owned()).expect("TEST_CODE key"),
    ));
    for scope in [
        ReadinessScope::Core,
        ReadinessScope::Producer {
            unit_id: unit.clone(),
            producer_id: producer.clone(),
        },
        ReadinessScope::Occurrence {
            unit_id: unit,
            producer_id: producer,
            occurrence_id,
        },
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
        ReadinessScope::Core,
        ReadinessStage::Running,
        false,
    );
    let bytes = snapshot.canonical_bytes();
    let mutations: [(fn(&mut serde_json::Value), &str); 6] = [
        (
            |fields| fields["captured_at"] = u64::MAX.into(),
            "captured_at",
        ),
        (
            |fields| fields["business_date"] = "TEST_CODE-SECRET-invalid-date".into(),
            "business_date",
        ),
        (|fields| fields["stage"] = "Unknown".into(), "stage"),
        (
            |fields| fields["dependency_refs"][0]["kind"] = "Unknown".into(),
            "dependency_kind",
        ),
        (
            |fields| fields["evidence_refs"][0]["kind"] = "Unknown".into(),
            "evidence_kind",
        ),
        (
            |fields| fields["evidence_refs"][0]["protected_uri"] = " TEST_CODE-SECRET ".into(),
            "protected_uri",
        ),
    ];
    for (mutate, expected_field) in mutations {
        let invalid_bytes = rewrite_fields(&bytes, mutate);
        let error =
            decode_readiness_snapshot(&catalog, &raw_digest(&invalid_bytes), &invalid_bytes)
                .expect_err("TEST_CODE invalid typed field");
        assert!(
            matches!(error, ReadinessDecodeError::InvalidField { field } if field == expected_field)
        );
        assert!(!format!("{error:?} {error}").contains("TEST_CODE-SECRET"));
    }
}
