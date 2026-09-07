use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

use rusqlite::{Connection, TransactionBehavior};

use crate::monitor::push_job::{raw_digest, GitSha40, Sha256Digest, UnitId};

use super::activation::{ActivationManifest, DesiredActivationState, PromotionJournalEntry};
use super::activation_codec::{journal_digest, manifest_digest, promotion_event_id};
use super::activation_transaction::{
    apply_activation_candidate, apply_activation_candidate_with_fault, ActivationApplyCandidate,
    ActivationApplyOutcome, ActivationOwnerCoordinationError, ActivationOwnerCoordinator,
    ActivationTransactionError, ActivationTransactionFault, UtcMicrosRange,
};
use super::migration::FoundationSchemaMigration;
use super::{inspect_raw_activation_facts, ActivationReconciliation, PromotionAction};

const UNIT_A: &str = "MU-p01";
const UNIT_B: &str = "MU-d01";
const DAY: u64 = 1_000_000;
const DAY_LENGTH: u64 = 86_400_000_000;

fn unit(value: &str) -> UnitId {
    UnitId::try_new(value.to_owned()).expect("TEST_CODE unit")
}

fn digest(character: char) -> Sha256Digest {
    Sha256Digest::parse("TEST_CODE digest", &character.to_string().repeat(64))
        .expect("TEST_CODE digest")
}

fn initialized_database(name: &str) -> (tempfile::TempDir, PathBuf) {
    let root = tempfile::tempdir().expect("TEST_CODE temp root");
    let database = root
        .path()
        .canonicalize()
        .expect("TEST_CODE canonical root")
        .join(name);
    let migration = FoundationSchemaMigration::bundled().expect("TEST_CODE bundled schema");
    let ddl = migration
        .ddl_bytes()
        .strip_prefix(b".bail on\n")
        .expect("TEST_CODE library-safe DDL suffix");
    let ddl = std::str::from_utf8(ddl).expect("TEST_CODE UTF-8 DDL");
    Connection::open(&database)
        .expect("TEST_CODE database")
        .execute_batch(ddl)
        .expect("TEST_CODE synthetic frozen schema");
    (root, database)
}

fn open_test_connection(database: &Path) -> Connection {
    let connection = Connection::open(database).expect("TEST_CODE connection");
    connection
        .busy_timeout(Duration::from_secs(5))
        .expect("TEST_CODE busy timeout");
    connection
}

#[allow(clippy::too_many_arguments)]
fn candidate(
    unit_id: &str,
    desired_state: DesiredActivationState,
    action: PromotionAction,
    physical_owner: &str,
    predecessor: Option<&ActivationManifest>,
    previous_journal: Option<&PromotionJournalEntry>,
    rollback_target: Option<&ActivationManifest>,
    day_start: u64,
    occurred_at: u64,
) -> ActivationApplyCandidate {
    let expected_generation = predecessor.map_or(0, ActivationManifest::generation);
    let generation = expected_generation + 1;
    let mut manifest = ActivationManifest {
        manifest_sha256: digest('0'),
        unit_id: unit(unit_id),
        generation,
        previous_manifest_sha256: predecessor.map(|value| value.manifest_sha256().clone()),
        desired_state,
        physical_owner: physical_owner.to_owned(),
        build_commit: GitSha40::parse(&"a".repeat(40)).expect("TEST_CODE git SHA"),
        build_sha256: digest('b'),
        catalog_sha256: digest('c'),
        business_schema_sha256: digest('d'),
        durable_schema_sha256: digest('e'),
        template_sha256: digest('f'),
        source_contract_sha256: digest('1'),
        evidence_sha256: raw_digest(format!("evidence-{unit_id}-{generation}").as_bytes()),
        approved_by: "operator-a".to_owned(),
        approved_at: occurred_at - 2,
        window_start: occurred_at - 10,
        window_end: occurred_at + 10,
        rollback_target_sha256: rollback_target.map(|value| value.manifest_sha256().clone()),
        created_at: occurred_at - 1,
    };
    manifest.manifest_sha256 = manifest_digest(&manifest);

    let mut journal = PromotionJournalEntry {
        event_id: promotion_event_id(unit_id, generation),
        unit_id: unit(unit_id),
        generation,
        from_manifest_sha256: predecessor.map(|value| value.manifest_sha256().clone()),
        to_manifest_sha256: manifest.manifest_sha256().clone(),
        actor: manifest.approved_by().to_owned(),
        action,
        reason: "activation.applied".to_owned(),
        window_start: manifest.window_start(),
        window_end: manifest.window_end(),
        evidence_sha256: manifest.evidence_sha256().clone(),
        rollback_target_sha256: manifest.rollback_target_sha256().cloned(),
        previous_sha256: previous_journal.map(|value| value.canonical_sha256().clone()),
        canonical_sha256: digest('0'),
        occurred_at,
    };
    journal.canonical_sha256 = journal_digest(&journal);
    ActivationApplyCandidate {
        expected_generation,
        business_day: UtcMicrosRange {
            start: day_start,
            end: day_start + DAY_LENGTH,
        },
        manifest,
        journal,
    }
}

fn initial_candidate(unit_id: &str, day_start: u64, occurred_at: u64) -> ActivationApplyCandidate {
    candidate(
        unit_id,
        DesiredActivationState::Disabled,
        PromotionAction::Initialize,
        "owner-legacy",
        None,
        None,
        None,
        day_start,
        occurred_at,
    )
}

fn shadow_chain(
    unit_id: &str,
    day_start: u64,
    offset: u64,
) -> (ActivationApplyCandidate, ActivationApplyCandidate) {
    let initial = initial_candidate(unit_id, day_start, day_start + offset);
    let shadow = candidate(
        unit_id,
        DesiredActivationState::Shadow,
        PromotionAction::EnterShadow,
        "owner-legacy",
        Some(&initial.manifest),
        Some(&initial.journal),
        None,
        day_start,
        day_start + offset + 100,
    );
    (initial, shadow)
}

fn active_chain(
    unit_id: &str,
    day_start: u64,
    offset: u64,
) -> (
    ActivationApplyCandidate,
    ActivationApplyCandidate,
    ActivationApplyCandidate,
) {
    let (initial, shadow) = shadow_chain(unit_id, day_start, offset);
    let active = candidate(
        unit_id,
        DesiredActivationState::Active,
        PromotionAction::Activate,
        "owner-new",
        Some(&shadow.manifest),
        Some(&shadow.journal),
        None,
        day_start,
        day_start + offset + 200,
    );
    (initial, shadow, active)
}

#[derive(Debug)]
struct HarnessCoordinator {
    times: VecDeque<u64>,
    pause_result: Result<(), ActivationOwnerCoordinationError>,
    validation_calls: Arc<AtomicUsize>,
    pause_calls: Arc<AtomicUsize>,
}

impl HarnessCoordinator {
    fn allowing(now: u64) -> Self {
        Self {
            times: VecDeque::from([now, now]),
            pause_result: Ok(()),
            validation_calls: Arc::new(AtomicUsize::new(0)),
            pause_calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn call_counters(&self) -> (Arc<AtomicUsize>, Arc<AtomicUsize>) {
        (self.validation_calls.clone(), self.pause_calls.clone())
    }
}

impl ActivationOwnerCoordinator for HarnessCoordinator {
    fn revalidate_approval_and_time(
        &mut self,
        _candidate: &ActivationApplyCandidate,
    ) -> Result<u64, ActivationOwnerCoordinationError> {
        self.validation_calls.fetch_add(1, Ordering::SeqCst);
        self.times
            .pop_front()
            .ok_or(ActivationOwnerCoordinationError::TimeUnavailable)
    }

    fn confirm_target_owner_paused(
        &mut self,
        _candidate: &ActivationApplyCandidate,
    ) -> Result<(), ActivationOwnerCoordinationError> {
        self.pause_calls.fetch_add(1, Ordering::SeqCst);
        self.pause_result
    }
}

fn apply(database: &Path, candidate: &ActivationApplyCandidate) -> ActivationApplyOutcome {
    let mut connection = open_test_connection(database);
    let mut coordinator = HarnessCoordinator::allowing(candidate.journal.occurred_at());
    apply_activation_candidate(&mut connection, candidate, &mut coordinator)
        .expect("TEST_CODE activation apply")
}

fn assert_empty(database: &Path) {
    let connection = open_test_connection(database);
    let manifests: i64 = connection
        .query_row(
            "SELECT count(*) FROM push_activation_manifests",
            [],
            |row| row.get(0),
        )
        .expect("TEST_CODE manifest count");
    let journal: i64 = connection
        .query_row("SELECT count(*) FROM push_promotion_journal", [], |row| {
            row.get(0)
        })
        .expect("TEST_CODE journal count");
    assert_eq!((manifests, journal), (0, 0));
}

#[test]
fn commits_both_rows_and_public_reader_recomputes_the_chain() {
    let (_root, database) = initialized_database("atomic.sqlite");
    let candidate = initial_candidate(UNIT_A, DAY, DAY + 100);

    assert_eq!(
        apply(&database, &candidate),
        ActivationApplyOutcome::Applied
    );

    let facts = inspect_raw_activation_facts(&database, candidate.manifest.unit_id())
        .expect("TEST_CODE independent public inspection");
    assert_eq!(
        facts.selected_unit().reconciliation(),
        ActivationReconciliation::CaughtUp { generation: 1 }
    );
    assert_eq!(facts.selected_unit().manifests(), &[candidate.manifest]);
    assert_eq!(facts.selected_unit().journal(), &[candidate.journal]);
}

#[test]
fn exact_recheck_is_persistence_only_and_changed_command_is_rejected_without_owner_calls() {
    let (_root, database) = initialized_database("recheck.sqlite");
    let candidate = initial_candidate(UNIT_A, DAY, DAY + 100);
    assert_eq!(
        apply(&database, &candidate),
        ActivationApplyOutcome::Applied
    );

    let mut connection = open_test_connection(&database);
    let mut coordinator = HarnessCoordinator::allowing(candidate.journal.occurred_at());
    let counters = coordinator.call_counters();
    assert_eq!(
        apply_activation_candidate(&mut connection, &candidate, &mut coordinator),
        Ok(ActivationApplyOutcome::AlreadyRecorded)
    );
    assert_eq!(counters.0.load(Ordering::SeqCst), 0);
    assert_eq!(counters.1.load(Ordering::SeqCst), 0);

    let mut changed = candidate.clone();
    changed.manifest.approved_by = "operator-b".to_owned();
    changed.manifest.manifest_sha256 = manifest_digest(&changed.manifest);
    changed.journal.actor = "operator-b".to_owned();
    changed.journal.to_manifest_sha256 = changed.manifest.manifest_sha256().clone();
    changed.journal.canonical_sha256 = journal_digest(&changed.journal);
    assert_eq!(
        apply_activation_candidate(&mut connection, &changed, &mut coordinator),
        Err(ActivationTransactionError::CommandConflict)
    );
    assert_eq!(counters.0.load(Ordering::SeqCst), 0);
    assert_eq!(counters.1.load(Ordering::SeqCst), 0);
}

#[test]
fn pending_manifest_in_any_unit_blocks_apply_before_owner_coordination() {
    let (_root, database) = initialized_database("pending.sqlite");
    let pending = initial_candidate(UNIT_B, DAY, DAY + 100);
    let connection = open_test_connection(&database);
    super::activation_transaction::insert_manifest_for_test(&connection, &pending.manifest)
        .expect("TEST_CODE pending manifest");

    let candidate = initial_candidate(UNIT_A, DAY, DAY + 200);
    let mut connection = open_test_connection(&database);
    let mut coordinator = HarnessCoordinator::allowing(candidate.journal.occurred_at());
    let counters = coordinator.call_counters();
    assert_eq!(
        apply_activation_candidate(&mut connection, &candidate, &mut coordinator),
        Err(ActivationTransactionError::PendingHistory)
    );
    assert_eq!(counters.0.load(Ordering::SeqCst), 0);
    assert_eq!(counters.1.load(Ordering::SeqCst), 0);
}

#[test]
fn schema_history_and_generation_failures_never_reach_owner_coordination() {
    let (_root, schema_database) = initialized_database("schema-rejected.sqlite");
    open_test_connection(&schema_database)
        .execute_batch("DROP TRIGGER push_activation_manifests_delete;")
        .expect("TEST_CODE corrupt registered schema");
    let candidate = initial_candidate(UNIT_A, DAY, DAY + 100);
    let mut connection = open_test_connection(&schema_database);
    let mut coordinator = HarnessCoordinator::allowing(candidate.journal.occurred_at());
    let counters = coordinator.call_counters();
    assert_eq!(
        apply_activation_candidate(&mut connection, &candidate, &mut coordinator),
        Err(ActivationTransactionError::SchemaRejected)
    );
    assert_eq!(counters.0.load(Ordering::SeqCst), 0);
    assert_eq!(counters.1.load(Ordering::SeqCst), 0);

    let (_root, history_database) = initialized_database("history-rejected.sqlite");
    let mut invalid_history = initial_candidate(UNIT_B, DAY, DAY + 100);
    invalid_history.manifest.manifest_sha256 = digest('9');
    super::activation_transaction::insert_manifest_for_test(
        &open_test_connection(&history_database),
        &invalid_history.manifest,
    )
    .expect("TEST_CODE invalid persisted digest");
    let mut connection = open_test_connection(&history_database);
    let mut coordinator = HarnessCoordinator::allowing(candidate.journal.occurred_at());
    let counters = coordinator.call_counters();
    assert_eq!(
        apply_activation_candidate(&mut connection, &candidate, &mut coordinator),
        Err(ActivationTransactionError::InvalidHistory)
    );
    assert_eq!(counters.0.load(Ordering::SeqCst), 0);
    assert_eq!(counters.1.load(Ordering::SeqCst), 0);

    let (_root, generation_database) = initialized_database("generation-conflict.sqlite");
    let mut ahead = candidate;
    ahead.expected_generation = 5;
    ahead.manifest.generation = 6;
    ahead.manifest.manifest_sha256 = manifest_digest(&ahead.manifest);
    ahead.journal.generation = 6;
    ahead.journal.event_id = promotion_event_id(UNIT_A, 6);
    ahead.journal.to_manifest_sha256 = ahead.manifest.manifest_sha256().clone();
    ahead.journal.canonical_sha256 = journal_digest(&ahead.journal);
    let mut connection = open_test_connection(&generation_database);
    let mut coordinator = HarnessCoordinator::allowing(ahead.journal.occurred_at());
    let counters = coordinator.call_counters();
    assert_eq!(
        apply_activation_candidate(&mut connection, &ahead, &mut coordinator),
        Err(ActivationTransactionError::GenerationConflict)
    );
    assert_eq!(counters.0.load(Ordering::SeqCst), 0);
    assert_eq!(counters.1.load(Ordering::SeqCst), 0);
}

#[test]
fn owner_preserving_edges_are_checked_before_owner_coordination() {
    let (_root, database) = initialized_database("owner-preservation.sqlite");
    let initial = initial_candidate(UNIT_A, DAY, DAY + 100);
    assert_eq!(apply(&database, &initial), ActivationApplyOutcome::Applied);

    let changed_owner = candidate(
        UNIT_A,
        DesiredActivationState::Shadow,
        PromotionAction::EnterShadow,
        "owner-new",
        Some(&initial.manifest),
        Some(&initial.journal),
        None,
        DAY,
        DAY + 200,
    );
    let mut connection = open_test_connection(&database);
    let mut coordinator = HarnessCoordinator::allowing(changed_owner.journal.occurred_at());
    let counters = coordinator.call_counters();
    assert_eq!(
        apply_activation_candidate(&mut connection, &changed_owner, &mut coordinator),
        Err(ActivationTransactionError::CandidateRejected)
    );
    assert_eq!(counters.0.load(Ordering::SeqCst), 0);
    assert_eq!(counters.1.load(Ordering::SeqCst), 0);
}

#[test]
fn rehashed_invalid_scalar_fields_are_rejected_before_owner_coordination() {
    let (_root, database) = initialized_database("invalid-scalars.sqlite");
    let original = initial_candidate(UNIT_A, DAY, DAY + 100);
    let mut invalid_reason = original.clone();
    invalid_reason.journal.reason = "activation.changed".to_owned();
    invalid_reason.journal.canonical_sha256 = journal_digest(&invalid_reason.journal);
    let mut invalid_owner = original;
    invalid_owner.manifest.physical_owner = "owner\0hidden".to_owned();
    invalid_owner.manifest.manifest_sha256 = manifest_digest(&invalid_owner.manifest);
    invalid_owner.journal.to_manifest_sha256 = invalid_owner.manifest.manifest_sha256().clone();
    invalid_owner.journal.canonical_sha256 = journal_digest(&invalid_owner.journal);
    for invalid in [invalid_reason, invalid_owner] {
        let mut connection = open_test_connection(&database);
        let mut coordinator = HarnessCoordinator::allowing(invalid.journal.occurred_at());
        let counters = coordinator.call_counters();
        assert_eq!(
            apply_activation_candidate(&mut connection, &invalid, &mut coordinator),
            Err(ActivationTransactionError::CandidateRejected)
        );
        assert_eq!(counters.0.load(Ordering::SeqCst), 0);
        assert_eq!(counters.1.load(Ordering::SeqCst), 0);
    }
    assert_empty(&database);
}

#[test]
fn cross_unit_activate_quota_counts_activate_even_when_owner_text_is_unchanged() {
    let (_root, database) = initialized_database("quota.sqlite");
    let a0 = initial_candidate(UNIT_A, DAY, DAY + 100);
    let b0 = initial_candidate(UNIT_B, DAY, DAY + 110);
    apply(&database, &a0);
    apply(&database, &b0);
    let a1 = candidate(
        UNIT_A,
        DesiredActivationState::Shadow,
        PromotionAction::EnterShadow,
        "owner-legacy",
        Some(&a0.manifest),
        Some(&a0.journal),
        None,
        DAY,
        DAY + 200,
    );
    let b1 = candidate(
        UNIT_B,
        DesiredActivationState::Shadow,
        PromotionAction::EnterShadow,
        "owner-legacy",
        Some(&b0.manifest),
        Some(&b0.journal),
        None,
        DAY,
        DAY + 210,
    );
    apply(&database, &a1);
    apply(&database, &b1);
    let a2 = candidate(
        UNIT_A,
        DesiredActivationState::Active,
        PromotionAction::Activate,
        "owner-legacy",
        Some(&a1.manifest),
        Some(&a1.journal),
        None,
        DAY,
        DAY + 300,
    );
    assert_eq!(apply(&database, &a2), ActivationApplyOutcome::Applied);

    let b2 = candidate(
        UNIT_B,
        DesiredActivationState::Active,
        PromotionAction::Activate,
        "owner-other",
        Some(&b1.manifest),
        Some(&b1.journal),
        None,
        DAY,
        DAY + 310,
    );
    let mut connection = open_test_connection(&database);
    let mut coordinator = HarnessCoordinator::allowing(b2.journal.occurred_at());
    let counters = coordinator.call_counters();
    assert_eq!(
        apply_activation_candidate(&mut connection, &b2, &mut coordinator),
        Err(ActivationTransactionError::DailyPromotionQuotaExceeded)
    );
    assert_eq!(counters.0.load(Ordering::SeqCst), 0);
    assert_eq!(counters.1.load(Ordering::SeqCst), 0);
}

#[test]
fn rollback_is_unlimited_but_blocks_a_later_activate_in_its_business_day() {
    let (_root, database) = initialized_database("rollback-quota.sqlite");
    let a0 = initial_candidate(UNIT_A, DAY, DAY + 100);
    apply(&database, &a0);
    let a1 = candidate(
        UNIT_A,
        DesiredActivationState::Shadow,
        PromotionAction::EnterShadow,
        "owner-legacy",
        Some(&a0.manifest),
        Some(&a0.journal),
        None,
        DAY,
        DAY + 200,
    );
    apply(&database, &a1);
    let a2 = candidate(
        UNIT_A,
        DesiredActivationState::Active,
        PromotionAction::Activate,
        "owner-new",
        Some(&a1.manifest),
        Some(&a1.journal),
        None,
        DAY,
        DAY + 300,
    );
    apply(&database, &a2);

    let next_day = DAY + DAY_LENGTH;
    let rollback = candidate(
        UNIT_A,
        DesiredActivationState::Shadow,
        PromotionAction::Rollback,
        "owner-legacy",
        Some(&a2.manifest),
        Some(&a2.journal),
        Some(&a1.manifest),
        next_day,
        next_day + 100,
    );
    assert_eq!(apply(&database, &rollback), ActivationApplyOutcome::Applied);

    let b0 = initial_candidate(UNIT_B, next_day, next_day + 200);
    apply(&database, &b0);
    let b1 = candidate(
        UNIT_B,
        DesiredActivationState::Shadow,
        PromotionAction::EnterShadow,
        "owner-legacy",
        Some(&b0.manifest),
        Some(&b0.journal),
        None,
        next_day,
        next_day + 300,
    );
    apply(&database, &b1);
    let b2 = candidate(
        UNIT_B,
        DesiredActivationState::Active,
        PromotionAction::Activate,
        "owner-new",
        Some(&b1.manifest),
        Some(&b1.journal),
        None,
        next_day,
        next_day + 400,
    );
    let mut connection = open_test_connection(&database);
    let mut coordinator = HarnessCoordinator::allowing(b2.journal.occurred_at());
    assert_eq!(
        apply_activation_candidate(&mut connection, &b2, &mut coordinator),
        Err(ActivationTransactionError::DailyPromotionQuotaExceeded)
    );
}

#[test]
fn utc_business_day_quota_uses_half_open_endpoints() {
    let (_root, database) = initialized_database("quota-endpoints.sqlite");
    let (a0, a1) = shadow_chain(UNIT_A, DAY, 100);
    let (b0, b1) = shadow_chain(UNIT_B, DAY, 110);
    for candidate in [&a0, &a1, &b0, &b1] {
        apply(&database, candidate);
    }
    let end_of_day = candidate(
        UNIT_A,
        DesiredActivationState::Active,
        PromotionAction::Activate,
        "owner-new",
        Some(&a1.manifest),
        Some(&a1.journal),
        None,
        DAY,
        DAY + DAY_LENGTH - 1,
    );
    assert_eq!(
        apply(&database, &end_of_day),
        ActivationApplyOutcome::Applied
    );

    let next_day = DAY + DAY_LENGTH;
    let start_of_next_day = candidate(
        UNIT_B,
        DesiredActivationState::Active,
        PromotionAction::Activate,
        "owner-new",
        Some(&b1.manifest),
        Some(&b1.journal),
        None,
        next_day,
        next_day,
    );
    assert_eq!(
        apply(&database, &start_of_next_day),
        ActivationApplyOutcome::Applied
    );
}

#[test]
fn time_checks_reject_past_receipts_endpoints_and_expiry_before_commit() {
    let (_root, database) = initialized_database("time.sqlite");
    let mut candidate = initial_candidate(UNIT_A, DAY, DAY + 100);
    candidate.business_day.start = candidate.journal.occurred_at() + 1;
    let mut connection = open_test_connection(&database);
    let mut coordinator = HarnessCoordinator::allowing(candidate.journal.occurred_at());
    assert_eq!(
        apply_activation_candidate(&mut connection, &candidate, &mut coordinator),
        Err(ActivationTransactionError::CandidateRejected)
    );
    assert_empty(&database);

    let mut empty_day = initial_candidate(UNIT_A, DAY, DAY + 100);
    empty_day.business_day.end = empty_day.business_day.start;
    let mut coordinator = HarnessCoordinator::allowing(empty_day.journal.occurred_at());
    assert_eq!(
        apply_activation_candidate(&mut connection, &empty_day, &mut coordinator),
        Err(ActivationTransactionError::CandidateRejected)
    );
    assert_empty(&database);

    let mut overflowing_day = initial_candidate(UNIT_A, DAY, DAY + 100);
    overflowing_day.business_day.end = i64::MAX as u64 + 1;
    let mut coordinator = HarnessCoordinator::allowing(overflowing_day.journal.occurred_at());
    assert_eq!(
        apply_activation_candidate(&mut connection, &overflowing_day, &mut coordinator),
        Err(ActivationTransactionError::CandidateRejected)
    );
    assert_empty(&database);

    let mut end_endpoint = initial_candidate(UNIT_A, DAY, DAY + 100);
    end_endpoint.business_day.end = end_endpoint.journal.occurred_at();
    let mut coordinator = HarnessCoordinator::allowing(end_endpoint.journal.occurred_at());
    assert_eq!(
        apply_activation_candidate(&mut connection, &end_endpoint, &mut coordinator),
        Err(ActivationTransactionError::CandidateRejected)
    );
    assert_empty(&database);

    let candidate = initial_candidate(UNIT_A, DAY, DAY + 100);
    let mut coordinator = HarnessCoordinator::allowing(candidate.manifest.window_end());
    assert_eq!(
        apply_activation_candidate(&mut connection, &candidate, &mut coordinator),
        Err(ActivationTransactionError::OwnerCoordination(
            ActivationOwnerCoordinationError::ApprovalRejected
        ))
    );
    assert_empty(&database);

    let mut future_created = candidate.clone();
    future_created.manifest.created_at = future_created.journal.occurred_at() + 1;
    future_created.manifest.manifest_sha256 = manifest_digest(&future_created.manifest);
    future_created.journal.to_manifest_sha256 = future_created.manifest.manifest_sha256().clone();
    future_created.journal.canonical_sha256 = journal_digest(&future_created.journal);
    let mut coordinator = HarnessCoordinator::allowing(future_created.journal.occurred_at());
    assert_eq!(
        apply_activation_candidate(&mut connection, &future_created, &mut coordinator),
        Err(ActivationTransactionError::OwnerCoordination(
            ActivationOwnerCoordinationError::ApprovalRejected
        ))
    );
    assert_eq!(coordinator.pause_calls.load(Ordering::SeqCst), 0);
    assert_empty(&database);

    let mut coordinator = HarnessCoordinator::allowing(candidate.journal.occurred_at() - 1);
    assert_eq!(
        apply_activation_candidate(&mut connection, &candidate, &mut coordinator),
        Err(ActivationTransactionError::OwnerCoordination(
            ActivationOwnerCoordinationError::ApprovalRejected
        ))
    );
    assert_eq!(coordinator.pause_calls.load(Ordering::SeqCst), 0);
    assert_empty(&database);

    let mut coordinator = HarnessCoordinator {
        times: VecDeque::from([
            candidate.journal.occurred_at(),
            candidate.manifest.window_end(),
        ]),
        pause_result: Ok(()),
        validation_calls: Arc::new(AtomicUsize::new(0)),
        pause_calls: Arc::new(AtomicUsize::new(0)),
    };
    assert_eq!(
        apply_activation_candidate(&mut connection, &candidate, &mut coordinator),
        Err(ActivationTransactionError::OwnerCoordination(
            ActivationOwnerCoordinationError::ApprovalRejected
        ))
    );
    assert_empty(&database);

    let mut coordinator = HarnessCoordinator {
        times: VecDeque::from([
            candidate.journal.occurred_at() + 2,
            candidate.journal.occurred_at() + 1,
        ]),
        pause_result: Ok(()),
        validation_calls: Arc::new(AtomicUsize::new(0)),
        pause_calls: Arc::new(AtomicUsize::new(0)),
    };
    assert_eq!(
        apply_activation_candidate(&mut connection, &candidate, &mut coordinator),
        Err(ActivationTransactionError::OwnerCoordination(
            ActivationOwnerCoordinationError::ApprovalRejected
        ))
    );
    assert_empty(&database);
}

#[test]
fn every_in_transaction_failure_rolls_back_without_half_history() {
    for (name, fault) in [
        (
            "after-manifest.sqlite",
            ActivationTransactionFault::AfterManifest,
        ),
        (
            "after-journal.sqlite",
            ActivationTransactionFault::AfterJournal,
        ),
    ] {
        let (_root, database) = initialized_database(name);
        let candidate = initial_candidate(UNIT_A, DAY, DAY + 100);
        let mut connection = open_test_connection(&database);
        let mut coordinator = HarnessCoordinator::allowing(candidate.journal.occurred_at());
        assert_eq!(
            apply_activation_candidate_with_fault(
                &mut connection,
                &candidate,
                &mut coordinator,
                fault,
            ),
            Err(ActivationTransactionError::StorageFailure)
        );
        assert_empty(&database);
    }

    let (_root, database) = initialized_database("owner-failure.sqlite");
    let candidate = initial_candidate(UNIT_A, DAY, DAY + 100);
    let mut connection = open_test_connection(&database);
    let mut coordinator = HarnessCoordinator::allowing(candidate.journal.occurred_at());
    coordinator.pause_result = Err(ActivationOwnerCoordinationError::PauseUncertain);
    assert_eq!(
        apply_activation_candidate(&mut connection, &candidate, &mut coordinator),
        Err(ActivationTransactionError::OwnerCoordination(
            ActivationOwnerCoordinationError::PauseUncertain
        ))
    );
    assert_empty(&database);
}

#[test]
fn database_lock_never_falls_back_to_an_unlocked_write_path() {
    let (_root, database) = initialized_database("locked.sqlite");
    let mut blocker = open_test_connection(&database);
    let blocker_transaction = blocker
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .expect("TEST_CODE blocker transaction");

    let candidate = initial_candidate(UNIT_A, DAY, DAY + 100);
    let mut contender = open_test_connection(&database);
    contender
        .busy_timeout(Duration::ZERO)
        .expect("TEST_CODE zero timeout");
    let mut coordinator = HarnessCoordinator::allowing(candidate.journal.occurred_at());
    let counters = coordinator.call_counters();
    assert_eq!(
        apply_activation_candidate(&mut contender, &candidate, &mut coordinator),
        Err(ActivationTransactionError::DatabaseLocked)
    );
    assert_eq!(counters.0.load(Ordering::SeqCst), 0);
    assert_eq!(counters.1.load(Ordering::SeqCst), 0);
    blocker_transaction
        .rollback()
        .expect("TEST_CODE release blocker");
    assert_empty(&database);
}

#[test]
fn real_commit_busy_is_uncertain_without_retry_and_leaves_no_half_history() {
    let (_root, database) = initialized_database("commit-busy.sqlite");
    let mut reader = open_test_connection(&database);
    let reader_transaction = reader
        .transaction_with_behavior(TransactionBehavior::Deferred)
        .expect("TEST_CODE shared reader transaction");
    let initial_count: i64 = reader_transaction
        .query_row(
            "SELECT count(*) FROM push_activation_manifests",
            [],
            |row| row.get(0),
        )
        .expect("TEST_CODE acquire shared read lock");
    assert_eq!(initial_count, 0);

    let candidate = initial_candidate(UNIT_A, DAY, DAY + 100);
    let mut writer = open_test_connection(&database);
    writer
        .busy_timeout(Duration::ZERO)
        .expect("TEST_CODE immediate commit failure");
    let mut coordinator = HarnessCoordinator::allowing(candidate.journal.occurred_at());
    let counters = coordinator.call_counters();
    assert_eq!(
        apply_activation_candidate(&mut writer, &candidate, &mut coordinator),
        Err(ActivationTransactionError::CommitUncertain)
    );
    assert_eq!(counters.0.load(Ordering::SeqCst), 2);
    assert_eq!(counters.1.load(Ordering::SeqCst), 1);

    drop(writer);
    reader_transaction
        .rollback()
        .expect("TEST_CODE release shared reader lock");
    drop(reader);

    let facts = inspect_raw_activation_facts(&database, candidate.manifest.unit_id())
        .expect("TEST_CODE inspect after real failed commit");
    assert_eq!(
        facts.selected_unit().reconciliation(),
        ActivationReconciliation::Unregistered
    );

    assert_eq!(
        apply(&database, &candidate),
        ActivationApplyOutcome::Applied
    );
    let facts = inspect_raw_activation_facts(&database, candidate.manifest.unit_id())
        .expect("TEST_CODE inspect explicit later apply");
    assert_eq!(
        facts.selected_unit().reconciliation(),
        ActivationReconciliation::CaughtUp { generation: 1 }
    );
}

#[test]
fn independent_connections_racing_the_same_generation_commit_at_most_once() {
    let (_root, database) = initialized_database("threads.sqlite");
    let candidate = initial_candidate(UNIT_A, DAY, DAY + 100);
    let barrier = Arc::new(Barrier::new(2));
    let mut handles = Vec::new();
    for _ in 0..2 {
        let database = database.clone();
        let candidate = candidate.clone();
        let barrier = barrier.clone();
        handles.push(thread::spawn(move || {
            let mut connection = open_test_connection(&database);
            let mut coordinator = HarnessCoordinator::allowing(candidate.journal.occurred_at());
            barrier.wait();
            apply_activation_candidate(&mut connection, &candidate, &mut coordinator)
        }));
    }
    let outcomes: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().expect("TEST_CODE contender thread"))
        .collect();
    assert_eq!(
        outcomes
            .iter()
            .filter(|result| **result == Ok(ActivationApplyOutcome::Applied))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|result| **result == Ok(ActivationApplyOutcome::AlreadyRecorded))
            .count(),
        1
    );
}

#[test]
fn independent_processes_race_the_same_sqlite_generation() {
    let (_root, database) = initialized_database("processes.sqlite");
    let start = database.with_extension("start");
    let first_result = database.with_extension("first.result");
    let second_result = database.with_extension("second.result");
    let mut first = spawn_competitor(&database, &start, &first_result, "same-unit", "first");
    let mut second = spawn_competitor(&database, &start, &second_result, "same-unit", "second");
    thread::sleep(Duration::from_millis(100));
    fs::write(&start, b"go").expect("TEST_CODE process barrier");
    assert!(first.wait().expect("TEST_CODE first child").success());
    assert!(second.wait().expect("TEST_CODE second child").success());

    let outcomes = [first_result, second_result]
        .map(|path| fs::read_to_string(path).expect("TEST_CODE child process result"));
    assert_eq!(
        outcomes
            .iter()
            .filter(|value| value.as_str() == "applied")
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|value| value.as_str() == "already-recorded")
            .count(),
        1
    );
}

#[test]
fn independent_processes_race_the_cross_unit_daily_quota() {
    let (_root, database) = initialized_database("cross-unit-processes.sqlite");
    let (a0, a1) = shadow_chain(UNIT_A, DAY, 100);
    let (b0, b1) = shadow_chain(UNIT_B, DAY, 110);
    for candidate in [&a0, &a1, &b0, &b1] {
        apply(&database, candidate);
    }

    let start = database.with_extension("start");
    let first_result = database.with_extension("first.result");
    let second_result = database.with_extension("second.result");
    let mut first = spawn_competitor(&database, &start, &first_result, "cross-unit", "a");
    let mut second = spawn_competitor(&database, &start, &second_result, "cross-unit", "b");
    thread::sleep(Duration::from_millis(100));
    fs::write(&start, b"go").expect("TEST_CODE process barrier");
    assert!(first.wait().expect("TEST_CODE first child").success());
    assert!(second.wait().expect("TEST_CODE second child").success());

    let outcomes = read_process_outcomes(first_result, second_result);
    assert_eq!(
        outcomes
            .iter()
            .filter(|value| value.as_str() == "applied")
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|value| value.as_str() == "quota")
            .count(),
        1
    );
}

#[test]
fn rollback_holding_the_process_lock_blocks_the_waiting_activate() {
    let (_root, database) = initialized_database("rollback-processes.sqlite");
    let (a0, a1, a2) = active_chain(UNIT_A, DAY, 100);
    for candidate in [&a0, &a1, &a2] {
        apply(&database, candidate);
    }
    let next_day = DAY + DAY_LENGTH;
    let (b0, b1) = shadow_chain(UNIT_B, next_day, 100);
    apply(&database, &b0);
    apply(&database, &b1);

    let start = database.with_extension("start");
    let locked = database.with_extension("locked");
    let release = database.with_extension("release");
    let rollback_result = database.with_extension("rollback.result");
    let activate_result = database.with_extension("activate.result");
    fs::write(&start, b"go").expect("TEST_CODE start rollback");
    let mut rollback = spawn_competitor_with_lock(
        &database,
        &start,
        &rollback_result,
        "rollback-activate",
        "rollback",
        &locked,
        &release,
    );
    wait_for_file(&locked);
    let mut activate = spawn_competitor(
        &database,
        &start,
        &activate_result,
        "rollback-activate",
        "activate",
    );
    thread::sleep(Duration::from_millis(100));
    fs::write(&release, b"commit").expect("TEST_CODE release rollback");
    assert!(rollback.wait().expect("TEST_CODE rollback child").success());
    assert!(activate.wait().expect("TEST_CODE activate child").success());
    assert_eq!(
        fs::read_to_string(rollback_result).expect("TEST_CODE rollback result"),
        "applied"
    );
    assert_eq!(
        fs::read_to_string(activate_result).expect("TEST_CODE activate result"),
        "quota"
    );
}

fn spawn_competitor(
    database: &Path,
    start: &Path,
    result: &Path,
    scenario: &str,
    role: &str,
) -> std::process::Child {
    let executable = std::env::current_exe().expect("TEST_CODE current test executable");
    Command::new(executable)
        .arg("--exact")
        .arg(
            "push_foundation::activation_transaction_tests::\
             activation_process_competitor_helper",
        )
        .arg("--ignored")
        .arg("--nocapture")
        .env("ACTIVATION_PROCESS_DATABASE", database)
        .env("ACTIVATION_PROCESS_START", start)
        .env("ACTIVATION_PROCESS_RESULT", result)
        .env("ACTIVATION_PROCESS_SCENARIO", scenario)
        .env("ACTIVATION_PROCESS_ROLE", role)
        .spawn()
        .expect("TEST_CODE child process")
}

#[allow(clippy::too_many_arguments)]
fn spawn_competitor_with_lock(
    database: &Path,
    start: &Path,
    result: &Path,
    scenario: &str,
    role: &str,
    locked: &Path,
    release: &Path,
) -> std::process::Child {
    let executable = std::env::current_exe().expect("TEST_CODE current test executable");
    Command::new(executable)
        .arg("--exact")
        .arg(
            "push_foundation::activation_transaction_tests::\
             activation_process_competitor_helper",
        )
        .arg("--ignored")
        .arg("--nocapture")
        .env("ACTIVATION_PROCESS_DATABASE", database)
        .env("ACTIVATION_PROCESS_START", start)
        .env("ACTIVATION_PROCESS_RESULT", result)
        .env("ACTIVATION_PROCESS_SCENARIO", scenario)
        .env("ACTIVATION_PROCESS_ROLE", role)
        .env("ACTIVATION_PROCESS_LOCKED", locked)
        .env("ACTIVATION_PROCESS_RELEASE", release)
        .spawn()
        .expect("TEST_CODE locked child process")
}

fn read_process_outcomes(first: PathBuf, second: PathBuf) -> [String; 2] {
    [first, second].map(|path| fs::read_to_string(path).expect("TEST_CODE child process result"))
}

fn wait_for_file(path: &Path) {
    for _ in 0..1_000 {
        if path.exists() {
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
    panic!("TEST_CODE child did not acquire SQLite lock");
}

struct ProcessCoordinator {
    inner: HarnessCoordinator,
    lock_barrier: Option<(PathBuf, PathBuf)>,
}

impl ActivationOwnerCoordinator for ProcessCoordinator {
    fn revalidate_approval_and_time(
        &mut self,
        candidate: &ActivationApplyCandidate,
    ) -> Result<u64, ActivationOwnerCoordinationError> {
        if let Some((locked, release)) = self.lock_barrier.take() {
            fs::write(locked, b"locked").expect("TEST_CODE locked marker");
            wait_for_file(&release);
        }
        self.inner.revalidate_approval_and_time(candidate)
    }

    fn confirm_target_owner_paused(
        &mut self,
        candidate: &ActivationApplyCandidate,
    ) -> Result<(), ActivationOwnerCoordinationError> {
        self.inner.confirm_target_owner_paused(candidate)
    }
}

fn process_candidate(scenario: &str, role: &str) -> ActivationApplyCandidate {
    match (scenario, role) {
        ("same-unit", "first" | "second") => initial_candidate(UNIT_A, DAY, DAY + 100),
        ("cross-unit", "a") => active_chain(UNIT_A, DAY, 100).2,
        ("cross-unit", "b") => active_chain(UNIT_B, DAY, 110).2,
        ("rollback-activate", "rollback") => {
            let (_, shadow, active) = active_chain(UNIT_A, DAY, 100);
            let next_day = DAY + DAY_LENGTH;
            candidate(
                UNIT_A,
                DesiredActivationState::Shadow,
                PromotionAction::Rollback,
                "owner-legacy",
                Some(&active.manifest),
                Some(&active.journal),
                Some(&shadow.manifest),
                next_day,
                next_day + 400,
            )
        }
        ("rollback-activate", "activate") => active_chain(UNIT_B, DAY + DAY_LENGTH, 100).2,
        _ => panic!("TEST_CODE unknown process scenario"),
    }
}

#[test]
#[ignore = "executed only by the parent process-competition tests"]
fn activation_process_competitor_helper() {
    let database =
        std::env::var("ACTIVATION_PROCESS_DATABASE").expect("TEST_CODE child database path");
    let start = PathBuf::from(
        std::env::var("ACTIVATION_PROCESS_START").expect("TEST_CODE child start path"),
    );
    let result = PathBuf::from(
        std::env::var("ACTIVATION_PROCESS_RESULT").expect("TEST_CODE child result path"),
    );
    wait_for_file(&start);

    let scenario = std::env::var("ACTIVATION_PROCESS_SCENARIO").expect("TEST_CODE child scenario");
    let role = std::env::var("ACTIVATION_PROCESS_ROLE").expect("TEST_CODE child role");
    let candidate = process_candidate(&scenario, &role);
    let lock_barrier = std::env::var("ACTIVATION_PROCESS_LOCKED")
        .ok()
        .zip(std::env::var("ACTIVATION_PROCESS_RELEASE").ok())
        .map(|(locked, release)| (PathBuf::from(locked), PathBuf::from(release)));
    let mut connection = open_test_connection(Path::new(&database));
    let mut coordinator = ProcessCoordinator {
        inner: HarnessCoordinator::allowing(candidate.journal.occurred_at()),
        lock_barrier,
    };
    let outcome = apply_activation_candidate(&mut connection, &candidate, &mut coordinator);
    let text = match outcome {
        Ok(ActivationApplyOutcome::Applied) => "applied",
        Ok(ActivationApplyOutcome::AlreadyRecorded) => "already-recorded",
        Err(ActivationTransactionError::DailyPromotionQuotaExceeded) => "quota",
        Err(_) => "unexpected-error",
    };
    fs::write(result, text).expect("TEST_CODE child result");
}
