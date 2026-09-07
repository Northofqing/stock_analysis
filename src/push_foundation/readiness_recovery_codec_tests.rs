use crate::monitor::push_job::{
    raw_digest, MachineCatalog, OccurrenceId, ProducerId, UnitId, UtcMicros,
};

use super::operational_readiness::{DependencyKind, ReadinessScope};
use super::readiness_recovery::{
    CandidateReadinessRecord, CandidateRecoveryClaim, ReadinessRecoveryError,
};
use super::readiness_recovery_codec::{decode_readiness_record, ReadinessRecordDecodeError};
use super::readiness_recovery_tests::{assessed, context};
use super::readiness_snapshot_codec::{decode_readiness_snapshot, ReadinessDecodeError};

fn recovery_pair_for(
    scope: &ReadinessScope,
    failed: DependencyKind,
) -> (CandidateReadinessRecord, CandidateReadinessRecord) {
    let (negative, evidence) = assessed(scope, Some(failed));
    let before = CandidateReadinessRecord::try_new(None, context(100), negative, evidence, vec![])
        .expect("TEST_CODE pending");
    let (ready, evidence) = assessed(scope, None);
    let claim = CandidateRecoveryClaim {
        evidence: evidence
            .iter()
            .find(|reference| reference.dependency_kind() == failed)
            .expect("TEST_CODE failed role")
            .clone(),
        observed_at: UtcMicros::try_new(150).expect("TEST_CODE source time"),
    };
    let after = CandidateReadinessRecord::try_new(
        Some(before.snapshot()),
        context(200),
        ready,
        evidence,
        vec![claim],
    )
    .expect("TEST_CODE recovered material");
    (before, after)
}

fn recovery_pair() -> (CandidateReadinessRecord, CandidateReadinessRecord) {
    recovery_pair_for(&ReadinessScope::Core, DependencyKind::Schema)
}

fn event_body(record: &CandidateReadinessRecord) -> serde_json::Value {
    serde_json::from_slice(
        record
            .event_bytes()
            .strip_prefix(b"OperationalReadinessRecoveryEvent/v1\0")
            .expect("TEST_CODE event domain"),
    )
    .expect("TEST_CODE event body")
}

fn event_bytes(body: &serde_json::Value) -> Vec<u8> {
    let mut bytes = b"OperationalReadinessRecoveryEvent/v1\0".to_vec();
    bytes.extend(serde_json::to_vec(body).expect("TEST_CODE JSON"));
    bytes
}

#[test]
fn w15_event_reload_rechecks_both_snapshot_joins_even_when_body_hash_is_recomputed() {
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let (before, after) = recovery_pair();
    let reloaded_snapshot = decode_readiness_snapshot(
        &catalog,
        after.snapshot().snapshot_id(),
        &after.snapshot().canonical_bytes(),
    )
    .expect("TEST_CODE snapshot reload");
    assert_eq!(
        decode_readiness_record(
            Some(before.snapshot()),
            &reloaded_snapshot,
            after.event_sha256(),
            &after.event_bytes()
        )
        .expect("TEST_CODE event reload"),
        after
    );
    assert_eq!(
        decode_readiness_record(
            None,
            before.snapshot(),
            before.event_sha256(),
            &before.event_bytes()
        )
        .expect("TEST_CODE initial pending reload"),
        before
    );
    assert_eq!(
        decode_readiness_record(
            Some(before.snapshot()),
            &reloaded_snapshot,
            &raw_digest(b"TEST_CODE wrong expected hash"),
            &after.event_bytes()
        ),
        Err(ReadinessRecordDecodeError::DigestMismatch)
    );

    for field in [
        "before_snapshot_id",
        "after_snapshot_id",
        "event_id",
        "after_material_sha256",
    ] {
        let mut body = event_body(&after);
        body[field] = serde_json::json!(raw_digest(b"TEST_CODE foreign join").as_str());
        let forged = event_bytes(&body);
        assert_eq!(
            decode_readiness_record(
                Some(before.snapshot()),
                &reloaded_snapshot,
                &raw_digest(&forged),
                &forged
            ),
            Err(ReadinessRecordDecodeError::InconsistentRecord),
            "{field}"
        );
    }
}

#[test]
fn w15_event_reload_roundtrips_initial_continuing_and_restored_records_in_all_scopes() {
    let unit_id = UnitId::try_new("MU-p01".to_owned()).expect("TEST_CODE unit");
    let producer_id = ProducerId::try_new("p01-scheduled".to_owned()).expect("TEST_CODE producer");
    for (scope, failed, restored_kind) in [
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
        let (before, after) = recovery_pair_for(&scope, failed);
        assert_eq!(event_body(&after)["kind"], restored_kind);
        let (negative, refs) = assessed(&scope, Some(failed));
        let pending = CandidateReadinessRecord::try_new(
            Some(before.snapshot()),
            context(180),
            negative,
            refs,
            vec![],
        )
        .expect("TEST_CODE continuing pending");
        let (ready, refs) = assessed(&scope, None);
        let initial_ready = CandidateReadinessRecord::try_new(
            None,
            context(200),
            ready.clone(),
            refs.clone(),
            vec![],
        )
        .expect("TEST_CODE initial ready");
        let continuing_ready = CandidateReadinessRecord::try_new(
            Some(after.snapshot()),
            context(250),
            ready,
            refs,
            vec![],
        )
        .expect("TEST_CODE continuing ready");
        for (previous, record, kind) in [
            (None, &before, "Pending"),
            (Some(before.snapshot()), &pending, "Pending"),
            (Some(before.snapshot()), &after, restored_kind),
            (None, &initial_ready, "ReadyObserved"),
            (Some(after.snapshot()), &continuing_ready, "ReadyObserved"),
        ] {
            assert_eq!(event_body(record)["kind"], kind);
            let decoded = decode_readiness_record(
                previous,
                record.snapshot(),
                record.event_sha256(),
                &record.event_bytes(),
            )
            .expect("TEST_CODE event reload");
            assert_eq!(&decoded, record);
            assert!(!format!("{decoded:?}").contains("TEST_CODE-SECRET"));
        }
    }
}

#[test]
fn w15_event_reload_rederives_changes_and_rejects_claim_drift_and_foreign_previous_snapshot() {
    use ReadinessRecordDecodeError::{InconsistentRecord, InvalidRecovery};
    let (before, after) = recovery_pair();
    let original = event_body(&after);
    let invalid_claim = |check| {
        InvalidRecovery(ReadinessRecoveryError::InvalidClaims {
            check,
            kind: DependencyKind::Schema,
        })
    };
    let cases = [
        (
            "/kind",
            serde_json::json!("ReadyObserved"),
            InconsistentRecord,
        ),
        (
            "/dependency_changes",
            serde_json::json!([]),
            InconsistentRecord,
        ),
        (
            "/dependency_changes/0/before_requirement/version",
            serde_json::json!("wrong-v1"),
            InconsistentRecord,
        ),
        (
            "/dependency_changes/0/after_requirement/contract_id",
            serde_json::json!("wrong-source"),
            InconsistentRecord,
        ),
        (
            "/dependency_changes/0/before_observation/status",
            serde_json::json!("Available"),
            InconsistentRecord,
        ),
        (
            "/dependency_changes/0/after_observation/evidence_sha256",
            serde_json::json!(raw_digest(b"wrong").as_str()),
            InconsistentRecord,
        ),
        (
            "/recovery_claims",
            serde_json::json!([]),
            InvalidRecovery(ReadinessRecoveryError::ExplicitRecoveryRequired),
        ),
        (
            "/recovery_claims",
            serde_json::json!([
                original["recovery_claims"][0],
                original["recovery_claims"][0]
            ]),
            invalid_claim("duplicate_dependency"),
        ),
        (
            "/recovery_claims/0/observed_at",
            serde_json::json!(99),
            invalid_claim("observation_time"),
        ),
        (
            "/recovery_claims/0/observed_at",
            serde_json::json!(201),
            invalid_claim("observation_time"),
        ),
        (
            "/recovery_claims/0/observed_at",
            serde_json::json!(151),
            InconsistentRecord,
        ),
        (
            "/recovery_claims/0/evidence/source_contract_id",
            serde_json::json!("TEST_CODE-foreign-source"),
            invalid_claim("after_evidence_mismatch"),
        ),
        (
            "/recovery_claims/0/evidence/source_contract_version",
            serde_json::json!("v2"),
            invalid_claim("after_evidence_mismatch"),
        ),
        (
            "/recovery_claims/0/evidence/protected_uri",
            serde_json::json!("vault://TEST_CODE-SECRET/other"),
            invalid_claim("after_evidence_mismatch"),
        ),
        (
            "/recovery_claims/0/evidence/sha256",
            serde_json::json!(raw_digest(b"unavailable").as_str()),
            invalid_claim("after_evidence_mismatch"),
        ),
        (
            "/recovery_claims/0/evidence/kind",
            serde_json::json!("DataAcquisitionAudit"),
            invalid_claim("after_evidence_mismatch"),
        ),
    ];
    for (path, value, expected) in cases {
        let mut body = original.clone();
        *body.pointer_mut(path).expect("TEST_CODE mutation target") = value;
        let bytes = event_bytes(&body);
        let result = decode_readiness_record(
            Some(before.snapshot()),
            after.snapshot(),
            &raw_digest(&bytes),
            &bytes,
        );
        assert_eq!(result, Err(expected), "{path}");
        assert!(!format!("{expected}").contains("TEST_CODE-SECRET"));
        assert!(!format!("{expected:?}").contains("TEST_CODE-SECRET"));
    }
    let (negative, refs) = assessed(&ReadinessScope::Core, Some(DependencyKind::Schema));
    let foreign = CandidateReadinessRecord::try_new(None, context(99), negative, refs, vec![])
        .expect("TEST_CODE different previous snapshot");
    assert_eq!(
        decode_readiness_record(
            Some(foreign.snapshot()),
            after.snapshot(),
            after.event_sha256(),
            &after.event_bytes()
        ),
        Err(InconsistentRecord)
    );
    assert_eq!(
        decode_readiness_record(
            None,
            after.snapshot(),
            after.event_sha256(),
            &after.event_bytes()
        ),
        Err(InvalidRecovery(ReadinessRecoveryError::UnexpectedClaims))
    );
}

#[test]
fn w15_event_reload_rejects_invalid_typed_claims_and_noncanonical_bytes_without_leakage() {
    use ReadinessRecordDecodeError::{InconsistentRecord, InvalidEvidence, InvalidField};
    let (before, after) = recovery_pair();
    let original = event_body(&after);
    let evidence_error = |field| InvalidEvidence(ReadinessDecodeError::InvalidField { field });
    for (path, value, expected) in [
        (
            "/schema_version",
            serde_json::json!(2),
            ReadinessRecordDecodeError::UnsupportedSchemaVersion,
        ),
        (
            "/schema_version",
            serde_json::json!(1.0),
            InvalidField {
                field: "schema_version",
            },
        ),
        (
            "/recovery_claims",
            serde_json::Value::Null,
            InvalidField {
                field: "recovery_claims",
            },
        ),
        (
            "/recovery_claims/0/observed_at",
            serde_json::json!(-1),
            InvalidField {
                field: "observed_at",
            },
        ),
        (
            "/recovery_claims/0/observed_at",
            serde_json::json!(18446744073709551615_u64),
            InvalidField {
                field: "observed_at",
            },
        ),
        (
            "/recovery_claims/0/observed_at",
            serde_json::json!(150.5),
            InvalidField {
                field: "observed_at",
            },
        ),
        (
            "/recovery_claims/0/evidence/dependency_kind",
            serde_json::json!("TEST_CODE-SECRET-role"),
            evidence_error("dependency_kind"),
        ),
        (
            "/recovery_claims/0/evidence/kind",
            serde_json::json!("TEST_CODE-SECRET-kind"),
            evidence_error("evidence_kind"),
        ),
        (
            "/recovery_claims/0/evidence/sha256",
            serde_json::json!("TEST_CODE-SECRET-digest"),
            evidence_error("sha256"),
        ),
        (
            "/recovery_claims/0/evidence/source_contract_id",
            serde_json::json!(""),
            evidence_error("source_contract_id"),
        ),
        (
            "/recovery_claims/0/evidence/source_contract_version",
            serde_json::json!(""),
            evidence_error("source_contract_version"),
        ),
        (
            "/recovery_claims/0/evidence/protected_uri",
            serde_json::json!(""),
            evidence_error("protected_uri"),
        ),
    ] {
        let mut body = original.clone();
        *body.pointer_mut(path).expect("TEST_CODE mutation target") = value;
        let bytes = event_bytes(&body);
        let result = decode_readiness_record(
            Some(before.snapshot()),
            after.snapshot(),
            &raw_digest(&bytes),
            &bytes,
        );
        assert_eq!(result, Err(expected), "{path}");
        assert!(!format!("{expected}").contains("TEST_CODE-SECRET"));
        assert!(!format!("{expected:?}").contains("TEST_CODE-SECRET"));
    }
    let mut extra = original.clone();
    extra["unexpected"] = serde_json::json!("TEST_CODE-SECRET-extra");
    let mut nested_extra = original.clone();
    nested_extra["recovery_claims"][0]["evidence"]["unexpected"] =
        serde_json::json!("TEST_CODE-SECRET-extra");
    let canonical_json = serde_json::to_string(&original).expect("TEST_CODE JSON");
    let duplicate = format!(
        "OperationalReadinessRecoveryEvent/v1\0{{\"kind\":\"TEST_CODE-SECRET-duplicate\",{}",
        &canonical_json[1..]
    );
    let pretty = format!(
        "OperationalReadinessRecoveryEvent/v1\0{}",
        serde_json::to_string_pretty(&original).expect("TEST_CODE pretty JSON")
    );
    for (bytes, expected) in [
        (event_bytes(&extra), InconsistentRecord),
        (event_bytes(&nested_extra), InconsistentRecord),
        (duplicate.into_bytes(), InconsistentRecord),
        (pretty.into_bytes(), InconsistentRecord),
        (
            b"WrongEvent/v1\0TEST_CODE-SECRET-domain".to_vec(),
            ReadinessRecordDecodeError::InvalidDomain,
        ),
        (
            b"OperationalReadinessRecoveryEvent/v1\0{TEST_CODE-SECRET-invalid-json".to_vec(),
            ReadinessRecordDecodeError::InvalidJson,
        ),
    ] {
        assert_eq!(
            decode_readiness_record(
                Some(before.snapshot()),
                after.snapshot(),
                &raw_digest(&bytes),
                &bytes
            ),
            Err(expected)
        );
        assert!(!format!("{expected}").contains("TEST_CODE-SECRET"));
        assert!(!format!("{expected:?}").contains("TEST_CODE-SECRET"));
    }
}
