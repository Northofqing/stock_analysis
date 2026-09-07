use std::collections::BTreeMap;
use std::path::Path;

use rusqlite::{params, Connection};

use crate::monitor::push_job::{
    derive_intent_id, derive_occurrence_id, AudienceId, BusinessDate, CompletionOwnerId, IntentId,
    IntentIdentityMaterial, MachineCatalog, Namespace, OccurrenceFamily, OccurrenceId,
    OccurrenceIdentityMaterial, OccurrenceKey, ProducerId, ReasonCode, RunId, Sha256Digest,
    SourceContractId, SubjectId, UnitId, UtcMicros,
};

use super::readiness_occurrence::{
    read_stored_occurrence, StoredOccurrenceExpectation, StoredOccurrenceReadError,
};
use super::{BusinessIntentStore, InitialDecisionKind, InitialIntentDraft, InitialIntentIdentity};
use super::{IntentState, IntentTransitionCommand, LeaseAction, LeaseOwnerId, TransitionActor};
use crate::push_foundation::FoundationSchemaMigration;

fn directory_bytes(directory: &Path) -> BTreeMap<String, Vec<u8>> {
    std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (
                entry.file_name().to_string_lossy().into_owned(),
                std::fs::read(entry.path()).unwrap(),
            )
        })
        .collect()
}

fn fixture(database: &Path, kind: InitialDecisionKind) -> StoredOccurrenceExpectation {
    fixture_with_identity(database, kind, |_| {})
}

fn fixture_with_identity(
    database: &Path,
    kind: InitialDecisionKind,
    alter: impl FnOnce(&mut InitialIntentIdentity),
) -> StoredOccurrenceExpectation {
    FoundationSchemaMigration::bundled()
        .unwrap()
        .apply_to(database)
        .unwrap();
    let catalog = MachineCatalog::bundled().unwrap();
    let producer_id = ProducerId::try_new("market-preopen-probe".to_owned()).unwrap();
    let producer = catalog.producer(&producer_id).unwrap();
    let namespace = Namespace::test(RunId::try_new("w15-known-occurrence".to_owned()).unwrap());
    let business_date = BusinessDate::parse("2026-09-07").unwrap();
    let occurrence = OccurrenceIdentityMaterial::new(
        business_date.clone(),
        producer.occurrence_family().clone(),
        OccurrenceKey::try_new("preopen-session".to_owned()).unwrap(),
    );
    let source_contract_id = SourceContractId::try_new("preopen-source".to_owned()).unwrap();
    let mut identity = InitialIntentIdentity::new(
        namespace.clone(),
        producer.unit_id().clone(),
        occurrence.clone(),
        producer.completion_owner().clone(),
        source_contract_id.clone(),
        SubjectId::entity("SECRET-account".to_owned()).unwrap(),
        AudienceId::try_new("portfolio-owner".to_owned()).unwrap(),
    );
    alter(&mut identity);
    let actual_occurrence_id = derive_occurrence_id(&identity.occurrence);
    let draft = match kind {
        InitialDecisionKind::NoData => InitialIntentDraft::no_data(
            identity,
            Sha256Digest::from_bytes([1; 32]),
            Sha256Digest::from_bytes([2; 32]),
            Sha256Digest::from_bytes([3; 32]),
            UtcMicros::try_new(100).unwrap(),
        ),
        InitialDecisionKind::Disabled => InitialIntentDraft::disabled(
            identity,
            Sha256Digest::from_bytes([1; 32]),
            Sha256Digest::from_bytes([2; 32]),
            Sha256Digest::from_bytes([3; 32]),
            UtcMicros::try_new(100).unwrap(),
        ),
        InitialDecisionKind::Ready => InitialIntentDraft::ready_for_recovery_test(
            identity,
            b"TEST_CODE protected prepared bytes".to_vec(),
            b"SECRET-account rendered bytes".to_vec(),
            Sha256Digest::from_bytes([2; 32]),
            Sha256Digest::from_bytes([3; 32]),
            UtcMicros::try_new(100).unwrap(),
        )
        .unwrap(),
    };
    let expected = StoredOccurrenceExpectation {
        intent_id: draft.intent_id().clone(),
        namespace,
        business_date,
        unit_id: producer.unit_id().clone(),
        producer_id,
        occurrence_id: actual_occurrence_id,
        source_contract_id,
    };
    let mut writer = BusinessIntentStore::open(database).unwrap();
    writer.record_initial(&draft).unwrap();
    expected
}

#[test]
fn w15_known_occurrence_reopens_no_data_from_a_verified_readonly_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().canonicalize().unwrap().join("business.sqlite3");
    let expected = fixture(&database, InitialDecisionKind::NoData);
    let before = directory_bytes(root.path());

    let actual = read_stored_occurrence(&database, &expected).unwrap();

    assert_eq!(actual.binding(), &expected);
    assert_eq!(
        actual.catalog_sha256().as_str(),
        "0aa6a2fd87ee9c235073cad3beef44229437f3fe62987b0db510ad36a93aace3"
    );
    assert_eq!(actual.decision_kind(), InitialDecisionKind::NoData);
    assert_eq!(actual.version(), 0);
    assert_eq!(actual.transition_head_sha256(), None);
    assert_eq!(
        actual.source_contract_sha256(),
        &Sha256Digest::from_bytes([3; 32])
    );
    assert!(!format!("{actual:?}").contains("SECRET-account"));
    assert_eq!(directory_bytes(root.path()), before);
}

#[test]
fn w15_known_occurrence_includes_ready_and_disabled_without_exposing_payloads() {
    for kind in [InitialDecisionKind::Ready, InitialDecisionKind::Disabled] {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().canonicalize().unwrap().join("business.sqlite3");
        let expected = fixture(&database, kind);
        let before = directory_bytes(root.path());
        let actual = read_stored_occurrence(&database, &expected).unwrap();
        assert_eq!(actual.binding(), &expected);
        assert_eq!(actual.decision_kind(), kind);
        assert_eq!(actual.version(), 0);
        assert_eq!(actual.transition_head_sha256(), None);
        assert!(!format!("{actual:?}").contains("SECRET-account"));
        assert!(!format!("{actual:?}").contains("prepared bytes"));
        assert_eq!(directory_bytes(root.path()), before);
    }
}

#[test]
fn w15_known_occurrence_rejects_each_identity_drift_and_unpersisted_identity() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().canonicalize().unwrap().join("business.sqlite3");
    let expected = fixture(&database, InitialDecisionKind::NoData);
    let before = directory_bytes(root.path());
    for field in [
        "intent",
        "namespace",
        "date",
        "unit",
        "producer",
        "occurrence",
        "source",
    ] {
        let mut foreign = expected.clone();
        let rejected = match field {
            "intent" => {
                foreign.intent_id = IntentId::from_digest(&Sha256Digest::from_bytes([7; 32]));
                StoredOccurrenceReadError::Missing
            }
            "namespace" => {
                foreign.namespace = Namespace::Production;
                StoredOccurrenceReadError::IdentityMismatch
            }
            "date" => {
                foreign.business_date = BusinessDate::parse("2026-09-08").unwrap();
                StoredOccurrenceReadError::IdentityMismatch
            }
            "unit" => {
                foreign.unit_id = UnitId::try_new("MU-chain-preopen".to_owned()).unwrap();
                StoredOccurrenceReadError::IdentityMismatch
            }
            "producer" => {
                foreign.producer_id =
                    ProducerId::try_new("unregistered-producer".to_owned()).unwrap();
                StoredOccurrenceReadError::CatalogRejected
            }
            "occurrence" => {
                foreign.occurrence_id =
                    OccurrenceId::from_digest(&Sha256Digest::from_bytes([8; 32]));
                StoredOccurrenceReadError::IdentityMismatch
            }
            "source" => {
                foreign.source_contract_id =
                    SourceContractId::try_new("foreign-source".to_owned()).unwrap();
                StoredOccurrenceReadError::IdentityMismatch
            }
            _ => unreachable!("TEST_CODE fixed field matrix"),
        };
        assert_eq!(
            read_stored_occurrence(&database, &foreign),
            Err(rejected),
            "{field}"
        );
        assert!(!format!("{rejected:?}: {rejected}").contains("SECRET"));
    }
    assert_eq!(
        read_stored_occurrence(&database, &expected)
            .unwrap()
            .binding(),
        &expected
    );
    assert_eq!(directory_bytes(root.path()), before);
}

#[test]
fn w15_known_occurrence_rejects_valid_persisted_but_foreign_catalog_relationships() {
    for wrong_family in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().canonicalize().unwrap().join("business.sqlite3");
        let expected = fixture_with_identity(&database, InitialDecisionKind::NoData, |identity| {
            if wrong_family {
                identity.occurrence = OccurrenceIdentityMaterial::new(
                    BusinessDate::parse("2026-09-07").unwrap(),
                    OccurrenceFamily::try_new("foreign-family".to_owned()).unwrap(),
                    OccurrenceKey::try_new("preopen-session".to_owned()).unwrap(),
                );
            } else {
                identity.completion_owner =
                    CompletionOwnerId::try_new("foreign-owner".to_owned()).unwrap();
            }
        });
        {
            let store = BusinessIntentStore::open(&database).unwrap();
            assert!(store.inspect(&expected.intent_id).unwrap().is_some());
        }
        let before = directory_bytes(root.path());
        assert_eq!(
            read_stored_occurrence(&database, &expected),
            Err(StoredOccurrenceReadError::IdentityMismatch)
        );
        assert_eq!(directory_bytes(root.path()), before);
    }
}

#[test]
fn w15_known_occurrence_does_not_infer_persistence_from_a_derived_intent_id() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().canonicalize().unwrap().join("business.sqlite3");
    let expected = fixture(&database, InitialDecisionKind::NoData);
    let catalog = MachineCatalog::bundled().unwrap();
    let registration = catalog.producer(&expected.producer_id).unwrap();
    let mut unpersisted = expected.clone();
    unpersisted.intent_id = derive_intent_id(&IntentIdentityMaterial::new(
        expected.namespace.clone(),
        expected.unit_id.clone(),
        registration.completion_owner().clone(),
        expected.source_contract_id.clone(),
        expected.occurrence_id.clone(),
        SubjectId::entity("never-persisted-subject".to_owned()).unwrap(),
        AudienceId::try_new("portfolio-owner".to_owned()).unwrap(),
    ));
    assert_ne!(unpersisted.intent_id, expected.intent_id);
    let before = directory_bytes(root.path());
    assert_eq!(
        read_stored_occurrence(&database, &unpersisted),
        Err(StoredOccurrenceReadError::Missing)
    );
    assert_eq!(directory_bytes(root.path()), before);
}

#[test]
fn w15_known_occurrence_verifies_real_transition_chain_and_rejects_middle_damage() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().canonicalize().unwrap().join("business.sqlite3");
    let expected = fixture(&database, InitialDecisionKind::Ready);
    let tail = {
        let mut writer = BusinessIntentStore::open(&database).unwrap();
        let transitions = [
            (
                IntentState::PendingDispatch,
                IntentState::AwaitingAuthority,
                ReasonCode::IntentDispatchClaimed,
            ),
            (
                IntentState::AwaitingAuthority,
                IntentState::ResolutionRequired,
                ReasonCode::TransportUncertain,
            ),
            (
                IntentState::ResolutionRequired,
                IntentState::ResolutionRequired,
                ReasonCode::IntentLeaseHeld,
            ),
        ];
        let mut tail = None;
        for (version, (from, to, reason)) in transitions.into_iter().enumerate() {
            let lease = if version == 0 {
                LeaseAction::Acquire {
                    owner: LeaseOwnerId::try_new("TEST_CODE-owner".to_owned()).unwrap(),
                    until: UtcMicros::try_new(1000).unwrap(),
                }
            } else {
                LeaseAction::Preserve
            };
            let command = IntentTransitionCommand::try_new(
                expected.intent_id.clone(),
                from,
                to,
                version as u64,
                TransitionActor::try_new("TEST_CODE-reader-fixture".to_owned()).unwrap(),
                reason,
                UtcMicros::try_new(200 + version as i64 * 100).unwrap(),
                lease,
            )
            .unwrap();
            tail = Some(
                writer
                    .apply_nonterminal_transition(&command)
                    .unwrap()
                    .receipt()
                    .unwrap()
                    .canonical_sha256()
                    .clone(),
            );
        }
        tail.unwrap()
    };
    let before = directory_bytes(root.path());
    let actual = read_stored_occurrence(&database, &expected).unwrap();
    assert_eq!(actual.version(), 3);
    assert_eq!(actual.transition_head_sha256(), Some(&tail));
    assert_eq!(directory_bytes(root.path()), before);

    {
        let connection = Connection::open(&database).unwrap();
        let original: String = connection
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name='push_intent_transitions_update'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        connection
            .execute_batch("DROP TRIGGER push_intent_transitions_update;")
            .unwrap();
        assert_eq!(connection.execute(
            "UPDATE push_intent_transitions SET previous_sha256=? WHERE intent_id=? AND result_version=2",
            params!["e".repeat(64), expected.intent_id.as_str()],
        ).unwrap(), 1);
        connection.execute_batch(&original).unwrap();
    }
    let corrupted = directory_bytes(root.path());
    assert_eq!(
        read_stored_occurrence(&database, &expected),
        Err(StoredOccurrenceReadError::InvalidFacts)
    );
    assert_eq!(directory_bytes(root.path()), corrupted);
}

#[test]
fn w15_known_occurrence_rejects_self_consistent_but_nonbundled_schema() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().canonicalize().unwrap().join("business.sqlite3");
    let expected = fixture(&database, InitialDecisionKind::NoData);
    {
        let connection = Connection::open(&database).unwrap();
        let registry_guard: String = connection
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name='push_foundation_objects_update'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        connection.execute_batch(
            "DROP TRIGGER push_foundation_objects_update;
             DROP TRIGGER push_intents_delete;
             CREATE TRIGGER push_intents_delete BEFORE DELETE ON push_intents BEGIN SELECT 1; END;
             UPDATE push_foundation_objects SET definition=(SELECT sql FROM sqlite_master WHERE name='push_intents_delete') WHERE name='push_intents_delete';",
        ).unwrap();
        connection.execute_batch(&registry_guard).unwrap();
        let migration = FoundationSchemaMigration::bundled().unwrap();
        // Prove the adversarial fixture still passes the older self-consistency check.
        crate::push_foundation::migration::attest_connection(&connection, migration.ddl_sha256())
            .unwrap();
    }
    let before = directory_bytes(root.path());
    assert_eq!(
        read_stored_occurrence(&database, &expected),
        Err(StoredOccurrenceReadError::SchemaRejected)
    );
    assert_eq!(directory_bytes(root.path()), before);
}

#[test]
fn w15_known_occurrence_rejects_missing_and_wal_sources_without_repair() {
    let root = tempfile::tempdir().unwrap();
    let canonical_root = root.path().canonicalize().unwrap();
    let database = canonical_root.join("business.sqlite3");
    let expected = fixture(&database, InitialDecisionKind::NoData);
    let missing = canonical_root.join("missing.sqlite3");
    let before = directory_bytes(root.path());
    assert_eq!(
        read_stored_occurrence(&missing, &expected),
        Err(StoredOccurrenceReadError::ReadOnlySourceRejected)
    );
    assert!(!missing.exists());
    assert_eq!(directory_bytes(root.path()), before);
    {
        let connection = Connection::open(&database).unwrap();
        let mode: String = connection
            .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
    }
    let wal_bytes = directory_bytes(root.path());
    assert_eq!(
        read_stored_occurrence(&database, &expected),
        Err(StoredOccurrenceReadError::ReadOnlySourceRejected)
    );
    assert_eq!(directory_bytes(root.path()), wal_bytes);
}
