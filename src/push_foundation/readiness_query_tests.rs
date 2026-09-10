use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::monitor::push_job::{MachineCatalog, Namespace, ProducerId, RunId, UnitId};
use rusqlite::{params, Connection};

use super::activation::{DesiredActivationState, PromotionAction};
use super::activation_readiness::{read_activation_deployment_set, ActivationDeploymentSetError};
use super::activation_readiness_tests::{
    append_generation, first_producer, request, two_unit_database, CurrentBinding,
};
use super::operational_readiness::{DependencyKind, ReadinessScope, ReadinessStage};
use super::readiness_deployment_set_tests::{deployment_assessment, v3_context};
use super::readiness_query::{
    load_current_v3_candidate, load_current_v3_candidate_with_checkpoint,
    CurrentV3CandidateRequest, ReadinessQueryError,
};
use super::readiness_recovery::CandidateReadinessRecord;
use super::readiness_recovery_tests::{assessed, context};
use super::readiness_store::{ReadinessRecordStore, ReadinessStoreError, StoredReadinessRecord};
use super::readiness_store_schema::initialize_database;

fn directory_bytes(root: &Path) -> BTreeMap<String, Vec<u8>> {
    std::fs::read_dir(root)
        .expect("TEST_CODE read query fixture directory")
        .map(|entry| {
            let entry = entry.expect("TEST_CODE query fixture directory entry");
            let name = entry.file_name().to_string_lossy().into_owned();
            let bytes = std::fs::read(entry.path()).expect("TEST_CODE query fixture artifact");
            (name, bytes)
        })
        .collect()
}

struct V3QueryFixture {
    activation_root: tempfile::TempDir,
    activation_database: PathBuf,
    bindings: Vec<CurrentBinding>,
    catalog: MachineCatalog,
    selected_unit: UnitId,
    enabled: ProducerId,
    recovery_units: Vec<UnitId>,
    readiness_root: tempfile::TempDir,
    readiness_database: PathBuf,
    namespace: Namespace,
    candidate: CandidateReadinessRecord,
    stored: StoredReadinessRecord,
}

impl V3QueryFixture {
    fn new(name: &str) -> Self {
        let (activation_root, activation_database, bindings) =
            two_unit_database(&format!("TEST_CODE-{name}-activation.sqlite3"));
        let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
        let selected_unit = bindings[0].unit_id.clone();
        let enabled = first_producer(&selected_unit);
        let recovery_units = vec![bindings[1].unit_id.clone()];
        let deployment_set = read_activation_deployment_set(
            &activation_database,
            &selected_unit,
            request(&bindings, vec![enabled.clone()], recovery_units.clone()),
        )
        .expect("TEST_CODE persisted deployment set");
        let namespace = deployment_set.namespace().clone();
        let (assessment, evidence) = deployment_assessment(
            &catalog,
            &deployment_set,
            Some(DependencyKind::Schema),
            ReadinessStage::Running,
        );
        let candidate = CandidateReadinessRecord::try_new_v3(
            &catalog,
            None,
            v3_context(deployment_set, 100),
            assessment,
            evidence,
            vec![],
        )
        .expect("TEST_CODE v3 record");
        let readiness_root = tempfile::tempdir().expect("TEST_CODE readiness root");
        let readiness_database = readiness_root
            .path()
            .canonicalize()
            .expect("TEST_CODE canonical readiness root")
            .join(format!("TEST_CODE-{name}-readiness.sqlite3"));
        initialize_database(&readiness_database, &namespace).expect("TEST_CODE readiness database");
        let stored = ReadinessRecordStore::at(&readiness_database, &namespace, &catalog)
            .append(None, &candidate)
            .expect("TEST_CODE persist v3 record");
        Self {
            activation_root,
            activation_database,
            bindings,
            catalog,
            selected_unit,
            enabled,
            recovery_units,
            readiness_root,
            readiness_database,
            namespace,
            candidate,
            stored,
        }
    }

    fn request(&self) -> CurrentV3CandidateRequest<'_> {
        CurrentV3CandidateRequest::new(
            &self.readiness_database,
            &self.activation_database,
            &self.namespace,
            self.candidate.snapshot().snapshot_id(),
            &self.selected_unit,
            request(
                &self.bindings,
                vec![self.enabled.clone()],
                self.recovery_units.clone(),
            ),
        )
    }

    fn directory_bytes(&self) -> (BTreeMap<String, Vec<u8>>, BTreeMap<String, Vec<u8>>) {
        (
            directory_bytes(self.readiness_root.path()),
            directory_bytes(self.activation_root.path()),
        )
    }
}

#[test]
fn current_v3_query_reopens_and_returns_the_exact_stored_record_without_writes() {
    let fixture = V3QueryFixture::new("current-v3");
    assert_eq!(
        fixture.candidate.snapshot().assessment().scope(),
        &ReadinessScope::Core
    );
    let before = fixture.directory_bytes();

    let queried =
        load_current_v3_candidate(fixture.request()).expect("TEST_CODE current v3 candidate");

    assert_eq!(queried, fixture.stored);
    assert_eq!(
        queried
            .candidate()
            .snapshot()
            .deployment_set()
            .expect("TEST_CODE persisted v3 deployment set")
            .unit_generations()
            .len(),
        52
    );
    assert_eq!(fixture.directory_bytes(), before);
}

#[test]
fn current_v3_query_rejects_a_non_selected_unit_generation_change_without_writes() {
    let mut fixture = V3QueryFixture::new("non-selected-generation");
    let changed = append_generation(
        &fixture.activation_database,
        &fixture.bindings[1].unit_id,
        2,
        Some(&fixture.bindings[1]),
        DesiredActivationState::Shadow,
        PromotionAction::EnterShadow,
        'd',
        '4',
    );
    fixture.bindings[1] = changed;
    let before = fixture.directory_bytes();

    assert_eq!(
        load_current_v3_candidate(fixture.request()),
        Err(ReadinessQueryError::Activation(
            ActivationDeploymentSetError::DeploymentChanged
        )),
        "TEST_CODE the query must compare the complete set, not only selected Unit"
    );
    assert_eq!(fixture.directory_bytes(), before);
}

#[test]
fn current_v3_query_rejects_enabled_and_recovery_configuration_changes_without_writes() {
    let fixture = V3QueryFixture::new("configuration");
    let before = fixture.directory_bytes();
    let changed_requests = [
        request(&fixture.bindings, vec![], fixture.recovery_units.clone()),
        request(&fixture.bindings, vec![fixture.enabled.clone()], vec![]),
    ];

    for changed in changed_requests {
        assert_eq!(
            load_current_v3_candidate(CurrentV3CandidateRequest::new(
                &fixture.readiness_database,
                &fixture.activation_database,
                &fixture.namespace,
                fixture.candidate.snapshot().snapshot_id(),
                &fixture.selected_unit,
                changed,
            )),
            Err(ReadinessQueryError::Activation(
                ActivationDeploymentSetError::DeploymentChanged
            )),
            "TEST_CODE full enabled/recovery configuration is part of the candidate"
        );
    }
    assert_eq!(fixture.directory_bytes(), before);
}

#[test]
fn current_v3_query_rejects_a_different_activation_database_complete_set() {
    let fixture = V3QueryFixture::new("cross-database-source");
    let (other_root, other_database, mut other_bindings) =
        two_unit_database("TEST_CODE-cross-database-other.sqlite3");
    let changed = append_generation(
        &other_database,
        &other_bindings[1].unit_id,
        2,
        Some(&other_bindings[1]),
        DesiredActivationState::Shadow,
        PromotionAction::EnterShadow,
        'd',
        '4',
    );
    other_bindings[1] = changed;
    let readiness_before = directory_bytes(fixture.readiness_root.path());
    let activation_before = directory_bytes(other_root.path());

    assert_eq!(
        load_current_v3_candidate(CurrentV3CandidateRequest::new(
            &fixture.readiness_database,
            &other_database,
            &fixture.namespace,
            fixture.candidate.snapshot().snapshot_id(),
            &fixture.selected_unit,
            request(
                &other_bindings,
                vec![fixture.enabled.clone()],
                fixture.recovery_units.clone(),
            ),
        )),
        Err(ReadinessQueryError::Activation(
            ActivationDeploymentSetError::DeploymentChanged
        ))
    );
    assert_eq!(
        directory_bytes(fixture.readiness_root.path()),
        readiness_before
    );
    assert_eq!(directory_bytes(other_root.path()), activation_before);
}

#[test]
fn current_v3_query_rejects_valid_history_while_the_store_still_loads_it() {
    let fixture = V3QueryFixture::new("historical-record");
    let successor = CandidateReadinessRecord::try_new_v3(
        &fixture.catalog,
        Some(fixture.candidate.snapshot()),
        v3_context(
            fixture
                .candidate
                .snapshot()
                .deployment_set()
                .expect("TEST_CODE v3 set")
                .clone(),
            200,
        ),
        fixture.candidate.snapshot().assessment().clone(),
        fixture.candidate.snapshot().evidence_refs().to_vec(),
        vec![],
    )
    .expect("TEST_CODE successor");
    let store = ReadinessRecordStore::at(
        &fixture.readiness_database,
        &fixture.namespace,
        &fixture.catalog,
    );
    let current = store
        .append(Some(&fixture.stored), &successor)
        .expect("TEST_CODE append successor");
    let before = fixture.directory_bytes();

    assert_eq!(
        load_current_v3_candidate(fixture.request()),
        Err(ReadinessQueryError::Store(
            ReadinessStoreError::HeadConflict
        ))
    );
    assert_eq!(
        store
            .load_record(fixture.candidate.snapshot().snapshot_id())
            .expect("TEST_CODE historical load remains supported"),
        fixture.stored
    );
    assert_eq!(
        store
            .load_current(successor.snapshot().snapshot_id())
            .expect("TEST_CODE successor is current"),
        current
    );
    assert_eq!(fixture.directory_bytes(), before);
}

#[test]
fn current_v3_query_rejects_v2_without_reading_or_writing_activation_state() {
    let (activation_root, activation_database, bindings) =
        two_unit_database("TEST_CODE-v2-rejected-activation.sqlite3");
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let legacy_context = context(100);
    let namespace = legacy_context.namespace.clone();
    let (assessment, evidence) = assessed(&ReadinessScope::Core, Some(DependencyKind::Schema));
    let legacy =
        CandidateReadinessRecord::try_new(None, legacy_context, assessment, evidence, vec![])
            .expect("TEST_CODE v2 record");
    let readiness_root = tempfile::tempdir().expect("TEST_CODE readiness root");
    let readiness_database = readiness_root
        .path()
        .canonicalize()
        .expect("TEST_CODE readiness root")
        .join("TEST_CODE-v2-rejected-readiness.sqlite3");
    initialize_database(&readiness_database, &namespace).expect("TEST_CODE readiness database");
    let store = ReadinessRecordStore::at(&readiness_database, &namespace, &catalog);
    let stored = store
        .append(None, &legacy)
        .expect("TEST_CODE persist v2 record");
    let readiness_before = directory_bytes(readiness_root.path());
    let activation_before = directory_bytes(activation_root.path());

    assert_eq!(
        load_current_v3_candidate(CurrentV3CandidateRequest::new(
            &readiness_database,
            &activation_database,
            &namespace,
            legacy.snapshot().snapshot_id(),
            &bindings[0].unit_id,
            request(&bindings, vec![], vec![]),
        )),
        Err(ReadinessQueryError::UnsupportedSnapshotVersion)
    );
    assert_eq!(
        store
            .load_record(legacy.snapshot().snapshot_id())
            .expect("TEST_CODE legacy store read remains supported"),
        stored
    );
    assert_eq!(directory_bytes(readiness_root.path()), readiness_before);
    assert_eq!(directory_bytes(activation_root.path()), activation_before);
}

#[test]
fn current_v3_query_rechecks_the_head_after_activation_and_rejects_a_real_successor() {
    let fixture = V3QueryFixture::new("head-race");
    let successor = CandidateReadinessRecord::try_new_v3(
        &fixture.catalog,
        Some(fixture.candidate.snapshot()),
        v3_context(
            fixture
                .candidate
                .snapshot()
                .deployment_set()
                .expect("TEST_CODE v3 set")
                .clone(),
            200,
        ),
        fixture.candidate.snapshot().assessment().clone(),
        fixture.candidate.snapshot().evidence_refs().to_vec(),
        vec![],
    )
    .expect("TEST_CODE successor");
    let activation_before = directory_bytes(fixture.activation_root.path());

    let outcome = load_current_v3_candidate_with_checkpoint(fixture.request(), || {
        ReadinessRecordStore::at(
            &fixture.readiness_database,
            &fixture.namespace,
            &fixture.catalog,
        )
        .append(Some(&fixture.stored), &successor)
        .expect("TEST_CODE real committed successor during query");
    });

    assert_eq!(
        outcome,
        Err(ReadinessQueryError::Store(
            ReadinessStoreError::HeadConflict
        )),
        "TEST_CODE omitting the second current-head read would incorrectly return the old record"
    );
    let store = ReadinessRecordStore::at(
        &fixture.readiness_database,
        &fixture.namespace,
        &fixture.catalog,
    );
    assert_eq!(
        store
            .load_record(fixture.candidate.snapshot().snapshot_id())
            .expect("TEST_CODE old history remains intact"),
        fixture.stored
    );
    assert_eq!(
        store
            .load_current(successor.snapshot().snapshot_id())
            .expect("TEST_CODE checkpoint committed the successor")
            .candidate(),
        &successor
    );
    assert_eq!(
        directory_bytes(fixture.activation_root.path()),
        activation_before
    );
}

#[test]
fn current_v3_query_failures_and_request_debug_are_path_and_evidence_safe() {
    let fixture = V3QueryFixture::new("safe-errors");
    let missing_root = tempfile::tempdir().expect("TEST_CODE missing store root");
    let missing_database = missing_root
        .path()
        .join("TEST_CODE-PROTECTED-missing.sqlite3");
    let missing_before = directory_bytes(missing_root.path());
    let fixture_before = fixture.directory_bytes();
    let foreign_namespace = Namespace::test(
        RunId::try_new("TEST_CODE-foreign-query".to_owned()).expect("TEST_CODE foreign run"),
    );
    let request_debug = format!("{:?}", fixture.request());
    assert!(!request_debug.contains("TEST_CODE-safe-errors-readiness.sqlite3"));
    assert!(!request_debug.contains("TEST_CODE-safe-errors-activation.sqlite3"));
    assert!(!request_debug.contains("vault://"));

    let missing = load_current_v3_candidate(CurrentV3CandidateRequest::new(
        &missing_database,
        &fixture.activation_database,
        &fixture.namespace,
        fixture.candidate.snapshot().snapshot_id(),
        &fixture.selected_unit,
        request(
            &fixture.bindings,
            vec![fixture.enabled.clone()],
            fixture.recovery_units.clone(),
        ),
    ))
    .expect_err("TEST_CODE missing store rejected");
    let foreign = load_current_v3_candidate(CurrentV3CandidateRequest::new(
        &fixture.readiness_database,
        &fixture.activation_database,
        &foreign_namespace,
        fixture.candidate.snapshot().snapshot_id(),
        &fixture.selected_unit,
        request(
            &fixture.bindings,
            vec![fixture.enabled.clone()],
            fixture.recovery_units.clone(),
        ),
    ))
    .expect_err("TEST_CODE foreign namespace rejected");
    for error in [missing, foreign] {
        let debug = format!("{error:?}");
        assert!(!debug.contains("TEST_CODE-PROTECTED-missing.sqlite3"));
        assert!(!debug.contains("TEST_CODE-safe-errors-readiness.sqlite3"));
        assert!(!debug.contains("vault://TEST_CODE-SECRET"));
    }
    assert_eq!(directory_bytes(missing_root.path()), missing_before);
    assert_eq!(fixture.directory_bytes(), fixture_before);

    let protected_detail = "vault://TEST_CODE-SECRET/corrupt-readiness-snapshot";
    let connection = Connection::open(&fixture.readiness_database)
        .expect("TEST_CODE readiness corruption connection");
    let trigger_sql: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master \
             WHERE type='trigger' AND name='operational_readiness_snapshot_no_update'",
            [],
            |row| row.get(0),
        )
        .expect("TEST_CODE preserve snapshot trigger");
    connection
        .execute_batch("DROP TRIGGER operational_readiness_snapshot_no_update;")
        .expect("TEST_CODE disable snapshot update guard");
    connection
        .execute(
            "UPDATE operational_readiness_snapshot SET canonical_bytes=?1 WHERE snapshot_id=?2",
            params![
                protected_detail.as_bytes(),
                fixture.candidate.snapshot().snapshot_id().as_str()
            ],
        )
        .expect("TEST_CODE corrupt stored snapshot bytes");
    connection
        .execute_batch(&trigger_sql)
        .expect("TEST_CODE restore snapshot update guard");
    drop(connection);
    let damaged_before = fixture.directory_bytes();
    let damaged = load_current_v3_candidate(CurrentV3CandidateRequest::new(
        &fixture.readiness_database,
        &fixture.activation_database,
        &fixture.namespace,
        fixture.candidate.snapshot().snapshot_id(),
        &fixture.selected_unit,
        request(
            &fixture.bindings,
            vec![fixture.enabled.clone()],
            fixture.recovery_units.clone(),
        ),
    ))
    .expect_err("TEST_CODE damaged store rejected");
    assert!(matches!(damaged, ReadinessQueryError::Store(_)));
    assert!(!format!("{damaged:?}").contains(protected_detail));
    assert_eq!(fixture.directory_bytes(), damaged_before);
}
