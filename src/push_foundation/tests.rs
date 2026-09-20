use std::path::Path;

use rusqlite::Connection;

use crate::monitor::push_job::{
    AudienceId, BusinessDate, CompletionOwnerId, Namespace, OccurrenceFamily,
    OccurrenceIdentityMaterial, OccurrenceKey, Sha256Digest, SourceContractId, SubjectId, UnitId,
    UtcMicros,
};

use super::{
    BusinessIntentStore, FoundationMigrationError, FoundationSchemaMigration, InitialCommitFault,
    InitialDecisionKind, InitialIntentDraft, InitialIntentIdentity, InitialIntentOutcome,
    IntentStoreError, IntentTransitionCommand, LeaseAction, LeaseOwnerId, TransitionActor,
    TransitionFault, TransitionOutcome,
};

#[test]
fn w07_bundled_migration_has_exact_authority_metadata() {
    let migration = FoundationSchemaMigration::bundled().expect("bundled migration is valid");

    assert_eq!(migration.schema_version(), 1);
    assert_eq!(migration.schema_description(), "push-foundation-v1");
    assert_eq!(
        migration.schema_signature(),
        "dd5f49a1f4e02ee1d585793cc2eff9c8b98b087b2ffd267f40c873c83960ecdd"
    );
    assert_eq!(
        migration.ddl_sha256().as_str(),
        "4bac8e58caa2f5d2362137b5e96dd087649044f45484a1284dbd7e1fd7baa953"
    );
    assert_eq!(migration.managed_object_count(), 25);
    assert_eq!(
        migration.ddl_bytes().split(|byte| *byte == b'\n').next(),
        Some(&b".bail on"[..])
    );
}

#[test]
fn w07_migration_rejects_ambiguous_or_unsafe_database_paths() {
    let migration = FoundationSchemaMigration::bundled().unwrap();
    assert!(matches!(
        migration.apply_to(Path::new("relative.sqlite3")),
        Err(FoundationMigrationError::DatabasePathNotAbsolute)
    ));

    let root = tempfile::tempdir().unwrap();
    assert!(matches!(
        migration.apply_to(&root.path().join("missing-parent/database.sqlite3")),
        Err(FoundationMigrationError::DatabaseParentMissing)
    ));
    assert!(matches!(
        migration.apply_to(root.path()),
        Err(FoundationMigrationError::DatabaseTargetNotRegular)
    ));

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        let real = root.path().join("real.sqlite3");
        Connection::open(&real).unwrap();
        let link = root.path().join("linked.sqlite3");
        symlink(&real, &link).unwrap();
        assert!(matches!(
            migration.apply_to(&link),
            Err(FoundationMigrationError::DatabaseTargetSymlink)
        ));
    }
}

#[test]
fn w07_fresh_apply_returns_attested_receipt() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("business.sqlite3");
    let migration = FoundationSchemaMigration::bundled().unwrap();

    let receipt = migration.apply_to(&database).expect("fresh apply succeeds");
    assert_eq!(receipt.schema_version(), 1);
    assert_eq!(receipt.schema_signature(), migration.schema_signature());
    assert_eq!(receipt.ddl_sha256(), migration.ddl_sha256());
    assert_eq!(receipt.managed_object_count(), 25);

    let connection =
        Connection::open_with_flags(&database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let header: (i64, String, String) = connection
        .query_row(
            "SELECT version,description,schema_signature FROM push_foundation_schema",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        header,
        (
            1,
            "push-foundation-v1".to_owned(),
            migration.schema_signature().to_owned()
        )
    );
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM push_foundation_objects", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        25
    );
}

#[test]
fn w07_fresh_apply_preserves_unrelated_legacy_schema_and_bytes() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("business.sqlite3");
    let connection = Connection::open(&database).unwrap();
    connection
        .execute_batch("CREATE TABLE push_legacy(id INTEGER PRIMARY KEY,payload BLOB NOT NULL);")
        .unwrap();
    let legacy_bytes = b"legacy\0exact\nbytes";
    connection
        .execute(
            "INSERT INTO push_legacy(id,payload) VALUES(1,?)",
            [legacy_bytes.as_slice()],
        )
        .unwrap();
    let definition: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='push_legacy'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    drop(connection);

    FoundationSchemaMigration::bundled()
        .unwrap()
        .apply_to(&database)
        .unwrap();

    let connection = Connection::open(&database).unwrap();
    let actual_definition: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='push_legacy'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let actual_bytes: Vec<u8> = connection
        .query_row("SELECT payload FROM push_legacy WHERE id=1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(actual_definition.as_bytes(), definition.as_bytes());
    assert_eq!(actual_bytes, legacy_bytes);
}

fn w07_registered_definitions(connection: &Connection) -> Vec<(String, String, Vec<u8>)> {
    let mut statement = connection
        .prepare(
            "SELECT name,object_type,CAST(definition AS BLOB) \
             FROM push_foundation_objects ORDER BY name",
        )
        .unwrap();
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

fn w07_live_managed_definitions(connection: &Connection) -> Vec<(String, String, Vec<u8>)> {
    let mut statement = connection
        .prepare(
            "SELECT s.name,s.type,CAST(s.sql AS BLOB) FROM sqlite_master s \
             JOIN push_foundation_objects r ON r.name=s.name AND r.object_type=s.type \
             ORDER BY s.name",
        )
        .unwrap();
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

fn w07_insert_pending_ready_intent(connection: &Connection) -> (Vec<u8>, Vec<u8>) {
    let prepared = b"PreparedPush/v1\0exact\nbytes".to_vec();
    let rendered = b"message  \nline two\0".to_vec();
    connection
        .execute(
            "INSERT INTO push_intents( \
                intent_id,job_decision_kind,namespace,unit_id,occurrence_family,occurrence_key, \
                completion_owner,source_contract_id,subject,audience,durable_decision_id, \
                business_date,prepared_push_bytes,rendered_bytes,payload_sha256,rendered_sha256, \
                evidence_sha256,template_sha256,source_contract_sha256,state,reason,created_at,updated_at \
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
            rusqlite::params![
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "Ready",
                "test",
                "MU-p01",
                "p01:{business_date}",
                "2026-09-07",
                "monitor_loop::P01_LAST",
                "p01-source-v1",
                "GLOBAL",
                "primary",
                "decision-p01",
                "2026-09-07",
                prepared,
                rendered,
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
                "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
                "PendingDispatch",
                "intent.created",
                1_i64,
                1_i64,
            ],
        )
        .unwrap();
    (prepared, rendered)
}

#[test]
fn w07_reapply_preserves_managed_definitions_and_nonterminal_exact_bytes() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("business.sqlite3");
    let migration = FoundationSchemaMigration::bundled().unwrap();
    migration.apply_to(&database).unwrap();

    let connection = Connection::open(&database).unwrap();
    let (prepared, rendered) = w07_insert_pending_ready_intent(&connection);
    let registered_before = w07_registered_definitions(&connection);
    let live_before = w07_live_managed_definitions(&connection);
    drop(connection);

    migration.apply_to(&database).unwrap();

    let connection = Connection::open(&database).unwrap();
    assert_eq!(w07_registered_definitions(&connection), registered_before);
    assert_eq!(w07_live_managed_definitions(&connection), live_before);
    let snapshot: (Vec<u8>, Vec<u8>, String, i64, i64) = connection
        .query_row(
            "SELECT prepared_push_bytes,rendered_bytes,state,version,lease_generation \
             FROM push_intents WHERE intent_id=?",
            ["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        snapshot,
        (prepared, rendered, "PendingDispatch".to_owned(), 0, 0)
    );
}

#[test]
fn w07_wrong_version_is_rejected_without_rewriting_original_database() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("migration-secret.sqlite3");
    let connection = Connection::open(&database).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE push_foundation_schema(version INTEGER,description TEXT,schema_signature TEXT); \
             INSERT INTO push_foundation_schema VALUES(2,'legacy-foundation','ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff'); \
             CREATE TABLE push_legacy(id INTEGER PRIMARY KEY,payload BLOB NOT NULL);",
        )
        .unwrap();
    let legacy = b"original\0legacy";
    connection
        .execute("INSERT INTO push_legacy VALUES(1,?)", [legacy.as_slice()])
        .unwrap();
    let original_schema: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='push_foundation_schema'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    drop(connection);

    let error = FoundationSchemaMigration::bundled()
        .unwrap()
        .apply_to(&database)
        .unwrap_err();
    match &error {
        FoundationMigrationError::MigrationRejected {
            exit_code,
            stderr_sha256,
        } => {
            assert_ne!(*exit_code, Some(0));
            assert_eq!(stderr_sha256.as_str().len(), 64);
        }
        other => panic!("expected typed migration rejection, got {other:?}"),
    }
    let rendered_error = format!("{error:?}");
    assert!(!rendered_error.contains("migration-secret"));
    assert!(!rendered_error.contains("legacy-foundation"));
    assert!(!rendered_error.contains("original"));

    let connection = Connection::open(&database).unwrap();
    let actual_schema: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='push_foundation_schema'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let header: (i64, String) = connection
        .query_row(
            "SELECT version,description FROM push_foundation_schema",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let actual_legacy: Vec<u8> = connection
        .query_row("SELECT payload FROM push_legacy", [], |row| row.get(0))
        .unwrap();
    let foundation_objects: i64 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name='push_foundation_objects'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(actual_schema.as_bytes(), original_schema.as_bytes());
    assert_eq!(header, (2, "legacy-foundation".to_owned()));
    assert_eq!(actual_legacy, legacy);
    assert_eq!(foundation_objects, 0);
}

#[test]
fn w07_partial_managed_schema_is_rejected_without_silent_completion() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("business.sqlite3");
    let connection = Connection::open(&database).unwrap();
    connection
        .execute_batch("CREATE TABLE push_intents(legacy_payload BLOB NOT NULL);")
        .unwrap();
    let payload = b"partial\0intent";
    connection
        .execute("INSERT INTO push_intents VALUES(?)", [payload.as_slice()])
        .unwrap();
    let definition: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='push_intents'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    drop(connection);

    assert!(matches!(
        FoundationSchemaMigration::bundled()
            .unwrap()
            .apply_to(&database),
        Err(FoundationMigrationError::MigrationRejected { .. })
    ));

    let connection = Connection::open(&database).unwrap();
    let actual_definition: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='push_intents'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let actual_payload: Vec<u8> = connection
        .query_row("SELECT legacy_payload FROM push_intents", [], |row| {
            row.get(0)
        })
        .unwrap();
    let metadata_count: i64 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name IN ('push_foundation_schema','push_foundation_objects')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(actual_definition.as_bytes(), definition.as_bytes());
    assert_eq!(actual_payload, payload);
    assert_eq!(metadata_count, 0);
}

#[test]
fn w07_missing_or_extra_managed_object_is_rejected_without_repair() {
    for mutation in ["missing", "extra"] {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("business.sqlite3");
        let migration = FoundationSchemaMigration::bundled().unwrap();
        migration.apply_to(&database).unwrap();
        let connection = Connection::open(&database).unwrap();
        let (prepared, rendered) = w07_insert_pending_ready_intent(&connection);
        match mutation {
            "missing" => connection
                .execute_batch("DROP INDEX push_intents_recovery;")
                .unwrap(),
            "extra" => connection
                .execute_batch(
                    "CREATE TRIGGER push_intents_rogue AFTER UPDATE ON push_intents BEGIN SELECT 1; END;",
                )
                .unwrap(),
            _ => unreachable!(),
        }
        drop(connection);

        assert!(matches!(
            migration.apply_to(&database),
            Err(FoundationMigrationError::MigrationRejected { .. })
        ));

        let connection = Connection::open(&database).unwrap();
        let object_count: i64 = connection
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name=?",
                [if mutation == "missing" {
                    "push_intents_recovery"
                } else {
                    "push_intents_rogue"
                }],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(object_count, if mutation == "missing" { 0 } else { 1 });
        let actual: (Vec<u8>, Vec<u8>, String, i64) = connection
            .query_row(
                "SELECT prepared_push_bytes,rendered_bytes,state,version FROM push_intents",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            actual,
            (prepared, rendered, "PendingDispatch".to_owned(), 0)
        );
    }
}

#[cfg(unix)]
#[test]
fn w07_migration_rejects_symbolic_link_parent_directory() {
    use std::os::unix::fs::symlink;

    let outer = tempfile::tempdir().unwrap();
    let real_parent = tempfile::tempdir().unwrap();
    let linked_parent = outer.path().join("linked-parent");
    symlink(real_parent.path(), &linked_parent).unwrap();

    assert!(matches!(
        FoundationSchemaMigration::bundled()
            .unwrap()
            .apply_to(&linked_parent.join("business.sqlite3")),
        Err(FoundationMigrationError::DatabaseParentSymlink)
    ));
    assert!(!real_parent.path().join("business.sqlite3").exists());
}

fn w08_digest(byte: char) -> Sha256Digest {
    Sha256Digest::parse("w08 fixture", &byte.to_string().repeat(64)).unwrap()
}

fn w08_identity(subject: &str) -> InitialIntentIdentity {
    InitialIntentIdentity::new(
        Namespace::Production,
        UnitId::try_new("MU-auction".to_owned()).unwrap(),
        OccurrenceIdentityMaterial::new(
            BusinessDate::parse("2026-09-07").unwrap(),
            OccurrenceFamily::try_new("auction-session".to_owned()).unwrap(),
            OccurrenceKey::try_new("main".to_owned()).unwrap(),
        ),
        CompletionOwnerId::try_new("owner-auction".to_owned()).unwrap(),
        SourceContractId::try_new("auction-source".to_owned()).unwrap(),
        SubjectId::entity(subject.to_owned()).unwrap(),
        AudienceId::try_new("portfolio-owner".to_owned()).unwrap(),
    )
}

fn w08_database() -> (tempfile::TempDir, std::path::PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("business.sqlite3");
    FoundationSchemaMigration::bundled()
        .unwrap()
        .apply_to(&database)
        .unwrap();
    (root, database)
}

#[test]
fn w08_ready_outbox_persists_exact_snapshot_render_and_computed_hashes() {
    let (_root, database) = w08_database();
    let prepared = crate::monitor::push_job::w08_prepared_push_fixture();
    let draft = InitialIntentDraft::ready(
        w08_identity("000001.SZ"),
        &prepared,
        w08_digest('e'),
        w08_digest('f'),
        UtcMicros::try_new(1_788_743_100_000_000).unwrap(),
    )
    .unwrap();
    let expected_snapshot = prepared.canonical_snapshot_bytes();
    let expected_render = prepared.replay_rendered_bytes().to_vec();
    let mut store = BusinessIntentStore::open(&database).unwrap();

    let outcome = store.record_initial(&draft).unwrap();
    let snapshot = match outcome {
        InitialIntentOutcome::Inserted(snapshot) => snapshot,
        other => panic!("expected inserted outbox, got {other:?}"),
    };

    assert_eq!(snapshot.decision_kind(), InitialDecisionKind::Ready);
    assert_eq!(snapshot.state().as_str(), "PendingDispatch");
    assert_eq!(snapshot.reason().as_str(), "intent.created");
    assert_eq!(snapshot.version(), 0);
    assert_eq!(snapshot.lease_generation(), 0);
    assert_eq!(
        snapshot.prepared_push_bytes(),
        Some(expected_snapshot.as_bytes())
    );
    assert_eq!(snapshot.rendered_bytes(), Some(expected_render.as_slice()));
    assert_eq!(snapshot.payload_sha256(), Some(expected_snapshot.sha256()));
    assert_eq!(snapshot.rendered_sha256(), Some(prepared.rendered_sha256()));
    assert_eq!(snapshot.namespace(), "Production");
    assert_eq!(snapshot.subject(), "Entity:000001.SZ");
}

#[test]
fn w08_non_sending_initial_rows_keep_the_entire_payload_group_null() {
    let (_root, database) = w08_database();
    let at = UtcMicros::try_new(1_788_743_100_000_000).unwrap();
    let no_data = InitialIntentDraft::no_data(
        w08_identity("000002.SZ"),
        w08_digest('a'),
        w08_digest('e'),
        w08_digest('f'),
        at,
    );
    let disabled = InitialIntentDraft::disabled(
        w08_identity("000003.SZ"),
        w08_digest('b'),
        w08_digest('e'),
        w08_digest('f'),
        at,
    );
    let mut store = BusinessIntentStore::open(&database).unwrap();

    for (draft, kind, state, reason) in [
        (
            no_data,
            InitialDecisionKind::NoData,
            "NoData",
            "intent.no_data",
        ),
        (
            disabled,
            InitialDecisionKind::Disabled,
            "Disabled",
            "policy.disabled",
        ),
    ] {
        let snapshot = match store.record_initial(&draft).unwrap() {
            InitialIntentOutcome::Inserted(snapshot) => snapshot,
            other => panic!("expected inserted non-send row, got {other:?}"),
        };
        assert_eq!(snapshot.decision_kind(), kind);
        assert_eq!(snapshot.state().as_str(), state);
        assert_eq!(snapshot.reason().as_str(), reason);
        assert_eq!(snapshot.prepared_push_bytes(), None);
        assert_eq!(snapshot.rendered_bytes(), None);
        assert_eq!(snapshot.payload_sha256(), None);
        assert_eq!(snapshot.rendered_sha256(), None);
    }
}

#[test]
fn w08_initial_retry_is_idempotent_and_immutable_drift_never_overwrites() {
    let (_root, database) = w08_database();
    let prepared = crate::monitor::push_job::w08_prepared_push_fixture();
    let draft = InitialIntentDraft::ready(
        w08_identity("000001.SZ"),
        &prepared,
        w08_digest('e'),
        w08_digest('f'),
        UtcMicros::try_new(1_788_743_100_000_000).unwrap(),
    )
    .unwrap();
    let drift = InitialIntentDraft::ready(
        w08_identity("000001.SZ"),
        &prepared,
        w08_digest('d'),
        w08_digest('f'),
        UtcMicros::try_new(1_788_743_100_000_000).unwrap(),
    )
    .unwrap();
    let mut store = BusinessIntentStore::open(&database).unwrap();

    let inserted = store.record_initial(&draft).unwrap();
    let original = inserted.snapshot().clone();
    let retry = store.record_initial(&draft).unwrap();
    assert!(matches!(retry, InitialIntentOutcome::ExistingIdentical(_)));
    assert_eq!(retry.snapshot(), &original);

    assert!(matches!(
        store.record_initial(&drift),
        Err(IntentStoreError::ImmutableConflict { .. })
    ));
    assert_eq!(store.inspect(draft.intent_id()).unwrap().unwrap(), original);
    assert_eq!(store.intent_count().unwrap(), 1);
}

fn w08_ready_draft() -> InitialIntentDraft {
    InitialIntentDraft::ready(
        w08_identity("000001.SZ"),
        &crate::monitor::push_job::w08_prepared_push_fixture(),
        w08_digest('e'),
        w08_digest('f'),
        UtcMicros::try_new(1_788_743_100_000_000).unwrap(),
    )
    .unwrap()
}

fn w08_actor(value: &str) -> TransitionActor {
    TransitionActor::try_new(value.to_owned()).unwrap()
}

fn w08_dispatch_claim(draft: &InitialIntentDraft) -> IntentTransitionCommand {
    IntentTransitionCommand::try_new(
        draft.intent_id().clone(),
        super::IntentState::PendingDispatch,
        super::IntentState::AwaitingAuthority,
        0,
        w08_actor("dispatcher-1"),
        crate::monitor::push_job::ReasonCode::IntentDispatchClaimed,
        UtcMicros::try_new(1_788_743_100_001_000).unwrap(),
        LeaseAction::Acquire {
            owner: LeaseOwnerId::try_new("dispatcher-1".to_owned()).unwrap(),
            until: UtcMicros::try_new(1_788_743_160_000_000).unwrap(),
        },
    )
    .unwrap()
}

fn w08_sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};

    hex::encode(Sha256::digest(bytes))
}

#[test]
fn w08_transition_versions_and_previous_hash_form_one_exact_chain() {
    let (_root, database) = w08_database();
    let draft = w08_ready_draft();
    let mut store = BusinessIntentStore::open(&database).unwrap();
    store.record_initial(&draft).unwrap();

    let first_command = w08_dispatch_claim(&draft);
    let first = store.apply_nonterminal_transition(&first_command).unwrap();
    let first_receipt = first.receipt().expect("first transition receipt").clone();
    assert!(matches!(first, TransitionOutcome::Applied(_)));
    let event_preimage = format!(
        "IntentTransitionV1\0{{\"expected_version\":0,\"intent_id\":\"{}\",\"result_version\":1}}",
        draft.intent_id().as_str()
    );
    assert_eq!(
        first_receipt.event_id().as_str(),
        w08_sha256(event_preimage.as_bytes())
    );
    let canonical_preimage = format!(
        concat!(
            "IntentTransitionV1\0{{",
            "\"actor\":\"dispatcher-1\",",
            "\"event_id\":\"{}\",",
            "\"expected_version\":0,",
            "\"from_state\":\"PendingDispatch\",",
            "\"intent_id\":\"{}\",",
            "\"occurred_at\":1788743100001000,",
            "\"operator_audit_ref\":null,",
            "\"operator_audit_sha256\":null,",
            "\"previous_sha256\":null,",
            "\"reason\":\"intent.dispatch_claimed\",",
            "\"result_version\":1,",
            "\"terminal_binding_sha256\":null,",
            "\"terminal_decision_id\":null,",
            "\"terminal_disposition\":null,",
            "\"terminal_ref_id\":null,",
            "\"to_state\":\"AwaitingAuthority\"",
            "}}"
        ),
        first_receipt.event_id().as_str(),
        draft.intent_id().as_str(),
    );
    assert_eq!(
        first_receipt.canonical_sha256().as_str(),
        w08_sha256(canonical_preimage.as_bytes())
    );
    assert_eq!(first_receipt.expected_version(), 0);
    assert_eq!(first_receipt.result_version(), 1);
    assert_eq!(first_receipt.previous_sha256(), None);

    let second_command = IntentTransitionCommand::try_new(
        draft.intent_id().clone(),
        super::IntentState::AwaitingAuthority,
        super::IntentState::ResolutionRequired,
        1,
        w08_actor("authority-recovery-1"),
        crate::monitor::push_job::ReasonCode::TransportUncertain,
        UtcMicros::try_new(1_788_743_100_002_000).unwrap(),
        LeaseAction::Preserve,
    )
    .unwrap();
    let second = store.apply_nonterminal_transition(&second_command).unwrap();
    let second_receipt = second.receipt().unwrap().clone();
    assert_eq!(second_receipt.expected_version(), 1);
    assert_eq!(second_receipt.result_version(), 2);
    assert_eq!(
        second_receipt.previous_sha256(),
        Some(first_receipt.canonical_sha256())
    );

    let third_command = IntentTransitionCommand::try_new(
        draft.intent_id().clone(),
        super::IntentState::ResolutionRequired,
        super::IntentState::ResolutionRequired,
        2,
        w08_actor("reconciler-1"),
        crate::monitor::push_job::ReasonCode::IntentLeaseHeld,
        UtcMicros::try_new(1_788_743_100_003_000).unwrap(),
        LeaseAction::Preserve,
    )
    .unwrap();
    let third = store.apply_nonterminal_transition(&third_command).unwrap();
    let third_receipt = third.receipt().unwrap().clone();
    assert_eq!(third_receipt.result_version(), 3);
    assert_eq!(
        third_receipt.previous_sha256(),
        Some(second_receipt.canonical_sha256())
    );

    let chain = store.inspect_transition_chain(draft.intent_id()).unwrap();
    assert_eq!(chain, vec![first_receipt, second_receipt, third_receipt]);
    let current = store.inspect(draft.intent_id()).unwrap().unwrap();
    assert_eq!(current.version(), 3);
    assert_eq!(current.state(), super::IntentState::ResolutionRequired);
    assert_eq!(
        current.previous_state(),
        Some(super::IntentState::ResolutionRequired)
    );
    assert_eq!(current.lease_owner(), Some("dispatcher-1"));
}

#[test]
fn w08_stale_or_competing_transition_is_zero_write_and_preserves_winner() {
    let (_root, database) = w08_database();
    let draft = w08_ready_draft();
    let mut store = BusinessIntentStore::open(&database).unwrap();
    store.record_initial(&draft).unwrap();
    let winner = w08_dispatch_claim(&draft);
    let winner_receipt = store
        .apply_nonterminal_transition(&winner)
        .unwrap()
        .receipt()
        .unwrap()
        .clone();
    let competing = IntentTransitionCommand::try_new(
        draft.intent_id().clone(),
        super::IntentState::PendingDispatch,
        super::IntentState::NoData,
        0,
        w08_actor("producer-2"),
        crate::monitor::push_job::ReasonCode::IntentNoData,
        UtcMicros::try_new(1_788_743_100_001_001).unwrap(),
        LeaseAction::Preserve,
    )
    .unwrap();

    let outcome = store.apply_nonterminal_transition(&competing).unwrap();
    assert!(matches!(outcome, TransitionOutcome::Conflict { .. }));
    assert_eq!(store.transition_count(draft.intent_id()).unwrap(), 1);
    assert_eq!(
        store.inspect_transition_chain(draft.intent_id()).unwrap(),
        vec![winner_receipt]
    );
    let current = store.inspect(draft.intent_id()).unwrap().unwrap();
    assert_eq!(current.state(), super::IntentState::AwaitingAuthority);
    assert!(current.prepared_push_bytes().is_some());
}

#[test]
fn w08_transaction_faults_before_commit_roll_back_cas_and_append_together() {
    for fault in [TransitionFault::AfterCas, TransitionFault::AfterAppend] {
        let (_root, database) = w08_database();
        let draft = w08_ready_draft();
        let mut store = BusinessIntentStore::open(&database).unwrap();
        let original = store.record_initial(&draft).unwrap().snapshot().clone();
        let command = w08_dispatch_claim(&draft);

        assert!(matches!(
            store.apply_nonterminal_transition_with_fault(&command, fault),
            Err(IntentStoreError::InjectedFault { .. })
        ));
        assert_eq!(store.inspect(draft.intent_id()).unwrap().unwrap(), original);
        assert_eq!(store.transition_count(draft.intent_id()).unwrap(), 0);
    }
}

#[test]
fn w08_commit_ack_loss_is_recovered_by_exact_event_without_second_write() {
    let (_root, database) = w08_database();
    let draft = w08_ready_draft();
    let mut store = BusinessIntentStore::open(&database).unwrap();
    store.record_initial(&draft).unwrap();
    let command = w08_dispatch_claim(&draft);

    assert!(matches!(
        store
            .apply_nonterminal_transition_with_fault(&command, TransitionFault::AfterCommitAckLost),
        Err(IntentStoreError::InjectedFault { .. })
    ));
    assert_eq!(store.transition_count(draft.intent_id()).unwrap(), 1);
    let retry = store.apply_nonterminal_transition(&command).unwrap();
    assert!(matches!(retry, TransitionOutcome::AlreadyCommitted(_)));
    assert_eq!(store.transition_count(draft.intent_id()).unwrap(), 1);
    assert_eq!(
        store.inspect(draft.intent_id()).unwrap().unwrap().version(),
        1
    );
}

#[test]
fn w08_initial_commit_boundaries_recover_without_duplicate_outbox() {
    let (_root, database) = w08_database();
    let draft = w08_ready_draft();
    let mut store = BusinessIntentStore::open(&database).unwrap();
    assert!(matches!(
        store.record_initial_with_fault(&draft, InitialCommitFault::BeforeCommit),
        Err(IntentStoreError::InjectedFault { .. })
    ));
    assert_eq!(store.intent_count().unwrap(), 0);

    assert!(matches!(
        store.record_initial_with_fault(&draft, InitialCommitFault::AfterCommitAckLost),
        Err(IntentStoreError::InjectedFault { .. })
    ));
    assert_eq!(store.intent_count().unwrap(), 1);
    let retry = store.record_initial(&draft).unwrap();
    assert!(matches!(retry, InitialIntentOutcome::ExistingIdentical(_)));
    assert_eq!(store.intent_count().unwrap(), 1);
}

#[test]
fn w08_initial_retry_after_transition_compares_only_immutable_material() {
    let (_root, database) = w08_database();
    let draft = w08_ready_draft();
    let mut store = BusinessIntentStore::open(&database).unwrap();
    store.record_initial(&draft).unwrap();
    store
        .apply_nonterminal_transition(&w08_dispatch_claim(&draft))
        .unwrap();

    let retry = store.record_initial(&draft).unwrap();
    assert!(matches!(retry, InitialIntentOutcome::ExistingIdentical(_)));
    assert_eq!(
        retry.snapshot().state(),
        super::IntentState::AwaitingAuthority
    );
    assert_eq!(retry.snapshot().version(), 1);
    assert_eq!(store.intent_count().unwrap(), 1);
    assert_eq!(store.transition_count(draft.intent_id()).unwrap(), 1);
}

#[test]
fn w08_schema_keeps_transition_rows_append_only() {
    let (_root, database) = w08_database();
    let draft = w08_ready_draft();
    let mut store = BusinessIntentStore::open(&database).unwrap();
    store.record_initial(&draft).unwrap();
    let receipt = store
        .apply_nonterminal_transition(&w08_dispatch_claim(&draft))
        .unwrap()
        .receipt()
        .unwrap()
        .clone();
    drop(store);

    let connection = Connection::open(&database).unwrap();
    assert!(connection
        .execute(
            "UPDATE push_intent_transitions SET actor='tampered' WHERE event_id=?",
            [receipt.event_id().as_str()],
        )
        .is_err());
    assert!(connection
        .execute(
            "DELETE FROM push_intent_transitions WHERE event_id=?",
            [receipt.event_id().as_str()],
        )
        .is_err());
    let exact: (String, String) = connection
        .query_row(
            "SELECT actor,canonical_sha256 FROM push_intent_transitions WHERE event_id=?",
            [receipt.event_id().as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(exact.0, receipt.actor());
    assert_eq!(exact.1, receipt.canonical_sha256().as_str());
}

#[test]
fn w08_dispatch_transition_requires_a_persisted_future_lease() {
    let (_root, database) = w08_database();
    let draft = w08_ready_draft();
    let mut store = BusinessIntentStore::open(&database).unwrap();
    let original = store.record_initial(&draft).unwrap().snapshot().clone();
    let without_lease = IntentTransitionCommand::try_new(
        draft.intent_id().clone(),
        super::IntentState::PendingDispatch,
        super::IntentState::AwaitingAuthority,
        0,
        w08_actor("dispatcher-1"),
        crate::monitor::push_job::ReasonCode::IntentDispatchClaimed,
        UtcMicros::try_new(1_788_743_100_001_000).unwrap(),
        LeaseAction::Preserve,
    )
    .unwrap();

    assert!(matches!(
        store.apply_nonterminal_transition(&without_lease),
        Err(IntentStoreError::InvalidTransition {
            check: "dispatch_requires_active_lease"
        })
    ));
    assert_eq!(store.inspect(draft.intent_id()).unwrap().unwrap(), original);
    assert_eq!(store.transition_count(draft.intent_id()).unwrap(), 0);
}

#[test]
fn w08_existing_head_event_rejects_a_retry_with_different_lease_material() {
    let (_root, database) = w08_database();
    let draft = w08_ready_draft();
    let mut store = BusinessIntentStore::open(&database).unwrap();
    store.record_initial(&draft).unwrap();
    let original = w08_dispatch_claim(&draft);
    store.apply_nonterminal_transition(&original).unwrap();
    let changed_lease = IntentTransitionCommand::try_new(
        draft.intent_id().clone(),
        super::IntentState::PendingDispatch,
        super::IntentState::AwaitingAuthority,
        0,
        w08_actor("dispatcher-1"),
        crate::monitor::push_job::ReasonCode::IntentDispatchClaimed,
        UtcMicros::try_new(1_788_743_100_001_000).unwrap(),
        LeaseAction::Acquire {
            owner: LeaseOwnerId::try_new("dispatcher-1".to_owned()).unwrap(),
            until: UtcMicros::try_new(1_788_743_160_000_001).unwrap(),
        },
    )
    .unwrap();

    assert!(matches!(
        store.apply_nonterminal_transition(&changed_lease).unwrap(),
        TransitionOutcome::Conflict { .. }
    ));
    assert_eq!(store.transition_count(draft.intent_id()).unwrap(), 1);
}

#[test]
fn w08_existing_event_fast_path_still_revalidates_the_complete_chain() {
    let (_root, database) = w08_database();
    let draft = w08_ready_draft();
    let mut store = BusinessIntentStore::open(&database).unwrap();
    store.record_initial(&draft).unwrap();
    let first_command = w08_dispatch_claim(&draft);
    store.apply_nonterminal_transition(&first_command).unwrap();
    let second_command = IntentTransitionCommand::try_new(
        draft.intent_id().clone(),
        super::IntentState::AwaitingAuthority,
        super::IntentState::ResolutionRequired,
        1,
        w08_actor("authority-recovery-1"),
        crate::monitor::push_job::ReasonCode::TransportUncertain,
        UtcMicros::try_new(1_788_743_100_002_000).unwrap(),
        LeaseAction::Preserve,
    )
    .unwrap();
    store.apply_nonterminal_transition(&second_command).unwrap();

    let tamper = Connection::open(&database).unwrap();
    tamper
        .execute_batch(
            "DROP TRIGGER push_intents_cas; \
             UPDATE push_intents SET version=3 WHERE intent_id IN (SELECT intent_id FROM push_intents);",
        )
        .unwrap();
    drop(tamper);

    assert!(matches!(
        store.record_initial(&draft),
        Err(IntentStoreError::IntegrityFailed {
            check: "intent_version_transition_count"
        })
    ));
    assert!(matches!(
        store.apply_nonterminal_transition(&first_command),
        Err(IntentStoreError::IntegrityFailed {
            check: "intent_version_transition_count"
        })
    ));
    assert_eq!(store.transition_count(draft.intent_id()).unwrap(), 2);
}

#[test]
fn w08_chain_read_rejects_a_hash_valid_but_semantically_invalid_edge() {
    let (_root, database) = w08_database();
    let draft = w08_ready_draft();
    let mut store = BusinessIntentStore::open(&database).unwrap();
    store.record_initial(&draft).unwrap();
    let receipt = store
        .apply_nonterminal_transition(&w08_dispatch_claim(&draft))
        .unwrap()
        .receipt()
        .unwrap()
        .clone();
    let invalid_preimage = format!(
        concat!(
            "IntentTransitionV1\0{{",
            "\"actor\":\"dispatcher-1\",",
            "\"event_id\":\"{}\",",
            "\"expected_version\":0,",
            "\"from_state\":\"PendingDispatch\",",
            "\"intent_id\":\"{}\",",
            "\"occurred_at\":1788743100001000,",
            "\"operator_audit_ref\":null,",
            "\"operator_audit_sha256\":null,",
            "\"previous_sha256\":null,",
            "\"reason\":\"intent.lease_held\",",
            "\"result_version\":1,",
            "\"terminal_binding_sha256\":null,",
            "\"terminal_decision_id\":null,",
            "\"terminal_disposition\":null,",
            "\"terminal_ref_id\":null,",
            "\"to_state\":\"AwaitingAuthority\"",
            "}}"
        ),
        receipt.event_id().as_str(),
        draft.intent_id().as_str(),
    );
    let invalid_sha256 = w08_sha256(invalid_preimage.as_bytes());

    let tamper = Connection::open(&database).unwrap();
    tamper
        .execute_batch(
            "DROP TRIGGER push_intent_transitions_update; \
             DROP TRIGGER push_intents_cas;",
        )
        .unwrap();
    tamper
        .execute(
            "UPDATE push_intent_transitions SET reason=?,canonical_sha256=? WHERE event_id=?",
            rusqlite::params![
                "intent.lease_held",
                invalid_sha256,
                receipt.event_id().as_str()
            ],
        )
        .unwrap();
    tamper
        .execute(
            "UPDATE push_intents SET reason='intent.lease_held' WHERE intent_id=?",
            [draft.intent_id().as_str()],
        )
        .unwrap();
    drop(tamper);

    assert!(matches!(
        store.inspect_transition_chain(draft.intent_id()),
        Err(IntentStoreError::IntegrityFailed {
            check: "transition_edge_reason"
        })
    ));
}
