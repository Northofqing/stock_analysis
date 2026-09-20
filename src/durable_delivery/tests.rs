use super::coordinator::{
    install_compound_commit_rollback_test_fault, install_database_bootstrap_test_hook,
    install_process_descriptor_snapshot_test_fault, AttemptLease, DatabaseBootstrapTestPhase,
    DatabaseOperationTestPhase, DeliveredPrecommitTestFault, OpenFileDescriptionProof,
    OperationPostvalidationTestFault, ProcessDescriptorSnapshotTestFault,
};
use super::model::sha256_hex;
use super::*;
use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Write};
use std::ops::Deref;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Barrier, Mutex};

static NEXT_TEST_ID: AtomicUsize = AtomicUsize::new(1);

#[cfg(unix)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FilesystemIdentity {
    device: u64,
    inode: u64,
    file_type: u32,
}

#[cfg(unix)]
impl FilesystemIdentity {
    fn capture(path: &Path) -> std::io::Result<Self> {
        use std::os::unix::fs::MetadataExt;

        let metadata = path.symlink_metadata()?;
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            file_type: metadata.mode() & 0o170_000,
        })
    }
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OwnedPathKind {
    FileOrSymlink,
    Directory,
}

#[cfg(unix)]
#[derive(Debug)]
struct OwnedPath {
    path: PathBuf,
    identity: FilesystemIdentity,
    kind: OwnedPathKind,
}

/// Deletes only exact inodes created inside this test's TEST_CODE namespaces.
///
/// This deliberately never uses `remove_dir_all`: if a path is replaced after
/// capture, cleanup fails closed and leaves the replacement untouched.
#[cfg(unix)]
#[derive(Default, Debug)]
struct OwnedTestPaths {
    entries: RefCell<Vec<OwnedPath>>,
    armed: std::cell::Cell<bool>,
}

#[cfg(unix)]
impl OwnedTestPaths {
    fn new() -> Self {
        Self {
            entries: RefCell::new(Vec::new()),
            armed: std::cell::Cell::new(true),
        }
    }

    fn record(&self, path: impl Into<PathBuf>, kind: OwnedPathKind) {
        let path = path.into();
        let lexical = path
            .strip_prefix(Path::new(env!("CARGO_MANIFEST_DIR")))
            .unwrap_or(&path);
        let mut components = lexical.components();
        let isolated_test_path = matches!(
            (
                components.next(),
                components.next(),
                components.next(),
            ),
            (
                Some(std::path::Component::Normal(data)),
                Some(std::path::Component::Normal(test)),
                Some(std::path::Component::Normal(test_code)),
            ) if data == "data"
                && test == "test"
                && test_code.to_string_lossy().starts_with("TEST_CODE")
        ) && components
            .all(|component| matches!(component, std::path::Component::Normal(_)));
        assert!(
            isolated_test_path,
            "cleanup ownership is restricted to lexical data/test/TEST_CODE_* paths: {}",
            path.display()
        );
        let identity = FilesystemIdentity::capture(&path)
            .unwrap_or_else(|error| panic!("capture test-owned path {}: {error}", path.display()));
        self.entries.borrow_mut().push(OwnedPath {
            path,
            identity,
            kind,
        });
    }

    fn record_if_present(&self, path: impl Into<PathBuf>, kind: OwnedPathKind) {
        let path = path.into();
        match path.symlink_metadata() {
            Ok(_) => self.record(path, kind),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!(
                "inspect test-owned path before cleanup capture {}: {error}",
                path.display()
            ),
        }
    }

    fn disarm(&self) {
        self.armed.set(false);
    }

    fn clean_now(&self) {
        if !self.armed.replace(false) {
            return;
        }
        let owned_paths = self.entries.borrow_mut().drain(..).collect::<Vec<_>>();
        for owned in owned_paths.into_iter().rev() {
            let Ok(current) = FilesystemIdentity::capture(&owned.path) else {
                continue;
            };
            if current != owned.identity {
                continue;
            }
            match owned.kind {
                OwnedPathKind::FileOrSymlink => {
                    let _ = std::fs::remove_file(&owned.path);
                }
                OwnedPathKind::Directory => {
                    let _ = std::fs::remove_dir(&owned.path);
                }
            }
        }
    }
}

#[cfg(unix)]
impl Drop for OwnedTestPaths {
    fn drop(&mut self) {
        self.clean_now();
    }
}

struct FixtureCoordinator(Option<Arc<DurableDeliveryCoordinator>>);

impl FixtureCoordinator {
    fn take(&mut self) -> Option<Arc<DurableDeliveryCoordinator>> {
        self.0.take()
    }
}

impl Clone for FixtureCoordinator {
    fn clone(&self) -> Self {
        Self(Some(
            self.0
                .as_ref()
                .expect("fixture coordinator is live")
                .clone(),
        ))
    }
}

impl Deref for FixtureCoordinator {
    type Target = DurableDeliveryCoordinator;

    fn deref(&self) -> &Self::Target {
        self.0.as_deref().expect("fixture coordinator is live")
    }
}

struct Fixture {
    database_path: PathBuf,
    coordinator: FixtureCoordinator,
    #[cfg(unix)]
    cleanup: OwnedTestPaths,
    #[cfg(unix)]
    production_storage_before: ProductionStorageSnapshot,
}

impl Fixture {
    fn new(label: &str) -> Self {
        #[cfg(unix)]
        let production_storage_before = ProductionStorageSnapshot::capture();
        let sequence = NEXT_TEST_ID.fetch_add(1, Ordering::SeqCst);
        let test_code = format!(
            "TEST_CODE_BR192_{label}_{}_{}",
            std::process::id(),
            sequence
        );
        let root = PathBuf::from("data/test").join(&test_code);
        #[cfg(unix)]
        let cleanup = OwnedTestPaths::new();
        std::fs::create_dir_all("data/test").expect("create shared lexical test namespace");
        std::fs::create_dir(&root).expect("create unique isolated TEST_CODE root");
        #[cfg(unix)]
        cleanup.record(&root, OwnedPathKind::Directory);
        let database_path = root.join("durable_delivery.sqlite3");
        let owner = format!("owner-{test_code}-0123456789abcdef");
        let config = CoordinatorConfig::test(&database_path, &test_code, owner);
        let coordinator =
            Arc::new(DurableDeliveryCoordinator::open(config).expect("open isolated coordinator"));
        #[cfg(unix)]
        for suffix in ["", "-journal", "-shm", "-wal"] {
            cleanup.record_if_present(
                PathBuf::from(format!("{}{suffix}", database_path.display())),
                OwnedPathKind::FileOrSymlink,
            );
        }
        Self {
            database_path,
            coordinator: FixtureCoordinator(Some(coordinator)),
            #[cfg(unix)]
            cleanup,
            #[cfg(unix)]
            production_storage_before,
        }
    }

    fn second_coordinator(&self, label: &str) -> Arc<DurableDeliveryCoordinator> {
        let test_code = self
            .database_path
            .parent()
            .and_then(Path::file_name)
            .and_then(|value| value.to_str())
            .expect("test root identity");
        Arc::new(
            DurableDeliveryCoordinator::open(CoordinatorConfig::test(
                &self.database_path,
                test_code,
                format!("owner-second-{label}-0123456789abcdef"),
            ))
            .expect("open second coordinator"),
        )
    }

    fn query_i64(&self, sql: &str) -> i64 {
        Connection::open(&self.database_path)
            .expect("open read connection")
            .query_row(sql, [], |row| row.get(0))
            .expect("query scalar")
    }

    fn query_strings(&self, sql: &str) -> Vec<String> {
        let connection = Connection::open(&self.database_path).expect("open read connection");
        let mut statement = connection.prepare(sql).expect("prepare");
        statement
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query")
            .collect::<std::result::Result<Vec<_>, _>>()
            .expect("collect")
    }

    fn query_blob(&self, sql: &str) -> Vec<u8> {
        Connection::open(&self.database_path)
            .expect("open read connection")
            .query_row(sql, [], |row| row.get(0))
            .expect("query blob")
    }
}

fn authority_table_rows(connection: &Connection, table: &str) -> Vec<Vec<String>> {
    assert!(
        table
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'),
        "TEST_CODE authority table name must be a plain identifier"
    );
    let mut metadata = connection
        .prepare(&format!("PRAGMA table_info({table})"))
        .expect("prepare authority table metadata");
    let columns = metadata
        .query_map([], |row| row.get::<_, String>(1))
        .expect("query authority table metadata")
        .collect::<std::result::Result<Vec<_>, _>>()
        .expect("collect authority table columns");
    assert!(!columns.is_empty(), "authority table {table} must exist");
    let quoted_columns = columns
        .iter()
        .map(|column| format!("\"{}\"", column.replace('"', "\"\"")))
        .collect::<Vec<_>>();
    let column_list = quoted_columns.join(",");
    let mut statement = connection
        .prepare(&format!(
            "SELECT {column_list} FROM {table} ORDER BY {column_list}"
        ))
        .expect("prepare authority table snapshot");
    let column_count = statement.column_count();
    statement
        .query_map([], |row| {
            (0..column_count)
                .map(|index| {
                    let value = row.get_ref(index)?;
                    Ok(match value {
                        rusqlite::types::ValueRef::Null => "null".to_owned(),
                        rusqlite::types::ValueRef::Integer(value) => {
                            format!("integer:{value}")
                        }
                        rusqlite::types::ValueRef::Real(value) => {
                            format!("real:{:016x}", value.to_bits())
                        }
                        rusqlite::types::ValueRef::Text(value) => {
                            format!("text:{}", hex::encode(value))
                        }
                        rusqlite::types::ValueRef::Blob(value) => {
                            format!("blob:{}", hex::encode(value))
                        }
                    })
                })
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .expect("query authority table snapshot")
        .collect::<std::result::Result<Vec<_>, _>>()
        .expect("collect authority table snapshot")
}

fn authority_snapshot(
    connection: &Connection,
    tables: &[&str],
) -> BTreeMap<String, Vec<Vec<String>>> {
    tables
        .iter()
        .map(|table| ((*table).to_owned(), authority_table_rows(connection, table)))
        .collect()
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let can_clean = match self.coordinator.take() {
            Some(coordinator) => {
                let can_clean = Arc::strong_count(&coordinator) == 1;
                drop(coordinator);
                can_clean
            }
            None => true,
        };
        #[cfg(unix)]
        if can_clean {
            for suffix in ["", "-journal", "-shm", "-wal"] {
                self.cleanup.record_if_present(
                    PathBuf::from(format!("{}{suffix}", self.database_path.display())),
                    OwnedPathKind::FileOrSymlink,
                );
            }
        } else {
            self.cleanup.disarm();
        }
        #[cfg(unix)]
        {
            self.cleanup.clean_now();
            self.production_storage_before.assert_unchanged();
        }
    }
}

fn isolation_test_config(database_path: impl Into<PathBuf>, test_code: &str) -> CoordinatorConfig {
    CoordinatorConfig::test(
        database_path,
        test_code,
        format!("owner-{test_code}-0123456789abcdef"),
    )
}

fn initialize_test_schema(connection: &mut Connection) -> Result<()> {
    super::schema::register_sha256_function(connection)?;
    connection.pragma_update(None, "foreign_keys", "OFF")?;
    let transaction = connection.transaction()?;
    super::schema::initialize_schema(&transaction)?;
    transaction.commit()?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    Ok(())
}

fn downgrade_manual_resolution_schema_for_test(connection: &mut Connection, schema_version: i64) {
    assert!(
        matches!(schema_version, 1 | 2),
        "legacy regression supports schema v1 or v2"
    );
    let accepted_audit_columns = if schema_version == 1 {
        ""
    } else {
        "accepted_audit_identity TEXT UNIQUE,
         accepted_audit_append_state TEXT
           CHECK(accepted_audit_append_state IN ('Pending','Appended')),
         accepted_audit_ref TEXT,"
    };
    let accepted_audit_column_names = if schema_version == 1 {
        ""
    } else {
        "accepted_audit_identity,accepted_audit_append_state,accepted_audit_ref,"
    };
    let ddl = format!(
        r#"
        CREATE TABLE manual_resolutions_legacy(
          resolution_identity TEXT PRIMARY KEY,
          decision_identity TEXT NOT NULL UNIQUE REFERENCES delivery_decisions(decision_identity),
          attempt_identity TEXT NOT NULL REFERENCES delivery_attempts(attempt_identity),
          disposition TEXT NOT NULL CHECK(disposition IN ('Accepted','Rejected')),
          operator_identity TEXT NOT NULL,
          reason TEXT NOT NULL,
          evidence_canonical BLOB NOT NULL,
          evidence_sha256 TEXT NOT NULL,
          receipt_canonical BLOB,
          frozen_delivery_audit_canonical BLOB,
          frozen_delivery_audit_sha256 TEXT,
          immutable_audit_ref TEXT NOT NULL,
          {accepted_audit_columns}
          resolved_at TEXT NOT NULL
        );
        INSERT INTO manual_resolutions_legacy(
          resolution_identity,decision_identity,attempt_identity,disposition,
          operator_identity,reason,evidence_canonical,evidence_sha256,
          receipt_canonical,frozen_delivery_audit_canonical,
          frozen_delivery_audit_sha256,immutable_audit_ref,
          {accepted_audit_column_names}
          resolved_at
        )
        SELECT
          resolution_identity,decision_identity,attempt_identity,disposition,
          operator_identity,reason,evidence_canonical,evidence_sha256,
          receipt_canonical,frozen_delivery_audit_canonical,
          frozen_delivery_audit_sha256,immutable_audit_ref,
          {accepted_audit_column_names}
          resolved_at
        FROM manual_resolutions;
        DROP TABLE manual_resolutions;
        ALTER TABLE manual_resolutions_legacy RENAME TO manual_resolutions;
        "#
    );
    connection
        .execute_batch(&ddl)
        .expect("downgrade manual resolution table for migration regression");
    connection
        .pragma_update(None, "user_version", schema_version)
        .expect("set legacy schema version");
}

fn schema_manifest_for_test(connection: &Connection) -> Vec<(String, String, String)> {
    let mut statement = connection
        .prepare(
            "SELECT type,name,COALESCE(sql,'')
             FROM sqlite_master
             WHERE name NOT LIKE 'sqlite_autoindex_%'
             ORDER BY type,name",
        )
        .expect("prepare TEST_CODE schema manifest");
    statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .expect("query TEST_CODE schema manifest")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("collect TEST_CODE schema manifest")
        .into_iter()
        .map(|(kind, name, sql)| {
            (
                kind,
                name,
                // SQLite's ALTER TABLE ... RENAME does not update self-FK
                // references declared in the CREATE TABLE body, so a v4→v5
                // migration leaves the predecessor FK pointing at
                // `immutable_audit_outbox_v5` even after the table is renamed
                // to `immutable_audit_outbox`. Normalize the target to the
                // canonical name so the migrated manifest matches the fresh
                // v5 manifest.
                sql.replace('"', "")
                    .replace(
                        "REFERENCES immutable_audit_outbox_v3(audit_identity)",
                        "REFERENCES immutable_audit_outbox(audit_identity)",
                    )
                    .replace(
                        "REFERENCES immutable_audit_outbox_v4(audit_identity)",
                        "REFERENCES immutable_audit_outbox(audit_identity)",
                    )
                    .replace(
                        "REFERENCES immutable_audit_outbox_v5(audit_identity)",
                        "REFERENCES immutable_audit_outbox(audit_identity)",
                    )
                    .replace(
                        "REFERENCES immutable_audit_outbox_v5(attempt_identity)",
                        "REFERENCES delivery_attempts(attempt_identity)",
                    )
                    .replace(
                        "REFERENCES immutable_audit_outbox_v4(attempt_identity)",
                        "REFERENCES delivery_attempts(attempt_identity)",
                    )
                    .replace(
                        "REFERENCES immutable_audit_outbox_v3(attempt_identity)",
                        "REFERENCES delivery_attempts(attempt_identity)",
                    )
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" "),
            )
        })
        .collect()
}

#[test]
fn br194_sha256_function_catalog_is_deterministic_innocuous_and_blob_only() {
    use rusqlite::functions::FunctionFlags;

    let connection =
        Connection::open_in_memory().expect("open TEST_CODE sha256 registration database");
    super::schema::register_sha256_function(&connection)
        .expect("register TEST_CODE sha256 function");
    let (encoding, flags): (String, i64) = connection
        .query_row(
            "SELECT enc,flags
             FROM pragma_function_list
             WHERE name='sha256_hex' AND type='s' AND narg=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read TEST_CODE sha256 function catalog");
    let required_flags =
        (FunctionFlags::SQLITE_DETERMINISTIC | FunctionFlags::SQLITE_INNOCUOUS).bits() as i64;
    assert_eq!(encoding.to_ascii_lowercase(), "utf8");
    assert_eq!(flags & required_flags, required_flags);
    assert_eq!(
        connection
            .query_row("SELECT sha256_hex(X'')", [], |row| row.get::<_, String>(0))
            .expect("hash TEST_CODE empty canonical blob"),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert!(
        connection
            .query_row("SELECT sha256_hex('')", [], |row| row.get::<_, String>(0))
            .is_err(),
        "sha256 authority must reject TEXT so canonical bytes cannot be re-encoded"
    );
}

fn downgrade_replay_schema_v4_for_test(connection: &mut Connection, replay_present: bool) {
    let decision_canonical = br#"{"schema":"TEST_CODE_V4_DECISION"}"#;
    let decision_hash = sha256_hex(decision_canonical);
    connection
        .execute(
            "INSERT INTO delivery_decisions(
               decision_identity,business_date,push_kind,sub_kind,cooldown_scope,
               scope_key,state,envelope_version,envelope_canonical,envelope_sha256,
               task_binding_present,transition_basis_canonical,transition_basis_sha256,
               reservation_generation,current_budget_reservation_identity,
               current_cooldown_reservation_identity,current_attempt_identity,
               current_disposition_identity,fence_generation,retry_authorized,
               created_at,updated_at
             ) VALUES(
               'TEST_CODE_V4_DECISION','2026-07-29','ReviewLhb','','Global',
               'global','Delivered',1,?1,?2,0,NULL,NULL,0,NULL,NULL,NULL,NULL,0,0,
               '2026-07-29T21:00:00Z','2026-07-29T21:00:00Z'
             )",
            params![decision_canonical.as_slice(), decision_hash],
        )
        .expect("seed historical v4 decision");
    let ordinary_canonical = br#"{"schema":"TEST_CODE_V4_AUDIT"}"#;
    let ordinary_hash = sha256_hex(ordinary_canonical);
    connection
        .execute(
            "INSERT INTO immutable_audit_outbox(
               audit_identity,decision_identity,attempt_identity,audit_kind,
               predecessor_audit_identity,audit_canonical,audit_sha256,
               append_state,immutable_audit_ref,created_at
             ) VALUES(
               'TEST_CODE_V4_AUDIT','TEST_CODE_V4_DECISION',NULL,
               'DecisionStateChanged',NULL,?1,?2,'Pending',NULL,
               '2026-07-29T21:00:00Z'
             )",
            params![ordinary_canonical.as_slice(), ordinary_hash],
        )
        .expect("seed historical v4 ordinary audit");
    let linked_canonical = br#"{"schema":"TEST_CODE_V4_AUDIT_CHILD"}"#;
    let linked_hash = sha256_hex(linked_canonical);
    connection
        .execute(
            "INSERT INTO immutable_audit_outbox(
               audit_identity,decision_identity,attempt_identity,audit_kind,
               predecessor_audit_identity,audit_canonical,audit_sha256,
               append_state,immutable_audit_ref,created_at
             ) VALUES(
               'TEST_CODE_V4_AUDIT_CHILD','TEST_CODE_V4_DECISION',NULL,
               'DecisionStateChanged','TEST_CODE_V4_AUDIT',?1,?2,'Pending',NULL,
               '2026-07-29T21:00:01Z'
             )",
            params![linked_canonical.as_slice(), linked_hash],
        )
        .expect("seed linked historical v4 audit");

    if replay_present {
        connection
            .pragma_update(None, "foreign_keys", "OFF")
            .expect("disable FK during replay-present v4 downgrade");
        let replay_canonical = br#"{"schema":"TEST_CODE_V4_REPLAY"}"#;
        let replay_hash = sha256_hex(replay_canonical);
        connection
            .execute(
                "INSERT INTO immutable_audit_outbox(
                   audit_identity,decision_identity,attempt_identity,audit_kind,
                   predecessor_audit_identity,audit_canonical,audit_sha256,
                   append_state,immutable_audit_ref,created_at
                 ) VALUES(
                   'TEST_CODE_V4_REPLAY_AUDIT','TEST_CODE_V4_DECISION',NULL,
                   'ReviewTerminalReplayStarted','TEST_CODE_V4_AUDIT_CHILD',
                   ?1,?2,'Pending',NULL,
                   '2026-07-29T21:01:00Z'
                 )",
                params![replay_canonical.as_slice(), replay_hash],
            )
            .expect("seed historical v4 replay audit");
        connection
            .execute(
                "INSERT INTO review_terminal_replay_attempts(
                   attempt_identity,business_date,review_task,task_identity,
                   decision_identity,replay_ordinal,started_at,
                   pre_sink_count,pre_sink_set_sha256,
                   pre_delivery_audit_count,pre_delivery_audit_set_sha256,
                   provider_calls,start_canonical,start_sha256,start_audit_identity
                 ) VALUES(
                   'TEST_CODE_V4_REPLAY_ATTEMPT','2026-07-29','R-04',
                   'TEST_CODE_V4_TASK','TEST_CODE_V4_DECISION',1,
                   '2026-07-29T21:01:00Z',0,?1,0,?1,0,?2,?3,
                   'TEST_CODE_V4_REPLAY_AUDIT'
                 )",
                params!["0".repeat(64), replay_canonical.as_slice(), replay_hash],
            )
            .expect("seed historical v4 replay attempt");
        connection
            .execute_batch(
                "DROP TRIGGER validate_review_terminal_replay_attempt_audit_insert;
                 DROP TRIGGER validate_review_terminal_replay_completion_audit_insert;
                 CREATE TRIGGER validate_review_terminal_replay_attempt_audit_insert
                 BEFORE INSERT ON review_terminal_replay_attempts
                 WHEN NOT EXISTS(
                   SELECT 1 FROM immutable_audit_outbox audit
                   WHERE audit.audit_identity=NEW.start_audit_identity
                     AND audit.decision_identity=NEW.decision_identity
                     AND audit.attempt_identity IS NULL
                     AND audit.audit_kind='ReviewTerminalReplayStarted'
                     AND audit.audit_canonical=NEW.start_canonical
                     AND audit.audit_sha256=NEW.start_sha256
                 )
                 BEGIN
                   SELECT RAISE(ABORT,'review terminal replay start audit mismatch');
                 END;
                 CREATE TRIGGER validate_review_terminal_replay_completion_audit_insert
                 BEFORE INSERT ON review_terminal_replay_completions
                 WHEN NOT EXISTS(
                   SELECT 1 FROM immutable_audit_outbox audit
                   WHERE audit.audit_identity=NEW.completion_audit_identity
                     AND audit.decision_identity=NEW.decision_identity
                     AND audit.attempt_identity IS NULL
                     AND audit.audit_kind='ReviewTerminalReplayCompleted'
                     AND audit.audit_canonical=NEW.completion_canonical
                     AND audit.audit_sha256=NEW.completion_sha256
                 )
                 BEGIN
                   SELECT RAISE(ABORT,'review terminal replay completion audit mismatch');
                 END;",
            )
            .expect("restore historical weak v4 replay triggers");
    } else {
        connection
            .execute_batch(
                "PRAGMA foreign_keys=OFF;
                 DROP TRIGGER validate_review_terminal_replay_attempt_audit_insert;
                 DROP TRIGGER validate_review_terminal_replay_completion_audit_insert;
                 DROP TRIGGER immutable_review_terminal_replay_attempt_update;
                 DROP TRIGGER immutable_review_terminal_replay_attempt_delete;
                 DROP TRIGGER immutable_review_terminal_replay_completion_update;
                 DROP TRIGGER immutable_review_terminal_replay_completion_delete;
                 DROP TABLE review_terminal_replay_completions;
                 DROP TABLE review_terminal_replay_attempts;
                 DROP TRIGGER immutable_outbox_payload_update;
                 DROP TRIGGER immutable_outbox_delete;
                 CREATE TABLE immutable_audit_outbox_v4_historical(
                   audit_identity TEXT PRIMARY KEY,
                   decision_identity TEXT NOT NULL
                     REFERENCES delivery_decisions(decision_identity),
                   attempt_identity TEXT REFERENCES delivery_attempts(attempt_identity),
                   audit_kind TEXT NOT NULL CHECK(audit_kind IN (
                     'DecisionStateChanged','LeaseGranted','LeaseHeartbeat',
                     'FenceRevoked','RecoveryClassified',
                     'SinkResultAuthorityClassified','LateReceiptObserved',
                     'BudgetReservationChanged','CooldownReservationChanged',
                     'BusinessDateOnceClaimed','DecisionIdentityConflict',
                     'ScheduleHydrationApplied')),
                   predecessor_audit_identity TEXT
                     REFERENCES immutable_audit_outbox(audit_identity),
                   audit_canonical BLOB NOT NULL,
                   audit_sha256 TEXT NOT NULL,
                   append_state TEXT NOT NULL CHECK(append_state IN ('Pending','Appended')),
                   immutable_audit_ref TEXT,
                   created_at TEXT NOT NULL
                 );
                 INSERT INTO immutable_audit_outbox_v4_historical
                   SELECT * FROM immutable_audit_outbox;
                 DROP TABLE immutable_audit_outbox;
                 ALTER TABLE immutable_audit_outbox_v4_historical
                   RENAME TO immutable_audit_outbox;",
            )
            .expect("restore exact replay-absent historical v4 schema");
        let historical_outbox_sql: String = connection
            .query_row(
                "SELECT sql FROM sqlite_master
                 WHERE type='table' AND name='immutable_audit_outbox'",
                [],
                |row| row.get(0),
            )
            .expect("read historical v4 outbox DDL");
        assert!(!historical_outbox_sql.contains("ReviewTerminalReplayStarted"));
        let replay_table_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type='table' AND name LIKE 'review_terminal_replay_%'",
                [],
                |row| row.get(0),
            )
            .expect("count historical v4 replay tables");
        assert_eq!(replay_table_count, 0);
    }
    connection
        .pragma_update(None, "user_version", 4_i64)
        .expect("set exact historical v4 schema version");
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .expect("enable FK enforcement for historical v4 migration");
}

fn real_v4_upgrade_target_snapshot(
    connection: &Connection,
    decision_identity: &str,
) -> BTreeMap<String, Vec<Vec<rusqlite::types::Value>>> {
    [
        ("delivery_decisions", "decision_identity"),
        ("daily_budget_reservations", "budget_reservation_identity"),
        ("cooldown_reservations", "cooldown_reservation_identity"),
        ("delivery_attempts", "attempt_identity"),
        ("delivery_state_events", "state_event_identity"),
        ("delivery_attempt_events", "attempt_event_identity"),
        ("daily_budget_reservation_events", "event_identity"),
        ("cooldown_reservation_events", "event_identity"),
        ("immutable_audit_outbox", "audit_identity"),
        ("sink_results", "result_event_identity"),
        ("delivery_disposition_payloads", "disposition_identity"),
        ("task_transition_payloads", "transition_identity"),
    ]
    .into_iter()
    .map(|(table, stable_order)| {
        let mut statement = connection
            .prepare(&format!(
                "SELECT * FROM {table} WHERE decision_identity=?1 ORDER BY {stable_order}"
            ))
            .unwrap_or_else(|error| panic!("prepare target snapshot for {table}: {error}"));
        let column_count = statement.column_count();
        let rows = statement
            .query_map([decision_identity], |row| {
                (0..column_count)
                    .map(|index| row.get(index))
                    .collect::<rusqlite::Result<Vec<rusqlite::types::Value>>>()
            })
            .unwrap_or_else(|error| panic!("query target snapshot for {table}: {error}"))
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap_or_else(|error| panic!("collect target snapshot for {table}: {error}"));
        (table.to_owned(), rows)
    })
    .collect()
}

fn audit_v4_upgrade_rowid_snapshot(connection: &Connection) -> BTreeMap<String, i64> {
    let mut statement = connection
        .prepare(
            "SELECT audit_identity,rowid
             FROM immutable_audit_outbox
             ORDER BY audit_identity",
        )
        .expect("prepare immutable-audit rowid snapshot");
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("query immutable-audit rowid snapshot")
        .collect::<rusqlite::Result<BTreeMap<_, _>>>()
        .expect("collect immutable-audit rowid snapshot")
}

fn audit_v4_upgrade_database_snapshot(
    connection: &Connection,
) -> (
    i64,
    Vec<(String, String, String)>,
    BTreeMap<String, Vec<Vec<String>>>,
    BTreeMap<String, i64>,
) {
    let version = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("read historical schema version snapshot");
    let tables = {
        let mut statement = connection
            .prepare(
                "SELECT name FROM sqlite_master
                 WHERE type='table' AND name NOT LIKE 'sqlite_%'
                 ORDER BY name",
            )
            .expect("prepare historical authority table list");
        statement
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query historical authority table list")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("collect historical authority table list")
    };
    let rows = tables
        .into_iter()
        .map(|table| {
            let contents = authority_table_rows(connection, &table);
            (table, contents)
        })
        .collect();
    let manifest = {
        let mut statement = connection
            .prepare(
                "SELECT type,name,COALESCE(sql,'')
                 FROM sqlite_master
                 WHERE name NOT LIKE 'sqlite_autoindex_%'
                 ORDER BY type,name",
            )
            .expect("prepare raw historical schema manifest");
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .expect("query raw historical schema manifest")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("collect raw historical schema manifest")
    };
    (
        version,
        manifest,
        rows,
        audit_v4_upgrade_rowid_snapshot(connection),
    )
}

fn real_audit_logical_tail(connection: &Connection, decision_identity: &str) -> (String, usize) {
    let mut statement = connection
        .prepare(
            "SELECT audit_identity,predecessor_audit_identity,audit_canonical,audit_sha256
             FROM immutable_audit_outbox
             WHERE decision_identity=?1
             ORDER BY audit_identity",
        )
        .expect("prepare real audit chain");
    let links = statement
        .query_map([decision_identity], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Vec<u8>>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .expect("query real audit chain")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("collect real audit chain");
    assert!(!links.is_empty(), "real decision must have an audit chain");
    let identities = links
        .iter()
        .map(|(identity, _, _, _)| identity.clone())
        .collect::<BTreeSet<_>>();
    let predecessors = links
        .iter()
        .filter_map(|(_, predecessor, _, _)| predecessor.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        links
            .iter()
            .filter(|(_, predecessor, _, _)| predecessor.is_none())
            .count(),
        1,
        "the real pre-upgrade audit chain must have one root"
    );
    for (identity, predecessor, canonical, digest) in &links {
        assert_eq!(
            sha256_hex(canonical),
            *digest,
            "audit {identity} must retain its exact canonical-byte digest"
        );
        if let Some(predecessor) = predecessor {
            assert!(
                identities.contains(predecessor),
                "audit {identity} must reference this decision's real chain"
            );
        }
    }
    let tails = identities
        .difference(&predecessors)
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        tails.len(),
        1,
        "the real pre-upgrade chain must have one tail"
    );
    let tail = tails[0].clone();
    let by_identity = links
        .iter()
        .map(|(identity, predecessor, _, _)| (identity.as_str(), predecessor.as_deref()))
        .collect::<BTreeMap<_, _>>();
    let mut visited = BTreeSet::new();
    let mut cursor = Some(tail.as_str());
    while let Some(identity) = cursor {
        assert!(visited.insert(identity), "real audit chain must be acyclic");
        cursor = *by_identity
            .get(identity)
            .expect("every predecessor must resolve in the real decision chain");
    }
    assert_eq!(
        visited.len(),
        links.len(),
        "the independently traversed real chain must include every target audit"
    );
    (tail, links.len())
}

fn foreign_key_violation_details(connection: &Connection) -> Vec<String> {
    let violations = {
        let mut statement = connection
            .prepare(
                "SELECT \"table\",rowid,parent,fkid
                 FROM pragma_foreign_key_check
                 ORDER BY \"table\",rowid,parent,fkid",
            )
            .expect("prepare exact foreign-key violation rows");
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<i64>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .expect("query exact foreign-key violation rows")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("collect exact foreign-key violation rows")
    };
    violations
        .into_iter()
        .map(|(table, rowid, reported_parent, fkid)| {
            let declared_links = {
                let mut statement = connection
                    .prepare(
                        "SELECT id,seq,\"table\",\"from\",\"to\",on_update,on_delete,\"match\"
                         FROM pragma_foreign_key_list(?1)
                         WHERE id=?2
                         ORDER BY seq",
                    )
                    .unwrap_or_else(|error| {
                        panic!("prepare declared foreign key for {table}/{fkid}: {error}")
                    });
                statement
                    .query_map(params![table.as_str(), fkid], |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, Option<String>>(4)?,
                            row.get::<_, String>(5)?,
                            row.get::<_, String>(6)?,
                            row.get::<_, String>(7)?,
                        ))
                    })
                    .unwrap_or_else(|error| {
                        panic!("query declared foreign key for {table}/{fkid}: {error}")
                    })
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .unwrap_or_else(|error| {
                        panic!("collect declared foreign key for {table}/{fkid}: {error}")
                    })
            };
            format!(
                "table={table} rowid={rowid:?} reported_parent={reported_parent} fkid={fkid} declared={declared_links:?}"
            )
        })
        .collect()
}

#[cfg(unix)]
struct PhysicalAliasFixture {
    test_code: String,
    foreign_test_code: String,
    test_root: PathBuf,
    foreign_test_root: PathBuf,
    cleanup: OwnedTestPaths,
    production_storage_before: ProductionStorageSnapshot,
}

#[cfg(unix)]
impl PhysicalAliasFixture {
    fn new(test_code: &str) -> Self {
        let foreign_test_code = format!("{test_code}_FOREIGN");
        Self {
            test_code: test_code.to_owned(),
            foreign_test_code: foreign_test_code.clone(),
            test_root: PathBuf::from("data/test").join(test_code),
            foreign_test_root: PathBuf::from("data/test").join(foreign_test_code),
            cleanup: OwnedTestPaths::new(),
            production_storage_before: ProductionStorageSnapshot::capture(),
        }
    }

    fn test_database_path(&self) -> PathBuf {
        self.test_root.join("durable_delivery.sqlite3")
    }

    fn foreign_database_path(&self) -> PathBuf {
        self.foreign_test_root.join("durable_delivery.sqlite3")
    }

    fn test_sidecar_path(&self, suffix: &str) -> PathBuf {
        PathBuf::from(format!("{}{suffix}", self.test_database_path().display()))
    }

    fn foreign_sidecar_path(&self, suffix: &str) -> PathBuf {
        PathBuf::from(format!(
            "{}{suffix}",
            self.foreign_database_path().display()
        ))
    }

    fn ensure_root(&self, root: &Path) {
        std::fs::create_dir_all("data/test").expect("create lexical test namespace");
        std::fs::create_dir(root).expect("create unique TEST_CODE namespace");
        self.cleanup.record(root, OwnedPathKind::Directory);
    }

    fn ensure_test_root(&self) {
        self.ensure_root(&self.test_root);
    }

    fn ensure_foreign_root(&self) {
        self.ensure_root(&self.foreign_test_root);
    }

    fn create_file(&self, path: &Path) {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .unwrap_or_else(|error| panic!("create test-owned file {}: {error}", path.display()));
        self.cleanup.record(path, OwnedPathKind::FileOrSymlink);
    }

    fn create_test_sentinel(&self) {
        self.ensure_test_root();
        self.create_file(&self.test_database_path());
    }

    fn create_foreign_sentinel(&self) {
        self.ensure_foreign_root();
        self.create_file(&self.foreign_database_path());
    }

    fn create_foreign_sidecar_sentinel(&self, suffix: &str) {
        self.ensure_foreign_root();
        self.create_file(&self.foreign_sidecar_path(suffix));
    }

    fn create_symlink(&self, target: impl AsRef<Path>, link: &Path) {
        std::os::unix::fs::symlink(target, link).unwrap_or_else(|error| {
            panic!("create test-owned symlink {}: {error}", link.display())
        });
        self.cleanup.record(link, OwnedPathKind::FileOrSymlink);
    }

    fn create_hard_link(&self, source: &Path, link: &Path) {
        std::fs::hard_link(source, link).unwrap_or_else(|error| {
            panic!("create test-owned hardlink {}: {error}", link.display())
        });
        self.cleanup.record(link, OwnedPathKind::FileOrSymlink);
    }

    fn rename_owned(&self, source: &Path, destination: &Path) {
        std::fs::rename(source, destination).unwrap_or_else(|error| {
            panic!(
                "rename test-owned path {} to {}: {error}",
                source.display(),
                destination.display()
            )
        });
        self.cleanup
            .record(destination, OwnedPathKind::FileOrSymlink);
    }

    fn rename_owned_directory(&self, source: &Path, destination: &Path) {
        std::fs::rename(source, destination).unwrap_or_else(|error| {
            panic!(
                "rename test-owned directory {} to {}: {error}",
                source.display(),
                destination.display()
            )
        });
        self.cleanup.record(destination, OwnedPathKind::Directory);
    }

    fn capture_sqlite_objects_beneath(&self, root: &Path) {
        let database_path = root.join("durable_delivery.sqlite3");
        for suffix in ["", "-journal", "-shm", "-wal"] {
            self.cleanup.record_if_present(
                PathBuf::from(format!("{}{suffix}", database_path.display())),
                OwnedPathKind::FileOrSymlink,
            );
        }
    }

    fn capture_sqlite_objects(&self) {
        self.capture_sqlite_objects_beneath(&self.test_root);
        self.capture_sqlite_objects_beneath(&self.foreign_test_root);
    }

    fn open_test(&self) -> Result<DurableDeliveryCoordinator> {
        let result = DurableDeliveryCoordinator::open(isolation_test_config(
            self.test_database_path(),
            &self.test_code,
        ));
        self.capture_sqlite_objects();
        result
    }
}

#[cfg(unix)]
impl Drop for PhysicalAliasFixture {
    fn drop(&mut self) {
        self.cleanup.clean_now();
        self.production_storage_before.assert_unchanged();
    }
}

#[cfg(unix)]
#[derive(Debug)]
struct RetainedProductionAncestor {
    path: PathBuf,
    anchor: std::fs::File,
    identity: FilesystemIdentity,
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ProductionObjectState {
    identity: FilesystemIdentity,
    length: u64,
    modified_nanos: i128,
}

#[cfg(unix)]
impl ProductionObjectState {
    /// `None` means the object is absent; absence is itself part of the contract.
    fn capture(path: &Path) -> Option<Self> {
        use std::os::unix::fs::MetadataExt;

        match path.symlink_metadata() {
            Ok(metadata) => Some(Self {
                identity: FilesystemIdentity::capture(path).unwrap_or_else(|error| {
                    panic!("stat production SQLite object {}: {error}", path.display())
                }),
                length: metadata.len(),
                modified_nanos: i128::from(metadata.mtime()) * 1_000_000_000
                    + i128::from(metadata.mtime_nsec()),
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => panic!(
                "stat production SQLite namespace entry {}: {error}",
                path.display()
            ),
        }
    }
}

#[cfg(unix)]
#[derive(Debug)]
struct ProductionStorageSnapshot {
    ancestors: Vec<RetainedProductionAncestor>,
    production_objects: Vec<(PathBuf, Option<ProductionObjectState>)>,
}

#[cfg(unix)]
impl ProductionStorageSnapshot {
    fn capture() -> Self {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let data = manifest.join("data");
        let ancestors = [manifest, data]
            .into_iter()
            .map(|path| {
                let anchor = std::fs::File::open(&path).unwrap_or_else(|error| {
                    panic!(
                        "retain production namespace ancestor {}: {error}",
                        path.display()
                    )
                });
                let metadata = anchor.metadata().unwrap_or_else(|error| {
                    panic!(
                        "stat retained production namespace ancestor {}: {error}",
                        path.display()
                    )
                });
                assert!(
                    metadata.is_dir(),
                    "production namespace ancestor must remain a directory: {}",
                    path.display()
                );
                RetainedProductionAncestor {
                    identity: FilesystemIdentity::capture(&path).unwrap_or_else(|error| {
                        panic!(
                            "stat production namespace ancestor {}: {error}",
                            path.display()
                        )
                    }),
                    path,
                    anchor,
                }
            })
            .collect::<Vec<_>>();

        let main = Path::new(env!("CARGO_MANIFEST_DIR")).join("data/durable_delivery.sqlite3");
        let production_objects = ["", "-journal", "-shm", "-wal"]
            .into_iter()
            .map(|suffix| {
                let path = PathBuf::from(format!("{}{suffix}", main.display()));
                let state = ProductionObjectState::capture(&path);
                (path, state)
            })
            .collect();
        Self {
            ancestors,
            production_objects,
        }
    }

    fn assert_unchanged(&self) {
        for ancestor in &self.ancestors {
            let retained = ancestor.anchor.metadata().unwrap_or_else(|error| {
                panic!(
                    "stat retained production namespace ancestor {}: {error}",
                    ancestor.path.display()
                )
            });
            assert!(
                retained.is_dir(),
                "retained production namespace ancestor changed type: {}",
                ancestor.path.display()
            );
            assert_eq!(
                FilesystemIdentity::capture(&ancestor.path).unwrap_or_else(|error| {
                    panic!(
                        "restat production namespace ancestor {}: {error}",
                        ancestor.path.display()
                    )
                }),
                ancestor.identity,
                "production namespace ancestor changed identity: {}",
                ancestor.path.display()
            );
        }
        for (path, before) in &self.production_objects {
            let after = ProductionObjectState::capture(path);
            match (before, after) {
                (None, None) => {}
                (None, Some(_)) => panic!(
                    "TEST_CODE fixture created a production SQLite artifact: {}",
                    path.display()
                ),
                (Some(_), None) => panic!(
                    "TEST_CODE fixture deleted a production SQLite artifact: {}",
                    path.display()
                ),
                (Some(before), Some(after)) => assert_eq!(
                    *before,
                    after,
                    "TEST_CODE fixture mutated a production SQLite artifact: {}",
                    path.display()
                ),
            }
        }
    }
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br192_test_fixtures_do_not_create_or_delete_production_main_wal_or_shm() {
    let before = ProductionStorageSnapshot::capture();
    {
        let test_code = format!(
            "TEST_CODE_BR192_FIXTURE_BOUNDARY_{}_{}",
            std::process::id(),
            NEXT_TEST_ID.fetch_add(1, Ordering::SeqCst)
        );
        let fixture = PhysicalAliasFixture::new(&test_code);
        fixture.ensure_test_root();
        fixture.create_foreign_sentinel();
        fixture.create_hard_link(
            &fixture.foreign_database_path(),
            &fixture.test_database_path(),
        );
    }
    before.assert_unchanged();
}

#[cfg(unix)]
#[test]
fn br192_test_cleanup_ownership_rejects_every_production_storage_leaf() {
    for suffix in ["", "-journal", "-shm", "-wal"] {
        let cleanup = OwnedTestPaths::new();
        let production_path = PathBuf::from(format!("data/durable_delivery.sqlite3{suffix}"));
        let rejected = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            cleanup.record(&production_path, OwnedPathKind::FileOrSymlink);
        }));
        assert!(
            rejected.is_err(),
            "cleanup ownership must reject production path {} before filesystem access",
            production_path.display()
        );
    }
}

#[test]
fn br192_production_config_rejects_every_nonfixed_database_root() {
    let mut config = CoordinatorConfig::production("owner-production-0123456789abcdef");
    assert_eq!(
        config.database_path,
        PathBuf::from("data/durable_delivery.sqlite3")
    );
    config.database_path = PathBuf::from("data/alternate/durable_delivery.sqlite3");
    assert!(matches!(
        config.validate(),
        Err(DurableDeliveryError::IsolationViolation(_))
    ));
}

#[test]
fn br192_test_database_path_accepts_only_the_exact_test_namespace() {
    let test_code = "TEST_CODE_BR192_EXACT_PATH";
    let exact_relative = PathBuf::from("data/test")
        .join(test_code)
        .join("durable_delivery.sqlite3");
    let exact_manifest_absolute = Path::new(env!("CARGO_MANIFEST_DIR")).join(&exact_relative);

    isolation_test_config(&exact_relative, test_code)
        .validate()
        .expect("exact lexical test namespace must be accepted");
    isolation_test_config(&exact_manifest_absolute, test_code)
        .validate()
        .expect("exact manifest-absolute test namespace must be accepted");
    assert_eq!(
        isolation_test_config(exact_manifest_absolute, test_code)
            .repository_relative_database_path()
            .expect("normalize exact manifest-absolute test namespace"),
        exact_relative,
        "both accepted representations must normalize to one repository-relative authority"
    );
}

#[test]
fn br192_test_database_path_rejects_parent_directory_alias_to_production() {
    let test_code = "TEST_CODE_BR192_TRAVERSAL";
    let aliased_production_path =
        PathBuf::from(format!("data/{test_code}/../durable_delivery.sqlite3"));

    assert!(matches!(
        isolation_test_config(aliased_production_path, test_code).validate(),
        Err(DurableDeliveryError::IsolationViolation(_))
    ));
}

#[test]
fn br192_test_database_path_rejects_every_non_exact_namespace_shape() {
    let test_code = "TEST_CODE_BR192_NON_EXACT";
    let invalid_paths = [
        PathBuf::from("data/test")
            .join(test_code)
            .join("nested/durable_delivery.sqlite3"),
        PathBuf::from("data/test/alias")
            .join(test_code)
            .join("durable_delivery.sqlite3"),
        PathBuf::from("data/test")
            .join(test_code)
            .join("../durable_delivery.sqlite3"),
        PathBuf::from("data")
            .join(test_code)
            .join("durable_delivery.sqlite3"),
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("data/test")
            .join(test_code)
            .join("../durable_delivery.sqlite3"),
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("data/test/alias")
            .join(test_code)
            .join("durable_delivery.sqlite3"),
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("manifest parent")
            .join("stock_analysis_alias/data/test")
            .join(test_code)
            .join("durable_delivery.sqlite3"),
        Path::new("/")
            .join("tmp/data/test")
            .join(test_code)
            .join("durable_delivery.sqlite3"),
    ];

    for invalid_path in invalid_paths {
        assert!(
            matches!(
                isolation_test_config(&invalid_path, test_code).validate(),
                Err(DurableDeliveryError::IsolationViolation(_))
            ),
            "non-exact test database path must be rejected: {}",
            invalid_path.display()
        );
    }
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br192_physical_store_accepts_real_exact_test_namespace() {
    let fixture = Fixture::new("PHYSICAL_EXACT");

    assert!(
        fixture.database_path.exists(),
        "exact physical test namespace must open its isolated database"
    );
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br192_descriptor_enumeration_entry_error_fails_open_explicitly() {
    let test_code = format!(
        "TEST_CODE_BR192_DESCRIPTOR_ENTRY_ERROR_{}_{}",
        std::process::id(),
        NEXT_TEST_ID.fetch_add(1, Ordering::SeqCst)
    );
    let fixture = PhysicalAliasFixture::new(&test_code);
    fixture.ensure_test_root();
    let _fault = install_process_descriptor_snapshot_test_fault(
        0,
        ProcessDescriptorSnapshotTestFault::EntryError,
    )
    .expect("install descriptor enumeration fault");

    assert!(matches!(
        fixture.open_test(),
        Err(DurableDeliveryError::IsolationViolation(reason))
            if reason.contains("ReadDir entry error")
    ));
    assert!(
        !fixture.test_database_path().exists(),
        "descriptor enumeration failure must precede main O_CREAT"
    );
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br192_descriptor_enumeration_ambiguity_fails_before_main_creation() {
    let test_code = format!(
        "TEST_CODE_BR192_DESCRIPTOR_CAPABILITY_AMBIGUITY_{}_{}",
        std::process::id(),
        NEXT_TEST_ID.fetch_add(1, Ordering::SeqCst)
    );
    let fixture = PhysicalAliasFixture::new(&test_code);
    fixture.ensure_test_root();
    let _fault = install_process_descriptor_snapshot_test_fault(
        0,
        ProcessDescriptorSnapshotTestFault::AmbiguityError,
    )
    .expect("install descriptor enumeration ambiguity");

    assert!(matches!(
        fixture.open_test(),
        Err(DurableDeliveryError::IsolationViolation(reason))
            if reason.contains("enumeration ambiguity")
    ));
    assert!(
        !fixture.test_database_path().exists(),
        "descriptor enumeration ambiguity must precede main O_CREAT"
    );
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br192_ofd_capability_failure_precedes_main_creation() {
    let test_code = format!(
        "TEST_CODE_BR192_OFD_CAPABILITY_{}_{}",
        std::process::id(),
        NEXT_TEST_ID.fetch_add(1, Ordering::SeqCst)
    );
    let fixture = PhysicalAliasFixture::new(&test_code);
    fixture.ensure_test_root();
    let _fault = install_database_bootstrap_test_hook(
        DatabaseBootstrapTestPhase::BeforeOpenFileDescriptionCapabilityProbe,
        || {
            Err(DurableDeliveryError::IsolationViolation(
                "TEST_CODE injected unsupported OFD capability".to_owned(),
            ))
        },
    )
    .expect("install OFD capability fault");

    assert!(matches!(
        fixture.open_test(),
        Err(DurableDeliveryError::IsolationViolation(reason))
            if reason.contains("unsupported OFD capability")
    ));
    assert!(
        !fixture.test_database_path().exists(),
        "OFD capability failure must precede main O_CREAT"
    );
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br192_ambiguous_main_descriptor_delta_fails_open() {
    let test_code = format!(
        "TEST_CODE_BR192_DESCRIPTOR_AMBIGUITY_{}_{}",
        std::process::id(),
        NEXT_TEST_ID.fetch_add(1, Ordering::SeqCst)
    );
    let fixture = PhysicalAliasFixture::new(&test_code);
    fixture.ensure_test_root();
    let absolute_database =
        Path::new(env!("CARGO_MANIFEST_DIR")).join(fixture.test_database_path());
    let _fault = install_process_descriptor_snapshot_test_fault(
        2,
        ProcessDescriptorSnapshotTestFault::InjectAmbiguousDescriptor {
            absolute_path: absolute_database,
        },
    )
    .expect("install descriptor ambiguity fault");

    assert!(matches!(
        fixture.open_test(),
        Err(DurableDeliveryError::IsolationViolation(reason))
            if reason.contains("ambiguous main descriptors")
    ));
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br192_compiled_repository_root_is_independent_of_foreign_cwd() {
    const CHILD_ENV: &str = "TEST_CODE_BR192_FOREIGN_CWD_CHILD";
    if std::env::var_os(CHILD_ENV).is_none() {
        let status = std::process::Command::new(
            std::env::current_exe().expect("resolve current TEST_CODE test binary"),
        )
        .current_dir("/")
        .env(CHILD_ENV, "1")
        .args([
            "--exact",
            "durable_delivery::tests::br192_compiled_repository_root_is_independent_of_foreign_cwd",
            "--nocapture",
        ])
        .status()
        .expect("spawn isolated foreign-CWD child");
        assert!(status.success(), "foreign-CWD child must pass");
        return;
    }

    let production_before = ProductionStorageSnapshot::capture();
    let cleanup = OwnedTestPaths::new();
    let test_code = format!("TEST_CODE_BR192_FOREIGN_CWD_{}", std::process::id());
    let relative_root = PathBuf::from("data/test").join(&test_code);
    let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let absolute_root = repository_root.join(&relative_root);
    assert!(
        repository_root.join("data/test").is_dir(),
        "shared test parent must pre-exist"
    );
    std::fs::create_dir(&absolute_root).expect("create absolute TEST_CODE namespace");
    cleanup.record(&absolute_root, OwnedPathKind::Directory);
    let relative_database = relative_root.join("durable_delivery.sqlite3");
    let coordinator =
        DurableDeliveryCoordinator::open(isolation_test_config(&relative_database, &test_code))
            .expect("compiled repository root must ignore foreign cwd");
    assert!(coordinator
        .inspect_pending_for_date("2026-07-30")
        .expect("query from foreign cwd")
        .is_empty());
    drop(coordinator);
    for suffix in ["", "-journal", "-shm", "-wal"] {
        cleanup.record_if_present(
            PathBuf::from(format!(
                "{}{suffix}",
                repository_root.join(&relative_database).display()
            )),
            OwnedPathKind::FileOrSymlink,
        );
    }
    cleanup.clean_now();
    production_before.assert_unchanged();
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br192_descriptor_binding_accepts_a_second_coordinator_with_process_shared_shm() {
    let fixture = Fixture::new("PROCESS_SHARED_SHM");
    let second = fixture.second_coordinator("process-shared-shm");

    assert!(second
        .inspect_pending_for_date("2026-07-30")
        .expect("second coordinator must retain the exact process-shared SHM proof")
        .is_empty());
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br192_process_shared_shm_survives_direct_connection_owner_drop() {
    let mut fixture = Fixture::new("PROCESS_SHARED_SHM_OWNER_DROP");
    let second = fixture.second_coordinator("process-shared-shm-owner-drop");
    let direct_owner = fixture
        .coordinator
        .take()
        .expect("direct SHM coordinator owner");
    drop(direct_owner);

    assert!(second
        .inspect_pending_for_date("2026-07-30")
        .expect("shared coordinator must retain the live process-shared SHM OFD proof")
        .is_empty());
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br192_two_processes_can_open_use_and_drop_the_same_database() {
    const CHILD_ENV: &str = "TEST_CODE_BR192_TWO_PROCESS_CHILD";
    const PATH_ENV: &str = "TEST_CODE_BR192_TWO_PROCESS_PATH";
    const CODE_ENV: &str = "TEST_CODE_BR192_TWO_PROCESS_CODE";
    if std::env::var_os(CHILD_ENV).is_some() {
        let database_path = PathBuf::from(std::env::var(PATH_ENV).expect("child database path"));
        let test_code = std::env::var(CODE_ENV).expect("child TEST_CODE");
        let coordinator = DurableDeliveryCoordinator::open(CoordinatorConfig::test(
            database_path,
            test_code,
            "owner-two-process-child-0123456789abcdef",
        ))
        .expect("child opens same database while parent owner is live");
        assert!(coordinator
            .inspect_pending_for_date("2026-07-30")
            .expect("child uses same database")
            .is_empty());
        return;
    }

    let fixture = Fixture::new("TWO_PROCESS_SAME_DATABASE");
    let test_code = fixture
        .database_path
        .parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
        .expect("test code");
    let status = std::process::Command::new(
        std::env::current_exe().expect("resolve current TEST_CODE test binary"),
    )
    .current_dir("/")
    .env(CHILD_ENV, "1")
    .env(
        PATH_ENV,
        Path::new(env!("CARGO_MANIFEST_DIR")).join(&fixture.database_path),
    )
    .env(CODE_ENV, test_code)
    .args([
        "--exact",
        "durable_delivery::tests::br192_two_processes_can_open_use_and_drop_the_same_database",
        "--nocapture",
    ])
    .status()
    .expect("spawn second database owner process");
    assert!(status.success(), "second database owner process must pass");
    assert!(fixture
        .coordinator
        .inspect_pending_for_date("2026-07-30")
        .expect("parent remains usable after child drop")
        .is_empty());
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br192_cross_process_reconciler_commits_exact_audit_acknowledgement() {
    const CHILD_ENV: &str = "TEST_CODE_BR192_CROSS_PROCESS_ACK_CHILD";
    const PATH_ENV: &str = "TEST_CODE_BR192_CROSS_PROCESS_ACK_PATH";
    const CODE_ENV: &str = "TEST_CODE_BR192_CROSS_PROCESS_ACK_CODE";
    const APPEND_PATH_ENV: &str = "TEST_CODE_BR192_CROSS_PROCESS_APPEND_PATH";
    if std::env::var_os(CHILD_ENV).is_some() {
        let database_path = PathBuf::from(std::env::var(PATH_ENV).expect("child database path"));
        let test_code = std::env::var(CODE_ENV).expect("child TEST_CODE");
        let coordinator = DurableDeliveryCoordinator::open(CoordinatorConfig::test(
            database_path,
            test_code,
            "owner-cross-process-ack-child-0123456789abcdef",
        ))
        .expect("child opens pending acknowledgement database");
        let append = PersistentTestAppendPort::new(PathBuf::from(
            std::env::var(APPEND_PATH_ENV).expect("child persistent append path"),
        ));
        let summary = coordinator
            .reconcile_all_pending(&append, now())
            .expect("child reconciles pending immutable acknowledgement");
        assert!(summary.progress_count > 0);
        return;
    }

    let fixture = Fixture::new("CROSS_PROCESS_ACK");
    let candidate = envelope(
        "CROSS_PROCESS_ACK",
        PushKind::ReviewProviderTopN,
        DeliverySubKind::None,
        "2026-07-30",
        true,
    );
    fixture
        .coordinator
        .prepare(&candidate, 1, now())
        .expect("parent prepares pending acknowledgement");
    let test_code = fixture
        .database_path
        .parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
        .expect("test code");
    let append_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(fixture.database_path.parent().expect("TEST_CODE parent"))
        .join("immutable_append.jsonl");
    let status = std::process::Command::new(
        std::env::current_exe().expect("resolve current TEST_CODE test binary"),
    )
    .current_dir("/")
    .env(CHILD_ENV, "1")
    .env(
        PATH_ENV,
        Path::new(env!("CARGO_MANIFEST_DIR")).join(&fixture.database_path),
    )
    .env(CODE_ENV, test_code)
    .env(APPEND_PATH_ENV, &append_path)
    .args([
        "--exact",
        "durable_delivery::tests::br192_cross_process_reconciler_commits_exact_audit_acknowledgement",
        "--nocapture",
    ])
    .status()
    .expect("spawn cross-process acknowledgement reconciler");
    assert!(status.success(), "child acknowledgement must succeed");
    fixture
        .cleanup
        .record_if_present(&append_path, OwnedPathKind::FileOrSymlink);
    assert_eq!(
        fixture.query_i64(
            "SELECT COUNT(*) FROM immutable_audit_outbox
             WHERE append_state='Pending' OR immutable_audit_ref IS NULL"
        ),
        0,
        "parent must observe the child's durable acknowledgement commit"
    );
    let persisted = PersistentTestAppendPort::new(&append_path)
        .records()
        .expect("parent reads child-persisted append evidence after child exit");
    assert!(
        !persisted.is_empty(),
        "cross-process acknowledgement must leave file-backed immutable records"
    );
    let connection = Connection::open(&fixture.database_path).expect("open parent verification DB");
    let mut statement = connection
        .prepare(
            "SELECT audit_identity,audit_canonical,audit_sha256,immutable_audit_ref
             FROM immutable_audit_outbox
             WHERE append_state='Appended'
             ORDER BY rowid",
        )
        .expect("prepare exact acknowledgement join");
    let acknowledged = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .expect("query acknowledged records")
        .collect::<std::result::Result<Vec<_>, _>>()
        .expect("collect acknowledged records");
    assert_eq!(
        acknowledged.len(),
        persisted.len(),
        "every child-persisted append must have one exact SQLite acknowledgement"
    );
    for (identity, canonical, sha256, immutable_ref) in acknowledged {
        assert_eq!(sha256_hex(&canonical), sha256);
        let exact = persisted
            .iter()
            .find(|record| record.identity == identity)
            .expect("SQLite acknowledgement identity exists in persisted append file");
        assert_eq!(exact.canonical, canonical);
        assert_eq!(exact.sha256, sha256);
        assert_eq!(exact.immutable_ref, immutable_ref);
    }
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br192_cross_process_reconciler_joins_exact_manual_accepted_delivery_audit() {
    const CHILD_ENV: &str = "TEST_CODE_BR192_MANUAL_ACCEPT_CROSS_PROCESS_CHILD";
    const PATH_ENV: &str = "TEST_CODE_BR192_MANUAL_ACCEPT_CROSS_PROCESS_PATH";
    const CODE_ENV: &str = "TEST_CODE_BR192_MANUAL_ACCEPT_CROSS_PROCESS_CODE";
    const APPEND_PATH_ENV: &str = "TEST_CODE_BR192_MANUAL_ACCEPT_CROSS_PROCESS_APPEND_PATH";
    if std::env::var_os(CHILD_ENV).is_some() {
        let database_path = PathBuf::from(std::env::var(PATH_ENV).expect("child database path"));
        let test_code = std::env::var(CODE_ENV).expect("child TEST_CODE");
        let coordinator = DurableDeliveryCoordinator::open(CoordinatorConfig::test(
            database_path,
            test_code,
            "owner-manual-accept-cross-process-child-0123456789abcdef",
        ))
        .expect("child opens manual accepted pending database");
        let append = PersistentTestAppendPort::new(PathBuf::from(
            std::env::var(APPEND_PATH_ENV).expect("child persistent append path"),
        ));
        let summary = coordinator
            .reconcile_all_pending(&append, now())
            .expect("child reconciles manual accepted immutable evidence");
        assert!(summary.progress_count > 0);
        return;
    }

    let fixture = Fixture::new("MANUAL_ACCEPT_CROSS_PROCESS");
    let append_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(fixture.database_path.parent().expect("TEST_CODE parent"))
        .join("immutable_append.jsonl");
    let initial_append = PersistentTestAppendPort::new(&append_path);
    let candidate = envelope(
        "MANUAL_ACCEPT_CROSS_PROCESS",
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        "2026-07-30",
        false,
    );
    prepare_reserved(&fixture, &candidate, &initial_append);
    fixture
        .cleanup
        .record_if_present(&append_path, OwnedPathKind::FileOrSymlink);
    let sink = StaticSink::new(AuthoritativeSinkResult::Uncertain(uncertainty(now())));
    let sinks: Vec<AuthoritativeSink> = vec![sink];
    fixture
        .coordinator
        .resume_deliverable(&candidate.decision_identity, &sinks, now())
        .expect("parent persists uncertain result");
    reconcile_terminal(
        &fixture,
        &initial_append,
        DecisionState::UncertainManualReview,
        &candidate.decision_identity,
    );
    fixture
        .coordinator
        .resolve_uncertain(
            &ManualResolutionCommand {
                decision_identity: candidate.decision_identity.clone(),
                disposition: ManualDisposition::Accepted {
                    receipt: Some(receipt(now())),
                },
                operator_identity: "TEST_CODE_OPERATOR_CROSS_PROCESS_0123456789".to_owned(),
                reason: "TEST_CODE_VERIFIED_ACCEPTANCE_CROSS_PROCESS".to_owned(),
                external_evidence: b"TEST_CODE_MANUAL_ACCEPT_CROSS_PROCESS_EVIDENCE".to_vec(),
                resolved_at: now(),
            },
            &initial_append,
        )
        .expect("parent freezes manual accepted audit pending evidence");
    assert_eq!(
        fixture
            .coordinator
            .decision_state(&candidate.decision_identity)
            .expect("parent pending state"),
        DecisionState::AcceptedAuditPending
    );

    let test_code = fixture
        .database_path
        .parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
        .expect("test code");
    let status = std::process::Command::new(
        std::env::current_exe().expect("resolve current TEST_CODE test binary"),
    )
    .current_dir("/")
    .env(CHILD_ENV, "1")
    .env(
        PATH_ENV,
        Path::new(env!("CARGO_MANIFEST_DIR")).join(&fixture.database_path),
    )
    .env(CODE_ENV, test_code)
    .env(APPEND_PATH_ENV, &append_path)
    .args([
        "--exact",
        "durable_delivery::tests::br192_cross_process_reconciler_joins_exact_manual_accepted_delivery_audit",
        "--nocapture",
    ])
    .status()
    .expect("spawn cross-process manual accepted reconciler");
    assert!(
        status.success(),
        "child manual accepted reconciliation must succeed"
    );
    assert_eq!(
        fixture
            .coordinator
            .decision_state(&candidate.decision_identity)
            .expect("parent observes child Delivered commit"),
        DecisionState::Delivered
    );
    fixture
        .coordinator
        .verify_manual_accepted_delivery(&candidate.decision_identity)
        .expect("parent validates the child's complete manual accepted audit evidence");

    let persisted = PersistentTestAppendPort::new(&append_path)
        .records()
        .expect("parent reads child-persisted append evidence");
    let connection = Connection::open(&fixture.database_path).expect("open parent verification DB");
    let (identity, canonical, sha256, immutable_ref): (String, Vec<u8>, String, String) =
        connection
            .query_row(
                "SELECT m.accepted_audit_identity,m.frozen_delivery_audit_canonical,
                    m.frozen_delivery_audit_sha256,m.accepted_audit_ref
             FROM manual_resolutions m
             JOIN delivery_disposition_payloads p
               ON p.resolution_identity=m.resolution_identity
              AND p.decision_identity=m.decision_identity
              AND p.disposition='ManualAccepted'
              AND p.append_state='Appended'
             WHERE m.decision_identity=?1
               AND m.disposition='Accepted'
               AND m.accepted_audit_append_state='Appended'",
                [candidate.decision_identity.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("query exact manual accepted acknowledgement");
    assert_eq!(sha256_hex(&canonical), sha256);
    let exact = persisted
        .iter()
        .filter(|record| {
            record.record_kind == "DeliveryAcceptedAudit" && record.identity == identity
        })
        .collect::<Vec<_>>();
    assert_eq!(
        exact.len(),
        1,
        "child file must contain exactly one SQLite-identified DeliveryAcceptedAudit"
    );
    let exact = exact[0];
    assert_eq!(exact.identity, identity);
    assert_eq!(exact.canonical, canonical);
    assert_eq!(exact.sha256, sha256);
    assert_eq!(exact.immutable_ref, immutable_ref);
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br192_cross_process_ofd_owner_cannot_be_borrowed_and_is_reusable_after_drop() {
    const CHILD_ENV: &str = "TEST_CODE_BR192_CROSS_PROCESS_OFD_CHILD";
    const PATH_ENV: &str = "TEST_CODE_BR192_CROSS_PROCESS_OFD_PATH";
    const CODE_ENV: &str = "TEST_CODE_BR192_CROSS_PROCESS_OFD_CODE";
    const OWNER_ENV: &str = "TEST_CODE_BR192_CROSS_PROCESS_OFD_OWNER";
    const EXPECT_SUCCESS_ENV: &str = "TEST_CODE_BR192_CROSS_PROCESS_OFD_EXPECT_SUCCESS";
    if std::env::var_os(CHILD_ENV).is_some() {
        let result = DurableDeliveryCoordinator::open(CoordinatorConfig::test(
            PathBuf::from(std::env::var(PATH_ENV).expect("child database path")),
            std::env::var(CODE_ENV).expect("child TEST_CODE"),
            std::env::var(OWNER_ENV).expect("child owner identity"),
        ));
        if std::env::var_os(EXPECT_SUCCESS_ENV).is_some() {
            result.expect("owner marker must be reusable after original owner drop");
        } else {
            assert!(matches!(
                result,
                Err(DurableDeliveryError::IsolationViolation(reason))
                    if reason.contains("cannot install SQLite main OFD marker")
            ));
        }
        return;
    }

    let mut fixture = Fixture::new("CROSS_PROCESS_OFD_OWNER_DROP");
    let test_code = fixture
        .database_path
        .parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
        .expect("test code")
        .to_owned();
    let owner = format!("owner-{test_code}-0123456789abcdef");
    let executable = std::env::current_exe().expect("resolve current TEST_CODE test binary");
    let database_path = Path::new(env!("CARGO_MANIFEST_DIR")).join(&fixture.database_path);
    let run_child = |expect_success: bool| {
        let mut command = std::process::Command::new(&executable);
        command
            .current_dir("/")
            .env(CHILD_ENV, "1")
            .env(PATH_ENV, &database_path)
            .env(CODE_ENV, &test_code)
            .env(OWNER_ENV, &owner)
            .args([
                "--exact",
                "durable_delivery::tests::br192_cross_process_ofd_owner_cannot_be_borrowed_and_is_reusable_after_drop",
                "--nocapture",
            ]);
        if expect_success {
            command.env(EXPECT_SUCCESS_ENV, "1");
        }
        command.status().expect("spawn cross-process OFD owner")
    };

    assert!(
        run_child(false).success(),
        "child must explicitly observe that a live owner-specific marker cannot be borrowed"
    );
    drop(fixture.coordinator.take());
    assert!(
        run_child(true).success(),
        "same deterministic owner marker must be reusable only after original owner drop"
    );
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br206_repeated_concurrent_coordinator_open_use_and_drop_preserves_attestation() {
    let fixture = Fixture::new("CONCURRENT_OPEN_DROP");
    let database_path = fixture.database_path.clone();
    let test_code = database_path
        .parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
        .expect("TEST_CODE namespace")
        .to_owned();
    // BR-206: repeat inside the committed regression so SQLite VFS descriptor
    // reuse is exercised by CI rather than only by an external stress loop.
    for round in 0..16 {
        let start = Arc::new(Barrier::new(5));
        let mut handles = Vec::new();
        for worker in 0..4 {
            let database_path = database_path.clone();
            let test_code = test_code.clone();
            let start = start.clone();
            handles.push(std::thread::spawn(move || {
                start.wait();
                let coordinator = DurableDeliveryCoordinator::open(CoordinatorConfig::test(
                    database_path,
                    test_code,
                    format!("owner-concurrent-{round}-{worker}-0123456789abcdef"),
                ))
                .expect("concurrent coordinator open");
                assert!(coordinator
                    .inspect_pending_for_date("2026-07-30")
                    .expect("concurrent coordinator operation")
                    .is_empty());
            }));
        }
        start.wait();
        for handle in handles {
            handle.join().expect("concurrent coordinator worker");
        }
        assert!(fixture
            .coordinator
            .inspect_pending_for_date("2026-07-30")
            .expect("original coordinator after concurrent open/drop")
            .is_empty());
    }
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br192_abrupt_process_exit_allows_exact_database_reopen() {
    const CHILD_ENV: &str = "TEST_CODE_BR192_CRASH_REOPEN_CHILD";
    const PATH_ENV: &str = "TEST_CODE_BR192_CRASH_REOPEN_PATH";
    const CODE_ENV: &str = "TEST_CODE_BR192_CRASH_REOPEN_CODE";
    if std::env::var_os(CHILD_ENV).is_some() {
        let database_path = PathBuf::from(std::env::var(PATH_ENV).expect("child database path"));
        let test_code = std::env::var(CODE_ENV).expect("child TEST_CODE");
        let coordinator = DurableDeliveryCoordinator::open(CoordinatorConfig::test(
            database_path,
            test_code,
            "owner-crash-child-0123456789abcdef",
        ))
        .expect("child coordinator open");
        assert!(coordinator
            .inspect_pending_for_date("2026-07-30")
            .expect("child operation before abrupt exit")
            .is_empty());
        std::process::exit(86);
    }

    let test_code = format!(
        "TEST_CODE_BR192_CRASH_REOPEN_{}_{}",
        std::process::id(),
        NEXT_TEST_ID.fetch_add(1, Ordering::SeqCst)
    );
    let fixture = PhysicalAliasFixture::new(&test_code);
    fixture.ensure_test_root();
    {
        let coordinator = fixture.open_test().expect("initialize exact database");
        drop(coordinator);
    }
    let status = std::process::Command::new(
        std::env::current_exe().expect("resolve current TEST_CODE test binary"),
    )
    .current_dir(env!("CARGO_MANIFEST_DIR"))
    .env(CHILD_ENV, "1")
    .env(PATH_ENV, fixture.test_database_path())
    .env(CODE_ENV, &test_code)
    .args([
        "--exact",
        "durable_delivery::tests::br192_abrupt_process_exit_allows_exact_database_reopen",
        "--nocapture",
    ])
    .status()
    .expect("spawn abrupt-exit child");
    assert_eq!(status.code(), Some(86), "child must exit without Drop");

    let reopened = fixture
        .open_test()
        .expect("exact database must reopen after abrupt process exit");
    assert!(reopened
        .inspect_pending_for_date("2026-07-30")
        .expect("operation after crash reopen")
        .is_empty());
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br192_ofd_marker_detects_same_inode_fd_number_aba() {
    let test_code = format!(
        "TEST_CODE_BR192_OFD_ABA_{}_{}",
        std::process::id(),
        NEXT_TEST_ID.fetch_add(1, Ordering::SeqCst)
    );
    let fixture = PhysicalAliasFixture::new(&test_code);
    fixture.create_test_sentinel();
    let database_path = fixture.test_database_path();
    let owner = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&database_path)
        .expect("open TEST_CODE OFD owner");
    let probe = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&database_path)
        .expect("open separate TEST_CODE OFD probe");
    let proof =
        OpenFileDescriptionProof::install_for_test(&owner, &probe).expect("install OFD proof");
    assert!(
        !proof
            .exclusive_probe_is_available_for_test(&probe)
            .expect("probe live marker"),
        "exclusive probe must conflict while the original OFD marker lives"
    );

    let released_descriptor = owner.as_raw_fd();
    drop(owner);
    let reused = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&database_path)
        .expect("reopen exact inode after owner close");
    assert_eq!(
        reused.as_raw_fd(),
        released_descriptor,
        "regression requires fd-number reuse"
    );
    assert!(matches!(
        proof.validate_descriptor_for_test(reused.as_raw_fd(), &probe),
        Err(DurableDeliveryError::IsolationViolation(_))
    ));
    assert!(
        proof
            .exclusive_probe_is_available_for_test(&probe)
            .expect("probe released marker"),
        "exclusive probe must succeed after the original OFD closes"
    );
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br192_ofd_marker_is_owner_specific_and_cannot_borrow_a_shared_holder() {
    let test_code = format!(
        "TEST_CODE_BR192_OFD_OWNER_{}_{}",
        std::process::id(),
        NEXT_TEST_ID.fetch_add(1, Ordering::SeqCst)
    );
    let fixture = PhysicalAliasFixture::new(&test_code);
    fixture.create_test_sentinel();
    let owner_a = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(fixture.test_database_path())
        .expect("open owner A");
    let duplicate_owner_identity = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(fixture.test_database_path())
        .expect("open duplicate owner identity");
    let owner_b = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(fixture.test_database_path())
        .expect("open owner B");
    let observer = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(fixture.test_database_path())
        .expect("open OFD observer");

    let proof_a = OpenFileDescriptionProof::install_with_owner_for_test(
        &owner_a,
        &observer,
        "TEST_CODE_OWNER_A_0123456789abcdef",
    )
    .expect("install owner A marker");
    assert!(matches!(
        OpenFileDescriptionProof::install_with_owner_for_test(
            &duplicate_owner_identity,
            &observer,
            "TEST_CODE_OWNER_A_0123456789abcdef",
        ),
        Err(DurableDeliveryError::IsolationViolation(_))
    ));
    let proof_b = OpenFileDescriptionProof::install_with_owner_for_test(
        &owner_b,
        &observer,
        "TEST_CODE_OWNER_B_0123456789abcdef",
    )
    .expect("different owner must use a disjoint deterministic marker");
    proof_a
        .validate_descriptor_for_test(owner_a.as_raw_fd(), &observer)
        .expect("owner A retains its own marker");
    proof_b
        .validate_descriptor_for_test(owner_b.as_raw_fd(), &observer)
        .expect("owner B retains its own marker");
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br192_malicious_preexisting_sidecar_fails_before_main_creation() {
    let test_code = format!(
        "TEST_CODE_BR192_PREEXISTING_SIDECAR_{}_{}",
        std::process::id(),
        NEXT_TEST_ID.fetch_add(1, Ordering::SeqCst)
    );
    let fixture = PhysicalAliasFixture::new(&test_code);
    fixture.ensure_test_root();
    fixture.create_foreign_sidecar_sentinel("-wal");
    fixture.create_symlink(
        fixture.foreign_sidecar_path("-wal"),
        &fixture.test_sidecar_path("-wal"),
    );

    assert!(matches!(
        fixture.open_test(),
        Err(DurableDeliveryError::IsolationViolation(_))
    ));
    assert!(
        !fixture.test_database_path().exists(),
        "hostile pre-existing sidecar must be rejected before main O_CREAT"
    );
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br192_construction_sidecar_swap_precedes_all_schema_and_policy_commits() {
    let test_code = format!(
        "TEST_CODE_BR192_CONSTRUCTION_SIDECAR_SWAP_{}_{}",
        std::process::id(),
        NEXT_TEST_ID.fetch_add(1, Ordering::SeqCst)
    );
    let fixture = PhysicalAliasFixture::new(&test_code);
    fixture.ensure_test_root();
    let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let wal = repository_root.join(fixture.test_sidecar_path("-wal"));
    let displaced = repository_root.join(format!(
        "{}.TEST_CODE_DISPLACED",
        fixture.test_sidecar_path("-wal").display()
    ));
    let callback_wal = wal.clone();
    let callback_displaced = displaced.clone();
    let _hook = install_database_bootstrap_test_hook(
        DatabaseBootstrapTestPhase::AfterMainReattestationBeforeSidecarAttestation,
        move || {
            std::fs::rename(&callback_wal, &callback_displaced)?;
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&callback_wal)?;
            Ok(())
        },
    )
    .expect("install construction sidecar swap");

    assert!(matches!(
        fixture.open_test(),
        Err(DurableDeliveryError::IsolationViolation(_))
    ));
    fixture
        .cleanup
        .record_if_present(&displaced, OwnedPathKind::FileOrSymlink);

    let replacement_identity =
        FilesystemIdentity::capture(&wal).expect("capture swapped WAL replacement");
    assert_eq!(
        FilesystemIdentity::capture(&wal).expect("revalidate swapped WAL replacement"),
        replacement_identity
    );
    std::fs::remove_file(&wal).expect("remove exact TEST_CODE swapped WAL replacement");
    std::fs::rename(&displaced, &wal).expect("restore exact TEST_CODE original WAL");
    fixture
        .cleanup
        .record_if_present(&wal, OwnedPathKind::FileOrSymlink);

    let connection =
        Connection::open(fixture.test_database_path()).expect("inspect failed bootstrap database");
    let schema_objects: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get(0),
        )
        .expect("count user schema objects");
    let user_version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("read user_version");
    assert_eq!(schema_objects, 0, "sidecar swap must precede all DDL");
    assert_eq!(user_version, 0, "sidecar swap must precede schema version");
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br192_main_swap_after_wal_before_reattestation_precedes_all_schema_and_policy_commits() {
    let test_code = format!(
        "TEST_CODE_BR192_POST_WAL_MAIN_SWAP_{}_{}",
        std::process::id(),
        NEXT_TEST_ID.fetch_add(1, Ordering::SeqCst)
    );
    let fixture = PhysicalAliasFixture::new(&test_code);
    fixture.ensure_test_root();
    let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let main = repository_root.join(fixture.test_database_path());
    let displaced = repository_root.join(format!(
        "{}.TEST_CODE_POST_WAL_DISPLACED",
        fixture.test_database_path().display()
    ));
    let callback_main = main.clone();
    let callback_displaced = displaced.clone();
    let _hook = install_database_bootstrap_test_hook(
        DatabaseBootstrapTestPhase::AfterWalMaterializationBeforeMainReattestation,
        move || {
            std::fs::rename(&callback_main, &callback_displaced)?;
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&callback_main)?;
            Ok(())
        },
    )
    .expect("install exact post-WAL/pre-reattestation main swap");

    assert!(matches!(
        fixture.open_test(),
        Err(DurableDeliveryError::IsolationViolation(_))
    ));
    fixture
        .cleanup
        .record_if_present(&displaced, OwnedPathKind::FileOrSymlink);

    std::fs::remove_file(&main).expect("remove exact TEST_CODE replacement main");
    std::fs::rename(&displaced, &main).expect("restore exact TEST_CODE original main");
    fixture
        .cleanup
        .record_if_present(&main, OwnedPathKind::FileOrSymlink);

    let connection = Connection::open(&main).expect("inspect rejected post-WAL bootstrap database");
    let schema_objects: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get(0),
        )
        .expect("count user schema objects after post-WAL rejection");
    let user_version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("read schema version after post-WAL rejection");
    assert_eq!(
        schema_objects, 0,
        "post-WAL main replacement must fail before DDL and policy rows"
    );
    assert_eq!(
        user_version, 0,
        "post-WAL main replacement must fail before schema version"
    );
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br192_schema_bootstrap_error_rolls_back_ddl_policy_and_user_version() {
    let test_code = format!(
        "TEST_CODE_BR192_SCHEMA_ROLLBACK_{}_{}",
        std::process::id(),
        NEXT_TEST_ID.fetch_add(1, Ordering::SeqCst)
    );
    let fixture = PhysicalAliasFixture::new(&test_code);
    fixture.ensure_test_root();
    let _hook = install_database_bootstrap_test_hook(
        DatabaseBootstrapTestPhase::AfterSchemaSqlBeforeCommitValidation,
        || {
            Err(DurableDeliveryError::IsolationViolation(
                "TEST_CODE reject schema bootstrap before commit".to_owned(),
            ))
        },
    )
    .expect("install schema bootstrap rollback fault");

    assert!(matches!(
        fixture.open_test(),
        Err(DurableDeliveryError::IsolationViolation(reason))
            if reason.contains("reject schema bootstrap before commit")
    ));
    let connection =
        Connection::open(fixture.test_database_path()).expect("inspect rolled-back bootstrap");
    let schema_objects: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get(0),
        )
        .expect("count rolled-back user schema objects");
    let user_version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("read rolled-back user_version");
    assert_eq!(schema_objects, 0, "DDL must roll back with bootstrap");
    assert_eq!(user_version, 0, "schema version must roll back");
}

#[test]
fn br194_schema_v5_migration_matrix_is_repeatable_and_rejects_newer_versions() {
    let mut fresh = Connection::open_in_memory().expect("open fresh schema regression database");
    initialize_test_schema(&mut fresh).expect("fresh v0 initializes directly to v5");
    assert_eq!(
        fresh
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .expect("read fresh schema version"),
        super::schema::SCHEMA_VERSION
    );
    initialize_test_schema(&mut fresh).expect("v5 initialization is idempotent");
    let fresh_manifest = schema_manifest_for_test(&fresh);

    for legacy_version in [1_i64, 2_i64, 3_i64, 4_i64] {
        let mut connection =
            Connection::open_in_memory().expect("open legacy migration regression database");
        initialize_test_schema(&mut connection).expect("materialize complete reference schema");
        if matches!(legacy_version, 1 | 2) {
            downgrade_manual_resolution_schema_for_test(&mut connection, legacy_version);
        } else if legacy_version == 4 {
            downgrade_replay_schema_v4_for_test(&mut connection, false);
        } else {
            connection
                .pragma_update(None, "user_version", legacy_version)
                .expect("set legacy schema version");
        }
        initialize_test_schema(&mut connection).unwrap_or_else(|error| {
            panic!("schema v{legacy_version} must migrate through v5: {error}")
        });
        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .expect("read migrated schema version"),
            super::schema::SCHEMA_VERSION
        );
        let replay_trigger_sql: String = connection
            .query_row(
                "SELECT sql FROM sqlite_master
                 WHERE type='trigger'
                   AND name='validate_review_terminal_replay_attempt_audit_insert'",
                [],
                |row| row.get(0),
            )
            .expect("read replay start authority trigger");
        assert!(
            replay_trigger_sql.contains("sha256_hex(NEW.start_canonical)=NEW.start_sha256")
                && replay_trigger_sql
                    .contains("sha256_hex(audit.audit_canonical)=audit.audit_sha256"),
            "v{legacy_version} migration must install v5 hash recomputation"
        );
        assert_eq!(
            schema_manifest_for_test(&connection),
            fresh_manifest,
            "schema v{legacy_version} must converge to the fresh v5 manifest"
        );
        if legacy_version == 4 {
            let preserved: (i64, i64, Option<String>, i64) = connection
                .query_row(
                    "SELECT
                       (SELECT COUNT(*) FROM delivery_decisions
                         WHERE decision_identity='TEST_CODE_V4_DECISION'),
                       (SELECT COUNT(*) FROM immutable_audit_outbox
                         WHERE audit_identity IN (
                           'TEST_CODE_V4_AUDIT','TEST_CODE_V4_AUDIT_CHILD'
                         )),
                       (SELECT predecessor_audit_identity
                          FROM immutable_audit_outbox
                         WHERE audit_identity='TEST_CODE_V4_AUDIT_CHILD'),
                       (SELECT COUNT(*) FROM review_terminal_replay_attempts)",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .expect("read replay-absent historical v4 preservation");
            assert_eq!(preserved, (1, 2, Some("TEST_CODE_V4_AUDIT".to_owned()), 0));
            let outbox_self_fk_count: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM pragma_foreign_key_list(
                       'immutable_audit_outbox'
                     )
                     WHERE \"table\"='immutable_audit_outbox'
                       AND \"from\"='predecessor_audit_identity'
                       AND \"to\"='audit_identity'",
                    [],
                    |row| row.get(0),
                )
                .expect("read migrated immutable-audit self FK");
            assert_eq!(outbox_self_fk_count, 1);
            let foreign_key_violation_count: i64 = connection
                .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                    row.get(0)
                })
                .expect("check migrated historical v4 foreign keys");
            assert_eq!(foreign_key_violation_count, 0);
        }
        initialize_test_schema(&mut connection)
            .unwrap_or_else(|error| panic!("migrated v5 must reopen idempotently: {error}"));
    }

    let mut replay_present =
        Connection::open_in_memory().expect("open replay-present v4 regression database");
    initialize_test_schema(&mut replay_present).expect("materialize v5 fixture base");
    downgrade_replay_schema_v4_for_test(&mut replay_present, true);
    initialize_test_schema(&mut replay_present)
        .expect("replay-present schema v4 must migrate to v5");
    assert_eq!(schema_manifest_for_test(&replay_present), fresh_manifest);
    let preserved: (i64, i64, String) = replay_present
        .query_row(
            "SELECT
               (SELECT COUNT(*) FROM immutable_audit_outbox
                 WHERE audit_identity='TEST_CODE_V4_REPLAY_AUDIT'),
               (SELECT COUNT(*) FROM review_terminal_replay_attempts
                 WHERE attempt_identity='TEST_CODE_V4_REPLAY_ATTEMPT'),
               (SELECT start_sha256 FROM review_terminal_replay_attempts
                 WHERE attempt_identity='TEST_CODE_V4_REPLAY_ATTEMPT')",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("read replay-present historical v4 preservation");
    assert_eq!(preserved.0, 1);
    assert_eq!(preserved.1, 1);
    assert_eq!(preserved.2.len(), 64);

    let mut corrupt =
        Connection::open_in_memory().expect("open corrupt historical v4 regression database");
    initialize_test_schema(&mut corrupt).expect("materialize v5 fixture for corrupt v4 rollback");
    downgrade_replay_schema_v4_for_test(&mut corrupt, false);
    corrupt
        .pragma_update(None, "foreign_keys", "OFF")
        .expect("disable FK only to construct corrupt historical TEST_CODE fixture");
    corrupt
        .execute(
            "UPDATE immutable_audit_outbox
                SET predecessor_audit_identity='TEST_CODE_V4_MISSING_AUDIT'
              WHERE audit_identity='TEST_CODE_V4_AUDIT_CHILD'",
            [],
        )
        .expect("construct corrupt historical v4 predecessor");
    corrupt
        .pragma_update(None, "foreign_keys", "ON")
        .expect("restore FK enforcement before corrupt migration");
    let corrupt_manifest_before = schema_manifest_for_test(&corrupt);
    assert!(matches!(
        initialize_test_schema(&mut corrupt),
        Err(DurableDeliveryError::InvalidConfiguration(reason))
            if reason.contains("foreign-key violation")
    ));
    assert_eq!(
        corrupt
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .expect("read rolled-back corrupt historical schema version"),
        4
    );
    assert_eq!(
        schema_manifest_for_test(&corrupt),
        corrupt_manifest_before,
        "failed corrupt migration must roll back all DDL"
    );
    let rolled_back_chain: (i64, Option<String>) = corrupt
        .query_row(
            "SELECT
               (SELECT COUNT(*) FROM immutable_audit_outbox),
               (SELECT predecessor_audit_identity
                  FROM immutable_audit_outbox
                 WHERE audit_identity='TEST_CODE_V4_AUDIT_CHILD')",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read rolled-back corrupt historical chain");
    assert_eq!(
        rolled_back_chain,
        (2, Some("TEST_CODE_V4_MISSING_AUDIT".to_owned()))
    );

    let mut newer = Connection::open_in_memory().expect("open newer schema regression database");
    initialize_test_schema(&mut newer).expect("materialize complete reference schema");
    newer
        .pragma_update(None, "user_version", super::schema::SCHEMA_VERSION + 1)
        .expect("set unsupported newer schema version");
    assert!(matches!(
        initialize_test_schema(&mut newer),
        Err(DurableDeliveryError::InvalidConfiguration(reason))
            if reason.contains("newer than supported")
    ));
}

#[test]
fn runtime_schema_guard_rejects_visible_drift_before_read_and_write_callbacks() {
    let fixture = Fixture::new("RUNTIME_SCHEMA_VISIBLE_DRIFT");
    let existing = envelope(
        "RUNTIME_SCHEMA_VISIBLE_DRIFT_EXISTING",
        PushKind::ReviewProviderTopN,
        DeliverySubKind::None,
        "2026-07-30",
        true,
    );
    fixture
        .coordinator
        .prepare(&existing, 1, now())
        .expect("prepare actual decision before schema drift");

    let drifted_version = super::schema::SCHEMA_VERSION - 1;
    let drift_connection =
        Connection::open(&fixture.database_path).expect("open isolated schema drift connection");
    drift_connection
        .pragma_update(None, "user_version", drifted_version)
        .expect("drift isolated database schema version");
    drop(drift_connection);

    let read = fixture
        .coordinator
        .decision_state(&existing.decision_identity);
    let new_candidate = envelope(
        "RUNTIME_SCHEMA_VISIBLE_DRIFT_NEW",
        PushKind::ReviewProviderTopN,
        DeliverySubKind::None,
        "2026-07-30",
        true,
    );
    let write = fixture.coordinator.prepare(&new_candidate, 1, now());

    assert!(matches!(
        read,
        Err(DurableDeliveryError::InvalidConfiguration(reason))
            if reason.contains("schema version")
    ));
    assert!(matches!(
        write,
        Err(DurableDeliveryError::InvalidConfiguration(reason))
            if reason.contains("schema version")
    ));
    assert_eq!(
        fixture.query_i64(&format!(
            "SELECT COUNT(*) FROM delivery_decisions WHERE decision_identity='{}'",
            new_candidate.decision_identity
        )),
        0,
        "visible drift must be rejected before the write callback changes business data"
    );
}

#[test]
fn runtime_schema_guard_rejects_zero_legacy_and_newer_versions_until_restored() {
    for (label, drifted_version) in [
        ("ZERO", 0),
        ("LEGACY", 4),
        ("NEWER", super::schema::SCHEMA_VERSION + 1),
    ] {
        let fixture = Fixture::new(&format!("RUNTIME_SCHEMA_MATRIX_{label}"));
        let candidate = envelope(
            &format!("RUNTIME_SCHEMA_MATRIX_{label}"),
            PushKind::ReviewProviderTopN,
            DeliverySubKind::None,
            "2026-07-30",
            true,
        );
        fixture
            .coordinator
            .prepare(&candidate, 1, now())
            .expect("prepare actual decision before matrix drift");
        set_isolated_schema_version(&fixture.database_path, drifted_version);

        assert_schema_version_error(
            fixture
                .coordinator
                .decision_state(&candidate.decision_identity),
            drifted_version,
        );

        set_isolated_schema_version(&fixture.database_path, super::schema::SCHEMA_VERSION);
        assert_eq!(
            fixture
                .coordinator
                .decision_state(&candidate.decision_identity)
                .expect("runtime read succeeds after restoring current schema version"),
            DecisionState::Reserved
        );
    }
}

#[test]
fn runtime_schema_guard_rechecks_after_operation_prevalidation_before_read_callback() {
    let fixture = Fixture::new("RUNTIME_SCHEMA_AFTER_OPERATION_PREVALIDATION");
    let candidate = envelope(
        "RUNTIME_SCHEMA_AFTER_OPERATION_PREVALIDATION",
        PushKind::ReviewProviderTopN,
        DeliverySubKind::None,
        "2026-07-30",
        true,
    );
    fixture
        .coordinator
        .prepare(&candidate, 1, now())
        .expect("prepare actual decision before prevalidation drift");
    let database_path = fixture.database_path.clone();
    let drifted_version = super::schema::SCHEMA_VERSION - 1;
    let coordinator = fixture_coordinator_arc(&fixture);
    let callback_count = Arc::new(AtomicUsize::new(0));
    let callback_count_by_probe = callback_count.clone();
    fixture
        .coordinator
        .install_database_operation_test_hook(
            DatabaseOperationTestPhase::AfterPreValidationBeforeSql,
            move || {
                set_isolated_schema_version(&database_path, drifted_version);
                coordinator.install_database_operation_test_hook(
                    DatabaseOperationTestPhase::AfterSqlBeforePostValidation,
                    move || {
                        callback_count_by_probe.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    },
                )?;
                Ok(())
            },
        )
        .expect("install drift after operation prevalidation");

    assert_schema_version_error(
        fixture
            .coordinator
            .decision_state(&candidate.decision_identity),
        drifted_version,
    );
    assert_eq!(
        callback_count.load(Ordering::SeqCst),
        0,
        "the read callback must not reach its post-SQL checkpoint after schema drift"
    );

    set_isolated_schema_version(&fixture.database_path, super::schema::SCHEMA_VERSION);
    let restored_state = fixture
        .coordinator
        .decision_state(&candidate.decision_identity)
        .expect("read succeeds after restoring current schema version");
    assert_eq!(restored_state, DecisionState::Reserved);
    assert_eq!(
        callback_count.load(Ordering::SeqCst),
        1,
        "the restored read must prove the post-SQL checkpoint is reachable"
    );
}

#[test]
fn runtime_schema_guard_rechecks_after_begin_immediate_before_write_callback() {
    let fixture = Fixture::new("RUNTIME_SCHEMA_AFTER_OUTER_CHECK");
    let candidate = envelope(
        "RUNTIME_SCHEMA_AFTER_OUTER_CHECK",
        PushKind::ReviewProviderTopN,
        DeliverySubKind::None,
        "2026-07-30",
        true,
    );
    let database_path = fixture.database_path.clone();
    let drifted_version = super::schema::SCHEMA_VERSION - 1;
    let coordinator = fixture_coordinator_arc(&fixture);
    let callback_count = Arc::new(AtomicUsize::new(0));
    let callback_count_by_probe = callback_count.clone();
    fixture
        .coordinator
        .install_database_operation_test_hook(
            DatabaseOperationTestPhase::AfterConnectionSchemaValidationBeforeTransaction,
            move || {
                set_isolated_schema_version(&database_path, drifted_version);
                coordinator.install_database_operation_test_hook(
                    DatabaseOperationTestPhase::AfterSqlBeforePreCommitValidation,
                    move || {
                        callback_count_by_probe.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    },
                )?;
                Ok(())
            },
        )
        .expect("install drift after outer validation");

    assert_schema_version_error(
        fixture.coordinator.prepare(&candidate, 1, now()),
        drifted_version,
    );
    assert_eq!(
        fixture.query_i64(&format!(
            "SELECT COUNT(*) FROM delivery_decisions WHERE decision_identity='{}'",
            candidate.decision_identity
        )),
        0,
        "the transaction callback must not run after the post-BEGIN check rejects drift"
    );
    assert_eq!(
        callback_count.load(Ordering::SeqCst),
        0,
        "the write callback must not reach its post-SQL checkpoint after schema drift"
    );

    set_isolated_schema_version(&fixture.database_path, super::schema::SCHEMA_VERSION);
    let restored = fixture
        .coordinator
        .prepare(&candidate, 1, now())
        .expect("prepare succeeds after restoring current schema version");
    assert_eq!(restored.state, DecisionState::Reserved);
    assert_eq!(
        callback_count.load(Ordering::SeqCst),
        1,
        "the restored write must prove the post-SQL checkpoint is reachable"
    );
}

#[test]
fn runtime_schema_guard_rolls_back_business_audit_and_version_drift_before_commit() {
    let fixture = Fixture::new("RUNTIME_SCHEMA_PRECOMMIT_ROLLBACK");
    let candidate = envelope(
        "RUNTIME_SCHEMA_PRECOMMIT_ROLLBACK",
        PushKind::ReviewProviderTopN,
        DeliverySubKind::None,
        "2026-07-30",
        true,
    );
    let tables = [
        "delivery_decisions",
        "delivery_state_events",
        "immutable_audit_outbox",
        "cooldown_reservations",
        "daily_budget_reservations",
    ];
    let before_connection =
        Connection::open(&fixture.database_path).expect("open precommit baseline connection");
    let before = authority_snapshot(&before_connection, &tables);
    drop(before_connection);
    let drifted_version = super::schema::SCHEMA_VERSION - 1;
    fixture
        .coordinator
        .install_operation_postvalidation_test_fault(
            OperationPostvalidationTestFault::SchemaVersion(drifted_version),
        )
        .expect("install transaction-local schema drift");

    assert_schema_version_error(
        fixture.coordinator.prepare(&candidate, 1, now()),
        drifted_version,
    );

    let after_connection =
        Connection::open(&fixture.database_path).expect("open precommit rollback connection");
    assert_eq!(
        after_connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .expect("read schema version after rollback"),
        super::schema::SCHEMA_VERSION,
        "transaction-local schema drift must roll back"
    );
    assert_eq!(
        authority_snapshot(&after_connection, &tables),
        before,
        "business rows, reservations, audit rows, and schema version must not partially commit"
    );
    drop(after_connection);

    assert_eq!(
        fixture
            .coordinator
            .prepare(&candidate, 1, now())
            .expect("same write succeeds after one-shot drift rollback")
            .state,
        DecisionState::Reserved
    );
}

#[test]
fn runtime_schema_guard_rejects_read_when_version_drifts_after_callback() {
    let fixture = Fixture::new("RUNTIME_SCHEMA_READ_POSTCHECK");
    let candidate = envelope(
        "RUNTIME_SCHEMA_READ_POSTCHECK",
        PushKind::ReviewProviderTopN,
        DeliverySubKind::None,
        "2026-07-30",
        true,
    );
    fixture
        .coordinator
        .prepare(&candidate, 1, now())
        .expect("prepare actual decision before read postcheck");
    let database_path = fixture.database_path.clone();
    let drifted_version = super::schema::SCHEMA_VERSION - 1;
    fixture
        .coordinator
        .install_database_operation_test_hook(
            DatabaseOperationTestPhase::AfterSqlBeforePostValidation,
            move || {
                set_isolated_schema_version(&database_path, drifted_version);
                Ok(())
            },
        )
        .expect("install drift after read callback");

    assert_schema_version_error(
        fixture
            .coordinator
            .decision_state(&candidate.decision_identity),
        drifted_version,
    );

    set_isolated_schema_version(&fixture.database_path, super::schema::SCHEMA_VERSION);
    assert_eq!(
        fixture
            .coordinator
            .decision_state(&candidate.decision_identity)
            .expect("read succeeds after restoring current schema version"),
        DecisionState::Reserved
    );
}

#[test]
fn runtime_schema_guard_reconcile_visible_drift_has_no_database_or_append_effect() {
    let fixture = Fixture::new("RUNTIME_SCHEMA_RECONCILE_VISIBLE_DRIFT");
    let candidate = envelope(
        "RUNTIME_SCHEMA_RECONCILE_VISIBLE_DRIFT",
        PushKind::ReviewProviderTopN,
        DeliverySubKind::None,
        "2026-07-30",
        true,
    );
    fixture
        .coordinator
        .prepare(&candidate, 1, now())
        .expect("prepare pending audit before visible drift");
    let tables = [
        "delivery_decisions",
        "delivery_state_events",
        "immutable_audit_outbox",
        "cooldown_reservations",
        "daily_budget_reservations",
    ];
    let before_connection =
        Connection::open(&fixture.database_path).expect("open reconcile baseline connection");
    let before = authority_snapshot(&before_connection, &tables);
    drop(before_connection);
    let drifted_version = super::schema::SCHEMA_VERSION - 1;
    set_isolated_schema_version(&fixture.database_path, drifted_version);
    let append = MemoryAppendPort::default();

    assert_schema_version_error(
        fixture.coordinator.reconcile_all_pending(&append, now()),
        drifted_version,
    );
    assert_eq!(
        append.record_count(),
        0,
        "visible drift must be rejected before any external append"
    );
    let after_connection =
        Connection::open(&fixture.database_path).expect("open rejected reconcile snapshot");
    assert_eq!(authority_snapshot(&after_connection, &tables), before);
    drop(after_connection);

    set_isolated_schema_version(&fixture.database_path, super::schema::SCHEMA_VERSION);
    let retry = fixture
        .coordinator
        .reconcile_all_pending(&append, now())
        .expect("reconcile succeeds after restoring current schema version");
    assert!(retry.progress_count > 0);
    assert!(append.record_count() > 0);
    assert_eq!(
        fixture
            .coordinator
            .decision_state(&candidate.decision_identity)
            .expect("public read succeeds after restored reconcile"),
        DecisionState::Reserved
    );
}

#[test]
fn runtime_schema_guard_external_append_survives_rejected_database_ack() {
    let fixture = Fixture::new("RUNTIME_SCHEMA_AFTER_EXTERNAL_APPEND");
    let candidate = envelope(
        "RUNTIME_SCHEMA_AFTER_EXTERNAL_APPEND",
        PushKind::ReviewProviderTopN,
        DeliverySubKind::None,
        "2026-07-30",
        true,
    );
    fixture
        .coordinator
        .prepare(&candidate, 1, now())
        .expect("prepare pending audit before append boundary drift");
    let drifted_version = super::schema::SCHEMA_VERSION - 1;
    let append = SchemaDriftAfterAppend::new(
        fixture_coordinator_arc(&fixture),
        "DecisionStateChanged",
        drifted_version,
    );

    assert_schema_version_error(
        fixture.coordinator.reconcile_all_pending(&append, now()),
        drifted_version,
    );
    assert_eq!(
        append.inner.count_kind("DecisionStateChanged"),
        1,
        "an already successful external append cannot be rolled back"
    );
    assert_eq!(
        fixture.query_i64(
            "SELECT COUNT(*) FROM immutable_audit_outbox
             WHERE audit_kind='DecisionStateChanged'
               AND append_state='Pending' AND immutable_audit_ref IS NULL"
        ),
        1,
        "the database acknowledgement and transaction-local version drift must roll back"
    );
    assert_eq!(
        fixture.query_i64("PRAGMA user_version"),
        super::schema::SCHEMA_VERSION
    );

    fixture
        .coordinator
        .reconcile_all_pending(&append, now())
        .expect("idempotent retry acknowledges the existing external append");
    assert_eq!(
        append.inner.count_kind("DecisionStateChanged"),
        1,
        "retry must reuse the same exact external append"
    );
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn runtime_schema_guard_final_open_boundary_rejects_version_drift() {
    let test_code = format!(
        "TEST_CODE_RUNTIME_SCHEMA_FINAL_OPEN_{}_{}",
        std::process::id(),
        NEXT_TEST_ID.fetch_add(1, Ordering::SeqCst)
    );
    let fixture = PhysicalAliasFixture::new(&test_code);
    fixture.ensure_test_root();
    let database_path = fixture.test_database_path();
    let callback_database_path = database_path.clone();
    let drifted_version = super::schema::SCHEMA_VERSION - 1;
    let _hook = install_database_bootstrap_test_hook(
        DatabaseBootstrapTestPhase::AfterFinalParentSyncBeforeSuccessValidation,
        move || {
            set_isolated_schema_version(&callback_database_path, drifted_version);
            Ok(())
        },
    )
    .expect("install final-open schema drift");

    assert_schema_version_error(fixture.open_test(), drifted_version);
    assert_eq!(
        Connection::open(&database_path)
            .expect("open rejected final bootstrap database")
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .expect("read rejected final bootstrap version"),
        drifted_version,
        "the external version change occurs after bootstrap committed"
    );
}

fn set_isolated_schema_version(database_path: &Path, version: i64) {
    Connection::open(database_path)
        .expect("open isolated schema-version connection")
        .pragma_update(None, "user_version", version)
        .expect("set isolated schema version");
}

fn assert_schema_version_error<T>(result: Result<T>, observed_version: i64) {
    match result {
        Err(DurableDeliveryError::InvalidConfiguration(reason)) => {
            assert!(
                reason.contains("schema version"),
                "unexpected error: {reason}"
            );
            assert!(
                reason.contains(&observed_version.to_string()),
                "schema error must report observed version {observed_version}: {reason}"
            );
            assert!(
                reason.contains(&super::schema::SCHEMA_VERSION.to_string()),
                "schema error must report required version {}: {reason}",
                super::schema::SCHEMA_VERSION
            );
        }
        Err(error) => panic!("expected schema-version configuration error, got {error}"),
        Ok(_) => panic!("schema version {observed_version} unexpectedly reached callback"),
    }
}

#[test]
fn audit_v4_upgrade_formal_open_rejects_missing_predecessor_and_rolls_back() {
    let mut fixture = Fixture::new("AUDIT_V4_UPGRADE_MISSING_PREDECESSOR");
    let append = MemoryAppendPort::default();
    let candidate = w12_foundation_envelope("AUDIT_V4_UPGRADE_MISSING_PREDECESSOR");
    prepare_reserved(&fixture, &candidate, &append);
    let test_code = fixture
        .database_path
        .parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
        .expect("isolated missing-predecessor TEST_CODE root")
        .to_owned();
    let config = CoordinatorConfig::test(
        &fixture.database_path,
        &test_code,
        "TEST_CODE_V4_MISSING_PREDECESSOR_OWNER_0123456789ABCDEF",
    );
    let coordinator = fixture
        .coordinator
        .take()
        .expect("take coordinator before corrupt historical shaping");
    assert_eq!(Arc::strong_count(&coordinator), 1);
    drop(coordinator);

    let mut historical = Connection::open(&fixture.database_path)
        .expect("open isolated missing-predecessor historical database");
    downgrade_replay_schema_v4_for_test(&mut historical, false);
    historical
        .pragma_update(None, "foreign_keys", "OFF")
        .expect("disable FK only while constructing missing-predecessor history");
    assert_eq!(
        historical
            .execute(
                "UPDATE immutable_audit_outbox
                    SET predecessor_audit_identity='TEST_CODE_V4_MISSING_PREDECESSOR'
                  WHERE audit_identity='TEST_CODE_V4_AUDIT_CHILD'",
                [],
            )
            .expect("construct one missing historical predecessor"),
        1
    );
    historical
        .pragma_update(None, "foreign_keys", "ON")
        .expect("restore FK enforcement before formal open");
    let violations_before = foreign_key_violation_details(&historical);
    assert_eq!(violations_before.len(), 1);
    assert!(violations_before[0].contains("table=immutable_audit_outbox"));
    assert!(violations_before[0].contains("reported_parent=immutable_audit_outbox"));
    let before = audit_v4_upgrade_database_snapshot(&historical);
    assert_eq!(before.0, 4);
    drop(historical);

    let error = match DurableDeliveryCoordinator::open(config) {
        Ok(_) => panic!("formal open must reject a missing historical predecessor"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        DurableDeliveryError::InvalidConfiguration(reason)
            if reason.contains("foreign-key violation")
    ));

    let rolled_back = Connection::open(&fixture.database_path)
        .expect("inspect formal-open missing-predecessor rollback");
    rolled_back
        .pragma_update(None, "foreign_keys", "ON")
        .expect("enforce FK while inspecting missing-predecessor rollback");
    assert_eq!(
        audit_v4_upgrade_database_snapshot(&rolled_back),
        before,
        "formal-open failure must preserve v4, raw DDL, every typed row, and every audit rowid"
    );
    assert_eq!(
        foreign_key_violation_details(&rolled_back),
        violations_before
    );
}

#[test]
fn audit_v4_upgrade_formal_open_rejects_non_outbox_foreign_key_and_rolls_back() {
    let mut fixture = Fixture::new("AUDIT_V4_UPGRADE_NON_OUTBOX_FOREIGN_KEY");
    let append = MemoryAppendPort::default();
    let candidate = envelope(
        "AUDIT_V4_UPGRADE_NON_OUTBOX_FOREIGN_KEY",
        PushKind::HoldingPlan,
        DeliverySubKind::None,
        "2026-07-30",
        false,
    );
    prepare_reserved(&fixture, &candidate, &append);
    let test_code = fixture
        .database_path
        .parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
        .expect("isolated non-outbox TEST_CODE root")
        .to_owned();
    let config = CoordinatorConfig::test(
        &fixture.database_path,
        &test_code,
        "TEST_CODE_V4_NON_OUTBOX_OWNER_0123456789ABCDEF",
    );
    let coordinator = fixture
        .coordinator
        .take()
        .expect("take coordinator before corrupt non-outbox historical shaping");
    assert_eq!(Arc::strong_count(&coordinator), 1);
    drop(coordinator);

    let mut historical = Connection::open(&fixture.database_path)
        .expect("open isolated non-outbox historical database");
    downgrade_replay_schema_v4_for_test(&mut historical, false);
    assert_eq!(
        historical
            .query_row(
                "SELECT COUNT(*)
                 FROM cooldown_heads head
                 JOIN cooldown_reservations reservation
                   ON reservation.cooldown_reservation_identity=
                      head.current_reservation_identity
                 WHERE reservation.decision_identity=?1",
                [candidate.decision_identity.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .expect("verify the real Rolling policy API created one cooldown head"),
        1
    );
    historical
        .pragma_update(None, "foreign_keys", "OFF")
        .expect("disable FK only while constructing non-outbox corrupt history");
    assert_eq!(
        historical
            .execute(
                "UPDATE cooldown_heads
                    SET current_reservation_identity='TEST_CODE_V4_MISSING_COOLDOWN_RESERVATION'
                  WHERE current_reservation_identity=(
                    SELECT current_cooldown_reservation_identity
                    FROM delivery_decisions WHERE decision_identity=?1
                  )",
                [candidate.decision_identity.as_str()],
            )
            .expect("construct one non-outbox historical FK violation"),
        1
    );
    historical
        .pragma_update(None, "foreign_keys", "ON")
        .expect("restore FK enforcement before formal non-outbox open");
    let violations_before = foreign_key_violation_details(&historical);
    assert_eq!(violations_before.len(), 1);
    assert!(violations_before[0].contains("table=cooldown_heads"));
    assert!(violations_before[0].contains("reported_parent=cooldown_reservations"));
    let before = audit_v4_upgrade_database_snapshot(&historical);
    assert_eq!(before.0, 4);
    drop(historical);

    let error = match DurableDeliveryCoordinator::open(config) {
        Ok(_) => panic!("formal open must reject a non-outbox historical FK violation"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        DurableDeliveryError::InvalidConfiguration(reason)
            if reason.contains("foreign-key violation")
    ));

    let rolled_back =
        Connection::open(&fixture.database_path).expect("inspect formal-open non-outbox rollback");
    rolled_back
        .pragma_update(None, "foreign_keys", "ON")
        .expect("enforce FK while inspecting non-outbox rollback");
    assert_eq!(
        audit_v4_upgrade_database_snapshot(&rolled_back),
        before,
        "formal-open failure must preserve v4, raw DDL, every typed row, and every audit rowid"
    );
    assert_eq!(
        foreign_key_violation_details(&rolled_back),
        violations_before
    );
}

#[test]
fn audit_v4_upgrade_defer_reset_does_not_hide_later_commit_violation() {
    let mut connection =
        Connection::open_in_memory().expect("open isolated deferred-FK migration database");
    initialize_test_schema(&mut connection).expect("materialize reference schema");
    downgrade_replay_schema_v4_for_test(&mut connection, false);
    let before = audit_v4_upgrade_database_snapshot(&connection);
    assert_eq!(before.0, 4);
    assert!(foreign_key_violation_details(&connection).is_empty());

    let transaction = connection
        .transaction()
        .expect("begin direct schema migration transaction");
    super::schema::initialize_schema(&transaction)
        .expect("migrate valid v4 history and reset only its stale deferred counter");
    assert_eq!(
        transaction
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .expect("read migrated version inside transaction"),
        super::schema::SCHEMA_VERSION
    );
    assert_eq!(
        transaction
            .pragma_query_value(None, "foreign_keys", |row| row.get::<_, i64>(0))
            .expect("verify FK enforcement stays enabled"),
        1
    );
    assert_eq!(
        transaction
            .pragma_query_value(None, "defer_foreign_keys", |row| row.get::<_, i64>(0))
            .expect("verify migration restores deferred enforcement"),
        1
    );
    assert!(foreign_key_violation_details(&transaction).is_empty());

    let canonical = br#"{"reason":"TEST_CODE_POST_RESET_DEFERRED_FK"}"#;
    let digest = sha256_hex(canonical);
    transaction
        .execute(
            "INSERT INTO immutable_audit_outbox(
               audit_identity,decision_identity,attempt_identity,audit_kind,
               predecessor_audit_identity,audit_canonical,audit_sha256,
               append_state,immutable_audit_ref,created_at
             ) VALUES(
               'TEST_CODE_POST_RESET_DEFERRED_AUDIT','TEST_CODE_V4_DECISION',NULL,
               'DecisionIdentityConflict','TEST_CODE_POST_RESET_MISSING_PREDECESSOR',
               ?1,?2,'Pending',NULL,'2026-07-29T22:00:00Z'
             )",
            params![canonical.as_slice(), digest],
        )
        .expect("defer a new real FK violation after the guarded counter reset");
    let deferred_violations = foreign_key_violation_details(&transaction);
    assert_eq!(deferred_violations.len(), 1);
    assert!(deferred_violations[0].contains("table=immutable_audit_outbox"));
    assert!(deferred_violations[0].contains("reported_parent=immutable_audit_outbox"));

    let commit_error = transaction
        .execute_batch("COMMIT")
        .expect_err("new deferred FK violation must still reject COMMIT");
    assert!(matches!(
        commit_error,
        rusqlite::Error::SqliteFailure(error, _)
            if error.extended_code == 787
    ));
    assert!(
        !transaction.is_autocommit(),
        "failed deferred-FK COMMIT must leave the transaction open for rollback"
    );
    transaction
        .execute_batch("ROLLBACK")
        .expect("roll back the rejected post-reset migration transaction");
    drop(transaction);

    assert_eq!(
        audit_v4_upgrade_database_snapshot(&connection),
        before,
        "rejected post-reset COMMIT must restore v4, raw DDL, every typed row, and every audit rowid"
    );
    assert!(foreign_key_violation_details(&connection).is_empty());
}

#[test]
fn audit_logical_tail_recovers_after_real_v4_upgrade() {
    const FIXED_OWNER: &str = "TEST_CODE_REAL_V4_UPGRADE_OWNER_0123456789ABCDEF";
    const HEARTBEAT_SAMPLE_BOUND: i64 = 16;

    let mut fixture = Fixture::new("AUDIT_LOGICAL_TAIL_REAL_V4_UPGRADE");
    let test_code = fixture
        .database_path
        .parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
        .expect("isolated TEST_CODE root")
        .to_owned();
    let coordinator_config =
        CoordinatorConfig::test(&fixture.database_path, &test_code, FIXED_OWNER);
    let bootstrap = fixture
        .coordinator
        .take()
        .expect("take bootstrap coordinator before fixed-owner reopen");
    assert_eq!(
        Arc::strong_count(&bootstrap),
        1,
        "the fixed-owner reopen must not retain a hidden coordinator Arc"
    );
    drop(bootstrap);
    fixture.coordinator = FixtureCoordinator(Some(Arc::new(
        DurableDeliveryCoordinator::open(coordinator_config.clone())
            .expect("reopen isolated coordinator with a deterministic owner"),
    )));

    let append = MemoryAppendPort::default();
    let candidate = w12_foundation_envelope("AUDIT_LOGICAL_TAIL_REAL_V4_UPGRADE");
    prepare_reserved(&fixture, &candidate, &append);
    let attempt = fixture
        .coordinator
        .begin_attempt(&candidate.decision_identity, 1, now())
        .expect("begin real Foundation-bound attempt")
        .expect("real Foundation-bound attempt lease");
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM sink_results"), 0);

    let mut ordering_witness = None;
    for offset_seconds in 1..=HEARTBEAT_SAMPLE_BOUND {
        let heartbeat_at = now() + chrono::Duration::seconds(offset_seconds);
        assert!(fixture
            .coordinator
            .heartbeat_attempt(
                &candidate.decision_identity,
                &attempt.attempt_identity,
                attempt.fence_token,
                heartbeat_at.clone(),
            )
            .expect("extend the real in-flight attempt with a fixed heartbeat sample"));
        let connection = Connection::open(&fixture.database_path)
            .expect("inspect bounded real heartbeat sample");
        let (logical_tail, chain_len) =
            real_audit_logical_tail(&connection, &candidate.decision_identity);
        let migration_sorted_tail: String = connection
            .query_row(
                "SELECT audit_identity
                 FROM immutable_audit_outbox
                 WHERE decision_identity=?1
                 ORDER BY
                   CASE WHEN predecessor_audit_identity IS NULL THEN 0 ELSE 1 END ASC,
                   predecessor_audit_identity ASC,
                   audit_identity ASC
                 LIMIT 1 OFFSET (
                   SELECT COUNT(*) - 1 FROM immutable_audit_outbox
                   WHERE decision_identity=?1
                 )",
                [candidate.decision_identity.as_str()],
                |row| row.get(0),
            )
            .expect("derive the historical migration's target physical tail");
        if migration_sorted_tail != logical_tail {
            ordering_witness = Some((
                heartbeat_at,
                offset_seconds,
                chain_len,
                logical_tail,
                migration_sorted_tail,
            ));
            break;
        }
    }
    let (
        last_heartbeat_at,
        heartbeat_sample_count,
        chain_len,
        pre_upgrade_logical_tail,
        migration_sorted_physical_tail,
    ) = ordering_witness.unwrap_or_else(|| {
        panic!(
            "the fixed {HEARTBEAT_SAMPLE_BOUND}-heartbeat real API sample must expose a v4 migration ordering witness"
        )
    });
    assert!(heartbeat_sample_count <= HEARTBEAT_SAMPLE_BOUND);
    assert!(
        chain_len > 2,
        "the real audit witness must be a longer chain"
    );
    assert_ne!(
        migration_sorted_physical_tail, pre_upgrade_logical_tail,
        "the bounded real API sample must witness that the legacy migration sort would select the wrong target tail"
    );

    let pre_upgrade_reconcile = fixture
        .coordinator
        .reconcile_foundation_decision(
            &candidate.decision_identity,
            candidate
                .foundation_binding()
                .expect("complete Foundation binding"),
            &append,
            last_heartbeat_at.clone(),
        )
        .expect("append the real heartbeat chain without expiring its lease");
    assert!(pre_upgrade_reconcile.progress_count > 0);
    assert_eq!(pre_upgrade_reconcile.provider_calls, 0);
    assert_eq!(pre_upgrade_reconcile.sink_calls, 0);
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM sink_results"), 0);

    let old_coordinator = fixture
        .coordinator
        .take()
        .expect("take every old coordinator Arc before downgrade");
    assert_eq!(
        Arc::strong_count(&old_coordinator),
        1,
        "the formal upgrade must not overlap an old coordinator Arc"
    );
    drop(old_coordinator);

    let mut historical = Connection::open(&fixture.database_path)
        .expect("open isolated database for historical v4 shaping");
    let (stable_logical_tail, stable_chain_len) =
        real_audit_logical_tail(&historical, &candidate.decision_identity);
    assert_eq!(stable_logical_tail, pre_upgrade_logical_tail);
    assert_eq!(stable_chain_len, chain_len);
    let target_before_downgrade =
        real_v4_upgrade_target_snapshot(&historical, &candidate.decision_identity);
    assert_eq!(
        historical
            .query_row(
                "SELECT COUNT(*) FROM immutable_audit_outbox
                 WHERE decision_identity=?1 AND append_state!='Appended'",
                [candidate.decision_identity.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .expect("verify the original real chain is durably appended before upgrade"),
        0
    );
    let replay_references_before: (i64, i64) = historical
        .query_row(
            "SELECT
               (SELECT COUNT(*) FROM review_terminal_replay_attempts
                 WHERE decision_identity=?1),
               (SELECT COUNT(*) FROM review_terminal_replay_completions
                 WHERE decision_identity=?1)",
            [candidate.decision_identity.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("prove target has no replay authority before historical shaping");
    assert_eq!(replay_references_before, (0, 0));
    let physical_tail_before_downgrade: String = historical
        .query_row(
            "SELECT audit_identity FROM immutable_audit_outbox
             WHERE decision_identity=?1 ORDER BY rowid DESC LIMIT 1",
            [candidate.decision_identity.as_str()],
            |row| row.get(0),
        )
        .expect("read target physical tail before historical shaping");
    assert_eq!(physical_tail_before_downgrade, pre_upgrade_logical_tail);

    downgrade_replay_schema_v4_for_test(&mut historical, false);
    assert_eq!(
        historical
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .expect("read exact historical schema version"),
        4
    );
    assert_eq!(
        historical
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type='table' AND name LIKE 'review_terminal_replay_%'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("prove replay tables are absent from the historical shape"),
        0
    );
    assert_eq!(
        historical
            .query_row(
                "SELECT COUNT(*) FROM pragma_foreign_key_list('immutable_audit_outbox')
                 WHERE \"table\"='immutable_audit_outbox'
                   AND \"from\"='predecessor_audit_identity'
                   AND \"to\"='audit_identity'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("verify the historical shape retains its exact final-name self-FK"),
        1
    );
    let historical_foreign_key_violations = foreign_key_violation_details(&historical);
    assert!(
        historical_foreign_key_violations.is_empty(),
        "historical v4 shape must preserve every original FK before formal upgrade: {historical_foreign_key_violations:#?}"
    );
    assert_eq!(
        real_v4_upgrade_target_snapshot(&historical, &candidate.decision_identity),
        target_before_downgrade,
        "historical table shaping may add its isolated Delivered sample but must preserve every target byte and reference"
    );
    let outbox_rowids_before_upgrade = audit_v4_upgrade_rowid_snapshot(&historical);
    assert_eq!(
        historical
            .query_row(
                "SELECT COUNT(*) FROM delivery_decisions
                 WHERE decision_identity='TEST_CODE_V4_DECISION' AND state='Delivered'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("identify the helper's isolated synthetic Delivered sample"),
        1
    );
    drop(historical);

    let schema_sql_reached = Arc::new(AtomicUsize::new(0));
    let schema_sql_reached_by_hook = schema_sql_reached.clone();
    let _schema_sql_hook = install_database_bootstrap_test_hook(
        DatabaseBootstrapTestPhase::AfterSchemaSqlBeforeCommitValidation,
        move || {
            schema_sql_reached_by_hook.fetch_add(1, Ordering::SeqCst);
            Ok(())
        },
    )
    .expect("install read-only formal-upgrade schema checkpoint");
    let formally_upgraded =
        DurableDeliveryCoordinator::open(coordinator_config).unwrap_or_else(|error| {
            panic!(
                "formal v4-to-current open failed; after_schema_sql_hook_reached={}: {error}",
                schema_sql_reached.load(Ordering::SeqCst)
            )
        });
    assert_eq!(
        schema_sql_reached.load(Ordering::SeqCst),
        1,
        "formal upgrade must pass all schema SQL before committing"
    );
    fixture.coordinator = FixtureCoordinator(Some(Arc::new(formally_upgraded)));
    let upgraded = Connection::open(&fixture.database_path)
        .expect("inspect the formally upgraded isolated database");
    upgraded
        .pragma_update(None, "foreign_keys", "ON")
        .expect("enable FK enforcement on the independent upgrade inspector");
    assert_eq!(
        upgraded
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .expect("read formally upgraded schema version"),
        super::schema::SCHEMA_VERSION
    );
    assert_eq!(
        upgraded
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("check formal upgrade foreign keys"),
        0
    );
    assert_eq!(
        upgraded
            .query_row(
                "SELECT COUNT(*) FROM pragma_foreign_key_list('immutable_audit_outbox')
                 WHERE \"table\"='immutable_audit_outbox'
                   AND \"from\"='predecessor_audit_identity'
                   AND \"to\"='audit_identity'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("check formal upgrade predecessor self-FK"),
        1
    );
    assert_eq!(
        real_v4_upgrade_target_snapshot(&upgraded, &candidate.decision_identity),
        target_before_downgrade,
        "formal v4-to-current upgrade must preserve exact target authority rows"
    );
    assert_eq!(
        audit_v4_upgrade_rowid_snapshot(&upgraded),
        outbox_rowids_before_upgrade,
        "formal v4-to-current upgrade must preserve every historical audit identity-to-rowid mapping"
    );
    let replay_references_after: (i64, i64) = upgraded
        .query_row(
            "SELECT
               (SELECT COUNT(*) FROM review_terminal_replay_attempts
                 WHERE decision_identity=?1),
               (SELECT COUNT(*) FROM review_terminal_replay_completions
                 WHERE decision_identity=?1)",
            [candidate.decision_identity.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("prove formal upgrade did not synthesize target replay authority");
    assert_eq!(replay_references_after, (0, 0));
    let actual_upgraded_physical_tail: String = upgraded
        .query_row(
            "SELECT audit_identity FROM immutable_audit_outbox
             WHERE decision_identity=?1 ORDER BY rowid DESC LIMIT 1",
            [candidate.decision_identity.as_str()],
            |row| row.get(0),
        )
        .expect("read actual target physical tail after formal v4 upgrade");
    assert_eq!(
        actual_upgraded_physical_tail, pre_upgrade_logical_tail,
        "the upgraded physical append tail must remain the unique pre-upgrade logical tail"
    );
    drop(upgraded);

    let recovered_at = last_heartbeat_at + chrono::Duration::seconds(121);
    let recovery = fixture
        .coordinator
        .reconcile_foundation_decision(
            &candidate.decision_identity,
            candidate
                .foundation_binding()
                .expect("exact recovery binding"),
            &append,
            recovered_at.clone(),
        )
        .expect("recover the single real expired Foundation attempt after formal upgrade");
    assert!(recovery.progress_count > 0);
    assert_eq!(recovery.provider_calls, 0);
    assert_eq!(recovery.sink_calls, 0);
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM sink_results"), 0);
    assert_eq!(
        fixture
            .coordinator
            .decision_state(&candidate.decision_identity)
            .expect("read upgraded recovery state"),
        DecisionState::UncertainManualReview
    );

    let terminal_after_recovery = fixture
        .coordinator
        .inspect_foundation_terminal(&candidate.decision_identity);
    let recovery_connection =
        Connection::open(&fixture.database_path).expect("inspect the recovered upgraded target");
    let fence_rows = recovery_connection
        .query_row(
            "SELECT COUNT(*),MIN(predecessor_audit_identity)
             FROM immutable_audit_outbox
             WHERE decision_identity=?1 AND attempt_identity=?2
               AND audit_kind='FenceRevoked'",
            params![
                candidate.decision_identity.as_str(),
                attempt.attempt_identity.as_str()
            ],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .expect("read the real recovered FenceRevoked audit predecessor");
    assert_eq!(
        fence_rows.0, 1,
        "single recovery must write one fence audit"
    );
    assert_eq!(
        fence_rows.1.as_deref(),
        Some(pre_upgrade_logical_tail.as_str()),
        "the real post-upgrade recovery must append to the preserved unique logical tail"
    );
    let terminal = match terminal_after_recovery
        .expect("real upgraded recovery must remain readable through the terminal inspector")
    {
        FoundationTerminalQuery::Terminal(record) => record,
        other => panic!("expected upgraded recovered terminal, got {other:?}"),
    };
    assert_eq!(
        terminal.disposition(),
        FoundationTerminalDisposition::Uncertain
    );
    assert_eq!(
        terminal.attempt_id(),
        Some(attempt.attempt_identity.as_str())
    );
    assert_eq!(
        terminal.durable_schema_version(),
        super::schema::SCHEMA_VERSION
    );
    assert_eq!(
        sha256_hex(terminal.evidence_bytes()),
        terminal.evidence_sha256()
    );
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM sink_results"), 0);
    assert_eq!(
        recovery_connection
            .query_row(
                "SELECT COUNT(*) FROM delivery_decisions
                 WHERE decision_identity='TEST_CODE_V4_DECISION' AND state='Delivered'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("verify scoped recovery isolates the helper's synthetic decision"),
        1
    );
    let target_audit_count_after_recovery = recovery_connection
        .query_row(
            "SELECT COUNT(*) FROM immutable_audit_outbox WHERE decision_identity=?1",
            [candidate.decision_identity.as_str()],
            |row| row.get::<_, i64>(0),
        )
        .expect("count target audit rows after first recovery");
    let recovery_facts = w16_recovery_facts(&fixture, Some(&candidate.decision_identity));
    let append_records = append
        .records
        .lock()
        .expect("read immutable append observations after first recovery")
        .clone();
    drop(recovery_connection);

    let duplicate_recovery = fixture
        .coordinator
        .reconcile_foundation_decision(
            &candidate.decision_identity,
            candidate
                .foundation_binding()
                .expect("duplicate recovery binding"),
            &append,
            recovered_at + chrono::Duration::seconds(1),
        )
        .expect("repeat completed upgraded recovery");
    assert_eq!(duplicate_recovery.progress_count, 0);
    assert_eq!(duplicate_recovery.provider_calls, 0);
    assert_eq!(duplicate_recovery.sink_calls, 0);
    assert_eq!(
        w16_recovery_facts(&fixture, Some(&candidate.decision_identity)),
        recovery_facts,
        "duplicate recovery must not alter target authority"
    );
    assert_eq!(
        *append
            .records
            .lock()
            .expect("read immutable append observations after duplicate recovery"),
        append_records,
        "duplicate recovery must not append a second audit"
    );
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM sink_results"), 0);

    let completed_coordinator = fixture
        .coordinator
        .take()
        .expect("take completed coordinator before restart");
    assert_eq!(
        Arc::strong_count(&completed_coordinator),
        1,
        "restart must release the only coordinator Arc"
    );
    drop(completed_coordinator);
    let restarted = fixture.second_coordinator("AUDIT_V4_UPGRADE_RESTART");
    let restarted_terminal = match restarted
        .inspect_foundation_terminal(&candidate.decision_identity)
        .expect("read upgraded terminal after coordinator restart")
    {
        FoundationTerminalQuery::Terminal(record) => record,
        other => panic!("expected restarted upgraded terminal, got {other:?}"),
    };
    assert_eq!(restarted_terminal, terminal);
    let restarted_connection = Connection::open(&fixture.database_path)
        .expect("inspect restarted upgraded target without coordinator mutation");
    assert_eq!(
        restarted_connection
            .query_row(
                "SELECT COUNT(*) FROM immutable_audit_outbox WHERE decision_identity=?1",
                [candidate.decision_identity.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .expect("count target audit rows after restart read"),
        target_audit_count_after_recovery
    );
    assert_eq!(
        w16_recovery_facts(&fixture, Some(&candidate.decision_identity)),
        recovery_facts,
        "restart terminal inspection must not alter target authority"
    );
    assert_eq!(
        *append
            .records
            .lock()
            .expect("read immutable append observations after restart"),
        append_records
    );
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM sink_results"), 0);
    drop(restarted_connection);
    fixture.coordinator = FixtureCoordinator(Some(restarted));
}

#[test]
fn br192_schema_v2_migration_rejects_historical_blank_manual_accepted_audit_refs() {
    for (label, whitespace) in [
        ("SPACE", " "),
        ("TAB", "\t"),
        ("LF", "\n"),
        ("CR", "\r"),
        ("MIXED", " \t\n\r "),
    ] {
        let mut connection =
            Connection::open_in_memory().expect("open historical blank-ref regression database");
        initialize_test_schema(&mut connection).expect("materialize complete reference schema");
        downgrade_manual_resolution_schema_for_test(&mut connection, 2);
        connection
            .pragma_update(None, "foreign_keys", "OFF")
            .expect("disable FK while seeding historical v2 blank ref regression");
        connection
            .execute(
                "INSERT INTO manual_resolutions(
                   resolution_identity,decision_identity,attempt_identity,disposition,
                   operator_identity,reason,evidence_canonical,evidence_sha256,
                   receipt_canonical,frozen_delivery_audit_canonical,
                   frozen_delivery_audit_sha256,immutable_audit_ref,
                   accepted_audit_identity,accepted_audit_append_state,
                   accepted_audit_ref,resolved_at
                 ) VALUES (
                   'TEST_CODE_RESOLUTION','TEST_CODE_DECISION','TEST_CODE_ATTEMPT','Accepted',
                   'TEST_CODE_OPERATOR','TEST_CODE_REASON',X'01','TEST_CODE_EVIDENCE_HASH',
                   NULL,X'02','TEST_CODE_AUDIT_HASH','TEST_CODE_AUTHORIZATION_REF',
                   'TEST_CODE_AUDIT_IDENTITY','Appended',?1,'2026-07-30T00:00:00.000Z'
                 )",
                [whitespace],
            )
            .unwrap_or_else(|error| panic!("v2 permits {label} historical blank ref: {error}"));
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .expect("restore FK before migration validation");

        assert!(matches!(
            initialize_test_schema(&mut connection),
            Err(DurableDeliveryError::InvalidConfiguration(reason))
                if reason.contains("blank manual accepted audit reference")
        ));
        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .expect("read rolled-back legacy schema version"),
            2,
            "failed {label} migration must leave the historical store at v2"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM manual_resolutions
                     WHERE accepted_audit_ref=?1",
                    [whitespace],
                    |row| row.get::<_, i64>(0),
                )
                .expect("historical blank row remains available for controlled recovery"),
            1
        );
    }
}

#[test]
fn br192_schema_v1_migration_rejects_historical_manual_acceptance() {
    let mut connection =
        Connection::open_in_memory().expect("open historical v1 acceptance regression database");
    initialize_test_schema(&mut connection).expect("materialize complete reference schema");
    downgrade_manual_resolution_schema_for_test(&mut connection, 1);
    connection
        .pragma_update(None, "foreign_keys", "OFF")
        .expect("disable FK while seeding historical v1 acceptance regression");
    connection
        .execute(
            "INSERT INTO manual_resolutions(
               resolution_identity,decision_identity,attempt_identity,disposition,
               operator_identity,reason,evidence_canonical,evidence_sha256,
               receipt_canonical,frozen_delivery_audit_canonical,
               frozen_delivery_audit_sha256,immutable_audit_ref,resolved_at
             ) VALUES (
               'TEST_CODE_V1_RESOLUTION','TEST_CODE_V1_DECISION','TEST_CODE_V1_ATTEMPT',
               'Accepted','TEST_CODE_V1_OPERATOR','TEST_CODE_V1_REASON',
               X'01','TEST_CODE_V1_EVIDENCE_HASH',NULL,X'02',
               'TEST_CODE_V1_AUDIT_HASH','TEST_CODE_V1_AUTHORIZATION_REF',
               '2026-07-30T00:00:00.000Z'
             )",
            [],
        )
        .expect("v1 stores acceptance without an append acknowledgement");
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .expect("restore FK before migration validation");

    assert!(matches!(
        initialize_test_schema(&mut connection),
        Err(DurableDeliveryError::InvalidConfiguration(reason))
            if reason.contains("schema-v1 contains 1 manual accepted resolution")
    ));
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .expect("read rolled-back v1 schema version"),
        1
    );
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br192_final_bootstrap_validation_rejects_ancestor_swap_after_parent_sync() {
    let test_code = format!(
        "TEST_CODE_BR192_FINAL_BOOTSTRAP_REBIND_{}_{}",
        std::process::id(),
        NEXT_TEST_ID.fetch_add(1, Ordering::SeqCst)
    );
    let fixture = PhysicalAliasFixture::new(&test_code);
    fixture.ensure_test_root();
    let test_root = fixture.test_root.clone();
    let retained_root = test_root.with_file_name(format!("{test_code}_RETAINED"));
    let callback_test_root = test_root.clone();
    let callback_retained_root = retained_root.clone();
    let _hook = install_database_bootstrap_test_hook(
        DatabaseBootstrapTestPhase::AfterFinalParentSyncBeforeSuccessValidation,
        move || {
            std::fs::rename(&callback_test_root, &callback_retained_root)?;
            std::fs::create_dir(&callback_test_root)?;
            Ok(())
        },
    )
    .expect("install final bootstrap ancestor swap");

    assert!(matches!(
        fixture.open_test(),
        Err(DurableDeliveryError::IsolationViolation(_))
    ));
    fixture
        .cleanup
        .record(&retained_root, OwnedPathKind::Directory);
    fixture.cleanup.record(&test_root, OwnedPathKind::Directory);
    fixture.capture_sqlite_objects_beneath(&retained_root);
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br192_stable_ancestor_nlink_refresh_allows_legitimate_test_child_directory() {
    let fixture = Fixture::new("STABLE_ANCESTOR_NLINK_REFRESH");
    let child = fixture
        .database_path
        .parent()
        .expect("TEST_CODE database parent")
        .join("legitimate-child");
    std::fs::create_dir(&child).expect("create legitimate TEST_CODE child directory");
    fixture.cleanup.record(&child, OwnedPathKind::Directory);

    assert!(fixture
        .coordinator
        .inspect_pending_for_date("2026-07-30")
        .expect("stable mkdir nlink drift is refreshed after full chain rebind")
        .is_empty());
    std::fs::remove_dir(&child).expect("remove legitimate TEST_CODE child directory");
    assert!(fixture
        .coordinator
        .inspect_pending_for_date("2026-07-30")
        .expect("stable rmdir nlink drift is refreshed after full chain rebind")
        .is_empty());
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br192_physical_store_does_not_create_a_missing_test_namespace_parent() {
    let test_code = format!(
        "TEST_CODE_BR192_MISSING_PARENT_{}_{}",
        std::process::id(),
        NEXT_TEST_ID.fetch_add(1, Ordering::SeqCst)
    );
    let fixture = PhysicalAliasFixture::new(&test_code);
    assert!(
        !fixture.test_root.exists(),
        "regression fixture requires an absent exact test namespace"
    );

    assert!(matches!(
        fixture.open_test(),
        Err(DurableDeliveryError::IsolationViolation(_))
    ));
    assert!(
        !fixture.test_root.exists(),
        "coordinator must not create path components before no-follow validation"
    );
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br192_physical_store_rejects_world_writable_test_namespace_parent() {
    use std::os::unix::fs::PermissionsExt;

    let test_code = format!(
        "TEST_CODE_BR192_WORLD_WRITABLE_PARENT_{}_{}",
        std::process::id(),
        NEXT_TEST_ID.fetch_add(1, Ordering::SeqCst)
    );
    let fixture = PhysicalAliasFixture::new(&test_code);
    fixture.ensure_test_root();
    std::fs::set_permissions(&fixture.test_root, std::fs::Permissions::from_mode(0o777))
        .expect("make regression parent world writable");

    assert!(matches!(
        fixture.open_test(),
        Err(DurableDeliveryError::IsolationViolation(_))
    ));
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br192_physical_store_rejects_world_writable_main_database() {
    use std::os::unix::fs::PermissionsExt;

    let test_code = format!(
        "TEST_CODE_BR192_WORLD_WRITABLE_MAIN_{}_{}",
        std::process::id(),
        NEXT_TEST_ID.fetch_add(1, Ordering::SeqCst)
    );
    let fixture = PhysicalAliasFixture::new(&test_code);
    fixture.create_test_sentinel();
    std::fs::set_permissions(
        fixture.test_database_path(),
        std::fs::Permissions::from_mode(0o666),
    )
    .expect("make regression main database world writable");

    assert!(matches!(
        fixture.open_test(),
        Err(DurableDeliveryError::IsolationViolation(_))
    ));
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br192_physical_store_rejects_test_parent_symlinked_to_foreign_test_namespace() {
    let test_code = format!(
        "TEST_CODE_BR192_SYMLINK_PARENT_{}_{}",
        std::process::id(),
        NEXT_TEST_ID.fetch_add(1, Ordering::SeqCst)
    );
    let fixture = PhysicalAliasFixture::new(&test_code);
    fixture.ensure_foreign_root();
    fixture.create_symlink(&fixture.foreign_test_code, &fixture.test_root);

    assert!(matches!(
        fixture.open_test(),
        Err(DurableDeliveryError::IsolationViolation(_))
    ));
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br192_physical_store_rejects_test_database_hardlinked_to_foreign_test_database() {
    use std::os::unix::fs::MetadataExt;

    let test_code = format!(
        "TEST_CODE_BR192_HARDLINK_{}_{}",
        std::process::id(),
        NEXT_TEST_ID.fetch_add(1, Ordering::SeqCst)
    );
    let fixture = PhysicalAliasFixture::new(&test_code);
    fixture.ensure_test_root();
    fixture.create_foreign_sentinel();
    let sentinel = b"TEST_CODE_BR192_MAIN_ATTESTATION_SENTINEL";
    std::fs::write(fixture.foreign_database_path(), sentinel).expect("write foreign test sentinel");
    let test_database_path = fixture.test_database_path();
    fixture.create_hard_link(&fixture.foreign_database_path(), &test_database_path);
    let foreign_metadata =
        std::fs::metadata(fixture.foreign_database_path()).expect("foreign test inode metadata");
    let test_metadata = std::fs::metadata(&test_database_path).expect("test inode metadata");
    assert_eq!(
        (foreign_metadata.dev(), foreign_metadata.ino()),
        (test_metadata.dev(), test_metadata.ino()),
        "regression fixture must address the same physical inode"
    );

    assert!(matches!(
        fixture.open_test(),
        Err(DurableDeliveryError::IsolationViolation(_))
    ));
    assert_eq!(
        std::fs::read(fixture.foreign_database_path()).expect("read foreign test sentinel"),
        sentinel,
        "main attestation rejection must happen before schema mutation"
    );
    for suffix in ["-journal", "-shm", "-wal"] {
        assert!(
            !fixture.test_sidecar_path(suffix).exists(),
            "main attestation rejection must happen before SQLite creates {suffix}"
        );
    }
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br192_physical_store_rejects_test_database_symlinked_to_foreign_test_database() {
    let test_code = format!(
        "TEST_CODE_BR192_FOREIGN_SYMLINK_{}_{}",
        std::process::id(),
        NEXT_TEST_ID.fetch_add(1, Ordering::SeqCst)
    );
    let fixture = PhysicalAliasFixture::new(&test_code);
    fixture.ensure_test_root();
    fixture.create_foreign_sentinel();
    let relative_foreign_target = PathBuf::from("..")
        .join(&fixture.foreign_test_code)
        .join("durable_delivery.sqlite3");
    fixture.create_symlink(relative_foreign_target, &fixture.test_database_path());

    assert!(matches!(
        fixture.open_test(),
        Err(DurableDeliveryError::IsolationViolation(_))
    ));
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br192_physical_store_rejects_foreign_test_database_hardlinked_to_test_database() {
    use std::os::unix::fs::MetadataExt;

    let test_code = format!(
        "TEST_CODE_BR192_FOREIGN_HARDLINK_{}_{}",
        std::process::id(),
        NEXT_TEST_ID.fetch_add(1, Ordering::SeqCst)
    );
    let fixture = PhysicalAliasFixture::new(&test_code);
    fixture.ensure_test_root();
    fixture.create_foreign_sentinel();
    fixture.create_hard_link(
        &fixture.foreign_database_path(),
        &fixture.test_database_path(),
    );
    let foreign_metadata =
        std::fs::metadata(fixture.foreign_database_path()).expect("foreign test inode metadata");
    let test_metadata =
        std::fs::metadata(fixture.test_database_path()).expect("test inode metadata");
    assert_eq!(
        (foreign_metadata.dev(), foreign_metadata.ino()),
        (test_metadata.dev(), test_metadata.ino()),
        "cross-test alias fixture must address the same physical inode"
    );

    assert!(matches!(
        fixture.open_test(),
        Err(DurableDeliveryError::IsolationViolation(_))
    ));
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br192_physical_store_rejects_test_wal_symlinked_to_foreign_test_wal() {
    let test_code = format!(
        "TEST_CODE_BR192_FOREIGN_WAL_SYMLINK_{}_{}",
        std::process::id(),
        NEXT_TEST_ID.fetch_add(1, Ordering::SeqCst)
    );
    let fixture = PhysicalAliasFixture::new(&test_code);
    fixture.create_test_sentinel();
    fixture.create_foreign_sidecar_sentinel("-wal");
    let relative_foreign_target = PathBuf::from("..")
        .join(&fixture.foreign_test_code)
        .join("durable_delivery.sqlite3-wal");
    fixture.create_symlink(relative_foreign_target, &fixture.test_sidecar_path("-wal"));

    assert!(matches!(
        fixture.open_test(),
        Err(DurableDeliveryError::IsolationViolation(_))
    ));
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br192_physical_store_rejects_test_shm_hardlinked_to_foreign_test_shm() {
    use std::os::unix::fs::MetadataExt;

    let test_code = format!(
        "TEST_CODE_BR192_FOREIGN_SHM_HARDLINK_{}_{}",
        std::process::id(),
        NEXT_TEST_ID.fetch_add(1, Ordering::SeqCst)
    );
    let fixture = PhysicalAliasFixture::new(&test_code);
    fixture.create_test_sentinel();
    fixture.create_foreign_sidecar_sentinel("-shm");
    let foreign_shm = fixture.foreign_sidecar_path("-shm");
    let test_shm = fixture.test_sidecar_path("-shm");
    fixture.create_hard_link(&foreign_shm, &test_shm);
    let foreign_metadata = std::fs::metadata(&foreign_shm).expect("foreign test SHM metadata");
    let test_metadata = std::fs::metadata(&test_shm).expect("test SHM metadata");
    assert_eq!(
        (foreign_metadata.dev(), foreign_metadata.ino()),
        (test_metadata.dev(), test_metadata.ino()),
        "cross-test sidecar alias fixture must address the same physical inode"
    );

    assert!(matches!(
        fixture.open_test(),
        Err(DurableDeliveryError::IsolationViolation(_))
    ));
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br192_physical_store_rejects_test_wal_hardlinked_to_foreign_test_wal() {
    let test_code = format!(
        "TEST_CODE_BR192_FOREIGN_WAL_HARDLINK_{}_{}",
        std::process::id(),
        NEXT_TEST_ID.fetch_add(1, Ordering::SeqCst)
    );
    let fixture = PhysicalAliasFixture::new(&test_code);
    fixture.create_test_sentinel();
    fixture.create_foreign_sidecar_sentinel("-wal");
    fixture.create_hard_link(
        &fixture.foreign_sidecar_path("-wal"),
        &fixture.test_sidecar_path("-wal"),
    );

    assert!(matches!(
        fixture.open_test(),
        Err(DurableDeliveryError::IsolationViolation(_))
    ));
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br192_physical_store_rejects_test_shm_symlinked_to_foreign_test_shm() {
    let test_code = format!(
        "TEST_CODE_BR192_FOREIGN_SHM_SYMLINK_{}_{}",
        std::process::id(),
        NEXT_TEST_ID.fetch_add(1, Ordering::SeqCst)
    );
    let fixture = PhysicalAliasFixture::new(&test_code);
    fixture.create_test_sentinel();
    fixture.create_foreign_sidecar_sentinel("-shm");
    let relative_foreign_target = PathBuf::from("..")
        .join(&fixture.foreign_test_code)
        .join("durable_delivery.sqlite3-shm");
    fixture.create_symlink(relative_foreign_target, &fixture.test_sidecar_path("-shm"));

    assert!(matches!(
        fixture.open_test(),
        Err(DurableDeliveryError::IsolationViolation(_))
    ));
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br192_descriptor_binding_rejects_leaf_swap_before_the_next_operation() {
    let test_code = format!(
        "TEST_CODE_BR192_POST_OPEN_SWAP_{}_{}",
        std::process::id(),
        NEXT_TEST_ID.fetch_add(1, Ordering::SeqCst)
    );
    let fixture = PhysicalAliasFixture::new(&test_code);
    fixture.ensure_test_root();
    let database_path = fixture.test_database_path();
    let coordinator = fixture
        .open_test()
        .expect("open exact test database before adversarial leaf replacement");
    let retained_database_path = fixture.test_root.join("retained-original.sqlite3");
    fixture.rename_owned(&database_path, &retained_database_path);
    fixture.create_file(&database_path);

    assert!(matches!(
        coordinator.inspect_pending_for_date("2026-07-30"),
        Err(DurableDeliveryError::IsolationViolation(_))
    ));
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br192_descriptor_binding_rejects_wal_leaf_swap_before_the_next_operation() {
    let test_code = format!(
        "TEST_CODE_BR192_POST_OPEN_WAL_SWAP_{}_{}",
        std::process::id(),
        NEXT_TEST_ID.fetch_add(1, Ordering::SeqCst)
    );
    let fixture = PhysicalAliasFixture::new(&test_code);
    fixture.ensure_test_root();
    let coordinator = fixture
        .open_test()
        .expect("open exact test database before adversarial WAL replacement");
    let wal_path = fixture.test_sidecar_path("-wal");
    let retained_wal_path = fixture.test_root.join("retained-original.sqlite3-wal");
    fixture.rename_owned(&wal_path, &retained_wal_path);
    fixture.create_file(&wal_path);

    assert!(matches!(
        coordinator.inspect_pending_for_date("2026-07-30"),
        Err(DurableDeliveryError::IsolationViolation(_))
    ));
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br192_descriptor_binding_rejects_shm_leaf_swap_before_the_next_operation() {
    let test_code = format!(
        "TEST_CODE_BR192_POST_OPEN_SHM_SWAP_{}_{}",
        std::process::id(),
        NEXT_TEST_ID.fetch_add(1, Ordering::SeqCst)
    );
    let fixture = PhysicalAliasFixture::new(&test_code);
    fixture.ensure_test_root();
    let coordinator = fixture
        .open_test()
        .expect("open exact test database before adversarial SHM replacement");
    let shm_path = fixture.test_sidecar_path("-shm");
    let retained_shm_path = fixture.test_root.join("retained-original.sqlite3-shm");
    fixture.rename_owned(&shm_path, &retained_shm_path);
    fixture.create_file(&shm_path);

    assert!(matches!(
        coordinator.inspect_pending_for_date("2026-07-30"),
        Err(DurableDeliveryError::IsolationViolation(_))
    ));
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br192_descriptor_binding_rejects_test_namespace_ancestor_rename_and_replacement() {
    let test_code = format!(
        "TEST_CODE_BR192_ANCESTOR_SWAP_{}_{}",
        std::process::id(),
        NEXT_TEST_ID.fetch_add(1, Ordering::SeqCst)
    );
    let fixture = PhysicalAliasFixture::new(&test_code);
    fixture.ensure_test_root();
    let coordinator = fixture
        .open_test()
        .expect("open exact test database before adversarial ancestor replacement");
    let retained_root = PathBuf::from("data/test").join(format!("{test_code}_RETAINED"));
    fixture.rename_owned_directory(&fixture.test_root, &retained_root);
    fixture.capture_sqlite_objects_beneath(&retained_root);
    fixture.ensure_test_root();

    assert!(matches!(
        coordinator.inspect_pending_for_date("2026-07-30"),
        Err(DurableDeliveryError::IsolationViolation(_))
    ));
}

#[cfg(unix)]
fn install_during_operation_leaf_swap(
    coordinator: &DurableDeliveryCoordinator,
    phase: DatabaseOperationTestPhase,
    source: PathBuf,
    retained: PathBuf,
) {
    coordinator
        .install_database_operation_test_hook(phase, move || {
            std::fs::rename(&source, &retained)?;
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&source)?;
            Ok(())
        })
        .expect("install one-shot TEST_CODE leaf swap");
}

#[cfg(unix)]
fn restore_during_operation_leaf_swap(source: &Path, retained: &Path) {
    std::fs::remove_file(source).expect("remove exact TEST_CODE replacement leaf");
    std::fs::rename(retained, source).expect("restore exact TEST_CODE retained SQLite leaf");
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br192_read_rejects_main_swap_after_prevalidation_before_sql() {
    let fixture = Fixture::new("DURING_OPERATION_PRE_SQL_MAIN_SWAP");
    let database_path = fixture.database_path.clone();
    let retained_database_path = database_path
        .parent()
        .expect("isolated database parent")
        .join("retained-before-sql.sqlite3");
    install_during_operation_leaf_swap(
        &fixture.coordinator,
        DatabaseOperationTestPhase::AfterPreValidationBeforeSql,
        database_path.clone(),
        retained_database_path.clone(),
    );

    let result = fixture.coordinator.inspect_pending_for_date("2026-07-30");
    fixture
        .cleanup
        .record_if_present(&retained_database_path, OwnedPathKind::FileOrSymlink);
    fixture
        .cleanup
        .record_if_present(&database_path, OwnedPathKind::FileOrSymlink);
    assert!(matches!(
        result,
        Err(DurableDeliveryError::IsolationViolation(_))
    ));
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br192_transaction_rejects_main_leaf_swap_after_sql_before_commit() {
    let mut fixture = Fixture::new("DURING_OPERATION_MAIN_SWAP");
    let database_path = fixture.database_path.clone();
    let retained_database_path = database_path
        .parent()
        .expect("isolated database parent")
        .join("retained-during-operation.sqlite3");
    install_during_operation_leaf_swap(
        &fixture.coordinator,
        DatabaseOperationTestPhase::AfterSqlBeforePreCommitValidation,
        database_path.clone(),
        retained_database_path.clone(),
    );

    let candidate = envelope(
        "DURING_OPERATION_MAIN_SWAP",
        PushKind::ReviewProviderTopN,
        DeliverySubKind::None,
        "2026-07-30",
        true,
    );
    let result = fixture.coordinator.prepare(&candidate, 1, now());

    fixture
        .cleanup
        .record_if_present(&retained_database_path, OwnedPathKind::FileOrSymlink);
    fixture
        .cleanup
        .record_if_present(&database_path, OwnedPathKind::FileOrSymlink);
    assert!(matches!(
        result,
        Err(DurableDeliveryError::IsolationViolation(_))
    ));
    restore_during_operation_leaf_swap(&database_path, &retained_database_path);
    drop(fixture.coordinator.take());
    let reopened = fixture.second_coordinator("DURING_OPERATION_MAIN_SWAP_REOPEN");
    let committed = reopened
        .transaction_row_count_after_fault_for_test(&candidate.decision_identity)
        .expect("query restored database after explicit rollback");
    assert_eq!(committed, 0, "isolation failure must not commit SQL");
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br192_transaction_rejects_wal_swap_after_sql_before_commit() {
    let mut fixture = Fixture::new("DURING_OPERATION_WAL_SWAP");
    let wal_path = PathBuf::from(format!("{}-wal", fixture.database_path.display()));
    let retained_wal_path = fixture
        .database_path
        .parent()
        .expect("isolated database parent")
        .join("retained-during-operation.sqlite3-wal");
    install_during_operation_leaf_swap(
        &fixture.coordinator,
        DatabaseOperationTestPhase::AfterSqlBeforePreCommitValidation,
        wal_path.clone(),
        retained_wal_path.clone(),
    );
    let candidate = envelope(
        "DURING_OPERATION_WAL_SWAP",
        PushKind::ReviewProviderTopN,
        DeliverySubKind::None,
        "2026-07-30",
        true,
    );
    let before = fixture
        .coordinator
        .transaction_persistence_snapshot_after_fault_for_test(&candidate.decision_identity)
        .expect("capture WAL-swap transaction baseline");
    let result = fixture.coordinator.prepare(&candidate, 1, now());
    fixture
        .cleanup
        .record_if_present(&retained_wal_path, OwnedPathKind::FileOrSymlink);
    fixture
        .cleanup
        .record_if_present(&wal_path, OwnedPathKind::FileOrSymlink);
    assert!(matches!(
        result,
        Err(DurableDeliveryError::IsolationViolation(_))
    ));
    restore_during_operation_leaf_swap(&wal_path, &retained_wal_path);
    drop(fixture.coordinator.take());
    let reopened = fixture.second_coordinator("DURING_OPERATION_WAL_SWAP_REOPEN");
    assert_eq!(
        reopened
            .transaction_persistence_snapshot_after_fault_for_test(&candidate.decision_identity)
            .expect("inspect WAL-swap rollback evidence"),
        before,
        "WAL swap must commit no decision SQL, policy mutation, or user_version change"
    );
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br192_transaction_rejects_shm_swap_after_sql_before_commit() {
    let mut fixture = Fixture::new("DURING_OPERATION_SHM_SWAP");
    let shm_path = PathBuf::from(format!("{}-shm", fixture.database_path.display()));
    let retained_shm_path = fixture
        .database_path
        .parent()
        .expect("isolated database parent")
        .join("retained-during-operation.sqlite3-shm");
    install_during_operation_leaf_swap(
        &fixture.coordinator,
        DatabaseOperationTestPhase::AfterSqlBeforePreCommitValidation,
        shm_path.clone(),
        retained_shm_path.clone(),
    );
    let candidate = envelope(
        "DURING_OPERATION_SHM_SWAP",
        PushKind::ReviewProviderTopN,
        DeliverySubKind::None,
        "2026-07-30",
        true,
    );
    let before = fixture
        .coordinator
        .transaction_persistence_snapshot_after_fault_for_test(&candidate.decision_identity)
        .expect("capture SHM-swap transaction baseline");
    let result = fixture.coordinator.prepare(&candidate, 1, now());
    fixture
        .cleanup
        .record_if_present(&retained_shm_path, OwnedPathKind::FileOrSymlink);
    fixture
        .cleanup
        .record_if_present(&shm_path, OwnedPathKind::FileOrSymlink);
    assert!(matches!(
        result,
        Err(DurableDeliveryError::IsolationViolation(_))
    ));
    restore_during_operation_leaf_swap(&shm_path, &retained_shm_path);
    drop(fixture.coordinator.take());
    let reopened = fixture.second_coordinator("DURING_OPERATION_SHM_SWAP_REOPEN");
    assert_eq!(
        reopened
            .transaction_persistence_snapshot_after_fault_for_test(&candidate.decision_identity)
            .expect("inspect SHM-swap rollback evidence"),
        before,
        "SHM swap must commit no decision SQL, policy mutation, or user_version change"
    );
}

#[cfg(unix)]
#[serial_test::serial(durable_physical_isolation)]
#[test]
fn br192_transaction_rejects_ancestor_swap_after_sql_before_commit() {
    let fixture = Fixture::new("DURING_OPERATION_ANCESTOR_SWAP");
    let test_root = fixture
        .database_path
        .parent()
        .expect("isolated database parent")
        .to_path_buf();
    let retained_root = test_root.with_file_name(format!(
        "{}_RETAINED",
        test_root
            .file_name()
            .expect("TEST_CODE root")
            .to_string_lossy()
    ));
    let hook_test_root = test_root.clone();
    let hook_retained_root = retained_root.clone();
    fixture
        .coordinator
        .install_database_operation_test_hook(
            DatabaseOperationTestPhase::AfterSqlBeforePreCommitValidation,
            move || {
                std::fs::rename(&hook_test_root, &hook_retained_root)?;
                std::fs::create_dir(&hook_test_root)?;
                Ok(())
            },
        )
        .expect("install one-shot TEST_CODE ancestor swap");
    let candidate = envelope(
        "DURING_OPERATION_ANCESTOR_SWAP",
        PushKind::ReviewProviderTopN,
        DeliverySubKind::None,
        "2026-07-30",
        true,
    );
    let before = fixture
        .coordinator
        .transaction_persistence_snapshot_after_fault_for_test(&candidate.decision_identity)
        .expect("capture ancestor-swap transaction baseline");
    let result = fixture.coordinator.prepare(&candidate, 1, now());
    fixture
        .cleanup
        .record(&retained_root, OwnedPathKind::Directory);
    fixture.cleanup.record(&test_root, OwnedPathKind::Directory);
    for suffix in ["", "-journal", "-shm", "-wal"] {
        fixture.cleanup.record_if_present(
            PathBuf::from(format!(
                "{}{suffix}",
                retained_root.join("durable_delivery.sqlite3").display()
            )),
            OwnedPathKind::FileOrSymlink,
        );
    }
    assert!(matches!(
        result,
        Err(DurableDeliveryError::IsolationViolation(_))
    ));
    assert_eq!(
        fixture
            .coordinator
            .transaction_persistence_snapshot_after_fault_for_test(&candidate.decision_identity)
            .expect("inspect ancestor-swap rollback evidence"),
        before,
        "ancestor swap must commit no decision SQL, policy mutation, or user_version change"
    );
}

#[test]
fn br192_after_sql_hook_error_explicitly_rolls_back_without_commit() {
    let fixture = Fixture::new("AFTER_SQL_HOOK_ROLLBACK");
    fixture
        .coordinator
        .install_database_operation_test_hook(
            DatabaseOperationTestPhase::AfterSqlBeforePreCommitValidation,
            || {
                Err(DurableDeliveryError::InvalidConfiguration(
                    "TEST_CODE_AFTER_SQL_HOOK_FAILURE".to_owned(),
                ))
            },
        )
        .expect("install one-shot rollback hook");
    let candidate = envelope(
        "AFTER_SQL_HOOK_ROLLBACK",
        PushKind::ReviewProviderTopN,
        DeliverySubKind::None,
        "2026-07-30",
        true,
    );

    assert!(matches!(
        fixture.coordinator.prepare(&candidate, 1, now()),
        Err(DurableDeliveryError::InvalidConfiguration(reason))
            if reason == "TEST_CODE_AFTER_SQL_HOOK_FAILURE"
    ));
    assert_eq!(
        fixture.query_i64(&format!(
            "SELECT COUNT(*) FROM delivery_decisions WHERE decision_identity='{}'",
            candidate.decision_identity
        )),
        0,
        "hook failure after SQL must explicitly roll back"
    );
}

#[test]
fn br192_compound_commit_and_rollback_failure_preserves_both_evidence() {
    let fixture = Fixture::new("COMPOUND_COMMIT_ROLLBACK_FAILURE");
    let _fault =
        install_compound_commit_rollback_test_fault().expect("install transaction-control fault");
    let candidate = envelope(
        "COMPOUND_COMMIT_ROLLBACK_FAILURE",
        PushKind::ReviewProviderTopN,
        DeliverySubKind::None,
        "2026-07-30",
        true,
    );

    let error = fixture
        .coordinator
        .prepare(&candidate, 1, now())
        .expect_err("injected COMMIT+ROLLBACK failure must be explicit");
    let rendered = error.to_string();
    assert!(rendered.contains("transaction commit failed"));
    assert!(rendered.contains("primary=sqlite durable-delivery failure"));
    assert!(rendered.contains("explicit_rollback=sqlite durable-delivery failure"));
    assert!(rendered.contains("post_rollback_validation=ok"));
    assert_eq!(
        fixture.query_i64(&format!(
            "SELECT COUNT(*) FROM delivery_decisions WHERE decision_identity='{}'",
            candidate.decision_identity
        )),
        0,
        "the real rollback still protects data while compound failure evidence is exercised"
    );
}

#[derive(Default)]
struct MemoryAppendPort {
    records: Mutex<BTreeMap<String, MemoryAppendRecord>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct MemoryAppendRecord {
    record_kind: String,
    canonical_bytes: Vec<u8>,
    sha256: String,
    immutable_ref: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct PersistentTestAppendRecord {
    record_kind: String,
    identity: String,
    canonical: Vec<u8>,
    sha256: String,
    immutable_ref: String,
}

struct PersistentTestAppendPort {
    path: PathBuf,
}

impl PersistentTestAppendPort {
    fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let relative = path.strip_prefix(manifest).unwrap_or(&path);
        let components = relative.components().collect::<Vec<_>>();
        assert!(
            matches!(
                components.as_slice(),
                [
                    std::path::Component::Normal(data),
                    std::path::Component::Normal(test),
                    std::path::Component::Normal(test_code),
                    std::path::Component::Normal(file),
                ] if *data == "data"
                    && *test == "test"
                    && test_code.to_string_lossy().starts_with("TEST_CODE")
                    && *file == "immutable_append.jsonl"
            ),
            "persistent append evidence must use one exact TEST_CODE file: {}",
            path.display()
        );
        Self { path }
    }

    fn records(&self) -> Result<Vec<PersistentTestAppendRecord>> {
        let file = match std::fs::File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        BufReader::new(file)
            .lines()
            .map(|line| {
                let line = line?;
                Ok(serde_json::from_str::<PersistentTestAppendRecord>(&line)?)
            })
            .collect()
    }
}

impl ImmutableAppendPort for PersistentTestAppendPort {
    fn append_exact(
        &self,
        record_kind: &str,
        identity: &str,
        canonical_bytes: &[u8],
        sha256: &str,
    ) -> Result<String> {
        if sha256_hex(canonical_bytes) != sha256 {
            return Err(DurableDeliveryError::ImmutableAppendConflict(
                identity.to_owned(),
            ));
        }
        if let Some(existing) = self
            .records()?
            .into_iter()
            .find(|record| record.identity == identity)
        {
            if existing.record_kind == record_kind
                && existing.canonical == canonical_bytes
                && existing.sha256 == sha256
            {
                return Ok(existing.immutable_ref);
            }
            return Err(DurableDeliveryError::ImmutableAppendConflict(
                identity.to_owned(),
            ));
        }
        let immutable_ref = format!("TEST_CODE_FILE_APPEND:{identity}:{sha256}");
        let record = PersistentTestAppendRecord {
            record_kind: record_kind.to_owned(),
            identity: identity.to_owned(),
            canonical: canonical_bytes.to_vec(),
            sha256: sha256.to_owned(),
            immutable_ref: immutable_ref.clone(),
        };
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        serde_json::to_writer(&mut file, &record)?;
        file.write_all(b"\n")?;
        file.flush()?;
        file.sync_all()?;
        Ok(immutable_ref)
    }
}

struct FailScheduleHydrationAppliedOnce {
    inner: MemoryAppendPort,
    fail_next_hydration_ack: std::sync::atomic::AtomicBool,
}

struct RollbackAcknowledgementAfterAppend {
    inner: MemoryAppendPort,
    coordinator: Arc<DurableDeliveryCoordinator>,
    target_kind: &'static str,
    armed: std::sync::atomic::AtomicBool,
}

struct SchemaDriftAfterAppend {
    inner: MemoryAppendPort,
    coordinator: Arc<DurableDeliveryCoordinator>,
    target_kind: &'static str,
    drifted_version: i64,
    armed: std::sync::atomic::AtomicBool,
}

struct EmptyAppendPort {
    inner: MemoryAppendPort,
    target_kind: &'static str,
}

struct MismatchedAuthorizationRefOnce<'a> {
    inner: &'a MemoryAppendPort,
    armed: std::sync::atomic::AtomicBool,
}

impl<'a> MismatchedAuthorizationRefOnce<'a> {
    fn new(inner: &'a MemoryAppendPort) -> Self {
        Self {
            inner,
            armed: std::sync::atomic::AtomicBool::new(true),
        }
    }
}

impl ImmutableAppendPort for MismatchedAuthorizationRefOnce<'_> {
    fn append_exact(
        &self,
        record_kind: &str,
        identity: &str,
        canonical_bytes: &[u8],
        sha256: &str,
    ) -> Result<String> {
        let immutable_ref =
            self.inner
                .append_exact(record_kind, identity, canonical_bytes, sha256)?;
        if record_kind == "ManualResolutionAuthorization"
            && self.armed.swap(false, Ordering::SeqCst)
        {
            Ok(format!("{immutable_ref}:TEST_CODE_MISMATCH"))
        } else {
            Ok(immutable_ref)
        }
    }
}

impl EmptyAppendPort {
    fn new(target_kind: &'static str) -> Self {
        Self {
            inner: MemoryAppendPort::default(),
            target_kind,
        }
    }
}

impl ImmutableAppendPort for EmptyAppendPort {
    fn append_exact(
        &self,
        record_kind: &str,
        identity: &str,
        canonical_bytes: &[u8],
        sha256: &str,
    ) -> Result<String> {
        let immutable_ref =
            self.inner
                .append_exact(record_kind, identity, canonical_bytes, sha256)?;
        if record_kind == self.target_kind {
            Ok(" \t\n\r ".to_owned())
        } else {
            Ok(immutable_ref)
        }
    }
}

impl RollbackAcknowledgementAfterAppend {
    fn new(coordinator: Arc<DurableDeliveryCoordinator>, target_kind: &'static str) -> Self {
        Self {
            inner: MemoryAppendPort::default(),
            coordinator,
            target_kind,
            armed: std::sync::atomic::AtomicBool::new(true),
        }
    }
}

impl ImmutableAppendPort for RollbackAcknowledgementAfterAppend {
    fn append_exact(
        &self,
        record_kind: &str,
        identity: &str,
        canonical_bytes: &[u8],
        sha256: &str,
    ) -> Result<String> {
        let immutable_ref =
            self.inner
                .append_exact(record_kind, identity, canonical_bytes, sha256)?;
        if record_kind == self.target_kind && self.armed.swap(false, Ordering::SeqCst) {
            self.coordinator.install_database_operation_test_hook(
                DatabaseOperationTestPhase::AfterSqlBeforePreCommitValidation,
                || {
                    Err(DurableDeliveryError::InvalidConfiguration(
                        "TEST_CODE_ACK_AFTER_UPDATE_BEFORE_COMMIT".to_owned(),
                    ))
                },
            )?;
        }
        Ok(immutable_ref)
    }
}

impl SchemaDriftAfterAppend {
    fn new(
        coordinator: Arc<DurableDeliveryCoordinator>,
        target_kind: &'static str,
        drifted_version: i64,
    ) -> Self {
        Self {
            inner: MemoryAppendPort::default(),
            coordinator,
            target_kind,
            drifted_version,
            armed: std::sync::atomic::AtomicBool::new(true),
        }
    }
}

impl ImmutableAppendPort for SchemaDriftAfterAppend {
    fn append_exact(
        &self,
        record_kind: &str,
        identity: &str,
        canonical_bytes: &[u8],
        sha256: &str,
    ) -> Result<String> {
        let immutable_ref =
            self.inner
                .append_exact(record_kind, identity, canonical_bytes, sha256)?;
        if record_kind == self.target_kind && self.armed.swap(false, Ordering::SeqCst) {
            self.coordinator
                .install_operation_postvalidation_test_fault(
                    OperationPostvalidationTestFault::SchemaVersion(self.drifted_version),
                )?;
        }
        Ok(immutable_ref)
    }
}

impl Default for FailScheduleHydrationAppliedOnce {
    fn default() -> Self {
        Self {
            inner: MemoryAppendPort::default(),
            fail_next_hydration_ack: std::sync::atomic::AtomicBool::new(true),
        }
    }
}

impl MemoryAppendPort {
    fn record_count(&self) -> usize {
        self.records.lock().expect("append records").len()
    }

    fn count_kind(&self, kind: &str) -> usize {
        self.records
            .lock()
            .expect("append records")
            .values()
            .filter(|record| record.record_kind == kind)
            .count()
    }
}

impl ImmutableAppendPort for FailScheduleHydrationAppliedOnce {
    fn append_exact(
        &self,
        record_kind: &str,
        identity: &str,
        canonical_bytes: &[u8],
        sha256: &str,
    ) -> Result<String> {
        if record_kind == "ScheduleHydrationApplied"
            && self.fail_next_hydration_ack.swap(false, Ordering::SeqCst)
        {
            return Err(DurableDeliveryError::Io(std::io::Error::other(
                "TEST_CODE_INJECTED_HYDRATION_ACK_APPEND_FAILURE",
            )));
        }
        self.inner
            .append_exact(record_kind, identity, canonical_bytes, sha256)
    }
}

impl ImmutableAppendPort for MemoryAppendPort {
    fn append_exact(
        &self,
        record_kind: &str,
        identity: &str,
        canonical_bytes: &[u8],
        sha256: &str,
    ) -> Result<String> {
        let mut records = self.records.lock().expect("append records");
        let immutable_ref = format!("immutable://{record_kind}/{identity}");
        match records.get(identity) {
            Some(stored)
                if stored.record_kind == record_kind
                    && stored.canonical_bytes == canonical_bytes
                    && stored.sha256 == sha256 =>
            {
                Ok(stored.immutable_ref.clone())
            }
            Some(_) => Err(DurableDeliveryError::ImmutableAppendConflict(
                identity.to_owned(),
            )),
            None => {
                records.insert(
                    identity.to_owned(),
                    MemoryAppendRecord {
                        record_kind: record_kind.to_owned(),
                        canonical_bytes: canonical_bytes.to_vec(),
                        sha256: sha256.to_owned(),
                        immutable_ref: immutable_ref.clone(),
                    },
                );
                Ok(immutable_ref)
            }
        }
    }
}

struct RacingAppendPort {
    inner: MemoryAppendPort,
    target_kind: &'static str,
    first_two_calls: Barrier,
    target_calls: AtomicUsize,
    target_identities: Mutex<Vec<String>>,
}

impl RacingAppendPort {
    fn new(target_kind: &'static str) -> Self {
        Self {
            inner: MemoryAppendPort::default(),
            target_kind,
            first_two_calls: Barrier::new(2),
            target_calls: AtomicUsize::new(0),
            target_identities: Mutex::new(Vec::new()),
        }
    }
}

impl ImmutableAppendPort for RacingAppendPort {
    fn append_exact(
        &self,
        record_kind: &str,
        identity: &str,
        canonical_bytes: &[u8],
        sha256: &str,
    ) -> Result<String> {
        let immutable_ref =
            self.inner
                .append_exact(record_kind, identity, canonical_bytes, sha256)?;
        if record_kind == self.target_kind {
            let call = self.target_calls.fetch_add(1, Ordering::SeqCst);
            if call < 2 {
                self.target_identities
                    .lock()
                    .expect("race identities")
                    .push(identity.to_owned());
                self.first_two_calls.wait();
            }
        }
        Ok(immutable_ref)
    }
}

struct StaticSink {
    calls: AtomicUsize,
    result: AuthoritativeSinkResult,
}

struct BlockingSink {
    calls: AtomicUsize,
    entered: Mutex<Option<Sender<()>>>,
    release: Barrier,
    result: AuthoritativeSinkResult,
}

impl BlockingSink {
    fn new(result: AuthoritativeSinkResult) -> (Arc<Self>, mpsc::Receiver<()>) {
        let (sender, receiver) = mpsc::channel();
        (
            Arc::new(Self {
                calls: AtomicUsize::new(0),
                entered: Mutex::new(Some(sender)),
                release: Barrier::new(2),
                result,
            }),
            receiver,
        )
    }
}

impl AuthoritativeSinkPort for BlockingSink {
    fn sink_identity(&self) -> &str {
        "TEST_CODE_BLOCKING_AUTHORITATIVE_SINK"
    }

    fn deliver(&self, _request: &AuthoritativeDeliveryRequest) -> AuthoritativeSinkResult {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(sender) = self.entered.lock().expect("entered sender").take() {
            sender.send(()).expect("signal sink entry");
        }
        self.release.wait();
        self.result.clone()
    }
}

impl StaticSink {
    fn new(result: AuthoritativeSinkResult) -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            result,
        })
    }
}

impl AuthoritativeSinkPort for StaticSink {
    fn sink_identity(&self) -> &str {
        "TEST_CODE_AUTHORITATIVE_SINK"
    }

    fn deliver(&self, _request: &AuthoritativeDeliveryRequest) -> AuthoritativeSinkResult {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.result.clone()
    }
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, 30, 8, 0, 0)
        .single()
        .expect("valid timestamp")
}

fn receipt(at: DateTime<Utc>) -> TypedReceipt {
    TypedReceipt {
        channel: "TEST_CODE_CHANNEL".to_owned(),
        provider: "TEST_CODE_PROVIDER".to_owned(),
        message_id: format!("TEST_CODE_MESSAGE_{}", at.timestamp()),
        platform_message_id: Some("TEST_CODE_PLATFORM_MESSAGE".to_owned()),
        accepted_at: at,
        latency_ms: Some(17),
    }
}

fn rejection(at: DateTime<Utc>, retry_authorized: bool) -> TypedRejection {
    TypedRejection {
        reason_code: "TEST_CODE_DEFINITE_REJECTION".to_owned(),
        evidence: b"TEST_CODE_REJECTION_EVIDENCE".to_vec(),
        retry_authorized,
        observed_at: at,
    }
}

fn uncertainty(at: DateTime<Utc>) -> TypedUncertainty {
    TypedUncertainty {
        reason_code: "TEST_CODE_TRANSPORT_UNCERTAIN".to_owned(),
        evidence: b"TEST_CODE_UNCERTAINTY_EVIDENCE".to_vec(),
        observed_at: at,
    }
}

fn envelope(
    label: &str,
    push_kind: PushKind,
    sub_kind: DeliverySubKind,
    business_date: &str,
    task_bound: bool,
) -> DeliveryEnvelope {
    let scope = compiled_policy_catalog()
        .into_iter()
        .find(|row| row.push_kind == push_kind && row.sub_kind == sub_kind)
        .expect("registered policy")
        .cooldown_scope;
    let scope_key = match scope {
        CooldownScope::Global => "GLOBAL".to_owned(),
        CooldownScope::PerTicket => format!("SSE:EQUITY:TEST_CODE_{label}"),
    };
    let binding = task_bound.then(|| {
        TaskBinding::new(
            format!("TEST_CODE_TASK_{label}"),
            format!("TEST_CODE_TRANSITION_BASIS_{label}").into_bytes(),
        )
        .expect("valid task binding")
    });
    let source_binding = if push_kind == PushKind::PreopenNewsHot {
        serde_json::to_vec(&serde_json::json!({
            "schema_version": "P01_SOURCE_BINDING_V1",
            "render_mode": "Scheduled"
        }))
        .expect("serialize TEST_CODE P-01 binding")
    } else {
        format!("TEST_CODE_SOURCE_BINDING_{label}").into_bytes()
    };
    DeliveryEnvelope::new(
        business_date,
        push_kind,
        sub_kind,
        scope_key,
        format!("TEST_CODE_OCCURRENCE_{label}"),
        format!("TEST_CODE_EVIDENCE_{label}"),
        source_binding,
        format!("TEST_CODE_SUBJECT_HASH_{label}"),
        format!("TEST_CODE_RENDERED_BODY_{label}").into_bytes(),
        true,
        binding,
    )
    .expect("valid envelope")
}

fn review_envelope_with_task_identity(
    label: &str,
    push_kind: PushKind,
    business_date: &str,
    task_identity: &str,
) -> DeliveryEnvelope {
    DeliveryEnvelope::new(
        business_date,
        push_kind,
        DeliverySubKind::None,
        "GLOBAL",
        format!("TEST_CODE_OCCURRENCE_{label}"),
        format!("TEST_CODE_EVIDENCE_{label}"),
        format!("TEST_CODE_SOURCE_BINDING_{label}").into_bytes(),
        format!("TEST_CODE_SUBJECT_HASH_{label}"),
        format!("TEST_CODE_RENDERED_BODY_{label}").into_bytes(),
        true,
        Some(
            TaskBinding::new(
                task_identity,
                format!("TEST_CODE_TRANSITION_BASIS_{label}").into_bytes(),
            )
            .expect("valid review task binding"),
        ),
    )
    .expect("valid review envelope")
}

fn prepare_reserved(
    fixture: &Fixture,
    envelope: &DeliveryEnvelope,
    append: &dyn ImmutableAppendPort,
) {
    let outcome = fixture
        .coordinator
        .prepare(envelope, 1, now())
        .expect("prepare");
    assert_eq!(outcome.state, DecisionState::Reserved);
    let summary = fixture
        .coordinator
        .reconcile_all_pending(append, now())
        .expect("reconcile prepare audits");
    assert_eq!(summary.provider_calls, 0);
    assert_eq!(summary.sink_calls, 0);
}

fn accepted_pending_fixture(
    fixture: &Fixture,
    label: &str,
    initial_append: &MemoryAppendPort,
) -> DeliveryEnvelope {
    let candidate = envelope(
        label,
        PushKind::ReviewProviderTopN,
        DeliverySubKind::None,
        "2026-07-30",
        true,
    );
    prepare_reserved(fixture, &candidate, initial_append);
    let sink = StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(now())));
    let sinks: Vec<AuthoritativeSink> = vec![sink];
    fixture
        .coordinator
        .resume_deliverable(&candidate.decision_identity, &sinks, now())
        .expect("freeze accepted disposition and delivery audit");
    candidate
}

fn fixture_coordinator_arc(fixture: &Fixture) -> Arc<DurableDeliveryCoordinator> {
    fixture
        .coordinator
        .0
        .as_ref()
        .expect("fixture coordinator is live")
        .clone()
}

fn establish_authoritative_delivered_projection(
    fixture: &Fixture,
    label: &str,
    append: &MemoryAppendPort,
    task_bound: bool,
) -> DeliveryEnvelope {
    let candidate = envelope(
        label,
        PushKind::ReviewProviderTopN,
        DeliverySubKind::None,
        "2026-07-30",
        task_bound,
    );
    prepare_reserved(fixture, &candidate, append);
    let sink = StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(now())));
    let sinks: Vec<AuthoritativeSink> = vec![sink];
    fixture
        .coordinator
        .resume_deliverable(&candidate.decision_identity, &sinks, now())
        .expect("persist authoritative acceptance");
    reconcile_terminal(
        fixture,
        append,
        DecisionState::Delivered,
        &candidate.decision_identity,
    );
    candidate
}

#[test]
fn br200_r09_business_date_once_preflight_reuses_delivered_without_writes() {
    let fixture = Fixture::new("BR200_R09_DELIVERED");
    let append = MemoryAppendPort::default();
    let task_identity = "TEST_CODE_TASK_BR200_R09";
    let candidate = review_envelope_with_task_identity(
        "BR200_R09_DELIVERED",
        PushKind::ReviewProviderTopN,
        "2026-07-30",
        task_identity,
    );
    prepare_reserved(&fixture, &candidate, &append);
    let sink = StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(now())));
    let sinks: Vec<AuthoritativeSink> = vec![sink.clone()];
    fixture
        .coordinator
        .resume_deliverable(&candidate.decision_identity, &sinks, now())
        .expect("deliver R-09");
    reconcile_terminal(
        &fixture,
        &append,
        DecisionState::Delivered,
        &candidate.decision_identity,
    );
    let decision_count = fixture.query_i64("SELECT COUNT(*) FROM delivery_decisions");

    let evidence = fixture
        .coordinator
        .inspect_review_task_occurrence(
            "2026-07-30",
            PushKind::ReviewProviderTopN,
            DeliverySubKind::None,
            "GLOBAL",
            task_identity,
        )
        .expect("read R-09 occurrence")
        .expect("existing R-09 occurrence");

    assert_eq!(evidence.decision_identity, candidate.decision_identity);
    assert_eq!(evidence.state, DecisionState::Delivered);
    assert!(evidence.schedule_hydration.is_some());
    assert_eq!(sink.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM delivery_decisions"),
        decision_count,
        "read-only preflight must not create a second decision"
    );
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM business_date_once_claims"),
        1
    );
}

#[test]
fn br214_retired_policy_denial_is_not_a_current_occurrence() {
    let fixture = Fixture::new("BR214_RETIRED_POLICY");

    let seed = |label: &str, task_identity: &str, policy_version: i64| {
        let envelope = review_envelope_with_task_identity(
            label,
            PushKind::ReviewLhb,
            "2026-07-30",
            task_identity,
        );
        let transition_basis = format!("TEST_CODE_TRANSITION_BASIS_{label}").into_bytes();
        let transition_basis_hash = sha256_hex(&transition_basis);
        // `decision_identity` is a hash over material that includes `policy_version`
        // (model.rs `DecisionIdentityMaterial`), so a retired-policy envelope must be
        // re-identified or `parse_envelope` rejects it as tampered evidence.
        #[derive(Serialize)]
        struct IdentityMaterial<'a> {
            domain: &'static str,
            policy_version: i64,
            business_date: &'a str,
            push_kind: PushKind,
            sub_kind: DeliverySubKind,
            cooldown_scope: CooldownScope,
            scope_key: &'a str,
            schedule_occurrence_identity: &'a str,
            source_evidence_fingerprint: &'a str,
            delivery_subject_hash: &'a str,
            rendered_content_sha256: &'a str,
        }
        let identity = sha256_hex(
            &serde_json::to_vec(&IdentityMaterial {
                domain: "durable-delivery-decision-v1",
                policy_version,
                business_date: &envelope.business_date,
                push_kind: envelope.push_kind,
                sub_kind: envelope.sub_kind,
                cooldown_scope: envelope.cooldown_scope,
                scope_key: &envelope.scope_key,
                schedule_occurrence_identity: &envelope.schedule_occurrence_identity,
                source_evidence_fingerprint: &envelope.source_evidence_fingerprint,
                delivery_subject_hash: &envelope.delivery_subject_hash,
                rendered_content_sha256: &envelope.rendered_content_sha256,
            })
            .expect("serialize identity material"),
        );
        let mut document: serde_json::Value =
            serde_json::to_value(&envelope).expect("serialize envelope");
        document["policy_version"] = serde_json::json!(policy_version);
        document["decision_identity"] = serde_json::json!(identity);
        let canonical = serde_json::to_vec(&document).expect("serialize patched envelope");
        let canonical_hash = sha256_hex(&canonical);
        let connection = Connection::open(&fixture.database_path).expect("open write connection");
        connection
            .execute(
                "INSERT INTO delivery_decisions(
                   decision_identity,business_date,push_kind,sub_kind,cooldown_scope,
                   scope_key,state,envelope_version,envelope_canonical,envelope_sha256,
                   task_binding_present,transition_basis_canonical,transition_basis_sha256,
                   reservation_generation,current_budget_reservation_identity,
                   current_cooldown_reservation_identity,current_attempt_identity,
                   current_disposition_identity,fence_generation,retry_authorized,
                   created_at,updated_at
                 ) VALUES(
                   ?1,'2026-07-30','ReviewLhb','NONE','Global',
                   'GLOBAL','RejectedDurable',1,?2,?3,1,?4,?5,0,NULL,NULL,NULL,NULL,0,0,
                   '2026-07-30T21:00:00Z','2026-07-30T21:00:00Z'
                 )",
                params![
                    identity,
                    canonical,
                    canonical_hash,
                    transition_basis,
                    transition_basis_hash
                ],
            )
            .expect("seed decision");
    };

    let inspect = |task_identity: &str| {
        fixture
            .coordinator
            .inspect_review_task_occurrence(
                "2026-07-30",
                PushKind::ReviewLhb,
                DeliverySubKind::None,
                "GLOBAL",
                task_identity,
            )
            .expect("read occurrence")
    };

    let current_task = "TEST_CODE_TASK_BR214_CURRENT";
    seed("BR214_CURRENT", current_task, POLICY_VERSION);
    let current = inspect(current_task).expect("current-policy denial stays authoritative");
    assert_eq!(current.state, DecisionState::RejectedDurable);

    let retired_task = "TEST_CODE_TASK_BR214_RETIRED";
    seed("BR214_RETIRED", retired_task, POLICY_VERSION - 1);
    assert!(
        inspect(retired_task).is_none(),
        "BR-214: a denial frozen under a retired policy_version must not keep denying \
         under the successor policy"
    );
}

#[test]
fn br214_daily_review_kinds_are_business_date_once() {
    let catalog = compiled_policy_catalog();
    for kind in [
        PushKind::ReviewMarket,
        PushKind::ReviewLhb,
        PushKind::ReviewSignal,
        PushKind::ReviewFailure,
        PushKind::TomorrowWatch,
        PushKind::ReviewProviderTopN,
    ] {
        let row = catalog
            .iter()
            .find(|row| row.push_kind == kind)
            .unwrap_or_else(|| panic!("missing policy for {kind}"));
        assert_eq!(
            row.window_mode,
            WindowMode::BusinessDateOnce,
            "BR-214: {kind} must be idempotent per business date, not per rolling window"
        );
    }
    assert_eq!(
        POLICY_VERSION, 5,
        "BR-245: TomorrowWatch Global BusinessDateOnce budget-exempt policy changed \
         policy semantics, POLICY_VERSION must be bumped because it is decision_identity hash \
         material (bumped 4 -> 5 on 2026-08-18)"
    );
}

#[test]
fn br245_prior_late_acceptance_does_not_block_next_business_date() {
    let fixture = Fixture::new("BR245_NEXT_BUSINESS_DATE");
    let append = MemoryAppendPort::default();
    let prior_accepted_at = Utc
        .with_ymd_and_hms(2026, 8, 17, 15, 20, 35)
        .single()
        .expect("valid prior acceptance timestamp")
        + chrono::Duration::milliseconds(788);
    let next_admission_at = Utc
        .with_ymd_and_hms(2026, 8, 18, 13, 0, 45)
        .single()
        .expect("valid next business-date timestamp");

    let prior = envelope(
        "BR245_PRIOR_ACCEPTED",
        PushKind::TomorrowWatch,
        DeliverySubKind::None,
        "2026-08-17",
        true,
    );
    let prior_prepare = fixture
        .coordinator
        .prepare(&prior, 1, prior_accepted_at)
        .expect("prepare prior R-07");
    assert_eq!(prior_prepare.state, DecisionState::Reserved);
    fixture
        .coordinator
        .reconcile_all_pending(&append, prior_accepted_at)
        .expect("append prior reservation audits");
    let prior_sink = StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(
        prior_accepted_at,
    )));
    let prior_sinks: Vec<AuthoritativeSink> = vec![prior_sink.clone()];
    fixture
        .coordinator
        .resume_deliverable(&prior.decision_identity, &prior_sinks, prior_accepted_at)
        .expect("accept prior R-07");
    fixture
        .coordinator
        .reconcile_all_pending(&append, prior_accepted_at)
        .expect("finalize prior R-07");
    assert_eq!(
        fixture
            .coordinator
            .decision_state(&prior.decision_identity)
            .expect("prior state"),
        DecisionState::Delivered
    );

    let next = envelope(
        "BR245_NEXT_ACCEPTED",
        PushKind::TomorrowWatch,
        DeliverySubKind::None,
        "2026-08-18",
        true,
    );
    let next_prepare = fixture
        .coordinator
        .prepare(&next, 1, next_admission_at)
        .expect("prepare next business-date R-07");
    assert_eq!(
        next_prepare.state,
        DecisionState::Reserved,
        "a prior business-date acceptance must not become a rolling cooldown conflict"
    );
    fixture
        .coordinator
        .reconcile_all_pending(&append, next_admission_at)
        .expect("append next reservation audits");
    let next_sink = StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(
        next_admission_at,
    )));
    let next_sinks: Vec<AuthoritativeSink> = vec![next_sink.clone()];
    fixture
        .coordinator
        .resume_deliverable(&next.decision_identity, &next_sinks, next_admission_at)
        .expect("deliver next business-date R-07");
    fixture
        .coordinator
        .reconcile_all_pending(&append, next_admission_at)
        .expect("finalize next business-date R-07");

    assert_eq!(prior_sink.calls.load(Ordering::SeqCst), 1);
    assert_eq!(next_sink.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture
            .coordinator
            .decision_state(&next.decision_identity)
            .expect("next state"),
        DecisionState::Delivered
    );
}

#[test]
fn br245_same_business_date_reuses_or_rejects_without_new_sink() {
    let fixture = Fixture::new("BR245_SAME_BUSINESS_DATE");
    let append = MemoryAppendPort::default();
    let admission_at = Utc
        .with_ymd_and_hms(2026, 8, 18, 13, 0, 45)
        .single()
        .expect("valid R-07 admission timestamp");
    let original = envelope(
        "BR245_ORIGINAL",
        PushKind::TomorrowWatch,
        DeliverySubKind::None,
        "2026-08-18",
        true,
    );
    let prepared = fixture
        .coordinator
        .prepare(&original, 1, admission_at)
        .expect("prepare original R-07");
    assert_eq!(prepared.state, DecisionState::Reserved);
    fixture
        .coordinator
        .reconcile_all_pending(&append, admission_at)
        .expect("append original reservation audits");
    let original_sink = StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(admission_at)));
    let original_sinks: Vec<AuthoritativeSink> = vec![original_sink.clone()];
    fixture
        .coordinator
        .resume_deliverable(&original.decision_identity, &original_sinks, admission_at)
        .expect("deliver original R-07");
    fixture
        .coordinator
        .reconcile_all_pending(&append, admission_at)
        .expect("finalize original R-07");

    let forbidden_replay_sink =
        StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(admission_at)));
    let forbidden_replay_sinks: Vec<AuthoritativeSink> = vec![forbidden_replay_sink.clone()];
    let replay = fixture
        .coordinator
        .prepare(&original, 1, admission_at + chrono::Duration::seconds(1))
        .expect("reuse original R-07 decision");
    assert_eq!(replay.state, DecisionState::Delivered);
    assert_eq!(replay.sink_calls, 0);
    let replay_resume = fixture
        .coordinator
        .resume_deliverable(
            &original.decision_identity,
            &forbidden_replay_sinks,
            admission_at + chrono::Duration::seconds(1),
        )
        .expect("inspect terminal replay");
    assert_eq!(replay_resume.state, DecisionState::Delivered);
    assert_eq!(replay_resume.sink_calls, 0);

    let conflicting = envelope(
        "BR245_CONFLICTING",
        PushKind::TomorrowWatch,
        DeliverySubKind::None,
        "2026-08-18",
        true,
    );
    let conflict = fixture
        .coordinator
        .prepare(&conflicting, 1, admission_at + chrono::Duration::seconds(2))
        .expect("persist same-date R-07 conflict");
    assert_eq!(conflict.state, DecisionState::RejectedAuditPending);
    assert_eq!(conflict.sink_calls, 0);
    fixture
        .coordinator
        .reconcile_all_pending(&append, admission_at + chrono::Duration::seconds(2))
        .expect("finalize same-date R-07 conflict");
    let forbidden_conflict_sink =
        StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(admission_at)));
    let forbidden_conflict_sinks: Vec<AuthoritativeSink> = vec![forbidden_conflict_sink.clone()];
    let conflict_resume = fixture
        .coordinator
        .resume_deliverable(
            &conflicting.decision_identity,
            &forbidden_conflict_sinks,
            admission_at + chrono::Duration::seconds(3),
        )
        .expect("inspect rejected same-date conflict");
    assert_eq!(conflict_resume.state, DecisionState::RejectedDurable);
    assert_eq!(conflict_resume.sink_calls, 0);

    assert_eq!(original_sink.calls.load(Ordering::SeqCst), 1);
    assert_eq!(forbidden_replay_sink.calls.load(Ordering::SeqCst), 0);
    assert_eq!(forbidden_conflict_sink.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM business_date_once_claims"),
        1
    );
}

#[test]
fn p01_policy_is_global_business_date_once_and_budget_exempt() {
    let row = compiled_policy_catalog()
        .into_iter()
        .find(|row| row.push_kind == PushKind::PreopenNewsHot)
        .expect("P-01 durable policy");

    assert_eq!(row.cooldown_scope, CooldownScope::Global);
    assert_eq!(row.window_mode, WindowMode::BusinessDateOnce);
    assert_eq!(row.sub_kind, DeliverySubKind::None);
    assert_eq!(row.base_cooldown_secs, Some(86_400));
    assert!(!row.counts_against_daily_budget);
    assert_eq!(row.push_kind.stable_template_id(), "preopen_news_hot_v1");
}

#[test]
fn t16_st_price_policy_is_per_ticket_rolling_and_budget_counted() {
    // 2026-09-19 用户决策: ST 涨跌幅变更提醒计入 30 条/日预算
    // (盘中信息卡, 非资金动作; 与 T0Advice/HoldingPlan 同待遇)。
    let row = compiled_policy_catalog()
        .into_iter()
        .find(|row| row.push_kind == PushKind::StPriceLimitChanged)
        .expect("T-16 durable policy");

    assert_eq!(row.cooldown_scope, CooldownScope::PerTicket);
    assert_eq!(row.window_mode, WindowMode::Rolling);
    assert_eq!(row.sub_kind, DeliverySubKind::None);
    assert_eq!(row.base_cooldown_secs, Some(86_400));
    assert!(row.counts_against_daily_budget);
    assert_eq!(row.push_kind.stable_template_id(), "st_price_limit_changed_v1");
}
#[test]
fn g5b_policy_is_global_no_cooldown_and_budget_exempt() {
    // 2026-09-20: G5b 每事件一推 (≤3/日), 无冷却 (WindowMode::None, HoldingEvent
    // 先例); 盘后归因类豁免日预算 (分流规则)。
    let row = compiled_policy_catalog()
        .into_iter()
        .find(|row| row.push_kind == PushKind::G5bAttribution)
        .expect("G5b durable policy");

    assert_eq!(row.cooldown_scope, CooldownScope::Global);
    assert_eq!(row.window_mode, WindowMode::None);
    assert_eq!(row.sub_kind, DeliverySubKind::None);
    assert_eq!(row.base_cooldown_secs, std::option::Option::None);
    assert!(!row.counts_against_daily_budget);
    assert_eq!(row.push_kind.stable_template_id(), "g5b_attribution_v1");
}

#[test]
fn a12_attribution_daily_policy_is_global_business_date_once_and_budget_exempt() {
    // 2026-09-20 用户决策 (分流规则): 每日必达类豁免日预算 — 15:05 归因日推
    // 与复盘类同语义 (BR-237), 不被盘中信号饿死。
    let row = compiled_policy_catalog()
        .into_iter()
        .find(|row| row.push_kind == PushKind::AttributionDaily)
        .expect("A-12 durable policy");

    assert_eq!(row.cooldown_scope, CooldownScope::Global);
    assert_eq!(row.window_mode, WindowMode::BusinessDateOnce);
    assert_eq!(row.sub_kind, DeliverySubKind::None);
    assert_eq!(row.base_cooldown_secs, Some(86_400));
    assert!(!row.counts_against_daily_budget);
    assert_eq!(row.push_kind.stable_template_id(), "attribution_daily_v1");
}

#[test]
fn intraday_market_policy_is_global_rolling_900_and_budget_counted() {
    // 2026-09-20: I-01 盘中轮动升级 counted — R-02 盘面走向每 5 分钟硬推,
    // Rolling 900s 镜像旧 L4 (notify cooldown_secs 900); 盘中信息卡计入
    // 30 条/日预算 (分流规则)。同 kind 的两个每日一次借用点 (BR-226 快照
    // 提醒/盘前预检) 同样计预算 — per-kind 粒度无法拆分, ≤3 槽/日残余行为
    // 记入 commit message。
    let row = compiled_policy_catalog()
        .into_iter()
        .find(|row| row.push_kind == PushKind::IntradayMarket)
        .expect("I-01 durable policy");

    assert_eq!(row.cooldown_scope, CooldownScope::Global);
    assert_eq!(row.window_mode, WindowMode::Rolling);
    assert_eq!(row.sub_kind, DeliverySubKind::None);
    assert_eq!(row.base_cooldown_secs, Some(900));
    assert!(row.counts_against_daily_budget);
    assert_eq!(row.push_kind.stable_template_id(), "intraday_market_v1");
}

#[test]
fn w13_p01_same_day_query_ignores_render_mode_but_reuses_one_claim() {
    let fixture = Fixture::new("W13_P01_SAME_DAY_KEY");
    let append = MemoryAppendPort::default();
    let p01_envelope = |mode: &str, label: &str| {
        DeliveryEnvelope::new(
            "2026-08-18",
            PushKind::PreopenNewsHot,
            DeliverySubKind::None,
            "GLOBAL",
            "p01:2026-08-18",
            format!("TEST_CODE_W13_P01_EVIDENCE_{label}"),
            serde_json::to_vec(&serde_json::json!({
                "schema_version": "P01_SOURCE_BINDING_V1",
                "render_mode": mode,
            }))
            .expect("serialize W13 P01 source binding"),
            "TEST_CODE_W13_P01_GLOBAL_SUBJECT",
            format!("TEST_CODE_W13_P01_RENDERED_{label}").into_bytes(),
            false,
            None,
        )
        .expect("valid W13 P01 envelope")
    };
    let scheduled = p01_envelope("Scheduled", "SCHEDULED");
    prepare_reserved(&fixture, &scheduled, &append);
    let sink = StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(now())));
    let sinks: Vec<AuthoritativeSink> = vec![sink.clone()];
    fixture
        .coordinator
        .resume_deliverable(&scheduled.decision_identity, &sinks, now())
        .expect("deliver scheduled P01 authority");
    reconcile_terminal(
        &fixture,
        &append,
        DecisionState::Delivered,
        &scheduled.decision_identity,
    );

    let compensation = p01_envelope("Compensation", "COMPENSATION");
    let conflict = fixture
        .coordinator
        .prepare(&compensation, 1, now() + chrono::Duration::seconds(1))
        .expect("same-day compensation remains the same P01 claim");
    assert_eq!(conflict.sink_calls, 0);
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM business_date_once_claims"),
        1
    );

    let terminal = match fixture
        .coordinator
        .inspect_p01_dedicated_terminal("2026-08-18")
        .expect("inspect exact P01 dedicated authority")
    {
        P01DedicatedTerminalQuery::Terminal(record) => record,
        other => panic!("expected exact P01 terminal, got {other:?}"),
    };
    assert_eq!(
        terminal.legacy_decision_identity,
        scheduled.decision_identity
    );
    assert_eq!(
        terminal.envelope_canonical,
        scheduled.canonical_bytes().unwrap()
    );
    assert_eq!(
        terminal.disposition,
        FoundationTerminalDisposition::Accepted
    );
    assert_eq!(
        terminal.accepted_channel.as_deref(),
        Some("TEST_CODE_CHANNEL")
    );
    assert_eq!(sink.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn p01_business_date_once_claim_inspection_is_read_only_without_task_binding() {
    let fixture = Fixture::new("P01_GENERIC_CLAIM_INSPECT");
    let append = MemoryAppendPort::default();
    let candidate = envelope(
        "P01_GENERIC_CLAIM_INSPECT",
        PushKind::PreopenNewsHot,
        DeliverySubKind::None,
        "2026-08-18",
        false,
    );
    prepare_reserved(&fixture, &candidate, &append);
    let decision_count = fixture.query_i64("SELECT COUNT(*) FROM delivery_decisions");

    let evidence = fixture
        .coordinator
        .inspect_business_date_once_claim(
            "2026-08-18",
            PushKind::PreopenNewsHot,
            DeliverySubKind::None,
            "GLOBAL",
            "TEST_CODE_OCCURRENCE_P01_GENERIC_CLAIM_INSPECT",
        )
        .expect("inspect generic P-01 claim")
        .expect("P-01 claim exists");

    assert_eq!(evidence.decision_identity, candidate.decision_identity);
    assert_eq!(evidence.state, DecisionState::Reserved);
    assert_eq!(evidence.sink_calls, 0);
    assert!(evidence.current_attempt_identity.is_none());
    assert!(evidence.authoritative_receipt_sha256.is_none());
    assert_eq!(evidence.source_binding_mode.as_deref(), Some("Scheduled"));
    assert!(evidence.schedule_hydration.is_none());

    let accepted_receipt = receipt(now());
    let receipt_canonical = serde_json::to_vec(&accepted_receipt).expect("serialize receipt");
    let mut receipt_preimage = b"stock_analysis.counted_receipt.v1\0".to_vec();
    receipt_preimage.extend_from_slice(&receipt_canonical);
    let expected_receipt_sha256 = sha256_hex(&receipt_preimage);
    let sink = StaticSink::new(AuthoritativeSinkResult::Accepted(accepted_receipt));
    let sinks: Vec<AuthoritativeSink> = vec![sink.clone()];
    fixture
        .coordinator
        .resume_deliverable(&candidate.decision_identity, &sinks, now())
        .expect("deliver generic P-01 claim");
    reconcile_terminal(
        &fixture,
        &append,
        DecisionState::Delivered,
        &candidate.decision_identity,
    );
    let delivered = fixture
        .coordinator
        .inspect_business_date_once_claim(
            "2026-08-18",
            PushKind::PreopenNewsHot,
            DeliverySubKind::None,
            "GLOBAL",
            "TEST_CODE_OCCURRENCE_P01_GENERIC_CLAIM_INSPECT",
        )
        .expect("inspect Delivered generic P-01 claim")
        .expect("Delivered P-01 claim exists");
    assert_eq!(delivered.state, DecisionState::Delivered);
    assert_eq!(delivered.sink_calls, 0);
    assert_eq!(delivered.source_binding_mode.as_deref(), Some("Scheduled"));
    assert!(delivered.current_attempt_identity.is_some());
    assert_eq!(
        delivered.authoritative_receipt_sha256.as_deref(),
        Some(expected_receipt_sha256.as_str())
    );
    assert_eq!(sink.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM delivery_decisions"),
        decision_count,
        "generic claim inspection must not prepare another decision"
    );
}

#[test]
fn p01_business_date_once_resume_uses_stored_envelope_and_never_resends_terminal_claims() {
    let fixture = Fixture::new("P01_GENERIC_CLAIM_RESUME");
    let append = MemoryAppendPort::default();
    let candidate = envelope(
        "P01_GENERIC_CLAIM_RESUME",
        PushKind::PreopenNewsHot,
        DeliverySubKind::None,
        "2026-08-18",
        false,
    );
    prepare_reserved(&fixture, &candidate, &append);
    let accepted = StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(now())));
    let accepted_sinks: Vec<AuthoritativeSink> = vec![accepted.clone()];

    let delivered = fixture
        .coordinator
        .resume_business_date_once_claim(
            "2026-08-18",
            PushKind::PreopenNewsHot,
            DeliverySubKind::None,
            "GLOBAL",
            "TEST_CODE_OCCURRENCE_P01_GENERIC_CLAIM_RESUME",
            &accepted_sinks,
            &append,
            now(),
        )
        .expect("resume exact stored P-01 claim")
        .expect("P-01 claim exists");
    assert_eq!(delivered.state, DecisionState::Delivered);
    assert_eq!(delivered.sink_calls, 1);
    assert!(delivered.current_attempt_identity.is_some());
    assert!(delivered.authoritative_receipt_sha256.is_some());
    assert_eq!(accepted.calls.load(Ordering::SeqCst), 1);

    let forbidden_resend = StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(now())));
    let forbidden_sinks: Vec<AuthoritativeSink> = vec![forbidden_resend.clone()];
    let repeated = fixture
        .coordinator
        .resume_business_date_once_claim(
            "2026-08-18",
            PushKind::PreopenNewsHot,
            DeliverySubKind::None,
            "GLOBAL",
            "TEST_CODE_OCCURRENCE_P01_GENERIC_CLAIM_RESUME",
            &forbidden_sinks,
            &append,
            now(),
        )
        .expect("inspect already Delivered P-01 claim")
        .expect("Delivered P-01 claim exists");
    assert_eq!(repeated.state, DecisionState::Delivered);
    assert_eq!(repeated.sink_calls, 0);
    assert_eq!(
        repeated.authoritative_receipt_sha256,
        delivered.authoritative_receipt_sha256
    );
    assert_eq!(forbidden_resend.calls.load(Ordering::SeqCst), 0);

    let uncertain_fixture = Fixture::new("P01_GENERIC_CLAIM_UNCERTAIN");
    let uncertain_append = MemoryAppendPort::default();
    let uncertain_candidate = envelope(
        "P01_GENERIC_CLAIM_UNCERTAIN",
        PushKind::PreopenNewsHot,
        DeliverySubKind::None,
        "2026-08-18",
        false,
    );
    prepare_reserved(&uncertain_fixture, &uncertain_candidate, &uncertain_append);
    let uncertain = StaticSink::new(AuthoritativeSinkResult::Uncertain(uncertainty(now())));
    let uncertain_sinks: Vec<AuthoritativeSink> = vec![uncertain];
    uncertain_fixture
        .coordinator
        .resume_deliverable(
            &uncertain_candidate.decision_identity,
            &uncertain_sinks,
            now(),
        )
        .expect("persist uncertain P-01 result");
    let forbidden_uncertain_resend =
        StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(now())));
    let forbidden_uncertain_sinks: Vec<AuthoritativeSink> =
        vec![forbidden_uncertain_resend.clone()];
    let uncertain_evidence = uncertain_fixture
        .coordinator
        .resume_business_date_once_claim(
            "2026-08-18",
            PushKind::PreopenNewsHot,
            DeliverySubKind::None,
            "GLOBAL",
            "TEST_CODE_OCCURRENCE_P01_GENERIC_CLAIM_UNCERTAIN",
            &forbidden_uncertain_sinks,
            &uncertain_append,
            now(),
        )
        .expect("reconcile uncertain P-01 claim")
        .expect("uncertain P-01 claim exists");
    assert_eq!(
        uncertain_evidence.state,
        DecisionState::UncertainManualReview
    );
    assert_eq!(uncertain_evidence.sink_calls, 0);
    assert!(uncertain_evidence.current_attempt_identity.is_some());
    assert!(uncertain_evidence.authoritative_receipt_sha256.is_none());
    assert_eq!(forbidden_uncertain_resend.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn br214_r04_business_date_once_preflight_prefers_original_delivered_over_later_denial() {
    let fixture = Fixture::new("BR200_R04_DELIVERED");
    let append = MemoryAppendPort::default();
    let task_identity = "TEST_CODE_TASK_BR200_R04";
    let delivered = review_envelope_with_task_identity(
        "BR200_R04_DELIVERED",
        PushKind::ReviewLhb,
        "2026-07-30",
        task_identity,
    );
    prepare_reserved(&fixture, &delivered, &append);
    let sink = StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(now())));
    let sinks: Vec<AuthoritativeSink> = vec![sink.clone()];
    fixture
        .coordinator
        .resume_deliverable(&delivered.decision_identity, &sinks, now())
        .expect("deliver R-04");
    reconcile_terminal(
        &fixture,
        &append,
        DecisionState::Delivered,
        &delivered.decision_identity,
    );

    let duplicate = review_envelope_with_task_identity(
        "BR200_R04_DUPLICATE",
        PushKind::ReviewLhb,
        "2026-07-30",
        task_identity,
    );
    let denied = fixture
        .coordinator
        .prepare(&duplicate, 1, now())
        .expect("freeze duplicate R-04 denial");
    assert_eq!(denied.state, DecisionState::RejectedAuditPending);
    reconcile_terminal(
        &fixture,
        &append,
        DecisionState::RejectedDurable,
        &duplicate.decision_identity,
    );
    let decision_count = fixture.query_i64("SELECT COUNT(*) FROM delivery_decisions");

    let evidence = fixture
        .coordinator
        .inspect_review_task_occurrence(
            "2026-07-30",
            PushKind::ReviewLhb,
            DeliverySubKind::None,
            "GLOBAL",
            task_identity,
        )
        .expect("read R-04 occurrence")
        .expect("existing R-04 occurrence");

    assert_eq!(evidence.decision_identity, delivered.decision_identity);
    assert_eq!(evidence.state, DecisionState::Delivered);
    assert!(evidence.schedule_hydration.is_some());
    assert_eq!(sink.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM delivery_decisions"),
        decision_count,
        "read-only preflight must not create a third decision"
    );
}

fn manual_accepted_pending_fixture(
    fixture: &Fixture,
    label: &str,
    append: &MemoryAppendPort,
) -> DeliveryEnvelope {
    let candidate = envelope(
        label,
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        "2026-07-30",
        false,
    );
    prepare_reserved(fixture, &candidate, append);
    let sink = StaticSink::new(AuthoritativeSinkResult::Uncertain(uncertainty(now())));
    let sinks: Vec<AuthoritativeSink> = vec![sink];
    fixture
        .coordinator
        .resume_deliverable(&candidate.decision_identity, &sinks, now())
        .expect("persist uncertain result");
    reconcile_terminal(
        fixture,
        append,
        DecisionState::UncertainManualReview,
        &candidate.decision_identity,
    );
    fixture
        .coordinator
        .resolve_uncertain(
            &ManualResolutionCommand {
                decision_identity: candidate.decision_identity.clone(),
                disposition: ManualDisposition::Accepted {
                    receipt: Some(receipt(now())),
                },
                operator_identity: format!("TEST_CODE_OPERATOR_{label}_0123456789"),
                reason: format!("TEST_CODE_VERIFIED_ACCEPTANCE_{label}"),
                external_evidence: format!("TEST_CODE_EXTERNAL_EVIDENCE_{label}").into_bytes(),
                resolved_at: now(),
            },
            append,
        )
        .expect("persist manual acceptance");
    candidate
}

fn establish_manual_delivered_projection(
    fixture: &Fixture,
    label: &str,
    append: &MemoryAppendPort,
) -> DeliveryEnvelope {
    let candidate = manual_accepted_pending_fixture(fixture, label, append);
    reconcile_terminal(
        fixture,
        append,
        DecisionState::Delivered,
        &candidate.decision_identity,
    );
    candidate
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DeliveredPrecommitPersistenceSnapshot {
    decision: Vec<String>,
    dispositions: Vec<String>,
    sink_results: Vec<String>,
    task_transitions: Vec<String>,
    outbox: Vec<String>,
    state_events: Vec<String>,
}

fn snapshot_rows(connection: &Connection, sql: &str, decision_identity: &str) -> Vec<String> {
    let mut statement = connection.prepare(sql).expect("prepare snapshot query");
    statement
        .query_map([decision_identity], |row| row.get::<_, String>(0))
        .expect("query persistence snapshot")
        .collect::<std::result::Result<Vec<_>, _>>()
        .expect("collect persistence snapshot")
}

fn delivered_precommit_persistence_snapshot(
    fixture: &Fixture,
    decision_identity: &str,
) -> DeliveredPrecommitPersistenceSnapshot {
    let connection =
        Connection::open(&fixture.database_path).expect("open Delivered precommit snapshot");
    DeliveredPrecommitPersistenceSnapshot {
        decision: snapshot_rows(
            &connection,
            "SELECT quote(decision_identity)||'|'||quote(state)||'|'||
                    quote(envelope_canonical)||'|'||quote(envelope_sha256)||'|'||
                    quote(current_attempt_identity)||'|'||
                    quote(current_disposition_identity)||'|'||
                    quote(fence_generation)||'|'||quote(retry_authorized)||'|'||
                    quote(updated_at)
             FROM delivery_decisions WHERE decision_identity=?1",
            decision_identity,
        ),
        dispositions: snapshot_rows(
            &connection,
            "SELECT quote(disposition_identity)||'|'||quote(attempt_identity)||'|'||
                    quote(resolution_identity)||'|'||quote(denial_identity)||'|'||
                    quote(disposition)||'|'||quote(disposition_canonical)||'|'||
                    quote(disposition_sha256)||'|'||quote(append_state)||'|'||
                    quote(immutable_audit_ref)||'|'||quote(created_at)
             FROM delivery_disposition_payloads
             WHERE decision_identity=?1 ORDER BY rowid",
            decision_identity,
        ),
        sink_results: snapshot_rows(
            &connection,
            "SELECT quote(result_event_identity)||'|'||quote(attempt_identity)||'|'||
                    quote(result_kind)||'|'||quote(observed_at)||'|'||
                    quote(fence_token)||'|'||quote(authoritative_for_state)||'|'||
                    quote(late_after_fence)||'|'||quote(authority_audit_identity)||'|'||
                    quote(late_receipt_audit_identity)||'|'||
                    quote(result_canonical)||'|'||quote(result_sha256)||'|'||
                    quote(channel)||'|'||quote(provider)||'|'||quote(message_id)||'|'||
                    quote(platform_message_id)||'|'||quote(accepted_at)||'|'||
                    quote(latency_ms)||'|'||quote(frozen_delivery_audit_canonical)||'|'||
                    quote(frozen_delivery_audit_sha256)||'|'||quote(delivery_audit_ref)
             FROM sink_results WHERE decision_identity=?1 ORDER BY rowid",
            decision_identity,
        ),
        task_transitions: snapshot_rows(
            &connection,
            "SELECT quote(transition_identity)||'|'||quote(disposition_identity)||'|'||
                    quote(task_binding_sha256)||'|'||quote(transition_canonical)||'|'||
                    quote(transition_sha256)||'|'||quote(append_state)||'|'||
                    quote(immutable_audit_ref)||'|'||quote(hydration_state)||'|'||
                    quote(hydration_ack_identity)||'|'||quote(hydrated_at)
             FROM task_transition_payloads
             WHERE decision_identity=?1 ORDER BY rowid",
            decision_identity,
        ),
        outbox: snapshot_rows(
            &connection,
            "SELECT quote(audit_identity)||'|'||quote(attempt_identity)||'|'||
                    quote(audit_kind)||'|'||quote(predecessor_audit_identity)||'|'||
                    quote(audit_canonical)||'|'||quote(audit_sha256)||'|'||
                    quote(append_state)||'|'||quote(immutable_audit_ref)||'|'||
                    quote(created_at)
             FROM immutable_audit_outbox
             WHERE decision_identity=?1 ORDER BY rowid",
            decision_identity,
        ),
        state_events: snapshot_rows(
            &connection,
            "SELECT quote(event_seq)||'|'||quote(state_event_identity)||'|'||
                    quote(from_state)||'|'||quote(to_state)||'|'||quote(actor)||'|'||
                    quote(operator_identity)||'|'||quote(evidence_canonical)||'|'||
                    quote(evidence_sha256)||'|'||quote(audit_identity)
             FROM delivery_state_events
             WHERE decision_identity=?1 ORDER BY event_seq",
            decision_identity,
        ),
    }
}

fn advance_to_delivered_precommit_boundary(
    fixture: &Fixture,
    candidate: &DeliveryEnvelope,
    append: &MemoryAppendPort,
) {
    fixture
        .coordinator
        .install_delivered_reconcile_test_hook(|| {
            Err(DurableDeliveryError::InvalidConfiguration(
                "TEST_CODE_STOP_BEFORE_FINAL_DELIVERED".to_owned(),
            ))
        })
        .expect("install one-shot pre-Delivered boundary hook");
    let error = fixture
        .coordinator
        .reconcile_all_pending(append, now())
        .expect_err("pre-Delivered boundary hook must stop before final transaction");
    assert!(
        error
            .to_string()
            .contains("TEST_CODE_STOP_BEFORE_FINAL_DELIVERED"),
        "unexpected pre-Delivered boundary error: {error}"
    );
    assert_eq!(
        fixture
            .coordinator
            .decision_state(&candidate.decision_identity)
            .expect("state at Delivered precommit boundary"),
        DecisionState::AcceptedTaskTransitionPending
    );
    assert_eq!(
        fixture
            .query_i64("SELECT COUNT(*) FROM immutable_audit_outbox WHERE append_state='Pending'"),
        0,
        "all legal audit acknowledgements must be complete before the final transaction"
    );
    assert_eq!(
        fixture.query_i64(
            "SELECT COUNT(*) FROM task_transition_payloads
             WHERE append_state='Appended' AND immutable_audit_ref IS NOT NULL"
        ),
        1,
        "the exact task-transition acknowledgement must predate the final transaction"
    );
}

#[allow(clippy::too_many_arguments)]
fn assert_delivered_precommit_fault_rolls_back_then_exact_retry_delivers(
    fixture: &Fixture,
    candidate: &DeliveryEnvelope,
    append: &MemoryAppendPort,
    fault: DeliveredPrecommitTestFault,
    canonical_sql: &str,
    sha256_sql: &str,
    immutable_trigger: &str,
    expected_error: &str,
) {
    advance_to_delivered_precommit_boundary(fixture, candidate, append);
    let before_fault =
        delivered_precommit_persistence_snapshot(fixture, &candidate.decision_identity);
    let original_canonical = fixture.query_blob(canonical_sql);
    let original_sha256 = fixture
        .query_strings(sha256_sql)
        .into_iter()
        .next()
        .expect("original semantic evidence hash");
    fixture
        .coordinator
        .install_delivered_precommit_test_fault(fault)
        .expect("install one-shot Delivered precommit fault");

    let error = fixture
        .coordinator
        .reconcile_all_pending(append, now())
        .expect_err("self-hashed semantic corruption must fail Delivered precommit");
    assert!(
        error.to_string().contains(expected_error),
        "unexpected Delivered precommit rejection: {error}"
    );
    assert_eq!(
        delivered_precommit_persistence_snapshot(fixture, &candidate.decision_identity),
        before_fault,
        "failed final BEGIN IMMEDIATE transaction must preserve the exact pre-call decision, \
         semantic evidence, acknowledgement, outbox and state-event rows"
    );
    assert_eq!(
        fixture.query_blob(canonical_sql),
        original_canonical,
        "same-transaction semantic mutation must roll back exact canonical bytes"
    );
    assert_eq!(
        fixture
            .query_strings(sha256_sql)
            .into_iter()
            .next()
            .expect("semantic evidence hash after rollback"),
        original_sha256,
        "same-transaction semantic mutation must roll back its self-consistent hash"
    );
    assert_eq!(
        fixture.query_i64(&format!(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type='trigger' AND name='{immutable_trigger}'"
        )),
        1,
        "test-only trigger drop must roll back with the rejected transaction"
    );
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM delivery_state_events WHERE to_state='Delivered'"),
        0,
        "rejected precommit must not persist a Delivered state event"
    );
    assert_eq!(
        fixture.query_i64(
            "SELECT COUNT(*)
             FROM immutable_audit_outbox o
             JOIN delivery_state_events e ON e.audit_identity=o.audit_identity
             WHERE e.to_state='Delivered'"
        ),
        0,
        "rejected precommit must not persist or acknowledge a Delivered audit"
    );

    reconcile_terminal(
        fixture,
        append,
        DecisionState::Delivered,
        &candidate.decision_identity,
    );
    assert_eq!(
        fixture.query_blob(canonical_sql),
        original_canonical,
        "exact legal evidence must remain unchanged on retry"
    );
    assert_eq!(
        fixture
            .query_strings(sha256_sql)
            .into_iter()
            .next()
            .expect("semantic evidence hash after exact retry"),
        original_sha256
    );
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM delivery_state_events WHERE to_state='Delivered'"),
        1,
        "same fixture must commit exactly one Delivered transition after exact retry"
    );
    assert_eq!(
        fixture.query_i64(
            "SELECT COUNT(*)
             FROM immutable_audit_outbox o
             JOIN delivery_state_events e ON e.audit_identity=o.audit_identity
             WHERE e.to_state='Delivered'
               AND o.append_state='Appended'
               AND o.immutable_audit_ref IS NOT NULL"
        ),
        1,
        "exact retry must durably acknowledge the single Delivered audit"
    );
}

#[test]
fn br192_delivered_cas_revalidates_current_evidence_inside_immediate_transaction() {
    for task_bound in [false, true] {
        let mode = if task_bound { "TASK" } else { "NO_TASK" };
        let race_fixture = Fixture::new(&format!("DELIVERED_TX_RACE_{mode}"));
        let append = MemoryAppendPort::default();
        let candidate = envelope(
            &format!("DELIVERED_TX_RACE_{mode}"),
            PushKind::ReviewProviderTopN,
            DeliverySubKind::None,
            "2026-07-30",
            task_bound,
        );
        prepare_reserved(&race_fixture, &candidate, &append);
        let sink = StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(now())));
        let sinks: Vec<AuthoritativeSink> = vec![sink];
        race_fixture
            .coordinator
            .resume_deliverable(&candidate.decision_identity, &sinks, now())
            .expect("persist authoritative acceptance");

        let second = race_fixture.second_coordinator(&format!("DELIVERED_TX_RACE_{mode}"));
        let decision_identity = candidate.decision_identity.clone();
        race_fixture
            .coordinator
            .install_delivered_reconcile_test_hook(move || {
                let start = Arc::new(Barrier::new(2));
                let worker_start = start.clone();
                let worker = std::thread::spawn(move || {
                    worker_start.wait();
                    second.replace_current_disposition_identity_for_test(
                        &decision_identity,
                        "TEST_CODE_RACED_CURRENT_DISPOSITION",
                    )
                });
                start.wait();
                worker.join().map_err(|_| {
                    DurableDeliveryError::InvalidConfiguration(
                        "TEST_CODE Delivered race worker panicked".to_owned(),
                    )
                })??;
                Ok(())
            })
            .expect("install Delivered race hook");

        let error = race_fixture
            .coordinator
            .reconcile_all_pending(&append, now())
            .expect_err("raced current evidence must reject Delivered");
        assert!(
            matches!(
                &error,
                DurableDeliveryError::PolicyMismatch(reason)
                    if reason.contains("current disposition")
            ),
            "unexpected raced Delivered rejection: {error}"
        );
        let state = race_fixture
            .coordinator
            .decision_state(&candidate.decision_identity)
            .expect("state after raced Delivered CAS");
        assert_ne!(
            state,
            DecisionState::Delivered,
            "raced current evidence must prevent Delivered"
        );

        let normal_fixture = Fixture::new(&format!("DELIVERED_TX_NORMAL_{mode}"));
        let normal_append = MemoryAppendPort::default();
        let normal = establish_authoritative_delivered_projection(
            &normal_fixture,
            &format!("DELIVERED_TX_NORMAL_{mode}"),
            &normal_append,
            task_bound,
        );
        assert_eq!(
            normal_fixture
                .coordinator
                .decision_state(&normal.decision_identity)
                .expect("normal Delivered state"),
            DecisionState::Delivered
        );
    }
}

#[test]
fn br192_delivered_rejects_self_hashed_authoritative_disposition_semantic_corruption() {
    let fixture = Fixture::new("DELIVERED_STRICT_DISPOSITION");
    let append = MemoryAppendPort::default();
    let candidate = accepted_pending_fixture(&fixture, "DELIVERED_STRICT_DISPOSITION", &append);
    assert_delivered_precommit_fault_rolls_back_then_exact_retry_delivers(
        &fixture,
        &candidate,
        &append,
        DeliveredPrecommitTestFault::AuthoritativeDispositionSemanticBinding,
        "SELECT p.disposition_canonical
         FROM delivery_decisions d
         JOIN delivery_disposition_payloads p
           ON p.disposition_identity=d.current_disposition_identity",
        "SELECT p.disposition_sha256
         FROM delivery_decisions d
         JOIN delivery_disposition_payloads p
           ON p.disposition_identity=d.current_disposition_identity",
        "immutable_disposition_payload_update",
        "disposition exact semantic binding mismatch",
    );
}

#[test]
fn br192_delivered_rejects_self_hashed_accepted_receipt_column_rebinding() {
    let fixture = Fixture::new("DELIVERED_STRICT_RECEIPT");
    let append = MemoryAppendPort::default();
    let candidate = accepted_pending_fixture(&fixture, "DELIVERED_STRICT_RECEIPT", &append);
    assert_delivered_precommit_fault_rolls_back_then_exact_retry_delivers(
        &fixture,
        &candidate,
        &append,
        DeliveredPrecommitTestFault::AcceptedSinkResultReceiptBinding,
        "SELECT result_canonical FROM sink_results
         WHERE authoritative_for_state=1 AND result_kind='Accepted'",
        "SELECT result_sha256 FROM sink_results
         WHERE authoritative_for_state=1 AND result_kind='Accepted'",
        "immutable_sink_result_update",
        "receipt/column exact binding mismatch",
    );
}

#[test]
fn br192_delivered_rejects_self_hashed_task_transition_semantic_corruption() {
    let fixture = Fixture::new("DELIVERED_STRICT_TASK");
    let append = MemoryAppendPort::default();
    let candidate = accepted_pending_fixture(&fixture, "DELIVERED_STRICT_TASK", &append);
    assert_delivered_precommit_fault_rolls_back_then_exact_retry_delivers(
        &fixture,
        &candidate,
        &append,
        DeliveredPrecommitTestFault::TaskTransitionSemanticBinding,
        "SELECT transition_canonical FROM task_transition_payloads",
        "SELECT transition_sha256 FROM task_transition_payloads",
        "immutable_task_transition_update",
        "task transition exact semantic binding mismatch",
    );
}

#[test]
fn br192_manual_accepted_reason_and_authorization_ref_tampering_fail_closed() {
    for (label, column, replacement, expected_error) in [
        (
            "REASON",
            "reason",
            "TEST_CODE_TAMPERED_REASON",
            "manual accepted delivery audit exact semantic binding mismatch",
        ),
        (
            "AUTH_REF",
            "immutable_audit_ref",
            "TEST_CODE_TAMPERED_AUTHORIZATION_REF",
            "delivery audit exact semantic binding mismatch",
        ),
    ] {
        let fixture = Fixture::new(&format!("MANUAL_ACCEPTED_TAMPER_{label}"));
        let append = MemoryAppendPort::default();
        let candidate = manual_accepted_pending_fixture(
            &fixture,
            &format!("MANUAL_ACCEPTED_TAMPER_{label}"),
            &append,
        );
        let connection =
            Connection::open(&fixture.database_path).expect("open manual tamper fixture");
        connection
            .execute_batch("DROP TRIGGER immutable_manual_resolution_update")
            .expect("remove test-only manual-resolution immutability guard");
        let changed = connection
            .execute(
                &format!("UPDATE manual_resolutions SET {column}=?1"),
                [replacement],
            )
            .expect("inject manual accepted tampering");
        assert_eq!(changed, 1);
        drop(connection);

        let error = fixture
            .coordinator
            .reconcile_all_pending(&append, now())
            .expect_err("tampered manual acceptance must fail closed");
        assert!(
            error.to_string().contains(expected_error),
            "unexpected {label} tamper rejection: {error}"
        );
        assert_ne!(
            fixture
                .coordinator
                .decision_state(&candidate.decision_identity)
                .expect("manual accepted state after tampering"),
            DecisionState::Delivered
        );
        assert_eq!(
            fixture
                .query_i64("SELECT COUNT(*) FROM delivery_state_events WHERE to_state='Delivered'"),
            0
        );
    }
}

#[test]
fn br192_manual_accepted_authorization_ref_mismatch_fails_before_acceptance_append_and_retries() {
    let fixture = Fixture::new("MANUAL_ACCEPTED_AUTHORIZATION_REF_MISMATCH");
    let append = MemoryAppendPort::default();
    let candidate = manual_accepted_pending_fixture(
        &fixture,
        "MANUAL_ACCEPTED_AUTHORIZATION_REF_MISMATCH",
        &append,
    );
    let resolution_identity = fixture
        .query_strings("SELECT resolution_identity FROM manual_resolutions")
        .into_iter()
        .next()
        .expect("manual resolution identity");
    let mismatched_once = MismatchedAuthorizationRefOnce::new(&append);

    assert!(matches!(
        fixture
            .coordinator
            .reconcile_all_pending(&mismatched_once, now()),
        Err(DurableDeliveryError::ImmutableAppendConflict(identity))
            if identity == resolution_identity
    ));
    assert_eq!(
        append.count_kind("ManualResolutionAuthorization"),
        1,
        "the byte-identical authorization retry must stay idempotent in immutable storage"
    );
    assert_eq!(
        append.count_kind("DeliveryAcceptedAudit"),
        0,
        "a mismatched authorization reference must fail before acceptance-audit append"
    );
    assert_eq!(
        fixture.query_i64(
            "SELECT COUNT(*) FROM manual_resolutions
             WHERE accepted_audit_append_state='Pending' AND accepted_audit_ref IS NULL"
        ),
        1,
        "a mismatched authorization reference must not acknowledge the acceptance audit"
    );
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM delivery_state_events WHERE to_state='Delivered'"),
        0
    );
    assert_ne!(
        fixture
            .coordinator
            .decision_state(&candidate.decision_identity)
            .expect("state after authorization reference mismatch"),
        DecisionState::Delivered
    );

    reconcile_terminal(
        &fixture,
        &mismatched_once,
        DecisionState::Delivered,
        &candidate.decision_identity,
    );
    assert_eq!(
        append.count_kind("ManualResolutionAuthorization"),
        1,
        "the same append port must reuse the one exact authorization record"
    );
    assert_eq!(
        append.count_kind("DeliveryAcceptedAudit"),
        1,
        "the exact retry must append one acceptance audit"
    );
}

#[test]
fn br192_operation_postvalidation_faults_fail_closed_rollback_and_retry() {
    let cases = [
        (
            "OUTBOX_REF",
            OperationPostvalidationTestFault::ImmutableAuditOutboxRef,
            false,
        ),
        (
            "DISPOSITION_REF",
            OperationPostvalidationTestFault::DeliveryDispositionRef,
            false,
        ),
        (
            "TASK_TRANSITION_REF",
            OperationPostvalidationTestFault::TaskTransitionRef,
            false,
        ),
        (
            "MANUAL_RESOLUTION_REF",
            OperationPostvalidationTestFault::ManualResolutionRef,
            true,
        ),
        (
            "SINK_DELIVERY_AUDIT_REF",
            OperationPostvalidationTestFault::SinkDeliveryAuditRef,
            false,
        ),
        (
            "TASK_HYDRATION_STATE",
            OperationPostvalidationTestFault::TaskHydrationState,
            false,
        ),
    ];

    for (label, fault, manual_setup) in cases {
        let fixture = Fixture::new(&format!("POSTVALIDATION_{label}"));
        let append = MemoryAppendPort::default();
        if manual_setup {
            establish_manual_delivered_projection(
                &fixture,
                &format!("POSTVALIDATION_SETUP_{label}"),
                &append,
            );
        } else {
            establish_authoritative_delivered_projection(
                &fixture,
                &format!("POSTVALIDATION_SETUP_{label}"),
                &append,
                true,
            );
        }
        let probe_kind = if manual_setup {
            PushKind::T0Advice
        } else {
            PushKind::HoldingEvent
        };
        let probe = envelope(
            &format!("POSTVALIDATION_PROBE_{label}"),
            probe_kind,
            DeliverySubKind::None,
            "2026-07-30",
            false,
        );
        fixture
            .coordinator
            .install_operation_postvalidation_test_fault(fault)
            .expect("install one-shot operation postvalidation fault");

        assert!(matches!(
            fixture.coordinator.prepare(&probe, 1, now()),
            Err(DurableDeliveryError::InvalidConfiguration(reason))
                if reason.contains("persisted")
        ));
        assert!(matches!(
            fixture.coordinator.decision_state(&probe.decision_identity),
            Err(DurableDeliveryError::DecisionNotFound(_))
        ));
        let retry = fixture
            .coordinator
            .prepare(&probe, 1, now())
            .expect("same operation succeeds after one-shot fault rollback");
        assert_eq!(retry.state, DecisionState::Reserved);
    }
}

#[test]
fn br192_audit_ack_update_failure_before_commit_leaves_pending_ref_unchanged() {
    let fixture = Fixture::new("AUDIT_ACK_ROLLBACK");
    let candidate = envelope(
        "AUDIT_ACK_ROLLBACK",
        PushKind::ReviewProviderTopN,
        DeliverySubKind::None,
        "2026-07-30",
        true,
    );
    fixture
        .coordinator
        .prepare(&candidate, 1, now())
        .expect("prepare pending audit");
    let append = RollbackAcknowledgementAfterAppend::new(
        fixture_coordinator_arc(&fixture),
        "DecisionStateChanged",
    );

    assert!(matches!(
        fixture.coordinator.reconcile_all_pending(&append, now()),
        Err(DurableDeliveryError::InvalidConfiguration(reason))
            if reason == "TEST_CODE_ACK_AFTER_UPDATE_BEFORE_COMMIT"
    ));
    assert_eq!(
        fixture.query_i64(
            "SELECT COUNT(*) FROM immutable_audit_outbox
             WHERE audit_kind='DecisionStateChanged'
               AND append_state='Pending' AND immutable_audit_ref IS NULL"
        ),
        1,
        "audit acknowledgement UPDATE must roll back with its immutable ref"
    );
}

#[test]
fn br192_disposition_ack_update_failure_before_commit_leaves_pending_ref_unchanged() {
    let fixture = Fixture::new("DISPOSITION_ACK_ROLLBACK");
    let initial_append = MemoryAppendPort::default();
    accepted_pending_fixture(&fixture, "DISPOSITION_ACK_ROLLBACK", &initial_append);
    let append = RollbackAcknowledgementAfterAppend::new(
        fixture_coordinator_arc(&fixture),
        "DeliveryDisposition",
    );

    assert!(matches!(
        fixture.coordinator.reconcile_all_pending(&append, now()),
        Err(DurableDeliveryError::InvalidConfiguration(reason))
            if reason == "TEST_CODE_ACK_AFTER_UPDATE_BEFORE_COMMIT"
    ));
    assert_eq!(
        fixture.query_i64(
            "SELECT COUNT(*) FROM delivery_disposition_payloads
             WHERE append_state='Pending' AND immutable_audit_ref IS NULL"
        ),
        1,
        "disposition acknowledgement UPDATE must roll back with its immutable ref"
    );
}

#[test]
fn br192_delivery_audit_ack_update_failure_before_commit_leaves_ref_unchanged() {
    let fixture = Fixture::new("DELIVERY_AUDIT_ACK_ROLLBACK");
    let initial_append = MemoryAppendPort::default();
    accepted_pending_fixture(&fixture, "DELIVERY_AUDIT_ACK_ROLLBACK", &initial_append);
    let append = RollbackAcknowledgementAfterAppend::new(
        fixture_coordinator_arc(&fixture),
        "DeliveryAcceptedAudit",
    );

    assert!(matches!(
        fixture.coordinator.reconcile_all_pending(&append, now()),
        Err(DurableDeliveryError::InvalidConfiguration(reason))
            if reason == "TEST_CODE_ACK_AFTER_UPDATE_BEFORE_COMMIT"
    ));
    assert_eq!(
        fixture.query_i64(
            "SELECT COUNT(*) FROM sink_results
             WHERE result_kind='Accepted' AND delivery_audit_ref IS NULL"
        ),
        1,
        "accepted delivery audit acknowledgement UPDATE must roll back"
    );
}

#[test]
fn br192_task_transition_ack_update_failure_before_commit_leaves_pending_ref_unchanged() {
    let fixture = Fixture::new("TASK_TRANSITION_ACK_ROLLBACK");
    let initial_append = MemoryAppendPort::default();
    accepted_pending_fixture(&fixture, "TASK_TRANSITION_ACK_ROLLBACK", &initial_append);
    let append = RollbackAcknowledgementAfterAppend::new(
        fixture_coordinator_arc(&fixture),
        "BR-140TaskTransition",
    );

    assert!(matches!(
        fixture.coordinator.reconcile_all_pending(&append, now()),
        Err(DurableDeliveryError::InvalidConfiguration(reason))
            if reason == "TEST_CODE_ACK_AFTER_UPDATE_BEFORE_COMMIT"
    ));
    assert_eq!(
        fixture.query_i64(
            "SELECT COUNT(*) FROM task_transition_payloads
             WHERE append_state='Pending' AND immutable_audit_ref IS NULL"
        ),
        1,
        "task transition acknowledgement UPDATE must roll back with its immutable ref"
    );
}

#[test]
fn br192_dual_reconciler_ack_has_one_cas_winner_and_one_exactly_once_loser() {
    let fixture = Fixture::new("DUAL_RECONCILER_ACK_CAS");
    let candidate = envelope(
        "DUAL_RECONCILER_ACK_CAS",
        PushKind::ReviewProviderTopN,
        DeliverySubKind::None,
        "2026-07-30",
        true,
    );
    fixture
        .coordinator
        .prepare(&candidate, 1, now())
        .expect("prepare pending audit");
    let first = fixture_coordinator_arc(&fixture);
    let second = fixture.second_coordinator("DUAL_RECONCILER_ACK_CAS");
    let append = Arc::new(RacingAppendPort::new("DecisionStateChanged"));
    let start = Arc::new(Barrier::new(3));

    let handles = [first, second].map(|coordinator| {
        let append = append.clone();
        let start = start.clone();
        std::thread::spawn(move || {
            start.wait();
            coordinator.reconcile_all_pending(append.as_ref(), now())
        })
    });
    start.wait();
    let outcomes = handles
        .into_iter()
        .map(|handle| handle.join().expect("reconciler thread"))
        .collect::<Vec<_>>();
    assert_eq!(
        outcomes.iter().filter(|outcome| outcome.is_ok()).count(),
        1,
        "exactly one reconciler must win the acknowledgement CAS"
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| {
                matches!(
                    outcome,
                    Err(DurableDeliveryError::PolicyMismatch(reason))
                        if reason.contains("compare-and-set affected 0 rows")
                )
            })
            .count(),
        1,
        "the losing reconciler must fail explicitly on a zero-row CAS"
    );
    let raced_identities = append
        .target_identities
        .lock()
        .expect("race identities")
        .clone();
    assert_eq!(raced_identities.len(), 2);
    assert_eq!(
        raced_identities[0], raced_identities[1],
        "both reconcilers must race the same immutable acknowledgement"
    );
    assert!(
        append
            .inner
            .records
            .lock()
            .expect("append records")
            .contains_key(&raced_identities[0]),
        "external immutable append remains exactly one record by identity"
    );
}

fn reconcile_terminal(
    fixture: &Fixture,
    append: &dyn ImmutableAppendPort,
    expected: DecisionState,
    decision_identity: &str,
) {
    let summary = fixture
        .coordinator
        .reconcile_all_pending(append, now())
        .expect("reconcile");
    assert_eq!(summary.provider_calls, 0);
    assert_eq!(summary.sink_calls, 0);
    assert_eq!(
        fixture
            .coordinator
            .decision_state(decision_identity)
            .expect("state"),
        expected
    );
}

#[test]
fn policy_catalog_has_twenty_seven_kinds_and_thirty_rows() {
    // 2026-08-07: I-09 SectorTop / I-09A SectorAnomaly 升级 counted,
    // policy catalog 15 kind/18 row → 17 kind/20 row。
    // 2026-08-12: R-03/R-11/R-12/R-13/A-10 复盘 dispatcher 升级 counted
    // (重启错过补偿重复推送修复) → 17 kind/20 row → 22 kind/25 row。
    // 2026-08-18: BR-241 P-01 durable owner → 23 kind/26 row。
    // 2026-09-19: T-16 ST 涨跌幅变更提醒升级 counted → 24 kind/27 row。
    // 2026-09-20: A-12 归因日推升级 counted → 25 kind/28 row。
    // 2026-09-20: G5b 深链归因升级 counted → 26 kind/29 row。
    // 2026-09-20: I-01 盘中轮动升级 counted → 27 kind/30 row。
    let fixture = Fixture::new("CATALOG");
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM delivery_policy_catalog"),
        30
    );
    assert_eq!(
        fixture.query_i64("SELECT COUNT(DISTINCT push_kind) FROM delivery_policy_catalog"),
        27
    );
    assert_eq!(compiled_policy_catalog().len(), 30);
}

#[test]
fn p01_schema_v7_to_v9_replays_only_policy_catalog_and_preserves_delivery_authority() {
    let mut fixture = Fixture::new("P01_SCHEMA_V7_TO_V9_POLICY_ONLY");
    let append = MemoryAppendPort::default();
    let candidate = envelope(
        "P01_SCHEMA_V7_TO_V9_POLICY_ONLY",
        PushKind::PreopenNewsHot,
        DeliverySubKind::None,
        "2026-08-18",
        false,
    );
    prepare_reserved(&fixture, &candidate, &append);
    let sink = StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(now())));
    let sinks: Vec<AuthoritativeSink> = vec![sink];
    fixture
        .coordinator
        .resume_deliverable(&candidate.decision_identity, &sinks, now())
        .expect("deliver TEST_CODE P-01 before migration");
    reconcile_terminal(
        &fixture,
        &append,
        DecisionState::Delivered,
        &candidate.decision_identity,
    );
    drop(fixture.coordinator.take());

    let mut connection =
        Connection::open(&fixture.database_path).expect("open TEST_CODE P-01 migration database");
    let authority_tables = [
        "delivery_decisions",
        "delivery_attempts",
        "business_date_once_claims",
        "sink_results",
        "cooldown_heads",
        "immutable_audit_outbox",
    ];
    let count = |connection: &Connection, table: &str| {
        connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap_or_else(|error| panic!("count {table}: {error}"))
    };
    let before = authority_tables
        .iter()
        .map(|table| ((*table).to_owned(), count(&connection, table)))
        .collect::<BTreeMap<_, _>>();

    connection
        .execute(
            "DELETE FROM delivery_policy_catalog WHERE push_kind='PreopenNewsHot'",
            [],
        )
        .expect("restore schema-v7 policy set");
    connection
        .execute("UPDATE delivery_policy_catalog SET policy_version=3", [])
        .expect("restore schema-v7 policy version");
    connection
        .pragma_update(None, "user_version", 7_i64)
        .expect("restore schema-v7 marker");

    initialize_test_schema(&mut connection).expect("migrate schema v7 to v9");

    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .expect("read migrated schema version"),
        9
    );
    assert_eq!(
        count(
            &connection,
            "delivery_policy_catalog WHERE push_kind='PreopenNewsHot'"
        ),
        1
    );
    let after = authority_tables
        .iter()
        .map(|table| ((*table).to_owned(), count(&connection, table)))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        after, before,
        "policy migration must preserve durable authority"
    );
}

#[test]
fn br245_schema_v9_replays_only_policy_catalog_and_preserves_all_authority_rows() {
    let mut fixture = Fixture::new("BR245_SCHEMA_V9_POLICY_ONLY");
    let append = MemoryAppendPort::default();
    let accepted_at = Utc
        .with_ymd_and_hms(2026, 8, 17, 15, 20, 35)
        .single()
        .expect("valid R-07 accepted timestamp")
        + chrono::Duration::milliseconds(788);
    let candidate = envelope(
        "BR245_SCHEMA_V9_POLICY_ONLY",
        PushKind::TomorrowWatch,
        DeliverySubKind::None,
        "2026-08-17",
        true,
    );
    let prepared = fixture
        .coordinator
        .prepare(&candidate, 1, accepted_at)
        .expect("prepare TEST_CODE R-07 before migration");
    assert_eq!(prepared.state, DecisionState::Reserved);
    fixture
        .coordinator
        .reconcile_all_pending(&append, accepted_at)
        .expect("append TEST_CODE R-07 reservation audits");
    let sink = StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(accepted_at)));
    let sinks: Vec<AuthoritativeSink> = vec![sink];
    fixture
        .coordinator
        .resume_deliverable(&candidate.decision_identity, &sinks, accepted_at)
        .expect("deliver TEST_CODE R-07 before migration");
    fixture
        .coordinator
        .reconcile_all_pending(&append, accepted_at)
        .expect("finalize TEST_CODE R-07 before migration");
    assert_eq!(
        fixture
            .coordinator
            .decision_state(&candidate.decision_identity)
            .expect("R-07 state before migration"),
        DecisionState::Delivered
    );
    drop(fixture.coordinator.take());

    let mut connection =
        Connection::open(&fixture.database_path).expect("open TEST_CODE BR-245 migration database");
    let authority_tables = [
        "delivery_decisions",
        "immutable_audit_outbox",
        "cooldown_reservations",
        "cooldown_heads",
        "business_date_once_claims",
        "daily_budget_reservations",
        "delivery_attempts",
        "sink_results",
        "review_terminal_replay_attempts",
        "review_terminal_replay_completions",
        "manual_resolutions",
        "delivery_disposition_payloads",
        "task_transition_payloads",
        "delivery_state_events",
        "delivery_attempt_events",
        "cooldown_reservation_events",
        "daily_budget_reservation_events",
    ];
    let before = authority_snapshot(&connection, &authority_tables);

    connection
        .execute("UPDATE delivery_policy_catalog SET policy_version=4", [])
        .expect("restore schema-v8 policy version");
    connection
        .execute(
            "UPDATE delivery_policy_catalog
             SET window_mode='Rolling', counts_against_daily_budget=1
             WHERE push_kind='TomorrowWatch' AND sub_kind='NONE'",
            [],
        )
        .expect("restore schema-v8 TomorrowWatch policy");
    connection
        .pragma_update(None, "user_version", 8_i64)
        .expect("restore schema-v8 marker");

    initialize_test_schema(&mut connection).expect("migrate schema v8 to v9");

    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .expect("read migrated schema version"),
        9
    );
    let migrated_policy = connection
        .query_row(
            "SELECT cooldown_scope,window_mode,base_cooldown_secs,
                    counts_against_daily_budget,policy_version
             FROM delivery_policy_catalog
             WHERE push_kind='TomorrowWatch' AND sub_kind='NONE'",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            },
        )
        .expect("read migrated TomorrowWatch policy");
    assert_eq!(
        migrated_policy,
        (
            "Global".to_owned(),
            "BusinessDateOnce".to_owned(),
            86_400,
            0,
            5
        )
    );
    assert_eq!(
        authority_snapshot(&connection, &authority_tables),
        before,
        "schema-v9 policy replay must preserve every authority-table value"
    );
}

#[test]
fn br192_wal_materialization_occurs_once_before_operational_binding() {
    let before = super::schema::wal_materialization_call_count_for_test();
    let fixture = Fixture::new("WAL_MATERIALIZATION_ONCE");
    let after_open = super::schema::wal_materialization_call_count_for_test();
    assert_eq!(
        after_open,
        before + 1,
        "coordinator bootstrap must materialize WAL exactly once"
    );

    let append = MemoryAppendPort::default();
    let candidate = envelope(
        "WAL_MATERIALIZATION_ONCE",
        PushKind::ReviewProviderTopN,
        DeliverySubKind::None,
        "2026-07-30",
        false,
    );
    prepare_reserved(&fixture, &candidate, &append);
    fixture
        .coordinator
        .inspect_pending_for_date("2026-07-30")
        .expect("operational read validates the bound WAL configuration");
    assert_eq!(
        super::schema::wal_materialization_call_count_for_test(),
        after_open,
        "post-binding configuration and operations must never rematerialize WAL"
    );
}

#[test]
fn br192_post_binding_marker_loss_fails_without_runtime_reattestation() {
    let before = super::coordinator::main_reattestation_call_count_for_test();
    let fixture = Fixture::new("POST_BINDING_MARKER_LOSS");
    let after_open = super::coordinator::main_reattestation_call_count_for_test();
    assert_eq!(
        after_open,
        before + 1,
        "bootstrap must consume exactly one main re-attestation"
    );
    let wal_calls = super::schema::wal_materialization_call_count_for_test();

    fixture
        .coordinator
        .remove_bound_main_ofd_marker_for_test()
        .expect("remove only the TEST_CODE-bound main marker");
    for _ in 0..2 {
        assert!(matches!(
            fixture
                .coordinator
                .inspect_pending_for_date("2026-07-30"),
            Err(DurableDeliveryError::IsolationViolation(reason))
                if reason.contains("lost owner-specific OFD marker")
        ));
    }
    assert_eq!(
        super::coordinator::main_reattestation_call_count_for_test(),
        after_open,
        "operational marker loss must not invoke bootstrap re-attestation"
    );
    assert_eq!(
        super::schema::wal_materialization_call_count_for_test(),
        wal_calls,
        "operational marker loss must not rematerialize WAL"
    );
}

#[test]
fn daily_report_subkind_overrides_are_transactional() {
    let fixture = Fixture::new("DAILY_REPORT");
    assert_eq!(
        fixture.query_i64(
            "SELECT COALESCE(override_cooldown_secs,-1)
             FROM delivery_policy_catalog
             WHERE push_kind='DailyReport' AND sub_kind='FactorIC'"
        ),
        -1
    );
    assert_eq!(
        fixture.query_i64(
            "SELECT override_cooldown_secs FROM delivery_policy_catalog
             WHERE push_kind='DailyReport' AND sub_kind='SectorTier'"
        ),
        1_800
    );
    assert_eq!(
        fixture.query_i64(
            "SELECT override_cooldown_secs FROM delivery_policy_catalog
             WHERE push_kind='DailyReport' AND sub_kind='CapitalVerify'"
        ),
        1_800
    );
}

#[test]
fn prepare_binding_cooldown_and_budget_are_one_transaction() {
    let fixture = Fixture::new("ATOMIC_PREPARE");
    let append = MemoryAppendPort::default();
    let envelope = envelope(
        "ATOMIC_PREPARE",
        // BR-237: ReviewProviderTopN 已豁免日预算, budget 原子性改用
        // SectorTop (BusinessDateOnce + counts_against_daily_budget=true)。
        PushKind::SectorTop,
        DeliverySubKind::None,
        "2026-07-30",
        true,
    );
    prepare_reserved(&fixture, &envelope, &append);
    assert_eq!(
        fixture.query_i64(
            "SELECT COUNT(*) FROM delivery_decisions
             WHERE state='Reserved' AND reservation_generation=1
               AND current_budget_reservation_identity IS NOT NULL
               AND current_cooldown_reservation_identity IS NOT NULL"
        ),
        1
    );
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM business_date_once_claims"),
        1
    );
}

#[test]
fn br237_review_kinds_exempt_from_daily_budget_signal_kinds_compete() {
    // catalog 语义: 11 个复盘类 (BusinessDateOnce 每日必达) 豁免日预算,
    // 盘中信号类继续竞争 30 槽 (8/13 复盘被做T T0Advice 烧满预算饿死事故)。
    let catalog = compiled_policy_catalog();
    for kind in [
        PushKind::ReviewMarket,
        PushKind::ReviewLhb,
        PushKind::ReviewSignal,
        PushKind::ReviewFailure,
        PushKind::TomorrowWatch,
        PushKind::ReviewProviderTopN,
        PushKind::IndustryChain,
        PushKind::PositionReview,
        PushKind::ReviewBacktest,
        PushKind::WatchlistTracking,
        PushKind::CatalystReview,
    ] {
        let row = catalog
            .iter()
            .find(|row| row.push_kind == kind)
            .expect("policy");
        assert!(
            !row.counts_against_daily_budget,
            "BR-237: {kind} 复盘类必须豁免日预算"
        );
    }
    for kind in [
        PushKind::T0Advice,
        PushKind::HoldingPlan,
        PushKind::SectorTop,
        PushKind::SectorAnomaly,
        PushKind::CloseCall,
        PushKind::StPriceLimitChanged,
    ] {
        let row = catalog
            .iter()
            .find(|row| row.push_kind == kind)
            .expect("policy");
        assert!(
            row.counts_against_daily_budget,
            "BR-237: {kind} 信号类必须计入日预算"
        );
    }

    // 行为: 30 槽填满后, 信号类第 31 个被 DailyBudgetFull 拒, 复盘类仍可投递且不占槽
    let fixture = Fixture::new("BR237_BUDGET_EXEMPT");
    let append = MemoryAppendPort::default();
    for slot in 0..DAILY_BUDGET_LIMIT {
        let signal = envelope(
            &format!("BR237_SIGNAL_{slot}"),
            PushKind::HoldingPlan,
            DeliverySubKind::None,
            "2026-07-30",
            false,
        );
        prepare_reserved(&fixture, &signal, &append);
    }
    assert_eq!(
        fixture.query_i64(
            "SELECT COUNT(*) FROM daily_budget_reservations
             WHERE state='Reserved'"
        ),
        DAILY_BUDGET_LIMIT,
        "30 槽应全部被信号类占用"
    );

    // 信号类第 31 个 → 预算满拒绝 (DailyBudgetFull → Rejected 终态链)
    let overflow = envelope(
        "BR237_SIGNAL_OVERFLOW",
        PushKind::HoldingPlan,
        DeliverySubKind::None,
        "2026-07-30",
        false,
    );
    let outcome = fixture
        .coordinator
        .prepare(&overflow, 1, now())
        .expect("prepare overflow");
    assert!(
        matches!(
            outcome.state,
            DecisionState::RejectedAuditPending
                | DecisionState::RejectedTaskTransitionPending
                | DecisionState::RejectedDurable
        ),
        "BR-237: 预算满时信号类必须被拒绝, got {:?}",
        outcome.state
    );
    fixture
        .coordinator
        .reconcile_all_pending(&append, now())
        .expect("reconcile overflow");

    // BR-245 R-07 复盘类 (豁免) 预算满时仍成功 Reserved → Delivered, 且不占新槽
    let review = envelope(
        "BR245_R07_BUDGET_EXEMPT",
        PushKind::TomorrowWatch,
        DeliverySubKind::None,
        "2026-07-30",
        true,
    );
    prepare_reserved(&fixture, &review, &append);
    let sink = StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(now())));
    let sinks: Vec<AuthoritativeSink> = vec![sink];
    fixture
        .coordinator
        .resume_deliverable(&review.decision_identity, &sinks, now())
        .expect("review deliver");
    reconcile_terminal(
        &fixture,
        &append,
        DecisionState::Delivered,
        &review.decision_identity,
    );
    assert_eq!(
        fixture.query_i64(
            "SELECT COUNT(*) FROM daily_budget_reservations
             WHERE state='Reserved'"
        ),
        DAILY_BUDGET_LIMIT,
        "BR-237: 豁免类投递不得占用预算槽"
    );
    assert_eq!(
        fixture.query_i64(&format!(
            "SELECT COUNT(*) FROM delivery_decisions
             WHERE decision_identity='{}' AND state='Delivered'",
            review.decision_identity
        )),
        1
    );
}

#[test]
fn thirty_slots_are_a_cross_process_hard_limit() {
    let fixture = Fixture::new("THIRTY");
    for index in 0..31 {
        let current = envelope(
            &format!("THIRTY_{index}"),
            PushKind::HoldingEvent,
            DeliverySubKind::None,
            "2026-07-30",
            false,
        );
        fixture
            .coordinator
            .prepare(&current, 1, now())
            .expect("durable admission result");
    }
    assert_eq!(
        fixture.query_i64(
            "SELECT COUNT(*) FROM daily_budget_reservations
             WHERE state IN ('Reserved','Accepted','Uncertain')"
        ),
        30
    );
    assert_eq!(
        fixture.query_i64(
            "SELECT COUNT(*) FROM delivery_decisions
             WHERE state='RejectedAuditPending' AND reservation_generation=0"
        ),
        1
    );
}

#[test]
fn console_observer_cannot_acknowledge_delivery() {
    let fixture = Fixture::new("CONSOLE");
    let envelope = envelope(
        "CONSOLE",
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        "2026-07-30",
        false,
    );
    let outcome = fixture
        .coordinator
        .prepare(&envelope, 0, now())
        .expect("durable denial");
    assert_eq!(outcome.state, DecisionState::RejectedAuditPending);
    assert_eq!(outcome.sink_calls, 0);
    assert_eq!(outcome.reservation_generation, 0);
}

#[test]
fn typed_sink_transport_failure_is_uncertain() {
    let fixture = Fixture::new("UNCERTAIN");
    let append = MemoryAppendPort::default();
    let envelope = envelope(
        "UNCERTAIN",
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        "2026-07-30",
        false,
    );
    prepare_reserved(&fixture, &envelope, &append);
    let sink = StaticSink::new(AuthoritativeSinkResult::Uncertain(uncertainty(now())));
    let sinks: Vec<AuthoritativeSink> = vec![sink.clone()];
    let outcome = fixture
        .coordinator
        .resume_deliverable(&envelope.decision_identity, &sinks, now())
        .expect("resume");
    assert_eq!(outcome.sink_calls, 1);
    assert_eq!(outcome.state, DecisionState::UncertainAuditPending);
    reconcile_terminal(
        &fixture,
        &append,
        DecisionState::UncertainManualReview,
        &envelope.decision_identity,
    );
    assert_eq!(sink.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn generic_disposition_is_required_and_task_transition_is_optional() {
    let fixture = Fixture::new("GENERIC");
    let append = MemoryAppendPort::default();
    let non_task = envelope(
        "GENERIC_NON_TASK",
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        "2026-07-30",
        false,
    );
    prepare_reserved(&fixture, &non_task, &append);
    let sink = StaticSink::new(AuthoritativeSinkResult::Rejected(rejection(now(), false)));
    let sinks: Vec<AuthoritativeSink> = vec![sink];
    fixture
        .coordinator
        .resume_deliverable(&non_task.decision_identity, &sinks, now())
        .expect("resume");
    reconcile_terminal(
        &fixture,
        &append,
        DecisionState::RejectedDurable,
        &non_task.decision_identity,
    );
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM delivery_disposition_payloads"),
        1
    );
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM task_transition_payloads"),
        0
    );
}

#[test]
fn pre_sink_denial_is_atomic_durable_and_hydratable() {
    let fixture = Fixture::new("DENIAL");
    let append = MemoryAppendPort::default();
    let envelope = envelope(
        "DENIAL",
        PushKind::ReviewProviderTopN,
        DeliverySubKind::None,
        "2026-07-30",
        true,
    );
    let first = fixture
        .coordinator
        .prepare(&envelope, 0, now())
        .expect("durable denial");
    let second = fixture
        .coordinator
        .prepare(&envelope, 0, now())
        .expect("idempotent denial replay");
    assert_eq!(first, second);
    assert_eq!(first.reservation_generation, 0);
    assert!(first.budget_reservation_identity.is_none());
    reconcile_terminal(
        &fixture,
        &append,
        DecisionState::RejectedDurable,
        &envelope.decision_identity,
    );
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM delivery_disposition_payloads"),
        1
    );
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM task_transition_payloads"),
        1
    );
}

#[test]
fn decision_dedup_requires_identical_canonical_bytes() {
    let fixture = Fixture::new("DEDUP");
    let envelope = envelope(
        "DEDUP",
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        "2026-07-30",
        false,
    );
    fixture
        .coordinator
        .prepare(&envelope, 1, now())
        .expect("first prepare");
    fixture
        .coordinator
        .prepare(&envelope, 1, now())
        .expect("byte-identical replay");
    let mut conflicting = envelope.clone();
    conflicting.replace_content_preserving_identity(b"TEST_CODE_CONFLICTING_BODY".to_vec());
    let error = fixture
        .coordinator
        .prepare(&conflicting, 1, now())
        .expect_err("same identity/different bytes must conflict");
    assert!(matches!(
        error,
        DurableDeliveryError::DecisionIdentityConflict { .. }
    ));
    assert_eq!(
        fixture.query_i64(
            "SELECT COUNT(*) FROM immutable_audit_outbox
             WHERE audit_kind='DecisionIdentityConflict'"
        ),
        1
    );
}

#[test]
fn source_binding_is_frozen_and_participates_in_replay_conflict_detection() {
    let fixture = Fixture::new("SOURCE_BINDING");
    let envelope = envelope(
        "SOURCE_BINDING",
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        "2026-07-30",
        false,
    )
    .with_provider_evidence(
        Some("2026-07-30T08:00:00Z".to_owned()),
        Some("2026-07-30".to_owned()),
        vec![
            "TEST_CODE_BATCH_VOLUME_RATIO".to_owned(),
            "TEST_CODE_BATCH_MAIN_NET_INFLOW".to_owned(),
        ],
    )
    .expect("complete provider evidence");
    fixture
        .coordinator
        .prepare(&envelope, 1, now())
        .expect("first prepare");
    fixture
        .coordinator
        .prepare(&envelope, 1, now())
        .expect("byte-identical source binding replay");

    let mut conflicting = envelope.clone();
    conflicting.replace_source_binding_preserving_identity(
        b"TEST_CODE_DIFFERENT_ORDERED_PROVIDER_PROJECTION".to_vec(),
    );
    let error = fixture
        .coordinator
        .prepare(&conflicting, 1, now())
        .expect_err("same identity with different frozen source binding must conflict");
    assert!(matches!(
        error,
        DurableDeliveryError::DecisionIdentityConflict { .. }
    ));

    let stored: DeliveryEnvelope = serde_json::from_slice(
        &fixture.query_blob("SELECT envelope_canonical FROM delivery_decisions"),
    )
    .expect("stored canonical envelope");
    assert_eq!(
        stored.source_binding_canonical,
        b"TEST_CODE_SOURCE_BINDING_SOURCE_BINDING"
    );
}

#[test]
fn one_active_budget_per_decision_and_slot() {
    let fixture = Fixture::new("UNIQUE_SLOT");
    let envelope = envelope(
        "UNIQUE_SLOT",
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        "2026-07-30",
        false,
    );
    fixture
        .coordinator
        .prepare(&envelope, 1, now())
        .expect("prepare");
    assert_eq!(
        fixture.query_i64(
            "SELECT COUNT(*) FROM daily_budget_reservations
             WHERE decision_identity IN (
               SELECT decision_identity FROM delivery_decisions
             ) AND state IN ('Reserved','Accepted','Uncertain')"
        ),
        1
    );
    assert_eq!(
        fixture.query_i64(
            "SELECT COUNT(*) FROM (
               SELECT business_date,slot_no,COUNT(*) c
               FROM daily_budget_reservations
               WHERE state IN ('Reserved','Accepted','Uncertain')
               GROUP BY business_date,slot_no HAVING c>1
             )"
        ),
        0
    );
}

#[test]
fn all_r09_dispositions_freeze_generic_and_br140_payloads() {
    let fixture = Fixture::new("R09_PAYLOADS");
    let append = MemoryAppendPort::default();
    let envelope = envelope(
        "R09_PAYLOADS",
        PushKind::ReviewProviderTopN,
        DeliverySubKind::None,
        "2026-07-30",
        true,
    );
    prepare_reserved(&fixture, &envelope, &append);
    let sink = StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(now())));
    let sinks: Vec<AuthoritativeSink> = vec![sink];
    fixture
        .coordinator
        .resume_deliverable(&envelope.decision_identity, &sinks, now())
        .expect("resume");
    reconcile_terminal(
        &fixture,
        &append,
        DecisionState::Delivered,
        &envelope.decision_identity,
    );
    assert_eq!(
        fixture.query_i64(
            "SELECT COUNT(*) FROM delivery_disposition_payloads
             WHERE disposition='Accepted' AND append_state='Appended'"
        ),
        1
    );
    assert_eq!(
        fixture.query_i64(
            "SELECT COUNT(*) FROM task_transition_payloads
             WHERE append_state='Appended'"
        ),
        1
    );
}

#[test]
fn manual_resolution_requires_operator_and_evidence() {
    let fixture = Fixture::new("MANUAL_REQUIRED");
    let append = MemoryAppendPort::default();
    let envelope = envelope(
        "MANUAL_REQUIRED",
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        "2026-07-30",
        false,
    );
    prepare_reserved(&fixture, &envelope, &append);
    let sink = StaticSink::new(AuthoritativeSinkResult::Uncertain(uncertainty(now())));
    let sinks: Vec<AuthoritativeSink> = vec![sink];
    fixture
        .coordinator
        .resume_deliverable(&envelope.decision_identity, &sinks, now())
        .expect("resume");
    reconcile_terminal(
        &fixture,
        &append,
        DecisionState::UncertainManualReview,
        &envelope.decision_identity,
    );
    let invalid = ManualResolutionCommand {
        decision_identity: envelope.decision_identity,
        disposition: ManualDisposition::Rejected,
        operator_identity: String::new(),
        reason: "TEST_CODE_REASON".to_owned(),
        external_evidence: Vec::new(),
        resolved_at: now(),
    };
    assert!(matches!(
        fixture.coordinator.resolve_uncertain(&invalid, &append),
        Err(DurableDeliveryError::InvalidManualResolution(_))
    ));
}

#[test]
fn manual_resolution_missing_decision_does_not_append_or_mutate_state() {
    let fixture = Fixture::new("MANUAL_MISSING_PRECHECK");
    let append = MemoryAppendPort::default();
    let before = (
        fixture.query_i64("SELECT COUNT(*) FROM delivery_decisions"),
        fixture.query_i64("SELECT COUNT(*) FROM delivery_state_events"),
        fixture.query_i64("SELECT COUNT(*) FROM daily_budget_reservations"),
        fixture.query_i64("SELECT COUNT(*) FROM cooldown_reservations"),
        fixture.query_i64("SELECT COUNT(*) FROM manual_resolutions"),
    );
    let command = ManualResolutionCommand {
        decision_identity: "TEST_CODE_BR192_MISSING_DECISION_0123456789".to_owned(),
        disposition: ManualDisposition::Rejected,
        operator_identity: "TEST_CODE_OPERATOR_0123456789".to_owned(),
        reason: "TEST_CODE_VERIFIED_REJECTION".to_owned(),
        external_evidence: b"TEST_CODE_MANUAL_REJECTION_EVIDENCE".to_vec(),
        resolved_at: now(),
    };

    assert!(matches!(
        fixture.coordinator.resolve_uncertain(&command, &append),
        Err(DurableDeliveryError::DecisionNotFound(identity))
            if identity == command.decision_identity
    ));
    assert_eq!(
        append.count_kind("ManualResolutionAuthorization"),
        0,
        "an unknown decision must not leave an immutable authorization"
    );
    assert_eq!(
        (
            fixture.query_i64("SELECT COUNT(*) FROM delivery_decisions"),
            fixture.query_i64("SELECT COUNT(*) FROM delivery_state_events"),
            fixture.query_i64("SELECT COUNT(*) FROM daily_budget_reservations"),
            fixture.query_i64("SELECT COUNT(*) FROM cooldown_reservations"),
            fixture.query_i64("SELECT COUNT(*) FROM manual_resolutions"),
        ),
        before,
        "an unknown decision must not change state or reservations"
    );
}

#[test]
fn manual_resolution_wrong_state_does_not_append_or_mutate_reservation() {
    let fixture = Fixture::new("MANUAL_WRONG_STATE_PRECHECK");
    let append = MemoryAppendPort::default();
    let envelope = envelope(
        "MANUAL_WRONG_STATE_PRECHECK",
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        "2026-07-30",
        false,
    );
    prepare_reserved(&fixture, &envelope, &append);
    let before = (
        fixture.query_i64("SELECT COUNT(*) FROM delivery_state_events"),
        fixture.query_i64("SELECT COUNT(*) FROM daily_budget_reservations WHERE state='Reserved'"),
        fixture.query_i64("SELECT COUNT(*) FROM cooldown_reservations WHERE state='Reserved'"),
        fixture.query_i64("SELECT COUNT(*) FROM manual_resolutions"),
    );
    let command = ManualResolutionCommand {
        decision_identity: envelope.decision_identity.clone(),
        disposition: ManualDisposition::Rejected,
        operator_identity: "TEST_CODE_OPERATOR_0123456789".to_owned(),
        reason: "TEST_CODE_VERIFIED_REJECTION".to_owned(),
        external_evidence: b"TEST_CODE_MANUAL_REJECTION_EVIDENCE".to_vec(),
        resolved_at: now(),
    };

    assert!(matches!(
        fixture.coordinator.resolve_uncertain(&command, &append),
        Err(DurableDeliveryError::InvalidManualResolution(reason))
            if reason.contains("expected UncertainManualReview")
    ));
    assert_eq!(
        append.count_kind("ManualResolutionAuthorization"),
        0,
        "an ineligible state must not leave an immutable authorization"
    );
    assert_eq!(
        fixture
            .coordinator
            .decision_state(&envelope.decision_identity)
            .expect("state"),
        DecisionState::Reserved
    );
    assert_eq!(
        (
            fixture.query_i64("SELECT COUNT(*) FROM delivery_state_events"),
            fixture.query_i64(
                "SELECT COUNT(*) FROM daily_budget_reservations WHERE state='Reserved'",
            ),
            fixture.query_i64(
                "SELECT COUNT(*) FROM cooldown_reservations WHERE state='Reserved'",
            ),
            fixture.query_i64("SELECT COUNT(*) FROM manual_resolutions"),
        ),
        before,
        "an ineligible state must not change state or reservations"
    );
}

#[test]
fn manual_accepted_cas_enters_accepted_audit_pending() {
    let fixture = Fixture::new("MANUAL_ACCEPT");
    let append = MemoryAppendPort::default();
    let envelope = envelope(
        "MANUAL_ACCEPT",
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        "2026-07-30",
        false,
    );
    prepare_reserved(&fixture, &envelope, &append);
    let sink = StaticSink::new(AuthoritativeSinkResult::Uncertain(uncertainty(now())));
    let sinks: Vec<AuthoritativeSink> = vec![sink];
    fixture
        .coordinator
        .resume_deliverable(&envelope.decision_identity, &sinks, now())
        .expect("resume");
    reconcile_terminal(
        &fixture,
        &append,
        DecisionState::UncertainManualReview,
        &envelope.decision_identity,
    );
    let state = fixture
        .coordinator
        .resolve_uncertain(
            &ManualResolutionCommand {
                decision_identity: envelope.decision_identity.clone(),
                disposition: ManualDisposition::Accepted {
                    receipt: Some(receipt(now())),
                },
                operator_identity: "TEST_CODE_OPERATOR_0123456789".to_owned(),
                reason: "TEST_CODE_VERIFIED_ACCEPTANCE".to_owned(),
                external_evidence: b"TEST_CODE_EXTERNAL_ACCEPTANCE_EVIDENCE".to_vec(),
                resolved_at: now(),
            },
            &append,
        )
        .expect("manual accepted");
    assert_eq!(state, DecisionState::AcceptedAuditPending);
    reconcile_terminal(
        &fixture,
        &append,
        DecisionState::Delivered,
        &envelope.decision_identity,
    );
    fixture
        .coordinator
        .verify_manual_accepted_delivery(&envelope.decision_identity)
        .expect("Delivered manual acceptance has queryable verified audit evidence");
    assert_eq!(
        fixture.query_i64(
            "SELECT COUNT(*) FROM manual_resolutions
             WHERE disposition='Accepted'
               AND accepted_audit_identity IS NOT NULL
               AND frozen_delivery_audit_sha256 IS NOT NULL
               AND accepted_audit_append_state='Appended'
               AND accepted_audit_ref IS NOT NULL"
        ),
        1
    );
}

#[test]
fn br192_manual_accepted_audit_ack_failure_rolls_back_and_exact_retry_reaches_delivered() {
    let fixture = Fixture::new("MANUAL_ACCEPT_ACK_RETRY");
    let initial_append = MemoryAppendPort::default();
    let candidate = envelope(
        "MANUAL_ACCEPT_ACK_RETRY",
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        "2026-07-30",
        false,
    );
    prepare_reserved(&fixture, &candidate, &initial_append);
    let sink = StaticSink::new(AuthoritativeSinkResult::Uncertain(uncertainty(now())));
    let sinks: Vec<AuthoritativeSink> = vec![sink];
    fixture
        .coordinator
        .resume_deliverable(&candidate.decision_identity, &sinks, now())
        .expect("persist uncertain result");
    reconcile_terminal(
        &fixture,
        &initial_append,
        DecisionState::UncertainManualReview,
        &candidate.decision_identity,
    );
    fixture
        .coordinator
        .resolve_uncertain(
            &ManualResolutionCommand {
                decision_identity: candidate.decision_identity.clone(),
                disposition: ManualDisposition::Accepted {
                    receipt: Some(receipt(now())),
                },
                operator_identity: "TEST_CODE_OPERATOR_ACK_RETRY_0123456789".to_owned(),
                reason: "TEST_CODE_VERIFIED_ACCEPTANCE_ACK_RETRY".to_owned(),
                external_evidence: b"TEST_CODE_MANUAL_ACCEPT_ACK_RETRY_EVIDENCE".to_vec(),
                resolved_at: now(),
            },
            &initial_append,
        )
        .expect("persist manual acceptance as audit pending");
    let rollback = RollbackAcknowledgementAfterAppend::new(
        fixture_coordinator_arc(&fixture),
        "DeliveryAcceptedAudit",
    );

    assert!(matches!(
        fixture
            .coordinator
            .reconcile_all_pending(&rollback, now()),
        Err(DurableDeliveryError::InvalidConfiguration(reason))
            if reason == "TEST_CODE_ACK_AFTER_UPDATE_BEFORE_COMMIT"
    ));
    assert_eq!(
        fixture
            .coordinator
            .decision_state(&candidate.decision_identity)
            .expect("manual accepted state after ack rollback"),
        DecisionState::AcceptedAuditPending
    );
    assert_eq!(
        fixture.query_i64(
            "SELECT COUNT(*) FROM manual_resolutions
             WHERE disposition='Accepted'
               AND accepted_audit_identity IS NOT NULL
               AND accepted_audit_append_state='Pending'
               AND accepted_audit_ref IS NULL"
        ),
        1,
        "manual accepted acknowledgement CAS must roll back as one unit"
    );
    assert!(matches!(
        fixture
            .coordinator
            .verify_manual_accepted_delivery(&candidate.decision_identity),
        Err(DurableDeliveryError::PolicyMismatch(_))
    ));
    reconcile_terminal(
        &fixture,
        &rollback.inner,
        DecisionState::Delivered,
        &candidate.decision_identity,
    );
    fixture
        .coordinator
        .verify_manual_accepted_delivery(&candidate.decision_identity)
        .expect("exact retry persists queryable manual accepted audit evidence");
    let audit_identity = fixture
        .query_strings(
            "SELECT accepted_audit_identity FROM manual_resolutions
             WHERE disposition='Accepted'",
        )
        .into_iter()
        .next()
        .expect("persisted manual accepted audit identity");
    let records = rollback.inner.records.lock().expect("retry append records");
    let record = records
        .get(&audit_identity)
        .expect("exact manual accepted audit was appended on retry");
    assert_eq!(record.record_kind, "DeliveryAcceptedAudit");
    assert_eq!(sha256_hex(&record.canonical_bytes), record.sha256.as_str());
    assert!(!record.immutable_ref.is_empty());
}

#[test]
fn br192_manual_accepted_whitespace_append_ref_stays_pending_and_exact_retry_delivers() {
    let fixture = Fixture::new("MANUAL_ACCEPT_WHITESPACE_REF");
    let initial_append = MemoryAppendPort::default();
    let candidate = envelope(
        "MANUAL_ACCEPT_WHITESPACE_REF",
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        "2026-07-30",
        false,
    );
    prepare_reserved(&fixture, &candidate, &initial_append);
    let sink = StaticSink::new(AuthoritativeSinkResult::Uncertain(uncertainty(now())));
    let sinks: Vec<AuthoritativeSink> = vec![sink];
    fixture
        .coordinator
        .resume_deliverable(&candidate.decision_identity, &sinks, now())
        .expect("persist uncertain result");
    reconcile_terminal(
        &fixture,
        &initial_append,
        DecisionState::UncertainManualReview,
        &candidate.decision_identity,
    );
    fixture
        .coordinator
        .resolve_uncertain(
            &ManualResolutionCommand {
                decision_identity: candidate.decision_identity.clone(),
                disposition: ManualDisposition::Accepted {
                    receipt: Some(receipt(now())),
                },
                operator_identity: "TEST_CODE_OPERATOR_WHITESPACE_REF_0123456789".to_owned(),
                reason: "TEST_CODE_VERIFIED_ACCEPTANCE_WHITESPACE_REF".to_owned(),
                external_evidence: b"TEST_CODE_MANUAL_ACCEPT_WHITESPACE_REF_EVIDENCE".to_vec(),
                resolved_at: now(),
            },
            &initial_append,
        )
        .expect("persist manual acceptance as audit pending");
    let empty_append = EmptyAppendPort::new("DeliveryAcceptedAudit");

    assert!(matches!(
        fixture
            .coordinator
            .reconcile_all_pending(&empty_append, now()),
        Err(DurableDeliveryError::PolicyMismatch(reason))
            if reason.contains("immutable append returned an empty reference")
    ));
    assert_eq!(
        fixture
            .coordinator
            .decision_state(&candidate.decision_identity)
            .expect("manual accepted state after whitespace append reference"),
        DecisionState::AcceptedAuditPending
    );
    assert_eq!(
        fixture.query_i64(
            "SELECT COUNT(*) FROM manual_resolutions
             WHERE disposition='Accepted'
               AND accepted_audit_append_state='Pending'
               AND accepted_audit_ref IS NULL"
        ),
        1,
        "whitespace immutable reference must never acknowledge the manual accepted audit"
    );
    assert!(matches!(
        fixture
            .coordinator
            .verify_manual_accepted_delivery(&candidate.decision_identity),
        Err(DurableDeliveryError::PolicyMismatch(_))
    ));

    let audit_identity = fixture
        .query_strings(
            "SELECT accepted_audit_identity FROM manual_resolutions
             WHERE disposition='Accepted'",
        )
        .into_iter()
        .next()
        .expect("persisted manual accepted audit identity");
    let before_retry = empty_append
        .inner
        .records
        .lock()
        .expect("whitespace append records")
        .get(&audit_identity)
        .cloned()
        .expect("external append exists despite rejected whitespace reference");

    reconcile_terminal(
        &fixture,
        &empty_append.inner,
        DecisionState::Delivered,
        &candidate.decision_identity,
    );
    fixture
        .coordinator
        .verify_manual_accepted_delivery(&candidate.decision_identity)
        .expect("exact retry persists complete manual accepted audit evidence");
    let records = empty_append
        .inner
        .records
        .lock()
        .expect("retry append records");
    assert_eq!(
        records
            .get(&audit_identity)
            .expect("same manual accepted audit identity after retry"),
        &before_retry,
        "retry must reuse the exact external identity/canonical/hash/reference"
    );
    assert_eq!(
        records
            .values()
            .filter(|record| record.record_kind == "DeliveryAcceptedAudit")
            .count(),
        1,
        "exact retry must not duplicate the manual accepted delivery audit"
    );
}

#[test]
fn br192_schema_v2_migration_rejects_historical_manual_acceptance_semantic_mismatch() {
    let mut fixture = Fixture::new("MIGRATION_MANUAL_ACCEPT_MISMATCH");
    let append = MemoryAppendPort::default();
    let candidate = envelope(
        "MIGRATION_MANUAL_ACCEPT_MISMATCH",
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        "2026-07-30",
        false,
    );
    prepare_reserved(&fixture, &candidate, &append);
    let sinks: Vec<AuthoritativeSink> = vec![StaticSink::new(AuthoritativeSinkResult::Uncertain(
        uncertainty(now()),
    ))];
    fixture
        .coordinator
        .resume_deliverable(&candidate.decision_identity, &sinks, now())
        .expect("persist uncertain result");
    reconcile_terminal(
        &fixture,
        &append,
        DecisionState::UncertainManualReview,
        &candidate.decision_identity,
    );
    fixture
        .coordinator
        .resolve_uncertain(
            &ManualResolutionCommand {
                decision_identity: candidate.decision_identity.clone(),
                disposition: ManualDisposition::Accepted {
                    receipt: Some(receipt(now())),
                },
                operator_identity: "TEST_CODE_OPERATOR_MIGRATION_MISMATCH_0123456789".to_owned(),
                reason: "TEST_CODE_VERIFIED_ACCEPTANCE_MIGRATION_MISMATCH".to_owned(),
                external_evidence: b"TEST_CODE_MIGRATION_MISMATCH_EVIDENCE".to_vec(),
                resolved_at: now(),
            },
            &append,
        )
        .expect("persist valid manual acceptance");

    let coordinator = fixture
        .coordinator
        .take()
        .expect("release coordinator before historical schema mutation");
    drop(coordinator);
    let mut connection =
        Connection::open(&fixture.database_path).expect("open historical schema fixture");
    downgrade_manual_resolution_schema_for_test(&mut connection, 2);
    let changed = connection
        .execute(
            "UPDATE manual_resolutions
             SET evidence_sha256='ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff'
             WHERE decision_identity=?1",
            [candidate.decision_identity.as_str()],
        )
        .expect("inject historical external-evidence mismatch");
    assert_eq!(changed, 1);

    assert!(matches!(
        initialize_test_schema(&mut connection),
        Err(DurableDeliveryError::InvalidConfiguration(reason))
            if reason.contains("invalid semantic binding")
                && reason.contains("external evidence hash mismatch")
    ));
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .expect("read rolled-back v2 schema version"),
        2
    );
}

#[test]
fn br192_schema_v2_migration_rejects_manual_reason_and_authorization_ref_tampering() {
    for (label, column, replacement, expected_error) in [
        (
            "REASON",
            "reason",
            "TEST_CODE_TAMPERED_MIGRATION_REASON",
            "manual accepted delivery audit exact semantic binding mismatch",
        ),
        (
            "AUTH_REF",
            "immutable_audit_ref",
            "TEST_CODE_TAMPERED_MIGRATION_AUTHORIZATION_REF",
            "delivery audit exact semantic binding mismatch",
        ),
    ] {
        let mut fixture = Fixture::new(&format!("MIGRATION_MANUAL_TAMPER_{label}"));
        let append = MemoryAppendPort::default();
        let candidate = manual_accepted_pending_fixture(
            &fixture,
            &format!("MIGRATION_MANUAL_TAMPER_{label}"),
            &append,
        );

        let coordinator = fixture
            .coordinator
            .take()
            .expect("release coordinator before historical schema mutation");
        drop(coordinator);
        let mut connection =
            Connection::open(&fixture.database_path).expect("open historical schema fixture");
        downgrade_manual_resolution_schema_for_test(&mut connection, 2);
        let changed = connection
            .execute(
                &format!("UPDATE manual_resolutions SET {column}=?1 WHERE decision_identity=?2"),
                params![replacement, candidate.decision_identity],
            )
            .expect("inject historical manual acceptance tampering");
        assert_eq!(changed, 1);

        assert!(matches!(
            initialize_test_schema(&mut connection),
            Err(DurableDeliveryError::InvalidConfiguration(reason))
                if reason.contains("invalid semantic binding")
                    && reason.contains(expected_error)
        ));
        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .expect("read rolled-back v2 schema version"),
            2
        );
    }
}

#[test]
fn br192_manual_acceptance_is_revalidated_before_task_pending_reaches_delivered() {
    let fixture = Fixture::new("TASK_PENDING_MANUAL_REVALIDATION");
    let initial_append = MemoryAppendPort::default();
    let candidate = envelope(
        "TASK_PENDING_MANUAL_REVALIDATION",
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        "2026-07-30",
        true,
    );
    prepare_reserved(&fixture, &candidate, &initial_append);
    let sinks: Vec<AuthoritativeSink> = vec![StaticSink::new(AuthoritativeSinkResult::Uncertain(
        uncertainty(now()),
    ))];
    fixture
        .coordinator
        .resume_deliverable(&candidate.decision_identity, &sinks, now())
        .expect("persist uncertain result");
    reconcile_terminal(
        &fixture,
        &initial_append,
        DecisionState::UncertainManualReview,
        &candidate.decision_identity,
    );
    fixture
        .coordinator
        .resolve_uncertain(
            &ManualResolutionCommand {
                decision_identity: candidate.decision_identity.clone(),
                disposition: ManualDisposition::Accepted {
                    receipt: Some(receipt(now())),
                },
                operator_identity: "TEST_CODE_OPERATOR_TASK_REVALIDATE_0123456789".to_owned(),
                reason: "TEST_CODE_VERIFIED_ACCEPTANCE_TASK_REVALIDATE".to_owned(),
                external_evidence: b"TEST_CODE_TASK_REVALIDATE_EVIDENCE".to_vec(),
                resolved_at: now(),
            },
            &initial_append,
        )
        .expect("persist manual acceptance");
    let task_ref_failure = EmptyAppendPort::new("BR-140TaskTransition");
    assert!(matches!(
        fixture
            .coordinator
            .reconcile_all_pending(&task_ref_failure, now()),
        Err(DurableDeliveryError::PolicyMismatch(reason))
            if reason.contains("immutable append returned an empty reference")
    ));
    assert_eq!(
        fixture
            .coordinator
            .decision_state(&candidate.decision_identity)
            .expect("state after task-transition append acknowledgement failure"),
        DecisionState::AcceptedTaskTransitionPending
    );

    let connection =
        Connection::open(&fixture.database_path).expect("open test-only semantic corruption");
    connection
        .execute_batch("DROP TRIGGER immutable_manual_resolution_update;")
        .expect("remove test-only immutability guard");
    connection
        .execute(
            "UPDATE manual_resolutions
             SET evidence_sha256='ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff'
             WHERE decision_identity=?1",
            [candidate.decision_identity.as_str()],
        )
        .expect("inject test-only historical evidence mismatch");
    drop(connection);

    assert!(matches!(
        fixture
            .coordinator
            .reconcile_all_pending(&task_ref_failure.inner, now()),
        Err(DurableDeliveryError::PolicyMismatch(reason))
            if reason.contains("external evidence hash mismatch")
    ));
    assert_eq!(
        fixture
            .coordinator
            .decision_state(&candidate.decision_identity)
            .expect("mismatched manual acceptance remains non-delivered"),
        DecisionState::AcceptedTaskTransitionPending
    );
}

#[test]
fn nonexpired_foreign_attempt_is_not_recovered() {
    let fixture = Fixture::new("LIVE_FOREIGN");
    let second = fixture.second_coordinator("LIVE_FOREIGN");
    let append = MemoryAppendPort::default();
    let envelope = envelope(
        "LIVE_FOREIGN",
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        "2026-07-30",
        false,
    );
    prepare_reserved(&fixture, &envelope, &append);
    fixture
        .coordinator
        .begin_attempt(&envelope.decision_identity, 1, now())
        .expect("begin attempt")
        .expect("attempt created");
    let summary = second
        .reconcile_all_pending(&append, now())
        .expect("foreign reconciliation");
    assert_eq!(summary.sink_calls, 0);
    assert_eq!(summary.provider_calls, 0);
    assert_eq!(
        summary.non_progressable_foreign_attempts,
        vec![envelope.decision_identity.clone()]
    );
    assert!(
        summary.locally_pending_decisions.is_empty(),
        "a live foreign lease is an explicit non-progressable boundary"
    );
    assert_eq!(
        second
            .decision_state(&envelope.decision_identity)
            .expect("state"),
        DecisionState::AttemptInFlight
    );
}

#[test]
fn expired_attempt_revokes_fence_once() {
    let fixture = Fixture::new("EXPIRED");
    let append = MemoryAppendPort::default();
    let envelope = envelope(
        "EXPIRED",
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        "2026-07-30",
        false,
    );
    prepare_reserved(&fixture, &envelope, &append);
    fixture
        .coordinator
        .begin_attempt(&envelope.decision_identity, 1, now())
        .expect("begin attempt")
        .expect("attempt created");
    let recovered_at = now() + chrono::Duration::seconds(121);
    let first = fixture
        .coordinator
        .reconcile_all_pending(&append, recovered_at)
        .expect("recover expired");
    assert!(first.progress_count > 0);
    assert_eq!(
        fixture.query_i64(
            "SELECT COUNT(*) FROM delivery_attempt_events
             WHERE event_kind='FenceRevoked'"
        ),
        1
    );
    let second = fixture
        .coordinator
        .reconcile_all_pending(&append, recovered_at)
        .expect("idempotent recovery");
    assert_eq!(second.progress_count, 0);
    assert_eq!(
        fixture.query_i64(
            "SELECT COUNT(*) FROM delivery_attempt_events
             WHERE event_kind='FenceRevoked'"
        ),
        1
    );
}

#[test]
fn accepted_result_commit_crash_loses_in_memory_receipt_and_never_resends() {
    let fixture = Fixture::new("CRASH");
    let append = MemoryAppendPort::default();
    let envelope = envelope(
        "CRASH",
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        "2026-07-30",
        false,
    );
    prepare_reserved(&fixture, &envelope, &append);
    let attempt = fixture
        .coordinator
        .begin_attempt(&envelope.decision_identity, 1, now())
        .expect("begin attempt")
        .expect("attempt created");
    let sink = StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(now())));
    let in_memory_result = sink.deliver(&attempt.request);
    assert!(matches!(
        in_memory_result,
        AuthoritativeSinkResult::Accepted(_)
    ));
    drop(in_memory_result);
    fixture
        .coordinator
        .reconcile_all_pending(&append, now() + chrono::Duration::seconds(121))
        .expect("recover crashed owner");
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM sink_results"),
        0,
        "lost in-memory receipt must not be fabricated as durable"
    );
    assert_eq!(
        fixture
            .coordinator
            .decision_state(&envelope.decision_identity)
            .expect("state"),
        DecisionState::UncertainManualReview
    );
    let sinks: Vec<AuthoritativeSink> = vec![sink.clone()];
    let resumed = fixture
        .coordinator
        .resume_deliverable(
            &envelope.decision_identity,
            &sinks,
            now() + chrono::Duration::seconds(122),
        )
        .expect("uncertain is not resumable");
    assert_eq!(resumed.sink_calls, 0);
    assert_eq!(sink.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn late_accepted_after_fence_stays_manual() {
    let fixture = Fixture::new("LATE");
    let append = MemoryAppendPort::default();
    let envelope = envelope(
        "LATE",
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        "2026-07-30",
        false,
    );
    prepare_reserved(&fixture, &envelope, &append);
    let attempt = fixture
        .coordinator
        .begin_attempt(&envelope.decision_identity, 1, now())
        .expect("begin attempt")
        .expect("attempt created");
    let recovered_at = now() + chrono::Duration::seconds(121);
    fixture
        .coordinator
        .reconcile_all_pending(&append, recovered_at)
        .expect("recover");
    fixture
        .coordinator
        .record_sink_result(
            &attempt.attempt_identity,
            attempt.fence_token,
            AuthoritativeSinkResult::Accepted(receipt(recovered_at)),
            recovered_at,
        )
        .expect("persist late receipt");
    fixture
        .coordinator
        .reconcile_all_pending(&append, recovered_at)
        .expect("append late audit");
    assert_eq!(
        fixture
            .coordinator
            .decision_state(&envelope.decision_identity)
            .expect("state"),
        DecisionState::UncertainManualReview
    );
    assert_eq!(
        fixture.query_i64(
            "SELECT COUNT(*) FROM sink_results
             WHERE result_kind='Accepted' AND authoritative_for_state=0
               AND late_after_fence=1"
        ),
        1
    );
    assert_eq!(
        fixture.query_i64(
            "SELECT COUNT(DISTINCT event_kind) FROM delivery_attempt_events
             WHERE event_kind IN ('SinkResultAuthorityClassified','LateReceiptObserved')"
        ),
        2
    );
}

#[test]
fn two_process_resume_calls_invoke_one_sink() {
    let fixture = Fixture::new("CONCURRENT_RESUME");
    let second = fixture.second_coordinator("CONCURRENT_RESUME");
    let append = MemoryAppendPort::default();
    let envelope = envelope(
        "CONCURRENT_RESUME",
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        "2026-07-30",
        false,
    );
    prepare_reserved(&fixture, &envelope, &append);
    let (sink, entered) = BlockingSink::new(AuthoritativeSinkResult::Accepted(receipt(now())));
    let first_coordinator = fixture.coordinator.clone();
    let first_identity = envelope.decision_identity.clone();
    let first_sink: AuthoritativeSink = sink.clone();
    let handle = std::thread::spawn(move || {
        first_coordinator.resume_deliverable(&first_identity, &[first_sink], now())
    });
    entered.recv().expect("first process entered sink");
    let second_sink: AuthoritativeSink = sink.clone();
    let loser = second
        .resume_deliverable(
            &envelope.decision_identity,
            &[second_sink],
            now() + chrono::Duration::seconds(1),
        )
        .expect("loser returns persisted in-flight state");
    assert_eq!(loser.sink_calls, 0);
    sink.release.wait();
    let winner = handle
        .join()
        .expect("winner thread")
        .expect("winner result");
    assert_eq!(winner.sink_calls, 1);
    assert_eq!(sink.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM delivery_attempts"),
        1
    );
}

#[test]
fn resume_reserved_after_restart_uses_stored_envelope() {
    let fixture = Fixture::new("RESTART");
    let second = fixture.second_coordinator("RESTART");
    let append = MemoryAppendPort::default();
    let envelope = envelope(
        "RESTART",
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        "2026-07-30",
        false,
    );
    prepare_reserved(&fixture, &envelope, &append);
    let sink = StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(now())));
    let sink_port: AuthoritativeSink = sink.clone();
    let result = second
        .resume_deliverable(&envelope.decision_identity, &[sink_port], now())
        .expect("restart resume");
    assert_eq!(result.sink_calls, 1);
    assert!(result.persisted_receipt);
    assert_eq!(sink.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn reconcile_replays_frozen_bytes_idempotently() {
    let fixture = Fixture::new("REPLAY");
    let append = MemoryAppendPort::default();
    let envelope = envelope(
        "REPLAY",
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        "2026-07-30",
        false,
    );
    let denied = fixture
        .coordinator
        .prepare(&envelope, 0, now())
        .expect("pre-sink denial");
    assert_eq!(denied.state, DecisionState::RejectedAuditPending);
    let first = fixture
        .coordinator
        .reconcile_all_pending(&append, now())
        .expect("first reconcile");
    assert!(first.progress_count > 0);
    let record_count = append.records.lock().expect("records").len();
    let second = fixture
        .coordinator
        .reconcile_all_pending(&append, now())
        .expect("second reconcile");
    assert_eq!(second.progress_count, 0);
    assert_eq!(append.records.lock().expect("records").len(), record_count);
}

#[test]
fn released_budget_generations_remain_queryable() {
    let fixture = Fixture::new("GENERATIONS");
    let append = MemoryAppendPort::default();
    let envelope = envelope(
        "GENERATIONS",
        // BR-237: ReviewProviderTopN 已豁免日预算, 换 SectorTop
        // (BusinessDateOnce + counts_against_daily_budget=true)。
        PushKind::SectorTop,
        DeliverySubKind::None,
        "2026-07-30",
        true,
    );
    prepare_reserved(&fixture, &envelope, &append);
    let first_sink = StaticSink::new(AuthoritativeSinkResult::Rejected(rejection(now(), true)));
    let first: AuthoritativeSink = first_sink;
    fixture
        .coordinator
        .resume_deliverable(&envelope.decision_identity, &[first], now())
        .expect("first rejection");
    reconcile_terminal(
        &fixture,
        &append,
        DecisionState::RejectedDurable,
        &envelope.decision_identity,
    );
    let second_sink = StaticSink::new(AuthoritativeSinkResult::Uncertain(uncertainty(
        now() + chrono::Duration::seconds(1),
    )));
    let second: AuthoritativeSink = second_sink;
    fixture
        .coordinator
        .resume_deliverable(
            &envelope.decision_identity,
            &[second],
            now() + chrono::Duration::seconds(1),
        )
        .expect("authorized retry");
    assert_eq!(
        fixture.query_strings(
            "SELECT state FROM daily_budget_reservations
             ORDER BY reservation_generation"
        ),
        vec!["Released".to_owned(), "Uncertain".to_owned()]
    );
    assert_eq!(
        fixture.query_strings(
            "SELECT CAST(reservation_generation AS TEXT)
             FROM daily_budget_reservations ORDER BY reservation_generation"
        ),
        vec!["1".to_owned(), "2".to_owned()]
    );
}

#[test]
fn previous_date_pending_is_reconciled_by_all_date_pass() {
    let fixture = Fixture::new("PREVIOUS_DATE");
    let append = MemoryAppendPort::default();
    let envelope = envelope(
        "PREVIOUS_DATE",
        PushKind::ReviewProviderTopN,
        DeliverySubKind::None,
        "2026-07-29",
        true,
    );
    fixture
        .coordinator
        .prepare(&envelope, 0, now())
        .expect("previous-date durable denial");
    assert!(
        fixture
            .coordinator
            .inspect_pending_for_date("2026-07-30")
            .expect("today diagnostic")
            .is_empty(),
        "date-scoped diagnostic must not pretend previous-date work is reconciled"
    );
    let summary = fixture
        .coordinator
        .reconcile_all_pending(&append, now())
        .expect("all-date reconcile");
    assert_eq!(summary.provider_calls, 0);
    assert_eq!(summary.sink_calls, 0);
    assert_eq!(
        fixture
            .coordinator
            .decision_state(&envelope.decision_identity)
            .expect("state"),
        DecisionState::RejectedDurable
    );
}

#[test]
fn uncertain_manual_review_is_a_non_progressable_readiness_boundary() {
    let fixture = Fixture::new("MANUAL_BOUNDARY");
    let append = MemoryAppendPort::default();
    let envelope = envelope(
        "MANUAL_BOUNDARY",
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        "2026-07-30",
        false,
    );
    prepare_reserved(&fixture, &envelope, &append);
    let sink = StaticSink::new(AuthoritativeSinkResult::Uncertain(uncertainty(now())));
    let sinks: Vec<AuthoritativeSink> = vec![sink];
    fixture
        .coordinator
        .resume_deliverable(&envelope.decision_identity, &sinks, now())
        .expect("record uncertain result");
    let summary = fixture
        .coordinator
        .reconcile_all_pending(&append, now())
        .expect("reach manual-review boundary");

    assert!(summary.locally_pending_decisions.is_empty());
    assert!(summary.deliverable_decisions.is_empty());
    assert_eq!(
        summary.non_progressable_manual_reviews,
        vec![envelope.decision_identity]
    );
}

#[test]
fn schedule_hydration_exposes_exact_basis_and_is_acknowledged_once() {
    let fixture = Fixture::new("HYDRATION_ACK");
    let append = MemoryAppendPort::default();
    let envelope = envelope(
        "HYDRATION_ACK",
        PushKind::ReviewProviderTopN,
        DeliverySubKind::None,
        "2026-07-30",
        true,
    );
    prepare_reserved(&fixture, &envelope, &append);
    let sink = StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(now())));
    let sinks: Vec<AuthoritativeSink> = vec![sink];
    fixture
        .coordinator
        .resume_deliverable(&envelope.decision_identity, &sinks, now())
        .expect("record accepted result");
    let summary = fixture
        .coordinator
        .reconcile_all_pending(&append, now())
        .expect("append transition and expose hydration");
    assert_eq!(summary.schedule_hydrations.len(), 1);
    let hydration = summary.schedule_hydrations[0].clone();
    assert_eq!(hydration.decision_identity, envelope.decision_identity);
    assert_eq!(hydration.task_identity, "TEST_CODE_TASK_HYDRATION_ACK");
    assert_eq!(
        hydration.transition_basis_canonical,
        b"TEST_CODE_TRANSITION_BASIS_HYDRATION_ACK"
    );
    assert_eq!(
        sha256_hex(&hydration.transition_basis_canonical),
        hydration.transition_basis_sha256
    );
    assert_eq!(hydration.hydration_state, ScheduleHydrationState::Pending);
    assert!(!hydration.transition_canonical.is_empty());
    assert_eq!(
        sha256_hex(&hydration.transition_canonical),
        hydration.transition_sha256
    );

    let replay = fixture
        .coordinator
        .prepare(&envelope, 1, now())
        .expect("idempotent decision replay");
    assert_eq!(replay.schedule_hydration, Some(hydration.clone()));

    assert!(fixture
        .coordinator
        .acknowledge_schedule_hydration(
            &hydration.transition_identity,
            &hydration.transition_sha256,
            now(),
        )
        .expect("first hydration acknowledgement"));
    let after_ack = fixture
        .coordinator
        .reconcile_all_pending(&append, now())
        .expect("append acknowledgement audit");
    assert_eq!(after_ack.schedule_hydrations.len(), 1);
    assert_eq!(
        after_ack.schedule_hydrations[0].hydration_state,
        ScheduleHydrationState::Applied
    );
    assert!(!fixture
        .coordinator
        .acknowledge_schedule_hydration(
            &hydration.transition_identity,
            &hydration.transition_sha256,
            now(),
        )
        .expect("idempotent hydration acknowledgement"));
    let restarted = fixture.second_coordinator("HYDRATION_RESTART");
    let restart_summary = restarted
        .reconcile_all_pending(&append, now())
        .expect("restart reconstructs applied task transition");
    assert_eq!(restart_summary.schedule_hydrations.len(), 1);
    assert_eq!(
        restart_summary.schedule_hydrations[0],
        after_ack.schedule_hydrations[0]
    );
    let replay_after_restart = restarted
        .prepare(&envelope, 1, now())
        .expect("terminal replay reconstructs applied hydration");
    assert_eq!(
        replay_after_restart.schedule_hydration,
        Some(after_ack.schedule_hydrations[0].clone())
    );
    assert_eq!(append.count_kind("ScheduleHydrationApplied"), 1);
}

#[test]
fn schedule_hydration_applied_is_reported_only_after_immutable_audit_and_restart_is_idempotent() {
    let mut fixture = Fixture::new("HYDRATION_DURABLE_ACK");
    let append = FailScheduleHydrationAppliedOnce::default();
    let envelope = envelope(
        "HYDRATION_DURABLE_ACK",
        PushKind::ReviewProviderTopN,
        DeliverySubKind::None,
        "2026-07-30",
        true,
    );
    prepare_reserved(&fixture, &envelope, &append);
    let sink = StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(now())));
    let sinks: Vec<AuthoritativeSink> = vec![sink];
    fixture
        .coordinator
        .resume_deliverable(&envelope.decision_identity, &sinks, now())
        .expect("record accepted result");
    let summary = fixture
        .coordinator
        .reconcile_all_pending(&append, now())
        .expect("append task transition");
    let hydration = summary
        .schedule_hydrations
        .into_iter()
        .next()
        .expect("pending hydration");

    let first = fixture
        .coordinator
        .persist_schedule_hydration_applied(
            &hydration.transition_identity,
            &hydration.transition_sha256,
            &append,
            now(),
        )
        .expect_err("immutable acknowledgement append failure must fail closed");
    assert!(first
        .to_string()
        .contains("TEST_CODE_INJECTED_HYDRATION_ACK_APPEND_FAILURE"));
    assert_eq!(
        fixture.query_strings(
            "SELECT hydration_state FROM task_transition_payloads
             ORDER BY transition_identity"
        ),
        vec!["Applied"]
    );
    assert_eq!(
        fixture.query_strings(
            "SELECT append_state FROM immutable_audit_outbox
             WHERE audit_kind='ScheduleHydrationApplied'"
        ),
        vec!["Pending"]
    );
    assert_eq!(append.inner.count_kind("ScheduleHydrationApplied"), 0);

    drop(fixture.coordinator.take());
    let restarted = fixture.second_coordinator("HYDRATION_DURABLE_ACK_RESTART");
    restarted
        .persist_schedule_hydration_applied(
            &hydration.transition_identity,
            &hydration.transition_sha256,
            &append,
            now(),
        )
        .expect("restart finishes the exact pending immutable acknowledgement");
    assert_eq!(
        fixture.query_strings(
            "SELECT append_state FROM immutable_audit_outbox
             WHERE audit_kind='ScheduleHydrationApplied'"
        ),
        vec!["Appended"]
    );
    assert_eq!(append.inner.count_kind("ScheduleHydrationApplied"), 1);

    restarted
        .persist_schedule_hydration_applied(
            &hydration.transition_identity,
            &hydration.transition_sha256,
            &append,
            now(),
        )
        .expect("repeated acknowledgement is idempotent");
    assert_eq!(append.inner.count_kind("ScheduleHydrationApplied"), 1);
}

#[test]
fn non_task_rejected_uncertain_and_manual_paths_terminate() {
    let rejected_fixture = Fixture::new("NON_TASK_REJECTED");
    let rejected_append = MemoryAppendPort::default();
    let rejected = envelope(
        "NON_TASK_REJECTED",
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        "2026-07-30",
        false,
    );
    prepare_reserved(&rejected_fixture, &rejected, &rejected_append);
    let rejected_sink = StaticSink::new(AuthoritativeSinkResult::Rejected(rejection(now(), false)));
    let rejected_sinks: Vec<AuthoritativeSink> = vec![rejected_sink];
    rejected_fixture
        .coordinator
        .resume_deliverable(&rejected.decision_identity, &rejected_sinks, now())
        .expect("record non-task rejection");
    reconcile_terminal(
        &rejected_fixture,
        &rejected_append,
        DecisionState::RejectedDurable,
        &rejected.decision_identity,
    );
    assert_eq!(
        rejected_fixture.query_i64("SELECT COUNT(*) FROM task_transition_payloads"),
        0
    );

    let manual_fixture = Fixture::new("NON_TASK_MANUAL");
    let manual_append = MemoryAppendPort::default();
    let manual = envelope(
        "NON_TASK_MANUAL",
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        "2026-07-30",
        false,
    );
    prepare_reserved(&manual_fixture, &manual, &manual_append);
    let uncertain_sink = StaticSink::new(AuthoritativeSinkResult::Uncertain(uncertainty(now())));
    let uncertain_sinks: Vec<AuthoritativeSink> = vec![uncertain_sink];
    manual_fixture
        .coordinator
        .resume_deliverable(&manual.decision_identity, &uncertain_sinks, now())
        .expect("record non-task uncertainty");
    reconcile_terminal(
        &manual_fixture,
        &manual_append,
        DecisionState::UncertainManualReview,
        &manual.decision_identity,
    );
    let state = manual_fixture
        .coordinator
        .resolve_uncertain(
            &ManualResolutionCommand {
                decision_identity: manual.decision_identity.clone(),
                disposition: ManualDisposition::Rejected,
                operator_identity: "TEST_CODE_OPERATOR_0123456789".to_owned(),
                reason: "TEST_CODE_VERIFIED_REJECTION".to_owned(),
                external_evidence: b"TEST_CODE_MANUAL_REJECTION_EVIDENCE".to_vec(),
                resolved_at: now(),
            },
            &manual_append,
        )
        .expect("manual non-task rejection");
    assert_eq!(state, DecisionState::ManualRejectedAuditPending);
    reconcile_terminal(
        &manual_fixture,
        &manual_append,
        DecisionState::ManualResolvedRejected,
        &manual.decision_identity,
    );
    assert_eq!(
        manual_fixture.query_i64("SELECT COUNT(*) FROM task_transition_payloads"),
        0
    );
}

#[test]
fn critical_state_lease_fence_and_late_receipt_audits_reconcile() {
    let fixture = Fixture::new("CRITICAL_AUDITS");
    let append = MemoryAppendPort::default();
    let envelope = envelope(
        "CRITICAL_AUDITS",
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        "2026-07-30",
        false,
    );
    prepare_reserved(&fixture, &envelope, &append);
    let attempt = fixture
        .coordinator
        .begin_attempt(&envelope.decision_identity, 1, now())
        .expect("begin attempt")
        .expect("attempt created");
    let recovered_at = now() + chrono::Duration::seconds(121);
    fixture
        .coordinator
        .reconcile_all_pending(&append, recovered_at)
        .expect("recover expired attempt");
    fixture
        .coordinator
        .record_sink_result(
            &attempt.attempt_identity,
            attempt.fence_token,
            AuthoritativeSinkResult::Accepted(receipt(recovered_at)),
            recovered_at,
        )
        .expect("record late accepted receipt");
    fixture
        .coordinator
        .reconcile_all_pending(&append, recovered_at)
        .expect("append authority and late-receipt audits");

    assert_eq!(
        fixture
            .query_i64("SELECT COUNT(*) FROM immutable_audit_outbox WHERE append_state='Pending'"),
        0
    );
    for kind in [
        "DecisionStateChanged",
        "LeaseGranted",
        "FenceRevoked",
        "RecoveryClassified",
        "SinkResultAuthorityClassified",
        "LateReceiptObserved",
    ] {
        assert!(
            append.count_kind(kind) > 0,
            "critical audit kind {kind} must be durably appended"
        );
    }
}

#[test]
fn cooldown_projection_events_are_append_only() {
    let fixture = Fixture::new("COOLDOWN_HISTORY");
    let append = MemoryAppendPort::default();
    let envelope = envelope(
        "COOLDOWN_HISTORY",
        PushKind::HoldingPlan,
        DeliverySubKind::None,
        "2026-07-30",
        false,
    );
    prepare_reserved(&fixture, &envelope, &append);
    let rejection_sink = StaticSink::new(AuthoritativeSinkResult::Rejected(rejection(now(), true)));
    let rejection_sinks: Vec<AuthoritativeSink> = vec![rejection_sink];
    fixture
        .coordinator
        .resume_deliverable(&envelope.decision_identity, &rejection_sinks, now())
        .expect("release first cooldown generation");
    reconcile_terminal(
        &fixture,
        &append,
        DecisionState::RejectedDurable,
        &envelope.decision_identity,
    );
    let uncertain_sink = StaticSink::new(AuthoritativeSinkResult::Uncertain(uncertainty(
        now() + chrono::Duration::seconds(1),
    )));
    let uncertain_sinks: Vec<AuthoritativeSink> = vec![uncertain_sink];
    fixture
        .coordinator
        .resume_deliverable(
            &envelope.decision_identity,
            &uncertain_sinks,
            now() + chrono::Duration::seconds(1),
        )
        .expect("reserve second cooldown generation");

    assert_eq!(
        fixture.query_strings(
            "SELECT state FROM cooldown_reservations ORDER BY reservation_generation"
        ),
        vec!["Released".to_owned(), "Uncertain".to_owned()]
    );
    assert_eq!(
        fixture.query_strings(
            "SELECT CAST(r.reservation_generation AS TEXT)
             FROM cooldown_reservation_events e
             JOIN cooldown_reservations r
               ON r.cooldown_reservation_identity=e.cooldown_reservation_identity
             ORDER BY e.rowid"
        ),
        vec![
            "1".to_owned(),
            "1".to_owned(),
            "1".to_owned(),
            "2".to_owned(),
            "2".to_owned(),
            "2".to_owned()
        ]
    );
}

#[test]
fn w12_legacy_envelope_keeps_exact_identity_and_canonical_bytes() {
    let legacy = envelope(
        "W12_LEGACY_GOLDEN",
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        "2026-07-30",
        false,
    );
    let canonical = legacy.canonical_bytes().expect("canonical legacy envelope");

    assert_eq!(
        legacy.decision_identity,
        "fd2b10332c1a463dcd5e9fc74e85f388e695bd27679ba61b45878691f5803056"
    );
    assert_eq!(
        sha256_hex(&canonical),
        "5e431e42aa9db00e7a548d490fea575b8c8ba8f882d22d4ceb9ac1843f4fe32f"
    );
    assert!(!canonical
        .windows(b"foundation_binding".len())
        .any(|window| window == b"foundation_binding"));
    assert!(legacy.foundation_binding().is_none());
}

#[test]
fn w12_foundation_binding_owns_application_decision_and_exact_cross_fields() {
    let mut candidate = envelope(
        "W12_FOUNDATION",
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        "2026-07-30",
        false,
    );
    candidate.source_evidence_fingerprint = sha256_hex(b"TEST_CODE_W12_FOUNDATION_SOURCE_EVIDENCE");
    candidate.schedule_occurrence_identity = sha256_hex(b"TEST_CODE_W12_OCCURRENCE");
    candidate.delivery_subject_hash = sha256_hex(b"TEST_CODE_W12_SUBJECT");
    let application_decision_id = sha256_hex(b"TEST_CODE_W12_APPLICATION_DECISION");
    let binding = FoundationDeliveryBinding::try_new(
        "Test:TEST_CODE_W12_RUN".to_owned(),
        application_decision_id.clone(),
        sha256_hex(b"TEST_CODE_W12_INTENT"),
        "MU-W12-generic".to_owned(),
        candidate.schedule_occurrence_identity.clone(),
        candidate.business_date.clone(),
        "Global".to_owned(),
        candidate.delivery_subject_hash.clone(),
        "TEST_CODE_W12_AUDIENCE".to_owned(),
        candidate.push_kind.stable_template_id().to_owned(),
        "v1".to_owned(),
        candidate.rendered_content_sha256.clone(),
        candidate.source_evidence_fingerprint.clone(),
        "TEST_CODE_CHANNEL".to_owned(),
    )
    .expect("valid W12 foundation binding");

    let bound = candidate
        .clone()
        .with_foundation_binding(binding)
        .expect("bind foundation decision");
    let persisted = bound.foundation_binding().expect("foundation binding");

    assert_eq!(bound.decision_identity, application_decision_id);
    assert_eq!(persisted.intent_id(), sha256_hex(b"TEST_CODE_W12_INTENT"));
    assert_eq!(persisted.required_channel(), "TEST_CODE_CHANNEL");
    assert_eq!(
        persisted.rendered_sha256(),
        candidate.rendered_content_sha256
    );
    assert_eq!(
        persisted
            .canonical_sha256()
            .expect("foundation binding canonical SHA")
            .len(),
        64
    );
    assert!(bound
        .canonical_bytes()
        .expect("foundation canonical envelope")
        .windows(b"foundation_binding".len())
        .any(|window| window == b"foundation_binding"));

    let wrong_date = FoundationDeliveryBinding::try_new(
        "Test:TEST_CODE_W12_RUN".to_owned(),
        sha256_hex(b"TEST_CODE_W12_APPLICATION_DECISION_BAD_DATE"),
        sha256_hex(b"TEST_CODE_W12_INTENT"),
        "MU-W12-generic".to_owned(),
        candidate.schedule_occurrence_identity.clone(),
        "2026-07-31".to_owned(),
        "Global".to_owned(),
        candidate.delivery_subject_hash.clone(),
        "TEST_CODE_W12_AUDIENCE".to_owned(),
        candidate.push_kind.stable_template_id().to_owned(),
        "v1".to_owned(),
        candidate.rendered_content_sha256.clone(),
        candidate.source_evidence_fingerprint.clone(),
        "TEST_CODE_CHANNEL".to_owned(),
    )
    .expect("individually valid binding");
    assert!(candidate.with_foundation_binding(wrong_date).is_err());
    assert!(FoundationDeliveryBinding::try_new(
        "Test:TEST_CODE_W12_RUN".to_owned(),
        sha256_hex(b"TEST_CODE_W12_APPLICATION_DECISION_BAD_CHANNEL"),
        sha256_hex(b"TEST_CODE_W12_INTENT"),
        "MU-W12-generic".to_owned(),
        sha256_hex(b"TEST_CODE_OCCURRENCE"),
        "2026-07-30".to_owned(),
        "Global".to_owned(),
        sha256_hex(b"TEST_CODE_SUBJECT_HASH"),
        "TEST_CODE_W12_AUDIENCE".to_owned(),
        "holding_event_v1".to_owned(),
        "v1".to_owned(),
        sha256_hex(b"TEST_CODE_RENDERED"),
        sha256_hex(b"TEST_CODE_SOURCE"),
        " bad-channel ".to_owned(),
    )
    .is_err());
}

fn w12_foundation_envelope(label: &str) -> DeliveryEnvelope {
    let mut candidate = envelope(
        label,
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        "2026-07-30",
        false,
    );
    candidate.source_evidence_fingerprint =
        sha256_hex(format!("TEST_CODE_W12_SOURCE_{label}").as_bytes());
    candidate.schedule_occurrence_identity =
        sha256_hex(format!("TEST_CODE_W12_OCCURRENCE_{label}").as_bytes());
    candidate.delivery_subject_hash =
        sha256_hex(format!("TEST_CODE_W12_SUBJECT_{label}").as_bytes());
    let binding = FoundationDeliveryBinding::try_new(
        format!("Test:TEST_CODE_W12_RUN_{label}"),
        sha256_hex(format!("TEST_CODE_W12_DECISION_{label}").as_bytes()),
        sha256_hex(format!("TEST_CODE_W12_INTENT_{label}").as_bytes()),
        "MU-W12-generic".to_owned(),
        candidate.schedule_occurrence_identity.clone(),
        candidate.business_date.clone(),
        "Global".to_owned(),
        candidate.delivery_subject_hash.clone(),
        format!("TEST_CODE_W12_AUDIENCE_{label}"),
        candidate.push_kind.stable_template_id().to_owned(),
        "v1".to_owned(),
        candidate.rendered_content_sha256.clone(),
        candidate.source_evidence_fingerprint.clone(),
        "TEST_CODE_CHANNEL".to_owned(),
    )
    .expect("valid W12 foundation binding");
    candidate
        .with_foundation_binding(binding)
        .expect("foundation-bound envelope")
}

fn w19_p01_recovery_envelope(label: &str) -> DeliveryEnvelope {
    DeliveryEnvelope::new(
        "2026-08-18",
        PushKind::PreopenNewsHot,
        DeliverySubKind::None,
        "GLOBAL",
        "p01:2026-08-18",
        format!("TEST_CODE_W19_P01_SOURCE_{label}"),
        br#"{"render_mode":"Scheduled","schema_version":"P01_SOURCE_BINDING_V1"}"#.to_vec(),
        format!("TEST_CODE_W19_P01_SUBJECT_{label}"),
        format!("TEST_CODE_W19_P01_RENDERED_{label}").into_bytes(),
        false,
        None,
    )
    .expect("valid W19 P01 recovery envelope")
}

fn w19_recover_uncertain_candidate(
    fixture: &Fixture,
    append: &MemoryAppendPort,
    candidate: &DeliveryEnvelope,
) -> (AttemptLease, DateTime<Utc>) {
    prepare_reserved(fixture, candidate, append);
    let attempt = fixture
        .coordinator
        .begin_attempt(&candidate.decision_identity, 1, now())
        .expect("begin real W19 recovery attempt")
        .expect("W19 recovery attempt created");
    let recovered_at = now() + chrono::Duration::seconds(121);
    let summary = fixture
        .coordinator
        .reconcile_all_pending(append, recovered_at)
        .expect("expire, classify and seal W19 recovery attempt");
    assert!(summary.progress_count > 0);
    assert_eq!(summary.provider_calls, 0);
    assert_eq!(summary.sink_calls, 0);
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM sink_results"), 0);
    assert_eq!(
        fixture
            .coordinator
            .decision_state(&candidate.decision_identity)
            .expect("read recovered decision state"),
        DecisionState::UncertainManualReview
    );
    (attempt, recovered_at)
}

#[test]
fn w12_terminal_read_model_distinguishes_missing_pending_and_accepted() {
    let fixture = Fixture::new("W12_TERMINAL_READ");
    let append = MemoryAppendPort::default();
    let candidate = w12_foundation_envelope("TERMINAL_READ");

    assert_eq!(
        fixture
            .coordinator
            .inspect_foundation_terminal(&candidate.decision_identity)
            .expect("missing query"),
        FoundationTerminalQuery::Missing
    );
    prepare_reserved(&fixture, &candidate, &append);
    assert!(matches!(
        fixture
            .coordinator
            .inspect_foundation_terminal(&candidate.decision_identity)
            .expect("pending query"),
        FoundationTerminalQuery::PendingSeal {
            state: DecisionState::Reserved
        }
    ));

    let sink = StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(now())));
    let sinks: Vec<AuthoritativeSink> = vec![sink.clone()];
    fixture
        .coordinator
        .resume_deliverable(&candidate.decision_identity, &sinks, now())
        .expect("record W12 accepted result");
    reconcile_terminal(
        &fixture,
        &append,
        DecisionState::Delivered,
        &candidate.decision_identity,
    );
    let terminal = match fixture
        .coordinator
        .inspect_foundation_terminal(&candidate.decision_identity)
        .expect("accepted terminal query")
    {
        FoundationTerminalQuery::Terminal(record) => record,
        other => panic!("expected W12 terminal record, got {other:?}"),
    };

    assert_eq!(
        terminal.disposition(),
        FoundationTerminalDisposition::Accepted
    );
    assert_eq!(terminal.attempt_id().is_some(), true);
    assert_eq!(terminal.required_channel(), "TEST_CODE_CHANNEL");
    assert_eq!(
        sha256_hex(terminal.evidence_bytes()),
        terminal.evidence_sha256()
    );
    assert_eq!(terminal.durable_schema_version(), 9);
    assert_eq!(sink.calls.load(Ordering::SeqCst), 1);
    let exact: serde_json::Value =
        serde_json::from_slice(terminal.evidence_bytes()).expect("exact typed result JSON");
    assert_eq!(exact["kind"], "Accepted");
    assert_eq!(exact["receipt"]["channel"], "TEST_CODE_CHANNEL");
    assert!(!format!("{terminal:?}").contains("TEST_CODE_MESSAGE"));
}

fn w16_recovery_facts(
    fixture: &Fixture,
    decision: Option<&str>,
) -> Vec<Vec<rusqlite::types::Value>> {
    let connection = Connection::open(&fixture.database_path).expect("isolated recovery snapshot");
    let mut result = Vec::new();
    for table in [
        "delivery_decisions",
        "delivery_attempts",
        "delivery_attempt_events",
        "delivery_state_events",
        "immutable_audit_outbox",
        "sink_results",
        "delivery_disposition_payloads",
        "task_transition_payloads",
        "daily_budget_reservations",
        "cooldown_reservations",
        "business_date_once_claims",
    ] {
        let mut statement = connection
            .prepare(&format!(
                "SELECT * FROM {table} WHERE (?1 IS NULL OR decision_identity=?1) ORDER BY rowid"
            ))
            .expect("prepare recovery facts");
        let columns = statement.column_count();
        result.extend(
            statement
                .query_map([decision], |row| {
                    (0..columns)
                        .map(|index| row.get(index))
                        .collect::<rusqlite::Result<Vec<_>>>()
                })
                .expect("read recovery facts")
                .collect::<rusqlite::Result<Vec<_>>>()
                .expect("collect facts"),
        );
    }
    result
}

fn w16_assert_empty_summary(summary: &ReconcileSummary) {
    assert_eq!(summary.provider_calls, 0);
    assert_eq!(summary.sink_calls, 0);
    assert!(summary.locally_pending_decisions.is_empty());
    assert!(summary.deliverable_decisions.is_empty());
    assert!(summary.non_progressable_foreign_attempts.is_empty());
    assert!(summary.non_progressable_manual_reviews.is_empty());
    assert!(summary.schedule_hydrations.is_empty());
}

#[test]
fn w16_scoped_recovery_preserves_other_attempts_payloads_and_hydrations() {
    let fixture = Fixture::new("W16_SCOPE_MATRIX");
    let append = MemoryAppendPort::default();
    let hydrated = envelope(
        "W16_HYDRATION",
        PushKind::CandidateTriggered,
        DeliverySubKind::None,
        "2026-07-29",
        true,
    );
    fixture
        .coordinator
        .prepare(&hydrated, 0, now())
        .expect("other task denial");
    let initial = fixture
        .coordinator
        .reconcile_all_pending(&append, now())
        .expect("seal other task");
    assert_eq!(initial.schedule_hydrations.len(), 1);
    let expired = envelope(
        "W16_EXPIRED_OTHER",
        PushKind::CandidateTriggered,
        DeliverySubKind::None,
        "2026-07-29",
        false,
    );
    fixture
        .coordinator
        .prepare(&expired, 1, now())
        .expect("other reservation");
    fixture
        .coordinator
        .begin_attempt(&expired.decision_identity, 1, now())
        .expect("other attempt")
        .expect("lease");
    let pending = envelope(
        "W16_PENDING_OTHER",
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        "2026-07-29",
        true,
    );
    fixture
        .coordinator
        .prepare(&pending, 0, now())
        .expect("other frozen pending payload");
    let reserved = envelope(
        "W16_RESERVED_OTHER",
        PushKind::CandidateTriggered,
        DeliverySubKind::None,
        "2026-07-29",
        false,
    );
    fixture
        .coordinator
        .prepare(&reserved, 1, now())
        .expect("other deliverable");
    let target = w12_foundation_envelope("W16_SCOPE_TARGET");
    fixture
        .coordinator
        .prepare(&target, 0, now())
        .expect("target frozen payload");
    let others = [&hydrated, &expired, &pending, &reserved];
    let before: Vec<_> = others
        .iter()
        .map(|other| w16_recovery_facts(&fixture, Some(&other.decision_identity)))
        .collect();
    let previous_records = append
        .records
        .lock()
        .expect("initial append records")
        .clone();
    let at = now() + chrono::Duration::seconds(121);
    let summary = fixture
        .coordinator
        .reconcile_foundation_decision(
            &target.decision_identity,
            target.foundation_binding().expect("binding"),
            &append,
            at,
        )
        .expect("scoped recovery bypasses earlier unrelated candidates");
    w16_assert_empty_summary(&summary);
    assert!(summary.progress_count > 0);
    assert_eq!(
        fixture
            .coordinator
            .decision_state(&target.decision_identity)
            .expect("target state"),
        DecisionState::RejectedDurable
    );
    for (other, facts) in others.iter().zip(&before) {
        assert_eq!(
            &w16_recovery_facts(&fixture, Some(&other.decision_identity)),
            facts
        );
    }
    let target_records = append
        .records
        .lock()
        .expect("target exact append records")
        .clone();
    for (identity, record) in &target_records {
        if !previous_records.contains_key(identity) {
            assert!(String::from_utf8_lossy(&record.canonical_bytes)
                .contains(&target.decision_identity));
        }
    }
    let replay = fixture
        .coordinator
        .reconcile_foundation_decision(
            &target.decision_identity,
            target.foundation_binding().expect("binding"),
            &append,
            at,
        )
        .expect("idempotent scoped recovery");
    w16_assert_empty_summary(&replay);
    assert_eq!(replay.progress_count, 0);
    assert_eq!(
        *append.records.lock().expect("replayed records"),
        target_records
    );
    let global = fixture
        .coordinator
        .reconcile_all_pending(&append, at)
        .expect("unchanged global recovery");
    assert!(global
        .non_progressable_manual_reviews
        .contains(&expired.decision_identity));
    assert!(global
        .deliverable_decisions
        .contains(&reserved.decision_identity));
    assert_eq!(global.schedule_hydrations.len(), 2);
    assert_eq!(
        fixture
            .coordinator
            .decision_state(&pending.decision_identity)
            .expect("other recovered payload"),
        DecisionState::RejectedDurable
    );
    assert_eq!(
        fixture
            .coordinator
            .decision_state(&expired.decision_identity)
            .expect("other recovered lease"),
        DecisionState::UncertainManualReview
    );
}

#[test]
fn w16_scoped_recovery_revokes_only_the_target_expired_attempt_once() {
    let fixture = Fixture::new("W16_SCOPED_EXPIRED");
    let append = MemoryAppendPort::default();
    let other = envelope(
        "W16_EARLIER_EXPIRED",
        PushKind::CandidateTriggered,
        DeliverySubKind::None,
        "2026-07-29",
        false,
    );
    let target = w12_foundation_envelope("W16_EXPIRED_TARGET");
    for candidate in [&other, &target] {
        fixture
            .coordinator
            .prepare(candidate, 1, now())
            .expect("prepare expired candidate");
        fixture
            .coordinator
            .begin_attempt(&candidate.decision_identity, 1, now())
            .expect("begin candidate")
            .expect("candidate lease");
    }
    let before = w16_recovery_facts(&fixture, Some(&other.decision_identity));
    let at = now() + chrono::Duration::seconds(121);
    let summary = fixture
        .coordinator
        .reconcile_foundation_decision(
            &target.decision_identity,
            target.foundation_binding().expect("binding"),
            &append,
            at,
        )
        .expect("recover target expired lease");
    assert_eq!(
        summary.non_progressable_manual_reviews,
        vec![target.decision_identity.clone()]
    );
    assert!(summary.locally_pending_decisions.is_empty());
    assert_eq!(summary.sink_calls, 0);
    assert_eq!(
        w16_recovery_facts(&fixture, Some(&other.decision_identity)),
        before
    );
    assert_eq!(
        fixture
            .coordinator
            .decision_state(&target.decision_identity)
            .expect("target uncertainty"),
        DecisionState::UncertainManualReview
    );
    let target_facts = w16_recovery_facts(&fixture, Some(&target.decision_identity));
    let records = append.records.lock().expect("target records").clone();
    let replay = fixture
        .coordinator
        .reconcile_foundation_decision(
            &target.decision_identity,
            target.foundation_binding().expect("binding"),
            &append,
            at,
        )
        .expect("replay uncertainty");
    assert_eq!(replay.progress_count, 0);
    assert_eq!(
        w16_recovery_facts(&fixture, Some(&target.decision_identity)),
        target_facts
    );
    assert_eq!(
        w16_recovery_facts(&fixture, Some(&other.decision_identity)),
        before
    );
    assert_eq!(*append.records.lock().expect("exact records"), records);
    assert_eq!(
        fixture.query_i64(
            "SELECT COUNT(*) FROM delivery_attempt_events WHERE event_kind='FenceRevoked'"
        ),
        1
    );
}

#[test]
fn w16_scoped_recovery_rejects_every_binding_mismatch_missing_and_legacy_without_writes() {
    let fixture = Fixture::new("W16_BINDING_REJECTION");
    let append = MemoryAppendPort::default();
    let target = w12_foundation_envelope("W16_BINDING_TARGET");
    let legacy = envelope(
        "W16_LEGACY_REJECTION",
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        "2026-07-29",
        false,
    );
    for candidate in [&legacy, &target] {
        fixture
            .coordinator
            .prepare(candidate, 0, now())
            .expect("pending denied candidate");
    }
    let before = w16_recovery_facts(&fixture, None);
    let binding = target.foundation_binding().expect("target binding");
    for (field, replacement) in [
        ("schema_version", serde_json::json!(2)),
        (
            "namespace",
            serde_json::json!("Test:TEST_CODE_DIFFERENT_NAMESPACE"),
        ),
        (
            "application_decision_id",
            serde_json::json!(sha256_hex(b"TEST_CODE_WRONG_DECISION")),
        ),
        (
            "intent_id",
            serde_json::json!(sha256_hex(b"TEST_CODE_WRONG_INTENT")),
        ),
        ("unit_id", serde_json::json!("MU-W16-other")),
        (
            "occurrence_id",
            serde_json::json!(sha256_hex(b"TEST_CODE_WRONG_OCCURRENCE")),
        ),
        ("business_date", serde_json::json!("2026-07-29")),
        ("subject", serde_json::json!("Entity:TEST_CODE_OTHER")),
        (
            "delivery_subject_hash",
            serde_json::json!(sha256_hex(b"TEST_CODE_WRONG_SUBJECT")),
        ),
        ("audience", serde_json::json!("TEST_CODE_OTHER_AUDIENCE")),
        ("template_id", serde_json::json!("TEST_CODE_OTHER_TEMPLATE")),
        ("template_version", serde_json::json!("v2")),
        (
            "rendered_sha256",
            serde_json::json!(sha256_hex(b"TEST_CODE_WRONG_RENDERED")),
        ),
        (
            "source_evidence_fingerprint",
            serde_json::json!(sha256_hex(b"TEST_CODE_WRONG_SOURCE")),
        ),
        (
            "required_channel",
            serde_json::json!("TEST_CODE_OTHER_CHANNEL"),
        ),
    ] {
        let mut value = serde_json::to_value(binding).expect("binding fields");
        value[field] = replacement;
        let wrong: FoundationDeliveryBinding =
            serde_json::from_value(value).expect("typed mismatched binding");
        assert!(
            matches!(
                fixture.coordinator.reconcile_foundation_decision(
                    &target.decision_identity,
                    &wrong,
                    &append,
                    now()
                ),
                Err(DurableDeliveryError::PolicyMismatch(_))
            ),
            "must reject {field}"
        );
        assert_eq!(
            w16_recovery_facts(&fixture, None),
            before,
            "zero writes for {field}"
        );
        assert!(append.records.lock().expect("zero append").is_empty());
    }
    assert!(matches!(
        fixture.coordinator.reconcile_foundation_decision(
            &sha256_hex(b"TEST_CODE_MISSING"),
            binding,
            &append,
            now()
        ),
        Err(DurableDeliveryError::DecisionNotFound(_))
    ));
    assert!(matches!(
        fixture.coordinator.reconcile_foundation_decision(
            &legacy.decision_identity,
            binding,
            &append,
            now()
        ),
        Err(DurableDeliveryError::PolicyMismatch(_))
    ));
    assert_eq!(w16_recovery_facts(&fixture, None), before);
    assert!(append
        .records
        .lock()
        .expect("zero append for missing and legacy")
        .is_empty());
}

#[test]
fn w16_scoped_recovery_blocks_on_another_decisions_pending_audit_predecessor() {
    let fixture = Fixture::new("W16_SCOPED_PREDECESSOR");
    let other = w12_foundation_envelope("W16_PREDECESSOR_OTHER");
    let target = w12_foundation_envelope("W16_PREDECESSOR_TARGET");
    fixture
        .coordinator
        .prepare(&target, 0, now())
        .expect("freeze target denial");
    // A real append acknowledgement failure leaves the target payload pending
    // after its existing audit chain has been appended through the coordinator.
    let staging_append = EmptyAppendPort::new("DeliveryDisposition");
    assert!(matches!(fixture.coordinator.reconcile_foundation_decision(
        &target.decision_identity, target.foundation_binding().expect("binding"),
        &staging_append, now()
    ), Err(DurableDeliveryError::PolicyMismatch(reason))
        if reason.contains("DeliveryDisposition immutable append returned an empty reference")));
    let append = staging_append.inner;
    assert_eq!(
        fixture
            .coordinator
            .decision_state(&target.decision_identity)
            .expect("staged target"),
        DecisionState::RejectedAuditPending
    );
    fixture
        .coordinator
        .prepare(&other, 0, now())
        .expect("freeze pending predecessor decision");
    let connection =
        Connection::open(&fixture.database_path).expect("isolated predecessor fixture");
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .expect("enforce fixture predecessor reference");
    let predecessor: String = connection.query_row(
        "SELECT audit_identity FROM immutable_audit_outbox WHERE decision_identity=?1 ORDER BY rowid LIMIT 1",
        [&other.decision_identity], |row| row.get(0)
    ).expect("other pending predecessor");
    // Freeze the cross-decision dependency on INSERT. Existing immutable
    // payloads and their predecessor links are never rewritten or removed.
    let canonical = serde_json::to_vec(&serde_json::json!({
        "decision_identity": target.decision_identity,
        "predecessor_audit_identity": predecessor,
        "reason": "TEST_CODE_W16_EXTERNAL_AUDIT_DEPENDENCY",
    }))
    .expect("frozen dependency bytes");
    let digest = sha256_hex(&canonical);
    let audit_identity = super::model::stable_identity(
        "delivery-critical-audit-v1",
        &[
            &target.decision_identity,
            "NONE",
            "DecisionIdentityConflict",
            &digest,
        ],
    );
    connection
        .execute(
            "INSERT INTO immutable_audit_outbox(
           audit_identity,decision_identity,attempt_identity,audit_kind,
           predecessor_audit_identity,audit_canonical,audit_sha256,
           append_state,immutable_audit_ref,created_at
         ) VALUES (?1,?2,NULL,'DecisionIdentityConflict',?3,?4,?5,'Pending',NULL,?6)",
            params![
                audit_identity,
                target.decision_identity,
                predecessor,
                canonical,
                digest,
                now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
            ],
        )
        .expect("freeze cross-decision prerequisite with immutable triggers active");
    let before = w16_recovery_facts(&fixture, None);
    let records_before = append
        .records
        .lock()
        .expect("staged append observations")
        .clone();
    assert!(matches!(
        fixture.coordinator.reconcile_foundation_decision(
            &target.decision_identity,
            target.foundation_binding().expect("binding"),
            &append,
            now()
        ),
        Err(DurableDeliveryError::AuditPredecessorBlocked)
    ));
    assert_eq!(w16_recovery_facts(&fixture, None), before);
    assert_eq!(
        *append.records.lock().expect("blocked append observations"),
        records_before
    );
    fixture
        .coordinator
        .reconcile_all_pending(&append, now())
        .expect("global recovery can append the prerequisite");
    let records = append.records.lock().expect("global prerequisite evidence");
    assert!(records.contains_key(&predecessor));
    assert_eq!(
        records
            .get(&audit_identity)
            .expect("dependent audit appended")
            .canonical_bytes,
        canonical
    );
    drop(records);
    for candidate in [&other, &target] {
        assert_eq!(
            fixture
                .coordinator
                .decision_state(&candidate.decision_identity)
                .expect("globally recovered"),
            DecisionState::RejectedDurable
        );
    }
    let replay = fixture
        .coordinator
        .reconcile_foundation_decision(
            &target.decision_identity,
            target.foundation_binding().expect("binding"),
            &append,
            now(),
        )
        .expect("already appended external predecessor permits scoped query");
    assert_eq!(replay.progress_count, 0);
}

#[test]
fn w12_terminal_read_model_rejects_legacy_unbound_authority() {
    let fixture = Fixture::new("W12_LEGACY_AUTHORITY");
    let append = MemoryAppendPort::default();
    let candidate = establish_authoritative_delivered_projection(
        &fixture,
        "W12_LEGACY_AUTHORITY",
        &append,
        false,
    );

    assert!(fixture
        .coordinator
        .inspect_foundation_terminal(&candidate.decision_identity)
        .is_err());
}

fn w12_terminal_record(
    fixture: &Fixture,
    decision_identity: &str,
) -> Box<FoundationTerminalRecord> {
    match fixture
        .coordinator
        .inspect_foundation_terminal(decision_identity)
        .expect("W12 terminal query")
    {
        FoundationTerminalQuery::Terminal(record) => record,
        other => panic!("expected W12 terminal record, got {other:?}"),
    }
}

#[test]
fn w19_recovered_uncertain_terminal_is_readable_after_expiry() {
    let fixture = Fixture::new("W19_RECOVERED_UNCERTAIN_READ");
    let append = MemoryAppendPort::default();
    let candidate = w12_foundation_envelope("W19_RECOVERED_UNCERTAIN_READ");
    prepare_reserved(&fixture, &candidate, &append);
    let attempt = fixture
        .coordinator
        .begin_attempt(&candidate.decision_identity, 1, now())
        .expect("begin real W19 recovery attempt")
        .expect("W19 recovery attempt created");
    let recovered_at = now() + chrono::Duration::seconds(121);

    let summary = fixture
        .coordinator
        .reconcile_all_pending(&append, recovered_at)
        .expect("expire, classify and seal the W19 recovery attempt");
    assert!(summary.progress_count > 0);
    assert_eq!(summary.provider_calls, 0);
    assert_eq!(summary.sink_calls, 0);
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM sink_results"), 0);
    assert_eq!(
        fixture
            .coordinator
            .decision_state(&candidate.decision_identity)
            .expect("read recovered decision state"),
        DecisionState::UncertainManualReview
    );

    let terminal = match fixture
        .coordinator
        .inspect_foundation_terminal(&candidate.decision_identity)
        .expect("sealed recovery classification is a readable terminal")
    {
        FoundationTerminalQuery::Terminal(record) => record,
        other => panic!("expected recovered W19 terminal, got {other:?}"),
    };
    assert_eq!(
        terminal.disposition(),
        FoundationTerminalDisposition::Uncertain
    );
    assert_eq!(
        terminal.attempt_id(),
        Some(attempt.attempt_identity.as_str())
    );
    assert_eq!(
        sha256_hex(terminal.evidence_bytes()),
        terminal.evidence_sha256()
    );
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM sink_results"), 0);
}

#[test]
fn w19_recovered_uncertain_p01_terminal_uses_the_same_real_recovery_evidence() {
    let fixture = Fixture::new("W19_RECOVERED_UNCERTAIN_P01");
    let append = MemoryAppendPort::default();
    let candidate = w19_p01_recovery_envelope("P01");
    let (attempt, _) = w19_recover_uncertain_candidate(&fixture, &append, &candidate);

    let terminal = match fixture
        .coordinator
        .inspect_p01_dedicated_terminal("2026-08-18")
        .expect("read recovered P01 authority")
    {
        P01DedicatedTerminalQuery::Terminal(record) => record,
        other => panic!("expected recovered P01 terminal, got {other:?}"),
    };
    assert_eq!(
        terminal.legacy_decision_identity,
        candidate.decision_identity
    );
    assert_eq!(
        terminal.disposition,
        FoundationTerminalDisposition::Uncertain
    );
    assert_eq!(
        terminal.attempt_id.as_deref(),
        Some(attempt.attempt_identity.as_str())
    );
    assert_eq!(
        sha256_hex(&terminal.evidence_bytes),
        terminal.evidence_sha256
    );
    assert!(terminal.accepted_channel.is_none());
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM sink_results"), 0);
}

#[test]
fn w19_recovered_uncertain_late_nonauthoritative_result_and_restart_remain_read_only() {
    let fixture = Fixture::new("W19_RECOVERED_UNCERTAIN_LATE_RESTART");
    let append = MemoryAppendPort::default();
    let candidate = w12_foundation_envelope("W19_RECOVERED_UNCERTAIN_LATE_RESTART");
    let (attempt, recovered_at) = w19_recover_uncertain_candidate(&fixture, &append, &candidate);
    let before_late = match fixture
        .coordinator
        .inspect_foundation_terminal(&candidate.decision_identity)
        .expect("read recovery before late result")
    {
        FoundationTerminalQuery::Terminal(record) => record,
        other => panic!("expected recovered terminal before late result, got {other:?}"),
    };

    fixture
        .coordinator
        .record_sink_result(
            &attempt.attempt_identity,
            attempt.fence_token,
            AuthoritativeSinkResult::Accepted(receipt(recovered_at)),
            recovered_at,
        )
        .expect("persist a real late non-authoritative result");
    fixture
        .coordinator
        .reconcile_all_pending(&append, recovered_at)
        .expect("seal late-result audits");
    assert_eq!(
        fixture.query_i64(
            "SELECT COUNT(*) FROM sink_results
             WHERE decision_identity=(SELECT decision_identity FROM delivery_decisions LIMIT 1)
               AND authoritative_for_state=0 AND late_after_fence=1"
        ),
        1
    );

    let facts_before_reads = w16_recovery_facts(&fixture, Some(&candidate.decision_identity));
    let append_before_reads = append.records.lock().expect("append records").clone();
    let after_late = match fixture
        .coordinator
        .inspect_foundation_terminal(&candidate.decision_identity)
        .expect("late non-authoritative result cannot replace recovery")
    {
        FoundationTerminalQuery::Terminal(record) => record,
        other => panic!("expected recovered terminal after late result, got {other:?}"),
    };
    let restarted = fixture.second_coordinator("W19_RECOVERED_UNCERTAIN_RESTART");
    let after_restart = match restarted
        .inspect_foundation_terminal(&candidate.decision_identity)
        .expect("read same recovery after restart")
    {
        FoundationTerminalQuery::Terminal(record) => record,
        other => panic!("expected recovered terminal after restart, got {other:?}"),
    };
    assert_eq!(after_late, before_late);
    assert_eq!(after_restart, before_late);
    assert_eq!(
        restarted
            .decision_state(&candidate.decision_identity)
            .expect("restarted recovery state"),
        DecisionState::UncertainManualReview
    );
    assert_eq!(
        w16_recovery_facts(&fixture, Some(&candidate.decision_identity)),
        facts_before_reads
    );
    assert_eq!(
        *append.records.lock().expect("unchanged append records"),
        append_before_reads
    );
}

#[test]
fn w19_recovered_uncertain_pending_audit_remains_pending_seal() {
    let fixture = Fixture::new("W19_RECOVERED_UNCERTAIN_PENDING_SEAL");
    let initial_append = MemoryAppendPort::default();
    let candidate = w12_foundation_envelope("W19_RECOVERED_UNCERTAIN_PENDING_SEAL");
    prepare_reserved(&fixture, &candidate, &initial_append);
    fixture
        .coordinator
        .begin_attempt(&candidate.decision_identity, 1, now())
        .expect("begin pending-seal recovery attempt")
        .expect("pending-seal attempt created");
    let empty_ref = EmptyAppendPort::new("FenceRevoked");
    assert!(matches!(
        fixture.coordinator.reconcile_all_pending(
            &empty_ref,
            now() + chrono::Duration::seconds(121)
        ),
        Err(DurableDeliveryError::PolicyMismatch(reason))
            if reason.contains("immutable append returned an empty reference")
    ));
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM sink_results"), 0);
    assert!(matches!(
        fixture
            .coordinator
            .inspect_foundation_terminal(&candidate.decision_identity)
            .expect("unsealed recovery remains readable as pending"),
        FoundationTerminalQuery::PendingSeal {
            state: DecisionState::UncertainAuditPending
        }
    ));
}

#[test]
fn w19_recovered_uncertain_rejects_missing_duplicate_wrong_binding_tamper_and_overflow() {
    for corruption in [
        "missing",
        "duplicate",
        "wrong-audit-binding",
        "tampered-canonical",
        "disposition-reference",
        "fence-overflow",
    ] {
        let fixture = Fixture::new(&format!("W19_RECOVERED_UNCERTAIN_{corruption}"));
        let append = MemoryAppendPort::default();
        let candidate = w12_foundation_envelope(&format!("W19_RECOVERED_{corruption}"));
        w19_recover_uncertain_candidate(&fixture, &append, &candidate);
        let connection = Connection::open(&fixture.database_path)
            .expect("open isolated W19 recovery corruption database");
        match corruption {
            "missing" => {
                connection
                    .execute_batch("DROP TRIGGER immutable_attempt_event_delete;")
                    .expect("remove isolated event delete guard");
                assert_eq!(
                    connection
                        .execute(
                            "DELETE FROM delivery_attempt_events
                             WHERE decision_identity=?1 AND event_kind='RecoveryClassified'",
                            [candidate.decision_identity.as_str()],
                        )
                        .expect("remove one isolated recovery event"),
                    1
                );
            }
            "duplicate" => {
                connection
                    .execute_batch("DROP TRIGGER immutable_attempt_event_update;")
                    .expect("remove isolated event update guard");
                assert_eq!(
                    connection
                        .execute(
                            "UPDATE delivery_attempt_events SET event_kind='FenceRevoked'
                             WHERE decision_identity=?1 AND event_kind='RecoveryClassified'",
                            [candidate.decision_identity.as_str()],
                        )
                        .expect("duplicate isolated recovery kind"),
                    1
                );
            }
            "wrong-audit-binding" => {
                connection
                    .execute_batch("DROP TRIGGER immutable_outbox_payload_update;")
                    .expect("remove isolated audit update guard");
                assert_eq!(
                    connection
                        .execute(
                            "UPDATE immutable_audit_outbox SET audit_kind='FenceRevoked'
                             WHERE decision_identity=?1 AND audit_kind='RecoveryClassified'",
                            [candidate.decision_identity.as_str()],
                        )
                        .expect("rebind isolated recovery audit"),
                    1
                );
            }
            "tampered-canonical" => {
                connection
                    .execute_batch("DROP TRIGGER immutable_attempt_event_update;")
                    .expect("remove isolated event update guard");
                assert_eq!(
                    connection
                        .execute(
                            "UPDATE delivery_attempt_events SET event_canonical=x'7b7d'
                             WHERE decision_identity=?1 AND event_kind='FenceRevoked'",
                            [candidate.decision_identity.as_str()],
                        )
                        .expect("tamper isolated recovery canonical"),
                    1
                );
            }
            "disposition-reference" => {
                connection
                    .execute_batch("DROP TRIGGER immutable_disposition_payload_update;")
                    .expect("remove isolated disposition update guard");
                assert_eq!(
                    connection
                        .execute(
                            "UPDATE delivery_disposition_payloads SET disposition_sha256=?1
                             WHERE decision_identity=?2",
                            params![
                                sha256_hex(b"TEST_CODE_W19_WRONG_DISPOSITION_REFERENCE"),
                                candidate.decision_identity
                            ],
                        )
                        .expect("tamper isolated recovery disposition reference"),
                    1
                );
            }
            "fence-overflow" => {
                assert_eq!(
                    connection
                        .execute(
                            "UPDATE delivery_attempts SET fence_token=9223372036854775807
                             WHERE decision_identity=?1",
                            [candidate.decision_identity.as_str()],
                        )
                        .expect("inject isolated fence overflow"),
                    1
                );
            }
            _ => unreachable!(),
        }
        drop(connection);

        let error = fixture
            .coordinator
            .inspect_foundation_terminal(&candidate.decision_identity)
            .expect_err("corrupt recovery evidence must be rejected");
        assert!(matches!(error, DurableDeliveryError::PolicyMismatch(_)));
        assert!(!error.to_string().contains("TEST_CODE"));
        assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM sink_results"), 0);
    }
}

#[test]
fn w19_recovered_uncertain_rejects_broken_fence_audit_predecessor() {
    let fixture = Fixture::new("W19_RECOVERED_UNCERTAIN_FENCE_PREDECESSOR");
    let append = MemoryAppendPort::default();
    let candidate = w12_foundation_envelope("W19_RECOVERED_UNCERTAIN_FENCE_PREDECESSOR");
    w19_recover_uncertain_candidate(&fixture, &append, &candidate);
    let connection = Connection::open(&fixture.database_path)
        .expect("open isolated W19 fence predecessor corruption database");
    connection
        .execute_batch("DROP TRIGGER immutable_outbox_payload_update;")
        .expect("remove isolated audit immutability guard");
    assert_eq!(
        connection
            .execute(
                "UPDATE immutable_audit_outbox SET predecessor_audit_identity=audit_identity
                 WHERE decision_identity=?1 AND audit_kind='FenceRevoked'",
                [candidate.decision_identity.as_str()],
            )
            .expect("inject isolated FenceRevoked predecessor self-loop"),
        1
    );
    drop(connection);

    let error = fixture
        .coordinator
        .inspect_foundation_terminal(&candidate.decision_identity)
        .expect_err("FenceRevoked audit predecessor self-loop must be rejected");
    assert!(matches!(error, DurableDeliveryError::PolicyMismatch(_)));
    assert!(!error.to_string().contains("TEST_CODE"));
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM sink_results"), 0);
}

#[test]
fn w19_recovered_uncertain_logical_predecessor_chain_ignores_physical_rowid_order() {
    let fixture = Fixture::new("W19_RECOVERED_UNCERTAIN_ROWID_REORDER");
    let append = MemoryAppendPort::default();
    let candidate = w12_foundation_envelope("W19_RECOVERED_UNCERTAIN_ROWID_REORDER");
    w19_recover_uncertain_candidate(&fixture, &append, &candidate);
    let before = fixture
        .query_strings("SELECT audit_identity FROM immutable_audit_outbox ORDER BY rowid ASC");
    let expected_terminal = fixture
        .coordinator
        .inspect_foundation_terminal(&candidate.decision_identity)
        .expect("read terminal before physical rowid reorder");
    let connection =
        Connection::open(&fixture.database_path).expect("open isolated W19 rowid reorder database");
    assert_eq!(
        connection
            .execute(
                "UPDATE immutable_audit_outbox SET rowid=-rowid WHERE decision_identity=?1",
                [candidate.decision_identity.as_str()],
            )
            .expect("reverse isolated audit physical rowids"),
        before.len()
    );
    drop(connection);
    let after = fixture
        .query_strings("SELECT audit_identity FROM immutable_audit_outbox ORDER BY rowid ASC");
    let mut reversed = before;
    reversed.reverse();
    assert_eq!(
        after, reversed,
        "the physical row order must actually change"
    );

    let actual_terminal = fixture
        .coordinator
        .inspect_foundation_terminal(&candidate.decision_identity)
        .expect("logical predecessor chain survives physical rowid reorder");
    assert_eq!(actual_terminal, expected_terminal);
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM sink_results"), 0);
}

#[test]
fn w19_recovered_uncertain_allows_a_real_intervening_conflict_audit() {
    let fixture = Fixture::new("W19_RECOVERED_UNCERTAIN_INTERVENING_CONFLICT");
    let append = MemoryAppendPort::default();
    let candidate = w12_foundation_envelope("W19_RECOVERED_UNCERTAIN_INTERVENING_CONFLICT");
    prepare_reserved(&fixture, &candidate, &append);
    fixture
        .coordinator
        .begin_attempt(&candidate.decision_identity, 1, now())
        .expect("begin real recovery attempt")
        .expect("real recovery attempt created");
    let mut conflicting = candidate.clone();
    conflicting.replace_content_preserving_identity(
        b"TEST_CODE_W19_INTERVENING_CONFLICTING_BODY".to_vec(),
    );
    assert!(matches!(
        fixture
            .coordinator
            .prepare(&conflicting, 1, now() + chrono::Duration::seconds(1)),
        Err(DurableDeliveryError::DecisionIdentityConflict { .. })
    ));
    fixture
        .coordinator
        .reconcile_all_pending(&append, now() + chrono::Duration::seconds(121))
        .expect("seal recovery after real intervening conflict audit");
    assert_eq!(
        fixture.query_i64(
            "SELECT COUNT(*) FROM immutable_audit_outbox fence
             JOIN immutable_audit_outbox conflict
               ON conflict.audit_identity=fence.predecessor_audit_identity
             WHERE fence.audit_kind='FenceRevoked'
               AND conflict.audit_kind='DecisionIdentityConflict'"
        ),
        1
    );

    let terminal = fixture
        .coordinator
        .inspect_foundation_terminal(&candidate.decision_identity)
        .expect("logical recovery chain accepts real intervening conflict audit");
    assert!(matches!(terminal, FoundationTerminalQuery::Terminal(record)
        if record.disposition() == FoundationTerminalDisposition::Uncertain));
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM sink_results"), 0);
}

#[test]
fn w19_recovered_uncertain_rejects_skipped_intervening_audit() {
    let fixture = Fixture::new("W19_RECOVERED_UNCERTAIN_SKIPPED_CONFLICT");
    let append = MemoryAppendPort::default();
    let candidate = w12_foundation_envelope("W19_RECOVERED_UNCERTAIN_SKIPPED_CONFLICT");
    prepare_reserved(&fixture, &candidate, &append);
    fixture
        .coordinator
        .begin_attempt(&candidate.decision_identity, 1, now())
        .expect("begin real recovery attempt")
        .expect("real recovery attempt created");
    let mut conflicting = candidate.clone();
    conflicting.replace_content_preserving_identity(
        b"TEST_CODE_W19_SKIPPED_INTERVENING_CONFLICTING_BODY".to_vec(),
    );
    assert!(matches!(
        fixture
            .coordinator
            .prepare(&conflicting, 1, now() + chrono::Duration::seconds(1)),
        Err(DurableDeliveryError::DecisionIdentityConflict { .. })
    ));
    fixture
        .coordinator
        .reconcile_all_pending(&append, now() + chrono::Duration::seconds(121))
        .expect("seal recovery after real intervening conflict audit");
    assert_eq!(
        fixture.query_i64(
            "SELECT COUNT(*) FROM immutable_audit_outbox fence
             JOIN immutable_audit_outbox conflict
               ON conflict.audit_identity=fence.predecessor_audit_identity
             WHERE fence.decision_identity=conflict.decision_identity
               AND fence.audit_kind='FenceRevoked'
               AND conflict.audit_kind='DecisionIdentityConflict'
               AND conflict.predecessor_audit_identity IS NOT NULL"
        ),
        1,
        "the real conflict must initially be the fence predecessor"
    );

    let connection = Connection::open(&fixture.database_path)
        .expect("open isolated W19 skipped-conflict corruption database");
    connection
        .execute_batch("DROP TRIGGER immutable_outbox_payload_update;")
        .expect("remove isolated audit payload guard");
    assert_eq!(
        connection
            .execute(
                "UPDATE immutable_audit_outbox
                 SET predecessor_audit_identity=(
                   SELECT conflict.predecessor_audit_identity
                   FROM immutable_audit_outbox conflict
                   WHERE conflict.decision_identity=?1
                     AND conflict.audit_kind='DecisionIdentityConflict')
                 WHERE decision_identity=?1 AND audit_kind='FenceRevoked'",
                [candidate.decision_identity.as_str()],
            )
            .expect("skip the real intervening conflict in the isolated fence chain"),
        1
    );
    drop(connection);
    assert_eq!(
        fixture.query_i64(
            "SELECT COUNT(*) FROM immutable_audit_outbox fence
             JOIN immutable_audit_outbox conflict
               ON conflict.decision_identity=fence.decision_identity
              AND conflict.audit_kind='DecisionIdentityConflict'
              AND conflict.predecessor_audit_identity=fence.predecessor_audit_identity
             WHERE fence.audit_kind='FenceRevoked'
               AND fence.audit_identity<>conflict.audit_identity"
        ),
        1,
        "the tampered fence and skipped conflict must be distinct successors of one predecessor"
    );

    let error = fixture
        .coordinator
        .inspect_foundation_terminal(&candidate.decision_identity)
        .expect_err("a skipped intervening audit must be rejected");
    assert!(matches!(error, DurableDeliveryError::PolicyMismatch(_)));
    assert!(!error.to_string().contains("TEST_CODE"));
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM sink_results"), 0);
}

#[test]
fn w19_recovered_uncertain_accepts_recovery_from_real_rejected_genesis_retry() {
    struct PanicAfterAttemptSink;
    impl AuthoritativeSinkPort for PanicAfterAttemptSink {
        fn sink_identity(&self) -> &str {
            "TEST_CODE_W19_PANIC_AFTER_ATTEMPT_SINK"
        }

        fn deliver(&self, _: &AuthoritativeDeliveryRequest) -> AuthoritativeSinkResult {
            panic!("TEST_CODE_W19_SIMULATED_PROCESS_EXIT_AFTER_ATTEMPT")
        }
    }

    let fixture = Fixture::new("W19_RECOVERED_UNCERTAIN_REJECTED_GENESIS");
    let append = MemoryAppendPort::default();
    let candidate = w12_foundation_envelope("W19_RECOVERED_UNCERTAIN_REJECTED_GENESIS");
    let denied = fixture
        .coordinator
        .prepare(&candidate, 0, now())
        .expect("prepare real rejected genesis");
    assert_eq!(denied.state, DecisionState::RejectedAuditPending);
    reconcile_terminal(
        &fixture,
        &append,
        DecisionState::RejectedDurable,
        &candidate.decision_identity,
    );
    fixture
        .coordinator
        .authorize_rejected_retry(&candidate.decision_identity)
        .expect("authorize real rejected decision retry");

    let attempt_started_at = now() + chrono::Duration::seconds(10);
    let recovered_at = attempt_started_at + chrono::Duration::seconds(121);
    let panic_sink: AuthoritativeSink = Arc::new(PanicAfterAttemptSink);
    let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        fixture.coordinator.resume_deliverable(
            &candidate.decision_identity,
            &[panic_sink],
            attempt_started_at,
        )
    }));
    assert!(interrupted.is_err());
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM delivery_attempts"),
        1
    );
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM sink_results"), 0);

    let summary = fixture
        .coordinator
        .reconcile_all_pending(&append, recovered_at)
        .expect("recover the expired authorized retry");
    assert!(summary.progress_count > 0);
    assert_eq!(summary.sink_calls, 0);
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM sink_results"), 0);
    let recovered = fixture
        .coordinator
        .inspect_foundation_terminal(&candidate.decision_identity)
        .expect("rejected-genesis retry recovery is readable");
    assert!(
        matches!(recovered, FoundationTerminalQuery::Terminal(record)
        if record.disposition() == FoundationTerminalDisposition::Uncertain)
    );
}

#[test]
fn w19_recovered_uncertain_rejects_truncated_wrong_decision_or_unsealed_predecessor() {
    for corruption in ["truncated", "wrong-decision", "unsealed"] {
        let fixture = Fixture::new(&format!("W19_RECOVERED_UNCERTAIN_PREDECESSOR_{corruption}"));
        let append = MemoryAppendPort::default();
        let candidate =
            w12_foundation_envelope(&format!("W19_RECOVERED_UNCERTAIN_PREDECESSOR_{corruption}"));
        w19_recover_uncertain_candidate(&fixture, &append, &candidate);
        let other = (corruption == "wrong-decision").then(|| {
            let other = envelope(
                "W19_RECOVERED_UNCERTAIN_OTHER_DECISION",
                PushKind::CandidateTriggered,
                DeliverySubKind::None,
                "2026-07-30",
                false,
            );
            fixture
                .coordinator
                .prepare(&other, 1, now())
                .expect("prepare isolated other decision");
            other
        });
        let connection = Connection::open(&fixture.database_path)
            .expect("open isolated W19 predecessor corruption database");
        connection
            .execute_batch("DROP TRIGGER immutable_outbox_payload_update;")
            .expect("remove isolated audit payload guard");
        let changed = match corruption {
            "truncated" => connection.execute(
                "UPDATE immutable_audit_outbox SET predecessor_audit_identity=NULL
                 WHERE decision_identity=?1 AND audit_kind='LeaseGranted'",
                [candidate.decision_identity.as_str()],
            ),
            "wrong-decision" => connection.execute(
                "UPDATE immutable_audit_outbox SET decision_identity=?1
                 WHERE audit_identity=(
                   SELECT predecessor_audit_identity FROM immutable_audit_outbox
                   WHERE decision_identity=?2 AND audit_kind='FenceRevoked')",
                params![
                    other.as_ref().expect("other decision").decision_identity,
                    candidate.decision_identity
                ],
            ),
            "unsealed" => connection.execute(
                "UPDATE immutable_audit_outbox SET append_state='Pending',immutable_audit_ref=NULL
                 WHERE decision_identity=?1 AND audit_kind='LeaseGranted'",
                [candidate.decision_identity.as_str()],
            ),
            _ => unreachable!(),
        }
        .expect("inject isolated predecessor corruption");
        assert_eq!(changed, 1);
        drop(connection);

        let error = fixture
            .coordinator
            .inspect_foundation_terminal(&candidate.decision_identity)
            .expect_err("broken logical predecessor chain must be rejected");
        assert!(matches!(error, DurableDeliveryError::PolicyMismatch(_)));
        assert!(!error.to_string().contains("TEST_CODE"));
        assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM sink_results"), 0);
    }
}

#[test]
fn w19_recovered_uncertain_rejects_any_authoritative_sink_conflict() {
    let fixture = Fixture::new("W19_RECOVERED_UNCERTAIN_AUTHORITY_CONFLICT");
    let append = MemoryAppendPort::default();
    let candidate = w12_foundation_envelope("W19_RECOVERED_UNCERTAIN_AUTHORITY_CONFLICT");
    let (attempt, recovered_at) = w19_recover_uncertain_candidate(&fixture, &append, &candidate);
    fixture
        .coordinator
        .record_sink_result(
            &attempt.attempt_identity,
            attempt.fence_token,
            AuthoritativeSinkResult::Accepted(receipt(recovered_at)),
            recovered_at,
        )
        .expect("persist actual late Accepted before isolated authority corruption");
    let connection = Connection::open(&fixture.database_path)
        .expect("open isolated W19 authoritative conflict database");
    connection
        .execute_batch("DROP TRIGGER immutable_sink_result_update;")
        .expect("remove isolated sink immutability guard");
    assert_eq!(
        connection
            .execute(
                "UPDATE sink_results
                 SET authoritative_for_state=1,late_after_fence=0,late_receipt_audit_identity=NULL
                 WHERE decision_identity=?1 AND result_kind='Accepted'",
                [candidate.decision_identity.as_str()],
            )
            .expect("inject isolated authoritative Accepted conflict"),
        1
    );
    drop(connection);

    assert!(matches!(
        fixture
            .coordinator
            .inspect_foundation_terminal(&candidate.decision_identity),
        Err(DurableDeliveryError::PolicyMismatch(reason))
            if reason == "foundation uncertain terminal has conflicting authoritative sources"
    ));
}

#[test]
fn w12_terminal_read_model_preserves_all_durable_terminal_dispositions() {
    let rejected_fixture = Fixture::new("W12_REJECTED_TERMINAL");
    let rejected_append = MemoryAppendPort::default();
    let rejected = w12_foundation_envelope("REJECTED_TERMINAL");
    prepare_reserved(&rejected_fixture, &rejected, &rejected_append);
    let rejected_sink = StaticSink::new(AuthoritativeSinkResult::Rejected(rejection(now(), false)));
    let rejected_sinks: Vec<AuthoritativeSink> = vec![rejected_sink];
    rejected_fixture
        .coordinator
        .resume_deliverable(&rejected.decision_identity, &rejected_sinks, now())
        .expect("record W12 rejection");
    reconcile_terminal(
        &rejected_fixture,
        &rejected_append,
        DecisionState::RejectedDurable,
        &rejected.decision_identity,
    );
    let rejected_terminal = w12_terminal_record(&rejected_fixture, &rejected.decision_identity);
    assert_eq!(
        rejected_terminal.disposition(),
        FoundationTerminalDisposition::Rejected
    );
    assert!(rejected_terminal.attempt_id().is_some());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(rejected_terminal.evidence_bytes())
            .expect("typed rejection")["kind"],
        "Rejected"
    );

    let denial_fixture = Fixture::new("W12_DENIAL_TERMINAL");
    let denial_append = MemoryAppendPort::default();
    let denial = w12_foundation_envelope("DENIAL_TERMINAL");
    denial_fixture
        .coordinator
        .prepare(&denial, 0, now())
        .expect("record W12 pre-attempt denial");
    reconcile_terminal(
        &denial_fixture,
        &denial_append,
        DecisionState::RejectedDurable,
        &denial.decision_identity,
    );
    let denial_terminal = w12_terminal_record(&denial_fixture, &denial.decision_identity);
    assert_eq!(
        denial_terminal.disposition(),
        FoundationTerminalDisposition::Rejected
    );
    assert!(denial_terminal.attempt_id().is_none());

    let uncertain_fixture = Fixture::new("W12_UNCERTAIN_TERMINAL");
    let uncertain_append = MemoryAppendPort::default();
    let uncertain = w12_foundation_envelope("UNCERTAIN_TERMINAL");
    prepare_reserved(&uncertain_fixture, &uncertain, &uncertain_append);
    let uncertain_sink = StaticSink::new(AuthoritativeSinkResult::Uncertain(uncertainty(now())));
    let uncertain_sinks: Vec<AuthoritativeSink> = vec![uncertain_sink];
    uncertain_fixture
        .coordinator
        .resume_deliverable(&uncertain.decision_identity, &uncertain_sinks, now())
        .expect("record W12 uncertainty");
    reconcile_terminal(
        &uncertain_fixture,
        &uncertain_append,
        DecisionState::UncertainManualReview,
        &uncertain.decision_identity,
    );
    let uncertain_terminal = w12_terminal_record(&uncertain_fixture, &uncertain.decision_identity);
    assert_eq!(
        uncertain_terminal.disposition(),
        FoundationTerminalDisposition::Uncertain
    );
    assert!(uncertain_terminal.attempt_id().is_some());

    let manual_rejected_fixture = Fixture::new("W12_MANUAL_REJECTED_TERMINAL");
    let manual_rejected_append = MemoryAppendPort::default();
    let manual_rejected = w12_foundation_envelope("MANUAL_REJECTED_TERMINAL");
    prepare_reserved(
        &manual_rejected_fixture,
        &manual_rejected,
        &manual_rejected_append,
    );
    let uncertain_sink = StaticSink::new(AuthoritativeSinkResult::Uncertain(uncertainty(now())));
    let uncertain_sinks: Vec<AuthoritativeSink> = vec![uncertain_sink];
    manual_rejected_fixture
        .coordinator
        .resume_deliverable(&manual_rejected.decision_identity, &uncertain_sinks, now())
        .expect("record uncertainty before manual rejection");
    reconcile_terminal(
        &manual_rejected_fixture,
        &manual_rejected_append,
        DecisionState::UncertainManualReview,
        &manual_rejected.decision_identity,
    );
    manual_rejected_fixture
        .coordinator
        .resolve_uncertain(
            &ManualResolutionCommand {
                decision_identity: manual_rejected.decision_identity.clone(),
                disposition: ManualDisposition::Rejected,
                operator_identity: "TEST_CODE_W12_OPERATOR".to_owned(),
                reason: "TEST_CODE_W12_CONFIRMED_NOT_DELIVERED".to_owned(),
                external_evidence: b"TEST_CODE_W12_MANUAL_REJECTION_EVIDENCE".to_vec(),
                resolved_at: now(),
            },
            &manual_rejected_append,
        )
        .expect("record W12 manual rejection");
    reconcile_terminal(
        &manual_rejected_fixture,
        &manual_rejected_append,
        DecisionState::ManualResolvedRejected,
        &manual_rejected.decision_identity,
    );
    let manual_rejected_terminal =
        w12_terminal_record(&manual_rejected_fixture, &manual_rejected.decision_identity);
    assert_eq!(
        manual_rejected_terminal.disposition(),
        FoundationTerminalDisposition::ManualNotDelivered
    );
    assert!(manual_rejected_terminal.attempt_id().is_some());

    let manual_accepted_fixture = Fixture::new("W12_MANUAL_ACCEPTED_TERMINAL");
    let manual_accepted_append = MemoryAppendPort::default();
    let manual_accepted = w12_foundation_envelope("MANUAL_ACCEPTED_TERMINAL");
    prepare_reserved(
        &manual_accepted_fixture,
        &manual_accepted,
        &manual_accepted_append,
    );
    let uncertain_sink = StaticSink::new(AuthoritativeSinkResult::Uncertain(uncertainty(now())));
    let uncertain_sinks: Vec<AuthoritativeSink> = vec![uncertain_sink];
    manual_accepted_fixture
        .coordinator
        .resume_deliverable(&manual_accepted.decision_identity, &uncertain_sinks, now())
        .expect("record uncertainty before manual acceptance");
    reconcile_terminal(
        &manual_accepted_fixture,
        &manual_accepted_append,
        DecisionState::UncertainManualReview,
        &manual_accepted.decision_identity,
    );
    manual_accepted_fixture
        .coordinator
        .resolve_uncertain(
            &ManualResolutionCommand {
                decision_identity: manual_accepted.decision_identity.clone(),
                disposition: ManualDisposition::Accepted {
                    receipt: Some(receipt(now())),
                },
                operator_identity: "TEST_CODE_W12_OPERATOR".to_owned(),
                reason: "TEST_CODE_W12_CONFIRMED_DELIVERED".to_owned(),
                external_evidence: b"TEST_CODE_W12_MANUAL_ACCEPTANCE_EVIDENCE".to_vec(),
                resolved_at: now(),
            },
            &manual_accepted_append,
        )
        .expect("record W12 manual acceptance");
    reconcile_terminal(
        &manual_accepted_fixture,
        &manual_accepted_append,
        DecisionState::Delivered,
        &manual_accepted.decision_identity,
    );
    let manual_accepted_terminal =
        w12_terminal_record(&manual_accepted_fixture, &manual_accepted.decision_identity);
    assert_eq!(
        manual_accepted_terminal.disposition(),
        FoundationTerminalDisposition::ManualAccepted
    );
    assert!(manual_accepted_terminal.attempt_id().is_some());
}

#[test]
fn w12_terminal_read_model_fails_closed_on_corrupt_disposition_join() {
    let fixture = Fixture::new("W12_CORRUPT_TERMINAL");
    let append = MemoryAppendPort::default();
    let candidate = w12_foundation_envelope("CORRUPT_TERMINAL");
    prepare_reserved(&fixture, &candidate, &append);
    let sink = StaticSink::new(AuthoritativeSinkResult::Rejected(rejection(now(), false)));
    let sinks: Vec<AuthoritativeSink> = vec![sink];
    fixture
        .coordinator
        .resume_deliverable(&candidate.decision_identity, &sinks, now())
        .expect("record W12 rejection before corruption");
    reconcile_terminal(
        &fixture,
        &append,
        DecisionState::RejectedDurable,
        &candidate.decision_identity,
    );

    let connection = Connection::open(&fixture.database_path).expect("open corruption connection");
    connection
        .execute_batch("DROP TRIGGER immutable_disposition_payload_update;")
        .expect("drop TEST_CODE immutable trigger");
    connection
        .execute(
            "UPDATE delivery_disposition_payloads SET disposition_sha256=?1 \
             WHERE decision_identity=?2",
            params![
                sha256_hex(b"TEST_CODE_W12_CORRUPT"),
                candidate.decision_identity
            ],
        )
        .expect("inject TEST_CODE disposition corruption");

    assert!(fixture
        .coordinator
        .inspect_foundation_terminal(&candidate.decision_identity)
        .is_err());
}

fn w12_accepted_terminal_fixture(label: &str) -> (Fixture, DeliveryEnvelope) {
    let fixture = Fixture::new(label);
    let append = MemoryAppendPort::default();
    let candidate = w12_foundation_envelope(label);
    prepare_reserved(&fixture, &candidate, &append);
    let sink = StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(now())));
    let sinks: Vec<AuthoritativeSink> = vec![sink];
    fixture
        .coordinator
        .resume_deliverable(&candidate.decision_identity, &sinks, now())
        .expect("record W12 accepted result before corruption");
    reconcile_terminal(
        &fixture,
        &append,
        DecisionState::Delivered,
        &candidate.decision_identity,
    );
    let terminal = w12_terminal_record(&fixture, &candidate.decision_identity);
    assert_eq!(
        terminal.disposition(),
        FoundationTerminalDisposition::Accepted
    );
    (fixture, candidate)
}

#[test]
fn w12_terminal_read_model_fails_closed_on_every_authority_join_corruption() {
    let (envelope_fixture, envelope) = w12_accepted_terminal_fixture("W12_CORRUPT_ENVELOPE");
    {
        let connection = Connection::open(&envelope_fixture.database_path)
            .expect("open envelope corruption connection");
        connection
            .execute_batch("DROP TRIGGER immutable_decision_envelope_update;")
            .expect("drop TEST_CODE immutable envelope trigger");
        connection
            .execute(
                "UPDATE delivery_decisions SET envelope_sha256=?1 WHERE decision_identity=?2",
                params![
                    sha256_hex(b"TEST_CODE_W12_CORRUPT_ENVELOPE"),
                    envelope.decision_identity
                ],
            )
            .expect("inject TEST_CODE envelope corruption");
    }
    assert!(envelope_fixture
        .coordinator
        .inspect_foundation_terminal(&envelope.decision_identity)
        .is_err());

    let (result_fixture, result) = w12_accepted_terminal_fixture("W12_CORRUPT_RESULT");
    {
        let connection = Connection::open(&result_fixture.database_path)
            .expect("open result corruption connection");
        connection
            .execute_batch("DROP TRIGGER immutable_sink_result_update;")
            .expect("drop TEST_CODE immutable result trigger");
        connection
            .execute(
                "UPDATE sink_results SET result_sha256=?1 WHERE decision_identity=?2",
                params![
                    sha256_hex(b"TEST_CODE_W12_CORRUPT_RESULT"),
                    result.decision_identity
                ],
            )
            .expect("inject TEST_CODE result corruption");
    }
    assert!(result_fixture
        .coordinator
        .inspect_foundation_terminal(&result.decision_identity)
        .is_err());

    let (audit_fixture, audit) = w12_accepted_terminal_fixture("W12_CORRUPT_AUDIT");
    {
        let connection = Connection::open(&audit_fixture.database_path)
            .expect("open audit corruption connection");
        connection
            .execute_batch("DROP TRIGGER immutable_sink_result_update;")
            .expect("drop TEST_CODE immutable audit trigger");
        connection
            .execute(
                "UPDATE sink_results SET frozen_delivery_audit_sha256=?1 \
                 WHERE decision_identity=?2",
                params![
                    sha256_hex(b"TEST_CODE_W12_CORRUPT_AUDIT"),
                    audit.decision_identity
                ],
            )
            .expect("inject TEST_CODE delivery audit corruption");
    }
    assert!(audit_fixture
        .coordinator
        .inspect_foundation_terminal(&audit.decision_identity)
        .is_err());

    let (attempt_fixture, attempt) = w12_accepted_terminal_fixture("W12_CORRUPT_ATTEMPT");
    {
        let connection = Connection::open(&attempt_fixture.database_path)
            .expect("open attempt corruption connection");
        connection
            .execute(
                "UPDATE delivery_attempts SET fence_token=fence_token+1000 \
                 WHERE decision_identity=?1",
                params![attempt.decision_identity],
            )
            .expect("inject TEST_CODE attempt/fence corruption");
    }
    assert!(attempt_fixture
        .coordinator
        .inspect_foundation_terminal(&attempt.decision_identity)
        .is_err());
}

#[test]
fn w12_terminal_read_model_rejects_accepted_receipt_for_wrong_required_channel() {
    let fixture = Fixture::new("W12_ACCEPTED_CHANNEL_MISMATCH");
    let append = MemoryAppendPort::default();
    let candidate = w12_foundation_envelope("ACCEPTED_CHANNEL_MISMATCH");
    prepare_reserved(&fixture, &candidate, &append);
    let mut wrong_receipt = receipt(now());
    wrong_receipt.channel = "TEST_CODE_WRONG_CHANNEL".to_owned();
    let sink = StaticSink::new(AuthoritativeSinkResult::Accepted(wrong_receipt));
    let sinks: Vec<AuthoritativeSink> = vec![sink];
    fixture
        .coordinator
        .resume_deliverable(&candidate.decision_identity, &sinks, now())
        .expect("record mismatched accepted receipt");
    reconcile_terminal(
        &fixture,
        &append,
        DecisionState::Delivered,
        &candidate.decision_identity,
    );

    assert!(fixture
        .coordinator
        .inspect_foundation_terminal(&candidate.decision_identity)
        .is_err());
}

#[test]
fn w12_terminal_read_model_rejects_manual_receipt_for_wrong_required_channel() {
    let fixture = Fixture::new("W12_MANUAL_CHANNEL_MISMATCH");
    let append = MemoryAppendPort::default();
    let candidate = w12_foundation_envelope("MANUAL_CHANNEL_MISMATCH");
    prepare_reserved(&fixture, &candidate, &append);
    let uncertain_sink = StaticSink::new(AuthoritativeSinkResult::Uncertain(uncertainty(now())));
    let uncertain_sinks: Vec<AuthoritativeSink> = vec![uncertain_sink];
    fixture
        .coordinator
        .resume_deliverable(&candidate.decision_identity, &uncertain_sinks, now())
        .expect("record uncertainty before mismatched manual receipt");
    reconcile_terminal(
        &fixture,
        &append,
        DecisionState::UncertainManualReview,
        &candidate.decision_identity,
    );
    let mut wrong_receipt = receipt(now());
    wrong_receipt.channel = "TEST_CODE_WRONG_CHANNEL".to_owned();
    fixture
        .coordinator
        .resolve_uncertain(
            &ManualResolutionCommand {
                decision_identity: candidate.decision_identity.clone(),
                disposition: ManualDisposition::Accepted {
                    receipt: Some(wrong_receipt),
                },
                operator_identity: "TEST_CODE_W12_OPERATOR".to_owned(),
                reason: "TEST_CODE_W12_CONFIRMED_DELIVERED".to_owned(),
                external_evidence: b"TEST_CODE_W12_MANUAL_ACCEPTANCE_EVIDENCE".to_vec(),
                resolved_at: now(),
            },
            &append,
        )
        .expect("record mismatched manual receipt");
    reconcile_terminal(
        &fixture,
        &append,
        DecisionState::Delivered,
        &candidate.decision_identity,
    );

    assert!(fixture
        .coordinator
        .inspect_foundation_terminal(&candidate.decision_identity)
        .is_err());
}
