use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use crate::monitor::push_job::{canonical_digest, namespace_value, Namespace, RunId};
use rusqlite::{params, Connection};

use super::readiness_store_schema::{initialize_database, open_read_only, open_writer};

fn namespace() -> Namespace {
    Namespace::test(RunId::try_new("TEST_CODE-w15-store".to_owned()).expect("TEST_CODE namespace"))
}

fn database_path(root: &tempfile::TempDir, name: &str) -> PathBuf {
    root.path()
        .canonicalize()
        .expect("TEST_CODE canonical operational store root")
        .join(name)
}

fn initialized_database(name: &str) -> (tempfile::TempDir, PathBuf, Namespace) {
    let root = tempfile::tempdir().expect("TEST_CODE create operational store root");
    let database = database_path(&root, name);
    let namespace = namespace();
    initialize_database(&database, &namespace).expect("TEST_CODE explicit initialization");
    (root, database, namespace)
}

fn insert_linked_pair(connection: &mut Connection) {
    let snapshot_id = "1".repeat(64);
    let event_id = "2".repeat(64);
    let transaction = connection
        .transaction()
        .expect("TEST_CODE begin linked insert");
    transaction
        .execute(
            "INSERT INTO operational_readiness_snapshot(\
                 snapshot_id,event_id,canonical_bytes\
             ) VALUES(?1,?2,?3)",
            params![snapshot_id, event_id, b"snapshot".as_slice()],
        )
        .expect("TEST_CODE insert snapshot");
    transaction
        .execute(
            "INSERT INTO operational_readiness_recovery_event(\
                 event_id,event_sha256,before_snapshot_id,after_snapshot_id,canonical_bytes\
             ) VALUES(?1,?1,NULL,?2,?3)",
            params![event_id, snapshot_id, b"event".as_slice()],
        )
        .expect("TEST_CODE insert recovery event");
    transaction
        .commit()
        .expect("TEST_CODE commit linked insert");
}

#[test]
fn w15_explicit_initialization_is_idempotent_and_namespace_bound() {
    let root = tempfile::tempdir().expect("TEST_CODE create operational store root");
    let root = root
        .path()
        .canonicalize()
        .expect("TEST_CODE canonical operational store root");
    let database = root.join("operational-readiness.sqlite3");
    let namespace = namespace();

    initialize_database(&database, &namespace).expect("TEST_CODE explicit initialization");
    let initialized_bytes = fs::read(&database).expect("TEST_CODE initialized database bytes");

    let connection = open_read_only(&database, &namespace).expect("TEST_CODE read-only open");
    let header: (i64, String, String) = connection
        .query_row(
            "SELECT schema_version, ddl_sha256, namespace_sha256 \
             FROM operational_readiness_schema WHERE singleton=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("TEST_CODE read schema header");
    let expected_namespace = canonical_digest(
        "OperationalReadinessNamespace/v1",
        &BTreeMap::from([("namespace", namespace_value(&namespace))]),
    );
    assert_eq!(header.0, 1);
    assert_eq!(header.1.len(), 64);
    assert!(header
        .1
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
    assert_eq!(header.2, expected_namespace.as_str());
    drop(connection);

    initialize_database(&database, &namespace).expect("TEST_CODE idempotent initialization");
    assert_eq!(
        fs::read(&database).expect("TEST_CODE reinitialized database bytes"),
        initialized_bytes
    );

    assert!(initialize_database(&database, &Namespace::Production).is_err());
    assert_eq!(
        fs::read(&database).expect("TEST_CODE namespace-rejected database bytes"),
        initialized_bytes
    );
}

#[test]
fn w15_all_persisted_identity_fields_require_lowercase_sha256() {
    let (_root, database, namespace) = initialized_database("identity-checks.sqlite3");
    let connection = open_writer(&database, &namespace).expect("TEST_CODE writer");
    connection
        .execute_batch("PRAGMA foreign_keys=OFF;")
        .expect("TEST_CODE isolate CHECK constraints");
    let digest_a = "a".repeat(64);
    let digest_b = "b".repeat(64);
    let digest_c = "c".repeat(64);
    let digest_d = "d".repeat(64);
    let digest_e = "e".repeat(64);
    let digest_f = "f".repeat(64);
    let digest_g = "7".repeat(64);
    let digest_h = "8".repeat(64);

    let invalid_snapshot_id = connection.execute(
        "INSERT INTO operational_readiness_snapshot(\
             snapshot_id,event_id,canonical_bytes\
         ) VALUES('not-a-digest',?1,?2)",
        params![digest_a, b"snapshot".as_slice()],
    );
    let invalid_snapshot_event_id = connection.execute(
        "INSERT INTO operational_readiness_snapshot(\
             snapshot_id,event_id,canonical_bytes\
         ) VALUES(?1,'NOT-LOWERCASE-SHA256',?2)",
        params![digest_b, b"snapshot".as_slice()],
    );
    let invalid_event_id = connection.execute(
        "INSERT INTO operational_readiness_recovery_event(\
             event_id,event_sha256,before_snapshot_id,after_snapshot_id,canonical_bytes\
         ) VALUES('not-a-digest',?1,NULL,?2,?3)",
        params![digest_a, digest_c, b"event".as_slice()],
    );
    let invalid_before_snapshot_id = connection.execute(
        "INSERT INTO operational_readiness_recovery_event(\
             event_id,event_sha256,before_snapshot_id,after_snapshot_id,canonical_bytes\
         ) VALUES(?1,?1,'not-a-digest',?2,?3)",
        params![digest_d, digest_e, b"event".as_slice()],
    );
    let invalid_after_snapshot_id = connection.execute(
        "INSERT INTO operational_readiness_recovery_event(\
             event_id,event_sha256,before_snapshot_id,after_snapshot_id,canonical_bytes\
         ) VALUES(?1,?1,NULL,'not-a-digest',?2)",
        params![digest_f, b"event".as_slice()],
    );
    let invalid_scope_key = connection.execute(
        "INSERT INTO operational_readiness_head(scope_key,version,snapshot_id,event_id) \
         VALUES('not-a-digest',1,?1,?2)",
        params![digest_g, digest_h],
    );

    assert!(invalid_snapshot_id.is_err());
    assert!(invalid_snapshot_event_id.is_err());
    assert!(invalid_event_id.is_err());
    assert!(invalid_before_snapshot_id.is_err());
    assert!(invalid_after_snapshot_id.is_err());
    assert!(invalid_scope_key.is_err());
}

#[test]
fn w15_unknown_sqlite_lookalike_object_is_rejected_without_mutation() {
    let (_root, database, namespace) = initialized_database("unknown-object.sqlite3");
    let connection = open_writer(&database, &namespace).expect("TEST_CODE writer");
    connection
        .execute_batch("CREATE TABLE sqliteX_foreign(value TEXT);")
        .expect("TEST_CODE inject unknown object");
    drop(connection);
    let tampered_bytes = fs::read(&database).expect("TEST_CODE tampered bytes");

    assert!(open_read_only(&database, &namespace).is_err());
    assert!(open_writer(&database, &namespace).is_err());
    assert_eq!(
        fs::read(&database).expect("TEST_CODE rejected bytes"),
        tampered_bytes
    );
}

#[test]
fn w15_append_only_objects_reject_update_delete_and_replace() {
    let (_root, database, namespace) = initialized_database("append-only.sqlite3");
    let mut connection = open_writer(&database, &namespace).expect("TEST_CODE writer");
    insert_linked_pair(&mut connection);
    connection
        .execute(
            "INSERT INTO operational_readiness_head(scope_key,version,snapshot_id,event_id) \
             VALUES(?1,1,?2,?3)",
            params!["3".repeat(64), "1".repeat(64), "2".repeat(64)],
        )
        .expect("TEST_CODE insert head");
    let header: (i64, String, String) = connection
        .query_row(
            "SELECT schema_version,ddl_sha256,namespace_sha256 \
             FROM operational_readiness_schema WHERE singleton=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("TEST_CODE header");

    let snapshot_update = connection.execute(
        "UPDATE operational_readiness_snapshot SET canonical_bytes=?1 WHERE snapshot_id=?2",
        params![b"changed".as_slice(), "1".repeat(64)],
    );
    let snapshot_delete = connection.execute(
        "DELETE FROM operational_readiness_snapshot WHERE snapshot_id=?1",
        params!["1".repeat(64)],
    );
    let snapshot_replace = connection.execute(
        "INSERT OR REPLACE INTO operational_readiness_snapshot(\
             snapshot_id,event_id,canonical_bytes\
         ) VALUES(?1,?2,?3)",
        params!["1".repeat(64), "2".repeat(64), b"changed".as_slice()],
    );
    let event_update = connection.execute(
        "UPDATE operational_readiness_recovery_event SET canonical_bytes=?1 WHERE event_id=?2",
        params![b"changed".as_slice(), "2".repeat(64)],
    );
    let event_delete = connection.execute(
        "DELETE FROM operational_readiness_recovery_event WHERE event_id=?1",
        params!["2".repeat(64)],
    );
    let event_replace = connection.execute(
        "INSERT OR REPLACE INTO operational_readiness_recovery_event(\
             event_id,event_sha256,before_snapshot_id,after_snapshot_id,canonical_bytes\
         ) VALUES(?1,?1,NULL,?2,?3)",
        params!["2".repeat(64), "1".repeat(64), b"changed".as_slice()],
    );
    let header_update = connection.execute(
        "UPDATE operational_readiness_schema SET schema_version=2 WHERE singleton=1",
        [],
    );
    let header_delete = connection.execute(
        "DELETE FROM operational_readiness_schema WHERE singleton=1",
        [],
    );
    let header_replace = connection.execute(
        "INSERT OR REPLACE INTO operational_readiness_schema(\
             singleton,schema_version,ddl_sha256,namespace_sha256\
         ) VALUES(1,?1,?2,?3)",
        params![header.0, header.1, header.2],
    );

    assert!(snapshot_update.is_err());
    assert!(snapshot_delete.is_err());
    assert!(snapshot_replace.is_err());
    assert!(event_update.is_err());
    assert!(event_delete.is_err());
    assert!(event_replace.is_err());
    assert!(header_update.is_err());
    assert!(header_delete.is_err());
    assert!(header_replace.is_err());
}

#[test]
fn w15_missing_read_and_write_open_never_create_a_database() {
    let root = tempfile::tempdir().expect("TEST_CODE create missing store root");
    let database = database_path(&root, "missing.sqlite3");

    assert!(open_read_only(&database, &namespace()).is_err());
    assert!(!database.exists());
    assert!(open_writer(&database, &namespace()).is_err());
    assert!(!database.exists());
}

#[test]
fn w15_existing_empty_foreign_and_partial_databases_are_rejected_unchanged() {
    let root = tempfile::tempdir().expect("TEST_CODE create rejected store root");
    let namespace = namespace();
    for (name, setup) in [
        ("empty.sqlite3", ""),
        ("foreign.sqlite3", "CREATE TABLE business_data(value TEXT);"),
        (
            "partial.sqlite3",
            "CREATE TABLE operational_readiness_schema(singleton INTEGER PRIMARY KEY);",
        ),
    ] {
        let database = database_path(&root, name);
        if setup.is_empty() {
            fs::write(&database, []).expect("TEST_CODE create empty file");
        } else {
            let connection = Connection::open(&database).expect("TEST_CODE create foreign DB");
            connection
                .execute_batch(setup)
                .expect("TEST_CODE create rejected schema");
        }
        let original_bytes = fs::read(&database).expect("TEST_CODE original rejected bytes");

        assert!(initialize_database(&database, &namespace).is_err());
        assert!(open_read_only(&database, &namespace).is_err());
        assert!(open_writer(&database, &namespace).is_err());
        assert_eq!(
            fs::read(&database).expect("TEST_CODE unchanged rejected bytes"),
            original_bytes,
            "{name} must not be modified"
        );
    }
}

#[test]
fn w15_tampered_header_and_object_definition_are_rejected_unchanged() {
    let (_root, database, namespace) = initialized_database("tampered.sqlite3");
    let connection = Connection::open(&database).expect("TEST_CODE fault injection connection");
    let trigger_sql: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master \
             WHERE type='trigger' AND name='operational_readiness_schema_no_update'",
            [],
            |row| row.get(0),
        )
        .expect("TEST_CODE preserve trigger definition");
    connection
        .execute_batch("DROP TRIGGER operational_readiness_schema_no_update;")
        .expect("TEST_CODE disable header guard");
    connection
        .execute(
            "UPDATE operational_readiness_schema SET ddl_sha256=?1 WHERE singleton=1",
            params!["f".repeat(64)],
        )
        .expect("TEST_CODE tamper header");
    connection
        .execute_batch(&trigger_sql)
        .expect("TEST_CODE restore exact trigger");
    drop(connection);
    let tampered_bytes = fs::read(&database).expect("TEST_CODE tampered bytes");

    assert!(open_read_only(&database, &namespace).is_err());
    assert!(open_writer(&database, &namespace).is_err());
    assert_eq!(
        fs::read(&database).expect("TEST_CODE rejected tampered bytes"),
        tampered_bytes
    );

    let object_database = database.with_file_name("tampered-object.sqlite3");
    initialize_database(&object_database, &namespace).expect("TEST_CODE object database");
    let connection = Connection::open(&object_database).expect("TEST_CODE object fault injection");
    connection
        .execute_batch(
            "DROP TRIGGER operational_readiness_snapshot_no_update; \
             CREATE TRIGGER operational_readiness_snapshot_no_update \
             BEFORE UPDATE ON operational_readiness_snapshot BEGIN SELECT 1; END;",
        )
        .expect("TEST_CODE rewrite trigger");
    drop(connection);
    let tampered_bytes = fs::read(&object_database).expect("TEST_CODE object tampered bytes");
    assert!(open_read_only(&object_database, &namespace).is_err());
    assert_eq!(
        fs::read(&object_database).expect("TEST_CODE rejected object bytes"),
        tampered_bytes
    );
}

#[test]
fn w15_reader_is_query_only_and_writer_enforces_foreign_keys() {
    let (_root, database, namespace) = initialized_database("connection-guards.sqlite3");
    let reader = open_read_only(&database, &namespace).expect("TEST_CODE reader");
    assert_eq!(
        reader
            .query_row("PRAGMA query_only", [], |row| row.get::<_, i64>(0))
            .expect("TEST_CODE query_only"),
        1
    );
    assert!(reader
        .execute(
            "INSERT INTO operational_readiness_head(\
                 scope_key,version,snapshot_id,event_id\
             ) VALUES(?1,1,?2,?3)",
            params!["3".repeat(64), "1".repeat(64), "2".repeat(64)],
        )
        .is_err());
    drop(reader);

    let writer = open_writer(&database, &namespace).expect("TEST_CODE writer");
    assert_eq!(
        writer
            .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
            .expect("TEST_CODE foreign_keys"),
        1
    );
}

#[test]
fn w15_errors_do_not_disclose_protected_database_paths() {
    let root = tempfile::tempdir().expect("TEST_CODE create protected path root");
    let secret = "TEST_CODE-SECRET-operational.sqlite3";
    let database = database_path(&root, secret);

    let errors = [
        open_read_only(&database, &namespace()).unwrap_err(),
        open_writer(&database, &namespace()).unwrap_err(),
    ];
    for error in errors {
        assert!(!format!("{error}").contains(secret));
        assert!(!format!("{error:?}").contains(secret));
    }
}
