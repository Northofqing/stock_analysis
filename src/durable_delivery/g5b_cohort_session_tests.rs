//! Real isolated database + filesystem behavior for the B local protocol.
#![cfg(unix)]
use super::*;
use crate::durable_delivery::{
    G5bConfiguredAnalysis, G5bDaySession, G5bSnapshotKind, PreparedG5bArtifact,
};
use crate::llm::{LlmError, LlmProvider};
use crate::monitor::alert_log::{AlertLog, AlertRecord};
use crate::monitor::g5b_selection_v2::{G5bSelectionEvidence, G5bSelectionV2Candidate};
use chrono::NaiveDate;
use std::os::unix::fs::{MetadataExt, PermissionsExt};

const DATE: &str = "2026-09-28";
fn date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 9, 28).unwrap()
}
fn clock(local: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(&format!("{DATE}T{local}+08:00"))
        .unwrap()
        .with_timezone(&Utc)
}
struct NoModelCall;
#[async_trait::async_trait]
impl LlmProvider for NoModelCall {
    fn name(&self) -> &'static str {
        "TEST_CODE_CONFIGURED_ONLY"
    }
    fn model(&self) -> &str {
        "TEST_CODE_NOT_CALLED"
    }
    async fn chat_json(
        &self,
        _: &str,
        _: &str,
    ) -> std::result::Result<serde_json::Value, LlmError> {
        panic!("B prepare/recovery must never call a model")
    }
}
fn ready(session: &G5bDaySession<'_>) -> G5bConfiguredAnalysis {
    session
        .configured_analysis_for_test(Arc::new(NoModelCall), clock("15:10:00"))
        .unwrap()
}
fn namespace(fixture: &Fixture) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(&fixture.database_path)
        .parent()
        .unwrap()
        .to_owned()
}
fn head(fixture: &Fixture) -> PathBuf {
    namespace(fixture).join("20260928.input-head.v1.json")
}
fn source(fixture: &Fixture) -> PathBuf {
    namespace(fixture).join("20260928.jsonl")
}
fn initialize(fixture: &Fixture) -> AlertLog {
    let log = fixture.g5b_input_log(DATE);
    log.initialize_date_input_head(date()).unwrap();
    fixture
        .cleanup
        .record(head(fixture), OwnedPathKind::FileOrSymlink);
    log
}
fn raw(code: &str, level: &str) -> Vec<u8> {
    let record: AlertRecord = serde_json::from_value(serde_json::json!({
        "origin":"production", "triggered_at":format!("{DATE}T15:00:00+08:00"),
        "code":code,"name":"TEST_CODE_B", "level":level,"category":"TEST_CODE_B",
        "message":"exact raw occurrence", "t1_locked":false
    }))
    .unwrap();
    let mut bytes = serde_json::to_vec(&record).unwrap();
    bytes.push(b'\n');
    bytes
}
fn append(fixture: &Fixture, log: &AlertLog, bytes: &[u8]) {
    log.append_test_date_raw_production_fixture(date(), bytes)
        .unwrap();
    fixture
        .cleanup
        .record(source(fixture), OwnedPathKind::FileOrSymlink);
    fixture
        .cleanup
        .record(head(fixture), OwnedPathKind::FileOrSymlink);
}
fn artifact_path(fixture: &Fixture, intent: &PreparedG5bArtifact) -> PathBuf {
    let connection = Connection::open(&fixture.database_path).unwrap();
    let (cohort,role):(String,String)=connection.query_row(
        "SELECT cohort_identity,artifact_role FROM g5b_artifact_events WHERE logical_intent=?1 AND phase='Prepared'",
        [intent.identity()],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
    namespace(fixture).join(format!(
        "20260928.{cohort}.{}.g5b-{}.v2",
        intent.identity(),
        role.to_ascii_lowercase()
    ))
}
fn publish(fixture: &Fixture, session: &G5bDaySession<'_>, intent: &PreparedG5bArtifact) {
    session.publish_prepared_artifact(intent).unwrap();
    let path = artifact_path(fixture, intent);
    fixture.cleanup.record(&path, OwnedPathKind::FileOrSymlink);
    assert_eq!(
        path.metadata().unwrap().nlink(),
        1,
        "atomic publication has no double-link crash stage"
    );
    assert_eq!(path.metadata().unwrap().mode() & 0o777, 0o400);
}
fn published<'a>(fixture: &'a Fixture) -> (G5bDaySession<'a>, PreparedG5bArtifact) {
    let session = fixture.coordinator.g5b_day_session(date()).unwrap();
    let intent = session.prepare_cohort(&ready(&session)).unwrap();
    publish(fixture, &session, &intent);
    session.commit_prepared_artifact(&intent).unwrap();
    (session, intent)
}
fn revision(fixture: &Fixture) -> i64 {
    fixture.query_i64("SELECT revision FROM g5b_day_heads")
}
fn blob_rows(fixture: &Fixture) -> Vec<Vec<String>> {
    let connection = Connection::open(&fixture.database_path).unwrap();
    let mut rows = Vec::new();
    for table in super::super::schema_g5b_cohort::TABLES {
        rows.extend(authority_table_rows(&connection, table));
    }
    rows
}

#[test]
fn g5b_cohort_b_first_admission_uses_current_prefix_and_distinguishes_identical_raw_occurrences() {
    let fixture = Fixture::new("COHORT_B_CURRENT_PREFIX");
    let log = initialize(&fixture);
    append(&fixture, &log, &raw("600001", "重要"));
    let old_bytes = {
        let fence = log.acquire_date_writer_fence(date()).unwrap();
        let prefix = log
            .inspect_date_input_prefix_locked(date(), &fence)
            .unwrap();
        G5bSelectionV2Candidate::from_locked_prefix(&prefix)
            .unwrap()
            .canonical_bytes()
            .to_vec()
    };
    append(&fixture, &log, &raw("600002", "紧急"));
    append(&fixture, &log, &raw("600002", "紧急"));
    let session = fixture.coordinator.g5b_day_session(date()).unwrap();
    let intent = session.prepare_cohort(&ready(&session)).unwrap();
    assert!(session.read_cohort().unwrap().is_none());
    assert_eq!(revision(&fixture), 1);
    assert_eq!(
        fixture.query_strings("SELECT artifact_state FROM g5b_day_heads"),
        vec!["Dirty"]
    );
    let bytes = fixture.query_blob("SELECT selection_canonical FROM g5b_cohorts");
    assert_ne!(bytes, old_bytes);
    let evidence = G5bSelectionEvidence::decode(&bytes).unwrap();
    assert_eq!(evidence.encoded().selected.len(), 3);
    let ordinals = evidence
        .encoded()
        .selected
        .iter()
        .map(|line| line.line_ordinal)
        .collect::<Vec<_>>();
    assert_eq!(ordinals, vec![2, 3, 1]);
    assert_eq!(
        evidence.encoded().selected[0].raw_line_bytes,
        evidence.encoded().selected[1].raw_line_bytes
    );
    assert_ne!(evidence.occurrences()[0].0, evidence.occurrences()[1].0);
    publish(&fixture, &session, &intent);
    session.commit_prepared_artifact(&intent).unwrap();
    let stored = session.read_cohort().unwrap().unwrap();
    assert_eq!(stored.selected_count(), 3);
    assert_eq!(stored.selection_bytes(), bytes);
    let original_identity = stored.identity();
    let original_intent = intent.identity().to_owned();
    drop(stored);
    drop(session);
    append(&fixture, &log, &raw("600003", "紧急"));
    let session = fixture.coordinator.g5b_day_session(date()).unwrap();
    let recovered = session.prepare_cohort(&ready(&session)).unwrap();
    assert_eq!(recovered.identity(), original_intent);
    assert_eq!(
        session.read_cohort().unwrap().unwrap().identity(),
        original_identity
    );
    assert_eq!(
        fixture.query_blob("SELECT selection_canonical FROM g5b_cohorts"),
        bytes,
        "a later eligible suffix never rewrites the frozen cohort"
    );
    assert_eq!(revision(&fixture), 2);
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM delivery_decisions"),
        0
    );
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM immutable_audit_outbox"),
        0
    );
}

#[test]
fn g5b_cohort_b_owner_window_keeps_whole_1520_minute_and_rejects_next_minute() {
    let fixture = Fixture::new("COHORT_B_WINDOW");
    let log = initialize(&fixture);
    append(&fixture, &log, &raw("600001", "重要"));
    let session = fixture.coordinator.g5b_day_session(date()).unwrap();
    for local in ["15:05:00", "15:20:00", "15:20:59.999999999"] {
        assert!(
            session
                .configured_analysis_for_test(Arc::new(NoModelCall), clock(local))
                .is_ok(),
            "{local}"
        );
    }
    for local in ["15:04:59.999999999", "15:21:00"] {
        assert!(
            session
                .configured_analysis_for_test(Arc::new(NoModelCall), clock(local))
                .is_err(),
            "{local}"
        );
    }
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM g5b_cohorts"), 0);
}

#[test]
fn g5b_cohort_b_restart_recovers_exact_synced_prepared_file_without_model_or_replacement() {
    let mut fixture = Fixture::new("COHORT_B_SYNCED_RECOVERY");
    let log = initialize(&fixture);
    append(&fixture, &log, &raw("600001", "重要"));
    let (intent_id, path, before) = {
        let session = fixture.coordinator.g5b_day_session(date()).unwrap();
        let intent = session.prepare_cohort(&ready(&session)).unwrap();
        publish(&fixture, &session, &intent);
        let path = artifact_path(&fixture, &intent);
        let metadata = path.metadata().unwrap();
        (
            intent.identity().to_owned(),
            path,
            (metadata.dev(), metadata.ino()),
        )
    };
    // Real persisted Prepared + atomic-published file, but no Committed SQL.
    let coordinator = fixture.coordinator.take().unwrap();
    assert_eq!(Arc::strong_count(&coordinator), 1);
    drop(coordinator);
    let reopened = fixture.second_coordinator("COHORT_B_RESTART");
    let session = reopened.g5b_day_session(date()).unwrap();
    assert!(session.read_cohort().unwrap().is_none());
    assert_eq!(session.recover_prepared_artifacts().unwrap(), 1);
    let metadata = path.metadata().unwrap();
    assert_eq!((metadata.dev(), metadata.ino()), before);
    assert_eq!(session.recover_prepared_artifacts().unwrap(), 0);
    assert_eq!(revision(&fixture), 2);
    assert_eq!(
        fixture.query_strings("SELECT artifact_state FROM g5b_day_heads"),
        vec!["Clean"]
    );
    assert_eq!(session.read_cohort().unwrap().unwrap().selected_count(), 1);
    assert_eq!(
        fixture
            .query_strings("SELECT logical_intent FROM g5b_artifact_events WHERE phase='Prepared'"),
        vec![intent_id]
    );
}

#[test]
fn g5b_cohort_b_committed_missing_or_same_bytes_new_inode_is_never_healed() {
    for replacement in [false, true] {
        let fixture = Fixture::new(if replacement {
            "COHORT_B_REPLACED"
        } else {
            "COHORT_B_MISSING"
        });
        let log = initialize(&fixture);
        append(&fixture, &log, &raw("600001", "重要"));
        let (session, intent) = published(&fixture);
        let path = artifact_path(&fixture, &intent);
        let bytes = std::fs::read(&path).unwrap();
        let retained = namespace(&fixture).join("TEST_CODE_original-selection");
        std::fs::rename(&path, &retained).unwrap();
        fixture
            .cleanup
            .record(&retained, OwnedPathKind::FileOrSymlink);
        if replacement {
            std::fs::write(&path, &bytes).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o400)).unwrap();
            fixture.cleanup.record(&path, OwnedPathKind::FileOrSymlink);
        }
        let before = blob_rows(&fixture);
        assert!(session.read_cohort().is_err());
        assert!(session.recover_prepared_artifacts().is_err());
        assert_eq!(blob_rows(&fixture), before);
        if replacement {
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        } else {
            assert!(!path.exists());
        }
    }
}

#[test]
fn g5b_cohort_b_foreign_target_and_hardlink_refuse_recovery_without_overwrite() {
    for hardlink in [false, true] {
        let fixture = Fixture::new(if hardlink {
            "COHORT_B_HARDLINK"
        } else {
            "COHORT_B_FOREIGN_TARGET"
        });
        let log = initialize(&fixture);
        append(&fixture, &log, &raw("600001", "重要"));
        let session = fixture.coordinator.g5b_day_session(date()).unwrap();
        let intent = session.prepare_cohort(&ready(&session)).unwrap();
        let target = artifact_path(&fixture, &intent);
        let foreign = namespace(&fixture).join("TEST_CODE_foreign-leaf");
        let desired = fixture
            .query_blob("SELECT desired_bytes FROM g5b_artifact_events WHERE phase='Prepared'");
        std::fs::write(
            &foreign,
            if hardlink {
                desired.as_slice()
            } else {
                b"foreign must survive"
            },
        )
        .unwrap();
        std::fs::set_permissions(&foreign, std::fs::Permissions::from_mode(0o400)).unwrap();
        fixture
            .cleanup
            .record(&foreign, OwnedPathKind::FileOrSymlink);
        if hardlink {
            std::fs::hard_link(&foreign, &target).unwrap();
        } else {
            std::fs::rename(&foreign, &target).unwrap();
        }
        fixture
            .cleanup
            .record(&target, OwnedPathKind::FileOrSymlink);
        let before = blob_rows(&fixture);
        let actual = std::fs::read(&target).unwrap();
        assert!(session.recover_prepared_artifacts().is_err());
        assert_eq!(blob_rows(&fixture), before);
        assert_eq!(std::fs::read(&target).unwrap(), actual);
        assert_eq!(
            fixture.query_strings("SELECT artifact_state FROM g5b_day_heads"),
            vec!["Dirty"]
        );
    }
}

#[test]
fn g5b_cohort_b_fixed_role_intents_use_fresh_revision_and_noop_replay_is_stable() {
    let fixture = Fixture::new("COHORT_B_FIXED_ROLES");
    let log = initialize(&fixture);
    append(&fixture, &log, &raw("600001", "重要"));
    let (session, _) = published(&fixture);
    let cohort = session.read_cohort().unwrap().unwrap();
    let attempt = session
        .prepare_opaque_snapshot(
            &cohort,
            G5bSnapshotKind::Attempt,
            Some(0),
            b"TEST_CODE_attempt-bytes",
        )
        .unwrap();
    assert_eq!(revision(&fixture), 3);
    let frozen = session
        .prepare_opaque_snapshot(
            &cohort,
            G5bSnapshotKind::Frozen,
            Some(0),
            b"TEST_CODE_opaque-not-model-receipt",
        )
        .unwrap();
    assert_eq!(revision(&fixture), 4);
    let same = session
        .prepare_opaque_snapshot(
            &cohort,
            G5bSnapshotKind::Attempt,
            Some(0),
            b"TEST_CODE_attempt-bytes",
        )
        .unwrap();
    assert_eq!(attempt.identity(), same.identity());
    assert_eq!(revision(&fixture), 4);
    assert!(session
        .prepare_opaque_snapshot(&cohort, G5bSnapshotKind::Frozen, Some(1), b"x")
        .is_err());
    assert!(session
        .prepare_opaque_snapshot(&cohort, G5bSnapshotKind::Archive, Some(0), b"x")
        .is_err());
    publish(&fixture, &session, &attempt);
    session.commit_prepared_artifact(&attempt).unwrap();
    assert_eq!(
        revision(&fixture),
        5,
        "original Prepared revision3 commits against current4"
    );
    assert_eq!(fixture.query_i64("SELECT prepared_revision FROM g5b_artifact_events WHERE artifact_role='Attempt' AND phase='Committed'"),3);
    assert_eq!(fixture.query_i64("SELECT commit_revision FROM g5b_artifact_events WHERE artifact_role='Attempt' AND phase='Committed'"),5);
    publish(&fixture, &session, &frozen);
    session.commit_prepared_artifact(&frozen).unwrap();
    let archive = session
        .prepare_opaque_snapshot(
            &cohort,
            G5bSnapshotKind::Archive,
            None,
            b"TEST_CODE_exact-archive-snapshot",
        )
        .unwrap();
    publish(&fixture, &session, &archive);
    session.commit_prepared_artifact(&archive).unwrap();
    let before = blob_rows(&fixture);
    assert_eq!(session.recover_prepared_artifacts().unwrap(), 0);
    assert_eq!(blob_rows(&fixture), before);
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM delivery_decisions"),
        0
    );
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM g5b_day_seals"), 0);
}

#[test]
fn g5b_cohort_b_prospective_observes_real_unchmodded_zero_head_and_cannot_backfill() {
    let fixture = Fixture::new("COHORT_B_PROSPECTIVE");
    let _log = initialize(&fixture);
    let original_head = std::fs::read(head(&fixture)).unwrap();
    let session = fixture.coordinator.g5b_day_session(date()).unwrap();
    session.prospective_for_test(clock("14:59:00")).unwrap();
    let saved = fixture.query_blob("SELECT prospective_canonical FROM g5b_day_heads");
    session.prospective_for_test(clock("15:04:59")).unwrap();
    assert_eq!(
        fixture.query_blob("SELECT prospective_canonical FROM g5b_day_heads"),
        saved
    );
    assert_eq!(revision(&fixture), 0);
    assert_eq!(std::fs::read(head(&fixture)).unwrap(), original_head);
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM g5b_cohorts"), 0);
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM delivery_decisions"),
        0
    );
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM immutable_audit_outbox"),
        0
    );
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM g5b_day_seals"), 0);
    let connection = Connection::open(&fixture.database_path).unwrap();
    super::super::schema::register_sha256_function(&connection).unwrap();
    assert!(connection
        .execute(
            "UPDATE g5b_day_heads SET prospective_canonical=NULL,prospective_sha256=NULL",
            []
        )
        .is_err());
    drop(session);
    let late = Fixture::new("COHORT_B_LATE_ZERO");
    initialize(&late);
    let session = late.coordinator.g5b_day_session(date()).unwrap();
    assert!(session.prospective_for_test(clock("15:21:00")).is_err());
    assert_eq!(late.query_i64("SELECT COUNT(*) FROM g5b_day_heads"), 0);
}

#[test]
fn g5b_cohort_b_prospective_head_inode_change_before_commit_rolls_back_observation() {
    let fixture = Fixture::new("COHORT_B_PROSPECTIVE_INODE");
    initialize(&fixture);
    let original = head(&fixture);
    let retained = namespace(&fixture).join("TEST_CODE_retained-zero-head");
    let bytes = std::fs::read(&original).unwrap();
    let hook_original = original.clone();
    let hook_retained = retained.clone();
    fixture
        .coordinator
        .install_database_operation_test_hook(
            DatabaseOperationTestPhase::AfterSqlBeforePreCommitValidation,
            move || {
                std::fs::rename(&hook_original, &hook_retained)?;
                std::fs::write(&hook_original, &bytes)?;
                Ok(())
            },
        )
        .unwrap();
    let session = fixture.coordinator.g5b_day_session(date()).unwrap();
    let result = session.prospective_for_test(clock("14:59:00"));
    fixture
        .cleanup
        .record(&retained, OwnedPathKind::FileOrSymlink);
    fixture
        .cleanup
        .record(&original, OwnedPathKind::FileOrSymlink);
    assert!(result.is_err());
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM g5b_day_heads"), 0);
}

#[test]
fn g5b_cohort_b_input_mutation_after_sql_cannot_publish_or_read_receipt() {
    let fixture = Fixture::new("COHORT_B_PREFIX_MUTATION");
    let log = initialize(&fixture);
    append(&fixture, &log, &raw("600001", "重要"));
    let path = source(&fixture);
    let hook_path = path.clone();
    fixture
        .coordinator
        .install_database_operation_test_hook(
            DatabaseOperationTestPhase::AfterSqlBeforePreCommitValidation,
            move || {
                let mut file = std::fs::OpenOptions::new().append(true).open(&hook_path)?;
                file.write_all(b"TEST_CODE_uncommitted_suffix\n")?;
                file.sync_all()?;
                Ok(())
            },
        )
        .unwrap();
    let session = fixture.coordinator.g5b_day_session(date()).unwrap();
    let prepared = session.prepare_cohort(&ready(&session));
    assert!(
        prepared.is_err(),
        "source changed at the actual precommit boundary"
    );
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM g5b_cohorts"), 0);
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM g5b_artifact_events"),
        0
    );
    assert!(session.read_cohort().unwrap().is_none());
}

#[test]
fn g5b_cohort_b_runtime_rejects_self_consistent_future_event_revision_and_unowned_seal() {
    for forged_seal in [false, true] {
        let fixture = Fixture::new(if forged_seal {
            "COHORT_B_FORGED_SEAL"
        } else {
            "COHORT_B_FUTURE_REVISION"
        });
        let log = initialize(&fixture);
        append(&fixture, &log, &raw("600001", "重要"));
        let session = fixture.coordinator.g5b_day_session(date()).unwrap();
        let intent = session.prepare_cohort(&ready(&session)).unwrap();
        if forged_seal {
            publish(&fixture, &session, &intent);
            session.commit_prepared_artifact(&intent).unwrap();
        }
        drop(session);
        let connection = Connection::open(&fixture.database_path).unwrap();
        super::super::schema::register_sha256_function(&connection).unwrap();
        if forged_seal {
            let cohort: String = connection
                .query_row("SELECT cohort_identity FROM g5b_cohorts", [], |r| r.get(0))
                .unwrap();
            let canonical = serde_json::to_vec(
                &serde_json::json!({"business_date":DATE,"cohort_identity":cohort,"revision":2}),
            )
            .unwrap();
            let mut preimage = b"g5b-cohort-day-seal-v1\0".to_vec();
            preimage.extend_from_slice(&canonical);
            connection.execute("INSERT INTO g5b_day_seals(seal_identity,business_date,cohort_identity,revision,seal_canonical,seal_sha256,seal_preimage) VALUES(?1,?2,?3,2,?4,?5,?6)",params![sha256_hex(&preimage),DATE,cohort,canonical,sha256_hex(&canonical),preimage]).unwrap();
        } else {
            // Deliberately bypass the catalog guard to independently exercise
            // row validation: every hash/column binds the same forged bytes.
            connection
                .execute_batch("DROP TRIGGER g5b_artifact_events_immutable_update")
                .unwrap();
            let bytes: Vec<u8> = connection
                .query_row("SELECT event_canonical FROM g5b_artifact_events", [], |r| {
                    r.get(0)
                })
                .unwrap();
            // Preserve struct field order instead of Value's key ordering.
            let original = String::from_utf8(bytes).unwrap();
            let canonical = original
                .replace("\"prepared_revision\":1,", "\"prepared_revision\":100,")
                .into_bytes();
            connection.execute("UPDATE g5b_artifact_events SET event_identity=?1,prepared_revision=100,event_canonical=?2,event_sha256=?3",params![event_hash(&canonical),canonical,sha256_hex(&canonical)]).unwrap();
        }
        assert!(super::super::coordinator::validate_g5b_cohort_rows(&connection).is_err());
        let before = blob_rows(&fixture);
        assert!(fixture
            .coordinator
            .decision_state("TEST_CODE_absent")
            .is_err());
        assert_eq!(blob_rows(&fixture), before);
    }
}

fn event_hash(canonical: &[u8]) -> String {
    let mut preimage = b"g5b-artifact-event-v1\0".to_vec();
    preimage.extend_from_slice(canonical);
    sha256_hex(&preimage)
}

#[test]
fn g5b_cohort_b_legacy_journal_and_unknown_snapshot_do_not_gain_first_admission() {
    for old_journal in [false, true] {
        let fixture = Fixture::new(if old_journal {
            "COHORT_B_LEGACY_JOURNAL"
        } else {
            "COHORT_B_UNKNOWN_SNAPSHOT"
        });
        let log = initialize(&fixture);
        append(&fixture, &log, &raw("600001", "重要"));
        let leaf = if old_journal {
            let directory = namespace(&fixture).join("attempts");
            std::fs::create_dir(&directory).unwrap();
            fixture.cleanup.record(&directory, OwnedPathKind::Directory);
            directory.join(format!("{DATE}.0.attempt"))
        } else {
            namespace(&fixture).join("20260928.TEST_CODE_unowned.g5b-selection.v2")
        };
        std::fs::write(&leaf, b"TEST_CODE_legacy_or_unowned").unwrap();
        fixture.cleanup.record(&leaf, OwnedPathKind::FileOrSymlink);
        let session = fixture.coordinator.g5b_day_session(date()).unwrap();
        assert!(session.prepare_cohort(&ready(&session)).is_err());
        assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM g5b_cohorts"), 0);
        assert_eq!(
            fixture.query_i64("SELECT COUNT(*) FROM g5b_artifact_events"),
            0
        );
        assert_eq!(
            std::fs::read(&leaf).unwrap(),
            b"TEST_CODE_legacy_or_unowned"
        );
    }
}
