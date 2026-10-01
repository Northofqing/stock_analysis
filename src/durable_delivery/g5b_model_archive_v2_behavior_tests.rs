//! C3: actual isolated owner/files, no provider during archive or replay.
use super::*;
use crate::monitor::g5b_analysis_v2::{
    archive_model_observations_v2, model_archive_bytes_for_test, prepare_model_archive_for_test,
    G5bModelArchiveCoverageV2,
};
use std::os::unix::fs::PermissionsExt;

pub(super) async fn freeze_member(
    fixture: &Fixture,
    provider: &Arc<dyn LlmProvider>,
    index: usize,
) {
    let live = work(
        claim_for_test(
            owner(fixture),
            date(),
            index,
            Arc::clone(provider),
            clock("15:10:00"),
        )
        .unwrap(),
    );
    capture_snapshots(fixture);
    live.assess().await.unwrap().freeze().unwrap();
    capture_snapshots(fixture);
}
pub(super) fn artifact(fixture: &Fixture, role: &str) -> PathBuf {
    let connection = Connection::open(&fixture.database_path).unwrap();
    let (cohort,intent):(String,String)=connection.query_row(
        "SELECT cohort_identity,logical_intent FROM g5b_artifact_events WHERE artifact_role=?1 AND phase='Prepared' ORDER BY prepared_revision LIMIT 1",
        [role],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
    namespace(fixture).join(format!(
        "20260928.{cohort}.{intent}.g5b-{}.v2",
        role.to_ascii_lowercase()
    ))
}
fn archive_count(fixture: &Fixture) -> i64 {
    fixture.query_i64("SELECT COUNT(*) FROM g5b_artifact_events WHERE artifact_role='Archive' AND phase='Prepared'")
}
fn tables(fixture: &Fixture) -> Vec<Vec<String>> {
    let connection = Connection::open(&fixture.database_path).unwrap();
    super::super::super::schema_g5b_cohort::TABLES
        .iter()
        .flat_map(|table| authority_table_rows(&connection, table))
        .collect()
}
pub(super) fn replacement(fixture: &Fixture, path: &Path) -> (PathBuf, PathBuf) {
    let replacement = path.with_extension("TEST_CODE_REPLACEMENT");
    let aside = path.with_extension("TEST_CODE_ORIGINAL_ASIDE");
    std::fs::write(&replacement, std::fs::read(path).unwrap()).unwrap();
    std::fs::set_permissions(&replacement, std::fs::Permissions::from_mode(0o400)).unwrap();
    fixture
        .cleanup
        .record(&replacement, OwnedPathKind::FileOrSymlink);
    (replacement, aside)
}
pub(super) fn replace(path: &Path, replacement: &Path, aside: &Path) {
    std::fs::rename(path, aside).unwrap();
    std::fs::rename(replacement, path).unwrap();
}
pub(super) fn capture_replaced(fixture: &Fixture, path: &Path, aside: &Path) {
    fixture.cleanup.record(path, OwnedPathKind::FileOrSymlink);
    fixture.cleanup.record(aside, OwnedPathKind::FileOrSymlink);
}
pub(super) fn arm_nth_sql(
    weak: std::sync::Weak<DurableDeliveryCoordinator>,
    remaining: usize,
    action: Arc<Mutex<Option<Box<dyn FnOnce() + Send>>>>,
    hit: Arc<AtomicUsize>,
) {
    let coordinator = weak.upgrade().unwrap();
    coordinator
        .install_database_operation_test_hook(
            DatabaseOperationTestPhase::AfterSqlBeforePreCommitValidation,
            move || {
                if remaining == 1 {
                    hit.fetch_add(1, Ordering::SeqCst);
                    action.lock().unwrap().take().unwrap()();
                } else {
                    arm_nth_sql(weak, remaining - 1, action, hit);
                }
                Ok(())
            },
        )
        .unwrap();
}

#[tokio::test]
async fn g5b_model_archive_v2_partial_then_full_are_immutable_and_identical_raw_members_remain_distinct(
) {
    let fixture = Fixture::new("C3_PARTIAL_FULL");
    let raw = raw("600001");
    let log = input(&fixture, &[raw.clone(), raw]);
    let (provider, calls) = model(&log, Mode::Good);
    freeze_member(&fixture, &provider, 0).await;
    let partial = archive_model_observations_v2(owner(&fixture), date())
        .unwrap()
        .unwrap();
    capture_snapshots(&fixture);
    assert_eq!(
        (
            partial.version(),
            partial.archived_count(),
            partial.selected_count()
        ),
        (1, 1, 2)
    );
    assert_eq!(partial.coverage(), G5bModelArchiveCoverageV2::Partial);
    let first_bytes = partial.canonical_bytes().to_vec();
    let first_path = artifact(&fixture, "Archive");
    let first_identity = partial.archive_identity().to_owned();
    freeze_member(&fixture, &provider, 1).await;
    let full = archive_model_observations_v2(owner(&fixture), date())
        .unwrap()
        .unwrap();
    capture_snapshots(&fixture);
    assert_eq!(
        (full.version(), full.archived_count(), full.selected_count()),
        (2, 2, 2)
    );
    assert_eq!(full.coverage(), G5bModelArchiveCoverageV2::Full);
    assert_ne!(full.archive_identity(), first_identity);
    assert_eq!(std::fs::read(first_path).unwrap(), first_bytes);
    let json: serde_json::Value = serde_json::from_slice(full.canonical_bytes()).unwrap();
    assert_eq!(json["previous_archive_identity"], first_identity);
    assert_ne!(
        json["items"][0]["occurrence_identity"],
        json["items"][1]["occurrence_identity"]
    );
    assert_ne!(
        json["items"][0]["attempt_identity"],
        json["items"][1]["attempt_identity"]
    );
    assert_eq!(archive_count(&fixture), 2);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM delivery_decisions"),
        0
    );
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM g5b_day_seals"), 0);
}

#[tokio::test]
async fn g5b_model_archive_v2_restart_replays_exact_bytes_without_provider_or_new_version() {
    let fixture = Fixture::new("C3_READ_RESTART");
    let log = input(&fixture, &[raw("600001")]);
    let (provider, calls) = model(&log, Mode::Good);
    freeze_member(&fixture, &provider, 0).await;
    let original = archive_model_observations_v2(owner(&fixture), date())
        .unwrap()
        .unwrap();
    capture_snapshots(&fixture);
    let before = tables(&fixture);
    let recovered = archive_model_observations_v2(fixture.second_coordinator("C3_RESTART"), date())
        .unwrap()
        .unwrap();
    assert_eq!(recovered.canonical_bytes(), original.canonical_bytes());
    assert_eq!(recovered.archive_identity(), original.archive_identity());
    assert_eq!(recovered.version(), 1);
    assert_eq!(tables(&fixture), before);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn g5b_model_archive_v2_prepared_recovery_preserves_original_partial_before_new_full_version()
{
    let fixture = Fixture::new("C3_PREPARED_RECOVERY");
    let log = input(&fixture, &[raw("600001"), raw("600002")]);
    let (provider, calls) = model(&log, Mode::Good);
    freeze_member(&fixture, &provider, 0).await;
    let prepared = prepare_model_archive_for_test(owner(&fixture), date()).unwrap();
    let saved = prepared.desired_bytes().to_vec();
    assert_eq!(archive_count(&fixture), 1);
    assert!(!artifact(&fixture, "Archive").exists());
    // Another real completed member may arrive after the original Prepared
    // payload. Recovery cannot rewrite the old partial intent to current full.
    freeze_member(&fixture, &provider, 1).await;
    let recovered =
        archive_model_observations_v2(fixture.second_coordinator("C3_PREPARED"), date())
            .unwrap()
            .unwrap();
    capture_snapshots(&fixture);
    assert_eq!(recovered.coverage(), G5bModelArchiveCoverageV2::Full);
    assert_eq!(recovered.version(), 2);
    assert_eq!(std::fs::read(artifact(&fixture, "Archive")).unwrap(), saved);
    let connection = Connection::open(&fixture.database_path).unwrap();
    let (bytes,count):(Vec<u8>,i64)=connection.query_row(
        "SELECT p.desired_bytes,(SELECT COUNT(*) FROM g5b_artifact_events c WHERE c.prepared_event_identity=p.event_identity AND c.phase='Committed') FROM g5b_artifact_events p WHERE p.logical_intent=?1 AND p.phase='Prepared'",
        [prepared.identity()],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
    assert_eq!(bytes, saved);
    assert_eq!(count, 1);
    assert_eq!(archive_count(&fixture), 2);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn g5b_model_archive_v2_published_prepared_recovery_uses_original_file_not_a_replacement() {
    let fixture = Fixture::new("C3_PUBLISHED_PREPARED");
    let log = input(&fixture, &[raw("600001")]);
    let (provider, calls) = model(&log, Mode::Good);
    freeze_member(&fixture, &provider, 0).await;
    let prepared = prepare_model_archive_for_test(owner(&fixture), date()).unwrap();
    let session = fixture.coordinator.g5b_day_session(date()).unwrap();
    session.publish_prepared_artifact(&prepared).unwrap();
    drop(session);
    capture_snapshots(&fixture);
    let path = artifact(&fixture, "Archive");
    use std::os::unix::fs::MetadataExt;
    let inode = path.metadata().unwrap().ino();
    let recovered = archive_model_observations_v2(owner(&fixture), date())
        .unwrap()
        .unwrap();
    capture_snapshots(&fixture);
    assert_eq!(recovered.canonical_bytes(), prepared.desired_bytes());
    assert_eq!(path.metadata().unwrap().ino(), inode);
    assert_eq!(archive_count(&fixture), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn g5b_model_archive_v2_committed_missing_or_same_bytes_new_inode_is_never_healed() {
    for missing in [true, false] {
        let fixture = Fixture::new("C3_ARCHIVE_INODE");
        let log = input(&fixture, &[raw("600001")]);
        let (provider, calls) = model(&log, Mode::Good);
        freeze_member(&fixture, &provider, 0).await;
        archive_model_observations_v2(owner(&fixture), date())
            .unwrap()
            .unwrap();
        capture_snapshots(&fixture);
        let path = artifact(&fixture, "Archive");
        if missing {
            std::fs::remove_file(&path).unwrap();
        } else {
            let (replacement, aside) = replacement(&fixture, &path);
            replace(&path, &replacement, &aside);
            capture_replaced(&fixture, &path, &aside);
        }
        let before = tables(&fixture);
        assert!(archive_model_observations_v2(owner(&fixture), date()).is_err());
        assert_eq!(tables(&fixture), before);
        if missing {
            assert!(!path.exists());
        }
        assert_eq!(archive_count(&fixture), 1);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn g5b_model_archive_v2_actual_bundle_rechecks_all_model_files_after_last_reader_sql_hook() {
    for role in ["Attempt", "Frozen"] {
        let fixture = Fixture::new("C3_BUNDLE_READER_LAST");
        let log = input(&fixture, &[raw("600001")]);
        let (provider, calls) = model(&log, Mode::Good);
        freeze_member(&fixture, &provider, 0).await;
        let path = artifact(&fixture, role);
        let (replacement, aside) = replacement(&fixture, &path);
        let before = tables(&fixture);
        let original = path.clone();
        let replacement_path = replacement.clone();
        let aside_path = aside.clone();
        let hit = Arc::new(AtomicUsize::new(0));
        arm_nth_sql(
            Arc::downgrade(&owner(&fixture)),
            1,
            Arc::new(Mutex::new(Some(Box::new(move || {
                replace(&original, &replacement_path, &aside_path)
            })))),
            Arc::clone(&hit),
        );
        assert!(archive_model_observations_v2(owner(&fixture), date()).is_err());
        assert_eq!(hit.load(Ordering::SeqCst), 1);
        capture_replaced(&fixture, &path, &aside);
        assert_eq!(tables(&fixture), before);
        assert_eq!(archive_count(&fixture), 0);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn g5b_model_archive_v2_after_archive_prepare_sql_model_or_prefix_mutation_rolls_back_registration(
) {
    for role in ["Attempt", "Frozen", "Source"] {
        let fixture = Fixture::new("C3_ARCHIVE_PRECOMMIT");
        let log = input(&fixture, &[raw("600001")]);
        let (provider, calls) = model(&log, Mode::Good);
        freeze_member(&fixture, &provider, 0).await;
        let path = if role == "Source" {
            namespace(&fixture).join("20260928.jsonl")
        } else {
            artifact(&fixture, role)
        };
        let pair = if role == "Source" {
            None
        } else {
            Some(replacement(&fixture, &path))
        };
        let original = path.clone();
        let owned_pair = pair.clone();
        let before = tables(&fixture);
        let hit = Arc::new(AtomicUsize::new(0));
        arm_nth_sql(
            Arc::downgrade(&owner(&fixture)),
            2,
            Arc::new(Mutex::new(Some(Box::new(move || {
                if let Some((replacement, aside)) = owned_pair {
                    replace(&original, &replacement, &aside);
                } else {
                    std::fs::OpenOptions::new()
                        .append(true)
                        .open(original)
                        .unwrap()
                        .write_all(b"uncommitted suffix\n")
                        .unwrap();
                }
            })))),
            Arc::clone(&hit),
        );
        assert!(archive_model_observations_v2(owner(&fixture), date()).is_err());
        assert_eq!(hit.load(Ordering::SeqCst), 1);
        if let Some((_, aside)) = pair {
            capture_replaced(&fixture, &path, &aside);
        }
        assert_eq!(
            tables(&fixture),
            before,
            "{role} after actual archive INSERT is fully rolled back"
        );
        assert_eq!(archive_count(&fixture), 0);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn g5b_model_archive_v2_no_completed_models_do_not_manufacture_empty_or_archive_facts() {
    let fixture = Fixture::new("C3_NO_MODEL");
    let log = input(&fixture, &[raw("600001")]);
    let (provider, calls) = model(&log, Mode::PanicIfCalled);
    let live =
        work(claim_for_test(owner(&fixture), date(), 0, provider, clock("15:10:00")).unwrap());
    capture_snapshots(&fixture);
    drop(live);
    let before = tables(&fixture);
    assert!(archive_model_observations_v2(owner(&fixture), date())
        .unwrap()
        .is_none());
    assert_eq!(tables(&fixture), before);
    assert_eq!(archive_count(&fixture), 0);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM g5b_day_seals"), 0);
}

#[tokio::test]
async fn g5b_model_archive_v2_final_commit_hook_revalidates_original_attempt_and_frozen_bundle() {
    for role in ["Attempt", "Frozen"] {
        let fixture = Fixture::new("C3_ARCHIVE_COMMIT_LAST");
        let log = input(&fixture, &[raw("600001")]);
        let (provider, calls) = model(&log, Mode::Good);
        freeze_member(&fixture, &provider, 0).await;
        prepare_model_archive_for_test(owner(&fixture), date()).unwrap();
        let path = artifact(&fixture, role);
        let (replacement, aside) = replacement(&fixture, &path);
        let original = path.clone();
        let replacement_path = replacement.clone();
        let aside_path = aside.clone();
        let before = tables(&fixture);
        let hit = Arc::new(AtomicUsize::new(0));
        // Actual recovery: bundle read (1), publish validation (2/3), commit
        // validation (4/5), saved preimages (6), actual Commit INSERT (7).
        arm_nth_sql(
            Arc::downgrade(&owner(&fixture)),
            7,
            Arc::new(Mutex::new(Some(Box::new(move || {
                replace(&original, &replacement_path, &aside_path)
            })))),
            Arc::clone(&hit),
        );
        assert!(archive_model_observations_v2(owner(&fixture), date()).is_err());
        assert_eq!(hit.load(Ordering::SeqCst), 1);
        capture_replaced(&fixture, &path, &aside);
        capture_snapshots(&fixture);
        assert_eq!(
            tables(&fixture),
            before,
            "actual Archive Commit rolls back after {role} replacement"
        );
        assert_eq!(archive_count(&fixture), 1);
        assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM g5b_artifact_events WHERE artifact_role='Archive' AND phase='Committed'"), 0);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn g5b_model_archive_v2_arbitrary_opaque_archive_unknown_schema_or_false_full_is_not_adopted()
{
    for attack in ["unknown", "duplicate", "false_full"] {
        let fixture = Fixture::new("C3_CLOSED_CODEC");
        let log = input(&fixture, &[raw("600001"), raw("600002")]);
        let (provider, calls) = model(&log, Mode::Good);
        freeze_member(&fixture, &provider, 0).await;
        let original = model_archive_bytes_for_test(owner(&fixture), date()).unwrap();
        let text = std::str::from_utf8(&original).unwrap();
        let changed = match attack {
            "unknown" => format!("{{\"caller_fact\":true,{}", &text[1..]),
            "duplicate" => format!("{{\"version\":1,{}", &text[1..]),
            _ => text.replace("\"coverage\":\"partial\"", "\"coverage\":\"full\""),
        };
        let session = fixture.coordinator.g5b_day_session(date()).unwrap();
        let cohort = session.read_cohort().unwrap().unwrap();
        session
            .prepare_opaque_snapshot(&cohort, G5bSnapshotKind::Archive, None, changed.as_bytes())
            .unwrap();
        drop(session);
        let before = tables(&fixture);
        assert!(archive_model_observations_v2(owner(&fixture), date()).is_err());
        assert_eq!(tables(&fixture), before);
        assert_eq!(archive_count(&fixture), 1);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
