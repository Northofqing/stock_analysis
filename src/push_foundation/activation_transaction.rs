//! Internal atomic activation-history append engine.
//!
//! This module authenticates neither its connection nor command inputs. The future application
//! coordinator must supply a writable connection that it authenticated and a candidate whose
//! authority was independently approved. The engine nevertheless rechecks the complete persisted
//! chain, CAS, quota, and time bounds while holding SQLite's write lock.

#![cfg_attr(not(test), allow(dead_code))]

use rusqlite::{
    params, Connection, Error as SqliteError, ErrorCode, Transaction, TransactionBehavior,
};

use super::activation::{ActivationManifest, PromotionAction, PromotionJournalEntry};
use super::activation_facts::{ActivationReconciliation, UnitActivationFacts};
use super::activation_owner::project_owner_admission;
use super::activation_store::{
    inspect_activation_transaction, validate_journal_chain, validate_journal_value,
    validate_manifest_chain, validate_manifest_value, ActivationInspectError,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct UtcMicrosRange {
    pub(super) start: u64,
    pub(super) end: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ActivationApplyCandidate {
    pub(super) expected_generation: u64,
    pub(super) business_day: UtcMicrosRange,
    pub(super) manifest: ActivationManifest,
    pub(super) journal: PromotionJournalEntry,
}

/// A narrow future adapter boundary. Implementations never receive a connection or transaction.
pub(super) trait ActivationOwnerCoordinator {
    /// Recheck durable approval and return a trustworthy current UTC microsecond timestamp.
    /// The engine calls this once after database validation and again immediately before commit.
    fn revalidate_approval_and_time(
        &mut self,
        candidate: &ActivationApplyCandidate,
    ) -> Result<u64, ActivationOwnerCoordinationError>;

    /// Confirm that the exact candidate's target owner is paused. Implementations must be bounded.
    fn confirm_target_owner_paused(
        &mut self,
        candidate: &ActivationApplyCandidate,
    ) -> Result<(), ActivationOwnerCoordinationError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(super) enum ActivationOwnerCoordinationError {
    #[error("activation approval is not currently valid")]
    ApprovalRejected,
    #[error("trusted activation time is unavailable")]
    TimeUnavailable,
    #[error("target owner pause could not be confirmed")]
    PauseUncertain,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ActivationApplyOutcome {
    /// Both append-only rows committed in this invocation.
    Applied,
    /// Exact manifest/journal rows were found by a later persistent recheck.
    ///
    /// The frozen tables do not retain an external command id or business-day identity. The future
    /// coordinator must separately match those approval-package fields; this outcome says nothing
    /// about that package, live owner state, or readiness.
    AlreadyRecorded,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(super) enum ActivationTransactionError {
    #[error("activation candidate is internally inconsistent")]
    CandidateRejected,
    #[error("activation schema attestation failed")]
    SchemaRejected,
    #[error("persisted activation history is invalid")]
    InvalidHistory,
    #[error("a unit has an unapplied activation manifest")]
    PendingHistory,
    #[error("activation expected generation no longer matches")]
    GenerationConflict,
    #[error("the same activation command identity has different fields")]
    CommandConflict,
    #[error("another activate or rollback already occupies this UTC business day")]
    DailyPromotionQuotaExceeded,
    #[error("activation owner coordination failed: {0}")]
    OwnerCoordination(ActivationOwnerCoordinationError),
    #[error("activation database is locked")]
    DatabaseLocked,
    #[error("activation database operation failed")]
    StorageFailure,
    #[error("activation commit acknowledgement is uncertain; recheck exact persisted rows")]
    CommitUncertain,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ActivationTransactionFault {
    None,
    AfterManifest,
    AfterJournal,
}

pub(super) fn apply_activation_candidate(
    connection: &mut Connection,
    candidate: &ActivationApplyCandidate,
    coordinator: &mut dyn ActivationOwnerCoordinator,
) -> Result<ActivationApplyOutcome, ActivationTransactionError> {
    apply_activation_candidate_inner(
        connection,
        candidate,
        coordinator,
        #[cfg(test)]
        ActivationTransactionFault::None,
    )
}

#[cfg(test)]
pub(super) fn apply_activation_candidate_with_fault(
    connection: &mut Connection,
    candidate: &ActivationApplyCandidate,
    coordinator: &mut dyn ActivationOwnerCoordinator,
    fault: ActivationTransactionFault,
) -> Result<ActivationApplyOutcome, ActivationTransactionError> {
    apply_activation_candidate_inner(connection, candidate, coordinator, fault)
}

fn apply_activation_candidate_inner(
    connection: &mut Connection,
    candidate: &ActivationApplyCandidate,
    coordinator: &mut dyn ActivationOwnerCoordinator,
    #[cfg(test)] fault: ActivationTransactionFault,
) -> Result<ActivationApplyOutcome, ActivationTransactionError> {
    validate_candidate_bounds(candidate)?;
    prepare_writable_connection(connection)?;

    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(map_sqlite)?;
    let result = apply_inside_transaction(
        &transaction,
        candidate,
        coordinator,
        #[cfg(test)]
        fault,
    );
    match result {
        Ok(ActivationApplyOutcome::AlreadyRecorded) => {
            transaction.rollback().map_err(map_sqlite)?;
            Ok(ActivationApplyOutcome::AlreadyRecorded)
        }
        Ok(ActivationApplyOutcome::Applied) => transaction
            .commit()
            .map(|()| ActivationApplyOutcome::Applied)
            .map_err(|_| ActivationTransactionError::CommitUncertain),
        Err(error) => {
            // Dropping an uncommitted rusqlite Transaction rolls it back. Preserve the original
            // semantic failure; a rollback error cannot make uncommitted rows authoritative.
            drop(transaction);
            Err(error)
        }
    }
}

fn apply_inside_transaction(
    transaction: &Transaction<'_>,
    candidate: &ActivationApplyCandidate,
    coordinator: &mut dyn ActivationOwnerCoordinator,
    #[cfg(test)] fault: ActivationTransactionFault,
) -> Result<ActivationApplyOutcome, ActivationTransactionError> {
    let facts = inspect_activation_transaction(transaction, candidate.manifest.unit_id())
        .map_err(map_inspection)?;

    // Schema attestation intentionally enabled query_only. Only this already locked, caller-opened
    // connection is restored to writable mode; no database is opened or selected here.
    transaction
        .execute_batch("PRAGMA query_only=OFF;")
        .map_err(map_sqlite)?;
    let query_only: i64 = transaction
        .query_row("PRAGMA query_only", [], |row| row.get(0))
        .map_err(map_sqlite)?;
    if query_only != 0 {
        return Err(ActivationTransactionError::StorageFailure);
    }

    if facts.units().iter().any(|unit| {
        matches!(
            unit.reconciliation(),
            ActivationReconciliation::Pending { .. }
        )
    }) {
        return Err(ActivationTransactionError::PendingHistory);
    }

    let selected = facts.selected_unit();
    for unit in facts
        .units()
        .iter()
        .filter(|unit| !unit.manifests().is_empty())
    {
        project_owner_admission(unit).map_err(|_| ActivationTransactionError::InvalidHistory)?;
    }

    let candidate_index = candidate
        .manifest
        .generation()
        .checked_sub(1)
        .and_then(|value| usize::try_from(value).ok());
    if let Some(existing_manifest) =
        candidate_index.and_then(|index| selected.manifests().get(index))
    {
        let existing_journal = selected
            .journal()
            .get(candidate_index.expect("candidate index was Some"));
        if existing_manifest == &candidate.manifest && existing_journal == Some(&candidate.journal)
        {
            return Ok(ActivationApplyOutcome::AlreadyRecorded);
        }
        return Err(ActivationTransactionError::CommandConflict);
    }

    let current_generation = selected
        .manifests()
        .last()
        .map_or(0, ActivationManifest::generation);
    if current_generation != candidate.expected_generation {
        return Err(ActivationTransactionError::GenerationConflict);
    }

    let projected = validate_candidate_chain(selected, candidate)?;
    project_owner_admission(&projected)
        .map_err(|_| ActivationTransactionError::CandidateRejected)?;
    enforce_preserved_owner(selected.manifests().last(), candidate)?;
    enforce_daily_quota(transaction, candidate)?;

    let locked_now = coordinator
        .revalidate_approval_and_time(candidate)
        .map_err(ActivationTransactionError::OwnerCoordination)?;
    validate_actual_time(candidate, locked_now)?;

    insert_manifest(transaction, &candidate.manifest)?;
    #[cfg(test)]
    if fault == ActivationTransactionFault::AfterManifest {
        return Err(ActivationTransactionError::StorageFailure);
    }

    coordinator
        .confirm_target_owner_paused(candidate)
        .map_err(ActivationTransactionError::OwnerCoordination)?;
    insert_journal(transaction, &candidate.journal)?;
    #[cfg(test)]
    if fault == ActivationTransactionFault::AfterJournal {
        return Err(ActivationTransactionError::StorageFailure);
    }

    // Reuse the reader's complete parser and chain validation after both writes. This is not a
    // compensating delete: any failure aborts the still-uncommitted transaction.
    let written = inspect_activation_transaction(transaction, candidate.manifest.unit_id())
        .map_err(map_inspection)?;
    let selected = written.selected_unit();
    if selected.manifests().last() != Some(&candidate.manifest)
        || selected.journal().last() != Some(&candidate.journal)
    {
        return Err(ActivationTransactionError::StorageFailure);
    }

    let commit_now = coordinator
        .revalidate_approval_and_time(candidate)
        .map_err(ActivationTransactionError::OwnerCoordination)?;
    validate_actual_time(candidate, commit_now)?;
    if commit_now < locked_now {
        return Err(ActivationTransactionError::OwnerCoordination(
            ActivationOwnerCoordinationError::ApprovalRejected,
        ));
    }
    Ok(ActivationApplyOutcome::Applied)
}

fn validate_candidate_bounds(
    candidate: &ActivationApplyCandidate,
) -> Result<(), ActivationTransactionError> {
    let expected = candidate
        .expected_generation
        .checked_add(1)
        .ok_or(ActivationTransactionError::CandidateRejected)?;
    if candidate.business_day.start >= candidate.business_day.end
        || candidate.manifest.generation() != expected
        || candidate.journal.generation() != expected
        || candidate.manifest.unit_id() != candidate.journal.unit_id()
        || candidate.journal.occurred_at() < candidate.business_day.start
        || candidate.journal.occurred_at() >= candidate.business_day.end
    {
        return Err(ActivationTransactionError::CandidateRejected);
    }
    validate_manifest_value(&candidate.manifest)
        .map_err(|_| ActivationTransactionError::CandidateRejected)?;
    validate_journal_value(&candidate.journal)
        .map_err(|_| ActivationTransactionError::CandidateRejected)?;
    for value in [
        candidate.business_day.start,
        candidate.business_day.end,
        candidate.manifest.generation(),
        candidate.manifest.approved_at(),
        candidate.manifest.window_start(),
        candidate.manifest.window_end(),
        candidate.manifest.created_at(),
        candidate.journal.generation(),
        candidate.journal.window_start(),
        candidate.journal.window_end(),
        candidate.journal.occurred_at(),
    ] {
        i64::try_from(value).map_err(|_| ActivationTransactionError::CandidateRejected)?;
    }
    Ok(())
}

fn validate_candidate_chain(
    persisted: &UnitActivationFacts,
    candidate: &ActivationApplyCandidate,
) -> Result<UnitActivationFacts, ActivationTransactionError> {
    let mut manifests = persisted.manifests().to_vec();
    manifests.push(candidate.manifest.clone());
    validate_manifest_chain(&manifests)
        .map_err(|_| ActivationTransactionError::CandidateRejected)?;

    let mut journal = persisted.journal().to_vec();
    journal.push(candidate.journal.clone());
    validate_journal_chain(&manifests, &journal)
        .map_err(|_| ActivationTransactionError::CandidateRejected)?;
    Ok(UnitActivationFacts {
        unit_id: persisted.unit_id().clone(),
        manifests,
        journal,
        reconciliation: ActivationReconciliation::CaughtUp {
            generation: candidate.manifest.generation(),
        },
    })
}

fn enforce_preserved_owner(
    predecessor: Option<&ActivationManifest>,
    candidate: &ActivationApplyCandidate,
) -> Result<(), ActivationTransactionError> {
    if matches!(
        candidate.journal.action(),
        PromotionAction::EnterShadow | PromotionAction::Drain | PromotionAction::Disable
    ) && predecessor.map(ActivationManifest::physical_owner)
        != Some(candidate.manifest.physical_owner())
    {
        return Err(ActivationTransactionError::CandidateRejected);
    }
    Ok(())
}

fn enforce_daily_quota(
    transaction: &Transaction<'_>,
    candidate: &ActivationApplyCandidate,
) -> Result<(), ActivationTransactionError> {
    if candidate.journal.action() != PromotionAction::Activate {
        return Ok(());
    }
    let count: i64 = transaction
        .query_row(
            "SELECT count(*) FROM push_promotion_journal \
             WHERE action IN ('Activate','Rollback') AND occurred_at>=?1 AND occurred_at<?2",
            params![
                to_i64(candidate.business_day.start)?,
                to_i64(candidate.business_day.end)?
            ],
            |row| row.get(0),
        )
        .map_err(map_sqlite)?;
    if count == 0 {
        Ok(())
    } else {
        Err(ActivationTransactionError::DailyPromotionQuotaExceeded)
    }
}

fn validate_actual_time(
    candidate: &ActivationApplyCandidate,
    now: u64,
) -> Result<(), ActivationTransactionError> {
    i64::try_from(now).map_err(|_| ActivationTransactionError::CandidateRejected)?;
    if now < candidate.manifest.approved_at()
        || candidate.manifest.created_at() > now
        || candidate.journal.occurred_at() > now
        || now < candidate.manifest.window_start()
        || now >= candidate.manifest.window_end()
        || now < candidate.business_day.start
        || now >= candidate.business_day.end
    {
        Err(ActivationTransactionError::OwnerCoordination(
            ActivationOwnerCoordinationError::ApprovalRejected,
        ))
    } else {
        Ok(())
    }
}

fn prepare_writable_connection(connection: &Connection) -> Result<(), ActivationTransactionError> {
    connection
        .execute_batch("PRAGMA foreign_keys=ON; PRAGMA query_only=OFF;")
        .map_err(map_sqlite)?;
    let foreign_keys: i64 = connection
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .map_err(map_sqlite)?;
    let query_only: i64 = connection
        .query_row("PRAGMA query_only", [], |row| row.get(0))
        .map_err(map_sqlite)?;
    if foreign_keys != 1 || query_only != 0 {
        Err(ActivationTransactionError::StorageFailure)
    } else {
        Ok(())
    }
}

fn insert_manifest(
    transaction: &Connection,
    manifest: &ActivationManifest,
) -> Result<(), ActivationTransactionError> {
    transaction
        .execute(
            "INSERT INTO push_activation_manifests(\
             manifest_sha256,unit_id,generation,previous_manifest_sha256,desired_state,\
             physical_owner,build_commit,build_sha256,catalog_sha256,business_schema_sha256,\
             durable_schema_sha256,template_sha256,source_contract_sha256,evidence_sha256,\
             approved_by,approved_at,window_start,window_end,rollback_target_sha256,created_at\
             ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20)",
            params![
                manifest.manifest_sha256().as_str(),
                manifest.unit_id().as_str(),
                to_i64(manifest.generation())?,
                manifest
                    .previous_manifest_sha256()
                    .map(|value| value.as_str()),
                manifest.desired_state().as_str(),
                manifest.physical_owner(),
                manifest.build_commit().as_str(),
                manifest.build_sha256().as_str(),
                manifest.catalog_sha256().as_str(),
                manifest.business_schema_sha256().as_str(),
                manifest.durable_schema_sha256().as_str(),
                manifest.template_sha256().as_str(),
                manifest.source_contract_sha256().as_str(),
                manifest.evidence_sha256().as_str(),
                manifest.approved_by(),
                to_i64(manifest.approved_at())?,
                to_i64(manifest.window_start())?,
                to_i64(manifest.window_end())?,
                manifest
                    .rollback_target_sha256()
                    .map(|value| value.as_str()),
                to_i64(manifest.created_at())?,
            ],
        )
        .map(|_| ())
        .map_err(map_sqlite)
}

#[cfg(test)]
pub(super) fn insert_manifest_for_test(
    connection: &Connection,
    manifest: &ActivationManifest,
) -> Result<(), ActivationTransactionError> {
    insert_manifest(connection, manifest)
}

fn insert_journal(
    transaction: &Transaction<'_>,
    journal: &PromotionJournalEntry,
) -> Result<(), ActivationTransactionError> {
    transaction
        .execute(
            "INSERT INTO push_promotion_journal(\
             event_id,unit_id,generation,from_manifest_sha256,to_manifest_sha256,actor,action,\
             reason,window_start,window_end,evidence_sha256,rollback_target_sha256,\
             previous_sha256,canonical_sha256,occurred_at\
             ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
            params![
                journal.event_id().as_str(),
                journal.unit_id().as_str(),
                to_i64(journal.generation())?,
                journal.from_manifest_sha256().map(|value| value.as_str()),
                journal.to_manifest_sha256().as_str(),
                journal.actor(),
                journal.action().as_str(),
                journal.reason(),
                to_i64(journal.window_start())?,
                to_i64(journal.window_end())?,
                journal.evidence_sha256().as_str(),
                journal.rollback_target_sha256().map(|value| value.as_str()),
                journal.previous_sha256().map(|value| value.as_str()),
                journal.canonical_sha256().as_str(),
                to_i64(journal.occurred_at())?,
            ],
        )
        .map(|_| ())
        .map_err(map_sqlite)
}

fn to_i64(value: u64) -> Result<i64, ActivationTransactionError> {
    i64::try_from(value).map_err(|_| ActivationTransactionError::CandidateRejected)
}

fn map_inspection(error: ActivationInspectError) -> ActivationTransactionError {
    match error {
        ActivationInspectError::SchemaRejected => ActivationTransactionError::SchemaRejected,
        ActivationInspectError::ReadOnlySourceRejected => {
            ActivationTransactionError::StorageFailure
        }
        ActivationInspectError::CatalogRejected
        | ActivationInspectError::UnknownRequestedUnit
        | ActivationInspectError::InvalidFacts => ActivationTransactionError::InvalidHistory,
    }
}

fn map_sqlite(error: SqliteError) -> ActivationTransactionError {
    match error.sqlite_error_code() {
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => {
            ActivationTransactionError::DatabaseLocked
        }
        _ => ActivationTransactionError::StorageFailure,
    }
}
