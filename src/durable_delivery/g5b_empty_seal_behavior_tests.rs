//! Actual isolated head/SQL/artifact protocol, not constructed successful seals.
#![cfg(unix)]
use super::*;
use crate::monitor::alert_log::{AlertInputHeadV1, AlertLog, AlertRecord};
use chrono::NaiveDate;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
const DATE: &str = "2026-09-28";
fn date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 9, 28).unwrap()
}
fn clock(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(&format!("{DATE}T{value}+08:00"))
        .unwrap()
        .with_timezone(&Utc)
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
fn record(fixture: &Fixture, path: impl AsRef<Path>) {
    fixture
        .cleanup
        .record_if_present(path.as_ref().to_path_buf(), OwnedPathKind::FileOrSymlink);
}
fn setup(fixture: &Fixture) -> AlertLog {
    let log = fixture.g5b_input_log(DATE);
    let session = fixture.coordinator.g5b_day_session(date()).unwrap();
    session
        .initialize_empty_prospective_for_test(clock("15:04:59.999"))
        .unwrap();
    record(fixture, head(fixture));
    log
}
fn selection_path(fixture: &Fixture) -> PathBuf {
    let connection = Connection::open(&fixture.database_path).unwrap();
    let (cohort,logical):(String,String)=connection.query_row("SELECT cohort_identity,logical_intent FROM g5b_artifact_events WHERE artifact_role='Selection' AND phase='Prepared'",[],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
    namespace(fixture).join(format!("20260928.{cohort}.{logical}.g5b-selection.v2"))
}
fn seal(fixture: &Fixture) {
    let session = fixture.coordinator.g5b_day_session(date()).unwrap();
    let cap = session
        .close_empty_window_for_test(clock("15:21:00"))
        .unwrap();
    assert_eq!(cap.business_date(), date());
    assert_eq!(cap.reason(), "NoEligibleInVerifiedClosedWindowPrefix");
    assert_eq!(cap.revision(), 2);
    record(fixture, selection_path(fixture));
}
fn b_rows(fixture: &Fixture) -> Vec<Vec<String>> {
    let connection = Connection::open(&fixture.database_path).unwrap();
    let mut rows = Vec::new();
    for table in super::super::schema_g5b_cohort::TABLES {
        rows.extend(authority_table_rows(&connection, table));
    }
    rows
}
fn raw(code: &str) -> Vec<u8> {
    let record:AlertRecord=serde_json::from_value(serde_json::json!({"origin":"production","triggered_at":format!("{DATE}T15:30:00+08:00"),"code":code,"name":"TEST_CODE_EMPTY_SUFFIX","level":"重要","category":"TEST_CODE_EMPTY_SUFFIX","message":"late real input","t1_locked":false})).unwrap();
    let mut bytes = serde_json::to_vec(&record).unwrap();
    bytes.push(b'\n');
    bytes
}
fn append(fixture: &Fixture, log: &AlertLog, code: &str) {
    log.append_test_date_raw_production_fixture(date(), &raw(code))
        .unwrap();
    record(fixture, source(fixture));
    record(fixture, head(fixture));
}
fn prepared_and_published(fixture: &Fixture) -> PreparedG5bArtifact {
    let session = fixture.coordinator.g5b_day_session(date()).unwrap();
    let intent = session
        .prepare_empty_closure_for_test(clock("15:21:00"))
        .unwrap();
    session.publish_prepared_artifact(&intent).unwrap();
    record(fixture, selection_path(fixture));
    intent
}
fn after_n_sql(
    coordinator: &Arc<DurableDeliveryCoordinator>,
    phase: DatabaseOperationTestPhase,
    count: usize,
    mutation: Arc<dyn Fn() -> Result<()> + Send + Sync>,
) {
    let owner = Arc::downgrade(coordinator);
    coordinator
        .install_database_operation_test_hook(phase, move || {
            if count == 1 {
                mutation()
            } else {
                after_n_sql(&owner.upgrade().unwrap(), phase, count - 1, mutation);
                Ok(())
            }
        })
        .unwrap();
}

#[test]
fn g5b_empty_seal_real_prospective_closed_zero_and_exact_replay_have_no_decision_side_effects() {
    let fixture = Fixture::new("EMPTY_REAL");
    setup(&fixture);
    let original_head = std::fs::read(head(&fixture)).unwrap();
    assert_eq!(fixture.query_i64("SELECT revision FROM g5b_day_heads"), 0);
    seal(&fixture);
    assert_eq!(std::fs::read(head(&fixture)).unwrap(), original_head);
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM delivery_decisions"),
        0
    );
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM delivery_attempts"),
        0
    );
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM sink_results"), 0);
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM immutable_audit_outbox"),
        0
    );
    let before = b_rows(&fixture);
    let reopened = fixture.second_coordinator("EMPTY_REAL_REOPEN");
    let session = reopened.g5b_day_session(date()).unwrap();
    let first = session.read_empty_seal().unwrap().unwrap();
    let later = DateTime::parse_from_rfc3339("2026-09-29T18:00:00+08:00")
        .unwrap()
        .with_timezone(&Utc);
    let second = session.close_empty_window_for_test(later).unwrap();
    assert_eq!(first.identity(), second.identity());
    assert_eq!(first.sha256(), second.sha256());
    assert_eq!(first.cohort_identity(), second.cohort_identity());
    assert_eq!(session.recover_prepared_artifacts().unwrap(), 0);
    assert_eq!(b_rows(&fixture), before);
    assert!(
        session.read_cohort().is_err(),
        "Empty never returns NonEmpty model work"
    );
}

#[test]
fn g5b_empty_seal_init_does_not_adopt_an_existing_zero_head_and_exact_owned_replay_is_noop() {
    let fixture = Fixture::new("EMPTY_EXISTING_ZERO_INIT");
    let log = fixture.g5b_input_log(DATE);
    log.initialize_date_input_head(date()).unwrap();
    record(&fixture, head(&fixture));
    let before = b_rows(&fixture);
    let bytes = std::fs::read(head(&fixture)).unwrap();
    let identity = FilesystemIdentity::capture(&head(&fixture)).unwrap();
    let leaves = std::fs::read_dir(namespace(&fixture))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<BTreeSet<_>>();
    let session = fixture.coordinator.g5b_day_session(date()).unwrap();
    assert!(session
        .initialize_empty_prospective_for_test(clock("15:00:00"))
        .is_err());
    assert_eq!(b_rows(&fixture), before);
    assert_eq!(std::fs::read(head(&fixture)).unwrap(), bytes);
    assert_eq!(
        FilesystemIdentity::capture(&head(&fixture)).unwrap(),
        identity
    );
    assert_eq!(
        std::fs::read_dir(namespace(&fixture))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<BTreeSet<_>>(),
        leaves
    );
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM g5b_day_heads"), 0);
    drop(session);

    let unknown = Fixture::new("EMPTY_EXISTING_SQL_ZERO_INIT");
    unknown.g5b_input_log(DATE);
    let connection = Connection::open(&unknown.database_path).unwrap();
    super::super::schema::register_sha256_function(&connection).unwrap();
    connection
        .execute(
            "INSERT INTO g5b_day_heads(business_date,revision,artifact_state) VALUES(?1,0,'Clean')",
            [DATE],
        )
        .unwrap();
    drop(connection);
    let before = b_rows(&unknown);
    let leaves = std::fs::read_dir(namespace(&unknown))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<BTreeSet<_>>();
    let session = unknown.coordinator.g5b_day_session(date()).unwrap();
    assert!(!head(&unknown).exists());
    assert!(session
        .initialize_empty_prospective_for_test(clock("15:00:00"))
        .is_err());
    assert!(!head(&unknown).exists());
    assert_eq!(b_rows(&unknown), before);
    assert_eq!(
        std::fs::read_dir(namespace(&unknown))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<BTreeSet<_>>(),
        leaves
    );
    drop(session);

    let owned = Fixture::new("EMPTY_OWNED_ZERO_REPLAY");
    owned.g5b_input_log(DATE);
    let session = owned.coordinator.g5b_day_session(date()).unwrap();
    session
        .initialize_empty_prospective_for_test(clock("15:00:00"))
        .unwrap();
    record(&owned, head(&owned));
    let before = b_rows(&owned);
    let identity = FilesystemIdentity::capture(&head(&owned)).unwrap();
    session
        .initialize_empty_prospective_for_test(clock("15:04:00"))
        .unwrap();
    assert_eq!(b_rows(&owned), before);
    assert_eq!(
        FilesystemIdentity::capture(&head(&owned)).unwrap(),
        identity
    );
}

#[test]
fn g5b_empty_seal_clock_boundaries_do_not_backfill_or_close_an_open_window() {
    for local in ["15:05:00", "15:21:00"] {
        let fixture = Fixture::new("EMPTY_LATE_INIT");
        fixture.g5b_input_log(DATE);
        let session = fixture.coordinator.g5b_day_session(date()).unwrap();
        assert!(session
            .initialize_empty_prospective_for_test(clock(local))
            .is_err());
        assert!(!head(&fixture).exists());
        assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM g5b_day_heads"), 0);
    }
    let fixture = Fixture::new("EMPTY_OPEN_WINDOW");
    setup(&fixture);
    let session = fixture.coordinator.g5b_day_session(date()).unwrap();
    let before = b_rows(&fixture);
    for local in ["15:20:00", "15:20:59.999"] {
        assert!(session.close_empty_window_for_test(clock(local)).is_err());
        assert_eq!(b_rows(&fixture), before);
    }
    let old = DateTime::parse_from_rfc3339("2026-09-29T15:21:00+08:00")
        .unwrap()
        .with_timezone(&Utc);
    assert!(session.close_empty_window_for_test(old).is_err());
    assert_eq!(b_rows(&fixture), before);
    session
        .close_empty_window_for_test(clock("15:21:00"))
        .unwrap();
    record(&fixture, selection_path(&fixture));
    let holiday = Fixture::new("EMPTY_HOLIDAY");
    let date = NaiveDate::from_ymd_opt(2026, 10, 2).unwrap();
    holiday.g5b_input_log("2026-10-02");
    let session = holiday.coordinator.g5b_day_session(date).unwrap();
    let now = DateTime::parse_from_rfc3339("2026-10-02T14:59:00+08:00")
        .unwrap()
        .with_timezone(&Utc);
    assert!(session.initialize_empty_prospective_for_test(now).is_err());
    assert_eq!(holiday.query_i64("SELECT COUNT(*) FROM g5b_day_heads"), 0);
}

#[test]
fn g5b_empty_seal_prehead_no_prospective_nonzero_and_existing_empty_source_remain_unknown() {
    let prehead = Fixture::new("EMPTY_PREHEAD");
    let log = prehead.g5b_input_log(DATE);
    std::fs::write(source(&prehead), b"").unwrap();
    record(&prehead, source(&prehead));
    let session = prehead.coordinator.g5b_day_session(date()).unwrap();
    assert!(session
        .initialize_empty_prospective_for_test(clock("15:00:00"))
        .is_err());
    assert!(!head(&prehead).exists());
    drop(session);
    drop(log);
    let missing = Fixture::new("EMPTY_NO_PROSPECTIVE");
    let log = missing.g5b_input_log(DATE);
    log.initialize_date_input_head(date()).unwrap();
    record(&missing, head(&missing));
    let session = missing.coordinator.g5b_day_session(date()).unwrap();
    assert!(session
        .close_empty_window_for_test(clock("15:21:00"))
        .is_err());
    assert_eq!(missing.query_i64("SELECT COUNT(*) FROM g5b_cohorts"), 0);
    for nonzero in [false, true] {
        let fixture = Fixture::new("EMPTY_SOURCE_EXISTS");
        let log = setup(&fixture);
        if nonzero {
            append(&fixture, &log, "600001");
        } else {
            std::fs::write(source(&fixture), b"").unwrap();
            record(&fixture, source(&fixture));
        }
        let session = fixture.coordinator.g5b_day_session(date()).unwrap();
        let before = b_rows(&fixture);
        assert!(session
            .close_empty_window_for_test(clock("15:21:00"))
            .is_err());
        assert_eq!(b_rows(&fixture), before);
    }
}

#[test]
fn g5b_empty_seal_original_zero_head_inode_and_immutable_prospective_cannot_be_replaced() {
    let fixture = Fixture::new("EMPTY_ZERO_REPLACED");
    setup(&fixture);
    let before = b_rows(&fixture);
    let retained = namespace(&fixture).join("TEST_CODE_retained_zero_head");
    let bytes = std::fs::read(head(&fixture)).unwrap();
    std::fs::rename(head(&fixture), &retained).unwrap();
    record(&fixture, &retained);
    std::fs::write(head(&fixture), &bytes).unwrap();
    record(&fixture, head(&fixture));
    let session = fixture.coordinator.g5b_day_session(date()).unwrap();
    assert!(session
        .close_empty_window_for_test(clock("15:21:00"))
        .is_err());
    assert_eq!(b_rows(&fixture), before);
    let connection = Connection::open(&fixture.database_path).unwrap();
    super::super::schema::register_sha256_function(&connection).unwrap();
    assert!(connection
        .execute(
            "UPDATE g5b_day_heads SET prospective_canonical=NULL,prospective_sha256=NULL",
            []
        )
        .is_err());
    assert_eq!(b_rows(&fixture), before);
}

#[test]
fn g5b_empty_seal_legacy_unknown_artifacts_and_real_legacy_decisions_prevent_first_seal() {
    for kind in ["unknown", "legacy", "legacy_alias"] {
        let fixture = Fixture::new("EMPTY_UNKNOWN_ARTIFACT");
        setup(&fixture);
        let path = if kind == "legacy" {
            let dir = namespace(&fixture).join("attempts");
            std::fs::create_dir(&dir).unwrap();
            fixture.cleanup.record(&dir, OwnedPathKind::Directory);
            dir.join("2026-09-28.99.attempt")
        } else if kind == "legacy_alias" {
            let target = namespace(&fixture).join("TEST_CODE_legacy_alias_target");
            std::fs::create_dir(&target).unwrap();
            fixture.cleanup.record(&target, OwnedPathKind::Directory);
            let alias = namespace(&fixture).join("attempts");
            std::os::unix::fs::symlink(&target, &alias).unwrap();
            record(&fixture, &alias);
            target.join("2026-09-28.99.attempt")
        } else {
            namespace(&fixture).join("20260928.TEST_CODE_unknown.g5b-selection.v2")
        };
        std::fs::write(&path, b"TEST_CODE_FOREIGN").unwrap();
        record(&fixture, &path);
        let before = b_rows(&fixture);
        let session = fixture.coordinator.g5b_day_session(date()).unwrap();
        assert!(session
            .close_empty_window_for_test(clock("15:21:00"))
            .is_err());
        assert_eq!(b_rows(&fixture), before);
    }
    let fixture = Fixture::new("EMPTY_LEGACY_DECISION");
    setup(&fixture);
    let candidate = g5b_frozen_envelope_for_date("EMPTY_LEGACY_DECISION", false, DATE);
    assert_eq!(candidate.business_date, DATE);
    let prepared = fixture
        .coordinator
        .prepare(&candidate, 1, clock("15:10:00"))
        .unwrap();
    assert_eq!(prepared.state, DecisionState::Reserved);
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM delivery_decisions WHERE business_date='2026-09-28' AND push_kind='G5bAttribution'"),
        1
    );
    let before = b_rows(&fixture);
    let session = fixture.coordinator.g5b_day_session(date()).unwrap();
    assert!(session
        .close_empty_window_for_test(clock("15:21:00"))
        .is_err());
    assert_eq!(b_rows(&fixture), before);
}

#[test]
fn g5b_empty_seal_synced_original_selection_recovers_after_restart_without_new_closing_bytes() {
    let fixture = Fixture::new("EMPTY_RECOVER_SYNCED");
    setup(&fixture);
    let intent = prepared_and_published(&fixture);
    let original = std::fs::read(selection_path(&fixture)).unwrap();
    assert_eq!(fixture.query_i64("SELECT revision FROM g5b_day_heads"), 1);
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM g5b_day_seals"), 0);
    let reopened = fixture.second_coordinator("EMPTY_RECOVER_SYNCED");
    let session = reopened.g5b_day_session(date()).unwrap();
    assert_eq!(session.recover_prepared_artifacts().unwrap(), 1);
    assert_eq!(std::fs::read(selection_path(&fixture)).unwrap(), original);
    assert_eq!(
        fixture.query_blob("SELECT desired_bytes FROM g5b_artifact_events WHERE phase='Committed'"),
        intent.desired_bytes()
    );
    let cap = session
        .close_empty_window_for_test(clock("16:00:00"))
        .unwrap();
    assert_eq!(cap.revision(), 2);
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM g5b_artifact_events"),
        2
    );
    let before = b_rows(&fixture);
    assert_eq!(session.recover_prepared_artifacts().unwrap(), 0);
    assert_eq!(b_rows(&fixture), before);
}

#[test]
fn g5b_empty_seal_committed_missing_or_same_bytes_replaced_inode_never_heals() {
    for replace in [false, true] {
        let fixture = Fixture::new("EMPTY_COMMITTED_FILE");
        setup(&fixture);
        seal(&fixture);
        let path = selection_path(&fixture);
        let before = b_rows(&fixture);
        let original = std::fs::read(&path).unwrap();
        let retained = namespace(&fixture).join("TEST_CODE_retained_empty_selection");
        std::fs::rename(&path, &retained).unwrap();
        record(&fixture, &retained);
        if replace {
            std::fs::write(&path, &original).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o400)).unwrap();
            record(&fixture, &path);
        }
        let session = fixture.coordinator.g5b_day_session(date()).unwrap();
        assert!(session.read_empty_seal().is_err());
        assert!(session.recover_prepared_artifacts().is_err());
        assert!(session
            .close_empty_window_for_test(clock("16:00:00"))
            .is_err());
        assert_eq!(b_rows(&fixture), before);
        assert_eq!(
            path.exists(),
            replace,
            "missing Committed target was not recreated"
        );
        if replace {
            assert_eq!(std::fs::read(path).unwrap(), original);
        }
    }
}

#[test]
fn g5b_empty_seal_valid_late_suffix_preserves_closure_and_known_source_identity_conflict_is_rejected(
) {
    let fixture = Fixture::new("EMPTY_LATE_SUFFIX");
    let log = setup(&fixture);
    seal(&fixture);
    let zero = {
        let session = fixture.coordinator.g5b_day_session(date()).unwrap();
        session.read_empty_seal().unwrap().unwrap()
    };
    let before = b_rows(&fixture);
    append(&fixture, &log, "600001");
    let positive = {
        let session = fixture.coordinator.g5b_day_session(date()).unwrap();
        session.refresh_empty_seal(&zero).unwrap()
    };
    assert_eq!(positive.identity(), zero.identity());
    assert_eq!(b_rows(&fixture), before);
    append(&fixture, &log, "600002");
    {
        let session = fixture.coordinator.g5b_day_session(date()).unwrap();
        assert_eq!(
            session.refresh_empty_seal(&positive).unwrap().identity(),
            zero.identity()
        );
    }
    // A self-consistent current head does not erase a source incarnation already
    // carried in an opaque actual-reader capability. No future inode is guessed.
    let retained = namespace(&fixture).join("TEST_CODE_retained_late_source");
    let bytes = std::fs::read(source(&fixture)).unwrap();
    std::fs::rename(source(&fixture), &retained).unwrap();
    record(&fixture, &retained);
    std::fs::write(source(&fixture), &bytes).unwrap();
    record(&fixture, source(&fixture));
    let metadata = std::fs::metadata(source(&fixture)).unwrap();
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(head(&fixture)).unwrap()).unwrap();
    value["source_identity"] = serde_json::json!({"device":metadata.dev(),"inode":metadata.ino()});
    let structural: AlertInputHeadV1 = serde_json::from_value(value).unwrap();
    let mut encoded = serde_json::to_vec(&structural).unwrap();
    encoded.push(b'\n');
    std::fs::write(head(&fixture), encoded).unwrap();
    record(&fixture, head(&fixture));
    let session = fixture.coordinator.g5b_day_session(date()).unwrap();
    assert!(session.refresh_empty_seal(&positive).is_err());
    assert_eq!(b_rows(&fixture), before);
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM delivery_decisions"),
        0
    );
}

#[test]
fn g5b_empty_seal_late_corrupt_or_oversized_prefix_and_symlink_are_rejected_without_reselection() {
    for mode in ["bad_suffix", "oversize", "symlink"] {
        let fixture = Fixture::new("EMPTY_BAD_LATE");
        let log = setup(&fixture);
        seal(&fixture);
        append(&fixture, &log, "600001");
        let before = b_rows(&fixture);
        if mode == "symlink" {
            let retained = namespace(&fixture).join("TEST_CODE_source_alias_target");
            std::fs::rename(source(&fixture), &retained).unwrap();
            record(&fixture, &retained);
            std::os::unix::fs::symlink(&retained, source(&fixture)).unwrap();
            record(&fixture, source(&fixture));
        } else {
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(source(&fixture))
                .unwrap();
            if mode == "oversize" {
                file.set_len(32 * 1024 * 1024 + 1).unwrap();
            } else {
                file.write_all(b"TEST_CODE_uncommitted_no_LF").unwrap();
            }
            file.sync_all().unwrap();
        }
        let session = fixture.coordinator.g5b_day_session(date()).unwrap();
        assert!(session.read_empty_seal().is_err(), "{mode}");
        assert_eq!(b_rows(&fixture), before);
        assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM g5b_day_seals"), 1);
    }
}

#[test]
fn g5b_empty_seal_real_eversealed_date_blocks_business_and_new_artifacts_but_exact_recovery_is_noop(
) {
    let fixture = Fixture::new("EMPTY_EVERSEALED");
    setup(&fixture);
    seal(&fixture);
    let before = b_rows(&fixture);
    {
        let session = fixture.coordinator.g5b_day_session(date()).unwrap();
        assert!(session.attempt_empty_artifact_reopen_for_test().is_err());
        assert_eq!(session.recover_prepared_artifacts().unwrap(), 0);
    }
    let candidate = g5b_frozen_envelope_for_date("EMPTY_EVERSEALED_FRESH", false, DATE);
    assert!(fixture
        .coordinator
        .prepare(&candidate, 1, clock("16:00:00"))
        .is_err());
    assert_eq!(b_rows(&fixture), before);
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM delivery_decisions"),
        0
    );
    let ordinary = envelope(
        "EMPTY_ORDINARY_AFTER_SEAL",
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        DATE,
        false,
    );
    fixture
        .coordinator
        .prepare(&ordinary, 1, clock("16:00:00"))
        .unwrap();
    assert_eq!(b_rows(&fixture), before);
}

#[test]
fn g5b_empty_seal_prepare_and_commit_after_sql_file_faults_roll_back_the_actual_write() {
    for family in ["prepare", "commit"] {
        for phase in [
            DatabaseOperationTestPhase::AfterSqlBeforePreCommitValidation,
            DatabaseOperationTestPhase::AfterSqlBeforePostValidation,
        ] {
            let fixture = Fixture::new("EMPTY_SQL_ROLLBACK");
            setup(&fixture);
            let intent = if family == "commit" {
                Some(prepared_and_published(&fixture))
            } else {
                None
            };
            let before = b_rows(&fixture);
            let changed = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let called = Arc::clone(&changed);
            let path = source(&fixture);
            after_n_sql(
                fixture.coordinator.0.as_ref().unwrap(),
                phase,
                3,
                Arc::new(move || {
                    std::fs::write(&path, b"")?;
                    called.store(true, Ordering::SeqCst);
                    Ok(())
                }),
            );
            let session = fixture.coordinator.g5b_day_session(date()).unwrap();
            let result = if let Some(intent) = intent.as_ref() {
                session.commit_prepared_artifact(intent).map(|_| ())
            } else {
                session
                    .prepare_empty_closure_for_test(clock("15:21:00"))
                    .map(|_| ())
            };
            record(&fixture, source(&fixture));
            assert!(
                changed.load(Ordering::SeqCst),
                "actual {family} SQL hook was not reached"
            );
            assert!(result.is_err(), "{family}");
            assert_eq!(
                b_rows(&fixture),
                before,
                "{family} must not persist its event/head change"
            );
        }
    }
}

#[test]
fn g5b_empty_seal_seal_cas_precommit_failure_rolls_back_and_postcommit_failure_recovers_exact_fact()
{
    for phase in [
        DatabaseOperationTestPhase::AfterSqlBeforePreCommitValidation,
        DatabaseOperationTestPhase::AfterSqlBeforePostValidation,
        DatabaseOperationTestPhase::AfterCommitBeforePostValidation,
    ] {
        let fixture = Fixture::new("EMPTY_SEAL_CAS_FAULT");
        setup(&fixture);
        let intent = prepared_and_published(&fixture);
        {
            let session = fixture.coordinator.g5b_day_session(date()).unwrap();
            session.commit_prepared_artifact(&intent).unwrap();
        }
        let before = b_rows(&fixture);
        let changed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let called = Arc::clone(&changed);
        let path = source(&fixture);
        let created_identity = Arc::new(Mutex::new(None));
        let captured_identity = Arc::clone(&created_identity);
        // Already-Committed closure: read1 + saved prepare2/3 + status4 +
        // exact commit5/6/7 + fresh closure8 precede the actual seal CAS9.
        after_n_sql(
            fixture.coordinator.0.as_ref().unwrap(),
            phase,
            9,
            Arc::new(move || {
                std::fs::write(&path, b"")?;
                *captured_identity.lock().unwrap() = Some(FilesystemIdentity::capture(&path)?);
                called.store(true, Ordering::SeqCst);
                Ok(())
            }),
        );
        let session = fixture.coordinator.g5b_day_session(date()).unwrap();
        let result = session.close_empty_window_for_test(clock("16:00:00"));
        record(&fixture, source(&fixture));
        assert!(
            changed.load(Ordering::SeqCst),
            "actual seal CAS fault phase was not reached"
        );
        assert!(result.is_err());
        if phase == DatabaseOperationTestPhase::AfterCommitBeforePostValidation {
            assert!(result
                .err()
                .unwrap()
                .to_string()
                .contains("after COMMIT succeeded"));
            assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM g5b_day_seals"), 1);
            assert_eq!(
                fixture.query_i64(
                    "SELECT COUNT(*) FROM g5b_day_heads WHERE current_seal_identity IS NOT NULL"
                ),
                1
            );
        } else {
            assert_eq!(b_rows(&fixture), before);
            assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM g5b_day_seals"), 0);
        }
        // Remove only the exact file created by this test fault, restoring the
        // original source-absent condition; no Committed artifact is healed.
        let identity = FilesystemIdentity::capture(&source(&fixture)).unwrap();
        assert_eq!(Some(identity), *created_identity.lock().unwrap());
        std::fs::remove_file(source(&fixture)).unwrap();
        let cap = session
            .close_empty_window_for_test(clock("16:01:00"))
            .unwrap();
        assert_eq!(cap.revision(), 2);
        assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM g5b_day_seals"), 1);
    }
}

#[test]
fn g5b_empty_seal_reader_second_sql_and_last_artifact_read_cannot_mint_stale_completion() {
    for final_file in [false, true] {
        let fixture = Fixture::new("EMPTY_READER_BOUNDARY");
        setup(&fixture);
        seal(&fixture);
        let before = b_rows(&fixture);
        let changed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let called = Arc::clone(&changed);
        let path = source(&fixture);
        let session = fixture.coordinator.g5b_day_session(date()).unwrap();
        if final_file {
            let armed = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let trigger = Arc::clone(&armed);
            // Once the second reader SQL has COMMITted: first file observation
            // is postvalidation; second is the final capability observation.
            session
                .install_empty_final_input_fault_for_test(armed, 2, move || {
                    std::fs::write(&path, b"")?;
                    called.store(true, Ordering::SeqCst);
                    Ok(())
                })
                .unwrap();
            after_n_sql(
                fixture.coordinator.0.as_ref().unwrap(),
                DatabaseOperationTestPhase::AfterCommitBeforePostValidation,
                2,
                Arc::new(move || {
                    trigger.store(true, Ordering::SeqCst);
                    Ok(())
                }),
            );
        } else {
            after_n_sql(
                fixture.coordinator.0.as_ref().unwrap(),
                DatabaseOperationTestPhase::AfterSqlBeforePreCommitValidation,
                2,
                Arc::new(move || {
                    std::fs::write(&path, b"")?;
                    called.store(true, Ordering::SeqCst);
                    Ok(())
                }),
            );
        }
        assert!(session.read_empty_seal().is_err());
        record(&fixture, source(&fixture));
        assert!(
            changed.load(Ordering::SeqCst),
            "specific final reader boundary was not reached: {final_file}"
        );
        assert_eq!(b_rows(&fixture), before);
    }
}

#[test]
fn g5b_empty_seal_last_sql_legal_head_drift_rolls_back_each_exact_owner_boundary() {
    for family in ["prospective", "prepare", "commit", "read", "seal"] {
        for fault in [
            OperationPostvalidationTestFault::G5bHeadRevisionAdvance,
            OperationPostvalidationTestFault::G5bHeadArtifactStateDrift,
        ] {
            // Prepared already intentionally has Dirty state. The revision
            // fault is the meaningful additional change at this boundary.
            if family == "prepare"
                && matches!(
                    fault,
                    OperationPostvalidationTestFault::G5bHeadArtifactStateDrift
                )
            {
                continue;
            }
            let fixture = Fixture::new("EMPTY_EXACT_SQL");
            if family == "prospective" {
                fixture.g5b_input_log(DATE);
            } else {
                setup(&fixture);
            }
            let intent = if matches!(family, "commit" | "seal") {
                Some(prepared_and_published(&fixture))
            } else {
                None
            };
            if family == "seal" {
                let session = fixture.coordinator.g5b_day_session(date()).unwrap();
                session
                    .commit_prepared_artifact(intent.as_ref().unwrap())
                    .unwrap();
            }
            if family == "read" {
                seal(&fixture);
            }
            let before = b_rows(&fixture);
            let invoked = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let called = Arc::clone(&invoked);
            let owner = Arc::downgrade(fixture.coordinator.0.as_ref().unwrap());
            let count = match family {
                "prospective" => 2,
                "read" => 2,
                "seal" => 9,
                _ => 3,
            };
            after_n_sql(
                fixture.coordinator.0.as_ref().unwrap(),
                DatabaseOperationTestPhase::AfterSqlBeforePostValidation,
                count,
                Arc::new(move || {
                    owner
                        .upgrade()
                        .unwrap()
                        .install_operation_postvalidation_test_fault(fault)?;
                    called.store(true, Ordering::SeqCst);
                    Ok(())
                }),
            );
            let session = fixture.coordinator.g5b_day_session(date()).unwrap();
            let result = match family {
                "prospective" => session.initialize_empty_prospective_for_test(clock("15:04:00")),
                "prepare" => session
                    .prepare_empty_closure_for_test(clock("15:21:00"))
                    .map(|_| ()),
                "commit" => session.commit_prepared_artifact(intent.as_ref().unwrap()),
                "read" => session.read_empty_seal().map(|_| ()),
                "seal" => session
                    .close_empty_window_for_test(clock("16:00:00"))
                    .map(|_| ()),
                _ => unreachable!(),
            };
            record(&fixture, head(&fixture));
            assert!(
                invoked.load(Ordering::SeqCst),
                "actual {family} last SQL phase was not reached"
            );
            let error = result.err().unwrap().to_string();
            assert!(error.contains("exact SQL"), "{family}/{fault:?}: {error}");
            assert_eq!(
                b_rows(&fixture),
                before,
                "{family}/{fault:?} must roll back the entire transaction"
            );
        }
    }
}

#[test]
fn g5b_empty_seal_postcommit_legal_sql_drift_is_retained_and_never_mints_completion() {
    let fixture = Fixture::new("EMPTY_POSTCOMMIT_SQL");
    setup(&fixture);
    seal(&fixture);
    let path = fixture.database_path.clone();
    let invoked = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let called = Arc::clone(&invoked);
    after_n_sql(
        fixture.coordinator.0.as_ref().unwrap(),
        DatabaseOperationTestPhase::AfterCommitBeforePostValidation,
        2,
        Arc::new(move || {
            let connection = Connection::open(&path)?;
            super::super::schema::register_sha256_function(&connection)?;
            assert_eq!(
                connection.execute(
                    "UPDATE g5b_day_heads SET revision=revision+1,current_seal_identity=NULL",
                    []
                )?,
                1
            );
            called.store(true, Ordering::SeqCst);
            Ok(())
        }),
    );
    let session = fixture.coordinator.g5b_day_session(date()).unwrap();
    let error = session.read_empty_seal().err().unwrap().to_string();
    assert!(invoked.load(Ordering::SeqCst));
    assert!(
        error.contains("exact SQL witness validation failed after COMMIT succeeded"),
        "{error}"
    );
    assert_eq!(fixture.query_i64("SELECT revision FROM g5b_day_heads"), 3);
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM g5b_day_seals"), 1);
    assert!(session.read_empty_seal().unwrap().is_none());
}

#[test]
fn g5b_empty_seal_runtime_rejects_self_consistent_unsupported_seal_version_with_original_manifest()
{
    let fixture = Fixture::new("EMPTY_BAD_SEAL_CODEC");
    setup(&fixture);
    seal(&fixture);
    let connection = Connection::open(&fixture.database_path).unwrap();
    super::super::schema::register_sha256_function(&connection).unwrap();
    let (identity, bytes): (String, Vec<u8>) = connection
        .query_row(
            "SELECT seal_identity,seal_canonical FROM g5b_day_seals",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let trigger: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='g5b_day_seals_immutable_update'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let bytes = String::from_utf8(bytes)
        .unwrap()
        .replacen("\"version\":1", "\"version\":2", 1)
        .into_bytes();
    let mut preimage = b"g5b-empty-day-seal-v1\0".to_vec();
    preimage.extend_from_slice(&bytes);
    let forged_identity = sha256_hex(&preimage);
    connection
        .execute_batch("DROP TRIGGER g5b_day_seals_immutable_update")
        .unwrap();
    connection.execute("UPDATE g5b_day_seals SET seal_identity=?1,seal_canonical=?2,seal_sha256=?3,seal_preimage=?4 WHERE seal_identity=?5",params![forged_identity,bytes,sha256_hex(&bytes),preimage,identity]).unwrap();
    connection
        .execute(
            "UPDATE g5b_day_heads SET current_seal_identity=?1",
            [&forged_identity],
        )
        .unwrap();
    connection.execute_batch(&trigger).unwrap();
    drop(connection);
    let session = fixture.coordinator.g5b_day_session(date()).unwrap();
    assert!(session.read_empty_seal().is_err());
    let test_code = fixture
        .database_path
        .parent()
        .unwrap()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap();
    assert!(DurableDeliveryCoordinator::open(CoordinatorConfig::test(
        &fixture.database_path,
        test_code,
        "owner-EMPTY_BAD_SEAL_REOPEN-0123456789abcdef"
    ))
    .is_err());
}
