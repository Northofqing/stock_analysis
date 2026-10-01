#![cfg(unix)]

use super::*;
use chrono::NaiveDate;
use fs2::FileExt;
use std::time::Duration as WaitDuration;

const G5B_DATE: &str = "2026-08-18";
const WORKER_TIMEOUT: WaitDuration = WaitDuration::from_secs(5);
const HELD_OBSERVATION: WaitDuration = WaitDuration::from_millis(150);

fn date() -> NaiveDate {
    NaiveDate::parse_from_str(G5B_DATE, "%Y-%m-%d").unwrap()
}

fn day_lock_path(fixture: &Fixture) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(&fixture.database_path)
        .parent()
        .unwrap()
        .join("20260818.g5b-day.lock")
}

// Use a separate read connection so a retained coordinator mutex cannot make
// the assertion itself wait for the blocked mutation.
fn database_snapshot(fixture: &Fixture) -> BTreeMap<String, Vec<Vec<String>>> {
    let connection = Connection::open(&fixture.database_path).unwrap();
    let tables = connection
        .prepare(
            "SELECT name FROM sqlite_master
             WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    let names = tables.iter().map(String::as_str).collect::<Vec<_>>();
    authority_snapshot(&connection, &names)
}

fn install_routing_signal(coordinator: &DurableDeliveryCoordinator) -> mpsc::Receiver<()> {
    let (ready, receiver) = mpsc::channel();
    coordinator
        .install_database_operation_test_hook(
            DatabaseOperationTestPhase::AfterMutationRoutingBeforeDateFence,
            move || {
                ready.send(()).unwrap();
                Ok(())
            },
        )
        .unwrap();
    receiver
}

fn assert_mutation_waits(
    fixture: &Fixture,
    operation: impl FnOnce(Arc<DurableDeliveryCoordinator>) -> Result<()> + Send + 'static,
) {
    let input_log = fixture.g5b_input_log(G5B_DATE);
    let before = database_snapshot(fixture);
    let fence = input_log.acquire_date_writer_fence(date()).unwrap();
    let ready = install_routing_signal(&fixture.coordinator);
    let coordinator = fixture_coordinator_arc(fixture);
    let (done, completed) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let result = operation(coordinator);
        let _ = done.send(());
        result
    });

    let reached_routing = ready.recv_timeout(WORKER_TIMEOUT);
    let early_completion = completed.recv_timeout(HELD_OBSERVATION);
    let while_held = database_snapshot(fixture);
    drop(fence);
    let completion = if early_completion.is_ok() {
        Ok(())
    } else {
        completed.recv_timeout(WORKER_TIMEOUT)
    };
    let result = worker.join().expect("TEST_CODE mutation worker panicked");

    reached_routing.expect("mutation must finish routing before waiting for its date");
    assert!(
        matches!(early_completion, Err(mpsc::RecvTimeoutError::Timeout)),
        "mutation completed while another writer held the same date fence"
    );
    assert_eq!(
        while_held, before,
        "held date permits no authority table changes"
    );
    completion.expect("mutation must complete after releasing its date fence");
    result.expect("mutation must remain legal after waiting");
    assert_ne!(
        database_snapshot(fixture),
        before,
        "mutation was not a no-op"
    );
}

#[derive(Clone, Copy, Debug)]
enum HeldMutation {
    Prepare,
    Attempt,
    SinkResult,
    Heartbeat,
    RetryAuthorization,
    RetryReservation,
    ManualResolution,
    AuditAcknowledgement,
    HydrationAcknowledgement,
}

#[test]
#[serial_test::serial(durable_physical_isolation)]
fn g5b_sqlite_date_fence_blocks_each_mutator_without_persistent_changes() {
    use HeldMutation::*;
    for case in [
        Prepare,
        Attempt,
        SinkResult,
        Heartbeat,
        RetryAuthorization,
        RetryReservation,
        ManualResolution,
        AuditAcknowledgement,
        HydrationAcknowledgement,
    ] {
        let label = format!("G5B_HELD_{case:?}");
        let fixture = Fixture::new(&label);
        let append = Arc::new(MemoryAppendPort::default());
        let mut candidate = g5b_frozen_envelope(&label, false);
        if matches!(case, HydrationAcknowledgement) {
            candidate.task_binding = Some(
                TaskBinding::new(
                    format!("TEST_CODE_TASK_{label}"),
                    format!("TEST_CODE_BASIS_{label}").into_bytes(),
                )
                .unwrap(),
            );
            candidate.validate().unwrap();
        }
        let mut attempt = None;
        let mut hydration = None;
        if !matches!(case, Prepare) {
            prepare_reserved(&fixture, &candidate, append.as_ref());
        }
        if !matches!(case, Prepare | Attempt) {
            attempt = fixture
                .coordinator
                .begin_attempt(&candidate.decision_identity, 1, now())
                .unwrap();
        }
        if matches!(
            case,
            RetryAuthorization | RetryReservation | ManualResolution
        ) {
            let lease = attempt.as_ref().unwrap();
            let result = if matches!(case, ManualResolution) {
                AuthoritativeSinkResult::Uncertain(uncertainty(now()))
            } else {
                AuthoritativeSinkResult::Rejected(rejection(now(), false))
            };
            fixture
                .coordinator
                .record_sink_result(&lease.attempt_identity, lease.fence_token, result, now())
                .unwrap();
            reconcile_terminal(
                &fixture,
                append.as_ref(),
                if matches!(case, ManualResolution) {
                    DecisionState::UncertainManualReview
                } else {
                    DecisionState::RejectedDurable
                },
                &candidate.decision_identity,
            );
            if matches!(case, RetryReservation) {
                fixture
                    .coordinator
                    .authorize_rejected_retry(&candidate.decision_identity)
                    .unwrap();
            }
        }
        if matches!(case, AuditAcknowledgement | HydrationAcknowledgement) {
            let lease = attempt.as_ref().unwrap();
            fixture
                .coordinator
                .record_sink_result(
                    &lease.attempt_identity,
                    lease.fence_token,
                    AuthoritativeSinkResult::Accepted(receipt(now())),
                    now(),
                )
                .unwrap();
            if matches!(case, HydrationAcknowledgement) {
                let summary = fixture
                    .coordinator
                    .reconcile_all_pending(append.as_ref(), now())
                    .unwrap();
                assert_eq!(summary.schedule_hydrations.len(), 1);
                hydration = summary.schedule_hydrations.into_iter().next();
                assert_eq!(
                    hydration.as_ref().unwrap().hydration_state,
                    ScheduleHydrationState::Pending
                );
            }
        }

        let identity = candidate.decision_identity.clone();
        let expected = match case {
            Prepare => DecisionState::Reserved,
            RetryAuthorization => DecisionState::RejectedDurable,
            Attempt | Heartbeat => DecisionState::AttemptInFlight,
            SinkResult | RetryReservation => DecisionState::AcceptedAuditPending,
            ManualResolution => DecisionState::ManualRejectedAuditPending,
            AuditAcknowledgement | HydrationAcknowledgement => DecisionState::Delivered,
        };
        assert_mutation_waits(&fixture, move |coordinator| {
            match case {
                Prepare => {
                    coordinator.prepare(&candidate, 1, now())?;
                }
                Attempt => {
                    assert!(coordinator.begin_attempt(&identity, 1, now())?.is_some());
                }
                SinkResult => {
                    let lease = attempt.unwrap();
                    coordinator.record_sink_result(
                        &lease.attempt_identity,
                        lease.fence_token,
                        AuthoritativeSinkResult::Accepted(receipt(now())),
                        now(),
                    )?;
                }
                Heartbeat => {
                    let lease = attempt.unwrap();
                    assert!(coordinator.heartbeat_attempt(
                        &identity,
                        &lease.attempt_identity,
                        lease.fence_token,
                        now() + chrono::Duration::seconds(60),
                    )?);
                }
                RetryAuthorization => coordinator.authorize_rejected_retry(&identity)?,
                RetryReservation => {
                    let sink = StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(now())));
                    let sinks: Vec<AuthoritativeSink> = vec![sink];
                    let resumed = coordinator.resume_deliverable(&identity, &sinks, now())?;
                    assert_eq!(resumed.sink_calls, 1);
                }
                ManualResolution => {
                    coordinator.resolve_uncertain(
                        &ManualResolutionCommand {
                            decision_identity: identity.clone(),
                            disposition: ManualDisposition::Rejected,
                            operator_identity: "TEST_CODE_OPERATOR_DATE_FENCE".to_owned(),
                            reason: "TEST_CODE_VERIFIED_REJECTION".to_owned(),
                            external_evidence: b"TEST_CODE_EXTERNAL_REJECTION".to_vec(),
                            resolved_at: now(),
                        },
                        append.as_ref(),
                    )?;
                }
                AuditAcknowledgement => {
                    coordinator.reconcile_all_pending(append.as_ref(), now())?;
                }
                HydrationAcknowledgement => {
                    let hydration = hydration.unwrap();
                    assert!(coordinator.acknowledge_schedule_hydration(
                        &hydration.transition_identity,
                        &hydration.transition_sha256,
                        now(),
                    )?);
                }
            }
            assert_eq!(coordinator.decision_state(&identity)?, expected);
            Ok(())
        });
    }
}

#[test]
#[serial_test::serial(durable_physical_isolation)]
fn g5b_sqlite_date_fence_wait_releases_same_coordinator_database_mutex() {
    let fixture = Fixture::new("G5B_WAIT_DATABASE_MUTEX");
    let candidate = g5b_frozen_envelope("WAIT_DATABASE_MUTEX", false);
    let input_log = fixture.g5b_input_log(G5B_DATE);
    let fence = input_log.acquire_date_writer_fence(date()).unwrap();
    let ready = install_routing_signal(&fixture.coordinator);
    let coordinator = fixture_coordinator_arc(&fixture);
    let (g5b_done, g5b_completed) = mpsc::channel();
    let g5b_worker = std::thread::spawn(move || {
        let result = coordinator.prepare(&candidate, 1, now());
        let _ = g5b_done.send(());
        result
    });
    let routed = ready.recv_timeout(WORKER_TIMEOUT);

    // Same date and same Arc: only G5b uses this fence. This also detects a
    // process attestation lease incorrectly retained while waiting for flock.
    let ordinary = envelope(
        "G5B_WAIT_ORDINARY",
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        G5B_DATE,
        false,
    );
    let coordinator = fixture_coordinator_arc(&fixture);
    let (ordinary_done, ordinary_completed) = mpsc::channel();
    let ordinary_worker = std::thread::spawn(move || {
        let result = coordinator.prepare(&ordinary, 1, now());
        let _ = ordinary_done.send(());
        result
    });
    let ordinary_before_release = ordinary_completed.recv_timeout(WORKER_TIMEOUT);
    let g5b_before_release = g5b_completed.try_recv();
    let persisted_g5b_while_held = fixture
        .query_i64("SELECT COUNT(*) FROM delivery_decisions WHERE push_kind='G5bAttribution'");
    drop(fence);
    let ordinary_result = ordinary_worker.join().unwrap();
    let g5b_result = g5b_worker.join().unwrap();

    routed.unwrap();
    ordinary_before_release.expect("ordinary writer was blocked by G5b's date wait");
    assert!(matches!(g5b_before_release, Err(mpsc::TryRecvError::Empty)));
    assert_eq!(persisted_g5b_while_held, 0);
    assert_eq!(ordinary_result.unwrap().state, DecisionState::Reserved);
    assert_eq!(g5b_result.unwrap().state, DecisionState::Reserved);
}

#[test]
#[serial_test::serial(durable_physical_isolation)]
fn g5b_sqlite_date_fence_conflicting_incoming_kind_uses_stored_owner_date() {
    let fixture = Fixture::new("G5B_STORED_CONFLICT_ROUTE");
    let stored = g5b_frozen_envelope("STORED_CONFLICT_ROUTE", false);
    let append = MemoryAppendPort::default();
    prepare_reserved(&fixture, &stored, &append);
    let original_bytes = fixture.query_blob("SELECT envelope_canonical FROM delivery_decisions");
    let mut original_tables = database_snapshot(&fixture);
    original_tables.remove("immutable_audit_outbox");
    let mut incoming = envelope(
        "G5B_CONFLICT_INCOMING_OTHER_DATE",
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        "2026-07-30",
        false,
    );
    incoming.decision_identity = stored.decision_identity.clone();
    let identity = stored.decision_identity.clone();
    assert_mutation_waits(&fixture, move |coordinator| {
        let result = coordinator.prepare(&incoming, 1, now());
        assert!(matches!(
            result,
            Err(DurableDeliveryError::DecisionIdentityConflict { decision_identity })
                if decision_identity == identity
        ));
        Ok(())
    });
    assert_eq!(
        fixture.query_blob("SELECT envelope_canonical FROM delivery_decisions"),
        original_bytes
    );
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM delivery_decisions"),
        1
    );
    assert_eq!(
        fixture.query_i64(
            "SELECT COUNT(*) FROM immutable_audit_outbox
             WHERE audit_kind='DecisionIdentityConflict' AND append_state='Pending'",
        ),
        1
    );
    let mut after_conflict = database_snapshot(&fixture);
    after_conflict.remove("immutable_audit_outbox");
    assert_eq!(
        after_conflict, original_tables,
        "conflict changed stored authority"
    );
}

#[test]
#[serial_test::serial(durable_physical_isolation)]
fn g5b_sqlite_date_fence_blocks_actual_delivered_finalizer_after_payloads_append() {
    let fixture = Fixture::new("G5B_HELD_FINALIZER");
    let append = Arc::new(MemoryAppendPort::default());
    let candidate = g5b_frozen_envelope("HELD_FINALIZER", false);
    prepare_reserved(&fixture, &candidate, append.as_ref());
    let sinks: Vec<AuthoritativeSink> = vec![StaticSink::new(AuthoritativeSinkResult::Accepted(
        receipt(now()),
    ))];
    fixture
        .coordinator
        .resume_deliverable(&candidate.decision_identity, &sinks, now())
        .unwrap();
    let input_log = fixture.g5b_input_log(G5B_DATE);
    let (ready, at_finalizer) = mpsc::channel();
    let (proceed, resume_finalizer) = mpsc::channel();
    fixture
        .coordinator
        .install_delivered_reconcile_test_hook(move || {
            ready.send(()).unwrap();
            resume_finalizer
                .recv_timeout(WORKER_TIMEOUT)
                .map_err(|error| {
                    DurableDeliveryError::InvalidConfiguration(format!(
                        "TEST_CODE finalizer handshake failed: {error}"
                    ))
                })?;
            Ok(())
        })
        .unwrap();
    let coordinator = fixture_coordinator_arc(&fixture);
    let worker_append = append.clone();
    let (done, completed) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let result = coordinator.reconcile_all_pending(worker_append.as_ref(), now());
        let _ = done.send(());
        result
    });
    let finalizer_reached = at_finalizer.recv_timeout(WORKER_TIMEOUT);
    let state_at_finalizer = fixture.query_strings("SELECT state FROM delivery_decisions");
    let disposition_appended = fixture.query_i64(
        "SELECT COUNT(*) FROM delivery_disposition_payloads WHERE append_state='Appended'",
    );
    let delivery_appended = fixture.query_i64(
        "SELECT COUNT(*) FROM sink_results
         WHERE result_kind='Accepted' AND delivery_audit_ref IS NOT NULL",
    );
    let pending_audits = fixture
        .query_i64("SELECT COUNT(*) FROM immutable_audit_outbox WHERE append_state='Pending'");
    let fence = input_log.acquire_date_writer_fence(date()).unwrap();
    let before = database_snapshot(&fixture);
    let routed = install_routing_signal(&fixture.coordinator);
    let continued = proceed.send(());
    let routing_reached = routed.recv_timeout(WORKER_TIMEOUT);
    let early = completed.recv_timeout(HELD_OBSERVATION);
    let while_held = database_snapshot(&fixture);
    drop(fence);
    let result = worker.join().unwrap();

    finalizer_reached.unwrap();
    continued.unwrap();
    routing_reached.unwrap();
    assert_eq!(state_at_finalizer, vec!["AcceptedAuditPending"]);
    assert_eq!(disposition_appended, 1);
    assert_eq!(delivery_appended, 1);
    assert_eq!(pending_audits, 0);
    assert!(matches!(early, Err(mpsc::RecvTimeoutError::Timeout)));
    assert_eq!(
        while_held, before,
        "actual finalizer wrote while date was held"
    );
    result.unwrap();
    assert_eq!(
        fixture
            .coordinator
            .decision_state(&candidate.decision_identity)
            .unwrap(),
        DecisionState::Delivered
    );
}

fn restore_lock_leaf(lock: &Path, retained: &Path) {
    std::fs::remove_file(lock).unwrap();
    std::fs::rename(retained, lock).unwrap();
}

#[test]
#[serial_test::serial(durable_physical_isolation)]
fn g5b_sqlite_date_fence_rejects_lock_inode_swap_after_all_sql_hooks_and_rolls_back() {
    for phase in [
        DatabaseOperationTestPhase::AfterSqlBeforePreCommitValidation,
        DatabaseOperationTestPhase::AfterSqlBeforePostValidation,
    ] {
        let label = format!("G5B_LOCK_PRECOMMIT_{phase:?}");
        let fixture = Fixture::new(&label);
        fixture.g5b_input_log(G5B_DATE);
        let candidate = g5b_frozen_envelope(&label, false);
        let before = database_snapshot(&fixture);
        let lock = day_lock_path(&fixture);
        let retained = lock.with_file_name("TEST_CODE_retained-precommit.g5b-day.lock");
        let hook_lock = lock.clone();
        let hook_retained = retained.clone();
        fixture
            .coordinator
            .install_database_operation_test_hook(phase, move || {
                std::fs::rename(&hook_lock, &hook_retained)?;
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&hook_lock)?;
                Ok(())
            })
            .unwrap();
        let result = fixture.coordinator.prepare(&candidate, 1, now());
        let hook_ran = retained.exists();
        fixture
            .cleanup
            .record_if_present(&retained, OwnedPathKind::FileOrSymlink);
        fixture
            .cleanup
            .record_if_present(&lock, OwnedPathKind::FileOrSymlink);
        if hook_ran {
            restore_lock_leaf(&lock, &retained);
        }

        assert!(hook_ran, "the after-SQL adversarial hook was not exercised");
        assert!(matches!(
            result,
            Err(DurableDeliveryError::IsolationViolation(_))
        ));
        assert_eq!(
            database_snapshot(&fixture),
            before,
            "{phase:?} must roll back"
        );
    }
}

#[test]
#[serial_test::serial(durable_physical_isolation)]
fn g5b_sqlite_date_fence_rejects_directory_swap_before_commit_and_rolls_back() {
    let fixture = Fixture::new("G5B_DIRECTORY_PRECOMMIT");
    fixture.g5b_input_log(G5B_DATE);
    let candidate = g5b_frozen_envelope("DIRECTORY_PRECOMMIT", false);
    let before = database_snapshot(&fixture);
    let root = day_lock_path(&fixture).parent().unwrap().to_path_buf();
    let retained = root.with_file_name(format!(
        "{}_RETAINED_PRECOMMIT",
        root.file_name().unwrap().to_str().unwrap()
    ));
    let hook_root = root.clone();
    let hook_retained = retained.clone();
    fixture
        .coordinator
        .install_database_operation_test_hook(
            DatabaseOperationTestPhase::AfterSqlBeforePreCommitValidation,
            move || {
                std::fs::rename(&hook_root, &hook_retained)?;
                std::fs::create_dir(&hook_root)?;
                Ok(())
            },
        )
        .unwrap();
    let result = fixture.coordinator.prepare(&candidate, 1, now());
    let hook_ran = retained.exists();
    fixture
        .cleanup
        .record_if_present(&retained, OwnedPathKind::Directory);
    for leaf in [
        "durable_delivery.sqlite3",
        "durable_delivery.sqlite3-journal",
        "durable_delivery.sqlite3-shm",
        "durable_delivery.sqlite3-wal",
        "20260818.g5b-day.lock",
    ] {
        fixture
            .cleanup
            .record_if_present(retained.join(leaf), OwnedPathKind::FileOrSymlink);
    }
    fixture
        .cleanup
        .record_if_present(&root, OwnedPathKind::Directory);
    if hook_ran {
        // This directory was created empty by the hook. Never recursively sweep
        // it, or inspect a substituted DB as the original authority.
        std::fs::remove_dir(&root).unwrap();
        std::fs::rename(&retained, &root).unwrap();
    }

    assert!(hook_ran);
    assert!(matches!(
        result,
        Err(DurableDeliveryError::IsolationViolation(_))
    ));
    assert_eq!(database_snapshot(&fixture), before);
}

#[test]
#[serial_test::serial(durable_physical_isolation)]
fn g5b_sqlite_date_fence_postcommit_inode_failure_preserves_committed_evidence() {
    let fixture = Fixture::new("G5B_LOCK_POSTCOMMIT");
    fixture.g5b_input_log(G5B_DATE);
    let candidate = g5b_frozen_envelope("LOCK_POSTCOMMIT", false);
    let before = database_snapshot(&fixture);
    let lock = day_lock_path(&fixture);
    let retained = lock.with_file_name("TEST_CODE_retained-postcommit.g5b-day.lock");
    let hook_lock = lock.clone();
    let hook_retained = retained.clone();
    fixture
        .coordinator
        .install_database_operation_test_hook(
            DatabaseOperationTestPhase::AfterCommitBeforePostValidation,
            move || {
                std::fs::rename(&hook_lock, &hook_retained)?;
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&hook_lock)?;
                Ok(())
            },
        )
        .unwrap();
    let result = fixture.coordinator.prepare(&candidate, 1, now());
    let hook_ran = retained.exists();
    fixture
        .cleanup
        .record_if_present(&retained, OwnedPathKind::FileOrSymlink);
    fixture
        .cleanup
        .record_if_present(&lock, OwnedPathKind::FileOrSymlink);
    if hook_ran {
        restore_lock_leaf(&lock, &retained);
    }

    assert!(hook_ran);
    let error = result.expect_err("changed postcommit lock identity must be reported");
    assert!(matches!(
        &error,
        DurableDeliveryError::IsolationViolation(_)
    ));
    assert!(error.to_string().contains("COMMIT succeeded"), "{error}");
    assert_ne!(database_snapshot(&fixture), before);
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM delivery_decisions"),
        1
    );
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM delivery_attempts"),
        0
    );
    assert!(fixture.query_i64("SELECT COUNT(*) FROM immutable_audit_outbox") > 0);
    assert_eq!(
        fixture.query_blob("SELECT envelope_canonical FROM delivery_decisions"),
        candidate.canonical_bytes().unwrap()
    );
}

fn assert_date_lock_available(lock: &Path) {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(lock)
        .unwrap();
    FileExt::try_lock_exclusive(&file)
        .expect("external callback entered while G5b date lock was still held");
    FileExt::unlock(&file).unwrap();
}

struct DateCheckingAppend {
    lock: PathBuf,
    inner: MemoryAppendPort,
    calls: AtomicUsize,
}

impl ImmutableAppendPort for DateCheckingAppend {
    fn append_exact(
        &self,
        record_kind: &str,
        identity: &str,
        canonical_bytes: &[u8],
        sha256: &str,
    ) -> Result<String> {
        assert_date_lock_available(&self.lock);
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner
            .append_exact(record_kind, identity, canonical_bytes, sha256)
    }
}

struct DateCheckingSink {
    lock: PathBuf,
    calls: AtomicUsize,
}

impl AuthoritativeSinkPort for DateCheckingSink {
    fn sink_identity(&self) -> &str {
        "TEST_CODE_DATE_CHECKING_SINK"
    }

    fn deliver(&self, _request: &AuthoritativeDeliveryRequest) -> AuthoritativeSinkResult {
        assert_date_lock_available(&self.lock);
        self.calls.fetch_add(1, Ordering::SeqCst);
        AuthoritativeSinkResult::Accepted(receipt(now()))
    }
}

#[test]
#[serial_test::serial(durable_physical_isolation)]
fn g5b_sqlite_date_fence_releases_before_physical_sink_and_immutable_callbacks() {
    let fixture = Fixture::new("G5B_EXTERNAL_CALLBACKS");
    fixture.g5b_input_log(G5B_DATE);
    let mut candidate = g5b_frozen_envelope("EXTERNAL_CALLBACKS", false);
    candidate.task_binding = Some(
        TaskBinding::new(
            "TEST_CODE_TASK_CALLBACK",
            b"TEST_CODE_CALLBACK_BASIS".to_vec(),
        )
        .unwrap(),
    );
    candidate.validate().unwrap();
    let append = DateCheckingAppend {
        lock: day_lock_path(&fixture),
        inner: MemoryAppendPort::default(),
        calls: AtomicUsize::new(0),
    };
    prepare_reserved(&fixture, &candidate, &append);
    let sink = Arc::new(DateCheckingSink {
        lock: day_lock_path(&fixture),
        calls: AtomicUsize::new(0),
    });
    let sinks: Vec<AuthoritativeSink> = vec![sink.clone()];
    let result = fixture
        .coordinator
        .resume_deliverable(&candidate.decision_identity, &sinks, now())
        .unwrap();
    assert_eq!(result.sink_calls, 1);
    assert!(result.persisted_receipt);
    reconcile_terminal(
        &fixture,
        &append,
        DecisionState::Delivered,
        &candidate.decision_identity,
    );
    assert_eq!(sink.calls.load(Ordering::SeqCst), 1);
    assert!(append.calls.load(Ordering::SeqCst) > 0);
    assert_eq!(append.inner.count_kind("BR-140TaskTransition"), 1);
    assert_eq!(
        fixture.query_i64(
            "SELECT COUNT(*) FROM sink_results
             WHERE result_kind='Accepted' AND authoritative_for_state=1",
        ),
        1
    );
}
