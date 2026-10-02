//! Real isolated facade routing, recovery and completion; no constructed seal.
#![cfg(unix)]
use super::*;
use crate::monitor::alert_log::{AlertLog, AlertRecord};
use crate::monitor::g5b_empty_v2::{self as facade, G5bEmptyInspectionV2, G5bEmptySealV2};
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
fn coordinator(fixture: &Fixture) -> Arc<DurableDeliveryCoordinator> {
    Arc::clone(fixture.coordinator.0.as_ref().unwrap())
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
        .record_if_present(path, OwnedPathKind::FileOrSymlink);
}
fn rows(fixture: &Fixture) -> Vec<Vec<String>> {
    let connection = Connection::open(&fixture.database_path).unwrap();
    super::super::schema_g5b_cohort::TABLES
        .iter()
        .flat_map(|table| authority_table_rows(&connection, table))
        .collect()
}
fn setup(fixture: &Fixture) -> AlertLog {
    let log = fixture.g5b_input_log(DATE);
    facade::initialize_empty_day_for_test(coordinator(fixture), date(), clock("15:04:00")).unwrap();
    record(fixture, head(fixture));
    log
}
fn selection(fixture: &Fixture) -> PathBuf {
    let connection = Connection::open(&fixture.database_path).unwrap();
    let (cohort, logical): (String, String) = connection
        .query_row(
            "SELECT cohort_identity,logical_intent FROM g5b_artifact_events WHERE phase='Prepared'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    namespace(fixture).join(format!("20260928.{cohort}.{logical}.g5b-selection.v2"))
}
fn prepare(fixture: &Fixture) -> PreparedG5bArtifact {
    let session = fixture.coordinator.g5b_day_session(date()).unwrap();
    session
        .prepare_empty_closure_for_test(clock("15:21:00"))
        .unwrap()
}
fn sealed(fixture: &Fixture) -> G5bEmptySealV2 {
    let cap =
        facade::close_empty_day_for_test(coordinator(fixture), date(), clock("15:21:00")).unwrap();
    record(fixture, selection(fixture));
    cap
}
fn pending(fixture: &Fixture, revision: i64) {
    match facade::inspect_empty_day_v2(coordinator(fixture), date()).unwrap() {
        G5bEmptyInspectionV2::EmptyPending(value) => {
            assert_eq!(value.business_date(), date());
            assert_eq!(value.revision(), revision);
            assert_eq!(value.cohort_identity().len(), 64);
        }
        _ => panic!("actual saved Empty must be classified as pending"),
    }
}
fn no_delivery(fixture: &Fixture) {
    for table in [
        "delivery_decisions",
        "delivery_attempts",
        "sink_results",
        "immutable_audit_outbox",
    ] {
        assert_eq!(
            fixture.query_i64(&format!("SELECT COUNT(*) FROM {table}")),
            0
        );
    }
}
fn append(fixture: &Fixture, log: &AlertLog, local: &str) {
    let value: AlertRecord = serde_json::from_value(serde_json::json!({
        "origin":"production","triggered_at":format!("{DATE}T{local}+08:00"),
        "code":"TEST_CODE_FACADE_SUFFIX","name":"TEST_CODE_FACADE_SUFFIX",
        "level":"重要","category":"TEST_CODE_FACADE_SUFFIX",
        "message":"actual late input","t1_locked":false
    }))
    .unwrap();
    let mut bytes = serde_json::to_vec(&value).unwrap();
    bytes.push(b'\n');
    log.append_test_date_raw_production_fixture(date(), &bytes)
        .unwrap();
    record(fixture, source(fixture));
    record(fixture, head(fixture));
}

// This Test-only configured object creates a real NonEmpty admission for the
// routing regression. It is never involved in any Empty completion path.
struct ConfiguredButNeverCalled;
#[async_trait::async_trait]
impl crate::llm::LlmProvider for ConfiguredButNeverCalled {
    fn name(&self) -> &'static str {
        "TEST_CODE_EMPTY_FACADE_NONEMPTY_ROUTE"
    }
    fn model(&self) -> &str {
        "TEST_CODE_NEVER_CALLED"
    }
    async fn chat_json(
        &self,
        _: &str,
        _: &str,
    ) -> std::result::Result<serde_json::Value, crate::llm::LlmError> {
        panic!("routing must not invoke a model")
    }
}

#[test]
fn g5b_empty_facade_nonempty_route_is_not_an_empty_or_model_capability() {
    let fixture = Fixture::new("EMPTY_FACADE_NONEMPTY");
    let log = setup(&fixture);
    append(&fixture, &log, "15:00:00");
    let target = {
        let session = fixture.coordinator.g5b_day_session(date()).unwrap();
        let ready = session
            .configured_analysis_for_test(Arc::new(ConfiguredButNeverCalled), clock("15:10:00"))
            .unwrap();
        let intent = session.prepare_cohort(&ready).unwrap();
        session.publish_prepared_artifact(&intent).unwrap();
        session.commit_prepared_artifact(&intent).unwrap();
        session.read_cohort().unwrap().unwrap();
        selection(&fixture)
    };
    record(&fixture, &target);
    let before = rows(&fixture);
    assert!(matches!(
        facade::inspect_empty_day_v2(coordinator(&fixture), date()).unwrap(),
        G5bEmptyInspectionV2::NonEmpty
    ));
    assert!(matches!(
        facade::recover_empty_day_v2(coordinator(&fixture), date()).unwrap(),
        G5bEmptyInspectionV2::NonEmpty
    ));
    assert!(facade::close_empty_day_v2(coordinator(&fixture), date()).is_err());
    assert_eq!(rows(&fixture), before);
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM g5b_day_seals"), 0);
    // The route remains low authority. The subsequent actual NonEmpty reader
    // must reject a corrupted committed artifact instead of trusting a kind.
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(&target, b"corrupt actual NonEmpty Selection").unwrap();
    assert!(matches!(
        facade::inspect_empty_day_v2(coordinator(&fixture), date()).unwrap(),
        G5bEmptyInspectionV2::NonEmpty
    ));
    let session = fixture.coordinator.g5b_day_session(date()).unwrap();
    assert!(session.read_cohort().is_err());
    no_delivery(&fixture);
}

#[test]
fn g5b_empty_facade_distinguishes_absent_prepared_committed_pending_and_actual_seal() {
    let fixture = Fixture::new("EMPTY_FACADE_ROUTE");
    fixture.g5b_input_log(DATE);
    assert!(matches!(
        facade::inspect_empty_day_v2(coordinator(&fixture), date()).unwrap(),
        G5bEmptyInspectionV2::Absent
    ));
    assert!(matches!(
        facade::recover_empty_day_v2(coordinator(&fixture), date()).unwrap(),
        G5bEmptyInspectionV2::Absent
    ));
    assert!(!head(&fixture).exists());
    setup(&fixture);
    assert!(matches!(
        facade::inspect_empty_day_v2(coordinator(&fixture), date()).unwrap(),
        G5bEmptyInspectionV2::Absent
    ));
    let intent = prepare(&fixture);
    let original = intent.desired_bytes().to_vec();
    assert!(!selection(&fixture).exists());
    let prepared = rows(&fixture);
    pending(&fixture, 1);
    assert_eq!(rows(&fixture), prepared);
    assert!(!selection(&fixture).exists(), "inspection must not publish");
    assert!(matches!(
        facade::recover_empty_day_v2(coordinator(&fixture), date()).unwrap(),
        G5bEmptyInspectionV2::EmptyPending(_)
    ));
    record(&fixture, selection(&fixture));
    assert_eq!(std::fs::read(selection(&fixture)).unwrap(), original);
    pending(&fixture, 2);
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM g5b_day_seals"), 0);
    assert_eq!(
        fixture.query_i64(
            "SELECT COUNT(*) FROM g5b_day_heads WHERE current_seal_identity IS NOT NULL"
        ),
        0
    );
    let committed = rows(&fixture);
    facade::recover_empty_day_v2(coordinator(&fixture), date()).unwrap();
    assert_eq!(rows(&fixture), committed, "exact recovery is a no-op");
    // The production overload cannot use the historical test clock to mint a
    // first seal. Only the existing Test owner can supply this fixture clock.
    assert!(facade::close_empty_day_v2(coordinator(&fixture), date()).is_err());
    let cap = sealed(&fixture);
    match facade::inspect_empty_day_v2(coordinator(&fixture), date()).unwrap() {
        G5bEmptyInspectionV2::EmptySealed(value) => {
            assert_eq!(value.identity(), cap.identity());
            assert_eq!(value.sha256(), cap.sha256());
            assert_eq!(value.revision(), 2);
            assert_eq!(value.reason(), "NoEligibleInVerifiedClosedWindowPrefix");
        }
        _ => panic!("only an actual verified seal grants completion"),
    }
    let before = rows(&fixture);
    let later = facade::close_empty_day_v2(coordinator(&fixture), date()).unwrap();
    assert_eq!(later.identity(), cap.identity());
    assert_eq!(rows(&fixture), before);
    no_delivery(&fixture);
}

#[test]
fn g5b_empty_facade_init_does_not_backfill_an_existing_head_or_use_a_late_clock() {
    for existing in [false, true] {
        let fixture = Fixture::new("EMPTY_FACADE_NO_BACKFILL");
        let log = fixture.g5b_input_log(DATE);
        if existing {
            log.initialize_date_input_head(date()).unwrap();
            record(&fixture, head(&fixture));
        }
        let before = rows(&fixture);
        let bytes = std::fs::read(head(&fixture)).ok();
        let attempted = if existing { "15:04:00" } else { "15:21:00" };
        assert!(facade::initialize_empty_day_for_test(
            coordinator(&fixture),
            date(),
            clock(attempted)
        )
        .is_err());
        assert!(facade::initialize_empty_day_v2(coordinator(&fixture), date()).is_err());
        assert_eq!(std::fs::read(head(&fixture)).ok(), bytes);
        assert_eq!(rows(&fixture), before);
        no_delivery(&fixture);
    }
}

#[test]
fn g5b_empty_facade_pending_rejects_conflicting_and_alias_publications_without_writing() {
    for alias in [false, true] {
        let fixture = Fixture::new("EMPTY_FACADE_BAD_PENDING");
        setup(&fixture);
        let intent = prepare(&fixture);
        {
            let session = fixture.coordinator.g5b_day_session(date()).unwrap();
            session.publish_prepared_artifact(&intent).unwrap();
        }
        let target = selection(&fixture);
        record(&fixture, &target);
        if alias {
            let foreign = namespace(&fixture).join("TEST_CODE_foreign_selection");
            std::fs::write(&foreign, intent.desired_bytes()).unwrap();
            record(&fixture, &foreign);
            std::fs::remove_file(&target).unwrap();
            std::os::unix::fs::symlink(&foreign, &target).unwrap();
            record(&fixture, &target);
        } else {
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
            std::fs::write(&target, b"conflicting saved leaf").unwrap();
        }
        let before = rows(&fixture);
        assert!(facade::inspect_empty_day_v2(coordinator(&fixture), date()).is_err());
        assert!(facade::recover_empty_day_v2(coordinator(&fixture), date()).is_err());
        assert_eq!(rows(&fixture), before);
        assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM g5b_day_seals"), 0);
        assert!(target.symlink_metadata().is_ok());
        no_delivery(&fixture);
    }
}

#[test]
fn g5b_empty_facade_committed_missing_or_same_bytes_new_inode_is_never_healed() {
    for replaced in [false, true] {
        let fixture = Fixture::new("EMPTY_FACADE_NO_HEAL");
        setup(&fixture);
        prepare(&fixture);
        facade::recover_empty_day_v2(coordinator(&fixture), date()).unwrap();
        let target = selection(&fixture);
        record(&fixture, &target);
        let bytes = std::fs::read(&target).unwrap();
        let inode = target.metadata().unwrap().ino();
        let old = namespace(&fixture).join("TEST_CODE_original_selection");
        std::fs::rename(&target, &old).unwrap();
        record(&fixture, &old);
        if replaced {
            std::fs::write(&target, &bytes).unwrap();
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o400)).unwrap();
            record(&fixture, &target);
            assert_ne!(target.metadata().unwrap().ino(), inode);
        }
        let before = rows(&fixture);
        assert!(facade::inspect_empty_day_v2(coordinator(&fixture), date()).is_err());
        assert!(facade::recover_empty_day_v2(coordinator(&fixture), date()).is_err());
        assert_eq!(rows(&fixture), before);
        assert_eq!(target.exists(), replaced);
        no_delivery(&fixture);
    }
}

#[test]
fn g5b_empty_facade_refresh_preserves_closed_prefix_but_rejects_known_source_and_database_conflicts(
) {
    let fixture = Fixture::new("EMPTY_FACADE_REFRESH");
    let log = setup(&fixture);
    let zero = sealed(&fixture);
    append(&fixture, &log, "15:30:00");
    let positive = facade::refresh_empty_day_v2(coordinator(&fixture), &zero).unwrap();
    assert_eq!(positive.identity(), zero.identity());
    assert_eq!(positive.cohort_identity(), zero.cohort_identity());
    assert_eq!(positive.revision(), zero.revision());
    let reopened = fixture.second_coordinator("EMPTY_FACADE_REFRESH_REOPEN");
    facade::refresh_empty_day_v2(reopened, &positive).unwrap();
    let other = Fixture::new("EMPTY_FACADE_OTHER_DB");
    setup(&other);
    sealed(&other);
    assert!(facade::refresh_empty_day_v2(coordinator(&other), &positive).is_err());
    let old = namespace(&fixture).join("TEST_CODE_original_input");
    let bytes = std::fs::read(source(&fixture)).unwrap();
    std::fs::rename(source(&fixture), &old).unwrap();
    record(&fixture, &old);
    std::fs::write(source(&fixture), bytes).unwrap();
    record(&fixture, source(&fixture));
    let before = rows(&fixture);
    assert!(facade::refresh_empty_day_v2(coordinator(&fixture), &positive).is_err());
    assert_eq!(rows(&fixture), before);
    no_delivery(&fixture);
    no_delivery(&other);
}

fn after_n_sql(
    owner: &Arc<DurableDeliveryCoordinator>,
    remaining: usize,
    action: Arc<dyn Fn() -> Result<()> + Send + Sync>,
) {
    let weak = Arc::downgrade(owner);
    owner
        .install_database_operation_test_hook(
            DatabaseOperationTestPhase::AfterSqlBeforePostValidation,
            move || {
                if remaining == 1 {
                    action()
                } else {
                    after_n_sql(&weak.upgrade().unwrap(), remaining - 1, action);
                    Ok(())
                }
            },
        )
        .unwrap();
}

#[test]
fn g5b_empty_facade_pending_last_sql_drift_rolls_back_and_final_file_fault_cannot_return_stale_pending(
) {
    for file_fault in [false, true] {
        let fixture = Fixture::new("EMPTY_FACADE_FINAL_BOUNDARY");
        setup(&fixture);
        prepare(&fixture);
        let before = rows(&fixture);
        let hit = Arc::new(std::sync::atomic::AtomicBool::new(false));
        if file_fault {
            let changed = Arc::clone(&hit);
            let path = head(&fixture);
            let session = fixture.coordinator.g5b_day_session(date()).unwrap();
            session
                .install_empty_final_input_fault_for_test(
                    Arc::new(std::sync::atomic::AtomicBool::new(true)),
                    1,
                    move || {
                        std::fs::write(&path, b"corrupt actual head after artifact read")?;
                        changed.store(true, Ordering::SeqCst);
                        Ok(())
                    },
                )
                .unwrap();
        } else {
            let owner = coordinator(&fixture);
            let weak = Arc::downgrade(&owner);
            let changed = Arc::clone(&hit);
            // Routing read, Empty snapshot read, then exact pending SQL read.
            after_n_sql(
                &owner,
                3,
                Arc::new(move || {
                    weak.upgrade()
                        .unwrap()
                        .install_operation_postvalidation_test_fault(
                            OperationPostvalidationTestFault::G5bHeadRevisionAdvance,
                        )?;
                    changed.store(true, Ordering::SeqCst);
                    Ok(())
                }),
            );
        }
        assert!(facade::inspect_empty_day_v2(coordinator(&fixture), date()).is_err());
        assert!(
            hit.load(Ordering::SeqCst),
            "actual final boundary was not hit"
        );
        assert_eq!(rows(&fixture), before);
        no_delivery(&fixture);
    }
}
