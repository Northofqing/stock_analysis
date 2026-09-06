use std::path::Path;

use rusqlite::Connection;

use super::{FoundationMigrationError, FoundationSchemaMigration};

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
