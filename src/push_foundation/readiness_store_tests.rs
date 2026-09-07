use crate::monitor::push_job::MachineCatalog;

use super::operational_readiness::{DependencyKind, ReadinessScope, ReadinessStatus};
use super::readiness_recovery::{CandidateReadinessRecord, ReadinessRecoveryKind};
use super::readiness_recovery_tests::{assessed, context};
use super::readiness_store::{ReadinessRecordStore, ReadinessStreamId};
use super::readiness_store_schema::initialize_database;

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
    assert!(!format!("{committed:?}").contains("TEST_CODE-SECRET"));
}
