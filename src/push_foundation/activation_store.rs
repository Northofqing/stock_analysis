//! Side-effect-free inspection of complete activation manifest and journal chains.

#![cfg_attr(not(test), allow(dead_code))]

use std::collections::BTreeMap;
use std::path::Path;

use rusqlite::Transaction;

use crate::monitor::push_job::{GitSha40, MachineCatalog, Sha256Digest, UnitId};

use super::activation::{
    ActivationManifest, DesiredActivationState, PromotionAction, PromotionJournalEntry,
};
use super::activation_codec::{journal_digest, manifest_digest, promotion_event_id};
use super::activation_facts::{ActivationReconciliation, RawActivationFacts, UnitActivationFacts};
use super::migration::attest_bundled_connection;
use super::readiness_store_schema::{with_rollback_read_only, ReadinessSchemaError};

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ActivationInspectError {
    #[error("activation database cannot be read without side effects")]
    ReadOnlySourceRejected,
    #[error("activation database schema is not the bundled foundation schema")]
    SchemaRejected,
    #[error("bundled machine catalog is unavailable")]
    CatalogRejected,
    #[error("requested activation unit is not in the bundled catalog")]
    UnknownRequestedUnit,
    #[error("activation facts failed content-integrity validation")]
    InvalidFacts,
}

impl From<ReadinessSchemaError> for ActivationInspectError {
    fn from(_: ReadinessSchemaError) -> Self {
        Self::ReadOnlySourceRejected
    }
}

/// Inspect every bundled unit before returning the requested unit projection.
///
/// This function validates only persisted content and relationships. In particular, historical
/// build/catalog/schema hashes are retained as claims and are not compared with the running
/// deployment. The result does not grant legacy, shadow, new, or rollback authority.
pub fn inspect_raw_activation_facts(
    database: &Path,
    selected_unit: &UnitId,
) -> Result<RawActivationFacts, ActivationInspectError> {
    with_rollback_read_only(database, |transaction| {
        inspect_activation_transaction(transaction, selected_unit)
    })
}

/// Recompute the complete activation history using the caller's existing transaction.
///
/// This is the single parsing, schema-attestation, and chain-validation kernel shared by the
/// public rollback-only reader and the internal activation writer. Schema attestation deliberately
/// leaves the connection in `query_only` mode; a writer must explicitly restore its already
/// authenticated writable connection before attempting any mutation.
pub(super) fn inspect_activation_transaction(
    transaction: &Transaction<'_>,
    selected_unit: &UnitId,
) -> Result<RawActivationFacts, ActivationInspectError> {
    let catalog = MachineCatalog::bundled().map_err(|_| ActivationInspectError::CatalogRejected)?;
    attest_bundled_connection(transaction).map_err(|_| ActivationInspectError::SchemaRejected)?;
    validate_foreign_keys(transaction)?;
    let manifests = read_manifests(transaction)?;
    let journal = read_journal(transaction)?;
    assemble_facts(&catalog, manifests, journal, selected_unit)
}

#[derive(Debug)]
struct ManifestRow {
    manifest_sha256: String,
    unit_id: String,
    generation: i64,
    previous_manifest_sha256: Option<String>,
    desired_state: String,
    physical_owner: String,
    build_commit: String,
    build_sha256: String,
    catalog_sha256: String,
    business_schema_sha256: String,
    durable_schema_sha256: String,
    template_sha256: String,
    source_contract_sha256: String,
    evidence_sha256: String,
    approved_by: String,
    approved_at: i64,
    window_start: i64,
    window_end: i64,
    rollback_target_sha256: Option<String>,
    created_at: i64,
}

#[derive(Debug)]
struct JournalRow {
    event_id: String,
    unit_id: String,
    generation: i64,
    from_manifest_sha256: Option<String>,
    to_manifest_sha256: String,
    actor: String,
    action: String,
    reason: String,
    window_start: i64,
    window_end: i64,
    evidence_sha256: String,
    rollback_target_sha256: Option<String>,
    previous_sha256: Option<String>,
    canonical_sha256: String,
    occurred_at: i64,
}

fn validate_foreign_keys(transaction: &Transaction<'_>) -> Result<(), ActivationInspectError> {
    let violations: i64 = transaction
        .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .map_err(|_| ActivationInspectError::InvalidFacts)?;
    if violations == 0 {
        Ok(())
    } else {
        Err(ActivationInspectError::InvalidFacts)
    }
}

fn read_manifests(
    transaction: &Transaction<'_>,
) -> Result<Vec<ActivationManifest>, ActivationInspectError> {
    let mut statement = transaction
        .prepare(
            "SELECT manifest_sha256,unit_id,generation,previous_manifest_sha256,desired_state,\
             physical_owner,build_commit,build_sha256,catalog_sha256,business_schema_sha256,\
             durable_schema_sha256,template_sha256,source_contract_sha256,evidence_sha256,\
             approved_by,approved_at,window_start,window_end,rollback_target_sha256,created_at \
             FROM push_activation_manifests ORDER BY unit_id COLLATE BINARY,generation",
        )
        .map_err(|_| ActivationInspectError::InvalidFacts)?;
    let rows = statement
        .query_map([], |row| {
            Ok(ManifestRow {
                manifest_sha256: row.get(0)?,
                unit_id: row.get(1)?,
                generation: row.get(2)?,
                previous_manifest_sha256: row.get(3)?,
                desired_state: row.get(4)?,
                physical_owner: row.get(5)?,
                build_commit: row.get(6)?,
                build_sha256: row.get(7)?,
                catalog_sha256: row.get(8)?,
                business_schema_sha256: row.get(9)?,
                durable_schema_sha256: row.get(10)?,
                template_sha256: row.get(11)?,
                source_contract_sha256: row.get(12)?,
                evidence_sha256: row.get(13)?,
                approved_by: row.get(14)?,
                approved_at: row.get(15)?,
                window_start: row.get(16)?,
                window_end: row.get(17)?,
                rollback_target_sha256: row.get(18)?,
                created_at: row.get(19)?,
            })
        })
        .map_err(|_| ActivationInspectError::InvalidFacts)?;
    rows.map(|row| {
        row.map_err(|_| ActivationInspectError::InvalidFacts)
            .and_then(parse_manifest)
    })
    .collect()
}

fn read_journal(
    transaction: &Transaction<'_>,
) -> Result<Vec<PromotionJournalEntry>, ActivationInspectError> {
    let mut statement = transaction
        .prepare(
            "SELECT event_id,unit_id,generation,from_manifest_sha256,to_manifest_sha256,actor,\
             action,reason,window_start,window_end,evidence_sha256,rollback_target_sha256,\
             previous_sha256,canonical_sha256,occurred_at \
             FROM push_promotion_journal ORDER BY unit_id COLLATE BINARY,generation",
        )
        .map_err(|_| ActivationInspectError::InvalidFacts)?;
    let rows = statement
        .query_map([], |row| {
            Ok(JournalRow {
                event_id: row.get(0)?,
                unit_id: row.get(1)?,
                generation: row.get(2)?,
                from_manifest_sha256: row.get(3)?,
                to_manifest_sha256: row.get(4)?,
                actor: row.get(5)?,
                action: row.get(6)?,
                reason: row.get(7)?,
                window_start: row.get(8)?,
                window_end: row.get(9)?,
                evidence_sha256: row.get(10)?,
                rollback_target_sha256: row.get(11)?,
                previous_sha256: row.get(12)?,
                canonical_sha256: row.get(13)?,
                occurred_at: row.get(14)?,
            })
        })
        .map_err(|_| ActivationInspectError::InvalidFacts)?;
    rows.map(|row| {
        row.map_err(|_| ActivationInspectError::InvalidFacts)
            .and_then(parse_journal_entry)
    })
    .collect()
}

fn parse_manifest(row: ManifestRow) -> Result<ActivationManifest, ActivationInspectError> {
    let manifest = ActivationManifest {
        manifest_sha256: digest(&row.manifest_sha256)?,
        unit_id: unit_id(row.unit_id)?,
        generation: positive(row.generation)?,
        previous_manifest_sha256: optional_digest(row.previous_manifest_sha256.as_deref())?,
        desired_state: DesiredActivationState::parse(&row.desired_state)
            .ok_or(ActivationInspectError::InvalidFacts)?,
        physical_owner: bounded_text(row.physical_owner)?,
        build_commit: GitSha40::parse(&row.build_commit)
            .map_err(|_| ActivationInspectError::InvalidFacts)?,
        build_sha256: digest(&row.build_sha256)?,
        catalog_sha256: digest(&row.catalog_sha256)?,
        business_schema_sha256: digest(&row.business_schema_sha256)?,
        durable_schema_sha256: digest(&row.durable_schema_sha256)?,
        template_sha256: digest(&row.template_sha256)?,
        source_contract_sha256: digest(&row.source_contract_sha256)?,
        evidence_sha256: digest(&row.evidence_sha256)?,
        approved_by: bounded_text(row.approved_by)?,
        approved_at: nonnegative(row.approved_at)?,
        window_start: nonnegative(row.window_start)?,
        window_end: nonnegative(row.window_end)?,
        rollback_target_sha256: optional_digest(row.rollback_target_sha256.as_deref())?,
        created_at: nonnegative(row.created_at)?,
    };
    validate_manifest_value(&manifest)?;
    Ok(manifest)
}

fn parse_journal_entry(row: JournalRow) -> Result<PromotionJournalEntry, ActivationInspectError> {
    let entry = PromotionJournalEntry {
        event_id: digest(&row.event_id)?,
        unit_id: unit_id(row.unit_id)?,
        generation: positive(row.generation)?,
        from_manifest_sha256: optional_digest(row.from_manifest_sha256.as_deref())?,
        to_manifest_sha256: digest(&row.to_manifest_sha256)?,
        actor: bounded_text(row.actor)?,
        action: PromotionAction::parse(&row.action).ok_or(ActivationInspectError::InvalidFacts)?,
        reason: bounded_text(row.reason)?,
        window_start: nonnegative(row.window_start)?,
        window_end: nonnegative(row.window_end)?,
        evidence_sha256: digest(&row.evidence_sha256)?,
        rollback_target_sha256: optional_digest(row.rollback_target_sha256.as_deref())?,
        previous_sha256: optional_digest(row.previous_sha256.as_deref())?,
        canonical_sha256: digest(&row.canonical_sha256)?,
        occurred_at: nonnegative(row.occurred_at)?,
    };
    validate_journal_value(&entry)?;
    Ok(entry)
}

/// Validate typed values which normally enter this module through the SQLite row parser.
/// Internal writers use the same checks because their candidate structs have not been parsed.
pub(super) fn validate_manifest_value(
    manifest: &ActivationManifest,
) -> Result<(), ActivationInspectError> {
    validate_unit_value(&manifest.unit_id)?;
    validate_bounded_text(&manifest.physical_owner)?;
    validate_bounded_text(&manifest.approved_by)?;
    if manifest.window_end <= manifest.window_start || manifest.approved_at > manifest.created_at {
        return Err(ActivationInspectError::InvalidFacts);
    }
    Ok(())
}

/// Validate typed journal values using the same scalar rules as the SQLite row parser.
pub(super) fn validate_journal_value(
    entry: &PromotionJournalEntry,
) -> Result<(), ActivationInspectError> {
    validate_unit_value(&entry.unit_id)?;
    validate_bounded_text(&entry.actor)?;
    validate_bounded_text(&entry.reason)?;
    if entry.window_end <= entry.window_start
        || entry.occurred_at < entry.window_start
        || entry.occurred_at >= entry.window_end
        || entry.reason != "activation.applied"
    {
        return Err(ActivationInspectError::InvalidFacts);
    }
    Ok(())
}

fn assemble_facts(
    catalog: &MachineCatalog,
    manifests: Vec<ActivationManifest>,
    journal: Vec<PromotionJournalEntry>,
    selected_unit: &UnitId,
) -> Result<RawActivationFacts, ActivationInspectError> {
    let mut manifests_by_unit = BTreeMap::<UnitId, Vec<ActivationManifest>>::new();
    for manifest in manifests {
        if catalog.unit(&manifest.unit_id).is_none() {
            return Err(ActivationInspectError::InvalidFacts);
        }
        manifests_by_unit
            .entry(manifest.unit_id.clone())
            .or_default()
            .push(manifest);
    }
    let mut journal_by_unit = BTreeMap::<UnitId, Vec<PromotionJournalEntry>>::new();
    for entry in journal {
        if catalog.unit(&entry.unit_id).is_none() {
            return Err(ActivationInspectError::InvalidFacts);
        }
        journal_by_unit
            .entry(entry.unit_id.clone())
            .or_default()
            .push(entry);
    }

    let mut units = Vec::with_capacity(catalog.units().len());
    for registration in catalog.units() {
        let unit_id = registration.id().clone();
        let manifests = manifests_by_unit.remove(&unit_id).unwrap_or_default();
        let journal = journal_by_unit.remove(&unit_id).unwrap_or_default();
        validate_manifest_chain(&manifests)?;
        validate_journal_chain(&manifests, &journal)?;
        let reconciliation = reconcile(manifests.len(), journal.len())?;
        units.push(UnitActivationFacts {
            unit_id,
            manifests,
            journal,
            reconciliation,
        });
    }
    if !manifests_by_unit.is_empty() || !journal_by_unit.is_empty() {
        return Err(ActivationInspectError::InvalidFacts);
    }
    let selected_unit_index = units
        .iter()
        .position(|facts| &facts.unit_id == selected_unit)
        .ok_or(ActivationInspectError::UnknownRequestedUnit)?;
    Ok(RawActivationFacts {
        units,
        selected_unit_index,
    })
}

pub(super) fn validate_manifest_chain(
    chain: &[ActivationManifest],
) -> Result<(), ActivationInspectError> {
    for (index, manifest) in chain.iter().enumerate() {
        let expected_generation = index as u64 + 1;
        if manifest.generation != expected_generation
            || manifest.manifest_sha256 != manifest_digest(manifest)
        {
            return Err(ActivationInspectError::InvalidFacts);
        }
        if index == 0 {
            if manifest.previous_manifest_sha256.is_some()
                || manifest.desired_state != DesiredActivationState::Disabled
                || manifest.rollback_target_sha256.is_some()
            {
                return Err(ActivationInspectError::InvalidFacts);
            }
            continue;
        }
        let previous = &chain[index - 1];
        if manifest.previous_manifest_sha256.as_ref() != Some(&previous.manifest_sha256) {
            return Err(ActivationInspectError::InvalidFacts);
        }
        match manifest.rollback_target_sha256.as_ref() {
            None if !valid_forward_edge(previous.desired_state, manifest.desired_state) => {
                return Err(ActivationInspectError::InvalidFacts);
            }
            Some(target_sha256) => {
                let target = chain[..index]
                    .iter()
                    .find(|candidate| &candidate.manifest_sha256 == target_sha256)
                    .ok_or(ActivationInspectError::InvalidFacts)?;
                if target.unit_id != manifest.unit_id
                    || target.desired_state != manifest.desired_state
                    || target.physical_owner != manifest.physical_owner
                {
                    return Err(ActivationInspectError::InvalidFacts);
                }
            }
            None => {}
        }
    }
    Ok(())
}

pub(super) fn validate_journal_chain(
    manifests: &[ActivationManifest],
    journal: &[PromotionJournalEntry],
) -> Result<(), ActivationInspectError> {
    if journal.len() > manifests.len() {
        return Err(ActivationInspectError::InvalidFacts);
    }
    for (index, entry) in journal.iter().enumerate() {
        let generation = index as u64 + 1;
        let manifest = &manifests[index];
        let expected_previous = index.checked_sub(1).map(|previous| &journal[previous]);
        if entry.generation != generation
            || entry.unit_id != manifest.unit_id
            || entry.event_id != promotion_event_id(entry.unit_id.as_str(), generation)
            || entry.canonical_sha256 != journal_digest(entry)
            || entry.from_manifest_sha256 != manifest.previous_manifest_sha256
            || entry.to_manifest_sha256 != manifest.manifest_sha256
            || entry.actor != manifest.approved_by
            || entry.window_start != manifest.window_start
            || entry.window_end != manifest.window_end
            || entry.evidence_sha256 != manifest.evidence_sha256
            || entry.rollback_target_sha256 != manifest.rollback_target_sha256
            || entry.previous_sha256.as_ref()
                != expected_previous.map(|previous| &previous.canonical_sha256)
            || entry.occurred_at < manifest.approved_at
            || entry.action != expected_action(manifest)?
        {
            return Err(ActivationInspectError::InvalidFacts);
        }
    }
    Ok(())
}

fn reconcile(
    manifest_count: usize,
    journal_count: usize,
) -> Result<ActivationReconciliation, ActivationInspectError> {
    match (manifest_count, journal_count) {
        (0, 0) => Ok(ActivationReconciliation::Unregistered),
        (manifests, journal) if manifests == journal => Ok(ActivationReconciliation::CaughtUp {
            generation: manifests as u64,
        }),
        (manifests, journal) if manifests == journal + 1 => Ok(ActivationReconciliation::Pending {
            executed_generation: (journal != 0).then_some(journal as u64),
            pending_generation: manifests as u64,
        }),
        _ => Err(ActivationInspectError::InvalidFacts),
    }
}

fn expected_action(
    manifest: &ActivationManifest,
) -> Result<PromotionAction, ActivationInspectError> {
    if manifest.rollback_target_sha256.is_some() {
        return Ok(PromotionAction::Rollback);
    }
    match (manifest.generation, manifest.desired_state) {
        (1, DesiredActivationState::Disabled) => Ok(PromotionAction::Initialize),
        (_, DesiredActivationState::Shadow) => Ok(PromotionAction::EnterShadow),
        (_, DesiredActivationState::Active) => Ok(PromotionAction::Activate),
        (_, DesiredActivationState::Draining) => Ok(PromotionAction::Drain),
        (generation, DesiredActivationState::Disabled) if generation > 1 => {
            Ok(PromotionAction::Disable)
        }
        _ => Err(ActivationInspectError::InvalidFacts),
    }
}

fn valid_forward_edge(before: DesiredActivationState, after: DesiredActivationState) -> bool {
    matches!(
        (before, after),
        (
            DesiredActivationState::Disabled,
            DesiredActivationState::Shadow
        ) | (
            DesiredActivationState::Shadow,
            DesiredActivationState::Active
        ) | (
            DesiredActivationState::Active,
            DesiredActivationState::Draining
        ) | (
            DesiredActivationState::Draining,
            DesiredActivationState::Disabled
        )
    )
}

fn unit_id(value: String) -> Result<UnitId, ActivationInspectError> {
    if value.contains('\0') {
        return Err(ActivationInspectError::InvalidFacts);
    }
    UnitId::try_new(value).map_err(|_| ActivationInspectError::InvalidFacts)
}

fn bounded_text(value: String) -> Result<String, ActivationInspectError> {
    validate_bounded_text(&value)?;
    Ok(value)
}

fn validate_unit_value(value: &UnitId) -> Result<(), ActivationInspectError> {
    if value.as_str().contains('\0') {
        Err(ActivationInspectError::InvalidFacts)
    } else {
        Ok(())
    }
}

fn validate_bounded_text(value: &str) -> Result<(), ActivationInspectError> {
    let character_count = value.chars().count();
    if character_count == 0 || character_count > 512 || value.contains('\0') {
        Err(ActivationInspectError::InvalidFacts)
    } else {
        Ok(())
    }
}

fn digest(value: &str) -> Result<Sha256Digest, ActivationInspectError> {
    Sha256Digest::parse("activation_sha256", value)
        .map_err(|_| ActivationInspectError::InvalidFacts)
}

fn optional_digest(value: Option<&str>) -> Result<Option<Sha256Digest>, ActivationInspectError> {
    value.map(digest).transpose()
}

fn nonnegative(value: i64) -> Result<u64, ActivationInspectError> {
    u64::try_from(value).map_err(|_| ActivationInspectError::InvalidFacts)
}

fn positive(value: i64) -> Result<u64, ActivationInspectError> {
    let value = nonnegative(value)?;
    if value == 0 {
        Err(ActivationInspectError::InvalidFacts)
    } else {
        Ok(value)
    }
}
