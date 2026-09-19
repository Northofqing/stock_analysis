use std::collections::HashMap;

use chrono::{DateTime, Duration, FixedOffset, NaiveDate, NaiveDateTime, TimeZone, Utc};
use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};

use crate::database::concepts::parse_cached_concept_rows;
use crate::database::read_open_position_rows;
use crate::monitor::push_job::{raw_digest, IntentId, UtcMicros};
use crate::pipeline::chain_analysis::preparation::{PositionInput, PositionObservationClock};

use super::positions_codec::{
    self as positions_codec, DecodedPositionConcepts, DecodedPositions, PositionCacheSourceRow,
};
use super::{
    board, check_lease, cluster, concept_rpc, inspect_concept_batch_from_recovery_on,
    inspect_run_on_at_layout, schema, storage, ChainPostCloseError, LocalChainPostClose, RunLease,
    RunRecovery,
};

struct StoredPositions {
    run_id: String,
    context_digest: String,
    input_digest: String,
    digest: String,
    owner: String,
    generation: u64,
    prior_head: u64,
    run_version: u64,
    observed_at: i64,
    committed_at: i64,
    row_count: usize,
    decoded: DecodedPositions,
}

struct StoredPositionConcepts {
    digest: String,
    run_id: String,
    context_digest: String,
    input_digest: String,
    owner: String,
    generation: u64,
    prior_head: u64,
    run_version: u64,
    observed_at: i64,
    committed_at: i64,
    positions_run_version: u64,
    positions_digest: String,
    requested_count: usize,
    cache_row_count: usize,
    cache_cutoff: String,
    local_offset_seconds: i32,
    cutoff_local_offset_seconds: i32,
    decoded: DecodedPositionConcepts,
}

pub(super) struct ValidatedPositionFacts<'transaction, 'connection, 'run> {
    transaction: &'transaction Transaction<'connection>,
    run: &'run RunRecovery,
    intent: String,
    layout_version: i64,
    positions: Option<StoredPositions>,
    concepts: Option<StoredPositionConcepts>,
}

impl<'transaction, 'connection, 'run> ValidatedPositionFacts<'transaction, 'connection, 'run> {
    fn capture(
        transaction: &'transaction Transaction<'connection>,
        intent: &IntentId,
        run: &'run RunRecovery,
        layout_version: i64,
        positions: Option<StoredPositions>,
        concepts: Option<StoredPositionConcepts>,
    ) -> Self {
        Self {
            transaction,
            run,
            intent: intent.as_str().to_owned(),
            layout_version,
            positions,
            concepts,
        }
    }

    pub(super) fn matches(
        &self,
        transaction: &Transaction<'_>,
        intent: &IntentId,
        run: &RunRecovery,
        layout_version: i64,
    ) -> bool {
        std::ptr::eq(self.transaction, transaction)
            && std::ptr::eq(self.run, run)
            && self.intent == intent.as_str()
            && self.layout_version == layout_version
    }

    /// Borrow only within the originating no-write inspection pass. The token
    /// is still consumed by DragonTiger later in that same pass.
    pub(super) fn position_batch_parent_for_read_pass(
        &self,
        transaction: &Transaction<'_>,
        intent: &IntentId,
        run: &RunRecovery,
        layout_version: i64,
    ) -> Result<Option<PositionConceptMaterial>, ChainPostCloseError> {
        if !self.matches(transaction, intent, run, layout_version) {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        Ok(self
            .concepts
            .as_ref()
            .map(|stored| position_material(intent, stored)))
    }
}

pub(super) struct PositionConceptMaterial {
    parent: PositionBatchParent,
    pub(super) cache_rows: Vec<PositionCacheSourceRow>,
}

pub(super) struct PositionBatchParent {
    _validated: (),
    pub(super) intent_id: IntentId,
    pub(super) run_id: String,
    pub(super) context_digest: String,
    pub(super) input_digest: String,
    pub(super) cache_version: u64,
    pub(super) cache_digest: String,
    pub(super) positions_version: u64,
    pub(super) positions_digest: String,
    pub(super) requested_codes: Vec<String>,
}

pub(super) struct PositionBatch {
    pub(super) parent: PositionBatchParent,
    pub(super) cached: HashMap<String, Vec<String>>,
    pub(super) work: Vec<(u64, String)>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PositionStageKind {
    EmptyPositions,
    AllCached,
    FetchedAndCached,
}

impl PositionStageKind {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::EmptyPositions => "EmptyPositions",
            Self::AllCached => "AllCached",
            Self::FetchedAndCached => "FetchedAndCached",
        }
    }
}

pub(super) struct PositionStageCompletion {
    pub(super) kind: PositionStageKind,
    pub(super) run_id: String,
    pub(super) context_digest: String,
    pub(super) input_digest: String,
    pub(super) positions_version: u64,
    pub(super) positions_digest: String,
    pub(super) positions_owner: String,
    pub(super) positions_generation: u64,
    pub(super) positions_time: i64,
    pub(super) concepts_version: Option<u64>,
    pub(super) concepts_digest: Option<String>,
    pub(super) concepts_owner: Option<String>,
    pub(super) concepts_generation: Option<u64>,
    pub(super) concepts_time: Option<i64>,
    pub(super) requested_codes: Vec<String>,
    pub(super) fetched: Vec<PositionRpcCompletion>,
    pub(super) completion_version: u64,
}

pub(super) struct PositionRpcCompletion {
    pub(super) ordinal: u64,
    pub(super) code: String,
    pub(super) occurrence_version: u64,
    pub(super) occurrence_digest: String,
    pub(super) occurrence_owner: String,
    pub(super) occurrence_generation: u64,
    pub(super) occurrence_time: i64,
    pub(super) terminal_version: u64,
    pub(super) terminal_digest: String,
    pub(super) terminal_owner: String,
    pub(super) terminal_generation: u64,
    pub(super) terminal_time: i64,
    pub(super) final_version: u64,
    pub(super) final_digest: String,
    pub(super) final_owner: String,
    pub(super) final_generation: u64,
    pub(super) final_time: i64,
    pub(super) cache_version: u64,
    pub(super) cache_digest: String,
    pub(super) cache_owner: String,
    pub(super) cache_generation: u64,
    pub(super) cache_time: i64,
}

impl PositionConceptMaterial {
    pub(super) fn into_batch(self) -> Result<PositionBatch, String> {
        let cached = parse_cached_concept_rows(
            self.cache_rows
                .into_iter()
                .map(|row| (row.code, row.concepts)),
        )?;
        let work = self
            .parent
            .requested_codes
            .iter()
            .enumerate()
            .filter(|(_, code)| !cached.contains_key(*code))
            .map(|(ordinal, code)| (ordinal as u64, code.clone()))
            .collect();
        Ok(PositionBatch {
            parent: self.parent,
            cached,
            work,
        })
    }

    pub(super) fn parse(self) -> Result<HashMap<String, Vec<String>>, String> {
        parse_cached_concept_rows(
            self.cache_rows
                .into_iter()
                .map(|row| (row.code, row.concepts)),
        )
    }
}

fn authority_valid(
    fact_generation: u64,
    fact_owner: &str,
    parent_generation: u64,
    parent_owner: &str,
    recovery: &RunRecovery,
) -> bool {
    fact_generation >= parent_generation
        && (fact_generation != parent_generation || fact_owner == parent_owner)
        && fact_generation <= recovery.generation
        && (fact_generation != recovery.generation || fact_owner == recovery.owner)
}

fn validate_parent_chain(
    transaction: &Transaction<'_>,
    intent: &IntentId,
    recovery: &RunRecovery,
    layout_version: i64,
    proof: Option<&schema::V12CatalogProof<'_, '_>>,
) -> Result<cluster::BoardParentFact, ChainPostCloseError> {
    let concepts = inspect_concept_batch_from_recovery_on(transaction, intent, recovery)?;
    let parent = cluster::validate_existing_cluster_facts_scoped(
        transaction,
        intent,
        recovery,
        &concepts,
        layout_version,
        proof,
    )?
    .ok_or(ChainPostCloseError::SchemaRejected)?;
    board::validate_positions_parent_scoped(
        transaction,
        intent,
        recovery,
        Some(&parent),
        layout_version,
        proof,
    )?;
    concept_rpc::validate_concept_rpc_facts(transaction, intent)?;
    Ok(parent)
}

fn load_positions(
    transaction: &Transaction<'_>,
    intent: &IntentId,
) -> Result<Option<StoredPositions>, ChainPostCloseError> {
    type Row = (
        String,
        String,
        String,
        i64,
        Vec<u8>,
        i64,
        String,
        String,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
    );
    transaction
        .query_row(
            "SELECT run_id,run_context_sha256,input_sha256,material_codec_version,\
                    CAST(material_bytes AS BLOB),material_length,material_sha256,lease_owner,\
                    lease_generation,prior_head_version,run_version,observed_at,committed_at,row_count \
             FROM chain_post_close_position_materials WHERE intent_id=?1",
            [intent.as_str()],
            |row| {
                Ok((
                    row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?,
                    row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?,
                    row.get(10)?, row.get(11)?, row.get(12)?, row.get(13)?,
                ))
            },
        )
        .optional()
        .map_err(|_| storage("position material read"))?
        .map(|row: Row| {
            let decoded = positions_codec::decode_positions(&row.4)?;
            if row.3 != 1
                || row.5 != i64::try_from(row.4.len()).unwrap_or(-1)
                || row.6 != raw_digest(&row.4).as_str()
                || row.8 < 1
                || row.9 < 0
                || row.10 != row.9.checked_add(1).unwrap_or(-1)
                || row.11 < 0
                || row.12 < row.11
                || row.13 < 0
                || usize::try_from(row.13).ok() != Some(decoded.rows.len())
                || decoded.observed_at.get() != row.11
            {
                return Err(ChainPostCloseError::SchemaRejected);
            }
            Ok(StoredPositions {
                run_id: row.0,
                context_digest: row.1,
                input_digest: row.2,
                digest: row.6,
                owner: row.7,
                generation: u64::try_from(row.8)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                prior_head: u64::try_from(row.9)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                run_version: u64::try_from(row.10)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                observed_at: row.11,
                committed_at: row.12,
                row_count: usize::try_from(row.13)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                decoded,
            })
        })
        .transpose()
}

fn load_position_concepts(
    transaction: &Transaction<'_>,
    intent: &IntentId,
) -> Result<Option<StoredPositionConcepts>, ChainPostCloseError> {
    type Row = (
        String,
        String,
        String,
        i64,
        Vec<u8>,
        i64,
        String,
        String,
        i64,
        i64,
        i64,
        i64,
        i64,
        String,
        i64,
        String,
        i64,
        i64,
        i64,
        String,
        i64,
        i64,
    );
    transaction
        .query_row(
            "SELECT run_id,run_context_sha256,input_sha256,material_codec_version,\
                    CAST(material_bytes AS BLOB),material_length,material_sha256,lease_owner,\
                    lease_generation,prior_head_version,run_version,observed_at,committed_at,\
                    batch_kind,positions_run_version,positions_sha256,requested_count,\
                    cache_row_count,cache_max_age_days,cache_cutoff,local_offset_seconds,\
                    cutoff_local_offset_seconds \
             FROM chain_post_close_position_concept_materials WHERE intent_id=?1",
            [intent.as_str()],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                    row.get(9)?,
                    row.get(10)?,
                    row.get(11)?,
                    row.get(12)?,
                    row.get(13)?,
                    row.get(14)?,
                    row.get(15)?,
                    row.get(16)?,
                    row.get(17)?,
                    row.get(18)?,
                    row.get(19)?,
                    row.get(20)?,
                    row.get(21)?,
                ))
            },
        )
        .optional()
        .map_err(|_| storage("position concept material read"))?
        .map(|row: Row| {
            let decoded = positions_codec::decode_position_concepts(&row.4)?;
            if row.3 != 1
                || row.5 != i64::try_from(row.4.len()).unwrap_or(-1)
                || row.6 != raw_digest(&row.4).as_str()
                || row.8 < 1
                || row.9 < 0
                || row.10 != row.9.checked_add(1).unwrap_or(-1)
                || row.11 < 0
                || row.12 < row.11
                || row.13 != "PositionConcepts"
                || row.14 < 1
                || row.16 < 1
                || row.17 < 0
                || row.18 != 7
                || usize::try_from(row.16).ok() != Some(decoded.requested_codes.len())
                || usize::try_from(row.17).ok() != Some(decoded.cache_rows.len())
                || decoded.observed_at.get() != row.11
                || decoded.positions_run_version != u64::try_from(row.14).unwrap_or(0)
                || decoded.positions_sha256 != row.15
                || decoded.cache_cutoff != row.19
                || i64::from(decoded.local_offset_seconds) != row.20
                || i64::from(decoded.cutoff_local_offset_seconds) != row.21
            {
                return Err(ChainPostCloseError::SchemaRejected);
            }
            Ok(StoredPositionConcepts {
                digest: row.6,
                run_id: row.0,
                context_digest: row.1,
                input_digest: row.2,
                owner: row.7,
                generation: u64::try_from(row.8)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                prior_head: u64::try_from(row.9)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                run_version: u64::try_from(row.10)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                observed_at: row.11,
                committed_at: row.12,
                positions_run_version: u64::try_from(row.14)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                positions_digest: row.15,
                requested_count: usize::try_from(row.16)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                cache_row_count: usize::try_from(row.17)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                cache_cutoff: row.19,
                local_offset_seconds: i32::try_from(row.20)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                cutoff_local_offset_seconds: i32::try_from(row.21)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                decoded,
            })
        })
        .transpose()
}

fn validate_positions(
    transaction: &Transaction<'_>,
    intent: &IntentId,
    stored: &StoredPositions,
    recovery: &RunRecovery,
    parent: &cluster::BoardParentFact,
    layout_version: i64,
    proof: Option<&schema::V12CatalogProof<'_, '_>>,
    validated_board: Option<&board::ValidatedBoardFacts<'_, '_>>,
) -> Result<(), ChainPostCloseError> {
    let board_parent = match validated_board {
        Some(board) => board.positions_parent_for_read_pass(
            transaction,
            intent,
            recovery,
            Some(parent),
            layout_version,
        )?,
        None => board::validate_positions_parent_scoped(
            transaction,
            intent,
            recovery,
            Some(parent),
            layout_version,
            proof,
        )?,
    };
    if stored.run_id != recovery.context.run_id().as_str()
        || stored.context_digest != recovery.context.canonical_sha256().as_str()
        || stored.input_digest != raw_digest(&recovery.input.encode()?).as_str()
        || stored.decoded.business_date != recovery.input.business_date()
        || stored.row_count != stored.decoded.rows.len()
        || stored.prior_head.checked_add(1) != Some(stored.run_version)
        || stored.prior_head < board_parent.run_version
        || stored.prior_head < parent.application_version
        || stored.run_version > recovery.head
        || stored.observed_at > stored.committed_at
        || stored.committed_at > recovery.updated_at
        || stored.observed_at < board_parent.ready_at
        || stored.observed_at < parent.applied_at
        || !authority_valid(
            stored.generation,
            &stored.owner,
            parent.application_generation,
            &parent.application_owner,
            recovery,
        )
        || !authority_valid(
            stored.generation,
            &stored.owner,
            board_parent.generation,
            &board_parent.owner,
            recovery,
        )
    {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    Ok(())
}

fn validate_position_concepts(
    stored: &StoredPositionConcepts,
    positions: &StoredPositions,
    recovery: &RunRecovery,
) -> Result<(), ChainPostCloseError> {
    let requested = positions.decoded.requested_codes();
    if positions.row_count == 0
        || stored.run_id != recovery.context.run_id().as_str()
        || stored.context_digest != recovery.context.canonical_sha256().as_str()
        || stored.input_digest != raw_digest(&recovery.input.encode()?).as_str()
        || stored.positions_run_version != positions.run_version
        || stored.positions_digest != positions.digest
        || stored.decoded.positions_run_version != positions.run_version
        || stored.decoded.positions_sha256 != positions.digest
        || stored.decoded.requested_codes != requested
        || stored.requested_count != positions.row_count
        || stored.cache_row_count != stored.decoded.cache_rows.len()
        || stored.prior_head.checked_add(1) != Some(stored.run_version)
        || stored.positions_run_version >= stored.run_version
        || stored.run_version > recovery.head
        || stored.observed_at > stored.committed_at
        || stored.committed_at > recovery.updated_at
        || stored.observed_at < positions.committed_at
        || !authority_valid(
            stored.generation,
            &stored.owner,
            positions.generation,
            &positions.owner,
            recovery,
        )
        || stored.cache_cutoff != stored.decoded.cache_cutoff
        || stored.local_offset_seconds != stored.decoded.local_offset_seconds
        || stored.cutoff_local_offset_seconds != stored.decoded.cutoff_local_offset_seconds
        || !valid_cache_observation(stored)
    {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    Ok(())
}

pub(super) fn validate_existing_position_facts_at_layout(
    transaction: &Transaction<'_>,
    intent: &IntentId,
    recovery: &RunRecovery,
    parent: Option<&cluster::BoardParentFact>,
    layout_version: i64,
) -> Result<(), ChainPostCloseError> {
    validate_existing_position_facts_and_capture_at_layout(
        transaction,
        intent,
        recovery,
        parent,
        layout_version,
    )
    .map(|_| ())
}

pub(super) fn validate_existing_position_facts_and_capture_at_layout<
    'transaction,
    'connection,
    'run,
>(
    transaction: &'transaction Transaction<'connection>,
    intent: &IntentId,
    recovery: &'run RunRecovery,
    parent: Option<&cluster::BoardParentFact>,
    layout_version: i64,
) -> Result<ValidatedPositionFacts<'transaction, 'connection, 'run>, ChainPostCloseError> {
    validate_existing_position_facts_and_capture_scoped(
        transaction,
        intent,
        recovery,
        parent,
        layout_version,
        None,
    )
}

pub(super) fn validate_existing_position_facts_and_capture_scoped<
    'transaction,
    'connection,
    'run,
>(
    transaction: &'transaction Transaction<'connection>,
    intent: &IntentId,
    recovery: &'run RunRecovery,
    parent: Option<&cluster::BoardParentFact>,
    layout_version: i64,
    proof: Option<&schema::V12CatalogProof<'_, '_>>,
) -> Result<ValidatedPositionFacts<'transaction, 'connection, 'run>, ChainPostCloseError> {
    validate_position_facts_with_board_validation(
        transaction,
        intent,
        recovery,
        parent,
        layout_version,
        proof,
        None,
    )
}

pub(super) fn validate_position_facts_after_board_validation<'transaction, 'connection, 'run>(
    transaction: &'transaction Transaction<'connection>,
    intent: &IntentId,
    recovery: &'run RunRecovery,
    parent: Option<&cluster::BoardParentFact>,
    proof: &schema::V12CatalogProof<'_, '_>,
    board: &board::ValidatedBoardFacts<'_, '_>,
) -> Result<ValidatedPositionFacts<'transaction, 'connection, 'run>, ChainPostCloseError> {
    validate_position_facts_with_board_validation(
        transaction,
        intent,
        recovery,
        parent,
        proof.layout(),
        Some(proof),
        Some(board),
    )
}

fn validate_position_facts_with_board_validation<'transaction, 'connection, 'run>(
    transaction: &'transaction Transaction<'connection>,
    intent: &IntentId,
    recovery: &'run RunRecovery,
    parent: Option<&cluster::BoardParentFact>,
    layout_version: i64,
    proof: Option<&schema::V12CatalogProof<'_, '_>>,
    validated_board: Option<&board::ValidatedBoardFacts<'_, '_>>,
) -> Result<ValidatedPositionFacts<'transaction, 'connection, 'run>, ChainPostCloseError> {
    if proof.is_some() && layout_version < 12 {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    if layout_version >= 12 {
        schema::verify_parent_layout_v12_scoped(transaction, proof)?;
    } else if !matches!(layout_version, 8 | 9 | 10 | 11) {
        return Err(ChainPostCloseError::UnsupportedVersion);
    }
    if let Some(board) = validated_board {
        if layout_version < 12 || proof.is_none() {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        // Even empty positions/concepts cannot accept another pass's token.
        // This checks identity only; readiness remains below the material loads.
        board.check(transaction, intent, recovery, parent, layout_version)?;
    }
    let positions = load_positions(transaction, intent)?;
    let concepts = load_position_concepts(transaction, intent)?;
    match (positions.as_ref(), concepts.as_ref()) {
        (None, None) => {}
        (Some(positions), None) => {
            let parent = parent.ok_or(ChainPostCloseError::SchemaRejected)?;
            validate_positions(
                transaction,
                intent,
                positions,
                recovery,
                parent,
                layout_version,
                proof,
                validated_board,
            )?;
        }
        (Some(positions), Some(concepts)) => {
            let parent = parent.ok_or(ChainPostCloseError::SchemaRejected)?;
            validate_positions(
                transaction,
                intent,
                positions,
                recovery,
                parent,
                layout_version,
                proof,
                validated_board,
            )?;
            validate_position_concepts(concepts, positions, recovery)?;
        }
        (None, Some(_)) => return Err(ChainPostCloseError::SchemaRejected),
    }
    Ok(ValidatedPositionFacts::capture(
        transaction,
        intent,
        recovery,
        layout_version,
        positions,
        concepts,
    ))
}

fn validate_full_run(
    transaction: &Transaction<'_>,
    intent: &IntentId,
    recovery: &RunRecovery,
    layout_version: i64,
) -> Result<cluster::BoardParentFact, ChainPostCloseError> {
    validate_full_run_scoped(transaction, intent, recovery, layout_version, None)
}

fn validate_full_run_scoped(
    transaction: &Transaction<'_>,
    intent: &IntentId,
    recovery: &RunRecovery,
    layout_version: i64,
    proof: Option<&schema::V12CatalogProof<'_, '_>>,
) -> Result<cluster::BoardParentFact, ChainPostCloseError> {
    let parent = validate_parent_chain(transaction, intent, recovery, layout_version, proof)?;
    validate_existing_position_facts_and_capture_scoped(
        transaction,
        intent,
        recovery,
        Some(&parent),
        layout_version,
        proof,
    )?;
    Ok(parent)
}

fn advance_run(
    transaction: &Transaction<'_>,
    lease: &RunLease,
    previous: u64,
    next: u64,
    now: UtcMicros,
    operation: &'static str,
) -> Result<(), ChainPostCloseError> {
    let changed = transaction
        .execute(
            "UPDATE chain_post_close_runs SET head_version=?1,updated_at=?2 \
             WHERE intent_id=?3 AND run_id=?4 AND lease_owner=?5 AND lease_generation=?6 \
               AND head_version=?7 AND lease_until>?2",
            params![
                next,
                now.get(),
                lease.intent_id.as_str(),
                lease.run_id.as_str(),
                lease.owner.as_str(),
                lease.generation,
                previous,
            ],
        )
        .map_err(|_| storage(operation))?;
    if changed != 1 {
        return Err(ChainPostCloseError::StaleLease {
            intent_id: lease.intent_id.as_str().to_owned(),
        });
    }
    Ok(())
}

impl LocalChainPostClose<'_> {
    pub(super) fn capture_or_load_positions(
        &mut self,
        lease: &RunLease,
        clock: &dyn PositionObservationClock,
    ) -> Result<(u64, Vec<PositionInput>), ChainPostCloseError> {
        let admitted_at = clock.now();
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("begin"))?;
        let layout_version = schema::runtime_layout_version(&transaction)?;
        // runtime_layout_version has attested the exact current catalog above.
        if !matches!(layout_version, 8 | 9 | 10 | 11 | 12 | 13) {
            return Err(ChainPostCloseError::UnsupportedVersion);
        }
        let recovery = inspect_run_on_at_layout(&transaction, &lease.intent_id, layout_version)?;
        check_lease(&transaction, lease, admitted_at)?;
        if recovery.context.run_id() != &lease.run_id
            || recovery.input.encode()? != lease.input.encode()?
            || recovery.generation != lease.generation
            || recovery.head != lease.head
        {
            return Err(ChainPostCloseError::StaleLease {
                intent_id: lease.intent_id.as_str().to_owned(),
            });
        }
        let parent = validate_full_run(&transaction, &lease.intent_id, &recovery, layout_version)?;
        if let Some(stored) = load_positions(&transaction, &lease.intent_id)? {
            validate_positions(
                &transaction,
                &lease.intent_id,
                &stored,
                &recovery,
                &parent,
                layout_version,
                None,
                None,
            )?;
            let result = stored.decoded.projection();
            transaction.commit().map_err(|_| storage("commit"))?;
            return Ok((lease.head, result));
        }
        let observed_at = clock.now();
        let rows =
            read_open_position_rows(&transaction).map_err(|_| storage("position source read"))?;
        let bytes = positions_codec::encode_positions(
            NaiveDate::parse_from_str(recovery.input.business_date(), "%Y-%m-%d")
                .map_err(|_| ChainPostCloseError::SchemaRejected)?,
            observed_at,
            &rows,
        )?;
        let decoded = positions_codec::decode_positions(&bytes)?;
        let committed_at = clock.now();
        if observed_at < admitted_at || observed_at > committed_at {
            return Err(ChainPostCloseError::InvalidInput {
                check: "position observation",
            });
        }
        check_lease(&transaction, lease, committed_at)?;
        let digest = raw_digest(&bytes);
        let next = lease
            .head
            .checked_add(1)
            .ok_or_else(|| storage("head overflow"))?;
        advance_run(
            &transaction,
            lease,
            lease.head,
            next,
            committed_at,
            "position material cas",
        )?;
        transaction
            .execute(
                "INSERT INTO chain_post_close_position_materials(\
                 intent_id,run_id,run_context_sha256,input_sha256,material_codec_version,\
                 material_bytes,material_length,material_sha256,lease_owner,lease_generation,\
                 prior_head_version,run_version,observed_at,committed_at,row_count) \
                 VALUES(?1,?2,?3,?4,1,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
                params![
                    lease.intent_id.as_str(),
                    lease.run_id.as_str(),
                    recovery.context.canonical_sha256().as_str(),
                    raw_digest(&recovery.input.encode()?).as_str(),
                    &bytes,
                    i64::try_from(bytes.len()).map_err(|_| storage("position material length"))?,
                    digest.as_str(),
                    lease.owner.as_str(),
                    lease.generation,
                    lease.head,
                    next,
                    observed_at.get(),
                    committed_at.get(),
                    i64::try_from(rows.len()).map_err(|_| storage("position row count"))?,
                ],
            )
            .map_err(|_| storage("position material fact"))?;
        super::validate_run_fact_versions_at_layout(
            &transaction,
            &lease.intent_id,
            next,
            layout_version,
        )?;
        let written = load_positions(&transaction, &lease.intent_id)?
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        let mut written_recovery = recovery;
        written_recovery.head = next;
        written_recovery.updated_at = committed_at.get();
        validate_positions(
            &transaction,
            &lease.intent_id,
            &written,
            &written_recovery,
            &parent,
            layout_version,
            None,
            None,
        )?;
        transaction.commit().map_err(|_| storage("commit"))?;
        Ok((next, decoded.projection()))
    }

    pub(super) fn capture_or_load_position_concepts(
        &mut self,
        lease: &RunLease,
        requested_codes: &[String],
        clock: &dyn PositionObservationClock,
    ) -> Result<(u64, PositionConceptMaterial), ChainPostCloseError> {
        let admitted_at = clock.now();
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("begin"))?;
        let layout_version = schema::runtime_layout_version(&transaction)?;
        // runtime_layout_version has attested the exact current catalog above.
        if !matches!(layout_version, 8 | 9 | 10 | 11 | 12 | 13) {
            return Err(ChainPostCloseError::UnsupportedVersion);
        }
        let recovery = inspect_run_on_at_layout(&transaction, &lease.intent_id, layout_version)?;
        check_lease(&transaction, lease, admitted_at)?;
        if recovery.context.run_id() != &lease.run_id
            || recovery.input.encode()? != lease.input.encode()?
            || recovery.generation != lease.generation
            || recovery.head != lease.head
        {
            return Err(ChainPostCloseError::StaleLease {
                intent_id: lease.intent_id.as_str().to_owned(),
            });
        }
        let parent = validate_full_run(&transaction, &lease.intent_id, &recovery, layout_version)?;
        let positions = load_positions(&transaction, &lease.intent_id)?
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        validate_positions(
            &transaction,
            &lease.intent_id,
            &positions,
            &recovery,
            &parent,
            layout_version,
            None,
            None,
        )?;
        if positions.decoded.requested_codes() != requested_codes {
            return Err(ChainPostCloseError::InvalidInput {
                check: "position concept codes",
            });
        }
        if positions.row_count == 0 {
            return Err(ChainPostCloseError::InvalidInput {
                check: "empty position concept batch",
            });
        }
        if let Some(stored) = load_position_concepts(&transaction, &lease.intent_id)? {
            validate_position_concepts(&stored, &positions, &recovery)?;
            let result = position_material(&lease.intent_id, &stored);
            transaction.commit().map_err(|_| storage("commit"))?;
            return Ok((lease.head, result));
        }
        let observation = clock.cache_observation();
        let observed_at = UtcMicros::try_new(observation.observed_local.timestamp_micros())
            .map_err(|_| ChainPostCloseError::InvalidInput {
                check: "position cache observation",
            })?;
        if observed_at < admitted_at {
            return Err(ChainPostCloseError::InvalidInput {
                check: "position cache observation",
            });
        }
        let cache_cutoff = observation
            .cutoff_local
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let cache_rows = {
            let mut statement = transaction
                .prepare(
                    "SELECT code,concepts,CAST(updated_at AS TEXT) FROM stock_concepts \
                     WHERE updated_at>=?1",
                )
                .map_err(|_| storage("position concept source prepare"))?;
            let rows = statement
                .query_map([&cache_cutoff], |row| {
                    Ok(PositionCacheSourceRow {
                        code: row.get(0)?,
                        concepts: row.get(1)?,
                        updated_at: row.get(2)?,
                    })
                })
                .map_err(|_| storage("position concept source read"))?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|_| storage("position concept source read"))?;
            rows
        };
        let bytes = positions_codec::encode_position_concepts(
            observed_at,
            observation.observed_local.offset().local_minus_utc(),
            observation.cutoff_local.offset().local_minus_utc(),
            cache_cutoff.clone(),
            positions.run_version,
            positions.digest.clone(),
            requested_codes.to_vec(),
            &cache_rows,
        )?;
        positions_codec::decode_position_concepts(&bytes)?;
        let committed_at = clock.now();
        validate_observation(
            &observation.observed_local,
            &observation.cutoff_local,
            committed_at,
        )?;
        check_lease(&transaction, lease, committed_at)?;
        let digest = raw_digest(&bytes);
        let next = lease
            .head
            .checked_add(1)
            .ok_or_else(|| storage("head overflow"))?;
        advance_run(
            &transaction,
            lease,
            lease.head,
            next,
            committed_at,
            "position concept material cas",
        )?;
        transaction
            .execute(
                "INSERT INTO chain_post_close_position_concept_materials(\
                 intent_id,run_id,run_context_sha256,input_sha256,material_codec_version,\
                 material_bytes,material_length,material_sha256,lease_owner,lease_generation,\
                 prior_head_version,run_version,observed_at,committed_at,batch_kind,\
                 positions_run_version,positions_sha256,requested_count,cache_row_count,\
                 cache_max_age_days,cache_cutoff,local_offset_seconds,cutoff_local_offset_seconds) \
                 VALUES(?1,?2,?3,?4,1,?5,?6,?7,?8,?9,?10,?11,?12,?13,'PositionConcepts',\
                 ?14,?15,?16,?17,7,?18,?19,?20)",
                params![
                    lease.intent_id.as_str(),
                    lease.run_id.as_str(),
                    recovery.context.canonical_sha256().as_str(),
                    raw_digest(&recovery.input.encode()?).as_str(),
                    &bytes,
                    i64::try_from(bytes.len())
                        .map_err(|_| storage("position concept material length"))?,
                    digest.as_str(),
                    lease.owner.as_str(),
                    lease.generation,
                    lease.head,
                    next,
                    observed_at.get(),
                    committed_at.get(),
                    positions.run_version,
                    positions.digest,
                    i64::try_from(requested_codes.len())
                        .map_err(|_| storage("position request count"))?,
                    i64::try_from(cache_rows.len())
                        .map_err(|_| storage("position cache row count"))?,
                    cache_cutoff,
                    observation.observed_local.offset().local_minus_utc(),
                    observation.cutoff_local.offset().local_minus_utc(),
                ],
            )
            .map_err(|_| storage("position concept material fact"))?;
        super::validate_run_fact_versions_at_layout(
            &transaction,
            &lease.intent_id,
            next,
            layout_version,
        )?;
        let mut written_recovery = recovery;
        written_recovery.head = next;
        written_recovery.updated_at = committed_at.get();
        let written = load_position_concepts(&transaction, &lease.intent_id)?
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        validate_position_concepts(&written, &positions, &written_recovery)?;
        transaction.commit().map_err(|_| storage("commit"))?;
        Ok((next, position_material(&lease.intent_id, &written)))
    }
}

fn validate_observation(
    observed: &DateTime<FixedOffset>,
    cutoff: &DateTime<FixedOffset>,
    committed: UtcMicros,
) -> Result<(), ChainPostCloseError> {
    if observed.signed_duration_since(cutoff) != Duration::days(7)
        || observed.timestamp_micros() < 0
        || observed.timestamp_micros() > committed.get()
    {
        return Err(ChainPostCloseError::InvalidInput {
            check: "position cache observation",
        });
    }
    Ok(())
}

fn valid_cache_observation(stored: &StoredPositionConcepts) -> bool {
    let Some(observed_offset) = FixedOffset::east_opt(stored.local_offset_seconds) else {
        return false;
    };
    let Some(cutoff_offset) = FixedOffset::east_opt(stored.cutoff_local_offset_seconds) else {
        return false;
    };
    let Some(observed_utc) = Utc.timestamp_micros(stored.observed_at).single() else {
        return false;
    };
    let Ok(cutoff_naive) = NaiveDateTime::parse_from_str(&stored.cache_cutoff, "%Y-%m-%d %H:%M:%S")
    else {
        return false;
    };
    if cutoff_naive.format("%Y-%m-%d %H:%M:%S").to_string() != stored.cache_cutoff {
        return false;
    }
    let Some(cutoff) = cutoff_offset.from_local_datetime(&cutoff_naive).single() else {
        return false;
    };
    let observed_local = observed_utc.with_timezone(&observed_offset);
    let expected_cutoff = (observed_utc - Duration::days(7)).with_timezone(&cutoff_offset);
    let age = observed_local.signed_duration_since(cutoff);
    expected_cutoff.format("%Y-%m-%d %H:%M:%S").to_string() == stored.cache_cutoff
        && age >= Duration::days(7)
        && age < Duration::days(7) + Duration::seconds(1)
}

fn position_material(
    intent: &IntentId,
    stored: &StoredPositionConcepts,
) -> PositionConceptMaterial {
    PositionConceptMaterial {
        parent: PositionBatchParent {
            _validated: (),
            intent_id: intent.clone(),
            run_id: stored.run_id.clone(),
            context_digest: stored.context_digest.clone(),
            input_digest: stored.input_digest.clone(),
            cache_version: stored.run_version,
            cache_digest: stored.digest.clone(),
            positions_version: stored.positions_run_version,
            positions_digest: stored.positions_digest.clone(),
            requested_codes: stored.decoded.requested_codes.clone(),
        },
        cache_rows: stored.decoded.cache_rows.clone(),
    }
}

pub(super) fn load_position_batch_parent(
    transaction: &Transaction<'_>,
    intent: &IntentId,
    recovery: &RunRecovery,
) -> Result<Option<PositionConceptMaterial>, ChainPostCloseError> {
    load_position_batch_parent_at_layout(transaction, intent, recovery, 9)
}

pub(super) fn load_position_batch_parent_at_layout(
    transaction: &Transaction<'_>,
    intent: &IntentId,
    recovery: &RunRecovery,
    layout_version: i64,
) -> Result<Option<PositionConceptMaterial>, ChainPostCloseError> {
    load_position_batch_parent_scoped(transaction, intent, recovery, layout_version, None)
}

pub(super) fn load_position_batch_parent_scoped(
    transaction: &Transaction<'_>,
    intent: &IntentId,
    recovery: &RunRecovery,
    layout_version: i64,
    proof: Option<&schema::V12CatalogProof<'_, '_>>,
) -> Result<Option<PositionConceptMaterial>, ChainPostCloseError> {
    if proof.is_some() && layout_version < 12 {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    if layout_version >= 12 {
        schema::verify_parent_layout_v12_scoped(transaction, proof)?;
    } else if !matches!(layout_version, 9 | 10 | 11) {
        return Err(ChainPostCloseError::UnsupportedVersion);
    }
    let concepts = load_position_concepts(transaction, intent)?;
    let Some(concepts) = concepts else {
        return Ok(None);
    };
    validate_full_run_scoped(transaction, intent, recovery, layout_version, proof)?;
    Ok(Some(position_material(intent, &concepts)))
}

pub(super) fn inspect_position_stage_completion_at_layout(
    transaction: &Transaction<'_>,
    intent: &IntentId,
    recovery: &RunRecovery,
    layout_version: i64,
) -> Result<PositionStageCompletion, ChainPostCloseError> {
    if layout_version >= 12 {
        schema::verify_parent_layout_v12(transaction)?;
    } else if !matches!(layout_version, 10 | 11) {
        return Err(ChainPostCloseError::UnsupportedVersion);
    }
    let parent = validate_full_run(transaction, intent, recovery, layout_version)?;
    let positions =
        load_positions(transaction, intent)?.ok_or(ChainPostCloseError::SchemaRejected)?;
    validate_positions(
        transaction,
        intent,
        &positions,
        recovery,
        &parent,
        layout_version,
        None,
        None,
    )?;
    let concepts = load_position_concepts(transaction, intent)?;
    position_stage_completion_from_materials(transaction, intent, recovery, positions, concepts)
}

pub(super) fn position_stage_completion_from_validated_at_layout<
    'transaction,
    'connection,
    'run,
>(
    transaction: &'transaction Transaction<'connection>,
    intent: &IntentId,
    recovery: &'run RunRecovery,
    layout_version: i64,
    validated: ValidatedPositionFacts<'transaction, 'connection, 'run>,
) -> Result<PositionStageCompletion, ChainPostCloseError> {
    if !validated.matches(transaction, intent, recovery, layout_version) {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    let positions = validated
        .positions
        .ok_or(ChainPostCloseError::SchemaRejected)?;
    position_stage_completion_from_materials(
        transaction,
        intent,
        recovery,
        positions,
        validated.concepts,
    )
}

fn position_stage_completion_from_materials(
    transaction: &Transaction<'_>,
    intent: &IntentId,
    recovery: &RunRecovery,
    positions: StoredPositions,
    concepts: Option<StoredPositionConcepts>,
) -> Result<PositionStageCompletion, ChainPostCloseError> {
    if positions.row_count == 0 {
        if concepts.is_some() || !positions.decoded.requested_codes().is_empty() {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        return Ok(PositionStageCompletion {
            kind: PositionStageKind::EmptyPositions,
            run_id: recovery.context.run_id().as_str().to_owned(),
            context_digest: recovery.context.canonical_sha256().as_str().to_owned(),
            input_digest: raw_digest(&recovery.input.encode()?).as_str().to_owned(),
            positions_version: positions.run_version,
            positions_digest: positions.digest,
            positions_owner: positions.owner,
            positions_generation: positions.generation,
            positions_time: positions.committed_at,
            concepts_version: None,
            concepts_digest: None,
            concepts_owner: None,
            concepts_generation: None,
            concepts_time: None,
            requested_codes: Vec::new(),
            fetched: Vec::new(),
            completion_version: positions.run_version,
        });
    }
    let concepts = concepts.ok_or(ChainPostCloseError::IncompleteEffect {
        intent_id: intent.as_str().to_owned(),
    })?;
    validate_position_concepts(&concepts, &positions, recovery)?;
    let concepts_version = concepts.run_version;
    let concepts_digest = concepts.digest.clone();
    let concepts_owner = concepts.owner.clone();
    let concepts_generation = concepts.generation;
    let concepts_time = concepts.committed_at;
    let batch = position_material(intent, &concepts)
        .into_batch()
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    let (kind, completion_version, fetched) = if batch.work.is_empty() {
        (PositionStageKind::AllCached, concepts_version, Vec::new())
    } else {
        let fetched = super::position_concept_rpc::completed_parent_facts(
            transaction,
            intent,
            recovery,
            &batch,
        )?;
        let completion_version = fetched
            .iter()
            .map(|fact| fact.cache_version)
            .max()
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        (
            PositionStageKind::FetchedAndCached,
            completion_version,
            fetched,
        )
    };
    Ok(PositionStageCompletion {
        kind,
        run_id: recovery.context.run_id().as_str().to_owned(),
        context_digest: recovery.context.canonical_sha256().as_str().to_owned(),
        input_digest: raw_digest(&recovery.input.encode()?).as_str().to_owned(),
        positions_version: positions.run_version,
        positions_digest: positions.digest,
        positions_owner: positions.owner,
        positions_generation: positions.generation,
        positions_time: positions.committed_at,
        concepts_version: Some(concepts_version),
        concepts_digest: Some(concepts_digest),
        concepts_owner: Some(concepts_owner),
        concepts_generation: Some(concepts_generation),
        concepts_time: Some(concepts_time),
        requested_codes: batch.parent.requested_codes,
        fetched,
        completion_version,
    })
}
