use std::path::PathBuf;

use rusqlite::{Connection, OpenFlags, TransactionBehavior};
use tempfile::TempDir;

use super::{ChainPostClose, ChainPostCloseError};
use crate::push_foundation::{BusinessIntentStore, FoundationSchemaMigration};

/// Owns the entire synthetic database lifetime. No caller-selected path or store is accepted.
struct TestBusinessFixture {
    store: Option<BusinessIntentStore>,
    directory: TempDir,
}

impl TestBusinessFixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let database = directory
            .path()
            .canonicalize()
            .unwrap()
            .join("business.sqlite");
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch("PRAGMA application_id=1413829460; PRAGMA user_version=73;")
            .unwrap();
        let foundation = FoundationSchemaMigration::bundled().unwrap();
        assert_eq!(
            foundation.ddl_sha256().as_str(),
            "4bac8e58caa2f5d2362137b5e96dd087649044f45484a1284dbd7e1fd7baa953"
        );
        // Only the digest-checked, fixed artifact's single CLI prefix is removed.
        let sql = foundation.ddl_bytes().strip_prefix(b".bail on\n").unwrap();
        connection
            .execute_batch(std::str::from_utf8(sql).unwrap())
            .unwrap();
        connection.close().unwrap();
        let store = BusinessIntentStore::open(&database).unwrap();
        Self {
            store: Some(store),
            directory,
        }
    }

    fn chain_post_close(&mut self) -> ChainPostClose<'_> {
        ChainPostClose {
            store: self.store.as_mut().unwrap(),
        }
    }

    fn database(&self) -> PathBuf {
        self.directory
            .path()
            .canonicalize()
            .unwrap()
            .join("business.sqlite")
    }

    fn persistent_bytes(&self) -> Vec<u8> {
        std::fs::read(self.database()).unwrap()
    }

    fn pragma_i64(&self, sql: &str) -> i64 {
        self.store
            .as_ref()
            .unwrap()
            .connection
            .query_row(sql, [], |row| row.get(0))
            .unwrap()
    }

    fn extension_metadata(
        &self,
    ) -> (
        Vec<(i64, i64, String, String)>,
        Vec<(String, String, Vec<u8>)>,
    ) {
        let connection = &self.store.as_ref().unwrap().connection;
        let headers = connection
            .prepare(
                "SELECT schema_version,artifact_codec_version,description,bundle_sha256 \
                 FROM chain_post_close_schema ORDER BY schema_version",
            )
            .unwrap()
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let objects = connection
            .prepare(
                "SELECT name,object_type,CAST(definition AS BLOB) \
                 FROM chain_post_close_objects ORDER BY name",
            )
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        (headers, objects)
    }

    fn reopen(&mut self) {
        let store = self.store.take().unwrap();
        store.connection.close().unwrap();
        let database = self
            .directory
            .path()
            .canonicalize()
            .unwrap()
            .join("business.sqlite");
        self.store = Some(BusinessIntentStore::open(&database).unwrap());
    }

    fn catalog(&self) -> Vec<(String, String, String, Option<Vec<u8>>)> {
        self.store
            .as_ref()
            .unwrap()
            .connection
            .prepare("SELECT type,name,tbl_name,CAST(sql AS BLOB) FROM sqlite_schema ORDER BY name")
            .unwrap()
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    fn foundation_catalog(&self) -> Vec<(String, String, Vec<u8>, Vec<u8>)> {
        self.store
            .as_ref()
            .unwrap()
            .connection
            .prepare(
                "SELECT r.name,r.object_type,CAST(r.definition AS BLOB),CAST(s.sql AS BLOB) \
                 FROM push_foundation_objects r JOIN sqlite_schema s \
                 ON s.name=r.name AND s.type=r.object_type ORDER BY r.name",
            )
            .unwrap()
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    fn assert_foundation_and_application_contract(&self) {
        let connection = &self.store.as_ref().unwrap().connection;
        let headers: Vec<(i64, String, String)> = connection
            .prepare("SELECT version,description,schema_signature FROM push_foundation_schema")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            headers,
            vec![(
                1,
                "push-foundation-v1".to_owned(),
                "dd5f49a1f4e02ee1d585793cc2eff9c8b98b087b2ffd267f40c873c83960ecdd".to_owned(),
            )]
        );
        let registered: i64 = connection
            .query_row("SELECT count(*) FROM push_foundation_objects", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(registered, 25);
        let catalog = self.foundation_catalog();
        assert_eq!(catalog.len(), 25);
        assert!(catalog
            .iter()
            .all(|(_, _, registered, actual)| registered == actual));
        for (pragma, expected) in [
            ("PRAGMA application_id", 1413829460_i64),
            ("PRAGMA user_version", 73_i64),
        ] {
            let actual: i64 = connection.query_row(pragma, [], |row| row.get(0)).unwrap();
            assert_eq!(actual, expected);
        }
    }
}

#[test]
fn installed_schema_survives_owned_business_database_reopen() {
    let mut fixture = TestBusinessFixture::new();
    fixture.assert_foundation_and_application_contract();
    let foundation_before = fixture.foundation_catalog();
    let catalog_before = fixture.catalog();
    let database = fixture.directory.path().join("business.sqlite");
    let bytes_before = std::fs::read(&database).unwrap();

    assert!(matches!(
        fixture.chain_post_close().verify_schema(),
        Err(ChainPostCloseError::NotInstalled)
    ));
    assert_eq!(fixture.catalog(), catalog_before);
    assert_eq!(std::fs::read(&database).unwrap(), bytes_before);

    let installed = fixture.chain_post_close().install_schema().unwrap();
    assert_eq!(installed.schema_version(), 1);
    assert_eq!(installed.artifact_codec_version(), 1);
    let installed_digest = installed.ddl_sha256().clone();
    let installed_catalog = fixture.catalog();
    assert!(installed_catalog
        .iter()
        .any(|(kind, name, _, _)| { kind == "table" && name == "chain_post_close_schema" }));
    assert_ne!(installed_catalog, catalog_before);
    fixture.assert_foundation_and_application_contract();
    assert_eq!(fixture.foundation_catalog(), foundation_before);

    fixture.reopen();
    let reopened_bytes = std::fs::read(&database).unwrap();
    let verified = fixture.chain_post_close().verify_schema().unwrap();
    assert_eq!(verified.schema_version(), 1);
    assert_eq!(verified.artifact_codec_version(), 1);
    assert_eq!(verified.ddl_sha256(), &installed_digest);
    assert_eq!(fixture.catalog(), installed_catalog);
    assert_eq!(std::fs::read(&database).unwrap(), reopened_bytes);
    fixture.assert_foundation_and_application_contract();
    assert_eq!(fixture.foundation_catalog(), foundation_before);
}

#[test]
fn malformed_or_case_aliased_schema_is_rejected_without_repair() {
    let cases = [
        (
            "lowercase partial installation",
            false,
            "CREATE TABLE chain_post_close_schema(unexpected TEXT);",
        ),
        (
            "missing protection trigger",
            true,
            "DROP TRIGGER chain_post_close_schema_update;",
        ),
        (
            "unregistered foreign-prefixed attached index",
            true,
            "CREATE INDEX unrelated_schema_index ON chain_post_close_schema(description);",
        ),
        (
            "forged trigger and matching registry",
            true,
            "DROP TRIGGER chain_post_close_objects_update; \
             DROP TRIGGER chain_post_close_schema_update; \
             CREATE TRIGGER chain_post_close_schema_update \
             BEFORE UPDATE ON chain_post_close_schema WHEN 0 \
             BEGIN SELECT RAISE(ABORT, 'TEST_CODE forged guard'); END; \
             UPDATE chain_post_close_objects \
             SET definition=(SELECT sql FROM sqlite_schema \
                 WHERE name='chain_post_close_schema_update') \
             WHERE name='chain_post_close_schema_update';",
        ),
        (
            "uppercase partial installation",
            false,
            "CREATE TABLE CHAIN_POST_CLOSE_SCHEMA(unexpected TEXT);",
        ),
    ];

    for (case, install_first, corruption) in cases {
        let mut fixture = TestBusinessFixture::new();
        let foundation_before = fixture.foundation_catalog();
        if install_first {
            fixture.chain_post_close().install_schema().unwrap();
        }
        {
            let connection = &fixture.store.as_ref().unwrap().connection;
            let registry_guard = if case == "forged trigger and matching registry" {
                Some(connection.query_row(
                    "SELECT sql FROM sqlite_schema WHERE name='chain_post_close_objects_update'",
                    [],
                    |row| row.get::<_, String>(0),
                ).unwrap())
            } else {
                None
            };
            connection.execute_batch(corruption).unwrap();
            if let Some(registry_guard) = registry_guard {
                connection.execute_batch(&registry_guard).unwrap();
                // This attack is internally self-consistent; only the independent bundle differs.
                let matching: i64 = connection
                    .query_row(
                        "SELECT count(*) FROM chain_post_close_objects r JOIN sqlite_schema s \
                     ON s.name=r.name AND s.type=r.object_type \
                     AND CAST(s.sql AS BLOB)=CAST(r.definition AS BLOB)",
                        [],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(matching, 8, "{case}: attack setup");
            }
        }
        fixture.reopen();
        let catalog_before = fixture.catalog();
        let database = fixture.directory.path().join("business.sqlite");
        let bytes_before = std::fs::read(&database).unwrap();

        let verified = fixture.chain_post_close().verify_schema();
        assert_eq!(fixture.catalog(), catalog_before, "{case}: verify catalog");
        assert_eq!(
            std::fs::read(&database).unwrap(),
            bytes_before,
            "{case}: verify bytes"
        );
        assert_eq!(
            verified,
            Err(ChainPostCloseError::SchemaRejected),
            "{case}: verify result"
        );

        let installed = fixture.chain_post_close().install_schema();
        assert_eq!(fixture.catalog(), catalog_before, "{case}: install catalog");
        assert_eq!(
            std::fs::read(&database).unwrap(),
            bytes_before,
            "{case}: install bytes"
        );
        assert_eq!(
            installed,
            Err(ChainPostCloseError::SchemaRejected),
            "{case}: install result"
        );
        fixture.assert_foundation_and_application_contract();
        assert_eq!(
            fixture.foundation_catalog(),
            foundation_before,
            "{case}: Foundation"
        );
    }
}

#[test]
fn production_entry_refuses_even_an_owned_installed_database() {
    let mut fixture = TestBusinessFixture::new();
    let catalog_before = fixture.catalog();
    let bytes_before = fixture.persistent_bytes();

    assert!(matches!(
        fixture.store.as_mut().unwrap().chain_post_close(),
        Err(ChainPostCloseError::ProductionRefused)
    ));
    assert_eq!(fixture.catalog(), catalog_before);
    assert_eq!(fixture.persistent_bytes(), bytes_before);

    fixture.chain_post_close().install_schema().unwrap();
    let catalog_installed = fixture.catalog();
    let bytes_installed = fixture.persistent_bytes();
    assert!(matches!(
        fixture.store.as_mut().unwrap().chain_post_close(),
        Err(ChainPostCloseError::ProductionRefused)
    ));
    assert_eq!(fixture.catalog(), catalog_installed);
    assert_eq!(fixture.persistent_bytes(), bytes_installed);
    fixture.assert_foundation_and_application_contract();
}

#[test]
fn schema_operations_preserve_read_only_and_existing_transaction() {
    let mut missing = TestBusinessFixture::new();
    missing
        .store
        .as_ref()
        .unwrap()
        .connection
        .execute_batch("PRAGMA query_only=ON;")
        .unwrap();
    let missing_catalog = missing.catalog();
    let missing_bytes = missing.persistent_bytes();
    assert_eq!(
        missing.chain_post_close().verify_schema(),
        Err(ChainPostCloseError::NotInstalled)
    );
    assert_eq!(missing.pragma_i64("PRAGMA query_only"), 1);
    assert_eq!(
        missing.chain_post_close().install_schema(),
        Err(ChainPostCloseError::ConnectionSafeguardFailed)
    );
    assert_eq!(missing.pragma_i64("PRAGMA query_only"), 1);
    assert_eq!(missing.catalog(), missing_catalog);
    assert_eq!(missing.persistent_bytes(), missing_bytes);

    let mut installed = TestBusinessFixture::new();
    let installed_receipt = installed.chain_post_close().install_schema().unwrap();
    installed
        .store
        .as_ref()
        .unwrap()
        .connection
        .execute_batch("PRAGMA query_only=ON;")
        .unwrap();
    let installed_catalog = installed.catalog();
    let installed_bytes = installed.persistent_bytes();
    assert_eq!(
        installed.chain_post_close().verify_schema().unwrap(),
        installed_receipt
    );
    assert_eq!(installed.pragma_i64("PRAGMA query_only"), 1);
    assert_eq!(
        installed.chain_post_close().install_schema(),
        Err(ChainPostCloseError::ConnectionSafeguardFailed)
    );
    assert_eq!(installed.pragma_i64("PRAGMA query_only"), 1);
    assert_eq!(installed.catalog(), installed_catalog);
    assert_eq!(installed.persistent_bytes(), installed_bytes);

    let mut damaged_read_only = TestBusinessFixture::new();
    damaged_read_only
        .chain_post_close()
        .install_schema()
        .unwrap();
    damaged_read_only
        .store
        .as_ref()
        .unwrap()
        .connection
        .execute_batch("DROP TRIGGER chain_post_close_schema_update; PRAGMA query_only=ON;")
        .unwrap();
    let damaged_catalog = damaged_read_only.catalog();
    let damaged_bytes = damaged_read_only.persistent_bytes();
    assert_eq!(
        damaged_read_only.chain_post_close().verify_schema(),
        Err(ChainPostCloseError::SchemaRejected)
    );
    assert_eq!(damaged_read_only.pragma_i64("PRAGMA query_only"), 1);
    assert_eq!(
        damaged_read_only.chain_post_close().install_schema(),
        Err(ChainPostCloseError::ConnectionSafeguardFailed)
    );
    assert_eq!(damaged_read_only.pragma_i64("PRAGMA query_only"), 1);
    assert_eq!(damaged_read_only.catalog(), damaged_catalog);
    assert_eq!(damaged_read_only.persistent_bytes(), damaged_bytes);

    let mut damaged_writable = TestBusinessFixture::new();
    damaged_writable
        .chain_post_close()
        .install_schema()
        .unwrap();
    damaged_writable
        .store
        .as_ref()
        .unwrap()
        .connection
        .execute_batch("DROP TRIGGER chain_post_close_schema_update;")
        .unwrap();
    let damaged_writable_catalog = damaged_writable.catalog();
    let damaged_writable_bytes = damaged_writable.persistent_bytes();
    assert_eq!(
        damaged_writable.chain_post_close().verify_schema(),
        Err(ChainPostCloseError::SchemaRejected)
    );
    assert_eq!(damaged_writable.pragma_i64("PRAGMA query_only"), 0);
    assert_eq!(damaged_writable.catalog(), damaged_writable_catalog);
    assert_eq!(damaged_writable.persistent_bytes(), damaged_writable_bytes);

    let mut caller_transaction = TestBusinessFixture::new();
    let receipt = caller_transaction
        .chain_post_close()
        .install_schema()
        .unwrap();
    let transaction_catalog = caller_transaction.catalog();
    let transaction_bytes = caller_transaction.persistent_bytes();
    caller_transaction
        .store
        .as_ref()
        .unwrap()
        .connection
        .execute_batch("BEGIN DEFERRED;")
        .unwrap();
    assert!(!caller_transaction
        .store
        .as_ref()
        .unwrap()
        .connection
        .is_autocommit());
    assert_eq!(
        caller_transaction.chain_post_close().verify_schema(),
        Err(ChainPostCloseError::ConnectionSafeguardFailed)
    );
    assert!(!caller_transaction
        .store
        .as_ref()
        .unwrap()
        .connection
        .is_autocommit());
    assert_eq!(
        caller_transaction.chain_post_close().install_schema(),
        Err(ChainPostCloseError::ConnectionSafeguardFailed)
    );
    assert!(!caller_transaction
        .store
        .as_ref()
        .unwrap()
        .connection
        .is_autocommit());
    assert_eq!(caller_transaction.catalog(), transaction_catalog);
    assert_eq!(caller_transaction.persistent_bytes(), transaction_bytes);
    caller_transaction
        .store
        .as_ref()
        .unwrap()
        .connection
        .execute_batch("ROLLBACK;")
        .unwrap();
    assert!(caller_transaction
        .store
        .as_ref()
        .unwrap()
        .connection
        .is_autocommit());
    assert_eq!(
        caller_transaction
            .chain_post_close()
            .verify_schema()
            .unwrap(),
        receipt
    );
}

#[test]
fn future_header_versions_are_rejected_without_repair() {
    for (case, mutation, expected_header) in [
        (
            "schema version",
            "UPDATE chain_post_close_schema SET schema_version=2;",
            (2, 1),
        ),
        (
            "artifact codec version",
            "UPDATE chain_post_close_schema SET artifact_codec_version=2;",
            (1, 2),
        ),
    ] {
        let mut fixture = TestBusinessFixture::new();
        fixture.chain_post_close().install_schema().unwrap();
        let catalog_installed = fixture.catalog();
        let foundation_before = fixture.foundation_catalog();
        {
            let connection = &fixture.store.as_ref().unwrap().connection;
            let update_guard: String = connection
                .query_row(
                    "SELECT sql FROM sqlite_schema \
                     WHERE type='trigger' AND name='chain_post_close_schema_update'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            connection
                .execute_batch(
                    "DROP TRIGGER chain_post_close_schema_update; \
                     PRAGMA ignore_check_constraints=ON;",
                )
                .unwrap();
            connection.execute_batch(mutation).unwrap();
            connection.execute_batch(&update_guard).unwrap();
            connection
                .execute_batch("PRAGMA ignore_check_constraints=OFF;")
                .unwrap();
            let header: (i64, i64) = connection
                .query_row(
                    "SELECT schema_version,artifact_codec_version \
                     FROM chain_post_close_schema",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(header, expected_header, "{case}: setup header");
        }
        assert_eq!(
            fixture.pragma_i64("PRAGMA ignore_check_constraints"),
            0,
            "{case}: check enforcement"
        );
        assert_eq!(fixture.catalog(), catalog_installed, "{case}: catalog");
        fixture.reopen();
        let catalog_before = fixture.catalog();
        let bytes_before = fixture.persistent_bytes();

        assert_eq!(
            fixture.chain_post_close().verify_schema(),
            Err(ChainPostCloseError::UnsupportedVersion),
            "{case}: verify"
        );
        assert_eq!(fixture.catalog(), catalog_before, "{case}: verify catalog");
        assert_eq!(
            fixture.persistent_bytes(),
            bytes_before,
            "{case}: verify bytes"
        );
        assert_eq!(
            fixture.chain_post_close().install_schema(),
            Err(ChainPostCloseError::UnsupportedVersion),
            "{case}: install"
        );
        assert_eq!(fixture.catalog(), catalog_before, "{case}: install catalog");
        assert_eq!(
            fixture.persistent_bytes(),
            bytes_before,
            "{case}: install bytes"
        );
        fixture.assert_foundation_and_application_contract();
        assert_eq!(
            fixture.foundation_catalog(),
            foundation_before,
            "{case}: Foundation"
        );
    }
}

#[test]
fn sealed_metadata_rejects_mutation_and_reinstallation_is_read_only() {
    let mut fixture = TestBusinessFixture::new();
    let installed = fixture.chain_post_close().install_schema().unwrap();
    let metadata_before = fixture.extension_metadata();
    let catalog_before = fixture.catalog();
    let bytes_before = fixture.persistent_bytes();
    let mutations = [
        (
            "schema insert",
            "INSERT INTO chain_post_close_schema \
             SELECT schema_version,artifact_codec_version,description,bundle_sha256 \
             FROM chain_post_close_schema;",
        ),
        (
            "schema update",
            "UPDATE chain_post_close_schema SET description=description;",
        ),
        ("schema delete", "DELETE FROM chain_post_close_schema;"),
        (
            "schema replace",
            "INSERT OR REPLACE INTO chain_post_close_schema \
             SELECT schema_version,artifact_codec_version,description,bundle_sha256 \
             FROM chain_post_close_schema;",
        ),
        (
            "objects insert",
            "INSERT INTO chain_post_close_objects \
             SELECT name,object_type,definition FROM chain_post_close_objects LIMIT 1;",
        ),
        (
            "objects update",
            "UPDATE chain_post_close_objects SET definition=definition \
             WHERE name='chain_post_close_schema';",
        ),
        (
            "objects delete",
            "DELETE FROM chain_post_close_objects WHERE name='chain_post_close_schema';",
        ),
        (
            "objects replace",
            "INSERT OR REPLACE INTO chain_post_close_objects \
             SELECT name,object_type,definition FROM chain_post_close_objects LIMIT 1;",
        ),
    ];
    for (case, mutation) in mutations {
        assert!(
            fixture
                .store
                .as_ref()
                .unwrap()
                .connection
                .execute_batch(mutation)
                .is_err(),
            "{case}"
        );
        assert_eq!(
            fixture.extension_metadata(),
            metadata_before,
            "{case}: rows"
        );
        assert_eq!(fixture.catalog(), catalog_before, "{case}: catalog");
        assert_eq!(fixture.persistent_bytes(), bytes_before, "{case}: bytes");
    }

    assert_eq!(
        fixture.chain_post_close().install_schema().unwrap(),
        installed
    );
    assert_eq!(fixture.extension_metadata(), metadata_before);
    assert_eq!(fixture.catalog(), catalog_before);
    assert_eq!(fixture.persistent_bytes(), bytes_before);

    fixture.reopen();
    assert_eq!(
        fixture.chain_post_close().verify_schema().unwrap(),
        installed
    );
    assert_eq!(fixture.extension_metadata(), metadata_before);
    assert_eq!(fixture.catalog(), catalog_before);
}

#[test]
fn installation_commit_contention_rolls_back_the_complete_extension() {
    let mut fixture = TestBusinessFixture::new();
    let journal_mode: String = fixture
        .store
        .as_ref()
        .unwrap()
        .connection
        .query_row("PRAGMA journal_mode=DELETE", [], |row| row.get(0))
        .unwrap();
    assert_eq!(journal_mode, "delete");
    let catalog_before = fixture.catalog();
    let foundation_before = fixture.foundation_catalog();
    let bytes_before = fixture.persistent_bytes();

    let mut reader = Connection::open_with_flags(
        fixture.database(),
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .unwrap();
    let reader_transaction = reader
        .transaction_with_behavior(TransactionBehavior::Deferred)
        .unwrap();
    let foundation_rows: i64 = reader_transaction
        .query_row("SELECT count(*) FROM push_foundation_schema", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(foundation_rows, 1);

    assert_eq!(
        fixture.chain_post_close().install_schema(),
        Err(ChainPostCloseError::StorageFailed {
            operation: "commit"
        })
    );
    assert!(fixture.store.as_ref().unwrap().connection.is_autocommit());
    assert_eq!(fixture.pragma_i64("PRAGMA query_only"), 0);
    assert_eq!(fixture.catalog(), catalog_before);
    assert_eq!(fixture.persistent_bytes(), bytes_before);
    fixture.assert_foundation_and_application_contract();
    assert_eq!(fixture.foundation_catalog(), foundation_before);

    reader_transaction.rollback().unwrap();
    reader.close().unwrap();
    fixture.reopen();
    assert_eq!(
        fixture.chain_post_close().verify_schema(),
        Err(ChainPostCloseError::NotInstalled)
    );
    fixture.chain_post_close().install_schema().unwrap();
    fixture.chain_post_close().verify_schema().unwrap();
    fixture.assert_foundation_and_application_contract();
}
