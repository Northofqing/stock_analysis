use std::collections::BTreeMap;
use std::path::Path;

use crate::monitor::push_job::{MachineCatalog, ProducerId, UnitId, UtcMicros};
use rusqlite::{params, Connection};

use super::operational_readiness::{DependencyKind, ReadinessScope, ReadinessStatus};
use super::readiness_recovery::{
    CandidateReadinessRecord, CandidateRecoveryClaim, ReadinessRecoveryKind,
};
use super::readiness_recovery_tests::{assessed, context};
use super::readiness_store::{
    ReadinessAppendFault, ReadinessRecordStore, ReadinessStoreError, ReadinessStreamId,
};
use super::readiness_store_schema::{
    initialize_database, with_read_only, with_write_transaction, ReadinessSchemaError,
};

fn directory_bytes(root: &Path) -> BTreeMap<String, Vec<u8>> {
    std::fs::read_dir(root)
        .expect("TEST_CODE read store directory")
        .map(|entry| {
            let entry = entry.expect("TEST_CODE store directory entry");
            let name = entry.file_name().to_string_lossy().into_owned();
            let bytes = std::fs::read(entry.path()).expect("TEST_CODE store artifact bytes");
            (name, bytes)
        })
        .collect()
}

#[test]
fn w15_store_reopens_the_same_pending_snapshot_event_and_head() {
    let root = tempfile::tempdir().expect("TEST_CODE store root");
    let database = root
        .path()
        .canonicalize()
        .expect("TEST_CODE canonical store root")
        .join("operational-readiness.sqlite3");
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let context = context(100);
    let namespace = context.namespace.clone();
    initialize_database(&database, &namespace).expect("TEST_CODE initialize explicit store");
    let (assessment, evidence) = assessed(&ReadinessScope::Core, Some(DependencyKind::Schema));
    let pending = CandidateReadinessRecord::try_new(None, context, assessment, evidence, vec![])
        .expect("TEST_CODE pending record");
    let stream = ReadinessStreamId::for_snapshot(pending.snapshot());
    let committed = {
        let store = ReadinessRecordStore::at(&database, &namespace, &catalog);
        assert_eq!(store.load_head(&stream).expect("TEST_CODE no head"), None);
        store
            .append(None, &pending)
            .expect("TEST_CODE first commit")
    };
    assert_eq!(committed.version(), 1);
    assert_eq!(committed.candidate(), &pending);
    assert_eq!(committed.candidate().kind(), ReadinessRecoveryKind::Pending);
    assert_eq!(
        committed.candidate().snapshot().assessment().status(),
        ReadinessStatus::CoreUnready
    );

    let reopened = ReadinessRecordStore::at(&database, &namespace, &catalog);
    assert_eq!(
        reopened
            .load_head(&stream)
            .expect("TEST_CODE reopened head"),
        Some(committed.clone())
    );
    assert_eq!(
        reopened
            .load_record(pending.snapshot().snapshot_id())
            .expect("TEST_CODE reopened record"),
        committed
    );
    assert_eq!(
        reopened
            .load_current(pending.snapshot().snapshot_id())
            .expect("TEST_CODE reopened current record"),
        committed
    );
    assert!(!format!("{committed:?}").contains("TEST_CODE-SECRET"));
}

#[test]
fn w15_store_rolls_back_event_snapshot_and_head_at_each_write_boundary() {
    for fault in [
        ReadinessAppendFault::Event,
        ReadinessAppendFault::Snapshot,
        ReadinessAppendFault::Head,
    ] {
        let root = tempfile::tempdir().expect("TEST_CODE atomic store root");
        let database = root
            .path()
            .canonicalize()
            .expect("TEST_CODE canonical root")
            .join("readiness.sqlite3");
        let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
        let namespace = context(100).namespace;
        initialize_database(&database, &namespace).expect("TEST_CODE initialize store");
        let store = ReadinessRecordStore::at(&database, &namespace, &catalog);
        let (assessment, evidence) = assessed(&ReadinessScope::Core, Some(DependencyKind::Schema));
        let before = CandidateReadinessRecord::try_new(
            None,
            context(100),
            assessment.clone(),
            evidence.clone(),
            vec![],
        )
        .expect("TEST_CODE first record");
        let first = store
            .append(None, &before)
            .expect("TEST_CODE initial commit");
        let after = CandidateReadinessRecord::try_new(
            Some(before.snapshot()),
            context(200),
            assessment,
            evidence,
            vec![],
        )
        .expect("TEST_CODE second pending");
        let stream = ReadinessStreamId::for_snapshot(before.snapshot());

        assert_eq!(
            store.append_with_fault(Some(&first), &after, fault),
            Err(ReadinessStoreError::Storage {
                operation: "injected_before_commit"
            }),
            "TEST_CODE pre-commit callback errors remain unchanged"
        );
        let reopened = ReadinessRecordStore::at(&database, &namespace, &catalog);
        assert_eq!(
            reopened
                .load_head(&stream)
                .expect("TEST_CODE old head survives"),
            Some(first.clone())
        );
        assert_eq!(
            reopened.load_record(after.snapshot().snapshot_id()),
            Err(ReadinessStoreError::RecordMissing)
        );
        // This would hit immutable-row uniqueness if either insert had escaped rollback.
        let retried = reopened
            .append(Some(&first), &after)
            .expect("TEST_CODE clean retry");
        assert_eq!(retried.version(), 2);
        assert_eq!(retried.candidate(), &after);
        assert_eq!(
            reopened
                .load_record(before.snapshot().snapshot_id())
                .expect("TEST_CODE original history"),
            first
        );
    }
}

#[test]
fn w15_store_rejects_middle_event_corruption_without_repair_or_sidecar_files() {
    let root = tempfile::tempdir().expect("TEST_CODE middle corruption root");
    let database = root
        .path()
        .canonicalize()
        .expect("TEST_CODE canonical root")
        .join("readiness.sqlite3");
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let namespace = context(100).namespace;
    initialize_database(&database, &namespace).expect("TEST_CODE initialize");
    let (assessment, evidence) = assessed(&ReadinessScope::Core, Some(DependencyKind::Schema));
    let first = CandidateReadinessRecord::try_new(
        None,
        context(100),
        assessment.clone(),
        evidence.clone(),
        vec![],
    )
    .expect("TEST_CODE first pending");
    let second = CandidateReadinessRecord::try_new(
        Some(first.snapshot()),
        context(200),
        assessment.clone(),
        evidence.clone(),
        vec![],
    )
    .expect("TEST_CODE second pending");
    let third = CandidateReadinessRecord::try_new(
        Some(second.snapshot()),
        context(300),
        assessment.clone(),
        evidence.clone(),
        vec![],
    )
    .expect("TEST_CODE third pending");
    let fourth = CandidateReadinessRecord::try_new(
        Some(third.snapshot()),
        context(400),
        assessment,
        evidence,
        vec![],
    )
    .expect("TEST_CODE fourth candidate");
    let store = ReadinessRecordStore::at(&database, &namespace, &catalog);
    let first_committed = store.append(None, &first).expect("TEST_CODE commit first");
    let second_committed = store
        .append(Some(&first_committed), &second)
        .expect("TEST_CODE commit second");
    let third_committed = store
        .append(Some(&second_committed), &third)
        .expect("TEST_CODE commit third");

    {
        let connection = Connection::open(&database).expect("TEST_CODE fault connection");
        let trigger_sql: String = connection
            .query_row(
                "SELECT sql FROM sqlite_master \
                 WHERE type='trigger' \
                   AND name='operational_readiness_recovery_event_no_update'",
                [],
                |row| row.get(0),
            )
            .expect("TEST_CODE preserve exact event trigger");
        connection
            .execute_batch("DROP TRIGGER operational_readiness_recovery_event_no_update;")
            .expect("TEST_CODE disable event update guard");
        connection
            .execute(
                "UPDATE operational_readiness_recovery_event \
                 SET canonical_bytes=?1 WHERE event_id=?2",
                params![
                    b"TEST_CODE-corrupt-event".as_slice(),
                    second.snapshot().recovery_event_id().as_str()
                ],
            )
            .expect("TEST_CODE corrupt middle event bytes");
        connection
            .execute_batch(&trigger_sql)
            .expect("TEST_CODE restore exact event trigger");
    }

    with_read_only(
        &database,
        &namespace,
        |_| Ok::<(), ReadinessSchemaError>(()),
    )
    .expect("TEST_CODE restored schema remains valid");
    let damaged_directory = directory_bytes(root.path());
    let stream = ReadinessStreamId::for_snapshot(third.snapshot());

    assert!(matches!(
        store.load_head(&stream),
        Err(ReadinessStoreError::Event(_))
    ));
    assert!(matches!(
        store.load_record(second.snapshot().snapshot_id()),
        Err(ReadinessStoreError::Event(_))
    ));
    assert!(matches!(
        store.append(Some(&third_committed), &fourth),
        Err(ReadinessStoreError::Event(_))
    ));
    assert_eq!(
        directory_bytes(root.path()),
        damaged_directory,
        "TEST_CODE rejected operations neither repair nor create sidecar files"
    );
}

#[test]
fn w15_store_replays_old_commits_after_response_loss_without_advancing_the_head() {
    let root = tempfile::tempdir().expect("TEST_CODE replay root");
    let database = root
        .path()
        .canonicalize()
        .expect("TEST_CODE canonical root")
        .join("readiness.sqlite3");
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let namespace = context(100).namespace;
    initialize_database(&database, &namespace).expect("TEST_CODE initialize");
    let store = ReadinessRecordStore::at(&database, &namespace, &catalog);
    let (assessment, evidence) = assessed(&ReadinessScope::Core, Some(DependencyKind::Schema));
    let before = CandidateReadinessRecord::try_new(
        None,
        context(100),
        assessment.clone(),
        evidence.clone(),
        vec![],
    )
    .expect("TEST_CODE first pending");
    let first = store.append(None, &before).expect("TEST_CODE first commit");
    let after = CandidateReadinessRecord::try_new(
        Some(before.snapshot()),
        context(200),
        assessment.clone(),
        evidence.clone(),
        vec![],
    )
    .expect("TEST_CODE second pending");
    // Lose the caller's response after durable commit, then reconstruct the store.
    drop(
        store
            .append(Some(&first), &after)
            .expect("TEST_CODE committed but response lost"),
    );
    let reopened = ReadinessRecordStore::at(&database, &namespace, &catalog);
    let second = reopened
        .load_record(after.snapshot().snapshot_id())
        .expect("TEST_CODE exact requery");
    assert_eq!(second.version(), 2);
    assert_eq!(second.candidate(), &after);
    assert_eq!(
        reopened
            .append(Some(&first), &after)
            .expect("TEST_CODE repeated second"),
        second
    );
    assert_eq!(
        reopened
            .append(None, &before)
            .expect("TEST_CODE historical replay"),
        first
    );
    let stream = ReadinessStreamId::for_snapshot(after.snapshot());
    assert_eq!(
        reopened
            .load_head(&stream)
            .expect("TEST_CODE head stays second"),
        Some(second.clone())
    );

    let competitor = CandidateReadinessRecord::try_new(
        Some(before.snapshot()),
        context(300),
        assessment,
        evidence,
        vec![],
    )
    .expect("TEST_CODE competing successor");
    assert_eq!(
        reopened.append(Some(&first), &competitor),
        Err(ReadinessStoreError::HeadConflict)
    );
    assert_eq!(
        reopened.append(Some(&second), &competitor),
        Err(ReadinessStoreError::HeadConflict)
    );
    assert_eq!(
        reopened.load_record(competitor.snapshot().snapshot_id()),
        Err(ReadinessStoreError::RecordMissing)
    );
    assert_eq!(
        reopened
            .load_head(&stream)
            .expect("TEST_CODE winner preserved"),
        Some(second)
    );
}

#[test]
fn w15_store_rechecks_exact_committed_record_when_commit_confirmation_is_lost() {
    let root = tempfile::tempdir().expect("TEST_CODE commit confirmation root");
    let database = root
        .path()
        .canonicalize()
        .expect("TEST_CODE canonical root")
        .join("readiness.sqlite3");
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let namespace = context(100).namespace;
    initialize_database(&database, &namespace).expect("TEST_CODE initialize");
    let store = ReadinessRecordStore::at(&database, &namespace, &catalog);
    let (assessment, evidence) = assessed(&ReadinessScope::Core, Some(DependencyKind::Schema));
    let before = CandidateReadinessRecord::try_new(
        None,
        context(100),
        assessment.clone(),
        evidence.clone(),
        vec![],
    )
    .expect("TEST_CODE first pending");
    let first = store.append(None, &before).expect("TEST_CODE first commit");
    let after = CandidateReadinessRecord::try_new(
        Some(before.snapshot()),
        context(200),
        assessment,
        evidence,
        vec![],
    )
    .expect("TEST_CODE second pending");

    // This is a lost-confirmation simulation after a real durable commit, not a
    // claim that SQLite produced an I/O failure. It must use the same
    // commit-error classification and exact-record recheck as a real failure.
    let recovered = store
        .append_with_fault(
            Some(&first),
            &after,
            ReadinessAppendFault::CommitConfirmationLost,
        )
        .expect("TEST_CODE exact committed fact resolves lost confirmation");

    assert_eq!(recovered.version(), 2);
    assert_eq!(recovered.candidate(), &after);
    let reopened = ReadinessRecordStore::at(&database, &namespace, &catalog);
    assert_eq!(
        reopened
            .load_record(after.snapshot().snapshot_id())
            .expect("TEST_CODE reopened exact record"),
        recovered
    );
    assert_eq!(
        reopened
            .load_head(&ReadinessStreamId::for_snapshot(after.snapshot()))
            .expect("TEST_CODE reopened head"),
        Some(recovered.clone())
    );
    assert_eq!(
        reopened
            .append(Some(&first), &after)
            .expect("TEST_CODE exact historical replay"),
        recovered,
        "TEST_CODE lost confirmation must not create a second event or advance the head"
    );
}

#[test]
fn w15_store_fails_closed_after_a_real_deferred_foreign_key_commit_rejection() {
    let root = tempfile::tempdir().expect("TEST_CODE deferred foreign key root");
    let database = root
        .path()
        .canonicalize()
        .expect("TEST_CODE canonical root")
        .join("readiness.sqlite3");
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let namespace = context(100).namespace;
    initialize_database(&database, &namespace).expect("TEST_CODE initialize");
    let store = ReadinessRecordStore::at(&database, &namespace, &catalog);
    let (assessment, evidence) = assessed(&ReadinessScope::Core, Some(DependencyKind::Schema));
    let before = CandidateReadinessRecord::try_new(
        None,
        context(100),
        assessment.clone(),
        evidence.clone(),
        vec![],
    )
    .expect("TEST_CODE first pending");
    let first = store.append(None, &before).expect("TEST_CODE first commit");
    let after = CandidateReadinessRecord::try_new(
        Some(before.snapshot()),
        context(200),
        assessment,
        evidence,
        vec![],
    )
    .expect("TEST_CODE second pending");
    let stream = ReadinessStreamId::for_snapshot(after.snapshot());

    // The test-only checkpoint executes a valid INSERT whose deferred foreign
    // key is unresolved. Reaching CommitUnconfirmed proves INSERT succeeded and
    // SQLite rejected the real transaction COMMIT, not the statement itself.
    assert_eq!(
        store.append_with_fault(
            Some(&first),
            &after,
            ReadinessAppendFault::DeferredForeignKeyCommitRejected,
        ),
        Err(ReadinessStoreError::CommitUnconfirmed)
    );
    assert_eq!(
        store
            .load_head(&stream)
            .expect("TEST_CODE rejected commit preserves head"),
        Some(first.clone())
    );
    assert_eq!(
        store.load_record(after.snapshot().snapshot_id()),
        Err(ReadinessStoreError::RecordMissing)
    );
    assert_eq!(
        store
            .load_record(before.snapshot().snapshot_id())
            .expect("TEST_CODE rejected commit preserves history"),
        first
    );

    let retried = store
        .append(Some(&first), &after)
        .expect("TEST_CODE explicit exact retry after unconfirmed commit");
    assert_eq!(retried.version(), 2);
    assert_eq!(retried.candidate(), &after);
    assert_eq!(
        store
            .load_head(&stream)
            .expect("TEST_CODE explicit retry becomes head"),
        Some(retried)
    );
}

#[test]
fn w15_store_does_not_confirm_a_lost_commit_when_exact_recheck_is_corrupt() {
    let root = tempfile::tempdir().expect("TEST_CODE corrupt recheck root");
    let database = root
        .path()
        .canonicalize()
        .expect("TEST_CODE canonical root")
        .join("readiness.sqlite3");
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let namespace = context(100).namespace;
    initialize_database(&database, &namespace).expect("TEST_CODE initialize");
    let store = ReadinessRecordStore::at(&database, &namespace, &catalog);
    let (assessment, evidence) = assessed(&ReadinessScope::Core, Some(DependencyKind::Schema));
    let candidate =
        CandidateReadinessRecord::try_new(None, context(100), assessment, evidence, vec![])
            .expect("TEST_CODE pending");
    let protected_detail = "file:///TEST_CODE-secret/readiness.sqlite3?sql=SELECT-canonical_bytes";

    let error = store
        .append_with_lost_confirmation_hook(None, &candidate, || {
            let connection = Connection::open(&database).expect("TEST_CODE corruption connection");
            let trigger_sql: String = connection
                .query_row(
                    "SELECT sql FROM sqlite_master \
                     WHERE type='trigger' \
                       AND name='operational_readiness_snapshot_no_update'",
                    [],
                    |row| row.get(0),
                )
                .expect("TEST_CODE preserve snapshot trigger");
            connection
                .execute_batch("DROP TRIGGER operational_readiness_snapshot_no_update;")
                .expect("TEST_CODE disable snapshot update guard");
            connection
                .execute(
                    "UPDATE operational_readiness_snapshot \
                     SET canonical_bytes=?1 WHERE snapshot_id=?2",
                    params![
                        protected_detail.as_bytes(),
                        candidate.snapshot().snapshot_id().as_str()
                    ],
                )
                .expect("TEST_CODE corrupt exact committed snapshot");
            connection
                .execute_batch(&trigger_sql)
                .expect("TEST_CODE restore exact snapshot trigger");
        })
        .expect_err("TEST_CODE corrupt exact recheck cannot confirm commit");

    assert_eq!(error, ReadinessStoreError::CommitUnconfirmed);
    let display = format!("{error}");
    let debug = format!("{error:?}");
    assert!(display.contains("unconfirmed"));
    assert!(debug.contains("CommitUnconfirmed"));
    for rendered in [display, debug] {
        assert!(!rendered.contains(protected_detail));
        assert!(!rendered.contains("SELECT"));
        assert!(!rendered.contains("canonical_bytes"));
    }
}

#[test]
fn w15_store_does_not_confirm_a_lost_commit_when_exact_recheck_fails() {
    let root = tempfile::tempdir().expect("TEST_CODE failed recheck root");
    let database = root
        .path()
        .canonicalize()
        .expect("TEST_CODE canonical root")
        .join("readiness.sqlite3");
    let unavailable = root.path().join("TEST_CODE-hidden-readiness.sqlite3");
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let namespace = context(100).namespace;
    initialize_database(&database, &namespace).expect("TEST_CODE initialize");
    let store = ReadinessRecordStore::at(&database, &namespace, &catalog);
    let (assessment, evidence) = assessed(&ReadinessScope::Core, Some(DependencyKind::Schema));
    let candidate =
        CandidateReadinessRecord::try_new(None, context(100), assessment, evidence, vec![])
            .expect("TEST_CODE pending");

    let error = store
        .append_with_lost_confirmation_hook(None, &candidate, || {
            std::fs::rename(&database, &unavailable)
                .expect("TEST_CODE hide committed database before exact recheck");
        })
        .expect_err("TEST_CODE failed exact recheck cannot confirm commit");
    assert_eq!(error, ReadinessStoreError::CommitUnconfirmed);

    std::fs::rename(&unavailable, &database).expect("TEST_CODE restore committed database");
    assert_eq!(
        store
            .load_record(candidate.snapshot().snapshot_id())
            .expect("TEST_CODE committed fact remains after recheck failure")
            .candidate(),
        &candidate
    );
}

#[test]
fn w15_store_reopens_an_explicit_producer_recovery_with_both_ends_intact() {
    let root = tempfile::tempdir().expect("TEST_CODE recovery store root");
    let database = root
        .path()
        .canonicalize()
        .expect("TEST_CODE canonical root")
        .join("readiness.sqlite3");
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let namespace = context(100).namespace;
    initialize_database(&database, &namespace).expect("TEST_CODE initialize");
    let scope = ReadinessScope::Producer {
        unit_id: UnitId::try_new("MU-p01".to_owned()).expect("TEST_CODE unit"),
        producer_id: ProducerId::try_new("p01-scheduled".to_owned()).expect("TEST_CODE producer"),
    };
    let (assessment, evidence) = assessed(&scope, Some(DependencyKind::SourceContract));
    let pending =
        CandidateReadinessRecord::try_new(None, context(100), assessment, evidence, vec![])
            .expect("TEST_CODE pending producer");
    let store = ReadinessRecordStore::at(&database, &namespace, &catalog);
    let first = store
        .append(None, &pending)
        .expect("TEST_CODE initial pending");
    let (assessment, evidence) = assessed(&scope, None);
    let recovered_ref = evidence
        .iter()
        .find(|item| item.dependency_kind() == DependencyKind::SourceContract)
        .expect("TEST_CODE recovery role")
        .clone();
    let recovered = CandidateReadinessRecord::try_new(
        Some(pending.snapshot()),
        context(200),
        assessment,
        evidence,
        vec![CandidateRecoveryClaim {
            evidence: recovered_ref,
            observed_at: UtcMicros::try_new(150).expect("TEST_CODE observation"),
        }],
    )
    .expect("TEST_CODE explicit recovery material");
    let second = store
        .append(Some(&first), &recovered)
        .expect("TEST_CODE recovery commit");
    let reopened = ReadinessRecordStore::at(&database, &namespace, &catalog);
    let queried = reopened
        .load_record(recovered.snapshot().snapshot_id())
        .expect("TEST_CODE requery full event");
    assert_eq!(queried, second);
    assert_eq!(queried.version(), 2);
    assert_eq!(
        queried.candidate().kind(),
        ReadinessRecoveryKind::ProducerContractRestored
    );
    assert_eq!(
        queried.candidate().before_snapshot_id(),
        Some(pending.snapshot().snapshot_id())
    );
    assert_eq!(
        queried.candidate().snapshot().assessment().status(),
        ReadinessStatus::Ready
    );
    assert_eq!(
        reopened
            .load_record(pending.snapshot().snapshot_id())
            .expect("TEST_CODE old endpoint"),
        first
    );
    assert_eq!(
        reopened.load_current(pending.snapshot().snapshot_id()),
        Err(ReadinessStoreError::HeadConflict),
        "TEST_CODE a valid historical record is not the current record"
    );
    assert_eq!(
        reopened
            .load_current(recovered.snapshot().snapshot_id())
            .expect("TEST_CODE current-only endpoint"),
        second
    );
    assert_eq!(
        reopened
            .load_head(&ReadinessStreamId::for_snapshot(recovered.snapshot()))
            .expect("TEST_CODE current endpoint"),
        Some(second)
    );
}

#[test]
fn w15_store_rejects_head_version_corruption_without_repairing_it() {
    let root = tempfile::tempdir().expect("TEST_CODE corruption root");
    let database = root
        .path()
        .canonicalize()
        .expect("TEST_CODE canonical root")
        .join("readiness.sqlite3");
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let namespace = context(100).namespace;
    initialize_database(&database, &namespace).expect("TEST_CODE initialize");
    let (assessment, evidence) = assessed(&ReadinessScope::Core, Some(DependencyKind::Schema));
    let candidate =
        CandidateReadinessRecord::try_new(None, context(100), assessment, evidence, vec![])
            .expect("TEST_CODE pending");
    let store = ReadinessRecordStore::at(&database, &namespace, &catalog);
    store.append(None, &candidate).expect("TEST_CODE commit");
    with_write_transaction(&database, &namespace, |connection| {
        connection
            .execute("UPDATE operational_readiness_head SET version=9", [])
            .map_err(|_| ReadinessSchemaError::ValidationFailed {
                check: "TEST_CODE head fault",
            })?;
        Ok::<(), ReadinessSchemaError>(())
    })
    .expect("TEST_CODE corrupt only persisted head version");
    // These ordinary file reads happen with every SQLite connection closed.
    let damaged = std::fs::read(&database).expect("TEST_CODE damaged file");
    let stream = ReadinessStreamId::for_snapshot(candidate.snapshot());
    assert_eq!(
        store.load_head(&stream),
        Err(ReadinessStoreError::Corrupt { check: "head_join" })
    );
    assert_eq!(
        store.load_record(candidate.snapshot().snapshot_id()),
        Err(ReadinessStoreError::Corrupt { check: "head_join" })
    );
    assert_eq!(
        std::fs::read(&database).expect("TEST_CODE rejected read did not repair"),
        damaged
    );
}

#[test]
fn w15_store_concurrent_successors_cannot_both_commit_the_same_head_version() {
    let root = tempfile::tempdir().expect("TEST_CODE concurrent root");
    let database = root
        .path()
        .canonicalize()
        .expect("TEST_CODE canonical root")
        .join("readiness.sqlite3");
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let namespace = context(100).namespace;
    initialize_database(&database, &namespace).expect("TEST_CODE initialize");
    let store = ReadinessRecordStore::at(&database, &namespace, &catalog);
    let (assessment, evidence) = assessed(&ReadinessScope::Core, Some(DependencyKind::Schema));
    let before = CandidateReadinessRecord::try_new(
        None,
        context(100),
        assessment.clone(),
        evidence.clone(),
        vec![],
    )
    .expect("TEST_CODE first");
    let first = store
        .append(None, &before)
        .expect("TEST_CODE initial commit");
    let left = CandidateReadinessRecord::try_new(
        Some(before.snapshot()),
        context(200),
        assessment.clone(),
        evidence.clone(),
        vec![],
    )
    .expect("TEST_CODE left");
    let right = CandidateReadinessRecord::try_new(
        Some(before.snapshot()),
        context(300),
        assessment,
        evidence,
        vec![],
    )
    .expect("TEST_CODE right");
    let barrier = std::sync::Barrier::new(2);
    let outcomes = std::thread::scope(|threads| {
        let left_thread = threads.spawn(|| {
            barrier.wait();
            ReadinessRecordStore::at(&database, &namespace, &catalog).append(Some(&first), &left)
        });
        let right_thread = threads.spawn(|| {
            barrier.wait();
            ReadinessRecordStore::at(&database, &namespace, &catalog).append(Some(&first), &right)
        });
        [
            left_thread.join().expect("TEST_CODE left thread"),
            right_thread.join().expect("TEST_CODE right thread"),
        ]
    });
    let winners: Vec<_> = outcomes
        .iter()
        .filter_map(|outcome| outcome.as_ref().ok())
        .collect();
    assert!(
        winners.len() <= 1,
        "TEST_CODE competing head commits: {outcomes:?}"
    );
    for error in outcomes.iter().filter_map(|outcome| outcome.as_ref().err()) {
        assert!(
            matches!(
                error,
                ReadinessStoreError::HeadConflict
                    | ReadinessStoreError::CommitUnconfirmed
                    | ReadinessStoreError::Schema(
                        ReadinessSchemaError::ConnectionSafeguardFailed {
                            check: "begin_write_transaction"
                        }
                    )
                    | ReadinessStoreError::Schema(ReadinessSchemaError::ValidationFailed {
                        check: "main_shared_lock"
                    })
            ),
            "TEST_CODE unexpected concurrency failure: {error:?}"
        );
    }
    let stream = ReadinessStreamId::for_snapshot(before.snapshot());
    if let Some(winner) = winners.first() {
        assert_eq!(winner.version(), 2);
        assert_eq!(
            store.load_head(&stream).expect("TEST_CODE single winner"),
            Some((*winner).clone())
        );
        let loser = if winner.candidate() == &left {
            &right
        } else {
            &left
        };
        assert_eq!(
            store.load_record(loser.snapshot().snapshot_id()),
            Err(ReadinessStoreError::RecordMissing)
        );
        assert_eq!(
            store.append(Some(&first), loser),
            Err(ReadinessStoreError::HeadConflict)
        );
    } else {
        // SQLite may reject both upgrades under contention; neither may leave a partial commit.
        assert_eq!(
            store
                .load_head(&stream)
                .expect("TEST_CODE no partial winner"),
            Some(first.clone())
        );
        assert_eq!(
            store.load_record(left.snapshot().snapshot_id()),
            Err(ReadinessStoreError::RecordMissing)
        );
        assert_eq!(
            store.load_record(right.snapshot().snapshot_id()),
            Err(ReadinessStoreError::RecordMissing)
        );
        assert_eq!(
            store
                .append(Some(&first), &left)
                .expect("TEST_CODE retry after contention")
                .version(),
            2
        );
    }
}
