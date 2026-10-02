//! Actual input/model/Archive/owner/physical sink/immutable append protocol.
//! No hand-written success seal, caller completion boolean or SQL factory.
use super::model_archive_v2_tests::{artifact, capture_replaced, freeze_member, replace};
use super::*;
use crate::monitor::g5b_analysis_v2::{
    archive_model_observations_v2, prepare_model_owner_v2_for_test,
};

pub(super) fn snapshot(fixture: &Fixture) -> BTreeMap<String, Vec<Vec<String>>> {
    let connection = Connection::open(&fixture.database_path).unwrap();
    let mut query=connection.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap();
    let names = query
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    authority_snapshot(
        &connection,
        &names.iter().map(String::as_str).collect::<Vec<_>>(),
    )
}
pub(super) fn envelopes(fixture: &Fixture) -> Vec<DeliveryEnvelope> {
    let connection = Connection::open(&fixture.database_path).unwrap();
    let mut query=connection.prepare("SELECT o.envelope_canonical FROM g5b_occurrence_owners o JOIN g5b_selected_occurrences m ON m.occurrence_identity=o.occurrence_identity ORDER BY m.selection_index").unwrap();
    let values = query
        .query_map([], |r| r.get::<_, Vec<u8>>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    values
        .into_iter()
        .map(|v| serde_json::from_slice(&v).unwrap())
        .collect()
}

// Stage on the original filesystem outside the closed date-artifact leaves.
// A date-prefixed replacement would be rejected before the intended fault;
// the actual rename still changes the original source's inode and retains it.
fn physical_replacement(fixture: &Fixture, path: &Path) -> (PathBuf, PathBuf) {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let staging = namespace(fixture).join("TEST_CODE_D2_PHYSICAL_FAULT_STAGE");
    std::fs::create_dir(&staging).unwrap();
    std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o700)).unwrap();
    fixture.cleanup.record(&staging, OwnedPathKind::Directory);
    let replacement = staging.join("TEST_CODE_REPLACEMENT");
    let aside = staging.join("TEST_CODE_ORIGINAL_ASIDE");
    let original = std::fs::metadata(path).unwrap();
    std::fs::write(&replacement, std::fs::read(path).unwrap()).unwrap();
    std::fs::set_permissions(&replacement, std::fs::Permissions::from_mode(0o400)).unwrap();
    let replaced = std::fs::metadata(&replacement).unwrap();
    assert_eq!(
        original.dev(),
        replaced.dev(),
        "fault rename must stay on the same filesystem"
    );
    assert_ne!(
        original.ino(),
        replaced.ino(),
        "fault must install a real new inode"
    );
    fixture
        .cleanup
        .record(&replacement, OwnedPathKind::FileOrSymlink);
    (replacement, aside)
}

fn sealed(fixture: &Fixture) -> VerifiedG5bPhysicalSeal {
    let coordinator = owner(fixture);
    let session = coordinator.g5b_day_session(date()).unwrap();
    match session.try_seal_physical_cohort().unwrap() {
        G5bPhysicalSealAttempt::Sealed(v) => v,
        G5bPhysicalSealAttempt::Incomplete => panic!("actual complete protocol must seal"),
    }
}
fn incomplete(fixture: &Fixture) {
    let before = snapshot(fixture);
    let coordinator = owner(fixture);
    assert!(matches!(
        coordinator
            .g5b_day_session(date())
            .unwrap()
            .try_seal_physical_cohort()
            .unwrap(),
        G5bPhysicalSealAttempt::Incomplete
    ));
    assert_eq!(
        snapshot(fixture),
        before,
        "incomplete observation must not write a seal or revision"
    );
}
pub(super) async fn frozen(fixture: &Fixture, count: usize) -> (AlertLog, Arc<AtomicUsize>) {
    let line = raw("600001");
    let log = input(fixture, &vec![line; count]);
    let (provider, calls) = model(&log, Mode::Good);
    for index in 0..count {
        freeze_member(fixture, &provider, index).await;
    }
    archive_model_observations_v2(owner(fixture), date())
        .unwrap()
        .unwrap();
    capture_snapshots(fixture);
    (log, calls)
}
pub(super) fn prepare(fixture: &Fixture, count: usize, sinks: usize) {
    for index in 0..count {
        prepare_model_owner_v2_for_test(owner(fixture), date(), index, sinks, clock("15:12:00"))
            .unwrap();
    }
}
pub(super) fn deliver(
    fixture: &Fixture,
    append: &MemoryAppendPort,
    result: AuthoritativeSinkResult,
) -> Arc<StaticSink> {
    let sink = StaticSink::new(result);
    let sinks: [AuthoritativeSink; 1] = [sink.clone()];
    for envelope in envelopes(fixture) {
        fixture
            .coordinator
            .resume_deliverable(&envelope.decision_identity, &sinks, clock("15:13:00"))
            .unwrap();
    }
    fixture
        .coordinator
        .reconcile_all_pending(append, clock("15:13:01"))
        .unwrap();
    sink
}
pub(super) fn attempt(fixture: &Fixture) -> (String, i64) {
    Connection::open(&fixture.database_path).unwrap().query_row("SELECT attempt_identity,fence_token FROM delivery_attempts ORDER BY attempt_no DESC LIMIT 1",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap()
}
fn pointer(fixture: &Fixture) -> Option<String> {
    Connection::open(&fixture.database_path)
        .unwrap()
        .query_row("SELECT current_seal_identity FROM g5b_day_heads", [], |r| {
            r.get(0)
        })
        .unwrap()
}

#[tokio::test]
async fn g5b_physical_seal_actual_one_two_three_original_members_restart_and_exact_noop() {
    for count in 1..=3 {
        let fixture = Fixture::new("D2_COMPLETE_SELECTED");
        let (_, calls) = frozen(&fixture, count).await;
        prepare(&fixture, count, 1);
        let append = MemoryAppendPort::default();
        let sink = deliver(
            &fixture,
            &append,
            AuthoritativeSinkResult::Accepted(receipt(clock("15:13:00"))),
        );
        assert_eq!(sink.calls.load(Ordering::SeqCst), count);
        let before_revision = fixture.query_i64("SELECT revision FROM g5b_day_heads");
        let cap = sealed(&fixture);
        let [inner, compressed, stored] = fixture
            .coordinator
            .physical_codec_sizes_for_test(date())
            .unwrap();
        assert!(inner < 32 * 1024 * 1024);
        assert!(
            stored < inner,
            "actual model evidence must retain lossless compact storage"
        );
        assert!(compressed < stored);
        let history = fixture
            .coordinator
            .physical_history_validation_usage_for_test()
            .unwrap();
        let parts = fixture
            .coordinator
            .physical_snapshot_parts_for_test(date())
            .unwrap();
        assert!(history <= 32 * 1024 * 1024);
        assert!(parts.iter().sum::<usize>() <= 32 * 1024 * 1024);
        let arena = fixture
            .coordinator
            .physical_arena_stats_for_test(date())
            .unwrap();
        assert_eq!(arena[0], 2);
        assert!(arena[4] > 0);
        assert!(
            arena[5] >= 2 * arena[4],
            "every actual model body shares complete Prepared/Committed SQL originals"
        );
        eprintln!("D2 actual count={count} inner={inner} compressed={compressed} stored={stored} history_validation={history} snapshot_parts={parts:?} arena[version,entries,unique_bytes,probe_usage,model_bodies,shared_original_sql_refs]={arena:?}");
        assert_eq!(cap.reason(), "AllSelectedPhysicalAccepted");
        assert_eq!(cap.selected_count(), count);
        assert_eq!(cap.business_date(), date());
        assert_eq!(cap.revision(), before_revision);
        let saved = snapshot(&fixture);
        let restart = fixture.second_coordinator("D2_READ_RESTART");
        let session = restart.g5b_day_session(date()).unwrap();
        let read = session.read_physical_seal().unwrap().unwrap();
        assert_eq!(read.identity(), cap.identity());
        assert_eq!(read.sha256(), cap.sha256());
        assert_eq!(read.cohort_identity(), cap.cohort_identity());
        let refreshed = session.refresh_physical_seal(&read).unwrap();
        assert_eq!(refreshed.identity(), cap.identity());
        drop(session);
        assert_eq!(sealed(&fixture).identity(), cap.identity());
        assert_eq!(snapshot(&fixture), saved);
        assert_eq!(calls.load(Ordering::SeqCst), count);
        assert_eq!(sink.calls.load(Ordering::SeqCst), count);
    }
}

#[tokio::test]
async fn g5b_physical_seal_partial_model_archive_owner_and_finalizer_never_complete() {
    let fixture = Fixture::new("D2_PARTIAL_OBLIGATIONS");
    let log = input(&fixture, &[raw("600001"), raw("600002")]);
    let (provider, calls) = model(&log, Mode::Good);
    freeze_member(&fixture, &provider, 0).await;
    archive_model_observations_v2(owner(&fixture), date())
        .unwrap()
        .unwrap();
    capture_snapshots(&fixture);
    incomplete(&fixture);
    prepare(&fixture, 1, 1);
    let append = MemoryAppendPort::default();
    deliver(
        &fixture,
        &append,
        AuthoritativeSinkResult::Accepted(receipt(clock("15:13:00"))),
    );
    incomplete(&fixture);
    freeze_member(&fixture, &provider, 1).await;
    capture_snapshots(&fixture);
    incomplete(&fixture);
    archive_model_observations_v2(owner(&fixture), date())
        .unwrap()
        .unwrap();
    capture_snapshots(&fixture);
    incomplete(&fixture);
    prepare_model_owner_v2_for_test(owner(&fixture), date(), 1, 1, clock("15:14:00")).unwrap();
    let envelope = envelopes(&fixture).remove(1);
    let pending = fixture
        .coordinator
        .begin_attempt(&envelope.decision_identity, 1, clock("15:14:00"))
        .unwrap()
        .unwrap();
    fixture
        .coordinator
        .record_sink_result(
            &pending.attempt_identity,
            pending.fence_token,
            AuthoritativeSinkResult::Accepted(receipt(clock("15:14:00"))),
            clock("15:14:00"),
        )
        .unwrap();
    incomplete(&fixture);
    fixture
        .coordinator
        .reconcile_all_pending(&append, clock("15:14:01"))
        .unwrap();
    assert_eq!(sealed(&fixture).selected_count(), 2);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn g5b_physical_seal_rejected_uncertain_manual_accepted_and_denied_are_not_physical() {
    for mode in 0..4 {
        let fixture = Fixture::new("D2_NON_PHYSICAL");
        frozen(&fixture, 1).await;
        prepare(&fixture, 1, if mode == 3 { 0 } else { 1 });
        let append = MemoryAppendPort::default();
        if mode == 3 {
            fixture
                .coordinator
                .reconcile_all_pending(&append, clock("15:13:00"))
                .unwrap();
        } else {
            let result = if mode == 0 {
                AuthoritativeSinkResult::Rejected(rejection(clock("15:13:00"), false))
            } else {
                AuthoritativeSinkResult::Uncertain(uncertainty(clock("15:13:00")))
            };
            deliver(&fixture, &append, result);
            if mode == 2 {
                let envelope = &envelopes(&fixture)[0];
                let command = ManualResolutionCommand {
                    decision_identity: envelope.decision_identity.clone(),
                    disposition: ManualDisposition::Accepted {
                        receipt: Some(receipt(clock("15:14:00"))),
                    },
                    operator_identity: "TEST_CODE_D2_OPERATOR".to_owned(),
                    reason: "actual manual review".to_owned(),
                    external_evidence: b"actual manual review evidence".to_vec(),
                    resolved_at: clock("15:14:00"),
                };
                fixture
                    .coordinator
                    .resolve_uncertain(&command, &append)
                    .unwrap();
                fixture
                    .coordinator
                    .reconcile_all_pending(&append, clock("15:14:01"))
                    .unwrap();
                assert_eq!(
                    fixture
                        .coordinator
                        .decision_state(&envelope.decision_identity)
                        .unwrap(),
                    DecisionState::Delivered
                );
            }
        }
        incomplete(&fixture);
        assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM g5b_day_seals"), 0);
    }
}

#[tokio::test]
async fn g5b_physical_seal_late_raw_preserved_invalidates_pointer_and_ack_recloses_fresh_revision()
{
    let fixture = Fixture::new("D2_LATE_REOPEN");
    let (_, calls) = frozen(&fixture, 1).await;
    prepare(&fixture, 1, 1);
    let append = MemoryAppendPort::default();
    let sink = deliver(
        &fixture,
        &append,
        AuthoritativeSinkResult::Accepted(receipt(clock("15:13:00"))),
    );
    let cap = sealed(&fixture);
    let original = fixture.query_blob("SELECT seal_canonical FROM g5b_day_seals");
    let original_raw = fixture
        .query_blob("SELECT result_canonical FROM sink_results WHERE authoritative_for_state=1");
    let (attempt, fence) = attempt(&fixture);
    let mut late = receipt(clock("15:15:00"));
    late.message_id = "TEST_CODE_DISTINCT_LATE".to_owned();
    for (index, result) in [
        AuthoritativeSinkResult::Accepted(late),
        AuthoritativeSinkResult::Uncertain(uncertainty(clock("15:16:00"))),
    ]
    .into_iter()
    .enumerate()
    {
        fixture
            .coordinator
            .record_sink_result(
                &attempt,
                fence,
                result,
                clock(if index == 0 { "15:15:00" } else { "15:16:00" }),
            )
            .unwrap();
        assert_eq!(
            fixture.query_i64("SELECT revision FROM g5b_day_heads"),
            cap.revision() + index as i64 + 1
        );
        assert!(pointer(&fixture).is_none());
        assert!(owner(&fixture)
            .g5b_day_session(date())
            .unwrap()
            .refresh_physical_seal(&cap)
            .is_err());
        incomplete(&fixture);
        let before = snapshot(&fixture);
        assert!(matches!(
            owner(&fixture)
                .g5b_day_session(date())
                .unwrap()
                .try_seal_physical_cohort_known(&cap)
                .unwrap(),
            G5bPhysicalSealAttempt::Incomplete
        ));
        assert_eq!(snapshot(&fixture), before);
    }
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM sink_results WHERE late_after_fence=1 AND authoritative_for_state=0"),2);
    assert_eq!(
        fixture.query_blob(
            "SELECT result_canonical FROM sink_results WHERE authoritative_for_state=1"
        ),
        original_raw
    );
    assert_eq!(
        fixture.query_blob("SELECT seal_canonical FROM g5b_day_seals"),
        original
    );
    fixture
        .coordinator
        .reconcile_all_pending(&append, clock("15:16:01"))
        .unwrap();
    let new = match owner(&fixture)
        .g5b_day_session(date())
        .unwrap()
        .try_seal_physical_cohort_known(&cap)
        .unwrap()
    {
        G5bPhysicalSealAttempt::Sealed(v) => v,
        G5bPhysicalSealAttempt::Incomplete => {
            panic!("real late acknowledgments must reclose with the original anchor")
        }
    };
    assert!(new.revision() > cap.revision());
    assert_ne!(new.identity(), cap.identity());
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM g5b_day_seals"), 2);
    let before = snapshot(&fixture);
    assert!(fixture
        .coordinator
        .record_sink_result(
            &attempt,
            fence,
            AuthoritativeSinkResult::Accepted(receipt(clock("15:13:00"))),
            clock("15:17:00")
        )
        .is_err());
    assert_eq!(snapshot(&fixture), before);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(sink.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn g5b_physical_seal_historical_pointer_null_still_blocks_new_business_but_exact_replay_and_conflict_audit_survive(
) {
    let fixture = Fixture::new("D2_EVER_SEALED");
    frozen(&fixture, 1).await;
    prepare(&fixture, 1, 1);
    let append = MemoryAppendPort::default();
    deliver(
        &fixture,
        &append,
        AuthoritativeSinkResult::Accepted(receipt(clock("15:13:00"))),
    );
    let cap = sealed(&fixture);
    let envelope = envelopes(&fixture).remove(0);
    let (attempt, fence) = attempt(&fixture);
    fixture
        .coordinator
        .record_sink_result(
            &attempt,
            fence,
            AuthoritativeSinkResult::Uncertain(uncertainty(clock("15:15:00"))),
            clock("15:15:00"),
        )
        .unwrap();
    assert!(pointer(&fixture).is_none());
    let before = snapshot(&fixture);
    fixture
        .coordinator
        .prepare(&envelope, 1, clock("15:16:00"))
        .unwrap();
    assert!(fixture
        .coordinator
        .begin_attempt(&envelope.decision_identity, 1, clock("15:16:00"))
        .unwrap()
        .is_none());
    assert!(!fixture
        .coordinator
        .heartbeat_attempt(
            &envelope.decision_identity,
            &attempt,
            fence,
            clock("15:16:00")
        )
        .unwrap());
    assert_eq!(snapshot(&fixture), before);
    let session = owner(&fixture);
    let session = session.g5b_day_session(date()).unwrap();
    assert!(session
        .prepare_opaque_snapshot(
            &session.read_cohort().unwrap().unwrap(),
            G5bSnapshotKind::Archive,
            None,
            b"different new archive"
        )
        .is_err());
    drop(session);
    let mut conflict = envelope.clone();
    conflict.rendered_content = b"conflicting incoming".to_vec();
    conflict.rendered_content_sha256 = sha256_hex(&conflict.rendered_content);
    assert!(matches!(
        fixture.coordinator.prepare(&conflict, 1, clock("15:16:00")),
        Err(DurableDeliveryError::DecisionIdentityConflict { .. })
    ));
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM immutable_audit_outbox WHERE audit_kind='DecisionIdentityConflict'"),1);
    incomplete(&fixture);
    fixture
        .coordinator
        .reconcile_all_pending(&append, clock("15:16:01"))
        .unwrap();
    assert_ne!(sealed(&fixture).identity(), cap.identity());
}

#[tokio::test]
async fn g5b_physical_seal_actual_suffix_refresh_then_rollback_or_committed_new_inode_refuses_without_healing(
) {
    for role in ["Source", "Selection", "Attempt", "Frozen", "Archive"] {
        let fixture = Fixture::new("D2_SOURCE_FILE_IDENTITY");
        let (log, calls) = frozen(&fixture, 1).await;
        prepare(&fixture, 1, 1);
        let append = MemoryAppendPort::default();
        deliver(
            &fixture,
            &append,
            AuthoritativeSinkResult::Accepted(receipt(clock("15:13:00"))),
        );
        let original = sealed(&fixture);
        log.append_test_date_raw_production_fixture(date(), &raw("600099"))
            .unwrap();
        capture_snapshots(&fixture);
        fixture.cleanup.record_if_present(
            namespace(&fixture).join("20260928.input-head.v1.json"),
            OwnedPathKind::FileOrSymlink,
        );
        let known = owner(&fixture)
            .g5b_day_session(date())
            .unwrap()
            .refresh_physical_seal(&original)
            .unwrap();
        assert_eq!(known.identity(), original.identity());
        let path = if role == "Source" {
            namespace(&fixture).join("20260928.jsonl")
        } else {
            artifact(&fixture, role)
        };
        let (new, aside) = physical_replacement(&fixture, &path);
        replace(&path, &new, &aside);
        capture_replaced(&fixture, &path, &aside);
        let before = snapshot(&fixture);
        assert!(owner(&fixture)
            .g5b_day_session(date())
            .unwrap()
            .refresh_physical_seal(&known)
            .is_err());
        assert!(owner(&fixture)
            .g5b_day_session(date())
            .unwrap()
            .read_physical_seal()
            .is_err());
        assert!(owner(&fixture)
            .g5b_day_session(date())
            .unwrap()
            .try_seal_physical_cohort_known(&known)
            .is_err());
        assert_eq!(snapshot(&fixture), before);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn g5b_physical_seal_unexplained_legacy_high_index_or_orphan_file_blocks_real_completion() {
    for legacy in [false, true] {
        let fixture = Fixture::new("D2_UNKNOWN_ARTIFACT");
        frozen(&fixture, 1).await;
        prepare(&fixture, 1, 1);
        let append = MemoryAppendPort::default();
        deliver(
            &fixture,
            &append,
            AuthoritativeSinkResult::Accepted(receipt(clock("15:13:00"))),
        );
        let path = if legacy {
            let directory = namespace(&fixture).join("attempts");
            std::fs::create_dir(&directory).unwrap();
            std::fs::set_permissions(
                &directory,
                std::os::unix::fs::PermissionsExt::from_mode(0o700),
            )
            .unwrap();
            fixture.cleanup.record(&directory, OwnedPathKind::Directory);
            directory.join(format!("{DATE}.99.attempt.json"))
        } else {
            namespace(&fixture).join("20260928.unexplained-result.v2")
        };
        std::fs::write(&path, b"unknown evidence").unwrap();
        fixture.cleanup.record(&path, OwnedPathKind::FileOrSymlink);
        let before = snapshot(&fixture);
        assert!(owner(&fixture)
            .g5b_day_session(date())
            .unwrap()
            .try_seal_physical_cohort()
            .is_err());
        assert_eq!(snapshot(&fixture), before);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(sealed(&fixture).selected_count(), 1);
    }
}

fn arm_after_sql(
    weak: std::sync::Weak<DurableDeliveryCoordinator>,
    phase: DatabaseOperationTestPhase,
    remaining: usize,
    action: Arc<Mutex<Option<Box<dyn FnOnce() + Send>>>>,
    hit: Arc<AtomicUsize>,
) {
    weak.upgrade()
        .unwrap()
        .install_database_operation_test_hook(phase, move || {
            if remaining == 1 {
                hit.fetch_add(1, Ordering::SeqCst);
                action.lock().unwrap().take().unwrap()();
            } else {
                arm_after_sql(weak, phase, remaining - 1, action, hit);
            }
            Ok(())
        })
        .unwrap();
}

#[tokio::test]
async fn g5b_physical_seal_after_all_sql_hooks_input_mutation_rolls_back_seal_and_pointer() {
    for phase in [
        DatabaseOperationTestPhase::AfterSqlBeforePreCommitValidation,
        DatabaseOperationTestPhase::AfterSqlBeforePostValidation,
    ] {
        let fixture = Fixture::new("D2_LAST_SQL_INPUT");
        frozen(&fixture, 1).await;
        prepare(&fixture, 1, 1);
        let append = MemoryAppendPort::default();
        deliver(
            &fixture,
            &append,
            AuthoritativeSinkResult::Accepted(receipt(clock("15:13:00"))),
        );
        let path = namespace(&fixture).join("20260928.jsonl");
        let (new, aside) = physical_replacement(&fixture, &path);
        let before = snapshot(&fixture);
        let hit = Arc::new(AtomicUsize::new(0));
        let original = path.clone();
        let replacement_path = new.clone();
        let aside_path = aside.clone();
        // Missing pointer read, qualifying snapshot, then the actual seal CAS.
        arm_after_sql(
            Arc::downgrade(&owner(&fixture)),
            phase,
            3,
            Arc::new(Mutex::new(Some(Box::new(move || {
                replace(&original, &replacement_path, &aside_path)
            })))),
            Arc::clone(&hit),
        );
        let error = owner(&fixture)
            .g5b_day_session(date())
            .unwrap()
            .try_seal_physical_cohort()
            .err()
            .expect("the actual input fault must reject sealing");
        assert_eq!(
            hit.load(Ordering::SeqCst),
            1,
            "phase={phase:?}; actual_error={error}"
        );
        capture_replaced(&fixture, &path, &aside);
        assert_eq!(snapshot(&fixture), before);
        assert!(pointer(&fixture).is_none());
    }
}

#[tokio::test]
async fn g5b_physical_seal_final_file_read_source_mutation_rejects_opaque_reader() {
    let fixture = Fixture::new("D2_FINAL_FILE_SOURCE");
    frozen(&fixture, 1).await;
    prepare(&fixture, 1, 1);
    let append = MemoryAppendPort::default();
    deliver(
        &fixture,
        &append,
        AuthoritativeSinkResult::Accepted(receipt(clock("15:13:00"))),
    );
    let cap = sealed(&fixture);
    let path = namespace(&fixture).join("20260928.jsonl");
    let (new, aside) = physical_replacement(&fixture, &path);
    let original = path.clone();
    let aside_path = aside.clone();
    let hit = Arc::new(AtomicUsize::new(0));
    let action_hit = Arc::clone(&hit);
    let coordinator = owner(&fixture);
    let session = coordinator.g5b_day_session(date()).unwrap();
    session
        .install_physical_file_fault_for_test(1, move || {
            action_hit.fetch_add(1, Ordering::SeqCst);
            replace(&original, &new, &aside_path);
            Ok(())
        })
        .unwrap();
    let error = session
        .refresh_physical_seal(&cap)
        .err()
        .expect("the actual final input fault must reject refresh");
    assert_eq!(hit.load(Ordering::SeqCst), 1, "actual_error={error}");
    drop(session);
    capture_replaced(&fixture, &path, &aside);
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM g5b_day_seals"), 1);
}

#[tokio::test]
async fn g5b_physical_seal_postcommit_failure_retains_exact_history_but_never_returns_capability() {
    let fixture = Fixture::new("D2_POST_COMMIT");
    frozen(&fixture, 1).await;
    prepare(&fixture, 1, 1);
    let append = MemoryAppendPort::default();
    deliver(
        &fixture,
        &append,
        AuthoritativeSinkResult::Accepted(receipt(clock("15:13:00"))),
    );
    let path = artifact(&fixture, "Archive");
    let (new, aside) = physical_replacement(&fixture, &path);
    let original = path.clone();
    let aside_path = aside.clone();
    let hit = Arc::new(AtomicUsize::new(0));
    arm_after_sql(
        Arc::downgrade(&owner(&fixture)),
        DatabaseOperationTestPhase::AfterCommitBeforePostValidation,
        3,
        Arc::new(Mutex::new(Some(Box::new(move || {
            replace(&original, &new, &aside_path)
        })))),
        Arc::clone(&hit),
    );
    let error = owner(&fixture)
        .g5b_day_session(date())
        .unwrap()
        .try_seal_physical_cohort()
        .err()
        .unwrap();
    assert!(
        error.to_string().contains("COMMIT succeeded"),
        "actual_error={error}"
    );
    assert_eq!(hit.load(Ordering::SeqCst), 1, "actual_error={error}");
    capture_replaced(&fixture, &path, &aside);
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM g5b_day_seals"), 1);
    assert!(pointer(&fixture).is_some());
    assert!(owner(&fixture)
        .g5b_day_session(date())
        .unwrap()
        .read_physical_seal()
        .is_err());
    // Restore the original retained inode, never synthesize a new same-byte one.
    std::fs::remove_file(&path).unwrap();
    std::fs::rename(&aside, &path).unwrap();
    fixture.cleanup.record(&path, OwnedPathKind::FileOrSymlink);
    let before = snapshot(&fixture);
    assert_eq!(sealed(&fixture).selected_count(), 1);
    assert_eq!(snapshot(&fixture), before);
}

fn arm_sql_fault(
    weak: std::sync::Weak<DurableDeliveryCoordinator>,
    phase: DatabaseOperationTestPhase,
    remaining: usize,
    hit: Arc<AtomicUsize>,
    fault: OperationPostvalidationTestFault,
) {
    weak.upgrade()
        .unwrap()
        .install_database_operation_test_hook(phase, move || {
            if remaining == 1 {
                hit.fetch_add(1, Ordering::SeqCst);
                weak.upgrade()
                    .unwrap()
                    .install_operation_postvalidation_test_fault(fault)?;
            } else {
                arm_sql_fault(weak, phase, remaining - 1, hit, fault);
            }
            Ok(())
        })
        .unwrap();
}

#[tokio::test]
async fn g5b_physical_seal_all_sql_hooks_legal_revision_or_dirty_drift_never_normalized() {
    for phase in [
        DatabaseOperationTestPhase::AfterSqlBeforePreCommitValidation,
        DatabaseOperationTestPhase::AfterSqlBeforePostValidation,
    ] {
        for fault in [
            OperationPostvalidationTestFault::G5bHeadRevisionAdvance,
            OperationPostvalidationTestFault::G5bHeadArtifactStateDrift,
        ] {
            let fixture = Fixture::new("D2_EXACT_HEAD_DRIFT");
            frozen(&fixture, 1).await;
            prepare(&fixture, 1, 1);
            let append = MemoryAppendPort::default();
            deliver(
                &fixture,
                &append,
                AuthoritativeSinkResult::Accepted(receipt(clock("15:13:00"))),
            );
            let before = snapshot(&fixture);
            let hit = Arc::new(AtomicUsize::new(0));
            arm_sql_fault(
                Arc::downgrade(&owner(&fixture)),
                phase,
                3,
                Arc::clone(&hit),
                fault,
            );
            assert!(owner(&fixture)
                .g5b_day_session(date())
                .unwrap()
                .try_seal_physical_cohort()
                .is_err());
            assert_eq!(hit.load(Ordering::SeqCst), 1);
            assert_eq!(snapshot(&fixture), before);
            assert!(pointer(&fixture).is_none());
        }
    }
}

#[tokio::test]
async fn g5b_physical_seal_second_reader_sql_hook_requires_exact_current_sql_and_final_files() {
    for file in [false, true] {
        let fixture = Fixture::new("D2_LAST_READER_BOUNDARY");
        frozen(&fixture, 1).await;
        prepare(&fixture, 1, 1);
        let append = MemoryAppendPort::default();
        deliver(
            &fixture,
            &append,
            AuthoritativeSinkResult::Accepted(receipt(clock("15:13:00"))),
        );
        let cap = sealed(&fixture);
        let before = snapshot(&fixture);
        let hit = Arc::new(AtomicUsize::new(0));
        let mut replaced = None;
        if file {
            let path = artifact(&fixture, "Frozen");
            let (new, aside) = physical_replacement(&fixture, &path);
            let original = path.clone();
            let aside_path = aside.clone();
            replaced = Some((path, aside));
            arm_after_sql(
                Arc::downgrade(&owner(&fixture)),
                DatabaseOperationTestPhase::AfterSqlBeforePostValidation,
                2,
                Arc::new(Mutex::new(Some(Box::new(move || {
                    replace(&original, &new, &aside_path)
                })))),
                Arc::clone(&hit),
            );
        } else {
            arm_sql_fault(
                Arc::downgrade(&owner(&fixture)),
                DatabaseOperationTestPhase::AfterSqlBeforePostValidation,
                2,
                Arc::clone(&hit),
                OperationPostvalidationTestFault::G5bHeadRevisionAdvance,
            );
        }
        let error = owner(&fixture)
            .g5b_day_session(date())
            .unwrap()
            .refresh_physical_seal(&cap)
            .err()
            .expect("the actual last reader fault must reject refresh");
        assert_eq!(
            hit.load(Ordering::SeqCst),
            1,
            "file={file}; actual_error={error}"
        );
        if let Some((path, aside)) = replaced {
            capture_replaced(&fixture, &path, &aside);
        }
        assert_eq!(snapshot(&fixture), before);
    }
}

#[tokio::test]
async fn g5b_physical_seal_real_rejected_retry_history_keeps_old_raw_and_closes_only_final_accepted(
) {
    let fixture = Fixture::new("D2_RETRY_HISTORY");
    frozen(&fixture, 1).await;
    prepare(&fixture, 1, 1);
    let append = MemoryAppendPort::default();
    deliver(
        &fixture,
        &append,
        AuthoritativeSinkResult::Rejected(rejection(clock("15:13:00"), true)),
    );
    incomplete(&fixture);
    let envelope = envelopes(&fixture).remove(0);
    let sink = StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(clock(
        "15:14:00",
    ))));
    let sinks: [AuthoritativeSink; 1] = [sink.clone()];
    fixture
        .coordinator
        .resume_deliverable(&envelope.decision_identity, &sinks, clock("15:14:00"))
        .unwrap();
    fixture
        .coordinator
        .reconcile_all_pending(&append, clock("15:14:01"))
        .unwrap();
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM delivery_attempts"),
        2
    );
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM sink_results WHERE result_kind='Rejected' AND authoritative_for_state=1"),1);
    assert_eq!(sealed(&fixture).selected_count(), 1);
    assert_eq!(sink.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn g5b_physical_seal_valid_audit_hashes_do_not_adopt_wrong_lease_time_or_disconnected_chain()
{
    for cycle in [false, true] {
        let fixture = Fixture::new("D2_AUDIT_ACTUAL_BINDING");
        frozen(&fixture, 1).await;
        prepare(&fixture, 1, 1);
        let append = MemoryAppendPort::default();
        deliver(
            &fixture,
            &append,
            AuthoritativeSinkResult::Accepted(receipt(clock("15:13:00"))),
        );
        let connection = Connection::open(&fixture.database_path).unwrap();
        connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
        let trigger: String = connection
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name='immutable_outbox_payload_update'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let (id,bytes,sha):(String,Vec<u8>,String)=connection.query_row("SELECT audit_identity,audit_canonical,audit_sha256 FROM immutable_audit_outbox WHERE audit_kind='LeaseGranted'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(sha256_hex(&bytes), sha);
        connection
            .execute_batch("DROP TRIGGER immutable_outbox_payload_update")
            .unwrap();
        if cycle {
            connection.execute("UPDATE immutable_audit_outbox SET predecessor_audit_identity=audit_identity WHERE audit_identity=?1",[&id]).unwrap();
        } else {
            connection
                .execute(
                    "UPDATE immutable_audit_outbox SET created_at=?1 WHERE audit_identity=?2",
                    params!["2026-09-28T07:12:59Z", id],
                )
                .unwrap();
        }
        connection.execute_batch(&trigger).unwrap();
        drop(connection);
        let before = snapshot(&fixture);
        let error = owner(&fixture)
            .g5b_day_session(date())
            .unwrap()
            .try_seal_physical_cohort()
            .err()
            .unwrap()
            .to_string();
        assert!(
            if cycle {
                error.contains("chain") || error.contains("fork")
            } else {
                error.contains("lease grant time")
            },
            "{error}"
        );
        assert_eq!(snapshot(&fixture), before);
        assert!(pointer(&fixture).is_none());
    }
}

#[tokio::test]
async fn g5b_physical_seal_known_suffix_head_generation_cannot_roll_back_to_original_cutoff() {
    use std::io::{Seek, SeekFrom};
    let fixture = Fixture::new("D2_KNOWN_SUFFIX_ROLLBACK");
    let (log, _) = frozen(&fixture, 1).await;
    prepare(&fixture, 1, 1);
    let append = MemoryAppendPort::default();
    deliver(
        &fixture,
        &append,
        AuthoritativeSinkResult::Accepted(receipt(clock("15:13:00"))),
    );
    let original = sealed(&fixture);
    let head = namespace(&fixture).join("20260928.input-head.v1.json");
    let source = namespace(&fixture).join("20260928.jsonl");
    let old_head = std::fs::read(&head).unwrap();
    let old_source = std::fs::read(&source).unwrap();
    log.append_test_date_raw_production_fixture(date(), &raw("600099"))
        .unwrap();
    fixture.cleanup.record(&head, OwnedPathKind::FileOrSymlink);
    let current_head = std::fs::read(&head).unwrap();
    let current_source = std::fs::read(&source).unwrap();
    let known = owner(&fixture)
        .g5b_day_session(date())
        .unwrap()
        .refresh_physical_seal(&original)
        .unwrap();
    // A legitimate late raw observation invalidates the current pointer. The
    // same known anchor must survive acknowledgment and actual resealing.
    let (attempt, fence) = attempt(&fixture);
    fixture
        .coordinator
        .record_sink_result(
            &attempt,
            fence,
            AuthoritativeSinkResult::Uncertain(uncertainty(clock("15:15:00"))),
            clock("15:15:00"),
        )
        .unwrap();
    fixture
        .coordinator
        .reconcile_all_pending(&append, clock("15:15:01"))
        .unwrap();
    assert!(pointer(&fixture).is_none());
    // Keep the real positive source inode while rolling both consistent files
    // back. Original cutoff alone would pass; the previously observed head must
    // reject losing a later legal generation.
    std::fs::write(&head, &old_head).unwrap();
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .open(&source)
        .unwrap();
    file.set_len(old_source.len() as u64).unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(&old_source).unwrap();
    file.sync_all().unwrap();
    let before = snapshot(&fixture);
    assert!(owner(&fixture)
        .g5b_day_session(date())
        .unwrap()
        .refresh_physical_seal(&known)
        .is_err());
    assert!(owner(&fixture)
        .g5b_day_session(date())
        .unwrap()
        .try_seal_physical_cohort_known(&known)
        .is_err());
    assert_eq!(snapshot(&fixture), before);
    std::fs::write(&head, &current_head).unwrap();
    file.set_len(current_source.len() as u64).unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(&current_source).unwrap();
    file.sync_all().unwrap();
    let new = match owner(&fixture)
        .g5b_day_session(date())
        .unwrap()
        .try_seal_physical_cohort_known(&known)
        .unwrap()
    {
        G5bPhysicalSealAttempt::Sealed(v) => v,
        G5bPhysicalSealAttempt::Incomplete => {
            panic!("exact current suffix plus drained late evidence must reclose")
        }
    };
    assert_ne!(new.identity(), original.identity());
    assert!(new.revision() > known.revision());
    assert_eq!(new.cohort_identity(), known.cohort_identity());
}

#[tokio::test]
async fn g5b_physical_seal_committed_missing_archive_is_never_recreated() {
    let fixture = Fixture::new("D2_COMMITTED_ARCHIVE_MISSING");
    frozen(&fixture, 1).await;
    prepare(&fixture, 1, 1);
    let append = MemoryAppendPort::default();
    deliver(
        &fixture,
        &append,
        AuthoritativeSinkResult::Accepted(receipt(clock("15:13:00"))),
    );
    let cap = sealed(&fixture);
    let path = artifact(&fixture, "Archive");
    std::fs::remove_file(&path).unwrap();
    let before = snapshot(&fixture);
    assert!(owner(&fixture)
        .g5b_day_session(date())
        .unwrap()
        .refresh_physical_seal(&cap)
        .is_err());
    assert!(owner(&fixture)
        .g5b_day_session(date())
        .unwrap()
        .try_seal_physical_cohort()
        .is_err());
    assert!(!path.exists());
    assert_eq!(snapshot(&fixture), before);
}

#[tokio::test]
async fn g5b_physical_seal_known_capability_cannot_cross_actual_namespace_or_database() {
    let one = Fixture::new("D2_KNOWN_SCOPE_ONE");
    let two = Fixture::new("D2_KNOWN_SCOPE_TWO");
    for fixture in [&one, &two] {
        frozen(fixture, 1).await;
        prepare(fixture, 1, 1);
        let append = MemoryAppendPort::default();
        deliver(
            fixture,
            &append,
            AuthoritativeSinkResult::Accepted(receipt(clock("15:13:00"))),
        );
    }
    let known = sealed(&one);
    let before = snapshot(&two);
    let coordinator = owner(&two);
    let session = coordinator.g5b_day_session(date()).unwrap();
    assert!(session.refresh_physical_seal(&known).is_err());
    assert!(session.try_seal_physical_cohort_known(&known).is_err());
    assert_eq!(snapshot(&two), before);
    assert!(pointer(&two).is_none());
    drop(session);
    assert_eq!(sealed(&one).identity(), known.identity());
}

#[test]
fn g5b_physical_seal_routing_list_includes_unpublished_real_cohort_without_granting_completion() {
    let fixture = Fixture::new("D2_ROUTING_PREPARED");
    assert!(owner(&fixture)
        .list_g5b_nonempty_cohort_dates()
        .unwrap()
        .is_empty());
    let log = input(&fixture, &[raw("600001")]);
    let (provider, _) = model(&log, Mode::PanicIfCalled);
    let coordinator = owner(&fixture);
    let session = coordinator.g5b_day_session(date()).unwrap();
    let ready = session
        .configured_analysis_for_test(provider, clock("15:10:00"))
        .unwrap();
    session.prepare_cohort(&ready).unwrap();
    drop(session);
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM g5b_day_heads WHERE cohort_identity IS NULL"),
        1
    );
    let before = snapshot(&fixture);
    assert_eq!(
        coordinator.list_g5b_nonempty_cohort_dates().unwrap(),
        vec![date()]
    );
    assert_eq!(snapshot(&fixture), before);
    assert!(pointer(&fixture).is_none());
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM delivery_decisions"),
        0
    );
}

#[tokio::test]
async fn g5b_physical_seal_routing_list_all_hooks_exact_membership_and_postcommit_evidence() {
    for phase in [
        DatabaseOperationTestPhase::AfterSqlBeforePreCommitValidation,
        DatabaseOperationTestPhase::AfterSqlBeforePostValidation,
        DatabaseOperationTestPhase::AfterCommitBeforePostValidation,
    ] {
        let fixture = Fixture::new("D2_ROUTING_LAST_SQL");
        frozen(&fixture, 1).await;
        let before = snapshot(&fixture);
        let revision = fixture.query_i64("SELECT revision FROM g5b_day_heads");
        let hit = Arc::new(AtomicUsize::new(0));
        let hook_hit = Arc::clone(&hit);
        let weak = Arc::downgrade(&owner(&fixture));
        let database_path = fixture.database_path.clone();
        owner(&fixture)
            .install_database_operation_test_hook(phase, move || {
                hook_hit.fetch_add(1, Ordering::SeqCst);
                if phase == DatabaseOperationTestPhase::AfterCommitBeforePostValidation {
                    // This is an actual independently committed legitimate head
                    // revision change after the read transaction's COMMIT.
                    // No reentry: the hook already owns the tested database lease;
                    // use its original attested path on a distinct raw connection.
                    let connection = Connection::open(&database_path).unwrap();
                    super::super::super::schema::register_sha256_function(&connection)?;
                    connection
                        .execute("UPDATE g5b_day_heads SET revision=revision+1", [])
                        .unwrap();
                } else {
                    weak.upgrade()
                        .unwrap()
                        .install_operation_postvalidation_test_fault(
                            OperationPostvalidationTestFault::G5bHeadRevisionAdvance,
                        )?;
                }
                Ok(())
            })
            .unwrap();
        let error = owner(&fixture)
            .list_g5b_nonempty_cohort_dates()
            .err()
            .unwrap()
            .to_string();
        assert_eq!(hit.load(Ordering::SeqCst), 1);
        if phase == DatabaseOperationTestPhase::AfterCommitBeforePostValidation {
            assert!(error.contains("COMMIT succeeded"), "{error}");
            assert_eq!(
                fixture.query_i64("SELECT revision FROM g5b_day_heads"),
                revision + 1
            );
        } else {
            assert_eq!(snapshot(&fixture), before);
        }
        assert_eq!(
            owner(&fixture).list_g5b_nonempty_cohort_dates().unwrap(),
            vec![date()]
        );
    }
}

#[tokio::test]
async fn g5b_physical_seal_known_historical_row_cannot_disappear_during_late_reclose() {
    let fixture = Fixture::new("D2_KNOWN_HISTORY_MISSING");
    frozen(&fixture, 1).await;
    prepare(&fixture, 1, 1);
    let append = MemoryAppendPort::default();
    deliver(
        &fixture,
        &append,
        AuthoritativeSinkResult::Accepted(receipt(clock("15:13:00"))),
    );
    let known = sealed(&fixture);
    let (attempt, fence) = attempt(&fixture);
    fixture
        .coordinator
        .record_sink_result(
            &attempt,
            fence,
            AuthoritativeSinkResult::Uncertain(uncertainty(clock("15:15:00"))),
            clock("15:15:00"),
        )
        .unwrap();
    assert!(pointer(&fixture).is_none());
    // A noncooperating SQL attacker removes the exact previously observed row
    // and restores the schema. Its missing history must not be reconstructed.
    let connection = Connection::open(&fixture.database_path).unwrap();
    super::super::super::schema::register_sha256_function(&connection).unwrap();
    connection
        .pragma_update(None, "foreign_keys", true)
        .unwrap();
    let trigger: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='g5b_day_seals_immutable_delete'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    connection
        .execute_batch("DROP TRIGGER g5b_day_seals_immutable_delete")
        .unwrap();
    connection
        .execute(
            "DELETE FROM g5b_day_seals WHERE seal_identity=?1",
            [known.identity()],
        )
        .unwrap();
    connection.execute_batch(&trigger).unwrap();
    drop(connection);
    let before = snapshot(&fixture);
    let coordinator = owner(&fixture);
    let session = coordinator.g5b_day_session(date()).unwrap();
    assert!(session.refresh_physical_seal(&known).is_err());
    assert!(session.try_seal_physical_cohort_known(&known).is_err());
    assert_eq!(snapshot(&fixture), before);
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM g5b_day_seals"), 0);
    assert!(pointer(&fixture).is_none());
}

#[tokio::test]
async fn g5b_physical_seal_current_lease_columns_bind_last_actual_predecessor_chain() {
    for heartbeat in [false, true] {
        for corrupt in [false, true] {
            let fixture = Fixture::new("D2_LAST_LEASE_BINDING");
            frozen(&fixture, 1).await;
            prepare(&fixture, 1, 1);
            let envelope = envelopes(&fixture).remove(0);
            let request = fixture
                .coordinator
                .begin_attempt(&envelope.decision_identity, 1, clock("15:13:00"))
                .unwrap()
                .unwrap();
            if heartbeat {
                for time in ["15:13:10", "15:13:20"] {
                    assert!(fixture
                        .coordinator
                        .heartbeat_attempt(
                            &envelope.decision_identity,
                            &request.attempt_identity,
                            request.fence_token,
                            clock(time)
                        )
                        .unwrap());
                }
            }
            let sink = StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(clock(
                "15:13:30",
            ))));
            let result = sink.deliver(&request.request);
            fixture
                .coordinator
                .record_sink_result(
                    &request.attempt_identity,
                    request.fence_token,
                    result,
                    clock("15:13:30"),
                )
                .unwrap();
            let append = MemoryAppendPort::default();
            fixture
                .coordinator
                .reconcile_all_pending(&append, clock("15:13:31"))
                .unwrap();
            assert_eq!(sink.calls.load(Ordering::SeqCst), 1);
            assert_eq!(
                fixture.query_i64(
                    "SELECT COUNT(*) FROM immutable_audit_outbox WHERE append_state!='Appended'"
                ),
                0
            );
            assert_eq!(
                fixture
                    .query_i64("SELECT COUNT(*) FROM delivery_decisions WHERE state='Delivered'"),
                1
            );
            if !corrupt {
                assert_eq!(sealed(&fixture).selected_count(), 1);
                continue;
            }
            let connection = Connection::open(&fixture.database_path).unwrap();
            super::super::super::schema::register_sha256_function(&connection).unwrap();
            // Both strings are valid RFC3339 and preserve a valid expiry after
            // heartbeat. All original event/audit bytes and refs remain intact.
            connection.execute("UPDATE delivery_attempts SET lease_heartbeat_at=?1,lease_expires_at=?2 WHERE attempt_identity=?3",
                params!["2026-09-28T07:13:40Z","2026-09-28T07:15:40Z",request.attempt_identity]).unwrap();
            drop(connection);
            let before = snapshot(&fixture);
            let error = owner(&fixture)
                .g5b_day_session(date())
                .unwrap()
                .try_seal_physical_cohort()
                .err()
                .unwrap()
                .to_string();
            assert!(error.contains("last original lease audit"), "{error}");
            assert_eq!(snapshot(&fixture), before);
            assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM g5b_day_seals"), 0);
            assert!(pointer(&fixture).is_none());
        }
    }
}

#[tokio::test]
async fn g5b_physical_seal_budget_single_stored_row_is_rejected_before_copy() {
    let fixture = Fixture::new("D2_SEAL_ONE_ROW_BUDGET");
    frozen(&fixture, 1).await;
    prepare(&fixture, 1, 1);
    let append = MemoryAppendPort::default();
    deliver(
        &fixture,
        &append,
        AuthoritativeSinkResult::Accepted(receipt(clock("15:13:00"))),
    );
    let cap = sealed(&fixture);
    let before = snapshot(&fixture);
    let coordinator = owner(&fixture);
    let [single, total] = coordinator.physical_history_copy_parts_for_test().unwrap();
    assert_eq!(single, total, "one actual fully accounted copied row");
    {
        let _guard = coordinator
            .install_physical_witness_budget_for_test(single - 1)
            .unwrap();
        let error = coordinator
            .physical_history_copy_count_for_test()
            .unwrap_err()
            .to_string();
        assert!(error.contains("budget exceeded before copy"), "{error}");
        assert!(coordinator
            .g5b_day_session(date())
            .unwrap()
            .refresh_physical_seal(&cap)
            .is_err());
    }
    assert_eq!(snapshot(&fixture), before);
    assert_eq!(
        coordinator
            .g5b_day_session(date())
            .unwrap()
            .refresh_physical_seal(&cap)
            .unwrap()
            .identity(),
        cap.identity()
    );
}

#[tokio::test]
async fn g5b_physical_seal_budget_cumulative_history_is_bounded_across_actual_rows() {
    let fixture = Fixture::new("D2_SEAL_HISTORY_SUM_BUDGET");
    frozen(&fixture, 1).await;
    prepare(&fixture, 1, 1);
    let append = MemoryAppendPort::default();
    deliver(
        &fixture,
        &append,
        AuthoritativeSinkResult::Accepted(receipt(clock("15:13:00"))),
    );
    let old = sealed(&fixture);
    let (attempt, fence) = attempt(&fixture);
    fixture
        .coordinator
        .record_sink_result(
            &attempt,
            fence,
            AuthoritativeSinkResult::Uncertain(uncertainty(clock("15:15:00"))),
            clock("15:15:00"),
        )
        .unwrap();
    fixture
        .coordinator
        .reconcile_all_pending(&append, clock("15:15:01"))
        .unwrap();
    let cap = match owner(&fixture)
        .g5b_day_session(date())
        .unwrap()
        .try_seal_physical_cohort_known(&old)
        .unwrap()
    {
        G5bPhysicalSealAttempt::Sealed(cap) => cap,
        G5bPhysicalSealAttempt::Incomplete => panic!("actual drained late evidence must reclose"),
    };
    // Actual immutable rows: tuple/list metadata and all original byte copies.
    let coordinator = owner(&fixture);
    let [largest, total] = coordinator.physical_history_copy_parts_for_test().unwrap();
    let limit = largest + 16;
    assert!(limit < total);
    let before = snapshot(&fixture);
    assert_eq!(
        coordinator.physical_history_copy_count_for_test().unwrap(),
        2
    );
    {
        let _guard = coordinator
            .install_physical_witness_budget_for_test(limit)
            .unwrap();
        assert_eq!(
            coordinator.physical_history_copy_parts_for_test().unwrap(),
            [largest, total],
            "every actual row plus metadata fits alone before the shared list fails"
        );
        let error = coordinator
            .physical_history_copy_count_for_test()
            .unwrap_err()
            .to_string();
        assert!(error.contains("budget exceeded before copy"), "{error}");
    }
    assert_eq!(snapshot(&fixture), before);
    assert_eq!(
        coordinator
            .g5b_day_session(date())
            .unwrap()
            .refresh_physical_seal(&cap)
            .unwrap()
            .identity(),
        cap.identity()
    );
}

#[tokio::test]
async fn g5b_physical_seal_sql_snapshot_shares_copy_and_encoding_budget() {
    let fixture = Fixture::new("D2_SQL_SNAPSHOT_TOTAL_BUDGET");
    frozen(&fixture, 1).await;
    prepare(&fixture, 1, 1);
    let append = MemoryAppendPort::default();
    deliver(
        &fixture,
        &append,
        AuthoritativeSinkResult::Accepted(receipt(clock("15:13:00"))),
    );
    let cap = sealed(&fixture);
    let coordinator = owner(&fixture);
    let parts = coordinator
        .physical_snapshot_parts_for_test(date())
        .unwrap();
    let limit = parts.iter().copied().max().unwrap() + 16;
    assert!(parts.iter().sum::<usize>() > limit);
    let original_len = coordinator.physical_snapshot_len_for_test(date()).unwrap();
    let before = snapshot(&fixture);
    {
        let _guard = coordinator
            .install_physical_witness_budget_for_test(limit)
            .unwrap();
        let separately = coordinator
            .physical_snapshot_parts_for_test(date())
            .unwrap();
        assert_eq!(separately,parts,"each individual component and encoding fits; only their shared total exceeds the limit");
        let error = coordinator
            .physical_snapshot_len_for_test(date())
            .unwrap_err()
            .to_string();
        assert!(error.contains("budget exceeded before"), "{error}");
    }
    assert_eq!(snapshot(&fixture), before);
    assert_eq!(
        coordinator.physical_snapshot_len_for_test(date()).unwrap(),
        original_len
    );
    assert_eq!(
        coordinator
            .g5b_day_session(date())
            .unwrap()
            .refresh_physical_seal(&cap)
            .unwrap()
            .identity(),
        cap.identity()
    );
}

#[test]
fn g5b_physical_seal_compact_byte_leaves_preserve_types_spelling_and_one_representation() {
    let fixture = Fixture::new("D2_COMPACT_BYTE_LEAVES");
    for bytes in [
        b"".as_slice(),
        b"{ \"b\":2,\"a\":1 }\n\t".as_slice(),
        b"nul\0slash\\quote\"\n".as_slice(),
        &[0xff, 0, 0x80, 0x7f],
        "中文原始字节".as_bytes(),
    ] {
        assert_eq!(
            fixture
                .coordinator
                .physical_byte_leaf_roundtrip_for_test(bytes)
                .unwrap(),
            bytes
        );
    }
    for encoded in [
        b"{\"bytes\":{\"Hex\":\"6162\"}}".as_slice(), // UTF8 must use Utf8
        b"{\"bytes\":{\"Hex\":\"FF\"}}".as_slice(),
        b"{\"bytes\":{\"Hex\":\"f\"}}".as_slice(),
        b"{\"bytes\":{\"Other\":\"ff\"}}".as_slice(),
        b"{\"bytes\":{\"Utf8\":\"ab\",\"Hex\":\"ff\"}}".as_slice(),
        b"{\"bytes\":{\"Utf8\":\"ab\"},\"bytes\":{\"Utf8\":\"cd\"}}".as_slice(),
        b"{\"bytes\":{\"Utf8\":\"\\u0061\"}}".as_slice(),
    ] {
        assert!(fixture
            .coordinator
            .physical_byte_leaf_decode_for_test(encoded)
            .is_err());
    }
}

#[tokio::test]
async fn g5b_physical_seal_compressed_codec_rejects_bombs_frames_alternative_profile_and_noncanonical(
) {
    let fixture = Fixture::new("D2_STRICT_COMPRESSED_CODEC");
    frozen(&fixture, 1).await;
    prepare(&fixture, 1, 1);
    let append = MemoryAppendPort::default();
    let sink = deliver(
        &fixture,
        &append,
        AuthoritativeSinkResult::Accepted(receipt(clock("15:13:00"))),
    );
    let cap = sealed(&fixture);
    let original = {
        let connection = Connection::open(&fixture.database_path).unwrap();
        connection
            .query_row("SELECT seal_canonical FROM g5b_day_seals", [], |r| {
                r.get::<_, Vec<u8>>(0)
            })
            .unwrap()
    };
    let coordinator = owner(&fixture);
    assert_eq!(
        coordinator
            .physical_codec_decode_for_test(&original)
            .unwrap(),
        1
    );
    let before = snapshot(&fixture);
    for fault in [
        "declared-bomb",
        "frame-bomb",
        "declared-small",
        "trailing",
        "concatenated",
        "large-window",
        "missing-size",
        "different-profile",
        "different-level",
        "inner-trailing-space",
        "unknown-codec",
        "unknown-field",
        "duplicate-codec",
        "wrapper-space",
    ] {
        let mutated = coordinator
            .physical_codec_variant_for_test(date(), fault)
            .unwrap();
        assert!(
            mutated != original,
            "fault must change actual stored bytes: {fault}; original_len={}; mutated_len={}",
            original.len(),
            mutated.len()
        );
        assert!(
            coordinator
                .physical_codec_decode_for_test(&mutated)
                .is_err(),
            "must reject {fault}"
        );
        assert_eq!(snapshot(&fixture), before);
    }
    assert_eq!(
        coordinator
            .g5b_day_session(date())
            .unwrap()
            .refresh_physical_seal(&cap)
            .unwrap()
            .identity(),
        cap.identity()
    );
    assert_eq!(sink.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn g5b_physical_seal_all_history_inflate_and_validation_share_one_budget() {
    let fixture = Fixture::new("D2_ALL_HISTORY_INFLATE");
    frozen(&fixture, 1).await;
    prepare(&fixture, 1, 1);
    let append = MemoryAppendPort::default();
    let sink = deliver(
        &fixture,
        &append,
        AuthoritativeSinkResult::Accepted(receipt(clock("15:13:00"))),
    );
    let old = sealed(&fixture);
    let (attempt, fence) = attempt(&fixture);
    fixture
        .coordinator
        .record_sink_result(
            &attempt,
            fence,
            AuthoritativeSinkResult::Uncertain(uncertainty(clock("15:15:00"))),
            clock("15:15:00"),
        )
        .unwrap();
    fixture
        .coordinator
        .reconcile_all_pending(&append, clock("15:15:01"))
        .unwrap();
    let current = match owner(&fixture)
        .g5b_day_session(date())
        .unwrap()
        .try_seal_physical_cohort_known(&old)
        .unwrap()
    {
        G5bPhysicalSealAttempt::Sealed(cap) => cap,
        G5bPhysicalSealAttempt::Incomplete => {
            panic!("actual drained late raw receipt must reclose")
        }
    };
    let coordinator = owner(&fixture);
    let individual = coordinator
        .physical_single_history_validation_usage_for_test(date())
        .unwrap();
    let total = coordinator
        .physical_history_validation_usage_for_test()
        .unwrap();
    let limit = individual + 16;
    assert!(limit < total);
    assert!(total <= 32 * 1024 * 1024);
    let before = snapshot(&fixture);
    {
        let _guard = coordinator
            .install_physical_witness_budget_for_test(limit)
            .unwrap();
        assert!(
            coordinator
                .physical_single_history_validation_usage_for_test(date())
                .unwrap()
                <= limit
        );
        assert_eq!(
            coordinator.physical_history_copy_count_for_test().unwrap(),
            2,
            "tiny stored wrappers alone fit"
        );
        assert!(
            coordinator
                .physical_history_validation_usage_for_test()
                .is_err(),
            "two actual inflated/validated histories must share the limit"
        );
    }
    assert_eq!(snapshot(&fixture), before);
    assert_eq!(
        coordinator
            .g5b_day_session(date())
            .unwrap()
            .refresh_physical_seal(&current)
            .unwrap()
            .identity(),
        current.identity()
    );
    assert_eq!(sink.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn g5b_physical_seal_byte_arena_actual_v1_noop_then_v2_late_reclose_retains_all_originals() {
    let fixture = Fixture::new("D2_ARENA_V1_V2_HISTORY");
    let (_, calls) = frozen(&fixture, 1).await;
    prepare(&fixture, 1, 1);
    let append = MemoryAppendPort::default();
    let sink = deliver(
        &fixture,
        &append,
        AuthoritativeSinkResult::Accepted(receipt(clock("15:13:00"))),
    );
    let coordinator = owner(&fixture);
    let old = match coordinator
        .g5b_day_session(date())
        .unwrap()
        .try_seal_physical_legacy_for_test()
        .unwrap()
    {
        G5bPhysicalSealAttempt::Sealed(v) => v,
        G5bPhysicalSealAttempt::Incomplete => panic!("actual original protocol must complete v1"),
    };
    let old_bytes = fixture.query_blob("SELECT seal_canonical FROM g5b_day_seals");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&old_bytes).unwrap()["version"],
        1
    );
    let stable = snapshot(&fixture);
    for variant in ["text-blob-alias", "missing-original-row"] {
        assert!(
            coordinator
                .physical_arena_invalid_sql_for_test(date(), variant)
                .is_err(),
            "{variant}"
        );
        assert_eq!(
            snapshot(&fixture),
            stable,
            "negative borrowed receipt must not mutate SQL"
        );
    }
    let before = snapshot(&fixture);
    let raw = fixture
        .query_blob("SELECT result_canonical FROM sink_results WHERE authoritative_for_state=1");
    assert_eq!(sealed(&fixture).identity(), old.identity());
    assert_eq!(snapshot(&fixture), before);
    let restart = fixture.second_coordinator("D2_ARENA_V1_READ_RESTART");
    assert_eq!(
        restart
            .g5b_day_session(date())
            .unwrap()
            .refresh_physical_seal(&old)
            .unwrap()
            .identity(),
        old.identity()
    );
    let (attempt, fence) = attempt(&fixture);
    fixture
        .coordinator
        .record_sink_result(
            &attempt,
            fence,
            AuthoritativeSinkResult::Uncertain(uncertainty(clock("15:15:00"))),
            clock("15:15:00"),
        )
        .unwrap();
    assert!(pointer(&fixture).is_none());
    incomplete(&fixture);
    fixture
        .coordinator
        .reconcile_all_pending(&append, clock("15:15:01"))
        .unwrap();
    let current = match coordinator
        .g5b_day_session(date())
        .unwrap()
        .try_seal_physical_cohort_known(&old)
        .unwrap()
    {
        G5bPhysicalSealAttempt::Sealed(v) => v,
        G5bPhysicalSealAttempt::Incomplete => panic!("actual late raw/audit must reclose v2"),
    };
    assert_ne!(current.identity(), old.identity());
    assert!(current.revision() > old.revision());
    let rows = Connection::open(&fixture.database_path)
        .unwrap()
        .prepare("SELECT seal_canonical FROM g5b_day_seals ORDER BY revision")
        .unwrap()
        .query_map([], |row| row.get::<_, Vec<u8>>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0], old_bytes);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&rows[1]).unwrap()["version"],
        2
    );
    assert_eq!(
        fixture.query_blob(
            "SELECT result_canonical FROM sink_results WHERE authoritative_for_state=1"
        ),
        raw
    );
    assert_eq!(
        snapshot(&fixture).get("g5b_artifact_events"),
        before.get("g5b_artifact_events")
    );
    let after = snapshot(&fixture);
    let restart = fixture.second_coordinator("D2_ARENA_MIXED_READ_RESTART");
    assert_eq!(
        restart
            .g5b_day_session(date())
            .unwrap()
            .refresh_physical_seal(&current)
            .unwrap()
            .identity(),
        current.identity()
    );
    assert_eq!(sealed(&fixture).identity(), current.identity());
    assert_eq!(snapshot(&fixture), after);
    assert!(
        coordinator
            .physical_history_validation_usage_for_test()
            .unwrap()
            <= 32 * 1024 * 1024
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(sink.calls.load(Ordering::SeqCst), 1);
}
