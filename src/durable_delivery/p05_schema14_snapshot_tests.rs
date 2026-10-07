//! Operator preflight checks reuse actual counted fixtures; no live namespace.
use super::*;
use crate::durable_delivery::inspect_schema14_extensions;
use rusqlite::{DatabaseName, OpenFlags};
use std::path::{Path, PathBuf};

fn snapshot(fixture: &Fixture) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("TEST_CODE_schema14_snapshot.sqlite3");
    let source =
        Connection::open_with_flags(&fixture.database_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    source.execute_batch("BEGIN").unwrap();
    let image = source.serialize(DatabaseName::Main).unwrap();
    std::fs::write(&path, &*image).unwrap();
    drop(image);
    source.execute_batch("ROLLBACK").unwrap();
    (dir, path)
}

fn readonly(path: &Path) -> Connection {
    let mut uri = url::Url::from_file_path(path).unwrap();
    uri.query_pairs_mut()
        .append_pair("mode", "ro")
        .append_pair("immutable", "1");
    Connection::open_with_flags(
        uri.as_str(),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .unwrap()
}

#[test]
fn schema14_snapshot_nonempty_accepted_owner_history_is_observed_without_writes_or_resend() {
    let f = Fixture::new("SCHEMA14_SNAPSHOT_COMPLETE");
    let (_operational_dir, db) = operational();
    let draft = start(&f, &db, true);
    let intent = complete_intent(&f, &db, &draft);
    let sink = deliver_all(&f, &db, &intent, 23, &Append::default());
    assert!(finalize(&f, DATE).completion_identity().is_some());
    let (dir, path) = snapshot(&f);
    let before = std::fs::read(&path).unwrap();
    let mut c = readonly(&path);
    let report = inspect_schema14_extensions(&mut c).unwrap();
    assert_eq!(report.schema_version, 14);
    assert_eq!(report.extension_row_counts.len(), 17);
    let count = |name| {
        report
            .extension_row_counts
            .iter()
            .find(|(table, _)| table == name)
            .unwrap()
            .1
    };
    assert_eq!(count("p05_s2_child_owners"), 2);
    assert_eq!(count("p05_s2_completion_receipts"), 1);
    assert_eq!(count("p05_baseline_origins"), 2);
    assert_eq!(report.extension_catalog_sha256.len(), 64);
    assert!(report.scope.contains("No full BR-194"));
    assert!(c.is_autocommit());
    assert_eq!(
        c.query_row("SELECT total_changes()", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(c
        .execute("DELETE FROM p05_s2_completion_heads", [])
        .is_err());
    drop(c);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    assert_eq!(sink.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn schema14_snapshot_sql_valid_revision_without_actual_event_is_rejected_without_healing() {
    let f = Fixture::new("SCHEMA14_SNAPSHOT_CHAIN");
    let (_dir, db) = operational();
    let draft = start(&f, &db, false);
    complete_intent(&f, &db, &draft);
    let (_snapshot_dir, path) = snapshot(&f);
    let c = Connection::open(&path).unwrap();
    c.execute(
        "UPDATE p05_unit_heads SET mutation_revision=mutation_revision+1",
        [],
    )
    .unwrap();
    drop(c);
    let before = std::fs::read(&path).unwrap();
    let mut c = readonly(&path);
    let error = inspect_schema14_extensions(&mut c).unwrap_err().to_string();
    assert!(error.contains("Unit current revision"), "{error}");
    assert!(c.is_autocommit());
    assert_eq!(
        c.query_row("SELECT mutation_revision FROM p05_unit_heads", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap(),
        4
    );
    drop(c);
    assert_eq!(std::fs::read(path).unwrap(), before);
}

#[test]
fn schema14_snapshot_version_label_does_not_authorize_migration_or_upgrade() {
    let f = Fixture::new("SCHEMA14_SNAPSHOT_VERSION");
    let (_dir, path) = snapshot(&f);
    Connection::open(&path)
        .unwrap()
        .pragma_update(None, "user_version", 9)
        .unwrap();
    let before = std::fs::read(&path).unwrap();
    let mut c = readonly(&path);
    let error = inspect_schema14_extensions(&mut c).unwrap_err().to_string();
    assert!(error.contains("required version 14"), "{error}");
    assert_eq!(
        c.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        9
    );
    drop(c);
    assert_eq!(std::fs::read(path).unwrap(), before);
}

#[test]
fn schema14_snapshot_missing_and_unknown_extension_objects_are_rejected_without_repair() {
    for ddl in [
        "DROP TRIGGER p05_s2_new_family_decision",
        "CREATE TABLE p05_s2_unknown(bytes BLOB)",
    ] {
        let f = Fixture::new("SCHEMA14_SNAPSHOT_CATALOG");
        let (_dir, path) = snapshot(&f);
        Connection::open(&path).unwrap().execute_batch(ddl).unwrap();
        let before = std::fs::read(&path).unwrap();
        let mut c = readonly(&path);
        let error = inspect_schema14_extensions(&mut c).unwrap_err().to_string();
        assert!(error.contains("P05 S2 catalog"), "{error}");
        drop(c);
        assert_eq!(std::fs::read(path).unwrap(), before);
    }
}

#[test]
fn schema14_snapshot_temp_shadow_and_attachment_cannot_replace_the_actual_main_schema() {
    let f = Fixture::new("SCHEMA14_SNAPSHOT_SHADOW");
    let (_dir, path) = snapshot(&f);
    for ddl in [
        "CREATE TEMP TABLE delivery_decisions(state TEXT)",
        "ATTACH ':memory:' AS foreign_db",
    ] {
        let mut c = readonly(&path);
        c.execute_batch(ddl).unwrap();
        let error = inspect_schema14_extensions(&mut c).unwrap_err().to_string();
        assert!(
            error.contains("attachments and temporary objects"),
            "{error}"
        );
        assert!(c.is_autocommit());
    }
}

#[test]
fn schema14_snapshot_writable_query_only_and_caller_transaction_are_not_admitted() {
    let f = Fixture::new("SCHEMA14_SNAPSHOT_CONNECTION");
    let (_dir, path) = snapshot(&f);
    let mut writable = Connection::open(&path).unwrap();
    writable.pragma_update(None, "query_only", true).unwrap();
    assert!(inspect_schema14_extensions(&mut writable)
        .unwrap_err()
        .to_string()
        .contains("idle read-only"));
    let mut c = readonly(&path);
    c.execute_batch("BEGIN").unwrap();
    assert!(inspect_schema14_extensions(&mut c)
        .unwrap_err()
        .to_string()
        .contains("idle read-only"));
    assert!(!c.is_autocommit());
    c.execute_batch("ROLLBACK").unwrap();
}
