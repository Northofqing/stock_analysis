use std::collections::BTreeMap;
use std::fs;
use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::{symlink, MetadataExt};

use crate::monitor::push_job::{canonical_digest, namespace_value, Namespace, RunId};
use rusqlite::{params, Connection};

use super::readiness_store_schema::{
    initialize_database, initialize_database_with_before_create_hook,
    initialize_database_with_connection_open_hooks, initialize_database_with_creation_hooks,
    with_read_only, with_read_only_hooks, with_write_transaction, with_write_transaction_hooks,
    ReadinessSchemaError,
};

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

fn directory_bytes(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fs::read_dir(root)
        .expect("TEST_CODE read database directory")
        .map(|entry| {
            let entry = entry.expect("TEST_CODE database directory entry");
            let name = entry.file_name().to_string_lossy().into_owned();
            let bytes = fs::read(entry.path()).expect("TEST_CODE database artifact bytes");
            (name, bytes)
        })
        .collect()
}

fn schema_error(check: &'static str) -> ReadinessSchemaError {
    ReadinessSchemaError::ValidationFailed { check }
}

fn insert_linked_pair(connection: &Connection) {
    let snapshot_id = "1".repeat(64);
    let event_id = "2".repeat(64);
    connection
        .execute(
            "INSERT INTO operational_readiness_snapshot(\
                 snapshot_id,event_id,canonical_bytes\
             ) VALUES(?1,?2,?3)",
            params![snapshot_id, event_id, b"snapshot".as_slice()],
        )
        .expect("TEST_CODE insert snapshot");
    connection
        .execute(
            "INSERT INTO operational_readiness_recovery_event(\
                 event_id,event_sha256,before_snapshot_id,after_snapshot_id,canonical_bytes\
             ) VALUES(?1,?1,NULL,?2,?3)",
            params![event_id, snapshot_id, b"event".as_slice()],
        )
        .expect("TEST_CODE insert recovery event");
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
    assert!(initialized_bytes.len() >= 100);
    assert_eq!(&initialized_bytes[..16], b"SQLite format 3\0");
    assert_eq!((initialized_bytes[18], initialized_bytes[19]), (1, 1));

    let header: (i64, String, String) = with_read_only(&database, &namespace, |connection| {
        connection
            .query_row(
                "SELECT schema_version, ddl_sha256, namespace_sha256 \
                 FROM operational_readiness_schema WHERE singleton=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|_| schema_error("TEST_CODE read schema header"))
    })
    .expect("TEST_CODE read-only transaction");
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
    with_write_transaction(&database, &namespace, |connection| {
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
        Ok::<(), ReadinessSchemaError>(())
    })
    .expect("TEST_CODE identity constraint transaction");
}

#[test]
fn w15_unknown_sqlite_lookalike_object_is_rejected_without_mutation() {
    let (_root, database, namespace) = initialized_database("unknown-object.sqlite3");
    with_write_transaction(&database, &namespace, |connection| {
        connection
            .execute_batch("CREATE TABLE sqliteX_foreign(value TEXT);")
            .map_err(|_| schema_error("TEST_CODE inject unknown object"))
    })
    .expect("TEST_CODE unknown object transaction");
    let tampered_bytes = fs::read(&database).expect("TEST_CODE tampered bytes");

    assert!(with_read_only(
        &database,
        &namespace,
        |_| Ok::<(), ReadinessSchemaError>(())
    )
    .is_err());
    assert!(with_write_transaction(
        &database,
        &namespace,
        |_| Ok::<(), ReadinessSchemaError>(())
    )
    .is_err());
    assert_eq!(
        fs::read(&database).expect("TEST_CODE rejected bytes"),
        tampered_bytes
    );
}

#[test]
fn w15_append_only_objects_reject_update_delete_and_replace() {
    let (_root, database, namespace) = initialized_database("append-only.sqlite3");
    with_write_transaction(&database, &namespace, |connection| {
        insert_linked_pair(connection);
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
        Ok::<(), ReadinessSchemaError>(())
    })
    .expect("TEST_CODE append-only transaction");
}

#[test]
fn w15_missing_read_and_write_open_never_create_a_database() {
    let root = tempfile::tempdir().expect("TEST_CODE create missing store root");
    let database = database_path(&root, "missing.sqlite3");

    assert!(
        with_read_only(&database, &namespace(), |_| Ok::<(), ReadinessSchemaError>(
            ()
        ))
        .is_err()
    );
    assert!(!database.exists());
    assert!(
        with_write_transaction(&database, &namespace(), |_| Ok::<(), ReadinessSchemaError>(
            ()
        ))
        .is_err()
    );
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
        assert!(with_read_only(
            &database,
            &namespace,
            |_| Ok::<(), ReadinessSchemaError>(())
        )
        .is_err());
        assert!(with_write_transaction(&database, &namespace, |_| {
            Ok::<(), ReadinessSchemaError>(())
        })
        .is_err());
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

    assert!(with_read_only(
        &database,
        &namespace,
        |_| Ok::<(), ReadinessSchemaError>(())
    )
    .is_err());
    assert!(with_write_transaction(
        &database,
        &namespace,
        |_| Ok::<(), ReadinessSchemaError>(())
    )
    .is_err());
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
    assert!(with_read_only(&object_database, &namespace, |_| {
        Ok::<(), ReadinessSchemaError>(())
    })
    .is_err());
    assert_eq!(
        fs::read(&object_database).expect("TEST_CODE rejected object bytes"),
        tampered_bytes
    );
}

#[test]
fn w15_reader_is_query_only_and_writer_enforces_foreign_keys() {
    let (_root, database, namespace) = initialized_database("connection-guards.sqlite3");
    with_read_only(&database, &namespace, |reader| {
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
        Ok::<(), ReadinessSchemaError>(())
    })
    .expect("TEST_CODE reader callback");

    with_write_transaction(&database, &namespace, |writer| {
        assert_eq!(
            writer
                .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
                .expect("TEST_CODE foreign_keys"),
            1
        );
        Ok::<(), ReadinessSchemaError>(())
    })
    .expect("TEST_CODE writer callback");
}

#[test]
fn w15_errors_do_not_disclose_protected_database_paths() {
    let root = tempfile::tempdir().expect("TEST_CODE create protected path root");
    let secret = "TEST_CODE-SECRET-operational.sqlite3";
    let database = database_path(&root, secret);

    let errors = [
        with_read_only(&database, &namespace(), |_| {
            Ok::<(), ReadinessSchemaError>(())
        })
        .unwrap_err(),
        with_write_transaction(&database, &namespace(), |_| {
            Ok::<(), ReadinessSchemaError>(())
        })
        .unwrap_err(),
    ];
    for error in errors {
        assert!(!format!("{error}").contains(secret));
        assert!(!format!("{error:?}").contains(secret));
    }
}

#[test]
fn w15_initializer_never_takes_over_a_target_created_after_missing_check() {
    let root = tempfile::tempdir().expect("TEST_CODE create race root");
    let database = database_path(&root, "raced.sqlite3");
    #[cfg(unix)]
    let mut competitor_inode = None;

    let result = initialize_database_with_before_create_hook(&database, &namespace(), || {
        let competitor = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&database)
            .expect("TEST_CODE competitor atomically creates target");
        #[cfg(unix)]
        {
            competitor_inode = Some(
                competitor
                    .metadata()
                    .expect("TEST_CODE competitor metadata")
                    .ino(),
            );
        }
        drop(competitor);
    });

    assert!(result.is_err());
    assert_eq!(
        fs::read(&database).expect("TEST_CODE competitor file remains"),
        Vec::<u8>::new()
    );
    #[cfg(unix)]
    assert_eq!(
        fs::metadata(&database)
            .expect("TEST_CODE raced target metadata")
            .ino(),
        competitor_inode.expect("TEST_CODE competitor inode")
    );

    #[cfg(unix)]
    {
        let link = database.with_file_name("raced-link.sqlite3");
        let competitor_target = database.with_file_name("competitor-owned.sqlite3");
        fs::write(&competitor_target, b"competitor-owned")
            .expect("TEST_CODE competitor target contents");
        let result = initialize_database_with_before_create_hook(&link, &namespace(), || {
            symlink(&competitor_target, &link).expect("TEST_CODE competitor creates symlink");
        });

        assert!(result.is_err());
        assert!(fs::symlink_metadata(&link)
            .expect("TEST_CODE raced symlink metadata")
            .file_type()
            .is_symlink());
        assert_eq!(
            fs::read_link(&link).expect("TEST_CODE raced symlink target"),
            competitor_target
        );
        assert_eq!(
            fs::read(&competitor_target).expect("TEST_CODE competitor contents remain"),
            b"competitor-owned"
        );
    }

    let replaced = database.with_file_name("replaced-after-claim.sqlite3");
    let claimed = database.with_file_name("claimed-by-initializer.sqlite3");
    let result = initialize_database_with_creation_hooks(
        &replaced,
        &namespace(),
        || {},
        || {
            fs::rename(&replaced, &claimed).expect("TEST_CODE preserve claimed inode");
            fs::write(&replaced, b"competitor replacement")
                .expect("TEST_CODE competitor replaces claimed path");
        },
    );
    assert!(result.is_err());
    assert_eq!(
        fs::read(&claimed).expect("TEST_CODE originally claimed file remains"),
        Vec::<u8>::new()
    );
    assert_eq!(
        fs::read(&replaced).expect("TEST_CODE competitor replacement remains"),
        b"competitor replacement"
    );
}

#[test]
fn w15_wal_database_is_rejected_before_read_or_write_open_side_effects() {
    let (root, database, namespace) = initialized_database("wal.sqlite3");
    let connection = Connection::open(&database).expect("TEST_CODE WAL fault injection");
    let journal_mode: String = connection
        .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
        .expect("TEST_CODE enable WAL");
    assert_eq!(journal_mode, "wal");
    connection
        .execute_batch("PRAGMA wal_autocheckpoint=0; PRAGMA user_version=7;")
        .expect("TEST_CODE create WAL transaction");
    drop(connection);

    let header = fs::read(&database).expect("TEST_CODE WAL database header");
    assert!(header.len() >= 100);
    assert_eq!(&header[..16], b"SQLite format 3\0");
    assert_eq!((header[18], header[19]), (2, 2));
    let before_reader = directory_bytes(root.path());

    let reader = with_read_only(
        &database,
        &namespace,
        |_| Ok::<(), ReadinessSchemaError>(()),
    );
    let after_reader = directory_bytes(root.path());
    let reader_rejected = reader.is_err();
    assert!(reader_rejected);
    assert_eq!(after_reader, before_reader);

    let before_writer = directory_bytes(root.path());
    let writer =
        with_write_transaction(
            &database,
            &namespace,
            |_| Ok::<(), ReadinessSchemaError>(()),
        );
    let after_writer = directory_bytes(root.path());
    let writer_rejected = writer.is_err();
    assert!(writer_rejected);
    assert_eq!(after_writer, before_writer);
}

#[test]
fn w15_malformed_or_unknown_sqlite_header_is_rejected_without_side_effects() {
    let (root, database, namespace) = initialized_database("unknown-header.sqlite3");
    let mut file = OpenOptions::new()
        .write(true)
        .open(&database)
        .expect("TEST_CODE open header fault injection");
    file.seek(SeekFrom::Start(18))
        .expect("TEST_CODE seek journal format");
    file.write_all(&[3, 3])
        .expect("TEST_CODE write unknown journal format");
    file.sync_all().expect("TEST_CODE sync unknown header");
    drop(file);
    let before_unknown = directory_bytes(root.path());

    assert!(with_read_only(
        &database,
        &namespace,
        |_| Ok::<(), ReadinessSchemaError>(())
    )
    .is_err());
    assert!(with_write_transaction(
        &database,
        &namespace,
        |_| Ok::<(), ReadinessSchemaError>(())
    )
    .is_err());
    assert_eq!(directory_bytes(root.path()), before_unknown);

    let malformed = database.with_file_name("malformed-header.sqlite3");
    fs::write(&malformed, b"SQLite format 3\0").expect("TEST_CODE write truncated SQLite header");
    let before_malformed = directory_bytes(root.path());
    assert!(
        with_read_only(&malformed, &namespace, |_| Ok::<(), ReadinessSchemaError>(
            ()
        ))
        .is_err()
    );
    assert!(with_write_transaction(&malformed, &namespace, |_| {
        Ok::<(), ReadinessSchemaError>(())
    })
    .is_err());
    assert_eq!(directory_bytes(root.path()), before_malformed);
}

#[test]
fn w15_initializer_writes_only_through_a_connection_bound_to_its_claimed_file() {
    let root = tempfile::tempdir().expect("TEST_CODE create connection-binding root");
    let database = database_path(&root, "connection-binding.sqlite3");
    let initializer_owned = database.with_file_name("initializer-owned.sqlite3");
    let competitor_after_open = database.with_file_name("competitor-after-open.sqlite3");

    let result = initialize_database_with_connection_open_hooks(
        &database,
        &namespace(),
        || {
            fs::rename(&database, &initializer_owned)
                .expect("TEST_CODE move initializer-owned file before SQLite open");
            let competitor = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&database)
                .expect("TEST_CODE create competitor file for SQLite open");
            drop(competitor);
        },
        || {
            fs::rename(&database, &competitor_after_open)
                .expect("TEST_CODE preserve SQLite-bound competitor file");
            fs::rename(&initializer_owned, &database)
                .expect("TEST_CODE restore initializer-owned path before stat");
        },
    );
    let outcome = match &result {
        Ok(_) => "Ok".to_owned(),
        Err(error) => format!("Err({error:?}; {error})"),
    };
    let initializer_bytes = fs::read(&database).expect("TEST_CODE initializer-owned file remains");
    let competitor_bytes =
        fs::read(&competitor_after_open).expect("TEST_CODE competitor file remains");
    eprintln!(
        "TEST_CODE initializer ABA outcome={outcome} initializer_len={} competitor_len={}",
        initializer_bytes.len(),
        competitor_bytes.len()
    );

    assert_eq!(
        competitor_bytes,
        Vec::<u8>::new(),
        "SQLite-bound competitor file must remain untouched; outcome={outcome}"
    );
    if result.is_ok() {
        assert!(initializer_bytes.len() >= 100);
        assert_eq!(&initializer_bytes[..16], b"SQLite format 3\0");
        assert_eq!((initializer_bytes[18], initializer_bytes[19]), (1, 1));
    }
}

#[test]
fn w15_journal_mode_change_after_header_check_is_rejected_before_sqlite_side_effects() {
    let details = Connection::open_in_memory().expect("TEST_CODE linked SQLite details");
    let (version, source_id): (String, String) = details
        .query_row("SELECT sqlite_version(),sqlite_source_id()", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .expect("TEST_CODE query linked SQLite details");
    eprintln!("TEST_CODE linked SQLite version={version} source_id={source_id}");

    let (reader_root, reader_database, namespace) =
        initialized_database("reader-mode-race.sqlite3");
    let mut reader_baseline = None;
    let reader = with_read_only_hooks(
        &reader_database,
        &namespace,
        || {
            let connection =
                Connection::open(&reader_database).expect("TEST_CODE reader WAL racer");
            let mode: String = connection
                .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
                .expect("TEST_CODE reader switches to WAL");
            assert_eq!(mode, "wal");
            drop(connection);
            reader_baseline = Some(directory_bytes(reader_root.path()));
        },
        || {},
        |_| Ok::<(), ReadinessSchemaError>(()),
    );
    let reader_after_open = directory_bytes(reader_root.path());
    let reader_rejected = reader.is_err();
    let reader_outcome = match &reader {
        Ok(_) => "Ok".to_owned(),
        Err(error) => format!("Err({error:?}; {error})"),
    };
    drop(reader);
    let reader_baseline = reader_baseline.expect("TEST_CODE reader WAL baseline");
    let reader_unchanged = reader_after_open == reader_baseline;
    eprintln!(
        "TEST_CODE reader mode race outcome={reader_outcome} rejected={reader_rejected} directory_unchanged={reader_unchanged}"
    );

    let (writer_root, writer_database, namespace) =
        initialized_database("writer-mode-race.sqlite3");
    let mut writer_baseline = None;
    let writer = with_write_transaction_hooks(
        &writer_database,
        &namespace,
        || {
            let connection =
                Connection::open(&writer_database).expect("TEST_CODE writer WAL racer");
            let mode: String = connection
                .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
                .expect("TEST_CODE writer switches to WAL");
            assert_eq!(mode, "wal");
            drop(connection);
            writer_baseline = Some(directory_bytes(writer_root.path()));
        },
        || {},
        |_| Ok::<(), ReadinessSchemaError>(()),
    );
    let writer_after_open = directory_bytes(writer_root.path());
    let writer_rejected = writer.is_err();
    let writer_outcome = match &writer {
        Ok(_) => "Ok".to_owned(),
        Err(error) => format!("Err({error:?}; {error})"),
    };
    drop(writer);
    let writer_baseline = writer_baseline.expect("TEST_CODE writer WAL baseline");
    let writer_unchanged = writer_after_open == writer_baseline;
    eprintln!(
        "TEST_CODE writer mode race outcome={writer_outcome} rejected={writer_rejected} directory_unchanged={writer_unchanged}"
    );

    assert!(
        reader_rejected && reader_unchanged && writer_rejected && writer_unchanged,
        "mode race must reject without side effects: reader=({reader_outcome}, unchanged={reader_unchanged}), writer=({writer_outcome}, unchanged={writer_unchanged})"
    );
}

#[test]
fn w15_controlled_reader_lock_blocks_wal_until_callback_returns() {
    let (root, database, namespace) = initialized_database("reader-lock.sqlite3");
    with_read_only_hooks(
        &database,
        &namespace,
        || {},
        || {
            let before = directory_bytes(root.path());
            let racer = Connection::open(&database).expect("TEST_CODE competing SQLite connection");
            let switch =
                racer.query_row("PRAGMA journal_mode=WAL", [], |row| row.get::<_, String>(0));
            let switched_to_wal = matches!(switch.as_deref(), Ok("wal"));
            drop(racer);
            assert!(!switched_to_wal, "SHARED lock must block WAL transition");
            assert_eq!(directory_bytes(root.path()), before);
        },
        |_| Ok::<(), ReadinessSchemaError>(()),
    )
    .expect("TEST_CODE controlled reader callback");

    let released = Connection::open(&database).expect("TEST_CODE post-reader SQLite connection");
    let mode: String = released
        .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
        .expect("TEST_CODE WAL succeeds after reader closes");
    assert_eq!(mode, "wal");
}

#[test]
fn w15_write_callback_commits_or_rolls_back_and_releases_lock() {
    let (_root, database, namespace) = initialized_database("writer-transaction.sqlite3");
    let failed = with_write_transaction(&database, &namespace, |connection| {
        insert_linked_pair(connection);
        Err::<(), ReadinessSchemaError>(schema_error("TEST_CODE requested rollback"))
    });
    assert!(failed.is_err());
    let count = with_read_only(&database, &namespace, |connection| {
        connection
            .query_row(
                "SELECT count(*) FROM operational_readiness_snapshot",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|_| schema_error("TEST_CODE rollback count"))
    })
    .expect("TEST_CODE read rollback result");
    assert_eq!(count, 0);

    with_write_transaction(&database, &namespace, |connection| {
        insert_linked_pair(connection);
        Ok::<(), ReadinessSchemaError>(())
    })
    .expect("TEST_CODE committed writer callback");
    let count = with_read_only(&database, &namespace, |connection| {
        connection
            .query_row(
                "SELECT count(*) FROM operational_readiness_snapshot",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|_| schema_error("TEST_CODE committed count"))
    })
    .expect("TEST_CODE read committed result");
    assert_eq!(count, 1);

    let released = Connection::open(&database).expect("TEST_CODE post-writer SQLite connection");
    let mode: String = released
        .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
        .expect("TEST_CODE WAL succeeds after writer closes");
    assert_eq!(mode, "wal");
}

#[test]
fn w15_hot_rollback_journal_is_rejected_without_recovery_side_effects() {
    let (root, database, namespace) = initialized_database("hot-journal.sqlite3");
    let mut journal_name = database.as_os_str().to_os_string();
    journal_name.push("-journal");
    let journal = PathBuf::from(journal_name);
    let fault = Connection::open(&database).expect("TEST_CODE hot-journal fault connection");
    fault
        .execute_batch(
            "PRAGMA synchronous=FULL; PRAGMA cache_size=1; PRAGMA cache_spill=ON; \
             BEGIN IMMEDIATE; \
             CREATE TABLE hot_journal_fault(value BLOB); \
             INSERT INTO hot_journal_fault(value) VALUES(zeroblob(262144));",
        )
        .expect("TEST_CODE write rollback journal");
    let journal_bytes = fs::read(&journal).expect("TEST_CODE capture rollback journal");
    assert!(journal_bytes.len() > 512);
    assert_eq!(
        &journal_bytes[..8],
        &[0xd9, 0xd5, 0x05, 0xf9, 0x20, 0xa1, 0x63, 0xd7]
    );
    fault
        .execute_batch("ROLLBACK;")
        .expect("TEST_CODE close live transaction");
    drop(fault);
    fs::write(&journal, &journal_bytes).expect("TEST_CODE restore hot rollback journal");
    let before = directory_bytes(root.path());

    assert!(with_read_only(
        &database,
        &namespace,
        |_| Ok::<(), ReadinessSchemaError>(())
    )
    .is_err());
    assert_eq!(directory_bytes(root.path()), before);
}
