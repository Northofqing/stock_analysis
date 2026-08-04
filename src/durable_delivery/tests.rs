use super::coordinator::{
    install_compound_commit_rollback_test_fault, install_database_bootstrap_test_hook,
    install_process_descriptor_snapshot_test_fault, DatabaseBootstrapTestPhase,
    DatabaseOperationTestPhase, DeliveredPrecommitTestFault, OpenFileDescriptionProof,
    OperationPostvalidationTestFault, ProcessDescriptorSnapshotTestFault,
};
use super::model::sha256_hex;
use super::*;
use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::BTreeMap;
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
                     REFERENCES immutable_audit_outbox_v4_historical(audit_identity),
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

impl Default for FailScheduleHydrationAppliedOnce {
    fn default() -> Self {
        Self {
            inner: MemoryAppendPort::default(),
            fail_next_hydration_ack: std::sync::atomic::AtomicBool::new(true),
        }
    }
}

impl MemoryAppendPort {
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
    DeliveryEnvelope::new(
        business_date,
        push_kind,
        sub_kind,
        scope_key,
        format!("TEST_CODE_OCCURRENCE_{label}"),
        format!("TEST_CODE_EVIDENCE_{label}"),
        format!("TEST_CODE_SOURCE_BINDING_{label}").into_bytes(),
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
fn br200_r04_rolling_preflight_prefers_original_delivered_over_later_denial() {
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
fn policy_catalog_has_fifteen_kinds_and_eighteen_rows() {
    let fixture = Fixture::new("CATALOG");
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM delivery_policy_catalog"),
        18
    );
    assert_eq!(
        fixture.query_i64("SELECT COUNT(DISTINCT push_kind) FROM delivery_policy_catalog"),
        15
    );
    assert_eq!(compiled_policy_catalog().len(), 18);
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
        PushKind::ReviewProviderTopN,
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
        PushKind::ReviewProviderTopN,
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
